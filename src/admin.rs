//! Administrator mode: the same executable, started elevated, measures or cleans the system targets.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{cleanup, storage, system};

pub const ARG: &str = "--admin";

const EXIT_OK: i32 = 0;
const EXIT_BAD_ARGS: i32 = 2;
const EXIT_WRITE_FAILED: i32 = 3;

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

/// Handles `--admin analyze <out>` and `--admin clean <in> <out>`. Exit codes: 0 ok, 2 bad arguments, 3 write failure.
pub fn run(args: &[String]) -> i32 {
    let parts: Vec<&str> = args.iter().map(String::as_str).collect();
    match parts.as_slice() {
        [_, ARG, "analyze", output] => match inside_exchange_dir(output) {
            Some(output) => write_result(&output, &cleanup::analyze_admin()),
            None => EXIT_BAD_ARGS,
        },
        [_, ARG, "clean", input, output] => {
            let (Some(input), Some(output)) = (inside_exchange_dir(input), inside_exchange_dir(output)) else { return EXIT_BAD_ARGS };
            let ids: Vec<String> = fs::read(&input).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            let catalog = cleanup::catalog();
            let known: Vec<String> = ids.into_iter().filter(|id| catalog.categories.iter().any(|c| &c.id == id)).collect();
            write_result(&output, &cleanup::clean(&known, true, &|_| {}))
        }
        _ => EXIT_BAD_ARGS,
    }
}

pub enum AdminError {
    Cancelled,
    Failed(String),
}

fn invoke<T: serde::de::DeserializeOwned>(params: String, output: &Path) -> Result<T, AdminError> {
    let _ = fs::remove_file(output);
    match system::run_elevated(&params) {
        Err(system::ElevationError::Cancelled) => Err(AdminError::Cancelled),
        Err(system::ElevationError::Failed(code)) => Err(AdminError::Failed(format!("não foi possível pedir permissão ({code})"))),
        Ok(0) => fs::read(output)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .ok_or_else(|| AdminError::Failed("o modo administrador não devolveu resultado".into())),
        Ok(code) => Err(AdminError::Failed(format!("o modo administrador terminou com código {code}"))),
    }
}

/// Asks for elevation and measures the system targets; returns bytes per category id.
pub fn measure_system() -> Result<HashMap<String, u64>, AdminError> {
    let output = exchange_dir().join("analysis.json");
    invoke(format!("{ARG} analyze \"{}\"", output.display()), &output)
}

/// Asks for elevation and cleans the system targets of the chosen categories.
pub fn clean_system(ids: &[String]) -> Result<Vec<cleanup::CleanupResult>, AdminError> {
    let input = exchange_dir().join("request.json");
    let output = exchange_dir().join("result.json");
    fs::write(&input, serde_json::to_vec(ids).unwrap_or_default()).map_err(|e| AdminError::Failed(e.to_string()))?;
    invoke(format!("{ARG} clean \"{}\" \"{}\"", input.display(), output.display()), &output)
}
