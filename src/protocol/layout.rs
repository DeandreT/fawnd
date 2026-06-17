//! Physical key layout and name <-> index mapping.
//!
//! The device addresses keys as a flat array of [`TOTAL_KEYS`] slots laid out in
//! visual reading order (6 matrix rows of 21 columns). Empty strings are matrix
//! gaps that have no physical key.
//!
//! Note: the "row" in the modify-row packet is a *protocol* slice of this flat
//! array — row 0 is slots `0..59`, row 1 is `59..118`, row 2 is `118..126` — and
//! does **not** line up with the visual matrix rows below.

use super::consts::{KEYS_PER_ROW, TOTAL_KEYS};

/// Flat key map, indexed by device key slot. Empty strings are matrix gaps.
#[rustfmt::skip]
pub const KEYBOARD_LAYOUT: [&str; TOTAL_KEYS] = [
    "ESC", "", "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "KP7", "KP8", "KP9", "", "", "", "",
    "TILDE", "1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "MINUS", "PLUS", "BACK", "KP4", "KP5", "KP6", "", "", "", "",
    "TAB", "Q", "W", "E", "R", "T", "Y", "U", "I", "O", "P", "BRKTS_L", "BRKTS_R", "SLASH_K29", "KP1", "KP2", "KP3", "", "", "", "",
    "CAPS", "A", "S", "D", "F", "G", "H", "J", "K", "L", "COLON", "QOTATN", "", "RETURN", "", "KP0", "KP_DEL", "", "", "", "",
    "SHF_L", "EUR_K45", "Z", "X", "C", "V", "B", "N", "M", "COMMA", "PERIOD", "SLASH", "", "SHF_R", "ARR_UP", "", "NUMS", "", "", "", "",
    "CTRL_L", "WIN_L", "ALT_L", "", "", "", "SPACE", "", "", "", "ALT_R", "FN1", "APP", "ARR_L", "ARR_DW", "ARR_R", "CTRL_R", "", "", "", "",
];

/// W, A, S, D slot indices.
pub const WASD_KEYS: &[usize] = &[44, 64, 65, 66];

/// Resolve a key name (case-insensitive) to its device slot index.
pub fn index_of(name: &str) -> Option<usize> {
    KEYBOARD_LAYOUT
        .iter()
        .position(|k| !k.is_empty() && k.eq_ignore_ascii_case(name))
}

/// Resolve a device slot index back to its key name.
pub fn name_of(index: usize) -> Option<&'static str> {
    KEYBOARD_LAYOUT.get(index).copied().filter(|k| !k.is_empty())
}

/// Which row packet (0, 1, or 2) a given slot index belongs to.
pub fn row_of(index: usize) -> usize {
    index / KEYS_PER_ROW
}
