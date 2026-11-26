use crate::win::{wide, Handle, PostMessageW, ShellExecuteW};
use std::{
    collections::HashMap,
    fs,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    ptr::null,
    sync::{Arc, Condvar, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub const WM_PLUGIN_READY: u32 = 0x8019;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub author: String,
    pub version: String,
    pub description: String,
    pub interval_secs: u32,
    pub command: String,
    pub action_label: String,
    pub action_command: String,
    pub dir: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PluginTone {
    #[default]
    Normal,
    Good,
    Warn,
    Critical,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct PluginRenderData {
    pub pill: String,
    pub alert: bool,
    pub title: String,
    pub value: String,
    pub subtitle: String,
    pub badge: String,
    pub tone: PluginTone,
    pub progress: Option<f32>,
    pub items: Vec<(String, String)>,
    pub updated: bool,
}

#[derive(Clone)]
pub struct PluginEntry {
    pub manifest: PluginManifest,
    pub data: PluginRenderData,
}

struct WakeState {
    paused: bool,
    quit: bool,
    reload: bool,
    enabled_ids: Vec<String>,
    run_now: Vec<String>,
    run_action: Vec<String>,
}

struct SharedPlugins {
    entries: Vec<PluginEntry>,
}

pub struct PluginHost {
    shared: Arc<Mutex<SharedPlugins>>,
    wake: Arc<(Mutex<WakeState>, Condvar)>,
    worker: Option<JoinHandle<()>>,
}

impl PluginHost {
    pub fn start(hwnd: Handle, enabled_ids: Vec<String>) -> Self {
        let manifests = discover_plugins();
        let initial_entries = manifests
            .into_iter()
            .map(|manifest| PluginEntry {
                data: PluginRenderData {
                    title: manifest.name.clone(),
                    subtitle: manifest.description.clone(),
                    ..Default::default()
                },
                manifest,
            })
            .collect();
        let shared = Arc::new(Mutex::new(SharedPlugins {
            entries: initial_entries,
        }));
        let wake = Arc::new((
            Mutex::new(WakeState {
                paused: false,
                quit: false,
                reload: false,
                enabled_ids,
                run_now: Vec::new(),
                run_action: Vec::new(),
            }),
            Condvar::new(),
        ));
        let shared_worker = Arc::clone(&shared);
        let wake_worker = Arc::clone(&wake);
        let worker = std::thread::Builder::new()
            .stack_size(192 * 1024)
            .spawn(move || worker_loop(hwnd, shared_worker, wake_worker))
            .ok();
        Self {
            shared,
            wake,
            worker,
        }
    }

    pub fn set_paused(&self, paused: bool) {
        let (lock, cv) = &*self.wake;
        let mut st = lock.lock().unwrap();
        if st.paused != paused {
            st.paused = paused;
            cv.notify_one();
        }
    }

    pub fn sync_enabled(&self, ids: &[String]) {
        let (lock, cv) = &*self.wake;
        let mut st = lock.lock().unwrap();
        if st.enabled_ids != ids {
            st.enabled_ids = ids.to_vec();
            cv.notify_one();
        }
    }

    pub fn reload(&self) {
        let manifests = discover_plugins();
        {
            let mut sh = self.shared.lock().unwrap();
            let mut old_data: HashMap<String, PluginRenderData> = sh
                .entries
                .drain(..)
                .map(|e| (e.manifest.id, e.data))
                .collect();
            sh.entries = manifests
                .into_iter()
                .map(|manifest| {
                    let data = old_data
                        .remove(&manifest.id)
                        .unwrap_or_else(|| PluginRenderData {
                            title: manifest.name.clone(),
                            subtitle: manifest.description.clone(),
                            ..Default::default()
                        });
                    PluginEntry { manifest, data }
                })
                .collect();
        }
        let (lock, cv) = &*self.wake;
        let mut st = lock.lock().unwrap();
        st.reload = true;
        cv.notify_one();
    }

    pub fn refresh(&self, id: &str) {
        let (lock, cv) = &*self.wake;
        let mut st = lock.lock().unwrap();
        if !st.run_now.iter().any(|x| x == id) {
            st.run_now.push(id.to_string());
        }
        cv.notify_one();
    }

    pub fn run_action(&self, id: &str) {
        let (lock, cv) = &*self.wake;
        let mut st = lock.lock().unwrap();
        if !st.run_action.iter().any(|x| x == id) {
            st.run_action.push(id.to_string());
        }
        cv.notify_one();
    }

    pub fn entries(&self) -> Vec<PluginEntry> {
        self.shared.lock().unwrap().entries.clone()
    }
}

impl Drop for PluginHost {
    fn drop(&mut self) {
        {
            let (lock, cv) = &*self.wake;
            let mut st = lock.lock().unwrap();
            st.quit = true;
            cv.notify_one();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn worker_loop(
    hwnd: Handle,
    shared: Arc<Mutex<SharedPlugins>>,
    wake: Arc<(Mutex<WakeState>, Condvar)>,
) {
    let mut schedule: HashMap<String, Instant> = HashMap::new();
    let (lock, cv) = &*wake;

    loop {
        let (force_ids, action_ids, enabled_ids) = {
            let mut st = lock.lock().unwrap();
            loop {
                if st.quit {
                    return;
                }
                if st.reload {
                    st.reload = false;
                    schedule.clear();
                }
                if !st.paused && !st.enabled_ids.is_empty() {
                    break;
                }
                st = cv.wait(st).unwrap();
            }
            (
                std::mem::take(&mut st.run_now),
                std::mem::take(&mut st.run_action),
                st.enabled_ids.clone(),
            )
        };

        let entries: Vec<PluginManifest> = {
            let sh = shared.lock().unwrap();
            sh.entries
                .iter()
                .filter(|e| enabled_ids.iter().any(|id| id == &e.manifest.id))
                .map(|e| e.manifest.clone())
                .collect()
        };

        // Run triggered custom actions first, then immediately refresh that plugin.
        let mut extra_refresh = Vec::new();
        for id in action_ids {
            if let Some(m) = entries
                .iter()
                .find(|m| m.id == id && !m.action_command.is_empty())
            {
                let _ = exec_plugin_command(&m.dir, &m.action_command);
                extra_refresh.push(id);
            }
        }

        let now = Instant::now();
        let mut due = Vec::new();
        for m in &entries {
            let forced = force_ids.iter().any(|id| id == &m.id)
                || extra_refresh.iter().any(|id| id == &m.id);
            let is_due = forced || schedule.get(&m.id).is_none_or(|&next| now >= next);
            if is_due {
                due.push(m.clone());
            }
        }

        let mut any_updated = false;
        for m in due {
            if lock.lock().unwrap().quit {
                return;
            }
            let rendered = match exec_plugin_command(&m.dir, &m.command) {
                Ok(out) => parse_output(&out, &m),
                Err(err) => PluginRenderData {
                    title: m.name.clone(),
                    value: "Error".into(),
                    subtitle: err,
                    badge: "ERR".into(),
                    tone: PluginTone::Critical,
                    updated: true,
                    ..Default::default()
                },
            };
            {
                let mut sh = shared.lock().unwrap();
                if let Some(entry) = sh.entries.iter_mut().find(|e| e.manifest.id == m.id) {
                    entry.data = rendered;
                }
            }
            schedule.insert(
                m.id.clone(),
                Instant::now() + Duration::from_secs(m.interval_secs as u64),
            );
            any_updated = true;
        }

        if any_updated && hwnd != 0 {
            unsafe {
                PostMessageW(hwnd, WM_PLUGIN_READY, 0, 0);
            }
        }

        let next_due = entries
            .iter()
            .filter_map(|m| schedule.get(&m.id).copied())
            .min();
        let wait_dur = next_due
            .map(|t| t.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|| Duration::from_secs(5))
            .clamp(Duration::from_millis(250), Duration::from_secs(30));

        let st = lock.lock().unwrap();
        if st.quit {
            return;
        }
        if st.run_now.is_empty() && st.run_action.is_empty() && !st.reload {
            drop(cv.wait_timeout(st, wait_dur).unwrap());
        }
    }
}

fn exec_plugin_command(dir: &Path, command: &str) -> Result<String, String> {
    if command.trim().is_empty() {
        return Err("Empty command".into());
    }
    let output = Command::new("cmd.exe")
        .args(["/C", command])
        .current_dir(dir)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .map_err(|e| format!("Failed to run: {e}"))?;

    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if text.trim().is_empty() && !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let first = err
            .lines()
            .next()
            .unwrap_or("Command exited with error")
            .trim();
        return Err(first.chars().take(72).collect());
    }
    if text.len() > 16_384 {
        text.truncate(16_384);
    }
    Ok(text)
}

pub fn discover_plugins() -> Vec<PluginManifest> {
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("plugins"));
            // Also check workspace root when running via target/release or target/debug
            if let Some(p2) = parent.parent().and_then(|p| p.parent()) {
                let candidate = p2.join("plugins");
                if candidate != parent.join("plugins") {
                    dirs.push(candidate);
                }
            }
        }
    }
    if let Some(user_dir) = user_plugins_dir() {
        dirs.push(user_dir);
    }

    let mut map: HashMap<String, PluginManifest> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for root in dirs {
        let Ok(read) = fs::read_dir(&root) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("plugin.ini");
            let Ok(raw) = fs::read_to_string(&manifest_path) else {
                continue;
            };
            if let Some(manifest) = parse_manifest(&raw, &path) {
                if !map.contains_key(&manifest.id) {
                    order.push(manifest.id.clone());
                }
                map.insert(manifest.id.clone(), manifest);
            }
        }
    }
    order.sort_by(|a, b| map[a].name.cmp(&map[b].name));
    order.into_iter().filter_map(|id| map.remove(&id)).collect()
}

pub fn user_plugins_dir() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("MoLap").join("plugins"))
}

pub fn open_plugins_folder(owner: Handle) {
    let target = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("plugins")))
        .filter(|p| p.exists())
        .or_else(|| {
            let u = user_plugins_dir()?;
            let _ = fs::create_dir_all(&u);
            Some(u)
        });
    let Some(path) = target else {
        return;
    };
    let path_str = path.to_string_lossy().into_owned();
    unsafe {
        ShellExecuteW(
            owner,
            wide("open").as_ptr(),
            wide(&path_str).as_ptr(),
            null(),
            null(),
            1,
        );
    }
}

pub fn parse_manifest(raw: &str, dir: &Path) -> Option<PluginManifest> {
    let mut id = String::new();
    let mut name = String::new();
    let mut author = "Community".to_string();
    let mut version = "1.0.0".to_string();
    let mut description = String::new();
    let mut interval_secs = 5u32;
    let mut command = String::new();
    let mut action_label = String::new();
    let mut action_command = String::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with(';')
            || trimmed.starts_with('[')
        {
            continue;
        }
        let Some((k, v)) = trimmed.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim();
        match key {
            "id" => {
                id = val
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                    .take(32)
                    .collect::<String>()
                    .to_ascii_lowercase();
            }
            "name" => name = val.chars().take(16).collect(),
            "author" => author = val.chars().take(24).collect(),
            "version" => version = val.chars().take(12).collect(),
            "description" => description = val.chars().take(72).collect(),
            "interval" => {
                if let Ok(secs) = val.parse::<u32>() {
                    interval_secs = secs.clamp(2, 3600);
                }
            }
            "command" => command = val.to_string(),
            "action_label" => action_label = val.chars().take(16).collect(),
            "action_command" => action_command = val.to_string(),
            _ => {}
        }
    }

    if id.is_empty() || command.is_empty() {
        return None;
    }
    if name.is_empty() {
        name = id.clone();
    }
    Some(PluginManifest {
        id,
        name,
        author,
        version,
        description,
        interval_secs,
        command,
        action_label,
        action_command,
        dir: dir.to_path_buf(),
    })
}

pub fn parse_output(raw: &str, manifest: &PluginManifest) -> PluginRenderData {
    let mut data = PluginRenderData {
        title: manifest.name.clone(),
        updated: true,
        ..Default::default()
    };
    let mut has_kv = false;
    let mut plain_lines = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once('=') {
            let key = k.trim();
            let val = v.trim();
            match key {
                "pill" => {
                    has_kv = true;
                    data.pill = val.chars().take(36).collect();
                }
                "alert" => {
                    has_kv = true;
                    data.alert = matches!(val.to_ascii_lowercase().as_str(), "1" | "true" | "yes");
                }
                "title" => {
                    has_kv = true;
                    data.title = val.chars().take(28).collect();
                }
                "value" => {
                    has_kv = true;
                    data.value = val.chars().take(36).collect();
                }
                "subtitle" => {
                    has_kv = true;
                    data.subtitle = val.chars().take(64).collect();
                }
                "badge" => {
                    has_kv = true;
                    data.badge = val.chars().take(14).collect();
                }
                "tone" => {
                    has_kv = true;
                    data.tone = match val.to_ascii_lowercase().as_str() {
                        "good" | "ok" | "green" => PluginTone::Good,
                        "warn" | "amber" | "yellow" => PluginTone::Warn,
                        "critical" | "error" | "red" => PluginTone::Critical,
                        _ => PluginTone::Normal,
                    };
                }
                "progress" => {
                    has_kv = true;
                    if let Ok(p) = val.trim_end_matches('%').trim().parse::<f32>() {
                        data.progress = Some((p / 100.0).clamp(0.0, 1.0));
                    }
                }
                "item" => {
                    has_kv = true;
                    if data.items.len() < 3 {
                        if let Some((left, right)) = val.split_once('|') {
                            data.items.push((
                                left.trim().chars().take(24).collect(),
                                right.trim().chars().take(24).collect(),
                            ));
                        } else {
                            data.items
                                .push((val.chars().take(32).collect(), String::new()));
                        }
                    }
                }
                _ => plain_lines.push(trimmed.to_string()),
            }
        } else {
            plain_lines.push(trimmed.to_string());
        }
    }

    if !has_kv && !plain_lines.is_empty() {
        data.value = plain_lines[0].chars().take(36).collect();
        if let Some(sub) = plain_lines.get(1) {
            data.subtitle = sub.chars().take(64).collect();
        }
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_parses_defaults_and_clamps_interval() {
        let raw = r#"
# Example plugin
id = Net-Pulse
name = Network
author = sangvominh
version = 0.2.0
description = Live download & upload throughput
interval = 1
command = powershell -NoProfile -File run.ps1
action_label = Flush DNS
action_command = ipconfig /flushdns
"#;
        let m = parse_manifest(raw, Path::new("plugins/network-pulse")).unwrap();
        assert_eq!(m.id, "net-pulse");
        assert_eq!(m.name, "Network");
        assert_eq!(m.interval_secs, 2); // clamped from 1 -> 2
        assert_eq!(m.action_label, "Flush DNS");
        assert!(parse_manifest("name = Broken", Path::new(".")).is_none());
    }

    #[test]
    fn output_parses_structured_kv_and_plain_text() {
        let m = parse_manifest("id=demo\ncommand=echo hi", Path::new(".")).unwrap();
        let structured = parse_output(
            "pill = ↓ 4.2 MB/s\nalert = true\nvalue = 4.2 MB/s\nsubtitle = Wi-Fi 6\nbadge = LIVE\ntone = good\nprogress = 72\nitem = Down | 4.2 MB/s\nitem = Up | 820 KB/s\n",
            &m,
        );
        assert_eq!(structured.pill, "↓ 4.2 MB/s");
        assert!(structured.alert);
        assert_eq!(structured.value, "4.2 MB/s");
        assert_eq!(structured.badge, "LIVE");
        assert_eq!(structured.tone, PluginTone::Good);
        assert_eq!(structured.progress, Some(0.72));
        assert_eq!(structured.items.len(), 2);

        let plain = parse_output("hello world\nsecond line", &m);
        assert_eq!(plain.value, "hello world");
        assert_eq!(plain.subtitle, "second line");
    }
}
