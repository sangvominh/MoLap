use crate::{
    sensors::{self, Control, Kind, Metric},
    win::*,
};
use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    mem::{size_of, zeroed},
    os::windows::{ffi::OsStrExt, process::CommandExt},
    process::Command,
    ptr::{null, null_mut},
    sync::{
        mpsc::{self, Receiver},
        Arc, Condvar, Mutex,
    },
    time::{Duration, Instant},
};

const TRAY: u32 = 0x8002;
const REPOSITION: u32 = 0x8003;
const INSTALL_DONE: u32 = 0x8004;
const DISPLAY_POWER: Guid = Guid {
    a: 0x6fe69556,
    b: 0x704a,
    c: 0x47a0,
    d: [0x8f, 0x24, 0xc9, 0x3d, 0x93, 0x6f, 0xda, 0x47],
};
const HELP: &str = "MoLap";
static TASKBAR: &[u16] = &[
    83, 104, 101, 108, 108, 95, 84, 114, 97, 121, 87, 110, 100, 0,
]; // Shell_TrayWnd
static NOTIFY: &[u16] = &[
    84, 114, 97, 121, 78, 111, 116, 105, 102, 121, 87, 110, 100, 0,
]; // TrayNotifyWnd

thread_local! {
    static UI: RefCell<Option<State>> = const { RefCell::new(None) };
    static RECREATE_STRIP: Cell<bool> = const { Cell::new(false) };
}

struct State {
    window: Handle,
    events: Receiver<Vec<Metric>>,
    control: Arc<(Mutex<Control>, Condvar)>,
    worker: Option<std::thread::JoinHandle<()>>,
    metrics: Vec<Metric>,
    place: Option<Rect>,
    taskbar_parent: Handle,
    attempted_parent: Handle,
    missing_tray: Option<Instant>,
    dpi: u32,
    light: bool,
    hidden: bool,
    locked: bool,
    screen_off: bool,
    tray: bool,
    opening_menu: bool,
    installing: bool,
    shell_message: u32,
    foreground_hook: Handle,
    explorer_hook: Handle,
    explorer_process: u32,
    power: Handle,
    session: Option<Library>,
    tip: Handle,
    tip_count: usize,
    tip_text: Vec<u16>,
    value_font: Handle,
    symbol_font: Handle,
}

fn state<T>(f: impl FnOnce(&mut State) -> T) -> Option<T> {
    UI.with(|ui| {
        ui.try_borrow_mut()
            .ok()
            .and_then(|mut ui| ui.as_mut().map(f))
    })
}

fn color(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
fn light_theme() -> bool {
    reg_u32(
        HKCU,
        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        "SystemUsesLightTheme",
    ) == Some(1)
}
fn driver_installed() -> bool {
    reg_u32(HKLM, r"SYSTEM\CurrentControlSet\Services\PawnIO", "Type").is_some()
}

impl State {
    fn new(window: Handle) -> Option<Self> {
        let (tx, events) = mpsc::sync_channel(1);
        let control = Arc::new((Mutex::new(Control::default()), Condvar::new()));
        let worker_control = Arc::clone(&control);
        let worker = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || sensors::worker(window, worker_control, tx))
            .ok()?;
        let mut state = Self {
            window,
            events,
            control,
            worker: Some(worker),
            metrics: Vec::new(),
            place: None,
            taskbar_parent: 0,
            attempted_parent: 0,
            missing_tray: None,
            dpi: 96,
            light: light_theme(),
            hidden: false,
            locked: false,
            screen_off: false,
            tray: false,
            opening_menu: false,
            installing: false,
            shell_message: 0,
            foreground_hook: 0,
            explorer_hook: 0,
            explorer_process: 0,
            power: 0,
            session: Library::system("wtsapi32.dll"),
            tip: 0,
            tip_count: 0,
            tip_text: Vec::new(),
            value_font: 0,
            symbol_font: 0,
        };
        unsafe {
            state.shell_message = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
            if state.shell_message != 0 {
                ChangeWindowMessageFilterEx(window, state.shell_message, 1, null_mut());
            }
            state.foreground_hook = SetWinEventHook(3, 3, 0, Some(shell_event), 0, 0, 2);
            state.power = RegisterPowerSettingNotification(window, &DISPLAY_POWER, 0);
            if let Some(register) = state
                .session
                .as_ref()
                .and_then(|dll| dll.symbol(b"WTSRegisterSessionNotification\0"))
            {
                let register: unsafe extern "system" fn(Handle, u32) -> Bool =
                    std::mem::transmute(register);
                register(window, 0);
            }
            SetTimer(window, 1, 300, 0);
            state.tip = CreateWindowExW(
                8,
                wide("tooltips_class32").as_ptr(),
                null(),
                0x80000003,
                0,
                0,
                0,
                0,
                window,
                0,
                GetModuleHandleW(null()),
                0,
            );
            if state.tip != 0 {
                SendMessageW(state.tip, 0x2005, 1, 0);
                SendMessageW(state.tip, 0x0418, 0, 420);
            }
        }
        state.refresh_fonts();
        Some(state)
    }

    fn suspend(&self) {
        let (lock, wake) = &*self.control;
        let mut control = lock.lock().unwrap();
        let pause = self.hidden || self.locked || self.screen_off;
        if control.paused != pause {
            control.paused = pause;
            unsafe {
                if pause {
                    KillTimer(self.window, 1);
                    KillTimer(self.window, 2);
                } else {
                    SetTimer(self.window, 1, 300, 0);
                }
            }
            wake.notify_one();
        }
    }

    fn scale(&self, n: i32) -> i32 {
        (n * self.dpi as i32 * 85 + 4_800) / 9_600
    }
    fn cell(&self, m: &Metric) -> i32 {
        self.scale(if m.kind == Kind::Ram { 71 } else { 51 })
    }
    fn key(&self) -> u32 {
        if self.light {
            color(248, 248, 248)
        } else {
            color(32, 32, 32)
        }
    }

    fn refresh_fonts(&mut self) {
        unsafe {
            if self.value_font != 0 {
                DeleteObject(self.value_font);
            }
            if self.symbol_font != 0 {
                DeleteObject(self.symbol_font);
            }
            self.value_font = font("Segoe UI", self.scale(12));
            self.symbol_font = font("Segoe Fluent Icons", self.scale(15));
            SetLayeredWindowAttributes(self.window, self.key(), 255, 1);
        }
    }

    fn reads_ready(&mut self) {
        if let Ok(next) = self.events.try_recv() {
            let relayout = next.len() != self.metrics.len()
                || next
                    .iter()
                    .zip(&self.metrics)
                    .any(|(a, b)| a.kind != b.kind);
            let repaint = relayout
                || next.iter().zip(&self.metrics).any(|(a, b)| {
                    a.value != b.value || a.temperature != b.temperature || a.capacity != b.capacity
                });
            self.metrics = next;
            if relayout {
                self.position();
            }
            if repaint {
                unsafe {
                    InvalidateRect(self.window, null(), 0);
                }
            }
        }
    }

    fn show_tray(&mut self, visible: bool) {
        if self.tray == visible {
            return;
        }
        unsafe {
            let mut icon: NotifyIcon = zeroed();
            icon.size = size_of::<NotifyIcon>() as u32;
            icon.window = self.window;
            icon.id = 1;
            icon.flags = 7;
            icon.message = TRAY;
            icon.icon = LoadIconW(0, 32512usize as *const u16);
            for (slot, ch) in icon.tip.iter_mut().zip("MoLap".encode_utf16()) {
                *slot = ch;
            }
            if Shell_NotifyIconW(if visible { 0 } else { 2 }, &mut icon) != 0 || !visible {
                self.tray = visible;
            }
        }
    }

    fn hide_strip(&mut self, tray: bool) {
        self.place = None;
        self.show_tray(tray);
        if self.hidden {
            unsafe {
                self.detach_taskbar();
            }
        }
        if unsafe { GetWindowLongW(self.window, -16) } & 0x10000000 != 0 {
            unsafe {
                ShowWindow(self.window, 0);
            }
        }
    }

    fn watch_explorer(&mut self, taskbar: Handle) {
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(taskbar, &mut pid);
        }
        if pid == 0 || pid == self.explorer_process {
            return;
        }
        unsafe {
            if self.explorer_hook != 0 {
                UnhookWinEvent(self.explorer_hook);
            }
            self.explorer_hook = SetWinEventHook(0x8002, 0x8004, 0, Some(shell_event), pid, 0, 2);
        }
        self.explorer_process = pid;
    }

    fn position(&mut self) {
        if self.opening_menu {
            return;
        }
        if self.hidden || self.locked || self.screen_off || self.metrics.is_empty() {
            self.hide_strip(self.hidden || self.metrics.is_empty());
            return;
        }
        unsafe {
            let taskbar = FindWindowW(TASKBAR.as_ptr(), null());
            if taskbar != 0 {
                self.watch_explorer(taskbar);
            }
            let tray = if taskbar != 0 {
                FindWindowExW(taskbar, 0, NOTIFY.as_ptr(), null())
            } else {
                0
            };
            let (mut bar, mut notify) = (Rect::default(), Rect::default());
            if taskbar == 0
                || tray == 0
                || GetWindowRect(taskbar, &mut bar) == 0
                || GetWindowRect(tray, &mut notify) == 0
                || notify.width() <= 0
            {
                let since = self.missing_tray.get_or_insert_with(Instant::now);
                if let Some(rect) = self
                    .place
                    .filter(|_| since.elapsed() < Duration::from_millis(900))
                {
                    self.place_window(rect);
                } else {
                    self.hide_strip(true);
                }
                return;
            }
            self.missing_tray = None;
            let dpi = match GetDpiForWindow(taskbar) {
                0 => 96,
                dpi => dpi,
            };
            if self.dpi != dpi {
                self.dpi = dpi;
                self.refresh_fonts();
            }
            let mut monitor = MonitorInfo {
                size: size_of::<MonitorInfo>() as u32,
                bounds: Rect::default(),
                work: Rect::default(),
                flags: 0,
            };
            if GetMonitorInfoW(MonitorFromWindow(taskbar, 2), &mut monitor) != 0 {
                bar.left = bar.left.max(monitor.bounds.left);
                bar.top = bar.top.max(monitor.bounds.top);
                bar.right = bar.right.min(monitor.bounds.right);
                bar.bottom = bar.bottom.min(monitor.bounds.bottom);
            }
            let right = bar.right.min(notify.left) - self.scale(5);
            let width = self.metrics.iter().map(|m| self.cell(m)).sum::<i32>();
            let height = self.scale(30);
            if bar.height() < height || right - width < bar.left {
                self.hide_strip(true);
                return;
            }
            self.show_tray(false);
            self.attach_taskbar(taskbar);
            let top = bar.top + (bar.height() - height) / 2;
            self.place_window(Rect {
                left: right - width,
                top,
                right,
                bottom: top + height,
            });
        }
    }

    unsafe fn detach_taskbar(&mut self) {
        if self.taskbar_parent == 0 {
            return;
        }
        SetParent(self.window, 0);
        SetWindowLongW(self.window, -16, 0x80000000u32 as i32);
        self.taskbar_parent = 0;
        self.attempted_parent = 0;
        self.place = None;
    }

    unsafe fn attach_taskbar(&mut self, taskbar: Handle) {
        if self.attempted_parent == taskbar {
            return;
        }
        self.attempted_parent = taskbar;

        // A taskbar child follows Explorer's own visibility and Z-order.
        // Some Windows configurations reject cross-process parenting.
        if self.taskbar_parent != 0 {
            SetParent(self.window, 0);
        }
        self.taskbar_parent = 0;
        self.place = None;

        const STYLE: i32 = -16;
        const CHILD: i32 = 0x44000000; // WS_CHILD | WS_CLIPSIBLINGS
        const POPUP: i32 = 0x80000000u32 as i32;
        let visibility = GetWindowLongW(self.window, STYLE) & 0x10000000;
        SetWindowLongW(self.window, STYLE, CHILD | visibility);
        SetParent(self.window, taskbar);
        if GetAncestor(self.window, 1) == taskbar {
            self.taskbar_parent = taskbar;
            if SetLayeredWindowAttributes(self.window, self.key(), 255, 1) == 0 {
                self.detach_taskbar();
                self.attempted_parent = taskbar;
                SetLayeredWindowAttributes(self.window, self.key(), 255, 1);
            }
        } else {
            SetWindowLongW(self.window, STYLE, POPUP | visibility);
        }
    }

    unsafe fn place_window(&mut self, rect: Rect) {
        let child = self.taskbar_parent != 0;
        let mut current = Rect::default();
        let same_size = self
            .place
            .is_some_and(|old| old.width() == rect.width() && old.height() == rect.height());
        let same = self.place.is_some()
            && GetWindowRect(self.window, &mut current) != 0
            && current == rect;
        let visible = GetWindowLongW(self.window, -16) & 0x10000000 != 0;
        if same && visible && !covered(self.window, rect, child) {
            return;
        }

        let mut point = Point {
            x: rect.left,
            y: rect.top,
        };
        if child && ScreenToClient(self.taskbar_parent, &mut point) == 0 {
            return;
        }
        let flags = 0x0250 | if same { 3 } else { 0 };
        if SetWindowPos(
            self.window,
            if child { 0 } else { -1 },
            point.x,
            point.y,
            rect.width(),
            rect.height(),
            flags,
        ) != 0
        {
            self.place = Some(rect);
            if !same_size {
                self.layout_tooltips(rect.height());
                InvalidateRect(self.window, null(), 0);
            }
        }
    }

    unsafe fn layout_tooltips(&mut self, height: i32) {
        if self.tip == 0 {
            return;
        }
        let mut left = 0;
        for (index, metric) in self.metrics.iter().enumerate() {
            let right = left + self.cell(metric);
            let mut tool = ToolInfo::new(self.window, index + 1);
            tool.rect = Rect {
                left,
                top: 0,
                right,
                bottom: height,
            };
            let message = if index < self.tip_count {
                0x0434
            } else {
                0x0432
            };
            SendMessageW(self.tip, message, 0, (&mut tool as *mut ToolInfo) as isize);
            left = right;
        }
        for index in self.metrics.len()..self.tip_count {
            let mut tool = ToolInfo::new(self.window, index + 1);
            SendMessageW(self.tip, 0x0433, 0, (&mut tool as *mut ToolInfo) as isize);
        }
        self.tip_count = self.metrics.len();
    }

    unsafe fn paint(&self, dc: Handle) {
        let mut rect = Rect::default();
        GetClientRect(self.window, &mut rect);
        if rect.width() <= 0 || rect.height() <= 0 {
            return;
        }
        let memory = CreateCompatibleDC(dc);
        if memory == 0 {
            self.render(dc, rect);
            return;
        }
        let image = CreateCompatibleBitmap(dc, rect.width(), rect.height());
        if image == 0 {
            DeleteDC(memory);
            self.render(dc, rect);
            return;
        }
        let old = SelectObject(memory, image);
        self.render(memory, rect);
        BitBlt(
            dc,
            0,
            0,
            rect.width(),
            rect.height(),
            memory,
            0,
            0,
            0x00cc0020,
        );
        SelectObject(memory, old);
        DeleteObject(image);
        DeleteDC(memory);
    }

    unsafe fn render(&self, dc: Handle, mut area: Rect) {
        let background = CreateSolidBrush(self.key());
        FillRect(dc, &area, background);
        DeleteObject(background);
        let ink = if self.light { 0 } else { 0xffffff };
        let brush = CreateSolidBrush(ink);
        let pen = CreatePen(0, self.scale(1).max(1), ink);
        let original_font = SelectObject(dc, self.value_font);
        let original_pen = SelectObject(dc, pen);
        let original_brush = SelectObject(dc, GetStockObject(5)); // NULL_BRUSH
        SetBkMode(dc, 1);
        let y = (area.height() - self.scale(15)) / 2;
        for m in &self.metrics {
            let size = self.cell(m);
            self.draw_icon(dc, m, area.left + self.scale(2), y, brush, ink);
            let mut text = Rect {
                left: area.left + self.scale(21),
                top: 0,
                right: area.left + size,
                bottom: if m.kind == Kind::Ram {
                    area.bottom
                } else {
                    area.bottom / 2
                },
            };
            SetTextColor(dc, ink);
            draw_text(dc, &m.value, text, 0x0824);
            if m.kind != Kind::Ram {
                text.top = text.bottom;
                text.bottom = area.bottom;
                let temp = m.temperature;
                let warm = if m.kind == Kind::Disk { 65 } else { 80 };
                let tone = match temp {
                    Some(t) if t >= warm + 10 => {
                        if self.light {
                            color(175, 25, 25)
                        } else {
                            color(255, 75, 65)
                        }
                    }
                    Some(t) if t >= warm => {
                        if self.light {
                            color(175, 76, 0)
                        } else {
                            color(255, 170, 55)
                        }
                    }
                    _ => ink,
                };
                SetTextColor(dc, tone);
                let value = temp
                    .map(|v| format!("{v}\u{b0}"))
                    .unwrap_or_else(|| "--\u{b0}".into());
                draw_text(dc, &value, text, 0x0824);
            }
            area.left += size;
        }
        SelectObject(dc, original_font);
        SelectObject(dc, original_pen);
        SelectObject(dc, original_brush);
        DeleteObject(pen);
        DeleteObject(brush);
    }

    unsafe fn draw_icon(
        &self,
        dc: Handle,
        m: &Metric,
        left: i32,
        top: i32,
        brush: Handle,
        ink: u32,
    ) {
        let (x, y) = (|v| left + self.scale(v), |v| top + self.scale(v));
        match m.kind {
            Kind::Disk => {
                Rectangle(dc, x(1), y(3), x(16), y(13));
                let right = m
                    .capacity
                    .map(|v| x(3) + (self.scale(11) as f64 * v).round() as i32);
                if let Some(right) = right {
                    FillRect(
                        dc,
                        &Rect {
                            left: x(3),
                            right,
                            top: y(5),
                            bottom: y(11),
                        },
                        brush,
                    );
                } else {
                    line(dc, x(6), y(8), x(10), y(8));
                }
            }
            Kind::Gpu => {
                RoundRect(dc, x(3), y(4), x(16), y(13), self.scale(3), self.scale(3));
                Ellipse(dc, x(6), y(6), x(12), y(12));
                FillRect(
                    dc,
                    &Rect {
                        left: x(8),
                        top: y(8),
                        right: x(10),
                        bottom: y(10),
                    },
                    brush,
                );
                line(dc, x(1), y(2), x(1), y(15));
                line(dc, x(1), y(2), x(4), y(2));
                line(dc, x(6), y(14), x(12), y(14));
            }
            kind => {
                let glyph = if kind == Kind::Cpu { 0xeea1 } else { 0xeea0 };
                let old = SelectObject(dc, self.symbol_font);
                let mut index = 0xffff;
                GetGlyphIndicesW(dc, &glyph, 1, &mut index, 1);
                if index != 0xffff {
                    SetTextColor(dc, ink);
                    let mut rect = Rect {
                        left,
                        top,
                        right: x(16),
                        bottom: y(16),
                    };
                    DrawTextW(dc, &glyph, 1, &mut rect, 0x0825);
                    SelectObject(dc, old);
                    return;
                }
                SelectObject(dc, old);
                if kind == Kind::Cpu {
                    Rectangle(dc, x(3), y(3), x(14), y(14));
                    for pin in [5, 8, 11] {
                        line(dc, x(pin), y(1), x(pin), y(3));
                        line(dc, x(pin), y(13), x(pin), y(16));
                        line(dc, x(1), y(pin), x(3), y(pin));
                        line(dc, x(13), y(pin), x(16), y(pin));
                    }
                } else {
                    Rectangle(dc, x(1), y(4), x(16), y(12));
                    Rectangle(dc, x(4), y(6), x(8), y(10));
                    Rectangle(dc, x(9), y(6), x(13), y(10));
                }
            }
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        {
            let mut control = self.control.0.lock().unwrap();
            control.quit = true;
            self.control.1.notify_one();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        unsafe {
            KillTimer(self.window, 1);
            if let Some(unregister) = self
                .session
                .as_ref()
                .and_then(|dll| dll.symbol(b"WTSUnRegisterSessionNotification\0"))
            {
                let unregister: unsafe extern "system" fn(Handle) -> Bool =
                    std::mem::transmute(unregister);
                unregister(self.window);
            }
            if self.power != 0 {
                UnregisterPowerSettingNotification(self.power);
            }
            if self.foreground_hook != 0 {
                UnhookWinEvent(self.foreground_hook);
            }
            if self.explorer_hook != 0 {
                UnhookWinEvent(self.explorer_hook);
            }
            if self.value_font != 0 {
                DeleteObject(self.value_font);
            }
            if self.symbol_font != 0 {
                DeleteObject(self.symbol_font);
            }
        }
        self.show_tray(false);
    }
}

unsafe fn font(face: &str, size: i32) -> Handle {
    CreateFontW(
        -size,
        0,
        0,
        0,
        400,
        0,
        0,
        0,
        1,
        0,
        0,
        4,
        0,
        wide(face).as_ptr(),
    )
}

unsafe fn draw_text(dc: Handle, value: &str, mut rect: Rect, flags: u32) {
    let text = wide(value);
    DrawTextW(dc, text.as_ptr(), text.len() as i32 - 1, &mut rect, flags);
}

unsafe fn line(dc: Handle, x: i32, y: i32, right: i32, bottom: i32) {
    MoveToEx(dc, x, y, null_mut());
    LineTo(dc, right, bottom);
}

unsafe fn covered(strip: Handle, rect: Rect, siblings: bool) -> bool {
    let mut window = GetWindow(strip, 3);
    for _ in 0..128 {
        if window == 0 {
            break;
        }
        let mut bounds = Rect::default();
        if (siblings || GetWindowLongW(window, -20) & 8 != 0)
            && IsWindowVisible(window) != 0
            && GetWindowRect(window, &mut bounds) != 0
            && rect.overlaps(bounds)
        {
            return true;
        }
        window = GetWindow(window, 3);
    }
    false
}

#[repr(C)]
struct ToolInfo {
    size: u32,
    flags: u32,
    window: Handle,
    id: usize,
    rect: Rect,
    module: Handle,
    text: *mut u16,
    param: isize,
    reserved: *mut c_void,
}

impl ToolInfo {
    fn new(window: Handle, id: usize) -> Self {
        Self {
            size: size_of::<Self>() as u32,
            flags: 0x10,
            window,
            id,
            rect: Rect::default(),
            module: 0,
            text: usize::MAX as *mut u16,
            param: 0,
            reserved: null_mut(),
        }
    }
}

#[repr(C)]
struct Header {
    source: Handle,
    id: usize,
    code: u32,
}
#[repr(C)]
struct ToolDisplay {
    header: Header,
    text: *mut u16,
    inline: [u16; 80],
    module: Handle,
    flags: u32,
    param: isize,
}

unsafe fn tool_text(s: &mut State, notification: isize) {
    if notification == 0 {
        return;
    }
    let header = &*(notification as *const Header);
    if header.source != s.tip || header.code != (-530i32) as u32 {
        return;
    } // TTN_GETDISPINFOW
    let data = &mut *(notification as *mut ToolDisplay);
    let text = header
        .id
        .checked_sub(1)
        .and_then(|index| s.metrics.get(index))
        .map(|m| {
            let temperature = if m.kind != Kind::Ram && m.temperature.is_none() {
                "\nTemperature unavailable"
            } else {
                ""
            };
            format!("{}{}", m.detail, temperature)
        });
    s.tip_text = wide(text.as_deref().unwrap_or(""));
    data.text = s.tip_text.as_mut_ptr();
}

fn information(window: Handle, text: &str, title: &str, flags: u32) -> i32 {
    unsafe { MessageBoxW(window, wide(text).as_ptr(), wide(title).as_ptr(), flags) }
}

fn restart_elevated(window: Handle) {
    if unsafe { IsUserAnAdmin() } != 0 {
        return;
    }
    if let Ok(path) = std::env::current_exe() {
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let result = unsafe {
            ShellExecuteW(
                window,
                wide("runas").as_ptr(),
                path.as_ptr(),
                wide("--restart").as_ptr(),
                null(),
                1,
            )
        };
        if result > 32 {
            unsafe {
                close_strip(window);
            }
        }
    }
}

fn enable_pawnio(window: Handle) {
    if driver_installed() {
        restart_elevated(window);
        return;
    }
    if information(
        window,
        "Install the official PawnIO driver for CPU and Intel UHD temperature?",
        "MoLap - Install PawnIO",
        0x0124,
    ) != 6
    {
        return;
    }
    if state(|s| {
        if s.installing {
            false
        } else {
            s.installing = true;
            true
        }
    }) != Some(true)
    {
        return;
    }
    let spawned = std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(move || {
            let status = std::env::var_os("LOCALAPPDATA")
                .map(std::path::PathBuf::from)
                .and_then(|dir| {
                    Command::new(dir.join("Microsoft/WindowsApps/winget.exe"))
                        .args([
                            "install",
                            "--id",
                            "namazso.PawnIO",
                            "--exact",
                            "--source",
                            "winget",
                            "--accept-source-agreements",
                            "--accept-package-agreements",
                        ])
                        .creation_flags(0x08000000)
                        .status()
                        .ok()
                });
            let failed = usize::from(status.is_none());
            unsafe {
                PostMessageW(window, INSTALL_DONE, failed, 0);
            }
        });
    if spawned.is_err() {
        state(|s| s.installing = false);
        information(window, "Could not start the installer.", HELP, 0x30);
    }
}

unsafe fn popup(window: Handle) {
    let Some((hidden, installing)) = state(|s| {
        s.opening_menu = true;
        (s.hidden, s.installing)
    }) else {
        return;
    };
    let menu = CreatePopupMenu();
    if menu == 0 {
        state(|s| s.opening_menu = false);
        return;
    }
    let item = |flags, id, label| AppendMenuW(menu, flags, id, wide(label).as_ptr());
    item(
        0,
        1,
        if hidden {
            "Show monitor"
        } else {
            "Hide monitor"
        },
    );
    item(0, 2, "Show readings");
    if IsUserAnAdmin() == 0 || !driver_installed() {
        item(
            if installing { 1 } else { 0 },
            3,
            if IsUserAnAdmin() != 0 {
                "Install PawnIO for temperature"
            } else {
                "Restart as administrator for temperature"
            },
        );
    }
    let start_with_windows = crate::startup::enabled();
    item(
        if start_with_windows { 8 } else { 0 },
        5,
        "Start with Windows",
    );
    item(0, 4, "Exit MoLap");
    let mut cursor = Point::default();
    GetCursorPos(&mut cursor);
    SetForegroundWindow(window);
    let command = TrackPopupMenu(menu, 0x0122, cursor.x, cursor.y, 0, window, null());
    DestroyMenu(menu);
    PostMessageW(window, 0, 0, 0);
    state(|s| s.opening_menu = false);
    match command {
        1 => {
            state(|s| {
                s.hidden = !s.hidden;
                s.suspend();
                s.position();
            });
        }
        2 => {
            if let Some(readings) = state(|s| {
                s.metrics
                    .iter()
                    .map(|m| {
                        let temp = m
                            .temperature
                            .map(|v| format!("  {v}\u{b0}C"))
                            .unwrap_or_default();
                        format!("{}  {}{}  {}", m.label, m.value, temp, m.detail)
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }) {
                information(window, &readings, HELP, 0x40);
            }
        }
        3 => enable_pawnio(window),
        5 => {
            if let Err(error) = crate::startup::set_enabled(!start_with_windows) {
                information(
                    window,
                    &format!("Could not update Windows startup: {error}"),
                    HELP,
                    0x30,
                );
            }
        }
        4 => {
            close_strip(window);
        }
        _ => {}
    }
}

unsafe extern "system" fn shell_event(
    _: Handle,
    _: u32,
    _: Handle,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    if object == 0 && child == 0 {
        state(|s| {
            if s.taskbar_parent == 0 && !s.hidden && !s.locked && !s.screen_off && !s.opening_menu {
                KillTimer(s.window, 2);
                SetTimer(s.window, 2, 35, 0);
            }
        });
    }
}

unsafe fn close_strip(window: Handle) {
    state(|s| {
        s.taskbar_parent = 0;
    });
    DestroyWindow(window);
}

unsafe extern "system" fn window_proc(window: Handle, message: u32, w: usize, l: isize) -> isize {
    match message {
        0x0002 => {
            UI.with(|ui| {
                let mut ui = ui.borrow_mut();
                RECREATE_STRIP.set(ui.as_ref().is_some_and(|s| s.taskbar_parent != 0));
                ui.take();
            });
            PostQuitMessage(0);
        }
        0x0010 => {
            close_strip(window);
        }
        0x000f => {
            let mut paint: Paint = zeroed();
            let dc = BeginPaint(window, &mut paint);
            if dc != 0 {
                state(|s| s.paint(dc));
            }
            EndPaint(window, &paint);
        }
        0x0014 => return 1,
        0x0021 => return 3, // MA_NOACTIVATE
        0x004e => {
            state(|s| tool_text(s, l));
        }
        0x0113 => {
            if w == 2 {
                KillTimer(window, 2);
            }
            state(|s| s.position());
        }
        0x0202 | 0x0205 => popup(window),
        0x0218 if w == 0x8013 && l != 0 => {
            let value = (l as *const u8).add(20).cast::<u32>().read_unaligned();
            state(|s| {
                s.screen_off = value == 0;
                s.suspend();
                s.position();
            });
            return 1;
        }
        0x02b1 if w == 7 || w == 8 => {
            state(|s| {
                s.locked = w == 7;
                s.suspend();
                s.position();
            });
        }
        0x001a | 0x031a => {
            state(|s| {
                let light = light_theme();
                if s.light != light {
                    s.light = light;
                    s.refresh_fonts();
                    InvalidateRect(window, null(), 0);
                }
                s.position();
            });
        }
        sensors::SAMPLE_READY => {
            state(State::reads_ready);
        }
        REPOSITION => {
            state(|s| s.position());
        }
        TRAY => {
            if l as u32 == 0x0202 || l as u32 == 0x0205 {
                popup(window);
            }
        }
        INSTALL_DONE => {
            state(|s| s.installing = false);
            if driver_installed() {
                restart_elevated(window);
            } else {
                information(
                    window,
                    if w != 0 {
                        "Could not start WinGet. Official installer: https://pawnio.eu/"
                    } else {
                        "PawnIO was not installed. You can retry from the menu."
                    },
                    HELP,
                    0x40,
                );
            }
        }
        _ => {
            if state(|s| message == s.shell_message) == Some(true) {
                state(|s| {
                    s.tray = false;
                    s.position();
                });
            } else {
                return DefWindowProcW(window, message, w, l);
            }
        }
    }
    0
}

pub fn run() {
    let restarting = std::env::args().skip(1).any(|a| a == "--restart");
    unsafe {
        let mutex = CreateMutexW(0, 0, wide(r"Local\MoLap.Monitor").as_ptr());
        if mutex == 0 {
            return;
        }
        let mutex = OwnedHandle(mutex);
        if !matches!(
            WaitForSingleObject(mutex.0, if restarting { 10_000 } else { 0 }),
            0 | 0x80
        ) {
            return;
        }
        SetProcessDpiAwarenessContext(-4);
        InitCommonControls();
        let class = wide("MoLapNativeStrip");
        let instance = GetModuleHandleW(null());
        let info = WindowClass {
            size: size_of::<WindowClass>() as u32,
            style: 0,
            callback: Some(window_proc),
            class_extra: 0,
            window_extra: 0,
            instance,
            icon: 0,
            cursor: LoadCursorW(0, 32512usize as *const u16),
            background: 0,
            menu: null(),
            class: class.as_ptr(),
            small_icon: 0,
        };
        if RegisterClassExW(&info) != 0 {
            loop {
                let window = CreateWindowExW(
                    0x08080088,
                    class.as_ptr(),
                    wide(HELP).as_ptr(),
                    0x80000000,
                    -32000,
                    -32000,
                    1,
                    1,
                    0,
                    0,
                    instance,
                    0,
                );
                if window != 0 {
                    if let Some(init) = State::new(window) {
                        UI.with(|ui| *ui.borrow_mut() = Some(init));
                        PostMessageW(window, REPOSITION, 0, 0);
                        let mut message = Message::default();
                        while GetMessageW(&mut message, 0, 0, 0) > 0 {
                            TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                    } else {
                        close_strip(window);
                    }
                }
                if !RECREATE_STRIP.replace(false) {
                    break;
                }
            }
        }
        ReleaseMutex(mutex.0);
    }
}
