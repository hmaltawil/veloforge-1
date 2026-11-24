//! The Progress tab: fitness, power curves, records and segment results.

use super::*;

impl App {
    /// Fill in power curves for rides saved before they were recorded,
    /// a couple per frame so the screen stays responsive.
    fn backfill_curves(&mut self, ctx: &egui::Context) {
        let mut done = 0;
        for i in 0..self.store.history.len() {
            if done >= 2 {
                ctx.request_repaint();
                break;
            }
            let h = &self.store.history[i];
            if !h.best.is_empty() || h.secs < 5.0 || self.curve_failed.contains(&h.file) {
                continue;
            }
            let file = h.file.clone();
            match std::fs::read_to_string(&file)
                .ok()
                .and_then(|x| crate::activity::parse_tcx(&x).ok())
            {
                Some(a) if !a.samples.is_empty() => {
                    self.store.history[i].best = crate::progress::best_curve(&a.samples);
                    self.dirty = true;
                }
                _ => {
                    self.curve_failed.insert(file);
                }
            }
            done += 1;
        }
    }

    pub(super) fn progress_tab(&mut self, ui: &mut egui::Ui) {
        use crate::progress;
        self.backfill_curves(&ui.ctx().clone());
        let today = Local::now().date_naive();
        let ftp = self.store.settings.ftp;
        let history = &self.store.history;
        ui.heading("Progress");
        ui.add_space(4.0);

        // Totals.
        let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
        let year_start = NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap();
        let periods = [
            (
                "This week",
                progress::totals_between(history, monday, today),
            ),
            (
                "Last 4 weeks",
                progress::totals_between(history, today - Duration::days(27), today),
            ),
            (
                "This year",
                progress::totals_between(history, year_start, today),
            ),
        ];
        let fit = progress::fitness(history, today, 120);
        let (ctl, atl, tsb) = fit.today();
        ui.horizontal_wrapped(|ui| {
            for (label, t) in periods {
                egui::Frame::new()
                    .fill(CARD)
                    .corner_radius(8)
                    .inner_margin(12)
                    .show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.set_width(250.0);
                            ui.label(RichText::new(label).small().weak());
                            ui.label(
                                RichText::new(format!(
                                    "{} · {}",
                                    fmt_hours(t.secs),
                                    if t.rides == 1 {
                                        "1 ride".into()
                                    } else {
                                        format!("{} rides", t.rides)
                                    }
                                ))
                                .size(18.0)
                                .strong(),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{} · {:.0} TSS · {:.0} kJ",
                                    self.store.settings.units.fmt_distance(t.km * 1000.0, 0),
                                    t.tss,
                                    t.kj
                                ))
                                .weak(),
                            );
                        });
                    });
            }
            egui::Frame::new()
                .fill(CARD)
                .corner_radius(8)
                .inner_margin(12)
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(250.0);
                        ui.label(RichText::new("Today").small().weak());
                        let (word, col) = match tsb {
                            t if t > 10.0 => ("Fresh", Color32::from_rgb(110, 210, 130)),
                            t if t > -10.0 => ("Balanced", Color32::from_gray(220)),
                            t if t > -25.0 => ("Training hard", Color32::from_rgb(240, 200, 80)),
                            _ => ("Very tired", Color32::from_rgb(240, 120, 90)),
                        };
                        ui.label(RichText::new(word).size(18.0).strong().color(col));
                        ui.label(
                            RichText::new(format!(
                                "Fitness {ctl:.0} · Fatigue {atl:.0} · Form {tsb:+.0}"
                            ))
                            .weak(),
                        );
                    });
                });
        });
        ui.add_space(10.0);

        // Fitness trend.
        ui.label(RichText::new("Fitness, fatigue and form (last 4 months)").strong());
        let (r, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width() - 14.0, 170.0),
            Sense::hover(),
        );
        let series = if history.is_empty() {
            vec![]
        } else {
            vec![
                ("Fitness", Color32::from_rgb(80, 160, 240), fit.ctl.clone()),
                ("Fatigue", Color32::from_rgb(230, 110, 140), fit.atl.clone()),
                ("Form", Color32::from_rgb(240, 200, 80), fit.tsb.clone()),
            ]
        };
        charts::lines(ui.painter(), r, &series, CARD, Color32::from_gray(160));
        ui.label(
            RichText::new("Fitness: your training load over the last ~6 weeks · Fatigue: the last week · Form = fitness − fatigue (positive means fresh, very negative means tired)")
                .small()
                .weak(),
        );
        ui.add_space(10.0);

        // Weekly hours.
        ui.label(RichText::new("Weekly riding (last 16 weeks)").strong());
        let weeks = progress::weekly(history, today, 16);
        let items: Vec<(String, f32, String)> = weeks
            .iter()
            .map(|(start, t)| {
                (
                    start.format("%d %b").to_string(),
                    t.secs / 3600.0,
                    if t.secs > 0.0 {
                        format!("{:.1} h", t.secs / 3600.0)
                    } else {
                        String::new()
                    },
                )
            })
            .collect();
        let (r, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width() - 14.0, 160.0),
            Sense::hover(),
        );
        charts::bars(ui.painter(), r, &items, ACCENT, CARD);
        ui.add_space(10.0);

        // Power curve.
        ui.label(RichText::new("Power curve: your best power for each duration").strong());
        let all = progress::merge_curves(history.iter().map(|h| h.best.as_slice()));
        let six_weeks = today - Duration::days(42);
        let recent = progress::merge_curves(
            history
                .iter()
                .filter(|h| h.start.date_naive() >= six_weeks)
                .map(|h| h.best.as_slice()),
        );
        let (r, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width() - 14.0, 230.0),
            Sense::hover(),
        );
        charts::power_curve(
            ui.painter(),
            r,
            &[
                ("All time", ACCENT, all.clone()),
                ("Last 6 weeks", Color32::WHITE, recent.clone()),
            ],
            ftp,
            CARD,
        );
        if !all.is_empty() {
            egui::Grid::new("curve-table")
                .num_columns(6)
                .spacing([22.0, 4.0])
                .show(ui, |ui| {
                    ui.label("");
                    let keys = [
                        (5, "5 s"),
                        (60, "1 min"),
                        (300, "5 min"),
                        (1200, "20 min"),
                        (3600, "60 min"),
                    ];
                    for (_, l) in keys {
                        ui.label(RichText::new(l).weak());
                    }
                    ui.end_row();
                    for (name, curve) in [("All time", &all), ("Last 6 weeks", &recent)] {
                        ui.label(name);
                        for (s, _) in keys {
                            let v = curve
                                .iter()
                                .find(|p| p.0 == s)
                                .map(|p| format!("{:.0} W", p.1))
                                .unwrap_or("—".into());
                            ui.label(RichText::new(v).strong());
                        }
                        ui.end_row();
                    }
                });
        }
        ui.add_space(10.0);

        // FTP history.
        ui.label(RichText::new("FTP history").strong());
        let entries = self.store.ftp_history.clone();
        if entries.len() < 2 {
            ui.label(RichText::new("Your FTP over time appears here once it changes, for example after a ramp test.").weak());
        }
        let (r, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width() - 14.0, 140.0),
            Sense::hover(),
        );
        charts::ftp_history(ui.painter(), r, &entries, today, CARD);
        ui.add_space(10.0);

        // Monthly totals.
        ui.label(RichText::new("Monthly totals").strong());
        egui::Grid::new("months")
            .striped(true)
            .num_columns(6)
            .spacing([24.0, 5.0])
            .show(ui, |ui| {
                for h in ["Month", "Rides", "Time", "Distance", "TSS", "Work"] {
                    ui.label(RichText::new(h).weak());
                }
                ui.end_row();
                let first = self
                    .store
                    .history
                    .iter()
                    .map(|h| h.start.date_naive())
                    .min()
                    .unwrap_or(today);
                for (m, t) in progress::monthly(&self.store.history, today, 12)
                    .into_iter()
                    .filter(|(m, _)| *m >= plan::first_of_month(first))
                {
                    ui.label(m.format("%B %Y").to_string());
                    ui.label(t.rides.to_string());
                    ui.label(fmt_hours(t.secs));
                    ui.label(self.store.settings.units.fmt_distance(t.km * 1000.0, 0));
                    ui.label(format!("{:.0}", t.tss));
                    ui.label(format!("{:.0} kJ", t.kj));
                    ui.end_row();
                }
            });
        self.personal_records(ui);
    }

    /// The segments timed on ride `s`, each with how it ranks.
    pub(super) fn segment_results(&self, ui: &mut egui::Ui, s: &ride::Summary) {
        use crate::segment::fmt_time;
        let mine: Vec<&crate::segment::Effort> = self
            .store
            .efforts
            .iter()
            .filter(|e| e.date == s.start && Some(&e.route_id) == s.route_id.as_ref())
            .collect();
        if mine.is_empty() {
            return;
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Segments").strong());
        egui::Grid::new(("segments", s.file.as_str()))
            .num_columns(4)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                for e in mine {
                    let mut times: Vec<f32> = self
                        .store
                        .efforts
                        .iter()
                        .filter(|o| o.route_id == e.route_id && o.key == e.key)
                        .map(|o| o.secs)
                        .collect();
                    times.sort_by(f32::total_cmp);
                    let rank = times.iter().position(|t| *t >= e.secs).unwrap_or(0) + 1;
                    ui.label(&e.name);
                    ui.label(RichText::new(fmt_time(e.secs)).strong());
                    ui.label(RichText::new(format!("{:.0} W", e.avg_power)).weak());
                    let (text, colour) = match (rank, times.len()) {
                        (_, 1) => ("First time".to_string(), theme::TEXT_DIM),
                        (1, _) => ("Personal record".to_string(), theme::GOOD),
                        (2, _) => ("2nd best".to_string(), theme::WARN),
                        (3, _) => ("3rd best".to_string(), theme::WARN),
                        _ => (
                            format!("+{} on your best", fmt_time(e.secs - times[0])),
                            theme::TEXT_DIM,
                        ),
                    };
                    ui.colored_label(colour, text);
                    ui.end_row();
                }
            });
    }

    /// Your best time on every segment you have ridden, route by route.
    fn personal_records(&self, ui: &mut egui::Ui) {
        use crate::segment::fmt_time;
        if self.store.efforts.is_empty() {
            return;
        }
        ui.add_space(16.0);
        ui.heading("Personal records");
        ui.label(
            RichText::new("Your best time on each climb and lap, from every ride on the route.")
                .weak(),
        );
        ui.add_space(6.0);
        for route in &self.routes {
            let efforts: Vec<&crate::segment::Effort> = self
                .store
                .efforts
                .iter()
                .filter(|e| e.route_id == route.id)
                .collect();
            if efforts.is_empty() {
                continue;
            }
            ui.label(RichText::new(self.store.settings.units.route_name(&route.name)).strong());
            egui::Grid::new(("records", route.id.as_str()))
                .striped(true)
                .num_columns(6)
                .spacing([24.0, 5.0])
                .show(ui, |ui| {
                    for h in ["Segment", "", "Best", "Power", "When", "Times ridden"] {
                        ui.label(RichText::new(h).weak());
                    }
                    ui.end_row();
                    for seg in crate::segment::for_route(route) {
                        let Some(best) =
                            crate::segment::best(&self.store.efforts, &route.id, &seg.key)
                        else {
                            continue;
                        };
                        let count = efforts.iter().filter(|e| e.key == seg.key).count();
                        ui.label(&seg.name);
                        ui.label(RichText::new(seg.describe(self.store.settings.units)).weak());
                        ui.label(RichText::new(fmt_time(best.secs)).strong());
                        ui.label(format!("{:.0} W", best.avg_power));
                        ui.label(best.date.format("%d %b %Y").to_string());
                        ui.label(count.to_string());
                        ui.end_row();
                    }
                });
            ui.add_space(8.0);
        }
    }
}
