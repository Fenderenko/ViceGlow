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
/// Interface languages (web/i18n.json, made by tools/i18n_src.py); Ukrainian strings are the keys.
pub const LANGS: [&str; 9] = ["uk", "en", "de", "fr", "es", "pt", "pl", "it", "tr"];

/// The Windows display language if the interface has it, otherwise English.
pub fn system_lang() -> &'static str {
    let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() } & 0x3ff;
    match id {
        0x22 => "uk",
        0x07 => "de",
        0x0c => "fr",
        0x0a => "es",
        0x16 => "pt",
        0x15 => "pl",
        0x10 => "it",
        0x1f => "tr",
        _ => "en",
    }
}

/// The interface language chosen in the settings (English by default).
pub fn lang() -> &'static str {
    match crate::config::get().rest.get("lang").and_then(|v| v.as_str()) {
        Some("system") => system_lang(),
        Some(l) => LANGS.iter().copied().find(|x| *x == l).unwrap_or("en"),
        None => "en",
    }
}

/// web/i18n.json: from disk in debug builds (edit and reload), embedded in release.
pub fn i18n_json() -> String {
    if cfg!(debug_assertions)
        && let Ok(s) = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/web/i18n.json"))
    {
        return s;
    }
    include_str!("../web/i18n.json").to_string()
}

type Dict = std::collections::HashMap<String, std::collections::HashMap<String, &'static str>>;

/// A Ukrainian interface string in the interface language (tray menu and tooltip).
pub fn tr(uk: &'static str) -> &'static str {
    static DICT: std::sync::OnceLock<Dict> = std::sync::OnceLock::new();
    let lang = lang();
    if lang == "uk" {
        return uk;
    }
    let d = DICT.get_or_init(|| {
        let v: serde_json::Value = serde_json::from_str(include_str!("../web/i18n.json")).unwrap_or_default();
        let mut d = Dict::new();
        if let Some(ui) = v.get("ui").and_then(|u| u.as_object()) {
            for (l, m) in ui {
                let m = m.as_object().map(|m| m.iter().filter_map(|(k, s)| Some((k.clone(), &*Box::leak(s.as_str()?.to_string().into_boxed_str())))).collect()).unwrap_or_default();
                d.insert(l.clone(), m);
            }
        }
        d
    });
    d.get(lang).and_then(|m| m.get(uk)).copied().unwrap_or(uk)
}

#[cfg(test)]
mod tests {
    #[test]
    fn i18n_has_every_language_and_tray_string() {
        let v: serde_json::Value = serde_json::from_str(include_str!("../web/i18n.json")).unwrap();
        for l in &super::LANGS[1..] {
            let m = &v["ui"][l];
            for k in ["Вимкнути", "Увімкнути", "Яскравість", "Колір", "Сцени", "Лампа", "Відкрити Vice Glow", "Вихід", "Увімкнено", "Вимкнено", "Вечір", "Кіно", "Стрім", "Читання", "Вечірка"] {
                assert!(m[k].is_string(), "{l}: {k}");
            }
        }
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
