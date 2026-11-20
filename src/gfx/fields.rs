//! What farmers leave in the fields, in metres: dry-stone walls along them
//! and round bales of hay. Both take their instance's tint (a wall's stone,
//! a bale's straw or wrapping).

use super::math::*;
use super::mesh::{rnd, tinted, Mesh};

/// How long one piece of wall is: pieces stand end to end along a field.
pub const WALL_PIECE: f32 = 3.3;
/// How far a wall reaches into the ground, so it never stands on air where
/// the field slopes.
const WALL_FOOT: f32 = 0.55;

const STONE: V3 = [0.2, 0.192, 0.175];
const LICHEN: V3 = [0.3, 0.3, 0.17];
const MOSS: V3 = [0.07, 0.1, 0.035];
const GAPS: V3 = [0.05, 0.05, 0.045];

impl Mesh {
    /// A stone: a box from `lo` to `hi` whose corners are each pushed a
    /// little out of true (by up to `rough`), lit as a rounded lump (its
    /// normals lean towards its corners). `back` leaves out the -z face
    /// (it lies against the wall's core), `front` the +z face.
    fn stone(&mut self, lo: V3, hi: V3, rough: f32, seed: u32, col: V3, skip: Skip) {
        let c = scale(add(lo, hi), 0.5);
        let corner = |i: u32| -> V3 {
            let p = [
                if i & 1 == 0 { lo[0] } else { hi[0] },
                if i & 2 == 0 { lo[1] } else { hi[1] },
                if i & 4 == 0 { lo[2] } else { hi[2] },
            ];
            let j = |k: u32| (rnd(seed, i, k) - 0.5) * 2.0 * rough;
            [p[0] + j(1), p[1] + j(2), p[2] + j(3) * 0.5]
        };
        let p: Vec<V3> = (0..8).map(corner).collect();
        // Faces as corner numbers, counter-clockwise from outside.
        let faces: [([usize; 4], V3, bool); 6] = [
            ([4, 5, 7, 6], [0.0, 0.0, 1.0], skip != Skip::Front),
            ([1, 0, 2, 3], [0.0, 0.0, -1.0], skip != Skip::Back),
            ([5, 1, 3, 7], [1.0, 0.0, 0.0], true),
            ([0, 4, 6, 2], [-1.0, 0.0, 0.0], true),
            ([6, 7, 3, 2], [0.0, 1.0, 0.0], true),
            ([0, 1, 5, 4], [0.0, -1.0, 0.0], skip == Skip::None),
        ];
        for (f, n, keep) in faces {
            if !keep {
                continue;
            }
            // Darker underneath and in the joints, lighter on top.
            let shade = 0.82 + 0.18 * n[1] + 0.08 * (rnd(seed, f[0] as u32, 9) - 0.5);
            let v = |i: usize| {
                let rounded = norm(add(scale(n, 0.62), scale(norm(sub(p[i], c)), 0.38)));
                (p[i], rounded, scale(col, shade))
            };
            for i in [0, 1, 2, 0, 2, 3] {
                let (pp, nn, cc) = v(f[i]);
                self.push(pp, nn, cc);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Skip {
    None,
    /// A face stone on the wall's +z side: its back is hidden in the wall.
    Back,
    /// On the -z side.
    Front,
}

/// One piece of dry-stone wall, `WALL_PIECE` long along x and centred on 0:
/// courses of rough stones on both faces, bigger at the foot, leaning in as
/// they rise, mossy low down and with lichen here and there, and a row of
/// coping stones set on edge along the top. Dark gaps between the stones
/// are the wall's core showing through.
pub fn stone_wall() -> Mesh {
    let mut m = Mesh::default();
    let half = WALL_PIECE / 2.0;
    let top = 0.9;
    // Half the wall's thickness at height y: wide at the foot, leaning in.
    let thick = |y: f32| 0.36 - 0.11 * (y / top).clamp(0.0, 1.0);
    // The core: dark, so the joints between stones read as shadow.
    let core = tinted(GAPS);
    let footing = tinted(add(scale(STONE, 0.55), scale(MOSS, 0.4)));
    let z0 = |y: f32| thick(y) - 0.06;
    let (b, t) = (-WALL_FOOT, top);
    for s in [1.0f32, -1.0] {
        let (zb, zt) = (s * (thick(0.0) - 0.06), s * (thick(t) - 0.06));
        let (x0, x1) = if s > 0.0 {
            (-half, half)
        } else {
            (half, -half)
        };
        // Below the stones, earthy footing stone; above, the dark joints.
        let foot = -0.3;
        let zf = s * (thick(0.0) - 0.02);
        m.quad(
            [x0, b, zf],
            [x1, b, zf],
            [x1, foot, zf],
            [x0, foot, zf],
            footing,
        );
        m.quad(
            [x0, foot, zb],
            [x1, foot, zb],
            [x1, t, zt],
            [x0, t, zt],
            core,
        );
    }
    // Each end in stones laid across the wall, course by course: where
    // pieces step down a slope, the higher one's end shows stonework, not
    // the core. (Below the ground it is the core's own end.)
    for (e, x) in [(0u32, -half), (1, half)] {
        let s = x.signum();
        let (a, c) = ([x, b, s * z0(0.0)], [x, b, -s * z0(0.0)]);
        let (d, f) = ([x, -0.3, s * z0(0.0)], [x, -0.3, -s * z0(0.0)]);
        m.quad(c, a, d, f, footing);
        let mut y = -0.3;
        let mut k = 0u32;
        while y < top - 0.05 {
            let q = |n: u32| rnd(43 + e, k, n);
            let h = ((0.2 + 0.08 * q(1)) * (1.0 - 0.2 * y / top)).min(top - y);
            let w = thick(y + h * 0.5) + 0.02;
            // One stone through, or two meeting in the middle.
            let split = q(2) < 0.5;
            let tone = scale(STONE, 0.72 + 0.36 * q(3));
            let depth = 0.22 + 0.12 * q(4);
            let (x0, x1) = if s > 0.0 {
                (x - depth, x + 0.015)
            } else {
                (x - 0.015, x + depth)
            };
            let lo_y = y + 0.008;
            let hi_y = y + h - 0.008;
            if split {
                let mid = 0.08 * (q(5) - 0.5);
                m.stone(
                    [x0, lo_y, -w],
                    [x1, hi_y, mid - 0.006],
                    0.025,
                    500 + e * 97 + k,
                    tinted(tone),
                    Skip::None,
                );
                let tone2 = scale(STONE, 0.72 + 0.36 * q(6));
                m.stone(
                    [x0, lo_y, mid + 0.006],
                    [x1, hi_y, w],
                    0.025,
                    700 + e * 97 + k,
                    tinted(tone2),
                    Skip::None,
                );
            } else {
                m.stone(
                    [x0, lo_y, -w],
                    [x1, hi_y, w],
                    0.025,
                    900 + e * 97 + k,
                    tinted(tone),
                    Skip::None,
                );
            }
            y += h;
            k += 1;
        }
    }
    // The face stones, course by course from just below the ground.
    let mut seed = 1u32;
    for (face, skip) in [(1.0f32, Skip::Back), (-1.0, Skip::Front)] {
        let mut y = -0.3;
        let mut course = 0u32;
        while y < top - 0.08 {
            let q = |n: u32| rnd(17 + face as u32, course, n);
            let h = (0.24 - 0.05 * (y / top)) * (0.85 + 0.3 * q(1));
            let h = h.min(top - y);
            let mut x = -half + 0.02 - 0.25 * q(2);
            while x < half - 0.03 {
                seed += 1;
                let r = |n: u32| rnd(seed, 3, n);
                let long = (0.5 - 0.18 * (y / top)) * (0.55 + 0.7 * r(1));
                let x1 = (x + long).min(half - 0.01);
                let x0 = x.max(-half + 0.01);
                if x1 - x0 > 0.08 {
                    let ym = y + h * 0.5;
                    let out = thick(ym) + 0.02 + 0.03 * r(2);
                    let (lo, hi) = if face > 0.0 {
                        (
                            [x0 + 0.012, y + 0.012, out - 0.16],
                            [x1 - 0.012, y + h - 0.012, out],
                        )
                    } else {
                        (
                            [x0 + 0.012, y + 0.012, -out],
                            [x1 - 0.012, y + h - 0.012, -out + 0.16],
                        )
                    };
                    // Grey stone of its own shade; mossy near the ground,
                    // now and then a patch of pale lichen.
                    let k = 0.78 + 0.4 * r(3);
                    let mut col = scale(STONE, k);
                    if y < 0.2 {
                        let w = 0.5 * (1.0 - (y + 0.14) / 0.34);
                        col = add(scale(col, 1.0 - w), scale(MOSS, w));
                    } else if r(4) < 0.18 {
                        col = add(scale(col, 0.55), scale(LICHEN, 0.45 * k));
                    }
                    m.stone(lo, hi, 0.03, seed, tinted(col), skip);
                }
                x = x1 + 0.006;
            }
            y += h;
            course += 1;
        }
    }
    // Coping: flat stones set on edge across the top, leaning a little
    // each their own way.
    let mut x = -half + 0.03;
    let mut k = 0u32;
    while x < half - 0.06 {
        let q = |n: u32| rnd(91, k, n);
        let w = 0.07 + 0.14 * q(1) * q(1);
        let h = 0.08 + 0.1 * q(2);
        // Some reach across the wall, some only most of the way.
        let (back, front) = (thick(top) - 0.06 * q(3), thick(top) - 0.06 * q(6));
        let lean = 0.09 * (q(4) - 0.5);
        let x1 = (x + w).min(half - 0.01);
        let col = scale(STONE, 0.62 + 0.38 * q(5));
        let mut piece = Mesh::default();
        piece.stone(
            [x, top - 0.05, -back],
            [x1, top + h, front],
            0.04,
            200 + k,
            tinted(col),
            Skip::None,
        );
        // Lean it: shear the top along the wall.
        for v in piece.v.chunks_mut(super::mesh::VERTEX_STRIDE) {
            v[0] += lean * ((v[1] - top) / h).max(0.0);
        }
        m.v.extend(piece.v);
        x = x1 + 0.006 + 0.02 * q(7);
        k += 1;
    }
    m
}

/// A round bale of hay or straw lying on its side, its axis along x: about
/// 1.5 m across and 1.2 m wide, sagging flat where it rests, with rounded
/// shoulders, the twine's lines around it and the rolled-up spiral showing
/// on its ends.
pub fn hay_bale() -> Mesh {
    let mut m = Mesh::default();
    let r = 0.76;
    let w = 0.6;
    let segs = 22;
    let centre = r - 0.05;
    // Where a point of the bale at angle `a` (0 = up) and radius `rr` lies,
    // flattened where it rests on the ground.
    let at = |x: f32, a: f32, rr: f32| -> V3 {
        let y = centre + a.cos() * rr;
        [x, y.max(0.02 + 0.04 * (y - 0.02).max(0.0)), a.sin() * rr]
    };
    // Around the side: rings across its width, rounded at the shoulders.
    let rings = [
        (-w, 0.9),
        (-w + 0.05, 0.975),
        (-w + 0.16, 1.0),
        (0.0, 1.01),
        (w - 0.16, 1.0),
        (w - 0.05, 0.975),
        (w, 0.9),
    ];
    let side = |i: usize, j: usize| -> (V3, V3, V3) {
        let (x, k) = rings[i];
        let a = j as f32 / segs as f32 * std::f32::consts::TAU;
        let p = at(x, a, r * k);
        let slope = if i == 0 {
            -0.7
        } else if i + 1 == rings.len() {
            0.7
        } else {
            (rings[i + 1].1 - rings[i - 1].1) / (rings[i + 1].0 - rings[i - 1].0) * -2.0
        };
        let n = norm([slope * 0.6, a.cos(), a.sin()]);
        // Straw in streaks around it; the twine as darker lines.
        let streak = 0.88 + 0.24 * rnd(5, j as u32 % segs as u32, i as u32 / 2);
        let twine = if i == 2 || i == 4 { 0.86 } else { 1.0 };
        (p, n, tinted(scale([1.0, 1.0, 1.0], streak * twine)))
    };
    for i in 0..rings.len() - 1 {
        for j in 0..segs {
            let q = [
                side(i, j),
                side(i + 1, j),
                side(i + 1, j + 1),
                side(i, j + 1),
            ];
            for k in [0, 2, 1, 0, 3, 2] {
                m.push(q[k].0, q[k].1, q[k].2);
            }
        }
    }
    // The ends: the roll's spiral, from the rim in to the middle.
    let radii = [0.9, 0.7, 0.5, 0.28, 0.0];
    for s in [-1.0f32, 1.0] {
        let x = s * w;
        let end = |i: usize, j: usize| -> (V3, V3, V3) {
            let a = j as f32 / segs as f32 * std::f32::consts::TAU;
            let rr = r * radii[i];
            // A little domed, and darker in the spiral's grooves.
            let p = at(x + s * 0.03 * (1.0 - radii[i] / 0.9), a, rr);
            let turn = (radii[i] * 14.0 + a).sin();
            let shade = 0.8 + 0.12 * turn + 0.06 * rnd(9, j as u32, i as u32);
            let n = norm([s, a.cos() * 0.2 * radii[i], a.sin() * 0.2 * radii[i]]);
            (p, n, tinted(scale([1.0, 1.0, 1.0], shade)))
        };
        for i in 0..radii.len() - 1 {
            for j in 0..segs {
                let q = [end(i, j), end(i + 1, j), end(i + 1, j + 1), end(i, j + 1)];
                let order = if s > 0.0 {
                    [0, 1, 2, 0, 2, 3]
                } else {
                    [0, 2, 1, 0, 3, 2]
                };
                for k in order {
                    m.push(q[k].0, q[k].1, q[k].2);
                }
            }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::super::mesh::VERTEX_STRIDE;
    use super::*;

    fn bounds(m: &Mesh) -> (V3, V3) {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for v in m.v.chunks(VERTEX_STRIDE) {
            for k in 0..3 {
                lo[k] = lo[k].min(v[k]);
                hi[k] = hi[k].max(v[k]);
            }
        }
        (lo, hi)
    }

    #[test]
    fn wall_pieces_join_end_to_end_and_stand_in_the_ground() {
        let m = stone_wall();
        assert!(m.v.iter().all(|v| v.is_finite()));
        let (lo, hi) = bounds(&m);
        // As long as a piece (give or take the rough stones), so pieces
        // neither gap nor overlap much.
        assert!((hi[0] - lo[0] - WALL_PIECE).abs() < 0.08, "{lo:?} {hi:?}");
        assert!(lo[1] <= -WALL_FOOT + 0.01);
        // About knee to waist high, to the top of its coping stones.
        assert!((1.0..1.25).contains(&hi[1]), "{}", hi[1]);
        assert!(hi[2] < 0.5 && lo[2] > -0.5);
        let tris = m.vertex_count() / 3;
        assert!(tris <= 1300, "{tris} triangles");
    }

    #[test]
    fn bales_rest_on_the_ground() {
        let m = hay_bale();
        assert!(m.v.iter().all(|v| v.is_finite()));
        let (lo, hi) = bounds(&m);
        assert!(lo[1] >= 0.0 && lo[1] < 0.05, "{}", lo[1]);
        assert!((1.4..1.6).contains(&hi[1]), "{}", hi[1]);
        assert!(m.vertex_count() / 3 <= 700);
    }
}
