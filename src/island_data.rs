use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
    pub target: f32,
}

impl Spring {
    pub const fn new(initial: f32) -> Self {
        Self {
            value: initial,
            velocity: 0.0,
            target: initial,
        }
    }

    pub fn set(&mut self, value: f32) {
        self.value = value;
        self.target = value;
        self.velocity = 0.0;
    }

    pub fn step(&mut self, dt: f32, stiffness: f32, damping: f32) -> bool {
        const FIXED_DT: f32 = 1.0 / 240.0;
        let mut rem = if dt.is_finite() {
            dt.clamp(0.0005, 0.05)
        } else {
            FIXED_DT
        };
        while rem > 0.0 {
            let h = if rem <= FIXED_DT + 1e-6 {
                rem
            } else {
                FIXED_DT
            };
            let force = -stiffness * (self.value - self.target) - damping * self.velocity;
            self.velocity += force * h;
            self.value += self.velocity * h;
            rem -= h;
        }
        if (self.value - self.target).abs() < 0.002 && self.velocity.abs() < 0.008 {
            self.value = self.target;
            self.velocity = 0.0;
            false
        } else {
            true
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    System = 0,
    Media = 1,
    Shelf = 2,
    Water = 3,
    Pomodoro = 4,
    Todo = 5,
}

impl Page {
    pub const COUNT: usize = 6;
    pub const ALL: [Self; Self::COUNT] = [
        Self::System,
        Self::Media,
        Self::Shelf,
        Self::Water,
        Self::Pomodoro,
        Self::Todo,
    ];

    pub fn from_index(index: usize) -> Self {
        Self::ALL[index % Self::COUNT]
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Media => "Media",
            Self::Shelf => "Shelf",
            Self::Water => "Hydrate",
            Self::Pomodoro => "Focus",
            Self::Todo => "Tasks",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PomodoroPhase {
    Focus,
    ShortBreak,
    LongBreak,
}

impl PomodoroPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Focus => "Focus",
            Self::ShortBreak => "Short Break",
            Self::LongBreak => "Long Break",
        }
    }
}

pub struct PomodoroState {
    pub focus_min: u32,
    pub short_break_min: u32,
    pub long_break_min: u32,
    pub sessions_per_round: u32,
    pub phase: PomodoroPhase,
    pub session_index: u32,
    pub running: bool,
    pub remaining_secs: u32,
    pub alert_secs: u8,
}

impl Default for PomodoroState {
    fn default() -> Self {
        Self {
            focus_min: 25,
            short_break_min: 5,
            long_break_min: 15,
            sessions_per_round: 4,
            phase: PomodoroPhase::Focus,
            session_index: 1,
            running: false,
            remaining_secs: 25 * 60,
            alert_secs: 0,
        }
    }
}

impl PomodoroState {
    pub fn phase_total_secs(&self) -> u32 {
        let mins = match self.phase {
            PomodoroPhase::Focus => self.focus_min,
            PomodoroPhase::ShortBreak => self.short_break_min,
            PomodoroPhase::LongBreak => self.long_break_min,
        };
        mins.max(1) * 60
    }

    pub fn progress(&self) -> f32 {
        let total = self.phase_total_secs() as f32;
        if total <= 0.0 {
            return 0.0;
        }
        (1.0 - (self.remaining_secs as f32 / total)).clamp(0.0, 1.0)
    }

    pub fn round_total_minutes(&self) -> u32 {
        self.sessions_per_round * self.focus_min
            + self.sessions_per_round.saturating_sub(1) * self.short_break_min
            + self.long_break_min
    }

    pub fn toggle(&mut self) {
        self.running = !self.running;
        self.alert_secs = 0;
    }

    pub fn reset(&mut self) {
        self.running = false;
        self.phase = PomodoroPhase::Focus;
        self.session_index = 1;
        self.remaining_secs = self.phase_total_secs();
        self.alert_secs = 0;
    }

    pub fn adjust_focus_min(&mut self, delta: i32) {
        if self.running {
            return;
        }
        let next = (self.focus_min as i32 + delta).clamp(5, 90) as u32;
        self.focus_min = next;
        if self.phase == PomodoroPhase::Focus {
            self.remaining_secs = self.phase_total_secs();
        }
    }

    pub fn skip_phase(&mut self) -> &'static str {
        self.running = false;
        self.alert_secs = 0;
        match self.phase {
            PomodoroPhase::Focus => {
                if self.session_index >= self.sessions_per_round {
                    self.phase = PomodoroPhase::LongBreak;
                } else {
                    self.phase = PomodoroPhase::ShortBreak;
                }
            }
            PomodoroPhase::ShortBreak => {
                self.phase = PomodoroPhase::Focus;
                self.session_index = (self.session_index + 1).min(self.sessions_per_round);
            }
            PomodoroPhase::LongBreak => {
                self.phase = PomodoroPhase::Focus;
                self.session_index = 1;
            }
        }
        self.remaining_secs = self.phase_total_secs();
        self.phase.label()
    }

    pub fn finish_time_label(&self, now_hour: u16, now_min: u16) -> String {
        let add_mins = self.remaining_secs.div_ceil(60);
        let total = (now_hour as u32 * 60 + now_min as u32 + add_mins) % (24 * 60);
        format!("{:02}:{:02}", total / 60, total % 60)
    }

    pub fn status_text(&self) -> &'static str {
        if self.running {
            "Running"
        } else if self.remaining_secs < self.phase_total_secs() {
            "Paused"
        } else {
            "Ready"
        }
    }

    pub fn tick_second(&mut self) -> Option<&'static str> {
        if self.alert_secs > 0 {
            self.alert_secs -= 1;
        }
        if !self.running {
            return None;
        }
        if self.remaining_secs > 1 {
            self.remaining_secs -= 1;
            return None;
        }
        self.running = false;
        self.alert_secs = 8;
        match self.phase {
            PomodoroPhase::Focus => {
                if self.session_index >= self.sessions_per_round {
                    self.phase = PomodoroPhase::LongBreak;
                } else {
                    self.phase = PomodoroPhase::ShortBreak;
                }
            }
            PomodoroPhase::ShortBreak => {
                self.phase = PomodoroPhase::Focus;
                self.session_index = (self.session_index + 1).min(self.sessions_per_round);
            }
            PomodoroPhase::LongBreak => {
                self.phase = PomodoroPhase::Focus;
                self.session_index = 1;
            }
        }
        self.remaining_secs = self.phase_total_secs();
        Some(self.phase.label())
    }
}

pub struct WaterState {
    pub date_key: u32,
    pub consumed_ml: u32,
    pub goal_ml: u32,
    pub cup_ml: u32,
    pub interval_min: u32,
    pub reminder_enabled: bool,
    pub next_reminder_secs: u32,
    pub settings_open: bool,
    pub due_alert: bool,
}

impl WaterState {
    pub fn new(today: u32) -> Self {
        Self {
            date_key: today,
            consumed_ml: 0,
            goal_ml: 1500,
            cup_ml: 250,
            interval_min: 60,
            reminder_enabled: true,
            next_reminder_secs: 60 * 60,
            settings_open: false,
            due_alert: false,
        }
    }

    pub fn sync_date(&mut self, today: u32) {
        if self.date_key != today {
            self.date_key = today;
            self.consumed_ml = 0;
            self.due_alert = false;
            self.next_reminder_secs = self.interval_min * 60;
        }
    }

    pub fn drink(&mut self) {
        self.consumed_ml = self.consumed_ml.saturating_add(self.cup_ml).min(99_950);
        self.due_alert = false;
        self.next_reminder_secs = self.interval_min * 60;
    }

    pub fn undo_drink(&mut self) -> bool {
        if self.consumed_ml == 0 {
            return false;
        }
        self.consumed_ml = self.consumed_ml.saturating_sub(self.cup_ml);
        true
    }

    pub fn fraction(&self) -> f32 {
        if self.goal_ml == 0 {
            return 0.0;
        }
        (self.consumed_ml as f32 / self.goal_ml as f32).clamp(0.0, 1.0)
    }

    pub fn percent(&self) -> u32 {
        if self.goal_ml == 0 {
            return 0;
        }
        ((self.consumed_ml as f32 / self.goal_ml as f32) * 100.0)
            .round()
            .clamp(0.0, 999.0) as u32
    }

    pub fn adjust_goal(&mut self, delta: i32) {
        let next = (self.goal_ml as i32 + delta).clamp(500, 5000);
        self.goal_ml = next as u32;
    }

    pub fn adjust_cup(&mut self, delta: i32) {
        let next = (self.cup_ml as i32 + delta).clamp(50, 1000);
        self.cup_ml = next as u32;
    }

    pub fn adjust_interval(&mut self, delta: i32) {
        let next = (self.interval_min as i32 + delta).clamp(15, 180);
        self.interval_min = next as u32;
        self.next_reminder_secs = self.interval_min * 60;
    }

    pub fn tick_second(&mut self, today: u32) -> bool {
        self.sync_date(today);
        if !self.reminder_enabled {
            self.due_alert = false;
            return false;
        }
        if self.next_reminder_secs > 1 {
            self.next_reminder_secs -= 1;
            return false;
        }
        self.next_reminder_secs = self.interval_min * 60;
        if self.consumed_ml < self.goal_ml {
            self.due_alert = true;
            return true;
        }
        false
    }

    pub fn next_time_label(&self, now_hour: u16, now_min: u16) -> String {
        let add_mins = self.next_reminder_secs.div_ceil(60);
        let total = (now_hour as u32 * 60 + now_min as u32 + add_mins) % (24 * 60);
        format!("{:02}:{:02}", total / 60, total % 60)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskItem {
    pub id: u64,
    pub title: String,
    pub done: bool,
    pub date_key: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditMode {
    Add,
    Edit(usize),
}

pub struct TodoState {
    pub tasks: Vec<TaskItem>,
    pub next_id: u64,
    pub today_only: bool,
    pub selected: Option<usize>,
    pub scroll: usize,
    pub editing: Option<EditMode>,
    pub input: String,
}

impl Default for TodoState {
    fn default() -> Self {
        Self {
            tasks: Vec::new(),
            next_id: 1,
            today_only: true,
            selected: None,
            scroll: 0,
            editing: None,
            input: String::new(),
        }
    }
}

impl TodoState {
    pub fn visible_indices(&self, today: u32) -> Vec<usize> {
        self.tasks
            .iter()
            .enumerate()
            .filter(|(_, item)| !self.today_only || item.date_key == today)
            .map(|(idx, _)| idx)
            .collect()
    }

    pub fn counts(&self, today: u32) -> (usize, usize) {
        let visible = self.visible_indices(today);
        let done = visible.iter().filter(|&&i| self.tasks[i].done).count();
        (done, visible.len())
    }

    pub fn top_pending_task(&self, today: u32) -> Option<&str> {
        self.visible_indices(today)
            .into_iter()
            .find(|&i| !self.tasks[i].done)
            .map(|i| self.tasks[i].title.as_str())
    }

    pub fn toggle_done(&mut self, task_idx: usize) {
        if let Some(task) = self.tasks.get_mut(task_idx) {
            task.done = !task.done;
        }
    }

    pub fn begin_add(&mut self) {
        self.editing = Some(EditMode::Add);
        self.input.clear();
    }

    pub fn begin_edit_at(&mut self, idx: usize) -> bool {
        let Some(task) = self.tasks.get(idx) else {
            return false;
        };
        self.selected = Some(idx);
        self.input = task.title.clone();
        self.editing = Some(EditMode::Edit(idx));
        true
    }

    pub fn begin_edit(&mut self, today: u32) -> bool {
        let idx = self
            .selected
            .filter(|&i| {
                self.tasks
                    .get(i)
                    .is_some_and(|t| !self.today_only || t.date_key == today)
            })
            .or_else(|| self.visible_indices(today).into_iter().next());
        let Some(idx) = idx else {
            return false;
        };
        self.begin_edit_at(idx)
    }

    pub fn push_char(&mut self, ch: char) {
        if ch.is_control() || self.input.chars().count() >= 64 {
            return;
        }
        self.input.push(ch);
    }

    pub fn backspace(&mut self) {
        self.input.pop();
    }

    pub fn commit(&mut self, today: u32) -> bool {
        let Some(mode) = self.editing.take() else {
            return false;
        };
        let title = self.input.trim().to_string();
        self.input.clear();
        if title.is_empty() {
            return false;
        }
        match mode {
            EditMode::Add => {
                let id = self.next_id;
                self.next_id += 1;
                self.tasks.push(TaskItem {
                    id,
                    title,
                    done: false,
                    date_key: today,
                });
                self.selected = Some(self.tasks.len() - 1);
            }
            EditMode::Edit(idx) => {
                if let Some(task) = self.tasks.get_mut(idx) {
                    task.title = title;
                }
            }
        }
        true
    }

    pub fn cancel(&mut self) {
        self.editing = None;
        self.input.clear();
    }

    pub fn delete_task(&mut self, task_idx: usize) {
        if task_idx >= self.tasks.len() {
            return;
        }
        self.tasks.remove(task_idx);
        self.selected = match self.selected {
            Some(sel) if sel == task_idx => None,
            Some(sel) if sel > task_idx => Some(sel - 1),
            other => other,
        };
        if let Some(EditMode::Edit(_)) = self.editing {
            self.cancel();
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShelfItem {
    pub path: String,
    pub name: String,
    pub ext: String,
    pub size_label: String,
    pub is_dir: bool,
}

impl ShelfItem {
    pub fn from_path(path: String) -> Option<Self> {
        let meta = fs::metadata(&path).ok()?;
        let is_dir = meta.is_dir();
        let p = Path::new(&path);
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| path.clone());
        let ext = if is_dir {
            "DIR".into()
        } else {
            p.extension()
                .map(|e| e.to_string_lossy().to_ascii_uppercase())
                .filter(|e| !e.is_empty())
                .map(|e| e.chars().take(4).collect())
                .unwrap_or_else(|| "FILE".into())
        };
        let size_label = if is_dir {
            "Folder".into()
        } else {
            format_bytes(meta.len())
        };
        Some(Self {
            path,
            name,
            ext,
            size_label,
            is_dir,
        })
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{} KB", bytes.div_ceil(KB))
    } else {
        format!("{bytes} B")
    }
}

#[derive(Default)]
pub struct ShelfState {
    pub items: Vec<ShelfItem>,
    pub scroll: usize,
    pub drag_over: bool,
}

impl ShelfState {
    pub fn add_paths(&mut self, paths: Vec<String>) -> bool {
        let mut changed = false;
        for path in paths {
            if self.items.len() >= 50 {
                break;
            }
            if self
                .items
                .iter()
                .any(|item| item.path.eq_ignore_ascii_case(&path))
            {
                continue;
            }
            if let Some(item) = ShelfItem::from_path(path) {
                self.items.push(item);
                changed = true;
            }
        }
        changed
    }

    pub fn remove(&mut self, idx: usize) {
        if idx < self.items.len() {
            self.items.remove(idx);
            let max_scroll = self.items.len().saturating_sub(3);
            self.scroll = self.scroll.min(max_scroll);
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.scroll = 0;
    }

    pub fn paths(&self) -> Vec<String> {
        self.items.iter().map(|i| i.path.clone()).collect()
    }

    pub fn load_from_path(&mut self, path: &Path) {
        self.items.clear();
        let Ok(raw) = fs::read_to_string(path) else {
            return;
        };
        let paths: Vec<String> = raw
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        self.add_paths(paths);
    }

    pub fn save_to_path(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let out = self
            .items
            .iter()
            .map(|i| i.path.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let _ = fs::write(path, out);
    }
}

pub struct IslandData {
    pub enabled: bool,
    pub page: Page,
    pub shelf: ShelfState,
    pub pomodoro: PomodoroState,
    pub water: WaterState,
    pub todo: TodoState,
}

impl IslandData {
    pub fn new(today: u32) -> Self {
        Self {
            enabled: true,
            page: Page::Media,
            shelf: ShelfState::default(),
            pomodoro: PomodoroState::default(),
            water: WaterState::new(today),
            todo: TodoState::default(),
        }
    }

    fn storage_path() -> Option<PathBuf> {
        let appdata = std::env::var_os("APPDATA")?;
        Some(PathBuf::from(appdata).join("MoLap").join("island.txt"))
    }

    fn shelf_path() -> Option<PathBuf> {
        let appdata = std::env::var_os("APPDATA")?;
        Some(PathBuf::from(appdata).join("MoLap").join("shelf.txt"))
    }

    pub fn load(today: u32) -> Self {
        let mut data = match Self::storage_path() {
            Some(path) => Self::load_from_path(&path, today),
            None => Self::new(today),
        };
        if let Some(sp) = Self::shelf_path() {
            data.shelf.load_from_path(&sp);
        }
        data
    }

    pub fn load_from_path(path: &Path, today: u32) -> Self {
        let mut data = Self::new(today);
        let Ok(raw) = fs::read_to_string(path) else {
            return data;
        };
        for line in raw.lines() {
            if let Some((key, val)) = line.split_once('=') {
                match key {
                    "enabled" => data.enabled = val != "0",
                    "page" => {
                        if let Ok(idx) = val.parse::<usize>() {
                            data.page = Page::from_index(idx);
                        }
                    }
                    "pomo_focus" => {
                        if let Ok(m) = val.parse::<u32>() {
                            data.pomodoro.focus_min = m.clamp(5, 120);
                            data.pomodoro.remaining_secs = data.pomodoro.phase_total_secs();
                        }
                    }
                    "water_date" => {
                        if let Ok(d) = val.parse::<u32>() {
                            data.water.date_key = d;
                        }
                    }
                    "water_consumed" => {
                        if let Ok(ml) = val.parse::<u32>() {
                            data.water.consumed_ml = ml.min(99_950);
                        }
                    }
                    "water_goal" => {
                        if let Ok(ml) = val.parse::<u32>() {
                            data.water.goal_ml = ml.clamp(500, 5000);
                        }
                    }
                    "water_cup" => {
                        if let Ok(ml) = val.parse::<u32>() {
                            data.water.cup_ml = ml.clamp(50, 1000);
                        }
                    }
                    "water_interval" => {
                        if let Ok(m) = val.parse::<u32>() {
                            data.water.interval_min = m.clamp(15, 180);
                            data.water.next_reminder_secs = data.water.interval_min * 60;
                        }
                    }
                    "water_remind" => data.water.reminder_enabled = val != "0",
                    "todo_today" => data.todo.today_only = val != "0",
                    "task" => {
                        let mut parts = val.splitn(4, '\t');
                        let Some(id) = parts.next().and_then(|s| s.parse::<u64>().ok()) else {
                            continue;
                        };
                        let Some(done) = parts.next().map(|s| s == "1") else {
                            continue;
                        };
                        let Some(date_key) = parts.next().and_then(|s| s.parse::<u32>().ok())
                        else {
                            continue;
                        };
                        let Some(title) = parts.next().map(unescape) else {
                            continue;
                        };
                        if !title.is_empty() {
                            data.todo.next_id = data.todo.next_id.max(id + 1);
                            data.todo.tasks.push(TaskItem {
                                id,
                                title,
                                done,
                                date_key,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        data.water.sync_date(today);
        data
    }

    pub fn save(&self) {
        if let Some(path) = Self::storage_path() {
            self.save_to_path(&path);
        }
        if let Some(sp) = Self::shelf_path() {
            self.shelf.save_to_path(&sp);
        }
    }

    pub fn save_shelf(&self) {
        if let Some(sp) = Self::shelf_path() {
            self.shelf.save_to_path(&sp);
        }
    }

    pub fn save_to_path(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let mut out = String::new();
        out.push_str(&format!("enabled={}\n", u8::from(self.enabled)));
        out.push_str(&format!("page={}\n", self.page.index()));
        out.push_str(&format!("pomo_focus={}\n", self.pomodoro.focus_min));
        out.push_str(&format!("water_date={}\n", self.water.date_key));
        out.push_str(&format!("water_consumed={}\n", self.water.consumed_ml));
        out.push_str(&format!("water_goal={}\n", self.water.goal_ml));
        out.push_str(&format!("water_cup={}\n", self.water.cup_ml));
        out.push_str(&format!("water_interval={}\n", self.water.interval_min));
        out.push_str(&format!(
            "water_remind={}\n",
            u8::from(self.water.reminder_enabled)
        ));
        out.push_str(&format!("todo_today={}\n", u8::from(self.todo.today_only)));
        for task in &self.todo.tasks {
            out.push_str(&format!(
                "task={}\t{}\t{}\t{}\n",
                task.id,
                u8::from(task.done),
                task.date_key,
                escape(&task.title)
            ));
        }
        let _ = fs::write(path, out);
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str(r"\\"),
            '\n' => out.push_str(r"\n"),
            '\r' => out.push_str(r"\r"),
            '\t' => out.push_str(r"\t"),
            other => out.push(other),
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_settles_to_target() {
        let mut s = Spring::new(0.0);
        s.target = 1.0;
        let mut steps = 0;
        while s.step(1.0 / 60.0, 290.0, 26.0) && steps < 600 {
            steps += 1;
        }
        assert!(steps < 300);
        assert_eq!(s.value, 1.0);
        assert_eq!(s.velocity, 0.0);
    }

    #[test]
    fn spring_substepped_handles_frame_drops_without_overshoot_explosion() {
        let mut s = Spring::new(0.0);
        s.target = 100.0;
        let mut steps = 0;
        while s.step(0.05, 480.0, 38.0) && steps < 120 {
            assert!(s.value.is_finite() && s.value >= -5.0 && s.value <= 130.0);
            steps += 1;
        }
        assert_eq!(s.value, 100.0);
        assert_eq!(s.velocity, 0.0);
    }

    #[test]
    fn pomodoro_round_total_and_transitions() {
        let mut p = PomodoroState::default();
        assert_eq!(p.round_total_minutes(), 130);
        assert_eq!(p.phase, PomodoroPhase::Focus);
        assert_eq!(p.session_index, 1);

        p.toggle();
        p.remaining_secs = 1;
        assert_eq!(p.tick_second(), Some("Short Break"));
        assert_eq!(p.phase, PomodoroPhase::ShortBreak);
        assert!(!p.running);

        p.session_index = 4;
        p.phase = PomodoroPhase::Focus;
        p.running = true;
        p.remaining_secs = 1;
        assert_eq!(p.tick_second(), Some("Long Break"));
        assert_eq!(p.phase, PomodoroPhase::LongBreak);

        p.adjust_focus_min(5);
        assert_eq!(p.focus_min, 30);
        assert_eq!(p.skip_phase(), "Focus");
        assert_eq!(p.session_index, 1);
    }

    #[test]
    fn water_drink_percent_and_daily_reset() {
        let mut w = WaterState::new(20261125);
        assert_eq!(w.percent(), 0);
        w.drink();
        assert_eq!(w.consumed_ml, 250);
        assert_eq!(w.percent(), 17);
        assert!(w.undo_drink());
        assert_eq!(w.consumed_ml, 0);
        assert!(!w.undo_drink());
        w.drink();
        w.adjust_goal(500);
        assert_eq!(w.goal_ml, 2000);
        w.sync_date(20261126);
        assert_eq!(w.consumed_ml, 0);
        assert_eq!(w.date_key, 20261126);
    }

    #[test]
    fn todo_crud_and_persistence_roundtrip() {
        let mut path = std::env::temp_dir();
        path.push("molap_island_test.txt");
        let _ = fs::remove_file(&path);

        let mut data = IslandData::new(20261125);
        data.page = Page::Water;
        data.water.drink();
        data.todo.begin_add();
        for ch in r"Code\MoLap".chars() {
            data.todo.push_char(ch);
        }
        assert!(data.todo.commit(20261125));
        assert_eq!(data.todo.top_pending_task(20261125), Some(r"Code\MoLap"));
        data.todo.toggle_done(0);
        assert_eq!(data.todo.top_pending_task(20261125), None);
        data.save_to_path(&path);

        let loaded = IslandData::load_from_path(&path, 20261125);
        let _ = fs::remove_file(&path);
        assert_eq!(loaded.page, Page::Water);
        assert_eq!(loaded.water.consumed_ml, 250);
        assert_eq!(loaded.todo.tasks.len(), 1);
        assert_eq!(loaded.todo.tasks[0].title, r"Code\MoLap");
        assert!(loaded.todo.tasks[0].done);
    }

    #[test]
    fn six_pages_and_shelf_dedup_persistence() {
        assert_eq!(Page::COUNT, 6);
        assert_eq!(Page::from_index(0), Page::System);
        assert_eq!(Page::from_index(1), Page::Media);
        assert_eq!(Page::from_index(5), Page::Todo);
        assert_eq!(Page::System.label(), "System");

        let dir = std::env::temp_dir().join("molap_shelf_test");
        let _ = fs::create_dir_all(&dir);
        let file_a = dir.join("sample.txt");
        let shelf_file = dir.join("shelf.txt");
        fs::write(&file_a, b"hello").unwrap();

        let mut shelf = ShelfState::default();
        assert!(shelf.add_paths(vec![file_a.to_string_lossy().into_owned()]));
        assert!(!shelf.add_paths(vec![file_a.to_string_lossy().into_owned()]));
        assert_eq!(shelf.items.len(), 1);
        assert_eq!(shelf.items[0].ext, "TXT");
        shelf.save_to_path(&shelf_file);

        let mut loaded = ShelfState::default();
        loaded.load_from_path(&shelf_file);
        assert_eq!(loaded.items.len(), 1);
        loaded.remove(0);
        assert!(loaded.items.is_empty());

        let _ = fs::remove_file(&file_a);
        let _ = fs::remove_file(&shelf_file);
        let _ = fs::remove_dir(&dir);
    }
}
