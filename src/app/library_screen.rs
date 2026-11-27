//! Library: everything you can ride. Workouts (the workouts screen), and
//! Routes: the sceneries down the side, the chosen route with its profile
//! and your best there, and all the routes.

use super::glass::{self, Glyph, Lead};
use super::*;
use crate::route::Scenery;

/// The two halves of the Library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LibraryView {
    Workouts,
    Routes,
}

impl App {
    pub(super) fn library_tab(&mut self, ui: &mut egui::Ui) {
        match self.library_view {
            LibraryView::Workouts => {
                ui.horizontal(|ui| {
                    self.library_switch(ui);
                });
                ui.add_space(10.0);
                self.workouts_tab(ui);
            }
            LibraryView::Routes => {
                // The pane's own padding is replaced by the sidebar layout.
                let pane = ui.max_rect().expand2(Vec2::new(28.0, 26.0));
                glass::area(ui, pane, "routes", |ui| {
                    let (side, main) = glass::split(ui);
                    if let Some(side) = side {
                        glass::area(ui, side, "routes-side", |ui| self.routes_sidebar(ui));
                    }
                    glass::area(ui, main, "routes-main", |ui| self.routes_main(ui));
                });
            }
        }
    }

    /// Workouts or Routes.
    fn library_switch(&mut self, ui: &mut egui::Ui) {
        if glass::pill(ui, "Workouts", self.library_view == LibraryView::Workouts).clicked() {
            self.library_view = LibraryView::Workouts;
        }
        if glass::pill(ui, "Routes", self.library_view == LibraryView::Routes).clicked() {
            self.library_view = LibraryView::Routes;
        }
    }

    fn routes_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.horizontal(|ui| self.library_switch(ui));
        glass::side_heading(ui, "Sceneries");
        let counts = |s: Option<Scenery>| {
            self.routes
                .iter()
                .filter(|r| !r.is_reversed() && s.is_none_or(|s| r.scenery == s))
                .count()
        };
        let all = counts(None);
        if glass::side_row(
            ui,
            self.library_scenery.is_none(),
            Lead::Icon(Glyph::Tiles),
            "All sceneries",
            &format!("{all} routes"),
        )
        .clicked()
        {
            self.library_scenery = None;
        }
        for s in Scenery::ALL {
            let n = counts(Some(s));
            if n == 0 {
                continue;
            }
            if glass::side_row(
                ui,
                self.library_scenery == Some(s),
                Lead::Icon(scenery_glyph(s)),
                s.label(),
                &format!("{n} route{}", if n == 1 { "" } else { "s" }),
            )
            .clicked()
            {
                self.library_scenery = Some(s);
            }
        }
        let left = ui.available_height() - 88.0;
        if left > 0.0 {
            ui.add_space(left);
        }
        if glass::side_row(ui, false, Lead::Icon(Glyph::Random), "New random route", "").clicked() {
            if self.ride_active() {
                self.notify("Finish the current ride first");
            } else {
                self.new_random_route();
                self.library_route = Some(self.route_idx);
            }
        }
        if glass::side_row(
            ui,
            false,
            Lead::Icon(Glyph::Folder),
            "Import a GPX file…",
            "",
        )
        .clicked()
        {
            self.pick_file("Import a route", &[("GPX route", &["gpx"])]);
        }
    }

    fn routes_main(&mut self, ui: &mut egui::Ui) {
        let units = self.store.settings.units;
        let shown: Vec<usize> = (0..self.routes.len())
            .filter(|&i| {
                let r = &self.routes[i];
                !r.is_reversed() && self.library_scenery.is_none_or(|s| r.scenery == s)
            })
            .collect();
        let pick = self
            .library_route
            .filter(|i| shown.contains(i))
            .or_else(|| shown.contains(&self.route_idx).then_some(self.route_idx))
            .or_else(|| shown.first().copied());
        ui.horizontal(|ui| {
            glass::title(ui, "Routes");
        });
        ui.add_space(12.0);
        let mut ride = None;
        if let Some(i) = pick {
            let r = &self.routes[i];
            let best = crate::segment::best(&self.store.efforts, r.base_id(), "lap");
            let mut max_grade: f64 = 0.0;
            let mut d = 0.0;
            while d < r.length {
                max_grade = max_grade.max(r.grade(d));
                d += 25.0;
            }
            glass::card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(r.scenery.label())
                                .font(theme::regular(13.0))
                                .color(glass::QUIET),
                        );
                        ui.label(
                            RichText::new(
                                units.route_name(r.name.split(" · ").next().unwrap_or(&r.name)),
                            )
                            .font(theme::semibold(22.0))
                            .color(Color32::WHITE),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if glass::pill_white(ui, "Ride this route").clicked() {
                            ride = Some(r.base_id().to_string());
                        }
                    });
                });
                ui.add_space(10.0);
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 150.0), Sense::hover());
                charts::elevation_profile(ui.painter(), rect, r, None, None, units);
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 36.0;
                    let (dist, unit) = units
                        .fmt_distance(r.length, 1)
                        .rsplit_once(' ')
                        .map(|(a, b)| (a.to_string(), b.to_string()))
                        .unwrap_or_default();
                    glass::figure(ui, "Distance", &dist, &unit, 24.0);
                    let climb = units.fmt_height(r.ascent);
                    let (cv, cu) = climb
                        .rsplit_once(' ')
                        .map(|(a, b)| (a.to_string(), b.to_string()))
                        .unwrap_or((climb.clone(), String::new()));
                    glass::figure(ui, "Climbing", &cv, &cu, 24.0);
                    glass::figure(ui, "Steepest", &format!("{max_grade:.0}"), "%", 24.0);
                    glass::figure(
                        ui,
                        "Shape",
                        if r.is_loop() { "Loop" } else { "A to B" },
                        "",
                        24.0,
                    );
                    match best {
                        Some(e) => glass::figure(
                            ui,
                            "Your best here",
                            &fmt_dur(e.secs),
                            &format!("{:.0} W · {}", e.avg_power, e.date.format("%-d %b")),
                            24.0,
                        ),
                        None => glass::figure(ui, "Your best here", "—", "ride a lap", 24.0),
                    }
                });
            });
        }
        ui.add_space(18.0);
        glass::heading(ui, "All routes");
        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .id_salt("all-routes")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for &i in &shown {
                    let r = &self.routes[i];
                    let meta = format!(
                        "{} · {} climbing · {}",
                        units.fmt_distance(r.length, 1),
                        units.fmt_height(r.ascent),
                        r.scenery.label()
                    );
                    let name = units.route_name(r.name.split(" · ").next().unwrap_or(&r.name));
                    let icon = if r.is_loop() {
                        Glyph::Loop
                    } else {
                        Glyph::Route
                    };
                    if glass::side_row(ui, Some(i) == pick, Lead::Icon(icon), &name, &meta)
                        .clicked()
                    {
                        self.library_route = Some(i);
                    }
                }
                ui.add_space(18.0);
                egui::CollapsingHeader::new(
                    RichText::new("Random routes and imported routes").color(glass::SOFT),
                )
                .id_salt("routes-manage")
                .show(ui, |ui| self.settings_routes(ui));
            });
        if let Some(id) = ride {
            if self.ride_active() {
                self.notify("Finish the current ride first");
            } else {
                self.choose_route(id);
                self.tab = Tab::Ride;
            }
        }
    }
}

/// Each scenery's icon in the Library.
fn scenery_glyph(s: Scenery) -> Glyph {
    match s {
        Scenery::Hills => Glyph::Barn,
        Scenery::Lake => Glyph::Lakeside,
        Scenery::Alpine => Glyph::Peaks,
        Scenery::Coast => Glyph::Coast,
        Scenery::Japan => Glyph::Torii,
    }
}
