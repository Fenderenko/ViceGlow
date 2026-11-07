//! Yeelight bulbs and lamps with "LAN Control" on: JSON lines over TCP 55443, found by an SSDP
//! search on 239.255.255.250:1982. The bulb takes about 60 commands a minute, so live colour
//! (sound, screen) is sent at most once a second.

use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpStream, UdpSocket},
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use super::{NetFound, NetProto};
use crate::lamp::{Caps, Op, State};

pub struct Yeelight {
    host: String,
    sock: Option<TcpStream>,
    id: u32,
    last_stream: Option<Instant>,
}

impl Yeelight {
    pub fn new(host: &str) -> Self {
        Self { host: host.into(), sock: None, id: 0, last_stream: None }
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        if self.sock.is_none() {
            let a = format!("{}:55443", self.host).parse().map_err(|_| "bad address".to_string())?;
            let s = TcpStream::connect_timeout(&a, Duration::from_millis(1500)).map_err(|e| e.to_string())?;
            let _ = s.set_read_timeout(Some(Duration::from_millis(1500)));
            self.sock = Some(s);
        }
        self.id = self.id.wrapping_add(1);
        let line = json!({ "id": self.id, "method": method, "params": params }).to_string() + "\r\n";
        let s = self.sock.as_mut().unwrap();
        if let Err(e) = s.write_all(line.as_bytes()) {
            self.sock = None;
            return Err(e.to_string());
        }
        let mut r = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
        // skip "props" notifications until the answer with our id
        for _ in 0..8 {
            let mut l = String::new();
            if r.read_line(&mut l).map_err(|e| e.to_string())? == 0 {
                self.sock = None;
                return Err("closed".into());
            }
            if let Ok(v) = serde_json::from_str::<Value>(&l)
                && v["id"] == self.id
            {
                return if v.get("error").is_some() { Err(v["error"]["message"].as_str().unwrap_or("error").into()) } else { Ok(v["result"].clone()) };
            }
        }
        Err("no answer".into())
    }
}

impl NetProto for Yeelight {
    fn caps(&self) -> Caps {
        Caps { state: true, color: true, ..Default::default() }
    }

    fn apply(&mut self, op: &Op, s: &State) -> Result<(), String> {
        let rgb = |c: [u8; 3]| ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32;
        match op {
            Op::Power(on) => self.call("set_power", json!([if *on { "on" } else { "off" }, "smooth", 300])).map(|_| ()),
            Op::Bright(p) => self.call("set_bright", json!([(*p).clamp(1, 100), "smooth", 200])).map(|_| ()),
            Op::Color(c) => self.call("set_rgb", json!([rgb(*c).max(1), "smooth", 200])).map(|_| ()),
            Op::Stream(c, b) => {
                if self.last_stream.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
                    return Ok(());
                }
                self.last_stream = Some(Instant::now());
                self.call("set_rgb", json!([rgb(*c).max(1), "smooth", 500]))?;
                if *b != s.bright { self.call("set_bright", json!([(*b).clamp(1, 100), "smooth", 500])).map(|_| ()) } else { Ok(()) }
            }
            _ => Ok(()),
        }
    }

    fn refresh(&mut self, s: &mut State) -> Result<bool, String> {
        let r = self.call("get_prop", json!(["power", "bright", "rgb"]))?;
        let g = |i: usize| r[i].as_str().unwrap_or("").to_string();
        s.power = g(0) == "on";
        s.bright = g(1).parse().unwrap_or(100);
        let c: u32 = g(2).parse().unwrap_or(0xFFFFFF);
        s.rgb = [(c >> 16) as u8, (c >> 8) as u8, c as u8];
        s.effect = None;
        Ok(true)
    }

    fn close(&mut self) {
        self.sock = None;
    }
}

pub fn discover(out: &mut Vec<NetFound>) {
    let Ok(s) = UdpSocket::bind("0.0.0.0:0") else { return };
    let _ = s.set_read_timeout(Some(Duration::from_millis(250)));
    let q = "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1982\r\nMAN: \"ssdp:discover\"\r\nST: wifi_bulb\r\n";
    let _ = s.send_to(q.as_bytes(), "239.255.255.250:1982");
    let t = Instant::now();
    let mut buf = [0u8; 2048];
    while t.elapsed() < Duration::from_millis(1500) {
        let Ok((n, _)) = s.recv_from(&mut buf) else { continue };
        let txt = String::from_utf8_lossy(&buf[..n]).to_string();
        let h = |k: &str| txt.lines().find_map(|l| l.split_once(':').filter(|x| x.0.trim().eq_ignore_ascii_case(k)).map(|x| x.1.trim().to_string())).unwrap_or_default();
        let loc = h("Location");
        let Some(host) = loc.strip_prefix("yeelight://").and_then(|x| x.split(':').next()) else { continue };
        if out.iter().any(|f| f.host == host) {
            continue;
        }
        let model = h("model");
        let name = h("name");
        out.push(NetFound { family: "yeelight", host: host.into(), name: if name.is_empty() { format!("Yeelight {model}") } else { name }, model: format!("Yeelight {model}"), extra: json!({ "id": h("id") }) });
    }
}
