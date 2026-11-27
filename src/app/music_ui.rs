//! Your music: your music folder and songs added one by one, playing them,
//! the Click's Y, A and Z, the card in Settings › Sound & music, and Now
//! playing while riding.

use super::glass::{self, Glyph};
use super::*;
use crate::music::{self, Action, Repeat, Song};
use std::path::PathBuf;
use std::sync::mpsc;

/// A number from 0 to 1 that changes every call, for shuffling.
fn pick() -> f32 {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (n % 10_000) as f32 / 10_000.0
}

/// What the music's work out of sight sends back.
enum Done {
    /// The music folder, looked through: the MP3s in it (None when it
    /// isn't there) and the ones not in the list yet, read.
    Scanned {
        folder: PathBuf,
        found: Option<Vec<String>>,
        new: Vec<Song>,
        by_hand: bool,
    },
    /// A song added one by one, copied in.
    Added(Result<Song, String>),
    /// The songs added together are all in.
    AddedAll,
}

/// The music's work out of sight (looking through the folder, copying
/// songs in), so the screen never stops for it, and what the Settings
/// card needs to remember.
pub(super) struct MusicWork {
    tx: mpsc::Sender<Done>,
    rx: mpsc::Receiver<Done>,
    ctx: Option<egui::Context>,
    /// Looks through the folder still running.
    scans: usize,
    /// Jobs copying songs in still running.
    adding: usize,
    /// Songs those jobs added so far, and the last one's title.
    added: usize,
    last_added: String,
    /// The folder has been looked through since VeloForge opened.
    scanned: bool,
    /// The music folder isn't there (a drive not plugged in).
    missing: bool,
    /// The MP3s found in the folder last time, and how many were new when
    /// you asked for a look.
    found: Option<usize>,
    new_by_hand: Option<usize>,
    /// The songs played before, latest last (their keys), for Previous
    /// while shuffling.
    history: Vec<String>,
    /// The song being dragged to a new place in the list.
    drag: Option<usize>,
}

impl Default for MusicWork {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        MusicWork {
            tx,
            rx,
            ctx: None,
            scans: 0,
            adding: 0,
            added: 0,
            last_added: String::new(),
            scanned: false,
            missing: false,
            found: None,
            new_by_hand: None,
            history: Vec::new(),
            drag: None,
        }
    }
}

/// A folder's path to show, with the home folder as ~.
fn show_path(p: &Path) -> String {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    if let Some(rest) = home.and_then(|h| p.strip_prefix(h).ok().map(Path::to_path_buf)) {
        return Path::new("~").join(rest).display().to_string();
    }
    p.display().to_string()
}

impl App {
    /// Where song `i`'s file is.
    fn song_file(&self, i: usize) -> Option<PathBuf> {
        let s = &self.store.settings;
        s.songs
            .get(i)?
            .path(&self.paths.music(), s.music_folder.as_deref())
    }

    /// Song `i` can play now: it isn't from a music folder that's missing.
    fn playable(&self, i: usize) -> bool {
        let s = &self.store.settings;
        s.songs
            .get(i)
            .is_some_and(|x| !x.folder || (s.music_folder.is_some() && !self.music_work.missing))
    }

    /// The key of the song playing, to find it again when the list changes.
    fn playing_key(&self) -> Option<String> {
        self.music
            .current
            .and_then(|i| self.store.settings.songs.get(i))
            .map(Song::key)
    }

    /// After the list changed: the song playing keeps playing at its new
    /// place, or stops when it left the list.
    fn relink_playing(&mut self, key: Option<String>) {
        let Some(key) = key else {
            return;
        };
        match self
            .store
            .settings
            .songs
            .iter()
            .position(|s| s.key() == key)
        {
            Some(i) => self.music.current = Some(i),
            None => self.music.stop(),
        }
    }

    /// Add an MP3 (dropped on the window): copied in out of sight.
    pub(super) fn add_song(&mut self, path: &Path, bytes: Option<&[u8]>) {
        self.add_songs(vec![(path.to_path_buf(), bytes.map(|b| b.to_vec()))]);
    }

    /// Copy MP3s into the data folder out of sight, adding each to the list
    /// as it's in.
    fn add_songs(&mut self, files: Vec<(PathBuf, Option<Vec<u8>>)>) {
        if files.is_empty() {
            return;
        }
        let w = &mut self.music_work;
        w.adding += 1;
        let (tx, ctx, dir) = (w.tx.clone(), w.ctx.clone(), self.paths.music());
        std::thread::spawn(move || {
            for (path, bytes) in files {
                let _ = tx.send(Done::Added(music::copy_in(&path, bytes.as_deref(), &dir)));
                if let Some(c) = &ctx {
                    c.request_repaint();
                }
            }
            let _ = tx.send(Done::AddedAll);
            if let Some(c) = &ctx {
                c.request_repaint();
            }
        });
    }

    /// Look through the music folder out of sight, for songs added to it or
    /// gone from it.
    fn scan_music_folder(&mut self, by_hand: bool) {
        self.music_work.scanned = true;
        let s = &self.store.settings;
        let Some(folder) = s.music_folder.clone() else {
            return;
        };
        let mut known: std::collections::HashSet<String> = s
            .songs
            .iter()
            .filter(|x| x.folder)
            .map(|x| x.file.clone())
            .collect();
        known.extend(s.music_hidden.iter().cloned());
        let w = &mut self.music_work;
        w.scans += 1;
        let (tx, ctx) = (w.tx.clone(), w.ctx.clone());
        std::thread::spawn(move || {
            let found = music::find_mp3s(&folder);
            let new = found
                .iter()
                .flatten()
                .filter(|f| !known.contains(*f))
                .filter_map(|f| music::read_song(&folder.join(f), f.clone(), true))
                .collect();
            let _ = tx.send(Done::Scanned {
                folder,
                found,
                new,
                by_hand,
            });
            if let Some(c) = &ctx {
                c.request_repaint();
            }
        });
    }

    /// Play from another music folder, or none: its songs replace the old
    /// folder's, and the songs added one by one stay.
    fn set_music_folder(&mut self, dir: Option<PathBuf>) {
        let key = self.playing_key();
        let s = &mut self.store.settings;
        s.songs.retain(|x| !x.folder);
        s.music_hidden.clear();
        s.music_folder = dir;
        let w = &mut self.music_work;
        w.missing = false;
        w.found = None;
        w.new_by_hand = None;
        self.relink_playing(key);
        self.mark_dirty();
        self.scan_music_folder(false);
    }

    /// What the work out of sight has finished.
    fn music_done(&mut self) {
        while let Ok(done) = self.music_work.rx.try_recv() {
            match done {
                Done::Scanned {
                    folder,
                    found,
                    new,
                    by_hand,
                } => {
                    let w = &mut self.music_work;
                    w.scans = w.scans.saturating_sub(1);
                    // A folder you've since changed from.
                    if self.store.settings.music_folder.as_ref() != Some(&folder) {
                        continue;
                    }
                    let Some(found) = found else {
                        w.missing = true;
                        continue;
                    };
                    w.missing = false;
                    w.found = Some(found.len());
                    w.new_by_hand = by_hand.then_some(new.len());
                    let key = self.playing_key();
                    let s = &mut self.store.settings;
                    let mut changed = music::merge_folder(&mut s.songs, &found, new);
                    // Hidden songs deleted from the folder are forgotten.
                    let hidden = s.music_hidden.len();
                    s.music_hidden.retain(|h| found.contains(h));
                    changed |= s.music_hidden.len() != hidden;
                    self.relink_playing(key);
                    if changed {
                        self.mark_dirty();
                    }
                }
                Done::Added(Ok(song)) => {
                    self.music_work.added += 1;
                    self.music_work.last_added = song.title.clone();
                    self.store.settings.songs.push(song);
                    self.mark_dirty();
                }
                Done::Added(Err(e)) => self.notify(e),
                Done::AddedAll => {
                    let w = &mut self.music_work;
                    w.adding = w.adding.saturating_sub(1);
                    if w.adding == 0 && w.added > 0 {
                        let msg = if w.added == 1 {
                            format!("Added {}", w.last_added)
                        } else {
                            format!("Added {} songs", w.added)
                        };
                        w.added = 0;
                        self.notify(msg);
                    }
                }
            }
        }
    }

    /// For songs added before tags were read: their title and artist from
    /// the tags, the first time they play.
    fn song_names(&mut self, i: usize) {
        let Some(song) = self.store.settings.songs.get(i).cloned() else {
            return;
        };
        if self.song_art.contains_key(&song.key()) {
            return;
        }
        self.song_art.insert(song.key(), None);
        if !song.artist.is_empty() {
            return;
        }
        let Some(path) = self.song_file(i) else {
            return;
        };
        let tags = music::read_tags(&path);
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
        self.add_songs(files.into_iter().map(|f| (f, None)).collect());
    }

    /// Play song `i` from its start. `remember`: the song before it goes in
    /// the history, for Previous while shuffling.
    fn play_song(&mut self, i: usize, remember: bool) {
        let path = self.song_file(i).filter(|_| self.playable(i));
        let Some(path) = path else {
            return self.notify("Your music folder isn't there. Is its drive plugged in?");
        };
        if remember {
            if let Some(k) = self.playing_key() {
                let h = &mut self.music_work.history;
                h.push(k);
                if h.len() > 200 {
                    h.remove(0);
                }
            }
        }
        let volume = self.store.settings.music_volume;
        if let Err(e) = self.music.play(i, &path, volume) {
            self.notify(e);
        }
        self.music_auto_paused = false;
    }

    /// The song after (`forward`) or before `from` that can play now,
    /// passing over any from a music folder that's missing.
    fn next_song(&self, from: Option<usize>, forward: bool, by_hand: bool) -> Option<usize> {
        let s = &self.store.settings;
        let len = s.songs.len();
        let mut at = from;
        for _ in 0..len {
            let i = music::step(
                at,
                len,
                s.music_shuffle,
                s.music_repeat,
                forward,
                by_hand,
                pick(),
            )?;
            if self.playable(i) {
                return Some(i);
            }
            at = Some(i);
        }
        None
    }

    /// Play or pause, next or previous: from the Click, the keyboard or the
    /// buttons on screen.
    pub(super) fn music_action(&mut self, a: Action) {
        if self.store.settings.songs.is_empty() {
            return self.notify("Add your music in Settings › Sound & music");
        }
        match a {
            Action::PlayPause => {
                if self.music.current.is_none() || self.music.finished() {
                    if let Some(i) = self.next_song(None, true, true) {
                        self.play_song(i, true);
                    }
                } else if self.music.is_playing() {
                    self.music.pause();
                } else {
                    self.music.resume();
                    self.music_auto_paused = false;
                }
            }
            Action::Next => {
                if let Some(i) = self.next_song(self.music.current, true, true) {
                    self.play_song(i, true);
                }
            }
            Action::Previous => {
                // Back to the start of this song first, then the one before:
                // while shuffling, the one you heard before.
                if self.music.position() > 3.0 {
                    return self.music.restart();
                }
                if self.store.settings.music_shuffle {
                    while let Some(k) = self.music_work.history.pop() {
                        let at = self.store.settings.songs.iter().position(|s| s.key() == k);
                        if let Some(j) = at.filter(|&j| self.playable(j)) {
                            if Some(j) != self.music.current {
                                return self.play_song(j, false);
                            }
                        }
                    }
                }
                if let Some(i) = self.next_song(self.music.current, false, true) {
                    self.play_song(i, false);
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
        if self.music_work.ctx.is_none() {
            self.music_work.ctx = Some(ctx.clone());
        }
        if !self.music_work.scanned {
            self.scan_music_folder(false);
        }
        self.music_done();
        if self.music.current.is_none() {
            return;
        }
        let s = &self.store.settings;
        if self.music.finished() {
            match self.next_song(self.music.current, true, false) {
                Some(i) => self.play_song(i, true),
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

    /// Settings › Sound & music: your music folder, the songs and how they
    /// play, then which Click buttons run them.
    pub(super) fn settings_music(&mut self, ui: &mut egui::Ui) {
        let mut play = None;
        let mut remove = None;
        let mut moved = None;
        let mut act = None;
        let (mut add, mut choose, mut rescan, mut forget, mut unhide) =
            (false, false, false, false, false);
        glass::card(ui, |ui| {
            {
                let s = &self.store.settings;
                let mins: f32 = s.songs.iter().map(|x| x.secs).sum::<f32>() / 60.0;
                let from_folder = s.songs.iter().filter(|x| x.folder).count();
                glass::heading(ui, "Music");
                let summary = if s.music_folder.is_some() {
                    format!(
                        "{} songs · {:.0} min · {} from your folder, {} added",
                        s.songs.len(),
                        mins,
                        from_folder,
                        s.songs.len() - from_folder
                    )
                } else {
                    format!(
                        "{} songs · {:.0} min · songs you add are copied into VeloForge's own folder",
                        s.songs.len(),
                        mins
                    )
                };
                ui.add(egui::Label::new(RichText::new(summary).color(glass::QUIET)).wrap());
            }
            ui.add_space(10.0);
            self.music_folder_box(ui, &mut choose, &mut rescan, &mut forget);
            if !self.store.settings.songs.is_empty() {
                ui.add_space(10.0);
                act = self.music_bar(ui);
            }
            ui.add_space(10.0);
            let row = Vec2::new(ui.available_width(), 30.0);
            ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if glass::pill_small(ui, "+ Add songs…", true)
                    .on_hover_text("Copy MP3s into VeloForge's own folder")
                    .clicked()
                {
                    add = true;
                }
                let hidden = self.store.settings.music_hidden.len();
                if hidden > 0
                    && glass::pill_small(ui, &format!("Show {hidden} hidden"), false)
                        .on_hover_text("Put the songs you hid back in the list")
                        .clicked()
                {
                    unhide = true;
                }
                // The hint takes the room left, on the left.
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(
                                "Drag a song by the dots on its left to change the order. Or drop MP3s onto the window.",
                            )
                            .font(theme::regular(12.5))
                            .color(glass::QUIET),
                        )
                        .truncate(),
                    );
                });
            });
            ui.add_space(4.0);
            let missing = self.music_work.missing || self.store.settings.music_folder.is_none();
            egui::ScrollArea::vertical()
                .id_salt("songs")
                .max_height(236.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if self.store.settings.songs.is_empty() {
                        ui.label(
                            RichText::new(
                                "No songs yet. Choose your music folder, or add MP3s, to ride to your own music.",
                            )
                            .color(glass::QUIET),
                        );
                    }
                    let drag = self.music_work.drag;
                    let mut rows = Vec::new();
                    let mut started = None;
                    let mut stopped = false;
                    for (i, song) in self.store.settings.songs.iter().enumerate() {
                        let on = self.music.current == Some(i);
                        let gone = song.folder && missing;
                        let (rect, resp) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 44.0),
                            Sense::click(),
                        );
                        rows.push(rect);
                        // Faded while its folder is missing, or while dragged.
                        let fade = if gone {
                            0.42
                        } else if drag == Some(i) {
                            0.5
                        } else {
                            1.0
                        };
                        let c = |c: Color32| c.gamma_multiply(fade);
                        let p = ui.painter();
                        if on || (resp.hovered() && drag.is_none() && !gone) {
                            p.rect_filled(
                                rect,
                                CornerRadius::same(12),
                                Color32::from_white_alpha(if on { 31 } else { 26 }),
                            );
                        }
                        // The handle: six dots to drag it by.
                        let handle = Rect::from_min_size(rect.min, Vec2::new(26.0, rect.height()));
                        let h = ui
                            .interact(handle, ui.scope_id().with(("song-drag", i)), Sense::drag())
                            .on_hover_cursor(egui::CursorIcon::Grab);
                        if h.drag_started() {
                            started = Some(i);
                        }
                        if h.drag_stopped() {
                            stopped = true;
                        }
                        let dot = if h.hovered() || drag == Some(i) {
                            Color32::WHITE
                        } else {
                            Color32::from_white_alpha(110)
                        };
                        for row in 0..3 {
                            for col in 0..2 {
                                ui.painter().circle_filled(
                                    handle.center()
                                        + Vec2::new(col as f32 * 5.0 - 2.5, row as f32 * 5.0 - 5.0),
                                    1.3,
                                    dot,
                                );
                            }
                        }
                        let p = ui.painter();
                        p.text(
                            Pos2::new(rect.left() + 48.0, rect.center().y),
                            Align2::RIGHT_CENTER,
                            if on {
                                if self.music.is_playing() { "▶" } else { "❚❚" }.to_string()
                            } else {
                                (i + 1).to_string()
                            },
                            theme::regular(12.5),
                            c(if on { Color32::WHITE } else { glass::QUIET }),
                        );
                        // Where it's from, its length and Hide or Remove, at
                        // the right.
                        let rm_rect =
                            Rect::from_min_max(Pos2::new(rect.right() - 64.0, rect.top()), rect.max);
                        p.text(
                            Pos2::new(rect.right() - 76.0, rect.center().y),
                            Align2::RIGHT_CENTER,
                            mmss(song.secs),
                            theme::regular(12.5),
                            c(glass::SOFT),
                        );
                        let tag = p.layout_no_wrap(
                            if song.folder { "Folder" } else { "Added" }.to_string(),
                            theme::regular(11.0),
                            c(glass::SOFT),
                        );
                        let tag_rect = Rect::from_center_size(
                            Pos2::new(rect.right() - 128.0 - tag.size().x / 2.0, rect.center().y),
                            tag.size() + Vec2::new(16.0, 6.0),
                        );
                        p.rect_filled(tag_rect, CornerRadius::same(9), c(Color32::from_white_alpha(26)));
                        p.galley(tag_rect.center() - tag.size() / 2.0, tag, c(glass::SOFT));
                        let x = rect.left() + 60.0;
                        let clip = p.with_clip_rect(Rect::from_min_max(
                            Pos2::new(x, rect.top()),
                            Pos2::new(tag_rect.left() - 10.0, rect.bottom()),
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
                            c(Color32::WHITE),
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
                            c(glass::QUIET),
                        );
                        let rm_resp = ui.interact(
                            rm_rect,
                            ui.scope_id().with(("rm-song", i)),
                            Sense::click(),
                        );
                        let rm_resp = if song.folder {
                            rm_resp.on_hover_text("Leave it out of the list. The file stays in your folder.")
                        } else {
                            rm_resp.on_hover_text("Delete VeloForge's copy of it")
                        };
                        ui.painter().text(
                            Pos2::new(rect.right() - 12.0, rect.center().y),
                            Align2::RIGHT_CENTER,
                            if song.folder { "Hide" } else { "Remove" },
                            theme::regular(12.5),
                            if rm_resp.hovered() {
                                Color32::WHITE
                            } else if song.folder {
                                glass::SOFT
                            } else {
                                glass::ALERT
                            },
                        );
                        if rm_resp.clicked() {
                            remove = Some(i);
                        } else if !gone
                            && resp
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                        {
                            play = Some(i);
                        }
                    }
                    if let Some(i) = started {
                        self.music_work.drag = Some(i);
                    }
                    // Dragging: a line where the song will go, scrolling
                    // near the ends of the list.
                    if let (Some(from), Some(pos)) =
                        (self.music_work.drag, ui.ctx().pointer_interact_pos())
                    {
                        let slot = rows.iter().filter(|r| r.center().y < pos.y).count();
                        if stopped {
                            self.music_work.drag = None;
                            let to = if slot > from { slot - 1 } else { slot };
                            if to != from {
                                moved = Some((from, to));
                            }
                        } else if let (Some(first), Some(last)) = (rows.first(), rows.last()) {
                            let y = if slot == 0 {
                                first.top() - 1.0
                            } else {
                                rows[slot - 1].bottom() + 1.0
                            };
                            ui.painter().hline(
                                first.left() + 8.0..=last.right() - 8.0,
                                y,
                                Stroke::new(2.0, Color32::WHITE),
                            );
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                            let clip = ui.clip_rect();
                            if pos.y < clip.top() + 24.0 {
                                ui.scroll_with_delta(Vec2::new(0.0, 6.0));
                            } else if pos.y > clip.bottom() - 24.0 {
                                ui.scroll_with_delta(Vec2::new(0.0, -6.0));
                            }
                            ui.ctx().request_repaint();
                        }
                    } else if stopped {
                        self.music_work.drag = None;
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
        if choose {
            let mut dialog = rfd::FileDialog::new().set_title("Choose your music folder");
            if let Some(dir) = &self.store.settings.music_folder {
                dialog = dialog.set_directory(dir);
            }
            if let Some(dir) = dialog.pick_folder() {
                self.set_music_folder(Some(dir));
            }
        }
        if forget {
            self.set_music_folder(None);
        }
        if rescan {
            self.scan_music_folder(true);
        }
        if unhide {
            self.store.settings.music_hidden.clear();
            self.mark_dirty();
            self.scan_music_folder(true);
        }
        if let Some(a) = act {
            self.music_action(a);
        }
        if let Some(i) = play {
            self.play_song(i, true);
        }
        if let Some((from, to)) = moved {
            let key = self.playing_key();
            let songs = &mut self.store.settings.songs;
            let song = songs.remove(from);
            songs.insert(to, song);
            self.relink_playing(key);
            self.mark_dirty();
        }
        if let Some(i) = remove {
            let key = self.playing_key();
            let s = &mut self.store.settings;
            let song = s.songs.remove(i);
            if song.folder {
                s.music_hidden.push(song.file);
            } else {
                let _ = std::fs::remove_file(self.paths.music().join(&song.file));
            }
            self.relink_playing(key);
            self.mark_dirty();
        }

        ui.add_space(14.0);
        self.settings_click_music(ui);
    }

    /// The music folder: where it is and what's in it, or a box to choose
    /// one.
    fn music_folder_box(
        &mut self,
        ui: &mut egui::Ui,
        choose: &mut bool,
        rescan: &mut bool,
        forget: &mut bool,
    ) {
        let w = &self.music_work;
        let Some(dir) = &self.store.settings.music_folder else {
            egui::Frame::new()
                .stroke(Stroke::new(1.0, Color32::from_white_alpha(77)))
                .corner_radius(16)
                .inner_margin(14)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new("Play the songs from a folder")
                            .font(theme::medium(14.0))
                            .color(Color32::WHITE),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(
                                "Pick the folder your MP3s are in. VeloForge plays them from there, folders inside it too, without copying them, and finds new ones each time it opens.",
                            )
                            .font(theme::regular(12.5))
                            .color(glass::QUIET),
                        )
                        .wrap(),
                    );
                    ui.add_space(8.0);
                    if glass::pill_white(ui, "Choose a music folder…").clicked() {
                        *choose = true;
                    }
                });
            return;
        };
        let (note, colour) = if w.scans > 0 {
            ("Looking through your folder…".to_string(), glass::QUIET)
        } else if w.missing {
            (
                "Not found. Is its drive plugged in? Its songs are greyed out until it's back."
                    .to_string(),
                glass::ALERT,
            )
        } else {
            let n = w.found.unwrap_or(0);
            let mut note = format!(
                "{n} MP3{} · played from there, not copied · checked each time VeloForge opens",
                if n == 1 { "" } else { "s" }
            );
            match w.new_by_hand {
                Some(0) => note = format!("Checked just now: {n} MP3s, nothing new."),
                Some(k) => {
                    note = format!("Checked just now: {n} MP3s, {k} new at the end of the list.")
                }
                None => {}
            }
            (note, glass::QUIET)
        };
        egui::Frame::new()
            .fill(if w.missing {
                Color32::from_rgba_unmultiplied(255, 180, 171, 36)
            } else {
                Color32::from_white_alpha(18)
            })
            .corner_radius(16)
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
                    ui.painter().rect_filled(
                        r,
                        CornerRadius::same(12),
                        Color32::from_white_alpha(36),
                    );
                    glass::glyph(ui.painter(), r.center(), Glyph::Folder, Color32::WHITE);
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(format!("Music folder · {}", show_path(dir)))
                                    .font(theme::medium(14.0))
                                    .color(Color32::WHITE),
                            )
                            .truncate(),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(note).font(theme::regular(12.5)).color(colour),
                            )
                            .wrap(),
                        );
                    });
                });
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if glass::pill_small(
                        ui,
                        if w.missing {
                            "Look again"
                        } else {
                            "Check for new songs"
                        },
                        true,
                    )
                    .clicked()
                    {
                        *rescan = true;
                    }
                    if glass::pill_small(ui, "Change…", true).clicked() {
                        *choose = true;
                    }
                    if glass::pill_small(ui, "Stop using it", false)
                        .on_hover_text(
                            "Take its songs out of the list. Your files stay where they are.",
                        )
                        .clicked()
                    {
                        *forget = true;
                    }
                });
            });
    }

    /// ⏮ ⏯ ⏭ and the song playing, so the music can be stopped here too.
    fn music_bar(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut act = None;
        egui::Frame::new()
            .fill(Color32::from_black_alpha(46))
            .corner_radius(16)
            .inner_margin(egui::Margin {
                left: 8,
                right: 14,
                top: 8,
                bottom: 8,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let playing = self.music.is_playing();
                    let buttons = [
                        (
                            34.0,
                            Glyph::PrevSong,
                            false,
                            Action::Previous,
                            "Previous song",
                        ),
                        (
                            40.0,
                            if playing { Glyph::Pause } else { Glyph::Play },
                            true,
                            Action::PlayPause,
                            if playing { "Pause" } else { "Play" },
                        ),
                        (34.0, Glyph::NextSong, false, Action::Next, "Next song"),
                    ];
                    for (size, g, white, a, hover) in buttons {
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::splat(size), Sense::click());
                        if !white {
                            ui.painter().circle_filled(
                                rect.center(),
                                size / 2.0,
                                Color32::from_white_alpha(31),
                            );
                        }
                        glass::paint_round(ui.painter(), rect, g, white, resp.hovered());
                        if resp
                            .on_hover_text(hover)
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            act = Some(a);
                        }
                    }
                    ui.add_space(6.0);
                    let (r, _) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::hover());
                    let p = ui.painter();
                    let song = self
                        .music
                        .current
                        .and_then(|i| self.store.settings.songs.get(i));
                    let pos = self.music.position();
                    let (title, artist, secs) = match song {
                        Some(s) => (
                            s.title.as_str(),
                            if s.artist.is_empty() {
                                "Your music"
                            } else {
                                s.artist.as_str()
                            },
                            s.secs,
                        ),
                        None => ("Nothing playing", "Press play to start your music", 0.0),
                    };
                    let time = if song.is_some() {
                        format!("{} / {}", mmss(pos), mmss(secs))
                    } else {
                        String::new()
                    };
                    let tg = p.layout_no_wrap(time, theme::regular(12.0), glass::QUIET);
                    let text_right = r.right() - tg.size().x - 12.0;
                    p.galley(
                        Pos2::new(r.right() - tg.size().x, r.top() + 4.0),
                        tg,
                        glass::QUIET,
                    );
                    let clip = p.with_clip_rect(Rect::from_min_max(
                        r.min,
                        Pos2::new(text_right, r.bottom()),
                    ));
                    let tg = clip.layout_no_wrap(
                        title.to_string(),
                        theme::semibold(13.0),
                        Color32::WHITE,
                    );
                    let tw = tg.size().x;
                    clip.galley(Pos2::new(r.left(), r.top() + 3.0), tg, Color32::WHITE);
                    clip.text(
                        Pos2::new(r.left() + tw, r.top() + 3.0),
                        Align2::LEFT_TOP,
                        format!(" · {artist}"),
                        theme::regular(13.0),
                        glass::QUIET,
                    );
                    let bar = Rect::from_min_size(
                        Pos2::new(r.left(), r.bottom() - 9.0),
                        Vec2::new(r.width(), 3.0),
                    );
                    p.rect_filled(bar, CornerRadius::same(2), Color32::from_white_alpha(51));
                    let frac = if secs > 0.0 {
                        (pos / secs).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    p.rect_filled(
                        Rect::from_min_size(bar.min, Vec2::new(bar.width() * frac, 3.0)),
                        CornerRadius::same(2),
                        Color32::WHITE,
                    );
                });
            });
        act
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
