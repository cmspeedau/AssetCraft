//! The AssetCraft desktop application.
//!
//! M5's shell: the three-pane browser layout from
//! `docs/reference/quixel-bridge/README.md`, backed by the live Fab catalog
//! through [`assetcraft_catalog`].
//!
//! ```text
//! cargo run -p assetcraft-gui             # live fab.com catalog
//! cargo run -p assetcraft-gui -- --demo   # offline demo catalog
//! ```
//!
//! Fab answers non-browser clients with a Cloudflare challenge, so the live
//! path cannot be exercised against fab.com from a dev box. `scripts/fab_stub.py`
//! serves the same wire shape locally, which is how the client, paging, and
//! thumbnail pipeline are checked end to end:
//!
//! ```text
//! python3 scripts/fab_stub.py 8791
//! ASSETCRAFT_FAB_BASE_URL=http://127.0.0.1:8791 cargo run -p assetcraft-gui
//! ```

#![forbid(unsafe_code)]

mod app;
mod data;
mod grid;
mod icons;
mod theme;
mod worker;

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
