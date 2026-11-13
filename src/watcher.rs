//! Watcher: a windowless process that follows the game, or the charger in Eco Mode, and restores the computer when the session is over.

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows_sys::Win32::System::Threading::{CreateMutexW, OpenMutexW};

use crate::storage::{LastSummary, Session};
use crate::{game_mode, i18n, storage, system};

pub const ARG: &str = "--watcher";
const WATCHER_MUTEX: &str = r"Local\Clutch.Watcher";
const WINDOW_MUTEX: &str = r"Local\Clutch.Window";
const SYNCHRONIZE: u32 = 0x0010_0000;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const POLL_INTERVAL: Duration = Duration::from_secs(3);
const MISSES_BEFORE_EXIT: u32 = 2;

/// Named mutex held while the owning process lives; other processes test for it with `marker_exists`.
pub struct Marker(HANDLE);

impl Drop for Marker {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn create_marker(name: &str) -> Option<Marker> {
    let h = unsafe { CreateMutexW(std::ptr::null(), 0, system::wide(name).as_ptr()) };
    match (h.is_null(), unsafe { GetLastError() } == ERROR_ALREADY_EXISTS) {
        (true, _) => None,
        (false, true) => {
            unsafe { CloseHandle(h) };
            None
        }
        (false, false) => Some(Marker(h)),
    }
}

fn marker_exists(name: &str) -> bool {
    let h = unsafe { OpenMutexW(SYNCHRONIZE, 0, system::wide(name).as_ptr()) };
    match h.is_null() {
        true => false,
        false => {
            unsafe { CloseHandle(h) };
            true
        }
    }
}

/// Marks the main window as open for as long as the returned marker is alive.
pub fn mark_window() -> Option<Marker> {
    create_marker(WINDOW_MUTEX)
}

pub fn is_running() -> bool {
    marker_exists(WATCHER_MUTEX)
}

/// Starts the watcher in the background unless one is already running.
pub fn start() -> bool {
    if is_running() {
        return true;
    }
    match std::env::current_exe() {
        Ok(exe) => Command::new(exe).arg(ARG).creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS).spawn().is_ok(),
        Err(_) => false,
    }
}

/// What tells that a session is over: the game exiting, or the charger being plugged in.
pub enum Watch {
    Game(PathBuf),
    Charger,
}

impl Watch {
    /// What to follow for a session; `None` when it only ends by hand.
    pub fn of(session: &Session) -> Option<Watch> {
        match (&session.game_folder, &session.eco) {
            (Some(folder), _) => Some(Watch::Game(PathBuf::from(folder))),
            (None, Some(eco)) if eco.end_when_plugged_in => Some(Watch::Charger),
            (None, _) => None,
        }
    }

    /// Whether what is followed is still going: the game running, or the computer on battery.
    pub fn ongoing(&self) -> bool {
        match self {
            Watch::Game(folder) => game_mode::game_running(folder),
            Watch::Charger => system::on_ac_power() == Some(false),
        }
    }
}

/// Waits for what the session follows to start and stop, then restores the computer, saves the summary and shows the window.
pub fn run() -> i32 {
    let Some(_marker) = create_marker(WATCHER_MUTEX) else { return 0 };
    let Some(watch) = storage::load_session().as_ref().and_then(Watch::of) else { return 0 };
    let mut seen = false;
    let mut misses = 0;
    loop {
        if !storage::session_active() {
            return 0;
        }
        match (watch.ongoing(), seen) {
            (true, _) => {
                seen = true;
                misses = 0;
            }
            (false, true) => misses += 1,
            (false, false) => {}
        }
        if misses >= MISSES_BEFORE_EXIT {
            break;
        }
        thread::sleep(POLL_INTERVAL);
    }
    let Some(session) = storage::claim_session() else { return 0 };
    let config = storage::load_config();
    let items = game_mode::deactivate(&session, config.reopens(&session), i18n::texts_for(config.language));
    let _ = storage::save_last_summary(&LastSummary { eco: session.eco.is_some(), items });
    if !marker_exists(WINDOW_MUTEX) {
        if let Ok(exe) = std::env::current_exe() {
            system::shell_open(&exe.to_string_lossy());
        }
    }
    0
}
