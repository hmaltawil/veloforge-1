//! Procedural low-poly meshes. Vertices are (position, normal, colour,
//! material); negative colour components mark parts tinted per instance.
//! Materials are listed at `OBJECT_FS` in `shaders.rs`.

use super::math::*;

pub const VERTEX_STRIDE: usize = 10;

pub const MAT_MATT: f32 = 0.0;
pub const MAT_WINDOW: f32 = 1.0;
pub const MAT_LAMP: f32 = 2.0;
pub const MAT_GLOSS: f32 = 3.0;
pub const MAT_REFLECTOR: f32 = 4.0;
pub const MAT_LEAF: f32 = 5.0;

#[derive(Default)]
pub struct Mesh {
    pub v: Vec<f32>,
    /// Material given to the vertices pushed from now on.
    pub mat: f32,
}

impl Mesh {
    pub fn vertex_count(&self) -> i32 {
        (self.v.len() / VERTEX_STRIDE) as i32
    }

    pub fn push(&mut self, p: V3, n: V3, c: V3) {
        self.v.extend_from_slice(&[
            p[0], p[1], p[2], n[0], n[1], n[2], c[0], c[1], c[2], self.mat,
        ]);
    }

    pub fn tri(&mut self, a: V3, b: V3, c: V3, col: V3) {
        let n = norm(cross(sub(b, a), sub(c, a)));
        self.push(a, n, col);
        self.push(b, n, col);
        self.push(c, n, col);
    }

    pub fn quad(&mut self, a: V3, b: V3, c: V3, d: V3, col: V3) {
        self.tri(a, b, c, col);
        self.tri(a, c, d, col);
    }

    /// Axis-aligned box.
    pub fn cuboid(&mut self, min: V3, max: V3, col: V3) {
        let [x0, y0, z0] = min;
        let [x1, y1, z1] = max;
        let p = |x, y, z| [x, y, z];
        self.quad(
            p(x0, y0, z1),
            p(x1, y0, z1),
            p(x1, y1, z1),
            p(x0, y1, z1),
            col,
        ); // +z
        self.quad(
            p(x1, y0, z0),
            p(x0, y0, z0),
            p(x0, y1, z0),
            p(x1, y1, z0),
            col,
        ); // -z
        self.quad(
            p(x1, y0, z1),
            p(x1, y0, z0),
            p(x1, y1, z0),
            p(x1, y1, z1),
            col,
        ); // +x
        self.quad(
            p(x0, y0, z0),
            p(x0, y0, z1),
            p(x0, y1, z1),
            p(x0, y1, z0),
            col,
        ); // -x
        self.quad(
            p(x0, y1, z1),
            p(x1, y1, z1),
            p(x1, y1, z0),
            p(x0, y1, z0),
            col,
        ); // +y
        self.quad(
            p(x0, y0, z0),
            p(x1, y0, z0),
            p(x1, y0, z1),
            p(x0, y0, z1),
            col,
        ); // -y
    }

    /// Tapered tube from `a` to `b` with smooth normals (a cone if `r1` = 0).
    pub fn tube(&mut self, a: V3, b: V3, r0: f32, r1: f32, sides: usize, col: V3) {
        let axis = sub(b, a);
        let l = len(axis);
        if l < 1e-5 {
            return;
        }
        let w = scale(axis, 1.0 / l);
        let helper = if w[1].abs() < 0.9 {
            [0.0, 1.0, 0.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        let u = norm(cross(w, helper));
        let v = cross(w, u);
        let slope = (r0 - r1) / l;
        let ring = |i: usize| {
            let t = i as f32 / sides as f32 * std::f32::consts::TAU;
            add(scale(u, t.cos()), scale(v, t.sin()))
        };
        for i in 0..sides {
            let (d0, d1) = (ring(i), ring(i + 1));
            let n0 = norm(add(d0, scale(w, slope)));
            let n1 = norm(add(d1, scale(w, slope)));
            let (p0, p1) = (add(a, scale(d0, r0)), add(a, scale(d1, r0)));
            let (q0, q1) = (add(b, scale(d0, r1)), add(b, scale(d1, r1)));
            self.push(p0, n0, col);
            self.push(p1, n1, col);
            self.push(q1, n1, col);
            self.push(p0, n0, col);
            self.push(q1, n1, col);
            self.push(q0, n0, col);
        }
    }

    /// Flat disc facing `n`.
    pub fn disc(&mut self, c: V3, r: f32, n: V3, sides: usize, col: V3) {
        let helper = if n[1].abs() < 0.9 {
            [0.0, 1.0, 0.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        let u = norm(cross(n, helper));
        let v = cross(n, u);
        for i in 0..sides {
            let t0 = i as f32 / sides as f32 * std::f32::consts::TAU;
            let t1 = (i + 1) as f32 / sides as f32 * std::f32::consts::TAU;
            let p0 = add(c, add(scale(u, t0.cos() * r), scale(v, t0.sin() * r)));
            let p1 = add(c, add(scale(u, t1.cos() * r), scale(v, t1.sin() * r)));
            self.push(c, n, col);
            self.push(p0, n, col);
            self.push(p1, n, col);
        }
    }

    /// Ellipsoid with smooth normals; `jitter` varies the colour per vertex.
    pub fn ellipsoid(&mut self, c: V3, r: V3, rings: usize, segs: usize, col: V3, jitter: f32) {
        let pt = |i: usize, j: usize| -> (V3, V3) {
            let th = i as f32 / rings as f32 * std::f32::consts::PI;
            let ph = j as f32 / segs as f32 * std::f32::consts::TAU;
            let d = [th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin()];
            let p = add(c, [d[0] * r[0], d[1] * r[1], d[2] * r[2]]);
            let n = norm([d[0] / r[0], d[1] / r[1], d[2] / r[2]]);
            (p, n)
        };
        let shade = |i: usize, j: usize| {
            let h = ((i * 73 + j * 151) % 97) as f32 / 97.0;
            // Lighter towards the top.
            let k = 1.0 + jitter * (h - 0.5) + 0.25 * (1.0 - i as f32 / rings as f32) - 0.1;
            scale(col, k)
        };
        for i in 0..rings {
            for j in 0..segs {
                let (a, na) = pt(i, j);
                let (b, nb) = pt(i + 1, j);
                let (cc, nc) = pt(i + 1, j + 1);
                let (d, nd) = pt(i, j + 1);
                let (ca, cb, ccc, cd) = (
                    shade(i, j),
                    shade(i + 1, j),
                    shade(i + 1, j + 1),
                    shade(i, j + 1),
                );
                self.push(a, na, ca);
                self.push(b, nb, cb);
                self.push(cc, nc, ccc);
                self.push(a, na, ca);
                self.push(cc, nc, ccc);
                self.push(d, nd, cd);
            }
        }
    }

    /// Torus around `axis` (unit), e.g. a bicycle tyre.
    #[allow(clippy::too_many_arguments)]
    pub fn torus(
        &mut self,
        c: V3,
        axis: V3,
        radius: f32,
        tube: f32,
        segs: usize,
        sides: usize,
        col: V3,
    ) {
        let helper = if axis[1].abs() < 0.9 {
            [0.0, 1.0, 0.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        let u = norm(cross(axis, helper));
        let v = cross(axis, u);
        let pt = |i: usize, j: usize| -> (V3, V3) {
            let a = i as f32 / segs as f32 * std::f32::consts::TAU;
            let b = j as f32 / sides as f32 * std::f32::consts::TAU;
            let dir = add(scale(u, a.cos()), scale(v, a.sin()));
            let n = add(scale(dir, b.cos()), scale(axis, b.sin()));
            (add(c, add(scale(dir, radius), scale(n, tube))), n)
        };
        for i in 0..segs {
            for j in 0..sides {
                let (a, na) = pt(i, j);
                let (b, nb) = pt(i + 1, j);
                let (cc, nc) = pt(i + 1, j + 1);
                let (d, nd) = pt(i, j + 1);
                self.push(a, na, col);
                self.push(b, nb, col);
                self.push(cc, nc, col);
                self.push(a, na, col);
                self.push(cc, nc, col);
                self.push(d, nd, col);
            }
        }
    }
}

/// Tinted marker: colours are negated so the instance tint applies.
fn tinted(c: V3) -> V3 {
    scale(c, -1.0)
}

/// Deterministic pseudo-random number in 0..1.
fn rnd(a: u32, b: u32, c: u32) -> f32 {
    let mut h = a
        .wrapping_mul(0x9E37_79B1)
        .wrapping_add(b.wrapping_mul(0x85EB_CA77))
        .wrapping_add(c.wrapping_mul(0xC2B2_AE3D));
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h & 0xFFFF) as f32 / 65535.0
}

/// The crown of a tree: where it is and how far it reaches. Foliage is
/// shaded as one soft volume around it.
#[derive(Clone, Copy)]
struct Crown {
    c: V3,
    r: f32,
}

impl Mesh {
    /// A lumpy clump of leaves. Normals lean away from the crown's centre
    /// and colours darken towards its inside and underside.
    #[allow(clippy::too_many_arguments)]
    fn clump(
        &mut self,
        c: V3,
        r: V3,
        rings: usize,
        segs: usize,
        col: V3,
        seed: u32,
        lump: f32,
        crown: Crown,
    ) {
        let pt = |i: usize, j: usize| -> (V3, V3, V3) {
            let j = if i == 0 || i == rings { 0 } else { j % segs };
            let th = i as f32 / rings as f32 * std::f32::consts::PI;
            let ph = (j as f32 + 0.5 * (i % 2) as f32) / segs as f32 * std::f32::consts::TAU;
            let d = [th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin()];
            let k = 1.0 + lump * (rnd(seed, i as u32, j as u32) - 0.5) * 2.0;
            let p = add(c, [d[0] * r[0] * k, d[1] * r[1] * k, d[2] * r[2] * k]);
            let local = norm([d[0] / r[0], d[1] / r[1], d[2] / r[2]]);
            let radial = norm(sub(p, crown.c));
            let n = norm(add(scale(local, 0.45), scale(radial, 0.55)));
            let height = ((p[1] - (crown.c[1] - crown.r)) / (2.0 * crown.r)).clamp(0.0, 1.0);
            let outer = (len(sub(p, crown.c)) / crown.r).clamp(0.0, 1.2);
            let k = (0.45 + 0.6 * height)
                * (0.55 + 0.45 * outer)
                * (1.0 + 0.2 * (rnd(seed + 7, i as u32, j as u32) - 0.5));
            (p, n, scale(col, k))
        };
        for i in 0..rings {
            for j in 0..segs {
                let quad = [pt(i, j), pt(i + 1, j), pt(i + 1, j + 1), pt(i, j + 1)];
                for idx in [0, 1, 2, 0, 2, 3] {
                    let (p, n, c) = quad[idx];
                    self.push(p, n, c);
                }
            }
        }
    }

    /// One whorl of conifer branches: a cone with a ragged, drooping rim.
    #[allow(clippy::too_many_arguments)]
    fn whorl(&mut self, y0: f32, y1: f32, r: f32, sides: usize, col: V3, seed: u32, turn: f32) {
        let rim = |i: usize| -> (V3, V3, V3) {
            let i = i % sides;
            let a = i as f32 / sides as f32 * std::f32::consts::TAU + turn;
            let tip = i & 1 == 0;
            let k = (if tip { 1.0 } else { 0.7 }) * (1.0 + 0.16 * (rnd(seed, i as u32, 1) - 0.5));
            let droop = (y1 - y0) * if tip { 0.2 } else { 0.05 };
            let p = [a.cos() * r * k, y0 - droop, a.sin() * r * k];
            let n = norm([a.cos() * 0.8, 0.6, a.sin() * 0.8]);
            let c = scale(col, if tip { 1.25 } else { 0.85 });
            (p, n, c)
        };
        let apex = [0.0, y1, 0.0];
        let under = [0.0, y0 + (y1 - y0) * 0.22, 0.0];
        for i in 0..sides {
            let (a, na, ca) = rim(i);
            let (b, nb, cb) = rim(i + 1);
            let nt = norm(add(add(na, nb), [0.0, 1.2, 0.0]));
            self.push(apex, nt, scale(col, 0.7));
            self.push(a, na, ca);
            self.push(b, nb, cb);
            // The shaded underside.
            let down = [0.0, -1.0, 0.0];
            self.push(under, down, scale(col, 0.3));
            self.push(b, down, scale(col, 0.45));
            self.push(a, down, scale(col, 0.45));
        }
    }

    /// A faceted lump of stone.
    fn boulder(&mut self, c: V3, r: V3, rings: usize, segs: usize, col: V3, seed: u32) {
        let pt = |i: usize, j: usize| -> V3 {
            let j = if i == 0 || i == rings { 0 } else { j % segs };
            let th = i as f32 / rings as f32 * std::f32::consts::PI;
            let ph = j as f32 / segs as f32 * std::f32::consts::TAU;
            let d = [th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin()];
            let k = 1.0 + 0.5 * (rnd(seed, i as u32, j as u32) - 0.5);
            add(c, [d[0] * r[0] * k, d[1] * r[1] * k, d[2] * r[2] * k])
        };
        for i in 0..rings {
            for j in 0..segs {
                let (a, b, cc, d) = (pt(i, j), pt(i + 1, j), pt(i + 1, j + 1), pt(i, j + 1));
                let shade =
                    |k: u32| scale(col, 0.8 + 0.4 * rnd(seed + 3, (i * segs + j) as u32, k));
                if i == 0 {
                    self.tri(a, cc, b, shade(0));
                } else if i + 1 == rings {
                    self.tri(a, d, b, shade(0));
                } else {
                    self.tri(a, cc, b, shade(0));
                    self.tri(a, d, cc, shade(1));
                }
            }
        }
    }
}

const BARK: V3 = [0.115, 0.075, 0.05];
const PINE_GREEN: V3 = [0.028, 0.07, 0.036];
const LEAF_GREEN: V3 = [0.07, 0.165, 0.035];

/// Unit-height conifer.
pub fn pine() -> Mesh {
    let mut m = Mesh::default();
    m.tube([0.0, 0.0, 0.0], [0.0, 0.07, 0.0], 0.05, 0.034, 7, BARK);
    m.tube([0.0, 0.07, 0.0], [0.0, 0.5, 0.0], 0.034, 0.018, 7, BARK);
    m.mat = MAT_LEAF;
    let tiers = 7;
    for k in 0..tiers {
        let t = k as f32 / (tiers - 1) as f32;
        let y0 = 0.15 + 0.66 * t;
        let r = 0.4 * (1.0 - t).powf(0.8) + 0.085;
        let y1 = if k + 1 == tiers {
            1.0
        } else {
            y0 + 0.24 - 0.06 * t
        };
        let col = scale(PINE_GREEN, 0.8 + 0.5 * t);
        m.whorl(y0, y1, r, 11, tinted(col), 40 + k as u32, k as f32 * 0.9);
    }
    m
}

/// Simplified conifer for distant trees.
pub fn pine_far() -> Mesh {
    let mut m = Mesh::default();
    m.tube([0.0, 0.0, 0.0], [0.0, 0.2, 0.0], 0.04, 0.03, 4, BARK);
    m.mat = MAT_LEAF;
    for (k, (y0, y1, r)) in [(0.15, 0.55, 0.42), (0.4, 0.8, 0.3), (0.65, 1.0, 0.18)]
        .into_iter()
        .enumerate()
    {
        let col = scale(PINE_GREEN, 0.85 + 0.2 * k as f32);
        m.whorl(y0, y1, r, 6, tinted(col), 70 + k as u32, k as f32);
    }
    m
}

/// Unit-height broadleaf tree: a spreading crown on a stout trunk.
pub fn broadleaf() -> Mesh {
    let mut m = Mesh::default();
    m.tube([0.0, 0.0, 0.0], [0.0, 0.06, 0.0], 0.075, 0.048, 7, BARK);
    m.tube([0.0, 0.06, 0.0], [0.015, 0.46, 0.0], 0.048, 0.032, 7, BARK);
    let fork = [0.015, 0.42, 0.0];
    for end in [
        [0.2, 0.62, 0.08],
        [-0.19, 0.62, -0.1],
        [0.03, 0.64, -0.2],
        [-0.05, 0.7, 0.18],
    ] {
        m.tube(fork, end, 0.024, 0.01, 5, BARK);
    }
    m.mat = MAT_LEAF;
    let crown = Crown {
        c: [0.0, 0.68, 0.0],
        r: 0.44,
    };
    let clumps: [(V3, V3, f32); 8] = [
        ([0.0, 0.7, 0.0], [0.3, 0.24, 0.3], 1.0),
        ([0.25, 0.62, 0.1], [0.22, 0.18, 0.22], 0.93),
        ([-0.23, 0.63, -0.12], [0.23, 0.19, 0.23], 1.04),
        ([0.05, 0.61, -0.26], [0.2, 0.17, 0.2], 0.9),
        ([-0.08, 0.6, 0.26], [0.21, 0.17, 0.2], 0.97),
        ([0.04, 0.9, 0.02], [0.2, 0.13, 0.2], 1.14),
        ([-0.17, 0.82, 0.11], [0.17, 0.14, 0.17], 1.08),
        ([0.19, 0.82, -0.09], [0.17, 0.14, 0.17], 1.1),
    ];
    for (k, (c, r, g)) in clumps.into_iter().enumerate() {
        m.clump(
            c,
            r,
            4,
            7,
            tinted(scale(LEAF_GREEN, g)),
            11 + k as u32,
            0.2,
            crown,
        );
    }
    m
}

/// Simplified broadleaf tree for distant trees.
pub fn broadleaf_far() -> Mesh {
    let mut m = Mesh::default();
    m.tube([0.0, 0.0, 0.0], [0.0, 0.5, 0.0], 0.05, 0.035, 4, BARK);
    m.mat = MAT_LEAF;
    let crown = Crown {
        c: [0.0, 0.68, 0.0],
        r: 0.44,
    };
    let leaf = tinted(LEAF_GREEN);
    m.clump(
        [0.0, 0.72, 0.0],
        [0.36, 0.28, 0.36],
        3,
        6,
        leaf,
        5,
        0.2,
        crown,
    );
    m.clump(
        [0.2, 0.62, 0.1],
        [0.24, 0.2, 0.24],
        2,
        5,
        leaf,
        6,
        0.2,
        crown,
    );
    m.clump(
        [-0.2, 0.63, -0.12],
        [0.25, 0.2, 0.25],
        2,
        5,
        leaf,
        7,
        0.2,
        crown,
    );
    m
}

/// Unit-height birch: a slim pale trunk and a light, airy crown.
pub fn birch() -> Mesh {
    let mut m = Mesh::default();
    let pale = [0.6, 0.58, 0.52];
    m.tube([0.0, 0.0, 0.0], [0.01, 0.78, 0.0], 0.026, 0.011, 6, pale);
    for (k, y) in [0.08, 0.2, 0.27, 0.41, 0.52].into_iter().enumerate() {
        let r = 0.027 - 0.018 * y;
        let h = 0.012 + 0.006 * (k % 2) as f32;
        m.tube(
            [0.0, y, 0.0],
            [0.0, y + h, 0.0],
            r,
            r,
            6,
            [0.05, 0.045, 0.04],
        );
    }
    for end in [[0.13, 0.66, 0.05], [-0.12, 0.6, -0.06], [0.02, 0.72, -0.13]] {
        m.tube([0.006, 0.47, 0.0], end, 0.011, 0.005, 4, pale);
    }
    m.mat = MAT_LEAF;
    let crown = Crown {
        c: [0.0, 0.7, 0.0],
        r: 0.34,
    };
    let leaf = [0.105, 0.19, 0.035];
    let clumps: [(V3, V3); 6] = [
        ([0.0, 0.84, 0.0], [0.17, 0.17, 0.17]),
        ([0.15, 0.68, 0.06], [0.15, 0.14, 0.15]),
        ([-0.14, 0.64, -0.07], [0.15, 0.15, 0.15]),
        ([0.02, 0.72, -0.16], [0.14, 0.13, 0.14]),
        ([-0.04, 0.6, 0.15], [0.14, 0.12, 0.14]),
        ([0.06, 0.5, -0.02], [0.12, 0.1, 0.12]),
    ];
    for (k, (c, r)) in clumps.into_iter().enumerate() {
        m.clump(c, r, 4, 6, tinted(leaf), 31 + k as u32, 0.22, crown);
    }
    m
}

/// Unit-height bush.
pub fn bush() -> Mesh {
    let mut m = Mesh {
        mat: MAT_LEAF,
        ..Default::default()
    };
    let crown = Crown {
        c: [0.0, 0.4, 0.0],
        r: 0.62,
    };
    let leaf = [0.055, 0.125, 0.035];
    m.clump(
        [0.0, 0.48, 0.0],
        [0.42, 0.5, 0.4],
        4,
        7,
        tinted(leaf),
        51,
        0.22,
        crown,
    );
    m.clump(
        [0.3, 0.3, 0.12],
        [0.3, 0.33, 0.3],
        3,
        6,
        tinted(scale(leaf, 1.1)),
        52,
        0.22,
        crown,
    );
    m.clump(
        [-0.26, 0.33, -0.16],
        [0.32, 0.36, 0.3],
        3,
        6,
        tinted(scale(leaf, 0.9)),
        53,
        0.22,
        crown,
    );
    m
}

/// Irregular boulder about one unit across.
pub fn rock() -> Mesh {
    let mut m = Mesh::default();
    m.boulder(
        [0.0, 0.25, 0.0],
        [0.5, 0.45, 0.42],
        4,
        7,
        tinted([0.13, 0.125, 0.12]),
        3,
    );
    m.boulder(
        [0.26, 0.16, 0.14],
        [0.3, 0.28, 0.26],
        3,
        6,
        tinted([0.11, 0.108, 0.105]),
        9,
    );
    m
}

/// Farmhouse one unit long (x), with a pitched roof; the roof is tinted.
pub fn house() -> Mesh {
    let mut m = Mesh::default();
    let wall = [0.66, 0.62, 0.54];
    let stone = [0.2, 0.19, 0.18];
    let timber = [0.11, 0.07, 0.045];
    // Stone footing, plastered walls.
    m.cuboid([-0.505, -0.1, -0.305], [0.505, 0.07, 0.305], stone);
    m.cuboid([-0.5, 0.07, -0.3], [0.5, 0.38, 0.3], wall);
    // Windows with shutters and a door, on both long sides.
    for side in [-1.0f32, 1.0] {
        let z = side * 0.302;
        let face = |m: &mut Mesh, a: f32, b: f32, y0: f32, y1: f32, z: f32, col: V3| {
            if side > 0.0 {
                m.quad([a, y0, z], [b, y0, z], [b, y1, z], [a, y1, z], col);
            } else {
                m.quad([b, y0, z], [a, y0, z], [a, y1, z], [b, y1, z], col);
            }
        };
        for (k, wx) in [-0.36f32, -0.17, 0.17, 0.36].into_iter().enumerate() {
            let lit = (k as i32 + if side > 0.0 { 0 } else { 1 }) % 3 != 0;
            m.mat = if lit { MAT_WINDOW } else { MAT_GLOSS };
            face(
                &mut m,
                wx - 0.042,
                wx + 0.042,
                0.17,
                0.29,
                z,
                [0.03, 0.04, 0.055],
            );
            m.mat = MAT_MATT;
            // Sill and shutters.
            face(
                &mut m,
                wx - 0.052,
                wx + 0.052,
                0.155,
                0.17,
                z + side * 0.004,
                [0.5, 0.48, 0.44],
            );
            for sx in [-0.064f32, 0.064] {
                face(
                    &mut m,
                    wx + sx - 0.02,
                    wx + sx + 0.02,
                    0.17,
                    0.29,
                    z + side * 0.002,
                    [0.1, 0.2, 0.13],
                );
            }
        }
        if side > 0.0 {
            face(&mut m, -0.04, 0.04, 0.07, 0.24, z + 0.002, timber);
        }
    }
    // Roof with overhang, on timber eaves.
    let roof = tinted([1.0, 1.0, 1.0]);
    let (ex, ez, ey, ry) = (0.55, 0.37, 0.355, 0.62);
    m.quad(
        [-ex, ey, ez],
        [ex, ey, ez],
        [ex, ry, 0.0],
        [-ex, ry, 0.0],
        roof,
    );
    m.quad(
        [ex, ey, -ez],
        [-ex, ey, -ez],
        [-ex, ry, 0.0],
        [ex, ry, 0.0],
        roof,
    );
    let under = tinted([0.45, 0.45, 0.45]);
    m.quad(
        [-ex, ey - 0.012, ez],
        [-ex, ry - 0.012, 0.0],
        [ex, ry - 0.012, 0.0],
        [ex, ey - 0.012, ez],
        under,
    );
    m.quad(
        [ex, ey - 0.012, -ez],
        [ex, ry - 0.012, 0.0],
        [-ex, ry - 0.012, 0.0],
        [-ex, ey - 0.012, -ez],
        under,
    );
    // Gables, boarded with dark timber.
    m.tri([0.5, 0.38, 0.3], [0.5, 0.38, -0.3], [0.5, 0.6, 0.0], timber);
    m.tri(
        [-0.5, 0.38, -0.3],
        [-0.5, 0.38, 0.3],
        [-0.5, 0.6, 0.0],
        timber,
    );
    m.cuboid([0.2, 0.44, -0.13], [0.29, 0.7, -0.04], [0.32, 0.2, 0.16]);
    m.cuboid([0.19, 0.7, -0.14], [0.3, 0.72, -0.03], stone);
    // A lean-to woodshed at one end.
    m.cuboid([-0.72, -0.1, -0.24], [-0.5, 0.2, 0.2], timber);
    m.quad(
        [-0.74, 0.19, 0.23],
        [-0.49, 0.3, 0.23],
        [-0.49, 0.3, -0.27],
        [-0.74, 0.19, -0.27],
        roof,
    );
    m
}

/// Unit box (x and z in [-0.5, 0.5], y in [0, 1]), fully tinted.
pub fn post() -> Mesh {
    let mut m = Mesh::default();
    m.cuboid([-0.5, 0.0, -0.5], [0.5, 1.0, 0.5], tinted([1.0, 1.0, 1.0]));
    m
}

/// The same box as a reflector that lights up in the headlight.
pub fn reflector() -> Mesh {
    let mut m = Mesh {
        mat: MAT_REFLECTOR,
        ..Default::default()
    };
    m.cuboid([-0.5, 0.0, -0.5], [0.5, 1.0, 0.5], tinted([1.0, 1.0, 1.0]));
    m
}

/// Thin board (x in [-0.5, 0.5], y in [0, 1]), fully tinted.
pub fn plank() -> Mesh {
    let mut m = Mesh::default();
    m.cuboid(
        [-0.5, 0.0, -0.03],
        [0.5, 1.0, 0.03],
        tinted([1.0, 1.0, 1.0]),
    );
    m
}

/// Floats per grass vertex: position (3), colour (3).
pub const GRASS_STRIDE: usize = 6;

/// A tuft of grass about one unit tall. Colours below zero take the meadow's
/// colour; `flowers` adds stems with flower heads (colour marker -2), and
/// `seeding` tall stalks with seed heads.
pub fn grass_tuft(blades: usize, flowers: usize, seeding: bool, seed: u32) -> Vec<f32> {
    let mut v = Vec::new();
    let mut tri = |a: V3, b: V3, c: V3, col: V3| {
        for p in [a, b, c] {
            v.extend_from_slice(&[p[0], p[1], p[2], col[0], col[1], col[2]]);
        }
    };
    let green = [-1.0, -1.0, -1.0];
    for k in 0..blades {
        let a = rnd(seed, k as u32, 0) * std::f32::consts::TAU;
        let (dx, dz) = (a.cos(), a.sin());
        let out = 0.1 + 0.5 * rnd(seed, k as u32, 1);
        let h = 0.55 + 0.45 * rnd(seed, k as u32, 2);
        let wd = 0.045 + 0.03 * rnd(seed, k as u32, 3);
        let lean = 0.15 + 0.45 * rnd(seed, k as u32, 4);
        // Root, a bend halfway up, and the tip leaning outwards.
        let root = [dx * out * 0.3, 0.0, dz * out * 0.3];
        let mid = [
            dx * (out * 0.3 + lean * 0.3 * h),
            h * 0.55,
            dz * (out * 0.3 + lean * 0.3 * h),
        ];
        let tip = [dx * (out * 0.3 + lean * h), h, dz * (out * 0.3 + lean * h)];
        let (sx, sz) = (-dz * wd, dx * wd);
        let (r0, r1) = (
            [root[0] - sx, 0.0, root[2] - sz],
            [root[0] + sx, 0.0, root[2] + sz],
        );
        let (m0, m1) = (
            [mid[0] - sx * 0.7, mid[1], mid[2] - sz * 0.7],
            [mid[0] + sx * 0.7, mid[1], mid[2] + sz * 0.7],
        );
        tri(r0, r1, m1, green);
        tri(r0, m1, m0, green);
        tri(m0, m1, tip, green);
    }
    for k in 0..flowers {
        let a = rnd(seed + 5, k as u32, 0) * std::f32::consts::TAU;
        let out = 0.25 * rnd(seed + 5, k as u32, 1);
        let h = 0.75 + 0.35 * rnd(seed + 5, k as u32, 2);
        let base = [a.cos() * out, 0.0, a.sin() * out];
        let top = [base[0] * 1.6, h, base[2] * 1.6];
        let stem = [-0.8, -0.8, -0.8];
        tri(
            [base[0] - 0.012, 0.0, base[2]],
            [base[0] + 0.012, 0.0, base[2]],
            top,
            stem,
        );
        tri(
            [base[0], 0.0, base[2] - 0.012],
            [base[0], 0.0, base[2] + 0.012],
            top,
            stem,
        );
        // The head: a little six-sided disc, tilted to the sky.
        let r = 0.085;
        let head = [-2.0, 0.0, 0.0];
        for i in 0..6 {
            let (a0, a1) = (
                i as f32 / 6.0 * std::f32::consts::TAU,
                (i + 1) as f32 / 6.0 * std::f32::consts::TAU,
            );
            tri(
                [top[0], top[1] + 0.03, top[2]],
                [top[0] + a0.cos() * r, top[1], top[2] + a0.sin() * r],
                [top[0] + a1.cos() * r, top[1], top[2] + a1.sin() * r],
                head,
            );
        }
    }
    if seeding {
        for k in 0..3u32 {
            let a = rnd(seed + 9, k, 0) * std::f32::consts::TAU;
            let lean = 0.12 + 0.2 * rnd(seed + 9, k, 1);
            let h = 1.25 + 0.5 * rnd(seed + 9, k, 2);
            let top = [a.cos() * lean, h, a.sin() * lean];
            let straw = [0.3, 0.26, 0.1];
            tri([-0.01, 0.0, 0.0], [0.01, 0.0, 0.0], top, [-0.9, -0.9, -0.9]);
            let (sx, sz) = (-a.sin() * 0.035, a.cos() * 0.035);
            tri(
                [top[0] * 0.8 - sx, h * 0.8, top[2] * 0.8 - sz],
                [top[0] * 0.8 + sx, h * 0.8, top[2] * 0.8 + sz],
                [top[0] * 1.15, h * 1.08, top[2] * 1.15],
                straw,
            );
        }
    }
    v
}

pub struct RiderPose {
    /// Ground point under the bottom bracket.
    pub pos: V3,
    /// Unit vectors: forward, up and right.
    pub fwd: V3,
    pub up: V3,
    pub right: V3,
    pub crank: f32,
    /// How far the wheels have turned, in radians.
    pub wheel: f32,
    pub jersey: V3,
}

/// Two-bone IK: the joint between `a` and `c` with bone lengths `l1`, `l2`,
/// bending towards `hint`.
fn ik(a: V3, c: V3, l1: f32, l2: f32, hint: V3) -> V3 {
    let ac = sub(c, a);
    let d = len(ac).clamp(0.05, l1 + l2 - 1e-3);
    let dir = norm(ac);
    let x = (d * d + l1 * l1 - l2 * l2) / (2.0 * d);
    let h = (l1 * l1 - x * x).max(0.0).sqrt();
    let bend = norm(sub(hint, scale(dir, dot(hint, dir))));
    add(add(a, scale(dir, x)), scale(bend, h))
}

/// A cyclist on a road bike, posed from the crank angle.
pub fn rider(p: &RiderPose) -> Mesh {
    let mut m = Mesh::default();
    let at = |x: f32, y: f32, z: f32| {
        add(
            p.pos,
            add(add(scale(p.fwd, x), scale(p.up, y)), scale(p.right, z)),
        )
    };
    let rubber = [0.012, 0.012, 0.014];
    let frame = [0.028, 0.03, 0.036];
    let metal = [0.22, 0.22, 0.23];
    let skin = [0.5, 0.3, 0.2];
    let shorts = [0.012, 0.012, 0.016];
    let white = [0.78, 0.78, 0.78];
    let jersey = p.jersey;
    let jersey_dark = scale(jersey, 0.5);

    // Wheels: tyre, rim, hub and spokes that turn with the road.
    for x in [-0.5f32, 0.5] {
        let hub = at(x, 0.34, 0.0);
        m.mat = MAT_MATT;
        m.torus(hub, p.right, 0.326, 0.016, 28, 6, rubber);
        m.mat = MAT_GLOSS;
        m.torus(hub, p.right, 0.296, 0.013, 28, 5, [0.06, 0.06, 0.065]);
        m.tube(
            at(x, 0.34, -0.035),
            at(x, 0.34, 0.035),
            0.018,
            0.018,
            6,
            metal,
        );
        for k in 0..10 {
            let a = p.wheel + k as f32 / 10.0 * std::f32::consts::TAU;
            let z = if k % 2 == 0 { 0.022 } else { -0.022 };
            let rim = at(x + a.sin() * 0.288, 0.34 + a.cos() * 0.288, 0.0);
            m.tube(add(hub, scale(p.right, z)), rim, 0.0045, 0.0045, 3, metal);
        }
    }
    // Frame, fork and stays.
    m.mat = MAT_GLOSS;
    let bb = at(-0.02, 0.28, 0.0);
    let seat_top = at(-0.17, 0.82, 0.0);
    let head_top = at(0.4, 0.8, 0.0);
    let head_bot = at(0.44, 0.64, 0.0);
    let tube = |m: &mut Mesh, a: V3, b: V3, r: f32| m.tube(a, b, r, r, 7, frame);
    tube(&mut m, bb, seat_top, 0.018);
    tube(&mut m, at(-0.155, 0.775, 0.0), head_top, 0.017);
    tube(&mut m, bb, head_bot, 0.024);
    tube(&mut m, head_bot, head_top, 0.022);
    for z in [-0.045f32, 0.045] {
        tube(&mut m, bb, at(-0.5, 0.34, z), 0.012);
        tube(&mut m, at(-0.16, 0.76, 0.0), at(-0.5, 0.34, z), 0.009);
        tube(&mut m, head_bot, at(0.5, 0.34, z), 0.013);
    }
    // Seat post and saddle.
    m.tube(
        seat_top,
        at(-0.19, 0.89, 0.0),
        0.013,
        0.013,
        6,
        [0.02, 0.02, 0.022],
    );
    m.mat = MAT_MATT;
    m.ellipsoid(at(-0.2, 0.905, 0.0), [0.13, 0.02, 0.065], 3, 7, rubber, 0.0);
    // Stem and drop handlebar with hoods.
    m.mat = MAT_GLOSS;
    m.tube(
        head_top,
        at(0.5, 0.86, 0.0),
        0.014,
        0.014,
        6,
        [0.02, 0.02, 0.022],
    );
    m.mat = MAT_MATT;
    let tape = [0.02, 0.02, 0.022];
    m.tube(
        at(0.5, 0.86, -0.2),
        at(0.5, 0.86, 0.2),
        0.012,
        0.012,
        6,
        tape,
    );
    for z in [-0.2f32, 0.2] {
        m.tube(at(0.5, 0.86, z), at(0.585, 0.855, z), 0.012, 0.012, 6, tape);
        m.tube(at(0.585, 0.855, z), at(0.6, 0.78, z), 0.012, 0.012, 6, tape);
        m.tube(at(0.6, 0.78, z), at(0.53, 0.73, z), 0.012, 0.012, 6, tape);
        m.tube(
            at(0.575, 0.855, z),
            at(0.61, 0.915, z),
            0.017,
            0.012,
            6,
            rubber,
        );
    }
    // Bottle, chainring, lights.
    m.tube(
        at(0.1, 0.4, 0.0),
        at(0.21, 0.54, 0.0),
        0.034,
        0.034,
        7,
        white,
    );
    m.tube(
        at(0.21, 0.54, 0.0),
        at(0.235, 0.572, 0.0),
        0.02,
        0.016,
        6,
        jersey,
    );
    m.mat = MAT_GLOSS;
    m.disc(
        add(bb, scale(p.right, 0.05)),
        0.095,
        p.right,
        14,
        [0.05, 0.05, 0.055],
    );
    m.mat = MAT_LAMP;
    m.cuboid_at(
        at(-0.2, 0.83, 0.0),
        p,
        [0.012, 0.02, 0.02],
        [0.9, 0.03, 0.02],
    );
    m.cuboid_at(
        at(0.53, 0.84, 0.0),
        p,
        [0.015, 0.014, 0.02],
        [0.9, 0.85, 0.7],
    );
    m.mat = MAT_MATT;

    // Cranks and pedals: right foot at `crank`, left opposite.
    let mut feet = [[0.0; 3]; 2];
    for (k, (side, phase)) in [(1.0f32, 0.0f32), (-1.0, std::f32::consts::PI)]
        .into_iter()
        .enumerate()
    {
        let a = p.crank + phase;
        let pedal = at(-0.02 + 0.17 * a.sin(), 0.28 - 0.17 * a.cos(), side * 0.1);
        m.tube(
            add(bb, scale(p.right, side * 0.07)),
            pedal,
            0.011,
            0.009,
            5,
            metal,
        );
        feet[k] = add(pedal, scale(p.up, 0.035));
        // Shoe: toes dip at the bottom of the stroke.
        let dip = -0.25 * a.cos().max(0.0) - 0.08;
        let heel = sub(feet[k], add(scale(p.fwd, 0.1), scale(p.up, 0.1 * dip)));
        let toe = add(feet[k], add(scale(p.fwd, 0.13), scale(p.up, 0.13 * dip)));
        m.mat = MAT_GLOSS;
        m.tube(heel, toe, 0.04, 0.03, 7, white);
        m.ellipsoid(toe, [0.032, 0.03, 0.032], 3, 6, white, 0.0);
        m.mat = MAT_MATT;
    }

    // Legs.
    let sway = 0.012 * p.crank.sin();
    let hips = at(-0.16, 0.95, sway * 0.5);
    for (k, side) in [1.0f32, -1.0].into_iter().enumerate() {
        let hip = add(hips, scale(p.right, side * 0.085));
        let ankle = add(feet[k], add(scale(p.up, 0.07), scale(p.fwd, -0.06)));
        let knee = ik(
            hip,
            ankle,
            0.44,
            0.45,
            add(p.fwd, scale(p.right, side * 0.12)),
        );
        m.tube(hip, knee, 0.08, 0.058, 9, shorts);
        // Shorts end above the knee.
        let hem = lerp(hip, knee, 0.72);
        m.tube(hem, knee, 0.064, 0.055, 9, skin);
        let calf = lerp(knee, ankle, 0.3);
        m.tube(knee, calf, 0.054, 0.06, 9, skin);
        let sock = lerp(knee, ankle, 0.76);
        m.tube(calf, sock, 0.06, 0.04, 9, skin);
        m.tube(sock, ankle, 0.04, 0.037, 8, white);
        m.ellipsoid(knee, [0.058, 0.058, 0.058], 4, 7, skin, 0.0);
    }
    m.ellipsoid(hips, [0.15, 0.105, 0.155], 5, 9, shorts, 0.0);

    // Torso with an arched back, arms, head.
    let shoulders = at(0.25, 1.27, -sway);
    let mid = add(lerp(hips, shoulders, 0.5), scale(p.up, 0.045));
    m.tube(hips, mid, 0.148, 0.165, 11, jersey);
    m.tube(mid, shoulders, 0.165, 0.16, 11, jersey);
    m.ellipsoid(mid, [0.165, 0.165, 0.165], 4, 9, jersey, 0.0);
    m.ellipsoid(shoulders, [0.15, 0.1, 0.205], 5, 9, jersey, 0.0);
    // Back pockets and a stripe.
    m.tube(
        lerp(hips, mid, 0.25),
        lerp(hips, mid, 0.55),
        0.158,
        0.164,
        11,
        jersey_dark,
    );
    m.tube(
        lerp(mid, shoulders, 0.35),
        lerp(mid, shoulders, 0.5),
        0.168,
        0.168,
        11,
        white,
    );
    for side in [1.0f32, -1.0] {
        let sh = add(shoulders, scale(p.right, side * 0.18));
        let hand = at(0.6, 0.93, side * 0.2);
        let elbow = ik(
            sh,
            hand,
            0.3,
            0.3,
            add(
                scale(p.up, -0.6),
                add(scale(p.fwd, -0.4), scale(p.right, side * 0.55)),
            ),
        );
        m.tube(sh, elbow, 0.052, 0.042, 8, jersey);
        m.tube(
            lerp(sh, elbow, 0.5),
            lerp(sh, elbow, 0.58),
            0.049,
            0.047,
            8,
            jersey_dark,
        );
        m.tube(lerp(sh, elbow, 0.58), elbow, 0.042, 0.038, 8, skin);
        m.ellipsoid(elbow, [0.039, 0.039, 0.039], 3, 6, skin, 0.0);
        m.tube(elbow, hand, 0.038, 0.03, 8, skin);
        m.ellipsoid(hand, [0.042, 0.036, 0.042], 3, 6, rubber, 0.0);
    }
    let neck = at(0.33, 1.38, -sway);
    m.tube(shoulders, neck, 0.055, 0.05, 7, skin);
    m.ellipsoid(at(0.38, 1.45, -sway), [0.1, 0.11, 0.09], 5, 8, skin, 0.0);
    // Helmet with vents and a tail, sunglasses.
    m.mat = MAT_GLOSS;
    m.ellipsoid(
        at(0.35, 1.51, -sway),
        [0.165, 0.095, 0.122],
        6,
        10,
        white,
        0.0,
    );
    m.ellipsoid(at(0.25, 1.5, -sway), [0.1, 0.06, 0.09], 4, 8, white, 0.0);
    for z in [-0.05f32, 0.0, 0.05] {
        m.ellipsoid(
            at(0.36, 1.585, z - sway),
            [0.11, 0.022, 0.014],
            3,
            6,
            [0.02, 0.02, 0.022],
            0.0,
        );
    }
    m.ellipsoid(at(0.36, 1.56, -sway), [0.13, 0.05, 0.04], 3, 6, jersey, 0.0);
    m.ellipsoid(
        at(0.465, 1.46, -sway),
        [0.03, 0.028, 0.085],
        3,
        7,
        [0.015, 0.015, 0.02],
        0.0,
    );
    m.mat = MAT_MATT;
    m
}

impl Mesh {
    /// Small box centred on `c`, aligned with the rider.
    fn cuboid_at(&mut self, c: V3, p: &RiderPose, half: V3, col: V3) {
        let corner = |x: f32, y: f32, z: f32| {
            add(
                c,
                add(
                    add(scale(p.fwd, x * half[0]), scale(p.up, y * half[1])),
                    scale(p.right, z * half[2]),
                ),
            )
        };
        let faces: [[(f32, f32, f32); 4]; 6] = [
            [
                (1.0, -1.0, -1.0),
                (1.0, -1.0, 1.0),
                (1.0, 1.0, 1.0),
                (1.0, 1.0, -1.0),
            ],
            [
                (-1.0, -1.0, 1.0),
                (-1.0, -1.0, -1.0),
                (-1.0, 1.0, -1.0),
                (-1.0, 1.0, 1.0),
            ],
            [
                (-1.0, 1.0, -1.0),
                (1.0, 1.0, -1.0),
                (1.0, 1.0, 1.0),
                (-1.0, 1.0, 1.0),
            ],
            [
                (-1.0, -1.0, 1.0),
                (1.0, -1.0, 1.0),
                (1.0, -1.0, -1.0),
                (-1.0, -1.0, -1.0),
            ],
            [
                (-1.0, -1.0, 1.0),
                (-1.0, 1.0, 1.0),
                (1.0, 1.0, 1.0),
                (1.0, -1.0, 1.0),
            ],
            [
                (1.0, -1.0, -1.0),
                (1.0, 1.0, -1.0),
                (-1.0, 1.0, -1.0),
                (-1.0, -1.0, -1.0),
            ],
        ];
        for f in faces {
            let q: Vec<V3> = f.iter().map(|&(x, y, z)| corner(x, y, z)).collect();
            self.quad(q[0], q[1], q[2], q[3], col);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meshes_are_well_formed() {
        for m in [
            pine(),
            broadleaf(),
            birch(),
            bush(),
            pine_far(),
            broadleaf_far(),
            rock(),
            house(),
            post(),
            reflector(),
            plank(),
        ] {
            assert!(m.vertex_count() > 0);
            assert_eq!(m.v.len() % (VERTEX_STRIDE * 3), 0);
            assert!(m.v.iter().all(|v| v.is_finite()));
        }
        for g in [grass_tuft(6, 0, false, 1), grass_tuft(4, 2, true, 2)] {
            assert!(!g.is_empty());
            assert_eq!(g.len() % (GRASS_STRIDE * 3), 0);
            assert!(g.iter().all(|v| v.is_finite()));
        }
        let pose = RiderPose {
            pos: [0.0; 3],
            fwd: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            right: [1.0, 0.0, 0.0],
            crank: 1.0,
            wheel: 0.4,
            jersey: [0.8, 0.2, 0.05],
        };
        let r = rider(&pose);
        assert!(r.v.iter().all(|v| v.is_finite()));
        // The rider stands roughly 1.6 m tall.
        let top =
            r.v.chunks(VERTEX_STRIDE)
                .map(|v| v[1])
                .fold(f32::MIN, f32::max);
        assert!((1.5..1.75).contains(&top), "{top}");
    }

    #[test]
    fn trees_keep_their_trunks_still() {
        // Only leaves flutter: trunks must not use the leaf material.
        for m in [pine(), broadleaf(), birch()] {
            let first = &m.v[..VERTEX_STRIDE];
            assert_eq!(first[9], MAT_MATT);
            assert!(m.v.chunks(VERTEX_STRIDE).any(|v| v[9] == MAT_LEAF));
        }
    }

    #[test]
    fn knee_ik_keeps_bone_lengths() {
        let k = ik(
            [0.0, 1.0, 0.0],
            [0.1, 0.2, 0.0],
            0.44,
            0.45,
            [1.0, 0.0, 0.0],
        );
        assert!((len(sub(k, [0.0, 1.0, 0.0])) - 0.44).abs() < 1e-3);
        assert!((len(sub(k, [0.1, 0.2, 0.0])) - 0.45).abs() < 1e-3);
        assert!(k[0] > 0.1, "knee bends forward");
    }
}
