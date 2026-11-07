//! iDeal LED (names ISP-* / IDL-*): 16-byte commands encrypted with AES-128-ECB (a fixed key from
//! the Android app) on …9600, effect palettes in plain text on …960a. "TYPE2" firmware commands
//! after github.com/8none1/idealLED; colour channels are 5-bit, brightness scales them.

use super::{BleProto, Frame, Gatt};
use crate::lamp::{Caps, Op, State, from_hsv};
use windows::{
    Win32::Security::Cryptography::{BCRYPT_AES_ALGORITHM, BCRYPT_ALG_HANDLE, BCRYPT_CHAIN_MODE_ECB, BCRYPT_CHAINING_MODE, BCRYPT_FLAGS, BCRYPT_KEY_HANDLE, BCryptCloseAlgorithmProvider, BCryptDecrypt, BCryptDestroyKey, BCryptEncrypt, BCryptGenerateSymmetricKey, BCryptOpenAlgorithmProvider, BCryptSetProperty},
    core::GUID,
};

const CMD: GUID = GUID::from_u128(0xd44bc439_abfd_45a2_b575_925416129600);
const COL: GUID = GUID::from_u128(0xd44bc439_abfd_45a2_b575_92541612960a);
const NTF: GUID = GUID::from_u128(0xd44bc439_abfd_45a2_b575_925416129601);
const KEY: [u8; 16] = [0x34, 0x52, 0x2A, 0x5B, 0x7A, 0x6E, 0x49, 0x2C, 0x08, 0x09, 0x0A, 0x9D, 0x8D, 0x2A, 0x23, 0xF8];

/// AES-128-ECB of one 16-byte block with Windows CNG.
pub fn aes(block: &[u8; 16], decrypt: bool) -> Option<[u8; 16]> {
    unsafe {
        let mut alg = BCRYPT_ALG_HANDLE::default();
        BCryptOpenAlgorithmProvider(&mut alg, BCRYPT_AES_ALGORITHM, None, Default::default()).ok().ok()?;
        let mode: Vec<u16> = BCRYPT_CHAIN_MODE_ECB.as_wide().iter().copied().chain([0]).collect();
        let mode_b = std::slice::from_raw_parts(mode.as_ptr() as *const u8, mode.len() * 2);
        let mut out = [0u8; 16];
        let mut ok = BCryptSetProperty(alg.into(), BCRYPT_CHAINING_MODE, mode_b, 0).is_ok();
        let mut key = BCRYPT_KEY_HANDLE::default();
        ok = ok && BCryptGenerateSymmetricKey(alg, &mut key, None, &KEY, 0).is_ok();
        if ok {
            let mut n = 0u32;
            let r = if decrypt { BCryptDecrypt(key, Some(block), None, None, Some(&mut out), &mut n, BCRYPT_FLAGS(0)) } else { BCryptEncrypt(key, Some(block), None, None, Some(&mut out), &mut n, BCRYPT_FLAGS(0)) };
            ok = r.is_ok() && n == 16;
            let _ = BCryptDestroyKey(key);
        }
        let _ = BCryptCloseAlgorithmProvider(alg, 0);
        ok.then_some(out)
    }
}

#[derive(Default)]
pub struct Ideal {
    base: [u8; 3],
}

fn enc(p: [u8; 16]) -> Frame {
    (0, aes(&p, false).map(|x| x.to_vec()).unwrap_or_default())
}

impl Ideal {
    fn color(&self, c: [u8; 3], bright: u8) -> Frame {
        let k = bright.clamp(1, 100) as u32;
        let s = c.map(|x| ((x as u32 * k / 100) >> 3) as u8);
        enc([0x0F, 0x53, 0x47, 0x4C, 0x53, 0x00, 0x00, 0x64, 0x50, s[0], s[1], s[2], 0x1F, 0x00, 0x00, 0x32])
    }
}

impl BleProto for Ideal {
    fn gatt(&self) -> Gatt {
        Gatt { write: vec![CMD], write2: Some(COL), notify: vec![NTF], response: false, gap: 30 }
    }

    fn caps(&self) -> Caps {
        Caps { color: true, fx: "ideal", speed: true, ..Default::default() }
    }

    fn frames(&mut self, op: &Op, s: &State) -> Vec<Frame> {
        match op {
            Op::Power(on) => vec![enc([0x05, 0x54, 0x55, 0x52, 0x4E, *on as u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])],
            Op::Color(c) => {
                self.base = *c;
                vec![self.color(*c, s.bright)]
            }
            Op::Stream(c, b) => vec![self.color(*c, *b)],
            Op::Bright(p) => match s.effect {
                Some(e) => self.effect(e, s.speed, *p),
                None => vec![self.color(self.base, *p)],
            },
            Op::Effect(e) => self.effect(*e, s.speed, s.bright),
            Op::Speed(v) => s.effect.map(|e| self.effect(e, *v, s.bright)).unwrap_or_default(),
            _ => vec![],
        }
    }
}

impl Ideal {
    /// The effect command and its 7-colour palette (plain text on the colour characteristic).
    fn effect(&self, id: u16, speed: u8, bright: u8) -> Vec<Frame> {
        let b = bright.clamp(1, 100);
        let mut pal = vec![21u8, 0];
        for i in 0..7 {
            pal.extend(from_hsv(i as f32 * 51.0, 1.0, b as f32 / 100.0));
        }
        vec![enc([0x0A, 0x4D, 0x55, 0x4C, 0x54, id as u8, 0x00, 0x64, speed.clamp(1, 100), 0x07, b, 0, 0, 0, 0, 0]), (1, pal)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aes_matches_capture() {
        // decrypting a captured colour packet gives the documented red colour command
        let red = [0x0F, 0x53, 0x47, 0x4C, 0x53, 0x00, 0x00, 0x64, 0x50, 0x1F, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x32];
        let e = aes(&red, false).unwrap();
        assert_eq!(aes(&e, true).unwrap(), red);
        assert_ne!(e, red);
    }
}
