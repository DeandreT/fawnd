//! Background worker that talks to the fawnd daemon over IPC.
//!
//! The GUI is a daemon client: the daemon owns the keyboard, and this worker
//! holds one socket connection, translating UI [`Command`]s into [`Request`]s
//! and daemon replies into [`Event`]s. All requests are serialized on this one
//! thread, off the UI thread.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use crate::ipc::{self, Request, Response};
use crate::protocol::consts::TOTAL_KEYS;

pub use super::api::{Command, Event};

/// UI-side handle to the worker.
pub struct Worker {
    tx: Sender<Command>,
    rx: Receiver<Event>,
}

impl Worker {
    /// Spawn the worker thread. `repaint` wakes the egui event loop whenever a
    /// new [`Event`] is emitted.
    pub fn spawn(repaint: egui::Context) -> Worker {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
        let (evt_tx, evt_rx) = mpsc::channel::<Event>();

        thread::Builder::new()
            .name("fawnd-ipc".into())
            .spawn(move || run(cmd_rx, evt_tx, repaint))
            .expect("spawn ipc worker");

        Worker {
            tx: cmd_tx,
            rx: evt_rx,
        }
    }

    /// Queue a command for the worker.
    pub fn send(&self, cmd: Command) {
        if self.tx.send(cmd).is_err() {
            tracing::warn!("ipc worker is gone");
        }
    }

    /// Drain all pending events without blocking.
    pub fn poll(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }
}

fn run(cmd_rx: Receiver<Command>, evt_tx: Sender<Event>, repaint: egui::Context) {
    use std::sync::mpsc::{RecvTimeoutError, TryRecvError};

    let emit = |evt: Event| {
        let _ = evt_tx.send(evt);
        repaint.request_repaint();
    };

    let mut client: Option<ipc::Client> = None;
    let mut live = false;

    connect(&mut client, &emit);

    loop {
        // Drain queued commands.
        loop {
            match cmd_rx.try_recv() {
                Ok(cmd) => {
                    if dispatch(cmd, &mut client, &mut live, &emit) {
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        // While live + connected, poll depth frames; otherwise idle on the
        // command channel so we don't busy-spin.
        if live && client.is_some() {
            match client.as_mut().unwrap().request(&Request::Depths) {
                Ok(Response::Depths(values)) => {
                    let mut frame = [0u8; TOTAL_KEYS];
                    let n = values.len().min(TOTAL_KEYS);
                    frame[..n].copy_from_slice(&values[..n]);
                    emit(Event::Depths(Box::new(frame)));
                }
                Ok(_) => {} // e.g. "no depth data" — ignore this round
                Err(e) => {
                    client = None;
                    emit(Event::Disconnected(e.to_string()));
                }
            }
            thread::sleep(Duration::from_millis(8));
        } else {
            match cmd_rx.recv_timeout(Duration::from_millis(250)) {
                Ok(cmd) => {
                    if dispatch(cmd, &mut client, &mut live, &emit) {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

/// Apply one command. Returns `true` if the worker should shut down.
fn dispatch(
    cmd: Command,
    client: &mut Option<ipc::Client>,
    live: &mut bool,
    emit: &impl Fn(Event),
) -> bool {
    match cmd {
        Command::Reconnect | Command::Refresh => connect(client, emit),
        Command::SetLiveDepth(on) => {
            *live = on;
            emit(Event::Status(if on {
                "Live key depth on".into()
            } else {
                "Live key depth off".into()
            }));
        }
        other => {
            let Some(c) = client.as_mut() else {
                emit(Event::Error("daemon not connected".into()));
                return false;
            };
            let (request, ok_status) = match command_to_request(other) {
                Some(pair) => pair,
                None => return false,
            };
            match c.request(&request) {
                Ok(Response::Ok) => emit(Event::Status(ok_status)),
                Ok(Response::Error(e)) => emit(Event::Error(e)),
                Ok(_) => emit(Event::Status(ok_status)),
                Err(e) => {
                    *client = None;
                    emit(Event::Disconnected(e.to_string()));
                }
            }
        }
    }
    false
}

/// Translate a mutating command into an IPC request plus a success message.
fn command_to_request(cmd: Command) -> Option<(Request, String)> {
    let pair = match cmd {
        Command::ApplyActuation(values) => {
            (Request::ApplyActuation(values), "actuation applied".into())
        }
        Command::SetRapidTrigger {
            rapid_trigger,
            turbo,
        } => (
            Request::SetRapidTrigger {
                rapid_trigger,
                turbo,
            },
            format!(
                "rapid trigger {}, turbo {}",
                on_off(rapid_trigger),
                on_off(turbo)
            ),
        ),
        Command::SetLed {
            direction,
            sequence,
            speed,
            brightness,
            rgb,
        } => (
            Request::SetLed {
                direction,
                sequence,
                speed,
                brightness,
                rgb,
            },
            "lighting updated".into(),
        ),
        Command::Reset => (Request::Reset, "defaults restored".into()),
        // Connect/Refresh/SetLiveDepth are handled before this point.
        Command::Reconnect | Command::Refresh | Command::SetLiveDepth(_) => return None,
    };
    Some(pair)
}

fn connect(client: &mut Option<ipc::Client>, emit: &impl Fn(Event)) {
    emit(Event::Status("connecting to daemon…".into()));
    match ipc::Client::connect() {
        Ok(mut c) => match c.request(&Request::Status) {
            Ok(Response::Status(status)) => {
                *client = Some(c);
                emit(Event::Connected(status));
            }
            Ok(Response::Error(e)) => emit(Event::Disconnected(e)),
            Ok(_) => emit(Event::Disconnected("unexpected daemon reply".into())),
            Err(e) => emit(Event::Disconnected(e.to_string())),
        },
        Err(e) => emit(Event::Disconnected(e.to_string())),
    }
}

fn on_off(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}
