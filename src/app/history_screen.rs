//! The History tab and the window showing one saved ride.

use super::*;

impl App {
    pub(super) fn history_tab(&mut self, ui: &mut egui::Ui) {
        let mut open: Option<usize> = None;
        let mut upload: Option<(String, String)> = None;
        ui.horizontal(|ui| {
            ui.heading("History");
            ui.label(
                RichText::new(format!(
                    "Activities are saved as .tcx in {}",
                    self.paths.activities.display()
                ))
                .weak(),
            );
            if ui.button("Open folder").clicked() {
                let _ = std::fs::create_dir_all(&self.paths.activities);
                crate::platform::open(&self.paths.activities);
            }
        });
        ui.label(
            RichText::new(
                "Upload the .tcx files to Strava, intervals.icu, TrainingPeaks or GoldenCheetah.",
            )
            .weak(),
        );
        ui.add_space(6.0);

        if !self.store.history.is_empty()
            && ui
                .link("Trends, power curve and totals are in Progress")
                .clicked()
        {
            self.tab = Tab::Progress;
        }
        ui.add_space(8.0);
        let mut delete = None;
        if self.store.history.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No rides yet").size(18.0));
                ui.label(RichText::new("Your rides appear here after you press Finish. Click one to see its charts and zones.").weak());
            });
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("history")
                    .striped(true)
                    .num_columns(12)
                    .spacing([14.0, 6.0])
                    .show(ui, |ui| {
                        for h in [
                            "Date",
                            "Ride",
                            "Time",
                            "Distance",
                            "Avg W",
                            "NP",
                            "TSS",
                            "HR",
                            "Cadence",
                            "Zones (power / HR)",
                            "intervals.icu",
                            "",
                        ] {
                            ui.label(RichText::new(h).strong());
                        }
                        ui.end_row();
                        for (i, h) in self.store.history.iter().enumerate().rev() {
                            ui.label(h.start.format("%a %d %b %Y %H:%M").to_string());
                            let mut name = h.name.clone();
                            if let Some(f) = h.ftp_estimate {
                                name += &format!("  (FTP est. {f:.0} W)");
                            }
                            if let Some(t) = h.lthr_estimate {
                                name += &format!("  (threshold HR {t:.0} bpm)");
                            }
                            if ui
                                .link(name)
                                .on_hover_text("Show charts and details")
                                .clicked()
                            {
                                open = Some(i);
                            }
                            ui.label(fmt_dur(h.secs));
                            ui.label(self.store.settings.units.fmt_distance(h.distance_m, 1));
                            ui.label(format!("{:.0}", h.avg_power));
                            ui.label(format!("{:.0}", h.np));
                            ui.label(format!("{:.0}", h.tss));
                            ui.label(h.avg_hr.map(|v| format!("{v:.0}")).unwrap_or("—".into()));
                            ui.label(
                                h.avg_cadence
                                    .map(|v| format!("{v:.0}"))
                                    .unwrap_or("—".into()),
                            );
                            let (r, _) =
                                ui.allocate_exact_size(Vec2::new(120.0, 16.0), Sense::hover());
                            if !h.power_zones.is_empty() {
                                charts::zone_bar(
                                    ui.painter(),
                                    Rect::from_min_size(r.min, Vec2::new(r.width(), 7.0)),
                                    &h.power_zones,
                                    power_zone_color,
                                );
                                charts::zone_bar(
                                    ui.painter(),
                                    Rect::from_min_size(
                                        r.min + Vec2::new(0.0, 9.0),
                                        Vec2::new(r.width(), 7.0),
                                    ),
                                    &h.hr_zones,
                                    charts::hr_zone_color,
                                );
                            }
                            match &h.icu_id {
                                Some(id) => {
                                    ui.hyperlink_to(
                                        "Uploaded",
                                        format!("https://intervals.icu/activities/{id}"),
                                    );
                                }
                                None if self.icu_creds.is_set() => {
                                    if ui.small_button("Upload").clicked() {
                                        upload = Some((h.file.clone(), h.name.clone()));
                                    }
                                }
                                None => {
                                    ui.label(RichText::new("—").weak());
                                }
                            }
                            ui.horizontal(|ui| {
                                if ui
                                    .small_button("Show file")
                                    .on_hover_text(&h.file)
                                    .clicked()
                                {
                                    let dir = Path::new(&h.file)
                                        .parent()
                                        .map(|p| p.to_path_buf())
                                        .unwrap_or_default();
                                    crate::platform::open(dir);
                                }
                                if ui
                                    .small_button("Delete")
                                    .on_hover_text(format!(
                                        "Delete this ride (its file goes to the {TRASH})"
                                    ))
                                    .clicked()
                                {
                                    delete = Some(i);
                                }
                            });
                            ui.end_row();
                        }
                    });
            });
        if let Some(i) = open {
            let summary = self.store.history[i].clone();
            let activity = std::fs::read_to_string(&summary.file)
                .map_err(|e| format!("Could not open {}: {e}", summary.file))
                .and_then(|x| crate::activity::parse_tcx(&x).map_err(|e| e.to_string()));
            self.details = Some(RideDetails { summary, activity });
        }
        if let Some(i) = delete {
            self.modal = Some(Modal::ConfirmDelete(i));
        }
        if let Some((file, name)) = upload {
            self.icu_upload(file, name);
        }
    }

    pub(super) fn details_window(&mut self, ctx: &egui::Context) {
        let Some(d) = &self.details else { return };
        let mut open = true;
        let ftp = self.store.settings.ftp;
        let s = &d.summary;
        // On Strava already, on its way, or not yet (None: not connected).
        let strava = self.strava_creds.is_connected().then(|| {
            let done = self
                .store
                .history
                .iter()
                .any(|h| h.file == s.file && h.strava_id.is_some());
            (done, self.strava_pending.contains(&s.file))
        });
        let mut upload = None;
        egui::Window::new(RichText::new(&s.name).strong())
            .id(egui::Id::unique("ride-details"))
            .open(&mut open)
            .collapsible(false)
            .default_size([960.0, 640.0])
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                // Never taller than the window it sits in: scroll instead.
                let room = (ui.ctx().content_rect().height() - 110.0).max(200.0);
                egui::ScrollArea::vertical()
                    .max_height(room)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(s.start.format("%A %d %B %Y, %H:%M").to_string()).weak(),
                        );
                        ui.add_space(4.0);
                        // From the stored training load, so it matches the TSS:
                        // both use the FTP at the time of the ride.
                        let ifac = if s.secs > 0.0 {
                            (s.tss * 36.0 / s.secs).max(0.0).sqrt()
                        } else {
                            0.0
                        };
                        ui.horizontal_wrapped(|ui| {
                            let stat = |ui: &mut egui::Ui, k: &str, v: String| {
                                egui::Frame::new()
                                    .fill(CARD)
                                    .corner_radius(6)
                                    .inner_margin(8)
                                    .show(ui, |ui| {
                                        ui.vertical(|ui| {
                                            ui.label(RichText::new(k).small().weak());
                                            ui.label(RichText::new(v).size(18.0).strong());
                                        });
                                    });
                            };
                            stat(ui, "Time", fmt_dur(s.secs));
                            stat(
                                ui,
                                "Distance",
                                self.store.settings.units.fmt_distance(s.distance_m, 2),
                            );
                            stat(
                                ui,
                                "Climbing",
                                self.store.settings.units.fmt_height(s.ascent_m),
                            );
                            stat(ui, "Avg power", format!("{:.0} W", s.avg_power));
                            stat(ui, "Normalized", format!("{:.0} W", s.np));
                            stat(ui, "Intensity", format!("{ifac:.2}"));
                            stat(ui, "TSS", format!("{:.0}", s.tss));
                            stat(ui, "Work", format!("{:.0} kJ", s.kj));
                            if let Some(h) = s.avg_hr {
                                stat(ui, "Avg HR", format!("{h:.0} bpm"));
                            }
                            if let Some(h) = s.max_hr {
                                stat(ui, "Max HR", format!("{h:.0} bpm"));
                            }
                            if let Some(c) = s.avg_cadence {
                                stat(ui, "Cadence", format!("{c:.0} rpm"));
                            }
                        });
                        ui.add_space(6.0);
                        match &d.activity {
                            Ok(a) => {
                                let (r, _) = ui.allocate_exact_size(
                                    Vec2::new(ui.available_width(), 230.0),
                                    Sense::hover(),
                                );
                                let targets = self.ride_targets(s);
                                charts::ride_chart(
                                    ui.painter(),
                                    r,
                                    &a.samples,
                                    ftp,
                                    targets.as_deref(),
                                    CARD,
                                );
                                ui.add_space(6.0);
                                ui.columns(2, |cols| {
                                    zone_breakdown(&mut cols[0], s);
                                    let ui = &mut cols[1];
                                    ui.add_space(6.0);
                                    ui.label(RichText::new("Best efforts").strong());
                                    let best = crate::activity::best_efforts(&a.samples);
                                    if best.is_empty() {
                                        ui.label(
                                            RichText::new(
                                                "Ride longer than 5 seconds to see best efforts.",
                                            )
                                            .weak(),
                                        );
                                    }
                                    egui::Grid::new("best")
                                        .num_columns(3)
                                        .spacing([18.0, 4.0])
                                        .show(ui, |ui| {
                                            for (label, _, w) in best {
                                                ui.label(label);
                                                ui.label(
                                                    RichText::new(format!("{w:.0} W")).strong(),
                                                );
                                                ui.label(
                                                    RichText::new(format!(
                                                        "{:.0}% FTP",
                                                        w / ftp.max(1.0) * 100.0
                                                    ))
                                                    .weak(),
                                                );
                                                ui.end_row();
                                            }
                                        });
                                });
                            }
                            Err(e) => {
                                ui.colored_label(Color32::from_rgb(240, 150, 80), e);
                            }
                        }
                        self.segment_results(ui, s);
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if ui.button("Show file").on_hover_text(&s.file).clicked() {
                                let dir = Path::new(&s.file)
                                    .parent()
                                    .map(|p| p.to_path_buf())
                                    .unwrap_or_default();
                                crate::platform::open(dir);
                            }
                            match strava {
                                Some((true, _)) => {
                                    ui.colored_label(theme::GOOD, "On Strava");
                                }
                                Some((false, true)) => {
                                    ui.spinner();
                                    ui.label("Uploading to Strava…");
                                }
                                Some((false, false)) if ui.button("Upload to Strava").clicked() => {
                                    upload = Some(s.clone());
                                }
                                _ => {}
                            }
                            ui.label(RichText::new(&s.file).small().weak());
                        });
                    });
            });
        if let Some(s) = upload {
            self.strava_upload(&s);
        }
        if !open {
            self.details = None;
        }
    }
}
