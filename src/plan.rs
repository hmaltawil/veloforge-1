//! Training calendar and plan generator.

use crate::workout::Category;
use chrono::{Datelike, Duration, NaiveDate};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannedWorkout {
    pub date: NaiveDate,
    pub workout_id: String,
    #[serde(default)]
    pub done: bool,
    /// Missed and deliberately skipped.
    #[serde(default)]
    pub skipped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Goal {
    FtpBuilder,
    Base,
    Maintenance,
}

impl Goal {
    pub const ALL: [Goal; 3] = [Goal::FtpBuilder, Goal::Base, Goal::Maintenance];

    pub fn label(self) -> &'static str {
        match self {
            Goal::FtpBuilder => "FTP builder",
            Goal::Base => "Aerobic base",
            Goal::Maintenance => "Maintain fitness",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Goal::FtpBuilder => "Sweet spot, threshold and VO2 work with a long endurance ride. Starts and ends with a ramp test.",
            Goal::Base => "Mostly zone 2 endurance with some tempo and sweet spot. Low stress, big aerobic gains.",
            Goal::Maintenance => "A balanced mix that keeps your fitness with modest time and fatigue.",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanConfig {
    pub goal: Goal,
    pub start: NaiveDate,
    pub weeks: u32,
    /// Monday first.
    pub days: [bool; 7],
    /// Index (Monday = 0) of the preferred long-ride day.
    pub long_day: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Long,
    Endurance,
    Recovery,
    Tempo,
    SweetSpot,
    Threshold,
    Vo2,
}

/// Progressions per slot: later build weeks pick later entries.
fn family(slot: Slot) -> &'static [&'static str] {
    match slot {
        Slot::Long => &[
            "endurance-75",
            "endurance-90",
            "endurance-90",
            "endurance-120",
        ],
        Slot::Endurance => &[
            "endurance-45",
            "endurance-60",
            "endurance-60",
            "endurance-75",
        ],
        Slot::Recovery => &["recovery-30", "recovery-45"],
        Slot::Tempo => &["tempo-2x15", "tempo-3x15"],
        Slot::SweetSpot => &["ss-3x10", "ss-3x12", "ss-3x15", "ss-2x20"],
        Slot::Threshold => &["thr-4x8", "thr-3x10", "ou-3x9", "thr-3x12", "thr-2x20"],
        Slot::Vo2 => &["vo2-5x3", "vo2-30-30", "vo2-6x3", "vo2-5x4"],
    }
}

/// Weekly slot pattern for a given number of riding days (excluding Long,
/// which is always included and placed on the long day).
fn pattern(goal: Goal, days: usize) -> Vec<Slot> {
    use Slot::*;
    let full: &[Slot] = match goal {
        Goal::FtpBuilder => &[SweetSpot, Vo2, Endurance, Threshold, Recovery, Endurance],
        Goal::Base => &[Endurance, Tempo, Endurance, SweetSpot, Recovery, Endurance],
        Goal::Maintenance => &[Threshold, Endurance, SweetSpot, Recovery, Vo2, Endurance],
    };
    let mut v = vec![Long];
    v.extend(full.iter().take(days.saturating_sub(1)));
    v
}

pub fn generate(cfg: &PlanConfig) -> Vec<PlannedWorkout> {
    let mut chosen: Vec<usize> = (0..7).filter(|&d| cfg.days[d]).collect();
    if chosen.is_empty() {
        return Vec::new();
    }
    let long_day = if chosen.contains(&cfg.long_day) {
        cfg.long_day
    } else {
        *chosen.last().unwrap()
    };
    let slots = pattern(cfg.goal, chosen.len());
    // Long ride goes on the long day; the rest fill the other days in order,
    // spreading hard sessions away from the long ride.
    chosen.retain(|&d| d != long_day);
    let mut assignment: Vec<(usize, Slot)> = vec![(long_day, Slot::Long)];
    assignment.extend(chosen.iter().copied().zip(slots[1..].iter().copied()));
    assignment.sort_by_key(|a| a.0);

    let monday = cfg.start - Duration::days(cfg.start.weekday().num_days_from_monday() as i64);
    let mut out = Vec::new();
    let mut level = 0usize;
    for week in 0..cfg.weeks {
        let recovery_week = (week + 1) % 4 == 0 && week + 1 != cfg.weeks;
        for (i, &(day, slot)) in assignment.iter().enumerate() {
            let date = monday + Duration::days(week as i64 * 7 + day as i64);
            if date < cfg.start {
                continue;
            }
            let first_hard = i
                == assignment
                    .iter()
                    .position(|a| a.1 != Slot::Long)
                    .unwrap_or(0);
            // Recovery weeks: one easy tempo session, the rest short and easy.
            let (slot, lvl) = if recovery_week {
                match slot {
                    Slot::Long => (Slot::Endurance, 1),
                    Slot::Vo2 | Slot::Threshold | Slot::SweetSpot if first_hard => (Slot::Tempo, 0),
                    Slot::Vo2 | Slot::Threshold | Slot::SweetSpot | Slot::Tempo => {
                        (Slot::Endurance, 0)
                    }
                    Slot::Endurance | Slot::Recovery => (Slot::Recovery, 0),
                }
            } else {
                (slot, level)
            };
            let fam = family(slot);
            // Progress through the family, then alternate between the two
            // hardest sessions rather than repeating one week after week.
            let pick = if lvl < fam.len() || fam.len() < 2 {
                lvl.min(fam.len() - 1)
            } else {
                fam.len() - 2 + (lvl - fam.len()) % 2
            };
            let mut id = fam[pick].to_string();
            if cfg.goal == Goal::FtpBuilder
                && first_hard
                && (week == 0 || (week + 1 == cfg.weeks && cfg.weeks >= 4))
            {
                id = "ramp-test".into();
            }
            out.push(PlannedWorkout {
                date,
                workout_id: id,
                done: false,
                skipped: false,
            });
        }
        if !recovery_week {
            level += 1;
        }
    }
    out
}

/// Hard sessions: missing one matters, and they should not be stacked.
pub fn is_key(c: Category) -> bool {
    matches!(
        c,
        Category::SweetSpot
            | Category::Threshold
            | Category::Vo2Max
            | Category::Anaerobic
            | Category::Test
    )
}

/// What to do about a missed planned workout.
#[derive(Debug, Clone, PartialEq)]
pub struct Missed {
    pub index: usize,
    /// New date for a key session, if a good free day was found.
    pub move_to: Option<NaiveDate>,
}

/// Planned workouts from the last week that were neither ridden nor
/// skipped. Key sessions get a free day within the next four days,
/// preferring days that do not sit next to another key session; easier
/// rides are simply dropped. Workouts planned on intervals.icu are left to
/// intervals.icu.
pub fn missed(
    plan: &[PlannedWorkout],
    today: NaiveDate,
    category: impl Fn(&str) -> Option<Category>,
) -> Vec<Missed> {
    let key_on = |plan: &[PlannedWorkout], d: NaiveDate| {
        plan.iter()
            .any(|p| p.date == d && !p.skipped && category(&p.workout_id).is_some_and(is_key))
    };
    let mut taken: Vec<NaiveDate> = plan
        .iter()
        .filter(|p| p.date >= today && !p.skipped)
        .map(|p| p.date)
        .collect();
    let mut out = Vec::new();
    for (i, p) in plan.iter().enumerate() {
        if p.done
            || p.skipped
            || p.date >= today
            || p.date < today - Duration::days(7)
            || p.workout_id.starts_with("icu-")
        {
            continue;
        }
        let key = category(&p.workout_id).is_some_and(is_key);
        let mut move_to = None;
        if key {
            let free: Vec<NaiveDate> = (0..4)
                .map(|k| today + Duration::days(k))
                .filter(|d| !taken.contains(d))
                .collect();
            let relaxed = free.iter().copied().find(|d| {
                !key_on(plan, *d - Duration::days(1)) && !key_on(plan, *d + Duration::days(1))
            });
            move_to = relaxed.or(free.first().copied());
            if let Some(d) = move_to {
                taken.push(d);
            }
        }
        out.push(Missed { index: i, move_to });
    }
    out
}

/// An easier workout for a tired rider, if today's plan is a hard one.
/// `form` is today's training stress balance (fitness minus fatigue).
pub fn easier_option(category: Category, form: f32) -> Option<&'static str> {
    if !is_key(category) || form > -25.0 {
        return None;
    }
    Some(if form < -35.0 {
        "recovery-30"
    } else {
        "endurance-45"
    })
}

pub fn weekday_label(i: usize) -> &'static str {
    ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][i % 7]
}

pub fn first_of_month(d: NaiveDate) -> NaiveDate {
    d.with_day(1).unwrap()
}

pub fn add_months(d: NaiveDate, months: i32) -> NaiveDate {
    let total = d.year() * 12 + d.month0() as i32 + months;
    NaiveDate::from_ymd_opt(total.div_euclid(12), total.rem_euclid(12) as u32 + 1, 1).unwrap()
}

/// The Monday on or before the first day of the month, for a 6-row grid.
pub fn month_grid_start(d: NaiveDate) -> NaiveDate {
    let first = first_of_month(d);
    first - Duration::days(first.weekday().num_days_from_monday() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workout::builtin;
    use chrono::Weekday;

    fn cfg(goal: Goal, days: [bool; 7]) -> PlanConfig {
        PlanConfig {
            goal,
            start: NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
            weeks: 8,
            days,
            long_day: 5,
        }
    }

    #[test]
    fn plan_uses_selected_days_and_known_workouts() {
        let days = [false, true, false, true, false, true, false];
        let plan = generate(&cfg(Goal::FtpBuilder, days));
        assert_eq!(plan.len(), 8 * 3);
        let lib = builtin();
        for p in &plan {
            assert!(days[p.date.weekday().num_days_from_monday() as usize]);
            assert!(lib.iter().any(|w| w.id == p.workout_id), "{}", p.workout_id);
        }
        // Saturday is the long day.
        assert!(plan
            .iter()
            .filter(|p| p.date.weekday() == Weekday::Sat)
            .all(|p| p.workout_id.starts_with("endurance")));
        assert_eq!(plan[0].workout_id, "ramp-test");
        assert!(plan
            .iter()
            .rev()
            .take(3)
            .any(|p| p.workout_id == "ramp-test"));
    }

    #[test]
    fn recovery_week_is_easier() {
        let plan = generate(&cfg(
            Goal::FtpBuilder,
            [true, false, true, false, true, true, false],
        ));
        let lib = builtin();
        let tss = |week: i64| -> f32 {
            let start = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap() + Duration::days(week * 7);
            plan.iter()
                .filter(|p| p.date >= start && p.date < start + Duration::days(7))
                .map(|p| lib.iter().find(|w| w.id == p.workout_id).unwrap().load().1)
                .sum()
        };
        assert!(tss(3) < tss(2) * 0.8);
    }

    #[test]
    fn missed_key_sessions_move_and_easy_ones_drop() {
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let pw = |date: &str, id: &str| PlannedWorkout {
            date: d(date),
            workout_id: id.into(),
            done: false,
            skipped: false,
        };
        let lib = builtin();
        let cat = |id: &str| lib.iter().find(|w| w.id == id).map(|w| w.category);
        let today = d("2026-11-05"); // Thursday
        let plan = vec![
            pw("2026-11-03", "thr-3x10"),     // missed key (Tue)
            pw("2026-11-04", "endurance-60"), // missed easy (Wed)
            pw("2026-11-05", "vo2-5x3"),      // today, key
            pw("2026-11-07", "endurance-90"), // Sat
            pw("2026-10-20", "ss-3x10"),      // too old to matter
            PlannedWorkout {
                done: true,
                ..pw("2026-11-02", "ss-3x12")
            },
            pw("2026-11-01", "icu-55"), // intervals.icu's business
        ];
        let m = missed(&plan, today, cat);
        assert_eq!(m.len(), 2);
        // Not tomorrow (next to today's VO2 session): Saturday is taken, so Sunday.
        assert_eq!(
            m[0],
            Missed {
                index: 0,
                move_to: Some(d("2026-11-08"))
            }
        );
        assert_eq!(
            m[1],
            Missed {
                index: 1,
                move_to: None
            }
        );
    }

    #[test]
    fn tired_riders_get_an_easier_option() {
        assert_eq!(easier_option(Category::Vo2Max, -10.0), None);
        assert_eq!(easier_option(Category::Vo2Max, -28.0), Some("endurance-45"));
        assert_eq!(
            easier_option(Category::Threshold, -40.0),
            Some("recovery-30")
        );
        assert_eq!(easier_option(Category::Endurance, -40.0), None);
    }

    #[test]
    fn late_build_weeks_vary() {
        let days = [false, true, false, true, false, true, false];
        let plan = generate(&PlanConfig {
            weeks: 12,
            ..cfg(Goal::FtpBuilder, days)
        });
        let start = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        let week = |w: i64| -> Vec<String> {
            let a = start + Duration::days(w * 7);
            plan.iter()
                .filter(|p| p.date >= a && p.date < a + Duration::days(7))
                .map(|p| p.workout_id.clone())
                .collect()
        };
        // Weeks 9 and 10 are consecutive build weeks at the top level.
        assert_ne!(week(8), week(9));
    }

    #[test]
    fn months() {
        let d = NaiveDate::from_ymd_opt(2026, 12, 15).unwrap();
        assert_eq!(
            add_months(d, 1),
            NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()
        );
        assert_eq!(
            add_months(d, -12),
            NaiveDate::from_ymd_opt(2025, 12, 1).unwrap()
        );
        assert_eq!(month_grid_start(d).weekday(), Weekday::Mon);
    }
}
