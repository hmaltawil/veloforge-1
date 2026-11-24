//! The Calendar & Plan tab.

use super::*;

impl App {
    pub(super) fn plan_tab(&mut self, ui: &mut egui::Ui) {
        let lib = self.library();
        let by_id = |id: &str| lib.iter().find(|w| w.id == id);
        egui::Panel::left("plan-gen")
            .default_size(290.0)
            .resizable(false)
            .show(ui, |ui| {
                ui.heading("Generate a plan");
                ui.add_space(4.0);
                egui::ComboBox::from_label("Goal")
                    .selected_text(self.plan_cfg.goal.label())
                    .show_ui(ui, |ui| {
                        for g in Goal::ALL {
                            ui.selectable_value(&mut self.plan_cfg.goal, g, g.label());
                        }
                    });
                ui.label(RichText::new(self.plan_cfg.goal.description()).weak());
                ui.add_space(6.0);
                ui.label("Start date");
                date_picker(ui, &mut self.plan_cfg.start);
                ui.add(egui::Slider::new(&mut self.plan_cfg.weeks, 2..=16).text("weeks"));
                ui.add_space(6.0);
                ui.label("Riding days");
                ui.horizontal_wrapped(|ui| {
                    for (d, on) in self.plan_cfg.days.iter_mut().enumerate() {
                        ui.toggle_value(on, plan::weekday_label(d));
                    }
                });
                egui::ComboBox::from_label("Long ride day")
                    .selected_text(plan::weekday_label(self.plan_cfg.long_day))
                    .show_ui(ui, |ui| {
                        for d in 0..7 {
                            ui.selectable_value(
                                &mut self.plan_cfg.long_day,
                                d,
                                plan::weekday_label(d),
                            );
                        }
                    });
                ui.checkbox(
                    &mut self.replace_plan,
                    "Replace planned (not done) workouts from start date",
                );
                ui.add_space(6.0);
                let preview = plan::generate(&self.plan_cfg);
                let hours: f32 = preview
                    .iter()
                    .filter_map(|p| by_id(&p.workout_id))
                    .map(|w| w.duration())
                    .sum::<f32>()
                    / 3600.0;
                let weeks = self.plan_cfg.weeks.max(1) as f32;
                ui.label(format!(
                    "{} workouts · ~{:.1} h/week",
                    preview.len(),
                    hours / weeks
                ));
                if ui
                    .add_enabled(
                        !preview.is_empty(),
                        egui::Button::new(RichText::new("Generate plan").color(Color32::WHITE))
                            .fill(Color32::from_rgb(44, 112, 196)),
                    )
                    .clicked()
                {
                    if self.replace_plan {
                        let start = self.plan_cfg.start;
                        self.store.plan.retain(|p| p.done || p.date < start);
                    }
                    let n = preview.len();
                    self.store.plan.extend(preview);
                    self.store.plan.sort_by_key(|p| p.date);
                    self.month = plan::first_of_month(self.plan_cfg.start);
                    self.mark_dirty();
                    self.notify(format!("Added {n} workouts to your calendar"));
                }
                ui.add_space(10.0);
                ui.separator();
                let today = Local::now().date_naive();
                if self.icu_creds.is_set() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("intervals.icu").strong());
                        if self.icu_busy {
                            ui.spinner();
                        } else if ui.small_button("Sync now").clicked() {
                            self.icu_sync();
                        }
                    });
                    if let Some(n) = &self.icu_note {
                        ui.label(RichText::new(n).small().weak());
                    }
                    ui.add_space(6.0);
                }
                ui.label(RichText::new("Up next").strong());
                for p in self
                    .store
                    .plan
                    .iter()
                    .filter(|p| p.date >= today && !p.done)
                    .take(6)
                {
                    let name = by_id(&p.workout_id)
                        .map(|w| w.name.as_str())
                        .unwrap_or("(missing)");
                    ui.label(format!("{}  {}", p.date.format("%a %d %b"), name));
                }
                ui.add_space(6.0);
                if ui.button("Clear all future planned workouts").clicked() {
                    self.store.plan.retain(|p| p.done || p.date < today);
                    self.mark_dirty();
                }
            });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            self.missed_banner(ui);
            self.tired_banner(ui, false);
            ui.horizontal(|ui| {
                if icon_button(ui, Icon::Left, "", None)
                    .on_hover_text("Previous month")
                    .clicked()
                {
                    self.month = plan::add_months(self.month, -1);
                }
                ui.heading(self.month.format("%B %Y").to_string());
                if icon_button(ui, Icon::Right, "", None)
                    .on_hover_text("Next month")
                    .clicked()
                {
                    self.month = plan::add_months(self.month, 1);
                }
                if ui.button("Today").clicked() {
                    self.month = plan::first_of_month(Local::now().date_naive());
                    self.sel_day = Local::now().date_naive();
                }
            });
            let today = Local::now().date_naive();
            let detail_h = 190.0;
            let avail = ui.available_rect_before_wrap();
            let grid = Rect::from_min_max(
                avail.min,
                Pos2::new(avail.max.x - 70.0, avail.max.y - detail_h),
            );
            let cols = 7.0;
            let head = 22.0;
            let cw = grid.width() / cols;
            let ch = (grid.height() - head) / 6.0;
            let painter = ui.painter().clone();
            for d in 0..7 {
                painter.text(
                    Pos2::new(grid.left() + cw * (d as f32 + 0.5), grid.top() + 10.0),
                    Align2::CENTER_CENTER,
                    plan::weekday_label(d),
                    FontId::proportional(13.0),
                    Color32::from_gray(170),
                );
            }
            let start = plan::month_grid_start(self.month);
            let mut clicked_day = None;
            for row in 0..6 {
                let mut week_tss = 0.0;
                for col in 0..7 {
                    let date = start + Duration::days(row * 7 + col);
                    let r = Rect::from_min_size(
                        Pos2::new(
                            grid.left() + cw * col as f32,
                            grid.top() + head + ch * row as f32,
                        ),
                        Vec2::new(cw, ch),
                    )
                    .shrink(2.0);
                    let resp = ui.interact(r, ui.scope_id().with(("cal", date)), Sense::click());
                    if resp.clicked() {
                        clicked_day = Some(date);
                    }
                    let in_month = date.month() == self.month.month();
                    let mut fill = if in_month {
                        CARD
                    } else {
                        Color32::from_rgb(24, 26, 32)
                    };
                    if resp.hovered() {
                        fill = fill.gamma_multiply(1.3);
                    }
                    painter.rect_filled(r, CornerRadius::same(6), fill);
                    if date == self.sel_day {
                        painter.rect_stroke(
                            r,
                            CornerRadius::same(6),
                            Stroke::new(2.0, Color32::from_rgb(240, 130, 60)),
                            egui::StrokeKind::Inside,
                        );
                    } else if date == today {
                        painter.rect_stroke(
                            r,
                            CornerRadius::same(6),
                            Stroke::new(1.5, Color32::from_rgb(90, 160, 240)),
                            egui::StrokeKind::Inside,
                        );
                    }
                    painter.text(
                        r.left_top() + Vec2::new(6.0, 4.0),
                        Align2::LEFT_TOP,
                        date.day().to_string(),
                        FontId::proportional(12.0),
                        if in_month {
                            Color32::from_gray(220)
                        } else {
                            Color32::from_gray(110)
                        },
                    );
                    let mut yy = r.top() + 22.0;
                    for p in self.store.plan.iter().filter(|p| p.date == date) {
                        let Some(w) = by_id(&p.workout_id) else {
                            continue;
                        };
                        week_tss += w.load().1;
                        if yy + 16.0 > r.bottom() {
                            break;
                        }
                        let pill = Rect::from_min_size(
                            Pos2::new(r.left() + 4.0, yy),
                            Vec2::new(r.width() - 8.0, 16.0),
                        );
                        let c = charts::category_color(w.category);
                        let missed = !p.done && !p.skipped && p.date < today;
                        let fill = if p.skipped {
                            Color32::from_gray(70)
                        } else if p.done {
                            c.gamma_multiply(0.45)
                        } else if missed {
                            c.gamma_multiply(0.35)
                        } else {
                            c.gamma_multiply(0.85)
                        };
                        painter.rect_filled(pill, CornerRadius::same(4), fill);
                        if missed {
                            painter.rect_stroke(
                                pill,
                                CornerRadius::same(4),
                                Stroke::new(1.5, Color32::from_rgb(235, 100, 80)),
                                egui::StrokeKind::Inside,
                            );
                        }
                        let mut tx = pill.left() + 4.0;
                        if p.done {
                            paint_icon(
                                &painter,
                                Rect::from_min_size(
                                    Pos2::new(tx, pill.top() + 3.0),
                                    Vec2::splat(10.0),
                                ),
                                Icon::Check,
                                Color32::BLACK,
                            );
                            tx += 13.0;
                        }
                        painter.with_clip_rect(pill).text(
                            Pos2::new(tx, pill.center().y),
                            Align2::LEFT_CENTER,
                            &w.name,
                            FontId::proportional(11.0),
                            Color32::BLACK,
                        );
                        yy += 18.0;
                    }
                    for h in self
                        .store
                        .history
                        .iter()
                        .filter(|h| h.start.date_naive() == date && h.workout_id.is_none())
                    {
                        if yy + 16.0 > r.bottom() {
                            break;
                        }
                        let pill = Rect::from_min_size(
                            Pos2::new(r.left() + 4.0, yy),
                            Vec2::new(r.width() - 8.0, 16.0),
                        );
                        painter.rect_filled(pill, CornerRadius::same(4), Color32::from_gray(80));
                        painter.with_clip_rect(pill).text(
                            pill.left_center() + Vec2::new(4.0, 0.0),
                            Align2::LEFT_CENTER,
                            format!(
                                "Ridden: {} · {}",
                                h.name,
                                self.store.settings.units.fmt_distance(h.distance_m, 0)
                            ),
                            FontId::proportional(11.0),
                            Color32::WHITE,
                        );
                        yy += 18.0;
                    }
                }
                let wr = Rect::from_min_size(
                    Pos2::new(
                        grid.right() + 6.0,
                        grid.top() + head + ch * row as f32 + 2.0,
                    ),
                    Vec2::new(62.0, ch - 4.0),
                );
                painter.text(
                    wr.center(),
                    Align2::CENTER_CENTER,
                    format!("{week_tss:.0}\nTSS"),
                    FontId::proportional(12.0),
                    Color32::from_gray(160),
                );
            }
            if let Some(d) = clicked_day {
                self.sel_day = d;
            }
            ui.allocate_rect(
                Rect::from_min_max(avail.min, Pos2::new(avail.max.x, avail.max.y - detail_h)),
                Sense::hover(),
            );

            // Day detail.
            ui.separator();
            ui.horizontal(|ui| {
                ui.heading(self.sel_day.format("%A %d %B").to_string());
                ui.separator();
                egui::ComboBox::from_id_salt("add-wk")
                    .width(220.0)
                    .selected_text(
                        by_id(&self.add_workout)
                            .map(|w| w.name.clone())
                            .unwrap_or_default(),
                    )
                    .show_ui(ui, |ui| {
                        for w in &lib {
                            ui.selectable_value(&mut self.add_workout, w.id.clone(), &w.name);
                        }
                    });
                if ui.button("Add to this day").clicked() {
                    self.store.plan.push(PlannedWorkout {
                        date: self.sel_day,
                        workout_id: self.add_workout.clone(),
                        done: false,
                        skipped: false,
                    });
                    self.mark_dirty();
                }
            });
            let mut action: Option<(usize, &str)> = None;
            egui::ScrollArea::vertical()
                .id_salt("day-detail")
                .show(ui, |ui| {
                    for (i, p) in self
                        .store
                        .plan
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| p.date == self.sel_day)
                    {
                        let w = by_id(&p.workout_id);
                        ui.horizontal(|ui| {
                            let (name, info) = match w {
                                Some(w) => (
                                    w.name.clone(),
                                    format!("{} · {:.0} TSS", fmt_dur(w.duration()), w.load().1),
                                ),
                                None => (format!("{} (missing)", p.workout_id), String::new()),
                            };
                            ui.label(RichText::new(name).strong());
                            ui.label(info);
                            if p.done {
                                ui.label(
                                    RichText::new("Done")
                                        .strong()
                                        .color(Color32::from_rgb(110, 210, 130)),
                                );
                            }
                            if w.is_some() && icon_button(ui, Icon::Play, "Ride", None).clicked() {
                                action = Some((i, "ride"));
                            }
                            if ui
                                .button(if p.done { "Mark not done" } else { "Mark done" })
                                .clicked()
                            {
                                action = Some((i, "toggle"));
                            }
                            if icon_button(ui, Icon::Left, "Day earlier", None).clicked() {
                                action = Some((i, "earlier"));
                            }
                            if icon_button(ui, Icon::Right, "Day later", None).clicked() {
                                action = Some((i, "later"));
                            }
                            if ui.button("Remove").clicked() {
                                action = Some((i, "remove"));
                            }
                        });
                    }
                    for h in self
                        .store
                        .history
                        .iter()
                        .filter(|h| h.start.date_naive() == self.sel_day)
                    {
                        ui.label(format!(
                            "Ridden: {} · {} · {} · {:.0} W avg · {:.0} TSS",
                            h.name,
                            fmt_dur(h.secs),
                            self.store.settings.units.fmt_distance(h.distance_m, 1),
                            h.avg_power,
                            h.tss
                        ));
                    }
                });
            if let Some((i, a)) = action {
                match a {
                    "ride" => {
                        let p = self.store.plan[i].clone();
                        self.load_ride(Some(p.workout_id), Some(p.date));
                        self.tab = Tab::Ride;
                    }
                    "toggle" => self.store.plan[i].done = !self.store.plan[i].done,
                    "earlier" => {
                        self.store.plan[i].date -= Duration::days(1);
                        self.sel_day = self.store.plan[i].date;
                    }
                    "later" => {
                        self.store.plan[i].date += Duration::days(1);
                        self.sel_day = self.store.plan[i].date;
                    }
                    "remove" => {
                        self.store.plan.remove(i);
                    }
                    _ => {}
                }
                self.mark_dirty();
            }
        });
    }
}
