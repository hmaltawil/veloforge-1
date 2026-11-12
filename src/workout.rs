//! Structured workouts: model, built-in library, Zwift .zwo import and
//! training-load estimates. Power targets are fractions of FTP.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Category {
    Recovery,
    Endurance,
    Tempo,
    SweetSpot,
    Threshold,
    Vo2Max,
    Anaerobic,
    Test,
    Custom,
    /// Planned on intervals.icu.
    Intervals,
}

impl Category {
    pub const ALL: [Category; 10] = [
        Category::Recovery,
        Category::Endurance,
        Category::Tempo,
        Category::SweetSpot,
        Category::Threshold,
        Category::Vo2Max,
        Category::Anaerobic,
        Category::Test,
        Category::Custom,
        Category::Intervals,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Recovery => "Recovery",
            Category::Endurance => "Endurance",
            Category::Tempo => "Tempo",
            Category::SweetSpot => "Sweet spot",
            Category::Threshold => "Threshold",
            Category::Vo2Max => "VO2 max",
            Category::Anaerobic => "Anaerobic",
            Category::Test => "FTP test",
            Category::Custom => "Custom",
            Category::Intervals => "From intervals.icu",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Step {
    Steady {
        secs: u32,
        power: f32,
    },
    Ramp {
        secs: u32,
        from: f32,
        to: f32,
    },
    Intervals {
        repeat: u32,
        on_secs: u32,
        on_power: f32,
        off_secs: u32,
        off_power: f32,
    },
    /// Rider-controlled section: the trainer follows the route grade.
    FreeRide {
        secs: u32,
    },
}

impl Step {
    pub fn secs(&self) -> u32 {
        match *self {
            Step::Steady { secs, .. } | Step::Ramp { secs, .. } | Step::FreeRide { secs } => secs,
            Step::Intervals {
                repeat,
                on_secs,
                off_secs,
                ..
            } => repeat * (on_secs + off_secs),
        }
    }

    pub fn describe(&self) -> String {
        let pct = |p: f32| format!("{:.0}%", p * 100.0);
        match *self {
            Step::Steady { secs, power } => format!("{} @ {}", fmt_dur(secs as f32), pct(power)),
            Step::Ramp { secs, from, to } => {
                format!("{} ramp {} to {}", fmt_dur(secs as f32), pct(from), pct(to))
            }
            Step::Intervals {
                repeat,
                on_secs,
                on_power,
                off_secs,
                off_power,
            } => format!(
                "{repeat}× ({} @ {} / {} @ {})",
                fmt_dur(on_secs as f32),
                pct(on_power),
                fmt_dur(off_secs as f32),
                pct(off_power)
            ),
            Step::FreeRide { secs } => format!("{} free ride", fmt_dur(secs as f32)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub start: f32,
    pub secs: f32,
    pub from: f32,
    pub to: f32,
    pub free: bool,
}

impl Segment {
    pub fn end(&self) -> f32 {
        self.start + self.secs
    }
    pub fn at(&self, t: f32) -> f32 {
        let u = ((t - self.start) / self.secs.max(1e-3)).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * u
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workout {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: Category,
    pub steps: Vec<Step>,
}

impl Workout {
    pub fn segments(&self) -> Vec<Segment> {
        let mut out = Vec::new();
        let mut t = 0.0;
        let mut push = |secs: u32, from: f32, to: f32, free: bool, t: &mut f32| {
            if secs > 0 {
                out.push(Segment {
                    start: *t,
                    secs: secs as f32,
                    from,
                    to,
                    free,
                });
                *t += secs as f32;
            }
        };
        for s in &self.steps {
            match *s {
                Step::Steady { secs, power } => push(secs, power, power, false, &mut t),
                Step::Ramp { secs, from, to } => push(secs, from, to, false, &mut t),
                Step::FreeRide { secs } => push(secs, 0.5, 0.5, true, &mut t),
                Step::Intervals {
                    repeat,
                    on_secs,
                    on_power,
                    off_secs,
                    off_power,
                } => {
                    for _ in 0..repeat {
                        push(on_secs, on_power, on_power, false, &mut t);
                        push(off_secs, off_power, off_power, false, &mut t);
                    }
                }
            }
        }
        out
    }

    pub fn duration(&self) -> f32 {
        self.steps.iter().map(|s| s.secs() as f32).sum()
    }

    /// Estimated (IF, TSS) assuming the targets are ridden exactly and free
    /// ride sections at 60% FTP.
    pub fn load(&self) -> (f32, f32) {
        let per_sec: Vec<f32> = self
            .segments()
            .iter()
            .flat_map(|s| {
                (0..s.secs as u32).map(move |i| {
                    if s.free {
                        0.6
                    } else {
                        s.at(s.start + i as f32)
                    }
                })
            })
            .collect();
        let np = normalized_power(&per_sec);
        let hours = per_sec.len() as f32 / 3600.0;
        (np, hours * np * np * 100.0)
    }

    pub fn max_target(&self) -> f32 {
        self.segments()
            .iter()
            .map(|s| s.from.max(s.to))
            .fold(0.0, f32::max)
    }
}

/// Returns the target (fraction of FTP, is_free_ride) at workout time `t`.
pub fn target_at(segs: &[Segment], t: f32) -> Option<(f32, bool)> {
    let i = segs.partition_point(|s| s.end() <= t);
    segs.get(i).map(|s| (s.at(t), s.free))
}

pub fn segment_index(segs: &[Segment], t: f32) -> Option<usize> {
    let i = segs.partition_point(|s| s.end() <= t);
    (i < segs.len()).then_some(i)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Beep {
    /// 3, 2, 1 seconds before a change.
    Countdown,
    /// The target changes now.
    Change,
    /// The workout is over.
    Finish,
}

/// The audio cue due when the workout clock moves from `before` to `after`,
/// if any. Only boundaries where the target really changes are announced.
pub fn beep_between(segs: &[Segment], before: f32, after: f32) -> Option<Beep> {
    for (i, s) in segs.iter().enumerate() {
        let b = s.end();
        if b <= before || b - 3.0 > after {
            continue;
        }
        let next = segs.get(i + 1);
        let changes = match next {
            None => true,
            Some(n) => n.free != s.free || (n.from - s.to).abs() >= 0.03,
        };
        if !changes {
            continue;
        }
        if before < b && b <= after {
            return Some(if next.is_none() {
                Beep::Finish
            } else {
                Beep::Change
            });
        }
        if [1.0, 2.0, 3.0]
            .iter()
            .any(|k| before < b - k && b - k <= after)
        {
            return Some(Beep::Countdown);
        }
    }
    None
}

/// Normalized power over 1 Hz samples (30 s rolling mean, 4th power mean).
pub fn normalized_power(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    if samples.len() < 30 {
        return samples.iter().sum::<f32>() / samples.len() as f32;
    }
    let mut sum: f64 = samples[..30].iter().map(|&v| v as f64).sum();
    let mut acc = (sum / 30.0).powi(4);
    let mut n = 1.0;
    for i in 30..samples.len() {
        sum += samples[i] as f64 - samples[i - 30] as f64;
        acc += (sum / 30.0).powi(4);
        n += 1.0;
    }
    (acc / n).powf(0.25) as f32
}

/// Best average power over a window of `secs` samples.
pub fn best_average(samples: &[f32], secs: usize) -> Option<f32> {
    if secs == 0 || samples.len() < secs {
        return None;
    }
    let mut sum: f64 = samples[..secs].iter().map(|&v| v as f64).sum();
    let mut best = sum;
    for i in secs..samples.len() {
        sum += samples[i] as f64 - samples[i - secs] as f64;
        best = best.max(sum);
    }
    Some((best / secs as f64) as f32)
}

pub fn fmt_dur(secs: f32) -> String {
    let s = secs.max(0.0).round() as u32;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else if s.is_multiple_of(60) && s >= 60 {
        format!("{}m", s / 60)
    } else if s >= 60 {
        format!("{}:{:02}", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

/// Parse a Zwift workout file (.zwo), as exported by intervals.icu,
/// TrainerRoad, WhatsOnZwift and others.
pub fn parse_zwo(xml: &str, id: &str) -> Result<Workout> {
    let doc = roxmltree::Document::parse(xml).context("invalid XML")?;
    let root = doc.root_element();
    let text = |tag: &str| {
        root.children()
            .find(|n| n.has_tag_name(tag))
            .and_then(|n| n.text())
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    let wk = root
        .children()
        .find(|n| n.has_tag_name("workout"))
        .context("no <workout> element")?;
    let mut steps = Vec::new();
    for n in wk.children().filter(|n| n.is_element()) {
        let num = |k: &str| n.attribute(k).and_then(|v| v.trim().parse::<f32>().ok());
        let secs = num("Duration").unwrap_or(0.0).round() as u32;
        let lo = num("PowerLow").or(num("Power")).unwrap_or(0.5);
        let hi = num("PowerHigh").or(num("Power")).unwrap_or(lo);
        let step = match n.tag_name().name() {
            "Warmup" | "Ramp" => Step::Ramp {
                secs,
                from: lo,
                to: hi,
            },
            // Cooldowns always descend, whichever order the attributes use.
            "Cooldown" => Step::Ramp {
                secs,
                from: lo.max(hi),
                to: lo.min(hi),
            },
            "SteadyState" | "SolidState" => Step::Steady {
                secs,
                power: num("Power").unwrap_or(lo),
            },
            "IntervalsT" => Step::Intervals {
                repeat: num("Repeat").unwrap_or(1.0).round() as u32,
                on_secs: num("OnDuration").unwrap_or(0.0).round() as u32,
                on_power: num("OnPower")
                    .or(num("PowerOnHigh"))
                    .or(num("PowerOnLow"))
                    .unwrap_or(1.0),
                off_secs: num("OffDuration").unwrap_or(0.0).round() as u32,
                off_power: num("OffPower")
                    .or(num("PowerOffLow"))
                    .or(num("PowerOffHigh"))
                    .unwrap_or(0.5),
            },
            "FreeRide" | "MaxEffort" => Step::FreeRide { secs },
            _ => continue,
        };
        if step.secs() > 0 {
            steps.push(step);
        }
    }
    anyhow::ensure!(!steps.is_empty(), "workout has no steps");
    let name = text("name");
    Ok(Workout {
        id: id.to_string(),
        name: if name.is_empty() {
            id.to_string()
        } else {
            name
        },
        description: text("description"),
        category: Category::Custom,
        steps,
    })
}

// ---------------------------------------------------------------------------
// Built-in library

const MIN: u32 = 60;

fn warmup(mins: u32) -> Step {
    Step::Ramp {
        secs: mins * MIN,
        from: 0.45,
        to: 0.72,
    }
}

fn cooldown(mins: u32) -> Step {
    Step::Ramp {
        secs: mins * MIN,
        from: 0.65,
        to: 0.40,
    }
}

fn steady(mins: u32, power: f32) -> Step {
    Step::Steady {
        secs: mins * MIN,
        power,
    }
}

fn reps(repeat: u32, on_secs: u32, on_power: f32, off_secs: u32, off_power: f32) -> Step {
    Step::Intervals {
        repeat,
        on_secs,
        on_power,
        off_secs,
        off_power,
    }
}

fn w(id: &str, name: &str, category: Category, description: &str, steps: Vec<Step>) -> Workout {
    Workout {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        category,
        steps,
    }
}

pub fn builtin() -> Vec<Workout> {
    use Category::*;
    let mut v = vec![
        w(
            "recovery-30",
            "Recovery Spin",
            Recovery,
            "Easy spin to flush the legs.",
            vec![
                Step::Ramp {
                    secs: 5 * MIN,
                    from: 0.40,
                    to: 0.50,
                },
                steady(20, 0.50),
                Step::Ramp {
                    secs: 5 * MIN,
                    from: 0.50,
                    to: 0.40,
                },
            ],
        ),
        w(
            "recovery-45",
            "Recovery Spin+",
            Recovery,
            "Longer easy spin with light cadence pickups.",
            vec![
                Step::Ramp {
                    secs: 5 * MIN,
                    from: 0.40,
                    to: 0.52,
                },
                reps(4, 6 * MIN, 0.52, 2 * MIN, 0.58),
                steady(3, 0.50),
                Step::Ramp {
                    secs: 5 * MIN,
                    from: 0.50,
                    to: 0.40,
                },
            ],
        ),
    ];
    for mins in [45, 60, 75, 90, 120] {
        v.push(w(
            &format!("endurance-{mins}"),
            &format!("Endurance {mins}"),
            Endurance,
            "Steady zone 2 aerobic riding.",
            vec![warmup(10), steady(mins - 15, 0.68), cooldown(5)],
        ));
    }
    v.extend([
        w("tempo-2x15", "Tempo 2×15", Tempo, "Two blocks of steady tempo.", vec![
            warmup(10), reps(2, 15 * MIN, 0.82, 5 * MIN, 0.55), cooldown(10),
        ]),
        w("tempo-3x15", "Tempo 3×15", Tempo, "Three blocks of steady tempo.", vec![
            warmup(10), reps(3, 15 * MIN, 0.83, 5 * MIN, 0.55), cooldown(10),
        ]),
        w("ss-3x10", "Sweet Spot 3×10", SweetSpot, "Sweet spot builds FTP with manageable fatigue.", vec![
            warmup(10), reps(3, 10 * MIN, 0.88, 5 * MIN, 0.55), cooldown(10),
        ]),
        w("ss-3x12", "Sweet Spot 3×12", SweetSpot, "Sweet spot builds FTP with manageable fatigue.", vec![
            warmup(10), reps(3, 12 * MIN, 0.90, 5 * MIN, 0.55), cooldown(8),
        ]),
        w("ss-3x15", "Sweet Spot 3×15", SweetSpot, "Sweet spot builds FTP with manageable fatigue.", vec![
            warmup(10), reps(3, 15 * MIN, 0.90, 5 * MIN, 0.55), cooldown(5),
        ]),
        w("ss-2x20", "Sweet Spot 2×20", SweetSpot, "Classic long sweet spot intervals.", vec![
            warmup(12), reps(2, 20 * MIN, 0.92, 6 * MIN, 0.55), cooldown(8),
        ]),
        w("thr-4x8", "Threshold 4×8", Threshold, "Threshold intervals just under FTP.", vec![
            warmup(12), reps(4, 8 * MIN, 0.98, 4 * MIN, 0.55), cooldown(8),
        ]),
        w("thr-3x10", "Threshold 3×10", Threshold, "Threshold intervals at FTP.", vec![
            warmup(12), reps(3, 10 * MIN, 1.00, 5 * MIN, 0.55), cooldown(8),
        ]),
        w("thr-3x12", "Threshold 3×12", Threshold, "Threshold intervals at FTP.", vec![
            warmup(12), reps(3, 12 * MIN, 1.00, 5 * MIN, 0.55), cooldown(8),
        ]),
        w("thr-2x20", "Threshold 2×20", Threshold, "The classic FTP workout.", vec![
            warmup(15), reps(2, 20 * MIN, 0.97, 8 * MIN, 0.55), cooldown(10),
        ]),
        w("ou-3x9", "Over-Unders 3×9", Threshold, "Alternate just under and over FTP to train lactate clearance.", {
            let mut s = vec![warmup(12)];
            for i in 0..3 {
                s.push(reps(3, 2 * MIN, 0.95, MIN, 1.08));
                if i < 2 {
                    s.push(steady(5, 0.55));
                }
            }
            s.push(cooldown(10));
            s
        }),
        w("vo2-5x3", "VO2 Max 5×3", Vo2Max, "Hard 3 minute efforts to raise your ceiling.", vec![
            warmup(12), reps(5, 3 * MIN, 1.15, 3 * MIN, 0.50), cooldown(10),
        ]),
        w("vo2-6x3", "VO2 Max 6×3", Vo2Max, "Hard 3 minute efforts to raise your ceiling.", vec![
            warmup(12), reps(6, 3 * MIN, 1.15, 3 * MIN, 0.50), cooldown(8),
        ]),
        w("vo2-5x4", "VO2 Max 5×4", Vo2Max, "Long VO2 efforts.", vec![
            warmup(12), reps(5, 4 * MIN, 1.12, 4 * MIN, 0.50), cooldown(8),
        ]),
        w("vo2-30-30", "30/30s", Vo2Max, "Two sets of 10×30 s on / 30 s off.", vec![
            warmup(12), reps(10, 30, 1.25, 30, 0.50), steady(6, 0.50), reps(10, 30, 1.25, 30, 0.50), cooldown(10),
        ]),
        w("sprints-8x30", "Sprint 8×30s", Anaerobic, "All-out-ish 30 s efforts with long rest.", vec![
            warmup(15), reps(8, 30, 1.60, 4 * MIN + 30, 0.50), cooldown(8),
        ]),
        w("ramp-test", "Ramp Test", Test, "Ride each 1-minute step until you can't hold it, then press Finish. FTP is estimated as 75% of your best minute.", {
            let mut s = vec![Step::Ramp { secs: 5 * MIN, from: 0.40, to: 0.50 }];
            for i in 0..25 {
                s.push(Step::Steady { secs: MIN, power: 0.40 + 0.06 * i as f32 });
            }
            s
        }),
        w("ftp-20", "20 Minute FTP Test", Test, "Free-ride the 20 minute block as hard as you can sustain. FTP is estimated as 95% of the 20 minute average.", vec![
            warmup(10),
            reps(3, MIN, 1.05, MIN, 0.55),
            steady(5, 0.55),
            Step::Steady { secs: 5 * MIN, power: 1.05 },
            steady(10, 0.50),
            Step::FreeRide { secs: 20 * MIN },
            cooldown(10),
        ]),
    ]);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_and_targets() {
        let wk = w(
            "t",
            "t",
            Category::Custom,
            "",
            vec![
                Step::Ramp {
                    secs: 100,
                    from: 0.5,
                    to: 1.0,
                },
                reps(2, 10, 1.2, 20, 0.5),
                Step::FreeRide { secs: 30 },
            ],
        );
        let segs = wk.segments();
        assert_eq!(segs.len(), 6);
        assert_eq!(wk.duration(), 190.0);
        assert!((target_at(&segs, 50.0).unwrap().0 - 0.75).abs() < 1e-5);
        assert_eq!(target_at(&segs, 105.0), Some((1.2, false)));
        assert_eq!(target_at(&segs, 115.0), Some((0.5, false)));
        assert!(target_at(&segs, 170.0).unwrap().1);
        assert_eq!(target_at(&segs, 190.0), None);
    }

    #[test]
    fn beeps_only_at_real_changes() {
        let wk = w(
            "t",
            "t",
            Category::Custom,
            "",
            vec![
                steady(1, 0.6),
                steady(1, 0.6), // same target: no cue at t=60
                reps(2, 30, 1.2, 30, 0.5),
            ],
        );
        let segs = wk.segments();
        let cues = |from: f32, to: f32| -> Vec<(f32, Beep)> {
            let mut out = Vec::new();
            let mut t = from;
            while t < to {
                if let Some(b) = beep_between(&segs, t, t + 0.1) {
                    out.push(((t + 0.1).floor(), b));
                }
                t += 0.1;
            }
            out
        };
        assert!(cues(55.0, 62.0).is_empty());
        let c = cues(115.0, 121.0);
        assert_eq!(
            c.iter().map(|x| x.1).collect::<Vec<_>>(),
            vec![
                Beep::Countdown,
                Beep::Countdown,
                Beep::Countdown,
                Beep::Change
            ]
        );
        assert_eq!(c.last().unwrap().0, 120.0);
        assert_eq!(cues(236.0, 241.0).last().map(|x| x.1), Some(Beep::Finish));
        // A long jump (window hidden) still yields a single cue.
        assert_eq!(beep_between(&segs, 100.0, 125.0), Some(Beep::Change));
    }

    #[test]
    fn one_hour_at_ftp_is_100_tss() {
        let wk = w("t", "t", Category::Custom, "", vec![steady(60, 1.0)]);
        let (ifac, tss) = wk.load();
        assert!((ifac - 1.0).abs() < 1e-4);
        assert!((tss - 100.0).abs() < 0.1);
    }

    #[test]
    fn best_average_window() {
        let s = [100.0, 100.0, 300.0, 300.0, 100.0];
        assert_eq!(best_average(&s, 2), Some(300.0));
        assert_eq!(best_average(&s, 6), None);
    }

    #[test]
    fn zwo_import() {
        let xml = r#"<workout_file>
            <author>me</author><name>Test ZWO</name><description>desc</description>
            <sportType>bike</sportType>
            <workout>
              <Warmup Duration="600" PowerLow="0.25" PowerHigh="0.75"/>
              <SteadyState Duration="300" Power="0.9"><textevent timeoffset="0" message="go"/></SteadyState>
              <IntervalsT Repeat="4" OnDuration="60" OffDuration="120" OnPower="1.2" OffPower="0.5"/>
              <FreeRide Duration="120" FlatRoad="1"/>
              <Cooldown Duration="300" PowerLow="0.25" PowerHigh="0.75"/>
            </workout>
          </workout_file>"#;
        let wk = parse_zwo(xml, "x").unwrap();
        assert_eq!(wk.name, "Test ZWO");
        assert_eq!(wk.steps.len(), 5);
        assert_eq!(
            wk.steps[4],
            Step::Ramp {
                secs: 300,
                from: 0.75,
                to: 0.25
            }
        );
        assert_eq!(wk.duration(), (600 + 300 + 4 * 180 + 120 + 300) as f32);
    }

    #[test]
    fn builtin_ids_unique() {
        let lib = builtin();
        let mut ids: Vec<_> = lib.iter().map(|w| w.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), lib.len());
    }
}
