# Third-Party Notices

MoLap is licensed under the [MIT License](LICENSE). Its runtime Rust binary compiles with zero external Cargo crate dependencies (`std` + native Windows APIs only), and incorporates or redistributes the following third-party works:

## 1. liquid-glass-rs optical math & navigation model
- **Source**: <https://github.com/frixaco/liquid-glass-rs>
- **Copyright**: `Copyright (c) 2026 frixaco`
- **License**: MIT License
- **Usage in MoLap**: QuartzCore-recovered optical scalar functions (`lens`, `shadow_cdf`, `ring_shadow`, `key_fill`, `highlight_color`), pyramidal backdrop mip sampling (`BackdropPyramid`), sRGB chromatic dispersion (`aberrate_srgb`), and spring-coupled glass droplet navigation (`LiquidNav` / `DropletGeom`) were adapted into `src/liquid_glass.rs` for CPU-side Win32/GDI+ rendering.

## 2. PawnIO Hardware Telemetry Modules
- **Bundled binaries**: `third-party/PawnIO/IntelMSR.bin` and `third-party/PawnIO/IntelMCHBAR.bin`
- **License**: GNU Lesser General Public License v2.1 or later (`LGPL-2.1-or-later`) — full license text in [`third-party/PawnIO/COPYING`](third-party/PawnIO/COPYING)
- **Checksums & Attribution**:
  - `IntelMSR.bin` (`PawnIO.Modules 0.2.9`): `Copyright (C) 2025 namazso`, SHA-256 `5bfda87500160076158befc77300f81a388d72b11df2c794a3939ebe66777098`
  - `IntelMCHBAR.bin` (`PawnIO.Modules 0.2.11`): `Copyright (C) 2026 Gen Li`, SHA-256 `3f82b832d99b4aac37d2a20fdb7c9baa2a3bc0488612c9019c9484eb0e8a6eae`
- **Full notices**: See [`third-party/PawnIO/NOTICE.md`](third-party/PawnIO/NOTICE.md) for upstream source links and exact build provenance.
