//! Steam libraries and installed games, read from Steam's own files.

use std::fs;
use std::path::{Path, PathBuf};

use crate::system;

const STEAMWORKS_REDIST_APP_ID: &str = "228980";

#[derive(Clone, Debug)]
pub struct Manifest {
    pub app_id: String,
    pub name: String,
    pub install_dir: PathBuf,
}

fn vdf_pair(line: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = line.split('"').collect();
    (parts.len() >= 5).then(|| (parts[1].to_string(), parts[3].replace("\\\\", "\\")))
}

pub fn steam_dir() -> Option<PathBuf> {
    let path = system::registry_string(false, r"Software\Valve\Steam", "SteamPath")?;
    let dir = PathBuf::from(path.replace('/', "\\"));
    dir.is_dir().then_some(dir)
}

/// Library folders configured in Steam, including Steam's own folder.
pub fn libraries() -> Vec<PathBuf> {
    let Some(steam) = steam_dir() else { return Vec::new() };
    let mut folders = vec![steam.clone()];
    if let Ok(text) = fs::read_to_string(steam.join(r"steamapps\libraryfolders.vdf")) {
        for (key, value) in text.lines().filter_map(vdf_pair) {
            let folder = PathBuf::from(&value);
            if key == "path" && folder.is_dir() && !folders.iter().any(|f| system::is_within(f, &folder) && system::is_within(&folder, f)) {
                folders.push(folder);
            }
        }
    }
    folders
}

pub fn manifests(library: &Path) -> Vec<Manifest> {
    let apps = library.join("steamapps");
    let Ok(entries) = fs::read_dir(&apps) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            name.starts_with("appmanifest_") && name.ends_with(".acf")
        })
        .filter_map(|e| {
            let text = fs::read_to_string(e.path()).ok()?;
            let mut app_id = None;
            let mut name = None;
            let mut install_dir = None;
            for (key, value) in text.lines().filter_map(vdf_pair) {
                match key.as_str() {
                    "appid" if app_id.is_none() => app_id = Some(value),
                    "name" if name.is_none() => name = Some(value),
                    "installdir" if install_dir.is_none() => install_dir = Some(value),
                    _ => {}
                }
            }
            Some(Manifest { app_id: app_id?, name: name?, install_dir: apps.join("common").join(install_dir?) })
        })
        .collect()
}

/// Installed games, without Steam's tools and support runtimes.
pub fn games() -> Vec<Manifest> {
    let ignore = ["redistributable", "steamworks", "proton", "steam linux runtime", "steamvr"];
    let mut list: Vec<Manifest> = libraries()
        .iter()
        .flat_map(|l| manifests(l))
        .filter(|m| m.app_id != STEAMWORKS_REDIST_APP_ID && !ignore.iter().any(|i| m.name.to_lowercase().contains(i)))
        .collect();
    list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    list
}
