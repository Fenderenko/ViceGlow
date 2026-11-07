//! WiZ bulbs and lamps: JSON over UDP 38899 (setPilot / getPilot), found by a broadcast getPilot.

use std::{
    net::UdpSocket,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use super::{NetFound, NetProto};
use crate::lamp::{Caps, Op, State};

pub struct Wiz {
    host: String,
    sock: Option<UdpSocket>,
}

impl Wiz {
    pub fn new(host: &str) -> Self {
        Self { host: host.into(), sock: None }
    }

    fn call(&mut self, method: &str, params: Value, answer: bool) -> Result<Value, String> {
        if self.sock.is_none() {
            let s = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
            let _ = s.set_read_timeout(Some(Duration::from_millis(800)));
            self.sock = Some(s);
        }
        let s = self.sock.as_ref().unwrap();
        let m = json!({ "method": method, "params": params }).to_string();
        s.send_to(m.as_bytes(), format!("{}:38899", self.host)).map_err(|e| e.to_string())?;
        if !answer {
            return Ok(Value::Null);
        }
        let mut buf = [0u8; 1024];
        let (n, _) = s.recv_from(&mut buf).map_err(|e| e.to_string())?;
        let v: Value = serde_json::from_slice(&buf[..n]).map_err(|e| e.to_string())?;
        Ok(v["result"].clone())
    }
}

impl NetProto for Wiz {
    fn caps(&self) -> Caps {
        Caps { state: true, color: true, fx: "wiz", speed: true, ..Default::default() }
    }

    fn apply(&mut self, op: &Op, s: &State) -> Result<(), String> {
        let dim = |p: u8| p.clamp(10, 100);
        let p = match op {
            Op::Power(on) => json!({ "state": on }),
            Op::Bright(b) => json!({ "dimming": dim(*b) }),
            Op::Color(c) => json!({ "r": c[0], "g": c[1], "b": c[2], "dimming": dim(s.bright) }),
            Op::Stream(c, b) => json!({ "r": c[0], "g": c[1], "b": c[2], "dimming": dim(*b) }),
            Op::Effect(e) => json!({ "sceneId": e, "speed": 20 + s.speed as u32 * 180 / 100 }),
            Op::Speed(v) => match s.effect {
                Some(e) => json!({ "sceneId": e, "speed": 20 + *v as u32 * 180 / 100 }),
                None => return Ok(()),
            },
            _ => return Ok(()),
        };
        self.call("setPilot", p, false).map(|_| ())
    }

    fn refresh(&mut self, s: &mut State) -> Result<bool, String> {
        let r = self.call("getPilot", json!({}), true)?;
        s.power = r["state"].as_bool().unwrap_or(false);
        s.bright = r["dimming"].as_u64().unwrap_or(100) as u8;
        let scene = r["sceneId"].as_u64().unwrap_or(0) as u16;
        s.effect = if scene == 0 { None } else { Some(scene) };
        if r.get("r").is_some() {
            let c = |k: &str| r[k].as_u64().unwrap_or(0) as u8;
            s.rgb = [c("r"), c("g"), c("b")];
        }
        Ok(true)
    }

    fn close(&mut self) {
        self.sock = None;
    }
}

pub fn discover(out: &mut Vec<NetFound>) {
    let Ok(s) = UdpSocket::bind("0.0.0.0:0") else { return };
    let _ = s.set_broadcast(true);
    let _ = s.set_read_timeout(Some(Duration::from_millis(250)));
    let _ = s.send_to(br#"{"method":"getPilot","params":{}}"#, "255.255.255.255:38899");
    let t = Instant::now();
    let mut buf = [0u8; 1024];
    while t.elapsed() < Duration::from_millis(1500) {
        let Ok((n, std::net::SocketAddr::V4(a))) = s.recv_from(&mut buf) else { continue };
        let Ok(v) = serde_json::from_slice::<Value>(&buf[..n]) else { continue };
        let mac = v["result"]["mac"].as_str().unwrap_or("");
        if v["method"] == "getPilot" && !mac.is_empty() && !out.iter().any(|f| f.host == a.ip().to_string()) {
            out.push(NetFound { family: "wiz", host: a.ip().to_string(), name: format!("WiZ {}", &mac[mac.len().saturating_sub(4)..]), model: "WiZ".into(), extra: json!({ "mac": mac }) });
        }
    }
}
