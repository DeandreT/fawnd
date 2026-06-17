//! Browser front-end for fawnd.

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsCast;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("fawnd-web is intended for the wasm32-unknown-unknown target");
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();

    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window not available"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("document not available"))?;
    let canvas = document
        .get_element_by_id("fawnd-canvas")
        .ok_or_else(|| JsValue::from_str("missing #fawnd-canvas"))?
        .dyn_into::<web_sys::HtmlCanvasElement>()?;

    let runner = Box::leak(Box::new(eframe::WebRunner::new()));
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(err) = runner
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(fawnd::gui::app_creator),
            )
            .await
        {
            web_sys::console::error_1(&err);
        }
    });

    Ok(())
}
