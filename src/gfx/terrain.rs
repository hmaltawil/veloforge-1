//! World-space terrain around a route: a height function shaped by the
//! distance to the road (the road sits in a valley), forests, rocks,
//! bushes, houses and lakes. Windows of terrain are built on a worker thread.

use super::fields;
use super::flora::{Cover, Palette, Spot};
use crate::route::{Route, Scenery, STEP};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// Floats per terrain vertex: position (3), normal (3), road distance, forest.
pub const TERRAIN_STRIDE: usize = 8;
/// Cells per side of a terrain tile.
const TILE_CELLS: usize = 32;

/// A run of the terrain's index list and the box around its triangles.
#[derive(Debug, Clone, Copy)]
pub struct Tile {
    pub first: u32,
    pub count: u32,
    pub lo: [f32; 3],
    pub hi: [f32; 3],
}
/// Floats per object instance: position (3), rotation, scale (2), tint (3).
pub const INSTANCE_STRIDE: usize = 9;

/// Kinds of scattered object, indexing [`TerrainData::instances`].
pub const KINDS: usize = 13;
pub const PINE: usize = 0;
pub const BROADLEAF: usize = 1;
pub const ROCK: usize = 2;
pub const HOUSE: usize = 3;
pub const BIRCH: usize = 4;
pub const BUSH: usize = 5;
pub const BARN: usize = 6;
pub const SHED: usize = 7;
pub const TOWNHOUSE: usize = 8;
pub const OAK: usize = 9;
pub const MAPLE: usize = 10;
pub const BALE: usize = 11;
pub const WALL: usize = 12;

/// How wide, for its size, each kind is where it meets the ground (radius):
/// a tree's trunk for its width, a bush's base, a rock's, a bale's.
pub(super) const TREE_FOOT: f64 = 0.1;
pub(super) const BUSH_FOOT: f64 = 0.3;
pub(super) const ROCK_FOOT: f64 = 0.35;
pub(super) const BALE_FOOT: f64 = 0.7;

/// The lowest the `ground` is under something standing at `x`, `z` with
/// a base `r` across (radius): its middle and eight points round its edge.
/// Set there (and a little lower), no side of it stands over the slope.
pub(super) fn lowest(ground: &dyn Fn(f64, f64) -> f64, x: f64, z: f64, r: f64) -> f64 {
    (0..8).fold(ground(x, z), |lo, k| {
        let a = k as f64 * std::f64::consts::FRAC_PI_4;
        lo.min(ground(x + a.cos() * r, z + a.sin() * r))
    })
}

/// The farmland patchwork the ground shader paints (`TERRAIN_FS`, where
/// the scenery is farmland or a lake): fields 150 by 105 m on a grid turned
/// a little from north, each with its own crop. Worked out here exactly as
/// there, so what stands in the fields suits what grows in them.
struct Field {
    /// Which field.
    cell: (i64, i64),
    /// What grows in it, as the shader picks: under 0.2 ripe wheat, 0.2 to
    /// 0.32 rapeseed, to 0.46 young green, to 0.58 ploughed, to 0.74
    /// pasture; above that, plain meadow.
    crop: f32,
    /// Metres to the field's nearest edge.
    edge: f64,
    /// The turn (as an instance's, see `Bales`) of the field's rows.
    rows: f32,
}

impl Field {
    fn at(x: f64, z: f64) -> Field {
        // q = mat2(0.94, 0.34, -0.34, 0.94) * w / vec2(150, 105), w = (x, z);
        // GLSL matrices are column by column.
        let (wx, wz) = (x as f32, z as f32);
        let q = (
            (0.94 * wx - 0.34 * wz) / 150.0,
            (0.34 * wx + 0.94 * wz) / 105.0,
        );
        let cell = (q.0.floor(), q.1.floor());
        let f = (q.0 - cell.0, q.1 - cell.1);
        let crop = hash12(cell.0 + 3.7, cell.1 + 3.7);
        let edge = (f.0.min(1.0 - f.0) * 150.0).min(f.1.min(1.0 - f.1) * 105.0) as f64;
        // q.x grows along (0.94, -0.34) in (east, north), q.y along
        // (0.34, 0.94); rows run along q.x where the crop is over 0.5.
        let along_x = (-0.34f32).atan2(0.94);
        let rows = if crop > 0.5 {
            along_x
        } else {
            along_x + std::f32::consts::FRAC_PI_2
        };
        Field {
            cell: (cell.0 as i64, cell.1 as i64),
            crop,
            edge,
            rows,
        }
    }
}

/// The shaders' `hash12`, in the same 32-bit floats.
fn hash12(px: f32, py: f32) -> f32 {
    let fract = |v: f32| v - v.floor();
    let mut p3 = [fract(px * 0.1031), fract(py * 0.1031), fract(px * 0.1031)];
    let d = p3[0] * (p3[1] + 33.33) + p3[1] * (p3[2] + 33.33) + p3[2] * (p3[0] + 33.33);
    for v in p3.iter_mut() {
        *v += d;
    }
    fract((p3[0] + p3[1]) * p3[2])
}

/// Bales in one cell of a field: where each lies from the cell's spot,
/// which way, and their colour.
struct Bales {
    at: Vec<(f64, f64)>,
    rot: f32,
    tint: [f32; 3],
}

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn hash2(x: i64, z: i64, seed: u64) -> u64 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (z as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ seed.wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^ (h >> 29)
}

fn unit(h: u64) -> f64 {
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// Small deterministic random sequence (splitmix64) seeded from a hash.
pub(super) struct Rng(pub(super) u64);

impl Rng {
    pub(super) fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        unit(z ^ (z >> 31))
    }
}

/// Value noise in [-1, 1].
fn noise(x: f64, z: f64, seed: u64) -> f64 {
    let (xi, zi) = (x.floor(), z.floor());
    let (fx, fz) = (x - xi, z - zi);
    let (ux, uz) = (fx * fx * (3.0 - 2.0 * fx), fz * fz * (3.0 - 2.0 * fz));
    let v = |dx: i64, dz: i64| unit(hash2(xi as i64 + dx, zi as i64 + dz, seed)) * 2.0 - 1.0;
    let a = v(0, 0) + (v(1, 0) - v(0, 0)) * ux;
    let b = v(0, 1) + (v(1, 1) - v(0, 1)) * ux;
    a + (b - a) * uz
}

fn fbm(x: f64, z: f64, octaves: u32, seed: u64) -> f64 {
    let (mut sum, mut amp, mut norm, mut f) = (0.0, 0.5, 0.0, 1.0);
    for i in 0..octaves {
        sum += amp
            * noise(
                x * f + i as f64 * 17.3,
                z * f - i as f64 * 9.1,
                seed + i as u64,
            );
        norm += amp;
        amp *= 0.5;
        f *= 2.03;
    }
    sum / norm
}

#[derive(Debug, Clone, Copy)]
pub struct Near {
    pub dist: f64,
    /// Distance along the route of the closest point.
    pub d: f64,
    /// +1 right of the direction of travel, -1 left.
    pub side: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub h: f64,
    pub dist: f64,
    pub forest: f64,
    pub water: bool,
}

/// A farm building, or a tree in its garden, placed by `farmsteads`.
struct Building {
    kind: usize,
    /// Which farmstead it belongs to.
    farm: i64,
    x: f64,
    z: f64,
    /// The height it stands at.
    h: f64,
    rot: f32,
    /// Its scale (across, up), and its tint (a building's roof colour).
    scale: (f32, f32),
    tint: [f32; 3],
    /// How far it reaches from its centre (metres).
    reach: f64,
}

/// Coarse grid of the nearest route point index, refined on lookup.
struct NearGrid {
    x0: f64,
    z0: f64,
    cell: f64,
    nx: usize,
    nz: usize,
    idx: Vec<u32>,
}

pub struct RouteGeo {
    pub route: Route,
    pts: Vec<(f64, f64)>,
    eles: Vec<f64>,
    grid: NearGrid,
    /// Every point of the road by `SEG_CELL` square, to find all the road
    /// near a spot (not only the nearest).
    segs: std::collections::HashMap<(i64, i64), Vec<u32>>,
    looped: bool,
    /// Side (+1 right, -1 left) the lake or sea is on.
    water_side: f64,
    pub water_level: Option<f64>,
    pub max_ele: f64,
    seed: u64,
    /// Level pads the farm buildings stand on: where, how high, and how far
    /// the ground is level (it blends back into the land beyond).
    pads: Vec<(f64, f64, f64, f64)>,
    /// The farm buildings and their gardens' trees, for the whole route.
    farms: Vec<Building>,
}

impl RouteGeo {
    pub fn new(route: &Route) -> RouteGeo {
        let pts: Vec<(f64, f64)> = route.points().iter().map(|p| (p.x, p.z)).collect();
        let looped = route.is_loop();
        let (min_ele, max_ele) = route.min_max_ele();
        // Shoelace: positive area = counter-clockwise, interior on the left.
        let area: f64 = pts
            .windows(2)
            .map(|w| w[0].0 * w[1].1 - w[1].0 * w[0].1)
            .sum();
        let inside = if area > 0.0 { -1.0 } else { 1.0 };
        let water_level = match route.scenery {
            Scenery::Lake if looped => Some(min_ele - 0.8),
            Scenery::Coast => Some(min_ele - 4.0),
            _ => None,
        };
        // A lake fills the inside of a loop; the sea surrounds it (or lies
        // to the left of a point-to-point route).
        let water_side = match (route.scenery, looped) {
            (Scenery::Coast, true) => -inside,
            (Scenery::Coast, false) => -1.0,
            _ => inside,
        };
        let grid = NearGrid::build(&pts);
        let mut segs: std::collections::HashMap<(i64, i64), Vec<u32>> = Default::default();
        for (i, &(x, z)) in pts.iter().enumerate() {
            segs.entry(((x / SEG_CELL).floor() as i64, (z / SEG_CELL).floor() as i64))
                .or_default()
                .push(i as u32);
        }
        let mut geo = RouteGeo {
            route: route.clone(),
            eles: route.points().iter().map(|p| p.ele).collect(),
            pts,
            grid,
            segs,
            looped,
            water_side,
            water_level,
            max_ele,
            seed: route.seed,
            pads: Vec::new(),
            farms: Vec::new(),
        };
        // The farms: first where the buildings go on the land as it is, and
        // a level pad for each at the average height of its ground; then
        // again on the levelled land, with their gardens.
        if Palette::of(route.scenery).farmhouses {
            let everywhere = (-1.0e7, -1.0e7, 2.0e7);
            let first = geo.farmsteads(everywhere.0, everywhere.1, everywhere.2);
            // One level yard per farmstead: all its buildings at the average
            // height of their ground, so pads that overlap agree.
            let buildings: Vec<&Building> = first
                .iter()
                .filter(|b| [HOUSE, TOWNHOUSE, BARN, SHED].contains(&b.kind))
                .collect();
            let mut yard: std::collections::HashMap<i64, (f64, f64)> = Default::default();
            for b in &buildings {
                let e = yard.entry(b.farm).or_default();
                for k in 0..9 {
                    let a = k as f64 * std::f64::consts::FRAC_PI_4;
                    let r = if k == 8 { 0.0 } else { b.reach * 0.75 };
                    e.0 += geo.sample(b.x + a.cos() * r, b.z + a.sin() * r).h;
                    e.1 += 1.0;
                }
            }
            geo.pads = buildings
                .iter()
                .map(|b| {
                    let (sum, n) = yard[&b.farm];
                    // Houses' yards reach past their front gardens.
                    let level = if [HOUSE, TOWNHOUSE].contains(&b.kind) {
                        b.reach + 6.0
                    } else {
                        b.reach + 1.0
                    };
                    (b.x, b.z, sum / n, level)
                })
                .collect();
            geo.farms = geo.farmsteads(everywhere.0, everywhere.1, everywhere.2);
        }
        geo
    }

    /// Snow lies on the mountains well above the highest road, so a climb
    /// that finishes at its top finishes on grass and rock.
    /// The surface of the road over (`x`, `z`), if a road covers it: the
    /// lowest of every stretch of road there, each at its height where it
    /// is nearest, and its verges (beyond `ROAD_CROWN` of the middle) taken
    /// as low as their outer edges. (The land uses `surface_from`.)
    #[cfg(test)]
    fn road_surface(&self, x: f64, z: f64) -> Option<f64> {
        self.surface_from(&self.road_near(x, z, 0.0), x, z)
    }

    /// The road's segments that may cover a spot within `around` of
    /// (`x`, `z`), in order along the route: looked up once for many spots
    /// close together (`surface_from`).
    fn road_near(&self, x: f64, z: f64, around: f64) -> Vec<u32> {
        let (ci, cj) = ((x / SEG_CELL).floor() as i64, (z / SEG_CELL).floor() as i64);
        let k = ((ROAD_HALF_WIDTH + around) / SEG_CELL).ceil() as i64;
        let mut near: Vec<u32> = Vec::new();
        for dj in -k..=k {
            for di in -k..=k {
                if let Some(v) = self.segs.get(&(ci + di, cj + dj)) {
                    near.extend_from_slice(v);
                }
            }
        }
        near.sort_unstable();
        near.dedup();
        near
    }

    /// The road's surface over (`x`, `z`) (as `road_surface`), from the
    /// segments `near` it (`road_near`).
    fn surface_from(&self, near: &[u32], x: f64, z: f64) -> Option<f64> {
        let n = self.pts.len();
        // Where along segment `i` the spot is nearest: how far from it, the
        // road's height there, and whether the spot is square on to it.
        let seg = |i: u32| {
            let j = i as usize;
            let ((ax, az), (bx, bz)) = (self.pts[j], self.pts[j + 1]);
            let (dx, dz) = (bx - ax, bz - az);
            let l2 = (dx * dx + dz * dz).max(1e-9);
            let along = ((x - ax) * dx + (z - az) * dz) / l2;
            let t = along.clamp(0.0, 1.0);
            let d = (x - ax - dx * t).hypot(z - az - dz * t);
            let ele = self.eles[j] + (self.eles[j + 1] - self.eles[j]) * t;
            (d, ele, (0.0..1.0).contains(&along))
        };
        let near = match near.iter().position(|&i| i as usize + 1 >= n) {
            Some(last) => &near[..last],
            None => near,
        };
        let mut cap: Option<f64> = None;
        // Each run of neighbouring points is one stretch of road.
        let mut start = 0;
        while start < near.len() {
            let mut end = start + 1;
            while end < near.len() && near[end] - near[end - 1] <= 8 {
                end += 1;
            }
            // Every stretch of the run the spot is square on to (or nearest
            // to, round a bend) holds the ground down: the two legs of a
            // hairpin are one run joined by its bend, and the spot may be
            // nearer the one than the other all the way round.
            let mut prev: Option<(f64, f64, bool)> = None;
            let mut here = seg(near[start]);
            for k in start..end {
                let next = (k + 1 < end).then(|| seg(near[k + 1]));
                let (d, ele, square) = here;
                let before = prev.is_none_or(|p| p.0 >= d);
                let after = next.is_none_or(|q| q.0 >= d);
                if (square || before && after) && d <= ROAD_HALF_WIDTH {
                    let top = ele - if d > ROAD_CROWN { ROAD_VERGE_DROP } else { 0.0 };
                    cap = Some(cap.map_or(top, |c: f64| c.min(top)));
                }
                prev = Some(here);
                if let Some(q) = next {
                    here = q;
                }
            }
            start = end;
        }
        cap
    }

    /// How far each corner of a land triangle with heights `h` must come
    /// down so that nowhere under a road is it higher than `ROAD_BED` below
    /// the road's surface (`surface` at a point given by its weights on the
    /// corners). As little as that takes: each time, the corners nearest the
    /// point most over the road come down the most.
    fn bed_drops(&self, h: [f64; 3], surface: impl Fn([f64; 3]) -> Option<f64>) -> [f64; 3] {
        // The land is flat across the triangle, so it is furthest over the
        // road where the road's edge or the foot of its verge crosses the
        // triangle's sides (or at a corner): those crossings, found to a few
        // millimetres, and points across the triangle about a metre apart
        // for roads that bend or end inside it.
        const STEPS: usize = 8;
        const SIDE_STEPS: usize = 16;
        // A step in what covers the spot: the road's edge, or a verge's foot.
        let changes = |s0: Option<f64>, s1: Option<f64>| match (s0, s1) {
            (Some(a), Some(b)) => (a - b).abs() > 0.1,
            (a, b) => a.is_some() != b.is_some(),
        };
        let mut under: Vec<([f64; 3], f64)> = Vec::new();
        let mut first = None;
        let mut mixed = false;
        for p in 0..=STEPS {
            for q in 0..=STEPS - p {
                let (u, v) = (p as f64 / STEPS as f64, q as f64 / STEPS as f64);
                let w = [1.0 - u - v, u, v];
                let top = surface(w);
                let f = *first.get_or_insert(top);
                mixed |= changes(f, top);
                if let Some(top) = top {
                    under.push((w, top - ROAD_BED));
                }
            }
        }
        let corner = |k: usize| {
            let mut w = [0.0; 3];
            w[k] = 1.0;
            w
        };
        let between = |a: [f64; 3], b: [f64; 3], t: f64| -> [f64; 3] {
            std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
        };
        // The triangle's sides, and where a road's edge runs inside it, lines
        // across it a metre apart each way (an edge bending inside crosses
        // those).
        let mut lines: Vec<([f64; 3], [f64; 3])> =
            (0..3).map(|k| (corner(k), corner((k + 1) % 3))).collect();
        for p in (1..STEPS).filter(|_| mixed) {
            let t = p as f64 / STEPS as f64;
            for k in 0..3 {
                let (a, b, c) = (corner(k), corner((k + 1) % 3), corner((k + 2) % 3));
                lines.push((between(a, b, t), between(a, c, t)));
            }
        }
        for (a, b) in lines {
            let mut prev = (0.0, surface(a));
            for step in 1..=SIDE_STEPS {
                let t = step as f64 / SIDE_STEPS as f64;
                let here = (t, surface(between(a, b, t)));
                if changes(prev.1, here.1) {
                    // Narrow it down, keeping the two sides of the step.
                    let (mut lo, mut hi) = (prev, here);
                    for _ in 0..7 {
                        let mid_t = (lo.0 + hi.0) / 2.0;
                        let mid = (mid_t, surface(between(a, b, mid_t)));
                        if changes(lo.1, mid.1) {
                            hi = mid;
                        } else {
                            lo = mid;
                        }
                    }
                    for (t, top) in [lo, hi] {
                        if let Some(top) = top {
                            under.push((between(a, b, t), top - ROAD_BED));
                        }
                    }
                }
                prev = here;
            }
        }
        let mut lower = [0.0f64; 3];
        let over = |lower: &[f64; 3], w: &[f64; 3], cap: f64| {
            (0..3).map(|k| w[k] * (h[k] - lower[k])).sum::<f64>() - cap
        };
        for _ in 0..12 {
            let Some((w, e)) = under
                .iter()
                .map(|(w, cap)| (*w, over(&lower, w, *cap)))
                .max_by(|a, b| a.1.total_cmp(&b.1))
            else {
                return lower;
            };
            if e <= 1e-4 {
                return lower;
            }
            // The least change that brings that point down to the cap.
            let n2: f64 = w.iter().map(|x| x * x).sum();
            for k in 0..3 {
                lower[k] += e * w[k] / n2;
            }
        }
        // Whatever is left over, all three together.
        let left = under
            .iter()
            .map(|(w, cap)| over(&lower, w, *cap))
            .fold(0.0, f64::max);
        lower.map(|l| l + left)
    }

    pub fn snow_level(&self) -> f64 {
        self.max_ele + 150.0
    }

    pub fn nearest(&self, x: f64, z: f64) -> Near {
        let mut best = Near {
            dist: f64::MAX,
            d: 0.0,
            side: 1.0,
        };
        for i0 in self.grid.lookup(x, z) {
            self.refine(x, z, i0 as i64, &mut best);
        }
        best
    }

    /// Search the route segments around index `i0` for a point nearer than `best`.
    fn refine(&self, x: f64, z: f64, i0: i64, best: &mut Near) {
        let n = self.pts.len();
        for k in -40..=40 {
            let mut j = i0 + k;
            if self.looped {
                j = j.rem_euclid(n as i64 - 1);
            } else if j < 0 || j + 1 >= n as i64 {
                continue;
            }
            let j = j as usize;
            let (ax, az) = self.pts[j];
            let (bx, bz) = self.pts[j + 1];
            let (dx, dz) = (bx - ax, bz - az);
            let l2 = (dx * dx + dz * dz).max(1e-9);
            let t = (((x - ax) * dx + (z - az) * dz) / l2).clamp(0.0, 1.0);
            let (qx, qz) = (ax + dx * t, az + dz * t);
            let dist = ((x - qx).powi(2) + (z - qz).powi(2)).sqrt();
            if dist < best.dist {
                // Right of travel is the direction rotated clockwise.
                let side = if dz * (x - qx) - dx * (z - qz) >= 0.0 {
                    1.0
                } else {
                    -1.0
                };
                *best = Near {
                    dist,
                    d: (j as f64 + t) * STEP,
                    side,
                };
            }
        }
    }

    /// Height the ground starts from: the road's own elevation beside the
    /// road, blending smoothly towards every other part of the route further
    /// away, so switchbacks and the two sides of a climb join up as one
    /// hillside instead of stepping where one becomes nearer than the other.
    /// Also how high and low the land may go there, hills and all: no
    /// steeper than `CONE` from any road (`land` holds it between them).
    fn base_height(&self, x: f64, z: f64, near: &Near) -> (f64, f64, f64) {
        let exact = self.route.at(near.d).ele;
        if near.dist <= 9.0 {
            return (exact, exact, exact);
        }
        let n = self.pts.len();
        let step = (n / 800).max(3);
        let (mut sw, mut se) = (0.0, 0.0);
        // The land rises or falls away from any road no steeper than
        // `CONE`: between these bounds (from every road, the nearest
        // exactly), so where all the other roads in reach are far below (a
        // summit), the land falls away from it as a mountainside, not a
        // causeway with cliffs either side.
        const CONE: f64 = 0.6;
        let (mut lo, mut hi) = (
            exact - CONE * (near.dist - 9.0),
            exact + CONE * (near.dist - 9.0),
        );
        for j in (0..n).step_by(step) {
            let (px, pz) = self.pts[j];
            let d = ((px - x).powi(2) + (pz - z).powi(2)).sqrt();
            if d < 1500.0 {
                let reach = CONE * (d - 9.0).max(0.0);
                lo = lo.max(self.eles[j] - reach);
                hi = hi.min(self.eles[j] + reach);
            }
            // Each point of road weighs as one over its distance to the
            // power 2.5: summed along a road, nearly one over the distance to
            // the road, so between two stretches of road (the legs of a
            // hairpin climb) the land rises nearly evenly from one to the
            // other rather than steepest in the middle. Roads further than
            // 1.5 km fade out (only the faintest weight is left, should
            // nothing be nearer); the offset keeps it smooth close by.
            let dd = (d - 9.0).max(0.0) + 12.0;
            let far = 1e-6 / (dd * dd * dd);
            let w = if d >= 1500.0 {
                far
            } else {
                (1.0 - smoothstep(800.0, 1500.0, d)) / (dd * dd * dd.sqrt()) + far
            };
            sw += w;
            se += w * self.eles[j];
        }
        // Where roads are steeper apart than the bounds allow, the blend
        // between them is the best there is.
        let blended = se / sw;
        let bounded = if lo <= hi {
            blended.clamp(lo, hi)
        } else {
            blended
        };
        let h = exact + (bounded - exact) * smoothstep(9.0, 24.0, near.dist);
        if lo <= hi {
            (h, lo, hi)
        } else {
            // Roads steeper apart than the land may be (the legs of a
            // steep hairpin climb): the land between is the slope from one
            // to the other, with no more than bumps on it.
            (h, h - 1.5, h + 1.5)
        }
    }

    /// What the land is like at (x, z): its height, how far the road is,
    /// how thick the forest and whether it is under water; levelled where a
    /// farm building stands.
    pub fn sample(&self, x: f64, z: f64) -> Sample {
        let mut smp = self.land(x, z);
        if smp.water {
            return smp;
        }
        // Level under each building, blending back into the slope around
        // it, never near the road. Where pads meet, the one a point is
        // well inside wins (weights raised to a high power).
        let (mut wmax, mut sum, mut total) = (0.0f64, 0.0, 0.0);
        for &(px, pz, ph, r) in &self.pads {
            let d = ((x - px).powi(2) + (z - pz).powi(2)).sqrt();
            if d < r + 10.0 {
                let w = 1.0 - smoothstep(r, r + 10.0, d);
                let k = w.powi(32);
                wmax = wmax.max(w);
                sum += k * ph;
                total += k;
            }
        }
        if total > 0.0 {
            let w = wmax * smoothstep(4.5, 7.0, smp.dist);
            smp.h += (sum / total - smp.h) * w;
        }
        smp
    }

    /// The land as it lies, before any levelling.
    fn land(&self, x: f64, z: f64) -> Sample {
        let near = self.nearest(x, z);
        let (road, lo, hi) = self.base_height(x, z, &near);
        let s = self.seed;
        let flat = road - 0.25;
        let forest_noise = fbm(x / 650.0 + 31.0, z / 650.0 - 7.0, 4, s + 5);
        let mut forest = smoothstep(-0.05, 0.3, forest_noise) * smoothstep(16.0, 40.0, near.dist);

        if let (Some(level), Scenery::Coast) = (self.water_level, self.route.scenery) {
            if near.side == self.water_side && near.dist > 8.0 {
                // A beach where the road is low, a cliff where it runs high.
                let above = road - level;
                let width = if above < 10.0 {
                    34.0
                } else {
                    5.0 + above * 0.3
                };
                let bed = level - 1.0 - smoothstep(8.0 + width, 260.0, near.dist) * 16.0;
                let mut h = flat + (bed - flat) * smoothstep(8.0, 8.0 + width, near.dist);
                h += fbm(x / 30.0, z / 30.0, 3, s + 3) * 1.2 * smoothstep(8.0, 20.0, near.dist);
                return Sample {
                    h,
                    dist: near.dist,
                    forest: 0.0,
                    water: h < level,
                };
            }
        }
        if let Some(level) = self.water_level {
            if near.side == self.water_side && near.dist > 9.0 {
                // Lake shore falls away from the road into the water.
                let bed = level - 0.6 - smoothstep(14.0, 70.0, near.dist) * 7.0;
                let h = flat + (bed - flat) * smoothstep(9.0, 16.0, near.dist);
                return Sample {
                    h,
                    dist: near.dist,
                    forest: 0.0,
                    water: h < level,
                };
            }
        }

        let t = smoothstep(9.0, 170.0, near.dist);
        let hills = match self.route.scenery {
            Scenery::Alpine => {
                let ridge = 1.0 - fbm(x / 800.0, z / 800.0, 5, s).abs();
                forest *= 1.0
                    - smoothstep(
                        self.max_ele + 40.0,
                        self.max_ele + 140.0,
                        road + 160.0 * ridge * ridge * t,
                    );
                170.0 * ridge * ridge * (0.35 + near.dist / 450.0)
            }
            Scenery::Hills => {
                38.0 * (0.4 + 0.6 * (0.5 + 0.5 * fbm(x / 420.0, z / 420.0, 5, s)))
                    * (0.5 + near.dist / 650.0)
            }
            Scenery::Lake => {
                26.0 * (0.4 + 0.6 * (0.5 + 0.5 * fbm(x / 420.0, z / 420.0, 5, s)))
                    * (0.5 + near.dist / 700.0)
            }
            Scenery::Coast => {
                // Rugged island hills rising inland.
                let ridge = 1.0 - fbm(x / 600.0, z / 600.0, 5, s).abs();
                70.0 * ridge * ridge * (0.3 + near.dist / 500.0)
            }
        };
        let bumps = fbm(x / 55.0, z / 55.0, 3, s + 9) * 1.8;
        // Hills and all, no steeper from any road than the land allows: the
        // ground between the legs of a hairpin climb is the slope between
        // them, not a ridge.
        let h = (flat + t * (hills + bumps)).clamp(lo - 0.25, hi.max(lo - 0.25));
        Sample {
            h,
            dist: near.dist,
            forest,
            water: false,
        }
    }

    /// The distant land around (`cx`, `cz`): the land's own heights on a
    /// coarse grid, with no trees or buildings (too small to see there).
    pub fn build_far(&self, cx: f64, cz: f64) -> FarData {
        let (size, n) = (FAR_SIZE, FAR_VERTS);
        let cell = size / (n - 1) as f64;
        let ox = (cx / cell).round() * cell;
        let oz = (cz / cell).round() * cell;
        let (x0, z0) = (ox - size / 2.0, oz - size / 2.0);
        let m = n + 2;
        let mut samples = Vec::with_capacity(m * m);
        for j in 0..m {
            for i in 0..m {
                samples
                    .push(self.sample(x0 + (i as f64 - 1.0) * cell, z0 + (j as f64 - 1.0) * cell));
            }
        }
        let at = |i: usize, j: usize| &samples[j * m + i];
        let mut verts = Vec::with_capacity(n * n * TERRAIN_STRIDE);
        for j in 0..n {
            for i in 0..n {
                let s = at(i + 1, j + 1);
                let dhdx = (at(i + 2, j + 1).h - at(i, j + 1).h) / (2.0 * cell);
                let dhdz = (at(i + 1, j + 2).h - at(i + 1, j).h) / (2.0 * cell);
                let (nx, ny, nz) = (-dhdx, 1.0, -dhdz);
                let l = (nx * nx + ny * ny + nz * nz).sqrt();
                verts.extend_from_slice(&[
                    (x0 + i as f64 * cell - ox) as f32,
                    s.h as f32,
                    -(z0 + j as f64 * cell - oz) as f32,
                    (nx / l) as f32,
                    (ny / l) as f32,
                    (-nz / l) as f32,
                    s.dist as f32,
                    s.forest as f32,
                ]);
            }
        }
        let mut indices = Vec::with_capacity((n - 1) * (n - 1) * 6);
        for j in 0..n - 1 {
            for i in 0..n - 1 {
                let a = (j * n + i) as u32;
                let (b, c) = (a + 1, a + n as u32);
                indices.extend_from_slice(&[a, c, b, b, c, c + 1]);
            }
        }
        FarData {
            ox,
            oz,
            verts,
            indices,
            route_id: self.route.id.clone(),
        }
    }

    /// Build a square window of terrain centred on (cx, cz).
    pub fn build_window(&self, cx: f64, cz: f64, size: f64, n: usize) -> TerrainData {
        let cell = size / (n - 1) as f64;
        // Snap to the grid so overlapping windows produce identical vertices.
        let ox = (cx / cell).round() * cell;
        let oz = (cz / cell).round() * cell;
        let x0 = ox - size / 2.0;
        let z0 = oz - size / 2.0;
        // One-cell border for normals.
        let m = n + 2;
        let mut samples = Vec::with_capacity(m * m);
        for j in 0..m {
            for i in 0..m {
                samples
                    .push(self.sample(x0 + (i as f64 - 1.0) * cell, z0 + (j as f64 - 1.0) * cell));
            }
        }
        // The road bed: no point of the land's triangles under a road
        // higher than just below its surface, whichever stretch of road it
        // is (so no bank rises over the road, at a hairpin's legs too).
        // Each triangle near a road asks its corners to come down as little
        // as that takes, and each point comes down as far as the most any
        // of its triangles asks: worked out from the land as shaped, so a
        // point comes out the same in every window. Land beside the road
        // keeps its height.
        let mut drop = vec![0.0f64; m * m];
        let near = ROAD_HALF_WIDTH + cell * std::f64::consts::SQRT_2;
        for j in 0..m - 1 {
            for i in 0..m - 1 {
                let (a, b, c, d) = (
                    j * m + i,
                    j * m + i + 1,
                    (j + 1) * m + i,
                    (j + 1) * m + i + 1,
                );
                let (x, z) = (x0 + (i as f64 - 1.0) * cell, z0 + (j as f64 - 1.0) * cell);
                // The cell's two triangles, as drawn: (a, b, c) and (b, d, c),
                // each corner with its offset in the cell.
                for tri in [
                    [(a, 0.0, 0.0), (b, 1.0, 0.0), (c, 0.0, 1.0)],
                    [(b, 1.0, 0.0), (d, 1.0, 1.0), (c, 0.0, 1.0)],
                ] {
                    if tri.iter().all(|&(v, _, _)| samples[v].dist > near) {
                        continue;
                    }
                    let h = tri.map(|(v, _, _)| samples[v].h);
                    let roads = self.road_near(x + cell * 0.5, z + cell * 0.5, cell);
                    if roads.is_empty() {
                        continue;
                    }
                    let lower = self.bed_drops(h, |w| {
                        let (u, v) = (0..3).fold((0.0, 0.0), |(u, v), k| {
                            (u + w[k] * tri[k].1, v + w[k] * tri[k].2)
                        });
                        self.surface_from(&roads, x + u * cell, z + v * cell)
                    });
                    for (k, &(v, _, _)) in tri.iter().enumerate() {
                        drop[v] = drop[v].max(lower[k]);
                    }
                }
            }
        }
        for (s, d) in samples.iter_mut().zip(&drop) {
            s.h -= d;
        }
        let at = |i: usize, j: usize| &samples[j * m + i];
        let mut verts = Vec::with_capacity(n * n * TERRAIN_STRIDE);
        for j in 0..n {
            for i in 0..n {
                let s = at(i + 1, j + 1);
                let dhdx = (at(i + 2, j + 1).h - at(i, j + 1).h) / (2.0 * cell);
                let dhdz = (at(i + 1, j + 2).h - at(i + 1, j).h) / (2.0 * cell);
                let (nx, ny, nz) = (-dhdx, 1.0, -dhdz);
                let l = (nx * nx + ny * ny + nz * nz).sqrt();
                let wx = x0 + i as f64 * cell;
                let wz = z0 + j as f64 * cell;
                // Render space: x east, y up, z = -north (right handed).
                verts.extend_from_slice(&[
                    (wx - ox) as f32,
                    s.h as f32,
                    -(wz - oz) as f32,
                    (nx / l) as f32,
                    (ny / l) as f32,
                    (-nz / l) as f32,
                    s.dist as f32,
                    s.forest as f32,
                ]);
            }
        }
        // Triangles in square tiles, each a run of the index list with its
        // bounding box, so tiles out of view can be skipped when drawing.
        let mut indices = Vec::with_capacity((n - 1) * (n - 1) * 6);
        let mut tiles = Vec::new();
        for tj in (0..n - 1).step_by(TILE_CELLS) {
            for ti in (0..n - 1).step_by(TILE_CELLS) {
                let first = indices.len() as u32;
                let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                for j in tj..(tj + TILE_CELLS).min(n - 1) {
                    for i in ti..(ti + TILE_CELLS).min(n - 1) {
                        let a = (j * n + i) as u32;
                        let b = a + 1;
                        let c = a + n as u32;
                        let d = c + 1;
                        indices.extend_from_slice(&[a, c, b, b, c, d]);
                        for v in [a, d] {
                            let p = &verts[v as usize * TERRAIN_STRIDE..][..3];
                            for k in 0..3 {
                                lo[k] = lo[k].min(p[k]);
                                hi[k] = hi[k].max(p[k]);
                            }
                        }
                    }
                }
                // Room for heights between the corners sampled.
                lo[1] -= 2.0;
                hi[1] += 2.0;
                tiles.push(Tile {
                    first,
                    count: indices.len() as u32 - first,
                    lo,
                    hi,
                });
            }
        }
        // The height of the ground as drawn (triangles between the grid's
        // points), for standing things on it: between the points, on steep
        // ground, it can be a metre or more from the land's own height.
        let ground = |x: f64, z: f64| -> f64 {
            let (u, v) = ((x - x0) / cell, (z - z0) / cell);
            let (i, j) = (
                u.floor().clamp(0.0, (n - 2) as f64),
                v.floor().clamp(0.0, (n - 2) as f64),
            );
            let (fx, fz) = (u - i, v - j);
            let h =
                |di: usize, dj: usize| samples[(j as usize + 1 + dj) * m + i as usize + 1 + di].h;
            let (ha, hb, hc, hd) = (h(0, 0), h(1, 0), h(0, 1), h(1, 1));
            // Each cell is two triangles, split from (i+1, j) to (i, j+1).
            if fx + fz <= 1.0 {
                ha + (hb - ha) * fx + (hc - ha) * fz
            } else {
                hd + (hc - hd) * (1.0 - fx) + (hb - hd) * (1.0 - fz)
            }
        };
        let (instances, shadows) = self.scatter(ox, oz, x0, z0, size, &ground);
        TerrainData {
            tiles,
            ox,
            oz,
            size,
            verts,
            indices,
            instances,
            shadows,
            route_id: self.route.id.clone(),
            water_level: self.water_level,
            snow_level: self.snow_level(),
            sea: self.route.scenery == Scenery::Coast,
        }
    }

    /// Trees, rocks and houses inside the window, plus blob shadows.
    /// Farmsteads along the road, inside the square from (x0, z0), `size`
    /// across: every 320 m or so, a house facing the road, often a barn
    /// behind it and sometimes a shed beside it.
    fn farmsteads(&self, x0: f64, z0: f64, size: f64) -> Vec<Building> {
        let mut out = Vec::new();
        let s = self.seed;
        let route = &self.route;
        let spacing = 320.0;
        let house_roofs = [[0.55, 0.22, 0.15], [0.35, 0.33, 0.32], [0.45, 0.28, 0.18]];
        let barn_roofs = [[0.4, 0.4, 0.4], [0.45, 0.2, 0.12], [0.28, 0.27, 0.26]];
        for k in 0..(route.span() / spacing) as i64 {
            let mut rng = Rng(hash2(k, 0, s + 991));
            if rng.next() > 0.55 {
                continue;
            }
            let d = k as f64 * spacing + rng.next() * 100.0;
            let p = route.at(d);
            let side = if rng.next() < 0.5 { -1.0 } else { 1.0 };
            let lateral = side * (24.0 + 26.0 * rng.next());
            // Along the road, away from it, and the turn that faces a
            // building's front (+z) towards it (heading is from north).
            let along = (p.heading.sin(), p.heading.cos());
            let away = (side * p.heading.cos(), -side * p.heading.sin());
            let facing = (std::f64::consts::FRAC_PI_2 - p.heading
                + if side > 0.0 {
                    std::f64::consts::PI
                } else {
                    0.0
                }) as f32;
            let at = |a: f64, w: f64| {
                (
                    p.x + away.0 * (lateral.abs() + w) + along.0 * a,
                    p.z + away.1 * (lateral.abs() + w) + along.1 * a,
                )
            };
            let two_storey = rng.next() < 0.3;
            let roof = house_roofs[(rng.next() * 3.0) as usize % 3];
            let scale = (0.9 + 0.2 * rng.next()) as f32;
            let (kind, reach) = if two_storey {
                (TOWNHOUSE, 7.5)
            } else {
                (HOUSE, 10.5)
            };
            let mut place = |kind, (x, z): (f64, f64), rot, scale: f32, roof, reach: f64| {
                if x < x0 || x > x0 + size || z < z0 || z > z0 + size {
                    return;
                }
                let smp = self.sample(x, z);
                if smp.water || smp.dist < 15.0 {
                    return;
                }
                // It stands at the top of its ground, its footing showing
                // on the downhill side; not on ground too steep for that.
                let reach = reach * scale as f64;
                let (mut lo, mut hi) = (smp.h, smp.h);
                for k in 0..8 {
                    let a = k as f64 * std::f64::consts::FRAC_PI_4;
                    let g = self.sample(x + a.cos() * reach * 0.75, z + a.sin() * reach * 0.75);
                    if g.water {
                        return;
                    }
                    lo = lo.min(g.h);
                    hi = hi.max(g.h);
                }
                if hi - lo > 3.0 {
                    return;
                }
                out.push(Building {
                    kind,
                    farm: k,
                    x,
                    z,
                    h: hi - 0.1,
                    rot,
                    scale: (scale, scale),
                    tint: roof,
                    reach,
                });
            };
            place(kind, at(0.0, 0.0), facing, scale, roof, reach);
            // A barn behind the house, along the road or end on to it.
            if rng.next() < 0.6 {
                let end_on = rng.next() < 0.4;
                let behind = if end_on { 23.0 } else { 18.0 };
                let shift = (rng.next() - 0.5) * 16.0;
                let turn = if end_on {
                    std::f32::consts::FRAC_PI_2
                } else {
                    0.0
                };
                let roof = barn_roofs[(rng.next() * 3.0) as usize % 3];
                place(
                    BARN,
                    at(shift, behind),
                    facing + turn,
                    (0.85 + 0.25 * rng.next()) as f32,
                    roof,
                    11.0,
                );
            }
            // A shed beside the house, at the end without the woodshed.
            if rng.next() < 0.5 {
                let gap = if two_storey { 11.0 } else { 13.0 };
                place(
                    SHED,
                    at(-side * gap, 1.5),
                    facing,
                    (0.9 + 0.2 * rng.next()) as f32,
                    roof,
                    3.5,
                );
            }
            // A fruit tree or two in the front garden.
            let fruit = if rng.next() < 0.75 {
                1 + usize::from(rng.next() < 0.45)
            } else {
                0
            };
            for _ in 0..fruit {
                // Inside the garden fence, on the side away from the table
                // (the farmhouse's is at its east end, the two-storey
                // house's at its west; a building's east, +x, lies at
                // `-side` along the road).
                let east = if two_storey { 1.0 } else { -1.0 };
                let depth = if two_storey { 4.0 } else { 4.2 };
                let (x, z) = at(
                    -side * east * (2.5 + 3.5 * rng.next()),
                    -(depth + 1.4 + 3.0 * rng.next()),
                );
                let height = (5.0 + 2.5 * rng.next()) as f32;
                let rot = (rng.next() * std::f64::consts::TAU) as f32;
                if x < x0 || x > x0 + size || z < z0 || z > z0 + size {
                    continue;
                }
                let smp = self.sample(x, z);
                if smp.water || smp.dist < 11.0 {
                    continue;
                }
                out.push(Building {
                    kind: BROADLEAF,
                    farm: k,
                    x,
                    z,
                    h: smp.h - 0.2,
                    rot,
                    scale: (height * 0.95, height),
                    tint: [1.0, 1.04, 0.9],
                    reach: 3.0,
                });
            }
        }
        out
    }

    /// A bale (or two end to end) in the cell `gi`, `gj` at `x`, `z`, if
    /// it lies inside a field of the farmland patchwork (`Field`) that is
    /// mown: a wheat field (straw) or a pasture (hay, often wrapped), near
    /// a farm, not at its edge, on level open ground. Some such fields have
    /// bales, lying in rows along the field's own direction.
    #[allow(clippy::too_many_arguments)]
    fn bale(
        &self,
        palette: &Palette,
        gi: i64,
        gj: i64,
        x: f64,
        z: f64,
        spot: &Spot,
        ground: &dyn Fn(f64, f64) -> f64,
    ) -> Option<Bales> {
        if palette.bales <= 0.0 || spot.forest > 0.05 || spot.dist < 40.0 {
            return None;
        }
        let field = Field::at(x, z);
        let (straw, hay) = (field.crop < 0.2, (0.58..0.74).contains(&field.crop));
        if !(straw || hay) || field.edge < 8.0 {
            return None;
        }
        let mut each = Rng(hash2(field.cell.0, field.cell.1, self.seed + 4111));
        if each.next() >= palette.bales {
            return None;
        }
        let mut cell = Rng(hash2(gi, gj, self.seed + 4127));
        if cell.next() >= 0.3 {
            return None;
        }
        // Farmland lies around the farms.
        if !self
            .farms
            .iter()
            .any(|b| (x - b.x).powi(2) + (z - b.z).powi(2) < 500.0f64.powi(2))
        {
            return None;
        }
        let tint = if straw {
            [0.55, 0.45, 0.24]
        } else {
            match each.next() {
                r if r < 0.35 => [0.5, 0.43, 0.22],
                r if r < 0.7 => [0.78, 0.8, 0.77],
                r if r < 0.88 => [0.2, 0.3, 0.17],
                _ => [0.05, 0.05, 0.055],
            }
        };
        let shade = (0.9 + 0.2 * cell.next()) as f32;
        let rot = field.rows + (0.1 * (cell.next() - 0.5)) as f32;
        let mut at = vec![(0.0, 0.0)];
        if cell.next() < 0.3 {
            // A second one end to end along its axis (x, turned by `rot`).
            // (World z is north; the turn is about render space's -z.)
            at.push((1.28 * rot.cos() as f64, 1.28 * rot.sin() as f64));
        }
        // On level, dry ground, out of the farmyards.
        let clear = at.iter().all(|&(dx, dz)| {
            let (bx, bz) = (x + dx, z + dz);
            let e = 1.5;
            let slope = (ground(bx + e, bz) - ground(bx - e, bz))
                .hypot(ground(bx, bz + e) - ground(bx, bz - e))
                / (2.0 * e);
            slope < 0.2
                && self.room_to_stand(bx, bz, 1.5, ground)
                && self
                    .farms
                    .iter()
                    .all(|b| (bx - b.x).powi(2) + (bz - b.z).powi(2) > (b.reach + 6.0).powi(2))
        });
        clear.then_some(Bales {
            at,
            rot,
            tint: tint.map(|c| c * shade),
        })
    }

    /// Whether `x`, `z` lies inside a cropped field or pasture of the
    /// farmland patchwork the ground is painted with, away from its edge:
    /// no tree, bush or rock stands there (trees grow along the edges,
    /// as hedgerows).
    fn in_field(&self, x: f64, z: f64, smp: &Sample, ground: &dyn Fn(f64, f64) -> f64) -> bool {
        if !matches!(self.route.scenery, Scenery::Hills | Scenery::Lake)
            || smp.forest > 0.2
            || smp.dist < 36.0
        {
            return false;
        }
        let e = 2.0;
        let slope = (ground(x + e, z) - ground(x - e, z))
            .hypot(ground(x, z + e) - ground(x, z - e))
            / (2.0 * e);
        if slope > 0.33 {
            return false;
        }
        let field = Field::at(x, z);
        field.crop < 0.74 && field.edge > 5.0
    }

    /// Dry-stone walls along the fields beside the road, piece by piece,
    /// standing on the ground as drawn: on stretches of 140 m where no wooden
    /// fence stands (see the fences in `gfx`), on the land side, broken by
    /// a gateway now and then and wherever a farmyard, water, thick forest
    /// or another stretch of road is in the way.
    fn walls(
        &self,
        x0: f64,
        z0: f64,
        size: f64,
        rel: &dyn Fn(f64, f64) -> (f32, f32),
        ground: &dyn Fn(f64, f64) -> f64,
        out: &mut Vec<f32>,
    ) {
        let route = &self.route;
        let sides: &[f64] = match route.scenery {
            Scenery::Hills | Scenery::Alpine => &[-1.0, 1.0],
            Scenery::Lake | Scenery::Coast => {
                if self.water_side > 0.0 {
                    &[-1.0]
                } else {
                    &[1.0]
                }
            }
        };
        const STRETCH: f64 = 140.0;
        const LATERAL: f64 = 8.8;
        let piece = fields::WALL_PIECE as f64;
        let point = |s: f64, side: f64| {
            let p = route.at(s);
            (
                p.x + side * LATERAL * p.heading.cos(),
                p.z - side * LATERAL * p.heading.sin(),
            )
        };
        let inside = |x: f64, z: f64, pad: f64| {
            x > x0 - pad && x < x0 + size + pad && z > z0 - pad && z < z0 + size + pad
        };
        let stretches = (route.span() / STRETCH).ceil() as i64;
        for &side in sides {
            for k in 0..stretches {
                let pick = super::terrain_hash(k, side as i64, route.seed) % 5;
                let walled = match route.scenery {
                    Scenery::Coast => pick < 2,
                    _ => pick == 2,
                };
                let (s0, s1) = (
                    k as f64 * STRETCH,
                    ((k + 1) as f64 * STRETCH).min(route.span()),
                );
                if !walled
                    || !inside(point(s0, side).0, point(s0, side).1, STRETCH)
                        && !inside(point(s1, side).0, point(s1, side).1, STRETCH)
                {
                    continue;
                }
                // A gateway somewhere along half the stretches.
                let gate =
                    (super::terrain_hash(k, 31 + side as i64, route.seed) % 1000) as f64 / 1000.0;
                let gate_at = s0 + 20.0 + (STRETCH - 40.0) * gate;
                let has_gate = gate < 0.5;
                let mut s = s0;
                let mut a = point(s, side);
                let mut n = 0u32;
                while s < s1 {
                    // The point along the road whose wall point is a piece's
                    // length from the last: a few secant steps.
                    let mut t = s + piece;
                    for _ in 0..4 {
                        let b = point(t, side);
                        let d = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
                        if d < 1e-6 {
                            break;
                        }
                        t = s + (t - s) * piece / d;
                    }
                    if t > s1 + 0.5 {
                        break;
                    }
                    let b = point(t, side);
                    let (cx, cz) = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
                    let gap = has_gate && (s - gate_at).abs() < 2.6;
                    // Not where the ground falls so steeply along it that the
                    // pieces would stand in steps, nor with an end in water.
                    let steady = (ground(a.0, a.1) - ground(b.0, b.1)).abs() < 0.8
                        && !self.sample(a.0, a.1).water
                        && !self.sample(b.0, b.1).water;
                    if !gap && steady && inside(cx, cz, 0.0) && self.wall_fits(cx, cz) {
                        let (rx, rz) = rel(cx, cz);
                        let rot = (b.1 - a.1).atan2(b.0 - a.0) as f32;
                        // Every other piece turned end for end, and each a
                        // shade of its own, so the pieces do not repeat.
                        let flip = if n % 2 == 1 {
                            std::f32::consts::PI
                        } else {
                            0.0
                        };
                        let tone =
                            0.9 + 0.2 * (hash2(k, n as i64, route.seed) % 100) as f32 / 100.0;
                        out.extend_from_slice(&[
                            rx,
                            lowest(ground, cx, cz, 0.4) as f32 - 0.03,
                            rz,
                            rot + flip,
                            1.0,
                            1.0,
                            tone,
                            tone * 0.99,
                            tone * 0.96,
                        ]);
                    }
                    s = t;
                    a = b;
                    n += 1;
                }
            }
        }
    }

    /// Whether a piece of wall may stand at `x`, `z`: on dry land, out of
    /// thick forest, clear of the road (where it bends back on itself) and
    /// of the farmyards.
    fn wall_fits(&self, x: f64, z: f64) -> bool {
        let smp = self.sample(x, z);
        !smp.water
            && smp.forest < 0.45
            && smp.dist > 7.5
            && self
                .farms
                .iter()
                .all(|b| (x - b.x).powi(2) + (z - b.z).powi(2) > (b.reach + 3.0).powi(2))
    }

    /// Whether something may stand at `x`, `z` on the ground as drawn: not
    /// on a slope so steep that it would stand out of it, and dry, well
    /// above the water, for `reach` around (so no crown hangs over it).
    fn room_to_stand(&self, x: f64, z: f64, reach: f64, ground: &dyn Fn(f64, f64) -> f64) -> bool {
        let e = 2.0;
        let slope = (ground(x + e, z) - ground(x - e, z))
            .hypot(ground(x, z + e) - ground(x, z - e))
            / (2.0 * e);
        if slope > 0.9 {
            return false;
        }
        if let Some(level) = self.water_level {
            if ground(x, z) < level + 0.6 {
                return false;
            }
            for k in 0..6 {
                let a = k as f64 * std::f64::consts::TAU / 6.0;
                let (px, pz) = (x + a.cos() * reach, z + a.sin() * reach);
                if ground(px, pz) < level + 0.2 {
                    return false;
                }
            }
        }
        true
    }

    fn scatter(
        &self,
        ox: f64,
        oz: f64,
        x0: f64,
        z0: f64,
        size: f64,
        ground: &dyn Fn(f64, f64) -> f64,
    ) -> ([Vec<f32>; KINDS], Vec<f32>) {
        let mut out: [Vec<f32>; KINDS] = Default::default();
        let mut shadows = Vec::new();
        let s = self.seed;
        let rel = |x: f64, z: f64| ((x - ox) as f32, (-(z - oz)) as f32);
        let cell = 11.0;
        let (gi0, gj0) = ((x0 / cell).floor() as i64, (z0 / cell).floor() as i64);
        let cells = (size / cell).ceil() as i64;
        let palette = Palette::of(self.route.scenery);
        let farms: Vec<&Building> = self
            .farms
            .iter()
            .filter(|b| b.x >= x0 && b.x <= x0 + size && b.z >= z0 && b.z <= z0 + size)
            .collect();
        for gj in gj0..gj0 + cells {
            for gi in gi0..gi0 + cells {
                let mut rng = Rng(hash2(gi, gj, s + 77));
                let x = (gi as f64 + rng.next()) * cell;
                let z = (gj as f64 + rng.next()) * cell;
                let smp = self.sample(x, z);
                let gh = ground(x, z) as f32;
                if smp.water || smp.dist < 9.5 || !self.room_to_stand(x, z, 3.0, ground) {
                    continue;
                }
                // Nothing grows where a building stands.
                if farms
                    .iter()
                    .any(|b| (x - b.x).powi(2) + (z - b.z).powi(2) < (b.reach + 2.0).powi(2))
                {
                    continue;
                }
                let spot = Spot {
                    forest: smp.forest,
                    dist: smp.dist,
                };
                // Bales lie in mown fields, open and away from the road; with
                // numbers of their own, so the trees stay where they were.
                if let Some(b) = self.bale(&palette, gi, gj, x, z, &spot, ground) {
                    let (rx, rz) = rel(x, z);
                    for (dx, dz) in b.at {
                        let (bx, bz) = (x + dx, z + dz);
                        let (px, pz) = rel(bx, bz);
                        let h = lowest(ground, bx, bz, BALE_FOOT) as f32;
                        out[BALE].extend_from_slice(&[
                            px,
                            h - 0.06,
                            pz,
                            b.rot,
                            1.0,
                            1.0,
                            b.tint[0],
                            b.tint[1],
                            b.tint[2],
                        ]);
                    }
                    shadows.extend_from_slice(&[rx, gh + 0.08, rz, 1.1]);
                    continue;
                }
                // Fields are kept clear.
                if self.in_field(x, z, &smp, ground) {
                    continue;
                }
                let r = rng.next();
                let (rx, rz) = rel(x, z);
                match palette.cover(&spot, r) {
                    Some(Cover::Tree) => {
                        let t = palette.tree(&spot, || rng.next());
                        let tall = 0.7 + 0.6 * rng.next() + smp.forest * 0.3;
                        let height = t.height * tall;
                        let width = height * t.width * (0.85 + 0.3 * rng.next());
                        // Each tree has its own shade of green, some a little yellower.
                        let tint = 0.78 + 0.44 * rng.next() as f32;
                        let warm = (rng.next() as f32).powi(2) * t.warm;
                        let rot = rng.next() * std::f64::consts::TAU;
                        let foot = lowest(ground, x, z, TREE_FOOT * width) as f32;
                        out[t.kind].extend_from_slice(&[
                            rx,
                            foot - 0.2,
                            rz,
                            rot as f32,
                            width as f32,
                            height as f32,
                            tint * (0.95 + 0.4 * warm),
                            tint * (1.0 + 0.08 * warm),
                            tint * (0.9 - 0.3 * warm),
                        ]);
                        shadows.extend_from_slice(&[rx, gh + 0.08, rz, width as f32 * 0.5]);
                    }
                    Some(Cover::Bush) => {
                        let height = 1.2 + 1.8 * rng.next();
                        let width = height * (1.2 + 0.8 * rng.next());
                        let tint = 0.8 + 0.4 * rng.next() as f32;
                        let rot = rng.next() * std::f64::consts::TAU;
                        let foot = lowest(ground, x, z, BUSH_FOOT * width) as f32;
                        out[BUSH].extend_from_slice(&[
                            rx,
                            foot - 0.15,
                            rz,
                            rot as f32,
                            width as f32,
                            height as f32,
                            tint,
                            tint,
                            tint * 0.9,
                        ]);
                        shadows.extend_from_slice(&[rx, gh + 0.08, rz, width as f32 * 0.55]);
                    }
                    Some(Cover::Rock) => {
                        let size = 1.0 + 3.5 * rng.next().powi(2);
                        let g = 0.8 + 0.3 * rng.next() as f32;
                        let rot = rng.next() * std::f64::consts::TAU;
                        let foot = lowest(ground, x, z, ROCK_FOOT * size) as f32;
                        out[ROCK].extend_from_slice(&[
                            rx,
                            foot - size as f32 * 0.25,
                            rz,
                            rot as f32,
                            size as f32,
                            size as f32 * 0.7,
                            g,
                            g,
                            g,
                        ]);
                    }
                    None => {}
                }
            }
        }
        self.walls(x0, z0, size, &rel, ground, &mut out[WALL]);
        for b in farms {
            let (rx, rz) = rel(b.x, b.z);
            let gh = ground(b.x, b.z) as f32;
            // Garden trees stand on the ground as drawn, like the rest.
            let h = if [BROADLEAF, OAK, MAPLE, PINE, BIRCH].contains(&b.kind) {
                lowest(ground, b.x, b.z, TREE_FOOT * b.scale.0 as f64) as f32 - 0.2
            } else {
                b.h as f32
            };
            out[b.kind].extend_from_slice(&[
                rx, h, rz, b.rot, b.scale.0, b.scale.1, b.tint[0], b.tint[1], b.tint[2],
            ]);
            shadows.extend_from_slice(&[rx, gh + 0.08, rz, b.reach as f32 * 0.8]);
        }
        (out, shadows)
    }
}

impl NearGrid {
    fn build(pts: &[(f64, f64)]) -> NearGrid {
        // Far enough out for the distant land (`FAR_SIZE`).
        let margin = 4500.0;
        let (mut xmin, mut xmax, mut zmin, mut zmax) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for &(x, z) in pts {
            xmin = xmin.min(x);
            xmax = xmax.max(x);
            zmin = zmin.min(z);
            zmax = zmax.max(z);
        }
        let (x0, z0) = (xmin - margin, zmin - margin);
        let (w, h) = (xmax - xmin + 2.0 * margin, zmax - zmin + 2.0 * margin);
        let cell = (w * h / 1.5e6).sqrt().max(15.0);
        let nx = (w / cell).ceil() as usize + 1;
        let nz = (h / cell).ceil() as usize + 1;
        let mut idx = vec![u32::MAX; nx * nz];
        let mut dist = vec![f64::MAX; nx * nz];
        let centre =
            |i: usize, j: usize| (x0 + (i as f64 + 0.5) * cell, z0 + (j as f64 + 0.5) * cell);
        let d2 = |c: (f64, f64), p: (f64, f64)| (c.0 - p.0).powi(2) + (c.1 - p.1).powi(2);
        for (k, &p) in pts.iter().enumerate() {
            let i = (((p.0 - x0) / cell) as usize).min(nx - 1);
            let j = (((p.1 - z0) / cell) as usize).min(nz - 1);
            let dd = d2(centre(i, j), p);
            if dd < dist[j * nx + i] {
                dist[j * nx + i] = dd;
                idx[j * nx + i] = k as u32;
            }
        }
        // Two-pass propagation of the nearest point index (vector distance transform).
        let relax =
            |i: usize, j: usize, di: i64, dj: i64, idx: &mut Vec<u32>, dist: &mut Vec<f64>| {
                let (ni, nj) = (i as i64 + di, j as i64 + dj);
                if ni < 0 || nj < 0 || ni >= nx as i64 || nj >= nz as i64 {
                    return;
                }
                let cand = idx[nj as usize * nx + ni as usize];
                if cand == u32::MAX {
                    return;
                }
                let dd = d2(centre(i, j), pts[cand as usize]);
                if dd < dist[j * nx + i] {
                    dist[j * nx + i] = dd;
                    idx[j * nx + i] = cand;
                }
            };
        for j in 0..nz {
            for i in 0..nx {
                for (di, dj) in [(-1, -1), (0, -1), (1, -1), (-1, 0)] {
                    relax(i, j, di, dj, &mut idx, &mut dist);
                }
            }
        }
        for j in (0..nz).rev() {
            for i in (0..nx).rev() {
                for (di, dj) in [(1, 0), (-1, 1), (0, 1), (1, 1)] {
                    relax(i, j, di, dj, &mut idx, &mut dist);
                }
            }
        }
        NearGrid {
            x0,
            z0,
            cell,
            nx,
            nz,
            idx,
        }
    }

    /// Candidate nearest point indices: those stored in the cell and its
    /// neighbours (a cell stores the point nearest its centre, which can be
    /// on a different part of the route than the one nearest `x, z`).
    fn lookup(&self, x: f64, z: f64) -> Vec<u32> {
        let i = (((x - self.x0) / self.cell).max(0.0) as usize).min(self.nx - 1);
        let j = (((z - self.z0) / self.cell).max(0.0) as usize).min(self.nz - 1);
        let mut out: Vec<u32> = Vec::with_capacity(3);
        for nj in j.saturating_sub(1)..=(j + 1).min(self.nz - 1) {
            for ni in i.saturating_sub(1)..=(i + 1).min(self.nx - 1) {
                let v = self.idx[nj * self.nx + ni];
                if v != u32::MAX && !out.iter().any(|&o| o.abs_diff(v) < 30) {
                    out.push(v);
                }
            }
        }
        if out.is_empty() {
            out.push(0);
        }
        out
    }
}

/// The distant land: a coarse sheet of the same land around a centre, out
/// to the mountains on the horizon, drawn where the detailed window does
/// not reach. Vertices as the window's (`TERRAIN_STRIDE`), from its own
/// centre.
pub struct FarData {
    pub ox: f64,
    pub oz: f64,
    pub verts: Vec<f32>,
    pub indices: Vec<u32>,
    pub route_id: String,
}

/// What the terrain worker sends back.
pub enum Built {
    Window(Box<TerrainData>),
    Far(FarData),
}

/// Across the distant land (metres), and its vertices across: about 60 m
/// apart. Inside the nearest mountain ring (3.9 km away).
pub const FAR_SIZE: f64 = 7600.0;
pub const FAR_VERTS: usize = 129;
/// The distant land is built again once the camera is this far from its
/// centre.
pub const FAR_MOVE: f64 = 1200.0;

pub struct TerrainData {
    /// Square tiles of the index list, to skip those out of view.
    pub tiles: Vec<Tile>,
    /// World position of the window centre (the render origin).
    pub ox: f64,
    pub oz: f64,
    pub size: f64,
    pub verts: Vec<f32>,
    pub indices: Vec<u32>,
    /// Instances per kind (see [`PINE`] and the other kind constants).
    pub instances: [Vec<f32>; KINDS],
    /// Blob shadows: position (3), radius.
    pub shadows: Vec<f32>,
    pub route_id: String,
    pub water_level: Option<f64>,
    pub snow_level: f64,
    /// Water reaching to the horizon (the sea).
    pub sea: bool,
}

impl TerrainData {
    /// The height of the ground as drawn at world (`x`, `z`), if the
    /// window covers it: the triangles between its points.
    pub fn ground(&self, x: f64, z: f64) -> Option<f64> {
        let n = ((self.verts.len() / TERRAIN_STRIDE) as f64).sqrt().round() as usize;
        if n < 2 {
            return None;
        }
        let cell = self.size / (n - 1) as f64;
        let half = self.size / 2.0;
        let (u, v) = ((x - self.ox + half) / cell, (z - self.oz + half) / cell);
        if u < 0.0 || v < 0.0 || u >= (n - 1) as f64 || v >= (n - 1) as f64 {
            return None;
        }
        let (i, j) = (u.floor() as usize, v.floor() as usize);
        let h = |i: usize, j: usize| self.verts[(j * n + i) * TERRAIN_STRIDE + 1] as f64;
        let (fx, fz) = (u - i as f64, v - j as f64);
        Some(if fx + fz <= 1.0 {
            h(i, j) + (h(i + 1, j) - h(i, j)) * fx + (h(i, j + 1) - h(i, j)) * fz
        } else {
            h(i + 1, j + 1)
                + (h(i, j + 1) - h(i + 1, j + 1)) * (1.0 - fx)
                + (h(i + 1, j) - h(i + 1, j + 1)) * (1.0 - fz)
        })
    }
}

struct Request {
    route: Arc<Route>,
    cx: f64,
    cz: f64,
}

/// Builds terrain windows off the UI thread.
/// Half the road's width to the outside of its verges (the road ribbon's
/// outer edge in `gfx`).
pub const ROAD_HALF_WIDTH: f64 = 5.8;
/// How far below the road's surface the land under it lies, at least.
pub const ROAD_BED: f64 = 0.02;
/// Half the width of the road's middle, between its verges.
pub const ROAD_CROWN: f64 = 4.6;
/// How far the verges fall from the middle to the road's outer edges.
pub const ROAD_VERGE_DROP: f64 = 0.25;
/// The size of the squares road points are sorted into.
const SEG_CELL: f64 = 16.0;

pub struct Worker {
    tx: Sender<Request>,
    pub rx: Receiver<Built>,
}

pub const WINDOW_SIZE: f64 = 2000.0;
pub const WINDOW_VERTS: usize = 257;

impl Worker {
    pub fn spawn() -> Worker {
        let (tx, req_rx) = channel::<Request>();
        let (done_tx, rx) = channel();
        std::thread::Builder::new()
            .name("terrain".into())
            .spawn(move || {
                let mut geo: Option<Arc<RouteGeo>> = None;
                let mut far_at: Option<(String, f64, f64)> = None;
                while let Ok(mut req) = req_rx.recv() {
                    // Only the newest request matters.
                    while let Ok(r) = req_rx.try_recv() {
                        req = r;
                    }
                    let g = match &geo {
                        Some(g)
                            if g.route.id == req.route.id && g.route.length == req.route.length =>
                        {
                            g.clone()
                        }
                        _ => {
                            let g = Arc::new(RouteGeo::new(&req.route));
                            geo = Some(g.clone());
                            g
                        }
                    };
                    let data = g.build_window(req.cx, req.cz, WINDOW_SIZE, WINDOW_VERTS);
                    if done_tx.send(Built::Window(Box::new(data))).is_err() {
                        break;
                    }
                    // The distant land, when the camera has moved far from
                    // where it was last built (or the route changed).
                    let stale = far_at
                        .as_ref()
                        .is_none_or(|(id, x, z): &(String, f64, f64)| {
                            *id != req.route.id || (req.cx - x).hypot(req.cz - z) > FAR_MOVE
                        });
                    if stale {
                        let far = g.build_far(req.cx, req.cz);
                        far_at = Some((req.route.id.clone(), far.ox, far.oz));
                        if done_tx.send(Built::Far(far)).is_err() {
                            break;
                        }
                    }
                }
            })
            .expect("spawn terrain worker");
        Worker { tx, rx }
    }

    pub fn request(&self, route: Arc<Route>, cx: f64, cz: f64) {
        let _ = self.tx.send(Request { route, cx, cz });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route;

    #[test]
    fn distant_land_is_quick_to_build_and_meets_the_window() {
        for r in [&route::builtin()[0], &route::builtin()[4]] {
            let g = RouteGeo::new(r);
            let p = r.at(1000.0);
            let t = std::time::Instant::now();
            let far = g.build_far(p.x, p.z);
            let ms = t.elapsed().as_millis();
            println!("{}: distant land in {ms} ms", r.id);
            assert_eq!(far.verts.len(), FAR_VERTS * FAR_VERTS * TERRAIN_STRIDE);
            assert!(far.verts.iter().all(|v| v.is_finite()));
            // Its heights are the land's own, so it meets the window.
            let w = g.build_window(p.x, p.z, WINDOW_SIZE, WINDOW_VERTS);
            let near = drawn(&w);
            let cell = FAR_SIZE / (FAR_VERTS - 1) as f64;
            for k in 0..FAR_VERTS {
                let v = &far.verts[(k * FAR_VERTS + k) * TERRAIN_STRIDE..];
                let (x, z) = (v[0] as f64 + far.ox, far.oz - v[2] as f64);
                if (x - w.ox).abs() < 900.0 && (z - w.oz).abs() < 900.0 {
                    assert!(
                        (v[1] as f64 - near(x, z)).abs() < 0.5,
                        "{} at {k}",
                        v[1] as f64 - near(x, z)
                    );
                }
            }
            let _ = cell;
        }
    }

    #[test]
    fn the_worker_sends_the_window_then_the_distant_land() {
        let r = Arc::new(route::builtin()[0].clone());
        let w = Worker::spawn();
        let p = r.at(100.0);
        w.request(r.clone(), p.x, p.z);
        let (mut window, mut far) = (false, false);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while std::time::Instant::now() < until && !(window && far) {
            match w.rx.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(Built::Window(_)) => window = true,
                Ok(Built::Far(_)) => far = true,
                Err(_) => {}
            }
        }
        assert!(window && far, "window {window}, distant land {far}");
    }

    #[test]
    fn nearest_matches_route_geometry() {
        let r = &route::builtin()[0];
        let geo = RouteGeo::new(r);
        for d in [0.0, 1234.0, 5000.0, 9000.0] {
            let p = r.at(d);
            // 30 m to the right of the road.
            let (x, z) = (p.x + 30.0 * p.heading.cos(), p.z - 30.0 * p.heading.sin());
            let n = geo.nearest(x, z);
            assert!((n.dist - 30.0).abs() < 1.5, "dist {} at {d}", n.dist);
            assert_eq!(n.side, 1.0);
            let on_road = geo.sample(p.x, p.z);
            assert!((on_road.h - (p.ele - 0.25)).abs() < 0.05);
        }
    }

    #[test]
    fn farm_buildings_face_the_road_and_keep_off_it() {
        for r in route::builtin() {
            let g = RouteGeo::new(&r);
            let farms = &g.farms;
            if r.scenery == Scenery::Alpine {
                continue;
            }
            assert!(farms.len() >= 3, "{}: {} buildings", r.id, farms.len());
            assert!(farms.iter().any(|b| b.kind == BARN), "{}", r.id);
            // The buildings (not the trees in their gardens).
            for b in farms
                .iter()
                .filter(|b| [HOUSE, TOWNHOUSE, BARN, SHED].contains(&b.kind))
            {
                let smp = g.sample(b.x, b.z);
                assert!(
                    smp.dist >= 15.0 && !smp.water,
                    "{}: building at {:.0} m from the road",
                    r.id,
                    smp.dist
                );
                // It stands on level ground: no floating door or flowers.
                let (mut lo, mut hi) = (smp.h, smp.h);
                for k in 0..12 {
                    let a = k as f64 * std::f64::consts::TAU / 12.0;
                    let g = g.sample(b.x + a.cos() * b.reach * 0.7, b.z + a.sin() * b.reach * 0.7);
                    lo = lo.min(g.h);
                    hi = hi.max(g.h);
                }
                assert!(
                    hi - lo < 0.3,
                    "{}: ground under a building varies {:.2} m",
                    r.id,
                    hi - lo
                );
                assert!(
                    (b.h - lo).abs() < 0.3,
                    "{}: building {:.2} m off the ground",
                    r.id,
                    b.h - lo
                );
                // Houses and sheds turn their front (+z) to the road:
                // towards where it is nearer, a few metres ahead.
                if b.kind == BARN {
                    continue;
                }
                let (c, s) = (b.rot.cos() as f64, b.rot.sin() as f64);
                // The front in world (east, north), as the object shader
                // turns it (render z is minus north).
                let front = (s, -c);
                let ahead = g.sample(b.x + front.0 * 5.0, b.z + front.1 * 5.0);
                assert!(
                    ahead.dist < smp.dist - 3.0,
                    "{}: a building turns its back to the road",
                    r.id
                );
            }
        }
    }

    #[test]
    fn things_stand_on_the_ground_as_drawn() {
        // On the lake route, whose banks are steep: everything stands on the
        // drawn triangles, its base set into the ground all round, so no
        // side of it floats over a slope, and no deeper than it needs.
        let r = crate::route::builtin()
            .into_iter()
            .find(|r| r.id == "lakeside")
            .unwrap();
        let g = RouteGeo::new(&r);
        let p = r.at(2500.0);
        let w = g.build_window(p.x, p.z, WINDOW_SIZE, WINDOW_VERTS);
        let ground = drawn(&w);
        let mut checked = 0;
        let mut on_slopes = 0;
        // How far below the ground each kind is set, and its base's radius.
        type Foot = fn(&[f32]) -> f64;
        let kinds: [(usize, f64, Foot); 9] = [
            (PINE, 0.2, |i| TREE_FOOT * i[4] as f64),
            (BROADLEAF, 0.2, |i| TREE_FOOT * i[4] as f64),
            (BIRCH, 0.2, |i| TREE_FOOT * i[4] as f64),
            (OAK, 0.2, |i| TREE_FOOT * i[4] as f64),
            (MAPLE, 0.2, |i| TREE_FOOT * i[4] as f64),
            (BUSH, 0.15, |i| BUSH_FOOT * i[4] as f64),
            (ROCK, 0.0, |i| ROCK_FOOT * i[4] as f64),
            (BALE, 0.06, |_| BALE_FOOT),
            (WALL, 0.03, |_| 0.4),
        ];
        for (kind, sunk, foot) in kinds {
            for inst in w.instances[kind].chunks(INSTANCE_STRIDE) {
                let (x, z) = (inst[0] as f64 + w.ox, w.oz - inst[2] as f64);
                let sunk = if kind == ROCK {
                    inst[4] as f64 * 0.25
                } else {
                    sunk
                };
                let base = inst[1] as f64 + sunk;
                let r = foot(inst);
                let mut lo = ground(x, z);
                for k in 0..16 {
                    let a = k as f64 * std::f64::consts::TAU / 16.0;
                    lo = lo.min(ground(x + a.cos() * r, z + a.sin() * r));
                }
                // Its model's bottom is in the ground all round its base:
                // a bale's rests 2 cm up, a wall's footing reaches 55 cm
                // down; the others start at their foot.
                let bottom = inst[1] as f64
                    + match kind {
                        BALE => 0.02,
                        WALL => -0.55,
                        _ => 0.0,
                    };
                assert!(
                    bottom <= lo + 0.01,
                    "kind {kind} {:.2} m over the ground",
                    bottom - lo
                );
                assert!(base >= lo - 0.3, "kind {kind} {:.2} m too deep", lo - base);
                if ground(x, z) - lo > 0.1 {
                    on_slopes += 1;
                }
                checked += 1;
            }
        }
        assert!(checked > 1000, "{checked}");
        assert!(on_slopes > 50, "{on_slopes} on slopes");
    }

    /// The ground as drawn in a window (the triangles between its vertices).
    fn drawn(w: &TerrainData) -> impl Fn(f64, f64) -> f64 + '_ {
        let n = WINDOW_VERTS;
        let cell = WINDOW_SIZE / (n - 1) as f64;
        let half = WINDOW_SIZE / 2.0;
        move |x: f64, z: f64| {
            let (u, v) = ((x - w.ox + half) / cell, (z - w.oz + half) / cell);
            let (i, j) = (
                (u.floor().max(0.0) as usize).min(n - 2),
                (v.floor().max(0.0) as usize).min(n - 2),
            );
            let h = |i: usize, j: usize| w.verts[(j * n + i) * TERRAIN_STRIDE + 1] as f64;
            let (fx, fz) = (u - i as f64, v - j as f64);
            if fx + fz <= 1.0 {
                h(i, j) + (h(i + 1, j) - h(i, j)) * fx + (h(i, j + 1) - h(i, j)) * fz
            } else {
                h(i + 1, j + 1)
                    + (h(i, j + 1) - h(i + 1, j + 1)) * (1.0 - fx)
                    + (h(i + 1, j) - h(i + 1, j + 1)) * (1.0 - fz)
            }
        }
    }

    #[test]
    fn nothing_stands_in_the_water_on_cliffs_or_in_the_fields() {
        for id in ["valley-loop", "lakeside", "island-coast", "col-loop"] {
            let r = crate::route::builtin()
                .into_iter()
                .find(|r| r.id == id)
                .unwrap();
            let g = RouteGeo::new(&r);
            let (mut bales, mut checked) = (0, 0);
            for at in [1500.0, 9000.0] {
                let p = r.at(at);
                let w = g.build_window(p.x, p.z, WINDOW_SIZE, WINDOW_VERTS);
                let ground = drawn(&w);
                let world = |inst: &[f32]| (inst[0] as f64 + w.ox, w.oz - inst[2] as f64);
                for kind in [PINE, BROADLEAF, BIRCH, OAK, MAPLE, BUSH, ROCK, BALE] {
                    for inst in w.instances[kind].chunks(INSTANCE_STRIDE) {
                        let (x, z) = world(inst);
                        // Garden trees stand in their gardens.
                        if g.farms.iter().any(|b| (b.x - x).hypot(b.z - z) < 0.5) {
                            continue;
                        }
                        let e = 2.0;
                        let slope = (ground(x + e, z) - ground(x - e, z))
                            .hypot(ground(x, z + e) - ground(x, z - e))
                            / (2.0 * e);
                        assert!(
                            slope <= 0.9 + 1e-6,
                            "{id}: kind {kind} on a {slope:.2} slope"
                        );
                        if let Some(level) = g.water_level {
                            assert!(
                                ground(x, z) >= level + 0.6 - 1e-6,
                                "{id}: kind {kind} in water"
                            );
                        }
                        let smp = g.sample(x, z);
                        if kind == BALE {
                            let f = Field::at(x, z);
                            assert!(
                                (f.crop < 0.2 || (0.58..0.74).contains(&f.crop)) && f.edge >= 6.5,
                                "{id}: a bale outside a mown field"
                            );
                            assert!(g.farms.iter().any(|b| (x - b.x).hypot(z - b.z) < 505.0));
                            bales += 1;
                        } else {
                            assert!(
                                !g.in_field(x, z, &smp, &ground),
                                "{id}: kind {kind} in a field"
                            );
                        }
                        checked += 1;
                    }
                }
            }
            assert!(checked > 1000, "{id}: {checked}");
            if id == "valley-loop" || id == "lakeside" {
                assert!(bales > 0, "{id}: no bales");
            } else {
                assert_eq!(bales, 0, "{id}: bales outside farmland");
            }
        }
    }

    #[test]
    fn the_land_between_the_legs_of_a_hairpin_rises_evenly() {
        // Two legs 140 m apart, 60 m apart in height, joined by a hairpin:
        // the land between climbs from one to the other as one slope.
        let mut pts = Vec::new();
        let leg = 1000.0;
        let n = (leg / STEP) as usize;
        let rise = 60.0;
        for i in 0..=n {
            let u = i as f64 / n as f64;
            pts.push(route::RoutePt {
                x: 0.0,
                z: u * leg,
                ele: 500.0 + 0.5 * rise * u,
                heading: 0.0,
            });
        }
        for k in 1..28 {
            let a = k as f64 / 28.0 * std::f64::consts::PI;
            pts.push(route::RoutePt {
                x: 70.0 - 70.0 * a.cos(),
                z: leg + 70.0 * a.sin(),
                ele: 500.0 + 0.5 * rise,
                heading: 0.0,
            });
        }
        for i in 0..=n {
            let u = i as f64 / n as f64;
            pts.push(route::RoutePt {
                x: 140.0,
                z: leg - u * leg,
                ele: 500.0 + 0.5 * rise + 0.5 * rise * u,
                heading: 0.0,
            });
        }
        let (course, beyond) = (pts, Vec::new());
        let mut course = course;
        // The road beyond: straight on south.
        let last = *course.last().unwrap();
        let mut beyond = beyond;
        for i in 1..400 {
            beyond.push(route::RoutePt {
                x: 140.0,
                z: last.z - i as f64 * STEP,
                ele: last.ele,
                heading: 0.0,
            });
        }
        course.dedup_by(|a, b| (a.x - b.x).hypot(a.z - b.z) < 1.0);
        let r = Route::point_to_point("hairpin", "Hairpin", Scenery::Alpine, 3, course, beyond);
        let g = RouteGeo::new(&r);
        // Across the gap near the bottom, where the legs differ most.
        let z = 150.0;
        let (lo, hi) = (
            g.base_height(12.0, z, &g.nearest(12.0, z)).0,
            g.base_height(128.0, z, &g.nearest(128.0, z)).0,
        );
        let mean = (hi - lo).abs() / 116.0;
        let mut steepest = 0.0f64;
        for k in 0..58 {
            let (x0, x1) = (12.0 + k as f64 * 2.0, 14.0 + k as f64 * 2.0);
            let (h0, h1) = (
                g.base_height(x0, z, &g.nearest(x0, z)).0,
                g.base_height(x1, z, &g.nearest(x1, z)).0,
            );
            steepest = steepest.max((h1 - h0).abs() / 2.0);
        }
        assert!(hi - lo > 0.6 * 0.47 * rise, "{lo:.1} to {hi:.1}");
        assert!(
            steepest < 1.35 * mean,
            "steepest {steepest:.2} against {mean:.2} on average"
        );
    }

    /// A climb of twelve legs `gap` m apart up an 8 % slope joined by
    /// hairpins, imported as from a GPX file.
    fn hairpin_climb(gap: f64) -> Route {
        let mut xml = String::from("<gpx><trk><trkseg>");
        let (mut x, mut z, mut ele) = (0.0f64, 0.0f64, 700.0f64);
        let push = |x: f64, z: f64, e: f64, xml: &mut String| {
            let lat = 45.0 + (z / 6_371_000.0).to_degrees();
            let lon = 6.0 + (x / (6_371_000.0 * 45f64.to_radians().cos())).to_degrees();
            *xml += &format!(r#"<trkpt lat="{lat:.7}" lon="{lon:.7}"><ele>{e:.1}</ele></trkpt>"#);
        };
        for leg in 0..12 {
            let dir = if leg % 2 == 0 { 1.0 } else { -1.0 };
            for _ in 0..60 {
                z += dir * 10.0;
                ele += 0.8;
                push(x, z, ele, &mut xml);
            }
            for k in 1..12 {
                let a = std::f64::consts::PI * k as f64 / 12.0;
                push(
                    x + gap / 2.0 - gap / 2.0 * a.cos(),
                    z + dir * gap * 0.4 * a.sin(),
                    ele,
                    &mut xml,
                );
            }
            x += gap;
        }
        xml += "</trkseg></trk></gpx>";
        let mut g = route::parse_gpx(&xml, "hairpins", "Hairpins").unwrap();
        g.scenery = Some(Scenery::Alpine);
        g.to_route()
    }

    #[test]
    fn a_window_knows_its_ground_as_drawn() {
        let r = &route::builtin()[0];
        let g = RouteGeo::new(r);
        let p = r.at(500.0);
        let w = g.build_window(p.x, p.z, WINDOW_SIZE, WINDOW_VERTS);
        let ground = drawn(&w);
        for k in 0..50 {
            let (x, z) = (p.x + k as f64 * 13.7 - 300.0, p.z + k as f64 * 7.3 - 150.0);
            assert!((w.ground(x, z).unwrap() - ground(x, z)).abs() < 1e-6);
        }
        assert!(w.ground(p.x + 5000.0, p.z).is_none());
    }

    #[test]
    fn no_land_rises_over_any_road() {
        let mut routes = vec![route::builtin()[0].clone(), route::builtin()[4].clone()];
        routes.push(hairpin_climb(40.0));
        // Legs closer than two roads' width and verges.
        routes.push(hairpin_climb(26.0));
        for r in routes {
            let g = RouteGeo::new(&r);
            let (mut checked, mut worst) = (0, f64::MIN);
            for at in [r.length * 0.3, r.length * 0.7] {
                let p = r.at(at);
                let w = g.build_window(p.x, p.z, WINDOW_SIZE, WINDOW_VERTS);
                let ground = drawn(&w);
                let half = WINDOW_SIZE / 2.0 - 20.0;
                let mut d = 0.0;
                while d < r.span() {
                    let q = r.at(d);
                    d += 2.5;
                    if (q.x - w.ox).abs() > half || (q.z - w.oz).abs() > half {
                        continue;
                    }
                    for k in -6..=6 {
                        let lat = k as f64 * 5.6 / 6.0;
                        let (x, z) = (q.x + lat * q.heading.cos(), q.z - lat * q.heading.sin());
                        // The road's surface there, edges a little lower.
                        let surface = q.ele - if lat.abs() > 4.6 { 0.25 } else { 0.0 };
                        let over = ground(x, z) - surface;
                        worst = worst.max(over);
                        checked += 1;
                    }
                }
            }
            assert!(checked > 1000, "{}: {checked}", r.id);
            assert!(worst < 0.02, "{}: land {worst:.2} m over the road", r.id);
        }
    }

    #[test]
    fn walls_stand_on_the_land_side_of_lakes_and_the_sea() {
        let mut routes: Vec<Route> = route::builtin()
            .into_iter()
            .filter(|r| r.id == "lakeside" || r.id == "island-coast")
            .collect();
        for seed in 1..3 {
            for scenery in [Scenery::Lake, Scenery::Coast] {
                routes.push(
                    crate::routegen::RandomRoute {
                        seed,
                        scenery,
                        km: 20,
                        hills: crate::routegen::Hills::Rolling,
                        shape: route::Shape::Loop,
                    }
                    .route(),
                );
            }
        }
        for r in routes {
            let id = r.id.clone();
            let g = RouteGeo::new(&r);
            let mut n = 0;
            for at in [2000.0, 9000.0] {
                let p = r.at(at);
                let w = g.build_window(p.x, p.z, WINDOW_SIZE, WINDOW_VERTS);
                for inst in w.instances[WALL].chunks(INSTANCE_STRIDE) {
                    let (x, z) = (inst[0] as f64 + w.ox, w.oz - inst[2] as f64);
                    let near = g.nearest(x, z);
                    assert_ne!(near.side, g.water_side, "{id}: a wall on the water side");
                    n += 1;
                }
            }
            assert!(n > 0, "{id}: no walls");
        }
    }

    #[test]
    fn walls_run_along_the_fields_and_bales_lie_in_them() {
        let r = &route::builtin()[0];
        let g = RouteGeo::new(r);
        let (mut walls, mut joined, mut bales) = (0, 0, 0);
        for at in [1500.0, 5000.0, 9000.0] {
            let p = r.at(at);
            let w = g.build_window(p.x, p.z, WINDOW_SIZE, WINDOW_VERTS);
            let world = |inst: &[f32]| (inst[0] as f64 + w.ox, w.oz - inst[2] as f64);
            let pieces: Vec<(f64, f64)> = w.instances[WALL]
                .chunks(INSTANCE_STRIDE)
                .map(world)
                .collect();
            for &(x, z) in &pieces {
                let smp = g.sample(x, z);
                assert!(!smp.water && smp.dist > 7.5, "a wall at {x:.0}, {z:.0}");
                // Pieces stand end to end: one a piece's length away.
                let piece = fields::WALL_PIECE as f64;
                if pieces
                    .iter()
                    .any(|&(a, b)| ((a - x).hypot(b - z) - piece).abs() < 0.15)
                {
                    joined += 1;
                }
            }
            walls += pieces.len();
            for inst in w.instances[BALE].chunks(INSTANCE_STRIDE) {
                let (x, z) = world(inst);
                let smp = g.sample(x, z);
                assert!(!smp.water && smp.dist > 16.0, "a bale at {x:.0}, {z:.0}");
                bales += 1;
            }
        }
        println!("{walls} pieces of wall, {joined} joined, {bales} bales");
        assert!(walls > 100, "{walls} pieces of wall");
        assert!(
            joined as f64 > 0.95 * walls as f64,
            "{joined} of {walls} joined"
        );
        assert!(bales > 10, "{bales} bales");
    }

    #[test]
    fn lake_is_inside_the_loop() {
        let r = crate::route::builtin()
            .into_iter()
            .find(|r| r.id == "lakeside")
            .unwrap();
        let geo = RouteGeo::new(&r);
        let level = geo.water_level.unwrap();
        // Average of all points lies inside the loop.
        let n = r.points().len() as f64;
        let cx = r.points().iter().map(|p| p.x).sum::<f64>() / n;
        let cz = r.points().iter().map(|p| p.z).sum::<f64>() / n;
        let s = geo.sample(cx, cz);
        assert!(s.water && s.h < level, "{s:?}");
    }

    #[test]
    fn window_has_terrain_and_trees() {
        let r = &route::builtin()[0];
        let geo = RouteGeo::new(r);
        let p = r.at(100.0);
        let w = geo.build_window(p.x, p.z, 600.0, 61);
        assert_eq!(w.verts.len(), 61 * 61 * TERRAIN_STRIDE);
        assert_eq!(w.indices.len(), 60 * 60 * 6);
        let (pines, broad) = (
            w.instances[PINE].len() / INSTANCE_STRIDE,
            [BROADLEAF, BIRCH, OAK, MAPLE]
                .iter()
                .map(|&k| w.instances[k].len())
                .sum::<usize>()
                / INSTANCE_STRIDE,
        );
        assert!(!w.instances[BUSH].is_empty());
        assert!(pines + broad > 50);
        // Hills scenery is mostly broadleaf with some conifers.
        assert!(broad > pines && pines > 0, "pines {pines} broad {broad}");
        // Not every cell has a tree.
        let cells = (600.0f64 / 11.0).powi(2) as usize;
        assert!(
            pines + broad < cells * 3 / 4,
            "{} of {cells}",
            pines + broad
        );
        assert!(w.verts.iter().all(|v| v.is_finite()));
    }
}

#[cfg(test)]
mod survey {
    use super::*;

    /// Survey a saved route (`VELOFORGE_ROUTE_JSON`): land over the road,
    /// road over the land, every 800 m.
    #[test]
    #[ignore]
    fn survey_route() {
        let json = std::fs::read_to_string(std::env::var("VELOFORGE_ROUTE_JSON").unwrap()).unwrap();
        let g: crate::route::GpxRoute = serde_json::from_str(&json).unwrap();
        let r = g.to_route();
        let geo = RouteGeo::new(&r);
        let mut c = 0.0;
        while c < r.length {
            let p = r.at(c);
            let w = geo.build_window(p.x, p.z, WINDOW_SIZE, WINDOW_VERTS);
            let (mut over, mut over_at, mut gap, mut gap_at) = (f64::MIN, 0.0, f64::MIN, 0.0);
            let mut d = (c - 400.0).max(0.0);
            while d < (c + 400.0).min(r.length) {
                let q = r.at(d);
                for k in -6..=6 {
                    let lat = k as f64 * 5.6 / 6.0;
                    let (x, z) = (q.x + lat * q.heading.cos(), q.z - lat * q.heading.sin());
                    let surface = q.ele - if lat.abs() > 4.6 { 0.25 } else { 0.0 };
                    if let Some(h) = w.ground(x, z) {
                        if h - surface > over {
                            over = h - surface;
                            over_at = d;
                        }
                    }
                }
                for side in [-1.0, 1.0] {
                    let lat = side * 6.5;
                    let (x, z) = (q.x + lat * q.heading.cos(), q.z - lat * q.heading.sin());
                    if let Some(h) = w.ground(x, z) {
                        if q.ele - h > gap {
                            gap = q.ele - h;
                            gap_at = d;
                        }
                    }
                }
                d += 2.5;
            }
            println!(
                "{:5.0} m: land over road {:5.2} (at {:5.0}), road over land beside it {:5.1} (at {:5.0})",
                c, over, over_at, gap, gap_at
            );
            c += 800.0;
        }
        if let Ok(at) = std::env::var("VELOFORGE_TURNS") {
            let d0: f64 = at.parse().unwrap();
            let pts = r.points();
            let i0 = (d0 / crate::route::STEP) as usize;
            for i in i0..i0 + 30 {
                let (a, b, c) = (pts[i - 1], pts[i], pts[i + 1]);
                let h1 = (b.x - a.x).atan2(b.z - a.z);
                let h2 = (c.x - b.x).atan2(c.z - b.z);
                let mut t = h2 - h1;
                if t > std::f64::consts::PI {
                    t -= std::f64::consts::TAU;
                }
                if t < -std::f64::consts::PI {
                    t += std::f64::consts::TAU;
                }
                println!(
                    "{:6.0} turn {:+6.1} deg  heading {:+.2}  ele {:.1}",
                    i as f64 * 5.0,
                    t.to_degrees(),
                    b.heading,
                    b.ele
                );
            }
        }
        if let Ok(at) = std::env::var("VELOFORGE_AT") {
            let d: f64 = at.parse().unwrap();
            let q = r.at(d);
            let w = geo.build_window(q.x, q.z, WINDOW_SIZE, WINDOW_VERTS);
            let cell = WINDOW_SIZE / (WINDOW_VERTS - 1) as f64;
            println!("road at {d}: ele {:.1} heading {:.2}", q.ele, q.heading);
            for k in -6..=6 {
                let lat = k as f64 * 5.6 / 6.0;
                let (x, z) = (q.x + lat * q.heading.cos(), q.z - lat * q.heading.sin());
                println!(
                    "  lat {lat:+.1}: ground {:.2} road {:?} near {:.1}",
                    w.ground(x, z).unwrap(),
                    geo.road_surface(x, z),
                    geo.nearest(x, z).dist
                );
            }
            // The window's points around.
            let n = WINDOW_VERTS;
            let half = WINDOW_SIZE / 2.0;
            let (u, v) = ((q.x - w.ox + half) / cell, (q.z - w.oz + half) / cell);
            for j in (v as usize).saturating_sub(1)..=(v as usize + 2) {
                for i in (u as usize).saturating_sub(1)..=(u as usize + 2) {
                    let (x, z) = (w.ox - half + i as f64 * cell, w.oz - half + j as f64 * cell);
                    let h = w.verts[(j * n + i) * TERRAIN_STRIDE + 1];
                    println!(
                        "  vertex ({i},{j}): h {h:.2} road {:?} near {:.1}",
                        geo.road_surface(x, z),
                        geo.nearest(x, z).dist
                    );
                }
            }
        }
    }
}

/// A survey of every kind of route (builtins, random routes of each scenery
/// and shape, imports of several kinds) to compare before and after a
/// change to the land or the road: how far any land rises over the road,
/// how much retaining wall shows, the sharpest turn, the steepest grade and
/// how far an import strays from its track. `VELOFORGE_ALPE` adds a GPX file.
/// `cargo test --release survey_all_kinds -- --ignored --nocapture`
#[cfg(test)]
mod survey_many {
    use super::*;
    use crate::route;

    fn gpx(pts: &[(f64, f64, f64)]) -> String {
        let mut xml = String::from("<gpx><trk><trkseg>");
        for &(x, z, e) in pts {
            let lat = 45.0 + (z / 6_371_000.0).to_degrees();
            let lon = 6.0 + (x / (6_371_000.0 * 45f64.to_radians().cos())).to_degrees();
            xml += &format!(r#"<trkpt lat="{lat:.7}" lon="{lon:.7}"><ele>{e:.1}</ele></trkpt>"#);
        }
        xml + "</trkseg></trk></gpx>"
    }

    type Track = Vec<(f64, f64, f64)>;

    /// Imports of other kinds than a hairpin climb.
    fn imports() -> Vec<(String, Track, Scenery)> {
        let mut out = Vec::new();
        // A town loop: blocks with square corners at junctions.
        let corners: [(f64, f64); 9] = [
            (0.0, 0.0),
            (300.0, 0.0),
            (300.0, 200.0),
            (500.0, 200.0),
            (500.0, 500.0),
            (150.0, 500.0),
            (150.0, 250.0),
            (0.0, 250.0),
            (0.0, 0.0),
        ];
        let mut town = Vec::new();
        for w in corners.windows(2) {
            let ((x0, z0), (x1, z1)) = (w[0], w[1]);
            let n = (x1 - x0).hypot(z1 - z0) / 10.0;
            for i in 0..n as usize {
                let u = i as f64 / n;
                town.push((
                    x0 + (x1 - x0) * u,
                    z0 + (z1 - z0) * u,
                    120.0 + 3.0 * (u * 6.0).sin(),
                ));
            }
        }
        out.push(("town loop, square corners".into(), town, Scenery::Hills));
        // A winding descent: S-bends down a valley side.
        let mut s_bends = Vec::new();
        let (mut x, mut z, mut h, mut e) = (0.0f64, 0.0f64, 0.0f64, 900.0f64);
        for i in 0..900 {
            h += 0.06 * ((i as f64) * 0.05).sin();
            x += 8.0 * h.sin();
            z += 8.0 * h.cos();
            e -= 0.5;
            s_bends.push((x, z, e));
        }
        out.push(("winding descent, S-bends".into(), s_bends, Scenery::Alpine));
        // Out and back on the same road.
        let mut there: Vec<(f64, f64, f64)> = (0..400)
            .map(|i| {
                let u = i as f64 * 10.0;
                (u, 200.0 * (u / 900.0).sin(), 50.0 + u * 0.03)
            })
            .collect();
        let back: Vec<(f64, f64, f64)> = there
            .iter()
            .rev()
            .skip(1)
            .map(|&(x, z, e)| (x + 1.5, z, e))
            .collect();
        there.extend(back);
        out.push(("out and back".into(), there, Scenery::Lake));
        // A recorded ride: GPS wander and jumpy heights on country roads.
        let mut rec = Vec::new();
        let (mut x, mut z, mut h) = (0.0f64, 0.0f64, 0.3f64);
        for i in 0..1500 {
            let n = |s: u32| ((s as f64 * 12.9898).sin() * 43758.5453).fract() - 0.5;
            h += 0.04 * ((i as f64) * 0.013).sin() + 0.03 * n(i);
            x += 7.0 * h.sin() + 2.0 * n(i + 7);
            z += 7.0 * h.cos() + 2.0 * n(i + 99);
            rec.push((
                x,
                z,
                200.0 + 40.0 * ((i as f64) * 0.004).sin() + 3.0 * n(i + 500),
            ));
        }
        out.push(("recorded ride, GPS noise".into(), rec, Scenery::Coast));
        out
    }

    fn survey(r: &Route, track: Option<&[(f64, f64)]>) -> String {
        let geo = RouteGeo::new(r);
        assert!(
            r.points()
                .iter()
                .all(|p| p.x.is_finite() && p.ele.is_finite()),
            "{}: not a number",
            r.name
        );
        let (mut over, mut walls, mut tall_walls, mut samples) = (f64::MIN, 0usize, 0usize, 0usize);
        let mut c = 0.0;
        while c < r.length {
            let p = r.at(c);
            let w = geo.build_window(p.x, p.z, WINDOW_SIZE, WINDOW_VERTS);
            let mut d = c;
            while d < (c + 800.0).min(r.length) {
                let q = r.at(d);
                for k in -6..=6 {
                    let lat = k as f64 * 5.6 / 6.0;
                    let (x, z) = (q.x + lat * q.heading.cos(), q.z - lat * q.heading.sin());
                    let surface = q.ele - if lat.abs() > 4.6 { 0.25 } else { 0.0 };
                    if let Some(h) = w.ground(x, z) {
                        over = over.max(h - surface);
                    }
                }
                for side in [-1.0, 1.0] {
                    let lat = side * 6.5;
                    let (x, z) = (q.x + lat * q.heading.cos(), q.z - lat * q.heading.sin());
                    if let Some(h) = w.ground(x, z) {
                        samples += 1;
                        if q.ele - h > 1.0 {
                            walls += 1;
                        }
                        if q.ele - h > 4.0 {
                            tall_walls += 1;
                        }
                    }
                }
                d += 5.0;
            }
            c += 800.0;
        }
        let pts = r.course_points();
        let wrap = |a: f64| {
            (a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
        };
        let turn = (2..pts.len())
            .map(|i| {
                let h1 = (pts[i - 1].x - pts[i - 2].x).atan2(pts[i - 1].z - pts[i - 2].z);
                let h2 = (pts[i].x - pts[i - 1].x).atan2(pts[i].z - pts[i - 1].z);
                wrap(h2 - h1).abs().to_degrees()
            })
            .fold(0.0, f64::max);
        let stray = track.map(|t| {
            pts.iter()
                .map(|p| {
                    t.iter()
                        .map(|q| (q.0 - p.x).hypot(q.1 - p.z))
                        .fold(f64::MAX, f64::min)
                })
                .fold(0.0, f64::max)
        });
        let mut grade = 0.0f64;
        let mut d = 0.0;
        while d < r.length {
            grade = grade.max(r.grade(d).abs());
            d += 10.0;
        }
        format!(
            "{:34} {:5.1} km  land over road {:+5.2} m  walls >1 m {:4.1} %  >4 m {:4.1} %  sharpest turn {:4.1} deg/5 m  steepest {:4.1} %{}",
            r.name.chars().take(34).collect::<String>(),
            r.length / 1000.0,
            over,
            100.0 * walls as f64 / samples.max(1) as f64,
            100.0 * tall_walls as f64 / samples.max(1) as f64,
            turn,
            grade,
            stray.map(|s| format!("  strays {s:4.1} m")).unwrap_or_default()
        )
    }

    #[test]
    #[ignore]
    fn survey_all_kinds_of_route() {
        for r in route::builtin() {
            println!("{}", survey(&r, None));
        }
        for (k, scenery) in Scenery::ALL.into_iter().enumerate() {
            for shape in route::Shape::ALL {
                let r = crate::routegen::RandomRoute {
                    seed: 100 + k as u64,
                    scenery,
                    km: 15,
                    hills: crate::routegen::Hills::Hilly,
                    shape,
                }
                .route();
                println!("{}", survey(&r, None));
            }
        }
        if let Ok(path) = std::env::var("VELOFORGE_ALPE") {
            let xml = std::fs::read_to_string(path).unwrap();
            let mut g = route::parse_gpx(&xml, "alpe", "Alpe d'Huez").unwrap();
            g.scenery = Some(Scenery::Alpine);
            let track: Vec<(f64, f64)> = g.pts.iter().map(|p| (p.2, p.3)).collect();
            println!("{}", survey(&g.to_route(), Some(&track)));
        }
        for (name, pts, scenery) in imports() {
            let mut g = route::parse_gpx(&gpx(&pts), "import", &name).unwrap();
            g.scenery = Some(scenery);
            g.name = name;
            let track: Vec<(f64, f64)> = g.pts.iter().map(|p| (p.2, p.3)).collect();
            let r = g.to_route();
            println!("{}", survey(&r, Some(&track)));
        }
    }
}
