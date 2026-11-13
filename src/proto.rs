//! BanlanX SP621E protocol (BLE service FFE0, characteristic FFE1: write with response + notify).
//! Commands are `A0 <op> <len> <data…>`. The state answer to `A0 70 00` comes as notifications
//! `53 43 <packet #> <total len> <chunk len> <chunk…>`; the chunks joined give the payload.
//! Timer set / delete were taken from the BanlanX app (Android HCI log) and checked on the lamp.

use serde::{Deserialize, Serialize};

pub const SOLID: u8 = 0xBE;
pub const MAX_SPEED: u8 = 10;
pub const MAX_LENGTH: u8 = 150;
/// Payload bytes before the timer list.
const HEAD: usize = 23;
const TIMER_LEN: usize = 7;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Timer {
    pub idx: u8,
    /// true — turns the lamp on, false — off
    pub on: bool,
    /// bit 0 Monday … bit 6 Sunday; 0x80 — once
    pub days: u8,
    /// seconds since midnight
    pub sec: u32,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct State {
    pub power: bool,
    /// 0 — one effect, 1 — cycle through the effects
    pub mode: u8,
    pub effect: u8,
    /// index into RGB permutations: RGB RBG GRB GBR BRG BGR
    pub order: u8,
    pub bright: u8,
    pub speed: u8,
    pub length: u8,
    pub rgb: [u8; 3],
    pub timers: Vec<Timer>,
}

pub fn query() -> Vec<u8> {
    vec![0xA0, 0x70, 0x00]
}

pub fn power(on: bool) -> Vec<u8> {
    vec![0xA0, 0x62, 0x01, on as u8]
}

pub fn effect(id: u8) -> Vec<u8> {
    vec![0xA0, 0x63, 0x01, id]
}

pub fn order(i: u8) -> Vec<u8> {
    vec![0xA0, 0x64, 0x01, i.min(5)]
}

pub fn brightness(v: u8) -> Vec<u8> {
    vec![0xA0, 0x66, 0x01, v]
}

pub fn speed(v: u8) -> Vec<u8> {
    vec![0xA0, 0x67, 0x01, v.clamp(1, MAX_SPEED)]
}

pub fn length(v: u8) -> Vec<u8> {
    vec![0xA0, 0x68, 0x01, v.clamp(1, MAX_LENGTH)]
}

/// Colour with the brightness level; the lamp shows it only with the solid effect.
pub fn color(rgb: [u8; 3], level: u8) -> Vec<u8> {
    vec![0xA0, 0x69, 0x04, rgb[0], rgb[1], rgb[2], level]
}

pub fn cycle(on: bool) -> Vec<u8> {
    vec![0xA0, 0x6A, 0x01, on as u8]
}

/// Creates or replaces timer `t.idx`; `now_sec` / `weekday` (Monday = 0) set the lamp clock.
pub fn timer_set(t: &Timer, now_sec: u32, weekday: u8) -> Vec<u8> {
    let n = now_sec.to_be_bytes();
    let s = t.sec.to_be_bytes();
    vec![0xA0, 0x6E, 0x0C, n[1], n[2], n[3], weekday, t.idx, 0x00, t.enabled as u8, s[1], s[2], s[3], t.on as u8, t.days]
}

/// The advertised name (sniffed from the BanlanX app: `A0 61 <len> <name>`); UTF-8, cut at
/// NAME_MAX bytes on a character boundary.
pub fn rename(name: &str) -> Vec<u8> {
    let mut end = 0;
    for (i, c) in name.char_indices() {
        if i + c.len_utf8() > NAME_MAX {
            break;
        }
        end = i + c.len_utf8();
    }
    let b = &name.as_bytes()[..end];
    let mut v = vec![0xA0, 0x61, b.len() as u8];
    v.extend_from_slice(b);
    v
}

pub const NAME_MAX: usize = 16;

pub fn timer_delete(idx: u8) -> Vec<u8> {
    vec![0xA0, 0x6F, 0x01, idx]
}

/// Joins the notification chunks of one state answer.
#[derive(Default)]
pub struct Reassembler {
    buf: Vec<u8>,
    total: usize,
    next: u8,
}

impl Reassembler {
    /// Returns the full payload once the last chunk arrived.
    pub fn feed(&mut self, d: &[u8]) -> Option<Vec<u8>> {
        if d.len() < 5 || d[0] != 0x53 || d[1] != 0x43 {
            return None;
        }
        let (pkt, total, len) = (d[2], d[3] as usize, d[4] as usize);
        let chunk = &d[5..(5 + len).min(d.len())];
        if pkt == 1 {
            self.buf.clear();
            self.total = total;
            self.next = 1;
        } else if pkt != self.next || total != self.total {
            self.buf.clear(); // out of sequence: drop until the next answer starts
            self.next = 0;
            return None;
        }
        self.buf.extend_from_slice(chunk);
        self.next = pkt.wrapping_add(1);
        if self.buf.len() >= self.total {
            self.next = 0;
            let mut p = std::mem::take(&mut self.buf);
            p.truncate(self.total);
            return Some(p);
        }
        None
    }
}

pub fn parse(p: &[u8]) -> Option<State> {
    if p.len() < HEAD {
        return None;
    }
    let n = p[22] as usize;
    let timers = p[HEAD..]
        .chunks_exact(TIMER_LEN)
        .take(n)
        .map(|t| Timer { idx: t[0], on: t[1] == 1, days: t[2], sec: u32::from_be_bytes([0, t[3], t[4], t[5]]), enabled: t[6] == 1 })
        .collect();
    Some(State { power: p[0] == 1, mode: p[1], effect: p[2], order: p[3], bright: p[4], speed: p[5], length: p[6], rgb: [p[7], p[8], p[9]], timers })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace().map(|x| u8::from_str_radix(x, 16).unwrap()).collect()
    }

    #[test]
    fn state_from_live_lamp() {
        let mut r = Reassembler::default();
        assert!(r.feed(&hex("53 43 01 25 0f 01 00 be 02 ff 0a 83 ff 00 ff 01 10 09 04 0b")).is_none());
        assert!(r.feed(&hex("53 43 02 25 0f 14 1a 32 37 50 53 73 02 00 01 7f 00 e1 00 01")).is_none());
        let p = r.feed(&hex("53 43 03 25 07 01 00 7f 00 2a a8 01")).unwrap();
        let s = parse(&p).unwrap();
        assert!(s.power);
        assert_eq!((s.mode, s.effect, s.order, s.bright, s.speed, s.length), (0, SOLID, 2, 255, 10, 131));
        assert_eq!(s.rgb, [255, 0, 255]);
        assert_eq!(s.timers, vec![
            Timer { idx: 0, on: true, days: 0x7f, sec: 16 * 3600, enabled: true },
            Timer { idx: 1, on: false, days: 0x7f, sec: 3 * 3600 + 2 * 60, enabled: true },
        ]);
    }

    #[test]
    fn out_of_sequence_is_dropped() {
        let mut r = Reassembler::default();
        assert!(r.feed(&hex("53 43 02 25 0f 14 1a 32 37 50 53 73 02 00 01 7f 00 e1 00 01")).is_none());
        assert!(r.feed(&hex("53 43 03 25 07 01 00 7f 00 2a a8 01")).is_none());
    }

    #[test]
    fn timer_matches_banlanx_app() {
        // sniffed: off timer 03:05 daily, enabled, sent at 00:06:13 on Saturday
        let t = Timer { idx: 1, on: false, days: 0x7f, sec: 3 * 3600 + 5 * 60, enabled: true };
        assert_eq!(timer_set(&t, 6 * 60 + 13, 5), hex("a0 6e 0c 00 01 75 05 01 00 01 00 2b 5c 00 7f"));
        assert_eq!(timer_delete(3), hex("a0 6f 01 03"));
    }

    #[test]
    fn rename_matches_banlanx_app() {
        // sniffed: the app renaming the lamp to "Globallll"
        assert_eq!(rename("Globallll"), hex("a0 61 09 47 6c 6f 62 61 6c 6c 6c 6c"));
        assert_eq!(rename("Торшер у вітальні").len(), 3 + 16);
    }
}
