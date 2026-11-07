//! Lamp families. Bluetooth families turn operations into frames for a GATT characteristic
//! (`BleProto`); network families talk to the lamp themselves (`NetProto`). Recognition from
//! adverts and network replies lives here too.
//! Only BanlanX SP621E is checked on a real lamp; the rest follow open-source implementations
//! (UniLED, led-ble / flux_led, elkbledom, lednetwf_ble, idealLED, bj_led, ledble-ledlamp,
//! curl_smile, govee_ble_lights, the vendors' LAN APIs) and are marked unverified in the window.

pub mod ble_banlanx;
pub mod ble_elk;
pub mod ble_govee;
pub mod ble_ideal;
pub mod ble_misc;
pub mod ble_triones;
pub mod ble_zengge;
pub mod http;
pub mod mdns;
pub mod net_banlanx;
pub mod net_govee;
pub mod net_hue;
pub mod net_lifx;
pub mod net_magichome;
pub mod net_nanoleaf;
pub mod net_wiz;
pub mod net_wled;
pub mod net_yeelight;

use serde_json::Value;
use windows::core::GUID;

use crate::lamp::{Caps, Op, State};

/// A frame for write characteristic `0` (or `1` where a family has a second one).
pub type Frame = (u8, Vec<u8>);

pub fn w(d: Vec<u8>) -> Frame {
    (0, d)
}

/// 16-bit Bluetooth SIG UUID in full form.
pub const fn u16id(x: u16) -> GUID {
    GUID::from_u128(((x as u128) << 96) | 0x0000_0000_0000_1000_8000_00805f9b34fb)
}

pub struct Gatt {
    /// candidates for characteristic 0, the first one found is used
    pub write: Vec<GUID>,
    /// second write characteristic (iDeal colour data)
    pub write2: Option<GUID>,
    pub notify: Vec<GUID>,
    /// write with response (the lamp acknowledges every frame)
    pub response: bool,
    /// pause between frames, ms (cheap clones drop frames sent back to back)
    pub gap: u64,
}

pub trait BleProto: Send {
    fn gatt(&self) -> Gatt;
    fn caps(&self) -> Caps;
    /// Frames right after connecting (handshakes).
    fn hello(&mut self) -> Vec<Frame> {
        Vec::new()
    }
    fn query(&self) -> Option<Frame> {
        None
    }
    /// Frames for `op`; `st` is the state before it.
    fn frames(&mut self, op: &Op, st: &State) -> Vec<Frame>;
    /// A notification; true when `st` was updated from it.
    fn notified(&mut self, _d: &[u8], _st: &mut State) -> bool {
        false
    }
    /// Sent every couple of seconds while connected (lamps that drop a quiet link).
    fn keepalive(&self) -> Option<Frame> {
        None
    }
}

pub trait NetProto: Send {
    fn caps(&self) -> Caps;
    fn apply(&mut self, op: &Op, st: &State) -> Result<(), String>;
    /// Reads the state if the lamp can tell it; Ok(false) — it cannot.
    fn refresh(&mut self, st: &mut State) -> Result<bool, String>;
    /// Effect names read from the device (caps.fx == "dyn").
    fn effects(&mut self) -> Vec<String> {
        Vec::new()
    }
    fn close(&mut self) {}
}

/// A lamp that answered on the local network.
#[derive(Clone, Debug)]
pub struct NetFound {
    pub family: &'static str,
    pub host: String,
    pub name: String,
    pub model: String,
    pub extra: Value,
}

impl NetFound {
    /// Stable id of the lamp in the saved list.
    pub fn id(&self) -> String {
        match self.family {
            "hue" => format!("hue:{}:{}", self.host, self.extra["light"].as_str().unwrap_or("")),
            f => format!("{f}:{}", self.host),
        }
    }
}

/// Asks every network family at once (about 1.5 s); `hue_users` maps bridge address → user.
pub fn discover_net(hue_users: &serde_json::Map<String, Value>) -> Vec<NetFound> {
    let hu = hue_users.clone();
    let jobs: Vec<std::thread::JoinHandle<Vec<NetFound>>> = vec![
        std::thread::spawn(|| run(net_magichome::discover)),
        std::thread::spawn(|| run(net_banlanx::discover)),
        std::thread::spawn(|| run(net_wled::discover)),
        std::thread::spawn(|| run(net_govee::discover)),
        std::thread::spawn(|| run(net_yeelight::discover)),
        std::thread::spawn(|| run(net_wiz::discover)),
        std::thread::spawn(|| run(net_lifx::discover)),
        std::thread::spawn(|| run(net_nanoleaf::discover)),
        std::thread::spawn(move || {
            let mut v = Vec::new();
            net_hue::discover(&mut v, &hu);
            v
        }),
    ];
    jobs.into_iter().filter_map(|j| j.join().ok()).flatten().collect()
}

fn run(f: fn(&mut Vec<NetFound>)) -> Vec<NetFound> {
    let mut v = Vec::new();
    f(&mut v);
    v
}

/// What an advert or a network reply told about a lamp.
#[derive(Clone, Debug, PartialEq)]
pub struct Ident {
    pub family: &'static str,
    pub model: String,
}

fn id(family: &'static str, model: impl Into<String>) -> Option<Ident> {
    Some(Ident { family, model: model.into() })
}

const BANLANX_MFR: [u16; 2] = [0x5053, 5053];

/// Recognises a lamp from its advert: manufacturer data first (BanlanX, Zengge), then the name.
/// `Some(family "unknown")` — looks like a light, the protocol is not known.
pub fn identify(name: &str, mfr: &[(u16, Vec<u8>)]) -> Option<Ident> {
    for (cid, d) in mfr {
        if BANLANX_MFR.contains(cid)
            && let Some(&code) = d.first()
            && let Some(i) = ble_banlanx::by_code(code)
        {
            return Some(i);
        }
        if (0x5A00..=0x5AFF).contains(cid) && d.len() == 27 {
            let pid = ((d[8] as u16) << 8) | d[9] as u16;
            return id("zengge", format!("Zengge 0x{pid:02X}"));
        }
    }
    let up = name.to_uppercase();
    let has = |p: &str| up.starts_with(p);
    if has("SP110E") {
        return id("sp110e", "SP110E");
    }
    if has("SP107E") {
        return id("sp107e", "SP107E");
    }
    if has("LEDNETWF") || has("IOTB") {
        return id("zengge", "Zengge");
    }
    if has("KS03~") {
        return id("keepsmile", "Keepsmile KS03");
    }
    if has("BJ_LED") {
        return id("mohuan", "MohuanLED");
    }
    if has("ISP-") || has("IDL-") {
        return id("ideal", "iDeal LED");
    }
    if has("GOVEE_") || has("IHOMENT_") || has("GBK_") {
        let m = name.split('_').nth(1).unwrap_or("Govee").to_string();
        return id("govee", format!("Govee {m}"));
    }
    // three protocols advertise as LEDBLE*; the characteristics decide after connecting
    if has("LEDBLE") {
        return id("ledble", "LEDBLE");
    }
    for p in ["LEDNET", "BLE-LED", "TRIONES", "LEDBLUE", "DREAM~", "QHM-", "AP-", "LD-0003"] {
        if has(p) {
            return id("triones", "Triones / HappyLighting");
        }
    }
    if let Some(v) = ble_elk::variant(&up) {
        return id("elk", format!("ELK ({})", v.name));
    }
    let low = name.to_lowercase();
    if ["led", "lamp", "light", "bulb", "strip", "rgb", "лампа"].iter().any(|k| low.contains(k)) {
        return id("unknown", "");
    }
    None
}

/// Families tried in order when connecting (ambiguous names).
pub fn candidates(family: &str, name: &str) -> Vec<&'static str> {
    match family {
        "ledble" => {
            if name.to_uppercase().starts_with("LEDBLE-01") {
                vec!["triones", "ledlamp", "elk"]
            } else {
                vec!["triones", "elk", "ledlamp"]
            }
        }
        f => ble_families().iter().copied().filter(|x| *x == f).collect(),
    }
}

pub fn ble_families() -> &'static [&'static str] {
    &["banlanx2", "banlanx3", "sp110e", "sp107e", "sp601e", "sp60xe", "sptech", "elk", "triones", "ledlamp", "zengge", "keepsmile", "mohuan", "ideal", "govee"]
}

/// A driver for a Bluetooth family; `model` and `name` pick variants inside it.
pub fn make_ble(family: &str, model: &str, name: &str) -> Option<Box<dyn BleProto>> {
    Some(match family {
        "banlanx2" | "banlanx3" | "sp110e" | "sp107e" | "sp601e" | "sp60xe" | "sptech" => Box::new(ble_banlanx::Banlanx::new(family, model)?),
        "elk" => Box::new(ble_elk::Elk::new(name)),
        "triones" => Box::new(ble_triones::Triones::new(name)),
        "zengge" => Box::new(ble_zengge::Zengge::new(name)),
        "govee" => Box::new(ble_govee::Govee::new(model)),
        "ideal" => Box::new(ble_ideal::Ideal::default()),
        "ledlamp" => Box::new(ble_misc::LedLamp::default()),
        "keepsmile" => Box::new(ble_misc::Keepsmile::default()),
        "mohuan" => Box::new(ble_misc::Mohuan::default()),
        _ => return None,
    })
}

/// A network driver; `host` is the address, `extra` keeps what pairing gave (tokens, ids).
pub fn make_net(family: &str, host: &str, extra: &Value) -> Option<Box<dyn NetProto>> {
    let s = |k: &str| extra.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    Some(match family {
        "magichome" => Box::new(net_magichome::MagicHome::new(host)),
        "banlanx_net" => Box::new(net_banlanx::BanlanxNet::new(host, extra.get("code").and_then(Value::as_u64).unwrap_or(0) as u8)),
        "wled" => Box::new(net_wled::Wled::new(host)),
        "govee_lan" => Box::new(net_govee::GoveeLan::new(host)),
        "yeelight" => Box::new(net_yeelight::Yeelight::new(host)),
        "wiz" => Box::new(net_wiz::Wiz::new(host)),
        "lifx" => Box::new(net_lifx::Lifx::new(host, &s("mac"))),
        "nanoleaf" => Box::new(net_nanoleaf::Nanoleaf::new(host, &s("token"))),
        "hue" => Box::new(net_hue::Hue::new(host, &s("user"), &s("light"))),
        _ => return None,
    })
}

/// The one model checked on a real lamp.
pub fn verified(family: &str, model: &str) -> bool {
    family == "banlanx2" && model == "SP621E"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies() {
        // the author's lamp: name LEDildo, BanlanX data 0d 00 …
        assert_eq!(identify("LEDildo", &[(0x5053, vec![0x0d, 0x00, 0x01])]), id("banlanx2", "SP621E"));
        assert_eq!(identify("", &[(0x5053, vec![0x27, 0x10])]).unwrap().family, "sptech");
        assert_eq!(identify("ELK-BLEDOM", &[]).unwrap().family, "elk");
        assert_eq!(identify("LEDBLE-01-9201", &[]).unwrap().family, "ledble");
        assert_eq!(candidates("ledble", "LEDBLE-01-9201")[1], "ledlamp");
        assert_eq!(identify("ihoment_H6076_ABCD", &[]).unwrap().model, "Govee H6076");
        assert_eq!(identify("QHM-095F", &[]).unwrap().family, "triones");
        assert_eq!(identify("My LED strip", &[]).unwrap().family, "unknown");
        assert_eq!(identify("Pixel 8", &[]), None);
        assert_eq!(u16id(0xffe1), GUID::from_u128(0x0000ffe1_0000_1000_8000_00805f9b34fb));
    }
}
