//! Govee lights over Bluetooth: 20-byte frames `33 <cmd> <data…> 00… <xor>` written without
//! response to …2b11; a keep-alive `aa 01` keeps the link. After govee_ble_lights and
//! govee_btled. Segmented (RGBIC) models take colour as `33 05 15 01 r g b … ff 7f`.

use super::{BleProto, Frame, Gatt, w};
use crate::lamp::{Caps, Op, State, byte};
use windows::core::GUID;

const CTL: GUID = GUID::from_u128(0x00010203_0405_0607_0809_0a0b0c0d2b11);
const NTF: GUID = GUID::from_u128(0x00010203_0405_0607_0809_0a0b0c0d2b10);
const SEGMENTED: [&str; 4] = ["H6053", "H6072", "H6102", "H6199"];

pub fn frame(head: u8, cmd: u8, data: &[u8]) -> Vec<u8> {
    let mut v = vec![head, cmd];
    v.extend_from_slice(&data[..data.len().min(17)]);
    v.resize(19, 0);
    let x = v.iter().fold(0u8, |a, b| a ^ b);
    v.push(x);
    v
}

pub struct Govee {
    seg: bool,
}

impl Govee {
    pub fn new(model: &str) -> Self {
        Self { seg: SEGMENTED.iter().any(|m| model.contains(m)) }
    }

    fn color(&self, c: [u8; 3]) -> Frame {
        w(if self.seg { frame(0x33, 0x05, &[0x15, 0x01, c[0], c[1], c[2], 0, 0, 0, 0, 0, 0xFF, 0x7F]) } else { frame(0x33, 0x05, &[0x02, c[0], c[1], c[2]]) })
    }
}

impl BleProto for Govee {
    fn gatt(&self) -> Gatt {
        Gatt { write: vec![CTL], write2: None, notify: vec![NTF], response: false, gap: 20 }
    }

    fn caps(&self) -> Caps {
        Caps { color: true, ..Default::default() }
    }

    fn frames(&mut self, op: &Op, s: &State) -> Vec<Frame> {
        match op {
            Op::Power(on) => vec![w(frame(0x33, 0x01, &[*on as u8]))],
            Op::Bright(p) => vec![w(frame(0x33, 0x04, &[byte(*p).max(1)]))],
            Op::Color(c) => vec![self.color(*c)],
            Op::Stream(c, b) => {
                let mut v = vec![self.color(*c)];
                if *b != s.bright {
                    v.push(w(frame(0x33, 0x04, &[byte(*b).max(1)])));
                }
                v
            }
            _ => vec![],
        }
    }

    fn keepalive(&self) -> Option<Frame> {
        Some(w(frame(0xAA, 0x01, &[])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_match_known_packets() {
        let on = frame(0x33, 0x01, &[1]);
        assert_eq!(on.len(), 20);
        assert_eq!(on[19], 0x33);
        assert_eq!(frame(0x33, 0x04, &[0xFF])[19], 0xC8);
        assert_eq!(frame(0x33, 0x05, &[0x02, 0xFF, 0, 0])[19], 0xCB);
        assert_eq!(frame(0x33, 0x05, &[0x15, 0x01, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0x7F])[19], 0x5D);
    }
}
