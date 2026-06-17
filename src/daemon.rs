//! The fawnd daemon: sole owner of the keyboard, serving clients over IPC.
//!
//! The HID handle is single-threaded, so exactly one **device thread** owns the
//! [`Controller`] and processes [`Job`]s from a channel. Everything else —
//! per-connection IPC handlers and the focus watcher — are *producers* that send
//! jobs and await a reply. This keeps device access serialized while allowing
//! concurrent clients and background automation.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use crate::config::Profile;
use crate::controller::Controller;
use crate::error::{Error, Result};
use crate::ipc::{self, Request, Response, Status};
use crate::rules;
use crate::watch;

/// A unit of work for the device thread: a request plus a reply channel.
pub struct Job {
    pub request: Request,
    pub reply: Sender<Response>,
}

/// Directory holding `<name>.toml` profiles (`$XDG_CONFIG_HOME/fawnd/profiles`).
pub fn profiles_dir() -> PathBuf {
    rules::config_dir().join("profiles")
}

/// Mutable daemon state owned by the device thread.
struct State {
    controller: Controller,
    active_profile: Option<String>,
}

/// Run the daemon: open the keyboard on a dedicated thread, start the optional
/// focus watcher, and serve IPC connections.
pub fn run() -> Result<()> {
    let path = ipc::socket_path();
    let _ = std::fs::remove_file(&path); // clear a stale socket (best effort)
    let listener = UnixListener::bind(&path)?;

    let (job_tx, job_rx) = mpsc::channel::<Job>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<String>>();

    // Device thread: owns the Controller and processes jobs serially.
    thread::Builder::new()
        .name("fawnd-device".into())
        .spawn(move || device_loop(job_rx, ready_tx))?;

    // Wait for the device to come up (or fail) before accepting clients.
    match ready_rx.recv() {
        Ok(Ok(desc)) => tracing::info!("device connected: {desc}"),
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(Error::Daemon("device thread exited during startup".into())),
    }

    // Optional focus watcher for automatic profile switching.
    let _watch = match watch::start(job_tx.clone()) {
        Ok(Some(handle)) => {
            tracing::info!("focus watcher active (auto profile switching)");
            Some(handle)
        }
        Ok(None) => {
            tracing::info!("no rules.toml — auto profile switching disabled");
            None
        }
        Err(e) => {
            tracing::warn!("focus watcher unavailable: {e}");
            None
        }
    };

    tracing::info!("listening on {}", path.display());
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let job_tx = job_tx.clone();
                thread::spawn(move || {
                    if let Err(e) = serve_connection(stream, job_tx) {
                        tracing::warn!("connection ended: {e}");
                    }
                });
            }
            Err(e) => tracing::warn!("accept failed: {e}"),
        }
    }
    Ok(())
}

fn device_loop(job_rx: mpsc::Receiver<Job>, ready_tx: Sender<Result<String>>) {
    let mut state = match Controller::open() {
        Ok(controller) => match controller.identity() {
            Ok(id) => {
                let _ = ready_tx.send(Ok(format!("{:?} fw {}", id.model, id.firmware_version)));
                State {
                    controller,
                    active_profile: None,
                }
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e));
                return;
            }
        },
        Err(e) => {
            let _ = ready_tx.send(Err(e));
            return;
        }
    };

    for job in job_rx {
        let response = handle(job.request, &mut state);
        let _ = job.reply.send(response);
    }
}

/// Run one client connection: read newline-delimited requests, forward each to
/// the device thread, and write back the response.
fn serve_connection(stream: UnixStream, job_tx: Sender<Job>) -> Result<()> {
    let reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => dispatch(request, &job_tx),
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

/// Send a request to the device thread and wait for its reply.
pub fn dispatch(request: Request, job_tx: &Sender<Job>) -> Response {
    let (reply_tx, reply_rx) = mpsc::channel();
    if job_tx.send(Job { request, reply: reply_tx }).is_err() {
        return Response::Error("device thread is gone".into());
    }
    reply_rx
        .recv()
        .unwrap_or_else(|_| Response::Error("device thread dropped the reply".into()))
}

/// Execute one request against the device. Device errors are returned as
/// [`Response::Error`] rather than dropping the connection.
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
        Request::ApplyActuation(values) => {
            for (i, mm) in values.iter().enumerate() {
                ctl.set_actuation_index(i, *mm);
            }
            into_response(ctl.flush_actuation(), &mut state.active_profile)
        }
        Request::SetRapidTrigger {
            rapid_trigger,
            turbo,
        } => into_response(
            ctl.set_rapid_trigger(rapid_trigger, turbo),
            &mut state.active_profile,
        ),
        Request::SetLed {
            direction,
            sequence,
            speed,
            brightness,
            rgb,
        } => into_response(
            ctl.set_led(direction, sequence, speed, brightness, rgb),
            &mut state.active_profile,
        ),
        Request::Reset => {
            let result = ctl.write_defaults();
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
