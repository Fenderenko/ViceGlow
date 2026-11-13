//! The window: borderless, opaque, wide, next to the tray, with WebView2 inside. The page talks to the
//! app through WebView2 web messages (JSON both ways), no local server.

use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

use webview2_com::{
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_COLOR, COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC, CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2, ICoreWebView2Controller, ICoreWebView2Controller2, ICoreWebView2Settings3,
        ICoreWebView2Settings5,
    },
    WebMessageReceivedEventHandler,
};
use windows::{
    Win32::{
        Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{
            Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmExtendFrameIntoClientArea, DwmSetWindowAttribute},
            Gdi::{CreateSolidBrush, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint},
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::MARGINS,
            WindowsAndMessaging::{
                CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, GetCursorPos, GetSystemMetrics, HWND_TOPMOST, ICON_BIG, ICON_SMALL, IDC_ARROW, IMAGE_ICON, LR_SHARED, LoadCursorW,
                LoadImageW, RegisterClassExW, SM_CXICON, SM_CXSMICON, SW_HIDE, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, SendMessageW, SetForegroundWindow, SetWindowPos, ShowWindow,
                WINDOW_STYLE, WM_ACTIVATE, WM_CLOSE, WM_NCCALCSIZE, WM_NCHITTEST, WM_SETICON, WM_SIZE, WNDCLASSEXW, WS_CAPTION, WS_CLIPCHILDREN, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_THICKFRAME,
            },
        },
    },
    core::{BOOL, HSTRING, PCWSTR, PWSTR, w},
};

use crate::{actions, config, engine, logf, tray, util};

const W: f64 = 1000.0; // DIP
const H: f64 = 640.0;
/// Window colours of the light and dark themes (also the page backgrounds)
const BG_LIGHT: (u8, u8, u8) = (0xF5, 0xF3, 0xFB);
const BG_DARK: (u8, u8, u8) = (0x14, 0x12, 0x19);

/// The dark theme is on: chosen, or "system" with Windows apps in dark mode.
fn dark() -> bool {
    match config::get().rest.get("theme").and_then(|v| v.as_str()) {
        Some("dark") => true,
        Some("system") => windows_dark(),
        _ => false,
    }
}

fn windows_dark() -> bool {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    let mut v = 1u32;
    let mut n = 4u32;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"), w!("AppsUseLightTheme"), RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as *mut _), Some(&mut n)) };
    r.is_ok() && v == 0
}

/// A pinned window stays open when it loses the focus.
fn pinned() -> bool {
    config::get().rest.get("pinned").and_then(|v| v.as_bool()).unwrap_or(false)
}

fn bg() -> (u8, u8, u8) {
    if dark() { BG_DARK } else { BG_LIGHT }
}

#[derive(Default)]
struct Fly {
    hwnd: HWND,
    ctl: Option<ICoreWebView2Controller>,
    wv: Option<ICoreWebView2>,
    creating: bool,
    visible: bool,
    hidden_at: Option<Instant>,
    class_ok: bool,
}

thread_local! {
    static FLY: RefCell<Fly> = RefCell::new(Fly::default());
}

fn fly<R>(f: impl FnOnce(&mut Fly) -> R) -> R {
    FLY.with(|x| f(&mut x.borrow_mut()))
}

pub fn toggle() {
    if fly(|f| f.visible) {
        hide();
    } else if fly(|f| f.hidden_at.is_none_or(|t| t.elapsed() > Duration::from_millis(400))) {
        open(); // a click on the icon that just took the focus away does not reopen it
    }
}

pub fn open() {
    if fly(|f| f.creating) {
        return;
    }
    if fly(|f| f.hwnd.is_invalid()) {
        create();
    } else {
        show();
    }
}

fn dpi() -> f64 {
    let d = unsafe { windows::Win32::UI::HiDpi::GetDpiForSystem() };
    if d < 96 { 1.0 } else { d as f64 / 96.0 }
}

/// Lower right corner of the work area of the monitor with the tray icon.
fn target_rect() -> (i32, i32, i32, i32) {
    let sc = dpi();
    let (w, h) = ((W * sc) as i32, (H * sc) as i32);
    let (cx, cy) = match tray::icon_rect().filter(|r| r.right > r.left) {
        Some(r) => ((r.left + r.right) / 2, (r.top + r.bottom) / 2),
        None => {
            let mut p = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut p);
            }
            (p.x, p.y)
        }
    };
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let _ = GetMonitorInfoW(MonitorFromPoint(POINT { x: cx, y: cy }, MONITOR_DEFAULTTONEAREST), &mut mi);
    }
    let wk = mi.rcWork;
    let m = (12.0 * sc) as i32;
    let h = h.min(wk.bottom - wk.top - 2 * m);
    let w = w.min(wk.right - wk.left - 2 * m);
    let mut y = wk.bottom - h - m;
    if cy < (wk.top + wk.bottom) / 2 && wk.top > mi.rcMonitor.top {
        y = wk.top + m; // taskbar at the top
    }
    (wk.right - w - m, y, w, h)
}

fn register_class() -> bool {
    if fly(|f| f.class_ok) {
        return true;
    }
    unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: inst.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: {
                let c = bg();
                CreateSolidBrush(COLORREF(c.0 as u32 | (c.1 as u32) << 8 | (c.2 as u32) << 16))
            },
            lpszClassName: w!("ViceGlowFly"),
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return false;
        }
    }
    fly(|f| f.class_ok = true);
    true
}

fn create() {
    if !register_class() {
        logf!("RegisterClassEx(fly) failed");
        return;
    }
    let (x, y, w, h) = target_rect();
    // the frame stays for the DWM shadow and rounding but takes no space (WM_NCCALCSIZE)
    let style = WS_POPUP | WS_THICKFRAME | WS_CAPTION | WS_CLIPCHILDREN;
    let hwnd = unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST, w!("ViceGlowFly"), w!("Vice Glow"), WINDOW_STYLE(style.0), x, y, w, h, None, None, Some(inst.into()), None)
    };
    let Ok(hwnd) = hwnd else {
        logf!("CreateWindowEx(fly) failed");
        return;
    };
    fly(|f| {
        f.hwnd = hwnd;
        f.creating = true;
        f.visible = true;
    });
    unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        for (which, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
            let cx = GetSystemMetrics(metric);
            if let Ok(ic) = LoadImageW(Some(inst.into()), PCWSTR(1 as *const u16), IMAGE_ICON, cx, cx, LR_SHARED) {
                SendMessageW(hwnd, WM_SETICON, Some(WPARAM(which as usize)), Some(LPARAM(ic.0 as isize)));
            }
        }
        let pref = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const _ as *const _, 4);
        let dark = BOOL::from(dark());
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as *const _, 4);
        let m = MARGINS { cxLeftWidth: 1, cxRightWidth: 1, cyTopHeight: 1, cyBottomHeight: 1 };
        let _ = DwmExtendFrameIntoClientArea(hwnd, &m);
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER);
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_SHOWWINDOW);
        let _ = SetForegroundWindow(hwnd);
    }
    engine::set_awake(true);

    let data = HSTRING::from(util::data_dir().join("webview").as_os_str());
    let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |res, env| {
        let Some(env) = env.filter(|_| res.is_ok()) else {
            fail("environment", res.err());
            return Ok(());
        };
        let h2 = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |res, ctl| {
            match ctl.filter(|_| res.is_ok()) {
                Some(ctl) => ready(hwnd, ctl),
                None => fail("controller", res.err()),
            }
            Ok(())
        }));
        if let Err(e) = unsafe { env.CreateCoreWebView2Controller(hwnd, &h2) } {
            fail("controller", Some(e));
        }
        Ok(())
    }));
    if let Err(e) = unsafe { CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), &data, None, &handler) } {
        fail("environment", Some(e));
    }
}

fn fail(what: &str, e: Option<windows::core::Error>) {
    logf!("WebView2 failed ({what}): {e:?} — is the WebView2 Runtime installed?");
    close();
}

/// The page with the effect tables and translations put in: read from disk in debug builds (edit and reload),
/// embedded in release.
fn page() -> String {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/web/");
    let (mut html, mut fx) = (include_str!("../web/index.html").to_string(), include_str!("../web/fx.json").to_string());
    if cfg!(debug_assertions) {
        if let Ok(s) = std::fs::read_to_string(format!("{dir}index.html")) {
            html = s;
        }
        if let Ok(s) = std::fs::read_to_string(format!("{dir}fx.json")) {
            fx = s;
        }
    }
    let i18n = util::i18n_json();
    html.replacen("window.FX=null", &format!("window.FX={}", fx.trim()), 1).replacen("window.I18N=null", &format!("window.I18N={}", i18n.trim()), 1)
}

fn ready(hwnd: HWND, ctl: ICoreWebView2Controller) {
    let Ok(wv) = (unsafe { ctl.CoreWebView2() }) else {
        fail("webview", None);
        return;
    };
    unsafe {
        if let Ok(st) = wv.Settings() {
            let _ = st.SetAreDefaultContextMenusEnabled(false);
            let _ = st.SetAreDevToolsEnabled(cfg!(debug_assertions));
            let _ = st.SetIsZoomControlEnabled(false);
            let _ = st.SetIsStatusBarEnabled(false);
            if let Ok(s3) = windows::core::Interface::cast::<ICoreWebView2Settings3>(&st) {
                let _ = s3.SetAreBrowserAcceleratorKeysEnabled(false);
            }
            if let Ok(s5) = windows::core::Interface::cast::<ICoreWebView2Settings5>(&st) {
                let _ = s5.SetIsPinchZoomEnabled(false);
            }
        }
        if let Ok(c2) = windows::core::Interface::cast::<ICoreWebView2Controller2>(&ctl) {
            let c = bg();
            let _ = c2.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR { A: 255, R: c.0, G: c.1, B: c.2 });
        }
        let mut token = Default::default();
        let _ = wv.add_WebMessageReceived(
            &WebMessageReceivedEventHandler::create(Box::new(|_, args| {
                if let Some(a) = args {
                    let mut p = PWSTR::null();
                    if a.WebMessageAsJson(&mut p).is_ok() && !p.is_null() {
                        let s = p.to_string().unwrap_or_default();
                        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                            actions::on_page(&v);
                        }
                    }
                }
                Ok(())
            })),
            &mut token,
        );
    }
    fly(|f| {
        f.ctl = Some(ctl.clone());
        f.wv = Some(wv.clone());
        f.creating = false;
    });
    resize();
    unsafe {
        let _ = ctl.SetIsVisible(fly(|f| f.visible));
        let _ = wv.NavigateToString(&HSTRING::from(page()));
    }
    let _ = hwnd;
}

fn resize() {
    let (ctl, hwnd) = fly(|f| (f.ctl.clone(), f.hwnd));
    if let Some(ctl) = ctl {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rc);
            let _ = ctl.SetBounds(rc);
        }
    }
}

fn show() {
    let hwnd = fly(|f| f.hwnd);
    let (x, y, w, h) = target_rect();
    unsafe {
        let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_SHOWWINDOW);
        let _ = SetForegroundWindow(hwnd);
    }
    if let Some(ctl) = fly(|f| f.ctl.clone()) {
        resize();
        unsafe {
            let _ = ctl.SetIsVisible(true);
            let _ = ctl.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
        }
    }
    fly(|f| f.visible = true);
    engine::set_awake(true);
    push_state();
}

pub fn hide() {
    let (hwnd, vis, ctl) = fly(|f| (f.hwnd, f.visible, f.ctl.clone()));
    if hwnd.is_invalid() || !vis {
        return;
    }
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
        if let Some(ctl) = ctl {
            let _ = ctl.SetIsVisible(false);
        }
    }
    fly(|f| {
        f.visible = false;
        f.hidden_at = Some(Instant::now());
    });
    engine::set_awake(false);
    engine::scan(false);
    crate::hotkeys::capture(false);
}

pub fn close() {
    let (ctl, hwnd) = fly(|f| {
        f.wv = None;
        f.creating = false;
        f.visible = false;
        (f.ctl.take(), std::mem::take(&mut f.hwnd))
    });
    if let Some(ctl) = ctl {
        unsafe {
            let _ = ctl.Close();
        }
    }
    if !hwnd.is_invalid() {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
    engine::set_awake(false);
}

/// Sends the current state to the page (if it is open).
pub fn push_state() {
    let Some(wv) = fly(|f| f.wv.clone().filter(|_| f.visible)) else { return };
    unsafe {
        let _ = wv.PostWebMessageAsJson(&HSTRING::from(actions::state_json()));
    }
}

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_NCCALCSIZE if wp.0 != 0 => return LRESULT(0),
        WM_NCHITTEST => return LRESULT(1), // HTCLIENT
        WM_SIZE => {
            resize();
            return LRESULT(0);
        }
        WM_ACTIVATE if wp.0 & 0xffff == 0 && !pinned() => hide(),
        WM_CLOSE => {
            hide();
            return LRESULT(0);
        }
        _ => {}
    }
    unsafe { DefWindowProcW(h, m, wp, lp) }
}
