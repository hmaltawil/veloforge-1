//! The Settings tab: a sidebar of pages, each made of a few cards with one
//! setting per row (name and a short explanation on the left, the control
//! on the right).

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum SettingsPage {
    #[default]
    Profile,
    Riding,
    Display,
    Sound,
    Remote,
    Icu,
    Routes,
    Data,
}

impl SettingsPage {
    const ALL: [SettingsPage; 8] = [
        SettingsPage::Profile,
        SettingsPage::Riding,
        SettingsPage::Display,
        SettingsPage::Sound,
        SettingsPage::Remote,
        SettingsPage::Icu,
        SettingsPage::Routes,
        SettingsPage::Data,
    ];

    fn title(self) -> &'static str {
        match self {
            SettingsPage::Profile => "Profile",
            SettingsPage::Riding => "Riding",
            SettingsPage::Display => "Display",
            SettingsPage::Sound => "Sound",
            SettingsPage::Remote => "Phone remote",
            SettingsPage::Icu => "intervals.icu",
            SettingsPage::Routes => "Routes",
            SettingsPage::Data => "Data & about",
        }
    }

    fn blurb(self) -> &'static str {
        match self {
            SettingsPage::Profile => {
                "Your numbers. Workout targets, zones and virtual speed are based on them."
            }
            SettingsPage::Riding => "How the trainer and the ride clock behave.",
            SettingsPage::Display => "Graphics, the ride screen, and the sky in the 3D world.",
            SettingsPage::Sound => "Beeps that tell you an interval is about to change.",
            SettingsPage::Remote => {
                "See your numbers on your phone and start, pause, skip steps or change the \
                 intensity without reaching for the keyboard."
            }
            SettingsPage::Icu => {
                "Bring the rides you plan on intervals.icu into your calendar, and upload your \
                 rides there automatically."
            }
            SettingsPage::Routes => "Routes you imported from GPX files.",
            SettingsPage::Data => "Where your rides and settings are kept.",
        }
    }
}

const LABEL_WIDTH: f32 = 220.0;

/// A titled group of settings.
fn card<R>(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let r = egui::Frame::new()
        .fill(CARD)
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !title.is_empty() {
                ui.label(RichText::new(title).strong().size(15.0));
                ui.add_space(8.0);
            }
            ui.spacing_mut().item_spacing.y = 10.0;
            add(ui)
        })
        .inner;
    ui.add_space(12.0);
    r
}

/// One setting: its name (and a short explanation) on the left, the control
/// on the right.
fn row<R>(ui: &mut egui::Ui, label: &str, hint: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(LABEL_WIDTH, 0.0),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                ui.set_width(LABEL_WIDTH);
                ui.label(label);
                if !hint.is_empty() {
                    ui.label(RichText::new(hint).weak().size(12.5));
                }
            },
        );
        ui.add_space(12.0);
        ui.vertical(add).inner
    })
    .inner
}

fn zone_list(ui: &mut egui::Ui, rows: impl Iterator<Item = (Color32, String)>) {
    for (color, text) in rows {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::new(14.0, 14.0), Sense::hover());
            ui.painter().rect_filled(r, 3.0, color);
            ui.label(text);
        });
    }
}

impl App {
    pub(super) fn settings_tab(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("settings-nav")
            .resizable(false)
            .exact_size(190.0)
            .frame(egui::Frame::new().inner_margin(egui::Margin {
                left: 0,
                right: 14,
                top: 6,
                bottom: 0,
            }))
            .show(ui, |ui| {
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                    ui.spacing_mut().button_padding = Vec2::new(12.0, 8.0);
                    for p in SettingsPage::ALL {
                        let selected = self.settings_page == p;
                        if ui
                            .selectable_label(selected, RichText::new(p.title()).size(15.0))
                            .clicked()
                        {
                            self.settings_page = p;
                        }
                    }
                });
            });

        let page = self.settings_page;
        egui::ScrollArea::vertical()
            .id_salt(("settings-page", page as u8))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(820.0);
                ui.add_space(4.0);
                ui.label(
                    RichText::new(page.title())
                        .size(22.0)
                        .strong()
                        .color(Color32::from_gray(235)),
                );
                ui.label(RichText::new(page.blurb()).color(Color32::from_gray(160)));
                ui.add_space(12.0);
                match page {
                    SettingsPage::Profile => self.settings_profile(ui),
                    SettingsPage::Riding => self.settings_riding(ui),
                    SettingsPage::Display => self.settings_display(ui),
                    SettingsPage::Sound => self.settings_sound(ui),
                    SettingsPage::Remote => self.settings_remote(ui),
                    SettingsPage::Icu => self.settings_icu(ui),
                    SettingsPage::Routes => self.settings_routes(ui),
                    SettingsPage::Data => self.settings_data(ui),
                }
            });
    }

    fn settings_profile(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.store.settings;
        let mut changed = false;
        let mut mass_changed = false;
        card(ui, "You", |ui| {
            changed |= row(
                ui,
                "FTP",
                "Functional threshold power: what you can hold for about an hour. Not sure? Ride the Ramp Test.",
                |ui| {
                    ui.add(
                        egui::DragValue::new(&mut s.ftp)
                            .range(50.0..=600.0)
                            .suffix(" W"),
                    )
                    .changed()
                },
            );
            changed |= row(
                ui,
                "Max heart rate",
                "Your highest heart rate. VeloForge offers to raise it when a ride goes higher.",
                |ui| {
                    ui.add(
                        egui::DragValue::new(&mut s.max_hr)
                            .range(120.0..=230.0)
                            .suffix(" bpm"),
                    )
                    .changed()
                },
            );
            mass_changed |= row(ui, "Your weight", "For W/kg and virtual speed.", |ui| {
                ui.add(
                    egui::DragValue::new(&mut s.rider_kg)
                        .range(30.0..=200.0)
                        .speed(0.1)
                        .suffix(" kg"),
                )
                .changed()
            });
            mass_changed |= row(ui, "Bike weight", "", |ui| {
                ui.add(
                    egui::DragValue::new(&mut s.bike_kg)
                        .range(3.0..=30.0)
                        .speed(0.1)
                        .suffix(" kg"),
                )
                .changed()
            });
            changed |= row(ui, "Jersey colour", "Your rider in the 3D world.", |ui| {
                ui.color_edit_button_srgb(&mut s.jersey).changed()
            });
        });
        if mass_changed {
            self.dev.send(Cmd::SetMass(s.rider_kg + s.bike_kg));
        }
        if changed || mass_changed {
            self.mark_dirty();
        }

        let (ftp, max_hr) = (self.store.settings.ftp, self.store.settings.max_hr);
        card(ui, "Your zones", |ui| {
            ui.label(
                RichText::new("Worked out from your FTP and max heart rate above.")
                    .weak()
                    .size(12.5),
            );
            ui.columns(2, |cols| {
                cols[0].label(RichText::new("Power").strong());
                zone_list(
                    &mut cols[0],
                    [
                        ("Z1 Recovery", 0.0, 0.55),
                        ("Z2 Endurance", 0.56, 0.75),
                        ("Z3 Tempo", 0.76, 0.90),
                        ("Z4 Threshold", 0.91, 1.05),
                        ("Z5 VO2 max", 1.06, 1.20),
                        ("Z6 Anaerobic", 1.21, 1.5),
                    ]
                    .into_iter()
                    .map(|(name, lo, hi)| {
                        (
                            charts::zone_color((lo + hi) / 2.0),
                            format!("{name}: {:.0}–{:.0} W", lo * ftp, hi * ftp),
                        )
                    }),
                );
                cols[1].label(RichText::new("Heart rate").strong());
                zone_list(
                    &mut cols[1],
                    charts::HR_ZONES
                        .iter()
                        .enumerate()
                        .map(|(i, (name, lo, hi))| {
                            (
                                charts::hr_zone_color(i),
                                format!("{name}: {:.0}–{:.0} bpm", lo * max_hr, hi * max_hr),
                            )
                        }),
                );
            });
        });
    }

    fn settings_riding(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.store.settings;
        let mut changed = false;
        card(ui, "Free ride", |ui| {
            changed |= row(
                ui,
                "Trainer difficulty",
                "How much of the road's gradient you feel: 100% is the real hill, 0% feels flat. Also on the Ride screen.",
                |ui| {
                    ui.add(
                        egui::Slider::new(&mut s.difficulty, 0.0..=1.0)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                    )
                    .changed()
                },
            );
        });
        card(ui, "Speed and clock", |ui| {
            changed |= row(
                ui,
                "Speed",
                "Wheel-on trainers like the KICKR SNAP report wheel speed.",
                |ui| {
                    let a = ui
                        .radio_value(
                            &mut s.trainer_speed,
                            true,
                            "From the trainer when it reports one",
                        )
                        .changed();
                    let b = ui
                        .radio_value(
                            &mut s.trainer_speed,
                            false,
                            "Virtual: calculated from power, weight and gradient",
                        )
                        .changed();
                    a || b
                },
            );
            changed |= row(ui, "Auto-pause", "", |ui| {
                ui.checkbox(&mut s.auto_pause, "Pause the clock when you stop pedalling")
                    .changed()
            });
        });
        card(ui, "Advanced: virtual speed", |ui| {
            ui.label(
                RichText::new(
                    "Only used when speed is calculated rather than read from the trainer.",
                )
                .weak()
                .size(12.5),
            );
            changed |= row(
                ui,
                "Aero drag (CdA)",
                "Drag area in m²: about 0.32 on the hoods, 0.25 in a tuck.",
                |ui| {
                    ui.add(
                        egui::DragValue::new(&mut s.cda)
                            .range(0.15..=0.6)
                            .speed(0.005)
                            .max_decimals(3),
                    )
                    .changed()
                },
            );
            changed |= row(
                ui,
                "Rolling resistance (Crr)",
                "About 0.004 for good road tyres.",
                |ui| {
                    ui.add(
                        egui::DragValue::new(&mut s.crr)
                            .range(0.001..=0.02)
                            .speed(0.0002)
                            .max_decimals(4),
                    )
                    .changed()
                },
            );
        });
        if changed {
            self.mark_dirty();
        }
    }

    fn settings_display(&mut self, ui: &mut egui::Ui) {
        let gpu_note = if !self.gpu_capable || self.gpu.error().is_some() {
            Some("Detailed graphics aren't available on this computer, so the simple view is used.")
        } else {
            None
        };
        let s = &mut self.store.settings;
        let mut changed = false;
        card(ui, "3D graphics", |ui| {
            changed |= row(
                ui,
                "Renderer",
                "Choose Simple if the 3D view looks wrong or will not start.",
                |ui| {
                    let a = ui
                        .radio_value(&mut s.gpu_3d, true, "Detailed (uses your graphics card)")
                        .changed();
                    let b = ui
                        .radio_value(
                            &mut s.gpu_3d,
                            false,
                            "Simple (for slow or unsupported graphics)",
                        )
                        .changed();
                    if let Some(note) = gpu_note {
                        ui.colored_label(Color32::from_rgb(240, 150, 80), note);
                    }
                    a || b
                },
            );
            changed |= row(
                ui,
                "Detail",
                "High: sharp shadows, thick grass, smooth edges. Medium halves all three. \
                 Low drops shadows and grass for older graphics cards.",
                |ui| {
                    ui.add_enabled_ui(s.gpu_3d, |ui| {
                        ui.horizontal(|ui| {
                            let mut c = false;
                            for q in crate::gfx::Quality::ALL {
                                c |= ui.radio_value(&mut s.gfx_quality, q, q.label()).changed();
                            }
                            c
                        })
                        .inner
                    })
                    .inner
                },
            );
            changed |= row(
                ui,
                "Frame rate",
                "How often the ride screen is drawn. 30 fps is smooth enough for riding and \
                 halves the work for older graphics cards and laptop batteries.",
                |ui| {
                    ui.horizontal(|ui| {
                        let mut c = false;
                        for (fps, label) in [(30, "30 fps"), (60, "60 fps"), (0, "Screen rate")] {
                            c |= ui.radio_value(&mut s.fps_limit, fps, label).changed();
                        }
                        c
                    })
                    .inner
                },
            );
        });
        card(ui, "Ride screen", |ui| {
            changed |= row(ui, "Show", "Press V while riding to switch.", |ui| {
                ui.horizontal(|ui| {
                    let a = ui.radio_value(&mut s.view_3d, true, "3D view").changed();
                    let b = ui.radio_value(&mut s.view_3d, false, "Dashboard").changed();
                    a || b
                })
                .inner
            });
        });
        card(ui, "Sky", |ui| {
            changed |= row(
                ui,
                "Time of day",
                "Night rides get stars and a bike light.",
                |ui| {
                    ui.horizontal(|ui| {
                        let mut c = false;
                        for t in TimeOfDay::ALL {
                            c |= ui.radio_value(&mut s.time_of_day, t, t.label()).changed();
                        }
                        c
                    })
                    .inner
                },
            );
            changed |= row(
                ui,
                "Weather",
                "Also on the Ride screen, next to 3D view.",
                |ui| {
                    ui.horizontal(|ui| {
                        let mut c = false;
                        for w in Weather::ALL {
                            c |= ui.radio_value(&mut s.weather, w, w.label()).changed();
                        }
                        c
                    })
                    .inner
                },
            );
        });
        if changed {
            self.mark_dirty();
        }
    }

    fn settings_sound(&mut self, ui: &mut egui::Ui) {
        let player = self.sound.player();
        let s = &mut self.store.settings;
        let mut changed = false;
        let mut test = false;
        let mut volume = None;
        card(ui, "Audio cues", |ui| {
            changed |= row(
                ui,
                "Beeps",
                "A 3-2-1 countdown before each interval, a chime at the end, and a warning if the trainer disconnects.",
                |ui| ui.checkbox(&mut s.sounds, "Play audio cues").changed(),
            );
            row(ui, "Volume", "", |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Slider::new(&mut s.volume, 0.05..=1.0)
                                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                        )
                        .changed()
                    {
                        volume = Some(s.volume);
                    }
                    test = ui.button("Test").clicked();
                });
                match &player {
                    Some(p) => ui.label(
                        RichText::new(format!("Playing through {p}"))
                            .weak()
                            .size(12.5),
                    ),
                    None => ui.colored_label(
                        Color32::from_rgb(240, 150, 80),
                        "No audio player found. Install one with: sudo dnf install pipewire-utils",
                    ),
                };
            });
        });
        if let Some(v) = volume {
            self.sound.set_volume(v);
            changed = true;
        }
        if test {
            self.sound.play(Cue::Done);
        }
        if changed {
            self.mark_dirty();
        }
    }

    fn settings_remote(&mut self, ui: &mut egui::Ui) {
        let mut on = self.store.settings.remote_enabled;
        let mut toggled = false;
        card(ui, "", |ui| {
            toggled = ui
                .checkbox(
                    &mut on,
                    RichText::new("Turn on the phone remote").size(15.0),
                )
                .changed();
            ui.label(
                RichText::new("Your phone must be on the same Wi-Fi as this computer.")
                    .weak()
                    .size(12.5),
            );
        });
        if toggled {
            self.store.settings.remote_enabled = on;
            self.mark_dirty();
            if on {
                self.start_remote();
            } else {
                self.remote = None;
                self.remote_error = None;
            }
        }
        if !on {
            return;
        }
        if let Some(e) = self.remote_error.clone() {
            let mut retry = false;
            card(ui, "Couldn't start", |ui| {
                ui.colored_label(Color32::from_rgb(240, 130, 90), e);
                retry = ui.button("Try again").clicked();
            });
            if retry {
                self.start_remote();
            }
            return;
        }
        if self.remote.is_none() {
            return;
        }
        let url = self.remote_url.clone();
        let connected = self.remote.as_ref().is_some_and(|r| r.phone_connected());
        let port = self.store.settings.remote_port;
        let mut rotate = false;
        card(ui, "Connect your phone", |ui| {
            ui.horizontal_top(|ui| {
                qr_code(ui, &url, 4.0);
                ui.add_space(16.0);
                ui.vertical(|ui| {
                    ui.label("Scan this with your phone's camera, or open:");
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(RichText::new(&url).monospace()).selectable(true));
                        if ui.small_button("Copy").clicked() {
                            ui.ctx().copy_text(url.clone());
                        }
                    });
                    if connected {
                        ui.colored_label(Color32::from_rgb(110, 210, 130), "● Phone connected");
                    } else {
                        ui.label(RichText::new("Waiting for your phone…").weak());
                    }
                    rotate = ui
                        .button("New code")
                        .on_hover_text("Make a new link; phones using the old one stop working")
                        .clicked();
                });
            });
        });
        card(ui, "Phone can't connect?", |ui| {
            let (why, cmd) = crate::platform::firewall_hint(port);
            ui.label(RichText::new(why).weak());
            ui.horizontal(|ui| {
                ui.add(
                    egui::Label::new(RichText::new(&cmd).monospace().size(12.5)).selectable(true),
                );
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(cmd.clone());
                }
            });
        });
        if rotate {
            self.store.settings.remote_token = crate::remote::new_token();
            self.mark_dirty();
            self.start_remote();
        }
    }

    fn settings_icu(&mut self, ui: &mut egui::Ui) {
        let mut connect = false;
        let mut disconnect = false;
        card(ui, "Account", |ui| {
            ui.label(
                RichText::new(
                    "Find your athlete ID and API key on intervals.icu under Settings → Developer Settings.",
                )
                .weak()
                .size(12.5),
            );
            row(ui, "Athlete ID", "", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.icu_edit.athlete)
                        .hint_text("e.g. i12345 (or 0)")
                        .desired_width(240.0),
                );
            });
            row(ui, "API key", "Stored only on this computer.", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.icu_edit.key)
                        .password(true)
                        .hint_text("paste your API key")
                        .desired_width(240.0),
                );
            });
            ui.horizontal(|ui| {
                let can = self.icu_edit.is_set() && !self.icu_busy;
                let label = if self.icu_creds.is_set() {
                    "Reconnect"
                } else {
                    "Connect"
                };
                connect = ui.add_enabled(can, egui::Button::new(label)).clicked();
                if self.icu_creds.is_set() {
                    disconnect = ui.button("Disconnect").clicked();
                }
                if self.icu_busy {
                    ui.spinner();
                }
                match &self.icu_status {
                    Some(Ok(m)) => {
                        ui.colored_label(Color32::from_rgb(110, 210, 130), m);
                    }
                    Some(Err(e)) => {
                        ui.colored_label(Color32::from_rgb(240, 130, 90), e);
                    }
                    None => {}
                }
            });
        });
        if connect {
            self.icu_busy = true;
            self.icu_status = None;
            self.icu
                .send(crate::icu::Request::Test(self.icu_edit.clone()));
        }
        if disconnect {
            self.icu_creds = Default::default();
            self.icu_edit = Default::default();
            self.icu_athlete = None;
            self.icu_status = None;
            let _ = std::fs::remove_file(self.paths.credentials_file());
        }
        if !self.icu_creds.is_set() {
            return;
        }
        let icu_ftp = self.icu_athlete.as_ref().and_then(|a| a.ftp);
        let s = &mut self.store.settings;
        let mut changed = false;
        let mut sync_now = false;
        card(ui, "Sync", |ui| {
            changed |= row(
                ui,
                "Planned rides",
                "The next four weeks, checked every 30 minutes.",
                |ui| {
                    let c = ui
                        .checkbox(&mut s.icu_sync_plan, "Add them to my calendar")
                        .changed();
                    sync_now = ui.button("Sync now").clicked();
                    c
                },
            );
            changed |= row(ui, "Finished rides", "", |ui| {
                ui.checkbox(&mut s.icu_upload, "Upload when I press Finish")
                    .changed()
            });
            if let Some(ftp) = icu_ftp.filter(|f| (f - s.ftp).abs() >= 1.0) {
                row(ui, "FTP", "", |ui| {
                    ui.label(format!(
                        "intervals.icu has {ftp:.0} W, VeloForge {:.0} W.",
                        s.ftp
                    ));
                    if ui.button(format!("Use {ftp:.0} W")).clicked() {
                        s.ftp = ftp;
                        changed = true;
                    }
                });
            }
        });
        if changed {
            self.mark_dirty();
        }
        if sync_now {
            self.icu_sync();
        }
    }

    fn settings_routes(&mut self, ui: &mut egui::Ui) {
        let mut remove = None;
        let mut rebuild = false;
        card(ui, "Imported routes", |ui| {
            ui.label(
                RichText::new(
                    "To import a route, drag a .gpx file (from Strava, Komoot, RideWithGPS…) onto the VeloForge window.",
                )
                .weak()
                .size(12.5),
            );
            if self.store.gpx_routes.is_empty() {
                ui.label(RichText::new("No imported routes yet.").italics());
            }
            for (i, g) in self.store.gpx_routes.iter_mut().enumerate() {
                row(ui, &g.name, "", |ui| {
                    ui.horizontal(|ui| {
                        let mut sc = g.scenery.unwrap_or(route::Scenery::Hills);
                        egui::ComboBox::from_id_salt(("scenery", i))
                            .selected_text(sc.label())
                            .show_ui(ui, |ui| {
                                for option in route::Scenery::ALL {
                                    ui.selectable_value(&mut sc, option, option.label());
                                }
                            });
                        if Some(sc) != g.scenery
                            && !(g.scenery.is_none() && sc == route::Scenery::Hills)
                        {
                            g.scenery = Some(sc);
                            rebuild = true;
                        }
                        if ui.small_button("Remove").clicked() {
                            remove = Some(i);
                        }
                    });
                });
            }
        });
        if rebuild {
            self.rebuild_routes();
            self.mark_dirty();
        }
        if let Some(i) = remove {
            self.store.gpx_routes.remove(i);
            self.rebuild_routes();
            self.mark_dirty();
        }
    }

    /// Ride files in the activities folder that no ride in History uses
    /// (left behind by rides deleted before deleting also moved the file).
    fn orphan_activity_files(&self) -> Vec<std::path::PathBuf> {
        let used: std::collections::HashSet<std::path::PathBuf> = self
            .store
            .history
            .iter()
            .filter_map(|h| Path::new(&h.file).canonicalize().ok())
            .collect();
        std::fs::read_dir(&self.paths.activities)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|e| e == "tcx")
                    && p.file_name().is_some_and(|n| n != "in-progress.tcx")
                    && p.canonicalize().is_ok_and(|c| !used.contains(&c))
            })
            .collect()
    }

    fn settings_data(&mut self, ui: &mut egui::Ui) {
        let data = self.paths.data.clone();
        let remembered = self.store.settings.auto_connect.len();
        let orphans = self.orphan_activity_files();
        let mut forget = false;
        let mut clean = false;
        card(ui, "Your data", |ui| {
            row(
                ui,
                "Folder",
                "Settings, calendar, rides (.tcx) and the log. Nothing about you or your rides leaves your computer unless you turn on intervals.icu.",
                |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(data.display().to_string()).monospace())
                            .selectable(true),
                    );
                    if ui.button("Open data folder").clicked() {
                        let _ = std::fs::create_dir_all(&data);
                        crate::platform::open(&data);
                    }
                },
            );
        });
        if !orphans.is_empty() {
            card(ui, "Leftover ride files", |ui| {
                row(
                    ui,
                    &match orphans.len() {
                        1 => "1 file".to_string(),
                        n => format!("{n} files"),
                    },
                    "Ride files in the activities folder that aren't in History any more, from rides you deleted earlier.",
                    |ui| {
                        clean = ui.button(format!("Move them to the {TRASH}")).clicked();
                    },
                );
            });
        }
        card(ui, "Devices", |ui| {
            row(
                ui,
                "Remembered devices",
                "VeloForge reconnects these automatically when it starts.",
                |ui| {
                    ui.label(match remembered {
                        0 => "None".to_string(),
                        1 => "1 device".to_string(),
                        n => format!("{n} devices"),
                    });
                    forget = ui
                        .add_enabled(
                            remembered > 0,
                            egui::Button::new("Forget remembered devices"),
                        )
                        .clicked();
                },
            );
        });
        let mut check_now = false;
        let mut changed = false;
        card(ui, "About", |ui| {
            ui.label(format!(
                "VeloForge {} — free, local indoor cycling.",
                env!("CARGO_PKG_VERSION")
            ));
            ui.hyperlink_to(
                "Source code on GitHub",
                "https://github.com/hmaltawil/veloforge-1",
            );
            row(
                ui,
                "Updates",
                "Asks GitHub once at start-up whether a newer version exists. Nothing about you or your rides is sent.",
                |ui| {
                    changed |= ui
                        .checkbox(
                            &mut self.store.settings.check_updates,
                            "Check for new versions",
                        )
                        .changed();
                    use crate::update::Status;
                    let checking = matches!(self.update, Some(Status::Checking));
                    match &self.update {
                        None => {}
                        Some(Status::Checking) => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Checking…").weak());
                            });
                        }
                        Some(Status::UpToDate) => {
                            ui.label(RichText::new("You have the latest version.").weak());
                        }
                        Some(Status::Available(r)) => {
                            ui.colored_label(
                                ACCENT,
                                format!("VeloForge {} is available.", r.version),
                            );
                            ui.hyperlink_to("Open the download page", &r.url);
                        }
                        Some(Status::Failed(e)) => {
                            ui.colored_label(
                                Color32::from_rgb(240, 150, 80),
                                format!("Could not check: {e}"),
                            );
                        }
                    }
                    check_now = ui
                        .add_enabled(!checking, egui::Button::new("Check now"))
                        .clicked();
                },
            );
        });
        if check_now {
            self.check_for_update();
        }
        if changed {
            self.mark_dirty();
        }
        if clean {
            let mut failed = 0;
            for f in &orphans {
                if crate::activity::move_to_trash(f).is_err() {
                    failed += 1;
                }
            }
            let moved = orphans.len() - failed;
            self.log_line(format!("Moved {moved} leftover ride files to the {TRASH}"));
            self.notify(if failed == 0 {
                format!("Moved {moved} leftover ride files to the {TRASH}")
            } else {
                format!("Moved {moved} files to the {TRASH}; {failed} could not be moved")
            });
        }
        if forget {
            self.store.settings.auto_connect.clear();
            self.dev.send(Cmd::AutoConnect(vec![]));
            self.mark_dirty();
        }
    }
}
