//! What every lamp family shares: the state shown in the window, the operations the app asks for
//! and what a family can do. Drivers turn operations into their own protocol.

use serde::Serialize;

pub use crate::proto::Timer;

/// Lamp state in family-neutral units.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct State {
    pub power: bool,
    /// 0..=100 %
    pub bright: u8,
    pub rgb: [u8; 3],
    /// None — one colour; Some — effect id of the family table
    pub effect: Option<u16>,
    /// 1..=100 %
    pub speed: u8,
    /// pixels lit by moving effects (families with `Caps::length`)
    pub length: u8,
    /// colour order index: RGB RBG GRB GBR BRG BGR
    pub order: u8,
    /// the lamp cycles through its effects by itself
    pub cycle: bool,
    pub timers: Vec<Timer>,
}

/// One request to the lamp. Brightness and speed are %, colours are what the lamp should emit.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Power(bool),
    Bright(u8),
    Color([u8; 3]),
    Effect(u16),
    Speed(u8),
    Length(u8),
    Order(u8),
    Cycle(bool),
    TimerSet(Timer),
    TimerDel(u8),
    /// live colour from the PC sound or screen: as cheap as the protocol allows, no mode switch
    Stream([u8; 3], u8),
    /// the name the lamp advertises (families with `Caps::rename`)
    Rename(String),
}

impl Op {
    /// Ops of one kind replace a waiting one (slider drags, live colour).
    pub fn kind(&self) -> (u8, u8) {
        match self {
            Op::Power(_) => (1, 0),
            Op::Bright(_) => (2, 0),
            Op::Color(_) => (3, 0),
            Op::Effect(_) => (4, 0),
            Op::Speed(_) => (5, 0),
            Op::Length(_) => (6, 0),
            Op::Order(_) => (7, 0),
            Op::Cycle(_) => (8, 0),
            Op::TimerSet(t) => (9, t.idx),
            Op::TimerDel(i) => (10, *i),
            Op::Stream(..) => (11, 0),
            Op::Rename(_) => (12, 0),
        }
    }
}

/// What a family supports (the window hides the rest).
#[derive(Clone, Debug, Default, Serialize)]
pub struct Caps {
    /// the lamp reports its state (otherwise the window shows what was last sent)
    pub state: bool,
    pub color: bool,
    /// effect table key in web/fx.json, or "dyn" (list read from the device)
    pub fx: &'static str,
    pub speed: bool,
    pub length: u8,
    pub order: bool,
    pub cycle: bool,
    pub timers: bool,
    /// the lamp can be renamed (its Bluetooth name)
    pub rename: bool,
}

/// Applies `op` to the state as the lamp will have it (shown before any answer).
pub fn apply(s: &mut State, op: &Op) {
    match op {
        Op::Power(on) => s.power = *on,
        Op::Bright(b) => s.bright = *b,
        Op::Color(c) => {
            s.rgb = *c;
            s.effect = None;
            s.cycle = false;
        }
        Op::Effect(e) => {
            s.effect = Some(*e);
            s.cycle = false;
        }
        Op::Speed(v) => s.speed = *v,
        Op::Length(v) => s.length = *v,
        Op::Order(v) => s.order = *v,
        Op::Cycle(on) => s.cycle = *on,
        Op::TimerSet(t) => {
            s.timers.retain(|x| x.idx != t.idx);
            s.timers.push(t.clone());
            s.timers.sort_by_key(|x| x.idx);
        }
        Op::TimerDel(i) => s.timers.retain(|x| x.idx != *i),
        Op::Stream(..) | Op::Rename(_) => {}
    }
}

/// 0..=255 → 0..=100 (rounded)
pub fn pct(v: u8) -> u8 {
    ((v as u32 * 100 + 127) / 255) as u8
}

/// 0..=100 → 0..=255
pub fn byte(p: u8) -> u8 {
    ((p.min(100) as u32 * 255 + 50) / 100) as u8
}

/// % → a 1..=max scale (speed 1..10 and the like)
pub fn to_scale(p: u8, max: u8) -> u8 {
    ((p.clamp(1, 100) as u32 * max as u32 + 50) / 100).clamp(1, max as u32) as u8
}

pub fn from_scale(v: u8, max: u8) -> u8 {
    ((v.min(max) as u32 * 100 + max as u32 / 2) / max.max(1) as u32).clamp(1, 100) as u8
}

/// Colour picked on the screen → what the LEDs should get: screen values carry gamma, LED PWM
/// is linear, so a raw 128 looks much brighter and paler on the lamp than on the screen.
/// `cal` scales the channels for a neutral white.
pub fn to_led(rgb: [u8; 3], gamma: bool, cal: [f32; 3]) -> [u8; 3] {
    let mut o = [0u8; 3];
    for i in 0..3 {
        let mut v = rgb[i] as f32 / 255.0;
        if gamma {
            v = v.powf(2.2);
        }
        o[i] = (v * cal[i].clamp(0.2, 1.0) * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    // a colour must not vanish to black after the curve
    if o == [0, 0, 0] && rgb != [0, 0, 0] {
        let m = rgb.iter().enumerate().max_by_key(|x| x.1).map(|x| x.0).unwrap_or(0);
        o[m] = 1;
    }
    o
}

/// The reverse of `to_led` for colours the lamp reports (the window shows screen colours).
pub fn from_led(rgb: [u8; 3], gamma: bool, cal: [f32; 3]) -> [u8; 3] {
    let mut o = [0u8; 3];
    for i in 0..3 {
        let mut v = rgb[i] as f32 / 255.0 / cal[i].clamp(0.2, 1.0);
        if gamma {
            v = v.clamp(0.0, 1.0).powf(1.0 / 2.2);
        }
        o[i] = (v * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    o
}

/// Hue (0..360), saturation and value (0..1) of a colour.
pub fn hsv(c: [u8; 3]) -> (f32, f32, f32) {
    let [r, g, b] = c.map(|x| x as f32 / 255.0);
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let d = mx - mn;
    let h = if d == 0.0 {
        0.0
    } else if mx == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if mx == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, if mx == 0.0 { 0.0 } else { d / mx }, mx)
}

pub fn from_hsv(h: f32, s: f32, v: f32) -> [u8; 3] {
    let f = |n: f32| {
        let k = (n + h / 60.0).rem_euclid(6.0);
        ((v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)) * 255.0).round() as u8
    };
    [f(5.0), f(3.0), f(1.0)]
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gamma_round_trip() {
        let c = [255, 106, 0];
        let led = to_led(c, true, [1.0; 3]);
        assert_eq!(led, [255, 37, 0]);
        let back = from_led(led, true, [1.0; 3]);
        assert!((back[1] as i32 - 106).abs() <= 2);
        assert_eq!(to_led([3, 0, 0], true, [1.0; 3]), [1, 0, 0]);
    }

    #[test]
    fn scales() {
        assert_eq!(to_scale(100, 10), 10);
        assert_eq!(to_scale(1, 10), 1);
        assert_eq!(from_scale(10, 10), 100);
        assert_eq!(pct(255), 100);
        assert_eq!(byte(50), 128);
    }

    #[test]
    fn hsv_round_trip() {
        let (h, s, v) = hsv([255, 106, 0]);
        assert_eq!(from_hsv(h, s, v), [255, 106, 0]);
    }
}
