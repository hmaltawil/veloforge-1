//! Your own music while riding: MP3s from your music folder (played from
//! there) or added one by one (copied into the data folder), played one
//! after another (or shuffled) on the computer's speakers.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A song in the list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Song {
    /// Its file: for a song from your music folder, its path inside that
    /// folder (with `/` between folders); otherwise its name in the data
    /// folder's `music`.
    pub file: String,
    pub title: String,
    pub artist: String,
    /// Length in seconds, 0 when unknown.
    pub secs: f32,
    /// From your music folder, played from there rather than copied.
    #[serde(default)]
    pub folder: bool,
}

impl Song {
    /// Tells this song from every other in the list, even one with the same
    /// file name from the other place.
    pub fn key(&self) -> String {
        format!("{}:{}", if self.folder { 'f' } else { 'c' }, self.file)
    }

    /// Where its file is: in the copied songs (`copied`) or your music
    /// folder.
    pub fn path(&self, copied: &Path, folder: Option<&Path>) -> Option<PathBuf> {
        if self.folder {
            folder.map(|f| f.join(&self.file))
        } else {
            Some(copied.join(&self.file))
        }
    }
}

/// The MP3s in `dir` and its folders, as paths inside it with `/`
/// between folders, sorted; None when `dir` can't be read (a drive that
/// isn't plugged in). Hidden files (and the `._` files macOS leaves on
/// other drives) are left out, and links to folders are not followed.
pub fn find_mp3s(dir: &Path) -> Option<Vec<String>> {
    fn walk(dir: &Path, prefix: &str, depth: u32, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = e.file_type() else {
                continue;
            };
            let rel = format!("{prefix}{name}");
            if kind.is_dir() {
                if depth < 12 {
                    walk(&e.path(), &format!("{rel}/"), depth + 1, out);
                }
            } else if name.to_ascii_lowercase().ends_with(".mp3") {
                out.push(rel);
            }
        }
    }
    std::fs::read_dir(dir).ok()?;
    let mut out = Vec::new();
    walk(dir, "", 0, &mut out);
    out.sort_by_key(|a| a.to_lowercase());
    Some(out)
}

/// A song from the MP3 at `path`: its length, and its title and artist
/// from its tags or its name. None when it isn't an MP3 that plays.
pub fn read_song(path: &Path, file: String, folder: bool) -> Option<Song> {
    let secs = duration(path)?;
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let (name_title, name_artist) = names_from_file(&stem);
    let tags = read_tags(path);
    Some(Song {
        file,
        title: tags.title.unwrap_or(name_title),
        artist: tags.artist.unwrap_or(name_artist),
        secs,
        folder,
    })
}

/// Copy the MP3 at `src` (or its `bytes`) into `dir`, under a name not
/// taken yet, and read it.
pub fn copy_in(src: &Path, bytes: Option<&[u8]>, dir: &Path) -> Result<Song, String> {
    let base = src
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "song.mp3".into());
    let data = match bytes {
        Some(b) => b.to_vec(),
        None => std::fs::read(src).map_err(|e| format!("Could not read {base}: {e}"))?,
    };
    let _ = std::fs::create_dir_all(dir);
    let mut name = base.clone();
    let mut n = 2;
    while dir.join(&name).exists() {
        name = format!("{n}-{base}");
        n += 1;
    }
    let dest = dir.join(&name);
    std::fs::write(&dest, &data).map_err(|e| format!("Could not keep {base}: {e}"))?;
    let Some(secs) = duration(&dest) else {
        let _ = std::fs::remove_file(&dest);
        return Err(format!("{base} is not an MP3 VeloForge can play"));
    };
    // Named after the file it came from, not its copy's name.
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let (title, artist) = names_from_file(&stem);
    let tags = read_tags(&dest);
    Ok(Song {
        file: name,
        title: tags.title.unwrap_or(title),
        artist: tags.artist.unwrap_or(artist),
        secs,
        folder: false,
    })
}

/// Bring the list up to date with what is in the music folder now
/// (`found`): its songs that are gone leave the list, the others keep
/// their places, and `new` ones go at the end. Returns whether it changed.
pub fn merge_folder(songs: &mut Vec<Song>, found: &[String], new: Vec<Song>) -> bool {
    let here: std::collections::HashSet<&str> = found.iter().map(|s| s.as_str()).collect();
    let before = songs.len();
    songs.retain(|s| !s.folder || here.contains(s.file.as_str()));
    let mut changed = songs.len() != before;
    for song in new {
        if !songs.iter().any(|s| s.key() == song.key()) {
            songs.push(song);
            changed = true;
        }
    }
    changed
}

/// What happens at the end of the list (or of a song).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Repeat {
    /// Stop after the last song.
    Off,
    /// Start the list again.
    #[default]
    All,
    /// Play the same song again.
    One,
}

/// What a button can do to the music.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    PlayPause,
    Next,
    Previous,
}

/// Title and artist from a file name: "Artist - Title" when it has a
/// dash, otherwise the whole name as the title.
pub fn names_from_file(stem: &str) -> (String, String) {
    let stem = stem.replace('_', " ");
    match stem.split_once(" - ") {
        Some((artist, title)) if !artist.trim().is_empty() && !title.trim().is_empty() => {
            (title.trim().to_string(), artist.trim().to_string())
        }
        _ => (stem.trim().to_string(), String::new()),
    }
}

/// What an MP3's tags say: its title, artist and cover picture.
#[derive(Debug, Default)]
pub struct Tags {
    pub title: Option<String>,
    pub artist: Option<String>,
    /// The picture's bytes (JPEG or PNG), the front cover when marked.
    pub cover: Option<Vec<u8>>,
}

/// Read an MP3's tags (ID3), if it has any.
pub fn read_tags(path: &Path) -> Tags {
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::{
        MetadataOptions, MetadataRevision, StandardTagKey, StandardVisualKey,
    };
    use symphonia::core::probe::Hint;
    let mut tags = Tags::default();
    let Ok(file) = std::fs::File::open(path) else {
        return tags;
    };
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let Ok(mut probed) = symphonia::default::get_probe().format(
        &hint,
        stream,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    ) else {
        return tags;
    };
    let mut read = |rev: &MetadataRevision| {
        for t in rev.tags() {
            let value = t.value.to_string().trim().to_string();
            if value.is_empty() {
                continue;
            }
            match t.std_key {
                Some(StandardTagKey::TrackTitle) if tags.title.is_none() => {
                    tags.title = Some(value)
                }
                Some(StandardTagKey::Artist) if tags.artist.is_none() => tags.artist = Some(value),
                Some(StandardTagKey::AlbumArtist) if tags.artist.is_none() => {
                    tags.artist = Some(value)
                }
                _ => {}
            }
        }
        let front = rev
            .visuals()
            .iter()
            .find(|v| v.usage == Some(StandardVisualKey::FrontCover))
            .or_else(|| rev.visuals().first());
        if tags.cover.is_none() {
            tags.cover = front.map(|v| v.data.to_vec());
        }
    };
    // ID3 at the start of the file is read while probing; other tags
    // come with the format.
    if let Some(rev) = probed.metadata.get().as_ref().and_then(|m| m.current()) {
        read(rev);
    }
    if let Some(rev) = probed.format.metadata().current() {
        read(rev);
    }
    tags
}

/// How long an MP3 plays, if its decoder can tell.
pub fn duration(path: &Path) -> Option<f32> {
    use rodio::Source;
    let f = std::fs::File::open(path).ok()?;
    let d = rodio::Decoder::new_mp3(std::io::BufReader::new(f)).ok()?;
    d.total_duration().map(|d| d.as_secs_f32())
}

/// The song to play after (`forward`) or before `current` in a list of
/// `len`: in order or shuffled (`pick` is a random number 0-1), and at the
/// end of the list starting again, stopping or repeating as `repeat` says.
/// Skipping by hand always moves on, even with Repeat One.
pub fn step(
    current: Option<usize>,
    len: usize,
    shuffle: bool,
    repeat: Repeat,
    forward: bool,
    by_hand: bool,
    pick: f32,
) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let Some(i) = current else {
        return Some(if shuffle {
            ((pick * len as f32) as usize).min(len - 1)
        } else {
            0
        });
    };
    if repeat == Repeat::One && !by_hand {
        return Some(i);
    }
    if shuffle && len > 1 {
        // Any other song.
        let j = ((pick * (len - 1) as f32) as usize).min(len - 2);
        return Some(if j >= i { j + 1 } else { j });
    }
    if forward {
        if i + 1 < len {
            Some(i + 1)
        } else if repeat == Repeat::Off && !by_hand {
            None
        } else {
            Some(0)
        }
    } else if i > 0 {
        Some(i - 1)
    } else {
        Some(len - 1)
    }
}

/// The speakers and the song playing on them. The speakers are opened the
/// first time a song plays, so a ride without music never touches them.
#[derive(Default)]
pub struct Player {
    output: Option<(rodio::OutputStream, rodio::OutputStreamHandle)>,
    sink: Option<rodio::Sink>,
    /// The song playing or paused, an index into the settings' songs.
    pub current: Option<usize>,
}

impl Player {
    /// Play `path` from its start, as song `index`.
    pub fn play(&mut self, index: usize, path: &Path, volume: f32) -> Result<(), String> {
        if self.output.is_none() {
            self.output = Some(
                rodio::OutputStream::try_default()
                    .map_err(|e| format!("Could not open the speakers: {e}"))?,
            );
        }
        let Some((_, handle)) = &self.output else {
            return Err("No speakers".into());
        };
        let f = std::fs::File::open(path).map_err(|e| format!("Could not open the song: {e}"))?;
        let source = rodio::Decoder::new_mp3(std::io::BufReader::new(f))
            .map_err(|e| format!("Could not read the song: {e}"))?;
        let sink = rodio::Sink::try_new(handle).map_err(|e| format!("Could not play: {e}"))?;
        sink.set_volume(volume);
        sink.append(source);
        sink.play();
        self.sink = Some(sink);
        self.current = Some(index);
        Ok(())
    }

    pub fn pause(&self) {
        if let Some(s) = &self.sink {
            s.pause();
        }
    }

    pub fn resume(&self) {
        if let Some(s) = &self.sink {
            s.play();
        }
    }

    pub fn stop(&mut self) {
        if let Some(s) = self.sink.take() {
            s.stop();
        }
        self.current = None;
    }

    /// A song is loaded and not paused.
    pub fn is_playing(&self) -> bool {
        self.sink
            .as_ref()
            .is_some_and(|s| !s.is_paused() && !s.empty())
    }

    /// The song has played to its end.
    pub fn finished(&self) -> bool {
        self.sink.as_ref().is_some_and(|s| s.empty())
    }

    /// Seconds into the song.
    pub fn position(&self) -> f32 {
        self.sink
            .as_ref()
            .map(|s| s.get_pos().as_secs_f32())
            .unwrap_or(0.0)
    }

    /// Back to the start of the song.
    pub fn restart(&self) {
        if let Some(s) = &self.sink {
            let _ = s.try_seek(std::time::Duration::ZERO);
        }
    }

    pub fn set_volume(&self, volume: f32) {
        if let Some(s) = &self.sink {
            s.set_volume(volume);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_come_from_the_file_name() {
        assert_eq!(
            names_from_file("Daft Punk - One More Time"),
            ("One More Time".into(), "Daft Punk".into())
        );
        assert_eq!(
            names_from_file("track_01"),
            ("track 01".into(), String::new())
        );
        assert_eq!(names_from_file(" - x"), ("- x".into(), String::new()));
    }

    fn song(file: &str, folder: bool) -> Song {
        Song {
            file: file.into(),
            title: file.into(),
            artist: String::new(),
            secs: 60.0,
            folder,
        }
    }

    #[test]
    fn finding_the_songs_in_a_folder() {
        let dir = std::env::temp_dir().join(format!("vf-music-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for f in [
            "b.mp3",
            "A.MP3",
            "notes.txt",
            ".hidden.mp3",
            "Album/._c.mp3",
            "Album/c.mp3",
            "Album/Disc 2/d.mp3",
        ] {
            let p = dir.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        }
        let found = find_mp3s(&dir).unwrap();
        assert_eq!(
            found,
            vec!["A.MP3", "Album/c.mp3", "Album/Disc 2/d.mp3", "b.mp3"]
        );
        // A song's path from the folder finds the file on any system.
        let s = song("Album/Disc 2/d.mp3", true);
        assert!(s.path(Path::new("/nowhere"), Some(&dir)).unwrap().exists());
        assert!(s.path(Path::new("/nowhere"), None).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
        // A folder that isn't there (a drive not plugged in).
        assert_eq!(find_mp3s(&dir), None);
    }

    #[test]
    fn the_list_follows_the_folder() {
        // Your order, with a song you added among the folder's.
        let mut songs = vec![
            song("c.mp3", true),
            song("x.mp3", false),
            song("a.mp3", true),
        ];
        // a.mp3 was deleted from the folder, b.mp3 is new.
        let found = vec!["b.mp3".to_string(), "c.mp3".to_string()];
        assert!(merge_folder(&mut songs, &found, vec![song("b.mp3", true)]));
        let files: Vec<_> = songs.iter().map(|s| s.key()).collect();
        assert_eq!(files, ["f:c.mp3", "c:x.mp3", "f:b.mp3"]);
        // Nothing new: nothing changes, and no song comes twice.
        assert!(!merge_folder(&mut songs, &found, vec![song("b.mp3", true)]));
        assert_eq!(songs.len(), 3);
        // An added song and a folder song may share a name.
        assert_ne!(song("x.mp3", true).key(), song("x.mp3", false).key());
    }

    #[test]
    fn stepping_through_the_list() {
        use Repeat::*;
        // In order, then round again, or stop at the end.
        assert_eq!(step(None, 3, false, All, true, false, 0.0), Some(0));
        assert_eq!(step(Some(1), 3, false, All, true, false, 0.0), Some(2));
        assert_eq!(step(Some(2), 3, false, All, true, false, 0.0), Some(0));
        assert_eq!(step(Some(2), 3, false, Off, true, false, 0.0), None);
        // By hand, the end always wraps round.
        assert_eq!(step(Some(2), 3, false, Off, true, true, 0.0), Some(0));
        assert_eq!(step(Some(0), 3, false, All, false, true, 0.0), Some(2));
        // Repeat One stays, unless skipped by hand.
        assert_eq!(step(Some(1), 3, false, One, true, false, 0.0), Some(1));
        assert_eq!(step(Some(1), 3, false, One, true, true, 0.0), Some(2));
        // Shuffle never picks the same song twice in a row.
        for k in 0..20 {
            let pick = k as f32 / 20.0;
            assert_ne!(step(Some(1), 3, true, All, true, false, pick), Some(1));
        }
        assert_eq!(step(None, 0, false, All, true, false, 0.0), None);
    }
}
