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
}

impl Scenery {
    pub const ALL: [Scenery; 4] = [
        Scenery::Hills,
        Scenery::Lake,
        Scenery::Alpine,
        Scenery::Coast,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Scenery::Hills => "Farmland and hills",
            Scenery::Lake => "Lakeside",
            Scenery::Alpine => "Mountains",
            Scenery::Coast => "Coast",
        }
    }

    /// Water at a fixed level beside the road (lake or sea).
    pub fn has_water(self) -> bool {
        matches!(self, Scenery::Lake | Scenery::Coast)
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
    pts: Vec<RoutePt>,
    pub length: f64,
    pub ascent: f64,
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
}

impl Route {
    fn from_points(
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
        }
    }

    /// Centreline points, one every `STEP` metres.
    pub fn points(&self) -> &[RoutePt] {
        &self.pts
    }

    /// Whether the route ends where it starts (a loop rather than point to point).
    pub fn is_loop(&self) -> bool {
        let (a, b) = (self.pts[0], self.pts[self.pts.len() - 1]);
        ((a.x - b.x).powi(2) + (a.z - b.z).powi(2)).sqrt() < 30.0
    }

    /// Sample the route at distance `d` (wraps around: every route is ridden as laps).
    pub fn at(&self, d: f64) -> RoutePt {
        let d = d.rem_euclid(self.length);
        let f = d / STEP;
        let i = (f.floor() as usize).min(self.pts.len() - 2);
        let u = f - i as f64;
        let a = self.pts[i];
        let b = self.pts[i + 1];
        let mut dh = b.heading - a.heading;
        if dh > std::f64::consts::PI {
            dh -= TAU;
        } else if dh < -std::f64::consts::PI {
            dh += TAU;
        }
        RoutePt {
            x: a.x + (b.x - a.x) * u,
            z: a.z + (b.z - a.z) * u,
            ele: a.ele + (b.ele - a.ele) * u,
            heading: a.heading + dh * u,
        }
    }

    /// Road grade in percent at distance `d`.
    pub fn grade(&self, d: f64) -> f64 {
        let h = 15.0;
        let mut a = d - h;
        let mut b = d + h;
        // Do not average across the start/finish seam of a non-loop route.
        if a < 0.0 {
            a = 0.0;
            b = 2.0 * h;
        } else if b > self.length {
            b = self.length;
            a = self.length - 2.0 * h;
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
        let amp = match self.scenery {
            Scenery::Alpine => 45.0,
            Scenery::Hills => 22.0,
            Scenery::Lake => 14.0,
            Scenery::Coast => 26.0,
        };
        let hill =
            0.55 + 0.25 * (d / 140.0 + s).sin() + 0.2 * (d / 53.0 + a / 31.0 + s * 2.0).sin();
        road + ramp * amp * hill - (1.0 - ramp) * 0.3 * (a / 9.0).min(1.0)
    }
}

/// Close a heading sequence into a loop and integrate positions.
fn integrate_loop(
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
    pub fn to_route(&self) -> Route {
        let pts = self
            .pts
            .iter()
            .map(|&(_, ele, x, z)| RoutePt {
                x,
                z,
                ele,
                heading: 0.0,
            })
            .collect();
        let seed = self
            .id
            .bytes()
            .fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64))
            % 97;
        Route::from_points(
            &self.id,
            &self.name,
            self.scenery.unwrap_or(Scenery::Hills),
            seed,
            pts,
        )
    }
}

/// Parse a GPX track or route, project it to local metres and resample.
pub fn parse_gpx(xml: &str, id: &str, fallback_name: &str) -> Result<GpxRoute> {
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
    let name = doc
        .descendants()
        .find(|n| n.has_tag_name("name"))
        .and_then(|n| n.text())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback_name.to_string());

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
        name,
        pts: out,
        scenery: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let g = parse_gpx(&xml, "hill", "x").unwrap();
        assert_eq!(g.name, "Hill");
        let r = g.to_route();
        assert!((r.length - 1100.0).abs() < 15.0, "{}", r.length);
        let grade = r.grade(500.0);
        assert!((grade - 5.0).abs() < 0.3, "{grade}");
    }
}
