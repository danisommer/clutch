//! Game Mode: computer status, safe program closing, activation and restore.

use std::collections::HashSet;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::storage::{self, ChosenGame, ClosedProgram, Config, Force, HistoryKind, ProgramConfig, Session};
use crate::steam;
use crate::system;

const CLOSE_TIMEOUT: Duration = Duration::from_secs(20);
const WSL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const MIN_FREE_MEMORY: u64 = 5 * 1024 * 1024 * 1024;
const LENOVO_KEY: &str = r"SYSTEM\CurrentControlSet\Services\LITSSVC\LNBITS\IC\MMC";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Ok,
    Warning,
    Neutral,
}

/// One "label — value" line shown to the user, with an optional suggestion below it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub label: String,
    pub value: String,
    pub status: Status,
    pub hint: Option<String>,
}

impl Item {
    pub fn new(label: impl Into<String>, value: impl Into<String>, status: Status, hint: Option<String>) -> Self {
        Item { label: label.into(), value: value.into(), status, hint }
    }
    fn ok(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self::new(label, value, Status::Ok, None)
    }
    fn neutral(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self::new(label, value, Status::Neutral, None)
    }
    fn warning(label: impl Into<String>, value: impl Into<String>, hint: impl Into<String>) -> Self {
        Self::new(label, value, Status::Warning, Some(hint.into()))
    }
    fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseOutcome {
    Closed,
    ClosedInBackground,
    Failed,
    StillOpen,
}

#[derive(Clone, Debug)]
pub struct CloseResult {
    pub name: String,
    pub outcome: CloseOutcome,
    pub record: ClosedProgram,
    pub had_window: bool,
}

impl CloseResult {
    pub fn item(&self) -> Item {
        match self.outcome {
            CloseOutcome::Closed => Item::ok(&self.name, "fechado"),
            CloseOutcome::ClosedInBackground => Item::ok(&self.name, "fechado")
                .with_hint(format!("rodava em segundo plano; desative isso nas configurações do {}", self.name)),
            CloseOutcome::Failed => Item::warning(&self.name, "não foi possível fechar", "feche manualmente"),
            CloseOutcome::StillOpen => Item::warning(&self.name, "continua aberto", "pode haver algo não salvo; feche manualmente"),
        }
    }

    pub fn closed(&self) -> bool {
        matches!(self.outcome, CloseOutcome::Closed | CloseOutcome::ClosedInBackground)
    }
}

fn pids(processes: &[system::Process]) -> Vec<u32> {
    processes.iter().map(|p| p.pid).collect()
}

fn record(name: &str, processes: &[system::Process]) -> (ClosedProgram, bool) {
    let with_window = processes.iter().find(|p| system::window_count(&[p.pid]) > 0);
    let main = with_window.or(processes.first());
    let record = ClosedProgram {
        name: name.to_string(),
        path: main.and_then(|p| system::process_path(p.pid)),
        aumid: main.and_then(|p| system::process_aumid(p.pid)),
    };
    (record, with_window.is_some())
}

fn remaining(program: &ProgramConfig, exclude: &HashSet<u32>) -> Vec<system::Process> {
    system::processes_named(&system::processes(), &program.processes, exclude)
}

fn terminate_all(program: &ProgramConfig, exclude: &HashSet<u32>, open: &[system::Process], on_success: CloseOutcome) -> CloseOutcome {
    for p in open {
        system::terminate_process(p.pid);
    }
    match remaining(program, exclude).is_empty() {
        true => on_success,
        false => CloseOutcome::Failed,
    }
}

fn resolve(program: &ProgramConfig, exclude: &HashSet<u32>) -> CloseOutcome {
    let open = remaining(program, exclude);
    if open.is_empty() {
        return CloseOutcome::Closed;
    }
    match program.force {
        Force::Always => terminate_all(program, exclude, &open, CloseOutcome::Closed),
        Force::WhenWindowless if system::window_count(&pids(&open)) == 0 => {
            terminate_all(program, exclude, &open, CloseOutcome::ClosedInBackground)
        }
        Force::WhenWindowless | Force::Never => CloseOutcome::StillOpen,
    }
}

/// Closes programs through their windows and terminates them only when their `Force` policy allows.
pub fn close_programs(programs: &[ProgramConfig]) -> Vec<CloseResult> {
    let all = system::processes();
    let exclude = system::self_and_ancestors(&all);
    let open: Vec<(&ProgramConfig, Vec<system::Process>)> = programs
        .iter()
        .map(|p| (p, system::processes_named(&all, &p.processes, &exclude)))
        .filter(|(_, procs)| !procs.is_empty())
        .collect();

    let records: Vec<(ClosedProgram, bool)> = open.iter().map(|(p, procs)| record(&p.name, procs)).collect();
    for (_, procs) in &open {
        system::request_close(&pids(procs));
    }

    let deadline = Instant::now() + CLOSE_TIMEOUT;
    while Instant::now() < deadline && open.iter().any(|(p, _)| !remaining(p, &exclude).is_empty()) {
        thread::sleep(Duration::from_millis(500));
    }

    open.iter()
        .zip(records)
        .map(|((program, _), (record, had_window))| CloseResult {
            name: program.name.clone(),
            outcome: resolve(program, &exclude),
            record,
            had_window,
        })
        .collect()
}

pub fn open_programs(config: &Config) -> Vec<String> {
    let all = system::processes();
    let exclude = system::self_and_ancestors(&all);
    config
        .programs
        .iter()
        .filter(|p| p.enabled && !system::processes_named(&all, &p.processes, &exclude).is_empty())
        .map(|p| p.name.clone())
        .collect()
}

fn wsl_item() -> Item {
    match system::wsl_running() {
        true => Item::neutral("WSL", "ligada"),
        false => Item::ok("WSL", "desligada"),
    }
}

fn charger_item() -> Option<Item> {
    system::on_ac_power().map(|plugged_in| match plugged_in {
        true => Item::ok("Carregador", "conectado"),
        false => Item::warning("Carregador", "desconectado", "conecte o carregador"),
    })
}

fn power_item(will_adjust: bool) -> Item {
    let chosen = system::chosen_power_mode();
    let effective = system::effective_power_mode();
    match (chosen, effective) {
        (_, Some(e)) if system::guid_eq(&e, &system::BEST_PERFORMANCE) => Item::ok("Modo de energia", "Melhor desempenho"),
        (Some(c), _) if system::guid_eq(&c, &system::BEST_PERFORMANCE) => {
            Item::warning("Modo de energia", "limitado pela Economia de energia", "desative a Economia de energia (Win+A)")
        }
        (_, Some(e)) if will_adjust => Item::neutral("Modo de energia", system::power_mode_name(&e)),
        (_, Some(e)) => Item::warning("Modo de energia", system::power_mode_name(&e), "não foi possível ativar Melhor desempenho"),
        (_, None) => Item::neutral("Modo de energia", "não identificado"),
    }
}

/// Thermal mode of Lenovo laptops, as set in Lenovo Vantage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LenovoMode {
    Adaptive,
    Performance,
    BatterySaver,
    Unknown,
}

impl LenovoMode {
    fn label(self) -> &'static str {
        match self {
            LenovoMode::Adaptive => "Energia adaptável",
            LenovoMode::Performance => "Desempenho",
            LenovoMode::BatterySaver => "Economia de bateria",
            LenovoMode::Unknown => "não identificado",
        }
    }
}

/// `None` when the computer does not have Lenovo's thermal service.
fn lenovo_mode() -> Option<LenovoMode> {
    if !system::registry_key_exists(true, LENOVO_KEY) {
        return None;
    }
    let automatic = system::registry_u32(true, LENOVO_KEY, "AutomaticModeSetting");
    let current = system::registry_u32(true, LENOVO_KEY, "CurrentSetting");
    Some(match (automatic, current) {
        (Some(2), _) => LenovoMode::Adaptive,
        (_, Some(3)) => LenovoMode::Performance,
        (_, Some(1)) => LenovoMode::BatterySaver,
        _ => LenovoMode::Unknown,
    })
}

fn lenovo_item() -> Option<Item> {
    lenovo_mode().map(|mode| match mode {
        LenovoMode::Performance => Item::ok("Modo Lenovo", mode.label()),
        LenovoMode::Unknown => Item::neutral("Modo Lenovo", mode.label()).with_hint("confira no Lenovo Vantage"),
        LenovoMode::Adaptive | LenovoMode::BatterySaver => Item::warning("Modo Lenovo", mode.label(), "selecione Desempenho no Lenovo Vantage"),
    })
}

fn memory_item(after_closing: bool) -> Item {
    let available = system::available_memory();
    let value = storage::format_size(available);
    match (available >= MIN_FREE_MEMORY, after_closing) {
        (true, _) => Item::ok("Memória disponível", value),
        (false, true) => Item::warning("Memória disponível", value, "pouca memória; feche outros programas"),
        (false, false) => Item::neutral("Memória disponível", value),
    }
}

fn screen_items() -> Vec<Item> {
    let screens = system::screens();
    let mut items: Vec<Item> = screens
        .iter()
        .map(|s| {
            let label = match (s.internal, s.name.trim()) {
                (true, _) => "Tela do notebook".to_string(),
                (false, "") => "Monitor externo".to_string(),
                (false, name) => name.to_string(),
            };
            let value = match s.primary && screens.len() > 1 {
                true => format!("{}x{}, {} Hz · principal", s.width, s.height, s.hz),
                false => format!("{}x{}, {} Hz", s.width, s.height, s.hz),
            };
            match s.hz < s.max_hz {
                true => Item::warning(label, value, format!("suporta {} Hz: Configurações > Sistema > Tela > Tela avançada", s.max_hz)),
                false => Item::ok(label, value),
            }
        })
        .collect();
    if screens.len() > 1 {
        if let Some((item, _)) = items.iter_mut().zip(&screens).find(|(i, s)| !s.internal && i.hint.is_none()) {
            item.hint = Some("para jogar só nele: Win+P > Somente segunda tela".into());
        }
    }
    items
}

fn hwinfo_item() -> Option<Item> {
    system::processes()
        .iter()
        .any(|p| p.name.eq_ignore_ascii_case("HWiNFO64"))
        .then(|| Item::neutral("HWiNFO", "aberto").with_hint("feche se não estiver monitorando"))
}

/// Current state of the computer, before Game Mode is activated.
pub fn overview(config: &Config) -> Vec<Item> {
    let open = open_programs(config);
    let mut items = vec![match open.is_empty() {
        true => Item::ok("Programas pesados", "nenhum aberto"),
        false => Item::neutral("Programas pesados", open.join(", ")),
    }];
    items.push(wsl_item());
    items.extend(charger_item());
    items.push(power_item(config.best_performance));
    items.extend(lenovo_item());
    items.push(memory_item(false));
    items.extend(screen_items());
    items.extend(hwinfo_item());
    items
}

pub struct Activation {
    pub items: Vec<Item>,
    pub session: Session,
}

fn shutdown_wsl_if_idle(items: &mut Vec<Item>) {
    if !system::wsl_running() {
        items.push(Item::ok("WSL", "desligada"));
        return;
    }
    let all = system::processes();
    let clients = system::wsl_clients(&system::self_and_ancestors(&all));
    if !clients.is_empty() {
        items.push(Item::warning("WSL", "mantida ligada", format!("em uso por {}", clients.join(", "))));
        return;
    }
    system::shutdown_wsl();
    let deadline = Instant::now() + WSL_SHUTDOWN_TIMEOUT;
    while system::wsl_running() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(500));
    }
    items.push(match system::wsl_running() {
        true => Item::warning("WSL", "não desligou", "rode \"wsl --shutdown\" em um terminal"),
        false => Item::ok("WSL", "desligada"),
    });
}

/// Closes programs, shuts down WSL, sets the power mode, launches the chosen game and saves the session.
pub fn activate(config: &Config, game: Option<&steam::Manifest>, progress: &dyn Fn(&str)) -> Activation {
    let mut items = Vec::new();
    let previous = system::chosen_power_mode();

    progress("Fechando programas");
    let enabled: Vec<ProgramConfig> = config.programs.iter().filter(|p| p.enabled).cloned().collect();
    let closed = close_programs(&enabled);
    items.extend(closed.iter().map(CloseResult::item));

    if config.shutdown_wsl {
        progress("Desligando a WSL");
        shutdown_wsl_if_idle(&mut items);
    }

    progress("Ajustando energia");
    let power_changed = config.best_performance
        && previous.is_some_and(|p| !system::guid_eq(&p, &system::BEST_PERFORMANCE))
        && system::set_power_mode(system::BEST_PERFORMANCE);
    items.extend(charger_item());
    items.push(power_item(false));
    items.extend(lenovo_item());
    items.push(memory_item(true));
    items.extend(screen_items().into_iter().filter(|i| i.status == Status::Warning || i.hint.is_some()));

    if let Some(g) = game {
        progress("Abrindo o jogo");
        let launched = system::shell_open(&format!("steam://rungameid/{}", g.app_id));
        items.push(match launched {
            true => Item::ok("Jogo", &g.name),
            false => Item::warning("Jogo", &g.name, "não foi possível abrir pela Steam"),
        });
    }

    let session = Session {
        started_at: storage::now_epoch(),
        previous_power_mode: previous.filter(|_| power_changed).map(|g| system::guid_to_string(&g)),
        closed_programs: closed.iter().filter(|c| c.closed() && c.had_window).map(|c| c.record.clone()).collect(),
        game: game.map(|g| ChosenGame { app_id: g.app_id.clone(), name: g.name.clone() }),
        game_folder: game.map(|g| g.install_dir.to_string_lossy().to_string()),
    };
    let _ = storage::save_session(&session);
    Activation { items, session }
}

/// Restores the previous power mode, reopens the closed programs and records the session in the history.
pub fn deactivate(session: &Session, reopen: bool) -> Vec<Item> {
    let mut items = Vec::new();
    if let Some(previous) = session.previous_power_mode.as_deref().and_then(system::guid_from_str) {
        items.push(match system::set_power_mode(previous) {
            true => Item::ok("Modo de energia", format!("voltou para {}", system::power_mode_name(&previous))),
            false => Item::warning("Modo de energia", "não foi possível restaurar", "ajuste em Configurações > Sistema > Energia"),
        });
    }
    if reopen {
        for program in &session.closed_programs {
            let target = program.aumid.as_ref().map(|a| format!("shell:AppsFolder\\{a}")).or_else(|| program.path.clone());
            items.push(match target.is_some_and(|t| system::shell_open(&t)) {
                true => Item::ok(&program.name, "reaberto"),
                false => Item::warning(&program.name, "não foi reaberto", "abra manualmente"),
            });
        }
    }
    if lenovo_mode() == Some(LenovoMode::Performance) {
        items.push(Item::neutral("Modo Lenovo", LenovoMode::Performance.label()).with_hint("volte para Energia adaptável no Lenovo Vantage"));
    }
    let duration = storage::now_epoch().saturating_sub(session.started_at);
    let summary = match &session.game {
        Some(g) => format!("{} por {}", g.name, storage::format_duration(duration)),
        None => format!("Modo Jogo por {}", storage::format_duration(duration)),
    };
    storage::add_history(HistoryKind::GameMode, 0, summary);
    storage::delete_session();
    items
}

/// Whether any process running from the game's install folder is alive.
pub fn game_running(folder: &Path) -> bool {
    let session = system::current_session();
    system::processes()
        .iter()
        .filter(|p| p.session == session)
        .filter_map(|p| system::process_path(p.pid))
        .any(|path| system::is_within(Path::new(&path), folder))
}

/// Programs with an open window that are not in the Game Mode list yet, as (process, window title).
pub fn programs_to_add(config: &Config) -> Vec<(String, String)> {
    let ignore = [
        "explorer", "ApplicationFrameHost", "SystemSettings", "TextInputHost", "ShellExperienceHost",
        "SearchHost", "StartMenuExperienceHost", "LockApp", "modo-jogo-limpeza", "ModoJogoLimpeza",
    ];
    let all = system::processes();
    let exclude = system::self_and_ancestors(&all);
    let mut seen = HashSet::new();
    system::open_windows()
        .into_iter()
        .filter_map(|(pid, title)| {
            let process = all.iter().find(|p| p.pid == pid)?;
            let listed = config.programs.iter().any(|c| c.processes.iter().any(|n| n.eq_ignore_ascii_case(&process.name)));
            let ignored = ignore.iter().any(|i| i.eq_ignore_ascii_case(&process.name)) || exclude.contains(&pid);
            (!listed && !ignored && seen.insert(process.name.to_lowercase())).then(|| (process.name.clone(), title))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn closes_notepad_through_its_window() {
        let mut child = std::process::Command::new("notepad.exe").spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let program = ProgramConfig { name: "Notepad".into(), processes: vec!["Notepad".into()], force: Force::Never, enabled: true };
        while Instant::now() < deadline && system::window_count(&pids(&remaining(&program, &HashSet::new()))) == 0 {
            thread::sleep(Duration::from_millis(200));
        }
        let results = close_programs(&[program]);
        let _ = child.kill();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].outcome, CloseOutcome::Closed);
        assert!(results[0].had_window);
    }

    #[test]
    #[ignore]
    fn print_overview() {
        for item in overview(&Config::default()) {
            println!("{:<30} {:<40} {:?} {}", item.label, item.value, item.status, item.hint.unwrap_or_default());
        }
        for game in steam::games() {
            println!("game: {} ({}) at {}", game.name, game.app_id, game.install_dir.display());
        }
        for (process, title) in programs_to_add(&Config::default()) {
            println!("can add: {process} — {title}");
        }
    }
}
