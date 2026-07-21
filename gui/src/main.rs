//! Graphical front-end for FerrisChess, built on egui/eframe. The engine
//! library does all the chess; this binary owns the windows, the board
//! widget, and a worker thread that runs the (blocking) search.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod board_widget;
mod clock;
mod engine_driver;
mod game;
mod input;
mod movelist;
mod pieces;
mod setup;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 720.0])
            .with_min_inner_size([640.0, 480.0])
            .with_title("FerrisChess"),
        ..Default::default()
    };
    eframe::run_native(
        "FerrisChess",
        options,
        Box::new(|cc| {
            // Without the loaders the piece SVGs render as "?" placeholders.
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(app::ChessApp::new()))
        }),
    )
}

/// Web entry point, started by the trunk-generated loader in `index.html`.
/// The canvas id must match the one in that file.
#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    // Route log output (including eframe's own web diagnostics) to the
    // browser console; without this, startup failures are invisible.
    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    wasm_bindgen_futures::spawn_local(async {
        let document = eframe::web_sys::window()
            .expect("no window")
            .document()
            .expect("no document");
        let canvas = document
            .get_element_by_id("ferrischess_canvas")
            .expect("index.html is missing #ferrischess_canvas")
            .dyn_into::<eframe::web_sys::HtmlCanvasElement>()
            .expect("#ferrischess_canvas is not a canvas");
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| {
                    // Without the loaders the piece SVGs render as "?" placeholders.
                    egui_extras::install_image_loaders(&cc.egui_ctx);
                    Ok(Box::new(app::ChessApp::new()))
                }),
            )
            .await;
        if let Err(error) = result {
            eframe::web_sys::console::error_2(&"eframe failed to start:".into(), &error);
        }
    });
}
