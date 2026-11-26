//! Your music: adding songs, playing them, the Click's Y, A and Z, the
//! card in Settings › Sound & music, and Now playing while riding.

use super::glass::{self, Glyph};
use super::*;
use crate::music::{self, Action, Repeat, Song};

/// A number from 0 to 1 that changes every call, for shuffling.
fn pick() -> f32 {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (n % 10_000) as f32 / 10_000.0
}

impl App {
    /// Copy an MP3 into the data folder and add it to the songs.
    pub(super) fn add_song(&mut self, path: &Path, bytes: Option<&[u8]>) {
        let data = match bytes {
            Some(b) => b.to_vec(),
            None => match std::fs::read(path) {
                Ok(b) => b,
                Err(e) => return self.notify(format!("Could not read {}: {e}", path.display())),
            },
        };
        let dir = self.paths.music();
        let _ = std::fs::create_dir_all(&dir);
        let base = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "song.mp3".into());
        let mut name = base.clone();
        let mut n = 2;
        while dir.join(&name).exists() {
            name = format!("{n}-{base}");
            n += 1;
        }
        let dest = dir.join(&name);
        if let Err(e) = std::fs::write(&dest, &data) {
            return self.notify(format!("Could not keep {base}: {e}"));
        }
        let Some(secs) = music::duration(&dest) else {
            let _ = std::fs::remove_file(&dest);
            return self.notify(format!("{base} is not an MP3 VeloForge can play"));
        };
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let (name_title, name_artist) = music::names_from_file(&stem);
        let tags = music::read_tags(&dest);
        let title = tags.title.unwrap_or(name_title);
        let artist = tags.artist.unwrap_or(name_artist);
        self.store.settings.songs.push(Song {
            file: name,
            title: title.clone(),
            artist,
            secs,
        });
        self.mark_dirty();
        self.notify(format!("Added {title}"));
    }

    /// For songs added before tags were read: their title and artist from
    /// the tags, the first time they play.
    fn song_names(&mut self, i: usize) {
        let Some(song) = self.store.settings.songs.get(i).cloned() else {
            return;
        };
        if self.song_art.contains_key(&song.file) {
            return;
        }
        self.song_art.insert(song.file.clone(), None);
        if !song.artist.is_empty() {
            return;
        }
        let tags = music::read_tags(&self.paths.music().join(&song.file));
        if let Some(a) = tags.artist {
            let s = &mut self.store.settings.songs[i];
            s.artist = a;
            if let Some(t) = tags.title {
                s.title = t;
            }
            self.mark_dirty();
        }
    }

    /// Ask for MP3s and add them.
    fn add_songs_dialog(&mut self) {
        let Some(files) = rfd::FileDialog::new()
            .set_title("Add songs")
            .add_filter("MP3 songs", &["mp3"])
            .pick_files()
        else {
            return;
        };
        for f in files {
            self.add_song(&f, None);
        }
    }

    /// Play song `i` from its start.
    fn play_song(&mut self, i: usize) {
        let Some(song) = self.store.settings.songs.get(i).cloned() else {
            return;
        };
        let path = self.paths.music().join(&song.file);
        let volume = self.store.settings.music_volume;
        if let Err(e) = self.music.play(i, &path, volume) {
            self.notify(e);
        }
        self.music_auto_paused = false;
    }

    /// Play or pause, next or previous: from the Click, the keyboard or the
    /// buttons on screen.
    pub(super) fn music_action(&mut self, a: Action) {
        let s = &self.store.settings;
        let len = s.songs.len();
        if len == 0 {
            return self.notify("Add songs in Settings › Sound & music");
        }
        let (shuffle, repeat) = (s.music_shuffle, s.music_repeat);
        match a {
            Action::PlayPause => {
                if self.music.current.is_none() || self.music.finished() {
                    if let Some(i) = music::step(None, len, shuffle, repeat, true, true, pick()) {
                        self.play_song(i);
                    }
                } else if self.music.is_playing() {
                    self.music.pause();
                } else {
                    self.music.resume();
                    self.music_auto_paused = false;
                }
            }
            Action::Next => {
                if let Some(i) =
                    music::step(self.music.current, len, shuffle, repeat, true, true, pick())
                {
                    self.play_song(i);
                }
            }
            Action::Previous => {
                // Back to the start of this song first, then the one before.
                if self.music.position() > 3.0 {
                    self.music.restart();
                } else if let Some(i) = music::step(
                    self.music.current,
                    len,
                    shuffle,
                    repeat,
                    false,
                    true,
                    pick(),
                ) {
                    self.play_song(i);
                }
            }
        }
    }

    /// One of the Click's other buttons: learn it, or do what it is for.
    pub(super) fn click_button(&mut self, bits: u64) {
        if bits == 0 {
            return;
        }
        let s = &mut self.store.settings;
        if let Some(a) = self.music_learn.take() {
            // The lowest button pressed is the one.
            let bit = 1u64 << bits.trailing_zeros().min(63);
            for slot in [&mut s.click_play, &mut s.click_next, &mut s.click_previous] {
                if *slot == bit {
                    *slot = 0;
                }
            }
            match a {
                Action::PlayPause => s.click_play = bit,
                Action::Next => s.click_next = bit,
                Action::Previous => s.click_previous = bit,
            }
            self.mark_dirty();
            self.notify("Button assigned");
            return;
        }
        let (play, next, prev) = (s.click_play, s.click_next, s.click_previous);
        if play != 0 && bits & play != 0 {
            self.music_action(Action::PlayPause);
        } else if next != 0 && bits & next != 0 {
            self.music_action(Action::Next);
        } else if prev != 0 && bits & prev != 0 {
            self.music_action(Action::Previous);
        }
    }

    /// Every frame: the next song when one ends, pausing with the ride,
    /// and stepping back while a beep plays.
    pub(super) fn music_tick(&mut self, ctx: &egui::Context) {
        if self.music.current.is_none() {
            return;
        }
        let s = &self.store.settings;
        if self.music.finished() {
            let len = s.songs.len();
            match music::step(
                self.music.current,
                len,
                s.music_shuffle,
                s.music_repeat,
                true,
                false,
                pick(),
            ) {
                Some(i) => self.play_song(i),
                None => self.music.stop(),
            }
            return;
        }
        let paused = self.ride.state == ride::State::Paused;
        if s.music_with_ride {
            if paused && self.music.is_playing() {
                self.music.pause();
                self.music_auto_paused = true;
            } else if !paused && self.music_auto_paused {
                self.music.resume();
                self.music_auto_paused = false;
            }
        }
        let ducking = s.music_duck && self.duck_until.is_some_and(|t| Instant::now() < t);
        self.music
            .set_volume(s.music_volume * if ducking { 0.3 } else { 1.0 });
        if self.music.is_playing() {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    /// Settings › Sound & music: the songs and how they play, then which
    /// Click buttons run them.
    pub(super) fn settings_music(&mut self, ui: &mut egui::Ui) {
        let mut play = None;
        let mut remove = None;
        let mut add = false;
        glass::card(ui, |ui| {
            ui.horizontal(|ui| {
                glass::heading(ui, "Music");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if glass::pill_white(ui, "+ Add songs…").clicked() {
                        add = true;
                    }
                });
            });
            {
                let s = &self.store.settings;
                let mins: f32 = s.songs.iter().map(|x| x.secs).sum::<f32>() / 60.0;
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "{} songs · {:.0} min · kept in VeloForge's own folder. Or drop MP3s onto the window.",
                            s.songs.len(),
                            mins
                        ))
                        .color(glass::QUIET),
                    )
                    .wrap(),
                );
            }
            ui.add_space(8.0);
            egui::ScrollArea::vertical()
                .id_salt("songs")
                .max_height(240.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if self.store.settings.songs.is_empty() {
                        ui.label(
                            RichText::new("No songs yet. Add MP3s to ride to your own music.")
                                .color(glass::QUIET),
                        );
                    }
                    for (i, song) in self.store.settings.songs.iter().enumerate() {
                        let on = self.music.current == Some(i);
                        let (rect, resp) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 44.0),
                            Sense::click(),
                        );
                        let p = ui.painter();
                        if on || resp.hovered() {
                            p.rect_filled(
                                rect,
                                CornerRadius::same(12),
                                Color32::from_white_alpha(if on { 31 } else { 26 }),
                            );
                        }
                        p.text(
                            Pos2::new(rect.left() + 30.0, rect.center().y),
                            Align2::RIGHT_CENTER,
                            if on {
                                "▶".to_string()
                            } else {
                                (i + 1).to_string()
                            },
                            theme::regular(12.5),
                            if on { Color32::WHITE } else { glass::QUIET },
                        );
                        let x = rect.left() + 46.0;
                        let clip = p.with_clip_rect(Rect::from_min_max(
                            Pos2::new(x, rect.top()),
                            Pos2::new(rect.right() - 130.0, rect.bottom()),
                        ));
                        clip.text(
                            Pos2::new(x, rect.center().y - 8.0),
                            Align2::LEFT_CENTER,
                            &song.title,
                            if on {
                                theme::semibold(13.5)
                            } else {
                                theme::regular(13.5)
                            },
                            Color32::WHITE,
                        );
                        clip.text(
                            Pos2::new(x, rect.center().y + 9.0),
                            Align2::LEFT_CENTER,
                            if song.artist.is_empty() {
                                "—"
                            } else {
                                &song.artist
                            },
                            theme::regular(12.0),
                            glass::QUIET,
                        );
                        p.text(
                            Pos2::new(rect.right() - 80.0, rect.center().y),
                            Align2::RIGHT_CENTER,
                            mmss(song.secs),
                            theme::regular(12.5),
                            glass::SOFT,
                        );
                        let rm = Rect::from_min_max(
                            Pos2::new(rect.right() - 70.0, rect.top()),
                            rect.max,
                        );
                        let rm_resp =
                            ui.interact(rm, ui.scope_id().with(("rm-song", i)), Sense::click());
                        ui.painter().text(
                            Pos2::new(rect.right() - 12.0, rect.center().y),
                            Align2::RIGHT_CENTER,
                            "Remove",
                            theme::regular(12.5),
                            if rm_resp.hovered() {
                                Color32::WHITE
                            } else {
                                glass::ALERT
                            },
                        );
                        if rm_resp.clicked() {
                            remove = Some(i);
                        } else if resp
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            play = Some(i);
                        }
                    }
                });
            ui.add_space(8.0);
            let s = &mut self.store.settings;
            let mut changed = false;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.label(RichText::new("Order").color(glass::SOFT));
                changed |= segmented(
                    ui,
                    &mut s.music_shuffle,
                    &[
                        (false, "In order", "Play the songs in the list's order"),
                        (true, "Shuffle", "Play them in a random order"),
                    ],
                );
                ui.add_space(12.0);
                ui.label(RichText::new("Repeat").color(glass::SOFT));
                changed |= segmented(
                    ui,
                    &mut s.music_repeat,
                    &[
                        (Repeat::Off, "Off", "Stop after the last song"),
                        (Repeat::All, "All", "Start the list again"),
                        (Repeat::One, "One", "Play the same song again"),
                    ],
                );
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Music volume").color(glass::SOFT));
                changed |= ui
                    .add(
                        egui::Slider::new(&mut s.music_volume, 0.0..=1.0)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                    )
                    .changed();
            });
            ui.horizontal_wrapped(|ui| {
                changed |= ui
                    .checkbox(
                        &mut s.music_with_ride,
                        "Pause the music when the ride pauses",
                    )
                    .changed();
                ui.add_space(12.0);
                changed |= ui
                    .checkbox(&mut s.music_duck, "Lower the music while a beep plays")
                    .changed();
            });
            if changed {
                self.mark_dirty();
            }
        });
        if add {
            self.add_songs_dialog();
        }
        if let Some(i) = play {
            self.play_song(i);
        }
        if let Some(i) = remove {
            let song = self.store.settings.songs.remove(i);
            let _ = std::fs::remove_file(self.paths.music().join(&song.file));
            match self.music.current {
                Some(c) if c == i => self.music.stop(),
                Some(c) if c > i => self.music.current = Some(c - 1),
                _ => {}
            }
            self.mark_dirty();
        }

        ui.add_space(14.0);
        self.settings_click_music(ui);
    }

    /// Which buttons on the Click's right half run the music.
    fn settings_click_music(&mut self, ui: &mut egui::Ui) {
        glass::card(ui, |ui| {
            glass::heading(ui, "Zwift Click, right half");
            ui.label(
                RichText::new("+ and B keep shifting. Y, A and Z run the music.")
                    .color(glass::QUIET),
            );
            ui.add_space(8.0);
            let rows = [
                (Action::PlayPause, "Play or pause"),
                (Action::Next, "Next song"),
                (Action::Previous, "Previous song"),
            ];
            egui::Grid::new("click-music")
                .num_columns(3)
                .spacing([18.0, 10.0])
                .show(ui, |ui| {
                    for (a, label) in rows {
                        ui.label(RichText::new(label).color(Color32::WHITE));
                        let bit = match a {
                            Action::PlayPause => self.store.settings.click_play,
                            Action::Next => self.store.settings.click_next,
                            Action::Previous => self.store.settings.click_previous,
                        };
                        ui.label(
                            RichText::new(button_name(bit))
                                .font(theme::medium(13.5))
                                .color(button_colour(bit)),
                        );
                        let learning = self.music_learn == Some(a);
                        if glass::pill(
                            ui,
                            if learning {
                                "Press a button on the Click…"
                            } else {
                                "Press to assign"
                            },
                            learning,
                        )
                        .clicked()
                        {
                            self.music_learn = if learning { None } else { Some(a) };
                        }
                        ui.end_row();
                    }
                });
            ui.add_space(4.0);
            ui.add(
                        egui::Label::new(
                            RichText::new(
                                "Press to assign: click it, then press the button on your Click you want for that. VeloForge remembers whichever button it was.",
                            )
                            .font(theme::regular(12.5))
                            .color(glass::QUIET),
                        )
                        .wrap(),
                    );
        });
    }

    /// While riding: the song, how far into it, and ⏮ ⏯ ⏭ with the Click
    /// letters under them.
    pub(super) fn now_playing(&mut self, ui: &mut egui::Ui, r: Rect) {
        let Some(i) = self.music.current else {
            return self.music_idle(ui, r);
        };
        self.song_names(i);
        let Some(song) = self.store.settings.songs.get(i).cloned() else {
            return;
        };
        let p = ui.painter();
        // The title and artist take the top, where a cover used to be.
        let cover = Rect::from_min_size(r.min + Vec2::new(12.0, 12.0), Vec2::new(0.0, 48.0));
        let x = r.left() + 16.0;
        let room = Rect::from_min_max(
            Pos2::new(x, r.top()),
            Pos2::new(r.right() - 12.0, cover.bottom()),
        );
        let time = ui.input(|i| i.time);
        let artist = if song.artist.is_empty() {
            "Your music"
        } else {
            &song.artist
        };
        let mut moving = marquee(
            p,
            room,
            cover.center().y - 9.0,
            &song.title,
            theme::semibold(14.0),
            Color32::WHITE,
            time,
        );
        moving |= marquee(
            p,
            room,
            cover.center().y + 10.0,
            artist,
            theme::regular(12.0),
            glass::SOFT,
            time,
        );
        if moving {
            ui.ctx().request_repaint();
        }
        // Progress.
        let pos = self.music.position();
        let bar = Rect::from_min_size(
            Pos2::new(r.left() + 12.0, cover.bottom() + 12.0),
            Vec2::new(r.width() - 24.0, 3.0),
        );
        p.rect_filled(bar, CornerRadius::same(2), Color32::from_white_alpha(51));
        let frac = if song.secs > 0.0 {
            (pos / song.secs).clamp(0.0, 1.0)
        } else {
            0.0
        };
        p.rect_filled(
            Rect::from_min_size(bar.min, Vec2::new(bar.width() * frac, 3.0)),
            CornerRadius::same(2),
            Color32::WHITE,
        );
        p.text(
            bar.left_bottom() + Vec2::new(0.0, 4.0),
            Align2::LEFT_TOP,
            mmss(pos),
            theme::regular(11.0),
            glass::QUIET,
        );
        p.text(
            bar.right_bottom() + Vec2::new(0.0, 4.0),
            Align2::RIGHT_TOP,
            mmss(song.secs),
            theme::regular(11.0),
            glass::QUIET,
        );
        // ⏮ ⏯ ⏭, the Click's letters under them.
        let y = bar.bottom() + 42.0;
        let c = r.center().x;
        let playing = self.music.is_playing();
        let s = &self.store.settings;
        let buttons = [
            (
                c - 52.0,
                36.0,
                Glyph::PrevSong,
                false,
                Action::Previous,
                s.click_previous,
            ),
            (
                c,
                42.0,
                if playing { Glyph::Pause } else { Glyph::Play },
                true,
                Action::PlayPause,
                s.click_play,
            ),
            (
                c + 52.0,
                36.0,
                Glyph::NextSong,
                false,
                Action::Next,
                s.click_next,
            ),
        ];
        let mut act = None;
        for (cx, size, g, _, a, bit) in buttons {
            let rect = Rect::from_center_size(Pos2::new(cx, y), Vec2::splat(size));
            let resp = ui
                .interact(
                    rect,
                    ui.scope_id().with(("music", cx as i32)),
                    Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            // Each in the colour of its button on the Click.
            let colour = if bit == 0 {
                Color32::from_white_alpha(60)
            } else {
                button_colour(bit)
            };
            let fill = if resp.hovered() {
                theme::mix(colour, Color32::WHITE, 0.2)
            } else {
                colour
            };
            ui.painter().circle_filled(rect.center(), size / 2.0, fill);
            glass::glyph(ui.painter(), rect.center(), g, Color32::WHITE);
            if resp.clicked() {
                act = Some(a);
            }
        }
        if let Some(a) = act {
            self.music_action(a);
        }
    }

    /// Nothing playing yet: a button to start the music.
    fn music_idle(&mut self, ui: &mut egui::Ui, r: Rect) {
        let p = ui.painter();
        let cover = Rect::from_min_size(r.min + Vec2::new(12.0, 12.0), Vec2::new(0.0, 48.0));
        p.text(
            Pos2::new(r.left() + 16.0, cover.center().y - 9.0),
            Align2::LEFT_CENTER,
            "Your music",
            theme::semibold(14.0),
            Color32::WHITE,
        );
        p.text(
            Pos2::new(r.left() + 16.0, cover.center().y + 10.0),
            Align2::LEFT_CENTER,
            format!("{} songs", self.store.settings.songs.len()),
            theme::regular(12.0),
            glass::SOFT,
        );
        let rect = Rect::from_center_size(
            Pos2::new(r.center().x, cover.bottom() + 46.0),
            Vec2::splat(42.0),
        );
        let resp = ui
            .interact(rect, ui.scope_id().with("music-start"), Sense::click())
            .on_hover_text("Play your music")
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        glass::paint_round(ui.painter(), rect, Glyph::Play, true, resp.hovered());
        if resp.clicked() {
            self.music_action(Action::PlayPause);
        }
    }
}

/// One line of text in `room`, at height `y`: still when it fits, and
/// when it does not, rolling slowly to the left (after a pause at its
/// start) with a second copy following, so all of it can be read.
/// Returns whether it is moving.
fn marquee(
    p: &egui::Painter,
    room: Rect,
    y: f32,
    text: &str,
    font: FontId,
    colour: Color32,
    time: f64,
) -> bool {
    let galley = p.layout_no_wrap(text.to_string(), font, colour);
    let (w, h) = (galley.size().x, galley.size().y);
    let clip = p.with_clip_rect(room.intersect(p.clip_rect()));
    if w <= room.width() {
        clip.galley(Pos2::new(room.left(), y - h / 2.0), galley, colour);
        return false;
    }
    const GAP: f32 = 48.0;
    const SPEED: f32 = 28.0; // points a second
    const PAUSE: f32 = 2.0; // seconds at the start of each turn
    let lap = w + GAP;
    let phase = (time as f32) % (PAUSE + lap / SPEED);
    let offset = ((phase - PAUSE) * SPEED).max(0.0);
    let at = Pos2::new(room.left() - offset, y - h / 2.0);
    clip.galley(at, galley.clone(), colour);
    clip.galley(at + Vec2::new(lap, 0.0), galley, colour);
    true
}

/// "3:48" for a song's times.
fn mmss(secs: f32) -> String {
    let s = secs.max(0.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

/// A Click button's letter.
fn button_name(bit: u64) -> String {
    match bit {
        0 => "—".into(),
        b if b == crate::zwift::Y => "Y".into(),
        b if b == crate::zwift::A => "A".into(),
        b if b == crate::zwift::Z => "Z".into(),
        b => format!("Button {b:#x}"),
    }
}

/// The Click's own colours for its letters.
fn button_colour(bit: u64) -> Color32 {
    match bit {
        b if b == crate::zwift::Y => Color32::from_rgb(43, 167, 199),
        b if b == crate::zwift::A => Color32::from_rgb(63, 167, 90),
        b if b == crate::zwift::Z => Color32::from_rgb(232, 102, 61),
        _ => glass::SOFT,
    }
}
