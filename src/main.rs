//! Vice Glow — LED lamps from the PC (BanlanX and other Bluetooth / Wi-Fi families): tray icon,
//! hotkeys, scenes, light from the PC sound or screen, and a window.

#![windows_subsystem = "windows"]

mod actions;
mod autostart;
mod config;
mod drv;
mod engine;
mod fly;
mod gatt;
mod lamp;
mod live;
mod proto;
mod tray;
mod util;

use windows::{
    Win32::{
        Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM},
        System::{
            Com::{COINIT_APARTMENTTHREADED, CoInitializeEx},
            LibraryLoader::GetModuleHandleW,
            Threading::CreateMutexW,
        },
        UI::{
            HiDpi::{DPI_AWARENESS_CONTEXT_SYSTEM_AWARE, SetProcessDpiAwarenessContext},
            Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey, VK_L, VK_NEXT, VK_PRIOR},
            WindowsAndMessaging::{
                AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowW, GetMessageW, GetWindowThreadProcessId, KillTimer, MSG, PostMessageW, PostQuitMessage, RegisterClassExW,
                RegisterWindowMessageW, SetTimer, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CONTEXTMENU, WM_ENDSESSION, WM_HOTKEY, WM_LBUTTONUP, WM_RBUTTONUP, WM_TIMER, WNDCLASSEXW,
            },
        },
    },
    core::w,
};

use crate::engine::Act;

/// The lamp state changed (from the engine thread).
pub const WM_STATE: u32 = WM_APP + 1;
pub const WM_TRAY: u32 = WM_APP + 2;
/// Another start of the exe asks to show the window.
pub const WM_SHOW: u32 = WM_APP + 3;

const HK_TOGGLE: i32 = 1;
const HK_UP: i32 = 2;
const HK_DOWN: i32 = 3;
const TIMER_TRAY: usize = 1;

static TASKBAR_CREATED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn main() {
    let tray_only = std::env::args().any(|a| a == "--tray");
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_SYSTEM_AWARE);
    }
    let _mutex = unsafe { CreateMutexW(None, false, w!(r"Local\ViceGlowSingleton")) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            if let Ok(h) = FindWindowW(w!("ViceGlowWnd"), None) {
                let mut pid = 0u32;
                GetWindowThreadProcessId(h, Some(&mut pid));
                let _ = AllowSetForegroundWindow(pid);
                let _ = PostMessageW(Some(h), WM_SHOW, WPARAM(0), LPARAM(0));
            }
        }
        return;
    }
    util::init_log();
    logf!("=== Vice Glow {} ===", env!("CARGO_PKG_VERSION"));
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED); // STA for WebView2
    }

    let hwnd = unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW { cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: w!("ViceGlowWnd"), ..Default::default() };
        RegisterClassExW(&wc);
        CreateWindowExW(WINDOW_EX_STYLE(0), w!("ViceGlowWnd"), w!("Vice Glow"), WINDOW_STYLE(0), 0, 0, 0, 0, None, None, Some(inst.into()), None)
    };
    let Ok(hwnd) = hwnd else {
        logf!("CreateWindowEx failed");
        return;
    };
    util::set_main_hwnd(hwnd);
    TASKBAR_CREATED.store(unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) }, std::sync::atomic::Ordering::Relaxed);

    autostart::sync_path();
    if !tray::add() {
        unsafe {
            SetTimer(Some(hwnd), TIMER_TRAY, 2000, None);
        }
    }
    register_hotkeys(hwnd);
    engine::start();
    if !tray_only {
        fly::open();
    }

    let mut m = MSG::default();
    while unsafe { GetMessageW(&mut m, None, 0, 0) }.0 > 0 {
        unsafe {
            let _ = TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
    shutdown(hwnd);
}

fn register_hotkeys(hwnd: HWND) {
    let mods = MOD_CONTROL | MOD_ALT;
    for (id, m, vk, name) in [(HK_TOGGLE, mods | MOD_NOREPEAT, VK_L.0, "Ctrl+Alt+L"), (HK_UP, mods, VK_PRIOR.0, "Ctrl+Alt+PgUp"), (HK_DOWN, mods, VK_NEXT.0, "Ctrl+Alt+PgDn")] {
        if unsafe { RegisterHotKey(Some(hwnd), id, m, vk as u32) }.is_err() {
            logf!("hotkey {name} is taken by another app");
        }
    }
}

fn shutdown(hwnd: HWND) {
    for id in [HK_TOGGLE, HK_UP, HK_DOWN] {
        unsafe {
            let _ = UnregisterHotKey(Some(hwnd), id);
        }
    }
    live::set("off");
    tray::remove();
    fly::close();
    engine::stop();
    logf!("exit");
}

pub fn quit() {
    unsafe { PostQuitMessage(0) };
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_STATE => {
            fly::push_state();
            tray::update();
        }
        WM_TRAY => match (lp.0 & 0xffff) as u32 {
            WM_LBUTTONUP => fly::toggle(),
            WM_RBUTTONUP | WM_CONTEXTMENU => tray::menu(h),
            _ => {}
        },
        WM_SHOW => fly::open(),
        WM_HOTKEY => {
            let step = config::get().step as i8;
            match wp.0 as i32 {
                HK_TOGGLE => {
                    if engine::state().is_some_and(|s| s.power) {
                        live::set("off");
                    }
                    engine::act(Act::Toggle);
                }
                HK_UP | HK_DOWN => {
                    live::set("off");
                    engine::act(Act::Bright(if wp.0 as i32 == HK_UP { step } else { -step }));
                }
                _ => {}
            }
        }
        WM_TIMER if wp.0 == TIMER_TRAY => {
            if tray::add() {
                unsafe {
                    let _ = KillTimer(Some(h), TIMER_TRAY);
                }
            }
        }
        WM_ENDSESSION if wp.0 != 0 => {
            shutdown(h);
            std::process::exit(0);
        }
        _ if m != 0 && m == TASKBAR_CREATED.load(std::sync::atomic::Ordering::Relaxed) => {
            tray::add(); // Explorer restarted
        }
        _ => {}
    }
    unsafe { DefWindowProcW(h, m, wp, lp) }
}
