//! The AssetCraft desktop application.
//!
//! M5's shell: the three-pane browser layout from
//! `docs/reference/quixel-bridge/README.md`, running on mock catalog data
//! until the live `assetcraft-catalog` client is wired in.

#![forbid(unsafe_code)]

mod app;
mod data;
mod grid;
mod icons;
mod theme;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("AssetCraft")
            .with_inner_size([1420.0, 880.0])
            .with_min_inner_size([980.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        "AssetCraft",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
