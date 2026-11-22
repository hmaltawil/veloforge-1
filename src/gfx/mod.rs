//! GPU ride view: OpenGL rendering of terrain, road, trees, grass, water,
//! sky and a 3D rider inside an egui paint callback.
//!
//! The scene is drawn into an off-screen HDR target with sun shadows from
//! two cascaded shadow maps, then finished in a post pass (bloom, light
//! shafts, tonemapping, grading) that also frosts the glass behind the HUD.
//! Falls back to the software view in `world.rs` if anything goes wrong
//! (see [`GpuWorld::error`]).

mod body;
mod buildings;
mod fields;
mod flora;
mod impostor;
mod lettering;
mod math;
mod mesh;
mod shaders;
mod terrain;

use crate::route::{Route, Scenery};
use eframe::egui::{self, Pos2, Rect};
use eframe::egui_glow;
use eframe::glow::{self, HasContext};
use math::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use terrain::{Built, FarData, TerrainData, Worker, INSTANCE_STRIDE, KINDS, TERRAIN_STRIDE};

use crate::world::{Scene, Sky, TimeOfDay, Weather};

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
    pub fn render_scale(self) -> f32 {
        match self {
            Quality::High | Quality::Medium => 1.0,
            Quality::Low => 2.0 / 3.0,
        }
    }
    /// Where trees change from their full meshes to cards (metres; every
    /// tree's own is shifted by up to `LOD_JITTER` either way).
    /// `VELOFORGE_CARD_AT=metres` overrides it, for trying distances.
    fn tree_lod(self) -> TreeLod {
        static CARD_AT: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
        let card_at = *CARD_AT.get_or_init(|| {
            std::env::var("VELOFORGE_CARD_AT")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|m| (20.0..1500.0).contains(m))
        });
        if let Some(card) = card_at {
            return TreeLod { card };
        }
        match self {
            Quality::High => TreeLod { card: 300.0 },
            Quality::Medium => TreeLod { card: 200.0 },
            Quality::Low => TreeLod { card: 150.0 },
        }
    }
    /// Levels of the blur pyramid (bloom and the frosted glass).
    fn blur_levels(self) -> usize {
        match self {
            Quality::High => 6,
            Quality::Medium => 5,
            Quality::Low => 4,
        }
    }
    /// Samples along each light shaft.
    fn ray_samples(self) -> i32 {
        match self {
            Quality::High => 28,
            _ => 16,
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
            // Clear mountain air: the distant land shows, out to the peaks.
            0.0006,
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

/// The shadow cascades: how far ahead of the camera each is centred, and its
/// half-width (metres).
const CASCADES: [(f32, f32); 2] = [(26.0, 40.0), (185.0, 230.0)];

/// Whether something standing at `foot`, `h` tall and `r` across, casts its
/// shadow into the cascade drawn with `light_vp`: whether, seen from the sun,
/// any of it lies inside the cascade's square (`r_unit`: one metre across in
/// the cascade's units).
fn casts_into(light_vp: &M4, foot: V3, h: f32, r: f32, r_unit: f32) -> bool {
    let a = transform(light_vp, [foot[0], foot[1], foot[2], 1.0]);
    let b = transform(light_vp, [foot[0], foot[1] + h, foot[2], 1.0]);
    let pad = 1.0 + r * r_unit;
    a[0].min(b[0]) <= pad
        && a[0].max(b[0]) >= -pad
        && a[1].min(b[1]) <= pad
        && a[1].max(b[1]) >= -pad
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
    for (k, (ahead, radius)) in CASCADES.into_iter().enumerate() {
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
    /// Each rider's bone matrices and jersey colour.
    skins: Vec<([M4; body::BONES], V3)>,
    posts: Vec<f32>,
    reflectors: Vec<f32>,
    planks: Vec<f32>,
    rider_shadow: Vec<f32>,
    /// Camera velocity, for rain streaks.
    cam_vel: V3,
    quality: Quality,
    /// Share of the view's width and height the world is drawn at.
    render_scale: f32,
    /// HUD panels relative to the view's top left corner, in points.
    glass: Vec<([f32; 4], f32)>,
    sea: bool,
    lamp_pos: V3,
    lamp_dir: V3,
    light_vp: [M4; 2],
    shadow_texels: [f32; 2],
    terrain: Option<Arc<TerrainData>>,
    /// New distant land to upload, when there is some.
    far: Option<Arc<FarData>>,
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
    /// Each rider's standing and tucking, easing between poses.
    stances: Vec<Stance>,
    /// The body's bones at rest, inverted.
    rest_inverse: [M4; body::BONES],
    /// Milliseconds the last frame took to prepare (for `--bench`).
    prep_ms: f32,
    /// The land window drawn now, for standing roadside things on it.
    land: Option<Arc<TerrainData>>,
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
            stances: Vec::new(),
            rest_inverse: {
                start_sculpting();
                body::rest_inverse()
            },
            prep_ms: 0.0,
            land: None,
        }
    }
}

/// How a rider sits on the bike, easing from one pose to the next.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stance {
    /// 0 seated .. 1 standing; 0 .. 1 crouched low.
    pub stand: f32,
    pub tuck: f32,
    /// Seconds of hard effort, and of steep climbing, so far.
    hard: f32,
    steep: f32,
}

impl Stance {
    /// Move on `dt` seconds for a rider on a `grade` (a fraction) at
    /// `speed` m/s, riding at `effort` × FTP.
    pub fn update(&mut self, dt: f32, grade: f32, speed: f32, effort: f32, pedalling: bool) {
        self.hard = if pedalling && effort >= 1.5 {
            self.hard + dt
        } else {
            0.0
        };
        self.steep = if pedalling && grade >= 0.08 {
            self.steep + dt
        } else {
            0.0
        };
        // Out of the saddle to launch a sprint, and now and then on a steep
        // climb: 20 seconds up, then a minute and more seated.
        let sprint = self.hard > 0.0 && self.hard < 15.0;
        let climb = self.steep > 0.0 && self.steep % 90.0 < 20.0;
        let stand_to = if sprint || climb { 1.0 } else { 0.0 };
        // Low and narrow going fast downhill, or freewheeling fast.
        let fast = smoothstep(10.0, 13.0, speed);
        let tuck_to = if !pedalling || grade <= -0.03 {
            fast * (1.0 - stand_to)
        } else {
            0.0
        };
        let k = 1.0 - (-dt * 2.5).exp();
        self.stand += (stand_to - self.stand) * k;
        self.tuck += (tuck_to - self.tuck) * k;
    }
}

/// The rider's sculpted body: made once, on a background thread, as soon as
/// a 3D view exists (it takes a moment), then handed to the graphics card.
static SKIN: std::sync::OnceLock<body::Skin> = std::sync::OnceLock::new();

/// Pictures of each kind of tree (`TREES`) for distant trees' cards
/// (`impostor.rs`), made on a background thread too.
static CARDS: std::sync::OnceLock<Vec<impostor::Atlas>> = std::sync::OnceLock::new();

fn start_sculpting() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("sculpt-rider".into())
            .spawn(|| {
                let _ = SKIN.set(body::build(0.01));
            });
        let _ = std::thread::Builder::new()
            .name("bake-trees".into())
            .spawn(|| {
                let atlases = (0..TREE_COUNT)
                    .map(|t| impostor::bake(&tree_mesh(t).v))
                    .collect();
                let _ = CARDS.set(atlases);
            });
    });
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

    /// Time the GPU's work on each frame (for `--bench`).
    pub fn set_timing(&self, on: bool) {
        if let Ok(mut g) = self.gl.lock() {
            g.timing = on;
        }
    }

    /// Milliseconds the GPU spent on each stage of the last timed frame (see
    /// [`STAGES`]; None if it cannot be measured here).
    pub fn gpu_ms(&self) -> Option<[f32; STAGES]> {
        self.gl.lock().ok().and_then(|g| g.gpu_ms)
    }

    /// What each stage of the last timed frame drew: triangles, pixel
    /// samples and fragment shader runs (None if this driver cannot count).
    pub fn stage_counts(&self) -> Option<[StageCounts; STAGES]> {
        self.gl.lock().ok().and_then(|g| g.counts)
    }

    /// Milliseconds the GPU last spent outside the 3D view: drawing the rest
    /// of the screen, and showing the frame (None if it cannot be measured).
    pub fn other_ms(&self) -> Option<[f32; 2]> {
        self.gl.lock().ok().and_then(|g| g.other_ms)
    }

    /// Mark on the GPU where egui starts drawing the frame (`end` false:
    /// paint into the bottom layer, first) or is done (`end`: the top
    /// layer, last), to time the work around the 3D view (for `--bench`).
    pub fn mark_frame(&self, painter: &egui::Painter, end: bool) {
        let state = self.gl.clone();
        let cb = egui_glow::CallbackFn::new(move |_info, p| {
            if let Ok(mut st) = state.lock() {
                st.mark(p.gl(), if end { MARK_END } else { MARK_BEGIN });
            }
        });
        painter.add(egui::PaintCallback {
            rect: Rect::from_min_size(painter.clip_rect().min, egui::vec2(1.0, 1.0)),
            callback: Arc::new(cb),
        });
    }

    /// Whether the view blurs what lies behind the HUD's glass panels (it
    /// cannot where off-screen rendering is unavailable).
    pub fn frosts_glass(&self) -> bool {
        self.gl.lock().is_ok_and(|g| !g.direct && g.error.is_none())
    }

    /// How many times the land has been uploaded so far (for `--bench`).
    pub fn land_uploads(&self) -> u32 {
        self.gl.lock().map(|g| g.land_uploads).unwrap_or(0)
    }

    /// Milliseconds the computer spent on the last frame: preparing it, and
    /// issuing its drawing commands (for `--bench`).
    pub fn cpu_ms(&self) -> (f32, f32) {
        let paint = self.gl.lock().map(|g| g.paint_ms).unwrap_or(0.0);
        (self.prep_ms, paint)
    }

    pub fn draw(
        &mut self,
        painter: &egui::Painter,
        rect: Rect,
        scene: &Scene,
        quality: Quality,
        render_scale: f32,
        glass: &[Glass],
    ) {
        let t0 = std::time::Instant::now();
        self.prepare(painter, rect, scene, quality, render_scale, glass);
        self.prep_ms = t0.elapsed().as_secs_f32() * 1000.0;
    }

    fn prepare(
        &mut self,
        painter: &egui::Painter,
        rect: Rect,
        scene: &Scene,
        quality: Quality,
        render_scale: f32,
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
        let my_lane = scene
            .riders
            .first()
            .map(|r| r.lateral)
            .unwrap_or(crate::world::LANE);
        let ([cx, cy, cz], target) = scene.camera.rig(route, d, my_lane);
        let (dx, dy, dz) = (target[0] - cx, target[1] - cy, target[2] - cz);
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
        let ahead = look_ahead(cx, cz, self.yaw);
        let needs = match &self.terrain {
            None => true,
            Some(t) => needs_new_window(ahead, (t.ox, t.oz)),
        };
        if needs && !self.pending {
            self.worker.request(route_arc.clone(), ahead.0, ahead.1);
            self.pending = true;
        }
        let mut upload = None;
        let mut far = None;
        while let Ok(built) = self.worker.rx.try_recv() {
            match built {
                Built::Window(t) if t.route_id == route.id => {
                    self.pending = false;
                    upload = Some(Arc::new(*t));
                }
                Built::Far(t) if t.route_id == route.id => far = Some(Arc::new(t)),
                _ => {}
            }
        }
        if let Some(t) = &upload {
            self.land = Some(t.clone());
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
        // Stone walls hold the road up where the land falls away beside it
        // (down from both edges, hidden where the ground meets the road).
        let mut walls = mesh::Mesh::default();
        let mut prev = row(slices[0]);
        let mut prev_s = slices[0];
        for &s in &slices[1..] {
            let mut cur = row(s);
            // In a bend tighter than the road is wide its inside edge would
            // fold back on itself: an edge never moves backwards.
            let h = route.at(s).heading;
            let ahead = [h.sin() as f32, 0.0, -h.cos() as f32];
            for k in 0..4 {
                if dot(sub(cur[k].0, prev[k].0), ahead) < 0.01 {
                    cur[k].0 = prev[k].0;
                }
            }
            let right = [h.cos() as f32, 0.0, h.sin() as f32];
            // Masonry close by: courses of stone, each block a shade of its
            // own, under a paler coping; further off one dark face.
            let courses: &[(f32, f32)] = if s < d + 250.0 {
                &[
                    (0.0, 0.3),
                    (0.3, 0.8),
                    (0.8, 1.3),
                    (1.3, 1.8),
                    (1.8, 2.3),
                    (2.3, 2.8),
                    (2.8, 3.4),
                    (3.4, 25.0),
                ]
            } else {
                &[(0.0, 25.0)]
            };
            for (k, out) in [(0usize, scale(right, -1.0)), (3, right)] {
                let (a, b) = (
                    add(prev[k].0, [0.0, -0.03, 0.0]),
                    add(cur[k].0, [0.0, -0.03, 0.0]),
                );
                for (row, &(top, bottom)) in courses.iter().enumerate() {
                    let hash =
                        terrain_hash((s / 2.5) as i64 * 31 + k as i64, row as i64, route.seed);
                    let shade = 0.7 + 0.6 * (hash % 100) as f32 / 100.0;
                    // Weathered grey stone, darker low down where it is damp.
                    let stone = if row == 0 && courses.len() > 1 {
                        scale([0.13, 0.125, 0.115], shade)
                    } else {
                        scale([0.075, 0.072, 0.066], shade * (1.0 - 0.04 * row as f32))
                    };
                    let (t, u) = ([0.0, -top, 0.0], [0.0, -bottom, 0.0]);
                    let quad = [add(a, t), add(b, t), add(b, u), add(a, u)];
                    for i in [0, 1, 2, 0, 2, 3] {
                        walls.push(quad[i], out, stone);
                    }
                }
            }
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
        let mut skins = Vec::new();
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
            if self.stances.len() <= i {
                self.stances.resize(i + 1, Stance::default());
            }
            let stance = &mut self.stances[i];
            stance.update(
                dt as f32,
                grade as f32,
                fig.speed as f32,
                fig.effort,
                fig.pedalling,
            );
            let stance = *stance;
            let j = fig.jersey;
            let pose = mesh::RiderPose {
                pos,
                fwd: fwd_r,
                up: up_r,
                right,
                crank: fig.crank,
                wheel: ((fig.d / 0.34) % std::f64::consts::TAU) as f32,
                jersey: srgb(j.r(), j.g(), j.b()),
                stand: stance.stand,
                tuck: stance.tuck,
                effort: fig.effort,
                time: (self.time % 3600.0) as f32,
            };
            let joints = mesh::joints(&pose);
            rider.extend(mesh::bike(&pose, &joints).v);
            skins.push((body::skinning(&self.rest_inverse, &joints), pose.jersey));
            rider_shadow.extend_from_slice(&[pos[0], pos[1] + 0.02, pos[2], 0.9]);
            if i == 0 {
                lamp_pos = add(add(pos, scale(up_r, 1.0)), scale(fwd_r, 0.7));
                lamp_dir = norm(add(scale(fwd_r, 14.0), scale(up_r, -1.1)));
            }
            if let Some(t) = &fig.tag {
                tags.push((add(pos, [0.0, 2.0, 0.0]), t.clone()));
            }
        }

        rider.extend(walls.v);

        // --- roadside posts and kilometre banners --------------------------
        let mut posts = Vec::new();
        let mut reflectors = Vec::new();
        let mut planks = Vec::new();
        let mut lettering = mesh::Mesh::default();
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
        for (ks, text) in route.banners(d - 10.0, d + 500.0) {
            let p = route.at(ks);
            for side in [-1.0, 1.0] {
                // Reaching well into the ground, wherever it lies.
                let pos = rp(
                    p.x + side * 5.4 * p.heading.cos(),
                    p.ele - 3.0,
                    p.z - side * 5.4 * p.heading.sin(),
                );
                posts
                    .extend_from_slice(&[pos[0], pos[1], pos[2], 0.0, 0.3, 8.3, 0.03, 0.03, 0.035]);
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
            // The lettering on the banner's face, towards the riders coming.
            let right = [p.heading.cos() as f32, 0.0, p.heading.sin() as f32];
            let face = [-(p.heading.sin() as f32), 0.0, p.heading.cos() as f32];
            // (The banner's plank is 0.03 thick before its width scales it.)
            let at = add(
                add(centre, [0.0, 0.6, 0.0]),
                scale(face, 0.03 * 11.1 + 0.02),
            );
            let height = if lettering::width(&text, 0.62) > 10.4 {
                0.5
            } else {
                0.62
            };
            lettering::paint(
                &mut lettering,
                &text,
                at,
                right,
                [0.0, 1.0, 0.0],
                height,
                [0.82, 0.82, 0.8],
            );
        }
        rider.extend(lettering.v);

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
                let (wx, wz) = (
                    p.x + lateral * p.heading.cos(),
                    p.z - lateral * p.heading.sin(),
                );
                // On the ground as drawn; none on a steep bank beside the road.
                let ground = self
                    .land
                    .as_ref()
                    .and_then(|l| l.ground(wx, wz))
                    .unwrap_or(p.ele - 0.3);
                if (ground - p.ele).abs() > 1.2 {
                    prev = None;
                    continue;
                }
                let pos = rp(wx, ground - 0.3, wz);
                posts.extend_from_slice(&[
                    pos[0],
                    pos[1],
                    pos[2],
                    -p.heading as f32,
                    0.13,
                    1.55,
                    wood[0],
                    wood[1],
                    wood[2],
                ]);
                if let Some(a) = prev {
                    let (dx, dz) = (pos[0] - a[0], pos[2] - a[2]);
                    let span = (dx * dx + dz * dz).sqrt();
                    let rot = (-dz).atan2(dx);
                    for rail in [0.8f32, 1.22] {
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
            skins,
            posts,
            reflectors,
            planks,
            rider_shadow,
            cam_vel,
            quality,
            render_scale: render_scale.clamp(0.25, 1.0),
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
            far,
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
            // Down well below any land seen in front of them.
            let y = -2500.0 + (top + 2500.0) * frac;
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
    /// How many of the instances (nearest first) cast a shadow into each
    /// cascade, for trees (see `split_scattered`).
    shadowed: [i32; 2],
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

/// How far a tree's shadow can reach along the ground with the sun at
/// `sun_y` (the sine of its height): the tallest trees are about 30 m.
fn shadow_reach(sun_y: f32) -> f32 {
    let s = sun_y.clamp(0.05, 1.0);
    (30.0 * (1.0 - s * s).sqrt() / s).min(400.0)
}

/// The noise texture's values: random, evenly spread, one byte per texel.
fn noise_values() -> Vec<u8> {
    (0..NOISE_SIZE * NOISE_SIZE)
        .map(|i| {
            // A well-mixed hash of the texel's index (splitmix64).
            let mut z = (i as u64).wrapping_add(0x9e37_79b9_7f4a_7c15);
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            ((z ^ (z >> 31)) >> 56) as u8
        })
        .collect()
}

/// The parts of the ground map to work out when the square it holds moves
/// from corner `old` to `new` (world metres; see `ground_pass`): the ground
/// newly inside, as texel rectangles (x, y, width, height) of at most
/// `GROUND_MAP_PIECE` texels each, wrapped round the map. None when it has
/// moved so far that all of it is new.
fn ground_map_todo(old: [i64; 2], new: [i64; 2]) -> Option<Vec<[i32; 4]>> {
    let size = GROUND_MAP_SIZE as i64;
    let (dx, dy) = (new[0] - old[0], new[1] - old[1]);
    if dx.abs() >= size || dy.abs() >= size {
        return None;
    }
    // World spans along one axis newly inside: [start, end).
    let band = |d: i64, old: i64, new: i64| {
        if d > 0 {
            Some((old + size, new + size))
        } else if d < 0 {
            Some((new, old))
        } else {
            None
        }
    };
    let mut world = Vec::new();
    if let Some(xs) = band(dx, old[0], new[0]) {
        world.push((xs, (new[1], new[1] + size)));
    }
    if let Some(ys) = band(dy, old[1], new[1]) {
        world.push(((new[0], new[0] + size), ys));
    }
    // A world span as one or two texel spans, wrapped round the map.
    let wrap = |(a, b): (i64, i64)| -> Vec<(i32, i32)> {
        let start = a.rem_euclid(size);
        let len = b - a;
        if start + len <= size {
            vec![(start as i32, len as i32)]
        } else {
            vec![
                (start as i32, (size - start) as i32),
                (0, (start + len - size) as i32),
            ]
        }
    };
    let mut out = Vec::new();
    for (xs, ys) in world {
        for (x, w) in wrap(xs) {
            for (y, h) in wrap(ys) {
                // In strips small enough to work out in a frame.
                let rows = (GROUND_MAP_PIECE / w as i64).clamp(1, h as i64) as i32;
                let mut y0 = y;
                while y0 < y + h {
                    let hh = rows.min(y + h - y0);
                    out.push([x, y0, w, hh]);
                    y0 += hh;
                }
            }
        }
    }
    Some(out)
}

/// Where the land window is centred: well ahead of the camera, so the
/// road is built far into the distance.
fn look_ahead(cx: f64, cz: f64, yaw: f64) -> (f64, f64) {
    (cx + yaw.sin() * 450.0, cz + yaw.cos() * 450.0)
}

/// Whether the land window centred at `origin` should be rebuilt around
/// `ahead`: once the point it was centred on has moved 320 m away. (The
/// camera itself is 450 m from a fresh window's centre, so measuring from it
/// rebuilt the land as soon as each window arrived.)
fn needs_new_window(ahead: (f64, f64), origin: (f64, f64)) -> bool {
    ((ahead.0 - origin.0).powi(2) + (ahead.1 - origin.1).powi(2)).sqrt() > 320.0
}

#[derive(Default)]
struct GlState {
    ready: bool,
    /// The rider's sculpted body, made on the first frame.
    skin: Option<Buffers>,
    skin_tried: bool,
    pub error: Option<String>,
    programs: HashMap<&'static str, Program>,
    sky_vao: Option<glow::VertexArray>,
    terrain: Option<Buffers>,
    /// The distant land (`FarData`), its vertices moved to the render
    /// origin they were uploaded for.
    far: Option<Buffers>,
    far_data: Option<Arc<FarData>>,
    far_origin: (f64, f64),
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
    /// Distant trees' cards (one per kind of tree) and their pictures
    /// (colour and normals), once made.
    cards: Vec<Buffers>,
    /// Random values for the ground's and road's noise (see `NOISE_TEX`).
    noise_tex: Option<glow::Texture>,
    /// Card shadows of trees out of view that would be cards in it.
    shadow_cards: Vec<Buffers>,
    card_tex: Option<Vec<[glow::Texture; 3]>>,
    card_tex_failed: bool,
    /// Each tree's level of detail, by where it stands in the world (in
    /// tenths of a metre), and the frame count for forgetting trees no
    /// longer in view.
    tree_states: HashMap<(i64, i64), TreeState>,
    lod_frame: u64,
    lod_clock: Option<std::time::Instant>,
    /// Grass tuft variants, each with its own share of the instances.
    grass: Vec<Buffers>,
    /// The sparse ring of grass around the dense patch.
    grass_ring: Vec<Buffers>,
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
    /// The ground's large-scale noise for the terrain window (`GROUND_MAP`),
    /// and the window origin it was worked out for.
    ground_map: Option<Target>,
    ground_map_failed: bool,
    /// The world corner of the square the ground map holds (whole metres),
    /// once it has been filled.
    ground_min: Option<[i64; 2]>,
    /// Parts of the ground map still to work out (texel rectangles: x, y,
    /// width, height), one a frame.
    ground_todo: Vec<[i32; 4]>,
    /// Its centre and half size in cloud-deck coordinates (0: no map).
    cloud_box: V3,
    /// The terrain's tiles, to draw only those in view.
    tiles: Vec<terrain::Tile>,
    /// Timing the passes on the GPU (for `--bench`).
    timing: bool,
    timers: Option<Timers>,
    timers_failed: bool,
    /// Milliseconds the GPU last spent on shadows, the scene and finishing.
    gpu_ms: Option<[f32; STAGES]>,
    /// What each stage of the last timed frame drew (see [`StageCounts`]),
    /// where the driver counts it.
    counts: Option<[StageCounts; STAGES]>,
    /// Milliseconds the last frame's drawing commands took to issue.
    paint_ms: f32,
    /// How many times the land has been uploaded (for `--bench`).
    land_uploads: u32,
    /// GPU timestamps around egui's drawing and the 3D view (for `--bench`).
    marks: Option<Marks>,
    marks_failed: bool,
    /// Milliseconds the GPU last spent outside the 3D view: drawing the
    /// rest of the screen (egui), and showing the frame.
    other_ms: Option<[f32; 2]>,
}

/// GPU timestamps for a few frames in flight: when egui starts drawing a
/// frame, when the 3D view starts and ends, and when egui is done.
struct Marks {
    q: Vec<[glow::Query; 4]>,
    /// Which of each frame's timestamps were recorded.
    issued: Vec<[bool; 4]>,
    frame: usize,
    /// When the last frame read back was done, to time its showing.
    last_end: Option<u64>,
}
const MARK_BEGIN: usize = 0;
const MARK_3D: usize = 1;
const MARK_3D_DONE: usize = 2;
const MARK_END: usize = 3;

/// Stages of a frame timed on the GPU: shadows, trees and objects, ground
/// and road (with the mountains), grass, the rest of the scene (shadow
/// blobs, sky, water, rain), and the finishing pass.
pub const STAGES: usize = 6;
const STAGE_FINISHING: usize = 5;

/// GPU timer queries for the stages of a frame, a few frames deep so
/// results are read without waiting for the GPU.
struct Timers {
    q: Vec<[glow::Query; STAGES]>,
    /// Counting queries alongside (none if the driver has none), one for
    /// each of `targets`.
    counted: Option<Vec<[[glow::Query; 3]; STAGES]>>,
    /// What is counted: triangles sent to be drawn, pixel samples that
    /// passed the depth test, and (where the driver has pipeline
    /// statistics) fragment shader runs.
    targets: Vec<u32>,
    used: Vec<bool>,
    frame: usize,
}

/// What one stage of a frame drew: the triangles it sent, the pixel
/// samples that passed the depth test (including ones later drawn over;
/// with multisampling, each sample counts), and how many times the
/// fragment shader ran (each pixel worked out, whether it was kept, drawn
/// over later or thrown away). Unlike the times, these are the same on any
/// graphics card.
#[derive(Clone, Copy, Default, Debug)]
pub struct StageCounts {
    pub triangles: u64,
    pub samples: u64,
    /// None where the driver has no pipeline statistics.
    pub fragments: Option<u64>,
}

/// Index runs of the terrain tiles that `vp` can see, merged where they
/// follow each other: (byte offset, count).
fn visible_runs(vp: &M4, tiles: &[terrain::Tile]) -> Vec<(i32, i32)> {
    let mut runs: Vec<(i32, i32)> = Vec::new();
    for t in tiles {
        // Outside if all eight corners are beyond the same clip plane.
        let mut out = [true; 6];
        for c in 0..8 {
            let p = [
                if c & 1 == 0 { t.lo[0] } else { t.hi[0] },
                if c & 2 == 0 { t.lo[1] } else { t.hi[1] },
                if c & 4 == 0 { t.lo[2] } else { t.hi[2] },
                1.0,
            ];
            let q = transform(vp, p);
            let tests = [
                q[0] < -q[3],
                q[0] > q[3],
                q[1] < -q[3],
                q[1] > q[3],
                q[2] < -q[3],
                q[2] > q[3],
            ];
            for k in 0..6 {
                out[k] &= tests[k];
            }
        }
        if out.iter().any(|&o| o) {
            continue;
        }
        let (first, count) = (t.first as i32 * 4, t.count as i32);
        match runs.last_mut() {
            Some(r) if r.0 + r.1 * 4 == first => r.1 += count,
            _ => runs.push((first, count)),
        }
    }
    runs
}

fn bytes(v: &[f32]) -> &[u8] {
    // SAFETY: f32 has no invalid bit patterns and u8 has alignment 1.
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

fn bytes_u32(v: &[u32]) -> &[u8] {
    // SAFETY: as above.
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

// The first `KINDS` match the kinds in `terrain`.
const OBJ_PINE: usize = terrain::PINE;
const OBJ_BROAD: usize = terrain::BROADLEAF;
const OBJ_ROCK: usize = terrain::ROCK;
const OBJ_HOUSE: usize = terrain::HOUSE;
const OBJ_BIRCH: usize = terrain::BIRCH;
const OBJ_BUSH: usize = terrain::BUSH;
const OBJ_BARN: usize = terrain::BARN;
const OBJ_SHED: usize = terrain::SHED;
const OBJ_TOWNHOUSE: usize = terrain::TOWNHOUSE;
const OBJ_OAK: usize = terrain::OAK;
const OBJ_MAPLE: usize = terrain::MAPLE;
const OBJ_BALE: usize = terrain::BALE;
/// Dry-stone walls along the fields, piece by piece.
const OBJ_WALL: usize = terrain::WALL;
const OBJ_POST: usize = KINDS;
const OBJ_PLANK: usize = KINDS + 1;
const OBJ_RIDER: usize = KINDS + 2;
const OBJ_PINE_FAR: usize = KINDS + 3;
const OBJ_BROAD_FAR: usize = KINDS + 4;
const OBJ_REFLECTOR: usize = KINDS + 5;
/// Each kind of tree, in the order of its cards, its meshes for trees
/// dissolving between levels and its meshes casting shadows only.
const TREES: [usize; 5] = [OBJ_PINE, OBJ_BROAD, OBJ_BIRCH, OBJ_OAK, OBJ_MAPLE];
const TREE_COUNT: usize = TREES.len();
/// The trees' meshes again, for trees dissolving into or out of cards.
const TREE_FADES: [usize; TREE_COUNT] = run(KINDS + 6);
/// Trees out of view whose shadow falls into it: drawn only into the shadows.
const TREE_SHADOWS: [usize; TREE_COUNT] = run(KINDS + 6 + TREE_COUNT);
/// Every tree object casting shadows: the trees, then fading, then
/// shadow-only ones.
const TREE_CASTERS: [usize; 3 * TREE_COUNT] = {
    let mut a = [0; 3 * TREE_COUNT];
    let mut i = 0;
    while i < TREE_COUNT {
        a[i] = TREES[i];
        a[TREE_COUNT + i] = TREE_FADES[i];
        a[2 * TREE_COUNT + i] = TREE_SHADOWS[i];
        i += 1;
    }
    a
};
/// `N` object numbers in a row from `first`.
const fn run<const N: usize>(first: usize) -> [usize; N] {
    let mut a = [0; N];
    let mut i = 0;
    while i < N {
        a[i] = first + i;
        i += 1;
    }
    a
}
/// The full mesh of tree `t` (in the order of `TREES`).
fn tree_mesh(t: usize) -> mesh::Mesh {
    [
        mesh::pine,
        mesh::broadleaf,
        mesh::birch,
        mesh::oak,
        mesh::maple,
    ][t]()
}
/// The farm buildings, drawn like the houses.
const BUILDINGS: [usize; 4] = [OBJ_HOUSE, OBJ_BARN, OBJ_SHED, OBJ_TOWNHOUSE];
/// Distances where trees change level of detail (`Quality::tree_lod`).
#[derive(Clone, Copy)]
struct TreeLod {
    card: f32,
}
/// How far each tree's own distances are shifted, at most, so trees
/// change level one at a time rather than along a line.
const LOD_JITTER: f32 = 6.0;
/// A tree changes level only this far past its distance, and changes back
/// only this far inside it, so it never flickers between the two.
const LOD_SLACK: f32 = 2.5;
/// Seconds a tree takes to dissolve from one level into the next.
const LOD_FADE_SECS: f32 = 0.45;
/// How far past its share a distant tree must be to be thinned out (or to
/// come back), so it never flickers.
const THIN_SLACK: f32 = 0.02;
/// Floats per instance of a tree changing level, and of a card: an
/// object's instance and the shares faded in and out (`LOD_FS`).
const FADE_STRIDE: usize = INSTANCE_STRIDE + 2;
const CARD_STRIDE: usize = FADE_STRIDE;

/// Which level of detail a tree is drawn at (`FULL`, `CARD` or `THINNED`),
/// the one it is leaving, and since when.
struct TreeState {
    level: u8,
    from: u8,
    since: f32,
    seen: u64,
}

/// A tree's levels of detail: its full mesh, a card, or thinned out of
/// distant forest (not drawn). There is no level between the mesh and the
/// card: a simpler mesh changing places with the full one showed, however
/// alike they were made.
const FULL: u8 = 0;
const CARD: u8 = 1;
const THINNED: u8 = 2;

/// What decides a tree's level of detail: its own distance to becoming a
/// card, how far it is across the ground (`d`) and in all (`dist`), and
/// `thin`: its share against the share of trees kept at that distance
/// (above 0, it is thinned out).
struct Reach {
    to_card: f32,
    thin: f32,
    d: f32,
    dist: f32,
}

/// The share of distant trees kept at `dist` metres: all of them up to
/// `THIN_FROM`, then fewer, down to 40% at the far edge.
fn keep_share(dist: f32) -> f32 {
    1.0 - 0.6 * ((dist - THIN_FROM) / (FAR_CULL - THIN_FROM)).clamp(0.0, 1.0)
}

/// The level of detail for a tree first seen (back in view, or new).
fn first_level(r: &Reach) -> u8 {
    if r.thin > 0.0 || r.dist > FAR_CULL - 60.0 {
        THINNED
    } else if r.d < r.to_card {
        FULL
    } else {
        CARD
    }
}

/// The level a tree at `level` moves to: it moves on only `LOD_SLACK` past
/// its distance and back only as far inside it, and is thinned out (or comes
/// back) only `THIN_SLACK` past its share, so it never flickers.
fn next_level(level: u8, r: &Reach) -> u8 {
    let mut l = level;
    loop {
        l = match l {
            FULL if r.d > r.to_card + LOD_SLACK => CARD,
            CARD if r.d < r.to_card - LOD_SLACK => FULL,
            CARD if r.thin > THIN_SLACK || r.dist > FAR_CULL - 40.0 => THINNED,
            THINNED if r.thin < -THIN_SLACK && r.dist < FAR_CULL - 80.0 => CARD,
            _ => return l,
        };
    }
}

/// A number in 0..1 from where a tree stands.
fn unit_hash(key: (i64, i64)) -> f32 {
    let mut h = (key.0 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (key.1 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    (h >> 40) as f32 / (1u64 << 24) as f32
}
/// Texture units for the cards' pictures.
const UNIT_CARD_ALBEDO: u32 = 9;
const UNIT_CARD_NORMAL: u32 = 10;
const UNIT_CARD_DEPTH: u32 = 11;
const UNIT_NOISE: u32 = 12;
/// Width and height of the noise texture (texels): its pattern repeats
/// every this many cells of the finest noise, 24 m of tarmac grain.
const NOISE_SIZE: usize = 1024;

/// Objects further than this are hidden in the haze anyway.
const FAR_CULL: f32 = 1600.0;
/// Distant forest thins out from here on.
const THIN_FROM: f32 = 400.0;
/// Bushes and rocks are too small to see beyond this.
const SMALL_CULL: f32 = 600.0;
/// Things this close are kept even behind the camera: their shadows may
/// fall into view.
const KEEP_AROUND: f32 = 150.0;

/// Side of the square of grass that follows the camera, in metres.
const GRASS_PATCH: f32 = 60.0;
/// The sparse grass that reaches beyond the dense patch (and grows under
/// it): the size of the patch it is spread over (centred with the dense
/// one), and its tufts as a share of the dense patch's. The dense patch
/// gives up `GRASS_DENSE_SHARE` of its tufts to make room for it.
const GRASS_RING: f32 = 104.0;
const GRASS_RING_SHARE: f32 = 0.55;
const GRASS_DENSE_SHARE: f32 = 0.87;

/// Where the grass patch's world grid sits relative to the terrain
/// window's origin: the origin in the grass shader's (x, -z) frame, as
/// whole patches and the remainder.
fn grass_anchor(origin: (f64, f64), patch: f64) -> ((f32, f32), (f32, f32)) {
    let s = (origin.0, -origin.1);
    let lap = ((s.0 / patch).floor(), (s.1 / patch).floor());
    (
        ((s.0 - lap.0 * patch) as f32, (s.1 - lap.1 * patch) as f32),
        (lap.0 as f32, lap.1 as f32),
    )
}
const RAIN_DROPS: usize = 6000;
/// Texture units.
const UNIT_HEIGHT: u32 = 4;
const UNIT_SM0: u32 = 5;
const UNIT_SM1: u32 = 6;
const UNIT_CLOUD: u32 = 7;
const UNIT_GROUND: u32 = 8;
/// The ground map's texels per side: about a metre each across the 2 km
/// terrain window, finer than the smallest noise it holds (2.2 m).
const GROUND_MAP_SIZE: i32 = 2048;
/// The most of the ground map worked out in one frame (texels).
const GROUND_MAP_PIECE: i64 = 256 * 1024;
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
            self.compile(gl, "ground_map", &[SCREEN_VS], &[NOISE, GROUND_MAP_FS], &[])?;
            // The ground and road read their noise from a texture.
            let noise_tex = format!("#define NOISE_TEX\n#define NOISE_SIZE {NOISE_SIZE}.0\n");
            self.compile(
                gl,
                "terrain",
                &[TERRAIN_VS],
                &[&noise_tex, NOISE, LIGHT, GROUND_MAP, TERRAIN_FS],
                &terrain_attribs,
            )?;
            self.compile(
                gl,
                "terrain_far",
                &[TERRAIN_VS],
                &[FAR_LAND, &noise_tex, NOISE, LIGHT, GROUND_MAP, TERRAIN_FS],
                &terrain_attribs,
            )?;
            self.compile(
                gl,
                "road",
                &[ROAD_VS],
                &[&noise_tex, NOISE, LIGHT, GROUND_MAP, ROAD_FS],
                &["a_pos", "a_uv"],
            )?;
            self.noise_tex = Some(Self::noise_texture(gl)?);
            self.compile(
                gl,
                "object",
                &[NOISE, WIND, OBJECT_VS],
                &[NOISE, LIGHT, FOLIAGE, OBJECT_FS],
                &object_attribs,
            )?;
            let mut fade_attribs = object_attribs.to_vec();
            fade_attribs.push("i_lod");
            self.compile(
                gl,
                "object_fade",
                &[LOD_FADE, NOISE, WIND, OBJECT_VS],
                &[LOD_FADE, NOISE, LIGHT, FOLIAGE, LOD_FS, OBJECT_FS],
                &fade_attribs,
            )?;
            let card_attribs = ["a_corner", "i_pos_rot", "i_scale", "i_tint", "i_lod"];
            self.compile(
                gl,
                "card",
                &[NOISE, WIND, IMPOSTOR_VS],
                &[NOISE, LIGHT, FOLIAGE, LOD_FS, IMPOSTOR_SURFACE, IMPOSTOR_FS],
                &card_attribs,
            )?;
            self.compile(
                gl,
                "sh_card",
                &[NOISE, WIND, IMPOSTOR_VS],
                &[IMPOSTOR_SURFACE, IMPOSTOR_DEPTH_FS],
                &card_attribs,
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
                &[NOISE, WIND, GROUND_MAP, GRASS_VS],
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
            self.compile(
                gl,
                "sh_object",
                &[NOISE, WIND, OBJECT_VS],
                &[DEPTH_FS],
                &object_attribs,
            )?;
            let skinned_attribs = ["a_pos", "a_nrm", "a_col", "a_mat", "a_bones", "a_weights"];
            self.compile(
                gl,
                "skinned",
                &[SKINNED_VS],
                &[NOISE, LIGHT, FOLIAGE, OBJECT_FS],
                &skinned_attribs,
            )?;
            self.compile(
                gl,
                "sh_skinned",
                &[SKINNED_VS],
                &[DEPTH_FS],
                &skinned_attribs,
            )?;
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
            let mut meshes: Vec<mesh::Mesh> = (0..KINDS)
                .map(|kind| match kind {
                    OBJ_ROCK => mesh::rock(),
                    OBJ_HOUSE => buildings::farmhouse(),
                    OBJ_BUSH => mesh::bush(),
                    OBJ_BARN => buildings::barn(),
                    OBJ_SHED => buildings::shed(),
                    OBJ_TOWNHOUSE => buildings::townhouse(),
                    OBJ_BALE => fields::hay_bale(),
                    OBJ_WALL => fields::stone_wall(),
                    tree => tree_mesh(TREES.iter().position(|&k| k == tree).expect("a tree")),
                })
                .collect();
            meshes.extend([
                mesh::post(),
                mesh::plank(),
                mesh::Mesh::default(),
                mesh::pine_far(),
                mesh::broadleaf_far(),
                mesh::reflector(),
            ]);
            meshes.extend((0..TREE_COUNT).map(tree_mesh));
            meshes.extend((0..TREE_COUNT).map(tree_mesh));
            debug_assert_eq!(meshes.len(), TREE_SHADOWS[TREE_COUNT - 1] + 1);
            for (i, m) in meshes.iter().enumerate() {
                // Trees changing level carry how far they are through it.
                let fading = TREE_FADES.contains(&i);
                let instance: &[(i32, i32)] = if fading {
                    &[(4, 0), (2, 4), (3, 6), (2, 9)]
                } else {
                    &[(4, 0), (2, 4), (3, 6)]
                };
                let mut b = Self::buffers(
                    gl,
                    &[(3, 0), (3, 3), (3, 6), (1, 9)],
                    mesh::VERTEX_STRIDE as i32,
                    &m.v,
                    Some(instance),
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
            // Cards: a quad from the foot of the tree up, one per tree.
            let card = [
                -1.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 1.0,
            ];
            for _ in 0..TREE_FADES.len() {
                let mut b = Self::buffers(
                    gl,
                    &[(2, 0)],
                    2,
                    &card,
                    Some(&[(4, 0), (2, 4), (3, 6), (2, 9)]),
                )?;
                b.count = 6;
                self.cards.push(b);
                let mut b = Self::buffers(
                    gl,
                    &[(2, 0)],
                    2,
                    &card,
                    Some(&[(4, 0), (2, 4), (3, 6), (2, 9)]),
                )?;
                b.count = 6;
                self.shadow_cards.push(b);
            }
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
            for b in self.grass.drain(..).chain(self.grass_ring.drain(..)) {
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
                let n = (total as f32 * GRASS_DENSE_SHARE * share) as usize;
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
            // The sparse grass: fewer tufts of fewer blades, spread over a
            // patch nearly twice the size, under and beyond the dense one.
            let ring = (total as f32 * GRASS_RING_SHARE) as usize;
            for (verts, share) in [
                (mesh::grass_tuft(4, 0, false, 5), 0.65),
                (mesh::grass_tuft(5, 0, true, 6), 0.35),
            ] {
                let n = (ring as f32 * share) as usize;
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
                        next() * GRASS_RING,
                        next() * GRASS_RING,
                        next() * std::f32::consts::TAU,
                        next(),
                    ]);
                }
                Self::set_instances(gl, &mut b, &seeds, 4);
                self.grass_ring.push(b);
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
                shadowed: [0; 2],
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
    /// Sort the scattered objects in and around the view into what is drawn
    /// how: trees close by as full meshes, further off as middle ones, far
    /// away as cards, and trees in the middle of changing level as both
    /// (they dissolve, see `LOD_FS`); bushes, rocks and houses as they are.
    unsafe fn split_scattered(&mut self, gl: &glow::Context, f: &FrameData) {
        let fwd = f.fwd;
        // How far a world-space radius reaches in clip x and y, for keeping
        // objects that overlap the edge of the view.
        let m = &f.vp;
        let reach_x = len([m[0], m[4], m[8]]);
        let reach_y = len([m[1], m[5], m[9]]);
        let lod = f.quality.tree_lod();
        let cards = self.card_tex.is_some();
        // Dissolves take real time, whatever the frame rate.
        let now = self
            .lod_clock
            .get_or_insert_with(std::time::Instant::now)
            .elapsed()
            .as_secs_f32();
        self.lod_frame += 1;
        let frame = self.lod_frame;
        let mut states = std::mem::take(&mut self.tree_states);
        let mut near: [Vec<f32>; KINDS] = Default::default();
        let mut near_fade: [Vec<f32>; TREE_COUNT] = Default::default();
        let mut far: [Vec<f32>; 2] = Default::default();
        let mut card: [Vec<f32>; TREE_COUNT] = Default::default();
        let mut shadow_card: [Vec<f32>; TREE_COUNT] = Default::default();
        let mut shadow_mesh: [Vec<f32>; TREE_COUNT] = Default::default();
        // Whether something of radius `r` at `p` is in view (or partly).
        let in_view = |p: V3, r: f32| {
            if dot(sub(p, f.cam), fwd) < -r {
                return false;
            }
            let q = transform(m, [p[0], p[1], p[2], 1.0]);
            q[0].abs() <= q[3] + r * reach_x && q[1].abs() <= q[3] + r * reach_y
        };
        // Which way and how far shadows fall along the ground, per metre of
        // height.
        let sun_flat = norm([f.sun[0], 0.0, f.sun[2]]);
        let per_metre = shadow_reach(f.sun[1]) / 30.0;
        // Whether the shadow of something `h` tall standing at `p` can fall
        // into view: its foot, middle or tip.
        let shadow_in_view = |p: V3, h: f32, r: f32| {
            let s = scale(sun_flat, -(h * per_metre).min(400.0));
            [0.0, 0.5, 1.0]
                .into_iter()
                .any(|k| in_view(add(p, scale(s, k)), r))
        };
        for (kind, list) in self.scattered.iter().enumerate() {
            let tree = TREES.iter().position(|&k| k == kind);
            let small = matches!(kind, OBJ_BUSH | OBJ_ROCK | OBJ_BALE | OBJ_WALL);
            for inst in list.as_chunks::<INSTANCE_STRIDE>().0 {
                let rel = sub([inst[0], inst[1], inst[2]], f.cam);
                let dist = len(rel);
                if dist > FAR_CULL || (small && dist > SMALL_CULL) {
                    continue;
                }
                // How far it reaches: trees carry their height, buildings
                // only a scale (they are modelled in metres, up to about
                // 12 m from their middle).
                let r = if BUILDINGS.contains(&kind) {
                    12.0 * inst[4]
                } else if kind == OBJ_WALL {
                    fields::WALL_PIECE * 0.6
                } else {
                    inst[5]
                };
                let foot = [inst[0], inst[1], inst[2]];
                if !in_view([inst[0], inst[1] + 0.5 * r, inst[2]], r) {
                    // Out of view, only its shadow can show: close by, and
                    // only if it falls into view.
                    let wide = if BUILDINGS.contains(&kind) {
                        r
                    } else {
                        inst[4].max(1.0)
                    };
                    if dist > KEEP_AROUND || !shadow_in_view(foot, r * 1.15, wide) {
                        continue;
                    }
                    // Trees cast it without being drawn in the view: the same
                    // shadow as if they were (full trees close by, a card's
                    // further out), so none changes as it comes into view.
                    let full = (inst[0] - f.cam[0]).hypot(inst[2] - f.cam[2]) < lod.card;
                    match tree {
                        Some(t) if full || !cards => shadow_mesh[t].extend_from_slice(inst),
                        Some(t) => {
                            shadow_card[t].extend_from_slice(inst);
                            shadow_card[t].extend_from_slice(&[1.0, 0.0]);
                        }
                        // The few houses and rocks as they are.
                        None => near[kind].extend_from_slice(inst),
                    }
                    continue;
                }
                let Some(t) = tree else {
                    near[kind].extend_from_slice(inst);
                    continue;
                };
                // Each tree's level, kept from frame to frame by where it
                // stands in the world, with distances of its own.
                let d = (rel[0] * rel[0] + rel[2] * rel[2]).sqrt();
                let key = (
                    ((inst[0] as f64 + f.origin.0) * 10.0).round() as i64,
                    ((f.origin.1 - inst[2] as f64) * 10.0).round() as i64,
                );
                let shift = (unit_hash(key) - 0.5) * 2.0 * LOD_JITTER;
                let reach = Reach {
                    to_card: lod.card + shift,
                    // Thin distant forest: in the haze fewer, slightly bigger
                    // trees look the same, and masses of tiny trees are slow
                    // to draw. Which trees stay depends only on where they
                    // stand in the world; with some slack, so none flickers.
                    thin: unit_hash((key.1, key.0 ^ 0x5bd1)) - keep_share(dist),
                    d,
                    dist,
                };
                let st = states.entry(key).or_insert_with(|| {
                    let l = first_level(&reach);
                    TreeState {
                        level: l,
                        from: l,
                        since: f32::NEG_INFINITY,
                        seen: 0,
                    }
                });
                st.seen = frame;
                let l = next_level(st.level, &reach);
                if l != st.level {
                    st.from = st.level;
                    st.level = l;
                    st.since = now;
                }
                let progress = ((now - st.since) / LOD_FADE_SECS).clamp(0.0, 1.0);
                // Without cards, distant trees switch to the simple meshes.
                let fading = progress < 1.0 && st.from != st.level && cards;
                let keep = keep_share(dist);
                let mut grown = *inst;
                let g = keep.sqrt().recip();
                for k in [4, 5] {
                    grown[k] *= g;
                }
                let mut place = |level: u8, fin: f32, fout: f32| {
                    let solid = fin >= 1.0 && fout <= 0.0;
                    match level {
                        FULL if solid => near[kind].extend_from_slice(inst),
                        FULL => {
                            near_fade[t].extend_from_slice(inst);
                            near_fade[t].extend_from_slice(&[fin, fout]);
                        }
                        CARD if cards => {
                            card[t].extend_from_slice(&grown);
                            card[t].extend_from_slice(&[fin, fout]);
                        }
                        CARD => far[if kind == OBJ_PINE { 0 } else { 1 }].extend_from_slice(&grown),
                        // Thinned out: not drawn.
                        _ => {}
                    }
                };
                if fading {
                    place(st.from, 1.0, progress);
                    place(st.level, progress, 0.0);
                } else {
                    place(st.level, 1.0, 0.0);
                }
            }
        }
        // Forget trees no longer in view: back in view, they start settled.
        states.retain(|_, s| s.seen == frame);
        self.tree_states = states;
        // Nearest first, so what they hide is not worked out at all. Trees
        // go in two runs, each nearest first: those that cast a shadow into
        // the near cascade, then the rest, so that cascade draws only the
        // first run (see `shadowed`).
        let near_vp = f.light_vp[0];
        let r_unit = 1.0 / CASCADES[0].1;
        let order = |list: &mut Vec<f32>, stride: usize, split: bool| -> [i32; 2] {
            let mut order: Vec<(bool, f32, usize)> = list
                .chunks_exact(stride)
                .enumerate()
                .map(|(i, inst)| {
                    let foot = [inst[0], inst[1], inst[2]];
                    let rel = sub(foot, f.cam);
                    let far = split && !casts_into(&near_vp, foot, inst[5] * 1.15, inst[4], r_unit);
                    (far, dot(rel, rel), i)
                })
                .collect();
            order.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
            let casters = order.iter().take_while(|o| !o.0).count() as i32;
            let sorted: Vec<f32> = order
                .iter()
                .flat_map(|&(_, _, i)| list[i * stride..(i + 1) * stride].iter().copied())
                .collect();
            *list = sorted;
            [casters, (list.len() / stride) as i32]
        };
        let mut shadowed = [[0i32; 2]; KINDS];
        for (kind, list) in near.iter_mut().enumerate() {
            let tree = TREES.contains(&kind);
            shadowed[kind] = order(list, INSTANCE_STRIDE, tree);
        }
        for list in far.iter_mut() {
            order(list, INSTANCE_STRIDE, false);
        }
        let fade_shadowed: Vec<[i32; 2]> = near_fade
            .iter_mut()
            .map(|list| order(list, FADE_STRIDE, true))
            .collect();
        let mesh_shadowed: Vec<[i32; 2]> = shadow_mesh
            .iter_mut()
            .map(|list| order(list, INSTANCE_STRIDE, true))
            .collect();
        for list in card.iter_mut() {
            order(list, FADE_STRIDE, false);
        }
        unsafe {
            for (kind, list) in near.iter().enumerate() {
                Self::set_instances(gl, &mut self.objects[kind], list, INSTANCE_STRIDE);
                self.objects[kind].shadowed = shadowed[kind];
            }
            for (t, &obj) in TREE_FADES.iter().enumerate() {
                Self::set_instances(gl, &mut self.objects[obj], &near_fade[t], FADE_STRIDE);
                self.objects[obj].shadowed = fade_shadowed[t];
                Self::set_instances(gl, &mut self.cards[t], &card[t], CARD_STRIDE);
                Self::set_instances(gl, &mut self.shadow_cards[t], &shadow_card[t], CARD_STRIDE);
                let obj = TREE_SHADOWS[t];
                Self::set_instances(gl, &mut self.objects[obj], &shadow_mesh[t], INSTANCE_STRIDE);
                self.objects[obj].shadowed = mesh_shadowed[t];
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

    /// Hand the cards' pictures to the graphics card once they are made,
    /// with all their mipmap levels.
    unsafe fn ensure_card_textures(&mut self, gl: &glow::Context) {
        if self.card_tex.is_some() || self.card_tex_failed {
            return;
        }
        let Some(atlases) = CARDS.get() else {
            return;
        };
        unsafe {
            Self::drain_errors(gl);
            let mut made = Vec::new();
            for a in atlases {
                let mut set = [None, None, None];
                for (k, slot) in set.iter_mut().enumerate() {
                    let tex = match gl.create_texture() {
                        Ok(t) => t,
                        Err(_) => {
                            self.card_tex_failed = true;
                            return;
                        }
                    };
                    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                    gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
                    for (level, l) in a.levels.iter().enumerate() {
                        let (data, internal, format) = match k {
                            0 => (&l.albedo, glow::RGBA8, glow::RGBA),
                            1 => (&l.normal, glow::RGBA8, glow::RGBA),
                            _ => (&l.depth, glow::R8, glow::RED),
                        };
                        gl.tex_image_2d(
                            glow::TEXTURE_2D,
                            level as i32,
                            internal as i32,
                            l.size as i32,
                            l.size as i32,
                            0,
                            format,
                            glow::UNSIGNED_BYTE,
                            glow::PixelUnpackData::Slice(Some(data)),
                        );
                    }
                    gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
                    for (p, v) in [
                        (glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR),
                        (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                        (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                        (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
                    ] {
                        gl.tex_parameter_i32(glow::TEXTURE_2D, p, v as i32);
                    }
                    *slot = Some(tex);
                }
                made.push([set[0].unwrap(), set[1].unwrap(), set[2].unwrap()]);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            if gl.get_error() != glow::NO_ERROR || made.len() != TREE_COUNT {
                self.card_tex_failed = true;
                return;
            }
            self.card_tex = Some(made);
        }
    }

    /// Draw the cards of distant trees, with `program` ("card", or
    /// "sh_card" facing the sun for shadows).
    unsafe fn draw_cards(&mut self, gl: &glow::Context, program: &'static str, face: V3) {
        let Some(tex) = self.card_tex.clone() else {
            return;
        };
        unsafe {
            let p = self.programs.get_mut(program).unwrap();
            p.v3(gl, "u_face", face);
            p.i1(gl, "u_imp_albedo", UNIT_CARD_ALBEDO as i32);
            p.i1(gl, "u_imp_normal", UNIT_CARD_NORMAL as i32);
            p.i1(gl, "u_imp_depth", UNIT_CARD_DEPTH as i32);
            for (b, set) in self.cards.iter().zip(tex) {
                if b.instances == 0 {
                    continue;
                }
                for (unit, t) in [UNIT_CARD_ALBEDO, UNIT_CARD_NORMAL, UNIT_CARD_DEPTH]
                    .into_iter()
                    .zip(set)
                {
                    gl.active_texture(glow::TEXTURE0 + unit);
                    gl.bind_texture(glow::TEXTURE_2D, Some(t));
                }
                gl.bind_vertex_array(Some(b.vao));
                gl.draw_arrays_instanced(glow::TRIANGLES, 0, b.count, b.instances);
            }
            gl.active_texture(glow::TEXTURE0);
        }
    }

    /// The noise texture: a random value per texel, repeating at its edges.
    unsafe fn noise_texture(gl: &glow::Context) -> Result<glow::Texture, String> {
        unsafe {
            let tex = gl.create_texture()?;
            gl.active_texture(glow::TEXTURE0 + UNIT_NOISE);
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::R8 as i32,
                NOISE_SIZE as i32,
                NOISE_SIZE as i32,
                0,
                glow::RED,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&noise_values())),
            );
            for (k, v) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::REPEAT),
                (glow::TEXTURE_WRAP_T, glow::REPEAT),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, k, v as i32);
            }
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            gl.active_texture(glow::TEXTURE0);
            Ok(tex)
        }
    }

    /// Upload the distant land, its vertices moved to the render `origin`
    /// (the detailed window's centre) and a little lower, so where the two
    /// overlap the detailed land is the one seen.
    unsafe fn upload_far(&mut self, gl: &glow::Context, origin: (f64, f64)) -> Result<(), String> {
        let Some(far) = self.far_data.clone() else {
            return Ok(());
        };
        let (dx, dz) = ((far.ox - origin.0) as f32, -(far.oz - origin.1) as f32);
        let mut verts = far.verts.clone();
        for v in verts.as_chunks_mut::<TERRAIN_STRIDE>().0 {
            v[0] += dx;
            v[1] -= 0.4;
            v[2] += dz;
        }
        unsafe {
            if self.far.is_none() {
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
                gl.buffer_data_u8_slice(
                    glow::ELEMENT_ARRAY_BUFFER,
                    bytes_u32(&far.indices),
                    glow::STATIC_DRAW,
                );
                gl.bind_vertex_array(None);
                b.ebo = Some(ebo);
                b.count = far.indices.len() as i32;
                self.far = Some(b);
            }
            let b = self.far.as_ref().unwrap();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(b.vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(&verts), glow::STATIC_DRAW);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
        }
        self.far_origin = origin;
        Ok(())
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
            // Every window has the same triangles: send them once.
            if b.count != t.indices.len() as i32 {
                gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, b.ebo);
                gl.buffer_data_u8_slice(
                    glow::ELEMENT_ARRAY_BUFFER,
                    bytes_u32(&t.indices),
                    glow::STATIC_DRAW,
                );
            }
            gl.bind_vertex_array(None);
            b.count = t.indices.len() as i32;
            self.tiles = t.tiles.clone();
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
                    // Out over the distant land too.
                    (terrain::FAR_SIZE / 2.0) as f32
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
            p.v3(gl, "u_eye", f.cam);
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
            p.i1(gl, "u_ground", UNIT_GROUND as i32);
            p.i1(gl, "u_noise", UNIT_NOISE as i32);
            let ground = self.ground_map.is_some() && self.ground_min.is_some();
            p.f1(
                gl,
                "u_ground_half",
                if ground { self.height_map[0] } else { 0.0 },
            );
            let strength = if f.quality == Quality::Low {
                0.0
            } else {
                l.shadow
            };
            gl.uniform_4_f32(p.loc(gl, "u_sm"), strength, texels[0], texels[1], inv_res);
        }
    }

    /// Draw each rider's body with `program` (bound, its other uniforms
    /// set): the sculpted mesh bent by the rider's bones.
    unsafe fn draw_skins(&mut self, gl: &glow::Context, f: &FrameData, program: &'static str) {
        let Some((vao, count)) = self.skin.as_ref().map(|b| (b.vao, b.count)) else {
            return;
        };
        unsafe {
            let p = self.programs.get_mut(program).unwrap();
            for (bones, tint) in &f.skins {
                let flat: Vec<f32> = bones.iter().flatten().copied().collect();
                gl.uniform_matrix_4_f32_slice(p.loc(gl, "u_bones"), false, &flat);
                p.v3(gl, "u_tint", *tint);
                gl.bind_vertex_array(Some(vao));
                gl.draw_elements(glow::TRIANGLES, count, glow::UNSIGNED_INT, 0);
            }
        }
    }

    /// Sculpt the rider's body and hand it to the graphics card.
    unsafe fn upload_skin(gl: &glow::Context, skin: &body::Skin) -> Result<Buffers, String> {
        unsafe {
            let mut b = Self::buffers(
                gl,
                &[(3, 0), (3, 3), (3, 6), (1, 9), (4, 10), (4, 14)],
                body::STRIDE as i32,
                &skin.vertices,
                None,
            )?;
            let ebo = gl.create_buffer()?;
            gl.bind_vertex_array(Some(b.vao));
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                bytes_u32(&skin.indices),
                glow::STATIC_DRAW,
            );
            gl.bind_vertex_array(None);
            b.ebo = Some(ebo);
            b.count = skin.indices.len() as i32;
            Ok(b)
        }
    }

    /// Draw instanced objects with the program that is bound.
    /// The terrain tiles `vp` can see (all `count` indices without tiles).
    unsafe fn draw_terrain(&self, gl: &glow::Context, vp: &M4, count: i32) {
        unsafe {
            if self.tiles.is_empty() {
                gl.draw_elements(glow::TRIANGLES, count, glow::UNSIGNED_INT, 0);
                return;
            }
            for (offset, n) in visible_runs(vp, &self.tiles) {
                gl.draw_elements(glow::TRIANGLES, n, glow::UNSIGNED_INT, offset);
            }
        }
    }

    /// Start timing stage `k` of this frame (see [`STAGES`]); `k == 0` also
    /// collects the results from a few frames ago.
    unsafe fn time_stage(&mut self, gl: &glow::Context, k: usize) {
        if !self.timing || self.timers_failed {
            return;
        }
        unsafe {
            if self.timers.is_none() {
                let v = gl.version();
                if v.is_embedded || (v.major, v.minor) < (3, 3) {
                    self.timers_failed = true;
                    return;
                }
                let mut q = Vec::new();
                for _ in 0..4 {
                    let mut set = Vec::new();
                    for _ in 0..STAGES {
                        match gl.create_query() {
                            Ok(x) => set.push(x),
                            Err(_) => {
                                self.timers_failed = true;
                                return;
                            }
                        }
                    }
                    q.push(std::array::from_fn(|i| set[i]));
                }
                let mut targets = vec![glow::PRIMITIVES_GENERATED, glow::SAMPLES_PASSED];
                if (v.major, v.minor) >= (4, 6)
                    || gl
                        .supported_extensions()
                        .contains("GL_ARB_pipeline_statistics_query")
                {
                    targets.push(glow::FRAGMENT_SHADER_INVOCATIONS);
                }
                let mut counted = Some(Vec::new());
                for _ in 0..q.len() {
                    let mut set = Vec::new();
                    for _ in 0..STAGES * 3 {
                        match gl.create_query() {
                            Ok(x) => set.push(x),
                            Err(_) => break,
                        }
                    }
                    if set.len() < STAGES * 3 {
                        counted = None;
                        break;
                    }
                    if let Some(c) = counted.as_mut() {
                        c.push(std::array::from_fn(|i| {
                            [set[3 * i], set[3 * i + 1], set[3 * i + 2]]
                        }));
                    }
                }
                self.timers = Some(Timers {
                    used: vec![false; q.len()],
                    q,
                    counted,
                    targets,
                    frame: 0,
                });
            }
            let t = self.timers.as_mut().unwrap();
            if k > 0 {
                gl.end_query(glow::TIME_ELAPSED);
                if t.counted.is_some() {
                    for &target in &t.targets {
                        gl.end_query(target);
                    }
                }
            } else {
                t.frame = (t.frame + 1) % t.q.len();
                let slot = t.frame;
                if t.used[slot] {
                    let set = t.q[slot];
                    if gl.get_query_parameter_u32(set[STAGES - 1], glow::QUERY_RESULT_AVAILABLE)
                        != 0
                    {
                        let ms = set.map(|q| {
                            gl.get_query_parameter_u64(q, glow::QUERY_RESULT) as f32 / 1.0e6
                        });
                        self.gpu_ms = Some(ms);
                        let stats = t.targets.len() > 2;
                        self.counts = t.counted.as_ref().map(|c| {
                            c[slot].map(|[tri, smp, frag]| StageCounts {
                                triangles: gl.get_query_parameter_u64(tri, glow::QUERY_RESULT),
                                samples: gl.get_query_parameter_u64(smp, glow::QUERY_RESULT),
                                fragments: stats
                                    .then(|| gl.get_query_parameter_u64(frag, glow::QUERY_RESULT)),
                            })
                        });
                    }
                }
                t.used[slot] = true;
            }
            let q = self.timers.as_ref().unwrap();
            gl.begin_query(glow::TIME_ELAPSED, q.q[q.frame][k]);
            if let Some(c) = &q.counted {
                for (&target, query) in q.targets.iter().zip(c[q.frame][k]) {
                    gl.begin_query(target, query);
                }
            }
        }
    }

    /// End the last stage of this frame.
    unsafe fn time_end(&mut self, gl: &glow::Context) {
        if self.timing && !self.timers_failed {
            if let Some(t) = &self.timers {
                unsafe {
                    gl.end_query(glow::TIME_ELAPSED);
                    if t.counted.is_some() {
                        for &target in &t.targets {
                            gl.end_query(target);
                        }
                    }
                }
            }
        }
    }

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

    /// Draw the instances that cast a shadow into cascade `k`.
    unsafe fn draw_shadowed(&self, gl: &glow::Context, which: &[usize], k: usize) {
        unsafe {
            for &i in which {
                let b = &self.objects[i];
                let n = b.shadowed[k].min(b.instances);
                if n > 0 && b.count > 0 {
                    gl.bind_vertex_array(Some(b.vao));
                    gl.draw_arrays_instanced(glow::TRIANGLES, 0, b.count, n);
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
                    self.draw_terrain(gl, &f.light_vp[k], count);
                }
                let p = self.programs.get_mut("sh_object").unwrap();
                gl.use_program(Some(p.id));
                p.m4(gl, "u_vp", &f.light_vp[k]);
                p.f2(gl, "u_origin", f.origin.0 as f32, f.origin.1 as f32);
                p.f1(gl, "u_obj_time", f.time);
                p.f1(gl, "u_wind", f.look.wind);
                // Trees dissolving between levels cast their shadow with
                // both: the shapes are alike, so it hardly shows. Only the
                // trees whose shadow can fall into this cascade.
                self.draw_shadowed(gl, &TREE_CASTERS, k);
                self.draw_instanced(gl, &[OBJ_BUSH]);
                if k == 1 {
                    self.draw_instanced(gl, &[OBJ_PINE_FAR, OBJ_BROAD_FAR]);
                    // Cards cast their shadow turned towards the sun.
                    let p = self.programs.get_mut("sh_card").unwrap();
                    gl.use_program(Some(p.id));
                    p.m4(gl, "u_vp", &f.light_vp[k]);
                    p.f2(gl, "u_origin", f.origin.0 as f32, f.origin.1 as f32);
                    p.f1(gl, "u_obj_time", f.time);
                    p.f1(gl, "u_wind", f.look.wind);
                    p.v3(gl, "u_eye", f.cam);
                    self.draw_cards(gl, "sh_card", f.sun);
                    std::mem::swap(&mut self.cards, &mut self.shadow_cards);
                    self.draw_cards(gl, "sh_card", f.sun);
                    std::mem::swap(&mut self.cards, &mut self.shadow_cards);
                }
                // Back to the objects' own program (the far cascade used the
                // cards'), standing still.
                let p = self.programs.get_mut("sh_object").unwrap();
                gl.use_program(Some(p.id));
                p.f1(gl, "u_wind", 0.0);
                self.draw_instanced(
                    gl,
                    &[OBJ_ROCK, OBJ_BALE, OBJ_WALL, OBJ_POST, OBJ_PLANK, OBJ_RIDER],
                );
                self.draw_instanced(gl, &BUILDINGS);
                let p = self.programs.get_mut("sh_skinned").unwrap();
                gl.use_program(Some(p.id));
                p.m4(gl, "u_vp", &f.light_vp[k]);
                self.draw_skins(gl, f, "sh_skinned");
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

    /// Work out the ground's large-scale noise once (`GROUND_MAP_FS`),
    /// instead of in every ground pixel, every frame. The map is pinned to
    /// the world and wraps round, so as the terrain window moves only the
    /// ground newly inside it is worked out, a piece a frame (it lies at the
    /// far edge of the window, deep in the haze). Without the map, each pixel
    /// works it out itself.
    unsafe fn ground_pass(&mut self, gl: &glow::Context, f: &FrameData) {
        unsafe {
            if self.height_tex.is_none() {
                return;
            }
            if self.ground_map.is_none() && !self.ground_map_failed {
                Self::drain_errors(gl);
                // Half floats where they can be drawn to, bytes otherwise.
                let made = Self::target(gl, GROUND_MAP_SIZE, GROUND_MAP_SIZE, true)
                    .or_else(|_| Self::target(gl, GROUND_MAP_SIZE, GROUND_MAP_SIZE, false));
                match made {
                    Ok(t) => {
                        gl.bind_texture(glow::TEXTURE_2D, Some(t.tex));
                        for k in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                            gl.tex_parameter_i32(glow::TEXTURE_2D, k, glow::REPEAT as i32);
                        }
                        gl.bind_texture(glow::TEXTURE_2D, None);
                        self.ground_map = Some(t);
                    }
                    Err(_) => self.ground_map_failed = true,
                }
                Self::drain_errors(gl);
            }
            let Some(fbo) = self.ground_map.as_ref().map(|m| m.fbo) else {
                return;
            };
            let half = (GROUND_MAP_SIZE / 2) as i64;
            let min = [
                f.origin.0.floor() as i64 - half,
                f.origin.1.floor() as i64 - half,
            ];
            let mut all = false;
            if self.ground_min != Some(min) {
                match self.ground_min.and_then(|old| ground_map_todo(old, min)) {
                    Some(parts) => self.ground_todo.extend(parts),
                    // The first map, or a jump beyond it: all of it, now.
                    None => {
                        self.ground_todo.clear();
                        all = true;
                    }
                }
                self.ground_min = Some(min);
            }
            let part = if all {
                Some([0, 0, GROUND_MAP_SIZE, GROUND_MAP_SIZE])
            } else {
                self.ground_todo.pop()
            };
            let Some([x, y, w, h]) = part else {
                return;
            };
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.viewport(0, 0, GROUND_MAP_SIZE, GROUND_MAP_SIZE);
            // The scissor box is egui's for the 3D view, which drawing it to
            // the screen uses later: put it back afterwards.
            let mut egui_box = [0i32; 4];
            gl.get_parameter_i32_slice(glow::SCISSOR_BOX, &mut egui_box);
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(x, y, w, h);
            gl.disable(glow::DEPTH_TEST);
            self.common(gl, "ground_map", f);
            let p = self.programs.get_mut("ground_map").unwrap();
            p.f2(gl, "u_map_min", min[0] as f32, min[1] as f32);
            gl.bind_vertex_array(self.sky_vao);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.disable(glow::SCISSOR_TEST);
            let [bx, by, bw, bh] = egui_box;
            gl.scissor(bx, by, bw, bh);
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
        let t0 = std::time::Instant::now();
        self.mark(gl, MARK_3D);
        self.paint_frame(gl, info, f);
        self.mark(gl, MARK_3D_DONE);
        self.paint_ms = t0.elapsed().as_secs_f32() * 1000.0;
    }

    /// Record GPU timestamp `k` of this frame (see `Marks`). At the start
    /// of a frame, read back the oldest frame in flight if it is done.
    fn mark(&mut self, gl: &glow::Context, k: usize) {
        if !self.timing || self.marks_failed {
            return;
        }
        unsafe {
            if self.marks.is_none() {
                let mut q = Vec::new();
                for _ in 0..4 {
                    let mut set = Vec::new();
                    for _ in 0..4 {
                        match gl.create_query() {
                            Ok(x) => set.push(x),
                            Err(_) => {
                                self.marks_failed = true;
                                return;
                            }
                        }
                    }
                    q.push(std::array::from_fn(|i| set[i]));
                }
                self.marks = Some(Marks {
                    issued: vec![[false; 4]; q.len()],
                    q,
                    frame: 0,
                    last_end: None,
                });
            }
            let m = self.marks.as_mut().unwrap();
            if k == MARK_BEGIN {
                m.frame = (m.frame + 1) % m.q.len();
                let slot = m.frame;
                let set = m.q[slot];
                let complete = m.issued[slot].iter().all(|&i| i);
                if complete
                    && gl.get_query_parameter_u32(set[MARK_END], glow::QUERY_RESULT_AVAILABLE) != 0
                {
                    let t = set.map(|q| gl.get_query_parameter_u64(q, glow::QUERY_RESULT));
                    let ms = |a: u64, b: u64| b.saturating_sub(a) as f32 / 1.0e6;
                    let egui = ms(t[MARK_BEGIN], t[MARK_3D]) + ms(t[MARK_3D_DONE], t[MARK_END]);
                    // Showing the frame read back before this one: from when
                    // it was done to when this one began.
                    if let Some(prev) = m.last_end {
                        self.other_ms = Some([egui, ms(prev, t[MARK_BEGIN])]);
                    }
                    m.last_end = Some(t[MARK_END]);
                } else {
                    m.last_end = None;
                }
                m.issued[slot] = [false; 4];
            } else if !m.issued[m.frame][MARK_BEGIN] {
                // This frame's start was not marked: leave it out.
                return;
            }
            gl.query_counter(m.q[m.frame][k], glow::TIMESTAMP);
            m.issued[m.frame][k] = true;
        }
    }

    fn paint_frame(&mut self, gl: &glow::Context, info: &egui::PaintCallbackInfo, f: &FrameData) {
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
            if !self.skin_tried {
                if let Some(skin) = SKIN.get() {
                    // Without it the bike is still drawn.
                    self.skin_tried = true;
                    self.skin = Self::upload_skin(gl, skin).ok();
                }
            }
            if let Some(t) = &f.terrain {
                self.land_uploads += 1;
                if let Err(e) = self.upload_terrain(gl, t) {
                    self.error = Some(e);
                    return;
                }
            }
            if let Some(far) = &f.far {
                self.far_data = Some(far.clone());
                self.far_origin = (f64::NAN, f64::NAN);
            }
            if self.far_data.is_some() && self.far_origin != f.origin {
                if let Err(e) = self.upload_far(gl, f.origin) {
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
            let scale = f.render_scale;
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
            self.ensure_card_textures(gl);
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

            self.time_stage(gl, 0);
            self.cloud_pass(gl, f);
            self.ground_pass(gl, f);
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
            gl.active_texture(glow::TEXTURE0 + UNIT_GROUND);
            gl.bind_texture(glow::TEXTURE_2D, self.ground_map.as_ref().map(|m| m.tex));
            gl.active_texture(glow::TEXTURE0 + UNIT_NOISE);
            gl.bind_texture(glow::TEXTURE_2D, self.noise_tex);
            gl.active_texture(glow::TEXTURE0);

            // --- the scene ---------------------------------------------------
            self.time_stage(gl, 1);
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
            self.draw_instanced(gl, &TREES);
            self.draw_instanced(gl, &[OBJ_BUSH, OBJ_PINE_FAR, OBJ_BROAD_FAR]);
            // Trees changing level: each level in its share of the pixels.
            self.common(gl, "object_fade", f);
            let p = self.programs.get_mut("object_fade").unwrap();
            p.f1(gl, "u_obj_time", f.time);
            p.f1(gl, "u_wind", f.look.wind);
            self.draw_instanced(gl, &TREE_FADES);
            // Distant trees as cards, with edges smoothed by the
            // multisampling where there is some.
            if self.card_tex.is_some() {
                let soft = self.targets.as_ref().is_some_and(|t| t.samples > 1);
                self.common(gl, "card", f);
                let p = self.programs.get_mut("card").unwrap();
                p.f1(gl, "u_obj_time", f.time);
                p.f1(gl, "u_wind", f.look.wind);
                p.f1(gl, "u_a2c", if soft { 1.0 } else { 0.0 });
                if soft {
                    gl.enable(glow::SAMPLE_ALPHA_TO_COVERAGE);
                }
                self.draw_cards(gl, "card", [0.0; 3]);
                gl.disable(glow::SAMPLE_ALPHA_TO_COVERAGE);
            }
            let p = self.programs.get_mut("object").unwrap();
            gl.use_program(Some(p.id));
            p.f1(gl, "u_wind", 0.0);
            self.draw_instanced(
                gl,
                &[
                    OBJ_ROCK,
                    OBJ_HOUSE,
                    OBJ_BARN,
                    OBJ_SHED,
                    OBJ_TOWNHOUSE,
                    OBJ_BALE,
                    OBJ_WALL,
                    OBJ_POST,
                    OBJ_REFLECTOR,
                    OBJ_PLANK,
                    OBJ_RIDER,
                ],
            );
            self.common(gl, "skinned", f);
            self.draw_skins(gl, f, "skinned");

            // Grass around the camera, before the ground: the ground behind
            // its blades is then never shaded.
            self.time_stage(gl, 3);
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
                p.i1(gl, "u_height", UNIT_HEIGHT as i32);
                p.v3(gl, "u_hmap", self.height_map);
                p.f1(gl, "u_water", f.water);
                p.f1(gl, "u_snow", f.snow);
                p.f1(gl, "u_gust", f.look.wind);
                // Pin the meadow to the world: the window's origin, in the
                // shader's (x, -z) frame, split into whole patches and the
                // rest, so it stays exact in single precision.
                for (patch, outer, set) in [
                    (GRASS_PATCH, 0.0, &self.grass),
                    (GRASS_RING, 1.0, &self.grass_ring),
                ] {
                    let (shift, lap) = grass_anchor(f.origin, patch as f64);
                    p.f1(gl, "u_patch", patch);
                    p.f1(gl, "u_outer", outer);
                    p.f2(gl, "u_shift", shift.0, shift.1);
                    p.f2(gl, "u_lap", lap.0, lap.1);
                    for b in set {
                        gl.bind_vertex_array(Some(b.vao));
                        gl.draw_arrays_instanced(glow::TRIANGLES, 0, b.count, b.instances);
                    }
                }
            }

            // Distant mountains, far to near.
            self.time_stage(gl, 2);
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
                self.draw_terrain(gl, &f.vp, count);
                // The distant land, out to the mountains, where the
                // window does not reach (the shader leaves out the rest).
                if let Some(far) = &self.far {
                    let (fvao, fcount) = (far.vao, far.count);
                    self.common(gl, "terrain_far", f);
                    let p = self.programs.get_mut("terrain_far").unwrap();
                    p.i1(gl, "u_scenery", f.scenery);
                    p.f1(gl, "u_water", f.water);
                    p.f1(gl, "u_snow", f.snow);
                    p.f1(gl, "u_cut", (terrain::WINDOW_SIZE / 2.0) as f32 - 40.0);
                    gl.bind_vertex_array(Some(fvao));
                    gl.draw_elements(glow::TRIANGLES, fcount, glow::UNSIGNED_INT, 0);
                }
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

            // Soft darkening under trees and riders.
            self.time_stage(gl, 4);
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
            self.time_stage(gl, STAGE_FINISHING);
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
            self.time_end(gl);
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
            let levels = f.quality.blur_levels().min(t.down.len()).max(2);
            p.f1(
                gl,
                "u_cheap",
                if f.quality == Quality::High { 0.0 } else { 1.0 },
            );
            let (mut src, mut sw, mut sh) = (t.color_tex, t.w, t.h);
            for (i, d) in t.down.iter().take(levels).enumerate() {
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
            let ups = (levels - 1).min(t.up.len());
            for i in (0..ups).rev() {
                let small = if i + 1 < ups {
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
                p.i1(gl, "u_samples", f.quality.ray_samples());
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
            p.f2(gl, "u_scene_texel", 1.0 / t.w as f32, 1.0 / t.h as f32);
            let smaller = t.w + 1 < screen[0] || t.h + 1 < screen[1];
            p.f1(gl, "u_upscale", if smaller { 1.0 } else { 0.0 });
            gl.uniform_4_f32_slice(p.loc(gl, "u_glass[0]"), &rects);
            gl.uniform_1_f32_slice(p.loc(gl, "u_glass_r[0]"), &radii);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ground_map_works_out_only_the_ground_newly_in_it() {
        let size = GROUND_MAP_SIZE as i64;
        // Which world place each texel holds for the square at `min`.
        let place = |min: [i64; 2], tx: i64, ty: i64| {
            [
                min[0] + (tx - min[0]).rem_euclid(size),
                min[1] + (ty - min[1]).rem_euclid(size),
            ]
        };
        let old = [-1000, 3000];
        for new in [
            [-680, 3000],
            [-1000, 2600],
            [-1300, 3330],
            [700, 1500],
            [-1000 - 2047, 3000],
        ] {
            let parts = ground_map_todo(old, new).unwrap();
            let mut todo = vec![false; (size * size) as usize];
            for [x, y, w, h] in parts {
                assert!(x >= 0 && y >= 0 && x + w <= size as i32 && y + h <= size as i32);
                assert!((w * h) as i64 <= GROUND_MAP_PIECE.max(w as i64));
                for ty in y..y + h {
                    for tx in x..x + w {
                        todo[(ty as i64 * size + tx as i64) as usize] = true;
                    }
                }
            }
            // Every texel whose place changed is worked out; the others,
            // still right, are left alone.
            for ty in 0..size {
                for tx in 0..size {
                    let changed = place(old, tx, ty) != place(new, tx, ty);
                    let done = todo[(ty * size + tx) as usize];
                    assert!(done || !changed, "{new:?}: texel {tx},{ty} missed");
                    assert!(changed || !done, "{new:?}: texel {tx},{ty} needlessly");
                }
            }
        }
        // Too far to keep any of it.
        assert!(ground_map_todo(old, [old[0] + size, old[1]]).is_none());
    }

    #[test]
    fn the_noise_texture_is_evenly_random() {
        let v = noise_values();
        assert_eq!(v.len(), NOISE_SIZE * NOISE_SIZE);
        let mut counts = [0usize; 256];
        for &b in &v {
            counts[b as usize] += 1;
        }
        // Every value turns up about equally often...
        let even = v.len() / 256;
        assert!(counts
            .iter()
            .all(|&c| c > even * 9 / 10 && c < even * 11 / 10));
        // ...and neighbours are unrelated.
        let corr: f64 = v
            .windows(2)
            .map(|w| (w[0] as f64 - 127.5) * (w[1] as f64 - 127.5))
            .sum::<f64>()
            / (v.len() as f64 * 127.5 * 127.5 / 3.0);
        assert!(corr.abs() < 0.01, "{corr}");
    }

    #[test]
    fn trees_cast_into_the_near_cascade_only_where_their_shadow_falls_in_it() {
        let eye = [0.0, 1.6, 0.0];
        let fwd = [0.0, 0.0, 1.0];
        let r_unit = 1.0 / CASCADES[0].1;
        let tree = |sun: V3, at: V3| {
            let (vps, _) = shadow_cascades(eye, fwd, norm(sun), 2048.0);
            casts_into(&vps[0], at, 26.0, 8.0, r_unit)
        };
        // Overhead sun: only trees around the rider.
        let noon = [0.1, 1.0, 0.05];
        assert!(tree(noon, [0.0, 0.0, 26.0]));
        assert!(tree(noon, [-30.0, 0.0, 40.0]));
        assert!(!tree(noon, [0.0, 0.0, 150.0]));
        assert!(!tree(noon, [200.0, 0.0, 0.0]));
        // The sun 30 degrees up, ahead: a tree 150 m up the road throws its
        // shadow back towards the rider, into the near cascade's stretch of
        // ground. (Measuring from the camera, as before, missed it.)
        let ahead = [0.0, 0.5, 0.866];
        assert!(tree(ahead, [0.0, 0.0, 150.0]));
        // With the sun behind, that shadow falls away, out of it.
        let behind = [0.0, 0.5, -0.866];
        assert!(!tree(behind, [0.0, 0.0, 150.0]));
        // Far off to the side it never reaches.
        assert!(!tree(ahead, [400.0, 0.0, 100.0]));
    }

    #[test]
    fn tree_shadows_reach_further_with_the_sun_low() {
        // Overhead, a shadow falls at the tree's foot.
        assert!(shadow_reach(1.0) < 1.0);
        // At 45 degrees, as long as the tree is tall.
        assert!((shadow_reach(std::f32::consts::FRAC_1_SQRT_2) - 30.0).abs() < 0.1);
        // Low in the sky, much longer, but never without end.
        assert!(shadow_reach(0.2) > 140.0);
        assert_eq!(shadow_reach(0.0), 400.0);
    }

    #[test]
    fn the_land_is_rebuilt_once_every_few_hundred_metres() {
        // Riding 2 km north, a new window is asked for each time the
        // previous one arrives (as the worker does, centred ahead).
        let mut origin: Option<(f64, f64)> = None;
        let mut builds = 0;
        for step in 0..=2000 {
            let ahead = look_ahead(0.0, step as f64, 0.0);
            if origin.is_none_or(|o| needs_new_window(ahead, o)) {
                origin = Some(ahead);
                builds += 1;
            }
            // The camera always stays well inside the 2 km window.
            let o = origin.unwrap();
            assert!((step as f64 - o.1).abs() < 800.0);
        }
        // The first window, then one every 320 m.
        assert_eq!(builds, 7);
        // A window that has just arrived is never out of date at once.
        let a = look_ahead(100.0, 200.0, 0.7);
        assert!(!needs_new_window(a, a));
    }

    #[test]
    fn trees_change_level_with_slack_and_never_flicker() {
        let at = |d: f32, thin: f32| Reach {
            to_card: 100.0,
            thin,
            d,
            dist: d,
        };
        assert_eq!(first_level(&at(30.0, -0.5)), FULL);
        assert_eq!(first_level(&at(200.0, -0.5)), CARD);
        // Riding away: a tree becomes a card only past its distance and the
        // slack...
        assert_eq!(next_level(FULL, &at(101.0, -0.5)), FULL);
        assert_eq!(next_level(FULL, &at(103.0, -0.5)), CARD);
        // ...and comes back only well inside it, so hovering at 100 m changes
        // nothing.
        assert_eq!(next_level(CARD, &at(99.0, -0.5)), CARD);
        assert_eq!(next_level(CARD, &at(97.0, -0.5)), FULL);
        // A jump (the view turned, a new window) settles in one go.
        assert_eq!(next_level(FULL, &at(900.0, 0.1)), THINNED);
        assert_eq!(next_level(THINNED, &at(10.0, -0.5)), FULL);
        // Thinned out of distant forest only clearly past its share, and back
        // only clearly inside it.
        assert_eq!(next_level(CARD, &at(800.0, 0.01)), CARD);
        assert_eq!(next_level(CARD, &at(800.0, 0.03)), THINNED);
        assert_eq!(next_level(THINNED, &at(800.0, -0.01)), THINNED);
        assert_eq!(next_level(THINNED, &at(800.0, -0.03)), CARD);
        // At the far edge of the forest, trees fade out, and in a little closer.
        assert_eq!(next_level(CARD, &at(FAR_CULL - 30.0, -0.5)), THINNED);
        assert_eq!(next_level(THINNED, &at(FAR_CULL - 60.0, -0.5)), THINNED);
        assert_eq!(next_level(THINNED, &at(FAR_CULL - 90.0, -0.5)), CARD);
        // Distances and which trees are thinned are shifted per tree, evenly.
        let shifts: Vec<f32> = (0..1000)
            .map(|i| unit_hash((i * 37, i * 11 - 500)))
            .collect();
        assert!(shifts.iter().all(|&h| (0.0..1.0).contains(&h)));
        let mean = shifts.iter().sum::<f32>() / shifts.len() as f32;
        assert!((mean - 0.5).abs() < 0.05, "mean {mean}");
        assert!((keep_share(100.0) - 1.0).abs() < 1e-6);
        assert!((keep_share(FAR_CULL) - 0.4).abs() < 1e-6);
    }

    #[test]
    fn cards_and_their_pictures_agree() {
        // The card shader places and picks views as `impostor.rs` drew them.
        for (name, v) in [
            ("IMP_FRAMES", impostor::FRAMES as f32),
            ("IMP_GRID", impostor::GRID as f32),
            ("IMP_HALF", impostor::HALF_WIDTH),
            ("IMP_BOTTOM", impostor::BOTTOM),
            ("IMP_TOP", impostor::TOP),
        ] {
            let decl = format!("const float {name} = {v:?};");
            assert!(
                shaders::IMPOSTOR_VS.contains(&decl),
                "IMPOSTOR_VS should declare {decl}"
            );
        }
        let decl = format!("const float IMP_DEPTH_HALF = {:?};", impostor::HALF_WIDTH);
        assert!(
            shaders::IMPOSTOR_SURFACE.contains(&decl),
            "IMPOSTOR_SURFACE: {decl}"
        );
    }

    /// The grass shader's layout, in f32 as on the GPU: a tuft's world
    /// spot (x, -z) and its per-lap random value.
    fn tuft(seed: (f32, f32), cam_world: (f64, f64), origin: (f64, f64)) -> (f64, f64, f32) {
        let patch = GRASS_PATCH;
        let fract = |v: f32| v - v.floor();
        let hash12 = |x: f32, y: f32| {
            let mut p = [fract(x * 0.1031), fract(y * 0.1031), fract(x * 0.1031)];
            let d = p[0] * (p[1] + 33.33) + p[1] * (p[2] + 33.33) + p[2] * (p[0] + 33.33);
            p = [p[0] + d, p[1] + d, p[2] + d];
            fract((p[0] + p[1]) * p[2])
        };
        let md = |a: f32| a - patch * (a / patch).floor();
        // The camera in window coordinates, as the app passes it.
        let centre = (
            (cam_world.0 - origin.0) as f32,
            -(cam_world.1 - origin.1) as f32,
        );
        let (shift, lap) = grass_anchor(origin, patch as f64);
        let c = (centre.0 + shift.0, centre.1 + shift.1);
        let l = (
            ((seed.0 - c.0) / patch).floor() - lap.0,
            ((seed.1 - c.1) / patch).floor() - lap.1,
        );
        let again = hash12(l.0 * 1.618 + seed.0 * 0.37, l.1 * 1.618 + seed.1 * 0.37);
        let rel = (
            md(seed.0 - c.0) - patch * 0.5,
            md(seed.1 - c.1) - patch * 0.5,
        );
        let xz = (centre.0 + rel.0, centre.1 + rel.1);
        (xz.0 as f64 + origin.0, xz.1 as f64 - origin.1, again)
    }

    #[test]
    fn grass_stays_put_when_the_terrain_window_moves() {
        // Before and after the window recentres (by a multiple of its
        // 7.8125 m cell, not of the patch), every tuft keeps its place and
        // its look.
        let cam = (12_345.6, -2_718.3);
        for (i, seed) in [(0.5f32, 59.0f32), (17.3, 3.1), (44.0, 28.8), (59.9, 0.2)]
            .into_iter()
            .enumerate()
        {
            let a = tuft(seed, cam, (12_000.0, -2_500.0));
            let b = tuft(
                seed,
                cam,
                (12_000.0 + 7.8125 * 97.0, -2_500.0 - 7.8125 * 41.0),
            );
            assert!(
                (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01,
                "tuft {i} moved: {a:?} {b:?}"
            );
            assert_eq!(a.2, b.2, "tuft {i} changed its look");
        }
    }

    /// Ride `secs` seconds in the same conditions.
    fn hold(s: &mut Stance, secs: f32, grade: f32, speed: f32, effort: f32, pedalling: bool) {
        for _ in 0..(secs * 10.0) as usize {
            s.update(0.1, grade, speed, effort, pedalling);
        }
    }

    #[test]
    fn riders_stand_now_and_then_on_steep_climbs() {
        let mut s = Stance::default();
        hold(&mut s, 10.0, 0.1, 4.0, 0.9, true);
        assert!(s.stand > 0.9, "up out of the saddle: {}", s.stand);
        hold(&mut s, 30.0, 0.1, 4.0, 0.9, true);
        assert!(s.stand < 0.1, "then seated again: {}", s.stand);
        // A gentle climb: seated throughout.
        let mut s = Stance::default();
        hold(&mut s, 60.0, 0.05, 6.0, 0.9, true);
        assert!(s.stand < 0.01);
    }

    #[test]
    fn hard_efforts_stand_only_at_the_start() {
        let mut s = Stance::default();
        hold(&mut s, 8.0, 0.0, 12.0, 2.2, true);
        assert!(s.stand > 0.9, "sprinting: {}", s.stand);
        // Holding a lot of power for long is done seated.
        hold(&mut s, 30.0, 0.0, 12.0, 2.2, true);
        assert!(s.stand < 0.1, "{}", s.stand);
    }

    #[test]
    fn fast_descents_are_ridden_low() {
        let mut s = Stance::default();
        hold(&mut s, 5.0, -0.06, 15.0, 1.0, true);
        assert!(s.tuck > 0.9, "pedalling downhill fast: {}", s.tuck);
        let mut s = Stance::default();
        hold(&mut s, 5.0, 0.0, 14.0, 0.0, false);
        assert!(s.tuck > 0.9, "freewheeling fast: {}", s.tuck);
        // Fast on the flat while pedalling: upright.
        let mut s = Stance::default();
        hold(&mut s, 5.0, 0.0, 14.0, 1.0, true);
        assert!(s.tuck < 0.01);
    }

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
    fn terrain_tiles_behind_the_camera_are_skipped() {
        let tile = |x: f32, z: f32, first: u32| terrain::Tile {
            first,
            count: 6,
            lo: [x - 10.0, -2.0, z - 10.0],
            hi: [x + 10.0, 2.0, z + 10.0],
        };
        // Looking along -z from 2 m up.
        let view = look_at([0.0, 2.0, 0.0], [0.0, 2.0, -10.0], [0.0, 1.0, 0.0]);
        let vp = mul(&perspective(1.0, 1.6, 0.5, 3000.0), &view);
        let tiles = [
            tile(0.0, -50.0, 0),
            tile(0.0, 50.0, 6),
            tile(0.0, -100.0, 12),
            tile(900.0, -20.0, 18),
        ];
        let runs = visible_runs(&vp, &tiles);
        // Ahead: drawn; behind and far off to the side: skipped.
        assert_eq!(runs, vec![(0, 6), (48, 6)]);
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
