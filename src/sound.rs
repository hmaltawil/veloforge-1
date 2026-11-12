//! Audio cues for workouts. Tones are rendered to small WAV files once and
//! played with the desktop's command-line player (PipeWire, PulseAudio or
//! ALSA), so no audio libraries are needed at build time.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    /// Countdown tick before an interval change.
    Tick,
    /// The interval changes now.
    Change,
    /// Workout complete.
    Done,
    /// Something needs attention (e.g. the trainer disconnected).
    Alert,
}

impl Cue {
    const ALL: [Cue; 4] = [Cue::Tick, Cue::Change, Cue::Done, Cue::Alert];

    fn file(self) -> &'static str {
        match self {
            Cue::Tick => "tick.wav",
            Cue::Change => "change.wav",
            Cue::Done => "done.wav",
            Cue::Alert => "alert.wav",
        }
    }

    /// (frequency Hz, duration ms) notes; 0 Hz is a pause.
    fn notes(self) -> &'static [(f32, u32)] {
        match self {
            Cue::Tick => &[(880.0, 110)],
            Cue::Change => &[(1320.0, 380)],
            Cue::Done => &[
                (660.0, 160),
                (0.0, 40),
                (880.0, 160),
                (0.0, 40),
                (1320.0, 420),
            ],
            Cue::Alert => &[(440.0, 220), (0.0, 90), (330.0, 320)],
        }
    }
}

const RATE: u32 = 44_100;
const PLAYERS: [&str; 3] = ["pw-play", "paplay", "aplay"];

pub struct Sound {
    dir: PathBuf,
    player: Option<&'static str>,
    volume: f32,
}

impl Sound {
    pub fn new(dir: PathBuf, volume: f32) -> Sound {
        let player = PLAYERS.into_iter().find(|p| in_path(p));
        let s = Sound {
            dir,
            player,
            volume,
        };
        s.write_files();
        s
    }

    /// Name of the audio player in use, if one was found.
    pub fn player(&self) -> Option<&'static str> {
        self.player
    }

    pub fn set_volume(&mut self, volume: f32) {
        if (volume - self.volume).abs() > 1e-3 {
            self.volume = volume;
            self.write_files();
        }
    }

    pub fn play(&self, cue: Cue) {
        let Some(player) = self.player else { return };
        let child = Command::new(player)
            .arg(self.dir.join(cue.file()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut child) = child {
            // Reap the process so it does not linger as a zombie.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }

    fn write_files(&self) {
        if std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        for cue in Cue::ALL {
            let _ = std::fs::write(
                self.dir.join(cue.file()),
                wav(&render(cue.notes(), self.volume)),
            );
        }
    }
}

fn in_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| is_executable(&dir.join(program))))
        .unwrap_or(false)
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Sine tones with short fades so they do not click.
fn render(notes: &[(f32, u32)], volume: f32) -> Vec<i16> {
    let amp = volume.clamp(0.0, 1.0) * 0.8 * i16::MAX as f32;
    let fade = (RATE / 125) as usize; // 8 ms
    let mut out = Vec::new();
    for &(freq, ms) in notes {
        let n = (RATE * ms / 1000) as usize;
        for i in 0..n {
            let env = (i.min(n - 1 - i) as f32 / fade as f32).min(1.0);
            let v = if freq > 0.0 {
                (i as f32 * freq * std::f32::consts::TAU / RATE as f32).sin()
            } else {
                0.0
            };
            out.push((v * env * amp) as i16);
        }
    }
    out
}

/// 16-bit mono PCM WAV.
fn wav(samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend(b"RIFF");
    b.extend((36 + data_len).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes()); // fmt chunk size
    b.extend(1u16.to_le_bytes()); // PCM
    b.extend(1u16.to_le_bytes()); // mono
    b.extend(RATE.to_le_bytes());
    b.extend((RATE * 2).to_le_bytes()); // byte rate
    b.extend(2u16.to_le_bytes()); // block align
    b.extend(16u16.to_le_bytes()); // bits per sample
    b.extend(b"data");
    b.extend(data_len.to_le_bytes());
    for s in samples {
        b.extend(s.to_le_bytes());
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_and_length() {
        let samples = render(Cue::Tick.notes(), 0.5);
        assert_eq!(samples.len(), (RATE * 110 / 1000) as usize);
        // Fades in and out: silent at both ends, loud in the middle.
        assert_eq!(samples[0], 0);
        assert!(samples.iter().map(|s| s.unsigned_abs()).max().unwrap() > 10_000);
        let w = wav(&samples);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(
            u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize,
            samples.len() * 2
        );
        assert_eq!(w.len(), 44 + samples.len() * 2);
    }

    #[test]
    fn missing_player_is_silent() {
        let dir = std::env::temp_dir().join(format!("veloforge-sound-test-{}", std::process::id()));
        let mut s = Sound::new(dir.clone(), 0.5);
        s.player = None;
        s.play(Cue::Done); // must not panic
        assert!(dir.join("done.wav").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
