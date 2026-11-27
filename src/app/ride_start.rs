//! Starting a ride by pedalling (a countdown, then it starts), and
//! finishing one by holding the Click's Y (then Y again to confirm, B to
//! carry on).

use super::glass;
use super::*;
use crate::zwift;

/// The countdown before a ride starts by itself.
const COUNTDOWN: f32 = 10.0;
/// Stopping pedalling this long during the countdown calls it off.
const STOPPED: f32 = 3.0;
/// Holding Y this long asks to finish.
const HOLD: f32 = 3.0;
/// "Finish the ride?" goes away by itself after this long.
const ASK_FOR: f32 = 15.0;

impl App {
    /// Pedalling now: cadence or power from the trainer or a sensor.
    fn pedalling_now(&self) -> bool {
        fresh(&self.metrics.cadence).is_some_and(|c| c >= 30.0)
            || fresh(&self.metrics.power).is_some_and(|p| p >= 40.0)
    }

    /// Seconds left in the countdown to the ride starting, if counting.
    pub(super) fn start_left(&self) -> Option<f32> {
        self.start_count
            .map(|t| (COUNTDOWN - t.elapsed().as_secs_f32()).max(0.0))
    }

    /// Every frame: the countdown, and how long Y has been held.
    pub(super) fn ride_start_tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let active = self.ride_active();

        // Holding Y through a ride asks to finish it.
        if let Some((since, false)) = self.y_held {
            if !active {
                self.y_held = None;
            } else if since.elapsed().as_secs_f32() >= HOLD {
                self.y_held = Some((since, true));
                self.finish_ask = Some(now);
                self.cue(Cue::Change);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(30));
            }
        }
        if let Some(t) = self.finish_ask {
            if !active || t.elapsed().as_secs_f32() > ASK_FOR {
                self.finish_ask = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
        }

        // Starting by pedalling.
        let pedalling = self.pedalling_now();
        if pedalling {
            self.last_pedal = Some(now);
        }
        if active {
            // Riding on after Finish should not start another ride: wait
            // until the rider has stopped once.
            self.start_armed = false;
            self.start_count = None;
            return;
        }
        if !pedalling {
            self.start_armed = true;
        }
        let may = self.store.settings.start_by_pedalling
            && self.tab == Tab::Ride
            && self.start_armed
            && self.can_start();
        if !may {
            self.start_count = None;
            return;
        }
        match self.start_count {
            None if pedalling => {
                self.start_count = Some(now);
                self.start_ticked = 0;
            }
            None => return,
            Some(_) => {}
        }
        let stopped = self
            .last_pedal
            .is_none_or(|t| t.elapsed().as_secs_f32() > STOPPED);
        if stopped {
            self.start_count = None;
            return;
        }
        let left = self.start_left().unwrap_or(0.0);
        if left <= 0.0 {
            self.start_count = None;
            self.ride.start();
            self.cue(Cue::Change);
            return;
        }
        // A tick for each of the last three seconds.
        let whole = left.ceil() as u32;
        if whole <= 3 && whole != self.start_ticked {
            self.start_ticked = whole;
            self.cue(Cue::Tick);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }

    /// A Click button went down. During a ride Y waits to see whether it
    /// is a press (the music) or a hold (finish); while asked to finish,
    /// Y says yes.
    pub(super) fn click_pressed(&mut self, bits: u64) {
        if self.pad_nav() {
            let rest = self.pad_buttons(bits);
            if rest != 0 {
                self.click_button(rest);
            }
            return;
        }
        if self.music_learn.is_none() && bits & zwift::Y != 0 {
            if self.finish_ask.is_some() {
                self.finish_ask = None;
                self.y_held = None;
                self.finish_ride();
                return self.click_button(bits & !zwift::Y);
            }
            if self.ride_active() {
                self.y_held = Some((Instant::now(), false));
                return self.click_button(bits & !zwift::Y);
            }
        }
        self.click_button(bits);
    }

    /// A Click button came back up: Y let go before the hold counts is a
    /// press after all.
    pub(super) fn click_released(&mut self, bits: u64) {
        if bits & zwift::Y == 0 {
            return;
        }
        if let Some((_, false)) = self.y_held.take() {
            self.click_button(zwift::Y);
        }
    }

    /// No to "Finish the ride?".
    pub(super) fn keep_riding(&mut self) {
        self.finish_ask = None;
        self.y_held = None;
    }

    /// Over the ride screen: the countdown, Y filling up as it is held,
    /// and "Finish the ride?".
    pub(super) fn ride_overlays(&mut self, ctx: &egui::Context) {
        if self.tab != Tab::Ride {
            return;
        }
        if let Some(left) = self.start_left() {
            countdown(ctx, left);
        }
        if let Some((since, false)) = self.y_held {
            let t = since.elapsed().as_secs_f32();
            if t > 0.3 {
                hold_bar(ctx, ((t - 0.3) / (HOLD - 0.3)).min(1.0));
            }
        }
        if self.finish_ask.is_some() {
            let practice = self.ride_simulated || self.demo;
            match finish_card(ctx, practice) {
                Some(true) => {
                    self.finish_ask = None;
                    self.finish_ride();
                }
                Some(false) => self.keep_riding(),
                None => {}
            }
        }
    }
}

/// The big number counting down to the start.
fn countdown(ctx: &egui::Context, left: f32) {
    let screen = ctx.content_rect();
    let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("start-countdown"));
    let p = ctx.layer_painter(layer);
    let c = screen.center() - Vec2::new(0.0, screen.height() * 0.08);
    let r = 92.0;
    p.circle_filled(c, r + 6.0, Color32::from_black_alpha(40));
    p.circle_filled(c, r, Color32::from_rgba_unmultiplied(28, 30, 36, 170));
    p.circle_stroke(c, r, Stroke::new(1.0, Color32::from_white_alpha(50)));
    // A ring that empties as the seconds go.
    let frac = left / COUNTDOWN;
    let n = 64;
    let pts: Vec<Pos2> = (0..=n)
        .map(|k| {
            let a =
                -std::f32::consts::FRAC_PI_2 + k as f32 / n as f32 * frac * std::f32::consts::TAU;
            c + Vec2::new(a.cos(), a.sin()) * (r - 8.0)
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(4.0, Color32::WHITE)));
    p.text(
        c - Vec2::new(0.0, 46.0),
        Align2::CENTER_CENTER,
        "Starting in",
        theme::regular(13.0),
        glass::SOFT,
    );
    p.text(
        c + Vec2::new(0.0, 4.0),
        Align2::CENTER_CENTER,
        format!("{}", left.ceil() as u32),
        theme::light(64.0),
        Color32::WHITE,
    );
    p.text(
        c + Vec2::new(0.0, r + 26.0),
        Align2::CENTER_CENTER,
        "Keep pedalling · Space starts now",
        theme::regular(13.0),
        glass::SOFT,
    );
}

/// "Hold Y to finish", filling up while Y is held.
fn hold_bar(ctx: &egui::Context, t: f32) {
    let screen = ctx.content_rect();
    let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("y-hold"));
    let p = ctx.layer_painter(layer);
    let r = Rect::from_center_size(
        Pos2::new(screen.center().x, screen.top() + 236.0),
        Vec2::new(220.0, 44.0),
    );
    p.rect_filled(
        r,
        CornerRadius::same(22),
        Color32::from_rgba_unmultiplied(28, 30, 36, 190),
    );
    let fill = Rect::from_min_size(r.min, Vec2::new(r.width() * t, r.height()));
    p.with_clip_rect(fill)
        .rect_filled(r, CornerRadius::same(22), Color32::from_rgb(43, 167, 199));
    p.text(
        r.center(),
        Align2::CENTER_CENTER,
        "Hold Y to finish",
        theme::medium(14.0),
        Color32::WHITE,
    );
}

/// "Finish the ride?": Some(true) to finish, Some(false) to ride on.
fn finish_card(ctx: &egui::Context, practice: bool) -> Option<bool> {
    let mut answer = None;
    ctx.input(|i| {
        if i.key_pressed(egui::Key::Enter) {
            answer = Some(true);
        } else if i.key_pressed(egui::Key::Escape) {
            answer = Some(false);
        }
    });
    let screen = ctx.content_rect();
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::unique("finish-dim"),
    ))
    .rect_filled(screen, 0.0, Color32::from_black_alpha(90));
    egui::Area::new(egui::Id::unique("finish-ask"))
        .order(egui::Order::Tooltip)
        .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(34, 36, 42, 235))
                .stroke(Stroke::new(1.0, Color32::from_white_alpha(40)))
                .corner_radius(28)
                .inner_margin(28)
                .show(ui, |ui| {
                    ui.set_width(380.0);
                    ui.spacing_mut().item_spacing.y = 8.0;
                    ui.label(
                        RichText::new("Finish the ride?")
                            .font(theme::semibold(22.0))
                            .color(Color32::WHITE),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(if practice {
                                "Rides with the simulated trainer are practice: this one is not saved."
                            } else {
                                "Your ride is saved to Activity."
                            })
                            .font(theme::regular(13.5))
                            .color(glass::SOFT),
                        )
                        .wrap(),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        if answer_button(ui, "Y", Color32::from_rgb(43, 167, 199), "Finish", true)
                            .clicked()
                        {
                            answer = Some(true);
                        }
                        if answer_button(ui, "B", super::home_click::B_MAGENTA, "Keep riding", false)
                            .clicked()
                        {
                            answer = Some(false);
                        }
                    });
                });
        });
    answer
}

/// A button with the Click's button on it: a coloured circle with its
/// letter, then what it does.
fn answer_button(
    ui: &mut egui::Ui,
    letter: &str,
    colour: Color32,
    text: &str,
    main: bool,
) -> egui::Response {
    let g = ui
        .painter()
        .layout_no_wrap(text.to_string(), theme::medium(14.0), Color32::WHITE);
    let size = Vec2::new(g.size().x + 30.0 + 26.0 + 8.0, 44.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = match (resp.hovered(), main) {
        (true, _) => Color32::from_white_alpha(70),
        (false, true) => Color32::from_white_alpha(46),
        (false, false) => Color32::from_white_alpha(20),
    };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(22), fill);
    let dot = Pos2::new(rect.left() + 9.0 + 13.0, rect.center().y);
    p.circle_filled(dot, 13.0, colour);
    p.text(
        dot,
        Align2::CENTER_CENTER,
        letter,
        theme::semibold(13.0),
        Color32::WHITE,
    );
    p.galley(
        Pos2::new(dot.x + 13.0 + 8.0, rect.center().y - g.size().y / 2.0),
        g,
        Color32::WHITE,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}
