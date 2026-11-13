//! Global hotkeys: on / off and brightness up / down. Combinations come from the settings
//! (config "hotkeys": {"toggle": [mods, vk], …}; [0, 0] — off), Ctrl+Alt+L / PgUp / PgDn by default.

use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use serde_json::{Value, json};
use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey, VK_L, VK_NEXT, VK_PRIOR};

use crate::{config, logf, util};

/// Actions in the order of their hotkey ids (1, 2, 3).
pub const ACTIONS: [&str; 3] = ["toggle", "up", "down"];
pub const HK_TOGGLE: i32 = 1;
pub const HK_UP: i32 = 2;
pub const HK_DOWN: i32 = 3;

const MODS: u32 = MOD_CONTROL.0 | MOD_ALT.0;
const DEFAULTS: [(u32, u32); 3] = [(MODS, VK_L.0 as u32), (MODS, VK_PRIOR.0 as u32), (MODS, VK_NEXT.0 as u32)];

/// While the window records a combination, the hotkeys are off so the keys reach the page.
static CAPTURE: AtomicBool = AtomicBool::new(false);
/// Actions whose combination another app holds.
static BUSY: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

/// The combination of every action: (modifiers, virtual key); (0, 0) — off.
pub fn keys() -> [(u32, u32); 3] {
    let c = config::get();
    let set = c.rest.get("hotkeys");
    let mut out = DEFAULTS;
    for (i, a) in ACTIONS.iter().enumerate() {
        if let Some(v) = set.and_then(|s| s.get(a)).and_then(Value::as_array)
            && let [m, k] = v.as_slice()
        {
            out[i] = (m.as_u64().unwrap_or(0) as u32 & 0xf, k.as_u64().unwrap_or(0) as u32 & 0xff);
        }
    }
    out
}

/// A combination the app accepts: Ctrl, Alt or Win with a key, or F1–F24 alone (with any modifiers).
pub fn valid(mods: u32, vk: u32) -> bool {
    let fkey = (0x70..=0x87).contains(&vk);
    let modifier = [0x10, 0x11, 0x12, 0x5b, 0x5c, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5].contains(&vk);
    vk != 0 && !modifier && (fkey || mods & (MOD_CONTROL.0 | MOD_ALT.0 | MOD_WIN.0) != 0)
}

pub fn unregister() {
    let h = util::main_hwnd();
    for id in [HK_TOGGLE, HK_UP, HK_DOWN] {
        unsafe {
            let _ = UnregisterHotKey(Some(h), id);
        }
    }
}

/// (Re)registers the hotkeys from the settings; nothing while a combination is being recorded.
pub fn register() {
    unregister();
    let mut busy = Vec::new();
    if !CAPTURE.load(Ordering::Relaxed) {
        let h = util::main_hwnd();
        for (i, (m, vk)) in keys().into_iter().enumerate() {
            if !valid(m, vk) {
                continue;
            }
            let rep = if i == 0 { MOD_NOREPEAT.0 } else { 0 }; // brightness repeats while held
            if unsafe { RegisterHotKey(Some(h), i as i32 + 1, HOT_KEY_MODIFIERS(m | rep), vk) }.is_err() {
                logf!("hotkey {} ({m:#x}+{vk:#x}) is taken by another app", ACTIONS[i]);
                busy.push(ACTIONS[i]);
            }
        }
    }
    *BUSY.lock().unwrap() = busy;
}

pub fn capture(on: bool) {
    if CAPTURE.swap(on, Ordering::Relaxed) != on {
        register();
    }
}

fn save(k: [(u32, u32); 3]) {
    let v: serde_json::Map<String, Value> = ACTIONS.iter().zip(k).map(|(a, (m, vk))| (a.to_string(), json!([m, vk]))).collect();
    config::update(|c| {
        c.rest.insert("hotkeys".into(), Value::Object(v));
    });
}

/// Sets the combination of `action`; one another action already uses — the two swap.
/// (0, 0) turns the hotkey off.
pub fn set(action: &str, mods: u32, vk: u32) {
    let Some(i) = ACTIONS.iter().position(|a| *a == action) else { return };
    let (mods, vk) = if vk == 0 { (0, 0) } else { (mods & (MOD_CONTROL.0 | MOD_ALT.0 | MOD_SHIFT.0 | MOD_WIN.0), vk & 0xff) };
    if vk != 0 && !valid(mods, vk) {
        return;
    }
    let mut k = keys();
    if vk != 0
        && let Some(j) = k.iter().position(|x| *x == (mods, vk))
    {
        k[j] = k[i];
    }
    k[i] = (mods, vk);
    save(k);
    CAPTURE.store(false, Ordering::Relaxed);
    register();
}

pub fn reset() {
    save(DEFAULTS);
    CAPTURE.store(false, Ordering::Relaxed);
    register();
}

/// For the page: {"toggle": [mods, vk], …, "busy": ["up", …]}.
pub fn state() -> Value {
    let mut v: serde_json::Map<String, Value> = ACTIONS.iter().zip(keys()).map(|(a, (m, vk))| (a.to_string(), json!([m, vk]))).collect();
    v.insert("busy".into(), json!(*BUSY.lock().unwrap()));
    Value::Object(v)
}

#[cfg(test)]
mod tests {
    use super::valid;

    #[test]
    fn combinations_need_a_modifier_except_f_keys() {
        assert!(valid(3, 0x4c)); // Ctrl+Alt+L
        assert!(valid(8, 0x21)); // Win+PgUp
        assert!(!valid(0, 0x4c)); // L alone
        assert!(!valid(4, 0x4c)); // Shift+L
        assert!(valid(0, 0x7b)); // F12
        assert!(!valid(2, 0x11)); // Ctrl alone
    }
}
