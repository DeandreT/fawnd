//! `fawnd` — a userspace driver for the DrunkDeer A75 Hall-effect keyboard.
//!
//! Module layout:
//! - [`protocol`]: transport-agnostic wire format (constants, layout, codecs).
//! - [`device`]: HID discovery and raw report I/O.
//! - [`controller`]: high-level per-key + global configuration.
//! - [`config`]: declarative on-disk profiles.

pub mod config;
pub mod controller;
pub mod device;
pub mod error;
pub mod gui;
pub mod protocol;

pub use controller::Controller;
pub use error::{Error, Result};
