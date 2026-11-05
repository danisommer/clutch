# Modo Jogo e Limpeza

**Game Mode and safe storage cleanup for Windows, in a single ~2 MB native app.**

*Modo Jogo e Limpeza* is Portuguese for "Game Mode & Cleanup". It prepares the computer for a gaming session and puts it back the way it was afterwards. It can also find and remove temporary files, caches and leftovers, deleting nothing until you confirm.

The interface is available in Brazilian Portuguese and English. By default it follows the Windows display language: Portuguese when Windows is in Portuguese, English otherwise.

## Download

Prebuilt executables are on the [Releases](https://github.com/danisommer/modo-jogo-limpeza/releases) page. The app is a single portable `ModoJogoLimpeza.exe`: download it, put it in any folder and run it. Each release also has:

- a `.sha256` file to check the download;
- a [Scoop](https://scoop.sh) manifest, which installs the app and adds a Start menu shortcut:

  ```powershell
  scoop install https://github.com/danisommer/modo-jogo-limpeza/releases/latest/download/modo-jogo-limpeza.json
  ```

The executable is not signed yet, so Windows SmartScreen may warn on the first run. Choose **More info**, then **Run anyway**.

## Features

### Game Mode

- **Status overview**: heavy programs that are open, WSL, charger, Windows power mode (including when Energy Saver is limiting it), available memory, refresh rate of each display, and the thermal mode on Lenovo laptops.
- **Safe program closing**: each program is first asked to close through its window, the same as clicking ✕, so it can save its work. It is terminated only if its policy allows:
  - `Never` (default for editors);
  - `WhenWindowless` (only when it is just running in the background);
  - `Always`.
- **WSL shutdown**: WSL is shut down unless some program is still using it.
- **Best performance**: switches the Windows power mode to *Best performance* while you play.
- **Game launch**: optionally starts a game installed with Steam, Epic Games or GOG, which are found automatically. Any other game (EA app, Battle.net, Ubisoft…) can be added by picking its executable; its folder is then the one the watcher follows.
- **Automatic restore**: the window closes itself to free memory, and a small background watcher (about 8 MB of RAM) follows the game. When the game exits, the watcher restores the previous power mode, reopens the programs that were closed and shows a summary.

### Cleanup

- **Analysis first**: the app analyzes before anything is touched and shows the size of each category. You check what to clean.
- **Categories covered**:
  - Windows temp files, Windows Update downloads, Delivery Optimization, crash dumps and the component store;
  - caches and logs of Chrome, Edge, Brave, Firefox, VS Code, Steam, Epic, Ubisoft, EA, Riot, Discord, Logitech G HUB and AMD driver installers;
  - npm, pip and Yarn package caches (not preselected);
  - leftovers of uninstalled programs.
- **Remembered selection**: what you check is remembered when you clean, and the next analysis starts the same way. Settings has a link to go back to the defaults.
- **Caution group**: categories that can cost you something are tagged and never preselected, not even after you cleaned them once. Examples are the Recycle Bin, AMD and DirectX shader caches, folders of uninstalled Steam games (they may hold replays or mods) and browser profiles of uninstalled browsers.
- **Elevation only when needed**: administrator rights are requested only for system items. That means one optional prompt to measure them and one prompt to clean them, and only when a selected category has any.
- **Running programs block their category**: if Chrome is open, its cache is skipped and you can close it from the app.

### History

A list of the cleanups done, with space freed, and of past Game Mode sessions.

### Settings

- **Theme**: light, dark, or following the Windows setting (the default).
- **Language**: Português (Brasil), English, or following Windows (the default).
- **Recycle Bin**: optionally, whole folders and files of the Leftovers and Caution groups go to the Recycle Bin instead of being deleted. Caches and temporary files are still deleted, since recycling them would not free any space. When an item cannot be recycled (too large, or on a drive without a Recycle Bin), Windows asks before deleting it for good.

## Safety model

Deleting files is the riskiest thing this app does, so the rules are enforced in code, not only in the catalog:

- **Allowed roots only**:
  - temp folders, `%LOCALAPPDATA%`, `%APPDATA%` and `%ProgramData%`;
  - specific Windows folders and Steam libraries.
- **Program Files** is reachable only for leftovers of programs that are **not installed**, checked against the Windows uninstall registry.
- **Protected folders are never touched**:
  - Desktop, Documents, Downloads, Pictures, Videos, Music and OneDrive;
  - `.ssh` and `.claude`;
  - WSL disks and Microsoft Store app data;
  - the app's own folders.
- **No following links**: junctions and symlinks are never followed.
- **Age limits**: temporary files must be older than 24 hours.
- **Files in use** are skipped and counted, never forced.
- **The elevated helper is the same executable** started with `--admin`:
  - it only accepts category ids that exist in the built-in catalog, and never reads [custom rules](#custom-rules);
  - it only reads and writes files inside the app's exchange folder.

## Requirements

- Windows 11, 64-bit. Windows 10 should work but has not been tested.
- To build: [Rust](https://rustup.rs) stable. See [Building](#building).

## Building

The project is developed with the GNU toolchain (`x86_64-pc-windows-gnu`). It needs `dlltool.exe` and `as.exe` from [MinGW-w64](https://www.mingw-w64.org/) binutils, for example from MSYS2.

```powershell
rustup default stable-x86_64-pc-windows-gnu
.\build.ps1             # release build, installed in %LOCALAPPDATA%\Programs\ModoJogoLimpeza
.\build.ps1 -Shortcut   # same, plus a desktop shortcut
.\build.ps1 -Debug      # debug build, not installed
```

`build.ps1` looks for the binutils in `%USERPROFILE%\.cargo\mingw-as`, or in the folder given with `-Binutils`, and falls back to `PATH`. A plain `cargo build --release` also works when both tools are on `PATH`. The MSVC toolchain has not been tested yet.

The icon and the version info shown in the file's Properties are embedded by [`build.rs`](build.rs). With the GNU toolchain this needs `windres.exe`, from the same binutils, in that folder or on `PATH` (or set `WINDRES` to its path). Without it the build still works and prints a warning.

### Releasing

1. Change `version` in `Cargo.toml`, build once so `Cargo.lock` picks it up, and commit both.
2. Push a tag with the same version, for example `git tag v0.2.0` and `git push origin v0.2.0`.

The [release workflow](.github/workflows/release.yml) runs the tests, builds with the GNU toolchain, requires the embedded resources, checks that the executable only depends on Windows DLLs and publishes the executable, its SHA-256 and the Scoop manifest.

### Tests

```powershell
cargo test                                      # unit tests, including the safety rules
cargo test print_ -- --ignored --nocapture      # read-only diagnostics of this machine
```

## Where things live

| What | Where |
| --- | --- |
| Program | `%LOCALAPPDATA%\Programs\ModoJogoLimpeza` |
| Settings, history, current session | `%LOCALAPPDATA%\ModoJogoLimpeza\*.json` |
| Custom cleanup rules | `%LOCALAPPDATA%\ModoJogoLimpeza\custom-categories.json` |
| Cleanup catalog | [`categories.json`](categories.json), embedded at build time |

### Catalog format

Each category has an `id`, a `name`, a `description` and a `group` (`system`, `programs`, `leftovers` or `caution`). The `name` and the `description` have one text per language, as in `{ "pt-BR": "Arquivos temporários", "en": "Temporary files" }`; a test fails when a built-in category misses one. A category can also have:

- `selectedByDefault`;
- `slow`;
- `blockedBy`, the processes that must be closed first;
- `targets`.

A target's `path` accepts environment variables, `%STEAM%` and `*` wildcards. Its `mode` is one of the following:

| Mode | Effect |
| --- | --- |
| `contents` | Deletes the files inside the folder (optionally only those older than `ageHours`, skipping `exclude`) and keeps the folder |
| `folder` / `file` | Removes the folder or file |
| `inactiveFolders` | Removes subfolders at `depth` with no writes in `ageHours` |
| `allButNewest` | Keeps only the newest entry (optionally matching `pattern`) |
| `staleVsCodeExtensions` | Removes extension folders that VS Code no longer references |
| `steamLeftovers` | Folders in `steamapps\common` without an installed game |
| `windowsUpdate` | Stops Windows Update, clears its download cache and starts it again |
| `deliveryOptimization` | Runs `Delete-DeliveryOptimizationCache` |
| `componentStore` | Runs `DISM /Online /Cleanup-Image /StartComponentCleanup` |
| `recycleBin` | Empties the user's Recycle Bin. It always runs before the other categories, so it never empties what the same cleanup just recycled |

Targets with `"admin": true` run in the elevated helper. Targets with `notInstalled` apply only while none of the listed programs is installed.

### Custom rules

Your own categories go in `%LOCALAPPDATA%\ModoJogoLimpeza\custom-categories.json`, in the same format as the catalog. **Settings → Open the rules file** creates it with an example and opens it in Notepad. The rules show up in the Cleanup page under **Your rules** at the next analysis.

```json
{
  "categories": [
    {
      "id": "my-editor-cache",
      "name": "My editor cache",
      "description": "Optional. A single text, or one per language as in the catalog.",
      "selectedByDefault": false,
      "targets": [
        { "path": "%LOCALAPPDATA%\\MyEditor\\Cache", "mode": "contents", "ageHours": 24 }
      ]
    }
  ]
}
```

Custom rules are held to stricter limits than the catalog, enforced in code:

- they go through the same allowed roots and protected folders;
- only the `contents`, `folder`, `file`, `inactiveFolders` and `allButNewest` modes are accepted;
- `"admin": true` is refused, and the elevated helper never reads this file, so nothing written there can run with administrator rights;
- an `id` that already exists is refused.

Rules that break these limits are skipped and listed in Settings, with the reason.

## Project layout

```
src/
  main.rs        entry point: window, --watcher and --admin modes
  app.rs         main window and pages
  theme.rs       light and dark palettes, fonts and widgets
  i18n.rs        every text shown to the user, in Portuguese and English
  game_mode.rs   status checks, closing programs, activation and restore
  watcher.rs     background process that ends the session when the game exits
  cleanup.rs     catalog, safety rules, analysis and removal
  admin.rs       elevated helper
  storage.rs     settings, history and session files
  steam.rs       Steam libraries and installed games
  games.rs       games from Steam, Epic Games, GOG and executables added by hand
  system.rs      Win32 wrappers (processes, windows, power, displays, registry, services)
categories.json  cleanup catalog
build.ps1        build and install script
build.rs         embeds the icon and version info in the executable
assets/icon.ico  executable and shortcut icon
.github/         release workflow
```

## Status

This is an early version. The safety rules have unit tests, but the elevated cleanup and the full Game Mode cycle still need testing on more machines. Use it at your own risk.

## Roadmap

Planned:

- [x] Downloadable releases (prebuilt executables on GitHub Releases)
- [x] Dark mode
- [x] Multiple languages, starting with English
- [ ] Support for more CPU and GPU vendors (NVIDIA and Intel shader caches, vendor tools)
- [ ] Support for more laptop vendors' performance modes (ASUS, MSI, Dell, HP, Acer…)
- [x] Deeper cleanup and optimization that stays safe and customizable
- [ ] More features in general

Ideas:

- [x] Option to send removed items to the Recycle Bin instead of deleting them
- [x] User-defined cleanup rules in a separate file, on top of the built-in catalog
- [ ] Per-game profiles (which programs to close, power settings)
- [x] Game launch from other launchers: Epic Games and GOG found automatically, any other game through its executable
- [ ] Automatic detection of EA app, Battle.net and Xbox app games
- [ ] Exportable cleanup report
- [x] Executable icon and version info embedded as Windows resources
- [x] CI with tests on every release
- [ ] Signed builds
- [x] Distribution through Scoop (manifest in each release)
- [ ] Distribution through winget
- [ ] Keyboard navigation and screen reader support

## License

No license has been chosen yet. Until one is added, all rights are reserved.
