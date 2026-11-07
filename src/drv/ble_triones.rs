//! Triones / HappyLighting / LEDBLE / Dream / QHM (Home Assistant led_ble, flux_led
//! "LEDENET original" protocol): `56 r g b w f0 aa` colours, `bb pattern delay 44` presets,
//! state `ef 01 77` → `66 …` (11 bytes). Written without response.

use super::{BleProto, Frame, Gatt, u16id, w};
use crate::lamp::{Caps, Op, State};

pub struct Triones {
    dream: bool,
    /// the colour as picked; brightness scales it (the protocol has no brightness of its own)
    base: [u8; 3],
}

impl Triones {
    pub fn new(name: &str) -> Self {
        Self { dream: name.to_uppercase().starts_with("DREAM"), base: [255, 255, 255] }
    }
}

/// Speed % → the preset delay byte (1 fastest … 31 slowest), as flux_led speedToDelay.
pub fn delay(speed: u8) -> u8 {
    ((100 - speed.min(100)) as u32 * 30 / 100 + 1) as u8
}

pub fn undelay(d: u8) -> u8 {
    (100 - (d.clamp(1, 31) as u32 - 1) * 100 / 30) as u8
}

pub fn scale(c: [u8; 3], pct: u8) -> [u8; 3] {
    let m = *c.iter().max().unwrap_or(&0) as u32;
    if m == 0 {
        return c;
    }
    let target = pct.clamp(1, 100) as u32 * 255 / 100;
    c.map(|x| (x as u32 * target / m).min(255) as u8)
}

impl BleProto for Triones {
    fn gatt(&self) -> Gatt {
        Gatt {
            write: [0xFF01, 0xFFD9, 0xFFE9, 0xFFD5, 0xFFE5].map(u16id).to_vec(),
            write2: None,
            notify: [0xFF02, 0xFFD4, 0xFFE4, 0xFFD0, 0xFFE0].map(u16id).to_vec(),
            response: false,
            gap: 20,
        }
    }

    fn caps(&self) -> Caps {
        Caps { state: true, color: true, fx: if self.dream { "dream" } else { "triones" }, speed: true, ..Default::default() }
    }

    fn query(&self) -> Option<Frame> {
        Some(w(vec![0xEF, 0x01, 0x77]))
    }

    fn frames(&mut self, op: &Op, s: &State) -> Vec<Frame> {
        let col = |c: [u8; 3]| w(vec![0x56, c[0], c[1], c[2], 0x00, 0xF0, 0xAA]);
        match op {
            Op::Power(on) => vec![w(vec![0xCC, if *on { 0x23 } else { 0x24 }, 0x33])],
            Op::Color(c) => {
                self.base = *c;
                vec![col(scale(*c, s.bright))]
            }
            Op::Stream(c, b) => vec![col(scale(*c, *b))],
            Op::Bright(p) => match s.effect {
                Some(e) if self.dream => vec![w(vec![0x9E, 0x00, e as u8, (s.speed as u32 * 255 / 100) as u8, (*p as u32 * 255 / 100) as u8, 0x00, 0xE9])],
                Some(_) => vec![],
                None => vec![col(scale(self.base, *p))],
            },
            Op::Effect(_) | Op::Speed(_) if self.dream => {
                let (e, sp) = match op {
                    Op::Effect(e) => (*e, s.speed),
                    Op::Speed(v) => (s.effect.unwrap_or(0), *v),
                    _ => unreachable!(),
                };
                vec![w(vec![0x9E, 0x00, e as u8, (sp as u32 * 255 / 100) as u8, (s.bright as u32 * 255 / 100) as u8, 0x00, 0xE9])]
            }
            Op::Effect(e) => vec![w(vec![0xBB, *e as u8, delay(s.speed.max(1)), 0x44])],
            Op::Speed(v) => match s.effect {
                Some(e) => vec![w(vec![0xBB, e as u8, delay(*v), 0x44])],
                None => vec![],
            },
            _ => vec![],
        }
    }

    fn notified(&mut self, d: &[u8], s: &mut State) -> bool {
        if d.len() == 4 && d[0] == 0xCC {
            s.power = d[1] == 0x23;
            return true;
        }
        if d.len() < 11 {
            return false;
        }
        s.power = d[2] == 0x23;
        let pattern = d[3];
        let rgb = [d[6], d[7], d[8]];
        if (0x25..=0x38).contains(&pattern) || self.dream && pattern != 0x41 && pattern != 0x61 {
            s.effect = Some(pattern as u16);
            s.speed = undelay(d[5]);
        } else {
            s.effect = None;
            let m = *rgb.iter().max().unwrap_or(&0);
            if m > 0 {
                s.bright = ((m as u32 * 100 + 127) / 255).max(1) as u8;
                s.rgb = rgb.map(|x| (x as u32 * 255 / m as u32) as u8);
                self.base = s.rgb;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_and_state() {
        let mut t = Triones::new("QHM-1");
        let s = State { bright: 50, speed: 50, ..Default::default() };
        assert_eq!(t.frames(&Op::Color([255, 0, 0]), &s), vec![w(vec![0x56, 127, 0, 0, 0, 0xF0, 0xAA])]);
        assert_eq!(t.frames(&Op::Effect(0x25), &s), vec![w(vec![0xBB, 0x25, 16, 0x44])]);
        let mut st = State::default();
        assert!(t.notified(&[0x66, 0x01, 0x23, 0x41, 0x21, 0x0a, 0xff, 0x00, 0x00, 0x01, 0x99], &mut st));
        assert!(st.power && st.effect.is_none());
        assert_eq!((st.rgb, st.bright), ([255, 0, 0], 100));
        assert_eq!(delay(100), 1);
        assert_eq!(delay(0), 31);
    }
}
