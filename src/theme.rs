//! The look of the app: colours, typefaces, widget styling and a few
//! painted building blocks (glass panels, pills) shared by every screen.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontFamily, FontId, Painter, Pos2, Rect, Stroke,
    StrokeKind, Vec2,
};

/// The accent colour, taken from the logo.
pub const ACCENT: Color32 = Color32::from_rgb(240, 112, 40);
/// Window background, the top bar, and surfaces raised above them.
pub const BG: Color32 = Color32::from_rgb(15, 17, 22);
pub const BAR: Color32 = Color32::from_rgb(11, 12, 16);
pub const CARD: Color32 = Color32::from_rgb(25, 28, 36);
pub const CARD_HI: Color32 = Color32::from_rgb(34, 38, 49);
pub const WELL: Color32 = Color32::from_rgb(10, 11, 15);
pub const TEXT: Color32 = Color32::from_rgb(232, 234, 240);
pub const TEXT_DIM: Color32 = Color32::from_rgb(158, 165, 178);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(108, 115, 130);
pub const GOOD: Color32 = Color32::from_rgb(74, 201, 120);
pub const WARN: Color32 = Color32::from_rgb(240, 182, 62);
pub const BAD: Color32 = Color32::from_rgb(236, 88, 78);
/// Hairline around cards and panels.
pub const HAIRLINE: Color32 = Color32::from_rgba_premultiplied(18, 19, 22, 18);

fn family(name: &'static str) -> FontFamily {
    FontFamily::Name(name.into())
}

/// Text at `size` in the regular weight.
pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
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
        family("medium"),
        family("semibold"),
        family("bold"),
    ]
}

/// Install Inter in four weights, ahead of egui's built-in fonts (which
/// stay as fallbacks for symbols Inter lacks). Digits are tabular, so live
/// numbers do not jitter.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let weights: [(&str, &'static [u8], FontFamily); 4] = [
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
        v.panel_fill = BG;
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
        v.selection.bg_fill = ACCENT.gamma_multiply(0.85);
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

/// A panel floating over the 3D view. Where the renderer frosts the glass
/// behind it only a sheen and an edge are painted; otherwise it is filled.
pub fn glass(painter: &Painter, rect: Rect, radius: f32, frosted: bool) {
    let r = CornerRadius::same(radius as u8);
    if frosted {
        painter.rect_filled(rect, r, Color32::from_rgba_unmultiplied(12, 14, 20, 70));
    } else {
        painter.rect_filled(rect, r, Color32::from_rgba_unmultiplied(10, 12, 17, 200));
    }
    painter.rect_stroke(
        rect,
        r,
        Stroke::new(1.0, Color32::from_white_alpha(26)),
        StrokeKind::Inside,
    );
}

/// A rounded card on an ordinary (opaque) screen.
pub fn card(painter: &Painter, rect: Rect, radius: f32) {
    let r = CornerRadius::same(radius as u8);
    painter.rect_filled(rect, r, CARD);
    painter.rect_stroke(rect, r, Stroke::new(1.0, HAIRLINE), StrokeKind::Inside);
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
    let r = painter.text(pos, Align2::LEFT_BOTTOM, text, semibold(size), color);
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
