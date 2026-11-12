//! Software 3D ride view drawn with egui meshes: a perspective camera chases
//! the rider along the route; terrain, road, trees and markers are painted far
//! to near with distance fog.

use crate::route::{Route, Scenery};
use eframe::egui::{Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke, Vec2};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TimeOfDay {
    #[default]
    Day,
    Evening,
    Night,
}

impl TimeOfDay {
    pub const ALL: [TimeOfDay; 3] = [TimeOfDay::Day, TimeOfDay::Evening, TimeOfDay::Night];
    pub fn label(self) -> &'static str {
        match self {
            TimeOfDay::Day => "Day",
            TimeOfDay::Evening => "Sunset",
            TimeOfDay::Night => "Night",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Weather {
    #[default]
    Clear,
    Cloudy,
    Fog,
    Rain,
}

impl Weather {
    pub const ALL: [Weather; 4] = [Weather::Clear, Weather::Cloudy, Weather::Fog, Weather::Rain];
    pub fn label(self) -> &'static str {
        match self {
            Weather::Clear => "Clear",
            Weather::Cloudy => "Cloudy",
            Weather::Fog => "Fog",
            Weather::Rain => "Rain",
        }
    }
}

/// Time of day and weather for the 3D views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sky {
    pub time: TimeOfDay,
    pub weather: Weather,
}

/// Falling rain drawn over a view: short slanted streaks that move with time.
pub fn rain(painter: &Painter, rect: Rect, time: f64, night: bool) {
    let painter = painter.with_clip_rect(rect);
    let alpha = if night { 40 } else { 70 };
    let col = Color32::from_rgba_unmultiplied(200, 210, 225, alpha);
    let n = ((rect.width() * rect.height()) / 2600.0) as u64;
    for i in 0..n {
        let h = hash(i, 4242);
        let speed = 900.0 + (h % 500) as f64;
        let len = 14.0 + ((h >> 12) % 18) as f32;
        let x0 = (h >> 24) % 10_000;
        let y0 = (h >> 40) % 10_000;
        let y = ((y0 as f64 + time * speed) % (rect.height() as f64 + 60.0)) as f32 - 30.0;
        let x = (x0 as f32 / 10_000.0) * (rect.width() + 80.0) - 40.0 - y * 0.12;
        let a = Pos2::new(rect.left() + x, rect.top() + y);
        painter.line_segment([a, a + Vec2::new(len * 0.12, len)], Stroke::new(1.0, col));
    }
}

type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt().max(1e-9);
    [a[0] / l, a[1] / l, a[2] / l]
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}
fn shade(c: Color32, k: f32) -> Color32 {
    let m = |x: u8| (x as f32 * k).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(m(c.r()), m(c.g()), m(c.b()))
}

fn hash(a: u64, b: u64) -> u64 {
    let mut x = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^ (x >> 29)
}

struct Palette {
    sky_top: Color32,
    sky_horizon: Color32,
    haze: Color32,
    grass_a: Color32,
    grass_b: Color32,
    road: Color32,
    shoulder: Color32,
    mtn_far: Color32,
    mtn_near: Color32,
    mtn_top: Color32,
    tree_a: Color32,
    tree_b: Color32,
    trunk: Color32,
    water: Color32,
    rock: Color32,
}

fn palette(s: Scenery) -> Palette {
    let c = Color32::from_rgb;
    match s {
        Scenery::Hills => Palette {
            sky_top: c(66, 128, 214),
            sky_horizon: c(186, 216, 240),
            haze: c(178, 202, 210),
            grass_a: c(92, 158, 70),
            grass_b: c(84, 148, 64),
            road: c(78, 80, 86),
            shoulder: c(150, 140, 120),
            mtn_far: c(132, 156, 182),
            mtn_near: c(98, 134, 112),
            mtn_top: c(150, 172, 192),
            tree_a: c(46, 110, 52),
            tree_b: c(74, 136, 58),
            trunk: c(98, 70, 46),
            water: c(60, 120, 170),
            rock: c(128, 126, 118),
        },
        Scenery::Lake => Palette {
            sky_top: c(84, 146, 222),
            sky_horizon: c(204, 226, 244),
            haze: c(190, 212, 224),
            grass_a: c(110, 168, 76),
            grass_b: c(102, 160, 70),
            road: c(84, 86, 92),
            shoulder: c(190, 176, 140),
            mtn_far: c(140, 164, 192),
            mtn_near: c(110, 146, 126),
            mtn_top: c(166, 186, 206),
            tree_a: c(54, 120, 60),
            tree_b: c(90, 150, 66),
            trunk: c(104, 76, 50),
            water: c(52, 118, 176),
            rock: c(140, 136, 126),
        },
        Scenery::Coast => Palette {
            sky_top: c(60, 130, 220),
            sky_horizon: c(200, 226, 246),
            haze: c(190, 214, 230),
            grass_a: c(118, 160, 78),
            grass_b: c(110, 152, 72),
            road: c(84, 86, 92),
            shoulder: c(206, 190, 150),
            mtn_far: c(150, 176, 200),
            mtn_near: c(120, 150, 128),
            mtn_top: c(170, 190, 210),
            tree_a: c(52, 112, 64),
            tree_b: c(96, 140, 70),
            trunk: c(104, 76, 50),
            water: c(30, 104, 170),
            rock: c(150, 140, 124),
        },
        Scenery::Alpine => Palette {
            sky_top: c(44, 104, 196),
            sky_horizon: c(176, 206, 236),
            haze: c(170, 192, 212),
            grass_a: c(96, 148, 74),
            grass_b: c(88, 138, 68),
            road: c(72, 74, 80),
            shoulder: c(140, 136, 128),
            mtn_far: c(118, 136, 162),
            mtn_near: c(92, 112, 118),
            mtn_top: c(240, 244, 250),
            tree_a: c(30, 84, 52),
            tree_b: c(44, 104, 62),
            trunk: c(86, 62, 42),
            water: c(60, 120, 170),
            rock: c(124, 122, 120),
        },
    }
}

/// How the time of day and weather change the simple view.
struct Atmos {
    sun: Option<(Color32, f64)>,
    moon: bool,
    stars: bool,
    clouds: (usize, Color32),
    /// Multiplies distances for fog (bigger: thicker).
    fog: f64,
}

fn tint(c: Color32, k: [f32; 3]) -> Color32 {
    let m = |x: u8, k: f32| (x as f32 * k).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(m(c.r(), k[0]), m(c.g(), k[1]), m(c.b(), k[2]))
}

fn grey(c: Color32, k: f32) -> Color32 {
    let l = (0.3 * c.r() as f32 + 0.6 * c.g() as f32 + 0.1 * c.b() as f32) * k;
    let l = l.round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(l, l, l)
}

fn atmosphere(p: &mut Palette, sky: Sky) -> Atmos {
    let c = Color32::from_rgb;
    let ground = |p: &mut Palette, k: [f32; 3]| {
        for col in [
            &mut p.grass_a,
            &mut p.grass_b,
            &mut p.road,
            &mut p.shoulder,
            &mut p.mtn_far,
            &mut p.mtn_near,
            &mut p.mtn_top,
            &mut p.tree_a,
            &mut p.tree_b,
            &mut p.trunk,
            &mut p.water,
            &mut p.rock,
        ] {
            *col = tint(*col, k);
        }
    };
    let mut a = Atmos {
        sun: Some((Color32::from_rgb(255, 248, 220), 0.34)),
        moon: false,
        stars: false,
        clouds: (9, Color32::from_rgba_unmultiplied(255, 255, 255, 150)),
        fog: 1.0,
    };
    match sky.time {
        TimeOfDay::Day => {}
        TimeOfDay::Evening => {
            p.sky_top = c(64, 84, 150);
            p.sky_horizon = c(250, 168, 108);
            p.haze = c(214, 160, 128);
            ground(p, [0.95, 0.76, 0.62]);
            a.sun = Some((c(255, 196, 120), 0.05));
            a.clouds.1 = Color32::from_rgba_unmultiplied(255, 190, 160, 150);
        }
        TimeOfDay::Night => {
            p.sky_top = c(6, 10, 26);
            p.sky_horizon = c(24, 34, 60);
            p.haze = c(22, 30, 50);
            ground(p, [0.2, 0.24, 0.34]);
            a.sun = None;
            a.moon = true;
            a.stars = true;
            a.clouds.1 = Color32::from_rgba_unmultiplied(70, 80, 100, 110);
        }
    }
    match sky.weather {
        Weather::Clear => {}
        Weather::Cloudy | Weather::Rain => {
            let dark = if sky.weather == Weather::Rain {
                0.7
            } else {
                0.9
            };
            p.sky_top = lerp(p.sky_top, grey(p.sky_horizon, 0.8 * dark), 0.75);
            p.sky_horizon = lerp(p.sky_horizon, grey(p.sky_horizon, 0.9 * dark), 0.6);
            p.haze = lerp(p.haze, grey(p.haze, 0.9 * dark), 0.6);
            ground(p, [0.82 * dark, 0.84 * dark, 0.88 * dark]);
            a.sun = None;
            a.moon = false;
            a.stars = false;
            a.clouds = (22, grey(p.sky_horizon, 0.9).gamma_multiply(0.85));
            a.fog = if sky.weather == Weather::Rain {
                2.0
            } else {
                1.3
            };
        }
        Weather::Fog => {
            let g = grey(p.haze, 1.0);
            p.sky_top = lerp(p.sky_top, g, 0.85);
            p.sky_horizon = g;
            p.haze = g;
            ground(p, [0.85, 0.86, 0.88]);
            a.sun = None;
            a.moon = false;
            a.stars = false;
            a.clouds.0 = 0;
            a.fog = 4.5;
        }
    }
    a
}

struct Cam {
    pos: V3,
    right: V3,
    up: V3,
    fwd: V3,
    f: f64,
    cx: f64,
    cy: f64,
}

const NEAR: f64 = 0.4;

impl Cam {
    fn to_cam(&self, p: V3) -> V3 {
        let r = sub(p, self.pos);
        [dot(r, self.right), dot(r, self.up), dot(r, self.fwd)]
    }
    fn proj(&self, c: V3) -> Pos2 {
        Pos2::new(
            (self.cx + self.f * c[0] / c[2]) as f32,
            (self.cy - self.f * c[1] / c[2]) as f32,
        )
    }
}

/// Collects triangles in painter order; text shapes interrupt the mesh.
struct Canvas<'a> {
    painter: &'a Painter,
    mesh: Mesh,
}

impl Canvas<'_> {
    fn flush(&mut self) {
        if !self.mesh.is_empty() {
            self.painter
                .add(Shape::mesh(std::mem::take(&mut self.mesh)));
        }
    }
    fn poly(&mut self, pts: &[Pos2], color: Color32) {
        if pts.len() < 3 {
            return;
        }
        let base = self.mesh.vertices.len() as u32;
        for &p in pts {
            self.mesh.colored_vertex(p, color);
        }
        for i in 1..pts.len() as u32 - 1 {
            self.mesh.add_triangle(base, base + i, base + i + 1);
        }
    }
    fn tri(&mut self, a: Pos2, b: Pos2, c: Pos2, color: Color32) {
        self.poly(&[a, b, c], color);
    }
    fn ellipse(&mut self, c: Pos2, rx: f32, ry: f32, color: Color32) {
        let n = 18;
        let pts: Vec<Pos2> = (0..n)
            .map(|i| {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                Pos2::new(c.x + rx * a.cos(), c.y + ry * a.sin())
            })
            .collect();
        self.poly(&pts, color);
    }
    /// A thick line as a quad.
    fn segment(&mut self, a: Pos2, b: Pos2, width: f32, color: Color32) {
        let d = b - a;
        let len = d.length().max(1e-3);
        let n = Vec2::new(-d.y, d.x) / len * (width / 2.0);
        self.poly(&[a + n, b + n, b - n, a - n], color);
        self.ellipse(a, width / 2.0, width / 2.0, color);
        self.ellipse(b, width / 2.0, width / 2.0, color);
    }
    /// Clip a camera-space polygon against the near plane and draw it.
    fn world_poly(&mut self, cam: &Cam, pts: &[V3], color: Color32) {
        let mut out: Vec<V3> = Vec::with_capacity(pts.len() + 2);
        for i in 0..pts.len() {
            let a = pts[i];
            let b = pts[(i + 1) % pts.len()];
            let ain = a[2] >= NEAR;
            let bin = b[2] >= NEAR;
            if ain {
                out.push(a);
            }
            if ain != bin {
                let t = (NEAR - a[2]) / (b[2] - a[2]);
                out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, NEAR]);
            }
        }
        if out.len() >= 3 {
            let screen: Vec<Pos2> = out.iter().map(|&c| cam.proj(c)).collect();
            self.poly(&screen, color);
        }
    }
}

/// A cyclist in the world.
#[derive(Debug, Clone)]
pub struct Figure {
    /// Position along the route, in metres.
    pub d: f64,
    /// Metres right of the road's centre line.
    pub lateral: f64,
    pub crank: f32,
    /// Metres per second, for leaning into bends.
    pub speed: f64,
    pub jersey: Color32,
    /// Name tag shown above the rider (companions only).
    pub tag: Option<String>,
}

/// What to draw: the route and the riders on it. The camera follows the
/// first rider.
pub struct Scene<'a> {
    pub route: &'a Route,
    pub riders: Vec<Figure>,
    pub sky: Sky,
    pub dt: f64,
}

/// Your lane, and the one a companion rides in (to your left).
pub const LANE: f64 = 1.4;
pub const COMPANION_LANE: f64 = 0.3;

#[derive(Default)]
pub struct World {
    yaw: f64,
    pitch: f64,
    initialised: bool,
    time: f64,
}

const FAR: f64 = 560.0;
const ROAD_HALF: f64 = 3.2;
const LATERALS: [f64; 16] = [
    -170.0, -95.0, -52.0, -27.0, -14.0, -7.5, -4.3, -ROAD_HALF, ROAD_HALF, 4.3, 7.5, 14.0, 27.0,
    52.0, 95.0, 170.0,
];

fn wrap_angle(a: f64) -> f64 {
    (a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

impl World {
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, painter: &Painter, rect: Rect, scene: &Scene) {
        let (route, dt) = (scene.route, scene.dt);
        let d = scene.riders.first().map(|r| r.d).unwrap_or(0.0);
        self.time += dt;
        let mut pal = palette(route.scenery);
        let atm = atmosphere(&mut pal, scene.sky);
        let painter = painter.with_clip_rect(rect);

        // --- camera -------------------------------------------------------
        let cam_d = d - 7.0;
        let lane_pt = |s: f64, lat: f64| {
            let p = route.at(s);
            [
                p.x + lat * p.heading.cos(),
                p.ele,
                p.z - lat * p.heading.sin(),
            ]
        };
        let mut cpos = lane_pt(cam_d, LANE);
        cpos[1] = route.at(cam_d).ele.max(route.at(d).ele - 1.0) + 2.5;
        let mut target = lane_pt(d + 16.0, LANE);
        target[1] += 0.8;
        let dir = sub(target, cpos);
        let yaw_t = dir[0].atan2(dir[2]);
        let pitch_t = dir[1]
            .atan2((dir[0] * dir[0] + dir[2] * dir[2]).sqrt())
            .clamp(-0.5, 0.5);
        if !self.initialised || wrap_angle(yaw_t - self.yaw).abs() > 1.2 {
            self.yaw = yaw_t;
            self.pitch = pitch_t;
            self.initialised = true;
        } else {
            let k = 1.0 - (-dt * 5.0).exp();
            self.yaw = wrap_angle(self.yaw + wrap_angle(yaw_t - self.yaw) * k);
            self.pitch += (pitch_t - self.pitch) * k;
        }
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let fwd = [sy * cp, sp, cy * cp];
        let right = [cy, 0.0, -sy];
        let up = cross(fwd, right);
        let f = rect.height() as f64 / 2.0 / (55f64.to_radians() / 2.0).tan();
        let cam = Cam {
            pos: cpos,
            right,
            up,
            fwd,
            f,
            cx: rect.center().x as f64,
            cy: rect.center().y as f64,
        };
        let horizon = (cam.cy + f * self.pitch.tan()) as f32;
        let light = norm([0.45, 0.8, 0.35]);
        let light_c = [dot(light, right), dot(light, up), dot(light, fwd)];

        let mut cv = Canvas {
            painter: &painter,
            mesh: Mesh::default(),
        };

        // --- sky ----------------------------------------------------------
        cv.poly(
            &[
                rect.left_top(),
                rect.right_top(),
                rect.right_bottom(),
                rect.left_bottom(),
            ],
            pal.haze,
        );
        let hz = horizon.clamp(rect.top(), rect.bottom());
        {
            let base = cv.mesh.vertices.len() as u32;
            cv.mesh.colored_vertex(rect.left_top(), pal.sky_top);
            cv.mesh.colored_vertex(rect.right_top(), pal.sky_top);
            cv.mesh
                .colored_vertex(Pos2::new(rect.right(), hz), pal.sky_horizon);
            cv.mesh
                .colored_vertex(Pos2::new(rect.left(), hz), pal.sky_horizon);
            cv.mesh.add_triangle(base, base + 1, base + 2);
            cv.mesh.add_triangle(base, base + 2, base + 3);
        }
        // Stars, then the sun or moon.
        if atm.stars {
            for i in 0..260u64 {
                let h = hash(i, 777);
                let ang = (h % 6283) as f64 / 1000.0;
                let elev = 0.04 + ((h >> 16) % 1000) as f64 / 1000.0 * 1.2;
                let dir = [ang.sin() * elev.cos(), elev.sin(), ang.cos() * elev.cos()];
                let c = [dot(dir, right), dot(dir, up), dot(dir, fwd)];
                if c[2] < 0.1 {
                    continue;
                }
                let p = cam.proj(c);
                let a = 90 + ((h >> 32) % 160) as u8;
                let r = if (h >> 50).is_multiple_of(9) {
                    1.4
                } else {
                    0.9
                };
                cv.ellipse(p, r, r, Color32::from_rgba_unmultiplied(235, 240, 255, a));
            }
        }
        let disc = if atm.moon {
            Some((Color32::from_rgb(236, 240, 250), 0.5, 0.022))
        } else {
            atm.sun.map(|(c, elev)| (c, elev, 0.035))
        };
        if let Some((col, elev, size)) = disc {
            let sun_dir = [
                2.2f64.sin() * elev.cos(),
                elev.sin(),
                2.2f64.cos() * elev.cos(),
            ];
            let sc = [dot(sun_dir, right), dot(sun_dir, up), dot(sun_dir, fwd)];
            if sc[2] > 0.05 {
                let p = cam.proj(sc);
                let r = (f * size) as f32;
                for (k, a) in [(3.2, 28u8), (2.0, 50), (1.0, 255)] {
                    let c = Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), a);
                    cv.ellipse(p, r * k, r * k, c);
                }
            }
        }
        // Clouds drift slowly across a fixed sky dome.
        for i in 0..atm.clouds.0 as u64 {
            let h = hash(i, route.seed + 100);
            let ang = (h % 6283) as f64 / 1000.0 + self.time * 0.002;
            let elev = 0.10 + ((h >> 16) % 100) as f64 / 100.0 * 0.22;
            let dir = [ang.sin() * elev.cos(), elev.sin(), ang.cos() * elev.cos()];
            let c = [dot(dir, right), dot(dir, up), dot(dir, fwd)];
            if c[2] < 0.1 {
                continue;
            }
            let p = cam.proj(c);
            let size = (f * (0.05 + ((h >> 32) % 40) as f64 / 1000.0)) as f32;
            let col = atm.clouds.1;
            for j in 0..5u64 {
                let hj = hash(h, j);
                let ox = ((hj % 200) as f32 / 100.0 - 1.0) * size * 1.6;
                let oy = ((hj >> 8) % 100) as f32 / 100.0 * size * 0.35;
                let r = size * (0.45 + ((hj >> 16) % 50) as f32 / 100.0);
                cv.ellipse(Pos2::new(p.x + ox, p.y - oy), r, r * 0.6, col);
            }
        }

        // --- distant mountains (a ring at infinity) -----------------------
        let s = route.seed as f64;
        let alp = match route.scenery {
            Scenery::Alpine => 2.2,
            Scenery::Coast => 0.6,
            _ => 1.0,
        };
        type Layer = (Box<dyn Fn(f64) -> f64>, Color32, Color32);
        let layers: [Layer; 2] = [
            (
                Box::new(move |a: f64| {
                    alp * (0.045
                        + 0.024 * (3.0 * a + s).sin()
                        + 0.016 * (7.0 * a + 2.0 * s).sin()
                        + 0.009 * (17.0 * a + 3.0 * s).sin()
                        + 0.004 * (41.0 * a).sin())
                }),
                lerp(pal.mtn_far, pal.haze, 0.35),
                lerp(pal.mtn_top, pal.haze, 0.25),
            ),
            (
                Box::new(move |a: f64| {
                    0.6 * alp.sqrt()
                        * (0.022
                            + 0.014 * (5.0 * a + 1.7 * s).sin()
                            + 0.009 * (11.0 * a + s).sin()
                            + 0.004 * (23.0 * a).sin())
                }),
                lerp(pal.mtn_near, pal.haze, 0.2),
                lerp(pal.mtn_near, pal.mtn_top, 0.25),
            ),
        ];
        for (height, base_col, top_col) in &layers {
            let step = 6.0;
            let mut x = rect.left();
            while x < rect.right() + step {
                let x2 = x + step;
                let ang = |px: f32| self.yaw + ((px as f64 - cam.cx) / f).atan();
                let y1 = horizon - (f * height(ang(x)).max(0.003).tan()) as f32;
                let y2 = horizon - (f * height(ang(x2)).max(0.003).tan()) as f32;
                let base = cv.mesh.vertices.len() as u32;
                cv.mesh.colored_vertex(Pos2::new(x, y1), *top_col);
                cv.mesh.colored_vertex(Pos2::new(x2, y2), *top_col);
                cv.mesh
                    .colored_vertex(Pos2::new(x2, horizon + 2.0), *base_col);
                cv.mesh
                    .colored_vertex(Pos2::new(x, horizon + 2.0), *base_col);
                cv.mesh.add_triangle(base, base + 1, base + 2);
                cv.mesh.add_triangle(base, base + 2, base + 3);
                x = x2;
            }
        }

        // --- terrain, road and objects, far to near ----------------------
        let mut slices = Vec::new();
        let mut sd = (cam_d / 5.0).floor() * 5.0;
        while sd < d + FAR {
            slices.push(sd);
            sd += if sd < d + 150.0 { 5.0 } else { 10.0 };
        }
        let fog = |dist: f64| {
            let dist = dist * atm.fog;
            (((dist - 30.0) / (FAR - 60.0)).clamp(0.0, 1.0).powf(1.1)) as f32
        };
        let point = |s: f64, lat: f64, y: f64| {
            let p = route.at(s);
            cam.to_cam([p.x + lat * p.heading.cos(), y, p.z - lat * p.heading.sin()])
        };
        let ground_y = |s: f64, lat: f64| {
            if lat.abs() <= 4.3 {
                route.at(s).ele + 0.02
            } else {
                route.terrain(s, lat)
            }
        };
        let rows: Vec<Vec<V3>> = slices
            .iter()
            .map(|&s| {
                LATERALS
                    .iter()
                    .map(|&l| point(s, l, ground_y(s, l)))
                    .collect()
            })
            .collect();
        let mid = LATERALS.len() / 2; // index of first right-side lateral
        let band_order: Vec<usize> = (0..mid - 1)
            .chain((mid..LATERALS.len() - 1).rev())
            .chain([mid - 1])
            .collect();

        for i in (0..slices.len() - 1).rev() {
            let (s_near, s_far) = (slices[i], slices[i + 1]);
            let (near, far) = (&rows[i], &rows[i + 1]);
            if near.iter().chain(far.iter()).all(|c| c[2] < NEAR) {
                continue;
            }
            let stripe = ((s_near / 20.0).floor() as i64).rem_euclid(2) == 0;
            for &j in &band_order {
                let quad = [far[j], far[j + 1], near[j + 1], near[j]];
                let lc = (LATERALS[j] + LATERALS[j + 1]) / 2.0;
                let mut col = if lc.abs() < ROAD_HALF {
                    pal.road
                } else if lc.abs() < 4.3 {
                    pal.shoulder
                } else if route.scenery.has_water() && lc < -14.0 {
                    let ripple = ((s_near * 0.11 + self.time * 0.6).sin() * 0.04 + 1.0) as f32;
                    shade(pal.water, ripple)
                } else if stripe {
                    pal.grass_a
                } else {
                    pal.grass_b
                };
                if route.scenery == Scenery::Alpine && lc.abs() > 60.0 {
                    col = lerp(col, pal.rock, 0.45);
                }
                // Lambert shading from the quad normal (camera space).
                let n = norm(cross(sub(quad[1], quad[0]), sub(quad[3], quad[0])));
                let n = if n[1] < 0.0 { [-n[0], -n[1], -n[2]] } else { n };
                let lambert = dot(n, light_c).max(0.0) as f32;
                let col = shade(col, 0.72 + 0.38 * lambert);
                let dist = (quad[0][2] + quad[2][2]) / 2.0;
                cv.world_poly(&cam, &quad, lerp(col, pal.haze, fog(dist)));
            }

            // Road markings within the slice (clipped to it so nearer road
            // slices never paint over them).
            if s_near < d + 220.0 {
                let ele = |s: f64| route.at(s).ele + 0.04;
                let mut mark = |s0: f64, s1: f64, l0: f64, l1: f64, col: Color32| {
                    let (a, b) = (s0.max(s_near), s1.min(s_far));
                    if b <= a {
                        return;
                    }
                    let q = [
                        point(b, l0, ele(b)),
                        point(b, l1, ele(b)),
                        point(a, l1, ele(a)),
                        point(a, l0, ele(a)),
                    ];
                    let dist = (q[0][2] + q[2][2]) / 2.0;
                    cv.world_poly(&cam, &q, lerp(col, pal.haze, fog(dist)));
                };
                let white = Color32::from_rgb(236, 236, 230);
                mark(s_near, s_far, -3.05, -2.9, white);
                mark(s_near, s_far, 2.9, 3.05, white);
                let k0 = (s_near / 10.0).floor() as i64;
                for k in k0..=k0 + 1 {
                    let s0 = k as f64 * 10.0;
                    mark(s0, s0 + 4.0, -0.07, 0.07, Color32::from_rgb(240, 210, 80));
                }
            }

            // Objects whose position falls in this slice, far first.
            let mut objs: Vec<(f64, f64, u64)> = Vec::new(); // (s, lateral, hash)
            let slot = 11.0;
            let k_lo = (s_near / slot).ceil() as i64;
            let k_hi = ((s_far - 1e-6) / slot).floor() as i64;
            for k in (k_lo..=k_hi).rev() {
                for side in [-1i64, 1] {
                    let h = hash(k as u64 ^ (route.seed << 40), side as u64 + 7);
                    let density = match route.scenery {
                        Scenery::Alpine => 62,
                        Scenery::Hills => 48,
                        Scenery::Lake => 40,
                        Scenery::Coast => 34,
                    };
                    if (h % 100) as u32 >= density {
                        continue;
                    }
                    let lat = side as f64
                        * (10.0 + ((h >> 8) % 70) as f64 + ((h >> 20) % 10) as f64 * 0.3);
                    if route.scenery.has_water() && lat < -12.0 {
                        continue;
                    }
                    objs.push((k as f64 * slot + ((h >> 28) % 8) as f64, lat, h));
                }
            }
            objs.sort_by(|a, b| {
                b.0.partial_cmp(&a.0)
                    .unwrap()
                    .then(b.1.abs().partial_cmp(&a.1.abs()).unwrap())
            });
            for &(os, lat, h) in &objs {
                let base = point(os, lat, route.terrain(os, lat) - 0.2);
                if base[2] < NEAR + 0.5 {
                    continue;
                }
                let pine = match route.scenery {
                    Scenery::Alpine => true,
                    Scenery::Hills => h % 5 < 2,
                    Scenery::Lake | Scenery::Coast => h % 4 == 0,
                };
                let height = 6.0 + ((h >> 36) % 90) as f64 / 10.0;
                self.tree(&mut cv, &cam, base, height, pine, h, &pal, fog(base[2]));
            }

            // Riders in this slice, far first.
            let mut here: Vec<&Figure> = scene
                .riders
                .iter()
                .filter(|r| r.d >= s_near && r.d < s_far)
                .collect();
            here.sort_by(|a, b| b.d.partial_cmp(&a.d).unwrap());
            for r in here {
                let c = point(r.d, r.lateral, route.at(r.d).ele + 0.02);
                if c[2] > NEAR {
                    let g = cam.proj(c);
                    let s = (f / c[2]) as f32;
                    draw_rider(&mut cv, g, s, r.crank, r.jersey);
                    if let Some(tag) = &r.tag {
                        cv.flush();
                        name_tag(cv.painter, Pos2::new(g.x, g.y - 1.75 * s), tag, c[2]);
                    }
                }
            }

            // Reflector posts and kilometre banners.
            for k in ((s_near / 50.0).ceil() as i64)..=((s_far - 1e-6) / 50.0).floor() as i64 {
                let ps = k as f64 * 50.0;
                for side in [-1.0, 1.0] {
                    let b = point(ps, side * 4.7, route.at(ps).ele);
                    let t = point(ps, side * 4.7, route.at(ps).ele + 1.0);
                    if b[2] > NEAR && t[2] > NEAR {
                        let (pb, pt) = (cam.proj(b), cam.proj(t));
                        let w = (f * 0.12 / b[2]) as f32;
                        let fg = fog(b[2]);
                        cv.segment(pb, pt, w.max(0.5), lerp(Color32::WHITE, pal.haze, fg));
                        let r = pt + (pb - pt) * 0.18;
                        cv.segment(
                            pt,
                            r,
                            w.max(0.5),
                            lerp(Color32::from_rgb(220, 40, 30), pal.haze, fg),
                        );
                    }
                }
            }
            let km_lo = (s_near / 1000.0).ceil() as i64;
            let km_hi = ((s_far - 1e-6) / 1000.0).floor() as i64;
            for km in km_lo..=km_hi {
                let ks = km as f64 * 1000.0;
                let lap_pos = ks.rem_euclid(route.length);
                let label = if lap_pos < 1.0 || route.length - lap_pos < 1.0 {
                    "START · FINISH".to_string()
                } else {
                    format!("{:.0} KM", lap_pos / 1000.0)
                };
                self.banner(
                    &mut cv,
                    &cam,
                    &point,
                    route.at(ks).ele,
                    ks,
                    &label,
                    pal.haze,
                    fog,
                );
            }
        }

        cv.flush();
        if scene.sky.weather == Weather::Rain {
            rain(
                &painter,
                rect,
                self.time,
                scene.sky.time == TimeOfDay::Night,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn tree(
        &self,
        cv: &mut Canvas,
        cam: &Cam,
        base: V3,
        height: f64,
        pine: bool,
        h: u64,
        pal: &Palette,
        fog: f32,
    ) {
        // World "up" expressed in camera space.
        let top = [
            base[0],
            base[1] + height * cam.up[1],
            base[2] + height * cam.fwd[1],
        ];
        let pb = cam.proj(base);
        let pt = cam.proj(top);
        let hpx = pb.y - pt.y;
        if hpx < 1.5 || pb.x < -hpx || pb.x > cam.cx as f32 * 2.0 + hpx {
            return;
        }
        let at = |u: f32| Pos2::new(pb.x + (pt.x - pb.x) * u, pb.y - hpx * u);
        let fogc = |c: Color32| lerp(c, pal.haze, fog);
        let tint = 0.9 + ((h >> 44) % 20) as f32 / 100.0;
        let trunk_w = (hpx * 0.07).max(1.0);
        if pine {
            cv.segment(at(0.0), at(0.25), trunk_w, fogc(pal.trunk));
            for (lo, hi, half, k) in [
                (0.15, 0.62, 0.36, 0.85),
                (0.38, 0.84, 0.28, 0.95),
                (0.6, 1.0, 0.18, 1.05),
            ] {
                let a = at(lo);
                let b = at(hi);
                let w = hpx * half;
                cv.tri(
                    Pos2::new(a.x - w, a.y),
                    Pos2::new(a.x + w, a.y),
                    b,
                    fogc(shade(pal.tree_a, k * tint)),
                );
            }
        } else {
            cv.segment(at(0.0), at(0.5), trunk_w * 1.2, fogc(pal.trunk));
            let c = at(0.66);
            let r = hpx * 0.3;
            cv.ellipse(c, r * 1.05, r, fogc(shade(pal.tree_b, 0.85 * tint)));
            cv.ellipse(
                Pos2::new(c.x - r * 0.25, c.y - r * 0.25),
                r * 0.6,
                r * 0.55,
                fogc(shade(pal.tree_b, 1.05 * tint)),
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn banner(
        &self,
        cv: &mut Canvas,
        cam: &Cam,
        point: &dyn Fn(f64, f64, f64) -> V3,
        ele: f64,
        s: f64,
        label: &str,
        haze: Color32,
        fog: impl Fn(f64) -> f32,
    ) {
        let (l, r) = (point(s, -5.0, ele), point(s, 5.0, ele));
        if l[2] < NEAR + 1.0 || r[2] < NEAR + 1.0 {
            return;
        }
        let up = |p: V3, y: f64| {
            let mut q = p;
            q[1] += y * cam.up[1];
            q[2] += y * cam.fwd[1];
            q
        };
        let fg = fog((l[2] + r[2]) / 2.0);
        let post = lerp(Color32::from_rgb(40, 40, 46), haze, fg);
        let w = (cam.f * 0.25 / l[2]) as f32;
        let (l0, l1, r0, r1) = (
            cam.proj(l),
            cam.proj(up(l, 5.2)),
            cam.proj(r),
            cam.proj(up(r, 5.2)),
        );
        cv.segment(l0, l1, w.max(1.0), post);
        cv.segment(r0, r1, w.max(1.0), post);
        let (bl0, bl1, br0, br1) = (
            cam.proj(up(l, 3.9)),
            cam.proj(up(l, 5.0)),
            cam.proj(up(r, 3.9)),
            cam.proj(up(r, 5.0)),
        );
        let col = lerp(Color32::from_rgb(220, 60, 40), haze, fg);
        cv.poly(&[bl1, br1, br0, bl0], col);
        let size = (bl0.y - bl1.y) * 0.7;
        if size > 5.0 {
            cv.flush();
            let centre = Pos2::new((bl0.x + br1.x) / 2.0, (bl0.y + br1.y) / 2.0);
            cv.painter.text(
                centre,
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(size.min(120.0)),
                lerp(Color32::WHITE, col, fg),
            );
        }
    }
}

/// A small rounded label over a companion's head; fades with distance.
pub fn name_tag(painter: &Painter, at: Pos2, text: &str, dist: f64) {
    let fade = (1.0 - (dist as f32 - 60.0) / 120.0).clamp(0.0, 1.0);
    if fade <= 0.0 {
        return;
    }
    let font = FontId::proportional(12.0);
    let galley = painter.layout_no_wrap(
        text.to_string(),
        font,
        Color32::from_white_alpha((240.0 * fade) as u8),
    );
    let r = Rect::from_center_size(at, galley.size() + Vec2::new(14.0, 6.0));
    painter.rect_filled(
        r,
        eframe::egui::CornerRadius::same(9),
        Color32::from_black_alpha((150.0 * fade) as u8),
    );
    painter.galley(r.center() - galley.size() / 2.0, galley, Color32::WHITE);
}

/// Rear view of a cyclist standing on the ground point `g`, `s` px per metre.
fn draw_rider(cv: &mut Canvas, g: Pos2, s: f32, crank: f32, jersey: Color32) {
    let p = |x: f32, y: f32| Pos2::new(g.x + x * s, g.y - y * s);
    let sway = crank.sin() * 0.012;
    let dark = Color32::from_rgb(28, 28, 32);
    let skin = Color32::from_rgb(222, 178, 142);
    let shorts = Color32::from_rgb(24, 24, 30);

    cv.ellipse(
        p(0.0, 0.0),
        0.42 * s,
        0.07 * s,
        Color32::from_rgba_unmultiplied(0, 0, 0, 70),
    );
    // Rear wheel and tyre.
    cv.ellipse(p(0.0, 0.35), 0.035 * s, 0.35 * s, dark);
    cv.ellipse(
        p(0.0, 0.35),
        0.012 * s,
        0.05 * s,
        Color32::from_rgb(150, 150, 156),
    );
    // Handlebars peeking out either side.
    cv.segment(p(-0.24, 1.0), p(0.24, 1.0), 0.03 * s, dark);
    // Legs: feet follow the cranks.
    let bb = 0.3;
    let hip_y = 0.93;
    for (side, phase) in [(-1.0f32, 0.0f32), (1.0, std::f32::consts::PI)] {
        let foot_y = bb + 0.17 * (crank + phase).cos();
        let hip = p(side * 0.1 + sway, hip_y);
        let knee_y = foot_y + (hip_y - foot_y) * 0.58 + 0.06;
        let knee = p(side * 0.14 + sway * 0.5, knee_y);
        let ankle = p(side * 0.115, foot_y + 0.06);
        let foot = p(side * 0.115, foot_y);
        cv.segment(hip, knee, 0.15 * s, shorts);
        cv.segment(knee, ankle, 0.1 * s, skin);
        cv.segment(
            ankle,
            p(side * 0.115, foot_y + 0.12),
            0.095 * s,
            Color32::WHITE,
        );
        cv.segment(ankle, foot, 0.09 * s, dark);
    }
    // Saddle.
    cv.segment(p(-0.05, 0.92), p(0.05, 0.92), 0.05 * s, dark);
    // Torso (leaning forward, foreshortened from behind).
    let bob = sway * 0.5;
    cv.poly(
        &[
            p(-0.17 + bob, 0.94),
            p(0.17 + bob, 0.94),
            p(0.22 + bob, 1.28),
            p(-0.22 + bob, 1.28),
        ],
        jersey,
    );
    cv.poly(
        &[
            p(-0.18 + bob, 1.06),
            p(0.18 + bob, 1.06),
            p(0.19 + bob, 1.11),
            p(-0.19 + bob, 1.11),
        ],
        shade(jersey, 0.65),
    );
    cv.poly(
        &[
            p(-0.17 + bob, 0.94),
            p(0.17 + bob, 0.94),
            p(0.17 + bob, 0.99),
            p(-0.17 + bob, 0.99),
        ],
        shorts,
    );
    // Arms reaching to the bars.
    for side in [-1.0f32, 1.0] {
        cv.segment(
            p(side * 0.2 + bob, 1.25),
            p(side * 0.23, 1.03),
            0.085 * s,
            jersey,
        );
        cv.segment(p(side * 0.225, 1.1), p(side * 0.23, 1.0), 0.07 * s, skin);
    }
    // Head and helmet.
    cv.ellipse(p(bob, 1.37), 0.075 * s, 0.07 * s, skin);
    cv.ellipse(
        p(bob, 1.43),
        0.12 * s,
        0.1 * s,
        Color32::from_rgb(240, 240, 244),
    );
    cv.segment(
        p(-0.05 + bob, 1.47),
        p(0.05 + bob, 1.47),
        0.02 * s,
        shade(jersey, 0.8),
    );
}
