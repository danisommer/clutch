//! Drivers: old packages in the driver store, driver updates from Windows Update and graphics drivers from their vendor.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{system, xml};

const CLASS_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Class";
const DISPLAY_CLASS: &str = "{4d36e968-e325-11ce-bfc1-08002be10318}";
const DRIVER_STORE: &str = r"System32\DriverStore\FileRepository";
const UTF8_OUTPUT: &str = "[Console]::OutputEncoding = [Text.Encoding]::UTF8; $ErrorActionPreference = 'Stop'";
const DRIVER_UPDATES: &str = "IsInstalled=0 and Type=''Driver''";
const AGENT: &str = concat!("Clutch/", env!("CARGO_PKG_VERSION"));
const AMD_DRIVERS_PAGE: &str = "https://www.amd.com/en/support/download/drivers.html";
const AMD_REFERER: &str = "https://www.amd.com/";
const AMD_INSTALLERS: &str = "https://drivers.amd.com/drivers/installer/";
const AMD_INSTALLER_NAME: &str = "amd-software-adrenalin-edition-";
const AMD_INSTALLER_FILE: &str = "amd-software-setup.exe";
const AMD_PUBLISHERS: [&str; 2] = ["Advanced Micro Devices", "Advanced Micro Devices, Inc."];
const NVIDIA_DRIVERS_PAGE: &str = "https://www.nvidia.com/drivers/";
const INTEL_DRIVERS_PAGE: &str = "https://www.intel.com/content/www/us/en/support/detect.html";
const UPDATE_SUCCEEDED: i32 = 2;
const UPDATE_SUCCEEDED_WITH_ERRORS: i32 = 3;

fn numbers(version: &str) -> Vec<u32> {
    version.split('.').filter_map(|part| part.trim().parse().ok()).collect()
}

fn powershell(script: &str) -> Result<String, String> {
    let output = system::run_hidden("powershell.exe", &["-NoProfile", "-NonInteractive", "-Command", script]).map_err(|e| e.to_string())?;
    match output.status.success() {
        true => Ok(String::from_utf8_lossy(&output.stdout).into_owned()),
        false => Err(first_line(&output.stderr, output.status.code())),
    }
}

fn first_line(text: &[u8], code: Option<i32>) -> String {
    let text = String::from_utf8_lossy(text);
    match text.lines().map(str::trim).find(|line| !line.is_empty()) {
        Some(line) => line.to_string(),
        None => format!("exit code {}", code.unwrap_or(-1)),
    }
}

/// A text as a PowerShell single-quoted string.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// A third-party driver package in the Windows driver store. `inf` is its published name, as in `oem7.inf`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub inf: String,
    pub original_name: String,
    pub provider: String,
    pub class_guid: String,
    pub date: (u32, u32, u32),
    pub version: Vec<u32>,
}

impl Package {
    fn same_driver(&self, other: &Package) -> bool {
        let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
        same(&self.original_name, &other.original_name) && same(&self.provider, &other.provider) && same(&self.class_guid, &other.class_guid)
    }

    fn newer_than(&self, other: &Package) -> bool {
        (self.date, &self.version) > (other.date, &other.version)
    }
}

/// Date and version of a driver, written by `pnputil` as `05/19/2026 15.11.30.14` in every language.
fn date_and_version(text: &str) -> Option<((u32, u32, u32), Vec<u32>)> {
    let (date, version) = text.trim().split_once(' ')?;
    let mut parts = date.split('/').map(|part| part.parse::<u32>().ok());
    let (month, day, year) = (parts.next()??, parts.next()??, parts.next()??);
    Some(((year, month, day), numbers(version)))
}

/// Packages listed by `pnputil /enum-drivers /format xml`.
fn parse_packages(listing: &str) -> Vec<Package> {
    listing
        .split("<Driver ")
        .skip(1)
        .filter_map(|rest| {
            let driver = format!("<Driver {}", rest.split("</Driver>").next()?);
            let text = |element: &str| xml::elements(&driver, element).first().map(|value| xml::unescape(value.trim()));
            let (date, version) = date_and_version(&text("DriverVersion")?)?;
            Some(Package {
                inf: xml::attribute(&driver, "Driver", "DriverName")?,
                original_name: text("OriginalName")?,
                provider: text("ProviderName").unwrap_or_default(),
                class_guid: text("ClassGuid").unwrap_or_default(),
                date,
                version,
            })
        })
        .collect()
}

fn packages() -> Vec<Package> {
    system::run_hidden("pnputil.exe", &["/enum-drivers", "/format", "xml"])
        .map(|output| parse_packages(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default()
}

/// Published INF names that some device, connected or not, is installed with, in lowercase.
fn infs_in_use() -> HashSet<String> {
    system::registry_subkey_names(true, CLASS_KEY)
        .iter()
        .flat_map(|class| {
            let path = format!(r"{CLASS_KEY}\{class}");
            let devices = system::registry_subkey_names(true, &path);
            devices.into_iter().filter_map(move |device| system::registry_string(true, &format!(r"{path}\{device}"), "InfPath"))
        })
        .map(|inf| inf.to_lowercase())
        .collect()
}

/// Packages no device uses that have a newer version of the same driver in the store.
fn superseded(packages: &[Package], in_use: &HashSet<String>) -> Vec<Package> {
    packages
        .iter()
        .filter(|package| !in_use.contains(&package.inf.to_lowercase()))
        .filter(|package| packages.iter().any(|other| other.same_driver(package) && other.newer_than(package)))
        .cloned()
        .collect()
}

/// Old driver packages that can leave the driver store.
pub fn superseded_packages() -> Vec<Package> {
    superseded(&packages(), &infs_in_use())
}

/// The folder of a package in the driver store: the one whose INF is the same file as the published one.
pub fn package_folder(package: &Package) -> Option<PathBuf> {
    let windows = PathBuf::from(std::env::var_os("SystemRoot")?);
    let published = fs::read(windows.join("INF").join(&package.inf)).ok()?;
    let prefix = format!("{}_", package.original_name.to_lowercase());
    fs::read_dir(windows.join(DRIVER_STORE))
        .ok()?
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().to_lowercase().starts_with(&prefix))
        .map(|entry| entry.path())
        .find(|folder| fs::read(folder.join(&package.original_name)).is_ok_and(|inf| inf == published))
}

fn is_published_inf(name: &str) -> bool {
    let number = name.strip_prefix("oem").and_then(|rest| rest.strip_suffix(".inf"));
    number.is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// Removes a package from the driver store. Windows refuses while a device uses it. Needs administrator rights.
pub fn delete_package(package: &Package) -> bool {
    is_published_inf(&package.inf) && system::run_hidden("pnputil.exe", &["/delete-driver", &package.inf]).is_ok_and(|output| output.status.success())
}

/// A driver update that Windows Update offers for this computer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Update {
    pub id: String,
    pub title: String,
    pub provider: String,
    #[serde(rename = "class")]
    pub device_class: String,
    pub date: String,
    pub bytes: u64,
}

impl Update {
    /// Firmware fails badly when interrupted, so it is always flagged.
    pub fn is_firmware(&self) -> bool {
        let title = self.title.to_lowercase();
        self.device_class.eq_ignore_ascii_case("Firmware") || title.contains("firmware") || title.contains("bios")
    }
}

/// Driver updates offered by Windows Update. It asks Microsoft's servers, which can take a minute.
pub fn search_updates() -> Result<Vec<Update>, String> {
    let script = format!(
        "{UTF8_OUTPUT}; $found = (New-Object -ComObject Microsoft.Update.Session).CreateUpdateSearcher().Search('{DRIVER_UPDATES}').Updates; \
         ConvertTo-Json -Compress -InputObject @($found | ForEach-Object {{ [pscustomobject]@{{ id = $_.Identity.UpdateID; title = [string]$_.Title; \
         provider = [string]$_.DriverProvider; class = [string]$_.DriverClass; date = $_.DriverVerDate.ToString('yyyy-MM-dd'); bytes = [int64]$_.MaxDownloadSize }} }})"
    );
    serde_json::from_str(powershell(&script)?.trim()).map_err(|e| e.to_string())
}

/// How one update ended, with the Windows Update result code.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Installed {
    pub id: String,
    pub code: i32,
}

impl Installed {
    pub fn succeeded(&self) -> bool {
        matches!(self.code, UPDATE_SUCCEEDED | UPDATE_SUCCEEDED_WITH_ERRORS)
    }
}

/// What an installation did. `restore_point` is `None` when none was asked for and `Some(false)` when Windows could not create it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InstallReport {
    pub restore_point: Option<bool>,
    pub reboot: bool,
    pub results: Vec<Installed>,
}

fn is_update_id(id: &str) -> bool {
    id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// Downloads and installs the chosen driver updates, optionally after a restore point; when Windows cannot create it,
/// nothing is installed. Only updates that a new search still offers are installed. Needs administrator rights.
pub fn install_updates(ids: &[String], restore_point: bool) -> Result<InstallReport, String> {
    let chosen: Vec<String> = ids.iter().filter(|id| is_update_id(id)).map(|id| quoted(id)).collect();
    let script = format!(
        "{UTF8_OUTPUT}; $ids = @({ids}); $restore = $null; \
         if ({restore}) {{ try {{ Checkpoint-Computer -Description 'Clutch: driver updates' -RestorePointType DEVICE_DRIVER_INSTALL; $restore = $true }} catch {{ $restore = $false }} }}; \
         $session = New-Object -ComObject Microsoft.Update.Session; $chosen = New-Object -ComObject Microsoft.Update.UpdateColl; \
         foreach ($update in $session.CreateUpdateSearcher().Search('{DRIVER_UPDATES}').Updates) {{ if ($ids -contains $update.Identity.UpdateID) {{ \
         if (-not $update.EulaAccepted) {{ $update.AcceptEula() }}; [void]$chosen.Add($update) }} }}; \
         $results = @(); $reboot = $false; \
         if ($restore -ne $false -and $chosen.Count -gt 0) {{ $downloader = $session.CreateUpdateDownloader(); $downloader.Updates = $chosen; [void]$downloader.Download(); \
         $installer = $session.CreateUpdateInstaller(); $installer.Updates = $chosen; $outcome = $installer.Install(); $reboot = [bool]$outcome.RebootRequired; \
         for ($i = 0; $i -lt $chosen.Count; $i++) {{ $results += [pscustomobject]@{{ id = $chosen.Item($i).Identity.UpdateID; code = [int]$outcome.GetUpdateResult($i).ResultCode }} }} }}; \
         ConvertTo-Json -Compress -Depth 4 -InputObject ([pscustomobject]@{{ restore_point = $restore; reboot = $reboot; results = @($results) }})",
        ids = chosen.join(","),
        restore = match restore_point {
            true => "$true",
            false => "$false",
        },
    );
    serde_json::from_str(powershell(&script)?.trim()).map_err(|e| e.to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vendor {
    Amd,
    Nvidia,
    Intel,
}

/// A graphics adapter and its driver. `software_version` is the AMD Software release, when the driver came with one.
#[derive(Clone, Debug)]
pub struct Gpu {
    pub name: String,
    pub vendor: Vendor,
    pub driver_version: String,
    pub driver_date: String,
    pub software_version: Option<String>,
}

fn vendor_of(hardware_id: &str) -> Option<Vendor> {
    let id = hardware_id.to_lowercase();
    match () {
        _ if id.contains("ven_1002") => Some(Vendor::Amd),
        _ if id.contains("ven_10de") => Some(Vendor::Nvidia),
        _ if id.contains("ven_8086") => Some(Vendor::Intel),
        _ => None,
    }
}

/// A registry driver date, written as month-day-year, in year-month-day order.
fn iso_date(registry_date: &str) -> String {
    let parts: Vec<&str> = registry_date.split('-').collect();
    match parts.as_slice() {
        [month, day, year] => format!("{year}-{month:0>2}-{day:0>2}"),
        _ => registry_date.to_string(),
    }
}

/// Graphics adapters from AMD, NVIDIA and Intel, with the driver each one is installed with.
pub fn gpus() -> Vec<Gpu> {
    let class = format!(r"{CLASS_KEY}\{DISPLAY_CLASS}");
    system::registry_subkey_names(true, &class)
        .iter()
        .filter_map(|device| {
            let key = format!(r"{class}\{device}");
            let value = |name: &str| system::registry_string(true, &key, name);
            Some(Gpu {
                vendor: vendor_of(&value("MatchingDeviceId")?)?,
                name: value("DriverDesc")?,
                driver_version: value("DriverVersion").unwrap_or_default(),
                driver_date: value("DriverDate").map(|date| iso_date(&date)).unwrap_or_default(),
                software_version: value("RadeonSoftwareVersion"),
            })
        })
        .collect()
}

fn curl(args: &[&str]) -> Result<Vec<u8>, String> {
    let output = system::run_hidden("curl.exe", args).map_err(|e| e.to_string())?;
    match output.status.success() {
        true => Ok(output.stdout),
        false => Err(first_line(&output.stderr, output.status.code())),
    }
}

/// Version and address of AMD's web installer, as linked in its drivers page.
fn amd_installer_link(page: &str) -> Option<(String, String)> {
    page.match_indices(AMD_INSTALLERS).find_map(|(at, _)| {
        let address = page[at..].split(|c: char| matches!(c, '"' | '\'' | '<' | '>') || c.is_whitespace()).next()?;
        let version = address.split(AMD_INSTALLER_NAME).nth(1)?.split('-').next()?;
        (address.ends_with("_web.exe") && numbers(version).len() == 3).then(|| (version.to_string(), address.to_string()))
    })
}

/// The latest AMD Software release, as (version, address of its web installer), read from AMD's site.
fn latest_amd_software() -> Result<(String, String), String> {
    let page = curl(&["--silent", "--show-error", "--fail", "--location", "--max-time", "30", "--user-agent", AGENT, AMD_DRIVERS_PAGE])?;
    amd_installer_link(&String::from_utf8_lossy(&page)).ok_or_else(|| AMD_DRIVERS_PAGE.to_string())
}

/// What can be done about the driver of a graphics adapter.
#[derive(Clone, Debug)]
pub enum GpuUpdate {
    UpToDate,
    Available { version: String, address: String },
    VendorPage(&'static str),
    Unknown(String),
}

/// Compares the installed AMD driver with the latest release on AMD's site. NVIDIA and Intel only get their drivers page.
pub fn gpu_update(gpu: &Gpu) -> GpuUpdate {
    match (gpu.vendor, &gpu.software_version) {
        (Vendor::Nvidia, _) => GpuUpdate::VendorPage(NVIDIA_DRIVERS_PAGE),
        (Vendor::Intel, _) => GpuUpdate::VendorPage(INTEL_DRIVERS_PAGE),
        (Vendor::Amd, installed) => match (latest_amd_software(), installed) {
            (Ok((version, _)), Some(installed)) if numbers(&version) <= numbers(installed) => GpuUpdate::UpToDate,
            (Ok((version, address)), _) => GpuUpdate::Available { version, address },
            (Err(error), _) => GpuUpdate::Unknown(error),
        },
    }
}

/// Why the vendor's installer was not started.
#[derive(Clone, Debug)]
pub enum InstallerError {
    Download(String),
    NotSigned,
    NotStarted,
}

/// Whether Windows finds in the file a valid signature whose certificate is named exactly as one of these publishers.
fn signed_by(file: &Path, publishers: &[&str]) -> bool {
    let script = format!(
        "{UTF8_OUTPUT}; $signature = Get-AuthenticodeSignature -LiteralPath {}; \
         if ($signature.Status -eq 'Valid') {{ $signature.SignerCertificate.GetNameInfo('SimpleName', $false) }}",
        quoted(&file.to_string_lossy())
    );
    powershell(&script).is_ok_and(|name| publishers.contains(&name.trim()))
}

/// Downloads AMD's web installer to `folder` and checks that AMD signed it. A file that fails the check is deleted.
fn download_amd_installer(address: &str, folder: &Path) -> Result<PathBuf, InstallerError> {
    let file = folder.join(AMD_INSTALLER_FILE);
    let target = file.to_string_lossy().to_string();
    let _ = fs::remove_file(&file);
    let trusted = address.starts_with(AMD_INSTALLERS).then_some(address).ok_or_else(|| InstallerError::Download(address.to_string()))?;
    curl(&["--silent", "--show-error", "--fail", "--max-time", "900", "--user-agent", AGENT, "--referer", AMD_REFERER, "--output", &target, trusted])
        .map_err(InstallerError::Download)?;
    match signed_by(&file, &AMD_PUBLISHERS) {
        true => Ok(file),
        false => {
            let _ = fs::remove_file(&file);
            Err(InstallerError::NotSigned)
        }
    }
}

/// Downloads AMD's web installer, checks its signature and starts it. The installer asks for administrator rights by itself.
pub fn run_amd_installer(address: &str, folder: &Path) -> Result<(), InstallerError> {
    let file = download_amd_installer(address, folder)?;
    match system::shell_open(&file.to_string_lossy()) {
        true => Ok(()),
        false => Err(InstallerError::NotStarted),
    }
}

/// What the Drivers page shows after checking: each graphics adapter with what can be done, and the Windows Update offers.
pub struct Check {
    pub gpus: Vec<(Gpu, GpuUpdate)>,
    pub updates: Result<Vec<Update>, String>,
}

pub fn check(progress: &dyn Fn(Step)) -> Check {
    progress(Step::Graphics);
    let gpus = gpus()
        .into_iter()
        .map(|gpu| {
            let update = gpu_update(&gpu);
            (gpu, update)
        })
        .collect();
    progress(Step::WindowsUpdate);
    Check { gpus, updates: search_updates() }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Graphics,
    WindowsUpdate,
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTING: &str = r#"<?xml version="1.0" encoding="utf-8"?>
        <PnpUtil Version="10.0.26200" Command="/enum-drivers /format xml">
            <Driver DriverName="oem22.inf">
                <OriginalName>acpivpc.inf</OriginalName>
                <ProviderName>Lenovo</ProviderName>
                <ClassName>System</ClassName>
                <ClassGuid>{4d36e97d-e325-11ce-bfc1-08002be10318}</ClassGuid>
                <DriverVersion>05/19/2026 15.11.30.14</DriverVersion>
                <SignerName>Microsoft Windows Hardware Compatibility Publisher</SignerName>
            </Driver>
            <Driver DriverName="oem7.inf">
                <OriginalName>acpivpc.inf</OriginalName>
                <ProviderName>Lenovo</ProviderName>
                <ClassGuid>{4d36e97d-e325-11ce-bfc1-08002be10318}</ClassGuid>
                <DriverVersion>08/14/2025 15.11.30.11</DriverVersion>
            </Driver>
            <Driver DriverName="oem16.inf">
                <OriginalName>smbusamd.inf</OriginalName>
                <ProviderName>Advanced Micro Devices, Inc</ProviderName>
                <ClassGuid>{4d36e97d-e325-11ce-bfc1-08002be10318}</ClassGuid>
                <DriverVersion>03/26/2024 5.12.0.44</DriverVersion>
            </Driver>
            <Driver DriverName="oem77.inf">
                <OriginalName>smbusamd.inf</OriginalName>
                <ProviderName>Advanced Micro Devices, Inc</ProviderName>
                <ClassGuid>{4d36e97d-e325-11ce-bfc1-08002be10318}</ClassGuid>
                <DriverVersion>09/09/2025 5.12.0.44</DriverVersion>
            </Driver>
            <Driver DriverName="oem40.inf">
                <OriginalName>xvdd.inf</OriginalName>
                <ProviderName>Xbox</ProviderName>
                <ClassGuid>{4d36e97b-e325-11ce-bfc1-08002be10318}</ClassGuid>
                <DriverVersion>10/11/2026 10.0.22030.1</DriverVersion>
            </Driver>
        </PnpUtil>"#;

    fn names(packages: &[Package]) -> Vec<&str> {
        packages.iter().map(|p| p.inf.as_str()).collect()
    }

    #[test]
    fn reads_the_driver_store_listing() {
        let packages = parse_packages(LISTING);
        assert_eq!(names(&packages), ["oem22.inf", "oem7.inf", "oem16.inf", "oem77.inf", "oem40.inf"]);
        assert_eq!(packages[0].original_name, "acpivpc.inf");
        assert_eq!(packages[0].provider, "Lenovo");
        assert_eq!((packages[0].date, packages[0].version.as_slice()), ((2026, 5, 19), [15, 11, 30, 14].as_slice()));
    }

    #[test]
    fn only_unused_packages_with_a_newer_version_are_superseded() {
        let packages = parse_packages(LISTING);
        let in_use: HashSet<String> = ["oem22.inf", "oem77.inf", "oem40.inf"].iter().map(|inf| inf.to_string()).collect();
        assert_eq!(names(&superseded(&packages, &in_use)), ["oem7.inf", "oem16.inf"]);
        let old_one_in_use: HashSet<String> = ["oem7.inf".to_string()].into_iter().collect();
        assert_eq!(names(&superseded(&packages, &old_one_in_use)), ["oem16.inf"], "a package in use stays, and so does the newest of each driver");
        assert!(superseded(&packages[4..], &HashSet::new()).is_empty(), "a driver with a single version is never superseded");
    }

    #[test]
    fn only_published_infs_can_be_deleted() {
        assert!(is_published_inf("oem7.inf"));
        assert!(is_published_inf("oem122.inf"));
        assert!(!is_published_inf("oem.inf"));
        assert!(!is_published_inf("usbstor.inf"));
        assert!(!is_published_inf("oem7.inf /force"));
    }

    #[test]
    fn finds_the_amd_installer_in_its_page() {
        let page = r#"<a href="https://drivers.amd.com/relnotes/command-line-installation.pdf">notes</a>
            <a href="https://drivers.amd.com/drivers/installer/26.20/beta/amd-software-adrenalin-edition-26.10.2-minimalsetup-261029_web.exe">Download</a>"#;
        let (version, address) = amd_installer_link(page).unwrap();
        assert_eq!(version, "26.10.2");
        assert_eq!(address, "https://drivers.amd.com/drivers/installer/26.20/beta/amd-software-adrenalin-edition-26.10.2-minimalsetup-261029_web.exe");
        assert_eq!(amd_installer_link("<a href=\"https://drivers.amd.com/drivers/installer/other.zip\">x</a>"), None);
    }

    #[test]
    fn compares_versions_by_their_numbers() {
        assert!(numbers("26.10.2") > numbers("26.10.1"));
        assert!(numbers("26.10.1") > numbers("26.9.3"));
        assert_eq!(numbers("26.10.1"), numbers("26.10.1"));
    }

    #[test]
    fn validates_update_ids_and_dates() {
        assert!(is_update_id("de4669dc-1154-47d6-90f2-1acb6ec142a5"));
        assert!(!is_update_id("de4669dc-1154-47d6-90f2-1acb6ec142a5'; Remove-Item"));
        assert!(!is_update_id("short"));
        assert_eq!(iso_date("10-16-2026"), "2026-10-16");
        assert_eq!(iso_date("3-5-2026"), "2026-03-05");
        assert_eq!(quoted("O'Brien"), "'O''Brien'");
    }

    #[test]
    #[ignore]
    fn print_drivers() {
        for gpu in gpus() {
            println!("gpu: {gpu:?} => {:?}", gpu_update(&gpu));
        }
        for package in superseded_packages() {
            println!("superseded: {} {} {:?} {:?} at {:?}", package.inf, package.original_name, package.date, package.version, package_folder(&package));
        }
    }

    #[test]
    #[ignore]
    fn downloads_the_amd_installer_and_checks_its_signature() {
        let (version, address) = latest_amd_software().unwrap();
        let folder = std::env::temp_dir();
        let file = download_amd_installer(&address, &folder).unwrap();
        println!("AMD Software {version}: {} bytes at {}", fs::metadata(&file).unwrap().len(), file.display());
        assert!(!signed_by(&file, &["Someone Else"]));
        fs::remove_file(&file).unwrap();
        assert!(matches!(download_amd_installer("https://example.com/setup.exe", &folder), Err(InstallerError::Download(_))));
    }

    #[test]
    #[ignore]
    fn print_windows_update_drivers() {
        println!("{:#?}", search_updates());
    }
}
