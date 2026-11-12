//! Installed games from Steam, Epic Games, GOG, the EA app, Battle.net and the Xbox app, plus executables the user added by hand.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::storage::{ChosenGame, Config, CustomGame, Launcher};
use crate::system::InstalledProgram;
use crate::{steam, system};

const EPIC_MANIFESTS: &str = r"Epic\EpicGamesLauncher\Data\Manifests";
const GOG_GAMES_KEY: &str = r"SOFTWARE\GOG.com\Games";
const EA_SETTINGS: &str = r"EA Desktop\machine.ini";
const EA_LIBRARY_SETTING: &str = "machine.downloadinplacedir=";
const EA_UNINSTALLER_FOLDER: &str = r"\eainstaller\";
const EA_MANIFEST: &str = r"__Installer\installerdata.xml";
const BATTLE_NET: &str = "Battle.net";
const BATTLE_NET_CLIENT: &str = "Battle.net.exe";
const XBOX_LIBRARY_FILE: &str = ".GamingRoot";
const XBOX_LIBRARY_HEADER: &[u8] = b"RGBX";
const XBOX_PACKAGES_KEY: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";

/// Battle.net games, as (code the client launches them by, start of the id in their uninstall command).
/// An id that starts another one comes after it, so the longer one is found first.
const BATTLE_NET_PRODUCTS: [(&str, &str); 31] = [
    ("WoW", "wow"),
    ("D3", "diablo3"),
    ("S2", "s2"),
    ("S1", "s1"),
    ("WTCG", "hs_beta"),
    ("Hero", "heroes"),
    ("Pro", "prometheus"),
    ("VIPR", "viper"),
    ("ODIN", "odin"),
    ("W3", "w3"),
    ("LAZR", "lazarus"),
    ("ZEUS", "zeus"),
    ("WLBY", "wlby"),
    ("OSI", "osi"),
    ("RTRO", "rtro"),
    ("FORE", "fore"),
    ("ANBS", "anbs"),
    ("AUKS", "auks"),
    ("Fen", "fen"),
    ("D1", "d1"),
    ("W1R", "w1r"),
    ("W2R", "w2r"),
    ("W1", "w1"),
    ("W2", "w2"),
    ("GRY", "gryphon"),
    ("ARIS", "aris"),
    ("SCOR", "scorpio"),
    ("ARK", "arkansas"),
    ("LBRA", "libra"),
    ("PNTA", "pinta"),
    ("AQUA", "aqua"),
];

/// How a game is started: through its launcher's URI, or by running a program in its own folder.
#[derive(Clone, Debug)]
pub enum Launch {
    Uri(String),
    Program { exe: PathBuf, args: String, dir: PathBuf },
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
            Launch::Program { exe, args, dir } => system::shell_open_in(&exe.to_string_lossy(), args, dir),
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
            let launch = Launch::Program { exe: exe.clone(), args: String::new(), dir };
            (install_dir.is_dir() && exe.is_file()).then(|| Game { launcher: Launcher::Gog, id, name, install_dir, launch })
        })
        .collect()
}

fn read_text(path: &Path) -> Option<String> {
    fs::read(path).ok().map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

fn folder_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default()
}

/// A folder path in a form that compares equal however it was written.
fn folder_key(path: &Path) -> String {
    path.to_string_lossy().trim_end_matches('\\').to_lowercase()
}

/// XML text without its comments, which often hold examples of the same elements.
fn without_comments(xml: &str) -> String {
    let mut text = String::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<!--") {
        text.push_str(&rest[..start]);
        rest = rest[start..].find("-->").map(|end| &rest[start + end + 3..]).unwrap_or("");
    }
    text.push_str(rest);
    text
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Value of an attribute of the first element with this name.
fn xml_attribute(xml: &str, element: &str, attribute: &str) -> Option<String> {
    let open = format!("<{element}");
    let start = xml.match_indices(&open).map(|(at, _)| at + open.len()).find(|&at| xml[at..].starts_with(char::is_whitespace))?;
    let tag = &xml[start..start + xml[start..].find('>')?];
    let marker = format!("{attribute}=\"");
    let value = tag.match_indices(&marker).find(|(at, _)| tag[..*at].ends_with(char::is_whitespace)).map(|(at, _)| at + marker.len())?;
    tag[value..].split('"').next().map(unescape)
}

/// Text of every element with this name that has no attributes.
fn xml_elements<'a>(xml: &'a str, element: &str) -> Vec<&'a str> {
    let (open, close) = (format!("<{element}>"), format!("</{element}>"));
    xml.split(open.as_str()).skip(1).filter_map(|rest| rest.split(close.as_str()).next()).collect()
}

/// The folder where the EA app installs games now, from its settings.
fn ea_library() -> Option<PathBuf> {
    let settings = read_text(&PathBuf::from(std::env::var_os("ProgramData")?).join(EA_SETTINGS))?;
    settings.lines().find_map(|line| line.strip_prefix(EA_LIBRARY_SETTING)).map(|folder| PathBuf::from(folder.trim()))
}

/// Launcher paths of an EA install manifest, relative to the game folder. Each comes after the registry value that holds that folder.
fn ea_launchers(manifest: &str) -> Vec<String> {
    xml_elements(&without_comments(manifest), "filePath")
        .into_iter()
        .filter_map(|path| path.rsplit(']').next())
        .map(|relative| relative.trim().to_string())
        .filter(|relative| !relative.is_empty())
        .collect()
}

fn ea_executable(folder: &Path) -> Option<PathBuf> {
    let manifest = read_text(&folder.join(EA_MANIFEST))?;
    ea_launchers(&manifest).into_iter().map(|relative| folder.join(relative)).find(|exe| exe.is_file())
}

/// EA app games: the folders its uninstallers point to and the ones in its library folder, when they hold an install manifest.
/// They are started by their own executable, which brings the EA app up.
fn ea_games(programs: &[InstalledProgram]) -> Vec<Game> {
    let registered = programs
        .iter()
        .filter(|p| !p.install_location.is_empty() && p.uninstall_string.to_lowercase().contains(EA_UNINSTALLER_FOLDER))
        .map(|p| PathBuf::from(&p.install_location));
    let in_library = ea_library().and_then(|folder| fs::read_dir(folder).ok()).into_iter().flat_map(|entries| entries.flatten()).map(|e| e.path());
    let mut seen = HashSet::new();
    registered
        .chain(in_library)
        .filter(|folder| seen.insert(folder_key(folder)))
        .filter_map(|folder| {
            let exe = ea_executable(&folder)?;
            let registered_name = programs.iter().find(|p| folder_key(Path::new(&p.install_location)) == folder_key(&folder)).map(|p| p.name.clone());
            Some(Game {
                launcher: Launcher::Ea,
                id: folder_key(&folder),
                name: registered_name.unwrap_or_else(|| folder_name(&folder)),
                launch: Launch::Program { exe, args: String::new(), dir: folder.clone() },
                install_dir: folder,
            })
        })
        .collect()
}

/// The product id in a Battle.net uninstall command, as in `--uid=prometheus_enus --lang=enUS`.
fn battle_net_uid(uninstall: &str) -> Option<&str> {
    let (before, after) = uninstall.split_once("--uid=")?;
    before.contains(BATTLE_NET).then(|| after.split_whitespace().next()).flatten()
}

fn battle_net_code(uid: &str) -> Option<&'static str> {
    let uid = uid.to_lowercase();
    BATTLE_NET_PRODUCTS.iter().find(|(_, id)| uid.starts_with(id)).map(|(code, _)| *code)
}

fn battle_net_client(programs: &[InstalledProgram]) -> Option<PathBuf> {
    let registered = programs.iter().find(|p| p.name == BATTLE_NET && !p.install_location.is_empty()).map(|p| PathBuf::from(&p.install_location));
    let default = std::env::var_os("ProgramFiles(x86)").map(|folder| PathBuf::from(folder).join(BATTLE_NET));
    registered.into_iter().chain(default).map(|folder| folder.join(BATTLE_NET_CLIENT)).find(|exe| exe.is_file())
}

/// Battle.net games, from their uninstall commands. The Battle.net client starts them by their code.
fn battle_net_games(programs: &[InstalledProgram]) -> Vec<Game> {
    let Some(client) = battle_net_client(programs) else { return Vec::new() };
    let client_dir = client.parent().map(Path::to_path_buf).unwrap_or_default();
    programs
        .iter()
        .filter_map(|p| {
            let code = battle_net_code(battle_net_uid(&p.uninstall_string)?)?;
            let install_dir = PathBuf::from(&p.install_location);
            let launch = Launch::Program { exe: client.clone(), args: format!("--exec=\"launch {code}\""), dir: client_dir.clone() };
            install_dir.is_dir().then(|| Game { launcher: Launcher::BattleNet, id: code.to_string(), name: p.name.clone(), install_dir, launch })
        })
        .collect()
}

/// The folder named in a `.GamingRoot` file: a header of 8 bytes that starts with `RGBX`, then the name in UTF-16.
fn gaming_root_folder(bytes: &[u8]) -> Option<String> {
    let name = bytes.strip_prefix(XBOX_LIBRARY_HEADER)?.get(4..)?;
    let units: Vec<u16> = name.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).take_while(|&unit| unit != 0).collect();
    String::from_utf16(&units).ok().filter(|folder| !folder.is_empty())
}

/// Folders where the Xbox app installs games: the one named at the root of each drive that has one.
fn xbox_libraries() -> Vec<PathBuf> {
    (b'A'..=b'Z')
        .filter_map(|letter| {
            let drive = PathBuf::from(format!("{}:\\", letter as char));
            let folder = gaming_root_folder(&fs::read(drive.join(XBOX_LIBRARY_FILE)).ok()?)?;
            Some(drive.join(folder))
        })
        .collect()
}

/// `Name_PublisherId` of the installed package with this name. Full names are `Name_Version_Architecture_ResourceId_PublisherId`.
fn package_family(identity: &str, packages: &[String]) -> Option<String> {
    let prefix = format!("{identity}_").to_lowercase();
    let full = packages.iter().find(|full| full.to_lowercase().starts_with(&prefix))?;
    full.rsplit('_').next().map(|publisher| format!("{identity}_{publisher}"))
}

/// An Xbox app game, from the two manifests in its `Content` folder. It is started as the Microsoft Store app it is.
fn xbox_game(folder: &Path, packages: &[String]) -> Option<Game> {
    let content = folder.join("Content");
    let config = without_comments(&read_text(&content.join("MicrosoftGame.config"))?);
    let manifest = without_comments(&read_text(&content.join("appxmanifest.xml"))?);
    let family = package_family(&xml_attribute(&config, "Identity", "Name")?, packages)?;
    let app = xml_attribute(&manifest, "Application", "Id")?;
    let name = xml_attribute(&config, "ShellVisuals", "DefaultDisplayName").filter(|name| !name.is_empty() && !name.starts_with("ms-resource:"));
    Some(Game {
        launcher: Launcher::Xbox,
        launch: Launch::Uri(format!(r"shell:AppsFolder\{family}!{app}")),
        id: family,
        name: name.unwrap_or_else(|| folder_name(folder)),
        install_dir: content,
    })
}

fn xbox_games() -> Vec<Game> {
    let packages = system::registry_subkey_names(false, XBOX_PACKAGES_KEY);
    xbox_libraries()
        .iter()
        .filter_map(|library| fs::read_dir(library).ok())
        .flat_map(|entries| entries.flatten())
        .filter_map(|entry| xbox_game(&entry.path(), &packages))
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
        launch: Launch::Program { exe, args: String::new(), dir },
    })
}

/// Every game that can be launched, sorted by name.
pub fn installed(config: &Config) -> Vec<Game> {
    let programs = system::uninstall_entries();
    let mut list = steam_games();
    list.extend(epic_games());
    list.extend(gog_games());
    list.extend(ea_games(&programs));
    list.extend(battle_net_games(&programs));
    list.extend(xbox_games());
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
    fn reads_ea_launchers() {
        let manifest = r#"<?xml version='1.0' encoding='utf-8'?>
            <DiPManifest version="4.0">
              <runtime>
                <!-- <launcher><filePath>[HKEY_LOCAL_MACHINE\SOFTWARE\Old\Install Dir]old.exe</filePath></launcher> -->
                <launcher uid="1-1">
                  <filePath>[HKEY_LOCAL_MACHINE\SOFTWARE\Respawn\Apex\Install Dir]EasyAntiCheat_launcher.exe</filePath>
                </launcher>
                <launcher uid="1-2">
                  <filePath>[HKEY_LOCAL_MACHINE\SOFTWARE\Respawn\Apex\Install Dir]bin\r5apex.exe</filePath>
                </launcher>
              </runtime>
            </DiPManifest>"#;
        assert_eq!(ea_launchers(manifest), ["EasyAntiCheat_launcher.exe", r"bin\r5apex.exe"]);
        assert!(ea_launchers("<DiPManifest></DiPManifest>").is_empty());
    }

    #[test]
    fn reads_battle_net_products() {
        let uninstall = r#""C:\ProgramData\Battle.net\Agent\Blizzard Uninstaller.exe" --lang=enUS --uid=prometheus_enus --displayname="Overwatch""#;
        assert_eq!(battle_net_uid(uninstall), Some("prometheus_enus"));
        assert_eq!(battle_net_uid(r#""C:\Other\Uninstaller.exe" --uid=wow"#), None);
        assert_eq!(battle_net_uid(r#""C:\ProgramData\Battle.net\Agent\Blizzard Uninstaller.exe""#), None);
        assert_eq!(battle_net_code("prometheus_enus"), Some("Pro"));
        assert_eq!(battle_net_code("WoW_classic"), Some("WoW"));
        assert_eq!(battle_net_code("w1r"), Some("W1R"));
        assert_eq!(battle_net_code("w1"), Some("W1"));
        assert_eq!(battle_net_code("unknown"), None);
    }

    #[test]
    fn longer_battle_net_ids_come_before_the_ones_they_start_with() {
        for (index, (_, id)) in BATTLE_NET_PRODUCTS.iter().enumerate() {
            let shadowed = BATTLE_NET_PRODUCTS[index + 1..].iter().find(|(_, later)| later.starts_with(id));
            assert!(shadowed.is_none(), "{id} hides {shadowed:?}");
        }
    }

    #[test]
    fn reads_the_xbox_library_file() {
        let mut bytes = b"RGBX\x01\x00\x00\x00".to_vec();
        bytes.extend("XboxGames\0".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(gaming_root_folder(&bytes), Some("XboxGames".to_string()));
        assert_eq!(gaming_root_folder(b"RGBX\x01\x00\x00\x00"), None);
        assert_eq!(gaming_root_folder(b"other"), None);
    }

    #[test]
    fn reads_xbox_manifests() {
        let config = without_comments(
            r#"<Game configVersion="1">
                 <Identity Name="Microsoft.4297127D64EC6" Publisher="CN=Microsoft Corporation" Version="2.6.2.0" />
                 <ExecutableList>
                   <Executable Name="Minecraft.exe" TargetDeviceFamily="PC" />
                   <!-- <Identity Name="Example" /> OverrideDisplayName="Xbox Game Override" -->
                 </ExecutableList>
                 <ShellVisuals OverrideDisplayName="Wrong" DefaultDisplayName="Ratchet &amp; Clank" />
               </Game>"#,
        );
        assert_eq!(xml_attribute(&config, "Identity", "Name").as_deref(), Some("Microsoft.4297127D64EC6"));
        assert_eq!(xml_attribute(&config, "ShellVisuals", "DefaultDisplayName").as_deref(), Some("Ratchet & Clank"));
        assert_eq!(xml_attribute(&config, "ShellVisuals", "DisplayName"), None);
        assert_eq!(xml_attribute(&config, "Missing", "Name"), None);
        let packages = ["Other.App_1.0.0.0_x64__abc".to_string(), "Microsoft.4297127D64EC6_2.6.2.0_x64__8wekyb3d8bbwe".to_string()];
        assert_eq!(package_family("Microsoft.4297127D64EC6", &packages).as_deref(), Some("Microsoft.4297127D64EC6_8wekyb3d8bbwe"));
        assert_eq!(package_family("Not.Installed", &packages), None);
    }

    #[test]
    #[ignore]
    fn print_games() {
        for game in installed(&Config::default()) {
            println!("{:?} {:<40} {} {:?}", game.launcher, game.name, game.install_dir.display(), game.launch);
        }
    }
}
