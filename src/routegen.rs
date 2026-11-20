//! Random routes: a new loop of road from a seed, a scenery, a length and
//! how hilly it is. The same choices always make the same route, so only
//! the choices are saved.
//!
//! The road's bends come from a few waves of curvature closed into one
//! loop (as the built-in routes are made), with sizes and counts drawn at
//! random for the length; a try is kept only if the road never comes back
//! near itself, nor bends too tightly. Its hills are waves too, scaled so
//! no slope is steeper than the hilliness allows.

use crate::route::{integrate_loop, Route, RoutePt, Scenery, Shape, STEP};
use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

/// How hilly a random route is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Hills {
    Flat,
    Rolling,
    Hilly,
    Mountains,
}

impl Hills {
    pub const ALL: [Hills; 4] = [Hills::Flat, Hills::Rolling, Hills::Hilly, Hills::Mountains];

    pub fn label(self) -> &'static str {
        match self {
            Hills::Flat => "Flat",
            Hills::Rolling => "Rolling",
            Hills::Hilly => "Hilly",
            Hills::Mountains => "Mountains",
        }
    }

    /// The steepest slope allowed (percent).
    fn steepest(self) -> f64 {
        match self {
            Hills::Flat => 2.0,
            Hills::Rolling => 5.0,
            Hills::Hilly => 8.5,
            Hills::Mountains => 10.0,
        }
    }

    /// Climbing per kilometre aimed at (metres), before the slopes are
    /// held to `steepest`.
    fn climb_per_km(self) -> f64 {
        match self {
            Hills::Flat => 2.5,
            Hills::Rolling => 9.0,
            Hills::Hilly => 18.0,
            Hills::Mountains => 30.0,
        }
    }
}

/// What a random route is made from: these always make the same route.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RandomRoute {
    pub seed: u64,
    pub scenery: Scenery,
    pub km: u32,
    pub hills: Hills,
    /// A loop, or from one place to another.
    #[serde(default)]
    pub shape: Shape,
}

impl Default for RandomRoute {
    fn default() -> Self {
        RandomRoute {
            seed: 0,
            scenery: Scenery::Hills,
            km: 20,
            hills: Hills::Rolling,
            shape: Shape::Loop,
        }
    }
}

/// A seed no route made before has had (as near as matters): the time,
/// well mixed.
pub fn fresh_seed() -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1);
    let mut r = Rng(t);
    r.next().to_bits() ^ t.rotate_left(17)
}

/// Lengths a random route may have (km).
pub const KM_RANGE: std::ops::RangeInclusive<u32> = 5..=60;

/// The closest two stretches of road more than `APART_ALONG` apart along
/// it may come to each other: room for fields, forest or water between.
const APART: f64 = 170.0;
const APART_ALONG: f64 = 700.0;
/// The tightest bend allowed (radius, metres).
const TIGHTEST: f64 = 32.0;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next()
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[((self.next() * items.len() as f64) as usize).min(items.len() - 1)]
    }
}

/// One wave of curvature or height: how many times it comes round in the
/// loop, how big it is, where it starts, and whether its peaks are sharp
/// (cubed: bends with straights between them).
#[derive(Clone, Copy)]
struct Wave {
    times: f64,
    size: f64,
    phase: f64,
    sharp: bool,
}

impl Wave {
    fn at(&self, u: f64) -> f64 {
        let s = (u * TAU * self.times + self.phase).sin();
        self.size * if self.sharp { s * s * s } else { s }
    }
}

impl RandomRoute {
    pub fn id(&self) -> String {
        format!("random-{:016x}", self.seed)
    }

    /// Make the route.
    pub fn route(&self) -> Route {
        let km = (self.km.clamp(*KM_RANGE.start(), *KM_RANGE.end())) as f64;
        let n = (km * 1000.0 / STEP) as usize;
        let mut rng = Rng(self.seed);
        let name_rng_seed = rng.next().to_bits();
        // Bends and hills: tries until the road keeps clear of itself, and
        // wherever it passes near itself the heights differ little enough
        // for a hillside between, not a cliff.
        let mut found = None;
        const TRIES: usize = 120;
        let looped = self.shape == Shape::Loop;
        // Point to point, the road climbs (or drops) as a whole: to a summit
        // in the mountains.
        let net = (!looped).then(|| {
            let climb = self.hills.climb_per_km() * km;
            match self.hills {
                Hills::Mountains => 0.55 * climb,
                Hills::Hilly => 0.25 * climb,
                _ => climb * 0.15 * (rng.next() * 2.0 - 1.0),
            }
        });
        for k in 0..TRIES {
            // Gentler with every try that fails.
            let calm = 1.0 - 0.65 * k as f64 / TRIES as f64;
            let t = turns(&mut rng, km, self.scenery, n, calm, looped);
            let p = if looped {
                integrate_loop(n, |u| t[((u * n as f64) as usize).min(n - 1)], |_| 0.0)
            } else {
                integrate_open(&t, 0.0, 0.0, 0.0, 0.0)
            };
            // Checked with the headings the route will have.
            let r = Route::from_points("", "", self.scenery, 0, p);
            if !keeps_clear(r.points(), looped, 0) {
                continue;
            }
            for _ in 0..6 {
                let eles = heights(&mut rng, km, self.hills, n, net);
                if gentle_between(r.points(), &eles, looped, 0) {
                    found = Some((r.points().to_vec(), eles));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        // A plain round loop or straight road, flat, never fails.
        let (mut pts, eles) = found.unwrap_or_else(|| {
            let p = if looped {
                integrate_loop(n, |_| 1.0, |_| 0.0)
            } else {
                integrate_open(&vec![0.0; n], 0.0, 0.0, 0.0, 0.0)
            };
            (p, vec![0.0; n + 1])
        });
        let base = match self.scenery {
            Scenery::Hills => 110.0,
            Scenery::Lake => 40.0,
            Scenery::Alpine => 680.0,
            Scenery::Coast => 18.0,
        };
        for (p, e) in pts.iter_mut().zip(&eles) {
            p.ele = base + e;
        }
        let mut route = if looped {
            Route::from_points(&self.id(), "", self.scenery, self.seed % 997, pts)
        } else {
            let course = Route::from_points("", "", self.scenery, 0, pts);
            let beyond = beyond(course.points(), self.scenery, self.seed % 997);
            Route::point_to_point(
                &self.id(),
                "",
                self.scenery,
                self.seed % 997,
                course.points().to_vec(),
                beyond,
            )
        };
        route.name = format!(
            "{} · {:.0} km, {:.0} m climb",
            name(&mut Rng(name_rng_seed), self.scenery, looped),
            route.length / 1000.0,
            route.ascent
        );
        route
    }
}

/// How much the road turns at each of the `n` steps of a loop `km` long
/// (radians, one full turn in all): a few big lobes give the loop its
/// shape, and bends, each turning its own way by its own angle at its own
/// radius, give the road its character. Bends to the left and right
/// balance out; mountain and coast roads have more and tighter ones.
/// `calm` (0..1) scales how much the lobes and bends turn.
/// Point to point (not `looped`), the bends still balance out, but there is
/// no one turn round in all: the road ends heading much as it began.
fn turns(rng: &mut Rng, km: f64, scenery: Scenery, n: usize, calm: f64, looped: bool) -> Vec<f64> {
    let step_turn = TAU / n as f64;
    let (base, whole) = if looped { (1.0, TAU) } else { (0.0, 0.0) };
    let lobes: Vec<Wave> = (0..2)
        .map(|k| Wave {
            times: if k == 0 {
                *rng.pick(&[2.0, 3.0, 3.0, 4.0])
            } else {
                rng.range(4.0, 7.0).round()
            },
            size: rng.range(0.6, 2.2) / (1.0 + k as f64) * calm,
            phase: rng.range(0.0, TAU),
            sharp: rng.next() < 0.5,
        })
        .collect();
    let mut t: Vec<f64> = (0..n)
        .map(|i| {
            let u = i as f64 / n as f64;
            step_turn * (base + lobes.iter().map(|w| w.at(u)).sum::<f64>())
        })
        .collect();
    let (per_km, angles, radii) = match scenery {
        Scenery::Alpine => ((1.3, 2.2), (0.35, 1.9), (40.0, 180.0)),
        Scenery::Coast => ((0.9, 1.6), (0.25, 1.4), (55.0, 260.0)),
        Scenery::Lake => ((0.6, 1.1), (0.2, 1.1), (80.0, 380.0)),
        Scenery::Hills => ((0.8, 1.5), (0.25, 1.4), (60.0, 320.0)),
    };
    let len = n as f64 * STEP;
    let bends = (km * rng.range(per_km.0, per_km.1)).round() as usize;
    for _ in 0..bends {
        let at = rng.next() * len;
        // More gentle bends than sharp ones.
        let angle = (angles.0 + (angles.1 - angles.0) * rng.next().powf(1.6)) * calm;
        let way = if rng.next() < 0.5 { -1.0 } else { 1.0 };
        let radius = (radii.0 + (radii.1 - radii.0) * rng.next()) / calm.max(0.05).sqrt();
        // A bell of turning, its peak at the bend's radius.
        let sigma = angle * radius / (2.0 * std::f64::consts::PI).sqrt();
        let reach = (3.0 * sigma / STEP).ceil() as i64;
        let centre = at / STEP;
        for k in -reach..=reach {
            let i = (centre.round() as i64 + k).rem_euclid(n as i64) as usize;
            let d = (centre.round() + k as f64 - centre) * STEP;
            t[i] +=
                way * angle * STEP / (sigma * (TAU).sqrt()) * (-0.5 * (d / sigma).powi(2)).exp();
        }
    }
    let mean = (t.iter().sum::<f64>() - whole) / n as f64;
    for v in t.iter_mut() {
        *v -= mean;
    }
    t
}

/// Whether a road keeps clear of itself (no two stretches far apart along
/// it come close) and bends no tighter than `TIGHTEST`: round a loop if
/// `looped`, else from end to end. Only stretches from point `from` on are
/// held to it (a road added to one that is as it is).
fn keeps_clear(pts: &[RoutePt], looped: bool, from: usize) -> bool {
    keeps_clear_by(pts, looped, from, APART)
}

/// `keeps_clear`, with stretches far apart along the road kept `apart`
/// metres apart.
fn keeps_clear_by(pts: &[RoutePt], looped: bool, from: usize, apart: f64) -> bool {
    let n = pts.len() - 1;
    // Bends, over 20 m.
    let k = 4;
    for i in from..n {
        if !looped && i + k > n {
            break;
        }
        let a = pts[i];
        let b = pts[(i + k) % n];
        let mut turn = (b.heading - a.heading).rem_euclid(TAU);
        if turn > std::f64::consts::PI {
            turn = TAU - turn;
        }
        if turn > 1e-9 && (k as f64 * STEP) / turn < TIGHTEST {
            return false;
        }
    }
    // Every 20 m into a grid of `apart` cells; neighbours compared.
    let step = 4;
    let cell = apart;
    let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> =
        std::collections::HashMap::new();
    for i in (0..n).step_by(step) {
        let key = (
            (pts[i].x / cell).floor() as i64,
            (pts[i].z / cell).floor() as i64,
        );
        grid.entry(key).or_default().push(i);
    }
    let along = |i: usize, j: usize| {
        let d = i.abs_diff(j) as f64 * STEP;
        if looped {
            d.min(n as f64 * STEP - d)
        } else {
            d
        }
    };
    for i in (0..n).step_by(step) {
        let (cx, cz) = (
            (pts[i].x / cell).floor() as i64,
            (pts[i].z / cell).floor() as i64,
        );
        for dx in -1..=1 {
            for dz in -1..=1 {
                for &j in grid.get(&(cx + dx, cz + dz)).into_iter().flatten() {
                    if (i >= from || j >= from)
                        && along(i, j) > APART_ALONG
                        && (pts[i].x - pts[j].x).hypot(pts[i].z - pts[j].z) < apart
                    {
                        return false;
                    }
                }
            }
        }
    }
    true
}

/// Whether, wherever the road passes within `NEAR_ENOUGH` of another stretch
/// of itself (far from it along the road), the two differ in height little
/// enough for the land between to be a hillside rather than a cliff.
fn gentle_between(pts: &[RoutePt], eles: &[f64], looped: bool, from: usize) -> bool {
    gentle_between_by(pts, eles, looped, from, 0.28)
}

/// `gentle_between`, with the land between averaging no steeper than
/// `slope`.
fn gentle_between_by(pts: &[RoutePt], eles: &[f64], looped: bool, from: usize, slope: f64) -> bool {
    const NEAR_ENOUGH: f64 = 600.0;
    let n = pts.len() - 1;
    let step = 6;
    let cell = NEAR_ENOUGH;
    let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> =
        std::collections::HashMap::new();
    let key = |i: usize| {
        (
            (pts[i].x / cell).floor() as i64,
            (pts[i].z / cell).floor() as i64,
        )
    };
    for i in (0..n).step_by(step) {
        grid.entry(key(i)).or_default().push(i);
    }
    for i in (0..n).step_by(step) {
        let (cx, cz) = key(i);
        for dx in -1..=1 {
            for dz in -1..=1 {
                for &j in grid.get(&(cx + dx, cz + dz)).into_iter().flatten() {
                    let along = {
                        let d = i.abs_diff(j) as f64 * STEP;
                        if looped {
                            d.min(n as f64 * STEP - d)
                        } else {
                            d
                        }
                    };
                    if along < APART_ALONG || (i < from && j < from) {
                        continue;
                    }
                    let d = (pts[i].x - pts[j].x).hypot(pts[i].z - pts[j].z);
                    // The road's own verges (about 30 m each) are level.
                    if d < NEAR_ENOUGH && (eles[i] - eles[j]).abs() > slope * (d - 60.0).max(0.0) {
                        return false;
                    }
                }
            }
        }
    }
    true
}

/// Heights along a loop of `n` steps (metres above its lowest point): a
/// long climb or two in the mountains, waves of hills elsewhere, scaled to
/// the climbing aimed at and then held under the steepest slope allowed.
///
/// A loop (`net` None) ends where it began; otherwise the road ends `net`
/// metres higher (or lower) than it starts.
fn heights(rng: &mut Rng, km: f64, hills: Hills, n: usize, net: Option<f64>) -> Vec<f64> {
    let target = net.unwrap_or(0.0);
    // Hills of every size, the longer ones higher, as land is: waves from
    // the whole loop's length down to about 600 m.
    let mut waves = Vec::new();
    let shortest = (km * 1.6).max(2.0);
    let count = 9;
    for k in 0..count {
        let times = shortest
            .powf(k as f64 / (count - 1) as f64)
            .round()
            .max(1.0);
        waves.push(Wave {
            times,
            size: times.powf(-1.15) * rng.range(0.35, 1.0),
            phase: rng.range(0.0, TAU),
            sharp: false,
        });
    }
    if hills == Hills::Mountains {
        // One climb to a pass per 25 km or so, as a long, smooth swell over
        // the rest.
        let times = (km / 25.0).round().max(1.0);
        waves.push(Wave {
            times,
            size: 2.5 / times,
            phase: rng.range(0.0, TAU),
            sharp: false,
        });
    }
    let raw: Vec<f64> = (0..=n)
        .map(|i| {
            let u = i as f64 / n as f64;
            waves.iter().map(|w| w.at(u)).sum::<f64>()
        })
        .collect();
    // Scale to the climbing aimed at.
    let climbed: f64 = raw.windows(2).map(|w| (w[1] - w[0]).max(0.0)).sum();
    let k = if climbed > 1e-9 {
        hills.climb_per_km() * km / climbed
    } else {
        0.0
    };
    // Then hold every slope under the steepest allowed, as a road is built:
    // a climb too steep becomes a longer, steady one. Slopes are capped, and
    // what that takes off the loop's heights is shared among the others so
    // the loop still ends where it starts.
    let limit = hills.steepest() / 100.0 * STEP;
    let mut rise: Vec<f64> = raw.windows(2).map(|w| (w[1] - w[0]) * k).collect();
    for _ in 0..8 {
        for r in rise.iter_mut() {
            *r = r.clamp(-limit, limit);
        }
        let left: f64 = rise.iter().sum::<f64>() - target;
        let free = rise.iter().filter(|r| r.abs() < limit).count();
        if left.abs() < 1e-6 || free == 0 {
            break;
        }
        let share = left / free as f64;
        for r in rise.iter_mut().filter(|r| r.abs() < limit) {
            *r -= share;
        }
    }
    let mut h = Vec::with_capacity(n + 1);
    let mut e = 0.0;
    h.push(e);
    for r in &rise {
        e += r;
        h.push(e);
    }
    // Close any last gap, then smooth over about 60 m (round the loop) so
    // the slope changes gently.
    let gap = h[n] - h[0] - target;
    for (i, v) in h.iter_mut().enumerate() {
        *v -= gap * i as f64 / n as f64;
    }
    let span = 6i64;
    let at = |i: i64| -> f64 {
        if net.is_none() {
            h[i.rem_euclid(n as i64) as usize]
        } else {
            h[i.clamp(0, n as i64) as usize]
        }
    };
    let ends = if net.is_none() { n } else { n + 1 };
    let smooth: Vec<f64> = (0..ends)
        .map(|i| (-span..=span).map(|j| at(i as i64 + j)).sum::<f64>() / (2 * span + 1) as f64)
        .collect();
    let lowest = smooth.iter().cloned().fold(f64::MAX, f64::min);
    let mut out: Vec<f64> = smooth.iter().map(|v| v - lowest).collect();
    if net.is_none() {
        out.push(out[0]);
    }
    out
}

/// A road from (`x`, `z`) heading `heading` (from north), turning by `t`
/// radians at each step: its points, `STEP` apart, at height `ele`.
fn integrate_open(t: &[f64], x: f64, z: f64, heading: f64, ele: f64) -> Vec<RoutePt> {
    let mut pts = Vec::with_capacity(t.len() + 1);
    let (mut x, mut z, mut heading) = (x, z, heading);
    pts.push(RoutePt { x, z, ele, heading });
    for &turn in t {
        let mid = heading + turn / 2.0;
        x += mid.sin() * STEP;
        z += mid.cos() * STEP;
        heading += turn;
        pts.push(RoutePt { x, z, ele, heading });
    }
    pts
}

/// How far the road beyond a point-to-point route's finish runs: long
/// enough for any workout still going.
pub const BEYOND_KM: f64 = 40.0;

/// The road beyond the finish of a point-to-point `course`: carrying on from
/// its last point the way it was heading, bending like the scenery's roads,
/// easing down off a climb and then rolling gently, keeping clear of the
/// course (and of a cliff's height away from it). Its points follow the
/// course's last, `STEP` apart.
pub fn beyond(course: &[RoutePt], scenery: Scenery, seed: u64) -> Vec<RoutePt> {
    let n = (BEYOND_KM * 1000.0 / STEP) as usize;
    let end = course[course.len() - 1];
    let before = course[course.len().saturating_sub(3)];
    let heading = (end.x - before.x).atan2(end.z - before.z);
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xB0_7D);
    let from = course.len();
    let make = |rng: &mut Rng, calm: f64| -> Vec<RoutePt> {
        let mut t = turns(rng, BEYOND_KM, scenery, n, calm, false);
        // Straight on for the first few hundred metres, past the finish.
        for (i, v) in t.iter_mut().enumerate().take(80) {
            *v *= i as f64 / 80.0;
        }
        let rolling = heights(rng, BEYOND_KM, Hills::Rolling, n, Some(0.0));
        let mut pts = integrate_open(&t, end.x, end.z, heading, end.ele);
        for (i, p) in pts.iter_mut().enumerate() {
            let d = i as f64 * STEP;
            // Down 2 % for the first 1.5 km, then rolling, easing in.
            let ease = (d / 1500.0).min(1.0);
            p.ele = end.ele - 0.02 * d.min(1500.0) + (rolling[i] - rolling[0]) * ease * 0.4;
        }
        pts
    };
    for k in 0..60 {
        let calm = 1.0 - 0.7 * k as f64 / 60.0;
        let road = make(&mut rng, calm);
        let mut all: Vec<RoutePt> = course.to_vec();
        all.extend_from_slice(&road[1..]);
        let all = Route::from_points("", "", scenery, 0, all);
        let eles: Vec<f64> = all.points().iter().map(|p| p.ele).collect();
        // A real course may have its own stretches close together (the
        // legs of a hairpin climb): the road beyond keeps 90 m from it,
        // and the land between no steeper than the land allows (see
        // `base_height` in `gfx::terrain`).
        if keeps_clear_by(all.points(), false, from, 90.0)
            && gentle_between_by(all.points(), &eles, false, from, 0.55)
        {
            return road[1..].to_vec();
        }
    }
    // Straight on, gently down then level.
    let mut road = integrate_open(&vec![0.0; n], end.x, end.z, heading, end.ele);
    for (i, p) in road.iter_mut().enumerate() {
        p.ele = end.ele - 0.02 * (i as f64 * STEP).min(1500.0);
    }
    road[1..].to_vec()
}

/// A name for a route, from words that suit its scenery.
fn name(rng: &mut Rng, scenery: Scenery, looped: bool) -> String {
    let (first, second): (&[&str], &[&str]) = match scenery {
        Scenery::Hills => (
            &[
                "Oak", "Mill", "Hay", "Thorn", "Fox", "Ash", "Elder", "Bramble", "Kestrel",
                "Barley", "Hazel", "Lark",
            ],
            &[
                "field", "ford", "hurst", "ley", "wick", "combe", "dale", "bury", "wood",
            ],
        ),
        Scenery::Lake => (
            &[
                "Still", "Reed", "Heron", "Willow", "Swan", "Glass", "Silver", "Alder", "Otter",
                "Rush",
            ],
            &["water", "mere", "shore", "haven", "holm", "wick", "bank"],
        ),
        Scenery::Alpine => (
            &[
                "High", "Eagle", "Ice", "Granite", "Larch", "Cloud", "Chamois", "Grey", "Storm",
                "Pine",
            ],
            &[" Col", " Pass", "horn", " Fell", " Peak", " Alp", " Ridge"],
        ),
        Scenery::Coast => (
            &[
                "Gull", "Salt", "Cliff", "Tide", "Kelp", "Puffin", "Amber", "Shell", "Wren",
                "Spray",
            ],
            &["head", " Cove", " Point", " Bay", "strand", "ness", "haven"],
        ),
    };
    let a = rng.pick(first);
    // A second word that joins the first cleanly ("Glassshore" does not).
    let mut b = rng.pick(second);
    for _ in 0..8 {
        let joined = !b.starts_with(' ');
        let clash = a.chars().last().map(|c| c.to_ascii_lowercase()) == b.chars().next();
        if !(joined && clash) {
            break;
        }
        b = rng.pick(second);
    }
    // "Granite Pass Loop", not "Granite Pass Round".
    let kind = if !looped {
        rng.pick(&["Road", "Way", "Run"])
    } else if b.starts_with(' ') {
        rng.pick(&["Loop", "Circuit"])
    } else {
        rng.pick(&["Loop", "Circuit", "Round"])
    };
    format!("{a}{b} {kind}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(seed: u64, scenery: Scenery, km: u32, hills: Hills) -> RandomRoute {
        RandomRoute {
            seed,
            scenery,
            km,
            hills,
            shape: Shape::Loop,
        }
    }

    #[test]
    fn the_same_choices_make_the_same_route() {
        let a = spec(42, Scenery::Hills, 20, Hills::Rolling).route();
        let b = spec(42, Scenery::Hills, 20, Hills::Rolling).route();
        let c = spec(43, Scenery::Hills, 20, Hills::Rolling).route();
        assert_eq!(a.name, b.name);
        assert_eq!(a.points().len(), b.points().len());
        assert!(a
            .points()
            .iter()
            .zip(b.points())
            .all(|(p, q)| p.x == q.x && p.ele == q.ele));
        assert!(a.points()[100].x != c.points()[100].x);
    }

    #[test]
    fn routes_are_loops_of_the_length_asked_that_keep_clear_of_themselves() {
        for seed in 0..40u64 {
            for (scenery, km) in [
                (Scenery::Hills, 8),
                (Scenery::Lake, 25),
                (Scenery::Alpine, 40),
                (Scenery::Coast, 60),
            ] {
                let r = spec(seed, scenery, km, Hills::Hilly).route();
                assert!(r.is_loop(), "{seed} {scenery:?}");
                assert!((r.length / 1000.0 - km as f64).abs() < 0.1);
                assert!(
                    keeps_clear(r.points(), true, 0),
                    "{seed} {scenery:?} {km} km"
                );
                let eles: Vec<f64> = r.points().iter().map(|p| p.ele).collect();
                assert!(
                    gentle_between(r.points(), &eles, true, 0),
                    "{seed} {scenery:?} {km} km: a cliff"
                );
                // Never the plain round loop kept for when every try fails.
                let turning: f64 = r
                    .points()
                    .windows(2)
                    .map(|w| {
                        let d = (w[1].heading - w[0].heading).rem_euclid(TAU);
                        d.min(TAU - d)
                    })
                    .sum();
                assert!(turning > 1.2 * TAU, "{seed} {scenery:?} {km} km: too round");
            }
        }
    }

    #[test]
    fn point_to_point_routes_finish_away_from_the_start_and_carry_on() {
        for seed in 0..25u64 {
            for (scenery, km, hills) in [
                (Scenery::Alpine, 14, Hills::Mountains),
                (Scenery::Hills, 30, Hills::Rolling),
                (Scenery::Coast, 20, Hills::Hilly),
            ] {
                let r = RandomRoute {
                    shape: Shape::PointToPoint,
                    ..spec(seed, scenery, km, hills)
                }
                .route();
                let what = format!("{seed} {scenery:?} {km} km");
                assert_eq!(r.shape(), Shape::PointToPoint);
                assert!(!r.is_loop(), "{what}");
                assert!((r.length / 1000.0 - km as f64).abs() < 0.1, "{what}");
                // The road beyond: long enough for any workout, joined on.
                assert!(r.span() >= r.length + BEYOND_KM * 1000.0 - STEP, "{what}");
                let (f, a) = (r.at(r.length), r.at(r.length + STEP));
                assert!((a.x - f.x).hypot(a.z - f.z) < STEP * 1.01, "{what}");
                // Lookups do not wrap back to the start.
                let past = r.at(r.span() + 500.0);
                let last = r.points()[r.points().len() - 1];
                assert!(past.x == last.x && past.z == last.z, "{what}");
                let s = r.at(0.0);
                let fin = r.at(r.length);
                assert!(
                    (s.x - fin.x).hypot(s.z - fin.z) > 500.0,
                    "{what}: ends together"
                );
                assert!(
                    r.points()
                        .iter()
                        .all(|p| p.x.is_finite() && p.ele.is_finite()),
                    "{what}: not a number"
                );
                // The course by the strict rules; the road beyond by its own.
                let course = r.course_points();
                let eles: Vec<f64> = course.iter().map(|p| p.ele).collect();
                assert!(keeps_clear(course, false, 0), "{what}");
                assert!(gentle_between(course, &eles, false, 0), "{what}: a cliff");
                let all: Vec<f64> = r.points().iter().map(|p| p.ele).collect();
                let from = course.len();
                assert!(
                    keeps_clear_by(r.points(), false, from, 90.0),
                    "{what}: beyond"
                );
                assert!(
                    gentle_between_by(r.points(), &all, false, from, 0.55),
                    "{what}: a cliff beyond"
                );
                if hills == Hills::Mountains {
                    assert!(
                        fin.ele - s.ele > 0.3 * 30.0 * km as f64,
                        "{what}: no summit"
                    );
                }
                // Easing off past the finish: no climb steeper than 6 %.
                for k in 0..200 {
                    let d = r.length + 30.0 + k as f64 * 100.0;
                    assert!(r.grade(d).abs() < 6.5, "{what}: {:.1}% at {d}", r.grade(d));
                }
            }
        }
    }

    #[test]
    fn slopes_and_climbing_suit_the_hilliness() {
        for seed in 0..30u64 {
            let mut last = 0.0;
            for hills in Hills::ALL {
                let r = spec(seed, Scenery::Hills, 30, hills).route();
                let steepest = (0..r.points().len() * 5 / 10)
                    .map(|i| r.grade(i as f64 * 10.0).abs())
                    .fold(0.0, f64::max);
                assert!(
                    steepest <= hills.steepest() + 0.3,
                    "{seed} {hills:?}: {steepest:.1}%"
                );
                let per_km = r.ascent / 30.0;
                assert!(
                    per_km <= hills.climb_per_km() * 1.05,
                    "{seed} {hills:?}: {per_km}"
                );
                // Each step up in hilliness climbs more.
                assert!(per_km > last, "{seed} {hills:?}: {per_km} after {last}");
                last = per_km;
            }
        }
    }

    #[test]
    fn names_carry_the_length_and_climb() {
        let r = spec(7, Scenery::Alpine, 33, Hills::Mountains).route();
        assert!(r.name.contains(" · 33 km, "), "{}", r.name);
        assert!(r.name.ends_with(" m climb"), "{}", r.name);
        assert!(r.id.starts_with("random-"));
        for seed in 0..300 {
            for scenery in Scenery::ALL {
                let n = name(&mut Rng(seed), scenery, true);
                let lower = n.to_lowercase();
                let doubled = lower
                    .as_bytes()
                    .windows(3)
                    .any(|w| w[0] == w[1] && w[1] == w[2]);
                assert!(!doubled, "{n}");
                assert!(!n.contains("Pass Round") && !n.contains("Col Round"), "{n}");
            }
        }
    }
}

#[cfg(test)]
mod look {
    use super::*;

    /// Write some routes' plans and profiles to look at:
    /// `VELOFORGE_ROUTES_OUT=dir cargo test write_routes -- --ignored`.
    #[test]
    #[ignore]
    fn write_routes() {
        let dir = std::env::var("VELOFORGE_ROUTES_OUT").expect("VELOFORGE_ROUTES_OUT");
        let mut i = 0;
        for (scenery, km, hills) in [
            (Scenery::Hills, 12, Hills::Rolling),
            (Scenery::Hills, 30, Hills::Hilly),
            (Scenery::Lake, 20, Hills::Flat),
            (Scenery::Alpine, 40, Hills::Mountains),
            (Scenery::Coast, 25, Hills::Rolling),
            (Scenery::Alpine, 15, Hills::Hilly),
        ] {
            for seed in [11u64, 12] {
                let t = std::time::Instant::now();
                let r = RandomRoute {
                    seed,
                    scenery,
                    km,
                    hills,
                    shape: Shape::Loop,
                }
                .route();
                let ms = t.elapsed().as_millis();
                let mut out = format!("# {} ({ms} ms)\n", r.name);
                for p in r.points() {
                    out += &format!("{} {} {}\n", p.x, p.z, p.ele);
                }
                std::fs::write(format!("{dir}/route-{i}.txt"), out).unwrap();
                i += 1;
            }
        }
    }
}
