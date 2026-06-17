//! Shared worker API: the messages exchanged between the egui UI and its backend
//! worker. The native ([`super::worker`]) and browser (`worker_web`) backends
//! differ in transport but speak this same command/event vocabulary.

use crate::ipc::Status;
use crate::protocol::consts::{LedSequence, TOTAL_KEYS};

/// A request from the UI to the worker.
pub enum Command {
    /// (Re)connect to the keyboard and fetch status.
    Reconnect,
    /// Re-query status.
    Refresh,
    /// Push a full per-key actuation map (mm), length [`TOTAL_KEYS`].
    ApplyActuation(Vec<f32>),
    /// Toggle global rapid trigger / turbo.
    SetRapidTrigger { rapid_trigger: bool, turbo: bool },
    /// Set lighting.
    SetLed {
        direction: u8,
        sequence: LedSequence,
        speed: u8,
        brightness: u8,
        rgb: u8,
    },
    /// Restore factory defaults.
    Reset,
    /// Start/stop streaming live key-depth frames.
    SetLiveDepth(bool),
}

/// A notification from the worker back to the UI.
pub enum Event {
    Connected(Status),
    Disconnected(String),
    Status(String),
    Error(String),
    /// One live key-depth frame (per-slot travel, ~0.1 mm units).
    Depths(Box<[u8; TOTAL_KEYS]>),
}
