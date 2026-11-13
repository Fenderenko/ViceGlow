//! The lamp engine: one thread owns the link to the active lamp (Bluetooth or network), the
//! queue of operations and the search for lamps nearby. Bluetooth lamps are let go after IDLE
//! without use: most take a single connection, so their phone app can reach them again.

use std::{
    sync::{
        Mutex, OnceLock,
        mpsc::{self, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant},
};

use serde::Serialize;
use serde_json::{Value, json};
use windows::{
    Devices::Bluetooth::{
        Advertisement::{BluetoothLEAdvertisementReceivedEventArgs, BluetoothLEAdvertisementWatcher, BluetoothLEScanningMode},
        GenericAttributeProfile::GattCharacteristic,
    },
    Foundation::TypedEventHandler,
    Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx},
    core::Ref,
};

use crate::{
    config::{self, Lamp},
    drv::{self, BleProto, Gatt, NetFound, NetProto},
    gatt,
    lamp::{self, Caps, Op, State, hex},
    logf, util,
};

const IDLE: Duration = Duration::from_secs(20);
const RETRY: Duration = Duration::from_secs(5);
/// a connection attempt longer than this counts as failed (Windows may wait much longer for a
/// lamp that is away or held by a phone); a late result is dropped
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// writes settle this long before the state is read back
const SETTLE: Duration = Duration::from_millis(250);
const KEEPALIVE: Duration = Duration::from_secs(2);
const NET_SCAN_EVERY: Duration = Duration::from_secs(20);
/// a found lamp not heard from for this long leaves the list
const FORGET_FOUND: Duration = Duration::from_secs(40);

#[derive(Clone, Copy, PartialEq, Serialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Conn {
    Off,
    Connecting,
    On,
}

/// A relative action from hotkeys and the tray, planned on fresh state.
#[derive(Clone, Copy, Debug)]
pub enum Act {
    Toggle,
    /// brightness step, % (negative — darker)
    Bright(i8),
}

#[derive(Clone, Serialize)]
pub struct Snap {
    pub lamp: String,
    pub conn: Conn,
    pub err: String,
    pub state: Option<State>,
    pub caps: Caps,
    pub fxdyn: Vec<String>,
    pub scanning: bool,
}

#[derive(Clone, Serialize, Debug)]
pub struct Found {
    pub id: String,
    pub family: &'static str,
    pub model: String,
    pub name: String,
    pub net: &'static str,
    pub rssi: Option<i16>,
    #[serde(skip)]
    pub addr: u64,
    #[serde(skip)]
    pub host: String,
    #[serde(skip)]
    pub extra: Value,
    #[serde(skip)]
    pub seen: Option<Instant>,
}

enum Msg {
    Ops(Vec<Op>),
    Act(Act),
    Refresh,
    Awake(bool),
    Select(String),
    Connect(String),
    Disconnect(String),
    Scan(bool),
    Notify(Vec<u8>, u64),
    Net(Vec<NetFound>),
    /// a connection attempt (made on its own thread) finished; stale ones are dropped
    Opened(u64, Result<Opened, String>),
    Quit,
}

/// What a finished connection attempt hands to the engine.
struct Opened {
    link: Link,
    caps: Caps,
    st: State,
    has_state: bool,
    fx: Vec<String>,
}

static TX: OnceLock<Mutex<Sender<Msg>>> = OnceLock::new();
static SNAP: Mutex<Option<Snap>> = Mutex::new(None);
static FOUND: Mutex<Vec<Found>> = Mutex::new(Vec::new());
/// a connection thread is still inside Windows (even after its timeout): no second one meanwhile
static OPENING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn snapshot() -> Snap {
    SNAP.lock().unwrap().clone().unwrap_or(Snap { lamp: String::new(), conn: Conn::Off, err: String::new(), state: None, caps: Caps::default(), fxdyn: Vec::new(), scanning: false })
}

pub fn state() -> Option<State> {
    SNAP.lock().unwrap().as_ref().and_then(|s| s.state.clone())
}

/// Lamps heard recently (not forgotten yet).
pub fn found() -> Vec<Found> {
    let mut f = FOUND.lock().unwrap();
    f.retain(|x| x.seen.is_some_and(|t| t.elapsed() < FORGET_FOUND));
    f.clone()
}

fn changed(f: impl FnOnce(&mut Snap)) {
    {
        let mut g = SNAP.lock().unwrap();
        let s = g.get_or_insert_with(|| Snap { lamp: String::new(), conn: Conn::Off, err: String::new(), state: None, caps: Caps::default(), fxdyn: Vec::new(), scanning: false });
        f(s);
    }
    util::post_ui(crate::WM_STATE, 0);
}

fn send(m: Msg) {
    if let Some(tx) = TX.get() {
        let _ = tx.lock().unwrap().send(m);
    }
}

pub fn ops(v: Vec<Op>) {
    if !v.is_empty() {
        send(Msg::Ops(v));
    }
}

pub fn act(a: Act) {
    send(Msg::Act(a));
}

pub fn refresh() {
    send(Msg::Refresh);
}

/// The window is shown: stay connected.
pub fn set_awake(on: bool) {
    send(Msg::Awake(on));
}

pub fn select(id: &str) {
    send(Msg::Select(id.into()));
}

/// "Підключити" for a saved or a found lamp (pairs network lamps that need it).
pub fn connect(id: &str) {
    send(Msg::Connect(id.into()));
}

pub fn disconnect(id: &str) {
    send(Msg::Disconnect(id.into()));
}

/// Searching for lamps while the devices page is open.
pub fn scan(on: bool) {
    send(Msg::Scan(on));
}

pub fn stop() {
    send(Msg::Quit);
}

pub fn start() {
    let (tx, rx) = mpsc::channel();
    let _ = TX.set(Mutex::new(tx.clone()));
    std::thread::Builder::new()
        .name("engine".into())
        .spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            let mut w = Worker { tx: Some(tx), ..Default::default() };
            w.load_active();
            loop {
                let wait = if w.last_write.is_some() || !w.pending.is_empty() { Duration::from_millis(30) } else { Duration::from_millis(400) };
                match rx.recv_timeout(wait) {
                    Ok(Msg::Quit) | Err(RecvTimeoutError::Disconnected) => break,
                    Ok(m) => w.take(m),
                    Err(RecvTimeoutError::Timeout) => {}
                }
                while let Ok(m) = rx.try_recv() {
                    if matches!(m, Msg::Quit) {
                        w.close("quit");
                        return;
                    }
                    w.take(m);
                }
                w.step();
            }
            w.close("quit");
        })
        .expect("engine thread");
}

enum Link {
    Ble { _dev: gatt::Dev, w0: GattCharacteristic, w1: Option<GattCharacteristic>, ntf: Option<(GattCharacteristic, i64)>, drv: Box<dyn BleProto>, g: Gatt },
    Net { drv: Box<dyn NetProto> },
}

impl Drop for Link {
    fn drop(&mut self) {
        match self {
            Link::Ble { ntf: Some((c, t)), .. } => {
                let _ = c.RemoveValueChanged(*t);
            }
            Link::Net { drv } => drv.close(),
            _ => {}
        }
    }
}

#[derive(Default)]
struct Worker {
    tx: Option<Sender<Msg>>,
    lamp: Option<Lamp>,
    link: Option<Link>,
    st: State,
    has_state: bool,
    caps: Caps,
    gen_: u64,
    awake: bool,
    pending: Vec<Op>,
    acts: Vec<Act>,
    acts_since: Option<Instant>,
    need_query: bool,
    last_use: Option<Instant>,
    last_write: Option<Instant>,
    last_keep: Option<Instant>,
    retry_at: Option<Instant>,
    /// the lamp is in single-colour mode for live colour (sound, screen)
    stream_ok: bool,
    scan: bool,
    watcher: Option<BluetoothLEAdvertisementWatcher>,
    net_scan_at: Option<Instant>,
    net_scanning: bool,
    /// a connection attempt is running (generation gen_), since when
    connecting: bool,
    /// the first state of a new link is still to come (timers are checked on it)
    fresh_link: bool,
    connect_at: Option<Instant>,
}

impl Worker {
    fn load_active(&mut self) {
        self.lamp = config::active();
        let id = self.lamp.as_ref().map(|l| l.id.clone()).unwrap_or_default();
        changed(|s| {
            s.lamp = id;
            s.conn = Conn::Off;
            s.state = None;
            s.err.clear();
            s.fxdyn.clear();
        });
        self.has_state = false;
        self.stream_ok = false;
    }

    fn take(&mut self, m: Msg) {
        match m {
            Msg::Ops(v) => {
                for op in v {
                    push_coalesced(&mut self.pending, op);
                }
                self.last_use = Some(Instant::now());
            }
            Msg::Act(a) => {
                self.acts.push(a);
                self.acts_since.get_or_insert_with(Instant::now);
                self.last_use = Some(Instant::now());
            }
            Msg::Refresh => {
                self.need_query = true;
                self.retry_at = None;
                self.last_use = Some(Instant::now());
            }
            Msg::Awake(a) => {
                self.awake = a;
                if a {
                    self.need_query = true;
                    self.retry_at = None;
                }
                self.last_use = Some(Instant::now());
            }
            Msg::Select(id) => {
                if self.lamp.as_ref().is_some_and(|l| l.id == id) {
                    return;
                }
                self.close("another lamp");
                config::update(|c| c.active = id);
                self.load_active();
                self.pending.clear();
                self.acts.clear();
                self.need_query = true;
                self.retry_at = None;
                self.last_use = Some(Instant::now());
            }
            Msg::Connect(id) => self.connect_req(&id),
            Msg::Disconnect(id) => {
                config::update(|c| {
                    if let Some(l) = c.lamps.iter_mut().find(|l| l.id == id) {
                        l.paused = true;
                    }
                });
                if self.lamp.as_ref().is_some_and(|l| l.id == id) {
                    self.close("disconnected by the user");
                    self.lamp = config::lamp(&id);
                    self.pending.clear();
                    self.acts.clear();
                    changed(|s| {
                        s.conn = Conn::Off;
                        s.err.clear();
                    });
                }
            }
            Msg::Scan(on) => {
                self.scan = on;
                if on {
                    self.net_scan_at = None;
                }
                changed(|s| s.scanning = on);
            }
            Msg::Notify(d, g) => {
                if g != self.gen_ {
                    return;
                }
                if let Some(Link::Ble { drv, .. }) = self.link.as_mut()
                    && drv.notified(&d, &mut self.st)
                {
                    self.has_state = true;
                    if std::mem::take(&mut self.fresh_link) {
                        self.sync_timers();
                    }
                    self.publish();
                }
            }
            Msg::Net(list) => {
                self.net_scanning = false;
                let mut f = FOUND.lock().unwrap();
                for n in list {
                    let id = n.id();
                    let e = Found { id: id.clone(), family: n.family, model: n.model, name: n.name, net: "wifi", rssi: None, addr: 0, host: n.host, extra: n.extra, seen: Some(Instant::now()) };
                    match f.iter_mut().find(|x| x.id == id) {
                        Some(x) => *x = e,
                        None => f.push(e),
                    }
                }
                drop(f);
                util::post_ui(crate::WM_STATE, 0);
            }
            Msg::Opened(g, r) => {
                if g != self.gen_ {
                    return; // cancelled meanwhile (another lamp, "Disconnect"); dropping closes it
                }
                self.connecting = false;
                let live = crate::live::active();
                match r {
                    Ok(o) => {
                        self.link = Some(o.link);
                        self.caps = o.caps.clone();
                        self.st = o.st;
                        self.has_state = o.has_state;
                        self.need_query = true;
                        self.retry_at = None;
                        self.last_keep = Some(Instant::now());
                        self.fresh_link = true;
                        let fx = o.fx;
                        changed(|s| {
                            s.conn = Conn::On;
                            s.err.clear();
                            s.caps = o.caps;
                            s.fxdyn = fx;
                        });
                        self.publish();
                    }
                    Err(e) => {
                        let name = self.lamp.as_ref().map(|l| l.name.clone()).unwrap_or_default();
                        logf!("lamp '{name}': {e}");
                        self.retry_at = Some(Instant::now() + RETRY);
                        if !self.awake && !live {
                            self.pending.clear();
                            self.acts.clear();
                            self.need_query = false;
                        }
                        changed(|s| {
                            s.conn = Conn::Off;
                            s.err = e;
                        });
                    }
                }
            }
            Msg::Quit => {}
        }
    }

    /// Saves a found lamp (or unpauses a saved one), makes it active and connects.
    fn connect_req(&mut self, id: &str) {
        let found = FOUND.lock().unwrap().iter().find(|f| f.id == id).cloned();
        if let Some(f) = &found {
            if f.family == "hue_bridge" {
                self.pair_hue(&f.host);
                return;
            }
            if f.family == "unknown" {
                return;
            }
        }
        if config::lamp(id).is_none() {
            let Some(f) = found else { return };
            let l = Lamp { id: f.id.clone(), family: f.family.into(), model: f.model.clone(), name: if f.name.is_empty() { f.model.clone() } else { f.name.clone() }, addr: f.addr, host: f.host.clone(), extra: f.extra.clone(), ..Default::default() };
            config::update(|c| c.lamps.push(l));
        }
        config::update(|c| {
            if let Some(l) = c.lamps.iter_mut().find(|l| l.id == id) {
                l.paused = false;
            }
        });
        if self.lamp.as_ref().is_some_and(|l| l.id == id) {
            self.lamp = config::lamp(id);
        } else {
            self.close("another lamp");
            config::update(|c| c.active = id.to_string());
            self.load_active();
        }
        self.need_query = true;
        self.retry_at = None;
        self.last_use = Some(Instant::now());
    }

    fn pair_hue(&mut self, host: &str) {
        match drv::net_hue::pair(host) {
            Ok(u) => {
                logf!("hue: paired with {host}");
                config::update(|c| {
                    c.hue_users.insert(host.into(), json!(u));
                });
                FOUND.lock().unwrap().retain(|f| !(f.family == "hue_bridge" && f.host == host));
                self.net_scan_at = None;
                changed(|s| s.err.clear());
            }
            Err(e) => changed(|s| s.err = if e == "pair" { "hue-button".into() } else { e }),
        }
    }

    /// The timers the app knows for the active lamp (kept in its settings).
    fn saved_timers(&self) -> Vec<lamp::Timer> {
        let id = self.lamp.as_ref().map(|l| l.id.clone()).unwrap_or_default();
        config::lamp(&id).and_then(|l| serde_json::from_value(l.extra.get("timers").cloned()?).ok()).unwrap_or_default()
    }

    fn save_timers(&self) {
        let Some(id) = self.lamp.as_ref().map(|l| l.id.clone()) else { return };
        let v = serde_json::to_value(&self.st.timers).unwrap_or(Value::Null);
        config::update(|c| {
            if let Some(l) = c.lamps.iter_mut().find(|l| l.id == id) {
                if !l.extra.is_object() {
                    l.extra = json!({});
                }
                l.extra["timers"] = v;
            }
        });
    }

    /// First state after connecting: a lamp that lost power forgets its timers — the saved ones
    /// go back; otherwise the lamp's own list is kept as the saved one. Writing a timer also sets
    /// the lamp clock, which a power loss resets too.
    fn sync_timers(&mut self) {
        if !self.caps.timers {
            return;
        }
        let saved = self.saved_timers();
        if self.st.timers.is_empty() && !saved.is_empty() {
            logf!("lamp: timers lost (power cut?), restoring {}", saved.len());
            for t in saved {
                push_coalesced(&mut self.pending, Op::TimerSet(t));
            }
        } else if let Some(t) = self.st.timers.first().cloned() {
            if self.st.timers != saved {
                self.save_timers();
            }
            push_coalesced(&mut self.pending, Op::TimerSet(t));
        }
    }

    fn publish(&self) {
        let st = self.st.clone();
        let known = self.has_state || !self.caps.state;
        changed(|s| s.state = known.then_some(st));
    }

    fn close(&mut self, why: &str) {
        if self.link.take().is_some() {
            logf!("lamp: disconnected ({why})");
        } else if self.connecting {
            logf!("lamp: connection attempt dropped ({why})");
        }
        // a new generation also cancels a connection attempt still running
        self.gen_ += 1;
        self.connecting = false;
        self.stream_ok = false;
    }

    fn step(&mut self) {
        self.scan_step();
        let live = crate::live::active();
        let Some(lamp) = self.lamp.clone() else {
            self.pending.clear();
            self.acts.clear();
            return;
        };
        if lamp.paused {
            self.pending.clear();
            self.acts.clear();
            return;
        }
        let want = self.awake || live || !self.pending.is_empty() || !self.acts.is_empty() || self.need_query;
        if want && self.link.is_none() && !self.connecting && !OPENING.load(std::sync::atomic::Ordering::Relaxed) && self.retry_at.is_none_or(|t| Instant::now() >= t) {
            changed(|s| {
                s.conn = Conn::Connecting;
                s.err.clear();
            });
            // the attempt may take many seconds (lamp away or held by the phone): it runs on its own
            // thread, so "Disconnect", another lamp and the window stay responsive
            self.gen_ += 1;
            self.connecting = true;
            self.connect_at = Some(Instant::now());
            logf!("lamp: connecting '{}'", lamp.name);
            if lamp.is_ble()
                && let Some(w) = self.watcher.take()
            {
                let _ = w.Stop(); // active scanning slows the connection down; it resumes afterwards
            }
            let (g, tx, lamp) = (self.gen_, self.tx.clone().unwrap(), lamp.clone());
            OPENING.store(true, std::sync::atomic::Ordering::Relaxed);
            std::thread::spawn(move || {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                }
                let r = open_link(&lamp, tx.clone(), g);
                OPENING.store(false, std::sync::atomic::Ordering::Relaxed);
                let _ = tx.send(Msg::Opened(g, r));
            });
        }
        if self.connecting && self.connect_at.is_some_and(|t| t.elapsed() > CONNECT_TIMEOUT) {
            logf!("lamp '{}': no answer in {} s", lamp.name, CONNECT_TIMEOUT.as_secs());
            self.gen_ += 1;
            self.connecting = false;
            self.retry_at = Some(Instant::now() + RETRY);
            if !self.awake && !live {
                self.pending.clear();
                self.acts.clear();
                self.need_query = false;
            }
            changed(|s| {
                s.conn = Conn::Off;
                s.err = "lamp does not answer".into();
            });
        }
        if self.link.is_none() {
            return;
        }
        let mut ok = true;
        // relative actions wait for a fresh state (a timer may have switched the lamp)
        if !self.acts.is_empty() {
            let fresh = self.has_state || !self.caps.state || self.acts_since.is_some_and(|t| t.elapsed() > Duration::from_millis(1500));
            if fresh {
                for a in std::mem::take(&mut self.acts) {
                    for op in plan(a, &self.st) {
                        push_coalesced(&mut self.pending, op);
                    }
                }
                self.acts_since = None;
            } else {
                self.need_query = true;
            }
        }
        let ops = std::mem::take(&mut self.pending);
        let mut wrote = false;
        for op in ops {
            if !ok {
                break;
            }
            let mut seq = Vec::new();
            if matches!(op, Op::Color(_) | Op::Effect(_) | Op::Stream(..)) && !self.st.power {
                seq.push(Op::Power(true));
            }
            match &op {
                Op::Stream(c, _) if !self.stream_ok => {
                    seq.push(Op::Color(*c));
                    self.stream_ok = true;
                }
                Op::Color(_) => self.stream_ok = true,
                Op::Effect(_) | Op::Cycle(true) => self.stream_ok = false,
                _ => {}
            }
            let stream = matches!(op, Op::Stream(..));
            seq.push(op);
            for o in seq {
                if let Err(e) = self.write_op(&o) {
                    logf!("lamp: {o:?} failed: {e}");
                    ok = false;
                    break;
                }
                lamp::apply(&mut self.st, &o);
                if matches!(o, Op::TimerSet(_) | Op::TimerDel(_)) {
                    self.save_timers();
                }
                if let Op::Stream(c, b) = &o {
                    self.st.rgb = *c;
                    self.st.bright = *b;
                }
                if !stream {
                    wrote = true;
                }
            }
            self.publish();
        }
        if wrote {
            self.last_write = Some(Instant::now());
        }
        let settled = self.last_write.is_some_and(|t| t.elapsed() >= SETTLE);
        if ok && (settled || self.need_query && self.last_write.is_none()) && !live {
            self.last_write = None;
            self.need_query = false;
            if let Err(e) = self.query() {
                logf!("lamp: query failed: {e}");
                ok = false;
            }
        } else if live && settled {
            self.last_write = None;
            self.need_query = false;
        }
        if ok
            && self.last_keep.is_some_and(|t| t.elapsed() >= KEEPALIVE)
            && let Some(Link::Ble { drv, w0, g, .. }) = self.link.as_ref()
        {
            if let Some(f) = drv.keepalive() {
                ok = gatt::write(w0, &f.1, g.response).is_ok();
            }
            self.last_keep = Some(Instant::now());
        }
        if !ok {
            self.close("connection lost");
            changed(|s| s.conn = Conn::Off);
            self.retry_at = Some(Instant::now() + Duration::from_secs(1));
            return;
        }
        if !self.awake && !live && self.pending.is_empty() && self.last_write.is_none() && self.last_use.is_none_or(|t| t.elapsed() > IDLE) {
            self.close("idle");
            changed(|s| s.conn = Conn::Off);
        }
    }

    fn write_op(&mut self, op: &Op) -> Result<(), String> {
        match self.link.as_mut().ok_or("no link")? {
            Link::Ble { drv, w0, w1, g, .. } => {
                let frames = drv.frames(op, &self.st);
                for (i, (ch, data)) in frames.iter().enumerate() {
                    if data.is_empty() {
                        continue;
                    }
                    let c = if *ch == 1 { w1.as_ref().unwrap_or(w0) } else { &*w0 };
                    gatt::write(c, data, g.response).map_err(|e| format!("write {}: {e}", hex(data)))?;
                    if g.gap > 0 && i + 1 < frames.len() {
                        std::thread::sleep(Duration::from_millis(g.gap));
                    }
                }
                Ok(())
            }
            Link::Net { drv } => drv.apply(op, &self.st),
        }
    }

    fn query(&mut self) -> Result<(), String> {
        match self.link.as_mut().ok_or("no link")? {
            Link::Ble { drv, w0, g, .. } => {
                if let Some(f) = drv.query() {
                    gatt::write(w0, &f.1, g.response)?;
                }
                Ok(())
            }
            Link::Net { drv } => {
                if drv.refresh(&mut self.st)? {
                    self.has_state = true;
                }
                self.publish();
                Ok(())
            }
        }
    }

    fn scan_step(&mut self) {
        if self.scan && self.watcher.is_none() && !self.connecting {
            self.watcher = start_watcher();
        } else if !self.scan
            && let Some(w) = self.watcher.take()
        {
            let _ = w.Stop();
        }
        if self.scan && !self.net_scanning && self.net_scan_at.is_none_or(|t| t.elapsed() >= NET_SCAN_EVERY) {
            self.net_scanning = true;
            self.net_scan_at = Some(Instant::now());
            let tx = self.tx.clone().unwrap();
            let users = config::get().hue_users;
            std::thread::spawn(move || {
                let v = drv::discover_net(&users);
                let _ = tx.send(Msg::Net(v));
            });
        }
    }
}

/// Connects to `lamp` (blocking; run on its own thread). Notifications are tagged with `generation`.
fn open_link(lamp: &Lamp, tx: Sender<Msg>, generation: u64) -> Result<Opened, String> {
    let mut st = State { bright: 100, speed: 50, length: 60, ..Default::default() };
    let mut has_state = false;
    if lamp.is_ble() {
        let t = Instant::now();
        let dev = gatt::open(lamp.addr)?;
        let name = if dev.name.is_empty() { lamp.name.clone() } else { dev.name.clone() };
        let mut picked = None;
        for fam in drv::candidates(&lamp.family, &name) {
            let Some(d) = drv::make_ble(fam, &lamp.model, &name) else { continue };
            let g = d.gatt();
            if let Some(w0) = dev.find(&g.write) {
                picked = Some((fam, d, g, w0));
                break;
            }
        }
        let Some((fam, mut d, g, w0)) = picked else { return Err("unknown protocol: no known characteristic".into()) };
        if fam != lamp.family {
            logf!("lamp '{}': {} protocol", lamp.name, fam);
            config::update(|c| {
                if let Some(l) = c.lamps.iter_mut().find(|l| l.id == lamp.id) {
                    l.family = fam.into();
                }
            });
        }
        let w1 = g.write2.and_then(|u| dev.find(&[u]));
        let mut ntf = None;
        if let Some(c) = dev.find(&g.notify) {
            match gatt::subscribe(&c, tx, |v, g| Msg::Notify(v, g), generation) {
                Ok(t) => ntf = Some((c, t)),
                Err(e) => logf!("lamp: {e}"),
            }
        }
        let hello = d.hello();
        for (i, (_, f)) in hello.iter().enumerate() {
            gatt::write(&w0, f, g.response)?;
            if i + 1 < hello.len() {
                std::thread::sleep(Duration::from_millis(300));
            }
        }
        let caps = d.caps();
        logf!("lamp: connected '{name}' {:012x} ({fam}) in {} ms", lamp.addr, t.elapsed().as_millis());
        return Ok(Opened { link: Link::Ble { _dev: dev, w0, w1, ntf, drv: d, g }, caps, st, has_state, fx: Vec::new() });
    }
    let mut extra = lamp.extra.clone();
    if lamp.family == "nanoleaf" && extra["token"].as_str().unwrap_or("").is_empty() {
        match drv::net_nanoleaf::pair(&lamp.host) {
            Ok(t) => {
                extra = json!({ "token": t });
                let e2 = extra.clone();
                config::update(|c| {
                    if let Some(l) = c.lamps.iter_mut().find(|l| l.id == lamp.id) {
                        l.extra = e2;
                    }
                });
            }
            Err(e) => return Err(if e == "pair" { "nanoleaf-button".into() } else { e }),
        }
    }
    let mut d = drv::make_net(&lamp.family, &lamp.host, &extra).ok_or("unknown protocol")?;
    let caps = d.caps();
    if caps.state && d.refresh(&mut st)? {
        has_state = true;
    }
    let fx = if caps.fx == "dyn" { d.effects() } else { Vec::new() };
    logf!("lamp: connected '{}' {} ({})", lamp.name, lamp.host, lamp.family);
    Ok(Opened { link: Link::Net { drv: d }, caps, st, has_state, fx })
}

/// Watches Bluetooth adverts and keeps FOUND up to date (lamps only).
fn start_watcher() -> Option<BluetoothLEAdvertisementWatcher> {
    let w = BluetoothLEAdvertisementWatcher::new().ok()?;
    let _ = w.SetScanningMode(BluetoothLEScanningMode::Active);
    w.Received(&TypedEventHandler::new(move |_, a: Ref<BluetoothLEAdvertisementReceivedEventArgs>| {
        let a = a.ok()?;
        let adv = a.Advertisement()?;
        let name = adv.LocalName()?.to_string();
        let mut mfr = Vec::new();
        for m in adv.ManufacturerData()? {
            if let (Ok(c), Ok(d)) = (m.CompanyId(), m.Data().and_then(|b| gatt::buf_bytes(&b))) {
                mfr.push((c, d));
            }
        }
        let (addr, rssi) = (a.BluetoothAddress()?, a.RawSignalStrengthInDBm()?);
        let id = format!("ble:{addr:012x}");
        let mut f = FOUND.lock().unwrap();
        if let Some(x) = f.iter_mut().find(|x| x.id == id) {
            x.rssi = Some(rssi);
            x.seen = Some(Instant::now());
            if !name.is_empty() && x.name.is_empty() {
                x.name = name;
            }
            return Ok(());
        }
        let Some(i) = drv::identify(&name, &mfr) else { return Ok(()) };
        f.push(Found { id, family: i.family, model: i.model, name, net: "ble", rssi: Some(rssi), addr, host: String::new(), extra: Value::Null, seen: Some(Instant::now()) });
        drop(f);
        util::post_ui(crate::WM_STATE, 0);
        Ok(())
    }))
    .ok()?;
    w.Start().ok()?;
    Some(w)
}

/// A newer op of the same kind replaces a waiting one (slider drags, live colour); timers are
/// kept apart by their number.
fn push_coalesced(q: &mut Vec<Op>, op: Op) {
    if let Some(i) = q.iter().rposition(|x| x.kind() == op.kind())
        && i == q.len() - 1
    {
        q[i] = op;
        return;
    }
    q.push(op);
}

/// Ops for a relative action on the current state.
pub fn plan(a: Act, s: &State) -> Vec<Op> {
    match a {
        Act::Toggle => vec![Op::Power(!s.power)],
        Act::Bright(step) => {
            let p = (s.bright as i32 + step as i32).clamp(1, 100) as u8;
            let mut v = Vec::new();
            if !s.power && step > 0 {
                v.push(Op::Power(true));
            }
            v.push(Op::Bright(p));
            v
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bright_step_turns_on_and_clamps() {
        let s = State { power: false, bright: 100, ..Default::default() };
        assert_eq!(plan(Act::Bright(10), &s), vec![Op::Power(true), Op::Bright(100)]);
        let s = State { power: true, bright: 5, ..Default::default() };
        assert_eq!(plan(Act::Bright(-10), &s), vec![Op::Bright(1)]);
    }

    #[test]
    fn coalescing_keeps_order() {
        let mut q = Vec::new();
        push_coalesced(&mut q, Op::Bright(10));
        push_coalesced(&mut q, Op::Bright(20));
        push_coalesced(&mut q, Op::Color([1, 2, 3]));
        push_coalesced(&mut q, Op::Bright(30));
        assert_eq!(q, vec![Op::Bright(20), Op::Color([1, 2, 3]), Op::Bright(30)]);
    }
}
