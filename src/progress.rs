//! Long-term numbers: fitness/fatigue/form, power curves and totals.

use crate::ride::{Sample, Summary};
use chrono::{Datelike, Duration, NaiveDate};
use serde::{Deserialize, Serialize};

/// Durations (seconds) of the power curve.
pub const CURVE_SECS: [u32; 11] = [5, 15, 30, 60, 120, 300, 600, 1200, 1800, 3600, 7200];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FtpEntry {
    pub date: NaiveDate,
    pub ftp: f32,
}

/// Best average power for each power-curve duration the ride is long enough for.
pub fn best_curve(samples: &[Sample]) -> Vec<(u32, f32)> {
    let p: Vec<f32> = samples.iter().map(|s| s.power).collect();
    CURVE_SECS
        .iter()
        .filter_map(|&secs| crate::workout::best_average(&p, secs as usize).map(|w| (secs, w)))
        .collect()
}

/// Highest value per duration across several rides' curves.
pub fn merge_curves<'a>(curves: impl Iterator<Item = &'a [(u32, f32)]>) -> Vec<(u32, f32)> {
    let mut best = [0.0f32; CURVE_SECS.len()];
    for c in curves {
        for &(secs, w) in c {
            if let Some(i) = CURVE_SECS.iter().position(|&s| s == secs) {
                best[i] = best[i].max(w);
            }
        }
    }
    CURVE_SECS
        .iter()
        .zip(best)
        .filter(|(_, w)| *w > 0.0)
        .map(|(&s, w)| (s, w))
        .collect()
}

pub struct Fitness {
    /// One value per day, oldest first, ending today.
    pub ctl: Vec<f32>,
    pub atl: Vec<f32>,
    pub tsb: Vec<f32>,
}

impl Fitness {
    /// Today's (fitness, fatigue, form).
    pub fn today(&self) -> (f32, f32, f32) {
        let i = self.ctl.len().saturating_sub(1);
        (
            self.ctl.get(i).copied().unwrap_or(0.0),
            self.atl.get(i).copied().unwrap_or(0.0),
            self.tsb.get(i).copied().unwrap_or(0.0),
        )
    }
}

/// Training load model: fitness is a 42-day and fatigue a 7-day exponentially
/// weighted average of daily TSS; form is yesterday's fitness minus fatigue.
pub fn fitness(history: &[Summary], today: NaiveDate, days: usize) -> Fitness {
    // Warm up the averages on older rides so the window starts realistic.
    let warmup = 120;
    let total = days + warmup;
    let mut daily = vec![0.0f32; total + 1];
    for h in history {
        let age = (today - h.start.date_naive()).num_days();
        if (0..=total as i64).contains(&age) {
            daily[total - age as usize] += h.tss;
        }
    }
    let (mut ctl, mut atl) = (0.0f32, 0.0f32);
    let mut f = Fitness {
        ctl: vec![],
        atl: vec![],
        tsb: vec![],
    };
    for (i, t) in daily.into_iter().enumerate() {
        let form = ctl - atl;
        ctl += (t - ctl) / 42.0;
        atl += (t - atl) / 7.0;
        if i >= warmup {
            f.tsb.push(form);
            f.ctl.push(ctl);
            f.atl.push(atl);
        }
    }
    f
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Totals {
    pub rides: u32,
    pub secs: f32,
    pub km: f64,
    pub tss: f32,
    pub kj: f32,
}

impl Totals {
    fn add(&mut self, h: &Summary) {
        self.rides += 1;
        self.secs += h.secs;
        self.km += h.distance_m / 1000.0;
        self.tss += h.tss;
        self.kj += h.kj;
    }
}

pub fn totals_between(history: &[Summary], from: NaiveDate, to: NaiveDate) -> Totals {
    let mut t = Totals::default();
    for h in history
        .iter()
        .filter(|h| (from..=to).contains(&h.start.date_naive()))
    {
        t.add(h);
    }
    t
}

/// Monday-based weeks, oldest first, ending with the current week.
pub fn weekly(history: &[Summary], today: NaiveDate, weeks: usize) -> Vec<(NaiveDate, Totals)> {
    let this_monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    (0..weeks)
        .rev()
        .map(|k| {
            let start = this_monday - Duration::weeks(k as i64);
            (
                start,
                totals_between(history, start, start + Duration::days(6)),
            )
        })
        .collect()
}

/// Calendar months, newest first.
pub fn monthly(history: &[Summary], today: NaiveDate, months: usize) -> Vec<(NaiveDate, Totals)> {
    (0..months)
        .map(|k| {
            let first = crate::plan::add_months(today, -(k as i32));
            let next = crate::plan::add_months(first, 1);
            (
                first,
                totals_between(history, first, next - Duration::days(1)),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};

    fn ride(date: NaiveDate, secs: f32, tss: f32, best: Vec<(u32, f32)>) -> Summary {
        Summary {
            start: Local
                .from_local_datetime(&date.and_hms_opt(18, 0, 0).unwrap())
                .unwrap(),
            name: "Ride".into(),
            workout_id: None,
            secs,
            distance_m: secs as f64 * 9.0,
            ascent_m: 0.0,
            avg_power: 180.0,
            np: 190.0,
            tss,
            kj: secs * 0.18,
            avg_hr: None,
            avg_cadence: None,
            file: String::new(),
            ftp_estimate: None,
            hr_zones: vec![],
            power_zones: vec![],
            max_hr: None,
            icu_id: None,
            route_id: None,
            best,
        }
    }

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn curves_merge_by_duration() {
        let a = vec![(5, 600.0), (60, 350.0), (1200, 240.0)];
        let b = vec![(5, 650.0), (60, 330.0)];
        assert_eq!(
            merge_curves([a.as_slice(), b.as_slice()].into_iter()),
            vec![(5, 650.0), (60, 350.0), (1200, 240.0)]
        );
    }

    #[test]
    fn fitness_rises_with_training_and_form_drops() {
        let today = d("2026-11-30");
        let history: Vec<Summary> = (0..28)
            .map(|i| ride(today - Duration::days(i), 3600.0, 80.0, vec![]))
            .collect();
        let f = fitness(&history, today, 60);
        assert_eq!(f.ctl.len(), 61);
        let (ctl, atl, tsb) = f.today();
        assert!(ctl > 30.0 && ctl < 80.0, "{ctl}");
        assert!(atl > ctl, "fatigue reacts faster");
        assert!(tsb < 0.0);
        // Rest days bring form back up.
        let rested = fitness(&history[7..], today, 60);
        assert!(rested.today().2 > tsb);
    }

    #[test]
    fn weekly_and_monthly_totals() {
        let history = vec![
            ride(d("2026-11-02"), 3600.0, 60.0, vec![]),
            ride(d("2026-11-04"), 1800.0, 30.0, vec![]),
            ride(d("2026-10-28"), 3600.0, 50.0, vec![]),
        ];
        let w = weekly(&history, d("2026-11-05"), 3);
        assert_eq!(w.len(), 3);
        assert_eq!(w[2].0, d("2026-11-02"));
        assert_eq!(w[2].1.rides, 2);
        assert_eq!(w[2].1.secs, 5400.0);
        assert_eq!(w[1].1.rides, 1);
        let m = monthly(&history, d("2026-11-05"), 2);
        assert_eq!(m[0].0, d("2026-11-01"));
        assert_eq!(m[0].1.rides, 2);
        assert_eq!(m[1].1.tss, 50.0);
    }
}
