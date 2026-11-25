//! The Devices tab: finding and connecting trainers and sensors, and
//! calibrating.

use super::*;

impl App {
    pub(super) fn on_calibration(&mut self, c: Calibration) {
        let Some(cal) = &mut self.calib else {
            // A late event after the dialog was closed.
            return;
        };
        cal.since = Instant::now();
        match c {
            Calibration::Started => cal.stage = CalStage::Starting,
            Calibration::Targets(lo, hi) => {
                cal.targets = Some((lo, hi));
                cal.stage = CalStage::SpeedUp;
            }
            Calibration::SpeedUp => cal.stage = CalStage::SpeedUp,
            Calibration::StopPedaling => {
                cal.stage = CalStage::Coast;
                self.cue(Cue::Change);
            }
            Calibration::Success => {
                cal.stage = CalStage::Done;
                self.cue(Cue::Done);
            }
            Calibration::Failed(m) => {
                cal.stage = CalStage::Failed(m);
                self.cue(Cue::Alert);
            }
            Calibration::Cancelled => self.calib = None,
        }
    }

    pub(super) fn devices_tab(&mut self, ui: &mut egui::Ui) {
        ui.columns(2, |cols| {
            let ui = &mut cols[0];
            egui::ScrollArea::vertical().id_salt("devices-left").auto_shrink([false, false]).show(ui, |ui| {
                ui.heading("Bluetooth devices");
                match &self.adapter {
                    None => {
                        ui.label("Starting Bluetooth…");
                    }
                    Some(Ok(name)) => {
                        ui.label(RichText::new(format!("Adapter: {name}")).weak());
                    }
                    Some(Err(e)) => {
                        ui.colored_label(Color32::from_rgb(240, 120, 90), format!("Bluetooth unavailable: {e}"));
                    }
                }
                ui.horizontal(|ui| {
                    if self.scanning {
                        ui.spinner();
                        if ui.button("Stop scan").clicked() {
                            self.dev.send(Cmd::StopScan);
                        }
                    } else if ui.button(RichText::new("Scan for devices").strong()).clicked() {
                        self.dev.send(Cmd::StartScan);
                    }
                    ui.checkbox(&mut self.show_all_devices, "Show all Bluetooth devices");
                });
                ui.add_space(6.0);
                let (shown, hidden): (Vec<DeviceInfo>, Vec<DeviceInfo>) =
                    self.found.iter().cloned().partition(|d| self.show_all_devices || is_fitness_device(d));
                for d in shown {
                    self.device_card(ui, &d);
                }
                if !hidden.is_empty() {
                    ui.label(RichText::new(format!("{} other Bluetooth devices hidden (phones, TVs, …)", hidden.len())).weak());
                }
                if self.found.is_empty() && !self.scanning {
                    ui.label(RichText::new("Wake your trainer by pedalling, then press Scan.").weak());
                }

                ui.add_space(12.0);
                ui.heading("Connections");
                if self.links.is_empty() {
                    ui.label(RichText::new("Nothing connected yet.").weak());
                }
                let ids: Vec<String> = self.links.keys().cloned().collect();
                for id in ids {
                    self.connection_card(ui, &id);
                }

                ui.add_space(12.0);
                self.shifting_section(ui);

                ui.add_space(12.0);
                ui.heading("Simulated trainer");
                ui.label(RichText::new("Try everything without hardware. The simulator follows ERG targets and grade like a real trainer. Real sensors always take priority over it.").weak());
                let sim_on = self.links.get(SIM_ID).is_some_and(|l| l.link == Link::Connected);
                ui.horizontal(|ui| {
                    if ui.button(if sim_on { "Disable simulator" } else { "Enable simulator" }).clicked() {
                        self.dev.send(Cmd::SimEnable(!sim_on));
                    }
                    ui.label("Rider effort");
                    if ui.add(egui::Slider::new(&mut self.sim_effort, 50.0..=1500.0).logarithmic(true).suffix(" W")).changed() {
                        self.dev.send(Cmd::SimEffort(self.sim_effort));
                    }
                });
            });

            let ui = &mut cols[1];
            egui::ScrollArea::vertical().id_salt("devices-right").auto_shrink([false, false]).show(ui, |ui| {
                ui.heading("Live data");
                let source = |m: &Option<Metric>| -> String {
                    m.as_ref()
                        .filter(|m| m.is_fresh())
                        .map(|m| self.links.get(&m.src).map(|l| l.name.clone()).unwrap_or_else(|| m.src.clone()))
                        .unwrap_or_default()
                };
                let row = |ui: &mut egui::Ui, k: &str, v: Option<f32>, unit: &str, src: String| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(k).weak());
                        ui.label(RichText::new(v.map(|v| format!("{v:.0} {unit}")).unwrap_or("—".into())).size(20.0).strong());
                        if !src.is_empty() {
                            ui.label(RichText::new(format!("from {src}")).weak());
                        }
                    });
                };
                row(ui, "Power", fresh(&self.metrics.power), "W", source(&self.metrics.power));
                row(ui, "Cadence", fresh(&self.metrics.cadence), "rpm", source(&self.metrics.cadence));
                if !self.has_cadence() {
                    ui.label(RichText::new("No cadence sensor connected. Cadence is optional: pair a cadence sensor, or a power meter that measures it, to see rpm.").weak());
                }
                row(ui, "Heart rate", fresh(&self.metrics.hr), "bpm", source(&self.metrics.hr));
                if fresh(&self.metrics.hr).is_none() && self.per_device.values().any(|d| d.hr_noisy == Some(true)) {
                    ui.colored_label(Color32::from_rgb(240, 150, 80), "Heart rate hidden: the strap's signal is noisy (see its card on the left).");
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.heading("Log");
                    if ui.button("Copy log").on_hover_text("Copy the log to paste it into a message").clicked() {
                        let text: Vec<&str> = self.log.iter().rev().map(|s| s.as_str()).collect();
                        ui.ctx().copy_text(text.join("\n"));
                        self.toast = Some(("Log copied to clipboard".into(), Instant::now()));
                    }
                    if ui.button("Open full log").on_hover_text(self.paths.log_file().display().to_string()).clicked() {
                        crate::platform::open(self.paths.log_file());
                    }
                    if ui.button("Save log to file").clicked() {
                        let text: Vec<&str> = self.log.iter().rev().map(|s| s.as_str()).collect();
                        let path = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default().join("veloforge-log.txt");
                        self.toast = Some(match std::fs::write(&path, text.join("\n")) {
                            Ok(()) => (format!("Log saved to {}", path.display()), Instant::now()),
                            Err(e) => (format!("Could not save log: {e}"), Instant::now()),
                        });
                    }
                    if ui.checkbox(&mut self.raw_log, "Log raw sensor data").on_hover_text("Record every packet from your sensors, useful for diagnosing odd readings").changed() {
                        self.dev.send(Cmd::RawLog(self.raw_log));
                    }
                });
                for l in &self.log {
                    ui.label(RichText::new(l).monospace().small());
                }
            });
        });
    }

    /// Virtual shifting: the option, the gear, and whether it can work.
    fn shifting_section(&mut self, ui: &mut egui::Ui) {
        ui.heading("Virtual shifting");
        let s = &mut self.store.settings;
        if ui
            .checkbox(&mut s.virtual_shifting, "Virtual shifting (for a single-cog setup like Zwift Cog)")
            .on_hover_text("Leave this off if your bike has a real cassette: then you shift your own gears and the trainer follows the road.")
            .changed()
        {
            self.mark_dirty();
        }
        ui.label(
            RichText::new(
                "For a bike with one fixed sprocket on the trainer. VeloForge gives it gears (a 34/50 chainset with an 11-34 cassette): \
                 your cadence in the gear you are in sets how fast you would go, and the trainer is asked for the power that speed needs \
                 on the road, so the same pedalling is harder in a bigger gear and up a steeper hill. Shift with + and - on the keyboard, \
                 the phone remote, or a Zwift Click. In a workout's power targets (ERG) the trainer holds the power and the gears rest.",
            )
            .weak(),
        );
        if self.gears_arg && !self.store.settings.virtual_shifting {
            ui.label(
                RichText::new("On for this run (started with --gears).").color(theme::TEXT_DIM),
            );
        }
        if !self.gears_on() {
            return;
        }
        ui.horizontal(|ui| {
            if ui
                .button(" - ")
                .on_hover_text("Easier gear (- or [)")
                .clicked()
            {
                self.shift(false);
            }
            let colour = if self.gear_flashing() {
                theme::ACCENT
            } else {
                theme::TEXT
            };
            ui.label(
                RichText::new(crate::gears::label(self.store.settings.gear))
                    .size(18.0)
                    .strong()
                    .color(colour),
            );
            if ui
                .button(" + ")
                .on_hover_text("Harder gear (+ or ])")
                .clicked()
            {
                self.shift(true);
            }
        });
        if !self.has_cadence() {
            ui.colored_label(
                theme::WARN,
                "No cadence coming in. Virtual shifting needs your cadence, from the trainer or a cadence sensor: pedal to wake them, \
                 or pair a cadence sensor. Until there is one, the trainer follows the road as without gears.",
            );
        }
        if self.trainer_link().is_none() {
            ui.label(
                RichText::new(
                    "Connect a smart trainer (or the simulator below) to feel the gears.",
                )
                .weak(),
            );
        }
    }

    fn device_card(&mut self, ui: &mut egui::Ui, d: &DeviceInfo) {
        let link = self.links.get(&d.id).map(|l| l.link);
        egui::Frame::new()
            .fill(CARD)
            .corner_radius(6)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&d.name).strong());
                    if let Some(r) = d.rssi {
                        ui.label(RichText::new(format!("{r} dBm")).weak());
                    }
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| match link {
                            Some(Link::Connected)
                            | Some(Link::Connecting)
                            | Some(Link::Reconnecting) => {
                                if ui.button("Disconnect").clicked() {
                                    self.dev.send(Cmd::Disconnect(d.id.clone()));
                                    self.store.settings.auto_connect.retain(|x| *x != d.id);
                                    self.mark_dirty();
                                }
                            }
                            _ => {
                                if ui.button("Connect").clicked() {
                                    self.dev.send(Cmd::Connect(d.id.clone()));
                                }
                            }
                        },
                    );
                });
                let caps: Vec<&str> = d.caps.iter().map(|c| c.label()).collect();
                if !caps.is_empty() {
                    ui.label(
                        RichText::new(caps.join(" · "))
                            .small()
                            .color(Color32::from_rgb(120, 190, 240)),
                    );
                }
            });
    }

    fn connection_card(&mut self, ui: &mut egui::Ui, id: &str) {
        let Some(l) = self.links.get(id) else { return };
        let (txt, c) = match l.link {
            Link::Connected => ("connected", Color32::from_rgb(90, 210, 120)),
            Link::Connecting => ("connecting…", Color32::from_rgb(230, 200, 90)),
            Link::Reconnecting => ("reconnecting…", Color32::from_rgb(230, 160, 70)),
            Link::Disconnected => ("disconnected", Color32::GRAY),
        };
        let live = self.per_device.get(id);
        let reading = |m: Option<&Option<Metric>>, unit: &str| {
            m.and_then(fresh).map(|v| format!("{v:.0} {unit}"))
        };
        let data: Vec<String> = [
            reading(live.map(|d| &d.power), "W"),
            reading(live.map(|d| &d.cadence), "rpm"),
            reading(live.map(|d| &d.hr), "bpm"),
            live.and_then(|d| fresh(&d.speed))
                .map(|v| self.store.settings.units.fmt_speed(v as f64)),
        ]
        .into_iter()
        .flatten()
        .collect();
        let rate = live
            .map(|d| {
                d.updates
                    .iter()
                    .filter(|t| t.elapsed().as_secs_f32() <= 5.0)
                    .count() as f32
                    / 5.0
            })
            .unwrap_or(0.0);
        let silent_for = live
            .and_then(|d| d.updates.back())
            .map(|t| t.elapsed().as_secs());
        let mut disconnect = false;
        let mut open_calibration = false;
        egui::Frame::new().fill(CARD).corner_radius(6).inner_margin(8).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(&l.name).strong());
                ui.colored_label(c, txt);
                if l.controllable && l.link == Link::Connected {
                    ui.label(RichText::new("controls resistance").color(Color32::from_rgb(240, 150, 70)));
                    if ui.button("Calibrate").on_hover_text("Spindown calibration for accurate power").clicked() {
                        open_calibration = true;
                    }
                }
                if l.link != Link::Disconnected && id != SIM_ID {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        disconnect = ui.button("Disconnect").clicked();
                    });
                }
            });
            let caps: Vec<&str> = l.caps.iter().map(|c| c.label()).collect();
            if !caps.is_empty() {
                ui.label(RichText::new(caps.join(" · ")).small().color(Color32::from_rgb(120, 190, 240)));
            }
            if l.link == Link::Connected {
                if data.is_empty() {
                    let msg = match silent_for {
                        Some(s) => format!("No data for {s} s — pedal to wake it. See the log on the right."),
                        None => "No data yet — pedal to wake it. See the log on the right.".into(),
                    };
                    ui.label(RichText::new(msg).color(Color32::from_rgb(230, 170, 70)));
                } else {
                    ui.label(
                        RichText::new(format!("Live: {}  ·  {rate:.1} updates/s", data.join(" · "))).color(Color32::from_rgb(140, 220, 160)),
                    );
                }
                if l.caps.contains(&Capability::HeartRate) && id != SIM_ID {
                    let strap_hr = live.and_then(|d| fresh(&d.hr));
                    let beat_hr = live.and_then(|d| fresh(&d.hr_rr));
                    let contact = live.and_then(|d| d.hr_contact);
                    let mut parts = Vec::new();
                    match contact {
                        Some(true) => parts.push("skin contact: yes".to_string()),
                        Some(false) => parts.push("skin contact: NO".to_string()),
                        None => {}
                    }
                    if let Some(b) = beat_hr {
                        parts.push(format!("from beat timing: {b:.0} bpm"));
                    }
                    if !parts.is_empty() {
                        ui.label(RichText::new(parts.join("  ·  ")).weak());
                    }
                    let disagree = matches!((strap_hr, beat_hr), (Some(a), Some(b)) if (a - b).abs() > 20.0);
                    let noisy = live.and_then(|d| d.hr_noisy) == Some(true);
                    if noisy {
                        ui.colored_label(
                            Color32::from_rgb(240, 150, 80),
                            "The strap is detecting beats at random intervals, so it is measuring electrical noise rather than your heart. Its readings are ignored. Wet the electrodes, tighten the strap, and replace its battery (CR2032) if it is old.",
                        );
                    } else if contact == Some(false) {
                        ui.colored_label(
                            Color32::from_rgb(240, 150, 80),
                            "The strap is not touching your skin properly, so its readings are ignored. Wet the two electrode pads and tighten the strap.",
                        );
                    } else if disagree {
                        ui.colored_label(
                            Color32::from_rgb(240, 150, 80),
                            "The strap's heart rate and its beat timing disagree, so the reading is unreliable. Wet the electrodes, tighten the strap, and check its battery.",
                        );
                    }
                }
                if l.caps.is_empty() && id != SIM_ID {
                    ui.label(
                        RichText::new("No fitness services found on this device. Remove it from your system Bluetooth settings, then connect again here.")
                            .color(Color32::from_rgb(240, 120, 90)),
                    );
                }
            }
        });
        if open_calibration {
            self.calib = Some(CalibUi {
                stage: CalStage::Intro,
                targets: None,
                since: Instant::now(),
            });
        }
        if disconnect {
            self.dev.send(Cmd::Disconnect(id.to_string()));
            self.store.settings.auto_connect.retain(|x| x != id);
            self.mark_dirty();
        }
    }

    pub(super) fn calibration_window(&mut self, ctx: &egui::Context) {
        let Some(cal) = &self.calib else { return };
        let stage = cal.stage.clone();
        let targets = cal.targets;
        let waited = cal.since.elapsed().as_secs_f32();
        // Give up if the trainer goes quiet.
        let timeout = match stage {
            CalStage::Starting => 10.0,
            CalStage::SpeedUp | CalStage::Coast => 90.0,
            _ => f32::INFINITY,
        };
        if waited > timeout {
            self.dev.send(Cmd::Calibrate(false));
            self.calib = Some(CalibUi {
                stage: CalStage::Failed(
                    "The trainer stopped responding. Make sure it is awake and try again.".into(),
                ),
                targets,
                since: Instant::now(),
            });
            return;
        }
        let speed = fresh(&self.metrics.speed);
        let ride_active = self.ride_active();
        let legacy = self
            .trainer_link()
            .is_some_and(|l| l.caps.contains(&Capability::LegacyWahoo));
        let units = self.store.settings.units;
        let mut action: Option<&str> = None;
        egui::Window::new("Trainer calibration").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.set_width(420.0);
            let big = |ui: &mut egui::Ui, text: String, color: Color32| {
                ui.label(RichText::new(text).size(26.0).strong().color(color));
            };
            let speed_line = |ui: &mut egui::Ui| {
                let text = speed.map(|v| units.fmt_speed(v as f64)).unwrap_or(format!("— {}", units.speed_unit()));
                ui.label(RichText::new(text).size(40.0).strong());
            };
            match &stage {
                CalStage::Intro => {
                    ui.label("Calibration (a \"spindown\") makes your trainer's power readings accurate.");
                    ui.add_space(6.0);
                    ui.label("• Ride for about 10 minutes first so the tyre is warm.");
                    ui.label("• Check your tyre pressure and roller tension as usual.");
                    ui.label("• Redo it whenever you change tyre pressure, tyre or wheel.");
                    ui.add_space(6.0);
                    ui.label("You will be asked to speed up, then to stop pedalling and let the wheel coast.");
                    ui.add_space(8.0);
                    if legacy {
                        ui.colored_label(
                            Color32::from_rgb(240, 150, 80),
                            "Your trainer's firmware only supports Wahoo's older protocol, which VeloForge cannot calibrate. Run the spindown in the Wahoo app (the result is stored in the trainer and applies here too), or update the trainer's firmware there to enable calibration in VeloForge.",
                        );
                    }
                    if ride_active {
                        ui.colored_label(Color32::from_rgb(240, 150, 80), "Finish or discard the current ride first.");
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!ride_active && !legacy, egui::Button::new(RichText::new("Start calibration").strong())).clicked() {
                            action = Some("start");
                        }
                        if ui.button("Close").clicked() {
                            action = Some("close");
                        }
                    });
                }
                CalStage::Starting => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Asking the trainer to start calibration…");
                    });
                    if ui.button("Cancel").clicked() {
                        action = Some("cancel");
                    }
                }
                CalStage::SpeedUp => {
                    let goal = targets.map(|(_, hi)| format!("Pedal up to {:.0} {}", units.speed(hi as f64), units.speed_unit())).unwrap_or("Speed up until told to stop".into());
                    big(ui, goal, Color32::from_rgb(240, 200, 90));
                    speed_line(ui);
                    if let (Some((_, hi)), Some(v)) = (targets, speed) {
                        let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 10.0), Sense::hover());
                        ui.painter().rect_filled(r, 4.0, Color32::from_white_alpha(25));
                        let col = if v >= hi { Color32::from_rgb(90, 210, 120) } else { Color32::from_rgb(240, 200, 90) };
                        ui.painter().rect_filled(Rect::from_min_size(r.min, Vec2::new(r.width() * (v / hi).clamp(0.0, 1.0), r.height())), 4.0, col);
                    }
                    ui.label(RichText::new("Hold that speed until the screen says stop.").weak());
                    if ui.button("Cancel").clicked() {
                        action = Some("cancel");
                    }
                }
                CalStage::Coast => {
                    big(ui, "Stop pedalling now".into(), Color32::from_rgb(120, 200, 255));
                    ui.label("Let the wheel coast to a stop on its own, without braking.");
                    speed_line(ui);
                    if ui.button("Cancel").clicked() {
                        action = Some("cancel");
                    }
                }
                CalStage::Done => {
                    big(ui, "Calibration complete".into(), Color32::from_rgb(90, 210, 120));
                    ui.label("Your trainer's power readings are calibrated.");
                    if ui.button("Close").clicked() {
                        action = Some("close");
                    }
                }
                CalStage::Failed(msg) => {
                    big(ui, "Calibration did not finish".into(), Color32::from_rgb(240, 120, 90));
                    ui.label(msg);
                    ui.horizontal(|ui| {
                        if ui.button("Try again").clicked() {
                            action = Some("start");
                        }
                        if ui.button("Close").clicked() {
                            action = Some("close");
                        }
                    });
                }
            }
        });
        match action {
            Some("start") => {
                self.calib = Some(CalibUi {
                    stage: CalStage::Starting,
                    targets: None,
                    since: Instant::now(),
                });
                self.dev.send(Cmd::Calibrate(true));
            }
            Some("cancel") => {
                self.dev.send(Cmd::Calibrate(false));
                self.calib = None;
            }
            Some("close") => self.calib = None,
            _ => {}
        }
    }
}

/// Devices worth listing by default: anything advertising a fitness service,
/// or with a name from a known fitness brand.
fn is_fitness_device(d: &DeviceInfo) -> bool {
    const NAMES: [&str; 24] = [
        "kickr", "wahoo", "tickr", "rpm", "hrm", "polar", "garmin", "coospo", "magene", "tacx",
        "neo", "flux", "elite", "saris", "wattbike", "stages", "assioma", "favero", "4iiii",
        "rally", "vector", "quarq", "zwift", "heart",
    ];
    let n = d.name.to_lowercase();
    !d.caps.is_empty() || NAMES.iter().any(|k| n.contains(k))
}
