//! Settings, history and the Game Mode session, stored as JSON in %LOCALAPPDATA%\ModoJogoLimpeza.

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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ChosenGame {
    pub app_id: String,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
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
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum HistoryKind {
    GameMode,
    Cleanup,
}

impl HistoryKind {
    pub fn label(self) -> &'static str {
        match self {
            HistoryKind::GameMode => "Modo Jogo",
            HistoryKind::Cleanup => "Limpeza",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub date: String,
    pub kind: HistoryKind,
    pub freed: u64,
    pub summary: String,
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
pub fn add_history(kind: HistoryKind, freed: u64, summary: String) {
    let mut history = load_history();
    history.insert(0, HistoryEntry { date: now_local(), kind, freed, summary });
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

pub fn now_local() -> String {
    let t = system::local_time();
    format!("{:02}/{:02}/{} {:02}:{:02}", t.wDay, t.wMonth, t.wYear, t.wHour, t.wMinute)
}

/// Human-readable size with a decimal comma: "6,4 GB", "820 MB", "12 KB".
pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    let (value, unit, decimals) = match b {
        _ if b >= KB * KB * KB => (b / (KB * KB * KB), "GB", 1),
        _ if b >= KB * KB => (b / (KB * KB), "MB", 0),
        _ if b >= KB => (b / KB, "KB", 0),
        _ => (b, "B", 0),
    };
    format!("{value:.decimals$} {unit}").replace('.', ",")
}

pub fn format_duration(seconds: u64) -> String {
    format!("{:02}:{:02}:{:02}", seconds / 3600, (seconds / 60) % 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes_with_decimal_comma() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1536), "2 KB");
        assert_eq!(format_size(820 * 1024 * 1024), "820 MB");
        assert_eq!(format_size(6_871_947_674), "6,4 GB");
    }

    #[test]
    fn formats_duration() {
        assert_eq!(format_duration(3725), "01:02:05");
    }
}
