//! Finishing and saving rides: finishing on its own at the end of a route,
//! checkpoints a ride can be recovered from, importing and opening files,
//! and putting activities in the trash.

use super::*;

impl App {
    /// Open the system's file dialog for a workout (.zwo) or route (.gpx),
    /// as `filters` (a name and its extensions) allow, and import what is
    /// chosen. The dialog runs on a thread of its own so the rest keeps
    /// going (on macOS it must be the main thread: there it waits).
    pub(crate) fn pick_file(&mut self, title: &str, filters: &[(&str, &[&str])]) {
        if self.picking.is_some() {
            return;
        }
        let mut dialog = rfd::FileDialog::new().set_title(title);
        for (name, exts) in filters {
            dialog = dialog.add_filter(*name, exts);
        }
        if let Some(dir) =
            directories::UserDirs::new().and_then(|u| u.download_dir().map(Path::to_path_buf))
        {
            dialog = dialog.set_directory(dir);
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let run = move || {
            let opened = Instant::now();
            let picked =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| dialog.pick_file()))
                    .map_err(|_| "no file dialog is available".to_string())
                    .and_then(|p| {
                        // No answer at once with nothing chosen: no dialog
                        // came up (no desktop portal on Linux), not a cancel.
                        if p.is_none() && opened.elapsed() < std::time::Duration::from_millis(300) {
                            Err("no file dialog came up".to_string())
                        } else {
                            Ok(p)
                        }
                    });
            let _ = tx.send(picked);
        };
        if cfg!(target_os = "macos") {
            run();
        } else if std::thread::Builder::new()
            .name("file-dialog".into())
            .spawn(run)
            .is_err()
        {
            self.notify("Could not open the file dialog");
            return;
        }
        self.picking = Some(rx);
    }

    /// Import the file chosen in the file dialog, once it is chosen.
    pub(super) fn check_picked_file(&mut self) {
        let Some(rx) = &self.picking else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(Some(path))) => {
                self.picking = None;
                self.import_file(&path, None);
            }
            Ok(Ok(None)) => self.picking = None,
            Ok(Err(e)) => {
                self.picking = None;
                self.notify(format!(
                    "Could not open the file dialog ({e}): type the file's path in Workouts instead"
                ));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.picking = None,
        }
    }

    /// Finish the ride by itself once its workout is done, or (riding
    /// without one) once a point-to-point route's finish is crossed, unless
    /// "Keep riding" was chosen. A workout still going at a point-to-point
    /// finish carries on along the road beyond it.
    pub(super) fn check_auto_finish(&mut self) {
        if !self.ride_active() {
            self.auto_finish = None;
            self.keep_riding = false;
            self.last_route_pos = self.ride.route_pos();
            return;
        }
        let route = &self.routes[self.route_idx];
        let (looped, finish) = (route.is_loop(), route.length);
        let pos = self.ride.route_pos();
        let crossed = !looped && self.last_route_pos < finish && pos >= finish;
        self.last_route_pos = pos;
        let workout = self.ride.mode == Mode::Workout && self.ride.workout.is_some();
        if crossed && workout && !self.ride.workout_done() {
            self.notify("Finish line! The road carries on for the rest of your workout");
        }
        if self.keep_riding || self.ride.state != ride::State::Running {
            return;
        }
        match self.auto_finish {
            None => {
                let why = if workout && self.ride.workout_done() {
                    Some("Workout complete!")
                } else if !workout && !looped && pos >= finish {
                    Some("You reached the finish!")
                } else {
                    None
                };
                if let Some(why) = why {
                    self.auto_finish = Some((Instant::now(), why));
                }
            }
            Some((since, _)) if since.elapsed().as_secs_f32() >= Self::AUTO_FINISH_SECS => {
                self.auto_finish = None;
                self.finish_ride();
            }
            Some(_) => {}
        }
    }

    /// The card counting down to the ride finishing by itself, with Finish
    /// now and Keep riding.
    pub(super) fn auto_finish_card(&mut self, ctx: &egui::Context) {
        let Some((since, why)) = self.auto_finish else {
            return;
        };
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
        let left = (Self::AUTO_FINISH_SECS - since.elapsed().as_secs_f32())
            .ceil()
            .max(0.0);
        let practice = self.ride_simulated || self.demo;
        let mut finish = false;
        let mut keep = false;
        egui::Area::new(egui::Id::unique("auto-finish"))
            .anchor(Align2::CENTER_CENTER, [0.0, -60.0])
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(18)
                    .show(ui, |ui| {
                        ui.set_min_width(320.0);
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new(why)
                                    .size(22.0)
                                    .strong()
                                    .color(Color32::from_rgb(120, 220, 140)),
                            );
                            ui.add_space(4.0);
                            ui.label(RichText::new(if practice {
                                format!("Finishing in {left:.0} s")
                            } else {
                                format!("Saving your ride in {left:.0} s")
                            }));
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("Finish now").color(Color32::WHITE),
                                        )
                                        .fill(Color32::from_rgb(44, 112, 196)),
                                    )
                                    .clicked()
                                {
                                    finish = true;
                                }
                                if ui
                                    .button("Keep riding")
                                    .on_hover_text(
                                        "Cool down or ride on; press Finish when you are done",
                                    )
                                    .clicked()
                                {
                                    keep = true;
                                }
                            });
                        });
                    });
            });
        if finish {
            self.auto_finish = None;
            self.finish_ride();
        } else if keep {
            self.auto_finish = None;
            self.keep_riding = true;
        }
    }

    pub(super) fn finish_ride(&mut self) {
        if self.ride.samples.len() < 10 {
            self.notify("Ride too short to save");
            self.reset_ride();
            return;
        }
        if self.ride_simulated || self.demo {
            // Practice with the simulator: no History, Progress, uploads or
            // segment records.
            self.log_line("Finished a simulated ride (not saved)".into());
            self.notify("Simulated ride finished: practice rides are not saved");
            self.reset_ride();
            return;
        }
        let ftp = self.store.settings.ftp;
        let started = self.ride.started.unwrap_or_else(Local::now);
        let file = self.activity_path(started, &self.ride.title());
        let res = std::fs::create_dir_all(&self.paths.activities)
            .and_then(|_| std::fs::write(&file, self.ride.to_tcx()));
        if let Err(e) = res {
            self.notify(format!("Could not write activity: {e}"));
            return;
        }
        // The same ride as a FIT file, which Strava and others read best.
        if let Err(e) = std::fs::write(file.with_extension("fit"), self.ride.to_fit()) {
            self.log_line(format!("Could not write the FIT file: {e}"));
        }
        let zones = self.store.settings.hr_zones();
        let mut summary = self.ride.summary(ftp, &zones, file.display().to_string());
        summary.route_id = Some(self.routes[self.route_idx].id.clone());
        self.tick_off_plan(
            summary.workout_id.as_deref(),
            self.ride.plan_date,
            started.date_naive(),
        );
        self.store.history.push(summary.clone());
        self.store.efforts.append(&mut self.ride_efforts);
        if self.store.settings.icu_upload {
            self.icu_upload(summary.file.clone(), summary.name.clone());
        }
        if self.store.settings.strava_upload {
            self.strava_upload(&summary);
        }
        self.save_now();
        self.modal = Some(Modal::Saved(Box::new(summary)));
        self.reset_ride();
    }

    pub(super) fn reset_ride(&mut self) {
        let mode = self.ride.mode;
        let w = self.ride.workout.clone();
        self.replace_ride(Ride::new(mode, w, None));
        self.dev.send(Cmd::Control(devices::Control::Grade(0.0)));
        self.clear_checkpoint();
    }

    /// Move a deleted ride's file to the Trash (Recycle Bin): only files in the
    /// activities folder that no other ride in History still uses.
    pub(super) fn trash_activity(&mut self, file: &str) {
        let path = Path::new(file);
        let ours = match (
            path.parent().and_then(|p| p.canonicalize().ok()),
            self.paths.activities.canonicalize().ok(),
        ) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        };
        let shared = self.store.history.iter().any(|h| h.file == file);
        if !ours || shared || !path.exists() {
            return;
        }
        // The ride's .tcx, and the .fit saved next to it.
        let fit = path.with_extension("fit");
        for p in [path, fit.as_path()] {
            if p != path && !p.exists() {
                continue;
            }
            match crate::activity::move_to_trash(p) {
                Ok(()) => self.log_line(format!("Moved {} to the {TRASH}", p.display())),
                Err(e) => {
                    self.log_line(format!(
                        "Could not move {} to the {TRASH}: {e}",
                        p.display()
                    ));
                    self.notify(format!(
                        "The ride was removed, but its files could not be moved to the {TRASH}"
                    ));
                }
            }
        }
    }

    fn activity_path(&self, started: chrono::DateTime<Local>, title: &str) -> std::path::PathBuf {
        let slug: String = title
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        self.paths.activities.join(format!(
            "{}_{}.tcx",
            started.format("%Y-%m-%d_%H%M%S"),
            slug
        ))
    }

    /// Tick off the calendar entry a ride came from, or today's match.
    /// Matched by date and workout (not by position) so editing the calendar
    /// mid-ride cannot tick off the wrong entry.
    fn tick_off_plan(
        &mut self,
        workout_id: Option<&str>,
        plan_date: Option<NaiveDate>,
        ride_date: NaiveDate,
    ) {
        let Some(wid) = workout_id else { return };
        let find = |date: NaiveDate| {
            self.store
                .plan
                .iter()
                .position(|p| p.date == date && !p.done && p.workout_id == wid)
        };
        if let Some(i) = plan_date.and_then(find).or_else(|| find(ride_date)) {
            self.store.plan[i].done = true;
        }
    }

    fn checkpoint_paths(&self) -> (std::path::PathBuf, std::path::PathBuf) {
        (
            self.paths.activities.join("in-progress.tcx"),
            self.paths.activities.join("in-progress.json"),
        )
    }

    /// Save the ride so far, so a crash or power cut loses at most a minute.
    pub(super) fn checkpoint(&mut self) {
        let (tcx, meta) = self.checkpoint_paths();
        let info = serde_json::json!({
            "name": self.ride.title(),
            "workout_id": self.ride.workout.as_ref().map(|w| w.id.clone()),
            "plan_date": self.ride.plan_date,
            "started": self.ride.started,
            "route_id": self.routes[self.route_idx].id,
            "simulated": self.ride_simulated || self.demo,
        });
        let res = std::fs::create_dir_all(&self.paths.activities)
            .and_then(|_| std::fs::write(&tcx, self.ride.to_tcx()))
            .and_then(|_| std::fs::write(&meta, info.to_string()));
        if let Err(e) = res {
            self.log_line(format!("Could not save ride checkpoint: {e}"));
        }
        self.last_checkpoint = Instant::now();
    }

    fn clear_checkpoint(&self) {
        let (tcx, meta) = self.checkpoint_paths();
        let _ = std::fs::remove_file(tcx);
        let _ = std::fs::remove_file(meta);
    }

    /// Turn a ride left behind by a crash into a normal saved ride.
    pub(super) fn recover_checkpoint(&mut self) {
        let (tcx, meta) = self.checkpoint_paths();
        let Ok(xml) = std::fs::read_to_string(&tcx) else {
            return;
        };
        let info: serde_json::Value = std::fs::read_to_string(&meta)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let activity = match crate::activity::parse_tcx(&xml) {
            Ok(a) if a.samples.len() >= 10 && info["simulated"].as_bool() != Some(true) => a,
            _ => {
                self.clear_checkpoint();
                return;
            }
        };
        let started = info["started"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&Local))
            .unwrap_or_else(|| activity.samples[0].time.with_timezone(&Local));
        let name = info["name"]
            .as_str()
            .map(String::from)
            .or(activity.name)
            .unwrap_or_else(|| "Ride".into());
        let workout_id = info["workout_id"].as_str().map(String::from);
        let plan_date = info["plan_date"]
            .as_str()
            .and_then(|s| s.parse::<NaiveDate>().ok());
        let file = self.activity_path(started, &name);
        if std::fs::rename(&tcx, &file).is_err() {
            return;
        }
        let _ = std::fs::remove_file(meta);
        let meta = ride::SummaryMeta {
            start: started,
            name: name.clone(),
            workout_id: workout_id.clone(),
        };
        let s = &self.store.settings;
        let mut summary = ride::summarize(
            &activity.samples,
            meta,
            s.ftp,
            &s.hr_zones(),
            file.display().to_string(),
        );
        let _ = std::fs::write(
            file.with_extension("fit"),
            crate::fit::activity(&activity.samples, &[0], summary.np),
        );
        summary.route_id = info["route_id"].as_str().map(String::from);
        self.tick_off_plan(workout_id.as_deref(), plan_date, started.date_naive());
        if self.store.settings.icu_upload {
            self.icu_upload(summary.file.clone(), summary.name.clone());
        }
        if self.store.settings.strava_upload {
            self.strava_upload(&summary);
        }
        self.store.history.push(summary);
        self.store.history.sort_by_key(|h| h.start);
        self.save_now();
        self.notify(format!(
            "Recovered your unfinished ride “{name}” from {}",
            started.format("%a %d %b %H:%M")
        ));
    }

    pub(super) fn import_file(&mut self, path: &Path, bytes: Option<&[u8]>) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let text = match bytes {
            Some(b) => String::from_utf8_lossy(b).into_owned(),
            None => match std::fs::read_to_string(path) {
                Ok(t) => t,
                Err(e) => return self.notify(format!("Cannot read {}: {e}", path.display())),
            },
        };
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("import")
            .to_string();
        // A route is called by its file's name ("Alpe_d_Huez.gpx": "Alpe d
        // Huez"), not the name inside it, often an app's placeholder.
        let title = stem.replace('_', " ");
        let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        let title = if title.is_empty() {
            stem.clone()
        } else {
            title
        };
        let id = format!(
            "{}-{}",
            ext,
            stem.to_lowercase()
                .replace(|c: char| !c.is_ascii_alphanumeric(), "-")
        );
        match ext.as_str() {
            "zwo" => match workout::parse_zwo(&text, &id) {
                Ok(w) => {
                    let name = w.name.clone();
                    self.store.custom_workouts.retain(|x| x.id != id);
                    self.store.custom_workouts.push(w);
                    self.selected_workout = Some(id);
                    self.tab = Tab::Workouts;
                    self.mark_dirty();
                    self.notify(format!("Imported workout “{name}”"));
                }
                Err(e) => self.notify(format!("Could not import workout: {e:#}")),
            },
            "gpx" => match route::parse_gpx(&text, &id, &title) {
                Ok(g) => {
                    let name = g.name.clone();
                    self.store.gpx_routes.retain(|x| x.id != id);
                    self.store.gpx_routes.push(g);
                    self.store.settings.route_id = id;
                    self.rebuild_routes();
                    self.mark_dirty();
                    self.notify(format!("Imported route “{name}”"));
                }
                Err(e) => self.notify(format!("Could not import route: {e:#}")),
            },
            _ => self.notify("Drop a .zwo workout or a .gpx route"),
        }
    }
}
