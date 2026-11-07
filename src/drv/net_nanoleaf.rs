//! Nanoleaf panels: the local OpenAPI on port 16021 (found by mDNS `_nanoleafapi._tcp`). A token
//! comes from pairing: hold the power button 5–7 s, then POST /api/v1/new within 30 s.

use std::time::Duration;

use serde_json::{Value, json};

use super::{NetFound, NetProto, http, mdns};
use crate::lamp::{Caps, Op, State, from_hsv, hsv};

const PORT: u16 = 16021;

pub struct Nanoleaf {
    host: String,
    token: String,
    names: Vec<String>,
}

pub fn pair(host: &str) -> Result<String, String> {
    match http::json(host, PORT, "POST", "/api/v1/new", None) {
        Ok(v) => v["auth_token"].as_str().map(String::from).ok_or_else(|| "no token".into()),
        Err(e) if e.contains("403") => Err("pair".into()),
        Err(e) => Err(e),
    }
}

impl Nanoleaf {
    pub fn new(host: &str, token: &str) -> Self {
        Self { host: host.into(), token: token.into(), names: Vec::new() }
    }

    fn put(&self, path: &str, v: Value) -> Result<(), String> {
        http::json(&self.host, PORT, "PUT", &format!("/api/v1/{}/{path}", self.token), Some(&v)).map(|_| ())
    }

    fn color(&self, c: [u8; 3], bright: u8) -> Result<(), String> {
        let (h, s, _) = hsv(c);
        self.put("state", json!({ "hue": { "value": h as u32 }, "sat": { "value": (s * 100.0) as u32 }, "brightness": { "value": bright.clamp(1, 100), "duration": 0 } }))
    }
}

impl NetProto for Nanoleaf {
    fn caps(&self) -> Caps {
        Caps { state: true, color: true, fx: "dyn", ..Default::default() }
    }

    fn apply(&mut self, op: &Op, s: &State) -> Result<(), String> {
        match op {
            Op::Power(on) => self.put("state", json!({ "on": { "value": on } })),
            Op::Bright(b) => self.put("state", json!({ "brightness": { "value": (*b).clamp(1, 100), "duration": 0 } })),
            Op::Color(c) => self.color(*c, s.bright),
            Op::Stream(c, b) => self.color(*c, *b),
            Op::Effect(e) => match self.names.get(*e as usize) {
                Some(n) => self.put("effects", json!({ "select": n })),
                None => Ok(()),
            },
            _ => Ok(()),
        }
    }

    fn refresh(&mut self, s: &mut State) -> Result<bool, String> {
        let v = http::json(&self.host, PORT, "GET", &format!("/api/v1/{}/", self.token), None)?;
        let st = &v["state"];
        s.power = st["on"]["value"].as_bool().unwrap_or(false);
        s.bright = st["brightness"]["value"].as_u64().unwrap_or(100) as u8;
        let h = st["hue"]["value"].as_f64().unwrap_or(0.0) as f32;
        let sat = st["sat"]["value"].as_f64().unwrap_or(0.0) as f32 / 100.0;
        s.rgb = from_hsv(h, sat, 1.0);
        let sel = v["effects"]["select"].as_str().unwrap_or("");
        if let Some(list) = v["effects"]["effectsList"].as_array() {
            self.names = list.iter().filter_map(|x| x.as_str().map(String::from)).collect();
        }
        s.effect = if st["colorMode"] == "effect" { self.names.iter().position(|n| n == sel).map(|i| i as u16) } else { None };
        Ok(true)
    }

    fn effects(&mut self) -> Vec<String> {
        if self.names.is_empty()
            && let Ok(v) = http::json(&self.host, PORT, "GET", &format!("/api/v1/{}/effects/effectsList", self.token), None)
        {
            self.names = v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        }
        self.names.clone()
    }
}

pub fn discover(out: &mut Vec<NetFound>) {
    for s in mdns::browse("_nanoleafapi._tcp.local", Duration::from_millis(1500)) {
        out.push(NetFound { family: "nanoleaf", host: s.ip.to_string(), name: s.name, model: "Nanoleaf".into(), extra: json!({}) });
    }
}
