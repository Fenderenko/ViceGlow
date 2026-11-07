//! The tray icon and its menu.

use windows::{
    Win32::{
        Foundation::{HWND, POINT, RECT},
        Graphics::Gdi::{CreateBitmap, DeleteObject, HBITMAP, HGDIOBJ},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFY_ICON_DATA_FLAGS, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect, Shell_NotifyIconW},
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, GetSystemMetrics, HICON, HMENU, IMAGE_ICON, LR_SHARED, LoadImageW, MENUITEMINFOW, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MIIM_BITMAP,
                PostMessageW, SM_CXSMICON, SetForegroundWindow, SetMenuItemInfoW, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL,
            },
        },
    },
    core::PCWSTR,
};

use crate::{
    actions, config,
    engine::{self, Act},
    fly,
    lamp::Op,
    live, util,
};

const ID_TOGGLE: u32 = 1;
const ID_OPEN: u32 = 2;
const ID_EXIT: u32 = 3;
const ID_BRIGHT: u32 = 100; // + percent
const ID_COLOR: u32 = 200; // + favourite index
const ID_SCENE: u32 = 300; // + scene index
const ID_LAMP: u32 = 400; // + lamp index

fn nid(flags: NOTIFY_ICON_DATA_FLAGS) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: util::main_hwnd(), uID: 1, uFlags: flags, ..Default::default() }
}

fn icon() -> HICON {
    unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let cx = GetSystemMetrics(SM_CXSMICON);
        LoadImageW(Some(inst.into()), PCWSTR(1 as *const u16), IMAGE_ICON, cx, cx, LR_SHARED).map(|h| HICON(h.0)).unwrap_or_default()
    }
}

fn tip() -> String {
    let name = config::active().map(|l| format!("Vice Glow — {}", l.name)).unwrap_or_else(|| "Vice Glow".into());
    match engine::state() {
        Some(l) if l.power => format!("{name}\n{}, {} %", util::tr("Увімкнено"), l.bright),
        Some(_) => format!("{name}\n{}", util::tr("Вимкнено")),
        None => name,
    }
}

/// Adds the icon; false while the taskbar is not ready (retried by the caller).
pub fn add() -> bool {
    let mut n = nid(NIF_MESSAGE | NIF_ICON | NIF_TIP);
    n.uCallbackMessage = crate::WM_TRAY;
    n.hIcon = icon();
    util::copy_wide(&mut n.szTip, &tip());
    unsafe { Shell_NotifyIconW(NIM_ADD, &n).as_bool() || Shell_NotifyIconW(NIM_MODIFY, &n).as_bool() }
}

pub fn update() {
    let mut n = nid(NIF_TIP);
    util::copy_wide(&mut n.szTip, &tip());
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &n);
    }
}

pub fn remove() {
    let n = nid(NOTIFY_ICON_DATA_FLAGS(0));
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &n);
    }
}

pub fn icon_rect() -> Option<RECT> {
    let id = NOTIFYICONIDENTIFIER { cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32, hWnd: util::main_hwnd(), uID: 1, ..Default::default() };
    unsafe { Shell_NotifyIconGetRect(&id) }.ok()
}

fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// A 16×16 colour square for a menu item (BGRA, 1 px darker border).
fn swatch(c: [u8; 3]) -> HBITMAP {
    const S: usize = 16;
    let mut px = vec![0u8; S * S * 4];
    for y in 0..S {
        for x in 0..S {
            let edge = x == 0 || y == 0 || x == S - 1 || y == S - 1;
            let k = if edge { 0.6 } else { 1.0 };
            let i = (y * S + x) * 4;
            px[i] = (c[2] as f32 * k) as u8;
            px[i + 1] = (c[1] as f32 * k) as u8;
            px[i + 2] = (c[0] as f32 * k) as u8;
            px[i + 3] = 255;
        }
    }
    unsafe { CreateBitmap(S as i32, S as i32, 1, 32, Some(px.as_ptr() as *const _)) }
}

fn append(m: HMENU, flags: windows::Win32::UI::WindowsAndMessaging::MENU_ITEM_FLAGS, id: usize, text: &str) {
    let t = util::wide(text);
    unsafe {
        let _ = AppendMenuW(m, flags, id, PCWSTR(t.as_ptr()));
    }
}

pub fn menu(hwnd: HWND) {
    let lamp = engine::state();
    let cfg = config::get();
    let mut bitmaps = Vec::new();
    unsafe {
        let Ok(m) = CreatePopupMenu() else { return };
        let on = lamp.as_ref().is_some_and(|l| l.power);
        append(m, MF_STRING, ID_TOGGLE as usize, util::tr(if on { "Вимкнути" } else { "Увімкнути" }));

        if let Ok(b) = CreatePopupMenu() {
            let cur = lamp.as_ref().map(|l| l.bright as u32);
            for p in [10u32, 25, 50, 75, 100] {
                let f = if cur == Some(p) { MF_STRING | MF_CHECKED } else { MF_STRING };
                append(b, f, (ID_BRIGHT + p) as usize, &format!("{p} %"));
            }
            append(m, MF_POPUP, b.0 as usize, util::tr("Яскравість"));
        }
        if let Ok(c) = CreatePopupMenu() {
            for (i, f) in cfg.favs.iter().enumerate() {
                let Some(rgb) = parse_hex(f) else { continue };
                let id = ID_COLOR + i as u32;
                append(c, MF_STRING, id as usize, &f.to_uppercase());
                let bm = swatch(rgb);
                let mi = MENUITEMINFOW { cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32, fMask: MIIM_BITMAP, hbmpItem: bm, ..Default::default() };
                let _ = SetMenuItemInfoW(c, id, false, &mi);
                bitmaps.push(bm);
            }
            let f = if cfg.favs.is_empty() { MF_POPUP | MF_GRAYED } else { MF_POPUP };
            append(m, f, c.0 as usize, util::tr("Колір"));
        }
        if !cfg.scenes.is_empty()
            && let Ok(sc) = CreatePopupMenu()
        {
            for (i, x) in cfg.scenes.iter().enumerate() {
                append(sc, MF_STRING, (ID_SCENE + i as u32) as usize, scene_name(&x.name));
            }
            append(m, MF_POPUP, sc.0 as usize, util::tr("Сцени"));
        }
        if cfg.lamps.len() > 1
            && let Ok(lm) = CreatePopupMenu()
        {
            for (i, l) in cfg.lamps.iter().enumerate() {
                let f = if l.id == cfg.active { MF_STRING | MF_CHECKED } else { MF_STRING };
                append(lm, f, (ID_LAMP + i as u32) as usize, &l.name);
            }
            append(m, MF_POPUP, lm.0 as usize, util::tr("Лампа"));
        }
        append(m, MF_SEPARATOR, 0, "");
        append(m, MF_STRING, ID_OPEN as usize, util::tr("Відкрити Vice Glow"));
        append(m, MF_STRING, ID_EXIT as usize, util::tr("Вихід"));

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, None, hwnd, None).0 as u32;
        let _ = PostMessageW(Some(hwnd), WM_NULL, Default::default(), Default::default());
        let _ = DestroyMenu(m);
        for bm in bitmaps {
            let _ = DeleteObject(HGDIOBJ(bm.0));
        }
        match cmd {
            ID_TOGGLE => {
                if engine::state().is_some_and(|s| s.power) {
                    live::set("off");
                }
                engine::act(Act::Toggle);
            }
            ID_OPEN => fly::open(),
            ID_EXIT => crate::quit(),
            c if (ID_BRIGHT + 1..=ID_BRIGHT + 100).contains(&c) => {
                let p = (c - ID_BRIGHT) as u8;
                live::set("off");
                let mut v = Vec::new();
                if !on {
                    v.push(Op::Power(true));
                }
                v.push(Op::Bright(p));
                engine::ops(v);
            }
            c if c >= ID_COLOR && ((c - ID_COLOR) as usize) < cfg.favs.len() => {
                if let Some(rgb) = parse_hex(&cfg.favs[(c - ID_COLOR) as usize]) {
                    live::set("off");
                    live::set_base(rgb);
                    engine::ops(vec![Op::Color(actions::led(rgb))]);
                }
            }
            c if c >= ID_SCENE && ((c - ID_SCENE) as usize) < cfg.scenes.len() => {
                actions::apply_scene(&cfg.scenes[(c - ID_SCENE) as usize].id);
            }
            c if c >= ID_LAMP && ((c - ID_LAMP) as usize) < cfg.lamps.len() => {
                engine::select(&cfg.lamps[(c - ID_LAMP) as usize].id);
            }
            _ => {}
        }
    }
}

/// Built-in scene names follow the interface language; the user's own stay as typed.
fn scene_name(n: &str) -> &str {
    for d in ["Вечір", "Кіно", "Стрім", "Читання", "Вечірка"] {
        if n == d {
            return util::tr(d);
        }
    }
    n
}
