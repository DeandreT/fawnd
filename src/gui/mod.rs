//! egui/eframe configuration UI for the keyboard.
//!
//! [`run`] launches the native window. The UI is a thin layer over
//! [`crate::controller::Controller`]: all device I/O happens on a background
//! [`worker::Worker`] thread and the UI exchanges plain data with it.

mod api;
mod app;
#[cfg(not(target_arch = "wasm32"))]
mod worker;
#[cfg(target_arch = "wasm32")]
#[path = "worker_web.rs"]
mod worker;

/// Launch the native configuration window.
#[cfg(not(target_arch = "wasm32"))]
pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 680.0])
            .with_min_inner_size([900.0, 500.0])
            .with_title("fawnd — DrunkDeer A75"),
        ..Default::default()
    };

    eframe::run_native(
        "fawnd",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}

#[cfg(target_arch = "wasm32")]
pub fn app_creator(
    cc: &eframe::CreationContext<'_>,
) -> Result<Box<dyn eframe::App>, Box<dyn std::error::Error + Send + Sync>> {
    Ok(Box::new(app::App::new(cc)))
}
