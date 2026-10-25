// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod canvas;
mod fsmodel;
mod icons;
mod layout;
mod ops;
mod script;
mod theme;
mod uicons;

use std::path::PathBuf;

fn main() -> eframe::Result {
    // Optional first argument: the folder to open as canvas root.
    let root = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Planar")
            .with_app_id("planar")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 560.0])
            .with_icon(theme::app_icon()),
        // Scripted UI checks run without vsync so a hidden window isn't throttled.
        glow_options: eframe::egui_glow::GlowConfiguration { vsync: std::env::var_os("PLANAR_SCRIPT").is_none(), ..Default::default() },
        ..Default::default()
    };
    eframe::run_native("Planar", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, root)))))
}
