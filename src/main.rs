#![windows_subsystem = "windows"]

mod admin;
mod app;
mod cleanup;
mod game_mode;
mod i18n;
mod steam;
mod storage;
mod system;
mod theme;
mod watcher;

use std::sync::Arc;

use eframe::egui;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some(admin::ARG) => std::process::exit(admin::run(&args)),
        Some(watcher::ARG) => std::process::exit(watcher::run()),
        _ => {}
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Modo Jogo e Limpeza")
            .with_inner_size([1080.0, 720.0])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_decorations(false)
            .with_icon(Arc::new(theme::icon())),
        centered: true,
        ..Default::default()
    };
    eframe::run_native("Modo Jogo e Limpeza", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
