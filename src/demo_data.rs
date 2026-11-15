//! A made-up season of riding for the README screenshots: two months of a
//! training plan ridden with the app's own ride engine, saved the way the
//! app saves rides (TCX and FIT files, summaries, segment times, FTP
//! history), plus the weeks still to come.
//!
//! Run with
//!   VELOFORGE_DEMO_OUT=/some/empty/dir cargo test make_demo_data -- --ignored
//! and start VeloForge with VELOFORGE_DATA=/some/empty/dir.

use crate::plan::{self, Goal, PlanConfig};
use crate::ride::{Live, Mode, Physics, Ride};
use crate::store::{HrBasis, Store};
use crate::{route, segment, workout};
use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone, Utc};

/// A small deterministic random generator, so the season is the same each
/// time.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) as f32) / (1u64 << 31) as f32
    }
    /// About -1 to 1.
    fn noise(&mut self) -> f32 {
        self.next() * 2.0 - 1.0
    }
}

/// How the made-up rider rides a workout or a route.
enum Effort {
    /// Follow the workout's targets (free-ride steps at this fraction of FTP).
    Targets { free: f32 },
    /// The ramp test: follow it until the target passes `limit` watts.
    Ramp { limit: f32 },
    /// The 20-minute test: `watts` in its free-ride block.
    Twenty { watts: f32 },
    /// A free ride on the route at about this fraction of FTP, more uphill.
    Route { level: f32 },
}

#[allow(clippy::too_many_arguments)]
fn ride_one(
    store: &mut Store,
    dir: &std::path::Path,
    rng: &mut Rng,
    date: NaiveDate,
    workout: Option<workout::Workout>,
    route: &route::Route,
    effort: Effort,
    minutes: u32,
) {
    let s = &store.settings;
    let ftp = s.ftp;
    let mode = if workout.is_some() {
        Mode::Workout
    } else {
        Mode::Route
    };
    let mut ride = Ride::new(mode, workout, None);
    // Evenings in the week, mornings at the weekend.
    let hour = if date.weekday().number_from_monday() >= 6 {
        9
    } else {
        18
    };
    let start = Local
        .from_local_datetime(
            &date
                .and_hms_opt(hour, 5 + (rng.next() * 40.0) as u32, 0)
                .unwrap(),
        )
        .unwrap();
    ride.start();
    ride.started = Some(start);
    let phys = Physics {
        mass: s.rider_kg + s.bike_kg,
        cda: s.cda,
        crr: s.crr,
        trainer_speed: false,
        drafting: false,
        flat_workouts: false,
    };
    let segs = segment::for_route(route);
    let mut tracker = segment::Tracker::default();
    let mut hr = 72.0f32;
    let mut smooth = 0.0f32;
    let limit = minutes as f64 * 60.0;
    while ride.elapsed < limit && !ride.workout_done() {
        let target = ride.target_watts(ftp);
        let free = ride.target().is_some_and(|t| t.1);
        let mut watts = match effort {
            Effort::Targets { free: level } => target.unwrap_or(ftp * level),
            Effort::Ramp { limit } => match target {
                Some(t) if t > limit => break,
                Some(t) => t,
                None => ftp * 0.5,
            },
            Effort::Twenty { watts } => {
                if free {
                    watts
                } else {
                    target.unwrap_or(ftp * 0.6)
                }
            }
            Effort::Route { level } => {
                // A sprint every twelve minutes or so.
                if ride.elapsed as i64 % 730 > 718 {
                    ftp * 3.4
                } else {
                    let g = route.grade(ride.route_pos()) as f32;
                    ftp * (level + 0.035 * g).clamp(0.45, 1.05)
                }
            }
        };
        if free && !matches!(effort, Effort::Twenty { .. }) {
            watts = ftp * 0.7;
        }
        smooth += (watts - smooth) * if watts > smooth { 0.6 } else { 0.35 };
        let power = (smooth * (1.0 + 0.035 * rng.noise())).max(0.0);
        // Heart rate follows effort with a lag, and drifts up over long rides.
        let steady = 92.0 + 78.0 * (smooth / ftp) + ride.elapsed as f32 / 400.0;
        hr += (steady.min(s.max_hr - 2.0) - hr) / 35.0;
        let cadence = ride.target_cadence().unwrap_or(88.0) + 2.5 * rng.noise();
        let live = Live {
            power,
            cadence: Some(cadence),
            hr: Some(hr + rng.noise()),
            speed_kmh: None,
        };
        ride.update(1.0, live, route, phys, ftp, s.difficulty, false);
        for (i, secs, avg_power, avg_hr) in tracker.update(
            &segs,
            route.length,
            ride.route_pos(),
            ride.elapsed,
            power,
            Some(hr),
        ) {
            let seg = &segs[i];
            store.efforts.push(segment::Effort {
                route_id: route.id.clone(),
                key: seg.key.clone(),
                name: seg.name.clone(),
                date: start + Duration::seconds(ride.elapsed as i64),
                secs,
                avg_power,
                avg_hr,
            });
        }
    }
    // The samples carry the ride's own clock, not today's.
    let t0 = start.with_timezone(&Utc);
    for (i, sm) in ride.samples.iter_mut().enumerate() {
        sm.time = t0 + Duration::seconds(i as i64);
    }
    let slug: String = ride
        .title()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let file = dir.join(format!("{}_{}.tcx", start.format("%Y-%m-%d_%H%M%S"), slug));
    std::fs::write(&file, ride.to_tcx()).unwrap();
    std::fs::write(file.with_extension("fit"), ride.to_fit()).unwrap();
    let zones = store.settings.hr_zones();
    let mut summary = ride.summary(ftp, &zones, file.display().to_string());
    summary.route_id = Some(route.id.clone());
    if let Some(new_ftp) = summary.ftp_estimate {
        store.settings.ftp = new_ftp;
        store
            .ftp_history
            .push(crate::progress::FtpEntry { date, ftp: new_ftp });
    }
    if let Some(lthr) = summary.lthr_estimate {
        store.settings.lthr = lthr;
    }
    store.history.push(summary);
}

#[test]
#[ignore = "makes demo data for screenshots; run by hand"]
fn make_demo_data() {
    let out = std::path::PathBuf::from(
        std::env::var("VELOFORGE_DEMO_OUT").expect("set VELOFORGE_DEMO_OUT to an empty folder"),
    );
    let activities = out.join("activities");
    std::fs::create_dir_all(&activities).unwrap();
    let mut rng = Rng(20_261_115);
    let mut store = Store::default();
    {
        let s = &mut store.settings;
        s.ftp = 215.0;
        s.rider_kg = 72.0;
        s.max_hr = 188.0;
        s.hr_basis = HrBasis::Threshold;
        s.onboarded = true;
        s.check_updates = false;
        s.icu_upload = false;
        s.strava_upload = false;
        s.jersey = [214, 84, 34];
        s.gfx_quality = crate::gfx::Quality::High;
        s.fps_limit = 30;
    }
    let today = Local::now().date_naive();
    // Eight weeks of aerobic base, then an FTP builder that is under way.
    let base = today - Duration::days(114);
    let start = today - Duration::days(58);
    store.ftp_history.push(crate::progress::FtpEntry {
        date: base - Duration::days(20),
        ftp: 215.0,
    });
    let days = [false, true, false, true, false, true, true];
    store.plan = plan::generate(&PlanConfig {
        goal: Goal::Base,
        start: base,
        weeks: 8,
        days,
        long_day: 5,
    });
    store.plan.retain(|p| p.date < start);
    store.plan.extend(plan::generate(&PlanConfig {
        goal: Goal::FtpBuilder,
        start,
        weeks: 12,
        days,
        long_day: 5,
    }));
    let workouts = workout::builtin();
    let find = |id: &str| workouts.iter().find(|w| w.id == id).cloned();
    let routes = route::builtin();
    let mut ridden = 0;
    let planned: Vec<_> = store.plan.clone();
    for (i, p) in planned.iter().enumerate() {
        if p.date >= today {
            break;
        }
        // Life happens: a few planned rides are missed.
        if ridden % 11 == 7 && p.date < today - Duration::days(3) {
            ridden += 1;
            continue;
        }
        let route = &routes[(ridden * 3 + 1) % routes.len()];
        let first_of_block = ridden == 0 || (p.date >= start && store.ftp_history.len() < 3);
        let (w, effort) = if first_of_block {
            // Each block starts with a ramp test (FTP 222, then 232).
            let limit = if ridden == 0 { 297.0 } else { 311.0 };
            (find("ramp-test"), Effort::Ramp { limit })
        } else if p.date >= today - Duration::days(9) && store.settings.ftp < 240.0 {
            // ...and the 20-minute test near the end shows the progress.
            (find("ftp-20"), Effort::Twenty { watts: 258.0 })
        } else {
            (find(&p.workout_id), Effort::Targets { free: 0.7 })
        };
        let Some(w) = w else { continue };
        let minutes = (w.duration() / 60.0).ceil() as u32 + 1;
        ride_one(
            &mut store,
            &activities,
            &mut rng,
            p.date,
            Some(w),
            route,
            effort,
            minutes,
        );
        store.plan[i].done = true;
        ridden += 1;
    }
    // A free ride on one of the routes every Wednesday.
    let mut d = base;
    while d.weekday() != chrono::Weekday::Wed {
        d += Duration::days(1);
    }
    let mut k = 0;
    while d < today {
        let route = &routes[[2, 3, 1, 0][k % 4]];
        ride_one(
            &mut store,
            &activities,
            &mut rng,
            d,
            None,
            route,
            Effort::Route {
                level: 0.66 + 0.02 * (k % 3) as f32,
            },
            70 + 10 * (k % 3) as u32,
        );
        d += Duration::days(7);
        k += 1;
    }
    store.history.sort_by_key(|h| h.start);
    store.settings.route_id = "col-loop".into();
    let json = serde_json::to_string_pretty(&store).unwrap();
    std::fs::write(out.join("state.json"), json).unwrap();
    println!(
        "{} rides, FTP {}, {} segment times in {}",
        store.history.len(),
        store.settings.ftp,
        store.efforts.len(),
        out.display()
    );
}
