//! Main window: navigation, Game Mode, Cleanup and History pages.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, CornerRadius, CursorIcon, Frame, Id, Layout, Margin, Pos2, Rect, Response,
    Sense, StrokeKind, Ui, Vec2, ViewportCommand,
};

use crate::admin::{self, AdminError};
use crate::cleanup::{self, Analysis, Catalog, Category, CleanupResult, Group};
use crate::game_mode::{self, Item, Status};
use crate::storage::{self, ChosenGame, Config, Force, HistoryEntry, HistoryKind, ProgramConfig, Session};
use crate::theme::{self, *};
use crate::{steam, system, watcher};

const NAV_WIDTH: f32 = 252.0;
const TITLE_BAR_HEIGHT: f32 = 40.0;
const AUTO_CLOSE_DELAY: Duration = Duration::from_secs(10);
const MONITOR_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    GameMode,
    Cleanup,
    History,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GamePhase {
    Ready,
    Activating,
    Active,
    Deactivating,
    Finished,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CleanupPhase {
    Idle,
    Analyzing,
    Analyzed,
    MeasuringSystem,
    Cleaning,
    Done,
}

enum Message {
    Overview { items: Vec<Item>, games: Vec<steam::Manifest> },
    Candidates(Vec<(String, String)>),
    GameProgress(String),
    Activated(game_mode::Activation),
    GameStarted,
    GameExited,
    Deactivated(Vec<Item>),
    CleanupProgress(String),
    Analyzed(Vec<Analysis>),
    SystemMeasured(Result<HashMap<String, u64>, String>),
    Cleaned { results: Vec<CleanupResult>, warning: Option<String> },
    BlockersClosed(Vec<Item>),
}

enum GameAction {
    Refresh,
    FindCandidates,
    Add(String),
    Remove(usize),
    Activate,
    Deactivate,
    Back,
    KeepOpen,
}

enum CleanupAction {
    Analyze,
    MeasureSystem,
    Clean,
    CloseBlockers(String),
}

pub struct App {
    page: Page,
    config: Config,
    catalog: Catalog,
    tx: Sender<Message>,
    rx: Receiver<Message>,
    free_space_c: Option<u64>,
    game_phase: GamePhase,
    overview_items: Vec<Item>,
    loading_overview: bool,
    games: Vec<steam::Manifest>,
    candidates: Option<Vec<(String, String)>>,
    game_progress: String,
    session: Option<Session>,
    activation_items: Vec<Item>,
    deactivation_items: Vec<Item>,
    game_running: bool,
    monitor: Option<Arc<AtomicBool>>,
    cleanup_phase: CleanupPhase,
    analyses: Vec<Analysis>,
    selection: HashSet<String>,
    cleanup_progress: String,
    results: Vec<CleanupResult>,
    cleanup_warning: Option<String>,
    blocker_items: Vec<Item>,
    history: Vec<HistoryEntry>,
    _window_marker: Option<watcher::Marker>,
    close_at: Option<Instant>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        theme::apply(&cc.egui_ctx);
        let (tx, rx) = mpsc::channel();
        let session = storage::load_session();
        let mut app = App {
            page: Page::GameMode,
            config: storage::load_config(),
            catalog: cleanup::catalog(),
            tx,
            rx,
            free_space_c: system::free_space("C:\\"),
            game_phase: if session.is_some() { GamePhase::Active } else { GamePhase::Ready },
            overview_items: Vec::new(),
            loading_overview: false,
            games: Vec::new(),
            candidates: None,
            game_progress: String::new(),
            session,
            activation_items: Vec::new(),
            deactivation_items: Vec::new(),
            game_running: false,
            monitor: None,
            cleanup_phase: CleanupPhase::Idle,
            analyses: Vec::new(),
            selection: HashSet::new(),
            cleanup_progress: String::new(),
            results: Vec::new(),
            cleanup_warning: None,
            blocker_items: Vec::new(),
            history: storage::load_history(),
            _window_marker: watcher::mark_window(),
            close_at: None,
        };
        app.refresh_overview(&cc.egui_ctx);
        if app.session.is_none() {
            if let Some(items) = storage::take_last_summary::<Vec<Item>>() {
                app.deactivation_items = items;
                app.game_phase = GamePhase::Finished;
            }
        }
        if let Some(folder) = app.session.as_ref().and_then(|s| s.game_folder.clone()) {
            watcher::start();
            app.start_monitor(&cc.egui_ctx, PathBuf::from(folder));
        }
        app
    }

    fn spawn_task(&self, ctx: &egui::Context, task: impl FnOnce(&Sender<Message>, &egui::Context) + Send + 'static) {
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            task(&tx, &ctx);
            ctx.request_repaint();
        });
    }

    fn save_config(&self) {
        let _ = storage::save_config(&self.config);
    }

    fn refresh_overview(&mut self, ctx: &egui::Context) {
        self.loading_overview = true;
        let config = self.config.clone();
        self.spawn_task(ctx, move |tx, _| {
            let items = game_mode::overview(&config);
            let games = steam::games();
            let _ = tx.send(Message::Overview { items, games });
        });
    }

    fn find_candidates(&mut self, ctx: &egui::Context) {
        let config = self.config.clone();
        self.spawn_task(ctx, move |tx, _| {
            let _ = tx.send(Message::Candidates(game_mode::programs_to_add(&config)));
        });
    }

    /// Follows the game while the window is open, so the page can show whether it is running.
    fn start_monitor(&mut self, ctx: &egui::Context, folder: PathBuf) {
        let cancel = Arc::new(AtomicBool::new(false));
        self.monitor = Some(cancel.clone());
        self.spawn_task(ctx, move |tx, ctx| {
            let mut seen = false;
            let mut misses = 0;
            while !cancel.load(Ordering::Relaxed) {
                match (game_mode::game_running(&folder), seen) {
                    (true, false) => {
                        seen = true;
                        let _ = tx.send(Message::GameStarted);
                        ctx.request_repaint();
                    }
                    (true, true) => misses = 0,
                    (false, true) => {
                        misses += 1;
                        if misses >= 2 {
                            let _ = tx.send(Message::GameExited);
                            return;
                        }
                    }
                    (false, false) => {}
                }
                thread::sleep(MONITOR_INTERVAL);
            }
        });
    }

    fn stop_monitor(&mut self) {
        if let Some(cancel) = self.monitor.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    fn activate(&mut self, ctx: &egui::Context) {
        self.game_phase = GamePhase::Activating;
        self.game_progress = "Preparando".into();
        let config = self.config.clone();
        let game = config.game.as_ref().and_then(|g| self.games.iter().find(|m| m.app_id == g.app_id).cloned());
        self.spawn_task(ctx, move |tx, ctx| {
            let progress = |step: &str| {
                let _ = tx.send(Message::GameProgress(step.into()));
                ctx.request_repaint();
            };
            let activation = game_mode::activate(&config, game.as_ref(), &progress);
            let _ = tx.send(Message::Activated(activation));
        });
    }

    fn deactivate(&mut self, ctx: &egui::Context) {
        self.stop_monitor();
        self.close_at = None;
        self.game_phase = GamePhase::Deactivating;
        let reopen = self.config.reopen_on_exit;
        self.spawn_task(ctx, move |tx, _| {
            let items = match storage::claim_session() {
                Some(session) => game_mode::deactivate(&session, reopen),
                None => wait_for_watcher_summary(),
            };
            let _ = tx.send(Message::Deactivated(items));
        });
    }

    fn analyze(&mut self, ctx: &egui::Context) {
        self.cleanup_phase = CleanupPhase::Analyzing;
        self.cleanup_warning = None;
        self.spawn_task(ctx, |tx, ctx| {
            let progress = |name: &str| {
                let _ = tx.send(Message::CleanupProgress(name.into()));
                ctx.request_repaint();
            };
            let _ = tx.send(Message::Analyzed(cleanup::analyze(&progress)));
        });
    }

    fn measure_system(&mut self, ctx: &egui::Context) {
        self.cleanup_phase = CleanupPhase::MeasuringSystem;
        self.spawn_task(ctx, |tx, _| {
            let result = admin::measure_system().map_err(|error| match error {
                AdminError::Cancelled => "Permissão negada: os itens do sistema não foram medidos.".to_string(),
                AdminError::Failed(e) => e,
            });
            let _ = tx.send(Message::SystemMeasured(result));
        });
    }

    /// Cleans the user targets in this process, then the system targets through one elevated run.
    fn clean(&mut self, ctx: &egui::Context) {
        self.cleanup_phase = CleanupPhase::Cleaning;
        self.cleanup_warning = None;
        let selected: Vec<&Category> = self.catalog.categories.iter().filter(|c| self.selection.contains(&c.id)).collect();
        let ids: Vec<String> = selected.iter().map(|c| c.id.clone()).collect();
        let admin_ids: Vec<String> = selected.iter().filter(|c| c.targets.iter().any(|t| t.admin)).map(|c| c.id.clone()).collect();
        self.spawn_task(ctx, move |tx, ctx| {
            let progress = |name: &str| {
                let _ = tx.send(Message::CleanupProgress(name.into()));
                ctx.request_repaint();
            };
            let mut by_id: HashMap<String, CleanupResult> = cleanup::clean(&ids, false, &progress).into_iter().map(|r| (r.id.clone(), r)).collect();
            let mut warning = None;
            if !admin_ids.is_empty() {
                progress("itens do sistema (aguardando permissão)");
                match admin::clean_system(&admin_ids) {
                    Ok(admin_results) => {
                        for r in admin_results {
                            let entry = by_id.entry(r.id.clone()).or_insert_with(|| CleanupResult { id: r.id.clone(), ..Default::default() });
                            entry.tally.add(r.tally);
                            if r.error.is_some() {
                                entry.error = r.error;
                            }
                        }
                    }
                    Err(AdminError::Cancelled) => warning = Some("Permissão negada: os itens do sistema não foram limpos.".to_string()),
                    Err(AdminError::Failed(e)) => warning = Some(e),
                }
            }
            let results: Vec<CleanupResult> = ids.iter().filter_map(|id| by_id.remove(id)).collect();
            let freed: u64 = results.iter().map(|r| r.tally.bytes).sum();
            let summary = match results.len() {
                1 => "1 categoria".to_string(),
                n => format!("{n} categorias"),
            };
            storage::add_history(HistoryKind::Cleanup, freed, summary);
            let _ = tx.send(Message::Cleaned { results, warning });
        });
    }

    /// Closes the programs blocking a category, using the Game Mode force policy when the program is listed there.
    fn close_blockers(&mut self, ctx: &egui::Context, id: &str) {
        let Some(category) = self.catalog.categories.iter().find(|c| c.id == id) else { return };
        let programs: Vec<ProgramConfig> = category
            .blocked_by
            .iter()
            .map(|b| {
                let force = self
                    .config
                    .programs
                    .iter()
                    .find(|p| p.processes.iter().any(|n| n.eq_ignore_ascii_case(&b.process)))
                    .map(|p| p.force)
                    .unwrap_or(Force::Never);
                ProgramConfig { name: b.name.clone(), processes: vec![b.process.clone()], force, enabled: true }
            })
            .collect();
        self.cleanup_phase = CleanupPhase::Analyzing;
        self.cleanup_progress = "programas abertos".into();
        self.spawn_task(ctx, move |tx, _| {
            let items = game_mode::close_programs(&programs).iter().map(|r| r.item()).collect();
            let _ = tx.send(Message::BlockersClosed(items));
        });
    }

    fn receive(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.rx.try_recv() {
            match message {
                Message::Overview { items, games } => {
                    self.overview_items = items;
                    self.games = games;
                    self.loading_overview = false;
                }
                Message::Candidates(list) => self.candidates = Some(list),
                Message::GameProgress(step) => self.game_progress = step,
                Message::Activated(activation) => {
                    let has_warning = activation.items.iter().any(|i| i.status == Status::Warning);
                    self.activation_items = activation.items;
                    let folder = activation.session.game_folder.clone();
                    self.session = Some(activation.session);
                    self.game_phase = GamePhase::Active;
                    self.game_running = false;
                    if let Some(f) = folder {
                        let watching = watcher::start();
                        self.start_monitor(ctx, PathBuf::from(f));
                        if watching && !has_warning {
                            self.close_at = Some(Instant::now() + AUTO_CLOSE_DELAY);
                        }
                    }
                }
                Message::GameStarted => self.game_running = true,
                Message::GameExited => {
                    self.game_running = false;
                    if self.game_phase == GamePhase::Active && !watcher::is_running() {
                        self.deactivate(ctx);
                    }
                }
                Message::Deactivated(items) => {
                    self.deactivation_items = items;
                    self.session = None;
                    self.game_phase = GamePhase::Finished;
                    self.history = storage::load_history();
                }
                Message::CleanupProgress(name) => self.cleanup_progress = name,
                Message::Analyzed(analyses) => {
                    self.analyses = analyses.into_iter().filter(Analysis::has_content).collect();
                    let preselected: HashSet<String> = self.catalog.categories.iter().filter(|c| c.selected_by_default).map(|c| c.id.clone()).collect();
                    self.selection = self.analyses.iter().filter(|a| preselected.contains(&a.id) && a.blockers.is_empty()).map(|a| a.id.clone()).collect();
                    self.cleanup_phase = CleanupPhase::Analyzed;
                }
                Message::SystemMeasured(Ok(sizes)) => {
                    for analysis in &mut self.analyses {
                        if let Some(&bytes) = sizes.get(&analysis.id) {
                            analysis.admin_bytes = Some(bytes);
                        }
                    }
                    self.analyses.retain(Analysis::has_content);
                    let ids: HashSet<String> = self.analyses.iter().map(|a| a.id.clone()).collect();
                    self.selection.retain(|id| ids.contains(id));
                    self.cleanup_phase = CleanupPhase::Analyzed;
                }
                Message::SystemMeasured(Err(error)) => {
                    self.cleanup_warning = Some(error);
                    self.cleanup_phase = CleanupPhase::Analyzed;
                }
                Message::Cleaned { results, warning } => {
                    self.results = results;
                    self.cleanup_warning = warning;
                    self.cleanup_phase = CleanupPhase::Done;
                    self.free_space_c = system::free_space("C:\\");
                    self.history = storage::load_history();
                }
                Message::BlockersClosed(items) => {
                    self.blocker_items = items;
                    self.analyze(ctx);
                }
            }
        }
    }

    fn title_bar(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let rect = ui.max_rect();
        let drag = ui.interact(rect, Id::new("drag"), Sense::CLICK | Sense::DRAG);
        if drag.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        let size = Vec2::new(46.0, rect.height());
        let close = Rect::from_min_size(Pos2::new(rect.right() - size.x, rect.top()), size);
        let minimize = Rect::from_min_size(Pos2::new(close.left() - size.x, rect.top()), size);
        let minimize_response = ui.interact(minimize, Id::new("minimize"), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand);
        let close_response = ui.interact(close, Id::new("close"), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand);
        let painter = ui.painter();
        if minimize_response.hovered() {
            painter.rect_filled(minimize, CornerRadius::ZERO, SURFACE);
        }
        if close_response.hovered() {
            painter.rect_filled(close, CornerRadius::ZERO, RED);
        }
        let c = minimize.center();
        painter.line_segment([c - Vec2::new(5.0, 0.0), c + Vec2::new(5.0, 0.0)], stroke(1.2, TEXT));
        let c = close.center();
        let color = if close_response.hovered() { WHITE } else { TEXT };
        painter.line_segment([c - Vec2::splat(5.0), c + Vec2::splat(5.0)], stroke(1.2, color));
        painter.line_segment([c + Vec2::new(-5.0, 5.0), c + Vec2::new(5.0, -5.0)], stroke(1.2, color));
        if minimize_response.clicked() {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        }
        if close_response.clicked() {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn navigation(&mut self, ui: &mut Ui) {
        ui.add_space(14.0);
        ui.label(text("Modo Jogo", semibold(24.0), TEXT));
        ui.label(text("e Limpeza", semibold(24.0), TEXT_FAINT));
        ui.add_space(16.0);
        let (bar, _) = ui.allocate_exact_size(Vec2::new(28.0, 4.0), Sense::hover());
        ui.painter().rect_filled(bar, CornerRadius::ZERO, RED);
        ui.add_space(48.0);
        for (number, page, name) in [(1, Page::GameMode, "Modo Jogo"), (2, Page::Cleanup, "Limpeza"), (3, Page::History, "Histórico")] {
            if nav_item(ui, number, name, self.page == page).clicked() {
                self.page = page;
                if page == Page::History {
                    self.history = storage::load_history();
                }
            }
        }
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.label(text(format!("v{}", env!("CARGO_PKG_VERSION")), regular(11.0), TEXT_FAINT));
            ui.add_space(14.0);
            if let Some(free) = self.free_space_c {
                ui.label(text(format!("{} livres", storage::format_size(free)), semibold(18.0), TEXT));
                ui.label(text("DISCO C:", semibold(10.5), TEXT_MUTED).extra_letter_spacing(1.4));
            }
        });
    }

    fn game_mode_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let action = match self.game_phase {
            GamePhase::Ready => self.game_ready(ui),
            GamePhase::Activating => {
                heading(ui, "Modo Jogo", "Preparando o computador.");
                loading(ui, &format!("{}…", self.game_progress));
                None
            }
            GamePhase::Active => self.game_active(ui),
            GamePhase::Deactivating => {
                heading(ui, "Modo Jogo", "Restaurando o computador.");
                loading(ui, "Reabrindo programas…");
                None
            }
            GamePhase::Finished => self.game_finished(ui),
        };
        match action {
            Some(GameAction::Refresh) => self.refresh_overview(ctx),
            Some(GameAction::FindCandidates) => self.find_candidates(ctx),
            Some(GameAction::Add(process)) => {
                self.config.programs.push(ProgramConfig { name: process.clone(), processes: vec![process], force: Force::Never, enabled: true });
                self.candidates = None;
                self.save_config();
                self.refresh_overview(ctx);
            }
            Some(GameAction::Remove(index)) => {
                self.config.programs.remove(index);
                self.save_config();
                self.refresh_overview(ctx);
            }
            Some(GameAction::Activate) => self.activate(ctx),
            Some(GameAction::Deactivate) => self.deactivate(ctx),
            Some(GameAction::KeepOpen) => self.close_at = None,
            Some(GameAction::Back) => {
                self.game_phase = GamePhase::Ready;
                self.free_space_c = system::free_space("C:\\");
                self.refresh_overview(ctx);
            }
            None => {}
        }
    }

    fn game_ready(&mut self, ui: &mut Ui) -> Option<GameAction> {
        heading(ui, "Modo Jogo", "Fecha programas pesados, desliga a WSL e ajusta a energia para jogar.");
        let mut action = None;
        let width = ui.available_width();
        let left = (width * 0.55).floor();
        let right = width - left - 48.0;
        let height = ui.available_height() - 80.0;
        let mut changed = false;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(left);
                ui.horizontal(|ui| {
                    section_label(ui, "Estado");
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if link(ui, "Atualizar", TEXT_MUTED).clicked() {
                            action = Some(GameAction::Refresh);
                        }
                    });
                });
                rule(ui);
                egui::ScrollArea::vertical().id_salt("overview").max_height(height).show(ui, |ui| {
                    ui.set_width(left);
                    match (self.loading_overview, self.overview_items.is_empty()) {
                        (true, true) => {
                            ui.add_space(12.0);
                            loading(ui, "Verificando…");
                        }
                        _ => {
                            for item in &self.overview_items {
                                item_row(ui, item, 196.0);
                            }
                        }
                    }
                });
            });
            ui.add_space(48.0);
            ui.vertical(|ui| {
                ui.set_width(right);
                egui::ScrollArea::vertical().id_salt("options").max_height(height + 26.0).show(ui, |ui| {
                    ui.set_width(right - 14.0);
                    section_label(ui, "Fechar programas");
                    rule(ui);
                    ui.add_space(8.0);
                    let mut remove = None;
                    for (index, program) in self.config.programs.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            changed |= checkbox(ui, &mut program.enabled, &program.name, true).changed();
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if link(ui, "Remover", TEXT_FAINT).clicked() {
                                    remove = Some(index);
                                }
                            });
                        });
                    }
                    if let Some(index) = remove {
                        action = Some(GameAction::Remove(index));
                    }
                    ui.add_space(4.0);
                    match &self.candidates {
                        None => {
                            if link(ui, "+ Adicionar programa aberto", RED).clicked() {
                                action = Some(GameAction::FindCandidates);
                            }
                        }
                        Some(list) if list.is_empty() => {
                            ui.label(text("Nenhum outro programa com janela aberta.", regular(13.0), TEXT_MUTED));
                        }
                        Some(list) => {
                            ui.label(text("Escolha o programa:", regular(13.0), TEXT_MUTED));
                            for (process, window_title) in list {
                                let label = format!("+ {process}  ·  {}", shorten(window_title, 34));
                                if link(ui, &label, RED).clicked() {
                                    action = Some(GameAction::Add(process.clone()));
                                }
                            }
                        }
                    }
                    ui.add_space(22.0);
                    section_label(ui, "Sistema");
                    rule(ui);
                    ui.add_space(8.0);
                    changed |= checkbox(ui, &mut self.config.shutdown_wsl, "Desligar a WSL", true).changed();
                    changed |= checkbox(ui, &mut self.config.best_performance, "Ativar Melhor desempenho", true).changed();
                    changed |= checkbox(ui, &mut self.config.reopen_on_exit, "Reabrir programas ao sair do jogo", true).changed();
                    ui.add_space(22.0);
                    section_label(ui, "Abrir jogo");
                    rule(ui);
                    ui.add_space(8.0);
                    if radio(ui, self.config.game.is_none(), "Nenhum").clicked() {
                        self.config.game = None;
                        changed = true;
                    }
                    for game in &self.games {
                        let chosen = self.config.game.as_ref().is_some_and(|g| g.app_id == game.app_id);
                        if radio(ui, chosen, &game.name).clicked() {
                            self.config.game = Some(ChosenGame { app_id: game.app_id.clone(), name: game.name.clone() });
                            changed = true;
                        }
                    }
                });
            });
        });
        if changed {
            self.save_config();
        }
        ui.add_space(24.0);
        if primary_button(ui, "Ativar Modo Jogo", 240.0, !self.loading_overview).clicked() {
            action = Some(GameAction::Activate);
        }
        action
    }

    fn game_active(&mut self, ui: &mut Ui) -> Option<GameAction> {
        let mut action = None;
        let Some(session) = &self.session else { return Some(GameAction::Back) };
        ui.label(text("Modo Jogo", semibold(40.0), TEXT));
        ui.add_space(4.0);
        tag(ui, "Ativo", RED);
        ui.add_space(26.0);
        let elapsed = storage::now_epoch().saturating_sub(session.started_at);
        ui.label(text(storage::format_duration(elapsed), light(72.0), TEXT));
        let status_line = match (&session.game, self.game_running) {
            (Some(g), true) => format!("{} em execução. O Modo Jogo encerra sozinho quando o jogo fechar.", g.name),
            (Some(g), false) => format!("Aguardando {} abrir.", g.name),
            (None, _) => "Nenhum jogo escolhido. Encerre quando terminar de jogar.".to_string(),
        };
        ui.label(text(status_line, regular(15.0), TEXT_MUTED));
        if let Some(deadline) = self.close_at {
            let remaining = deadline.saturating_duration_since(Instant::now()).as_secs() + 1;
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(text(format!("Esta janela fecha em {remaining} s para liberar memória. O Modo Jogo continua ativo."), regular(13.0), TEXT_MUTED));
                if link(ui, "Manter aberta", RED).clicked() {
                    action = Some(GameAction::KeepOpen);
                }
            });
        }
        ui.add_space(28.0);
        let height = ui.available_height() - 90.0;
        section_label(ui, "O que foi feito");
        rule(ui);
        egui::ScrollArea::vertical().id_salt("activation").max_height(height).show(ui, |ui| {
            if self.activation_items.is_empty() {
                ui.add_space(10.0);
                ui.label(text("Sessão retomada depois que o app foi reaberto.", regular(14.0), TEXT_MUTED));
            }
            for item in &self.activation_items {
                item_row(ui, item, 196.0);
            }
        });
        ui.add_space(20.0);
        if secondary_button(ui, "Encerrar Modo Jogo", 240.0).clicked() {
            action = Some(GameAction::Deactivate);
        }
        action
    }

    fn game_finished(&mut self, ui: &mut Ui) -> Option<GameAction> {
        heading(ui, "Modo Jogo", "Encerrado. O computador voltou ao normal.");
        section_label(ui, "Ao encerrar");
        rule(ui);
        if self.deactivation_items.is_empty() {
            ui.add_space(10.0);
            ui.label(text("Nada precisou ser restaurado.", regular(14.0), TEXT_MUTED));
            ui.add_space(10.0);
        }
        for item in &self.deactivation_items {
            item_row(ui, item, 196.0);
        }
        ui.add_space(24.0);
        primary_button(ui, "Voltar", 200.0, true).clicked().then_some(GameAction::Back)
    }

    fn cleanup_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        heading(ui, "Limpeza", "Analise o que pode ser removido e marque o que limpar. Nada é apagado sem você confirmar.");
        let action = match self.cleanup_phase {
            CleanupPhase::Idle => {
                ui.label(text("Arquivos temporários, caches de programas e restos de instalações.", regular(14.0), TEXT_MUTED));
                ui.add_space(20.0);
                primary_button(ui, "Analisar", 200.0, true).clicked().then_some(CleanupAction::Analyze)
            }
            CleanupPhase::Analyzing => {
                loading(ui, &format!("Analisando {}…", self.cleanup_progress.to_lowercase()));
                None
            }
            CleanupPhase::MeasuringSystem => {
                loading(ui, "Aguardando a permissão de administrador…");
                None
            }
            CleanupPhase::Cleaning => {
                loading(ui, &format!("Limpando {}…", self.cleanup_progress.to_lowercase()));
                None
            }
            CleanupPhase::Analyzed => self.cleanup_list(ui),
            CleanupPhase::Done => self.cleanup_results(ui),
        };
        match action {
            Some(CleanupAction::Analyze) => {
                self.blocker_items.clear();
                self.analyze(ctx);
            }
            Some(CleanupAction::MeasureSystem) => self.measure_system(ctx),
            Some(CleanupAction::Clean) => self.clean(ctx),
            Some(CleanupAction::CloseBlockers(id)) => self.close_blockers(ctx, &id),
            None => {}
        }
    }

    fn cleanup_list(&mut self, ui: &mut Ui) -> Option<CleanupAction> {
        let mut action = None;
        let found: u64 = self.analyses.iter().map(Analysis::total).sum();
        let admin_pending = self.analyses.iter().any(Analysis::admin_pending);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                section_label(ui, "Encontrado");
                ui.label(text(storage::format_size(found), light(44.0), TEXT));
            });
            ui.with_layout(Layout::right_to_left(Align::Max), |ui| {
                if link(ui, "Analisar de novo", TEXT_MUTED).clicked() {
                    action = Some(CleanupAction::Analyze);
                }
                if admin_pending {
                    ui.add_space(16.0);
                    if link(ui, "Medir itens do sistema", RED).on_hover_text("Pede permissão de administrador").clicked() {
                        action = Some(CleanupAction::MeasureSystem);
                    }
                }
            });
        });
        if let Some(warning) = &self.cleanup_warning {
            ui.label(text(warning, semibold(13.0), RED));
        }
        for item in &self.blocker_items {
            ui.label(text(format!("{}: {}", item.label, item.value), regular(13.0), status_color(item.status)));
        }
        ui.add_space(14.0);
        let height = ui.available_height() - 104.0;
        let width = ui.available_width();
        egui::ScrollArea::vertical().id_salt("categories").max_height(height).show(ui, |ui| {
            ui.set_width(width);
            for group in Group::ALL {
                let categories: Vec<&Category> = self
                    .catalog
                    .categories
                    .iter()
                    .filter(|c| c.group == group && self.analyses.iter().any(|a| a.id == c.id))
                    .collect();
                if categories.is_empty() {
                    continue;
                }
                section_label(ui, group.label());
                rule(ui);
                for category in categories {
                    let Some(analysis) = self.analyses.iter().find(|a| a.id == category.id) else { continue };
                    if let Some(a) = category_row(ui, category, analysis, &mut self.selection) {
                        action = Some(a);
                    }
                }
                ui.add_space(22.0);
            }
        });
        rule(ui);
        ui.add_space(14.0);
        let selected: u64 = self.analyses.iter().filter(|a| self.selection.contains(&a.id)).map(Analysis::total).sum();
        let selected_has_unknown = self.analyses.iter().any(|a| self.selection.contains(&a.id) && (a.unknown_size || a.admin_pending()));
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                section_label(ui, "Selecionado");
                let extra = if selected_has_unknown { " + sistema" } else { "" };
                ui.label(text(format!("{}{extra}", storage::format_size(selected)), semibold(24.0), TEXT));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if primary_button(ui, "Limpar selecionados", 240.0, !self.selection.is_empty()).clicked() {
                    action = Some(CleanupAction::Clean);
                }
            });
        });
        action
    }

    fn cleanup_results(&mut self, ui: &mut Ui) -> Option<CleanupAction> {
        let freed: u64 = self.results.iter().map(|r| r.tally.bytes).sum();
        section_label(ui, "Liberado");
        ui.label(text(storage::format_size(freed), light(72.0), TEXT));
        if let Some(warning) = &self.cleanup_warning {
            ui.label(text(warning, semibold(13.0), RED));
        }
        ui.add_space(20.0);
        let height = ui.available_height() - 90.0;
        rule(ui);
        egui::ScrollArea::vertical().id_salt("results").max_height(height).show(ui, |ui| {
            for result in &self.results {
                let name = self.catalog.categories.iter().find(|c| c.id == result.id).map(|c| c.name.clone()).unwrap_or_default();
                item_row(ui, &result_item(name, result), 260.0);
            }
        });
        ui.add_space(20.0);
        secondary_button(ui, "Analisar de novo", 220.0).clicked().then_some(CleanupAction::Analyze)
    }

    fn history_page(&mut self, ui: &mut Ui) {
        heading(ui, "Histórico", "Limpezas feitas e sessões de Modo Jogo.");
        if self.history.is_empty() {
            ui.label(text("Nada por aqui ainda.", regular(14.0), TEXT_MUTED));
            return;
        }
        rule(ui);
        let width = ui.available_width();
        egui::ScrollArea::vertical().id_salt("history").show(ui, |ui| {
            ui.set_width(width);
            for entry in &self.history {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    column(ui, 150.0, text(&entry.date, regular(13.0), TEXT_MUTED));
                    column(ui, 120.0, text(entry.kind.label(), semibold(14.0), TEXT));
                    ui.label(text(&entry.summary, regular(13.0), TEXT_MUTED));
                    if entry.freed > 0 {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(text(storage::format_size(entry.freed), semibold(14.0), TEXT));
                        });
                    }
                });
                ui.add_space(10.0);
                rule(ui);
            }
        });
    }

    /// When the watcher ends the session while the window is open, shows the summary it left.
    fn follow_watcher(&mut self) {
        if storage::session_active() {
            return;
        }
        if let Some(items) = storage::take_last_summary::<Vec<Item>>() {
            self.stop_monitor();
            self.close_at = None;
            self.deactivation_items = items;
            self.session = None;
            self.game_phase = GamePhase::Finished;
            self.history = storage::load_history();
        }
    }
}

/// Used when the watcher claimed the session first: waits up to 30 s for its summary.
fn wait_for_watcher_summary() -> Vec<Item> {
    for _ in 0..60 {
        if let Some(items) = storage::take_last_summary::<Vec<Item>>() {
            return items;
        }
        thread::sleep(Duration::from_millis(500));
    }
    Vec::new()
}

impl eframe::App for App {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop_monitor();
        if storage::session_active() && self.session.as_ref().is_some_and(|s| s.game_folder.is_some()) {
            watcher::start();
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.receive(ctx);
        if self.game_phase == GamePhase::Active {
            ctx.request_repaint_after(Duration::from_secs(1));
            self.follow_watcher();
            if self.close_at.is_some_and(|deadline| Instant::now() >= deadline) {
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
        egui::TopBottomPanel::top("title_bar")
            .exact_height(TITLE_BAR_HEIGHT)
            .show_separator_line(false)
            .frame(Frame::new().fill(BACKGROUND))
            .show(ctx, |ui| self.title_bar(ui, ctx));
        egui::SidePanel::left("navigation")
            .exact_width(NAV_WIDTH)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(BACKGROUND).inner_margin(Margin { left: 40, right: 16, top: 4, bottom: 32 }))
            .show(ctx, |ui| self.navigation(ui));
        egui::CentralPanel::default()
            .frame(Frame::new().fill(BACKGROUND).inner_margin(Margin { left: 48, right: 56, top: 4, bottom: 32 }))
            .show(ctx, |ui| match self.page {
                Page::GameMode => self.game_mode_page(ui, ctx),
                Page::Cleanup => self.cleanup_page(ui, ctx),
                Page::History => self.history_page(ui),
            });
        let screen = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("borders")));
        painter.rect_stroke(screen, CornerRadius::ZERO, stroke(1.0, RULE), StrokeKind::Inside);
        let x = screen.left() + NAV_WIDTH;
        painter.line_segment([Pos2::new(x, screen.top() + TITLE_BAR_HEIGHT), Pos2::new(x, screen.bottom() - 32.0)], stroke(1.0, RULE));
    }
}

fn nav_item(ui: &mut Ui, number: u32, name: &str, active: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::CLICK);
    let hovered = response.hovered();
    let painter = ui.painter();
    let number_color = if active { RED } else { TEXT_FAINT };
    painter.text(Pos2::new(rect.left(), rect.center().y), Align2::LEFT_CENTER, format!("{number:02}"), semibold(12.0), number_color);
    let (font, color) = match (active, hovered) {
        (true, _) => (semibold(16.0), TEXT),
        (false, true) => (regular(16.0), TEXT),
        (false, false) => (regular(16.0), TEXT_MUTED),
    };
    painter.text(Pos2::new(rect.left() + 34.0, rect.center().y), Align2::LEFT_CENTER, name, font, color);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

fn category_row(ui: &mut Ui, category: &Category, analysis: &Analysis, selection: &mut HashSet<String>) -> Option<CleanupAction> {
    let mut action = None;
    let blocked = !analysis.blockers.is_empty();
    ui.add_space(12.0);
    ui.horizontal_top(|ui| {
        let mut checked = selection.contains(&category.id);
        if bare_checkbox(ui, &mut checked, !blocked).changed() {
            match checked {
                true => selection.insert(category.id.clone()),
                false => selection.remove(&category.id),
            };
        }
        ui.add_space(10.0);
        let text_width = ui.available_width() - 120.0;
        ui.vertical(|ui| {
            ui.set_width(text_width);
            ui.horizontal(|ui| {
                ui.label(text(&category.name, semibold(14.5), if blocked { TEXT_FAINT } else { TEXT }));
                if analysis.has_admin {
                    tag(ui, "Admin", TEXT_MUTED);
                }
                if category.slow {
                    tag(ui, "Demorado", TEXT_MUTED);
                }
                if category.group == Group::Caution {
                    tag(ui, "Atenção", RED);
                }
            });
            ui.add(egui::Label::new(text(&category.description, regular(12.5), TEXT_MUTED)).wrap());
            if !analysis.details.is_empty() {
                ui.add(egui::Label::new(text(analysis.details.join("  ·  "), regular(12.0), TEXT_MUTED)).wrap());
            }
            if blocked {
                ui.horizontal(|ui| {
                    ui.label(text(format!("Feche {} para limpar.", analysis.blockers.join(", ")), semibold(12.5), RED));
                    if link(ui, "Fechar agora", RED).clicked() {
                        action = Some(CleanupAction::CloseBlockers(category.id.clone()));
                    }
                });
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            let (value, color) = match (analysis.total(), analysis.unknown_size || analysis.admin_pending()) {
                (0, true) => ("—".to_string(), TEXT_FAINT),
                (bytes, true) => (format!("{}+", storage::format_size(bytes)), TEXT),
                (bytes, false) => (storage::format_size(bytes), TEXT),
            };
            ui.label(text(value, semibold(14.5), color));
        });
    });
    ui.add_space(12.0);
    rule(ui);
    action
}

fn result_item(name: String, result: &CleanupResult) -> Item {
    let value = match result.tally.bytes {
        0 => "nada removido".to_string(),
        bytes => storage::format_size(bytes),
    };
    let hint = match (&result.error, result.tally.skipped) {
        (Some(error), _) => Some(error.clone()),
        (None, 0) => None,
        (None, 1) => Some("1 item em uso foi mantido".to_string()),
        (None, n) => Some(format!("{n} itens em uso foram mantidos")),
    };
    let status = if result.error.is_some() { Status::Warning } else { Status::Ok };
    Item::new(name, value, status, hint)
}

fn shorten(value: &str, max: usize) -> String {
    match value.chars().count() > max {
        true => format!("{}…", value.chars().take(max).collect::<String>()),
        false => value.to_string(),
    }
}
