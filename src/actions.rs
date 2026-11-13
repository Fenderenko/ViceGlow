//! What the page asks for, turned into engine operations and settings; the state the page shows.

use serde_json::{Value, json};

use crate::{
    autostart, config, drv, engine, fly, live,
    lamp::{Op, Timer, from_led, to_led},
};

/// Families that take raw LED values (the gamma and white balance apply); smart lamps (Hue,
/// LIFX, WLED…) correct colours themselves.
pub fn raw_leds(family: &str) -> bool {
    !matches!(family, "govee" | "govee_lan" | "wled" | "yeelight" | "wiz" | "lifx" | "nanoleaf" | "hue")
}

/// A screen colour → what the active lamp should get.
pub fn led(c: [u8; 3]) -> [u8; 3] {
    let cfg = config::get();
    match config::active() {
        Some(l) if raw_leds(&l.family) => to_led(c, cfg.gamma, l.cal),
        _ => c,
    }
}

fn screen_color(c: [u8; 3]) -> [u8; 3] {
    let cfg = config::get();
    match config::active() {
        Some(l) if raw_leds(&l.family) => from_led(c, cfg.gamma, l.cal),
        _ => c,
    }
}

fn u8_of(m: &Value, k: &str) -> u8 {
    m.get(k).and_then(Value::as_u64).unwrap_or(0).min(255) as u8
}

fn bool_of(m: &Value, k: &str) -> bool {
    m.get(k).and_then(Value::as_bool).unwrap_or(false)
}

fn str_of<'a>(m: &'a Value, k: &str) -> &'a str {
    m.get(k).and_then(Value::as_str).unwrap_or("")
}

fn rgb_of(m: &Value, k: &str) -> Option<[u8; 3]> {
    let a = m.get(k)?.as_array()?;
    (a.len() == 3).then(|| [0, 1, 2].map(|i| a[i].as_u64().unwrap_or(0).min(255) as u8))
}

/// A message from the page (JSON object with "t").
pub fn on_page(m: &Value) {
    let t = str_of(m, "t");
    match t {
        "ready" => fly::push_state(),
        "refresh" => engine::refresh(),
        "hide" => fly::hide(),
        "hk" => {
            if let Some(on) = m.get("capture").and_then(Value::as_bool) {
                crate::hotkeys::capture(on);
            } else if bool_of(m, "reset") {
                crate::hotkeys::reset();
            } else if let Some(a) = m.get("id").and_then(Value::as_str) {
                let n = |k| m.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
                crate::hotkeys::set(a, n("mods"), n("vk"));
            }
            fly::push_state();
        }
        "power" => {
            let on = bool_of(m, "on");
            if !on {
                live::set("off");
            }
            engine::ops(vec![Op::Power(on)]);
            fly::push_state();
        }
        "bright" => {
            // a hand on the brightness takes over from the sound / screen light
            if live::active() {
                live::set("off");
                fly::push_state();
            }
            engine::ops(vec![Op::Bright(u8_of(m, "v").clamp(1, 100))]);
        }
        "color" => {
            if let Some(c) = rgb_of(m, "rgb") {
                live::set_base(c);
                if live::mode() == "audio" {
                    return; // the pulse takes the new colour by itself
                }
                live::set("off");
                engine::ops(vec![Op::Color(led(c))]);
            }
        }
        "effect" => {
            live::set("off");
            engine::ops(vec![Op::Effect(m.get("id").and_then(Value::as_u64).unwrap_or(0) as u16)]);
        }
        "speed" => engine::ops(vec![Op::Speed(u8_of(m, "v").clamp(1, 100))]),
        "length" => engine::ops(vec![Op::Length(u8_of(m, "v").max(1))]),
        "cycle" => engine::ops(vec![Op::Cycle(bool_of(m, "on"))]),
        "order" => engine::ops(vec![Op::Order(u8_of(m, "v").min(5))]),
        "timer" => {
            let tm = Timer { idx: u8_of(m, "idx"), on: bool_of(m, "on"), days: u8_of(m, "days"), sec: m.get("sec").and_then(Value::as_u64).unwrap_or(0).min(86399) as u32, enabled: bool_of(m, "enabled") };
            engine::ops(vec![Op::TimerSet(tm)]);
        }
        "timerDel" => engine::ops(vec![Op::TimerDel(u8_of(m, "idx"))]),
        "live" => {
            if let Some(c) = rgb_of(m, "rgb") {
                live::set_base(c);
            }
            live::set(str_of(m, "mode"));
            fly::push_state();
        }
        "scene" => apply_scene(str_of(m, "id")),
        "sceneSave" => {
            let st = engine::state().unwrap_or_default();
            let mode = match live::mode() {
                "off" if st.effect.is_some() => "effect",
                "off" => "color",
                l => l,
            };
            let sc = config::Scene {
                id: format!("s{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)),
                name: str_of(m, "name").chars().take(24).collect(),
                icon: str_of(m, "icon").into(),
                mode: mode.into(),
                rgb: if mode == "color" { screen_color(st.rgb) } else { rgb_of(m, "rgb").unwrap_or(st.rgb) },
                bright: st.bright.max(1),
                effect: str_of(m, "fx").into(),
                speed: st.speed.max(1),
            };
            config::update(|c| c.scenes.push(sc));
            fly::push_state();
            crate::tray::update();
        }
        "sceneDel" => {
            let id = str_of(m, "id").to_string();
            config::update(|c| c.scenes.retain(|s| s.id != id));
            fly::push_state();
        }
        "favs" => {
            let favs: Vec<String> = m.get("list").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).take(12).collect()).unwrap_or_default();
            config::update(|c| c.favs = favs);
            fly::push_state();
        }
        "settings" => {
            config::update(|c| {
                if let Some(v) = m.get("step").and_then(Value::as_u64) {
                    c.step = v.clamp(1, 50) as u8;
                }
                if let Some(v) = m.get("gamma").and_then(Value::as_bool) {
                    c.gamma = v;
                }
                if let Some(v) = m.get("audioMode").and_then(Value::as_str) {
                    c.audio_mode = v.into();
                }
                if let Some(v) = m.get("audioGain").and_then(Value::as_u64) {
                    c.audio_gain = v.min(100) as u8;
                }
                if let Some(v) = m.get("screenEdges").and_then(Value::as_bool) {
                    c.screen_edges = v;
                }
                if let Some(v) = m.get("screenSat").and_then(Value::as_u64) {
                    c.screen_sat = v.min(100) as u8;
                }
                if let Some(v) = m.get("lang").and_then(Value::as_str).filter(|v| *v == "system" || crate::util::LANGS.contains(v)) {
                    c.rest.insert("lang".into(), json!(v));
                }
                if let Some(v) = m.get("theme").and_then(Value::as_str).filter(|v| ["light", "dark", "system"].contains(v)) {
                    c.rest.insert("theme".into(), json!(v));
                }
                if let Some(v) = m.get("pinned").and_then(Value::as_bool) {
                    c.rest.insert("pinned".into(), json!(v));
                }
                if let Some(v) = m.get("fxFavs").filter(|v| v.is_array()) {
                    c.rest.insert("fxFavs".into(), v.clone());
                }
            });
            if let Some(v) = m.get("autostart").and_then(Value::as_bool) {
                autostart::set(v);
            }
            fly::push_state();
        }
        "cal" => {
            let Some(a) = m.get("gains").and_then(Value::as_array) else { return };
            let g = [0, 1, 2].map(|i| a.get(i).and_then(Value::as_f64).unwrap_or(1.0).clamp(0.2, 1.0) as f32);
            let id = engine::snapshot().lamp;
            config::update(|c| {
                if let Some(l) = c.lamps.iter_mut().find(|l| l.id == id) {
                    l.cal = g;
                }
            });
            // show a white with the new balance at once
            engine::ops(vec![Op::Color(led([255, 255, 255]))]);
            fly::push_state();
        }
        "select" => engine::select(str_of(m, "id")),
        "connect" => engine::connect(str_of(m, "id")),
        "disconnect" => engine::disconnect(str_of(m, "id")),
        "forget" => {
            let id = str_of(m, "id").to_string();
            engine::disconnect(&id);
            config::update(|c| {
                c.lamps.retain(|l| l.id != id);
                if c.active == id {
                    c.active = c.lamps.first().map(|l| l.id.clone()).unwrap_or_default();
                }
            });
            let next = config::get().active;
            engine::select(&next);
            fly::push_state();
        }
        "rename" => {
            let (id, name) = (str_of(m, "id").to_string(), str_of(m, "name").trim().chars().take(32).collect::<String>());
            if !name.is_empty() {
                let snap = engine::snapshot();
                // the lamp itself takes the name too where the protocol allows it
                if snap.lamp == id && snap.caps.rename {
                    engine::ops(vec![Op::Rename(name.clone())]);
                }
                config::update(|c| {
                    if let Some(l) = c.lamps.iter_mut().find(|l| l.id == id) {
                        l.name = name;
                    }
                });
                fly::push_state();
                crate::tray::update();
            }
        }
        "scan" => engine::scan(bool_of(m, "on")),
        "open" => {
            // only the project page
            let url = str_of(m, "url");
            if url.starts_with("https://github.com/Fenderenko/") {
                crate::util::open_url(url);
            }
        }
        _ => {}
    }
}

/// Effect tables of the page (web/fx.json): table → [{id, name, kind, …}].
fn fx_tables() -> &'static Value {
    static T: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    T.get_or_init(|| serde_json::from_str(include_str!("../web/fx.json")).unwrap_or(Value::Null))
}

/// A scene effect for the active lamp: "<table>:<id>" when the table matches, otherwise the first
/// effect of the same kind ("rainbow", "breath"…), as effects differ between families.
fn scene_effect(want: &str) -> Option<u16> {
    let table = engine::snapshot().caps.fx;
    let list = fx_tables().get(table)?.as_array()?;
    let (t, id) = want.split_once(':').unwrap_or(("", want));
    if t == table {
        return id.parse().ok();
    }
    let kind = if t.is_empty() { want } else { "rainbow" };
    list.iter().find(|e| e["kind"] == kind).or_else(|| list.iter().find(|e| e["kind"] == "rainbow")).and_then(|e| e["id"].as_u64()).map(|x| x as u16)
}

/// Restores a saved light (from the window or the tray).
pub fn apply_scene(id: &str) {
    let Some(s) = config::get().scenes.into_iter().find(|s| s.id == id) else { return };
    live::set_base(s.rgb);
    let mut ops = vec![Op::Power(true), Op::Bright(s.bright.clamp(1, 100))];
    match s.mode.as_str() {
        "audio" | "screen" => live::set(&s.mode),
        "effect" => {
            live::set("off");
            if let Some(e) = scene_effect(&s.effect) {
                ops.push(Op::Effect(e));
                ops.push(Op::Speed(s.speed.clamp(1, 100)));
            }
        }
        _ => {
            live::set("off");
            ops.push(Op::Color(led(s.rgb)));
        }
    }
    engine::ops(ops);
    fly::push_state();
}

/// The page state: lamps, the active lamp, settings.
pub fn state_json() -> String {
    let s = engine::snapshot();
    let c = config::get();
    let found = engine::found();
    let lamps: Vec<Value> = c
        .lamps
        .iter()
        .map(|l| {
            let f = found.iter().find(|f| f.id == l.id);
            json!({
                "id": l.id, "name": l.name, "model": l.model, "family": l.family,
                "net": if l.is_ble() { "ble" } else { "wifi" },
                "paused": l.paused, "active": l.id == s.lamp,
                "near": f.is_some(), "rssi": f.and_then(|f| f.rssi),
                "verified": drv::verified(&l.family, &l.model),
                "cal": l.cal,
            })
        })
        .collect();
    let found: Vec<Value> = found
        .iter()
        .filter(|f| !c.lamps.iter().any(|l| l.id == f.id))
        .map(|f| json!({ "id": f.id, "name": f.name, "model": f.model, "family": f.family, "net": f.net, "rssi": f.rssi, "verified": drv::verified(f.family, &f.model) }))
        .collect();
    let lamp_state = s.state.map(|mut st| {
        st.rgb = screen_color(st.rgb);
        st
    });
    json!({
        "t": "state",
        "ver": env!("CARGO_PKG_VERSION"),
        "lamp": s.lamp,
        "lamps": lamps,
        "found": found,
        "conn": s.conn,
        "err": s.err,
        "caps": s.caps,
        "fxdyn": s.fxdyn,
        "scanning": s.scanning,
        "st": lamp_state,
        "live": live::mode(),
        "cfg": {
            "favs": c.favs, "step": c.step, "autostart": autostart::enabled(), "gamma": c.gamma,
            "scenes": c.scenes, "audioMode": c.audio_mode, "audioGain": c.audio_gain,
            "screenEdges": c.screen_edges, "screenSat": c.screen_sat,
            "fxFavs": c.rest.get("fxFavs").cloned().unwrap_or(json!([])),
            "theme": c.rest.get("theme").cloned().unwrap_or(json!("light")),
            "lang": c.rest.get("lang").cloned().unwrap_or(json!("en")),
            "sysLang": crate::util::system_lang(),
            "pinned": c.rest.get("pinned").cloned().unwrap_or(json!(false)),
            "hotkeys": crate::hotkeys::state(),
        },
    })
    .to_string()
}
