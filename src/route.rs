//! Virtual routes: a centerline sampled every few metres with elevation,
//! plus the scenery parameters the 3D view uses. Routes are generated
//! procedurally or imported from GPX.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

pub const STEP: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scenery {
    Hills,
    Lake,
    Alpine,
    /// Sea on the left (on the outside of a loop).
    Coast,
    /// The Japanese countryside: a lake among wooded hills, cherries in
    /// blossom, thatched farmhouses, shrines and temples.
    Japan,
}

impl Scenery {
    pub const ALL: [Scenery; 5] = [
        Scenery::Hills,
        Scenery::Lake,
        Scenery::Alpine,
        Scenery::Coast,
        Scenery::Japan,
    ];

    pub fn label(self) -> &'static str {
        self.spec().label
    }

    /// Water at a fixed level beside the road (lake or sea).
    pub fn has_water(self) -> bool {
        self.spec().water != crate::scenery::Water::None
    }
}

/// Whether a route comes back to where it starts (ridden in laps) or runs
/// from one place to another (it finishes there, and a road carries on
/// beyond the finish for whoever is still riding).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Shape {
    #[default]
    Loop,
    PointToPoint,
}

impl Shape {
    pub const ALL: [Shape; 2] = [Shape::Loop, Shape::PointToPoint];

    pub fn label(self) -> &'static str {
        match self {
            Shape::Loop => "Loop",
            Shape::PointToPoint => "Point to point",
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RoutePt {
    pub x: f64,
    pub z: f64,
    pub ele: f64,
    pub heading: f64,
}

#[derive(Debug, Clone)]
pub struct Route {
    pub id: String,
    pub name: String,
    pub scenery: Scenery,
    pub seed: u64,
    /// The course's points, and for a point-to-point route the road beyond
    /// its finish after them.
    pts: Vec<RoutePt>,
    /// The course: a lap of a loop, or start to finish.
    pub length: f64,
    pub ascent: f64,
    /// Point to point: the course finishes at `length` and the road goes on.
    open: bool,
}

/// After a route's id: the route ridden the other way.
pub const REVERSED: &str = "~reversed";

/// A route's name ridden the other way; a name ending in the climb (as
/// "… · 12 km, 600 m climb") gives `ascent` there, when it is known.
pub fn reversed_name(name: &str, ascent: Option<f64>) -> String {
    let name = match (
        ascent,
        name.strip_suffix(" m climb")
            .and_then(|s| s.rsplit_once(", ")),
    ) {
        (Some(a), Some((head, m))) if m.parse::<f64>().is_ok() => {
            format!("{head}, {a:.0} m climb")
        }
        _ => name.to_string(),
    };
    format!("{name} · reversed")
}

/// Points stored for user-imported routes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpxRoute {
    pub id: String,
    pub name: String,
    /// (distance m, elevation m, x m, z m), resampled every STEP metres.
    pub pts: Vec<(f64, f64, f64, f64)>,
    /// Chosen scenery (hills if unset).
    #[serde(default)]
    pub scenery: Option<Scenery>,
    /// Loop or point to point (worked out from its ends if unset).
    #[serde(default)]
    pub shape: Option<Shape>,
}

impl Route {
    pub(crate) fn from_points(
        id: &str,
        name: &str,
        scenery: Scenery,
        seed: u64,
        mut pts: Vec<RoutePt>,
    ) -> Route {
        // Headings from geometry, lightly smoothed.
        let n = pts.len();
        let raw: Vec<f64> = (0..n)
            .map(|i| {
                let a = pts[i.saturating_sub(1)];
                let b = pts[(i + 1).min(n - 1)];
                (b.x - a.x).atan2(b.z - a.z)
            })
            .collect();
        for i in 0..n {
            pts[i].heading = raw[i];
        }
        let ascent = pts.windows(2).map(|w| (w[1].ele - w[0].ele).max(0.0)).sum();
        let length = (n - 1) as f64 * STEP;
        Route {
            id: id.into(),
            name: name.into(),
            scenery,
            seed,
            pts,
            length,
            ascent,
            open: false,
        }
    }

    /// A point-to-point route: the `course` (its first point the start, its
    /// last the finish), then the road `beyond` it (points after the
    /// finish, `STEP` apart).
    pub(crate) fn point_to_point(
        id: &str,
        name: &str,
        scenery: Scenery,
        seed: u64,
        course: Vec<RoutePt>,
        beyond: Vec<RoutePt>,
    ) -> Route {
        let finish = (course.len() - 1) as f64 * STEP;
        let ascent = course
            .windows(2)
            .map(|w| (w[1].ele - w[0].ele).max(0.0))
            .sum();
        let mut pts = course;
        pts.extend(beyond);
        let mut r = Route::from_points(id, name, scenery, seed, pts);
        r.length = finish;
        r.ascent = ascent;
        r.open = true;
        r
    }

    /// The same road ridden the other way: a loop the other way round from
    /// the same start line; a point-to-point route from its finish to its
    /// start, with a road of its own beyond. Its id is the route's with
    /// `REVERSED` after it, so its rides, records and land are its own.
    pub fn reversed(&self) -> Route {
        let mut course = self.course_points().to_vec();
        course.reverse();
        let id = format!("{}{REVERSED}", self.id);
        if !self.open {
            let mut r = Route::from_points(&id, "", self.scenery, self.seed, course);
            r.name = reversed_name(&self.name, None);
            return r;
        }
        let beyond = crate::routegen::beyond(&course, self.scenery, self.seed);
        let mut r = Route::point_to_point(&id, "", self.scenery, self.seed, course, beyond);
        // Point to point, the climb one way is not the climb the other.
        r.name = reversed_name(&self.name, Some(r.ascent));
        r
    }

    /// Whether this is a route ridden the other way (see `reversed`).
    pub fn is_reversed(&self) -> bool {
        self.id.ends_with(REVERSED)
    }

    /// The id of the route this is, whichever way it is ridden.
    pub fn base_id(&self) -> &str {
        self.id.strip_suffix(REVERSED).unwrap_or(&self.id)
    }

    pub fn shape(&self) -> Shape {
        if self.open {
            Shape::PointToPoint
        } else {
            Shape::Loop
        }
    }

    /// How far the road runs: the lap of a loop, or for a point-to-point
    /// route its course and the road beyond the finish.
    pub fn span(&self) -> f64 {
        (self.pts.len() - 1) as f64 * STEP
    }

    /// Where `d` (distance ridden from the start) is on the course: within
    /// the lap of a loop; on a point-to-point route `d` itself, past
    /// `length` once beyond the finish.
    pub fn along(&self, d: f64) -> f64 {
        if self.open {
            d.max(0.0)
        } else {
            d.rem_euclid(self.length)
        }
    }

    /// The banners over the road from `from` to `to` (distance ridden):
    /// where each stands and what it says. A loop has one every kilometre
    /// and START · FINISH on its line; a point-to-point route has START,
    /// one every kilometre and FINISH, and none beyond.
    pub fn banners(&self, from: f64, to: f64) -> Vec<(f64, String)> {
        let mut out = Vec::new();
        let mut km = (from / 1000.0)
            .ceil()
            .max(if self.open { 0.0 } else { f64::MIN }) as i64;
        while (km as f64) * 1000.0 < to {
            let ks = km as f64 * 1000.0;
            km += 1;
            if self.open {
                if ks > self.length - 150.0 {
                    break;
                }
                let text = if km == 1 {
                    "START".to_string()
                } else {
                    format!("{:.0} KM", ks / 1000.0)
                };
                out.push((ks, text));
                continue;
            }
            let lap_pos = ks.rem_euclid(self.length);
            let text = if lap_pos < 1.0 || self.length - lap_pos < 1.0 {
                "START · FINISH".to_string()
            } else {
                format!("{:.0} KM", lap_pos / 1000.0)
            };
            out.push((ks, text));
        }
        if self.open && self.length >= from && self.length < to {
            out.push((self.length, "FINISH".to_string()));
        }
        out
    }

    /// The course's points: a lap, or start to finish.
    pub fn course_points(&self) -> &[RoutePt] {
        let n = ((self.length / STEP).round() as usize + 1).min(self.pts.len());
        &self.pts[..n]
    }

    /// Centreline points, one every `STEP` metres.
    pub fn points(&self) -> &[RoutePt] {
        &self.pts
    }

    /// Whether the route ends where it starts (a loop rather than point to point).
    pub fn is_loop(&self) -> bool {
        if self.open {
            return false;
        }
        let (a, b) = (self.pts[0], self.pts[self.pts.len() - 1]);
        ((a.x - b.x).powi(2) + (a.z - b.z).powi(2)).sqrt() < 30.0
    }

    /// Sample the route at distance `d`: a loop wraps round, lap after lap;
    /// a point-to-point route runs on past its finish to the end of the road
    /// beyond, and stops there.
    pub fn at(&self, d: f64) -> RoutePt {
        let d = if self.open {
            d.clamp(0.0, self.span())
        } else {
            d.rem_euclid(self.length)
        };
        let f = d / STEP;
        let i = (f.floor() as usize).min(self.pts.len() - 2);
        let u = f - i as f64;
        let a = self.pts[i];
        let b = self.pts[i + 1];
        // Between points the road follows a smooth curve leaving each point
        // the way the road heads there (a cubic Hermite curve), so a bend
        // is a curve, not a run of corners.
        let chord = (b.x - a.x).hypot(b.z - a.z);
        let (ma, mb) = (
            (a.heading.sin() * chord, a.heading.cos() * chord),
            (b.heading.sin() * chord, b.heading.cos() * chord),
        );
        let (u2, u3) = (u * u, u * u * u);
        let (h00, h10, h01, h11) = (
            2.0 * u3 - 3.0 * u2 + 1.0,
            u3 - 2.0 * u2 + u,
            -2.0 * u3 + 3.0 * u2,
            u3 - u2,
        );
        let (d00, d10, d01, d11) = (
            6.0 * u2 - 6.0 * u,
            3.0 * u2 - 4.0 * u + 1.0,
            -6.0 * u2 + 6.0 * u,
            3.0 * u2 - 2.0 * u,
        );
        let dx = d00 * a.x + d10 * ma.0 + d01 * b.x + d11 * mb.0;
        let dz = d00 * a.z + d10 * ma.1 + d01 * b.z + d11 * mb.1;
        RoutePt {
            x: h00 * a.x + h10 * ma.0 + h01 * b.x + h11 * mb.0,
            z: h00 * a.z + h10 * ma.1 + h01 * b.z + h11 * mb.1,
            ele: a.ele + (b.ele - a.ele) * u,
            heading: if dx.hypot(dz) > 1e-9 {
                dx.atan2(dz)
            } else {
                a.heading
            },
        }
    }

    /// Road grade in percent at distance `d` (any lap).
    pub fn grade(&self, d: f64) -> f64 {
        let h = 15.0;
        if self.is_loop() {
            // The road carries on round the start line.
            return (self.at(d + h).ele - self.at(d - h).ele) / (2.0 * h) * 100.0;
        }
        let (d, end) = if self.open {
            (d.clamp(0.0, self.span()), self.span())
        } else {
            (d.rem_euclid(self.length), self.length)
        };
        let mut a = d - h;
        let mut b = d + h;
        // Do not average across the start/finish seam of a non-loop route.
        if a < 0.0 {
            a = 0.0;
            b = 2.0 * h;
        } else if b > end {
            b = end;
            a = end - 2.0 * h;
        }
        (self.at(b).ele - self.at(a).ele) / (b - a) * 100.0
    }

    /// Elevation profile for charts: (distance, elevation) every ~`step` m.
    pub fn profile(&self, samples: usize) -> Vec<(f64, f64)> {
        (0..=samples)
            .map(|i| {
                let d = self.length * i as f64 / samples as f64;
                (d, self.at(d.min(self.length - 0.01)).ele)
            })
            .collect()
    }

    pub fn min_max_ele(&self) -> (f64, f64) {
        self.pts.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| {
            (lo.min(p.ele), hi.max(p.ele))
        })
    }

    /// Height of the ground at a lateral offset from the road centre
    /// (positive = right of travel direction). Roads sit in gentle valleys.
    pub fn terrain(&self, d: f64, lateral: f64) -> f64 {
        let road = self.at(d).ele;
        let a = lateral.abs();
        if self.scenery.has_water() && lateral < -14.0 {
            return road - 1.5;
        }
        let ramp = ((a - 9.0) / 90.0).clamp(0.0, 1.0);
        let ramp = ramp * ramp * (3.0 - 2.0 * ramp);
        let s = self.seed as f64 * 0.37 + if lateral > 0.0 { 0.0 } else { 17.0 };
        let amp = self.scenery.spec().routes.side_hills;
        let hill =
            0.55 + 0.25 * (d / 140.0 + s).sin() + 0.2 * (d / 53.0 + a / 31.0 + s * 2.0).sin();
        road + ramp * amp * hill - (1.0 - ramp) * 0.3 * (a / 9.0).min(1.0)
    }
}

/// Close a heading sequence into a loop and integrate positions.
pub(crate) fn integrate_loop(
    n: usize,
    curvature: impl Fn(f64) -> f64,
    ele: impl Fn(f64) -> f64,
) -> Vec<RoutePt> {
    let raw: Vec<f64> = (0..n).map(|i| curvature(i as f64 / n as f64)).collect();
    let total: f64 = raw.iter().sum();
    let mut heading = 0.0;
    let mut pts = Vec::with_capacity(n + 1);
    let (mut x, mut z) = (0.0, 0.0);
    for i in 0..=n {
        pts.push(RoutePt {
            x,
            z,
            ele: ele(i as f64 / n as f64),
            heading,
        });
        if let Some(k) = raw.get(i) {
            // Scale turning so the loop turns exactly once.
            heading += k / total * TAU;
            x += heading.sin() * STEP;
            z += heading.cos() * STEP;
        }
    }
    // Remove the residual gap so the loop closes.
    let (gx, gz) = (pts[n].x, pts[n].z);
    for (i, p) in pts.iter_mut().enumerate() {
        let u = i as f64 / n as f64;
        p.x -= gx * u;
        p.z -= gz * u;
    }
    pts
}

/// A piece of road for [`build_segments`].
#[derive(Debug, Clone, Copy)]
enum Seg {
    /// Straight road: length in metres and grade in percent.
    Straight(f64, f64),
    /// Bend: radius in metres, turn in degrees (positive turns right) and grade.
    Arc(f64, f64, f64),
}

/// Lay out a loop from straights and bends, closing any small residual gap
/// and smoothing the elevation so grade changes are gradual.
fn build_segments(start_ele: f64, segs: &[Seg]) -> Vec<RoutePt> {
    // Expand into per-step (turn, grade).
    let mut steps: Vec<(f64, f64)> = Vec::new();
    for s in segs {
        match *s {
            Seg::Straight(len, grade) => {
                let n = (len / STEP).round() as usize;
                steps.extend(std::iter::repeat_n((0.0, grade), n));
            }
            Seg::Arc(radius, deg, grade) => {
                let len = radius * deg.abs().to_radians();
                let n = (len / STEP).round().max(1.0) as usize;
                let turn = deg.to_radians() / n as f64;
                steps.extend(std::iter::repeat_n((turn, grade), n));
            }
        }
    }
    let n = steps.len();
    let mut pts = Vec::with_capacity(n + 1);
    let (mut x, mut z, mut heading, mut ele) = (0.0f64, 0.0f64, 0.0f64, start_ele);
    for i in 0..=n {
        pts.push(RoutePt { x, z, ele, heading });
        if let Some(&(turn, grade)) = steps.get(i) {
            let mid = heading + turn / 2.0;
            x += mid.sin() * STEP;
            z += mid.cos() * STEP;
            heading += turn;
            ele += grade / 100.0 * STEP;
        }
    }
    // Close the loop: spread the position and elevation gaps along the route.
    let (gx, gz, ge) = (
        pts[n].x - pts[0].x,
        pts[n].z - pts[0].z,
        pts[n].ele - pts[0].ele,
    );
    for (i, p) in pts.iter_mut().enumerate() {
        let u = i as f64 / n as f64;
        p.x -= gx * u;
        p.z -= gz * u;
        p.ele -= ge * u;
    }
    // Smooth elevation over ~80 m (wrapping around the loop).
    let k = 8i64;
    let raw: Vec<f64> = pts[..n].iter().map(|p| p.ele).collect();
    for (i, p) in pts.iter_mut().enumerate().take(n) {
        let sum: f64 = (-k..=k)
            .map(|j| raw[(i as i64 + j).rem_euclid(n as i64) as usize])
            .sum();
        p.ele = sum / (2 * k + 1) as f64;
    }
    pts[n].ele = pts[0].ele;
    pts
}

/// An original mountain pass: a valley approach, ten switchback legs up a
/// steep face joined by tight hairpins, and a long descent back.
fn hairpin_pass() -> Route {
    use Seg::*;
    let mut segs = vec![
        Straight(300.0, 1.5),
        Arc(400.0, 15.0, 2.0),
        Straight(350.0, 3.0),
        Arc(400.0, -15.0, 3.0),
        Straight(500.0, 3.5),
        Arc(60.0, 90.0, 4.0),
    ];
    for leg in 0..10 {
        segs.push(Straight(340.0, 7.5 + (leg % 3) as f64 * 0.6));
        if leg < 9 {
            // Hairpins alternate left and right, climbing gently.
            let dir = if leg % 2 == 0 { -1.0 } else { 1.0 };
            segs.push(Arc(36.0, 180.0 * dir, 4.0));
        }
    }
    segs.extend([
        Straight(250.0, 1.0),
        Straight(600.0, -6.5),
        Arc(150.0, -90.0, -8.0),
        Straight(1800.0, -10.5),
        Arc(150.0, -90.0, -7.0),
        Straight(560.0, -4.0),
        Arc(150.0, -90.0, -2.0),
    ]);
    Route::from_points(
        "hairpin-pass",
        "Hairpin Pass · 9 km, 300 m climb",
        Scenery::Alpine,
        29,
        build_segments(620.0, &segs),
    )
}

pub fn builtin() -> Vec<Route> {
    let mk = |id: &str,
              name: &str,
              scenery,
              seed,
              km: f64,
              curv: &dyn Fn(f64) -> f64,
              ele: &dyn Fn(f64) -> f64| {
        let n = (km * 1000.0 / STEP) as usize;
        Route::from_points(id, name, scenery, seed, integrate_loop(n, curv, ele))
    };
    vec![
        mk(
            "valley-loop",
            "Valley Loop · 12 km rolling",
            Scenery::Hills,
            3,
            12.0,
            // Winding, but never crossing or running alongside itself.
            &|u| 1.0 + 3.5 * (u * TAU * 3.0).sin() + 3.0 * (u * TAU * 7.0 + 1.0).sin(),
            &|u| {
                120.0
                    + 28.0 * (u * TAU * 2.0).sin()
                    + 11.0 * (u * TAU * 5.0 + 1.0).sin()
                    + 4.0 * (u * TAU * 13.0 + 2.0).sin()
            },
        ),
        mk(
            "lakeside",
            "Lakeside Flats · 16 km flat",
            Scenery::Lake,
            11,
            16.0,
            &|u| 1.0 + 3.0 * (u * TAU * 4.0).sin() + 2.0 * (u * TAU * 9.0 + 2.0).sin(),
            &|u| 40.0 + 3.0 * (u * TAU * 3.0).sin() + 1.5 * (u * TAU * 11.0).sin(),
        ),
        mk(
            "col-loop",
            "Col Loop · 14 km, 420 m climb",
            Scenery::Alpine,
            7,
            14.0,
            &|u| 1.0 + 7.0 * (u * TAU * 6.0).sin().powi(3) + 2.0 * (u * TAU * 2.0).sin(),
            &|u| {
                // Long climb in the first half, descent in the second.
                let base = 800.0 + 210.0 * (1.0 - (u * TAU).cos());
                base + 6.0 * (u * TAU * 9.0).sin() + 2.0 * (u * TAU * 17.0 + 1.0).sin()
            },
        ),
        mk(
            "island-coast",
            "Island Coast · 18 km rolling",
            Scenery::Coast,
            17,
            18.0,
            &|u| 1.0 + 4.0 * (u * TAU * 5.0).sin().powi(3) + 2.5 * (u * TAU * 11.0 + 0.7).sin(),
            &|u| {
                // Low coast road with a few climbs over headlands.
                22.0 + 14.0 * (u * TAU * 3.0).sin().max(-0.4)
                    + 9.0 * (u * TAU * 7.0 + 1.3).sin().max(-0.2)
                    + 3.0 * (u * TAU * 19.0).sin()
            },
        ),
        hairpin_pass(),
        mk(
            "sakura-lake",
            "Sakura Lake · 15 km gentle",
            Scenery::Japan,
            29,
            15.0,
            &|u| 1.0 + 3.2 * (u * TAU * 4.0 + 0.6).sin() + 2.4 * (u * TAU * 9.0 + 1.9).sin(),
            // Gently up and down along the shore: the lake lies just below
            // the lowest of the road, so a road high above it would stand on
            // a cliff over the water.
            &|u| {
                300.0
                    + 4.0 * (u * TAU * 2.0 + 0.4).sin()
                    + 2.5 * (u * TAU * 5.0 + 1.1).sin()
                    + 1.0 * (u * TAU * 13.0).sin()
            },
        ),
        mk(
            "criterium",
            "Park Criterium · 4 km",
            Scenery::Hills,
            21,
            4.0,
            &|u| 1.0 + 4.0 * (u * TAU * 4.0).sin().powi(5),
            &|u| 60.0 + 6.0 * (u * TAU).sin() + 2.0 * (u * TAU * 3.0).sin(),
        ),
    ]
}

impl GpxRoute {
    /// Loop or point to point: as chosen, else a loop if it ends within
    /// 30 m of where it starts.
    pub fn shape(&self) -> Shape {
        self.shape.unwrap_or_else(|| {
            let (a, b) = (self.pts[0], self.pts[self.pts.len() - 1]);
            if (a.2 - b.2).hypot(a.3 - b.3) < 30.0 {
                Shape::Loop
            } else {
                Shape::PointToPoint
            }
        })
    }

    pub fn to_route(&self) -> Route {
        let track: Vec<RoutePt> = self
            .pts
            .iter()
            .map(|&(_, ele, x, z)| RoutePt {
                x,
                z,
                ele,
                heading: 0.0,
            })
            .collect();
        let mut pts = clean_track(&track);
        let seed = self
            .id
            .bytes()
            .fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64))
            % 97;
        let scenery = self.scenery.unwrap_or(Scenery::Hills);
        match self.shape() {
            Shape::Loop => {
                // Ridden as a loop though its ends are apart: a straight road
                // back to the start closes it.
                let (a, b) = (pts[pts.len() - 1], pts[0]);
                let gap = (b.x - a.x).hypot(b.z - a.z);
                let steps = (gap / STEP).round() as usize;
                for k in 1..=steps {
                    let u = k as f64 / steps as f64;
                    pts.push(RoutePt {
                        x: a.x + (b.x - a.x) * u,
                        z: a.z + (b.z - a.z) * u,
                        ele: a.ele + (b.ele - a.ele) * u,
                        heading: 0.0,
                    });
                }
                Route::from_points(&self.id, &self.name, scenery, seed, pts)
            }
            Shape::PointToPoint => {
                let beyond = crate::routegen::beyond(&pts, scenery, seed);
                Route::point_to_point(&self.id, &self.name, scenery, seed, pts, beyond)
            }
        }
    }
}

/// The tightest bend a cleaned track keeps (radius of its centre line,
/// metres): about a real hairpin's, and wider than the road's half width, so
/// its edges never fold over.
pub const TIGHTEST_BEND: f64 = 11.0;
/// The steepest a cleaned track climbs or falls (percent).
pub const STEEPEST_GRADE: f64 = 20.0;

/// A road from a GPS track (points `STEP` apart), as a road engineer would
/// lay it: the track's jitter and its corners at single points smoothed
/// away, every bend at least `TIGHTEST_BEND` round where the track allows,
/// keeping close to the track; its heights smoothed over about 200 m and
/// no steeper than `STEEPEST_GRADE`. Its ends stay where they are.
pub(crate) fn clean_track(track: &[RoutePt]) -> Vec<RoutePt> {
    let n = track.len();
    if n < 8 {
        return track.to_vec();
    }
    let at = |v: &[RoutePt], i: i64| v[i.clamp(0, n as i64 - 1) as usize];
    // Jitter: a gentle average over about 25 m.
    let weights = [0.06, 0.24, 0.4, 0.24, 0.06];
    let mut pts: Vec<RoutePt> = (0..n as i64)
        .map(|i| {
            let (mut x, mut z) = (0.0, 0.0);
            for (k, w) in weights.iter().enumerate() {
                let p = at(track, i + k as i64 - 2);
                x += w * p.x;
                z += w * p.z;
            }
            RoutePt {
                x,
                z,
                ..track[i as usize]
            }
        })
        .collect();
    pts[0] = track[0];
    pts[n - 1] = track[n - 1];
    // Bends ease in and out rather than start at a point.
    for _ in 0..2 {
        let prev = pts.clone();
        for i in 2..n - 2 {
            let (mut x, mut z) = (0.0, 0.0);
            for (k, w) in weights.iter().enumerate() {
                let p = prev[i + k - 2];
                x += w * p.x;
                z += w * p.z;
            }
            pts[i].x = x;
            pts[i].z = z;
        }
    }
    // Bends: no tighter than `TIGHTEST_BEND`, as a road is laid out. The
    // road turns at most so much at each point; what a tight bend turns
    // beyond that is passed to its neighbours, spreading the bend round a
    // wider curve. The road is laid again from those headings through each
    // bend and joined back onto the track after it.
    let max_turn = STEP / TIGHTEST_BEND;
    let wrap = |a: f64| (a + std::f64::consts::PI).rem_euclid(TAU) - std::f64::consts::PI;
    // A few times over: joining a bend back onto the track bends it a
    // little again.
    for _round in 0..5 {
        let seg: Vec<f64> = (1..n)
            .map(|i| (pts[i].x - pts[i - 1].x).hypot(pts[i].z - pts[i - 1].z))
            .collect();
        let head: Vec<f64> = (1..n)
            .map(|i| (pts[i].x - pts[i - 1].x).atan2(pts[i].z - pts[i - 1].z))
            .collect();
        // turn[k]: the turn from step k to step k + 1.
        let mut turn: Vec<f64> = (1..head.len())
            .map(|k| wrap(head[k] - head[k - 1]))
            .collect();
        let mut changed = vec![false; turn.len()];
        for _ in 0..2000 {
            let mut any = false;
            for k in 0..turn.len() {
                // As much as the road may turn over the distance about here.
                let max_turn = max_turn * (seg[k] + seg[k + 1]) / (2.0 * STEP);
                if turn[k].abs() > max_turn + 1e-9 {
                    let excess = turn[k] - max_turn * turn[k].signum();
                    turn[k] -= excess;
                    changed[k] = true;
                    any = true;
                    match (k > 0, k + 1 < turn.len()) {
                        (true, true) => {
                            turn[k - 1] += excess / 2.0;
                            turn[k + 1] += excess / 2.0;
                            changed[k - 1] = true;
                            changed[k + 1] = true;
                        }
                        (true, false) => {
                            turn[k - 1] += excess;
                            changed[k - 1] = true;
                        }
                        (false, true) => {
                            turn[k + 1] += excess;
                            changed[k + 1] = true;
                        }
                        _ => {}
                    }
                }
            }
            if !any {
                break;
            }
        }
        // Each stretch of changed turns, with a little road either side, is
        // laid again from its headings; what it misses the track by at its end
        // is spread along it, so it joins back on.
        let m_turns = turn.len();
        let mut k = 0;
        while k < m_turns {
            if !changed[k] {
                k += 1;
                continue;
            }
            let mut e = k;
            while e + 1 < m_turns && (changed[e + 1] || changed[(e + 2).min(m_turns - 1)]) {
                e += 1;
            }
            // Points a..=b are laid again (point i sits after step i - 1).
            let a = k.saturating_sub(6);
            let b = (e + 30).min(n - 1);
            let mut h = head[a];
            let mut laid = vec![pts[a]];
            for i in a + 1..=b {
                // Step i - 1 runs from point i - 1 to point i.
                if i - 1 > a {
                    h += turn[i - 2];
                }
                let p = laid[laid.len() - 1];
                laid.push(RoutePt {
                    x: p.x + h.sin() * seg[i - 1],
                    z: p.z + h.cos() * seg[i - 1],
                    ..pts[i]
                });
            }
            let miss = (pts[b].x - laid[b - a].x, pts[b].z - laid[b - a].z);
            for (j, p) in laid.iter().enumerate() {
                let u = j as f64 / (b - a) as f64;
                pts[a + j].x = p.x + miss.0 * u;
                pts[a + j].z = p.z + miss.1 * u;
            }
            k = e + 1;
        }
    }
    // Back to points `STEP` apart along the eased road (easing shortens
    // it), each with the track's height where it came from.
    let mut along = vec![0.0; n];
    for i in 1..n {
        along[i] = along[i - 1] + (pts[i].x - pts[i - 1].x).hypot(pts[i].z - pts[i - 1].z);
    }
    let total = along[n - 1];
    let m = ((total / STEP).round() as usize).max(2);
    let mut out = Vec::with_capacity(m + 1);
    let mut j = 0;
    for k in 0..=m {
        let d = total * k as f64 / m as f64;
        while j + 2 < n && along[j + 1] < d {
            j += 1;
        }
        let span = (along[j + 1] - along[j]).max(1e-9);
        let u = ((d - along[j]) / span).clamp(0.0, 1.0);
        let (a, b) = (pts[j], pts[j + 1]);
        out.push(RoutePt {
            x: a.x + (b.x - a.x) * u,
            z: a.z + (b.z - a.z) * u,
            ele: a.ele + (b.ele - a.ele) * u,
            heading: 0.0,
        });
    }
    // Heights: averaged over about 200 m, then no step steeper than allowed
    // (eased from both ends so neither is favoured).
    let m = out.len();
    // (GPS and map heights along a road on a steep slope jump about: the
    // slope's height, not the road's, leaks in.)
    let reach = 20i64;
    let eles: Vec<f64> = out.iter().map(|p| p.ele).collect();
    for (i, p) in out.iter_mut().enumerate() {
        let (mut sum, mut wsum) = (0.0, 0.0);
        for k in -reach..=reach {
            let j = (i as i64 + k).clamp(0, m as i64 - 1) as usize;
            let w = 1.0 - (k.abs() as f64 / (reach + 1) as f64);
            sum += w * eles[j];
            wsum += w;
        }
        p.ele = sum / wsum;
    }
    let step_max = STEEPEST_GRADE / 100.0 * STEP;
    for _ in 0..200 {
        let mut changed = false;
        for i in 0..m - 1 {
            let d = out[i + 1].ele - out[i].ele;
            if d.abs() > step_max + 1e-9 {
                let fix = (d.abs() - step_max) / 2.0 * d.signum();
                out[i].ele += fix;
                out[i + 1].ele -= fix;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    out
}

/// Parse a GPX track or route, project it to local metres and resample.
/// The route is called `name` (the app passes the file's name: the name
/// inside a GPX file is often an app's placeholder, such as "New file").
pub fn parse_gpx(xml: &str, id: &str, name: &str) -> Result<GpxRoute> {
    let doc = roxmltree::Document::parse(xml).context("invalid GPX XML")?;
    let mut raw = Vec::new();
    for n in doc
        .descendants()
        .filter(|n| n.has_tag_name("trkpt") || n.has_tag_name("rtept"))
    {
        let lat: f64 = n
            .attribute("lat")
            .and_then(|v| v.parse().ok())
            .context("trkpt without lat")?;
        let lon: f64 = n
            .attribute("lon")
            .and_then(|v| v.parse().ok())
            .context("trkpt without lon")?;
        let ele = n
            .children()
            .find(|c| c.has_tag_name("ele"))
            .and_then(|c| c.text())
            .and_then(|t| t.trim().parse::<f64>().ok());
        raw.push((lat, lon, ele));
    }
    anyhow::ensure!(raw.len() >= 2, "GPX has fewer than two track points");

    // Equirectangular projection around the first point.
    let (lat0, lon0) = (raw[0].0.to_radians(), raw[0].1.to_radians());
    const R: f64 = 6_371_000.0;
    let mut last_ele = raw.iter().find_map(|p| p.2).unwrap_or(0.0);
    let mut proj: Vec<(f64, f64, f64, f64)> = Vec::new(); // (dist, x, z, ele)
    for &(lat, lon, ele) in &raw {
        let x = (lon.to_radians() - lon0) * lat0.cos() * R;
        let z = (lat.to_radians() - lat0) * R;
        let ele = ele.unwrap_or(last_ele);
        last_ele = ele;
        let d = match proj.last() {
            Some(&(pd, px, pz, _)) => {
                let step = ((x - px).powi(2) + (z - pz).powi(2)).sqrt();
                if step < 0.5 {
                    continue;
                }
                pd + step
            }
            None => 0.0,
        };
        proj.push((d, x, z, ele));
    }
    let total = proj.last().unwrap().0;
    anyhow::ensure!(total > 200.0, "GPX route is shorter than 200 m");

    // Resample every STEP metres.
    let n = (total / STEP).floor() as usize;
    let mut out = Vec::with_capacity(n + 1);
    let mut j = 0;
    for i in 0..=n {
        let d = i as f64 * STEP;
        while j + 2 < proj.len() && proj[j + 1].0 < d {
            j += 1;
        }
        let (d0, x0, z0, e0) = proj[j];
        let (d1, x1, z1, e1) = proj[(j + 1).min(proj.len() - 1)];
        let u = if d1 > d0 {
            ((d - d0) / (d1 - d0)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        out.push((
            d,
            e0 + (e1 - e0) * u,
            x0 + (x1 - x0) * u,
            z0 + (z1 - z0) * u,
        ));
    }
    // Smooth elevation (GPS altitude is noisy) over ~50 m.
    let k = 5usize;
    let eles: Vec<f64> = out.iter().map(|p| p.1).collect();
    for i in 0..out.len() {
        let lo = i.saturating_sub(k);
        let hi = (i + k).min(out.len() - 1);
        out[i].1 = eles[lo..=hi].iter().sum::<f64>() / (hi - lo + 1) as f64;
    }
    Ok(GpxRoute {
        id: id.into(),
        name: name.into(),
        pts: out,
        scenery: None,
        shape: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Scenery::ALL` lists every scenery, once each. The menus and the tests
    /// go through `ALL`, so a scenery left out of it would be missing from
    /// both with nothing to say so. Every scenery is taken from the names
    /// saved routes are read by (serde's list of them), not a list kept by
    /// hand that could be left out as easily.
    #[test]
    fn every_scenery_is_in_the_list_of_them() {
        use serde::de::{self, Visitor};
        /// Takes the names of an enum's variants from its `Deserialize`.
        struct Names<'a>(&'a mut &'static [&'static str]);
        impl<'de> serde::Deserializer<'de> for Names<'_> {
            type Error = de::value::Error;
            fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Self::Error> {
                Err(de::Error::custom("not an enum"))
            }
            fn deserialize_enum<V: Visitor<'de>>(
                self,
                _: &'static str,
                variants: &'static [&'static str],
                _: V,
            ) -> Result<V::Value, Self::Error> {
                *self.0 = variants;
                Err(de::Error::custom("names taken"))
            }
            serde::forward_to_deserialize_any! {
                bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
                bytes byte_buf option unit unit_struct newtype_struct seq tuple
                tuple_struct map struct identifier ignored_any
            }
        }
        let mut names: &'static [&'static str] = &[];
        let _ = <Scenery as serde::Deserialize>::deserialize(Names(&mut names));
        assert!(!names.is_empty(), "no names taken from Scenery");
        for name in names {
            let s: Scenery = serde_json::from_str(&format!("\"{name}\"")).unwrap();
            let listed = Scenery::ALL.iter().filter(|&&a| a == s).count();
            assert_eq!(
                listed, 1,
                "{name} is in Scenery::ALL {listed} times, not once"
            );
        }
        assert_eq!(Scenery::ALL.len(), names.len());
    }

    #[test]
    fn the_grade_is_the_same_on_every_lap() {
        for r in builtin() {
            for d in [100.0, 1234.0, r.length * 0.5, r.length - 5.0] {
                let first = r.grade(d);
                for lap in 1..4 {
                    let later = r.grade(d + lap as f64 * r.length);
                    assert!(
                        (first - later).abs() < 1e-6,
                        "{} at {d}: {first} vs {later}",
                        r.name
                    );
                }
            }
        }
    }

    #[test]
    fn a_hairpin_climb_is_ridden_point_to_point_with_a_road_beyond() {
        // Twelve legs 130 m apart up an 8 % slope, joined by hairpins.
        let mut xml = String::from("<gpx><trk><trkseg>");
        let (mut x, mut z, mut ele) = (0.0f64, 0.0f64, 700.0f64);
        let push = |x: f64, z: f64, e: f64, xml: &mut String| {
            let lat = 45.0 + (z / 6_371_000.0).to_degrees();
            let lon = 6.0 + (x / (6_371_000.0 * 45f64.to_radians().cos())).to_degrees();
            *xml += &format!(r#"<trkpt lat="{lat:.7}" lon="{lon:.7}"><ele>{e:.1}</ele></trkpt>"#);
        };
        for leg in 0..12 {
            let dir = if leg % 2 == 0 { 1.0 } else { -1.0 };
            for _ in 0..95 {
                z += dir * 10.0;
                ele += 0.8;
                push(x, z, ele, &mut xml);
            }
            for k in 1..20 {
                let a = std::f64::consts::PI * k as f64 / 20.0;
                push(
                    x + 65.0 - 65.0 * a.cos(),
                    z + dir * 39.0 * a.sin(),
                    ele,
                    &mut xml,
                );
            }
            x += 130.0;
        }
        xml += "</trkseg></trk></gpx>";
        let g = parse_gpx(&xml, "hairpins", "Hairpins").unwrap();
        assert_eq!(g.shape(), Shape::PointToPoint);
        let r = g.to_route();
        assert_eq!(r.shape(), Shape::PointToPoint);
        assert!(r
            .points()
            .iter()
            .all(|p| p.x.is_finite() && p.ele.is_finite()));
        assert!(r.span() > r.length + 30_000.0, "a road beyond the finish");
        // The road beyond keeps clear of the course's legs.
        let course = r.course_points();
        for p in &r.points()[course.len() + 100..] {
            let nearest = course
                .iter()
                .map(|c| (c.x - p.x).hypot(c.z - p.z))
                .fold(f64::MAX, f64::min);
            assert!(nearest > 85.0, "{nearest:.0} m from the course");
        }
    }

    #[test]
    fn a_rough_track_is_cleaned_into_a_road() {
        // Straight legs meeting at sharp corners (a planner's line through
        // map points), with GPS jitter, a height spike and a far too steep
        // step.
        let mut track = Vec::new();
        let corners = [
            (0.0, 0.0),
            (400.0, 0.0),
            (400.0, 300.0),
            (120.0, 330.0),
            (130.0, 700.0),
        ];
        let mut k = 0u32;
        for w in corners.windows(2) {
            let ((x0, z0), (x1, z1)): ((f64, f64), (f64, f64)) = (w[0], w[1]);
            let n = ((x1 - x0).hypot(z1 - z0) / STEP) as usize;
            for i in 0..n {
                let u = i as f64 / n as f64;
                let jitter = |s: u32| ((s as f64 * 12.9898).sin() * 43758.5453).fract() - 0.5;
                k += 1;
                track.push(RoutePt {
                    x: x0 + (x1 - x0) * u + 1.5 * jitter(k),
                    z: z0 + (z1 - z0) * u + 1.5 * jitter(k + 999),
                    ele: 100.0 + track.len() as f64 * 0.3 + if k == 120 { 25.0 } else { 0.0 },
                    heading: 0.0,
                });
            }
        }
        let road = clean_track(&track);
        // Every bend at least as round as allowed (over 10 m either side).
        for i in 2..road.len() - 2 {
            let (a, b, c) = (road[i - 2], road[i], road[i + 2]);
            let (ab, bc, ca) = (
                (a.x - b.x).hypot(a.z - b.z),
                (b.x - c.x).hypot(b.z - c.z),
                (c.x - a.x).hypot(c.z - a.z),
            );
            let area = ((b.x - a.x) * (c.z - a.z) - (c.x - a.x) * (b.z - a.z)).abs() / 2.0;
            if area > 1e-6 {
                let r = ab * bc * ca / (4.0 * area);
                assert!(r > TIGHTEST_BEND * 0.9, "a bend of {r:.1} m at {i}");
            }
        }
        // Points evenly apart, close to the track, ends where they were.
        for w in road.windows(2) {
            let d = (w[1].x - w[0].x).hypot(w[1].z - w[0].z);
            assert!((d - STEP).abs() < 0.3, "{d}");
        }
        for p in &road {
            let off = track
                .iter()
                .map(|t| (t.x - p.x).hypot(t.z - p.z))
                .fold(f64::MAX, f64::min);
            assert!(off < 12.0, "{off:.1} m off the track");
        }
        assert!((road[0].x - track[0].x).abs() < 1e-9);
        let (l, t) = (road[road.len() - 1], track[track.len() - 1]);
        assert!((l.x - t.x).hypot(l.z - t.z) < 1e-6);
        // No step steeper than allowed, and the spike gone.
        for w in road.windows(2) {
            assert!((w[1].ele - w[0].ele).abs() <= STEEPEST_GRADE / 100.0 * STEP + 1e-6);
        }
        let spike = road.iter().map(|p| p.ele).fold(f64::MIN, f64::max);
        assert!(spike < 100.0 + track.len() as f64 * 0.3 + 5.0, "{spike}");
    }

    #[test]
    fn hairpins_are_laid_round_and_stay_on_the_track() {
        // Legs 30 m apart joined by hairpins far tighter than a road: a
        // V at each end.
        let mut track = Vec::new();
        let (mut x, mut z) = (0.0f64, 0.0f64);
        for leg in 0..6 {
            let dir = if leg % 2 == 0 { 1.0 } else { -1.0 };
            for _ in 0..80 {
                z += dir * STEP;
                track.push(RoutePt {
                    x,
                    z,
                    ele: 0.0,
                    heading: 0.0,
                });
            }
            for _ in 0..6 {
                x += STEP;
                track.push(RoutePt {
                    x,
                    z,
                    ele: 0.0,
                    heading: 0.0,
                });
            }
        }
        let road = clean_track(&track);
        let wrap = |a: f64| (a + std::f64::consts::PI).rem_euclid(TAU) - std::f64::consts::PI;
        // (Spacing the points evenly again after laying adds a little.)
        let max_step = (STEP / TIGHTEST_BEND).to_degrees() * 1.25;
        for i in 2..road.len() {
            let h1 = (road[i - 1].x - road[i - 2].x).atan2(road[i - 1].z - road[i - 2].z);
            let h2 = (road[i].x - road[i - 1].x).atan2(road[i].z - road[i - 1].z);
            let t = wrap(h2 - h1).abs().to_degrees();
            assert!(t <= max_step, "{t:.1} degrees at {i}");
        }
        for p in &road {
            let off = track
                .iter()
                .map(|t| (t.x - p.x).hypot(t.z - p.z))
                .fold(f64::MAX, f64::min);
            assert!(off < 10.0, "{off:.1} m off the track");
        }
    }

    #[test]
    fn builtin_loops_close() {
        for r in builtin() {
            let a = r.at(0.0);
            let b = r.at(r.length - 0.001);
            assert!(
                ((a.x - b.x).powi(2) + (a.z - b.z).powi(2)).sqrt() < 1.0,
                "{}",
                r.name
            );
            assert!((a.ele - b.ele).abs() < 1.0, "{}", r.name);
            for i in 0..200 {
                let g = r.grade(r.length * i as f64 / 200.0);
                assert!(g.abs() < 16.0, "{} grade {g}", r.name);
            }
        }
    }

    #[test]
    fn roads_never_cross_themselves() {
        // Parts of a route at least 600 m apart along the road must be well
        // apart on the ground too (hairpin legs are about 70 m apart).
        for r in builtin() {
            let p = r.points();
            let n = p.len() - 1;
            let gap = (600.0 / STEP) as usize;
            for i in (0..n).step_by(2) {
                for j in ((i + gap)..n).step_by(2) {
                    if (j - i).min(n - (j - i)) < gap {
                        continue;
                    }
                    let d = ((p[i].x - p[j].x).powi(2) + (p[i].z - p[j].z).powi(2)).sqrt();
                    assert!(
                        d > 55.0,
                        "{}: road at {:.0} m is {d:.0} m from the road at {:.0} m",
                        r.id,
                        i as f64 * STEP,
                        j as f64 * STEP
                    );
                }
            }
        }
    }

    #[test]
    fn hairpin_pass_climbs_through_switchbacks() {
        let r = hairpin_pass();
        assert!(r.is_loop());
        assert!((8_500.0..10_000.0).contains(&r.length), "{}", r.length);
        let (lo, hi) = r.min_max_ele();
        assert!(hi - lo > 250.0, "{}", hi - lo);
        // Count hairpins: heading reverses within about 110 m.
        let mut hairpins = 0;
        let mut d = 0.0;
        while d < r.length - 120.0 {
            let a = r.at(d).heading;
            let b = r.at(d + 110.0).heading;
            let turn = (b - a + std::f64::consts::PI).rem_euclid(TAU) - std::f64::consts::PI;
            if turn.abs() > 2.8 {
                hairpins += 1;
                d += 150.0;
            } else {
                d += 5.0;
            }
        }
        assert_eq!(hairpins, 9);
        // Switchback legs do not cross each other.
        let pts = r.points();
        for (i, p) in pts.iter().enumerate().step_by(4) {
            for q in pts.iter().skip(i + 30).step_by(4) {
                let d = ((p.x - q.x).powi(2) + (p.z - q.z).powi(2)).sqrt();
                let along = (q.ele - p.ele).abs();
                assert!(d > 20.0 || along < 3.0, "road overlaps itself");
            }
        }
    }

    #[test]
    fn island_coast_turns_right_so_the_sea_is_on_the_left() {
        let r = builtin()
            .into_iter()
            .find(|r| r.id == "island-coast")
            .unwrap();
        let pts = r.points();
        let area: f64 = pts
            .windows(2)
            .map(|w| w[0].x * w[1].z - w[1].x * w[0].z)
            .sum();
        assert!(area < 0.0, "clockwise loop");
        let (lo, _) = r.min_max_ele();
        assert!(lo > 3.0);
    }

    #[test]
    fn a_loop_reversed_is_the_same_road_the_other_way_round() {
        let f = builtin().into_iter().find(|r| r.id == "col-loop").unwrap();
        let r = f.reversed();
        assert_eq!(r.id, "col-loop~reversed");
        assert!(r.is_reversed() && !f.is_reversed());
        assert_eq!(r.base_id(), "col-loop");
        assert_eq!(r.name, "Col Loop · 14 km, 420 m climb · reversed");
        assert_eq!(r.shape(), Shape::Loop);
        assert!((r.length - f.length).abs() < 1e-6);
        assert!((r.ascent - f.ascent).abs() < 1e-6);
        for d in [0.0, 1234.0, 7000.0, 13_000.0] {
            let (a, b) = (r.at(d), f.at(f.length - d));
            assert!((a.x - b.x).hypot(a.z - b.z) < 0.5, "{d}");
            assert!((a.ele - b.ele).abs() < 0.01, "{d}");
            let turn = (a.heading - b.heading).rem_euclid(TAU);
            assert!((turn - std::f64::consts::PI).abs() < 0.05, "{d}: {turn}");
            assert!((r.grade(d) + f.grade(f.length - d)).abs() < 0.2, "{d}");
        }
    }

    #[test]
    fn a_point_to_point_route_reversed_starts_at_its_finish() {
        let mut xml = String::from(r#"<?xml version="1.0"?><gpx><trk><trkseg>"#);
        for i in 0..200 {
            let lat = 45.0 + i as f64 * 0.0001;
            xml += &format!(
                r#"<trkpt lat="{lat}" lon="7.0"><ele>{}</ele></trkpt>"#,
                100.0 + i as f64 * 0.5
            );
        }
        xml += "</trkseg></trk></gpx>";
        let f = parse_gpx(&xml, "gpx-climb", "Climb").unwrap().to_route();
        let r = f.reversed();
        assert_eq!(r.shape(), Shape::PointToPoint);
        assert_eq!(r.name, "Climb · reversed");
        assert!((r.length - f.length).abs() < 1e-6);
        let (start, finish) = (r.at(0.0), f.at(f.length));
        assert!((start.x - finish.x).hypot(start.z - finish.z) < 0.5);
        // Up one way, down the other, and a road on past the new finish.
        assert!(f.grade(1000.0) > 3.0 && r.grade(1000.0) < -3.0);
        assert!(r.ascent < 1.0 && f.ascent > 90.0);
        assert!(r.span() > r.length + 1000.0);
    }

    #[test]
    fn a_reversed_name_gives_the_climb_that_way() {
        let n = "Summit Road · 12 km, 600 m climb";
        assert_eq!(
            reversed_name(n, Some(85.4)),
            "Summit Road · 12 km, 85 m climb · reversed"
        );
        assert_eq!(reversed_name(n, None), format!("{n} · reversed"));
        assert_eq!(reversed_name("Alpe", Some(9.0)), "Alpe · reversed");
    }

    #[test]
    fn gpx_import() {
        let mut xml = String::from(r#"<?xml version="1.0"?><gpx><trk><name>Hill</name><trkseg>"#);
        for i in 0..100 {
            let lat = 45.0 + i as f64 * 0.0001; // ~11 m per point
            xml += &format!(
                r#"<trkpt lat="{lat}" lon="7.0"><ele>{}</ele></trkpt>"#,
                100.0 + i as f64 * 0.55
            );
        }
        xml += "</trkseg></trk></gpx>";
        let g = parse_gpx(&xml, "hill", "My hill").unwrap();
        // The name given (the file's), not the one inside the file.
        assert_eq!(g.name, "My hill");
        let r = g.to_route();
        assert!((r.length - 1100.0).abs() < 15.0, "{}", r.length);
        let grade = r.grade(500.0);
        assert!((grade - 5.0).abs() < 0.3, "{grade}");
    }
}

#[cfg(test)]
mod gpx_out {
    /// Turn a GPX file into an imported route as the app keeps it (JSON):
    /// `VELOFORGE_GPX_IN=a.gpx VELOFORGE_GPX_OUT=a.json cargo test gpx_to_json -- --ignored`.
    #[test]
    #[ignore]
    fn gpx_to_json() {
        let src = std::env::var("VELOFORGE_GPX_IN").expect("VELOFORGE_GPX_IN");
        let out = std::env::var("VELOFORGE_GPX_OUT").expect("VELOFORGE_GPX_OUT");
        let xml = std::fs::read_to_string(&src).unwrap();
        let mut g = super::parse_gpx(&xml, "gpx-test", "Test").unwrap();
        g.scenery = Some(super::Scenery::Alpine);
        let r = g.to_route();
        println!(
            "{} km, {} m climb, {:?}",
            r.length / 1000.0,
            r.ascent,
            r.shape()
        );
        std::fs::write(out, serde_json::to_string(&g).unwrap()).unwrap();
    }
}

#[cfg(test)]
mod grades {
    #[test]
    #[ignore]
    fn print_grades() {
        let json = std::fs::read_to_string(std::env::var("VELOFORGE_ROUTE_JSON").unwrap()).unwrap();
        let g: super::GpxRoute = serde_json::from_str(&json).unwrap();
        let r = g.to_route();
        let mut worst = (0.0f64, 0.0);
        let mut d = 0.0;
        while d < r.length {
            let gr = r.grade(d);
            if gr.abs() > worst.0.abs() {
                worst = (gr, d);
            }
            d += 10.0;
        }
        println!(
            "steepest {:.1} % at {:.0} m; {:.2} km, {:.0} m climb",
            worst.0,
            worst.1,
            r.length / 1000.0,
            r.ascent
        );
        // How far the road strays from the track, and how near stretches
        // far apart along it come.
        let track: Vec<(f64, f64)> = g.pts.iter().map(|p| (p.2, p.3)).collect();
        let course = r.course_points();
        let stray = course
            .iter()
            .map(|p| {
                track
                    .iter()
                    .map(|t| (t.0 - p.x).hypot(t.1 - p.z))
                    .fold(f64::MAX, f64::min)
            })
            .fold(0.0, f64::max);
        let mut closest = f64::MAX;
        for i in (0..course.len()).step_by(2) {
            for j in (i + 40..course.len()).step_by(2) {
                closest = closest.min((course[i].x - course[j].x).hypot(course[i].z - course[j].z));
            }
        }
        println!("strays up to {stray:.1} m from the track; stretches 200 m+ apart along come within {closest:.1} m");
        let pts = r.points();
        let mut i = 2;
        while i + 12 < pts.len() {
            let mut t = pts[i + 12].heading - pts[i].heading;
            while t > std::f64::consts::PI {
                t -= std::f64::consts::TAU;
            }
            while t < -std::f64::consts::PI {
                t += std::f64::consts::TAU;
            }
            if t.abs() > 2.4 {
                let step = (i..i + 12)
                    .map(|k| {
                        let mut u = pts[k + 1].heading - pts[k].heading;
                        while u > std::f64::consts::PI {
                            u -= std::f64::consts::TAU;
                        }
                        while u < -std::f64::consts::PI {
                            u += std::f64::consts::TAU;
                        }
                        u.abs().to_degrees()
                    })
                    .fold(0.0, f64::max);
                println!(
                    "hairpin at {:.0} m: {:.0} deg in 60 m, up to {:.1} deg a step",
                    i as f64 * 5.0,
                    t.to_degrees(),
                    step
                );
                i += 20;
            } else {
                i += 1;
            }
        }
    }
}
