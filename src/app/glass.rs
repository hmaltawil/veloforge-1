//! Building blocks shared by the glass screens: a sidebar beside the main
//! area, rows, pill and round buttons, veil cards, and thin-line icons.

use super::*;

/// The sidebar's width on every screen that has one.
pub(super) const SIDEBAR: f32 = 236.0;

/// Text colours on glass.
pub(super) const SOFT: Color32 = Color32::from_rgba_premultiplied(219, 219, 219, 219);
pub(super) const QUIET: Color32 = Color32::from_rgba_premultiplied(184, 184, 184, 184);
/// Green, amber and pink used for status on glass.
pub(super) const OK: Color32 = Color32::from_rgb(110, 231, 160);
pub(super) const WAIT: Color32 = Color32::from_rgb(255, 209, 102);
pub(super) const ALERT: Color32 = Color32::from_rgb(255, 180, 171);

/// Split the glass pane into a sidebar and a main area, with a hairline
/// between them: the two areas inside the mock-ups' padding. Narrow windows
/// drop the sidebar.
pub(super) fn split(ui: &egui::Ui) -> (Option<Rect>, Rect) {
    let full = ui.max_rect();
    let (side, main) = if full.width() > 820.0 {
        let s = Rect::from_min_max(full.min, Pos2::new(full.left() + SIDEBAR, full.bottom()));
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::unique(super::pad::SIDE_KEY), s.right()));
        ui.painter().line_segment(
            [s.right_top(), s.right_bottom()],
            Stroke::new(1.0, Color32::from_white_alpha(31)),
        );
        (
            Some(s.shrink2(Vec2::new(18.0, 26.0))),
            Rect::from_min_max(Pos2::new(s.right(), full.top()), full.max),
        )
    } else {
        (None, full)
    };
    (
        side,
        Rect::from_min_max(
            main.min + Vec2::new(28.0, 26.0),
            main.max - Vec2::new(28.0, 20.0),
        ),
    )
}

/// Run `add` in `rect`.
pub(super) fn area<R>(
    ui: &mut egui::Ui,
    rect: Rect,
    salt: &str,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).id_salt(salt), add)
        .inner
}

/// A white-veil card: the mock-ups' sections inside the glass.
pub(super) fn card<R>(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(Color32::from_white_alpha(23))
        .corner_radius(22)
        .inner_margin(18)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui)
        })
        .inner
}

/// A screen's title.
pub(super) fn title(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .font(theme::medium(26.0))
            .color(Color32::WHITE),
    );
}

/// A section's heading inside a screen.
pub(super) fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .font(theme::semibold(17.0))
            .color(Color32::WHITE),
    );
}

/// A heading in a sidebar.
pub(super) fn side_heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.label(
            RichText::new(text)
                .font(theme::medium(15.0))
                .color(Color32::WHITE),
        );
    });
    ui.add_space(2.0);
}

/// Small capitals over a figure.
pub(super) fn caps(text: &str) -> RichText {
    RichText::new(text.to_uppercase())
        .font(theme::medium(11.5))
        .color(QUIET)
}

/// A row in a sidebar: an icon or colour stripe, a name and a quieter line
/// under it; lit when chosen or under the pointer.
pub(super) fn side_row(
    ui: &mut egui::Ui,
    selected: bool,
    lead: Lead,
    name: &str,
    meta: &str,
) -> egui::Response {
    let h = if meta.is_empty() { 40.0 } else { 52.0 };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, CornerRadius::same(12), Color32::from_white_alpha(36));
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(12), Color32::from_white_alpha(26));
    }
    let x = match lead {
        Lead::Icon(g) => {
            glyph(p, Pos2::new(rect.left() + 21.0, rect.center().y), g, SOFT);
            rect.left() + 42.0
        }
        Lead::Stripe(c) => {
            p.rect_filled(
                Rect::from_center_size(
                    Pos2::new(rect.left() + 14.0, rect.center().y),
                    Vec2::new(3.0, h - 22.0),
                ),
                CornerRadius::same(2),
                c,
            );
            rect.left() + 26.0
        }
        Lead::Tile(top, big) => {
            let tile = Rect::from_min_size(
                Pos2::new(rect.left() + 12.0, rect.center().y - 18.0),
                Vec2::splat(36.0),
            );
            p.rect_filled(tile, CornerRadius::same(9), Color32::from_white_alpha(31));
            p.text(
                tile.center() - Vec2::new(0.0, 7.0),
                Align2::CENTER_CENTER,
                top,
                theme::medium(9.5),
                QUIET,
            );
            p.text(
                tile.center() + Vec2::new(0.0, 6.0),
                Align2::CENTER_CENTER,
                big,
                theme::semibold(14.0),
                Color32::WHITE,
            );
            tile.right() + 12.0
        }
    };
    let clip = p.with_clip_rect(Rect::from_min_max(
        Pos2::new(x, rect.top()),
        rect.max - Vec2::new(6.0, 0.0),
    ));
    let font = if selected {
        theme::medium(13.5)
    } else {
        theme::regular(13.5)
    };
    if meta.is_empty() {
        clip.text(
            Pos2::new(x, rect.center().y),
            Align2::LEFT_CENTER,
            name,
            font,
            if selected { Color32::WHITE } else { SOFT },
        );
    } else {
        clip.text(
            Pos2::new(x, rect.center().y - 9.0),
            Align2::LEFT_CENTER,
            name,
            font,
            Color32::WHITE,
        );
        clip.text(
            Pos2::new(x, rect.center().y + 9.0),
            Align2::LEFT_CENTER,
            meta,
            theme::regular(12.0),
            QUIET,
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// What leads a sidebar row.
#[derive(Clone, Copy)]
pub(super) enum Lead<'a> {
    Icon(Glyph),
    Stripe(Color32),
    /// A small date tile: weekday over the day.
    Tile(&'a str, &'a str),
}

/// A pill button: lit (`on`) or clear, white text.
pub(super) fn pill(ui: &mut egui::Ui, text: &str, on: bool) -> egui::Response {
    let g = ui
        .painter()
        .layout_no_wrap(text.to_string(), theme::medium(13.5), Color32::WHITE);
    let size = Vec2::new(g.size().x + 32.0, 34.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() {
        Color32::from_white_alpha(61)
    } else if on {
        Color32::from_white_alpha(41)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(17), fill);
    ui.painter().galley(
        rect.center() - g.size() / 2.0,
        g,
        if on { Color32::WHITE } else { SOFT },
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A pill button with a warning colour, for removing things.
pub(super) fn pill_alert(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let g = ui
        .painter()
        .layout_no_wrap(text.to_string(), theme::medium(13.5), ALERT);
    let size = Vec2::new(g.size().x + 32.0, 34.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() {
        Color32::from_white_alpha(41)
    } else {
        Color32::from_white_alpha(23)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(17), fill);
    ui.painter()
        .galley(rect.center() - g.size() / 2.0, g, ALERT);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The main action: a white pill with dark text.
pub(super) fn pill_white(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let g = ui.painter().layout_no_wrap(
        text.to_string(),
        theme::semibold(13.5),
        Color32::from_rgb(28, 29, 33),
    );
    let size = Vec2::new(g.size().x + 36.0, 36.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() {
        Color32::WHITE
    } else {
        Color32::from_white_alpha(235)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(18), fill);
    ui.painter().galley(
        rect.center() - g.size() / 2.0,
        g,
        Color32::from_rgb(28, 29, 33),
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A round button with an icon: white with a dark icon (`white`) or clear
/// with a white icon.
pub(super) fn round(
    ui: &mut egui::Ui,
    size: f32,
    g: Glyph,
    white: bool,
    hover: &str,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    paint_round(ui.painter(), rect, g, white, resp.hovered());
    resp.on_hover_text(hover)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Paint a round button (for buttons laid out by hand).
pub(super) fn paint_round(p: &egui::Painter, rect: Rect, g: Glyph, white: bool, hovered: bool) {
    let r = rect.width() / 2.0;
    if white {
        p.circle_filled(
            rect.center(),
            r,
            if hovered {
                Color32::WHITE
            } else {
                Color32::from_white_alpha(235)
            },
        );
        glyph(p, rect.center(), g, Color32::from_rgb(28, 29, 33));
    } else {
        if hovered {
            p.circle_filled(rect.center(), r, Color32::from_white_alpha(41));
        }
        glyph(p, rect.center(), g, Color32::WHITE);
    }
}

/// An on/off switch. Returns the response, changed when flipped.
pub(super) fn toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(44.0, 26.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let p = ui.painter();
    let track = Color32::from_white_alpha(41).lerp_to_gamma(Color32::from_white_alpha(235), t);
    p.rect_filled(rect, CornerRadius::same(13), track);
    if resp.hovered() {
        p.rect_stroke(
            rect,
            CornerRadius::same(13),
            Stroke::new(1.0, Color32::from_white_alpha(90)),
            egui::StrokeKind::Inside,
        );
    }
    let x = egui::lerp(rect.left() + 13.0..=rect.right() - 13.0, t);
    let knob = Color32::WHITE.lerp_to_gamma(Color32::from_rgb(28, 29, 33), t);
    p.circle_filled(Pos2::new(x, rect.center().y), 10.0, knob);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A small rounded label: something a device can do.
pub(super) fn chip(ui: &mut egui::Ui, text: &str) {
    let g = ui
        .painter()
        .layout_no_wrap(text.to_string(), theme::regular(12.0), Color32::WHITE);
    let (rect, _) = ui.allocate_exact_size(g.size() + Vec2::new(20.0, 8.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(12), Color32::from_white_alpha(31));
    ui.painter()
        .galley(rect.center() - g.size() / 2.0, g, Color32::WHITE);
}

/// A smaller pill button, for rows in a sidebar.
pub(super) fn pill_small(ui: &mut egui::Ui, text: &str, on: bool) -> egui::Response {
    let g = ui
        .painter()
        .layout_no_wrap(text.to_string(), theme::medium(12.5), Color32::WHITE);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(g.size().x + 24.0, 30.0), Sense::click());
    let fill = if resp.hovered() {
        Color32::from_white_alpha(61)
    } else if on {
        Color32::from_white_alpha(36)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(15), fill);
    ui.painter().galley(
        rect.center() - g.size() / 2.0,
        g,
        if on { Color32::WHITE } else { SOFT },
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A coloured dot and a word in a lighter shade of it: a device's state
/// at the top of its card.
pub(super) fn status_lit(ui: &mut egui::Ui, colour: Color32, text: &str) -> egui::Response {
    let lit = colour.lerp_to_gamma(Color32::WHITE, 0.45);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 4.0, colour);
        ui.label(RichText::new(text).font(theme::regular(12.5)).color(lit));
    })
    .response
}

/// A coloured dot and a word: how a device or a ride is.
pub(super) fn status(ui: &mut egui::Ui, colour: Color32, text: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 3.5, colour);
        ui.label(RichText::new(text).font(theme::regular(12.5)).color(SOFT));
    });
}

/// A figure with its label above and unit beside it.
pub(super) fn figure(ui: &mut egui::Ui, label: &str, value: &str, unit: &str, size: f32) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(caps(label));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(
                RichText::new(value)
                    .font(theme::regular(size))
                    .color(Color32::WHITE),
            );
            if !unit.is_empty() {
                ui.label(
                    RichText::new(unit)
                        .font(theme::regular((size * 0.45).max(12.0)))
                        .color(QUIET),
                );
            }
        });
    });
}

/// The icons, drawn in thin lines about 20 points across.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Glyph {
    Menu(Tab),
    Route,
    Random,
    Plus,
    Minus,
    Play,
    Pause,
    Skip,
    Back,
    Forward,
    PrevSong,
    NextSong,
    Camera,
    Sun,
    Grid,
    Cube,
    People,
    Heart,
    Bolt,
    Gauge,
    Check,
    Folder,
    Image,
    /// A plug: what is connected.
    Plug,
    /// A chainring seen side on: gears.
    Crank,
    /// A little robot: the simulated trainer.
    Robot,
    /// A dial with its needle: calibration.
    Dial,
    /// Lines of text: the log.
    Lines,
    /// Two arrows chasing round: another one.
    Refresh,
    /// Four tiles: every scenery.
    Tiles,
    /// A barn: farmland and hills.
    Barn,
    /// A pine over water: lakeside.
    Lakeside,
    /// Peaks with a snow line: mountains.
    Peaks,
    /// Waves under a sun: the coast.
    Coast,
    /// A torii gate: Japan.
    Torii,
    /// A closed track with its start: a loop route.
    Loop,
}

pub(super) fn glyph(p: &egui::Painter, c: Pos2, g: Glyph, colour: Color32) {
    let s = Stroke::new(1.6, colour);
    let at = |x: f32, y: f32| c + Vec2::new(x, y);
    let line = |pts: &[(f32, f32)]| {
        p.add(egui::Shape::line(
            pts.iter().map(|&(x, y)| at(x, y)).collect(),
            s,
        ));
    };
    let fill = |pts: &[(f32, f32)]| {
        p.add(egui::Shape::convex_polygon(
            pts.iter().map(|&(x, y)| at(x, y)).collect(),
            colour,
            Stroke::NONE,
        ));
    };
    // A point on the mock-ups' 24-point icon grid, drawn three quarters
    // size round the centre.
    let grid = |x: f32, y: f32| c + Vec2::new((x - 12.0) * 0.75, (y - 12.0) * 0.75);
    let gline = |pts: &[(f32, f32)]| {
        p.add(egui::Shape::line(
            pts.iter().map(|&(x, y)| grid(x, y)).collect(),
            s,
        ));
    };
    match g {
        Glyph::Menu(tab) => super::menu::menu_icon(p, c, tab, colour),
        Glyph::Route => {
            p.circle_stroke(at(-5.0, 5.5), 2.0, s);
            p.circle_stroke(at(5.0, -5.5), 2.0, s);
            line(&[
                (-3.0, 5.5),
                (2.0, 5.5),
                (4.0, 3.0),
                (2.0, 0.5),
                (-2.0, -0.5),
                (-4.0, -3.0),
                (-2.0, -5.5),
                (3.0, -5.5),
            ]);
        }
        Glyph::Random => {
            line(&[(-8.0, -4.0), (-3.0, -4.0), (3.0, 4.0), (8.0, 4.0)]);
            line(&[(-8.0, 4.0), (-3.0, 4.0), (3.0, -4.0), (8.0, -4.0)]);
            line(&[(5.5, -6.5), (8.0, -4.0), (5.5, -1.5)]);
            line(&[(5.5, 1.5), (8.0, 4.0), (5.5, 6.5)]);
        }
        Glyph::Plus => {
            line(&[(-6.0, 0.0), (6.0, 0.0)]);
            line(&[(0.0, -6.0), (0.0, 6.0)]);
        }
        Glyph::Minus => line(&[(-6.0, 0.0), (6.0, 0.0)]),
        Glyph::Play => fill(&[(-4.0, -6.0), (6.0, 0.0), (-4.0, 6.0)]),
        Glyph::Pause => {
            p.rect_filled(
                Rect::from_min_max(at(-5.0, -6.0), at(-1.5, 6.0)),
                CornerRadius::same(1),
                colour,
            );
            p.rect_filled(
                Rect::from_min_max(at(1.5, -6.0), at(5.0, 6.0)),
                CornerRadius::same(1),
                colour,
            );
        }
        Glyph::Skip => {
            line(&[(-5.0, -6.0), (4.0, 0.0), (-5.0, 6.0), (-5.0, -6.0)]);
            line(&[(6.0, -6.0), (6.0, 6.0)]);
        }
        Glyph::Back => line(&[(3.0, -6.0), (-3.0, 0.0), (3.0, 6.0)]),
        Glyph::PrevSong => {
            fill(&[(6.0, -6.0), (6.0, 6.0), (-3.0, 0.0)]);
            p.rect_filled(
                Rect::from_min_max(at(-6.0, -6.0), at(-3.8, 6.0)),
                CornerRadius::same(1),
                colour,
            );
        }
        Glyph::NextSong => {
            fill(&[(-6.0, -6.0), (-6.0, 6.0), (3.0, 0.0)]);
            p.rect_filled(
                Rect::from_min_max(at(3.8, -6.0), at(6.0, 6.0)),
                CornerRadius::same(1),
                colour,
            );
        }
        Glyph::Forward => line(&[(-3.0, -6.0), (3.0, 0.0), (-3.0, 6.0)]),
        Glyph::Camera => {
            line(&[
                (-8.0, -4.0),
                (-5.0, -4.0),
                (-3.0, -7.0),
                (3.0, -7.0),
                (5.0, -4.0),
                (8.0, -4.0),
                (8.0, 7.0),
                (-8.0, 7.0),
                (-8.0, -4.0),
            ]);
            p.circle_stroke(at(0.0, 1.5), 3.5, s);
        }
        Glyph::Sun => {
            p.circle_stroke(c, 4.0, s);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let (y, x) = a.sin_cos();
                line(&[(x * 6.5, y * 6.5), (x * 9.0, y * 9.0)]);
            }
        }
        Glyph::Grid => {
            for (x, y) in [(-8.0, -8.0), (1.0, -8.0), (-8.0, 1.0), (1.0, 1.0)] {
                p.rect_stroke(
                    Rect::from_min_size(at(x, y), Vec2::splat(7.0)),
                    2.0,
                    s,
                    egui::StrokeKind::Middle,
                );
            }
        }
        Glyph::Cube => {
            line(&[
                (0.0, -8.0),
                (7.0, -4.0),
                (7.0, 4.0),
                (0.0, 8.0),
                (-7.0, 4.0),
                (-7.0, -4.0),
                (0.0, -8.0),
            ]);
            line(&[(-7.0, -4.0), (0.0, 0.0), (7.0, -4.0)]);
            line(&[(0.0, 0.0), (0.0, 8.0)]);
        }
        Glyph::People => {
            p.circle_stroke(at(-3.0, -4.0), 3.0, s);
            p.circle_stroke(at(5.0, -3.0), 2.5, s);
            line(&[
                (-9.0, 7.0),
                (-8.0, 2.5),
                (-3.0, 1.0),
                (2.0, 2.5),
                (3.0, 7.0),
            ]);
            line(&[(4.0, 1.5), (7.5, 2.0), (9.0, 6.0)]);
        }
        Glyph::Heart => line(&[
            (0.0, 7.0),
            (-7.0, 0.0),
            (-7.5, -3.5),
            (-5.0, -6.5),
            (-2.0, -6.5),
            (0.0, -4.0),
            (2.0, -6.5),
            (5.0, -6.5),
            (7.5, -3.5),
            (7.0, 0.0),
            (0.0, 7.0),
        ]),
        Glyph::Bolt => line(&[
            (1.5, -8.0),
            (-5.0, 1.0),
            (0.0, 1.0),
            (-1.5, 8.0),
            (5.0, -1.0),
            (0.0, -1.0),
            (1.5, -8.0),
        ]),
        Glyph::Gauge => {
            p.circle_stroke(c, 8.0, s);
            line(&[(0.0, 0.0), (4.0, -4.0)]);
        }
        Glyph::Check => line(&[(-6.0, 0.5), (-2.0, 4.5), (6.5, -4.5)]),
        Glyph::Folder => line(&[
            (-8.0, -5.0),
            (-3.0, -5.0),
            (-1.0, -3.0),
            (8.0, -3.0),
            (8.0, 7.0),
            (-8.0, 7.0),
            (-8.0, -5.0),
        ]),
        Glyph::Image => {
            p.rect_stroke(
                Rect::from_min_max(at(-8.0, -6.0), at(8.0, 7.0)),
                2.0,
                s,
                egui::StrokeKind::Middle,
            );
            line(&[(-8.0, 5.0), (-3.0, 0.0), (1.0, 4.0), (4.0, 1.0), (8.0, 5.0)]);
            p.circle_filled(at(3.5, -2.5), 1.6, colour);
        }
        Glyph::Plug => {
            line(&[(-3.0, -9.0), (-3.0, -5.0)]);
            line(&[(3.0, -9.0), (3.0, -5.0)]);
            let mut pts = vec![(-6.0, -1.0), (-6.0, -5.0), (6.0, -5.0), (6.0, -1.0)];
            for k in 1..12 {
                let a = std::f32::consts::PI * k as f32 / 12.0;
                pts.push((6.0 * a.cos(), -1.0 + 6.0 * a.sin()));
            }
            pts.push((-6.0, -1.0));
            line(&pts);
            line(&[(0.0, 5.0), (0.0, 9.0)]);
        }
        Glyph::Crank => {
            p.circle_stroke(c, 8.0, s);
            p.circle_stroke(c, 3.0, s);
            line(&[(0.0, -8.0), (0.0, -3.0)]);
            line(&[(0.0, 3.0), (0.0, 8.0)]);
            line(&[(-8.0, 0.0), (-3.0, 0.0)]);
            line(&[(3.0, 0.0), (8.0, 0.0)]);
        }
        Glyph::Robot => {
            line(&[(0.0, -9.0), (0.0, -6.0)]);
            p.rect_stroke(
                Rect::from_min_max(at(-7.0, -6.0), at(7.0, 6.0)),
                3.0,
                s,
                egui::StrokeKind::Middle,
            );
            p.circle_filled(at(-3.0, -1.0), 1.1, colour);
            p.circle_filled(at(3.0, -1.0), 1.1, colour);
            line(&[(-3.0, 3.0), (3.0, 3.0)]);
        }
        Glyph::Dial => {
            // Three quarters of a circle, from the bottom round to the left.
            let pts: Vec<(f32, f32)> = (0..=18)
                .map(|k| {
                    let a =
                        std::f32::consts::FRAC_PI_2 - k as f32 * std::f32::consts::PI * 1.5 / 18.0;
                    (8.0 * a.cos(), 8.0 * a.sin())
                })
                .collect();
            line(&pts);
            line(&[(0.0, 0.0), (4.0, -3.0)]);
            line(&[(-8.0, 0.0), (-5.0, 0.0)]);
        }
        // Drawn on the mock-ups' 24-point grid, three quarters size.
        Glyph::Tiles => {
            for (x, y) in [(3.5, 3.5), (13.5, 3.5), (3.5, 13.5), (13.5, 13.5)] {
                p.rect_stroke(
                    Rect::from_min_size(grid(x, y), Vec2::splat(7.0 * 0.75)),
                    1.5,
                    s,
                    egui::StrokeKind::Middle,
                );
            }
        }
        Glyph::Barn => {
            gline(&[
                (4.0, 20.5),
                (4.0, 10.0),
                (12.0, 4.0),
                (20.0, 10.0),
                (20.0, 20.5),
                (4.0, 20.5),
            ]);
            gline(&[(9.0, 20.5), (9.0, 14.5), (15.0, 14.5), (15.0, 20.5)]);
            gline(&[(9.0, 14.5), (15.0, 20.5)]);
            gline(&[(15.0, 14.5), (9.0, 20.5)]);
            gline(&[(10.5, 10.0), (13.5, 10.0)]);
        }
        Glyph::Lakeside => {
            gline(&[(8.0, 12.5), (8.0, 9.0)]);
            gline(&[(8.0, 2.5), (4.0, 9.0), (12.0, 9.0), (8.0, 2.5)]);
            for y in [15.5, 19.5] {
                let pts: Vec<(f32, f32)> = (0..=24)
                    .map(|k| {
                        let x = 3.0 + k as f32 * 0.75;
                        (x, y - 0.9 * (std::f32::consts::PI * (x - 3.0) / 4.5).sin())
                    })
                    .collect();
                gline(&pts);
            }
        }
        Glyph::Peaks => {
            gline(&[
                (2.0, 20.0),
                (9.0, 8.0),
                (13.0, 14.0),
                (15.5, 10.5),
                (22.0, 20.0),
                (2.0, 20.0),
            ]);
            gline(&[(7.2, 11.0), (9.0, 12.5), (10.8, 11.0)]);
        }
        Glyph::Coast => {
            for (mid, amp) in [(13.25, 1.75), (18.25, 1.25)] {
                let pts: Vec<(f32, f32)> = (0..=24)
                    .map(|k| {
                        let x = 2.0 + k as f32 * 0.75;
                        (
                            x,
                            mid + amp * (std::f32::consts::TAU * (x - 2.0) / 12.0).cos(),
                        )
                    })
                    .collect();
                gline(&pts);
            }
            p.circle_stroke(grid(17.0, 6.0), 2.5 * 0.75, s);
        }
        Glyph::Torii => {
            gline(&[
                (3.0, 6.0),
                (7.0, 6.7),
                (12.0, 6.9),
                (17.0, 6.7),
                (21.0, 6.0),
            ]);
            gline(&[(5.0, 9.5), (19.0, 9.5)]);
            gline(&[(7.0, 7.0), (7.0, 20.0)]);
            gline(&[(17.0, 7.0), (17.0, 20.0)]);
            gline(&[(12.0, 7.0), (12.0, 9.5)]);
        }
        Glyph::Loop => {
            p.rect_stroke(
                Rect::from_min_max(grid(3.5, 6.5), grid(20.5, 17.5)),
                5.5 * 0.75,
                s,
                egui::StrokeKind::Middle,
            );
            p.circle_filled(grid(8.5, 17.5), 1.8 * 0.75 + 0.4, colour);
            gline(&[(13.5, 5.0), (12.0, 6.5), (13.5, 8.0)]);
        }
        Glyph::Refresh => {
            // Two arcs, each ending in an arrowhead.
            let arc = |from: f32| -> Vec<(f32, f32)> {
                (0..=10)
                    .map(|k| {
                        let a = from + k as f32 * 2.3 / 10.0;
                        (7.0 * a.cos(), 7.0 * a.sin())
                    })
                    .collect()
            };
            for from in [std::f32::consts::PI * 1.1, std::f32::consts::PI * 0.1] {
                let pts = arc(from);
                line(&pts);
                let (x, y) = pts[pts.len() - 1];
                let a = from + 2.3;
                // Back along the arc and out, back along the arc and in.
                let (tx, ty) = (-a.sin(), a.cos());
                let (nx, ny) = (a.cos(), a.sin());
                line(&[
                    (x - 3.5 * tx + 2.5 * nx, y - 3.5 * ty + 2.5 * ny),
                    (x, y),
                    (x - 3.5 * tx - 2.5 * nx, y - 3.5 * ty - 2.5 * ny),
                ]);
            }
        }
        Glyph::Lines => {
            line(&[(-8.0, -6.0), (8.0, -6.0)]);
            line(&[(-8.0, 0.0), (2.0, 0.0)]);
            line(&[(-8.0, 6.0), (5.0, 6.0)]);
        }
    }
}
