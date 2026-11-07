//! BanlanX controllers over Bluetooth (service FFE0, characteristic FFE1, write with response).
//! The model is the first byte of the manufacturer data (company 0x5053). Protocols after UniLED
//! dev_v3 (github.com/monty68/uniled): banlanx2 (SP611E/617E/620E/621E — the author's lamp, checked
//! live), banlanx3 (SP613E/614E/623E/624E), SP110E, SP107E, SP601E, SP602E/608E and SPTech
//! (SP630E…SP64CE, `53` frames).

use super::{BleProto, Frame, Gatt, Ident, u16id, w};
use crate::{
    lamp::{Caps, Op, State, byte, from_scale, pct, to_scale},
    proto,
};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Fam {
    B2,
    B3,
    Sp110,
    Sp107,
    Sp601,
    Sp60x,
    SpTech,
}

const SPTECH: [(u8, &str); 27] = [
    (0x1F, "SP630E"),
    (0x20, "SP631E"),
    (0x2C, "SP641E"),
    (0x21, "SP632E"),
    (0x2D, "SP642E"),
    (0x4A, "SP642E"),
    (0x22, "SP633E"),
    (0x2E, "SP643E"),
    (0x23, "SP634E"),
    (0x2F, "SP644E"),
    (0x24, "SP635E"),
    (0x30, "SP645E"),
    (0x25, "SP636E"),
    (0x31, "SP646E"),
    (0x26, "SP637E"),
    (0x32, "SP647E"),
    (0x27, "SP638E"),
    (0x33, "SP648E"),
    (0x45, "SP648E"),
    (0x4C, "SP648E"),
    (0x68, "SP648E"),
    (0x28, "SP639E"),
    (0x34, "SP649E"),
    (0x29, "SP63AE"),
    (0x35, "SP64AE"),
    (0x2A, "SP63BE"),
    (0x2B, "SP63CE"),
];

/// Models of the SPTech family driven by PWM (single colour per channel) rather than SPI pixels.
fn sptech_pwm(model: &str) -> bool {
    ["SP631E", "SP641E", "SP632E", "SP642E", "SP633E", "SP643E", "SP634E", "SP644E", "SP635E", "SP645E"].contains(&model)
}

pub fn by_code(code: u8) -> Option<Ident> {
    let (f, m): (&'static str, &str) = match code {
        0x01 => ("sp601e", "SP601E"),
        0x02 => ("sp60xe", "SP602E"),
        0x05 => ("sp60xe", "SP608E"),
        0x04 | 0x10..=0x15 => ("banlanx2", "SP611E"),
        0x17 => ("banlanx2", "SP617E"),
        0x1B => ("banlanx2", "SP620E"),
        0x0D | 0x16 => ("banlanx2", "SP621E"),
        0x09 => ("banlanx3", "SP613E"),
        0x0A => ("banlanx3", "SP614E"),
        0x0E => ("banlanx3", "SP623E"),
        0x0F => ("banlanx3", "SP624E"),
        0x36 => ("sptech", "SP64BE"),
        0x37 => ("sptech", "SP64CE"),
        c => ("sptech", SPTECH.iter().find(|x| x.0 == c)?.1),
    };
    Some(Ident { family: f, model: m.into() })
}

pub struct Banlanx {
    fam: Fam,
    model: String,
    b2: proto::Reassembler,
    buf: Vec<u8>,
    want: usize,
    next: u8,
}

impl Banlanx {
    pub fn new(family: &str, model: &str) -> Option<Self> {
        let fam = match family {
            "banlanx2" => Fam::B2,
            "banlanx3" => Fam::B3,
            "sp110e" => Fam::Sp110,
            "sp107e" => Fam::Sp107,
            "sp601e" => Fam::Sp601,
            "sp60xe" => Fam::Sp60x,
            "sptech" => Fam::SpTech,
            _ => return None,
        };
        Some(Self { fam, model: model.into(), b2: Default::default(), buf: Vec::new(), want: 0, next: 0 })
    }

    fn mic(&self) -> bool {
        matches!(self.model.as_str(), "SP613E" | "SP614E")
    }
}

/// SPTech BLE frame: `53 <cmd> <key 0> 01 00 <len> <data>`.
fn st(cmd: u8, data: &[u8]) -> Frame {
    let mut v = vec![0x53, cmd, 0x00, 0x01, 0x00, data.len() as u8];
    v.extend_from_slice(data);
    w(v)
}

/// SPTech light modes; an effect id carries its mode in the high byte.
const ST_STATIC: u8 = 0x01;
const ST_DYNAMIC: u8 = 0x03;

/// Decodes an SPTech status payload (chunk list); shared with the Wi-Fi models.
pub fn sptech_status(p: &[u8], s: &mut State) -> bool {
    let Some((&lf, mut d)) = p.split_first() else { return false };
    let mut ok = false;
    while d.len() > 1 + lf as usize {
        let t = d[0];
        let n = d[1..=1 + lf as usize].iter().fold(0usize, |a, &b| (a << 8) | b as usize);
        d = &d[2 + lf as usize..];
        if d.len() < n {
            break;
        }
        let c = &d[..n];
        if (t == 2 || t == 3) && c.len() >= 16 {
            s.power = c[1] != 0;
            s.cycle = c[2] != 0;
            s.order = c[3];
            let (mode, fx) = (c[4], c[5]);
            s.effect = if mode == ST_STATIC { None } else { Some(((mode as u16) << 8) | fx as u16) };
            s.bright = pct(c[7]).max(1);
            s.rgb = [c[9], c[10], c[11]];
            s.speed = from_scale(c[14], 10);
            s.length = c[15];
            ok = true;
        }
        d = &d[n..];
    }
    ok
}

impl BleProto for Banlanx {
    fn gatt(&self) -> Gatt {
        Gatt { write: vec![u16id(0xFFE1)], write2: None, notify: vec![u16id(0xFFE1)], response: true, gap: 0 }
    }

    fn caps(&self) -> Caps {
        let fx = match self.fam {
            Fam::B2 => "banlanx2",
            Fam::B3 if self.mic() => "banlanx3m",
            Fam::B3 => "banlanx3",
            Fam::Sp110 => "sp110e",
            Fam::Sp107 => "sp107e",
            Fam::Sp601 | Fam::Sp60x => "sp601e",
            Fam::SpTech if sptech_pwm(&self.model) => "sptech_pwm",
            Fam::SpTech => "sptech_spi",
        };
        Caps {
            state: !matches!(self.fam, Fam::Sp601 | Fam::Sp60x),
            color: true,
            fx,
            speed: true,
            length: match self.fam {
                Fam::B2 | Fam::SpTech | Fam::Sp601 => 150,
                Fam::Sp60x => 240,
                _ => 0,
            },
            order: self.fam == Fam::B2,
            cycle: matches!(self.fam, Fam::B2 | Fam::B3 | Fam::Sp110 | Fam::SpTech),
            timers: self.fam == Fam::B2,
            rename: self.fam == Fam::B2,
        }
    }

    fn query(&self) -> Option<Frame> {
        Some(w(match self.fam {
            Fam::B2 => proto::query(),
            Fam::B3 => vec![0x1D, 0x00],
            Fam::Sp110 => vec![0, 0, 0, 0x10],
            Fam::Sp107 => vec![0, 0, 0, 0x02],
            Fam::Sp601 => vec![0xAA, 0x2F, 0x00],
            Fam::Sp60x => vec![0x88, 0x8F, 0x00],
            Fam::SpTech => return Some(st(0x02, &[0x01])),
        }))
    }

    fn frames(&mut self, op: &Op, s: &State) -> Vec<Frame> {
        let lvl = byte(s.bright.max(1));
        match self.fam {
            Fam::B2 => match op {
                Op::Power(on) => vec![w(proto::power(*on))],
                Op::Bright(p) => vec![w(proto::brightness(byte(*p).max(1)))],
                Op::Color(c) => {
                    let mut v = Vec::new();
                    if s.cycle {
                        v.push(w(proto::cycle(false)));
                    }
                    if s.effect.is_some() {
                        v.push(w(proto::effect(proto::SOLID)));
                    }
                    v.push(w(proto::color(*c, lvl)));
                    v
                }
                Op::Stream(c, b) => vec![w(proto::color(*c, byte(*b).max(1)))],
                Op::Effect(e) => {
                    let mut v = Vec::new();
                    if s.cycle {
                        v.push(w(proto::cycle(false)));
                    }
                    v.push(w(proto::effect(*e as u8)));
                    v
                }
                Op::Speed(p) => vec![w(proto::speed(to_scale(*p, proto::MAX_SPEED)))],
                Op::Length(l) => vec![w(proto::length(*l))],
                Op::Order(o) => vec![w(proto::order(*o))],
                Op::Cycle(on) => vec![w(proto::cycle(*on))],
                Op::TimerSet(t) => {
                    let (now, wd) = crate::util::local_now();
                    vec![w(proto::timer_set(t, now, wd))]
                }
                Op::TimerDel(i) => vec![w(proto::timer_delete(*i))],
                Op::Rename(n) => vec![w(proto::rename(n))],
            },
            Fam::B3 => match op {
                Op::Power(on) => vec![w(vec![0x0F, 0x01, *on as u8])],
                Op::Bright(p) => vec![w(vec![0x12, 0x01, byte(*p).max(1)])],
                Op::Color(c) => {
                    let mut v = Vec::new();
                    if s.effect.is_some() || s.cycle {
                        v.push(w(vec![0x15, 0x01, 0x63]));
                    }
                    v.push(w(vec![0x13, 0x04, c[0], c[1], c[2], lvl]));
                    v
                }
                Op::Stream(c, b) => vec![w(vec![0x13, 0x04, c[0], c[1], c[2], byte(*b).max(1)])],
                Op::Effect(e) => vec![w(vec![0x15, 0x01, *e as u8])],
                Op::Speed(p) => vec![w(vec![0x14, 0x01, to_scale(*p, 10)])],
                Op::Cycle(on) => vec![w(vec![0x16, 0x01, *on as u8])],
                _ => vec![],
            },
            Fam::Sp110 | Fam::Sp107 => {
                let (on, off, bri, col, fx, spd, solid) = if self.fam == Fam::Sp110 { (0xAA, 0xAB, 0x2A, 0x1E, 0x2C, 0x03, 0x79) } else { (0xAA, 0xBB, 10, 12, 8, 9, 0xB5) };
                let f = |a: u8, c: u8| w(vec![a, 0, 0, c]);
                match op {
                    Op::Power(p) => vec![f(0, if *p { on } else { off })],
                    Op::Bright(p) => vec![f(byte(*p).max(1), bri)],
                    Op::Color(c) | Op::Stream(c, _) => {
                        let mut v = Vec::new();
                        if matches!(op, Op::Color(_)) && (s.effect.is_some() || s.cycle) {
                            v.push(f(solid, fx));
                        }
                        if let Op::Stream(_, b) = op {
                            v.push(f(byte(*b).max(1), bri));
                        }
                        v.push(w(vec![c[0], c[1], c[2], col]));
                        v
                    }
                    Op::Effect(e) => vec![f(*e as u8, fx)],
                    Op::Speed(p) => vec![f(to_scale(*p, 186), spd)],
                    Op::Cycle(true) if self.fam == Fam::Sp110 => vec![f(0, 0x06)],
                    Op::Cycle(false) if self.fam == Fam::Sp110 => vec![f(s.effect.unwrap_or(solid as u16) as u8, fx)],
                    _ => vec![],
                }
            }
            Fam::Sp601 | Fam::Sp60x => {
                // every channel at once: SP601E takes "channels − 1", SP60xE a bit mask
                let (h, base, ch) = if self.fam == Fam::Sp601 { (0xAA, 0x20, 1u8) } else { (0x88, 0x80, 0xFF) };
                let f = |c: u8, d: &[u8]| {
                    let mut v = vec![h, base + c, d.len() as u8 + 1, ch];
                    v.extend_from_slice(d);
                    w(v)
                };
                match op {
                    Op::Power(p) => vec![f(2, &[*p as u8])],
                    Op::Bright(p) => vec![f(5, &[byte(*p).max(1)])],
                    Op::Color(c) | Op::Stream(c, _) => {
                        let mut v = Vec::new();
                        if matches!(op, Op::Color(_)) && s.effect.is_some() {
                            v.push(f(3, &[0x19]));
                        }
                        if let Op::Stream(_, b) = op {
                            v.push(f(5, &[byte(*b).max(1)]));
                        }
                        v.push(f(9, &[c[0], c[1], c[2], 0xFF]));
                        v
                    }
                    Op::Effect(e) => vec![f(3, &[*e as u8])],
                    Op::Speed(p) => vec![f(6, &[to_scale(*p, 10)])],
                    Op::Length(l) if self.fam == Fam::Sp601 => vec![f(7, &[(*l).clamp(1, 150)])],
                    Op::Length(l) => vec![f(7, &[0, (*l).max(1)])],
                    _ => vec![],
                }
            }
            Fam::SpTech => match op {
                Op::Power(p) => vec![st(0x50, &[*p as u8])],
                Op::Bright(p) => vec![st(0x51, &[0x00, byte(*p).max(1)])],
                Op::Color(c) => {
                    let mut v = Vec::new();
                    if s.effect.is_some() {
                        v.push(st(0x53, &[ST_STATIC, 0x01]));
                    }
                    v.push(st(0x52, &[c[0], c[1], c[2], lvl]));
                    v
                }
                Op::Stream(c, b) => vec![st(0x52, &[c[0], c[1], c[2], byte(*b).max(1)])],
                Op::Effect(e) => vec![st(0x53, &[(*e >> 8).max(ST_DYNAMIC as u16) as u8, *e as u8])],
                Op::Speed(p) => vec![st(0x54, &[to_scale(*p, 10)])],
                Op::Length(l) => vec![st(0x55, &[(*l).clamp(1, 150)])],
                Op::Cycle(on) => vec![st(0x58, &[*on as u8])],
                _ => vec![],
            },
        }
    }

    fn notified(&mut self, d: &[u8], s: &mut State) -> bool {
        match self.fam {
            Fam::B2 => {
                let Some(p) = self.b2.feed(d) else { return false };
                let Some(l) = proto::parse(&p) else { return false };
                s.power = l.power;
                s.cycle = l.mode == 1;
                s.effect = if l.effect == proto::SOLID { None } else { Some(l.effect as u16) };
                s.order = l.order;
                s.bright = pct(l.bright).max(1);
                s.speed = from_scale(l.speed, proto::MAX_SPEED);
                s.length = l.length;
                s.rgb = l.rgb;
                s.timers = l.timers;
                true
            }
            Fam::B3 => {
                // [1, total, chunk len, payload…] then [n, chunk len, payload…]
                if d.len() < 2 {
                    return false;
                }
                if d[0] == 1 && d.len() >= 3 {
                    self.buf = d[3..].to_vec();
                    self.want = d[1] as usize;
                    self.next = 2;
                } else if d[0] == self.next && self.want > 0 {
                    self.buf.extend_from_slice(&d[2..]);
                    self.next += 1;
                } else {
                    self.want = 0;
                    return false;
                }
                if self.buf.len() < self.want || self.want < 10 {
                    return false;
                }
                let p = std::mem::take(&mut self.buf);
                self.want = 0;
                s.power = p[0] == 1;
                s.bright = pct(p[1]).max(1);
                s.speed = from_scale(p[2], 10);
                s.order = p[3];
                s.cycle = p[5] != 0;
                s.effect = if p[4] == 0x63 { None } else { Some(p[4] as u16) };
                s.rgb = [p[6], p[7], p[8]];
                true
            }
            Fam::Sp110 => {
                let p = match d.len() {
                    13 => &d[1..],
                    12 => d,
                    _ => return false,
                };
                s.power = p[0] != 0;
                s.cycle = p[1] == 0;
                s.effect = if p[1] == 0x79 || p[1] == 0 { None } else { Some(p[1] as u16) };
                s.speed = ((p[2] as u32 * 100 / 186) as u8).max(1);
                s.bright = pct(p[3]).max(1);
                s.rgb = [p[8], p[9], p[10]];
                true
            }
            Fam::Sp107 => {
                if d.len() == 15 && d[0] == 0 && d[1] == 1 {
                    self.buf = d[2..].to_vec();
                    return false;
                }
                if !(d.len() == 15 && d[0] == 0 && d[1] == 2 && self.buf.len() == 13) {
                    return false;
                }
                let mut p = std::mem::take(&mut self.buf);
                p.extend_from_slice(&d[2..]);
                s.power = p[0] != 0;
                s.effect = if p[5] == 0xB5 { None } else { Some(p[5] as u16) };
                s.speed = ((p[9] as u32 * 100 / 186) as u8).max(1);
                s.bright = pct(p[10]).max(1);
                s.rgb = [p[13], p[14], p[15]];
                true
            }
            Fam::Sp601 | Fam::Sp60x => false,
            Fam::SpTech => {
                // 53 02 <key> .. .. <len> payload; long answers may come in several writes
                if d.len() >= 6 && d[0] == 0x53 && d[1] == 0x02 {
                    if d[2] != 0 {
                        return false; // encoded answers are not supported (as in UniLED)
                    }
                    self.want = d[5] as usize;
                    self.buf = d[6..].to_vec();
                } else if self.want > 0 {
                    self.buf.extend_from_slice(d);
                } else {
                    return false;
                }
                if self.buf.len() < self.want {
                    return false;
                }
                let mut p = std::mem::take(&mut self.buf);
                p.truncate(self.want);
                self.want = 0;
                sptech_status(&p, s)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes() {
        assert_eq!(by_code(0x0D).unwrap().model, "SP621E");
        assert_eq!(by_code(0x27).unwrap().model, "SP638E");
        assert_eq!(by_code(0x36).unwrap().family, "sptech");
        assert!(by_code(0x99).is_none());
    }

    #[test]
    fn b2_frames_match_the_lamp() {
        let mut b = Banlanx::new("banlanx2", "SP621E").unwrap();
        let s = State { power: true, bright: 100, effect: Some(5), ..Default::default() };
        let f = b.frames(&Op::Color([1, 2, 3]), &s);
        assert_eq!(f, vec![w(proto::effect(proto::SOLID)), w(vec![0xA0, 0x69, 0x04, 1, 2, 3, 255])]);
        assert_eq!(b.frames(&Op::Speed(100), &s), vec![w(vec![0xA0, 0x67, 0x01, 10])]);
    }

    #[test]
    fn sptech_frames() {
        let mut b = Banlanx::new("sptech", "SP638E").unwrap();
        let s = State { power: true, bright: 100, ..Default::default() };
        assert_eq!(b.frames(&Op::Power(true), &s), vec![w(vec![0x53, 0x50, 0, 1, 0, 1, 1])]);
        assert_eq!(b.frames(&Op::Effect(0x0305), &s), vec![w(vec![0x53, 0x53, 0, 1, 0, 2, 3, 5])]);
    }

    #[test]
    fn sptech_status_chunks() {
        // length field 1 byte; chunk 2: power on, static mode, level ff, colour 10 20 30, speed 5
        let mut c2 = vec![0u8; 24];
        c2[1] = 1;
        c2[4] = 1;
        c2[7] = 0xFF;
        c2[9..12].copy_from_slice(&[0x10, 0x20, 0x30]);
        c2[14] = 5;
        c2[15] = 60;
        let mut p = vec![0, 2, 24];
        p.extend(c2);
        let mut s = State::default();
        assert!(sptech_status(&p, &mut s));
        assert!(s.power && s.effect.is_none());
        assert_eq!((s.bright, s.rgb, s.speed, s.length), (100, [0x10, 0x20, 0x30], 50, 60));
    }
}
