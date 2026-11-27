//! Home: today's workout with one button to start it, the week's planned
//! rides, what comes next, and whether the devices are ready.

use super::*;

/// Home's sidebar width, as in the mock-ups.
const SIDEBAR: f32 = 236.0;
/// The bar along the bottom with the ride and the devices.
const RIDE_BAR: f32 = 122.0;
/// The ride bar when its choices need two lines (a narrow window).
const RIDE_BAR_TALL: f32 = 162.0;

impl App {
    pub(super) fn home_tab(&mut self, ui: &mut egui::Ui) {
        let today = Local::now().date_naive();
        let full = ui.max_rect();
        let main = if full.width() > 820.0 {
            let side =
                Rect::from_min_max(full.min, Pos2::new(full.left() + SIDEBAR, full.bottom()));
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::unique(super::pad::SIDE_KEY), side.right()));
            ui.painter().line_segment(
                [side.right_top(), side.right_bottom()],
                Stroke::new(1.0, Color32::from_white_alpha(31)),
            );
            let inner = side.shrink2(Vec2::new(18.0, 26.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                self.home_sidebar(ui, today)
            });
            Rect::from_min_max(Pos2::new(side.right(), full.top()), full.max)
        } else {
            full
        };
        let inner = Rect::from_min_max(
            main.min + Vec2::new(28.0, 26.0),
            main.max - Vec2::new(28.0, 20.0),
        );
        let bar = Rect::from_min_max(
            Pos2::new(
                inner.left(),
                inner.bottom()
                    - if inner.width() < 780.0 {
                        RIDE_BAR_TALL
                    } else {
                        RIDE_BAR
                    },
            ),
            inner.max,
        );
        // With a Click, a line above the bar says what its buttons do.
        let legend = self.pad_nav() && self.pad_hints();
        let content = Rect::from_min_max(
            inner.min,
            Pos2::new(inner.right(), bar.top() - if legend { 46.0 } else { 18.0 }),
        );
        if legend {
            super::home_click::legend(
                ui.painter(),
                Pos2::new(bar.center().x, bar.top() - 22.0),
                self.store.settings.click_pad,
            );
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
            self.home_main(ui, today)
        });
        self.home_ride_bar(ui, bar, today);
    }

    /// Today's planned workout, if it has not been ridden or skipped.
    fn todays_workout(&self, today: NaiveDate) -> Option<Workout> {
        self.store
            .plan
            .iter()
            .find(|p| p.date == today && !p.done && !p.skipped)
            .and_then(|p| self.find_workout(&p.workout_id))
    }

    /// Start today's workout, or a free ride; while a ride is going, just
    /// go back to it.
    fn home_start(&mut self, workout: Option<String>, today: NaiveDate) {
        if !self.ride_active() {
            let date = workout.as_ref().map(|_| today);
            self.load_ride(workout, date);
        }
        self.tab = Tab::Ride;
    }

    /// The name of the route the next ride is on, without its distance.
    fn route_short_name(&self) -> String {
        self.routes
            .get(self.route_idx)
            .map(|r| r.name.split(" · ").next().unwrap_or(&r.name).to_string())
            .unwrap_or_default()
    }

    fn home_sidebar(&mut self, ui: &mut egui::Ui, today: NaiveDate) {
        ui.spacing_mut().item_spacing.y = 4.0;
        // Search finds workouts in the Library.
        let (row, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::hover());
        side_icon(ui.painter(), row, SideIcon::Search);
        let field = Rect::from_min_max(Pos2::new(row.left() + 42.0, row.top()), row.max);
        let edit = ui.put(
            field,
            egui::TextEdit::singleline(&mut self.workout_filter)
                .hint_text("Search")
                .frame(egui::Frame::NONE)
                .align(Align2::LEFT_CENTER),
        );
        if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            self.tab = Tab::Library;
        }
        if side_row(ui, SideIcon::FreeRide, "Free ride").clicked() {
            self.home_start(None, today);
        }
        if side_row(ui, SideIcon::Workouts, "Workouts").clicked() {
            self.tab = Tab::Library;
        }
        if side_row(ui, SideIcon::Routes, "Routes").clicked() {
            self.library_view = library_screen::LibraryView::Routes;
            self.tab = Tab::Library;
        }
        if side_row(ui, SideIcon::Random, "Random route").clicked() {
            if self.ride_active() {
                self.notify("Finish the current ride first");
            } else {
                self.new_random_route();
                self.tab = Tab::Ride;
            }
        }

        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                RichText::new("Coming up")
                    .font(theme::medium(15.0))
                    .color(Color32::WHITE),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(12.0);
                let (r, _) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::hover());
                super::menu::menu_icon(ui.painter(), r.center(), Tab::Calendar, theme::TEXT);
            });
        });
        ui.add_space(2.0);
        let next: Vec<(NaiveDate, Workout)> = self
            .store
            .plan
            .iter()
            .filter(|p| p.date > today && !p.done && !p.skipped)
            .take(4)
            .filter_map(|p| self.find_workout(&p.workout_id).map(|w| (p.date, w)))
            .collect();
        if next.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(RichText::new("Nothing planned yet").color(theme::TEXT_DIM));
            });
        }
        for (date, w) in &next {
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 52.0), Sense::click());
            let p = ui.painter();
            if resp.hovered() {
                p.rect_filled(rect, CornerRadius::same(12), Color32::from_white_alpha(26));
            }
            let tile = Rect::from_min_size(
                Pos2::new(rect.left() + 12.0, rect.center().y - 18.0),
                Vec2::splat(36.0),
            );
            p.rect_filled(tile, CornerRadius::same(9), Color32::from_white_alpha(31));
            p.text(
                tile.center() - Vec2::new(0.0, 7.0),
                Align2::CENTER_CENTER,
                date.format("%a").to_string().to_uppercase(),
                theme::medium(9.5),
                theme::TEXT_DIM,
            );
            p.text(
                tile.center() + Vec2::new(0.0, 6.0),
                Align2::CENTER_CENTER,
                date.day().to_string(),
                theme::semibold(14.0),
                Color32::WHITE,
            );
            let x = tile.right() + 12.0;
            let text_clip =
                p.with_clip_rect(Rect::from_min_max(Pos2::new(x, rect.top()), rect.max));
            text_clip.text(
                Pos2::new(x, rect.center().y - 9.0),
                Align2::LEFT_CENTER,
                &w.name,
                theme::regular(13.5),
                Color32::WHITE,
            );
            text_clip.text(
                Pos2::new(x, rect.center().y + 9.0),
                Align2::LEFT_CENTER,
                format!("{} · {:.0} TSS", fmt_dur(w.duration()), w.load().1),
                theme::regular(12.0),
                theme::TEXT_DIM,
            );
            if resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                self.sel_day = *date;
                self.tab = Tab::Calendar;
            }
        }

        // "Plan a workout" sits at the bottom of the sidebar.
        let left = ui.available_height() - 40.0;
        if left > 0.0 {
            ui.add_space(left);
        }
        if side_row(ui, SideIcon::Plus, "Plan a workout").clicked() {
            self.sel_day = today;
            self.tab = Tab::Calendar;
        }
    }

    fn home_main(&mut self, ui: &mut egui::Ui, today: NaiveDate) {
        let greeting = match chrono::Timelike::hour(&Local::now()) {
            5..=11 => "Good morning",
            12..=17 => "Good afternoon",
            _ => "Good evening",
        };
        ui.label(
            RichText::new(greeting)
                .font(theme::medium(26.0))
                .color(Color32::WHITE),
        );
        if self.store.settings.home_quote {
            ui.add_space(6.0);
            self.home_quote(ui, today);
            ui.add_space(10.0);
        } else {
            ui.add_space(14.0);
        }
        self.home_featured(ui, today);
        ui.add_space(18.0);
        self.home_week(ui, today);
    }

    /// A line about cycling that suits the day, with a button for another.
    fn home_quote(&mut self, ui: &mut egui::Ui, today: NaiveDate) {
        use super::glass;
        let planned = self.todays_workout(today).map(|w| w.category);
        let rode = |d: NaiveDate| self.store.history.iter().any(|h| h.start.date_naive() == d);
        let days_in_a_row = (1..=7)
            .take_while(|&k| rode(today - chrono::Duration::days(k)))
            .count() as u32;
        let plan_nearby = self
            .store
            .plan
            .iter()
            .any(|p| (p.date - today).num_days().abs() <= 3 && !p.skipped);
        let day = crate::quotes::kind_of_day(planned, rode(today), days_in_a_row, plan_nearby);
        let quote = crate::quotes::pick(day, today, self.quote_more);
        let width = ui.available_width();
        let mut another = false;
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let (mark, _) = ui.allocate_exact_size(Vec2::new(30.0, 44.0), Sense::hover());
            ui.painter().text(
                mark.left_top() + Vec2::new(0.0, -6.0),
                Align2::LEFT_TOP,
                "\u{201c}",
                theme::light(66.0),
                Color32::from_white_alpha(128),
            );
            // The text takes the width the button leaves it.
            let text_w = (width - 30.0 - 36.0 - 2.0 * 12.0).max(120.0);
            ui.vertical(|ui| {
                ui.set_width(text_w);
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.add_space(6.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(quote.text)
                            .font(theme::light(21.0))
                            .color(Color32::WHITE),
                    )
                    .wrap(),
                );
                if !quote.by.is_empty() {
                    ui.label(
                        RichText::new(quote.by)
                            .font(theme::regular(12.5))
                            .color(glass::QUIET),
                    );
                }
            });
            ui.vertical(|ui| {
                ui.add_space(2.0);
                let (r, resp) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::click());
                ui.painter().circle_filled(
                    r.center(),
                    18.0,
                    Color32::from_white_alpha(if resp.hovered() { 46 } else { 26 }),
                );
                glass::glyph(
                    ui.painter(),
                    r.center(),
                    glass::Glyph::Refresh,
                    Color32::WHITE,
                );
                another = resp
                    .on_hover_text("Another quote")
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked();
            });
        });
        if another {
            self.quote_more += 1;
        }
    }

    /// Today's workout: what it is, its shape in zone colours, and Start.
    fn home_featured(&mut self, ui: &mut egui::Ui, today: NaiveDate) {
        let planned = self.todays_workout(today);
        let route = self.route_short_name();
        let mut start = false;
        egui::Frame::new()
            .fill(Color32::from_white_alpha(23))
            .corner_radius(22)
            .inner_margin(18)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let button = 64.0;
                    let text_w = ui.available_width() - button - 24.0;
                    ui.vertical(|ui| {
                        ui.set_width(text_w);
                        ui.spacing_mut().item_spacing.y = 6.0;
                        match &planned {
                            Some(w) => {
                                let (ifac, tss) = w.load();
                                ui.label(
                                    RichText::new("Planned for today")
                                        .font(theme::regular(13.0))
                                        .color(theme::TEXT_DIM),
                                );
                                ui.label(
                                    RichText::new(&w.name)
                                        .font(theme::semibold(20.0))
                                        .color(Color32::WHITE),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "{} · {tss:.0} TSS · IF {ifac:.2} · on {route}",
                                        fmt_dur(w.duration()),
                                    ))
                                    .font(theme::regular(13.0))
                                    .color(theme::TEXT),
                                );
                                ui.add_space(4.0);
                                let (r, _) = ui.allocate_exact_size(
                                    Vec2::new(ui.available_width(), 40.0),
                                    Sense::hover(),
                                );
                                zone_blocks(ui.painter(), r, &w.segments());
                            }
                            None => {
                                ui.label(
                                    RichText::new("Today")
                                        .font(theme::regular(13.0))
                                        .color(theme::TEXT_DIM),
                                );
                                ui.label(
                                    RichText::new("Nothing planned")
                                        .font(theme::semibold(20.0))
                                        .color(Color32::WHITE),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "Ride freely on {route}, or pick a workout in the Library."
                                    ))
                                    .font(theme::regular(13.0))
                                    .color(theme::TEXT),
                                );
                            }
                        }
                    });
                    ui.add_space(12.0);
                    ui.vertical_centered(|ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        let label = if planned.is_some() {
                            "Start"
                        } else {
                            "Free ride"
                        };
                        if play_button(ui, button, label).clicked() {
                            start = true;
                        }
                        ui.label(
                            RichText::new(label)
                                .font(theme::regular(12.0))
                                .color(theme::TEXT),
                        );
                    });
                });
            });
        if start {
            self.home_start(planned.map(|w| w.id), today);
        }
    }

    /// The week's planned rides, Monday to Sunday, and how far through
    /// them you are.
    fn home_week(&mut self, ui: &mut egui::Ui, today: NaiveDate) {
        let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
        let sunday = monday + Duration::days(6);
        let week: Vec<(PlannedWorkout, Workout)> = self
            .store
            .plan
            .iter()
            .filter(|p| (monday..=sunday).contains(&p.date))
            .filter_map(|p| self.find_workout(&p.workout_id).map(|w| (p.clone(), w)))
            .collect();
        let done = crate::progress::totals_between(&self.store.history, monday, today);
        let planned_secs: f32 = week
            .iter()
            .filter(|(p, _)| !p.skipped)
            .map(|(_, w)| w.duration())
            .sum();
        let planned_tss: f32 = week
            .iter()
            .filter(|(p, _)| !p.skipped)
            .map(|(_, w)| w.load().1)
            .sum();
        let (_, _, form) = crate::progress::fitness(&self.store.history, today, 1).today();
        let word = match form {
            t if t > 10.0 => "fresh",
            t if t > -10.0 => "balanced",
            t if t > -25.0 => "training hard",
            _ => "very tired",
        };

        ui.horizontal(|ui| {
            ui.label(
                RichText::new("This week")
                    .font(theme::medium(20.0))
                    .color(Color32::WHITE),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!(
                        "{} of {} h · {:.0} of {:.0} TSS · Form {form:+.0}, {word}",
                        hours_minutes(done.secs),
                        hours_minutes(planned_secs),
                        done.tss,
                        planned_tss,
                    ))
                    .font(theme::regular(13.0))
                    .color(theme::TEXT),
                );
            });
        });
        ui.add_space(8.0);
        if week.is_empty() {
            ui.label(RichText::new("Nothing planned this week.").color(theme::TEXT_DIM));
            return;
        }
        let mut open = None;
        egui::ScrollArea::vertical()
            .id_salt("home-week")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                for (p, w) in &week {
                    let (rect, resp) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), 58.0), Sense::click());
                    let painter = ui.painter();
                    let is_today = p.date == today;
                    let fill = if resp.hovered() {
                        Some(Color32::from_white_alpha(31))
                    } else if is_today {
                        Some(Color32::from_white_alpha(26))
                    } else {
                        None
                    };
                    if let Some(f) = fill {
                        painter.rect_filled(rect, CornerRadius::same(14), f);
                    }
                    let mid = rect.center().y;
                    painter.text(
                        Pos2::new(rect.left() + 14.0, mid),
                        Align2::LEFT_CENTER,
                        p.date.format("%a %-d").to_string(),
                        theme::regular(13.0),
                        theme::TEXT,
                    );
                    let tile = Rect::from_min_size(
                        Pos2::new(rect.left() + 82.0, mid - 20.0),
                        Vec2::splat(40.0),
                    );
                    painter.rect_filled(
                        tile,
                        CornerRadius::same(10),
                        Color32::from_white_alpha(26),
                    );
                    mini_bars(painter, tile.shrink(5.0), &w.segments());
                    let x = tile.right() + 14.0;
                    painter.text(
                        Pos2::new(x, mid - 9.0),
                        Align2::LEFT_CENTER,
                        &w.name,
                        theme::medium(14.0),
                        Color32::WHITE,
                    );
                    painter.text(
                        Pos2::new(x, mid + 10.0),
                        Align2::LEFT_CENTER,
                        format!(
                            "{} · {:.0} TSS · {}",
                            fmt_dur(w.duration()),
                            w.load().1,
                            w.category.label()
                        ),
                        theme::regular(12.5),
                        theme::TEXT_DIM,
                    );
                    let (status, colour) = if p.done {
                        ("Done", Color32::from_rgb(110, 231, 160))
                    } else if p.skipped {
                        ("Skipped", theme::TEXT_DIM)
                    } else if is_today {
                        ("Today", Color32::WHITE)
                    } else if p.date < today {
                        ("Missed", Color32::from_rgb(255, 180, 171))
                    } else {
                        ("Planned", theme::TEXT_DIM)
                    };
                    painter.text(
                        Pos2::new(rect.right() - 16.0, mid),
                        Align2::RIGHT_CENTER,
                        status,
                        theme::regular(12.5),
                        colour,
                    );
                    if resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        open = Some(p.date);
                    }
                }
            });
        if let Some(d) = open {
            self.sel_day = d;
            self.tab = Tab::Calendar;
        }
    }

    /// The bar along the bottom: the ride you would start, whether the
    /// devices are ready, and a round start button.
    fn home_ride_bar(&mut self, ui: &mut egui::Ui, whole: Rect, _today: NaiveDate) {
        {
            let painter = ui.painter();
            let r = CornerRadius::same(20);
            painter.rect_filled(whole, r, Color32::from_white_alpha(31));
            painter.rect_stroke(
                whole,
                r,
                Stroke::new(1.0, Color32::from_white_alpha(36)),
                egui::StrokeKind::Inside,
            );
        }
        // Above: what to ride and where (only before a ride starts).
        let bar = Rect::from_min_max(Pos2::new(whole.left(), whole.bottom() - 64.0), whole.max);
        let choices = Rect::from_min_max(
            whole.min + Vec2::new(16.0, 12.0),
            Pos2::new(whole.right() - 16.0, bar.top()),
        );
        if self.ride_active() {
            ui.painter().text(
                choices.left_center(),
                Align2::LEFT_CENTER,
                "A ride is going: finish it to choose another.",
                theme::regular(13.0),
                theme::TEXT_DIM,
            );
        } else {
            super::glass::area(ui, choices, "home-choices", |ui| self.ride_choices(ui));
        }
        ui.painter().line_segment(
            [
                Pos2::new(whole.left() + 16.0, bar.top()),
                Pos2::new(whole.right() - 16.0, bar.top()),
            ],
            Stroke::new(1.0, Color32::from_white_alpha(31)),
        );
        let painter = ui.painter();
        let mid = bar.center().y;
        let x = bar.left() + 18.0;
        let (name, mode) = match self.ride.mode {
            Mode::Workout => (
                self.ride
                    .workout
                    .as_ref()
                    .map(|w| w.name.clone())
                    .unwrap_or_else(|| "Workout".into()),
                "ERG",
            ),
            Mode::Route => ("Free ride".to_string(), "follows the road"),
            Mode::Slope => ("Slope".to_string(), "fixed gradient"),
            Mode::HeartRate => ("Heart rate ride".to_string(), "holds your heart rate"),
        };
        painter.text(
            Pos2::new(x, mid - 9.0),
            Align2::LEFT_CENTER,
            &name,
            theme::medium(14.0),
            Color32::WHITE,
        );
        painter.text(
            Pos2::new(x, mid + 10.0),
            Align2::LEFT_CENTER,
            format!("Today · {} · {mode}", self.route_short_name()),
            theme::regular(12.5),
            theme::TEXT_DIM,
        );

        // Start, on the right.
        let start =
            Rect::from_center_size(Pos2::new(bar.right() - 12.0 - 22.0, mid), Vec2::splat(44.0));
        let resp = ui
            .interact(start, ui.scope_id().with("home-bar-start"), Sense::click())
            .on_hover_text(if self.ride_active() {
                "Back to the ride".to_string()
            } else {
                format!("Ride: {name}")
            })
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        paint_play(ui.painter(), start, resp.hovered());
        if resp.clicked() {
            self.tab = Tab::Ride;
        }

        // The devices, left of Start; they open Devices.
        let devices = [
            (self.trainer_link().is_some(), "Trainer"),
            (fresh(&self.metrics.power).is_some(), "Power"),
            (fresh(&self.metrics.hr).is_some(), "HR"),
        ];
        let font = theme::regular(12.5);
        let mut right = start.left() - 18.0;
        let mut area = Rect::NOTHING;
        for (ok, label) in devices.iter().rev() {
            let text = if *ok || !self.scanning {
                label.to_string()
            } else {
                format!("{label} searching")
            };
            let g = ui
                .painter()
                .layout_no_wrap(text, font.clone(), Color32::from_white_alpha(219));
            let w = g.size().x;
            let left = right - w - 13.0;
            let dot = if *ok {
                Color32::from_rgb(110, 231, 160)
            } else if self.scanning {
                Color32::from_rgb(255, 209, 102)
            } else {
                Color32::from_gray(120)
            };
            ui.painter()
                .circle_filled(Pos2::new(left + 3.5, mid), 3.5, dot);
            let pos = Pos2::new(left + 13.0, mid - g.size().y / 2.0);
            area = area.union(Rect::from_min_size(pos, g.size()));
            area = area.union(Rect::from_center_size(
                Pos2::new(left + 3.5, mid),
                Vec2::splat(8.0),
            ));
            ui.painter().galley(pos, g, Color32::WHITE);
            right = left - 14.0;
        }
        if ui
            .interact(
                area.expand(6.0),
                ui.scope_id().with("home-bar-devices"),
                Sense::click(),
            )
            .on_hover_text("Open Devices")
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            self.tab = Tab::Devices;
        }
    }
}

/// "1:05" for an hour and five minutes.
fn hours_minutes(secs: f32) -> String {
    let m = (secs / 60.0).round() as u32;
    format!("{}:{:02}", m / 60, m % 60)
}

/// A workout's shape as blocks in zone colours, ramps sloping.
fn zone_blocks(p: &egui::Painter, rect: Rect, segs: &[crate::workout::Segment]) {
    let total: f32 = segs.iter().map(|s| s.secs).sum::<f32>().max(1.0);
    let top = 1.3;
    let height = |f: f32| rect.height() * (f / top).clamp(0.12, 1.0);
    let mut x = rect.left();
    for s in segs {
        let w = rect.width() * s.secs / total;
        let (from, to) = if s.free { (0.6, 0.6) } else { (s.from, s.to) };
        let colour = if s.free {
            Color32::from_gray(150)
        } else {
            charts::zone_color((from + to) / 2.0)
        };
        let gap = if w > 3.0 { 1.0 } else { 0.0 };
        p.add(egui::Shape::convex_polygon(
            vec![
                Pos2::new(x, rect.bottom()),
                Pos2::new(x + w - gap, rect.bottom()),
                Pos2::new(x + w - gap, rect.bottom() - height(to)),
                Pos2::new(x, rect.bottom() - height(from)),
            ],
            colour.gamma_multiply(0.92),
            Stroke::NONE,
        ));
        x += w;
    }
}

/// Five little bars for a row's tile: the workout's intensity across its
/// length.
fn mini_bars(p: &egui::Painter, rect: Rect, segs: &[crate::workout::Segment]) {
    let total: f32 = segs.iter().map(|s| s.secs).sum::<f32>().max(1.0);
    let n = 5;
    let w = (rect.width() - (n - 1) as f32) / n as f32;
    for i in 0..n {
        let t = total * (i as f32 + 0.5) / n as f32;
        let f = segs
            .iter()
            .find(|s| t >= s.start && t < s.start + s.secs)
            .map(|s| if s.free { 0.6 } else { (s.from + s.to) / 2.0 })
            .unwrap_or(0.5);
        let h = rect.height() * (f / 1.3).clamp(0.15, 1.0);
        let x = rect.left() + i as f32 * (w + 1.0);
        p.rect_filled(
            Rect::from_min_max(
                Pos2::new(x, rect.bottom() - h),
                Pos2::new(x + w, rect.bottom()),
            ),
            CornerRadius::same(1),
            charts::zone_color(f),
        );
    }
}

/// A round white button with a play sign.
fn play_button(ui: &mut egui::Ui, size: f32, label: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let resp = resp
        .on_hover_text(label)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    paint_play(ui.painter(), rect, resp.hovered());
    resp
}

fn paint_play(p: &egui::Painter, rect: Rect, hovered: bool) {
    let fill = if hovered {
        Color32::WHITE
    } else {
        Color32::from_white_alpha(235)
    };
    p.circle_filled(rect.center(), rect.width() / 2.0, fill);
    let s = rect.width() * 0.16;
    let c = rect.center() + Vec2::new(s * 0.2, 0.0);
    p.add(egui::Shape::convex_polygon(
        vec![
            c + Vec2::new(-s * 0.8, -s),
            c + Vec2::new(s, 0.0),
            c + Vec2::new(-s * 0.8, s),
        ],
        Color32::from_rgb(28, 29, 33),
        Stroke::NONE,
    ));
}

#[derive(Clone, Copy)]
enum SideIcon {
    Search,
    FreeRide,
    Workouts,
    Routes,
    Random,
    Plus,
}

/// A row in Home's sidebar: an icon and a word, lit under the pointer.
fn side_row(ui: &mut egui::Ui, icon: SideIcon, text: &str) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(12), Color32::from_white_alpha(26));
    }
    side_icon(p, rect, icon);
    p.text(
        Pos2::new(rect.left() + 42.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        theme::regular(13.5),
        Color32::from_white_alpha(219),
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The sidebar's icons, in the menu's thin-line style.
fn side_icon(p: &egui::Painter, row: Rect, icon: SideIcon) {
    let c = Pos2::new(row.left() + 21.0, row.center().y);
    let colour = Color32::from_white_alpha(219);
    let s = Stroke::new(1.6, colour);
    let at = |x: f32, y: f32| c + Vec2::new(x, y);
    let line = |pts: &[(f32, f32)]| {
        p.add(egui::Shape::line(
            pts.iter().map(|&(x, y)| at(x, y)).collect(),
            s,
        ));
    };
    match icon {
        SideIcon::Search => {
            p.circle_stroke(at(-1.5, -1.5), 5.5, s);
            line(&[(2.5, 2.5), (7.0, 7.0)]);
        }
        SideIcon::FreeRide => super::menu::menu_icon(p, c, Tab::Ride, colour),
        SideIcon::Workouts => super::menu::menu_icon(p, c, Tab::Library, colour),
        SideIcon::Routes => {
            p.circle_stroke(at(-5.0, 5.5), 2.0, s);
            p.circle_stroke(at(5.0, -5.5), 2.0, s);
            line(&[
                (-3.0, 5.5),
                (2.0, 5.5),
                (4.0, 3.0),
                (2.0, 0.5),
                (-2.0, -0.5),
                (-4.0, -3.0),
                (-2.0, -5.5),
                (3.0, -5.5),
            ]);
        }
        SideIcon::Random => {
            line(&[(-8.0, -4.0), (-3.0, -4.0), (3.0, 4.0), (8.0, 4.0)]);
            line(&[(-8.0, 4.0), (-3.0, 4.0), (3.0, -4.0), (8.0, -4.0)]);
            line(&[(5.5, -6.5), (8.0, -4.0), (5.5, -1.5)]);
            line(&[(5.5, 1.5), (8.0, 4.0), (5.5, 6.5)]);
        }
        SideIcon::Plus => {
            p.circle_filled(c, 14.0, Color32::from_white_alpha(41));
            line(&[(-4.5, 0.0), (4.5, 0.0)]);
            line(&[(0.0, -4.5), (0.0, 4.5)]);
        }
    }
}
