//! The Settings tab: a sidebar of pages, each made of a few cards with one
//! setting per row (name and a short explanation on the left, the control
//! on the right).

use super::*;
use crate::store::HrBasis;
use crate::units::Units;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum SettingsPage {
    #[default]
    Profile,
    Riding,
    Appearance,
    Sound,
    /// Strava, intervals.icu and the phone remote.
    Connections,
    Data,
}

impl SettingsPage {
    const ALL: [SettingsPage; 6] = [
        SettingsPage::Profile,
        SettingsPage::Riding,
        SettingsPage::Appearance,
        SettingsPage::Sound,
        SettingsPage::Connections,
        SettingsPage::Data,
    ];

    fn title(self) -> &'static str {
        match self {
            SettingsPage::Profile => "Profile",
            SettingsPage::Riding => "Riding",
            SettingsPage::Appearance => "Appearance",
            SettingsPage::Sound => "Sound & music",
            SettingsPage::Connections => "Connections",
            SettingsPage::Data => "Data & about",
        }
    }

    fn glyph(self) -> super::glass::Glyph {
        use super::glass::Glyph;
        match self {
            SettingsPage::Profile => Glyph::People,
            SettingsPage::Riding => Glyph::Menu(Tab::Ride),
            SettingsPage::Appearance => Glyph::Image,
            SettingsPage::Sound => Glyph::Bolt,
            SettingsPage::Connections => Glyph::Route,
            SettingsPage::Data => Glyph::Folder,
        }
    }

    fn blurb(self) -> &'static str {
        match self {
            SettingsPage::Profile => {
                "Your numbers. Workout targets, zones and virtual speed are based on them."
            }
            SettingsPage::Riding => "How the trainer and the ride clock behave.",
            SettingsPage::Appearance => {
                "The picture behind every screen, how see-through the panels are, the text \
                 size and the 3D world's quality."
            }
            SettingsPage::Sound => {
                "Your own music while you ride, and the beeps before an interval changes."
            }
            SettingsPage::Connections => {
                "Strava and intervals.icu for your rides and plans, and the phone remote."
            }
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
/// A weight kept in kg, edited in the chosen units.
pub(super) fn weight_value(
    ui: &mut egui::Ui,
    kg: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    units: Units,
) -> bool {
    let mut w = units.weight(*kg as f64);
    let lo = units.weight(*range.start() as f64);
    let hi = units.weight(*range.end() as f64);
    let changed = ui
        .add(
            egui::DragValue::new(&mut w)
                .range(lo..=hi)
                .speed(0.1)
                .fixed_decimals(1)
                .suffix(format!(" {}", units.weight_unit())),
        )
        .changed();
    if changed {
        *kg = units.weight_to_kg(w) as f32;
    }
    changed
}

fn row<R>(ui: &mut egui::Ui, label: &str, hint: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    // On a narrow page the labels' column gives way to the controls.
    let label_w = LABEL_WIDTH.min(ui.available_width() * 0.4);
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(label_w, 0.0),
            egui::Layout::top_down(egui::Align::LEFT),
            |ui| {
                ui.set_width(label_w);
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
        use super::glass::{self, Lead};
        let pane = ui.max_rect().expand2(Vec2::new(28.0, 26.0));
        glass::area(ui, pane, "settings", |ui| {
            let (side, main) = glass::split(ui);
            if let Some(side) = side {
                glass::area(ui, side, "settings-side", |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    let st = &self.store.settings;
                    let meta = format!(
                        "FTP {:.0} W · {:.0} kg · {:.1} W/kg",
                        st.ftp,
                        st.rider_kg,
                        st.ftp / st.rider_kg.max(1.0)
                    );
                    if glass::side_row(ui, false, Lead::Tile("YOU", "R"), "Profile", &meta)
                        .clicked()
                    {
                        self.settings_page = SettingsPage::Profile;
                    }
                    ui.add_space(8.0);
                    for p in SettingsPage::ALL {
                        if glass::side_row(
                            ui,
                            self.settings_page == p,
                            Lead::Icon(p.glyph()),
                            p.title(),
                            "",
                        )
                        .clicked()
                        {
                            self.settings_page = p;
                        }
                    }
                });
            }
            glass::area(ui, main, "settings-main", |ui| {
                let page = self.settings_page;
                if side.is_none() {
                    ui.horizontal_wrapped(|ui| {
                        for p in SettingsPage::ALL {
                            if glass::pill(ui, p.title(), self.settings_page == p).clicked() {
                                self.settings_page = p;
                            }
                        }
                    });
                }
                glass::title(ui, page.title());
                ui.label(RichText::new(page.blurb()).color(glass::QUIET));
                ui.add_space(12.0);
                // The page is as wide as the pane (at most 860 points): rows
                // wrap rather than push the cards past its edge.
                let width = ui.max_rect().width().min(860.0);
                egui::ScrollArea::vertical()
                    .id_salt(("settings-page", page as u8))
                    .auto_shrink([false, false])
                    .max_width(width + 12.0)
                    .show(ui, |ui| {
                        ui.set_width(width);
                        match page {
                            SettingsPage::Profile => self.settings_profile(ui),
                            SettingsPage::Riding => self.settings_riding(ui),
                            SettingsPage::Appearance => {
                                self.settings_appearance(ui);
                                ui.add_space(14.0);
                                self.settings_display(ui);
                            }
                            SettingsPage::Sound => {
                                self.settings_music(ui);
                                ui.add_space(14.0);
                                glass::heading(ui, "Workout beeps");
                                self.settings_sound(ui);
                            }
                            SettingsPage::Connections => {
                                glass::heading(ui, "Strava");
                                self.settings_strava(ui);
                                ui.add_space(14.0);
                                glass::heading(ui, "intervals.icu");
                                self.settings_icu(ui);
                                ui.add_space(14.0);
                                glass::heading(ui, "Phone remote");
                                self.settings_remote(ui);
                            }
                            SettingsPage::Data => self.settings_data(ui),
                        }
                    });
            });
        });
    }

    /// The picture behind the screens (Waves, or pictures you add), how
    /// dark the glass is, how much the picture is dimmed, and text size.
    fn settings_appearance(&mut self, ui: &mut egui::Ui) {
        use super::glass;
        glass::heading(ui, "Background");
        ui.add_space(6.0);
        let mut choose = None;
        let mut remove = None;
        let mut add = false;
        let mut tiles: Vec<(String, String)> =
            vec![(String::new(), "Waves · a colour per section".into())];
        tiles.extend(self.store.settings.pictures.iter().map(|p| {
            (
                p.clone(),
                p.rsplit_once('.').map(|x| x.0).unwrap_or(p).to_string(),
            )
        }));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(14.0, 14.0);
            let size = Vec2::new(168.0, 105.0);
            for (file, name) in &tiles {
                let tex = self.picture_thumb(ui.ctx(), file);
                let chosen = &self.store.settings.wallpaper == file;
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
                    if let Some(t) = tex {
                        ui.painter().add(
                            egui::epaint::RectShape::filled(
                                r,
                                CornerRadius::same(16),
                                Color32::WHITE,
                            )
                            .with_texture(t, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))),
                        );
                    } else {
                        ui.painter().rect_filled(
                            r,
                            CornerRadius::same(16),
                            Color32::from_white_alpha(20),
                        );
                    }
                    if chosen {
                        ui.painter().rect_stroke(
                            r.expand(3.0),
                            CornerRadius::same(19),
                            Stroke::new(2.0, Color32::WHITE),
                            egui::StrokeKind::Inside,
                        );
                        let c = Pos2::new(r.right() - 18.0, r.top() + 18.0);
                        ui.painter()
                            .circle_filled(c, 11.0, Color32::from_white_alpha(235));
                        glass::glyph(
                            ui.painter(),
                            c,
                            glass::Glyph::Check,
                            Color32::from_rgb(28, 29, 33),
                        );
                    }
                    if resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        choose = Some(file.clone());
                    }
                    ui.allocate_ui(Vec2::new(size.x, 18.0), |ui| {
                        ui.set_width(size.x);
                        ui.add(
                            egui::Label::new(
                                RichText::new(name)
                                    .font(if chosen {
                                        theme::medium(13.0)
                                    } else {
                                        theme::regular(13.0)
                                    })
                                    .color(Color32::WHITE),
                            )
                            .truncate(),
                        );
                    });
                    if !file.is_empty()
                        && ui
                            .add(
                                egui::Label::new(
                                    RichText::new("Remove")
                                        .font(theme::regular(12.5))
                                        .color(glass::ALERT),
                                )
                                .sense(Sense::click()),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                    {
                        remove = Some(file.clone());
                    }
                });
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                let (r, resp) = ui.allocate_exact_size(size, Sense::click());
                ui.painter()
                    .rect_filled(r, CornerRadius::same(16), Color32::from_white_alpha(15));
                ui.painter().rect_stroke(
                    r,
                    CornerRadius::same(16),
                    Stroke::new(1.5, Color32::from_white_alpha(128)),
                    egui::StrokeKind::Inside,
                );
                glass::glyph(
                    ui.painter(),
                    r.center() - Vec2::new(0.0, 8.0),
                    glass::Glyph::Plus,
                    Color32::WHITE,
                );
                ui.painter().text(
                    r.center() + Vec2::new(0.0, 16.0),
                    Align2::CENTER_CENTER,
                    "JPG or PNG",
                    theme::regular(12.0),
                    glass::SOFT,
                );
                if resp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    add = true;
                }
                ui.label(RichText::new("Your own picture…").color(Color32::WHITE));
            });
        });
        if let Some(f) = choose {
            self.store.settings.wallpaper = f;
            self.mark_dirty();
        }
        if let Some(f) = remove {
            let _ = std::fs::remove_file(self.paths.pictures().join(&f));
            self.store.settings.pictures.retain(|p| p != &f);
            if self.store.settings.wallpaper == f {
                self.store.settings.wallpaper.clear();
            }
            self.picture_thumbs.remove(&f);
            self.mark_dirty();
        }
        if add {
            self.add_picture();
        }

        ui.add_space(16.0);
        glass::heading(ui, "Glass");
        ui.add_space(4.0);
        let s = &mut self.store.settings;
        let mut changed = false;
        changed |= row(
            ui,
            "Panel darkness",
            "Darker panels read better over bright pictures.",
            |ui| {
                ui.add(egui::Slider::new(&mut s.panel_darkness, 30..=85).suffix("%"))
                    .changed()
            },
        );
        changed |= row(ui, "Dim the picture", "", |ui| {
            ui.add(egui::Slider::new(&mut s.dim_picture, 0..=60).suffix("%"))
                .changed()
        });
        changed |= row(
            ui,
            "Text size",
            "For reading from the bike, or on a TV across the room.",
            |ui| {
                // The nearest of the four sizes, in tenths so they compare.
                let mut size = [10, 12, 14, 17]
                    .into_iter()
                    .min_by_key(|t| (s.text_size * 10.0 - *t as f32).abs() as i32)
                    .unwrap_or(10);
                let c = segmented(
                    ui,
                    &mut size,
                    &[
                        (10, "Default", "Standard size"),
                        (12, "Large", "Everything 20% larger"),
                        (14, "Larger", "Everything 40% larger"),
                        (17, "TV", "Everything 70% larger, for a TV across the room"),
                    ],
                );
                if c {
                    s.text_size = size as f32 / 10.0;
                }
                c
            },
        );
        changed |= row(
            ui,
            "Numbers",
            "How heavy the figures are while riding.",
            |ui| {
                segmented(
                    ui,
                    &mut s.bold_figures,
                    &[
                        (false, "Light", "Thin figures, as in the mock-ups"),
                        (true, "Bold", "Heavy figures, easier to read from far away"),
                    ],
                )
            },
        );
        changed |= row(
            ui,
            "Quote on Home",
            "A line about cycling under the greeting, a new one each day.",
            |ui| {
                segmented(
                    ui,
                    &mut s.home_quote,
                    &[
                        (true, "Show", "A quote that suits the day's plan"),
                        (false, "Hide", "Just the greeting"),
                    ],
                )
            },
        );
        if changed {
            self.mark_dirty();
        }
    }

    /// Your bike's gearing for virtual shifting: one or two chainrings, the
    /// wheel, the cassette (a common one or your own), and the gears they
    /// give.
    fn settings_gearing(&mut self, ui: &mut egui::Ui) {
        use super::glass;
        use crate::gears;
        let before = gears::get(self.store.settings.gear).ratio();
        let mut changed = false;
        let units = self.store.settings.units;
        card(ui, "Your gearing", |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "For virtual shifting: set it up like your own bike and the gears feel the same.",
                    )
                    .color(glass::QUIET),
                )
                .wrap(),
            );
            ui.add_space(4.0);
            let s = &mut self.store.settings;
            let text = &mut self.cassette_text;
            if ui.available_width() > 640.0 {
                ui.columns(2, |cols| {
                    changed |= gearing_front(&mut cols[0], s);
                    changed |= gearing_cassette(&mut cols[1], s, text);
                });
            } else {
                changed |= gearing_front(ui, s);
                ui.add_space(6.0);
                changed |= gearing_cassette(ui, s, text);
            }
            if changed {
                super::apply_gearing(s);
                s.gear = gears::nearest(&gears::all(), before);
            }
            ui.add_space(4.0);
            ui.separator();
            gearing_result(ui, s.gear, units, &s.chainrings, &s.cassette);
        });
        if changed {
            self.mark_dirty();
        }
    }

    /// Ask for a picture, copy it into the data folder and show it.
    fn add_picture(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Choose a picture for the background")
            .add_filter("Picture", &["jpg", "jpeg", "png"])
            .pick_file()
        else {
            return;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            self.notify("Could not read that picture");
            return;
        };
        if crate::backdrop::decode(&bytes).is_none() {
            self.notify("That file is not a JPEG or PNG picture VeloForge can read");
            return;
        }
        let dir = self.paths.pictures();
        let _ = std::fs::create_dir_all(&dir);
        let base = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "picture.jpg".into());
        let mut name = base.clone();
        let mut n = 2;
        while dir.join(&name).exists() {
            name = format!("{n}-{base}");
            n += 1;
        }
        if let Err(e) = std::fs::write(dir.join(&name), &bytes) {
            self.notify(format!("Could not keep the picture: {e}"));
            return;
        }
        self.store.settings.pictures.push(name.clone());
        self.store.settings.wallpaper = name;
        self.mark_dirty();
    }

    /// A small copy of a background picture for its tile ("" is Waves).
    fn picture_thumb(&mut self, ctx: &egui::Context, file: &str) -> Option<egui::TextureId> {
        if !self.picture_thumbs.contains_key(file) {
            let bytes = if file.is_empty() {
                Some(crate::backdrop::WAVES.to_vec())
            } else {
                std::fs::read(self.paths.pictures().join(file)).ok()
            };
            let thumb = bytes
                .and_then(|b| crate::backdrop::decode(&b))
                .map(|p| crate::backdrop::shrink(&p, 340))
                .map(|p| {
                    ctx.load_texture(
                        format!("thumb-{file}"),
                        egui::ColorImage::from_rgba_unmultiplied(p.size, &p.rgba),
                        egui::TextureOptions::LINEAR,
                    )
                });
            self.picture_thumbs.insert(file.to_string(), thumb);
        }
        self.picture_thumbs
            .get(file)
            .and_then(|t| t.as_ref().map(|t| t.id()))
    }

    fn settings_profile(&mut self, ui: &mut egui::Ui) {
        self.rider_card(ui);
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
            changed |= row(
                ui,
                "Threshold heart rate",
                "The heart rate you can hold for about an hour (LTHR). The 20-minute FTP test \
                 measures it with a heart rate strap, or enter one you know.",
                |ui| {
                    ui.add(
                        egui::DragValue::new(&mut s.lthr)
                            .range(0.0..=220.0)
                            .custom_formatter(|v, _| {
                                if v < 1.0 {
                                    "not set".into()
                                } else {
                                    format!("{v:.0} bpm")
                                }
                            })
                            .custom_parser(|t| {
                                let t = t.trim().trim_end_matches("bpm").trim();
                                if t.is_empty() || t.starts_with("not") {
                                    Some(0.0)
                                } else {
                                    t.parse().ok()
                                }
                            }),
                    )
                    .changed()
                },
            );
            changed |= row(
                ui,
                "Heart rate zones from",
                "Max heart rate: 50 to 100% of it. Threshold heart rate (needs one above): \
                 Coggan's cycling zones, which follow your fitness more closely.",
                |ui| {
                    ui.horizontal(|ui| {
                        let mut c = ui
                            .radio_value(&mut s.hr_basis, HrBasis::Max, "Max heart rate")
                            .changed();
                        c |= ui
                            .add_enabled_ui(s.lthr > 0.0, |ui| {
                                ui.radio_value(
                                    &mut s.hr_basis,
                                    HrBasis::Threshold,
                                    "Threshold heart rate",
                                )
                            })
                            .inner
                            .changed();
                        c
                    })
                    .inner
                },
            );
            let units = s.units;
            mass_changed |= row(ui, "Your weight", "For W/kg and virtual speed.", |ui| {
                weight_value(ui, &mut s.rider_kg, 30.0..=200.0, units)
            });
            mass_changed |= row(ui, "Bike weight", "", |ui| {
                weight_value(ui, &mut s.bike_kg, 3.0..=30.0, units)
            });
        });
        if mass_changed {
            self.dev.send(Cmd::SetMass(s.rider_kg + s.bike_kg));
        }
        if changed || mass_changed {
            self.mark_dirty();
        }

        let set = &self.store.settings;
        let ftp = set.ftp;
        let hr_zones = set.hr_zones();
        let basis = match set.hr_basis {
            HrBasis::Threshold if set.lthr > 0.0 => "threshold heart rate",
            _ => "max heart rate",
        };
        card(ui, "Your zones", |ui| {
            ui.label(
                RichText::new(format!("Worked out from your FTP and {basis} above."))
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
                    charts::HR_ZONE_NAMES.iter().enumerate().map(|(i, name)| {
                        let (lo, hi) = hr_zones.range(i);
                        (
                            charts::hr_zone_color(i),
                            format!("{name}: {lo:.0}–{hi:.0} bpm"),
                        )
                    }),
                );
            });
        });
    }

    fn settings_riding(&mut self, ui: &mut egui::Ui) {
        self.settings_gearing(ui);
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
                "Units",
                "How speed, distance, climbing and weight are shown.",
                |ui| {
                    let a = ui
                        .radio_value(&mut s.units, Units::Metric, "Metric (km/h, km, m, kg)")
                        .changed();
                    let b = ui
                        .radio_value(&mut s.units, Units::Imperial, "Imperial (mph, mi, ft, lb)")
                        .changed();
                    a || b
                },
            );
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
            changed |= row(ui, "Auto-start", "", |ui| {
                ui.checkbox(
                    &mut s.start_by_pedalling,
                    "Start the ride 10 s after you start pedalling",
                )
                .changed()
            });
            changed |= row(
                ui,
                "Drafting",
                "Close behind the pacer or a ghost you push less air: up to a third less \
                 at half a metre, fading out by 15 m. Your speed and the trainer follow.",
                |ui| {
                    ui.checkbox(&mut s.drafting, "Riding behind someone saves effort")
                        .changed()
                },
            );
            changed |= row(
                ui,
                "Workouts",
                "The trainer always holds a workout's power. On the flat, the route's hills \
                 don't change your speed, distance or climbing either, so rides of the same \
                 workout compare. Free-ride steps then ride flat too.",
                |ui| {
                    ui.checkbox(&mut s.flat_workouts, "Ride workouts on the flat")
                        .changed()
                },
            );
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
                 Low drops shadows and grass, for older graphics cards (and, with Resolution \
                 on Automatic, draws the world at a lower resolution).",
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
                "Resolution",
                "How sharply the 3D world is drawn. Lower is much lighter work for the \
                 graphics card; the numbers and panels stay sharp. Automatic: full on High and \
                 Medium detail, two thirds on Low.",
                |ui| {
                    ui.add_enabled_ui(s.gpu_3d, |ui| {
                        ui.horizontal(|ui| {
                            let mut c = false;
                            for (pct, label) in
                                [(0, "Automatic"), (100, "100%"), (75, "75%"), (50, "50%")]
                            {
                                c |= ui.radio_value(&mut s.resolution, pct, label).changed();
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
            changed |= row(
                ui,
                "Screen",
                "Pedalling isn't keyboard or mouse input, so without this the screen can \
                 dim, blank or lock, or the computer sleep, mid-ride. Only while a ride is \
                 running: pausing, finishing or ten minutes without pedalling lets them \
                 sleep as usual.",
                |ui| {
                    ui.checkbox(&mut s.keep_awake, "Keep the screen awake while riding")
                        .changed()
                },
            );
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

    fn settings_strava(&mut self, ui: &mut egui::Ui) {
        let mut connect = false;
        let mut disconnect = false;
        let connected = self.strava_creds.is_connected();
        card(ui, "Connect", |ui| {
            if !connected {
                ui.label(
                    RichText::new(
                        "Strava lets each person connect apps through their own free Strava \
                         API application. Setting it up takes two minutes, once:",
                    )
                    .weak()
                    .size(12.5),
                );
                ui.add_space(4.0);
                for (i, step) in [
                    "Open strava.com/settings/api (the button below) and sign in.",
                    "Fill in the form: any name (e.g. VeloForge), category Training, any \
                     website (e.g. https://github.com/hmaltawil/veloforge-1), and \
                     Authorization Callback Domain: localhost. Upload any icon.",
                    "Copy the Client ID and Client Secret it shows into the boxes below.",
                    "Press Connect, then Authorize on the Strava page that opens.",
                ]
                .iter()
                .enumerate()
                {
                    ui.label(RichText::new(format!("{}. {step}", i + 1)).size(12.5));
                }
                ui.add_space(4.0);
                if ui.button("Open strava.com/settings/api").clicked() {
                    crate::platform::open(crate::strava::API_SETTINGS);
                }
                ui.add_space(6.0);
            }
            row(ui, "Client ID", "", |ui| {
                ui.add_enabled(
                    !connected,
                    egui::TextEdit::singleline(&mut self.strava_edit.client_id)
                        .hint_text("e.g. 123456")
                        .desired_width(240.0),
                );
            });
            row(ui, "Client Secret", "Stored only on this computer.", |ui| {
                ui.add_enabled(
                    !connected,
                    egui::TextEdit::singleline(&mut self.strava_edit.client_secret)
                        .password(true)
                        .hint_text("paste the Client Secret")
                        .desired_width(240.0),
                );
            });
            ui.horizontal(|ui| {
                if connected {
                    disconnect = ui.button("Disconnect").clicked();
                } else {
                    let can = self.strava_edit.has_app() && !self.strava_busy;
                    connect = ui.add_enabled(can, egui::Button::new("Connect")).clicked();
                }
                if self.strava_busy {
                    ui.spinner();
                    ui.label("Waiting for you to press Authorize on Strava…");
                }
                match &self.strava_status {
                    Some(Ok(m)) => {
                        ui.colored_label(Color32::from_rgb(110, 210, 130), m);
                    }
                    Some(Err(e)) => {
                        ui.colored_label(Color32::from_rgb(240, 130, 90), e);
                    }
                    None if connected => {
                        let who = &self.strava_creds.athlete;
                        ui.colored_label(
                            Color32::from_rgb(110, 210, 130),
                            if who.is_empty() {
                                "Connected".to_string()
                            } else {
                                format!("Connected as {who}")
                            },
                        );
                    }
                    None => {}
                }
            });
        });
        if connect {
            self.strava_busy = true;
            self.strava_status = None;
            self.strava
                .send(crate::strava::Request::Connect(self.strava_edit.clone()));
        }
        if disconnect {
            // Keep the app's ID and secret for reconnecting; forget the tokens.
            self.strava_creds = crate::strava::Credentials {
                client_id: self.strava_creds.client_id.clone(),
                client_secret: self.strava_creds.client_secret.clone(),
                ..Default::default()
            };
            self.strava_edit = self.strava_creds.clone();
            self.strava_status = None;
            let _ = self.strava_creds.save(&self.paths.strava_file());
        }
        if !self.strava_creds.is_connected() {
            return;
        }
        let s = &mut self.store.settings;
        let mut changed = false;
        card(ui, "Uploads", |ui| {
            changed |= row(
                ui,
                "Finished rides",
                "Sent as FIT files and marked as indoor (trainer) rides.",
                |ui| {
                    ui.checkbox(&mut s.strava_upload, "Upload when I press Finish")
                        .changed()
                },
            );
        });
        if changed {
            self.mark_dirty();
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

    pub(super) fn settings_routes(&mut self, ui: &mut egui::Ui) {
        let mut make = false;
        let mut forget = None;
        let mut changed = false;
        let units = self.store.settings.units;
        card(ui, "Random routes", |ui| {
            ui.label(
                RichText::new(
                    "A new road each time, with its own bends, hills, farms and forest. Make one here or from the route menu on the Ride tab (+ New random route).",
                )
                .weak()
                .size(12.5),
            );
            let r = &mut self.store.settings.random_route;
            changed |= row(ui, "Scenery", "", |ui| {
                let mut c = false;
                egui::ComboBox::from_id_salt("random-scenery")
                    .selected_text(r.scenery.label())
                    .show_ui(ui, |ui| {
                        for option in route::Scenery::ALL {
                            c |= ui
                                .selectable_value(&mut r.scenery, option, option.label())
                                .changed();
                        }
                    });
                c
            });
            changed |= row(ui, "Length", "", |ui| {
                let mut km = r.km as f64;
                let lo = *crate::routegen::KM_RANGE.start() as f64;
                let hi = *crate::routegen::KM_RANGE.end() as f64;
                let c = ui
                    .add(
                        egui::Slider::new(&mut km, lo..=hi)
                            .step_by(1.0)
                            .custom_formatter(|v, _| units.route_name(&format!("{v:.0} km"))),
                    )
                    .changed();
                r.km = km.round() as u32;
                c
            });
            changed |= row(
                ui,
                "Shape",
                "A loop is ridden lap after lap. Point to point finishes somewhere else; the road carries on past the finish if your workout is still going.",
                |ui| {
                    let mut c = false;
                    ui.horizontal(|ui| {
                        for shape in route::Shape::ALL {
                            c |= ui
                                .selectable_value(&mut r.shape, shape, shape.label())
                                .changed();
                        }
                    });
                    c
                },
            );
            changed |= row(
                ui,
                "Hills",
                "Flat: under 2 %. Rolling: up to 5 %. Hilly: up to 8.5 %. Mountains: long climbs up to 10 %.",
                |ui| {
                    let mut c = false;
                    ui.horizontal(|ui| {
                        for h in crate::routegen::Hills::ALL {
                            c |= ui.selectable_value(&mut r.hills, h, h.label()).changed();
                        }
                    });
                    c
                },
            );
            row(ui, "", "", |ui| {
                if ui.button("Make a new route and ride it").clicked() {
                    make = true;
                }
            });
            if !self.store.random_routes.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new("Made so far").strong());
            }
            for (i, spec) in self.store.random_routes.iter().enumerate() {
                let id = spec.id();
                let name = self
                    .routes
                    .iter()
                    .find(|r| r.base_id() == id)
                    .map(|r| units.route_name(&r.name))
                    .unwrap_or_default();
                row(ui, &name, spec.scenery.label(), |ui| {
                    if ui.small_button("Remove").clicked() {
                        forget = Some(i);
                    }
                });
            }
        });
        if changed {
            self.mark_dirty();
        }
        if make {
            self.new_random_route();
            self.tab = super::Tab::Ride;
        }
        if let Some(i) = forget {
            self.store.random_routes.remove(i);
            self.rebuild_routes();
            self.mark_dirty();
        }
        let mut remove = None;
        let mut rebuild = false;
        let mut renamed = None;
        let mut pick = false;
        let picking = self.picking.is_some();
        card(ui, "Imported routes", |ui| {
            ui.label(
                RichText::new(
                    "Ride a real route: import a .gpx file from Strava, Komoot, RideWithGPS… (or drop one onto the window). A climb or any route that ends away from its start is ridden point to point.",
                )
                .weak()
                .size(12.5),
            );
            if ui
                .add_enabled(!picking, egui::Button::new("Import a GPX route…"))
                .clicked()
            {
                pick = true;
            }
            if self.store.gpx_routes.is_empty() {
                ui.label(RichText::new("No imported routes yet.").italics());
            }
            for (i, g) in self.store.gpx_routes.iter_mut().enumerate() {
                ui.horizontal_top(|ui| {
                    // Its name, to change as you like.
                    if ui
                        .add(egui::TextEdit::singleline(&mut g.name).desired_width(LABEL_WIDTH))
                        .on_hover_text("The route's name")
                        .changed()
                    {
                        renamed = Some((g.id.clone(), g.name.clone()));
                    }
                    ui.add_space(12.0);
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
                        let mut shape = g.shape();
                        egui::ComboBox::from_id_salt(("shape", i))
                            .selected_text(shape.label())
                            .show_ui(ui, |ui| {
                                for option in route::Shape::ALL {
                                    ui.selectable_value(&mut shape, option, option.label());
                                }
                            });
                        if shape != g.shape() {
                            g.shape = Some(shape);
                            rebuild = true;
                        }
                        if ui.small_button("Remove").clicked() {
                            remove = Some(i);
                        }
                    });
                });
            }
        });
        if pick {
            self.pick_file("Import a route", &[("Routes (.gpx)", &["gpx"])]);
        }
        if let Some((id, name)) = renamed {
            for r in self.routes.iter_mut().filter(|r| r.base_id() == id) {
                r.name = if r.is_reversed() {
                    route::reversed_name(&name, None)
                } else {
                    name.clone()
                };
            }
            self.mark_dirty();
        }
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

/// The chainrings (one or two, with their teeth) and the wheel.
fn gearing_front(ui: &mut egui::Ui, s: &mut crate::store::Settings) -> bool {
    use super::glass;
    let mut changed = false;
    ui.label(RichText::new("Chainrings").color(glass::SOFT));
    let mut two = s.chainrings.len() > 1;
    if segmented(
        ui,
        &mut two,
        &[
            (
                false,
                "1× one ring",
                "One chainring, as on most gravel bikes",
            ),
            (true, "2× two rings", "A small and a big chainring"),
        ],
    ) {
        s.chainrings = if two { vec![34, 50] } else { vec![40] };
        changed = true;
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        let names: &[&str] = if s.chainrings.len() > 1 {
            &["Small ring", "Big ring"]
        } else {
            &["Ring"]
        };
        for (i, name) in names.iter().enumerate() {
            let Some(teeth) = s.chainrings.get_mut(i) else {
                continue;
            };
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(RichText::new(*name).color(glass::QUIET).size(12.5));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if glass::round(ui, 30.0, glass::Glyph::Minus, false, "Fewer teeth").clicked()
                        && *teeth > 20
                    {
                        *teeth -= 1;
                        changed = true;
                    }
                    changed |= ui
                        .add(
                            egui::DragValue::new(teeth)
                                .range(20..=60)
                                .suffix(" T")
                                .speed(0.2),
                        )
                        .changed();
                    if glass::round(ui, 30.0, glass::Glyph::Plus, false, "More teeth").clicked()
                        && *teeth < 60
                    {
                        *teeth += 1;
                        changed = true;
                    }
                });
            });
        }
    });
    ui.add_space(8.0);
    ui.label(RichText::new("Wheel and tyre").color(glass::SOFT));
    egui::ComboBox::from_id_salt("tyre")
        .selected_text(s.tyre.as_str())
        .show_ui(ui, |ui| {
            for (name, _) in crate::gears::TYRES {
                if ui.selectable_label(s.tyre == name, name).clicked() && s.tyre != name {
                    s.tyre = name.to_string();
                    changed = true;
                }
            }
        });
    ui.add(
        egui::Label::new(
            RichText::new(
                "Bigger tyres roll further each turn: the same gear goes a little faster.",
            )
            .size(12.0)
            .color(glass::QUIET),
        )
        .wrap(),
    );
    changed
}

/// The cassette: a common one from the list, or your own typed in.
fn gearing_cassette(
    ui: &mut egui::Ui,
    s: &mut crate::store::Settings,
    text: &mut Option<String>,
) -> bool {
    use super::glass;
    let mut changed = false;
    ui.label(RichText::new("Cassette").color(glass::SOFT));
    egui::ScrollArea::vertical()
        .id_salt("cassettes")
        .max_height(262.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (name, cogs) in crate::gears::CASSETTES {
                let on = s.cassette == cogs;
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), Sense::click());
                let p = ui.painter();
                if on {
                    p.rect_filled(rect, CornerRadius::same(12), Color32::from_white_alpha(36));
                } else if resp.hovered() {
                    p.rect_filled(rect, CornerRadius::same(12), Color32::from_white_alpha(26));
                }
                let dot = Pos2::new(rect.left() + 20.0, rect.center().y);
                if on {
                    p.circle_stroke(dot, 6.0, Stroke::new(4.0, Color32::WHITE));
                } else {
                    p.circle_stroke(dot, 7.0, Stroke::new(1.5, Color32::from_white_alpha(128)));
                }
                let count = format!("{} cogs", cogs.len());
                let right = p.text(
                    Pos2::new(rect.right() - 12.0, rect.center().y),
                    Align2::RIGHT_CENTER,
                    &count,
                    theme::regular(12.0),
                    glass::QUIET,
                );
                let clip = p.with_clip_rect(Rect::from_min_max(
                    Pos2::new(rect.left() + 36.0, rect.top()),
                    Pos2::new(right.left() - 8.0, rect.bottom()),
                ));
                clip.text(
                    Pos2::new(rect.left() + 36.0, rect.center().y - 8.0),
                    Align2::LEFT_CENTER,
                    name,
                    if on {
                        theme::semibold(13.5)
                    } else {
                        theme::regular(13.5)
                    },
                    Color32::WHITE,
                );
                clip.text(
                    Pos2::new(rect.left() + 36.0, rect.center().y + 9.0),
                    Align2::LEFT_CENTER,
                    cogs.iter()
                        .map(|c| c.to_string())
                        .collect::<Vec<_>>()
                        .join("-"),
                    theme::regular(11.5),
                    glass::QUIET,
                );
                if resp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                    && !on
                {
                    s.cassette = cogs.to_vec();
                    *text = None;
                    changed = true;
                }
            }
        });
    ui.add_space(4.0);
    ui.label(
        RichText::new("Or type your own, smallest to biggest")
            .size(12.5)
            .color(glass::QUIET),
    );
    let preset = crate::gears::CASSETTES.iter().any(|c| s.cassette == c.1);
    let t = text.get_or_insert_with(|| {
        if preset {
            String::new()
        } else {
            s.cassette
                .iter()
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join("-")
        }
    });
    if ui
        .add(
            egui::TextEdit::singleline(t)
                .hint_text("e.g. 10-11-12-13-15-17-19-21-24-28-32-38-46")
                .desired_width(f32::INFINITY),
        )
        .changed()
    {
        let mut cogs: Vec<u8> = t
            .split(|c: char| !c.is_ascii_digit())
            .filter_map(|x| x.parse().ok())
            .filter(|&c| (8..=60).contains(&c))
            .collect();
        cogs.sort_unstable();
        cogs.dedup();
        if cogs.len() >= 2 && cogs != s.cassette {
            s.cassette = cogs;
            changed = true;
        }
    }
    changed
}

/// What the gearing gives: how many gears, the easiest and hardest, and
/// every gear as a bar.
fn gearing_result(ui: &mut egui::Ui, gear: usize, units: Units, rings: &[u8], cogs: &[u8]) {
    use super::glass;
    use crate::gears;
    let all = gears::all();
    let n = all.len();
    let kmh = |i: usize| units.speed(gears::speed(90.0, i) * 3.6);
    let name = |g: gears::Gear| format!("{}×{}", g.ring, g.cog);
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(format!("{n} gears · {}", gears::describe(rings, cogs)))
                .font(theme::semibold(15.0))
                .color(Color32::WHITE),
        );
        ui.label(
            RichText::new(format!(
                "Easiest {} ≈ {:.0} {u} · hardest {} ≈ {:.0} {u}, at 90 rpm",
                name(all[0]),
                kmh(0),
                name(all[n - 1]),
                kmh(n - 1),
                u = units.speed_unit()
            ))
            .color(glass::SOFT),
        );
    });
    let labels = ui.available_width() / n as f32 > 34.0;
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), if labels { 92.0 } else { 72.0 }),
        Sense::hover(),
    );
    let gap = 4.0;
    let w = (rect.width() - gap * (n - 1) as f32) / n as f32;
    let max = all[n - 1].ratio() as f32;
    let big = all.iter().map(|g| g.ring).max().unwrap_or(0);
    let two = all.iter().any(|g| g.ring != big);
    let gear = gear.min(n - 1);
    let p = ui.painter();
    for (i, g) in all.iter().enumerate() {
        let x = rect.left() + i as f32 * (w + gap);
        let h = 12.0 + g.ratio() as f32 / max * 58.0;
        let bottom = rect.top() + 70.0;
        let fill = if i == gear {
            Color32::from_white_alpha(235)
        } else if two && g.ring == big {
            Color32::from_white_alpha(82)
        } else {
            Color32::from_white_alpha(51)
        };
        p.rect_filled(
            Rect::from_min_max(Pos2::new(x, bottom - h), Pos2::new(x + w, bottom)),
            CornerRadius::same(4),
            fill,
        );
        if labels {
            p.text(
                Pos2::new(x + w / 2.0, bottom + 14.0),
                Align2::CENTER_CENTER,
                name(*g),
                theme::regular(10.5),
                if i == gear {
                    Color32::WHITE
                } else {
                    glass::QUIET
                },
            );
        }
    }
    ui.add(
        egui::Label::new(
            RichText::new(if two {
                "Two rings are shifted as a rider does: the small ring on the bigger cogs, then the big ring, \
                 leaving out crossed and near-duplicate gears. The highlighted gear is the one you are in."
            } else {
                "One ring: every cog is a gear, in order. The highlighted gear is the one you are in; \
                 changing the gearing moves you to the nearest one."
            })
            .size(12.5)
            .color(glass::QUIET),
        )
        .wrap(),
    );
}

/// Jersey colours offered as swatches (sRGB); any other is a click away.
const JERSEYS: [(&str, [u8; 3]); 9] = [
    ("Orange", [230, 90, 40]),
    ("Red", [201, 37, 44]),
    ("Blue", [37, 99, 201]),
    ("Teal", [20, 163, 160]),
    ("Green", [60, 154, 62]),
    ("Yellow", [242, 194, 48]),
    ("Pink", [226, 85, 154]),
    ("Black", [27, 28, 32]),
    ("White", [237, 237, 239]),
];

/// Paints offered for the aero bike (sRGB).
const PAINTS: [(&str, [u8; 3]); 7] = [
    ("Red", [206, 32, 39]),
    ("Black", [22, 23, 26]),
    ("White", [237, 237, 239]),
    ("Blue", [30, 79, 168]),
    ("Sage", [124, 148, 115]),
    ("Sand", [200, 180, 142]),
    ("Orange", [232, 102, 42]),
];

/// The views the preview's buttons turn to: from the side, three
/// quarters from the front, and head on.
const VIEWS: [(&str, f32); 3] = [
    ("Side", 0.0),
    ("¾", 0.62),
    ("Front", std::f32::consts::FRAC_PI_2),
];

/// A row of round colour swatches; the chosen one ringed. True if one was
/// clicked.
fn swatches(
    ui: &mut egui::Ui,
    value: &mut [u8; 3],
    options: &[(&str, [u8; 3])],
    size: f32,
) -> bool {
    let mut picked = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 9.0;
        for (name, rgb) in options {
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size + 6.0), Sense::click());
            let p = ui.painter();
            let r = size / 2.0;
            p.circle_filled(rect.center(), r, Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
            p.circle_stroke(
                rect.center(),
                r,
                Stroke::new(1.0, Color32::from_white_alpha(60)),
            );
            if *value == *rgb {
                p.circle_stroke(rect.center(), r + 2.5, Stroke::new(2.0, Color32::WHITE));
            } else if resp.hovered() {
                p.circle_stroke(
                    rect.center(),
                    r + 2.5,
                    Stroke::new(1.5, Color32::from_white_alpha(120)),
                );
            }
            if resp
                .on_hover_text(*name)
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                *value = *rgb;
                picked = true;
            }
        }
    });
    picked
}

impl App {
    /// Settings › Profile's first card: the rider on the bike in 3D,
    /// turning when dragged, beside the jersey, bike, kit and skin choices.
    fn rider_card(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        card(ui, "Your rider", |ui| {
            let wide = ui.available_width() > 620.0;
            let preview_w = if wide {
                (ui.available_width() * 0.5).min(460.0)
            } else {
                ui.available_width()
            };
            let layout = if wide {
                egui::Layout::left_to_right(egui::Align::TOP)
            } else {
                egui::Layout::top_down(egui::Align::LEFT)
            };
            ui.with_layout(layout, |ui| {
                ui.spacing_mut().item_spacing.x = 22.0;
                ui.allocate_ui_with_layout(
                    Vec2::new(preview_w, 0.0),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| self.rider_preview(ui, preview_w),
                );
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), 0.0),
                    egui::Layout::top_down(egui::Align::LEFT),
                    |ui| changed |= self.rider_choices(ui),
                );
            });
        });
        if changed {
            self.mark_dirty();
        }
    }

    /// The preview: a stage with the rider drawn on it, the view buttons
    /// above, and a hint to drag.
    fn rider_preview(&mut self, ui: &mut egui::Ui, w: f32) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (name, yaw) in VIEWS {
                let on = (wrap_pi(self.preview_yaw_to - yaw)).abs() < 0.05;
                let text = RichText::new(name).size(12.5).color(if on {
                    Color32::WHITE
                } else {
                    Color32::from_white_alpha(200)
                });
                let b = egui::Button::new(text)
                    .fill(if on {
                        Color32::from_white_alpha(50)
                    } else {
                        Color32::TRANSPARENT
                    })
                    .corner_radius(99);
                if ui.add(b).clicked() {
                    self.preview_yaw_to = yaw;
                }
            }
        });
        let h = (w * 0.86).max(200.0);
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::drag());
        let p = ui.painter_at(rect);
        if resp.dragged() {
            let dx = resp.drag_delta().x * 0.012;
            self.preview_yaw -= dx;
            self.preview_yaw_to = self.preview_yaw;
        }
        // Ease to the view picked, the short way round.
        let gap = wrap_pi(self.preview_yaw_to - self.preview_yaw);
        self.preview_yaw += gap * 0.18;
        let s = &self.store.settings;
        if self.gpu_capable {
            let look = crate::gfx::PreviewLook {
                jersey: s.jersey,
                skin: s.skin,
                yaw: self.preview_yaw,
                time: ui.input(|i| i.time),
            };
            self.gpu.paint_preview(&p, rect, &look);
            ui.ctx().request_repaint();
        } else {
            p.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "The 3D preview needs 3D graphics,\nwhich are off on this computer.",
                theme::regular(13.0),
                Color32::from_white_alpha(190),
            );
        }
        let resp = resp.on_hover_cursor(egui::CursorIcon::Grab);
        let _ = resp;
        p.text(
            rect.center_bottom() + Vec2::new(0.0, -4.0),
            egui::Align2::CENTER_BOTTOM,
            "Drag to turn",
            theme::regular(12.0),
            Color32::from_white_alpha(200),
        );
    }

    /// Jersey, bike, kit and skin. True if any changed.
    fn rider_choices(&mut self, ui: &mut egui::Ui) -> bool {
        let s = &mut self.store.settings;
        let mut changed = false;
        let label = |ui: &mut egui::Ui, t: &str| {
            ui.label(
                RichText::new(t)
                    .size(13.0)
                    .color(Color32::from_white_alpha(200)),
            );
        };
        ui.spacing_mut().item_spacing.y = 8.0;
        label(ui, "Jersey colour");
        ui.horizontal_wrapped(|ui| {
            changed |= swatches(ui, &mut s.jersey, &JERSEYS, 26.0);
            changed |= colour_wheel(ui, &mut s.jersey, "jersey-wheel");
        });
        ui.add_space(4.0);
        // The aero bike, where this build has it (see tools/bike-model).
        let aero = crate::gfx::aero_bike_available();
        if aero {
            label(ui, "Bike");
            changed |= segmented(
                ui,
                &mut s.classic_bike,
                &[
                    (false, "Aero road", "A red aero road bike with disc brakes"),
                    (true, "Classic", "The simpler bike VeloForge started with"),
                ],
            );
        }
        if aero && !s.classic_bike {
            ui.horizontal_wrapped(|ui| {
                changed |= swatches(ui, &mut s.bike_colour, &PAINTS, 22.0);
                changed |= colour_wheel(ui, &mut s.bike_colour, "bike-wheel");
            });
        }
        ui.add_space(4.0);
        ui.separator();
        ui.label(RichText::new("Kit").strong().size(14.0));
        ui.label(
            RichText::new("Black or white, each on its own.")
                .size(12.5)
                .color(Color32::from_white_alpha(170)),
        );
        for (i, name) in ["Helmet", "Gloves", "Socks", "Shoes"]
            .into_iter()
            .enumerate()
        {
            // Each row its own id: the switches' labels are the same.
            ui.push_id(name, |ui| {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        Vec2::new(90.0, 24.0),
                        egui::Label::new(RichText::new(name).size(13.5)),
                    );
                    changed |= segmented(
                        ui,
                        &mut s.kit_white[i],
                        &[(false, "Black", ""), (true, "White", "")],
                    );
                });
            });
        }
        ui.add_space(4.0);
        ui.separator();
        label(ui, "Skin tone");
        changed |= swatches(ui, &mut s.skin, &crate::world::SKIN_TONES, 26.0);
        changed
    }
}

/// An angle wrapped to -π..π.
fn wrap_pi(a: f32) -> f32 {
    let t = std::f32::consts::TAU;
    (a + std::f32::consts::PI).rem_euclid(t) - std::f32::consts::PI
}

/// sRGB (0..1) from hue (0..1, red at 0), saturation and brightness.
fn hsv_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let i = h6.floor();
    let f = h6 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i as i32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// Hue, saturation and brightness (0..1) of an sRGB colour.
fn rgb_hsv(c: [u8; 3]) -> [f32; 3] {
    let [r, g, b] = c.map(|x| x as f32 / 255.0);
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    let h = if d <= 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    let s = if max <= 0.0 { 0.0 } else { d / max };
    [h, s, max]
}

fn to_u8(c: [f32; 3]) -> [u8; 3] {
    c.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn colour32(c: [f32; 3]) -> Color32 {
    let [r, g, b] = to_u8(c);
    Color32::from_rgb(r, g, b)
}

/// The colour in a hex code: "#1e4fa8", "1E4FA8" or the short "#14a".
fn parse_hex(t: &str) -> Option<[u8; 3]> {
    let t = t.trim().trim_start_matches('#');
    let t: String = match t.len() {
        3 => t.chars().flat_map(|c| [c, c]).collect(),
        6 => t.to_string(),
        _ => return None,
    };
    let byte = |i: usize| u8::from_str_radix(&t[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// Any colour: a rainbow-ringed chip that opens a colour wheel (hue round
/// it, strength out from the middle), a brightness slider and a hex code
/// box. True if the colour changed.
fn colour_wheel(ui: &mut egui::Ui, value: &mut [u8; 3], salt: &str) -> bool {
    let id = ui.make_persistent_id(salt);
    let ctx = ui.ctx().clone();
    let mut open = ctx.data(|d| d.get_temp::<bool>(id).unwrap_or(false));
    let (chip, resp) = ui.allocate_exact_size(Vec2::new(32.0, 32.0), Sense::click());
    let p = ui.painter();
    let c = chip.center();
    // A rainbow ring round the chosen colour says "any colour".
    for i in 0..36 {
        let a0 = i as f32 / 36.0 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / 36.0 * std::f32::consts::TAU;
        p.line_segment(
            [c + 13.0 * Vec2::angled(a0), c + 13.0 * Vec2::angled(a1)],
            Stroke::new(
                3.0,
                colour32(hsv_rgb(-(a0 / std::f32::consts::TAU), 0.85, 1.0)),
            ),
        );
    }
    p.circle_filled(c, 9.0, Color32::from_rgb(value[0], value[1], value[2]));
    let resp = resp
        .on_hover_text("Any colour: a wheel, or a hex code")
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        open = !open;
    }
    let mut changed = false;
    if open {
        // Hue kept while the colour is grey or black, where it is lost.
        let hsv_id = id.with("hsv");
        let mut hsv = ctx
            .data(|d| d.get_temp::<[f32; 3]>(hsv_id))
            .unwrap_or(rgb_hsv(*value));
        if to_u8(hsv_rgb(hsv[0], hsv[1], hsv[2])) != *value {
            hsv = rgb_hsv(*value);
        }
        let hex_id = id.with("hex");
        let area = egui::Area::new(id.with("pop"))
            .order(egui::Order::Foreground)
            .fixed_pos(chip.left_bottom() + Vec2::new(-80.0, 8.0))
            .show(&ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .corner_radius(16)
                    .inner_margin(14)
                    .show(ui, |ui| {
                        ui.set_width(196.0);
                        ui.spacing_mut().item_spacing.y = 10.0;
                        let size = 196.0;
                        let (rect, wr) =
                            ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());
                        let centre = rect.center();
                        let r = size / 2.0 - 4.0;
                        let mut mesh = egui::Mesh::default();
                        mesh.colored_vertex(centre, colour32(hsv_rgb(0.0, 0.0, hsv[2])));
                        let n = 72;
                        for i in 0..=n {
                            let h = i as f32 / n as f32;
                            let a = -h * std::f32::consts::TAU;
                            mesh.colored_vertex(
                                centre + r * Vec2::angled(a),
                                colour32(hsv_rgb(h, 1.0, hsv[2])),
                            );
                        }
                        for i in 0..n as u32 {
                            mesh.add_triangle(0, i + 1, i + 2);
                        }
                        ui.painter().add(egui::Shape::mesh(mesh));
                        if let (true, Some(at)) =
                            (wr.is_pointer_button_down_on(), wr.interact_pointer_pos())
                        {
                            let d = at - centre;
                            hsv[0] = (-d.y.atan2(d.x) / std::f32::consts::TAU).rem_euclid(1.0);
                            hsv[1] = (d.length() / r).min(1.0);
                            if hsv[2] < 0.05 {
                                hsv[2] = 1.0;
                            }
                            changed = true;
                        }
                        let mark =
                            centre + hsv[1] * r * Vec2::angled(-hsv[0] * std::f32::consts::TAU);
                        ui.painter().circle(
                            mark,
                            7.0,
                            colour32(hsv_rgb(hsv[0], hsv[1], hsv[2])),
                            Stroke::new(2.5, Color32::WHITE),
                        );
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Brightness").size(12.5));
                            ui.spacing_mut().slider_width = 110.0;
                            if ui
                                .add(egui::Slider::new(&mut hsv[2], 0.0..=1.0).show_value(false))
                                .changed()
                            {
                                changed = true;
                            }
                        });
                        if changed {
                            *value = to_u8(hsv_rgb(hsv[0], hsv[1], hsv[2]));
                        }
                        ui.horizontal(|ui| {
                            let (sw, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
                            ui.painter().rect_filled(
                                sw,
                                6.0,
                                Color32::from_rgb(value[0], value[1], value[2]),
                            );
                            let current =
                                format!("#{:02X}{:02X}{:02X}", value[0], value[1], value[2]);
                            let mut text = ctx
                                .data(|d| d.get_temp::<String>(hex_id))
                                .unwrap_or(current.clone());
                            let edit = ui.add(
                                egui::TextEdit::singleline(&mut text)
                                    .desired_width(110.0)
                                    .hint_text("#1E4FA8")
                                    .font(egui::TextStyle::Monospace),
                            );
                            if edit.changed() {
                                if let Some(c) = parse_hex(&text) {
                                    *value = c;
                                    hsv = rgb_hsv(c);
                                    changed = true;
                                }
                            }
                            if edit.has_focus() {
                                ctx.data_mut(|d| d.insert_temp(hex_id, text));
                            } else {
                                ctx.data_mut(|d| d.remove::<String>(hex_id));
                            }
                        });
                    });
            });
        ctx.data_mut(|d| d.insert_temp(hsv_id, hsv));
        // Clicking away, or Escape, closes it.
        let away = ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|at| !area.response.rect.contains(at) && !chip.contains(at))
        });
        if away || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            open = false;
        }
    }
    ctx.data_mut(|d| d.insert_temp(id, open));
    changed
}
