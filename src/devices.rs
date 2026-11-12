//! Bluetooth device manager. Runs a tokio runtime on its own thread, talks
//! to BlueZ through btleplug and exchanges messages with the UI via channels.

use crate::protocol::{self, ftms, wahoo, CadenceCalc};
use btleplug::api::{
    Central, CentralEvent, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::{Stream, StreamExt};
use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::watch;

pub const SIM_ID: &str = "simulated-trainer";

static RAW_LOG: AtomicBool = AtomicBool::new(false);
/// While a spindown runs, normal trainer commands are held back.
static CALIBRATING: AtomicBool = AtomicBool::new(false);

/// Progress of a trainer spindown (calibration).
#[derive(Debug, Clone, PartialEq)]
pub enum Calibration {
    Started,
    /// Target speed window in km/h.
    Targets(f32, f32),
    SpeedUp,
    StopPedaling,
    Success,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    SmartTrainer,
    LegacyWahoo,
    Power,
    HeartRate,
    Cadence,
}

impl Capability {
    pub fn label(self) -> &'static str {
        match self {
            Capability::SmartTrainer => "Smart trainer (FTMS)",
            Capability::LegacyWahoo => "Wahoo trainer (legacy)",
            Capability::Power => "Power",
            Capability::HeartRate => "Heart rate",
            Capability::Cadence => "Cadence",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub rssi: Option<i16>,
    pub caps: Vec<Capability>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Reading {
    pub power: Option<f32>,
    pub cadence: Option<f32>,
    pub speed_kmh: Option<f32>,
    pub hr: Option<f32>,
    /// Heart rate strap skin contact, if reported.
    pub hr_contact: Option<bool>,
    /// Heart rate computed from beat-to-beat intervals.
    pub hr_rr: Option<f32>,
    /// The strap's beat timing is erratic: it is measuring noise.
    pub hr_noisy: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Control {
    Erg(u16),
    /// Grade in percent.
    Grade(f32),
}

#[derive(Debug, Clone)]
pub enum Cmd {
    StartScan,
    StopScan,
    Connect(String),
    Disconnect(String),
    /// Device ids to connect to automatically whenever they are seen.
    AutoConnect(Vec<String>),
    Control(Control),
    /// Rider mass + bike mass, used for legacy Wahoo simulation mode.
    SetMass(f32),
    SimEnable(bool),
    SimEffort(f32),
    /// Log every sensor packet as hex (for diagnosing odd readings).
    RawLog(bool),
    /// Start (true) or cancel (false) a trainer spindown.
    Calibrate(bool),
}

#[derive(Debug, Clone)]
pub enum Event {
    Adapter(Result<String, String>),
    Scanning(bool),
    Devices(Vec<DeviceInfo>),
    Link {
        id: String,
        name: String,
        link: Link,
        caps: Vec<Capability>,
        controllable: bool,
    },
    Data {
        id: String,
        reading: Reading,
    },
    Calibration(Calibration),
    Log(String),
}

#[derive(Clone)]
struct Ui {
    tx: std::sync::mpsc::Sender<Event>,
    ctx: eframe::egui::Context,
}

impl Ui {
    fn send(&self, e: Event) {
        let _ = self.tx.send(e);
        self.ctx.request_repaint();
    }
    fn log(&self, s: impl Into<String>) {
        self.send(Event::Log(s.into()));
    }
}

pub struct DeviceHandle {
    pub tx: UnboundedSender<Cmd>,
    pub rx: std::sync::mpsc::Receiver<Event>,
}

impl DeviceHandle {
    pub fn send(&self, c: Cmd) {
        let _ = self.tx.send(c);
    }
}

pub fn spawn(ctx: eframe::egui::Context) -> DeviceHandle {
    let (cmd_tx, cmd_rx) = unbounded_channel();
    let (ev_tx, ev_rx) = std::sync::mpsc::channel();
    let ui = Ui { tx: ev_tx, ctx };
    std::thread::Builder::new()
        .name("bluetooth".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(run(cmd_rx, ui));
        })
        .expect("spawn bluetooth thread");
    DeviceHandle {
        tx: cmd_tx,
        rx: ev_rx,
    }
}

#[derive(Clone)]
enum ControlKind {
    Ftms(Characteristic),
    Wahoo {
        ch: Characteristic,
        sim_mode: bool,
        last_write: Instant,
    },
}

/// Wahoo's legacy protocol rejects commands that arrive while the previous
/// one is still being processed, and occasionally needs unlocking again.
const WAHOO_GAP: Duration = Duration::from_millis(250);

async fn wahoo_write(
    p: &Peripheral,
    ch: &Characteristic,
    data: &[u8],
    last_write: &mut Instant,
) -> btleplug::Result<()> {
    let mut result = Ok(());
    for attempt in 0..3 {
        let since = last_write.elapsed();
        if since < WAHOO_GAP {
            tokio::time::sleep(WAHOO_GAP - since).await;
        }
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let _ = p.write(ch, &wahoo::unlock(), WriteType::WithResponse).await;
            tokio::time::sleep(WAHOO_GAP).await;
        }
        result = p.write(ch, data, WriteType::WithResponse).await;
        *last_write = Instant::now();
        if result.is_ok() {
            break;
        }
    }
    result
}

#[derive(Clone)]
struct Controller {
    id: String,
    peripheral: Peripheral,
    kind: ControlKind,
}

enum Internal {
    Connected {
        id: String,
        peripheral: Peripheral,
        controller: Option<Controller>,
    },
    Failed {
        id: String,
        err: String,
        attempt: u32,
    },
}

type EventStream = Pin<Box<dyn Stream<Item = CentralEvent> + Send>>;

struct Sim {
    on: bool,
    power: f32,
    hr: f32,
    effort: f32,
    rng: u64,
    t: f32,
    /// Simulated spindown: (stage, seconds in stage, wheel speed km/h).
    spindown: Option<(ftms::SpinDownStatus, f32, f32)>,
}

impl Sim {
    fn noise(&mut self) -> f32 {
        // xorshift64*, mapped to [-1, 1]
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let v = self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40;
        v as f32 / (1u64 << 23) as f32 - 1.0
    }

    fn step(&mut self, dt: f32, control: Option<Control>) -> Reading {
        self.t += dt;
        let target = match control {
            Some(Control::Erg(w)) => w as f32,
            Some(Control::Grade(g)) => self.effort * (1.0 + g * 0.04).clamp(0.4, 1.8),
            None => self.effort,
        };
        self.power += (target - self.power) * (dt * 1.5).min(1.0);
        let power = (self.power + self.noise() * 6.0).max(0.0);
        let cadence = (88.0 + self.noise() * 2.0 + (self.t * 0.05).sin() * 3.0).max(0.0);
        let hr_target = 62.0 + power * 0.42;
        self.hr += (hr_target - self.hr) * (dt * 0.04).min(1.0);
        Reading {
            power: Some(power),
            cadence: Some(cadence),
            hr: Some(self.hr),
            ..Default::default()
        }
    }

    /// Advance a simulated spindown; returns a reading and any progress event.
    fn spindown_step(&mut self, dt: f32) -> Option<(Reading, Option<Calibration>)> {
        use ftms::SpinDownStatus::*;
        let (stage, t, speed) = self.spindown.as_mut()?;
        *t += dt;
        let mut event = None;
        match *stage {
            SpeedUp => {
                *speed += 4.0 * dt;
                if *speed >= 38.5 {
                    *stage = StopPedaling;
                    *t = 0.0;
                    event = Some(Calibration::StopPedaling);
                }
            }
            StopPedaling => {
                *speed = (*speed - 6.0 * dt).max(0.0);
                if *t > 5.0 {
                    self.spindown = None;
                    CALIBRATING.store(false, Ordering::SeqCst);
                    return Some((Reading::default(), Some(Calibration::Success)));
                }
            }
            _ => {}
        }
        let pedalling = *stage == SpeedUp;
        let r = Reading {
            power: Some(if pedalling { 150.0 } else { 0.0 }),
            cadence: Some(if pedalling { 95.0 } else { 0.0 }),
            speed_kmh: Some(*speed),
            hr: Some(self.hr),
            ..Default::default()
        };
        Some((r, event))
    }
}

struct State {
    adapter: Option<Adapter>,
    ui: Ui,
    int_tx: UnboundedSender<Internal>,
    peripherals: HashMap<String, Peripheral>,
    names: HashMap<String, String>,
    /// Devices the user wants connected (reconnect on drop).
    wanted: HashSet<String>,
    connecting: HashSet<String>,
    connected: HashSet<String>,
    connected_at: HashMap<String, Instant>,
    /// Bumped on every (re)connect so stale notification tasks exit.
    generations: HashMap<String, Arc<AtomicU32>>,
    auto: HashSet<String>,
    controller: Option<Controller>,
    last_control: Option<Control>,
    /// Latest wanted trainer command; the writer task always sends only the
    /// newest value, so a slow or dead trainer never blocks this loop.
    control_tx: watch::Sender<Option<Control>>,
    /// Bumped whenever the controlling trainer changes so old writers exit.
    writer_gen: Arc<AtomicU32>,
    mass: f32,
    scanning_until: Option<Instant>,
    last_auto_scan: Instant,
    sim: Sim,
}

async fn init_adapter() -> anyhow::Result<Adapter> {
    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    adapters
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no Bluetooth adapter found"))
}

async fn next_central(s: &mut Option<EventStream>) -> Option<CentralEvent> {
    match s {
        Some(s) => s.next().await,
        None => futures::future::pending().await,
    }
}

async fn run(mut cmd_rx: UnboundedReceiver<Cmd>, ui: Ui) {
    let adapter = match init_adapter().await {
        Ok(a) => {
            let info = a
                .adapter_info()
                .await
                .unwrap_or_else(|_| "Bluetooth adapter".into());
            ui.send(Event::Adapter(Ok(info)));
            Some(a)
        }
        Err(e) => {
            ui.send(Event::Adapter(Err(format!(
                "{e}. {}",
                crate::platform::BLUETOOTH_HINT
            ))));
            None
        }
    };
    let mut central = match &adapter {
        Some(a) => a.events().await.ok(),
        None => None,
    };
    let (int_tx, mut int_rx) = unbounded_channel();
    let mut st = State {
        adapter,
        ui,
        int_tx,
        peripherals: HashMap::new(),
        names: HashMap::new(),
        wanted: HashSet::new(),
        connecting: HashSet::new(),
        connected: HashSet::new(),
        connected_at: HashMap::new(),
        generations: HashMap::new(),
        auto: HashSet::new(),
        controller: None,
        last_control: None,
        control_tx: watch::channel(None).0,
        writer_gen: Arc::new(AtomicU32::new(0)),
        mass: 85.0,
        scanning_until: None,
        last_auto_scan: Instant::now(),
        sim: Sim {
            on: false,
            power: 0.0,
            hr: 70.0,
            effort: 180.0,
            rng: 0x9E37_79B9_7F4A_7C15,
            t: 0.0,
            spindown: None,
        },
    };
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let mut last_list = Instant::now();
    let mut last_sim = Instant::now();
    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => match cmd {
                Some(c) => st.handle(c).await,
                None => break,
            },
            ev = next_central(&mut central) => match ev {
                Some(e) => st.central_event(e).await,
                None => central = None,
            },
            Some(m) = int_rx.recv() => st.internal(m).await,
            _ = tick.tick() => {
                if st.sim.on {
                    let dt = last_sim.elapsed().as_secs_f32();
                    last_sim = Instant::now();
                    let reading = match st.sim.spindown_step(dt) {
                        Some((r, event)) => {
                            if let Some(e) = event {
                                st.ui.send(Event::Calibration(e));
                            }
                            r
                        }
                        None => st.sim.step(dt, st.last_control),
                    };
                    st.ui.send(Event::Data { id: SIM_ID.into(), reading });
                }
                if let Some(until) = st.scanning_until {
                    if Instant::now() > until {
                        st.stop_scan().await;
                    } else if last_list.elapsed() > Duration::from_millis(900) {
                        last_list = Instant::now();
                        st.publish_devices().await;
                    }
                } else if st.should_search_remembered() {
                    // A remembered trainer that was asleep at startup still
                    // gets connected once it wakes up.
                    st.last_auto_scan = Instant::now();
                    st.start_scan().await;
                }
            }
        }
    }
    if let Some(c) = &st.controller {
        let _ = c.peripheral.disconnect().await;
    }
}

fn caps_from_services(services: &[uuid::Uuid]) -> Vec<Capability> {
    let mut caps = Vec::new();
    if services.contains(&protocol::FTMS_SERVICE) {
        caps.push(Capability::SmartTrainer);
    }
    if services.contains(&protocol::CPS_SERVICE) {
        caps.push(Capability::Power);
    }
    if services.contains(&protocol::HR_SERVICE) {
        caps.push(Capability::HeartRate);
    }
    if services.contains(&protocol::CSC_SERVICE) {
        caps.push(Capability::Cadence);
    }
    caps
}

impl State {
    async fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::StartScan => self.start_scan().await,
            Cmd::StopScan => self.stop_scan().await,
            Cmd::Connect(id) => {
                if id == SIM_ID {
                    self.enable_sim(true);
                } else {
                    self.stop_scan().await;
                    self.wanted.insert(id.clone());
                    self.connect(id, 0);
                }
            }
            Cmd::Disconnect(id) => {
                if id == SIM_ID {
                    self.enable_sim(false);
                    return;
                }
                self.wanted.remove(&id);
                self.auto.remove(&id);
                if let Some(p) = self.peripherals.get(&id) {
                    let _ = p.disconnect().await;
                }
                self.on_disconnected(&id);
            }
            Cmd::AutoConnect(ids) => {
                self.auto = ids.into_iter().collect();
                if !self.auto.is_empty() && self.adapter.is_some() {
                    self.start_scan().await;
                }
            }
            Cmd::Control(c) => {
                self.last_control = Some(c);
                self.control_tx.send_replace(Some(c));
            }
            Cmd::SetMass(m) => self.mass = m,
            Cmd::SimEnable(on) => self.enable_sim(on),
            Cmd::SimEffort(w) => self.sim.effort = w,
            Cmd::RawLog(on) => RAW_LOG.store(on, Ordering::Relaxed),
            Cmd::Calibrate(start) => self.calibrate(start),
        }
    }

    fn calibrate(&mut self, start: bool) {
        let fail = |ui: &Ui, msg: &str| {
            CALIBRATING.store(false, Ordering::SeqCst);
            ui.send(Event::Calibration(Calibration::Failed(msg.into())));
        };
        if !start {
            if CALIBRATING.swap(false, Ordering::SeqCst) {
                self.sim.spindown = None;
                if let Some(Controller {
                    peripheral,
                    kind: ControlKind::Ftms(cp),
                    ..
                }) = self.controller.clone()
                {
                    tokio::spawn(async move {
                        let _ = peripheral
                            .write(&cp, &ftms::spin_down(false), WriteType::WithResponse)
                            .await;
                    });
                }
            }
            self.ui.send(Event::Calibration(Calibration::Cancelled));
            return;
        }
        match self.controller.clone() {
            Some(Controller { peripheral, kind: ControlKind::Ftms(cp), .. }) => {
                CALIBRATING.store(true, Ordering::SeqCst);
                self.ui.send(Event::Calibration(Calibration::Started));
                let ui = self.ui.clone();
                tokio::spawn(async move {
                    // Let any in-flight trainer command finish first.
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    let cmd = ftms::spin_down(true);
                    let write = peripheral.write(&cp, &cmd, WriteType::WithResponse);
                    match tokio::time::timeout(Duration::from_secs(5), write).await {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => fail(&ui, &format!("The trainer did not accept the request: {}", friendly_error(&e.to_string()))),
                        Err(_) => fail(&ui, "The trainer did not respond"),
                    }
                });
            }
            Some(_) => fail(
                &self.ui,
                "This trainer is using Wahoo's older protocol, which VeloForge cannot calibrate. Run the spindown in the Wahoo app, or update the trainer firmware.",
            ),
            None if self.sim.on => {
                CALIBRATING.store(true, Ordering::SeqCst);
                self.sim.spindown = Some((ftms::SpinDownStatus::SpeedUp, 0.0, 15.0));
                self.ui.send(Event::Calibration(Calibration::Started));
                self.ui.send(Event::Calibration(Calibration::Targets(32.0, 38.0)));
                self.ui.send(Event::Calibration(Calibration::SpeedUp));
            }
            None => fail(&self.ui, "No smart trainer is connected"),
        }
    }

    /// Remembered devices are missing and nothing is connected (scanning while
    /// connected destabilises some adapters, so only search when idle).
    fn should_search_remembered(&self) -> bool {
        self.adapter.is_some()
            && !self.auto.is_empty()
            && self.connected.is_empty()
            && self.connecting.is_empty()
            && self.last_auto_scan.elapsed() > Duration::from_secs(45)
    }

    fn enable_sim(&mut self, on: bool) {
        self.sim.on = on;
        let link = if on {
            Link::Connected
        } else {
            Link::Disconnected
        };
        self.ui.send(Event::Link {
            id: SIM_ID.into(),
            name: "Simulated trainer".into(),
            link,
            caps: vec![
                Capability::SmartTrainer,
                Capability::HeartRate,
                Capability::Cadence,
            ],
            controllable: on,
        });
    }

    async fn start_scan(&mut self) {
        let Some(a) = &self.adapter else {
            self.ui.log("Cannot scan: no Bluetooth adapter");
            return;
        };
        match a.start_scan(ScanFilter::default()).await {
            Ok(()) => {
                self.scanning_until = Some(Instant::now() + Duration::from_secs(30));
                self.ui.send(Event::Scanning(true));
            }
            Err(e) => self.ui.log(format!("Scan failed: {e}")),
        }
    }

    async fn stop_scan(&mut self) {
        self.scanning_until = None;
        if let Some(a) = &self.adapter {
            let _ = a.stop_scan().await;
        }
        self.ui.send(Event::Scanning(false));
    }

    async fn publish_devices(&mut self) {
        let Some(a) = &self.adapter else { return };
        let Ok(list) = a.peripherals().await else {
            return;
        };
        let mut out = Vec::new();
        for p in list {
            let Ok(Some(props)) = p.properties().await else {
                continue;
            };
            let Some(name) = props
                .local_name
                .clone()
                .or(props.advertisement_name.clone())
            else {
                continue;
            };
            let id = p.id().to_string();
            let caps = caps_from_services(&props.services);
            self.names.insert(id.clone(), name.clone());
            self.peripherals.insert(id.clone(), p);
            out.push(DeviceInfo {
                id,
                name,
                rssi: props.rssi,
                caps,
            });
        }
        // Known fitness devices first, then by signal strength.
        out.sort_by_key(|d| (d.caps.is_empty(), -(d.rssi.unwrap_or(-127) as i32)));
        let auto: Vec<String> = out
            .iter()
            .filter(|d| self.auto.contains(&d.id))
            .map(|d| d.id.clone())
            .collect();
        let all_found = auto.len() == self.auto.len();
        let mut started = false;
        for id in auto {
            if !self.connected.contains(&id) && !self.connecting.contains(&id) {
                self.ui
                    .log(format!("Auto-connecting to {}", self.name_of(&id)));
                self.wanted.insert(id.clone());
                started = true;
                self.connect(id, 0);
            }
        }
        if started && all_found {
            self.stop_scan().await;
        }
        self.ui.send(Event::Devices(out));
    }

    fn name_of(&self, id: &str) -> String {
        self.names
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.to_string())
    }

    fn connect(&mut self, id: String, attempt: u32) {
        let Some(cached) = self.peripherals.get(&id).cloned() else {
            self.ui.log(format!("Unknown device {id}; scan first"));
            return;
        };
        if !self.connecting.insert(id.clone()) {
            return;
        }
        let name = self.name_of(&id);
        self.ui.send(Event::Link {
            id: id.clone(),
            name: name.clone(),
            link: if attempt == 0 {
                Link::Connecting
            } else {
                Link::Reconnecting
            },
            caps: vec![],
            controllable: false,
        });
        let ui = self.ui.clone();
        let int_tx = self.int_tx.clone();
        let adapter = self.adapter.clone();
        let mass = self.mass;
        let gen_cell = self.generations.entry(id.clone()).or_default().clone();
        let generation = gen_cell.fetch_add(1, Ordering::SeqCst) + 1;
        tokio::spawn(async move {
            if attempt > 0 {
                let delay = [2, 3, 5, 8, 10, 15][(attempt as usize - 1).min(5)];
                tokio::time::sleep(Duration::from_secs(delay)).await;
            }
            let result = async {
                // BlueZ deletes unpaired devices shortly after they disconnect,
                // so always look the device up again instead of reusing a
                // possibly dead object.
                let p = match &adapter {
                    Some(a) if attempt > 0 => find_live(a, &id).await?,
                    _ => cached,
                };
                let controller = setup(
                    p.clone(),
                    id.clone(),
                    name,
                    mass,
                    ui,
                    (gen_cell, generation),
                )
                .await?;
                anyhow::Ok((p, controller))
            }
            .await;
            let msg = match result {
                Ok((peripheral, controller)) => Internal::Connected {
                    id,
                    peripheral,
                    controller,
                },
                Err(e) => Internal::Failed {
                    id,
                    err: e.to_string(),
                    attempt,
                },
            };
            let _ = int_tx.send(msg);
        });
    }

    async fn internal(&mut self, m: Internal) {
        match m {
            Internal::Connected {
                id,
                peripheral,
                controller,
            } => {
                self.connecting.remove(&id);
                self.peripherals.insert(id.clone(), peripheral);
                self.connected_at.insert(id.clone(), Instant::now());
                if !self.wanted.contains(&id) {
                    // User disconnected while we were connecting.
                    if let Some(p) = self.peripherals.get(&id) {
                        let _ = p.disconnect().await;
                    }
                    return;
                }
                self.connected.insert(id.clone());
                if let Some(c) = controller {
                    let replace = self.controller.as_ref().is_none_or(|old| old.id == c.id);
                    if replace {
                        self.spawn_writer(c.clone());
                        self.controller = Some(c);
                    }
                }
            }
            Internal::Failed { id, err, attempt } => {
                self.connecting.remove(&id);
                // Log the first failure and then only occasionally.
                if attempt % 5 == 0 {
                    let retry = if self.wanted.contains(&id) {
                        " — retrying"
                    } else {
                        ""
                    };
                    self.ui.log(format!(
                        "{}: {}{retry}",
                        self.name_of(&id),
                        friendly_error(&err)
                    ));
                }
                if self.wanted.contains(&id) {
                    self.connect(id, attempt + 1);
                } else {
                    self.on_disconnected(&id);
                }
            }
        }
    }

    async fn central_event(&mut self, e: CentralEvent) {
        if let CentralEvent::DeviceDisconnected(pid) = e {
            let id = pid.to_string();
            if !self.connected.contains(&id) {
                return;
            }
            self.connected.remove(&id);
            self.drop_controller(&id);
            let after = self
                .connected_at
                .remove(&id)
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(0);
            if self.wanted.contains(&id) {
                self.ui.log(format!(
                    "{} disconnected after {after} s, reconnecting…",
                    self.name_of(&id)
                ));
                self.connect(id, 1);
            } else {
                self.on_disconnected(&id);
            }
        }
    }

    fn drop_controller(&mut self, id: &str) {
        if self.controller.as_ref().is_some_and(|c| c.id == id) {
            self.controller = None;
            self.writer_gen.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn on_disconnected(&mut self, id: &str) {
        self.connected.remove(id);
        self.drop_controller(id);
        self.ui.send(Event::Link {
            id: id.to_string(),
            name: self.name_of(id),
            link: Link::Disconnected,
            caps: vec![],
            controllable: false,
        });
    }

    /// Start the task that sends trainer commands, replacing any previous one.
    fn spawn_writer(&mut self, mut ctrl: Controller) {
        let generation = self.writer_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let gen_cell = self.writer_gen.clone();
        let mut rx = self.control_tx.subscribe();
        // Apply the current target straight away (e.g. after a reconnect).
        rx.mark_changed();
        let ui = self.ui.clone();
        let mass = self.mass;
        tokio::spawn(async move {
            let mut held = false;
            loop {
                tokio::select! {
                    r = rx.changed() => if r.is_err() { break },
                    // While a command is held back, poll for the end of calibration.
                    _ = tokio::time::sleep(Duration::from_millis(500)), if held => {}
                }
                if gen_cell.load(Ordering::SeqCst) != generation {
                    break;
                }
                let latest = *rx.borrow_and_update();
                if CALIBRATING.load(Ordering::SeqCst) {
                    held = true;
                    continue;
                }
                held = false;
                if let Some(c) = latest {
                    write_control(&mut ctrl, c, mass, &ui).await;
                }
            }
        });
    }
}

async fn write_control(ctrl: &mut Controller, c: Control, mass: f32, ui: &Ui) {
    let p = ctrl.peripheral.clone();
    let (ch, data) = match (&mut ctrl.kind, c) {
        (ControlKind::Ftms(ch), Control::Erg(w)) => {
            (ch.clone(), vec![ftms::set_target_power(w as i16)])
        }
        (ControlKind::Ftms(ch), Control::Grade(g)) => {
            (ch.clone(), vec![ftms::set_simulation(g, 0.0, 0.004, 0.51)])
        }
        (ControlKind::Wahoo { ch, sim_mode, .. }, Control::Erg(w)) => {
            *sim_mode = false;
            (ch.clone(), vec![wahoo::set_erg(w)])
        }
        (ControlKind::Wahoo { ch, sim_mode, .. }, Control::Grade(g)) => {
            let mut cmds = Vec::new();
            if !*sim_mode {
                cmds.push(wahoo::set_sim_mode(mass, 0.004, 0.51));
                *sim_mode = true;
            }
            cmds.push(wahoo::set_grade(g));
            (ch.clone(), cmds)
        }
    };
    for d in data {
        if RAW_LOG.load(Ordering::Relaxed) {
            let hex: Vec<String> = d.iter().map(|b| format!("{b:02x}")).collect();
            ui.log(format!("sent to trainer ({c:?}): {}", hex.join(" ")));
        }
        let write = async {
            match &mut ctrl.kind {
                ControlKind::Wahoo { last_write, .. } => wahoo_write(&p, &ch, &d, last_write).await,
                ControlKind::Ftms(_) => p.write(&ch, &d, WriteType::WithResponse).await,
            }
        };
        match tokio::time::timeout(Duration::from_secs(5), write).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => ui.log(format!(
                "Trainer command failed: {}. If the Wahoo app or Zwift is open on another device, close it.",
                friendly_error(&e.to_string())
            )),
            Err(_) => ui.log("Trainer command timed out"),
        }
    }
}

/// Find the device's current BlueZ object, scanning until it reappears.
async fn find_live(a: &Adapter, id: &str) -> anyhow::Result<Peripheral> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut scanning = false;
    let found = loop {
        if let Ok(list) = a.peripherals().await {
            if let Some(p) = list.into_iter().find(|p| p.id().to_string() == id) {
                break Some(p);
            }
        }
        if !scanning {
            scanning = a.start_scan(ScanFilter::default()).await.is_ok();
        }
        if Instant::now() > deadline {
            break None;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    };
    if scanning {
        let _ = a.stop_scan().await;
    }
    found.ok_or_else(|| anyhow::anyhow!("not found nearby — is it awake? (pedal to wake it)"))
}

/// Turn BlueZ/D-Bus errors into something a rider can act on.
fn friendly_error(err: &str) -> String {
    let e = err.to_lowercase();
    if e.contains("doesn't exist") || e.contains("unknownobject") || e.contains("unknown object") {
        "lost track of the device, searching for it again".into()
    } else if e.contains("le-connection-abort") || e.contains("connection-abort") {
        "the Bluetooth adapter aborted the connection".into()
    } else if e.contains("timed out") || e.contains("timeout") {
        "connection timed out — is it awake and in range?".into()
    } else if e.contains("in progress") {
        "a connection attempt is already in progress".into()
    } else {
        err.to_string()
    }
}

/// Connect, discover services, subscribe to everything useful and take
/// control of the trainer if it has a control point.
async fn setup(
    p: Peripheral,
    id: String,
    name: String,
    mass: f32,
    ui: Ui,
    (gen_cell, generation): (Arc<AtomicU32>, u32),
) -> anyhow::Result<Option<Controller>> {
    if p.is_connected().await? {
        // Connected by the system (e.g. paired in the desktop's Bluetooth
        // settings): BlueZ may still be resolving its services.
        ui.log(format!("{name}: already connected by the system"));
    } else {
        // BlueZ sometimes finishes connecting well before the connect call
        // returns, so also check every second and carry on as soon as the
        // device is connected.
        let connect = p.connect_with_timeout(Duration::from_secs(20));
        tokio::pin!(connect);
        let mut poll = tokio::time::interval(Duration::from_secs(1));
        poll.tick().await;
        loop {
            tokio::select! {
                r = &mut connect => {
                    r?;
                    break;
                }
                _ = poll.tick() => {
                    if p.is_connected().await.unwrap_or(false) {
                        break;
                    }
                }
            }
        }
    }
    const KNOWN: [uuid::Uuid; 6] = [
        protocol::INDOOR_BIKE_DATA,
        protocol::FTMS_CONTROL_POINT,
        protocol::CPS_MEASUREMENT,
        protocol::HR_MEASUREMENT,
        protocol::CSC_MEASUREMENT,
        protocol::WAHOO_TRAINER,
    ];
    let mut chars = Default::default();
    for _ in 0..15 {
        p.discover_services_with_timeout(Duration::from_secs(20))
            .await?;
        chars = p.characteristics();
        if chars.iter().any(|c| KNOWN.contains(&c.uuid)) {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let services: Vec<String> = p
        .services()
        .iter()
        .map(|s| protocol::describe_uuid(s.uuid))
        .collect();
    ui.log(format!(
        "{name}: services: {}",
        if services.is_empty() {
            "none".into()
        } else {
            services.join(", ")
        }
    ));
    let find = |u| chars.iter().find(|c| c.uuid == u).cloned();
    let ibd = find(protocol::INDOOR_BIKE_DATA);
    let cp = find(protocol::FTMS_CONTROL_POINT);
    let status = find(protocol::FTMS_STATUS);
    let cpm = find(protocol::CPS_MEASUREMENT);
    let hr = find(protocol::HR_MEASUREMENT);
    let csc = find(protocol::CSC_MEASUREMENT);
    let wahoo_ch = find(protocol::WAHOO_TRAINER);

    let mut stream = p.notifications().await?;
    for c in [&ibd, &cpm, &hr, &csc].into_iter().flatten() {
        if let Err(e) = p.subscribe(c).await {
            ui.log(format!(
                "{name}: could not subscribe to {}: {e}",
                protocol::describe_uuid(c.uuid)
            ));
        }
    }
    let subscribed = [&ibd, &cpm, &hr, &csc]
        .iter()
        .filter(|c| c.is_some())
        .count();
    let has_ibd = ibd.is_some();
    let got_data = Arc::new(AtomicBool::new(false));
    let data_ui = ui.clone();
    let data_name = name.clone();
    let data_id = id.clone();
    let data_flag = got_data.clone();
    tokio::spawn(async move {
        let start = Instant::now();
        let mut cad = CadenceCalc::default();
        let mut wheel = protocol::WheelSpeedCalc::default();
        let mut seen = HashSet::new();
        let mut last_contact: Option<bool> = None;
        let mut beats = protocol::BeatCheck::default();
        let mut last_noisy: Option<bool> = None;
        while let Some(n) = stream.next().await {
            if gen_cell.load(Ordering::SeqCst) != generation {
                break;
            }
            if RAW_LOG.load(Ordering::Relaxed) {
                let hex: Vec<String> = n.value.iter().map(|b| format!("{b:02x}")).collect();
                data_ui.log(format!(
                    "{data_name} {}: {}",
                    protocol::describe_uuid(n.uuid),
                    hex.join(" ")
                ));
            }
            if seen.insert(n.uuid) {
                data_ui.log(format!(
                    "{data_name}: receiving {}",
                    protocol::describe_uuid(n.uuid)
                ));
            }
            data_flag.store(true, Ordering::Relaxed);
            let now = start.elapsed().as_secs_f64();
            let mut r = Reading::default();
            if n.uuid == protocol::INDOOR_BIKE_DATA {
                let Some(d) = protocol::parse_indoor_bike_data(&n.value) else {
                    continue;
                };
                r.power = d.power.map(|p| p.max(0) as f32);
                r.cadence = d.cadence;
                r.speed_kmh = d.speed_kmh;
                r.hr = d.heart_rate.filter(|&h| h > 0).map(f32::from);
            } else if n.uuid == protocol::CPS_MEASUREMENT {
                let Some(d) = protocol::parse_cycling_power(&n.value) else {
                    continue;
                };
                if !has_ibd {
                    r.power = Some(d.power.max(0) as f32);
                    r.speed_kmh = d.wheel.map(|w| wheel.update(w, now));
                }
                r.cadence = d.crank.map(|c| cad.update(c, now));
            } else if n.uuid == protocol::HR_MEASUREMENT {
                let Some(h) = protocol::parse_heart_rate(&n.value) else {
                    continue;
                };
                if h.contact != last_contact {
                    match h.contact {
                        Some(false) => data_ui.log(format!(
                            "{data_name}: no skin contact — readings ignored. Wet the strap's electrodes and tighten it."
                        )),
                        Some(true) if last_contact == Some(false) => data_ui.log(format!("{data_name}: skin contact OK")),
                        _ => {}
                    }
                    last_contact = h.contact;
                }
                r.hr_contact = h.contact;
                r.hr_rr = protocol::rr_bpm(&h.rr);
                let noisy = beats.update(&h.rr);
                if noisy.is_some() && noisy != last_noisy {
                    if noisy == Some(true) {
                        data_ui.log(format!(
                            "{data_name}: heart rate signal is noisy (beats detected at random intervals) — readings ignored. \
                             Wet the electrodes, tighten the strap and check its battery."
                        ));
                    } else if last_noisy == Some(true) {
                        data_ui.log(format!("{data_name}: heart rate signal is steady again"));
                    }
                    last_noisy = noisy;
                }
                r.hr_noisy = noisy;
                // Without skin contact, or when the beat timing is erratic, a
                // strap reports noise (often around 190-200 bpm).
                let plausible = (25..=240).contains(&h.bpm);
                let trusted = plausible && h.contact != Some(false) && noisy != Some(true);
                r.hr = trusted.then_some(f32::from(h.bpm));
            } else if n.uuid == protocol::CSC_MEASUREMENT {
                r.cadence = protocol::parse_csc(&n.value).map(|c| cad.update(c, now));
            } else if n.uuid == protocol::FTMS_STATUS {
                use ftms::SpinDownStatus::*;
                let event = match ftms::parse_spin_down_status(&n.value) {
                    Some(SpeedUp) => Calibration::SpeedUp,
                    Some(StopPedaling) => Calibration::StopPedaling,
                    Some(Success) => Calibration::Success,
                    Some(Error) => Calibration::Failed("The trainer reported a calibration error. Try again, keeping a steady speed.".into()),
                    None => continue,
                };
                if matches!(event, Calibration::Success | Calibration::Failed(_)) {
                    CALIBRATING.store(false, Ordering::SeqCst);
                }
                data_ui.send(Event::Calibration(event));
                continue;
            } else if n.uuid == protocol::FTMS_CONTROL_POINT {
                if let Some((op, result)) = ftms::parse_response(&n.value) {
                    if op == ftms::OP_SPIN_DOWN {
                        let event = match ftms::parse_spin_down_targets(&n.value) {
                            Some((lo, hi)) => Calibration::Targets(lo, hi),
                            None if result == 0x01 => Calibration::SpeedUp,
                            None => {
                                CALIBRATING.store(false, Ordering::SeqCst);
                                Calibration::Failed(format!(
                                    "The trainer refused calibration ({}). Run the spindown in the Wahoo app instead.",
                                    ftms::result_text(result)
                                ))
                            }
                        };
                        data_ui.send(Event::Calibration(event));
                    } else if result != 0x01 {
                        data_ui.log(format!(
                            "{data_name}: trainer rejected command 0x{op:02x} ({})",
                            ftms::result_text(result)
                        ));
                    }
                }
                continue;
            } else {
                continue;
            }
            data_ui.send(Event::Data {
                id: data_id.clone(),
                reading: r,
            });
        }
    });
    if subscribed > 0 {
        let watch_ui = ui.clone();
        let watch_name = name.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(8)).await;
            if !got_data.load(Ordering::Relaxed) {
                watch_ui.log(format!(
                    "{watch_name}: connected but no data received yet. Pedal to wake it; if it stays silent, \
                     remove (forget) it in your system Bluetooth settings and connect again from VeloForge."
                ));
            }
        });
    } else {
        ui.log(format!(
            "{name}: no power, trainer, heart rate or cadence services found"
        ));
    }

    let mut caps = Vec::new();
    if ibd.is_some() || cp.is_some() {
        caps.push(Capability::SmartTrainer);
    } else if wahoo_ch.is_some() {
        caps.push(Capability::LegacyWahoo);
    }
    if cpm.is_some() {
        caps.push(Capability::Power);
    }
    if hr.is_some() {
        caps.push(Capability::HeartRate);
    }
    if csc.is_some() {
        caps.push(Capability::Cadence);
    }

    let write = |ch: Characteristic, d: Vec<u8>| {
        let p = p.clone();
        async move { p.write(&ch, &d, WriteType::WithResponse).await }
    };
    let controller = if let Some(cp) = cp {
        if let Some(st) = &status {
            if let Err(e) = p.subscribe(st).await {
                ui.log(format!(
                    "{name}: could not subscribe to trainer status: {e}"
                ));
            }
        }
        p.subscribe(&cp).await?;
        write(cp.clone(), ftms::request_control()).await?;
        write(cp.clone(), ftms::start()).await?;
        Some(Controller {
            id: id.clone(),
            peripheral: p.clone(),
            kind: ControlKind::Ftms(cp),
        })
    } else if let Some(ch) = wahoo_ch {
        let _ = p.subscribe(&ch).await;
        // Give the trainer a moment after (re)connecting before commanding it.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let mut last_write = Instant::now();
        wahoo_write(&p, &ch, &wahoo::unlock(), &mut last_write).await?;
        wahoo_write(
            &p,
            &ch,
            &wahoo::set_sim_mode(mass, 0.004, 0.51),
            &mut last_write,
        )
        .await?;
        Some(Controller {
            id: id.clone(),
            peripheral: p.clone(),
            kind: ControlKind::Wahoo {
                ch,
                sim_mode: true,
                last_write,
            },
        })
    } else {
        None
    };

    ui.log(format!(
        "{name}: {}",
        match controller.as_ref().map(|c| &c.kind) {
            Some(ControlKind::Ftms(_)) => "resistance control via FTMS",
            Some(ControlKind::Wahoo { .. }) => "resistance control via Wahoo legacy protocol",
            None => "no resistance control (sensor only)",
        }
    ));
    ui.send(Event::Link {
        id,
        name,
        link: Link::Connected,
        caps,
        controllable: controller.is_some(),
    });
    Ok(controller)
}
