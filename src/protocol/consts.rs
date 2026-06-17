//! Wire-level constants for the DrunkDeer HID protocol.
//!
//! Reverse-engineered and verified against an A75 ANSI (`352d:2383`).

/// DrunkDeer USB vendor ID.
pub const VENDOR_ID: u16 = 0x352D;

/// Known DrunkDeer product IDs. The A75 ANSI enumerates as `0x2383`.
pub const PRODUCT_IDS: &[u16] = &[0x2382, 0x2383, 0x2384, 0x2386];

/// The configuration endpoint lives on a vendor-defined usage page, *not* the
/// boot-keyboard interface. Devices expose multiple HID interfaces; we must
/// select this one to talk the config protocol.
pub const USAGE_PAGE: u16 = 0xFF00;
pub const USAGE: u16 = 0x00;

/// First byte of every outbound 64-byte HID report (the HID report ID).
pub const REPORT_ID: u8 = 0x04;

/// Total HID report length, including the leading report ID byte.
pub const REPORT_LEN: usize = 64;
/// Payload length after the report ID byte.
pub const PAYLOAD_LEN: usize = REPORT_LEN - 1; // 63

/// Keys addressed per "row" packet. The A75 is split into 59 + 59 + 8 = 126
/// logical key slots across three row packets (row 2 only carries 8 keys).
pub const KEYS_PER_ROW: usize = 59;
/// Number of key slots in the final (short) row.
pub const LAST_ROW_KEYS: usize = 8;
/// Total addressable key slots across all rows (`KEYBOARD_LAYOUT.len()`).
pub const TOTAL_KEYS: usize = 126;

/// Default actuation byte = 2.0 mm (encoded as `mm * 10`).
pub const DEFAULT_ACTUATION: u8 = 0x14;

/// Actuation range, in encoded byte units (`mm * 10`).
pub const ACTUATION_MIN: u8 = 0x02; // 0.2 mm
pub const ACTUATION_MAX: u8 = 0x26; // 3.8 mm

/// Command byte (payload\[0]) for each packet family.
pub mod cmd {
    /// Identity / handshake query and reply.
    pub const IDENTITY: u8 = 0xA0;
    /// LED mode selection.
    pub const LED_MODE: u8 = 0xAE;
    /// Per-key modify (actuation / downstroke / upstroke / key-tracking toggle).
    pub const MODIFY_KEY: u8 = 0xB6;
    /// Global rapid-trigger + turbo (snap-tap) toggle.
    pub const RAPID_TRIGGER: u8 = 0xB5;
    /// Live key-depth tracking stream (inbound reports).
    pub const KEY_TRACKING: u8 = 0xB7;
}

/// Sub-command byte (payload\[1]) for `MODIFY_KEY` packets.
pub mod modify {
    pub const ACTUATION: u8 = 0x01;
    pub const KEY_TRACKING: u8 = 0x03;
    pub const DOWNSTROKE: u8 = 0x04;
    pub const UPSTROKE: u8 = 0x05;
}

/// LED lighting effect sequences (payload value for `LED_MODE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LedSequence {
    Off = 0x00,
    Always = 0x02,
    Spectrum = 0x03,
    Breath = 0x04,
    Press = 0x05,
    Stars = 0x06,
    Wave = 0x07,
    Surf = 0x08,
    SurfDown = 0x09,
    Ripple = 0x0A,
    Fish = 0x0B,
    Fountain = 0x0C,
    Traffic = 0x0D,
    Snake = 0x0E,
    SurfRepeat = 0x0F,
    SurfCross = 0x10,
    LaserKey = 0x11,
    FountainRandom = 0x12,
    Custom = 0x13,
}
