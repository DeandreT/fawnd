//! `fawnd` — a userspace driver for the DrunkDeer A75 Hall-effect keyboard.
//!
//! Module layout:
//! - [`protocol`]: transport-agnostic wire format (constants, layout, codecs).
//! - [`device`]: HID discovery and raw report I/O.
//! - [`controller`]: high-level per-key + global configuration.
//! - [`config`]: declarative on-disk profiles.

pub mod config;
#[cfg(not(target_arch = "wasm32"))]
pub mod controller;
#[cfg(not(target_arch = "wasm32"))]
pub mod daemon;
#[cfg(not(target_arch = "wasm32"))]
pub mod device;
pub mod error;
pub mod gui;
pub mod ipc;
pub mod protocol;
#[cfg(not(target_arch = "wasm32"))]
pub mod rules;
#[cfg(not(target_arch = "wasm32"))]
pub mod watch;

#[cfg(not(target_arch = "wasm32"))]
pub use controller::Controller;
pub use error::{Error, Result};
