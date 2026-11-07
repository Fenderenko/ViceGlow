//! WLED (open firmware, often in DIY lamps): JSON API over HTTP, found by mDNS `_wled._tcp`.

use std::time::Duration;

use serde_json::{Value, json};

use super::{NetFound, NetProto, http, mdns};
use crate::lamp::{Caps, Op, State, byte, pct};

pub struct Wled {
    host: String,
}

impl Wled {
    pub fn new(host: &str) -> Self {
        Self { host: host.into() }
    }

    fn post(&self, v: Value) -> Result<(), String> {
        http::json(&self.host, 80, "POST", "/json/state", Some(&v)).map(|_| ())
    }
}

impl NetProto for Wled {
    fn caps(&self) -> Caps {
        Caps { state: true, color: true, fx: "dyn", speed: true, ..Default::default() }
    }

    fn apply(&mut self, op: &Op, _s: &State) -> Result<(), String> {
        match op {
            Op::Power(on) => self.post(json!({ "on": on })),
            Op::Bright(p) => self.post(json!({ "bri": byte(*p).max(1) })),
            Op::Color(c) => self.post(json!({ "seg": [{ "col": [[c[0], c[1], c[2]]], "fx": 0 }] })),
            Op::Stream(c, b) => self.post(json!({ "seg": [{ "col": [[c[0], c[1], c[2]]] }], "bri": byte(*b).max(1), "tt": 0 })),
            Op::Effect(e) => self.post(json!({ "seg": [{ "fx": e }] })),
            Op::Speed(p) => self.post(json!({ "seg": [{ "sx": byte(*p) }] })),
            _ => Ok(()),
        }
    }

    fn refresh(&mut self, s: &mut State) -> Result<bool, String> {
        let v = http::json(&self.host, 80, "GET", "/json/state", None)?;
        s.power = v["on"].as_bool().unwrap_or(false);
        s.bright = pct(v["bri"].as_u64().unwrap_or(255) as u8).max(1);
        let seg = &v["seg"][0];
        let fx = seg["fx"].as_u64().unwrap_or(0) as u16;
        s.effect = if fx == 0 { None } else { Some(fx) };
        s.speed = pct(seg["sx"].as_u64().unwrap_or(128) as u8).max(1);
        if let Some(c) = seg["col"][0].as_array() {
            let ch = |i: usize| c.get(i).and_then(Value::as_u64).unwrap_or(0) as u8;
            s.rgb = [ch(0), ch(1), ch(2)];
        }
        Ok(true)
    }

    fn effects(&mut self) -> Vec<String> {
        http::json(&self.host, 80, "GET", "/json/eff", None).ok().and_then(|v| v.as_array().map(|a| a.iter().map(|x| x.as_str().unwrap_or("").to_string()).collect())).unwrap_or_default()
    }
}

pub fn discover(out: &mut Vec<NetFound>) {
    for s in mdns::browse("_wled._tcp.local", Duration::from_millis(1500)) {
        out.push(NetFound { family: "wled", host: s.ip.to_string(), name: s.name, model: "WLED".into(), extra: json!({}) });
    }
}
