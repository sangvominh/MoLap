# MoLap

**Lightweight Windows Dynamic Island with Liquid Glass UI, Hardware Telemetry, and a Language-Agnostic Open Plugin SDK.**

Built in pure Rust (`std` + native Win32, GDI+, DWM Acrylic, and Windows COM/WinRT APIs) with **zero external Cargo dependencies**, MoLap brings a smooth, translucent Dynamic Island capsule to the top of your Windows desktop — complete with spring physics, live thermal/CPU monitoring, media controls, synced lyrics, productivity mini-apps, and hot-reloadable community plugins.

---

## Highlights

- **Liquid Glass Optics & Spring Physics**: Real-time QuartzCore-inspired frosted glass refraction, chromatic rim highlights, DWM Desktop Acrylic Control Center, and 240 Hz substepped spring motion (`Fluid`, `Snappy`, `Gentle`) that drops to **0% idle GPU/CPU draw load** once settled.
- **Smart Telemetry & CPU Hog Alerts**: Compact CPU/GPU/RAM and thermal readouts (via signed `PawnIO` `IntelMSR`/`IntelMCHBAR` probes) with automatic temporary expansion alerts whenever a background process spikes CPU usage and thermal load.
- **Built-in Applets (6 Pages + Plugin Tabs)**:
  - **System & Thermals**: Live CPU/GPU/RAM load, package temperature, top-process spike detection, and PawnIO probe controls.
  - **Media & Synced Lyrics**: Windows SMTC session playback controls, album-art dominant chroma tinting, and live time-synced lyrics via LRCLIB.
  - **Volume HUD**: Responsive system volume & audio output capsule overlay.
  - **File Shelf**: Drag-and-drop or multi-file staging tray directly inside the Dynamic Island.
  - **Focus Timer (Pomodoro)**: Persistent work/break countdown timer.
  - **Hydration & Quick Todo**: Persistent daily water intake tracker and checklist.
- **Open Plugin SDK**: Add custom Dynamic Island tabs written in **any language** (PowerShell, Python, Node.js, Go, batch scripts, or compiled binaries) using a minimal `plugin.ini` manifest and `key=value` stdout protocol.

---

## Quick Start & Daily Usage

### Requirements
- **OS**: Windows 10 (1903+) or Windows 11 (22H2 / 24H2 / 26H2 recommended).
- **Build Toolchain** *(only if building from source)*: Rust MSVC toolchain (`rustup` + `cargo` + `rustc`, edition `2021`).

### Build from Source

```bat
git clone https://github.com/sangvominh/monitorLap.git
cd monitorLap
build.bat
```

Or run Cargo directly:

```powershell
cargo build --release --locked
```

`build.bat` compiles `target\release\molap.exe` and copies it to `MoLap.exe` in the repository root.

### Running MoLap

1. Launch `MoLap.exe`. It starts silently in the Windows System Tray and floats the Dynamic Island capsule at the top of your primary display.
2. **Right-click the System Tray icon** for quick actions:
   - `Control Center...` — Open the glass settings window (`General`, `Island`, `Plugins`).
   - `Dynamic Island` — Show/hide the floating capsule.
   - `Pin Island Expanded` — Keep the capsule expanded while working.
   - `Hardware Readings` — Open the hardware sensor readout view.
   - **PawnIO Thermal Setup / Elevation** — Enable live CPU/iGPU package temperatures when needed.
   - `Start with Windows` — Toggle per-user Windows `Run` registry startup.
   - `Quit MoLap` — Exit cleanly.
3. **Customize via Control Center**:
   - **General**: Theme (`System`, `Dark`, `Light`), Glass finish, Motion Feel (`Fluid`, `Snappy`, `Gentle`), Temperature Unit (`°C` / `°F`), Windows startup toggle, and live thermal probe status.
   - **Island**: Capsule Height calibration (`Auto`, `Standard`, or `Small (26H2)` for Windows 11 26H2 compact taskbar spacing), Anchor & offset, Capsule width/radius, Idle behavior, Volume HUD toggle, and CPU spike alert behavior.
   - **Plugins**: View active bundled or user plugins, inspect errors, and trigger hot reloads.

### Runtime Configuration & Data Paths

| Purpose | Location |
| :--- | :--- |
| Control Center preferences | `%APPDATA%\MoLap\settings.txt` |
| Hydration, Pomodoro & Todo state | `%APPDATA%\MoLap\island_state.ini` |
| User-installed plugins | `%APPDATA%\MoLap\plugins\<plugin-id>\plugin.ini` |
| Bundled / portable plugins | `.\plugins\<plugin-id>\plugin.ini` *(next to `MoLap.exe`)* |
| Bundled PawnIO modules | `.\third-party\PawnIO\IntelMSR.bin`, `IntelMCHBAR.bin` |

---

## Contributing to MoLap

MoLap is designed as an open playground with **two clear contribution tracks**. Whether you enjoy low-level Rust/Win32 rendering or want to build lightweight Dynamic Island widgets in 50 lines of PowerShell or Python, you are welcome here.

Read the full [`CONTRIBUTING.md`](CONTRIBUTING.md) before opening a PR.

### Track 1: Core App Engineering (`src/`)

Improve the core Windows runtime, glass renderer, telemetry probes, media integration, or UX:

| Module | Responsibility |
| :--- | :--- |
| [`src/main.rs`](src/main.rs) | Windows subsystem entry point, manifest embedding, startup handoff |
| [`src/ui.rs`](src/ui.rs) | System tray host, popup menu, sensor coordinator, power/lock suspension |
| [`src/island.rs`](src/island.rs) | Dynamic Island window lifecycle, pill/toast layout, 6 built-in applets, plugin renderer |
| [`src/island_data.rs`](src/island_data.rs) | 240 Hz substepped `Spring` physics, `Page` state, persistent Shelf/Hydration/Pomodoro/Todo |
| [`src/liquid_glass.rs`](src/liquid_glass.rs) | Optical scalar math (`lens`, `shadow_cdf`, `ring_shadow`), `BackdropPyramid`, `aberrate_srgb`, `LiquidNav` |
| [`src/gdiplus.rs`](src/gdiplus.rs) | `gdiplus.dll` flat C API FFI wrapper, Windows 11 typography, DIB `Surface` compositing |
| [`src/settings.rs`](src/settings.rs) | `AppSettings` schema, palettes, `update_live` zero-I/O preview & `flush_save` |
| [`src/settings_ui.rs`](src/settings_ui.rs) | 3-tab Control Center (`General`, `Island`, `Plugins`) with DWM Acrylic & surface caching |
| [`src/sensors.rs`](src/sensors.rs) | Background CPU, RAM, GPU, disk, PawnIO thermal sampling, top CPU hog detector |
| [`src/media.rs`](src/media.rs) | Windows SMTC media session controller, album art chroma extraction, volume polling |
| [`src/lyrics.rs`](src/lyrics.rs) | LRCLIB synced lyrics client and LRC timestamp parser |
| [`src/shelf.rs`](src/shelf.rs) | `HDROP` / OLE drag-and-drop & multi-file OpenFileName staging helper |
| [`src/plugin.rs`](src/plugin.rs) | `plugin.ini` discovery, background worker scheduler, `key=value` parser, `WM_PLUGIN_READY` |
| [`src/startup.rs`](src/startup.rs) | Per-user Windows `Run` registry toggle |
| [`src/win.rs`](src/win.rs) | Raw Win32 FFI bindings (`kernel32`, `user32`, `gdi32`, `shell32`, `ole32`, `advapi32`, `dwmapi`) |

**Core Dev Loop**:

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

> **Note**: Live hardware sensor tests (such as `sample_molap_laptop`) are marked `#[ignore]` for headless CI and can be run locally with `cargo test -- --ignored`. Core MoLap keeps `[dependencies]` in `Cargo.toml` empty — please avoid adding external crates unless discussed first in an issue.

---

### Track 2: Open Plugin Development (`plugins/` & SDK)

Build new Dynamic Island pages without writing Rust or compiling `MoLap.exe`.

#### 60-Second Plugin Quickstart

1. Create a folder `%APPDATA%\MoLap\plugins\hello-world\` (or `plugins\hello-world\` next to `MoLap.exe`).
2. Add `plugin.ini`:

```ini
id = hello-world
name = Hello
author = yourname
version = 0.1.0
description = Minimal custom Dynamic Island plugin
interval = 15
command = powershell -NoProfile -ExecutionPolicy Bypass -File run.ps1
```

3. Add `run.ps1` printing `key=value` lines to stdout:

```powershell
Write-Output "title=Workspace Ready"
Write-Output "value=Online"
Write-Output "subtitle=Updated at $(Get-Date -Format 'HH:mm:ss')"
Write-Output "tone=good"
Write-Output "progress=0.85"
Write-Output "item=Host | $env:COMPUTERNAME"
```

4. Open **Control Center → Plugins → Reload** (or restart MoLap). Your plugin appears as a dedicated tab inside the expanded Dynamic Island.

#### Bundled Showcase Plugins

- [`plugins/git-status/`](plugins/git-status/) — Tracks current Git branch, dirty/ahead counts, and provides a one-click `Fetch` action (`MOLAP_GIT_DIR`).
- [`plugins/network-pulse/`](plugins/network-pulse/) — Live download/upload throughput meter with `Net Pulse` capsule pill metrics.

See the complete specification, copy-paste PowerShell & Python templates, and debugging guide in [`docs/PLUGIN_SDK.md`](docs/PLUGIN_SDK.md).

---

## License, Third-Party Notices & Security

- **License**: Released under the [MIT License](LICENSE).
- **Third-Party Attribution**: Optical math inspiration from [`liquid-glass-rs`](https://github.com/frixaco/liquid-glass-rs) and LGPL-2.1-or-later `PawnIO` telemetry binaries are documented in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) and [`third-party/PawnIO/NOTICE.md`](third-party/PawnIO/NOTICE.md).
- **Security**: Please review [`SECURITY.md`](SECURITY.md) before reporting vulnerabilities or installing untrusted third-party plugins.
