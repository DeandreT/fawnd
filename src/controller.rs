//! High-level keyboard control built on top of [`Device`] and the protocol.
//!
//! The controller owns the desired per-key state (actuation, downstroke,
//! upstroke) and knows how to flush it to the device as row packets.

use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::device::Device;
use crate::error::{Error, Result};
use crate::protocol::consts::{
    DEFAULT_ACTUATION, KEYS_PER_ROW, LAST_ROW_KEYS, LedSequence, TOTAL_KEYS, cmd,
};
use crate::protocol::layout;
use crate::protocol::packet::{self, RowKind};
use crate::protocol::Identity;

/// The device tolerates only so much back-to-back traffic; pace writes.
const WRITE_PACING: Duration = Duration::from_millis(20);

/// Approximate maximum travel reading from the key-depth stream (≈4.0 mm).
pub const DEPTH_MAX_RAW: u8 = 40;

/// Per-key configuration, mirrored from what we last pushed to the device.
///
/// The device has no read-back for per-key values, so this is the source of
/// truth on our side.
pub struct Controller {
    device: Device,
    actuation: [u8; TOTAL_KEYS],
    downstroke: [u8; TOTAL_KEYS],
    upstroke: [u8; TOTAL_KEYS],
}

impl Controller {
    /// Open the keyboard and seed in-memory state with defaults.
    pub fn open() -> Result<Controller> {
        Ok(Controller {
            device: Device::open()?,
            actuation: [DEFAULT_ACTUATION; TOTAL_KEYS],
            downstroke: [0; TOTAL_KEYS],
            upstroke: [0; TOTAL_KEYS],
        })
    }

    /// Query keyboard identity (model, firmware, current toggle state).
    pub fn identity(&self) -> Result<Identity> {
        self.device.identity()
    }

    // ── per-key mutation (in memory; call `flush_*` to push) ──────────────

    /// Set the actuation point (mm) for keys named in `names`.
    pub fn set_actuation(&mut self, names: &[&str], mm: f32) -> Result<()> {
        let value = packet::mm_to_byte(mm);
        for name in names {
            let idx = layout::index_of(name).ok_or_else(|| Error::UnknownKey(name.to_string()))?;
            self.actuation[idx] = value;
        }
        Ok(())
    }

    /// Set the actuation point (mm) for every key.
    pub fn set_actuation_all(&mut self, mm: f32) {
        self.actuation.fill(packet::mm_to_byte(mm));
    }

    /// Set the actuation point (mm) for a single key slot index.
    pub fn set_actuation_index(&mut self, index: usize, mm: f32) {
        if index < TOTAL_KEYS {
            self.actuation[index] = packet::mm_to_byte(mm);
        }
    }

    /// Read the current in-memory actuation point (mm) for a key slot index.
    pub fn actuation_mm(&self, index: usize) -> f32 {
        packet::byte_to_mm(self.actuation.get(index).copied().unwrap_or(0))
    }

    // ── flush to device ──────────────────────────────────────────────────

    /// Push the in-memory actuation map to the device.
    pub fn flush_actuation(&self) -> Result<()> {
        self.flush_rows(&self.actuation, RowKind::Actuation)
    }

    /// Push the in-memory rapid-trigger down/up sensitivity maps to the device.
    pub fn flush_rapid_trigger_curve(&self) -> Result<()> {
        self.flush_rows(&self.downstroke, RowKind::Downstroke)?;
        self.flush_rows(&self.upstroke, RowKind::Upstroke)
    }

    fn flush_rows(&self, values: &[u8; TOTAL_KEYS], kind: RowKind) -> Result<()> {
        for (row, chunk) in values.chunks(KEYS_PER_ROW).enumerate() {
            self.device
                .write(&packet::modify_row(row as u8, chunk, kind))?;
            sleep(WRITE_PACING);
        }
        Ok(())
    }

    // ── global toggles ───────────────────────────────────────────────────

    /// Enable/disable global rapid trigger and turbo (snap-tap).
    pub fn set_rapid_trigger(&self, rapid_trigger: bool, turbo: bool) -> Result<()> {
        self.device
            .write(&packet::rapid_trigger_turbo(rapid_trigger, turbo))
    }

    /// Select an LED lighting mode.
    pub fn set_led(
        &self,
        direction: u8,
        sequence: LedSequence,
        speed: u8,
        brightness: u8,
        rgb: u8,
    ) -> Result<()> {
        self.device
            .write(&packet::led_mode(direction, sequence, speed, brightness, rgb, false))
    }

    /// Toggle the live key-depth tracking stream.
    pub fn set_key_tracking(&self, enabled: bool) -> Result<()> {
        self.device.write(&packet::key_tracking(enabled))
    }

    /// Request and collect one full key-depth frame.
    ///
    /// The stream is request/response: one request (`0xB6 03 01`) yields three
    /// inbound `0xB7` packets (one per protocol row). Each value is the key's
    /// current travel in ~0.1 mm units (0 = released, ~[`DEPTH_MAX_RAW`] = bottomed
    /// out). Returns the assembled per-slot frame, or `None` if no packet arrived
    /// within `timeout`.
    pub fn poll_depths(&self, timeout: Duration) -> Result<Option<[u8; TOTAL_KEYS]>> {
        self.device.write(&packet::key_tracking(true))?;

        let mut frame = [0u8; TOTAL_KEYS];
        let mut seen = [false; 3];
        let deadline = Instant::now() + timeout;

        while seen.contains(&false) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let Some(data) = self.device.read(remaining.min(WRITE_PACING))? else {
                continue;
            };
            if data.first() != Some(&cmd::KEY_TRACKING) || data.len() < 5 {
                continue;
            }
            let row = data[3] as usize;
            if row >= 3 {
                continue;
            }
            let base = row * KEYS_PER_ROW;
            let count = if row == 2 { LAST_ROW_KEYS } else { KEYS_PER_ROW };
            for (i, &v) in data[4..].iter().take(count).enumerate() {
                if base + i < TOTAL_KEYS {
                    frame[base + i] = v;
                }
            }
            seen[row] = true;
        }

        Ok(seen.contains(&true).then_some(frame))
    }

    /// Restore factory-ish defaults: 2.0 mm actuation, rapid trigger off,
    /// lights off.
    pub fn write_defaults(&mut self) -> Result<()> {
        self.actuation.fill(DEFAULT_ACTUATION);
        self.downstroke.fill(0);
        self.upstroke.fill(0);
        self.set_led(0, LedSequence::Off, 5, 9, 0xff)?;
        self.set_rapid_trigger(false, false)?;
        self.flush_actuation()?;
        self.flush_rapid_trigger_curve()
    }
}
