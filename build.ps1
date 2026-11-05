<#
.SYNOPSIS
    Builds Modo Jogo e Limpeza and installs the executable in %LOCALAPPDATA%\Programs\ModoJogoLimpeza.
.DESCRIPTION
    Uses the Rust GNU toolchain (x86_64-pc-windows-gnu), which needs dlltool.exe and as.exe from
    MinGW-w64 binutils. They are taken from -Binutils when that folder exists, otherwise from PATH.
    The folder is added to PATH by its 8.3 short name, because dlltool cannot call as.exe from a
    path that contains spaces. When windres.exe from the same binutils is there too, the icon and the
    version info are embedded in the executable; without it the build only prints a warning.
.PARAMETER Debug
    Builds without optimizations and does not install.
.PARAMETER Shortcut
    Also creates a "Modo Jogo e Limpeza" shortcut on the desktop.
.PARAMETER Binutils
    Folder with dlltool.exe and as.exe. Defaults to %USERPROFILE%\.cargo\mingw-as.
#>
param(
    [switch]$Debug,
    [switch]$Shortcut,
    [string]$Binutils = (Join-Path $env:USERPROFILE '.cargo\mingw-as')
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

$manifest = Join-Path $PSScriptRoot 'Cargo.toml'
if ($Debug) {
    cargo build --manifest-path $manifest
    exit $LASTEXITCODE
}

cargo build --release --manifest-path $manifest
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$destination = Join-Path $env:LOCALAPPDATA 'Programs\ModoJogoLimpeza'
$exe = Join-Path $destination 'ModoJogoLimpeza.exe'
$icon = Join-Path $destination 'icon.ico'
New-Item -ItemType Directory -Force $destination | Out-Null
Copy-Item (Join-Path $PSScriptRoot 'target\release\modo-jogo-limpeza.exe') $exe -Force
Copy-Item (Join-Path $PSScriptRoot 'assets\icon.ico') $icon -Force
Write-Host "Installed in $destination"

if ($Shortcut) {
    $link = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Modo Jogo e Limpeza.lnk'
    $lnk = (New-Object -ComObject WScript.Shell).CreateShortcut($link)
    $lnk.TargetPath = $exe
    $lnk.WorkingDirectory = $destination
    $lnk.IconLocation = "$icon,0"
    $lnk.Description = 'Game Mode and storage cleanup'
    $lnk.Save()
    Write-Host "Shortcut created: $link"
}
