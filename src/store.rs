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
    /// Start the ride by pedalling: a 10 s countdown, then it starts.
    pub start_by_pedalling: bool,
    pub jersey: [u8; 3],
    /// Your rider's skin tone in the 3D world.
    pub skin: [u8; 3],
    pub cda: f32,
    pub crr: f32,
    pub route_id: String,
    pub auto_connect: Vec<String>,
    /// Ride screen: 3D world (true) or numbers dashboard (false).
    pub view_3d: bool,
    /// Show the trainer's wheel speed rather than virtual speed.
    pub trainer_speed: bool,
    /// Riding close behind the pacer or a ghost saves effort.
    pub drafting: bool,
    /// Ride workouts as if the road were flat.
    pub flat_workouts: bool,
    /// Metric or imperial units on screen.
    pub units: crate::units::Units,
    pub max_hr: f32,
    /// Threshold heart rate (LTHR) in bpm, 0 while unknown.
    pub lthr: f32,
    /// What the heart rate zones are worked out from.
    pub hr_basis: HrBasis,
    /// Audio cues during workouts.
    pub sounds: bool,
    pub volume: f32,
    /// Render the 3D world on the GPU (false: simple software view).
    pub gpu_3d: bool,
    /// How much detail the GPU view draws.
    pub gfx_quality: crate::gfx::Quality,
    /// How sharply the 3D world is drawn, in percent of the view; 0 picks
    /// by `gfx_quality`.
    pub resolution: u8,
    /// Most frames per second on the ride screen; 0 = as often as the
    /// screen refreshes.
    pub fps_limit: u32,
    /// The first-run welcome has been completed.
    pub onboarded: bool,
    /// Download planned workouts from intervals.icu into the calendar.
    pub icu_sync_plan: bool,
    /// Upload finished rides to intervals.icu.
    pub icu_upload: bool,
    /// Upload finished rides to Strava (once connected).
    pub strava_upload: bool,
    /// Time of day and weather in the 3D world.
    pub time_of_day: crate::world::TimeOfDay,
    pub camera: crate::world::Camera,
    pub weather: crate::world::Weather,
    /// Serve the phone remote on the local network.
    pub remote_enabled: bool,
    pub remote_port: u16,
    /// Access code the phone must present (part of the QR code link).
    pub remote_token: String,
    /// Ask GitHub at start-up whether a newer version has been released.
    pub check_updates: bool,
    /// Ask the system not to dim or blank the screen during a ride.
    pub keep_awake: bool,
    /// What the next random route is made of (its seed is new each time).
    pub random_route: crate::routegen::RandomRoute,
    /// Virtual shifting, for a bike with one fixed cog on the trainer (a
    /// Zwift Cog): the gears are the app's, and set the trainer's power.
    pub virtual_shifting: bool,
    /// The virtual gear you are in (an index into the gears your gearing
    /// gives), kept from ride to ride.
    pub gear: usize,
    /// Your bike's gearing for virtual shifting: chainring teeth (one or
    /// two rings), cassette cogs smallest first, and the wheel and tyre
    /// (a name from `gears::TYRES`).
    pub chainrings: Vec<u8>,
    /// A Click with a working left pad has been seen: Home is steered with
    /// the pad, and Y, Z and A stay with the music.
    pub click_pad: bool,
    /// Keep a Click v2's left half going without Zwift, by restarting it
    /// about once a minute.
    pub click_restart: bool,
    /// Ride the classic bike rather than the aero road bike.
    pub classic_bike: bool,
    /// The aero bike's paint (sRGB).
    pub bike_colour: [u8; 3],
    /// Your jersey in polka dots: white, with dots in the jersey colour.
    pub jersey_dots: bool,
    /// The aero bike's tyres with tan sidewalls rather than all black.
    pub tan_walls: bool,
    /// White (true) or black kit: helmet, gloves, socks and shoes.
    pub kit_white: [bool; 4],
    pub cassette: Vec<u8>,
    pub tyre: String,
    /// The picture behind the screens: empty for Waves (a colour for each
    /// part of the app), or the file name of one of `pictures`.
    pub wallpaper: String,
    /// Pictures you added, copied into the data folder's `pictures`.
    pub pictures: Vec<String>,
    /// How dark the glass panels are, in percent.
    pub panel_darkness: u8,
    /// How much the picture is dimmed, in percent.
    pub dim_picture: u8,
    /// Text and controls larger than standard (1.0) for reading from the bike.
    pub text_size: f32,
    /// Figures while riding in bold rather than light.
    pub bold_figures: bool,
    /// A line about cycling under Home's greeting.
    pub home_quote: bool,
    /// Your music: the songs, from your music folder and added one by one,
    /// in your order.
    pub songs: Vec<crate::music::Song>,
    /// The folder your music is played from, when you chose one.
    pub music_folder: Option<PathBuf>,
    /// Songs in the music folder you hid from the list (their paths in it).
    pub music_hidden: Vec<String>,
    pub music_volume: f32,
    pub music_shuffle: bool,
    pub music_repeat: crate::music::Repeat,
    /// Pause the music while the ride is paused.
    pub music_with_ride: bool,
    /// Lower the music while a workout beep plays.
    pub music_duck: bool,
    /// The Click v2 buttons (their bits) for play/pause, next and
    /// previous; 0 for none. Y, A and Z unless you assign others.
    pub click_play: u64,
    pub click_next: u64,
    pub click_previous: u64,
}

/// What heart rate zones are worked out from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HrBasis {
    #[default]
    Max,
    Threshold,
}

impl Settings {
    /// Heart rate zones from max heart rate, or from threshold heart rate
    /// when chosen and known.
    pub fn hr_zones(&self) -> crate::charts::HrZones {
        match self.hr_basis {
            HrBasis::Threshold if self.lthr > 0.0 => {
                crate::charts::HrZones::from_threshold(self.lthr, self.max_hr)
            }
            _ => crate::charts::HrZones::from_max(self.max_hr),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            ftp: 200.0,
            rider_kg: 75.0,
            bike_kg: 9.0,
            difficulty: 0.5,
            auto_pause: true,
            start_by_pedalling: true,
            jersey: [230, 90, 40],
            skin: crate::world::SKIN_TONES[2].1,
            cda: 0.32,
            crr: 0.004,
            route_id: "valley-loop".into(),
            auto_connect: Vec::new(),
            view_3d: true,
            trainer_speed: true,
            drafting: true,
            flat_workouts: false,
            units: Default::default(),
            max_hr: 185.0,
            lthr: 0.0,
            hr_basis: HrBasis::Max,
            sounds: true,
            volume: 0.6,
            gpu_3d: true,
            gfx_quality: Default::default(),
            resolution: 0,
            fps_limit: 60,
            onboarded: false,
            icu_sync_plan: true,
            icu_upload: true,
            strava_upload: true,
            time_of_day: Default::default(),
            camera: Default::default(),
            weather: Default::default(),
            remote_enabled: false,
            remote_port: crate::remote::DEFAULT_PORT,
            remote_token: String::new(),
            check_updates: true,
            keep_awake: true,
            random_route: Default::default(),
            virtual_shifting: false,
            gear: crate::gears::START_GEAR,
            chainrings: vec![34, 50],
            click_pad: false,
            click_restart: true,
            classic_bike: false,
            bike_colour: [206, 32, 39],
            jersey_dots: false,
            tan_walls: false,
            kit_white: [true, false, true, true],
            cassette: vec![11, 13, 15, 17, 19, 21, 23, 25, 27, 30, 34],
            tyre: crate::gears::TYRES[0].0.to_string(),
            wallpaper: String::new(),
            pictures: Vec::new(),
            panel_darkness: 56,
            dim_picture: 22,
            text_size: 1.0,
            bold_figures: false,
            home_quote: true,
            songs: Vec::new(),
            music_folder: None,
            music_hidden: Vec::new(),
            music_volume: 0.7,
            music_shuffle: false,
            music_repeat: Default::default(),
            music_with_ride: true,
            music_duck: true,
            click_play: crate::zwift::Y,
            click_next: crate::zwift::A,
            click_previous: crate::zwift::Z,
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
    /// Random routes made, kept by what they were made from.
    pub random_routes: Vec<crate::routegen::RandomRoute>,
    /// FTP over time, one entry per day it changed.
    pub ftp_history: Vec<crate::progress::FtpEntry>,
    /// Every timed ride of a route's segments.
    pub efforts: Vec<crate::segment::Effort>,
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

    /// Where the songs you add are kept.
    pub fn music(&self) -> PathBuf {
        self.data.join("music")
    }

    /// Where pictures added for the background are kept.
    pub fn pictures(&self) -> PathBuf {
        self.data.join("pictures")
    }

    pub fn log_file(&self) -> PathBuf {
        self.data.join("veloforge.log")
    }

    /// intervals.icu credentials, kept apart from the rest and private.
    pub fn credentials_file(&self) -> PathBuf {
        self.data.join("credentials.json")
    }

    pub fn strava_file(&self) -> PathBuf {
        self.data.join("strava.json")
    }
}

impl Store {
    pub fn load(path: &Path) -> Store {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Store::default();
        };
        match serde_json::from_str(&text) {
            Ok(store) => store,
            Err(e) => {
                // Keep the file as it was, then read what can be read.
                eprintln!("veloforge: could not read all of {}: {e}", path.display());
                let backup = path.with_extension("json.bak");
                let _ = std::fs::copy(path, backup);
                let (store, lost) = Store::from_readable(&text);
                for part in &lost {
                    eprintln!("veloforge: left out {part} (kept in state.json.bak)");
                }
                store
            }
        }
    }

    /// The state in `text`, leaving out only the parts that cannot be
    /// read (each named in the list returned): a setting, or an entry in a
    /// list (a ride, a route, a workout), rather than everything. Every
    /// field has a default, so each part can be tried on its own.
    fn from_readable(text: &str) -> (Store, Vec<String>) {
        use serde_json::{Map, Value};
        let Ok(Value::Object(all)) = serde_json::from_str::<Value>(text) else {
            return (Store::default(), vec!["all of it (not JSON)".into()]);
        };
        let reads = |key: &str, v: Value| {
            let mut m = Map::new();
            m.insert(key.to_string(), v);
            serde_json::from_value::<Store>(Value::Object(m)).is_ok()
        };
        let mut kept = Map::new();
        let mut lost = Vec::new();
        for (key, v) in all {
            if reads(&key, v.clone()) {
                kept.insert(key, v);
                continue;
            }
            match v {
                // A list: keep the entries that read.
                Value::Array(items) => {
                    let mut good = Vec::new();
                    for (i, item) in items.into_iter().enumerate() {
                        if reads(&key, Value::Array(vec![item.clone()])) {
                            good.push(item);
                        } else {
                            lost.push(format!("{key}[{i}]"));
                        }
                    }
                    kept.insert(key, Value::Array(good));
                }
                // The settings: keep the ones that read.
                Value::Object(fields) => {
                    let mut good = Map::new();
                    for (field, x) in fields {
                        let mut one = Map::new();
                        one.insert(field.clone(), x.clone());
                        if reads(&key, Value::Object(one)) {
                            good.insert(field, x);
                        } else {
                            lost.push(format!("{key}.{field}"));
                        }
                    }
                    kept.insert(key, Value::Object(good));
                }
                _ => lost.push(key),
            }
        }
        let store = serde_json::from_value(Value::Object(kept)).unwrap_or_default();
        (store, lost)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::Scenery;

    /// State saved by a newer version, with a scenery this one does not
    /// know, keeps everything: the routes with that scenery read as
    /// farmland and hills.
    #[test]
    fn a_scenery_from_a_newer_version_keeps_everything_else() {
        let mut store = Store::default();
        store.settings.ftp = 287.0;
        let spec = crate::routegen::RandomRoute::default();
        store.random_routes.push(spec.clone());
        store.settings.random_route = spec;
        store.gpx_routes.push(GpxRoute {
            id: "gpx-1".into(),
            name: "Somewhere".into(),
            pts: vec![(0.0, 100.0, 0.0, 0.0), (5.0, 100.0, 5.0, 0.0)],
            scenery: Some(Scenery::Coast),
            shape: None,
        });
        let text = serde_json::to_string(&store)
            .unwrap()
            .replace("\"Hills\"", "\"Desert\"")
            .replace("\"Coast\"", "\"Desert\"");
        assert!(text.contains("Desert"));
        let back: Store = serde_json::from_str(&text).expect("reads it all");
        assert_eq!(back.settings.ftp, 287.0);
        assert_eq!(back.random_routes.len(), 1);
        assert_eq!(back.random_routes[0].scenery, Scenery::Hills);
        assert_eq!(back.settings.random_route.scenery, Scenery::Hills);
        assert_eq!(back.gpx_routes[0].scenery, Some(Scenery::Hills));
    }

    /// Anything else that cannot be read loses only itself: one setting,
    /// one entry in a list; and the file is kept as it was.
    #[test]
    fn what_cannot_be_read_loses_only_itself() {
        let mut store = Store::default();
        store.settings.ftp = 301.0;
        store.settings.rider_kg = 71.5;
        let spec = crate::routegen::RandomRoute::default();
        store.random_routes = vec![spec.clone(), spec.clone(), spec];
        let mut v = serde_json::to_value(&store).unwrap();
        // A setting with a value no version knows, and a random route
        // spoiled beyond reading.
        v["settings"]["time_of_day"] = "Twilight".into();
        v["random_routes"][1] = "not a route".into();
        let dir = std::env::temp_dir().join(format!("veloforge-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        std::fs::write(&path, v.to_string()).unwrap();
        let back = Store::load(&path);
        assert_eq!(back.settings.ftp, 301.0);
        assert_eq!(back.settings.rider_kg, 71.5);
        assert_eq!(back.settings.time_of_day, Settings::default().time_of_day);
        assert_eq!(back.random_routes.len(), 2);
        let kept = std::fs::read_to_string(dir.join("state.json.bak")).unwrap();
        assert!(kept.contains("Twilight") && kept.contains("not a route"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
