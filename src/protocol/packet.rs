//! Builders for outbound config packets and parsers for inbound reports.
//!
//! Every builder returns a [`PAYLOAD_LEN`]-byte payload (the bytes *after* the
//! HID report ID). The transport layer is responsible for prepending
//! [`REPORT_ID`] and padding to [`REPORT_LEN`].

use super::consts::{
    ACTUATION_MAX, ACTUATION_MIN, DEFAULT_ACTUATION, KEYS_PER_ROW, LAST_ROW_KEYS, LedSequence,
    PAYLOAD_LEN, cmd, modify,
};

pub type Payload = [u8; PAYLOAD_LEN];

fn payload(prefix: &[u8]) -> Payload {
    let mut p = [0u8; PAYLOAD_LEN];
    p[..prefix.len()].copy_from_slice(prefix);
    p
}

/// Convert an actuation/travel distance in millimetres to the device byte
/// encoding (`mm * 10`), clamped to the supported range.
pub fn mm_to_byte(mm: f32) -> u8 {
    let raw = (mm * 10.0).round() as i32;
    raw.clamp(ACTUATION_MIN as i32, ACTUATION_MAX as i32) as u8
}

/// Convert a device actuation byte back to millimetres.
pub fn byte_to_mm(b: u8) -> f32 {
    b as f32 / 10.0
}

/// Handshake: ask the keyboard to report its identity.
pub fn identity() -> Payload {
    payload(&[cmd::IDENTITY, 0x02])
}

/// Select an LED lighting mode.
pub fn led_mode(
    direction: u8,
    sequence: LedSequence,
    speed: u8,
    brightness: u8,
    rgb: u8,
    turbo: bool,
) -> Payload {
    payload(&[
        cmd::LED_MODE,
        0x01,
        if turbo { 0x01 } else { 0x00 },
        direction,
        sequence as u8,
        speed,
        brightness,
        rgb,
    ])
}

/// Toggle global rapid trigger and turbo (snap-tap).
pub fn rapid_trigger_turbo(rapid_trigger: bool, turbo: bool) -> Payload {
    payload(&[
        cmd::RAPID_TRIGGER,
        0x00,
        0x1E,
        0x01,
        0x00,
        0x00,
        0x01,
        turbo as u8,
        rapid_trigger as u8,
    ])
}

/// Toggle the live key-depth tracking stream.
pub fn key_tracking(enabled: bool) -> Payload {
    payload(&[cmd::MODIFY_KEY, modify::KEY_TRACKING, enabled as u8])
}

/// Which per-key value a modify-row packet carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// Actuation point (default 2.0 mm).
    Actuation,
    /// Rapid-trigger downstroke sensitivity (default 0.0 mm).
    Downstroke,
    /// Rapid-trigger upstroke sensitivity (default 0.0 mm).
    Upstroke,
}

impl RowKind {
    fn sub_command(self) -> u8 {
        match self {
            RowKind::Actuation => modify::ACTUATION,
            RowKind::Downstroke => modify::DOWNSTROKE,
            RowKind::Upstroke => modify::UPSTROKE,
        }
    }

    fn default_value(self) -> u8 {
        match self {
            RowKind::Actuation => DEFAULT_ACTUATION,
            RowKind::Downstroke | RowKind::Upstroke => 0x00,
        }
    }
}

/// Build a per-key modify packet for one protocol row (0, 1, or 2).
///
/// `keys` are placed starting at row offset 0; any remaining slots in the row
/// are padded with the kind's default value. Row 2 only carries
/// [`LAST_ROW_KEYS`] slots.
pub fn modify_row(row: u8, keys: &[u8], kind: RowKind) -> Payload {
    let capacity = if row == 2 { LAST_ROW_KEYS } else { KEYS_PER_ROW };
    let mut p = payload(&[cmd::MODIFY_KEY, kind.sub_command(), 0x00, row]);

    let n = keys.len().min(capacity);
    for i in 0..capacity {
        p[4 + i] = if i < n { keys[i] } else { kind.default_value() };
    }
    p
}
