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
use crate::units::Units;
use crate::workout::{self, fmt_dur, Category, Step, Workout};
use crate::world::{Figure, Scene, Sky, TimeOfDay, Weather, World};
use chrono::{Datelike, Duration, Local, NaiveDate};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};
use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::time::Instant;

mod bench;
mod calendar_screen;
mod devices_screen;
mod history_screen;
mod hud;
mod progress_screen;
mod recording;
mod ride_screen;
mod segments;
mod settings_ui;
mod sync;
mod widgets;
mod windows;
mod workouts_screen;

use bench::Bench;
use widgets::*;
// The workout builder's number fields.
pub(crate) use widgets::{cadence_edit, pct_edit, secs_edit};

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
/// Store a reading. Real devices win over the simulated trainer, except
/// while the simulator is switched on (`sim_on`): then it takes over, so
/// a connected trainer that nobody is pedalling cannot hold the rider still.
fn set_metric(slot: &mut Option<Metric>, v: f32, src: &str, sim_on: bool) -> bool {
    let sim = src == SIM_ID;
    let held = |by_sim: bool| {
        slot.as_ref()
            .is_some_and(|m| (m.src == SIM_ID) == by_sim && m.is_fresh())
    };
    if (sim && !sim_on && held(false)) || (!sim && sim_on && held(true)) {
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
    /// A custom workout: its id and name.
    ConfirmDeleteWorkout(String, String),
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
    builder: crate::builder::Builder,
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
    strava: crate::strava::Worker,
    strava_creds: crate::strava::Credentials,
    /// The Client ID and Secret being typed in Settings.
    strava_edit: crate::strava::Credentials,
    strava_status: Option<Result<String, String>>,
    strava_busy: bool,
    /// Rides (by file) on their way to Strava.
    strava_pending: Vec<String>,
    /// The selected route's segments (worked out once per route).
    segments: (String, Vec<crate::segment::Segment>),
    seg_tracker: crate::segment::Tracker,
    /// Segments timed on this ride, saved with it.
    ride_efforts: Vec<crate::segment::Effort>,
    /// The latest segment result, shown in the banner for a while.
    seg_flash: Option<(Vec<BannerLine>, Instant)>,
    /// The ride's power has come from the simulated trainer: practice, not
    /// a real ride, so it is not saved.
    ride_simulated: bool,
    last_checkpoint: Instant,
    /// Rides whose power curve could not be computed (missing file).
    curve_failed: std::collections::HashSet<String>,
    /// "Keep the plan" was chosen for today's tiredness suggestion.
    tired_dismissed: Option<NaiveDate>,
    /// Started with --demo: skip the welcome.
    demo: bool,
    /// `--bench`: measuring how fast the 3D view draws.
    bench: Option<Bench>,
    /// The ride finishing by itself (its workout done, or a point-to-point
    /// finish reached): since when, and why.
    auto_finish: Option<(Instant, &'static str)>,
    /// A file being chosen in the system's file dialog (on its own
    /// thread): the file, or None if cancelled or there is no dialog.
    picking: Option<std::sync::mpsc::Receiver<Result<Option<std::path::PathBuf>, String>>>,
    /// "Keep riding" was chosen: this ride finishes when you say.
    keep_riding: bool,
    /// Where along the route the ride was last frame, to see the finish
    /// line crossed.
    last_route_pos: f64,
    /// When this frame's `logic` started and the last `ui` ended.
    frame_marks: (Option<Instant>, Option<Instant>),
    /// `VELOFORGE_STILL=metres`, for comparing screenshots: the 3D view
    /// holds the rider at that distance with its clock stopped.
    still: Option<f64>,
    graphics: String,
    egui_ctx: egui::Context,
    remote: Option<crate::remote::Remote>,
    remote_url: String,
    remote_error: Option<String>,
    remote_pushed: Instant,
    settings_page: settings_ui::SettingsPage,
    /// When you last changed gear (virtual shifting), for the flash on the
    /// gear shown.
    gear_flash: Option<Instant>,
    /// `--gears`: virtual shifting on for this run, whatever the setting.
    gears_arg: bool,
    /// Shift buttons (a Zwift Click) by device: presses, and when and which
    /// way the last one was.
    presses: std::collections::HashMap<String, (u32, Instant, bool)>,
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
            builder: Default::default(),
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
            strava: crate::strava::Worker::spawn(cc.egui_ctx.clone()),
            strava_creds: Default::default(),
            strava_edit: Default::default(),
            strava_status: None,
            strava_busy: false,
            strava_pending: Vec::new(),
            segments: (String::new(), Vec::new()),
            seg_tracker: Default::default(),
            ride_efforts: Vec::new(),
            seg_flash: None,
            ride_simulated: false,
            egui_ctx: cc.egui_ctx.clone(),
            remote: None,
            remote_url: String::new(),
            remote_error: None,
            remote_pushed: Instant::now(),
            settings_page: Default::default(),
            update: None,
            update_rx: None,
            gear_flash: None,
            gears_arg: false,
            presses: Default::default(),
            last_checkpoint: Instant::now(),
            curve_failed: Default::default(),
            demo: false,
            bench: None,
            auto_finish: None,
            picking: None,
            keep_riding: false,
            last_route_pos: 0.0,
            frame_marks: (None, None),
            still: std::env::var("VELOFORGE_STILL")
                .ok()
                .and_then(|v| v.parse().ok()),
            graphics: String::new(),
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
            app.graphics = format!("{renderer} (OpenGL {version})");
            app.log_line(format!("Graphics: {}", app.graphics));
        }
        app.strava_creds = crate::strava::Credentials::load(&app.paths.strava_file());
        app.strava_edit = app.strava_creds.clone();
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
        // `--bench [seconds]`: ride the Valley Loop (or `--route`, from
        // `--km`) with the simulator and measure how fast the 3D view
        // draws, then print it and quit.
        if let Some(i) = args.iter().position(|a| a == "--bench") {
            let secs = args
                .get(i + 1)
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(60.0)
                .clamp(5.0, 600.0);
            app.bench = Some(Bench::new(secs));
            app.store.settings.view_3d = true;
            if let Some(r) = app.routes.iter().position(|r| r.id == "valley-loop") {
                app.route_idx = r;
            }
            app.gpu.set_timing(true);
            // `--counts`: count what each part draws instead of timing it
            // undisturbed (the counting can slow the frames).
            app.gpu.set_counting(args.iter().any(|a| a == "--counts"));
        }
        if app.bench.is_some() || args.iter().any(|a| a == "--demo") {
            // `--route alpe`: ride another route (built in, imported or
            // random), by its id or any part of its name.
            if let Some(want) = args
                .iter()
                .position(|a| a == "--route")
                .map(|i| args.get(i + 1).cloned().unwrap_or_default())
            {
                match find_route(&app.routes, &want) {
                    Ok(r) => app.route_idx = r,
                    Err(why) => {
                        eprintln!("veloforge: --route {want:?}: {why}. The routes:");
                        for r in &app.routes {
                            eprintln!("  {}  ({})", r.name, r.id);
                        }
                        std::process::exit(2);
                    }
                }
            }
            // `--reverse`: ride it the other way.
            if args.iter().any(|a| a == "--reverse") && !app.routes[app.route_idx].is_reversed() {
                let i = app.route_idx;
                app.routes[i] = app.routes[i].reversed();
            }
            app.demo = true;
            app.dev.send(Cmd::SimEnable(true));
            // `--gears`: ride with virtual shifting (not saved as a setting).
            app.gears_arg = args.iter().any(|a| a == "--gears");
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
                let route = &app.routes[app.route_idx];
                if km < 0.0
                    || (route.shape() == route::Shape::PointToPoint && km * 1000.0 >= route.length)
                {
                    eprintln!(
                        "veloforge: --km {km}: {} is {:.1} km long",
                        route.name,
                        route.length / 1000.0
                    );
                    std::process::exit(2);
                }
                app.ride.distance = km * 1000.0;
                if let Some(b) = &mut app.bench {
                    b.from_km = km;
                }
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
        self.routes
            .extend(self.store.random_routes.iter().map(|r| r.route()));
        // The route chosen, the other way round if its id says so.
        let want = &self.store.settings.route_id;
        let (base, reversed) = match want.strip_suffix(route::REVERSED) {
            Some(base) => (base, true),
            None => (want.as_str(), false),
        };
        match self.routes.iter().position(|r| r.id == base) {
            Some(i) => {
                self.route_idx = i;
                if reversed {
                    self.routes[i] = self.routes[i].reversed();
                }
            }
            None => self.route_idx = 0,
        }
    }

    /// Ride the route with id `id` next (a route's id with
    /// `route::REVERSED` after it rides it the other way).
    fn choose_route(&mut self, id: String) {
        if id == self.routes[self.route_idx].id {
            return;
        }
        let swap_way = self.routes[self.route_idx].is_reversed() || id.ends_with(route::REVERSED);
        self.store.settings.route_id = id;
        if swap_way {
            // The routes are made again, this one the way chosen.
            self.rebuild_routes();
        } else if let Some(i) = self
            .routes
            .iter()
            .position(|r| r.id == self.store.settings.route_id)
        {
            self.route_idx = i;
        }
        self.mark_dirty();
        // A ghost belongs to the route it was ridden on.
        if matches!(self.ride.companion, Some(ride::Companion::Ghost(_))) {
            self.ride.companion = None;
        }
    }

    /// Make a new random route from the choices in Settings, add it to the
    /// routes and ride it next.
    pub(crate) fn new_random_route(&mut self) {
        let spec = crate::routegen::RandomRoute {
            seed: crate::routegen::fresh_seed(),
            ..self.store.settings.random_route.clone()
        };
        let id = spec.id();
        self.store.random_routes.push(spec);
        self.store.settings.route_id = id;
        self.rebuild_routes();
        // A ghost belongs to the route it was ridden on.
        if matches!(self.ride.companion, Some(ride::Companion::Ghost(_))) {
            self.ride.companion = None;
        }
        self.mark_dirty();
        let name = self
            .store
            .settings
            .units
            .route_name(&self.routes[self.route_idx].name);
        self.notify(format!("New route: {name}"));
    }

    fn library(&self) -> Vec<Workout> {
        let mut v = workout::builtin();
        v.extend(self.store.custom_workouts.iter().cloned());
        v
    }

    /// A saved workout ride's target steps in watts, with the FTP the ride
    /// was done at (worked out from its stored training load).
    fn ride_targets(&self, s: &Summary) -> Option<Vec<charts::TargetBlock>> {
        let w = self.find_workout(s.workout_id.as_deref()?)?;
        let ifac = (s.tss * 36.0 / s.secs.max(1.0)).sqrt();
        let ftp = if ifac > 0.05 {
            s.np / ifac
        } else {
            self.store.settings.ftp
        };
        Some(
            w.segments()
                .iter()
                .map(|g| charts::TargetBlock {
                    start: g.start,
                    end: g.end(),
                    from: g.from * ftp,
                    to: g.to * ftp,
                    free: g.free,
                })
                .collect(),
        )
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
        self.seg_tracker.reset();
        self.ride_efforts.clear();
        self.seg_flash = None;
        self.ride_simulated = false;
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
            Action::Camera => {
                let s = &mut self.store.settings;
                s.camera = s.camera.next();
                let label = s.camera.label();
                self.mark_dirty();
                self.notify(format!("Camera: {label}"));
            }
            Action::Finish => {
                if self.ride_active() {
                    self.finish_ride();
                }
            }
            Action::ShiftUp | Action::ShiftDown => self.shift(a == Action::ShiftUp),
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
                Event::Shift { id, up } => {
                    let p = self.presses.entry(id).or_insert((0, Instant::now(), up));
                    *p = (p.0 + 1, Instant::now(), up);
                    self.shift(up);
                }
                Event::Data { id, reading } => {
                    let now = Instant::now();
                    let sim_on = self.sim_on();
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
                        set_metric(&mut dev.power, p, &id, false);
                        if set_metric(&mut self.metrics.power, p, &id, sim_on) {
                            self.metrics.recent_power.push_back((now, p));
                            while self.metrics.recent_power.len() > 64 {
                                self.metrics.recent_power.pop_front();
                            }
                        }
                    }
                    if let Some(v) = reading.speed_kmh {
                        set_metric(&mut dev.speed, v, &id, false);
                        set_metric(&mut self.metrics.speed, v, &id, sim_on);
                    }
                    if let Some(c) = reading.cadence {
                        set_metric(&mut dev.cadence, c, &id, false);
                        set_metric(&mut self.metrics.cadence, c, &id, sim_on);
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
                        set_metric(&mut dev.hr_rr, rr, &id, false);
                    }
                    if let Some(h) = reading.hr {
                        set_metric(&mut dev.hr, h, &id, false);
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
                            set_metric(&mut self.metrics.hr, h, &id, sim_on);
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

    fn frame_wait(&self) -> std::time::Duration {
        if self.bench.is_some() {
            return std::time::Duration::ZERO;
        }
        match self.store.settings.fps_limit {
            0 => std::time::Duration::ZERO,
            fps => std::time::Duration::from_secs_f64(1.0 / fps as f64)
                .saturating_sub(self.last_frame.elapsed()),
        }
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
            drafting: s.drafting,
            flat_workouts: s.flat_workouts,
        };
        self.ride.gear = self.gears_on().then(|| crate::gears::clamp(s.gear));
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
        // A ride with the simulator on at any point is practice, even with
        // a real trainer connected too.
        if self.ride.state == ride::State::Running
            && (self.sim_on() || self.metrics.power.as_ref().is_some_and(|m| m.src == SIM_ID))
        {
            self.ride_simulated = true;
        }
        self.track_segments(live.power, live.hr);
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

    /// The trainer being ridden: the simulated one while it is switched on.
    fn trainer_link(&self) -> Option<&LinkInfo> {
        let usable = |l: &&LinkInfo| l.controllable && l.link == Link::Connected;
        self.links
            .get(SIM_ID)
            .filter(usable)
            .or_else(|| self.links.values().find(usable))
    }

    fn sim_on(&self) -> bool {
        self.links
            .get(SIM_ID)
            .is_some_and(|l| l.link == Link::Connected)
    }

    /// Seconds between the ride being done and it finishing by itself.
    const AUTO_FINISH_SECS: f32 = 10.0;
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

/// A line of the banner over the 3D view: text, size, colour, emphasised.
pub(crate) type BannerLine = (String, f32, Color32, bool);

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
        let now = Instant::now();
        if let (Some(b), (Some(start), Some(end))) = (&mut self.bench, self.frame_marks) {
            if end > start {
                let (prep, paint) = self.gpu.cpu_ms();
                b.cpu_frame(
                    end.duration_since(start).as_secs_f32() * 1000.0,
                    now.duration_since(end).as_secs_f32() * 1000.0,
                    prep,
                    paint,
                );
            }
        }
        self.frame_marks.0 = Some(now);
        self.pump_events();
        self.tick_ride();
        self.check_auto_finish();
        self.check_picked_file();
        self.pump_icu();
        self.pump_strava();
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

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // When benchmarking, mark on the graphics card where this frame's
        // drawing starts and ends, around the 3D view.
        let ctx = ui.ctx().clone();
        if self.bench.is_some() {
            self.gpu
                .mark_frame(&ctx.layer_painter(egui::LayerId::background()), false);
        }
        self.ui_frame(ui, frame);
        if self.bench.is_some() {
            let top = egui::LayerId::new(egui::Order::Debug, egui::Id::unique("bench-end"));
            self.gpu.mark_frame(&ctx.layer_painter(top), true);
        }
        self.frame_marks.1 = Some(Instant::now());
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
    fn ui_frame(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
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
            let (space, up, down, v, n, c) = ctx.input_mut(|i| {
                let none = egui::Modifiers::NONE;
                (
                    i.consume_key(none, egui::Key::Space),
                    i.consume_key(none, egui::Key::ArrowUp),
                    i.consume_key(none, egui::Key::ArrowDown),
                    i.consume_key(none, egui::Key::V),
                    i.consume_key(none, egui::Key::N),
                    i.consume_key(none, egui::Key::C),
                )
            });
            // Shifting: + (or =, the same key without Shift) or ], and - or [;
            // the number pad's + and - too.
            let (shift_up, shift_down) = ctx.input_mut(|i| {
                let none = egui::Modifiers::NONE;
                let mut up = 0;
                let mut down = 0;
                for k in [egui::Key::Plus, egui::Key::Equals, egui::Key::CloseBracket] {
                    up += i.count_and_consume_key(none, k);
                }
                for k in [egui::Key::Minus, egui::Key::OpenBracket] {
                    down += i.count_and_consume_key(none, k);
                }
                (up, down)
            });
            for _ in 0..shift_up {
                self.ride_action(crate::remote::Action::ShiftUp);
            }
            for _ in 0..shift_down {
                self.ride_action(crate::remote::Action::ShiftDown);
            }
            use crate::remote::Action;
            for (pressed, a) in [
                (c, Action::Camera),
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
        self.auto_finish_card(&ctx);

        if let Some((msg, at)) = &self.toast {
            if at.elapsed().as_secs_f32() < 4.0 {
                egui::Area::new(egui::Id::unique("toast"))
                    .anchor(Align2::CENTER_BOTTOM, [0.0, -24.0])
                    .show(&ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            // As wide as the message (wrapping only past 560
                            // points), not one word to a line.
                            let text = RichText::new(msg).size(15.0);
                            let width = egui::WidgetText::from(text.clone())
                                .into_galley(
                                    ui,
                                    Some(egui::TextWrapMode::Extend),
                                    f32::INFINITY,
                                    egui::FontSelection::Default,
                                )
                                .size()
                                .x;
                            ui.set_width(width.min(560.0));
                            ui.add(egui::Label::new(text).wrap());
                        });
                    });
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            } else {
                self.toast = None;
            }
        }

        if self.tab == Tab::Ride || self.ride.state == ride::State::Running {
            ctx.request_repaint_after(self.frame_wait());
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
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

    // ------------------------------------------------------------------
    // Workouts

    // ------------------------------------------------------------------
    // Calendar & plan

    // ------------------------------------------------------------------
    // History

    // ------------------------------------------------------------------
    // Devices

    // ------------------------------------------------------------------
    // Settings
}

/// The route `want` names (`--route`): the one with that id, else the
/// one whose name or id contains it, ignoring case and anything but letters
/// and digits (so "alpe" and "alpe d huez" both find "Alpe d'Huez", and
/// "alpe" an import named after its file, "gpx-alpe-d-huez").
fn find_route(routes: &[Route], want: &str) -> Result<usize, String> {
    if let Some(i) = routes.iter().position(|r| r.id == want) {
        return Ok(i);
    }
    let plain = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    let want = plain(want);
    if want.is_empty() {
        return Err("no route named".into());
    }
    let found: Vec<usize> = (0..routes.len())
        .filter(|&i| plain(&routes[i].name).contains(&want) || plain(&routes[i].id).contains(&want))
        .collect();
    match found[..] {
        [i] => Ok(i),
        [] => Err("no route has that id or name".into()),
        _ => Err(format!(
            "{} routes match: {}",
            found.len(),
            found
                .iter()
                .map(|&i| routes[i].name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_are_found_by_id_or_any_part_of_their_name() {
        let mut routes = route::builtin();
        let xml = std::fs::read_to_string("tests/data/alpe_d_huez.gpx").unwrap();
        let mut alpe = route::parse_gpx(&xml, "gpx-1234", "Alpe d'Huez").unwrap();
        // (The file calls itself "New file": named as a rider would.)
        alpe.name = "Alpe d'Huez".into();
        routes.push(alpe.to_route());
        let name = |w: &str| find_route(&routes, w).map(|i| routes[i].name.clone());
        assert!(name("hairpin-pass").unwrap().starts_with("Hairpin Pass"));
        assert!(name("valley").unwrap().starts_with("Valley Loop"));
        let alpe_name = routes.last().unwrap().name.clone();
        for w in ["alpe", "ALPE D HUEZ", "Alpe d'Huez", "gpx-1234", "1234"] {
            assert_eq!(name(w).unwrap(), alpe_name, "{w}");
        }
        // Imported before imports were named after their files: found by
        // the file's name in its id.
        let mut old = route::parse_gpx(&xml, "gpx-alpe-d-huez", "New file").unwrap();
        old.name = "New file".into();
        let mut with_old = route::builtin();
        with_old.push(old.to_route());
        let found = find_route(&with_old, "alpe").unwrap();
        assert_eq!(with_old[found].id, "gpx-alpe-d-huez");
        // Two routes have it in their names (Valley Loop, Col Loop): neither
        // is picked.
        let both = name("loop").unwrap_err();
        assert!(both.contains("2 routes match"), "{both}");
        assert!(name("nowhere").is_err());
        assert!(name("").is_err());
    }

    #[test]
    fn the_simulator_takes_over_only_while_it_is_on() {
        let mut slot = None;
        // A connected trainer reports 0 W: with the simulator off it wins...
        assert!(set_metric(&mut slot, 0.0, "kickr", false));
        assert!(!set_metric(&mut slot, 180.0, SIM_ID, false));
        assert_eq!(slot.as_ref().unwrap().v, 0.0);
        // ...with the simulator on, the simulator's power is used instead.
        assert!(set_metric(&mut slot, 180.0, SIM_ID, true));
        assert!(!set_metric(&mut slot, 0.0, "kickr", true));
        assert_eq!(slot.as_ref().unwrap().src, SIM_ID);
        // Switched off again, the real trainer is back at once.
        assert!(set_metric(&mut slot, 120.0, "kickr", false));
        assert_eq!(slot.as_ref().unwrap().src, "kickr");
    }
}
