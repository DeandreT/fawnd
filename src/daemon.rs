//! The fawnd daemon: sole owner of the keyboard, serving clients over IPC.
//!
//! Because the HID handle is single-threaded, the daemon services connections
//! sequentially on one thread — the device is never shared. This also makes the
//! daemon the single owner of the request/response depth stream, avoiding
//! contention with a directly-connected GUI/CLI.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use crate::config::Profile;
use crate::controller::Controller;
use crate::error::{Error, Result};
use crate::ipc::{self, Request, Response, Status};

/// Directory holding `<name>.toml` profiles (`$XDG_CONFIG_HOME/fawnd/profiles`).
pub fn profiles_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("fawnd")
        .join("profiles")
}

/// Mutable daemon state shared across requests.
struct State {
    controller: Controller,
    active_profile: Option<String>,
}

/// Run the daemon: open the keyboard, bind the socket, and serve forever.
pub fn run() -> Result<()> {
    let path = ipc::socket_path();
    // Remove a stale socket from a previous run (best effort).
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    tracing::info!("listening on {}", path.display());

    let controller = Controller::open()?;
    let id = controller.identity()?;
    tracing::info!("device connected: {:?} fw {}", id.model, id.firmware_version);

    let mut state = State {
        controller,
        active_profile: None,
    };

    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                if let Err(e) = serve_connection(stream, &mut state) {
                    tracing::warn!("connection ended: {e}");
                }
            }
            Err(e) => tracing::warn!("accept failed: {e}"),
        }
    }
    Ok(())
}

fn serve_connection(stream: UnixStream, state: &mut State) -> Result<()> {
    let reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(req) => handle(req, state),
            Err(e) => Response::Error(format!("bad request: {e}")),
        };
        let mut out = serde_json::to_string(&response)
            .unwrap_or_else(|e| format!("{{\"Error\":\"serialize: {e}\"}}"));
        out.push('\n');
        writer.write_all(out.as_bytes())?;
        writer.flush()?;
    }
    Ok(())
}

/// Dispatch one request against the device. Device errors are returned to the
/// client as [`Response::Error`] rather than dropping the connection.
fn handle(req: Request, state: &mut State) -> Response {
    let ctl = &mut state.controller;
    match req {
        Request::Status => match ctl.identity() {
            Ok(id) => Response::Status(Status {
                model: format!("{:?}", id.model),
                firmware: id.firmware_version,
                rapid_trigger: id.rapid_trigger,
                turbo: id.turbo,
                active_profile: state.active_profile.clone(),
            }),
            Err(e) => Response::Error(e.to_string()),
        },
        Request::ListProfiles => match list_profiles() {
            Ok(names) => Response::Profiles(names),
            Err(e) => Response::Error(e.to_string()),
        },
        Request::ApplyProfile(name) => match load_profile(&name) {
            Ok(profile) => match profile.apply(ctl) {
                Ok(()) => {
                    tracing::info!("applied profile {name}");
                    state.active_profile = Some(name);
                    Response::Ok
                }
                Err(e) => Response::Error(e.to_string()),
            },
            Err(e) => Response::Error(e.to_string()),
        },
        Request::SetActuationAll(mm) => {
            ctl.set_actuation_all(mm);
            into_response(ctl.flush_actuation(), &mut state.active_profile)
        }
        Request::SetRapidTrigger {
            rapid_trigger,
            turbo,
        } => into_response(ctl.set_rapid_trigger(rapid_trigger, turbo), &mut state.active_profile),
        Request::Reset => {
            let result = ctl.write_defaults();
            if result.is_ok() {
                state.active_profile = None;
            }
            into_response(result, &mut state.active_profile)
        }
        Request::Depths => match ctl.poll_depths(Duration::from_millis(80)) {
            Ok(Some(frame)) => Response::Depths(frame.to_vec()),
            Ok(None) => Response::Error("no depth data".into()),
            Err(e) => Response::Error(e.to_string()),
        },
    }
}

/// A manual change clears the "active profile" label, since the live config no
/// longer matches a stored profile.
fn into_response(result: Result<()>, active: &mut Option<String>) -> Response {
    match result {
        Ok(()) => {
            *active = None;
            Response::Ok
        }
        Err(e) => Response::Error(e.to_string()),
    }
}

fn list_profiles() -> Result<Vec<String>> {
    let dir = profiles_dir();
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
    }
    names.sort();
    Ok(names)
}

fn load_profile(name: &str) -> Result<Profile> {
    let path = profiles_dir().join(format!("{name}.toml"));
    if !path.exists() {
        return Err(Error::Config(format!("no profile named '{name}'")));
    }
    Profile::load(&path)
}
