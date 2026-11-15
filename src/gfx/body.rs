//! The rider's body: one continuous, sculpted surface, made once and bent
//! by a skeleton on the graphics card.
//!
//! The body is sculpted as a signed distance field: anatomical forms (rib
//! cage, shoulder girdle, deltoids, quads, hamstrings, the two heads of the
//! calf, a face with jaw, nose and ears) blended into one another the way
//! clay is smoothed together, with the kit, shoes, gloves, helmet and
//! glasses on top. A surface net turns it into a single smooth mesh with
//! no seams; each vertex is painted from where it lies on the body (sleeve
//! hems, jersey panels and pockets, bib shorts and leg grippers, socks...)
//! and tied to the bones nearest it. Each frame only the 17 bones move,
//! following the pose in `mesh::joints`.

use super::math::*;
use super::mesh::{joints, Joints, RiderPose, MAT_GLOSS};

pub const BONES: usize = 17;
pub const PELVIS: usize = 0;
pub const SPINE: usize = 1;
pub const CHEST: usize = 2;
pub const NECK: usize = 3;
pub const HEAD: usize = 4;
/// Per side, right then left.
pub const THIGH: [usize; 2] = [5, 8];
pub const SHIN: [usize; 2] = [6, 9];
pub const FOOT: [usize; 2] = [7, 10];
pub const UPPER_ARM: [usize; 2] = [11, 14];
pub const FOREARM: [usize; 2] = [12, 15];
pub const HAND: [usize; 2] = [13, 16];

/// Skin and fabric get their own shading in `OBJECT_FS`.
pub const MAT_SKIN: f32 = 6.0;
pub const MAT_FABRIC: f32 = 7.0;

/// Floats per vertex: position, normal, colour, material, four bone
/// indices and four weights.
pub const STRIDE: usize = 18;

/// What a part of the body is covered in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paint {
    Skin,
    Hair,
    Jersey,
    JerseyDark,
    Accent,
    Zip,
    Shorts,
    Gripper,
    Sock,
    Shoe,
    Sole,
    Dial,
    Glove,
    Helmet,
    Vent,
    HelmetAccent,
    Lens,
}

impl Paint {
    /// Colour and material. Negative colours are multiples of the jersey
    /// colour, filled in when drawn.
    fn look(self) -> (V3, f32) {
        let fabric = MAT_FABRIC;
        match self {
            Paint::Skin => ([0.56, 0.34, 0.23], MAT_SKIN),
            Paint::Hair => ([0.045, 0.032, 0.024], fabric),
            Paint::Jersey => ([-1.0, -1.0, -1.0], fabric),
            Paint::JerseyDark => ([-0.42, -0.42, -0.42], fabric),
            Paint::Accent => ([0.82, 0.82, 0.82], fabric),
            Paint::Zip => ([0.03, 0.03, 0.035], MAT_GLOSS),
            Paint::Shorts => ([0.014, 0.014, 0.018], fabric),
            Paint::Gripper => ([0.06, 0.06, 0.066], fabric),
            Paint::Sock => ([0.82, 0.82, 0.82], fabric),
            Paint::Shoe => ([0.86, 0.86, 0.87], MAT_GLOSS),
            Paint::Sole => ([0.02, 0.02, 0.022], 0.0),
            Paint::Dial => ([0.03, 0.03, 0.034], MAT_GLOSS),
            Paint::Glove => ([0.022, 0.022, 0.026], fabric),
            Paint::Helmet => ([0.86, 0.86, 0.88], MAT_GLOSS),
            Paint::Vent => ([0.02, 0.02, 0.024], 0.0),
            Paint::HelmetAccent => ([-1.0, -1.0, -1.0], MAT_GLOSS),
            Paint::Lens => ([0.012, 0.014, 0.024], MAT_GLOSS),
        }
    }
}

// ---------------------------------------------------------------------
// Shapes

#[derive(Debug, Clone, Copy)]
enum Shape {
    /// A rounded cone from `a` (radius `ra`) to `b` (radius `rb`).
    Cone {
        a: V3,
        b: V3,
        ra: f32,
        rb: f32,
    },
    Ellipsoid {
        c: V3,
        axes: [V3; 3],
        r: V3,
    },
    /// A box with rounded edges.
    Box {
        c: V3,
        axes: [V3; 3],
        half: V3,
        round: f32,
    },
    /// A slot cut a few millimetres into an ellipsoidal shell: between the
    /// ellipsoids `outer` and `inner` (same centre and axes), within `w`
    /// of the plane `side` across, and from `from` to `to` along the first
    /// axis.
    Slot {
        c: V3,
        axes: [V3; 3],
        outer: V3,
        inner: V3,
        side: f32,
        w: f32,
        from: f32,
        to: f32,
    },
}

/// The approximate distance to an ellipsoid with radii `r`, from `q` in
/// its own axes.
fn ellipsoid_dist(q: V3, r: V3) -> f32 {
    let q0 = [q[0] / r[0], q[1] / r[1], q[2] / r[2]];
    let q1 = [q0[0] / r[0], q0[1] / r[1], q0[2] / r[2]];
    let k0 = len(q0);
    let k1 = len(q1).max(1e-9);
    k0 * (k0 - 1.0) / k1
}

fn local(p: V3, c: V3, axes: &[V3; 3]) -> V3 {
    let d = sub(p, c);
    [dot(d, axes[0]), dot(d, axes[1]), dot(d, axes[2])]
}

impl Shape {
    fn dist(&self, p: V3) -> f32 {
        match *self {
            Shape::Cone { a, b, ra, rb } => {
                let ba = sub(b, a);
                let t = (dot(sub(p, a), ba) / dot(ba, ba).max(1e-9)).clamp(0.0, 1.0);
                len(sub(p, add(a, scale(ba, t)))) - (ra + (rb - ra) * t)
            }
            Shape::Ellipsoid { c, axes, r } => ellipsoid_dist(local(p, c, &axes), r),
            Shape::Slot {
                c,
                axes,
                outer,
                inner,
                side,
                w,
                from,
                to,
            } => {
                let q = local(p, c, &axes);
                let band = ellipsoid_dist(q, outer).max(-ellipsoid_dist(q, inner));
                band.max((q[2] - side).abs() - w)
                    .max(from - q[0])
                    .max(q[0] - to)
            }
            Shape::Box {
                c,
                axes,
                half,
                round,
            } => {
                let q = local(p, c, &axes);
                let d = [
                    q[0].abs() - half[0],
                    q[1].abs() - half[1],
                    q[2].abs() - half[2],
                ];
                let out = len([d[0].max(0.0), d[1].max(0.0), d[2].max(0.0)]);
                out + d[0].max(d[1]).max(d[2]).min(0.0) - round
            }
        }
    }

    /// A box around the shape, grown by `m`.
    fn bounds(&self, m: f32) -> (V3, V3) {
        let (c, r) = match *self {
            Shape::Cone { a, b, ra, rb } => {
                let r = ra.max(rb);
                let lo = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
                let hi = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
                return (
                    [lo[0] - r - m, lo[1] - r - m, lo[2] - r - m],
                    [hi[0] + r + m, hi[1] + r + m, hi[2] + r + m],
                );
            }
            Shape::Ellipsoid { c, r, .. } => (c, r[0].max(r[1]).max(r[2])),
            Shape::Box { c, half, round, .. } => (c, len(half) + round),
            Shape::Slot { c, outer, .. } => (c, outer[0].max(outer[1]).max(outer[2])),
        };
        let r = r + m;
        (
            [c[0] - r, c[1] - r, c[2] - r],
            [c[0] + r, c[1] + r, c[2] + r],
        )
    }
}

/// Polynomial smooth minimum: blends two surfaces within `k` of each other.
fn smin(a: f32, b: f32, k: f32) -> f32 {
    if k <= 0.0 {
        return a.min(b);
    }
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.min(b) - h * h * k * 0.25
}

/// How a shape's surface is painted, from a point on it.
type Painter = Box<dyn Fn(V3) -> Paint + Send + Sync>;

/// Bones a part may follow: limbs follow only their own chain, so the
/// right leg never pulls on the left.
type Region = &'static [usize];

struct Part {
    shape: Shape,
    /// Blend with the rest of its group within this distance.
    k: f32,
    paint: Painter,
    region: Region,
    /// Follow this one bone rigidly (shoes, gloves, head and helmet).
    rigid: Option<usize>,
    lo: V3,
    hi: V3,
}

/// Shapes blended together; groups are joined without blending (the
/// helmet sits on the head, it does not melt into it).
struct Group {
    parts: Vec<Part>,
    /// Grooves cut into the group (helmet vents).
    cuts: Vec<Shape>,
    /// Half-spaces the group is clipped to: keep where `dot(p - o, n) < 0`.
    clips: Vec<(V3, V3)>,
}

const FAR: f32 = 1.0;

impl Group {
    fn dist(&self, p: V3) -> (f32, usize, f32) {
        let mut d = FAR;
        let mut nearest = (usize::MAX, f32::MAX);
        for (i, part) in self.parts.iter().enumerate() {
            if p[0] < part.lo[0]
                || p[1] < part.lo[1]
                || p[2] < part.lo[2]
                || p[0] > part.hi[0]
                || p[1] > part.hi[1]
                || p[2] > part.hi[2]
            {
                continue;
            }
            let di = part.shape.dist(p);
            d = smin(d, di, part.k);
            if di.abs() < nearest.1 {
                nearest = (i, di.abs());
            }
        }
        for c in &self.cuts {
            d = d.max(-c.dist(p));
        }
        for (o, n) in &self.clips {
            d = d.max(dot(sub(p, *o), *n));
        }
        (d, nearest.0, nearest.1)
    }
}

/// The sculpture: its groups, and each bone's segment for weighting.
pub struct Sculpt {
    groups: Vec<Group>,
    bones: [(V3, V3); BONES],
}

impl Sculpt {
    /// Distance to the body's surface (negative inside).
    pub fn dist(&self, p: V3) -> f32 {
        self.groups.iter().map(|g| g.dist(p).0).fold(FAR, f32::min)
    }

    /// The part whose surface `p` lies on.
    fn part_at(&self, p: V3) -> Option<&Part> {
        let mut best: Option<(&Part, f32)> = None;
        for g in &self.groups {
            let (d, i, _) = g.dist(p);
            if i == usize::MAX {
                continue;
            }
            if best.is_none_or(|b| d.abs() < b.1) {
                best = Some((&g.parts[i], d.abs()));
            }
        }
        best.map(|b| b.0)
    }

    fn normal(&self, p: V3) -> V3 {
        let e = 0.0008;
        let d = |o: V3| self.dist(add(p, o));
        norm([
            d([e, 0.0, 0.0]) - d([-e, 0.0, 0.0]),
            d([0.0, e, 0.0]) - d([0.0, -e, 0.0]),
            d([0.0, 0.0, e]) - d([0.0, 0.0, -e]),
        ])
    }

    /// Soft shadowing from nearby surfaces: armpits, between the legs,
    /// under the helmet's rim.
    fn occlusion(&self, p: V3, n: V3) -> f32 {
        let mut occ = 0.0;
        let mut w = 1.0;
        for i in 1..=4 {
            let h = 0.012 * i as f32;
            occ += w * (h - self.dist(add(p, scale(n, h)))).max(0.0) / h;
            w *= 0.55;
        }
        (1.0 - 0.5 * occ).clamp(0.35, 1.0)
    }

    /// Bone indices and weights for a vertex of `part` at `p`.
    fn weights(&self, part: &Part, p: V3) -> ([f32; 4], [f32; 4]) {
        if let Some(b) = part.rigid {
            return ([b as f32, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]);
        }
        let mut near: Vec<(usize, f32)> = part
            .region
            .iter()
            .map(|&b| {
                let (a, c) = self.bones[b];
                let ba = sub(c, a);
                let along = dot(sub(p, a), ba) / dot(ba, ba).max(1e-9);
                let t = along.clamp(0.0, 1.0);
                let d = len(sub(p, add(a, scale(ba, t))));
                // Skin in front of the shoulder and hip joints (the top of
                // the shoulder, the side of the hip) belongs to the torso,
                // not to the limb: it must not swing with the arm or leg.
                let limb = UPPER_ARM.contains(&b) || THIGH.contains(&b);
                let behind = if limb {
                    (-along).max(0.0) * len(ba)
                } else {
                    0.0
                };
                (b, d + 1.5 * behind)
            })
            .collect();
        near.sort_by(|a, b| a.1.total_cmp(&b.1));
        let d0 = near[0].1;
        // Blend over a few centimetres around each joint.
        let mut out: Vec<(usize, f32)> = near
            .iter()
            .take(4)
            .map(|&(b, d)| (b, (-(d - d0) / 0.022).exp()))
            .filter(|&(_, w)| w > 0.02)
            .collect();
        let sum: f32 = out.iter().map(|x| x.1).sum();
        for x in &mut out {
            x.1 /= sum;
        }
        let mut idx = [0.0; 4];
        let mut wt = [0.0; 4];
        for (i, (b, w)) in out.into_iter().enumerate() {
            idx[i] = b as f32;
            wt[i] = w;
        }
        (idx, wt)
    }
}

// ---------------------------------------------------------------------
// The skeleton

/// Each bone's segment (from its joint to the next) in the pose `j`, and
/// the direction its frame keeps to the side.
fn segments(j: &Joints) -> [(V3, V3, V3); BONES] {
    let side = j.body.right;
    let bike_side = j.bike.right;
    let [hf, hu, _] = j.head_axes;
    let toe = |k: usize| add(j.ball[k], scale(j.foot_fwd[k], 0.09));
    let mut s = [([0.0; 3], [0.0; 3], side); BONES];
    s[PELVIS] = (j.hips, j.spine(0.3), side);
    s[SPINE] = (j.spine(0.3), j.spine(0.62), side);
    s[CHEST] = (j.spine(0.62), j.shoulders, side);
    s[NECK] = (j.neck_base, sub(j.head, scale(hu, 0.05)), side);
    s[HEAD] = (j.head, add(j.head, scale(hf, 0.1)), side);
    for k in 0..2 {
        s[THIGH[k]] = (j.hip[k], j.knee[k], side);
        s[SHIN[k]] = (j.knee[k], j.ankle[k], side);
        s[FOOT[k]] = (j.ankle[k], toe(k), bike_side);
        s[UPPER_ARM[k]] = (j.shoulder[k], j.elbow[k], side);
        s[FOREARM[k]] = (j.elbow[k], j.wrist[k], side);
        s[HAND[k]] = (j.wrist[k], j.grip[k], bike_side);
    }
    s
}

/// Each bone's frame in the pose `j`: origin at its joint, x along it.
/// The chest also breathes.
pub fn frames(j: &Joints) -> [M4; BONES] {
    let segs = segments(j);
    let mut out = [[0.0; 16]; BONES];
    for (b, &(a, c, side)) in segs.iter().enumerate() {
        let (x, y, z) = if b == HEAD {
            (j.head_axes[0], j.head_axes[1], j.head_axes[2])
        } else {
            let x = norm(sub(c, a));
            let z = norm(sub(side, scale(x, dot(side, x))));
            (x, cross(z, x), z)
        };
        let (sy, sz) = if b == CHEST {
            (j.chest, j.chest)
        } else {
            (1.0, 1.0)
        };
        out[b] = [
            x[0],
            x[1],
            x[2],
            0.0,
            y[0] * sy,
            y[1] * sy,
            y[2] * sy,
            0.0,
            z[0] * sz,
            z[1] * sz,
            z[2] * sz,
            0.0,
            a[0],
            a[1],
            a[2],
            1.0,
        ];
    }
    out
}

/// Inverse of a frame with orthonormal axes.
fn rigid_inverse(m: &M4) -> M4 {
    let x = [m[0], m[1], m[2]];
    let y = [m[4], m[5], m[6]];
    let z = [m[8], m[9], m[10]];
    let o = [m[12], m[13], m[14]];
    [
        x[0],
        y[0],
        z[0],
        0.0,
        x[1],
        y[1],
        z[1],
        0.0,
        x[2],
        y[2],
        z[2],
        0.0,
        -dot(x, o),
        -dot(y, o),
        -dot(z, o),
        1.0,
    ]
}

/// The pose the body is sculpted in: seated, pedals level, in the body's
/// own space (x forward, y up, z right, the ground under the bottom
/// bracket at the origin).
pub fn rest_pose() -> RiderPose {
    RiderPose {
        pos: [0.0; 3],
        fwd: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        right: [0.0, 0.0, 1.0],
        crank: std::f32::consts::FRAC_PI_2,
        wheel: 0.0,
        jersey: [1.0; 3],
        stand: 0.0,
        tuck: 0.0,
        effort: 0.0,
        time: 0.0,
    }
}

/// Each bone's rest frame, inverted (what `build` puts in `Skin`).
pub fn rest_inverse() -> [M4; BONES] {
    frames(&joints(&rest_pose())).map(|m| rigid_inverse(&m))
}

/// The matrices that bend the sculpted body into the pose `j`: each bone's
/// frame now, after the inverse of its frame at rest.
pub fn skinning(rest_inverse: &[M4; BONES], j: &Joints) -> [M4; BONES] {
    let now = frames(j);
    let mut out = [[0.0; 16]; BONES];
    for b in 0..BONES {
        out[b] = mul(&now[b], &rest_inverse[b]);
    }
    out
}

// ---------------------------------------------------------------------
// The sculpture

fn part(shape: Shape, k: f32, paint: Painter, region: Region) -> Part {
    let (lo, hi) = shape.bounds(k + 0.03);
    Part {
        shape,
        k,
        paint,
        region,
        rigid: None,
        lo,
        hi,
    }
}

fn rigid(shape: Shape, k: f32, paint: Painter, bone: usize) -> Part {
    Part {
        rigid: Some(bone),
        ..part(shape, k, paint, &[])
    }
}

fn solid(p: Paint) -> Painter {
    Box::new(move |_| p)
}

/// Paint in bands along the segment `a`-`b`: `bands` are (up to this share
/// of the way, paint).
fn bands(a: V3, b: V3, bands: &'static [(f32, Paint)]) -> Painter {
    Box::new(move |p| {
        let ba = sub(b, a);
        let t = dot(sub(p, a), ba) / dot(ba, ba).max(1e-9);
        bands
            .iter()
            .find(|(end, _)| t <= *end)
            .map(|x| x.1)
            .unwrap_or(bands[bands.len() - 1].1)
    })
}

fn cone(a: V3, b: V3, ra: f32, rb: f32) -> Shape {
    Shape::Cone { a, b, ra, rb }
}

fn ell(c: V3, axes: [V3; 3], r: V3) -> Shape {
    Shape::Ellipsoid { c, axes, r }
}

/// Unit axes along `along`, with the second towards `towards`.
fn axes_along(along: V3, towards: V3) -> [V3; 3] {
    let x = norm(along);
    let y = norm(sub(towards, scale(x, dot(towards, x))));
    [x, y, cross(x, y)]
}

/// The rider sculpted in the pose `j`.
pub fn sculpt(j: &Joints) -> Sculpt {
    const LEG: [&[usize]; 2] = [&[PELVIS, 5, 6], &[PELVIS, 8, 9]];
    const LOWER_LEG: [&[usize]; 2] = [&[5, 6, 7], &[8, 9, 10]];
    const ARM: [&[usize]; 2] = [&[CHEST, 11, 12, 13], &[CHEST, 14, 15, 16]];
    const TORSO: &[usize] = &[PELVIS, SPINE, CHEST, NECK, 11, 14, 5, 8];
    const GLUTE: [&[usize]; 2] = [&[PELVIS, SPINE, 5], &[PELVIS, SPINE, 8]];
    const NECK_REGION: &[usize] = &[CHEST, NECK, HEAD];

    let z = j.body.right;
    let up = j.body.up;
    let fwd = j.body.fwd;
    let back = j.arch;
    let along = j.back_dir;
    let torso_axes = [along, back, z];
    let at = |t: f32, out: f32, side: f32| add(j.spine(t), add(scale(back, out), scale(z, side)));

    let mut body = Group {
        parts: Vec::new(),
        cuts: Vec::new(),
        clips: Vec::new(),
    };
    let mut add_part = |p: Part| body.parts.push(p);

    // --- torso ---------------------------------------------------------
    // Every torso shape shares the torso's painting.
    let jj = *j;
    let tp = || -> Painter { Box::new(move |p| torso_paint_fn(&jj, p)) };
    // Hips and lower back, waist, rib cage, shoulder girdle.
    add_part(part(
        ell(at(0.04, -0.008, 0.0), torso_axes, [0.11, 0.088, 0.145]),
        0.05,
        tp(),
        TORSO,
    ));
    add_part(part(
        ell(at(0.32, 0.0, 0.0), torso_axes, [0.15, 0.092, 0.135]),
        0.06,
        tp(),
        TORSO,
    ));
    add_part(part(
        ell(at(0.62, 0.0, 0.0), torso_axes, [0.17, 0.112, 0.15]),
        0.06,
        tp(),
        TORSO,
    ));
    add_part(part(
        ell(at(0.88, 0.01, 0.0), torso_axes, [0.1, 0.085, 0.165]),
        0.05,
        tp(),
        TORSO,
    ));
    // Lats down the sides of the back, chest and belly in front, the
    // trapezius sloping up to the neck.
    for s in [1.0f32, -1.0] {
        add_part(part(
            ell(at(0.64, 0.035, s * 0.105), torso_axes, [0.15, 0.06, 0.06]),
            0.05,
            tp(),
            TORSO,
        ));
        add_part(part(
            ell(
                at(0.74, -0.065, s * 0.07),
                torso_axes,
                [0.065, 0.045, 0.072],
            ),
            0.04,
            tp(),
            TORSO,
        ));
        add_part(part(
            cone(
                at(0.93, 0.035, s * 0.125),
                add(
                    add(j.neck_base, scale(along, 0.035)),
                    add(scale(back, 0.012), scale(z, s * 0.032)),
                ),
                0.042,
                0.036,
            ),
            0.05,
            tp(),
            TORSO,
        ));
    }
    add_part(part(
        ell(at(0.42, -0.05, 0.0), torso_axes, [0.12, 0.06, 0.11]),
        0.05,
        tp(),
        TORSO,
    ));
    // Glutes, in the shorts.
    for (k, s) in [1.0f32, -1.0].into_iter().enumerate() {
        add_part(part(
            ell(
                add(j.hips, j.body.dir(-0.045, -0.025, s * 0.068)),
                [fwd, up, z],
                [0.064, 0.066, 0.064],
            ),
            0.045,
            solid(Paint::Shorts),
            GLUTE[k],
        ));
    }

    // --- legs ----------------------------------------------------------
    const THIGH_BANDS: &[(f32, Paint)] = &[
        (0.6, Paint::Shorts),
        (0.66, Paint::Gripper),
        (9.0, Paint::Skin),
    ];
    const SHIN_BANDS: &[(f32, Paint)] = &[(0.58, Paint::Skin), (9.0, Paint::Sock)];
    for (k, s) in [1.0f32, -1.0].into_iter().enumerate() {
        let (hip, knee, ankle) = (j.hip[k], j.knee[k], j.ankle[k]);
        let u = norm(sub(knee, hip));
        let front = norm(cross(z, u));
        let ax = [u, front, z];
        let on =
            |t: f32, f: f32, sd: f32| add(lerp(hip, knee, t), add(scale(front, f), scale(z, sd)));
        let thigh_paint = || bands(hip, knee, THIGH_BANDS);
        add_part(part(
            cone(sub(hip, scale(u, 0.035)), knee, 0.076, 0.051),
            0.04,
            thigh_paint(),
            LEG[k],
        ));
        // Quads, the teardrop above the knee, hamstrings.
        add_part(part(
            ell(on(0.52, 0.026, 0.004 * s), ax, [0.18, 0.058, 0.066]),
            0.04,
            thigh_paint(),
            LEG[k],
        ));
        add_part(part(
            ell(on(0.8, 0.012, -0.024 * s), ax, [0.07, 0.038, 0.042]),
            0.03,
            thigh_paint(),
            LEG[k],
        ));
        add_part(part(
            ell(on(0.5, -0.03, 0.0), ax, [0.18, 0.05, 0.056]),
            0.04,
            thigh_paint(),
            LEG[k],
        ));
        // Knee, shin, the two heads of the calf, the ankle.
        let v = norm(sub(ankle, knee));
        let sfront = norm(cross(z, v));
        let sax = [v, sfront, z];
        let son = |t: f32, f: f32, sd: f32| {
            add(lerp(knee, ankle, t), add(scale(sfront, f), scale(z, sd)))
        };
        let shin_paint = || bands(knee, ankle, SHIN_BANDS);
        add_part(part(
            ell(add(knee, scale(front, 0.006)), ax, [0.05, 0.048, 0.048]),
            0.03,
            solid(Paint::Skin),
            LOWER_LEG[k],
        ));
        add_part(part(
            cone(knee, ankle, 0.047, 0.03),
            0.035,
            shin_paint(),
            LOWER_LEG[k],
        ));
        for (head, off) in [(1.0f32, 0.02f32), (-1.0, 0.016)] {
            add_part(part(
                ell(son(0.3, -0.028, head * off * s), sax, [0.12, 0.042, 0.033]),
                0.035,
                shin_paint(),
                LOWER_LEG[k],
            ));
        }
        add_part(part(
            ell(ankle, sax, [0.03, 0.03, 0.034]),
            0.02,
            solid(Paint::Sock),
            LOWER_LEG[k],
        ));
        // The ankle bones: the inner one higher and further forward.
        for (side, up_by, fwd_by) in [(1.0f32, 0.0f32, -0.004f32), (-1.0, 0.01, 0.006)] {
            add_part(part(
                ell(
                    add(
                        ankle,
                        add(
                            scale(z, side * s * 0.028),
                            add(scale(v, -up_by), scale(sfront, fwd_by)),
                        ),
                    ),
                    sax,
                    [0.016, 0.014, 0.01],
                ),
                0.012,
                solid(Paint::Sock),
                LOWER_LEG[k],
            ));
        }
        let heel = add(
            j.ball[k],
            add(scale(j.foot_fwd[k], -0.175), scale(j.foot_up[k], 0.06)),
        );
        add_part(part(
            cone(son(0.66, -0.026, 0.0), heel, 0.017, 0.012),
            0.015,
            bands(knee, ankle, SHIN_BANDS),
            LOWER_LEG[k],
        ));
    }

    // --- arms ----------------------------------------------------------
    const UPPER_BANDS: &[(f32, Paint)] = &[
        (0.46, Paint::Jersey),
        (0.51, Paint::JerseyDark),
        (0.535, Paint::Accent),
        (9.0, Paint::Skin),
    ];
    for (k, s) in [1.0f32, -1.0].into_iter().enumerate() {
        let (sh, el, wr) = (j.shoulder[k], j.elbow[k], j.wrist[k]);
        let u = norm(sub(el, sh));
        let front = norm(cross(z, u));
        let ax = [u, front, z];
        let upper_paint = || bands(sh, el, UPPER_BANDS);
        // Deltoid capping the shoulder, tapering to its insertion halfway
        // down the upper arm.
        add_part(part(
            ell(
                add(
                    sh,
                    add(scale(u, 0.022), add(scale(z, s * 0.006), scale(up, 0.004))),
                ),
                axes_along(u, scale(z, s)),
                [0.068, 0.047, 0.043],
            ),
            0.04,
            upper_paint(),
            ARM[k],
        ));
        add_part(part(
            cone(sh, el, 0.044, 0.034),
            0.035,
            upper_paint(),
            ARM[k],
        ));
        // Biceps in front, triceps behind.
        add_part(part(
            ell(
                add(lerp(sh, el, 0.55), scale(front, 0.008)),
                ax,
                [0.085, 0.034, 0.036],
            ),
            0.03,
            upper_paint(),
            ARM[k],
        ));
        add_part(part(
            ell(
                add(lerp(sh, el, 0.45), scale(front, -0.012)),
                ax,
                [0.09, 0.03, 0.036],
            ),
            0.03,
            upper_paint(),
            ARM[k],
        ));
        // The point of the elbow, on the outside of the bend.
        let mid = lerp(sh, wr, 0.5);
        let out = sub(el, mid);
        let out = norm(sub(
            out,
            scale(norm(sub(wr, sh)), dot(out, norm(sub(wr, sh)))),
        ));
        add_part(part(
            ell(add(el, scale(out, 0.018)), ax, [0.024, 0.02, 0.021]),
            0.025,
            solid(Paint::Skin),
            ARM[k],
        ));
        let v = norm(sub(wr, el));
        let fax = axes_along(v, z);
        add_part(part(
            cone(el, wr, 0.034, 0.023),
            0.03,
            solid(Paint::Skin),
            ARM[k],
        ));
        add_part(part(
            ell(lerp(el, wr, 0.27), fax, [0.085, 0.038, 0.034]),
            0.03,
            solid(Paint::Skin),
            ARM[k],
        ));
    }
    // Neck.
    let [hf, hu, _] = j.head_axes;
    add_part(part(
        cone(
            sub(j.neck_base, scale(along, 0.02)),
            sub(j.head, add(scale(hu, 0.055), scale(hf, 0.032))),
            0.055,
            0.047,
        ),
        0.04,
        solid(Paint::Skin),
        NECK_REGION,
    ));

    let mut groups = vec![body];

    // --- hands (gloved, gripping the hoods) ----------------------------
    let bike = j.bike;
    let mut hands = Group {
        parts: Vec::new(),
        cuts: Vec::new(),
        clips: Vec::new(),
    };
    for k in 0..2 {
        let g = j.grip[k];
        let s = if k == 0 { 1.0f32 } else { -1.0 };
        let ax = [bike.fwd, bike.up, bike.right];
        hands.parts.push(rigid(
            ell(
                add(g, bike.dir(-0.018, 0.016, 0.0)),
                ax,
                [0.052, 0.03, 0.04],
            ),
            0.02,
            solid(Paint::Glove),
            HAND[k],
        ));
        // Fingers wrapped down round the hood, and the thumb inside.
        hands.parts.push(rigid(
            cone(
                add(g, bike.dir(0.025, 0.02, 0.0)),
                add(g, bike.dir(0.03, -0.035, 0.0)),
                0.019,
                0.016,
            ),
            0.02,
            solid(Paint::Skin),
            HAND[k],
        ));
        hands.parts.push(rigid(
            cone(
                add(j.wrist[k], bike.dir(0.01, 0.0, -s * 0.02)),
                add(g, bike.dir(0.02, 0.0, -s * 0.03)),
                0.013,
                0.011,
            ),
            0.015,
            solid(Paint::Glove),
            HAND[k],
        ));
        // The wrist, joining the forearm.
        hands.parts.push(part(
            cone(
                j.wrist[k],
                add(g, bike.dir(-0.03, 0.015, 0.0)),
                0.024,
                0.026,
            ),
            0.02,
            solid(Paint::Glove),
            ARM[k],
        ));
    }
    groups.push(hands);

    // --- shoes ---------------------------------------------------------
    let mut shoes = Group {
        parts: Vec::new(),
        cuts: Vec::new(),
        clips: Vec::new(),
    };
    for k in 0..2 {
        let (b, f, n) = (j.ball[k], j.foot_fwd[k], j.foot_up[k]);
        let s = if k == 0 { 1.0f32 } else { -1.0 };
        let ax = [f, n, bike.right];
        let on = |x: f32, y: f32, w: f32| {
            add(b, add(scale(f, x), add(scale(n, y), scale(bike.right, w))))
        };
        let shoe = |shape: Shape, k2: f32, p: Paint| rigid(shape, k2, solid(p), FOOT[k]);
        // White uppers with dark details.
        let upper = |shape: Shape, k2: f32| {
            let paint: Painter = Box::new(move |p| {
                let q = sub(p, b);
                let (x, y) = (dot(q, f), dot(q, n));
                // The rim of the ankle opening, and a strap across the
                // forefoot.
                if (x < -0.105 && y > 0.083 + (x + 0.105) * -0.25)
                    || ((-0.012..0.006).contains(&(x + 0.02 * y)) && y > 0.034)
                {
                    Paint::Dial
                } else {
                    Paint::Shoe
                }
            });
            rigid(shape, k2, paint, FOOT[k])
        };
        // Low, narrow toe; the forefoot over the cleat; the instep rising
        // to the ankle; the heel cup.
        shoes.parts.push(upper(
            ell(on(0.072, 0.021, 0.0), ax, [0.05, 0.021, 0.036]),
            0.03,
        ));
        shoes.parts.push(upper(
            ell(on(0.0, 0.03, 0.0), ax, [0.07, 0.029, 0.042]),
            0.03,
        ));
        shoes.parts.push(upper(
            Shape::Box {
                c: on(-0.06, 0.035, 0.0),
                axes: ax,
                half: [0.075, 0.016, 0.02],
                round: 0.017,
            },
            0.03,
        ));
        shoes.parts.push(upper(
            ell(on(-0.08, 0.052, 0.0), ax, [0.062, 0.04, 0.037]),
            0.03,
        ));
        shoes.parts.push(upper(
            ell(on(-0.148, 0.05, 0.0), ax, [0.044, 0.048, 0.035]),
            0.025,
        ));
        shoes.parts.push(shoe(
            Shape::Box {
                c: on(-0.04, 0.003, 0.0),
                axes: ax,
                half: [0.125, 0.004, 0.038],
                round: 0.005,
            },
            0.004,
            Paint::Sole,
        ));
        // The dial on the instep and the heel tab.
        shoes.parts.push(shoe(
            ell(on(-0.07, 0.088, s * 0.014), ax, [0.014, 0.01, 0.014]),
            0.004,
            Paint::Dial,
        ));
        shoes.parts.push(shoe(
            Shape::Box {
                c: on(-0.19, 0.062, 0.0),
                axes: ax,
                half: [0.004, 0.026, 0.01],
                round: 0.003,
            },
            0.004,
            Paint::Dial,
        ));
    }
    groups.push(shoes);

    // --- head ----------------------------------------------------------
    let h = j.head;
    let hax = j.head_axes;
    let on = |f: f32, u: f32, r: f32| add(h, add(scale(hf, f), add(scale(hu, u), scale(z, r))));
    let mut head = Group {
        parts: Vec::new(),
        cuts: Vec::new(),
        clips: Vec::new(),
    };
    let hh = (h, hf, hu);
    let skull_paint: Painter = Box::new(move |p| {
        let (h, hf, hu) = hh;
        let q = sub(p, h);
        // Hair below the helmet at the back.
        if dot(q, hf) < -0.045 && dot(q, hu) > -0.06 {
            Paint::Hair
        } else {
            Paint::Skin
        }
    });
    head.parts.push(rigid(
        ell(on(-0.01, 0.01, 0.0), hax, [0.098, 0.106, 0.077]),
        0.03,
        skull_paint,
        HEAD,
    ));
    head.parts.push(rigid(
        ell(on(0.036, -0.05, 0.0), hax, [0.07, 0.062, 0.062]),
        0.035,
        solid(Paint::Skin),
        HEAD,
    ));
    head.parts.push(rigid(
        ell(on(0.07, -0.088, 0.0), hax, [0.03, 0.028, 0.034]),
        0.025,
        solid(Paint::Skin),
        HEAD,
    ));
    head.parts.push(rigid(
        cone(
            on(0.088, -0.004, 0.0),
            on(0.101, -0.028, 0.0),
            0.007,
            0.0095,
        ),
        0.01,
        solid(Paint::Skin),
        HEAD,
    ));
    for s in [1.0f32, -1.0] {
        // Cheekbones.
        head.parts.push(rigid(
            ell(on(0.062, -0.022, s * 0.042), hax, [0.03, 0.022, 0.026]),
            0.02,
            solid(Paint::Skin),
            HEAD,
        ));
    }
    for s in [1.0f32, -1.0] {
        head.parts.push(rigid(
            ell(on(-0.008, -0.01, s * 0.078), hax, [0.026, 0.034, 0.012]),
            0.01,
            solid(Paint::Skin),
            HEAD,
        ));
    }
    groups.push(head);

    // --- helmet: a shell with vents, the rim sloping down at the back ---
    let mut helmet = Group {
        parts: Vec::new(),
        cuts: Vec::new(),
        clips: Vec::new(),
    };
    // Long vents following the shell's curve from the brow to the back,
    // shorter towards the sides.
    let shell_c = on(-0.012, 0.036, 0.0);
    let shell_r = [0.143f32, 0.092, 0.104];
    let vents: Vec<Shape> = [-0.066f32, -0.034, 0.0, 0.034, 0.066]
        .iter()
        .map(|&r| {
            let short = r.abs() * 0.9;
            Shape::Slot {
                c: shell_c,
                axes: hax,
                outer: [shell_r[0] + 0.02, shell_r[1] + 0.02, shell_r[2] + 0.02],
                inner: [shell_r[0] - 0.006, shell_r[1] - 0.006, shell_r[2] - 0.006],
                side: r,
                w: 0.0068 - r.abs() * 0.02,
                from: -0.075 + short,
                to: 0.11 - short,
            }
        })
        .collect();
    let vv = vents.clone();
    let hz = (h, hf, hu, z);
    let helmet_paint: Painter = Box::new(move |p| {
        if vv.iter().any(|v| v.dist(p) < 0.002) {
            return Paint::Vent;
        }
        let (h, hf, hu, z) = hz;
        let q = sub(p, h);
        if dot(q, z).abs() < 0.012 && dot(q, hu) > 0.08 && dot(q, hf) > -0.1 {
            return Paint::HelmetAccent;
        }
        // A dark band along the rim.
        let rim = -0.006 + 0.12 * dot(q, hf);
        if dot(q, hu) - rim < 0.0045 {
            return Paint::Vent;
        }
        Paint::Helmet
    });
    let hp: std::sync::Arc<Painter> = std::sync::Arc::new(helmet_paint);
    let shared = |hp: &std::sync::Arc<Painter>| -> Painter {
        let hp = hp.clone();
        Box::new(move |p| hp(p))
    };
    helmet.parts.push(rigid(
        ell(on(-0.012, 0.036, 0.0), hax, [0.143, 0.092, 0.104]),
        0.03,
        shared(&hp),
        HEAD,
    ));
    helmet.parts.push(rigid(
        ell(on(-0.105, 0.024, 0.0), hax, [0.055, 0.05, 0.078]),
        0.03,
        shared(&hp),
        HEAD,
    ));
    helmet.cuts = vents;
    // Keep the shell above its rim: dot(p - o, n) < 0 with n pointing down.
    let rim_normal = norm(sub(scale(hf, 0.12), hu));
    helmet.clips.push((on(0.0, -0.006, 0.0), rim_normal));
    groups.push(helmet);

    // --- sunglasses: a thin wraparound lens and arms to the ears --------
    let mut glasses = Group {
        parts: Vec::new(),
        cuts: Vec::new(),
        clips: Vec::new(),
    };
    let lens_c = on(-0.004, 0.006, 0.0);
    glasses.parts.push(rigid(
        ell(lens_c, hax, [0.116, 0.058, 0.095]),
        0.0,
        solid(Paint::Lens),
        HEAD,
    ));
    // Hollow it to a shell (as thick as the mesh can hold), and keep only
    // the band across the eyes round the front.
    glasses.cuts.push(ell(lens_c, hax, [0.1, 0.044, 0.08]));
    glasses.clips.push((on(0.0, 0.033, 0.0), hu));
    glasses.clips.push((on(0.0, -0.022, 0.0), scale(hu, -1.0)));
    glasses.clips.push((on(0.045, 0.0, 0.0), scale(hf, -1.0)));
    for s in [1.0f32, -1.0] {
        glasses.parts.push(rigid(
            cone(
                on(0.05, 0.012, s * 0.082),
                on(-0.03, 0.01, s * 0.08),
                0.003,
                0.003,
            ),
            0.0,
            solid(Paint::Lens),
            HEAD,
        ));
    }
    groups.push(glasses);

    Sculpt {
        groups,
        bones: segments(j).map(|(a, b, _)| (a, b)),
    }
}

fn dist_to(a: V3, b: V3) -> f32 {
    len(sub(a, b))
}

/// The torso's paint at `p`: shorts, the jersey's hem, collar, chest band,
/// zip, pockets and darker side panels.
fn torso_paint_fn(j: &Joints, p: V3) -> Paint {
    let chord = sub(j.shoulders, j.pelvis);
    let s = dot(sub(p, j.pelvis), chord) / dot(chord, chord);
    let q = sub(p, j.spine(s.clamp(0.0, 1.0)));
    let out = dot(q, j.arch);
    let side = dot(sub(p, j.pelvis), j.body.right);
    if s < 0.12 {
        return Paint::Shorts;
    }
    if s < 0.155 {
        return Paint::JerseyDark;
    }
    if dist_to(p, j.neck_base) < 0.075 {
        return Paint::JerseyDark;
    }
    if (0.62..0.68).contains(&s) {
        return Paint::Accent;
    }
    if out < -0.02 && side.abs() < 0.006 && s > 0.3 {
        return Paint::Zip;
    }
    // Three pockets across the lower back.
    if out > 0.0 && (0.155..0.34).contains(&s) {
        let seam = [-0.15, -0.05, 0.05, 0.15]
            .iter()
            .any(|x| (side - x).abs() < 0.006);
        if seam || (0.325..0.34).contains(&s) {
            return Paint::JerseyDark;
        }
    }
    if side.abs() > 0.125 && s < 0.86 {
        return Paint::JerseyDark;
    }
    Paint::Jersey
}

// ---------------------------------------------------------------------
// The mesh

/// The body as a mesh: `STRIDE` floats per vertex, and triangles.
pub struct Skin {
    pub vertices: Vec<f32>,
    pub indices: Vec<u32>,
}

/// Build the body with cells `cell` metres across (about 8 mm looks
/// smooth up close).
pub fn build(cell: f32) -> Skin {
    let rest = joints(&rest_pose());
    let sculpt = sculpt(&rest);
    let lo = [-0.42f32, 0.02, -0.34];
    let hi = [0.74f32, 1.72, 0.34];
    let n = [
        ((hi[0] - lo[0]) / cell).ceil() as usize + 1,
        ((hi[1] - lo[1]) / cell).ceil() as usize + 1,
        ((hi[2] - lo[2]) / cell).ceil() as usize + 1,
    ];
    let idx = |i: usize, j: usize, k: usize| (i * n[1] + j) * n[2] + k;
    let pos = |i: usize, j: usize, k: usize| {
        [
            lo[0] + i as f32 * cell,
            lo[1] + j as f32 * cell,
            lo[2] + k as f32 * cell,
        ]
    };

    // Distances on the grid, worked out in detail only near the surface:
    // a coarse pass first, then blocks it shows the surface may cross.
    let mut field = vec![FAR; n[0] * n[1] * n[2]];
    const B: usize = 4;
    let reach = cell * B as f32 * 1.8;
    for bi in (0..n[0]).step_by(B) {
        for bj in (0..n[1]).step_by(B) {
            for bk in (0..n[2]).step_by(B) {
                let c = pos(
                    (bi + B / 2).min(n[0] - 1),
                    (bj + B / 2).min(n[1] - 1),
                    (bk + B / 2).min(n[2] - 1),
                );
                let d = sculpt.dist(c);
                let fill = d.abs() > reach;
                for i in bi..(bi + B + 1).min(n[0]) {
                    for j in bj..(bj + B + 1).min(n[1]) {
                        for k in bk..(bk + B + 1).min(n[2]) {
                            let v = &mut field[idx(i, j, k)];
                            if fill {
                                if *v == FAR {
                                    *v = d.signum() * reach;
                                }
                            } else {
                                *v = sculpt.dist(pos(i, j, k));
                            }
                        }
                    }
                }
            }
        }
    }

    // Surface net: a vertex in each cell the surface crosses...
    let mut cell_vertex = vec![u32::MAX; n[0] * n[1] * n[2]];
    let mut points: Vec<V3> = Vec::new();
    const CORNERS: [(usize, usize, usize); 8] = [
        (0, 0, 0),
        (1, 0, 0),
        (0, 1, 0),
        (1, 1, 0),
        (0, 0, 1),
        (1, 0, 1),
        (0, 1, 1),
        (1, 1, 1),
    ];
    const EDGES: [(usize, usize); 12] = [
        (0, 1),
        (2, 3),
        (4, 5),
        (6, 7),
        (0, 2),
        (1, 3),
        (4, 6),
        (5, 7),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    for i in 0..n[0] - 1 {
        for j in 0..n[1] - 1 {
            for k in 0..n[2] - 1 {
                let v: Vec<f32> = CORNERS
                    .iter()
                    .map(|&(a, b, c)| field[idx(i + a, j + b, k + c)])
                    .collect();
                let inside = v.iter().filter(|x| **x < 0.0).count();
                if inside == 0 || inside == 8 {
                    continue;
                }
                let mut sum = [0.0f32; 3];
                let mut count = 0.0;
                for &(a, b) in &EDGES {
                    if (v[a] < 0.0) != (v[b] < 0.0) {
                        let t = v[a] / (v[a] - v[b]);
                        let pa = CORNERS[a];
                        let pb = CORNERS[b];
                        let p = [
                            pa.0 as f32 + (pb.0 as f32 - pa.0 as f32) * t,
                            pa.1 as f32 + (pb.1 as f32 - pa.1 as f32) * t,
                            pa.2 as f32 + (pb.2 as f32 - pa.2 as f32) * t,
                        ];
                        sum = add(sum, p);
                        count += 1.0;
                    }
                }
                let c = scale(sum, 1.0 / count);
                let p = add(pos(i, j, k), scale(c, cell));
                cell_vertex[idx(i, j, k)] = points.len() as u32;
                points.push(p);
            }
        }
    }
    // ...joined across every grid edge the surface crosses.
    let mut indices: Vec<u32> = Vec::new();
    let mut quad = |a: u32, b: u32, c: u32, d: u32, flip: bool, points: &Vec<V3>| {
        if [a, b, c, d].contains(&u32::MAX) {
            return;
        }
        let (a, b, c, d) = if flip { (a, d, c, b) } else { (a, b, c, d) };
        // Split along the shorter diagonal.
        let ac = len(sub(points[a as usize], points[c as usize]));
        let bd = len(sub(points[b as usize], points[d as usize]));
        if ac <= bd {
            indices.extend([a, b, c, a, c, d]);
        } else {
            indices.extend([a, b, d, b, c, d]);
        }
    };
    for i in 0..n[0] - 1 {
        for j in 0..n[1] - 1 {
            for k in 0..n[2] - 1 {
                let here = field[idx(i, j, k)] < 0.0;
                if j > 0 && k > 0 && here != (field[idx(i + 1, j, k)] < 0.0) {
                    quad(
                        cell_vertex[idx(i, j - 1, k - 1)],
                        cell_vertex[idx(i, j, k - 1)],
                        cell_vertex[idx(i, j, k)],
                        cell_vertex[idx(i, j - 1, k)],
                        here,
                        &points,
                    );
                }
                if i > 0 && k > 0 && here != (field[idx(i, j + 1, k)] < 0.0) {
                    quad(
                        cell_vertex[idx(i - 1, j, k - 1)],
                        cell_vertex[idx(i - 1, j, k)],
                        cell_vertex[idx(i, j, k)],
                        cell_vertex[idx(i, j, k - 1)],
                        here,
                        &points,
                    );
                }
                if i > 0 && j > 0 && here != (field[idx(i, j, k + 1)] < 0.0) {
                    quad(
                        cell_vertex[idx(i - 1, j - 1, k)],
                        cell_vertex[idx(i, j - 1, k)],
                        cell_vertex[idx(i, j, k)],
                        cell_vertex[idx(i - 1, j, k)],
                        here,
                        &points,
                    );
                }
            }
        }
    }

    // Settle each vertex onto the true surface, then shade, weight and
    // paint it.
    let mut verts: Vec<Vert> = points
        .iter()
        .map(|p| {
            let mut p = *p;
            for _ in 0..2 {
                let d = sculpt.dist(p);
                let g = sculpt.normal(p);
                p = sub(p, scale(g, d));
            }
            let nrm = sculpt.normal(p);
            let part = sculpt.part_at(p);
            let (bones, weights) = part
                .map(|x| sculpt.weights(x, p))
                .unwrap_or(([PELVIS as f32, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]));
            Vert {
                p,
                n: nrm,
                ao: sculpt.occlusion(p, nrm),
                bones,
                weights,
                paint: sculpt.paint_at(p),
            }
        })
        .collect();

    // Crisp kit lines: a triangle whose corners are painted differently is
    // cut exactly along the boundary between the paints, so a sleeve hem
    // or a chest band is a clean line, not a jagged blend.
    let mut out = Vec::with_capacity(indices.len());
    for t in indices.chunks(3) {
        split(&sculpt, &mut verts, [t[0], t[1], t[2]], 0, &mut out);
    }

    let mut vertices = Vec::with_capacity(verts.len() * STRIDE);
    for v in &verts {
        let (col, mat) = v.paint.look();
        let col = scale(col, v.ao);
        vertices.extend_from_slice(&[
            v.p[0], v.p[1], v.p[2], v.n[0], v.n[1], v.n[2], col[0], col[1], col[2], mat,
        ]);
        vertices.extend_from_slice(&v.bones);
        vertices.extend_from_slice(&v.weights);
    }
    Skin {
        vertices,
        indices: out,
    }
}

/// A vertex of the body while it is being made.
#[derive(Debug, Clone, Copy)]
struct Vert {
    p: V3,
    n: V3,
    ao: f32,
    bones: [f32; 4],
    weights: [f32; 4],
    paint: Paint,
}

impl Sculpt {
    fn paint_at(&self, p: V3) -> Paint {
        self.part_at(p).map(|x| (x.paint)(p)).unwrap_or(Paint::Skin)
    }
}

/// A point `t` of the way from `a` to `b`. Its bones are a merge of both
/// ends' (the same however it is reached), so the two copies made along a
/// boundary bend exactly alike and no crack opens between them.
fn between(a: &Vert, b: &Vert, t: f32, paint: Paint) -> Vert {
    let mut bw: Vec<(f32, f32)> = Vec::new();
    for (v, f) in [(a, 1.0 - t), (b, t)] {
        for i in 0..4 {
            let w = v.weights[i] * f;
            if w <= 0.0 {
                continue;
            }
            match bw.iter_mut().find(|x| x.0 == v.bones[i]) {
                Some(x) => x.1 += w,
                None => bw.push((v.bones[i], w)),
            }
        }
    }
    bw.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.total_cmp(&y.0)));
    bw.truncate(4);
    let sum: f32 = bw.iter().map(|x| x.1).sum();
    let mut bones = [0.0; 4];
    let mut weights = [0.0; 4];
    for (i, (b, w)) in bw.into_iter().enumerate() {
        bones[i] = b;
        weights[i] = w / sum;
    }
    Vert {
        p: lerp(a.p, b.p, t),
        n: norm(lerp(a.n, b.n, t)),
        ao: a.ao + (b.ao - a.ao) * t,
        bones,
        weights,
        paint,
    }
}

/// Where along `a`-`b` the paint stops being `a`'s.
fn crossing(sculpt: &Sculpt, a: &Vert, b: &Vert) -> f32 {
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..7 {
        let mid = (lo + hi) / 2.0;
        if sculpt.paint_at(lerp(a.p, b.p, mid)) == a.paint {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo + hi) / 2.0
}

/// Add the triangle `t` to `out`, cut along paint boundaries.
fn split(sculpt: &Sculpt, verts: &mut Vec<Vert>, t: [u32; 3], depth: u32, out: &mut Vec<u32>) {
    let v = t.map(|i| verts[i as usize]);
    let same = |i: usize, j: usize| v[i].paint == v[j].paint;
    if (same(0, 1) && same(1, 2)) || depth >= 3 {
        out.extend(t);
        return;
    }
    // The corner on its own (if two share a paint), else the first.
    let odd = if same(1, 2) {
        0
    } else if same(0, 2) {
        1
    } else if same(0, 1) {
        2
    } else {
        0
    };
    let (o, x, y) = (odd, (odd + 1) % 3, (odd + 2) % 3);
    let tx = crossing(sculpt, &v[o], &v[x]);
    let ty = crossing(sculpt, &v[o], &v[y]);
    let mut push = |vert: Vert| {
        verts.push(vert);
        (verts.len() - 1) as u32
    };
    // Each crossing twice: once in the odd corner's paint, once in the
    // paint beyond the boundary.
    let ox = push(between(&v[o], &v[x], tx, v[o].paint));
    let oy = push(between(&v[o], &v[y], ty, v[o].paint));
    let xo = push(between(&v[o], &v[x], tx, v[x].paint));
    let yo = push(between(&v[o], &v[y], ty, v[y].paint));
    out.extend([t[o], ox, oy]);
    split(sculpt, verts, [xo, t[x], t[y]], depth + 1, out);
    split(sculpt, verts, [xo, t[y], yo], depth + 1, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_is_one_closed_well_formed_surface() {
        let skin = build(0.02);
        let n = skin.vertices.len() / STRIDE;
        assert!(n > 1000, "{n} vertices");
        assert_eq!(skin.indices.len() % 3, 0);
        assert!(skin.indices.iter().all(|&i| (i as usize) < n));
        for v in skin.vertices.chunks(STRIDE) {
            assert!(v.iter().all(|x| x.is_finite()), "{v:?}");
            assert!((len([v[3], v[4], v[5]]) - 1.0).abs() < 1e-3, "unit normals");
            let w: f32 = v[14..18].iter().sum();
            assert!((w - 1.0).abs() < 1e-3, "weights sum to 1: {w}");
            assert!(v[10..14].iter().all(|b| (*b as usize) < BONES));
        }
    }

    #[test]
    fn at_rest_the_bones_do_not_move_anything() {
        let rest = joints(&rest_pose());
        let skin = build(0.03);
        let _ = skin;
        for m in skinning(&rest_inverse(), &rest) {
            for (i, x) in m.iter().enumerate() {
                let identity = if i % 5 == 0 { 1.0 } else { 0.0 };
                assert!((x - identity).abs() < 1e-4, "{m:?}");
            }
        }
    }

    #[test]
    fn feet_follow_the_pedals() {
        // Turn the cranks: the shoe's vertices move with the foot bone.
        let skin = build(0.03);
        let mut p = rest_pose();
        p.crank += 1.0;
        let posed = joints(&p);
        let _ = &skin;
        let m = skinning(&rest_inverse(), &posed);
        let f = FOOT[0];
        let rest_toe = segments(&joints(&rest_pose()))[f].1;
        let moved = transform(&m[f], [rest_toe[0], rest_toe[1], rest_toe[2], 1.0]);
        let want = segments(&posed)[f].1;
        assert!(len(sub([moved[0], moved[1], moved[2]], want)) < 1e-4);
    }

    #[test]
    fn the_rider_wears_his_kit() {
        let skin = build(0.015);
        let count = |c: V3| {
            skin.vertices
                .chunks(STRIDE)
                .filter(|v| len(sub([v[6], v[7], v[8]], c)) < 0.35 * len(c).max(0.05))
                .count()
        };
        // Jersey (marked for the jersey colour), helmet and shoes are there.
        let jersey = skin.vertices.chunks(STRIDE).filter(|v| v[6] < 0.0).count();
        assert!(jersey > 200, "{jersey}");
        assert!(count(Paint::Helmet.look().0) > 50);
        assert!(count(Paint::Shoe.look().0) > 30);
        // Head, helmet and shoes are rigid.
        let rigid = skin
            .vertices
            .chunks(STRIDE)
            .filter(|v| v[14] == 1.0 && (v[10] as usize == HEAD))
            .count();
        assert!(rigid > 100);
    }
}
