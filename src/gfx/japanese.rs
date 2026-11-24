//! The Japanese countryside's trees and buildings: cherry in blossom,
//! red-leaved Japanese maple, cedar for the wooded hills, cloud-pruned
//! black pine, a thatched farmhouse, a shrine gate and a stone lantern.
//!
//! Trees are one unit tall, like the others in `mesh.rs` (the app sizes
//! them); buildings are in metres, x along the building (its long side
//! faces the road, +z), y up. No GL here: the Model Lab builds them too.

use super::math::*;
use super::mesh::{hued, rnd, rough, tinted, Crown, Mesh, MAT_GLOSS, MAT_LAMP, MAT_LEAF, MAT_MATT};

// ---- trees ------------------------------------------------------------------

/// Cherry bark: dark, a little purple, banded across with pale lenticels.
const CHERRY_BARK: V3 = [0.085, 0.062, 0.06];
/// Blossom: the palest pink, nearly white in the sun.
const BLOSSOM: V3 = [0.6, 0.38, 0.49];
/// Japanese maple: deep crimson.
const MOMIJI: V3 = [0.34, 0.035, 0.045];
const CEDAR_BARK: V3 = [0.16, 0.085, 0.055];
const CEDAR_GREEN: V3 = [0.03, 0.068, 0.042];
const PINE_BARK: V3 = [0.085, 0.075, 0.07];
const PINE_NEEDLES: V3 = [0.032, 0.074, 0.036];

/// Bark banded across, as a cherry's is: every few rings a little paler.
fn banded(col: V3, seed: u32) -> impl Fn(usize, usize) -> V3 {
    move |ring, side| {
        let band = if ring % 3 == 1 { 1.25 } else { 0.92 };
        scale(
            col,
            band * (0.85 + 0.3 * rnd(seed, ring as u32, side as u32)),
        )
    }
}

/// Unit-height cherry in full blossom: a short, dark trunk that forks low
/// into five limbs spreading wide and a little down at their ends, under a
/// broad, soft umbrella of pale pink. Drawn wider than tall, it is the tree
/// people stop under.
pub fn sakura() -> Mesh {
    let mut m = Mesh::default();
    m.sweep(
        &[
            ([0.0, 0.0, 0.0], 0.085),
            ([0.004, 0.035, 0.002], 0.058),
            ([0.014, 0.12, 0.006], 0.05),
            ([0.01, 0.2, 0.0], 0.047),
            ([0.004, 0.26, -0.004], 0.044),
        ],
        8,
        banded(CHERRY_BARK, 3),
    );
    let whole = Crown {
        c: [0.0, 0.62, 0.0],
        r: 0.52,
    };
    let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
    // Five limbs: out low, then rising, their ends bowing under the
    // blossom; each with a twig off to one side.
    let mut ends = Vec::new();
    for k in 0..5 {
        let q = |n: u32| rnd(71, k as u32, n);
        let a = k as f32 * golden * 2.0 + 0.35 * q(1);
        let dir = [a.cos(), 0.0, a.sin()];
        let side = [-dir[2], 0.0, dir[0]];
        let from = [0.006, 0.23 + 0.04 * q(2), 0.0];
        let reach = 0.3 + 0.06 * q(3);
        let elbow = add(from, add(scale(dir, reach * 0.35), [0.0, 0.13, 0.0]));
        let knee = add(
            from,
            add(
                add(scale(dir, reach * 0.72), scale(side, 0.04 * (q(4) - 0.5))),
                [0.0, 0.26 + 0.04 * q(5), 0.0],
            ),
        );
        let end = add(from, add(scale(dir, reach), [0.0, 0.28 + 0.03 * q(6), 0.0]));
        m.sweep(
            &[(from, 0.036), (elbow, 0.027), (knee, 0.017), (end, 0.008)],
            5,
            banded(CHERRY_BARK, 11 + k as u32),
        );
        let turn = if k % 2 == 0 { 0.9 } else { -0.9 };
        let tdir = [(a + turn).cos(), 0.0, (a + turn).sin()];
        let tip = add(
            elbow,
            add(scale(tdir, 0.16), [0.0, 0.27 + 0.04 * q(7), 0.0]),
        );
        let tmid = add(elbow, add(scale(tdir, 0.08), [0.0, 0.14, 0.0]));
        m.sweep(
            &[(elbow, 0.014), (tmid, 0.01), (tip, 0.005)],
            4,
            banded(CHERRY_BARK, 21 + k as u32),
        );
        ends.push((end, dir, tip));
    }
    m.mat = MAT_LEAF;
    for (k, &(end, dir, tip)) in ends.iter().enumerate() {
        // A wide, soft cloud at each limb's end, its underside low so the
        // dark limbs show through beneath.
        m.leaf_mass(
            add(end, add(scale(dir, 0.04), [0.0, 0.01, 0.0])),
            [0.2, 0.12, 0.19],
            6,
            (0.075, 0.115),
            (0.7, 1.0),
            BLOSSOM,
            151 + k as u32,
            whole,
            (4, 6),
        );
        m.leaf_mass(
            add(tip, [0.0, 0.03, 0.0]),
            [0.15, 0.1, 0.14],
            4,
            (0.07, 0.1),
            (0.65, 0.95),
            BLOSSOM,
            161 + k as u32,
            whole,
            (3, 5),
        );
    }
    // The domed top.
    m.leaf_mass(
        [0.0, 0.83, 0.0],
        [0.27, 0.11, 0.26],
        6,
        (0.08, 0.115),
        (0.6, 0.95),
        BLOSSOM,
        171,
        whole,
        (4, 6),
    );
    m
}

/// Unit-height Japanese maple, of the red-leaved kind that is crimson from
/// spring to autumn: a low trunk that splits into three arching stems, and
/// fine foliage held in flat, layered tiers, deep red inside and brighter
/// where the light comes through.
pub fn momiji() -> Mesh {
    let mut m = Mesh::default();
    let bark = [0.1, 0.085, 0.075];
    m.sweep(
        &[
            ([0.0, 0.0, 0.0], 0.06),
            ([0.004, 0.04, 0.0], 0.04),
            ([0.008, 0.12, 0.004], 0.034),
        ],
        7,
        rough(bark, 13),
    );
    let whole = Crown {
        c: [0.0, 0.6, 0.0],
        r: 0.5,
    };
    // Four stems leaning out from the fork and curving up.
    let from = [0.008, 0.11, 0.004];
    let mut stems = Vec::new();
    for k in 0..4 {
        let q = |n: u32| rnd(83, k as u32, n);
        let a = k as f32 * 1.57 + 0.5 * q(1);
        let dir = [a.cos(), 0.0, a.sin()];
        let lean = 0.2 + 0.08 * q(2);
        let mid = add(from, add(scale(dir, lean * 0.55), [0.0, 0.26, 0.0]));
        let top = add(from, add(scale(dir, lean), [0.0, 0.7 + 0.06 * q(3), 0.0]));
        m.sweep(
            &[(from, 0.024), (mid, 0.016), (top, 0.006)],
            5,
            rough(bark, 31 + k as u32),
        );
        stems.push((a, top));
    }
    m.mat = MAT_LEAF;
    // Tiers held flat, like open hands: wide low down, smaller higher up,
    // with light between them. Each stem carries a pad at each tier,
    // reaching out past it.
    let tiers = [(0.36f32, 0.21f32), (0.54, 0.19), (0.71, 0.16), (0.84, 0.12)];
    for (k, &(a, top)) in stems.iter().enumerate() {
        for (t, &(h, w)) in tiers.iter().enumerate() {
            let q = |n: u32| rnd(89, (k * 4 + t) as u32, n);
            let along = ((h - from[1]) / (top[1] - from[1])).clamp(0.0, 1.0);
            let on = lerp(from, top, along);
            let swing = a + 0.6 * (q(1) - 0.5);
            let out = [swing.cos(), 0.0, swing.sin()];
            let c = add(on, scale(out, w * (0.55 + 0.2 * q(2))));
            // Brighter and more scarlet the higher the tier.
            let col = hued(
                MOMIJI,
                0.3 * t as f32 + 0.2 * (q(3) - 0.5),
                0.92 + 0.1 * t as f32,
            );
            m.leaf_mass(
                c,
                [w, w * 0.2, w * 0.9],
                6,
                (0.045, 0.07),
                (0.75, 1.0),
                col,
                181 + (k * 4 + t) as u32,
                whole,
                (3, 6),
            );
        }
    }
    m
}

/// Unit-height Japanese cedar: a straight, reddish trunk to the top and a
/// tall, narrow spire of dark, blue-green tufts, ragged at its edges. In
/// stands it clothes the hillsides; drawn about a third as wide as tall.
pub fn sugi() -> Mesh {
    let mut m = Mesh::default();
    m.sweep(
        &[
            ([0.0, 0.0, 0.0], 0.036),
            ([0.0, 0.03, 0.0], 0.026),
            ([0.002, 0.4, 0.0], 0.02),
            ([0.0, 0.75, 0.002], 0.012),
            ([0.0, 0.97, 0.0], 0.003),
        ],
        6,
        // Bark in long strips: paler and darker by the side, not the ring.
        |_ring, side| scale(CEDAR_BARK, if side % 2 == 0 { 1.15 } else { 0.8 }),
    );
    let whole = Crown {
        c: [0.0, 0.6, 0.0],
        r: 0.42,
    };
    m.mat = MAT_LEAF;
    let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
    // Tufts spiralling up the trunk, the spire narrowing to the top.
    let n = 24;
    for k in 0..n {
        let q = |i: u32| rnd(97, k as u32, i);
        let t = k as f32 / (n - 1) as f32;
        let y = 0.12 + 0.84 * t;
        let r = 0.24 * (1.0 - t) + 0.02;
        let a = k as f32 * golden + q(1);
        let off = r * 0.45;
        let c = [a.cos() * off, y, a.sin() * off];
        let col = hued(CEDAR_GREEN, 0.3 * (q(2) - 0.5) + 0.25 * t, 0.95 + 0.15 * t);
        m.leaf_mass(
            c,
            [r, 0.075 + 0.025 * (1.0 - t), r],
            4,
            (0.04 + 0.035 * (1.0 - t), 0.06 + 0.04 * (1.0 - t)),
            (0.6, 1.0),
            col,
            211 + k as u32,
            whole,
            (3, 5),
        );
    }
    m
}

/// Unit-height black pine, pruned by generations of gardeners: a dark,
/// plated trunk that leans out and turns back, a few long limbs held level,
/// and on them flat cushions of needles, like clouds resting on branches.
pub fn kuromatsu() -> Mesh {
    let mut m = Mesh::default();
    // The trunk leans one way, then turns back towards the top.
    let trunk = [
        ([0.0, 0.0, 0.0], 0.05),
        ([0.01, 0.03, 0.0], 0.04),
        ([0.07, 0.2, 0.01], 0.035),
        ([0.13, 0.38, 0.0], 0.03),
        ([0.12, 0.56, -0.01], 0.024),
        ([0.06, 0.74, 0.0], 0.017),
        ([0.03, 0.86, 0.0], 0.01),
    ];
    m.sweep(&trunk, 7, rough(PINE_BARK, 41));
    let whole = Crown {
        c: [0.05, 0.58, 0.0],
        r: 0.5,
    };
    // Limbs from the trunk, held out level: where they start (a point on
    // the trunk), which way they go, how far, and the pads they carry.
    let limbs: [(usize, f32, f32, f32); 5] = [
        (3, 3.0, 0.4, -0.07),
        (3, 0.3, 0.3, -0.04),
        (4, 4.6, 0.3, -0.02),
        (4, 1.6, 0.25, 0.0),
        (5, 3.5, 0.18, 0.02),
    ];
    let mut pads = Vec::new();
    for (k, &(at, a, reach, rise)) in limbs.iter().enumerate() {
        let q = |n: u32| rnd(53, k as u32, n);
        let from = trunk[at].0;
        let dir = [a.cos(), 0.0, a.sin()];
        let mid = add(from, add(scale(dir, reach * 0.5), [0.0, rise - 0.02, 0.0]));
        let end = add(from, add(scale(dir, reach), [0.0, rise, 0.0]));
        m.sweep(
            &[(from, 0.018), (mid, 0.012), (end, 0.006)],
            5,
            rough(PINE_BARK, 61 + k as u32),
        );
        // A big pad at the end, a smaller one part way along.
        pads.push((add(end, [0.0, 0.035, 0.0]), 0.13 + 0.02 * q(1)));
        let side = [-dir[2], 0.0, dir[0]];
        let s = if k % 2 == 0 { 1.0 } else { -1.0 };
        pads.push((
            add(mid, add(scale(side, 0.05 * s), [0.0, 0.04, 0.0])),
            0.085 + 0.015 * q(2),
        ));
    }
    // The crown pad on top.
    pads.push(([0.035, 0.9, 0.0], 0.12));
    m.mat = MAT_LEAF;
    for (k, &(c, r)) in pads.iter().enumerate() {
        m.leaf_mass(
            c,
            [r, r * 0.24, r * 0.9],
            6,
            (0.045, 0.065),
            (0.7, 1.0),
            PINE_NEEDLES,
            231 + k as u32,
            whole,
            (3, 6),
        );
    }
    m
}

// ---- buildings --------------------------------------------------------------

const DARK_WOOD: V3 = [0.075, 0.05, 0.035];
const PLASTER: V3 = [0.64, 0.61, 0.54];
const PAPER: V3 = [0.7, 0.66, 0.55];
const STONE: V3 = [0.16, 0.155, 0.145];
/// How deep the stone footings go (as for the farm buildings): on a slope
/// a building stands at the top of its ground.
const FOUNDATION: f32 = 3.5;

impl Mesh {
    /// A flat polygon (convex, its points anticlockwise seen from the side
    /// it faces), each point's colour from `col`.
    fn poly(&mut self, pts: &[V3], col: impl Fn(V3) -> V3) {
        if pts.len() < 3 {
            return;
        }
        let n = norm(cross(sub(pts[1], pts[0]), sub(pts[2], pts[0])));
        for i in 1..pts.len() - 1 {
            for p in [pts[0], pts[i], pts[i + 1]] {
                self.push(p, n, col(p));
            }
        }
    }

    /// A prism standing on `y0` up to `y1`: `sides` flat faces, `r0` across
    /// at the bottom and `r1` at the top (corner radius), turned `turn`,
    /// with its top and bottom closed.
    #[allow(clippy::too_many_arguments)]
    fn prism(
        &mut self,
        c: [f32; 2],
        y0: f32,
        y1: f32,
        r0: f32,
        r1: f32,
        sides: usize,
        turn: f32,
        col: V3,
    ) {
        let pt = |i: usize, y: f32, r: f32| {
            let a = turn + i as f32 / sides as f32 * std::f32::consts::TAU;
            [c[0] + a.cos() * r, y, c[1] + a.sin() * r]
        };
        for i in 0..sides {
            let shade = scale(col, 0.92 + 0.16 * rnd(7, i as u32, (y0 * 100.0) as u32));
            self.quad(
                pt(i + 1, y0, r0),
                pt(i, y0, r0),
                pt(i, y1, r1),
                pt(i + 1, y1, r1),
                shade,
            );
            self.tri([c[0], y1, c[1]], pt(i + 1, y1, r1), pt(i, y1, r1), col);
            self.tri(
                [c[0], y0, c[1]],
                pt(i, y0, r0),
                pt(i + 1, y0, r0),
                scale(col, 0.6),
            );
        }
    }

    /// A beam along x from `-half` to `half`, `h` deep and `w` thick
    /// (across z), its middle at `y` and its ends swept up by `lift` (as a
    /// shrine gate's top beams are), cut square.
    #[allow(clippy::too_many_arguments)]
    fn swept_beam(&mut self, half: f32, y: f32, h: f32, w: f32, lift: f32, z: f32, col: V3) {
        let n = 12;
        let at = |i: usize| {
            let x = -half + 2.0 * half * i as f32 / n as f32;
            let u = (x / half).abs();
            (x, y + lift * u.powi(3))
        };
        let (z0, z1) = (z - w / 2.0, z + w / 2.0);
        for i in 0..n {
            let ((xa, ya), (xb, yb)) = (at(i), at(i + 1));
            let (ta, tb, ba, bb) = (ya + h / 2.0, yb + h / 2.0, ya - h / 2.0, yb - h / 2.0);
            self.quad([xa, ta, z1], [xa, ta, z0], [xb, tb, z0], [xb, tb, z1], col);
            self.quad(
                [xa, ba, z0],
                [xa, ba, z1],
                [xb, bb, z1],
                [xb, bb, z0],
                scale(col, 0.55),
            );
            self.quad([xa, ba, z1], [xb, bb, z1], [xb, tb, z1], [xa, ta, z1], col);
            self.quad([xb, bb, z0], [xa, ba, z0], [xa, ta, z0], [xb, tb, z0], col);
        }
        for (i, s) in [(0, -1.0f32), (n, 1.0)] {
            let (x, yy) = at(i);
            let (t, b) = (yy + h / 2.0, yy - h / 2.0);
            if s > 0.0 {
                self.quad([x, b, z1], [x, b, z0], [x, t, z0], [x, t, z1], col);
            } else {
                self.quad([x, b, z0], [x, b, z1], [x, t, z1], [x, t, z0], col);
            }
        }
    }
}

/// The parts of a convex polygon between heights `y0` and `y1`.
fn slab(pts: &[V3], y0: f32, y1: f32) -> Vec<V3> {
    let clip = |pts: Vec<V3>, keep: &dyn Fn(V3) -> f32| -> Vec<V3> {
        let mut out = Vec::new();
        for i in 0..pts.len() {
            let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
            let (da, db) = (keep(a), keep(b));
            if da >= 0.0 {
                out.push(a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                out.push(lerp(a, b, da / (da - db)));
            }
        }
        out
    };
    let above = clip(pts.to_vec(), &|p| p[1] - y0);
    clip(above, &|p| y1 - p[1])
}

/// The thatched farmhouse (minka): 12 m long, timber-framed, plastered
/// above and boarded below, raised on stone, under a great hipped roof of
/// thatch, a small gable at each end of its ridge and the ridge capped and
/// saddled in dark timber. Paper screens line a veranda along the front;
/// two stand open on the warm, lamp-lit room inside. Stepping stones,
/// a stack of firewood and potted plants make it lived in.
pub fn minka() -> Mesh {
    let mut m = Mesh::default();
    let (len, dep) = (12.0f32, 8.0f32);
    let (hx, hz) = (len / 2.0, dep / 2.0);
    let floor = 0.6;
    let eave_wall = 2.95;
    // Stone footing, raising the floor off the ground.
    m.cuboid(
        [-hx - 0.1, -FOUNDATION, -hz - 0.1],
        [hx + 0.1, floor, hz + 0.1],
        STONE,
    );
    // Walls: boarded below, plastered above, framed by dark posts and a
    // beam at the head of the boards.
    let board_top = 1.75;
    m.cuboid([-hx, floor, -hz], [hx, board_top, hz], [0.11, 0.075, 0.05]);
    m.cuboid(
        [-hx + 0.02, board_top, -hz + 0.02],
        [hx - 0.02, eave_wall, hz - 0.02],
        PLASTER,
    );
    let post = 0.11;
    let step = 2.0;
    let mut x = -hx;
    while x <= hx + 0.01 {
        for z in [-hz, hz] {
            m.cuboid(
                [x - post, floor, z - post],
                [x + post, eave_wall, z + post],
                DARK_WOOD,
            );
        }
        x += step;
    }
    for x in [-hx, hx] {
        let mut z = -hz + step;
        while z < hz - 0.01 {
            m.cuboid(
                [x - post, floor, z - post],
                [x + post, eave_wall, z + post],
                DARK_WOOD,
            );
            z += step;
        }
    }
    for (y0, y1) in [
        (board_top - 0.06, board_top + 0.06),
        (eave_wall - 0.18, eave_wall),
    ] {
        m.cuboid(
            [-hx - 0.05, y0, -hz - 0.05],
            [hx + 0.05, y1, hz + 0.05],
            DARK_WOOD,
        );
    }
    // The front: a veranda (engawa) the length of the house, and paper
    // screens (shoji) in place of the wall, two of them open.
    let front = hz;
    m.cuboid(
        [-hx + 0.3, floor - 0.08, front],
        [hx - 0.3, floor, front + 1.0],
        [0.2, 0.135, 0.085],
    );
    for k in 0..12 {
        // The deck's boards.
        let x0 = -hx + 0.3 + k as f32 * (len - 0.6) / 12.0;
        m.cuboid(
            [x0, floor, front],
            [x0 + 0.015, floor + 0.005, front + 1.0],
            [0.12, 0.08, 0.05],
        );
    }
    for x in [-hx + 0.4, -2.0, 2.0, hx - 0.4] {
        m.cuboid(
            [x - 0.06, -0.3, front + 0.85],
            [x + 0.06, floor, front + 0.97],
            DARK_WOOD,
        );
    }
    let panes = 8;
    let (sx0, sx1) = (-hx + 0.4, hx - 0.4);
    let pw = (sx1 - sx0) / panes as f32;
    let (sy0, sy1) = (floor + 0.05, 2.55);
    let z = front + 0.012;
    for k in 0..panes {
        let x0 = sx0 + k as f32 * pw;
        let open = k == 3 || k == 4;
        if open {
            // The room inside: tatami, a dark back wall, a lantern's glow.
            m.quad(
                [x0, sy0, z - 0.02],
                [x0 + pw, sy0, z - 0.02],
                [x0 + pw, sy1, z - 0.02],
                [x0, sy1, z - 0.02],
                [0.03, 0.022, 0.016],
            );
            m.quad(
                [x0, sy0 + 0.01, z],
                [x0 + pw, sy0 + 0.01, z],
                [x0 + pw, sy0 + 0.01, z - 1.2],
                [x0, sy0 + 0.01, z - 1.2],
                [0.3, 0.28, 0.16],
            );
            if k == 3 {
                m.mat = MAT_LAMP;
                m.prism(
                    [x0 + pw, z - 0.9],
                    sy0 + 1.25,
                    sy0 + 1.6,
                    0.13,
                    0.13,
                    8,
                    0.0,
                    [0.5, 0.33, 0.15],
                );
                m.mat = MAT_MATT;
            }
            continue;
        }
        // Paper in a lattice of thin bars.
        m.quad(
            [x0, sy0, z],
            [x0 + pw, sy0, z],
            [x0 + pw, sy1, z],
            [x0, sy1, z],
            PAPER,
        );
        let bar = 0.018;
        let zb = z + 0.012;
        m.quad(
            [x0, sy0, zb],
            [x0 + 0.05, sy0, zb],
            [x0 + 0.05, sy1, zb],
            [x0, sy1, zb],
            DARK_WOOD,
        );
        m.quad(
            [x0 + pw - 0.05, sy0, zb],
            [x0 + pw, sy0, zb],
            [x0 + pw, sy1, zb],
            [x0 + pw - 0.05, sy1, zb],
            DARK_WOOD,
        );
        m.quad(
            [x0, sy0, zb],
            [x0 + pw, sy0, zb],
            [x0 + pw, sy0 + 0.3, zb],
            [x0, sy0 + 0.3, zb],
            [0.14, 0.095, 0.06],
        );
        for c in 1..3 {
            let xc = x0 + pw * c as f32 / 3.0;
            m.quad(
                [xc - bar, sy0 + 0.3, zb],
                [xc + bar, sy0 + 0.3, zb],
                [xc + bar, sy1, zb],
                [xc - bar, sy1, zb],
                DARK_WOOD,
            );
        }
        for r in 1..6 {
            let yr = sy0 + 0.3 + (sy1 - sy0 - 0.3) * r as f32 / 6.0;
            m.quad(
                [x0, yr - bar, zb],
                [x0 + pw, yr - bar, zb],
                [x0 + pw, yr + bar, zb],
                [x0, yr + bar, zb],
                DARK_WOOD,
            );
        }
    }
    // The roof: thatch, hipped, with a small gable at each end of the
    // ridge (irimoya), all one steep pitch, thick at its edges.
    let over = 1.35;
    let (ex, ez) = (hx + over, hz + over);
    let pitch = 1.15;
    let ey = eave_wall + 0.25;
    let ridge = ey + pitch * ez;
    // Where the hips meet the gables, a third of the way down.
    let gz = ez * 0.36;
    let gy = ridge - pitch * gz;
    let gx = ex - (ez - gz);
    let thick = 0.75;
    let straw = tinted([0.3, 0.215, 0.115]);
    let aged = tinted([0.15, 0.11, 0.07]);
    // Darker low down, where rain runs off, in courses of thatch.
    let course = move |p: V3| {
        let t = ((p[1] - ey) / (ridge - ey)).clamp(0.0, 1.0);
        let band = ((p[1] - ey) / 0.45).floor() as i32;
        let k = if band % 2 == 0 { 1.0 } else { 0.8 };
        let grain = 0.9 + 0.2 * rnd(23, band as u32, (p[0] * 3.0 + p[2] * 3.0) as i32 as u32);
        scale(lerp(aged, straw, 0.25 + 0.75 * t), k * grain)
    };
    let front_face = [
        [-ex, ey, ez],
        [ex, ey, ez],
        [gx, gy, gz],
        [gx, ridge, 0.0],
        [-gx, ridge, 0.0],
        [-gx, gy, gz],
    ];
    let back_face = front_face.map(|p| [-p[0], p[1], -p[2]]);
    let east_face = [[ex, ey, ez], [ex, ey, -ez], [gx, gy, -gz], [gx, gy, gz]];
    let west_face = east_face.map(|p| [-p[0], p[1], -p[2]]);
    for face in [
        &front_face[..],
        &back_face[..],
        &east_face[..],
        &west_face[..],
    ] {
        let mut y = ey;
        while y < ridge {
            let band = slab(face, y, y + 0.45);
            m.poly(&band, course);
            y += 0.45;
        }
        // Its underside, a thatch's thickness lower.
        let under: Vec<V3> = face
            .iter()
            .rev()
            .map(|p| [p[0], p[1] - thick, p[2]])
            .collect();
        m.poly(&under, |_| tinted([0.1, 0.075, 0.045]));
    }
    // The cut edge of the thatch round the eaves: paler, fresher straw.
    let edge = tinted([0.38, 0.29, 0.16]);
    let rim = [[-ex, ey, ez], [ex, ey, ez], [ex, ey, -ez], [-ex, ey, -ez]];
    for i in 0..4 {
        let (a, b) = (rim[i], rim[(i + 1) % 4]);
        let (a1, b1) = ([a[0], a[1] - thick, a[2]], [b[0], b[1] - thick, b[2]]);
        m.quad(a1, b1, b, a, edge);
    }
    // The gables: dark boards with a latticed smoke vent.
    for s in [1.0f32, -1.0] {
        let x = gx * s;
        let (a, b) = ([x, gy, gz * s], [x, gy, -gz * s]);
        m.tri(a, b, [x, ridge, 0.0], DARK_WOOD);
        let x2 = x + 0.03 * s;
        for k in 0..5 {
            let zz = -gz * 0.5 + gz * k as f32 / 4.0;
            let top = ridge - (zz.abs() / gz) * (ridge - gy);
            let (zl, zr) = if s > 0.0 {
                (zz + 0.03, zz - 0.03)
            } else {
                (zz - 0.03, zz + 0.03)
            };
            m.quad(
                [x2, gy + 0.2, zl],
                [x2, gy + 0.2, zr],
                [x2, top - 0.25, zr],
                [x2, top - 0.25, zl],
                [0.2, 0.14, 0.09],
            );
        }
        // The gable's little roof of thatch over the vent.
        let (p, q) = (
            [x + 0.35 * s, gy - 0.05, gz + 0.3],
            [x + 0.35 * s, gy - 0.05, -gz - 0.3],
        );
        if s > 0.0 {
            m.quad(
                q,
                p,
                [x, gy + 0.25, gz + 0.3],
                [x, gy + 0.25, -gz - 0.3],
                straw,
            );
        } else {
            m.quad(
                p,
                q,
                [x, gy + 0.25, -gz - 0.3],
                [x, gy + 0.25, gz + 0.3],
                straw,
            );
        }
    }
    // The ridge: a dark cap, saddled every metre with crossed timbers.
    m.cuboid(
        [-gx - 0.45, ridge - 0.1, -0.42],
        [gx + 0.45, ridge + 0.32, 0.42],
        [0.05, 0.04, 0.03],
    );
    m.cuboid(
        [-gx - 0.5, ridge + 0.32, -0.12],
        [gx + 0.5, ridge + 0.42, 0.12],
        DARK_WOOD,
    );
    let saddles = (2.0 * gx / 1.0) as i32;
    for k in 0..=saddles {
        let x = -gx + 2.0 * gx * k as f32 / saddles as f32;
        m.cuboid(
            [x - 0.07, ridge - 0.25, -0.62],
            [x + 0.07, ridge + 0.38, 0.62],
            [0.09, 0.065, 0.045],
        );
    }
    // Stepping stones up to the veranda, a stack of firewood under the
    // eaves at one end, and plants in glazed pots.
    for (k, &(x, z)) in [(0.3f32, 1.6f32), (-0.2, 2.5), (0.25, 3.4), (-0.1, 4.3)]
        .iter()
        .enumerate()
    {
        m.boulder(
            [x, -0.02, front + z],
            [0.34, 0.09, 0.3],
            3,
            7,
            STONE,
            300 + k as u32,
        );
    }
    m.cuboid([0.8, -0.2, front + 1.05], [1.6, 0.32, front + 1.5], STONE);
    for k in 0..4 {
        let y = 0.12 + k as f32 * 0.24;
        for j in 0..6 {
            let z = -2.6 + j as f32 * 0.3 + if k % 2 == 0 { 0.0 } else { 0.15 };
            let tone = 0.85 + 0.3 * rnd(17, k, j);
            m.tube(
                [hx + 0.15, y, z],
                [hx + 1.0, y, z],
                0.12,
                0.12,
                6,
                scale([0.3, 0.2, 0.12], tone),
            );
        }
    }
    for (k, x) in [-4.6f32, -3.9, 4.2].into_iter().enumerate() {
        let pot = if k == 1 {
            [0.06, 0.09, 0.14]
        } else {
            [0.24, 0.12, 0.06]
        };
        m.mat = MAT_GLOSS;
        m.prism([x, front + 1.35], 0.0, 0.42, 0.2, 0.26, 10, 0.0, pot);
        m.mat = MAT_LEAF;
        m.ellipsoid_in(
            [x, 0.65, front + 1.35],
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            [0.3, 0.26, 0.3],
            4,
            7,
            tinted([0.06, 0.13, 0.04]),
        );
        m.mat = MAT_MATT;
    }
    m
}

/// A shrine gate (torii) in vermilion: two pillars on black feet, leaning
/// in a little, a tie beam through them, and the great top beam swept up
/// at its ends and capped in black, with the shrine's name board between.
/// 5 m tall and 5.4 m across the top.
pub fn torii() -> Mesh {
    let mut m = Mesh::default();
    let red = [0.6, 0.13, 0.05];
    let black = [0.025, 0.022, 0.02];
    let (base, top) = (1.75f32, 1.62f32);
    for s in [-1.0f32, 1.0] {
        // Stone footings, black feet, then the pillar.
        m.prism(
            [s * base, 0.0],
            -FOUNDATION,
            0.08,
            0.34,
            0.34,
            8,
            0.2,
            STONE,
        );
        m.tube(
            [s * base, 0.0, 0.0],
            [s * base, 0.55, 0.0],
            0.23,
            0.23,
            12,
            black,
        );
        m.mat = MAT_GLOSS;
        m.tube(
            [s * base, 0.55, 0.0],
            [s * top, 4.25, 0.0],
            0.19,
            0.165,
            12,
            red,
        );
        m.mat = MAT_MATT;
    }
    m.mat = MAT_GLOSS;
    // The tie beam, through the pillars and out past them.
    m.cuboid([-2.35, 3.55, -0.1], [2.35, 3.85, 0.1], red);
    // The name board on its strut between the beams.
    m.cuboid([-0.1, 3.85, -0.08], [0.1, 4.25, 0.08], red);
    m.mat = MAT_MATT;
    m.cuboid([-0.36, 3.88, 0.08], [0.36, 4.5, 0.14], black);
    m.cuboid([-0.3, 3.94, 0.14], [0.3, 4.44, 0.15], [0.42, 0.33, 0.12]);
    // The top beams: the lower in vermilion, the cap above it black, both
    // swept up towards their ends.
    m.mat = MAT_GLOSS;
    m.swept_beam(2.55, 4.38, 0.26, 0.34, 0.22, 0.0, red);
    m.swept_beam(2.75, 4.66, 0.3, 0.42, 0.32, 0.0, black);
    m.mat = MAT_MATT;
    m
}

/// A stone lantern (toro) of the old shrine kind, 2.3 m tall: a six-sided
/// base, a round post, a platform, the firebox with its windows lit, a
/// broad roof turned up at its corners and a jewel on top. Moss gathers on
/// the roof.
pub fn toro() -> Mesh {
    let mut m = Mesh::default();
    let stone = [0.175, 0.168, 0.155];
    let dark = scale(stone, 0.75);
    let turn = std::f32::consts::FRAC_PI_6;
    m.prism([0.0, 0.0], -0.3, 0.12, 0.5, 0.5, 6, turn, dark);
    m.prism([0.0, 0.0], 0.12, 0.3, 0.42, 0.3, 6, turn, stone);
    m.prism([0.0, 0.0], 0.3, 0.92, 0.17, 0.15, 12, 0.0, stone);
    m.prism([0.0, 0.0], 0.92, 1.1, 0.3, 0.42, 6, turn, stone);
    // The firebox: its six faces, two of them open on the light inside.
    let (y0, y1) = (1.1f32, 1.5f32);
    let r = 0.27;
    m.mat = MAT_LAMP;
    m.prism(
        [0.0, 0.0],
        y0 + 0.04,
        y1 - 0.04,
        0.18,
        0.18,
        6,
        turn,
        [0.55, 0.36, 0.14],
    );
    m.mat = MAT_MATT;
    for i in 0..6 {
        let a0 = turn + i as f32 / 6.0 * std::f32::consts::TAU;
        let a1 = turn + (i + 1) as f32 / 6.0 * std::f32::consts::TAU;
        let p = |a: f32, y: f32| [a.cos() * r, y, a.sin() * r];
        if i % 3 == 0 {
            // A window: stone only round its edges.
            let am = |t: f32| a0 + (a1 - a0) * t;
            m.quad(
                p(a1, y0),
                p(a0, y0),
                p(a0, y0 + 0.06),
                p(a1, y0 + 0.06),
                stone,
            );
            m.quad(
                p(a1, y1 - 0.06),
                p(a0, y1 - 0.06),
                p(a0, y1),
                p(a1, y1),
                stone,
            );
            m.quad(
                p(am(0.18), y0),
                p(a0, y0),
                p(a0, y1),
                p(am(0.18), y1),
                stone,
            );
            m.quad(
                p(a1, y0),
                p(am(0.82), y0),
                p(am(0.82), y1),
                p(a1, y1),
                stone,
            );
        } else {
            m.quad(p(a1, y0), p(a0, y0), p(a0, y1), p(a1, y1), stone);
        }
    }
    // The roof: broad and low, each corner turned up, moss on top.
    let (ry, rr) = (1.54f32, 0.64f32);
    m.prism([0.0, 0.0], y1, ry, 0.3, 0.3, 6, turn, dark);
    let moss = [0.1, 0.14, 0.05];
    for i in 0..6 {
        let a0 = turn + i as f32 / 6.0 * std::f32::consts::TAU;
        let a1 = turn + (i + 1) as f32 / 6.0 * std::f32::consts::TAU;
        let am = (a0 + a1) / 2.0;
        let corner = |a: f32| [a.cos() * rr, ry + 0.14, a.sin() * rr];
        let mid = [am.cos() * rr * 0.9, ry, am.sin() * rr * 0.9];
        let apex = [0.0, ry + 0.32, 0.0];
        let tone = |k: f32| lerp(stone, moss, k);
        m.tri(apex, mid, corner(a0), tone(0.35));
        m.tri(apex, corner(a1), mid, tone(0.45));
        // The underside and the roof's thick edge.
        m.tri([0.0, ry - 0.02, 0.0], corner(a0), mid, scale(dark, 0.7));
        m.tri([0.0, ry - 0.02, 0.0], mid, corner(a1), scale(dark, 0.7));
        let down = |p: V3| [p[0], p[1] - 0.07, p[2]];
        m.quad(down(corner(a0)), down(mid), mid, corner(a0), dark);
        m.quad(down(mid), down(corner(a1)), corner(a1), mid, dark);
    }
    // The jewel on top.
    m.prism([0.0, 0.0], ry + 0.3, ry + 0.38, 0.1, 0.12, 8, 0.0, stone);
    m.ellipsoid_in(
        [0.0, ry + 0.5, 0.0],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        [0.1, 0.13, 0.1],
        5,
        8,
        stone,
    );
    m
}
