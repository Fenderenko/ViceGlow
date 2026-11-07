//! Govee Wi-Fi lights with "LAN Control" switched on in the Govee app: JSON over UDP, scan to
//! 239.255.255.250:4001 (answers to port 4002), commands to the lamp's port 4003. The window
//! shows what was last sent (state answers would need port 4002 kept open).

use std::{
    net::{Ipv4Addr, UdpSocket},
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use super::{NetFound, NetProto};
use crate::lamp::{Caps, Op, State};

pub struct GoveeLan {
    host: String,
    sock: Option<UdpSocket>,
}

impl GoveeLan {
    pub fn new(host: &str) -> Self {
        Self { host: host.into(), sock: None }
    }

    fn send(&mut self, cmd: &str, data: Value) -> Result<(), String> {
        if self.sock.is_none() {
            self.sock = Some(UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?);
        }
        let m = json!({ "msg": { "cmd": cmd, "data": data } }).to_string();
        self.sock.as_ref().unwrap().send_to(m.as_bytes(), format!("{}:4003", self.host)).map(|_| ()).map_err(|e| e.to_string())
    }
}

impl NetProto for GoveeLan {
    fn caps(&self) -> Caps {
        Caps { color: true, ..Default::default() }
    }

    fn apply(&mut self, op: &Op, s: &State) -> Result<(), String> {
        let col = |c: [u8; 3]| json!({ "color": { "r": c[0], "g": c[1], "b": c[2] }, "colorTemInKelvin": 0 });
        match op {
            Op::Power(on) => self.send("turn", json!({ "value": *on as u8 })),
            Op::Bright(p) => self.send("brightness", json!({ "value": (*p).clamp(1, 100) })),
            Op::Color(c) => self.send("colorwc", col(*c)),
            Op::Stream(c, b) => {
                self.send("colorwc", col(*c))?;
                if *b != s.bright { self.send("brightness", json!({ "value": (*b).clamp(1, 100) })) } else { Ok(()) }
            }
            _ => Ok(()),
        }
    }

    fn refresh(&mut self, _s: &mut State) -> Result<bool, String> {
        Ok(false)
    }
}

pub fn discover(out: &mut Vec<NetFound>) {
    let Ok(s) = UdpSocket::bind("0.0.0.0:4002") else { return };
    let _ = s.set_read_timeout(Some(Duration::from_millis(250)));
    let _ = s.join_multicast_v4(&Ipv4Addr::new(239, 255, 255, 250), &Ipv4Addr::UNSPECIFIED);
    let scan = json!({ "msg": { "cmd": "scan", "data": { "account_topic": "reserve" } } }).to_string();
    let _ = s.send_to(scan.as_bytes(), "239.255.255.250:4001");
    let t = Instant::now();
    let mut buf = [0u8; 2048];
    while t.elapsed() < Duration::from_millis(1500) {
        let Ok((n, _)) = s.recv_from(&mut buf) else { continue };
        let Ok(v) = serde_json::from_slice::<Value>(&buf[..n]) else { continue };
        let d = &v["msg"]["data"];
        if v["msg"]["cmd"] == "scan"
            && let Some(ip) = d["ip"].as_str()
        {
            let sku = d["sku"].as_str().unwrap_or("Govee");
            out.push(NetFound { family: "govee_lan", host: ip.into(), name: format!("Govee {sku}"), model: format!("Govee {sku}"), extra: json!({ "device": d["device"] }) });
        }
    }
}
