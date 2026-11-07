//! Magic Home / Zengge Wi-Fi controllers (flux_led "LEDENET" protocol): TCP 5577, frames with a
//! sum byte; found by the UDP broadcast "HF-A11ASSISTHREAD" on 48899.

use std::{
    io::{Read, Write},
    net::{TcpStream, UdpSocket},
    time::{Duration, Instant},
};

use serde_json::json;

use super::{NetFound, NetProto, ble_triones::{delay, scale, undelay}};
use crate::lamp::{Caps, Op, State};

pub struct MagicHome {
    host: String,
    sock: Option<TcpStream>,
    base: [u8; 3],
}

fn sum(mut v: Vec<u8>) -> Vec<u8> {
    let s = v.iter().fold(0u8, |a, b| a.wrapping_add(*b));
    v.push(s);
    v
}

impl MagicHome {
    pub fn new(host: &str) -> Self {
        Self { host: host.into(), sock: None, base: [255, 255, 255] }
    }

    fn conn(&mut self) -> Result<&mut TcpStream, String> {
        if self.sock.is_none() {
            let a = format!("{}:5577", self.host).parse().map_err(|_| "bad address".to_string())?;
            let s = TcpStream::connect_timeout(&a, Duration::from_millis(1500)).map_err(|e| e.to_string())?;
            let _ = s.set_read_timeout(Some(Duration::from_millis(1000)));
            let _ = s.set_nodelay(true);
            self.sock = Some(s);
        }
        Ok(self.sock.as_mut().unwrap())
    }

    fn send(&mut self, f: Vec<u8>) -> Result<(), String> {
        let r = self.conn()?.write_all(&sum(f)).map_err(|e| e.to_string());
        if r.is_err() {
            self.sock = None;
        }
        r
    }
}

impl NetProto for MagicHome {
    fn caps(&self) -> Caps {
        Caps { state: true, color: true, fx: "triones", speed: true, ..Default::default() }
    }

    fn apply(&mut self, op: &Op, s: &State) -> Result<(), String> {
        let col = |c: [u8; 3]| vec![0x31, c[0], c[1], c[2], 0x00, 0xF0, 0x0F];
        match op {
            Op::Power(on) => self.send(vec![0x71, if *on { 0x23 } else { 0x24 }, 0x0F]),
            Op::Color(c) => {
                self.base = *c;
                self.send(col(scale(*c, s.bright)))
            }
            Op::Stream(c, b) => self.send(col(scale(*c, *b))),
            Op::Bright(p) if s.effect.is_none() => self.send(col(scale(self.base, *p))),
            Op::Effect(e) => self.send(vec![0x61, *e as u8, delay(s.speed.max(1)), 0x0F]),
            Op::Speed(v) => match s.effect {
                Some(e) => self.send(vec![0x61, e as u8, delay(*v), 0x0F]),
                None => Ok(()),
            },
            _ => Ok(()),
        }
    }

    fn refresh(&mut self, s: &mut State) -> Result<bool, String> {
        self.send(vec![0x81, 0x8A, 0x8B])?;
        let mut b = [0u8; 14];
        let r = self.conn()?.read_exact(&mut b).map_err(|e| e.to_string());
        if r.is_err() {
            self.sock = None;
            r?;
        }
        if b[0] != 0x81 {
            return Ok(false);
        }
        s.power = b[2] == 0x23;
        if (0x25..=0x38).contains(&b[3]) {
            s.effect = Some(b[3] as u16);
            s.speed = undelay(b[5]);
        } else {
            s.effect = None;
            let rgb = [b[6], b[7], b[8]];
            let m = *rgb.iter().max().unwrap();
            if m > 0 {
                s.bright = ((m as u32 * 100 + 127) / 255).max(1) as u8;
                s.rgb = rgb.map(|x| (x as u32 * 255 / m as u32) as u8);
                self.base = s.rgb;
            }
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
    let _ = s.send_to(b"HF-A11ASSISTHREAD", "255.255.255.255:48899");
    let t = Instant::now();
    let mut buf = [0u8; 256];
    while t.elapsed() < Duration::from_millis(1500) {
        if let Ok((n, _)) = s.recv_from(&mut buf) {
            let txt = String::from_utf8_lossy(&buf[..n]).to_string();
            let p: Vec<&str> = txt.split(',').collect();
            if p.len() >= 3 && p[0].parse::<std::net::Ipv4Addr>().is_ok() {
                out.push(NetFound { family: "magichome", host: p[0].into(), name: format!("Magic Home {}", &p[1][p[1].len().saturating_sub(4)..]), model: format!("Magic Home {}", p[2]), extra: json!({}) });
            }
        }
    }
}
