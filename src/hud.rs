//! The heads-up display over the 3D ride view: glass panels with your
//! effort on the left, the ride on the right, the interval in the middle
//! and the route along the bottom.

use super::*;
use crate::gfx::Glass;
use crate::theme;

const MARGIN: f32 = 14.0;
const PANEL_W: f32 = 226.0;
const PAD: f32 = 14.0;
const RADIUS: f32 = 14.0;
/// Height of the strip with the map and the profile.
const STRIP_H: f32 = 98.0;

/// One line of the banner in the middle: text, size, colour, emphasised.
type Line = (String, f32, Color32, bool);

/// Where each panel goes, worked out before anything is painted so the 3D
/// view can frost the glass behind them.
pub(super) struct HudPlan {
    left: Rect,
    right: Rect,
    map: Rect,
    profile: Rect,
    banner: Option<Rect>,
    warning: Option<Rect>,
}

impl HudPlan {
    pub(super) fn glass(&self) -> Vec<Glass> {
        let mut g = vec![
            (self.left, RADIUS),
            (self.right, RADIUS),
            (self.map, RADIUS),
            (self.profile, RADIUS),
        ];
        g.extend(self.banner.map(|b| (b, RADIUS)));
        g.extend(self.warning.map(|w| (w, 10.0)));
        g
    }
}

impl App {
    fn hud_target(&self) -> Option<f32> {
        self.ride.target_watts(self.store.settings.ftp)
    }

    /// The trainer cannot be controlled: say so, and offer the way out.
    fn trainer_message(&self) -> Option<String> {
        if self.trainer_link().is_some() {
            return None;
        }
        let connected: Vec<&str> = self
            .links
            .iter()
            .filter(|(id, l)| l.link == Link::Connected && id.as_str() != SIM_ID)
            .map(|(_, l)| l.name.as_str())
            .collect();
        Some(if connected.is_empty() {
            "No trainer connected — click here to connect one (or try the simulator)".to_string()
        } else {
            format!(
                "{} connected, but no resistance control found — click here for details",
                connected.join(", ")
            )
        })
    }

    /// Where the "no trainer" notice sits; clicking it opens Devices. Over
    /// the 3D view (`hud`) it clears the strip along the bottom.
    pub(super) fn banner_rect(view: Rect, hud: bool) -> Rect {
        let lift = if hud { MARGIN + STRIP_H } else { 0.0 };
        Rect::from_center_size(
            Pos2::new(view.center().x, view.bottom() - lift - 30.0),
            Vec2::new(620.0f32.min(view.width() - 2.0 * MARGIN), 34.0),
        )
    }

    /// The "no trainer" notice itself.
    pub(super) fn trainer_warning(&self, painter: &egui::Painter, r: Rect) {
        let Some(msg) = self.trainer_message() else {
            return;
        };
        painter.rect_filled(
            r,
            CornerRadius::same(10),
            Color32::from_rgba_unmultiplied(150, 70, 20, 170),
        );
        painter.text(
            r.center(),
            Align2::CENTER_CENTER,
            msg,
            theme::medium(13.0),
            Color32::WHITE,
        );
    }

    /// What the banner at the top says right now, if anything.
    fn banner_lines(&self) -> Vec<Line> {
        let ftp = self.store.settings.ftp;
        let dim = theme::TEXT_DIM;
        let ready = self.ride.state == ride::State::Ready;
        let hint = |text: &str| (text.to_string(), 12.5, theme::WARN, false);
        if self.ride.mode == Mode::Workout {
            let Some(w) = &self.ride.workout else {
                return Vec::new();
            };
            let t = self.ride.workout_time() as f32;
            if self.ride.workout_done() {
                return vec![
                    ("Workout complete!".into(), 24.0, theme::GOOD, true),
                    (
                        "Keep spinning to cool down, then press Finish".into(),
                        13.0,
                        dim,
                        false,
                    ),
                ];
            }
            let Some(i) = workout::segment_index(&self.ride.segs, t) else {
                return Vec::new();
            };
            let seg = self.ride.segs[i];
            let left = seg.end() - t;
            let now = seg.with_cadence(
                if seg.free {
                    "Free ride — your effort".to_string()
                } else {
                    format!("{:.0} W", seg.at(t) * self.ride.intensity * ftp)
                },
                "  ·  ",
            );
            let mut lines = vec![
                (
                    format!("{}  ·  {} of {}", w.name, i + 1, self.ride.segs.len()),
                    12.0,
                    dim,
                    false,
                ),
                (
                    format!("{now}  ·  {} left", fmt_dur(left.ceil())),
                    26.0,
                    Color32::WHITE,
                    true,
                ),
            ];
            if let Some(n) = self.ride.segs.get(i + 1) {
                lines.push((
                    n.with_cadence(
                        if n.free {
                            format!("Next: {} free ride", fmt_dur(n.secs))
                        } else {
                            format!(
                                "Next: {} at {:.0} W",
                                fmt_dur(n.secs),
                                n.from * self.ride.intensity * ftp
                            )
                        },
                        ", ",
                    ),
                    12.5,
                    dim,
                    false,
                ));
            }
            if ready {
                lines.push(hint("Press Start (or Space) when you're ready"));
            }
            lines
        } else if self.ride.mode == Mode::HeartRate {
            let c = &self.ride.hr_control;
            let status = match (fresh(&self.metrics.hr), c.power) {
                (None, Some(p)) if c.missing > 15.0 => (
                    format!("No heart rate signal — holding {p:.0} W"),
                    12.5,
                    theme::WARN,
                    false,
                ),
                (Some(h), Some(_)) => (format!("Heart rate {h:.0} bpm"), 12.5, dim, false),
                _ if ready => hint("Press Start (or Space) to begin"),
                _ => ("Waiting for heart rate…".into(), 12.5, dim, false),
            };
            vec![
                ("Heart rate ride".into(), 12.0, dim, false),
                (
                    match c.power {
                        Some(p) => format!("Holding {:.0} bpm  ·  {p:.0} W", c.target_bpm),
                        None => format!("Target {:.0} bpm", c.target_bpm),
                    },
                    26.0,
                    Color32::WHITE,
                    true,
                ),
                status,
            ]
        } else if ready {
            vec![
                (
                    self.routes[self.route_idx].name.clone(),
                    19.0,
                    Color32::WHITE,
                    true,
                ),
                hint("Press Start (or Space) to roll out"),
            ]
        } else {
            Vec::new()
        }
    }

    pub(super) fn hud_plan(&self, view: Rect) -> HudPlan {
        let top = view.top() + MARGIN;
        let mut left_h: f32 = 110.0 + 62.0;
        if self.hud_target().is_some() {
            left_h += 36.0;
        }
        let mut right_h: f32 = 84.0 + 2.0 * 46.0 + 6.0;
        if self.companion_status().is_some() {
            right_h += 50.0;
        }
        let strip_top = view.bottom() - MARGIN - STRIP_H;
        // On a short window the side panels give way to the strip.
        let room = (strip_top - 10.0 - top).max(120.0);
        let left = Rect::from_min_size(
            Pos2::new(view.left() + MARGIN, top),
            Vec2::new(PANEL_W, left_h.min(room)),
        );
        let right = Rect::from_min_size(
            Pos2::new(view.right() - MARGIN - PANEL_W, top),
            Vec2::new(PANEL_W, right_h.min(room)),
        );
        let map = Rect::from_min_size(
            Pos2::new(view.left() + MARGIN, strip_top),
            Vec2::new(STRIP_H * 1.25, STRIP_H),
        );
        let profile = Rect::from_min_max(
            Pos2::new(map.right() + 10.0, strip_top),
            Pos2::new(view.right() - MARGIN, strip_top + STRIP_H),
        );
        let lines = self.banner_lines();
        let banner = (!lines.is_empty()).then(|| {
            let h: f32 = lines.iter().map(|l| l.1 + 7.0).sum::<f32>() + 15.0;
            let w = (view.width() - 2.0 * (PANEL_W + MARGIN + 12.0)).clamp(260.0, 440.0);
            Rect::from_min_size(Pos2::new(view.center().x - w / 2.0, top), Vec2::new(w, h))
        });
        HudPlan {
            left,
            right,
            map,
            profile,
            banner,
            warning: self
                .trainer_message()
                .map(|_| Self::banner_rect(view, true)),
        }
    }

    /// Paint the HUD. `frosted`: the 3D view blurs what is behind the panels.
    pub(super) fn hud(&self, painter: &egui::Painter, view: Rect, plan: &HudPlan, frosted: bool) {
        let painter = painter.with_clip_rect(view);
        for (r, radius) in plan.glass() {
            theme::glass(&painter, r, radius, frosted);
        }
        self.hud_effort(&painter.with_clip_rect(plan.left), plan.left);
        self.hud_ride(&painter.with_clip_rect(plan.right), plan.right);
        self.hud_map(&painter, plan.map);
        self.hud_profile(&painter, plan.profile);
        if let Some(b) = plan.banner {
            let mut y = b.top() + 10.0;
            for (text, size, color, strong) in self.banner_lines() {
                let font = if strong {
                    theme::semibold(size)
                } else {
                    theme::regular(size)
                };
                painter.text(
                    Pos2::new(b.center().x, y),
                    Align2::CENTER_TOP,
                    text,
                    font,
                    color,
                );
                y += size + 7.0;
            }
        }
        if self.ride.state == ride::State::Paused {
            let r = Rect::from_center_size(view.center(), Vec2::new(250.0, 84.0));
            painter.rect_filled(r, CornerRadius::same(16), Color32::from_black_alpha(150));
            painter.text(
                r.center() - Vec2::new(0.0, 8.0),
                Align2::CENTER_CENTER,
                "PAUSED",
                theme::bold(36.0),
                Color32::WHITE,
            );
            painter.text(
                r.center() + Vec2::new(0.0, 24.0),
                Align2::CENTER_CENTER,
                "Space to carry on",
                theme::regular(12.5),
                theme::TEXT_DIM,
            );
        }
        if let Some(r) = plan.warning {
            self.trainer_warning(&painter, r);
        }
    }

    /// Left panel: power with its zone, target, cadence and heart rate.
    fn hud_effort(&self, painter: &egui::Painter, r: Rect) {
        let s = &self.store.settings;
        let ftp = s.ftp.max(1.0);
        let power = self.metrics.power3s();
        let (x0, x1) = (r.left() + PAD, r.right() - PAD);
        let mut y = r.top() + 12.0;
        theme::caption(
            painter,
            Pos2::new(x0, y),
            Align2::LEFT_TOP,
            "POWER · 3 S",
            theme::TEXT_DIM,
        );
        let frac = power.map(|p| p / ftp);
        let pcol = frac.map(charts::zone_color).unwrap_or(theme::TEXT_FAINT);
        if let Some(f) = frac {
            theme::pill(
                painter,
                Pos2::new(x1, y - 3.0),
                Align2::RIGHT_TOP,
                charts::zone_name(f),
                theme::mix(pcol, Color32::WHITE, 0.25),
            );
        }
        y += 70.0;
        theme::value(
            painter,
            Pos2::new(x0 - 2.0, y + 6.0),
            &power.map(|p| format!("{p:.0}")).unwrap_or("—".into()),
            "W",
            56.0,
            Color32::WHITE,
        );
        if let Some(p) = power {
            painter.text(
                Pos2::new(x1, y - 14.0),
                Align2::RIGHT_BOTTOM,
                format!("{:.1}", p / s.rider_kg.max(1.0)),
                theme::semibold(17.0),
                theme::TEXT,
            );
            painter.text(
                Pos2::new(x1, y - 1.0),
                Align2::RIGHT_BOTTOM,
                "W/kg",
                theme::medium(10.5),
                theme::TEXT_DIM,
            );
        }
        // The six power zones, with the one you are in lit up.
        y += 10.0;
        let zone = frac.map(charts::power_zone);
        let seg_w = (x1 - x0 - 5.0 * 3.0) / 6.0;
        for z in 0..6 {
            let c = charts::zone_color([0.5, 0.65, 0.85, 1.0, 1.15, 1.4][z]);
            let on = zone == Some(z);
            let bar = Rect::from_min_size(
                Pos2::new(
                    x0 + z as f32 * (seg_w + 3.0),
                    y + if on { 0.0 } else { 1.5 },
                ),
                Vec2::new(seg_w, if on { 7.0 } else { 4.0 }),
            );
            painter.rect_filled(
                bar,
                CornerRadius::same(3),
                if on { c } else { c.gamma_multiply(0.32) },
            );
        }
        y += 18.0;
        let rule = |y: f32| {
            painter.line_segment(
                [Pos2::new(x0, y), Pos2::new(x1, y)],
                Stroke::new(1.0, Color32::from_white_alpha(18)),
            );
        };
        if let Some(t) = self.hud_target() {
            rule(y);
            let mid = y + 18.0;
            theme::caption(
                painter,
                Pos2::new(x0, mid),
                Align2::LEFT_CENTER,
                "TARGET",
                theme::TEXT_DIM,
            );
            let tr = painter.text(
                Pos2::new(x1, mid),
                Align2::RIGHT_CENTER,
                format!("{t:.0} W"),
                theme::semibold(18.0),
                charts::zone_color(t / ftp),
            );
            // How far off the target you are.
            if let Some(p) = power.filter(|_| self.ride.state == ride::State::Running) {
                let d = p - t;
                let close = d.abs() <= (t * 0.05).max(8.0);
                painter.text(
                    Pos2::new(tr.left() - 8.0, mid),
                    Align2::RIGHT_CENTER,
                    format!("{d:+.0}"),
                    theme::medium(12.0),
                    if close { theme::GOOD } else { theme::WARN },
                );
            }
            y += 36.0;
        }
        rule(y);
        y += 10.0;
        // Cadence and heart rate side by side.
        let half = (x1 - x0) / 2.0;
        theme::caption(
            painter,
            Pos2::new(x0, y),
            Align2::LEFT_TOP,
            "CADENCE",
            theme::TEXT_DIM,
        );
        let cadence = fresh(&self.metrics.cadence).filter(|_| self.has_cadence());
        if let Some(t) = self.ride.target_cadence() {
            // "→ 85": the target, beside the caption (green when close).
            painter.text(
                Pos2::new(x0 + half - 4.0, y),
                Align2::RIGHT_TOP,
                format!("→ {t:.0}"),
                theme::semibold(10.5),
                charts::cadence_colour(cadence, t),
            );
        }
        theme::value(
            painter,
            Pos2::new(x0 - 1.0, y + 48.0),
            &cadence.map(|c| format!("{c:.0}")).unwrap_or("—".into()),
            "rpm",
            28.0,
            Color32::WHITE,
        );
        let hx = x0 + half + 4.0;
        let hr = fresh(&self.metrics.hr);
        theme::caption(
            painter,
            Pos2::new(hx, y),
            Align2::LEFT_TOP,
            "HEART RATE",
            theme::TEXT_DIM,
        );
        if let Some(h) = hr {
            let z = charts::hr_zone(h, s.max_hr);
            painter.text(
                Pos2::new(x1, y),
                Align2::RIGHT_TOP,
                format!("Z{}", z + 1),
                theme::semibold(10.5),
                theme::mix(charts::hr_zone_color(z), Color32::WHITE, 0.25),
            );
        }
        theme::value(
            painter,
            Pos2::new(hx - 1.0, y + 48.0),
            &hr.map(|h| format!("{h:.0}")).unwrap_or("—".into()),
            "bpm",
            28.0,
            Color32::WHITE,
        );
    }

    /// Right panel: speed, distance, time, gradient, climbing, companion.
    fn hud_ride(&self, painter: &egui::Painter, r: Rect) {
        let (x0, x1) = (r.left() + PAD, r.right() - PAD);
        let mut y = r.top() + 12.0;
        let route = &self.routes[self.route_idx];
        let grade = if self.ride.mode == Mode::Slope {
            self.ride.manual_grade as f64
        } else {
            route.grade(self.ride.route_pos())
        };
        let label = if self.ride.speed_from_trainer {
            "SPEED · TRAINER"
        } else {
            "SPEED"
        };
        theme::caption(
            painter,
            Pos2::new(x0, y),
            Align2::LEFT_TOP,
            label,
            theme::TEXT_DIM,
        );
        // In free ride, what the trainer was actually set to.
        if let (Some(devices::Control::Grade(g)), Mode::Route) =
            (self.ride.trainer_setting(), self.ride.mode)
        {
            theme::pill(
                painter,
                Pos2::new(x1, y - 3.0),
                Align2::RIGHT_TOP,
                &format!("Trainer {g:+.1}%"),
                theme::TEXT_DIM,
            );
        }
        y += 62.0;
        theme::value(
            painter,
            Pos2::new(x0 - 2.0, y + 4.0),
            &format!("{:.1}", self.ride.speed * 3.6),
            "km/h",
            46.0,
            Color32::WHITE,
        );
        y += 10.0;
        let half = (x1 - x0) / 2.0;
        let cell = |col: usize, y: f32, label: &str, text: String, unit: &str, c: Color32| {
            let x = x0 + col as f32 * (half + 4.0);
            theme::caption(
                painter,
                Pos2::new(x, y + 8.0),
                Align2::LEFT_TOP,
                label,
                theme::TEXT_DIM,
            );
            theme::value(painter, Pos2::new(x - 1.0, y + 46.0), &text, unit, 21.0, c);
        };
        let rule = |y: f32| {
            painter.line_segment(
                [Pos2::new(x0, y), Pos2::new(x1, y)],
                Stroke::new(1.0, Color32::from_white_alpha(18)),
            );
        };
        rule(y);
        cell(
            0,
            y,
            "DISTANCE",
            format!("{:.2}", self.ride.distance / 1000.0),
            "km",
            Color32::WHITE,
        );
        cell(
            1,
            y,
            "TIME",
            fmt_dur(self.ride.elapsed as f32),
            "",
            Color32::WHITE,
        );
        y += 46.0;
        // The gradient takes its colour from the profile below.
        let gcol = theme::mix(charts::grade_color(grade as f32), Color32::WHITE, 0.2);
        cell(0, y, "GRADE", format!("{grade:+.1}"), "%", gcol);
        cell(
            1,
            y,
            "CLIMBED",
            format!("{:.0}", self.ride.ascent),
            "m",
            Color32::WHITE,
        );
        y += 52.0;
        if let Some((title, text, unit, col)) = self.companion_status() {
            rule(y);
            theme::caption(
                painter,
                Pos2::new(x0, y + 8.0),
                Align2::LEFT_TOP,
                &title,
                theme::TEXT_DIM,
            );
            theme::value(
                painter,
                Pos2::new(x0 - 1.0, y + 46.0),
                &text,
                &unit,
                21.0,
                col,
            );
        }
    }

    /// The route seen from above, with what you have ridden and where you are.
    fn hud_map(&self, painter: &egui::Painter, r: Rect) {
        let route = &self.routes[self.route_idx];
        let pts = route.points();
        if pts.len() < 2 {
            return;
        }
        let (mut x0, mut x1, mut z0, mut z1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for p in pts {
            x0 = x0.min(p.x);
            x1 = x1.max(p.x);
            z0 = z0.min(p.z);
            z1 = z1.max(p.z);
        }
        let inner = r.shrink(12.0);
        let scale = (inner.width() as f64 / (x1 - x0).max(1.0))
            .min(inner.height() as f64 / (z1 - z0).max(1.0));
        let (cx, cz) = ((x0 + x1) / 2.0, (z0 + z1) / 2.0);
        // North is up.
        let to_screen = |x: f64, z: f64| {
            Pos2::new(
                inner.center().x + ((x - cx) * scale) as f32,
                inner.center().y - ((z - cz) * scale) as f32,
            )
        };
        let step = (pts.len() / 240).max(1);
        let here = self.ride.route_pos().rem_euclid(route.length);
        let split = ((here / route::STEP) as usize).min(pts.len() - 1);
        let line = |range: std::ops::Range<usize>| -> Vec<Pos2> {
            let mut v: Vec<Pos2> = range
                .clone()
                .step_by(step)
                .map(|i| to_screen(pts[i].x, pts[i].z))
                .collect();
            if let Some(last) = range.last() {
                v.push(to_screen(pts[last].x, pts[last].z));
            }
            v
        };
        painter.add(egui::Shape::line(
            line(0..pts.len()),
            Stroke::new(2.5, Color32::from_white_alpha(70)),
        ));
        if split > 0 {
            painter.add(egui::Shape::line(
                line(0..split + 1),
                Stroke::new(2.5, theme::ACCENT),
            ));
        }
        let dot = |d: f64, c: Color32, radius: f32| {
            let p = route.at(d);
            let at = to_screen(p.x, p.z);
            painter.circle_filled(at, radius + 3.5, Color32::from_black_alpha(110));
            painter.circle_filled(at, radius, c);
        };
        if let Some(gap) = self.ride.companion_gap() {
            dot(here + gap, Color32::from_rgb(150, 205, 255), 3.0);
        }
        dot(here, Color32::WHITE, 4.0);
    }

    /// The elevation (or the workout) along the bottom.
    fn hud_profile(&self, painter: &egui::Painter, r: Rect) {
        self.ride_profile(painter, r, None);
    }
}
