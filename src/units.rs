//! Metric or imperial units for what VeloForge shows. Everything is kept
//! and calculated in metric (metres, km/h, kg); only the display changes.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Units {
    #[default]
    Metric,
    Imperial,
}

const KM_PER_MI: f64 = 1.609_344;
const M_PER_FT: f64 = 0.3048;
const KG_PER_LB: f64 = 0.453_592_37;

impl Units {
    pub fn imperial(self) -> bool {
        self == Units::Imperial
    }

    /// A speed given in km/h, in km/h or mph.
    pub fn speed(self, kmh: f64) -> f64 {
        if self.imperial() {
            kmh / KM_PER_MI
        } else {
            kmh
        }
    }
    pub fn speed_unit(self) -> &'static str {
        if self.imperial() {
            "mph"
        } else {
            "km/h"
        }
    }

    /// A distance given in metres, in km or miles.
    pub fn distance(self, metres: f64) -> f64 {
        let km = metres / 1000.0;
        if self.imperial() {
            km / KM_PER_MI
        } else {
            km
        }
    }
    pub fn distance_unit(self) -> &'static str {
        if self.imperial() {
            "mi"
        } else {
            "km"
        }
    }

    /// A height (climbing, elevation) given in metres, in m or ft.
    pub fn height(self, metres: f64) -> f64 {
        if self.imperial() {
            metres / M_PER_FT
        } else {
            metres
        }
    }
    pub fn height_unit(self) -> &'static str {
        if self.imperial() {
            "ft"
        } else {
            "m"
        }
    }

    /// A weight given in kg, in kg or lb.
    pub fn weight(self, kg: f64) -> f64 {
        if self.imperial() {
            kg / KG_PER_LB
        } else {
            kg
        }
    }
    /// A weight shown in these units, back in kg.
    pub fn weight_to_kg(self, w: f64) -> f64 {
        if self.imperial() {
            w * KG_PER_LB
        } else {
            w
        }
    }
    pub fn weight_unit(self) -> &'static str {
        if self.imperial() {
            "lb"
        } else {
            "kg"
        }
    }

    /// "12.3 km" / "7.6 mi", with `decimals`.
    pub fn fmt_distance(self, metres: f64, decimals: usize) -> String {
        format!(
            "{:.*} {}",
            decimals,
            self.distance(metres),
            self.distance_unit()
        )
    }

    /// "172 m" / "564 ft".
    pub fn fmt_height(self, metres: f64) -> String {
        format!("{:.0} {}", self.height(metres), self.height_unit())
    }

    /// "24.3 km/h" / "15.1 mph".
    pub fn fmt_speed(self, kmh: f64) -> String {
        format!("{:.1} {}", self.speed(kmh), self.speed_unit())
    }

    /// A short distance ahead: "320 m" or "1.2 km"; "350 ft" or "0.8 mi".
    pub fn fmt_short(self, metres: f64) -> String {
        if self.imperial() {
            let ft = metres / M_PER_FT;
            if ft < 1000.0 {
                format!("{:.0} ft", (ft / 10.0).round() * 10.0)
            } else {
                format!("{:.1} mi", ft / 5280.0)
            }
        } else if metres < 950.0 {
            format!("{:.0} m", (metres / 10.0).round() * 10.0)
        } else {
            format!("{:.1} km", metres / 1000.0)
        }
    }

    /// A route's name with the kilometres in it ("Valley Loop · 12 km
    /// rolling") in these units ("… 7.5 mi rolling").
    pub fn route_name(self, name: &str) -> String {
        if !self.imperial() {
            return name.to_string();
        }
        let words: Vec<&str> = name.split(' ').collect();
        let mut out = Vec::with_capacity(words.len());
        let mut i = 0;
        while i < words.len() {
            match (words[i].parse::<f64>(), words.get(i + 1)) {
                (Ok(km), Some(&"km")) | (Ok(km), Some(&"km,")) => {
                    let comma = if words[i + 1].ends_with(',') { "," } else { "" };
                    out.push(format!("{:.1} mi{comma}", km / KM_PER_MI));
                    i += 2;
                }
                (Ok(m), Some(&"m")) => {
                    out.push(format!("{:.0} ft", m / M_PER_FT));
                    i += 2;
                }
                _ => {
                    out.push(words[i].to_string());
                    i += 1;
                }
            }
        }
        out.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        let i = Units::Imperial;
        assert!((i.speed(40.0) - 24.85).abs() < 0.01);
        assert!((i.distance(10_000.0) - 6.214).abs() < 0.001);
        assert!((i.height(100.0) - 328.08).abs() < 0.01);
        assert!((i.weight(75.0) - 165.35).abs() < 0.01);
        assert!((i.weight_to_kg(i.weight(75.0)) - 75.0).abs() < 1e-9);
        let m = Units::Metric;
        assert_eq!(m.speed(40.0), 40.0);
        assert_eq!(m.fmt_distance(12_345.0, 1), "12.3 km");
        assert_eq!(i.fmt_distance(12_345.0, 1), "7.7 mi");
        assert_eq!(m.fmt_short(320.0), "320 m");
        assert_eq!(i.fmt_short(100.0), "330 ft");
        assert_eq!(i.fmt_short(2000.0), "1.2 mi");
    }

    #[test]
    fn route_names_follow_the_units() {
        let i = Units::Imperial;
        assert_eq!(
            i.route_name("Valley Loop · 12 km rolling"),
            "Valley Loop · 7.5 mi rolling"
        );
        assert_eq!(
            i.route_name("Col Loop · 14 km, 420 m climb"),
            "Col Loop · 8.7 mi, 1378 ft climb"
        );
        assert_eq!(Units::Metric.route_name("Park · 4 km"), "Park · 4 km");
    }
}
