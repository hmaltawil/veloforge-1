//! The look of the app: colours, typefaces, widget styling and a few
//! painted building blocks (glass panels, pills) shared by every screen.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontFamily, FontId, Id, Painter, Pos2, Rect, Stroke,
    StrokeKind, TextureId, Vec2,
};

/// The accent colour, taken from the logo.
pub const ACCENT: Color32 = Color32::from_rgb(240, 112, 40);
/// Window background, and surfaces raised above it.
pub const BG: Color32 = Color32::from_rgb(15, 17, 22);
/// Cards and raised surfaces are a light veil rather than a solid colour,
/// so on the glass screens the picture shows through; on the dark ride
/// screen they come out close to the old solid blue-greys.
pub const CARD: Color32 = Color32::from_rgba_premultiplied(15, 15, 15, 15);
pub const CARD_HI: Color32 = Color32::from_rgba_premultiplied(28, 28, 28, 28);
pub const WELL: Color32 = Color32::from_rgb(10, 11, 15);
pub const TEXT: Color32 = Color32::from_rgb(232, 234, 240);
pub const TEXT_DIM: Color32 = Color32::from_rgb(158, 165, 178);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(108, 115, 130);
pub const GOOD: Color32 = Color32::from_rgb(74, 201, 120);
pub const WARN: Color32 = Color32::from_rgb(240, 182, 62);
pub const BAD: Color32 = Color32::from_rgb(236, 88, 78);

fn family(name: &'static str) -> FontFamily {
    FontFamily::Name(name.into())
}

/// Text at `size` in the regular weight.
pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}
/// Text at `size` in the light weight (big figures, when chosen).
pub fn light(size: f32) -> FontId {
    FontId::new(size, family("light"))
}
pub fn medium(size: f32) -> FontId {
    FontId::new(size, family("medium"))
}
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, family("semibold"))
}
pub fn bold(size: f32) -> FontId {
    FontId::new(size, family("bold"))
}
/// The font families a fallback font should join.
pub fn families() -> Vec<FontFamily> {
    vec![
        FontFamily::Proportional,
        FontFamily::Monospace,
        family("light"),
        family("medium"),
        family("semibold"),
        family("bold"),
    ]
}

/// Install Inter in five weights, ahead of egui's built-in fonts (which
/// stay as fallbacks for symbols Inter lacks). Digits are tabular, so live
/// numbers do not jitter.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let weights: [(&str, &'static [u8], FontFamily); 5] = [
        (
            "inter-light",
            include_bytes!("../assets/fonts/Inter-Light.otf"),
            family("light"),
        ),
        (
            "inter",
            include_bytes!("../assets/fonts/Inter-Regular.otf"),
            FontFamily::Proportional,
        ),
        (
            "inter-medium",
            include_bytes!("../assets/fonts/Inter-Medium.otf"),
            family("medium"),
        ),
        (
            "inter-semibold",
            include_bytes!("../assets/fonts/Inter-SemiBold.otf"),
            family("semibold"),
        ),
        (
            "inter-bold",
            include_bytes!("../assets/fonts/Inter-Bold.otf"),
            family("bold"),
        ),
    ];
    for (name, bytes, fam) in weights {
        fonts.font_data.insert(
            name.to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(bytes)),
        );
        let mut list = vec![name.to_owned()];
        list.extend(fallbacks.iter().cloned());
        fonts.families.insert(fam, list);
    }
    ctx.set_fonts(fonts);
}

/// A calm dark theme: blue-black surfaces, soft corners, an orange accent.
pub fn apply(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.all_styles_mut(|style| {
        use egui::TextStyle;
        style.text_styles = [
            (TextStyle::Small, regular(11.0)),
            (TextStyle::Body, regular(13.5)),
            (TextStyle::Button, medium(13.0)),
            (TextStyle::Heading, semibold(20.0)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into();
        let mut v = egui::Visuals::dark();
        // Panels are clear so the screens' glass and picture show through;
        // the ride screen fills its own with `BG`.
        v.panel_fill = Color32::TRANSPARENT;
        v.window_fill = Color32::from_rgb(24, 27, 35);
        v.extreme_bg_color = WELL;
        v.faint_bg_color = Color32::from_rgb(21, 24, 31);
        v.code_bg_color = WELL;
        v.override_text_color = None;
        v.window_corner_radius = CornerRadius::same(14);
        v.menu_corner_radius = CornerRadius::same(10);
        v.window_stroke = Stroke::new(1.0, Color32::from_white_alpha(20));
        v.window_shadow = egui::Shadow {
            offset: [0, 12],
            blur: 36,
            spread: 0,
            color: Color32::from_black_alpha(140),
        };
        v.popup_shadow = egui::Shadow {
            offset: [0, 6],
            blur: 18,
            spread: 0,
            color: Color32::from_black_alpha(110),
        };
        // Chosen items are lit with white, like the glass; the accent is
        // kept for actions and data.
        v.selection.bg_fill = Color32::from_white_alpha(52);
        v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        v.hyperlink_color = Color32::from_rgb(125, 180, 250);
        v.warn_fg_color = WARN;
        v.error_fg_color = BAD;
        v.slider_trailing_fill = true;
        v.handle_shape = egui::style::HandleShape::Circle;
        let w = &mut v.widgets;
        for wv in [
            &mut w.noninteractive,
            &mut w.inactive,
            &mut w.hovered,
            &mut w.active,
            &mut w.open,
        ] {
            wv.corner_radius = CornerRadius::same(8);
        }
        w.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_white_alpha(16));
        w.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(198, 203, 214));
        w.noninteractive.bg_fill = CARD;
        w.inactive.weak_bg_fill = CARD_HI;
        w.inactive.bg_fill = CARD_HI;
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(214, 218, 228));
        w.hovered.weak_bg_fill = Color32::from_rgb(46, 51, 65);
        w.hovered.bg_fill = Color32::from_rgb(46, 51, 65);
        w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_white_alpha(30));
        w.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        w.active.weak_bg_fill = Color32::from_rgb(56, 62, 78);
        w.active.bg_fill = Color32::from_rgb(56, 62, 78);
        w.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        w.open.weak_bg_fill = Color32::from_rgb(46, 51, 65);
        w.open.bg_fill = Color32::from_rgb(46, 51, 65);
        style.visuals = v;
        style.spacing.item_spacing = egui::vec2(8.0, 7.0);
        style.spacing.button_padding = egui::vec2(11.0, 5.0);
        style.spacing.interact_size.y = 28.0;
        style.spacing.window_margin = egui::Margin::same(16);
        style.spacing.menu_margin = egui::Margin::same(8);
        style.spacing.combo_height = 320.0;
        style.spacing.scroll.bar_width = 8.0;
        style.spacing.scroll.floating = true;
    });
}

/// Where the frosted copy of the picture behind the screens is, so glass
/// panels can show the part of it that lies behind them.
#[derive(Clone, Copy, Debug)]
pub struct Backdrop {
    pub frosted: TextureId,
    /// The screen area the picture covers, and the part of the picture
    /// (0-1) shown there.
    pub screen: Rect,
    pub uv: Rect,
    /// The veil over the glass (Settings › Appearance › Panel darkness).
    pub tint: Color32,
}

fn backdrop_id() -> Id {
    Id::unique("theme-backdrop")
}

/// What is behind the glass this frame, if anything.
pub fn backdrop(ctx: &egui::Context) -> Option<Backdrop> {
    ctx.data(|d| d.get_temp::<Backdrop>(backdrop_id()))
}

/// Tell the glass panels what is behind them this frame.
pub fn set_backdrop(ctx: &egui::Context, b: Backdrop) {
    ctx.data_mut(|d| d.insert_temp(backdrop_id(), b));
}

/// A frosted glass panel over the picture behind the screens: the soft
/// copy of the picture where the panel is, a dark tint so text reads on
/// any picture, a hairline edge, a faint light along the top and a deep
/// shadow.
pub fn frosted(painter: &Painter, rect: Rect, radius: f32) {
    let r = CornerRadius::same(radius.min(255.0) as u8);
    painter.add(
        egui::Shadow {
            offset: [0, 30],
            blur: 80,
            spread: 0,
            color: Color32::from_black_alpha(71),
        }
        .as_shape(rect, r),
    );
    match backdrop(painter.ctx()) {
        Some(b) if b.screen.width() > 0.0 && b.screen.height() > 0.0 => {
            let at = |p: Pos2| {
                let t = (p - b.screen.min) / b.screen.size();
                b.uv.min + t * b.uv.size()
            };
            let uv = Rect::from_min_max(at(rect.min), at(rect.max));
            painter.add(
                egui::epaint::RectShape::filled(rect, r, Color32::WHITE)
                    .with_texture(b.frosted, uv),
            );
            painter.rect_filled(rect, r, b.tint);
        }
        _ => {
            painter.rect_filled(rect, r, Color32::from_rgb(28, 30, 38));
        }
    }
    painter.rect_stroke(
        rect,
        r,
        Stroke::new(1.0, Color32::from_white_alpha(41)),
        StrokeKind::Inside,
    );
    // Light catching the top edge.
    let top = Rect::from_min_max(
        rect.min,
        Pos2::new(rect.max.x, rect.min.y + radius.max(2.0)),
    );
    painter
        .with_clip_rect(top.intersect(painter.clip_rect()))
        .rect_stroke(
            rect.shrink(1.0),
            r,
            Stroke::new(1.0, Color32::from_white_alpha(36)),
            StrokeKind::Inside,
        );
}

/// The dark veil over frosted glass, so white text reads on any picture:
/// `darkness` percent of a warm grey.
pub fn glass_tint(darkness: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(40, 42, 48, (darkness.min(100) as f32 * 2.55).round() as u8)
}

/// A rounded card on an ordinary (opaque) screen.
pub fn card(painter: &Painter, rect: Rect, radius: f32) {
    frosted(painter, rect, radius);
}

/// Small capitals above a number.
pub fn caption(painter: &Painter, pos: Pos2, align: Align2, text: &str, color: Color32) -> Rect {
    painter.text(pos, align, text, medium(10.5), color)
}

/// A number with its unit set smaller beside it; returns the space used.
pub fn value(
    painter: &Painter,
    pos: Pos2,
    text: &str,
    unit: &str,
    size: f32,
    color: Color32,
) -> Rect {
    let r = painter.text(pos, Align2::LEFT_BOTTOM, text, figure(size), color);
    if unit.is_empty() {
        return r;
    }
    // Sit the unit on the number's baseline.
    let u = painter.text(
        Pos2::new(r.right() + size * 0.1 + 2.0, pos.y - size * 0.13),
        Align2::LEFT_BOTTOM,
        unit,
        medium((size * 0.3).clamp(11.0, 16.0)),
        TEXT_DIM,
    );
    r.union(u)
}

/// A rounded label: tinted background, coloured text.
pub fn pill(painter: &Painter, pos: Pos2, align: Align2, text: &str, color: Color32) -> Rect {
    let galley = painter.layout_no_wrap(text.to_owned(), medium(11.0), color);
    let size = galley.size() + Vec2::new(14.0, 6.0);
    let rect = align.anchor_size(pos, size);
    painter.rect_filled(
        rect,
        CornerRadius::same(9),
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 38),
    );
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
    rect
}

static BOLD_FIGURES: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Show figures bold (Settings › Appearance › Numbers) or light.
pub fn set_bold_figures(bold: bool) {
    BOLD_FIGURES.store(bold, std::sync::atomic::Ordering::Relaxed);
}

/// The font for a big figure: bold or light, as chosen in Settings.
pub fn figure(size: f32) -> FontId {
    if BOLD_FIGURES.load(std::sync::atomic::Ordering::Relaxed) {
        bold(size)
    } else {
        light(size)
    }
}

/// Mix two colours (0: `a`, 1: `b`).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(
        f(a.r(), b.r()),
        f(a.g(), b.g()),
        f(a.b(), b.b()),
        f(a.a(), b.a()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_blends_between_its_ends() {
        let (a, b) = (
            Color32::from_rgb(0, 100, 200),
            Color32::from_rgb(100, 200, 0),
        );
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
        assert_eq!(mix(a, b, 0.5), Color32::from_rgb(50, 150, 100));
        assert_eq!(mix(a, b, 7.0), b);
    }

    #[test]
    fn bundled_fonts_are_opentype() {
        for bytes in [
            &include_bytes!("../assets/fonts/Inter-Light.otf")[..],
            &include_bytes!("../assets/fonts/Inter-Regular.otf")[..],
            &include_bytes!("../assets/fonts/Inter-Medium.otf")[..],
            &include_bytes!("../assets/fonts/Inter-SemiBold.otf")[..],
            &include_bytes!("../assets/fonts/Inter-Bold.otf")[..],
        ] {
            assert_eq!(&bytes[..4], b"OTTO");
            assert!(bytes.len() > 20_000);
        }
    }
}
