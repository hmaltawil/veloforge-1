//! The aero road bike: a modern race bike built from the app's own shapes,
//! free to share like the rest of VeloForge.
//!
//! - Frame: deep, flat-backed aero tubes, seat stays dropped low on the
//!   seat tube, a tapered head tube and a fork whose blades sweep forward.
//! - Wheels: 50 mm carbon rims on bladed spokes, with grey rim graphics
//!   that turn with them.
//! - Cockpit: a one-piece bar and stem with a flat aero top and rubber
//!   hoods, a bike computer and front light out in front.
//! - Drivetrain: two chainrings, a 12-speed cassette, the chain over them
//!   and through the rear derailleur; disc brakes, and no cables showing.
//!
//! It keeps the classic bike's points of contact (hubs, bottom bracket,
//! crank length, saddle and hoods), so the rider's pose fits it as it is.
//! Everything is in the bike's frame: `x` forward, `y` up, `z` right, in
//! metres, the ground under the bottom bracket at the origin.

use super::math::*;
use super::mesh::{Frame, Joints, Mesh, RiderPose, MAT_GLOSS, MAT_LAMP, MAT_MATT};
use std::f32::consts::{PI, TAU};

/// The hubs' height and where they are along the bike (as the classic
/// bike's: the rider's pose is fitted to these).
const HUB_Y: f32 = 0.34;
const REAR_X: f32 = -0.5;
const FRONT_X: f32 = 0.57;
/// The bottom bracket, and the crank length.
const BB: [f32; 2] = [-0.02, 0.28];
pub const CRANK: f32 = 0.17;
/// The pedals' axles end this far out (the feet are placed there).
const PEDAL_Z: f32 = 0.1;

/// The steering axis at height `y` (73 degrees, a fork offset behind the
/// front hub).
fn steer(y: f32) -> [f32; 2] {
    [FRONT_X - 0.047 - (y - HUB_Y) * 0.306, y]
}

/// Where the fork's crown is, up the steering axis: high enough to clear
/// the front tyre.
const CROWN_Y: f32 = 0.685;

/// The tyres: the radius of the middle of their cross-section, and its
/// radius (a 28 mm tyre).
const TYRE: (f32, f32) = (0.3275, 0.0142);

const CARBON: V3 = [0.022, 0.022, 0.026];
const RUBBER: V3 = [0.016, 0.016, 0.018];
const TAPE: V3 = [0.032, 0.032, 0.036];
const STEEL: V3 = [0.2, 0.2, 0.21];
const SILVER: V3 = [0.5, 0.5, 0.53];
const GRAPHIC: V3 = [0.3, 0.3, 0.32];

/// The cross-section a tube is swept with.
#[derive(Clone, Copy)]
enum Shape {
    /// Round, or elliptic when its depth and width differ.
    Round,
    /// An aero section: a round leading edge, tapering back to a flat,
    /// cut-off tail (a "Kammtail").
    Aero,
    /// A rounded box: the higher the number, the squarer.
    Boxy(f32),
}

impl Shape {
    /// The outline at angle `a`: along the leading direction, then the side.
    fn at(self, a: f32) -> (f32, f32) {
        let (s, c) = a.sin_cos();
        match self {
            Shape::Round => (c, s),
            Shape::Aero => {
                if c >= 0.0 {
                    (c, s)
                } else {
                    ((c * 1.7).max(-1.3), s * (1.0 + 0.32 * c))
                }
            }
            Shape::Boxy(p) => {
                let e = 2.0 / p;
                (c.signum() * c.abs().powf(e), s.signum() * s.abs().powf(e))
            }
        }
    }
}

/// A point of a swept tube: where (in the bike's frame), and its half
/// depth (along the leading direction) and half width.
type Station = (V3, f32, f32);

struct Builder {
    m: Mesh,
    f: Frame,
    far: bool,
}

impl Builder {
    fn at(&self, p: V3) -> V3 {
        self.f.at(p[0], p[1], p[2])
    }
    fn dir(&self, d: V3) -> V3 {
        self.f.dir(d[0], d[1], d[2])
    }
    fn sides(&self, near: usize) -> usize {
        if self.far {
            (near / 2).max(4)
        } else {
            near
        }
    }

    /// Emit a triangle facing `n`, whatever order its corners came in.
    fn tri(&mut self, p: [(V3, V3); 3], col: V3) {
        let face = cross(sub(p[1].0, p[0].0), sub(p[2].0, p[0].0));
        let n = add(add(p[0].1, p[1].1), p[2].1);
        let order = if dot(face, n) >= 0.0 {
            [0, 1, 2]
        } else {
            [0, 2, 1]
        };
        for i in order {
            self.m.push(p[i].0, p[i].1, col);
        }
    }

    /// A surface through closed rings of points, normals pointing away
    /// from each ring's middle, with flat ends if asked.
    fn loft(&mut self, rings: &[Vec<V3>], col: V3, caps: (bool, bool)) {
        let n = rings.len();
        if n < 2 {
            return;
        }
        let k = rings[0].len();
        let centre = |i: usize| {
            let s = rings[i].iter().fold([0.0; 3], |a, &p| add(a, p));
            scale(s, 1.0 / k as f32)
        };
        let normal = |i: usize, j: usize| {
            let along = sub(rings[(i + 1).min(n - 1)][j], rings[i.saturating_sub(1)][j]);
            let around = sub(rings[i][(j + 1) % k], rings[i][(j + k - 1) % k]);
            let mut nn = cross(around, along);
            if dot(nn, sub(rings[i][j], centre(i))) < 0.0 {
                nn = scale(nn, -1.0);
            }
            if len(nn) < 1e-9 {
                return norm(sub(rings[i][j], centre(i)));
            }
            norm(nn)
        };
        for i in 0..n - 1 {
            for j in 0..k {
                let j1 = (j + 1) % k;
                let a = (rings[i][j], normal(i, j));
                let b = (rings[i][j1], normal(i, j1));
                let c = (rings[i + 1][j1], normal(i + 1, j1));
                let d = (rings[i + 1][j], normal(i + 1, j));
                self.tri([a, b, c], col);
                self.tri([a, c, d], col);
            }
        }
        for (end, on) in [(0, caps.0), (n - 1, caps.1)] {
            if !on {
                continue;
            }
            let c = centre(end);
            let other = centre(if end == 0 { 1 } else { n - 2 });
            let out = norm(sub(c, other));
            for j in 0..k {
                let (p, q) = (rings[end][j], rings[end][(j + 1) % k]);
                self.tri([(c, out), (p, out), (q, out)], col);
            }
        }
    }

    /// A tube swept through `stations` (in the bike's frame), its sections
    /// of `shape` with their leading edge towards `lead` and their width
    /// along `side` (both in the bike's frame, made square to the tube).
    #[allow(clippy::too_many_arguments)]
    fn tube(
        &mut self,
        stations: &[Station],
        shape: Shape,
        lead: V3,
        side: V3,
        sides: usize,
        col: V3,
        caps: (bool, bool),
    ) {
        let sides = self.sides(sides);
        let pts: Vec<V3> = stations.iter().map(|s| self.at(s.0)).collect();
        let (lead, side) = (self.dir(lead), self.dir(side));
        let n = pts.len();
        let mut rings = Vec::with_capacity(n);
        // The section's width and leading axes: set by `side` and `lead` at
        // the first point, then carried along the tube without twisting
        // (however it bends), as a ribbon would be.
        let mut axes: Option<(V3, V3)> = None;
        for i in 0..n {
            let t = norm(sub(pts[(i + 1).min(n - 1)], pts[i.saturating_sub(1)]));
            let (w, d) = match axes {
                None => {
                    let mut w = sub(side, scale(t, dot(side, t)));
                    if len(w) < 1e-4 {
                        w = cross(t, lead);
                    }
                    let w = norm(w);
                    let mut d = cross(w, t);
                    if dot(d, lead) < 0.0 {
                        d = scale(d, -1.0);
                    }
                    (w, d)
                }
                Some((w0, d0)) => {
                    let w = norm(sub(w0, scale(t, dot(w0, t))));
                    let mut d = cross(w, t);
                    if dot(d, d0) < 0.0 {
                        d = scale(d, -1.0);
                    }
                    (w, d)
                }
            };
            axes = Some((w, d));
            let (_, half_d, half_w) = stations[i];
            let ring = (0..sides)
                .map(|j| {
                    let (u, v) = shape.at(j as f32 / sides as f32 * TAU);
                    add(pts[i], add(scale(d, u * half_d), scale(w, v * half_w)))
                })
                .collect();
            rings.push(ring);
        }
        self.loft(&rings, col, caps);
    }

    /// A shape turned about an axle at `c` (in the bike's frame), across
    /// the bike: `profile` is a closed outline of (radius, offset to the
    /// right) points, swept over `arc` (start, length) radians from the
    /// top, turned on by `spin`. Smooth across the outline, or with a
    /// crease at every corner.
    #[allow(clippy::too_many_arguments)]
    fn lathe(
        &mut self,
        c: V3,
        profile: &[(f32, f32)],
        segs: usize,
        arc: (f32, f32),
        spin: f32,
        col: V3,
        smooth: bool,
    ) {
        let segs = self.sides(segs);
        let n = profile.len();
        let area: f32 = (0..n)
            .map(|k| {
                let (a, b) = (profile[k], profile[(k + 1) % n]);
                a.0 * b.1 - b.0 * a.1
            })
            .sum();
        // Each edge's outward normal in the outline's plane.
        let edge_n: Vec<(f32, f32)> = (0..n)
            .map(|k| {
                let (a, b) = (profile[k], profile[(k + 1) % n]);
                let (dr, dz) = (b.0 - a.0, b.1 - a.1);
                let l = (dr * dr + dz * dz).sqrt().max(1e-9);
                if area > 0.0 {
                    (dz / l, -dr / l)
                } else {
                    (-dz / l, dr / l)
                }
            })
            .collect();
        let full = arc.1 >= TAU - 1e-4;
        let steps = if full {
            segs
        } else {
            ((segs as f32 * arc.1 / TAU).ceil() as usize).max(2)
        };
        let centre = self.at(c);
        let (up, fwd, right) = (self.f.up, self.f.fwd, self.f.right);
        let radial = |s: f32| add(scale(up, s.cos()), scale(fwd, s.sin()));
        let point =
            |s: f32, (r, z): (f32, f32)| add(centre, add(scale(radial(s), r), scale(right, z)));
        let dir3 = |s: f32, (nr, nz): (f32, f32)| norm(add(scale(radial(s), nr), scale(right, nz)));
        for i in 0..steps {
            let s0 = arc.0 + spin + arc.1 * i as f32 / steps as f32;
            let s1 = arc.0 + spin + arc.1 * (i + 1) as f32 / steps as f32;
            for k in 0..n {
                let k1 = (k + 1) % n;
                let (n0, n1) = if smooth {
                    let avg = |v: usize| {
                        let (a, b) = (edge_n[(v + n - 1) % n], edge_n[v]);
                        let (x, y) = (a.0 + b.0, a.1 + b.1);
                        let l = (x * x + y * y).sqrt().max(1e-9);
                        (x / l, y / l)
                    };
                    (avg(k), avg(k1))
                } else {
                    (edge_n[k], edge_n[k])
                };
                let a = (point(s0, profile[k]), dir3(s0, n0));
                let b = (point(s1, profile[k]), dir3(s1, n0));
                let cc = (point(s1, profile[k1]), dir3(s1, n1));
                let d = (point(s0, profile[k1]), dir3(s0, n1));
                self.tri([a, b, cc], col);
                self.tri([a, cc, d], col);
            }
        }
    }

    /// A flat ring (or disc, with `inner` 0) about an axle at `c`, its two
    /// faces `half` apart across the bike.
    #[allow(clippy::too_many_arguments)]
    fn ring(&mut self, c: V3, outer: f32, inner: f32, half: f32, segs: usize, spin: f32, col: V3) {
        let profile = [(outer, -half), (outer, half), (inner, half), (inner, -half)];
        self.lathe(c, &profile, segs, (0.0, TAU), spin, col, false);
    }
}

/// Points along a smooth curve through `pts` (Catmull-Rom), `per` to each
/// span, their depths and widths blended along.
fn curve(pts: &[Station], per: usize) -> Vec<Station> {
    let n = pts.len();
    if n < 3 {
        return pts.to_vec();
    }
    let mut out = Vec::new();
    for i in 0..n - 1 {
        let p0 = pts[i.saturating_sub(1)].0;
        let (p1, p2) = (pts[i].0, pts[i + 1].0);
        let p3 = pts[(i + 2).min(n - 1)].0;
        for s in 0..per {
            let t = s as f32 / per as f32;
            let (t2, t3) = (t * t, t * t * t);
            let p = [0, 1, 2].map(|a| {
                0.5 * (2.0 * p1[a]
                    + (p2[a] - p0[a]) * t
                    + (2.0 * p0[a] - 5.0 * p1[a] + 4.0 * p2[a] - p3[a]) * t2
                    + (3.0 * p1[a] - p0[a] - 3.0 * p2[a] + p3[a]) * t3)
            });
            let d = pts[i].1 + (pts[i + 1].1 - pts[i].1) * t;
            let w = pts[i].2 + (pts[i + 1].2 - pts[i].2) * t;
            out.push((p, d, w));
        }
    }
    out.push(pts[n - 1]);
    out
}

const FWD: V3 = [1.0, 0.0, 0.0];
const UP: V3 = [0.0, 1.0, 0.0];
const RIGHT: V3 = [0.0, 0.0, 1.0];

/// The aero bike for the pose `j`, its frame painted `paint`. `far` makes
/// a lighter one for riders further off: fewer sides, and no spokes,
/// chain or small parts.
pub fn aero_bike(p: &RiderPose, j: &Joints, paint: V3, far: bool) -> Mesh {
    let mut b = Builder {
        m: Mesh::default(),
        f: j.bike,
        far,
    };
    wheels(&mut b, p.wheel);
    frame(&mut b, paint);
    seat(&mut b);
    cockpit(&mut b);
    drivetrain(&mut b, p, j);
    bottle(&mut b, p.jersey);
    b.m.mat = MAT_MATT;
    b.m
}

fn wheels(b: &mut Builder, wheel: f32) {
    for x in [REAR_X, FRONT_X] {
        let hub = [x, HUB_Y, 0.0];
        // A 28 mm tyre.
        b.m.mat = MAT_MATT;
        let tyre: Vec<(f32, f32)> = (0..6)
            .map(|k| {
                let a = k as f32 / 6.0 * TAU;
                (TYRE.0 + TYRE.1 * a.cos(), TYRE.1 * a.sin())
            })
            .collect();
        b.lathe(hub, &tyre, 56, (0.0, TAU), 0.0, RUBBER, true);
        // A 50 mm deep carbon rim: wide at the tyre, rounded at the spokes.
        b.m.mat = MAT_GLOSS;
        let half = [
            (0.3135, 0.0122),
            (0.304, 0.0139),
            (0.285, 0.0136),
            (0.2685, 0.009),
        ];
        let mut rim: Vec<(f32, f32)> = half.to_vec();
        rim.push((0.2628, 0.0));
        rim.extend(half.iter().rev().map(|&(r, z)| (r, -z)));
        if b.far {
            rim = vec![
                (0.3135, 0.0122),
                (0.285, 0.0136),
                (0.2628, 0.0),
                (0.285, -0.0136),
                (0.3135, -0.0122),
            ];
        }
        b.lathe(hub, &rim, 40, (0.0, TAU), 0.0, CARBON, true);
        // Its graphics: two grey bands on each side, turning with it.
        if !b.far {
            b.m.mat = MAT_MATT;
            for z in [-0.0143f32, 0.0143] {
                for k in 0..2 {
                    let start = wheel + k as f32 * PI + 0.3;
                    let band = [
                        (0.293, z - 0.0004),
                        (0.293, z + 0.0004),
                        (0.285, z + 0.0004),
                        (0.285, z - 0.0004),
                    ];
                    b.lathe(hub, &band, 40, (start, 1.1), 0.0, GRAPHIC, false);
                }
            }
        }
        // The hub, its flanges, and a thru-axle.
        b.m.mat = MAT_GLOSS;
        let hub_profile = [
            (0.013, -0.045),
            (0.013, 0.045),
            (0.006, 0.07),
            (0.006, -0.07),
        ];
        b.lathe(hub, &hub_profile, 14, (0.0, TAU), 0.0, CARBON, false);
        // Far off, the spokes, the hub's flanges and the brake's rotor are
        // too small to see.
        if b.far {
            continue;
        }
        for z in [-0.032f32, 0.032] {
            let flange = [(0.024, z - 0.002), (0.024, z + 0.002), (0.012, z)];
            b.lathe([x, HUB_Y, 0.0], &flange, 12, (0.0, TAU), 0.0, CARBON, false);
        }
        // Bladed spokes, two-cross laced in pairs.
        {
            let count = if x == REAR_X { 24 } else { 20 };
            for k in 0..count {
                let a = wheel + k as f32 / count as f32 * TAU;
                let z = if k % 2 == 0 { 0.032 } else { -0.032 };
                let at_hub = a - 0.12 * if k % 2 == 0 { 1.0 } else { -1.0 };
                let from = [x + at_hub.sin() * 0.022, HUB_Y + at_hub.cos() * 0.022, z];
                let to = [x + a.sin() * 0.264, HUB_Y + a.cos() * 0.264, z * 0.25];
                b.tube(
                    &[(from, 0.0022, 0.0009), (to, 0.0022, 0.0009)],
                    Shape::Round,
                    [a.cos(), -a.sin(), 0.0],
                    RIGHT,
                    3,
                    [0.05, 0.05, 0.055],
                    (false, false),
                );
            }
        }
        // A disc brake's rotor on the left, and its carrier.
        let rz = if x == REAR_X { -0.052 } else { -0.048 };
        b.m.mat = MAT_GLOSS;
        b.ring([x, HUB_Y, rz], 0.08, 0.064, 0.0009, 28, 0.0, SILVER);
        b.ring(
            [x, HUB_Y, rz],
            0.064,
            0.016,
            0.0012,
            14,
            wheel,
            [0.07, 0.07, 0.075],
        );
    }
}

fn frame(b: &mut Builder, paint: V3) {
    b.m.mat = MAT_GLOSS;
    let at = |y: f32, z: f32| {
        let s = steer(y);
        [s[0], s[1], z]
    };
    // Head tube, widening at the bottom into the fork's crown, clear of
    // the tyre.
    b.tube(
        &curve(
            &[
                (at(CROWN_Y, 0.0), 0.034, 0.036),
                (at(CROWN_Y + 0.014, 0.0), 0.035, 0.034),
                (at(0.73, 0.0), 0.034, 0.027),
                (at(0.78, 0.0), 0.031, 0.021),
                (at(0.848, 0.0), 0.028, 0.018),
            ],
            3,
        ),
        Shape::Aero,
        FWD,
        RIGHT,
        16,
        paint,
        (true, true),
    );
    // Down tube, widening into the bottom bracket.
    let bb = [BB[0], BB[1], 0.0];
    b.tube(
        &[
            (at(0.75, 0.0), 0.032, 0.022),
            (at(0.69, 0.0), 0.034, 0.023),
            ([0.21, 0.505, 0.0], 0.033, 0.024),
            ([0.04, 0.335, 0.0], 0.036, 0.028),
            ([BB[0] + 0.006, BB[1] + 0.006, 0.0], 0.038, 0.032),
        ],
        Shape::Aero,
        FWD,
        RIGHT,
        16,
        paint,
        (false, false),
    );
    // Top tube, sloping down to the seat tube.
    let seat_top = [-0.165, 0.775, 0.0];
    b.tube(
        &[
            (at(0.822, 0.0), 0.025, 0.018),
            ([0.12, 0.8, 0.0], 0.022, 0.017),
            ([-0.15, 0.776, 0.0], 0.02, 0.016),
        ],
        Shape::Boxy(2.6),
        UP,
        RIGHT,
        14,
        paint,
        (false, false),
    );
    // Seat tube, curving a little round the back wheel.
    b.tube(
        &curve(
            &[
                ([BB[0] - 0.004, BB[1] + 0.01, 0.0], 0.034, 0.024),
                ([-0.052, 0.44, 0.0], 0.031, 0.021),
                ([-0.1, 0.6, 0.0], 0.028, 0.02),
                (seat_top, 0.026, 0.019),
            ],
            3,
        ),
        Shape::Aero,
        FWD,
        RIGHT,
        16,
        paint,
        (false, true),
    );
    // The bottom bracket's shell.
    b.lathe(
        bb,
        &[
            (0.026, -0.044),
            (0.026, 0.044),
            (0.014, 0.044),
            (0.014, -0.044),
        ],
        16,
        (0.0, TAU),
        0.0,
        paint,
        false,
    );
    for side in [1.0f32, -1.0] {
        let drop = [REAR_X, HUB_Y, side * 0.062];
        // Dropped seat stays, from low on the seat tube.
        b.tube(
            &curve(
                &[
                    ([-0.112, 0.61, side * 0.012], 0.011, 0.009),
                    ([-0.3, 0.47, side * 0.05], 0.009, 0.0075),
                    ([REAR_X + 0.012, HUB_Y + 0.012, side * 0.062], 0.008, 0.007),
                ],
                4,
            ),
            Shape::Round,
            UP,
            RIGHT,
            10,
            paint,
            (false, false),
        );
        // Chain stays: deep at the bottom bracket, the drive side deeper.
        let deep = if side > 0.0 { 1.12 } else { 1.0 };
        b.tube(
            &curve(
                &[
                    (
                        [BB[0] - 0.02, BB[1] + 0.006, side * 0.036],
                        0.017 * deep,
                        0.012,
                    ),
                    ([-0.27, 0.31, side * 0.058], 0.012 * deep, 0.009),
                    ([REAR_X + 0.012, HUB_Y - 0.006, side * 0.062], 0.009, 0.007),
                ],
                4,
            ),
            Shape::Round,
            UP,
            RIGHT,
            10,
            paint,
            (false, false),
        );
        // The dropout round the axle.
        b.lathe(
            drop,
            &[
                (0.017, -0.004),
                (0.017, 0.004),
                (0.007, 0.004),
                (0.007, -0.004),
            ],
            14,
            (0.0, TAU),
            0.0,
            paint,
            false,
        );
        // Fork blades, growing out of the crown either side of the tyre
        // and sweeping forward to the front hub.
        let root = steer(CROWN_Y + 0.008);
        let mid = steer(0.6);
        b.tube(
            &curve(
                &[
                    ([root[0], root[1], side * 0.02], 0.027, 0.012),
                    (
                        [root[0] + 0.006, root[1] - 0.03, side * 0.036],
                        0.026,
                        0.012,
                    ),
                    ([mid[0] + 0.024, 0.6, side * 0.048], 0.022, 0.011),
                    ([FRONT_X - 0.036, 0.46, side * 0.053], 0.017, 0.0095),
                    ([FRONT_X, HUB_Y, side * 0.054], 0.011, 0.008),
                ],
                4,
            ),
            Shape::Aero,
            FWD,
            RIGHT,
            12,
            paint,
            (false, true),
        );
    }
}

fn seat(b: &mut Builder) {
    b.m.mat = MAT_GLOSS;
    // An aero seat post, along the seat tube.
    let post_dir = norm([-0.145, 0.495, 0.0]);
    let top = add([-0.165, 0.775, 0.0], scale(post_dir, 0.105));
    b.tube(
        &[([-0.163, 0.765, 0.0], 0.022, 0.015), (top, 0.02, 0.014)],
        Shape::Aero,
        FWD,
        RIGHT,
        14,
        CARBON,
        (false, true),
    );
    // The clamp and rails.
    b.tube(
        &[
            ([top[0] - 0.02, top[1] + 0.006, 0.0], 0.008, 0.018),
            ([top[0] + 0.02, top[1] + 0.006, 0.0], 0.008, 0.018),
        ],
        Shape::Boxy(4.0),
        UP,
        RIGHT,
        8,
        CARBON,
        (true, true),
    );
    for side in [1.0f32, -1.0] {
        b.tube(
            &curve(
                &[
                    ([-0.275, 0.905, side * 0.03], 0.0035, 0.0035),
                    ([-0.24, 0.891, side * 0.024], 0.0035, 0.0035),
                    ([-0.14, 0.893, side * 0.018], 0.0035, 0.0035),
                    ([-0.105, 0.905, side * 0.014], 0.0035, 0.0035),
                ],
                3,
            ),
            Shape::Round,
            UP,
            RIGHT,
            5,
            STEEL,
            (false, false),
        );
    }
    // A short-nosed race saddle, its top where the classic bike's is.
    b.m.mat = MAT_MATT;
    let slices: [(f32, f32, f32, f32); 8] = [
        // x, top, half width, half thickness
        (-0.302, 0.914, 0.034, 0.006),
        (-0.296, 0.921, 0.058, 0.011),
        (-0.275, 0.925, 0.071, 0.013),
        (-0.235, 0.925, 0.068, 0.013),
        (-0.19, 0.923, 0.046, 0.012),
        (-0.14, 0.922, 0.03, 0.011),
        (-0.1, 0.92, 0.024, 0.01),
        (-0.082, 0.913, 0.016, 0.006),
    ];
    let stations: Vec<Station> = slices
        .iter()
        .map(|&(x, top, w, t)| ([x, top - t, 0.0], t, w))
        .collect();
    b.tube(
        &curve(&stations, 2),
        Shape::Boxy(3.0),
        UP,
        RIGHT,
        18,
        RUBBER,
        (true, true),
    );
}

fn cockpit(b: &mut Builder) {
    b.m.mat = MAT_GLOSS;
    let head_top = steer(0.848);
    let cap = steer(0.872);
    // Spacers and the top cap.
    b.tube(
        &[
            ([head_top[0], head_top[1], 0.0], 0.019, 0.017),
            ([cap[0], cap[1], 0.0], 0.019, 0.017),
        ],
        Shape::Aero,
        FWD,
        RIGHT,
        12,
        CARBON,
        (false, true),
    );
    // The one-piece bar and stem: the stem forward from the steerer...
    let stem = steer(0.862);
    b.tube(
        &[
            ([stem[0] - 0.012, 0.862, 0.0], 0.018, 0.02),
            ([0.44, 0.863, 0.0], 0.016, 0.018),
            ([0.5, 0.864, 0.0], 0.014, 0.016),
        ],
        Shape::Boxy(3.0),
        UP,
        RIGHT,
        12,
        CARBON,
        (true, false),
    );
    // ...the flat aero tops, narrowing at each end into the round bar...
    let tops: Vec<Station> = [
        ([0.503, 0.8635, -0.165], 0.0135, 0.0108),
        ([0.498, 0.864, -0.13], 0.021, 0.0094),
        ([0.495, 0.864, -0.07], 0.024, 0.0089),
        ([0.494, 0.864, 0.0], 0.025, 0.0088),
        ([0.495, 0.864, 0.07], 0.024, 0.0089),
        ([0.498, 0.864, 0.13], 0.021, 0.0094),
        ([0.503, 0.8635, 0.165], 0.0135, 0.0108),
    ]
    .to_vec();
    b.tube(
        &curve(&tops, 3),
        Shape::Aero,
        FWD,
        UP,
        14,
        CARBON,
        (true, true),
    );
    // ...and round in front, down into the drops, taped.
    b.m.mat = MAT_MATT;
    for side in [1.0f32, -1.0] {
        let z = |s: f32| side * s;
        b.tube(
            &curve(
                &[
                    ([0.4985, 0.8638, z(0.135)], 0.0112, 0.0112),
                    ([0.51, 0.862, z(0.182)], 0.0118, 0.0118),
                    ([0.548, 0.86, z(0.197)], 0.0118, 0.0118),
                    ([0.585, 0.856, z(0.2)], 0.0118, 0.0118),
                    ([0.603, 0.844, z(0.2015)], 0.0118, 0.0118),
                    ([0.612, 0.824, z(0.203)], 0.0118, 0.0118),
                    ([0.608, 0.775, z(0.207)], 0.0118, 0.0118),
                    ([0.58, 0.742, z(0.21)], 0.0118, 0.0118),
                    ([0.535, 0.736, z(0.21)], 0.0118, 0.0118),
                ],
                3,
            ),
            Shape::Round,
            UP,
            RIGHT,
            10,
            TAPE,
            (false, true),
        );
        // The hood, rising to a horn where the hand holds it.
        b.tube(
            &curve(
                &[
                    ([0.574, 0.857, z(0.2)], 0.016, 0.0135),
                    ([0.596, 0.874, z(0.2)], 0.018, 0.0145),
                    ([0.614, 0.896, z(0.2)], 0.016, 0.0135),
                    ([0.624, 0.915, z(0.2)], 0.01, 0.0105),
                ],
                2,
            ),
            Shape::Boxy(2.6),
            UP,
            RIGHT,
            12,
            RUBBER,
            (true, true),
        );
        // The brake lever, curving down in front of the drops.
        b.m.mat = MAT_GLOSS;
        b.tube(
            &curve(
                &[
                    ([0.627, 0.902, z(0.2)], 0.008, 0.007),
                    ([0.645, 0.86, z(0.202)], 0.0075, 0.006),
                    ([0.647, 0.802, z(0.205)], 0.0065, 0.0055),
                    ([0.634, 0.764, z(0.207)], 0.006, 0.005),
                ],
                2,
            ),
            Shape::Aero,
            FWD,
            RIGHT,
            8,
            CARBON,
            (false, true),
        );
        b.m.mat = MAT_MATT;
    }
    if b.far {
        return;
    }
    // A bike computer on an out-front mount, a light underneath.
    b.m.mat = MAT_GLOSS;
    b.tube(
        &[
            ([0.495, 0.866, 0.0], 0.005, 0.012),
            ([0.585, 0.866, 0.0], 0.005, 0.012),
        ],
        Shape::Boxy(4.0),
        UP,
        RIGHT,
        8,
        CARBON,
        (false, true),
    );
    b.tube(
        &[
            ([0.565, 0.878, 0.0], 0.009, 0.026),
            ([0.632, 0.874, 0.0], 0.008, 0.026),
        ],
        Shape::Boxy(5.0),
        UP,
        RIGHT,
        12,
        [0.05, 0.05, 0.055],
        (true, true),
    );
    b.m.mat = MAT_LAMP;
    b.tube(
        &[
            ([0.572, 0.8875, 0.0], 0.0006, 0.02),
            ([0.624, 0.8835, 0.0], 0.0006, 0.02),
        ],
        Shape::Boxy(6.0),
        UP,
        RIGHT,
        8,
        [0.16, 0.2, 0.19],
        (true, true),
    );
    b.m.mat = MAT_GLOSS;
    b.tube(
        &[
            ([0.56, 0.848, 0.0], 0.011, 0.011),
            ([0.6, 0.848, 0.0], 0.011, 0.011),
        ],
        Shape::Round,
        UP,
        RIGHT,
        10,
        CARBON,
        (false, false),
    );
    b.m.mat = MAT_LAMP;
    b.ring_facing_forward([0.6005, 0.848, 0.0], 0.0095, [0.95, 0.9, 0.78]);
    // A rear light on the seat post.
    b.m.mat = MAT_GLOSS;
    b.tube(
        &[
            ([-0.205, 0.825, 0.0], 0.008, 0.011),
            ([-0.21, 0.795, 0.0], 0.008, 0.011),
        ],
        Shape::Boxy(4.0),
        FWD,
        RIGHT,
        8,
        CARBON,
        (true, true),
    );
    b.m.mat = MAT_LAMP;
    b.tube(
        &[
            ([-0.2145, 0.823, 0.0], 0.0015, 0.008),
            ([-0.2185, 0.797, 0.0], 0.0015, 0.008),
        ],
        Shape::Boxy(4.0),
        FWD,
        RIGHT,
        8,
        [0.95, 0.03, 0.02],
        (true, true),
    );
    b.m.mat = MAT_MATT;
}

impl Builder {
    /// A small disc facing forward (a lamp's lens).
    fn ring_facing_forward(&mut self, c: V3, r: f32, col: V3) {
        let (c, n) = (self.at(c), self.f.fwd);
        let (u, v) = (self.f.up, self.f.right);
        let k = 12;
        for i in 0..k {
            let a0 = i as f32 / k as f32 * TAU;
            let a1 = (i + 1) as f32 / k as f32 * TAU;
            let p = |a: f32| add(c, add(scale(u, a.cos() * r), scale(v, a.sin() * r)));
            self.tri([(c, n), (p(a0), n), (p(a1), n)], col);
        }
    }
}

/// Where a chain leaves the top of one sprocket (`a`, radius `ra`) for the
/// top of another: the points the line touches both at.
fn top_tangent(a: [f32; 2], ra: f32, c: [f32; 2], rc: f32) -> ([f32; 2], [f32; 2]) {
    let (dx, dy) = (a[0] - c[0], a[1] - c[1]);
    let d = (dx * dx + dy * dy).sqrt();
    let e = [dx / d, dy / d];
    // Square to the line between them, pointing up.
    let f = [-e[1], e[0]];
    let f = if f[1] < 0.0 { [-f[0], -f[1]] } else { f };
    let k = (ra - rc) / d;
    let s = (1.0 - k * k).max(0.0).sqrt();
    let n = [e[0] * k + f[0] * s, e[1] * k + f[1] * s];
    (
        [a[0] + n[0] * ra, a[1] + n[1] * ra],
        [c[0] + n[0] * rc, c[1] + n[1] * rc],
    )
}

fn drivetrain(b: &mut Builder, p: &RiderPose, j: &Joints) {
    let rear = [REAR_X, HUB_Y];
    // The axle through the bottom bracket.
    b.m.mat = MAT_GLOSS;
    b.tube(
        &[
            ([BB[0], BB[1], -0.07], 0.012, 0.012),
            ([BB[0], BB[1], 0.07], 0.012, 0.012),
        ],
        Shape::Round,
        UP,
        RIGHT,
        10,
        STEEL,
        (false, false),
    );
    // Chainrings, 52 and 36, on a four-armed spider that turns with the
    // cranks.
    let (big, small, ring_z) = (0.106, 0.077, 0.052);
    b.ring(
        [BB[0], BB[1], ring_z],
        big,
        big - 0.026,
        0.0016,
        36,
        0.0,
        [0.06, 0.06, 0.065],
    );
    b.ring(
        [BB[0], BB[1], ring_z - 0.006],
        small,
        small - 0.011,
        0.0015,
        28,
        0.0,
        [0.12, 0.12, 0.13],
    );
    // The front derailleur's cage, just above the big ring, on its mount.
    if !b.far {
        let cage = [
            (0.128, ring_z + 0.009),
            (0.128, ring_z + 0.011),
            (0.111, ring_z + 0.011),
            (0.111, ring_z + 0.009),
        ];
        b.lathe(
            [BB[0], BB[1], 0.0],
            &cage,
            36,
            (-0.95, 0.75),
            0.0,
            CARBON,
            false,
        );
        b.tube(
            &[
                ([-0.07, 0.425, 0.0], 0.012, 0.012),
                ([-0.072, 0.405, ring_z + 0.008], 0.009, 0.008),
            ],
            Shape::Boxy(3.0),
            UP,
            FWD,
            8,
            CARBON,
            (true, true),
        );
    }
    for k in 0..4 {
        let a = p.crank + PI * 0.25 + k as f32 * PI * 0.5;
        let (s, c) = a.sin_cos();
        b.tube(
            &[
                (
                    [BB[0] + s * 0.018, BB[1] + c * 0.018, ring_z + 0.002],
                    0.012,
                    0.0028,
                ),
                (
                    [BB[0] + s * 0.096, BB[1] + c * 0.096, ring_z + 0.002],
                    0.008,
                    0.0025,
                ),
            ],
            Shape::Boxy(4.0),
            [c, -s, 0.0],
            RIGHT,
            8,
            CARBON,
            (false, true),
        );
    }
    // Crank arms and pedals.
    for (k, side) in [1.0f32, -1.0].into_iter().enumerate() {
        let th = p.crank + if side > 0.0 { 0.0 } else { PI };
        let (s, c) = th.sin_cos();
        let arm_z = side * 0.068;
        let end = [BB[0] + CRANK * s, BB[1] + CRANK * c];
        b.m.mat = MAT_GLOSS;
        b.tube(
            &[
                ([BB[0], BB[1], arm_z], 0.016, 0.008),
                (
                    [BB[0] + 0.5 * CRANK * s, BB[1] + 0.5 * CRANK * c, arm_z],
                    0.012,
                    0.007,
                ),
                ([end[0], end[1], arm_z], 0.011, 0.007),
            ],
            Shape::Boxy(3.0),
            [c, -s, 0.0],
            RIGHT,
            10,
            CARBON,
            (true, true),
        );
        b.tube(
            &[
                ([end[0], end[1], arm_z], 0.005, 0.005),
                ([end[0], end[1], side * PEDAL_Z], 0.005, 0.005),
            ],
            Shape::Round,
            UP,
            RIGHT,
            6,
            STEEL,
            (false, false),
        );
        // The pedal's body tilts with the shoe clipped into it.
        let spindle = b.at([end[0], end[1], side * PEDAL_Z]);
        let (fwd, up) = (j.foot_fwd[k], j.foot_up[k]);
        let pedal = |x: f32, y: f32| add(spindle, add(scale(fwd, x), scale(up, y)));
        let saved = b.f;
        b.f = Frame {
            pos: [0.0; 3],
            fwd: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            right: [0.0, 0.0, 1.0],
        };
        let width = scale(j.bike.right, 1.0);
        let (lead, w) = (up, width);
        b.tube(
            &[
                (pedal(-0.034, -0.002), 0.006, 0.026),
                (pedal(0.0, 0.0), 0.009, 0.034),
                (pedal(0.042, 0.002), 0.006, 0.03),
            ],
            Shape::Boxy(3.0),
            lead,
            w,
            10,
            CARBON,
            (true, true),
        );
        b.f = saved;
    }
    b.m.mat = MAT_GLOSS;
    // A 12-speed cassette, 11 to 34, stepped in one piece.
    let mut cass: Vec<(f32, f32)> = Vec::new();
    let cogs = if b.far { 4 } else { 12 };
    for i in 0..cogs {
        let r = 0.0225 + (0.069 - 0.0225) * (1.0 - i as f32 / (cogs - 1) as f32);
        let z = 0.02 + i as f32 * 0.036 / cogs as f32;
        cass.push((r, z));
        cass.push((r, z + 0.0016));
        cass.push((r - 0.005, z + 0.036 / cogs as f32));
    }
    cass.push((0.014, 0.058));
    cass.push((0.014, 0.02));
    b.lathe(
        [REAR_X, HUB_Y, 0.0],
        &cass,
        12,
        (0.0, TAU),
        0.0,
        SILVER,
        false,
    );
    if b.far {
        return;
    }
    // Disc brake calipers on the left: on the fork, and on the chain stay.
    for (c, rot) in [
        ([FRONT_X - 0.058, HUB_Y + 0.05, -0.054], -0.85f32),
        ([REAR_X + 0.06, HUB_Y + 0.042, -0.058], 0.95),
    ] {
        let (s, co) = rot.sin_cos();
        let d = [co * 0.024, s * 0.024, 0.0];
        b.tube(
            &[(sub(c, d), 0.013, 0.011), (add(c, d), 0.013, 0.011)],
            Shape::Boxy(4.0),
            [-s, co, 0.0],
            RIGHT,
            8,
            CARBON,
            (true, true),
        );
    }
    // The rear derailleur under the cassette: its body, cage and wheels.
    // The cog in use (a middle one): its radius, and where across.
    let cog = (0.0395, 0.042);
    let upper = [REAR_X - 0.008, HUB_Y - 0.072];
    let lower = [REAR_X + 0.028, HUB_Y - 0.148];
    let rd_z = cog.1;
    b.tube(
        &curve(
            &[
                ([REAR_X + 0.004, HUB_Y - 0.008, 0.066], 0.012, 0.009),
                ([REAR_X - 0.036, HUB_Y - 0.04, 0.072], 0.014, 0.011),
                (
                    [upper[0] - 0.006, upper[1] + 0.004, rd_z + 0.016],
                    0.012,
                    0.009,
                ),
            ],
            3,
        ),
        Shape::Boxy(3.0),
        UP,
        RIGHT,
        10,
        CARBON,
        (true, true),
    );
    for z in [rd_z - 0.011, rd_z + 0.011] {
        b.tube(
            &[
                ([upper[0], upper[1], z], 0.016, 0.0015),
                ([lower[0], lower[1], z], 0.017, 0.0015),
            ],
            Shape::Boxy(5.0),
            [0.9, 0.4, 0.0],
            RIGHT,
            8,
            CARBON,
            (true, true),
        );
    }
    for w in [upper, lower] {
        b.ring(
            [w[0], w[1], rd_z],
            0.0155,
            0.004,
            0.003,
            14,
            p.wheel,
            [0.06, 0.06, 0.065],
        );
    }
    // The chain: over the top from the big ring to the cog, round the back
    // of the cog, down through the derailleur and back along the bottom.
    let chain_z = |x: f32| {
        let t = ((x - BB[0]) / (REAR_X - BB[0])).clamp(0.0, 1.0);
        ring_z + (cog.1 - ring_z) * t
    };
    let ring_c = [BB[0], BB[1]];
    let (front_top, back_top) = top_tangent(ring_c, big, rear, cog.0);
    // Angles round a sprocket are measured from its top, growing forward.
    let a_top = (front_top[0] - ring_c[0]).atan2(front_top[1] - ring_c[1]);
    let mut links: Vec<[f32; 2]> = Vec::new();
    links.push(front_top);
    links.push(back_top);
    // Round the back of the cog, from its top tangent to its bottom.
    let a0 = (back_top[0] - rear[0]).atan2(back_top[1] - rear[1]);
    for k in 1..=8 {
        let a = a0 - k as f32 / 8.0 * (PI - 0.3);
        links.push([rear[0] + a.sin() * cog.0, rear[1] + a.cos() * cog.0]);
    }
    // Down through the jockey wheels.
    links.push([upper[0] + 0.0155, upper[1]]);
    links.push([lower[0] - 0.0155, lower[1]]);
    links.push([lower[0], lower[1] - 0.0155]);
    // Forward along the bottom to the ring, then round its front.
    let bottom_in = [ring_c[0] - 0.012, ring_c[1] - big];
    links.push(bottom_in);
    for k in 1..=12 {
        let a = PI - k as f32 / 12.0 * (PI - a_top);
        links.push([ring_c[0] + a.sin() * big, ring_c[1] + a.cos() * big]);
    }
    let chain: Vec<Station> = links
        .iter()
        .map(|q| ([q[0], q[1], chain_z(q[0])], 0.0034, 0.0028))
        .collect();
    b.tube(
        &chain,
        Shape::Boxy(4.0),
        UP,
        RIGHT,
        6,
        [0.13, 0.13, 0.14],
        (false, false),
    );
}

fn bottle(b: &mut Builder, cap: V3) {
    // On the down tube, in a black cage.
    let bb = [BB[0], BB[1]];
    let top = steer(0.69);
    let along = norm([top[0] - bb[0], top[1] - bb[1], 0.0]);
    let out = [-along[1], along[0], 0.0];
    let length = len([top[0] - bb[0], top[1] - bb[1], 0.0]);
    let at = |t: f32, off: f32| {
        add(
            add([bb[0], bb[1], 0.0], scale(along, t * length)),
            scale(out, off),
        )
    };
    let axis = 0.072;
    let profile: [(f32, f32); 7] = [
        (0.24, 0.022),
        (0.26, 0.035),
        (0.45, 0.036),
        (0.53, 0.034),
        (0.565, 0.02),
        (0.575, 0.014),
        (0.6, 0.012),
    ];
    b.m.mat = MAT_MATT;
    let body: Vec<Station> = profile[..5]
        .iter()
        .map(|&(t, r)| (at(t, axis), r, r))
        .collect();
    b.tube(
        &body,
        Shape::Round,
        out,
        RIGHT,
        14,
        [0.86, 0.86, 0.84],
        (true, false),
    );
    let lid: Vec<Station> = profile[4..]
        .iter()
        .map(|&(t, r)| (at(t, axis), r, r))
        .collect();
    b.tube(&lid, Shape::Round, out, RIGHT, 10, cap, (false, true));
    if b.far {
        return;
    }
    b.m.mat = MAT_GLOSS;
    for side in [1.0f32, -1.0] {
        let rail: Vec<Station> = [0.27, 0.36, 0.48]
            .iter()
            .map(|&t| {
                let p = at(t, axis - 0.012);
                ([p[0], p[1], side * 0.03], 0.0022, 0.0022)
            })
            .collect();
        b.tube(&rail, Shape::Round, out, RIGHT, 5, CARBON, (true, true));
    }
}

#[cfg(test)]
mod tests {
    use super::super::mesh::{joints, VERTEX_STRIDE};
    use super::*;

    fn pose(crank: f32, stand: f32) -> RiderPose {
        RiderPose {
            pos: [0.0; 3],
            fwd: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            right: [0.0, 0.0, 1.0],
            crank,
            wheel: crank * 2.4,
            jersey: [0.8, 0.3, 0.1],
            stand,
            tuck: 0.0,
            effort: 1.0,
            time: 0.0,
        }
    }

    fn verts(m: &Mesh) -> Vec<(V3, V3)> {
        m.v.chunks(VERTEX_STRIDE)
            .map(|v| ([v[0], v[1], v[2]], [v[3], v[4], v[5]]))
            .collect()
    }

    #[test]
    fn the_aero_bike_is_well_formed() {
        for far in [false, true] {
            for crank in [0.0f32, 0.9, 2.2, 4.0] {
                let p = pose(crank, 0.0);
                let m = aero_bike(&p, &joints(&p), [0.85, 0.86, 0.88], far);
                assert_eq!(m.v.len() % (VERTEX_STRIDE * 3), 0);
                for (pos, n) in verts(&m) {
                    assert!(pos.iter().all(|c| c.is_finite()), "{pos:?}");
                    let l = len(n);
                    assert!((l - 1.0).abs() < 0.02, "normal of length {l} at {pos:?}");
                }
            }
        }
    }

    #[test]
    fn the_aero_bike_stands_on_its_tyres_and_fits_the_rider() {
        let p = pose(1.0, 0.0);
        let j = joints(&p);
        let v = verts(&aero_bike(&p, &j, [0.85, 0.86, 0.88], false));
        // The tyres touch the ground, nothing goes below it.
        let low = v.iter().map(|(q, _)| q[1]).fold(f32::MAX, f32::min);
        assert!(low.abs() < 0.004, "lowest point {low}");
        // The saddle's top is where the classic bike's is.
        let saddle = v
            .iter()
            .filter(|(q, _)| (q[0] + 0.21).abs() < 0.02 && q[2].abs() < 0.02 && q[1] > 0.86)
            .map(|(q, _)| q[1])
            .fold(f32::MIN, f32::max);
        assert!((saddle - 0.925).abs() < 0.006, "saddle top {saddle}");
        // Each hand's grip is on its hood.
        for g in j.grip {
            let near = v
                .iter()
                .map(|(q, _)| len(sub(*q, g)))
                .fold(f32::MAX, f32::min);
            assert!(near < 0.025, "hood {near} m from the grip");
        }
        // Each pedal is under its foot.
        for b in j.ball {
            let near = v
                .iter()
                .map(|(q, _)| len(sub(*q, b)))
                .fold(f32::MAX, f32::min);
            assert!(near < 0.03, "pedal {near} m from the foot");
        }
    }

    /// How far `q` is outside the tyres (less than 0 inside one): each tyre
    /// a ring round its hub.
    fn outside_tyres(q: V3) -> f32 {
        [REAR_X, FRONT_X]
            .iter()
            .map(|&x| {
                let rho = (q[0] - x).hypot(q[1] - HUB_Y);
                (rho - TYRE.0).hypot(q[2]) - TYRE.1
            })
            .fold(f32::MAX, f32::min)
    }

    #[test]
    fn nothing_rubs_on_the_tyres() {
        let p = pose(1.0, 0.0);
        let j = joints(&p);
        let mut b = Builder {
            m: Mesh::default(),
            f: j.bike,
            far: false,
        };
        frame(&mut b, [0.8; 3]);
        seat(&mut b);
        cockpit(&mut b);
        drivetrain(&mut b, &p, &j);
        bottle(&mut b, [0.8; 3]);
        let (gap, at) = verts(&b.m)
            .iter()
            .map(|&(q, _)| (outside_tyres(q), q))
            .fold((f32::MAX, [0.0; 3]), |a, c| if c.0 < a.0 { c } else { a });
        assert!(gap > 0.003, "{:.1} mm from a tyre at {at:?}", gap * 1000.0);
    }

    /// Cheap for the graphics card. Building it is the cost: about 0.7 ms
    /// (0.3 far off) here, against the classic bike's 0.04, so the app
    /// should build the parts that do not turn once, not every frame.
    #[test]
    fn the_aero_bike_stays_cheap_to_draw() {
        let p = pose(1.0, 0.0);
        let j = joints(&p);
        let tris = |far| aero_bike(&p, &j, [0.85, 0.86, 0.88], far).vertex_count() / 3;
        let (near, far) = (tris(false), tris(true));
        assert!(near < 14_000, "{near} triangles");
        assert!(far < 5_000, "{far} triangles far off");
    }
}
