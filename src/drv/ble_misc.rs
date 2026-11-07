//! Small write-only families: LED Lamp (LEDBLE-01, `7e ff … ef`, ledble-ledlamp), Keepsmile
//! (KS03~, `5a…a5` frames, curl_smile) and MohuanLED (BJ_LED, `69 96 …`, bj_led). None of them
//! reports its state; brightness is scaled into the colour where the protocol has none.

use super::{BleProto, Frame, Gatt, u16id, w};
use crate::lamp::{Caps, Op, State};

#[derive(Default)]
pub struct LedLamp;

impl BleProto for LedLamp {
    fn gatt(&self) -> Gatt {
        Gatt { write: vec![u16id(0xFFE1)], write2: None, notify: vec![], response: false, gap: 30 }
    }

    fn caps(&self) -> Caps {
        Caps { color: true, fx: "ledlamp", speed: true, ..Default::default() }
    }

    fn frames(&mut self, op: &Op, s: &State) -> Vec<Frame> {
        let col = |c: [u8; 3]| w(vec![0x7e, 0xff, 0x05, 0x03, c[0], c[1], c[2], 0xff, 0xef]);
        let bri = |p: u8| w(vec![0x7e, 0xff, 0x01, p.clamp(1, 100), 0x00, 0xff, 0xff, 0xff, 0xff, 0xef]);
        match op {
            Op::Power(true) => vec![w(vec![0x7e, 0xff, 0x04, 0x01, 0xff, 0xff, 0xff, 0xff, 0xef])],
            Op::Power(false) => vec![w(vec![0x7e, 0xff, 0x04, 0x00, 0xff, 0xff, 0xff, 0xff, 0xef])],
            Op::Bright(p) => vec![bri(*p)],
            Op::Color(c) => vec![col(*c)],
            Op::Stream(c, b) => {
                let mut v = vec![col(*c)];
                if *b != s.bright {
                    v.push(bri(*b));
                }
                v
            }
            Op::Effect(e) => vec![w(vec![0x7e, 0xff, 0x03, *e as u8, 0x03, 0xff, 0xff, 0xff, 0xef])],
            Op::Speed(v) => vec![w(vec![0x7e, 0xff, 0x02, (*v).min(100), 0x00, 0xff, 0xff, 0xff, 0xef])],
            _ => vec![],
        }
    }
}

#[derive(Default)]
pub struct Keepsmile;

impl BleProto for Keepsmile {
    fn gatt(&self) -> Gatt {
        Gatt { write: vec![u16id(0xAFD1)], write2: None, notify: vec![], response: false, gap: 200 }
    }

    fn caps(&self) -> Caps {
        Caps { color: true, fx: "keepsmile", speed: true, ..Default::default() }
    }

    fn hello(&mut self) -> Vec<Frame> {
        vec![w(vec![0x5F, 0x01, 0x00, 0xF5])]
    }

    fn frames(&mut self, op: &Op, s: &State) -> Vec<Frame> {
        // colour channels and brightness are 0..100 on the wire (curl_smile)
        let col = |c: [u8; 3], b: u8| {
            let p = c.map(|x| (x as u32 * 100 / 255) as u8);
            w(vec![0x5A, 0x00, 0x01, p[0], p[1], p[2], 0x00, b.clamp(1, 100), 0x00, 0xA5])
        };
        let fx = |e: u16, sp: u8, b: u8| w(vec![0x5C, 0x00, e as u8, sp.min(100), b.clamp(1, 100), 0x00, 0xC5]);
        match op {
            Op::Power(on) => vec![w(vec![0x5B, if *on { 0xF0 } else { 0x0F }, 0x00, 0xB5])],
            Op::Color(c) => vec![col(*c, s.bright)],
            Op::Stream(c, b) => vec![col(*c, *b)],
            Op::Bright(p) => match s.effect {
                Some(e) => vec![fx(e, s.speed, *p)],
                None => vec![col(s.rgb, *p)],
            },
            Op::Effect(e) => vec![fx(*e, s.speed, s.bright)],
            Op::Speed(v) => s.effect.map(|e| vec![fx(e, *v, s.bright)]).unwrap_or_default(),
            _ => vec![],
        }
    }
}

#[derive(Default)]
pub struct Mohuan;

impl BleProto for Mohuan {
    fn gatt(&self) -> Gatt {
        Gatt { write: vec![u16id(0xEE01)], write2: None, notify: vec![], response: false, gap: 30 }
    }

    fn caps(&self) -> Caps {
        Caps { color: true, fx: "mohuan", speed: true, ..Default::default() }
    }

    fn frames(&mut self, op: &Op, s: &State) -> Vec<Frame> {
        let col = |c: [u8; 3], b: u8| {
            let k = b.clamp(1, 100) as u32;
            w(vec![0x69, 0x96, 0x05, 0x02, (c[0] as u32 * k / 100) as u8, (c[1] as u32 * k / 100) as u8, (c[2] as u32 * k / 100) as u8])
        };
        // speed byte: 01 fast … 0a slow
        let fx = |e: u16, sp: u8| w(vec![0x69, 0x96, 0x03, 0x03, e as u8, (10 - (sp.min(100) as u32 * 9 / 100)) as u8]);
        match op {
            Op::Power(on) => vec![w(vec![0x69, 0x96, 0x02, 0x01, *on as u8])],
            Op::Color(c) => vec![col(*c, s.bright)],
            Op::Stream(c, b) => vec![col(*c, *b)],
            Op::Bright(p) if s.effect.is_none() => vec![col(s.rgb, *p)],
            Op::Effect(e) => vec![fx(*e, s.speed)],
            Op::Speed(v) => s.effect.map(|e| vec![fx(e, *v)]).unwrap_or_default(),
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames() {
        let s = State { bright: 100, speed: 50, ..Default::default() };
        assert_eq!(Mohuan.frames(&Op::Power(false), &s)[0].1, vec![0x69, 0x96, 0x02, 0x01, 0x00]);
        assert_eq!(Mohuan.frames(&Op::Effect(2), &s)[0].1, vec![0x69, 0x96, 0x03, 0x03, 0x02, 0x06]);
        assert_eq!(Keepsmile.frames(&Op::Power(true), &s)[0].1, vec![0x5B, 0xF0, 0x00, 0xB5]);
        assert_eq!(LedLamp.frames(&Op::Color([1, 2, 3]), &s)[0].1, vec![0x7e, 0xff, 0x05, 0x03, 1, 2, 3, 0xff, 0xef]);
    }
}
