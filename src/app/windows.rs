//! Windows over the app: the welcome and the dialogs.

use super::*;

impl App {
    fn welcome_window(&mut self, ctx: &egui::Context) {
        if self.store.settings.onboarded || self.demo {
            return;
        }
        let mut action: Option<&str> = None;
        egui::Window::new(RichText::new("Welcome to VeloForge").size(20.0).strong())
            .id(egui::Id::unique("welcome"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.label("A few details make workouts, speed and heart rate zones fit you. You can change them any time in Settings.");
                ui.add_space(10.0);
                let s = &mut self.store.settings;
                egui::Grid::new("welcome-grid").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
                    ui.label("FTP");
                    ui.add(egui::DragValue::new(&mut s.ftp).range(50.0..=600.0).suffix(" W"));
                    ui.end_row();
                    ui.label("");
                    ui.label(
                        RichText::new("The power you can hold for about an hour. Not sure? Start with 2.5 W per kg of your weight, then ride the Ramp Test.")
                            .small()
                            .weak(),
                    );
                    ui.end_row();
                    ui.label("Units");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut s.units, Units::Metric, "Metric");
                        ui.radio_value(&mut s.units, Units::Imperial, "Imperial");
                    });
                    ui.end_row();
                    ui.label("Your weight");
                    settings_ui::weight_value(ui, &mut s.rider_kg, 30.0..=200.0, s.units);
                    ui.end_row();
                    ui.label("Max heart rate");
                    ui.add(egui::DragValue::new(&mut s.max_hr).range(120.0..=230.0).suffix(" bpm"));
                    ui.end_row();
                    ui.label("");
                    ui.label(RichText::new("Not sure? 220 minus your age is a rough start; VeloForge offers to update it after rides.").small().weak());
                    ui.end_row();
                    ui.label("Sounds");
                    ui.checkbox(&mut s.sounds, "Beep before interval changes");
                    ui.end_row();
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("Connect my trainer").strong()).fill(Color32::from_rgb(44, 112, 196))).clicked() {
                        action = Some("devices");
                    }
                    if ui.button("Try it with the simulator").clicked() {
                        action = Some("sim");
                    }
                    if ui.button("Skip").clicked() {
                        action = Some("skip");
                    }
                });
            });
        if let Some(a) = action {
            self.store.settings.onboarded = true;
            let s = &self.store.settings;
            self.dev.send(Cmd::SetMass(s.rider_kg + s.bike_kg));
            self.mark_dirty();
            match a {
                "devices" => {
                    self.tab = Tab::Devices;
                    self.dev.send(Cmd::StartScan);
                }
                "sim" => {
                    self.dev.send(Cmd::SimEnable(true));
                    self.tab = Tab::Ride;
                }
                _ => {}
            }
        }
    }

    pub(super) fn modals(&mut self, ctx: &egui::Context) {
        self.welcome_window(ctx);
        self.calibration_window(ctx);
        self.details_window(ctx);
        let mut close = false;
        let mut strava = None;
        match &self.modal {
            Some(Modal::Saved(s)) => {
                let s = s.as_ref().clone();
                egui::Window::new("Ride saved")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        // Scroll rather than run off a short screen.
                        let room = (ui.ctx().content_rect().height() - 120.0).max(200.0);
                        egui::ScrollArea::vertical()
                            .max_height(room)
                            .min_scrolled_height(room)
                            .show(ui, |ui| {
                                ui.heading(&s.name);
                                egui::Grid::new("sum").num_columns(2).show(ui, |ui| {
                                    for (k, v) in [
                                        ("Time", fmt_dur(s.secs)),
                                        (
                                            "Distance",
                                            self.store.settings.units.fmt_distance(s.distance_m, 2),
                                        ),
                                        (
                                            "Climbing",
                                            self.store.settings.units.fmt_height(s.ascent_m),
                                        ),
                                        ("Average power", format!("{:.0} W", s.avg_power)),
                                        ("Normalized power", format!("{:.0} W", s.np)),
                                        ("Training stress", format!("{:.0} TSS", s.tss)),
                                        ("Work", format!("{:.0} kJ", s.kj)),
                                    ] {
                                        ui.label(k);
                                        ui.label(RichText::new(v).strong());
                                        ui.end_row();
                                    }
                                });
                                zone_breakdown(ui, &s);
                                self.segment_results(ui, &s);
                                ui.add_space(4.0);
                                ui.label(RichText::new(&s.file).small().weak());
                                if self.strava_creds.is_connected() {
                                    ui.horizontal(|ui| {
                                        if s.strava_id.is_some() {
                                            ui.colored_label(theme::GOOD, "Uploaded to Strava");
                                        } else if self.strava_pending.contains(&s.file) {
                                            ui.spinner();
                                            ui.label("Uploading to Strava…");
                                        } else if ui.button("Upload to Strava").clicked() {
                                            strava = Some(s.clone());
                                        }
                                    });
                                }
                                if let Some(m) =
                                    s.max_hr.filter(|m| *m > self.store.settings.max_hr)
                                {
                                    ui.separator();
                                    ui.label(format!(
                                        "New highest heart rate: {m:.0} bpm (max is set to {:.0}).",
                                        self.store.settings.max_hr
                                    ));
                                    if ui.button(format!("Set max heart rate to {m:.0}")).clicked()
                                    {
                                        self.store.settings.max_hr = m.round();
                                        self.mark_dirty();
                                    }
                                }
                                if let Some(t) = s.lthr_estimate {
                                    ui.separator();
                                    let set = &self.store.settings;
                                    let current = if set.lthr > 0.0 {
                                        format!("currently {:.0} bpm", set.lthr)
                                    } else {
                                        "not set yet".to_string()
                                    };
                                    ui.label(
                                        RichText::new(format!(
                                            "Threshold heart rate: {t:.0} bpm ({current})"
                                        ))
                                        .size(16.0),
                                    );
                                    ui.label(
                                RichText::new(
                                    "95% of your average heart rate over the 20 minutes. Heart \
                                     rate zones based on it fit your fitness better than ones \
                                     based on your max.",
                                )
                                .small()
                                .weak(),
                            );
                                    let using = set.hr_basis == crate::store::HrBasis::Threshold
                                        && (set.lthr - t).abs() < 0.5;
                                    if !using
                                        && ui
                                            .button(format!(
                                                "Use {t:.0} bpm for my heart rate zones"
                                            ))
                                            .clicked()
                                    {
                                        let set = &mut self.store.settings;
                                        set.lthr = t;
                                        set.hr_basis = crate::store::HrBasis::Threshold;
                                        self.mark_dirty();
                                    }
                                }
                                if let Some(f) = s.ftp_estimate {
                                    ui.separator();
                                    ui.label(
                                        RichText::new(format!(
                                            "Estimated FTP: {f:.0} W (currently {:.0} W)",
                                            self.store.settings.ftp
                                        ))
                                        .size(16.0),
                                    );
                                    if ui.button(format!("Set FTP to {f:.0} W")).clicked() {
                                        self.store.settings.ftp = f;
                                        self.mark_dirty();
                                        close = true;
                                    }
                                }
                                if ui.button("Close").clicked() {
                                    close = true;
                                }
                            });
                    });
            }
            Some(Modal::ConfirmDelete(i)) => {
                let i = *i;
                let name = self
                    .store
                    .history
                    .get(i)
                    .map(|h| format!("{} on {}", h.name, h.start.format("%a %d %b")))
                    .unwrap_or_default();
                egui::Window::new("Delete ride?")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(format!("Delete {name}?"));
                        ui.label(
                            RichText::new(format!(
                                "Its files go to the {TRASH}, so you can still restore them from there."
                            ))
                            .weak(),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Delete").clicked() {
                                if i < self.store.history.len() {
                                    let ride = self.store.history.remove(i);
                                    self.trash_activity(&ride.file);
                                    // Its segment times go with it.
                                    self.store.efforts.retain(|e| {
                                        e.date != ride.start
                                            || Some(&e.route_id) != ride.route_id.as_ref()
                                    });
                                    if self
                                        .details
                                        .as_ref()
                                        .is_some_and(|d| d.summary.file == ride.file)
                                    {
                                        self.details = None;
                                    }
                                    self.mark_dirty();
                                }
                                close = true;
                            }
                            if ui.button("Keep").clicked() {
                                close = true;
                            }
                        });
                    });
            }
            Some(Modal::ConfirmDeleteWorkout(id, name)) => {
                let (id, name) = (id.clone(), name.clone());
                egui::Window::new("Delete workout?")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label(format!("Delete the workout “{name}”?"));
                        ui.label(
                            RichText::new("This removes the whole workout, not just one block.")
                                .weak(),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Delete workout").clicked() {
                                self.store.custom_workouts.retain(|c| c.id != id);
                                if self.selected_workout.as_deref() == Some(id.as_str()) {
                                    self.selected_workout = None;
                                }
                                self.editing = false;
                                self.mark_dirty();
                                close = true;
                            }
                            if ui.button("Keep").clicked() {
                                close = true;
                            }
                        });
                    });
            }
            Some(Modal::ConfirmDiscard) => {
                egui::Window::new("Discard ride?")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(ctx, |ui| {
                        ui.label("This ride will not be saved.");
                        ui.horizontal(|ui| {
                            if ui.button("Discard").clicked() {
                                self.reset_ride();
                                close = true;
                            }
                            if ui.button("Keep riding").clicked() {
                                close = true;
                            }
                        });
                    });
            }
            None => {}
        }
        if let Some(s) = strava {
            self.strava_upload(&s);
        }
        if close {
            self.modal = None;
        }
    }
}
