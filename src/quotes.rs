//! A line about cycling on Home, one a day, picked to suit the day: a hard
//! workout, an easy one, a rest day, a test, or a run of rides.

use crate::workout::Category;
use chrono::{Datelike, NaiveDate};

/// The kind of day a quote suits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Day {
    /// Nothing planned: getting going.
    Starting,
    Hard,
    Easy,
    Rest,
    /// Riding several days in a row.
    Streak,
    Test,
    /// Any day at all.
    Any,
}

pub struct Quote {
    pub text: &'static str,
    /// Who said it; empty for the lines written for VeloForge.
    pub by: &'static str,
    pub day: Day,
}

const fn q(day: Day, text: &'static str, by: &'static str) -> Quote {
    Quote { text, by, day }
}

pub const QUOTES: [Quote; 30] = [
    q(
        Day::Any,
        "Life is like riding a bicycle. To keep your balance, you must keep moving.",
        "Albert Einstein",
    ),
    q(
        Day::Any,
        "Ride as much or as little, or as long or as short as you feel. But ride.",
        "Eddy Merckx",
    ),
    q(
        Day::Hard,
        "It never gets easier, you just go faster.",
        "Greg LeMond",
    ),
    q(Day::Hard, "Shut up, legs.", "Jens Voigt"),
    q(
        Day::Any,
        "Melancholy is incompatible with bicycling.",
        "James E. Starrs",
    ),
    q(
        Day::Any,
        "Get a bicycle. You will not regret it, if you live.",
        "Mark Twain",
    ),
    q(
        Day::Starting,
        "The hardest part of the ride is clipping in. You\u{2019}ve already done it.",
        "",
    ),
    q(Day::Starting, "Five minutes of warm-up. Then decide.", ""),
    q(
        Day::Starting,
        "Nobody regrets a ride once it\u{2019}s done.",
        "",
    ),
    q(Day::Starting, "Small gears, big plans.", ""),
    q(Day::Hard, "The interval ends. The strength stays.", ""),
    q(Day::Hard, "Breathe out, pedal round, hold the line.", ""),
    q(
        Day::Hard,
        "One more minute is always possible. Then one more.",
        "",
    ),
    q(
        Day::Hard,
        "Discomfort is just the sound of getting faster.",
        "",
    ),
    q(
        Day::Hard,
        "Today\u{2019}s burn is next month\u{2019}s easy pace.",
        "",
    ),
    q(Day::Easy, "Easy days make the hard days count.", ""),
    q(
        Day::Easy,
        "Ride slow enough to enjoy the view, even an indoor one.",
        "",
    ),
    q(Day::Easy, "Zone 2 is where engines are built.", ""),
    q(Day::Easy, "Smooth circles, quiet mind.", ""),
    q(
        Day::Rest,
        "Rest is training too. Your legs are rebuilding.",
        "",
    ),
    q(
        Day::Rest,
        "Fitness is made on the bike and banked off it.",
        "",
    ),
    q(Day::Rest, "Sleep well. Tomorrow has watts in it.", ""),
    q(Day::Streak, "Consistency beats heroics.", ""),
    q(
        Day::Streak,
        "Every kilometre counts, especially the ones you didn\u{2019}t feel like riding.",
        "",
    ),
    q(
        Day::Streak,
        "You don\u{2019}t have to be fast. You have to keep showing up.",
        "",
    ),
    q(Day::Streak, "A good week is just good days in a row.", ""),
    q(
        Day::Test,
        "Today you find out what all those rides were for.",
        "",
    ),
    q(Day::Test, "Pace the start, empty the finish.", ""),
    q(
        Day::Any,
        "The road doesn\u{2019}t care how you feel. Ride it anyway.",
        "",
    ),
    q(Day::Any, "Climbs end. Descents are earned.", ""),
];

/// What kind of day it is: from today's planned workout if there is one;
/// otherwise a streak after `days_in_a_row` days of riding (up to
/// yesterday), a rest day when other days are planned or today's ride is
/// done, else a day for getting going.
pub fn kind_of_day(
    planned: Option<Category>,
    rode_today: bool,
    days_in_a_row: u32,
    plan_nearby: bool,
) -> Day {
    if rode_today {
        return Day::Rest;
    }
    match planned {
        Some(Category::Recovery | Category::Endurance) => Day::Easy,
        Some(
            Category::Tempo
            | Category::SweetSpot
            | Category::Threshold
            | Category::Vo2Max
            | Category::Anaerobic,
        ) => Day::Hard,
        Some(Category::Test) => Day::Test,
        Some(Category::Custom | Category::Intervals) => Day::Starting,
        None if days_in_a_row >= 3 => Day::Streak,
        None if plan_nearby => Day::Rest,
        None => Day::Starting,
    }
}

/// Today's quote for a `day`: the same all day, another tomorrow; `more`
/// steps on through the others.
pub fn pick(day: Day, date: NaiveDate, more: usize) -> &'static Quote {
    let pool: Vec<&Quote> = QUOTES
        .iter()
        .filter(|q| q.day == day || q.day == Day::Any)
        .collect();
    // Days scattered over the list, so tomorrow is not simply the next.
    let n = date.num_days_from_ce().unsigned_abs() as usize;
    pool[(n.wrapping_mul(7) + more) % pool.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_day_suits_the_plan() {
        assert_eq!(
            kind_of_day(Some(Category::Vo2Max), false, 0, true),
            Day::Hard
        );
        assert_eq!(
            kind_of_day(Some(Category::Endurance), false, 0, true),
            Day::Easy
        );
        assert_eq!(kind_of_day(Some(Category::Test), false, 5, true), Day::Test);
        assert_eq!(
            kind_of_day(Some(Category::Vo2Max), true, 0, true),
            Day::Rest
        );
        assert_eq!(kind_of_day(None, false, 3, false), Day::Streak);
        assert_eq!(kind_of_day(None, false, 0, true), Day::Rest);
        assert_eq!(kind_of_day(None, false, 0, false), Day::Starting);
    }

    #[test]
    fn a_quote_a_day_that_suits_it() {
        let d = NaiveDate::from_ymd_opt(2026, 11, 26).unwrap();
        for day in [
            Day::Starting,
            Day::Hard,
            Day::Easy,
            Day::Rest,
            Day::Streak,
            Day::Test,
        ] {
            let q = pick(day, d, 0);
            assert!(q.day == day || q.day == Day::Any);
            // The same all day.
            assert_eq!(q.text, pick(day, d, 0).text);
            // Another one when asked.
            assert_ne!(q.text, pick(day, d, 1).text);
        }
        // Another tomorrow.
        assert_ne!(
            pick(Day::Hard, d, 0).text,
            pick(Day::Hard, d.succ_opt().unwrap(), 0).text
        );
    }
}
