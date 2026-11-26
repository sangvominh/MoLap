# Security Policy

## Reporting a Vulnerability

If you discover a security issue in MoLap core (for example in Win32 FFI memory handling, PawnIO driver interaction, plugin command spawning, or settings parsing) or in an official bundled plugin, please **do not** disclose exploit details in a public GitHub issue.

Instead:
1. Open a private GitHub Security Advisory on the repository (or contact the maintainer privately via GitHub).
2. Include the MoLap version, Windows build, reproduction steps, and the affected module (e.g., `src/plugin.rs`, `src/sensors.rs`, `plugins/<name>/`).
3. Allow reasonable time for a fix and coordinated release before public disclosure.

## Security Model Notes

- **Zero third-party Rust crates**: Core MoLap compiles with an empty `[dependencies]` table in `Cargo.toml`, minimizing supply-chain surface.
- **Plugins execute local commands**: Any `plugin.ini` inside `./plugins/<id>/` or `%APPDATA%\MoLap\plugins\<id>/` runs its declared `command` (and optional `action_command`) under the current Windows user account. Only install third-party plugins from sources you trust and inspect their scripts before enabling them.
- **PawnIO thermal probes**: Hardware MSR/MCHBAR temperature reading uses the signed PawnIO driver and optional elevation prompts only when explicitly requested by the user.
