//! The ride screen: the 3D view's menus and banners, the route profile,
//! the riders drawn in the world, and the live dashboard.

use super::*;

impl App {
    /// "You missed N workouts" with options to reschedule or skip them.
    pub(super) fn missed_banner(&mut self, ui: &mut egui::Ui) {
        let today = Local::now().date_naive();
        let lib = self.library();
        let missed = plan::missed(&self.store.plan, today, |id| {
            lib.iter().find(|w| w.id == id).map(|w| w.category)
        });
        if missed.is_empty() {
            return;
        }
        let name = |i: usize| {
            let p = &self.store.plan[i];
            let n = lib
                .iter()
                .find(|w| w.id == p.workout_id)
                .map(|w| w.name.clone())
                .unwrap_or_default();
            format!("{n} ({})", p.date.format("%a"))
        };
        let list: Vec<String> = missed.iter().map(|m| name(m.index)).collect();
        let moves: Vec<String> = missed
            .iter()
            .filter_map(|m| {
                m.move_to
                    .map(|d| format!("{} → {}", name(m.index), d.format("%a %d %b")))
            })
            .collect();
        let mut action = None;
        egui::Frame::new()
            .fill(Color32::from_rgb(52, 38, 30))
            .corner_radius(8)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(format!("You missed {}: {}", if list.len() == 1 { "a planned workout".into() } else { format!("{} planned workouts", list.len()) }, list.join(", "))).strong());
                if !moves.is_empty() {
                    ui.label(RichText::new(format!("Rescheduling moves the hard sessions to free days ({}) and skips the easy rides.", moves.join(", "))).weak());
                }
                ui.horizontal(|ui| {
                    if !moves.is_empty() && ui.add(egui::Button::new("Reschedule").fill(ACCENT)).clicked() {
                        action = Some(true);
                    }
                    if ui.button(if moves.is_empty() { "Mark as skipped" } else { "Skip them all" }).clicked() {
                        action = Some(false);
                    }
                });
            });
        ui.add_space(6.0);
        if let Some(reschedule) = action {
            for m in missed {
                match (reschedule, m.move_to) {
                    (true, Some(d)) => self.store.plan[m.index].date = d,
                    _ => self.store.plan[m.index].skipped = true,
                }
            }
            self.store.plan.sort_by_key(|p| p.date);
            self.mark_dirty();
        }
    }

    /// Suggest an easier ride when today's planned workout is hard and form
    /// says you are very tired.
    pub(super) fn tired_banner(&mut self, ui: &mut egui::Ui, floating: bool) {
        let today = Local::now().date_naive();
        if self.tired_dismissed == Some(today) {
            return;
        }
        let Some(i) = self
            .store
            .plan
            .iter()
            .position(|p| p.date == today && !p.done && !p.skipped)
        else {
            return;
        };
        let id = self.store.plan[i].workout_id.clone();
        let Some(cat) = self.workout_category(&id) else {
            return;
        };
        let (_, _, form) = crate::progress::fitness(&self.store.history, today, 1).today();
        let Some(easier) = plan::easier_option(cat, form) else {
            return;
        };
        let (Some(hard), Some(easy)) = (self.find_workout(&id), self.find_workout(easier)) else {
            return;
        };
        let mut action = None;
        let content = |ui: &mut egui::Ui, action: &mut Option<bool>| {
            ui.label(
                RichText::new(format!("You look very tired today (form {form:+.0}).")).strong(),
            );
            ui.label(format!(
                "Consider swapping {} for {} and saving the hard session for when you're fresher.",
                hard.name, easy.name
            ));
            ui.horizontal(|ui| {
                if ui
                    .add(egui::Button::new(format!("Ride {} instead", easy.name)).fill(ACCENT))
                    .clicked()
                {
                    *action = Some(true);
                }
                if ui.button("Keep the plan").clicked() {
                    *action = Some(false);
                }
            });
        };
        if floating {
            egui::Area::new(egui::Id::unique("tired"))
                .anchor(Align2::CENTER_TOP, [0.0, 235.0])
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style())
                        .inner_margin(12)
                        .show(ui, |ui| content(ui, &mut action));
                });
        } else {
            egui::Frame::new()
                .fill(Color32::from_rgb(40, 38, 54))
                .corner_radius(8)
                .inner_margin(10)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    content(ui, &mut action);
                });
            ui.add_space(6.0);
        }
        match action {
            Some(true) => {
                self.store.plan[i].workout_id = easier.to_string();
                self.mark_dirty();
                if !self.ride_active() {
                    self.load_ride(Some(easier.to_string()), Some(today));
                }
                self.notify(format!("Swapped for {}", easy.name));
            }
            Some(false) => self.tired_dismissed = Some(today),
            None => {}
        }
    }

    pub(super) fn ride_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let mut v = self.store.settings.view_3d;
            if segmented(
                ui,
                &mut v,
                &[
                    (true, "3D", "The 3D world (V switches)"),
                    (false, "Dashboard", "Numbers and charts (V switches)"),
                ],
            ) {
                self.store.settings.view_3d = v;
                self.mark_dirty();
            }
            if self.store.settings.view_3d {
                self.sky_menu(ui);
            }
            // Once you are riding, what and where is settled: the choices
            // make way for the controls you need on the bike.
            let active = self.ride_active();
            if !active {
                let mut mode = self.ride.mode;
                if segmented(
                    ui,
                    &mut mode,
                    &[
                        (
                            Mode::Workout,
                            "Workout",
                            "Structured workout: the trainer holds the target power (ERG)",
                        ),
                        (
                            Mode::Route,
                            "Free ride",
                            "Resistance follows the road's gradient",
                        ),
                        (Mode::Slope, "Slope", "Choose a fixed gradient yourself"),
                        (
                            Mode::HeartRate,
                            "Heart rate",
                            "Hold a target heart rate: VeloForge adjusts the trainer's power for you",
                        ),
                    ],
                ) {
                    let w = if mode == Mode::Workout {
                        self.ride_workout
                            .clone()
                            .or(Some("endurance-60".into()))
                            .and_then(|id| self.find_workout(&id))
                    } else {
                        None
                    };
                    self.ride_workout = w
                        .as_ref()
                        .map(|w| w.id.clone())
                        .or(self.ride_workout.clone());
                    self.replace_ride(Ride::new(mode, w, None));
                    if mode == Mode::HeartRate {
                        // Default to the middle of zone 2.
                        self.ride.hr_control =
                            ride::HrControl::new((self.store.settings.max_hr * 0.65).round());
                    }
                }
                if self.ride.mode == Mode::Workout {
                    let current = self
                        .ride
                        .workout
                        .as_ref()
                        .map(|w| w.name.clone())
                        .unwrap_or("Choose workout".into());
                    let mut pick = None;
                    egui::ComboBox::from_id_salt("ride-workout")
                        .width(150.0)
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            for w in self.library() {
                                let label = format!("{}  ·  {}", w.name, fmt_dur(w.duration()));
                                if ui
                                    .selectable_label(
                                        self.ride_workout.as_deref() == Some(&w.id),
                                        label,
                                    )
                                    .clicked()
                                {
                                    pick = Some(w.id.clone());
                                }
                            }
                        });
                    if let Some(id) = pick {
                        self.load_ride(Some(id), None);
                    }
                }
                let mut ri = self.route_idx;
                let units = self.store.settings.units;
                let mut random = false;
                egui::ComboBox::from_id_salt("ride-route")
                    .width(160.0)
                    .selected_text(units.route_name(&self.routes[ri].name))
                    .show_ui(ui, |ui| {
                        for (i, r) in self.routes.iter().enumerate() {
                            ui.selectable_value(&mut ri, i, units.route_name(&r.name));
                        }
                        ui.separator();
                        let r = &self.store.settings.random_route;
                        random = ui
                            .selectable_label(false, "+ New random route")
                            .on_hover_text(format!(
                                "{}, {} km, {} (change these in Settings → Routes)",
                                r.scenery.label(),
                                r.km,
                                r.hills.label().to_lowercase()
                            ))
                            .clicked();
                    });
                if random {
                    self.new_random_route();
                    ri = self.route_idx;
                }
                if ri != self.route_idx {
                    // Another route is ridden the way it is made.
                    self.choose_route(self.routes[ri].base_id().to_string());
                }
                let current = &self.routes[self.route_idx];
                let mut reversed = current.is_reversed();
                let turn = ui
                    .toggle_value(&mut reversed, "Reverse")
                    .on_hover_text(if current.shape() == route::Shape::Loop {
                        "Ride the loop the other way round"
                    } else {
                        "Ride from the finish back to the start"
                    });
                if turn.changed() {
                    let base = current.base_id().to_string();
                    self.choose_route(if reversed {
                        format!("{base}{}", route::REVERSED)
                    } else {
                        base
                    });
                }
            }
            self.companion_menu(ui);
            match self.ride.state {
                ride::State::Ready => {
                    let can = self.can_start();
                    let resp = ui
                        .add_enabled_ui(can, |ui| {
                            icon_button(
                                ui,
                                Icon::Play,
                                "Start ride",
                                Some(Color32::from_rgb(36, 150, 84)),
                            )
                        })
                        .inner;
                    if resp.on_hover_text("Space").clicked() {
                        self.ride.start();
                    }
                }
                ride::State::Running | ride::State::Paused => {
                    let running = self.ride.state == ride::State::Running;
                    let (icon, label) = if running {
                        (Icon::Pause, "Pause")
                    } else {
                        (Icon::Play, "Resume")
                    };
                    if icon_button(ui, icon, label, None)
                        .on_hover_text("Space")
                        .clicked()
                    {
                        if running {
                            self.ride.state = ride::State::Paused;
                        } else {
                            self.ride.start();
                        }
                    }
                    let finish = ui
                        .add(
                            egui::Button::new(
                                RichText::new("Finish").color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(44, 112, 196)),
                        )
                        .on_hover_text(if self.ride_simulated || self.demo {
                            "End the ride. Rides with the simulated trainer are practice: they \
                             are not saved."
                        } else {
                            "End and save the ride"
                        });
                    if finish.clicked() {
                        self.finish_ride();
                    }
                    if ui.button("Discard").clicked() {
                        self.modal = Some(Modal::ConfirmDiscard);
                    }
                }
            }
            match self.ride.mode {
                Mode::Workout => {
                    if self.ride_active()
                        && !self.ride.workout_done()
                        && icon_button(ui, Icon::Right, "Skip step", None)
                            .on_hover_text("Jump to the next step (N)")
                            .clicked()
                    {
                        self.ride.skip_step();
                    }
                    if ui
                        .small_button(" - ")
                        .on_hover_text("Easier (Down arrow key)")
                        .clicked()
                    {
                        self.ride.intensity = (self.ride.intensity - 0.05).max(0.5);
                    }
                    ui.label(format!("{:.0}%", self.ride.intensity * 100.0))
                        .on_hover_text("Workout intensity: scales every target");
                    if ui
                        .small_button(" + ")
                        .on_hover_text("Harder (Up arrow key)")
                        .clicked()
                    {
                        self.ride.intensity = (self.ride.intensity + 0.05).min(1.5);
                    }
                }
                Mode::Slope => {
                    ui.label(RichText::new("Slope").color(theme::TEXT_DIM));
                    ui.add(
                        egui::Slider::new(&mut self.ride.manual_grade, -5.0..=15.0)
                            .step_by(0.5)
                            .suffix("%"),
                    );
                }
                Mode::HeartRate => {
                    ui.label(RichText::new("Target").color(theme::TEXT_DIM));
                    let zones = self.store.settings.hr_zones();
                    ui.add(
                        egui::DragValue::new(&mut self.ride.hr_control.target_bpm)
                            .range(80.0..=200.0)
                            .suffix(" bpm"),
                    )
                    .on_hover_text("Up/Down arrow keys change it by 2 bpm");
                    let z = zones.zone(self.ride.hr_control.target_bpm);
                    ui.label(RichText::new(charts::HR_ZONE_NAMES[z]).color(charts::hr_zone_color(z)));
                }
                Mode::Route => {
                    ui.label(RichText::new("Difficulty").color(theme::TEXT_DIM));
                    if ui
                        .add(
                            egui::Slider::new(&mut self.store.settings.difficulty, 0.0..=1.0)
                                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                        )
                        .on_hover_text(
                            "How much of the road's gradient the trainer gives you: 100% feels \
                             like the real hill, 50% like half of it, 0% like flat road. It only \
                             changes anything on climbs and descents. Up/Down arrow keys change it.",
                        )
                        .changed()
                    {
                        self.mark_dirty();
                    }
                }
            }
            ui.label(RichText::new("Keys").color(theme::TEXT_FAINT).underline())
                .on_hover_text(
                    "Space: start / pause / resume\n\
                     Up / Down: workout intensity, slope, or trainer difficulty\n\
                     N: skip to the next workout step\n\
                     V: switch between 3D view and dashboard\n\
                     C: change the camera in the 3D view",
                );
        });
        ui.add_space(2.0);

        // Never draw past the window edge, even if a control row overflowed.
        let avail = ui.available_rect_before_wrap().intersect(ui.clip_rect());
        ui.allocate_rect(avail, Sense::hover());
        let painter = ui.painter_at(avail);
        // The time since the last frame, as it really passed: egui's
        // stable_dt is only a guess (a 60th of a second) whenever the app
        // waits for its frame-rate limit, which slowed the 3D world (wind,
        // posture, camera) to half speed at 30 fps.
        let dt = ui.ctx().input(|i| i.unstable_dt as f64).clamp(0.0, 0.1);
        // Without a trainer, a notice on the view leads to Devices.
        let view_3d = self.store.settings.view_3d;
        let chart_h = 104.0;
        let board = Rect::from_min_max(
            avail.min,
            Pos2::new(avail.max.x, avail.max.y - chart_h - 8.0),
        );
        if self.trainer_link().is_none() {
            let banner = ui.interact(
                Self::banner_rect(if view_3d { avail } else { board }, view_3d),
                ui.scope_id().with("trainer-banner"),
                Sense::click(),
            );
            if banner
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                self.tab = Tab::Devices;
            }
        }
        if !view_3d {
            // Numbers above, the workout or the road below.
            let chart_rect =
                Rect::from_min_max(Pos2::new(avail.min.x, avail.max.y - chart_h), avail.max);
            self.dashboard(&painter, board);
            self.ride_profile(&painter, chart_rect, Some(CARD));
            return;
        }

        // The 3D world fills the screen; the HUD floats over it on glass.
        let view = avail;
        if let Some(e) = self.gpu.error().filter(|_| !self.gpu_reported) {
            self.gpu_reported = true;
            self.log_line(format!("3D graphics: {e}"));
            self.notify("Using the simple 3D view (the detailed one is not supported here — see the log in Devices)");
        }
        let mut riders = self.scene_riders();
        let mut dt = dt;
        if let Some(d) = self.still {
            dt = 0.0;
            for r in &mut riders {
                r.d = d;
                r.crank = 0.0;
            }
        }
        let scene = Scene {
            riders,
            route: &self.routes[self.route_idx],
            sky: Sky {
                time: self.store.settings.time_of_day,
                weather: self.store.settings.weather,
            },
            camera: self.store.settings.camera,
            dt,
        };
        let plan = self.hud_plan(view);
        let gpu = self.store.settings.gpu_3d && self.gpu_capable && self.gpu.error().is_none();
        let clip = painter.with_clip_rect(view);
        if gpu {
            let s = &self.store.settings;
            let scale = match s.resolution {
                0 => s.gfx_quality.render_scale(),
                pct => pct as f32 / 100.0,
            };
            self.gpu
                .draw(&clip, view, &scene, s.gfx_quality, scale, &plan.glass());
            if self.bench.is_some() {
                let gpu_ms = self.gpu.gpu_ms();
                let size = (
                    view.width() * ui.ctx().pixels_per_point(),
                    view.height() * ui.ctx().pixels_per_point(),
                );
                self.bench_frame(ui.ctx(), gpu_ms, size, scale);
            }
        } else {
            self.world.draw(&clip, view, &scene);
        }
        self.hud(&painter, view, &plan, gpu && self.gpu.frosts_glass());
        if self.ride.state == ride::State::Ready {
            self.tired_banner(ui, true);
        }
    }

    /// The workout's steps, or the road's elevation where there is no workout.
    pub(super) fn ride_profile(&self, painter: &egui::Painter, rect: Rect, bg: Option<Color32>) {
        if self.ride.mode == Mode::Workout && !self.ride.segs.is_empty() {
            let p: Vec<(f32, f32)> = self
                .ride
                .samples
                .iter()
                .map(|s| (s.elapsed, s.power))
                .collect();
            let progress =
                (self.ride.state != ride::State::Ready).then_some(self.ride.workout_time() as f32);
            charts::workout_profile(
                painter,
                rect,
                &self.ride.segs,
                self.store.settings.ftp,
                self.ride.intensity,
                progress,
                &p,
                bg,
            );
        } else {
            charts::elevation_profile(
                painter,
                rect,
                &self.routes[self.route_idx],
                Some(self.ride.route_pos()),
                bg,
                self.store.settings.units,
            );
        }
    }

    /// You, and whoever you ride with, for the 3D views.
    fn scene_riders(&self) -> Vec<Figure> {
        let [r, g, b] = self.store.settings.jersey;
        // Whoever is behind moves into the other's line to draft: from
        // 15 m back, but never on top of them when level.
        let (mut mine, mut theirs) = (crate::world::LANE, crate::world::COMPANION_LANE);
        if let Some(gap) = self
            .ride
            .companion_gap()
            .filter(|_| self.store.settings.drafting)
        {
            let smooth = |a: f64, b: f64, x: f64| {
                let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            };
            let tuck = smooth(15.0, 7.0, gap.abs()) * smooth(1.8, 3.0, gap.abs());
            if gap > 0.0 {
                mine += (theirs - mine) * tuck;
            } else {
                theirs += (mine - theirs) * tuck;
            }
        }
        let mut riders = vec![Figure {
            d: self.ride.route_pos(),
            lateral: mine,
            crank: self.ride.crank_angle,
            speed: self.ride.speed,
            effort: self.ride.effort,
            pedalling: self.ride.pedalling,
            jersey: Color32::from_rgb(r, g, b),
            tag: None,
        }];
        if let Some(gap) = self.ride.companion_gap() {
            let (jersey, tag) = match &self.ride.companion {
                Some(ride::Companion::Pacer(p)) => {
                    (self.pacer_jersey(), format!("Pacer · {:.0} W", p.watts))
                }
                Some(ride::Companion::Ghost(gh)) => (
                    Color32::from_rgb(200, 232, 255),
                    format!("Ghost · {}", gh.label.split(',').next().unwrap_or("")),
                ),
                None => unreachable!(),
            };
            riders.push(Figure {
                d: self.ride.route_pos() + gap,
                lateral: theirs,
                crank: self.ride.companion_crank,
                speed: self.ride.speed,
                effort: self.ride.companion_effort,
                pedalling: self.ride.companion_pedalling,
                jersey,
                tag: Some(tag),
            });
        }
        riders
    }

    /// Time of day and weather for the 3D world.
    fn sky_menu(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.store.settings;
        let label = if s.weather == Weather::Clear {
            s.time_of_day.label().to_string()
        } else {
            format!("{} · {}", s.time_of_day.label(), s.weather.label())
        };
        let mut changed = false;
        ui.menu_button(format!("{label} ▾"), |ui| {
            ui.set_min_width(170.0);
            ui.label(RichText::new("Time of day").weak());
            for t in TimeOfDay::ALL {
                changed |= ui.radio_value(&mut s.time_of_day, t, t.label()).changed();
            }
            ui.separator();
            ui.label(RichText::new("Weather").weak());
            for w in Weather::ALL {
                changed |= ui.radio_value(&mut s.weather, w, w.label()).changed();
            }
            ui.separator();
            ui.label(RichText::new("Camera (C switches)").weak());
            for c in crate::world::Camera::ALL {
                changed |= ui.radio_value(&mut s.camera, c, c.label()).changed();
            }
        })
        .response
        .on_hover_text("Time of day, weather and camera");
        if changed {
            self.mark_dirty();
        }
    }

    /// "Ride with": nobody, a pacer at a fixed power, or a ghost of an
    /// earlier ride on this route.
    fn companion_menu(&mut self, ui: &mut egui::Ui) {
        let label = match &self.ride.companion {
            None => "Ride alone".to_string(),
            Some(ride::Companion::Pacer(p)) => format!("Pacer {:.0} W", p.watts),
            Some(ride::Companion::Ghost(_)) => "Ghost".to_string(),
        };
        let ghosts: Vec<(String, ride::Summary)> = self
            .ghost_candidates()
            .into_iter()
            .map(|h| (Self::ghost_label(h), h.clone()))
            .collect();
        let ftp = self.store.settings.ftp;
        let mut load = None;
        ui.menu_button(format!("{label} ▾"), |ui| {
            ui.set_min_width(260.0);
            if ui
                .selectable_label(self.ride.companion.is_none(), "Ride alone")
                .clicked()
            {
                self.ride.companion = None;
                ui.close();
            }
            ui.separator();
            let is_pacer = matches!(self.ride.companion, Some(ride::Companion::Pacer(_)));
            if ui
                .selectable_label(is_pacer, "Pacer at a steady power")
                .on_hover_text(
                    "Another rider holding the power you choose, with the same weight and bike",
                )
                .clicked()
                && !is_pacer
            {
                // Join at your position so a mid-ride pacer is not kilometres back.
                self.ride.companion = Some(ride::Companion::Pacer(ride::Pacer {
                    watts: ((ftp * 0.75) / 5.0).round() * 5.0,
                    distance: self.ride.distance,
                    speed: self.ride.speed,
                }));
            }
            if let Some(ride::Companion::Pacer(p)) = &mut self.ride.companion {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.add(
                        egui::DragValue::new(&mut p.watts)
                            .range(50.0..=600.0)
                            .speed(1.0)
                            .suffix(" W"),
                    );
                    ui.label(
                        RichText::new(format!("{:.0}% of FTP", p.watts / ftp.max(1.0) * 100.0))
                            .weak(),
                    );
                });
            }
            ui.separator();
            ui.label(RichText::new("Race a ghost of an earlier ride on this route").weak());
            if ghosts.is_empty() {
                ui.label(
                    RichText::new("Ride this route once (at least a kilometre) to unlock")
                        .weak()
                        .italics(),
                );
            }
            for (label, h) in &ghosts {
                let selected = matches!(&self.ride.companion,
                    Some(ride::Companion::Ghost(g)) if &g.label == label);
                let text = format!(
                    "{label}  ·  {}  ·  {}",
                    self.store
                        .settings
                        .units
                        .fmt_speed(h.distance_m / h.secs.max(1.0) as f64 * 3.6),
                    fmt_dur(h.secs)
                );
                if ui.selectable_label(selected, text).clicked() {
                    load = Some(h.clone());
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Ride with a pacer or race your ghost");
        if let Some(h) = load {
            self.load_ghost(&h);
        }
    }

    /// A cadence source has reported recently.
    pub(super) fn has_cadence(&self) -> bool {
        self.metrics
            .cadence_seen
            .is_some_and(|t| t.elapsed().as_secs() < 30)
    }

    fn speed_label(&self) -> &'static str {
        if self.ride.speed_from_trainer {
            "SPEED (trainer)"
        } else {
            "SPEED (virtual)"
        }
    }

    fn source_name(&self, m: &Option<Metric>) -> Option<String> {
        let m = m.as_ref().filter(|m| m.is_fresh())?;
        Some(
            self.links
                .get(&m.src)
                .map(|l| l.name.clone())
                .unwrap_or_else(|| m.src.clone()),
        )
    }

    /// Numbers-first ride screen, an alternative to the 3D view.
    fn dashboard(&self, painter: &egui::Painter, rect: Rect) {
        let s = &self.store.settings;
        let ftp = s.ftp;
        painter.rect_filled(rect, 0.0, theme::BG);
        let r = rect.shrink(12.0);
        let gap = 10.0;
        let h1 = r.height() * 0.28;
        let h2 = r.height() * 0.19;
        let h3 = r.height() * 0.13;
        let row = |y: f32, h: f32, n: usize| -> Vec<Rect> {
            let w = (r.width() - gap * (n - 1) as f32) / n as f32;
            (0..n)
                .map(|i| {
                    Rect::from_min_size(
                        Pos2::new(r.left() + i as f32 * (w + gap), y),
                        Vec2::new(w, h),
                    )
                })
                .collect()
        };
        let sub = |rect: Rect, text: String, c: Color32| {
            painter.text(
                rect.right_top() + Vec2::new(-12.0, 8.0),
                Align2::RIGHT_TOP,
                text,
                FontId::proportional(13.0),
                c,
            );
        };
        let dash = || "—".to_string();
        let grey = Color32::from_gray(150);

        // Row 1: the big four.
        let power = self.metrics.power3s();
        let target = self.ride.target_watts(ftp);
        let grade = self.routes[self.route_idx].grade(self.ride.route_pos());
        let tiles = row(r.top(), h1, 4);
        let pcol = power
            .map(|p| charts::zone_color(p / ftp))
            .unwrap_or(Color32::GRAY);
        tile(
            painter,
            tiles[0],
            "POWER (3s)",
            &power.map(|p| format!("{p:.0}")).unwrap_or_else(dash),
            "W",
            pcol,
        );
        if let Some(p) = power {
            sub(
                tiles[0],
                format!(
                    "{} · {:.1} W/kg",
                    charts::zone_name(p / ftp),
                    p / s.rider_kg.max(1.0)
                ),
                pcol,
            );
        }
        if let Some(t) = target {
            tile(
                painter,
                tiles[1],
                "TARGET",
                &format!("{t:.0}"),
                "W",
                charts::zone_color(t / ftp),
            );
            sub(tiles[1], format!("{:.0}% FTP", t / ftp * 100.0), grey);
        } else {
            let g = if self.ride.mode == Mode::Slope {
                self.ride.manual_grade as f64
            } else {
                grade
            };
            tile(painter, tiles[1], "GRADE", &format!("{g:+.1}"), "%", grey);
        }
        let cadence_tile = self.has_cadence();
        if cadence_tile {
            tile(
                painter,
                tiles[2],
                "CADENCE",
                &fresh(&self.metrics.cadence)
                    .map(|c| format!("{c:.0}"))
                    .unwrap_or_else(dash),
                "rpm",
                Color32::from_rgb(90, 170, 230),
            );
            if let Some(t) = self.ride.target_cadence() {
                let live = fresh(&self.metrics.cadence);
                sub(
                    tiles[2],
                    format!("target {t:.0} rpm"),
                    charts::cadence_colour(live, t),
                );
            } else if let Some(src) = self.source_name(&self.metrics.cadence) {
                sub(tiles[2], src, grey);
            }
        } else {
            tile(
                painter,
                tiles[2],
                self.speed_label(),
                &format!(
                    "{:.1}",
                    self.store.settings.units.speed(self.ride.speed * 3.6)
                ),
                self.store.settings.units.speed_unit(),
                Color32::from_rgb(90, 170, 230),
            );
        }
        let hr = fresh(&self.metrics.hr);
        let hr_col = hr
            .map(|h| charts::hr_zone_color(s.hr_zones().zone(h)))
            .unwrap_or(Color32::from_rgb(230, 70, 80));
        tile(
            painter,
            tiles[3],
            "HEART RATE",
            &hr.map(|c| format!("{c:.0}")).unwrap_or_else(dash),
            "bpm",
            hr_col,
        );
        if let Some(h) = hr {
            let z = s.hr_zones().zone(h);
            let share = match s.hr_basis {
                crate::store::HrBasis::Threshold if s.lthr > 0.0 => {
                    format!("{:.0}% threshold", h / s.lthr * 100.0)
                }
                _ => format!("{:.0}% max", h / s.max_hr * 100.0),
            };
            sub(
                tiles[3],
                format!("{} · {share}", charts::HR_ZONE_NAMES[z]),
                hr_col,
            );
        }

        // Row 2: interval / ride status.
        let y2 = r.top() + h1 + gap;
        let panel = Rect::from_min_size(Pos2::new(r.left(), y2), Vec2::new(r.width(), h2));
        painter.rect_filled(panel, CornerRadius::same(8), theme::CARD);
        let big = FontId::proportional((h2 * 0.36).min(56.0));
        let small = FontId::proportional(14.0);
        let t = self.ride.workout_time() as f32;
        let left = panel.left_center() + Vec2::new(18.0, 0.0);
        let mid = panel.center();
        let right = panel.right_center() - Vec2::new(18.0, 0.0);
        let bar = |frac: f32, col: Color32| {
            let b = Rect::from_min_size(
                Pos2::new(panel.left() + 18.0, panel.bottom() - 16.0),
                Vec2::new(panel.width() - 36.0, 6.0),
            );
            painter.rect_filled(b, 3.0, Color32::from_white_alpha(25));
            painter.rect_filled(
                Rect::from_min_size(
                    b.min,
                    Vec2::new(b.width() * frac.clamp(0.0, 1.0), b.height()),
                ),
                3.0,
                col,
            );
        };
        match (&self.ride.workout, self.ride.mode) {
            (Some(w), Mode::Workout) if self.ride.workout_done() => {
                painter.text(
                    mid,
                    Align2::CENTER_CENTER,
                    "Workout complete!",
                    big,
                    Color32::from_rgb(120, 220, 140),
                );
                painter.text(
                    mid + Vec2::new(0.0, h2 * 0.32),
                    Align2::CENTER_CENTER,
                    if self.keep_riding {
                        format!("{} — keep spinning to cool down, then press Finish", w.name)
                    } else {
                        format!("{} — well done", w.name)
                    },
                    small.clone(),
                    Color32::from_gray(200),
                );
            }
            (Some(w), Mode::Workout) => {
                if let Some(i) = workout::segment_index(&self.ride.segs, t) {
                    let seg = self.ride.segs[i];
                    let now = seg.with_cadence(
                        if seg.free {
                            "Free ride".to_string()
                        } else {
                            format!("{:.0} W", seg.at(t) * self.ride.intensity * ftp)
                        },
                        "  ·  ",
                    );
                    painter.text(
                        left - Vec2::new(0.0, h2 * 0.22),
                        Align2::LEFT_CENTER,
                        format!("{}  ·  interval {}/{}", w.name, i + 1, self.ride.segs.len()),
                        small.clone(),
                        Color32::from_gray(190),
                    );
                    painter.text(
                        left + Vec2::new(0.0, h2 * 0.08),
                        Align2::LEFT_CENTER,
                        now,
                        big.clone(),
                        Color32::WHITE,
                    );
                    painter.text(
                        mid - Vec2::new(0.0, h2 * 0.22),
                        Align2::CENTER_CENTER,
                        "INTERVAL TIME LEFT",
                        small.clone(),
                        Color32::from_gray(160),
                    );
                    painter.text(
                        mid + Vec2::new(0.0, h2 * 0.08),
                        Align2::CENTER_CENTER,
                        fmt_dur((seg.end() - t).ceil()),
                        big.clone(),
                        Color32::WHITE,
                    );
                    let next = self.ride.segs.get(i + 1).map(|n| {
                        n.with_cadence(
                            if n.free {
                                format!("Next: {} free ride", fmt_dur(n.secs))
                            } else {
                                format!(
                                    "Next: {} @ {:.0} W",
                                    fmt_dur(n.secs),
                                    n.from * self.ride.intensity * ftp
                                )
                            },
                            ", ",
                        )
                    });
                    painter.text(
                        right - Vec2::new(0.0, h2 * 0.18),
                        Align2::RIGHT_CENTER,
                        next.unwrap_or_else(|| "Last interval".into()),
                        small.clone(),
                        Color32::from_gray(210),
                    );
                    painter.text(
                        right + Vec2::new(0.0, h2 * 0.1),
                        Align2::RIGHT_CENTER,
                        format!("Workout left {}", fmt_dur(w.duration() - t)),
                        small.clone(),
                        Color32::from_gray(160),
                    );
                    let col = if seg.free {
                        Color32::GRAY
                    } else {
                        charts::zone_color(seg.at(t))
                    };
                    bar((t - seg.start) / seg.secs, col);
                }
            }
            (_, Mode::HeartRate) => {
                let c = self.ride.hr_control;
                let hr = fresh(&self.metrics.hr);
                painter.text(
                    left - Vec2::new(0.0, h2 * 0.22),
                    Align2::LEFT_CENTER,
                    "TARGET HEART RATE",
                    small.clone(),
                    Color32::from_gray(190),
                );
                painter.text(
                    left + Vec2::new(0.0, h2 * 0.08),
                    Align2::LEFT_CENTER,
                    format!("{:.0} bpm", c.target_bpm),
                    big.clone(),
                    Color32::WHITE,
                );
                painter.text(
                    mid - Vec2::new(0.0, h2 * 0.22),
                    Align2::CENTER_CENTER,
                    "POWER HELD",
                    small.clone(),
                    Color32::from_gray(160),
                );
                painter.text(
                    mid + Vec2::new(0.0, h2 * 0.08),
                    Align2::CENTER_CENTER,
                    c.power.map(|p| format!("{p:.0} W")).unwrap_or("—".into()),
                    big.clone(),
                    Color32::WHITE,
                );
                let note = match hr {
                    Some(h) if (h - c.target_bpm).abs() <= 3.0 => "On target".to_string(),
                    Some(h) if h < c.target_bpm => {
                        format!("{:.0} bpm below — easing power up", c.target_bpm - h)
                    }
                    Some(h) => format!("{:.0} bpm above — easing power down", h - c.target_bpm),
                    None => "No heart rate signal — holding power".to_string(),
                };
                painter.text(
                    right,
                    Align2::RIGHT_CENTER,
                    note,
                    small.clone(),
                    Color32::from_gray(210),
                );
                if let Some(h) = hr {
                    let col = charts::hr_zone_color(s.hr_zones().zone(h));
                    bar((h / c.target_bpm.max(1.0)).min(1.2) / 1.2, col);
                }
            }
            _ => {
                let route = &self.routes[self.route_idx];
                let lap = (self.ride.route_pos() / route.length).floor() as u32 + 1;
                let along = route.along(self.ride.route_pos()).min(route.length);
                // Laps of a loop; point to point, the way to the finish.
                let stage = if route.is_loop() {
                    format!("Lap {lap}")
                } else if self.ride.route_pos() < route.length {
                    "To the finish".to_string()
                } else {
                    "Past the finish".to_string()
                };
                let (mode, g) = if self.ride.mode == Mode::Slope {
                    ("Manual slope".to_string(), self.ride.manual_grade as f64)
                } else {
                    let felt = match self.ride.trainer_setting() {
                        Some(devices::Control::Grade(g)) => format!(" · trainer set to {g:+.1}%"),
                        _ => String::new(),
                    };
                    (
                        format!("Free ride · difficulty {:.0}%{felt}", s.difficulty * 100.0),
                        grade,
                    )
                };
                painter.text(
                    left - Vec2::new(0.0, h2 * 0.22),
                    Align2::LEFT_CENTER,
                    mode,
                    small.clone(),
                    Color32::from_gray(190),
                );
                painter.text(
                    left + Vec2::new(0.0, h2 * 0.08),
                    Align2::LEFT_CENTER,
                    format!("{g:+.1}%"),
                    big.clone(),
                    Color32::WHITE,
                );
                painter.text(
                    mid - Vec2::new(0.0, h2 * 0.22),
                    Align2::CENTER_CENTER,
                    self.store.settings.units.route_name(&route.name),
                    small.clone(),
                    Color32::from_gray(160),
                );
                painter.text(
                    mid + Vec2::new(0.0, h2 * 0.08),
                    Align2::CENTER_CENTER,
                    stage,
                    big.clone(),
                    Color32::WHITE,
                );
                painter.text(
                    right,
                    Align2::RIGHT_CENTER,
                    format!(
                        "{:.2} / {}",
                        self.store.settings.units.distance(along),
                        self.store.settings.units.fmt_distance(route.length, 1)
                    ),
                    small.clone(),
                    Color32::from_gray(210),
                );
                bar(
                    (along / route.length) as f32,
                    Color32::from_rgb(90, 150, 100),
                );
            }
        }
        if self.ride.state == ride::State::Ready {
            painter.text(
                panel.center_top() + Vec2::new(0.0, 10.0),
                Align2::CENTER_TOP,
                "Press Start (or Space) when you're ready",
                small.clone(),
                Color32::from_rgb(240, 200, 90),
            );
        } else if self.ride.state == ride::State::Paused {
            painter.text(
                panel.center_top() + Vec2::new(0.0, 10.0),
                Align2::CENTER_TOP,
                "PAUSED",
                FontId::proportional(18.0),
                Color32::from_rgb(240, 200, 90),
            );
        }

        // Row 3: ride totals.
        let y3 = y2 + h2 + gap;
        let p = self.ride.power_series();
        let hrs: Vec<f32> = self
            .ride
            .samples
            .iter()
            .filter_map(|s| s.hr)
            .filter(|h| *h > 0.0)
            .collect();
        let avg_hr = (!hrs.is_empty()).then(|| hrs.iter().sum::<f32>() / hrs.len() as f32);
        // `+ 0.0` turns the empty sum's -0.0 into 0.0 so it never shows as "-0".
        let kj = p.iter().sum::<f32>() / 1000.0 + 0.0;
        let tiles = row(y3, h3, 6);
        tile(
            painter,
            tiles[0],
            "TIME",
            &fmt_dur(self.ride.elapsed as f32),
            "",
            grey,
        );
        tile(
            painter,
            tiles[1],
            // Leave room for the companion's gap beside the caption.
            if self.ride.companion.is_some() {
                "DIST"
            } else {
                "DISTANCE"
            },
            &format!(
                "{:.2}",
                self.store.settings.units.distance(self.ride.distance)
            ),
            self.store.settings.units.distance_unit(),
            grey,
        );
        if let Some((title, value, unit, col)) = self.companion_status() {
            let who = title.split(' ').next().unwrap_or("").to_lowercase();
            sub(tiles[1], format!("{who} {value} {unit}"), col);
        }
        if cadence_tile {
            tile(
                painter,
                tiles[2],
                self.speed_label(),
                &format!(
                    "{:.1}",
                    self.store.settings.units.speed(self.ride.speed * 3.6)
                ),
                self.store.settings.units.speed_unit(),
                grey,
            );
        } else {
            tile(
                painter,
                tiles[2],
                "CLIMBED",
                &format!("{:.0}", self.ride.ascent),
                "m",
                grey,
            );
        }
        tile(
            painter,
            tiles[3],
            "AVG POWER",
            &if p.is_empty() {
                dash()
            } else {
                format!("{:.0}", self.ride.avg_power())
            },
            "W",
            grey,
        );
        tile(
            painter,
            tiles[4],
            "NORMALIZED",
            &if p.is_empty() {
                dash()
            } else {
                format!("{:.0}", workout::normalized_power(&p))
            },
            "W",
            grey,
        );
        tile(
            painter,
            tiles[5],
            "AVG HR / ENERGY",
            &format!(
                "{} · {kj:.0}",
                avg_hr.map(|h| format!("{h:.0}")).unwrap_or_else(dash)
            ),
            "bpm · kJ",
            grey,
        );

        // Row 4: live chart.
        let y4 = y3 + h3 + gap;
        let chart = Rect::from_min_max(Pos2::new(r.left(), y4), r.max);
        let target: Vec<Option<f32>> = self
            .ride
            .samples
            .iter()
            .map(|s| {
                if self.ride.mode != Mode::Workout {
                    return None;
                }
                workout::target_at(&self.ride.segs, s.elapsed)
                    .filter(|t| !t.1)
                    .map(|t| (t.0 * self.ride.intensity * ftp).round())
            })
            .collect();
        let hr: Vec<Option<f32>> = self
            .ride
            .samples
            .iter()
            .map(|s| s.hr.filter(|h| *h > 0.0))
            .collect();
        charts::live_chart(painter, chart, &p, &target, &hr, ftp, 600, theme::CARD);
        self.trainer_warning(painter, Self::banner_rect(rect, false));
    }
}
