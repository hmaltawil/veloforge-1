//! Virtual shifting, for bikes with a single fixed cog on the trainer (a
//! Zwift Cog and the like): the gears live in the app. Your cadence, the
//! gear you are in and the wheel give the speed a real bike would go; the
//! road's physics give the power that speed needs on this gradient; the
//! trainer is asked for that power. So the same pedalling is harder in a
//! bigger gear and on a steeper road, as outdoors.

use crate::ride::Physics;

/// One gear: chainring and sprocket teeth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gear {
    pub ring: u8,
    pub cog: u8,
}

impl Gear {
    /// Wheel turns per turn of the cranks.
    pub fn ratio(self) -> f64 {
        self.ring as f64 / self.cog as f64
    }
}

const fn g(ring: u8, cog: u8) -> Gear {
    Gear { ring, cog }
}

/// A 34/50 chainset with an 11-34 cassette (11-13-15-17-19-21-23-25-27-
/// 30-34), shifted as a rider shifts it: the small ring from the biggest
/// sprocket to the 17 for climbing, then the big ring from the 23 down to
/// the 11. That leaves out the crossed gears riders avoid (and the
/// duplicates: 34x17 and 50x25 are the same gear), and every step is 8 to
/// 18%, like the cassette's own.
pub const GEARS: [Gear; 15] = [
    g(34, 34),
    g(34, 30),
    g(34, 27),
    g(34, 25),
    g(34, 23),
    g(34, 21),
    g(34, 19),
    g(34, 17),
    g(50, 23),
    g(50, 21),
    g(50, 19),
    g(50, 17),
    g(50, 15),
    g(50, 13),
    g(50, 11),
];

/// Where a new rider starts: the middle gear (34x17, about 23 km/h at 90 rpm).
pub const START_GEAR: usize = 7;

/// Rolling circumference of a 700x25c road wheel, in metres.
pub const WHEEL_M: f64 = 2.105;

/// Below this cadence the cranks are taken as still (coasting, or a sensor
/// winding down): the trainer eases off to its lightest.
pub const MIN_CADENCE: f32 = 20.0;

/// The lightest the trainer is asked for. Some trainers turn down a target
/// of 0 W, and their resistance cannot go lower than this anyway.
pub const MIN_WATTS: f32 = 10.0;

/// The most the trainer is asked for (a sprint in top gear).
pub const MAX_WATTS: f32 = 2000.0;

/// The gear index brought into range (a stored one from an older gear set).
pub fn clamp(gear: usize) -> usize {
    gear.min(GEARS.len() - 1)
}

/// One gear up (`up`) or down, stopping at the ends.
pub fn shift(gear: usize, up: bool) -> usize {
    let gear = clamp(gear);
    if up {
        clamp(gear + 1)
    } else {
        gear.saturating_sub(1)
    }
}

/// "50x17 · gear 12/15".
pub fn label(gear: usize) -> String {
    let i = clamp(gear);
    format!("{} · gear {}/{}", teeth(i), i + 1, GEARS.len())
}

/// "50x17".
pub fn teeth(gear: usize) -> String {
    let g = GEARS[clamp(gear)];
    format!("{}x{}", g.ring, g.cog)
}

/// The speed in m/s that `rpm` turns the wheel at in `gear`.
pub fn speed(rpm: f32, gear: usize) -> f64 {
    rpm.max(0.0) as f64 / 60.0 * GEARS[clamp(gear)].ratio() * WHEEL_M
}

/// The power in watts that holds a steady `v` m/s on a `grade` percent
/// road: the same forces as the ride's virtual speed (gravity, rolling
/// resistance, air, and the drivetrain's 97%), so riding at this power
/// settles at this speed. Negative where the road alone would go faster.
pub fn power_for_speed(v: f64, grade: f64, phys: &Physics) -> f64 {
    let theta = (grade / 100.0).atan();
    let m = phys.mass as f64;
    let force = m * 9.81 * (theta.sin() + phys.crr as f64 * theta.cos())
        + 0.5 * 1.204 * phys.cda as f64 * v * v;
    force * v / 0.97
}

/// What the trainer should hold for `rpm` in `gear` on a `grade` percent
/// road, before easing: between [`MIN_WATTS`] and [`MAX_WATTS`], and the
/// lightest when the cranks are (nearly) still.
pub fn target_power(rpm: f32, gear: usize, grade: f64, phys: &Physics) -> f32 {
    if rpm < MIN_CADENCE {
        return MIN_WATTS;
    }
    let w = power_for_speed(speed(rpm, gear), grade, phys) as f32;
    w.clamp(MIN_WATTS, MAX_WATTS)
}

/// How quickly the trainer's power follows a new target: most of the way in
/// about a second, so a shift or a change of gradient comes on smoothly.
const EASE_SECS: f32 = 0.8;
/// And never faster than this, in watts per second.
const MAX_RATE: f32 = 400.0;

/// Move `current` towards `target` over `dt` seconds, smoothly.
pub fn ease(current: f32, target: f32, dt: f64) -> f32 {
    let dt = dt as f32;
    let k = 1.0 - (-dt / EASE_SECS).exp();
    let step = ((target - current) * k).clamp(-MAX_RATE * dt, MAX_RATE * dt);
    current + step
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phys() -> Physics {
        Physics {
            mass: 84.0,
            cda: 0.32,
            crr: 0.004,
            trainer_speed: true,
            drafting: false,
            flat_workouts: false,
        }
    }

    #[test]
    fn gears_climb_in_even_steps() {
        assert_eq!(GEARS.len(), 15);
        for w in GEARS.windows(2) {
            let step = w[1].ratio() / w[0].ratio();
            assert!(
                (1.07..1.19).contains(&step),
                "{:?} -> {:?}: {step:.3}",
                w[0],
                w[1]
            );
        }
        assert_eq!(GEARS[0].ratio(), 1.0);
        assert!((GEARS[14].ratio() - 50.0 / 11.0).abs() < 1e-9);
        assert_eq!(teeth(START_GEAR), "34x17");
        assert_eq!(label(11), "50x17 · gear 12/15");
    }

    #[test]
    fn shifting_stops_at_the_ends() {
        assert_eq!(shift(0, false), 0);
        assert_eq!(shift(0, true), 1);
        assert_eq!(shift(14, true), 14);
        assert_eq!(shift(14, false), 13);
        // A stored gear from a bigger gear set comes back into range.
        assert_eq!(clamp(40), 14);
        assert_eq!(shift(40, false), 13);
    }

    #[test]
    fn cadence_and_gear_give_the_speed_of_a_real_bike() {
        // 90 rpm in 50x11 on a 700x25c wheel: 90/60 x 4.545 x 2.105 m.
        let v = speed(90.0, 14);
        assert!((v - 14.352).abs() < 0.01, "{v}");
        assert!((v * 3.6 - 51.7).abs() < 0.1);
        // The middle gear at 90 rpm: about 23 km/h.
        assert!((speed(90.0, START_GEAR) * 3.6 - 22.7).abs() < 0.1);
        assert_eq!(speed(0.0, 7), 0.0);
        assert_eq!(speed(-5.0, 7), 0.0);
    }

    #[test]
    fn the_power_needed_matches_the_ride_physics() {
        let p = phys();
        // Riding the power that a speed needs settles at that speed.
        for (v, grade) in [(8.0, 0.0), (5.0, 6.0), (3.0, 12.0), (12.0, -2.0)] {
            let w = power_for_speed(v, grade, &p) as f32;
            let mut s = 1.0;
            for _ in 0..20000 {
                s = crate::ride::virtual_speed(s, w, grade, &p, 0.05);
            }
            assert!((s - v).abs() < 0.02, "{v} m/s at {grade}%: {w} W gives {s}");
        }
        // 30 km/h on the flat: 111 W of air, 27 W of rolling and the
        // drivetrain's share, for this rider on the hoods.
        let flat = power_for_speed(30.0 / 3.6, 0.0, &p);
        assert!((140.0..147.0).contains(&flat), "{flat}");
        // Steep enough downhill, the road alone goes faster.
        assert!(power_for_speed(8.0, -6.0, &p) < 0.0);
    }

    #[test]
    fn a_bigger_gear_or_a_steeper_road_is_harder() {
        let p = phys();
        let w = |gear, grade| target_power(90.0, gear, grade, &p);
        for gear in 0..GEARS.len() - 1 {
            assert!(w(gear + 1, 2.0) > w(gear, 2.0), "gear {gear}");
        }
        assert!(w(7, 8.0) > w(7, 4.0));
        assert!(w(7, 4.0) > w(7, 0.0));
        // Faster pedalling in the same gear is harder too.
        assert!(target_power(100.0, 7, 0.0, &p) > target_power(80.0, 7, 0.0, &p));
        // 90 rpm in the bottom gear up 8% is 11.4 km/h: about 240 W, as on
        // a real bike; slower pedalling makes it lighter, as there.
        let climb = w(0, 8.0);
        assert!((210.0..270.0).contains(&climb), "{climb}");
        assert!(target_power(60.0, 0, 8.0, &p) < climb * 0.7);
    }

    #[test]
    fn coasting_and_descents_ask_for_the_least() {
        let p = phys();
        assert_eq!(target_power(0.0, 10, 5.0, &p), MIN_WATTS);
        assert_eq!(target_power(15.0, 10, 5.0, &p), MIN_WATTS);
        assert_eq!(target_power(90.0, 3, -8.0, &p), MIN_WATTS);
        assert_eq!(target_power(200.0, 14, 10.0, &p), MAX_WATTS);
    }

    #[test]
    fn power_follows_a_shift_smoothly() {
        let mut w = 150.0;
        let mut prev = w;
        let mut t = 0.0;
        while t < 4.0 {
            w = ease(w, 300.0, 0.1);
            // Never more than 40 W in a tenth of a second.
            assert!(w - prev <= 40.0 + 1e-3, "{prev} -> {w}");
            assert!(w >= prev && w <= 300.0);
            prev = w;
            t += 0.1;
        }
        assert!((w - 300.0).abs() < 2.0, "{w}");
        // After one second, most of the way there.
        let mut w1 = 150.0;
        for _ in 0..10 {
            w1 = ease(w1, 300.0, 0.1);
        }
        assert!(w1 > 250.0, "{w1}");
        // The same in smaller steps.
        let mut w2 = 150.0;
        for _ in 0..100 {
            w2 = ease(w2, 300.0, 0.01);
        }
        assert!((w1 - w2).abs() < 5.0, "{w1} vs {w2}");
    }
}
