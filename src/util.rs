//! Paths, the log and posting to the UI thread.

use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicIsize, Ordering},
    },
};

use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::PostMessageW,
};

static MAIN_HWND: AtomicIsize = AtomicIsize::new(0);
static LOG: OnceLock<Mutex<Option<File>>> = OnceLock::new();

/// %APPDATA%\Vice Glow; the settings of the first builds (%APPDATA%\Glowo) move over once.
pub fn data_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let d = base.join("Vice Glow");
        let old = base.join("Glowo").join("config.json");
        if !d.join("config.json").exists() && old.exists() {
            let _ = std::fs::create_dir_all(&d);
            let _ = std::fs::copy(&old, d.join("config.json"));
        }
        let _ = std::fs::create_dir_all(&d);
        d
    })
    .clone()
}

/// Starts a fresh log for this run; the previous one is kept as viceglow.old.log.
pub fn init_log() {
    let d = data_dir();
    let _ = std::fs::rename(d.join("viceglow.log"), d.join("viceglow.old.log"));
    let f = OpenOptions::new().create(true).write(true).truncate(true).open(d.join("viceglow.log")).ok();
    let _ = LOG.set(Mutex::new(f));
}

pub fn log(s: &str) {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let line = format!("{}.{:03} {s}\n", local_hms(t / 1000), t % 1000);
    if cfg!(debug_assertions) {
        eprint!("{line}");
    }
    if let Some(m) = LOG.get()
        && let Ok(mut g) = m.lock()
        && let Some(f) = g.as_mut()
    {
        let _ = f.write_all(line.as_bytes());
    }
}

#[macro_export]
macro_rules! logf {
    ($($a:tt)*) => { $crate::util::log(&format!($($a)*)) };
}

fn local_hms(unix: u128) -> String {
    let s = (unix as i64 + local_offset()).rem_euclid(86400);
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// Local time: (seconds since midnight, weekday with Monday = 0).
pub fn local_now() -> (u32, u8) {
    let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let l = unix + local_offset();
    let day = l.div_euclid(86400);
    // 1970-01-01 was a Thursday (Monday = 0 → 3)
    (l.rem_euclid(86400) as u32, ((day + 3).rem_euclid(7)) as u8)
}

/// Seconds to add to UTC for local time (with daylight saving).
fn local_offset() -> i64 {
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    const TIME_ZONE_ID_DAYLIGHT: u32 = 2;
    let mut tz = TIME_ZONE_INFORMATION::default();
    let r = unsafe { GetTimeZoneInformation(&mut tz) };
    let mut bias = tz.Bias;
    if r == TIME_ZONE_ID_DAYLIGHT {
        bias += tz.DaylightBias;
    } else {
        bias += tz.StandardBias;
    }
    -(bias as i64) * 60
}

pub fn set_main_hwnd(h: HWND) {
    MAIN_HWND.store(h.0 as isize, Ordering::Relaxed);
}

pub fn main_hwnd() -> HWND {
    HWND(MAIN_HWND.load(Ordering::Relaxed) as *mut _)
}

/// Posts a message to the hidden main window (any thread).
pub fn post_ui(msg: u32, wp: usize) {
    let h = main_hwnd();
    if !h.is_invalid() {
        unsafe {
            let _ = PostMessageW(Some(h), msg, WPARAM(wp), LPARAM(0));
        }
    }
}

/// Opens a web page in the default browser.
pub fn open_url(url: &str) {
    use windows::{
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        core::{PCWSTR, w},
    };
    let u = wide(url);
    unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(u.as_ptr()), None, None, SW_SHOWNORMAL);
    }
}

/// The interface language: English unless Ukrainian is chosen (or "system" with a Ukrainian
/// Windows).
pub fn english() -> bool {
    match crate::config::get().rest.get("lang").and_then(|v| v.as_str()) {
        Some("en") => true,
        Some("uk") => false,
        Some("system") => (unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() } & 0x3ff) != 0x22,
        _ => true,
    }
}

/// A Ukrainian interface string in the interface language (tray menu and tooltip).
pub fn tr(uk: &'static str) -> &'static str {
    if !english() {
        return uk;
    }
    match uk {
        "Вимкнути" => "Turn off",
        "Увімкнути" => "Turn on",
        "Яскравість" => "Brightness",
        "Колір" => "Colour",
        "Сцени" => "Scenes",
        "Лампа" => "Lamp",
        "Відкрити Vice Glow" => "Open Vice Glow",
        "Вихід" => "Exit",
        "Увімкнено" => "On",
        "Вимкнено" => "Off",
        "Вечір" => "Evening",
        "Кіно" => "Movie",
        "Стрім" => "Stream",
        "Читання" => "Reading",
        "Вечірка" => "Party",
        _ => uk,
    }
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn copy_wide(dst: &mut [u16], s: &str) {
    let n = dst.len().saturating_sub(1);
    let mut i = 0;
    for c in s.encode_utf16().take(n) {
        dst[i] = c;
        i += 1;
    }
    dst[i] = 0;
}
