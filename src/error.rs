//! Crate error type.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[cfg(not(target_arch = "wasm32"))]
    #[error("HID error: {0}")]
    Hid(#[from] hidapi::HidError),

    #[error(
        "no DrunkDeer keyboard found (looked for vendor {:#06x})",
        crate::protocol::consts::VENDOR_ID
    )]
    DeviceNotFound,

    #[error("device did not respond to identity handshake")]
    NoResponse,

    #[error("unknown key name: {0}")]
    UnknownKey(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("daemon: {0}")]
    Daemon(String),
}

pub type Result<T> = std::result::Result<T, Error>;
