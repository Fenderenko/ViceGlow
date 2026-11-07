//! Philips Hue through the bridge (local API v1 over HTTP). The bridge is found by mDNS
//! `_hue._tcp`; a user name comes from pairing (press the bridge button, then POST /api). Every
//! colour light of a paired bridge is a lamp of its own.

use std::time::Duration;

use serde_json::{Value, json};

use super::{NetFound, NetProto, http, mdns};
use crate::lamp::{Caps, Op, State};

pub fn pair(host: &str) -> Result<String, String> {
    let v = http::json(host, 80, "POST", "/api", Some(&json!({ "devicetype": "viceglow#pc" })))?;
    if let Some(u) = v[0]["success"]["username"].as_str() {
        return Ok(u.into());
    }
    if v[0]["error"]["type"] == 101 {
        return Err("pair".into());
    }
    Err(v[0]["error"]["description"].as_str().unwrap_or("bridge refused").into())
}

/// sRGB → CIE xy (Wide gamut, as Philips documents for its lamps).
fn xy(c: [u8; 3]) -> [f64; 2] {
    let lin = |v: u8| {
        let v = v as f64 / 255.0;
        if v > 0.04045 { ((v + 0.055) / 1.055).powf(2.4) } else { v / 12.92 }
    };
    let (r, g, b) = (lin(c[0]), lin(c[1]), lin(c[2]));
    let x = r * 0.664511 + g * 0.154324 + b * 0.162028;
    let y = r * 0.283881 + g * 0.668433 + b * 0.047685;
    let z = r * 0.000088 + g * 0.072310 + b * 0.986039;
    let s = x + y + z;
    if s == 0.0 { [0.3227, 0.329] } else { [x / s, y / s] }
}

fn from_xy(x: f64, y: f64) -> [u8; 3] {
    let yy = 1.0;
    let xx = yy / y.max(0.0001) * x;
    let zz = yy / y.max(0.0001) * (1.0 - x - y);
    let r = xx * 1.656492 - yy * 0.354851 - zz * 0.255038;
    let g = -xx * 0.707196 + yy * 1.655397 + zz * 0.036152;
    let b = xx * 0.051713 - yy * 0.121364 + zz * 1.011530;
    let m = r.max(g).max(b).max(1e-6);
    let gam = |v: f64| {
        let v = (v / m).max(0.0);
        let v = if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    [gam(r), gam(g), gam(b)]
}

pub struct Hue {
    host: String,
    user: String,
    light: String,
}

impl Hue {
    pub fn new(host: &str, user: &str, light: &str) -> Self {
        Self { host: host.into(), user: user.into(), light: light.into() }
    }

    fn set(&self, v: Value) -> Result<(), String> {
        http::json(&self.host, 80, "PUT", &format!("/api/{}/lights/{}/state", self.user, self.light), Some(&v)).map(|_| ())
    }
}

impl NetProto for Hue {
    fn caps(&self) -> Caps {
        Caps { state: true, color: true, ..Default::default() }
    }

    fn apply(&mut self, op: &Op, _s: &State) -> Result<(), String> {
        let bri = |p: u8| (p.clamp(1, 100) as u32 * 254 / 100).max(1);
        match op {
            Op::Power(on) => self.set(json!({ "on": on })),
            Op::Bright(p) => self.set(json!({ "bri": bri(*p) })),
            Op::Color(c) => self.set(json!({ "xy": xy(*c) })),
            Op::Stream(c, b) => self.set(json!({ "xy": xy(*c), "bri": bri(*b), "transitiontime": 1 })),
            _ => Ok(()),
        }
    }

    fn refresh(&mut self, s: &mut State) -> Result<bool, String> {
        let v = http::json(&self.host, 80, "GET", &format!("/api/{}/lights/{}", self.user, self.light), None)?;
        let st = &v["state"];
        if st.is_null() {
            return Err(v[0]["error"]["description"].as_str().unwrap_or("light not found").into());
        }
        s.power = st["on"].as_bool().unwrap_or(false);
        s.bright = ((st["bri"].as_u64().unwrap_or(254) * 100 / 254) as u8).max(1);
        if let (Some(x), Some(y)) = (st["xy"][0].as_f64(), st["xy"][1].as_f64()) {
            s.rgb = from_xy(x, y);
        }
        s.effect = None;
        Ok(true)
    }
}

/// Bridges on the network; with a saved user name also their colour lights.
pub fn discover(out: &mut Vec<NetFound>, users: &serde_json::Map<String, Value>) {
    for b in mdns::browse("_hue._tcp.local", Duration::from_millis(1500)) {
        let host = b.ip.to_string();
        match users.get(&host).and_then(Value::as_str) {
            Some(u) => {
                let Ok(v) = http::json(&host, 80, "GET", &format!("/api/{u}/lights"), None) else { continue };
                let Some(m) = v.as_object() else { continue };
                for (id, l) in m {
                    if l["state"].get("xy").is_none() {
                        continue; // white-only lights
                    }
                    let name = l["name"].as_str().unwrap_or("Hue").to_string();
                    out.push(NetFound { family: "hue", host: host.clone(), name, model: format!("Philips Hue {}", l["modelid"].as_str().unwrap_or("")), extra: json!({ "user": u, "light": id }) });
                }
            }
            None => out.push(NetFound { family: "hue_bridge", host, name: b.name, model: "Philips Hue Bridge".into(), extra: json!({}) }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xy_round_trip() {
        let c = from_xy(xy([255, 0, 0])[0], xy([255, 0, 0])[1]);
        assert!(c[0] > 240 && c[1] < 20 && c[2] < 20);
    }
}
