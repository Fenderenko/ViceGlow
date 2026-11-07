//! LIFX: the official binary LAN protocol over UDP 56700 (36-byte header, little-endian).
//! GetService (2) broadcast → StateService (3) gives the target; SetPower (117), SetColor (102),
//! Get (101) → State (107) with HSBK.

use std::{
    net::UdpSocket,
    time::{Duration, Instant},
};

use serde_json::json;

use super::{NetFound, NetProto};
use crate::lamp::{Caps, Op, State, from_hsv, hsv};

const SOURCE: u32 = 0x474C_4F57; // "GLOW"

fn packet(target: [u8; 8], tagged: bool, seq: u8, typ: u16, payload: &[u8], res: bool) -> Vec<u8> {
    let size = 36 + payload.len();
    let mut v = Vec::with_capacity(size);
    v.extend((size as u16).to_le_bytes());
    let proto: u16 = 1024 | 0x1000 | if tagged { 0x2000 } else { 0 };
    v.extend(proto.to_le_bytes());
    v.extend(SOURCE.to_le_bytes());
    v.extend(target);
    v.extend([0u8; 6]);
    v.push(if res { 0x01 } else { 0x00 });
    v.push(seq);
    v.extend([0u8; 8]);
    v.extend(typ.to_le_bytes());
    v.extend([0u8; 2]);
    v.extend_from_slice(payload);
    v
}

fn mac_bytes(mac: &str) -> [u8; 8] {
    let mut t = [0u8; 8];
    for (i, h) in mac.split(':').take(6).enumerate() {
        t[i] = u8::from_str_radix(h, 16).unwrap_or(0);
    }
    t
}

pub struct Lifx {
    host: String,
    target: [u8; 8],
    seq: u8,
    sock: Option<UdpSocket>,
}

impl Lifx {
    pub fn new(host: &str, mac: &str) -> Self {
        Self { host: host.into(), target: mac_bytes(mac), seq: 0, sock: None }
    }

    fn send(&mut self, typ: u16, payload: &[u8], res: bool) -> Result<(), String> {
        if self.sock.is_none() {
            let s = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
            let _ = s.set_read_timeout(Some(Duration::from_millis(800)));
            self.sock = Some(s);
        }
        self.seq = self.seq.wrapping_add(1);
        let p = packet(self.target, false, self.seq, typ, payload, res);
        self.sock.as_ref().unwrap().send_to(&p, format!("{}:56700", self.host)).map(|_| ()).map_err(|e| e.to_string())
    }

    fn color(&mut self, c: [u8; 3], bright: u8, ms: u32) -> Result<(), String> {
        let (h, s, _) = hsv(c);
        let mut p = vec![0u8];
        for v in [(h / 360.0 * 65535.0) as u16, (s * 65535.0) as u16, (bright.clamp(1, 100) as u32 * 65535 / 100) as u16, 3500u16] {
            p.extend(v.to_le_bytes());
        }
        p.extend(ms.to_le_bytes());
        self.send(102, &p, false)
    }
}

impl NetProto for Lifx {
    fn caps(&self) -> Caps {
        Caps { state: true, color: true, ..Default::default() }
    }

    fn apply(&mut self, op: &Op, s: &State) -> Result<(), String> {
        match op {
            Op::Power(on) => {
                let mut p = (if *on { 65535u16 } else { 0 }).to_le_bytes().to_vec();
                p.extend(300u32.to_le_bytes());
                self.send(117, &p, false)
            }
            Op::Bright(b) => self.color(s.rgb, *b, 150),
            Op::Color(c) => self.color(*c, s.bright, 150),
            Op::Stream(c, b) => self.color(*c, *b, 0),
            _ => Ok(()),
        }
    }

    fn refresh(&mut self, st: &mut State) -> Result<bool, String> {
        self.send(101, &[], true)?;
        let mut buf = [0u8; 256];
        let t = Instant::now();
        while t.elapsed() < Duration::from_millis(900) {
            let (n, _) = self.sock.as_ref().unwrap().recv_from(&mut buf).map_err(|e| e.to_string())?;
            if n >= 36 + 52 && u16::from_le_bytes([buf[32], buf[33]]) == 107 {
                let p = &buf[36..n];
                let u = |i: usize| u16::from_le_bytes([p[i], p[i + 1]]) as f32 / 65535.0;
                st.rgb = from_hsv(u(0) * 360.0, u(2), 1.0);
                st.bright = ((u(4) * 100.0).round() as u8).max(1);
                st.power = u16::from_le_bytes([p[10], p[11]]) > 0;
                st.effect = None;
                return Ok(true);
            }
        }
        Err("no answer".into())
    }

    fn close(&mut self) {
        self.sock = None;
    }
}

pub fn discover(out: &mut Vec<NetFound>) {
    let Ok(s) = UdpSocket::bind("0.0.0.0:0") else { return };
    let _ = s.set_broadcast(true);
    let _ = s.set_read_timeout(Some(Duration::from_millis(250)));
    let _ = s.send_to(&packet([0; 8], true, 1, 2, &[], true), "255.255.255.255:56700");
    let t = Instant::now();
    let mut buf = [0u8; 256];
    while t.elapsed() < Duration::from_millis(1500) {
        let Ok((n, std::net::SocketAddr::V4(a))) = s.recv_from(&mut buf) else { continue };
        if n < 36 || u16::from_le_bytes([buf[32], buf[33]]) != 3 {
            continue;
        }
        let mac: Vec<String> = buf[8..14].iter().map(|b| format!("{b:02x}")).collect();
        let host = a.ip().to_string();
        if !out.iter().any(|f| f.host == host) {
            out.push(NetFound { family: "lifx", host, name: format!("LIFX {}", mac[4..].join("")), model: "LIFX".into(), extra: json!({ "mac": mac.join(":") }) });
        }
    }
}
