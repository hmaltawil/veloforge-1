//! Bluetooth GATT protocol helpers: UUIDs, notification parsers and control
//! point encoders for FTMS, Cycling Power, Heart Rate, CSC and the legacy
//! Wahoo trainer extension. Everything here is pure and unit tested.

use uuid::Uuid;

/// Expand a 16-bit Bluetooth SIG UUID onto the base UUID.
pub const fn uuid16(v: u16) -> Uuid {
    Uuid::from_u128(((v as u128) << 96) | 0x0000_0000_0000_1000_8000_0080_5f9b_34fb)
}

pub const FTMS_SERVICE: Uuid = uuid16(0x1826);
pub const INDOOR_BIKE_DATA: Uuid = uuid16(0x2AD2);
pub const FTMS_CONTROL_POINT: Uuid = uuid16(0x2AD9);
pub const FTMS_STATUS: Uuid = uuid16(0x2ADA);
pub const CPS_SERVICE: Uuid = uuid16(0x1818);
pub const CPS_MEASUREMENT: Uuid = uuid16(0x2A63);
pub const HR_SERVICE: Uuid = uuid16(0x180D);
pub const HR_MEASUREMENT: Uuid = uuid16(0x2A37);
pub const CSC_SERVICE: Uuid = uuid16(0x1816);
pub const CSC_MEASUREMENT: Uuid = uuid16(0x2A5B);
/// Wahoo's proprietary trainer control characteristic (inside the Cycling
/// Power service) used by KICKR/SNAP units on firmware without FTMS.
pub const WAHOO_TRAINER: Uuid = Uuid::from_u128(0xa026e005_0a7d_4ab3_97fa_f1500f9feb8b);

/// Human-readable name for a UUID, e.g. "Heart Rate Measurement (2a37)".
pub fn describe_uuid(u: Uuid) -> String {
    let base = uuid16(0).as_u128();
    let v = u.as_u128();
    if v & !(0xFFFF_FFFFu128 << 96) != base {
        return if u == WAHOO_TRAINER {
            "Wahoo trainer control".into()
        } else {
            u.to_string()
        };
    }
    let short = (v >> 96) as u32;
    let name = match short {
        0x1800 => "Generic Access",
        0x1801 => "Generic Attribute",
        0x180A => "Device Information",
        0x180F => "Battery",
        0x180D => "Heart Rate service",
        0x1816 => "Cycling Speed and Cadence service",
        0x1818 => "Cycling Power service",
        0x1826 => "Fitness Machine service (FTMS)",
        0x2A37 => "heart rate",
        0x2A5B => "speed/cadence",
        0x2A63 => "cycling power",
        0x2AD2 => "indoor bike data",
        0x2AD9 => "FTMS control point",
        0x2ADA => "FTMS status",
        _ => "",
    };
    if name.is_empty() {
        format!("{short:04x}")
    } else {
        format!("{name} ({short:04x})")
    }
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Self { b, i: 0 }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.i..self.i + n)?;
        self.i += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|s| s[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|s| u16::from_le_bytes([s[0], s[1]]))
    }
    fn i16(&mut self) -> Option<i16> {
        self.take(2).map(|s| i16::from_le_bytes([s[0], s[1]]))
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        self.take(n).map(|_| ())
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct BikeData {
    pub speed_kmh: Option<f32>,
    pub cadence: Option<f32>,
    pub power: Option<i16>,
    pub heart_rate: Option<u8>,
}

/// Parse an FTMS Indoor Bike Data notification. Truncated packets yield the
/// fields that were present before the cut.
pub fn parse_indoor_bike_data(b: &[u8]) -> Option<BikeData> {
    let mut r = Reader::new(b);
    let flags = r.u16()?;
    let mut d = BikeData::default();
    let mut fill = || -> Option<()> {
        // Bit 0 is "More Data": when clear, instantaneous speed is present.
        if flags & 0x0001 == 0 {
            d.speed_kmh = Some(r.u16()? as f32 / 100.0);
        }
        if flags & 0x0002 != 0 {
            r.skip(2)?; // average speed
        }
        if flags & 0x0004 != 0 {
            d.cadence = Some(r.u16()? as f32 / 2.0);
        }
        if flags & 0x0008 != 0 {
            r.skip(2)?; // average cadence
        }
        if flags & 0x0010 != 0 {
            r.skip(3)?; // total distance
        }
        if flags & 0x0020 != 0 {
            r.skip(2)?; // resistance level
        }
        if flags & 0x0040 != 0 {
            d.power = Some(r.i16()?);
        }
        if flags & 0x0080 != 0 {
            r.skip(2)?; // average power
        }
        if flags & 0x0100 != 0 {
            r.skip(5)?; // expended energy
        }
        if flags & 0x0200 != 0 {
            d.heart_rate = Some(r.u8()?);
        }
        Some(())
    };
    fill();
    Some(d)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrankData {
    pub revs: u16,
    /// Last crank event time in 1/1024 s.
    pub time: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WheelData {
    pub revs: u32,
    /// Last wheel event time in 1/2048 s.
    pub time: u16,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PowerData {
    pub power: i16,
    pub wheel: Option<WheelData>,
    pub crank: Option<CrankData>,
}

/// Parse a Cycling Power Measurement notification.
pub fn parse_cycling_power(b: &[u8]) -> Option<PowerData> {
    let mut r = Reader::new(b);
    let flags = r.u16()?;
    let power = r.i16()?;
    let mut crank = None;
    let mut wheel = None;
    let mut fill = || -> Option<()> {
        if flags & 0x0001 != 0 {
            r.skip(1)?; // pedal power balance
        }
        if flags & 0x0004 != 0 {
            r.skip(2)?; // accumulated torque
        }
        if flags & 0x0010 != 0 {
            let b = r.take(4)?;
            wheel = Some(WheelData {
                revs: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                time: r.u16()?,
            });
        }
        if flags & 0x0020 != 0 {
            crank = Some(CrankData {
                revs: r.u16()?,
                time: r.u16()?,
            });
        }
        Some(())
    };
    fill();
    Some(PowerData {
        power,
        wheel,
        crank,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeartRate {
    pub bpm: u16,
    /// Whether the strap touches the skin, if it reports that.
    pub contact: Option<bool>,
    /// Beat-to-beat (RR) intervals in seconds.
    pub rr: Vec<f32>,
}

/// Parse a Heart Rate Measurement notification.
pub fn parse_heart_rate(b: &[u8]) -> Option<HeartRate> {
    let mut r = Reader::new(b);
    let flags = r.u8()?;
    let bpm = if flags & 0x01 != 0 {
        r.u16()?
    } else {
        u16::from(r.u8()?)
    };
    // Bits 1-2: 2 = contact supported but not detected, 3 = detected.
    let contact = match (flags >> 1) & 0x03 {
        2 => Some(false),
        3 => Some(true),
        _ => None,
    };
    let mut rr = Vec::new();
    // Skip the energy-expended field if present, then read RR intervals.
    if (flags & 0x08 == 0 || r.skip(2).is_some()) && flags & 0x10 != 0 {
        while let Some(v) = r.u16() {
            rr.push(v as f32 / 1024.0);
        }
    }
    Some(HeartRate { bpm, contact, rr })
}

/// Tracks recent beat-to-beat intervals to spot a noisy heart rate signal:
/// a real heartbeat is regular, electrical noise produces erratic intervals.
#[derive(Debug, Default)]
pub struct BeatCheck {
    recent: std::collections::VecDeque<f32>,
}

impl BeatCheck {
    /// Add intervals; returns Some(true) when the signal looks like noise.
    pub fn update(&mut self, rr: &[f32]) -> Option<bool> {
        for &v in rr {
            self.recent.push_back(v);
            if self.recent.len() > 16 {
                self.recent.pop_front();
            }
        }
        if self.recent.len() < 8 {
            return None;
        }
        let n = self.recent.len() as f32;
        let mean = self.recent.iter().sum::<f32>() / n;
        let var = self.recent.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / n;
        // Even very irregular real rhythms rarely exceed ~25% variation.
        Some(var.sqrt() / mean > 0.3)
    }
}

/// Heart rate implied by beat-to-beat intervals, ignoring implausible ones.
pub fn rr_bpm(rr: &[f32]) -> Option<f32> {
    let valid: Vec<f32> = rr
        .iter()
        .copied()
        .filter(|s| (0.25..2.0).contains(s))
        .collect();
    (!valid.is_empty()).then(|| 60.0 * valid.len() as f32 / valid.iter().sum::<f32>())
}

/// Parse a CSC Measurement notification, returning crank data if present.
pub fn parse_csc(b: &[u8]) -> Option<CrankData> {
    let mut r = Reader::new(b);
    let flags = r.u8()?;
    if flags & 0x01 != 0 {
        r.skip(6)?;
    }
    if flags & 0x02 != 0 {
        Some(CrankData {
            revs: r.u16()?,
            time: r.u16()?,
        })
    } else {
        None
    }
}

/// Turns cumulative crank revolution events into a cadence in rpm.
#[derive(Debug, Default)]
pub struct CadenceCalc {
    last: Option<CrankData>,
    last_event_s: f64,
    cadence: f32,
}

impl CadenceCalc {
    pub fn update(&mut self, c: CrankData, now_s: f64) -> f32 {
        match self.last {
            Some(prev) if prev.revs != c.revs && prev.time != c.time => {
                let revs = c.revs.wrapping_sub(prev.revs) as f32;
                let dt = c.time.wrapping_sub(prev.time) as f32 / 1024.0;
                let rpm = revs * 60.0 / dt;
                if rpm.is_finite() && rpm < 250.0 {
                    self.cadence = rpm;
                }
                self.last = Some(c);
                self.last_event_s = now_s;
            }
            Some(_) => {
                if now_s - self.last_event_s > 3.0 {
                    self.cadence = 0.0;
                }
            }
            None => {
                self.last = Some(c);
                self.last_event_s = now_s;
            }
        }
        self.cadence
    }
}

/// Turns cumulative wheel revolution events into a speed in km/h.
#[derive(Debug)]
pub struct WheelSpeedCalc {
    /// Wheel circumference in metres (700x25c ≈ 2.105 m).
    pub circumference: f32,
    last: Option<WheelData>,
    last_event_s: f64,
    speed: f32,
}

impl Default for WheelSpeedCalc {
    fn default() -> Self {
        WheelSpeedCalc {
            circumference: 2.105,
            last: None,
            last_event_s: 0.0,
            speed: 0.0,
        }
    }
}

impl WheelSpeedCalc {
    pub fn update(&mut self, w: WheelData, now_s: f64) -> f32 {
        match self.last {
            Some(prev) if prev.revs != w.revs && prev.time != w.time => {
                let revs = w.revs.wrapping_sub(prev.revs) as f32;
                let dt = w.time.wrapping_sub(prev.time) as f32 / 2048.0;
                let kmh = revs * self.circumference / dt * 3.6;
                if kmh.is_finite() && kmh < 120.0 {
                    self.speed = kmh;
                }
                self.last = Some(w);
                self.last_event_s = now_s;
            }
            Some(_) => {
                if now_s - self.last_event_s > 3.0 {
                    self.speed = 0.0;
                }
            }
            None => {
                self.last = Some(w);
                self.last_event_s = now_s;
            }
        }
        self.speed
    }
}

/// FTMS control point commands.
pub mod ftms {
    pub const OP_REQUEST_CONTROL: u8 = 0x00;
    pub const OP_SET_TARGET_POWER: u8 = 0x05;
    pub const OP_START: u8 = 0x07;
    pub const OP_SET_SIM: u8 = 0x11;
    pub const OP_SPIN_DOWN: u8 = 0x13;
    pub const OP_RESPONSE: u8 = 0x80;
    /// Fitness Machine Status opcode for spin down progress.
    pub const STATUS_SPIN_DOWN: u8 = 0x14;

    pub fn request_control() -> Vec<u8> {
        vec![OP_REQUEST_CONTROL]
    }

    /// Spin down (calibration) control: start, or ignore/cancel.
    pub fn spin_down(start: bool) -> Vec<u8> {
        vec![OP_SPIN_DOWN, if start { 0x01 } else { 0x02 }]
    }

    /// Target speed window (km/h) from a successful spin down response.
    pub fn parse_spin_down_targets(b: &[u8]) -> Option<(f32, f32)> {
        match b {
            [OP_RESPONSE, OP_SPIN_DOWN, 0x01, l0, l1, h0, h1, ..] => Some((
                u16::from_le_bytes([*l0, *l1]) as f32 / 100.0,
                u16::from_le_bytes([*h0, *h1]) as f32 / 100.0,
            )),
            _ => None,
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum SpinDownStatus {
        SpeedUp,
        Success,
        Error,
        StopPedaling,
    }

    /// Spin down progress from a Fitness Machine Status notification.
    pub fn parse_spin_down_status(b: &[u8]) -> Option<SpinDownStatus> {
        match b {
            [STATUS_SPIN_DOWN, 0x01, ..] => Some(SpinDownStatus::SpeedUp),
            [STATUS_SPIN_DOWN, 0x02, ..] => Some(SpinDownStatus::Success),
            [STATUS_SPIN_DOWN, 0x03, ..] => Some(SpinDownStatus::Error),
            [STATUS_SPIN_DOWN, 0x04, ..] => Some(SpinDownStatus::StopPedaling),
            _ => None,
        }
    }

    pub fn start() -> Vec<u8> {
        vec![OP_START]
    }

    pub fn set_target_power(watts: i16) -> Vec<u8> {
        let b = watts.to_le_bytes();
        vec![OP_SET_TARGET_POWER, b[0], b[1]]
    }

    /// Indoor bike simulation: grade in percent, wind in m/s, rolling
    /// resistance coefficient and wind resistance coefficient (kg/m).
    pub fn set_simulation(grade_pct: f32, wind_mps: f32, crr: f32, cw: f32) -> Vec<u8> {
        let wind =
            ((wind_mps * 1000.0).round() as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let grade =
            ((grade_pct * 100.0).round() as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let crr = (crr * 10000.0).round().clamp(0.0, 255.0) as u8;
        let cw = (cw * 100.0).round().clamp(0.0, 255.0) as u8;
        let w = wind.to_le_bytes();
        let g = grade.to_le_bytes();
        vec![OP_SET_SIM, w[0], w[1], g[0], g[1], crr, cw]
    }

    /// Returns (request opcode, result code) for a control point response.
    pub fn parse_response(b: &[u8]) -> Option<(u8, u8)> {
        match b {
            [OP_RESPONSE, op, result, ..] => Some((*op, *result)),
            _ => None,
        }
    }

    pub fn result_text(code: u8) -> &'static str {
        match code {
            0x01 => "success",
            0x02 => "not supported",
            0x03 => "invalid parameter",
            0x04 => "operation failed",
            0x05 => "control not permitted",
            _ => "unknown result",
        }
    }
}

/// Legacy Wahoo trainer commands (pre-FTMS KICKR / SNAP firmware).
pub mod wahoo {
    pub fn unlock() -> Vec<u8> {
        vec![0x20, 0xEE, 0xFC]
    }

    pub fn set_erg(watts: u16) -> Vec<u8> {
        let b = watts.to_le_bytes();
        vec![0x42, b[0], b[1]]
    }

    /// Enter simulation mode with total mass (kg), rolling resistance and
    /// wind resistance coefficients.
    pub fn set_sim_mode(mass_kg: f32, crr: f32, cw: f32) -> Vec<u8> {
        let m = ((mass_kg * 100.0).round().clamp(0.0, 65535.0) as u16).to_le_bytes();
        let r = ((crr * 1000.0).round().clamp(0.0, 65535.0) as u16).to_le_bytes();
        let w = ((cw * 1000.0).round().clamp(0.0, 65535.0) as u16).to_le_bytes();
        vec![0x43, m[0], m[1], r[0], r[1], w[0], w[1]]
    }

    /// Grade in percent, encoded as a fraction in [-1, 1] mapped onto u16.
    pub fn set_grade(grade_pct: f32) -> Vec<u8> {
        let g = (grade_pct / 100.0).clamp(-1.0, 1.0);
        let v = (((g + 1.0) * 65535.0 / 2.0).round() as u32).min(65535) as u16;
        let b = v.to_le_bytes();
        vec![0x46, b[0], b[1]]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_uuids() {
        assert_eq!(describe_uuid(HR_MEASUREMENT), "heart rate (2a37)");
        assert_eq!(describe_uuid(uuid16(0x1234)), "1234");
        assert_eq!(describe_uuid(WAHOO_TRAINER), "Wahoo trainer control");
    }

    #[test]
    fn uuid16_expands_to_base_uuid() {
        assert_eq!(
            FTMS_SERVICE.to_string(),
            "00001826-0000-1000-8000-00805f9b34fb"
        );
        assert_eq!(
            HR_MEASUREMENT.to_string(),
            "00002a37-0000-1000-8000-00805f9b34fb"
        );
    }

    #[test]
    fn indoor_bike_data_speed_cadence_power() {
        // flags: speed present (bit0 clear), cadence (bit2), power (bit6)
        let flags: u16 = 0x0044;
        let mut b = flags.to_le_bytes().to_vec();
        b.extend(3250u16.to_le_bytes()); // 32.50 km/h
        b.extend(180u16.to_le_bytes()); // 90 rpm
        b.extend(245i16.to_le_bytes());
        let d = parse_indoor_bike_data(&b).unwrap();
        assert_eq!(d.speed_kmh, Some(32.5));
        assert_eq!(d.cadence, Some(90.0));
        assert_eq!(d.power, Some(245));
        assert_eq!(d.heart_rate, None);
    }

    #[test]
    fn indoor_bike_data_skips_optional_fields() {
        // speed, avg speed, distance, resistance, power, heart rate
        let flags: u16 = 0x0002 | 0x0010 | 0x0020 | 0x0040 | 0x0200;
        let mut b = flags.to_le_bytes().to_vec();
        b.extend(2000u16.to_le_bytes());
        b.extend(1900u16.to_le_bytes());
        b.extend([1, 2, 3]);
        b.extend(10i16.to_le_bytes());
        b.extend(301i16.to_le_bytes());
        b.push(142);
        let d = parse_indoor_bike_data(&b).unwrap();
        assert_eq!(d.speed_kmh, Some(20.0));
        assert_eq!(d.power, Some(301));
        assert_eq!(d.heart_rate, Some(142));
    }

    #[test]
    fn indoor_bike_data_truncated_keeps_prefix() {
        let flags: u16 = 0x0040;
        let mut b = flags.to_le_bytes().to_vec();
        b.extend(1500u16.to_le_bytes());
        b.push(0x10); // power cut in half
        let d = parse_indoor_bike_data(&b).unwrap();
        assert_eq!(d.speed_kmh, Some(15.0));
        assert_eq!(d.power, None);
    }

    #[test]
    fn cycling_power_with_crank() {
        let flags: u16 = 0x0020;
        let mut b = flags.to_le_bytes().to_vec();
        b.extend(210i16.to_le_bytes());
        b.extend(77u16.to_le_bytes());
        b.extend(4096u16.to_le_bytes());
        let p = parse_cycling_power(&b).unwrap();
        assert_eq!(p.power, 210);
        assert_eq!(
            p.crank,
            Some(CrankData {
                revs: 77,
                time: 4096
            })
        );
    }

    #[test]
    fn wheel_speed_from_cycling_power() {
        let flags: u16 = 0x0010;
        let mut b = flags.to_le_bytes().to_vec();
        b.extend(150i16.to_le_bytes());
        b.extend(1000u32.to_le_bytes());
        b.extend(2048u16.to_le_bytes());
        let p = parse_cycling_power(&b).unwrap();
        assert_eq!(
            p.wheel,
            Some(WheelData {
                revs: 1000,
                time: 2048
            })
        );
        let mut w = WheelSpeedCalc::default();
        w.update(p.wheel.unwrap(), 0.0);
        // 4 revolutions of 2.105 m in 1 s = 30.3 km/h
        let kmh = w.update(
            WheelData {
                revs: 1004,
                time: 4096,
            },
            1.0,
        );
        assert!((kmh - 30.312).abs() < 0.01, "{kmh}");
    }

    #[test]
    fn heart_rate_formats() {
        assert_eq!(parse_heart_rate(&[0x00, 150]).unwrap().bpm, 150);
        assert_eq!(parse_heart_rate(&[0x01, 0x2C, 0x01]).unwrap().bpm, 300);
        // Typical TICKR packet: contact detected, RR interval of 0.833 s (72 bpm).
        let h = parse_heart_rate(&[0x16, 72, 0x55, 0x03]).unwrap();
        assert_eq!(h.bpm, 72);
        assert_eq!(h.contact, Some(true));
        assert!((rr_bpm(&h.rr).unwrap() - 72.0).abs() < 0.2);
        // No skin contact.
        assert_eq!(parse_heart_rate(&[0x04, 195]).unwrap().contact, Some(false));
        // Energy expended field is skipped before the RR intervals.
        let h = parse_heart_rate(&[0x1E, 80, 0x10, 0x00, 0x00, 0x03]).unwrap();
        assert_eq!(h.rr, vec![0.75]);
        assert_eq!(rr_bpm(&[5.0]), None);
    }

    #[test]
    fn noisy_beats_are_detected() {
        // Intervals decoded from a real TICKR packet stream reading "204 bpm" at rest.
        let packets: [&[u8]; 6] = [
            &[0x16, 0xcc, 0x4d, 0x02, 0xeb, 0x00, 0xc6, 0x00],
            &[0x16, 0xcc, 0xfb, 0x01, 0xf8, 0x00, 0xb5, 0x00, 0x06, 0x01],
            &[0x16, 0xcc, 0x8c, 0x01, 0xfc, 0x00],
            &[0x16, 0xcc, 0x5a, 0x01, 0xf9, 0x01, 0xa7, 0x00, 0xc2, 0x00],
            &[0x16, 0xcc, 0xec, 0x00, 0x01, 0x01, 0x99, 0x01],
            &[0x16, 0xcc, 0xfc, 0x00, 0xf6, 0x00, 0xfe, 0x00, 0x08, 0x01],
        ];
        let mut check = BeatCheck::default();
        let mut verdict = None;
        for p in packets {
            let h = parse_heart_rate(p).unwrap();
            assert_eq!(h.bpm, 204);
            verdict = check.update(&h.rr).or(verdict);
        }
        assert_eq!(verdict, Some(true));
        // A steady 60-70 bpm heartbeat is not flagged.
        let mut check = BeatCheck::default();
        let steady: Vec<f32> = (0..12)
            .map(|i| 0.9 + 0.04 * ((i % 3) as f32 - 1.0))
            .collect();
        assert_eq!(check.update(&steady), Some(false));
    }

    #[test]
    fn cadence_from_crank_events_handles_wrap() {
        let mut c = CadenceCalc::default();
        c.update(
            CrankData {
                revs: 65535,
                time: 65000,
            },
            0.0,
        );
        // 1 rev in 0.75 s = 80 rpm, both counters wrap
        let rpm = c.update(
            CrankData {
                revs: 0,
                time: 65000u16.wrapping_add(768),
            },
            0.75,
        );
        assert!((rpm - 80.0).abs() < 0.01);
        // no new events for more than 3 s drops to zero
        assert_eq!(
            c.update(
                CrankData {
                    revs: 0,
                    time: 65000u16.wrapping_add(768)
                },
                5.0
            ),
            0.0
        );
    }

    #[test]
    fn ftms_commands() {
        assert_eq!(ftms::set_target_power(250), vec![0x05, 250, 0]);
        assert_eq!(
            ftms::set_simulation(5.0, 0.0, 0.004, 0.51),
            vec![0x11, 0, 0, 0xF4, 0x01, 40, 51]
        );
        assert_eq!(
            ftms::set_simulation(-2.5, 0.0, 0.004, 0.51)[3..5],
            (-250i16).to_le_bytes()
        );
        assert_eq!(
            ftms::parse_response(&[0x80, 0x05, 0x01]),
            Some((0x05, 0x01))
        );
    }

    #[test]
    fn spin_down_messages() {
        assert_eq!(ftms::spin_down(true), vec![0x13, 0x01]);
        assert_eq!(ftms::spin_down(false), vec![0x13, 0x02]);
        // 30.00 .. 34.50 km/h
        let resp = [0x80, 0x13, 0x01, 0xB8, 0x0B, 0x7A, 0x0D];
        assert_eq!(ftms::parse_spin_down_targets(&resp), Some((30.0, 34.5)));
        assert_eq!(ftms::parse_spin_down_targets(&[0x80, 0x13, 0x02]), None);
        assert_eq!(
            ftms::parse_spin_down_status(&[0x14, 0x04]),
            Some(ftms::SpinDownStatus::StopPedaling)
        );
        assert_eq!(
            ftms::parse_spin_down_status(&[0x14, 0x02]),
            Some(ftms::SpinDownStatus::Success)
        );
        assert_eq!(ftms::parse_spin_down_status(&[0x04]), None);
    }

    #[test]
    fn wahoo_commands() {
        assert_eq!(wahoo::set_erg(300), vec![0x42, 0x2C, 0x01]);
        assert_eq!(wahoo::set_grade(0.0), vec![0x46, 0x00, 0x80]);
        assert_eq!(wahoo::set_grade(100.0), vec![0x46, 0xFF, 0xFF]);
    }
}
