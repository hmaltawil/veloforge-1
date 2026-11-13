//! GPU ride view: OpenGL rendering of terrain, road, trees, grass, water,
//! sky and a 3D rider inside an egui paint callback.
//!
//! The scene is drawn into an off-screen HDR target with sun shadows from
//! two cascaded shadow maps, then finished in a post pass (bloom, light
//! shafts, tonemapping, grading) that also frosts the glass behind the HUD.
//! Falls back to the software view in `world.rs` if anything goes wrong
//! (see [`GpuWorld::error`]).

mod math;
mod mesh;
mod shaders;
mod terrain;

use crate::route::{Route, Scenery};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect};
use eframe::egui_glow;
use eframe::glow::{self, HasContext};
use math::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use terrain::{TerrainData, Worker, INSTANCE_STRIDE, KINDS, TERRAIN_STRIDE};

use crate::world::{Scene, Sky, TimeOfDay, Weather, LANE};

/// How much the 3D view asks of the graphics card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Quality {
    /// No shadows, grass or anti-aliasing, and the world drawn at a lower
    /// resolution and scaled up.
    Low,
    Medium,
    #[default]
    High,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::High, Quality::Medium, Quality::Low];
    pub fn label(self) -> &'static str {
        match self {
            Quality::High => "High",
            Quality::Medium => "Medium",
            Quality::Low => "Low",
        }
    }
    fn msaa(self) -> i32 {
        match self {
            Quality::High => 4,
            Quality::Medium => 2,
            Quality::Low => 0,
        }
    }
    fn shadow_res(self) -> i32 {
        match self {
            Quality::High => 2048,
            Quality::Medium => 1024,
            Quality::Low => 0,
        }
    }
    /// The share of the view's width and height the world is drawn at. The
    /// work grows with the number of pixels, so 2/3 is under half of it.
    fn render_scale(self) -> f32 {
        match self {
            Quality::High | Quality::Medium => 1.0,
            Quality::Low => 2.0 / 3.0,
        }
    }
    fn grass(self) -> usize {
        match self {
            Quality::High => 30_000,
            Quality::Medium => 13_000,
            Quality::Low => 0,
        }
    }
}

/// A HUD panel the 3D view frosts the glass behind: rectangle and corner
/// radius, in points.
pub type Glass = (Rect, f32);

fn srgb(r: u8, g: u8, b: u8) -> V3 {
    let f = |c: u8| (c as f32 / 255.0).powf(2.2);
    [f(r), f(g), f(b)]
}

#[derive(Debug, Clone, Copy)]
struct Look {
    sky_top: V3,
    sky_hor: V3,
    /// Horizon colour opposite the sun.
    sky_anti: V3,
    sun_col: V3,
    sun_dir: V3,
    fog: f32,
    /// How quickly haze thins with height (fog hugs everything).
    fog_falloff: f32,
    /// Distant mountains visible.
    mountains: bool,
    /// Cloud cover, 0 (fair weather) to 1 (overcast).
    clouds: f32,
    cloud_lit: V3,
    cloud_dark: V3,
    /// Strength of the glow around the sun.
    glow: f32,
    /// Colour of the sun or moon disc.
    disk: V3,
    stars: f32,
    exposure: f32,
    /// Fade colours towards grey (moonlight).
    desat: f32,
    wet: f32,
    /// Bike headlight strength.
    lamp: f32,
    /// How dark and crisp shadows are (0: none).
    shadow: f32,
    night: f32,
    /// How much of the blurred image is mixed back in.
    bloom: f32,
    /// Strength of the shafts of light around the sun.
    rays: f32,
    /// Contrast, saturation and vignette of the final grade.
    grade: V3,
    tint_shadow: V3,
    tint_light: V3,
    /// Meadow colours: lush, pale and dry.
    grass: [V3; 3],
    /// How hard the wind blows through grass and trees.
    wind: f32,
}

fn dir(azimuth: f32, elevation: f32) -> V3 {
    norm([
        azimuth.sin() * elevation.cos(),
        elevation.sin(),
        -(azimuth.cos() * elevation.cos()),
    ])
}

fn grey(c: V3, k: f32) -> V3 {
    let l = (0.3 * c[0] + 0.6 * c[1] + 0.1 * c[2]) * k;
    [l, l, l]
}

fn mix3(a: V3, b: V3, t: f32) -> V3 {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn look(s: Scenery, sky: Sky) -> Look {
    let (sky_top, sky_hor, sun_col, fog) = match s {
        Scenery::Hills => (
            srgb(44, 108, 196),
            srgb(176, 204, 228),
            [1.1, 1.0, 0.86],
            0.00125,
        ),
        Scenery::Lake => (
            srgb(48, 118, 208),
            srgb(186, 212, 234),
            [1.1, 1.01, 0.88],
            0.0011,
        ),
        Scenery::Coast => (
            srgb(40, 116, 212),
            srgb(182, 216, 238),
            [1.12, 1.03, 0.9],
            0.00095,
        ),
        Scenery::Alpine => (
            srgb(28, 86, 190),
            srgb(160, 194, 228),
            [1.16, 1.07, 0.94],
            0.00085,
        ),
    };
    let grass = match s {
        Scenery::Hills => [[0.036, 0.09, 0.02], [0.08, 0.15, 0.036], [0.16, 0.16, 0.06]],
        Scenery::Lake => [
            [0.032, 0.09, 0.024],
            [0.07, 0.15, 0.04],
            [0.135, 0.16, 0.058],
        ],
        Scenery::Alpine => [
            [0.032, 0.092, 0.028],
            [0.072, 0.155, 0.042],
            [0.125, 0.165, 0.065],
        ],
        Scenery::Coast => [
            [0.05, 0.095, 0.024],
            [0.11, 0.155, 0.042],
            [0.21, 0.185, 0.08],
        ],
    };
    let mut l = Look {
        sky_top,
        sky_hor,
        sky_anti: sky_hor,
        sun_col,
        sun_dir: dir(2.3, 0.62),
        fog,
        fog_falloff: 0.003,
        mountains: true,
        clouds: 0.0,
        cloud_lit: [1.1, 1.07, 1.02],
        cloud_dark: [0.5, 0.56, 0.68],
        glow: 1.0,
        disk: scale(sun_col, 40.0),
        stars: 0.0,
        exposure: 1.0,
        desat: 0.0,
        wet: 0.0,
        lamp: 0.0,
        shadow: 1.0,
        night: 0.0,
        bloom: 0.06,
        rays: 0.5,
        grade: [0.3, 1.1, 0.22],
        tint_shadow: [0.97, 0.99, 1.04],
        tint_light: [1.02, 1.0, 0.97],
        grass,
        wind: 0.6,
    };
    match sky.time {
        TimeOfDay::Day => {}
        TimeOfDay::Evening => {
            l.sky_top = srgb(38, 62, 132);
            l.sky_hor = srgb(255, 150, 82);
            l.sky_anti = srgb(150, 118, 160);
            l.sun_col = [1.55, 0.66, 0.24];
            l.sun_dir = dir(2.3, 0.105);
            l.glow = 2.8;
            l.cloud_lit = [1.5, 0.7, 0.42];
            l.cloud_dark = [0.3, 0.22, 0.34];
            l.disk = [60.0, 26.0, 8.0];
            l.fog *= 1.25;
            l.bloom = 0.1;
            l.rays = 1.1;
            l.grade = [0.35, 1.14, 0.3];
            l.tint_shadow = [0.94, 0.96, 1.08];
            l.tint_light = [1.05, 1.0, 0.93];
        }
        TimeOfDay::Night => {
            l.sky_top = srgb(3, 6, 18);
            l.sky_hor = srgb(14, 23, 46);
            l.sky_anti = l.sky_hor;
            l.sun_col = [0.12, 0.17, 0.29];
            l.sun_dir = dir(1.1, 0.7);
            l.glow = 0.55;
            l.cloud_lit = [0.075, 0.09, 0.125];
            l.cloud_dark = [0.02, 0.025, 0.04];
            l.disk = [5.0, 5.3, 5.8];
            l.stars = 1.0;
            l.exposure = 2.5;
            l.desat = 0.35;
            l.lamp = 1.0;
            l.night = 1.0;
            l.shadow = 0.85;
            l.bloom = 0.13;
            l.rays = 0.0;
            l.grade = [0.25, 1.0, 0.36];
            l.tint_shadow = [0.92, 0.97, 1.1];
            l.tint_light = [1.0, 1.0, 1.0];
        }
    }
    match sky.weather {
        Weather::Clear => {}
        Weather::Cloudy | Weather::Rain => {
            let rain = sky.weather == Weather::Rain;
            let dark = if rain { 0.6 } else { 0.85 };
            l.sky_top = mix3(l.sky_top, grey(l.sky_hor, 0.75 * dark), 0.8);
            l.sky_hor = mix3(l.sky_hor, grey(l.sky_hor, 0.9 * dark), 0.65);
            l.sky_anti = l.sky_hor;
            l.cloud_lit = grey(l.sky_hor, 1.2);
            l.cloud_dark = grey(l.sky_hor, 0.5);
            l.clouds = if rain { 1.0 } else { 0.8 };
            l.sun_col = scale(l.sun_col, if rain { 0.3 } else { 0.5 });
            l.glow *= 0.3;
            l.disk = [0.0; 3];
            l.stars = 0.0;
            l.fog *= if rain { 3.2 } else { 1.5 };
            l.wet = if rain { 1.0 } else { 0.0 };
            l.shadow *= if rain { 0.35 } else { 0.6 };
            l.rays *= 0.25;
            l.grade[1] *= 0.92;
            l.wind = if rain { 1.4 } else { 0.9 };
        }
        Weather::Fog => {
            // One even grey: sky, clouds and distance all melt together.
            let g = grey(l.sky_hor, 0.95);
            l.sky_top = g;
            l.sky_hor = g;
            l.sky_anti = g;
            l.cloud_lit = scale(g, 1.0 / 0.85);
            l.cloud_dark = l.cloud_lit;
            l.clouds = 1.0;
            l.sun_col = scale(l.sun_col, 0.4);
            l.glow = 0.0;
            l.disk = scale(l.disk, 0.01);
            l.stars = 0.0;
            l.fog = 0.0085;
            l.fog_falloff = 0.0004;
            l.mountains = false;
            l.wet = 0.3;
            l.shadow *= 0.3;
            l.rays = 0.0;
            l.bloom *= 1.8;
            l.grade[1] *= 0.85;
            l.wind = 0.25;
        }
    }
    l
}

/// View-projection matrices of the two shadow cascades around the camera,
/// and the world size of one shadow texel in each.
fn shadow_cascades(eye: V3, fwd: V3, sun: V3, res: f32) -> ([M4; 2], [f32; 2]) {
    let flat = norm([fwd[0], 0.0, fwd[2]]);
    let up = if sun[1].abs() > 0.95 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    // Looking down the sun's rays, from the origin's side.
    let view = look_at(sun, [0.0; 3], up);
    let mut vps = [[0.0; 16]; 2];
    let mut texels = [0.0; 2];
    for (k, (ahead, radius)) in [(26.0f32, 40.0f32), (185.0, 230.0)].into_iter().enumerate() {
        let c = add(eye, scale(flat, ahead));
        let lc = transform(&view, [c[0], c[1], c[2], 1.0]);
        // Move in whole texels so shadow edges do not crawl as you ride.
        let texel = 2.0 * radius / res.max(1.0);
        let (x, y) = (
            (lc[0] / texel).round() * texel,
            (lc[1] / texel).round() * texel,
        );
        let proj = ortho(
            x - radius,
            x + radius,
            y - radius,
            y + radius,
            -lc[2] - 1600.0,
            -lc[2] + 1600.0,
        );
        vps[k] = mul(&proj, &view);
        texels[k] = texel;
    }
    (vps, texels)
}

/// Everything the GPU needs for one frame.
struct FrameData {
    vp: M4,
    inv_vp: M4,
    cam: V3,
    /// Camera viewing direction.
    fwd: V3,
    sun: V3,
    look: Look,
    time: f32,
    origin: (f64, f64),
    scenery: i32,
    water: f32,
    snow: f32,
    anchor: V3,
    road: Vec<f32>,
    rider: Vec<f32>,
    posts: Vec<f32>,
    reflectors: Vec<f32>,
    planks: Vec<f32>,
    rider_shadow: Vec<f32>,
    /// Camera velocity, for rain streaks.
    cam_vel: V3,
    quality: Quality,
    /// HUD panels relative to the view's top left corner, in points.
    glass: Vec<([f32; 4], f32)>,
    sea: bool,
    lamp_pos: V3,
    lamp_dir: V3,
    light_vp: [M4; 2],
    shadow_texels: [f32; 2],
    terrain: Option<Arc<TerrainData>>,
    mountains: Option<(Vec<f32>, Vec<Ring>)>,
}

struct CurrentTerrain {
    ox: f64,
    oz: f64,
    water: Option<f64>,
    snow: f64,
}

/// CPU side, owned by the app.
pub struct GpuWorld {
    gl: Arc<Mutex<GlState>>,
    worker: Worker,
    route: Option<Arc<Route>>,
    terrain: Option<CurrentTerrain>,
    pending: bool,
    mountains_for: Option<String>,
    yaw: f64,
    pitch: f64,
    initialised: bool,
    time: f64,
    /// Side of the road a lake is on (+1 right, -1 left).
    water_side: f64,
}

impl Default for GpuWorld {
    fn default() -> Self {
        GpuWorld {
            gl: Arc::new(Mutex::new(GlState::default())),
            worker: Worker::spawn(),
            route: None,
            terrain: None,
            pending: false,
            mountains_for: None,
            yaw: 0.0,
            pitch: 0.0,
            initialised: false,
            time: 0.0,
            water_side: 1.0,
        }
    }
}

/// Small deterministic hash for scenery that comes and goes along a route.
fn terrain_hash(a: i64, b: i64, seed: u64) -> u64 {
    let mut h = (a as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((b as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
        ^ seed.wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^ (h >> 29)
}

/// Which side of the route its inside is on: +1 right of the direction of
/// travel, -1 left (where a lake lies, if the route circles one).
fn inside_of(route: &Route) -> f64 {
    let area: f64 = route
        .points()
        .windows(2)
        .map(|w| w[0].x * w[1].z - w[1].x * w[0].z)
        .sum();
    if area > 0.0 {
        -1.0
    } else {
        1.0
    }
}

fn wrap_angle(a: f64) -> f64 {
    (a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

impl GpuWorld {
    /// Why the GPU renderer gave up, if it did.
    pub fn error(&self) -> Option<String> {
        self.gl.lock().ok().and_then(|g| g.error.clone())
    }

    /// Whether the view blurs what lies behind the HUD's glass panels (it
    /// cannot where off-screen rendering is unavailable).
    pub fn frosts_glass(&self) -> bool {
        self.gl.lock().is_ok_and(|g| !g.direct && g.error.is_none())
    }

    pub fn draw(
        &mut self,
        painter: &egui::Painter,
        rect: Rect,
        scene: &Scene,
        quality: Quality,
        glass: &[Glass],
    ) {
        let (route, dt) = (scene.route, scene.dt);
        let d = scene.riders.first().map(|r| r.d).unwrap_or(0.0);
        self.time += dt;
        if self
            .route
            .as_ref()
            .is_none_or(|r| r.id != route.id || r.length != route.length)
        {
            self.route = Some(Arc::new(route.clone()));
            self.water_side = inside_of(route);
            self.terrain = None;
            self.pending = false;
            self.initialised = false;
        }
        let route_arc = self.route.clone().unwrap();

        // --- camera (world space: x east, z north) ----------------------
        let lane = |s: f64, lat: f64| {
            let p = route.at(s);
            (
                p.x + lat * p.heading.cos(),
                p.z - lat * p.heading.sin(),
                p.ele,
            )
        };
        let (cx, cz, _) = lane(d - 7.0, LANE);
        let cy = route.at(d - 7.0).ele.max(route.at(d).ele - 1.0) + 2.5;
        let (tx, tz, te) = lane(d + 16.0, LANE);
        let (dx, dy, dz) = (tx - cx, te + 0.8 - cy, tz - cz);
        let yaw_t = dx.atan2(dz);
        let pitch_t = dy.atan2((dx * dx + dz * dz).sqrt()).clamp(-0.5, 0.5);
        if !self.initialised || wrap_angle(yaw_t - self.yaw).abs() > 1.2 {
            self.yaw = yaw_t;
            self.pitch = pitch_t;
            self.initialised = true;
        } else {
            let k = 1.0 - (-dt * 5.0).exp();
            self.yaw = wrap_angle(self.yaw + wrap_angle(yaw_t - self.yaw) * k);
            self.pitch += (pitch_t - self.pitch) * k;
        }

        // --- terrain windows --------------------------------------------
        let ahead = (cx + self.yaw.sin() * 450.0, cz + self.yaw.cos() * 450.0);
        let needs = match &self.terrain {
            None => true,
            Some(t) => ((cx - t.ox).powi(2) + (cz - t.oz).powi(2)).sqrt() > 320.0,
        };
        if needs && !self.pending {
            self.worker.request(route_arc.clone(), ahead.0, ahead.1);
            self.pending = true;
        }
        let mut upload = None;
        while let Ok(t) = self.worker.rx.try_recv() {
            if t.route_id == route.id {
                self.pending = false;
                upload = Some(Arc::new(t));
            }
        }
        if let Some(t) = &upload {
            self.terrain = Some(CurrentTerrain {
                ox: t.ox,
                oz: t.oz,
                water: t.water_level,
                snow: t.snow_level,
            });
        }
        let origin = match &self.terrain {
            Some(t) => (t.ox, t.oz),
            None => ((cx / 100.0).round() * 100.0, (cz / 100.0).round() * 100.0),
        };
        let rp = |x: f64, y: f64, z: f64| -> V3 {
            [(x - origin.0) as f32, y as f32, -(z - origin.1) as f32]
        };

        let eye = rp(cx, cy, cz);
        let fwd = [
            (self.yaw.sin() * self.pitch.cos()) as f32,
            self.pitch.sin() as f32,
            -(self.yaw.cos() * self.pitch.cos()) as f32,
        ];
        let aspect = rect.width() / rect.height().max(1.0);
        let proj = perspective(55f32.to_radians(), aspect, 0.25, 12000.0);
        let view = look_at(eye, add(eye, fwd), [0.0, 1.0, 0.0]);
        let vp = mul(&proj, &view);
        let inv_vp = invert(&vp).unwrap_or([0.0; 16]);

        // --- road ribbon -------------------------------------------------
        let mut road = Vec::with_capacity(20_000);
        let mut slices = Vec::new();
        let mut s = ((d - 25.0) / 2.5).floor() * 2.5;
        while s < d + 1100.0 {
            slices.push(s);
            s += if s < d + 150.0 {
                2.5
            } else if s < d + 400.0 {
                5.0
            } else {
                10.0
            };
        }
        let laterals: [(f64, f64); 4] = [(-5.8, -0.25), (-4.6, 0.03), (4.6, 0.03), (5.8, -0.25)];
        let row = |s: f64| -> Vec<(V3, f32)> {
            let p = route.at(s);
            laterals
                .iter()
                .map(|&(l, dy)| {
                    (
                        rp(
                            p.x + l * p.heading.cos(),
                            p.ele + dy,
                            p.z - l * p.heading.sin(),
                        ),
                        l as f32,
                    )
                })
                .collect()
        };
        let mut prev = row(slices[0]);
        let mut prev_s = slices[0];
        for &s in &slices[1..] {
            let cur = row(s);
            for k in 0..3 {
                let (a, ua) = prev[k];
                let (b, ub) = prev[k + 1];
                let (c, uc) = cur[k + 1];
                let (e, ue) = cur[k];
                let (va, vc) = (prev_s as f32, s as f32);
                for (p, u, v) in [
                    (a, ua, va),
                    (b, ub, va),
                    (c, uc, vc),
                    (a, ua, va),
                    (c, uc, vc),
                    (e, ue, vc),
                ] {
                    road.extend_from_slice(&[p[0], p[1], p[2], u, v]);
                }
            }
            prev = cur;
            prev_s = s;
        }

        // --- riders --------------------------------------------------------
        let mut rider = Vec::new();
        let mut rider_shadow = Vec::new();
        let mut tags = Vec::new();
        let (mut lamp_pos, mut lamp_dir) = ([0.0; 3], [0.0, 0.0, -1.0]);
        for (i, fig) in scene.riders.iter().enumerate() {
            // Skip companions out of sight behind the camera or far ahead.
            if i > 0 && !(d - 8.0..d + 900.0).contains(&fig.d) {
                continue;
            }
            let here = route.at(fig.d);
            let grade = route.grade(fig.d) / 100.0;
            let (h_sin, h_cos) = (here.heading.sin() as f32, here.heading.cos() as f32);
            let flat_right = [h_cos, 0.0, h_sin];
            let fwd_r = norm([h_sin, grade as f32, -h_cos]);
            let flat_up = cross(flat_right, fwd_r);
            // Lean into bends, and rock a little with each pedal stroke.
            let turn =
                wrap_angle(route.at(fig.d + 4.0).heading - route.at(fig.d - 4.0).heading) / 8.0;
            let lean = ((fig.speed * fig.speed * turn / 9.81).atan() * 0.8).clamp(-0.5, 0.5) as f32
                + 0.018 * fig.crank.sin();
            let (ls, lc) = lean.sin_cos();
            let up_r = add(scale(flat_up, lc), scale(flat_right, ls));
            let right = sub(scale(flat_right, lc), scale(flat_up, ls));
            let (rx, rz, re) = lane(fig.d, fig.lateral);
            let pos = rp(rx, re + 0.02, rz);
            let j = fig.jersey;
            rider.extend(
                mesh::rider(&mesh::RiderPose {
                    pos,
                    fwd: fwd_r,
                    up: up_r,
                    right,
                    crank: fig.crank,
                    wheel: ((fig.d / 0.34) % std::f64::consts::TAU) as f32,
                    jersey: srgb(j.r(), j.g(), j.b()),
                })
                .v,
            );
            rider_shadow.extend_from_slice(&[pos[0], pos[1] + 0.02, pos[2], 0.9]);
            if i == 0 {
                lamp_pos = add(add(pos, scale(up_r, 1.0)), scale(fwd_r, 0.7));
                lamp_dir = norm(add(scale(fwd_r, 14.0), scale(up_r, -1.1)));
            }
            if let Some(t) = &fig.tag {
                tags.push((add(pos, [0.0, 2.0, 0.0]), t.clone()));
            }
        }

        // --- roadside posts and kilometre banners --------------------------
        let mut posts = Vec::new();
        let mut reflectors = Vec::new();
        let mut planks = Vec::new();
        let mut labels = Vec::new();
        let mut k = ((d - 20.0) / 50.0).ceil() as i64;
        while (k as f64) * 50.0 < d + 700.0 {
            let ps = k as f64 * 50.0;
            let p = route.at(ps);
            for side in [-1.0, 1.0] {
                let pos = rp(
                    p.x + side * 4.9 * p.heading.cos(),
                    p.ele,
                    p.z - side * 4.9 * p.heading.sin(),
                );
                posts.extend_from_slice(&[
                    pos[0], pos[1], pos[2], 0.0, 0.11, 1.05, 0.75, 0.75, 0.72,
                ]);
                // A red reflector on the right, white on the left.
                let c = if side > 0.0 {
                    [0.7, 0.03, 0.02]
                } else {
                    [0.7, 0.7, 0.66]
                };
                reflectors.extend_from_slice(&[
                    pos[0],
                    pos[1] + 0.8,
                    pos[2],
                    0.0,
                    0.118,
                    0.15,
                    c[0],
                    c[1],
                    c[2],
                ]);
            }
            k += 1;
        }
        let mut km = ((d - 10.0) / 1000.0).ceil() as i64;
        while (km as f64) * 1000.0 < d + 500.0 {
            let ks = km as f64 * 1000.0;
            let p = route.at(ks);
            for side in [-1.0, 1.0] {
                let pos = rp(
                    p.x + side * 5.4 * p.heading.cos(),
                    p.ele,
                    p.z - side * 5.4 * p.heading.sin(),
                );
                posts
                    .extend_from_slice(&[pos[0], pos[1], pos[2], 0.0, 0.3, 5.3, 0.03, 0.03, 0.035]);
            }
            let centre = rp(p.x, p.ele + 3.9, p.z);
            planks.extend_from_slice(&[
                centre[0],
                centre[1],
                centre[2],
                -p.heading as f32,
                11.1,
                1.2,
                0.02,
                0.022,
                0.028,
            ]);
            // An accent stripe along the banner's lower edge.
            planks.extend_from_slice(&[
                centre[0],
                centre[1] - 0.09,
                centre[2],
                -p.heading as f32,
                11.3,
                0.1,
                0.72,
                0.16,
                0.03,
            ]);
            let lap_pos = ks.rem_euclid(route.length);
            let text = if lap_pos < 1.0 || route.length - lap_pos < 1.0 {
                "START · FINISH".to_string()
            } else {
                format!("{:.0} KM", lap_pos / 1000.0)
            };
            labels.push((add(centre, [0.0, 0.58, 0.0]), text));
            km += 1;
        }

        // --- fences along the fields -----------------------------------------
        let fence_sides: &[f64] = match (route.scenery, self.water_side) {
            (Scenery::Hills, _) => &[-1.0, 1.0],
            (Scenery::Lake, w) if w > 0.0 => &[-1.0],
            (Scenery::Lake, _) => &[1.0],
            _ => &[],
        };
        let wood = [0.17, 0.125, 0.085];
        for &side in fence_sides {
            let spacing = 3.2;
            let mut prev: Option<V3> = None;
            let mut k = ((d - 30.0) / spacing).floor() as i64;
            while (k as f64) * spacing < d + 320.0 {
                let fs = k as f64 * spacing;
                k += 1;
                // Fenced stretches come and go: fields, then open meadow.
                let stretch = (fs / 140.0).floor() as i64;
                let fenced = terrain_hash(stretch, side as i64, route.seed) % 5 < 2;
                if !fenced {
                    prev = None;
                    continue;
                }
                let p = route.at(fs);
                let lateral = side * 8.4;
                let pos = rp(
                    p.x + lateral * p.heading.cos(),
                    p.ele - 0.3,
                    p.z - lateral * p.heading.sin(),
                );
                posts.extend_from_slice(&[
                    pos[0],
                    pos[1],
                    pos[2],
                    -p.heading as f32,
                    0.13,
                    1.25,
                    wood[0],
                    wood[1],
                    wood[2],
                ]);
                if let Some(a) = prev {
                    let (dx, dz) = (pos[0] - a[0], pos[2] - a[2]);
                    let span = (dx * dx + dz * dz).sqrt();
                    let rot = (-dz).atan2(dx);
                    for rail in [0.5f32, 0.92] {
                        planks.extend_from_slice(&[
                            (a[0] + pos[0]) / 2.0,
                            (a[1] + pos[1]) / 2.0 + rail,
                            (a[2] + pos[2]) / 2.0,
                            rot,
                            span + 0.1,
                            0.11,
                            wood[0] * 1.15,
                            wood[1] * 1.15,
                            wood[2] * 1.15,
                        ]);
                    }
                }
                prev = Some(pos);
            }
        }

        // --- birds wheeling over the road ahead ------------------------------
        if scene.sky.time != TimeOfDay::Night
            && matches!(scene.sky.weather, Weather::Clear | Weather::Cloudy)
        {
            let block = ((d + 150.0) / 700.0).floor();
            let centre_s = block * 700.0 + 420.0;
            let c = route.at(centre_s);
            let seed = terrain_hash(block as i64, 7, route.seed);
            let side = if seed & 1 == 0 { 1.0 } else { -1.0 };
            let (bx, bz) = (
                c.x + side * 60.0 * c.heading.cos(),
                c.z - side * 60.0 * c.heading.sin(),
            );
            let t = self.time as f32;
            let dark = [0.02, 0.02, 0.025];
            let mut m = mesh::Mesh::default();
            for b in 0..7 {
                let fb = b as f32;
                let phase = fb * 0.9 + (seed % 100) as f32;
                let radius = 28.0 + 9.0 * fb;
                let a = t * (0.22 - 0.012 * fb) + phase;
                let centre = rp(bx, c.ele + 38.0 + 5.0 * b as f64, bz);
                let pos = add(
                    centre,
                    [
                        a.cos() * radius,
                        (t * 0.4 + fb).sin() * 3.0,
                        a.sin() * radius,
                    ],
                );
                // Flying along the circle, banking into it, wings beating.
                let fwd_b = [-a.sin(), 0.0, a.cos()];
                let out = [a.cos(), 0.0, a.sin()];
                let flap = (t * 7.0 + fb * 1.7).sin() * 0.45;
                let body = scale(fwd_b, 0.35);
                for wing in [-1.0f32, 1.0] {
                    let tip = add(
                        pos,
                        add(scale(out, wing * 0.85), [0.0, flap + 0.12 * wing, 0.0]),
                    );
                    m.tri(add(pos, body), tip, sub(pos, body), dark);
                }
            }
            rider.extend(m.v);
        }

        // --- mountains ring (rebuilt per route) ------------------------------
        let mountains = if self.mountains_for.as_deref() != Some(route.id.as_str()) {
            self.mountains_for = Some(route.id.clone());
            Some(mountain_ring(route.seed, route.scenery))
        } else {
            None
        };

        let look = look(route.scenery, scene.sky);
        let cam_vel = {
            let h = route.at(d);
            let v = scene.riders.first().map(|r| r.speed).unwrap_or(0.0) as f32;
            [
                h.heading.sin() as f32 * v,
                0.0,
                -(h.heading.cos() as f32) * v,
            ]
        };
        let (water, snow) = match &self.terrain {
            Some(t) => (
                t.water.map(|w| w as f32).unwrap_or(-1.0e9),
                // Only the Alps climb above the snow line.
                if route.scenery == Scenery::Alpine {
                    t.snow as f32
                } else {
                    1.0e9
                },
            ),
            None => (-1.0e9, 1.0e9),
        };
        let (light_vp, shadow_texels) =
            shadow_cascades(eye, fwd, look.sun_dir, quality.shadow_res() as f32);
        let frame = FrameData {
            light_vp,
            shadow_texels,
            vp,
            inv_vp,
            cam: eye,
            fwd,
            sun: look.sun_dir,
            look,
            time: self.time as f32,
            origin,
            scenery: match route.scenery {
                Scenery::Hills => 0,
                Scenery::Lake => 1,
                Scenery::Alpine => 2,
                Scenery::Coast => 3,
            },
            water,
            snow,
            anchor: [eye[0], eye[1] - 60.0, eye[2]],
            road,
            rider,
            posts,
            reflectors,
            planks,
            rider_shadow,
            cam_vel,
            quality,
            glass: glass
                .iter()
                .map(|(r, radius)| {
                    (
                        [
                            r.left() - rect.left(),
                            r.top() - rect.top(),
                            r.width(),
                            r.height(),
                        ],
                        *radius,
                    )
                })
                .collect(),
            sea: route.scenery == Scenery::Coast,
            lamp_pos,
            lamp_dir,
            terrain: upload,
            mountains,
        };

        let state = self.gl.clone();
        let frame = Arc::new(frame);
        let cb = egui_glow::CallbackFn::new(move |info, p| {
            if let Ok(mut st) = state.lock() {
                st.paint(p.gl(), &info, &frame);
            }
        });
        painter.add(egui::PaintCallback {
            rect,
            callback: Arc::new(cb),
        });

        for (p, text) in tags {
            if let Some((x, y, w)) = project(&vp, p, rect.width(), rect.height()) {
                crate::world::name_tag(
                    painter,
                    Pos2::new(rect.left() + x, rect.top() + y),
                    &text,
                    w as f64,
                );
            }
        }

        // Banner text drawn by egui on top of the 3D view.
        for (p, text) in labels {
            if let Some((x, y, w)) = project(&vp, p, rect.width(), rect.height()) {
                let size = (rect.height() * 0.62 / w) * 1.6;
                if (6.0..140.0).contains(&size) {
                    let fade = (1.0 - (w - 250.0) / 250.0).clamp(0.0, 1.0);
                    painter.with_clip_rect(rect).text(
                        Pos2::new(rect.left() + x, rect.top() + y),
                        Align2::CENTER_CENTER,
                        text,
                        FontId::proportional(size),
                        Color32::from_white_alpha((235.0 * fade) as u8),
                    );
                }
            }
        }
    }
}

/// One ring of distant mountains: where its vertices start, and how it is
/// shaded.
#[derive(Debug, Clone, Copy)]
struct Ring {
    first: i32,
    count: i32,
    haze: f32,
    snowline: f32,
    treeline: f32,
}

/// Distant mountain silhouettes as rings around the camera, far to near.
fn mountain_ring(seed: u64, scenery: Scenery) -> (Vec<f32>, Vec<Ring>) {
    // Height of each ring, then where snow starts and trees stop.
    let (heights, snowline, treeline) = match scenery {
        Scenery::Alpine => ([1500.0, 1000.0, 520.0], 520.0, 300.0),
        Scenery::Hills => ([520.0, 380.0, 230.0], 1.0e9, 1.0e9),
        Scenery::Lake => ([760.0, 460.0, 200.0], 640.0, 420.0),
        // Distant islands and headlands with open sea between them.
        Scenery::Coast => ([520.0, 420.0, 300.0], 1.0e9, 170.0),
    };
    let mut v = Vec::new();
    let mut rings = Vec::new();
    for (ring, (radius, haze)) in [(7400.0f32, 0.66f32), (5400.0, 0.5), (3900.0, 0.34)]
        .into_iter()
        .enumerate()
    {
        let height = heights[ring];
        let n = 480;
        let s = seed as f32 * 0.731 + ring as f32 * 4.1;
        let h = |a: f32| -> f32 {
            let mut sum = 0.0;
            for (k, amp) in [
                (3.0, 0.32),
                (5.0, 0.22),
                (8.0, 0.16),
                (13.0, 0.1),
                (21.0, 0.06),
                (34.0, 0.035),
                (55.0, 0.02),
                (89.0, 0.012),
            ] {
                sum += amp * ((a * k + s * k * 0.37).sin()).abs().powf(0.8);
            }
            if scenery == Scenery::Coast {
                height * (sum - 0.42)
            } else {
                height * (0.15 + sum)
            }
        };
        let col = match ring {
            0 => [0.1, 0.105, 0.125],
            1 => [0.095, 0.095, 0.1],
            _ => [0.085, 0.082, 0.078],
        };
        let point = |i: usize, frac: f32| -> (V3, V3) {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            let top = h(a);
            let y = -400.0 + (top + 400.0) * frac;
            let dir = [a.sin(), 0.0, -a.cos()];
            let slope = (h(a + 0.004) - h(a - 0.004)) / (0.008 * radius);
            let tangent = [a.cos(), 0.0, a.sin()];
            let n = norm(add(
                add(scale(dir, -0.6), [0.0, 0.75, 0.0]),
                scale(tangent, -slope * 2.5),
            ));
            ([dir[0] * radius, y, dir[2] * radius], n)
        };
        let first = (v.len() / 9) as i32;
        for i in 0..n {
            for (f0, f1) in [(0.0, 0.62), (0.62, 1.0)] {
                let corners = [
                    point(i, f0),
                    point(i + 1, f0),
                    point(i + 1, f1),
                    point(i, f1),
                ];
                for idx in [0, 1, 2, 0, 2, 3] {
                    let (p, nn) = corners[idx];
                    v.extend_from_slice(&[
                        p[0], p[1], p[2], nn[0], nn[1], nn[2], col[0], col[1], col[2],
                    ]);
                }
            }
        }
        rings.push(Ring {
            first,
            count: (v.len() / 9) as i32 - first,
            haze,
            snowline,
            treeline,
        });
    }
    (v, rings)
}

// ---------------------------------------------------------------------------
// GL side

struct Program {
    id: glow::Program,
    locs: HashMap<&'static str, Option<glow::UniformLocation>>,
}

impl Program {
    unsafe fn loc(
        &mut self,
        gl: &glow::Context,
        name: &'static str,
    ) -> Option<&glow::UniformLocation> {
        let id = self.id;
        self.locs
            .entry(name)
            .or_insert_with(|| unsafe { gl.get_uniform_location(id, name) })
            .as_ref()
    }
    unsafe fn f1(&mut self, gl: &glow::Context, name: &'static str, v: f32) {
        unsafe { gl.uniform_1_f32(self.loc(gl, name), v) }
    }
    unsafe fn i1(&mut self, gl: &glow::Context, name: &'static str, v: i32) {
        unsafe { gl.uniform_1_i32(self.loc(gl, name), v) }
    }
    unsafe fn f2(&mut self, gl: &glow::Context, name: &'static str, x: f32, y: f32) {
        unsafe { gl.uniform_2_f32(self.loc(gl, name), x, y) }
    }
    unsafe fn v3(&mut self, gl: &glow::Context, name: &'static str, v: V3) {
        unsafe { gl.uniform_3_f32(self.loc(gl, name), v[0], v[1], v[2]) }
    }
    unsafe fn m4(&mut self, gl: &glow::Context, name: &'static str, m: &M4) {
        unsafe { gl.uniform_matrix_4_f32_slice(self.loc(gl, name), false, m) }
    }
}

struct Buffers {
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    /// Instance buffer, if instanced.
    ibo: Option<glow::Buffer>,
    ebo: Option<glow::Buffer>,
    count: i32,
    instances: i32,
}

/// A texture that can be drawn into.
struct Target {
    fbo: glow::Framebuffer,
    tex: glow::Texture,
    w: i32,
    h: i32,
}

/// Off-screen buffers the size of the view.
struct Targets {
    w: i32,
    h: i32,
    samples: i32,
    /// Floating point colour; otherwise the scene shaders encode for the
    /// screen themselves.
    hdr: bool,
    /// The scene is drawn here (multisampled where possible)...
    scene_fbo: glow::Framebuffer,
    scene_color: glow::Renderbuffer,
    scene_depth: glow::Renderbuffer,
    /// ...and resolved into textures that later passes read.
    resolve_fbo: glow::Framebuffer,
    color_tex: glow::Texture,
    depth_tex: glow::Texture,
    /// Progressively smaller and blurrier copies of the scene.
    down: Vec<Target>,
    up: Vec<Target>,
    rays: Target,
}

struct ShadowMaps {
    res: i32,
    fbo: [glow::Framebuffer; 2],
    tex: [glow::Texture; 2],
}

#[derive(Default)]
struct GlState {
    ready: bool,
    pub error: Option<String>,
    programs: HashMap<&'static str, Program>,
    sky_vao: Option<glow::VertexArray>,
    terrain: Option<Buffers>,
    road: Option<Buffers>,
    water: Option<Buffers>,
    mountains: Option<(Buffers, Vec<Ring>)>,
    /// Object meshes, indexed by the `OBJ_` constants.
    objects: Vec<Buffers>,
    /// All instances of the current terrain window, per kind; split into
    /// near and far each frame.
    scattered: [Vec<f32>; KINDS],
    blobs: Option<Buffers>,
    rider_blob: Option<Buffers>,
    /// Grass tuft variants, each with its own share of the instances.
    grass: Vec<Buffers>,
    grass_for: Option<Quality>,
    rain: Option<Buffers>,
    /// Terrain heights for placing grass: height, road distance, forest.
    height_tex: Option<glow::Texture>,
    /// Half size, cell size and cells per side of the height texture.
    height_map: V3,
    targets: Option<Targets>,
    /// Off-screen rendering is not possible here: draw straight to the screen.
    direct: bool,
    shadow_maps: Option<ShadowMaps>,
    /// Cloud shadows for the deck above the view, made once per frame.
    cloud_map: Option<Target>,
    cloud_map_failed: bool,
    /// Its centre and half size in cloud-deck coordinates (0: no map).
    cloud_box: V3,
    /// Scratch space for sorting nearby objects by distance.
    order: [Vec<(f32, [f32; INSTANCE_STRIDE])>; KINDS],
}

fn bytes(v: &[f32]) -> &[u8] {
    // SAFETY: f32 has no invalid bit patterns and u8 has alignment 1.
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

fn bytes_u32(v: &[u32]) -> &[u8] {
    // SAFETY: as above.
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

// The first six match the kinds in `terrain`.
const OBJ_PINE: usize = terrain::PINE;
const OBJ_BROAD: usize = terrain::BROADLEAF;
const OBJ_ROCK: usize = terrain::ROCK;
const OBJ_HOUSE: usize = terrain::HOUSE;
const OBJ_BIRCH: usize = terrain::BIRCH;
const OBJ_BUSH: usize = terrain::BUSH;
const OBJ_POST: usize = 6;
const OBJ_PLANK: usize = 7;
const OBJ_RIDER: usize = 8;
const OBJ_PINE_FAR: usize = 9;
const OBJ_BROAD_FAR: usize = 10;
const OBJ_REFLECTOR: usize = 11;
/// Trees closer than this use the detailed meshes.
const NEAR_TREES: f32 = 190.0;
/// Objects further than this are hidden in the haze anyway.
const FAR_CULL: f32 = 1600.0;
/// Bushes and rocks are too small to see beyond this.
const SMALL_CULL: f32 = 600.0;
/// Things this close are kept even behind the camera: their shadows may
/// fall into view.
const KEEP_AROUND: f32 = 150.0;
/// Side of the square of grass that follows the camera, in metres.
const GRASS_PATCH: f32 = 60.0;
const RAIN_DROPS: usize = 6000;
/// Texture units.
const UNIT_HEIGHT: u32 = 4;
const UNIT_SM0: u32 = 5;
const UNIT_SM1: u32 = 6;
const UNIT_CLOUD: u32 = 7;
/// The cloud shadow map: texels per side, and the half size of the patch of
/// cloud deck it covers, in metres. The finest cloud detail is about 65 m
/// across, several texels.
const CLOUD_MAP_SIZE: i32 = 256;
const CLOUD_MAP_HALF: f32 = 1500.0;

/// Deterministic random numbers in 0..1 for instance buffers.
fn noise_stream(seed: u64) -> impl FnMut() -> f32 {
    let mut s = seed;
    move || {
        s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    }
}

impl GlState {
    unsafe fn compile(
        &mut self,
        gl: &glow::Context,
        name: &'static str,
        vs: &[&str],
        fs: &[&str],
        attribs: &[&str],
    ) -> Result<(), String> {
        unsafe {
            let v = gl.version();
            let header = if v.is_embedded {
                "#version 300 es\nprecision highp float;\nprecision highp int;\n\
                 precision highp sampler2D;\nprecision highp sampler2DShadow;\n"
            } else {
                "#version 330 core\n"
            };
            let program = gl.create_program()?;
            let mut shaders = Vec::new();
            for (kind, src) in [
                (glow::VERTEX_SHADER, vs.concat()),
                (glow::FRAGMENT_SHADER, fs.concat()),
            ] {
                let sh = gl.create_shader(kind)?;
                gl.shader_source(sh, &format!("{header}{src}"));
                gl.compile_shader(sh);
                if !gl.get_shader_compile_status(sh) {
                    return Err(format!("{name} shader: {}", gl.get_shader_info_log(sh)));
                }
                gl.attach_shader(program, sh);
                shaders.push(sh);
            }
            for (i, a) in attribs.iter().enumerate() {
                gl.bind_attrib_location(program, i as u32, a);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                return Err(format!(
                    "{name} program: {}",
                    gl.get_program_info_log(program)
                ));
            }
            for sh in shaders {
                gl.detach_shader(program, sh);
                gl.delete_shader(sh);
            }
            self.programs.insert(
                name,
                Program {
                    id: program,
                    locs: HashMap::new(),
                },
            );
            Ok(())
        }
    }

    unsafe fn init(&mut self, gl: &glow::Context) -> Result<(), String> {
        unsafe {
            let v = gl.version();
            if (!v.is_embedded && (v.major, v.minor) < (3, 3)) || (v.is_embedded && v.major < 3) {
                return Err(format!(
                    "OpenGL {}.{} is too old (3.3 needed)",
                    v.major, v.minor
                ));
            }
            use shaders::*;
            let terrain_attribs = ["a_pos", "a_nrm", "a_dist", "a_forest"];
            let object_attribs = [
                "a_pos",
                "a_nrm",
                "a_col",
                "a_mat",
                "i_pos_rot",
                "i_scale",
                "i_tint",
            ];
            self.compile(gl, "sky", &[SCREEN_VS], &[NOISE, LIGHT, SKY_FS], &[])?;
            self.compile(
                gl,
                "cloud_map",
                &[SCREEN_VS],
                &[NOISE, LIGHT, CLOUD_MAP_FS],
                &[],
            )?;
            self.compile(
                gl,
                "terrain",
                &[TERRAIN_VS],
                &[NOISE, LIGHT, TERRAIN_FS],
                &terrain_attribs,
            )?;
            self.compile(
                gl,
                "road",
                &[ROAD_VS],
                &[NOISE, LIGHT, ROAD_FS],
                &["a_pos", "a_uv"],
            )?;
            self.compile(
                gl,
                "object",
                &[OBJECT_VS],
                &[NOISE, LIGHT, OBJECT_FS],
                &object_attribs,
            )?;
            self.compile(
                gl,
                "blob",
                &[BLOB_VS],
                &[NOISE, LIGHT, BLOB_FS],
                &["a_corner", "i_shadow"],
            )?;
            self.compile(
                gl,
                "grass",
                &[NOISE, GRASS_VS],
                &[NOISE, LIGHT, GRASS_FS],
                &["a_pos", "a_col", "i_seed"],
            )?;
            self.compile(
                gl,
                "rain",
                &[NOISE, RAIN_VS],
                &[NOISE, LIGHT, RAIN_FS],
                &["a_corner", "i_seed"],
            )?;
            self.compile(
                gl,
                "water",
                &[WATER_VS],
                &[NOISE, LIGHT, WATER_FS],
                &["a_pos"],
            )?;
            self.compile(
                gl,
                "mountain",
                &[MOUNTAIN_VS],
                &[NOISE, LIGHT, MOUNTAIN_FS],
                &["a_pos", "a_nrm", "a_col"],
            )?;
            // Depth-only versions for the shadow maps.
            self.compile(
                gl,
                "sh_terrain",
                &[TERRAIN_VS],
                &[DEPTH_FS],
                &terrain_attribs,
            )?;
            self.compile(gl, "sh_object", &[OBJECT_VS], &[DEPTH_FS], &object_attribs)?;
            // Post-processing.
            self.compile(gl, "down", &[SCREEN_VS], &[DOWN_FS], &[])?;
            self.compile(gl, "up", &[SCREEN_VS], &[UP_FS], &[])?;
            self.compile(gl, "rays", &[SCREEN_VS], &[RAYS_FS], &[])?;
            self.compile(gl, "composite", &[SCREEN_VS], &[COMPOSITE_FS], &[])?;
            self.sky_vao = Some(gl.create_vertex_array()?);
            // For troubleshooting: draw straight to the screen, no post pass.
            self.direct = std::env::var("VELOFORGE_POST").is_ok_and(|v| v == "0");

            self.road = Some(Self::buffers(gl, &[(3, 0), (2, 3)], 5, &[], None)?);
            self.water = Some(Self::buffers(gl, &[(3, 0)], 3, &[], None)?);
            let meshes = [
                mesh::pine(),
                mesh::broadleaf(),
                mesh::rock(),
                mesh::house(),
                mesh::birch(),
                mesh::bush(),
                mesh::post(),
                mesh::plank(),
                mesh::Mesh::default(),
                mesh::pine_far(),
                mesh::broadleaf_far(),
                mesh::reflector(),
            ];
            for m in meshes.iter() {
                let mut b = Self::buffers(
                    gl,
                    &[(3, 0), (3, 3), (3, 6), (1, 9)],
                    mesh::VERTEX_STRIDE as i32,
                    &m.v,
                    Some(&[(4, 0), (2, 4), (3, 6)]),
                )?;
                b.count = m.vertex_count();
                self.objects.push(b);
            }
            // The rider is drawn once with an identity instance.
            let rider = &mut self.objects[OBJ_RIDER];
            gl.bind_buffer(glow::ARRAY_BUFFER, rider.ibo);
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytes(&[0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0]),
                glow::STATIC_DRAW,
            );
            rider.instances = 1;
            let quad = [
                -1.0f32, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0,
            ];
            let mut sh = Self::buffers(gl, &[(2, 0)], 2, &quad, Some(&[(4, 0)]))?;
            sh.count = 6;
            self.blobs = Some(sh);
            let mut rs = Self::buffers(gl, &[(2, 0)], 2, &quad, Some(&[(4, 0)]))?;
            rs.count = 6;
            self.rider_blob = Some(rs);

            // Rain: streaks scattered through a box that follows the camera.
            let streak = [
                -1.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 1.0,
            ];
            let mut rain = Self::buffers(gl, &[(2, 0)], 2, &streak, Some(&[(4, 0)]))?;
            rain.count = 6;
            let mut next = noise_stream(77);
            let seeds: Vec<f32> = (0..RAIN_DROPS * 4).map(|_| next()).collect();
            Self::set_instances(gl, &mut rain, &seeds, 4);
            self.rain = Some(rain);
            gl.bind_vertex_array(None);
            Ok(())
        }
    }

    /// Grass tufts for the chosen quality: a few variants of the tuft, each
    /// with its share of positions inside the patch.
    unsafe fn ensure_grass(&mut self, gl: &glow::Context, quality: Quality) -> Result<(), String> {
        if self.grass_for == Some(quality) {
            return Ok(());
        }
        unsafe {
            for b in self.grass.drain(..) {
                gl.delete_vertex_array(b.vao);
                gl.delete_buffer(b.vbo);
                if let Some(i) = b.ibo {
                    gl.delete_buffer(i);
                }
            }
            let total = quality.grass();
            let variants = [
                (mesh::grass_tuft(6, 0, false, 1), 0.46),
                (mesh::grass_tuft(7, 0, false, 2), 0.3),
                (mesh::grass_tuft(5, 0, true, 3), 0.14),
                (mesh::grass_tuft(4, 2, false, 4), 0.1),
            ];
            let mut next = noise_stream(4242);
            for (verts, share) in variants {
                let n = (total as f32 * share) as usize;
                if n == 0 {
                    continue;
                }
                let mut b = Self::buffers(
                    gl,
                    &[(3, 0), (3, 3)],
                    mesh::GRASS_STRIDE as i32,
                    &verts,
                    Some(&[(4, 0)]),
                )?;
                let mut seeds = Vec::with_capacity(n * 4);
                for _ in 0..n {
                    seeds.extend_from_slice(&[
                        next() * GRASS_PATCH,
                        next() * GRASS_PATCH,
                        next() * std::f32::consts::TAU,
                        next(),
                    ]);
                }
                Self::set_instances(gl, &mut b, &seeds, 4);
                self.grass.push(b);
            }
            gl.bind_vertex_array(None);
        }
        self.grass_for = Some(quality);
        Ok(())
    }

    /// Create a VAO with a vertex buffer laid out as `(components, offset)`
    /// floats, and optionally an instance buffer (attributes follow on).
    unsafe fn buffers(
        gl: &glow::Context,
        layout: &[(i32, i32)],
        stride: i32,
        data: &[f32],
        instance: Option<&[(i32, i32)]>,
    ) -> Result<Buffers, String> {
        unsafe {
            let vao = gl.create_vertex_array()?;
            gl.bind_vertex_array(Some(vao));
            let vbo = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(data), glow::STATIC_DRAW);
            for (i, &(n, off)) in layout.iter().enumerate() {
                gl.enable_vertex_attrib_array(i as u32);
                gl.vertex_attrib_pointer_f32(i as u32, n, glow::FLOAT, false, stride * 4, off * 4);
            }
            let mut ibo = None;
            if let Some(inst) = instance {
                let b = gl.create_buffer()?;
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(b));
                let istride: i32 = inst.iter().map(|x| x.0).sum();
                for (k, &(n, off)) in inst.iter().enumerate() {
                    let loc = (layout.len() + k) as u32;
                    gl.enable_vertex_attrib_array(loc);
                    gl.vertex_attrib_pointer_f32(loc, n, glow::FLOAT, false, istride * 4, off * 4);
                    gl.vertex_attrib_divisor(loc, 1);
                }
                ibo = Some(b);
            }
            gl.bind_vertex_array(None);
            Ok(Buffers {
                vao,
                vbo,
                ibo,
                ebo: None,
                count: (data.len() as i32) / stride,
                instances: 0,
            })
        }
    }

    unsafe fn set_instances(gl: &glow::Context, b: &mut Buffers, data: &[f32], stride: usize) {
        unsafe {
            gl.bind_buffer(glow::ARRAY_BUFFER, b.ibo);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(data), glow::DYNAMIC_DRAW);
        }
        b.instances = (data.len() / stride) as i32;
    }

    /// Cull scattered objects behind the camera or lost in the haze, and use
    /// simplified meshes for distant trees.
    unsafe fn split_scattered(&mut self, gl: &glow::Context, f: &FrameData) {
        let fwd = f.fwd;
        let mut near: [Vec<f32>; KINDS] = Default::default();
        let mut far: [Vec<f32>; 2] = Default::default();
        for (kind, list) in self.scattered.iter().enumerate() {
            let tree = matches!(kind, OBJ_PINE | OBJ_BROAD | OBJ_BIRCH);
            let small = matches!(kind, OBJ_BUSH | OBJ_ROCK);
            for inst in list.as_chunks::<INSTANCE_STRIDE>().0 {
                let rel = sub([inst[0], inst[1], inst[2]], f.cam);
                let dist = len(rel);
                if dist > FAR_CULL
                    || (small && dist > SMALL_CULL)
                    || (dot(rel, fwd) < -inst[5] && dist > KEEP_AROUND)
                {
                    continue;
                }
                if tree && dist > NEAR_TREES {
                    far[if kind == OBJ_PINE { 0 } else { 1 }].extend_from_slice(inst);
                } else {
                    near[kind].extend_from_slice(inst);
                }
            }
        }
        // Nearest first, so what they hide is not worked out at all.
        for (kind, list) in near.iter_mut().enumerate() {
            let order = &mut self.order[kind];
            order.clear();
            order.extend(list.as_chunks::<INSTANCE_STRIDE>().0.iter().map(|inst| {
                let rel = sub([inst[0], inst[1], inst[2]], f.cam);
                (dot(rel, rel), *inst)
            }));
            order.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
            list.clear();
            list.extend(order.iter().flat_map(|(_, inst)| *inst));
        }
        unsafe {
            for (kind, list) in near.iter().enumerate() {
                Self::set_instances(gl, &mut self.objects[kind], list, INSTANCE_STRIDE);
            }
            Self::set_instances(
                gl,
                &mut self.objects[OBJ_PINE_FAR],
                &far[0],
                INSTANCE_STRIDE,
            );
            Self::set_instances(
                gl,
                &mut self.objects[OBJ_BROAD_FAR],
                &far[1],
                INSTANCE_STRIDE,
            );
        }
    }

    unsafe fn upload_terrain(&mut self, gl: &glow::Context, t: &TerrainData) -> Result<(), String> {
        unsafe {
            if self.terrain.is_none() {
                let mut b = Self::buffers(
                    gl,
                    &[(3, 0), (3, 3), (1, 6), (1, 7)],
                    TERRAIN_STRIDE as i32,
                    &[],
                    None,
                )?;
                let ebo = gl.create_buffer()?;
                gl.bind_vertex_array(Some(b.vao));
                gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));
                gl.bind_vertex_array(None);
                b.ebo = Some(ebo);
                self.terrain = Some(b);
            }
            let b = self.terrain.as_mut().unwrap();
            gl.bind_vertex_array(Some(b.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(b.vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&t.verts), glow::STATIC_DRAW);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, b.ebo);
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                bytes_u32(&t.indices),
                glow::STATIC_DRAW,
            );
            gl.bind_vertex_array(None);
            b.count = t.indices.len() as i32;
            self.scattered = t.instances.clone();
            Self::set_instances(gl, self.blobs.as_mut().unwrap(), &t.shadows, 4);

            // The same heights as a texture, for planting grass on the GPU.
            let n = ((t.verts.len() / TERRAIN_STRIDE) as f64).sqrt().round() as usize;
            let mut texels = Vec::with_capacity(n * n * 4);
            for v in t.verts.as_chunks::<TERRAIN_STRIDE>().0 {
                texels.extend_from_slice(&[v[1], v[6], v[7], 0.0]);
            }
            if self.height_tex.is_none() {
                self.height_tex = Some(gl.create_texture()?);
            }
            gl.active_texture(glow::TEXTURE0 + UNIT_HEIGHT);
            gl.bind_texture(glow::TEXTURE_2D, self.height_tex);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA32F as i32,
                n as i32,
                n as i32,
                0,
                glow::RGBA,
                glow::FLOAT,
                glow::PixelUnpackData::Slice(Some(bytes(&texels))),
            );
            for (k, v) in [
                (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
                (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, k, v as i32);
            }
            gl.active_texture(glow::TEXTURE0);
            self.height_map = [
                (t.size / 2.0) as f32,
                (t.size / (n - 1) as f64) as f32,
                n as f32,
            ];

            // Water plane covering the window.
            let w = self.water.as_mut().unwrap();
            if let Some(level) = t.water_level {
                let h = if t.sea {
                    14_000.0
                } else {
                    (t.size / 2.0) as f32
                };
                let y = level as f32;
                let quad = [-h, y, -h, h, y, -h, h, y, h, -h, y, -h, h, y, h, -h, y, h];
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(w.vbo));
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&quad), glow::STATIC_DRAW);
                w.count = 6;
            } else {
                w.count = 0;
            }
            Ok(())
        }
    }

    // --- off-screen targets -------------------------------------------------

    unsafe fn texture(
        gl: &glow::Context,
        w: i32,
        h: i32,
        internal: u32,
        format: u32,
        ty: u32,
        filter: u32,
    ) -> Result<glow::Texture, String> {
        unsafe {
            let tex = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                internal as i32,
                w,
                h,
                0,
                format,
                ty,
                glow::PixelUnpackData::Slice(None),
            );
            for (k, v) in [
                (glow::TEXTURE_MIN_FILTER, filter),
                (glow::TEXTURE_MAG_FILTER, filter),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, k, v as i32);
            }
            Ok(tex)
        }
    }

    /// A colour texture with a framebuffer to draw into it.
    unsafe fn target(gl: &glow::Context, w: i32, h: i32, hdr: bool) -> Result<Target, String> {
        unsafe {
            let (internal, ty) = if hdr {
                (glow::RGBA16F, glow::HALF_FLOAT)
            } else {
                (glow::RGBA8, glow::UNSIGNED_BYTE)
            };
            let tex = Self::texture(gl, w, h, internal, glow::RGBA, ty, glow::LINEAR)?;
            let fbo = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(tex),
                0,
            );
            if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                gl.delete_framebuffer(fbo);
                gl.delete_texture(tex);
                return Err("render target not supported".into());
            }
            Ok(Target { fbo, tex, w, h })
        }
    }

    unsafe fn make_targets(
        gl: &glow::Context,
        w: i32,
        h: i32,
        samples: i32,
        hdr: bool,
    ) -> Result<Targets, String> {
        unsafe {
            let color_format = if hdr { glow::RGBA16F } else { glow::RGBA8 };
            let scene_color = gl.create_renderbuffer()?;
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(scene_color));
            gl.renderbuffer_storage_multisample(glow::RENDERBUFFER, samples, color_format, w, h);
            let scene_depth = gl.create_renderbuffer()?;
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(scene_depth));
            gl.renderbuffer_storage_multisample(
                glow::RENDERBUFFER,
                samples,
                glow::DEPTH_COMPONENT24,
                w,
                h,
            );
            let scene_fbo = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(scene_fbo));
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::RENDERBUFFER,
                Some(scene_color),
            );
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(scene_depth),
            );
            let scene_ok =
                gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE;

            let resolved = Self::target(gl, w, h, hdr);
            let depth_tex = Self::texture(
                gl,
                w,
                h,
                glow::DEPTH_COMPONENT24,
                glow::DEPTH_COMPONENT,
                glow::UNSIGNED_INT,
                glow::NEAREST,
            )?;
            let cleanup = |gl: &glow::Context| {
                gl.delete_framebuffer(scene_fbo);
                gl.delete_renderbuffer(scene_color);
                gl.delete_renderbuffer(scene_depth);
                gl.delete_texture(depth_tex);
            };
            let resolved = match resolved {
                Ok(r) if scene_ok => r,
                Ok(r) => {
                    gl.delete_framebuffer(r.fbo);
                    gl.delete_texture(r.tex);
                    cleanup(gl);
                    return Err("scene buffer not supported".into());
                }
                Err(e) => {
                    cleanup(gl);
                    return Err(e);
                }
            };
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(resolved.fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::TEXTURE_2D,
                Some(depth_tex),
                0,
            );
            if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                gl.delete_framebuffer(resolved.fbo);
                gl.delete_texture(resolved.tex);
                cleanup(gl);
                return Err("resolve buffer not supported".into());
            }
            let mut t = Targets {
                w,
                h,
                samples,
                hdr,
                scene_fbo,
                scene_color,
                scene_depth,
                resolve_fbo: resolved.fbo,
                color_tex: resolved.tex,
                depth_tex,
                down: Vec::new(),
                up: Vec::new(),
                rays: Self::target(gl, (w / 2).max(1), (h / 2).max(1), false)?,
            };
            // The blur pyramid: halve until it gets small.
            let (mut lw, mut lh) = (w, h);
            for _ in 0..6 {
                lw = (lw / 2).max(1);
                lh = (lh / 2).max(1);
                t.down.push(Self::target(gl, lw, lh, hdr)?);
                if lw.min(lh) <= 8 {
                    break;
                }
            }
            for d in &t.down[..t.down.len() - 1] {
                t.up.push(Self::target(gl, d.w, d.h, hdr)?);
            }
            Ok(t)
        }
    }

    unsafe fn delete_targets(gl: &glow::Context, t: Targets) {
        unsafe {
            gl.delete_framebuffer(t.scene_fbo);
            gl.delete_renderbuffer(t.scene_color);
            gl.delete_renderbuffer(t.scene_depth);
            gl.delete_framebuffer(t.resolve_fbo);
            gl.delete_texture(t.color_tex);
            gl.delete_texture(t.depth_tex);
            for x in t.down.into_iter().chain(t.up).chain([t.rays]) {
                gl.delete_framebuffer(x.fbo);
                gl.delete_texture(x.tex);
            }
        }
    }

    unsafe fn drain_errors(gl: &glow::Context) {
        // Bounded: a lost context reports an error for ever.
        for _ in 0..32 {
            if unsafe { gl.get_error() } == glow::NO_ERROR {
                break;
            }
        }
    }

    /// Make sure the off-screen buffers match the view's size and quality.
    unsafe fn ensure_targets(&mut self, gl: &glow::Context, w: i32, h: i32, quality: Quality) {
        if self.direct {
            return;
        }
        unsafe {
            let samples = quality
                .msaa()
                .min(gl.get_parameter_i32(glow::MAX_SAMPLES).max(0));
            if let Some(t) = &self.targets {
                // Fewer samples than asked for means the driver could not do more.
                if t.w == w && t.h == h && t.samples <= samples {
                    return;
                }
            }
            if let Some(t) = self.targets.take() {
                Self::delete_targets(gl, t);
            }
            // Best first: floating point with anti-aliasing, down to plain 8 bit.
            let no_hdr = std::env::var("VELOFORGE_HDR").is_ok_and(|v| v == "0");
            for (s, hdr) in [(samples, true), (0, true), (samples, false), (0, false)] {
                if hdr && no_hdr {
                    continue;
                }
                // Drain errors from failed attempts so they are not blamed on egui.
                Self::drain_errors(gl);
                if let Ok(t) = Self::make_targets(gl, w, h, s, hdr) {
                    self.targets = Some(t);
                    break;
                }
            }
            Self::drain_errors(gl);
            if self.targets.is_none() {
                self.direct = true;
            }
        }
    }

    unsafe fn ensure_shadow_maps(
        &mut self,
        gl: &glow::Context,
        quality: Quality,
    ) -> Result<(), String> {
        // A tiny stand-in keeps the shadow samplers valid when shadows are off.
        let res = quality.shadow_res().max(4);
        if self.shadow_maps.as_ref().is_some_and(|s| s.res == res) {
            return Ok(());
        }
        unsafe {
            if let Some(s) = self.shadow_maps.take() {
                for k in 0..2 {
                    gl.delete_framebuffer(s.fbo[k]);
                    gl.delete_texture(s.tex[k]);
                }
            }
            let mut fbos = Vec::new();
            let mut texs = Vec::new();
            for _ in 0..2 {
                let tex = Self::texture(
                    gl,
                    res,
                    res,
                    glow::DEPTH_COMPONENT24,
                    glow::DEPTH_COMPONENT,
                    glow::UNSIGNED_INT,
                    glow::LINEAR,
                )?;
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_COMPARE_MODE,
                    glow::COMPARE_REF_TO_TEXTURE as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_COMPARE_FUNC,
                    glow::LEQUAL as i32,
                );
                let fbo = gl.create_framebuffer()?;
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.framebuffer_texture_2d(
                    glow::FRAMEBUFFER,
                    glow::DEPTH_ATTACHMENT,
                    glow::TEXTURE_2D,
                    Some(tex),
                    0,
                );
                gl.draw_buffers(&[glow::NONE]);
                gl.read_buffer(glow::NONE);
                if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                    return Err("shadow maps not supported".into());
                }
                // Everything is lit until the first shadow pass.
                gl.clear_depth_f32(1.0);
                gl.depth_mask(true);
                gl.clear(glow::DEPTH_BUFFER_BIT);
                fbos.push(fbo);
                texs.push(tex);
            }
            self.shadow_maps = Some(ShadowMaps {
                res,
                fbo: [fbos[0], fbos[1]],
                tex: [texs[0], texs[1]],
            });
        }
        Ok(())
    }

    // --- per-pass helpers ---------------------------------------------------

    /// Bind a program and set the uniforms every lit shader shares.
    unsafe fn common(&mut self, gl: &glow::Context, name: &'static str, f: &FrameData) {
        let (texels, inv_res) = match &self.shadow_maps {
            Some(s) => (f.shadow_texels, 1.0 / s.res as f32),
            None => ([0.0; 2], 1.0),
        };
        let encode = !self.targets.as_ref().is_some_and(|t| t.hdr);
        unsafe {
            let p = self.programs.get_mut(name).unwrap();
            gl.use_program(Some(p.id));
            p.m4(gl, "u_vp", &f.vp);
            p.v3(gl, "u_cam", f.cam);
            p.v3(gl, "u_sun", f.sun);
            let l = &f.look;
            for (name, v) in [
                ("u_sun_col", l.sun_col),
                ("u_sky_top", l.sky_top),
                ("u_sky_hor", l.sky_hor),
                ("u_sky_anti", l.sky_anti),
                ("u_cloud_lit", l.cloud_lit),
                ("u_cloud_dark", l.cloud_dark),
                ("u_disk", l.disk),
                ("u_lamp_pos", f.lamp_pos),
                ("u_lamp_dir", f.lamp_dir),
                ("u_grass0", l.grass[0]),
                ("u_grass1", l.grass[1]),
                ("u_grass2", l.grass[2]),
            ] {
                p.v3(gl, name, v);
            }
            for (name, v) in [
                ("u_fog", l.fog),
                ("u_exposure", l.exposure),
                ("u_desat", l.desat),
                ("u_fog_falloff", l.fog_falloff),
                ("u_glow", l.glow),
                ("u_clouds", l.clouds),
                ("u_stars", l.stars),
                ("u_wet", l.wet),
                ("u_lamp", l.lamp),
                ("u_night", l.night),
                ("u_moon", l.night),
                ("u_time", f.time),
                ("u_encode", if encode { 1.0 } else { 0.0 }),
            ] {
                p.f1(gl, name, v);
            }
            p.f2(gl, "u_origin", f.origin.0 as f32, f.origin.1 as f32);
            p.m4(gl, "u_lvp0", &f.light_vp[0]);
            p.m4(gl, "u_lvp1", &f.light_vp[1]);
            p.i1(gl, "u_sm0", UNIT_SM0 as i32);
            p.i1(gl, "u_sm1", UNIT_SM1 as i32);
            p.i1(gl, "u_cloud_map", UNIT_CLOUD as i32);
            p.v3(gl, "u_cloud_box", self.cloud_box);
            let strength = if f.quality == Quality::Low {
                0.0
            } else {
                l.shadow
            };
            gl.uniform_4_f32(p.loc(gl, "u_sm"), strength, texels[0], texels[1], inv_res);
        }
    }

    /// Draw instanced objects with the program that is bound.
    unsafe fn draw_instanced(&self, gl: &glow::Context, which: &[usize]) {
        unsafe {
            for &i in which {
                let b = &self.objects[i];
                if b.instances > 0 && b.count > 0 {
                    gl.bind_vertex_array(Some(b.vao));
                    gl.draw_arrays_instanced(glow::TRIANGLES, 0, b.count, b.instances);
                }
            }
        }
    }

    /// Render the two shadow cascades from the sun's point of view.
    unsafe fn shadow_pass(&mut self, gl: &glow::Context, f: &FrameData) {
        let Some(maps) = &self.shadow_maps else {
            return;
        };
        let (res, fbos) = (maps.res, maps.fbo);
        unsafe {
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.color_mask(false, false, false, false);
            gl.enable(glow::POLYGON_OFFSET_FILL);
            gl.polygon_offset(2.5, 6.0);
            gl.viewport(0, 0, res, res);
            for (k, fbo) in fbos.into_iter().enumerate() {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.clear_depth_f32(1.0);
                gl.clear(glow::DEPTH_BUFFER_BIT);
                if let Some(t) = &self.terrain {
                    let (vao, count) = (t.vao, t.count);
                    let p = self.programs.get_mut("sh_terrain").unwrap();
                    gl.use_program(Some(p.id));
                    p.m4(gl, "u_vp", &f.light_vp[k]);
                    gl.bind_vertex_array(Some(vao));
                    gl.draw_elements(glow::TRIANGLES, count, glow::UNSIGNED_INT, 0);
                }
                let p = self.programs.get_mut("sh_object").unwrap();
                gl.use_program(Some(p.id));
                p.m4(gl, "u_vp", &f.light_vp[k]);
                p.f1(gl, "u_obj_time", f.time);
                p.f1(gl, "u_wind", f.look.wind);
                self.draw_instanced(gl, &[OBJ_PINE, OBJ_BROAD, OBJ_BIRCH, OBJ_BUSH]);
                if k == 1 {
                    self.draw_instanced(gl, &[OBJ_PINE_FAR, OBJ_BROAD_FAR]);
                }
                let p = self.programs.get_mut("sh_object").unwrap();
                p.f1(gl, "u_wind", 0.0);
                self.draw_instanced(gl, &[OBJ_ROCK, OBJ_HOUSE, OBJ_POST, OBJ_PLANK, OBJ_RIDER]);
            }
            gl.disable(glow::POLYGON_OFFSET_FILL);
            gl.color_mask(true, true, true, true);
        }
    }

    /// Work out the cloud shadows once, for the patch of cloud deck above
    /// the view, instead of for every pixel that is lit (`CLOUD_MAP_FS`).
    /// Without the map, each pixel works them out itself.
    unsafe fn cloud_pass(&mut self, gl: &glow::Context, f: &FrameData) {
        unsafe {
            if self.cloud_map.is_none() && !self.cloud_map_failed {
                Self::drain_errors(gl);
                match Self::target(gl, CLOUD_MAP_SIZE, CLOUD_MAP_SIZE, false) {
                    Ok(t) => self.cloud_map = Some(t),
                    Err(_) => self.cloud_map_failed = true,
                }
                Self::drain_errors(gl);
            }
            let Some(fbo) = self.cloud_map.as_ref().map(|m| m.fbo) else {
                self.cloud_box = [0.0; 3];
                return;
            };
            // Centred where the line from the camera towards the sun meets
            // the deck (as `cloud_q` in the shaders).
            let t = 1400.0 / f.sun[1].max(0.12);
            self.cloud_box = [
                f.cam[0] + f.sun[0] * t + f.origin.0 as f32 + f.time * 6.0,
                -(f.cam[2] + f.sun[2] * t) + f.origin.1 as f32 + f.time * 2.0,
                CLOUD_MAP_HALF,
            ];
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.viewport(0, 0, CLOUD_MAP_SIZE, CLOUD_MAP_SIZE);
            gl.disable(glow::DEPTH_TEST);
            self.common(gl, "cloud_map", f);
            gl.bind_vertex_array(self.sky_vao);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
    }

    /// Copy the scene drawn so far into the textures later passes read.
    unsafe fn resolve(gl: &glow::Context, t: &Targets) {
        unsafe {
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(t.scene_fbo));
            gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(t.resolve_fbo));
            gl.blit_framebuffer(
                0,
                0,
                t.w,
                t.h,
                0,
                0,
                t.w,
                t.h,
                glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT,
                glow::NEAREST,
            );
        }
    }

    fn paint(&mut self, gl: &glow::Context, info: &egui::PaintCallbackInfo, f: &FrameData) {
        if self.error.is_some() {
            return;
        }
        unsafe {
            if !self.ready {
                if let Err(e) = self.init(gl) {
                    self.error = Some(e);
                    return;
                }
                self.ready = true;
            }
            if let Some(t) = &f.terrain {
                if let Err(e) = self.upload_terrain(gl, t) {
                    self.error = Some(e);
                    return;
                }
            }
            if let Some((v, rings)) = &f.mountains {
                match Self::buffers(gl, &[(3, 0), (3, 3), (3, 6)], 9, v, None) {
                    Ok(b) => {
                        if let Some((old, _)) = self.mountains.take() {
                            gl.delete_vertex_array(old.vao);
                            gl.delete_buffer(old.vbo);
                        }
                        self.mountains = Some((b, rings.clone()));
                    }
                    Err(e) => {
                        self.error = Some(e);
                        return;
                    }
                }
            }
            // Whatever egui was drawing into is where the finished frame goes.
            let screen = gl.get_parameter_framebuffer(glow::DRAW_FRAMEBUFFER_BINDING);
            let view = info.viewport_in_pixels();
            let (w, h) = (view.width_px.max(1), view.height_px.max(1));
            if let Err(e) = self
                .ensure_shadow_maps(gl, f.quality)
                .and_then(|_| self.ensure_grass(gl, f.quality))
            {
                self.error = Some(e);
                return;
            }
            let scale = f.quality.render_scale();
            let (rw, rh) = (
                ((w as f32 * scale).round() as i32).max(1),
                ((h as f32 * scale).round() as i32).max(1),
            );
            self.ensure_targets(gl, rw, rh, f.quality);

            // Per-frame geometry.
            let road = self.road.as_mut().unwrap();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(road.vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&f.road), glow::STREAM_DRAW);
            road.count = (f.road.len() / 5) as i32;
            let rider = &mut self.objects[OBJ_RIDER];
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(rider.vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&f.rider), glow::STREAM_DRAW);
            rider.count = (f.rider.len() / mesh::VERTEX_STRIDE) as i32;
            self.split_scattered(gl, f);
            Self::set_instances(gl, &mut self.objects[OBJ_POST], &f.posts, INSTANCE_STRIDE);
            Self::set_instances(
                gl,
                &mut self.objects[OBJ_REFLECTOR],
                &f.reflectors,
                INSTANCE_STRIDE,
            );
            Self::set_instances(gl, &mut self.objects[OBJ_PLANK], &f.planks, INSTANCE_STRIDE);
            Self::set_instances(gl, self.rider_blob.as_mut().unwrap(), &f.rider_shadow, 4);

            gl.disable(glow::BLEND);
            gl.disable(glow::CULL_FACE);
            gl.disable(glow::SCISSOR_TEST);

            self.cloud_pass(gl, f);
            let shadows = f.quality != Quality::Low && f.look.shadow > 0.0;
            if shadows {
                self.shadow_pass(gl, f);
            }
            if let Some(maps) = &self.shadow_maps {
                for (unit, tex) in [(UNIT_SM0, maps.tex[0]), (UNIT_SM1, maps.tex[1])] {
                    gl.active_texture(glow::TEXTURE0 + unit);
                    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                }
            }
            gl.active_texture(glow::TEXTURE0 + UNIT_HEIGHT);
            gl.bind_texture(glow::TEXTURE_2D, self.height_tex);
            gl.active_texture(glow::TEXTURE0 + UNIT_CLOUD);
            gl.bind_texture(glow::TEXTURE_2D, self.cloud_map.as_ref().map(|m| m.tex));
            gl.active_texture(glow::TEXTURE0);

            // --- the scene ---------------------------------------------------
            let set_view = |gl: &glow::Context| {
                gl.viewport(
                    view.left_px,
                    view.from_bottom_px,
                    view.width_px,
                    view.height_px,
                );
            };
            match &self.targets {
                Some(t) => {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(t.scene_fbo));
                    gl.viewport(0, 0, t.w, t.h);
                }
                None => {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, screen);
                    set_view(gl);
                    gl.enable(glow::SCISSOR_TEST);
                }
            }
            gl.depth_mask(true);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::DEPTH_BUFFER_BIT);

            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);

            // Objects first: the ground hidden behind them then fails the
            // depth test and is not worked out at all. Trees and bushes sway
            // in the wind; everything else stands still.
            self.common(gl, "object", f);
            let p = self.programs.get_mut("object").unwrap();
            p.f1(gl, "u_obj_time", f.time);
            p.f1(gl, "u_wind", f.look.wind);
            self.draw_instanced(
                gl,
                &[
                    OBJ_PINE,
                    OBJ_BROAD,
                    OBJ_BIRCH,
                    OBJ_BUSH,
                    OBJ_PINE_FAR,
                    OBJ_BROAD_FAR,
                ],
            );
            let p = self.programs.get_mut("object").unwrap();
            p.f1(gl, "u_wind", 0.0);
            self.draw_instanced(
                gl,
                &[
                    OBJ_ROCK,
                    OBJ_HOUSE,
                    OBJ_POST,
                    OBJ_REFLECTOR,
                    OBJ_PLANK,
                    OBJ_RIDER,
                ],
            );

            // Distant mountains, far to near.
            if let Some((b, rings)) = self.mountains.as_ref().filter(|_| f.look.mountains) {
                let (vao, rings) = (b.vao, rings.clone());
                self.common(gl, "mountain", f);
                let p = self.programs.get_mut("mountain").unwrap();
                p.v3(gl, "u_anchor", f.anchor);
                gl.bind_vertex_array(Some(vao));
                for (k, r) in rings.iter().enumerate() {
                    p.f1(gl, "u_haze", r.haze);
                    p.f1(gl, "u_snowline", r.snowline);
                    p.f1(gl, "u_treeline", r.treeline);
                    p.f1(gl, "u_ring", k as f32);
                    gl.draw_arrays(glow::TRIANGLES, r.first, r.count);
                }
            }

            // Terrain.
            if let Some(t) = &self.terrain {
                let (vao, count) = (t.vao, t.count);
                self.common(gl, "terrain", f);
                let p = self.programs.get_mut("terrain").unwrap();
                p.i1(gl, "u_scenery", f.scenery);
                p.f1(gl, "u_water", f.water);
                p.f1(gl, "u_snow", f.snow);
                gl.bind_vertex_array(Some(vao));
                gl.draw_elements(glow::TRIANGLES, count, glow::UNSIGNED_INT, 0);
            }

            // Road, pulled slightly towards the camera so it never z-fights the ground.
            gl.enable(glow::POLYGON_OFFSET_FILL);
            gl.polygon_offset(-1.0, -4.0);
            let (rvao, rcount) = {
                let r = self.road.as_ref().unwrap();
                (r.vao, r.count)
            };
            self.common(gl, "road", f);
            let p = self.programs.get_mut("road").unwrap();
            p.f1(gl, "u_snow", f.snow);
            gl.bind_vertex_array(Some(rvao));
            gl.draw_arrays(glow::TRIANGLES, 0, rcount);
            gl.disable(glow::POLYGON_OFFSET_FILL);

            // Grass around the camera.
            if !self.grass.is_empty() && self.height_tex.is_some() {
                self.common(gl, "grass", f);
                let p = self.programs.get_mut("grass").unwrap();
                let flat = norm([f.fwd[0], 0.0, f.fwd[2]]);
                let ahead = GRASS_PATCH * 0.3;
                p.f2(
                    gl,
                    "u_centre",
                    f.cam[0] + flat[0] * ahead,
                    f.cam[2] + flat[2] * ahead,
                );
                p.f1(gl, "u_patch", GRASS_PATCH);
                p.i1(gl, "u_height", UNIT_HEIGHT as i32);
                p.v3(gl, "u_hmap", self.height_map);
                p.f1(gl, "u_water", f.water);
                p.f1(gl, "u_snow", f.snow);
                p.f1(gl, "u_gust", f.look.wind);
                for b in &self.grass {
                    gl.bind_vertex_array(Some(b.vao));
                    gl.draw_arrays_instanced(glow::TRIANGLES, 0, b.count, b.instances);
                }
            }

            // Soft darkening under trees and riders.
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.depth_mask(false);
            gl.enable(glow::POLYGON_OFFSET_FILL);
            gl.polygon_offset(-2.0, -8.0);
            self.common(gl, "blob", f);
            let p = self.programs.get_mut("blob").unwrap();
            // Where the sun casts real shadows, only a hint is needed.
            let sun = if shadows { f.look.shadow } else { 0.0 };
            for (b, strength) in [
                (self.blobs.as_ref(), 0.42 - 0.24 * sun),
                (self.rider_blob.as_ref(), 0.5 - 0.2 * sun),
            ] {
                if let Some(b) = b.filter(|b| b.instances > 0) {
                    p.f1(gl, "u_strength", strength);
                    gl.bind_vertex_array(Some(b.vao));
                    gl.draw_arrays_instanced(glow::TRIANGLES, 0, b.count, b.instances);
                }
            }
            gl.disable(glow::POLYGON_OFFSET_FILL);
            gl.disable(glow::BLEND);
            gl.depth_mask(true);

            // Sky: after everything solid, so it is worked out only where it
            // shows. It sits at the far end of the depth range, behind all.
            gl.depth_mask(false);
            self.common(gl, "sky", f);
            let p = self.programs.get_mut("sky").unwrap();
            p.m4(gl, "u_inv_vp", &f.inv_vp);
            gl.bind_vertex_array(self.sky_vao);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.depth_mask(true);

            // Water sees what lies beneath it through a copy of the scene so far.
            let water = self
                .water
                .as_ref()
                .filter(|w| w.count > 0)
                .map(|w| (w.vao, w.count));
            if let Some((vao, count)) = water {
                if let Some(t) = &self.targets {
                    Self::resolve(gl, t);
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(t.scene_fbo));
                    gl.active_texture(glow::TEXTURE0 + 1);
                    gl.bind_texture(glow::TEXTURE_2D, Some(t.depth_tex));
                    gl.active_texture(glow::TEXTURE0);
                    gl.bind_texture(glow::TEXTURE_2D, Some(t.color_tex));
                }
                let seen = self.targets.as_ref().map(|t| (t.w, t.h));
                self.common(gl, "water", f);
                let p = self.programs.get_mut("water").unwrap();
                p.i1(gl, "u_scene", 0);
                p.i1(gl, "u_depth", 1);
                p.m4(gl, "u_inv_vp", &f.inv_vp);
                p.f1(gl, "u_has_scene", if seen.is_some() { 1.0 } else { 0.0 });
                let (sw, sh) = seen.unwrap_or((1, 1));
                p.f2(gl, "u_res", sw as f32, sh as f32);
                p.f1(gl, "u_sea", if f.sea { 1.0 } else { 0.0 });
                gl.bind_vertex_array(Some(vao));
                gl.draw_arrays(glow::TRIANGLES, 0, count);
            }

            // Rain.
            if f.look.wet >= 1.0 {
                if let Some(r) = self.rain.as_ref().map(|r| (r.vao, r.count, r.instances)) {
                    gl.enable(glow::BLEND);
                    gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                    gl.depth_mask(false);
                    self.common(gl, "rain", f);
                    let p = self.programs.get_mut("rain").unwrap();
                    p.v3(gl, "u_cam_vel", f.cam_vel);
                    gl.bind_vertex_array(Some(r.0));
                    gl.draw_arrays_instanced(glow::TRIANGLES, 0, r.1, r.2);
                    gl.disable(glow::BLEND);
                    gl.depth_mask(true);
                }
            }

            // --- finishing ---------------------------------------------------
            gl.disable(glow::DEPTH_TEST);
            gl.depth_mask(false);
            if let Some(t) = self.targets.take() {
                Self::resolve(gl, &t);
                self.post(gl, &t, f, info.pixels_per_point, [w, h]);
                gl.bind_framebuffer(glow::FRAMEBUFFER, screen);
                set_view(gl);
                gl.enable(glow::SCISSOR_TEST);
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
                self.targets = Some(t);
            }
            for unit in [1, 2, 3, UNIT_HEIGHT, UNIT_SM0, UNIT_SM1, UNIT_CLOUD] {
                gl.active_texture(glow::TEXTURE0 + unit);
                gl.bind_texture(glow::TEXTURE_2D, None);
            }
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.depth_mask(true);
            gl.bind_vertex_array(None);
            gl.use_program(None);
        }
    }

    /// Blur pyramid, light shafts, and the final composite left bound and
    /// ready to draw to the screen, which is `screen` pixels (the scene may
    /// have been drawn smaller).
    unsafe fn post(
        &mut self,
        gl: &glow::Context,
        t: &Targets,
        f: &FrameData,
        ppp: f32,
        screen: [i32; 2],
    ) {
        unsafe {
            gl.bind_vertex_array(self.sky_vao);
            gl.active_texture(glow::TEXTURE0);
            // Down: each level half the size of the last.
            let p = self.programs.get_mut("down").unwrap();
            gl.use_program(Some(p.id));
            p.i1(gl, "u_tex", 0);
            let (mut src, mut sw, mut sh) = (t.color_tex, t.w, t.h);
            for (i, d) in t.down.iter().enumerate() {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(d.fbo));
                gl.viewport(0, 0, d.w, d.h);
                gl.bind_texture(glow::TEXTURE_2D, Some(src));
                p.f2(gl, "u_texel", 1.0 / sw as f32, 1.0 / sh as f32);
                p.f1(gl, "u_first", if i == 0 && t.hdr { 1.0 } else { 0.0 });
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
                (src, sw, sh) = (d.tex, d.w, d.h);
            }
            // Up: fold each blurrier level back into the sharper one above it.
            let p = self.programs.get_mut("up").unwrap();
            gl.use_program(Some(p.id));
            p.i1(gl, "u_tex", 0);
            p.i1(gl, "u_base", 1);
            p.f1(gl, "u_mix", 0.62);
            for i in (0..t.up.len()).rev() {
                let small = if i + 1 < t.up.len() {
                    &t.up[i + 1]
                } else {
                    &t.down[i + 1]
                };
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(t.up[i].fbo));
                gl.viewport(0, 0, t.up[i].w, t.up[i].h);
                gl.active_texture(glow::TEXTURE0 + 1);
                gl.bind_texture(glow::TEXTURE_2D, Some(t.down[i].tex));
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(small.tex));
                p.f2(gl, "u_texel", 1.0 / small.w as f32, 1.0 / small.h as f32);
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
            }

            // Shafts of light, if the sun is in or near the picture.
            let sun_clip = transform(
                &f.vp,
                [
                    f.cam[0] + f.sun[0] * 1000.0,
                    f.cam[1] + f.sun[1] * 1000.0,
                    f.cam[2] + f.sun[2] * 1000.0,
                    1.0,
                ],
            );
            let mut rays = 0.0;
            let mut sun_uv = [0.5, 0.5];
            if sun_clip[3] > 0.0 && t.hdr && f.quality != Quality::Low {
                let (x, y) = (sun_clip[0] / sun_clip[3], sun_clip[1] / sun_clip[3]);
                let off = x.abs().max(y.abs());
                rays = f.look.rays * (1.0 - ((off - 0.9) / 0.8).clamp(0.0, 1.0));
                sun_uv = [x * 0.5 + 0.5, y * 0.5 + 0.5];
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(t.rays.fbo));
            gl.viewport(0, 0, t.rays.w, t.rays.h);
            if rays > 0.0 {
                let p = self.programs.get_mut("rays").unwrap();
                gl.use_program(Some(p.id));
                p.i1(gl, "u_depth", 0);
                p.f2(gl, "u_sun_uv", sun_uv[0], sun_uv[1]);
                p.f1(gl, "u_aspect", t.w as f32 / t.h as f32);
                gl.bind_texture(glow::TEXTURE_2D, Some(t.depth_tex));
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
            } else {
                gl.clear_color(0.0, 0.0, 0.0, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
            }

            // Composite.
            let l = &f.look;
            let bloom = t.up.first().unwrap_or(&t.down[0]);
            let blur = t.up.get(1).unwrap_or(bloom);
            for (unit, tex) in [
                (3, t.rays.tex),
                (2, blur.tex),
                (1, bloom.tex),
                (0, t.color_tex),
            ] {
                gl.active_texture(glow::TEXTURE0 + unit);
                gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            }
            let p = self.programs.get_mut("composite").unwrap();
            gl.use_program(Some(p.id));
            p.i1(gl, "u_scene", 0);
            p.i1(gl, "u_bloom", 1);
            p.i1(gl, "u_blur", 2);
            p.i1(gl, "u_rays", 3);
            p.f1(gl, "u_hdr", if t.hdr { 1.0 } else { 0.0 });
            p.f1(gl, "u_exposure", l.exposure);
            p.f1(gl, "u_desat", l.desat);
            p.f1(gl, "u_bloom_k", l.bloom);
            p.v3(gl, "u_ray_col", scale(l.sun_col, rays * 0.55));
            // The glass panels are placed in screen pixels.
            p.f2(gl, "u_res", screen[0] as f32, screen[1] as f32);
            p.v3(gl, "u_grade", l.grade);
            p.v3(gl, "u_tint_shadow", l.tint_shadow);
            p.v3(gl, "u_tint_light", l.tint_light);
            p.f1(gl, "u_time", f.time);
            let n = f.glass.len().min(16);
            let mut rects = [0.0f32; 64];
            let mut radii = [0.0f32; 16];
            for (i, (r, radius)) in f.glass.iter().take(n).enumerate() {
                for k in 0..4 {
                    rects[i * 4 + k] = r[k] * ppp;
                }
                radii[i] = radius * ppp;
            }
            p.i1(gl, "u_glass_n", n as i32);
            gl.uniform_4_f32_slice(p.loc(gl, "u_glass[0]"), &rects);
            gl.uniform_1_f32_slice(p.loc(gl, "u_glass_r[0]"), &radii);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mountain_ring_is_well_formed() {
        for s in [
            Scenery::Hills,
            Scenery::Lake,
            Scenery::Alpine,
            Scenery::Coast,
        ] {
            let (v, rings) = mountain_ring(7, s);
            assert_eq!(v.len() % 9, 0);
            assert_eq!(rings.len(), 3);
            assert_eq!(
                rings.iter().map(|r| r.count).sum::<i32>(),
                (v.len() / 9) as i32
            );
            assert!(
                rings.windows(2).all(|w| w[0].haze > w[1].haze),
                "far rings are hazier"
            );
            assert!(v.iter().all(|x| x.is_finite()));
        }
    }

    #[test]
    fn shadow_cascades_cover_the_road_ahead() {
        let eye = [10.0, 3.0, -20.0];
        let fwd = norm([0.3, -0.1, -1.0]);
        for sun in [dir(2.3, 0.62), dir(2.3, 0.105), dir(1.1, 0.7)] {
            let (vps, texels) = shadow_cascades(eye, fwd, sun, 2048.0);
            assert!(texels[0] < texels[1]);
            // A point on the road 20 m ahead is inside the near cascade, one
            // 250 m ahead inside the far one.
            for (k, ahead) in [(0, 20.0), (1, 250.0)] {
                let p = add(eye, scale(norm([fwd[0], 0.0, fwd[2]]), ahead));
                let c = transform(&vps[k], [p[0], 0.0, p[2], 1.0]);
                assert!(c.iter().take(3).all(|v| v.abs() < 1.0), "{c:?}");
            }
            // Towards the sun is nearer the light: smaller depth.
            let a = transform(&vps[0], [eye[0], 0.0, eye[2], 1.0]);
            let b = add([eye[0], 0.0, eye[2]], scale(sun, 50.0));
            let b = transform(&vps[0], [b[0], b[1], b[2], 1.0]);
            assert!(b[2] < a[2]);
        }
    }

    #[test]
    fn every_look_is_sane() {
        for s in Scenery::ALL {
            for time in TimeOfDay::ALL {
                for weather in Weather::ALL {
                    let l = look(s, Sky { time, weather });
                    assert!((len(l.sun_dir) - 1.0).abs() < 1e-4);
                    assert!(l.sun_dir[1] > 0.0, "the light is above the horizon");
                    assert!((0.0..=1.0).contains(&l.shadow));
                    assert!(l.fog > 0.0 && l.exposure > 0.0);
                }
            }
        }
    }
}
