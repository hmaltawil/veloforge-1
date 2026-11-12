//! World-space terrain around a route: a height function shaped by the
//! distance to the road (the road sits in a valley), forests, rocks,
//! bushes, houses and lakes. Windows of terrain are built on a worker thread.

use crate::route::{Route, Scenery, STEP};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// Floats per terrain vertex: position (3), normal (3), road distance, forest.
pub const TERRAIN_STRIDE: usize = 8;
/// Floats per object instance: position (3), rotation, scale (2), tint (3).
pub const INSTANCE_STRIDE: usize = 9;

/// Kinds of scattered object, indexing [`TerrainData::instances`].
pub const KINDS: usize = 6;
pub const PINE: usize = 0;
pub const BROADLEAF: usize = 1;
pub const ROCK: usize = 2;
pub const HOUSE: usize = 3;
pub const BIRCH: usize = 4;
pub const BUSH: usize = 5;

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
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
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
    looped: bool,
    /// Side (+1 right, -1 left) the lake or sea is on.
    water_side: f64,
    pub water_level: Option<f64>,
    pub max_ele: f64,
    seed: u64,
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
        RouteGeo {
            route: route.clone(),
            eles: route.points().iter().map(|p| p.ele).collect(),
            pts,
            grid,
            looped,
            water_side,
            water_level,
            max_ele,
            seed: route.seed,
        }
    }

    pub fn snow_level(&self) -> f64 {
        self.max_ele + 35.0
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
    fn base_height(&self, x: f64, z: f64, near: &Near) -> f64 {
        let exact = self.route.at(near.d).ele;
        if near.dist <= 9.0 {
            return exact;
        }
        let n = self.pts.len();
        let step = (n / 800).max(3);
        let (mut sw, mut se) = (0.0, 0.0);
        for j in (0..n).step_by(step) {
            let (px, pz) = self.pts[j];
            let d = ((px - x).powi(2) + (pz - z).powi(2)).sqrt();
            // The offset keeps the slope between two nearby roads gentle.
            let w = 1.0 / ((d - 9.0).max(0.0) + 20.0).powi(3);
            sw += w;
            se += w * self.eles[j];
        }
        exact + (se / sw - exact) * smoothstep(9.0, 30.0, near.dist)
    }

    pub fn sample(&self, x: f64, z: f64) -> Sample {
        let near = self.nearest(x, z);
        let road = self.base_height(x, z, &near);
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
        let h = flat + t * (hills + bumps);
        Sample {
            h,
            dist: near.dist,
            forest,
            water: false,
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
        let mut indices = Vec::with_capacity((n - 1) * (n - 1) * 6);
        for j in 0..n - 1 {
            for i in 0..n - 1 {
                let a = (j * n + i) as u32;
                let b = a + 1;
                let c = a + n as u32;
                let d = c + 1;
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        let (instances, shadows) = self.scatter(ox, oz, x0, z0, size);
        TerrainData {
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
    fn scatter(
        &self,
        ox: f64,
        oz: f64,
        x0: f64,
        z0: f64,
        size: f64,
    ) -> ([Vec<f32>; KINDS], Vec<f32>) {
        let mut out: [Vec<f32>; KINDS] = Default::default();
        let mut shadows = Vec::new();
        let s = self.seed;
        let rel = |x: f64, z: f64| ((x - ox) as f32, (-(z - oz)) as f32);
        let cell = 11.0;
        let (gi0, gj0) = ((x0 / cell).floor() as i64, (z0 / cell).floor() as i64);
        let cells = (size / cell).ceil() as i64;
        let pine_share = match self.route.scenery {
            Scenery::Alpine => 0.95,
            Scenery::Hills => 0.35,
            Scenery::Lake => 0.45,
            Scenery::Coast => 0.7,
        };
        let (birch_share, bush_chance) = match self.route.scenery {
            Scenery::Alpine => (0.3, 0.02),
            Scenery::Hills => (0.3, 0.05),
            Scenery::Lake => (0.4, 0.05),
            Scenery::Coast => (0.15, 0.07),
        };
        for gj in gj0..gj0 + cells {
            for gi in gi0..gi0 + cells {
                let mut rng = Rng(hash2(gi, gj, s + 77));
                let x = (gi as f64 + rng.next()) * cell;
                let z = (gj as f64 + rng.next()) * cell;
                let smp = self.sample(x, z);
                if smp.water || smp.dist < 9.5 {
                    continue;
                }
                let r = rng.next();
                let chance = smp.forest * 0.85 + 0.025;
                let (rx, rz) = rel(x, z);
                if r < chance {
                    let pine = rng.next() < pine_share;
                    // Birches stand among the broadleaves, mostly at the forest's edge.
                    let birch = !pine && rng.next() < birch_share * (1.3 - smp.forest);
                    let tall = 0.7 + 0.6 * rng.next() + smp.forest * 0.3;
                    let height = if pine {
                        14.0 * tall
                    } else if birch {
                        11.0 * tall
                    } else {
                        10.5 * tall
                    };
                    let width = height
                        * (if pine {
                            0.72
                        } else if birch {
                            0.7
                        } else {
                            1.0
                        })
                        * (0.85 + 0.3 * rng.next());
                    // Each tree has its own shade of green, some a little yellower.
                    let tint = 0.78 + 0.44 * rng.next() as f32;
                    let warm = (rng.next() as f32).powi(2) * if pine { 0.25 } else { 0.7 };
                    let rot = rng.next() * std::f64::consts::TAU;
                    let kind = if pine {
                        PINE
                    } else if birch {
                        BIRCH
                    } else {
                        BROADLEAF
                    };
                    out[kind].extend_from_slice(&[
                        rx,
                        smp.h as f32 - 0.2,
                        rz,
                        rot as f32,
                        width as f32,
                        height as f32,
                        tint * (0.95 + 0.4 * warm),
                        tint * (1.0 + 0.08 * warm),
                        tint * (0.9 - 0.3 * warm),
                    ]);
                    shadows.extend_from_slice(&[rx, smp.h as f32 + 0.08, rz, width as f32 * 0.5]);
                } else if r < chance + bush_chance * (0.4 + smp.forest) && smp.dist > 12.0 {
                    let height = 1.2 + 1.8 * rng.next();
                    let width = height * (1.2 + 0.8 * rng.next());
                    let tint = 0.8 + 0.4 * rng.next() as f32;
                    let rot = rng.next() * std::f64::consts::TAU;
                    out[BUSH].extend_from_slice(&[
                        rx,
                        smp.h as f32 - 0.15,
                        rz,
                        rot as f32,
                        width as f32,
                        height as f32,
                        tint,
                        tint,
                        tint * 0.9,
                    ]);
                    shadows.extend_from_slice(&[rx, smp.h as f32 + 0.08, rz, width as f32 * 0.55]);
                } else if r < chance
                    + bush_chance * 1.4
                    + if self.route.scenery == Scenery::Alpine {
                        0.05
                    } else {
                        0.006
                    }
                {
                    let size = 1.0 + 3.5 * rng.next().powi(2);
                    let g = 0.8 + 0.3 * rng.next() as f32;
                    let rot = rng.next() * std::f64::consts::TAU;
                    out[ROCK].extend_from_slice(&[
                        rx,
                        smp.h as f32 - size as f32 * 0.25,
                        rz,
                        rot as f32,
                        size as f32,
                        size as f32 * 0.7,
                        g,
                        g,
                        g,
                    ]);
                }
            }
        }
        // Farmhouses along the road.
        if self.route.scenery != Scenery::Alpine {
            let spacing = 320.0;
            let route = &self.route;
            let k_max = (route.length / spacing) as i64;
            for k in 0..k_max {
                let mut rng = Rng(hash2(k, 0, s + 991));
                if rng.next() > 0.55 {
                    continue;
                }
                let d = k as f64 * spacing + rng.next() * 100.0;
                let p = route.at(d);
                let side = if rng.next() < 0.5 { -1.0 } else { 1.0 };
                let lateral = side * (24.0 + 26.0 * rng.next());
                let x = p.x + lateral * p.heading.cos();
                let z = p.z - lateral * p.heading.sin();
                if x < x0 || x > x0 + size || z < z0 || z > z0 + size {
                    continue;
                }
                let smp = self.sample(x, z);
                if smp.water || smp.dist < 15.0 {
                    continue;
                }
                let (rx, rz) = rel(x, z);
                let length = 11.0 + 6.0 * rng.next();
                let roof = [[0.55, 0.22, 0.15], [0.35, 0.33, 0.32], [0.45, 0.28, 0.18]]
                    [(rng.next() * 3.0) as usize % 3];
                // Long side parallel to the road (heading is measured from north).
                let rot = (std::f64::consts::FRAC_PI_2 - p.heading) as f32;
                out[HOUSE].extend_from_slice(&[
                    rx,
                    smp.h as f32 - 0.3,
                    rz,
                    rot,
                    length as f32,
                    length as f32 * 0.75,
                    roof[0],
                    roof[1],
                    roof[2],
                ]);
                shadows.extend_from_slice(&[rx, smp.h as f32 + 0.08, rz, length as f32 * 0.8]);
            }
        }
        (out, shadows)
    }
}

impl NearGrid {
    fn build(pts: &[(f64, f64)]) -> NearGrid {
        let margin = 2600.0;
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

pub struct TerrainData {
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

struct Request {
    route: Arc<Route>,
    cx: f64,
    cz: f64,
}

/// Builds terrain windows off the UI thread.
pub struct Worker {
    tx: Sender<Request>,
    pub rx: Receiver<TerrainData>,
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
                    if done_tx.send(data).is_err() {
                        break;
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
    fn lake_is_inside_the_loop() {
        let r = route::builtin()
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
            (w.instances[BROADLEAF].len() + w.instances[BIRCH].len()) / INSTANCE_STRIDE,
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
