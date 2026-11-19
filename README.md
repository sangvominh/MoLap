# MoLap

MoLap is a native Windows taskbar monitor for CPU, RAM, GPUs, and disks. It is written in Rust and calls Win32 directly. You can run the release without installing Rust or .NET.

## Download and run

1. Download `MoLap-v0.1.0-windows-x64.zip` from the **Assets** section on [GitHub Releases](https://github.com/sangvominh/MoLap/releases).
2. Extract the ZIP and open `MoLap.exe`. The monitor appears on the taskbar near the system tray and clock.

MoLap targets Windows 10 and 11 on x64. Available sensors depend on your hardware and drivers.

## Readings

| Component | What MoLap shows |
| --- | --- |
| CPU | Usage percentage and temperature |
| RAM | Memory in use, in GB |
| GPU | Usage and temperature for each integrated or dedicated GPU |
| Disk | Active time and temperature for each disk. The fill inside the icon shows how much storage is used. The Windows drive appears first. |

Hover over an item for details. Left-click the monitor to open its menu:

- **Show readings** opens a list of all readings.
- **Hide monitor** hides the taskbar strip. To restore it, click the MoLap icon in the system tray or under the **`^`** overflow arrow, then choose **Show monitor**.
- **Exit MoLap** closes the app.

## Temperature and PawnIO

Intel CPU and Ice Lake integrated GPU temperatures may require [PawnIO](https://pawnio.eu/). Select **Restart as administrator for temperature** in the MoLap menu. If PawnIO is missing, MoLap asks before installing it through WinGet. Windows may ask you to confirm administrator access.

`--°` means MoLap could not get a temperature reading for that device. NVIDIA GPUs and disks use separate sensor interfaces. See the [third-party notices](third-party/PawnIO/NOTICE.md) for the PawnIO module licenses and source links.

## Build from source

Install [Rust and Cargo](https://www.rust-lang.org/tools/install) with a Windows x64 toolchain. Open PowerShell in the repository and run:

```powershell
.\build.bat
```

The script creates `MoLap.exe` in the repository root. The first release was built with the `x86_64-pc-windows-gnu` target.
