//! The mountains: ranges that rise out of the land from a few kilometres
//! beyond the roads, and the mesh that carries the land on past the
//! distant land's edge to the horizon.
//!
//! The ranges are part of the land itself (`terrain.rs` mixes `height` in
//! from `START` to `FULL` metres from the nearest road), so the distant land
//! rises into their foothills, and the mesh here starts exactly on that
//! land's edge. Heights are in the world, not around the camera: riding on,
//! the ranges stay where they are, and climbing, they sink as they should.
//!
//! No GL here, so the Model Lab can build the same mesh.

/// What kind of mountains (the route's scenery).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Rolling wooded hills.
    Hills,
    /// Pre-Alps around a lake, snow only on the highest tops.
    Lake,
    /// High ranges: forest, rock and snow.
    Alpine,
    /// Islands and headlands across the sea.
    Coast,
    /// Green, rounded mountains, and one great volcano rising alone above
    /// them, snow on its upper slopes.
    Japan,
}

/// The mountains around one route.
#[derive(Debug, Clone, Copy)]
pub struct Params {
    pub kind: Kind,
    pub seed: u64,
    /// The route's lowest and highest points (m).
    pub min_ele: f64,
    pub max_ele: f64,
    /// Lake or sea level.
    pub water: Option<f64>,
    /// The land's snow line (alpine).
    pub snow: f64,
    /// Where a lone volcano stands (world x, z), if one does.
    pub volcano: Option<[f64; 2]>,
}

/// The volcano: how high its summit stands above the valley floors, and
/// how far from it (m) its slopes reach.
pub const VOLCANO_HEIGHT: f64 = 2000.0;
pub const VOLCANO_REACH: f64 = 7500.0;

/// From this far from the nearest road the land starts to rise into the
/// mountains, and by `FULL` it is all mountain.
pub const START: f64 = 1000.0;
pub const FULL: f64 = 3500.0;

/// How much of the land at `dist` metres from the nearest road is mountain.
pub fn share(dist: f64) -> f64 {
    smoothstep(START, FULL, dist)
}

impl Params {
    /// The valley floors between the ranges.
    pub fn floor(&self) -> f64 {
        match self.kind {
            Kind::Alpine => self.min_ele - 80.0,
            Kind::Hills => self.min_ele - 30.0,
            Kind::Lake | Kind::Japan => self.water.unwrap_or(self.min_ele) - 15.0,
            // Open sea between the islands.
            Kind::Coast => self.water.unwrap_or(self.min_ele) - 90.0,
        }
    }

    /// How high the highest peaks stand above the valley floors.
    pub fn relief(&self) -> f64 {
        let climb = (self.max_ele - self.min_ele).max(0.0);
        match self.kind {
            Kind::Alpine => 1800.0 + climb,
            Kind::Lake => 1300.0 + 0.8 * climb,
            Kind::Hills => 380.0 + 0.6 * climb,
            Kind::Coast => 420.0 + 0.6 * climb,
            Kind::Japan => 900.0 + 0.6 * climb,
        }
    }

    /// Where the forest stops.
    pub fn treeline(&self) -> f64 {
        match self.kind {
            // As on the land by the road.
            Kind::Alpine => self.max_ele + 90.0,
            Kind::Lake => self.floor() + 0.5 * self.relief(),
            Kind::Hills => 1.0e9,
            Kind::Coast => self.floor() + 0.55 * self.relief(),
            // Bare volcanic rock above it on the volcano.
            Kind::Japan => self.floor() + 900.0,
        }
    }

    /// Where snow starts (on gentle slopes; steep rock sheds it).
    pub fn snowline(&self) -> f64 {
        match self.kind {
            // As on the land by the road.
            Kind::Alpine => self.snow,
            // Only the highest tops.
            Kind::Lake => self.floor() + 0.8 * self.relief(),
            Kind::Hills | Kind::Coast => 1.0e9,
            // The volcano's upper half: above every other top.
            Kind::Japan => self.floor() + 0.55 * VOLCANO_HEIGHT,
        }
    }
}

/// The mountains' own height at world (`x`, `z`), `dist` metres from the
/// nearest road: a few great massifs with lower shoulders, cols and valleys
/// between, ridges branching down from the peaks, rising from the valley
/// floors (`Params::floor`), and higher the further they are from the
/// roads (the land by a road follows the road, so the ranges stand back
/// from it).
pub fn height(p: &Params, x: f64, z: f64, dist: f64) -> f64 {
    let s = p.seed.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ 0x6d6f_756e;
    // Further from the roads, higher ranges.
    let env = 0.45 + 0.55 * smoothstep(2000.0, 9000.0, dist);
    // Massifs and the broad gaps between them.
    let mass = smoothstep(-0.32, 0.42, fbm(x / 14000.0, z / 14000.0, 3, s + 3));
    // Warp the ridges so they wander rather than run in grid lines.
    let wx = x + 750.0 * fbm(x / 4300.0, z / 4300.0, 3, s + 1);
    let wz = z + 750.0 * fbm(x / 4300.0 + 7.7, z / 4300.0 - 3.1, 3, s + 2);
    let shape = match p.kind {
        Kind::Alpine => ridged(wx / 7000.0, wz / 7000.0, 5, 0.45, s + 4) * 1.25,
        Kind::Lake => {
            0.9 * ridged(wx / 5500.0, wz / 5500.0, 5, 0.45, s + 4)
                + 0.3 * (0.5 + 0.5 * fbm(wx / 2000.0, wz / 2000.0, 4, s + 5))
        }
        Kind::Hills => {
            // Rounded tops.
            let b = 0.5 + 0.5 * fbm(wx / 2300.0, wz / 2300.0, 5, s + 4);
            1.0 - (1.0 - b) * (1.0 - b)
        }
        Kind::Coast => ridged(wx / 4500.0, wz / 4500.0, 5, 0.45, s + 4) * 1.15,
        Kind::Japan => {
            // Rounded, wooded tops, a little ridged.
            let b = 0.5 + 0.5 * fbm(wx / 2600.0, wz / 2600.0, 5, s + 4);
            0.75 * (1.0 - (1.0 - b) * (1.0 - b))
                + 0.35 * ridged(wx / 6000.0, wz / 6000.0, 4, 0.45, s + 6)
        }
    };
    let rise = match p.kind {
        // Islands: where the massif mask is low, open sea.
        Kind::Coast => mass * mass * shape * 1.2,
        // High ranges all round, the massifs higher still.
        Kind::Alpine => (0.35 + 0.65 * mass) * shape,
        _ => (0.16 + 0.84 * mass) * shape,
    };
    let land = p.floor() + p.relief() * env * rise;
    match p.volcano {
        Some(v) => land.max(p.floor() + volcano(x - v[0], z - v[1])),
        None => land,
    }
}

/// The volcano's height above the valley floors, (`dx`, `dz`) from its
/// summit: a lone cone whose slopes steepen gently towards the top, as
/// such volcanoes' do, with a shallow crater at its summit.
pub fn volcano(dx: f64, dz: f64) -> f64 {
    let r = dx.hypot(dz);
    if r >= VOLCANO_REACH {
        return 0.0;
    }
    let t = 1.0 - r / VOLCANO_REACH;
    let crater = 140.0 * (1.0 - smoothstep(0.0, 320.0, r));
    VOLCANO_HEIGHT * t.powf(1.35) - crater
}

/// How much forest grows on the mountains at (`x`, `z`), `h` high.
pub fn forest(p: &Params, x: f64, z: f64, h: f64) -> f64 {
    let s = p.seed ^ 0x666f_7265;
    // Mostly wooded below the tree line, with clearings and meadows.
    let patches = smoothstep(-0.45, 0.05, fbm(x / 1600.0, z / 1600.0, 3, s));
    let tl = p.treeline() + 120.0 * fbm(x / 700.0, z / 700.0, 2, s + 1);
    let below = 1.0 - smoothstep(tl - 90.0, tl + 60.0, h);
    match p.kind {
        Kind::Hills => 0.25 + 0.75 * patches,
        _ => (0.2 + 0.8 * patches) * below,
    }
}

// ---------------------------------------------------------------------------
// The mesh beyond the distant land

/// Rays out from the distant land's edge for each of its cells along it.
pub const RAYS_PER_CELL: usize = 2;
/// Rings of vertices along each ray, from the land's edge out.
pub const RINGS: usize = 48;
/// How far out the mesh reaches from the distant land's centre (inside the
/// far plane, 12 km, wherever the camera is in the land).
pub const OUTER: f64 = 10_500.0;
/// Floats per vertex: position, normal, forest, snow, rock, shade, and how
/// far beyond the distant land's edge.
pub const STRIDE: usize = 11;

/// Where the rays leave the distant land's edge, a square `half` metres
/// from its centre with `cells` cells along a side: offsets from the
/// centre (east, north), anticlockwise from the south-west corner.
pub fn seam(half: f64, cells: usize) -> Vec<[f64; 2]> {
    let per_side = cells * RAYS_PER_CELL;
    let mut out = Vec::with_capacity(4 * per_side);
    for side in 0..4 {
        for i in 0..per_side {
            let t = -half + 2.0 * half * i as f64 / per_side as f64;
            out.push(match side {
                0 => [t, -half],
                1 => [half, t],
                2 => [-t, half],
                _ => [-half, -t],
            });
        }
    }
    out
}

/// How far along a ray ring `k` lies, from the edge (0) to `OUTER` (1):
/// closer together near the land, where they are seen larger.
fn ring_t(k: usize) -> f64 {
    let a = 2.2;
    ((a * k as f64 / (RINGS - 1) as f64).exp() - 1.0) / (a.exp() - 1.0)
}

/// Every vertex's place, ring by ring from the edge out: offsets from the
/// distant land's centre (east, north).
pub fn points(seam: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut out = Vec::with_capacity(seam.len() * RINGS);
    for k in 0..RINGS {
        let t = ring_t(k);
        for s in seam {
            let q = s[0].hypot(s[1]);
            let r = q + (OUTER - q) * t;
            out.push([s[0] / q * r, s[1] / q * r]);
        }
    }
    out
}

/// The land at one vertex: height, and how much of it is forest.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ground {
    pub h: f64,
    pub forest: f64,
}

/// The mesh from the land at every point of `points` (the first ring is the
/// distant land's own edge, `edge_normals` its normals there, render
/// coordinates): vertices (`STRIDE` floats, render coordinates from the
/// distant land's centre: x east, y up, z south).
pub fn build(p: &Params, pts: &[[f64; 2]], land: &[Ground], edge_normals: &[[f32; 3]]) -> Vec<f32> {
    let n = edge_normals.len();
    assert_eq!(pts.len(), n * RINGS);
    assert_eq!(land.len(), pts.len());
    let at = |j: usize, k: usize| -> [f64; 3] {
        let i = k * n + j % n;
        [pts[i][0], land[i].h, pts[i][1]]
    };
    let snowline = p.snowline();
    let treeline = p.treeline();
    let alpine = p.kind == Kind::Alpine;
    let mut v = Vec::with_capacity(pts.len() * STRIDE);
    for k in 0..RINGS {
        for j in 0..n {
            let i = k * n + j;
            let c = at(j, k);
            let (prev, next) = (at(j + n - 1, k), at(j + 1, k));
            let (inn, out) = (at(j, k.saturating_sub(1)), at(j, (k + 1).min(RINGS - 1)));
            // World normal (y up, z north) from the neighbours.
            let normal = if k == 0 {
                let e = edge_normals[j];
                [e[0] as f64, e[1] as f64, -e[2] as f64]
            } else {
                let a = sub(next, prev);
                let b = sub(out, inn);
                let mut nn = cross(b, a);
                if nn[1] < 0.0 {
                    nn = [-nn[0], -nn[1], -nn[2]];
                }
                normalise(nn)
            };
            // How far the point stands above (ridge) or sinks below (gully)
            // its neighbours, for its spacing.
            let spacing = (dist(prev, next) + dist(inn, out).max(1.0)) * 0.25;
            let around = (prev[1] + next[1] + inn[1] + out[1]) * 0.25;
            let bend = if k == 0 || k == RINGS - 1 {
                0.0
            } else {
                ((c[1] - around) / spacing).clamp(-1.0, 1.0)
            };
            let gully = (-bend * 5.0).clamp(0.0, 1.0);
            let ridge = (bend * 5.0).clamp(0.0, 1.0);
            let up = normal[1];
            let steep = smoothstep(0.8, 0.5, up);
            let h = c[1];
            // Snow on the upper slopes, lower in the gullies, thin on the
            // ridges, and off the steep rock.
            let snow = if snowline < 1.0e8 {
                let above = h - snowline + gully * 140.0 - ridge * 50.0;
                // (Half at the line, where the shader's edge is: as on the
                // land, but fading in over a few hundred metres.)
                smoothstep(-200.0, 200.0, above) * (1.0 - 0.9 * steep)
            } else {
                0.0
            };
            // Bare rock on steep faces, and above the trees in the high
            // ranges.
            let rock = if alpine {
                steep.max(0.85 * smoothstep(treeline + 80.0, treeline + 520.0, h))
            } else {
                steep * 0.9
            };
            let forest = land[i].forest
                * (1.0 - steep)
                * (1.0 - smoothstep(treeline - 60.0, treeline + 60.0, h));
            let shade = 1.0 - 0.4 * gully;
            let beyond = c[0].hypot(c[2]) - pts[j][0].hypot(pts[j][1]);
            v.extend_from_slice(&[
                c[0] as f32,
                h as f32,
                -c[2] as f32,
                normal[0] as f32,
                normal[1] as f32,
                -normal[2] as f32,
                forest as f32,
                snow as f32,
                rock as f32,
                shade as f32,
                beyond.max(0.0) as f32,
            ]);
        }
    }
    v
}

/// Sectors of the mesh round the circle, each drawn only when in view.
pub const SECTORS: usize = 32;

/// Triangles for `rays` rays (the same for every mesh): sector by sector,
/// each nearest first.
pub fn indices(rays: usize) -> Vec<u32> {
    let mut out = Vec::with_capacity(rays * (RINGS - 1) * 6);
    let per = rays / SECTORS;
    for sector in 0..SECTORS {
        for k in 0..RINGS - 1 {
            for j in sector * per..(sector + 1) * per {
                let a = (k * rays + j) as u32;
                let b = (k * rays + (j + 1) % rays) as u32;
                let (c, d) = (a + rays as u32, b + rays as u32);
                out.extend_from_slice(&[a, b, c, b, d, c]);
            }
        }
    }
    out
}

/// Each sector's run of `indices` (first index, how many) and the box
/// round its vertices in `verts` (render coordinates, from the distant
/// land's centre): to leave out those out of view.
#[allow(clippy::type_complexity)]
pub fn sectors(verts: &[f32], rays: usize) -> Vec<(u32, u32, [f32; 3], [f32; 3])> {
    let per = rays / SECTORS;
    let count = (per * (RINGS - 1) * 6) as u32;
    (0..SECTORS)
        .map(|sector| {
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for k in 0..RINGS {
                for j in sector * per..=(sector + 1) * per {
                    let v = &verts[(k * rays + j % rays) * STRIDE..][..3];
                    for c in 0..3 {
                        lo[c] = lo[c].min(v[c]);
                        hi[c] = hi[c].max(v[c]);
                    }
                }
            }
            (sector as u32 * count, count, lo, hi)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Noise

fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalise(a: [f64; 3]) -> [f64; 3] {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-12);
    [a[0] / l, a[1] / l, a[2] / l]
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = sub(a, b);
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

fn hash2(x: i64, z: i64, seed: u64) -> u64 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (z as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ seed.wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^ (h >> 29)
}

/// 16 directions round the circle, 22.5 degrees apart (cos, sin).
const H: f64 = std::f64::consts::FRAC_1_SQRT_2;
const DIRS: [(f64, f64); 16] = [
    (1.0, 0.0),
    (0.923_879_5, 0.382_683_4),
    (H, H),
    (0.382_683_4, 0.923_879_5),
    (0.0, 1.0),
    (-0.382_683_4, 0.923_879_5),
    (-H, H),
    (-0.923_879_5, 0.382_683_4),
    (-1.0, 0.0),
    (-0.923_879_5, -0.382_683_4),
    (-H, -H),
    (-0.382_683_4, -0.923_879_5),
    (0.0, -1.0),
    (0.382_683_4, -0.923_879_5),
    (H, -H),
    (0.923_879_5, -0.382_683_4),
];

/// Gradient noise, about -1 to 1.
fn gradient(x: f64, z: f64, seed: u64) -> f64 {
    let (xi, zi) = (x.floor(), z.floor());
    let (fx, fz) = (x - xi, z - zi);
    let fade = |t: f64| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (ux, uz) = (fade(fx), fade(fz));
    // One of 16 directions round the circle at each corner (a table, not
    // a sine and cosine for each: the mountains take some 20 of these a
    // point, for 50,000 points each time the distant land moves on).
    let g = |dx: i64, dz: i64| {
        let (c, s) = DIRS[(hash2(xi as i64 + dx, zi as i64 + dz, seed) >> 60) as usize];
        c * (fx - dx as f64) + s * (fz - dz as f64)
    };
    let a = g(0, 0) + (g(1, 0) - g(0, 0)) * ux;
    let b = g(0, 1) + (g(1, 1) - g(0, 1)) * ux;
    (a + (b - a) * uz) * 1.41
}

fn fbm(x: f64, z: f64, octaves: u32, seed: u64) -> f64 {
    let (mut sum, mut amp, mut norm, mut f) = (0.0, 0.5, 0.0, 1.0);
    for i in 0..octaves {
        sum += amp
            * gradient(
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

/// Ridges, 0 to about 1: sharp crests where the noise crosses zero, each
/// finer octave weighted by the one before, so the crests are rugged and
/// the valleys between them smooth.
fn ridged(x: f64, z: f64, octaves: u32, gain: f64, seed: u64) -> f64 {
    let (mut sum, mut amp, mut norm, mut weight, mut f) = (0.0, 1.0, 0.0, 1.0, 1.0);
    for i in 0..octaves {
        let n = gradient(
            x * f + i as f64 * 31.7,
            z * f - i as f64 * 12.3,
            seed + i as u64,
        );
        let r = (1.0 - n.abs()).max(0.0);
        let r = r * r * weight;
        weight = (r * 1.8).clamp(0.0, 1.0);
        sum += r * amp;
        norm += amp;
        amp *= gain;
        f *= 2.05;
    }
    sum / norm
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpe() -> Params {
        Params {
            kind: Kind::Alpine,
            seed: 7,
            min_ele: 720.0,
            max_ele: 1830.0,
            water: None,
            snow: 1980.0,
            volcano: None,
        }
    }

    /// The volcano: a lone cone, highest at its summit but for a shallow
    /// crater, falling evenly away all round to nothing at its foot, with
    /// slopes steeper towards the top (as Fuji's).
    #[test]
    fn the_volcano_rises_to_one_summit_with_a_crater() {
        let rim = (0..=40)
            .map(|k| volcano(k as f64 * 20.0, 0.0))
            .fold(0.0, f64::max);
        assert!(volcano(0.0, 0.0) < rim, "a crater at the summit");
        assert!(rim > 0.92 * VOLCANO_HEIGHT);
        for a in [0.0f64, 1.3, 2.9, 4.4] {
            let at = |r: f64| volcano(a.cos() * r, a.sin() * r);
            assert!((at(3000.0) - volcano(3000.0, 0.0)).abs() < 1e-6, "round");
            let mut last = at(1000.0);
            for r in (1500..7500).step_by(500) {
                let h = at(r as f64);
                assert!(h < last, "falls away at {r} m");
                last = h;
            }
            assert_eq!(at(VOLCANO_REACH + 1.0), 0.0);
        }
        let steep = |r: f64| (volcano(r - 50.0, 0.0) - volcano(r + 50.0, 0.0)) / 100.0;
        assert!(steep(1500.0) > steep(6000.0), "steeper towards the top");
    }

    #[test]
    fn gradient_directions_go_round_the_circle() {
        for (i, &(c, s)) in DIRS.iter().enumerate() {
            let a = i as f64 * std::f64::consts::TAU / 16.0;
            assert!(
                (c - a.cos()).abs() < 1e-6 && (s - a.sin()).abs() < 1e-6,
                "{i}"
            );
        }
    }

    #[test]
    fn mesh_is_well_formed() {
        let seam = seam(3800.0, 128);
        let pts = points(&seam);
        assert_eq!(seam.len(), 4 * 128 * RAYS_PER_CELL);
        // Rings run outward along each ray, to `OUTER`.
        for j in 0..seam.len() {
            let r = |k: usize| pts[k * seam.len() + j][0].hypot(pts[k * seam.len() + j][1]);
            for k in 1..RINGS {
                assert!(r(k) > r(k - 1), "ray {j} ring {k}");
            }
            assert!((r(RINGS - 1) - OUTER).abs() < 1e-6);
        }
        let p = alpe();
        let land: Vec<Ground> = pts
            .iter()
            .map(|q| {
                let h = height(&p, q[0], q[1], 6000.0);
                Ground {
                    h,
                    forest: forest(&p, q[0], q[1], h),
                }
            })
            .collect();
        let v = build(&p, &pts, &land, &vec![[0.0, 1.0, 0.0]; seam.len()]);
        assert_eq!(v.len(), pts.len() * STRIDE);
        assert!(v.iter().all(|f| f.is_finite()));
        for c in v.chunks(STRIDE) {
            assert!(c[4] > 0.0, "normals face up");
            for w in &c[6..10] {
                assert!((0.0..=1.0).contains(w));
            }
        }
        let idx = indices(seam.len());
        assert!(idx.iter().all(|&i| (i as usize) < pts.len()));
        assert_eq!(idx.len(), seam.len() * (RINGS - 1) * 6);
        let sec = sectors(&v, seam.len());
        assert_eq!(sec.len(), SECTORS);
        let ring = |t: &[u32]| t.iter().min().unwrap() / seam.len() as u32;
        for (first, count, lo, hi) in sec {
            let run = &idx[first as usize..(first + count) as usize];
            // Nearest first within the sector: each triangle on the same
            // ring as the one before, or further out.
            assert!(run
                .chunks(3)
                .zip(run.chunks(3).skip(1))
                .all(|(a, b)| ring(a) <= ring(b)));
            // Its box holds all its triangles.
            for &i in run {
                let p = &v[i as usize * STRIDE..][..3];
                assert!((0..3).all(|c| p[c] >= lo[c] && p[c] <= hi[c]));
            }
        }
        // Every triangle in one sector or another, once.
        let mut all: Vec<[u32; 3]> = idx.chunks(3).map(|t| [t[0], t[1], t[2]]).collect();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), idx.len() / 3);
    }

    /// The high ranges: a few dominant peaks well above the rest, valleys
    /// between, and the highest tops above the snow.
    #[test]
    fn ranges_have_peaks_and_valleys() {
        for kind in [Kind::Alpine, Kind::Lake, Kind::Hills, Kind::Coast] {
            let p = Params { kind, ..alpe() };
            let mut hs: Vec<f64> = (0..20_000)
                .map(|i| {
                    let a = i as f64 * 0.000_314_159 * 2.0;
                    let r = 5000.0 + (i % 97) as f64 * 55.0;
                    height(&p, r * a.cos(), r * a.sin(), r)
                })
                .collect();
            // Slopes over 30 m, as the distant land sees them.
            let mut slopes: Vec<f64> = (0..4000)
                .map(|i| {
                    let a = i as f64 * 0.001_57;
                    let r = 6000.0 + (i % 89) as f64 * 40.0;
                    let (x, z) = (r * a.cos(), r * a.sin());
                    let h = |x: f64, z: f64| height(&p, x, z, r);
                    ((h(x + 30.0, z) - h(x, z)).powi(2) + (h(x, z + 30.0) - h(x, z)).powi(2)).sqrt()
                        / 30.0
                })
                .collect();
            slopes.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let sq = |f: f64| {
                slopes[((slopes.len() - 1) as f64 * f) as usize]
                    .atan()
                    .to_degrees()
            };
            println!(
                "{kind:?}: slopes 50% {:.0}°, 90% {:.0}°, 99% {:.0}°",
                sq(0.5),
                sq(0.9),
                sq(0.99)
            );
            hs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let q = |f: f64| hs[((hs.len() - 1) as f64 * f) as usize] - p.floor();
            println!(
                "{kind:?}: relief {:.0}, above the floor: 10% {:.0}, 50% {:.0}, 90% {:.0}, 99% {:.0}, top {:.0}",
                p.relief(),
                q(0.1),
                q(0.5),
                q(0.9),
                q(0.99),
                q(1.0)
            );
            // Rolling hills less so than mountains.
            let stand_out = if kind == Kind::Hills { 1.8 } else { 2.0 };
            assert!(q(0.99) > stand_out * q(0.5), "{kind:?}: peaks stand out");
            if kind == Kind::Alpine {
                // Mostly mountainsides, some cliffs (real alpine ground).
                assert!(sq(0.5) < 33.0 && sq(0.9) < 55.0, "{kind:?}: not too steep");
            }
            assert!(q(1.0) <= p.relief() * 1.2, "{kind:?}");
            if kind == Kind::Alpine {
                assert!(
                    p.floor() + q(0.99) > p.snowline() + 600.0,
                    "snow on the tops"
                );
                assert!(p.floor() + q(0.5) < p.snowline(), "not on everything");
            }
        }
    }
}
