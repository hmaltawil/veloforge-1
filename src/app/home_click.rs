//! Home's ride bar from the bike: the workout and route lists step with
//! ▲ ▼ (or Y and B) while lit, the ride's mode, and what is drawn for the
//! Click there (the little list either side, the hint line, the ring).

use super::glass;
use super::*;

/// The Click's colours, for its buttons on screen.
pub(super) const Y_BLUE: Color32 = Color32::from_rgb(43, 167, 199);
pub(super) const Z_ORANGE: Color32 = Color32::from_rgb(232, 102, 61);
pub(super) const A_GREEN: Color32 = Color32::from_rgb(63, 167, 90);
pub(super) const B_MAGENTA: Color32 = Color32::from_rgb(214, 60, 142);

impl App {
    /// The workout (0) or route (1) list one step back or on.
    pub(super) fn home_step(&mut self, list: usize, on: bool) {
        let step = |i: usize, n: usize| if on { (i + 1) % n } else { (i + n - 1) % n };
        if list == 0 {
            let lib = self.library();
            if lib.is_empty() {
                return;
            }
            let i = self
                .ride_workout
                .as_ref()
                .and_then(|id| lib.iter().position(|w| &w.id == id))
                .unwrap_or(0);
            let id = lib[step(i, lib.len())].id.clone();
            self.load_ride(Some(id), None);
        } else {
            let n = self.routes.len();
            let id = self.routes[step(self.route_idx, n)].base_id().to_string();
            self.choose_route(id);
        }
    }

    /// Change the ride's mode (Workout, Free ride, Slope, Heart rate).
    pub(super) fn set_ride_mode(&mut self, mode: Mode) {
        let w = if mode == Mode::Workout {
            self.ride_workout
                .clone()
                .or(Some("endurance-60".into()))
                .and_then(|id| self.find_workout(&id))
        } else {
            None
        };
        self.ride_workout = w
            .as_ref()
            .map(|w| w.id.clone())
            .or(self.ride_workout.clone());
        self.replace_ride(Ride::new(mode, w, None));
        if mode == Mode::HeartRate {
            self.ride.hr_control =
                ride::HrControl::new((self.store.settings.max_hr * 0.65).round());
        }
    }

    /// The names either side of the lit workout or route, for the little
    /// list above it.
    pub(super) fn home_neighbours(&self, list: usize) -> Option<(String, String)> {
        let around = |names: Vec<String>, i: usize| {
            let n = names.len();
            (n > 1).then(|| (names[(i + n - 1) % n].clone(), names[(i + 1) % n].clone()))
        };
        match list {
            0 => {
                let lib = self.library();
                let i = self
                    .ride_workout
                    .as_ref()
                    .and_then(|id| lib.iter().position(|w| &w.id == id))
                    .unwrap_or(0);
                around(lib.into_iter().map(|w| w.name).collect(), i)
            }
            _ => {
                let units = self.store.settings.units;
                around(
                    self.routes
                        .iter()
                        .map(|r| units.route_name(&r.name))
                        .collect(),
                    self.route_idx,
                )
            }
        }
    }
}

/// The white ring around what the Click has lit.
pub(super) fn ring(ctx: &egui::Context, r: Rect, radius: f32) {
    let p = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::unique("home-click-ring"),
    ));
    let r = r.expand(3.0);
    for (grow, alpha) in [(9.0, 18u8), (6.0, 30), (3.0, 50)] {
        p.rect_stroke(
            r.expand(grow / 2.0),
            CornerRadius::same((radius + grow / 2.0) as u8),
            Stroke::new(grow, Color32::from_white_alpha(alpha)),
            egui::StrokeKind::Outside,
        );
    }
    p.rect_stroke(
        r,
        CornerRadius::same(radius as u8),
        Stroke::new(2.0, Color32::WHITE),
        egui::StrokeKind::Outside,
    );
}

/// One of the left pad's arrows, drawn as a grey circle with a triangle
/// pointing its way (`dir`: 0 left, 1 up, 2 right, 3 down).
pub(super) fn arrow_dot(p: &egui::Painter, c: Pos2, r: f32, dir: u8) {
    p.circle_filled(c, r, Color32::from_white_alpha(70));
    let s = r * 0.45;
    let pts = match dir {
        0 => [
            c + Vec2::new(-s, 0.0),
            c + Vec2::new(s * 0.7, -s),
            c + Vec2::new(s * 0.7, s),
        ],
        1 => [
            c + Vec2::new(0.0, -s),
            c + Vec2::new(s, s * 0.7),
            c + Vec2::new(-s, s * 0.7),
        ],
        2 => [
            c + Vec2::new(s, 0.0),
            c + Vec2::new(-s * 0.7, s),
            c + Vec2::new(-s * 0.7, -s),
        ],
        _ => [
            c + Vec2::new(0.0, s),
            c + Vec2::new(-s, -s * 0.7),
            c + Vec2::new(s, -s * 0.7),
        ],
    };
    p.add(egui::Shape::convex_polygon(
        pts.to_vec(),
        Color32::WHITE,
        Stroke::NONE,
    ));
}

/// A Click button drawn as a coloured circle with its letter.
pub(super) fn key_dot(p: &egui::Painter, c: Pos2, r: f32, colour: Color32, letter: &str) {
    p.circle_filled(c, r, colour);
    p.text(
        c,
        Align2::CENTER_CENTER,
        letter,
        theme::semibold(r * 1.05),
        Color32::WHITE,
    );
}

/// Above the lit workout or route: the one before (Y), it, and the one
/// after (B).
pub(super) fn peek(
    ctx: &egui::Context,
    anchor: Rect,
    pad: bool,
    prev: &str,
    now: &str,
    next: &str,
) {
    let p = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::unique("home-click-peek"),
    ));
    let font = theme::regular(12.5);
    let bold = theme::semibold(13.5);
    let widest = [(prev, &font), (now, &bold), (next, &font)]
        .iter()
        .map(|(t, f)| {
            p.layout_no_wrap(t.to_string(), (*f).clone(), Color32::WHITE)
                .size()
                .x
        })
        .fold(0.0f32, f32::max);
    let w = (widest + 58.0).max(anchor.width());
    let h = 3.0 * 30.0 + 12.0;
    let r = Rect::from_min_size(
        // Above the hint line over the bar, not on it.
        Pos2::new(anchor.left(), anchor.top() - 50.0 - h),
        Vec2::new(w, h),
    );
    p.rect_filled(
        r.translate(Vec2::new(0.0, 8.0)).expand(4.0),
        CornerRadius::same(20),
        Color32::from_black_alpha(50),
    );
    p.rect_filled(
        r,
        CornerRadius::same(16),
        Color32::from_rgba_unmultiplied(28, 30, 36, 240),
    );
    p.rect_stroke(
        r,
        CornerRadius::same(16),
        Stroke::new(1.0, Color32::from_white_alpha(40)),
        egui::StrokeKind::Inside,
    );
    for (k, (text, lit)) in [(prev, false), (now, true), (next, false)]
        .iter()
        .enumerate()
    {
        let row = Rect::from_min_size(
            Pos2::new(r.left() + 6.0, r.top() + 6.0 + k as f32 * 30.0),
            Vec2::new(w - 12.0, 30.0),
        );
        if *lit {
            p.rect_filled(row, CornerRadius::same(10), Color32::from_white_alpha(36));
        }
        let c = Pos2::new(row.left() + 16.0, row.center().y);
        match (k, pad) {
            (0, false) => key_dot(&p, c, 9.0, Y_BLUE, "Y"),
            (2, false) => key_dot(&p, c, 9.0, B_MAGENTA, "B"),
            (0, true) => arrow_dot(&p, c, 9.0, 1),
            (2, true) => arrow_dot(&p, c, 9.0, 3),
            _ => {}
        }
        let clip = p.with_clip_rect(row.shrink(2.0));
        clip.text(
            Pos2::new(row.left() + 34.0, row.center().y),
            Align2::LEFT_CENTER,
            *text,
            if *lit { bold.clone() } else { font.clone() },
            if *lit {
                Color32::WHITE
            } else {
                Color32::from_white_alpha(140)
            },
        );
    }
}

/// One mark in the legend: a Click button or a pad arrow.
#[derive(Clone, Copy)]
enum Mark {
    Key(Color32, &'static str),
    Arrow(u8),
}

/// What the Click does on Home, in one line above the ride bar: with the
/// left pad, its arrows; without, Z A and Y B.
pub(super) fn legend(p: &egui::Painter, centre: Pos2, pad: bool) {
    let font = theme::regular(12.5);
    let plus = [Mark::Key(Color32::from_white_alpha(60), "+")];
    let minus = [Mark::Key(Color32::from_white_alpha(60), "−")];
    let parts: [(&[Mark], &str); 3] = if pad {
        [
            (
                &[
                    Mark::Arrow(0),
                    Mark::Arrow(1),
                    Mark::Arrow(2),
                    Mark::Arrow(3),
                ],
                "move",
            ),
            (&plus, "press what's lit"),
            (&minus, "back"),
        ]
    } else {
        [
            (
                &[Mark::Key(Z_ORANGE, "Z"), Mark::Key(A_GREEN, "A")],
                "move along the bar",
            ),
            (
                &[Mark::Key(Y_BLUE, "Y"), Mark::Key(B_MAGENTA, "B")],
                "change what's lit",
            ),
            (&plus, "start the ride"),
        ]
    };
    let gap = 22.0;
    let widths: Vec<f32> = parts
        .iter()
        .map(|(marks, text)| {
            marks.len() as f32 * 24.0
                + 4.0
                + p.layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE)
                    .size()
                    .x
        })
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + gap * (parts.len() - 1) as f32;
    let mut x = centre.x - total / 2.0;
    for ((marks, text), w) in parts.iter().zip(widths) {
        for m in marks.iter() {
            let c = Pos2::new(x + 10.0, centre.y);
            match *m {
                Mark::Key(colour, letter) => key_dot(p, c, 10.0, colour, letter),
                Mark::Arrow(dir) => arrow_dot(p, c, 10.0, dir),
            }
            x += 24.0;
        }
        p.text(
            Pos2::new(x + 4.0, centre.y),
            Align2::LEFT_CENTER,
            *text,
            font.clone(),
            glass::SOFT,
        );
        x += w - marks.len() as f32 * 24.0 + gap;
    }
}
