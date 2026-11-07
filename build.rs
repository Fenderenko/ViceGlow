//! Windows resources for the exe: the icon (drawn here: a neon spiral lighting an orb) and the
//! version info. Written as a .res file that link.exe takes directly; the version comes from
//! Cargo.toml.

use std::{env, fs, path::PathBuf};

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
const LANG_EN: u16 = 0x0409;
const SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let ver = env::var("CARGO_PKG_VERSION").unwrap();
    let icons: Vec<(u32, Vec<u8>)> = SIZES.iter().map(|&s| (s, dib(s))).collect();
    let mut res = Vec::new();
    res.extend(entry(0, 0, 0, &[]));
    for (i, (_, d)) in icons.iter().enumerate() {
        res.extend(entry(RT_ICON, i as u16 + 1, 0, d));
    }
    res.extend(entry(RT_GROUP_ICON, 1, 0, &group_icon(&icons)));
    res.extend(entry(RT_VERSION, 1, LANG_EN, &version_info(&ver)));
    println!("cargo:rerun-if-env-changed=GLOWO_ICON_PNG");
    if let Ok(png) = env::var("GLOWO_ICON_PNG") {
        sheet(&png);
    }
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("viceglow.res");
    fs::write(&out, res).unwrap();
    println!("cargo:rustc-link-arg-bins={}", out.display());
}

/// The mark: a neon "G" that spirals in and lights an orb, on a transparent background
/// (24-unit box, the same path as the logo in web/index.html).
fn spiral() -> Vec<(f32, f32)> {
    let mut p = Vec::new();
    let a0 = (6.7f32 - 12.0).atan2(18.3 - 12.0);
    let n = 160;
    for i in 0..=n {
        // large arc, counter-clockwise on screen: from the upper right through the top to the right
        let a = a0 - (std::f32::consts::TAU + a0) * i as f32 / n as f32;
        p.push((12.0 + 8.2 * a.cos(), 12.0 + 8.2 * a.sin()));
    }
    let cubic = |p: &mut Vec<(f32, f32)>, a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)| {
        for i in 1..=60 {
            let t = i as f32 / 60.0;
            let u = 1.0 - t;
            let f = |k: usize| {
                let (pa, pb, pc, pd) = if k == 0 { (a.0, b.0, c.0, d.0) } else { (a.1, b.1, c.1, d.1) };
                u * u * u * pa + 3.0 * u * u * t * pb + 3.0 * u * t * t * pc + t * t * t * pd
            };
            p.push((f(0), f(1)));
        }
    };
    cubic(&mut p, (20.2, 12.0), (20.2, 17.2), (12.6, 18.6), (9.8, 14.8));
    cubic(&mut p, (9.8, 14.8), (8.4, 12.8), (9.6, 9.4), (11.9, 10.6));
    p
}

fn seg_dist(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l = dx * dx + dy * dy;
    let t = if l == 0.0 { 0.0 } else { (((px - a.0) * dx + (py - a.1) * dy) / l).clamp(0.0, 1.0) };
    ((px - a.0 - t * dx).powi(2) + (py - a.1 - t * dy).powi(2)).sqrt()
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Violet → pink → peach along the diagonal.
fn tube_colour(x: f32, y: f32) -> [f32; 3] {
    let t = (((x + y) / 2.0 - 4.0) / 16.0).clamp(0.0, 1.0);
    let (v, p, o) = ([155.0, 107.0, 255.0], [255.0, 79.0, 176.0], [255.0, 159.0, 107.0]);
    if t < 0.6 { mix(v, p, t / 0.6) } else { mix(p, o, (t - 0.6) / 0.4) }
}

/// Premultiplied "over".
fn over(dst: &mut [f32; 4], c: [f32; 3], a: f32) {
    let a = a.clamp(0.0, 1.0);
    for k in 0..3 {
        dst[k] = c[k] * a + dst[k] * (1.0 - a);
    }
    dst[3] = a + dst[3] * (1.0 - a);
}

/// Colour of one pixel (premultiplied RGB 0..255, alpha 0..1), 4×4 samples.
fn pixel(s: f32, px: f32, py: f32, path: &[(f32, f32)]) -> [f32; 4] {
    let k = 24.0 / s; // units per pixel
    let small = s <= 20.0;
    let w = if small { 2.5 } else { 1.8 };
    let mut acc = [0f32; 4];
    for sy in 0..4 {
        for sx in 0..4 {
            let x = (px + (sx as f32 + 0.5) / 4.0) * k;
            let y = (py + (sy as f32 + 0.5) / 4.0) * k;
            let d = path.windows(2).map(|q| seg_dist(x, y, q[0], q[1])).fold(f32::MAX, f32::min);
            let r = ((x - 12.7).powi(2) + (y - 12.0).powi(2)).sqrt();
            let mut c = [0f32; 4];
            // soft glows
            over(&mut c, [255.0, 79.0, 176.0], 0.45 * (-(r * r) / (2.0 * 2.6 * 2.6)).exp());
            over(&mut c, [177.0, 92.0, 255.0], 0.42 * (-(d * d) / (2.0 * 1.7 * 1.7)).exp());
            let col = tube_colour(x, y);
            over(&mut c, col, 0.4 * (-((d - w * 0.5).max(0.0)).powi(2) / (2.0 * 0.7 * 0.7)).exp());
            // the tube and its hot core
            over(&mut c, col, (w / 2.0 - d) / k + 0.5);
            if !small {
                over(&mut c, [255.0, 255.0, 255.0], 0.7 * ((w * 0.11 - d) / k + 0.5).clamp(0.0, 1.0));
            }
            // the orb (radial white → pink)
            let ro = if small { 2.3 } else { 1.9 };
            let hl = ((x - 12.4).powi(2) + (y - 11.55).powi(2)).sqrt() / (ro * 1.25);
            let oc = if hl < 0.5 { mix([255.0; 3], [255.0, 224.0, 242.0], hl / 0.5) } else { mix([255.0, 224.0, 242.0], [255.0, 122.0, 198.0], ((hl - 0.5) / 0.5).min(1.0)) };
            over(&mut c, oc, (ro - r) / k + 0.5);
            for i in 0..4 {
                acc[i] += c[i];
            }
        }
    }
    acc.map(|v| v / 16.0)
}

/// One icon image as a DIB: BITMAPINFOHEADER, BGRA rows bottom-up, empty AND mask.
fn dib(s: u32) -> Vec<u8> {
    let mask_row = s.div_ceil(32) * 4;
    let mut b = Vec::new();
    for v in [40u32, s, s * 2] {
        b.extend(v.to_le_bytes());
    }
    b.extend(1u16.to_le_bytes());
    b.extend(32u16.to_le_bytes());
    for v in [0u32, s * s * 4 + mask_row * s, 0, 0, 0, 0] {
        b.extend(v.to_le_bytes());
    }
    let path = spiral();
    for y in (0..s).rev() {
        for x in 0..s {
            let p = pixel(s as f32, x as f32, y as f32, &path);
            let a = p[3];
            // premultiplied → straight alpha
            let un = |c: f32| if a > 0.001 { (c / a).round().clamp(0.0, 255.0) as u8 } else { 0 };
            let (r, g, bl) = (un(p[0]), un(p[1]), un(p[2]));
            b.extend([bl, g, r, (a * 255.0).round().clamp(0.0, 255.0) as u8]);
        }
    }
    b.extend(std::iter::repeat_n(0u8, (mask_row * s) as usize));
    b
}

fn group_icon(icons: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut b = Vec::new();
    for v in [0u16, 1, icons.len() as u16] {
        b.extend(v.to_le_bytes());
    }
    for (i, (s, d)) in icons.iter().enumerate() {
        let wh = if *s >= 256 { 0 } else { *s as u8 };
        b.extend([wh, wh, 0, 0]);
        b.extend(1u16.to_le_bytes());
        b.extend(32u16.to_le_bytes());
        b.extend((d.len() as u32).to_le_bytes());
        b.extend((i as u16 + 1).to_le_bytes());
    }
    b
}

fn pad4(b: &mut Vec<u8>) {
    while b.len() % 4 != 0 {
        b.push(0);
    }
}

/// One resource of a .res file: header (type and name by number) and data, both DWORD aligned.
fn entry(typ: u16, id: u16, lang: u16, data: &[u8]) -> Vec<u8> {
    let mut hdr = Vec::new();
    hdr.extend((data.len() as u32).to_le_bytes());
    hdr.extend(32u32.to_le_bytes());
    hdr.extend([0xFF, 0xFF]);
    hdr.extend(typ.to_le_bytes());
    hdr.extend([0xFF, 0xFF]);
    hdr.extend(id.to_le_bytes());
    hdr.extend(0u32.to_le_bytes());
    let flags: u16 = if typ == 0 { 0 } else { 0x1030 };
    hdr.extend(flags.to_le_bytes());
    hdr.extend(lang.to_le_bytes());
    hdr.extend(0u32.to_le_bytes());
    hdr.extend(0u32.to_le_bytes());
    hdr.extend(data);
    pad4(&mut hdr);
    hdr
}

fn utf16z(s: &str) -> Vec<u8> {
    s.encode_utf16().chain([0]).flat_map(|c| c.to_le_bytes()).collect()
}

/// VS_VERSIONINFO node: wLength, wValueLength, wType, szKey, padding, value, padding, children.
fn ver_node(key: &str, value: &[u8], text: bool, value_len: Option<u16>, children: &[Vec<u8>]) -> Vec<u8> {
    let mut b = vec![0, 0, 0, 0];
    b.extend((text as u16).to_le_bytes());
    b.extend(utf16z(key));
    pad4(&mut b);
    b.extend(value);
    for c in children {
        pad4(&mut b);
        b.extend(c);
    }
    let len = b.len() as u16;
    let vl = value_len.unwrap_or(value.len() as u16);
    b[0..2].copy_from_slice(&len.to_le_bytes());
    b[2..4].copy_from_slice(&vl.to_le_bytes());
    b
}

fn version_info(ver: &str) -> Vec<u8> {
    let mut nums: Vec<u32> = ver.split('-').next().unwrap().split('.').filter_map(|x| x.parse().ok()).take(4).collect();
    nums.resize(4, 0);
    let (ms, ls) = ((nums[0] << 16) | nums[1], (nums[2] << 16) | nums[3]);
    let mut fixed = Vec::new();
    for v in [0xFEEF04BDu32, 0x00010000, ms, ls, ms, ls, 0x3F, 0, 0x40004, 1, 0, 0, 0] {
        fixed.extend(v.to_le_bytes());
    }
    let strings = [
        ("CompanyName", "Fenderenko"),
        ("FileDescription", "Vice Glow"),
        ("FileVersion", ver),
        ("InternalName", "Vice Glow"),
        ("OriginalFilename", "ViceGlow.exe"),
        ("ProductName", "Vice Glow"),
        ("ProductVersion", ver),
    ];
    let nodes: Vec<Vec<u8>> = strings.iter().map(|(k, v)| ver_node(k, &utf16z(v), true, Some(v.encode_utf16().count() as u16 + 1), &[])).collect();
    let table = ver_node("040904B0", &[], true, None, &nodes);
    let sfi = ver_node("StringFileInfo", &[], true, None, &[table]);
    let mut tr = Vec::new();
    tr.extend(LANG_EN.to_le_bytes());
    tr.extend(0x04B0u16.to_le_bytes());
    let var = ver_node("Translation", &tr, false, None, &[]);
    let vfi = ver_node("VarFileInfo", &[], true, None, &[var]);
    ver_node("VS_VERSION_INFO", &fixed, false, None, &[sfi, vfi])
}

/// A check sheet: every size on a dark and a light taskbar colour, as PNG (GLOWO_ICON_PNG=path).
fn sheet(path: &str) {
    let path_pts = spiral();
    let sizes = [16u32, 20, 24, 32, 48, 64, 256];
    let (w, h) = (sizes.iter().map(|s| s + 16).sum::<u32>() + 16, 2 * (256 + 32));
    let mut img = vec![0u8; (w * h * 3) as usize];
    for (row, bg) in [[32u8, 32, 32], [243, 243, 243]].iter().enumerate() {
        for y in 0..h / 2 {
            for x in 0..w {
                let i = (((row as u32 * h / 2 + y) * w + x) * 3) as usize;
                img[i..i + 3].copy_from_slice(bg);
            }
        }
        let mut ox = 16;
        for &s in &sizes {
            let oy = row as u32 * h / 2 + 16;
            for y in 0..s {
                for x in 0..s {
                    let p = pixel(s as f32, x as f32, y as f32, &path_pts);
                    let i = (((oy + y) * w + ox + x) * 3) as usize;
                    for k in 0..3 {
                        img[i + k] = (p[k] + bg[k] as f32 * (1.0 - p[3])).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
            ox += s + 16;
        }
    }
    let mut raw = Vec::new();
    for y in 0..h {
        raw.push(0);
        raw.extend_from_slice(&img[(y * w * 3) as usize..((y + 1) * w * 3) as usize]);
    }
    // zlib with stored blocks
    let mut z = vec![0x78, 0x01];
    for (i, c) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(last as u8);
        z.extend((c.len() as u16).to_le_bytes());
        z.extend((!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend(((b << 16) | a).to_be_bytes());
    let crc = |d: &[u8]| {
        let mut c = 0xFFFF_FFFFu32;
        for &x in d {
            c ^= x as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    };
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |t: &[u8; 4], d: &[u8]| {
        out.extend((d.len() as u32).to_be_bytes());
        let mut td = t.to_vec();
        td.extend_from_slice(d);
        out.extend_from_slice(&td);
        out.extend(crc(&td).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend(w.to_be_bytes());
    ihdr.extend(h.to_be_bytes());
    ihdr.extend([8, 2, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    fs::write(path, out).unwrap();
}
