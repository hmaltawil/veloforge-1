//! The side menu, the picture behind the screens, and Activity (rides and
//! trends under one place in the menu).

use super::*;
use crate::backdrop::{self, Picture};
use std::collections::HashMap;

/// Each place in the menu is a circle this wide, its name inside.
const ITEM: f32 = 66.0;
const PAD: f32 = 8.0;
const GAP: f32 = 6.0;
/// The line between the places and the set-up places, with its margins.
const DIVIDER: f32 = 9.0;

/// The menu's margins: roomy round the glass screens, tight beside the 3D
/// world so riding keeps the width.
pub(super) fn margin(tab: Tab) -> egui::Margin {
    if tab == Tab::Ride {
        egui::Margin {
            left: 14,
            right: 8,
            top: 14,
            bottom: 14,
        }
    } else {
        egui::Margin {
            left: 48,
            right: 20,
            top: 48,
            bottom: 40,
        }
    }
}

/// How wide the menu's panel is: the pill and its margins.
pub(super) fn width(tab: Tab) -> f32 {
    let m = margin(tab);
    (m.left + m.right) as f32 + ITEM + PAD * 2.0
}

impl Tab {
    /// How the picture is turned for this part of the app (degrees round
    /// the colour wheel, then how colourful): each part has its own colour.
    fn tint(self) -> (f32, f32) {
        match self {
            Tab::Home | Tab::Ride => (0.0, 1.0),
            Tab::Library => (-105.0, 0.8),
            Tab::Calendar => (60.0, 0.85),
            Tab::Activity => (-150.0, 0.72),
            Tab::Devices => (-72.0, 0.92),
            Tab::Settings => (0.0, 0.22),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Tab::Home => "Home",
            Tab::Ride => "Ride",
            Tab::Library => "Library",
            Tab::Calendar => "Calendar",
            Tab::Activity => "Activity",
            Tab::Devices => "Devices",
            Tab::Settings => "Settings",
        }
    }
}

/// The picture behind the screens, decoded once, and the textures made
/// from it for each part of the app as they are first shown.
#[derive(Default)]
pub(super) struct Wallpapers {
    /// The picture's file, or `None` for Waves.
    source: Option<std::path::PathBuf>,
    /// `None` until first needed; `Some(None)` if it could not be read.
    picture: Option<Option<Picture>>,
    made: HashMap<Tab, Made>,
}

struct Made {
    sharp: egui::TextureHandle,
    frosted: egui::TextureHandle,
    size: [usize; 2],
}

impl Wallpapers {
    /// The textures for `tab`. Waves has a colour for each part of the app;
    /// a picture of your own is shown as it is everywhere.
    fn get(
        &mut self,
        ctx: &egui::Context,
        tab: Tab,
        source: Option<std::path::PathBuf>,
    ) -> Option<&Made> {
        if self.source != source {
            self.source = source;
            self.picture = None;
            self.made.clear();
        }
        let own = self.source.clone();
        let picture = self
            .picture
            .get_or_insert_with(|| match &own {
                None => backdrop::decode(backdrop::WAVES),
                Some(p) => std::fs::read(p).ok().and_then(|b| backdrop::decode(&b)),
            })
            .as_ref()?;
        let key = if own.is_some() { Tab::Home } else { tab };
        Some(self.made.entry(key).or_insert_with(|| {
            let (turn, colour) = if own.is_some() {
                (0.0, 1.0)
            } else {
                key.tint()
            };
            let p = backdrop::tint(picture, turn, colour);
            let f = backdrop::frosted(&p);
            let load = |name: &str, p: &Picture| {
                ctx.load_texture(
                    name,
                    egui::ColorImage::from_rgba_unmultiplied(p.size, &p.rgba),
                    egui::TextureOptions::LINEAR,
                )
            };
            Made {
                sharp: load(&format!("wallpaper-{key:?}"), &p),
                frosted: load(&format!("wallpaper-{key:?}-frosted"), &f),
                size: p.size,
            }
        }))
    }
}

/// The part of a picture (0-1) that covers `screen` without stretching:
/// as much as fits, trimmed evenly from the sides or the top and bottom.
fn cover(picture: [usize; 2], screen: Vec2) -> Rect {
    let pa = picture[0] as f32 / picture[1].max(1) as f32;
    let sa = screen.x / screen.y.max(1.0);
    if sa > pa {
        let h = pa / sa;
        Rect::from_min_max(
            Pos2::new(0.0, (1.0 - h) / 2.0),
            Pos2::new(1.0, (1.0 + h) / 2.0),
        )
    } else {
        let w = sa / pa;
        Rect::from_min_max(
            Pos2::new((1.0 - w) / 2.0, 0.0),
            Pos2::new((1.0 + w) / 2.0, 1.0),
        )
    }
}

impl App {
    /// Paint the picture behind everything, in the colour of the part of
    /// the app that is open, and tell the glass panels where it is.
    pub(super) fn paint_backdrop(&mut self, ctx: &egui::Context) {
        // Riding shows the 3D world; only the menu's strip shows the picture.
        let tab = if self.tab == Tab::Ride {
            Tab::Home
        } else {
            self.tab
        };
        let look = &self.store.settings;
        let zoom = look.text_size.clamp(0.8, 2.0);
        if (ctx.zoom_factor() - zoom).abs() > 0.001 {
            ctx.set_zoom_factor(zoom);
        }
        theme::set_bold_figures(look.bold_figures);
        let source =
            (!look.wallpaper.is_empty()).then(|| self.paths.pictures().join(&look.wallpaper));
        let dim = Color32::from_rgba_unmultiplied(
            10,
            12,
            16,
            (look.dim_picture.min(90) as f32 * 2.55) as u8,
        );
        let tint = theme::glass_tint(look.panel_darkness);
        let Some(made) = self.wallpapers.get(ctx, tab, source) else {
            return;
        };
        let screen = ctx.content_rect();
        let uv = cover(made.size, screen.size());
        let mut painter = ctx.layer_painter(egui::LayerId::background());
        if self.tab == Tab::Ride {
            // The 3D world or the dashboard covers everything: nothing to
            // paint, but the dashboard's glass needs to know the picture.
            painter.set_clip_rect(Rect::NOTHING);
        }
        painter.image(made.sharp.id(), screen, uv, Color32::WHITE);
        // Dim it a little, so it stays behind what is on the screen.
        painter.rect_filled(screen, 0.0, dim);
        theme::set_backdrop(
            ctx,
            theme::Backdrop {
                frosted: made.frosted.id(),
                screen,
                uv,
                tint,
            },
        );
    }

    /// The menu down the left: five places a rider goes, in the order they
    /// use them, then the two set-up places; while recording, the ride's
    /// clock, and a new version when there is one.
    pub(super) fn side_menu(&mut self, ui: &mut egui::Ui) {
        // Rides start from Home (or Library and Calendar); while one is
        // going, the recording clock below leads back to it.
        let places = [Tab::Home, Tab::Library, Tab::Calendar, Tab::Activity];
        let setup = [Tab::Devices, Tab::Settings];
        let status = self.status_lines(ui.input(|i| i.time));
        let line = 18.0;
        let status_h = if status.is_empty() {
            0.0
        } else {
            DIVIDER + 2.0 * GAP + status.len() as f32 * line
        };
        let items = places.len() + setup.len();
        let height = PAD * 2.0 + items as f32 * ITEM + items as f32 * GAP + DIVIDER + status_h;
        let space = ui.max_rect();
        let top = (space.center().y - height / 2.0).max(space.top());
        let pill = Rect::from_min_size(
            Pos2::new(space.left(), top),
            Vec2::new(ITEM + PAD * 2.0, height),
        );
        theme::frosted(ui.painter(), pill, pill.width() / 2.0);

        let mut y = pill.top() + PAD;
        for (i, tab) in places.iter().chain(&setup).enumerate() {
            if i == places.len() {
                self.menu_divider(ui, pill, y + DIVIDER / 2.0);
                y += DIVIDER + GAP;
            }
            let item = Rect::from_min_size(Pos2::new(pill.left() + PAD, y), Vec2::splat(ITEM));
            self.menu_item(ui, item, *tab);
            y += ITEM + GAP;
        }
        y -= GAP;
        if !status.is_empty() {
            self.menu_divider(ui, pill, y + GAP + DIVIDER / 2.0);
            y += GAP + DIVIDER + GAP;
            let left = pill.left() + 16.0;
            for (i, s) in status.into_iter().enumerate() {
                let row = Rect::from_min_size(
                    Pos2::new(pill.left() + 4.0, y + i as f32 * line),
                    Vec2::new(pill.width() - 8.0, line),
                );
                let resp = ui.interact(row, ui.scope_id().with(("status", i)), Sense::click());
                let resp = if s.hover.is_empty() {
                    resp
                } else {
                    resp.on_hover_text(&s.hover)
                };
                let c = row.left_center();
                let dot = Pos2::new(left, c.y);
                if s.glow {
                    ui.painter()
                        .circle_filled(dot, 5.5, s.colour.gamma_multiply(0.25));
                }
                ui.painter().circle_filled(dot, 3.0, s.colour);
                let text = ui.painter().layout_no_wrap(
                    s.text.clone(),
                    theme::medium(10.5),
                    if s.glow { theme::TEXT } else { theme::TEXT_DIM },
                );
                ui.painter().with_clip_rect(row).galley(
                    Pos2::new(left + 9.0, c.y - text.size().y / 2.0),
                    text,
                    Color32::WHITE,
                );
                let clickable = s.link.is_some() || s.ride;
                let clicked = clickable
                    && resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked();
                if clicked {
                    match s.link {
                        Some(url) => ui.ctx().open_url(egui::OpenUrl::new_tab(url)),
                        None => self.tab = Tab::Ride,
                    }
                }
            }
        }
    }

    fn menu_divider(&self, ui: &egui::Ui, pill: Rect, y: f32) {
        ui.painter().line_segment(
            [
                Pos2::new(pill.left() + PAD + 8.0, y),
                Pos2::new(pill.right() - PAD - 8.0, y),
            ],
            Stroke::new(1.0, Color32::from_white_alpha(46)),
        );
    }

    /// One place in the menu: a circle with its icon and name, lit when
    /// it is open or under the pointer.
    fn menu_item(&mut self, ui: &mut egui::Ui, rect: Rect, tab: Tab) {
        let resp = ui
            .interact(rect, ui.scope_id().with(("menu", tab)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let open = self.tab == tab;
        let fill = if open {
            Some(Color32::from_white_alpha(62))
        } else if resp.hovered() {
            Some(Color32::from_white_alpha(36))
        } else {
            None
        };
        let p = ui.painter();
        if let Some(f) = fill {
            p.circle_filled(rect.center(), ITEM / 2.0, f);
        }
        let colour = if open || resp.hovered() {
            Color32::WHITE
        } else {
            Color32::from_rgb(226, 228, 236)
        };
        menu_icon(p, rect.center() - Vec2::new(0.0, 7.0), tab, colour);
        p.text(
            rect.center() + Vec2::new(0.0, 15.0),
            Align2::CENTER_CENTER,
            tab.label(),
            theme::medium(10.5),
            colour,
        );
        if resp.clicked() {
            self.tab = tab;
        }
    }

    /// What the bottom of the menu says: the ride being recorded and a new
    /// version. (How the devices are shows on Home and in Devices.)
    fn status_lines(&self, time: f64) -> Vec<StatusLine> {
        let mut out = Vec::new();
        if self.ride.state == ride::State::Running {
            out.push(StatusLine {
                text: fmt_dur(self.ride.elapsed as f32),
                hover: "Recording: click to go back to the ride".into(),
                colour: theme::BAD.gamma_multiply(0.55 + 0.45 * (time * 2.6).sin() as f32),
                glow: true,
                link: None,
                ride: true,
            });
        }
        if self.ride.state != ride::State::Running {
            if let Some(crate::update::Status::Available(r)) = &self.update {
                out.push(StatusLine {
                    text: "Update".into(),
                    hover: format!(
                        "VeloForge {} is available. Click to open its download page.",
                        r.version
                    ),
                    colour: ACCENT,
                    glow: true,
                    link: Some(r.url.clone()),
                    ride: false,
                });
            }
        }
        out
    }
}

/// A line at the bottom of the menu: a coloured dot and a word.
struct StatusLine {
    text: String,
    hover: String,
    colour: Color32,
    /// Lit up (connected, recording) rather than greyed out.
    glow: bool,
    /// Opened in the browser when clicked.
    link: Option<String>,
    /// Leads back to the ride when clicked.
    ride: bool,
}

/// The menu's icons, drawn in thin lines about 20 points across.
pub(super) fn menu_icon(p: &egui::Painter, c: Pos2, tab: Tab, colour: Color32) {
    let s = Stroke::new(1.6, colour);
    let at = |x: f32, y: f32| c + Vec2::new(x, y);
    let line = |pts: &[(f32, f32)]| {
        p.add(egui::Shape::line(
            pts.iter().map(|&(x, y)| at(x, y)).collect(),
            s,
        ));
    };
    match tab {
        Tab::Home => {
            line(&[(-8.5, -0.5), (0.0, -8.0), (8.5, -0.5)]);
            line(&[(-6.0, -2.5), (-6.0, 7.5), (6.0, 7.5), (6.0, -2.5)]);
            line(&[(-2.0, 7.5), (-2.0, 2.5), (2.0, 2.5), (2.0, 7.5)]);
        }
        Tab::Ride => {
            p.circle_stroke(at(-5.5, 3.0), 4.5, s);
            p.circle_stroke(at(5.5, 3.0), 4.5, s);
            line(&[(-5.5, 3.0), (-1.0, -3.5), (4.0, -3.5), (5.5, 3.0)]);
            line(&[(-1.0, -3.5), (0.5, 3.0), (-5.5, 3.0)]);
            line(&[(0.5, 3.0), (4.0, -3.5)]);
            line(&[(-3.0, -5.5), (0.0, -5.5)]);
            line(&[(2.5, -6.5), (4.0, -3.5)]);
        }
        Tab::Library => {
            p.rect_stroke(
                Rect::from_min_max(at(-8.0, -7.0), at(-3.5, 8.0)),
                1.0,
                s,
                egui::StrokeKind::Middle,
            );
            p.rect_stroke(
                Rect::from_min_max(at(-2.0, -5.0), at(2.5, 8.0)),
                1.0,
                s,
                egui::StrokeKind::Middle,
            );
            line(&[
                (4.0, -5.5),
                (8.5, -6.5),
                (9.0, 7.0),
                (6.0, 7.8),
                (4.0, -5.5),
            ]);
        }
        Tab::Calendar => {
            p.rect_stroke(
                Rect::from_min_max(at(-8.0, -5.5), at(8.0, 8.0)),
                2.0,
                s,
                egui::StrokeKind::Middle,
            );
            line(&[(-8.0, -1.0), (8.0, -1.0)]);
            line(&[(-4.0, -8.5), (-4.0, -3.5)]);
            line(&[(4.0, -8.5), (4.0, -3.5)]);
        }
        Tab::Activity => {
            line(&[
                (-9.0, 1.0),
                (-5.0, 1.0),
                (-2.0, -6.5),
                (2.0, 7.0),
                (5.0, 1.0),
                (9.0, 1.0),
            ]);
        }
        Tab::Devices => {
            line(&[
                (-5.0, -4.0),
                (5.0, 4.5),
                (0.0, 9.0),
                (0.0, -9.0),
                (5.0, -4.5),
                (-5.0, 4.0),
            ]);
        }
        Tab::Settings => {
            for (y, knob) in [(-5.5, -3.0), (0.0, 4.0), (5.5, -1.0)] {
                line(&[(-8.5, y), (8.5, y)]);
                p.circle_filled(at(knob, y), 2.6, colour);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_covers_the_screen_without_stretching() {
        // Wider screen than picture: full width, trimmed top and bottom.
        let r = cover([1600, 1000], Vec2::new(2000.0, 1000.0));
        assert!((r.width() - 1.0).abs() < 1e-6);
        assert!((r.height() - 0.8).abs() < 1e-6);
        assert!((r.center().y - 0.5).abs() < 1e-6);
        // Taller screen: full height, trimmed at the sides.
        let r = cover([1600, 1000], Vec2::new(800.0, 1000.0));
        assert!((r.height() - 1.0).abs() < 1e-6);
        assert!((r.width() - 0.5).abs() < 1e-6);
    }
}
