//! Small pieces of interface the screens share: icons and buttons, tabs,
//! list rows, segmented choices, the date picker, number fields, tiles
//! and power zones.

use super::*;

/// Icons are painted as shapes so they never depend on installed fonts.
#[derive(Clone, Copy)]
pub(super) enum Icon {
    Left,
    Right,
    Play,
    Check,
}

pub(super) fn paint_icon(painter: &egui::Painter, r: Rect, icon: Icon, c: Color32) {
    let (x0, x1, y0, y1, cx, cy) = (
        r.left(),
        r.right(),
        r.top(),
        r.bottom(),
        r.center().x,
        r.center().y,
    );
    let tri = |pts: Vec<Pos2>| egui::Shape::convex_polygon(pts, c, Stroke::NONE);
    let w = (r.width() * 0.18).max(1.5);
    match icon {
        Icon::Left => {
            painter.add(tri(vec![
                Pos2::new(x1 - 2.0, y0 + 1.0),
                Pos2::new(x1 - 2.0, y1 - 1.0),
                Pos2::new(x0 + 2.0, cy),
            ]));
        }
        Icon::Right | Icon::Play => {
            painter.add(tri(vec![
                Pos2::new(x0 + 2.0, y0 + 1.0),
                Pos2::new(x1 - 2.0, cy),
                Pos2::new(x0 + 2.0, y1 - 1.0),
            ]));
        }
        Icon::Check => {
            let pts = vec![
                Pos2::new(x0 + 1.0, cy),
                Pos2::new(cx - 1.0, y1 - 2.0),
                Pos2::new(x1 - 1.0, y0 + 2.0),
            ];
            painter.add(egui::Shape::line(pts, Stroke::new(w, c)));
        }
    }
}

/// A button with a painted icon and optional text and fill colour.
pub(super) fn icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    text: &str,
    fill: Option<Color32>,
) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, Color32::WHITE);
    let pad = ui.spacing().button_padding;
    let icon_w = galley.size().y.max(12.0) * 0.8;
    let gap = if text.is_empty() { 0.0 } else { 6.0 };
    let size = Vec2::new(
        pad.x * 2.0 + icon_w + gap + galley.size().x,
        (pad.y * 2.0 + galley.size().y).max(ui.spacing().interact_size.y),
    );
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let vis = ui.style().interact(&resp);
        let bg = match fill {
            Some(f) if resp.hovered() => f.gamma_multiply(1.2),
            Some(f) => f,
            None => vis.weak_bg_fill,
        };
        ui.painter().rect(
            rect,
            vis.corner_radius,
            bg,
            vis.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let c = if fill.is_some() {
            Color32::WHITE
        } else {
            vis.text_color()
        };
        let ir = Rect::from_center_size(
            Pos2::new(rect.left() + pad.x + icon_w / 2.0, rect.center().y),
            Vec2::splat(icon_w),
        );
        paint_icon(ui.painter(), ir, icon, c);
        ui.painter().galley_with_override_text_color(
            Pos2::new(ir.right() + gap, rect.center().y - galley.size().y / 2.0),
            galley,
            c,
        );
    }
    resp
}

/// A row in a list that fills the width: a stripe of colour, a name, and
/// details set quieter on the right.
pub(super) fn list_row(
    ui: &mut egui::Ui,
    selected: bool,
    name: &str,
    meta: &str,
    colour: Color32,
) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if selected {
            p.rect_filled(rect, CornerRadius::same(8), theme::CARD_HI);
        } else if resp.hovered() {
            p.rect_filled(rect, CornerRadius::same(8), Color32::from_white_alpha(9));
        }
        let stripe = Rect::from_center_size(
            Pos2::new(rect.left() + 7.0, rect.center().y),
            Vec2::new(3.0, if selected { 18.0 } else { 12.0 }),
        );
        p.rect_filled(
            stripe,
            CornerRadius::same(2),
            if selected {
                colour
            } else {
                colour.gamma_multiply(0.6)
            },
        );
        let m = p.text(
            Pos2::new(rect.right() - 10.0, rect.center().y),
            Align2::RIGHT_CENTER,
            meta,
            theme::regular(12.0),
            if selected {
                theme::TEXT
            } else {
                theme::TEXT_FAINT
            },
        );
        // The name gives way to the details when the list is narrow.
        p.with_clip_rect(Rect::from_min_max(
            rect.min,
            Pos2::new(m.left() - 8.0, rect.bottom()),
        ))
        .text(
            Pos2::new(rect.left() + 18.0, rect.center().y),
            Align2::LEFT_CENTER,
            name,
            if selected {
                theme::medium(13.5)
            } else {
                theme::regular(13.5)
            },
            if selected {
                Color32::WHITE
            } else {
                theme::TEXT
            },
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A row of choices in one rounded well, the chosen one raised. Returns
/// whether the choice changed. Each option: value, label, hover text.
pub(super) fn segmented<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    value: &mut T,
    options: &[(T, &str, &str)],
) -> bool {
    let font = theme::medium(13.0);
    let galleys: Vec<_> = options
        .iter()
        .map(|o| {
            ui.painter()
                .layout_no_wrap(o.1.to_string(), font.clone(), Color32::WHITE)
        })
        .collect();
    let h = ui.spacing().interact_size.y;
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 22.0).collect();
    let total: f32 = widths.iter().sum::<f32>() + 6.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total, h), Sense::hover());
    let enabled = ui.is_enabled();
    let round = CornerRadius::same((h / 2.0) as u8);
    ui.painter()
        .rect_filled(rect, round, Color32::from_white_alpha(14));
    let mut changed = false;
    let mut x = rect.left() + 3.0;
    for (i, (o, galley)) in options.iter().zip(galleys).enumerate() {
        let r = Rect::from_min_size(
            Pos2::new(x, rect.top() + 3.0),
            Vec2::new(widths[i], h - 6.0),
        );
        x += widths[i];
        let resp = ui
            .interact(r, ui.scope_id().with(("segment", o.1)), Sense::click())
            .on_hover_text(o.2);
        let selected = *value == o.0;
        if resp.clicked() && !selected {
            *value = o.0;
            changed = true;
        }
        let color = if !enabled {
            if selected {
                theme::TEXT_DIM
            } else {
                Color32::from_gray(78)
            }
        } else if selected {
            Color32::WHITE
        } else if resp.hovered() {
            theme::TEXT
        } else {
            theme::TEXT_DIM
        };
        if selected {
            ui.painter().rect_filled(
                r,
                CornerRadius::same(((h - 6.0) / 2.0) as u8),
                Color32::from_white_alpha(if enabled { 44 } else { 22 }),
            );
        }
        ui.painter().galley_with_override_text_color(
            r.center() - galley.size() / 2.0,
            galley,
            color,
        );
    }
    changed
}

/// Paint `text` as a QR code with `px`-sized modules on a white card.
pub(super) fn qr_code(ui: &mut egui::Ui, text: &str, px: f32) {
    let Ok(code) = qrcode::QrCode::new(text.as_bytes()) else {
        return;
    };
    let n = code.width();
    let quiet = 3;
    let size = (n + 2 * quiet) as f32 * px;
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 8.0, Color32::WHITE);
    for (i, c) in code.to_colors().iter().enumerate() {
        if *c == qrcode::Color::Dark {
            let (x, y) = ((i % n + quiet) as f32, (i / n + quiet) as f32);
            painter.rect_filled(
                Rect::from_min_size(rect.min + Vec2::new(x * px, y * px), Vec2::splat(px)),
                0.0,
                Color32::BLACK,
            );
        }
    }
}

/// A gap in metres as ("35", "m ahead"), or ("115", "ft ahead").
pub(super) fn gap_text(gap: f64, units: Units) -> (String, String) {
    let side = if gap >= 0.0 { "ahead" } else { "behind" };
    let feet = units.height(gap.abs());
    if units.imperial() && feet < 1000.0 {
        (format!("{feet:.0}"), format!("ft {side}"))
    } else if units.imperial() || gap.abs() >= 1000.0 {
        (
            format!("{:.2}", units.distance(gap.abs())),
            format!("{} {side}", units.distance_unit()),
        )
    } else {
        (format!("{:.0}", gap.abs()), format!("m {side}"))
    }
}

/// Long durations as "12 h 53 min".
pub(super) fn fmt_hours(secs: f32) -> String {
    let m = (secs / 60.0).round() as u32;
    if m < 60 {
        format!("{m} min")
    } else {
        format!("{} h {:02} min", m / 60, m % 60)
    }
}

pub(super) fn date_picker(ui: &mut egui::Ui, date: &mut NaiveDate) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        if icon_button(ui, Icon::Left, "", None)
            .on_hover_text("Previous day")
            .clicked()
        {
            *date -= Duration::days(1);
            changed = true;
        }
        ui.label(RichText::new(date.format("%a %d %b %Y").to_string()).font(theme::medium(13.0)));
        if icon_button(ui, Icon::Right, "", None)
            .on_hover_text("Next day")
            .clicked()
        {
            *date += Duration::days(1);
            changed = true;
        }
        if ui.small_button("Today").clicked() {
            *date = Local::now().date_naive();
            changed = true;
        }
    });
    changed
}

pub(crate) fn secs_edit(ui: &mut egui::Ui, secs: &mut u32) -> bool {
    ui.add(
        egui::DragValue::new(secs)
            .range(5..=4 * 3600)
            .speed(5.0)
            .custom_formatter(|v, _| fmt_dur(v as f32))
            .custom_parser(|s| {
                let parts: Vec<&str> = s.trim().split(':').collect();
                match parts.as_slice() {
                    [m, s] => Some(m.parse::<f64>().ok()? * 60.0 + s.parse::<f64>().ok()?),
                    [v] => {
                        let v = v.trim_end_matches('s');
                        if let Some(m) = v.strip_suffix('m') {
                            Some(m.parse::<f64>().ok()? * 60.0)
                        } else {
                            v.parse().ok()
                        }
                    }
                    _ => None,
                }
            }),
    )
    .changed()
}

pub(crate) fn pct_edit(ui: &mut egui::Ui, frac: &mut f32) -> bool {
    let mut pct = *frac * 100.0;
    let changed = ui
        .add(
            egui::DragValue::new(&mut pct)
                .range(0.0..=300.0)
                .speed(1.0)
                .suffix("%")
                .max_decimals(0),
        )
        .changed();
    if changed {
        *frac = pct / 100.0;
    }
    changed
}

/// A step's cadence target in rpm; 0 ("any rpm") means none.
pub(crate) fn cadence_edit(ui: &mut egui::Ui, cadence: &mut Option<u32>) -> bool {
    let mut rpm = cadence.unwrap_or(0);
    let changed = ui
        .add(
            egui::DragValue::new(&mut rpm)
                .range(0..=200)
                .speed(1.0)
                .custom_formatter(|v, _| {
                    if v < 1.0 {
                        "any rpm".into()
                    } else {
                        format!("{v:.0} rpm")
                    }
                })
                .custom_parser(|s| {
                    let s = s.trim().trim_end_matches("rpm").trim();
                    if s.is_empty() || s == "-" || s.eq_ignore_ascii_case("any") {
                        Some(0.0)
                    } else {
                        s.parse().ok()
                    }
                }),
        )
        .on_hover_text("Target cadence for this step (0 = no target)")
        .changed();
    if changed {
        *cadence = (rpm > 0).then_some(rpm);
    }
    changed
}

/// A dashboard tile: caption, a big number with its unit, and a stripe of
/// colour down the left edge.
pub(super) fn tile(
    painter: &egui::Painter,
    rect: Rect,
    label: &str,
    value: &str,
    unit: &str,
    accent: Color32,
) {
    theme::card(painter, rect, 22.0);
    painter.rect_filled(
        Rect::from_min_max(
            rect.left_top() + Vec2::new(0.0, 12.0),
            rect.left_bottom() + Vec2::new(3.0, -12.0),
        ),
        CornerRadius::same(2),
        accent,
    );
    theme::caption(
        painter,
        rect.left_top() + Vec2::new(16.0, 10.0),
        Align2::LEFT_TOP,
        label,
        theme::TEXT_DIM,
    );
    theme::value(
        painter,
        rect.left_bottom() + Vec2::new(14.0, -4.0),
        value,
        unit,
        (rect.height() * 0.52).min(rect.width() * 0.34),
        Color32::WHITE,
    );
}

pub(super) const POWER_ZONE_NAMES: [&str; 6] = [
    "Z1 Recovery",
    "Z2 Endurance",
    "Z3 Tempo",
    "Z4 Threshold",
    "Z5 VO2 max",
    "Z6 Anaerobic",
];

pub(super) fn power_zone_color(z: usize) -> Color32 {
    charts::zone_color([0.5, 0.65, 0.85, 1.0, 1.15, 1.4][z.min(5)])
}

/// Time in power and heart rate zones as labelled bars.
pub(super) fn zone_breakdown(ui: &mut egui::Ui, s: &Summary) {
    let rows = |ui: &mut egui::Ui,
                title: &str,
                secs: &[f32],
                names: &[&str],
                color: &dyn Fn(usize) -> Color32| {
        let total: f32 = secs.iter().sum();
        if total <= 0.0 {
            return;
        }
        ui.add_space(6.0);
        ui.label(RichText::new(title).strong());
        for (i, sec) in secs.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_sized([100.0, 16.0], egui::Label::new(names[i]));
                let (r, _) = ui.allocate_exact_size(Vec2::new(160.0, 12.0), Sense::hover());
                ui.painter()
                    .rect_filled(r, 2.0, Color32::from_white_alpha(20));
                ui.painter().rect_filled(
                    Rect::from_min_size(r.min, Vec2::new(r.width() * sec / total, r.height())),
                    2.0,
                    color(i),
                );
                ui.label(format!("{}  {:.0}%", fmt_dur(*sec), sec / total * 100.0));
            });
        }
    };
    rows(
        ui,
        "Time in power zones",
        &s.power_zones,
        &POWER_ZONE_NAMES,
        &power_zone_color,
    );
    rows(
        ui,
        "Time in heart rate zones",
        &s.hr_zones,
        &charts::HR_ZONE_NAMES,
        &charts::hr_zone_color,
    );
}
