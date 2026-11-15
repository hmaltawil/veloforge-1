//! FIT activity files: Garmin's compact binary format, which Strava,
//! intervals.icu, TrainingPeaks and Garmin Connect read best.
//!
//! A file is a header, then messages, then a checksum. Each kind of message
//! is described once by a definition (its fields' numbers, sizes and types)
//! and then written as plain values. Field numbers follow the FIT profile.

use crate::ride::Sample;
use chrono::{DateTime, Utc};

/// FIT times count seconds from 1989-12-31 00:00 UTC.
const FIT_EPOCH: i64 = 631_065_600;

/// Base types.
const ENUM: u8 = 0x00;
const UINT8: u8 = 0x02;
const UINT16: u8 = 0x84;
const UINT32: u8 = 0x86;
const UINT32Z: u8 = 0x8C;

/// Global message numbers.
const FILE_ID: u16 = 0;
const SESSION: u16 = 18;
const LAP: u16 = 19;
const RECORD: u16 = 20;
const EVENT: u16 = 21;
const ACTIVITY: u16 = 34;

fn fit_time(t: DateTime<Utc>) -> u32 {
    (t.timestamp() - FIT_EPOCH).max(0) as u32
}

/// The FIT checksum (CRC-16 with the profile's nibble table).
pub fn crc(data: &[u8]) -> u16 {
    const TABLE: [u16; 16] = [
        0x0000, 0xCC01, 0xD801, 0x1400, 0xF001, 0x3C00, 0x2800, 0xE401, 0xA001, 0x6C00, 0x7800,
        0xB401, 0x5000, 0x9C01, 0x8801, 0x4400,
    ];
    let mut crc = 0u16;
    for &byte in data {
        for nibble in [byte & 0x0F, byte >> 4] {
            let tmp = TABLE[(crc & 0x0F) as usize];
            crc = ((crc >> 4) & 0x0FFF) ^ tmp ^ TABLE[nibble as usize];
        }
    }
    crc
}

/// A field value; `None` is written as the type's "invalid" marker.
#[derive(Clone, Copy)]
enum V {
    U8(Option<u8>),
    U16(Option<u16>),
    U32(Option<u32>),
}

/// One field of a message: (field number, base type).
type Field = (u8, u8);

struct Writer {
    body: Vec<u8>,
}

impl Writer {
    fn define(&mut self, local: u8, global: u16, fields: &[Field]) {
        self.body.push(0x40 | local);
        self.body.push(0); // reserved
        self.body.push(0); // little-endian values
        self.body.extend(global.to_le_bytes());
        self.body.push(fields.len() as u8);
        for &(num, ty) in fields {
            let size = match ty {
                ENUM | UINT8 => 1,
                UINT16 => 2,
                _ => 4,
            };
            self.body.extend([num, size, ty]);
        }
    }

    fn data(&mut self, local: u8, values: &[V]) {
        self.body.push(local);
        for v in values {
            match *v {
                V::U8(x) => self.body.push(x.unwrap_or(0xFF)),
                V::U16(x) => self.body.extend(x.unwrap_or(0xFFFF).to_le_bytes()),
                V::U32(x) => self.body.extend(x.unwrap_or(0xFFFF_FFFF).to_le_bytes()),
            }
        }
    }

    fn finish(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.body.len() + 16);
        out.push(14); // header size
        out.push(0x20); // protocol 2.0
        out.extend(2132u16.to_le_bytes()); // profile 21.32
        out.extend((self.body.len() as u32).to_le_bytes());
        out.extend(b".FIT");
        let header_crc = crc(&out);
        out.extend(header_crc.to_le_bytes());
        out.extend(&self.body);
        let file_crc = crc(&out);
        out.extend(file_crc.to_le_bytes());
        out
    }
}

/// Totals over some samples, for laps and the session.
struct Totals {
    start: u32,
    end: u32,
    /// Wall-clock and moving seconds, in thousandths.
    elapsed: u32,
    timer: u32,
    distance: u32,
    avg_power: Option<u16>,
    max_power: Option<u16>,
    avg_hr: Option<u8>,
    max_hr: Option<u8>,
    avg_cadence: Option<u8>,
    ascent: u16,
    work: u32,
}

/// Totals for `s`; distance counts from `from` metres (where the previous
/// lap ended), so the laps add up to the whole ride.
fn totals(s: &[Sample], from: Option<f64>) -> Totals {
    let first = s.first();
    let last = s.last();
    let start = first.map(|x| fit_time(x.time)).unwrap_or(0);
    let end = last.map(|x| fit_time(x.time)).unwrap_or(start);
    let mean = |v: Vec<f32>| (!v.is_empty()).then(|| v.iter().sum::<f32>() / v.len() as f32);
    let hrs: Vec<f32> = s.iter().filter_map(|x| x.hr).filter(|h| *h > 0.0).collect();
    let cads: Vec<f32> = s
        .iter()
        .filter_map(|x| x.cadence)
        .filter(|c| *c > 0.0)
        .collect();
    let power: Vec<f32> = s.iter().map(|x| x.power).collect();
    let ascent: f64 = s
        .windows(2)
        .map(|w| w[1].ele - w[0].ele)
        .filter(|c| *c > 0.0 && *c < 5.0)
        .sum();
    let distance = match (first, last) {
        (Some(a), Some(b)) => (b.distance - from.unwrap_or(a.distance)).max(0.0),
        _ => 0.0,
    };
    Totals {
        start,
        end,
        elapsed: (end - start + 1) * 1000,
        timer: s.len() as u32 * 1000,
        distance: (distance * 100.0).round() as u32,
        avg_power: mean(power.clone()).map(|p| p.round() as u16),
        max_power: power
            .iter()
            .cloned()
            .reduce(f32::max)
            .map(|p| p.round() as u16),
        avg_hr: mean(hrs.clone()).map(|h| h.round() as u8),
        max_hr: hrs
            .iter()
            .cloned()
            .reduce(f32::max)
            .map(|h| h.round() as u8),
        avg_cadence: mean(cads).map(|c| c.round() as u8),
        ascent: ascent.round() as u16,
        work: power.iter().sum::<f32>().round() as u32,
    }
}

/// A FIT activity: one record a second, a lap from each index in
/// `lap_starts` (the first must be 0), and the ride's normalized power.
pub fn activity(samples: &[Sample], lap_starts: &[usize], np: f32) -> Vec<u8> {
    let mut w = Writer { body: Vec::new() };
    let all = totals(samples, None);

    // File id: an activity, made by a development (unregistered) app.
    w.define(
        0,
        FILE_ID,
        &[
            (0, ENUM),
            (1, UINT16),
            (2, UINT16),
            (3, UINT32Z),
            (4, UINT32),
        ],
    );
    w.data(
        0,
        &[
            V::U8(Some(4)),
            V::U16(Some(255)),
            V::U16(Some(1)),
            V::U32(Some(all.start.max(1))),
            V::U32(Some(all.start)),
        ],
    );

    // Timer start and stop events, with a pause wherever the clock stopped.
    w.define(1, EVENT, &[(253, UINT32), (0, ENUM), (1, ENUM)]);
    let event = |w: &mut Writer, t: u32, kind: u8| {
        w.data(1, &[V::U32(Some(t)), V::U8(Some(0)), V::U8(Some(kind))])
    };
    const START: u8 = 0;
    const STOP: u8 = 1;
    const STOP_ALL: u8 = 4;

    // Records: time, distance (cm), speed (mm/s), altitude ((m + 500) × 5),
    // heart rate, cadence, power.
    w.define(
        2,
        RECORD,
        &[
            (253, UINT32),
            (5, UINT32),
            (6, UINT16),
            (2, UINT16),
            (3, UINT8),
            (4, UINT8),
            (7, UINT16),
        ],
    );
    event(&mut w, all.start, START);
    let mut prev: Option<u32> = None;
    for s in samples {
        let t = fit_time(s.time);
        if let Some(p) = prev {
            if t > p + 2 {
                event(&mut w, p, STOP);
                event(&mut w, t, START);
            }
        }
        prev = Some(t);
        let rate = |v: Option<f32>| v.filter(|v| *v > 0.0).map(|v| v.round().min(254.0) as u8);
        w.data(
            2,
            &[
                V::U32(Some(t)),
                V::U32(Some((s.distance * 100.0).round().max(0.0) as u32)),
                V::U16(Some((s.speed * 1000.0).round().clamp(0.0, 65534.0) as u16)),
                V::U16(Some(
                    ((s.ele + 500.0) * 5.0).round().clamp(0.0, 65534.0) as u16
                )),
                V::U8(rate(s.hr)),
                V::U8(rate(s.cadence)),
                V::U16(Some(s.power.round().clamp(0.0, 65534.0) as u16)),
            ],
        );
    }
    event(&mut w, all.end, STOP_ALL);

    // Laps.
    w.define(
        3,
        LAP,
        &[
            (253, UINT32),
            (254, UINT16),
            (2, UINT32),
            (7, UINT32),
            (8, UINT32),
            (9, UINT32),
            (19, UINT16),
            (20, UINT16),
            (15, UINT8),
            (16, UINT8),
            (17, UINT8),
            (21, UINT16),
            (0, ENUM),
            (1, ENUM),
            (24, ENUM),
            (25, ENUM),
        ],
    );
    let mut starts: Vec<usize> = lap_starts
        .iter()
        .copied()
        .filter(|&i| i < samples.len())
        .collect();
    if starts.first() != Some(&0) {
        starts.insert(0, 0);
    }
    for (k, &a) in starts.iter().enumerate() {
        let b = starts.get(k + 1).copied().unwrap_or(samples.len());
        let lap = totals(
            &samples[a..b],
            a.checked_sub(1).map(|i| samples[i].distance),
        );
        w.data(
            3,
            &[
                V::U32(Some(lap.end)),
                V::U16(Some(k as u16)),
                V::U32(Some(lap.start)),
                V::U32(Some(lap.elapsed)),
                V::U32(Some(lap.timer)),
                V::U32(Some(lap.distance)),
                V::U16(lap.avg_power),
                V::U16(lap.max_power),
                V::U8(lap.avg_hr),
                V::U8(lap.max_hr),
                V::U8(lap.avg_cadence),
                V::U16(Some(lap.ascent)),
                V::U8(Some(9)), // lap
                V::U8(Some(1)), // stop
                V::U8(Some(0)), // manual
                V::U8(Some(2)), // cycling
            ],
        );
    }

    // The session: a virtual ride on an indoor trainer.
    w.define(
        4,
        SESSION,
        &[
            (253, UINT32),
            (254, UINT16),
            (2, UINT32),
            (7, UINT32),
            (8, UINT32),
            (9, UINT32),
            (5, ENUM),
            (6, ENUM),
            (20, UINT16),
            (21, UINT16),
            (34, UINT16),
            (16, UINT8),
            (17, UINT8),
            (18, UINT8),
            (22, UINT16),
            (48, UINT32),
            (25, UINT16),
            (26, UINT16),
            (0, ENUM),
            (1, ENUM),
            (28, ENUM),
        ],
    );
    w.data(
        4,
        &[
            V::U32(Some(all.end)),
            V::U16(Some(0)),
            V::U32(Some(all.start)),
            V::U32(Some(all.elapsed)),
            V::U32(Some(all.timer)),
            V::U32(Some(all.distance)),
            V::U8(Some(2)),  // cycling
            V::U8(Some(58)), // virtual activity
            V::U16(all.avg_power),
            V::U16(all.max_power),
            V::U16((np > 0.0).then(|| np.round() as u16)),
            V::U8(all.avg_hr),
            V::U8(all.max_hr),
            V::U8(all.avg_cadence),
            V::U16(Some(all.ascent)),
            V::U32(Some(all.work)),
            V::U16(Some(0)),
            V::U16(Some(starts.len() as u16)),
            V::U8(Some(8)), // session
            V::U8(Some(1)), // stop
            V::U8(Some(0)), // activity end
        ],
    );

    // The activity, with the local time of its end.
    let offset = chrono::Local::now().offset().local_minus_utc() as i64;
    w.define(
        5,
        ACTIVITY,
        &[
            (253, UINT32),
            (0, UINT32),
            (1, UINT16),
            (2, ENUM),
            (3, ENUM),
            (4, ENUM),
            (5, UINT32),
        ],
    );
    w.data(
        5,
        &[
            V::U32(Some(all.end)),
            V::U32(Some(all.timer)),
            V::U16(Some(1)),
            V::U8(Some(0)),  // manual
            V::U8(Some(26)), // activity
            V::U8(Some(1)),  // stop
            V::U32(Some((all.end as i64 + offset).max(0) as u32)),
        ],
    );
    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    pub fn samples(n: usize) -> Vec<Sample> {
        let t0 = Utc.with_ymd_and_hms(2026, 11, 15, 8, 0, 0).unwrap();
        (0..n)
            .map(|i| Sample {
                time: t0 + chrono::Duration::seconds(i as i64),
                elapsed: i as f32,
                power: 200.0 + (i % 10) as f32,
                cadence: Some(90.0),
                hr: (i > 5).then_some(140.0),
                speed: 9.0,
                distance: i as f64 * 9.0,
                ele: 100.0 + i as f64 * 0.1,
            })
            .collect()
    }

    #[test]
    fn checksum_matches_the_fit_sdk() {
        // The SDK's algorithm on a known header: CRC over the first 12 bytes.
        assert_eq!(crc(b""), 0);
        assert_eq!(crc(b"123456789"), 0xBB3D);
    }

    #[test]
    fn a_file_has_a_valid_header_and_checksum() {
        let f = activity(&samples(120), &[0, 60], 205.0);
        assert_eq!(f[0], 14);
        assert_eq!(&f[8..12], b".FIT");
        let size = u32::from_le_bytes(f[4..8].try_into().unwrap()) as usize;
        assert_eq!(f.len(), 14 + size + 2);
        assert_eq!(crc(&f[..12]), u16::from_le_bytes([f[12], f[13]]));
        // The checksum over everything, its own two bytes included, is 0.
        assert_eq!(crc(&f), 0);
    }

    #[test]
    fn laps_add_up_to_the_ride() {
        let s = samples(120);
        let whole = totals(&s, None).distance;
        let a = totals(&s[..60], None).distance;
        let b = totals(&s[60..], Some(s[59].distance)).distance;
        assert_eq!(a + b, whole);
    }
}
