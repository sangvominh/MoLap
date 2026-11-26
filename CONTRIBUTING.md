# Contributing to MoLap

Thank you for contributing to **MoLap**! To keep the project fast, lightweight, and easy to extend, contributions are organized into **two primary tracks**:

1. **Track 1 — Core Application (`src/`)**: Improving the pure-Rust Windows engine, Liquid Glass rendering, telemetry, SMTC/lyrics, Control Center UX, and plugin runtime.
2. **Track 2 — Community Plugins (`plugins/` & SDK)**: Creating new language-agnostic Dynamic Island widgets, integrations, or improving the Plugin SDK.

Please open an issue first for major architectural changes or new bundled plugins so we can align on scope.

---

## Track 1: Core Application Contributions (`src/`)

### Development Setup
- **OS**: Windows 10 or Windows 11 (x64).
- **Toolchain**: Rust stable (`edition = "2021"`, MSVC target `x86_64-pc-windows-msvc`).
- No CMake, LLVM, Node.js, or external Rust crates are required.

### Engineering Principles
1. **Zero External Rust Crates**: Keep `[dependencies]` in `Cargo.toml` empty. MoLap targets `std` + direct Win32/GDI+/DWM/COM FFI (`src/win.rs`, `src/gdiplus.rs`).
2. **0% Idle Overhead**: Animations (`240 Hz` substepped springs in `src/island_data.rs`, hover/droplet loops in `src/settings_ui.rs`) must stop timers immediately when settled and avoid disk writes during slider drags (`settings::update_live` + `flush_save()`).
3. **Optical Consistency**: UI surfaces in the Island and Control Center use `src/liquid_glass.rs` and `src/gdiplus.rs` (frosted translucent surfaces, crisp rim highlights, no redundant hard borders or duplicated button backgrounds).
4. **English UI Strings**: Keep all user-facing UI labels, tray menus, logs, comments, and documentation in clear, concise English.
5. **Safe FFI & Resource Cleanup**: Every Win32 handle (`HWND`, `HDC`, `HBITMAP`, `HANDLE`, GDI+ handles) must be paired with RAII `Drop` guards or deterministic cleanup paths.

### Required Verification Before Opening a Core PR

Run all four checks locally from the repository root:

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

If your change touches hardware probes (`src/sensors.rs`) or Win32 display/DPI logic, test `MoLap.exe` visually in both **Light** and **Dark** themes and note your Windows build in the PR description.

---

## Track 2: Plugin Contributions (`plugins/` & SDK)

Plugins let anyone extend MoLap's Dynamic Island without modifying Rust code.

### Plugin Design Guidelines
- **Minimal & Readable**: Dynamic Island real estate is compact. Keep `title` under ~24 characters, `value` short and scannable, and emit at most 3 `item=Label | Value` rows.
- **Respect `interval`**: Use `interval = 2..5` only for lightweight local reads (like network counters) and `interval = 15..300` for CLI tools or network/API calls.
- **No Popping Consoles or Hangs**: MoLap spawns plugin commands with `CREATE_NO_WINDOW`. Still, ensure your script sets short command/HTTP timeouts (e.g., 2–4 seconds) and degrades cleanly (returning `tone=warn` + brief `subtitle`) when offline or when a required CLI tool is missing.
- **Safe Defaults**: Never hardcode personal tokens, absolute user paths (`C:\Users\yourname\...`), or destructive `action_command` operations. Read configuration from environment variables when needed (see `MOLAP_GIT_DIR` in [`plugins/git-status/run.ps1`](plugins/git-status/run.ps1)).
- **Document Everything**: Include a concise `README.md` inside any submitted `plugins/<id>/` folder explaining dependencies, configuration env vars, and permissions.

### Submitting a Plugin
- **Bundled Showcase PR**: Add your folder under `plugins/<plugin-id>/` with `plugin.ini`, script(s), and `README.md`. Ensure PowerShell scripts pass `[System.Management.Automation.Language.Parser]::ParseFile` cleanly and follow [`docs/PLUGIN_SDK.md`](docs/PLUGIN_SDK.md).
- **Community Showcase Issue**: If you maintain a plugin in your own repository, open an issue using the **Plugin Submission** template so other users can discover it.

---

## Pull Request Workflow

1. Fork the repository and branch from `main` (e.g., `feat/core-media-seek` or `plugin/spotify-queue`).
2. Keep PRs focused on a single feature, fix, or plugin.
3. Fill out `.github/PULL_REQUEST_TEMPLATE.md` accurately — list every command you actually ran and manual UI checks performed.
4. Keep `MoLap.exe`, `target/`, and scratch directories out of your commits.
