<#
.SYNOPSIS
    Builds Clutch and installs the executable in %LOCALAPPDATA%\Programs\Clutch.
.DESCRIPTION
    Uses the Rust GNU toolchain (x86_64-pc-windows-gnu), which needs dlltool.exe and as.exe from
    MinGW-w64 binutils. They are taken from -Binutils when that folder exists, otherwise from PATH.
    The folder is added to PATH by its 8.3 short name, because dlltool cannot call as.exe from a
    path that contains spaces. When windres.exe from the same binutils is there too, the icon and the
    version info are embedded in the executable; without it the build only prints a warning.
.PARAMETER Debug
    Builds without optimizations and does not install.
.PARAMETER Shortcut
    Also creates a "Clutch" shortcut on the desktop.
.PARAMETER Installer
    Builds the installer, target\release\Clutch-Setup.exe, instead of installing the executable.
    Needs Inno Setup 6.3 or later.
.PARAMETER Binutils
    Folder with dlltool.exe and as.exe. Defaults to %USERPROFILE%\.cargo\mingw-as.
.PARAMETER InnoSetup
    Folder with ISCC.exe. Defaults to %ProgramFiles(x86)%\Inno Setup 6, otherwise ISCC.exe is taken from PATH.
#>
param(
    [switch]$Debug,
    [switch]$Shortcut,
    [switch]$Installer,
    [string]$Binutils = (Join-Path $env:USERPROFILE '.cargo\mingw-as'),
    [string]$InnoSetup = (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6')
)

if (Test-Path (Join-Path $Binutils 'dlltool.exe')) {
    $short = (New-Object -ComObject Scripting.FileSystemObject).GetFolder($Binutils).ShortPath
    $env:Path = "$short;$env:Path"
}
foreach ($tool in 'dlltool.exe', 'as.exe') {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        throw "$tool not found. Install MinGW-w64 binutils or pass -Binutils <folder>."
    }
}
if ($Installer -and -not $Debug) {
    $iscc = Join-Path $InnoSetup 'ISCC.exe'
    if (-not (Test-Path $iscc)) { $iscc = (Get-Command iscc -ErrorAction SilentlyContinue).Source }
    if (-not $iscc) { throw 'ISCC.exe not found. Install Inno Setup or pass -InnoSetup <folder>.' }
}

$manifest = Join-Path $PSScriptRoot 'Cargo.toml'
if ($Debug) {
    cargo build --manifest-path $manifest
    exit $LASTEXITCODE
}

cargo build --release --manifest-path $manifest
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

if ($Installer) {
    $version = (Select-String -Path $manifest -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    & $iscc /Q "/DAppVersion=$version" (Join-Path $PSScriptRoot 'installer.iss')
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Write-Host "Installer built: $(Join-Path $PSScriptRoot 'target\release\Clutch-Setup.exe')"
    exit 0
}

$destination = Join-Path $env:LOCALAPPDATA 'Programs\Clutch'
$exe = Join-Path $destination 'Clutch.exe'
$icon = Join-Path $destination 'icon.ico'
New-Item -ItemType Directory -Force $destination | Out-Null
Copy-Item (Join-Path $PSScriptRoot 'target\release\clutch.exe') $exe -Force
Copy-Item (Join-Path $PSScriptRoot 'assets\icon.ico') $icon -Force
Write-Host "Installed in $destination"

if ($Shortcut) {
    $link = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Clutch.lnk'
    $lnk = (New-Object -ComObject WScript.Shell).CreateShortcut($link)
    $lnk.TargetPath = $exe
    $lnk.WorkingDirectory = $destination
    $lnk.IconLocation = "$icon,0"
    $lnk.Description = 'Game Mode, Eco Mode, storage cleanup and driver updates'
    $lnk.Save()
    Write-Host "Shortcut created: $link"
}
