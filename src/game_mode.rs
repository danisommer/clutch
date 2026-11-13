//! Game Mode: computer status, safe program closing, activation and restore.

use std::collections::HashSet;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::eco_mode;
use crate::games::Game;
use crate::i18n::Texts;
use crate::storage::{self, ClosedProgram, Config, Force, HistoryKind, ProgramConfig, Session, Summary};
use crate::system;

const CLOSE_TIMEOUT: Duration = Duration::from_secs(20);
const WSL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const MIN_FREE_MEMORY: u64 = 5 * 1024 * 1024 * 1024;
const LENOVO_KEY: &str = r"SYSTEM\CurrentControlSet\Services\LITSSVC\LNBITS\IC\MMC";
const WSL: &str = "WSL";
const HWINFO: &str = "HWiNFO";

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
    pub fn ok(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self::new(label, value, Status::Ok, None)
    }
    pub fn neutral(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self::new(label, value, Status::Neutral, None)
    }
    pub fn warning(label: impl Into<String>, value: impl Into<String>, hint: impl Into<String>) -> Self {
        Self::new(label, value, Status::Warning, Some(hint.into()))
    }
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
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
    pub fn item(&self, t: &Texts) -> Item {
        match self.outcome {
            CloseOutcome::Closed => Item::ok(&self.name, t.closed),
            CloseOutcome::ClosedInBackground => Item::ok(&self.name, t.closed).with_hint(t.ran_in_background(&self.name)),
            CloseOutcome::Failed => Item::warning(&self.name, t.could_not_close, t.close_manually),
            CloseOutcome::StillOpen => Item::warning(&self.name, t.still_open, t.maybe_unsaved),
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

pub fn wsl_item(t: &Texts) -> Item {
    match system::wsl_running() {
        true => Item::neutral(WSL, t.wsl_running),
        false => Item::ok(WSL, t.wsl_off),
    }
}

fn charger_item(t: &Texts) -> Option<Item> {
    system::on_ac_power().map(|plugged_in| match plugged_in {
        true => Item::ok(t.charger, t.plugged_in),
        false => Item::warning(t.charger, t.unplugged, t.plug_in),
    })
}

fn power_item(t: &Texts, will_adjust: bool) -> Item {
    let chosen = system::chosen_power_mode();
    let effective = system::effective_power_mode();
    let name = |g: &windows_sys::core::GUID| t.power_mode_name(system::power_mode(g));
    match (chosen, effective) {
        (_, Some(e)) if system::guid_eq(&e, &system::BEST_PERFORMANCE) => Item::ok(t.power_mode, name(&e)),
        (Some(c), _) if system::guid_eq(&c, &system::BEST_PERFORMANCE) => Item::warning(t.power_mode, t.limited_by_saver, t.turn_off_saver),
        (_, Some(e)) if will_adjust => Item::neutral(t.power_mode, name(&e)),
        (_, Some(e)) => Item::warning(t.power_mode, name(&e), t.could_not_set_best),
        (_, None) => Item::neutral(t.power_mode, t.unknown),
    }
}

/// Thermal mode of Lenovo laptops, as set in Lenovo Vantage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LenovoMode {
    Adaptive,
    Performance,
    BatterySaver,
    Unknown,
}

impl LenovoMode {
    pub fn label(self, t: &Texts) -> &'static str {
        match self {
            LenovoMode::Adaptive => t.lenovo_adaptive,
            LenovoMode::Performance => t.lenovo_performance,
            LenovoMode::BatterySaver => t.lenovo_battery_saving,
            LenovoMode::Unknown => t.unknown,
        }
    }
}

/// `None` when the computer does not have Lenovo's thermal service.
pub fn lenovo_mode() -> Option<LenovoMode> {
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

fn lenovo_item(t: &Texts) -> Option<Item> {
    lenovo_mode().map(|mode| match mode {
        LenovoMode::Performance => Item::ok(t.lenovo_mode, mode.label(t)),
        LenovoMode::Unknown => Item::neutral(t.lenovo_mode, mode.label(t)).with_hint(t.lenovo_check),
        LenovoMode::Adaptive | LenovoMode::BatterySaver => Item::warning(t.lenovo_mode, mode.label(t), t.lenovo_select_performance),
    })
}

fn memory_item(t: &Texts, after_closing: bool) -> Item {
    let available = system::available_memory();
    let value = t.size(available);
    match (available >= MIN_FREE_MEMORY, after_closing) {
        (true, _) => Item::ok(t.available_memory, value),
        (false, true) => Item::warning(t.available_memory, value, t.low_memory),
        (false, false) => Item::neutral(t.available_memory, value),
    }
}

fn screen_items(t: &Texts) -> Vec<Item> {
    let screens = system::screens();
    let mut items: Vec<Item> = screens
        .iter()
        .map(|s| {
            let label = match (s.internal, s.name.trim()) {
                (true, _) => t.laptop_screen.to_string(),
                (false, "") => t.external_monitor.to_string(),
                (false, name) => name.to_string(),
            };
            let value = t.screen_mode(s.width, s.height, s.hz, s.primary && screens.len() > 1);
            match s.hz < s.max_hz {
                true => Item::warning(label, value, t.supports_hz(s.max_hz)),
                false => Item::ok(label, value),
            }
        })
        .collect();
    if screens.len() > 1 {
        if let Some((item, _)) = items.iter_mut().zip(&screens).find(|(i, s)| !s.internal && i.hint.is_none()) {
            item.hint = Some(t.second_screen_only.into());
        }
    }
    items
}

fn hwinfo_item(t: &Texts) -> Option<Item> {
    system::processes()
        .iter()
        .any(|p| p.name.eq_ignore_ascii_case("HWiNFO64"))
        .then(|| Item::neutral(HWINFO, t.open).with_hint(t.close_if_not_monitoring))
}

/// Current state of the computer, before Game Mode is activated.
pub fn overview(config: &Config, t: &Texts) -> Vec<Item> {
    let open = open_programs(config);
    let mut items = vec![match open.is_empty() {
        true => Item::ok(t.heavy_programs, t.none_open),
        false => Item::neutral(t.heavy_programs, open.join(", ")),
    }];
    items.push(wsl_item(t));
    items.extend(charger_item(t));
    items.push(power_item(t, config.best_performance));
    items.extend(lenovo_item(t));
    items.push(memory_item(t, false));
    items.extend(screen_items(t));
    items.extend(hwinfo_item(t));
    items
}

pub struct Activation {
    pub items: Vec<Item>,
    pub session: Session,
}

/// Shuts WSL down unless some program is using it, and reports what happened.
pub fn shutdown_wsl_if_idle(t: &Texts, items: &mut Vec<Item>) {
    if !system::wsl_running() {
        items.push(Item::ok(WSL, t.wsl_off));
        return;
    }
    let all = system::processes();
    let clients = system::wsl_clients(&system::self_and_ancestors(&all));
    if !clients.is_empty() {
        items.push(Item::warning(WSL, t.wsl_kept_running, t.wsl_in_use_by(&clients.join(", "))));
        return;
    }
    system::shutdown_wsl();
    let deadline = Instant::now() + WSL_SHUTDOWN_TIMEOUT;
    while system::wsl_running() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(500));
    }
    items.push(match system::wsl_running() {
        true => Item::warning(WSL, t.wsl_did_not_stop, t.wsl_shutdown_hint),
        false => Item::ok(WSL, t.wsl_off),
    });
}

/// Closes programs, shuts down WSL, sets the power mode, launches the chosen game and saves the session.
pub fn activate(config: &Config, game: Option<&Game>, t: &Texts, progress: &dyn Fn(&str)) -> Activation {
    let mut items = Vec::new();
    let previous = system::chosen_power_mode();

    progress(t.step_closing);
    let enabled: Vec<ProgramConfig> = config.programs.iter().filter(|p| p.enabled).cloned().collect();
    let closed = close_programs(&enabled);
    items.extend(closed.iter().map(|c| c.item(t)));

    if config.shutdown_wsl {
        progress(t.step_wsl);
        shutdown_wsl_if_idle(t, &mut items);
    }

    progress(t.step_power);
    let power_changed = config.best_performance
        && previous.is_some_and(|p| !system::guid_eq(&p, &system::BEST_PERFORMANCE))
        && system::set_power_mode(system::BEST_PERFORMANCE);
    items.extend(charger_item(t));
    items.push(power_item(t, false));
    items.extend(lenovo_item(t));
    items.push(memory_item(t, true));
    items.extend(screen_items(t).into_iter().filter(|i| i.status == Status::Warning || i.hint.is_some()));

    if let Some(g) = game {
        progress(t.step_game);
        items.push(match g.start() {
            true => Item::ok(t.game, &g.name),
            false => Item::warning(t.game, &g.name, t.could_not_launch(g.launcher)),
        });
    }

    let session = Session {
        started_at: storage::now_epoch(),
        previous_power_mode: previous.filter(|_| power_changed).map(|g| system::guid_to_string(&g)),
        closed_programs: closed.iter().filter(|c| c.closed() && c.had_window).map(|c| c.record.clone()).collect(),
        game: game.map(Game::chosen),
        game_folder: game.map(|g| g.install_dir.to_string_lossy().to_string()),
        eco: None,
    };
    let _ = storage::save_session(&session);
    Activation { items, session }
}

/// Ends a session of either mode: restores what it changed, reopens the closed programs and records it in the history.
pub fn deactivate(session: &Session, reopen: bool, t: &Texts) -> Vec<Item> {
    let mut items = Vec::new();
    if let Some(previous) = session.previous_power_mode.as_deref().and_then(system::guid_from_str) {
        items.push(match system::set_power_mode(previous) {
            true => Item::ok(t.power_mode, t.power_back_to(t.power_mode_name(system::power_mode(&previous)))),
            false => Item::warning(t.power_mode, t.could_not_restore, t.power_settings_hint),
        });
    }
    if let Some(eco) = &session.eco {
        items.extend(eco_mode::restore(eco, t));
    }
    if reopen {
        for program in &session.closed_programs {
            let target = program.aumid.as_ref().map(|a| format!("shell:AppsFolder\\{a}")).or_else(|| program.path.clone());
            items.push(match target.is_some_and(|path| system::shell_open(&path)) {
                true => Item::ok(&program.name, t.reopened),
                false => Item::warning(&program.name, t.not_reopened, t.open_manually),
            });
        }
    }
    let seconds = storage::now_epoch().saturating_sub(session.started_at);
    let (left_on, kind, summary) = match session.eco {
        Some(_) => (LenovoMode::BatterySaver, HistoryKind::EcoMode, Summary::EcoMode { eco_seconds: seconds }),
        None => (LenovoMode::Performance, HistoryKind::GameMode, Summary::GameMode { game: session.game.as_ref().map(|g| g.name.clone()), seconds }),
    };
    if lenovo_mode() == Some(left_on) {
        items.push(Item::neutral(t.lenovo_mode, left_on.label(t)).with_hint(t.lenovo_back_to_adaptive));
    }
    storage::add_history(kind, 0, summary);
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
        "SearchHost", "StartMenuExperienceHost", "LockApp", crate::APP_NAME,
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
    use crate::i18n::PT_BR;

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
        for item in overview(&Config::default(), &PT_BR) {
            println!("{:<30} {:<40} {:?} {}", item.label, item.value, item.status, item.hint.unwrap_or_default());
        }
        for game in crate::games::installed(&Config::default()) {
            println!("game: {} ({:?} {}) at {}", game.name, game.launcher, game.id, game.install_dir.display());
        }
        for (process, title) in programs_to_add(&Config::default()) {
            println!("can add: {process} — {title}");
        }
    }
}
