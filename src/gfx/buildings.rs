//! Farm buildings, in metres: x along the building (its long side faces
//! the road), z across it, y up. Their roofs take each building's tint.

use super::math::*;
use super::mesh::{tinted, Mesh, MAT_GLOSS, MAT_MATT, MAT_WINDOW};

const STONE: V3 = [0.2, 0.19, 0.18];
const TIMBER: V3 = [0.11, 0.07, 0.045];
const FRAME: V3 = [0.58, 0.56, 0.52];
const GLASS: V3 = [0.03, 0.04, 0.055];

/// A side of a building, seen from outside.
#[derive(Clone, Copy, PartialEq)]
enum Side {
    /// The long sides: +z (front, towards the road) and -z.
    Front,
    Back,
    /// The gable ends: +x and -x.
    East,
    West,
}

/// A point on a wall plane: `u` to the right as seen from outside, `v` up,
/// `d` out from the building's centre.
fn at(side: Side, u: f32, v: f32, d: f32) -> V3 {
    match side {
        Side::Front => [u, v, d],
        Side::Back => [-u, v, -d],
        Side::East => [d, v, -u],
        Side::West => [-d, v, u],
    }
}

impl Mesh {
    /// A flat rectangle on a wall, facing out.
    #[allow(clippy::too_many_arguments)]
    fn panel(&mut self, side: Side, u0: f32, u1: f32, v0: f32, v1: f32, d: f32, col: V3) {
        self.quad(
            at(side, u0, v0, d),
            at(side, u1, v0, d),
            at(side, u1, v1, d),
            at(side, u0, v1, d),
            col,
        );
    }

    /// A window centred at `u`, from `v0` to `v1`, on a wall `d` out: a
    /// frame, glass (lit at night when `lit`), a cross of glazing bars, a
    /// sill and, when given their colour, shutters.
    #[allow(clippy::too_many_arguments)]
    fn window(
        &mut self,
        side: Side,
        u: f32,
        w: f32,
        v0: f32,
        v1: f32,
        d: f32,
        lit: bool,
        shutters: Option<V3>,
    ) {
        let (a, b) = (u - w / 2.0, u + w / 2.0);
        let f = 0.08;
        // Layers far enough apart not to flicker against each other from
        // a distance.
        self.panel(side, a - f, b + f, v0 - f, v1 + f, d + 0.04, FRAME);
        self.mat = if lit { MAT_WINDOW } else { MAT_GLOSS };
        self.panel(side, a, b, v0, v1, d + 0.07, GLASS);
        self.mat = MAT_MATT;
        let bar = 0.035;
        self.panel(side, u - bar, u + bar, v0, v1, d + 0.1, FRAME);
        let mid = v0 + (v1 - v0) * 0.6;
        self.panel(side, a, b, mid - bar, mid + bar, d + 0.1, FRAME);
        // The sill sticks out.
        let s0 = at(side, a - 0.12, v0 - f - 0.07, d);
        let s1 = at(side, b + 0.12, v0 - f, d + 0.12);
        self.cuboid(
            [s0[0].min(s1[0]), s0[1], s0[2].min(s1[2])],
            [s0[0].max(s1[0]), s1[1], s0[2].max(s1[2])],
            [0.42, 0.4, 0.37],
        );
        if let Some(col) = shutters {
            let sw = w * 0.5;
            self.panel(
                side,
                a - f - sw - 0.04,
                a - f - 0.04,
                v0 - f,
                v1 + f,
                d + 0.05,
                col,
            );
            self.panel(
                side,
                b + f + 0.04,
                b + f + sw + 0.04,
                v0 - f,
                v1 + f,
                d + 0.05,
                col,
            );
        }
    }

    /// A door centred at `u`, `w` wide and `h` high, with its frame, a
    /// step in front and, if `canopy`, a little roof over it.
    #[allow(clippy::too_many_arguments)]
    fn door(&mut self, side: Side, u: f32, w: f32, h: f32, d: f32, col: V3, canopy: bool) {
        let (a, b) = (u - w / 2.0, u + w / 2.0);
        self.panel(side, a - 0.1, b + 0.1, 0.0, h + 0.1, d + 0.04, FRAME);
        self.panel(side, a, b, 0.0, h, d + 0.07, col);
        // Two panels and a handle.
        let ins = scale(col, 0.75);
        self.panel(side, a + 0.12, b - 0.12, h * 0.55, h - 0.15, d + 0.09, ins);
        self.panel(side, a + 0.12, b - 0.12, 0.15, h * 0.45, d + 0.09, ins);
        self.panel(
            side,
            b - 0.2,
            b - 0.12,
            h * 0.48,
            h * 0.52,
            d + 0.1,
            [0.5, 0.45, 0.3],
        );
        let p0 = at(side, a - 0.35, -0.3, d);
        let p1 = at(side, b + 0.35, 0.0, d + 0.55);
        self.cuboid(
            [p0[0].min(p1[0]), p0[1], p0[2].min(p1[2])],
            [p0[0].max(p1[0]), p1[1], p0[2].max(p1[2])],
            STONE,
        );
        if canopy {
            let c0 = at(side, a - 0.5, h + 0.25, d);
            let c1 = at(side, b + 0.5, h + 0.4, d + 0.9);
            self.cuboid(
                [c0[0].min(c1[0]), c0[1], c0[2].min(c1[2])],
                [c0[0].max(c1[0]), c1[1], c0[2].max(c1[2])],
                tinted([0.85, 0.85, 0.85]),
            );
        }
    }

    /// Walls and a pitched roof: a box `len` by `dep` up to `wall`, gables
    /// up to the ridge at `ridge`, the roof overhanging by `over`, with
    /// thickness, fascia boards and a ridge cap. The roof takes the tint.
    #[allow(clippy::too_many_arguments)]
    fn gabled(
        &mut self,
        len: f32,
        dep: f32,
        wall: f32,
        ridge: f32,
        over: f32,
        walls: V3,
        plinth: f32,
    ) {
        let (hx, hz) = (len / 2.0, dep / 2.0);
        // A stone footing, then the walls.
        self.cuboid(
            [-hx - 0.06, -0.6, -hz - 0.06],
            [hx + 0.06, plinth, hz + 0.06],
            STONE,
        );
        self.cuboid([-hx, plinth, -hz], [hx, wall, hz], walls);
        // The gables.
        for (x, s) in [(hx, 1.0f32), (-hx, -1.0)] {
            let (a, b) = ([x, wall, -hz * s], [x, wall, hz * s]);
            self.tri(a, [x, ridge, 0.0], b, walls);
        }
        // Each side of the roof: the top, the underside, the fascia board
        // along the eaves and the boards along the gable edges.
        let roof = tinted([1.0, 1.0, 1.0]);
        let under = tinted([0.42, 0.42, 0.42]);
        let t = 0.18;
        let slope = (ridge - wall) / hz;
        let (ex, ez) = (hx + over, hz + over);
        let ey = wall - over * slope;
        for s in [1.0f32, -1.0] {
            let eave = |x: f32, dy: f32| [x, ey + dy, ez * s];
            let top = |x: f32, dy: f32| [x, ridge + dy, 0.0];
            if s > 0.0 {
                self.quad(eave(-ex, t), eave(ex, t), top(ex, t), top(-ex, t), roof);
                self.quad(
                    eave(-ex, 0.0),
                    top(-ex, 0.0),
                    top(ex, 0.0),
                    eave(ex, 0.0),
                    under,
                );
                self.quad(
                    eave(-ex, 0.0),
                    eave(ex, 0.0),
                    eave(ex, t),
                    eave(-ex, t),
                    TIMBER,
                );
            } else {
                self.quad(eave(ex, t), eave(-ex, t), top(-ex, t), top(ex, t), roof);
                self.quad(
                    eave(ex, 0.0),
                    top(ex, 0.0),
                    top(-ex, 0.0),
                    eave(-ex, 0.0),
                    under,
                );
                self.quad(
                    eave(ex, 0.0),
                    eave(-ex, 0.0),
                    eave(-ex, t),
                    eave(ex, t),
                    TIMBER,
                );
            }
            // Verge boards at both gable ends.
            for x in [ex, -ex] {
                let o = if (x > 0.0) == (s > 0.0) { 1.0 } else { -1.0 };
                let (p, q) = (eave(x, 0.0), top(x, 0.0));
                let (pt, qt) = (eave(x, t), top(x, t));
                if o > 0.0 {
                    self.quad(p, q, qt, pt, TIMBER);
                } else {
                    self.quad(q, p, pt, qt, TIMBER);
                }
            }
        }
        // Rows of tiles across each slope, a little darker at each row's
        // lower edge.
        let rows = ((ridge - ey) / 0.42).round().max(3.0) as usize;
        let edge = tinted([0.78, 0.78, 0.78]);
        for s in [1.0f32, -1.0] {
            for k in 1..rows {
                let f = k as f32 / rows as f32;
                let (y, z) = (ey + (ridge - ey) * f, ez * s * (1.0 - f));
                let (y2, z2) = (
                    y + (ridge - ey) / rows as f32 * 0.22,
                    z - ez * s / rows as f32 * 0.22,
                );
                let lift = t + 0.04;
                let p = |x: f32, yy: f32, zz: f32| [x, yy + lift, zz];
                if s > 0.0 {
                    self.quad(
                        p(-ex, y, z),
                        p(ex, y, z),
                        p(ex, y2, z2),
                        p(-ex, y2, z2),
                        edge,
                    );
                } else {
                    self.quad(
                        p(ex, y, z),
                        p(-ex, y, z),
                        p(-ex, y2, z2),
                        p(ex, y2, z2),
                        edge,
                    );
                }
            }
        }
        // The ridge cap.
        self.cuboid(
            [-ex, ridge + t - 0.05, -0.16],
            [ex, ridge + t + 0.12, 0.16],
            scale(roof, 0.8),
        );
    }

    /// A chimney through the roof at (x, z), up to `top`.
    fn chimney(&mut self, x: f32, z: f32, from: f32, top: f32) {
        let w = 0.32;
        self.cuboid(
            [x - w, from, z - w],
            [x + w, top, z + w],
            [0.36, 0.22, 0.17],
        );
        self.cuboid(
            [x - w - 0.08, top, z - w - 0.08],
            [x + w + 0.08, top + 0.14, z + w + 0.08],
            STONE,
        );
    }
}

/// The farmhouse: one and a half storeys, 14 m long, plastered, with green
/// shutters, a door under a little roof, windows in the gables, a chimney
/// and a lean-to woodshed at one end.
pub fn farmhouse() -> Mesh {
    let mut m = Mesh::default();
    let (len, dep, wall, ridge) = (14.0, 8.4, 3.7, 7.6);
    let walls = [0.66, 0.62, 0.54];
    let green = [0.09, 0.19, 0.12];
    m.gabled(len, dep, wall, ridge, 0.55, walls, 0.45);
    let d = dep / 2.0;
    for side in [Side::Front, Side::Back] {
        for (k, u) in [-5.0f32, -2.6, 2.6, 5.0].into_iter().enumerate() {
            let lit = (k + if side == Side::Front { 0 } else { 1 }) % 3 != 0;
            m.window(side, u, 1.0, 1.25, 2.65, d, lit, Some(green));
        }
    }
    m.door(Side::Front, 0.0, 1.1, 2.3, d, TIMBER, true);
    // Small windows high in the gables, and one below in the east end.
    for side in [Side::East, Side::West] {
        m.window(
            side,
            0.0,
            0.7,
            4.3,
            5.2,
            len / 2.0,
            side == Side::East,
            None,
        );
    }
    m.window(
        Side::East,
        -1.8,
        0.9,
        1.3,
        2.6,
        len / 2.0,
        false,
        Some(green),
    );
    m.chimney(3.0, -1.6, 4.5, 8.6);
    // A lean-to woodshed at the west end.
    let x0 = -len / 2.0;
    m.cuboid([x0 - 3.0, -0.6, -3.0], [x0, 2.4, 2.6], TIMBER);
    let lean = tinted([1.0, 1.0, 1.0]);
    m.quad(
        [x0 - 3.3, 2.3, 2.9],
        [x0 + 0.1, 3.2, 2.9],
        [x0 + 0.1, 3.2, -3.3],
        [x0 - 3.3, 2.3, -3.3],
        lean,
    );
    // Logs stacked under it.
    m.cuboid(
        [x0 - 2.8, 0.0, 2.6],
        [x0 - 0.2, 1.3, 2.85],
        [0.32, 0.22, 0.13],
    );
    m
}

/// A two-storey house: 11 m long, ochre, a steep roof, two rows of
/// windows, a porch over the door and a chimney at each end.
pub fn townhouse() -> Mesh {
    let mut m = Mesh::default();
    let (len, dep, wall, ridge) = (11.0, 8.0, 6.3, 10.3);
    let walls = [0.7, 0.58, 0.4];
    let shutters = [0.3, 0.12, 0.08];
    m.gabled(len, dep, wall, ridge, 0.45, walls, 0.5);
    let d = dep / 2.0;
    // A band between the floors.
    m.cuboid(
        [-len / 2.0 - 0.04, 3.2, -d - 0.04],
        [len / 2.0 + 0.04, 3.4, d + 0.04],
        [0.55, 0.47, 0.35],
    );
    for side in [Side::Front, Side::Back] {
        for (row, (v0, v1)) in [(1.0f32, 2.5f32), (4.0, 5.5)].into_iter().enumerate() {
            for (k, u) in [-3.9f32, -1.5, 1.5, 3.9].into_iter().enumerate() {
                if side == Side::Front && row == 0 && (k == 1 || k == 2) {
                    continue;
                }
                let lit = (k + row * 2 + if side == Side::Front { 0 } else { 1 }) % 3 == 0;
                m.window(side, u, 0.95, v0, v1, d, lit, Some(shutters));
            }
        }
    }
    m.door(Side::Front, 0.0, 1.2, 2.4, d, [0.16, 0.09, 0.06], true);
    for side in [Side::East, Side::West] {
        m.window(side, 0.0, 0.8, 7.0, 8.1, len / 2.0, false, None);
        m.window(
            side,
            -1.6,
            0.9,
            4.0,
            5.4,
            len / 2.0,
            side == Side::West,
            Some(shutters),
        );
    }
    m.chimney(4.4, 0.0, 8.0, 11.4);
    m.chimney(-4.4, 0.0, 8.0, 11.4);
    m
}

/// A barn: 18 m long and tall, red boards on a stone footing, big double
/// doors braced in white, and a hayloft door in the gable.
pub fn barn() -> Mesh {
    let mut m = Mesh::default();
    let (len, dep, wall, ridge) = (18.0, 10.0, 5.0, 10.2);
    let red = [0.36, 0.1, 0.07];
    let dark = [0.24, 0.065, 0.045];
    let white = [0.62, 0.6, 0.55];
    m.gabled(len, dep, wall, ridge, 0.5, red, 0.6);
    let (hx, hz) = (len / 2.0, dep / 2.0);
    // Boards: darker joints, every 60 cm along the long sides.
    for side in [Side::Front, Side::Back] {
        let mut u = -hx + 0.3;
        while u < hx {
            m.panel(side, u - 0.03, u + 0.03, 0.6, wall, hz + 0.03, dark);
            u += 0.6;
        }
    }
    // Corner boards.
    for (x, z) in [(hx, hz), (hx, -hz), (-hx, hz), (-hx, -hz)] {
        m.cuboid([x - 0.12, 0.6, z - 0.12], [x + 0.12, wall, z + 0.12], white);
    }
    // The big doors on the front, each braced with a white X.
    let (dw, dh) = (4.2, 4.0);
    m.panel(
        Side::Front,
        -dw / 2.0 - 0.2,
        dw / 2.0 + 0.2,
        0.0,
        dh + 0.2,
        hz + 0.05,
        white,
    );
    for (a, b) in [(-dw / 2.0, 0.0), (0.0, dw / 2.0)] {
        m.panel(Side::Front, a + 0.05, b - 0.05, 0.05, dh, hz + 0.08, dark);
        let (p, q) = (a + 0.25, b - 0.25);
        let (v0, v1) = (0.3, dh - 0.3);
        let w = 0.13;
        // The diagonals, as thin slanted strips.
        for (s0, s1) in [(p, q), (q, p)] {
            m.quad(
                at(Side::Front, s0 - w, v0, hz + 0.11),
                at(Side::Front, s0 + w, v0, hz + 0.11),
                at(Side::Front, s1 + w, v1, hz + 0.11),
                at(Side::Front, s1 - w, v1, hz + 0.11),
                white,
            );
        }
        m.panel(
            Side::Front,
            a + 0.1,
            b - 0.1,
            dh - 0.3,
            dh - 0.12,
            hz + 0.11,
            white,
        );
        m.panel(Side::Front, a + 0.1, b - 0.1, 0.12, 0.3, hz + 0.11, white);
    }
    // Small windows along the sides.
    for side in [Side::Front, Side::Back] {
        for u in [-6.5f32, 6.5] {
            m.window(side, u, 0.8, 2.6, 3.4, hz, false, None);
        }
    }
    // The hayloft door high in each gable, and a door below.
    for side in [Side::East, Side::West] {
        m.panel(side, -1.0, 1.0, 5.6, 7.6, hx + 0.05, white);
        m.panel(side, -0.85, 0.85, 5.75, 7.45, hx + 0.08, dark);
        m.panel(side, -1.0, 1.0, 0.0, 2.6, hx + 0.05, white);
        m.panel(side, -0.85, 0.85, 0.05, 2.45, hx + 0.08, dark);
    }
    m
}

/// A shed: 5 m long, weathered boards and a roof sloping one way.
pub fn shed() -> Mesh {
    let mut m = Mesh::default();
    let (hx, hz) = (2.5, 1.8);
    let wood = [0.3, 0.22, 0.15];
    let dark = [0.2, 0.14, 0.09];
    m.cuboid(
        [-hx - 0.05, -0.5, -hz - 0.05],
        [hx + 0.05, 0.15, hz + 0.05],
        [0.25, 0.24, 0.22],
    );
    // Higher at the front, lower at the back.
    let (front, back) = (2.8, 2.2);
    m.cuboid([-hx, 0.15, -hz], [hx, back, hz], wood);
    m.quad(
        [-hx, back, hz],
        [hx, back, hz],
        [hx, front, hz],
        [-hx, front, hz],
        wood,
    );
    // The ends, rising from the back to the front.
    m.tri([hx, back, -hz], [hx, back, hz], [hx, front, hz], wood);
    m.tri([-hx, back, hz], [-hx, back, -hz], [-hx, front, hz], wood);
    let mut u = -hx + 0.25;
    while u < hx {
        m.panel(
            Side::Front,
            u - 0.025,
            u + 0.025,
            0.15,
            front,
            hz + 0.03,
            dark,
        );
        u += 0.5;
    }
    m.door(Side::Front, -0.9, 1.1, 2.0, hz, dark, false);
    m.window(Side::Front, 1.3, 0.7, 1.2, 1.9, hz, false, None);
    // The roof, overhanging, with thickness.
    let roof = tinted([1.0, 1.0, 1.0]);
    let (ex, ez) = (hx + 0.35, hz + 0.4);
    let slope = (front - back) / (2.0 * hz);
    let (yf, yb) = (front + 0.4 * slope, back - 0.4 * slope);
    let t = 0.12;
    m.quad(
        [-ex, yf + t, ez],
        [ex, yf + t, ez],
        [ex, yb + t, -ez],
        [-ex, yb + t, -ez],
        roof,
    );
    m.quad(
        [-ex, yf, ez],
        [-ex, yb, -ez],
        [ex, yb, -ez],
        [ex, yf, ez],
        tinted([0.42, 0.42, 0.42]),
    );
    m.quad(
        [-ex, yf, ez],
        [ex, yf, ez],
        [ex, yf + t, ez],
        [-ex, yf + t, ez],
        TIMBER,
    );
    m.quad(
        [ex, yb, -ez],
        [-ex, yb, -ez],
        [-ex, yb + t, -ez],
        [ex, yb + t, -ez],
        TIMBER,
    );
    m
}
