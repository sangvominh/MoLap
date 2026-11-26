# MoLap Open Plugin SDK (`v0.2`)

MoLap can host lightweight community widgets directly inside the Dynamic Island.
Plugins are completely language-agnostic — write them in **PowerShell, Python, Node.js, Batch, Go, Rust, or any executable** that prints simple `key=value` lines to standard output.

---

## 1. Discovery Paths

MoLap scans these two directories on startup and whenever you click **Reload** in **Control Center → Plugins**:

1. **Portable / Bundled Plugins** *(next to `MoLap.exe`)*:
   ```text
   .\plugins\<plugin-id>\plugin.ini
   ```
2. **Per-User Plugins** *(takes precedence if both exist with the same `<id>`)*:
   ```text
   %APPDATA%\MoLap\plugins\<plugin-id>\plugin.ini
   ```

Each plugin folder must contain a `plugin.ini` manifest file.

---

## 2. Manifest Format (`plugin.ini`)

```ini
id = weather-now
name = Weather
author = yourname
version = 1.0.0
description = Current local temperature and forecast
interval = 300
command = powershell -NoProfile -ExecutionPolicy Bypass -File run.ps1
action_label = Refresh
action_command = powershell -NoProfile -ExecutionPolicy Bypass -File run.ps1 --force
```

| Key | Required | Description |
| :--- | :---: | :--- |
| `id` | Yes | Unique ASCII slug (`[a-z0-9-_]`), used internally and in `settings.txt`. |
| `name` | Yes | Short display title shown in the Dynamic Island tab bar and Control Center. |
| `author` | No | Maintainer handle or name. |
| `version` | No | Semantic version string (e.g. `1.0.0`). |
| `description` | No | One-line summary displayed inside Control Center → Plugins. |
| `interval` | No | Poll interval in seconds (`1` to `3600`, default `5`). |
| `command` | Yes | Command executed relative to the plugin folder on every poll tick. |
| `action_label` | No | Optional short button label shown in the expanded plugin view. |
| `action_command` | No | Command executed asynchronously when the user clicks `action_label`. |

---

## 3. Output Protocol (`stdout`)

Every time `command` runs, MoLap parses UTF-8 lines formatted as `key=value`. Empty lines and `#` comments are ignored.

| Key | Type / Example | Description |
| :--- | :--- | :--- |
| `title` | `title=Branch: main` | Primary headline in the expanded plugin card. |
| `value` | `value=3 modified` | Right-aligned bold metric value. |
| `subtitle` | `subtitle=Up to date with origin` | Secondary detail line below the headline. |
| `badge` | `badge=CLEAN` | Small accent status pill next to the title. |
| `tone` | `neutral` \| `good` \| `warn` \| `alert` | Accent semantic color applied to badges, indicators, and progress bars. |
| `progress` | `0.0` .. `1.0` | Optional normalized progress bar rendered inside the card. |
| `item` | `item=Down \| 14.2 MB/s` | Up to **3** `Label \| Value` key-value rows shown below the header. |
| `pill` | `pill=14.2 MB/s` | Optional compact status badge surfaced in the Dynamic Island capsule. |
| `alert` | `alert=true` | When `true` or `1`, surfaces a temporary Dynamic Island toast notification. |

> **Tip**: Lines with malformed keys are ignored; emit `title` first or always print a clean fallback `title` and `tone=warn` if an external CLI or API call fails.

---

## 4. Lifecycle, Environment & Runtime Guarantees

- **Zero Idle Cost**: Disabled plugins consume `0%` CPU and never spawn subprocesses.
- **Hidden Subprocesses**: `command` and `action_command` run asynchronously in background worker threads with `CREATE_NO_WINDOW` (no flashing console windows).
- **Working Directory**: Commands always execute with their working directory set to the plugin folder (`...\plugins\<plugin-id>\`).
- **Event-Driven UI Refresh**: Once `stdout` closes, MoLap notifies the UI thread via `WM_PLUGIN_READY` and updates the cached plugin card only when content changes.
- **No Network Lock-In**: MoLap does not inject or block environment variables; scripts inherit normal user environment variables (e.g., `MOLAP_GIT_DIR`, `USERPROFILE`, `PATH`).

---

## 5. Quick-Start Copy-Paste Templates

### Option A: PowerShell (`plugins/cpu-check/plugin.ini` + `run.ps1`)

**`plugin.ini`**:
```ini
id = cpu-check
name = Quick Env
author = community
version = 0.1.0
description = Sample PowerShell system uptime and memory check
interval = 10
command = powershell -NoProfile -ExecutionPolicy Bypass -File run.ps1
```

**`run.ps1`**:
```powershell
$ErrorActionPreference = 'Stop'

try {
    $os = Get-CimInstance Win32_OperatingSystem
    $uptime = (Get-Date) - $os.LastBootUpTime
    $freeGb = [math]::Round($os.FreePhysicalMemory / 1MB, 1)
    $totalGb = [math]::Round($os.TotalVisibleMemorySize / 1MB, 1)
    $usedRatio = [math]::Min([math]::Max(($totalGb - $freeGb) / [math]::Max($totalGb, 0.1), 0.0), 1.0)

    Write-Output "title=Windows Uptime"
    Write-Output ("value={0}d {1}h" -f $uptime.Days, $uptime.Hours)
    Write-Output ("subtitle={0} GB free of {1} GB RAM" -f $freeGb, $totalGb)
    Write-Output "badge=ONLINE"
    Write-Output "tone=good"
    Write-Output ("progress={0:F2}" -f $usedRatio)
    Write-Output ("item=Host | {0}" -f $env:COMPUTERNAME)
    Write-Output ("item=User | {0}" -f $env:USERNAME)
} catch {
    Write-Output "title=System Probe Unavailable"
    Write-Output "value=--"
    Write-Output ("subtitle={0}" -f $_.Exception.Message)
    Write-Output "tone=warn"
}
```

### Option B: Python (`plugins/pomodoro-hook/plugin.ini` + `run.py`)

**`plugin.ini`**:
```ini
id = py-sample
name = Py Sample
author = community
version = 0.1.0
description = Sample Python plugin using standard library only
interval = 30
command = python run.py
```

**`run.py`**:
```python
from datetime import datetime
import platform

now = datetime.now()
print("title=Local Clock")
print(f"value={now.strftime('%H:%M')}")
print(f"subtitle={now.strftime('%A, %b %d')}")
print("badge=PY")
print("tone=neutral")
print(f"item=Python | {platform.python_version()}")
print(f"item=Arch | {platform.machine()}")
```

---

## 6. Local Testing & Debugging

1. **Test your script standalone first** from inside the plugin folder:
   ```powershell
   cd plugins\<your-plugin-id>
   powershell -NoProfile -ExecutionPolicy Bypass -File run.ps1
   ```
   Verify that output completes quickly (ideally `< 500 ms`), contains clean `key=value` lines, and never prompts interactively on `stdin`.
2. **Validate PowerShell syntax** before committing:
   ```powershell
   $tokens = $null; $errs = $null
   [System.Management.Automation.Language.Parser]::ParseFile("run.ps1", [ref]$tokens, [ref]$errs) | Out-Null
   if ($errs.Count -gt 0) { $errs } else { "Syntax OK" }
   ```
3. **Hot-Reload in MoLap**:
   - Open **Control Center → Plugins**, click **Reload**, and confirm your plugin is listed and enabled.
   - If your command exits non-zero or prints an error to `stderr`, MoLap captures the failure state on the plugin card so you can diagnose missing tools or path issues.

---

## 7. UX & Performance Best Practices

- **Keep strings tight**: Target `<= 24` chars for `title`, `<= 14` chars for `value`, and `<= 42` chars for `subtitle` so text stays crisp at `100%`–`200%` DPI scaling.
- **Pick sensible intervals**: Fast local counters can use `interval = 2..5`; external tools (`git`, `curl`, API queries) should use `interval = 10..300`.
- **Fail gracefully**: Catch exceptions and output valid fallback `key=value` lines (`tone=warn` + clear `subtitle`) instead of crashing with raw stack traces.
- **Use `alert=true` sparingly**: Reserve `alert=true` for genuinely actionable state changes (e.g., build finished, CI failed) rather than every poll tick.

---

## 8. Bundled Examples

Explore the shipped plugins in [`plugins/`](../plugins/):
- [`plugins/git-status/`](../plugins/git-status/) — Inspects `MOLAP_GIT_DIR` (or fallback repo root), displays branch/ahead/dirty counts, and wires `action_label = Fetch` to `git fetch --prune`.
- [`plugins/network-pulse/`](../plugins/network-pulse/) — Samples active Windows network interface deltas to report live `Down` and `Up` throughput.
