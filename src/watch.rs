//! Focus watcher: drives automatic profile switching from the active window.
//!
//! On KDE/KWin (Wayland) there is no portable API to read the focused window's
//! app-id, so we go through KWin's scripting engine: a small JS script connects
//! to `workspace.windowActivated` and pushes each activation's `resourceClass`
//! to a D-Bus service this module exposes. Incoming activations are matched
//! against [`Rules`] and applied via the daemon's job channel.

use std::process::Command;
use std::sync::mpsc::Sender;

use crate::daemon::{self, Job};
use crate::error::{Error, Result};
use crate::ipc::{Request, Response};
use crate::rules::{self, Rules};

const DBUS_NAME: &str = "org.fawnd.Focus";
const DBUS_PATH: &str = "/Focus";
const KWIN_SCRIPT_NAME: &str = "fawnd";

/// KWin JS: report the active window's resource class to our D-Bus service.
const KWIN_SCRIPT: &str = r#"
function report(w) {
    if (w && w.resourceClass) {
        callDBus("org.fawnd.Focus", "/Focus", "org.fawnd.Focus", "Activated",
                 "" + w.resourceClass);
    }
}
workspace.windowActivated.connect(report);
if (workspace.activeWindow) {
    report(workspace.activeWindow);
}
"#;

/// Live focus watcher. Keep it alive for the daemon's lifetime; dropping it
/// releases the D-Bus name and stops serving activations.
pub struct Watcher {
    _conn: zbus::blocking::Connection,
}

/// D-Bus object receiving window activations from the KWin script.
struct FocusService {
    job_tx: Sender<Job>,
    rules: Rules,
    current: Option<String>,
}

#[zbus::interface(name = "org.fawnd.Focus")]
impl FocusService {
    /// Called by the KWin script whenever the focused window changes.
    fn activated(&mut self, app_id: String) {
        let Some(profile) = self.rules.profile_for(&app_id).map(str::to_owned) else {
            return;
        };
        if self.current.as_deref() == Some(profile.as_str()) {
            return; // already on this profile; don't re-apply on every focus
        }
        tracing::info!("focus '{app_id}' -> profile '{profile}'");
        match daemon::dispatch(Request::ApplyProfile(profile.clone()), &self.job_tx) {
            Response::Ok => self.current = Some(profile),
            other => tracing::warn!("auto-switch to '{profile}' failed: {other:?}"),
        }
    }
}

/// Start the focus watcher if rules are configured. Returns `Ok(None)` when no
/// `rules.toml` exists (auto-switching simply disabled).
pub fn start(job_tx: Sender<Job>) -> Result<Option<Watcher>> {
    let Some(rules) = Rules::load()? else {
        return Ok(None);
    };

    let service = FocusService {
        job_tx,
        rules,
        current: None,
    };

    let conn = zbus::blocking::connection::Builder::session()
        .map_err(zerr)?
        .name(DBUS_NAME)
        .map_err(zerr)?
        .serve_at(DBUS_PATH, service)
        .map_err(zerr)?
        .build()
        .map_err(zerr)?;

    install_kwin_script()?;

    Ok(Some(Watcher { _conn: conn }))
}

fn zerr(e: zbus::Error) -> Error {
    Error::Daemon(format!("dbus: {e}"))
}

/// Write the KWin script and (re)load it into the running compositor.
fn install_kwin_script() -> Result<()> {
    let dir = rules::config_dir();
    std::fs::create_dir_all(&dir)?;
    let script_path = dir.join("kwin-focus.js");
    std::fs::write(&script_path, KWIN_SCRIPT)?;
    let script = script_path
        .to_str()
        .ok_or_else(|| Error::Daemon("non-UTF-8 script path".into()))?;

    // Replace any script from a previous run (ignore failure if not loaded).
    let _ = kwin_call(&["/Scripting", "unloadScript", KWIN_SCRIPT_NAME]);

    let load = kwin_call(&["/Scripting", "loadScript", script, KWIN_SCRIPT_NAME])?;
    if !load.status.success() {
        return Err(Error::Daemon(format!(
            "KWin loadScript failed: {}",
            String::from_utf8_lossy(&load.stderr).trim()
        )));
    }
    let start = kwin_call(&["/Scripting", "start"])?;
    if !start.status.success() {
        return Err(Error::Daemon(format!(
            "KWin script start failed: {}",
            String::from_utf8_lossy(&start.stderr).trim()
        )));
    }
    Ok(())
}

const QDBUS_BINS: &[&str] = &["qdbus6", "qdbus", "qdbus-qt6"];

/// Invoke a method on `org.kde.KWin` via whichever qdbus binary is present.
fn kwin_call(args: &[&str]) -> Result<std::process::Output> {
    for bin in QDBUS_BINS {
        match Command::new(bin).arg("org.kde.KWin").args(args).output() {
            Ok(output) => return Ok(output),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(Error::Daemon(
        "no qdbus binary found (KDE/Qt tools required for focus watching)".into(),
    ))
}
