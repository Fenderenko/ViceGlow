//! Live light: the PC sound (WASAPI loopback of the default output) or the screen colour (a
//! 64×36 GDI copy) drives the lamp. One thread while a mode is on; colours go to the engine as
//! `Op::Stream`, only when they change, at most 20 (sound) or 8 (screen) times a second.

use std::{
    sync::{
        Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant},
};

use windows::Win32::{
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HALFTONE, HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject, SetBrushOrgEx, SetStretchBltMode, StretchBlt,
    },
    Media::Audio::{AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender},
    System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree},
    UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN},
};

use crate::{
    actions, config, engine,
    lamp::{Op, from_hsv, hsv},
    logf,
};

const OFF: u8 = 0;
const AUDIO: u8 = 1;
const SCREEN: u8 = 2;

static MODE: AtomicU8 = AtomicU8::new(OFF);
/// bumped on every start: an older thread sees it and ends
static RUN: AtomicU8 = AtomicU8::new(0);
/// the colour sound pulses with (screen colour of the page)
static BASE: Mutex<[u8; 3]> = Mutex::new([255, 106, 26]);

pub fn active() -> bool {
    MODE.load(Ordering::Relaxed) != OFF
}

pub fn mode() -> &'static str {
    match MODE.load(Ordering::Relaxed) {
        AUDIO => "audio",
        SCREEN => "screen",
        _ => "off",
    }
}

pub fn set_base(c: [u8; 3]) {
    *BASE.lock().unwrap() = c;
}

/// "audio" | "screen" | anything else — off.
pub fn set(m: &str) {
    let want = match m {
        "audio" => AUDIO,
        "screen" => SCREEN,
        _ => OFF,
    };
    if want == MODE.load(Ordering::Relaxed) {
        return;
    }
    MODE.store(want, Ordering::Relaxed);
    let run = RUN.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    if want == OFF {
        engine::refresh();
        return;
    }
    let _ = std::thread::Builder::new().name("live".into()).spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let alive = || RUN.load(Ordering::Relaxed) == run;
        if want == AUDIO {
            while alive() {
                if let Err(e) = audio(&alive) {
                    logf!("live sound: {e}");
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        } else {
            screen(&alive);
        }
    });
}

/// Sends a colour unless it is the same as the last one.
struct Out {
    last: Option<([u8; 3], u8)>,
    at: Instant,
    every: Duration,
}

impl Out {
    fn new(every: Duration) -> Self {
        Self { last: None, at: Instant::now(), every }
    }

    fn put(&mut self, c: [u8; 3], b: u8) {
        if self.at.elapsed() < self.every {
            return;
        }
        let near = self.last.is_some_and(|(lc, lb)| (lb as i32 - b as i32).abs() < 2 && lc.iter().zip(c).all(|(x, y)| (*x as i32 - y as i32).abs() < 4));
        if near {
            return;
        }
        self.last = Some((c, b));
        self.at = Instant::now();
        engine::ops(vec![Op::Stream(actions::led(c), b.clamp(1, 100))]);
    }
}

fn audio(alive: &dyn Fn() -> bool) -> windows::core::Result<()> {
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole)?;
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None)?;
        let fmt = client.GetMixFormat()?;
        let (rate, ch, bits) = ((*fmt).nSamplesPerSec as f32, (*fmt).nChannels as usize, (*fmt).wBitsPerSample);
        let r = client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, 2_000_000, 0, fmt, None);
        CoTaskMemFree(Some(fmt as *const _));
        r?;
        if bits != 32 {
            return Err(windows::core::Error::from_hresult(windows::core::HRESULT(0x80070032u32 as i32)));
        }
        let cap: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        let a = (-2.0 * std::f32::consts::PI * 150.0 / rate).exp();
        let (mut lp, mut env, mut peak, mut hue) = (0f32, 0f32, 0.02f32, 0f32);
        let mut out = Out::new(Duration::from_millis(50));
        let mut last_audio = Instant::now();
        while alive() {
            std::thread::sleep(Duration::from_millis(25));
            let (mut sum_b, mut sum_a, mut n) = (0f32, 0f32, 0usize);
            while cap.GetNextPacketSize()? > 0 {
                let mut data = std::ptr::null_mut();
                let (mut frames, mut flags) = (0u32, 0u32);
                cap.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 == 0 && !data.is_null() {
                    let s = std::slice::from_raw_parts(data as *const f32, frames as usize * ch);
                    for f in s.chunks_exact(ch) {
                        let x = f.iter().sum::<f32>() / ch as f32;
                        lp = a * lp + (1.0 - a) * x;
                        sum_b += lp * lp;
                        sum_a += x * x;
                        n += 1;
                    }
                }
                cap.ReleaseBuffer(frames)?;
            }
            let level = if n > 0 {
                last_audio = Instant::now();
                ((sum_b / n as f32) * 0.75 + (sum_a / n as f32) * 0.25).sqrt()
            } else {
                0.0
            };
            // fast rise, slow fall; the loudest recent level is the top
            env = if level > env { level } else { env * 0.86 };
            peak = (peak * 0.997).max(env).max(0.01);
            let cfg = config::get();
            let gain = 0.5 + cfg.audio_gain as f32 / 100.0;
            let k = (env / peak * gain).clamp(0.0, 1.0);
            if last_audio.elapsed() > Duration::from_secs(3) {
                // nothing plays: a calm glow
                out.put(*BASE.lock().unwrap(), 20);
                continue;
            }
            if cfg.audio_mode == "spectrum" {
                hue = (hue + 0.6 + k * 5.0) % 360.0;
                out.put(from_hsv(hue, 1.0, 1.0), (8.0 + k * 92.0) as u8);
            } else {
                out.put(*BASE.lock().unwrap(), (6.0 + k * 94.0) as u8);
            }
        }
        let _ = client.Stop();
    }
    Ok(())
}

const SW: i32 = 64;
const SH: i32 = 36;

fn screen(alive: &dyn Fn() -> bool) {
    let mut out = Out::new(Duration::from_millis(120));
    let mut avg: Option<[f32; 3]> = None;
    unsafe {
        let src = GetDC(None);
        let mem = CreateCompatibleDC(Some(src));
        let bi = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: SW, biHeight: -SH, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
        let mut bits = std::ptr::null_mut();
        let Ok(bmp) = CreateDIBSection(Some(mem), &bi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(mem);
            ReleaseDC(None, src);
            return;
        };
        let old = SelectObject(mem, HGDIOBJ(bmp.0));
        SetStretchBltMode(mem, HALFTONE);
        let _ = SetBrushOrgEx(mem, 0, 0, None);
        while alive() {
            std::thread::sleep(Duration::from_millis(120));
            let (w, h) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
            if !StretchBlt(mem, 0, 0, SW, SH, Some(src), 0, 0, w, h, SRCCOPY).as_bool() {
                continue;
            }
            let px = std::slice::from_raw_parts(bits as *const u8, (SW * SH * 4) as usize);
            let cfg = config::get();
            let (mut acc, mut wsum, mut lum) = ([0f32; 3], 0f32, 0f32);
            for y in 0..SH {
                for x in 0..SW {
                    if cfg.screen_edges && x > SW / 5 && x < SW * 4 / 5 && y > SH / 5 && y < SH * 4 / 5 {
                        continue;
                    }
                    let i = ((y * SW + x) * 4) as usize;
                    let c = [px[i + 2], px[i + 1], px[i]];
                    let (_, s, v) = hsv(c);
                    // colourful pixels count more than grey ones
                    let wgt = 0.08 + s * v * 2.0;
                    for k in 0..3 {
                        acc[k] += c[k] as f32 * wgt;
                    }
                    wsum += wgt;
                    lum += v;
                }
            }
            if wsum == 0.0 {
                continue;
            }
            let cnt = if cfg.screen_edges { (SW * SH) as f32 * 0.64 } else { (SW * SH) as f32 };
            let now = acc.map(|x| x / wsum);
            let a = avg.get_or_insert(now);
            for k in 0..3 {
                a[k] += (now[k] - a[k]) * 0.45;
            }
            let (hh, s, _) = hsv(a.map(|x| x.round().clamp(0.0, 255.0) as u8));
            let s = (s * (1.0 + cfg.screen_sat as f32 / 100.0)).min(1.0);
            let bright = (lum / cnt * 100.0).clamp(4.0, 100.0);
            out.put(from_hsv(hh, s, 1.0), bright as u8);
        }
        SelectObject(mem, old);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(mem);
        ReleaseDC(None, src);
    }
}
