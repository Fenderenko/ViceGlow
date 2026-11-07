//! BanlanX Wi-Fi controllers (SP530E, SP538E/548E, SP539E/549E): the SPTech commands over TCP
//! 8587 with the header "SPTECH\0" + cmd, key, 00 00, length (16-bit); found by a UDP broadcast
//! to 6454 ("SpNet"). After UniLED dev_v3 (lib/net).

use std::{
    io::{Read, Write},
    net::{TcpStream, UdpSocket},
    time::{Duration, Instant},
};

use serde_json::json;

use super::{NetFound, NetProto, ble_banlanx::sptech_status};
use crate::lamp::{Caps, Op, State, byte, to_scale};

const MAGIC: &[u8; 7] = b"SPTECH\0";

pub struct BanlanxNet {
    host: String,
    sock: Option<TcpStream>,
}

impl BanlanxNet {
    pub fn new(host: &str, _code: u8) -> Self {
        Self { host: host.into(), sock: None }
    }

    fn conn(&mut self) -> Result<&mut TcpStream, String> {
        if self.sock.is_none() {
            let a = format!("{}:8587", self.host).parse().map_err(|_| "bad address".to_string())?;
            let s = TcpStream::connect_timeout(&a, Duration::from_millis(1500)).map_err(|e| e.to_string())?;
            let _ = s.set_read_timeout(Some(Duration::from_millis(1500)));
            let _ = s.set_nodelay(true);
            self.sock = Some(s);
        }
        Ok(self.sock.as_mut().unwrap())
    }

    fn send(&mut self, cmd: u8, data: &[u8]) -> Result<(), String> {
        let mut v = MAGIC.to_vec();
        v.extend([cmd, 0, 0, 0, (data.len() >> 8) as u8, data.len() as u8]);
        v.extend_from_slice(data);
        let r = self.conn()?.write_all(&v).map_err(|e| e.to_string());
        if r.is_err() {
            self.sock = None;
        }
        r
    }
}

impl NetProto for BanlanxNet {
    fn caps(&self) -> Caps {
        Caps { state: true, color: true, fx: "sptech_spi", speed: true, length: 150, cycle: true, ..Default::default() }
    }

    fn apply(&mut self, op: &Op, s: &State) -> Result<(), String> {
        let lvl = byte(s.bright.max(1));
        match op {
            Op::Power(p) => self.send(0x50, &[*p as u8]),
            Op::Bright(p) => self.send(0x51, &[0, byte(*p).max(1)]),
            Op::Color(c) => {
                if s.effect.is_some() {
                    self.send(0x53, &[1, 1])?;
                }
                self.send(0x52, &[c[0], c[1], c[2], lvl])
            }
            Op::Stream(c, b) => self.send(0x52, &[c[0], c[1], c[2], byte(*b).max(1)]),
            Op::Effect(e) => self.send(0x53, &[(*e >> 8).max(3) as u8, *e as u8]),
            Op::Speed(p) => self.send(0x54, &[to_scale(*p, 10)]),
            Op::Length(l) => self.send(0x55, &[(*l).clamp(1, 150)]),
            Op::Cycle(on) => self.send(0x58, &[*on as u8]),
            _ => Ok(()),
        }
    }

    fn refresh(&mut self, s: &mut State) -> Result<bool, String> {
        self.send(0x02, &[])?;
        let mut h = [0u8; 13];
        let st = self.conn()?;
        let r = st.read_exact(&mut h).map_err(|e| e.to_string());
        if r.is_err() {
            self.sock = None;
            r?;
        }
        let n = ((h[11] as usize) << 8) | h[12] as usize;
        let mut p = vec![0u8; n];
        self.conn()?.read_exact(&mut p).map_err(|e| e.to_string())?;
        Ok(sptech_status(&p, s))
    }

    fn close(&mut self) {
        self.sock = None;
    }
}

pub fn discover(out: &mut Vec<NetFound>) {
    let Ok(s) = UdpSocket::bind("0.0.0.0:6454").or_else(|_| UdpSocket::bind("0.0.0.0:0")) else { return };
    let _ = s.set_broadcast(true);
    let _ = s.set_read_timeout(Some(Duration::from_millis(250)));
    let msg = [0x53, 0x70, 0x4e, 0x65, 0x74, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x02, 0xe0];
    let _ = s.send_to(&msg, "255.255.255.255:6454");
    let t = Instant::now();
    let mut buf = [0u8; 512];
    while t.elapsed() < Duration::from_millis(1500) {
        let Ok((n, std::net::SocketAddr::V4(a))) = s.recv_from(&mut buf) else { continue };
        const REPLY: [u8; 14] = [0x53, 0x70, 0x4e, 0x65, 0x74, 0x00, 0x00, 0x21, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01];
        if n < REPLY.len() + 12 || buf[..REPLY.len()] != REPLY {
            continue;
        }
        let d = &buf[REPLY.len()..n];
        let code = d[3];
        let name = if d[11] > 0 { String::from_utf8_lossy(&d[12..(12 + d[11] as usize - 1).min(d.len())]).to_string() } else { String::new() };
        let model = match code {
            0x4E => "SP530E",
            0x56 => "SP538E",
            0x63 | 0x69 => "SP548E",
            0x57 => "SP539E",
            0x64 => "SP549E",
            _ => continue,
        };
        out.push(NetFound { family: "banlanx_net", host: a.ip().to_string(), name: if name.is_empty() { model.into() } else { name }, model: format!("BanlanX {model}"), extra: json!({ "code": code }) });
    }
}
