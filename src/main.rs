// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod arrange;
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
    // Scripted UI checks run without vsync so a hidden window isn't throttled.
    let vsync = std::env::var_os("PLANAR_SCRIPT").is_none();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Planar")
            .with_app_id("planar")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([900.0, 560.0])
            .with_icon(theme::app_icon()),
        glow_options: eframe::egui_glow::GlowConfiguration { vsync, ..Default::default() },
        ..Default::default()
    };
    let run = |options: eframe::NativeOptions| {
        let root = root.clone();
        eframe::run_native("Planar", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, root)))))
    };

    // Windows: Direct3D 12 through wgpu (also covers machines without a GPU driver via
    // the WARP software adapter), falling back to OpenGL. Vulkan stays off: wgpu would
    // load every installed Vulkan driver, and some (e.g. older Intel UHD drivers) crash
    // the process while being enumerated, which no fallback can catch.
    #[cfg(windows)]
    let result = {
        let mut wgpu_options = options.clone();
        wgpu_options.renderer = eframe::Renderer::Wgpu;
        if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_options.wgpu_setup {
            setup.instance_descriptor.backends = eframe::wgpu::Backends::DX12;
        }
        if !vsync {
            wgpu_options.wgpu_options.surface.present_mode = eframe::wgpu::PresentMode::AutoNoVsync;
        }
        run(wgpu_options).or_else(|_| run(eframe::NativeOptions { renderer: eframe::Renderer::Glow, ..options }))
    };
    #[cfg(not(windows))]
    let result = run(options);

    // A GUI-subsystem executable has no console, so report startup failures in a dialog.
    #[cfg(windows)]
    if let Err(err) = &result {
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Planar")
            .set_description(format!("Planar could not start.\n\n{err}\n\nUpdating the graphics driver usually fixes this."))
            .show();
    }
    result
}
