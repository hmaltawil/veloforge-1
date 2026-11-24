//! Talking to other services: intervals.icu, Strava, the update check and
//! the phone remote.

use super::*;

impl App {
    pub(super) fn start_remote(&mut self) {
        self.remote = None;
        let s = &mut self.store.settings;
        if s.remote_token.is_empty() {
            s.remote_token = crate::remote::new_token();
            self.dirty = true;
        }
        let ctx = self.egui_ctx.clone();
        let port = s.remote_port;
        match crate::remote::Remote::start(port, s.remote_token.clone(), move || {
            ctx.request_repaint()
        }) {
            Ok(r) => {
                self.remote_url = r.url();
                self.remote_error = None;
                self.remote = Some(r);
                self.log_line(format!("Phone remote listening on port {port}"));
            }
            Err(e) => {
                let msg = if e.kind() == std::io::ErrorKind::AddrInUse {
                    format!(
                        "Port {port} is already in use. Is VeloForge already open in another window?"
                    )
                } else {
                    format!("Could not start the phone remote: {e}")
                };
                self.log_line(msg.clone());
                self.remote_error = Some(msg);
            }
        }
    }

    /// Handle phone actions and publish the ride state for the phone.
    pub(super) fn pump_remote(&mut self, ctx: &egui::Context) {
        let Some(r) = &self.remote else { return };
        let actions: Vec<_> = r.rx.try_iter().collect();
        let acted = !actions.is_empty();
        for a in actions {
            self.ride_action(a);
        }
        let Some(r) = &self.remote else { return };
        // After a button press, answer at once so the phone shows the result.
        if acted || self.remote_pushed.elapsed().as_millis() >= 300 {
            self.remote_pushed = Instant::now();
            r.set_state(self.remote_state());
        }
        if r.phone_connected() {
            ctx.request_repaint_after(std::time::Duration::from_millis(350));
        }
    }

    /// Live numbers and controls for the phone page, as JSON.
    fn remote_state(&self) -> String {
        let s = &self.store.settings;
        let ftp = s.ftp;
        let hex = |c: Color32| format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b());
        let power = self.metrics.power3s();
        let hr = fresh(&self.metrics.hr);
        let target = self.ride.target_watts(ftp);
        let t = self.ride.workout_time() as f32;
        let interval = match (&self.ride.workout, self.ride.mode) {
            (Some(w), Mode::Workout) if self.ride.workout_done() => Some(serde_json::json!({
                "label": w.name,
                "now": "Workout complete!",
                "next": "Keep spinning to cool down, then finish",
                "progress": 1.0,
                "color": "#78dc8c",
            })),
            (Some(w), Mode::Workout) => workout::segment_index(&self.ride.segs, t).map(|i| {
                let seg = self.ride.segs[i];
                // The phone's cadence tile shows the current cadence target,
                // so this big line stays short enough not to wrap.
                let now = if seg.free {
                    "Free ride".to_string()
                } else {
                    format!("{:.0} W", seg.at(t) * self.ride.intensity * ftp)
                };
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
                let col = if seg.free {
                    Color32::GRAY
                } else {
                    charts::zone_color(seg.at(t))
                };
                serde_json::json!({
                    "label": format!("{} · {}/{}", w.name, i + 1, self.ride.segs.len()),
                    "now": format!("{now} · {} left", fmt_dur((seg.end() - t).ceil())),
                    "next": next.unwrap_or_else(|| "Last interval".into()),
                    "progress": (t - seg.start) / seg.secs,
                    "color": hex(col),
                })
            }),
            _ => None,
        };
        let (adjust_label, adjust_value) = match self.ride.mode {
            Mode::Workout => ("Intensity", format!("{:.0}%", self.ride.intensity * 100.0)),
            Mode::Slope => ("Slope", format!("{:+.1}%", self.ride.manual_grade)),
            Mode::Route => (
                "Trainer difficulty",
                format!("{:.0}%", s.difficulty * 100.0),
            ),
            Mode::HeartRate => (
                "Target heart rate",
                format!("{:.0} bpm", self.ride.hr_control.target_bpm),
            ),
        };
        let companion = self.companion_status().map(|(title, value, unit, _)| {
            let who = title.split(" · ").next().unwrap_or("").to_lowercase();
            let mut who = who.chars();
            let who: String = who
                .next()
                .map(|c| c.to_uppercase().chain(who).collect())
                .unwrap_or_default();
            format!("{who} {value} {unit}")
        });
        let grade = if self.ride.mode == Mode::Slope {
            self.ride.manual_grade as f64
        } else {
            self.routes[self.route_idx].grade(self.ride.route_pos())
        };
        serde_json::json!({
            "state": match self.ride.state {
                ride::State::Ready => "ready",
                ride::State::Running => "running",
                ride::State::Paused => "paused",
            },
            "title": format!(
                "{} · {}",
                self.ride.title(),
                self.store.settings.units.route_name(&self.routes[self.route_idx].name)
            ),
            "power": power.map(f32::round),
            "power_color": power.map(|p| hex(charts::zone_color(p / ftp))),
            "hr": hr.map(f32::round),
            "hr_color": hr.map(|h| hex(charts::hr_zone_color(s.hr_zones().zone(h)))),
            "target": target,
            "target_color": target.map(|t| hex(charts::zone_color(t / ftp))),
            "cadence": fresh(&self.metrics.cadence).map(f32::round),
            "cadence_target": self.ride.target_cadence().map(f32::round),
            "speed": s.units.speed(self.ride.speed * 3.6),
            "speed_unit": s.units.speed_unit(),
            "distance_unit": s.units.distance_unit(),
            "grade": grade,
            "grade_color": hex(charts::grade_color(grade as f32)),
            "draft": (self.ride.draft >= 0.02).then(|| (self.ride.draft * 100.0).round()),
            "segment": self.segment_lines().into_iter().take(2).map(|l| l.0).collect::<Vec<_>>(),
            "notice": self
                .toast
                .as_ref()
                .filter(|(_, at)| at.elapsed().as_secs_f32() < 5.0)
                .map(|(m, _)| m.clone()),
            "camera": s.camera.label(),
            "time": fmt_dur(self.ride.elapsed as f32),
            "distance": s.units.distance(self.ride.distance),
            "interval": interval,
            "companion": companion,
            "can_start": self.can_start(),
            "can_skip": self.ride.mode == Mode::Workout && self.ride_active() && !self.ride.workout_done(),
            "adjust_label": adjust_label,
            "adjust_value": adjust_value,
            "view_3d": s.view_3d,
        })
        .to_string()
    }

    pub(super) fn icu_sync(&mut self) {
        if !self.icu_creds.is_set() || !self.store.settings.icu_sync_plan {
            return;
        }
        let today = Local::now().date_naive();
        self.icu_busy = true;
        self.icu_last_sync = Some(Instant::now());
        self.icu.send(crate::icu::Request::Sync {
            creds: self.icu_creds.clone(),
            oldest: today - Duration::days(1),
            newest: today + Duration::days(28),
        });
    }

    pub(super) fn icu_upload(&mut self, file: String, name: String) {
        if self.icu_creds.is_set() {
            self.log_line(format!("Uploading “{name}” to intervals.icu…"));
            self.icu.send(crate::icu::Request::Upload {
                creds: self.icu_creds.clone(),
                file,
                name,
            });
        }
    }

    /// Send a ride to Strava, as its FIT file where there is one.
    pub(super) fn strava_upload(&mut self, s: &ride::Summary) {
        if !self.strava_creds.is_connected() {
            return;
        }
        let fit = Path::new(&s.file).with_extension("fit");
        let file = if fit.exists() {
            fit.display().to_string()
        } else {
            s.file.clone()
        };
        self.log_line(format!("Uploading “{}” to Strava…", s.name));
        self.strava_pending.push(s.file.clone());
        self.strava.send(crate::strava::Request::Upload {
            creds: self.strava_creds.clone(),
            file,
            name: s.name.clone(),
            key: s.file.clone(),
        });
    }

    pub(super) fn pump_strava(&mut self) {
        use crate::strava::Reply;
        while let Ok(reply) = self.strava.rx.try_recv() {
            match reply {
                Reply::Connected(Ok(creds)) => {
                    self.strava_busy = false;
                    let who = if creds.athlete.is_empty() {
                        "Connected".to_string()
                    } else {
                        format!("Connected as {}", creds.athlete)
                    };
                    self.log_line(format!("Strava: {who}"));
                    self.strava_status = Some(Ok(who));
                    if let Err(e) = creds.save(&self.paths.strava_file()) {
                        self.notify(format!("Could not save the Strava connection: {e}"));
                    }
                    self.strava_edit = creds.clone();
                    self.strava_creds = creds;
                }
                Reply::Connected(Err(e)) => {
                    self.strava_busy = false;
                    self.log_line(format!("Strava: {e}"));
                    self.strava_status = Some(Err(e));
                }
                Reply::Uploaded { key, result, creds } => {
                    self.strava_pending.retain(|k| *k != key);
                    if let Some(c) = creds {
                        let _ = c.save(&self.paths.strava_file());
                        self.strava_creds = c;
                    }
                    let name = self
                        .store
                        .history
                        .iter()
                        .find(|h| h.file == key)
                        .map(|h| h.name.clone())
                        .unwrap_or_default();
                    match result {
                        Ok(id) => {
                            if let Some(h) = self.store.history.iter_mut().find(|h| h.file == key) {
                                h.strava_id = Some(id);
                            }
                            if let Some(Modal::Saved(s)) = &mut self.modal {
                                if s.file == key {
                                    s.strava_id = Some(String::new());
                                }
                            }
                            self.mark_dirty();
                            self.log_line(format!("Uploaded “{name}” to Strava"));
                            self.notify("Ride uploaded to Strava");
                        }
                        Err(e) => {
                            self.log_line(format!("Strava upload failed: {e}"));
                            self.notify(format!("Upload to Strava failed: {e}"));
                        }
                    }
                }
            }
        }
    }

    /// Ask GitHub, in the background, whether a newer version exists.
    pub(super) fn check_for_update(&mut self) {
        let url = std::env::var("VELOFORGE_UPDATE_URL")
            .unwrap_or_else(|_| crate::update::LATEST_URL.to_string());
        self.update = Some(crate::update::Status::Checking);
        self.update_rx = Some(crate::update::check(self.egui_ctx.clone(), url));
    }

    pub(super) fn pump_update(&mut self) {
        use crate::update::Status;
        use std::sync::mpsc::TryRecvError;
        let Some(rx) = &self.update_rx else { return };
        let status = match rx.try_recv() {
            Ok(status) => status,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Status::Failed("the check stopped".into()),
        };
        self.update_rx = None;
        match &status {
            Status::Available(r) => {
                self.log_line(format!("VeloForge {} is available: {}", r.version, r.url))
            }
            Status::UpToDate => self.log_line("VeloForge is up to date".into()),
            Status::Failed(e) => self.log_line(format!("Could not check for a new version: {e}")),
            Status::Checking => {}
        }
        self.update = Some(status);
    }

    pub(super) fn pump_icu(&mut self) {
        use crate::icu::Reply;
        while let Ok(reply) = self.icu.rx.try_recv() {
            match reply {
                Reply::Connected(Ok(a)) => {
                    self.icu_busy = false;
                    self.icu_status = Some(Ok(format!("Connected as {}", a.name)));
                    if self.icu_creds != self.icu_edit {
                        self.icu_creds = self.icu_edit.clone();
                        if let Err(e) = self.icu_creds.save(&self.paths.credentials_file()) {
                            self.notify(format!("Could not save intervals.icu settings: {e}"));
                        }
                    }
                    self.log_line(format!("intervals.icu: connected as {}", a.name));
                    self.icu_athlete = Some(a);
                    self.icu_sync();
                }
                Reply::Connected(Err(e)) => {
                    self.icu_busy = false;
                    self.log_line(format!("intervals.icu: {e}"));
                    self.icu_status = Some(Err(e));
                }
                Reply::Synced {
                    oldest,
                    newest,
                    result,
                } => {
                    self.icu_busy = false;
                    match result {
                        Ok(events) => self.apply_icu_sync(oldest, newest, events),
                        Err(e) => {
                            self.log_line(format!("intervals.icu sync failed: {e}"));
                            self.icu_note = Some(format!("Sync failed: {e}"));
                        }
                    }
                }
                Reply::Uploaded { file, result } => match result {
                    Ok(id) => {
                        let name = self
                            .store
                            .history
                            .iter()
                            .find(|h| h.file == file)
                            .map(|h| h.name.clone())
                            .unwrap_or_default();
                        if let Some(h) = self.store.history.iter_mut().find(|h| h.file == file) {
                            h.icu_id = Some(id);
                        }
                        self.mark_dirty();
                        self.log_line(format!("Uploaded “{name}” to intervals.icu"));
                        self.notify("Ride uploaded to intervals.icu");
                    }
                    Err(e) => {
                        self.log_line(format!("intervals.icu upload failed: {e}"));
                        self.notify(format!("Upload to intervals.icu failed: {e}"));
                    }
                },
            }
        }
    }

    /// Mirror intervals.icu's planned rides into the calendar.
    fn apply_icu_sync(
        &mut self,
        oldest: NaiveDate,
        newest: NaiveDate,
        events: Vec<crate::icu::PlannedRide>,
    ) {
        let keep: std::collections::HashSet<String> = events
            .iter()
            .map(|(e, _)| format!("icu-{}", e.id))
            .collect();
        // Planned rides removed on intervals.icu disappear here too.
        self.store.plan.retain(|p| {
            !(p.workout_id.starts_with("icu-")
                && !p.done
                && p.date >= oldest
                && p.date <= newest
                && !keep.contains(&p.workout_id))
        });
        let mut unreadable = Vec::new();
        for (event, zwo) in &events {
            let id = format!("icu-{}", event.id);
            match zwo
                .clone()
                .and_then(|x| workout::parse_zwo(&x, &id).map_err(|e| e.to_string()))
            {
                Ok(mut w) => {
                    w.name = event.name.clone();
                    w.category = Category::Intervals;
                    self.store.custom_workouts.retain(|c| c.id != id);
                    self.store.custom_workouts.push(w);
                }
                Err(e) => {
                    if !self.store.custom_workouts.iter().any(|c| c.id == id) {
                        unreadable.push(format!("{} ({e})", event.name));
                        continue;
                    }
                }
            }
            match self.store.plan.iter_mut().find(|p| p.workout_id == id) {
                Some(p) => p.date = event.date,
                None => self.store.plan.push(PlannedWorkout {
                    date: event.date,
                    workout_id: id,
                    done: false,
                    skipped: false,
                }),
            }
        }
        // Forget downloaded workouts nothing refers to any more.
        let used: std::collections::HashSet<String> = self
            .store
            .plan
            .iter()
            .map(|p| p.workout_id.clone())
            .collect();
        self.store
            .custom_workouts
            .retain(|c| !c.id.starts_with("icu-") || used.contains(&c.id));
        self.store.plan.sort_by_key(|p| p.date);
        self.mark_dirty();
        let n = events.len() - unreadable.len();
        self.icu_note = Some(format!(
            "Synced at {} · {n} planned ride{}",
            Local::now().format("%H:%M"),
            if n == 1 { "" } else { "s" }
        ));
        self.log_line(format!("intervals.icu: {n} planned rides synced"));
        for u in unreadable {
            self.log_line(format!("intervals.icu: could not use planned ride {u}"));
        }
    }
}
