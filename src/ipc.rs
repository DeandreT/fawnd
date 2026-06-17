//! IPC between the daemon (which owns the keyboard) and clients (CLI / GUI).
//!
//! The wire format is newline-delimited JSON over a Unix domain socket: one
//! [`Request`] per line in, one [`Response`] per line out.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Path to the daemon's control socket.
///
/// Prefers `$XDG_RUNTIME_DIR/fawnd.sock` (per-user, cleaned up on logout),
/// falling back to the system temp dir.
pub fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("fawnd.sock")
}

/// A request from a client to the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    /// Keyboard model / firmware / toggle state and active profile.
    Status,
    /// Names of profiles available in the profile store.
    ListProfiles,
    /// Apply a named profile from the store.
    ApplyProfile(String),
    /// Set the actuation point (mm) for every key.
    SetActuationAll(f32),
    /// Toggle global rapid trigger / turbo.
    SetRapidTrigger { rapid_trigger: bool, turbo: bool },
    /// Restore factory defaults.
    Reset,
    /// Sample one live key-depth frame.
    Depths,
}

/// The daemon's reply to a [`Request`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    Ok,
    Status(Status),
    Profiles(Vec<String>),
    Depths(Vec<u8>),
    Error(String),
}

/// Keyboard status snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub model: String,
    pub firmware: String,
    pub rapid_trigger: bool,
    pub turbo: bool,
    pub active_profile: Option<String>,
}

/// A connected client handle.
pub struct Client {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Client {
    /// Connect to the running daemon, or error if it isn't running.
    pub fn connect() -> Result<Client> {
        let path = socket_path();
        let writer = UnixStream::connect(&path).map_err(|e| {
            Error::Daemon(format!(
                "not running (no socket at {}): {e}",
                path.display()
            ))
        })?;
        let reader = BufReader::new(writer.try_clone()?);
        Ok(Client { writer, reader })
    }

    /// Send one request and read one response.
    pub fn request(&mut self, req: &Request) -> Result<Response> {
        let mut line = serde_json::to_string(req).map_err(|e| Error::Daemon(e.to_string()))?;
        line.push('\n');
        self.writer.write_all(line.as_bytes())?;
        self.writer.flush()?;

        let mut resp = String::new();
        if self.reader.read_line(&mut resp)? == 0 {
            return Err(Error::Daemon("daemon closed the connection".into()));
        }
        serde_json::from_str(&resp).map_err(|e| Error::Daemon(e.to_string()))
    }
}
