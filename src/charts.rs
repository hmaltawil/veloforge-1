//! Small painter-based charts: workout profiles, elevation, fitness trends.

use crate::route::Route;
use crate::theme;
use crate::workout::{Category, Segment};
use eframe::egui::{Align2, Color32, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke};

pub fn zone_color(frac: f32) -> Color32 {
    match frac {
        f if f < 0.56 => Color32::from_rgb(130, 136, 150),
        f if f < 0.76 => Color32::from_rgb(58, 132, 222),
        f if f < 0.91 => Color32::from_rgb(64, 178, 96),
        f if f < 1.06 => Color32::from_rgb(236, 196, 52),
        f if f < 1.21 => Color32::from_rgb(240, 128, 48),
        _ => Color32::from_rgb(222, 58, 58),
    }
}

pub fn category_color(c: Category) -> Color32 {
    match c {
        Category::Recovery => zone_color(0.5),
        Category::Endurance => zone_color(0.7),
        Category::Tempo => zone_color(0.85),
        Category::SweetSpot => Color32::from_rgb(200, 190, 60),
        Category::Threshold => zone_color(1.0),
        Category::Vo2Max => zone_color(1.15),
        Category::Anaerobic => zone_color(1.5),
        Category::Test => Color32::from_rgb(170, 90, 220),
        Category::Custom => Color32::from_rgb(90, 170, 190),
        Category::Intervals => Color32::from_rgb(120, 150, 230),
    }
}

/// Colour for a road gradient in percent: blue downhill, green on the
/// flat, through yellow and orange to red and purple on the steepest climbs.
pub fn grade_color(grade: f32) -> Color32 {
    const STOPS: [(f32, [u8; 3]); 7] = [
        (-8.0, [84, 120, 190]),
        (-3.0, [86, 150, 168]),
        (0.0, [84, 176, 118]),
        (2.5, [150, 196, 84]),
        (5.0, [238, 180, 60]),
        (8.0, [230, 84, 62]),
        (13.0, [168, 72, 186]),
    ];
    let g = grade.clamp(STOPS[0].0, STOPS[STOPS.len() - 1].0);
    let i = STOPS
        .windows(2)
        .position(|w| g <= w[1].0)
        .unwrap_or(STOPS.len() - 2);
    let (a, b) = (STOPS[i], STOPS[i + 1]);
    let t = (g - a.0) / (b.0 - a.0);
    let f = |k: usize| (a.1[k] as f32 + (b.1[k] as f32 - a.1[k] as f32) * t).round() as u8;
    Color32::from_rgb(f(0), f(1), f(2))
}

/// Workout profile with optional progress marker and actual power overlay.
/// Without a background it is drawn for a glass panel.
#[allow(clippy::too_many_arguments)]
pub fn workout_profile(
    painter: &Painter,
    rect: Rect,
    segs: &[Segment],
    ftp: f32,
    intensity: f32,
    progress: Option<f32>,
    actual: &[(f32, f32)],
    bg: Option<Color32>,
) {
    if let Some(bg) = bg {
        painter.rect_filled(rect, 10.0, bg);
    }
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let plot = rect.shrink2(eframe::egui::vec2(10.0, 8.0));
    let total = segs.last().map(|s| s.end()).unwrap_or(1.0).max(1.0);
    let max = segs.iter().map(|s| s.from.max(s.to)).fold(1.3f32, f32::max) * intensity * 1.08;
    let x = |t: f32| plot.left() + t / total * plot.width();
    let y = |p: f32| plot.bottom() - (p / max).clamp(0.0, 1.0) * plot.height();
    let mut mesh = Mesh::default();
    // A hair of space between steps, where they are wide enough to spare it.
    let gap = if plot.width() / segs.len().max(1) as f32 > 6.0 {
        0.75
    } else {
        0.0
    };
    for s in segs {
        let (a, b) = (s.from * intensity, s.to * intensity);
        let mut col = if s.free {
            Color32::from_rgb(110, 116, 128)
        } else {
            zone_color((s.from + s.to) / 2.0)
        };
        if progress.is_some_and(|p| p >= s.end()) {
            col = col.gamma_multiply(0.45);
        }
        let (h0, h1) = if s.free { (0.5, 0.5) } else { (a, b) };
        let base = mesh.vertices.len() as u32;
        let foot = col.gamma_multiply(0.55);
        mesh.colored_vertex(Pos2::new(x(s.start) + gap, y(h0)), col);
        mesh.colored_vertex(Pos2::new(x(s.end()) - gap, y(h1)), col);
        mesh.colored_vertex(Pos2::new(x(s.end()) - gap, plot.bottom()), foot);
        mesh.colored_vertex(Pos2::new(x(s.start) + gap, plot.bottom()), foot);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    painter.add(Shape::mesh(mesh));
    // FTP line.
    let fy = y(1.0);
    painter.extend(Shape::dashed_line(
        &[Pos2::new(plot.left(), fy), Pos2::new(plot.right(), fy)],
        Stroke::new(1.0, Color32::from_white_alpha(80)),
        4.0,
        4.0,
    ));
    painter.text(
        Pos2::new(plot.left() + 2.0, fy - 2.0),
        Align2::LEFT_BOTTOM,
        format!("FTP {ftp:.0} W"),
        crate::theme::medium(10.0),
        Color32::from_white_alpha(170),
    );
    if !actual.is_empty() && ftp > 0.0 {
        let pts: Vec<Pos2> = actual
            .iter()
            .step_by((actual.len() / 600).max(1))
            .map(|&(t, p)| Pos2::new(x(t), y(p / ftp)))
            .collect();
        painter.add(Shape::line(
            pts.clone(),
            Stroke::new(3.5, Color32::from_black_alpha(90)),
        ));
        painter.add(Shape::line(pts, Stroke::new(1.5, Color32::WHITE)));
    }
    if let Some(p) = progress {
        let px = x(p.min(total));
        painter.line_segment(
            [
                Pos2::new(px, rect.top() + 3.0),
                Pos2::new(px, rect.bottom() - 3.0),
            ],
            Stroke::new(2.0, Color32::WHITE),
        );
    }
}

/// The route's elevation, coloured by gradient, with the rider's position.
/// Without a background it is drawn for a glass panel.
pub fn elevation_profile(
    painter: &Painter,
    rect: Rect,
    route: &Route,
    d: Option<f64>,
    bg: Option<Color32>,
) {
    if let Some(bg) = bg {
        painter.rect_filled(rect, 10.0, bg);
    }
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let plot = Rect::from_min_max(
        Pos2::new(rect.left() + 10.0, rect.top() + 22.0),
        Pos2::new(rect.right() - 10.0, rect.bottom() - 6.0),
    );
    let samples = (plot.width() / 2.5).clamp(60.0, 600.0) as usize;
    let prof = route.profile(samples);
    let (lo, hi) = route.min_max_ele();
    let span = (hi - lo).max(30.0);
    let x = |dist: f64| plot.left() + (dist / route.length) as f32 * plot.width();
    let y = |e: f64| plot.bottom() - 3.0 - ((e - lo) / span) as f32 * (plot.height() - 5.0);
    let here = d.map(|d| d.rem_euclid(route.length));
    let mut mesh = Mesh::default();
    let mut crest = Vec::with_capacity(prof.len());
    // Gradient over a few samples, so single bumps do not flicker in colour.
    let reach = (samples / 60).max(1);
    let colour = |i: usize| {
        let (a, b) = (i.saturating_sub(reach), (i + reach).min(prof.len() - 1));
        let g = (prof[b].1 - prof[a].1) / (prof[b].0 - prof[a].0).max(1.0) * 100.0;
        let c = grade_color(g as f32);
        // What is already behind you fades back.
        if here.is_some_and(|h| prof[i].0 < h) {
            c.gamma_multiply(0.45)
        } else {
            c
        }
    };
    for (i, p) in prof.iter().enumerate() {
        let c = colour(i);
        let top = Pos2::new(x(p.0), y(p.1));
        crest.push(top);
        mesh.colored_vertex(top, c.gamma_multiply(0.85));
        mesh.colored_vertex(Pos2::new(top.x, plot.bottom()), c.gamma_multiply(0.18));
        if i > 0 {
            let b = (i as u32 - 1) * 2;
            mesh.add_triangle(b, b + 2, b + 1);
            mesh.add_triangle(b + 1, b + 2, b + 3);
        }
    }
    painter.add(Shape::mesh(mesh));
    painter.add(Shape::line(
        crest,
        Stroke::new(1.5, Color32::from_white_alpha(150)),
    ));
    painter.text(
        rect.left_top() + eframe::egui::vec2(12.0, 6.0),
        Align2::LEFT_TOP,
        &route.name,
        crate::theme::medium(11.5),
        Color32::from_white_alpha(225),
    );
    let stats = match here {
        Some(h) => format!(
            "{:.1} of {:.1} km  ·  {:.0} m climbing per lap",
            h / 1000.0,
            route.length / 1000.0,
            route.ascent
        ),
        None => format!(
            "{:.1} km  ·  {:.0} m climbing per lap",
            route.length / 1000.0,
            route.ascent
        ),
    };
    painter.text(
        rect.right_top() + eframe::egui::vec2(-12.0, 6.0),
        Align2::RIGHT_TOP,
        stats,
        crate::theme::regular(11.0),
        Color32::from_white_alpha(160),
    );
    if let Some(dd) = here {
        let p = Pos2::new(x(dd), y(route.at(dd).ele));
        painter.line_segment(
            [
                Pos2::new(p.x, plot.top() - 2.0),
                Pos2::new(p.x, plot.bottom()),
            ],
            Stroke::new(1.0, Color32::from_white_alpha(110)),
        );
        painter.circle_filled(p, 8.0, Color32::from_white_alpha(40));
        painter.circle_filled(p, 4.5, Color32::WHITE);
        painter.circle_stroke(p, 4.5, Stroke::new(1.5, Color32::from_black_alpha(160)));
    }
}

/// Simple multi-series line chart with a shared y range.
pub fn lines(
    painter: &Painter,
    rect: Rect,
    series: &[(&str, Color32, Vec<f32>)],
    bg: Color32,
    text: Color32,
) {
    painter.rect_filled(rect, 4.0, bg);
    let n = series.iter().map(|s| s.2.len()).max().unwrap_or(0);
    if n < 2 {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Ride a few times to see trends",
            FontId::proportional(13.0),
            text,
        );
        return;
    }
    let lo = series
        .iter()
        .flat_map(|s| s.2.iter().copied())
        .fold(0.0f32, f32::min);
    let hi = series
        .iter()
        .flat_map(|s| s.2.iter().copied())
        .fold(10.0f32, f32::max);
    let inner = rect.shrink2(eframe::egui::vec2(8.0, 20.0));
    let x = |i: usize| inner.left() + i as f32 / (n - 1) as f32 * inner.width();
    let y = |v: f32| inner.bottom() - (v - lo) / (hi - lo) * inner.height();
    if lo < 0.0 {
        painter.line_segment(
            [
                Pos2::new(inner.left(), y(0.0)),
                Pos2::new(inner.right(), y(0.0)),
            ],
            Stroke::new(1.0, text.gamma_multiply(0.3)),
        );
    }
    let mut lx = rect.left() + 8.0;
    for (name, col, vals) in series {
        let pts: Vec<Pos2> = vals
            .iter()
            .enumerate()
            .map(|(i, &v)| Pos2::new(x(i), y(v)))
            .collect();
        painter.add(Shape::line(pts, Stroke::new(2.0, *col)));
        let last = vals.last().copied().unwrap_or(0.0);
        let r = painter.text(
            Pos2::new(lx, rect.top() + 4.0),
            Align2::LEFT_TOP,
            format!("{name} {last:.0}"),
            FontId::proportional(12.0),
            *col,
        );
        lx = r.right() + 14.0;
    }
}

pub fn zone_name(frac: f32) -> &'static str {
    match frac {
        f if f < 0.56 => "Z1 Recovery",
        f if f < 0.76 => "Z2 Endurance",
        f if f < 0.91 => "Z3 Tempo",
        f if f < 1.06 => "Z4 Threshold",
        f if f < 1.21 => "Z5 VO2 max",
        _ => "Z6 Anaerobic",
    }
}

/// Rolling chart of the last `window` seconds: power (white), target
/// (zone-coloured steps) and heart rate (red, right axis).
#[allow(clippy::too_many_arguments)]
pub fn live_chart(
    painter: &Painter,
    rect: Rect,
    power: &[f32],
    target: &[Option<f32>],
    hr: &[Option<f32>],
    ftp: f32,
    window: usize,
    bg: Color32,
) {
    painter.rect_filled(rect, 6.0, bg);
    let inner = rect.shrink2(eframe::egui::vec2(44.0, 24.0));
    let label = |p: Pos2, a: Align2, t: String, c: Color32| {
        painter.text(p, a, t, FontId::proportional(11.0), c);
    };
    if power.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Power and heart rate chart appears once you start riding",
            FontId::proportional(14.0),
            Color32::from_gray(140),
        );
        return;
    }
    let n = power.len();
    let from = n.saturating_sub(window);
    let span = window.max(60) as f32;
    let pmax = power[from..]
        .iter()
        .copied()
        .chain(target[from..].iter().flatten().copied())
        .fold(ftp * 1.3, f32::max)
        * 1.08;
    let x = |i: usize| inner.right() - (n - 1 - i) as f32 / span * inner.width();
    let yp = |p: f32| inner.bottom() - (p / pmax).clamp(0.0, 1.0) * inner.height();
    let yh = |h: f32| inner.bottom() - ((h - 50.0) / 150.0).clamp(0.0, 1.0) * inner.height();
    // Grid: FTP line and watt labels.
    let grid = Color32::from_white_alpha(25);
    for w in [ftp * 0.5, ftp, ftp * 1.5] {
        if w < pmax {
            painter.line_segment(
                [
                    Pos2::new(inner.left(), yp(w)),
                    Pos2::new(inner.right(), yp(w)),
                ],
                Stroke::new(1.0, grid),
            );
            label(
                Pos2::new(inner.left() - 6.0, yp(w)),
                Align2::RIGHT_CENTER,
                format!("{w:.0}"),
                Color32::from_gray(150),
            );
        }
    }
    for h in [100.0, 150.0] {
        label(
            Pos2::new(inner.right() + 6.0, yh(h)),
            Align2::LEFT_CENTER,
            format!("{h:.0}"),
            Color32::from_rgb(230, 110, 120),
        );
    }
    // Target steps.
    let mut i = from;
    while i < n {
        if let Some(t) = target[i] {
            let mut j = i;
            while j + 1 < n && target[j + 1] == Some(t) {
                j += 1;
            }
            let r = Rect::from_min_max(
                Pos2::new(x(i), yp(t)),
                Pos2::new(x(j).max(x(i) + 1.0), inner.bottom()),
            );
            painter.rect_filled(r, 0.0, zone_color(t / ftp).gamma_multiply(0.35));
            painter.line_segment(
                [r.left_top(), r.right_top()],
                Stroke::new(2.0, zone_color(t / ftp)),
            );
            i = j + 1;
        } else {
            i += 1;
        }
    }
    let pts: Vec<Pos2> = (from..n).map(|i| Pos2::new(x(i), yp(power[i]))).collect();
    painter.add(Shape::line(pts, Stroke::new(1.8, Color32::WHITE)));
    let mut seg: Vec<Pos2> = Vec::new();
    for (i, h) in hr.iter().enumerate().skip(from) {
        match *h {
            Some(h) => seg.push(Pos2::new(x(i), yh(h))),
            None => {
                if seg.len() > 1 {
                    painter.add(Shape::line(
                        std::mem::take(&mut seg),
                        Stroke::new(1.8, Color32::from_rgb(230, 70, 90)),
                    ));
                }
                seg.clear();
            }
        }
    }
    if seg.len() > 1 {
        painter.add(Shape::line(
            seg,
            Stroke::new(1.8, Color32::from_rgb(230, 70, 90)),
        ));
    }
    label(
        Pos2::new(rect.left() + 10.0, rect.top() + 6.0),
        Align2::LEFT_TOP,
        format!("Last {} min", window / 60),
        Color32::from_gray(170),
    );
    label(
        Pos2::new(rect.left() + 90.0, rect.top() + 6.0),
        Align2::LEFT_TOP,
        "Power (W)".into(),
        Color32::WHITE,
    );
    label(
        Pos2::new(rect.left() + 160.0, rect.top() + 6.0),
        Align2::LEFT_TOP,
        "Heart rate (bpm)".into(),
        Color32::from_rgb(230, 70, 90),
    );
    label(
        Pos2::new(rect.left() + 270.0, rect.top() + 6.0),
        Align2::LEFT_TOP,
        "Target".into(),
        Color32::from_rgb(64, 178, 96),
    );
}

/// Heart rate zones as fractions of max heart rate: (name, low, high).
pub const HR_ZONES: [(&str, f32, f32); 5] = [
    ("Z1 Recovery", 0.50, 0.60),
    ("Z2 Endurance", 0.60, 0.70),
    ("Z3 Tempo", 0.70, 0.80),
    ("Z4 Threshold", 0.80, 0.90),
    ("Z5 Maximum", 0.90, 1.00),
];

/// Heart rate zone index (0-4); anything below Z2 counts as Z1.
pub fn hr_zone(hr: f32, max_hr: f32) -> usize {
    let f = hr / max_hr.max(1.0);
    HR_ZONES.iter().rposition(|z| f >= z.1).unwrap_or(0)
}

pub fn hr_zone_color(z: usize) -> Color32 {
    [
        Color32::from_rgb(130, 136, 150),
        Color32::from_rgb(58, 132, 222),
        Color32::from_rgb(64, 178, 96),
        Color32::from_rgb(240, 150, 48),
        Color32::from_rgb(222, 58, 58),
    ][z.min(4)]
}

/// Power zone index (0-5) for a fraction of FTP, matching `zone_color`.
pub fn power_zone(frac: f32) -> usize {
    [0.56, 0.76, 0.91, 1.06, 1.21]
        .iter()
        .filter(|&&t| frac >= t)
        .count()
}

/// Horizontal stacked bar of seconds per zone.
pub fn zone_bar(painter: &Painter, rect: Rect, secs: &[f32], color: impl Fn(usize) -> Color32) {
    let total: f32 = secs.iter().sum();
    painter.rect_filled(rect, 2.0, Color32::from_white_alpha(20));
    if total <= 0.0 {
        return;
    }
    let mut x = rect.left();
    for (i, s) in secs.iter().enumerate() {
        let w = rect.width() * s / total;
        if w > 0.0 {
            painter.rect_filled(
                Rect::from_min_max(Pos2::new(x, rect.top()), Pos2::new(x + w, rect.bottom())),
                0.0,
                color(i),
            );
        }
        x += w;
    }
}

/// Whole-ride chart: elevation (shaded), power (white, 30 s average over a
/// faint raw trace) and heart rate (red, right axis).
pub fn ride_chart(
    painter: &Painter,
    rect: Rect,
    samples: &[crate::ride::Sample],
    ftp: f32,
    bg: Color32,
) {
    painter.rect_filled(rect, 6.0, bg);
    let n = samples.len();
    if n < 2 {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "No data recorded",
            FontId::proportional(14.0),
            Color32::from_gray(140),
        );
        return;
    }
    let inner = rect.shrink2(eframe::egui::vec2(46.0, 26.0));
    let x = |i: usize| inner.left() + i as f32 / (n - 1) as f32 * inner.width();
    // Elevation backdrop.
    let (lo, hi) = samples.iter().fold((f64::MAX, f64::MIN), |(a, b), s| {
        (a.min(s.ele), b.max(s.ele))
    });
    let span = (hi - lo).max(20.0);
    let ye = |e: f64| inner.bottom() - ((e - lo) / span) as f32 * inner.height() * 0.45;
    let mut mesh = Mesh::default();
    let ecol = Color32::from_rgb(52, 66, 58);
    let step = (n / 800).max(1);
    let idx: Vec<usize> = (0..n).step_by(step).chain([n - 1]).collect();
    for w in idx.windows(2) {
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(Pos2::new(x(w[0]), ye(samples[w[0]].ele)), ecol);
        mesh.colored_vertex(Pos2::new(x(w[1]), ye(samples[w[1]].ele)), ecol);
        mesh.colored_vertex(Pos2::new(x(w[1]), inner.bottom()), ecol);
        mesh.colored_vertex(Pos2::new(x(w[0]), inner.bottom()), ecol);
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base, base + 2, base + 3);
    }
    painter.add(Shape::mesh(mesh));

    let power: Vec<f32> = samples.iter().map(|s| s.power).collect();
    let pmax = power.iter().copied().fold(ftp * 1.3, f32::max) * 1.05;
    let yp = |p: f32| inner.bottom() - (p / pmax).clamp(0.0, 1.0) * inner.height();
    let yh = |h: f32| inner.bottom() - ((h - 50.0) / 150.0).clamp(0.0, 1.0) * inner.height();
    let grid = Color32::from_white_alpha(22);
    let label = |p: Pos2, a: Align2, t: String, c: Color32| {
        painter.text(p, a, t, FontId::proportional(11.0), c);
    };
    for w in [ftp * 0.5, ftp, ftp * 1.5] {
        if w < pmax {
            painter.line_segment(
                [
                    Pos2::new(inner.left(), yp(w)),
                    Pos2::new(inner.right(), yp(w)),
                ],
                Stroke::new(1.0, grid),
            );
            label(
                Pos2::new(inner.left() - 6.0, yp(w)),
                Align2::RIGHT_CENTER,
                format!("{w:.0}"),
                Color32::from_gray(150),
            );
        }
    }
    for h in [100.0, 150.0] {
        label(
            Pos2::new(inner.right() + 6.0, yh(h)),
            Align2::LEFT_CENTER,
            format!("{h:.0}"),
            Color32::from_rgb(230, 110, 120),
        );
    }
    // Time axis every 10 or 30 minutes.
    let tick = if n > 3 * 3600 {
        1800
    } else if n > 3600 {
        900
    } else {
        600
    };
    for t in (tick..n).step_by(tick) {
        let px = x(t);
        painter.line_segment(
            [Pos2::new(px, inner.top()), Pos2::new(px, inner.bottom())],
            Stroke::new(1.0, grid),
        );
        label(
            Pos2::new(px, inner.bottom() + 4.0),
            Align2::CENTER_TOP,
            crate::workout::fmt_dur(t as f32),
            Color32::from_gray(150),
        );
    }
    let raw: Vec<Pos2> = idx.iter().map(|&i| Pos2::new(x(i), yp(power[i]))).collect();
    painter.add(Shape::line(
        raw,
        Stroke::new(1.0, Color32::from_white_alpha(45)),
    ));
    // 30 s rolling average.
    let mut avg = Vec::with_capacity(n);
    let mut sum = 0.0;
    for i in 0..n {
        sum += power[i];
        if i >= 30 {
            sum -= power[i - 30];
        }
        avg.push(sum / (i + 1).min(30) as f32);
    }
    let smooth: Vec<Pos2> = idx.iter().map(|&i| Pos2::new(x(i), yp(avg[i]))).collect();
    painter.add(Shape::line(smooth, Stroke::new(1.8, Color32::WHITE)));
    let mut seg: Vec<Pos2> = Vec::new();
    for &i in &idx {
        match samples[i].hr.filter(|h| *h > 0.0) {
            Some(h) => seg.push(Pos2::new(x(i), yh(h))),
            None => {
                if seg.len() > 1 {
                    painter.add(Shape::line(
                        std::mem::take(&mut seg),
                        Stroke::new(1.6, Color32::from_rgb(230, 70, 90)),
                    ));
                }
                seg.clear();
            }
        }
    }
    if seg.len() > 1 {
        painter.add(Shape::line(
            seg,
            Stroke::new(1.6, Color32::from_rgb(230, 70, 90)),
        ));
    }
    label(
        Pos2::new(rect.left() + 10.0, rect.top() + 6.0),
        Align2::LEFT_TOP,
        "Power (W, 30 s average)".into(),
        Color32::WHITE,
    );
    label(
        Pos2::new(rect.left() + 170.0, rect.top() + 6.0),
        Align2::LEFT_TOP,
        "Heart rate (bpm)".into(),
        Color32::from_rgb(230, 70, 90),
    );
    label(
        Pos2::new(rect.left() + 285.0, rect.top() + 6.0),
        Align2::LEFT_TOP,
        "Elevation".into(),
        Color32::from_rgb(110, 150, 120),
    );
}

/// Vertical bars with a label under each and a value above.
pub fn bars(
    painter: &Painter,
    rect: Rect,
    items: &[(String, f32, String)],
    color: Color32,
    bg: Color32,
) {
    painter.rect_filled(rect, 6.0, bg);
    let inner = rect.shrink2(eframe::egui::vec2(12.0, 22.0));
    let max = items.iter().map(|i| i.1).fold(0.0f32, f32::max);
    if items.is_empty() || max <= 0.0 {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "No rides in this period yet",
            FontId::proportional(13.0),
            Color32::from_gray(140),
        );
        return;
    }
    let slot = inner.width() / items.len() as f32;
    let w = (slot * 0.62).min(46.0);
    for (i, (label, v, top)) in items.iter().enumerate() {
        let cx = inner.left() + slot * (i as f32 + 0.5);
        let h = (v / max) * (inner.height() - 14.0);
        let r = Rect::from_min_max(
            Pos2::new(cx - w / 2.0, inner.bottom() - h),
            Pos2::new(cx + w / 2.0, inner.bottom()),
        );
        if *v > 0.0 {
            painter.rect_filled(r, 3.0, color);
            painter.text(
                Pos2::new(cx, r.top() - 3.0),
                Align2::CENTER_BOTTOM,
                top,
                FontId::proportional(10.5),
                Color32::from_gray(200),
            );
        }
        painter.text(
            Pos2::new(cx, inner.bottom() + 4.0),
            Align2::CENTER_TOP,
            label,
            FontId::proportional(10.5),
            Color32::from_gray(140),
        );
    }
}

/// A named power curve: (name, colour, (seconds, watts) points).
pub type PowerCurveSeries<'a> = (&'a str, Color32, Vec<(u32, f32)>);

/// Power curve on a logarithmic duration axis: `curves` are (name, colour, points).
pub fn power_curve(
    painter: &Painter,
    rect: Rect,
    curves: &[PowerCurveSeries],
    ftp: f32,
    bg: Color32,
) {
    painter.rect_filled(rect, 6.0, bg);
    let inner = rect.shrink2(eframe::egui::vec2(46.0, 26.0));
    let pts: Vec<f32> = curves
        .iter()
        .flat_map(|c| c.2.iter().map(|p| p.1))
        .collect();
    if pts.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Ride with a power meter or smart trainer to build your power curve",
            FontId::proportional(13.0),
            Color32::from_gray(140),
        );
        return;
    }
    let (t0, t1) = (5f32.ln(), 7200f32.ln());
    let x = |s: u32| inner.left() + ((s as f32).ln() - t0) / (t1 - t0) * inner.width();
    let pmax = pts.iter().copied().fold(ftp * 1.2, f32::max) * 1.08;
    let y = |w: f32| inner.bottom() - (w / pmax).clamp(0.0, 1.0) * inner.height();
    let grid = Color32::from_white_alpha(22);
    for (s, label) in [
        (5, "5s"),
        (60, "1m"),
        (300, "5m"),
        (1200, "20m"),
        (3600, "1h"),
        (7200, "2h"),
    ] {
        painter.line_segment(
            [
                Pos2::new(x(s), inner.top()),
                Pos2::new(x(s), inner.bottom()),
            ],
            Stroke::new(1.0, grid),
        );
        painter.text(
            Pos2::new(x(s), inner.bottom() + 4.0),
            Align2::CENTER_TOP,
            label,
            FontId::proportional(11.0),
            Color32::from_gray(150),
        );
    }
    let step = if pmax > 800.0 { 200.0 } else { 100.0 };
    let mut w = step;
    while w < pmax {
        painter.line_segment(
            [
                Pos2::new(inner.left(), y(w)),
                Pos2::new(inner.right(), y(w)),
            ],
            Stroke::new(1.0, grid),
        );
        painter.text(
            Pos2::new(inner.left() - 6.0, y(w)),
            Align2::RIGHT_CENTER,
            format!("{w:.0}"),
            FontId::proportional(11.0),
            Color32::from_gray(150),
        );
        w += step;
    }
    painter.line_segment(
        [
            Pos2::new(inner.left(), y(ftp)),
            Pos2::new(inner.right(), y(ftp)),
        ],
        Stroke::new(1.0, Color32::from_rgb(240, 200, 80).gamma_multiply(0.6)),
    );
    painter.text(
        Pos2::new(inner.right(), y(ftp) - 3.0),
        Align2::RIGHT_BOTTOM,
        format!("FTP {ftp:.0} W"),
        FontId::proportional(11.0),
        Color32::from_rgb(240, 200, 80),
    );
    let mut lx = rect.left() + 10.0;
    for (name, col, points) in curves {
        if points.len() > 1 {
            let line: Vec<Pos2> = points.iter().map(|&(s, w)| Pos2::new(x(s), y(w))).collect();
            painter.add(Shape::line(line, Stroke::new(2.2, *col)));
        }
        for &(s, w) in points {
            painter.circle_filled(Pos2::new(x(s), y(w)), 3.0, *col);
        }
        let r = painter.text(
            Pos2::new(lx, rect.top() + 6.0),
            Align2::LEFT_TOP,
            *name,
            FontId::proportional(12.0),
            *col,
        );
        lx = r.right() + 16.0;
    }
}

/// Step line of FTP over time.
pub fn ftp_history(
    painter: &Painter,
    rect: Rect,
    entries: &[crate::progress::FtpEntry],
    today: chrono::NaiveDate,
    bg: Color32,
) {
    painter.rect_filled(rect, 6.0, bg);
    if entries.is_empty() {
        return;
    }
    let inner = rect.shrink2(eframe::egui::vec2(46.0, 24.0));
    let first = entries[0].date.min(today - chrono::Duration::days(30));
    let span = (today - first).num_days().max(1) as f32;
    let x =
        |d: chrono::NaiveDate| inner.left() + (d - first).num_days() as f32 / span * inner.width();
    let lo = entries.iter().map(|e| e.ftp).fold(f32::MAX, f32::min) * 0.9;
    let hi = entries.iter().map(|e| e.ftp).fold(0.0f32, f32::max) * 1.08;
    let y = |f: f32| inner.bottom() - (f - lo) / (hi - lo).max(1.0) * inner.height();
    let col = Color32::from_rgb(240, 200, 80);
    let mut pts = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        if i > 0 {
            pts.push(Pos2::new(x(e.date), y(entries[i - 1].ftp)));
        }
        pts.push(Pos2::new(x(e.date), y(e.ftp)));
    }
    pts.push(Pos2::new(x(today), y(entries[entries.len() - 1].ftp)));
    painter.add(Shape::line(pts, Stroke::new(2.0, col)));
    for e in entries {
        let p = Pos2::new(x(e.date), y(e.ftp));
        painter.circle_filled(p, 3.5, col);
        painter.text(
            p - eframe::egui::vec2(0.0, 6.0),
            Align2::CENTER_BOTTOM,
            format!("{:.0}", e.ftp),
            FontId::proportional(11.0),
            Color32::from_gray(210),
        );
    }
    painter.text(
        Pos2::new(inner.left(), inner.bottom() + 4.0),
        Align2::LEFT_TOP,
        first.format("%d %b %Y").to_string(),
        FontId::proportional(11.0),
        Color32::from_gray(140),
    );
    painter.text(
        Pos2::new(inner.right(), inner.bottom() + 4.0),
        Align2::RIGHT_TOP,
        "today",
        FontId::proportional(11.0),
        Color32::from_gray(140),
    );
}

/// Colour for a cadence target: green within 5 rpm of it, amber further off,
/// dim when there is no cadence reading.
pub fn cadence_colour(live: Option<f32>, target: f32) -> Color32 {
    match live {
        Some(c) if (c - target).abs() <= 5.0 => theme::GOOD,
        Some(_) => theme::WARN,
        None => theme::TEXT_DIM,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grade_colours_run_from_blue_to_red() {
        let down = grade_color(-8.0);
        let flat = grade_color(0.0);
        let steep = grade_color(8.0);
        assert!(down.b() > down.r() && down.b() > down.g());
        assert!(flat.g() > flat.r() && flat.g() > flat.b());
        assert!(steep.r() > steep.g() && steep.r() > steep.b());
        // Out-of-range gradients clamp to the ends of the scale.
        assert_eq!(grade_color(-40.0), grade_color(-8.0));
        assert_eq!(grade_color(40.0), grade_color(13.0));
    }

    #[test]
    fn zones() {
        assert_eq!(hr_zone(80.0, 190.0), 0);
        assert_eq!(hr_zone(120.0, 190.0), 1); // 63%
        assert_eq!(hr_zone(160.0, 190.0), 4 - 1); // 84%
        assert_eq!(hr_zone(185.0, 190.0), 4);
        assert_eq!(power_zone(0.5), 0);
        assert_eq!(power_zone(0.88), 2);
        assert_eq!(power_zone(1.0), 3);
        assert_eq!(power_zone(1.5), 5);
        // Same thresholds as the zone colours.
        for f in [0.3, 0.6, 0.8, 0.95, 1.1, 1.3] {
            assert_eq!(
                zone_name(f),
                [
                    "Z1 Recovery",
                    "Z2 Endurance",
                    "Z3 Tempo",
                    "Z4 Threshold",
                    "Z5 VO2 max",
                    "Z6 Anaerobic"
                ][power_zone(f)]
            );
        }
    }
}
