//! Administrator mode: the same executable, started elevated, measures or cleans the system targets.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::i18n::{self, Language};
use crate::{cleanup, debloat, drivers, storage, system};

pub const ARG: &str = "--admin";

const EXIT_OK: i32 = 0;
const EXIT_BAD_ARGS: i32 = 2;
const EXIT_WRITE_FAILED: i32 = 3;

/// What the window asks the elevated process to clean, and the language of its messages.
#[derive(Serialize, Deserialize)]
struct CleanRequest {
    ids: Vec<String>,
    recycle_folders: bool,
    language: Language,
}

/// Decrapify items to apply or undo in the elevated process.
#[derive(Serialize, Deserialize)]
struct DebloatRequest {
    ids: Vec<String>,
    action: debloat::Action,
}

/// Driver updates from Windows Update to install in the elevated process, optionally after a restore point.
#[derive(Serialize, Deserialize)]
struct DriversRequest {
    ids: Vec<String>,
    restore_point: bool,
}

/// Folder used to pass requests and results between the window and the elevated process.
pub fn exchange_dir() -> PathBuf {
    let dir = storage::data_dir().join("tmp");
    let _ = fs::create_dir_all(&dir);
    dir
}

fn inside_exchange_dir(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);
    system::is_within(path, &exchange_dir()).then(|| path.to_path_buf())
}

fn write_result<T: serde::Serialize>(path: &Path, value: &T) -> i32 {
    match serde_json::to_vec(value).map(|b| fs::write(path, b)) {
        Ok(Ok(())) => EXIT_OK,
        _ => EXIT_WRITE_FAILED,
    }
}

fn read_request<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// Handles `--admin analyze <out>`, `--admin clean <in> <out>`, `--admin debloat <in> <out>` and `--admin drivers <in> <out>`.
/// Exit codes: 0 ok, 2 bad arguments, 3 write failure.
pub fn run(args: &[String]) -> i32 {
    let parts: Vec<&str> = args.iter().map(String::as_str).collect();
    match parts.as_slice() {
        [_, ARG, "analyze", output] => match inside_exchange_dir(output) {
            Some(output) => write_result(&output, &cleanup::analyze_admin()),
            None => EXIT_BAD_ARGS,
        },
        [_, ARG, "clean", input, output] => {
            let (Some(input), Some(output)) = (inside_exchange_dir(input), inside_exchange_dir(output)) else { return EXIT_BAD_ARGS };
            let Some(request) = read_request::<CleanRequest>(&input) else { return EXIT_BAD_ARGS };
            let catalog = cleanup::builtin_catalog();
            let known: Vec<String> = request.ids.into_iter().filter(|id| catalog.categories.iter().any(|c| &c.id == id)).collect();
            write_result(&output, &cleanup::clean(&known, true, request.recycle_folders, i18n::texts(request.language), &|_| {}))
        }
        [_, ARG, "debloat", input, output] => {
            let (Some(input), Some(output)) = (inside_exchange_dir(input), inside_exchange_dir(output)) else { return EXIT_BAD_ARGS };
            let Some(request) = read_request::<DebloatRequest>(&input) else { return EXIT_BAD_ARGS };
            let catalog = debloat::catalog();
            let known: Vec<String> = request.ids.into_iter().filter(|id| catalog.items.iter().any(|i| &i.id == id)).collect();
            write_result(&output, &debloat::run(&catalog, &known, request.action, true))
        }
        [_, ARG, "drivers", input, output] => {
            let (Some(input), Some(output)) = (inside_exchange_dir(input), inside_exchange_dir(output)) else { return EXIT_BAD_ARGS };
            let Some(request) = read_request::<DriversRequest>(&input) else { return EXIT_BAD_ARGS };
            write_result(&output, &drivers::install_updates(&request.ids, request.restore_point))
        }
        _ => EXIT_BAD_ARGS,
    }
}

#[derive(Debug)]
pub enum AdminError {
    Cancelled,
    RequestFailed(u32),
    NoResult,
    ExitCode(u32),
    Io(String),
}

fn invoke<T: serde::de::DeserializeOwned>(params: String, output: &Path) -> Result<T, AdminError> {
    let _ = fs::remove_file(output);
    match system::run_elevated(&params) {
        Err(system::ElevationError::Cancelled) => Err(AdminError::Cancelled),
        Err(system::ElevationError::Failed(code)) => Err(AdminError::RequestFailed(code)),
        Ok(0) => fs::read(output).ok().and_then(|b| serde_json::from_slice(&b).ok()).ok_or(AdminError::NoResult),
        Ok(code) => Err(AdminError::ExitCode(code)),
    }
}

/// Asks for elevation and measures the system targets; returns bytes per category id.
pub fn measure_system() -> Result<HashMap<String, u64>, AdminError> {
    let output = exchange_dir().join("analysis.json");
    invoke(format!("{ARG} analyze \"{}\"", output.display()), &output)
}

/// Asks for elevation and cleans the system targets of the chosen categories.
pub fn clean_system(ids: &[String], recycle_folders: bool, language: Language) -> Result<Vec<cleanup::CleanupResult>, AdminError> {
    let input = exchange_dir().join("request.json");
    let output = exchange_dir().join("result.json");
    let request = CleanRequest { ids: ids.to_vec(), recycle_folders, language };
    let bytes = serde_json::to_vec(&request).map_err(|e| AdminError::Io(e.to_string()))?;
    fs::write(&input, bytes).map_err(|e| AdminError::Io(e.to_string()))?;
    invoke(format!("{ARG} clean \"{}\" \"{}\"", input.display(), output.display()), &output)
}

/// Asks for elevation and installs the chosen driver updates. The inner error is what Windows Update answered.
pub fn install_drivers(ids: &[String], restore_point: bool) -> Result<Result<drivers::InstallReport, String>, AdminError> {
    let input = exchange_dir().join("drivers-request.json");
    let output = exchange_dir().join("drivers-result.json");
    let request = DriversRequest { ids: ids.to_vec(), restore_point };
    let bytes = serde_json::to_vec(&request).map_err(|e| AdminError::Io(e.to_string()))?;
    fs::write(&input, bytes).map_err(|e| AdminError::Io(e.to_string()))?;
    invoke(format!("{ARG} drivers \"{}\" \"{}\"", input.display(), output.display()), &output)
}

/// Asks for elevation and applies or undoes the parts of the chosen decrapify items that need administrator rights.
pub fn debloat_system(ids: &[String], action: debloat::Action) -> Result<Vec<debloat::Outcome>, AdminError> {
    let input = exchange_dir().join("debloat-request.json");
    let output = exchange_dir().join("debloat-result.json");
    let request = DebloatRequest { ids: ids.to_vec(), action };
    let bytes = serde_json::to_vec(&request).map_err(|e| AdminError::Io(e.to_string()))?;
    fs::write(&input, bytes).map_err(|e| AdminError::Io(e.to_string()))?;
    invoke(format!("{ARG} debloat \"{}\" \"{}\"", input.display(), output.display()), &output)
}
