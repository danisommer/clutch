#![windows_subsystem = "windows"]

mod admin;
mod app;
mod cleanup;
mod debloat;
mod drivers;
mod eco_mode;
mod game_mode;
mod games;
mod i18n;
mod steam;
mod storage;
mod system;
mod theme;
mod watcher;
mod xml;

use std::sync::Arc;

use eframe::egui;

const APP_NAME: &str = "Clutch";

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some(admin::ARG) => std::process::exit(admin::run(&args)),
        Some(watcher::ARG) => std::process::exit(watcher::run()),
        _ => {}
    }
    storage::migrate_legacy_data();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([1080.0, 720.0])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_decorations(false)
            .with_icon(Arc::new(theme::icon())),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(APP_NAME, options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
