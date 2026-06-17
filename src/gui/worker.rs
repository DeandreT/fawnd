//! Background worker that owns the [`Controller`] and serializes all device
//! I/O off the UI thread.
//!
//! The `hidapi` device handle is not moved across threads — the worker creates
//! the controller inside its own thread and only exchanges plain data
//! (`Command` / `Event`) with the GUI over channels.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use crate::controller::Controller;
use crate::protocol::consts::{LedSequence, TOTAL_KEYS};
use crate::protocol::Identity;

/// Requests from the UI to the device worker.
pub enum Command {
    /// (Re)open the device and report identity.
    Reconnect,
    /// Re-query identity / toggle state.
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

/// Notifications from the worker back to the UI.
pub enum Event {
    Connected(Identity),
    Disconnected(String),
    Status(String),
    Error(String),
    /// One live key-depth frame (per-slot travel, ~0.1 mm units).
    Depths(Box<[u8; TOTAL_KEYS]>),
}

/// UI-side handle to the worker.
pub struct Worker {
    tx: Sender<Command>,
    rx: Receiver<Event>,
}

impl Worker {
    /// Spawn the worker thread. `repaint` is used to wake the egui event loop
    /// whenever a new [`Event`] is emitted.
    pub fn spawn(repaint: egui::Context) -> Worker {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
        let (evt_tx, evt_rx) = mpsc::channel::<Event>();

        thread::Builder::new()
            .name("fawnd-device".into())
            .spawn(move || run(cmd_rx, evt_tx, repaint))
            .expect("spawn device worker");

        Worker {
            tx: cmd_tx,
            rx: evt_rx,
        }
    }

    /// Queue a command for the worker. Errors are surfaced as [`Event`]s, so the
    /// send result is intentionally ignored beyond logging.
    pub fn send(&self, cmd: Command) {
        if self.tx.send(cmd).is_err() {
            tracing::warn!("device worker is gone");
        }
    }

    /// Drain all pending events without blocking.
    pub fn poll(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }
}

fn run(cmd_rx: Receiver<Command>, evt_tx: Sender<Event>, repaint: egui::Context) {
    use std::sync::mpsc::RecvTimeoutError;
    use std::time::Duration;

    let emit = |evt: Event| {
        let _ = evt_tx.send(evt);
        repaint.request_repaint();
    };

    let mut controller: Option<Controller> = None;
    let mut live = false;

    // Initial connect attempt.
    connect(&mut controller, &emit);

    loop {
        // Drain any queued commands first.
        loop {
            match cmd_rx.try_recv() {
                Ok(cmd) => {
                    if dispatch(cmd, &mut controller, &mut live, &emit) {
                        return;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
            }
        }

        // While live and connected, stream depth frames; otherwise idle until a
        // command arrives (so we don't busy-spin).
        if live && controller.is_some() {
            let ctl = controller.as_mut().unwrap();
            match ctl.poll_depths(Duration::from_millis(50)) {
                Ok(Some(frame)) => emit(Event::Depths(Box::new(frame))),
                Ok(None) => {}
                Err(e) => {
                    controller = None;
                    emit(Event::Disconnected(e.to_string()));
                }
            }
            // Cap the stream to ~120 Hz to bound CPU and repaint load.
            std::thread::sleep(Duration::from_millis(8));
        } else {
            match cmd_rx.recv_timeout(Duration::from_millis(250)) {
                Ok(cmd) => {
                    if dispatch(cmd, &mut controller, &mut live, &emit) {
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
    controller: &mut Option<Controller>,
    live: &mut bool,
    emit: &impl Fn(Event),
) -> bool {
    match cmd {
        Command::Reconnect | Command::Refresh => connect(controller, emit),
        Command::SetLiveDepth(on) => {
            *live = on;
            emit(Event::Status(if on {
                "Live key depth on".into()
            } else {
                "Live key depth off".into()
            }));
        }
        other => {
            let Some(ctl) = controller.as_mut() else {
                emit(Event::Error("not connected".into()));
                return false;
            };
            match handle(ctl, other) {
                Ok(status) => emit(Event::Status(status)),
                Err(e) => {
                    *controller = None;
                    emit(Event::Disconnected(e.to_string()));
                }
            }
        }
    }
    false
}

fn connect(controller: &mut Option<Controller>, emit: &impl Fn(Event)) {
    emit(Event::Status("connecting…".into()));
    match Controller::open() {
        Ok(ctl) => match ctl.identity() {
            Ok(id) => {
                *controller = Some(ctl);
                emit(Event::Connected(id));
            }
            Err(e) => emit(Event::Disconnected(e.to_string())),
        },
        Err(e) => emit(Event::Disconnected(e.to_string())),
    }
}

/// Execute a single command against a connected controller. Returns a status
/// string on success, or `Err` on a device I/O failure (handled as a
/// disconnect by the caller).
fn handle(ctl: &mut Controller, cmd: Command) -> crate::Result<String> {
    let status = match cmd {
        Command::Reconnect | Command::Refresh | Command::SetLiveDepth(_) => {
            unreachable!("handled in dispatch()")
        }
        Command::ApplyActuation(values) => {
            for (i, mm) in values.iter().enumerate().take(TOTAL_KEYS) {
                ctl.set_actuation_index(i, *mm);
            }
            ctl.flush_actuation()?;
            "actuation applied".into()
        }
        Command::SetRapidTrigger {
            rapid_trigger,
            turbo,
        } => {
            ctl.set_rapid_trigger(rapid_trigger, turbo)?;
            format!("rapid trigger {}, turbo {}", on_off(rapid_trigger), on_off(turbo))
        }
        Command::SetLed {
            direction,
            sequence,
            speed,
            brightness,
            rgb,
        } => {
            ctl.set_led(direction, sequence, speed, brightness, rgb)?;
            "lighting updated".into()
        }
        Command::Reset => {
            ctl.write_defaults()?;
            "defaults restored".into()
        }
    };
    Ok(status)
}

fn on_off(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}
