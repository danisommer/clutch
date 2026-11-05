//! Windows access: processes, windows, power, displays, registry, services and elevation.

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use windows_sys::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes,
    QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_CANCELLED, GetLastError, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM,
    SYSTEMTIME,
};
use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
use windows_sys::Win32::Graphics::Gdi::{
    DEVMODEW, DISPLAY_DEVICEW, ENUM_CURRENT_SETTINGS, EnumDisplayDevicesW, EnumDisplaySettingsW,
};
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows_sys::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;
use windows_sys::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
    TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Services::{
    CloseServiceHandle, ControlService, OpenSCManagerW, OpenServiceW, QueryServiceStatus,
    SC_MANAGER_CONNECT, SERVICE_CONTROL_STOP, SERVICE_QUERY_STATUS, SERVICE_RUNNING,
    SERVICE_START, SERVICE_STATUS, SERVICE_STOP, SERVICE_STOPPED, StartServiceW,
};
use windows_sys::Win32::System::SystemInformation::{
    GetLocalTime, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcessId, GetExitCodeProcess, INFINITE, OpenProcess,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
};
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST,
    OPENFILENAMEW,
};
use windows_sys::Win32::UI::Shell::{
    FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING,
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures,
    FOLDERID_Videos, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI,
    SHERB_NOSOUND, SHEmptyRecycleBinW, SHFILEOPSTRUCTW, SHFileOperationW, SHGetKnownFolderPath, SHQUERYRBINFO,
    SHQueryRecycleBinW, ShellExecuteExW, ShellExecuteW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GetWindow, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible, PostMessageW, SW_HIDE, SW_SHOWNORMAL, WM_CLOSE,
};
use windows_sys::core::GUID;
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const NO_SYSTEM_BATTERY: u8 = 128;
const DISPLAY_DEVICE_PRIMARY: u32 = 0x4;

/// Null-terminated UTF-16 string for Win32 calls.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

#[derive(Clone, Debug)]
pub struct Process {
    pub pid: u32,
    pub parent: u32,
    pub name: String,
    pub session: u32,
}

/// Running processes, with names stripped of the `.exe` extension.
pub fn processes() -> Vec<Process> {
    let mut list = Vec::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return list;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snapshot, &mut entry);
        while ok != 0 {
            let exe = from_wide(&entry.szExeFile);
            let name = exe.strip_suffix(".exe").or_else(|| exe.strip_suffix(".EXE")).unwrap_or(&exe).to_string();
            let mut session = u32::MAX;
            ProcessIdToSessionId(entry.th32ProcessID, &mut session);
            list.push(Process { pid: entry.th32ProcessID, parent: entry.th32ParentProcessID, name, session });
            ok = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
    }
    list
}

pub fn current_session() -> u32 {
    let mut session = 0;
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) };
    session
}

/// Processes in the current session whose name is in `names`, skipping the pids in `exclude`.
pub fn processes_named(all: &[Process], names: &[String], exclude: &HashSet<u32>) -> Vec<Process> {
    let session = current_session();
    all.iter()
        .filter(|p| p.session == session && !exclude.contains(&p.pid) && names.iter().any(|n| n.eq_ignore_ascii_case(&p.name)))
        .cloned()
        .collect()
}

/// This process and all of its ancestors, so the app never closes whatever launched it.
pub fn self_and_ancestors(all: &[Process]) -> HashSet<u32> {
    let parents: HashMap<u32, u32> = all.iter().map(|p| (p.pid, p.parent)).collect();
    let mut set = HashSet::new();
    let mut current = unsafe { GetCurrentProcessId() };
    while set.insert(current) {
        match parents.get(&current) {
            Some(&parent) if parent != 0 => current = parent,
            _ => break,
        }
    }
    set
}

pub fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buffer = vec![0u16; 1024];
        let mut length = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut length);
        CloseHandle(h);
        (ok != 0).then(|| String::from_utf16_lossy(&buffer[..length as usize]))
    }
}

/// Microsoft Store app identifier of the process, when it is a packaged app.
pub fn process_aumid(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let mut buffer = vec![0u16; 512];
        let mut length = buffer.len() as u32;
        let result = GetApplicationUserModelId(h, &mut length, buffer.as_mut_ptr());
        CloseHandle(h);
        (result == 0).then(|| from_wide(&buffer))
    }
}

pub fn terminate_process(pid: u32) -> bool {
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if h.is_null() {
            return false;
        }
        let ok = TerminateProcess(h, 1) != 0;
        if ok {
            WaitForSingleObject(h, 3000);
        }
        CloseHandle(h);
        ok
    }
}

struct WindowSearch {
    pids: HashSet<u32>,
    windows: Vec<(HWND, u32)>,
}

unsafe extern "system" fn collect_window(hwnd: HWND, data: LPARAM) -> i32 {
    let search = unsafe { &mut *(data as *mut WindowSearch) };
    let mut owner = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, &mut owner) };
    let top_level = unsafe { IsWindowVisible(hwnd) != 0 && GetWindow(hwnd, GW_OWNER).is_null() };
    if top_level && (search.pids.is_empty() || search.pids.contains(&owner)) {
        search.windows.push((hwnd, owner));
    }
    1
}

fn top_level_windows(pids: HashSet<u32>) -> Vec<(HWND, u32)> {
    let mut search = WindowSearch { pids, windows: Vec::new() };
    unsafe { EnumWindows(Some(collect_window), &mut search as *mut WindowSearch as LPARAM) };
    search.windows
}

pub fn window_count(pids: &[u32]) -> usize {
    if pids.is_empty() {
        return 0;
    }
    top_level_windows(pids.iter().copied().collect()).len()
}

/// Asks every top-level window of the processes to close, like clicking its X.
pub fn request_close(pids: &[u32]) -> usize {
    if pids.is_empty() {
        return 0;
    }
    let windows = top_level_windows(pids.iter().copied().collect());
    for (hwnd, _) in &windows {
        unsafe { PostMessageW(*hwnd, WM_CLOSE, 0, 0) };
    }
    windows.len()
}

/// Visible top-level windows, with their pid and title.
pub fn open_windows() -> Vec<(u32, String)> {
    top_level_windows(HashSet::new())
        .into_iter()
        .filter_map(|(hwnd, pid)| unsafe {
            let length = GetWindowTextLengthW(hwnd);
            if length <= 0 {
                return None;
            }
            let mut buffer = vec![0u16; length as usize + 1];
            GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
            Some((pid, from_wide(&buffer)))
        })
        .collect()
}

pub const BEST_PERFORMANCE: GUID = GUID::from_u128(0xded574b5_45a0_4f42_8737_46345c09c238);
pub const BALANCED: GUID = GUID::from_u128(0);
pub const BEST_EFFICIENCY: GUID = GUID::from_u128(0x961cc777_2547_4f9d_8174_7d86181b8a7a);

type GetOverlayFn = unsafe extern "system" fn(*mut GUID) -> u32;
type SetOverlayFn = unsafe extern "system" fn(GUID) -> u32;

fn powrprof_function(name: &[u8]) -> Option<unsafe extern "system" fn() -> isize> {
    unsafe {
        let module = LoadLibraryW(wide("powrprof.dll").as_ptr());
        if module.is_null() {
            return None;
        }
        GetProcAddress(module, name.as_ptr())
    }
}

fn read_overlay(name: &[u8]) -> Option<GUID> {
    let f = powrprof_function(name)?;
    let f: GetOverlayFn = unsafe { std::mem::transmute(f) };
    let mut guid = GUID::from_u128(0);
    (unsafe { f(&mut guid) } == 0).then_some(guid)
}

/// Power mode chosen by the user (Balanced, Best performance...).
pub fn chosen_power_mode() -> Option<GUID> {
    read_overlay(b"PowerGetActualOverlayScheme\0")
}

/// Power mode Windows is actually applying, after Energy Saver is taken into account.
pub fn effective_power_mode() -> Option<GUID> {
    read_overlay(b"PowerGetEffectiveOverlayScheme\0")
}

pub fn set_power_mode(mode: GUID) -> bool {
    match powrprof_function(b"PowerSetActiveOverlayScheme\0") {
        Some(f) => {
            let f: SetOverlayFn = unsafe { std::mem::transmute(f) };
            unsafe { f(mode) == 0 }
        }
        None => false,
    }
}

pub fn guid_eq(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

pub fn guid_to_string(g: &GUID) -> String {
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        g.data1, g.data2, g.data3, g.data4[0], g.data4[1], g.data4[2], g.data4[3], g.data4[4], g.data4[5], g.data4[6], g.data4[7]
    )
}

pub fn guid_from_str(text: &str) -> Option<GUID> {
    u128::from_str_radix(&text.replace('-', ""), 16).ok().map(GUID::from_u128)
}

/// The power modes Windows Settings offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerMode {
    BestPerformance,
    Balanced,
    BestEfficiency,
    Unknown,
}

pub fn power_mode(g: &GUID) -> PowerMode {
    match () {
        _ if guid_eq(g, &BEST_PERFORMANCE) => PowerMode::BestPerformance,
        _ if guid_eq(g, &BALANCED) => PowerMode::Balanced,
        _ if guid_eq(g, &BEST_EFFICIENCY) => PowerMode::BestEfficiency,
        _ => PowerMode::Unknown,
    }
}

/// Language of the Windows interface, as a LANGID.
pub fn user_ui_language() -> u16 {
    unsafe { GetUserDefaultUILanguage() }
}

/// `Some(true)` on AC power, `Some(false)` on battery, `None` on computers without a battery.
pub fn on_ac_power() -> Option<bool> {
    let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    if unsafe { GetSystemPowerStatus(&mut status) } == 0 || status.BatteryFlag == NO_SYSTEM_BATTERY {
        return None;
    }
    Some(status.ACLineStatus == 1)
}

pub fn available_memory() -> u64 {
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    unsafe { GlobalMemoryStatusEx(&mut status) };
    status.ullAvailPhys
}

pub fn free_space(drive: &str) -> Option<u64> {
    let mut free = 0u64;
    let ok = unsafe { GetDiskFreeSpaceExW(wide(drive).as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    (ok != 0).then_some(free)
}

pub fn local_time() -> SYSTEMTIME {
    let mut t: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    t
}

#[derive(Clone, Debug)]
pub struct Screen {
    pub name: String,
    pub internal: bool,
    pub primary: bool,
    pub width: u32,
    pub height: u32,
    pub hz: u32,
    pub max_hz: u32,
}

fn screen_modes(gdi_name: &str) -> Option<(u32, u32, u32, u32)> {
    let name = wide(gdi_name);
    let mut current: DEVMODEW = unsafe { std::mem::zeroed() };
    current.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
    if unsafe { EnumDisplaySettingsW(name.as_ptr(), ENUM_CURRENT_SETTINGS, &mut current) } == 0 {
        return None;
    }
    let mut max_hz = current.dmDisplayFrequency;
    let mut index = 0u32;
    loop {
        let mut mode: DEVMODEW = unsafe { std::mem::zeroed() };
        mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        if unsafe { EnumDisplaySettingsW(name.as_ptr(), index, &mut mode) } == 0 {
            break;
        }
        if mode.dmPelsWidth == current.dmPelsWidth && mode.dmPelsHeight == current.dmPelsHeight {
            max_hz = max_hz.max(mode.dmDisplayFrequency);
        }
        index += 1;
    }
    Some((current.dmPelsWidth, current.dmPelsHeight, current.dmDisplayFrequency, max_hz))
}

fn primary_screens() -> HashSet<String> {
    let mut primary = HashSet::new();
    let mut index = 0u32;
    loop {
        let mut device: DISPLAY_DEVICEW = unsafe { std::mem::zeroed() };
        device.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
        if unsafe { EnumDisplayDevicesW(std::ptr::null(), index, &mut device, 0) } == 0 {
            break;
        }
        if device.StateFlags & DISPLAY_DEVICE_PRIMARY != 0 {
            primary.insert(from_wide(&device.DeviceName));
        }
        index += 1;
    }
    primary
}

/// Active screens with resolution, current refresh rate and the highest rate at that resolution.
pub fn screens() -> Vec<Screen> {
    let mut path_count = 0u32;
    let mut mode_count = 0u32;
    if unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count) } != 0 {
        return Vec::new();
    }
    let mut paths: Vec<DISPLAYCONFIG_PATH_INFO> = vec![unsafe { std::mem::zeroed() }; path_count as usize];
    let mut modes: Vec<DISPLAYCONFIG_MODE_INFO> = vec![unsafe { std::mem::zeroed() }; mode_count as usize];
    let status = unsafe {
        QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut path_count, paths.as_mut_ptr(), &mut mode_count, modes.as_mut_ptr(), std::ptr::null_mut())
    };
    if status != 0 {
        return Vec::new();
    }
    let primary = primary_screens();
    paths.truncate(path_count as usize);
    paths
        .iter()
        .filter_map(|path| {
            let mut target: DISPLAYCONFIG_TARGET_DEVICE_NAME = unsafe { std::mem::zeroed() };
            target.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            target.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            target.header.adapterId = path.targetInfo.adapterId;
            target.header.id = path.targetInfo.id;
            unsafe { DisplayConfigGetDeviceInfo(&mut target.header) };

            let mut source: DISPLAYCONFIG_SOURCE_DEVICE_NAME = unsafe { std::mem::zeroed() };
            source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            source.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            source.header.adapterId = path.sourceInfo.adapterId;
            source.header.id = path.sourceInfo.id;
            unsafe { DisplayConfigGetDeviceInfo(&mut source.header) };

            let gdi_name = from_wide(&source.viewGdiDeviceName);
            let (width, height, hz, max_hz) = screen_modes(&gdi_name)?;
            let technology = path.targetInfo.outputTechnology as u32;
            Some(Screen {
                name: from_wide(&target.monitorFriendlyDeviceName),
                internal: matches!(technology, 11 | 13 | 0x8000_0000),
                primary: primary.contains(&gdi_name),
                width,
                height,
                hz,
                max_hz,
            })
        })
        .collect()
}

fn registry_root(local_machine: bool) -> RegKey {
    RegKey::predef(if local_machine { HKEY_LOCAL_MACHINE } else { HKEY_CURRENT_USER })
}

pub fn registry_u32(local_machine: bool, path: &str, value: &str) -> Option<u32> {
    registry_root(local_machine).open_subkey_with_flags(path, KEY_READ).ok()?.get_value::<u32, _>(value).ok()
}

pub fn registry_string(local_machine: bool, path: &str, value: &str) -> Option<String> {
    registry_root(local_machine).open_subkey_with_flags(path, KEY_READ).ok()?.get_value::<String, _>(value).ok()
}

pub fn registry_key_exists(local_machine: bool, path: &str) -> bool {
    registry_root(local_machine).open_subkey_with_flags(path, KEY_READ).is_ok()
}

/// String values of every subkey of a HKLM key, in the 32-bit view, as (subkey name, lowercase value name → data).
pub fn registry_subkeys_32(path: &str) -> Vec<(String, HashMap<String, String>)> {
    let flags = KEY_READ | KEY_WOW64_32KEY;
    let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(path, flags) else { return Vec::new() };
    key.enum_keys()
        .flatten()
        .filter_map(|name| {
            let sub = key.open_subkey_with_flags(&name, flags).ok()?;
            let values = sub
                .enum_values()
                .flatten()
                .filter_map(|(value, _)| sub.get_value::<String, _>(&value).ok().map(|data| (value.to_lowercase(), data)))
                .collect();
            Some((name, values))
        })
        .collect()
}

/// Display names of installed programs, as listed in Settings > Apps.
pub fn installed_programs() -> Vec<String> {
    let mut names = Vec::new();
    let path = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    let sources = [
        (HKEY_LOCAL_MACHINE, KEY_READ | KEY_WOW64_64KEY),
        (HKEY_LOCAL_MACHINE, KEY_READ | KEY_WOW64_32KEY),
        (HKEY_CURRENT_USER, KEY_READ),
    ];
    for (root, flags) in sources {
        if let Ok(key) = RegKey::predef(root).open_subkey_with_flags(path, flags) {
            for sub in key.enum_keys().flatten() {
                if let Ok(name) = key.open_subkey_with_flags(&sub, flags).and_then(|s| s.get_value::<String, _>("DisplayName")) {
                    names.push(name);
                }
            }
        }
    }
    names
}

pub fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let mut path: *mut u16 = std::ptr::null_mut();
        if SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut path) != 0 || path.is_null() {
            return None;
        }
        let mut length = 0;
        while *path.add(length) != 0 {
            length += 1;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(path, length));
        CoTaskMemFree(path as *const c_void);
        Some(PathBuf::from(text))
    }
}

/// Personal folders the cleanup must never touch.
pub fn personal_folders() -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = [FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Pictures, FOLDERID_Videos, FOLDERID_Music]
        .iter()
        .filter_map(known_folder)
        .collect();
    if let Ok(onedrive) = std::env::var("OneDrive") {
        folders.push(PathBuf::from(onedrive));
    }
    folders
}

pub fn run_hidden(program: &str, args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new(program).args(args).creation_flags(CREATE_NO_WINDOW).output()
}

pub fn shutdown_wsl() -> bool {
    run_hidden("wsl.exe", &["--shutdown"]).map(|o| o.status.success()).unwrap_or(false)
}

pub fn wsl_running() -> bool {
    processes().iter().any(|p| p.name.eq_ignore_ascii_case("vmmemWSL"))
}

/// Names of the programs currently using WSL.
pub fn wsl_clients(exclude: &HashSet<u32>) -> Vec<String> {
    let all = processes();
    let by_pid: HashMap<u32, &Process> = all.iter().map(|p| (p.pid, p)).collect();
    let mut names: Vec<String> = all
        .iter()
        .filter(|p| p.name.eq_ignore_ascii_case("wsl") && !exclude.contains(&p.pid))
        .filter_map(|p| by_pid.get(&p.parent).map(|parent| parent.name.clone()))
        .filter(|n| !n.eq_ignore_ascii_case("wsl"))
        .collect();
    names.sort();
    names.dedup();
    names
}

#[derive(Clone, Copy)]
enum DialogKind {
    Open,
    Save,
}

/// Shows the standard Windows file dialog and waits for the answer. Call it from a background thread.
fn file_dialog(kind: DialogKind, title: &str, filter_name: &str, pattern: &str, default_name: &str, extension: &str) -> Option<PathBuf> {
    let initialized = unsafe { CoInitializeEx(std::ptr::null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32) } >= 0;
    let mut buffer: Vec<u16> = default_name.encode_utf16().collect();
    buffer.resize(1024, 0);
    let filter: Vec<u16> = format!("{filter_name}\0{pattern}\0\0").encode_utf16().collect();
    let title = wide(title);
    let extension = wide(extension);
    let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = buffer.as_mut_ptr();
    dialog.nMaxFile = buffer.len() as u32;
    dialog.lpstrTitle = title.as_ptr();
    let result = match kind {
        DialogKind::Open => {
            dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
            unsafe { GetOpenFileNameW(&mut dialog) }
        }
        DialogKind::Save => {
            dialog.Flags = OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
            dialog.lpstrDefExt = extension.as_ptr();
            unsafe { GetSaveFileNameW(&mut dialog) }
        }
    };
    if initialized {
        unsafe { CoUninitialize() };
    }
    (result != 0).then(|| PathBuf::from(from_wide(&buffer)))
}

/// Asks the user for an existing file matching `pattern`, such as "*.exe".
pub fn pick_file(title: &str, filter_name: &str, pattern: &str) -> Option<PathBuf> {
    file_dialog(DialogKind::Open, title, filter_name, pattern, "", "")
}

/// Asks the user where to save a file, suggesting `default_name`.
pub fn choose_save_path(title: &str, filter_name: &str, pattern: &str, default_name: &str, extension: &str) -> Option<PathBuf> {
    file_dialog(DialogKind::Save, title, filter_name, pattern, default_name, extension)
}

/// Opens a text file in Notepad, which every Windows has, so it can be edited whatever the file associations are.
pub fn open_in_notepad(path: &Path) -> bool {
    Command::new("notepad.exe").arg(path).spawn().is_ok()
}

/// Opens a file, program, URL or Store app through the Windows shell.
pub fn shell_open(target: &str) -> bool {
    let result = unsafe {
        ShellExecuteW(std::ptr::null_mut(), wide("open").as_ptr(), wide(target).as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL)
    };
    result as isize > 32
}

/// Starts a program through the Windows shell with `dir` as its working folder, as games expect.
pub fn shell_open_in(target: &str, dir: &Path) -> bool {
    let dir = wide(&dir.to_string_lossy());
    let result = unsafe {
        ShellExecuteW(std::ptr::null_mut(), wide("open").as_ptr(), wide(target).as_ptr(), std::ptr::null(), dir.as_ptr(), SW_SHOWNORMAL)
    };
    result as isize > 32
}

/// Why an item did not reach the Recycle Bin.
#[derive(Clone, Copy, Debug)]
pub enum RecycleError {
    Declined,
    Failed(i32),
}

/// Moves a file or folder to the Recycle Bin. When it cannot be recycled, Windows asks before deleting it for good.
pub fn send_to_recycle_bin(path: &Path) -> Result<(), RecycleError> {
    let from: Vec<u16> = path.as_os_str().encode_wide().chain([0, 0]).collect();
    let flags = FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT | FOF_WANTNUKEWARNING;
    let mut operation: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    operation.wFunc = FO_DELETE;
    operation.pFrom = from.as_ptr();
    operation.fFlags = flags as u16;
    match (unsafe { SHFileOperationW(&mut operation) }, operation.fAnyOperationsAborted) {
        (0, 0) => Ok(()),
        (0, _) => Err(RecycleError::Declined),
        (code, _) => Err(RecycleError::Failed(code)),
    }
}

/// Size of the current user's Recycle Bin on every drive.
pub fn recycle_bin_size() -> u64 {
    let mut info: SHQUERYRBINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHQUERYRBINFO>() as u32;
    match unsafe { SHQueryRecycleBinW(std::ptr::null(), &mut info) } {
        0 => info.i64Size.max(0) as u64,
        _ => 0,
    }
}

/// Empties the current user's Recycle Bin on every drive, without asking; returns the HRESULT on failure.
pub fn empty_recycle_bin() -> Result<(), i32> {
    let flags = SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND;
    match unsafe { SHEmptyRecycleBinW(std::ptr::null_mut(), std::ptr::null(), flags) } {
        0 => Ok(()),
        code => Err(code),
    }
}

#[derive(Debug)]
pub enum ElevationError {
    Cancelled,
    Failed(u32),
}

/// Runs this same executable as administrator, waits for it and returns its exit code.
pub fn run_elevated(params: &str) -> Result<u32, ElevationError> {
    let exe = std::env::current_exe().map_err(|_| ElevationError::Failed(0))?;
    let exe_w = wide(&exe.to_string_lossy());
    let verb = wide("runas");
    let params_w = wide(params);
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = exe_w.as_ptr();
        info.lpParameters = params_w.as_ptr();
        info.nShow = SW_HIDE;
        if ShellExecuteExW(&mut info) == 0 {
            let error = GetLastError();
            return Err(if error == ERROR_CANCELLED { ElevationError::Cancelled } else { ElevationError::Failed(error) });
        }
        let process: HANDLE = info.hProcess;
        WaitForSingleObject(process, INFINITE);
        let mut code = 0u32;
        GetExitCodeProcess(process, &mut code);
        CloseHandle(process);
        Ok(code)
    }
}

fn wait_for_state(service: *mut c_void, wanted: u32, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        let mut status: SERVICE_STATUS = unsafe { std::mem::zeroed() };
        if unsafe { QueryServiceStatus(service, &mut status) } == 0 {
            return false;
        }
        if status.dwCurrentState == wanted {
            return true;
        }
        if start.elapsed() > timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// Why a service could not be controlled, with the Win32 error code.
#[derive(Clone, Copy, Debug)]
pub enum ServiceError {
    NoManager(u32),
    Unavailable(u32),
}

/// Stops a Windows service; returns whether it was running before.
pub fn stop_service(name: &str) -> Result<bool, ServiceError> {
    unsafe {
        let manager = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
        if manager.is_null() {
            return Err(ServiceError::NoManager(GetLastError()));
        }
        let service = OpenServiceW(manager, wide(name).as_ptr(), SERVICE_STOP | SERVICE_START | SERVICE_QUERY_STATUS);
        if service.is_null() {
            let error = GetLastError();
            CloseServiceHandle(manager);
            return Err(ServiceError::Unavailable(error));
        }
        let mut status: SERVICE_STATUS = std::mem::zeroed();
        QueryServiceStatus(service, &mut status);
        let was_running = status.dwCurrentState == SERVICE_RUNNING;
        if was_running {
            ControlService(service, SERVICE_CONTROL_STOP, &mut status);
            wait_for_state(service, SERVICE_STOPPED, Duration::from_secs(30));
        }
        CloseServiceHandle(service);
        CloseServiceHandle(manager);
        Ok(was_running)
    }
}

pub fn start_service(name: &str) {
    unsafe {
        let manager = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
        if manager.is_null() {
            return;
        }
        let service = OpenServiceW(manager, wide(name).as_ptr(), SERVICE_START | SERVICE_QUERY_STATUS);
        if !service.is_null() {
            StartServiceW(service, 0, std::ptr::null());
            CloseServiceHandle(service);
        }
        CloseServiceHandle(manager);
    }
}

/// Case-insensitive check that `path` is `base` or lies inside it.
pub fn is_within(path: &Path, base: &Path) -> bool {
    let p = path.to_string_lossy().to_lowercase();
    let b = base.to_string_lossy().to_lowercase();
    let b = b.trim_end_matches('\\');
    p == b || p.starts_with(&format!("{b}\\"))
}
