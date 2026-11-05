//! Main window: navigation, Game Mode, Cleanup, History and Settings pages.

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

use crate::admin;
use crate::cleanup::{self, Analysis, Catalog, Category, CleanupResult, Group};
use crate::game_mode::{self, Item, Status};
use crate::i18n::{self, Texts};
use crate::storage::{
    self, ChosenGame, Config, Force, HistoryEntry, HistoryKind, LanguageChoice, ProgramConfig, Session, Summary, ThemeChoice,
};
use crate::theme::{self, *};
use crate::{steam, system, watcher};

const NAV_WIDTH: f32 = 252.0;
const TITLE_BAR_HEIGHT: f32 = 40.0;
const AUTO_CLOSE_DELAY: Duration = Duration::from_secs(10);
const MONITOR_INTERVAL: Duration = Duration::from_secs(3);
const BRAND_FIRST_LINE: &str = "Modo Jogo";
const BRAND_SECOND_LINE: &str = "e Limpeza";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    GameMode,
    Cleanup,
    History,
    Settings,
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
    t: &'static Texts,
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
    custom_rules: cleanup::CustomRules,
    settings_warning: Option<String>,
    _window_marker: Option<watcher::Marker>,
    close_at: Option<Instant>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        let config = storage::load_config();
        theme::apply(&cc.egui_ctx, config.theme);
        let (tx, rx) = mpsc::channel();
        let session = storage::load_session();
        let mut app = App {
            page: Page::GameMode,
            t: i18n::texts_for(config.language),
            config,
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
            custom_rules: cleanup::CustomRules::default(),
            settings_warning: None,
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
        let t = self.t;
        self.spawn_task(ctx, move |tx, _| {
            let items = game_mode::overview(&config, t);
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
        self.game_progress = self.t.step_preparing.into();
        let config = self.config.clone();
        let t = self.t;
        let game = config.game.as_ref().and_then(|g| self.games.iter().find(|m| m.app_id == g.app_id).cloned());
        self.spawn_task(ctx, move |tx, ctx| {
            let progress = |step: &str| {
                let _ = tx.send(Message::GameProgress(step.into()));
                ctx.request_repaint();
            };
            let activation = game_mode::activate(&config, game.as_ref(), t, &progress);
            let _ = tx.send(Message::Activated(activation));
        });
    }

    fn deactivate(&mut self, ctx: &egui::Context) {
        self.stop_monitor();
        self.close_at = None;
        self.game_phase = GamePhase::Deactivating;
        let reopen = self.config.reopen_on_exit;
        let t = self.t;
        self.spawn_task(ctx, move |tx, _| {
            let items = match storage::claim_session() {
                Some(session) => game_mode::deactivate(&session, reopen, t),
                None => wait_for_watcher_summary(),
            };
            let _ = tx.send(Message::Deactivated(items));
        });
    }

    fn analyze(&mut self, ctx: &egui::Context) {
        self.cleanup_phase = CleanupPhase::Analyzing;
        self.cleanup_warning = None;
        self.catalog = cleanup::catalog();
        let t = self.t;
        self.spawn_task(ctx, move |tx, ctx| {
            let progress = |name: &str| {
                let _ = tx.send(Message::CleanupProgress(name.into()));
                ctx.request_repaint();
            };
            let _ = tx.send(Message::Analyzed(cleanup::analyze(t, &progress)));
        });
    }

    fn measure_system(&mut self, ctx: &egui::Context) {
        self.cleanup_phase = CleanupPhase::MeasuringSystem;
        let t = self.t;
        self.spawn_task(ctx, move |tx, _| {
            let result = admin::measure_system().map_err(|error| t.admin_error(error, t.permission_denied_measure));
            let _ = tx.send(Message::SystemMeasured(result));
        });
    }

    /// Remembers which analyzed categories were checked, so the next analysis starts the same way.
    fn remember_selection(&mut self) {
        for analysis in self.analyses.iter().filter(|a| a.blockers.is_empty()) {
            let remembers = self.catalog.categories.iter().find(|c| c.id == analysis.id).is_some_and(|c| c.remembers_choice());
            if remembers {
                self.config.cleanup_choices.insert(analysis.id.clone(), self.selection.contains(&analysis.id));
            }
        }
        self.save_config();
    }

    /// Cleans the user targets in this process, then the system targets through one elevated run.
    fn clean(&mut self, ctx: &egui::Context) {
        self.remember_selection();
        self.cleanup_phase = CleanupPhase::Cleaning;
        self.cleanup_warning = None;
        let t = self.t;
        let recycle_folders = self.config.recycle_folders;
        let selected: Vec<&Category> = self.catalog.categories.iter().filter(|c| self.selection.contains(&c.id)).collect();
        let ids: Vec<String> = selected.iter().map(|c| c.id.clone()).collect();
        let admin_ids: Vec<String> = selected.iter().filter(|c| c.targets.iter().any(|t| t.admin)).map(|c| c.id.clone()).collect();
        self.spawn_task(ctx, move |tx, ctx| {
            let progress = |name: &str| {
                let _ = tx.send(Message::CleanupProgress(name.into()));
                ctx.request_repaint();
            };
            let mut by_id: HashMap<String, CleanupResult> = cleanup::clean(&ids, false, recycle_folders, t, &progress).into_iter().map(|r| (r.id.clone(), r)).collect();
            let mut warning = None;
            if !admin_ids.is_empty() {
                progress(t.system_items_waiting);
                match admin::clean_system(&admin_ids, recycle_folders, t.language) {
                    Ok(admin_results) => {
                        for r in admin_results {
                            let entry = by_id.entry(r.id.clone()).or_insert_with(|| CleanupResult { id: r.id.clone(), ..Default::default() });
                            entry.tally.add(r.tally);
                            if r.error.is_some() {
                                entry.error = r.error;
                            }
                        }
                    }
                    Err(error) => warning = Some(t.admin_error(error, t.permission_denied_clean)),
                }
            }
            let results: Vec<CleanupResult> = ids.iter().filter_map(|id| by_id.remove(id)).collect();
            let freed: u64 = results.iter().map(|r| r.tally.bytes).sum();
            storage::add_history(HistoryKind::Cleanup, freed, Summary::Cleanup { categories: results.len() });
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
        self.cleanup_progress = self.t.open_programs.into();
        let t = self.t;
        self.spawn_task(ctx, move |tx, _| {
            let items = game_mode::close_programs(&programs).iter().map(|r| r.item(t)).collect();
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
                    let preselected: HashSet<String> = self
                        .catalog
                        .categories
                        .iter()
                        .filter(|c| c.initially_selected(&self.config.cleanup_choices))
                        .map(|c| c.id.clone())
                        .collect();
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
        let pal = palette(ctx);
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
            painter.rect_filled(minimize, CornerRadius::ZERO, pal.surface);
        }
        if close_response.hovered() {
            painter.rect_filled(close, CornerRadius::ZERO, pal.accent);
        }
        let c = minimize.center();
        painter.line_segment([c - Vec2::new(5.0, 0.0), c + Vec2::new(5.0, 0.0)], stroke(1.2, pal.text));
        let c = close.center();
        let color = if close_response.hovered() { pal.on_accent } else { pal.text };
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
        let pal = palette(ui.ctx());
        let t = self.t;
        ui.add_space(14.0);
        ui.label(text(BRAND_FIRST_LINE, semibold(24.0), pal.text));
        ui.label(text(BRAND_SECOND_LINE, semibold(24.0), pal.text_faint));
        ui.add_space(16.0);
        let (bar, _) = ui.allocate_exact_size(Vec2::new(28.0, 4.0), Sense::hover());
        ui.painter().rect_filled(bar, CornerRadius::ZERO, pal.accent);
        ui.add_space(48.0);
        let pages = [(1, Page::GameMode, t.game_mode), (2, Page::Cleanup, t.cleanup), (3, Page::History, t.history), (4, Page::Settings, t.settings)];
        for (number, page, name) in pages {
            if nav_item(ui, number, name, self.page == page).clicked() {
                self.page = page;
                match page {
                    Page::History => self.history = storage::load_history(),
                    Page::Settings => self.refresh_custom_rules(),
                    Page::GameMode | Page::Cleanup => {}
                }
            }
        }
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.label(text(format!("v{}", env!("CARGO_PKG_VERSION")), regular(11.0), pal.text_faint));
            ui.add_space(14.0);
            if let Some(free) = self.free_space_c {
                ui.label(text(t.free_space(&t.size(free)), semibold(18.0), pal.text));
                ui.label(text(t.drive_c, semibold(10.5), pal.text_muted).extra_letter_spacing(1.4));
            }
        });
    }

    fn game_mode_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = self.t;
        let action = match self.game_phase {
            GamePhase::Ready => self.game_ready(ui),
            GamePhase::Activating => {
                heading(ui, t.game_mode, t.preparing_subtitle);
                loading(ui, &format!("{}…", self.game_progress));
                None
            }
            GamePhase::Active => self.game_active(ui),
            GamePhase::Deactivating => {
                heading(ui, t.game_mode, t.restoring_subtitle);
                loading(ui, t.reopening_programs);
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
        let pal = palette(ui.ctx());
        let t = self.t;
        heading(ui, t.game_mode, t.game_mode_subtitle);
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
                    section_label(ui, t.status_section);
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        if link(ui, t.refresh, pal.text_muted).clicked() {
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
                            loading(ui, t.checking);
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
                    section_label(ui, t.close_programs_section);
                    rule(ui);
                    ui.add_space(8.0);
                    let mut remove = None;
                    for (index, program) in self.config.programs.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            changed |= checkbox(ui, &mut program.enabled, &program.name, true).changed();
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if link(ui, t.remove, pal.text_faint).clicked() {
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
                            if link(ui, t.add_open_program, pal.accent_text).clicked() {
                                action = Some(GameAction::FindCandidates);
                            }
                        }
                        Some(list) if list.is_empty() => {
                            ui.label(text(t.no_other_windows, regular(13.0), pal.text_muted));
                        }
                        Some(list) => {
                            ui.label(text(t.choose_program, regular(13.0), pal.text_muted));
                            for (process, window_title) in list {
                                let label = format!("+ {process}  ·  {}", shorten(window_title, 34));
                                if link(ui, &label, pal.accent_text).clicked() {
                                    action = Some(GameAction::Add(process.clone()));
                                }
                            }
                        }
                    }
                    ui.add_space(22.0);
                    section_label(ui, t.system_section);
                    rule(ui);
                    ui.add_space(8.0);
                    changed |= checkbox(ui, &mut self.config.shutdown_wsl, t.shutdown_wsl, true).changed();
                    changed |= checkbox(ui, &mut self.config.best_performance, t.best_performance, true).changed();
                    changed |= checkbox(ui, &mut self.config.reopen_on_exit, t.reopen_on_exit, true).changed();
                    ui.add_space(22.0);
                    section_label(ui, t.launch_game_section);
                    rule(ui);
                    ui.add_space(8.0);
                    if radio(ui, self.config.game.is_none(), t.no_game).clicked() {
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
        if primary_button(ui, t.activate, 240.0, !self.loading_overview).clicked() {
            action = Some(GameAction::Activate);
        }
        action
    }

    fn game_active(&mut self, ui: &mut Ui) -> Option<GameAction> {
        let pal = palette(ui.ctx());
        let t = self.t;
        let mut action = None;
        let Some(session) = &self.session else { return Some(GameAction::Back) };
        ui.label(text(t.game_mode, semibold(40.0), pal.text));
        ui.add_space(4.0);
        tag(ui, t.active_tag, pal.accent_text);
        ui.add_space(26.0);
        let elapsed = storage::now_epoch().saturating_sub(session.started_at);
        ui.label(text(storage::format_duration(elapsed), light(72.0), pal.text));
        let status_line = match (&session.game, self.game_running) {
            (Some(g), true) => t.game_running(&g.name),
            (Some(g), false) => t.waiting_for_game(&g.name),
            (None, _) => t.no_game_chosen.to_string(),
        };
        ui.label(text(status_line, regular(15.0), pal.text_muted));
        if let Some(deadline) = self.close_at {
            let remaining = deadline.saturating_duration_since(Instant::now()).as_secs() + 1;
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(text(t.window_closes_in(remaining), regular(13.0), pal.text_muted));
                if link(ui, t.keep_open, pal.accent_text).clicked() {
                    action = Some(GameAction::KeepOpen);
                }
            });
        }
        ui.add_space(28.0);
        let height = ui.available_height() - 90.0;
        section_label(ui, t.what_was_done);
        rule(ui);
        egui::ScrollArea::vertical().id_salt("activation").max_height(height).show(ui, |ui| {
            if self.activation_items.is_empty() {
                ui.add_space(10.0);
                ui.label(text(t.session_resumed, regular(14.0), pal.text_muted));
            }
            for item in &self.activation_items {
                item_row(ui, item, 196.0);
            }
        });
        ui.add_space(20.0);
        if secondary_button(ui, t.deactivate, 240.0).clicked() {
            action = Some(GameAction::Deactivate);
        }
        action
    }

    fn game_finished(&mut self, ui: &mut Ui) -> Option<GameAction> {
        let pal = palette(ui.ctx());
        let t = self.t;
        heading(ui, t.game_mode, t.finished_subtitle);
        section_label(ui, t.on_exit_section);
        rule(ui);
        if self.deactivation_items.is_empty() {
            ui.add_space(10.0);
            ui.label(text(t.nothing_restored, regular(14.0), pal.text_muted));
            ui.add_space(10.0);
        }
        for item in &self.deactivation_items {
            item_row(ui, item, 196.0);
        }
        ui.add_space(24.0);
        primary_button(ui, t.back, 200.0, true).clicked().then_some(GameAction::Back)
    }

    fn cleanup_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let pal = palette(ui.ctx());
        let t = self.t;
        heading(ui, t.cleanup, t.cleanup_subtitle);
        let action = match self.cleanup_phase {
            CleanupPhase::Idle => {
                ui.label(text(t.cleanup_intro, regular(14.0), pal.text_muted));
                ui.add_space(20.0);
                primary_button(ui, t.analyze, 200.0, true).clicked().then_some(CleanupAction::Analyze)
            }
            CleanupPhase::Analyzing => {
                loading(ui, &t.analyzing(&self.cleanup_progress.to_lowercase()));
                None
            }
            CleanupPhase::MeasuringSystem => {
                loading(ui, t.waiting_for_admin);
                None
            }
            CleanupPhase::Cleaning => {
                loading(ui, &t.cleaning(&self.cleanup_progress.to_lowercase()));
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
        let pal = palette(ui.ctx());
        let t = self.t;
        let mut action = None;
        let found: u64 = self.analyses.iter().map(Analysis::total).sum();
        let admin_pending = self.analyses.iter().any(Analysis::admin_pending);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                section_label(ui, t.found);
                ui.label(text(t.size(found), light(44.0), pal.text));
            });
            ui.with_layout(Layout::right_to_left(Align::Max), |ui| {
                if link(ui, t.analyze_again, pal.text_muted).clicked() {
                    action = Some(CleanupAction::Analyze);
                }
                if admin_pending {
                    ui.add_space(16.0);
                    if link(ui, t.measure_system, pal.accent_text).on_hover_text(t.asks_for_admin).clicked() {
                        action = Some(CleanupAction::MeasureSystem);
                    }
                }
            });
        });
        if let Some(warning) = &self.cleanup_warning {
            ui.label(text(warning, semibold(13.0), pal.accent_text));
        }
        for item in &self.blocker_items {
            ui.label(text(format!("{}: {}", item.label, item.value), regular(13.0), status_color(pal, item.status)));
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
                section_label(ui, t.group(group));
                rule(ui);
                for category in categories {
                    let Some(analysis) = self.analyses.iter().find(|a| a.id == category.id) else { continue };
                    if let Some(a) = category_row(ui, t, category, analysis, &mut self.selection) {
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
                section_label(ui, t.selected);
                let extra = if selected_has_unknown { t.plus_system } else { "" };
                ui.label(text(format!("{}{extra}", t.size(selected)), semibold(24.0), pal.text));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if primary_button(ui, t.clean_selected, 240.0, !self.selection.is_empty()).clicked() {
                    action = Some(CleanupAction::Clean);
                }
            });
        });
        action
    }

    fn cleanup_results(&mut self, ui: &mut Ui) -> Option<CleanupAction> {
        let pal = palette(ui.ctx());
        let t = self.t;
        let freed: u64 = self.results.iter().map(|r| r.tally.bytes).sum();
        let recycled: u64 = self.results.iter().map(|r| r.tally.recycled).sum();
        section_label(ui, t.freed);
        ui.label(text(t.size(freed), light(72.0), pal.text));
        if recycled > 0 {
            ui.label(text(format!("+ {}", t.in_recycle_bin(&t.size(recycled))), semibold(15.0), pal.text_muted));
        }
        if let Some(warning) = &self.cleanup_warning {
            ui.label(text(warning, semibold(13.0), pal.accent_text));
        }
        ui.add_space(20.0);
        let height = ui.available_height() - 90.0;
        rule(ui);
        egui::ScrollArea::vertical().id_salt("results").max_height(height).show(ui, |ui| {
            for result in &self.results {
                let name = self.catalog.categories.iter().find(|c| c.id == result.id).map(|c| c.name.get(t.language).to_string()).unwrap_or_default();
                item_row(ui, &result_item(t, name, result), 260.0);
            }
        });
        ui.add_space(20.0);
        secondary_button(ui, t.analyze_again, 220.0).clicked().then_some(CleanupAction::Analyze)
    }

    fn history_page(&mut self, ui: &mut Ui) {
        let pal = palette(ui.ctx());
        let t = self.t;
        heading(ui, t.history, t.history_subtitle);
        if self.history.is_empty() {
            ui.label(text(t.history_empty, regular(14.0), pal.text_muted));
            return;
        }
        rule(ui);
        let width = ui.available_width();
        egui::ScrollArea::vertical().id_salt("history").show(ui, |ui| {
            ui.set_width(width);
            for entry in &self.history {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    column(ui, 150.0, text(t.history_date(&entry.date), regular(13.0), pal.text_muted));
                    column(ui, 120.0, text(t.history_kind(entry.kind), semibold(14.0), pal.text));
                    ui.label(text(t.history_summary(&entry.summary), regular(13.0), pal.text_muted));
                    if entry.freed > 0 {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(text(t.size(entry.freed), semibold(14.0), pal.text));
                        });
                    }
                });
                ui.add_space(10.0);
                rule(ui);
            }
        });
    }

    fn settings_page(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let t = self.t;
        heading(ui, t.settings, t.settings_subtitle);
        section_label(ui, t.theme_section);
        rule(ui);
        ui.add_space(8.0);
        for (choice, label) in [(ThemeChoice::System, t.follow_windows), (ThemeChoice::Light, t.theme_light), (ThemeChoice::Dark, t.theme_dark)] {
            if radio(ui, self.config.theme == choice, label).clicked() && self.config.theme != choice {
                self.config.theme = choice;
                theme::set_theme(ctx, choice);
                self.save_config();
            }
        }
        ui.add_space(22.0);
        section_label(ui, t.language_section);
        rule(ui);
        ui.add_space(8.0);
        let languages = [(LanguageChoice::System, t.follow_windows), (LanguageChoice::PortugueseBrazil, "Português (Brasil)"), (LanguageChoice::English, "English")];
        for (choice, label) in languages {
            if radio(ui, self.config.language == choice, label).clicked() && self.config.language != choice {
                self.config.language = choice;
                self.t = i18n::texts_for(choice);
                self.save_config();
                self.refresh_overview(ctx);
            }
        }
        ui.add_space(22.0);
        section_label(ui, t.cleanup);
        rule(ui);
        ui.add_space(8.0);
        self.cleanup_settings(ui);
    }

    fn refresh_custom_rules(&mut self) {
        self.custom_rules = cleanup::custom_rules(&cleanup::builtin_catalog());
    }

    /// Opens the custom rules in Notepad, creating the file with an example the first time.
    fn open_custom_rules(&mut self) {
        let t = self.t;
        self.settings_warning = match cleanup::ensure_custom_rules_file() {
            Ok(path) if system::open_in_notepad(&path) => None,
            Ok(path) => Some(t.could_not_open(&path.display().to_string())),
            Err(e) => Some(t.could_not_open(&e.to_string())),
        };
        self.refresh_custom_rules();
    }

    fn cleanup_settings(&mut self, ui: &mut Ui) {
        let pal = palette(ui.ctx());
        let t = self.t;
        if checkbox(ui, &mut self.config.recycle_folders, t.recycle_folders, true).changed() {
            self.save_config();
        }
        ui.label(text(format!("→ {}", t.recycle_folders_hint), regular(12.5), pal.text_muted));
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(text(t.selection_remembered, regular(13.0), pal.text_muted));
            if !self.config.cleanup_choices.is_empty() && link(ui, t.reset_selection, pal.accent_text).clicked() {
                self.config.cleanup_choices.clear();
                self.save_config();
            }
        });
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.label(text(t.custom_rules_count(self.custom_rules.categories.len()), semibold(14.0), pal.text));
            if link(ui, t.refresh, pal.text_muted).clicked() {
                self.refresh_custom_rules();
            }
        });
        ui.add(egui::Label::new(text(t.custom_rules_hint, regular(12.5), pal.text_muted)).wrap());
        for problem in &self.custom_rules.problems {
            ui.add(egui::Label::new(text(t.custom_problem(problem), semibold(12.5), pal.accent_text)).wrap());
        }
        if let Some(warning) = &self.settings_warning {
            ui.add(egui::Label::new(text(warning, semibold(12.5), pal.accent_text)).wrap());
        }
        ui.add_space(4.0);
        if link(ui, t.open_rules_file, pal.accent_text).clicked() {
            self.open_custom_rules();
        }
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
        let pal = palette(ctx);
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
            .frame(Frame::new().fill(pal.background))
            .show(ctx, |ui| self.title_bar(ui, ctx));
        egui::SidePanel::left("navigation")
            .exact_width(NAV_WIDTH)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::new().fill(pal.background).inner_margin(Margin { left: 40, right: 16, top: 4, bottom: 32 }))
            .show(ctx, |ui| self.navigation(ui));
        egui::CentralPanel::default()
            .frame(Frame::new().fill(pal.background).inner_margin(Margin { left: 48, right: 56, top: 4, bottom: 32 }))
            .show(ctx, |ui| match self.page {
                Page::GameMode => self.game_mode_page(ui, ctx),
                Page::Cleanup => self.cleanup_page(ui, ctx),
                Page::History => self.history_page(ui),
                Page::Settings => self.settings_page(ui, ctx),
            });
        let screen = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("borders")));
        painter.rect_stroke(screen, CornerRadius::ZERO, stroke(1.0, pal.rule), StrokeKind::Inside);
        let x = screen.left() + NAV_WIDTH;
        painter.line_segment([Pos2::new(x, screen.top() + TITLE_BAR_HEIGHT), Pos2::new(x, screen.bottom() - 32.0)], stroke(1.0, pal.rule));
    }
}

fn nav_item(ui: &mut Ui, number: u32, name: &str, active: bool) -> Response {
    let pal = palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::CLICK);
    let hovered = response.hovered();
    let painter = ui.painter();
    let number_color = if active { pal.accent_text } else { pal.text_faint };
    painter.text(Pos2::new(rect.left(), rect.center().y), Align2::LEFT_CENTER, format!("{number:02}"), semibold(12.0), number_color);
    let (font, color) = match (active, hovered) {
        (true, _) => (semibold(16.0), pal.text),
        (false, true) => (regular(16.0), pal.text),
        (false, false) => (regular(16.0), pal.text_muted),
    };
    painter.text(Pos2::new(rect.left() + 34.0, rect.center().y), Align2::LEFT_CENTER, name, font, color);
    response.on_hover_cursor(CursorIcon::PointingHand)
}

fn category_row(ui: &mut Ui, t: &Texts, category: &Category, analysis: &Analysis, selection: &mut HashSet<String>) -> Option<CleanupAction> {
    let pal = palette(ui.ctx());
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
                ui.label(text(category.name.get(t.language), semibold(14.5), if blocked { pal.text_faint } else { pal.text }));
                if analysis.has_admin {
                    tag(ui, t.tag_admin, pal.text_muted);
                }
                if category.slow {
                    tag(ui, t.tag_slow, pal.text_muted);
                }
                if category.group == Group::Caution {
                    tag(ui, t.tag_caution, pal.accent_text);
                }
            });
            ui.add(egui::Label::new(text(category.description.get(t.language), regular(12.5), pal.text_muted)).wrap());
            if !analysis.details.is_empty() {
                ui.add(egui::Label::new(text(analysis.details.join("  ·  "), regular(12.0), pal.text_muted)).wrap());
            }
            if blocked {
                ui.horizontal(|ui| {
                    ui.label(text(t.close_to_clean(&analysis.blockers.join(", ")), semibold(12.5), pal.accent_text));
                    if link(ui, t.close_now, pal.accent_text).clicked() {
                        action = Some(CleanupAction::CloseBlockers(category.id.clone()));
                    }
                });
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            let (value, color) = match (analysis.total(), analysis.unknown_size || analysis.admin_pending()) {
                (0, true) => ("—".to_string(), pal.text_faint),
                (bytes, true) => (format!("{}+", t.size(bytes)), pal.text),
                (bytes, false) => (t.size(bytes), pal.text),
            };
            ui.label(text(value, semibold(14.5), color));
        });
    });
    ui.add_space(12.0);
    rule(ui);
    action
}

fn result_item(t: &Texts, name: String, result: &CleanupResult) -> Item {
    let value = match (result.tally.bytes, result.tally.recycled) {
        (0, 0) => t.nothing_removed.to_string(),
        (0, recycled) => t.in_recycle_bin(&t.size(recycled)),
        (bytes, 0) => t.size(bytes),
        (bytes, recycled) => format!("{} + {}", t.size(bytes), t.in_recycle_bin(&t.size(recycled))),
    };
    let hint = match (&result.error, result.tally.skipped) {
        (Some(error), _) => Some(error.clone()),
        (None, 0) => None,
        (None, n) => Some(t.kept_in_use(n)),
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
