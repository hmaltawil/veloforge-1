//! The Devices tab: finding and connecting trainers and sensors, and
//! calibrating.

use super::glass::{self, Glyph, Lead};
use super::*;

/// The parts of the Devices screen its sidebar leads to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DevSection {
    Connected,
    Shifting,
    Simulator,
    Calibration,
    Log,
}

const SECTIONS: [(DevSection, &str, Glyph); 5] = [
    (DevSection::Connected, "Connected", Glyph::Plug),
    (DevSection::Shifting, "Virtual shifting", Glyph::Crank),
    (DevSection::Simulator, "Simulator", Glyph::Robot),
    (DevSection::Calibration, "Calibration", Glyph::Dial),
    (DevSection::Log, "Live data & log", Glyph::Lines),
];

/// What one of the cards at the top shows.
enum Slot {
    /// A connected (or connecting) device.
    Link(String),
    /// No trainer yet.
    Trainer,
    /// Power read from the trainer, or none yet.
    Power,
    /// No heart-rate strap yet.
    Heart,
}

/// A card at the top, at least `want` tall so a row's cards line up, with
/// its buttons along the bottom. Returns how tall its content needs it.
fn device_frame(
    ui: &mut egui::Ui,
    want: f32,
    body: impl FnOnce(&mut egui::Ui),
    buttons: Option<impl FnOnce(&mut egui::Ui)>,
) -> f32 {
    let mut natural = 0.0;
    egui::Frame::new()
        .fill(Color32::from_white_alpha(23))
        .corner_radius(22)
        .inner_margin(18)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height((want - 36.0).max(0.0));
            let gap = 12.0;
            ui.spacing_mut().item_spacing.y = gap;
            let top = ui.cursor().top();
            body(ui);
            // The cursor sits one gap below the last thing drawn.
            let body_h = ui.cursor().top() - top - gap;
            let rest = if buttons.is_some() { gap + 34.0 } else { 0.0 };
            natural = body_h + rest + 36.0;
            let pad = want - natural;
            if pad > 0.0 && buttons.is_some() {
                ui.add_space(pad);
            }
            if let Some(b) = buttons {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    b(ui)
                });
            }
        });
    natural
}

/// The top of a device card: its state, its name and a line about it.
fn card_head(
    ui: &mut egui::Ui,
    colour: Color32,
    status: &str,
    title: &str,
    sub: &str,
) -> egui::Response {
    let r = glass::status_lit(ui, colour, status);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        glass::heading(ui, title);
        ui.add(
            egui::Label::new(
                RichText::new(sub)
                    .font(theme::regular(12.5))
                    .color(glass::QUIET),
            )
            .truncate(),
        );
    });
    r
}

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
        let pane = ui.max_rect().expand2(Vec2::new(28.0, 26.0));
        glass::area(ui, pane, "devices", |ui| {
            let (side, main) = glass::split(ui);
            if let Some(side) = side {
                glass::area(ui, side, "devices-side", |ui| {
                    self.devices_sidebar(ui, true)
                });
            }
            glass::area(ui, main, "devices-main", |ui| {
                self.devices_main(ui, side.is_none())
            });
        });
    }

    /// Down the side: the screen's sections, then the devices found nearby.
    fn devices_sidebar(&mut self, ui: &mut egui::Ui, side: bool) {
        ui.spacing_mut().item_spacing.y = 4.0;
        if side {
            for (sec, name, g) in SECTIONS {
                if glass::side_row(ui, self.devices_section == sec, Lead::Icon(g), name, "")
                    .clicked()
                {
                    self.choose_section(sec);
                }
            }
            ui.add_space(14.0);
        }
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                RichText::new("Nearby")
                    .font(theme::medium(15.0))
                    .color(Color32::WHITE),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                if self.scanning {
                    glass::status(ui, glass::WAIT, "Scanning");
                } else {
                    glass::status(ui, Color32::from_gray(120), "Idle");
                }
            });
        });
        ui.add_space(4.0);
        match &self.adapter {
            None => {
                ui.label(RichText::new("   Starting Bluetooth…").color(glass::QUIET));
            }
            Some(Ok(_)) => {}
            Some(Err(e)) => {
                ui.colored_label(glass::ALERT, format!("Bluetooth unavailable: {e}"));
            }
        }
        let (shown, hidden): (Vec<DeviceInfo>, Vec<DeviceInfo>) = self
            .found
            .iter()
            .cloned()
            .partition(|d| self.show_all_devices || is_fitness_device(d));
        let list = |ui: &mut egui::Ui, app: &mut App| {
            if shown.is_empty() && app.adapter.as_ref().is_some_and(|a| a.is_ok()) {
                ui.label(
                    RichText::new(if app.scanning {
                        "   Looking for trainers and sensors…"
                    } else {
                        "   Scan to find your trainer and sensors."
                    })
                    .font(theme::regular(12.5))
                    .color(glass::QUIET),
                );
            }
            for d in &shown {
                app.nearby_row(ui, d);
            }
        };
        if side {
            egui::ScrollArea::vertical()
                .id_salt("devices-left")
                .max_height((ui.available_height() - 120.0).max(80.0))
                .auto_shrink([false, true])
                .show(ui, |ui| list(ui, self));
        } else {
            list(ui, self);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let n = hidden.len();
            let text = if self.show_all_devices || n == 0 {
                "Show all Bluetooth devices".to_string()
            } else {
                format!("Show all Bluetooth devices ({n} hidden)")
            };
            ui.checkbox(
                &mut self.show_all_devices,
                RichText::new(text)
                    .font(theme::regular(12.5))
                    .color(glass::QUIET),
            );
        });
        if side {
            let left = ui.available_height() - 60.0;
            if left > 0.0 {
                ui.add_space(left);
            }
        }
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.add(
                egui::Label::new(
                    RichText::new("Can't see your strap? Wet the contacts and put it on. Straps only wake up when worn.")
                        .font(theme::regular(12.5))
                        .color(glass::QUIET),
                )
                .wrap(),
            );
        });
    }

    /// Go to a section of the screen, from the sidebar.
    fn choose_section(&mut self, sec: DevSection) {
        self.devices_section = sec;
        self.devices_jump = true;
        if sec == DevSection::Calibration {
            let real = self
                .links
                .iter()
                .any(|(id, l)| id != SIM_ID && l.controllable && l.link == Link::Connected);
            if real {
                self.calib = Some(CalibUi {
                    stage: CalStage::Intro,
                    targets: None,
                    since: Instant::now(),
                });
            } else {
                self.toast = Some((
                    "Connect your smart trainer to calibrate it".into(),
                    Instant::now(),
                ));
            }
        }
    }

    /// Scroll to `sec` if it was just chosen in the sidebar.
    fn jump_here(&self, ui: &egui::Ui, secs: &[DevSection]) {
        if self.devices_jump && secs.contains(&self.devices_section) {
            ui.scroll_to_rect(
                Rect::from_min_size(ui.cursor().min, Vec2::splat(1.0)),
                Some(egui::Align::TOP),
            );
        }
    }

    fn devices_main(&mut self, ui: &mut egui::Ui, narrow: bool) {
        ui.horizontal(|ui| {
            glass::title(ui, "Devices");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.scanning {
                    if glass::pill(ui, "Stop scan", true).clicked() {
                        self.dev.send(Cmd::StopScan);
                    }
                } else if glass::pill_white(ui, "Scan for devices").clicked() {
                    self.dev.send(Cmd::StartScan);
                }
            });
        });
        ui.add_space(12.0);
        egui::ScrollArea::vertical()
            .id_salt("devices-main")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                if narrow {
                    self.devices_sidebar(ui, false);
                    ui.add_space(12.0);
                }
                self.jump_here(ui, &[DevSection::Connected, DevSection::Calibration]);
                self.device_cards(ui);
                ui.add_space(4.0);
                self.jump_here(ui, &[DevSection::Shifting]);
                glass::card(ui, |ui| self.shifting_section(ui));
                ui.add_space(4.0);
                self.jump_here(ui, &[DevSection::Simulator]);
                glass::card(ui, |ui| self.simulator_section(ui));
                ui.add_space(4.0);
                self.jump_here(ui, &[DevSection::Log]);
                glass::card(ui, |ui| self.log_section(ui));
            });
        self.devices_jump = false;
    }

    /// What goes in the cards at the top, in order: the trainer, the
    /// power, the heart rate, then anything else connected.
    fn card_slots(&self) -> Vec<Slot> {
        let up = |l: &LinkInfo| l.link != Link::Disconnected;
        let trainer = self
            .links
            .get_key_value(SIM_ID)
            .filter(|(_, l)| l.controllable && l.link == Link::Connected)
            .or_else(|| {
                self.links
                    .iter()
                    .find(|(_, l)| l.controllable && l.link == Link::Connected)
            })
            .map(|(id, _)| id.clone());
        let meter = self
            .links
            .iter()
            .find(|(id, l)| {
                *id != SIM_ID && up(l) && !l.controllable && l.caps.contains(&Capability::Power)
            })
            .map(|(id, _)| id.clone());
        let strap = self
            .links
            .iter()
            .find(|(id, l)| {
                *id != SIM_ID
                    && up(l)
                    && Some(*id) != trainer.as_ref()
                    && l.caps.contains(&Capability::HeartRate)
            })
            .map(|(id, _)| id.clone());
        let mut slots = vec![
            trainer.clone().map(Slot::Link).unwrap_or(Slot::Trainer),
            meter.clone().map(Slot::Link).unwrap_or(Slot::Power),
            strap.clone().map(Slot::Link).unwrap_or(Slot::Heart),
        ];
        let taken = [trainer, meter, strap];
        for (id, l) in &self.links {
            if taken.iter().any(|t| t.as_ref() == Some(id)) || (id == SIM_ID && !up(l)) {
                continue;
            }
            slots.push(Slot::Link(id.clone()));
        }
        slots
    }

    /// The cards for what is connected, in rows of equal height.
    fn device_cards(&mut self, ui: &mut egui::Ui) {
        let slots = self.card_slots();
        let gap = 12.0;
        let n = (((ui.available_width() + gap) / (230.0 + gap)).floor() as usize)
            .clamp(1, 4)
            .min(slots.len());
        for (row, chunk) in slots.chunks(n).enumerate() {
            let key = ui.scope_id().with(("device-row", row, n));
            let want: f32 = ui.ctx().data(|d| d.get_temp(key)).unwrap_or(0.0);
            let mut tallest = 0.0f32;
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.columns(n, |cols| {
                    for (ui, slot) in cols.iter_mut().zip(chunk) {
                        let h = match slot {
                            Slot::Link(id) => self.link_card(ui, id, want),
                            _ => self.empty_card(ui, slot, want),
                        };
                        tallest = tallest.max(h);
                    }
                });
            });
            if (tallest - want).abs() > 0.5 {
                ui.ctx().data_mut(|d| d.insert_temp(key, tallest));
                ui.ctx().request_repaint();
            }
            ui.add_space(gap - 8.0);
        }
    }

    /// A card for something not connected (or read from the trainer).
    fn empty_card(&self, ui: &mut egui::Ui, slot: &Slot, want: f32) -> f32 {
        let grey = Color32::from_gray(130);
        let from_sim =
            |m: &Option<Metric>| m.as_ref().is_some_and(|m| m.is_fresh() && m.src == SIM_ID);
        let trainer = self.trainer_link().map(|l| l.name.clone());
        let (colour, status, title, sub, text) = match slot {
            Slot::Trainer => (
                grey,
                "Not connected".to_string(),
                "Smart trainer",
                "FTMS or Wahoo trainers".to_string(),
                "Scan, then connect it from Nearby. No trainer to hand? Switch on the simulated trainer below.",
            ),
            Slot::Power => match &trainer {
                Some(t) => (
                    glass::OK,
                    "From the trainer".to_string(),
                    "Power",
                    format!("Read from {t}"),
                    "Got a power meter on the bike? Connect it from Nearby to read your power from it as well.",
                ),
                None => (
                    grey,
                    "Nothing yet".to_string(),
                    "Power",
                    "From a trainer or power meter".to_string(),
                    "Connect a smart trainer or a power meter from Nearby.",
                ),
            },
            _ => {
                let (c, st) = if from_sim(&self.metrics.hr) {
                    (glass::OK, "From the simulated trainer")
                } else if self.scanning {
                    (glass::WAIT, "Searching")
                } else {
                    (grey, "Not connected")
                };
                (
                    c,
                    st.to_string(),
                    "Heart rate",
                    "A chest strap or a watch that broadcasts".to_string(),
                    "Not needed for workouts in ERG. Needed for heart-rate rides.",
                )
            }
        };
        device_frame(
            ui,
            want,
            |ui| {
                card_head(ui, colour, &status, title, &sub);
                ui.add(
                    egui::Label::new(
                        RichText::new(text)
                            .font(theme::regular(13.0))
                            .color(glass::SOFT),
                    )
                    .wrap(),
                );
            },
            None::<fn(&mut egui::Ui)>,
        )
    }

    /// A card for a connected device: its state, live readings, what it
    /// can do, any trouble, and its buttons.
    fn link_card(&mut self, ui: &mut egui::Ui, id: &str, want: f32) -> f32 {
        let Some(l) = self.links.get(id).cloned() else {
            return 0.0;
        };
        let sim = id == SIM_ID;
        let (status, colour) = match l.link {
            Link::Connected if sim => ("On", glass::OK),
            Link::Connected => ("Connected", glass::OK),
            Link::Connecting => ("Connecting…", glass::WAIT),
            Link::Reconnecting => ("Reconnecting…", glass::WAIT),
            Link::Disconnected => ("Disconnected", Color32::from_gray(130)),
        };
        let has = |c: Capability| l.caps.contains(&c);
        let title = if sim {
            "Simulated trainer"
        } else if l.controllable || has(Capability::SmartTrainer) || has(Capability::LegacyWahoo) {
            "Smart trainer"
        } else if has(Capability::Shifter) {
            "Zwift Click"
        } else if has(Capability::HeartRate) {
            "Heart rate"
        } else if has(Capability::Power) {
            "Power meter"
        } else if has(Capability::Cadence) {
            "Cadence sensor"
        } else {
            "Bluetooth device"
        };
        let mut sub: Vec<String> = Vec::new();
        if sim {
            sub.push(format!("Rider effort {:.0} W", self.sim_effort));
        } else {
            sub.push(l.name.clone());
            if has(Capability::SmartTrainer) {
                sub.push("FTMS".into());
            } else if has(Capability::LegacyWahoo) {
                sub.push("Wahoo".into());
            }
        }
        if l.controllable {
            sub.push("controls resistance".into());
        }
        let sub = sub.join(" · ");

        let live = self.per_device.get(id);
        let mut figures: Vec<(String, &str)> = Vec::new();
        if let Some(d) = live {
            for (m, unit) in [(&d.power, "W"), (&d.cadence, "rpm"), (&d.hr, "bpm")] {
                if let Some(v) = fresh(m) {
                    figures.push((format!("{v:.0}"), unit));
                }
            }
        }
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
        let connected = l.link == Link::Connected;

        let mut chips: Vec<&str> = Vec::new();
        for (c, name) in [
            (Capability::Power, "Power"),
            (Capability::Cadence, "Cadence"),
            (Capability::HeartRate, "Heart rate"),
            (Capability::Shifter, "Shifting"),
        ] {
            if has(c) {
                chips.push(name);
            }
        }
        if l.controllable {
            chips.extend(["ERG", "Slope"]);
        }

        // Lines under the readings: green news, amber trouble.
        let mut notes: Vec<(Color32, String)> = Vec::new();
        let amber = Color32::from_rgb(255, 196, 120);
        if connected && has(Capability::Shifter) {
            notes.push((
                Color32::from_rgb(167, 243, 197),
                match self.presses.get(id) {
                    Some(&(n, at, up)) => format!(
                        "{n} press{} · last {} {} s ago",
                        if n == 1 { "" } else { "es" },
                        if up { "+ (up)" } else { "− (down)" },
                        at.elapsed().as_secs()
                    ),
                    None => "Press + or − to try it".to_string(),
                },
            ));
            if !self.gears_on() {
                notes.push((
                    amber,
                    "Turn on virtual shifting below for its presses to change gear.".into(),
                ));
            }
        } else if connected && figures.is_empty() && !sim {
            notes.push((
                amber,
                match silent_for {
                    Some(s) => {
                        format!("No data for {s} s: pedal to wake it. The log below shows more.")
                    }
                    None => "No data yet: pedal to wake it. The log below shows more.".into(),
                },
            ));
        }
        if connected && has(Capability::HeartRate) && !sim {
            let strap_hr = live.and_then(|d| fresh(&d.hr));
            let beat_hr = live.and_then(|d| fresh(&d.hr_rr));
            let contact = live.and_then(|d| d.hr_contact);
            let disagree =
                matches!((strap_hr, beat_hr), (Some(a), Some(b)) if (a - b).abs() > 20.0);
            if live.and_then(|d| d.hr_noisy) == Some(true) {
                notes.push((amber, "Measuring electrical noise, not your heart, so it is ignored. Wet the electrodes, tighten the strap, and replace an old battery (CR2032).".into()));
            } else if contact == Some(false) {
                notes.push((amber, "Not touching your skin properly, so it is ignored. Wet the two pads and tighten the strap.".into()));
            } else if disagree {
                notes.push((amber, "Its heart rate and beat timing disagree, so the reading is unreliable. Wet the electrodes, tighten the strap, check the battery.".into()));
            }
        }
        if connected && l.caps.is_empty() && !sim {
            notes.push((
                glass::ALERT,
                "No fitness services found. Remove it in your system's Bluetooth settings, then connect again here.".into(),
            ));
        }

        let can_calibrate = connected && l.controllable && !sim;
        let can_drop = !sim && l.link != Link::Disconnected;
        let can_join = !sim && l.link == Link::Disconnected;
        let mut calibrate = false;
        let mut drop = false;
        let mut join = false;
        let buttons = (can_calibrate || can_drop || can_join).then_some(|ui: &mut egui::Ui| {
            if can_calibrate
                && glass::pill(ui, "Calibrate", true)
                    .on_hover_text("Spindown calibration for accurate power")
                    .clicked()
            {
                calibrate = true;
            }
            if can_drop && glass::pill(ui, "Disconnect", false).clicked() {
                drop = true;
            }
            if can_join && glass::pill(ui, "Connect", true).clicked() {
                join = true;
            }
        });
        let h = device_frame(
            ui,
            want,
            |ui| {
                card_head(ui, colour, status, title, &sub)
                    .on_hover_text(format!("{rate:.1} updates a second"));
                if !figures.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 18.0;
                        for (v, unit) in &figures {
                            ui.scope(|ui| {
                                ui.spacing_mut().item_spacing.x = 4.0;
                                ui.label(
                                    RichText::new(v)
                                        .font(theme::medium(26.0))
                                        .color(Color32::WHITE),
                                );
                                ui.label(
                                    RichText::new(*unit)
                                        .font(theme::regular(12.0))
                                        .color(glass::QUIET),
                                );
                            });
                        }
                    });
                }
                if !chips.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::splat(6.0);
                        for c in &chips {
                            glass::chip(ui, c);
                        }
                    });
                }
                for (c, t) in &notes {
                    ui.add(
                        egui::Label::new(RichText::new(t).font(theme::regular(12.5)).color(*c))
                            .wrap(),
                    );
                }
            },
            buttons,
        );
        if calibrate {
            self.calib = Some(CalibUi {
                stage: CalStage::Intro,
                targets: None,
                since: Instant::now(),
            });
        }
        if drop {
            self.dev.send(Cmd::Disconnect(id.to_string()));
            self.store.settings.auto_connect.retain(|x| x != id);
            self.mark_dirty();
        }
        if join {
            self.dev.send(Cmd::Connect(id.to_string()));
        }
        h
    }

    /// Virtual shifting: the switch, the gear you are in, and every gear.
    fn shifting_section(&mut self, ui: &mut egui::Ui) {
        use crate::gears;
        ui.spacing_mut().item_spacing.y = 10.0;
        let width = ui.available_width();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_max_width(width - 110.0);
                ui.spacing_mut().item_spacing.y = 2.0;
                glass::heading(ui, "Virtual shifting");
                ui.add(
                    egui::Label::new(
                        RichText::new("Shift with + and − on the keyboard, the phone remote, or a Zwift Click.")
                            .font(theme::regular(13.0))
                            .color(glass::SOFT),
                    )
                    .wrap(),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut on = self.store.settings.virtual_shifting;
                if glass::toggle(ui, &mut on)
                    .on_hover_text(
                        "For a bike with one fixed sprocket on the trainer, like a Zwift Cog. VeloForge gives it gears: \
                         your cadence in the gear you are in sets how fast you would go, and the trainer is asked for the \
                         power that speed needs on the road. In a workout's power targets (ERG) the trainer holds the \
                         power and the gears rest. Leave it off if your bike has a real cassette.",
                    )
                    .changed()
                {
                    self.store.settings.virtual_shifting = on;
                    self.mark_dirty();
                }
                ui.label(
                    RichText::new(if on { "On" } else { "Off" })
                        .font(theme::regular(13.0))
                        .color(Color32::WHITE),
                );
            });
        });
        if self.gears_arg && !self.store.settings.virtual_shifting {
            ui.label(
                RichText::new("On for this run (started with --gears).")
                    .font(theme::regular(12.5))
                    .color(glass::QUIET),
            );
        }
        if !self.gears_on() {
            return;
        }
        let gear = gears::clamp(self.store.settings.gear);
        let n = gears::count();
        let units = self.store.settings.units;
        let kmh = gears::speed(90.0, gear) * 3.6;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.label(
                RichText::new(gears::teeth(gear).replace('x', "×"))
                    .font(theme::medium(30.0))
                    .color(if self.gear_flashing() {
                        theme::ACCENT
                    } else {
                        Color32::WHITE
                    }),
            );
            ui.label(
                RichText::new(format!(
                    "gear {} of {n} · about {:.0} {} at 90 rpm",
                    gear + 1,
                    units.speed(kmh),
                    units.speed_unit()
                ))
                .font(theme::regular(13.0))
                .color(glass::SOFT),
            );
        });

        // Every gear as a bar, as tall as the gear is hard; click one to
        // pick it.
        let labels = ui.available_width() / n as f32 > 34.0;
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), if labels { 92.0 } else { 72.0 }),
            Sense::click(),
        );
        let gap = 4.0;
        let w = (rect.width() - gap * (n - 1) as f32) / n as f32;
        let max = gears::get(n - 1).ratio() as f32;
        let big_ring = gears::all().iter().map(|g| g.ring).max().unwrap_or(0);
        let p = ui.painter();
        let hover = resp
            .hover_pos()
            .map(|pos| (((pos.x - rect.left()) / (w + gap)) as usize).min(n - 1));
        for (i, g) in gears::all().iter().enumerate() {
            let x = rect.left() + i as f32 * (w + gap);
            let h = 12.0 + g.ratio() as f32 / max * 58.0;
            let bottom = rect.top() + 70.0;
            let bar = Rect::from_min_max(Pos2::new(x, bottom - h), Pos2::new(x + w, bottom));
            let fill = if i == gear {
                Color32::from_white_alpha(235)
            } else if hover == Some(i) {
                Color32::from_white_alpha(120)
            } else if g.ring == big_ring {
                Color32::from_white_alpha(82)
            } else {
                Color32::from_white_alpha(46)
            };
            p.rect_filled(bar, CornerRadius::same(4), fill);
            if labels {
                p.text(
                    Pos2::new(x + w / 2.0, bottom + 14.0),
                    Align2::CENTER_CENTER,
                    format!("{}×{}", g.ring, g.cog),
                    theme::regular(10.5),
                    if i == gear {
                        Color32::WHITE
                    } else {
                        glass::QUIET
                    },
                );
            }
        }
        if let (true, Some(i)) = (resp.clicked(), hover.filter(|&i| i != gear)) {
            self.store.settings.gear = i;
            self.gear_flash = Some(Instant::now());
            self.mark_dirty();
        }
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        if let Some(i) = hover {
            resp.on_hover_text(format!(
                "Gear {} of {n}: {}",
                i + 1,
                gears::teeth(i).replace('x', "×")
            ));
        }
        let st = &self.store.settings;
        ui.label(
            RichText::new(format!(
                "{}. Click a gear to choose it; set your own bike's gearing in Settings › Riding.",
                gears::describe(&st.chainrings, &st.cassette)
            ))
            .font(theme::regular(12.5))
            .color(glass::QUIET),
        );
        if !self.has_cadence() {
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "No cadence coming in: pedal to wake the trainer, or pair a cadence sensor. \
                         Until then the trainer follows the road as without gears.",
                    )
                    .font(theme::regular(12.5))
                    .color(Color32::from_rgb(255, 196, 120)),
                )
                .wrap(),
            );
        }
        let clicks = self
            .links
            .values()
            .any(|l| l.link == Link::Connected && l.caps.contains(&Capability::Shifter));
        if clicks {
            ui.horizontal(|ui| {
                let mut on = self.store.settings.click_restart;
                if glass::toggle(ui, &mut on).changed() {
                    self.store.settings.click_restart = on;
                    self.mark_dirty();
                }
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "Keep the Click's left half going without Zwift: when Zwift has it locked, \
                             VeloForge restarts it about once a minute (a pause of a second or two).",
                        )
                        .font(theme::regular(12.5))
                        .color(glass::SOFT),
                    )
                    .wrap(),
                );
            });
        }
        if !clicks {
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "A Zwift Click v2 shows in Nearby as two devices, one per half: connect both. \
                         While Zwift keeps the left half locked, B on the right half shifts down.",
                    )
                    .font(theme::regular(12.5))
                    .color(glass::QUIET),
                )
                .wrap(),
            );
        }
    }

    /// The simulated trainer: what it is, how hard the pretend rider
    /// pedals, and its switch.
    fn simulator_section(&mut self, ui: &mut egui::Ui) {
        let sim_on = self.sim_on();
        let wide = ui.available_width() > 640.0;
        let text = |ui: &mut egui::Ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            glass::heading(ui, "Simulated trainer");
            ui.add(
                egui::Label::new(
                    RichText::new("Try the app without a trainer. Takes over while switched on.")
                        .font(theme::regular(13.0))
                        .color(glass::SOFT),
                )
                .wrap(),
            );
        };
        let controls = |ui: &mut egui::Ui, app: &mut App| {
            let mut on = sim_on;
            if glass::toggle(ui, &mut on).changed() {
                app.dev.send(Cmd::SimEnable(on));
            }
            ui.label(
                RichText::new(if sim_on { "On" } else { "Off" })
                    .font(theme::regular(13.0))
                    .color(Color32::WHITE),
            );
            ui.add_space(14.0);
            // Left to right inside the row, which is laid out from the right.
            ui.allocate_ui_with_layout(
                Vec2::new(300.0, 34.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.label(
                        RichText::new("Rider effort")
                            .font(theme::regular(13.0))
                            .color(glass::SOFT),
                    );
                    ui.spacing_mut().slider_width = 160.0;
                    if ui
                        .add(
                            egui::Slider::new(&mut app.sim_effort, 50.0..=1500.0)
                                .logarithmic(true)
                                .suffix(" W"),
                        )
                        .changed()
                    {
                        app.dev.send(Cmd::SimEffort(app.sim_effort));
                    }
                },
            );
        };
        if wide {
            let width = ui.available_width();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_max_width(width - 420.0);
                    text(ui);
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    controls(ui, self)
                });
            });
        } else {
            text(ui);
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                controls(ui, self)
            });
        }
    }

    /// The log: recent messages, and ways to keep them.
    fn log_section(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            glass::heading(ui, "Log");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if glass::pill(ui, "Open full log", true)
                    .on_hover_text(self.paths.log_file().display().to_string())
                    .clicked()
                {
                    crate::platform::open(self.paths.log_file());
                }
                if glass::pill(ui, "Save to file", true).clicked() {
                    let text: Vec<&str> = self.log.iter().rev().map(|s| s.as_str()).collect();
                    let path = std::env::var_os("HOME")
                        .map(std::path::PathBuf::from)
                        .unwrap_or_default()
                        .join("veloforge-log.txt");
                    self.toast = Some(match std::fs::write(&path, text.join("\n")) {
                        Ok(()) => (format!("Log saved to {}", path.display()), Instant::now()),
                        Err(e) => (format!("Could not save log: {e}"), Instant::now()),
                    });
                }
                if glass::pill(ui, "Copy log", true).clicked() {
                    let text: Vec<&str> = self.log.iter().rev().map(|s| s.as_str()).collect();
                    ui.ctx().copy_text(text.join("\n"));
                    self.toast = Some(("Log copied to clipboard".into(), Instant::now()));
                }
                if ui
                    .checkbox(&mut self.raw_log, "Log raw sensor data")
                    .on_hover_text(
                        "Record every packet from your sensors, useful for diagnosing odd readings",
                    )
                    .changed()
                {
                    self.dev.send(Cmd::RawLog(self.raw_log));
                }
            });
        });
        ui.add_space(4.0);
        egui::Frame::new()
            .fill(Color32::from_black_alpha(70))
            .corner_radius(14)
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .id_salt("log")
                    .max_height(110.0)
                    .show(ui, |ui| {
                        for l in &self.log {
                            ui.label(RichText::new(l).monospace().small().color(glass::SOFT));
                        }
                    });
            });
    }

    /// A device found nearby: an icon, its name, what it is and how
    /// strong its signal is, and a button to connect it.
    fn nearby_row(&mut self, ui: &mut egui::Ui, d: &DeviceInfo) {
        let link = self.links.get(&d.id).map(|l| l.link);
        let connected = matches!(
            link,
            Some(Link::Connected) | Some(Link::Connecting) | Some(Link::Reconnecting)
        );
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 52.0), Sense::hover());
        let has = |c: Capability| d.caps.contains(&c);
        let (icon, kind) = if has(Capability::Shifter) {
            (Glyph::Crank, "Shifter")
        } else if has(Capability::SmartTrainer) || has(Capability::LegacyWahoo) {
            (Glyph::Bolt, "Smart trainer")
        } else if has(Capability::HeartRate) {
            (Glyph::Heart, "Heart rate")
        } else if has(Capability::Power) {
            (Glyph::Bolt, "Power")
        } else if has(Capability::Cadence) {
            (Glyph::Gauge, "Cadence")
        } else {
            (Glyph::Menu(Tab::Devices), "Bluetooth")
        };
        let signal = match d.rssi {
            Some(r) if r >= -65 => " · strong signal",
            Some(r) if r >= -80 => " · good signal",
            Some(_) => " · weak signal",
            None => "",
        };
        let kind = match link {
            Some(Link::Connecting) => "Connecting…".to_string(),
            Some(Link::Reconnecting) => "Reconnecting…".to_string(),
            Some(Link::Connected) => format!("{kind} · connected"),
            _ => format!("{kind}{signal}"),
        };
        {
            let p = ui.painter();
            if connected {
                p.rect_filled(rect, CornerRadius::same(12), Color32::from_white_alpha(31));
            } else if resp.hovered() {
                p.rect_filled(rect, CornerRadius::same(12), Color32::from_white_alpha(20));
            }
            let c = Pos2::new(rect.left() + 29.0, rect.center().y);
            p.circle_filled(c, 17.0, Color32::from_white_alpha(31));
            glass::glyph(p, c, icon, Color32::WHITE);
        }
        let mut btn = Rect::NOTHING;
        let mut clicked = false;
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(Vec2::new(8.0, 0.0)))
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                let r = glass::pill_small(
                    ui,
                    if connected { "Disconnect" } else { "Connect" },
                    !connected,
                );
                btn = r.rect;
                clicked = r.clicked();
            },
        );
        let x = rect.left() + 56.0;
        let clip = ui.painter().with_clip_rect(Rect::from_min_max(
            Pos2::new(x, rect.top()),
            Pos2::new(btn.left() - 6.0, rect.bottom()),
        ));
        clip.text(
            Pos2::new(x, rect.center().y - 9.0),
            Align2::LEFT_CENTER,
            &d.name,
            theme::regular(13.5),
            Color32::WHITE,
        );
        clip.text(
            Pos2::new(x, rect.center().y + 9.0),
            Align2::LEFT_CENTER,
            kind,
            theme::regular(12.0),
            glass::QUIET,
        );
        let mut about: Vec<String> = vec![d.name.clone()];
        about.extend(d.caps.iter().map(|c| c.label().to_string()));
        if let Some(r) = d.rssi {
            about.push(format!("{r} dBm"));
        }
        resp.on_hover_text(about.join(" · "));
        if clicked {
            if connected {
                self.dev.send(Cmd::Disconnect(d.id.clone()));
                self.store.settings.auto_connect.retain(|x| *x != d.id);
                self.mark_dirty();
            } else {
                self.dev.send(Cmd::Connect(d.id.clone()));
            }
        }
        ui.add_space(2.0);
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
