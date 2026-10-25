//! Zwift controller BLE protocol: UUIDs, handshake and message decoding.
//!
//! Everything here comes from community reverse engineering (makinolo.com,
//! BikeControl, qdomyos-zwift) and is confirmed against real hardware by the
//! probe binary.

use uuid::Uuid;

/// Bluetooth SIG company identifier for Zwift, used in manufacturer data.
pub const ZWIFT_COMPANY_ID: u16 = 0x094A;

/// Legacy Zwift controller service.
pub const SERVICE_LEGACY: Uuid = Uuid::from_u128(0x00000001_19ca_4651_86e5_fa29dcdd09d1);
/// 16-bit service used by newer firmware (0xFC82), expanded to 128 bits.
pub const SERVICE_FC82: Uuid = Uuid::from_u128(0x0000fc82_0000_1000_8000_00805f9b34fb);

/// Notify characteristic carrying button and status events.
pub const CHAR_ASYNC: Uuid = Uuid::from_u128(0x00000002_19ca_4651_86e5_fa29dcdd09d1);
/// Write characteristic for commands (the handshake goes here).
pub const CHAR_SYNC_RX: Uuid = Uuid::from_u128(0x00000003_19ca_4651_86e5_fa29dcdd09d1);
/// Indicate characteristic carrying replies to commands.
pub const CHAR_SYNC_TX: Uuid = Uuid::from_u128(0x00000004_19ca_4651_86e5_fa29dcdd09d1);

/// Handshake written to SYNC_RX on connect, and periodically as a keepalive.
pub const RIDE_ON: &[u8] = b"RideOn";
/// Reportedly keeps an already unlocked left pod unlocked across reconnects.
pub const UNLOCK_ACK: &[u8] = &[0xff, 0x04, 0x00];

/// Message type of a Zwift Ride / Click v2 keypad status (button bitmap).
pub const MSG_RIDE_KEYPAD: u8 = 0x23;
/// Message type of an original Click (v1) keypad status.
pub const MSG_CLICK_KEYPAD: u8 = 0x37;
/// Replies on SYNC_TX starting with this byte are a crypto challenge (locked pod).
pub const MSG_CHALLENGE: u8 = 0xff;

/// Button bits of the Ride / Click v2 bitmap. A bit cleared to 0 means pressed.
pub const RIDE_BUTTONS: &[(u32, &str)] = &[
    (0x00001, "LEFT"),
    (0x00002, "UP"),
    (0x00004, "RIGHT"),
    (0x00008, "DOWN"),
    (0x00010, "A"),
    (0x00020, "B"),
    (0x00040, "Y"),
    (0x00100, "Z"),
    (0x00200, "SHIFT_UP_L"),
    (0x00400, "SHIFT_DN_L"),
    (0x00800, "POWERUP_L"),
    (0x01000, "ONOFF_L"),
    (0x02000, "SHIFT_UP_R"),
    (0x04000, "SHIFT_DN_R"),
    (0x10000, "POWERUP_R"),
    (0x20000, "ONOFF_R"),
];

/// Returns true if advertised data looks like a Zwift controller.
pub fn is_zwift_controller(
    name: Option<&str>,
    services: &[Uuid],
    manufacturer_ids: &[u16],
) -> bool {
    name.is_some_and(|n| n.to_ascii_lowercase().contains("zwift"))
        || services
            .iter()
            .any(|s| *s == SERVICE_LEGACY || *s == SERVICE_FC82)
        || manufacturer_ids.contains(&ZWIFT_COMPANY_ID)
}

/// What a reply on SYNC_TX means for the handshake.
#[derive(Debug, PartialEq, Eq)]
pub enum HandshakeReply {
    /// The pod echoed `RideOn`: it will send button events.
    RideOn,
    /// The pod sent a crypto challenge: it needs a recent Zwift unlock.
    Locked,
    Other,
}

pub fn classify_sync_reply(data: &[u8]) -> HandshakeReply {
    if data.starts_with(RIDE_ON) {
        HandshakeReply::RideOn
    } else if data.first() == Some(&MSG_CHALLENGE) {
        HandshakeReply::Locked
    } else {
        HandshakeReply::Other
    }
}

/// A decoded button message.
#[derive(Debug, PartialEq, Eq)]
pub enum Buttons {
    /// Ride / Click v2: names of the buttons currently held down.
    Ride {
        bitmap: u32,
        pressed: Vec<&'static str>,
    },
    /// Click v1: state of the + and - buttons.
    ClickV1 { plus: bool, minus: bool },
}

/// Decodes a message from the async characteristic, if it is a button message.
pub fn decode_buttons(data: &[u8]) -> Option<Buttons> {
    let (&kind, payload) = data.split_first()?;
    match kind {
        MSG_RIDE_KEYPAD => {
            let bitmap = varint_field(payload, 1)? as u32;
            Some(Buttons::Ride {
                bitmap,
                pressed: pressed_ride_buttons(bitmap),
            })
        }
        MSG_CLICK_KEYPAD => {
            // Fields hold 0 for pressed, 1 for released.
            let plus = varint_field(payload, 1) == Some(0);
            let minus = varint_field(payload, 2) == Some(0);
            Some(Buttons::ClickV1 { plus, minus })
        }
        _ => None,
    }
}

pub fn pressed_ride_buttons(bitmap: u32) -> Vec<&'static str> {
    RIDE_BUTTONS
        .iter()
        .filter(|(mask, _)| bitmap & mask == 0)
        .map(|(_, name)| *name)
        .collect()
}

/// Finds the first varint field with the given number in a protobuf message.
fn varint_field(mut buf: &[u8], field: u64) -> Option<u64> {
    while !buf.is_empty() {
        let key = read_varint(&mut buf)?;
        let (number, wire_type) = (key >> 3, key & 0x7);
        match wire_type {
            0 => {
                let value = read_varint(&mut buf)?;
                if number == field {
                    return Some(value);
                }
            }
            1 => buf = buf.get(8..)?,
            2 => {
                let len = read_varint(&mut buf)? as usize;
                buf = buf.get(len..)?;
            }
            5 => buf = buf.get(4..)?,
            _ => return None,
        }
    }
    None
}

fn read_varint(buf: &mut &[u8]) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let (&byte, rest) = buf.split_first()?;
        *buf = rest;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_varint(mut v: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }

    fn ride_frame(bitmap: u32) -> Vec<u8> {
        let mut frame = vec![MSG_RIDE_KEYPAD, 0x08];
        frame.extend(encode_varint(bitmap.into()));
        // Analog group (field 2) that the decoder must skip.
        frame.extend([0x12, 0x02, 0x08, 0x00]);
        frame
    }

    #[test]
    fn idle_ride_bitmap_has_no_presses() {
        let msg = decode_buttons(&ride_frame(0xffff_ffff)).unwrap();
        assert_eq!(
            msg,
            Buttons::Ride {
                bitmap: 0xffff_ffff,
                pressed: vec![]
            }
        );
    }

    #[test]
    fn cleared_bits_are_pressed() {
        let bitmap: u32 = !0x20 & !0x2000;
        let Some(Buttons::Ride { pressed, .. }) = decode_buttons(&ride_frame(bitmap)) else {
            panic!("not a ride frame");
        };
        assert_eq!(pressed, vec!["B", "SHIFT_UP_R"]);
    }

    #[test]
    fn click_v1_plus_pressed() {
        let frame = [MSG_CLICK_KEYPAD, 0x08, 0x00, 0x10, 0x01];
        assert_eq!(
            decode_buttons(&frame),
            Some(Buttons::ClickV1 {
                plus: true,
                minus: false
            })
        );
    }

    #[test]
    fn other_messages_are_ignored() {
        assert_eq!(decode_buttons(&[0x19, 0x08, 0x01]), None);
        assert_eq!(decode_buttons(&[]), None);
    }

    #[test]
    fn truncated_frame_does_not_panic() {
        assert_eq!(decode_buttons(&[MSG_RIDE_KEYPAD, 0x08, 0xff]), None);
    }

    #[test]
    fn handshake_replies() {
        assert_eq!(
            classify_sync_reply(b"RideOn\x01\x02"),
            HandshakeReply::RideOn
        );
        assert_eq!(classify_sync_reply(&[0xff, 0x01]), HandshakeReply::Locked);
        assert_eq!(classify_sync_reply(&[0x3c]), HandshakeReply::Other);
    }

    #[test]
    fn detects_controller_by_any_signal() {
        assert!(is_zwift_controller(Some("Zwift Click"), &[], &[]));
        assert!(is_zwift_controller(None, &[SERVICE_FC82], &[]));
        assert!(is_zwift_controller(None, &[], &[ZWIFT_COMPANY_ID]));
        assert!(!is_zwift_controller(Some("Heart Rate"), &[], &[0x0001]));
    }
}
