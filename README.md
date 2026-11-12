# Clutch

**Game Mode and safe storage cleanup for Windows, in a single ~2 MB native app.**

Clutch prepares the computer for a gaming session and puts it back the way it was afterwards. It can also find and remove temporary files, caches and leftovers, deleting nothing until you confirm.

The app used to be called *Modo Jogo e Limpeza*. Its settings and history move to the new folder the first time Clutch runs.

The interface is available in Brazilian Portuguese and English. By default it follows the Windows display language: Portuguese when Windows is in Portuguese, English otherwise.

## Download

Prebuilt executables are on the [Releases](https://github.com/danisommer/modo-jogo-limpeza/releases) page. The app is a single portable `Clutch.exe`: download it, put it in any folder and run it. Each release also has:

- a `.sha256` file to check the download;
- a [Scoop](https://scoop.sh) manifest, which installs the app and adds a Start menu shortcut:

  ```powershell
  scoop install https://github.com/danisommer/modo-jogo-limpeza/releases/latest/download/clutch.json
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
- **Game launch**: optionally starts a game installed with Steam, Epic Games, GOG, the EA app, Battle.net or the Xbox app, which are found automatically. Any other game (Ubisoft, Riot…) can be added by picking its executable; its folder is then the one the watcher follows.
- **Per-game settings**: a chosen game can keep its own list of programs to close, WSL and power choices. The profile starts as a copy of the general settings and can be turned off to use them again.
- **Automatic restore**: the window closes itself to free memory, and a small background watcher (about 8 MB of RAM) follows the game. When the game exits, the watcher restores the previous power mode, reopens the programs that were closed and shows a summary.

### Eco Mode

The opposite of Game Mode, for when the battery has to last. Everything it changes is put back when it ends.

- **Status overview**: charger, Windows power mode, Energy Saver, refresh rate and brightness of the laptop screen, WSL, and the thermal mode on Lenovo laptops.
- **Lower power use**, each part optional:
  - switches the Windows power mode to *Best power efficiency*;
  - makes Windows Energy Saver come on at any battery level (Windows only turns it on while on battery);
  - lowers the laptop screen to its lowest refresh rate from 60 Hz up;
  - dims the laptop screen to 40%, and never raises it.
- **Programs and WSL**: optionally closes programs of the Game Mode list, with the same safe closing, and shuts WSL down unless some program is using it.
- **Automatic end**: the background watcher ends Eco Mode when the charger is plugged in, restores everything and shows a summary. It can also be ended by hand at any time.

Game Mode and Eco Mode are never on together, because both set the power mode.

### Cleanup

- **Analysis first**: the app analyzes before anything is touched and shows the size of each category. You check what to clean.
- **Categories covered**:
  - Windows temp files, Windows Update downloads, Delivery Optimization, crash dumps and the component store;
  - caches and logs of Chrome, Edge, Brave, Firefox, VS Code, Steam, Epic, Ubisoft, EA, Riot, Discord, Logitech G HUB and AMD driver installers;
  - npm, pip and Yarn package caches (not preselected);
  - leftovers of uninstalled programs.
- **Remembered selection**: what you check is remembered when you clean, and the next analysis starts the same way. Settings has a link to go back to the defaults.
- **Caution group**: categories that can cost you something are tagged and never preselected, not even after you cleaned them once. Examples are the Recycle Bin, AMD and DirectX shader caches, old driver packages (without them a driver cannot be rolled back), folders of uninstalled Steam games (they may hold replays or mods) and browser profiles of uninstalled browsers.
- **Elevation only when needed**: administrator rights are requested only for system items. That means one optional prompt to measure them and one prompt to clean them, and only when a selected category has any.
- **Running programs block their category**: if Chrome is open, its cache is skipped and you can close it from the app.
- **Report**: the results of a cleanup can be exported to a text file.

### Drivers

Looks for newer drivers and installs only what you choose. It is the only page that goes online, and only when you click: it asks Windows Update and the graphics card vendor's site.

- **Graphics driver from its vendor**: shows the driver of AMD, NVIDIA and Intel graphics cards. For AMD, it compares the installed AMD Software release with the latest one on AMD's site. When there is a newer one, it downloads AMD's own installer, checks that the file carries AMD's digital signature and opens it. NVIDIA and Intel get a link to their drivers page.
- **Windows Update drivers**: lists the driver updates that Windows Update offers for this computer, which Microsoft tests and signs. Nothing starts checked. A restore point is created first, unless you turn that off; when Windows cannot create it, nothing is installed. Firmware updates are flagged.
- **Old driver packages**: Cleanup has a Caution category for the driver versions that no device uses and that have a newer version in the driver store. Windows itself refuses to remove a package that is still in use.

### Decrapify

A page for what Windows does for Microsoft rather than for you. It analyzes first and shows the state of each item on this computer; nothing changes until you check items and apply them.

- **Data collection**:
  - the telemetry services (`DiagTrack`) and the data collection scheduled tasks;
  - diagnostic data at the minimum the edition allows, tailored experiences, feedback requests, inking and typing personalization, activity history and the language list for websites;
  - optionally, app launch tracking, Windows Error Reporting and the Compatibility Appraiser, which runs `CompatTelRunner.exe`.
- **Ads and suggestions**: advertising ID, apps installed without asking, tips, suggestions in Settings and in Start, lock screen tips, File Explorer promotions, search highlights and Bing results in Start search.
- **AI**: Recall and Copilot.
- **Background**: uploading updates to other PCs (Delivery Optimization) and, optionally, Edge running hidden.
- **Bundled apps**: uninstalled for the current user only, such as Feedback Hub, Widgets, the Bing apps, Microsoft 365 (Office) and Tips. Apps people may rely on (Phone Link, Outlook, Teams, Quick Assist, Get Help) are tagged as caution and never preselected.

Every change can be undone back to the Windows default, except app removals; for those, the page links to the app in the Microsoft Store. Xbox, Game Bar, Windows Search, Windows Update, Defender and OneDrive are left alone. The items are in [`debloat.json`](debloat.json). Services, tasks, machine settings and policies go through the elevated helper, which only accepts ids from that file.

### History

A list of the cleanups done, with space freed, and of past Game Mode and Eco Mode sessions.

### Accessibility

Every button, link, checkbox and option can be reached with **Tab** and activated with **Space** or **Enter**, with a visible focus outline. Screen readers such as Narrator and NVDA get each control's role, label and state through AccessKit.

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
  - it only installs driver updates that its own new search of Windows Update still offers;
  - it only reads and writes files inside the app's exchange folder.
- **Downloaded installers** run only after Windows confirms a valid signature from the vendor. A file that fails the check is deleted.

## Requirements

- Windows 11, 64-bit. Windows 10 should work but has not been tested.
- To build: [Rust](https://rustup.rs) stable. See [Building](#building).

## Building

The project is developed with the GNU toolchain (`x86_64-pc-windows-gnu`). It needs `dlltool.exe` and `as.exe` from [MinGW-w64](https://www.mingw-w64.org/) binutils, for example from MSYS2.

```powershell
rustup default stable-x86_64-pc-windows-gnu
.\build.ps1             # release build, installed in %LOCALAPPDATA%\Programs\Clutch
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
| Program | `%LOCALAPPDATA%\Programs\Clutch` |
| Settings, history, current session | `%LOCALAPPDATA%\Clutch\*.json` |
| Custom cleanup rules | `%LOCALAPPDATA%\Clutch\custom-categories.json` |
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
| `driverStore` | Removes, with `pnputil`, the driver packages that no device uses and that have a newer version in the driver store |

Targets with `"admin": true` run in the elevated helper. Targets with `notInstalled` apply only while none of the listed programs is installed.

### Custom rules

Your own categories go in `%LOCALAPPDATA%\Clutch\custom-categories.json`, in the same format as the catalog. **Settings → Open the rules file** creates it with an example and opens it in Notepad. The rules show up in the Cleanup page under **Your rules** at the next analysis.

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
  eco_mode.rs    Eco Mode: status checks, activation and what it restores
  watcher.rs     background process that ends the session when the game exits or the charger is plugged in
  cleanup.rs     catalog, safety rules, analysis and removal
  debloat.rs     Decrapify: catalog, state of each item, apply and undo
  drivers.rs     driver store, Windows Update drivers and the graphics vendor's installer
  xml.rs         minimal reading of the XML that Windows tools and launchers write
  admin.rs       elevated helper
  storage.rs     settings, history and session files
  steam.rs       Steam libraries and installed games
  games.rs       games from Steam, Epic Games, GOG, EA app, Battle.net, Xbox app and executables added by hand
  system.rs      Win32 wrappers (processes, windows, power, displays, registry, services)
categories.json  cleanup catalog
debloat.json     Decrapify catalog
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
- [x] Turn off Windows data collection, ads and bundled apps (Decrapify)
- [x] Eco Mode, the opposite of Game Mode, for battery life
- [x] Driver updates from Windows Update and from AMD, and cleanup of old driver packages
- [ ] More features in general

Ideas:

- [x] Option to send removed items to the Recycle Bin instead of deleting them
- [x] User-defined cleanup rules in a separate file, on top of the built-in catalog
- [x] Per-game profiles (which programs to close, power settings)
- [x] Game launch from other launchers: Epic Games and GOG found automatically, any other game through its executable
- [x] Automatic detection of EA app, Battle.net and Xbox app games
- [x] Exportable cleanup report
- [x] Executable icon and version info embedded as Windows resources
- [x] CI with tests on every release
- [ ] Signed builds
- [x] Distribution through Scoop (manifest in each release)
- [ ] Distribution through winget
- [x] Keyboard navigation and screen reader support

## License

[MIT](LICENSE).
