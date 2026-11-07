//! ELK-BLEDOM, MELK and their clones (apps duoCo Strip, Lotus Lantern, Lotus Lamp X): fixed
//! `7e … ef` frames that differ between firmwares, written without response. Templates after
//! github.com/dave-code-ruiz/elkbledom (models.json); "ii" is brightness 0..100, "vv" an effect or
//! speed value, "rr gg bb" the colour.

use super::{BleProto, Frame, Gatt, u16id, w};
use crate::lamp::{Caps, Op, State};
use windows::core::GUID;

pub struct Variant {
    pub name: &'static str,
    write: u16,
    notify: u16,
    on: &'static str,
    off: &'static str,
    br: &'static str,
    col: &'static str,
    fx: &'static str,
    spd: &'static str,
    table: &'static str,
    /// MELK firmwares wait for a login before they take commands
    login: bool,
}

const G_ON: &str = "7e 00 04 f0 00 01 ff 00 ef";
const G_OFF: &str = "7e 00 04 00 00 00 ff 00 ef";
const G_COL: &str = "7e 00 05 03 rr gg bb 00 ef";
const G_FX: &str = "7e 00 03 vv 03 00 00 00 ef";
const G_SPD: &str = "7e 00 02 vv 00 00 00 00 ef";
const M_FX: &str = "7e 05 03 vv 06 ff ff 00 ef";
const M_SPD: &str = "7e 04 02 vv ff ff ff 00 ef";
const NUS: u16 = 0;

const fn v(name: &'static str, write: u16, notify: u16, on: &'static str, off: &'static str, br: &'static str, col: &'static str, fx: &'static str, spd: &'static str, table: &'static str, login: bool) -> Variant {
    Variant { name, write, notify, on, off, br, col, fx, spd, table, login }
}

/// Longest names first: a name matches the first prefix it starts with.
const VARIANTS: [Variant; 22] = [
    v("ELK-BLEDOM0E", 0xFFF3, 0xFFF4, "7e 07 04 ff 00 01 02 01 ef", "7e 07 04 00 00 00 02 01 ef", "7e 04 01 ii 01 ff 02 01 ef", "7e 07 05 03 rr gg bb 0a ef", "7e 07 03 vv 03 ff ff 00 ef", "7e 07 02 vv 00 00 00 00 ef", "elk", false),
    v("ELK-BLEDOB", 0xFFF3, 0xFFF4, "7e 07 04 ff 00 01 02 01 ef", "7e 07 04 00 00 00 02 01 ef", "7e 04 01 ii 01 ff 02 01 ef", "7e 07 05 03 rr gg bb 0a ef", "7e 07 03 vv 03 ff ff 00 ef", "7e 07 02 vv 00 00 00 00 ef", "elk", false),
    v("ELK-BLEDDM", 0xFFF3, 0xFFF4, "7e 04 04 f0 00 01 ff 00 ef", "7e 04 04 00 00 00 ff 00 ef", "7e 04 01 ii ff 00 ff 00 ef", G_COL, G_FX, G_SPD, "elk", false),
    v("ELK-BLEDOM", 0xFFF3, 0xFFF4, G_ON, G_OFF, "7e 00 01 ii ff 00 ff 00 ef", G_COL, G_FX, G_SPD, "elk", false),
    v("ELK-BULB2", 0xFFF3, 0xFFF4, G_ON, "7e 00 04 01 00 00 00 00 ef", "7e 04 01 ii ff 00 ff 00 ef", G_COL, G_FX, G_SPD, "elk", false),
    v("ELK-BULB", 0xFFF3, 0xFFF4, "7e 00 04 01 00 00 00 00 ef", G_OFF, "7e 04 01 ii ff 00 ff 00 ef", G_COL, G_FX, G_SPD, "elk", false),
    v("ELK-LAMPL", 0xFFF3, 0xFFF4, "7e 00 04 01 00 00 00 00 ef", G_OFF, "7e 04 01 ii ff 00 ff 00 ef", G_COL, "7e 00 03 vv 03 00 00 00 00 ef", G_SPD, "elk", false),
    v("ELK-BTCW", 0xFFF3, 0xFFF4, G_ON, G_OFF, "7e 00 01 ii 00 00 00 00 ef", "", "", "", "", false),
    v("ELK-BTC", 0xFFF3, 0xFFF4, G_ON, G_OFF, "7e 00 01 ii 00 00 00 00 ef", "7e 00 05 03 rr gg bb 0a ef", "7e 00 03 vv 03 ff ff 00 ef", G_SPD, "elk", false),
    v("ELK-BLE", 0xFFF3, 0xFFF4, G_ON, G_OFF, "7e 04 01 ii ff 00 ff 00 ef", G_COL, G_FX, G_SPD, "elk", false),
    v("MELK-OA21", 0xFFF3, 0xFFF4, "7e 04 04 f0 00 01 ff 00 ef", "7e 04 04 00 00 00 ff 00 ef", "7e 04 01 ii 01 ff ff 00 ef", G_COL, "", M_SPD, "", true),
    v("MELK-OG10", 0xFFF3, 0xFFF4, "7e 07 04 ff 00 01 02 01 ef", "7e 07 04 00 00 00 02 00 ef", "7e 04 01 ii ff 00 ff 00 ef", G_COL, M_FX, M_SPD, "melk_ox", true),
    v("MELK-OF10", 0xFFF3, 0xFFF4, "7e 04 04 f0 00 01 ff 00 ef", "7e 04 04 00 00 00 ff 00 ef", "7e 04 01 ii 01 ff ff 00 ef", G_COL, M_FX, M_SPD, "melk_ox", true),
    v("MELK-OC10", 0xFFF3, 0xFFF4, "7e 04 04 f0 00 01 ff 00 ef", "7e 04 04 00 00 00 ff 00 ef", "7e 04 01 ii 01 ff ff 00 ef", G_COL, M_FX, M_SPD, "melk_ox", true),
    v("MELK-OA10", 0xFFF3, 0xFFF4, "7e 04 04 f0 00 01 ff 00 ef", "7e 04 04 00 00 00 ff 00 ef", "7e 04 01 ii 01 ff ff 00 ef", G_COL, M_FX, M_SPD, "melk_ox", true),
    v("MELK", 0xFFF3, 0xFFF4, "7e 00 04 01 00 00 00 00 ef", G_OFF, "7e 04 01 ii ff 00 ff 00 ef", G_COL, M_FX, M_SPD, "melk", true),
    v("JACKYLED", 0xFFE1, 0xFFE1, "7e 04 04 01 ff ff ff 00 ef", "7e 04 04 00 00 00 00 00 ef", "7e 04 01 ii ff ff ff 00 ef", "7e 07 05 03 rr bb gg 00 ef", "7e 05 03 vv 03 ff ff 00 ef", G_SPD, "elk", false),
    v("XROCKER", 0xFFE1, 0xFFE1, "7b ff 04 01 ff ff ff ff bf", "7b ff 04 00 ff ff ff ff bf", "7b 04 01 ii ff ff ff ff bf", "7b ff 05 03 rr gg bb ff bf", "7b ff 03 vv 03 ff ff ff bf", "7b ff 02 vv ff ff ff ff bf", "elk", false),
    v("DMRRBA-007", NUS, NUS, "5a 01 02 01", "5a 01 02 00", "5a 03 01 ii 00", "5a 07 01 rr gg bb", "5a 04 01 vv", "5a 04 04 vv", "dmrrba", false),
    v("LEDBLE", 0xFFE1, 0xFFE2, "7e 00 04 01 00 00 00 00 ef", G_OFF, "7e 04 01 ii ff 00 ff 00 ef", G_COL, "7e 00 03 vv 03 00 00 00 00 ef", G_SPD, "elk", false),
    v("LED-", 0xFFE1, 0xFFE1, "7e 04 04 01 ff ff ff 00 ef", "7e 04 04 00 00 00 00 00 ef", "7e 04 01 ii ff ff ff 00 ef", "7e 07 05 03 rr bb gg 00 ef", "7e 05 03 vv 03 ff ff 00 ef", G_SPD, "elk", false),
    // unknown ELK firmwares get the most common frames
    v("ELK-", 0xFFF3, 0xFFF4, G_ON, G_OFF, "7e 00 01 ii ff 00 ff 00 ef", G_COL, G_FX, G_SPD, "elk", false),
];

const NUS_TX: GUID = GUID::from_u128(0x6e400002_b5a3_f393_e0a9_e50e24dcca9e);
const NUS_RX: GUID = GUID::from_u128(0x6e400003_b5a3_f393_e0a9_e50e24dcca9e);

/// The variant for an advertised name (upper case); `LEDBLE` here is only reached from the
/// ambiguous LEDBLE family.
pub fn variant(up: &str) -> Option<&'static Variant> {
    VARIANTS.iter().find(|v| up.starts_with(v.name) && v.name != "LEDBLE")
}

fn variant_any(up: &str) -> &'static Variant {
    VARIANTS.iter().find(|v| up.starts_with(v.name)).unwrap_or(&VARIANTS[VARIANTS.len() - 1])
}

/// Fills a template.
fn fill(t: &str, i: u8, v: u8, c: [u8; 3]) -> Vec<u8> {
    t.split_whitespace()
        .map(|x| match x {
            "ii" => i,
            "vv" => v,
            "rr" => c[0],
            "gg" => c[1],
            "bb" => c[2],
            h => u8::from_str_radix(h, 16).unwrap_or(0),
        })
        .collect()
}

pub struct Elk {
    v: &'static Variant,
}

impl Elk {
    pub fn new(name: &str) -> Self {
        Self { v: variant_any(&name.to_uppercase()) }
    }
}

impl BleProto for Elk {
    fn gatt(&self) -> Gatt {
        let (wr, nt) = if self.v.write == NUS { (NUS_TX, NUS_RX) } else { (u16id(self.v.write), u16id(self.v.notify)) };
        Gatt { write: vec![wr], write2: None, notify: vec![nt], response: false, gap: 50 }
    }

    fn caps(&self) -> Caps {
        Caps { state: false, color: !self.v.col.is_empty(), fx: self.v.table, speed: !self.v.spd.is_empty(), ..Default::default() }
    }

    fn hello(&mut self) -> Vec<Frame> {
        if self.v.login { vec![w(vec![0x7e, 0x07, 0x83]), w(vec![0x7e, 0x04, 0x04]), w(fill(self.v.on, 0, 0, [0; 3]))] } else { Vec::new() }
    }

    fn frames(&mut self, op: &Op, s: &State) -> Vec<Frame> {
        let t = |tpl: &str, i: u8, v: u8, c: [u8; 3]| if tpl.is_empty() { vec![] } else { vec![w(fill(tpl, i, v, c))] };
        match op {
            Op::Power(on) => t(if *on { self.v.on } else { self.v.off }, 0, 0, [0; 3]),
            Op::Bright(p) => t(self.v.br, (*p).clamp(1, 100), 0, [0; 3]),
            Op::Color(c) => t(self.v.col, 0, 0, *c),
            Op::Stream(c, b) => {
                let mut v = t(self.v.col, 0, 0, *c);
                if *b != s.bright {
                    v.extend(t(self.v.br, (*b).clamp(1, 100), 0, [0; 3]));
                }
                v
            }
            Op::Effect(e) => t(self.v.fx, 0, *e as u8, [0; 3]),
            Op::Speed(p) => t(self.v.spd, 0, (*p).clamp(1, 100), [0; 3]),
            _ => vec![],
        }
    }

    /// Some firmwares report `7e ?? 01 <power> r g b <brightness %> ef` on their own.
    fn notified(&mut self, d: &[u8], s: &mut State) -> bool {
        if d.len() != 9 || d[0] != 0x7e || d[8] != 0xef || d[2] != 0x01 {
            return false;
        }
        match d[3] {
            0x23 | 0xf0 | 0x01 => s.power = true,
            0x24 | 0x00 => s.power = false,
            _ => return false,
        }
        if d[4..7] != [0xff; 3] {
            s.rgb = [d[4], d[5], d[6]];
        }
        if d[7] != 0xff {
            s.bright = d[7].clamp(1, 100);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_and_frames() {
        assert_eq!(variant("ELK-BLEDOM0E-12").unwrap().name, "ELK-BLEDOM0E");
        assert_eq!(variant("ELK-BLEDOM").unwrap().name, "ELK-BLEDOM");
        assert!(variant("LEDBLE-1234").is_none());
        let mut e = Elk::new("ELK-BLEDOM");
        let s = State::default();
        assert_eq!(e.frames(&Op::Color([1, 2, 3]), &s), vec![w(vec![0x7e, 0, 5, 3, 1, 2, 3, 0, 0xef])]);
        assert_eq!(e.frames(&Op::Bright(50), &s), vec![w(vec![0x7e, 0, 1, 50, 0xff, 0, 0xff, 0, 0xef])]);
        let mut j = Elk::new("JACKYLED-1");
        assert_eq!(j.frames(&Op::Color([1, 2, 3]), &s)[0].1[4..7], [1, 3, 2]);
    }
}
