//! Settings, history and the Game Mode session, stored as JSON in %LOCALAPPDATA%\ModoJogoLimpeza.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::system;

const HISTORY_LIMIT: usize = 100;

/// When a program may be terminated if it does not close on request.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Force {
    Never,
    WhenWindowless,
    Always,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProgramConfig {
    pub name: String,
    pub processes: Vec<String>,
    pub force: Force,
    pub enabled: bool,
}

/// Where a game comes from. Choices saved before other launchers existed are Steam games.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Launcher {
    #[default]
    Steam,
    Epic,
    Gog,
    Custom,
}

/// The game to launch: its launcher and the launcher's id for it (the executable path for custom games).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ChosenGame {
    #[serde(default)]
    pub launcher: Launcher,
    #[serde(alias = "app_id")]
    pub id: String,
    pub name: String,
}

/// Game Mode settings that one game keeps for itself instead of using the general ones.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GameProfile {
    pub launcher: Launcher,
    pub id: String,
    pub keep_open: Vec<String>,
    pub shutdown_wsl: bool,
    pub best_performance: bool,
}

/// A game added by picking its executable.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CustomGame {
    pub name: String,
    pub exe: String,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum LanguageChoice {
    #[default]
    System,
    PortugueseBrazil,
    English,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub programs: Vec<ProgramConfig>,
    pub game: Option<ChosenGame>,
    pub shutdown_wsl: bool,
    pub best_performance: bool,
    pub reopen_on_exit: bool,
    pub theme: ThemeChoice,
    pub language: LanguageChoice,
    pub cleanup_choices: HashMap<String, bool>,
    pub recycle_folders: bool,
    pub custom_games: Vec<CustomGame>,
    pub profiles: Vec<GameProfile>,
    pub removed_apps: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        let program = |name: &str, processes: &[&str], force| ProgramConfig {
            name: name.into(),
            processes: processes.iter().map(|p| p.to_string()).collect(),
            force,
            enabled: true,
        };
        Config {
            programs: vec![
                program("Visual Studio Code", &["Code"], Force::Never),
                program("Google Chrome", &["chrome"], Force::WhenWindowless),
                program("WhatsApp", &["WhatsApp.Root", "WhatsApp"], Force::Always),
                program("Spotify", &["Spotify", "SpotifyLauncher", "SpotifyXboxGamebarWebView"], Force::Always),
            ],
            game: None,
            shutdown_wsl: true,
            best_performance: true,
            reopen_on_exit: true,
            theme: ThemeChoice::System,
            language: LanguageChoice::System,
            cleanup_choices: HashMap::new(),
            recycle_folders: false,
            custom_games: Vec::new(),
            profiles: Vec::new(),
            removed_apps: Vec::new(),
        }
    }
}

impl Config {
    fn profile_index(&self) -> Option<usize> {
        let game = self.game.as_ref()?;
        self.profiles.iter().position(|p| p.launcher == game.launcher && p.id == game.id)
    }

    pub fn has_profile(&self) -> bool {
        self.profile_index().is_some()
    }

    /// The settings Game Mode uses: the chosen game's profile when it has one, otherwise the general ones.
    pub fn effective(&self) -> Config {
        let mut config = self.clone();
        if let Some(profile) = self.profile_index().map(|i| &self.profiles[i]) {
            for program in &mut config.programs {
                program.enabled = !profile.keep_open.contains(&program.name);
            }
            config.shutdown_wsl = profile.shutdown_wsl;
            config.best_performance = profile.best_performance;
        }
        config
    }

    /// Whether the program at `index` is closed, in the chosen game's profile or in the general settings.
    pub fn set_closes(&mut self, index: usize, closes: bool) {
        let name = self.programs[index].name.clone();
        match self.profile_index() {
            Some(p) => {
                let keep_open = &mut self.profiles[p].keep_open;
                keep_open.retain(|n| n != &name);
                if !closes {
                    keep_open.push(name);
                }
            }
            None => self.programs[index].enabled = closes,
        }
    }

    pub fn set_shutdown_wsl(&mut self, value: bool) {
        match self.profile_index() {
            Some(p) => self.profiles[p].shutdown_wsl = value,
            None => self.shutdown_wsl = value,
        }
    }

    pub fn set_best_performance(&mut self, value: bool) {
        match self.profile_index() {
            Some(p) => self.profiles[p].best_performance = value,
            None => self.best_performance = value,
        }
    }

    /// Gives the chosen game a profile that starts as a copy of the current settings.
    pub fn create_game_profile(&mut self) {
        let current = self.effective();
        let Some(game) = current.game.as_ref() else { return };
        self.remove_game_profile();
        self.profiles.push(GameProfile {
            launcher: game.launcher,
            id: game.id.clone(),
            keep_open: current.programs.iter().filter(|p| !p.enabled).map(|p| p.name.clone()).collect(),
            shutdown_wsl: current.shutdown_wsl,
            best_performance: current.best_performance,
        });
    }

    /// Makes the chosen game use the general settings again.
    pub fn remove_game_profile(&mut self) {
        if let Some(index) = self.profile_index() {
            self.profiles.remove(index);
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum HistoryKind {
    GameMode,
    Cleanup,
}

/// Local date and time, kept as numbers so it can be shown in any language.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Stamp {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
}

/// Entries written before translation keep their date as Portuguese text.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HistoryDate {
    Local(Stamp),
    Legacy(String),
}

/// What an entry was about; entries written before translation keep a Portuguese sentence.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Summary {
    Cleanup { categories: usize },
    GameMode { game: Option<String>, seconds: u64 },
    Legacy(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub date: HistoryDate,
    pub kind: HistoryKind,
    pub freed: u64,
    pub summary: Summary,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClosedProgram {
    pub name: String,
    pub path: Option<String>,
    pub aumid: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub started_at: u64,
    pub previous_power_mode: Option<String>,
    pub closed_programs: Vec<ClosedProgram>,
    pub game: Option<ChosenGame>,
    pub game_folder: Option<String>,
}

pub fn data_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
    let dir = base.join("ModoJogoLimpeza");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// Reads a data file; an unreadable one is set aside as `.json.corrupt` so defaults take over.
fn read_json<T: DeserializeOwned>(name: &str) -> Option<T> {
    let path = data_dir().join(name);
    let text = fs::read_to_string(&path).ok()?;
    match serde_json::from_str(strip_bom(&text)) {
        Ok(value) => Some(value),
        Err(_) => {
            let _ = fs::rename(&path, path.with_extension("json.corrupt"));
            None
        }
    }
}

/// Writes through a temporary file and a rename, so readers never see a half-written file.
fn write_json<T: Serialize>(name: &str, value: &T) -> std::io::Result<()> {
    let path = data_dir().join(name);
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?)?;
    fs::rename(&temporary, &path)
}

pub fn load_config() -> Config {
    read_json("config.json").unwrap_or_default()
}

pub fn save_config(config: &Config) -> std::io::Result<()> {
    write_json("config.json", config)
}

pub fn load_history() -> Vec<HistoryEntry> {
    read_json("history.json").unwrap_or_default()
}

/// Prepends an entry and keeps only the most recent ones.
pub fn add_history(kind: HistoryKind, freed: u64, summary: Summary) {
    let mut history = load_history();
    history.insert(0, HistoryEntry { date: HistoryDate::Local(now_local()), kind, freed, summary });
    history.truncate(HISTORY_LIMIT);
    let _ = write_json("history.json", &history);
}

pub fn load_session() -> Option<Session> {
    read_json("session.json")
}

pub fn save_session(session: &Session) -> std::io::Result<()> {
    write_json("session.json", session)
}

pub fn session_active() -> bool {
    data_dir().join("session.json").exists()
}

/// Takes ownership of the session to end it. Only one process succeeds, because the rename is atomic.
pub fn claim_session() -> Option<Session> {
    let dir = data_dir();
    let closing = dir.join("session-closing.json");
    fs::rename(dir.join("session.json"), &closing).ok()?;
    let text = fs::read_to_string(&closing).ok()?;
    serde_json::from_str(strip_bom(&text)).ok()
}

pub fn delete_session() {
    let dir = data_dir();
    let _ = fs::remove_file(dir.join("session.json"));
    let _ = fs::remove_file(dir.join("session-closing.json"));
}

pub fn save_last_summary<T: Serialize>(items: &T) -> std::io::Result<()> {
    write_json("last-summary.json", items)
}

/// Reads and deletes the summary left by the watcher when it ended a session.
pub fn take_last_summary<T: DeserializeOwned>() -> Option<T> {
    let summary = read_json("last-summary.json");
    let _ = fs::remove_file(data_dir().join("last-summary.json"));
    summary
}

pub fn now_epoch() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn now_local() -> Stamp {
    let t = system::local_time();
    Stamp { year: t.wYear, month: t.wMonth, day: t.wDay, hour: t.wHour, minute: t.wMinute }
}

pub fn format_duration(seconds: u64) -> String {
    format!("{:02}:{:02}:{:02}", seconds / 3600, (seconds / 60) % 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_duration() {
        assert_eq!(format_duration(3725), "01:02:05");
    }
}
