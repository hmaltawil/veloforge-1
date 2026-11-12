//! Reading saved rides back from TCX files: for the ride details view and
//! for recovering a ride interrupted by a crash or power cut.

use crate::ride::Sample;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

pub struct Activity {
    pub name: Option<String>,
    pub samples: Vec<Sample>,
}

pub fn parse_tcx(xml: &str) -> Result<Activity> {
    let doc = roxmltree::Document::parse(xml).context("not a valid TCX file")?;
    let child_text = |n: roxmltree::Node, tag: &str| -> Option<String> {
        n.children()
            .find(|c| c.has_tag_name(tag))
            .and_then(|c| c.text())
            .map(|t| t.trim().to_string())
    };
    let num =
        |n: roxmltree::Node, tag: &str| child_text(n, tag).and_then(|t| t.parse::<f64>().ok());
    let mut samples = Vec::new();
    for (i, tp) in doc
        .descendants()
        .filter(|n| n.has_tag_name("Trackpoint"))
        .enumerate()
    {
        let Some(time) = child_text(tp, "Time").and_then(|t| DateTime::parse_from_rfc3339(&t).ok())
        else {
            continue;
        };
        let hr = tp
            .children()
            .find(|c| c.has_tag_name("HeartRateBpm"))
            .and_then(|h| num(h, "Value"))
            .map(|v| v as f32);
        let ext = tp.descendants().find(|c| c.tag_name().name() == "TPX");
        let ext_num = |tag: &str| {
            ext.and_then(|e| e.children().find(|c| c.tag_name().name() == tag))
                .and_then(|c| c.text())
                .and_then(|t| t.trim().parse::<f32>().ok())
        };
        samples.push(Sample {
            time: time.with_timezone(&Utc),
            elapsed: i as f32,
            power: ext_num("Watts").unwrap_or(0.0),
            cadence: num(tp, "Cadence").map(|v| v as f32),
            hr,
            speed: ext_num("Speed").unwrap_or(0.0),
            distance: num(tp, "DistanceMeters").unwrap_or(0.0),
            ele: num(tp, "AltitudeMeters").unwrap_or(0.0),
        });
    }
    let name = doc
        .descendants()
        .find(|n| n.has_tag_name("Notes"))
        .and_then(|n| n.text())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    Ok(Activity { name, samples })
}

/// Move a file to the desktop's Trash (the freedesktop.org layout that
/// Dolphin, Nautilus and others use), so it can be restored from there.
pub fn move_to_trash(path: &std::path::Path) -> std::io::Result<()> {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
        })
        .ok_or_else(|| std::io::Error::other("no home folder"))?;
    trash_into(
        path,
        &data_home.join("Trash"),
        chrono::Local::now().naive_local(),
    )
}

fn trash_into(
    path: &std::path::Path,
    trash: &std::path::Path,
    now: chrono::NaiveDateTime,
) -> std::io::Result<()> {
    use std::io::Write;
    let path = std::fs::canonicalize(path)?;
    let files = trash.join("files");
    let info = trash.join("info");
    std::fs::create_dir_all(&files)?;
    std::fs::create_dir_all(&info)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| std::io::Error::other("bad file name"))?;
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (name.to_string(), String::new()),
    };
    // Reserve a free name by creating its .trashinfo file first.
    for n in 1..1000 {
        let candidate = if n == 1 {
            name.to_string()
        } else {
            format!("{stem}.{n}{ext}")
        };
        let info_path = info.join(format!("{candidate}.trashinfo"));
        let mut f = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&info_path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let target = files.join(&candidate);
        if target.exists() {
            let _ = std::fs::remove_file(&info_path);
            continue;
        }
        write!(
            f,
            "[Trash Info]\nPath={}\nDeletionDate={}\n",
            url_path(&path),
            now.format("%Y-%m-%dT%H:%M:%S")
        )?;
        let moved = std::fs::rename(&path, &target).or_else(|_| {
            // A different drive: copy, then remove the original.
            std::fs::copy(&path, &target).and_then(|_| std::fs::remove_file(&path))
        });
        if moved.is_err() {
            let _ = std::fs::remove_file(&info_path);
        }
        return moved;
    }
    Err(std::io::Error::other(
        "too many files with this name in the Trash",
    ))
}

/// Percent-encode a path for a .trashinfo file.
fn url_path(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str()
        .as_bytes()
        .iter()
        .map(|&b| {
            if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Best average power over common durations: (label, seconds, watts).
pub fn best_efforts(samples: &[Sample]) -> Vec<(&'static str, usize, f32)> {
    let p: Vec<f32> = samples.iter().map(|s| s.power).collect();
    [
        ("5 s", 5),
        ("1 min", 60),
        ("5 min", 300),
        ("20 min", 1200),
        ("60 min", 3600),
    ]
    .into_iter()
    .filter_map(|(label, secs)| crate::workout::best_average(&p, secs).map(|w| (label, secs, w)))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ride::{Live, Mode, Physics, Ride};
    use crate::route;

    #[test]
    fn tcx_roundtrip() {
        let r = &route::builtin()[1];
        let mut ride = Ride::new(Mode::Route, None, None);
        ride.start();
        let phys = Physics {
            mass: 83.0,
            cda: 0.32,
            crr: 0.004,
            trainer_speed: true,
        };
        for i in 0..900 {
            let live = Live {
                power: 150.0 + (i % 50) as f32,
                cadence: Some(88.0),
                hr: Some(130.0),
                speed_kmh: None,
            };
            ride.update(0.1, live, r, phys, 250.0, 0.5, true);
        }
        let a = parse_tcx(&ride.to_tcx()).unwrap();
        assert_eq!(a.name.as_deref(), Some("Free ride"));
        assert_eq!(a.samples.len(), ride.samples.len());
        let (x, y) = (&a.samples[50], &ride.samples[50]);
        assert_eq!(x.power, y.power.round());
        assert_eq!(x.hr, Some(130.0));
        assert_eq!(x.cadence, Some(88.0));
        assert!((x.distance - y.distance).abs() < 0.1);
        assert!(best_efforts(&a.samples).iter().any(|b| b.0 == "1 min"));
    }

    #[test]
    fn trashed_rides_can_be_restored() {
        let dir = std::env::temp_dir().join(format!("vf-trash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let acts = dir.join("my rides");
        std::fs::create_dir_all(&acts).unwrap();
        let trash = dir.join("Trash");
        let now = chrono::NaiveDate::from_ymd_opt(2026, 10, 31)
            .unwrap()
            .and_hms_opt(20, 5, 0)
            .unwrap();
        for _ in 0..2 {
            let f = acts.join("2026-10-31_Free_ride.tcx");
            std::fs::write(&f, "<tcx/>").unwrap();
            trash_into(&f, &trash, now).unwrap();
            assert!(!f.exists());
        }
        // The second file with the same name gets a new name.
        assert!(trash.join("files/2026-10-31_Free_ride.tcx").exists());
        assert!(trash.join("files/2026-10-31_Free_ride.2.tcx").exists());
        let info =
            std::fs::read_to_string(trash.join("info/2026-10-31_Free_ride.tcx.trashinfo")).unwrap();
        assert!(info.starts_with("[Trash Info]\n"));
        assert!(
            info.contains("/my%20rides/2026-10-31_Free_ride.tcx\n"),
            "{info}"
        );
        assert!(info.contains("DeletionDate=2026-10-31T20:05:00"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_tcx("not xml").is_err());
        assert!(parse_tcx("<TrainingCenterDatabase/>")
            .unwrap()
            .samples
            .is_empty());
    }
}
