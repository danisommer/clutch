//! Cleanup engine: catalog, safety rules, analysis and removal.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::{steam, storage, system};

const CATALOG: &str = include_str!("../categories.json");
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

#[derive(Clone, Debug, Deserialize)]
pub struct Catalog {
    pub categories: Vec<Category>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Group {
    System,
    Programs,
    Leftovers,
    Caution,
}

impl Group {
    pub const ALL: [Group; 4] = [Group::System, Group::Programs, Group::Leftovers, Group::Caution];

    pub fn label(self) -> &'static str {
        match self {
            Group::System => "Sistema",
            Group::Programs => "Programas",
            Group::Leftovers => "Restos",
            Group::Caution => "Atenção",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub id: String,
    pub name: String,
    pub description: String,
    pub group: Group,
    #[serde(default = "default_true")]
    pub selected_by_default: bool,
    #[serde(default)]
    pub slow: bool,
    #[serde(default)]
    pub blocked_by: Vec<BlockingProgram>,
    pub targets: Vec<Target>,
}

fn default_true() -> bool {
    true
}

/// A program that must be closed before its category can be cleaned.
#[derive(Clone, Debug, Deserialize)]
pub struct BlockingProgram {
    pub process: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub path: String,
    pub mode: Mode,
    #[serde(default)]
    pub admin: bool,
    #[serde(default)]
    pub age_hours: u64,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub depth: Option<u32>,
    #[serde(default)]
    pub not_installed: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Contents,
    Folder,
    File,
    InactiveFolders,
    AllButNewest,
    StaleVsCodeExtensions,
    SteamLeftovers,
    WindowsUpdate,
    DeliveryOptimization,
    ComponentStore,
}

pub fn catalog() -> Catalog {
    serde_json::from_str(CATALOG).expect("categories.json is invalid")
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Tally {
    pub bytes: u64,
    pub files: u64,
    pub skipped: u64,
}

impl Tally {
    pub fn add(&mut self, other: Tally) {
        self.bytes += other.bytes;
        self.files += other.files;
        self.skipped += other.skipped;
    }
}

fn is_reparse_point(meta: &fs::Metadata) -> bool {
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn handle_file(path: &Path, meta: &fs::Metadata, delete: bool, tally: &mut Tally) {
    let size = meta.len();
    if !delete {
        tally.bytes += size;
        tally.files += 1;
        return;
    }
    if meta.permissions().readonly() {
        let mut permissions = meta.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        let _ = fs::set_permissions(path, permissions);
    }
    match fs::remove_file(path) {
        Ok(()) => {
            tally.bytes += size;
            tally.files += 1;
        }
        Err(_) => tally.skipped += 1,
    }
}

fn walk(folder: &Path, cutoff: SystemTime, exclude: &[String], delete: bool, tally: &mut Tally) {
    let Ok(entries) = fs::read_dir(folder) else {
        tally.skipped += 1;
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if exclude.iter().any(|e| e.eq_ignore_ascii_case(&name)) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            tally.skipped += 1;
            continue;
        };
        if is_reparse_point(&meta) {
            continue;
        }
        let path = entry.path();
        if meta.is_dir() {
            walk(&path, cutoff, &[], delete, tally);
            if delete {
                let _ = fs::remove_dir(&path);
            }
            continue;
        }
        if meta.modified().map(|m| m <= cutoff).unwrap_or(false) {
            handle_file(&path, &meta, delete, tally);
        }
    }
}

fn age_cutoff(hours: u64) -> SystemTime {
    SystemTime::now() - Duration::from_secs(hours * 3600)
}

/// Measures (`delete == false`) or deletes files older than `age_hours` under `path`, never following
/// junctions or symlinks. `remove_root` also removes `path` itself once it is empty.
fn sweep(path: &Path, age_hours: u64, exclude: &[String], delete: bool, remove_root: bool) -> Tally {
    let mut tally = Tally::default();
    let Ok(meta) = fs::symlink_metadata(path) else { return tally };
    if is_reparse_point(&meta) {
        return tally;
    }
    let cutoff = age_cutoff(age_hours);
    if meta.is_file() {
        if meta.modified().map(|m| m <= cutoff).unwrap_or(false) {
            handle_file(path, &meta, delete, &mut tally);
        }
        return tally;
    }
    walk(path, cutoff, exclude, delete, &mut tally);
    if delete && remove_root {
        let _ = fs::remove_dir(path);
    }
    tally
}

fn last_write(folder: &Path) -> SystemTime {
    let mut newest = fs::metadata(folder).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
    let mut stack = vec![folder.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else { continue };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if is_reparse_point(&meta) {
                continue;
            }
            if let Ok(m) = meta.modified() {
                newest = newest.max(m);
            }
            if meta.is_dir() {
                stack.push(entry.path());
            }
        }
    }
    newest
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var(name).ok().filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn variable(name: &str) -> Option<PathBuf> {
    match name {
        "TEMP" => Some(std::env::temp_dir()),
        "STEAM" => steam::steam_dir(),
        "WINDIR" => env_path("SystemRoot").or_else(|| env_path("WINDIR")),
        "PROGRAMFILES" => env_path("ProgramFiles"),
        "PROGRAMFILES(X86)" => env_path("ProgramFiles(x86)"),
        other => env_path(other),
    }
}

fn expand_variables(text: &str) -> Option<String> {
    let mut result = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        let end = rest[start + 1..].find('%')? + start + 1;
        result.push_str(&rest[..start]);
        result.push_str(&variable(&rest[start + 1..end])?.to_string_lossy());
        rest = &rest[end + 1..];
    }
    result.push_str(rest);
    Some(result)
}

fn matches_pattern(name: &str, pattern: &str) -> bool {
    let name = name.to_lowercase();
    let parts: Vec<String> = pattern.to_lowercase().split('*').map(String::from).collect();
    let mut position = 0;
    for (i, part) in parts.iter().enumerate() {
        match name[position..].find(part.as_str()) {
            Some(found) if i > 0 || found == 0 => position += found + part.len(),
            _ => return false,
        }
    }
    parts.last().is_none_or(|last| last.is_empty() || name.ends_with(last.as_str()))
}

/// Resolves the variables and `*` wildcards of a catalog path into the paths that exist.
fn expand(text: &str) -> Vec<PathBuf> {
    let Some(full) = expand_variables(text) else { return Vec::new() };
    let mut segments = full.split('\\').filter(|s| !s.is_empty());
    let Some(drive) = segments.next() else { return Vec::new() };
    let mut bases = vec![PathBuf::from(format!("{drive}\\"))];
    for segment in segments {
        bases = match segment.contains('*') {
            false => bases.into_iter().map(|b| b.join(segment)).collect(),
            true => bases
                .iter()
                .filter_map(|b| fs::read_dir(b).ok())
                .flat_map(|entries| entries.flatten())
                .filter(|e| matches_pattern(&e.file_name().to_string_lossy(), segment))
                .map(|e| e.path())
                .collect(),
        };
    }
    bases.into_iter().filter(|b| b.exists()).collect()
}

fn normalize(path: &Path) -> Option<PathBuf> {
    let full = fs::canonicalize(path).ok()?;
    let text = full.to_string_lossy().to_string();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => Some(PathBuf::from(rest)),
        Some(_) => None,
        None => Some(full),
    }
}

/// A folder the cleanup may work in. `emptiable` allows clearing the folder itself;
/// `leftovers_only` restricts it to targets of uninstalled programs.
struct Root {
    path: PathBuf,
    emptiable: bool,
    leftovers_only: bool,
}

struct Rules {
    roots: Vec<Root>,
    protected: Vec<PathBuf>,
}

impl Rules {
    fn load() -> Self {
        let mut roots = Vec::new();
        let mut add = |path: Option<PathBuf>, emptiable: bool, leftovers_only: bool| {
            if let Some(p) = path.as_deref().and_then(normalize) {
                roots.push(Root { path: p, emptiable, leftovers_only });
            }
        };
        add(Some(std::env::temp_dir()), true, false);
        add(env_path("LOCALAPPDATA"), false, false);
        add(env_path("APPDATA"), false, false);
        add(env_path("ProgramData"), false, false);
        if let Some(windir) = variable("WINDIR") {
            for sub in [
                "Temp",
                "SystemTemp",
                r"SoftwareDistribution\Download",
                "LiveKernelReports",
                "Minidump",
                "MEMORY.DMP",
                r"ServiceProfiles\NetworkService\AppData\Local\Microsoft\Windows\DeliveryOptimization\Cache",
            ] {
                add(Some(windir.join(sub)), true, false);
            }
        }
        add(env_path("SystemDrive").map(|d| PathBuf::from(format!("{}\\AMD", d.display()))), true, false);
        if let Some(profile) = env_path("USERPROFILE") {
            add(Some(profile.join(r".vscode\extensions")), true, false);
            add(Some(profile.join(".warp")), true, true);
        }
        add(steam::steam_dir(), false, false);
        for library in steam::libraries() {
            add(Some(library.join(r"steamapps\common")), true, false);
        }
        add(env_path("ProgramFiles"), false, true);
        add(env_path("ProgramFiles(x86)"), false, true);

        let mut protected = system::personal_folders();
        if let Some(profile) = env_path("USERPROFILE") {
            protected.push(profile.join(".claude"));
            protected.push(profile.join(".ssh"));
        }
        if let Some(local) = env_path("LOCALAPPDATA") {
            protected.push(local.join("wsl"));
            protected.push(local.join("Packages"));
            protected.push(local.join(r"Programs\ModoJogoLimpeza"));
        }
        protected.push(storage::data_dir());
        if let Some(exe_dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
            protected.push(exe_dir);
        }
        let protected = protected.iter().filter_map(|p| normalize(p)).collect();
        Rules { roots, protected }
    }

    /// Accepts the path only if it is inside an allowed root and does not contain or sit inside a protected folder.
    fn validate(&self, path: &Path, is_leftover: bool) -> Result<PathBuf, String> {
        let p = normalize(path).ok_or_else(|| "caminho não encontrado".to_string())?;
        if self.protected.iter().any(|protected| system::is_within(&p, protected) || system::is_within(protected, &p)) {
            return Err(format!("pasta protegida: {}", p.display()));
        }
        let root = self
            .roots
            .iter()
            .filter(|r| system::is_within(&p, &r.path) && (is_leftover || !r.leftovers_only))
            .max_by_key(|r| r.path.as_os_str().len())
            .ok_or_else(|| format!("fora das pastas permitidas: {}", p.display()))?;
        let is_root_itself = system::is_within(&root.path, &p);
        match is_root_itself && !root.emptiable {
            true => Err(format!("não é permitido esvaziar {}", p.display())),
            false => Ok(p),
        }
    }
}

enum Operation {
    Contents { path: PathBuf, age_hours: u64, exclude: Vec<String> },
    Remove(PathBuf),
    WindowsUpdate(PathBuf),
    DeliveryOptimization(PathBuf),
    ComponentStore,
}

fn file_name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn valid_paths(rules: &Rules, paths: impl IntoIterator<Item = PathBuf>, is_leftover: bool) -> Vec<PathBuf> {
    paths.into_iter().filter_map(|p| rules.validate(&p, is_leftover).ok()).collect()
}

fn stale_extensions(folder: &Path) -> Vec<PathBuf> {
    let Ok(text) = fs::read_to_string(folder.join("extensions.json")) else { return Vec::new() };
    let Ok(serde_json::Value::Array(list)) = serde_json::from_str::<serde_json::Value>(&text) else { return Vec::new() };
    let installed: HashSet<String> = list
        .iter()
        .filter_map(|e| {
            e.get("relativeLocation")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| e.pointer("/location/path").and_then(|v| v.as_str()).map(|p| file_name_of(Path::new(p))))
        })
        .map(|n| n.to_lowercase())
        .collect();
    if installed.is_empty() {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(folder) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            name != ".obsolete" && !installed.contains(&name)
        })
        .map(|e| e.path())
        .collect()
}

fn steam_leftovers() -> Vec<PathBuf> {
    steam::libraries()
        .iter()
        .flat_map(|library| {
            let installed: HashSet<String> = steam::manifests(library).iter().map(|m| file_name_of(&m.install_dir).to_lowercase()).collect();
            fs::read_dir(library.join(r"steamapps\common"))
                .into_iter()
                .flat_map(|entries| entries.flatten())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .filter(move |e| !installed.contains(&e.file_name().to_string_lossy().to_lowercase()))
                .map(|e| e.path())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn folders_at_depth(base: &Path, depth: u32) -> Vec<PathBuf> {
    let mut level = vec![base.to_path_buf()];
    for _ in 0..depth {
        level = level
            .iter()
            .filter_map(|p| fs::read_dir(p).ok())
            .flat_map(|entries| entries.flatten())
            .filter(|e| e.metadata().map(|m| m.is_dir() && !is_reparse_point(&m)).unwrap_or(false))
            .map(|e| e.path())
            .collect();
    }
    level
}

fn all_but_newest(base: &Path, pattern: Option<&str>) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(base) else { return Vec::new() };
    let mut items: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| match pattern {
            Some(p) => matches_pattern(&e.file_name().to_string_lossy(), p),
            None => e.file_type().map(|t| t.is_dir()).unwrap_or(false),
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    items.sort_by(|a, b| b.0.cmp(&a.0));
    items.into_iter().skip(1).map(|(_, p)| p).collect()
}

/// Turns a catalog target into concrete, already validated operations, plus details for the UI.
fn operations(target: &Target, rules: &Rules) -> (Vec<Operation>, Vec<String>) {
    let is_leftover = !target.not_installed.is_empty();
    let paths = || expand(&target.path);
    match target.mode {
        Mode::Contents => (
            valid_paths(rules, paths(), is_leftover)
                .into_iter()
                .map(|p| Operation::Contents { path: p, age_hours: target.age_hours, exclude: target.exclude.clone() })
                .collect(),
            Vec::new(),
        ),
        Mode::Folder | Mode::File => {
            let found = valid_paths(rules, paths(), is_leftover);
            let details = match is_leftover && !found.is_empty() {
                true => target.not_installed.clone(),
                false => Vec::new(),
            };
            (found.into_iter().map(Operation::Remove).collect(), details)
        }
        Mode::InactiveFolders => {
            let cutoff = age_cutoff(target.age_hours);
            let inactive: Vec<PathBuf> = paths()
                .iter()
                .flat_map(|b| folders_at_depth(b, target.depth.unwrap_or(1)))
                .filter(|p| last_write(p) <= cutoff)
                .collect();
            let inactive = valid_paths(rules, inactive, false);
            let details = match inactive.len() {
                0 => Vec::new(),
                1 => vec!["1 sessão".into()],
                n => vec![format!("{n} sessões")],
            };
            (inactive.into_iter().map(Operation::Remove).collect(), details)
        }
        Mode::AllButNewest => {
            let older = paths().iter().flat_map(|b| all_but_newest(b, target.pattern.as_deref())).collect::<Vec<_>>();
            (valid_paths(rules, older, false).into_iter().map(Operation::Remove).collect(), Vec::new())
        }
        Mode::StaleVsCodeExtensions => {
            let stale = valid_paths(rules, paths().iter().flat_map(|b| stale_extensions(b)), false);
            let details = match stale.len() {
                0 => Vec::new(),
                1 => vec!["1 extensão desatualizada".into()],
                n => vec![format!("{n} extensões desatualizadas")],
            };
            (stale.into_iter().map(Operation::Remove).collect(), details)
        }
        Mode::SteamLeftovers => {
            let leftovers = valid_paths(rules, steam_leftovers(), false);
            let details = leftovers
                .iter()
                .map(|l| {
                    let drive = l.to_string_lossy().chars().take(2).collect::<String>();
                    let size = storage::format_size(sweep(l, 0, &[], false, false).bytes);
                    format!("{} ({drive}, {size})", file_name_of(l))
                })
                .collect();
            (leftovers.into_iter().map(Operation::Remove).collect(), details)
        }
        Mode::WindowsUpdate => (valid_paths(rules, paths(), false).into_iter().map(Operation::WindowsUpdate).collect(), Vec::new()),
        Mode::DeliveryOptimization => (valid_paths(rules, paths(), false).into_iter().map(Operation::DeliveryOptimization).collect(), Vec::new()),
        Mode::ComponentStore => (vec![Operation::ComponentStore], Vec::new()),
    }
}

fn measure(operation: &Operation) -> Option<Tally> {
    match operation {
        Operation::Contents { path, age_hours, exclude } => Some(sweep(path, *age_hours, exclude, false, false)),
        Operation::Remove(p) | Operation::WindowsUpdate(p) | Operation::DeliveryOptimization(p) => Some(sweep(p, 0, &[], false, false)),
        Operation::ComponentStore => None,
    }
}

fn execute(operation: &Operation) -> Result<Tally, String> {
    match operation {
        Operation::Contents { path, age_hours, exclude } => Ok(sweep(path, *age_hours, exclude, true, false)),
        Operation::Remove(p) => Ok(sweep(p, 0, &[], true, true)),
        Operation::WindowsUpdate(p) => {
            let was_running = system::stop_service("wuauserv")?;
            let tally = sweep(p, 0, &[], true, false);
            if was_running {
                system::start_service("wuauserv");
            }
            Ok(tally)
        }
        Operation::DeliveryOptimization(p) => {
            let before = sweep(p, 0, &[], false, false);
            system::run_hidden("powershell.exe", &["-NoProfile", "-NonInteractive", "-Command", "Delete-DeliveryOptimizationCache -Force"])
                .map_err(|e| format!("não foi possível limpar: {e}"))?;
            let after = sweep(p, 0, &[], false, false);
            Ok(Tally { bytes: before.bytes.saturating_sub(after.bytes), files: before.files.saturating_sub(after.files), skipped: 0 })
        }
        Operation::ComponentStore => {
            let before = system::free_space("C:\\").unwrap_or(0);
            let output = system::run_hidden("dism.exe", &["/Online", "/Cleanup-Image", "/StartComponentCleanup"])
                .map_err(|e| format!("não foi possível rodar o DISM: {e}"))?;
            if !output.status.success() {
                return Err(format!("o DISM terminou com código {}", output.status.code().unwrap_or(-1)));
            }
            let after = system::free_space("C:\\").unwrap_or(0);
            Ok(Tally { bytes: after.saturating_sub(before), files: 0, skipped: 0 })
        }
    }
}

/// A target with `notInstalled` applies only while none of those programs is installed.
fn applies(target: &Target, installed: &[String]) -> bool {
    !target.not_installed.iter().any(|n| {
        let n = n.to_lowercase();
        installed.iter().any(|i| i.contains(&n))
    })
}

/// Whether an admin-only target has something to measure, checked without elevation.
fn admin_target_present(target: &Target) -> bool {
    match target.mode {
        Mode::ComponentStore | Mode::WindowsUpdate | Mode::DeliveryOptimization => true,
        _ => !expand(&target.path).is_empty(),
    }
}

/// Names of the category's blocking programs that are running in this session.
pub fn open_blockers(category: &Category, processes: &[system::Process]) -> Vec<String> {
    let session = system::current_session();
    category
        .blocked_by
        .iter()
        .filter(|b| processes.iter().any(|p| p.session == session && p.name.eq_ignore_ascii_case(&b.process)))
        .map(|b| b.name.clone())
        .collect()
}

fn installed_lowercase() -> Vec<String> {
    system::installed_programs().into_iter().map(|n| n.to_lowercase()).collect()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Analysis {
    pub id: String,
    pub user_bytes: u64,
    pub admin_bytes: Option<u64>,
    pub has_admin: bool,
    pub unknown_size: bool,
    pub details: Vec<String>,
    pub blockers: Vec<String>,
}

impl Analysis {
    pub fn total(&self) -> u64 {
        self.user_bytes + self.admin_bytes.unwrap_or(0)
    }

    /// True when the admin part was found but not measured yet.
    pub fn admin_pending(&self) -> bool {
        self.has_admin && self.admin_bytes.is_none()
    }

    pub fn has_content(&self) -> bool {
        self.total() > 0 || self.unknown_size || self.admin_pending()
    }
}

/// Measures everything that can be measured without administrator rights; admin parts are flagged to be measured later.
pub fn analyze(progress: &dyn Fn(&str)) -> Vec<Analysis> {
    let rules = Rules::load();
    let installed = installed_lowercase();
    let processes = system::processes();
    catalog()
        .categories
        .iter()
        .map(|category| {
            progress(&category.name);
            let mut analysis = Analysis { id: category.id.clone(), blockers: open_blockers(category, &processes), ..Default::default() };
            for target in category.targets.iter().filter(|t| applies(t, &installed)) {
                if target.admin {
                    analysis.has_admin |= admin_target_present(target);
                    analysis.unknown_size |= target.mode == Mode::ComponentStore;
                    continue;
                }
                let (ops, details) = operations(target, &rules);
                analysis.user_bytes += ops.iter().filter_map(measure).map(|t| t.bytes).sum::<u64>();
                analysis.details.extend(details);
            }
            analysis.details.dedup();
            analysis
        })
        .collect()
}

/// Measures the admin-only parts of each category. Runs in the elevated process.
pub fn analyze_admin() -> HashMap<String, u64> {
    let rules = Rules::load();
    let installed = installed_lowercase();
    catalog()
        .categories
        .iter()
        .filter(|c| c.targets.iter().any(|t| t.admin))
        .map(|category| {
            let bytes = category
                .targets
                .iter()
                .filter(|t| t.admin && applies(t, &installed))
                .flat_map(|t| operations(t, &rules).0)
                .filter_map(|op| measure(&op))
                .map(|t| t.bytes)
                .sum();
            (category.id.clone(), bytes)
        })
        .collect()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CleanupResult {
    pub id: String,
    pub tally: Tally,
    pub error: Option<String>,
}

/// Cleans the chosen categories. Without elevation only the user targets run; elevated, only the admin targets.
pub fn clean(ids: &[String], elevated: bool, progress: &dyn Fn(&str)) -> Vec<CleanupResult> {
    let rules = Rules::load();
    let installed = installed_lowercase();
    let processes = system::processes();
    let catalog = catalog();
    ids.iter()
        .filter_map(|id| catalog.categories.iter().find(|c| &c.id == id))
        .filter(|c| c.targets.iter().any(|t| t.admin == elevated))
        .map(|category| {
            progress(&category.name);
            let mut result = CleanupResult { id: category.id.clone(), ..Default::default() };
            let blockers = open_blockers(category, &processes);
            if !blockers.is_empty() {
                result.error = Some(format!("feche: {}", blockers.join(", ")));
                return result;
            }
            for target in category.targets.iter().filter(|t| t.admin == elevated && applies(t, &installed)) {
                for operation in operations(target, &rules).0 {
                    match execute(&operation) {
                        Ok(t) => result.tally.add(t),
                        Err(e) => result.error = Some(e),
                    }
                }
            }
            result
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_matching() {
        assert!(matches_pattern("app-1.0.nupkg", "*.nupkg"));
        assert!(matches_pattern("webcache_4430", "webcache*"));
        assert!(!matches_pattern("RELEASES", "*.nupkg"));
        assert!(!matches_pattern("xwebcache", "webcache*"));
    }

    #[test]
    fn refuses_protected_folders() {
        let rules = Rules::load();
        let mut forbidden = system::personal_folders();
        let profile = env_path("USERPROFILE").unwrap();
        let local = env_path("LOCALAPPDATA").unwrap();
        forbidden.extend([profile.join(".claude"), local.join("wsl"), local.join("Packages"), local.clone(), profile.clone()]);
        for folder in forbidden.iter().filter(|p| p.exists()) {
            assert!(rules.validate(folder, false).is_err(), "should refuse {}", folder.display());
            assert!(rules.validate(folder, true).is_err(), "should refuse {} even as a leftover", folder.display());
        }
    }

    #[test]
    fn accepts_temp_and_refuses_unlisted_folders() {
        let rules = Rules::load();
        assert!(rules.validate(&std::env::temp_dir(), false).is_ok());
        assert!(rules.validate(Path::new(r"C:\Windows\System32"), false).is_err());
        assert!(rules.validate(&env_path("ProgramFiles").unwrap(), true).is_err());
    }

    fn test_folder(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!("mjl-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    fn age(path: &Path, hours: u64) {
        let when = SystemTime::now() - Duration::from_secs(hours * 3600);
        fs::File::options().write(true).open(path).unwrap().set_modified(when).unwrap();
    }

    #[test]
    fn deletes_only_old_files_and_honors_exclusions() {
        let base = test_folder("age");
        fs::create_dir_all(base.join("sub")).unwrap();
        fs::create_dir_all(base.join("keep")).unwrap();
        let old = base.join("old.txt");
        let new = base.join("new.txt");
        let old_in_sub = base.join(r"sub\a.txt");
        let excluded = base.join(r"keep\x.txt");
        fs::write(&old, b"12345").unwrap();
        fs::write(&new, b"123").unwrap();
        fs::write(&old_in_sub, b"1").unwrap();
        fs::write(&excluded, b"1").unwrap();
        for p in [&old, &old_in_sub, &excluded] {
            age(p, 48);
        }
        let exclude = vec!["keep".to_string()];
        let measured = sweep(&base, 24, &exclude, false, false);
        assert_eq!((measured.files, measured.bytes), (2, 6));
        assert!(old.exists(), "measuring must not delete");
        let done = sweep(&base, 24, &exclude, true, false);
        assert_eq!((done.files, done.bytes), (2, 6));
        assert!(!old.exists() && !old_in_sub.exists());
        assert!(new.exists() && excluded.exists());
        assert!(!base.join("sub").exists(), "the emptied subfolder should be removed");
        assert!(base.exists(), "contents mode must keep the root");
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn does_not_follow_junctions() {
        let base = test_folder("junction");
        let outside = test_folder("junction-outside");
        let important = outside.join("important.txt");
        fs::write(&important, b"do not delete").unwrap();
        age(&important, 48);
        let junction = base.join("link");
        let created = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&outside)
            .output()
            .unwrap();
        assert!(created.status.success(), "could not create the test junction");
        sweep(&base, 0, &[], true, true);
        assert!(important.exists(), "the file behind the junction was deleted");
        let _ = fs::remove_dir(&junction);
        let _ = fs::remove_dir_all(&base);
        fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn catalog_is_valid() {
        let c = catalog();
        let ids: HashSet<&str> = c.categories.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids.len(), c.categories.len(), "duplicate ids in the catalog");
    }

    #[test]
    #[ignore]
    fn print_analysis() {
        let catalog = catalog();
        for a in analyze(&|_| {}).iter().filter(|a| a.has_content()) {
            let name = &catalog.categories.iter().find(|c| c.id == a.id).unwrap().name;
            println!(
                "{:<36} {:>10} admin={:<5} unknown={:<5} blockers={:?} details={:?}",
                name,
                storage::format_size(a.total()),
                a.has_admin,
                a.unknown_size,
                a.blockers,
                a.details
            );
        }
    }
}
