//! Zengge LEDnetWF / Magic Hue / IOTBT (app Zengge): payloads wrapped as
//! `00 <seq> 80 00 <len16> <len+1> <0b command | 0a query> <payload + sum>`, characteristic FF01,
//! notifications on FF02. After github.com/8none1/lednetwf_ble; the product id (advert bytes 8..9
//! or the name "LEDnetWF02<id4><mac6>") picks the effect command.

use super::{BleProto, Frame, Gatt, u16id, w};
use crate::lamp::{Caps, Op, State, hsv};

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// effects 37..56 with checksum and inverted speed
    Simple,
    /// rings 0x53: effects 1..113, 4 bytes, no checksum
    Addr,
    /// strips / symphony: `42` effects 1..100
    Symphony,
    /// Telink IOTBT bulbs
    Iotbt,
}

pub struct Zengge {
    kind: Kind,
    seq: u8,
    base: [u8; 3],
}

fn product(name: &str) -> Option<u16> {
    if let Some(h) = name.strip_prefix("Zengge 0x") {
        return u16::from_str_radix(h, 16).ok();
    }
    let up = name.to_uppercase();
    if up.starts_with("IOTB") {
        return Some(0);
    }
    if up.starts_with("LEDNETWF") && name.len() == 20 {
        return u16::from_str_radix(&name[10..14], 16).ok();
    }
    None
}

impl Zengge {
    /// `name` is the advertised name or the model ("Zengge 0x53").
    pub fn new(name: &str) -> Self {
        let kind = match product(name) {
            Some(0) => Kind::Iotbt,
            Some(0x08 | 0x3C | 0x54 | 0x5B | 0x62 | 0x55) => Kind::Simple,
            Some(0x53 | 0x1D) => Kind::Addr,
            _ => Kind::Symphony,
        };
        Self { kind, seq: 0, base: [255, 255, 255] }
    }

    fn wrap(&mut self, family: u8, mut p: Vec<u8>, sum: bool) -> Frame {
        if sum {
            let ck = p.iter().fold(0u8, |a, b| a.wrapping_add(*b));
            p.push(ck);
        }
        self.seq = self.seq.wrapping_add(1);
        let n = p.len();
        let mut v = vec![0x00, self.seq, 0x80, 0x00, (n >> 8) as u8, n as u8, (n + 1) as u8, family];
        v.extend(p);
        w(v)
    }

    fn color(&mut self, c: [u8; 3], bright: u8) -> Frame {
        match self.kind {
            Kind::Simple => {
                let k = bright.clamp(1, 100) as u32;
                let s = c.map(|x| (x as u32 * k / 100) as u8);
                self.wrap(0x0b, vec![0x31, s[0], s[1], s[2], 0, 0, 0xF0, 0x0F], true)
            }
            Kind::Iotbt => {
                let (h, sat, _) = hsv(c);
                let hue = if sat < 0.1 { 0 } else { (h / 360.0 * 239.0) as u8 + 1 };
                let lvl = ((bright.clamp(1, 100) as f32 / 100.0).powf(2.2) * 31.0).round() as u8;
                self.wrap(0x0a, vec![0xE2, 0x0B, hue, 0xE0 | lvl.min(31)], false)
            }
            _ => {
                let (h, s, _) = hsv(c);
                let packed = ((h as u16).min(359) << 7) | (s * 100.0) as u16;
                self.wrap(0x0b, vec![0x3B, 0xA1, (packed >> 8) as u8, packed as u8, bright.clamp(1, 100), 0, 0, c[0], c[1], c[2], 0, 0], true)
            }
        }
    }

    fn effect(&mut self, id: u16, speed: u8, bright: u8) -> Frame {
        let b = bright.clamp(1, 100);
        match self.kind {
            Kind::Simple => {
                let inv = (1 + 30 * (100 - speed.min(100)) as u32 / 100).clamp(1, 31) as u8;
                self.wrap(0x0b, vec![0x38, id as u8, inv, b], true)
            }
            Kind::Addr => self.wrap(0x0b, vec![0x38, id as u8, speed.min(100), b], false),
            Kind::Symphony => self.wrap(0x0b, vec![0x42, id as u8, speed.min(100), b], true),
            Kind::Iotbt => self.wrap(0x0a, vec![0xE0, 0x02, 0x00, id as u8, speed.clamp(1, 100), b], false),
        }
    }
}

impl BleProto for Zengge {
    fn gatt(&self) -> Gatt {
        Gatt { write: vec![u16id(0xFF01)], write2: None, notify: vec![u16id(0xFF02)], response: false, gap: 30 }
    }

    fn caps(&self) -> Caps {
        let fx = match self.kind {
            Kind::Simple => "zengge_simple",
            Kind::Addr => "zengge_53",
            Kind::Symphony => "zengge_sym",
            Kind::Iotbt => "zengge_iotbt",
        };
        Caps { state: true, color: true, fx, speed: true, ..Default::default() }
    }

    fn query(&self) -> Option<Frame> {
        let mut z = Zengge { kind: self.kind, seq: self.seq, base: self.base };
        Some(if self.kind == Kind::Iotbt { z.wrap(0x0a, vec![0xEA, 0x81, 0x8A, 0x8B], false) } else { z.wrap(0x0a, vec![0x81, 0x8A, 0x8B], true) })
    }

    fn frames(&mut self, op: &Op, s: &State) -> Vec<Frame> {
        match op {
            Op::Power(on) => {
                let m = if *on { 0x23 } else { 0x24 };
                vec![if self.kind == Kind::Iotbt { self.wrap(0x0a, vec![0x71, m], false) } else { self.wrap(0x0b, vec![0x3B, m, 0, 0, 0, 0, 0, 0, 0, 0x32, 0, 0], true) }]
            }
            Op::Color(c) => {
                self.base = *c;
                vec![self.color(*c, s.bright)]
            }
            Op::Stream(c, b) => vec![self.color(*c, *b)],
            Op::Bright(p) => match s.effect {
                Some(e) => vec![self.effect(e, s.speed, *p)],
                None => vec![self.color(self.base, *p)],
            },
            Op::Effect(e) => vec![self.effect(*e, s.speed.max(1), s.bright)],
            Op::Speed(v) => match s.effect {
                Some(e) => vec![self.effect(e, *v, s.bright)],
                None => vec![],
            },
            _ => vec![],
        }
    }

    fn notified(&mut self, d: &[u8], s: &mut State) -> bool {
        if d.len() < 9 {
            return false;
        }
        let p = &d[8..];
        if p[0] == 0xEA && p.len() > 6 && p[1] == 0x81 {
            s.power = p[6] == 0x23;
            return true;
        }
        if p[0] != 0x81 || p.len() < 9 {
            return false;
        }
        s.power = p[2] == 0x23;
        let (mode, sub) = (p[3], p[4]);
        let rgb = [p[6], p[7], p[8]];
        if self.kind == Kind::Simple && (37..=56).contains(&mode) {
            s.effect = Some(mode as u16);
            s.speed = (100 - (p[5].clamp(1, 31) as u32 - 1) * 100 / 30) as u8;
        } else if mode == 0x25 {
            s.effect = Some(sub as u16);
            s.bright = p[6].clamp(1, 100);
            s.speed = p[7].min(100);
        } else if mode == 0x61 {
            s.effect = None;
            let m = *rgb.iter().max().unwrap_or(&0);
            if m > 0 {
                s.rgb = rgb.map(|x| (x as u32 * 255 / m as u32) as u8);
                self.base = s.rgb;
                if self.kind == Kind::Simple {
                    s.bright = ((m as u32 * 100 + 127) / 255).max(1) as u8;
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapper_and_query() {
        let z = Zengge::new("LEDnetWF02001D0CDA81");
        assert!(z.kind == Kind::Addr);
        // derived from lednetwf_ble: 00 SS 80 00 00 04 05 0A 81 8A 8B 96
        assert_eq!(z.query().unwrap().1, vec![0x00, 0x01, 0x80, 0x00, 0x00, 0x04, 0x05, 0x0A, 0x81, 0x8A, 0x8B, 0x96]);
        let mut z = Zengge::new("Zengge 0x56");
        let on = z.frames(&Op::Power(true), &State::default());
        assert_eq!(on[0].1[8..], [0x3B, 0x23, 0, 0, 0, 0, 0, 0, 0, 0x32, 0, 0, 0x90]);
    }
}
