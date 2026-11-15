//! The workout builder: a custom workout's steps as blocks on a chart.
//! Drag a block's top edge to set its power, its right edge to set its
//! length, or the block itself to move it; the selected block's exact
//! numbers are below the chart. Changes can be undone.

use crate::app::{cadence_edit, pct_edit, secs_edit};
use crate::charts::zone_color;
use crate::theme;
use crate::workout::{fmt_dur, Step};
use eframe::egui::{
    self, Align2, Color32, CursorIcon, Key, Modifiers, Pos2, Rect, RichText, Sense, Shape, Stroke,
    Vec2,
};

/// Which part of a block a press grabbed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Handle {
    /// The top edge: its power.
    Power(Part),
    /// The right edge: its length.
    Length(Part),
    /// The block itself, to move it.
    Body,
}

/// Which number of a step a handle changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Whole,
    /// A ramp's starting and finishing power.
    Start,
    End,
    /// An interval set's efforts and recoveries, and how many repeats.
    On,
    Off,
    Repeats,
}

/// One drawn piece of a step: a steady block, a ramp, or one effort or
/// recovery of an interval set.
#[derive(Clone, Copy, Debug)]
struct Piece {
    t0: f32,
    t1: f32,
    p0: f32,
    p1: f32,
    free: bool,
    ramp: bool,
    power: Option<Part>,
    length: Option<Part>,
}

/// Time and power to screen and back. Frozen while dragging, so the chart
/// does not rescale under the pointer.
#[derive(Clone, Copy, Debug)]
struct View {
    plot: Rect,
    px_per_sec: f32,
    max: f32,
}

impl View {
    fn new(plot: Rect, steps: &[Step]) -> View {
        let total = steps
            .iter()
            .map(|s| s.secs() as f32)
            .sum::<f32>()
            .max(600.0);
        let top = steps.iter().map(top_power).fold(1.3f32, f32::max);
        View {
            plot,
            px_per_sec: plot.width() / total,
            max: top * 1.12,
        }
    }
    fn x(&self, t: f32) -> f32 {
        self.plot.left() + t * self.px_per_sec
    }
    fn y(&self, p: f32) -> f32 {
        self.plot.bottom() - (p / self.max).clamp(0.0, 1.0) * self.plot.height()
    }
    fn power_at(&self, y: f32) -> f32 {
        (self.plot.bottom() - y) / self.plot.height() * self.max
    }
}

fn top_power(s: &Step) -> f32 {
    match *s {
        Step::Steady { power, .. } => power,
        Step::Ramp { from, to, .. } => from.max(to),
        Step::Intervals {
            on_power,
            off_power,
            ..
        } => on_power.max(off_power),
        Step::FreeRide { .. } => 0.5,
    }
}

/// Where each step starts, in seconds.
fn starts(steps: &[Step]) -> Vec<f32> {
    let mut t = 0.0;
    steps
        .iter()
        .map(|s| {
            let start = t;
            t += s.secs() as f32;
            start
        })
        .collect()
}

fn pieces(step: &Step, start: f32) -> Vec<Piece> {
    let block = |t0: f32, secs: u32, p0: f32, p1: f32| Piece {
        t0,
        t1: t0 + secs as f32,
        p0,
        p1,
        free: false,
        ramp: false,
        power: Some(Part::Whole),
        length: Some(Part::Whole),
    };
    match *step {
        Step::Steady { secs, power, .. } => vec![block(start, secs, power, power)],
        Step::Ramp { secs, from, to, .. } => vec![Piece {
            ramp: true,
            ..block(start, secs, from, to)
        }],
        Step::FreeRide { secs, .. } => vec![Piece {
            free: true,
            power: None,
            ..block(start, secs, 0.5, 0.5)
        }],
        Step::Intervals {
            repeat,
            on_secs,
            on_power,
            off_secs,
            off_power,
            ..
        } => {
            let mut out = Vec::new();
            let mut t = start;
            for r in 0..repeat.max(1) {
                let last = r + 1 == repeat.max(1);
                out.push(Piece {
                    power: Some(Part::On),
                    length: (r == 0).then_some(Part::On),
                    ..block(t, on_secs, on_power, on_power)
                });
                t += on_secs as f32;
                out.push(Piece {
                    power: Some(Part::Off),
                    length: if last && repeat > 1 {
                        Some(Part::Repeats)
                    } else {
                        (r == 0).then_some(Part::Off)
                    },
                    ..block(t, off_secs, off_power, off_power)
                });
                t += off_secs as f32;
            }
            out
        }
    }
}

/// Lengths snap to round numbers: 15 s steps up to 5 minutes, 30 s up to
/// 20 minutes, whole minutes beyond.
pub fn snap_secs(secs: f32) -> u32 {
    let step = if secs < 300.0 {
        15.0
    } else if secs < 1200.0 {
        30.0
    } else {
        60.0
    };
    ((secs / step).round() * step).clamp(15.0, 4.0 * 3600.0) as u32
}

/// Power snaps to whole percent of FTP.
pub fn snap_power(frac: f32) -> f32 {
    ((frac * 100.0).round() / 100.0).clamp(0.0, 3.0)
}

/// `orig` changed by dragging `handle`: the pointer moved `d_secs` along
/// the chart and is level with `power`.
pub fn dragged(orig: &Step, handle: Handle, d_secs: f32, power: f32) -> Step {
    let mut s = orig.clone();
    let power = snap_power(power);
    let longer = |secs: u32| snap_secs(secs as f32 + d_secs);
    match (&mut s, handle) {
        (Step::Steady { power: p, .. }, Handle::Power(_)) => *p = power,
        (Step::Ramp { from, .. }, Handle::Power(Part::Start)) => *from = power,
        (Step::Ramp { to, .. }, Handle::Power(_)) => *to = power,
        (Step::Intervals { on_power, .. }, Handle::Power(Part::On)) => *on_power = power,
        (Step::Intervals { off_power, .. }, Handle::Power(_)) => *off_power = power,
        (
            Step::Steady { secs, .. } | Step::Ramp { secs, .. } | Step::FreeRide { secs, .. },
            Handle::Length(_),
        ) => *secs = longer(*secs),
        (Step::Intervals { on_secs, .. }, Handle::Length(Part::On)) => *on_secs = longer(*on_secs),
        (Step::Intervals { off_secs, .. }, Handle::Length(Part::Off)) => {
            *off_secs = longer(*off_secs)
        }
        (
            Step::Intervals {
                repeat,
                on_secs,
                off_secs,
                ..
            },
            Handle::Length(_),
        ) => {
            let each = (*on_secs + *off_secs).max(1) as f32;
            let total = *repeat as f32 * each + d_secs;
            *repeat = (total / each).round().clamp(1.0, 50.0) as u32;
        }
        _ => {}
    }
    s
}

/// Move the step at `from` so it ends up at index `to` of the result.
pub fn move_step(steps: &mut Vec<Step>, from: usize, to: usize) {
    if from < steps.len() {
        let s = steps.remove(from);
        steps.insert(to.min(steps.len()), s);
    }
}

/// The zone blocks: (zone, power as a share of FTP, seconds).
const ZONE_BLOCKS: [(&str, f32, u32); 6] = [
    ("Recovery", 0.5, 600),
    ("Endurance", 0.65, 600),
    ("Tempo", 0.83, 600),
    ("Threshold", 0.98, 480),
    ("VO2 max", 1.13, 180),
    ("Anaerobic", 1.3, 60),
];

/// The blocks the palette adds.
fn templates() -> [(&'static str, &'static str, Step); 5] {
    [
        (
            "Warm-up",
            "10 minutes rising from 45% to 75% of FTP",
            Step::Ramp {
                secs: 600,
                from: 0.45,
                to: 0.75,
                cadence: None,
            },
        ),
        (
            "Intervals",
            "5 × 3 minutes at 105% with 2 minutes easy between",
            Step::Intervals {
                repeat: 5,
                on_secs: 180,
                on_power: 1.05,
                off_secs: 120,
                off_power: 0.55,
                on_cadence: None,
                off_cadence: None,
            },
        ),
        (
            "Ramp",
            "5 minutes rising from 60% to 90% of FTP",
            Step::Ramp {
                secs: 300,
                from: 0.6,
                to: 0.9,
                cadence: None,
            },
        ),
        (
            "Cool-down",
            "5 minutes easing from 65% to 40% of FTP",
            Step::Ramp {
                secs: 300,
                from: 0.65,
                to: 0.4,
                cadence: None,
            },
        ),
        (
            "Free ride",
            "10 minutes riding the road as you like",
            Step::FreeRide {
                secs: 600,
                cadence: None,
            },
        ),
    ]
}

struct Drag {
    step: usize,
    handle: Handle,
    press: Pos2,
    orig: Step,
    view: View,
}

#[derive(Default)]
pub struct Builder {
    workout: Option<String>,
    pub selected: Option<usize>,
    drag: Option<Drag>,
    undo: Vec<Vec<Step>>,
    redo: Vec<Vec<Step>>,
    /// The steps as of the last finished change.
    committed: Vec<Step>,
}

impl Builder {
    /// Start afresh when another workout is opened.
    fn attach(&mut self, id: &str, steps: &[Step]) {
        if self.workout.as_deref() != Some(id) {
            *self = Builder {
                workout: Some(id.to_string()),
                committed: steps.to_vec(),
                ..Default::default()
            };
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn undo(&mut self, steps: &mut Vec<Step>) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(steps, prev.clone()));
            self.committed = prev;
            self.drag = None;
        }
    }

    fn redo(&mut self, steps: &mut Vec<Step>) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(steps, next.clone()));
            self.committed = next;
            self.drag = None;
        }
    }

    /// The builder for workout `id`; true if its steps changed.
    pub fn show(&mut self, ui: &mut egui::Ui, id: &str, steps: &mut Vec<Step>, ftp: f32) -> bool {
        self.attach(id, steps);
        let before = steps.clone();
        if self.selected.is_some_and(|i| i >= steps.len()) {
            self.selected = steps.len().checked_sub(1);
        }

        self.palette(ui, steps);
        ui.add_space(4.0);
        self.chart(ui, steps, ftp);
        ui.label(
            RichText::new(
                "Drag a block's top edge to set its power, its right edge to set its length, \
                 or the block to move it. Delete removes the selected block, Ctrl+D \
                 duplicates it, Ctrl+Z undoes.",
            )
            .small()
            .color(theme::TEXT_FAINT),
        );
        ui.add_space(6.0);
        self.inspector(ui, steps, ftp);
        self.keys(ui, steps);

        // One undo step per finished change, not one per frame of a drag.
        let settled = !ui.input(|i| i.pointer.any_down()) && !ui.ctx().egui_wants_keyboard_input();
        if settled && *steps != self.committed {
            self.undo
                .push(std::mem::replace(&mut self.committed, steps.clone()));
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        *steps != before
    }

    fn palette(&mut self, ui: &mut egui::Ui, steps: &mut Vec<Step>) {
        // A steady block in each zone, coloured like it.
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Zone").color(theme::TEXT_DIM));
            for (k, (name, power, secs)) in ZONE_BLOCKS.iter().enumerate() {
                let colour = zone_color(*power);
                let button = egui::Button::new(
                    RichText::new(format!("Z{}", k + 1))
                        .color(Color32::BLACK)
                        .strong(),
                )
                .fill(colour)
                .corner_radius(6.0)
                .min_size(Vec2::new(40.0, 0.0));
                if ui
                    .add(button)
                    .on_hover_text(format!(
                        "{name}: {} at {:.0}% of FTP, after the selected block",
                        fmt_dur(*secs as f32),
                        power * 100.0
                    ))
                    .clicked()
                {
                    self.insert(
                        steps,
                        Step::Steady {
                            secs: *secs,
                            power: *power,
                            cadence: None,
                        },
                    );
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Add").color(theme::TEXT_DIM));
            for (label, help, step) in templates() {
                let colour = match &step {
                    Step::FreeRide { .. } => Color32::from_rgb(110, 116, 128),
                    s => zone_color(top_power(s) * 0.92),
                };
                let button = egui::Button::new(RichText::new(label).color(Color32::WHITE))
                    .fill(colour.gamma_multiply(0.55))
                    .corner_radius(6.0);
                if ui
                    .add(button)
                    .on_hover_text(format!("{help}, after the selected block"))
                    .clicked()
                {
                    self.insert(steps, step);
                }
            }
            ui.add_space(12.0);
            if ui
                .add_enabled(self.can_undo(), egui::Button::new("Undo"))
                .on_hover_text("Ctrl+Z")
                .clicked()
            {
                self.undo(steps);
            }
            if ui
                .add_enabled(self.can_redo(), egui::Button::new("Redo"))
                .on_hover_text("Ctrl+Shift+Z")
                .clicked()
            {
                self.redo(steps);
            }
        });
    }

    /// Add `step` after the selected block (or at the end) and select it.
    fn insert(&mut self, steps: &mut Vec<Step>, step: Step) {
        let at = self.selected.map(|i| i + 1).unwrap_or(steps.len());
        let at = at.min(steps.len());
        steps.insert(at, step);
        self.selected = Some(at);
    }

    fn chart(&mut self, ui: &mut egui::Ui, steps: &mut Vec<Step>, ftp: f32) {
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), 230.0),
            Sense::click_and_drag(),
        );
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 10.0, theme::CARD);
        let plot = Rect::from_min_max(
            rect.min + Vec2::new(12.0, 14.0),
            rect.max - Vec2::new(12.0, 22.0),
        );
        let view = self
            .drag
            .as_ref()
            .map(|d| d.view)
            .unwrap_or_else(|| View::new(plot, steps));

        // Interaction.
        let hover = resp.hover_pos().and_then(|p| hit(&view, steps, p));
        if let Some((_, h)) = hover.filter(|_| self.drag.is_none()) {
            ui.ctx().set_cursor_icon(cursor(h));
        }
        if resp.drag_started() {
            let press = ui.input(|i| i.pointer.press_origin()).unwrap_or_default();
            if let Some((i, handle)) = hit(&view, steps, press) {
                self.selected = Some(i);
                self.drag = Some(Drag {
                    step: i,
                    handle,
                    press,
                    orig: steps[i].clone(),
                    view,
                });
            }
        }
        let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
        if let (Some(d), Some(p)) = (&self.drag, pointer) {
            ui.ctx().set_cursor_icon(match d.handle {
                Handle::Body => CursorIcon::Grabbing,
                h => cursor(h),
            });
            if d.handle != Handle::Body && d.step < steps.len() {
                let d_secs = (p.x - d.press.x) / d.view.px_per_sec;
                steps[d.step] = dragged(&d.orig, d.handle, d_secs, d.view.power_at(p.y));
            }
        }
        let mut drop_at = None;
        if let (Some(d), Some(p)) = (&self.drag, pointer) {
            if d.handle == Handle::Body {
                drop_at = Some(drop_index(&view, steps, d.step, p.x));
            }
        }
        if resp.drag_stopped() {
            if let (Some(d), Some(to)) = (self.drag.take(), drop_at) {
                if to != d.step {
                    move_step(steps, d.step, to);
                    self.selected = Some(to);
                }
            }
            self.drag = None;
            drop_at = None;
        }
        if resp.clicked() {
            self.selected = resp
                .interact_pointer_pos()
                .and_then(|p| hit(&view, steps, p))
                .map(|(i, _)| i);
        }

        // The zones' colours up the side, as a scale.
        let mut lo = 0.0;
        for (k, hi) in crate::charts::ZONE_STARTS
            .iter()
            .copied()
            .chain([view.max])
            .enumerate()
        {
            let hi = hi.min(view.max);
            if hi > lo {
                let bar = Rect::from_min_max(
                    Pos2::new(rect.left() + 4.0, view.y(hi)),
                    Pos2::new(rect.left() + 8.0, view.y(lo)),
                );
                let mid = if k == 0 { lo + 0.2 } else { (lo + hi) / 2.0 };
                painter.rect_filled(bar, 1.0, zone_color(mid).gamma_multiply(0.85));
            }
            lo = hi;
        }

        // Grid: 50%, FTP and 150%, and the time along the bottom.
        for frac in [0.5f32, 1.0, 1.5] {
            if frac >= view.max {
                continue;
            }
            let y = view.y(frac);
            let stroke = Stroke::new(
                1.0,
                Color32::from_white_alpha(if frac == 1.0 { 70 } else { 22 }),
            );
            if frac == 1.0 {
                painter.extend(Shape::dashed_line(
                    &[Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                    stroke,
                    4.0,
                    4.0,
                ));
                painter.text(
                    Pos2::new(plot.right() - 2.0, y - 2.0),
                    Align2::RIGHT_BOTTOM,
                    format!("FTP {ftp:.0} W"),
                    theme::medium(10.0),
                    Color32::from_white_alpha(150),
                );
            } else {
                painter.hline(plot.x_range(), y, stroke);
            }
        }
        let total_secs = plot.width() / view.px_per_sec;
        let tick = [60.0, 300.0, 600.0, 900.0, 1800.0, 3600.0]
            .into_iter()
            .find(|t| total_secs / t <= 10.0)
            .unwrap_or(3600.0);
        let mut t = tick;
        while t < total_secs {
            let x = view.x(t);
            painter.vline(
                x,
                plot.bottom()..=plot.bottom() + 4.0,
                Stroke::new(1.0, theme::TEXT_FAINT),
            );
            painter.text(
                Pos2::new(x, plot.bottom() + 6.0),
                Align2::CENTER_TOP,
                fmt_dur(t),
                theme::regular(10.0),
                theme::TEXT_FAINT,
            );
            t += tick;
        }

        // The blocks.
        let starts = starts(steps);
        let dragging_body = self
            .drag
            .as_ref()
            .filter(|d| d.handle == Handle::Body)
            .map(|d| d.step);
        for (i, (step, &start)) in steps.iter().zip(&starts).enumerate() {
            let selected = self.selected == Some(i);
            let alpha = if dragging_body == Some(i) { 0.35 } else { 1.0 };
            for pc in pieces(step, start) {
                draw_piece(&painter, &view, &pc, selected, alpha);
            }
            label_step(&painter, &view, step, start);
        }
        // The block being moved follows the pointer, with a marker where
        // it will land.
        if let (Some(d), Some(p), Some(to)) = (&self.drag, pointer, drop_at) {
            let dx = (p.x - d.press.x) / view.px_per_sec;
            for pc in pieces(&steps[d.step], starts[d.step] + dx) {
                draw_piece(&painter, &view, &pc, true, 0.8);
            }
            let mut others: Vec<f32> = Vec::new();
            for (j, (s, &st)) in steps.iter().zip(&starts).enumerate() {
                if j != d.step {
                    others.push(st);
                    let _ = s;
                }
            }
            let t = others.get(to).copied().unwrap_or_else(|| {
                starts.last().copied().unwrap_or(0.0)
                    + steps.last().map(|s| s.secs() as f32).unwrap_or(0.0)
            });
            let x = view.x(t);
            painter.vline(x, plot.y_range(), Stroke::new(3.0, theme::ACCENT));
        }
        // Hovered edge, and the value being set.
        let edge = self.drag.as_ref().map(|d| (d.step, d.handle)).or(hover);
        if let Some((i, h)) = edge.filter(|&(i, h)| h != Handle::Body && i < steps.len()) {
            for pc in pieces(&steps[i], starts[i]) {
                let (x0, x1) = (view.x(pc.t0), view.x(pc.t1));
                let white = Stroke::new(2.5, Color32::WHITE);
                if matches!(h, Handle::Power(part) if pc.power == Some(part) || (pc.ramp && pc.power.is_some()))
                {
                    let (a, b) = match h {
                        Handle::Power(Part::Start) => (x0, (x0 + x1) / 2.0),
                        Handle::Power(Part::End) => ((x0 + x1) / 2.0, x1),
                        _ => (x0, x1),
                    };
                    let yat = |x: f32| {
                        let u = ((x - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0);
                        view.y(pc.p0 + (pc.p1 - pc.p0) * u)
                    };
                    painter.line_segment([Pos2::new(a, yat(a)), Pos2::new(b, yat(b))], white);
                }
                if matches!(h, Handle::Length(part) if pc.length == Some(part)) {
                    painter.line_segment(
                        [Pos2::new(x1, view.y(pc.p1)), Pos2::new(x1, plot.bottom())],
                        white,
                    );
                }
            }
            if let (Some(d), Some(p)) = (&self.drag, pointer) {
                let text = value_text(&steps[d.step], d.handle, ftp);
                let galley_pos = p + Vec2::new(14.0, -18.0);
                let font = theme::semibold(12.0);
                let g = painter.layout_no_wrap(text, font, Color32::WHITE);
                let r = Rect::from_min_size(galley_pos, g.size()).expand(5.0);
                painter.rect_filled(r, 5.0, Color32::from_black_alpha(210));
                painter.galley(galley_pos, g, Color32::WHITE);
            }
        }
        if steps.is_empty() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Add blocks with the buttons above",
                theme::regular(14.0),
                theme::TEXT_DIM,
            );
        }
    }

    fn inspector(&mut self, ui: &mut egui::Ui, steps: &mut Vec<Step>, ftp: f32) {
        let Some(i) = self.selected.filter(|&i| i < steps.len()) else {
            ui.label(RichText::new("Click a block to edit its numbers.").color(theme::TEXT_DIM));
            return;
        };
        let watts = |frac: f32| format!("{:.0} W", frac * ftp);
        let mut action = None;
        egui::Frame::new()
            .fill(theme::CARD)
            .corner_radius(8.0)
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("Block {}", i + 1)).strong());
                    match &mut steps[i] {
                        Step::Steady {
                            secs,
                            power,
                            cadence,
                        } => {
                            ui.label("Steady for");
                            secs_edit(ui, secs);
                            ui.label("at");
                            pct_edit(ui, power);
                            ui.label(RichText::new(watts(*power)).weak());
                            cadence_edit(ui, cadence);
                        }
                        Step::Ramp {
                            secs,
                            from,
                            to,
                            cadence,
                        } => {
                            ui.label("Ramp for");
                            secs_edit(ui, secs);
                            ui.label("from");
                            pct_edit(ui, from);
                            ui.label("to");
                            pct_edit(ui, to);
                            ui.label(
                                RichText::new(format!("{} → {}", watts(*from), watts(*to))).weak(),
                            );
                            cadence_edit(ui, cadence);
                        }
                        Step::Intervals {
                            repeat,
                            on_secs,
                            on_power,
                            off_secs,
                            off_power,
                            on_cadence,
                            off_cadence,
                        } => {
                            ui.label("Intervals");
                            ui.add(egui::DragValue::new(repeat).range(1..=50).suffix(" ×"));
                            secs_edit(ui, on_secs);
                            ui.label("at");
                            pct_edit(ui, on_power);
                            ui.label(RichText::new(watts(*on_power)).weak());
                            cadence_edit(ui, on_cadence);
                            ui.label("then");
                            secs_edit(ui, off_secs);
                            ui.label("at");
                            pct_edit(ui, off_power);
                            cadence_edit(ui, off_cadence);
                        }
                        Step::FreeRide { secs, cadence } => {
                            ui.label("Free ride for");
                            secs_edit(ui, secs);
                            cadence_edit(ui, cadence);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(i > 0, egui::Button::new("Move earlier"))
                        .clicked()
                    {
                        action = Some(Action::Move(i, i - 1));
                    }
                    if ui
                        .add_enabled(i + 1 < steps.len(), egui::Button::new("Move later"))
                        .clicked()
                    {
                        action = Some(Action::Move(i, i + 1));
                    }
                    if ui.button("Duplicate").on_hover_text("Ctrl+D").clicked() {
                        action = Some(Action::Duplicate(i));
                    }
                    if ui
                        .button("Delete block")
                        .on_hover_text("Delete key")
                        .clicked()
                    {
                        action = Some(Action::Delete(i));
                    }
                });
            });
        if let Some(a) = action {
            self.apply(a, steps);
        }
    }

    fn keys(&mut self, ui: &mut egui::Ui, steps: &mut Vec<Step>) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let shift_cmd = Modifiers::COMMAND | Modifiers::SHIFT;
        let (undo, redo, redo_y, dup, del, back, left, right) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::Z),
                i.consume_key(shift_cmd, Key::Z),
                i.consume_key(Modifiers::COMMAND, Key::Y),
                i.consume_key(Modifiers::COMMAND, Key::D),
                i.consume_key(Modifiers::NONE, Key::Delete),
                i.consume_key(Modifiers::NONE, Key::Backspace),
                i.consume_key(Modifiers::NONE, Key::ArrowLeft),
                i.consume_key(Modifiers::NONE, Key::ArrowRight),
            )
        });
        if undo {
            self.undo(steps);
        }
        if redo || redo_y {
            self.redo(steps);
        }
        if let Some(i) = self.selected.filter(|&i| i < steps.len()) {
            if dup {
                self.apply(Action::Duplicate(i), steps);
            }
            if del || back {
                self.apply(Action::Delete(i), steps);
            }
            if left {
                self.selected = Some(i.saturating_sub(1));
            }
            if right {
                self.selected = Some((i + 1).min(steps.len() - 1));
            }
        } else if (left || right) && !steps.is_empty() {
            self.selected = Some(if left { steps.len() - 1 } else { 0 });
        }
    }

    fn apply(&mut self, a: Action, steps: &mut Vec<Step>) {
        match a {
            Action::Move(from, to) => {
                move_step(steps, from, to);
                self.selected = Some(to);
            }
            Action::Duplicate(i) => {
                let copy = steps[i].clone();
                steps.insert(i + 1, copy);
                self.selected = Some(i + 1);
            }
            Action::Delete(i) => {
                steps.remove(i);
                self.selected = if steps.is_empty() {
                    None
                } else {
                    Some(i.min(steps.len() - 1))
                };
            }
        }
    }
}

enum Action {
    Move(usize, usize),
    Duplicate(usize),
    Delete(usize),
}

fn cursor(h: Handle) -> CursorIcon {
    match h {
        Handle::Power(_) => CursorIcon::ResizeVertical,
        Handle::Length(_) => CursorIcon::ResizeHorizontal,
        Handle::Body => CursorIcon::Grab,
    }
}

/// The step and handle under `p`: edges first, then blocks.
fn hit(view: &View, steps: &[Step], p: Pos2) -> Option<(usize, Handle)> {
    let starts = starts(steps);
    let all: Vec<(usize, Piece)> = steps
        .iter()
        .zip(&starts)
        .enumerate()
        .flat_map(|(i, (s, &st))| pieces(s, st).into_iter().map(move |pc| (i, pc)))
        .collect();
    let top_at = |pc: &Piece, x: f32| {
        let (x0, x1) = (view.x(pc.t0), view.x(pc.t1));
        let u = ((x - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0);
        view.y(pc.p0 + (pc.p1 - pc.p0) * u)
    };
    let bottom = view.plot.bottom();
    // Right edges.
    for (i, pc) in &all {
        if let Some(part) = pc.length {
            let x1 = view.x(pc.t1);
            if (p.x - x1).abs() <= 5.0 && p.y >= top_at(pc, x1) - 6.0 && p.y <= bottom + 2.0 {
                return Some((*i, Handle::Length(part)));
            }
        }
    }
    // Top edges.
    for (i, pc) in &all {
        let (x0, x1) = (view.x(pc.t0), view.x(pc.t1));
        if let Some(part) = pc.power {
            if p.x >= x0 && p.x <= x1 && (p.y - top_at(pc, p.x)).abs() <= 6.0 {
                let part = if pc.ramp {
                    if p.x < (x0 + x1) / 2.0 {
                        Part::Start
                    } else {
                        Part::End
                    }
                } else {
                    part
                };
                return Some((*i, Handle::Power(part)));
            }
        }
    }
    // Blocks.
    for (i, pc) in &all {
        let (x0, x1) = (view.x(pc.t0), view.x(pc.t1));
        if p.x >= x0 && p.x <= x1 && p.y >= top_at(pc, p.x) && p.y <= bottom {
            return Some((*i, Handle::Body));
        }
    }
    None
}

/// Where the step being moved would land, with the pointer at `x`: after
/// every other step whose middle is to its left.
fn drop_index(view: &View, steps: &[Step], moving: usize, x: f32) -> usize {
    let starts = starts(steps);
    steps
        .iter()
        .zip(&starts)
        .enumerate()
        .filter(|&(j, (s, &st))| j != moving && view.x(st + s.secs() as f32 / 2.0) < x)
        .count()
}

fn draw_piece(painter: &egui::Painter, view: &View, pc: &Piece, selected: bool, alpha: f32) {
    let (x0, x1) = (view.x(pc.t0), view.x(pc.t1));
    let gap = if x1 - x0 > 6.0 { 0.75 } else { 0.0 };
    let pts = [
        Pos2::new(x0 + gap, view.y(pc.p0)),
        Pos2::new(x1 - gap, view.y(pc.p1)),
        Pos2::new(x1 - gap, view.plot.bottom()),
        Pos2::new(x0 + gap, view.plot.bottom()),
    ];
    let mut mesh = egui::Mesh::default();
    crate::charts::block_mesh(
        &mut mesh,
        (x0 + gap, x1 - gap),
        (pc.p0, pc.p1),
        |p| view.y(p),
        view.plot.bottom(),
        pc.free.then_some(0.5),
        |c| {
            let c = if selected {
                theme::mix(c, Color32::WHITE, 0.18)
            } else {
                c
            };
            c.gamma_multiply(alpha)
        },
    );
    painter.add(Shape::mesh(mesh));
    if selected {
        painter.add(Shape::closed_line(
            pts.to_vec(),
            Stroke::new(1.5, Color32::from_white_alpha((230.0 * alpha) as u8)),
        ));
    }
}

/// The step's numbers on the block, where it is wide enough to hold them.
fn label_step(painter: &egui::Painter, view: &View, step: &Step, start: f32) {
    let secs = step.secs() as f32;
    let (x0, x1) = (view.x(start), view.x(start + secs));
    let pct = |f: f32| format!("{:.0}%", f * 100.0);
    let (top, bottom) = match *step {
        Step::Steady { power, secs, .. } => (pct(power), fmt_dur(secs as f32)),
        Step::Ramp { from, to, secs, .. } => {
            (format!("{}→{}", pct(from), pct(to)), fmt_dur(secs as f32))
        }
        Step::Intervals {
            repeat,
            on_secs,
            off_secs,
            on_power,
            ..
        } => (
            format!("{repeat}× {}", pct(on_power)),
            format!("{}/{}", fmt_dur(on_secs as f32), fmt_dur(off_secs as f32)),
        ),
        Step::FreeRide { secs, .. } => ("Free".to_string(), fmt_dur(secs as f32)),
    };
    let font = theme::semibold(11.0);
    let cx = (x0 + x1) / 2.0;
    if let Step::Intervals { on_power, .. } = *step {
        // Over the bars, on one line.
        let line = format!("{top} · {bottom}");
        let width = painter
            .layout_no_wrap(line.clone(), font.clone(), Color32::WHITE)
            .size()
            .x;
        let y = view.y(on_power) - 9.0;
        if x1 - x0 >= width + 8.0 && y > view.plot.top() - 8.0 {
            painter.text(
                Pos2::new(cx, y),
                Align2::CENTER_CENTER,
                line,
                font,
                Color32::from_white_alpha(235),
            );
        }
        return;
    }
    let w = painter
        .layout_no_wrap(top.clone(), font.clone(), Color32::WHITE)
        .size()
        .x
        .max(
            painter
                .layout_no_wrap(bottom.clone(), font.clone(), Color32::WHITE)
                .size()
                .x,
        );
    if x1 - x0 < w + 8.0 {
        return;
    }
    let cx = (x0 + x1) / 2.0;
    let y = view.plot.bottom() - 30.0;
    for (k, text) in [top, bottom].into_iter().enumerate() {
        painter.text(
            Pos2::new(cx, y + k as f32 * 13.0),
            Align2::CENTER_CENTER,
            text,
            font.clone(),
            Color32::from_white_alpha(235),
        );
    }
}

/// What a drag is setting, for the label beside the pointer.
fn value_text(step: &Step, handle: Handle, ftp: f32) -> String {
    let pw = |f: f32| format!("{:.0}% · {:.0} W", f * 100.0, f * ftp);
    match (step, handle) {
        (Step::Steady { power, .. }, Handle::Power(_)) => pw(*power),
        (Step::Ramp { from, .. }, Handle::Power(Part::Start)) => pw(*from),
        (Step::Ramp { to, .. }, Handle::Power(_)) => pw(*to),
        (Step::Intervals { on_power, .. }, Handle::Power(Part::On)) => pw(*on_power),
        (Step::Intervals { off_power, .. }, Handle::Power(_)) => pw(*off_power),
        (Step::Intervals { on_secs, .. }, Handle::Length(Part::On)) => fmt_dur(*on_secs as f32),
        (Step::Intervals { off_secs, .. }, Handle::Length(Part::Off)) => fmt_dur(*off_secs as f32),
        (Step::Intervals { repeat, .. }, Handle::Length(_)) => format!("{repeat} ×"),
        (s, _) => fmt_dur(s.secs() as f32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steady(secs: u32, power: f32) -> Step {
        Step::Steady {
            secs,
            power,
            cadence: None,
        }
    }

    #[test]
    fn lengths_and_powers_snap_to_round_numbers() {
        assert_eq!(snap_secs(97.0), 90);
        assert_eq!(snap_secs(103.0), 105);
        assert_eq!(snap_secs(610.0), 600);
        assert_eq!(snap_secs(1531.0), 1560);
        assert_eq!(snap_secs(-50.0), 15);
        assert!((snap_power(0.734) - 0.73).abs() < 1e-6);
        assert_eq!(snap_power(-0.2), 0.0);
    }

    #[test]
    fn dragging_edges_changes_the_right_numbers() {
        let s = steady(300, 0.75);
        assert_eq!(
            dragged(&s, Handle::Power(Part::Whole), 0.0, 0.912),
            steady(300, 0.91)
        );
        assert_eq!(
            dragged(&s, Handle::Length(Part::Whole), 62.0, 0.0),
            steady(360, 0.75)
        );
        let r = Step::Ramp {
            secs: 600,
            from: 0.5,
            to: 0.7,
            cadence: None,
        };
        match dragged(&r, Handle::Power(Part::Start), 0.0, 0.4) {
            Step::Ramp { from, to, .. } => {
                assert!((from - 0.4).abs() < 1e-6 && (to - 0.7).abs() < 1e-6)
            }
            other => panic!("{other:?}"),
        }
        let iv = Step::Intervals {
            repeat: 4,
            on_secs: 120,
            on_power: 1.1,
            off_secs: 60,
            off_power: 0.5,
            on_cadence: None,
            off_cadence: None,
        };
        // Two more repeats' worth to the right.
        match dragged(&iv, Handle::Length(Part::Repeats), 350.0, 0.0) {
            Step::Intervals {
                repeat, on_secs, ..
            } => {
                assert_eq!(repeat, 6);
                assert_eq!(on_secs, 120);
            }
            other => panic!("{other:?}"),
        }
        match dragged(&iv, Handle::Power(Part::Off), 0.0, 0.6) {
            Step::Intervals {
                on_power,
                off_power,
                ..
            } => assert!((on_power - 1.1).abs() < 1e-6 && (off_power - 0.6).abs() < 1e-6),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn blocks_move_where_they_are_dropped() {
        let mut s = vec![steady(60, 0.5), steady(60, 0.6), steady(60, 0.7)];
        move_step(&mut s, 0, 2);
        assert_eq!(s, vec![steady(60, 0.6), steady(60, 0.7), steady(60, 0.5)]);
        // Dropped past the end stays at the end.
        move_step(&mut s, 0, 9);
        assert_eq!(s[2], steady(60, 0.6));
    }

    #[test]
    fn the_pointer_finds_edges_and_blocks() {
        // Two 10-minute blocks across a 1200-pixel plot: 1 px per second.
        let steps = vec![steady(600, 0.5), steady(600, 1.0)];
        let plot = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 300.0));
        let view = View::new(plot, &steps);
        let top = view.y(1.0);
        assert_eq!(
            hit(&view, &steps, Pos2::new(900.0, top)),
            Some((1, Handle::Power(Part::Whole)))
        );
        assert_eq!(
            hit(&view, &steps, Pos2::new(600.0, 290.0)),
            Some((0, Handle::Length(Part::Whole)))
        );
        assert_eq!(
            hit(&view, &steps, Pos2::new(900.0, 290.0)),
            Some((1, Handle::Body))
        );
        // Above the easy block: nothing.
        assert_eq!(hit(&view, &steps, Pos2::new(300.0, top)), None);
        // Dropping the first block past the second's middle puts it last.
        assert_eq!(drop_index(&view, &steps, 0, 1000.0), 1);
        assert_eq!(drop_index(&view, &steps, 0, 100.0), 0);
    }
}
