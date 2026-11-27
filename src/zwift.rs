//! The Zwift Click as a shifter: its Bluetooth messages, decoded.
//!
//! Zwift's controllers (Click, Play, Ride) speak Zwift's own protocol over
//! a custom Bluetooth service, not a standard one. Nothing here is from
//! Zwift: it is what open-source projects found by listening to the
//! devices, which this follows:
//!
//! - ajchellew/zwiftplay (the service, the "RideOn" handshake, the Click's
//!   message 0x37 and its test bytes, captured from a real Click by
//!   cagnulein of qdomyos-zwift);
//! - OpenBikeControl/bikecontrol, formerly SwiftControl (the newer Ride
//!   service FC82, the Click v2 and Ride message 0x23 and its button bits,
//!   the advertised device types, the Click v2 handshake reply);
//! - makinolo.com's write-ups of the Play and Ride protocols.
//!
//! Supported: the Click (v1): its + and − buttons; the Click v2: + on its
//! right half, − on its left half (Zwift locks that half to its own app
//! until it has unlocked it, see the README), and B on the right half as a
//! second − . A Zwift Ride's shift buttons decode the same way, untested.
//! The Play encrypts its messages and is not supported.
//!
//! Pure functions only: the Bluetooth side is in `devices.rs`.

use uuid::Uuid;

/// The service of the Click (v1) and Play.
pub const SERVICE: Uuid = Uuid::from_u128(0x00000001_19ca_4651_86e5_fa29dcdd09d1);
/// The service of the Ride and Click v2 (and the Play after a firmware
/// update), with the same characteristics.
pub const SERVICE_FC82: Uuid = Uuid::from_u128(0x0000fc82_0000_1000_8000_00805f9b34fb);
/// Notifications: button presses, battery, idle.
pub const ASYNC: Uuid = Uuid::from_u128(0x00000002_19ca_4651_86e5_fa29dcdd09d1);
/// Written to: the handshake.
pub const SYNC_RX: Uuid = Uuid::from_u128(0x00000003_19ca_4651_86e5_fa29dcdd09d1);
/// Indications: the reply to the handshake.
pub const SYNC_TX: Uuid = Uuid::from_u128(0x00000004_19ca_4651_86e5_fa29dcdd09d1);

/// Zwift, Inc.'s Bluetooth company id, in its advertisements.
pub const MANUFACTURER_ID: u16 = 0x094A;

/// "RideOn": the handshake, and the start of the reply.
pub const RIDE_ON: &[u8] = b"RideOn";

/// Sent to a Click v2's left half after the handshake: a half that Zwift
/// has unlocked recently stays unlocked across reconnects (found by
/// OpenBikeControl, used by qdomyos-zwift and Torqa). It does nothing to a
/// locked half: no key, no answer to Zwift's challenge.
pub const UNLOCK_ACK: &[u8] = &[0xFF, 0x04, 0x00];

/// The handshake is sent again this often, so a Click v2 never dozes off
/// (it drops its link after about a minute without traffic).
pub const KEEPALIVE_SECS: u64 = 3;

/// Which Zwift device an advertisement is from: the first byte of its
/// manufacturer data (after the company id).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    ClickV1,
    /// The Click v2 is two devices: the right half has +, the left −.
    ClickV2Right,
    ClickV2Left,
    PlayLeft,
    PlayRight,
    RideLeft,
    RideRight,
    /// Something else of Zwift's (a Zwift Hub trainer is 0x01).
    Other(u8),
}

impl Kind {
    pub fn from_manufacturer_data(data: &[u8]) -> Option<Kind> {
        Some(match *data.first()? {
            0x09 => Kind::ClickV1,
            0x0A => Kind::ClickV2Right,
            0x0B => Kind::ClickV2Left,
            0x02 => Kind::PlayRight,
            0x03 => Kind::PlayLeft,
            0x07 => Kind::RideRight,
            0x08 => Kind::RideLeft,
            b => Kind::Other(b),
        })
    }

    /// Shown after the device's name: "Click v2, right half (+)".
    pub fn label(self) -> String {
        match self {
            Kind::ClickV1 => "Click".into(),
            Kind::ClickV2Right => "Click v2, right half (+)".into(),
            Kind::ClickV2Left => "Click v2, left half (−)".into(),
            Kind::PlayLeft => "Play, left (not supported)".into(),
            Kind::PlayRight => "Play, right (not supported)".into(),
            Kind::RideLeft => "Ride, left".into(),
            Kind::RideRight => "Ride, right".into(),
            Kind::Other(b) => format!("Zwift device type 0x{b:02x}"),
        }
    }

    /// A controller (shift buttons), rather than a trainer or anything else.
    pub fn is_controller(self) -> bool {
        !matches!(self, Kind::Other(_))
    }

    fn is_click_v2(self) -> bool {
        matches!(self, Kind::ClickV2Left | Kind::ClickV2Right)
    }
}

/// What to write to start the device talking. The Click v2 is greeted
/// with the start of its own reply, "RideOn" 02 03 (as BikeControl does);
/// everything else with plain "RideOn", which also asks for its messages
/// unencrypted.
pub fn handshake(kind: Option<Kind>) -> Vec<u8> {
    let mut h = RIDE_ON.to_vec();
    if kind.is_some_and(Kind::is_click_v2) {
        h.extend([0x02, 0x03]);
    }
    h
}

/// The other greeting, tried once if the first gets no answer.
pub fn other_handshake(sent: &[u8]) -> Vec<u8> {
    if sent.len() > RIDE_ON.len() {
        RIDE_ON.to_vec()
    } else {
        handshake(Some(Kind::ClickV2Right))
    }
}

/// Which buttons are down.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Buttons {
    /// Shift to a harder gear: + .
    pub up: bool,
    /// Shift to an easier gear: − .
    pub down: bool,
    /// The other buttons held (a Click v2's or Ride's Y, Z, A and the
    /// rest), one bit each, for the music; 0 on a Click (v1).
    pub other: u64,
}

/// One message from the device, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// The reply to the handshake: "RideOn" and what follows it (01 03 from
    /// a Click, 02 03 from a Click v2, then a key we do not need).
    RideOn(Vec<u8>),
    /// The buttons, from a Click (0x37) or a Click v2 / Ride (0x23).
    Buttons(Buttons),
    /// Nothing happening (0x15), about once a second.
    Idle,
    /// Battery level in percent (0x19).
    Battery(u8),
    /// A Click v2 saying it has stopped (FF 05 00 EA 05 or FF 05 00 FA 05):
    /// seen when Zwift has it locked.
    Stopped,
    /// A Click v2 half that Zwift has locked, answering the handshake with
    /// Zwift's challenge (FF 03 …) instead of RideOn.
    Locked,
    /// The device is leaving (0xFE).
    Goodbye,
    /// A message we do not use, by its type byte.
    Other(u8),
    /// Could not be read: why.
    Malformed(&'static str),
}

/// A message type of the Click (v1): its two buttons.
const CLICK: u8 = 0x37;
/// The Ride's (and Click v2's) keypad message.
const KEYPAD: u8 = 0x23;
const IDLE: u8 = 0x15;
const BATTERY: u8 = 0x19;
const GOODBYE: u8 = 0xFE;

/// The Ride and Click v2 report every button as one bit, set while it is
/// up. Right side: the shift-up buttons (+ on a Click v2); left side: the
/// shift-down ones (− on a Click v2).
const SHIFT_UP_RIGHT: u64 = 0x1000;
const SHIFT_DOWN_RIGHT: u64 = 0x2000;
const SHIFT_UP_LEFT: u64 = 0x0100;
const SHIFT_DOWN_LEFT: u64 = 0x0200;
/// B on the right half: shifts down too, so a Click v2 whose left half is
/// locked can still shift both ways.
const B: u64 = 0x0020;
/// The Click v2's other face buttons, as the Ride numbers them: A (right),
/// Y (top) and Z (left). Not used for shifting: free for the music.
/// The pad on a Click v2's left half (and a Ride's left hood): left, up,
/// right and down, for finding your way round the app.
pub const PAD_LEFT: u64 = 0x0001;
pub const PAD_UP: u64 = 0x0002;
pub const PAD_RIGHT: u64 = 0x0004;
pub const PAD_DOWN: u64 = 0x0008;
pub const PAD: u64 = PAD_LEFT | PAD_UP | PAD_RIGHT | PAD_DOWN;
pub const A: u64 = 0x0010;
pub const Y: u64 = 0x0040;
pub const Z: u64 = 0x0080;
/// Every button bit the keypad message can carry.
const ALL_BUTTONS: u64 = 0xFFFF;

/// Decode one notification or indication.
pub fn decode(bytes: &[u8]) -> Message {
    if bytes.starts_with(RIDE_ON) {
        return Message::RideOn(bytes[RIDE_ON.len()..].to_vec());
    }
    let Some((&kind, body)) = bytes.split_first() else {
        return Message::Malformed("empty");
    };
    match kind {
        CLICK => match click_buttons(body) {
            Ok(b) => Message::Buttons(b),
            Err(why) => Message::Malformed(why),
        },
        KEYPAD => match keypad_buttons(body) {
            Ok(b) => Message::Buttons(b),
            Err(why) => Message::Malformed(why),
        },
        IDLE => Message::Idle,
        BATTERY => match fields(body) {
            // Field 1 on a Click; field 2 on a Click v2 (19 10 50: 80%).
            Ok(f) => match f.iter().find(|(n, _)| *n == 1 || *n == 2) {
                Some(&(_, level)) => Message::Battery(level.min(100) as u8),
                None => Message::Malformed("battery message without a level"),
            },
            Err(why) => Message::Malformed(why),
        },
        GOODBYE if body.is_empty() => Message::Goodbye,
        0xFF if matches!(body, [0x05, 0x00, 0xEA | 0xFA, 0x05]) => Message::Stopped,
        0xFF if body.first() == Some(&0x03) => Message::Locked,
        k => Message::Other(k),
    }
}

/// A Click (v1): field 1 is +, field 2 is −, and 0 means pressed: idle is
/// 08 01 10 01. A field left out is taken as not pressed.
fn click_buttons(body: &[u8]) -> Result<Buttons, &'static str> {
    let mut b = Buttons::default();
    let mut seen = false;
    for (n, v) in fields(body)? {
        match n {
            1 => b.up = v == 0,
            2 => b.down = v == 0,
            _ => continue,
        }
        seen = true;
    }
    if seen {
        Ok(b)
    } else {
        Err("Click message without its buttons")
    }
}

/// A Ride or Click v2: field 1 holds a bit per button, cleared while it is
/// pressed (all set, 08 FF FF FF FF 0F, is nothing pressed). Other fields
/// (the Ride's paddles) are not used.
fn keypad_buttons(body: &[u8]) -> Result<Buttons, &'static str> {
    let Some(&(_, map)) = fields(body)?.iter().find(|(n, _)| *n == 1) else {
        return Err("keypad message without its buttons");
    };
    let down = |mask: u64| map & mask == 0;
    let shifting = SHIFT_UP_RIGHT | SHIFT_DOWN_RIGHT | SHIFT_UP_LEFT | SHIFT_DOWN_LEFT | B;
    Ok(Buttons {
        up: down(SHIFT_UP_RIGHT) || down(SHIFT_DOWN_RIGHT),
        down: down(SHIFT_UP_LEFT) || down(SHIFT_DOWN_LEFT) || down(B),
        other: !map & ALL_BUTTONS & !shifting,
    })
}

/// The whole-number fields of a protobuf message: (field number, value).
/// Fields of other kinds are stepped over.
fn fields(mut b: &[u8]) -> Result<Vec<(u32, u64)>, &'static str> {
    let mut out = Vec::new();
    while !b.is_empty() {
        let key = varint(&mut b)?;
        let number = (key >> 3) as u32;
        match key & 7 {
            0 => out.push((number, varint(&mut b)?)),
            1 => b = b.get(8..).ok_or("message cut short")?,
            2 => {
                let len = varint(&mut b)? as usize;
                b = b.get(len..).ok_or("message cut short")?;
            }
            5 => b = b.get(4..).ok_or("message cut short")?,
            _ => return Err("not a message VeloForge knows"),
        }
    }
    Ok(out)
}

fn varint(b: &mut &[u8]) -> Result<u64, &'static str> {
    let mut v = 0u64;
    for i in 0..10 {
        let (&byte, rest) = b.split_first().ok_or("message cut short")?;
        *b = rest;
        v |= ((byte & 0x7f) as u64) << (7 * i);
        if byte & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err("number too long")
}

/// Turns the button state, as it is reported over and over, into presses:
/// a shift for each button that has gone down since the last message.
#[derive(Debug, Default)]
pub struct Presses {
    last: Buttons,
    pressed_other: u64,
    released_other: u64,
}

impl Presses {
    /// The shifts this message brings: (up, down).
    pub fn update(&mut self, now: Buttons) -> (bool, bool) {
        let up = now.up && !self.last.up;
        let down = now.down && !self.last.down;
        self.pressed_other = now.other & !self.last.other;
        self.released_other = self.last.other & !now.other;
        self.last = now;
        (up, down)
    }

    /// The other buttons (not shifting) that went down with the last
    /// message, one bit each.
    pub fn other_pressed(&self) -> u64 {
        self.pressed_other
    }

    /// The other buttons that came back up with the last message.
    pub fn other_released(&self) -> u64 {
        self.released_other
    }
}

/// Bytes as hex, for the log: "37 08 00 10 01".
pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(hex: &str) -> Vec<u8> {
        hex.split_whitespace()
            .map(|h| u8::from_str_radix(h, 16).unwrap())
            .collect()
    }

    fn buttons(hex: &str) -> Buttons {
        match decode(&bytes(hex)) {
            Message::Buttons(b) => b,
            m => panic!("{hex}: {m:?}"),
        }
    }

    const NONE: Buttons = Buttons {
        up: false,
        down: false,
        other: 0,
    };
    const UP: Buttons = Buttons {
        up: true,
        down: false,
        other: 0,
    };
    const DOWN: Buttons = Buttons {
        up: false,
        down: true,
        other: 0,
    };

    #[test]
    fn click_v1_buttons() {
        // Captured from a real Click (zwiftplay's ZwiftClickTest).
        assert_eq!(buttons("37 08 01 10 01"), NONE);
        assert_eq!(buttons("37 08 00 10 01"), UP);
        assert_eq!(buttons("37 08 01 10 00"), DOWN);
        assert_eq!(
            buttons("37 08 00 10 00"),
            Buttons {
                up: true,
                down: true,
                other: 0,
            }
        );
        // A field left out is not a press; an unknown one is ignored.
        assert_eq!(buttons("37 10 01"), NONE);
        assert_eq!(buttons("37 08 00 10 01 18 05"), UP);
    }

    #[test]
    fn click_v2_and_ride_buttons() {
        // Nothing pressed: every bit set (BikeControl's released frame).
        assert_eq!(buttons("23 08 ff ff ff ff 0f"), NONE);
        // + (shift up, right) and − (shift up, left) on a Click v2.
        assert_eq!(buttons("23 08 ff df ff ff 0f"), UP);
        assert_eq!(buttons("23 08 ff fd ff ff 0f"), DOWN);
        assert_eq!(
            buttons("23 08 ff dd ff ff 0f"),
            Buttons {
                up: true,
                down: true,
                other: 0,
            }
        );
        // B on the right half shifts down as well.
        assert_eq!(buttons("23 08 df ff ff ff 0f"), DOWN);
        // Left on the pad (makinolo's example, FFFFFFFE) is not a shift,
        // but one of the other buttons.
        assert_eq!(
            buttons("23 08 fe ff ff ff 0f"),
            Buttons {
                up: false,
                down: false,
                other: 0x1,
            }
        );
        // Y, A and Z on the right half: for the music, not shifting.
        // (Seven bits a byte: Z's bit is the second byte's first.)
        for (map, bit) in [("bf ff", Y), ("ef ff", A), ("ff fe", Z)] {
            assert_eq!(
                buttons(&format!("23 08 {map} ff ff 0f")),
                Buttons {
                    up: false,
                    down: false,
                    other: bit,
                }
            );
        }
        // A Ride also sends its paddles in another field: stepped over.
        assert_eq!(buttons("23 08 ff df ff ff 0f 1a 04 08 00 10 32"), UP);
    }

    #[test]
    fn other_messages() {
        let reply = decode(&bytes("52 69 64 65 4f 6e 01 03 aa bb"));
        assert_eq!(reply, Message::RideOn(vec![1, 3, 0xaa, 0xbb]));
        assert_eq!(decode(b"RideOn"), Message::RideOn(vec![]));
        assert_eq!(decode(&bytes("15")), Message::Idle);
        assert_eq!(decode(&bytes("19 08 59")), Message::Battery(89));
        assert_eq!(decode(&bytes("fe")), Message::Goodbye);
        assert_eq!(decode(&bytes("ff 05 00 ea 05")), Message::Stopped);
        assert_eq!(decode(&bytes("ff 05 00 fa 05")), Message::Stopped);
        assert_eq!(decode(&bytes("3c 08 00 12 02 0a 00")), Message::Other(0x3c));
        assert_eq!(decode(&bytes("07 08 01")), Message::Other(0x07));
    }

    #[test]
    fn nonsense_is_reported_not_fatal() {
        for hex in ["", "37", "37 08", "37 08 ff", "23 12 05 01", "23 0b", "19"] {
            let m = decode(&bytes(hex));
            assert!(matches!(m, Message::Malformed(_)), "{hex}: {m:?}");
        }
        // Any bytes at all decode to something, without panicking.
        let mut seed = 7u32;
        for len in 0..40 {
            let b: Vec<u8> = (0..len)
                .map(|_| {
                    seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    (seed >> 16) as u8
                })
                .collect();
            let _ = decode(&b);
            let mut with_kind = vec![0x23];
            with_kind.extend(&b);
            let _ = decode(&with_kind);
            with_kind[0] = 0x37;
            let _ = decode(&with_kind);
        }
    }

    #[test]
    fn a_press_shifts_once_however_often_it_is_reported() {
        let mut p = Presses::default();
        let seq = [
            ("37 08 01 10 01", (false, false)),
            ("37 08 00 10 01", (true, false)),
            ("37 08 00 10 01", (false, false)), // held: reported again
            ("37 08 00 10 01", (false, false)),
            ("37 08 01 10 01", (false, false)), // released
            ("37 08 00 10 01", (true, false)),  // pressed again
            ("37 08 01 10 00", (false, true)),  // + up, − down at once
            ("37 08 01 10 01", (false, false)),
        ];
        for (hex, want) in seq {
            assert_eq!(p.update(buttons(hex)), want, "{hex}");
        }
    }

    #[test]
    fn a_click_v2_battery() {
        assert_eq!(decode(&bytes("19 10 50")), Message::Battery(80));
    }

    #[test]
    fn the_left_pad() {
        for (map, bit) in [
            ("fe ff", PAD_LEFT),
            ("fd ff", PAD_UP),
            ("fb ff", PAD_RIGHT),
            ("f7 ff", PAD_DOWN),
        ] {
            assert_eq!(
                buttons(&format!("23 08 {map} ff ff 0f")).other,
                bit,
                "{map}"
            );
        }
    }

    #[test]
    fn a_locked_half_is_recognised() {
        assert_eq!(decode(&bytes("ff 03 12 34 56")), Message::Locked);
        assert_eq!(decode(&bytes("ff 05 00 ea 05")), Message::Stopped);
        assert_eq!(decode(&bytes("ff 04 00")), Message::Other(0xff));
        assert_eq!(UNLOCK_ACK, &[0xff, 0x04, 0x00]);
    }

    #[test]
    fn y_held_then_let_go() {
        let mut p = Presses::default();
        let up = "23 08 ff ff ff ff 0f";
        let y = "23 08 bf ff ff ff 0f";
        p.update(buttons(up));
        p.update(buttons(y));
        assert_eq!((p.other_pressed(), p.other_released()), (Y, 0));
        // Held: reported again, neither pressed nor let go.
        p.update(buttons(y));
        assert_eq!((p.other_pressed(), p.other_released()), (0, 0));
        p.update(buttons(up));
        assert_eq!((p.other_pressed(), p.other_released()), (0, Y));
    }

    #[test]
    fn devices_are_told_apart_by_their_advertisement() {
        assert_eq!(
            Kind::from_manufacturer_data(&[0x09, 0x12, 0x34]),
            Some(Kind::ClickV1)
        );
        assert_eq!(
            Kind::from_manufacturer_data(&[0x0A]),
            Some(Kind::ClickV2Right)
        );
        assert_eq!(
            Kind::from_manufacturer_data(&[0x0B]),
            Some(Kind::ClickV2Left)
        );
        assert_eq!(Kind::from_manufacturer_data(&[0x08]), Some(Kind::RideLeft));
        assert_eq!(Kind::from_manufacturer_data(&[0x01]), Some(Kind::Other(1)));
        assert_eq!(Kind::from_manufacturer_data(&[]), None);
        assert!(!Kind::Other(1).is_controller(), "a Zwift Hub trainer");
        assert!(Kind::ClickV1.is_controller());
    }

    #[test]
    fn handshakes() {
        assert_eq!(handshake(Some(Kind::ClickV1)), b"RideOn");
        assert_eq!(handshake(None), b"RideOn");
        assert_eq!(handshake(Some(Kind::ClickV2Left)), b"RideOn\x02\x03");
        assert_eq!(handshake(Some(Kind::ClickV2Right)), b"RideOn\x02\x03");
        assert_eq!(other_handshake(b"RideOn"), b"RideOn\x02\x03");
        assert_eq!(other_handshake(b"RideOn\x02\x03"), b"RideOn");
        assert_eq!(hex(b"Ri"), "52 69");
    }
}
