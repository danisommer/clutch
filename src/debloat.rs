//! Decrapify: turns off Windows data collection, ads and background extras, and uninstalls bundled apps for the current user.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::i18n::LocalizedText;
use crate::system::{self, ServiceError};

const CATALOG: &str = include_str!("../debloat.json");
const SERVICE_AUTOMATIC: u32 = 2;
const SERVICE_MANUAL: u32 = 3;
const SERVICE_DISABLED: u32 = 4;

#[derive(Clone, Debug, Deserialize)]
pub struct Catalog {
    pub items: Vec<Item>,
}

/// The catalog embedded in the executable; the elevated helper only accepts its ids.
pub fn catalog() -> Catalog {
    serde_json::from_str(CATALOG).expect("debloat.json is invalid")
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Section {
    DataCollection,
    Ads,
    Ai,
    Background,
    Apps,
}

impl Section {
    pub const ALL: [Section; 5] = [Section::DataCollection, Section::Ads, Section::Ai, Section::Background, Section::Apps];
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub section: Section,
    pub name: LocalizedText,
    pub description: LocalizedText,
    #[serde(default)]
    pub caution: bool,
    #[serde(default = "default_true")]
    pub selected_by_default: bool,
    pub targets: Vec<Target>,
}

fn default_true() -> bool {
    true
}

impl Item {
    pub fn needs_admin(&self) -> bool {
        self.targets.iter().any(Target::needs_admin)
    }

    pub fn has_apps(&self) -> bool {
        self.targets.iter().any(|t| matches!(t, Target::App { .. }))
    }

    /// Whether undo brings something back; uninstalled apps only come back from the Microsoft Store.
    pub fn can_undo(&self) -> bool {
        self.targets.iter().any(|t| !matches!(t, Target::App { .. }))
    }

    /// Caution items always start unchecked.
    pub fn starts_selected(&self) -> bool {
        self.selected_by_default && !self.caution
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StartType {
    Automatic,
    Manual,
}

impl StartType {
    fn code(self) -> u32 {
        match self {
            StartType::Automatic => SERVICE_AUTOMATIC,
            StartType::Manual => SERVICE_MANUAL,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub enum Hive {
    #[serde(rename = "HKCU")]
    CurrentUser,
    #[serde(rename = "HKLM")]
    LocalMachine,
}

impl Hive {
    fn local_machine(self) -> bool {
        self == Hive::LocalMachine
    }
}

/// One thing an item changes. Applying disables services and tasks, writes `apply` and uninstalls apps.
/// Undoing goes back to the Windows default; a `null` restore value means the value is deleted.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Target {
    Service { name: String, restore: StartType },
    Task { path: String },
    Registry { hive: Hive, key: String, value: String, apply: u32, restore: Option<u32> },
    App { package: String },
}

impl Target {
    /// Services, tasks, machine settings and policies need administrator rights; the user's own settings and apps do not.
    pub fn needs_admin(&self) -> bool {
        match self {
            Target::Service { .. } | Target::Task { .. } => true,
            Target::Registry { hive: Hive::LocalMachine, .. } => true,
            Target::Registry { key, .. } => key.to_lowercase().starts_with(r"software\policies\"),
            Target::App { .. } => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TargetState {
    Applied,
    Default,
    Absent,
}

/// What an item looks like on this computer. `Absent` means none of its targets exists, so there is nothing to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemState {
    Applied,
    Partial,
    NotApplied,
    Absent,
}

fn target_state(target: &Target, installed: &HashSet<String>) -> TargetState {
    match target {
        Target::Service { name, .. } => match system::service_start_type(name) {
            None => TargetState::Absent,
            Some(SERVICE_DISABLED) => TargetState::Applied,
            Some(_) => TargetState::Default,
        },
        Target::Task { path } => match system::task_enabled(path) {
            None => TargetState::Absent,
            Some(false) => TargetState::Applied,
            Some(true) => TargetState::Default,
        },
        Target::Registry { hive, key, value, apply, .. } => match system::registry_u32(hive.local_machine(), key, value) {
            Some(current) if current == *apply => TargetState::Applied,
            _ => TargetState::Default,
        },
        Target::App { package } => match installed.contains(&package.to_lowercase()) {
            true => TargetState::Default,
            false => TargetState::Absent,
        },
    }
}

fn item_state(states: &[TargetState]) -> ItemState {
    let present = states.iter().filter(|s| **s != TargetState::Absent).count();
    let applied = states.iter().filter(|s| **s == TargetState::Applied).count();
    match (present, applied) {
        (0, _) => ItemState::Absent,
        (p, a) if p == a => ItemState::Applied,
        (_, 0) => ItemState::NotApplied,
        _ => ItemState::Partial,
    }
}

/// State of every item on this computer, by item id.
pub fn analyze(catalog: &Catalog) -> HashMap<String, ItemState> {
    let installed = match catalog.items.iter().any(Item::has_apps) {
        true => system::installed_packages(),
        false => HashSet::new(),
    };
    catalog
        .items
        .iter()
        .map(|item| {
            let states: Vec<TargetState> = item.targets.iter().map(|t| target_state(t, &installed)).collect();
            (item.id.clone(), item_state(&states))
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Action {
    Apply,
    Undo,
}

/// What did not work for one target, without text, so the window can show it in its language.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Failure {
    Service { name: String, code: u32 },
    Task { path: String },
    Registry { value: String, error: String },
    App { package: String },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Outcome {
    pub id: String,
    pub failures: Vec<Failure>,
}

fn service_code(error: ServiceError) -> u32 {
    match error {
        ServiceError::NoManager(code) | ServiceError::Unavailable(code) | ServiceError::NotChanged(code) => code,
    }
}

fn change_service(name: &str, action: Action, restore: StartType) -> Result<(), Failure> {
    let failure = |error| Failure::Service { name: name.to_string(), code: service_code(error) };
    match action {
        Action::Apply => {
            system::set_service_start_type(name, SERVICE_DISABLED).map_err(failure)?;
            system::stop_service(name).map(|_| ()).map_err(failure)
        }
        Action::Undo => {
            system::set_service_start_type(name, restore.code()).map_err(failure)?;
            if restore == StartType::Automatic {
                system::start_service(name);
            }
            Ok(())
        }
    }
}

fn change_registry(hive: Hive, key: &str, value: &str, data: Option<u32>) -> Result<(), Failure> {
    let result = match data {
        Some(data) => system::set_registry_u32(hive.local_machine(), key, value, data),
        None => system::delete_registry_value(hive.local_machine(), key, value),
    };
    result.map_err(|e| Failure::Registry { value: value.to_string(), error: e.to_string() })
}

/// Applies or undoes one service, task or registry target. Apps are handled together by `remove_apps`.
fn change(target: &Target, action: Action) -> Result<(), Failure> {
    match (target, action) {
        (Target::Service { name, restore }, _) => change_service(name, action, *restore),
        (Target::Task { path }, _) => match system::set_task_enabled(path, action == Action::Undo) {
            true => Ok(()),
            false => Err(Failure::Task { path: path.clone() }),
        },
        (Target::Registry { hive, key, value, apply, .. }, Action::Apply) => change_registry(*hive, key, value, Some(*apply)),
        (Target::Registry { hive, key, value, restore, .. }, Action::Undo) => change_registry(*hive, key, value, *restore),
        (Target::App { .. }, _) => Ok(()),
    }
}

/// A Microsoft Store package name, safe to put in a PowerShell command.
fn is_package_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
}

/// Uninstalls the apps of the chosen items for the current user in one go, then reports the ones that are still installed.
fn remove_apps(chosen: &[&Item], outcomes: &mut [Outcome]) {
    let packages: Vec<&str> = chosen
        .iter()
        .flat_map(|item| item.targets.iter())
        .filter_map(|t| match t {
            Target::App { package } if is_package_name(package) => Some(package.as_str()),
            _ => None,
        })
        .collect();
    if packages.is_empty() {
        return;
    }
    let still_installed = match system::remove_packages(&packages) {
        Ok(()) => system::installed_packages(),
        Err(_) => packages.iter().map(|p| p.to_lowercase()).collect(),
    };
    for (item, outcome) in chosen.iter().zip(outcomes.iter_mut()) {
        let failed = item.targets.iter().filter_map(|t| match t {
            Target::App { package } if still_installed.contains(&package.to_lowercase()) => Some(Failure::App { package: package.clone() }),
            _ => None,
        });
        outcome.failures.extend(failed);
    }
}

/// Applies or undoes the chosen items. Elevated, only the targets that need administrator rights run; otherwise only the others.
pub fn run(catalog: &Catalog, ids: &[String], action: Action, elevated: bool) -> Vec<Outcome> {
    let chosen: Vec<&Item> = catalog.items.iter().filter(|i| ids.contains(&i.id)).collect();
    let mut outcomes: Vec<Outcome> = chosen
        .iter()
        .map(|item| Outcome {
            id: item.id.clone(),
            failures: item
                .targets
                .iter()
                .filter(|t| t.needs_admin() == elevated && !matches!(t, Target::App { .. }))
                .filter_map(|t| change(t, action).err())
                .collect(),
        })
        .collect();
    if let (Action::Apply, false) = (action, elevated) {
        remove_apps(&chosen, &mut outcomes);
    }
    outcomes
}

/// Microsoft Store search for an app, used to reinstall what was uninstalled here.
pub fn store_search_uri(query: &str) -> String {
    let encoded: String = query
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("ms-windows-store://search/?query={encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_valid() {
        let c = catalog();
        let ids: HashSet<&str> = c.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids.len(), c.items.len(), "duplicate ids in debloat.json");
        for item in &c.items {
            assert!(item.name.is_translated() && item.description.is_translated(), "{} needs pt-BR and en texts", item.id);
            assert!(!item.targets.is_empty(), "{} has no targets", item.id);
            for target in &item.targets {
                match target {
                    Target::Service { name, .. } => assert!(!name.is_empty()),
                    Target::Task { path } => assert!(path.starts_with('\\'), "{path} must be a full task path"),
                    Target::Registry { key, value, .. } => assert!(!key.starts_with('\\') && !value.is_empty(), "{key}"),
                    Target::App { package } => assert!(is_package_name(package), "{package} is not a plain package name"),
                }
            }
        }
    }

    #[test]
    fn combines_target_states() {
        use TargetState::*;
        assert_eq!(item_state(&[Applied, Applied]), ItemState::Applied);
        assert_eq!(item_state(&[Applied, Absent]), ItemState::Applied);
        assert_eq!(item_state(&[Applied, Default]), ItemState::Partial);
        assert_eq!(item_state(&[Default, Absent]), ItemState::NotApplied);
        assert_eq!(item_state(&[Absent, Absent]), ItemState::Absent);
    }

    #[test]
    fn policies_need_admin_but_user_settings_do_not() {
        let target = |hive, key: &str| Target::Registry { hive, key: key.into(), value: "x".into(), apply: 0, restore: None };
        assert!(target(Hive::CurrentUser, r"Software\Policies\Microsoft\Edge").needs_admin());
        assert!(target(Hive::LocalMachine, r"SOFTWARE\Microsoft\Windows").needs_admin());
        assert!(!target(Hive::CurrentUser, r"Software\Microsoft\Windows\CurrentVersion\AdvertisingInfo").needs_admin());
        assert!(!Target::App { package: "Microsoft.BingNews".into() }.needs_admin());
    }

    #[test]
    fn encodes_store_search() {
        assert_eq!(store_search_uri("Hub de Comentários"), "ms-windows-store://search/?query=Hub%20de%20Coment%C3%A1rios");
    }
}
