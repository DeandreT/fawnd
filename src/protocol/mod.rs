//! DrunkDeer HID wire protocol: constants, key layout, and packet codecs.
//!
//! This module is transport-agnostic — it knows how to *build* and *parse*
//! bytes but never touches a device. See [`crate::device`] for I/O.

pub mod consts;
pub mod layout;
pub mod packet;

pub use consts::{LedSequence, TOTAL_KEYS};
pub use packet::{Payload, RowKind, byte_to_mm, mm_to_byte};

/// The keyboard model, as reported in the identity handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    A75,
    A75Pro,
    G60,
    G65,
    G75,
    Unknown,
}

impl Model {
    /// Map the 3-byte model signature from an identity reply to a [`Model`].
    pub fn from_signature(sig: [u8; 3]) -> Model {
        match sig {
            [0x0b, 0x01, 0x01] | [0x0b, 0x04, 0x01] => Model::A75,
            [0x0b, 0x04, 0x03] => Model::A75Pro,
            [0x0b, 0x03, 0x01] => Model::G60,
            [0x0f, 0x01, 0x01] | [0x0b, 0x02, 0x01] => Model::G65,
            [0x0b, 0x04, 0x05] => Model::G75,
            _ => Model::Unknown,
        }
    }
}

/// Parsed identity-handshake reply.
#[derive(Debug, Clone)]
pub struct Identity {
    pub model: Model,
    pub firmware_version: String,
    pub rapid_trigger: bool,
    pub turbo: bool,
}

impl Identity {
    /// Parse an identity reply. `data` is the report payload *after* the report
    /// ID byte (i.e. starting at the command byte). Returns `None` if the
    /// command byte is not [`consts::cmd::IDENTITY`], the payload is too short,
    /// or it's a zero-signature stub the device emits while busy (which would
    /// otherwise decode to a bogus `Unknown`/`0.00` identity).
    pub fn parse(data: &[u8]) -> Option<Identity> {
        // Layout (payload index): [0]=cmd 0xA0, [1]=0x02, [2]=0x00, [3]=0x01,
        // [4..7]=model signature, [7..9]=firmware (LE u16),
        // [15]=turbo, [16]=rapid trigger.
        if data.first() != Some(&consts::cmd::IDENTITY) || data.len() < 17 {
            return None;
        }
        let sig = [data[4], data[5], data[6]];
        if sig == [0, 0, 0] {
            return None; // stub/echo reply, not a real identity
        }
        let version = u16::from_le_bytes([data[7], data[8]]);
        Some(Identity {
            model: Model::from_signature(sig),
            firmware_version: format!("0.0{version}"),
            turbo: data[15] != 0,
            rapid_trigger: data[16] != 0,
        })
    }
}
