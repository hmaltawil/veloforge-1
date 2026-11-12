//! A ride in progress: physics for virtual speed, trainer control decisions,
//! 1 Hz recording and TCX export.

use crate::devices::Control;
use crate::route::Route;
use crate::workout::{self, Segment, Workout};
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Follow a structured workout in ERG mode.
    Workout,
    /// Resistance follows the route grade.
    Route,
    /// Manually chosen grade.
    Slope,
    /// Hold a target heart rate by adjusting ERG power.
    HeartRate,
}

/// Heart-rate mode controller: nudges power every few seconds, slowly,
/// because heart rate lags effort by a minute or more.
#[derive(Debug, Clone, Copy)]
pub struct HrControl {
    pub target_bpm: f32,
    /// Current power target in watts (None until the ride starts).
    pub power: Option<f32>,
    /// Seconds without a heart rate reading.
    pub missing: f64,
    last_adjust: f64,
}

impl HrControl {
    const PERIOD: f64 = 10.0;
    const MAX_UP: f32 = 6.0;
    const MAX_DOWN: f32 = 10.0;

    pub fn new(target_bpm: f32) -> HrControl {
        HrControl {
            target_bpm,
            power: None,
            missing: 0.0,
            last_adjust: 0.0,
        }
    }

    /// Advance the controller; returns the power target.
    pub fn update(&mut self, clock: f64, dt: f64, hr: Option<f32>, ftp: f32) -> f32 {
        let (lo, hi) = (0.35 * ftp, 1.05 * ftp);
        let p = *self.power.get_or_insert(0.5 * ftp);
        self.missing = if hr.is_some() { 0.0 } else { self.missing + dt };
        if clock - self.last_adjust < Self::PERIOD {
            return p;
        }
        self.last_adjust = clock;
        let Some(hr) = hr else { return p };
        let err = self.target_bpm - hr;
        // Ignore small wobbles; step up gently, back off a little faster.
        let step = if err.abs() < 2.0 {
            0.0
        } else {
            (err * 0.8).clamp(-Self::MAX_DOWN, Self::MAX_UP)
        };
        let p = (p + step).clamp(lo, hi);
        self.power = Some(p);
        p
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Ready,
    Running,
    Paused,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Live {
    pub power: f32,
    pub cadence: Option<f32>,
    pub hr: Option<f32>,
    /// Speed reported by the trainer (wheel speed on wheel-on trainers).
    pub speed_kmh: Option<f32>,
}

#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub time: DateTime<Utc>,
    /// Workout clock when the sample was taken (moving time plus skipped steps).
    pub elapsed: f32,
    pub power: f32,
    pub cadence: Option<f32>,
    pub hr: Option<f32>,
    pub speed: f32,
    pub distance: f64,
    pub ele: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct Physics {
    pub mass: f32,
    pub cda: f32,
    pub crr: f32,
    /// Use the trainer's reported speed when it has one, instead of
    /// computing a virtual speed from power and gradient.
    pub trainer_speed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub start: DateTime<Local>,
    pub name: String,
    pub workout_id: Option<String>,
    pub secs: f32,
    pub distance_m: f64,
    pub ascent_m: f64,
    pub avg_power: f32,
    pub np: f32,
    pub tss: f32,
    pub kj: f32,
    pub avg_hr: Option<f32>,
    pub avg_cadence: Option<f32>,
    pub file: String,
    #[serde(default)]
    pub ftp_estimate: Option<f32>,
    /// Seconds in each heart rate zone (Z1-Z5).
    #[serde(default)]
    pub hr_zones: Vec<f32>,
    /// Seconds in each power zone (Z1-Z6).
    #[serde(default)]
    pub power_zones: Vec<f32>,
    #[serde(default)]
    pub max_hr: Option<f32>,
    /// intervals.icu activity id once uploaded.
    #[serde(default)]
    pub icu_id: Option<String>,
    /// Best average power per power-curve duration: (seconds, watts).
    #[serde(default)]
    pub best: Vec<(u32, f32)>,
    /// Route the ride was on (for ghost rides).
    #[serde(default)]
    pub route_id: Option<String>,
}

/// Rides start just past the start/finish arch.
const START_OFFSET: f64 = 12.0;

/// Speed after `dt` seconds of riding at `power` watts on a `grade` percent
/// road: drive force against gravity, rolling resistance and aero drag.
pub fn virtual_speed(v: f64, power: f32, grade: f64, phys: &Physics, dt: f64) -> f64 {
    let theta = (grade / 100.0).atan();
    let m = phys.mass as f64;
    let drive = power as f64 * 0.97 / v.max(1.5);
    let resist = m * 9.81 * (theta.sin() + phys.crr as f64 * theta.cos())
        + 0.5 * 1.204 * phys.cda as f64 * v * v;
    let v = (v + (drive - resist) / m * dt).clamp(0.0, 30.0);
    if power < 1.0 && v < 0.6 {
        0.0
    } else {
        v
    }
}

/// A rider who rides at a steady power, with the same physics as you.
#[derive(Debug, Clone, PartialEq)]
pub struct Pacer {
    pub watts: f32,
    pub distance: f64,
    pub speed: f64,
}

/// A previous ride on the same route, replayed second by second.
#[derive(Debug, Clone, PartialEq)]
pub struct Ghost {
    /// Short description, e.g. "Fri 30 Oct".
    pub label: String,
    /// Distance at each second of moving time.
    pub track: Vec<f64>,
}

impl Ghost {
    pub fn from_samples(label: String, samples: &[Sample]) -> Option<Ghost> {
        let mut track: Vec<f64> = samples.iter().map(|s| s.distance).collect();
        // Keep it monotonic even if a file has a glitch.
        for i in 1..track.len() {
            track[i] = track[i].max(track[i - 1]);
        }
        (track.len() >= 10 && track.last().is_some_and(|d| *d > 200.0))
            .then_some(Ghost { label, track })
    }

    /// Distance after `t` seconds of moving time.
    pub fn distance_at(&self, t: f64) -> f64 {
        let i = t.max(0.0).floor() as usize;
        match (self.track.get(i), self.track.get(i + 1)) {
            (Some(a), Some(b)) => a + (b - a) * (t - i as f64),
            _ => self.track.last().copied().unwrap_or(0.0),
        }
    }

    /// Moving time at which the ghost passed `d` metres, if it got that far.
    pub fn time_at(&self, d: f64) -> Option<f64> {
        let i = self.track.partition_point(|&x| x < d);
        if i >= self.track.len() {
            return None;
        }
        if i == 0 {
            return Some(0.0);
        }
        let (a, b) = (self.track[i - 1], self.track[i]);
        let f = if b > a { (d - a) / (b - a) } else { 0.0 };
        Some(i as f64 - 1.0 + f)
    }

    pub fn duration(&self) -> f64 {
        self.track.len().saturating_sub(1) as f64
    }
}

/// Someone to ride with.
#[derive(Debug, Clone, PartialEq)]
pub enum Companion {
    Pacer(Pacer),
    Ghost(Ghost),
}

impl Companion {
    pub fn pacer(watts: f32) -> Companion {
        Companion::Pacer(Pacer {
            watts,
            distance: 0.0,
            speed: 0.0,
        })
    }

    /// The same companion back at the start line, for a new ride.
    pub fn restarted(&self) -> Companion {
        match self {
            Companion::Pacer(p) => Companion::pacer(p.watts),
            g => g.clone(),
        }
    }
}

/// What a summary needs besides the samples.
pub struct SummaryMeta {
    pub start: DateTime<Local>,
    pub name: String,
    pub workout_id: Option<String>,
}

/// Totals, training load and time in zones for 1 Hz samples.
pub fn summarize(
    samples: &[Sample],
    meta: SummaryMeta,
    ftp: f32,
    max_hr: f32,
    file: String,
) -> Summary {
    let p: Vec<f32> = samples.iter().map(|s| s.power).collect();
    let np = workout::normalized_power(&p);
    let secs = samples.len() as f32;
    let ifac = if ftp > 0.0 { np / ftp } else { 0.0 };
    let mean = |f: &dyn Fn(&Sample) -> Option<f32>| {
        let v: Vec<f32> = samples.iter().filter_map(f).filter(|v| *v > 0.0).collect();
        (!v.is_empty()).then(|| v.iter().sum::<f32>() / v.len() as f32)
    };
    let mut hr_zones = vec![0.0; 5];
    for h in samples.iter().filter_map(|s| s.hr).filter(|h| *h > 0.0) {
        hr_zones[crate::charts::hr_zone(h, max_hr)] += 1.0;
    }
    let mut power_zones = vec![0.0; 6];
    if ftp > 0.0 {
        for s in samples {
            power_zones[crate::charts::power_zone(s.power / ftp)] += 1.0;
        }
    }
    let ascent = samples
        .windows(2)
        .map(|w| w[1].ele - w[0].ele)
        .filter(|c| *c > 0.0 && *c < 5.0)
        .sum();
    Summary {
        start: meta.start,
        name: meta.name,
        workout_id: meta.workout_id,
        secs,
        distance_m: samples.last().map(|s| s.distance).unwrap_or(0.0),
        ascent_m: ascent,
        avg_power: p.iter().sum::<f32>() / secs.max(1.0),
        np,
        tss: secs / 3600.0 * ifac * ifac * 100.0,
        kj: p.iter().sum::<f32>() / 1000.0,
        avg_hr: mean(&|s| s.hr),
        avg_cadence: mean(&|s| s.cadence),
        file,
        ftp_estimate: None,
        hr_zones,
        power_zones,
        max_hr: samples
            .iter()
            .filter_map(|s| s.hr)
            .fold(None, |m, h| Some(m.map_or(h, |m: f32| m.max(h)))),
        icu_id: None,
        best: crate::progress::best_curve(samples),
        route_id: None,
    }
}

pub struct Ride {
    pub mode: Mode,
    pub workout: Option<Workout>,
    pub segs: Vec<Segment>,
    /// Date of the calendar entry this ride was started from.
    pub plan_date: Option<chrono::NaiveDate>,
    pub state: State,
    pub started: Option<DateTime<Local>>,
    /// Moving time.
    pub elapsed: f64,
    /// Workout time skipped with "skip step" (the workout clock runs ahead
    /// of moving time by this much).
    pub skipped: f64,
    pub distance: f64,
    pub speed: f64,
    pub ascent: f64,
    pub intensity: f32,
    pub manual_grade: f32,
    pub hr_control: HrControl,
    pub samples: Vec<Sample>,
    pub crank_angle: f32,
    /// Whether `speed` currently comes from the trainer.
    pub speed_from_trainer: bool,
    pub companion: Option<Companion>,
    pub companion_crank: f32,
    next_sample: f64,
    idle_for: f64,
    last_ele: Option<f64>,
    last_sent: Option<(Control, f64)>,
    clock: f64,
}

impl Ride {
    pub fn new(mode: Mode, workout: Option<Workout>, plan_date: Option<chrono::NaiveDate>) -> Ride {
        let segs = workout.as_ref().map(|w| w.segments()).unwrap_or_default();
        Ride {
            mode,
            workout,
            segs,
            plan_date,
            state: State::Ready,
            started: None,
            elapsed: 0.0,
            skipped: 0.0,
            distance: 0.0,
            speed: 0.0,
            ascent: 0.0,
            intensity: 1.0,
            manual_grade: 1.0,
            hr_control: HrControl::new(130.0),
            samples: Vec::new(),
            crank_angle: 0.0,
            speed_from_trainer: false,
            companion: None,
            companion_crank: 0.0,
            next_sample: 0.0,
            idle_for: 0.0,
            last_ele: None,
            last_sent: None,
            clock: 0.0,
        }
    }

    pub fn duration(&self) -> Option<f32> {
        self.workout.as_ref().map(|w| w.duration())
    }

    /// Position in the workout, in seconds.
    pub fn workout_time(&self) -> f64 {
        self.elapsed + self.skipped
    }

    pub fn workout_done(&self) -> bool {
        self.duration()
            .is_some_and(|d| self.workout_time() as f32 >= d)
    }

    /// Jump to the start of the next workout step.
    pub fn skip_step(&mut self) {
        let t = self.workout_time() as f32;
        if let Some(i) = workout::segment_index(&self.segs, t) {
            self.skipped += (self.segs[i].end() - t) as f64;
        }
    }

    /// Current target as (fraction of FTP, free ride).
    pub fn target(&self) -> Option<(f32, bool)> {
        if self.mode != Mode::Workout {
            return None;
        }
        workout::target_at(&self.segs, self.workout_time() as f32)
            .map(|(p, free)| (p * self.intensity, free))
    }

    pub fn target_watts(&self, ftp: f32) -> Option<f32> {
        if self.mode == Mode::HeartRate {
            return self.hr_control.power.map(f32::round);
        }
        self.target().filter(|t| !t.1).map(|t| (t.0 * ftp).round())
    }

    /// Jump the workout clock (used by `--demo --workout`).
    pub fn skip_to(&mut self, t: f64) {
        self.skipped = t - self.elapsed;
    }

    /// Position along the route (distance ridden plus the start offset).
    pub fn route_pos(&self) -> f64 {
        self.distance + START_OFFSET
    }

    /// The companion's distance ridden, in metres.
    pub fn companion_distance(&self) -> Option<f64> {
        match self.companion.as_ref()? {
            Companion::Pacer(p) => Some(p.distance),
            Companion::Ghost(g) => Some(g.distance_at(self.elapsed)),
        }
    }

    /// Metres the companion is ahead of you (negative: behind).
    pub fn companion_gap(&self) -> Option<f64> {
        self.companion_distance().map(|d| d - self.distance)
    }

    /// Seconds you are behind the ghost (negative: ahead). None when the
    /// ghost's ride was shorter than yours so far.
    pub fn ghost_time_gap(&self) -> Option<f64> {
        let Some(Companion::Ghost(g)) = &self.companion else {
            return None;
        };
        g.time_at(self.distance).map(|t| self.elapsed - t)
    }

    pub fn start(&mut self) {
        if self.started.is_none() {
            self.started = Some(Local::now());
        }
        self.state = State::Running;
    }

    /// Advance the ride by `dt` seconds. Returns a trainer command when one
    /// needs to be sent.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        dt: f64,
        live: Live,
        route: &Route,
        phys: Physics,
        ftp: f32,
        difficulty: f32,
        auto_pause: bool,
    ) -> Option<Control> {
        self.clock += dt;
        let road_grade = route.grade(self.route_pos());
        let running = self.state == State::Running;
        if running {
            let trainer = live.speed_kmh.filter(|_| phys.trainer_speed);
            self.speed_from_trainer = trainer.is_some();
            if let Some(kmh) = trainer {
                // Lightly smoothed so the world does not jerk between packets.
                let target = (kmh as f64 / 3.6).clamp(0.0, 30.0);
                self.speed += (target - self.speed) * (dt * 4.0).min(1.0);
            } else {
                self.speed = virtual_speed(self.speed, live.power, road_grade, &phys, dt);
            }
            let moving =
                live.power > 0.0 || live.cadence.is_some_and(|c| c > 0.0) || self.speed > 0.5;
            self.idle_for = if moving { 0.0 } else { self.idle_for + dt };
            let paused = auto_pause && self.idle_for > 2.0;
            if !paused {
                self.elapsed += dt;
                self.distance += self.speed * dt;
                let companion_speed = match &mut self.companion {
                    Some(Companion::Pacer(p)) => {
                        let g = route.grade(p.distance + START_OFFSET);
                        p.speed = virtual_speed(p.speed, p.watts, g, &phys, dt);
                        p.distance += p.speed * dt;
                        p.speed
                    }
                    Some(Companion::Ghost(g)) => {
                        g.distance_at(self.elapsed + 0.5) - g.distance_at(self.elapsed - 0.5)
                    }
                    None => 0.0,
                };
                // About 90 rpm at 32 km/h, slower on climbs.
                let rpm = (55.0 + companion_speed as f32 * 3.6).min(95.0);
                if companion_speed > 0.5 {
                    self.companion_crank = (self.companion_crank
                        + rpm / 60.0 * std::f32::consts::TAU * dt as f32)
                        % std::f32::consts::TAU;
                }
                let ele = route.at(self.route_pos()).ele;
                if let Some(prev) = self.last_ele {
                    // Ignore the jump when a point-to-point route wraps to its start.
                    let climb = ele - prev;
                    if climb > 0.0 && climb < 5.0 {
                        self.ascent += climb;
                    }
                }
                self.last_ele = Some(ele);
                if self.elapsed >= self.next_sample {
                    self.next_sample += 1.0;
                    self.samples.push(Sample {
                        time: Utc::now(),
                        elapsed: self.workout_time() as f32,
                        power: live.power,
                        cadence: live.cadence,
                        hr: live.hr,
                        speed: self.speed as f32,
                        distance: self.distance,
                        ele,
                    });
                }
            }
            let rpm = live
                .cadence
                .unwrap_or(if live.power > 0.0 { 85.0 } else { 0.0 });
            self.crank_angle = (self.crank_angle + rpm / 60.0 * std::f32::consts::TAU * dt as f32)
                % std::f32::consts::TAU;
        }

        let desired = match self.mode {
            Mode::Workout if running => match self.target() {
                Some((p, false)) => Control::Erg((p * ftp).round().clamp(0.0, 2000.0) as u16),
                _ => Control::Grade((road_grade as f32 * difficulty).clamp(-10.0, 20.0)),
            },
            // Before starting / while paused in a workout keep resistance light.
            Mode::Workout => Control::Grade(0.0),
            Mode::Route => Control::Grade((road_grade as f32 * difficulty).clamp(-10.0, 20.0)),
            Mode::Slope => Control::Grade(self.manual_grade),
            Mode::HeartRate if running => {
                let p = self.hr_control.update(self.clock, dt, live.hr, ftp);
                Control::Erg(p.round() as u16)
            }
            Mode::HeartRate => Control::Grade(0.0),
        };
        let send = match (self.last_sent, desired) {
            (None, _) => true,
            (Some((Control::Erg(a), t)), Control::Erg(b)) => a != b && self.clock - t >= 1.0,
            (Some((Control::Grade(a), t)), Control::Grade(b)) => {
                (a - b).abs() >= 0.1 && self.clock - t >= 1.0
            }
            _ => true,
        };
        if send {
            // Round grades so a rounding flicker does not resend.
            let desired = match desired {
                Control::Grade(g) => Control::Grade((g * 10.0).round() / 10.0),
                c => c,
            };
            self.last_sent = Some((desired, self.clock));
            Some(desired)
        } else {
            None
        }
    }

    /// The last command sent to the trainer.
    pub fn trainer_setting(&self) -> Option<Control> {
        self.last_sent.map(|(c, _)| c)
    }

    pub fn power_series(&self) -> Vec<f32> {
        self.samples.iter().map(|s| s.power).collect()
    }

    pub fn avg_power(&self) -> f32 {
        let n = self.samples.len().max(1) as f32;
        self.samples.iter().map(|s| s.power).sum::<f32>() / n
    }

    /// FTP estimate for test workouts.
    pub fn ftp_estimate(&self) -> Option<f32> {
        let id = self.workout.as_ref()?.id.as_str();
        let p = self.power_series();
        match id {
            "ramp-test" => workout::best_average(&p, 60).map(|b| (b * 0.75).round()),
            "ftp-20" => workout::best_average(&p, 20 * 60).map(|b| (b * 0.95).round()),
            _ => None,
        }
    }

    pub fn summary(&self, ftp: f32, max_hr: f32, file: String) -> Summary {
        let meta = SummaryMeta {
            start: self.started.unwrap_or_else(Local::now),
            name: self.title(),
            workout_id: self.workout.as_ref().map(|w| w.id.clone()),
        };
        let mut s = summarize(&self.samples, meta, ftp, max_hr, file);
        // The live totals are integrated more finely than the 1 Hz samples.
        s.distance_m = self.distance;
        s.ascent_m = self.ascent;
        s.ftp_estimate = self.ftp_estimate();
        s
    }

    pub fn title(&self) -> String {
        match (&self.workout, self.mode) {
            (Some(w), Mode::Workout) => w.name.clone(),
            (_, Mode::Slope) => "Free ride (manual slope)".into(),
            (_, Mode::HeartRate) => {
                format!("Heart rate ride ({:.0} bpm)", self.hr_control.target_bpm)
            }
            _ => "Free ride".into(),
        }
    }

    /// Garmin Training Center XML with power and speed extensions.
    pub fn to_tcx(&self) -> String {
        let start = self
            .samples
            .first()
            .map(|s| s.time)
            .unwrap_or_else(Utc::now);
        let ts = |t: DateTime<Utc>| t.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let secs = self.samples.len();
        let max_speed = self.samples.iter().map(|s| s.speed).fold(0.0, f32::max);
        // Mechanical kJ ≈ dietary kcal at typical cycling efficiency.
        let kcal = self.samples.iter().map(|s| s.power).sum::<f32>() / 1000.0;
        let mut x = String::new();
        x += "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n";
        x += "<TrainingCenterDatabase xmlns=\"http://www.garmin.com/xmlschemas/TrainingCenterDatabase/v2\" xmlns:ns3=\"http://www.garmin.com/xmlschemas/ActivityExtension/v2\">\n";
        x += " <Activities>\n  <Activity Sport=\"Biking\">\n";
        let _ = writeln!(x, "   <Id>{}</Id>", ts(start));
        let _ = writeln!(x, "   <Lap StartTime=\"{}\">", ts(start));
        let _ = writeln!(x, "    <TotalTimeSeconds>{secs}</TotalTimeSeconds>");
        let _ = writeln!(
            x,
            "    <DistanceMeters>{:.1}</DistanceMeters>",
            self.distance
        );
        let _ = writeln!(x, "    <MaximumSpeed>{max_speed:.2}</MaximumSpeed>");
        let _ = writeln!(x, "    <Calories>{}</Calories>", kcal.round() as u32);
        x += "    <Intensity>Active</Intensity>\n    <TriggerMethod>Manual</TriggerMethod>\n    <Track>\n";
        for s in &self.samples {
            x += "     <Trackpoint>";
            let _ = write!(x, "<Time>{}</Time>", ts(s.time));
            let _ = write!(x, "<AltitudeMeters>{:.1}</AltitudeMeters>", s.ele);
            let _ = write!(x, "<DistanceMeters>{:.1}</DistanceMeters>", s.distance);
            if let Some(hr) = s.hr.filter(|h| *h > 0.0) {
                let _ = write!(
                    x,
                    "<HeartRateBpm><Value>{}</Value></HeartRateBpm>",
                    hr.round() as u32
                );
            }
            if let Some(c) = s.cadence {
                let _ = write!(x, "<Cadence>{}</Cadence>", c.round().min(254.0) as u32);
            }
            let _ = write!(
                x,
                "<Extensions><ns3:TPX><ns3:Speed>{:.2}</ns3:Speed><ns3:Watts>{}</ns3:Watts></ns3:TPX></Extensions>",
                s.speed,
                s.power.round() as u32
            );
            x += "</Trackpoint>\n";
        }
        x += "    </Track>\n   </Lap>\n";
        let _ = writeln!(x, "   <Notes>{}</Notes>", xml_escape(&self.title()));
        x += "   <Creator xsi:type=\"Device_t\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"><Name>VeloForge</Name></Creator>\n";
        x += "  </Activity>\n </Activities>\n</TrainingCenterDatabase>\n";
        x
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route;

    fn phys() -> Physics {
        Physics {
            mass: 83.0,
            cda: 0.32,
            crr: 0.004,
            trainer_speed: true,
        }
    }

    #[test]
    fn steady_200w_on_flat_reaches_realistic_speed() {
        let r = &route::builtin()[1]; // lakeside, nearly flat
        let mut ride = Ride::new(Mode::Route, None, None);
        ride.start();
        for _ in 0..(120 * 20) {
            ride.update(
                0.05,
                Live {
                    power: 200.0,
                    cadence: Some(90.0),
                    hr: None,
                    speed_kmh: None,
                },
                r,
                phys(),
                250.0,
                0.5,
                true,
            );
        }
        let kmh = ride.speed * 3.6;
        assert!((28.0..38.0).contains(&kmh), "{kmh}");
        assert_eq!(ride.samples.len(), 120);
        assert!(ride.to_tcx().contains("<ns3:Watts>200</ns3:Watts>"));
    }

    #[test]
    fn workout_mode_sends_erg_targets_and_throttles() {
        let r = &route::builtin()[0];
        let wk = workout::builtin()
            .into_iter()
            .find(|w| w.id == "thr-2x20")
            .unwrap();
        let mut ride = Ride::new(Mode::Workout, Some(wk), None);
        let live = Live {
            power: 150.0,
            cadence: Some(90.0),
            hr: None,
            speed_kmh: None,
        };
        assert_eq!(
            ride.update(0.1, live, r, phys(), 250.0, 0.5, true),
            Some(Control::Grade(0.0))
        );
        ride.start();
        let first = ride.update(0.1, live, r, phys(), 250.0, 0.5, true);
        assert!(
            matches!(first, Some(Control::Erg(w)) if (110..=115).contains(&w)),
            "{first:?}"
        );
        // Inside the warmup ramp the target changes but sends are throttled to 1 Hz.
        let mut sent = 0;
        for _ in 0..100 {
            if ride
                .update(0.1, live, r, phys(), 250.0, 0.5, true)
                .is_some()
            {
                sent += 1;
            }
        }
        assert!(sent <= 10, "{sent}");
    }

    #[test]
    fn trainer_speed_overrides_virtual_speed() {
        let r = &route::builtin()[2]; // col loop: steep, virtual speed would be low
        let mut ride = Ride::new(Mode::Route, None, None);
        ride.start();
        let live = Live {
            power: 120.0,
            cadence: Some(85.0),
            hr: None,
            speed_kmh: Some(28.0),
        };
        for _ in 0..100 {
            ride.update(0.05, live, r, phys(), 250.0, 0.5, true);
        }
        assert!(ride.speed_from_trainer);
        assert!(
            (ride.speed * 3.6 - 28.0).abs() < 0.5,
            "{}",
            ride.speed * 3.6
        );
        // Turning the option off falls back to physics.
        let virt = Physics {
            trainer_speed: false,
            ..phys()
        };
        ride.update(0.05, live, r, virt, 250.0, 0.5, true);
        assert!(!ride.speed_from_trainer);
    }

    #[test]
    fn wrapping_point_to_point_route_does_not_add_fake_climbing() {
        // A 1.1 km climb of ~55 m; riding past the end wraps to the bottom.
        let mut xml = String::from("<gpx><trk><trkseg>");
        for i in 0..100 {
            xml += &format!(
                r#"<trkpt lat="{}" lon="7.0"><ele>{}</ele></trkpt>"#,
                45.0 + i as f64 * 0.0001,
                100.0 + i as f64 * 0.55
            );
        }
        xml += "</trkseg></trk></gpx>";
        let r = route::parse_gpx(&xml, "climb", "Climb").unwrap().to_route();
        let mut ride = Ride::new(Mode::Route, None, None);
        ride.start();
        let live = Live {
            power: 250.0,
            cadence: Some(90.0),
            hr: None,
            speed_kmh: Some(36.0),
        };
        // 36 km/h for 200 s = 2 km, i.e. almost two laps of the climb.
        for _ in 0..2000 {
            ride.update(0.1, live, &r, phys(), 250.0, 0.5, true);
        }
        assert!(ride.distance > r.length * 1.5);
        assert!(ride.ascent < 2.0 * 60.0, "{}", ride.ascent);
        assert!(ride.ascent > 60.0, "{}", ride.ascent);
    }

    #[test]
    fn skip_step_moves_the_workout_not_the_ride_time() {
        let r = &route::builtin()[1];
        let wk = workout::builtin()
            .into_iter()
            .find(|w| w.id == "ss-3x10")
            .unwrap();
        let mut ride = Ride::new(Mode::Workout, Some(wk), None);
        ride.start();
        let live = Live {
            power: 150.0,
            cadence: Some(90.0),
            hr: None,
            speed_kmh: None,
        };
        for _ in 0..100 {
            ride.update(0.1, live, r, phys(), 200.0, 0.5, true);
        }
        // Skip the 10 minute warm-up: now in the first sweet spot interval.
        ride.skip_step();
        assert!((ride.workout_time() - 600.0).abs() < 1e-6);
        assert!(ride.elapsed < 11.0);
        let c = ride.update(1.1, live, r, phys(), 200.0, 0.5, true);
        assert_eq!(c, Some(Control::Erg(176)));
        assert!(ride.samples.last().unwrap().elapsed >= 600.0);
    }

    #[test]
    fn heart_rate_mode_settles_on_target() {
        let r = &route::builtin()[1];
        let mut ride = Ride::new(Mode::HeartRate, None, None);
        ride.hr_control = HrControl::new(130.0);
        ride.start();
        // A rider whose heart rate follows power with a ~40 s lag:
        // steady state 60 + 0.45 W, so 130 bpm needs about 156 W.
        let mut hr = 70.0f32;
        let mut power = 0.0f32;
        let mut max_power = 0.0f32;
        for _ in 0..(20 * 60 * 10) {
            if let Some(Control::Erg(w)) = ride.update(
                0.1,
                Live {
                    power,
                    cadence: Some(90.0),
                    hr: Some(hr),
                    speed_kmh: None,
                },
                r,
                phys(),
                250.0,
                0.5,
                true,
            ) {
                power = w as f32;
            }
            max_power = max_power.max(power);
            hr += (60.0 + 0.45 * power - hr) * 0.1 / 40.0;
        }
        assert!((hr - 130.0).abs() < 4.0, "hr {hr}");
        assert!((power - 156.0).abs() < 15.0, "power {power}");
        assert!(max_power <= 250.0 * 1.05, "{max_power}");
    }

    #[test]
    fn heart_rate_mode_holds_power_without_a_signal() {
        let mut c = HrControl::new(140.0);
        let p0 = c.update(0.0, 0.1, None, 200.0);
        let p1 = c.update(60.0, 60.0, None, 200.0);
        assert_eq!(p0, p1);
        assert!(c.missing >= 60.0);
    }

    #[test]
    fn pacer_rides_with_the_same_physics() {
        let r = &route::builtin()[1];
        let mut ride = Ride::new(Mode::Route, None, None);
        ride.companion = Some(Companion::pacer(200.0));
        ride.start();
        let live = Live {
            power: 200.0,
            cadence: Some(90.0),
            hr: None,
            speed_kmh: None,
        };
        for _ in 0..(300 * 10) {
            ride.update(0.1, live, r, phys(), 250.0, 0.5, true);
        }
        // Same power, same physics: side by side.
        assert!(ride.companion_gap().unwrap().abs() < 1.0);
        let mut faster = Ride::new(Mode::Route, None, None);
        faster.companion = Some(Companion::pacer(250.0));
        faster.start();
        for _ in 0..(300 * 10) {
            faster.update(0.1, live, r, phys(), 250.0, 0.5, true);
        }
        let gap = faster.companion_gap().unwrap();
        assert!(gap > 50.0, "{gap}");
        assert_eq!(
            faster.companion.as_ref().map(Companion::restarted),
            Some(Companion::pacer(250.0))
        );
    }

    #[test]
    fn ghost_replays_a_previous_ride() {
        let samples: Vec<Sample> = (0..100)
            .map(|i| Sample {
                time: Utc::now(),
                elapsed: i as f32,
                power: 200.0,
                cadence: None,
                hr: None,
                speed: 8.0,
                distance: i as f64 * 8.0,
                ele: 0.0,
            })
            .collect();
        let g = Ghost::from_samples("Yesterday".into(), &samples).unwrap();
        assert_eq!(g.distance_at(10.5), 84.0);
        assert_eq!(g.distance_at(500.0), 792.0);
        assert_eq!(g.time_at(84.0), Some(10.5));
        assert_eq!(g.time_at(900.0), None);
        let r = &route::builtin()[1];
        let mut ride = Ride::new(Mode::Route, None, None);
        ride.companion = Some(Companion::Ghost(g));
        ride.start();
        let live = Live {
            power: 150.0,
            cadence: Some(90.0),
            hr: None,
            speed_kmh: Some(25.2), // 7 m/s, slower than the ghost
        };
        for _ in 0..600 {
            ride.update(0.1, live, r, phys(), 250.0, 0.5, true);
        }
        // A minute at 1 m/s slower: about 60 m, or 7.5 s at the ghost's speed.
        let gap = ride.companion_gap().unwrap();
        assert!((55.0..65.0).contains(&gap), "{gap}");
        let behind = ride.ghost_time_gap().unwrap();
        assert!((6.5..8.5).contains(&behind), "{behind}");
        assert!(Ghost::from_samples("x".into(), &samples[..5]).is_none());
    }

    #[test]
    fn trainer_difficulty_scales_the_grade_sent() {
        let r = &route::builtin()[2]; // col loop: a steady climb at the start
        let mut ride = Ride::new(Mode::Route, None, None);
        ride.start();
        let live = Live {
            power: 200.0,
            cadence: Some(90.0),
            hr: None,
            speed_kmh: Some(20.0),
        };
        let last = |ride: &mut Ride, difficulty: f32| {
            let mut sent = None;
            for _ in 0..30 {
                if let Some(Control::Grade(g)) =
                    ride.update(0.1, live, r, phys(), 250.0, difficulty, true)
                {
                    sent = Some(g);
                }
            }
            sent
        };
        let road = r.grade(ride.route_pos()) as f32;
        assert!(road > 2.0, "{road}");
        let half = last(&mut ride, 0.5).unwrap();
        let full = last(&mut ride, 1.0).unwrap();
        let none = last(&mut ride, 0.0).unwrap();
        assert!((half - road * 0.5).abs() < 0.3, "{half} vs {road}");
        assert!((full - road).abs() < 0.3, "{full} vs {road}");
        assert_eq!(none, 0.0);
    }

    #[test]
    fn auto_pause_stops_clock() {
        let r = &route::builtin()[1];
        let mut ride = Ride::new(Mode::Route, None, None);
        ride.start();
        for _ in 0..100 {
            ride.update(0.1, Live::default(), r, phys(), 250.0, 0.5, true);
        }
        assert!(ride.elapsed < 2.5);
    }
}
