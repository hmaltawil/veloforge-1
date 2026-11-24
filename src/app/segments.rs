//! Segments along the route, and the riders who ride with you: ghosts of
//! past rides and the pacer.

use super::*;

impl App {
    /// The selected route's segments.
    fn route_segments(&mut self) -> &[crate::segment::Segment] {
        let route = &self.routes[self.route_idx];
        if self.segments.0 != route.id {
            self.segments = (route.id.clone(), crate::segment::for_route(route));
        }
        &self.segments.1
    }

    /// Your best on segment `key` of the selected route, before this ride
    /// and so far on it.
    fn segment_best(&self, key: &str) -> Option<f32> {
        let route = &self.routes[self.route_idx].id;
        self.store
            .efforts
            .iter()
            .chain(&self.ride_efforts)
            .filter(|e| &e.route_id == route && e.key == key)
            .map(|e| e.secs)
            .reduce(f32::min)
    }

    /// Time the segments ridden since the last frame, and say how each
    /// finished one went.
    pub(super) fn track_segments(&mut self, power: f32, hr: Option<f32>) {
        if self.ride.state != ride::State::Running {
            return;
        }
        // Climbs come round each lap; point to point, only once.
        let route = &self.routes[self.route_idx];
        let length = match route.shape() {
            route::Shape::Loop => route.length,
            route::Shape::PointToPoint => 1e12,
        };
        let (pos, t) = (self.ride.route_pos(), self.ride.elapsed);
        let segs = self.route_segments().to_vec();
        let done = self.seg_tracker.update(&segs, length, pos, t, power, hr);
        for (i, secs, avg_power, avg_hr) in done {
            let seg = &segs[i];
            let best = self.segment_best(&seg.key);
            use crate::segment::fmt_time;
            let (headline, colour, detail) = match best {
                Some(b) if secs < b => (
                    format!("New personal record! {} · {}", seg.name, fmt_time(secs)),
                    theme::GOOD,
                    format!("{} faster than your best", fmt_time(b - secs)),
                ),
                Some(b) => (
                    format!("{} · {}", seg.name, fmt_time(secs)),
                    theme::TEXT,
                    format!(
                        "{} slower than your best ({})",
                        fmt_time(secs - b),
                        fmt_time(b)
                    ),
                ),
                None => (
                    format!("{} · {}", seg.name, fmt_time(secs)),
                    theme::TEXT,
                    "Your first time: that's the one to beat".to_string(),
                ),
            };
            if best.is_some_and(|b| secs < b) {
                self.cue(Cue::Done);
            }
            self.log_line(format!("{headline} ({detail}, {avg_power:.0} W)"));
            self.seg_flash = Some((
                vec![
                    (headline, 17.0, colour, true),
                    (
                        format!("{detail}  ·  {avg_power:.0} W"),
                        12.5,
                        theme::TEXT_DIM,
                        false,
                    ),
                ],
                Instant::now(),
            ));
            let started = self.ride.started.unwrap_or_else(Local::now);
            self.ride_efforts.push(crate::segment::Effort {
                route_id: self.routes[self.route_idx].id.clone(),
                key: seg.key.clone(),
                name: seg.name.clone(),
                date: started,
                secs,
                avg_power,
                avg_hr,
            });
        }
    }

    /// Banner lines about segments: the result just in, the one being
    /// ridden, or the next one coming up.
    pub(crate) fn segment_lines(&self) -> Vec<BannerLine> {
        use crate::segment::{fmt_time, Kind};
        if let Some((lines, at)) = &self.seg_flash {
            if at.elapsed().as_secs_f32() < 8.0 {
                return lines.clone();
            }
        }
        if self.segments.0 != self.routes[self.route_idx].id
            || self.ride.state == ride::State::Ready
        {
            return Vec::new();
        }
        let segs = &self.segments.1;
        let best = |key: &str| {
            self.segment_best(key)
                .map(|b| format!("best {}", fmt_time(b)))
                .unwrap_or_else(|| "no time yet".into())
        };
        if let Some((i, done_m, secs)) = self.seg_tracker.current(segs) {
            let s = &segs[i];
            let left = (s.length - done_m).max(0.0);
            return vec![
                (
                    format!(
                        "{}  ·  {} to go",
                        s.name,
                        self.store.settings.units.fmt_short(left)
                    ),
                    15.0,
                    theme::ACCENT,
                    true,
                ),
                (
                    format!("{}  ·  {}", fmt_time(secs as f32), best(&s.key)),
                    12.5,
                    theme::TEXT_DIM,
                    false,
                ),
            ];
        }
        // The next climb, within half a kilometre.
        let route = &self.routes[self.route_idx];
        let pos = route.along(self.ride.route_pos());
        let looped = route.is_loop();
        segs.iter()
            .filter(|s| s.kind == Kind::Climb)
            .map(|s| {
                let ahead = s.start - pos;
                (
                    s,
                    if looped {
                        ahead.rem_euclid(route.length)
                    } else {
                        ahead
                    },
                )
            })
            .filter(|(_, ahead)| (0.0..=500.0).contains(ahead))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(s, ahead)| {
                vec![
                    (
                        format!(
                            "{} in {}",
                            s.name,
                            self.store.settings.units.fmt_short(ahead)
                        ),
                        13.0,
                        theme::TEXT,
                        true,
                    ),
                    (
                        format!(
                            "{}  ·  {}",
                            s.describe(self.store.settings.units),
                            best(&s.key)
                        ),
                        12.5,
                        theme::TEXT_DIM,
                        false,
                    ),
                ]
            })
            .unwrap_or_default()
    }

    /// Previous rides on the selected route a ghost can replay, fastest first.
    pub(super) fn ghost_candidates(&self) -> Vec<&ride::Summary> {
        let route = &self.routes[self.route_idx].id;
        let mut v: Vec<&ride::Summary> = self
            .store
            .history
            .iter()
            .filter(|h| h.route_id.as_ref() == Some(route) && h.distance_m >= 1000.0)
            .collect();
        v.sort_by(|a, b| {
            let speed = |h: &ride::Summary| h.distance_m / h.secs.max(1.0) as f64;
            speed(b).partial_cmp(&speed(a)).unwrap()
        });
        v.truncate(8);
        v
    }

    pub(super) fn ghost_label(h: &ride::Summary) -> String {
        h.start.format("%a %d %b, %H:%M").to_string()
    }

    pub(super) fn load_ghost(&mut self, h: &ride::Summary) {
        let label = Self::ghost_label(h);
        let ghost = std::fs::read_to_string(&h.file)
            .ok()
            .and_then(|x| crate::activity::parse_tcx(&x).ok())
            .and_then(|a| ride::Ghost::from_samples(label, &a.samples));
        match ghost {
            Some(g) => self.ride.companion = Some(ride::Companion::Ghost(g)),
            None => self.notify("Could not read that ride's file for a ghost"),
        }
    }

    /// The pacer's jersey: a colour that stands out from yours.
    pub(super) fn pacer_jersey(&self) -> Color32 {
        let [r, g, b] = self.store.settings.jersey;
        if b as u16 > (r as u16 + g as u16) / 2 + 20 {
            Color32::from_rgb(242, 196, 40)
        } else {
            Color32::from_rgb(40, 120, 220)
        }
    }

    /// "Pacer · 35 m ahead" style description of the companion's gap.
    pub(super) fn companion_status(&self) -> Option<(String, String, String, Color32)> {
        let gap = self.ride.companion_gap()?;
        let (title, value, unit) = match self.ride.companion.as_ref()? {
            ride::Companion::Pacer(p) => {
                let (v, u) = gap_text(gap, self.store.settings.units);
                (format!("PACER · {:.0} W", p.watts), v, u)
            }
            ride::Companion::Ghost(g) => match self.ride.ghost_time_gap() {
                Some(t) if self.ride.elapsed > 1.0 => (
                    "GHOST".to_string(),
                    fmt_dur(t.abs().round() as f32),
                    if t > 0.5 { "behind" } else { "ahead" }.to_string(),
                ),
                _ if self.ride.elapsed > g.duration() => (
                    "GHOST · finished".to_string(),
                    gap_text(gap, self.store.settings.units).0,
                    gap_text(gap, self.store.settings.units).1,
                ),
                _ => {
                    let (v, u) = gap_text(gap, self.store.settings.units);
                    ("GHOST".to_string(), v, u)
                }
            },
        };
        let col = if gap > 1.0 {
            Color32::from_rgb(240, 150, 70)
        } else {
            Color32::from_rgb(110, 210, 130)
        };
        let title = if self.ride.draft >= 0.02 {
            format!("{title} · DRAFT −{:.0}%", self.ride.draft * 100.0)
        } else {
            title
        };
        Some((title, value, unit, col))
    }
}
