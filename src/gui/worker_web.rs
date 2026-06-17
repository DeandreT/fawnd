//! Browser-side worker backed by WebHID.
//!
//! WebHID does not expose blocking reads like `hidapi`; output reports are
//! promises and input reports arrive through `inputreport` events. This module
//! keeps the egui-facing `Worker` API synchronous by queueing commands and
//! completing the actual device I/O on the browser task queue.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use js_sys::{Array, Function, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};

use crate::ipc::Status;
use crate::protocol::Identity;
use crate::protocol::consts::{
    DEFAULT_ACTUATION, KEYS_PER_ROW, LAST_ROW_KEYS, LedSequence, PRODUCT_IDS, REPORT_ID,
    TOTAL_KEYS, USAGE, USAGE_PAGE, VENDOR_ID, cmd,
};
use crate::protocol::packet::{self, Payload, RowKind};

const WRITE_PACING_MS: i32 = 20;
const IDENTITY_TIMEOUT_MS: i32 = 2_000;
const DEPTH_TIMEOUT_MS: i32 = 80;

type HidDevice = JsValue;

pub use super::api::{Command, Event};

/// UI-side handle to the browser worker.
pub struct Worker {
    state: Rc<RefCell<State>>,
}

struct State {
    events: Vec<Event>,
    pending: VecDeque<Command>,
    reports: VecDeque<Vec<u8>>,
    device: Option<HidDevice>,
    input_handler: Option<Closure<dyn FnMut(JsValue)>>,
    repaint: egui::Context,
    busy: bool,
    live: bool,
    live_task_running: bool,
}

impl Worker {
    /// Create the browser worker and try to reconnect to any already-granted
    /// DrunkDeer device. A new browser permission prompt still requires the
    /// user to click `Reconnect`.
    pub fn spawn(repaint: egui::Context) -> Worker {
        let state = Rc::new(RefCell::new(State {
            events: vec![Event::Status(
                "WebHID ready. Click Reconnect and select the DrunkDeer config interface.".into(),
            )],
            pending: VecDeque::new(),
            reports: VecDeque::new(),
            device: None,
            input_handler: None,
            repaint,
            busy: false,
            live: false,
            live_task_running: false,
        }));
        spawn_local(connect_task(state.clone(), false));
        Worker { state }
    }

    /// Queue a command for asynchronous WebHID processing.
    pub fn send(&self, cmd: Command) {
        self.state.borrow_mut().pending.push_back(cmd);
        run_queue(self.state.clone());
    }

    /// Drain all pending events without blocking.
    pub fn poll(&self) -> Vec<Event> {
        self.state.borrow_mut().events.drain(..).collect()
    }
}

fn run_queue(state: Rc<RefCell<State>>) {
    if state.borrow().busy {
        return;
    }
    state.borrow_mut().busy = true;
    spawn_local(async move {
        loop {
            let Some(cmd) = state.borrow_mut().pending.pop_front() else {
                state.borrow_mut().busy = false;
                break;
            };
            handle_command(state.clone(), cmd).await;
        }
    });
}

async fn handle_command(state: Rc<RefCell<State>>, cmd: Command) {
    match cmd {
        Command::Reconnect => connect_task(state, true).await,
        Command::Refresh => connect_task(state, false).await,
        Command::SetLiveDepth(on) => {
            state.borrow_mut().live = on;
            if on {
                emit(&state, Event::Status("Live key depth on".into()));
                start_live_task(state);
            } else {
                emit(&state, Event::Status("Live key depth off".into()));
                emit(&state, Event::Depths(Box::new([0; TOTAL_KEYS])));
            }
        }
        other => {
            let result = match state.borrow().device.clone() {
                Some(device) => dispatch_device_command(&device, other).await,
                None => Err("keyboard not connected; click Reconnect first".into()),
            };
            match result {
                Ok(status) => emit(&state, Event::Status(status)),
                Err(err) => emit(&state, Event::Error(err)),
            }
        }
    }
}

async fn dispatch_device_command(device: &HidDevice, cmd: Command) -> Result<String, String> {
    match cmd {
        Command::ApplyActuation(values) => {
            let mut raw = [DEFAULT_ACTUATION; TOTAL_KEYS];
            for (dst, mm) in raw.iter_mut().zip(values) {
                *dst = packet::mm_to_byte(mm);
            }
            flush_rows(device, &raw, RowKind::Actuation).await?;
            Ok("actuation applied".into())
        }
        Command::SetRapidTrigger {
            rapid_trigger,
            turbo,
        } => {
            send_payload(device, &packet::rapid_trigger_turbo(rapid_trigger, turbo)).await?;
            Ok(format!(
                "rapid trigger {}, turbo {}",
                on_off(rapid_trigger),
                on_off(turbo)
            ))
        }
        Command::SetLed {
            direction,
            sequence,
            speed,
            brightness,
            rgb,
        } => {
            send_payload(
                device,
                &packet::led_mode(direction, sequence, speed, brightness, rgb, false),
            )
            .await?;
            Ok("lighting updated".into())
        }
        Command::Reset => {
            send_payload(
                device,
                &packet::led_mode(0, LedSequence::Off, 5, 9, 0xff, false),
            )
            .await?;
            send_payload(device, &packet::rapid_trigger_turbo(false, false)).await?;
            flush_rows(device, &[DEFAULT_ACTUATION; TOTAL_KEYS], RowKind::Actuation).await?;
            flush_rows(device, &[0; TOTAL_KEYS], RowKind::Downstroke).await?;
            flush_rows(device, &[0; TOTAL_KEYS], RowKind::Upstroke).await?;
            Ok("defaults restored".into())
        }
        Command::Reconnect | Command::Refresh | Command::SetLiveDepth(_) => Ok(String::new()),
    }
}

async fn connect_task(state: Rc<RefCell<State>>, prompt: bool) {
    emit(
        &state,
        Event::Status(if prompt {
            "opening WebHID chooser...".into()
        } else {
            "checking for an authorized WebHID device...".into()
        }),
    );

    let result = async {
        let device = select_device(prompt).await?;
        attach_input_handler(&state, &device)?;
        open_device(&device).await?;
        state.borrow_mut().reports.clear();
        let identity = identity(&state, &device).await?;

        state.borrow_mut().device = Some(device);
        Ok::<_, String>(identity)
    }
    .await;

    match result {
        Ok(identity) => emit(
            &state,
            Event::Connected(Status {
                model: format!("{:?}", identity.model),
                firmware: identity.firmware_version,
                rapid_trigger: identity.rapid_trigger,
                turbo: identity.turbo,
                active_profile: Some("WebHID".into()),
            }),
        ),
        Err(err) => {
            state.borrow_mut().device = None;
            emit(&state, Event::Disconnected(err));
        }
    }
}

async fn select_device(prompt: bool) -> Result<HidDevice, String> {
    let hid = hid()?;
    let devices = if prompt {
        request_devices(&hid).await?
    } else {
        get_devices(&hid).await?
    };

    devices
        .into_iter()
        .find(is_supported_device)
        .ok_or_else(|| {
            if prompt {
                "no DrunkDeer config interface selected".into()
            } else {
                "no authorized DrunkDeer keyboard; click Reconnect to grant WebHID access".into()
            }
        })
}

async fn get_devices(hid: &JsValue) -> Result<Vec<HidDevice>, String> {
    let get_devices = method(hid, "getDevices")?;
    let promise = get_devices
        .call0(hid)
        .map_err(|e| format!("WebHID getDevices failed: {}", js_error(e)))?
        .dyn_into::<Promise>()
        .map_err(|_| "WebHID getDevices did not return a Promise".to_string())?;
    let value = JsFuture::from(promise)
        .await
        .map_err(|e| format!("WebHID getDevices failed: {}", js_error(e)))?;
    Ok(array_to_vec(&Array::from(&value)))
}

async fn request_devices(hid: &JsValue) -> Result<Vec<HidDevice>, String> {
    let request_device = method(hid, "requestDevice")?;
    let options = Object::new();
    Reflect::set(&options, &"filters".into(), &request_filters())
        .map_err(|e| format!("cannot build WebHID filters: {}", js_error(e)))?;

    let promise = request_device
        .call1(hid, &options)
        .map_err(|e| format!("WebHID requestDevice failed: {}", js_error(e)))?
        .dyn_into::<Promise>()
        .map_err(|_| "WebHID requestDevice did not return a Promise".to_string())?;
    let value = JsFuture::from(promise)
        .await
        .map_err(|e| format!("WebHID requestDevice failed: {}", js_error(e)))?;
    Ok(array_to_vec(&Array::from(&value)))
}

fn request_filters() -> Array {
    let filters = Array::new();
    for product_id in PRODUCT_IDS {
        let filter = Object::new();
        let _ = Reflect::set(
            &filter,
            &"vendorId".into(),
            &JsValue::from(VENDOR_ID as u32),
        );
        let _ = Reflect::set(&filter, &"productId".into(), &JsValue::from(*product_id));
        let _ = Reflect::set(&filter, &"usagePage".into(), &JsValue::from(USAGE_PAGE));
        let _ = Reflect::set(&filter, &"usage".into(), &JsValue::from(USAGE));
        filters.push(&filter);
    }
    filters
}

fn is_supported_device(device: &HidDevice) -> bool {
    let vendor_id = get_u16(device, "vendorId");
    let product_id = get_u16(device, "productId");
    vendor_id == Some(VENDOR_ID) && product_id.is_some_and(|id| PRODUCT_IDS.contains(&id))
}

async fn open_device(device: &HidDevice) -> Result<(), String> {
    if get_bool(device, "opened").unwrap_or(false) {
        return Ok(());
    }

    let open = method(device, "open")?;
    let promise = open
        .call0(device)
        .map_err(|e| format!("WebHID open failed: {}", js_error(e)))?
        .dyn_into::<Promise>()
        .map_err(|_| "WebHID open did not return a Promise".to_string())?;
    JsFuture::from(promise)
        .await
        .map_err(|e| format!("WebHID open failed: {}", js_error(e)))?;
    Ok(())
}

fn attach_input_handler(state: &Rc<RefCell<State>>, device: &HidDevice) -> Result<(), String> {
    let handler_state = state.clone();
    let handler = Closure::wrap(Box::new(move |event: JsValue| {
        if let Some(payload) = input_payload(&event) {
            let mut state = handler_state.borrow_mut();
            state.reports.push_back(payload);
            state.repaint.request_repaint();
        }
    }) as Box<dyn FnMut(JsValue)>);

    Reflect::set(
        device,
        &"oninputreport".into(),
        handler.as_ref().unchecked_ref(),
    )
    .map_err(|e| format!("cannot attach inputreport handler: {}", js_error(e)))?;

    state.borrow_mut().input_handler = Some(handler);
    Ok(())
}

async fn identity(state: &Rc<RefCell<State>>, device: &HidDevice) -> Result<Identity, String> {
    state.borrow_mut().reports.clear();
    send_payload(device, &packet::identity()).await?;

    let deadline = now_ms() + IDENTITY_TIMEOUT_MS as f64;
    while now_ms() < deadline {
        while let Some(report) = state.borrow_mut().reports.pop_front() {
            if let Some(identity) = Identity::parse(&report) {
                return Ok(identity);
            }
        }
        sleep_ms(20).await;
    }

    Err("device did not respond to identity handshake".into())
}

fn start_live_task(state: Rc<RefCell<State>>) {
    if state.borrow().live_task_running {
        return;
    }
    state.borrow_mut().live_task_running = true;

    spawn_local(async move {
        loop {
            let device = {
                let state_ref = state.borrow();
                if !state_ref.live {
                    break;
                }
                state_ref.device.clone()
            };

            let Some(device) = device else {
                emit(&state, Event::Error("keyboard not connected".into()));
                break;
            };

            match sample_depths(&state, &device).await {
                Ok(Some(frame)) => emit(&state, Event::Depths(Box::new(frame))),
                Ok(None) => {}
                Err(err) => {
                    emit(&state, Event::Error(err));
                    break;
                }
            }
            sleep_ms(8).await;
        }

        let mut state = state.borrow_mut();
        state.live = false;
        state.live_task_running = false;
    });
}

async fn sample_depths(
    state: &Rc<RefCell<State>>,
    device: &HidDevice,
) -> Result<Option<[u8; TOTAL_KEYS]>, String> {
    send_payload(device, &packet::key_tracking(true)).await?;

    let mut frame = [0u8; TOTAL_KEYS];
    let mut seen = [false; 3];
    let deadline = now_ms() + DEPTH_TIMEOUT_MS as f64;

    while now_ms() < deadline && !(seen[0] && seen[1] && seen[2]) {
        while let Some(report) = state.borrow_mut().reports.pop_front() {
            if report.first() != Some(&cmd::KEY_TRACKING) || report.len() < 5 {
                continue;
            }
            let row = report[3] as usize;
            if row >= 3 {
                continue;
            }
            let base = row * KEYS_PER_ROW;
            let count = if row == 2 {
                LAST_ROW_KEYS
            } else {
                KEYS_PER_ROW
            };
            for (i, &value) in report[4..].iter().take(count).enumerate() {
                if base + i < TOTAL_KEYS {
                    frame[base + i] = value;
                }
            }
            seen[row] = true;
        }
        if !(seen[0] && seen[1] && seen[2]) {
            sleep_ms(4).await;
        }
    }

    Ok(seen.iter().any(|&row| row).then_some(frame))
}

async fn flush_rows(
    device: &HidDevice,
    values: &[u8; TOTAL_KEYS],
    kind: RowKind,
) -> Result<(), String> {
    for (row, chunk) in values.chunks(KEYS_PER_ROW).enumerate() {
        send_payload(device, &packet::modify_row(row as u8, chunk, kind)).await?;
        sleep_ms(WRITE_PACING_MS).await;
    }
    Ok(())
}

async fn send_payload(device: &HidDevice, payload: &Payload) -> Result<(), String> {
    let send_report = method(device, "sendReport")?;
    let data = Uint8Array::from(payload.as_slice());
    let promise = send_report
        .call2(device, &JsValue::from(REPORT_ID), &data)
        .map_err(|e| format!("WebHID sendReport failed: {}", js_error(e)))?
        .dyn_into::<Promise>()
        .map_err(|_| "WebHID sendReport did not return a Promise".to_string())?;
    JsFuture::from(promise)
        .await
        .map_err(|e| format!("WebHID sendReport failed: {}", js_error(e)))?;
    Ok(())
}

fn input_payload(event: &JsValue) -> Option<Vec<u8>> {
    let data = Reflect::get(event, &"data".into()).ok()?;
    let data = data.dyn_into::<js_sys::DataView>().ok()?;
    let mut bytes = Vec::with_capacity(data.byte_length() as usize);
    for i in 0..data.byte_length() {
        bytes.push(data.get_uint8(i));
    }

    let report_id = Reflect::get(event, &"reportId".into())
        .ok()
        .and_then(|value| value.as_f64())
        .map(|value| value as u8)
        .unwrap_or(0);

    if report_id == REPORT_ID {
        Some(bytes)
    } else if bytes.first() == Some(&REPORT_ID) {
        Some(bytes[1..].to_vec())
    } else {
        Some(bytes)
    }
}

fn hid() -> Result<JsValue, String> {
    let window = web_sys::window().ok_or_else(|| "window not available".to_string())?;
    let navigator = Reflect::get(window.as_ref(), &"navigator".into())
        .map_err(|e| format!("navigator not available: {}", js_error(e)))?;
    let hid = Reflect::get(&navigator, &"hid".into())
        .map_err(|e| format!("WebHID unavailable: {}", js_error(e)))?;

    if hid.is_undefined() || hid.is_null() {
        return Err("WebHID is unavailable in this browser or context. Use a Chromium browser over HTTPS or localhost.".into());
    }
    Ok(hid)
}

fn array_to_vec(array: &Array) -> Vec<JsValue> {
    array.iter().collect()
}

fn method(target: &JsValue, name: &str) -> Result<Function, String> {
    Reflect::get(target, &name.into())
        .map_err(|e| format!("{name} unavailable: {}", js_error(e)))?
        .dyn_into::<Function>()
        .map_err(|_| format!("{name} is not callable"))
}

fn get_u16(target: &JsValue, name: &str) -> Option<u16> {
    Reflect::get(target, &name.into())
        .ok()
        .and_then(|value| value.as_f64())
        .map(|value| value as u16)
}

fn get_bool(target: &JsValue, name: &str) -> Option<bool> {
    Reflect::get(target, &name.into())
        .ok()
        .and_then(|value| value.as_bool())
}

fn emit(state: &Rc<RefCell<State>>, event: Event) {
    let mut state = state.borrow_mut();
    state.events.push(event);
    state.repaint.request_repaint();
}

async fn sleep_ms(ms: i32) {
    let promise = Promise::new(&mut |resolve, _reject| {
        let Some(window) = web_sys::window() else {
            return;
        };
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
    });
    let _ = JsFuture::from(promise).await;
}

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map(|performance| performance.now())
        .unwrap_or(0.0)
}

fn js_error(value: JsValue) -> String {
    if let Some(message) = value.as_string() {
        return message;
    }
    Reflect::get(&value, &"message".into())
        .ok()
        .and_then(|message| message.as_string())
        .unwrap_or_else(|| format!("{value:?}"))
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}
