//! Your own music while riding: MP3s copied into the data folder, played
//! one after another (or shuffled) on the computer's speakers.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// A song you added.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Song {
    /// Its file name in the data folder's `music`.
    pub file: String,
    pub title: String,
    pub artist: String,
    /// Length in seconds, 0 when unknown.
    pub secs: f32,
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
