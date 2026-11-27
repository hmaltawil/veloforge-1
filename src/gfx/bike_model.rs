//! The aero road bike, modelled in Blender and baked by
//! `tools/bike-model/bake.py` into pieces laid out as VeloForge's meshes
//! are (x forward, y up, z right, the bottom bracket at (-0.02, 0.28)):
//! the frame, two wheels and the crankset, each about the point it turns
//! on, and the two pedals. Each frame the wheels turn with the road, the
//! crankset with the pedals, and the pedals sit under the feet.

use super::math::*;
use super::mesh::{Frame, Joints, Mesh, RiderPose, MAT_GLOSS, MAT_LAMP, MAT_MATT, VERTEX_STRIDE};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

// The baked bike, when it was there to build in (`build.rs`); without it
// riders ride the classic bike.
#[cfg(aero_model)]
static BAKED: &[u8] = include_bytes!("../../assets/models/bike-aero.vfb");
/// The same bike in less detail, for riders further away (pacers, ghosts).
#[cfg(aero_model)]
static BAKED_FAR: &[u8] = include_bytes!("../../assets/models/bike-aero-far.vfb");
#[cfg(not(aero_model))]
static BAKED: &[u8] = &[];
#[cfg(not(aero_model))]
static BAKED_FAR: &[u8] = &[];

/// Whether this build has the aero bike (its model was there to build in).
pub fn available() -> bool {
    model().is_some()
}

/// Ride the aero bike (true) or the classic one drawn in `mesh::bike`.
static AERO: AtomicBool = AtomicBool::new(true);

pub fn set_aero(on: bool) {
    AERO.store(on, Ordering::Relaxed);
}

pub fn aero() -> bool {
    AERO.load(Ordering::Relaxed) && model().is_some()
}

/// Tyres with tan sidewalls (gumwall) rather than all black.
static TAN_WALLS: AtomicBool = AtomicBool::new(false);

pub fn set_tan_walls(on: bool) {
    TAN_WALLS.store(on, Ordering::Relaxed);
}

/// The frame's paint, chosen by the rider (linear colour).
static PAINT: std::sync::RwLock<V3> = std::sync::RwLock::new([0.62, 0.015, 0.02]);

pub fn set_paint(c: V3) {
    if let Ok(mut p) = PAINT.write() {
        *p = c;
    }
}

fn paint() -> V3 {
    PAINT.read().map(|p| *p).unwrap_or([0.62, 0.015, 0.02])
}

struct Piece {
    name: String,
    pivot: V3,
    /// Vertices about the pivot, `VERTEX_STRIDE` floats each.
    v: Vec<f32>,
}

pub struct Model {
    /// Crank length, and the right crank's angle as modelled (0 up,
    /// growing forward).
    pub crank: f32,
    crank_at: f32,
    /// Where the right hand holds the hood.
    pub grip: V3,
    pieces: Vec<Piece>,
}

/// The baked bike, read once.
pub fn model() -> Option<&'static Model> {
    static MODEL: OnceLock<Option<Model>> = OnceLock::new();
    MODEL.get_or_init(|| read(BAKED)).as_ref()
}

/// Its lighter copy.
fn model_far() -> Option<&'static Model> {
    static MODEL: OnceLock<Option<Model>> = OnceLock::new();
    MODEL.get_or_init(|| read(BAKED_FAR)).as_ref()
}

fn read(b: &[u8]) -> Option<Model> {
    let mut at = 0usize;
    let mut take = |n: usize| -> Option<&[u8]> {
        let s = b.get(at..at + n)?;
        at += n;
        Some(s)
    };
    if take(4)? != b"VFBK" {
        return None;
    }
    let u32_ = |s: &[u8]| u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
    let f32_ = |s: &[u8; 4]| f32::from_le_bytes(*s);
    if u32_(take(4)?) != 1 {
        return None;
    }
    let head: Vec<f32> = take(20)?.as_chunks::<4>().0.iter().map(f32_).collect();
    let n = u32_(take(4)?) as usize;
    let mut pieces = Vec::with_capacity(n);
    for _ in 0..n {
        let len = take(1)?[0] as usize;
        let name = String::from_utf8_lossy(take(len)?).into_owned();
        let p: Vec<f32> = take(12)?.as_chunks::<4>().0.iter().map(f32_).collect();
        let count = u32_(take(4)?) as usize;
        let v: Vec<f32> = take(count * VERTEX_STRIDE * 4)?
            .as_chunks::<4>()
            .0
            .iter()
            .map(f32_)
            .collect();
        pieces.push(Piece {
            name,
            pivot: [p[0], p[1], p[2]],
            v,
        });
    }
    Some(Model {
        crank: head[0],
        crank_at: head[1],
        // Hands rest on top of the hoods.
        grip: [head[2], head[3] + 0.012, head[4]],
        pieces,
    })
}

/// Turn a point about the right-hand axis so that one at angle `a` (0 up,
/// growing forward) goes to `a + d`.
fn turn(x: f32, y: f32, (s, c): (f32, f32)) -> (f32, f32) {
    (x * c + y * s, y * c - x * s)
}

/// The aero bike for the pose `j`: in full, or (`far`) the lighter copy.
pub fn mesh(p: &RiderPose, j: &Joints, far: bool) -> Mesh {
    let mut m = Mesh::default();
    let Some(model) = (if far { model_far() } else { model() }) else {
        return m;
    };
    let bike = j.bike;
    let r = model.crank;
    let paint = paint();
    let tan = TAN_WALLS.load(Ordering::Relaxed);
    for piece in &model.pieces {
        // Where the piece sits, and how far it is turned.
        let (pivot, angle) = match piece.name.as_str() {
            "rear_wheel" | "front_wheel" => (piece.pivot, p.wheel),
            "crankset" => (piece.pivot, p.crank - model.crank_at),
            "pedal_right" | "pedal_left" => {
                // Under the foot: on the crank's circle, as wide as the
                // rider's feet.
                let side = if piece.name == "pedal_right" {
                    1.0
                } else {
                    -1.0
                };
                let th = p.crank
                    + if side > 0.0 {
                        0.0
                    } else {
                        std::f32::consts::PI
                    };
                (
                    [-0.02 + r * th.sin(), 0.28 + r * th.cos(), side * 0.095],
                    0.0,
                )
            }
            _ => (piece.pivot, 0.0),
        };
        let sc = angle.sin_cos();
        let wheel = piece.name.ends_with("wheel");
        for v in piece.v.as_chunks::<VERTEX_STRIDE>().0 {
            let (px, py) = turn(v[0], v[1], sc);
            let (nx, ny) = turn(v[3], v[4], sc);
            // The frame's paint (baked as -1) in the rider's colour.
            let (colour, mat) = if v[6] < 0.0 {
                (paint, v[9])
            } else if wheel && matte_black(v) {
                ([0.022, 0.022, 0.024], 0.0)
            } else if tan && wheel && tan_wall(v) {
                (TAN_WALL, 0.0)
            } else {
                ([v[6], v[7], v[8]], v[9])
            };
            m.mat = mat;
            m.push(
                bike.at(pivot[0] + px, pivot[1] + py, pivot[2] + v[2]),
                bike.dir(nx, ny, v[5]),
                colour,
            );
        }
    }
    rear_light(&mut m, &bike, far);
    cockpit(&mut m, &bike, far);
    m
}

/// A box `half` in size about `c` (in the bike's terms), turned `tilt`
/// radians nose up about the bike's sideways axis, faces flat-shaded.
fn tilted_box(m: &mut Mesh, bike: &Frame, c: V3, half: V3, tilt: f32, col: V3) {
    let (s, k) = tilt.sin_cos();
    let at = |x: f32, y: f32, z: f32| {
        let (x, y) = (x * half[0], y * half[1]);
        bike.at(
            c[0] + x * k - y * s,
            c[1] + x * s + y * k,
            c[2] + z * half[2],
        )
    };
    let corner = |i: usize| {
        at(
            if i & 1 == 0 { -1.0 } else { 1.0 },
            if i & 2 == 0 { -1.0 } else { 1.0 },
            if i & 4 == 0 { -1.0 } else { 1.0 },
        )
    };
    // Each face's corners, wound to face out.
    for f in [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ] {
        m.quad(corner(f[0]), corner(f[1]), corner(f[2]), corner(f[3]), col);
    }
}

/// Out in front of the bar: a mount holding a bike computer on top,
/// tilted up towards the rider, and a front light under it whose lens
/// glows white (`MAT_LAMP`), brighter after dark.
fn cockpit(m: &mut Mesh, bike: &Frame, far: bool) {
    let black = [0.03, 0.03, 0.032];
    let sides = if far { 6 } else { 14 };
    // The mount: an arm forward from the middle of the bar.
    m.mat = MAT_MATT;
    m.tube(
        bike.at(0.462, 0.848, 0.0),
        bike.at(0.535, 0.846, 0.0),
        0.009,
        0.008,
        sides.min(8),
        black,
    );
    // The computer: a slim slab, its screen on top.
    let tilt = 0.14;
    let c = [0.522, 0.866, 0.0];
    m.mat = MAT_GLOSS;
    tilted_box(m, bike, c, [0.042, 0.009, 0.025], tilt, black);
    // The screen: a dark glass panel with lighter data fields on it.
    let (s, k) = tilt.sin_cos();
    let on_top = |x: f32, z: f32| {
        let y = 0.0092;
        bike.at(c[0] + x * k - y * s, c[1] + x * s + y * k, z)
    };
    let up = bike.dir(-s, k, 0.0);
    let panel = |m: &mut Mesh, x0: f32, x1: f32, z0: f32, z1: f32, col: V3| {
        let p = [
            on_top(x0, z0),
            on_top(x0, z1),
            on_top(x1, z1),
            on_top(x1, z0),
        ];
        for j in [0, 1, 2, 0, 2, 3] {
            m.push(p[j], up, col);
        }
    };
    panel(m, -0.034, 0.03, -0.019, 0.019, [0.06, 0.075, 0.085]);
    if !far {
        // Fields of figures, as the screen shows them.
        let fig = [0.32, 0.34, 0.35];
        for (x0, x1, z0, z1) in [
            (0.012, 0.024, -0.015, -0.002),
            (0.012, 0.024, 0.003, 0.015),
            (-0.006, 0.006, -0.015, 0.015),
            (-0.026, -0.014, -0.015, -0.002),
            (-0.026, -0.014, 0.003, 0.015),
        ] {
            panel(m, x0, x1, z0, z1, fig);
        }
    }
    // Its holder under it, down to the arm.
    m.mat = MAT_MATT;
    m.tube(
        bike.at(0.52, 0.852, 0.0),
        bike.at(0.52, 0.86, 0.0),
        0.008,
        0.008,
        sides.min(8),
        black,
    );

    // The front light: a barrel hanging under the arm, its lens forward.
    let (back, front, y, r) = (0.47, 0.565, 0.818, 0.0135);
    m.tube(
        bike.at(0.53, 0.84, 0.0),
        bike.at(0.53, y + r * 0.6, 0.0),
        0.006,
        0.006,
        sides.min(8),
        black,
    );
    m.mat = MAT_GLOSS;
    m.tube(
        bike.at(back, y, 0.0),
        bike.at(front, y, 0.0),
        r * 0.92,
        r,
        sides,
        black,
    );
    // A grey band near the front, as the lens housing.
    m.tube(
        bike.at(front - 0.012, y, 0.0),
        bike.at(front, y, 0.0),
        r * 1.04,
        r * 1.04,
        sides,
        [0.28, 0.28, 0.3],
    );
    m.disc(
        bike.at(back, y, 0.0),
        r * 0.92,
        bike.dir(-1.0, 0.0, 0.0),
        sides,
        black,
    );
    m.mat = MAT_LAMP;
    m.disc(
        bike.at(front + 0.0005, y, 0.0),
        r * 0.95,
        bike.dir(1.0, 0.0, 0.0),
        sides,
        [0.95, 0.95, 0.9],
    );
}

/// Where the rear light sits: the back of the seatpost, a hand's width
/// above the top tube (in the bike's terms), and the post's lean.
const LIGHT_POST: [f32; 2] = [-0.168, 0.742];
const LIGHT_LEAN: [f32; 2] = [-0.184, 0.983];

/// A rear light strapped to the seatpost: a slim body, wider at the top
/// and rounded at both ends, a ribbed red lens round a black panel with a
/// bright LED window at its top, on a small clamp with a rubber strap.
/// The lens and LED glow (`MAT_LAMP`), brighter after dark. `far`: a
/// coarser one for riders further away.
fn rear_light(m: &mut Mesh, bike: &Frame, far: bool) {
    let up = LIGHT_LEAN;
    // Rearward, square to the post.
    let back = [-up[1], up[0]];
    // A point `b` behind the post, `h` up it and `z` to the right, relative
    // to where the light hangs.
    let at = |b: f32, h: f32, z: f32| {
        bike.at(
            LIGHT_POST[0] + back[0] * b + up[0] * h,
            LIGHT_POST[1] + back[1] * b + up[1] * h,
            z,
        )
    };
    let dir =
        |b: f32, h: f32, z: f32| bike.dir(back[0] * b + up[0] * h, back[1] * b + up[1] * h, z);
    let black = [0.03, 0.03, 0.032];

    // The body: rings up its height, each a rounded square across.
    const HEIGHT: f32 = 0.074;
    const DEPTH: f32 = 0.012; // half, front to back
    let (rings, round) = if far { (12, 12) } else { (40, 32) };
    let gap = 0.016; // from the post's back to the body's middle
    let shape = |t: f32, a: f32| -> (f32, f32, f32) {
        // Half width: 18 mm at the top, 12 at the bottom.
        let w = 0.012 + 0.006 * t.powf(0.7);
        // Rounded at both ends: flat sides, soft caps.
        let e = (1.0 - (2.0 * t - 1.0).abs().powi(6))
            .max(0.0)
            .powf(1.0 / 6.0);
        let (s, c) = a.sin_cos();
        let sq = |x: f32| x.signum() * x.abs().sqrt();
        (DEPTH * e * sq(c), (t - 0.5) * HEIGHT, w * e * sq(s))
    };
    let point = |i: usize, k: usize| {
        let t = i as f32 / rings as f32;
        let a = k as f32 / round as f32 * std::f32::consts::TAU;
        shape(t, a)
    };
    for i in 0..rings {
        for k in 0..round {
            let corners = [(i, k), (i + 1, k), (i + 1, k + 1), (i, k + 1)];
            let p = corners.map(|(i, k)| point(i, k));
            // What this patch is: the black front and panel, the LED, or the
            // ribbed lens.
            let t = (i as f32 + 0.5) / rings as f32;
            let a = (k as f32 + 0.5) / round as f32 * std::f32::consts::TAU;
            let (nb, nz) = (a.cos(), a.sin());
            let (colour, mat) = if nb < -0.2 {
                (black, MAT_MATT)
            } else if nb > 0.55 && nz.abs() < 0.62 && t > 0.1 && t < 0.92 {
                if t > 0.68 && t < 0.86 && nz.abs() < 0.4 {
                    ([1.0, 0.07, 0.04], MAT_LAMP)
                } else {
                    ([0.02, 0.02, 0.022], MAT_GLOSS)
                }
            } else if (t * 26.0) as i32 % 2 == 0 {
                ([0.34, 0.025, 0.02], MAT_LAMP)
            } else {
                ([0.2, 0.015, 0.012], MAT_LAMP)
            };
            m.mat = mat;
            let pos = p.map(|(b, h, z)| at(gap + b, h, z));
            // Normals across the rounded square, leaning up or down at
            // the caps.
            let nrm = corners.map(|(i, k)| {
                let (b0, h0, z0) = point(i, k);
                let (b1, _, z1) = point((i + 1).min(rings), k);
                let (bd, _, zd) = point(i.saturating_sub(1), k);
                let slope = ((b1 - bd).hypot(z1 - zd)) / (2.0 * HEIGHT / rings as f32);
                let out = (b0.hypot(z0)).max(1e-6);
                let h = if h0 > 0.0 { slope } else { -slope };
                norm(dir(b0 / out, h.clamp(-4.0, 4.0) * 0.5, z0 / out))
            });
            for j in [0, 1, 2, 0, 2, 3] {
                m.push(pos[j], nrm[j], colour);
            }
        }
    }

    // The clamp from the post to the body, and the strap round the post.
    m.mat = MAT_MATT;
    let sides = if far { 4 } else { 8 };
    m.tube(
        at(0.0, 0.012, 0.0),
        at(gap - 0.006, 0.012, 0.0),
        0.007,
        0.006,
        sides,
        black,
    );
    let loop_pts: Vec<V3> = (0..=10)
        .map(|i| {
            let a = i as f32 / 10.0 * std::f32::consts::TAU;
            at(-0.022 + 0.026 * a.cos(), 0.012, 0.011 * a.sin())
        })
        .collect();
    for w in loop_pts.windows(2) {
        m.tube(w[0], w[1], 0.0035, 0.0035, sides.min(6), black);
    }
}

/// The tyres' tan sidewalls (gumwall), in the light's terms.
const TAN_WALL: V3 = [0.5, 0.34, 0.17];

/// Whether a wheel's vertex is on a tyre's sidewall: the black tyre (not
/// the rim or hub) from just above the rim's edge (0.330 from the axle)
/// to where the tread rounds over (0.339), which stays black.
fn tan_wall(v: &[f32; VERTEX_STRIDE]) -> bool {
    let out = (v[0] * v[0] + v[1] * v[1]).sqrt();
    (v[6] - 0.02).abs() < 0.005 && v[9] < 0.5 && out > 0.3285 && out < 0.3385
}

/// Whether a wheel's vertex is part of what turns matte black: the glossy
/// grey rims and the silver spokes (further out than the cassette and
/// disc rotor, which stay metal). Tyres are matte black already.
fn matte_black(v: &[f32; VERTEX_STRIDE]) -> bool {
    let grey = |c: f32| (v[6] - c).abs() < 0.01;
    let out = (v[0] * v[0] + v[1] * v[1]).sqrt();
    (grey(0.07) && v[9] > 2.5) || (grey(0.42) && out > 0.095)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg_attr(not(aero_model), ignore = "the baked bike isn't in this build")]
    fn the_far_bike_matches() {
        let (near, far) = (model().unwrap(), model_far().unwrap());
        assert_eq!(near.pieces.len(), far.pieces.len());
        assert!((near.crank - far.crank).abs() < 1e-4);
        assert!(far.pieces[0].v.len() < near.pieces[0].v.len() / 2);
    }

    #[test]
    #[cfg_attr(not(aero_model), ignore = "the baked bike isn't in this build")]
    fn the_baked_bike_reads() {
        let m = model().expect("the baked bike reads");
        let names: Vec<&str> = m.pieces.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "frame",
                "rear_wheel",
                "front_wheel",
                "crankset",
                "pedal_right",
                "pedal_left"
            ]
        );
        // A road bike's crank, hoods ahead of and above the bottom bracket.
        assert!((0.13..0.18).contains(&m.crank), "{}", m.crank);
        assert!(
            m.grip[0] > 0.4 && m.grip[1] > 0.8 && m.grip[2] > 0.1,
            "{:?}",
            m.grip
        );
        // Wheels on the ground: as tall as their hubs are high.
        for w in &m.pieces[1..3] {
            let r =
                w.v.as_chunks::<VERTEX_STRIDE>()
                    .0
                    .iter()
                    .map(|v| (v[0] * v[0] + v[1] * v[1]).sqrt())
                    .fold(0.0f32, f32::max);
            assert!(
                (r - w.pivot[1]).abs() < 0.01,
                "{} {r} {:?}",
                w.name,
                w.pivot
            );
        }
    }

    #[test]
    #[cfg(not(aero_model))]
    fn without_the_model_riders_ride_the_classic_bike() {
        set_aero(true);
        assert!(!available() && !aero());
        assert!(model_far().is_none());
    }

    #[test]
    fn turning_goes_forward_from_the_top() {
        // A point at the top turned a quarter goes to the front.
        let (x, y) = turn(0.0, 1.0, (std::f32::consts::FRAC_PI_2).sin_cos());
        assert!((x - 1.0).abs() < 1e-6 && y.abs() < 1e-6);
    }

    /// How long building each bike takes, per frame: `cargo test --release
    /// bike_build_time -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bike_build_time() {
        use super::super::mesh;
        let pose = RiderPose {
            pos: [0.0, 0.0, 0.0],
            fwd: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            right: [1.0, 0.0, 0.0],
            crank: 1.0,
            wheel: 2.0,
            jersey: [0.6, 0.1, 0.1],
            stand: 0.0,
            tuck: 0.0,
            effort: 1.0,
            time: 0.0,
        };
        let j = mesh::joints(&pose);
        for (name, aero, far) in [
            ("aero", true, false),
            ("aero far", true, true),
            ("classic", false, false),
        ] {
            let t = std::time::Instant::now();
            let mut verts = 0;
            for _ in 0..200 {
                let m = if aero {
                    mesh(&pose, &j, far)
                } else {
                    mesh::bike(&pose, &j)
                };
                verts = m.vertex_count();
            }
            let ms = t.elapsed().as_secs_f64() * 1000.0 / 200.0;
            println!(
                "{name}: {ms:.2} ms a frame, {verts} vertices, {:.1} MB sent a frame",
                verts as f64 * VERTEX_STRIDE as f64 * 4.0 / 1e6
            );
        }
    }
}
