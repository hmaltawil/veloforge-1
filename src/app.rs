//! The egui application: tabs for riding, workouts, the training calendar,
//! history, devices and settings.

use crate::charts;
use crate::devices::{
    self, Calibration, Capability, Cmd, DeviceHandle, DeviceInfo, Event, Link, SIM_ID,
};
use crate::gfx::GpuWorld;
use crate::plan::{self, Goal, PlanConfig, PlannedWorkout};
use crate::platform::TRASH;
use crate::ride::{self, Live, Mode, Physics, Ride, Summary};
use crate::route::{self, Route};
use crate::sound::{Cue, Sound};
use crate::store::{Paths, Store};
use crate::theme::{self, ACCENT, CARD};
use crate::workout::{self, fmt_dur, Category, Step, Workout};
use crate::world::{Figure, Scene, Sky, TimeOfDay, Weather, World};
use chrono::{Datelike, Duration, Local, NaiveDate};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};
use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::time::Instant;

#[path = "hud.rs"]
mod hud;
#[path = "settings_ui.rs"]
mod settings_ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Ride,
    Workouts,
    Plan,
    History,
    Progress,
    Devices,
    Settings,
}

struct LinkInfo {
    name: String,
    link: Link,
    caps: Vec<Capability>,
    controllable: bool,
}

#[derive(Clone)]
struct Metric {
    v: f32,
    at: Instant,
    /// Device id the value came from.
    src: String,
}

impl Metric {
    fn is_fresh(&self) -> bool {
        self.at.elapsed().as_secs_f32() < 3.0
    }
}

#[derive(Default)]
struct Metrics {
    power: Option<Metric>,
    cadence: Option<Metric>,
    hr: Option<Metric>,
    speed: Option<Metric>,
    recent_power: VecDeque<(Instant, f32)>,
    /// Arrival times of recent packets (per-device diagnostics).
    updates: VecDeque<Instant>,
    /// Heart rate strap skin contact, as last reported.
    hr_contact: Option<bool>,
    /// Heart rate from beat-to-beat intervals.
    hr_rr: Option<Metric>,
    hr_noisy: Option<bool>,
    /// When any cadence was last received (cadence tiles hide without it).
    cadence_seen: Option<Instant>,
}

fn fresh(m: &Option<Metric>) -> Option<f32> {
    m.as_ref().filter(|m| m.is_fresh()).map(|m| m.v)
}

/// Store a reading, never letting the simulator override a live real sensor.
fn set_metric(slot: &mut Option<Metric>, v: f32, src: &str) -> bool {
    let sim = src == SIM_ID;
    if sim
        && slot
            .as_ref()
            .is_some_and(|m| m.src != SIM_ID && m.is_fresh())
    {
        return false;
    }
    *slot = Some(Metric {
        v,
        at: Instant::now(),
        src: src.to_string(),
    });
    true
}

impl Metrics {
    fn power3s(&self) -> Option<f32> {
        fresh(&self.power)?;
        let v: Vec<f32> = self
            .recent_power
            .iter()
            .filter(|(t, _)| t.elapsed().as_secs_f32() < 3.0)
            .map(|x| x.1)
            .collect();
        (!v.is_empty()).then(|| v.iter().sum::<f32>() / v.len() as f32)
    }
}

enum Modal {
    Saved(Box<Summary>),
    ConfirmDiscard,
    ConfirmDelete(usize),
}

#[derive(Debug, Clone, PartialEq)]
enum CalStage {
    Intro,
    Starting,
    SpeedUp,
    Coast,
    Done,
    Failed(String),
}

/// The trainer calibration (spindown) dialog.
struct CalibUi {
    stage: CalStage,
    /// Target speed window in km/h, if the trainer gave one.
    targets: Option<(f32, f32)>,
    since: Instant,
}

pub struct App {
    dev: DeviceHandle,
    store: Store,
    paths: Paths,
    dirty: bool,
    last_save: Instant,
    tab: Tab,

    adapter: Option<Result<String, String>>,
    scanning: bool,
    found: Vec<DeviceInfo>,
    links: BTreeMap<String, LinkInfo>,
    log: VecDeque<String>,
    metrics: Metrics,
    per_device: std::collections::HashMap<String, Metrics>,
    sim_effort: f32,
    raw_log: bool,
    show_all_devices: bool,

    routes: Vec<Route>,
    route_idx: usize,
    ride: Ride,
    ride_workout: Option<String>,
    world: World,
    last_frame: Instant,

    selected_workout: Option<String>,
    editing: bool,
    workout_filter: String,
    schedule_date: NaiveDate,

    plan_cfg: PlanConfig,
    replace_plan: bool,
    month: NaiveDate,
    sel_day: NaiveDate,
    add_workout: String,

    import_path: String,
    toast: Option<(String, Instant)>,
    modal: Option<Modal>,
    calib: Option<CalibUi>,
    details: Option<RideDetails>,
    sound: Sound,
    gpu: GpuWorld,
    /// The window has a depth buffer, so the GPU view can run.
    gpu_capable: bool,
    gpu_reported: bool,
    logo: egui::TextureHandle,
    icu: crate::icu::Worker,
    /// Saved credentials (what syncing uses).
    icu_creds: crate::icu::Credentials,
    /// Credentials being edited in Settings.
    icu_edit: crate::icu::Credentials,
    icu_status: Option<Result<String, String>>,
    icu_athlete: Option<crate::icu::Athlete>,
    icu_busy: bool,
    icu_last_sync: Option<Instant>,
    icu_note: Option<String>,
    last_checkpoint: Instant,
    /// Rides whose power curve could not be computed (missing file).
    curve_failed: std::collections::HashSet<String>,
    /// "Keep the plan" was chosen for today's tiredness suggestion.
    tired_dismissed: Option<NaiveDate>,
    /// Started with --demo: skip the welcome.
    demo: bool,
    egui_ctx: egui::Context,
    remote: Option<crate::remote::Remote>,
    remote_url: String,
    remote_error: Option<String>,
    remote_pushed: Instant,
    settings_page: settings_ui::SettingsPage,
    /// What the last check for a newer version found.
    update: Option<crate::update::Status>,
    update_rx: Option<std::sync::mpsc::Receiver<crate::update::Status>>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, depth_buffer: bool) -> App {
        theme::apply(&cc.egui_ctx);
        install_fallback_font(&cc.egui_ctx);
        let logo = cc.egui_ctx.load_texture(
            "logo",
            egui::ColorImage::from_rgba_unmultiplied(
                [64, 64],
                include_bytes!("../packaging/icons/veloforge-64.rgba"),
            ),
            egui::TextureOptions::LINEAR,
        );

        let paths = Paths::new();
        let mut store = Store::load(&paths.state_file());
        // People who already ride with VeloForge do not need the welcome.
        if !store.history.is_empty() || !store.settings.auto_connect.is_empty() {
            store.settings.onboarded = true;
        }
        let sound = Sound::new(paths.data.join("sounds"), store.settings.volume);
        let dev = devices::spawn(cc.egui_ctx.clone());
        let s = &store.settings;
        dev.send(Cmd::SetMass(s.rider_kg + s.bike_kg));
        if !s.auto_connect.is_empty() {
            dev.send(Cmd::AutoConnect(s.auto_connect.clone()));
        }
        let today = Local::now().date_naive();
        let mut app = App {
            dev,
            paths,
            dirty: false,
            last_save: Instant::now(),
            tab: Tab::Ride,
            adapter: None,
            scanning: false,
            found: Vec::new(),
            links: BTreeMap::new(),
            log: VecDeque::new(),
            metrics: Metrics::default(),
            per_device: Default::default(),
            sim_effort: 180.0,
            raw_log: false,
            show_all_devices: false,
            routes: Vec::new(),
            route_idx: 0,
            ride: Ride::new(Mode::Route, None, None),
            ride_workout: None,
            world: World::default(),
            last_frame: Instant::now(),
            selected_workout: None,
            editing: false,
            workout_filter: String::new(),
            schedule_date: today,
            plan_cfg: PlanConfig {
                goal: Goal::FtpBuilder,
                start: today
                    + Duration::days(((7 - today.weekday().num_days_from_monday()) % 7) as i64),
                weeks: 8,
                days: [false, true, false, true, false, true, false],
                long_day: 5,
            },
            replace_plan: true,
            month: plan::first_of_month(today),
            sel_day: today,
            add_workout: "endurance-60".into(),
            import_path: String::new(),
            toast: None,
            modal: None,
            calib: None,
            details: None,
            sound,
            gpu: GpuWorld::default(),
            gpu_capable: depth_buffer && cc.gl.is_some(),
            gpu_reported: false,
            logo,
            icu: match std::env::var("VELOFORGE_ICU_URL") {
                Ok(url) => crate::icu::Worker::spawn_with_base(cc.egui_ctx.clone(), url),
                Err(_) => crate::icu::Worker::spawn(cc.egui_ctx.clone()),
            },
            icu_creds: Default::default(),
            icu_edit: Default::default(),
            icu_status: None,
            icu_athlete: None,
            icu_busy: false,
            icu_last_sync: None,
            icu_note: None,
            egui_ctx: cc.egui_ctx.clone(),
            remote: None,
            remote_url: String::new(),
            remote_error: None,
            remote_pushed: Instant::now(),
            settings_page: Default::default(),
            update: None,
            update_rx: None,
            last_checkpoint: Instant::now(),
            curve_failed: Default::default(),
            demo: false,
            tired_dismissed: None,
            store,
        };
        app.rebuild_routes();
        app.log_line(format!(
            "VeloForge {} started ({} graphics)",
            env!("CARGO_PKG_VERSION"),
            if app.gpu_capable {
                "detailed"
            } else {
                "simple"
            }
        ));
        if let Some(gl) = &cc.gl {
            use eframe::glow::HasContext;
            // SAFETY: plain queries on the context eframe has just made current.
            let (renderer, version) = unsafe {
                (
                    gl.get_parameter_string(eframe::glow::RENDERER),
                    gl.get_parameter_string(eframe::glow::VERSION),
                )
            };
            // Which graphics chip draws the app (two on many laptops).
            app.log_line(format!("Graphics: {renderer} (OpenGL {version})"));
        }
        app.icu_creds = crate::icu::Credentials::load(&app.paths.credentials_file());
        app.icu_edit = app.icu_creds.clone();
        if app.icu_creds.is_set() {
            app.icu_busy = true;
            app.icu
                .send(crate::icu::Request::Test(app.icu_creds.clone()));
        }
        if app.store.settings.check_updates {
            app.check_for_update();
        }
        app.recover_checkpoint();
        if app.store.settings.remote_enabled {
            app.start_remote();
        }
        // `--demo`: ride the simulator straight away, no hardware needed.
        let args: Vec<String> = std::env::args().collect();
        if args.iter().any(|a| a == "--demo") {
            app.demo = true;
            app.dev.send(Cmd::SimEnable(true));
            if args.iter().any(|a| a == "--heart-rate") {
                app.ride = Ride::new(Mode::HeartRate, None, None);
                app.ride.hr_control =
                    ride::HrControl::new((app.store.settings.max_hr * 0.65).round());
            }
            // `--workout [id]`: a workout (sweet spot unless named), starting
            // 10 s before the end of its first step.
            if let Some(i) = args.iter().position(|a| a == "--workout") {
                let id = args
                    .get(i + 1)
                    .filter(|v| !v.starts_with("--"))
                    .cloned()
                    .unwrap_or_else(|| "ss-3x10".into());
                app.load_ride(Some(id), None);
                let first = app.ride.segs.first().map_or(0.0, |s| s.end());
                app.ride.skip_to((first - 10.0).max(0.0) as f64);
            }
            // `--km 1.5`: start partway along the route.
            if let Some(km) = args
                .iter()
                .position(|a| a == "--km")
                .and_then(|i| args.get(i + 1))
                .and_then(|v| v.parse::<f64>().ok())
            {
                app.ride.distance = km * 1000.0;
            }
            if args.iter().any(|a| a == "--pacer") {
                let w = ((app.store.settings.ftp * 0.75) / 5.0).round() * 5.0;
                app.ride.companion = Some(ride::Companion::pacer(w));
            }
            if args.iter().any(|a| a == "--ghost") {
                if let Some(h) = app.ghost_candidates().first().map(|h| (*h).clone()) {
                    app.load_ghost(&h);
                }
            }
            app.ride.start();
            return app;
        }
        // Offer today's planned workout on the ride screen.
        if let Some(p) = app
            .store
            .plan
            .iter()
            .find(|p| p.date == today && !p.done)
            .cloned()
        {
            app.load_ride(Some(p.workout_id), Some(p.date));
        }
        app
    }

    fn rebuild_routes(&mut self) {
        self.routes = route::builtin();
        self.routes
            .extend(self.store.gpx_routes.iter().map(|g| g.to_route()));
        self.route_idx = self
            .routes
            .iter()
            .position(|r| r.id == self.store.settings.route_id)
            .unwrap_or(0);
    }

    fn library(&self) -> Vec<Workout> {
        let mut v = workout::builtin();
        v.extend(self.store.custom_workouts.iter().cloned());
        v
    }

    fn find_workout(&self, id: &str) -> Option<Workout> {
        self.store
            .custom_workouts
            .iter()
            .find(|w| w.id == id)
            .cloned()
            .or_else(|| workout::builtin().into_iter().find(|w| w.id == id))
    }

    fn notify(&mut self, s: impl Into<String>) {
        self.toast = Some((s.into(), Instant::now()));
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn save_now(&mut self) {
        self.record_ftp();
        if let Err(e) = self.store.save(&self.paths.state_file()) {
            self.notify(format!("Could not save: {e}"));
        }
        self.dirty = false;
        self.last_save = Instant::now();
    }

    /// Remember the FTP whenever it changes (one entry per day).
    fn record_ftp(&mut self) {
        let ftp = self.store.settings.ftp;
        let today = Local::now().date_naive();
        let h = &mut self.store.ftp_history;
        match h.last_mut() {
            Some(last) if (last.ftp - ftp).abs() < 0.5 => {}
            Some(last) if last.date == today => last.ftp = ftp,
            _ => h.push(crate::progress::FtpEntry { date: today, ftp }),
        }
    }

    fn ride_active(&self) -> bool {
        self.ride.state != ride::State::Ready
    }

    fn load_ride(&mut self, workout_id: Option<String>, plan_date: Option<NaiveDate>) {
        if self.ride_active() {
            self.notify("Finish the current ride first");
            return;
        }
        let w = workout_id.as_deref().and_then(|id| self.find_workout(id));
        let mode = if w.is_some() {
            Mode::Workout
        } else {
            Mode::Route
        };
        self.ride_workout = w.as_ref().map(|w| w.id.clone());
        self.replace_ride(Ride::new(mode, w, plan_date));
    }

    /// Swap in a new ride, keeping whoever you were riding with.
    fn replace_ride(&mut self, mut ride: Ride) {
        ride.companion = self.ride.companion.as_ref().map(ride::Companion::restarted);
        ride.hr_control.target_bpm = self.ride.hr_control.target_bpm;
        self.ride = ride;
    }

    /// Previous rides on the selected route a ghost can replay, fastest first.
    fn ghost_candidates(&self) -> Vec<&ride::Summary> {
        let route = &self.routes[self.route_idx].id;
        let mut v: Vec<&ride::Summary> = self
            .store
            .history
            .iter()
            .filter(|h| h.route_id.as_ref() == Some(route) && h.distance_m >= 1000.0)
            .collect();
        v.sort_by(|a, b| {
            let speed = |h: &ride::Summary| h.distance_m / h.secs.max(1.0) as f64;
            speed(b).partial_cmp(&speed(a)).unwrap()
        });
        v.truncate(8);
        v
    }

    fn ghost_label(h: &ride::Summary) -> String {
        h.start.format("%a %d %b, %H:%M").to_string()
    }

    fn load_ghost(&mut self, h: &ride::Summary) {
        let label = Self::ghost_label(h);
        let ghost = std::fs::read_to_string(&h.file)
            .ok()
            .and_then(|x| crate::activity::parse_tcx(&x).ok())
            .and_then(|a| ride::Ghost::from_samples(label, &a.samples));
        match ghost {
            Some(g) => self.ride.companion = Some(ride::Companion::Ghost(g)),
            None => self.notify("Could not read that ride's file for a ghost"),
        }
    }

    /// The pacer's jersey: a colour that stands out from yours.
    fn pacer_jersey(&self) -> Color32 {
        let [r, g, b] = self.store.settings.jersey;
        if b as u16 > (r as u16 + g as u16) / 2 + 20 {
            Color32::from_rgb(242, 196, 40)
        } else {
            Color32::from_rgb(40, 120, 220)
        }
    }

    /// "Pacer · 35 m ahead" style description of the companion's gap.
    fn companion_status(&self) -> Option<(String, String, String, Color32)> {
        let gap = self.ride.companion_gap()?;
        let (title, value, unit) = match self.ride.companion.as_ref()? {
            ride::Companion::Pacer(p) => {
                let (v, u) = gap_text(gap);
                (format!("PACER · {:.0} W", p.watts), v, u)
            }
            ride::Companion::Ghost(g) => match self.ride.ghost_time_gap() {
                Some(t) if self.ride.elapsed > 1.0 => (
                    "GHOST".to_string(),
                    fmt_dur(t.abs().round() as f32),
                    if t > 0.5 { "behind" } else { "ahead" }.to_string(),
                ),
                _ if self.ride.elapsed > g.duration() => (
                    "GHOST · finished".to_string(),
                    gap_text(gap).0,
                    gap_text(gap).1,
                ),
                _ => {
                    let (v, u) = gap_text(gap);
                    ("GHOST".to_string(), v, u)
                }
            },
        };
        let col = if gap > 1.0 {
            Color32::from_rgb(240, 150, 70)
        } else {
            Color32::from_rgb(110, 210, 130)
        };
        Some((title, value, unit, col))
    }

    // ------------------------------------------------------------------
    // Ride controls (keyboard and phone remote)

    fn ride_action(&mut self, a: crate::remote::Action) {
        use crate::remote::Action;
        match a {
            Action::Toggle => match self.ride.state {
                ride::State::Running => self.ride.state = ride::State::Paused,
                _ if self.can_start() => self.ride.start(),
                _ => {}
            },
            Action::Skip => {
                if self.ride.mode == Mode::Workout
                    && self.ride_active()
                    && !self.ride.workout_done()
                {
                    self.ride.skip_step();
                }
            }
            Action::View => {
                self.store.settings.view_3d = !self.store.settings.view_3d;
                self.mark_dirty();
            }
            Action::Finish => {
                if self.ride_active() {
                    self.finish_ride();
                }
            }
            Action::Up | Action::Down => {
                let delta = if a == Action::Up { 1.0 } else { -1.0 };
                match self.ride.mode {
                    Mode::Workout => {
                        self.ride.intensity = (self.ride.intensity + delta * 0.05).clamp(0.5, 1.5)
                    }
                    Mode::Slope => {
                        self.ride.manual_grade =
                            (self.ride.manual_grade + delta * 0.5).clamp(-10.0, 20.0)
                    }
                    Mode::Route => {
                        self.store.settings.difficulty =
                            (self.store.settings.difficulty + delta * 0.1).clamp(0.0, 1.0);
                        self.mark_dirty();
                    }
                    Mode::HeartRate => {
                        let t = &mut self.ride.hr_control.target_bpm;
                        *t = (*t + delta * 2.0).clamp(80.0, 200.0);
                    }
                }
            }
        }
    }

    fn start_remote(&mut self) {
        self.remote = None;
        let s = &mut self.store.settings;
        if s.remote_token.is_empty() {
            s.remote_token = crate::remote::new_token();
            self.dirty = true;
        }
        let ctx = self.egui_ctx.clone();
        let port = s.remote_port;
        match crate::remote::Remote::start(port, s.remote_token.clone(), move || {
            ctx.request_repaint()
        }) {
            Ok(r) => {
                self.remote_url = r.url();
                self.remote_error = None;
                self.remote = Some(r);
                self.log_line(format!("Phone remote listening on port {port}"));
            }
            Err(e) => {
                let msg = if e.kind() == std::io::ErrorKind::AddrInUse {
                    format!(
                        "Port {port} is already in use. Is VeloForge already open in another window?"
                    )
                } else {
                    format!("Could not start the phone remote: {e}")
                };
                self.log_line(msg.clone());
                self.remote_error = Some(msg);
            }
        }
    }

    /// Handle phone actions and publish the ride state for the phone.
    fn pump_remote(&mut self, ctx: &egui::Context) {
        let Some(r) = &self.remote else { return };
        let actions: Vec<_> = r.rx.try_iter().collect();
        for a in actions {
            self.ride_action(a);
        }
        let Some(r) = &self.remote else { return };
        if self.remote_pushed.elapsed().as_millis() >= 300 {
            self.remote_pushed = Instant::now();
            r.set_state(self.remote_state());
        }
        if r.phone_connected() {
            ctx.request_repaint_after(std::time::Duration::from_millis(350));
        }
    }

    /// Live numbers and controls for the phone page, as JSON.
    fn remote_state(&self) -> String {
        let s = &self.store.settings;
        let ftp = s.ftp;
        let hex = |c: Color32| format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b());
        let power = self.metrics.power3s();
        let hr = fresh(&self.metrics.hr);
        let target = self.ride.target_watts(ftp);
        let t = self.ride.workout_time() as f32;
        let interval = match (&self.ride.workout, self.ride.mode) {
            (Some(w), Mode::Workout) if self.ride.workout_done() => Some(serde_json::json!({
                "label": w.name,
                "now": "Workout complete!",
                "next": "Keep spinning to cool down, then finish",
                "progress": 1.0,
                "color": "#78dc8c",
            })),
            (Some(w), Mode::Workout) => workout::segment_index(&self.ride.segs, t).map(|i| {
                let seg = self.ride.segs[i];
                // The phone's cadence tile shows the current cadence target,
                // so this big line stays short enough not to wrap.
                let now = if seg.free {
                    "Free ride".to_string()
                } else {
                    format!("{:.0} W", seg.at(t) * self.ride.intensity * ftp)
                };
                let next = self.ride.segs.get(i + 1).map(|n| {
                    n.with_cadence(
                        if n.free {
                            format!("Next: {} free ride", fmt_dur(n.secs))
                        } else {
                            format!(
                                "Next: {} @ {:.0} W",
                                fmt_dur(n.secs),
                                n.from * self.ride.intensity * ftp
                            )
                        },
                        ", ",
                    )
                });
                let col = if seg.free {
                    Color32::GRAY
                } else {
                    charts::zone_color(seg.at(t))
                };
                serde_json::json!({
                    "label": format!("{} · {}/{}", w.name, i + 1, self.ride.segs.len()),
                    "now": format!("{now} · {} left", fmt_dur((seg.end() - t).ceil())),
                    "next": next.unwrap_or_else(|| "Last interval".into()),
                    "progress": (t - seg.start) / seg.secs,
                    "color": hex(col),
                })
            }),
            _ => None,
        };
        let (adjust_label, adjust_value) = match self.ride.mode {
            Mode::Workout => ("Intensity", format!("{:.0}%", self.ride.intensity * 100.0)),
            Mode::Slope => ("Slope", format!("{:+.1}%", self.ride.manual_grade)),
            Mode::Route => (
                "Trainer difficulty",
                format!("{:.0}%", s.difficulty * 100.0),
            ),
            Mode::HeartRate => (
                "Target heart rate",
                format!("{:.0} bpm", self.ride.hr_control.target_bpm),
            ),
        };
        let companion = self.companion_status().map(|(title, value, unit, _)| {
            let who = title.split(" · ").next().unwrap_or("").to_lowercase();
            let mut who = who.chars();
            let who: String = who
                .next()
                .map(|c| c.to_uppercase().chain(who).collect())
                .unwrap_or_default();
            format!("{who} {value} {unit}")
        });
        let grade = if self.ride.mode == Mode::Slope {
            self.ride.manual_grade as f64
        } else {
            self.routes[self.route_idx].grade(self.ride.route_pos())
        };
        serde_json::json!({
            "state": match self.ride.state {
                ride::State::Ready => "ready",
                ride::State::Running => "running",
                ride::State::Paused => "paused",
            },
            "title": format!("{} · {}", self.ride.title(), self.routes[self.route_idx].name),
            "power": power.map(f32::round),
            "power_color": power.map(|p| hex(charts::zone_color(p / ftp))),
            "hr": hr.map(f32::round),
            "hr_color": hr.map(|h| hex(charts::hr_zone_color(charts::hr_zone(h, s.max_hr)))),
            "target": target,
            "target_color": target.map(|t| hex(charts::zone_color(t / ftp))),
            "cadence": fresh(&self.metrics.cadence).map(f32::round),
            "cadence_target": self.ride.target_cadence().map(f32::round),
            "speed": self.ride.speed * 3.6,
            "grade": grade,
            "time": fmt_dur(self.ride.elapsed as f32),
            "distance": self.ride.distance / 1000.0,
            "interval": interval,
            "companion": companion,
            "can_start": self.can_start(),
            "can_skip": self.ride.mode == Mode::Workout && self.ride_active() && !self.ride.workout_done(),
            "adjust_label": adjust_label,
            "adjust_value": adjust_value,
            "view_3d": s.view_3d,
        })
        .to_string()
    }

    // ------------------------------------------------------------------
    // Device events and ride ticking

    fn pump_events(&mut self) {
        while let Ok(e) = self.dev.rx.try_recv() {
            match e {
                Event::Adapter(a) => {
                    self.log_line(match &a {
                        Ok(name) => format!("Bluetooth ready: {name}"),
                        Err(e) => format!("Bluetooth unavailable: {e}"),
                    });
                    self.adapter = Some(a);
                }
                Event::Scanning(s) => self.scanning = s,
                Event::Devices(d) => self.found = d,
                Event::Link {
                    id,
                    name,
                    link,
                    caps,
                    controllable,
                } => {
                    if link == Link::Connected
                        && id != SIM_ID
                        && !self.store.settings.auto_connect.contains(&id)
                    {
                        self.store.settings.auto_connect.push(id.clone());
                        self.mark_dirty();
                    }
                    let entry = self.links.entry(id).or_insert(LinkInfo {
                        name: name.clone(),
                        link,
                        caps: vec![],
                        controllable: false,
                    });
                    let was_controlling = entry.controllable && entry.link == Link::Connected;
                    entry.name = name;
                    entry.link = link;
                    if !caps.is_empty() || link == Link::Disconnected {
                        entry.caps = caps;
                        entry.controllable = controllable;
                    }
                    let name = entry.name.clone();
                    if link == Link::Connected {
                        self.log_line(format!("Connected: {name}"));
                    } else if was_controlling && self.ride.state == ride::State::Running {
                        self.cue(Cue::Alert);
                        self.notify(format!("{name} disconnected — reconnecting…"));
                    }
                }
                Event::Calibration(c) => self.on_calibration(c),
                Event::Data { id, reading } => {
                    let now = Instant::now();
                    let dev = self.per_device.entry(id.clone()).or_default();
                    dev.updates.push_back(now);
                    while dev
                        .updates
                        .front()
                        .is_some_and(|t| t.elapsed().as_secs_f32() > 5.0)
                    {
                        dev.updates.pop_front();
                    }
                    if let Some(p) = reading.power {
                        set_metric(&mut dev.power, p, &id);
                        if set_metric(&mut self.metrics.power, p, &id) {
                            self.metrics.recent_power.push_back((now, p));
                            while self.metrics.recent_power.len() > 64 {
                                self.metrics.recent_power.pop_front();
                            }
                        }
                    }
                    if let Some(v) = reading.speed_kmh {
                        set_metric(&mut dev.speed, v, &id);
                        set_metric(&mut self.metrics.speed, v, &id);
                    }
                    if let Some(c) = reading.cadence {
                        set_metric(&mut dev.cadence, c, &id);
                        set_metric(&mut self.metrics.cadence, c, &id);
                    }
                    if reading.hr_contact.is_some() {
                        dev.hr_contact = reading.hr_contact;
                    }
                    if reading.hr_noisy.is_some() {
                        dev.hr_noisy = reading.hr_noisy;
                    }
                    if reading.cadence.is_some() {
                        self.metrics.cadence_seen = Some(now);
                    }
                    if let Some(rr) = reading.hr_rr {
                        set_metric(&mut dev.hr_rr, rr, &id);
                    }
                    if let Some(h) = reading.hr {
                        set_metric(&mut dev.hr, h, &id);
                        // Prefer a dedicated heart rate sensor over HR relayed by a trainer.
                        let dedicated = self
                            .links
                            .get(&id)
                            .is_some_and(|l| l.caps.contains(&Capability::HeartRate));
                        let current_dedicated = self.metrics.hr.as_ref().is_some_and(|m| {
                            m.is_fresh()
                                && m.src != id
                                && self
                                    .links
                                    .get(&m.src)
                                    .is_some_and(|l| l.caps.contains(&Capability::HeartRate))
                        });
                        if dedicated || !current_dedicated {
                            set_metric(&mut self.metrics.hr, h, &id);
                        }
                    }
                }
                Event::Log(s) => self.log_line(s),
            }
        }
    }

    fn log_line(&mut self, s: String) {
        let now = Local::now();
        self.log
            .push_front(format!("{}  {s}", now.format("%H:%M:%S")));
        self.log.truncate(if self.raw_log { 800 } else { 200 });
        append_log_file(
            &self.paths.log_file(),
            &format!("{}  {s}", now.format("%Y-%m-%d %H:%M:%S")),
        );
    }

    fn tick_ride(&mut self) {
        // While the window is hidden ticks can be a second or more apart;
        // integrate in small steps so physics and timing stay accurate.
        let mut dt = self.last_frame.elapsed().as_secs_f64().min(5.0);
        self.last_frame = Instant::now();
        let s = &self.store.settings;
        let live = Live {
            power: fresh(&self.metrics.power).unwrap_or(0.0),
            cadence: fresh(&self.metrics.cadence),
            hr: fresh(&self.metrics.hr),
            speed_kmh: fresh(&self.metrics.speed),
        };
        let phys = Physics {
            mass: s.rider_kg + s.bike_kg,
            cda: s.cda,
            crr: s.crr,
            trainer_speed: s.trainer_speed,
        };
        let route = &self.routes[self.route_idx];
        let before = self.ride.workout_time() as f32;
        let mut control = None;
        while dt > 0.0 {
            let step = dt.min(0.1);
            dt -= step;
            if let Some(c) =
                self.ride
                    .update(step, live, route, phys, s.ftp, s.difficulty, s.auto_pause)
            {
                control = Some(c);
            }
        }
        if let Some(c) = control {
            self.dev.send(Cmd::Control(c));
        }
        let after = self.ride.workout_time() as f32;
        if self.ride.mode == Mode::Workout && after > before {
            match workout::beep_between(&self.ride.segs, before, after) {
                Some(workout::Beep::Countdown) => self.cue(Cue::Tick),
                Some(workout::Beep::Change) => self.cue(Cue::Change),
                Some(workout::Beep::Finish) => self.cue(Cue::Done),
                None => {}
            }
        }
    }

    fn can_start(&self) -> bool {
        let calibrating = self.calib.as_ref().is_some_and(|c| {
            matches!(
                c.stage,
                CalStage::Starting | CalStage::SpeedUp | CalStage::Coast
            )
        });
        !calibrating
            && (self.ride.mode != Mode::Workout
                || self
                    .ride
                    .workout
                    .as_ref()
                    .is_some_and(|w| w.duration() > 0.0))
    }

    fn cue(&self, cue: Cue) {
        if self.store.settings.sounds {
            self.sound.play(cue);
        }
    }

    fn on_calibration(&mut self, c: Calibration) {
        let Some(cal) = &mut self.calib else {
            // A late event after the dialog was closed.
            return;
        };
        cal.since = Instant::now();
        match c {
            Calibration::Started => cal.stage = CalStage::Starting,
            Calibration::Targets(lo, hi) => {
                cal.targets = Some((lo, hi));
                cal.stage = CalStage::SpeedUp;
            }
            Calibration::SpeedUp => cal.stage = CalStage::SpeedUp,
            Calibration::StopPedaling => {
                cal.stage = CalStage::Coast;
                self.cue(Cue::Change);
            }
            Calibration::Success => {
                cal.stage = CalStage::Done;
                self.cue(Cue::Done);
            }
            Calibration::Failed(m) => {
                cal.stage = CalStage::Failed(m);
                self.cue(Cue::Alert);
            }
            Calibration::Cancelled => self.calib = None,
        }
    }

    fn trainer_link(&self) -> Option<&LinkInfo> {
        self.links
            .values()
            .find(|l| l.controllable && l.link == Link::Connected)
    }

    fn finish_ride(&mut self) {
        if self.ride.samples.len() < 10 {
            self.notify("Ride too short to save");
            self.reset_ride();
            return;
        }
        let ftp = self.store.settings.ftp;
        let started = self.ride.started.unwrap_or_else(Local::now);
        let file = self.activity_path(started, &self.ride.title());
        let res = std::fs::create_dir_all(&self.paths.activities)
            .and_then(|_| std::fs::write(&file, self.ride.to_tcx()));
        if let Err(e) = res {
            self.notify(format!("Could not write activity: {e}"));
            return;
        }
        let mut summary =
            self.ride
                .summary(ftp, self.store.settings.max_hr, file.display().to_string());
        summary.route_id = Some(self.routes[self.route_idx].id.clone());
        self.tick_off_plan(
            summary.workout_id.as_deref(),
            self.ride.plan_date,
            started.date_naive(),
        );
        self.store.history.push(summary.clone());
        if self.store.settings.icu_upload {
            self.icu_upload(summary.file.clone(), summary.name.clone());
        }
        self.save_now();
        self.modal = Some(Modal::Saved(Box::new(summary)));
        self.reset_ride();
    }

    fn reset_ride(&mut self) {
        let mode = self.ride.mode;
        let w = self.ride.workout.clone();
        self.replace_ride(Ride::new(mode, w, None));
        self.dev.send(Cmd::Control(devices::Control::Grade(0.0)));
        self.clear_checkpoint();
    }

    fn icu_sync(&mut self) {
        if !self.icu_creds.is_set() || !self.store.settings.icu_sync_plan {
            return;
        }
        let today = Local::now().date_naive();
        self.icu_busy = true;
        self.icu_last_sync = Some(Instant::now());
        self.icu.send(crate::icu::Request::Sync {
            creds: self.icu_creds.clone(),
            oldest: today - Duration::days(1),
            newest: today + Duration::days(28),
        });
    }

    fn icu_upload(&mut self, file: String, name: String) {
        if self.icu_creds.is_set() {
            self.log_line(format!("Uploading “{name}” to intervals.icu…"));
            self.icu.send(crate::icu::Request::Upload {
                creds: self.icu_creds.clone(),
                file,
                name,
            });
        }
    }

    /// Ask GitHub, in the background, whether a newer version exists.
    fn check_for_update(&mut self) {
        let url = std::env::var("VELOFORGE_UPDATE_URL")
            .unwrap_or_else(|_| crate::update::LATEST_URL.to_string());
        self.update = Some(crate::update::Status::Checking);
        self.update_rx = Some(crate::update::check(self.egui_ctx.clone(), url));
    }

    fn pump_update(&mut self) {
        use crate::update::Status;
        use std::sync::mpsc::TryRecvError;
        let Some(rx) = &self.update_rx else { return };
        let status = match rx.try_recv() {
            Ok(status) => status,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Status::Failed("the check stopped".into()),
        };
        self.update_rx = None;
        match &status {
            Status::Available(r) => {
                self.log_line(format!("VeloForge {} is available: {}", r.version, r.url))
            }
            Status::UpToDate => self.log_line("VeloForge is up to date".into()),
            Status::Failed(e) => self.log_line(format!("Could not check for a new version: {e}")),
            Status::Checking => {}
        }
        self.update = Some(status);
    }

    fn pump_icu(&mut self) {
        use crate::icu::Reply;
        while let Ok(reply) = self.icu.rx.try_recv() {
            match reply {
                Reply::Connected(Ok(a)) => {
                    self.icu_busy = false;
                    self.icu_status = Some(Ok(format!("Connected as {}", a.name)));
                    if self.icu_creds != self.icu_edit {
                        self.icu_creds = self.icu_edit.clone();
                        if let Err(e) = self.icu_creds.save(&self.paths.credentials_file()) {
                            self.notify(format!("Could not save intervals.icu settings: {e}"));
                        }
                    }
                    self.log_line(format!("intervals.icu: connected as {}", a.name));
                    self.icu_athlete = Some(a);
                    self.icu_sync();
                }
                Reply::Connected(Err(e)) => {
                    self.icu_busy = false;
                    self.log_line(format!("intervals.icu: {e}"));
                    self.icu_status = Some(Err(e));
                }
                Reply::Synced {
                    oldest,
                    newest,
                    result,
                } => {
                    self.icu_busy = false;
                    match result {
                        Ok(events) => self.apply_icu_sync(oldest, newest, events),
                        Err(e) => {
                            self.log_line(format!("intervals.icu sync failed: {e}"));
                            self.icu_note = Some(format!("Sync failed: {e}"));
                        }
                    }
                }
                Reply::Uploaded { file, result } => match result {
                    Ok(id) => {
                        let name = self
                            .store
                            .history
                            .iter()
                            .find(|h| h.file == file)
                            .map(|h| h.name.clone())
                            .unwrap_or_default();
                        if let Some(h) = self.store.history.iter_mut().find(|h| h.file == file) {
                            h.icu_id = Some(id);
                        }
                        self.mark_dirty();
                        self.log_line(format!("Uploaded “{name}” to intervals.icu"));
                        self.notify("Ride uploaded to intervals.icu");
                    }
                    Err(e) => {
                        self.log_line(format!("intervals.icu upload failed: {e}"));
                        self.notify(format!("Upload to intervals.icu failed: {e}"));
                    }
                },
            }
        }
    }

    /// Mirror intervals.icu's planned rides into the calendar.
    fn apply_icu_sync(
        &mut self,
        oldest: NaiveDate,
        newest: NaiveDate,
        events: Vec<crate::icu::PlannedRide>,
    ) {
        let keep: std::collections::HashSet<String> = events
            .iter()
            .map(|(e, _)| format!("icu-{}", e.id))
            .collect();
        // Planned rides removed on intervals.icu disappear here too.
        self.store.plan.retain(|p| {
            !(p.workout_id.starts_with("icu-")
                && !p.done
                && p.date >= oldest
                && p.date <= newest
                && !keep.contains(&p.workout_id))
        });
        let mut unreadable = Vec::new();
        for (event, zwo) in &events {
            let id = format!("icu-{}", event.id);
            match zwo
                .clone()
                .and_then(|x| workout::parse_zwo(&x, &id).map_err(|e| e.to_string()))
            {
                Ok(mut w) => {
                    w.name = event.name.clone();
                    w.category = Category::Intervals;
                    self.store.custom_workouts.retain(|c| c.id != id);
                    self.store.custom_workouts.push(w);
                }
                Err(e) => {
                    if !self.store.custom_workouts.iter().any(|c| c.id == id) {
                        unreadable.push(format!("{} ({e})", event.name));
                        continue;
                    }
                }
            }
            match self.store.plan.iter_mut().find(|p| p.workout_id == id) {
                Some(p) => p.date = event.date,
                None => self.store.plan.push(PlannedWorkout {
                    date: event.date,
                    workout_id: id,
                    done: false,
                    skipped: false,
                }),
            }
        }
        // Forget downloaded workouts nothing refers to any more.
        let used: std::collections::HashSet<String> = self
            .store
            .plan
            .iter()
            .map(|p| p.workout_id.clone())
            .collect();
        self.store
            .custom_workouts
            .retain(|c| !c.id.starts_with("icu-") || used.contains(&c.id));
        self.store.plan.sort_by_key(|p| p.date);
        self.mark_dirty();
        let n = events.len() - unreadable.len();
        self.icu_note = Some(format!(
            "Synced at {} · {n} planned ride{}",
            Local::now().format("%H:%M"),
            if n == 1 { "" } else { "s" }
        ));
        self.log_line(format!("intervals.icu: {n} planned rides synced"));
        for u in unreadable {
            self.log_line(format!("intervals.icu: could not use planned ride {u}"));
        }
    }

    /// Move a deleted ride's file to the Trash (Recycle Bin): only files in the
    /// activities folder that no other ride in History still uses.
    fn trash_activity(&mut self, file: &str) {
        let path = Path::new(file);
        let ours = match (
            path.parent().and_then(|p| p.canonicalize().ok()),
            self.paths.activities.canonicalize().ok(),
        ) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        };
        let shared = self.store.history.iter().any(|h| h.file == file);
        if !ours || shared || !path.exists() {
            return;
        }
        match crate::activity::move_to_trash(path) {
            Ok(()) => self.log_line(format!("Moved {} to the {TRASH}", path.display())),
            Err(e) => {
                self.log_line(format!(
                    "Could not move {} to the {TRASH}: {e}",
                    path.display()
                ));
                self.notify(format!(
                    "The ride was removed, but its file could not be moved to the {TRASH}"
                ));
            }
        }
    }

    fn activity_path(&self, started: chrono::DateTime<Local>, title: &str) -> std::path::PathBuf {
        let slug: String = title
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        self.paths.activities.join(format!(
            "{}_{}.tcx",
            started.format("%Y-%m-%d_%H%M%S"),
            slug
        ))
    }

    /// Tick off the calendar entry a ride came from, or today's match.
    /// Matched by date and workout (not by position) so editing the calendar
    /// mid-ride cannot tick off the wrong entry.
    fn tick_off_plan(
        &mut self,
        workout_id: Option<&str>,
        plan_date: Option<NaiveDate>,
        ride_date: NaiveDate,
    ) {
        let Some(wid) = workout_id else { return };
        let find = |date: NaiveDate| {
            self.store
                .plan
                .iter()
                .position(|p| p.date == date && !p.done && p.workout_id == wid)
        };
        if let Some(i) = plan_date.and_then(find).or_else(|| find(ride_date)) {
            self.store.plan[i].done = true;
        }
    }

    fn checkpoint_paths(&self) -> (std::path::PathBuf, std::path::PathBuf) {
        (
            self.paths.activities.join("in-progress.tcx"),
            self.paths.activities.join("in-progress.json"),
        )
    }

    /// Save the ride so far, so a crash or power cut loses at most a minute.
    fn checkpoint(&mut self) {
        let (tcx, meta) = self.checkpoint_paths();
        let info = serde_json::json!({
            "name": self.ride.title(),
            "workout_id": self.ride.workout.as_ref().map(|w| w.id.clone()),
            "plan_date": self.ride.plan_date,
            "started": self.ride.started,
            "route_id": self.routes[self.route_idx].id,
        });
        let res = std::fs::create_dir_all(&self.paths.activities)
            .and_then(|_| std::fs::write(&tcx, self.ride.to_tcx()))
            .and_then(|_| std::fs::write(&meta, info.to_string()));
        if let Err(e) = res {
            self.log_line(format!("Could not save ride checkpoint: {e}"));
        }
        self.last_checkpoint = Instant::now();
    }

    fn clear_checkpoint(&self) {
        let (tcx, meta) = self.checkpoint_paths();
        let _ = std::fs::remove_file(tcx);
        let _ = std::fs::remove_file(meta);
    }

    /// Turn a ride left behind by a crash into a normal saved ride.
    fn recover_checkpoint(&mut self) {
        let (tcx, meta) = self.checkpoint_paths();
        let Ok(xml) = std::fs::read_to_string(&tcx) else {
            return;
        };
        let info: serde_json::Value = std::fs::read_to_string(&meta)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let activity = match crate::activity::parse_tcx(&xml) {
            Ok(a) if a.samples.len() >= 10 => a,
            _ => {
                self.clear_checkpoint();
                return;
            }
        };
        let started = info["started"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&Local))
            .unwrap_or_else(|| activity.samples[0].time.with_timezone(&Local));
        let name = info["name"]
            .as_str()
            .map(String::from)
            .or(activity.name)
            .unwrap_or_else(|| "Ride".into());
        let workout_id = info["workout_id"].as_str().map(String::from);
        let plan_date = info["plan_date"]
            .as_str()
            .and_then(|s| s.parse::<NaiveDate>().ok());
        let file = self.activity_path(started, &name);
        if std::fs::rename(&tcx, &file).is_err() {
            return;
        }
        let _ = std::fs::remove_file(meta);
        let meta = ride::SummaryMeta {
            start: started,
            name: name.clone(),
            workout_id: workout_id.clone(),
        };
        let s = &self.store.settings;
        let mut summary = ride::summarize(
            &activity.samples,
            meta,
            s.ftp,
            s.max_hr,
            file.display().to_string(),
        );
        summary.route_id = info["route_id"].as_str().map(String::from);
        self.tick_off_plan(workout_id.as_deref(), plan_date, started.date_naive());
        if self.store.settings.icu_upload {
            self.icu_upload(summary.file.clone(), summary.name.clone());
        }
        self.store.history.push(summary);
        self.store.history.sort_by_key(|h| h.start);
        self.save_now();
        self.notify(format!(
            "Recovered your unfinished ride “{name}” from {}",
            started.format("%a %d %b %H:%M")
        ));
    }

    fn import_file(&mut self, path: &Path, bytes: Option<&[u8]>) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let text = match bytes {
            Some(b) => String::from_utf8_lossy(b).into_owned(),
            None => match std::fs::read_to_string(path) {
                Ok(t) => t,
                Err(e) => return self.notify(format!("Cannot read {}: {e}", path.display())),
            },
        };
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("import")
            .to_string();
        let id = format!(
            "{}-{}",
            ext,
            stem.to_lowercase()
                .replace(|c: char| !c.is_ascii_alphanumeric(), "-")
        );
        match ext.as_str() {
            "zwo" => match workout::parse_zwo(&text, &id) {
                Ok(w) => {
                    let name = w.name.clone();
                    self.store.custom_workouts.retain(|x| x.id != id);
                    self.store.custom_workouts.push(w);
                    self.selected_workout = Some(id);
                    self.tab = Tab::Workouts;
                    self.mark_dirty();
                    self.notify(format!("Imported workout “{name}”"));
                }
                Err(e) => self.notify(format!("Could not import workout: {e:#}")),
            },
            "gpx" => match route::parse_gpx(&text, &id, &stem) {
                Ok(g) => {
                    let name = g.name.clone();
                    self.store.gpx_routes.retain(|x| x.id != id);
                    self.store.gpx_routes.push(g);
                    self.store.settings.route_id = id;
                    self.rebuild_routes();
                    self.mark_dirty();
                    self.notify(format!("Imported route “{name}”"));
                }
                Err(e) => self.notify(format!("Could not import route: {e:#}")),
            },
            _ => self.notify("Drop a .zwo workout or a .gpx route"),
        }
    }
}

/// Keep a log on disk for troubleshooting, rotated at 2 MB.
fn append_log_file(path: &Path, line: &str) {
    use std::io::Write;
    if std::fs::metadata(path).is_ok_and(|m| m.len() > 2_000_000) {
        let _ = std::fs::rename(path, path.with_extension("log.old"));
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{line}");
    }
}

/// egui's bundled fonts lack some symbols (arrows, check marks, dots); use
/// a common system font as a fallback when one is installed.
fn install_fallback_font(ctx: &egui::Context) {
    let add_last = |name: &str, data: egui::FontData| {
        ctx.add_font(egui::FontInsert::new(
            name,
            data,
            theme::families()
                .into_iter()
                .map(|family| egui::InsertFontFamily {
                    family,
                    priority: egui::FontPriority::Lowest,
                })
                .collect(),
        ));
    };
    if let Some(bytes) = crate::platform::fallback_fonts()
        .into_iter()
        .find_map(|path| std::fs::read(path).ok())
    {
        add_last("system-fallback", egui::FontData::from_owned(bytes));
    }
    // Last resort, already built in: egui's Hack font has the few symbols
    // (such as the menu arrow ▾) that a system may lack.
    add_last(
        "bundled-symbols",
        egui::FontData::from_static(epaint_default_fonts::HACK_REGULAR),
    );
}

// ----------------------------------------------------------------------
// Small widgets

/// Icons are painted as shapes so they never depend on installed fonts.
#[derive(Clone, Copy)]
enum Icon {
    Left,
    Right,
    Up,
    Play,
    Pause,
    Close,
    Check,
}

fn paint_icon(painter: &egui::Painter, r: Rect, icon: Icon, c: Color32) {
    let (x0, x1, y0, y1, cx, cy) = (
        r.left(),
        r.right(),
        r.top(),
        r.bottom(),
        r.center().x,
        r.center().y,
    );
    let tri = |pts: Vec<Pos2>| egui::Shape::convex_polygon(pts, c, Stroke::NONE);
    let w = (r.width() * 0.18).max(1.5);
    match icon {
        Icon::Left => {
            painter.add(tri(vec![
                Pos2::new(x1 - 2.0, y0 + 1.0),
                Pos2::new(x1 - 2.0, y1 - 1.0),
                Pos2::new(x0 + 2.0, cy),
            ]));
        }
        Icon::Right | Icon::Play => {
            painter.add(tri(vec![
                Pos2::new(x0 + 2.0, y0 + 1.0),
                Pos2::new(x1 - 2.0, cy),
                Pos2::new(x0 + 2.0, y1 - 1.0),
            ]));
        }
        Icon::Up => {
            painter.add(tri(vec![
                Pos2::new(x0 + 1.0, y1 - 2.0),
                Pos2::new(cx, y0 + 2.0),
                Pos2::new(x1 - 1.0, y1 - 2.0),
            ]));
        }
        Icon::Pause => {
            painter.rect_filled(
                Rect::from_min_max(Pos2::new(x0 + 2.0, y0 + 1.0), Pos2::new(cx - 1.5, y1 - 1.0)),
                1.0,
                c,
            );
            painter.rect_filled(
                Rect::from_min_max(Pos2::new(cx + 1.5, y0 + 1.0), Pos2::new(x1 - 2.0, y1 - 1.0)),
                1.0,
                c,
            );
        }
        Icon::Close => {
            let s = Stroke::new(w, c);
            painter.line_segment(
                [Pos2::new(x0 + 2.0, y0 + 2.0), Pos2::new(x1 - 2.0, y1 - 2.0)],
                s,
            );
            painter.line_segment(
                [Pos2::new(x1 - 2.0, y0 + 2.0), Pos2::new(x0 + 2.0, y1 - 2.0)],
                s,
            );
        }
        Icon::Check => {
            let pts = vec![
                Pos2::new(x0 + 1.0, cy),
                Pos2::new(cx - 1.0, y1 - 2.0),
                Pos2::new(x1 - 1.0, y0 + 2.0),
            ];
            painter.add(egui::Shape::line(pts, Stroke::new(w, c)));
        }
    }
}

/// A button with a painted icon and optional text and fill colour.
fn icon_button(ui: &mut egui::Ui, icon: Icon, text: &str, fill: Option<Color32>) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, Color32::WHITE);
    let pad = ui.spacing().button_padding;
    let icon_w = galley.size().y.max(12.0) * 0.8;
    let gap = if text.is_empty() { 0.0 } else { 6.0 };
    let size = Vec2::new(
        pad.x * 2.0 + icon_w + gap + galley.size().x,
        (pad.y * 2.0 + galley.size().y).max(ui.spacing().interact_size.y),
    );
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let vis = ui.style().interact(&resp);
        let bg = match fill {
            Some(f) if resp.hovered() => f.gamma_multiply(1.2),
            Some(f) => f,
            None => vis.weak_bg_fill,
        };
        ui.painter().rect(
            rect,
            vis.corner_radius,
            bg,
            vis.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let c = if fill.is_some() {
            Color32::WHITE
        } else {
            vis.text_color()
        };
        let ir = Rect::from_center_size(
            Pos2::new(rect.left() + pad.x + icon_w / 2.0, rect.center().y),
            Vec2::splat(icon_w),
        );
        paint_icon(ui.painter(), ir, icon, c);
        ui.painter().galley_with_override_text_color(
            Pos2::new(ir.right() + gap, rect.center().y - galley.size().y / 2.0),
            galley,
            c,
        );
    }
    resp
}

/// A tab in the top bar: plain text, with an accent bar under the open one.
fn nav_tab(ui: &mut egui::Ui, selected: bool, text: &str, compact: bool) -> egui::Response {
    let font = if selected {
        theme::semibold(14.0)
    } else {
        theme::medium(14.0)
    };
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, Color32::WHITE);
    let pad = if compact { 14.0 } else { 24.0 };
    let size = galley.size() + Vec2::new(pad, 12.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let color = if selected {
            Color32::WHITE
        } else if resp.hovered() {
            theme::TEXT
        } else {
            theme::TEXT_DIM
        };
        if resp.hovered() && !selected {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(8), Color32::from_white_alpha(10));
        }
        ui.painter().galley_with_override_text_color(
            rect.center() - galley.size() / 2.0,
            galley,
            color,
        );
        if selected {
            let bar = Rect::from_center_size(
                Pos2::new(rect.center().x, rect.bottom() + 6.5),
                Vec2::new(rect.width() - pad + 4.0, 3.0),
            );
            ui.painter()
                .with_clip_rect(ui.ctx().content_rect())
                .rect_filled(bar, CornerRadius::same(2), ACCENT);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A row in a list that fills the width: a stripe of colour, a name, and
/// details set quieter on the right.
fn list_row(
    ui: &mut egui::Ui,
    selected: bool,
    name: &str,
    meta: &str,
    colour: Color32,
) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if selected {
            p.rect_filled(rect, CornerRadius::same(8), theme::CARD_HI);
        } else if resp.hovered() {
            p.rect_filled(rect, CornerRadius::same(8), Color32::from_white_alpha(9));
        }
        let stripe = Rect::from_center_size(
            Pos2::new(rect.left() + 7.0, rect.center().y),
            Vec2::new(3.0, if selected { 18.0 } else { 12.0 }),
        );
        p.rect_filled(
            stripe,
            CornerRadius::same(2),
            if selected {
                colour
            } else {
                colour.gamma_multiply(0.6)
            },
        );
        let m = p.text(
            Pos2::new(rect.right() - 10.0, rect.center().y),
            Align2::RIGHT_CENTER,
            meta,
            theme::regular(12.0),
            if selected {
                theme::TEXT
            } else {
                theme::TEXT_FAINT
            },
        );
        // The name gives way to the details when the list is narrow.
        p.with_clip_rect(Rect::from_min_max(
            rect.min,
            Pos2::new(m.left() - 8.0, rect.bottom()),
        ))
        .text(
            Pos2::new(rect.left() + 18.0, rect.center().y),
            Align2::LEFT_CENTER,
            name,
            if selected {
                theme::medium(13.5)
            } else {
                theme::regular(13.5)
            },
            if selected {
                Color32::WHITE
            } else {
                theme::TEXT
            },
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A row of choices in one rounded well, the chosen one raised. Returns
/// whether the choice changed. Each option: value, label, hover text.
fn segmented<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    value: &mut T,
    options: &[(T, &str, &str)],
) -> bool {
    let font = theme::medium(13.0);
    let galleys: Vec<_> = options
        .iter()
        .map(|o| {
            ui.painter()
                .layout_no_wrap(o.1.to_string(), font.clone(), Color32::WHITE)
        })
        .collect();
    let h = ui.spacing().interact_size.y;
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 22.0).collect();
    let total: f32 = widths.iter().sum::<f32>() + 6.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total, h), Sense::hover());
    let enabled = ui.is_enabled();
    ui.painter()
        .rect_filled(rect, CornerRadius::same(9), Color32::from_rgb(22, 25, 32));
    let mut changed = false;
    let mut x = rect.left() + 3.0;
    for (i, (o, galley)) in options.iter().zip(galleys).enumerate() {
        let r = Rect::from_min_size(
            Pos2::new(x, rect.top() + 3.0),
            Vec2::new(widths[i], h - 6.0),
        );
        x += widths[i];
        let resp = ui
            .interact(r, ui.scope_id().with(("segment", o.1)), Sense::click())
            .on_hover_text(o.2);
        let selected = *value == o.0;
        if resp.clicked() && !selected {
            *value = o.0;
            changed = true;
        }
        let color = if !enabled {
            if selected {
                theme::TEXT_DIM
            } else {
                Color32::from_gray(78)
            }
        } else if selected {
            Color32::WHITE
        } else if resp.hovered() {
            theme::TEXT
        } else {
            theme::TEXT_DIM
        };
        if selected {
            ui.painter().rect_filled(
                r,
                CornerRadius::same(7),
                if enabled {
                    Color32::from_rgb(58, 64, 82)
                } else {
                    Color32::from_rgb(38, 42, 54)
                },
            );
        }
        ui.painter().galley_with_override_text_color(
            r.center() - galley.size() / 2.0,
            galley,
            color,
        );
    }
    changed
}

/// Paint `text` as a QR code with `px`-sized modules on a white card.
fn qr_code(ui: &mut egui::Ui, text: &str, px: f32) {
    let Ok(code) = qrcode::QrCode::new(text.as_bytes()) else {
        return;
    };
    let n = code.width();
    let quiet = 3;
    let size = (n + 2 * quiet) as f32 * px;
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 8.0, Color32::WHITE);
    for (i, c) in code.to_colors().iter().enumerate() {
        if *c == qrcode::Color::Dark {
            let (x, y) = ((i % n + quiet) as f32, (i / n + quiet) as f32);
            painter.rect_filled(
                Rect::from_min_size(rect.min + Vec2::new(x * px, y * px), Vec2::splat(px)),
                0.0,
                Color32::BLACK,
            );
        }
    }
}

/// A gap in metres as ("35", "m ahead").
fn gap_text(gap: f64) -> (String, String) {
    let side = if gap >= 0.0 { "ahead" } else { "behind" };
    if gap.abs() >= 1000.0 {
        (format!("{:.2}", gap.abs() / 1000.0), format!("km {side}"))
    } else {
        (format!("{:.0}", gap.abs()), format!("m {side}"))
    }
}

/// Long durations as "12 h 53 min".
fn fmt_hours(secs: f32) -> String {
    let m = (secs / 60.0).round() as u32;
    if m < 60 {
        format!("{m} min")
    } else {
        format!("{} h {:02} min", m / 60, m % 60)
    }
}

fn date_picker(ui: &mut egui::Ui, date: &mut NaiveDate) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        if icon_button(ui, Icon::Left, "", None)
            .on_hover_text("Previous day")
            .clicked()
        {
            *date -= Duration::days(1);
            changed = true;
        }
        ui.label(RichText::new(date.format("%a %d %b %Y").to_string()).font(theme::medium(13.0)));
        if icon_button(ui, Icon::Right, "", None)
            .on_hover_text("Next day")
            .clicked()
        {
            *date += Duration::days(1);
            changed = true;
        }
        if ui.small_button("Today").clicked() {
            *date = Local::now().date_naive();
            changed = true;
        }
    });
    changed
}

fn secs_edit(ui: &mut egui::Ui, secs: &mut u32) -> bool {
    ui.add(
        egui::DragValue::new(secs)
            .range(5..=4 * 3600)
            .speed(5.0)
            .custom_formatter(|v, _| fmt_dur(v as f32))
            .custom_parser(|s| {
                let parts: Vec<&str> = s.trim().split(':').collect();
                match parts.as_slice() {
                    [m, s] => Some(m.parse::<f64>().ok()? * 60.0 + s.parse::<f64>().ok()?),
                    [v] => {
                        let v = v.trim_end_matches('s');
                        if let Some(m) = v.strip_suffix('m') {
                            Some(m.parse::<f64>().ok()? * 60.0)
                        } else {
                            v.parse().ok()
                        }
                    }
                    _ => None,
                }
            }),
    )
    .changed()
}

fn pct_edit(ui: &mut egui::Ui, frac: &mut f32) -> bool {
    let mut pct = *frac * 100.0;
    let changed = ui
        .add(
            egui::DragValue::new(&mut pct)
                .range(0.0..=300.0)
                .speed(1.0)
                .suffix("%")
                .max_decimals(0),
        )
        .changed();
    if changed {
        *frac = pct / 100.0;
    }
    changed
}

/// A step's cadence target in rpm; 0 ("any rpm") means none.
fn cadence_edit(ui: &mut egui::Ui, cadence: &mut Option<u32>) -> bool {
    let mut rpm = cadence.unwrap_or(0);
    let changed = ui
        .add(
            egui::DragValue::new(&mut rpm)
                .range(0..=200)
                .speed(1.0)
                .custom_formatter(|v, _| {
                    if v < 1.0 {
                        "any rpm".into()
                    } else {
                        format!("{v:.0} rpm")
                    }
                })
                .custom_parser(|s| {
                    let s = s.trim().trim_end_matches("rpm").trim();
                    if s.is_empty() || s == "-" || s.eq_ignore_ascii_case("any") {
                        Some(0.0)
                    } else {
                        s.parse().ok()
                    }
                }),
        )
        .on_hover_text("Target cadence for this step (0 = no target)")
        .changed();
    if changed {
        *cadence = (rpm > 0).then_some(rpm);
    }
    changed
}

/// A dashboard tile: caption, a big number with its unit, and a stripe of
/// colour down the left edge.
fn tile(
    painter: &egui::Painter,
    rect: Rect,
    label: &str,
    value: &str,
    unit: &str,
    accent: Color32,
) {
    theme::card(painter, rect, 12.0);
    painter.rect_filled(
        Rect::from_min_max(
            rect.left_top() + Vec2::new(0.0, 12.0),
            rect.left_bottom() + Vec2::new(3.0, -12.0),
        ),
        CornerRadius::same(2),
        accent,
    );
    theme::caption(
        painter,
        rect.left_top() + Vec2::new(16.0, 10.0),
        Align2::LEFT_TOP,
        label,
        theme::TEXT_DIM,
    );
    theme::value(
        painter,
        rect.left_bottom() + Vec2::new(14.0, -4.0),
        value,
        unit,
        (rect.height() * 0.52).min(rect.width() * 0.34),
        Color32::WHITE,
    );
}

/// Devices worth listing by default: anything advertising a fitness service,
/// or with a name from a known fitness brand.
fn is_fitness_device(d: &DeviceInfo) -> bool {
    const NAMES: [&str; 24] = [
        "kickr", "wahoo", "tickr", "rpm", "hrm", "polar", "garmin", "coospo", "magene", "tacx",
        "neo", "flux", "elite", "saris", "wattbike", "stages", "assioma", "favero", "4iiii",
        "rally", "vector", "quarq", "zwift", "heart",
    ];
    let n = d.name.to_lowercase();
    !d.caps.is_empty() || NAMES.iter().any(|k| n.contains(k))
}

const POWER_ZONE_NAMES: [&str; 6] = [
    "Z1 Recovery",
    "Z2 Endurance",
    "Z3 Tempo",
    "Z4 Threshold",
    "Z5 VO2 max",
    "Z6 Anaerobic",
];

fn power_zone_color(z: usize) -> Color32 {
    charts::zone_color([0.5, 0.65, 0.85, 1.0, 1.15, 1.4][z.min(5)])
}

/// Time in power and heart rate zones as labelled bars.
fn zone_breakdown(ui: &mut egui::Ui, s: &Summary) {
    let rows = |ui: &mut egui::Ui,
                title: &str,
                secs: &[f32],
                names: &[&str],
                color: &dyn Fn(usize) -> Color32| {
        let total: f32 = secs.iter().sum();
        if total <= 0.0 {
            return;
        }
        ui.add_space(6.0);
        ui.label(RichText::new(title).strong());
        for (i, sec) in secs.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_sized([100.0, 16.0], egui::Label::new(names[i]));
                let (r, _) = ui.allocate_exact_size(Vec2::new(160.0, 12.0), Sense::hover());
                ui.painter()
                    .rect_filled(r, 2.0, Color32::from_white_alpha(20));
                ui.painter().rect_filled(
                    Rect::from_min_size(r.min, Vec2::new(r.width() * sec / total, r.height())),
                    2.0,
                    color(i),
                );
                ui.label(format!("{}  {:.0}%", fmt_dur(*sec), sec / total * 100.0));
            });
        }
    };
    rows(
        ui,
        "Time in power zones",
        &s.power_zones,
        &POWER_ZONE_NAMES,
        &power_zone_color,
    );
    let hr_names: Vec<&str> = charts::HR_ZONES.iter().map(|z| z.0).collect();
    rows(
        ui,
        "Time in heart rate zones",
        &s.hr_zones,
        &hr_names,
        &charts::hr_zone_color,
    );
}

/// A saved ride opened from History.
struct RideDetails {
    summary: Summary,
    activity: Result<crate::activity::Activity, String>,
}

// ----------------------------------------------------------------------

impl eframe::App for App {
    /// Ride timing, trainer control and saving live here rather than in `ui`
    /// because eframe keeps calling `logic` while the window is minimised.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump_events();
        self.tick_ride();
        self.pump_icu();
        self.pump_update();
        if self.icu_creds.is_set()
            && self.store.settings.icu_sync_plan
            && !self.icu_busy
            && self.icu_athlete.is_some()
            && self
                .icu_last_sync
                .is_none_or(|t| t.elapsed().as_secs() > 30 * 60)
        {
            self.icu_sync();
        }
        if self.ride_active()
            && self.ride.samples.len() >= 10
            && self.last_checkpoint.elapsed().as_secs() >= 60
        {
            self.checkpoint();
        }
        if self.dirty && self.last_save.elapsed().as_secs_f32() > 2.0 {
            self.save_now();
        }
        self.pump_remote(ctx);
        if self.ride.state == ride::State::Running {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Drag & drop import.
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            let path = f.path().to_path_buf();
            match f.bytes() {
                Ok(bytes) => self.import_file(&path, Some(&bytes)),
                Err(_) => self.import_file(&path, None),
            }
        }

        // Keyboard shortcuts while riding.
        if self.tab == Tab::Ride && !ctx.egui_wants_keyboard_input() {
            // Consume the keys so a focused button is not activated as well.
            let (space, up, down, v, n) = ctx.input_mut(|i| {
                let none = egui::Modifiers::NONE;
                (
                    i.consume_key(none, egui::Key::Space),
                    i.consume_key(none, egui::Key::ArrowUp),
                    i.consume_key(none, egui::Key::ArrowDown),
                    i.consume_key(none, egui::Key::V),
                    i.consume_key(none, egui::Key::N),
                )
            });
            use crate::remote::Action;
            for (pressed, a) in [
                (n, Action::Skip),
                (v, Action::View),
                (space, Action::Toggle),
                (up, Action::Up),
                (down, Action::Down),
            ] {
                if pressed {
                    self.ride_action(a);
                }
            }
        }

        egui::Panel::top("tabs")
            .frame(
                egui::Frame::new()
                    .fill(theme::BAR)
                    .inner_margin(egui::Margin::symmetric(10, 8)),
            )
            .show(ui, |ui| self.top_bar(ui));
        egui::CentralPanel::default_margins().show(ui, |ui| match self.tab {
            Tab::Ride => self.ride_tab(ui),
            Tab::Workouts => self.workouts_tab(ui),
            Tab::Plan => self.plan_tab(ui),
            Tab::History => self.history_tab(ui),
            Tab::Progress => {
                egui::ScrollArea::vertical()
                    .id_salt("progress-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.progress_tab(ui));
            }
            Tab::Devices => self.devices_tab(ui),
            Tab::Settings => self.settings_tab(ui),
        });
        self.modals(&ctx);

        if let Some((msg, at)) = &self.toast {
            if at.elapsed().as_secs_f32() < 4.0 {
                egui::Area::new(egui::Id::unique("toast"))
                    .anchor(Align2::CENTER_BOTTOM, [0.0, -24.0])
                    .show(&ctx, |ui| {
                        egui::Frame::popup(ui.style())
                            .show(ui, |ui| ui.label(RichText::new(msg).size(15.0)));
                    });
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            } else {
                self.toast = None;
            }
        }

        if self.tab == Tab::Ride || self.ride.state == ride::State::Running {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Never lose a ride by closing the window.
        if self.ride_active() && self.ride.samples.len() >= 10 {
            self.finish_ride();
        }
        self.dev.send(Cmd::Control(devices::Control::Grade(0.0)));
        if self.dirty {
            let _ = self.store.save(&self.paths.state_file());
        }
    }
}

impl App {
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        // On a narrow window the name and the chips' details make way.
        let compact = ui.available_width() < 1120.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = if compact { 0.0 } else { 4.0 };
            ui.add_space(4.0);
            ui.add(egui::Image::new(egui::load::SizedTexture::new(
                self.logo.id(),
                Vec2::splat(24.0),
            )));
            ui.add_space(4.0);
            if !compact {
                ui.label(
                    RichText::new("VeloForge")
                        .font(theme::semibold(17.0))
                        .color(Color32::from_rgb(244, 240, 234)),
                );
                ui.add_space(18.0);
            }
            for (tab, label) in [
                (Tab::Ride, "Ride"),
                (Tab::Workouts, "Workouts"),
                (Tab::Plan, "Calendar & Plan"),
                (Tab::History, "History"),
                (Tab::Progress, "Progress"),
                (Tab::Devices, "Devices"),
                (Tab::Settings, "Settings"),
            ] {
                if nav_tab(ui, self.tab == tab, label, compact).clicked() {
                    self.tab = tab;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                // Status chips (right to left).
                let chip = |ui: &mut egui::Ui, ok: bool, label: &str| {
                    let c = if ok {
                        theme::GOOD
                    } else {
                        Color32::from_gray(90)
                    };
                    egui::Frame::new()
                        .fill(if ok {
                            Color32::from_rgb(24, 36, 32)
                        } else {
                            theme::CARD
                        })
                        .corner_radius(12)
                        .inner_margin(egui::Margin::symmetric(10, 4))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                let (r, _) =
                                    ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                                if ok {
                                    ui.painter().circle_filled(
                                        r.center(),
                                        6.0,
                                        c.gamma_multiply(0.25),
                                    );
                                }
                                ui.painter().circle_filled(r.center(), 3.5, c);
                                ui.label(RichText::new(label).font(theme::medium(12.0)).color(
                                    if ok {
                                        Color32::from_rgb(214, 232, 222)
                                    } else {
                                        theme::TEXT_FAINT
                                    },
                                ));
                            });
                        });
                };
                chip(ui, fresh(&self.metrics.hr).is_some(), "HR");
                if self.remote.as_ref().is_some_and(|r| r.phone_connected()) {
                    chip(ui, true, "Phone");
                }
                chip(ui, fresh(&self.metrics.power).is_some(), "Power");
                let trainer = self.trainer_link().map(|l| l.name.clone());
                chip(
                    ui,
                    trainer.is_some(),
                    match &trainer {
                        Some(_) if compact => "Trainer",
                        Some(name) => name,
                        None => "No trainer",
                    },
                );
                if self.ride.state != ride::State::Running {
                    if let Some(crate::update::Status::Available(r)) = &self.update {
                        let label = if compact {
                            "Update".to_string()
                        } else {
                            format!("Update {}", r.version)
                        };
                        let button = egui::Button::new(
                            RichText::new(label)
                                .font(theme::medium(12.0))
                                .color(Color32::WHITE),
                        )
                        .fill(ACCENT)
                        .corner_radius(12);
                        if ui
                            .add(button)
                            .on_hover_text(format!(
                                "VeloForge {} is available. Click to open its download page.",
                                r.version
                            ))
                            .clicked()
                        {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(&r.url));
                        }
                    }
                }
                if self.ride.state == ride::State::Running {
                    // A recording light that breathes.
                    let t = ui.input(|i| i.time);
                    let pulse = 0.55 + 0.45 * (t * 2.6).sin() as f32;
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(fmt_dur(self.ride.elapsed as f32))
                            .font(theme::medium(12.5))
                            .color(Color32::from_rgb(240, 120, 110)),
                    );
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                    ui.painter()
                        .circle_filled(r.center(), 4.0, theme::BAD.gamma_multiply(pulse));
                }
            });
        });
    }

    // ------------------------------------------------------------------
    // Ride

    fn workout_category(&self, id: &str) -> Option<Category> {
        self.find_workout(id).map(|w| w.category)
    }

    /// "You missed N workouts" with options to reschedule or skip them.
    fn missed_banner(&mut self, ui: &mut egui::Ui) {
        let today = Local::now().date_naive();
        let lib = self.library();
        let missed = plan::missed(&self.store.plan, today, |id| {
            lib.iter().find(|w| w.id == id).map(|w| w.category)
        });
        if missed.is_empty() {
            return;
        }
        let name = |i: usize| {
            let p = &self.store.plan[i];
            let n = lib
                .iter()
                .find(|w| w.id == p.workout_id)
                .map(|w| w.name.clone())
                .unwrap_or_default();
            format!("{n} ({})", p.date.format("%a"))
        };
        let list: Vec<String> = missed.iter().map(|m| name(m.index)).collect();
        let moves: Vec<String> = missed
            .iter()
            .filter_map(|m| {
                m.move_to
                    .map(|d| format!("{} → {}", name(m.index), d.format("%a %d %b")))
            })
            .collect();
        let mut action = None;
        egui::Frame::new()
            .fill(Color32::from_rgb(52, 38, 30))
            .corner_radius(8)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(format!("You missed {}: {}", if list.len() == 1 { "a planned workout".into() } else { format!("{} planned workouts", list.len()) }, list.join(", "))).strong());
                if !moves.is_empty() {
                    ui.label(RichText::new(format!("Rescheduling moves the hard sessions to free days ({}) and skips the easy rides.", moves.join(", "))).weak());
                }
                ui.horizontal(|ui| {
                    if !moves.is_empty() && ui.add(egui::Button::new("Reschedule").fill(ACCENT)).clicked() {
                        action = Some(true);
                    }
                    if ui.button(if moves.is_empty() { "Mark as skipped" } else { "Skip them all" }).clicked() {
                        action = Some(false);
                    }
                });
            });
        ui.add_space(6.0);
        if let Some(reschedule) = action {
            for m in missed {
                match (reschedule, m.move_to) {
                    (true, Some(d)) => self.store.plan[m.index].date = d,
                    _ => self.store.plan[m.index].skipped = true,
                }
            }
            self.store.plan.sort_by_key(|p| p.date);
            self.mark_dirty();
        }
    }

    /// Suggest an easier ride when today's planned workout is hard and form
    /// says you are very tired.
    fn tired_banner(&mut self, ui: &mut egui::Ui, floating: bool) {
        let today = Local::now().date_naive();
        if self.tired_dismissed == Some(today) {
            return;
        }
        let Some(i) = self
            .store
            .plan
            .iter()
            .position(|p| p.date == today && !p.done && !p.skipped)
        else {
            return;
        };
        let id = self.store.plan[i].workout_id.clone();
        let Some(cat) = self.workout_category(&id) else {
            return;
        };
        let (_, _, form) = crate::progress::fitness(&self.store.history, today, 1).today();
        let Some(easier) = plan::easier_option(cat, form) else {
            return;
        };
        let (Some(hard), Some(easy)) = (self.find_workout(&id), self.find_workout(easier)) else {
            return;
        };
        let mut action = None;
        let content = |ui: &mut egui::Ui, action: &mut Option<bool>| {
            ui.label(
                RichText::new(format!("You look very tired today (form {form:+.0}).")).strong(),
            );
            ui.label(format!(
                "Consider swapping {} for {} and saving the hard session for when you're fresher.",
                hard.name, easy.name
            ));
            ui.horizontal(|ui| {
                if ui
                    .add(egui::Button::new(format!("Ride {} instead", easy.name)).fill(ACCENT))
                    .clicked()
                {
                    *action = Some(true);
                }
                if ui.button("Keep the plan").clicked() {
                    *action = Some(false);
                }
            });
        };
        if floating {
            egui::Area::new(egui::Id::unique("tired"))
                .anchor(Align2::CENTER_TOP, [0.0, 235.0])
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style())
                        .inner_margin(12)
                        .show(ui, |ui| content(ui, &mut action));
                });
        } else {
            egui::Frame::new()
                .fill(Color32::from_rgb(40, 38, 54))
                .corner_radius(8)
                .inner_margin(10)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    content(ui, &mut action);
                });
            ui.add_space(6.0);
        }
        match action {
            Some(true) => {
                self.store.plan[i].workout_id = easier.to_string();
                self.mark_dirty();
                if !self.ride_active() {
                    self.load_ride(Some(easier.to_string()), Some(today));
                }
                self.notify(format!("Swapped for {}", easy.name));
            }
            Some(false) => self.tired_dismissed = Some(today),
            None => {}
        }
    }

    fn ride_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let mut v = self.store.settings.view_3d;
            if segmented(
                ui,
                &mut v,
                &[
                    (true, "3D", "The 3D world (V switches)"),
                    (false, "Dashboard", "Numbers and charts (V switches)"),
                ],
            ) {
                self.store.settings.view_3d = v;
                self.mark_dirty();
            }
            if self.store.settings.view_3d {
                self.sky_menu(ui);
            }
            // Once you are riding, what and where is settled: the choices
            // make way for the controls you need on the bike.
            let active = self.ride_active();
            if !active {
                let mut mode = self.ride.mode;
                if segmented(
                    ui,
                    &mut mode,
                    &[
                        (
                            Mode::Workout,
                            "Workout",
                            "Structured workout: the trainer holds the target power (ERG)",
                        ),
                        (
                            Mode::Route,
                            "Free ride",
                            "Resistance follows the road's gradient",
                        ),
                        (Mode::Slope, "Slope", "Choose a fixed gradient yourself"),
                        (
                            Mode::HeartRate,
                            "Heart rate",
                            "Hold a target heart rate: VeloForge adjusts the trainer's power for you",
                        ),
                    ],
                ) {
                    let w = if mode == Mode::Workout {
                        self.ride_workout
                            .clone()
                            .or(Some("endurance-60".into()))
                            .and_then(|id| self.find_workout(&id))
                    } else {
                        None
                    };
                    self.ride_workout = w
                        .as_ref()
                        .map(|w| w.id.clone())
                        .or(self.ride_workout.clone());
                    self.replace_ride(Ride::new(mode, w, None));
                    if mode == Mode::HeartRate {
                        // Default to the middle of zone 2.
                        self.ride.hr_control =
                            ride::HrControl::new((self.store.settings.max_hr * 0.65).round());
                    }
                }
                if self.ride.mode == Mode::Workout {
                    let current = self
                        .ride
                        .workout
                        .as_ref()
                        .map(|w| w.name.clone())
                        .unwrap_or("Choose workout".into());
                    let mut pick = None;
                    egui::ComboBox::from_id_salt("ride-workout")
                        .width(150.0)
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for w in self.library() {
                                let label = format!("{}  ·  {}", w.name, fmt_dur(w.duration()));
                                if ui
                                    .selectable_label(
                                        self.ride_workout.as_deref() == Some(&w.id),
                                        label,
                                    )
                                    .clicked()
                                {
                                    pick = Some(w.id.clone());
                                }
                            }
                        });
                    if let Some(id) = pick {
                        self.load_ride(Some(id), None);
                    }
                }
                let mut ri = self.route_idx;
                egui::ComboBox::from_id_salt("ride-route")
                    .width(160.0)
                    .selected_text(&self.routes[ri].name)
                    .show_ui(ui, |ui| {
                        for (i, r) in self.routes.iter().enumerate() {
                            ui.selectable_value(&mut ri, i, &r.name);
                        }
                    });
                if ri != self.route_idx {
                    self.route_idx = ri;
                    self.store.settings.route_id = self.routes[ri].id.clone();
                    self.mark_dirty();
                    // A ghost belongs to the route it was ridden on.
                    if matches!(self.ride.companion, Some(ride::Companion::Ghost(_))) {
                        self.ride.companion = None;
                    }
                }
            }
            self.companion_menu(ui);
            match self.ride.state {
                ride::State::Ready => {
                    let can = self.can_start();
                    let resp = ui
                        .add_enabled_ui(can, |ui| {
                            icon_button(
                                ui,
                                Icon::Play,
                                "Start ride",
                                Some(Color32::from_rgb(36, 150, 84)),
                            )
                        })
                        .inner;
                    if resp.on_hover_text("Space").clicked() {
                        self.ride.start();
                    }
                }
                ride::State::Running | ride::State::Paused => {
                    let running = self.ride.state == ride::State::Running;
                    let (icon, label) = if running {
                        (Icon::Pause, "Pause")
                    } else {
                        (Icon::Play, "Resume")
                    };
                    if icon_button(ui, icon, label, None)
                        .on_hover_text("Space")
                        .clicked()
                    {
                        if running {
                            self.ride.state = ride::State::Paused;
                        } else {
                            self.ride.start();
                        }
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Finish").color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(44, 112, 196)),
                        )
                        .clicked()
                    {
                        self.finish_ride();
                    }
                    if ui.button("Discard").clicked() {
                        self.modal = Some(Modal::ConfirmDiscard);
                    }
                }
            }
            match self.ride.mode {
                Mode::Workout => {
                    if self.ride_active()
                        && !self.ride.workout_done()
                        && icon_button(ui, Icon::Right, "Skip step", None)
                            .on_hover_text("Jump to the next step (N)")
                            .clicked()
                    {
                        self.ride.skip_step();
                    }
                    if ui
                        .small_button(" - ")
                        .on_hover_text("Easier (Down arrow key)")
                        .clicked()
                    {
                        self.ride.intensity = (self.ride.intensity - 0.05).max(0.5);
                    }
                    ui.label(format!("{:.0}%", self.ride.intensity * 100.0))
                        .on_hover_text("Workout intensity: scales every target");
                    if ui
                        .small_button(" + ")
                        .on_hover_text("Harder (Up arrow key)")
                        .clicked()
                    {
                        self.ride.intensity = (self.ride.intensity + 0.05).min(1.5);
                    }
                }
                Mode::Slope => {
                    ui.label(RichText::new("Slope").color(theme::TEXT_DIM));
                    ui.add(
                        egui::Slider::new(&mut self.ride.manual_grade, -5.0..=15.0)
                            .step_by(0.5)
                            .suffix("%"),
                    );
                }
                Mode::HeartRate => {
                    ui.label(RichText::new("Target").color(theme::TEXT_DIM));
                    let max_hr = self.store.settings.max_hr;
                    ui.add(
                        egui::DragValue::new(&mut self.ride.hr_control.target_bpm)
                            .range(80.0..=200.0)
                            .suffix(" bpm"),
                    )
                    .on_hover_text("Up/Down arrow keys change it by 2 bpm");
                    let z = charts::hr_zone(self.ride.hr_control.target_bpm, max_hr);
                    ui.label(RichText::new(charts::HR_ZONES[z].0).color(charts::hr_zone_color(z)));
                }
                Mode::Route => {
                    ui.label(RichText::new("Difficulty").color(theme::TEXT_DIM));
                    if ui
                        .add(
                            egui::Slider::new(&mut self.store.settings.difficulty, 0.0..=1.0)
                                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                        )
                        .on_hover_text(
                            "How much of the road's gradient the trainer gives you: 100% feels \
                             like the real hill, 50% like half of it, 0% like flat road. It only \
                             changes anything on climbs and descents. Up/Down arrow keys change it.",
                        )
                        .changed()
                    {
                        self.mark_dirty();
                    }
                }
            }
            ui.label(RichText::new("Keys").color(theme::TEXT_FAINT).underline())
                .on_hover_text(
                    "Space: start / pause / resume\n\
                     Up / Down: workout intensity, slope, or trainer difficulty\n\
                     N: skip to the next workout step\n\
                     V: switch between 3D view and dashboard",
                );
        });
        ui.add_space(2.0);

        // Never draw past the window edge, even if a control row overflowed.
        let avail = ui.available_rect_before_wrap().intersect(ui.clip_rect());
        ui.allocate_rect(avail, Sense::hover());
        let painter = ui.painter_at(avail);
        let dt = ui.ctx().input(|i| i.stable_dt as f64).min(0.1);
        // Without a trainer, a notice on the view leads to Devices.
        let view_3d = self.store.settings.view_3d;
        let chart_h = 104.0;
        let board = Rect::from_min_max(
            avail.min,
            Pos2::new(avail.max.x, avail.max.y - chart_h - 8.0),
        );
        if self.trainer_link().is_none() {
            let banner = ui.interact(
                Self::banner_rect(if view_3d { avail } else { board }, view_3d),
                ui.scope_id().with("trainer-banner"),
                Sense::click(),
            );
            if banner
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                self.tab = Tab::Devices;
            }
        }
        if !view_3d {
            // Numbers above, the workout or the road below.
            let chart_rect =
                Rect::from_min_max(Pos2::new(avail.min.x, avail.max.y - chart_h), avail.max);
            self.dashboard(&painter, board);
            self.ride_profile(&painter, chart_rect, Some(CARD));
            return;
        }

        // The 3D world fills the screen; the HUD floats over it on glass.
        let view = avail;
        if let Some(e) = self.gpu.error().filter(|_| !self.gpu_reported) {
            self.gpu_reported = true;
            self.log_line(format!("3D graphics: {e}"));
            self.notify("Using the simple 3D view (the detailed one is not supported here — see the log in Devices)");
        }
        let scene = Scene {
            riders: self.scene_riders(),
            route: &self.routes[self.route_idx],
            sky: Sky {
                time: self.store.settings.time_of_day,
                weather: self.store.settings.weather,
            },
            dt,
        };
        let plan = self.hud_plan(view);
        let gpu = self.store.settings.gpu_3d && self.gpu_capable && self.gpu.error().is_none();
        let clip = painter.with_clip_rect(view);
        if gpu {
            self.gpu.draw(
                &clip,
                view,
                &scene,
                self.store.settings.gfx_quality,
                &plan.glass(),
            );
        } else {
            self.world.draw(&clip, view, &scene);
        }
        self.hud(&painter, view, &plan, gpu && self.gpu.frosts_glass());
        if self.ride.state == ride::State::Ready {
            self.tired_banner(ui, true);
        }
    }

    /// The workout's steps, or the road's elevation where there is no workout.
    fn ride_profile(&self, painter: &egui::Painter, rect: Rect, bg: Option<Color32>) {
        if self.ride.mode == Mode::Workout && !self.ride.segs.is_empty() {
            let p: Vec<(f32, f32)> = self
                .ride
                .samples
                .iter()
                .map(|s| (s.elapsed, s.power))
                .collect();
            let progress =
                (self.ride.state != ride::State::Ready).then_some(self.ride.workout_time() as f32);
            charts::workout_profile(
                painter,
                rect,
                &self.ride.segs,
                self.store.settings.ftp,
                self.ride.intensity,
                progress,
                &p,
                bg,
            );
        } else {
            charts::elevation_profile(
                painter,
                rect,
                &self.routes[self.route_idx],
                Some(self.ride.route_pos()),
                bg,
            );
        }
    }

    /// You, and whoever you ride with, for the 3D views.
    fn scene_riders(&self) -> Vec<Figure> {
        let [r, g, b] = self.store.settings.jersey;
        let mut riders = vec![Figure {
            d: self.ride.route_pos(),
            lateral: crate::world::LANE,
            crank: self.ride.crank_angle,
            speed: self.ride.speed,
            jersey: Color32::from_rgb(r, g, b),
            tag: None,
        }];
        if let Some(gap) = self.ride.companion_gap() {
            let (jersey, tag) = match &self.ride.companion {
                Some(ride::Companion::Pacer(p)) => {
                    (self.pacer_jersey(), format!("Pacer · {:.0} W", p.watts))
                }
                Some(ride::Companion::Ghost(gh)) => (
                    Color32::from_rgb(200, 232, 255),
                    format!("Ghost · {}", gh.label.split(',').next().unwrap_or("")),
                ),
                None => unreachable!(),
            };
            riders.push(Figure {
                d: self.ride.route_pos() + gap,
                lateral: crate::world::COMPANION_LANE,
                crank: self.ride.companion_crank,
                speed: self.ride.speed,
                jersey,
                tag: Some(tag),
            });
        }
        riders
    }

    /// Time of day and weather for the 3D world.
    fn sky_menu(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.store.settings;
        let label = if s.weather == Weather::Clear {
            s.time_of_day.label().to_string()
        } else {
            format!("{} · {}", s.time_of_day.label(), s.weather.label())
        };
        let mut changed = false;
        ui.menu_button(format!("{label} ▾"), |ui| {
            ui.set_min_width(170.0);
            ui.label(RichText::new("Time of day").weak());
            for t in TimeOfDay::ALL {
                changed |= ui.radio_value(&mut s.time_of_day, t, t.label()).changed();
            }
            ui.separator();
            ui.label(RichText::new("Weather").weak());
            for w in Weather::ALL {
                changed |= ui.radio_value(&mut s.weather, w, w.label()).changed();
            }
        })
        .response
        .on_hover_text("Time of day and weather");
        if changed {
            self.mark_dirty();
        }
    }

    /// "Ride with": nobody, a pacer at a fixed power, or a ghost of an
    /// earlier ride on this route.
    fn companion_menu(&mut self, ui: &mut egui::Ui) {
        let label = match &self.ride.companion {
            None => "Ride alone".to_string(),
            Some(ride::Companion::Pacer(p)) => format!("Pacer {:.0} W", p.watts),
            Some(ride::Companion::Ghost(_)) => "Ghost".to_string(),
        };
        let ghosts: Vec<(String, ride::Summary)> = self
            .ghost_candidates()
            .into_iter()
            .map(|h| (Self::ghost_label(h), h.clone()))
            .collect();
        let ftp = self.store.settings.ftp;
        let mut load = None;
        ui.menu_button(format!("{label} ▾"), |ui| {
            ui.set_min_width(260.0);
            if ui
                .selectable_label(self.ride.companion.is_none(), "Ride alone")
                .clicked()
            {
                self.ride.companion = None;
                ui.close();
            }
            ui.separator();
            let is_pacer = matches!(self.ride.companion, Some(ride::Companion::Pacer(_)));
            if ui
                .selectable_label(is_pacer, "Pacer at a steady power")
                .on_hover_text(
                    "Another rider holding the power you choose, with the same weight and bike",
                )
                .clicked()
                && !is_pacer
            {
                // Join at your position so a mid-ride pacer is not kilometres back.
                self.ride.companion = Some(ride::Companion::Pacer(ride::Pacer {
                    watts: ((ftp * 0.75) / 5.0).round() * 5.0,
                    distance: self.ride.distance,
                    speed: self.ride.speed,
                }));
            }
            if let Some(ride::Companion::Pacer(p)) = &mut self.ride.companion {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.add(
                        egui::DragValue::new(&mut p.watts)
                            .range(50.0..=600.0)
                            .speed(1.0)
                            .suffix(" W"),
                    );
                    ui.label(
                        RichText::new(format!("{:.0}% of FTP", p.watts / ftp.max(1.0) * 100.0))
                            .weak(),
                    );
                });
            }
            ui.separator();
            ui.label(RichText::new("Race a ghost of an earlier ride on this route").weak());
            if ghosts.is_empty() {
                ui.label(
                    RichText::new("Ride this route once (1 km or more) to unlock")
                        .weak()
                        .italics(),
                );
            }
            for (label, h) in &ghosts {
                let selected = matches!(&self.ride.companion,
                    Some(ride::Companion::Ghost(g)) if &g.label == label);
                let text = format!(
                    "{label}  ·  {:.1} km/h  ·  {}",
                    h.distance_m / h.secs.max(1.0) as f64 * 3.6,
                    fmt_dur(h.secs)
                );
                if ui.selectable_label(selected, text).clicked() {
                    load = Some(h.clone());
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Ride with a pacer or race your ghost");
        if let Some(h) = load {
            self.load_ghost(&h);
        }
    }

    /// A cadence source has reported recently.
    fn has_cadence(&self) -> bool {
        self.metrics
            .cadence_seen
            .is_some_and(|t| t.elapsed().as_secs() < 30)
    }

    fn speed_label(&self) -> &'static str {
        if self.ride.speed_from_trainer {
            "SPEED (trainer)"
        } else {
            "SPEED (virtual)"
        }
    }

    fn source_name(&self, m: &Option<Metric>) -> Option<String> {
        let m = m.as_ref().filter(|m| m.is_fresh())?;
        Some(
            self.links
                .get(&m.src)
                .map(|l| l.name.clone())
                .unwrap_or_else(|| m.src.clone()),
        )
    }

    /// Numbers-first ride screen, an alternative to the 3D view.
    fn dashboard(&self, painter: &egui::Painter, rect: Rect) {
        let s = &self.store.settings;
        let ftp = s.ftp;
        painter.rect_filled(rect, 0.0, theme::BG);
        let r = rect.shrink(12.0);
        let gap = 10.0;
        let h1 = r.height() * 0.28;
        let h2 = r.height() * 0.19;
        let h3 = r.height() * 0.13;
        let row = |y: f32, h: f32, n: usize| -> Vec<Rect> {
            let w = (r.width() - gap * (n - 1) as f32) / n as f32;
            (0..n)
                .map(|i| {
                    Rect::from_min_size(
                        Pos2::new(r.left() + i as f32 * (w + gap), y),
                        Vec2::new(w, h),
                    )
                })
                .collect()
        };
        let sub = |rect: Rect, text: String, c: Color32| {
            painter.text(
                rect.right_top() + Vec2::new(-12.0, 8.0),
                Align2::RIGHT_TOP,
                text,
                FontId::proportional(13.0),
                c,
            );
        };
        let dash = || "—".to_string();
        let grey = Color32::from_gray(150);

        // Row 1: the big four.
        let power = self.metrics.power3s();
        let target = self.ride.target_watts(ftp);
        let grade = self.routes[self.route_idx].grade(self.ride.route_pos());
        let tiles = row(r.top(), h1, 4);
        let pcol = power
            .map(|p| charts::zone_color(p / ftp))
            .unwrap_or(Color32::GRAY);
        tile(
            painter,
            tiles[0],
            "POWER (3s)",
            &power.map(|p| format!("{p:.0}")).unwrap_or_else(dash),
            "W",
            pcol,
        );
        if let Some(p) = power {
            sub(
                tiles[0],
                format!(
                    "{} · {:.1} W/kg",
                    charts::zone_name(p / ftp),
                    p / s.rider_kg.max(1.0)
                ),
                pcol,
            );
        }
        if let Some(t) = target {
            tile(
                painter,
                tiles[1],
                "TARGET",
                &format!("{t:.0}"),
                "W",
                charts::zone_color(t / ftp),
            );
            sub(tiles[1], format!("{:.0}% FTP", t / ftp * 100.0), grey);
        } else {
            let g = if self.ride.mode == Mode::Slope {
                self.ride.manual_grade as f64
            } else {
                grade
            };
            tile(painter, tiles[1], "GRADE", &format!("{g:+.1}"), "%", grey);
        }
        let cadence_tile = self.has_cadence();
        if cadence_tile {
            tile(
                painter,
                tiles[2],
                "CADENCE",
                &fresh(&self.metrics.cadence)
                    .map(|c| format!("{c:.0}"))
                    .unwrap_or_else(dash),
                "rpm",
                Color32::from_rgb(90, 170, 230),
            );
            if let Some(t) = self.ride.target_cadence() {
                let live = fresh(&self.metrics.cadence);
                sub(
                    tiles[2],
                    format!("target {t:.0} rpm"),
                    charts::cadence_colour(live, t),
                );
            } else if let Some(src) = self.source_name(&self.metrics.cadence) {
                sub(tiles[2], src, grey);
            }
        } else {
            tile(
                painter,
                tiles[2],
                self.speed_label(),
                &format!("{:.1}", self.ride.speed * 3.6),
                "km/h",
                Color32::from_rgb(90, 170, 230),
            );
        }
        let hr = fresh(&self.metrics.hr);
        let hr_col = hr
            .map(|h| charts::hr_zone_color(charts::hr_zone(h, s.max_hr)))
            .unwrap_or(Color32::from_rgb(230, 70, 80));
        tile(
            painter,
            tiles[3],
            "HEART RATE",
            &hr.map(|c| format!("{c:.0}")).unwrap_or_else(dash),
            "bpm",
            hr_col,
        );
        if let Some(h) = hr {
            let z = charts::hr_zone(h, s.max_hr);
            sub(
                tiles[3],
                format!(
                    "{} · {:.0}% max",
                    charts::HR_ZONES[z].0,
                    h / s.max_hr * 100.0
                ),
                hr_col,
            );
        }

        // Row 2: interval / ride status.
        let y2 = r.top() + h1 + gap;
        let panel = Rect::from_min_size(Pos2::new(r.left(), y2), Vec2::new(r.width(), h2));
        painter.rect_filled(panel, CornerRadius::same(8), theme::CARD);
        let big = FontId::proportional((h2 * 0.36).min(56.0));
        let small = FontId::proportional(14.0);
        let t = self.ride.workout_time() as f32;
        let left = panel.left_center() + Vec2::new(18.0, 0.0);
        let mid = panel.center();
        let right = panel.right_center() - Vec2::new(18.0, 0.0);
        let bar = |frac: f32, col: Color32| {
            let b = Rect::from_min_size(
                Pos2::new(panel.left() + 18.0, panel.bottom() - 16.0),
                Vec2::new(panel.width() - 36.0, 6.0),
            );
            painter.rect_filled(b, 3.0, Color32::from_white_alpha(25));
            painter.rect_filled(
                Rect::from_min_size(
                    b.min,
                    Vec2::new(b.width() * frac.clamp(0.0, 1.0), b.height()),
                ),
                3.0,
                col,
            );
        };
        match (&self.ride.workout, self.ride.mode) {
            (Some(w), Mode::Workout) if self.ride.workout_done() => {
                painter.text(
                    mid,
                    Align2::CENTER_CENTER,
                    "Workout complete!",
                    big,
                    Color32::from_rgb(120, 220, 140),
                );
                painter.text(
                    mid + Vec2::new(0.0, h2 * 0.32),
                    Align2::CENTER_CENTER,
                    format!("{} — keep spinning to cool down, then press Finish", w.name),
                    small.clone(),
                    Color32::from_gray(200),
                );
            }
            (Some(w), Mode::Workout) => {
                if let Some(i) = workout::segment_index(&self.ride.segs, t) {
                    let seg = self.ride.segs[i];
                    let now = seg.with_cadence(
                        if seg.free {
                            "Free ride".to_string()
                        } else {
                            format!("{:.0} W", seg.at(t) * self.ride.intensity * ftp)
                        },
                        "  ·  ",
                    );
                    painter.text(
                        left - Vec2::new(0.0, h2 * 0.22),
                        Align2::LEFT_CENTER,
                        format!("{}  ·  interval {}/{}", w.name, i + 1, self.ride.segs.len()),
                        small.clone(),
                        Color32::from_gray(190),
                    );
                    painter.text(
                        left + Vec2::new(0.0, h2 * 0.08),
                        Align2::LEFT_CENTER,
                        now,
                        big.clone(),
                        Color32::WHITE,
                    );
                    painter.text(
                        mid - Vec2::new(0.0, h2 * 0.22),
                        Align2::CENTER_CENTER,
                        "INTERVAL TIME LEFT",
                        small.clone(),
                        Color32::from_gray(160),
                    );
                    painter.text(
                        mid + Vec2::new(0.0, h2 * 0.08),
                        Align2::CENTER_CENTER,
                        fmt_dur((seg.end() - t).ceil()),
                        big.clone(),
                        Color32::WHITE,
                    );
                    let next = self.ride.segs.get(i + 1).map(|n| {
                        n.with_cadence(
                            if n.free {
                                format!("Next: {} free ride", fmt_dur(n.secs))
                            } else {
                                format!(
                                    "Next: {} @ {:.0} W",
                                    fmt_dur(n.secs),
                                    n.from * self.ride.intensity * ftp
                                )
                            },
                            ", ",
                        )
                    });
                    painter.text(
                        right - Vec2::new(0.0, h2 * 0.18),
                        Align2::RIGHT_CENTER,
                        next.unwrap_or_else(|| "Last interval".into()),
                        small.clone(),
                        Color32::from_gray(210),
                    );
                    painter.text(
                        right + Vec2::new(0.0, h2 * 0.1),
                        Align2::RIGHT_CENTER,
                        format!("Workout left {}", fmt_dur(w.duration() - t)),
                        small.clone(),
                        Color32::from_gray(160),
                    );
                    let col = if seg.free {
                        Color32::GRAY
                    } else {
                        charts::zone_color(seg.at(t))
                    };
                    bar((t - seg.start) / seg.secs, col);
                }
            }
            (_, Mode::HeartRate) => {
                let c = self.ride.hr_control;
                let hr = fresh(&self.metrics.hr);
                painter.text(
                    left - Vec2::new(0.0, h2 * 0.22),
                    Align2::LEFT_CENTER,
                    "TARGET HEART RATE",
                    small.clone(),
                    Color32::from_gray(190),
                );
                painter.text(
                    left + Vec2::new(0.0, h2 * 0.08),
                    Align2::LEFT_CENTER,
                    format!("{:.0} bpm", c.target_bpm),
                    big.clone(),
                    Color32::WHITE,
                );
                painter.text(
                    mid - Vec2::new(0.0, h2 * 0.22),
                    Align2::CENTER_CENTER,
                    "POWER HELD",
                    small.clone(),
                    Color32::from_gray(160),
                );
                painter.text(
                    mid + Vec2::new(0.0, h2 * 0.08),
                    Align2::CENTER_CENTER,
                    c.power.map(|p| format!("{p:.0} W")).unwrap_or("—".into()),
                    big.clone(),
                    Color32::WHITE,
                );
                let note = match hr {
                    Some(h) if (h - c.target_bpm).abs() <= 3.0 => "On target".to_string(),
                    Some(h) if h < c.target_bpm => {
                        format!("{:.0} bpm below — easing power up", c.target_bpm - h)
                    }
                    Some(h) => format!("{:.0} bpm above — easing power down", h - c.target_bpm),
                    None => "No heart rate signal — holding power".to_string(),
                };
                painter.text(
                    right,
                    Align2::RIGHT_CENTER,
                    note,
                    small.clone(),
                    Color32::from_gray(210),
                );
                if let Some(h) = hr {
                    let col = charts::hr_zone_color(charts::hr_zone(h, s.max_hr));
                    bar((h / c.target_bpm.max(1.0)).min(1.2) / 1.2, col);
                }
            }
            _ => {
                let route = &self.routes[self.route_idx];
                let lap = (self.ride.route_pos() / route.length).floor() as u32 + 1;
                let along = self.ride.route_pos().rem_euclid(route.length);
                let (mode, g) = if self.ride.mode == Mode::Slope {
                    ("Manual slope".to_string(), self.ride.manual_grade as f64)
                } else {
                    let felt = match self.ride.trainer_setting() {
                        Some(devices::Control::Grade(g)) => format!(" · trainer set to {g:+.1}%"),
                        _ => String::new(),
                    };
                    (
                        format!("Free ride · difficulty {:.0}%{felt}", s.difficulty * 100.0),
                        grade,
                    )
                };
                painter.text(
                    left - Vec2::new(0.0, h2 * 0.22),
                    Align2::LEFT_CENTER,
                    mode,
                    small.clone(),
                    Color32::from_gray(190),
                );
                painter.text(
                    left + Vec2::new(0.0, h2 * 0.08),
                    Align2::LEFT_CENTER,
                    format!("{g:+.1}%"),
                    big.clone(),
                    Color32::WHITE,
                );
                painter.text(
                    mid - Vec2::new(0.0, h2 * 0.22),
                    Align2::CENTER_CENTER,
                    route.name.clone(),
                    small.clone(),
                    Color32::from_gray(160),
                );
                painter.text(
                    mid + Vec2::new(0.0, h2 * 0.08),
                    Align2::CENTER_CENTER,
                    format!("Lap {lap}"),
                    big.clone(),
                    Color32::WHITE,
                );
                painter.text(
                    right,
                    Align2::RIGHT_CENTER,
                    format!("{:.2} / {:.1} km", along / 1000.0, route.length / 1000.0),
                    small.clone(),
                    Color32::from_gray(210),
                );
                bar(
                    (along / route.length) as f32,
                    Color32::from_rgb(90, 150, 100),
                );
            }
        }
        if self.ride.state == ride::State::Ready {
            painter.text(
                panel.center_top() + Vec2::new(0.0, 10.0),
                Align2::CENTER_TOP,
                "Press Start (or Space) when you're ready",
                small.clone(),
                Color32::from_rgb(240, 200, 90),
            );
        } else if self.ride.state == ride::State::Paused {
            painter.text(
                panel.center_top() + Vec2::new(0.0, 10.0),
                Align2::CENTER_TOP,
                "PAUSED",
                FontId::proportional(18.0),
                Color32::from_rgb(240, 200, 90),
            );
        }

        // Row 3: ride totals.
        let y3 = y2 + h2 + gap;
        let p = self.ride.power_series();
        let hrs: Vec<f32> = self
            .ride
            .samples
            .iter()
            .filter_map(|s| s.hr)
            .filter(|h| *h > 0.0)
            .collect();
        let avg_hr = (!hrs.is_empty()).then(|| hrs.iter().sum::<f32>() / hrs.len() as f32);
        // `+ 0.0` turns the empty sum's -0.0 into 0.0 so it never shows as "-0".
        let kj = p.iter().sum::<f32>() / 1000.0 + 0.0;
        let tiles = row(y3, h3, 6);
        tile(
            painter,
            tiles[0],
            "TIME",
            &fmt_dur(self.ride.elapsed as f32),
            "",
            grey,
        );
        tile(
            painter,
            tiles[1],
            // Leave room for the companion's gap beside the caption.
            if self.ride.companion.is_some() {
                "DIST"
            } else {
                "DISTANCE"
            },
            &format!("{:.2}", self.ride.distance / 1000.0),
            "km",
            grey,
        );
        if let Some((title, value, unit, col)) = self.companion_status() {
            let who = title.split(' ').next().unwrap_or("").to_lowercase();
            sub(tiles[1], format!("{who} {value} {unit}"), col);
        }
        if cadence_tile {
            tile(
                painter,
                tiles[2],
                self.speed_label(),
                &format!("{:.1}", self.ride.speed * 3.6),
                "km/h",
                grey,
            );
        } else {
            tile(
                painter,
                tiles[2],
                "CLIMBED",
                &format!("{:.0}", self.ride.ascent),
                "m",
                grey,
            );
        }
        tile(
            painter,
            tiles[3],
            "AVG POWER",
            &if p.is_empty() {
                dash()
            } else {
                format!("{:.0}", self.ride.avg_power())
            },
            "W",
            grey,
        );
        tile(
            painter,
            tiles[4],
            "NORMALIZED",
            &if p.is_empty() {
                dash()
            } else {
                format!("{:.0}", workout::normalized_power(&p))
            },
            "W",
            grey,
        );
        tile(
            painter,
            tiles[5],
            "AVG HR / ENERGY",
            &format!(
                "{} · {kj:.0}",
                avg_hr.map(|h| format!("{h:.0}")).unwrap_or_else(dash)
            ),
            "bpm · kJ",
            grey,
        );

        // Row 4: live chart.
        let y4 = y3 + h3 + gap;
        let chart = Rect::from_min_max(Pos2::new(r.left(), y4), r.max);
        let target: Vec<Option<f32>> = self
            .ride
            .samples
            .iter()
            .map(|s| {
                if self.ride.mode != Mode::Workout {
                    return None;
                }
                workout::target_at(&self.ride.segs, s.elapsed)
                    .filter(|t| !t.1)
                    .map(|t| (t.0 * self.ride.intensity * ftp).round())
            })
            .collect();
        let hr: Vec<Option<f32>> = self
            .ride
            .samples
            .iter()
            .map(|s| s.hr.filter(|h| *h > 0.0))
            .collect();
        charts::live_chart(painter, chart, &p, &target, &hr, ftp, 600, theme::CARD);
        self.trainer_warning(painter, Self::banner_rect(rect, false));
    }

    // ------------------------------------------------------------------
    // Workouts

    fn workouts_tab(&mut self, ui: &mut egui::Ui) {
        let lib = self.library();
        if self.selected_workout.is_none() {
            self.selected_workout = self
                .ride_workout
                .clone()
                .or_else(|| lib.first().map(|w| w.id.clone()));
        }
        egui::Panel::left("workout-list")
            .default_size(300.0)
            .resizable(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.workout_filter)
                            .hint_text("Search…")
                            .desired_width(150.0),
                    );
                    if ui
                        .button("New")
                        .on_hover_text("Create a custom workout")
                        .clicked()
                    {
                        let id = format!("custom-{}", Local::now().timestamp());
                        self.store.custom_workouts.push(Workout {
                            id: id.clone(),
                            name: "My workout".into(),
                            description: String::new(),
                            category: Category::Custom,
                            steps: vec![
                                Step::Ramp {
                                    secs: 600,
                                    from: 0.45,
                                    to: 0.72,
                                    cadence: None,
                                },
                                Step::Steady {
                                    secs: 1200,
                                    power: 0.75,
                                    cadence: None,
                                },
                                Step::Ramp {
                                    secs: 300,
                                    from: 0.65,
                                    to: 0.4,
                                    cadence: None,
                                },
                            ],
                        });
                        self.selected_workout = Some(id);
                        self.editing = true;
                        self.mark_dirty();
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let filter = self.workout_filter.to_lowercase();
                        for cat in Category::ALL {
                            let items: Vec<&Workout> = lib
                                .iter()
                                .filter(|w| {
                                    w.category == cat
                                        && (filter.is_empty()
                                            || w.name.to_lowercase().contains(&filter))
                                })
                                .collect();
                            if items.is_empty() {
                                continue;
                            }
                            let colour = charts::category_color(cat);
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(cat.label().to_uppercase())
                                    .font(theme::semibold(10.5))
                                    .color(theme::mix(colour, Color32::WHITE, 0.15)),
                            );
                            for w in items {
                                let sel = self.selected_workout.as_deref() == Some(&w.id);
                                let (_, tss) = w.load();
                                let meta = format!("{} · {:.0} TSS", fmt_dur(w.duration()), tss);
                                if list_row(ui, sel, &w.name, &meta, colour).clicked() {
                                    self.selected_workout = Some(w.id.clone());
                                    self.editing = false;
                                }
                            }
                            ui.add_space(6.0);
                        }
                    });
            });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Import .zwo / .gpx:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.import_path)
                        .hint_text("/path/to/file.zwo")
                        .desired_width(320.0),
                );
                if ui.button("Import").clicked() {
                    let p = self.import_path.trim().to_string();
                    let p = if let Some(rest) = p.strip_prefix("~/") {
                        std::env::var("HOME")
                            .map(|h| format!("{h}/{rest}"))
                            .unwrap_or(p)
                    } else {
                        p
                    };
                    self.import_file(Path::new(&p), None);
                }
                ui.label(RichText::new("…or drag & drop files onto the window").weak());
            });
            ui.separator();
            let Some(id) = self.selected_workout.clone() else {
                ui.label("Select a workout on the left.");
                return;
            };
            let Some(w) = self.find_workout(&id) else {
                return;
            };
            let custom = self.store.custom_workouts.iter().position(|c| c.id == id);
            let ftp = self.store.settings.ftp;
            let (ifac, tss) = w.load();
            ui.heading(&w.name);
            ui.label(format!(
                "{} · {}  ·  IF {:.2}  ·  {:.0} TSS  ·  max {:.0} W",
                w.category.label(),
                fmt_dur(w.duration()),
                ifac,
                tss,
                w.max_target() * ftp
            ));
            if !w.description.is_empty() {
                ui.label(RichText::new(&w.description).weak());
            }
            ui.add_space(6.0);
            let (r, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 150.0), Sense::hover());
            charts::workout_profile(
                ui.painter(),
                r,
                &w.segments(),
                ftp,
                1.0,
                None,
                &[],
                Some(CARD),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let rideable = w.duration() > 0.0;
                if ui
                    .add_enabled_ui(rideable, |ui| {
                        icon_button(
                            ui,
                            Icon::Play,
                            "Ride now",
                            Some(Color32::from_rgb(36, 150, 84)),
                        )
                    })
                    .inner
                    .clicked()
                {
                    self.load_ride(Some(id.clone()), None);
                    self.tab = Tab::Ride;
                }
                ui.separator();
                ui.label("Schedule on");
                date_picker(ui, &mut self.schedule_date);
                if ui.button("Add to calendar").clicked() {
                    self.store.plan.push(PlannedWorkout {
                        date: self.schedule_date,
                        workout_id: id.clone(),
                        done: false,
                        skipped: false,
                    });
                    self.mark_dirty();
                    self.notify(format!(
                        "Scheduled {} on {}",
                        w.name,
                        self.schedule_date.format("%a %d %b")
                    ));
                }
                ui.separator();
                if custom.is_some() {
                    if ui.selectable_label(self.editing, "Edit").clicked() {
                        self.editing = !self.editing;
                    }
                    if ui.button("Delete").clicked() {
                        self.store.custom_workouts.retain(|c| c.id != id);
                        self.selected_workout = None;
                        self.mark_dirty();
                    }
                } else if ui.button("Copy to custom").clicked() {
                    let mut c = w.clone();
                    c.id = format!("custom-{}", Local::now().timestamp());
                    c.name = format!("{} (copy)", w.name);
                    c.category = Category::Custom;
                    self.selected_workout = Some(c.id.clone());
                    self.store.custom_workouts.push(c);
                    self.editing = true;
                    self.mark_dirty();
                }
            });
            ui.separator();
            match (custom, self.editing) {
                (Some(ci), true) => {
                    if self.workout_editor(ui, ci) {
                        self.mark_dirty();
                    }
                }
                _ => {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (i, s) in w.steps.iter().enumerate() {
                            ui.label(format!("{:>2}. {}", i + 1, s.describe()));
                        }
                    });
                }
            }
        });
    }

    fn workout_editor(&mut self, ui: &mut egui::Ui, ci: usize) -> bool {
        let mut changed = false;
        let w = &mut self.store.custom_workouts[ci];
        egui::Grid::new("wk-meta").num_columns(2).show(ui, |ui| {
            ui.label("Name");
            changed |= ui.text_edit_singleline(&mut w.name).changed();
            ui.end_row();
            ui.label("Description");
            changed |= ui.text_edit_singleline(&mut w.description).changed();
            ui.end_row();
        });
        ui.add_space(6.0);
        let mut remove = None;
        let mut swap = None;
        egui::ScrollArea::vertical()
            .max_height(ui.available_height() - 40.0)
            .show(ui, |ui| {
                for (i, step) in w.steps.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{:>2}.", i + 1));
                        match step {
                            Step::Steady {
                                secs,
                                power,
                                cadence,
                            } => {
                                ui.label("Steady");
                                changed |= secs_edit(ui, secs);
                                ui.label("at");
                                changed |= pct_edit(ui, power);
                                changed |= cadence_edit(ui, cadence);
                            }
                            Step::Ramp {
                                secs,
                                from,
                                to,
                                cadence,
                            } => {
                                ui.label("Ramp");
                                changed |= secs_edit(ui, secs);
                                ui.label("from");
                                changed |= pct_edit(ui, from);
                                ui.label("to");
                                changed |= pct_edit(ui, to);
                                changed |= cadence_edit(ui, cadence);
                            }
                            Step::Intervals {
                                repeat,
                                on_secs,
                                on_power,
                                off_secs,
                                off_power,
                                on_cadence,
                                off_cadence,
                            } => {
                                ui.label("Intervals");
                                changed |= ui
                                    .add(egui::DragValue::new(repeat).range(1..=50).suffix("×"))
                                    .changed();
                                changed |= secs_edit(ui, on_secs);
                                ui.label("@");
                                changed |= pct_edit(ui, on_power);
                                changed |= cadence_edit(ui, on_cadence);
                                ui.label("then");
                                changed |= secs_edit(ui, off_secs);
                                ui.label("@");
                                changed |= pct_edit(ui, off_power);
                                changed |= cadence_edit(ui, off_cadence);
                            }
                            Step::FreeRide { secs, cadence } => {
                                ui.label("Free ride");
                                changed |= secs_edit(ui, secs);
                                changed |= cadence_edit(ui, cadence);
                            }
                        }
                        if icon_button(ui, Icon::Up, "", None)
                            .on_hover_text("Move up")
                            .clicked()
                            && i > 0
                        {
                            swap = Some(i);
                        }
                        if icon_button(ui, Icon::Close, "", None)
                            .on_hover_text("Remove step")
                            .clicked()
                        {
                            remove = Some(i);
                        }
                    });
                }
            });
        if let Some(i) = remove {
            w.steps.remove(i);
            changed = true;
        }
        if let Some(i) = swap {
            w.steps.swap(i, i - 1);
            changed = true;
        }
        ui.horizontal(|ui| {
            ui.label("Add:");
            let add: Option<Step> = if ui.button("Steady").clicked() {
                Some(Step::Steady {
                    secs: 300,
                    power: 0.75,
                    cadence: None,
                })
            } else if ui.button("Ramp").clicked() {
                Some(Step::Ramp {
                    secs: 300,
                    from: 0.5,
                    to: 0.75,
                    cadence: None,
                })
            } else if ui.button("Intervals").clicked() {
                Some(Step::Intervals {
                    repeat: 4,
                    on_secs: 120,
                    on_power: 1.05,
                    off_secs: 120,
                    off_power: 0.5,
                    on_cadence: None,
                    off_cadence: None,
                })
            } else if ui.button("Free ride").clicked() {
                Some(Step::FreeRide {
                    secs: 600,
                    cadence: None,
                })
            } else {
                None
            };
            if let Some(s) = add {
                w.steps.push(s);
                changed = true;
            }
        });
        changed
    }

    // ------------------------------------------------------------------
    // Calendar & plan

    fn plan_tab(&mut self, ui: &mut egui::Ui) {
        let lib = self.library();
        let by_id = |id: &str| lib.iter().find(|w| w.id == id);
        egui::Panel::left("plan-gen")
            .default_size(290.0)
            .resizable(false)
            .show(ui, |ui| {
                ui.heading("Generate a plan");
                ui.add_space(4.0);
                egui::ComboBox::from_label("Goal")
                    .selected_text(self.plan_cfg.goal.label())
                    .show_ui(ui, |ui| {
                        for g in Goal::ALL {
                            ui.selectable_value(&mut self.plan_cfg.goal, g, g.label());
                        }
                    });
                ui.label(RichText::new(self.plan_cfg.goal.description()).weak());
                ui.add_space(6.0);
                ui.label("Start date");
                date_picker(ui, &mut self.plan_cfg.start);
                ui.add(egui::Slider::new(&mut self.plan_cfg.weeks, 2..=16).text("weeks"));
                ui.add_space(6.0);
                ui.label("Riding days");
                ui.horizontal_wrapped(|ui| {
                    for (d, on) in self.plan_cfg.days.iter_mut().enumerate() {
                        ui.toggle_value(on, plan::weekday_label(d));
                    }
                });
                egui::ComboBox::from_label("Long ride day")
                    .selected_text(plan::weekday_label(self.plan_cfg.long_day))
                    .show_ui(ui, |ui| {
                        for d in 0..7 {
                            ui.selectable_value(
                                &mut self.plan_cfg.long_day,
                                d,
                                plan::weekday_label(d),
                            );
                        }
                    });
                ui.checkbox(
                    &mut self.replace_plan,
                    "Replace planned (not done) workouts from start date",
                );
                ui.add_space(6.0);
                let preview = plan::generate(&self.plan_cfg);
                let hours: f32 = preview
                    .iter()
                    .filter_map(|p| by_id(&p.workout_id))
                    .map(|w| w.duration())
                    .sum::<f32>()
                    / 3600.0;
                let weeks = self.plan_cfg.weeks.max(1) as f32;
                ui.label(format!(
                    "{} workouts · ~{:.1} h/week",
                    preview.len(),
                    hours / weeks
                ));
                if ui
                    .add_enabled(
                        !preview.is_empty(),
                        egui::Button::new(RichText::new("Generate plan").color(Color32::WHITE))
                            .fill(Color32::from_rgb(44, 112, 196)),
                    )
                    .clicked()
                {
                    if self.replace_plan {
                        let start = self.plan_cfg.start;
                        self.store.plan.retain(|p| p.done || p.date < start);
                    }
                    let n = preview.len();
                    self.store.plan.extend(preview);
                    self.store.plan.sort_by_key(|p| p.date);
                    self.month = plan::first_of_month(self.plan_cfg.start);
                    self.mark_dirty();
                    self.notify(format!("Added {n} workouts to your calendar"));
                }
                ui.add_space(10.0);
                ui.separator();
                let today = Local::now().date_naive();
                if self.icu_creds.is_set() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("intervals.icu").strong());
                        if self.icu_busy {
                            ui.spinner();
                        } else if ui.small_button("Sync now").clicked() {
                            self.icu_sync();
                        }
                    });
                    if let Some(n) = &self.icu_note {
                        ui.label(RichText::new(n).small().weak());
                    }
                    ui.add_space(6.0);
                }
                ui.label(RichText::new("Up next").strong());
                for p in self
                    .store
                    .plan
                    .iter()
                    .filter(|p| p.date >= today && !p.done)
                    .take(6)
                {
                    let name = by_id(&p.workout_id)
                        .map(|w| w.name.as_str())
                        .unwrap_or("(missing)");
                    ui.label(format!("{}  {}", p.date.format("%a %d %b"), name));
                }
                ui.add_space(6.0);
                if ui.button("Clear all future planned workouts").clicked() {
                    self.store.plan.retain(|p| p.done || p.date < today);
                    self.mark_dirty();
                }
            });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            self.missed_banner(ui);
            self.tired_banner(ui, false);
            ui.horizontal(|ui| {
                if icon_button(ui, Icon::Left, "", None)
                    .on_hover_text("Previous month")
                    .clicked()
                {
                    self.month = plan::add_months(self.month, -1);
                }
                ui.heading(self.month.format("%B %Y").to_string());
                if icon_button(ui, Icon::Right, "", None)
                    .on_hover_text("Next month")
                    .clicked()
                {
                    self.month = plan::add_months(self.month, 1);
                }
                if ui.button("Today").clicked() {
                    self.month = plan::first_of_month(Local::now().date_naive());
                    self.sel_day = Local::now().date_naive();
                }
            });
            let today = Local::now().date_naive();
            let detail_h = 190.0;
            let avail = ui.available_rect_before_wrap();
            let grid = Rect::from_min_max(
                avail.min,
                Pos2::new(avail.max.x - 70.0, avail.max.y - detail_h),
            );
            let cols = 7.0;
            let head = 22.0;
            let cw = grid.width() / cols;
            let ch = (grid.height() - head) / 6.0;
            let painter = ui.painter().clone();
            for d in 0..7 {
                painter.text(
                    Pos2::new(grid.left() + cw * (d as f32 + 0.5), grid.top() + 10.0),
                    Align2::CENTER_CENTER,
                    plan::weekday_label(d),
                    FontId::proportional(13.0),
                    Color32::from_gray(170),
                );
            }
            let start = plan::month_grid_start(self.month);
            let mut clicked_day = None;
            for row in 0..6 {
                let mut week_tss = 0.0;
                for col in 0..7 {
                    let date = start + Duration::days(row * 7 + col);
                    let r = Rect::from_min_size(
                        Pos2::new(
                            grid.left() + cw * col as f32,
                            grid.top() + head + ch * row as f32,
                        ),
                        Vec2::new(cw, ch),
                    )
                    .shrink(2.0);
                    let resp = ui.interact(r, ui.scope_id().with(("cal", date)), Sense::click());
                    if resp.clicked() {
                        clicked_day = Some(date);
                    }
                    let in_month = date.month() == self.month.month();
                    let mut fill = if in_month {
                        CARD
                    } else {
                        Color32::from_rgb(24, 26, 32)
                    };
                    if resp.hovered() {
                        fill = fill.gamma_multiply(1.3);
                    }
                    painter.rect_filled(r, CornerRadius::same(6), fill);
                    if date == self.sel_day {
                        painter.rect_stroke(
                            r,
                            CornerRadius::same(6),
                            Stroke::new(2.0, Color32::from_rgb(240, 130, 60)),
                            egui::StrokeKind::Inside,
                        );
                    } else if date == today {
                        painter.rect_stroke(
                            r,
                            CornerRadius::same(6),
                            Stroke::new(1.5, Color32::from_rgb(90, 160, 240)),
                            egui::StrokeKind::Inside,
                        );
                    }
                    painter.text(
                        r.left_top() + Vec2::new(6.0, 4.0),
                        Align2::LEFT_TOP,
                        date.day().to_string(),
                        FontId::proportional(12.0),
                        if in_month {
                            Color32::from_gray(220)
                        } else {
                            Color32::from_gray(110)
                        },
                    );
                    let mut yy = r.top() + 22.0;
                    for p in self.store.plan.iter().filter(|p| p.date == date) {
                        let Some(w) = by_id(&p.workout_id) else {
                            continue;
                        };
                        week_tss += w.load().1;
                        if yy + 16.0 > r.bottom() {
                            break;
                        }
                        let pill = Rect::from_min_size(
                            Pos2::new(r.left() + 4.0, yy),
                            Vec2::new(r.width() - 8.0, 16.0),
                        );
                        let c = charts::category_color(w.category);
                        let missed = !p.done && !p.skipped && p.date < today;
                        let fill = if p.skipped {
                            Color32::from_gray(70)
                        } else if p.done {
                            c.gamma_multiply(0.45)
                        } else if missed {
                            c.gamma_multiply(0.35)
                        } else {
                            c.gamma_multiply(0.85)
                        };
                        painter.rect_filled(pill, CornerRadius::same(4), fill);
                        if missed {
                            painter.rect_stroke(
                                pill,
                                CornerRadius::same(4),
                                Stroke::new(1.5, Color32::from_rgb(235, 100, 80)),
                                egui::StrokeKind::Inside,
                            );
                        }
                        let mut tx = pill.left() + 4.0;
                        if p.done {
                            paint_icon(
                                &painter,
                                Rect::from_min_size(
                                    Pos2::new(tx, pill.top() + 3.0),
                                    Vec2::splat(10.0),
                                ),
                                Icon::Check,
                                Color32::BLACK,
                            );
                            tx += 13.0;
                        }
                        painter.with_clip_rect(pill).text(
                            Pos2::new(tx, pill.center().y),
                            Align2::LEFT_CENTER,
                            &w.name,
                            FontId::proportional(11.0),
                            Color32::BLACK,
                        );
                        yy += 18.0;
                    }
                    for h in self
                        .store
                        .history
                        .iter()
                        .filter(|h| h.start.date_naive() == date && h.workout_id.is_none())
                    {
                        if yy + 16.0 > r.bottom() {
                            break;
                        }
                        let pill = Rect::from_min_size(
                            Pos2::new(r.left() + 4.0, yy),
                            Vec2::new(r.width() - 8.0, 16.0),
                        );
                        painter.rect_filled(pill, CornerRadius::same(4), Color32::from_gray(80));
                        painter.with_clip_rect(pill).text(
                            pill.left_center() + Vec2::new(4.0, 0.0),
                            Align2::LEFT_CENTER,
                            format!("Ridden: {} · {:.0} km", h.name, h.distance_m / 1000.0),
                            FontId::proportional(11.0),
                            Color32::WHITE,
                        );
                        yy += 18.0;
                    }
                }
                let wr = Rect::from_min_size(
                    Pos2::new(
                        grid.right() + 6.0,
                        grid.top() + head + ch * row as f32 + 2.0,
                    ),
                    Vec2::new(62.0, ch - 4.0),
                );
                painter.text(
                    wr.center(),
                    Align2::CENTER_CENTER,
                    format!("{week_tss:.0}\nTSS"),
                    FontId::proportional(12.0),
                    Color32::from_gray(160),
                );
            }
            if let Some(d) = clicked_day {
                self.sel_day = d;
            }
            ui.allocate_rect(
                Rect::from_min_max(avail.min, Pos2::new(avail.max.x, avail.max.y - detail_h)),
                Sense::hover(),
            );

            // Day detail.
            ui.separator();
            ui.horizontal(|ui| {
                ui.heading(self.sel_day.format("%A %d %B").to_string());
                ui.separator();
                egui::ComboBox::from_id_salt("add-wk")
                    .width(220.0)
                    .selected_text(
                        by_id(&self.add_workout)
                            .map(|w| w.name.clone())
                            .unwrap_or_default(),
                    )
                    .show_ui(ui, |ui| {
                        for w in &lib {
                            ui.selectable_value(&mut self.add_workout, w.id.clone(), &w.name);
                        }
                    });
                if ui.button("Add to this day").clicked() {
                    self.store.plan.push(PlannedWorkout {
                        date: self.sel_day,
                        workout_id: self.add_workout.clone(),
                        done: false,
                        skipped: false,
                    });
                    self.mark_dirty();
                }
            });
            let mut action: Option<(usize, &str)> = None;
            egui::ScrollArea::vertical()
                .id_salt("day-detail")
                .show(ui, |ui| {
                    for (i, p) in self
                        .store
                        .plan
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| p.date == self.sel_day)
                    {
                        let w = by_id(&p.workout_id);
                        ui.horizontal(|ui| {
                            let (name, info) = match w {
                                Some(w) => (
                                    w.name.clone(),
                                    format!("{} · {:.0} TSS", fmt_dur(w.duration()), w.load().1),
                                ),
                                None => (format!("{} (missing)", p.workout_id), String::new()),
                            };
                            ui.label(RichText::new(name).strong());
                            ui.label(info);
                            if p.done {
                                ui.label(
                                    RichText::new("Done")
                                        .strong()
                                        .color(Color32::from_rgb(110, 210, 130)),
                                );
                            }
                            if w.is_some() && icon_button(ui, Icon::Play, "Ride", None).clicked() {
                                action = Some((i, "ride"));
                            }
                            if ui
                                .button(if p.done { "Mark not done" } else { "Mark done" })
                                .clicked()
                            {
                                action = Some((i, "toggle"));
                            }
                            if icon_button(ui, Icon::Left, "Day earlier", None).clicked() {
                                action = Some((i, "earlier"));
                            }
                            if icon_button(ui, Icon::Right, "Day later", None).clicked() {
                                action = Some((i, "later"));
                            }
                            if ui.button("Remove").clicked() {
                                action = Some((i, "remove"));
                            }
                        });
                    }
                    for h in self
                        .store
                        .history
                        .iter()
                        .filter(|h| h.start.date_naive() == self.sel_day)
                    {
                        ui.label(format!(
                            "Ridden: {} · {} · {:.1} km · {:.0} W avg · {:.0} TSS",
                            h.name,
                            fmt_dur(h.secs),
                            h.distance_m / 1000.0,
                            h.avg_power,
                            h.tss
                        ));
                    }
                });
            if let Some((i, a)) = action {
                match a {
                    "ride" => {
                        let p = self.store.plan[i].clone();
                        self.load_ride(Some(p.workout_id), Some(p.date));
                        self.tab = Tab::Ride;
                    }
                    "toggle" => self.store.plan[i].done = !self.store.plan[i].done,
                    "earlier" => {
                        self.store.plan[i].date -= Duration::days(1);
                        self.sel_day = self.store.plan[i].date;
                    }
                    "later" => {
                        self.store.plan[i].date += Duration::days(1);
                        self.sel_day = self.store.plan[i].date;
                    }
                    "remove" => {
                        self.store.plan.remove(i);
                    }
                    _ => {}
                }
                self.mark_dirty();
            }
        });
    }

    // ------------------------------------------------------------------
    // History

    /// Fill in power curves for rides saved before they were recorded,
    /// a couple per frame so the screen stays responsive.
    fn backfill_curves(&mut self, ctx: &egui::Context) {
        let mut done = 0;
        for i in 0..self.store.history.len() {
            if done >= 2 {
                ctx.request_repaint();
                break;
            }
            let h = &self.store.history[i];
            if !h.best.is_empty() || h.secs < 5.0 || self.curve_failed.contains(&h.file) {
                continue;
            }
            let file = h.file.clone();
            match std::fs::read_to_string(&file)
                .ok()
                .and_then(|x| crate::activity::parse_tcx(&x).ok())
            {
                Some(a) if !a.samples.is_empty() => {
                    self.store.history[i].best = crate::progress::best_curve(&a.samples);
                    self.dirty = true;
                }
                _ => {
                    self.curve_failed.insert(file);
                }
            }
            done += 1;
        }
    }

    fn progress_tab(&mut self, ui: &mut egui::Ui) {
        use crate::progress;
        self.backfill_curves(&ui.ctx().clone());
        let today = Local::now().date_naive();
        let ftp = self.store.settings.ftp;
        let history = &self.store.history;
        ui.heading("Progress");
        ui.add_space(4.0);

        // Totals.
        let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
        let year_start = NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap();
        let periods = [
            (
                "This week",
                progress::totals_between(history, monday, today),
            ),
            (
                "Last 4 weeks",
                progress::totals_between(history, today - Duration::days(27), today),
            ),
            (
                "This year",
                progress::totals_between(history, year_start, today),
            ),
        ];
        let fit = progress::fitness(history, today, 120);
        let (ctl, atl, tsb) = fit.today();
        ui.horizontal_wrapped(|ui| {
            for (label, t) in periods {
                egui::Frame::new()
                    .fill(CARD)
                    .corner_radius(8)
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.set_width(250.0);
                            ui.label(RichText::new(label).small().weak());
                            ui.label(
                                RichText::new(format!(
                                    "{} · {}",
                                    fmt_hours(t.secs),
                                    if t.rides == 1 {
                                        "1 ride".into()
                                    } else {
                                        format!("{} rides", t.rides)
                                    }
                                ))
                                .size(18.0)
                                .strong(),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{:.0} km · {:.0} TSS · {:.0} kJ",
                                    t.km, t.tss, t.kj
                                ))
                                .weak(),
                            );
                        });
                    });
            }
            egui::Frame::new()
                .fill(CARD)
                .corner_radius(8)
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(250.0);
                        ui.label(RichText::new("Today").small().weak());
                        let (word, col) = match tsb {
                            t if t > 10.0 => ("Fresh", Color32::from_rgb(110, 210, 130)),
                            t if t > -10.0 => ("Balanced", Color32::from_gray(220)),
                            t if t > -25.0 => ("Training hard", Color32::from_rgb(240, 200, 80)),
                            _ => ("Very tired", Color32::from_rgb(240, 120, 90)),
                        };
                        ui.label(RichText::new(word).size(18.0).strong().color(col));
                        ui.label(
                            RichText::new(format!(
                                "Fitness {ctl:.0} · Fatigue {atl:.0} · Form {tsb:+.0}"
                            ))
                            .weak(),
                        );
                    });
                });
        });
        ui.add_space(10.0);

        // Fitness trend.
        ui.label(RichText::new("Fitness, fatigue and form (last 4 months)").strong());
        let (r, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width() - 14.0, 170.0),
            Sense::hover(),
        );
        let series = if history.is_empty() {
            vec![]
        } else {
            vec![
                ("Fitness", Color32::from_rgb(80, 160, 240), fit.ctl.clone()),
                ("Fatigue", Color32::from_rgb(230, 110, 140), fit.atl.clone()),
                ("Form", Color32::from_rgb(240, 200, 80), fit.tsb.clone()),
            ]
        };
        charts::lines(ui.painter(), r, &series, CARD, Color32::from_gray(160));
        ui.label(
            RichText::new("Fitness: your training load over the last ~6 weeks · Fatigue: the last week · Form = fitness − fatigue (positive means fresh, very negative means tired)")
                .small()
                .weak(),
        );
        ui.add_space(10.0);

        // Weekly hours.
        ui.label(RichText::new("Weekly riding (last 16 weeks)").strong());
        let weeks = progress::weekly(history, today, 16);
        let items: Vec<(String, f32, String)> = weeks
            .iter()
            .map(|(start, t)| {
                (
                    start.format("%d %b").to_string(),
                    t.secs / 3600.0,
                    if t.secs > 0.0 {
                        format!("{:.1} h", t.secs / 3600.0)
                    } else {
                        String::new()
                    },
                )
            })
            .collect();
        let (r, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width() - 14.0, 160.0),
            Sense::hover(),
        );
        charts::bars(ui.painter(), r, &items, ACCENT, CARD);
        ui.add_space(10.0);

        // Power curve.
        ui.label(RichText::new("Power curve: your best power for each duration").strong());
        let all = progress::merge_curves(history.iter().map(|h| h.best.as_slice()));
        let six_weeks = today - Duration::days(42);
        let recent = progress::merge_curves(
            history
                .iter()
                .filter(|h| h.start.date_naive() >= six_weeks)
                .map(|h| h.best.as_slice()),
        );
        let (r, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width() - 14.0, 230.0),
            Sense::hover(),
        );
        charts::power_curve(
            ui.painter(),
            r,
            &[
                ("All time", ACCENT, all.clone()),
                ("Last 6 weeks", Color32::WHITE, recent.clone()),
            ],
            ftp,
            CARD,
        );
        if !all.is_empty() {
            egui::Grid::new("curve-table")
                .num_columns(6)
                .spacing([22.0, 4.0])
                .show(ui, |ui| {
                    ui.label("");
                    let keys = [
                        (5, "5 s"),
                        (60, "1 min"),
                        (300, "5 min"),
                        (1200, "20 min"),
                        (3600, "60 min"),
                    ];
                    for (_, l) in keys {
                        ui.label(RichText::new(l).weak());
                    }
                    ui.end_row();
                    for (name, curve) in [("All time", &all), ("Last 6 weeks", &recent)] {
                        ui.label(name);
                        for (s, _) in keys {
                            let v = curve
                                .iter()
                                .find(|p| p.0 == s)
                                .map(|p| format!("{:.0} W", p.1))
                                .unwrap_or("—".into());
                            ui.label(RichText::new(v).strong());
                        }
                        ui.end_row();
                    }
                });
        }
        ui.add_space(10.0);

        // FTP history.
        ui.label(RichText::new("FTP history").strong());
        let entries = self.store.ftp_history.clone();
        if entries.len() < 2 {
            ui.label(RichText::new("Your FTP over time appears here once it changes, for example after a ramp test.").weak());
        }
        let (r, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width() - 14.0, 140.0),
            Sense::hover(),
        );
        charts::ftp_history(ui.painter(), r, &entries, today, CARD);
        ui.add_space(10.0);

        // Monthly totals.
        ui.label(RichText::new("Monthly totals").strong());
        egui::Grid::new("months")
            .striped(true)
            .num_columns(6)
            .spacing([24.0, 5.0])
            .show(ui, |ui| {
                for h in ["Month", "Rides", "Time", "Distance", "TSS", "Work"] {
                    ui.label(RichText::new(h).weak());
                }
                ui.end_row();
                let first = self
                    .store
                    .history
                    .iter()
                    .map(|h| h.start.date_naive())
                    .min()
                    .unwrap_or(today);
                for (m, t) in progress::monthly(&self.store.history, today, 12)
                    .into_iter()
                    .filter(|(m, _)| *m >= plan::first_of_month(first))
                {
                    ui.label(m.format("%B %Y").to_string());
                    ui.label(t.rides.to_string());
                    ui.label(fmt_hours(t.secs));
                    ui.label(format!("{:.0} km", t.km));
                    ui.label(format!("{:.0}", t.tss));
                    ui.label(format!("{:.0} kJ", t.kj));
                    ui.end_row();
                }
            });
    }

    fn history_tab(&mut self, ui: &mut egui::Ui) {
        let mut open: Option<usize> = None;
        let mut upload: Option<(String, String)> = None;
        ui.horizontal(|ui| {
            ui.heading("History");
            ui.label(
                RichText::new(format!(
                    "Activities are saved as .tcx in {}",
                    self.paths.activities.display()
                ))
                .weak(),
            );
            if ui.button("Open folder").clicked() {
                let _ = std::fs::create_dir_all(&self.paths.activities);
                crate::platform::open(&self.paths.activities);
            }
        });
        ui.label(
            RichText::new(
                "Upload the .tcx files to Strava, intervals.icu, TrainingPeaks or GoldenCheetah.",
            )
            .weak(),
        );
        ui.add_space(6.0);

        if !self.store.history.is_empty()
            && ui
                .link("Trends, power curve and totals are in Progress")
                .clicked()
        {
            self.tab = Tab::Progress;
        }
        ui.add_space(8.0);
        let mut delete = None;
        if self.store.history.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No rides yet").size(18.0));
                ui.label(RichText::new("Your rides appear here after you press Finish. Click one to see its charts and zones.").weak());
            });
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("history")
                    .striped(true)
                    .num_columns(12)
                    .spacing([14.0, 6.0])
                    .show(ui, |ui| {
                        for h in [
                            "Date",
                            "Ride",
                            "Time",
                            "Distance",
                            "Avg W",
                            "NP",
                            "TSS",
                            "HR",
                            "Cadence",
                            "Zones (power / HR)",
                            "intervals.icu",
                            "",
                        ] {
                            ui.label(RichText::new(h).strong());
                        }
                        ui.end_row();
                        for (i, h) in self.store.history.iter().enumerate().rev() {
                            ui.label(h.start.format("%a %d %b %Y %H:%M").to_string());
                            let mut name = h.name.clone();
                            if let Some(f) = h.ftp_estimate {
                                name += &format!("  (FTP est. {f:.0} W)");
                            }
                            if ui
                                .link(name)
                                .on_hover_text("Show charts and details")
                                .clicked()
                            {
                                open = Some(i);
                            }
                            ui.label(fmt_dur(h.secs));
                            ui.label(format!("{:.1} km", h.distance_m / 1000.0));
                            ui.label(format!("{:.0}", h.avg_power));
                            ui.label(format!("{:.0}", h.np));
                            ui.label(format!("{:.0}", h.tss));
                            ui.label(h.avg_hr.map(|v| format!("{v:.0}")).unwrap_or("—".into()));
                            ui.label(
                                h.avg_cadence
                                    .map(|v| format!("{v:.0}"))
                                    .unwrap_or("—".into()),
                            );
                            let (r, _) =
                                ui.allocate_exact_size(Vec2::new(120.0, 16.0), Sense::hover());
                            if !h.power_zones.is_empty() {
                                charts::zone_bar(
                                    ui.painter(),
                                    Rect::from_min_size(r.min, Vec2::new(r.width(), 7.0)),
                                    &h.power_zones,
                                    power_zone_color,
                                );
                                charts::zone_bar(
                                    ui.painter(),
                                    Rect::from_min_size(
                                        r.min + Vec2::new(0.0, 9.0),
                                        Vec2::new(r.width(), 7.0),
                                    ),
                                    &h.hr_zones,
                                    charts::hr_zone_color,
                                );
                            }
                            match &h.icu_id {
                                Some(id) => {
                                    ui.hyperlink_to(
                                        "Uploaded",
                                        format!("https://intervals.icu/activities/{id}"),
                                    );
                                }
                                None if self.icu_creds.is_set() => {
                                    if ui.small_button("Upload").clicked() {
                                        upload = Some((h.file.clone(), h.name.clone()));
                                    }
                                }
                                None => {
                                    ui.label(RichText::new("—").weak());
                                }
                            }
                            ui.horizontal(|ui| {
                                if ui
                                    .small_button("Show file")
                                    .on_hover_text(&h.file)
                                    .clicked()
                                {
                                    let dir = Path::new(&h.file)
                                        .parent()
                                        .map(|p| p.to_path_buf())
                                        .unwrap_or_default();
                                    crate::platform::open(dir);
                                }
                                if ui
                                    .small_button("Delete")
                                    .on_hover_text(format!(
                                        "Delete this ride (its file goes to the {TRASH})"
                                    ))
                                    .clicked()
                                {
                                    delete = Some(i);
                                }
                            });
                            ui.end_row();
                        }
                    });
            });
        if let Some(i) = open {
            let summary = self.store.history[i].clone();
            let activity = std::fs::read_to_string(&summary.file)
                .map_err(|e| format!("Could not open {}: {e}", summary.file))
                .and_then(|x| crate::activity::parse_tcx(&x).map_err(|e| e.to_string()));
            self.details = Some(RideDetails { summary, activity });
        }
        if let Some(i) = delete {
            self.modal = Some(Modal::ConfirmDelete(i));
        }
        if let Some((file, name)) = upload {
            self.icu_upload(file, name);
        }
    }

    // ------------------------------------------------------------------
    // Devices

    fn devices_tab(&mut self, ui: &mut egui::Ui) {
        ui.columns(2, |cols| {
            let ui = &mut cols[0];
            egui::ScrollArea::vertical().id_salt("devices-left").auto_shrink([false, false]).show(ui, |ui| {
                ui.heading("Bluetooth devices");
                match &self.adapter {
                    None => {
                        ui.label("Starting Bluetooth…");
                    }
                    Some(Ok(name)) => {
                        ui.label(RichText::new(format!("Adapter: {name}")).weak());
                    }
                    Some(Err(e)) => {
                        ui.colored_label(Color32::from_rgb(240, 120, 90), format!("Bluetooth unavailable: {e}"));
                    }
                }
                ui.horizontal(|ui| {
                    if self.scanning {
                        ui.spinner();
                        if ui.button("Stop scan").clicked() {
                            self.dev.send(Cmd::StopScan);
                        }
                    } else if ui.button(RichText::new("Scan for devices").strong()).clicked() {
                        self.dev.send(Cmd::StartScan);
                    }
                    ui.checkbox(&mut self.show_all_devices, "Show all Bluetooth devices");
                });
                ui.add_space(6.0);
                let (shown, hidden): (Vec<DeviceInfo>, Vec<DeviceInfo>) =
                    self.found.iter().cloned().partition(|d| self.show_all_devices || is_fitness_device(d));
                for d in shown {
                    self.device_card(ui, &d);
                }
                if !hidden.is_empty() {
                    ui.label(RichText::new(format!("{} other Bluetooth devices hidden (phones, TVs, …)", hidden.len())).weak());
                }
                if self.found.is_empty() && !self.scanning {
                    ui.label(RichText::new("Wake your trainer by pedalling, then press Scan.").weak());
                }

                ui.add_space(12.0);
                ui.heading("Connections");
                if self.links.is_empty() {
                    ui.label(RichText::new("Nothing connected yet.").weak());
                }
                let ids: Vec<String> = self.links.keys().cloned().collect();
                for id in ids {
                    self.connection_card(ui, &id);
                }

                ui.add_space(12.0);
                ui.heading("Simulated trainer");
                ui.label(RichText::new("Try everything without hardware. The simulator follows ERG targets and grade like a real trainer. Real sensors always take priority over it.").weak());
                let sim_on = self.links.get(SIM_ID).is_some_and(|l| l.link == Link::Connected);
                ui.horizontal(|ui| {
                    if ui.button(if sim_on { "Disable simulator" } else { "Enable simulator" }).clicked() {
                        self.dev.send(Cmd::SimEnable(!sim_on));
                    }
                    ui.label("Rider effort");
                    if ui.add(egui::Slider::new(&mut self.sim_effort, 50.0..=450.0).suffix(" W")).changed() {
                        self.dev.send(Cmd::SimEffort(self.sim_effort));
                    }
                });
            });

            let ui = &mut cols[1];
            egui::ScrollArea::vertical().id_salt("devices-right").auto_shrink([false, false]).show(ui, |ui| {
                ui.heading("Live data");
                let source = |m: &Option<Metric>| -> String {
                    m.as_ref()
                        .filter(|m| m.is_fresh())
                        .map(|m| self.links.get(&m.src).map(|l| l.name.clone()).unwrap_or_else(|| m.src.clone()))
                        .unwrap_or_default()
                };
                let row = |ui: &mut egui::Ui, k: &str, v: Option<f32>, unit: &str, src: String| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(k).weak());
                        ui.label(RichText::new(v.map(|v| format!("{v:.0} {unit}")).unwrap_or("—".into())).size(20.0).strong());
                        if !src.is_empty() {
                            ui.label(RichText::new(format!("from {src}")).weak());
                        }
                    });
                };
                row(ui, "Power", fresh(&self.metrics.power), "W", source(&self.metrics.power));
                row(ui, "Cadence", fresh(&self.metrics.cadence), "rpm", source(&self.metrics.cadence));
                if !self.has_cadence() {
                    ui.label(RichText::new("No cadence sensor connected. Cadence is optional: pair a cadence sensor, or a power meter that measures it, to see rpm.").weak());
                }
                row(ui, "Heart rate", fresh(&self.metrics.hr), "bpm", source(&self.metrics.hr));
                if fresh(&self.metrics.hr).is_none() && self.per_device.values().any(|d| d.hr_noisy == Some(true)) {
                    ui.colored_label(Color32::from_rgb(240, 150, 80), "Heart rate hidden: the strap's signal is noisy (see its card on the left).");
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.heading("Log");
                    if ui.button("Copy log").on_hover_text("Copy the log to paste it into a message").clicked() {
                        let text: Vec<&str> = self.log.iter().rev().map(|s| s.as_str()).collect();
                        ui.ctx().copy_text(text.join("\n"));
                        self.toast = Some(("Log copied to clipboard".into(), Instant::now()));
                    }
                    if ui.button("Open full log").on_hover_text(self.paths.log_file().display().to_string()).clicked() {
                        crate::platform::open(self.paths.log_file());
                    }
                    if ui.button("Save log to file").clicked() {
                        let text: Vec<&str> = self.log.iter().rev().map(|s| s.as_str()).collect();
                        let path = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default().join("veloforge-log.txt");
                        self.toast = Some(match std::fs::write(&path, text.join("\n")) {
                            Ok(()) => (format!("Log saved to {}", path.display()), Instant::now()),
                            Err(e) => (format!("Could not save log: {e}"), Instant::now()),
                        });
                    }
                    if ui.checkbox(&mut self.raw_log, "Log raw sensor data").on_hover_text("Record every packet from your sensors, useful for diagnosing odd readings").changed() {
                        self.dev.send(Cmd::RawLog(self.raw_log));
                    }
                });
                for l in &self.log {
                    ui.label(RichText::new(l).monospace().small());
                }
            });
        });
    }

    fn device_card(&mut self, ui: &mut egui::Ui, d: &DeviceInfo) {
        let link = self.links.get(&d.id).map(|l| l.link);
        egui::Frame::new()
            .fill(CARD)
            .corner_radius(6)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&d.name).strong());
                    if let Some(r) = d.rssi {
                        ui.label(RichText::new(format!("{r} dBm")).weak());
                    }
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| match link {
                            Some(Link::Connected)
                            | Some(Link::Connecting)
                            | Some(Link::Reconnecting) => {
                                if ui.button("Disconnect").clicked() {
                                    self.dev.send(Cmd::Disconnect(d.id.clone()));
                                    self.store.settings.auto_connect.retain(|x| *x != d.id);
                                    self.mark_dirty();
                                }
                            }
                            _ => {
                                if ui.button("Connect").clicked() {
                                    self.dev.send(Cmd::Connect(d.id.clone()));
                                }
                            }
                        },
                    );
                });
                let caps: Vec<&str> = d.caps.iter().map(|c| c.label()).collect();
                if !caps.is_empty() {
                    ui.label(
                        RichText::new(caps.join(" · "))
                            .small()
                            .color(Color32::from_rgb(120, 190, 240)),
                    );
                }
            });
    }

    fn connection_card(&mut self, ui: &mut egui::Ui, id: &str) {
        let Some(l) = self.links.get(id) else { return };
        let (txt, c) = match l.link {
            Link::Connected => ("connected", Color32::from_rgb(90, 210, 120)),
            Link::Connecting => ("connecting…", Color32::from_rgb(230, 200, 90)),
            Link::Reconnecting => ("reconnecting…", Color32::from_rgb(230, 160, 70)),
            Link::Disconnected => ("disconnected", Color32::GRAY),
        };
        let live = self.per_device.get(id);
        let reading = |m: Option<&Option<Metric>>, unit: &str| {
            m.and_then(fresh).map(|v| format!("{v:.0} {unit}"))
        };
        let data: Vec<String> = [
            reading(live.map(|d| &d.power), "W"),
            reading(live.map(|d| &d.cadence), "rpm"),
            reading(live.map(|d| &d.hr), "bpm"),
            live.and_then(|d| fresh(&d.speed))
                .map(|v| format!("{v:.1} km/h")),
        ]
        .into_iter()
        .flatten()
        .collect();
        let rate = live
            .map(|d| {
                d.updates
                    .iter()
                    .filter(|t| t.elapsed().as_secs_f32() <= 5.0)
                    .count() as f32
                    / 5.0
            })
            .unwrap_or(0.0);
        let silent_for = live
            .and_then(|d| d.updates.back())
            .map(|t| t.elapsed().as_secs());
        let mut disconnect = false;
        let mut open_calibration = false;
        egui::Frame::new().fill(CARD).corner_radius(6).inner_margin(8).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(&l.name).strong());
                ui.colored_label(c, txt);
                if l.controllable && l.link == Link::Connected {
                    ui.label(RichText::new("controls resistance").color(Color32::from_rgb(240, 150, 70)));
                    if ui.button("Calibrate").on_hover_text("Spindown calibration for accurate power").clicked() {
                        open_calibration = true;
                    }
                }
                if l.link != Link::Disconnected && id != SIM_ID {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        disconnect = ui.button("Disconnect").clicked();
                    });
                }
            });
            let caps: Vec<&str> = l.caps.iter().map(|c| c.label()).collect();
            if !caps.is_empty() {
                ui.label(RichText::new(caps.join(" · ")).small().color(Color32::from_rgb(120, 190, 240)));
            }
            if l.link == Link::Connected {
                if data.is_empty() {
                    let msg = match silent_for {
                        Some(s) => format!("No data for {s} s — pedal to wake it. See the log on the right."),
                        None => "No data yet — pedal to wake it. See the log on the right.".into(),
                    };
                    ui.label(RichText::new(msg).color(Color32::from_rgb(230, 170, 70)));
                } else {
                    ui.label(
                        RichText::new(format!("Live: {}  ·  {rate:.1} updates/s", data.join(" · "))).color(Color32::from_rgb(140, 220, 160)),
                    );
                }
                if l.caps.contains(&Capability::HeartRate) && id != SIM_ID {
                    let strap_hr = live.and_then(|d| fresh(&d.hr));
                    let beat_hr = live.and_then(|d| fresh(&d.hr_rr));
                    let contact = live.and_then(|d| d.hr_contact);
                    let mut parts = Vec::new();
                    match contact {
                        Some(true) => parts.push("skin contact: yes".to_string()),
                        Some(false) => parts.push("skin contact: NO".to_string()),
                        None => {}
                    }
                    if let Some(b) = beat_hr {
                        parts.push(format!("from beat timing: {b:.0} bpm"));
                    }
                    if !parts.is_empty() {
                        ui.label(RichText::new(parts.join("  ·  ")).weak());
                    }
                    let disagree = matches!((strap_hr, beat_hr), (Some(a), Some(b)) if (a - b).abs() > 20.0);
                    let noisy = live.and_then(|d| d.hr_noisy) == Some(true);
                    if noisy {
                        ui.colored_label(
                            Color32::from_rgb(240, 150, 80),
                            "The strap is detecting beats at random intervals, so it is measuring electrical noise rather than your heart. Its readings are ignored. Wet the electrodes, tighten the strap, and replace its battery (CR2032) if it is old.",
                        );
                    } else if contact == Some(false) {
                        ui.colored_label(
                            Color32::from_rgb(240, 150, 80),
                            "The strap is not touching your skin properly, so its readings are ignored. Wet the two electrode pads and tighten the strap.",
                        );
                    } else if disagree {
                        ui.colored_label(
                            Color32::from_rgb(240, 150, 80),
                            "The strap's heart rate and its beat timing disagree, so the reading is unreliable. Wet the electrodes, tighten the strap, and check its battery.",
                        );
                    }
                }
                if l.caps.is_empty() && id != SIM_ID {
                    ui.label(
                        RichText::new("No fitness services found on this device. Remove it from your system Bluetooth settings, then connect again here.")
                            .color(Color32::from_rgb(240, 120, 90)),
                    );
                }
            }
        });
        if open_calibration {
            self.calib = Some(CalibUi {
                stage: CalStage::Intro,
                targets: None,
                since: Instant::now(),
            });
        }
        if disconnect {
            self.dev.send(Cmd::Disconnect(id.to_string()));
            self.store.settings.auto_connect.retain(|x| x != id);
            self.mark_dirty();
        }
    }

    // ------------------------------------------------------------------
    // Settings

    fn calibration_window(&mut self, ctx: &egui::Context) {
        let Some(cal) = &self.calib else { return };
        let stage = cal.stage.clone();
        let targets = cal.targets;
        let waited = cal.since.elapsed().as_secs_f32();
        // Give up if the trainer goes quiet.
        let timeout = match stage {
            CalStage::Starting => 10.0,
            CalStage::SpeedUp | CalStage::Coast => 90.0,
            _ => f32::INFINITY,
        };
        if waited > timeout {
            self.dev.send(Cmd::Calibrate(false));
            self.calib = Some(CalibUi {
                stage: CalStage::Failed(
                    "The trainer stopped responding. Make sure it is awake and try again.".into(),
                ),
                targets,
                since: Instant::now(),
            });
            return;
        }
        let speed = fresh(&self.metrics.speed);
        let ride_active = self.ride_active();
        let legacy = self
            .trainer_link()
            .is_some_and(|l| l.caps.contains(&Capability::LegacyWahoo));
        let mut action: Option<&str> = None;
        egui::Window::new("Trainer calibration").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.set_width(420.0);
            let big = |ui: &mut egui::Ui, text: String, color: Color32| {
                ui.label(RichText::new(text).size(26.0).strong().color(color));
            };
            let speed_line = |ui: &mut egui::Ui| {
                ui.label(RichText::new(speed.map(|v| format!("{v:.1} km/h")).unwrap_or("— km/h".into())).size(40.0).strong());
            };
            match &stage {
                CalStage::Intro => {
                    ui.label("Calibration (a \"spindown\") makes your trainer's power readings accurate.");
                    ui.add_space(6.0);
                    ui.label("• Ride for about 10 minutes first so the tyre is warm.");
                    ui.label("• Check your tyre pressure and roller tension as usual.");
                    ui.label("• Redo it whenever you change tyre pressure, tyre or wheel.");
                    ui.add_space(6.0);
                    ui.label("You will be asked to speed up, then to stop pedalling and let the wheel coast.");
                    ui.add_space(8.0);
                    if legacy {
                        ui.colored_label(
                            Color32::from_rgb(240, 150, 80),
                            "Your trainer's firmware only supports Wahoo's older protocol, which VeloForge cannot calibrate. Run the spindown in the Wahoo app (the result is stored in the trainer and applies here too), or update the trainer's firmware there to enable calibration in VeloForge.",
                        );
                    }
                    if ride_active {
                        ui.colored_label(Color32::from_rgb(240, 150, 80), "Finish or discard the current ride first.");
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!ride_active && !legacy, egui::Button::new(RichText::new("Start calibration").strong())).clicked() {
                            action = Some("start");
                        }
                        if ui.button("Close").clicked() {
                            action = Some("close");
                        }
                    });
                }
                CalStage::Starting => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Asking the trainer to start calibration…");
                    });
                    if ui.button("Cancel").clicked() {
                        action = Some("cancel");
                    }
                }
                CalStage::SpeedUp => {
                    let goal = targets.map(|(_, hi)| format!("Pedal up to {hi:.0} km/h")).unwrap_or("Speed up until told to stop".into());
                    big(ui, goal, Color32::from_rgb(240, 200, 90));
                    speed_line(ui);
                    if let (Some((_, hi)), Some(v)) = (targets, speed) {
                        let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 10.0), Sense::hover());
                        ui.painter().rect_filled(r, 4.0, Color32::from_white_alpha(25));
                        let col = if v >= hi { Color32::from_rgb(90, 210, 120) } else { Color32::from_rgb(240, 200, 90) };
                        ui.painter().rect_filled(Rect::from_min_size(r.min, Vec2::new(r.width() * (v / hi).clamp(0.0, 1.0), r.height())), 4.0, col);
                    }
                    ui.label(RichText::new("Hold that speed until the screen says stop.").weak());
                    if ui.button("Cancel").clicked() {
                        action = Some("cancel");
                    }
                }
                CalStage::Coast => {
                    big(ui, "Stop pedalling now".into(), Color32::from_rgb(120, 200, 255));
                    ui.label("Let the wheel coast to a stop on its own, without braking.");
                    speed_line(ui);
                    if ui.button("Cancel").clicked() {
                        action = Some("cancel");
                    }
                }
                CalStage::Done => {
                    big(ui, "Calibration complete".into(), Color32::from_rgb(90, 210, 120));
                    ui.label("Your trainer's power readings are calibrated.");
                    if ui.button("Close").clicked() {
                        action = Some("close");
                    }
                }
                CalStage::Failed(msg) => {
                    big(ui, "Calibration did not finish".into(), Color32::from_rgb(240, 120, 90));
                    ui.label(msg);
                    ui.horizontal(|ui| {
                        if ui.button("Try again").clicked() {
                            action = Some("start");
                        }
                        if ui.button("Close").clicked() {
                            action = Some("close");
                        }
                    });
                }
            }
        });
        match action {
            Some("start") => {
                self.calib = Some(CalibUi {
                    stage: CalStage::Starting,
                    targets: None,
                    since: Instant::now(),
                });
                self.dev.send(Cmd::Calibrate(true));
            }
            Some("cancel") => {
                self.dev.send(Cmd::Calibrate(false));
                self.calib = None;
            }
            Some("close") => self.calib = None,
            _ => {}
        }
    }

    fn details_window(&mut self, ctx: &egui::Context) {
        let Some(d) = &self.details else { return };
        let mut open = true;
        let ftp = self.store.settings.ftp;
        let s = &d.summary;
        egui::Window::new(RichText::new(&s.name).strong())
            .id(egui::Id::unique("ride-details"))
            .open(&mut open)
            .collapsible(false)
            .default_size([960.0, 640.0])
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                // Never taller than the window it sits in: scroll instead.
                let room = (ui.ctx().content_rect().height() - 110.0).max(200.0);
                egui::ScrollArea::vertical()
                    .max_height(room)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(s.start.format("%A %d %B %Y, %H:%M").to_string()).weak(),
                        );
                        ui.add_space(4.0);
                        let ifac = if ftp > 0.0 { s.np / ftp } else { 0.0 };
                        ui.horizontal_wrapped(|ui| {
                            let stat = |ui: &mut egui::Ui, k: &str, v: String| {
                                egui::Frame::new()
                                    .fill(CARD)
                                    .corner_radius(6)
                                    .inner_margin(8)
                                    .show(ui, |ui| {
                                        ui.vertical(|ui| {
                                            ui.label(RichText::new(k).small().weak());
                                            ui.label(RichText::new(v).size(18.0).strong());
                                        });
                                    });
                            };
                            stat(ui, "Time", fmt_dur(s.secs));
                            stat(ui, "Distance", format!("{:.2} km", s.distance_m / 1000.0));
                            stat(ui, "Climbing", format!("{:.0} m", s.ascent_m));
                            stat(ui, "Avg power", format!("{:.0} W", s.avg_power));
                            stat(ui, "Normalized", format!("{:.0} W", s.np));
                            stat(ui, "Intensity", format!("{ifac:.2}"));
                            stat(ui, "TSS", format!("{:.0}", s.tss));
                            stat(ui, "Work", format!("{:.0} kJ", s.kj));
                            if let Some(h) = s.avg_hr {
                                stat(ui, "Avg HR", format!("{h:.0} bpm"));
                            }
                            if let Some(h) = s.max_hr {
                                stat(ui, "Max HR", format!("{h:.0} bpm"));
                            }
                            if let Some(c) = s.avg_cadence {
                                stat(ui, "Cadence", format!("{c:.0} rpm"));
                            }
                        });
                        ui.add_space(6.0);
                        match &d.activity {
                            Ok(a) => {
                                let (r, _) = ui.allocate_exact_size(
                                    Vec2::new(ui.available_width(), 230.0),
                                    Sense::hover(),
                                );
                                charts::ride_chart(ui.painter(), r, &a.samples, ftp, CARD);
                                ui.add_space(6.0);
                                ui.columns(2, |cols| {
                                    zone_breakdown(&mut cols[0], s);
                                    let ui = &mut cols[1];
                                    ui.add_space(6.0);
                                    ui.label(RichText::new("Best efforts").strong());
                                    let best = crate::activity::best_efforts(&a.samples);
                                    if best.is_empty() {
                                        ui.label(
                                            RichText::new(
                                                "Ride longer than 5 seconds to see best efforts.",
                                            )
                                            .weak(),
                                        );
                                    }
                                    egui::Grid::new("best")
                                        .num_columns(3)
                                        .spacing([18.0, 4.0])
                                        .show(ui, |ui| {
                                            for (label, _, w) in best {
                                                ui.label(label);
                                                ui.label(
                                                    RichText::new(format!("{w:.0} W")).strong(),
                                                );
                                                ui.label(
                                                    RichText::new(format!(
                                                        "{:.0}% FTP",
                                                        w / ftp.max(1.0) * 100.0
                                                    ))
                                                    .weak(),
                                                );
                                                ui.end_row();
                                            }
                                        });
                                });
                            }
                            Err(e) => {
                                ui.colored_label(Color32::from_rgb(240, 150, 80), e);
                            }
                        }
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui.button("Show file").on_hover_text(&s.file).clicked() {
                                let dir = Path::new(&s.file)
                                    .parent()
                                    .map(|p| p.to_path_buf())
                                    .unwrap_or_default();
                                crate::platform::open(dir);
                            }
                            ui.label(RichText::new(&s.file).small().weak());
                        });
                    });
            });
        if !open {
            self.details = None;
        }
    }

    fn welcome_window(&mut self, ctx: &egui::Context) {
        if self.store.settings.onboarded || self.demo {
            return;
        }
        let mut action: Option<&str> = None;
        egui::Window::new(RichText::new("Welcome to VeloForge").size(20.0).strong())
            .id(egui::Id::unique("welcome"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.label("A few details make workouts, speed and heart rate zones fit you. You can change them any time in Settings.");
                ui.add_space(10.0);
                let s = &mut self.store.settings;
                egui::Grid::new("welcome-grid").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
                    ui.label("FTP");
                    ui.add(egui::DragValue::new(&mut s.ftp).range(50.0..=600.0).suffix(" W"));
                    ui.end_row();
                    ui.label("");
                    ui.label(
                        RichText::new("The power you can hold for about an hour. Not sure? Start with 2.5 × your weight, then ride the Ramp Test.")
                            .small()
                            .weak(),
                    );
                    ui.end_row();
                    ui.label("Your weight");
                    ui.add(egui::DragValue::new(&mut s.rider_kg).range(30.0..=200.0).speed(0.1).suffix(" kg"));
                    ui.end_row();
                    ui.label("Max heart rate");
                    ui.add(egui::DragValue::new(&mut s.max_hr).range(120.0..=230.0).suffix(" bpm"));
                    ui.end_row();
                    ui.label("");
                    ui.label(RichText::new("Not sure? 220 minus your age is a rough start; VeloForge offers to update it after rides.").small().weak());
                    ui.end_row();
                    ui.label("Sounds");
                    ui.checkbox(&mut s.sounds, "Beep before interval changes");
                    ui.end_row();
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("Connect my trainer").strong()).fill(Color32::from_rgb(44, 112, 196))).clicked() {
                        action = Some("devices");
                    }
                    if ui.button("Try it with the simulator").clicked() {
                        action = Some("sim");
                    }
                    if ui.button("Skip").clicked() {
                        action = Some("skip");
                    }
                });
            });
        if let Some(a) = action {
            self.store.settings.onboarded = true;
            let s = &self.store.settings;
            self.dev.send(Cmd::SetMass(s.rider_kg + s.bike_kg));
            self.mark_dirty();
            match a {
                "devices" => {
                    self.tab = Tab::Devices;
                    self.dev.send(Cmd::StartScan);
                }
                "sim" => {
                    self.dev.send(Cmd::SimEnable(true));
                    self.tab = Tab::Ride;
                }
                _ => {}
            }
        }
    }

    fn modals(&mut self, ctx: &egui::Context) {
        self.welcome_window(ctx);
        self.calibration_window(ctx);
        self.details_window(ctx);
        let mut close = false;
        match &self.modal {
            Some(Modal::Saved(s)) => {
                let s = s.as_ref().clone();
                egui::Window::new("Ride saved")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.heading(&s.name);
                        egui::Grid::new("sum").num_columns(2).show(ui, |ui| {
                            for (k, v) in [
                                ("Time", fmt_dur(s.secs)),
                                ("Distance", format!("{:.2} km", s.distance_m / 1000.0)),
                                ("Climbing", format!("{:.0} m", s.ascent_m)),
                                ("Average power", format!("{:.0} W", s.avg_power)),
                                ("Normalized power", format!("{:.0} W", s.np)),
                                ("Training stress", format!("{:.0} TSS", s.tss)),
                                ("Work", format!("{:.0} kJ", s.kj)),
                            ] {
                                ui.label(k);
                                ui.label(RichText::new(v).strong());
                                ui.end_row();
                            }
                        });
                        zone_breakdown(ui, &s);
                        ui.add_space(4.0);
                        ui.label(RichText::new(&s.file).small().weak());
                        if let Some(m) = s.max_hr.filter(|m| *m > self.store.settings.max_hr) {
                            ui.separator();
                            ui.label(format!(
                                "New highest heart rate: {m:.0} bpm (max is set to {:.0}).",
                                self.store.settings.max_hr
                            ));
                            if ui.button(format!("Set max heart rate to {m:.0}")).clicked() {
                                self.store.settings.max_hr = m.round();
                                self.mark_dirty();
                            }
                        }
                        if let Some(f) = s.ftp_estimate {
                            ui.separator();
                            ui.label(
                                RichText::new(format!(
                                    "Estimated FTP: {f:.0} W (currently {:.0} W)",
                                    self.store.settings.ftp
                                ))
                                .size(16.0),
                            );
                            if ui.button(format!("Set FTP to {f:.0} W")).clicked() {
                                self.store.settings.ftp = f;
                                self.mark_dirty();
                                close = true;
                            }
                        }
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                    });
            }
            Some(Modal::ConfirmDelete(i)) => {
                let i = *i;
                let name = self
                    .store
                    .history
                    .get(i)
                    .map(|h| format!("{} on {}", h.name, h.start.format("%a %d %b")))
                    .unwrap_or_default();
                egui::Window::new("Delete ride?")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(format!("Delete {name}?"));
                        ui.label(
                            RichText::new(format!(
                                "Its .tcx file goes to the {TRASH}, so you can still restore it from there."
                            ))
                            .weak(),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Delete").clicked() {
                                if i < self.store.history.len() {
                                    let ride = self.store.history.remove(i);
                                    self.trash_activity(&ride.file);
                                    if self
                                        .details
                                        .as_ref()
                                        .is_some_and(|d| d.summary.file == ride.file)
                                    {
                                        self.details = None;
                                    }
                                    self.mark_dirty();
                                }
                                close = true;
                            }
                            if ui.button("Keep").clicked() {
                                close = true;
                            }
                        });
                    });
            }
            Some(Modal::ConfirmDiscard) => {
                egui::Window::new("Discard ride?")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label("This ride will not be saved.");
                        ui.horizontal(|ui| {
                            if ui.button("Discard").clicked() {
                                self.reset_ride();
                                close = true;
                            }
                            if ui.button("Keep riding").clicked() {
                                close = true;
                            }
                        });
                    });
            }
            None => {}
        }
        if close {
            self.modal = None;
        }
    }
}
