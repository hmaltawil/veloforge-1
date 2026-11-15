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

/// One cross-section of a `Mesh::loft`: centre, a direction across it,
/// half width along that direction and half depth at right angles to it,
/// and the colour from here to the next ring.
#[derive(Clone, Copy)]
pub struct Ring {
    pub c: V3,
    pub side: V3,
    pub w: f32,
    pub d: f32,
    pub col: V3,
}

/// A place and three unit directions: forward, up and right.
#[derive(Clone, Copy)]
pub struct Frame {
    pub pos: V3,
    pub fwd: V3,
    pub up: V3,
    pub right: V3,
}

impl Frame {
    /// A point `x` forward, `y` up and `z` right of `pos`.
    pub fn at(&self, x: f32, y: f32, z: f32) -> V3 {
        add(self.pos, self.dir(x, y, z))
    }
    /// A direction given in this frame's terms.
    pub fn dir(&self, x: f32, y: f32, z: f32) -> V3 {
        add(
            add(scale(self.fwd, x), scale(self.up, y)),
            scale(self.right, z),
        )
    }
    /// Leaned `a` radians to the right about the forward axis through `pos`.
    pub fn rolled(&self, a: f32) -> Frame {
        let (s, c) = a.sin_cos();
        Frame {
            up: add(scale(self.up, c), scale(self.right, s)),
            right: sub(scale(self.right, c), scale(self.up, s)),
            ..*self
        }
    }
    fn axes(&self) -> [V3; 3] {
        [self.fwd, self.up, self.right]
    }
}

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
    /// Ellipsoid with its radii along `axes` (unit, at right angles; the
    /// second is its top), so it turns with whatever it belongs to.
    pub fn ellipsoid_in(
        &mut self,
        c: V3,
        axes: [V3; 3],
        r: V3,
        rings: usize,
        segs: usize,
        col: V3,
    ) {
        self.ellipsoid_shaded(c, axes, r, rings, segs, col, 0.0);
    }

    #[allow(clippy::too_many_arguments)]
    fn ellipsoid_shaded(
        &mut self,
        c: V3,
        axes: [V3; 3],
        r: V3,
        rings: usize,
        segs: usize,
        col: V3,
        jitter: f32,
    ) {
        let pt = |i: usize, j: usize| -> (V3, V3) {
            let th = i as f32 / rings as f32 * std::f32::consts::PI;
            let ph = j as f32 / segs as f32 * std::f32::consts::TAU;
            let d = [th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin()];
            let along = |k: usize, v: f32| scale(axes[k], v);
            let p = add(
                c,
                add(
                    add(along(0, d[0] * r[0]), along(1, d[1] * r[1])),
                    along(2, d[2] * r[2]),
                ),
            );
            let n = norm(add(
                add(along(0, d[0] / r[0]), along(1, d[1] / r[1])),
                along(2, d[2] / r[2]),
            ));
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

    /// A smooth body through `rings` (at least two, at different places):
    /// each ring is an ellipse across the line through them, and its colour
    /// covers the stretch to the next ring.
    pub fn loft(&mut self, rings: &[Ring], sides: usize) {
        let n = rings.len();
        if n < 2 {
            return;
        }
        // Each ring's directions across: along `side`, and at right angles.
        let across: Vec<(V3, V3)> = (0..n)
            .map(|i| {
                let t = norm(sub(
                    rings[(i + 1).min(n - 1)].c,
                    rings[i.saturating_sub(1)].c,
                ));
                let side = rings[i].side;
                let s = norm(sub(side, scale(t, dot(side, t))));
                (s, cross(t, s))
            })
            .collect();
        let pt = |i: usize, k: usize| -> (V3, V3) {
            let a = k as f32 / sides as f32 * std::f32::consts::TAU;
            let (ca, sa) = (a.cos(), a.sin());
            let (s, q) = across[i];
            let r = &rings[i];
            let p = add(r.c, add(scale(s, r.w * ca), scale(q, r.d * sa)));
            let nrm = norm(add(
                scale(s, ca / r.w.max(1e-4)),
                scale(q, sa / r.d.max(1e-4)),
            ));
            (p, nrm)
        };
        for (i, ring) in rings[..n - 1].iter().enumerate() {
            let col = ring.col;
            for k in 0..sides {
                let (a, na) = pt(i, k);
                let (b, nb) = pt(i, k + 1);
                let (c, nc) = pt(i + 1, k + 1);
                let (d, nd) = pt(i + 1, k);
                self.push(a, na, col);
                self.push(b, nb, col);
                self.push(c, nc, col);
                self.push(a, na, col);
                self.push(c, nc, col);
                self.push(d, nd, col);
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
    /// The right pedal's angle: 0 at the top, growing as the pedals turn
    /// forward. The left pedal is opposite.
    pub crank: f32,
    /// How far the wheels have turned, in radians.
    pub wheel: f32,
    pub jersey: V3,
    /// 0 seated .. 1 standing on the pedals.
    pub stand: f32,
    /// 0 .. 1 crouched low for a fast descent.
    pub tuck: f32,
    /// Power as a share of FTP: harder work, deeper breaths.
    pub effort: f32,
    /// Seconds, for breathing.
    pub time: f32,
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

/// The front hub, forward of the bottom bracket's ground point (the rear
/// hub is 0.5 m behind it).
const FRONT_HUB: f32 = 0.57;

/// Bone lengths, in metres.
const THIGH: f32 = 0.41;
const SHIN: f32 = 0.40;
const UPPER_ARM: f32 = 0.27;
const FOREARM: f32 = 0.26;

/// Rings along the straight line from `a` to `b`: (share of the way, half
/// width, half depth, colour).
fn limb(a: V3, b: V3, side: V3, rings: &[(f32, f32, f32, V3)]) -> Vec<Ring> {
    rings
        .iter()
        .map(|&(t, w, d, col)| Ring {
            c: lerp(a, b, t),
            side,
            w,
            d,
            col,
        })
        .collect()
}

/// Mix of three poses: seated, standing (`stand`) and tucked (`tuck`).
fn pose3(
    seated: [f32; 2],
    standing: [f32; 2],
    tucked: [f32; 2],
    stand: f32,
    tuck: f32,
) -> [f32; 2] {
    let mix =
        |k: usize| seated[k] + (standing[k] - seated[k]) * stand + (tucked[k] - seated[k]) * tuck;
    [mix(0), mix(1)]
}

/// A cyclist on a road bike, posed from the crank angle.
pub fn rider(p: &RiderPose) -> Mesh {
    use std::f32::consts::{PI, TAU};
    let mut m = Mesh::default();
    let stand = p.stand.clamp(0.0, 1.0);
    let tuck = p.tuck.clamp(0.0, 1.0) * (1.0 - stand);
    let body = Frame {
        pos: p.pos,
        fwd: p.fwd,
        up: p.up,
        right: p.right,
    };
    // Out of the saddle, riders rock the bike away from the pedal they push.
    let bike = body.rolled(-0.13 * stand * p.crank.sin());
    let at = |x: f32, y: f32, z: f32| bike.at(x, y, z);

    let rubber = [0.012, 0.012, 0.014];
    let frame = [0.028, 0.03, 0.036];
    let metal = [0.22, 0.22, 0.23];
    let skin = [0.5, 0.3, 0.2];
    let shorts = [0.012, 0.012, 0.016];
    let white = [0.78, 0.78, 0.78];
    let sole = [0.03, 0.03, 0.035];
    let jersey = p.jersey;
    let jersey_dark = scale(jersey, 0.5);

    // Wheels: tyre, rim, hub and spokes that turn with the road.
    for x in [-0.5f32, FRONT_HUB] {
        let hub = at(x, 0.34, 0.0);
        m.mat = MAT_MATT;
        m.torus(hub, bike.right, 0.326, 0.016, 28, 6, rubber);
        m.mat = MAT_GLOSS;
        m.torus(hub, bike.right, 0.296, 0.013, 28, 5, [0.06, 0.06, 0.065]);
        m.tube(
            at(x, 0.34, -0.035),
            at(x, 0.34, 0.035),
            0.018,
            0.018,
            6,
            metal,
        );
        for k in 0..10 {
            let a = p.wheel + k as f32 / 10.0 * TAU;
            let z = if k % 2 == 0 { 0.022 } else { -0.022 };
            let rim = at(x + a.sin() * 0.288, 0.34 + a.cos() * 0.288, 0.0);
            m.tube(
                add(hub, scale(bike.right, z)),
                rim,
                0.0045,
                0.0045,
                3,
                metal,
            );
        }
    }
    // Frame, fork and stays.
    m.mat = MAT_GLOSS;
    let bb = at(-0.02, 0.28, 0.0);
    let seat_top = at(-0.17, 0.82, 0.0);
    // The head tube lies along the steering axis (73 degrees), which
    // passes a fork offset behind the front hub; the fork crown and the
    // tubes stay clear of the tyre.
    let steer = |y: f32| at(FRONT_HUB - 0.047 - (y - 0.34) * 0.306, y, 0.0);
    let head_top = steer(0.86);
    let head_bot = steer(0.7);
    let tube = |m: &mut Mesh, a: V3, b: V3, r: f32| m.tube(a, b, r, r, 7, frame);
    tube(&mut m, bb, seat_top, 0.018);
    tube(&mut m, at(-0.155, 0.775, 0.0), steer(0.83), 0.017);
    tube(&mut m, bb, steer(0.74), 0.024);
    tube(&mut m, head_bot, head_top, 0.022);
    for z in [-0.045f32, 0.045] {
        tube(&mut m, bb, at(-0.5, 0.34, z), 0.012);
        tube(&mut m, at(-0.16, 0.76, 0.0), at(-0.5, 0.34, z), 0.009);
        m.tube(
            add(head_bot, scale(bike.right, z * 0.6)),
            at(FRONT_HUB, 0.34, z),
            0.014,
            0.009,
            7,
            frame,
        );
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
    m.ellipsoid_in(
        at(-0.2, 0.905, 0.0),
        bike.axes(),
        [0.13, 0.02, 0.065],
        3,
        7,
        rubber,
    );
    // Stem and drop handlebar with hoods.
    m.mat = MAT_GLOSS;
    let spacer = add(head_top, scale(bike.up, 0.015));
    m.tube(head_top, spacer, 0.018, 0.018, 7, [0.02, 0.02, 0.022]);
    m.tube(
        spacer,
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
        add(bb, scale(bike.right, 0.05)),
        0.095,
        bike.right,
        14,
        [0.05, 0.05, 0.055],
    );
    m.mat = MAT_LAMP;
    m.cuboid_at(
        at(-0.2, 0.83, 0.0),
        &bike,
        [0.012, 0.02, 0.02],
        [0.9, 0.03, 0.02],
    );
    m.cuboid_at(
        at(0.53, 0.84, 0.0),
        &bike,
        [0.015, 0.014, 0.02],
        [0.9, 0.85, 0.7],
    );
    m.mat = MAT_MATT;

    // Cranks, pedals and shoes. Ankling: the heel drops over the top of the
    // stroke and the toes point down round the bottom.
    let mut ankles = [[0.0; 3]; 2];
    for (k, side) in [1.0f32, -1.0].into_iter().enumerate() {
        let th = p.crank + if side > 0.0 { 0.0 } else { PI };
        let spindle = at(-0.02 + 0.17 * th.sin(), 0.28 + 0.17 * th.cos(), side * 0.1);
        m.tube(
            at(-0.02, 0.28, side * 0.07),
            spindle,
            0.011,
            0.009,
            5,
            metal,
        );
        m.cuboid_at(spindle, &bike, [0.045, 0.008, 0.04], rubber);
        let toe_down = (0.32 - 0.22 * (th - 0.6).cos()) * (1.0 - 0.3 * stand);
        // Heel to toe, and the top of the foot.
        let f = bike.dir(toe_down.cos(), -toe_down.sin(), 0.0);
        let n = bike.dir(toe_down.sin(), toe_down.cos(), 0.0);
        let ball = add(spindle, scale(n, 0.012));
        ankles[k] = add(ball, add(scale(f, -0.13), scale(n, 0.075)));
        let along = |u: f32, h: f32| add(ball, add(scale(f, u), scale(n, h)));
        let shoe: Vec<Ring> = [
            (-0.178, 0.022, 0.026, 0.04),
            (-0.168, 0.036, 0.042, 0.045),
            (-0.11, 0.04, 0.05, 0.052),
            (-0.03, 0.045, 0.038, 0.036),
            (0.04, 0.043, 0.03, 0.028),
            (0.085, 0.03, 0.02, 0.022),
            (0.1, 0.012, 0.01, 0.02),
        ]
        .iter()
        .map(|&(u, w, d, h)| Ring {
            c: along(u, h),
            side: bike.right,
            w,
            d,
            col: white,
        })
        .collect();
        m.mat = MAT_GLOSS;
        m.loft(&shoe, 9);
        m.mat = MAT_MATT;
        let sole_rings: Vec<Ring> = [(-0.17, 0.034), (0.0, 0.046), (0.09, 0.025)]
            .iter()
            .map(|&(u, w)| Ring {
                c: along(u, 0.006),
                side: bike.right,
                w,
                d: 0.007,
                col: sole,
            })
            .collect();
        m.loft(&sole_rings, 8);
    }

    // Hips: over the saddle, or forward over the pedals when standing,
    // swaying towards the pedal being pushed.
    let push = p.crank.sin();
    let hc = pose3([-0.13, 1.03], [-0.02, 1.06], [-0.16, 1.02], stand, tuck);
    let hips = body.at(hc[0], hc[1], (0.006 + 0.03 * stand) * push);
    // The hip of the straightening leg dips a little.
    let dip = 0.01 * p.crank.cos();
    for (k, side) in [1.0f32, -1.0].into_iter().enumerate() {
        let hip = add(hips, body.dir(0.0, side * dip, side * 0.09));
        let ankle = ankles[k];
        let knee = ik(
            hip,
            ankle,
            THIGH,
            SHIN,
            add(body.fwd, scale(body.right, side * 0.1)),
        );
        // The thigh starts inside the hips, and a rounded glute covers the
        // join, so no gap opens at the back as the knee comes up.
        let thigh = norm(sub(knee, hip));
        m.loft(
            &limb(
                sub(hip, scale(thigh, 0.07)),
                knee,
                body.right,
                &[
                    (0.0, 0.08, 0.084, shorts),
                    (0.15, 0.086, 0.09, shorts),
                    (0.42, 0.08, 0.086, shorts),
                    (0.75, 0.064, 0.07, skin),
                    (0.79, 0.061, 0.066, skin),
                    (1.0, 0.052, 0.055, skin),
                ],
            ),
            10,
        );
        m.ellipsoid_in(
            add(hips, body.dir(-0.03, -0.03, side * 0.075)),
            body.axes(),
            [0.088, 0.08, 0.085],
            5,
            9,
            shorts,
        );
        // The calf bulges behind the shin.
        let shin = norm(sub(ankle, knee));
        let back = norm(sub(scale(shin, dot(body.fwd, shin)), body.fwd));
        let mut calf = limb(
            knee,
            ankle,
            body.right,
            &[
                (0.0, 0.05, 0.052, skin),
                (0.14, 0.05, 0.058, skin),
                (0.32, 0.05, 0.064, skin),
                (0.62, 0.039, 0.044, white),
                (0.75, 0.036, 0.04, white),
                (1.0, 0.032, 0.035, white),
            ],
        );
        for (r, b) in calf.iter_mut().zip([0.0, 0.008, 0.014, 0.004, 0.0, 0.0]) {
            r.c = add(r.c, scale(back, b));
        }
        m.loft(&calf, 10);
        m.ellipsoid_in(knee, body.axes(), [0.052, 0.052, 0.05], 4, 7, skin);
    }

    // Torso: an arched back from the hips to the shoulders, breathing.
    let sc = pose3([0.26, 1.33], [0.38, 1.38], [0.3, 1.21], stand, tuck);
    let sway = -0.008 * push;
    let shoulders = body.at(sc[0], sc[1], sway);
    let pelvis = add(hips, body.dir(-0.02, 0.0, 0.0));
    let back_dir = norm(sub(shoulders, pelvis));
    let arch = norm(cross(body.right, back_dir));
    let ctrl = add(lerp(pelvis, shoulders, 0.5), scale(arch, 0.06));
    let spine = |t: f32| {
        let a = lerp(pelvis, ctrl, t);
        let b = lerp(ctrl, shoulders, t);
        lerp(a, b, t)
    };
    let rate = 0.25 + 0.2 * p.effort.clamp(0.0, 2.0);
    let breath = (0.012 + 0.012 * p.effort.clamp(0.0, 2.0)) * (p.time * rate * TAU).sin();
    let chest = 1.0 + breath;
    let ring = |t: f32, w: f32, d: f32, col: V3| Ring {
        c: spine(t),
        side: body.right,
        w,
        d,
        col,
    };
    let torso = [
        Ring {
            c: add(pelvis, add(scale(arch, -0.05), scale(back_dir, -0.03))),
            side: body.right,
            w: 0.1,
            d: 0.07,
            col: shorts,
        },
        // Narrow at the waist, broad across the back and shoulders.
        ring(0.0, 0.16, 0.112, shorts),
        ring(0.12, 0.148, 0.104, jersey_dark),
        ring(0.3, 0.14, 0.1, jersey),
        ring(0.5, 0.165 * chest, 0.115 * chest, jersey),
        ring(0.68, 0.19 * chest, 0.118 * chest, white),
        ring(0.76, 0.2 * chest, 0.114 * chest, jersey),
        ring(0.9, 0.205, 0.095, jersey),
        ring(1.0, 0.195, 0.07, jersey),
        Ring {
            c: add(shoulders, scale(back_dir, 0.05)),
            side: body.right,
            w: 0.065,
            d: 0.055,
            col: skin,
        },
    ];
    m.loft(&torso, 14);

    // Arms, from the shoulders to the hands on the brake hoods.
    for side in [1.0f32, -1.0] {
        let sh = add(shoulders, body.dir(0.0, -0.025, side * 0.2));
        m.ellipsoid_in(sh, body.axes(), [0.06, 0.062, 0.055], 5, 8, jersey);
        let grip = at(0.59, 0.89, side * 0.2);
        let wrist = add(grip, bike.dir(-0.045, 0.035, 0.0));
        let elbow = ik(
            sh,
            wrist,
            UPPER_ARM,
            FOREARM,
            body.dir(-0.35 + 0.2 * tuck, -0.5, side * 0.55),
        );
        m.loft(
            &limb(
                sh,
                elbow,
                body.fwd,
                &[
                    (0.0, 0.05, 0.05, jersey),
                    (0.44, 0.046, 0.047, jersey_dark),
                    (0.52, 0.044, 0.045, skin),
                    (1.0, 0.036, 0.038, skin),
                ],
            ),
            9,
        );
        m.loft(
            &limb(
                elbow,
                wrist,
                body.fwd,
                &[
                    (0.0, 0.037, 0.037, skin),
                    (0.3, 0.042, 0.04, skin),
                    (1.0, 0.029, 0.022, skin),
                ],
            ),
            9,
        );
        m.ellipsoid_in(elbow, body.axes(), [0.037, 0.037, 0.037], 3, 6, skin);
        // A gloved hand wrapped round the hood.
        m.ellipsoid_in(
            add(grip, bike.dir(-0.01, 0.01, 0.0)),
            bike.axes(),
            [0.055, 0.04, 0.042],
            4,
            7,
            rubber,
        );
    }

    // Neck and head, held level to look up the road.
    let hd = pose3([0.13, 0.15], [0.13, 0.14], [0.16, 0.12], stand, tuck);
    let head = body.at(sc[0] + hd[0], sc[1] + hd[1], 0.0);
    let hf = norm(body.dir(1.0, 0.12, 0.0));
    let hu = cross(body.right, hf);
    let head_axes = [hf, hu, body.right];
    let neck_base = add(shoulders, scale(back_dir, 0.02));
    m.loft(
        &limb(
            neck_base,
            add(head, scale(hu, -0.04)),
            body.right,
            &[(0.0, 0.055, 0.06, skin), (1.0, 0.048, 0.052, skin)],
        ),
        8,
    );
    m.ellipsoid_in(head, head_axes, [0.095, 0.108, 0.078], 6, 10, skin);
    m.ellipsoid_in(
        add(head, add(scale(hf, 0.035), scale(hu, -0.05))),
        head_axes,
        [0.075, 0.062, 0.068],
        4,
        8,
        skin,
    );
    m.ellipsoid_in(
        add(head, add(scale(hf, 0.098), scale(hu, -0.015))),
        head_axes,
        [0.022, 0.025, 0.016],
        3,
        5,
        skin,
    );
    // Helmet with vents and a tail, and sunglasses.
    let on_head = |f: f32, u: f32, r: f32| {
        add(
            head,
            add(scale(hf, f), add(scale(hu, u), scale(body.right, r))),
        )
    };
    let dark = [0.02, 0.02, 0.022];
    let lens = [0.015, 0.015, 0.02];
    // (forward, up, right of the head's centre), radii, colour.
    let parts: [((f32, f32, f32), V3, V3); 7] = [
        // A snug shell a couple of centimetres over the head.
        ((-0.01, 0.05, 0.0), [0.128, 0.078, 0.094], white),
        ((-0.09, 0.03, 0.0), [0.06, 0.045, 0.07], white),
        ((0.0, 0.118, -0.038), [0.085, 0.014, 0.01], dark),
        ((0.0, 0.121, 0.0), [0.085, 0.014, 0.01], dark),
        ((0.0, 0.118, 0.038), [0.085, 0.014, 0.01], dark),
        ((0.0, 0.1, 0.0), [0.1, 0.032, 0.028], jersey),
        ((0.085, 0.002, 0.0), [0.022, 0.024, 0.078], lens),
    ];
    m.mat = MAT_GLOSS;
    for ((f, u, r), radii, col) in parts {
        m.ellipsoid_in(on_head(f, u, r), head_axes, radii, 5, 9, col);
    }
    m.mat = MAT_MATT;
    m
}

impl Mesh {
    /// Small box centred on `c`, aligned with `p`.
    fn cuboid_at(&mut self, c: V3, p: &Frame, half: V3, col: V3) {
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
            stand: 0.0,
            tuck: 0.0,
            effort: 1.0,
            time: 3.0,
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

    fn pose(crank: f32, stand: f32, tuck: f32) -> RiderPose {
        RiderPose {
            pos: [0.0; 3],
            fwd: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            right: [1.0, 0.0, 0.0],
            crank,
            wheel: 0.0,
            jersey: [0.8, 0.2, 0.05],
            stand,
            tuck,
            effort: 1.0,
            time: 0.0,
        }
    }

    /// Where the right shoe's lowest point is, forward of the bottom bracket.
    fn right_foot_forward(crank: f32) -> f32 {
        let r = rider(&pose(crank, 0.0, 0.0));
        // The shoes are the only white parts below the wheel hubs on the
        // right of the bike (x > 0.07).
        let feet: Vec<&[f32]> =
            r.v.chunks(VERTEX_STRIDE)
                .filter(|v| v[1] < 0.6 && v[0] > 0.07 && v[6] > 0.7 && v[9] == MAT_GLOSS)
                .collect();
        assert!(!feet.is_empty());
        -feet.iter().map(|v| v[2]).sum::<f32>() / feet.len() as f32
    }

    #[test]
    fn pedals_turn_forwards() {
        // Just past the top the right foot comes forward, and just past
        // the bottom it goes back, as the pedals turn the way the wheels do.
        assert!(right_foot_forward(0.3) > right_foot_forward(0.0));
        assert!(
            right_foot_forward(std::f32::consts::PI + 0.3)
                < right_foot_forward(std::f32::consts::PI)
        );
    }

    #[test]
    fn every_pose_is_well_formed() {
        for (stand, tuck) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (0.5, 0.5)] {
            for k in 0..8 {
                let r = rider(&pose(k as f32, stand, tuck));
                assert_eq!(r.v.len() % (VERTEX_STRIDE * 3), 0);
                assert!(r.v.iter().all(|v| v.is_finite()), "{stand} {tuck} {k}");
            }
        }
    }

    #[test]
    fn ellipsoids_turn_with_their_axes() {
        // Long along the first axis, wherever that points.
        let mut m = Mesh::default();
        let axes = [[0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]];
        m.ellipsoid_in([0.0; 3], axes, [0.3, 0.1, 0.05], 6, 12, [1.0; 3]);
        let extent = |k: usize| {
            m.v.chunks(VERTEX_STRIDE)
                .map(|v| v[k].abs())
                .fold(0.0f32, f32::max)
        };
        assert!((extent(2) - 0.3).abs() < 0.01);
        assert!((extent(1) - 0.1).abs() < 0.01);
        assert!((extent(0) - 0.05).abs() < 0.01);
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
