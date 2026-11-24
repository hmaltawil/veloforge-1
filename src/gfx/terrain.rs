//! World-space terrain around a route: a height function shaped by the
//! distance to the road (the road sits in a valley), forests, rocks,
//! bushes, houses and lakes. Windows of terrain are built on a worker thread.

use super::fields;
use super::flora::{Cover, Palette, Spot};
use super::mountains;
use crate::route::{Route, STEP};
use crate::scenery::{Buildings, LandShape, Planting, Water};
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
pub const KINDS: usize = 22;
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
pub const SAKURA: usize = 13;
pub const MOMIJI: usize = 14;
pub const SUGI: usize = 15;
pub const KUROMATSU: usize = 16;
pub const MINKA: usize = 17;
pub const TORII: usize = 18;
pub const TORO: usize = 19;
pub const PAGODA: usize = 20;
pub const TEA: usize = 21;

/// How the renderer treats a kind of scattered object.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// Full models close by, cards further out, dissolving from one to the
    /// other; swaying in the wind.
    Tree,
    /// Swaying in the wind; drawn only close by.
    Bush,
    /// Farm buildings and houses.
    Building,
    /// Small things standing still (rocks, bales, walls); drawn only close
    /// by.
    Still,
}

/// How far an object reaches from its foot, for seeing whether it (or its
/// shadow) is in view.
#[derive(Clone, Copy)]
pub enum Reach {
    /// Its height (instances carry it).
    Height,
    /// This far, times its scale (buildings, modelled in metres).
    Scaled(f32),
    /// This far, whatever its size.
    Fixed(f32),
}

/// One kind of scattered object: how it is drawn.
pub struct ObjectKind {
    pub group: Group,
    pub reach: Reach,
    /// Its model.
    pub model: fn() -> super::mesh::Mesh,
    /// How far off it is drawn, if nearer than its group's distance.
    pub draw_to: Option<f32>,
}

/// Every kind of scattered object, in the order of the numbers above. A
/// new kind is a number, an entry here, and where it is placed (in
/// `RouteGeo::scatter` or alongside it); the renderer draws it from this.
pub const OBJECTS: [ObjectKind; KINDS] = {
    use super::{buildings, fields, japanese, mesh};
    const fn kind(group: Group, reach: Reach, model: fn() -> mesh::Mesh) -> ObjectKind {
        ObjectKind {
            group,
            reach,
            model,
            draw_to: None,
        }
    }
    // Buildings reach up to about 12 m from their middle.
    const BUILT: Reach = Reach::Scaled(12.0);
    [
        kind(Group::Tree, Reach::Height, mesh::pine),
        kind(Group::Tree, Reach::Height, mesh::broadleaf),
        kind(Group::Still, Reach::Height, mesh::rock),
        kind(Group::Building, BUILT, buildings::farmhouse),
        kind(Group::Tree, Reach::Height, mesh::birch),
        kind(Group::Bush, Reach::Height, mesh::bush),
        kind(Group::Building, BUILT, buildings::barn),
        kind(Group::Building, BUILT, buildings::shed),
        kind(Group::Building, BUILT, buildings::townhouse),
        kind(Group::Tree, Reach::Height, mesh::oak),
        kind(Group::Tree, Reach::Height, mesh::maple),
        kind(Group::Still, Reach::Height, fields::hay_bale),
        kind(
            Group::Still,
            Reach::Fixed(fields::WALL_PIECE * 0.6),
            fields::stone_wall,
        ),
        kind(Group::Tree, Reach::Height, japanese::sakura),
        kind(Group::Tree, Reach::Height, japanese::momiji),
        kind(Group::Tree, Reach::Height, japanese::sugi),
        kind(Group::Tree, Reach::Height, japanese::kuromatsu),
        kind(Group::Building, BUILT, japanese::minka),
        kind(Group::Building, Reach::Scaled(4.5), japanese::torii),
        kind(Group::Building, Reach::Scaled(1.4), japanese::toro),
        kind(Group::Building, Reach::Scaled(16.0), japanese::pagoda),
        // Tea fields are many small rows: drawn only nearer.
        ObjectKind {
            group: Group::Still,
            reach: Reach::Fixed(2.2),
            model: japanese::tea_row,
            draw_to: Some(300.0),
        },
    ]
};

/// Whether kind `k` is in group `g`.
pub fn is(k: usize, g: Group) -> bool {
    OBJECTS[k].group == g
}

/// How many kinds are in any of `groups` (for sizing the lists below).
pub const fn count_in(groups: &[Group]) -> usize {
    let mut n = 0;
    let mut k = 0;
    while k < KINDS {
        if in_groups(k, groups) {
            n += 1;
        }
        k += 1;
    }
    n
}

/// The kinds in any of `groups`, in order; `N` from [`count_in`].
pub const fn kinds_in<const N: usize>(groups: &[Group]) -> [usize; N] {
    let mut out = [0; N];
    let (mut n, mut k) = (0, 0);
    while k < KINDS {
        if in_groups(k, groups) {
            out[n] = k;
            n += 1;
        }
        k += 1;
    }
    assert!(n == N, "N does not match the number of kinds");
    out
}

const fn in_groups(k: usize, groups: &[Group]) -> bool {
    let mut i = 0;
    while i < groups.len() {
        if OBJECTS[k].group as u8 == groups[i] as u8 {
            return true;
        }
        i += 1;
    }
    false
}

/// How wide, for its size, each kind is where it meets the ground (radius):
/// a tree's trunk for its width, a bush's base, a rock's, a bale's.
pub(super) const TREE_FOOT: f64 = 0.1;
pub(super) const BUSH_FOOT: f64 = 0.3;
pub(super) const ROCK_FOOT: f64 = 0.35;
pub(super) const BALE_FOOT: f64 = 0.7;

/// A tree's instance and its blob shadow: `kind`, at `rel` (`at` in the
/// world), `height` and `width` (metres), turned by `turn` (0..1 of a
/// circle), its shade and its warmth.
#[allow(clippy::too_many_arguments)]
fn plant(
    out: &mut [Vec<f32>; KINDS],
    shadows: &mut Vec<f32>,
    kind: usize,
    (rx, rz): (f32, f32),
    ground: &dyn Fn(f64, f64) -> f64,
    (x, z): (f64, f64),
    height: f64,
    width: f64,
    turn: f64,
    tint: f32,
    warm: f32,
) {
    let foot = lowest(ground, x, z, TREE_FOOT * width) as f32;
    out[kind].extend_from_slice(&[
        rx,
        foot - 0.2,
        rz,
        (turn * std::f64::consts::TAU) as f32,
        width as f32,
        height as f32,
        tint * (0.95 + 0.4 * warm),
        tint * (1.0 + 0.08 * warm),
        tint * (0.9 - 0.3 * warm),
    ]);
    shadows.extend_from_slice(&[rx, ground(x, z) as f32 + 0.08, rz, width as f32 * 0.5]);
}

/// Where Japan's volcano stands for a route through `pts`: beyond the
/// route on a side its seed picks, its summit 7.5 km past the route's
/// furthest point from its middle, so it rises across the land from much
/// of the ride and stands wholly in the mountains.
fn volcano_site(pts: &[(f64, f64)], seed: u64) -> [f64; 2] {
    let n = pts.len().max(1) as f64;
    let (cx, cz) = pts
        .iter()
        .fold((0.0, 0.0), |(x, z), p| (x + p.0 / n, z + p.1 / n));
    let reach = pts
        .iter()
        .map(|p| (p.0 - cx).hypot(p.1 - cz))
        .fold(0.0, f64::max);
    let a = (hash2(17, 23, seed) % 3600) as f64 / 3600.0 * std::f64::consts::TAU;
    [
        cx + a.cos() * (reach + 7500.0),
        cz + a.sin() * (reach + 7500.0),
    ]
}

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
    /// The mountain ranges the land rises into far from the roads.
    pub mountains: mountains::Params,
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
        let spec = route.scenery.spec();
        let water_level = spec.water.level(min_ele, looped);
        // A lake fills the inside of a loop; the sea surrounds it (or lies
        // to the left of a point-to-point route).
        let water_side = spec.water.side(inside, looped);
        let grid = NearGrid::build(&pts);
        let volcano =
            (spec.mountains == mountains::Kind::Japan).then(|| volcano_site(&pts, route.seed));
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
            mountains: mountains::Params {
                kind: spec.mountains,
                seed: route.seed,
                min_ele,
                max_ele,
                water: water_level,
                snow: max_ele + SNOW_ABOVE,
                volcano,
            },
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
                .filter(|b| is(b.kind, Group::Building))
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
                    let level = if [HOUSE, TOWNHOUSE, MINKA].contains(&b.kind) {
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
        self.max_ele + SNOW_ABOVE
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
        let (mut sw_far, mut se_far) = (0.0, 0.0);
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
            // From every road, however far (cut off at some distance, the
            // land would step where a road dropped out of reach).
            let reach = CONE * (d - 9.0).max(0.0);
            lo = lo.max(self.eles[j] - reach);
            hi = hi.min(self.eles[j] + reach);
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
            sw_far += far;
            se_far += far * self.eles[j];
        }
        // Beyond the near roads' reach the land follows the whole route,
        // eased into from 1.1 km (as the near roads fade out, the faintest
        // weights left would otherwise take over all at once, a cliff).
        let blended = se_far / sw_far
            + (se / sw - se_far / sw_far) * (1.0 - smoothstep(1100.0, 1500.0, near.dist));
        // Where roads are steeper apart than the bounds allow, the blend
        // between them is the best there is.
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

    /// The land as it lies, before any levelling: shaped by the roads near
    /// by, rising into the mountains far from them.
    fn land(&self, x: f64, z: f64) -> Sample {
        let near = self.nearest(x, z);
        self.land_near(x, z, &near)
    }

    /// The land for the mountains' mesh beyond the distant land: as `land`,
    /// but all mountain (and quicker) where no road is near.
    fn mountain_ground(&self, x: f64, z: f64) -> mountains::Ground {
        let near = self.nearest(x, z);
        if near.dist >= mountains::FULL {
            let h = mountains::height(&self.mountains, x, z, near.dist);
            let forest = mountains::forest(&self.mountains, x, z, h);
            return mountains::Ground { h, forest };
        }
        let s = self.land_near(x, z, &near);
        mountains::Ground {
            h: s.h,
            forest: s.forest,
        }
    }

    /// `land`, given the nearest road.
    fn land_near(&self, x: f64, z: f64, near: &Near) -> Sample {
        let (road, lo, hi) = self.base_height(x, z, near);
        let s = self.seed;
        let flat = road - 0.25;
        // Far from the roads the land rises into the mountains.
        let w = mountains::share(near.dist);
        let m = if w > 0.0 {
            mountains::height(&self.mountains, x, z, near.dist)
        } else {
            0.0
        };
        let mountain = |h: f64, forest: f64| {
            if w > 0.0 {
                let h = h + (m - h) * w;
                let f = mountains::forest(&self.mountains, x, z, h);
                (h, forest + (f - forest) * w)
            } else {
                (h, forest)
            }
        };
        let under = |h: f64| self.water_level.is_some_and(|level| h < level);
        let forest_noise = fbm(x / 650.0 + 31.0, z / 650.0 - 7.0, 4, s + 5);
        let mut forest = smoothstep(-0.05, 0.3, forest_noise) * smoothstep(16.0, 40.0, near.dist);

        let spec = self.route.scenery.spec();
        if let (Some(level), Water::Sea) = (self.water_level, spec.water) {
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
                // Islands and headlands across the sea.
                let (h, forest) = mountain(h, 0.0);
                return Sample {
                    h,
                    dist: near.dist,
                    forest,
                    water: h < level,
                };
            }
        }
        if let Some(level) = self.water_level {
            if near.side == self.water_side && near.dist > 9.0 {
                // Lake shore falls away from the road into the water.
                let bed = level - 0.6 - smoothstep(14.0, 70.0, near.dist) * 7.0;
                // The higher the road stands above the lake, the further back
                // its shore lies: a bank about 1.8 m wide for each metre the
                // road is more than 4 m up, so a road climbing away from the
                // water runs above a slope, not along the top of a cliff (the
                // drawn ground would reach under its edge as a wall). A road
                // near the water keeps its shore 9 to 16 m off, as before.
                let bank = 7.0 + 1.8 * (flat - level - 4.0).max(0.0);
                // (The whole of it, however far from the road: no mountains
                // rise out of the lake.)
                let h = flat + (bed - flat) * smoothstep(9.0, 9.0 + bank, near.dist);
                return Sample {
                    h,
                    dist: near.dist,
                    forest: 0.0,
                    water: h < level,
                };
            }
        }

        let t = smoothstep(9.0, 170.0, near.dist);
        // The hills by the road rise with the distance from it, up to where
        // the mountains take over.
        let far = near.dist.min(900.0);
        let land = &spec.land;
        let hills = match land.shape {
            LandShape::Ridged => {
                let ridge = 1.0 - fbm(x / land.size, z / land.size, 5, s).abs();
                if spec.snow {
                    // Forest thins out towards the bare tops.
                    forest *= 1.0
                        - smoothstep(
                            self.max_ele + 40.0,
                            self.max_ele + 140.0,
                            road + 160.0 * ridge * ridge * t,
                        );
                }
                land.height * ridge * ridge * (land.near + far / land.growth)
            }
            LandShape::Rounded => {
                land.height
                    * (0.4 + 0.6 * (0.5 + 0.5 * fbm(x / land.size, z / land.size, 5, s)))
                    * (land.near + far / land.growth)
            }
        };
        let bumps = fbm(x / 55.0, z / 55.0, 3, s + 9) * 1.8;
        // Hills, mountains and all, no steeper from any road than the land
        // allows: the ground between the legs of a hairpin climb is the
        // slope between them, not a ridge.
        let (h, forest) = mountain(flat + t * (hills + bumps), forest);
        let h = h.clamp(lo - 0.25, hi.max(lo - 0.25));
        Sample {
            h,
            dist: near.dist,
            forest,
            water: w > 0.0 && under(h),
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
        // Row by row on two threads (more would take cores the app and the
        // driver need: on a four-core laptop frames slowed while it built).
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .clamp(1, 2);
        let rows: Vec<usize> = (0..m).collect();
        let mut samples = Vec::with_capacity(m * m);
        std::thread::scope(|sc| {
            let parts: Vec<_> = rows
                .chunks(m.div_ceil(threads))
                .map(|js| {
                    sc.spawn(move || {
                        let mut out = Vec::with_capacity(js.len() * m);
                        for &j in js {
                            for i in 0..m {
                                out.push(self.sample(
                                    x0 + (i as f64 - 1.0) * cell,
                                    z0 + (j as f64 - 1.0) * cell,
                                ));
                            }
                        }
                        out
                    })
                })
                .collect();
            for p in parts {
                samples.extend(p.join().expect("distant land"));
            }
        });
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
        let (pts, ground, edge_normals) = self.mountain_input(ox, oz, &verts);
        let mountains = mountains::build(&self.mountains, &pts, &ground, &edge_normals);
        FarData {
            ox,
            oz,
            verts,
            indices,
            mountains,
            route_id: self.route.id.clone(),
        }
    }

    /// What the mountains' mesh beyond the distant land centred on (`ox`,
    /// `oz`) is made from (`mountains::build`): where its vertices are, the
    /// land there, and the distant land's normals along its edge.
    #[allow(clippy::type_complexity)]
    pub fn mountain_input(
        &self,
        ox: f64,
        oz: f64,
        verts: &[f32],
    ) -> (Vec<[f64; 2]>, Vec<mountains::Ground>, Vec<[f32; 3]>) {
        // The mountains beyond, from the land's edge out: the first ring
        // is the edge itself (between its vertices, along the line between
        // them), so the two meet without a gap.
        let (size, n) = (FAR_SIZE, FAR_VERTS);
        let cell = size / (n - 1) as f64;
        let half = size / 2.0;
        let seam = mountains::seam(half, n - 1);
        let pts = mountains::points(&seam);
        let vert = |i: usize, j: usize| &verts[(j * n + i) * TERRAIN_STRIDE..][..TERRAIN_STRIDE];
        let mut ground = Vec::with_capacity(pts.len());
        let mut edge_normals = Vec::with_capacity(seam.len());
        for q in &seam {
            let fi = ((q[0] + half) / cell).clamp(0.0, (n - 1) as f64);
            let fj = ((q[1] + half) / cell).clamp(0.0, (n - 1) as f64);
            let (i0, j0) = ((fi as usize).min(n - 2), (fj as usize).min(n - 2));
            let (ti, tj) = ((fi - i0 as f64) as f32, (fj - j0 as f64) as f32);
            let mix = |k: usize| {
                let a = vert(i0, j0)[k] + (vert(i0 + 1, j0)[k] - vert(i0, j0)[k]) * ti;
                let b = vert(i0, j0 + 1)[k] + (vert(i0 + 1, j0 + 1)[k] - vert(i0, j0 + 1)[k]) * ti;
                a + (b - a) * tj
            };
            ground.push(mountains::Ground {
                h: mix(1) as f64,
                forest: mix(7) as f64,
            });
            let nn = [mix(3), mix(4), mix(5)];
            let l = (nn[0] * nn[0] + nn[1] * nn[1] + nn[2] * nn[2])
                .sqrt()
                .max(1e-6);
            edge_normals.push([nn[0] / l, nn[1] / l, nn[2] / l]);
        }
        // The rest on two threads (some 50,000 points, a fifth of a second
        // on one; more would take cores the app and the driver need).
        let rest = &pts[seam.len()..];
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .clamp(1, 2);
        let chunk = rest.len().div_ceil(threads);
        std::thread::scope(|sc| {
            let parts: Vec<_> = rest
                .chunks(chunk)
                .map(|part| {
                    sc.spawn(move || {
                        part.iter()
                            .map(|q| self.mountain_ground(ox + q[0], oz + q[1]))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            for p in parts {
                ground.extend(p.join().expect("mountain land"));
            }
        });
        (pts, ground, edge_normals)
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
            sea: self.route.scenery.spec().water == Water::Sea,
        }
    }

    /// The Japanese countryside along the road, inside the square from
    /// (x0, z0), `size` across: every 320 m or so a thatched farmhouse
    /// facing the road with a tree in its garden, or a small shrine (a gate
    /// facing the road between two stone lanterns, cedars behind it); and
    /// now and then a temple, its pagoda set back behind a gate that frames
    /// it from the road, cherries either side of it.
    fn villages(&self, x0: f64, z0: f64, size: f64) -> Vec<Building> {
        let mut out = Vec::new();
        let s = self.seed;
        let route = &self.route;
        let spacing = 320.0;
        for k in 0..(route.span() / spacing) as i64 {
            let mut rng = Rng(hash2(k, 0, s + 1993));
            let what = rng.next();
            let temple = k % 9 == 4;
            if !temple && what > 0.62 {
                continue;
            }
            let d = k as f64 * spacing + rng.next() * 100.0;
            let p = route.at(d);
            let mut side = if rng.next() < 0.5 { -1.0 } else { 1.0 };
            // A temple's pagoda stands back from the road: on the first of
            // a few sites, either side and nearer or further back, that is
            // dry land off the lake's bank and off the paddies.
            let mut back = 0.0;
            if temple {
                back = 58.0 + 8.0 * rng.next();
                let land = |x: f64, z: f64| self.sample(x, z).h;
                let free = |sd: f64, b: f64| {
                    let (x, z) = (
                        p.x + sd * b * p.heading.cos(),
                        p.z - sd * b * p.heading.sin(),
                    );
                    let smp = self.sample(x, z);
                    !smp.water
                        && smp.dist >= 15.0
                        && !self.by_the_lake(x, z)
                        && !self.on_paddies(x, z, &smp, &land)
                };
                let sites = [
                    (side, back),
                    (side, back + 18.0),
                    (-side, back),
                    (-side, back + 18.0),
                    (side, back - 14.0),
                    (-side, back - 14.0),
                ];
                if let Some(&(sd, b)) = sites.iter().find(|&&(sd, b)| free(sd, b)) {
                    (side, back) = (sd, b);
                }
            }
            let along = (p.heading.sin(), p.heading.cos());
            let away = (side * p.heading.cos(), -side * p.heading.sin());
            let facing = (std::f64::consts::FRAC_PI_2 - p.heading
                + if side > 0.0 {
                    std::f64::consts::PI
                } else {
                    0.0
                }) as f32;
            // A spot `out` from the road and `x` along it in the building's
            // own terms (its +x, east, lies at -side along the road).
            let at = |x: f64, out: f64| {
                (
                    p.x + away.0 * out - along.0 * side * x,
                    p.z + away.1 * out - along.1 * side * x,
                )
            };
            let inside =
                |(x, z): (f64, f64)| x >= x0 && x <= x0 + size && z >= z0 && z <= z0 + size;
            let mut place = |kind: usize, (x, z): (f64, f64), rot: f32, scale: f32, reach: f64| {
                if !inside((x, z)) {
                    return;
                }
                let smp = self.sample(x, z);
                let land = |x: f64, z: f64| self.sample(x, z).h;
                if smp.water
                    || smp.dist < 15.0
                    || self.by_the_lake(x, z)
                    || self.on_paddies(x, z, &smp, &land)
                {
                    return;
                }
                // It stands at the top of its ground; not on ground too
                // steep for that.
                let reach = reach * scale as f64;
                let (mut lo, mut hi) = (smp.h, smp.h);
                for j in 0..8 {
                    let a = j as f64 * std::f64::consts::FRAC_PI_4;
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
                    tint: [1.0, 1.0, 1.0],
                    reach,
                });
            };
            // Trees: a kind, a spot, and its height (metres).
            let mut trees: Vec<(usize, (f64, f64), f64)> = Vec::new();
            if temple {
                place(PAGODA, at(0.0, back), facing, 1.0, 9.0);
                place(TORII, at(0.0, 24.0), facing, 1.0, 4.5);
                for x in [-3.4, 3.4] {
                    place(TORO, at(x, 22.5), facing, 1.0, 1.4);
                }
                // Maples either side of the gate.
                for x in [-6.5, 6.5] {
                    trees.push((MOMIJI, at(x, 25.0 + 2.0 * rng.next()), 5.0 + rng.next()));
                }
                for x in [-9.0, 9.0] {
                    trees.push((
                        SAKURA,
                        at(x, back - 6.0 - 4.0 * rng.next()),
                        7.0 + 1.5 * rng.next(),
                    ));
                    trees.push((
                        SAKURA,
                        at(x * 0.8, 36.0 + 6.0 * rng.next()),
                        6.5 + 1.5 * rng.next(),
                    ));
                }
            } else if what < 0.5 {
                let lateral = 26.0 + 24.0 * rng.next();
                let scale = (0.92 + 0.16 * rng.next()) as f32;
                place(MINKA, at(0.0, lateral), facing, scale, 11.0);
                // A tree in the front garden, beyond the azaleas or the
                // water basin.
                let x = if rng.next() < 0.5 { -1.0 } else { 1.0 } * (6.0 + 1.2 * rng.next());
                let front = lateral - 4.0 * scale as f64 - 1.5 - 1.5 * rng.next();
                let pick = rng.next();
                let (kind, h) = if pick < 0.5 {
                    (SAKURA, 6.5 + 2.0 * rng.next())
                } else if pick < 0.8 {
                    (MOMIJI, 4.5 + 1.5 * rng.next())
                } else {
                    (KUROMATSU, 6.0 + 2.0 * rng.next())
                };
                trees.push((kind, at(x, front), h));
            } else {
                // A small shrine: the gate facing the road between two
                // lanterns, and a stand of cedars behind.
                let lateral = 20.0 + 6.0 * rng.next();
                place(
                    TORII,
                    at(0.0, lateral),
                    facing,
                    (0.85 + 0.15 * rng.next()) as f32,
                    4.5,
                );
                for x in [-3.2, 3.2] {
                    place(TORO, at(x, lateral - 1.5), facing, 0.9, 1.4);
                }
                // A maple beside the gate.
                let x = if rng.next() < 0.5 { -5.5 } else { 5.5 };
                trees.push((MOMIJI, at(x, lateral + 1.0), 4.5 + 1.2 * rng.next()));
                for j in 0..4 {
                    let x = -6.0 + 4.0 * j as f64 + 1.5 * (rng.next() - 0.5);
                    trees.push((
                        SUGI,
                        at(x, lateral + 7.0 + 4.0 * rng.next()),
                        20.0 + 6.0 * rng.next(),
                    ));
                }
            }
            for (kind, (x, z), height) in trees {
                if !inside((x, z)) {
                    continue;
                }
                let smp = self.sample(x, z);
                let land = |x: f64, z: f64| self.sample(x, z).h;
                if smp.water || smp.dist < 11.0 || self.on_paddies(x, z, &smp, &land) {
                    continue;
                }
                let width = match kind {
                    SAKURA => 1.35,
                    MOMIJI => 1.3,
                    SUGI => 0.42,
                    _ => 1.1,
                };
                out.push(Building {
                    kind,
                    farm: k,
                    x,
                    z,
                    h: smp.h - 0.2,
                    rot: (rng.next() * std::f64::consts::TAU) as f32,
                    scale: ((height * width) as f32, height as f32),
                    tint: [1.0, 1.0, 1.0],
                    reach: 3.0,
                });
            }
        }
        out
    }

    /// Trees, rocks and houses inside the window, plus blob shadows.
    /// Farmsteads along the road, inside the square from (x0, z0), `size`
    /// across: every 320 m or so, a house facing the road, often a barn
    /// behind it and sometimes a shed beside it.
    fn farmsteads(&self, x0: f64, z0: f64, size: f64) -> Vec<Building> {
        if self.route.scenery.spec().buildings == Buildings::Japanese {
            return self.villages(x0, z0, size);
        }
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
                if smp.water || smp.dist < 15.0 || self.by_the_lake(x, z) {
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
        if self.route.scenery.spec().paddies {
            return self.on_paddies(x, z, smp, ground);
        }
        if !self.route.scenery.spec().fields || smp.forest > 0.2 || smp.dist < 36.0 {
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

    /// Whether `x`, `z` is on rice paddies, or so near them that the ground
    /// may be painted as one (`TERRAIN_FS`): anywhere in a paddy field (not
    /// only away from its edge: rice fields have no hedgerows), with a
    /// margin round where the painting fades in by the road, at the
    /// woods' edge and on slopes. Nothing grows or stands there.
    fn on_paddies(&self, x: f64, z: f64, smp: &Sample, ground: &dyn Fn(f64, f64) -> f64) -> bool {
        if smp.forest > 0.36 || smp.dist < 26.0 {
            return false;
        }
        let e = 2.0;
        let slope = (ground(x + e, z) - ground(x - e, z))
            .hypot(ground(x, z + e) - ground(x, z - e))
            / (2.0 * e);
        slope < 0.4 && Field::at(x, z).crop < 0.74
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
        let spec = route.scenery.spec();
        let sides = spec.edge_sides(self.water_side);
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
                let walled = spec.walled.contains(&pick);
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

    /// Whether `x`, `z` is on a lake's bank, between the road and the
    /// water: no place for a building (the bank slopes down to the lake,
    /// and the higher the road, the wider it is).
    fn by_the_lake(&self, x: f64, z: f64) -> bool {
        self.route.scenery.spec().water == Water::Lake
            && self.water_level.is_some()
            && self.nearest(x, z).side == self.water_side
    }

    /// Whether a tree may be planted at `x`, `z`: on dry land at least
    /// `from_road` from the road, with room for `reach` around it, not in
    /// a field or on a building's ground.
    fn can_plant(
        &self,
        x: f64,
        z: f64,
        from_road: f64,
        reach: f64,
        ground: &dyn Fn(f64, f64) -> f64,
        farms: &[&Building],
    ) -> bool {
        let smp = self.sample(x, z);
        !smp.water
            && smp.dist >= from_road
            && self.room_to_stand(x, z, reach, ground)
            && !self.in_field(x, z, &smp, ground)
            && farms
                .iter()
                .all(|b| (x - b.x).powi(2) + (z - b.z).powi(2) >= (b.reach + 2.0).powi(2))
    }

    /// The Japanese countryside's woods, in the cell `gi`, `gj` (at `x`,
    /// `z`): where the land is wooded and well back from the road. Cedar
    /// stands where the large-scale pattern says so, their trees set
    /// unevenly, of an age but not of one height, their edges ragged and
    /// broadleaves mixed in there; elsewhere broadleaf woods with Japanese
    /// maples at their edges; and through both, now and then, a wild
    /// mountain cherry in flower. Whether the cell is wood (and so done
    /// with).
    #[allow(clippy::too_many_arguments)]
    fn japanese_woods(
        &self,
        (gi, gj): (i64, i64),
        (x, z): (f64, f64),
        smp: &Sample,
        ground: &dyn Fn(f64, f64) -> f64,
        rel: &dyn Fn(f64, f64) -> (f32, f32),
        farms: &[&Building],
        out: &mut [Vec<f32>; KINDS],
        shadows: &mut Vec<f32>,
    ) -> bool {
        // A ragged edge to the woods: the forest's own edge, roughened.
        let edge = 0.18 + 0.12 * fbm(x / 60.0 + 4.0, z / 60.0 - 2.0, 2, self.seed + 31);
        if smp.forest < edge {
            return false;
        }
        let s = self.seed;
        let mut rng = Rng(hash2(gi, gj, s + 4111));
        let cell = 11.0;
        // How cedar the stand is: deep in a plantation all cedar, at its
        // ragged edge mixed with broadleaves, beyond it broadleaf wood.
        let stand = fbm(x / 700.0 + 13.0, z / 700.0 - 5.0, 3, s + 21)
            + 0.18 * fbm(x / 120.0 - 7.0, z / 120.0 + 3.0, 2, s + 25);
        let cedar_share = smoothstep(-0.3, -0.05, stand);
        let age = 0.5 + 0.5 * fbm(x / 400.0 - 3.0, z / 400.0 + 9.0, 2, s + 23);
        // One tree a cell where the wood is open, sometimes two where it is
        // thick: set anywhere in the cell, not in rows.
        let n = 1 + usize::from(smp.forest > 0.55 && rng.next() < 0.45 * cedar_share + 0.2);
        for _ in 0..n {
            let px = (gi as f64 + 0.1 + 0.8 * rng.next()) * cell;
            let pz = (gj as f64 + 0.1 + 0.8 * rng.next()) * cell;
            let pick = rng.next();
            let wild_cherry = rng.next() < 0.05;
            let (kind, height, ratio, warm) = if wild_cherry {
                (
                    SAKURA,
                    7.5 + 3.5 * rng.next(),
                    1.1 + 0.4 * rng.next(),
                    0.5 * rng.next() as f32,
                )
            } else if pick < cedar_share {
                (
                    SUGI,
                    (15.0 + 9.0 * age) * (0.8 + 0.35 * rng.next()),
                    0.36 + 0.12 * rng.next(),
                    0.0,
                )
            } else if smp.forest < 0.45 && rng.next() < 0.45 {
                (
                    MOMIJI,
                    6.0 * (0.75 + 0.5 * rng.next()),
                    1.1 + 0.4 * rng.next(),
                    0.2,
                )
            } else {
                (
                    BROADLEAF,
                    10.5 * (0.65 + 0.7 * rng.next()),
                    0.85 + 0.3 * rng.next(),
                    0.7 * (rng.next() as f32).powi(2),
                )
            };
            if !self.can_plant(px, pz, 16.0, 2.5, ground, farms) {
                continue;
            }
            let tint = 0.82 + 0.36 * rng.next() as f32;
            plant(
                out,
                shadows,
                kind,
                rel(px, pz),
                ground,
                (px, pz),
                height,
                height * ratio,
                rng.next(),
                tint,
                warm,
            );
        }
        true
    }

    /// Along the road in the Japanese countryside, planted as people plant:
    /// cherries in groups of three to eight at uneven spacing, on one side
    /// or both, more often near a farm, a shrine or a temple, with gaps and
    /// the odd tree alone between; and along the lake shore black pines in
    /// small, uneven groups with open shore between. Every tree its own
    /// size, shape and shade.
    #[allow(clippy::too_many_arguments)]
    fn japanese_roadside(
        &self,
        (x0, z0, size): (f64, f64, f64),
        ground: &dyn Fn(f64, f64) -> f64,
        rel: &dyn Fn(f64, f64) -> (f32, f32),
        farms: &[&Building],
        out: &mut [Vec<f32>; KINDS],
        shadows: &mut Vec<f32>,
    ) {
        let route = &self.route;
        let s = self.seed;
        let inside = |(x, z): (f64, f64)| x >= x0 && x <= x0 + size && z >= z0 && z <= z0 + size;
        let near_window = |x: f64, z: f64| {
            x > x0 - 30.0 && x < x0 + size + 30.0 && z > z0 - 30.0 && z < z0 + size + 30.0
        };
        let beside = |d: f64, side: f64, lateral: f64| {
            let p = route.at(d);
            (
                p.x + side * lateral * p.heading.cos(),
                p.z - side * lateral * p.heading.sin(),
            )
        };
        // The cherries: walk along the road; now and then a group begins.
        // Every choice is made the same whatever the window, so each
        // stretch of road gets the same trees from every window; the
        // window only decides which of them are written.
        let mut rng = Rng(hash2(0, 0, s + 505));
        let mut d = 0.0;
        while d < route.span() {
            let p = route.at(d);
            let step = 20.0 + 40.0 * rng.next();
            // Groups are likelier near the farms, shrines and temples.
            let settled = self
                .farms
                .iter()
                .any(|b| (p.x - b.x).powi(2) + (p.z - b.z).powi(2) < 150.0 * 150.0);
            if rng.next() > if settled { 0.5 } else { 0.14 } {
                d += step;
                continue;
            }
            let trees = 1 + (rng.next() * rng.next() * 8.0) as usize;
            let both = rng.next() < 0.35;
            let first_side = if rng.next() < 0.5 { -1.0 } else { 1.0 };
            let mut at = d;
            for _ in 0..trees {
                for side in [first_side, -first_side] {
                    if side != first_side && !both {
                        continue;
                    }
                    let missing = rng.next() < 0.12;
                    let lateral = 9.8 + 4.0 * rng.next() * rng.next();
                    let along = at + 2.0 * (rng.next() - 0.5);
                    let height = 5.5 + 3.5 * rng.next();
                    let width = height * (1.1 + 0.5 * rng.next());
                    let tint = 0.88 + 0.24 * rng.next() as f32;
                    // Some blossom pinker, some whiter.
                    let warm = 0.55 * (rng.next() as f32).powi(2);
                    let turn = rng.next();
                    // The shore is the pines' (below).
                    let shore = self.water_level.is_some() && side == self.water_side;
                    if missing || shore || !near_window(p.x, p.z) {
                        continue;
                    }
                    let (x, z) = beside(along, side, lateral);
                    if inside((x, z)) && self.can_plant(x, z, 9.0, 2.5, ground, farms) {
                        let at = rel(x, z);
                        plant(
                            out,
                            shadows,
                            SAKURA,
                            at,
                            ground,
                            (x, z),
                            height,
                            width,
                            turn,
                            tint,
                            warm,
                        );
                    }
                }
                at += 7.0 + 8.0 * rng.next();
            }
            d = at + step;
        }
        // The shore pines, on the water's side of a lake road.
        if self.water_level.is_none() {
            return;
        }
        let side = self.water_side;
        let mut rng = Rng(hash2(1, 0, s + 507));
        let mut d = 0.0;
        while d < route.span() {
            let p = route.at(d);
            let step = 25.0 + 60.0 * rng.next();
            if rng.next() > 0.45 {
                d += step;
                continue;
            }
            let trees = 1 + (rng.next() * 3.5) as usize;
            let mut at = d;
            for _ in 0..trees {
                let lateral = 9.6 + 3.0 * rng.next();
                let height = 5.0 + 5.0 * rng.next();
                let width = height * (0.85 + 0.5 * rng.next());
                let tint = 0.85 + 0.3 * rng.next() as f32;
                let turn = rng.next();
                if near_window(p.x, p.z) {
                    let (x, z) = beside(at, side, lateral);
                    if inside((x, z)) && self.can_plant(x, z, 9.0, 2.5, ground, farms) {
                        let at = rel(x, z);
                        plant(
                            out,
                            shadows,
                            KUROMATSU,
                            at,
                            ground,
                            (x, z),
                            height,
                            width,
                            turn,
                            tint,
                            0.0,
                        );
                    }
                }
                at += 4.0 + 8.0 * rng.next();
            }
            d = at + step;
        }
    }

    /// Tea fields: here and there on a gentle slope near the road, a field
    /// of clipped rows following the contour, 1.9 m apart, laid end to end.
    fn tea_fields(
        &self,
        (x0, z0, size): (f64, f64, f64),
        ground: &dyn Fn(f64, f64) -> f64,
        rel: &dyn Fn(f64, f64) -> (f32, f32),
        farms: &[&Building],
        out: &mut Vec<f32>,
    ) {
        let s = self.seed;
        let estate = 150.0;
        let (ei0, ej0) = (
            (x0 / estate).floor() as i64 - 1,
            (z0 / estate).floor() as i64 - 1,
        );
        let (ei1, ej1) = (
            ((x0 + size) / estate).floor() as i64 + 1,
            ((z0 + size) / estate).floor() as i64 + 1,
        );
        for ej in ej0..=ej1 {
            for ei in ei0..=ei1 {
                let mut rng = Rng(hash2(ei, ej, s + 777));
                if rng.next() > 0.3 {
                    continue;
                }
                let cx = (ei as f64 + 0.25 + 0.5 * rng.next()) * estate;
                let cz = (ej as f64 + 0.25 + 0.5 * rng.next()) * estate;
                let smp = self.sample(cx, cz);
                if smp.water || smp.dist < 30.0 || smp.dist > 170.0 || smp.forest > 0.3 {
                    continue;
                }
                let e = 3.0;
                let gx = (self.sample(cx + e, cz).h - self.sample(cx - e, cz).h) / (2.0 * e);
                let gz = (self.sample(cx, cz + e).h - self.sample(cx, cz - e).h) / (2.0 * e);
                let slope = gx.hypot(gz);
                if !(0.03..=0.16).contains(&slope) {
                    continue;
                }
                // Rows along the contour (`u`), one after another up the
                // slope (`v`).
                let (ux, uz) = (-gz / slope, gx / slope);
                let (vx, vz) = (gx / slope, gz / slope);
                let rot = uz.atan2(ux) as f32;
                let (ra, rb) = (38.0 + 14.0 * rng.next(), 26.0 + 10.0 * rng.next());
                let rows = (rb / 1.9) as i64;
                let lengths = (ra / 4.05) as i64;
                for r in -rows..=rows {
                    for l in -lengths..=lengths {
                        let (u, v) = (l as f64 * 4.05, r as f64 * 1.9);
                        // An oval field with a ragged edge.
                        let edge = 1.0 + 0.12 * (hash2(r, l, s + 779) % 100) as f64 / 100.0;
                        if (u / ra).powi(2) + (v / rb).powi(2) > edge {
                            continue;
                        }
                        let (x, z) = (cx + ux * u + vx * v, cz + uz * u + vz * v);
                        if x < x0 || x > x0 + size || z < z0 || z > z0 + size {
                            continue;
                        }
                        let here = self.sample(x, z);
                        if here.water
                            || here.dist < 12.0
                            || here.forest > 0.35
                            || self.on_paddies(x, z, &here, ground)
                        {
                            continue;
                        }
                        if !self.room_to_stand(x, z, 2.2, ground)
                            || farms.iter().any(|b| {
                                (x - b.x).powi(2) + (z - b.z).powi(2) < (b.reach + 3.0).powi(2)
                            })
                        {
                            continue;
                        }
                        let (rx, rz) = rel(x, z);
                        let tone = 0.9 + 0.2 * (hash2(l, r, s + 781) % 100) as f32 / 100.0;
                        out.extend_from_slice(&[
                            rx,
                            lowest(ground, x, z, 2.0) as f32 - 0.05,
                            rz,
                            rot,
                            1.0,
                            1.0,
                            tone,
                            tone,
                            tone,
                        ]);
                    }
                }
            }
        }
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
        let japan = self.route.scenery.spec().planting == Planting::Japanese;
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
                if japan
                    && self.japanese_woods(
                        (gi, gj),
                        (x, z),
                        &smp,
                        ground,
                        &rel,
                        &farms,
                        &mut out,
                        &mut shadows,
                    )
                {
                    continue;
                }
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
                    // Japan's open land is left open: its trees are planted
                    // (`japanese_woods`, `japanese_roadside`).
                    Some(Cover::Tree) if japan => {}
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
        if japan {
            let window = (x0, z0, size);
            self.japanese_roadside(window, ground, &rel, &farms, &mut out, &mut shadows);
            self.tea_fields(window, ground, &rel, &farms, &mut out[TEA]);
        }
        for b in farms {
            let (rx, rz) = rel(b.x, b.z);
            let gh = ground(b.x, b.z) as f32;
            // Garden trees stand on the ground as drawn, like the rest.
            let h = if is(b.kind, Group::Tree) {
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
    /// The mountains beyond its edge (`mountains::build`), from the same
    /// centre.
    pub mountains: Vec<f32>,
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
/// The land's snow line, above the route's highest point (alpine).
const SNOW_ABOVE: f64 = 150.0;
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
    use crate::route::Scenery;

    /// Nothing stands on the rice paddies: no tree, bush, rock, tea row
    /// or building anywhere the ground is painted as a paddy (the land
    /// shader's mask: flat, open, back from the road, in a paddy field).
    #[test]
    fn nothing_stands_on_the_rice_paddies() {
        let r = route::builtin()
            .into_iter()
            .find(|r| r.scenery == Scenery::Japan)
            .unwrap();
        let g = RouteGeo::new(&r);
        let mut checked = 0;
        for at in [800.0, 4200.0, 9000.0, 12500.0] {
            let p = r.at(at);
            let w = g.build_window(p.x, p.z, WINDOW_SIZE, WINDOW_VERTS);
            let ground = |x: f64, z: f64| w.ground(x, z).unwrap_or_else(|| g.sample(x, z).h);
            for (kind, list) in w.instances.iter().enumerate() {
                for i in list.chunks(INSTANCE_STRIDE) {
                    let (x, z) = (i[0] as f64 + w.ox, w.oz - i[2] as f64);
                    let smp = g.sample(x, z);
                    let e = 2.0;
                    let slope = (ground(x + e, z) - ground(x - e, z))
                        .hypot(ground(x, z + e) - ground(x, z - e))
                        / (2.0 * e);
                    let f = Field::at(x, z);
                    // Where the shader paints paddy at all.
                    let painted = slope < 0.395
                        && smp.forest < 0.35
                        && smp.dist > 28.0
                        && f.crop < 0.74
                        && f.edge > 1.5;
                    assert!(!painted, "kind {kind} on the paddies at ({x:.0}, {z:.0})");
                    checked += 1;
                }
            }
        }
        assert!(checked > 1000, "{checked} things checked");
    }

    /// Two windows that overlap hold the same trees and objects where they
    /// overlap: what stands at a spot does not hang on which window was
    /// built, so nothing appears, vanishes or moves as the land moves on.
    #[test]
    fn overlapping_windows_hold_the_same_things() {
        let r = route::builtin()
            .into_iter()
            .find(|r| r.scenery == Scenery::Japan)
            .unwrap();
        let g = RouteGeo::new(&r);
        let (a, b) = (r.at(1000.0), r.at(1700.0));
        let wa = g.build_window(a.x, a.z, WINDOW_SIZE, WINDOW_VERTS);
        let wb = g.build_window(b.x, b.z, WINDOW_SIZE, WINDOW_VERTS);
        // Well inside both windows.
        let half = WINDOW_SIZE / 2.0 - 60.0;
        let inside =
            |x: f64, z: f64, w: &TerrainData| (x - w.ox).abs() < half && (w.oz - z).abs() < half;
        // Kind, world position and height; positions are kept relative to
        // each window's middle in single precision, so allow a fifth of a
        // metre.
        let things = |w: &TerrainData| {
            let mut v = Vec::new();
            for (kind, list) in w.instances.iter().enumerate() {
                for i in list.chunks(INSTANCE_STRIDE) {
                    let (x, z) = (i[0] as f64 + w.ox, w.oz - i[2] as f64);
                    if inside(x, z, &wa) && inside(x, z, &wb) {
                        v.push((kind, x, z, i[5]));
                    }
                }
            }
            v
        };
        let (ta, tb) = (things(&wa), things(&wb));
        assert!(ta.iter().any(|t| t.0 == SAKURA), "cherries in the overlap");
        assert_eq!(ta.len(), tb.len(), "as many things in each");
        for t in &ta {
            assert!(
                tb.iter().any(|u| u.0 == t.0
                    && (u.1 - t.1).abs() < 0.2
                    && (u.2 - t.2).abs() < 0.2
                    && (u.3 - t.3).abs() < 0.01),
                "kind {} at ({:.1}, {:.1}) is in one window only",
                t.0,
                t.1,
                t.2
            );
        }
    }

    /// Each kind's number names its own entry in `OBJECTS`: the model there
    /// is the kind's.
    #[test]
    fn object_numbers_match_their_entries() {
        use super::super::{buildings, fields, japanese, mesh};
        type Model = fn() -> mesh::Mesh;
        let kinds: [(usize, Model, Group); KINDS] = [
            (PINE, mesh::pine, Group::Tree),
            (BROADLEAF, mesh::broadleaf, Group::Tree),
            (ROCK, mesh::rock, Group::Still),
            (HOUSE, buildings::farmhouse, Group::Building),
            (BIRCH, mesh::birch, Group::Tree),
            (BUSH, mesh::bush, Group::Bush),
            (BARN, buildings::barn, Group::Building),
            (SHED, buildings::shed, Group::Building),
            (TOWNHOUSE, buildings::townhouse, Group::Building),
            (OAK, mesh::oak, Group::Tree),
            (MAPLE, mesh::maple, Group::Tree),
            (BALE, fields::hay_bale, Group::Still),
            (WALL, fields::stone_wall, Group::Still),
            (SAKURA, japanese::sakura, Group::Tree),
            (MOMIJI, japanese::momiji, Group::Tree),
            (SUGI, japanese::sugi, Group::Tree),
            (KUROMATSU, japanese::kuromatsu, Group::Tree),
            (MINKA, japanese::minka, Group::Building),
            (TORII, japanese::torii, Group::Building),
            (TORO, japanese::toro, Group::Building),
            (PAGODA, japanese::pagoda, Group::Building),
            (TEA, japanese::tea_row, Group::Still),
        ];
        for (k, model, group) in kinds {
            assert!(
                (OBJECTS[k].model)().v == model().v,
                "kind {k}: another model"
            );
            assert!(OBJECTS[k].group == group, "kind {k}: another group");
        }
    }

    /// The distant land rises into the mountains with no cliffs: once a
    /// step of 300 m stood 1.5 km from the start of a climb, where the land
    /// stopped following the nearest road all at once.
    #[test]
    fn distant_land_has_no_cliffs() {
        let xml = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/data/alpe_d_huez.gpx"
        ))
        .unwrap();
        let mut g = route::parse_gpx(&xml, "alpe", "Alpe d'Huez").unwrap();
        g.scenery = Some(Scenery::Alpine);
        let alpe = g.to_route();
        for (r, km) in [(&alpe, 2.5), (&alpe, 12.0), (&route::builtin()[0], 3.0)] {
            let geo = RouteGeo::new(r);
            let p = r.at(km * 1000.0);
            let far = geo.build_far(p.x, p.z);
            let n = FAR_VERTS;
            let cell = FAR_SIZE / (n - 1) as f64;
            let h = |i: usize, j: usize| far.verts[(j * n + i) * TERRAIN_STRIDE + 1] as f64;
            let mut steepest: f64 = 0.0;
            for j in 1..n - 1 {
                for i in 1..n - 1 {
                    let g =
                        (h(i + 1, j) - h(i - 1, j)).hypot(h(i, j + 1) - h(i, j - 1)) / (2.0 * cell);
                    steepest = steepest.max(g);
                }
            }
            println!(
                "{} at {km} km: steepest {:.0}°",
                r.id,
                steepest.atan().to_degrees()
            );
            assert!(steepest < 2.8, "{} at {km} km: {steepest:.2}", r.id);
        }
    }

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
        // Both come in about a second, but on a busy machine (a CI runner
        // building land in many tests at once) much later: this checks they
        // come, not how fast.
        let until = std::time::Instant::now() + std::time::Duration::from_secs(120);
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
            // Each kind of countryside has its own buildings.
            let theirs: &[usize] = match r.scenery.spec().buildings {
                Buildings::Japanese => &[MINKA, TORII, PAGODA],
                _ => &[BARN],
            };
            for &kind in theirs {
                assert!(
                    farms.iter().any(|b| b.kind == kind),
                    "{}: no kind {kind}",
                    r.id
                );
            }
            // The buildings (not the trees in their gardens).
            for b in farms.iter().filter(|b| is(b.kind, Group::Building)) {
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
        // (Buildings stand on levelled ground of their own.)
        let kinds = (0..KINDS).filter(|&k| !is(k, Group::Building)).map(|k| {
            let (sunk, foot): (f64, Foot) = match k {
                _ if is(k, Group::Tree) => (0.2, |i| TREE_FOOT * i[4] as f64),
                BUSH => (0.15, |i| BUSH_FOOT * i[4] as f64),
                ROCK => (0.0, |i| ROCK_FOOT * i[4] as f64),
                BALE => (0.06, |_| BALE_FOOT),
                WALL => (0.03, |_| 0.4),
                TEA => (0.0, |_| 2.0),
                _ => panic!("kind {k}: say here how it stands on the ground"),
            };
            (k, sunk, foot)
        });
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
                // Not buildings (on levelled ground) or walls (following
                // the road, whatever the slope).
                for kind in (0..KINDS).filter(|&k| !is(k, Group::Building) && k != WALL) {
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
                        shape: crate::route::Shape::Loop,
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
/// how far an import strays from its track. A real climb with 21 hairpins
/// (`tests/data/alpe_d_huez.gpx`) is always included; `VELOFORGE_SURVEY_GPX`
/// adds another GPX file.
/// `cargo test --release survey_all_kinds -- --ignored --nocapture`
#[cfg(test)]
mod survey_many {
    use super::*;
    use crate::route;
    use crate::route::Scenery;

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

    /// A shaded relief map of the land around Alpe d'Huez and the first
    /// built-in route of each scenery, to see the land and mountains whole:
    /// `VELOFORGE_MAP_OUT=<dir>` writes one `.ppm` for each (24 km across,
    /// north up, the road in red, snow white).
    #[test]
    #[ignore]
    fn relief_map() {
        let out = std::env::var("VELOFORGE_MAP_OUT").expect("VELOFORGE_MAP_OUT");
        let xml = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/data/alpe_d_huez.gpx"
        ))
        .unwrap();
        let mut g = route::parse_gpx(&xml, "alpe", "Alpe d'Huez").unwrap();
        g.scenery = Some(Scenery::Alpine);
        let mut routes = vec![("alpe".to_string(), g.to_route())];
        for s in Scenery::ALL {
            if let Some(r) = route::builtin().into_iter().find(|r| r.scenery == s) {
                routes.push((r.id.clone(), r));
            }
        }
        for (name, r) in routes {
            let geo = RouteGeo::new(&r);
            let pts = r.points();
            let mut cx = pts.iter().map(|p| p.x).sum::<f64>() / pts.len() as f64;
            let mut cz = pts.iter().map(|p| p.z).sum::<f64>() / pts.len() as f64;
            let mut size = 24_000.0;
            // `VELOFORGE_MAP_AT=<km>,<size m>`: closer, around that point.
            if let Some((km, s)) = std::env::var("VELOFORGE_MAP_AT").ok().and_then(|v| {
                let (a, b) = v.split_once(',')?;
                Some((a.parse::<f64>().ok()?, b.parse::<f64>().ok()?))
            }) {
                let p = r.at(km * 1000.0);
                (cx, cz, size) = (p.x, p.z, s);
            }
            let n = 600usize;
            let cell = size / n as f64;
            let mut h = vec![0.0; n * n];
            let mut water = vec![false; n * n];
            for j in 0..n {
                for i in 0..n {
                    let x = cx - size / 2.0 + i as f64 * cell;
                    let z = cz + size / 2.0 - j as f64 * cell;
                    let s = geo.land(x, z);
                    h[j * n + i] = s.h;
                    water[j * n + i] = s.water;
                }
            }
            let (lo, hi) = h
                .iter()
                .fold((f64::MAX, f64::MIN), |a, &v| (a.0.min(v), a.1.max(v)));
            let mut img = format!("P6 {n} {n} 255\n").into_bytes();
            for j in 0..n {
                for i in 0..n {
                    let at = |i: usize, j: usize| h[j.min(n - 1) * n + i.min(n - 1)];
                    let (dx, dz) = (at(i + 1, j) - at(i, j), at(i, j + 1) - at(i, j));
                    // Light from the north-west.
                    let shade = (0.75 - (dx - dz) / cell * 0.9).clamp(0.15, 1.0);
                    let t = (h[j * n + i] - lo) / (hi - lo).max(1.0);
                    let mut c = [0.35 + 0.5 * t, 0.55 + 0.3 * t, 0.3 + 0.5 * t];
                    if h[j * n + i] > geo.snow_level() {
                        c = [0.95, 0.95, 1.0];
                    }
                    if water[j * n + i] {
                        c = [0.2, 0.35, 0.6];
                    }
                    img.extend(
                        c.iter()
                            .map(|v| (v * shade * 255.0).clamp(0.0, 255.0) as u8),
                    );
                }
            }
            for p in pts.iter().step_by(2) {
                let i = ((p.x - cx + size / 2.0) / cell) as usize;
                let j = ((cz + size / 2.0 - p.z) / cell) as usize;
                if i < n && j < n {
                    let k = 15 + 3 * (j * n + i);
                    img[k..k + 3].copy_from_slice(&[230, 20, 20]);
                }
            }
            std::fs::write(format!("{out}/{name}.ppm"), img).unwrap();
            println!(
                "{name}: {lo:.0} to {hi:.0} m, snow from {:.0} m",
                geo.snow_level()
            );
        }
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
        let mut files = vec![(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/alpe_d_huez.gpx").to_string(),
            "Alpe d'Huez".to_string(),
        )];
        if let Ok(path) = std::env::var("VELOFORGE_SURVEY_GPX") {
            files.push((path, "Your GPX file".into()));
        }
        for (path, name) in files {
            let xml = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
            let mut g = route::parse_gpx(&xml, "survey", &name).unwrap();
            g.scenery = Some(Scenery::Alpine);
            g.name = name;
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
