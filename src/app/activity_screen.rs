//! Activity: your totals down the side, the chosen ride in full (figures,
//! chart, zones, best efforts) with the earlier rides under it, the trends,
//! and your records.

use super::glass::{self, Glyph, Lead};
use super::*;

impl App {
    pub(super) fn activity_tab(&mut self, ui: &mut egui::Ui) {
        if matches!(
            self.activity_view,
            ActivityView::Trends | ActivityView::Table
        ) {
            ui.horizontal(|ui| self.activity_switch(ui));
            ui.add_space(10.0);
            if self.activity_view == ActivityView::Table {
                self.history_tab(ui);
            } else {
                egui::ScrollArea::vertical()
                    .id_salt("progress-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.progress_tab(ui));
            }
            return;
        }
        let pane = ui.max_rect().expand2(Vec2::new(28.0, 26.0));
        glass::area(ui, pane, "activity", |ui| {
            let (side, main) = glass::split(ui);
            if let Some(side) = side {
                glass::area(ui, side, "activity-side", |ui| self.activity_sidebar(ui));
            }
            glass::area(ui, main, "activity-main", |ui| {
                ui.horizontal(|ui| self.activity_switch(ui));
                ui.add_space(12.0);
                match self.activity_view {
                    ActivityView::Records => self.records(ui),
                    _ => self.ride_detail(ui),
                }
            });
        });
    }

    fn activity_switch(&mut self, ui: &mut egui::Ui) {
        for (v, label) in [
            (ActivityView::Rides, "Rides"),
            (ActivityView::Trends, "Trends"),
            (ActivityView::Records, "Records"),
        ] {
            if glass::pill(ui, label, self.activity_view == v).clicked() {
                self.activity_view = v;
            }
        }
    }

    fn activity_sidebar(&mut self, ui: &mut egui::Ui) {
        use crate::progress::totals_between;
        ui.spacing_mut().item_spacing.y = 4.0;
        glass::side_heading(ui, "Totals");
        let today = Local::now().date_naive();
        let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
        let year = NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap_or(today);
        let units = self.store.settings.units;
        for (label, from) in [
            ("This week", monday),
            ("Last 4 weeks", today - Duration::days(27)),
            ("This year", year),
        ] {
            let t = totals_between(&self.store.history, from, today);
            let meta = format!(
                "{} rides · {} · {} · {:.0} TSS",
                t.rides,
                fmt_hours(t.secs),
                units.fmt_distance(t.km * 1000.0, 0),
                t.tss
            );
            glass::side_row(ui, false, Lead::Icon(Glyph::Gauge), label, &meta);
        }
        let (fit, fat, form) = crate::progress::fitness(&self.store.history, today, 1).today();
        glass::side_row(
            ui,
            false,
            Lead::Icon(Glyph::Heart),
            &format!("Form {form:+.0}"),
            &format!("Fitness {fit:.0} · fatigue {fat:.0}"),
        );
        let left = ui.available_height() - 44.0;
        if left > 0.0 {
            ui.add_space(left);
        }
        if glass::side_row(
            ui,
            false,
            Lead::Icon(Glyph::Grid),
            "All rides as a table",
            "",
        )
        .clicked()
        {
            self.activity_view = ActivityView::Table;
        }
    }

    /// The chosen ride in full, then the earlier rides.
    fn ride_detail(&mut self, ui: &mut egui::Ui) {
        if self.store.history.is_empty() {
            glass::title(ui, "No rides yet");
            ui.label(
                RichText::new("Your rides show here once you have saved one.").color(glass::QUIET),
            );
            return;
        }
        let mut order: Vec<usize> = (0..self.store.history.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(self.store.history[i].start));
        let chosen = self
            .activity_ride
            .as_ref()
            .and_then(|f| {
                order
                    .iter()
                    .copied()
                    .find(|&i| &self.store.history[i].file == f)
            })
            .unwrap_or(order[0]);
        let s = self.store.history[chosen].clone();
        // The ride's samples, read once when it is chosen.
        if self.activity_cache.as_ref().map(|d| &d.summary.file) != Some(&s.file) {
            let activity = std::fs::read_to_string(&s.file)
                .map_err(|e| format!("Could not open {}: {e}", s.file))
                .and_then(|x| crate::activity::parse_tcx(&x).map_err(|e| e.to_string()));
            self.activity_cache = Some(RideDetails {
                summary: s.clone(),
                activity,
            });
        }
        let ftp = self.store.settings.ftp;
        let units = self.store.settings.units;
        let mut pick = None;
        let mut delete = false;
        egui::ScrollArea::vertical()
            .id_salt("ride-detail")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        glass::title(ui, &s.name);
                        ui.label(
                            RichText::new(s.start.format("%A %-d %B %Y, %H:%M").to_string())
                                .color(glass::QUIET),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if glass::pill_alert(ui, "Delete").clicked() {
                            delete = true;
                        }
                        if glass::pill(ui, "Show file", true)
                            .on_hover_text(&s.file)
                            .clicked()
                        {
                            let dir = Path::new(&s.file)
                                .parent()
                                .map(|p| p.to_path_buf())
                                .unwrap_or_default();
                            crate::platform::open(dir);
                        }
                    });
                });
                ui.add_space(14.0);
                glass::card(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(34.0, 10.0);
                        let split = |t: String| match t.rsplit_once(' ') {
                            Some((a, b)) => (a.to_string(), b.to_string()),
                            None => (t, String::new()),
                        };
                        glass::figure(ui, "Time", &fmt_dur(s.secs), "", 24.0);
                        let (d, du) = split(units.fmt_distance(s.distance_m, 2));
                        glass::figure(ui, "Distance", &d, &du, 24.0);
                        glass::figure(ui, "Avg power", &format!("{:.0}", s.avg_power), "W", 24.0);
                        glass::figure(ui, "Normalized", &format!("{:.0}", s.np), "W", 24.0);
                        glass::figure(ui, "Training load", &format!("{:.0}", s.tss), "TSS", 24.0);
                        if let Some(h) = s.avg_hr {
                            glass::figure(ui, "Avg heart rate", &format!("{h:.0}"), "bpm", 24.0);
                        }
                        glass::figure(ui, "Energy", &format!("{:.0}", s.kj), "kJ", 24.0);
                        let (c, cu) = split(units.fmt_height(s.ascent_m));
                        glass::figure(ui, "Climbing", &c, &cu, 24.0);
                    });
                    ui.add_space(12.0);
                    match self.activity_cache.as_ref().map(|d| &d.activity) {
                        Some(Ok(a)) => {
                            let (r, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), 200.0),
                                Sense::hover(),
                            );
                            let targets = self.ride_targets(&s);
                            charts::ride_chart(
                                ui.painter(),
                                r,
                                &a.samples,
                                ftp,
                                targets.as_deref(),
                                Color32::from_black_alpha(40),
                            );
                        }
                        Some(Err(e)) => {
                            ui.colored_label(glass::ALERT, e);
                        }
                        None => {}
                    }
                });
                ui.add_space(14.0);
                ui.columns(2, |cols| {
                    glass::card(&mut cols[0], |ui| {
                        glass::heading(ui, "Zones");
                        zone_breakdown(ui, &s);
                    });
                    glass::card(&mut cols[1], |ui| {
                        glass::heading(ui, "Best efforts");
                        ui.add_space(4.0);
                        let best = match self.activity_cache.as_ref().map(|d| &d.activity) {
                            Some(Ok(a)) => crate::activity::best_efforts(&a.samples),
                            _ => Vec::new(),
                        };
                        if best.is_empty() {
                            ui.label(
                                RichText::new("Ride longer than 5 seconds to see them.")
                                    .color(glass::QUIET),
                            );
                        }
                        egui::Grid::new("best-efforts")
                            .num_columns(3)
                            .spacing([22.0, 6.0])
                            .show(ui, |ui| {
                                for (label, _, w) in best {
                                    ui.label(RichText::new(label).color(glass::SOFT));
                                    ui.label(
                                        RichText::new(format!("{w:.0} W"))
                                            .font(theme::medium(14.0))
                                            .color(Color32::WHITE),
                                    );
                                    ui.label(
                                        RichText::new(format!(
                                            "{:.0}% FTP",
                                            w / ftp.max(1.0) * 100.0
                                        ))
                                        .color(glass::QUIET),
                                    );
                                    ui.end_row();
                                }
                            });
                    });
                });
                ui.add_space(18.0);
                glass::heading(ui, "Earlier");
                ui.add_space(4.0);
                for &i in order.iter().take(60) {
                    let h = &self.store.history[i];
                    let colour = h
                        .workout_id
                        .as_deref()
                        .and_then(|id| self.workout_category(id))
                        .map(charts::category_color)
                        .unwrap_or(Color32::from_gray(150));
                    let meta = format!(
                        "{} · {} · {} · {:.0} W · {:.0} TSS",
                        h.start.format("%a %-d %b"),
                        fmt_dur(h.secs),
                        units.fmt_distance(h.distance_m, 1),
                        h.avg_power,
                        h.tss
                    );
                    if glass::side_row(ui, i == chosen, Lead::Stripe(colour), &h.name, &meta)
                        .clicked()
                    {
                        pick = Some(h.file.clone());
                    }
                }
            });
        if let Some(f) = pick {
            self.activity_ride = Some(f);
        }
        if delete {
            self.modal = Some(Modal::ConfirmDelete(chosen));
        }
    }

    /// Your bests: power for each length of effort, and the biggest rides.
    fn records(&mut self, ui: &mut egui::Ui) {
        glass::title(ui, "Records");
        ui.label(RichText::new("Your best from every saved ride.").color(glass::QUIET));
        ui.add_space(14.0);
        let history = &self.store.history;
        let units = self.store.settings.units;
        let ftp = self.store.settings.ftp;
        egui::ScrollArea::vertical()
            .id_salt("records")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                glass::card(ui, |ui| {
                    glass::heading(ui, "Power");
                    ui.add_space(6.0);
                    egui::Grid::new("power-records")
                        .num_columns(4)
                        .spacing([34.0, 10.0])
                        .show(ui, |ui| {
                            for (secs, label) in [
                                (5, "5 seconds"),
                                (15, "15 seconds"),
                                (60, "1 minute"),
                                (300, "5 minutes"),
                                (1200, "20 minutes"),
                                (3600, "1 hour"),
                            ] {
                                let best = history
                                    .iter()
                                    .filter_map(|h| {
                                        h.best
                                            .iter()
                                            .find(|(d, _)| *d == secs)
                                            .map(|(_, w)| (*w, h))
                                    })
                                    .max_by(|a, b| a.0.total_cmp(&b.0));
                                ui.label(RichText::new(label).color(glass::SOFT));
                                match best {
                                    Some((w, h)) => {
                                        ui.label(
                                            RichText::new(format!("{w:.0} W"))
                                                .font(theme::medium(16.0))
                                                .color(Color32::WHITE),
                                        );
                                        ui.label(
                                            RichText::new(format!(
                                                "{:.0}% FTP",
                                                w / ftp.max(1.0) * 100.0
                                            ))
                                            .color(glass::QUIET),
                                        );
                                        ui.label(
                                            RichText::new(format!(
                                                "{} · {}",
                                                h.name,
                                                h.start.format("%-d %b %Y")
                                            ))
                                            .color(glass::QUIET),
                                        );
                                    }
                                    None => {
                                        ui.label(RichText::new("—").color(glass::QUIET));
                                        ui.label("");
                                        ui.label("");
                                    }
                                }
                                ui.end_row();
                            }
                        });
                });
                ui.add_space(14.0);
                glass::card(ui, |ui| {
                    glass::heading(ui, "Rides");
                    ui.add_space(6.0);
                    let row = |ui: &mut egui::Ui,
                               label: &str,
                               h: Option<&Summary>,
                               value: &dyn Fn(&Summary) -> String| {
                        ui.label(RichText::new(label).color(glass::SOFT));
                        match h {
                            Some(h) => {
                                ui.label(
                                    RichText::new(value(h))
                                        .font(theme::medium(16.0))
                                        .color(Color32::WHITE),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "{} · {}",
                                        h.name,
                                        h.start.format("%-d %b %Y")
                                    ))
                                    .color(glass::QUIET),
                                );
                            }
                            None => {
                                ui.label("—");
                                ui.label("");
                            }
                        }
                        ui.end_row();
                    };
                    egui::Grid::new("ride-records")
                        .num_columns(3)
                        .spacing([34.0, 10.0])
                        .show(ui, |ui| {
                            row(
                                ui,
                                "Longest ride",
                                history
                                    .iter()
                                    .max_by(|a, b| a.distance_m.total_cmp(&b.distance_m)),
                                &|h| units.fmt_distance(h.distance_m, 1),
                            );
                            row(
                                ui,
                                "Longest time",
                                history.iter().max_by(|a, b| a.secs.total_cmp(&b.secs)),
                                &|h| fmt_dur(h.secs),
                            );
                            row(
                                ui,
                                "Most climbing",
                                history
                                    .iter()
                                    .max_by(|a, b| a.ascent_m.total_cmp(&b.ascent_m)),
                                &|h| units.fmt_height(h.ascent_m),
                            );
                            row(
                                ui,
                                "Hardest ride",
                                history.iter().max_by(|a, b| a.tss.total_cmp(&b.tss)),
                                &|h| format!("{:.0} TSS", h.tss),
                            );
                            row(
                                ui,
                                "Most energy",
                                history.iter().max_by(|a, b| a.kj.total_cmp(&b.kj)),
                                &|h| format!("{:.0} kJ", h.kj),
                            );
                        });
                });
            });
    }
}
