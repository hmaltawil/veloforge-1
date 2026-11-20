//! Random routes: a new loop of road from a seed, a scenery, a length and
//! how hilly it is. The same choices always make the same route, so only
//! the choices are saved.
//!
//! The road's bends come from a few waves of curvature closed into one
//! loop (as the built-in routes are made), with sizes and counts drawn at
//! random for the length; a try is kept only if the road never comes back
//! near itself, nor bends too tightly. Its hills are waves too, scaled so
//! no slope is steeper than the hilliness allows.

use crate::route::{integrate_loop, Route, Scenery, STEP};
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
}

impl Default for RandomRoute {
    fn default() -> Self {
        RandomRoute {
            seed: 0,
            scenery: Scenery::Hills,
            km: 20,
            hills: Hills::Rolling,
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
        for k in 0..TRIES {
            // Gentler with every try that fails.
            let calm = 1.0 - 0.65 * k as f64 / TRIES as f64;
            let t = turns(&mut rng, km, self.scenery, n, calm);
            let p = integrate_loop(n, |u| t[((u * n as f64) as usize).min(n - 1)], |_| 0.0);
            // Checked with the headings the route will have.
            let r = Route::from_points("", "", self.scenery, 0, p);
            if !keeps_clear(r.points()) {
                continue;
            }
            for _ in 0..6 {
                let eles = heights(&mut rng, km, self.hills, n);
                if gentle_between(r.points(), &eles) {
                    found = Some((r.points().to_vec(), eles));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        // A plain round loop, flat, never fails.
        let (mut pts, eles) =
            found.unwrap_or_else(|| (integrate_loop(n, |_| 1.0, |_| 0.0), vec![0.0; n + 1]));
        let base = match self.scenery {
            Scenery::Hills => 110.0,
            Scenery::Lake => 40.0,
            Scenery::Alpine => 680.0,
            Scenery::Coast => 18.0,
        };
        for (p, e) in pts.iter_mut().zip(&eles) {
            p.ele = base + e;
        }
        let mut route = Route::from_points(&self.id(), "", self.scenery, self.seed % 997, pts);
        route.name = format!(
            "{} · {:.0} km, {:.0} m climb",
            name(&mut Rng(name_rng_seed), self.scenery),
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
fn turns(rng: &mut Rng, km: f64, scenery: Scenery, n: usize, calm: f64) -> Vec<f64> {
    let step_turn = TAU / n as f64;
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
            step_turn * (1.0 + lobes.iter().map(|w| w.at(u)).sum::<f64>())
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
        let radius = (radii.0 + (radii.1 - radii.0) * rng.next()) / calm.sqrt();
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
    let mean = (t.iter().sum::<f64>() - TAU) / n as f64;
    for v in t.iter_mut() {
        *v -= mean;
    }
    t
}

/// Whether a loop keeps clear of itself (no two stretches far apart along
/// it come close) and bends no tighter than `TIGHTEST`.
fn keeps_clear(pts: &[crate::route::RoutePt]) -> bool {
    let n = pts.len() - 1;
    // Bends, over 20 m.
    let k = 4;
    for i in 0..n {
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
    // Every 20 m into a grid of `APART` cells; neighbours compared.
    let step = 4;
    let cell = APART;
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
        d.min(n as f64 * STEP - d)
    };
    for i in (0..n).step_by(step) {
        let (cx, cz) = (
            (pts[i].x / cell).floor() as i64,
            (pts[i].z / cell).floor() as i64,
        );
        for dx in -1..=1 {
            for dz in -1..=1 {
                for &j in grid.get(&(cx + dx, cz + dz)).into_iter().flatten() {
                    if along(i, j) > APART_ALONG
                        && (pts[i].x - pts[j].x).hypot(pts[i].z - pts[j].z) < APART
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
fn gentle_between(pts: &[crate::route::RoutePt], eles: &[f64]) -> bool {
    const NEAR_ENOUGH: f64 = 600.0;
    // The steepest the land between may average.
    const SLOPE: f64 = 0.28;
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
                        d.min(n as f64 * STEP - d)
                    };
                    if along < APART_ALONG {
                        continue;
                    }
                    let d = (pts[i].x - pts[j].x).hypot(pts[i].z - pts[j].z);
                    // The road's own verges (about 30 m each) are level.
                    if d < NEAR_ENOUGH && (eles[i] - eles[j]).abs() > SLOPE * (d - 60.0).max(0.0) {
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
fn heights(rng: &mut Rng, km: f64, hills: Hills, n: usize) -> Vec<f64> {
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
        let left: f64 = rise.iter().sum();
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
    let gap = h[n] - h[0];
    for (i, v) in h.iter_mut().enumerate() {
        *v -= gap * i as f64 / n as f64;
    }
    let span = 6i64;
    let smooth: Vec<f64> = (0..n)
        .map(|i| {
            (-span..=span)
                .map(|j| h[(i as i64 + j).rem_euclid(n as i64) as usize])
                .sum::<f64>()
                / (2 * span + 1) as f64
        })
        .collect();
    let lowest = smooth.iter().cloned().fold(f64::MAX, f64::min);
    let mut out: Vec<f64> = smooth.iter().map(|v| v - lowest).collect();
    out.push(out[0]);
    out
}

/// A name for a route, from words that suit its scenery.
fn name(rng: &mut Rng, scenery: Scenery) -> String {
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
    let kind = if b.starts_with(' ') {
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
                assert!(keeps_clear(r.points()), "{seed} {scenery:?} {km} km");
                let eles: Vec<f64> = r.points().iter().map(|p| p.ele).collect();
                assert!(
                    gentle_between(r.points(), &eles),
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
                let n = name(&mut Rng(seed), scenery);
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
