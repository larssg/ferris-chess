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

/// The web build arrives with the trunk/wasm milestone; the entry point is
/// split already so nothing here needs restructuring for it.
#[cfg(target_arch = "wasm32")]
fn main() {
    unimplemented!("web entry point lands with the wasm milestone");
}
