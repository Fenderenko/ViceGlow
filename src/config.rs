//! Settings in %APPDATA%\Vice Glow\config.json. Unknown fields are kept; 0.1.0 files (one lamp in
//! addr / name) move to the lamp list on load.

use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{logf, util};

/// A saved lamp. Bluetooth lamps have `addr`, network lamps `host`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Lamp {
    /// "ble:<address>" or "<family>:<host>"
    pub id: String,
    pub family: String,
    pub model: String,
    pub name: String,
    pub addr: u64,
    pub host: String,
    pub extra: Value,
    /// "Відключити": no connection until "Підключити"
    pub paused: bool,
    /// white balance of the LEDs (channel gains)
    pub cal: [f32; 3],
}

impl Default for Lamp {
    fn default() -> Self {
        Self { id: String::new(), family: String::new(), model: String::new(), name: String::new(), addr: 0, host: String::new(), extra: Value::Null, paused: false, cal: [1.0; 3] }
    }
}

impl Lamp {
    pub fn is_ble(&self) -> bool {
        self.addr != 0
    }
}

/// A saved light: one click restores it.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Scene {
    pub id: String,
    pub name: String,
    /// icon key in the page (moon, film, cam, book, spark, sun, heart)
    pub icon: String,
    /// "color" | "effect" | "audio" | "screen"
    pub mode: String,
    pub rgb: [u8; 3],
    pub bright: u8,
    /// effect by name (ids differ between families); "rainbow" picks the first rainbow effect
    pub effect: String,
    pub speed: u8,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub lamps: Vec<Lamp>,
    pub active: String,
    /// favourite colours, "#rrggbb"
    pub favs: Vec<String>,
    /// brightness step of the hotkeys and the tray menu, %
    pub step: u8,
    /// screen colours → LED values (gamma); see lamp::to_led
    pub gamma: bool,
    pub scenes: Vec<Scene>,
    /// Hue bridge address → user name from pairing
    pub hue_users: Map<String, Value>,
    /// light from PC sound: "pulse" (brightness follows the music) | "spectrum" (colour too)
    pub audio_mode: String,
    pub audio_gain: u8,
    /// screen colour: whole screen or its edges
    pub screen_edges: bool,
    pub screen_sat: u8,
    /// 0.1.0
    #[serde(skip_serializing)]
    pub addr: u64,
    #[serde(skip_serializing)]
    pub name: String,
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

fn scene(id: &str, name: &str, icon: &str, mode: &str, rgb: [u8; 3], bright: u8, effect: &str, speed: u8) -> Scene {
    Scene { id: id.into(), name: name.into(), icon: icon.into(), mode: mode.into(), rgb, bright, effect: effect.into(), speed }
}

pub fn default_scenes() -> Vec<Scene> {
    vec![
        scene("evening", "Вечір", "moon", "color", [255, 120, 40], 40, "", 50),
        scene("movie", "Кіно", "film", "screen", [0, 0, 0], 35, "", 50),
        scene("stream", "Стрім", "cam", "audio", [150, 70, 255], 80, "", 50),
        scene("reading", "Читання", "book", "color", [255, 196, 140], 90, "", 50),
        scene("party", "Вечірка", "spark", "effect", [0, 0, 0], 100, "rainbow", 80),
    ]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            lamps: Vec::new(),
            active: String::new(),
            favs: ["#ff1a1a", "#ff6a1a", "#ffc400", "#18e05a", "#00c2ff", "#2a3bff", "#a100ff", "#ffffff"].map(String::from).to_vec(),
            step: 10,
            gamma: true,
            scenes: default_scenes(),
            hue_users: Map::new(),
            audio_mode: "pulse".into(),
            audio_gain: 50,
            screen_edges: false,
            screen_sat: 60,
            addr: 0,
            name: String::new(),
            rest: Map::new(),
        }
    }
}

static CFG: OnceLock<Mutex<Config>> = OnceLock::new();

fn cell() -> &'static Mutex<Config> {
    CFG.get_or_init(|| Mutex::new(load()))
}

fn migrate(c: &mut Config) {
    if c.addr != 0 && c.lamps.is_empty() {
        let id = format!("ble:{:012x}", c.addr);
        c.lamps.push(Lamp { id: id.clone(), family: "banlanx2".into(), model: "SP621E".into(), name: if c.name.is_empty() { "Торшер".into() } else { c.name.clone() }, addr: c.addr, extra: json!({}), ..Default::default() });
        c.active = id;
        // 0.1.0 favourites were pastel screen colours; keep them as they are
    }
    c.addr = 0;
    c.name.clear();
    if c.active.is_empty() {
        c.active = c.lamps.first().map(|l| l.id.clone()).unwrap_or_default();
    }
}

fn load() -> Config {
    let p = util::data_dir().join("config.json");
    let mut c = match std::fs::read_to_string(&p) {
        Ok(s) => serde_json::from_str(s.trim_start_matches('\u{feff}')).unwrap_or_else(|e| {
            logf!("config.json unreadable ({e}), using defaults");
            Config::default()
        }),
        Err(_) => Config::default(),
    };
    migrate(&mut c);
    c
}

pub fn get() -> Config {
    cell().lock().unwrap().clone()
}

pub fn lamp(id: &str) -> Option<Lamp> {
    cell().lock().unwrap().lamps.iter().find(|l| l.id == id).cloned()
}

pub fn active() -> Option<Lamp> {
    let g = cell().lock().unwrap();
    g.lamps.iter().find(|l| l.id == g.active).cloned()
}

/// Changes the settings and writes them at once (small file, rare changes).
pub fn update(f: impl FnOnce(&mut Config)) {
    let mut g = cell().lock().unwrap();
    f(&mut g);
    let p = util::data_dir().join("config.json");
    let tmp = p.with_extension("tmp");
    if let Ok(s) = serde_json::to_string_pretty(&*g)
        && std::fs::write(&tmp, s).is_ok()
    {
        let _ = std::fs::rename(&tmp, &p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_config_moves_to_the_lamp_list() {
        let mut c: Config = serde_json::from_str(r##"{"addr": 281470975594215, "name": "LEDildo", "favs": ["#ffffff"], "step": 10}"##).unwrap();
        migrate(&mut c);
        assert_eq!(c.lamps.len(), 1);
        assert_eq!(c.lamps[0].id, "ble:ffff1183cee7");
        assert_eq!(c.lamps[0].family, "banlanx2");
        assert_eq!(c.active, c.lamps[0].id);
        let v = serde_json::to_value(&c).unwrap();
        assert!(v.get("addr").is_none() && v["lamps"][0]["addr"] == 281470975594215u64);
        assert_eq!(c.scenes.len(), 5);
    }
}
