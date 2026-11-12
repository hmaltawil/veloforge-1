//! Persistent state: settings, calendar, history, custom workouts and routes,
//! stored as JSON in the XDG data directory.

use crate::plan::PlannedWorkout;
use crate::ride::Summary;
use crate::route::GpxRoute;
use crate::workout::Workout;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub ftp: f32,
    pub rider_kg: f32,
    pub bike_kg: f32,
    /// Fraction of the route grade sent to the trainer.
    pub difficulty: f32,
    pub auto_pause: bool,
    pub jersey: [u8; 3],
    pub cda: f32,
    pub crr: f32,
    pub route_id: String,
    pub auto_connect: Vec<String>,
    /// Ride screen: 3D world (true) or numbers dashboard (false).
    pub view_3d: bool,
    /// Show the trainer's wheel speed rather than virtual speed.
    pub trainer_speed: bool,
    pub max_hr: f32,
    /// Audio cues during workouts.
    pub sounds: bool,
    pub volume: f32,
    /// Render the 3D world on the GPU (false: simple software view).
    pub gpu_3d: bool,
    /// How much detail the GPU view draws.
    pub gfx_quality: crate::gfx::Quality,
    /// The first-run welcome has been completed.
    pub onboarded: bool,
    /// Download planned workouts from intervals.icu into the calendar.
    pub icu_sync_plan: bool,
    /// Upload finished rides to intervals.icu.
    pub icu_upload: bool,
    /// Time of day and weather in the 3D world.
    pub time_of_day: crate::world::TimeOfDay,
    pub weather: crate::world::Weather,
    /// Serve the phone remote on the local network.
    pub remote_enabled: bool,
    pub remote_port: u16,
    /// Access code the phone must present (part of the QR code link).
    pub remote_token: String,
    /// Ask GitHub at start-up whether a newer version has been released.
    pub check_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            ftp: 200.0,
            rider_kg: 75.0,
            bike_kg: 9.0,
            difficulty: 0.5,
            auto_pause: true,
            jersey: [230, 90, 40],
            cda: 0.32,
            crr: 0.004,
            route_id: "valley-loop".into(),
            auto_connect: Vec::new(),
            view_3d: true,
            trainer_speed: true,
            max_hr: 185.0,
            sounds: true,
            volume: 0.6,
            gpu_3d: true,
            gfx_quality: Default::default(),
            onboarded: false,
            icu_sync_plan: true,
            icu_upload: true,
            time_of_day: Default::default(),
            weather: Default::default(),
            remote_enabled: false,
            remote_port: crate::remote::DEFAULT_PORT,
            remote_token: String::new(),
            check_updates: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    pub settings: Settings,
    pub plan: Vec<PlannedWorkout>,
    pub history: Vec<Summary>,
    pub custom_workouts: Vec<Workout>,
    pub gpx_routes: Vec<GpxRoute>,
    /// FTP over time, one entry per day it changed.
    pub ftp_history: Vec<crate::progress::FtpEntry>,
}

pub struct Paths {
    pub data: PathBuf,
    pub activities: PathBuf,
}

impl Paths {
    pub fn new() -> Paths {
        let data = std::env::var_os("VELOFORGE_DATA")
            .map(PathBuf::from)
            .or_else(|| {
                directories::ProjectDirs::from("", "", "veloforge")
                    .map(|p| p.data_dir().to_path_buf())
            })
            .unwrap_or_else(|| PathBuf::from(".veloforge"));
        let activities = data.join("activities");
        Paths { data, activities }
    }

    pub fn state_file(&self) -> PathBuf {
        self.data.join("state.json")
    }

    pub fn log_file(&self) -> PathBuf {
        self.data.join("veloforge.log")
    }

    /// intervals.icu credentials, kept apart from the rest and private.
    pub fn credentials_file(&self) -> PathBuf {
        self.data.join("credentials.json")
    }
}

impl Store {
    pub fn load(path: &Path) -> Store {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
                eprintln!(
                    "veloforge: could not parse {}: {e}; starting fresh",
                    path.display()
                );
                let backup = path.with_extension("json.bak");
                let _ = std::fs::copy(path, backup);
                Store::default()
            }),
            Err(_) => Store::default(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?).context("writing state")?;
        std::fs::rename(&tmp, path).context("replacing state")?;
        Ok(())
    }
}
