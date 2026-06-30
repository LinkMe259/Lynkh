mod api;
mod app;
mod config;
mod hwid;
mod models;
mod ui_helpers;

use app::ProgramLoginApp;
use eframe::egui::{self, Vec2};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(Vec2::new(980.0, 660.0))
            .with_min_inner_size(Vec2::new(700.0, 520.0)),
        ..Default::default()
    };

    eframe::run_native(
        "NOVA Store Program Login",
        options,
        Box::new(|cc| Ok(Box::new(ProgramLoginApp::new(cc)))),
    )
}
