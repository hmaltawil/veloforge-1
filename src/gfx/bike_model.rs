//! The aero road bike, modelled in Blender and baked by
//! `tools/bike-model/bake.py` into pieces laid out as VeloForge's meshes
//! are (x forward, y up, z right, the bottom bracket at (-0.02, 0.28)):
//! the frame, two wheels and the crankset, each about the point it turns
//! on, and the two pedals. Each frame the wheels turn with the road, the
//! crankset with the pedals, and the pedals sit under the feet.

use super::math::*;
use super::mesh::{Joints, Mesh, RiderPose, VERTEX_STRIDE};
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
    m
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
