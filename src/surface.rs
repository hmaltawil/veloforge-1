//! The road's surface: tarmac, gravel or cobbles, all along the route or
//! in stretches (Mixed). It sets how the road looks and how hard it rolls.
//! Pure and tested; `gfx` draws it and `App` puts it in the physics.

use serde::{Deserialize, Serialize};

/// What the rider chose for the road.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Surface {
    #[default]
    Tarmac,
    Gravel,
    Cobbles,
    /// Stretches of each, laid out by the route so every ride of it is
    /// the same.
    Mixed,
}

/// What one stretch of road is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Tarmac,
    Gravel,
    Cobbles,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Tarmac => "Tarmac",
            Kind::Gravel => "Gravel",
            Kind::Cobbles => "Cobbles",
        }
    }

    /// The shader's number for it.
    pub fn code(self) -> f32 {
        match self {
            Kind::Tarmac => 0.0,
            Kind::Gravel => 1.0,
            Kind::Cobbles => 2.0,
        }
    }

    /// Rolling resistance on it, from yours on tarmac (`base`): loose
    /// gravel adds about half again, cobbles about double.
    pub fn crr(self, base: f32) -> f32 {
        match self {
            Kind::Tarmac => base,
            Kind::Gravel => base + 0.002,
            Kind::Cobbles => base + 0.004,
        }
    }
}

/// A stretch of one surface, from `start` to `end` metres along the course.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stretch {
    pub start: f64,
    pub end: f64,
    pub kind: Kind,
}

/// The most stretches a course is cut into (what the road shader takes).
pub const MAX_STRETCHES: usize = 24;

fn hash(seed: u64, i: u64) -> f64 {
    let mut h = seed ^ i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^= h >> 33;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// The course's stretches, start to end (`length` metres; `seed` from the
/// route, so Mixed is the same every ride). Mixed starts and ends on
/// tarmac, and never puts the same surface twice in a row: stretches of
/// 300 m to 1.5 km, or longer on a long course so there are at most
/// `MAX_STRETCHES`.
pub fn stretches(surface: Surface, length: f64, seed: u64) -> Vec<Stretch> {
    let one = |kind| {
        vec![Stretch {
            start: 0.0,
            end: length,
            kind,
        }]
    };
    match surface {
        Surface::Tarmac => one(Kind::Tarmac),
        Surface::Gravel => one(Kind::Gravel),
        Surface::Cobbles => one(Kind::Cobbles),
        Surface::Mixed => {
            let scale = (length / (MAX_STRETCHES as f64 * 300.0)).max(1.0);
            let mut out = Vec::new();
            let mut at = 0.0;
            let mut kind = Kind::Tarmac;
            let mut i = 0;
            while at < length {
                let long = if kind == Kind::Tarmac { 1.4 } else { 1.0 };
                let len = (300.0 + 1200.0 * hash(seed, i)) * scale * long;
                let mut end = (at + len).min(length);
                // No crumb of a stretch left at the end.
                if length - end < 200.0 * scale || out.len() + 2 >= MAX_STRETCHES {
                    end = length;
                }
                out.push(Stretch {
                    start: at,
                    end,
                    kind,
                });
                at = end;
                i += 1;
                // The next: never the same; tarmac half the time.
                let r = hash(seed, 1000 + i);
                kind = match kind {
                    Kind::Tarmac if r < 0.55 => Kind::Gravel,
                    Kind::Tarmac => Kind::Cobbles,
                    _ if r < 0.6 => Kind::Tarmac,
                    Kind::Gravel => Kind::Cobbles,
                    Kind::Cobbles => Kind::Gravel,
                };
            }
            // Finish on tarmac: the last stretch, if not, becomes tarmac.
            let several = out.len() > 1;
            if let Some(last) = out.last_mut() {
                if last.kind != Kind::Tarmac && several {
                    last.kind = Kind::Tarmac;
                }
            }
            // Two tarmacs in a row join up.
            let mut joined: Vec<Stretch> = Vec::with_capacity(out.len());
            for s in out {
                match joined.last_mut() {
                    Some(p) if p.kind == s.kind => p.end = s.end,
                    _ => joined.push(s),
                }
            }
            joined
        }
    }
}

/// Where `d` metres along falls on the course: round again on a loop,
/// and past the finish of a point-to-point course the road stays as it
/// ended.
fn on_course(d: f64, length: f64, looped: bool) -> f64 {
    if length <= 0.0 {
        0.0
    } else if looped {
        d.rem_euclid(length)
    } else {
        d.clamp(0.0, length)
    }
}

/// The surface `d` metres along, and the next different one ahead with
/// how far away it is (none on one surface all along).
pub fn at(stretches: &[Stretch], d: f64, length: f64, looped: bool) -> (Kind, Option<(Kind, f64)>) {
    let x = on_course(d, length, looped);
    let Some(i) = stretches
        .iter()
        .position(|s| x >= s.start && x < s.end)
        .or_else(|| stretches.len().checked_sub(1))
    else {
        return (Kind::Tarmac, None);
    };
    let here = stretches[i].kind;
    let next = if let Some(n) = stretches.get(i + 1) {
        Some((n.kind, stretches[i].end - x))
    } else if looped && stretches.len() > 1 && stretches[0].kind != here {
        Some((stretches[0].kind, length - x))
    } else {
        None
    };
    (here, next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_surface_all_along() {
        for (s, k) in [
            (Surface::Tarmac, Kind::Tarmac),
            (Surface::Gravel, Kind::Gravel),
            (Surface::Cobbles, Kind::Cobbles),
        ] {
            let st = stretches(s, 12_000.0, 7);
            assert_eq!(st.len(), 1);
            assert_eq!(at(&st, 5000.0, 12_000.0, true), (k, None));
        }
    }

    #[test]
    fn mixed_stretches_are_whole_and_the_same_every_ride() {
        for (len, seed) in [(12_000.0, 1u64), (3_000.0, 2), (120_000.0, 3), (800.0, 4)] {
            let st = stretches(Surface::Mixed, len, seed);
            assert_eq!(st, stretches(Surface::Mixed, len, seed));
            assert!(st.len() <= MAX_STRETCHES, "{len}: {}", st.len());
            assert_eq!(st[0].start, 0.0);
            assert_eq!(st.last().unwrap().end, len);
            assert_eq!(st[0].kind, Kind::Tarmac);
            for w in st.windows(2) {
                assert_eq!(w[0].end, w[1].start);
                assert_ne!(w[0].kind, w[1].kind);
            }
            if len >= 3_000.0 {
                // A real mix: some of each off tarmac.
                assert!(st.iter().any(|s| s.kind != Kind::Tarmac), "{len}");
            }
        }
        // Another route, another layout.
        assert_ne!(
            stretches(Surface::Mixed, 12_000.0, 1),
            stretches(Surface::Mixed, 12_000.0, 2)
        );
    }

    #[test]
    fn where_you_are_and_what_comes_next() {
        let st = vec![
            Stretch {
                start: 0.0,
                end: 1000.0,
                kind: Kind::Tarmac,
            },
            Stretch {
                start: 1000.0,
                end: 1500.0,
                kind: Kind::Cobbles,
            },
            Stretch {
                start: 1500.0,
                end: 3000.0,
                kind: Kind::Gravel,
            },
        ];
        assert_eq!(
            at(&st, 200.0, 3000.0, true),
            (Kind::Tarmac, Some((Kind::Cobbles, 800.0)))
        );
        assert_eq!(
            at(&st, 1200.0, 3000.0, true),
            (Kind::Cobbles, Some((Kind::Gravel, 300.0)))
        );
        // Round a loop: after the last stretch comes the first.
        assert_eq!(
            at(&st, 2500.0, 3000.0, true),
            (Kind::Gravel, Some((Kind::Tarmac, 500.0)))
        );
        assert_eq!(at(&st, 3200.0, 3000.0, true).0, Kind::Tarmac);
        // Past a point-to-point finish the road stays as it ended.
        assert_eq!(at(&st, 3500.0, 3000.0, false), (Kind::Gravel, None));
    }

    #[test]
    fn rougher_rolls_harder() {
        assert_eq!(Kind::Tarmac.crr(0.004), 0.004);
        assert!(Kind::Gravel.crr(0.004) > 0.0055);
        assert!(Kind::Cobbles.crr(0.004) > Kind::Gravel.crr(0.004));
    }
}
