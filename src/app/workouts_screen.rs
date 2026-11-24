//! The Workouts tab: the library, the editor and `.zwo` export.

use super::*;

impl App {
    pub(super) fn workout_category(&self, id: &str) -> Option<Category> {
        self.find_workout(id).map(|w| w.category)
    }

    pub(super) fn workouts_tab(&mut self, ui: &mut egui::Ui) {
        let lib = self.library();
        if self.selected_workout.is_none() {
            self.selected_workout = self
                .ride_workout
                .clone()
                .or_else(|| lib.first().map(|w| w.id.clone()));
        }
        egui::Panel::left("workout-list")
            .default_size(300.0)
            .resizable(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.workout_filter)
                            .hint_text("Search…")
                            .desired_width(150.0),
                    );
                    if ui
                        .button("New")
                        .on_hover_text("Create a custom workout")
                        .clicked()
                    {
                        let id = format!("custom-{}", Local::now().timestamp());
                        self.store.custom_workouts.push(Workout {
                            id: id.clone(),
                            name: "My workout".into(),
                            description: String::new(),
                            category: Category::Custom,
                            steps: vec![
                                Step::Ramp {
                                    secs: 600,
                                    from: 0.45,
                                    to: 0.72,
                                    cadence: None,
                                },
                                Step::Steady {
                                    secs: 1200,
                                    power: 0.75,
                                    cadence: None,
                                },
                                Step::Ramp {
                                    secs: 300,
                                    from: 0.65,
                                    to: 0.4,
                                    cadence: None,
                                },
                            ],
                        });
                        self.selected_workout = Some(id);
                        self.editing = true;
                        self.mark_dirty();
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let filter = self.workout_filter.to_lowercase();
                        for cat in Category::ALL {
                            let items: Vec<&Workout> = lib
                                .iter()
                                .filter(|w| {
                                    w.category == cat
                                        && (filter.is_empty()
                                            || w.name.to_lowercase().contains(&filter))
                                })
                                .collect();
                            if items.is_empty() {
                                continue;
                            }
                            let colour = charts::category_color(cat);
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(cat.label().to_uppercase())
                                    .font(theme::semibold(10.5))
                                    .color(theme::mix(colour, Color32::WHITE, 0.15)),
                            );
                            for w in items {
                                let sel = self.selected_workout.as_deref() == Some(&w.id);
                                let (_, tss) = w.load();
                                let meta = format!("{} · {:.0} TSS", fmt_dur(w.duration()), tss);
                                if list_row(ui, sel, &w.name, &meta, colour).clicked() {
                                    self.selected_workout = Some(w.id.clone());
                                    self.editing = false;
                                }
                            }
                            ui.add_space(6.0);
                        }
                    });
            });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.picking.is_none(), egui::Button::new("Import a file…"))
                    .on_hover_text("A workout (.zwo) or a route (.gpx)")
                    .clicked()
                {
                    self.pick_file(
                        "Import a workout or route",
                        &[
                            ("Workouts and routes", &["zwo", "gpx"]),
                            ("Workouts (.zwo)", &["zwo"]),
                            ("Routes (.gpx)", &["gpx"]),
                        ],
                    );
                }
                ui.label(RichText::new("or its path:").weak());
                ui.add(
                    egui::TextEdit::singleline(&mut self.import_path)
                        .hint_text("/path/to/file.zwo")
                        .desired_width(260.0),
                );
                if ui.button("Import").clicked() {
                    let p = self.import_path.trim().to_string();
                    let p = if let Some(rest) = p.strip_prefix("~/") {
                        std::env::var("HOME")
                            .map(|h| format!("{h}/{rest}"))
                            .unwrap_or(p)
                    } else {
                        p
                    };
                    self.import_file(Path::new(&p), None);
                }
                ui.label(RichText::new("You can also drop files onto the window").weak());
            });
            ui.separator();
            let Some(id) = self.selected_workout.clone() else {
                ui.label("Select a workout on the left.");
                return;
            };
            let Some(w) = self.find_workout(&id) else {
                return;
            };
            let custom = self.store.custom_workouts.iter().position(|c| c.id == id);
            let ftp = self.store.settings.ftp;
            let (ifac, tss) = w.load();
            ui.heading(&w.name);
            ui.label(format!(
                "{} · {}  ·  IF {:.2}  ·  {:.0} TSS  ·  max {:.0} W",
                w.category.label(),
                fmt_dur(w.duration()),
                ifac,
                tss,
                w.max_target() * ftp
            ));
            if !w.description.is_empty() {
                ui.label(RichText::new(&w.description).weak());
            }
            ui.add_space(6.0);
            // While building, the builder's own chart takes its place.
            if !(custom.is_some() && self.editing) {
                let (r, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 150.0), Sense::hover());
                charts::workout_profile(
                    ui.painter(),
                    r,
                    &w.segments(),
                    ftp,
                    1.0,
                    None,
                    &[],
                    Some(CARD),
                );
                ui.add_space(6.0);
            }
            ui.horizontal(|ui| {
                let rideable = w.duration() > 0.0;
                if ui
                    .add_enabled_ui(rideable, |ui| {
                        icon_button(
                            ui,
                            Icon::Play,
                            "Ride now",
                            Some(Color32::from_rgb(36, 150, 84)),
                        )
                    })
                    .inner
                    .clicked()
                {
                    self.load_ride(Some(id.clone()), None);
                    self.tab = Tab::Ride;
                }
                ui.separator();
                ui.label("Schedule on");
                date_picker(ui, &mut self.schedule_date);
                if ui.button("Add to calendar").clicked() {
                    self.store.plan.push(PlannedWorkout {
                        date: self.schedule_date,
                        workout_id: id.clone(),
                        done: false,
                        skipped: false,
                    });
                    self.mark_dirty();
                    self.notify(format!(
                        "Scheduled {} on {}",
                        w.name,
                        self.schedule_date.format("%a %d %b")
                    ));
                }
                ui.separator();
                if custom.is_some() {
                    if ui.selectable_label(self.editing, "Edit").clicked() {
                        self.editing = !self.editing;
                    }
                    if ui
                        .button("Delete workout")
                        .on_hover_text("Delete this whole workout (asks first)")
                        .clicked()
                    {
                        self.modal = Some(Modal::ConfirmDeleteWorkout(id.clone(), w.name.clone()));
                    }
                } else if ui.button("Copy to custom").clicked() {
                    let mut c = w.clone();
                    c.id = format!("custom-{}", Local::now().timestamp());
                    c.name = format!("{} (copy)", w.name);
                    c.category = Category::Custom;
                    self.selected_workout = Some(c.id.clone());
                    self.store.custom_workouts.push(c);
                    self.editing = true;
                    self.mark_dirty();
                }
                if ui
                    .button("Export .zwo")
                    .on_hover_text(
                        "Save it as a Zwift workout file, for Zwift, TrainerRoad, MyWhoosh, \
                         intervals.icu and others (into your Downloads folder)",
                    )
                    .clicked()
                {
                    self.export_zwo(&w);
                }
            });
            ui.separator();
            // The workout must still be there (and in the same place).
            let custom = custom.filter(|&ci| {
                self.store
                    .custom_workouts
                    .get(ci)
                    .is_some_and(|c| c.id == id)
            });
            match (custom, self.editing) {
                (Some(ci), true) => {
                    if self.workout_editor(ui, ci, &id, ftp) {
                        self.mark_dirty();
                    }
                }
                _ => {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (i, s) in w.steps.iter().enumerate() {
                            ui.label(format!("{:>2}. {}", i + 1, s.describe()));
                        }
                    });
                }
            }
        });
    }

    /// Save `w` as a .zwo file in the Downloads folder (or the data folder),
    /// and show where.
    fn export_zwo(&mut self, w: &Workout) {
        let dir = directories::UserDirs::new()
            .and_then(|u| u.download_dir().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| self.paths.data.clone());
        let slug: String = w
            .name
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect();
        let path = dir.join(format!("{}.zwo", slug.trim_matches('_')));
        match std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::write(&path, crate::workout::to_zwo(w)))
        {
            Ok(()) => {
                self.log_line(format!("Exported {} to {}", w.name, path.display()));
                self.notify(format!("Saved {}", path.display()));
            }
            Err(e) => self.notify(format!("Could not save the .zwo file: {e}")),
        }
    }

    /// A custom workout's name and description, and the builder for its
    /// steps.
    fn workout_editor(&mut self, ui: &mut egui::Ui, ci: usize, id: &str, ftp: f32) -> bool {
        let mut changed = false;
        let w = &mut self.store.custom_workouts[ci];
        egui::Grid::new("wk-meta").num_columns(2).show(ui, |ui| {
            ui.label("Name");
            changed |= ui.text_edit_singleline(&mut w.name).changed();
            ui.end_row();
            ui.label("Description");
            changed |= ui.text_edit_singleline(&mut w.description).changed();
            ui.end_row();
        });
        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .id_salt("builder-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                changed |= self.builder.show(ui, id, &mut w.steps, ftp);
            });
        changed
    }
}
