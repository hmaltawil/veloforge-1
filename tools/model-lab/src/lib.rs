//! The app's own model code (src/gfx/math.rs, mesh.rs and body.rs, included
//! as they are) behind a few functions a web page can call.

#![allow(dead_code)]

#[path = "../../../src/gfx/body.rs"]
mod body;
#[path = "../../../src/gfx/math.rs"]
mod math;
#[path = "../../../src/gfx/mesh.rs"]
mod mesh;

use math::*;
use std::cell::RefCell;

struct Lab {
    skin: Option<body::Skin>,
    rest_inv: Option<[M4; body::BONES]>,
    /// The posed body: position, normal, colour, material (10 floats).
    body: Vec<f32>,
    bike: Vec<f32>,
    model: Vec<f32>,
    joints: Vec<f32>,
}

thread_local! {
    static LAB: RefCell<Lab> = const { RefCell::new(Lab {
        skin: None,
        rest_inv: None,
        body: Vec::new(),
        bike: Vec::new(),
        model: Vec::new(),
        joints: Vec::new(),
    }) };
}

/// Build the body with cells `cell` metres across (the app uses 0.01).
/// Returns the number of indices.
#[no_mangle]
pub extern "C" fn build_body(cell: f32) -> u32 {
    LAB.with(|l| {
        let mut l = l.borrow_mut();
        let skin = body::build(cell);
        let n = skin.indices.len() as u32;
        l.skin = Some(skin);
        l.rest_inv = Some(body::rest_inverse());
        n
    })
}

#[no_mangle]
pub extern "C" fn body_indices() -> *const u32 {
    LAB.with(|l| {
        l.borrow()
            .skin
            .as_ref()
            .map_or(std::ptr::null(), |s| s.indices.as_ptr())
    })
}

#[no_mangle]
pub extern "C" fn body_vertex_count() -> u32 {
    LAB.with(|l| {
        l.borrow()
            .skin
            .as_ref()
            .map_or(0, |s| (s.vertices.len() / body::STRIDE) as u32)
    })
}

/// Pose the rider and the bike. `lean` and `grade` are in degrees.
/// Afterwards `body_ptr` holds the bent body (10 floats a vertex, in the
/// same order as the indices refer to) and `bike_ptr`/`bike_len` the bike
/// (10 floats a vertex, plain triangles).
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn pose(
    crank: f32,
    stand: f32,
    tuck: f32,
    lean: f32,
    grade: f32,
    effort: f32,
    time: f32,
    r: f32,
    g: f32,
    b: f32,
) {
    LAB.with(|l| {
        let mut l = l.borrow_mut();
        let slope = (grade / 100.0).atan();
        let fwd = [slope.cos(), slope.sin(), 0.0];
        let flat_right = [0.0, 0.0, 1.0];
        let flat_up = cross(flat_right, fwd);
        let (ls, lc) = lean.to_radians().sin_cos();
        let up = add(scale(flat_up, lc), scale(flat_right, ls));
        let right = sub(scale(flat_right, lc), scale(flat_up, ls));
        let jersey = [r, g, b];
        let p = mesh::RiderPose {
            pos: [0.0; 3],
            fwd,
            up,
            right,
            crank,
            wheel: crank * 2.4,
            jersey,
            stand,
            tuck,
            effort,
            time,
        };
        let j = mesh::joints(&p);
        l.bike = mesh::bike(&p, &j).v;
        l.joints.clear();
        for v in [j.hips, j.shoulders, j.head]
            .iter()
            .chain(&j.hip)
            .chain(&j.knee)
            .chain(&j.ankle)
            .chain(&j.shoulder)
            .chain(&j.elbow)
            .chain(&j.wrist)
        {
            l.joints.extend_from_slice(v);
        }
        let (Some(skin), Some(rest_inv)) = (l.skin.as_ref(), l.rest_inv.as_ref()) else {
            return;
        };
        let m = body::skinning(rest_inv, &j);
        let n = skin.vertices.len() / body::STRIDE;
        let mut out = Vec::with_capacity(n * 10);
        for s in skin.vertices.chunks_exact(body::STRIDE) {
            let mut pp = [0.0f32; 3];
            let mut nn = [0.0f32; 3];
            for k in 0..4 {
                let w = s[14 + k];
                if w == 0.0 {
                    continue;
                }
                let mm = &m[s[10 + k] as usize];
                let tp = transform(mm, [s[0], s[1], s[2], 1.0]);
                let tn = transform(mm, [s[3], s[4], s[5], 0.0]);
                pp = add(pp, scale([tp[0], tp[1], tp[2]], w));
                nn = add(nn, scale([tn[0], tn[1], tn[2]], w));
            }
            let nn = norm(nn);
            // Negative colours are shares of the jersey colour.
            let c = if s[6] < 0.0 {
                [-s[6] * r, -s[7] * g, -s[8] * b]
            } else {
                [s[6], s[7], s[8]]
            };
            out.extend_from_slice(&[
                pp[0], pp[1], pp[2], nn[0], nn[1], nn[2], c[0], c[1], c[2], s[9],
            ]);
        }
        l.body = out;
    })
}

#[no_mangle]
pub extern "C" fn body_ptr() -> *const f32 {
    LAB.with(|l| l.borrow().body.as_ptr())
}

#[no_mangle]
pub extern "C" fn bike_ptr() -> *const f32 {
    LAB.with(|l| l.borrow().bike.as_ptr())
}

#[no_mangle]
pub extern "C" fn bike_len() -> u32 {
    LAB.with(|l| l.borrow().bike.len() as u32)
}

/// Joint positions (hips, shoulders, head, then hip, knee, ankle, shoulder,
/// elbow, wrist for each side), 3 floats each.
#[no_mangle]
pub extern "C" fn joints_ptr() -> *const f32 {
    LAB.with(|l| l.borrow().joints.as_ptr())
}

/// Build one of the scenery models: 0 pine, 1 broadleaf, 2 birch, 3 bush,
/// 4 rock, 5 house, 6 far pine, 7 far broadleaf. Returns the float count.
#[no_mangle]
pub extern "C" fn model(kind: u32) -> u32 {
    LAB.with(|l| {
        let mut l = l.borrow_mut();
        let m = match kind {
            0 => mesh::pine(),
            1 => mesh::broadleaf(),
            2 => mesh::birch(),
            3 => mesh::bush(),
            4 => mesh::rock(),
            5 => mesh::house(),
            6 => mesh::pine_far(),
            _ => mesh::broadleaf_far(),
        };
        l.model = m.v;
        l.model.len() as u32
    })
}

/// Build grass tuft variant `kind` (0..4, as the app's meadow uses them):
/// position and colour, 6 floats a vertex. Returns the float count; the
/// floats are at `model_ptr`.
#[no_mangle]
pub extern "C" fn grass(kind: u32) -> u32 {
    LAB.with(|l| {
        let mut l = l.borrow_mut();
        l.model = match kind {
            0 => mesh::grass_tuft(6, 0, false, 1),
            1 => mesh::grass_tuft(7, 0, false, 2),
            2 => mesh::grass_tuft(5, 0, true, 3),
            _ => mesh::grass_tuft(4, 2, false, 4),
        };
        l.model.len() as u32
    })
}

#[no_mangle]
pub extern "C" fn model_ptr() -> *const f32 {
    LAB.with(|l| l.borrow().model.as_ptr())
}
