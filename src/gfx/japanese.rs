//! The Japanese countryside's trees and buildings: cherry in blossom,
//! red-leaved Japanese maple, cedar for the wooded hills, cloud-pruned
//! black pine, a thatched farmhouse, a shrine gate, a five-storey pagoda
//! and a stone lantern.
//!
//! Trees are one unit tall, like the others in `mesh.rs` (the app sizes
//! them); buildings are in metres, x along the building (its long side
//! faces the road, +z), y up. No GL here: the Model Lab builds them too.

use super::body::MAT_FABRIC;
use super::math::*;
use super::mesh::{
    hued, rnd, rough, tinted, Crown, Mesh, MAT_GLOSS, MAT_LAMP, MAT_LEAF, MAT_MATT, VERTEX_STRIDE,
};

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

/// One tier or cushion of foliage made the way the pine's tiers are
/// (`whorl` in `mesh.rs`): a low cone of branches round `at`, from its
/// drooping, ragged rim at `y0` up to `y1` over its middle, paler at the
/// tips and shaded beneath. Closed all round, it leaves no gaps.
#[allow(clippy::too_many_arguments)]
fn cushion(
    m: &mut Mesh,
    at: [f32; 2],
    y0: f32,
    y1: f32,
    r: f32,
    sides: usize,
    col: V3,
    seed: u32,
    turn: f32,
) {
    let rim = |i: usize| -> (V3, V3, V3) {
        let i = i % sides;
        let q = |n: u32| rnd(seed, i as u32, n);
        let a = (i as f32 + 0.35 * (q(2) - 0.5)) / sides as f32 * std::f32::consts::TAU + turn;
        let tip = i & 1 == 0;
        let k = if tip {
            0.9 + 0.2 * q(1)
        } else {
            0.62 + 0.16 * q(1)
        };
        let droop = (y1 - y0) * if tip { 0.12 + 0.2 * q(3) } else { 0.04 };
        let p = [at[0] + a.cos() * r * k, y0 - droop, at[1] + a.sin() * r * k];
        let n = norm([a.cos() * 0.8, 0.6, a.sin() * 0.8]);
        let c = if tip {
            hued(col, 0.2 * q(4), 1.15 + 0.06 * q(4))
        } else {
            scale(col, 0.8)
        };
        (p, n, c)
    };
    let apex = [at[0], y1, at[1]];
    let under = [at[0], y0 + (y1 - y0) * 0.22, at[1]];
    for i in 0..sides {
        let (a, na, ca) = rim(i);
        let (b, nb, cb) = rim(i + 1);
        let nt = norm(add(add(na, nb), [0.0, 1.2, 0.0]));
        m.push(apex, nt, scale(col, 0.7));
        m.push(a, na, ca);
        m.push(b, nb, cb);
        let down = [0.0, -1.0, 0.0];
        m.push(under, down, scale(col, 0.3));
        m.push(b, down, scale(col, 0.45));
        m.push(a, down, scale(col, 0.45));
    }
}

/// Unit-height cherry in full blossom: a short, dark trunk that forks low
/// into five limbs spreading wide, a leader rising through the middle, and
/// over them a broad, soft umbrella of pale pink, every cloud of blossom
/// on a limb or twig of its own. Drawn wider than tall, it is the tree
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
            ([0.0, 0.5, 0.004], 0.026),
            ([0.004, 0.68, 0.0], 0.016),
            ([0.0, 0.8, 0.0], 0.006),
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
        // A wide, soft cloud round each limb's end, its underside low so
        // the dark limbs show beneath.
        m.leaf_mass(
            add(end, add(scale(dir, 0.02), [0.0, 0.01, 0.0])),
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
            add(tip, [0.0, 0.02, 0.0]),
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
    // The middle of the crown, round the leader, closing it between the
    // limbs' clouds; and the domed top on the leader's end.
    m.leaf_mass(
        [0.0, 0.64, 0.0],
        [0.28, 0.12, 0.27],
        6,
        (0.085, 0.12),
        (0.55, 0.85),
        BLOSSOM,
        169,
        whole,
        (4, 6),
    );
    m.leaf_mass(
        [0.0, 0.8, 0.0],
        [0.25, 0.11, 0.24],
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

/// One dome of a pine's cloud of needles: round `c` (the middle of its
/// flat underside), `r` across and `h` high, its outline lumpy with tufts,
/// darker towards its rim and underneath, paler where the new growth is
/// on top. Closed all round.
#[allow(clippy::too_many_arguments)]
fn needle_dome(m: &mut Mesh, c: V3, r: f32, h: f32, col: V3, seed: u32) {
    let (rings, segs) = (3usize, 12usize);
    let pt = |i: usize, j: usize| -> (V3, V3, V3) {
        let j = j % segs;
        let th = i as f32 / rings as f32 * std::f32::consts::FRAC_PI_2;
        let a = (j as f32 + 0.5 * (i % 2) as f32) / segs as f32 * std::f32::consts::TAU;
        // Tufts: the rim and the rings above it bulge in and out.
        let tuft = if i == rings {
            1.0
        } else {
            1.0 + 0.22 * (rnd(seed, i as u32, j as u32) - 0.5)
        };
        let (rr, y) = (
            r * th.cos() * tuft,
            h * th.sin() * (0.9 + 0.2 * rnd(seed + 1, i as u32, j as u32)),
        );
        let p = [c[0] + a.cos() * rr, c[1] + y, c[2] + a.sin() * rr];
        let n = norm([a.cos() * th.cos() / r, th.sin() / h, a.sin() * th.cos() / r]);
        let up = i as f32 / rings as f32;
        let shade = (0.62 + 0.55 * up) * (0.9 + 0.2 * rnd(seed + 2, i as u32, j as u32));
        let col = if i + 1 >= rings && rnd(seed + 3, i as u32, j as u32) > 0.6 {
            hued(col, 0.35, 1.2)
        } else {
            col
        };
        (p, n, scale(col, shade))
    };
    for i in 0..rings {
        for j in 0..segs {
            let q = [pt(i, j), pt(i + 1, j), pt(i + 1, j + 1), pt(i, j + 1)];
            for idx in [0, 2, 1, 0, 3, 2] {
                let (p, n, cc) = q[idx];
                m.push(p, n, cc);
            }
        }
        if i == 0 {
            // The flat, shaded underside.
            for j in 0..segs {
                let (a, b) = (pt(0, j), pt(0, j + 1));
                let down = [0.0, -1.0, 0.0];
                m.push([c[0], c[1] - h * 0.08, c[2]], down, scale(col, 0.3));
                m.push(a.0, down, scale(col, 0.4));
                m.push(b.0, down, scale(col, 0.4));
            }
        }
    }
}

/// A cloud of needles, as pruned pines carry them: a broad dome with two
/// smaller ones swelling from its sides, all overlapping into one closed,
/// rounded cushion, flat beneath.
fn needle_cloud(m: &mut Mesh, c: V3, r: f32, col: V3, seed: u32) {
    needle_dome(m, c, r, r * 0.42, col, seed);
    let a = rnd(seed, 0, 9) * std::f32::consts::TAU;
    for k in 0..2 {
        let b = a + k as f32 * 2.6;
        let at = [
            c[0] + b.cos() * r * 0.5,
            c[1] + r * 0.06,
            c[2] + b.sin() * r * 0.5,
        ];
        needle_dome(
            m,
            at,
            r * 0.58,
            r * 0.4,
            hued(col, 0.1, 1.04),
            seed + 11 + k,
        );
    }
}

/// Unit-height Japanese maple, of the red-leaved kind that is crimson from
/// spring to autumn: a slender, gently winding trunk that rises through the
/// middle of the crown, four stems growing out of it low down and curving
/// up and out, and fine foliage in layered tiers that overlap into one
/// crown, each pad on a twig of its own; deep red inside, brighter where
/// the light comes through.
pub fn momiji() -> Mesh {
    let mut m = Mesh::default();
    let bark = [0.1, 0.085, 0.075];
    // The trunk, in one piece from the ground to the top of the crown: a
    // little flare at its foot, then slim, winding gently.
    let trunk = [
        ([0.0, 0.0, 0.0], 0.042),
        ([0.002, 0.025, 0.0], 0.03),
        ([0.008, 0.12, 0.004], 0.025),
        ([0.002, 0.24, -0.004], 0.02),
        ([-0.006, 0.42, 0.002], 0.014),
        ([0.0, 0.64, 0.004], 0.009),
        ([0.004, 0.86, 0.0], 0.004),
    ];
    m.sweep(&trunk, 7, rough(bark, 13));
    let whole = Crown {
        c: [0.0, 0.6, 0.0],
        r: 0.5,
    };
    let on_trunk = |y: f32| {
        let k = trunk
            .iter()
            .position(|p| p.0[1] >= y)
            .unwrap_or(trunk.len() - 1)
            .max(1);
        let (a, b) = (trunk[k - 1].0, trunk[k].0);
        lerp(a, b, ((y - a[1]) / (b[1] - a[1])).clamp(0.0, 1.0))
    };
    let tiers = [(0.36f32, 0.2f32), (0.52, 0.19), (0.67, 0.16), (0.8, 0.12)];
    let mut pads = Vec::new();
    for k in 0..4 {
        let q = |n: u32| rnd(83, k as u32, n);
        let a = k as f32 * 1.57 + 0.5 * q(1);
        let dir = [a.cos(), 0.0, a.sin()];
        // Out of the trunk at a narrow angle, then curving out and up.
        let from = on_trunk(0.12 + 0.05 * k as f32);
        let lean = 0.16 + 0.05 * q(2);
        let p1 = add(from, add(scale(dir, 0.025), [0.0, 0.07, 0.0]));
        let p2 = add(from, add(scale(dir, lean * 0.6), [0.0, 0.25, 0.0]));
        let top = add(from, add(scale(dir, lean), [0.0, 0.58 + 0.05 * q(3), 0.0]));
        m.sweep(
            &[(from, 0.018), (p1, 0.015), (p2, 0.011), (top, 0.005)],
            5,
            rough(bark, 31 + k as u32),
        );
        let path = [from, p1, p2, top];
        let on_stem = |y: f32| {
            let i = path.iter().position(|p| p[1] >= y).unwrap_or(3).max(1);
            let (a, b) = (path[i - 1], path[i]);
            lerp(a, b, ((y - a[1]) / (b[1] - a[1])).clamp(0.0, 1.0))
        };
        for (t, &(h, w)) in tiers.iter().enumerate() {
            let q = |n: u32| rnd(89, (k * 4 + t) as u32, n);
            let on = on_stem(h.min(top[1]));
            let swing = a + 0.5 * (q(1) - 0.5);
            let out = [swing.cos(), 0.0, swing.sin()];
            // The top tier sits on the stem's end; the others reach out
            // from it on a twig.
            let c = if h >= top[1] {
                add(top, [0.0, 0.02, 0.0])
            } else {
                add(on, add(scale(out, w * 0.3), [0.0, 0.02, 0.0]))
            };
            if h < top[1] {
                m.sweep(
                    &[(on, 0.008), (c, 0.004)],
                    4,
                    rough(bark, 41 + (k * 4 + t) as u32),
                );
            }
            pads.push((c, w, t, k * 4 + t));
        }
    }
    // The trunk carries a pad at each tier too, closing the crown.
    for (t, &(h, w)) in tiers.iter().enumerate() {
        pads.push((add(on_trunk(h), [0.0, 0.02, 0.0]), w * 0.8, t, 16 + t));
    }
    m.mat = MAT_LEAF;
    for &(c, w, t, n) in &pads {
        let q = |i: u32| rnd(97, n as u32, i);
        // Brighter and more scarlet the higher the tier.
        let col = hued(
            MOMIJI,
            0.3 * t as f32 + 0.2 * (q(1) - 0.5),
            0.92 + 0.1 * t as f32,
        );
        m.leaf_mass(
            c,
            [w, w * 0.42, w * 0.9],
            5,
            (0.06, 0.09),
            (0.5, 0.8),
            col,
            181 + n as u32,
            whole,
            (3, 6),
        );
    }
    m
}

/// Unit-height Japanese cedar: a straight, reddish trunk to the top inside
/// a tall, narrow spire of dark blue-green, built in overlapping tiers the
/// way the pine is, so its outline is closed and only its tips are ragged.
/// In stands it clothes the hillsides; drawn about a third as wide as tall.
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
    m.mat = MAT_LEAF;
    let tiers = 16;
    for k in 0..tiers {
        let t = k as f32 / (tiers - 1) as f32;
        let q = |n: u32| rnd(97, k as u32, n);
        let y0 = 0.1 + 0.8 * t + 0.012 * (q(1) - 0.5);
        let vary = 1.0 + (0.03 + 0.08 * t) * (2.0 * q(2) - 1.0);
        let r = (0.2 * (1.0 - t).powf(0.95) + 0.035) * vary;
        let y1 = if k + 1 == tiers {
            1.0
        } else {
            y0 + 0.1 - 0.03 * t
        };
        let lean = q(6) * std::f32::consts::TAU;
        let off = 0.008 * (1.0 - t) * q(7);
        let at = [lean.cos() * off, lean.sin() * off];
        let col = hued(CEDAR_GREEN, 0.3 * (q(3) - 0.5) + 0.15 * t, 0.8 + 0.45 * t);
        let turn = k as f32 * 2.399_963 + 0.3 * q(4);
        cushion(&mut m, at, y0, y1, r, 12, tinted(col), 300 + k as u32, turn);
    }
    m
}

/// Unit-height black pine, pruned by generations of gardeners: a dark,
/// plated trunk that leans out and turns back, a few long limbs held level,
/// and on each limb and twig a rounded cloud of needles resting on it,
/// flat beneath, lumpy with tufts and paler with new growth on top.
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
    // Limbs from the trunk, held out level: where they start (a point on
    // the trunk), which way they go, how far and how they rise.
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
        // A big cushion on the limb's end; a smaller one on a twig from
        // part way along.
        pads.push((end, 0.13 + 0.02 * q(1)));
        let side = [-dir[2], 0.0, dir[0]];
        let s = if k % 2 == 0 { 1.0 } else { -1.0 };
        let twig = add(mid, add(scale(side, 0.07 * s), [0.0, 0.03, 0.0]));
        m.sweep(
            &[(mid, 0.008), (twig, 0.004)],
            4,
            rough(PINE_BARK, 71 + k as u32),
        );
        pads.push((twig, 0.085 + 0.015 * q(2)));
    }
    // The crown cushion on the trunk's top.
    pads.push((trunk[6].0, 0.12));
    m.mat = MAT_LEAF;
    for (k, &(c, r)) in pads.iter().enumerate() {
        let q = |n: u32| rnd(59, k as u32, n);
        let col = tinted(hued(PINE_NEEDLES, 0.3 * (q(1) - 0.5), 0.9 + 0.2 * q(2)));
        // The cloud sits on the branch, its flat underside just below it.
        needle_cloud(
            &mut m,
            add(c, [0.0, -r * 0.08, 0.0]),
            r * 1.05,
            col,
            400 + 7 * k as u32,
        );
    }
    m
}

// ---- buildings --------------------------------------------------------------

const DARK_WOOD: V3 = [0.075, 0.05, 0.035];
const PLASTER: V3 = [0.64, 0.61, 0.54];
const PAPER: V3 = [0.7, 0.66, 0.55];
const STONE: V3 = [0.16, 0.155, 0.145];
/// Shrine and temple lacquer.
const VERMILION: V3 = [0.6, 0.11, 0.045];
/// Copper roof tiles gone turquoise with age, and their ridges.
const TILE: V3 = [0.085, 0.215, 0.205];
const RIDGE: V3 = [0.045, 0.11, 0.105];
const UNDER_EAVES: V3 = [0.16, 0.045, 0.03];
const BRONZE: V3 = [0.1, 0.08, 0.055];
/// Temple bells: bronze with a golden sheen.
const BELL: V3 = [0.3, 0.2, 0.07];
const GOLD: V3 = [0.5, 0.36, 0.12];
/// How deep the stone footings go (as for the farm buildings): on a slope
/// a building stands at the top of its ground.
const FOUNDATION: f32 = 3.5;

impl Mesh {
    /// Another model added to this one, scaled by `s` and moved to `at`
    /// (models without smoke: smoke's vertices are not positions).
    fn place(&mut self, other: &Mesh, at: V3, s: f32) {
        for v in other.v.chunks(VERTEX_STRIDE) {
            self.v.extend_from_slice(&[
                v[0] * s + at[0],
                v[1] * s + at[1],
                v[2] * s + at[2],
                v[3],
                v[4],
                v[5],
                v[6],
                v[7],
                v[8],
                v[9],
            ]);
        }
    }

    /// A paper lantern (chochin) hanging at `c`, `r` across and `h` tall,
    /// glowing with the colour of its paper, black caps top and bottom and
    /// its cord up to `hang`.
    fn chochin(&mut self, c: V3, r: f32, h: f32, paper: V3, hang: f32) {
        let axes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        self.mat = MAT_LAMP;
        self.ellipsoid_in(c, axes, [r, h / 2.0, r], 6, 10, paper);
        self.mat = MAT_MATT;
        let black = [0.02, 0.018, 0.016];
        for s in [-1.0f32, 1.0] {
            let y = c[1] + s * h * 0.46;
            self.prism(
                [c[0], c[2]],
                y - 0.03,
                y + 0.03,
                r * 0.55,
                r * 0.55,
                10,
                0.0,
                black,
            );
        }
        self.tube(
            add(c, [0.0, h * 0.49, 0.0]),
            [c[0], hang, c[2]],
            0.012,
            0.012,
            4,
            black,
        );
    }

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

    /// A beam along x from `-half` to `half`, `h` deep and `w` thick (across
    /// z), its middle following `curve` (height at each x), so beams on the
    /// same curve stack exactly; cut square at its ends.
    #[allow(clippy::too_many_arguments)]
    fn swept_beam(
        &mut self,
        half: f32,
        curve: &dyn Fn(f32) -> f32,
        h: f32,
        w: f32,
        z: f32,
        col: V3,
    ) {
        let n = 16;
        let at = |i: usize| {
            let x = -half + 2.0 * half * i as f32 / n as f32;
            (x, curve(x))
        };
        let (z0, z1) = (z - w / 2.0, z + w / 2.0);
        for i in 0..n {
            let ((xa, ya), (xb, yb)) = (at(i), at(i + 1));
            let (ta, tb, ba, bb) = (ya + h / 2.0, yb + h / 2.0, ya - h / 2.0, yb - h / 2.0);
            self.quad([xa, ta, z1], [xb, tb, z1], [xb, tb, z0], [xa, ta, z0], col);
            self.quad(
                [xa, ba, z0],
                [xb, bb, z0],
                [xb, bb, z1],
                [xa, ba, z1],
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

    /// A square temple roof of copper tiles gone turquoise, centred on the
    /// y axis: from its eaves, `w` out from the middle at height `eave_y`,
    /// up to `top_w` out at `top_y`, its pitch flattening towards the eaves
    /// and its corners swept up by `lift` and out by `flare`, the way
    /// temple roofs curve. Rows of tiles run down it; ridges run down its
    /// corners, each with a bronze bell hanging from its end; the eaves are
    /// `thick` deep, edged in vermilion, dark beneath.
    #[allow(clippy::too_many_arguments)]
    fn temple_roof(
        &mut self,
        eave_y: f32,
        w: f32,
        top_y: f32,
        top_w: f32,
        lift: f32,
        flare: f32,
        thick: f32,
    ) {
        let (ns, nv) = (16usize, 6usize);
        let sides: [([f32; 2], [f32; 2]); 4] = [
            ([1.0, 0.0], [0.0, 1.0]),
            ([0.0, 1.0], [-1.0, 0.0]),
            ([-1.0, 0.0], [0.0, -1.0]),
            ([0.0, -1.0], [1.0, 0.0]),
        ];
        let surface = |d: [f32; 2], t: [f32; 2], s: f32, v: f32| -> V3 {
            // The eaves flare out towards the corners, and only their last
            // stretch curls up; the hips still fall to the corners.
            let a = s.abs().powi(3);
            let curl = s.abs().powi(6) * (1.0 - v).powi(2);
            let h = w + flare * a * (1.0 - v) + (top_w - w) * v;
            let ye = eave_y + lift * curl;
            let y = ye + (top_y - ye) * v.powf(1.6);
            [d[0] * h + t[0] * s * h, y, d[1] * h + t[1] * s * h]
        };
        let down = |p: V3| [p[0], p[1] - thick, p[2]];
        for (d, t) in sides {
            let sv = |i: usize, j: usize| (-1.0 + 2.0 * i as f32 / ns as f32, j as f32 / nv as f32);
            let pt = |i: usize, j: usize| {
                let (s, v) = sv(i, j);
                surface(d, t, s, v)
            };
            let normal = |i: usize, j: usize| {
                let (s, v) = sv(i, j);
                let e = 1e-3;
                let ds = sub(
                    surface(d, t, (s + e).min(1.0), v),
                    surface(d, t, (s - e).max(-1.0), v),
                );
                let dv = sub(
                    surface(d, t, s, (v + e).min(1.0)),
                    surface(d, t, s, (v - e).max(0.0)),
                );
                let n = norm(cross(dv, ds));
                if n[1] < 0.0 {
                    scale(n, -1.0)
                } else {
                    n
                }
            };
            let along = [t[0], 0.0, t[1]];
            self.mat = MAT_GLOSS;
            for i in 0..ns {
                // Rows of round tiles: every other row a shade darker and
                // each turned a little to the light, so the rows show.
                let (tile, tilt) = if i % 2 == 0 {
                    (TILE, 0.2)
                } else {
                    (scale(TILE, 0.86), -0.2)
                };
                for j in 0..nv {
                    let k = [(i, j), (i, j + 1), (i + 1, j + 1), (i + 1, j)];
                    for idx in [0, 1, 2, 0, 2, 3] {
                        let (a, b) = k[idx];
                        let n = norm(add(normal(a, b), scale(along, tilt)));
                        // Weathered paler towards the top.
                        let c = scale(tile, 0.92 + 0.16 * b as f32 / nv as f32);
                        self.push(pt(a, b), n, c);
                    }
                }
            }
            self.mat = MAT_MATT;
            for i in 0..ns {
                for j in (0..nv).step_by(3) {
                    let j1 = (j + 3).min(nv);
                    let (a, b, c, dd) = (
                        down(pt(i, j)),
                        down(pt(i, j1)),
                        down(pt(i + 1, j1)),
                        down(pt(i + 1, j)),
                    );
                    self.quad(a, dd, c, b, UNDER_EAVES);
                }
                // The vermilion edge of the eaves.
                let (a, b) = (pt(i, 0), pt(i + 1, 0));
                self.quad(down(b), down(a), a, b, VERMILION);
            }
            // The ridge down the corner, and the bell hanging from its end.
            let ridge: Vec<(V3, f32)> = (0..=nv)
                .map(|j| {
                    let v = j as f32 / nv as f32;
                    (
                        add(surface(d, t, 1.0, v), [0.0, 0.07, 0.0]),
                        0.12 - 0.05 * v,
                    )
                })
                .collect();
            self.mat = MAT_GLOSS;
            self.sweep(&ridge, 6, |_, _| RIDGE);
            let c = down(surface(d, t, 1.0, 0.0));
            let c = [c[0] * 0.97, c[1], c[2] * 0.97];
            self.tube(c, add(c, [0.0, -0.32, 0.0]), 0.015, 0.015, 4, BRONZE);
            self.tube(
                add(c, [0.0, -0.32, 0.0]),
                add(c, [0.0, -0.62, 0.0]),
                0.05,
                0.13,
                10,
                BELL,
            );
            // A gilded cap on the corner's tip.
            let tip = add(surface(d, t, 1.0, 0.0), [0.0, 0.05, 0.0]);
            let out = [d[0] * 0.12 + t[0] * 0.12, 0.28, d[1] * 0.12 + t[1] * 0.12];
            self.tube(tip, add(tip, out), 0.08, 0.025, 8, GOLD);
            self.mat = MAT_MATT;
        }
        let tw = top_w;
        self.quad(
            [-tw, top_y, tw],
            [tw, top_y, tw],
            [tw, top_y, -tw],
            [-tw, top_y, -tw],
            RIDGE,
        );
    }

    /// One storey of a pagoda, `b` out from its middle and `h` tall from
    /// `y`: white walls framed in vermilion posts and beams, a vermilion
    /// door in the middle of each side and, above the ground floor, a
    /// balcony with a railing all round.
    fn storey(&mut self, y: f32, b: f32, h: f32, balcony: bool) {
        self.cuboid(
            [-b + 0.05, y, -b + 0.05],
            [b - 0.05, y + h, b - 0.05],
            PLASTER,
        );
        self.mat = MAT_GLOSS;
        // Beams round it: at the foot, over the doors and at the head.
        for (y0, y1) in [
            (y, y + 0.2),
            (y + h * 0.66, y + h * 0.66 + 0.14),
            (y + h - 0.18, y + h),
        ] {
            self.cuboid(
                [-b - 0.04, y0, -b - 0.04],
                [b + 0.04, y1, b + 0.04],
                VERMILION,
            );
        }
        // Posts at the corners and either side of the middle bay.
        let post = 0.1;
        for u in [-b, -b / 3.0, b / 3.0, b] {
            for (x, z) in [(u, -b), (u, b), (-b, u), (b, u)] {
                self.cuboid(
                    [x - post, y, z - post],
                    [x + post, y + h, z + post],
                    VERMILION,
                );
            }
        }
        // The doors: two leaves in the middle bay, a darker panel in each.
        let (dy0, dy1) = (y + 0.2, y + h * 0.66);
        for (d, t) in [
            ([1.0f32, 0.0f32], [0.0f32, 1.0f32]),
            ([0.0, 1.0], [-1.0, 0.0]),
            ([-1.0, 0.0], [0.0, -1.0]),
            ([0.0, -1.0], [1.0, 0.0]),
        ] {
            let at = |u: f32, yy: f32, out: f32| {
                [d[0] * (b + out) + t[0] * u, yy, d[1] * (b + out) + t[1] * u]
            };
            let half = b / 3.0 - post;
            self.quad(
                at(-half, dy0, 0.02),
                at(half, dy0, 0.02),
                at(half, dy1, 0.02),
                at(-half, dy1, 0.02),
                VERMILION,
            );
            for (u0, u1) in [(-half + 0.12, -0.06), (0.06, half - 0.12)] {
                self.quad(
                    at(u0, dy0 + 0.15, 0.03),
                    at(u1, dy0 + 0.15, 0.03),
                    at(u1, dy1 - 0.15, 0.03),
                    at(u0, dy1 - 0.15, 0.03),
                    scale(VERMILION, 0.55),
                );
            }
            self.quad(
                at(-0.03, dy0, 0.035),
                at(0.03, dy0, 0.035),
                at(0.03, dy1, 0.035),
                at(-0.03, dy1, 0.035),
                [0.04, 0.02, 0.015],
            );
            // Rows of gilded studs across each leaf.
            self.mat = MAT_GLOSS;
            for yy in [dy0 + 0.3, (dy0 + dy1) / 2.0, dy1 - 0.3] {
                for u in [-half * 0.55, half * 0.55] {
                    let (a, b) = (at(u - 0.05, yy - 0.03, 0.04), at(u + 0.05, yy - 0.03, 0.04));
                    let (c, e) = (at(u + 0.05, yy + 0.03, 0.04), at(u - 0.05, yy + 0.03, 0.04));
                    self.quad(a, b, c, e, GOLD);
                }
            }
            // Lattice windows in the bays either side: dark behind slim
            // vermilion bars.
            let (wy0, wy1) = (dy0 + 0.35, dy1 - 0.15);
            for side in [-1.0f32, 1.0] {
                let (u0, u1) = if side < 0.0 {
                    (-b + post + 0.15, -b / 3.0 - post - 0.15)
                } else {
                    (b / 3.0 + post + 0.15, b - post - 0.15)
                };
                if u1 - u0 < 0.3 || wy1 - wy0 < 0.3 {
                    continue;
                }
                let dark = [0.035, 0.02, 0.015];
                self.quad(
                    at(u0, wy0, 0.02),
                    at(u1, wy0, 0.02),
                    at(u1, wy1, 0.02),
                    at(u0, wy1, 0.02),
                    dark,
                );
                let bars = ((u1 - u0) / 0.14) as usize;
                for k in 1..bars {
                    let u = u0 + (u1 - u0) * k as f32 / bars as f32;
                    let (a, b) = (at(u - 0.022, wy0, 0.03), at(u + 0.022, wy0, 0.03));
                    let (c, e) = (at(u + 0.022, wy1, 0.03), at(u - 0.022, wy1, 0.03));
                    self.quad(a, b, c, e, VERMILION);
                }
            }
            self.mat = MAT_MATT;
        }
        if balcony {
            let o = b + 0.6;
            self.cuboid([-o, y - 0.14, -o], [o, y, o], VERMILION);
            for (y0, y1) in [(y + 0.36, y + 0.42), (y + 0.74, y + 0.82)] {
                for (a, c) in [
                    ([-o, -o], [o, -o + 0.07]),
                    ([-o, o - 0.07], [o, o]),
                    ([-o, -o], [-o + 0.07, o]),
                    ([o - 0.07, -o], [o, o]),
                ] {
                    self.cuboid([a[0], y0, a[1]], [c[0], y1, c[1]], VERMILION);
                }
            }
            let n = ((2.0 * o) / 0.75).round() as usize;
            for k in 0..=n {
                let u = -o + 2.0 * o * k as f32 / n as f32;
                for (x, z) in [(u, -o), (u, o), (-o, u), (o, u)] {
                    self.cuboid(
                        [x - 0.035, y, z - 0.035],
                        [x + 0.035, y + 0.82, z + 0.035],
                        VERMILION,
                    );
                }
            }
        }
        self.mat = MAT_MATT;
    }

    /// The brackets that carry the eaves, over a storey `b` out from its
    /// middle at `y`: two vermilion tiers stepping out, the ends of their
    /// blocks painted white.
    fn brackets(&mut self, y: f32, b: f32) {
        self.mat = MAT_GLOSS;
        self.cuboid(
            [-b - 0.18, y, -b - 0.18],
            [b + 0.18, y + 0.26, b + 0.18],
            VERMILION,
        );
        self.cuboid(
            [-b - 0.45, y + 0.26, -b - 0.45],
            [b + 0.45, y + 0.52, b + 0.45],
            VERMILION,
        );
        self.mat = MAT_MATT;
        for (o, y0) in [(b + 0.18, y + 0.07), (b + 0.45, y + 0.33)] {
            let n = ((2.0 * o) / 0.42).round() as usize;
            for (d, t) in [
                ([1.0f32, 0.0f32], [0.0f32, 1.0f32]),
                ([0.0, 1.0], [-1.0, 0.0]),
                ([-1.0, 0.0], [0.0, -1.0]),
                ([0.0, -1.0], [1.0, 0.0]),
            ] {
                let at = |u: f32, yy: f32| {
                    [
                        d[0] * (o + 0.01) + t[0] * u,
                        yy,
                        d[1] * (o + 0.01) + t[1] * u,
                    ]
                };
                for k in 0..n {
                    let u = -o + (k as f32 + 0.5) * 2.0 * o / n as f32;
                    self.quad(
                        at(u - 0.07, y0),
                        at(u + 0.07, y0),
                        at(u + 0.07, y0 + 0.12),
                        at(u - 0.07, y0 + 0.12),
                        PLASTER,
                    );
                }
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
        let patch = (p[0] * 0.7 + p[2] * 0.7) as i32 as u32;
        let grain = 0.9 + 0.2 * rnd(23, band as u32, patch);
        let thatch = scale(lerp(aged, straw, 0.25 + 0.75 * t), k * grain);
        // Moss gathering on the old thatch low down, where it stays damp.
        let moss = (1.0 - t).powi(2)
            * if rnd(29, band as u32, patch) > 0.55 {
                0.55
            } else {
                0.0
            };
        lerp(thatch, tinted([0.08, 0.1, 0.035]), moss)
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
    // Smoke from the hearth, rising out of the gable's vent.
    m.smoke([gx + 0.35, (gy + ridge) / 2.0, 0.0]);
    // An indigo curtain (noren) over the open room, split in four, the
    // family's crest in white across the middle.
    m.mat = MAT_FABRIC;
    let indigo = [0.025, 0.045, 0.11];
    for k in 0..4 {
        let x0 = -1.38 + k as f32 * 0.695;
        let zc = front + 0.03;
        m.quad(
            [x0, 1.85, zc],
            [x0 + 0.66, 1.85, zc],
            [x0 + 0.66, sy1, zc],
            [x0, sy1, zc],
            indigo,
        );
    }
    m.disc(
        [0.0, 2.25, front + 0.035],
        0.2,
        [0.0, 0.0, 1.0],
        14,
        [0.62, 0.6, 0.56],
    );
    m.disc([0.0, 2.25, front + 0.04], 0.13, [0.0, 0.0, 1.0], 14, indigo);
    m.mat = MAT_MATT;
    // Red paper lanterns either side of it, at the veranda's edge.
    for x in [-1.75f32, 1.75] {
        m.chochin([x, 2.3, front + 0.92], 0.17, 0.3, [0.42, 0.07, 0.035], 3.1);
    }
    // Persimmons drying on strings under the eaves.
    let axes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for (k, x) in [-5.1f32, -4.75, -4.4, -4.05].into_iter().enumerate() {
        let z = front + 0.55;
        let top = 3.15;
        m.tube(
            [x, top, z],
            [x, top - 0.98, z],
            0.006,
            0.006,
            3,
            [0.3, 0.25, 0.18],
        );
        m.mat = MAT_GLOSS;
        for j in 0..7 {
            let y = top - 0.12 - j as f32 * 0.13;
            let tone = 0.85 + 0.3 * rnd(131, k as u32, j);
            m.ellipsoid_in(
                [x, y, z],
                axes,
                [0.05, 0.06, 0.05],
                3,
                6,
                scale([0.55, 0.17, 0.025], tone),
            );
        }
        m.mat = MAT_MATT;
    }
    // Two cushions and a tray of tea set out on the veranda.
    m.mat = MAT_FABRIC;
    for (x, col) in [(2.5f32, [0.32, 0.07, 0.05]), (3.4, [0.05, 0.08, 0.16])] {
        m.cuboid(
            [x - 0.28, floor, front + 0.28],
            [x + 0.28, floor + 0.07, front + 0.84],
            col,
        );
    }
    m.mat = MAT_MATT;
    m.cuboid(
        [2.8, floor, front + 0.4],
        [3.1, floor + 0.025, front + 0.7],
        [0.14, 0.08, 0.04],
    );
    m.mat = MAT_GLOSS;
    m.ellipsoid_in(
        [2.95, floor + 0.09, front + 0.55],
        axes,
        [0.08, 0.065, 0.08],
        4,
        8,
        [0.03, 0.03, 0.03],
    );
    for z in [front + 0.45, front + 0.65] {
        m.prism(
            [3.05, z],
            floor + 0.025,
            floor + 0.07,
            0.025,
            0.03,
            8,
            0.0,
            [0.55, 0.52, 0.45],
        );
    }
    m.mat = MAT_MATT;
    // In the garden: a stone water basin with a bamboo spout, a small
    // lantern beside it and azaleas in flower.
    let (bx, bz) = (-3.3f32, front + 2.7);
    m.prism([bx, bz], -0.2, 0.38, 0.36, 0.33, 9, 0.0, STONE);
    m.mat = MAT_GLOSS;
    m.disc(
        [bx, 0.36, bz],
        0.26,
        [0.0, 1.0, 0.0],
        12,
        [0.02, 0.04, 0.05],
    );
    m.mat = MAT_MATT;
    let bamboo = [0.3, 0.3, 0.11];
    m.tube(
        [bx - 0.75, -0.1, bz],
        [bx - 0.75, 0.75, bz],
        0.05,
        0.05,
        6,
        bamboo,
    );
    m.tube(
        [bx - 0.75, 0.7, bz],
        [bx - 0.15, 0.6, bz],
        0.035,
        0.035,
        6,
        bamboo,
    );
    for k in 0..4u32 {
        let a = k as f32 * 1.7;
        m.boulder(
            [bx + a.cos() * 0.55, -0.05, bz + a.sin() * 0.55],
            [0.18, 0.07, 0.15],
            2,
            6,
            STONE,
            340 + k,
        );
    }
    m.place(&toro(), [bx - 1.1, 0.0, bz + 0.6], 0.5);
    m.mat = MAT_LEAF;
    for (k, &(x, z, r)) in [(2.6f32, 2.2f32, 0.55f32), (3.6, 2.5, 0.45), (4.7, 2.1, 0.6)]
        .iter()
        .enumerate()
    {
        m.ellipsoid_in(
            [x, r * 0.55, front + z],
            axes,
            [r, r * 0.6, r * 0.9],
            4,
            8,
            tinted([0.05, 0.1, 0.035]),
        );
        let pink = hued([0.55, 0.12, 0.3], 0.1 * k as f32, 1.0);
        m.ellipsoid_in(
            [x, r * 0.68, front + z],
            axes,
            [r * 0.97, r * 0.52, r * 0.87],
            4,
            8,
            tinted(pink),
        );
    }
    m.mat = MAT_MATT;
    m
}

/// A shrine gate (torii) in the old, grand style: vermilion pillars on
/// black feet, leaning in a little and banded in black at their heads; a
/// tie beam through them, held by wedges; a name board in a black frame
/// edged in gold; and the great top beams, vermilion under black, swept up
/// at their ends and roofed in turquoise copper tiles with gilded end caps.
/// The pillars rise into the beams, which sit on them and on each other.
/// About 5.7 m tall and 6 m across the top.
pub fn torii() -> Mesh {
    let mut m = Mesh::default();
    let black = [0.025, 0.022, 0.02];
    let (hk, yk, lift) = (3.0f32, 4.95f32, 0.36f32);
    let (kh, sh) = (0.3f32, 0.28f32);
    let kasagi = move |x: f32| yk + lift * (x.abs() / hk).powf(2.5);
    let shimaki = move |x: f32| kasagi(x) - (kh + sh) / 2.0;
    let (base, top) = (1.8f32, 1.66f32);
    // Where each pillar meets the lower beam: a little into it.
    let pillar_top = shimaki(top) - sh / 2.0 + 0.06;
    let pillar_x = |y: f32| base + (top - base) * (y - 0.6) / (pillar_top - 0.6);
    for s in [-1.0f32, 1.0] {
        m.prism([s * base, 0.0], -FOUNDATION, 0.1, 0.38, 0.38, 8, 0.2, STONE);
        m.mat = MAT_GLOSS;
        m.tube(
            [s * base, 0.1, 0.0],
            [s * pillar_x(0.62), 0.62, 0.0],
            0.25,
            0.24,
            14,
            black,
        );
        m.tube(
            [s * base, 0.6, 0.0],
            [s * top, pillar_top, 0.0],
            0.2,
            0.175,
            14,
            VERMILION,
        );
        // A black band at the pillar's head, just under the beam.
        let yb = pillar_top - 0.3;
        m.tube(
            [s * pillar_x(yb), yb, 0.0],
            [s * pillar_x(yb + 0.14), yb + 0.14, 0.0],
            0.19,
            0.19,
            14,
            black,
        );
        m.mat = MAT_MATT;
    }
    m.mat = MAT_GLOSS;
    // The tie beam through the pillars, and a wedge outside each.
    let (ny0, ny1) = (3.62f32, 3.94f32);
    m.cuboid([-2.45, ny0, -0.11], [2.45, ny1, 0.11], VERMILION);
    for s in [-1.0f32, 1.0] {
        let x = pillar_x((ny0 + ny1) / 2.0) + 0.2;
        let (x0, x1) = if s > 0.0 {
            (x, x + 0.12)
        } else {
            (-x - 0.12, -x)
        };
        m.cuboid([x0, ny0 - 0.06, -0.13], [x1, ny1 + 0.06, 0.13], black);
    }
    // The strut between the beams, and the name board on it.
    let under = shimaki(0.0) - sh / 2.0;
    m.cuboid([-0.12, ny1, -0.1], [0.12, under, 0.1], VERMILION);
    m.mat = MAT_MATT;
    let (py0, py1) = (ny1 + 0.04, under - 0.02);
    m.cuboid([-0.36, py0, 0.1], [0.36, py1, 0.18], black);
    m.mat = MAT_GLOSS;
    m.cuboid([-0.31, py0 + 0.05, 0.18], [0.31, py1 - 0.05, 0.195], GOLD);
    m.mat = MAT_MATT;
    m.cuboid(
        [-0.26, py0 + 0.1, 0.195],
        [0.26, py1 - 0.1, 0.205],
        [0.03, 0.035, 0.04],
    );
    m.mat = MAT_GLOSS;
    for k in 0..3 {
        let y = py0 + 0.14 + k as f32 * (py1 - py0 - 0.28) / 3.0;
        m.cuboid([-0.07, y, 0.205], [0.07, y + 0.07, 0.212], GOLD);
    }
    // The top beams, the lower vermilion and the upper black, on one curve.
    m.swept_beam(2.78, &shimaki, sh, 0.38, 0.0, VERMILION);
    m.swept_beam(hk, &kasagi, kh, 0.46, 0.0, black);
    // The roof of copper tiles along the top beam, rows running across it,
    // its ends closed and capped in gold.
    let n = 30;
    let (ez, rise) = (0.31f32, 0.13f32);
    let roof = |x: f32, z: f32| {
        let y = kasagi(x) + kh / 2.0;
        [x, y + rise * (1.0 - z.abs() / ez), z]
    };
    for i in 0..n {
        let x0 = -hk - 0.06 + (2.0 * hk + 0.12) * i as f32 / n as f32;
        let x1 = -hk - 0.06 + (2.0 * hk + 0.12) * (i + 1) as f32 / n as f32;
        let tile = if i % 2 == 0 { TILE } else { scale(TILE, 0.86) };
        m.quad(
            roof(x0, ez),
            roof(x1, ez),
            roof(x1, 0.0),
            roof(x0, 0.0),
            tile,
        );
        m.quad(
            roof(x1, -ez),
            roof(x0, -ez),
            roof(x0, 0.0),
            roof(x1, 0.0),
            tile,
        );
        // Its eaves' edge.
        let dn = |p: V3| [p[0], p[1] - 0.05, p[2]];
        m.quad(
            dn(roof(x0, ez)),
            dn(roof(x1, ez)),
            roof(x1, ez),
            roof(x0, ez),
            RIDGE,
        );
        m.quad(
            dn(roof(x1, -ez)),
            dn(roof(x0, -ez)),
            roof(x0, -ez),
            roof(x1, -ez),
            RIDGE,
        );
    }
    let ridge: Vec<(V3, f32)> = (0..=n)
        .map(|i| {
            let x = -hk - 0.06 + (2.0 * hk + 0.12) * i as f32 / n as f32;
            (add(roof(x, 0.0), [0.0, 0.02, 0.0]), 0.045)
        })
        .collect();
    m.sweep(&ridge, 6, |_, _| RIDGE);
    for s in [-1.0f32, 1.0] {
        let x = s * (hk + 0.06);
        let (a, b, c) = (roof(x, ez), roof(x, -ez), roof(x, 0.0));
        if s > 0.0 {
            m.tri(a, b, c, RIDGE);
        } else {
            m.tri(b, a, c, RIDGE);
        }
        let y = kasagi(x);
        let (x0, x1) = if s > 0.0 {
            (x - 0.02, x + 0.06)
        } else {
            (x - 0.06, x + 0.02)
        };
        m.cuboid(
            [x0, y - kh / 2.0 - 0.02, -0.25],
            [x1, y + kh / 2.0 + 0.05, 0.25],
            GOLD,
        );
    }
    m.mat = MAT_MATT;
    m
}

/// A five-storey pagoda on a stone platform, about 26 m to the tip of its
/// spire: white walls framed in vermilion, a balcony with a railing round
/// every storey above the first, rows of brackets carrying deep eaves, and
/// five roofs of turquoise copper tiles, each smaller than the one below,
/// their corners swept up and hung with bronze bells. A bronze spire of
/// nine rings rises from the top, finished with a gilded jewel.
pub fn pagoda() -> Mesh {
    let mut m = Mesh::default();
    // The stone platform, and steps up to it at the front.
    m.cuboid(
        [-4.7, -FOUNDATION, -4.7],
        [4.7, 0.9, 4.7],
        scale(STONE, 0.85),
    );
    m.cuboid([-4.85, 0.82, -4.85], [4.85, 0.95, 4.85], STONE);
    for k in 0..3 {
        let y = 0.3 * (k + 1) as f32;
        m.cuboid(
            [-1.5, -0.5, 4.7],
            [1.5, y, 4.7 + 0.42 * (3 - k) as f32],
            scale(STONE, 1.06),
        );
    }
    let storeys = 5;
    let mut y = 0.95;
    let mut top = 0.0;
    for k in 0..storeys {
        let b = 2.55 - 0.27 * k as f32;
        let h = if k == 0 { 2.5 } else { 1.75 };
        m.storey(y, b, h, k > 0);
        m.brackets(y + h, b);
        let eave = y + h + 0.62;
        let w = b + if k == 0 { 2.7 } else { 2.4 };
        let last = k + 1 == storeys;
        let top_w = if last {
            0.42
        } else {
            2.55 - 0.27 * (k + 1) as f32 + 0.15
        };
        top = eave + if last { 2.4 } else { 1.65 };
        m.temple_roof(eave, w, top, top_w, 0.5, 0.5, 0.45);
        y = top - 0.12;
    }
    // The spire: a base, a pole through nine rings, and the jewel.
    m.mat = MAT_GLOSS;
    m.prism([0.0, 0.0], top - 0.1, top + 0.45, 0.5, 0.42, 8, 0.0, BRONZE);
    m.tube(
        [0.0, top + 0.45, 0.0],
        [0.0, top + 5.6, 0.0],
        0.1,
        0.07,
        10,
        BRONZE,
    );
    for k in 0..9 {
        let yr = top + 1.0 + k as f32 * 0.42;
        m.torus(
            [0.0, yr, 0.0],
            [0.0, 1.0, 0.0],
            0.36 - 0.012 * k as f32,
            0.05,
            16,
            6,
            BRONZE,
        );
    }
    m.disc([0.0, top + 4.95, 0.0], 0.3, [0.0, 1.0, 0.0], 12, GOLD);
    m.ellipsoid_in(
        [0.0, top + 5.25, 0.0],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        [0.2, 0.28, 0.2],
        6,
        10,
        GOLD,
    );
    m.mat = MAT_MATT;
    // Moss at the platform's foot.
    m.cuboid(
        [-4.75, -0.05, -4.75],
        [4.75, 0.14, 4.75],
        [0.06, 0.085, 0.03],
    );
    // A vermilion fence round the platform's edge, open at the steps.
    let o = 4.45;
    m.mat = MAT_GLOSS;
    let n = 10;
    for k in 0..=n {
        let u = -o + 2.0 * o * k as f32 / n as f32;
        for (x, z) in [(u, -o), (u, o), (-o, u), (o, u)] {
            if z > o - 0.01 && x.abs() < 1.6 {
                continue;
            }
            m.cuboid(
                [x - 0.06, 0.95, z - 0.06],
                [x + 0.06, 1.85, z + 0.06],
                VERMILION,
            );
        }
    }
    for (y0, y1) in [(1.35, 1.43), (1.72, 1.82)] {
        m.cuboid([-o, y0, -o - 0.05], [o, y1, -o + 0.05], VERMILION);
        m.cuboid([-o - 0.05, y0, -o], [-o + 0.05, y1, o], VERMILION);
        m.cuboid([o - 0.05, y0, -o], [o + 0.05, y1, o], VERMILION);
        for (x0, x1) in [(-o, -1.6), (1.6, o)] {
            m.cuboid([x0, y0, o - 0.05], [x1, y1, o + 0.05], VERMILION);
        }
    }
    m.mat = MAT_MATT;
    // Stone lanterns either side of the steps.
    let lantern = toro();
    for x in [-2.3f32, 2.3] {
        m.place(&lantern, [x, 0.95, 3.8], 0.85);
    }
    // A great red lantern hanging before the door, and a gilded name
    // board above it.
    m.chochin([0.0, 3.0, 3.25], 0.42, 0.75, [0.45, 0.06, 0.03], 3.55);
    m.mat = MAT_GLOSS;
    m.cuboid([-0.6, 3.05, 2.62], [0.6, 3.4, 2.7], GOLD);
    m.mat = MAT_MATT;
    m.cuboid([-0.52, 3.09, 2.7], [0.52, 3.36, 2.72], [0.03, 0.03, 0.035]);
    // Flame-shaped ornaments of gilt on the spire, under the jewel.
    m.mat = MAT_GLOSS;
    for k in 0..4 {
        let a = k as f32 * std::f32::consts::FRAC_PI_2 + std::f32::consts::FRAC_PI_4;
        let (c, sn) = (a.cos(), a.sin());
        let p = |r: f32, y: f32| [c * r, top + y, sn * r];
        m.quad(
            p(0.06, 4.3),
            p(0.34, 4.45),
            p(0.3, 4.85),
            p(0.06, 4.95),
            GOLD,
        );
        m.quad(
            p(0.06, 4.95),
            p(0.3, 4.85),
            p(0.34, 4.45),
            p(0.06, 4.3),
            GOLD,
        );
    }
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
