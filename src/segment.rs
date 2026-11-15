//! Segments: the climbs on a route (and the whole lap), timed whenever you
//! ride them, with your personal records.
//!
//! Climbs are found from the route's elevation, so imported routes get them
//! too. Every route is ridden as laps, so positions repeat each lap.

use crate::route::Route;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Climb,
    /// Once round the route.
    Lap,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Stable within its route, to match efforts across rides.
    pub key: String,
    pub name: String,
    pub kind: Kind,
    /// Metres along the route; for a lap both are the start line.
    pub start: f64,
    pub end: f64,
    pub length: f64,
    /// Average gradient, percent (climbs).
    pub grade: f64,
}

impl Segment {
    /// "1.2 km at 5.4%" or "12.0 km" (in miles when imperial).
    pub fn describe(&self, units: crate::units::Units) -> String {
        let len = units.fmt_distance(self.length, 1);
        match self.kind {
            Kind::Climb => format!("{len} at {:.1}%", self.grade),
            Kind::Lap => len,
        }
    }
}

/// One timed ride of a segment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Effort {
    pub route_id: String,
    pub key: String,
    pub name: String,
    pub date: DateTime<Local>,
    pub secs: f32,
    pub avg_power: f32,
    #[serde(default)]
    pub avg_hr: Option<f32>,
}

/// Where a ride's laps (and lap segment) start: where the ride starts.
pub const START_LINE: f64 = 12.0;

const STEP: f64 = 10.0;

/// The route's climbs, in order, and its lap.
pub fn for_route(route: &Route) -> Vec<Segment> {
    let len = route.length;
    let n = (len / STEP).floor() as usize;
    let mut out = Vec::new();
    if n >= 20 {
        // Elevation every 10 m, smoothed over 100 m so bumps do not count.
        let raw: Vec<f64> = (0..=n).map(|i| route.at(i as f64 * STEP).ele).collect();
        let ele: Vec<f64> = (0..=n)
            .map(|i| {
                let (a, b) = (i.saturating_sub(5), (i + 5).min(n));
                raw[a..=b].iter().sum::<f64>() / (b - a + 1) as f64
            })
            .collect();
        // Rising stretches (2% or more over each 10 m)...
        let rising: Vec<bool> = (0..n).map(|i| ele[i + 1] - ele[i] >= 0.2).collect();
        let mut runs: Vec<(usize, usize)> = Vec::new();
        let mut i = 0;
        while i < n {
            if rising[i] {
                let a = i;
                while i < n && rising[i] {
                    i += 1;
                }
                runs.push((a, i));
            } else {
                i += 1;
            }
        }
        // ...joined across short, nearly flat breaks...
        let mut joined: Vec<(usize, usize)> = Vec::new();
        for r in runs {
            if let Some(last) = joined.last_mut() {
                let gap = r.0 - last.1;
                if gap as f64 * STEP <= 150.0 && ele[last.1] - ele[r.0] <= 3.0 {
                    last.1 = r.1;
                    continue;
                }
            }
            joined.push(r);
        }
        // ...that are long and steep enough to be a climb.
        for (a, b) in joined {
            let length = (b - a) as f64 * STEP;
            let gain = raw[b] - raw[a];
            let grade = gain / length * 100.0;
            if length >= 400.0 && gain >= 15.0 && grade >= 3.0 {
                out.push(Segment {
                    key: format!("climb-{}", a as f64 * STEP),
                    name: String::new(),
                    kind: Kind::Climb,
                    start: a as f64 * STEP,
                    end: b as f64 * STEP,
                    length,
                    grade,
                });
            }
        }
        let count = out.len();
        for (k, s) in out.iter_mut().enumerate() {
            s.name = if count == 1 {
                "The climb".into()
            } else {
                format!("Climb {}", k + 1)
            };
        }
    }
    out.push(Segment {
        key: "lap".into(),
        name: if route.is_loop() {
            "Lap".into()
        } else {
            "Whole route".into()
        },
        kind: Kind::Lap,
        start: START_LINE,
        end: START_LINE,
        length: len,
        grade: 0.0,
    });
    out
}

/// A segment being ridden.
#[derive(Debug, Clone)]
struct Open {
    seg: usize,
    /// Ride clock and position when it started.
    t0: f64,
    pos0: f64,
    power: f64,
    hr: f64,
    hr_secs: f64,
}

/// A segment just finished: (index into the segments, seconds, average
/// power, average heart rate).
pub type Finished = (usize, f32, f32, Option<f32>);

/// Times segments as the ride goes along.
#[derive(Debug, Default)]
pub struct Tracker {
    prev: Option<(f64, f64)>,
    open: Vec<Open>,
}

impl Tracker {
    pub fn reset(&mut self) {
        *self = Tracker::default();
    }

    /// The segment being ridden (other than the lap) and how far into it.
    pub fn current(&self, segs: &[Segment]) -> Option<(usize, f64, f64)> {
        let (t, pos) = self.prev?;
        self.open
            .iter()
            .find(|o| segs.get(o.seg).is_some_and(|s| s.kind == Kind::Climb))
            .map(|o| (o.seg, pos - o.pos0, t - o.t0))
    }

    /// Move to `pos` metres along the route (not wrapped) at ride time `t`
    /// seconds, riding at `power` watts and `hr` bpm. Returns the segments
    /// finished on the way.
    pub fn update(
        &mut self,
        segs: &[Segment],
        length: f64,
        pos: f64,
        t: f64,
        power: f32,
        hr: Option<f32>,
    ) -> Vec<Finished> {
        // A ride starting exactly on a start line starts that segment.
        let (t_prev, p_prev) = self.prev.unwrap_or((t, pos - 1e-3));
        self.prev = Some((t, pos));
        let moved = pos - p_prev;
        if length <= 0.0 || moved < 0.0 || moved > 300.0 {
            // A jump (a new route, or the clock went back): start afresh.
            self.open.clear();
            return Vec::new();
        }
        let dt = (t - t_prev).max(0.0);
        for o in &mut self.open {
            o.power += power as f64 * dt;
            if let Some(h) = hr.filter(|h| *h > 0.0) {
                o.hr += h as f64 * dt;
                o.hr_secs += dt;
            }
        }
        if moved == 0.0 {
            return Vec::new();
        }
        // Where the next copy of `x` (one per lap) after p_prev is, if passed.
        let crossed = |x: f64| {
            let k = ((p_prev - x) / length).floor() + 1.0;
            let at = x + k * length;
            (at <= pos).then(|| t_prev + dt * (at - p_prev) / moved)
        };
        let mut done = Vec::new();
        // Finish first, so a lap ends before the next one starts.
        self.open.retain(|o| {
            let s = &segs[o.seg];
            match crossed(s.end) {
                Some(at) if at > o.t0 => {
                    let secs = (at - o.t0) as f32;
                    let ridden = (at - o.t0).max(1e-3);
                    let avg_hr = (o.hr_secs > ridden * 0.5).then(|| (o.hr / o.hr_secs) as f32);
                    done.push((o.seg, secs, (o.power / ridden) as f32, avg_hr));
                    false
                }
                _ => true,
            }
        });
        for (i, s) in segs.iter().enumerate() {
            if let Some(at) = crossed(s.start) {
                if self.open.iter().all(|o| o.seg != i) {
                    let pos0 = p_prev + moved * (at - t_prev) / dt.max(1e-9);
                    self.open.push(Open {
                        seg: i,
                        t0: at,
                        pos0: if dt > 0.0 { pos0 } else { pos },
                        power: 0.0,
                        hr: 0.0,
                        hr_secs: 0.0,
                    });
                }
            }
        }
        done
    }
}

/// The fastest of `efforts` on segment `key` of route `route_id`.
pub fn best<'a>(efforts: &'a [Effort], route_id: &str, key: &str) -> Option<&'a Effort> {
    efforts
        .iter()
        .filter(|e| e.route_id == route_id && e.key == key)
        .min_by(|a, b| a.secs.total_cmp(&b.secs))
}

/// "4:05" or "1:02:31".
pub fn fmt_time(secs: f32) -> String {
    let s = secs.max(0.0).round() as u32;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn climb(start: f64, end: f64) -> Segment {
        Segment {
            key: format!("climb-{start}"),
            name: "Climb".into(),
            kind: Kind::Climb,
            start,
            end,
            length: end - start,
            grade: 5.0,
        }
    }

    fn lap(len: f64) -> Segment {
        Segment {
            key: "lap".into(),
            name: "Lap".into(),
            kind: Kind::Lap,
            start: START_LINE,
            end: START_LINE,
            length: len,
            grade: 0.0,
        }
    }

    /// Ride `to` metres at 5 m/s from the start line, ticking every 0.1 s.
    fn ride(segs: &[Segment], len: f64, to: f64) -> Vec<Finished> {
        let mut t = Tracker::default();
        let mut out = Vec::new();
        let mut pos = START_LINE;
        let mut clock = 0.0;
        while pos < START_LINE + to {
            out.extend(t.update(segs, len, pos, clock, 200.0, Some(150.0)));
            pos += 0.5;
            clock += 0.1;
        }
        out
    }

    #[test]
    fn climbs_and_laps_are_timed() {
        let segs = vec![climb(100.0, 600.0), lap(1000.0)];
        let done = ride(&segs, 1000.0, 2100.0);
        // The climb twice, the lap twice.
        let climbs: Vec<&Finished> = done.iter().filter(|d| d.0 == 0).collect();
        let laps: Vec<&Finished> = done.iter().filter(|d| d.0 == 1).collect();
        assert_eq!(climbs.len(), 2);
        assert_eq!(laps.len(), 2);
        // 500 m at 5 m/s = 100 s; 1000 m = 200 s.
        assert!((climbs[0].1 - 100.0).abs() < 0.2, "{:?}", climbs[0]);
        assert!((laps[0].1 - 200.0).abs() < 0.2, "{:?}", laps[0]);
        assert!((climbs[0].2 - 200.0).abs() < 0.5);
        assert_eq!(climbs[0].3, Some(150.0));
    }

    #[test]
    fn a_climb_across_the_start_line_is_timed() {
        // On a 1000 m loop: from 900 m round to 300 m.
        let segs = vec![climb(900.0, 300.0)];
        let done = ride(&segs, 1000.0, 1500.0);
        assert_eq!(done.len(), 1);
        assert!((done[0].1 - 80.0).abs() < 0.2, "{:?}", done[0]);
    }

    #[test]
    fn jumps_abandon_open_segments() {
        let segs = vec![climb(100.0, 600.0)];
        let mut t = Tracker::default();
        t.update(&segs, 1000.0, 90.0, 0.0, 200.0, None);
        t.update(&segs, 1000.0, 110.0, 4.0, 200.0, None);
        assert!(t.current(&segs).is_some());
        // Teleported: nothing is credited.
        assert!(t.update(&segs, 1000.0, 5000.0, 5.0, 200.0, None).is_empty());
        assert!(t.current(&segs).is_none());
    }

    #[test]
    fn built_in_routes_have_climbs() {
        let routes = crate::route::builtin();
        let mut total = 0;
        for r in &routes {
            let segs = for_route(r);
            assert_eq!(segs.last().unwrap().kind, Kind::Lap);
            for s in segs.iter().filter(|s| s.kind == Kind::Climb) {
                assert!(s.length >= 400.0 && s.grade >= 3.0, "{} {:?}", r.name, s);
                assert!(s.end > s.start);
                total += 1;
            }
            let keys: std::collections::HashSet<&str> =
                segs.iter().map(|s| s.key.as_str()).collect();
            assert_eq!(keys.len(), segs.len(), "keys are unique");
        }
        assert!(total >= 3, "{total} climbs across the built-in routes");
    }

    #[test]
    fn the_best_effort_is_the_fastest() {
        let e = |secs: f32, key: &str| Effort {
            route_id: "valley".into(),
            key: key.into(),
            name: "Climb 1".into(),
            date: Local::now(),
            secs,
            avg_power: 200.0,
            avg_hr: None,
        };
        let all = vec![
            e(250.0, "climb-100"),
            e(240.0, "climb-100"),
            e(100.0, "lap"),
        ];
        assert_eq!(best(&all, "valley", "climb-100").unwrap().secs, 240.0);
        assert!(best(&all, "col", "climb-100").is_none());
        assert_eq!(fmt_time(245.4), "4:05");
        assert_eq!(fmt_time(3751.0), "1:02:31");
    }
}
