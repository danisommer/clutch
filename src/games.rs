//! Installed games from Steam, Epic Games and GOG, plus executables the user added by hand.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::storage::{ChosenGame, Config, CustomGame, Launcher};
use crate::{steam, system};

const EPIC_MANIFESTS: &str = r"Epic\EpicGamesLauncher\Data\Manifests";
const GOG_GAMES_KEY: &str = r"SOFTWARE\GOG.com\Games";

/// How a game is started: through its launcher's URI, or by running its executable in its own folder.
#[derive(Clone, Debug)]
pub enum Launch {
    Uri(String),
    Program { exe: PathBuf, dir: PathBuf },
}

#[derive(Clone, Debug)]
pub struct Game {
    pub launcher: Launcher,
    pub id: String,
    pub name: String,
    pub install_dir: PathBuf,
    pub launch: Launch,
}

impl Game {
    pub fn is(&self, chosen: &ChosenGame) -> bool {
        self.launcher == chosen.launcher && self.id == chosen.id
    }

    pub fn chosen(&self) -> ChosenGame {
        ChosenGame { launcher: self.launcher, id: self.id.clone(), name: self.name.clone() }
    }

    /// Starts the game; `false` when Windows could not open it.
    pub fn start(&self) -> bool {
        match &self.launch {
            Launch::Uri(uri) => system::shell_open(uri),
            Launch::Program { exe, dir } => system::shell_open_in(&exe.to_string_lossy(), dir),
        }
    }
}

fn steam_games() -> Vec<Game> {
    steam::games()
        .into_iter()
        .map(|m| Game {
            launcher: Launcher::Steam,
            launch: Launch::Uri(format!("steam://rungameid/{}", m.app_id)),
            id: m.app_id,
            name: m.name,
            install_dir: m.install_dir,
        })
        .collect()
}

/// Fields of an Epic Games Launcher `.item` manifest.
#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct EpicManifest {
    display_name: String,
    install_location: String,
    app_name: String,
    catalog_namespace: String,
    catalog_item_id: String,
    main_game_app_name: String,
    #[serde(rename = "bIsIncompleteInstall")]
    is_incomplete_install: bool,
}

impl EpicManifest {
    /// Complete installs of games, without DLC entries and the Unreal Engine tools.
    fn is_game(&self) -> bool {
        let main_game = self.main_game_app_name.is_empty() || self.main_game_app_name == self.app_name;
        !self.is_incomplete_install && main_game && !self.app_name.is_empty() && !self.display_name.contains("Unreal Engine")
    }

    /// The same URI the launcher writes in its desktop shortcuts.
    fn launch_uri(&self) -> String {
        match (self.catalog_namespace.is_empty(), self.catalog_item_id.is_empty()) {
            (false, false) => format!(
                "com.epicgames.launcher://apps/{}%3A{}%3A{}?action=launch&silent=true",
                self.catalog_namespace, self.catalog_item_id, self.app_name
            ),
            _ => format!("com.epicgames.launcher://apps/{}?action=launch&silent=true", self.app_name),
        }
    }
}

fn epic_games() -> Vec<Game> {
    let Some(folder) = std::env::var_os("ProgramData").map(|d| PathBuf::from(d).join(EPIC_MANIFESTS)) else { return Vec::new() };
    let Ok(entries) = fs::read_dir(folder) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("item")))
        .filter_map(|e| serde_json::from_slice::<EpicManifest>(&fs::read(e.path()).ok()?).ok())
        .filter(|m| m.is_game() && Path::new(&m.install_location).is_dir())
        .map(|m| Game {
            launcher: Launcher::Epic,
            launch: Launch::Uri(m.launch_uri()),
            id: m.app_name,
            name: m.display_name,
            install_dir: PathBuf::from(m.install_location),
        })
        .collect()
}

/// GOG games, from the keys their installers write; they are started by their own executable.
fn gog_games() -> Vec<Game> {
    system::registry_subkeys_32(GOG_GAMES_KEY)
        .into_iter()
        .filter_map(|(id, values)| {
            let name = values.get("gamename")?.clone();
            let install_dir = PathBuf::from(values.get("path")?);
            let exe = PathBuf::from(values.get("exe")?);
            let dir = values.get("workingdir").filter(|d| !d.is_empty()).map(PathBuf::from).unwrap_or_else(|| install_dir.clone());
            (install_dir.is_dir() && exe.is_file()).then(|| Game { launcher: Launcher::Gog, id, name, install_dir, launch: Launch::Program { exe, dir } })
        })
        .collect()
}

/// A game the user added by picking its executable; the folder of the executable is the game folder.
pub fn custom_game(game: &CustomGame) -> Option<Game> {
    let exe = PathBuf::from(&game.exe);
    let dir = exe.parent()?.to_path_buf();
    exe.is_file().then(|| Game {
        launcher: Launcher::Custom,
        id: game.exe.clone(),
        name: game.name.clone(),
        install_dir: dir.clone(),
        launch: Launch::Program { exe, dir },
    })
}

/// Every game that can be launched, sorted by name.
pub fn installed(config: &Config) -> Vec<Game> {
    let mut list = steam_games();
    list.extend(epic_games());
    list.extend(gog_games());
    list.extend(config.custom_games.iter().filter_map(custom_game));
    list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_epic_manifests() {
        let game = r#"{ "DisplayName": "Hades", "InstallLocation": "C:\\Games\\Hades", "AppName": "Min", "CatalogNamespace": "ns", "CatalogItemId": "item", "MainGameAppName": "Min", "bIsIncompleteInstall": false }"#;
        let manifest: EpicManifest = serde_json::from_str(game).unwrap();
        assert!(manifest.is_game());
        assert_eq!(manifest.launch_uri(), "com.epicgames.launcher://apps/ns%3Aitem%3AMin?action=launch&silent=true");
        let dlc: EpicManifest = serde_json::from_str(r#"{ "DisplayName": "Hades DLC", "AppName": "Dlc", "MainGameAppName": "Min" }"#).unwrap();
        assert!(!dlc.is_game());
        let partial: EpicManifest = serde_json::from_str(r#"{ "DisplayName": "Big", "AppName": "Big", "bIsIncompleteInstall": true }"#).unwrap();
        assert!(!partial.is_game());
    }

    #[test]
    #[ignore]
    fn print_games() {
        for game in installed(&Config::default()) {
            println!("{:?} {:<40} {} {:?}", game.launcher, game.name, game.install_dir.display(), game.launch);
        }
    }
}
