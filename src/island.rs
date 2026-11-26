use crate::{
    gdiplus::{self, argb, mix_color, with_alpha, GdiplusApi, GpImage, RectF, Surface, TextStyle},
    island_data::{EditMode, IslandData, Page, PomodoroPhase, Spring},
    liquid_glass::LiquidNav,
    lyrics::{active_lyric, LyricsHandle, LyricsStatus, WM_LYRICS_READY},
    media::{format_mmss, MediaCmd, MediaHandle, WM_MEDIA_READY},
    plugin::{PluginEntry, PluginHost, PluginTone, WM_PLUGIN_READY},
    sensors::{Kind, Metric},
    settings::{self, IdleMode, IslandAnchor, Palette, TaskbarFit},
    shelf::{
        copy_files_to_clipboard, handle_wm_dropfiles, open_file, pick_files_dialog,
        read_files_from_clipboard, start_drag_out, take_dropped_paths, DropTargetGuard,
        WM_SHELF_DRAG_ENTER, WM_SHELF_DRAG_LEAVE, WM_SHELF_DROP,
    },
    win::*,
};
use std::{
    cell::RefCell,
    ffi::c_void,
    mem::{size_of, transmute},
    ptr::{null, null_mut},
    time::{Duration, Instant},
};

const CANVAS_W: f32 = 504.0;
const CANVAS_H: f32 = 264.0;

const COMPACT_TELEMETRY_W: f32 = 204.0;
const COMPACT_PRODUCTIVITY_W: f32 = 178.0;
const COMPACT_MINIMAL_W: f32 = 84.0;
const COMPACT_LIVE_W: f32 = 240.0;
const COMPACT_LYRICS_W: f32 = 340.0;

const EXPANDED_W: f32 = 464.0;
const EXPANDED_H: f32 = 196.0;
const EXPANDED_R: f32 = 30.0;

const TIMER_SECOND: usize = 1;
const TIMER_ANIM: usize = 2;
const TIMER_LEAVE: usize = 3;
const TIMER_ZORDER: usize = 4;
const TIMER_LYRICS: usize = 5;
const TIMER_WAVE: usize = 6;

const WM_ISLAND_CHECK_FS: u32 = 0x8011;

const GUID_DEVINTERFACE_USB_DEVICE: Guid = Guid {
    a: 0xA5DC_BF10,
    b: 0x6530,
    c: 0x11D2,
    d: [0x90, 0x1F, 0x00, 0xC0, 0x4F, 0xB9, 0x51, 0xED],
};
const GUID_DEVINTERFACE_MOUSE: Guid = Guid {
    a: 0x378D_E44C,
    b: 0x56EF,
    c: 0x11D1,
    d: [0xBC, 0x8C, 0x00, 0xA0, 0xC9, 0x14, 0x05, 0xDD],
};
const GUID_DEVINTERFACE_KEYBOARD: Guid = Guid {
    a: 0x884B_96C3,
    b: 0x56EF,
    c: 0x11D1,
    d: [0xBC, 0x8C, 0x00, 0xA0, 0xC9, 0x14, 0x05, 0xDD],
};

#[repr(C)]
struct DevBroadcastHdr {
    size: u32,
    dev_type: u32,
    reserved: u32,
}

#[repr(C)]
struct DevBroadcastDeviceInterface {
    size: u32,
    dev_type: u32,
    reserved: u32,
    class_guid: Guid,
    name: [u16; 1],
}

#[repr(C)]
struct DevBroadcastVolume {
    size: u32,
    dev_type: u32,
    reserved: u32,
    unit_mask: u32,
    flags: u16,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ToastKind {
    Charging,
    Unplugged,
    BatterySaverOn,
    BatterySaverOff,
    LowBattery,
    AudioConnected { headphone: bool },
    MouseConnected,
    MouseRemoved,
    KeyboardConnected,
    KeyboardRemoved,
    UsbConnected,
    UsbRemoved,
    DriveConnected,
    DriveRemoved,
}

struct ActiveToast {
    kind: ToastKind,
    title: String,
    subtitle: String,
    pct: u8,
    until: Instant,
}

struct HiResTimer {
    _lib: Library,
    begin: unsafe extern "system" fn(u32) -> u32,
    end: unsafe extern "system" fn(u32) -> u32,
    active: bool,
}

impl HiResTimer {
    fn load() -> Option<Self> {
        let lib = Library::system("winmm.dll")?;
        unsafe {
            Some(Self {
                begin: transmute::<*const c_void, unsafe extern "system" fn(u32) -> u32>(
                    lib.symbol(b"timeBeginPeriod\0")?,
                ),
                end: transmute::<*const c_void, unsafe extern "system" fn(u32) -> u32>(
                    lib.symbol(b"timeEndPeriod\0")?,
                ),
                _lib: lib,
                active: false,
            })
        }
    }

    fn set_active(&mut self, enable: bool) {
        if self.active != enable {
            self.active = enable;
            unsafe {
                if enable {
                    (self.begin)(1);
                } else {
                    (self.end)(1);
                }
            }
        }
    }
}

impl Drop for HiResTimer {
    fn drop(&mut self) {
        self.set_active(false);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NavSlot {
    Page(Page),
    Plugin(usize),
}

impl NavSlot {
    fn label<'b>(&self, plugins: &'b [PluginEntry]) -> &'b str {
        match *self {
            NavSlot::Page(p) => p.label(),
            NavSlot::Plugin(idx) => plugins
                .get(idx)
                .map(|e| e.manifest.name.as_str())
                .unwrap_or("Plugin"),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Expand,
    ExpandTo(Page),
    ExpandToPlugin(usize),
    PrevPage,
    NextPage,
    SelectSlot(usize),
    EnableTempSensor,
    PluginRefresh(usize),
    PluginTrigger(usize),
    MediaCopyTrack,
    MediaToggleLyrics,
    MediaPrev,
    MediaToggle,
    MediaNext,
    MediaSeek(RectF),
    MediaToggleVol,
    VolMute,
    VolSlider(RectF),
    ShelfAddFiles,
    ShelfPaste,
    ShelfDragAll,
    ShelfClear,
    ShelfItem(usize),
    ShelfCopy(usize),
    ShelfDelete(usize),
    PomoToggle,
    PomoReset,
    PomoSkip,
    PomoAdjustMin(i32),
    WaterDrink,
    WaterUndo,
    WaterToggleRemind,
    WaterToggleSettings,
    WaterGoal(i32),
    WaterCup(i32),
    WaterInterval(i32),
    TodoFilterToday,
    TodoAdd,
    TodoEdit,
    TodoEditAt(usize),
    TodoToggle(usize),
    TodoSelect(usize),
    TodoDelete(usize),
}

#[derive(Clone, Copy)]
enum ShelfDrag {
    Single(usize),
    All,
}

#[derive(Clone, Copy)]
enum ActiveSlider {
    Seek(RectF),
    Volume(RectF),
    DockNav,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClickEffect {
    None,
    PickFiles,
    ToggleLyrics,
    EnableTempSensor,
}

struct Island<'a> {
    hwnd: Handle,
    api: &'a GdiplusApi,
    surface: Option<Surface<'a>>,
    origin: Point,
    dpi: u32,
    card_top: f32,
    compact_h: f32,
    data: IslandData,
    active_slot: NavSlot,
    plugin_host: PluginHost,
    plugin_entries: Vec<PluginEntry>,
    media: MediaHandle,
    lyrics: LyricsHandle,
    lyrics_timer: bool,
    wave_timer: bool,
    last_lyric_idx: Option<usize>,
    metrics: Vec<Metric>,
    cpu_hist: [f32; 28],
    _drop_target: DropTargetGuard,
    dev_notify: Handle,
    hires: Option<HiResTimer>,
    art_id: u64,
    art: Option<GpImage<'a>>,
    art_tint: Option<(u8, u8, u8)>,
    wave: [f32; 5],
    wave_phase: f32,
    toast: Option<ActiveToast>,
    toast_pulse: f32,
    last_dev_toast: Option<Instant>,
    last_power: Option<(u8, bool, bool)>,
    last_audio_id: String,
    volume_open: bool,
    last_vol: f32,
    last_muted: bool,
    vol_initialized: bool,
    vol_hud_until: Option<Instant>,
    copied_until: Option<Instant>,
    track_copied_until: Option<Instant>,
    active_slider: Option<ActiveSlider>,
    drag_press: Option<(ShelfDrag, f32, f32)>,
    dragging_out: bool,
    expand: Spring,
    compact_w: Spring,
    slide: Spring,
    dock_nav: LiquidNav,
    dock_bounds: RectF,
    press_spring: Spring,
    last_frame: Instant,
    animating: bool,
    watch_timer: bool,
    outside_since: Option<Instant>,
    pinned: bool,
    suspended: bool,
    fullscreen_hidden: bool,
    shell_msg: u32,
    fs_hook: Handle,
    move_hook: Handle,
    explorer_hook: Handle,
    explorer_pid: u32,
    cached_time: SystemTime,
    caret_tick: u8,
    hovered: Option<Action>,
    prev_hovered: Option<Action>,
    hover_t: f32,
    pressed: Option<Action>,
    actions: Vec<(RectF, Action)>,
    hand_cursor: Handle,
}

thread_local! {
    static ISLAND: RefCell<Option<Island<'static>>> = const { RefCell::new(None) };
}

fn with_island<T>(f: impl FnOnce(&mut Island<'static>) -> T) -> Option<T> {
    ISLAND.with(|cell| {
        cell.try_borrow_mut()
            .ok()
            .and_then(|mut opt| opt.as_mut().map(f))
    })
}

fn local_time() -> SystemTime {
    let mut st = SystemTime::default();
    unsafe {
        GetLocalTime(&mut st);
    }
    st
}

fn date_key(st: &SystemTime) -> u32 {
    st.year as u32 * 10_000 + st.month as u32 * 100 + st.day as u32
}

fn battery_power_info() -> (u8, bool, bool, Option<u32>) {
    let mut ps = SystemPowerStatus::default();
    if unsafe { GetSystemPowerStatus(&mut ps) } != 0 {
        let pct = if ps.battery_percent <= 100 {
            ps.battery_percent
        } else {
            100
        };
        let charging = ps.ac_line == 1 || (ps.battery_flag & 8) != 0;
        let saver = ps.system_status != 0;
        let life = (ps.battery_life_time != u32::MAX && ps.battery_life_time >= 60)
            .then_some(ps.battery_life_time);
        (pct, charging, saver, life)
    } else {
        (100, true, false, None)
    }
}

fn format_battery_remaining(secs: u32) -> String {
    let total_mins = secs / 60;
    let h = total_mins / 60;
    let m = total_mins % 60;
    if h > 0 {
        format!("{h}h {m:02}m")
    } else {
        format!("{m}m")
    }
}

fn drive_free_label(letter: char) -> Option<String> {
    if !letter.is_ascii_alphabetic() {
        return None;
    }
    let path = [letter as u16, b':' as u16, b'\\' as u16, 0];
    let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
    if unsafe { GetDiskFreeSpaceExW(path.as_ptr(), &mut avail, &mut total, &mut free) } != 0
        && total > 0
    {
        let gb = avail as f64 / 1_073_741_824.0;
        if gb >= 1.0 {
            Some(format!("{gb:.1} GB"))
        } else {
            let mb = avail as f64 / 1_048_576.0;
            Some(format!("{mb:.0} MB"))
        }
    } else {
        None
    }
}

fn clean_audio_name(raw: &str) -> String {
    let s = raw.trim();
    if let (Some(open), Some(close)) = (s.find('('), s.rfind(')')) {
        if close > open + 1 {
            let inner = &s[open + 1..close];
            let cleaned = inner
                .trim_start_matches(|c: char| c.is_ascii_digit() || c == '-' || c == ' ')
                .replace("(R)", "")
                .trim()
                .to_string();
            if !cleaned.is_empty() {
                return cleaned;
            }
        }
    }
    s.replace("(R)", "").trim().to_string()
}

fn short_gpu_name(raw: &str) -> String {
    let mut s = raw
        .replace("(R)", "")
        .replace("(TM)", "")
        .replace("NVIDIA GeForce ", "")
        .replace("AMD Radeon ", "")
        .replace("Intel UHD Graphics", "Intel UHD")
        .replace("Intel Iris Xe Graphics", "Iris Xe")
        .replace("Graphics", "");
    s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.is_empty() {
        "GPU".into()
    } else {
        s
    }
}

fn copy_text_to_clipboard(hwnd: Handle, text: &str) -> bool {
    let utf16: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    let bytes = utf16.len() * 2;
    unsafe {
        if OpenClipboard(hwnd) == 0 {
            return false;
        }
        EmptyClipboard();
        let mem = GlobalAlloc(0x0042, bytes);
        if mem == 0 {
            CloseClipboard();
            return false;
        }
        let ptr = GlobalLock(mem) as *mut u16;
        if ptr.is_null() {
            GlobalFree(mem);
            CloseClipboard();
            return false;
        }
        std::ptr::copy_nonoverlapping(utf16.as_ptr(), ptr, utf16.len());
        GlobalUnlock(mem);
        if SetClipboardData(13, mem) == 0 {
            GlobalFree(mem);
            CloseClipboard();
            return false;
        }
        CloseClipboard();
        true
    }
}

fn primary_monitor_geometry() -> (Rect, Rect, u32) {
    unsafe {
        let mut info = MonitorInfo {
            size: size_of::<MonitorInfo>() as u32,
            bounds: Rect::default(),
            work: Rect::default(),
            flags: 0,
        };
        let monitor = MonitorFromWindow(0, 1);
        if monitor != 0 && GetMonitorInfoW(monitor, &mut info) != 0 {
            let taskbar = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
            let dpi = if taskbar != 0 {
                match GetDpiForWindow(taskbar) {
                    0 => 96,
                    d => d,
                }
            } else {
                96
            };
            return (info.bounds, info.work, dpi);
        }
    }
    let fallback = Rect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1080,
    };
    (fallback, fallback, 96)
}

fn effective_dpi(base_dpi: u32, scale_pct: u8) -> u32 {
    let pct = (scale_pct as u32).clamp(80, 140);
    ((base_dpi.max(96) * pct + 50) / 100).max(72)
}

fn compute_origin(
    bounds: Rect,
    work: Rect,
    scale: f32,
    px_w: i32,
    px_h: i32,
    cfg: &settings::AppSettings,
) -> Point {
    let area_left = work.left.max(bounds.left);
    let area_right = work.right.min(bounds.right);
    let area_w = (area_right - area_left).max(bounds.width());
    let base_x = match cfg.island_anchor {
        IslandAnchor::Center => area_left + (area_w - px_w) / 2,
        IslandAnchor::Left => area_left,
        IslandAnchor::Right => area_right - px_w,
    };
    let ox_px = ((cfg.island_offset_x as f32) * scale).round() as i32;
    let oy_px = ((cfg.island_offset_y as f32) * scale).round() as i32;
    let max_x = (area_right - px_w).max(area_left);
    let min_y = bounds.top - (8.0 * scale).round() as i32;
    let max_y = (work.bottom - px_h).max(bounds.top);
    Point {
        x: (base_x + ox_px).clamp(area_left, max_x),
        y: (bounds.top + oy_px).clamp(min_y, max_y),
    }
}

fn detect_taskbar_fit(bounds: Rect, dpi: u32, fit: TaskbarFit) -> (f32, f32) {
    match fit {
        TaskbarFit::Small => return (3.0, 26.0),
        TaskbarFit::Standard => return (8.0, 32.0),
        TaskbarFit::Auto => {}
    }
    unsafe {
        let taskbar = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
        if taskbar != 0 {
            let mut bar = Rect::default();
            if GetWindowRect(taskbar, &mut bar) != 0 {
                let is_top = bar.top <= bounds.top + 8
                    && bar.height() > 12
                    && bar.height() < bounds.height() / 3;
                if is_top {
                    let scale = (dpi.max(96) as f32) / 96.0;
                    let bar_h = (bar.height() as f32) / scale;
                    let compact_h = (bar_h - 6.0).clamp(22.0, 30.0);
                    let card_top = ((bar_h - compact_h) * 0.5).clamp(2.0, 8.0);
                    return (card_top, compact_h);
                }
            }
        }
    }
    (8.0, 32.0)
}

fn is_foreground_fullscreen(self_hwnd: Handle) -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg == 0 || fg == self_hwnd || IsIconic(fg) != 0 || IsWindowVisible(fg) == 0 {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(fg, &mut pid);
        let mut self_pid = 0u32;
        GetWindowThreadProcessId(self_hwnd, &mut self_pid);
        if pid != 0 && pid == self_pid {
            return false;
        }
        let mut class_buf = [0u16; 64];
        let len = GetClassNameW(fg, class_buf.as_mut_ptr(), class_buf.len() as i32);
        if len > 0 {
            let class_name = String::from_utf16_lossy(&class_buf[..len as usize]);
            if matches!(
                class_name.as_str(),
                "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
            ) {
                return false;
            }
        }
        let (mon_rect, _, _) = primary_monitor_geometry();
        let mut fg_rect = Rect::default();
        if GetWindowRect(fg, &mut fg_rect) == 0 {
            return false;
        }
        fg_rect.left <= mon_rect.left
            && fg_rect.top <= mon_rect.top
            && fg_rect.right >= mon_rect.right
            && fg_rect.bottom >= mon_rect.bottom
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn parse_pct(s: &str) -> Option<f32> {
    s.trim()
        .strip_suffix('%')
        .and_then(|v| v.trim().parse::<f32>().ok())
        .map(|v| v.clamp(0.0, 100.0))
}

fn thermal_color(temp: Option<i32>, warm: i32, pal: &Palette) -> u32 {
    match temp {
        Some(t) if t >= warm + 10 => pal.accent_red,
        Some(t) if t >= warm => pal.accent_amber,
        Some(_) => pal.ink_soft,
        None => pal.muted,
    }
}

fn format_ram_summary(m: &Metric) -> String {
    if let Some(after) = m.detail.strip_prefix("Physical memory:") {
        if let Some((ratio, _)) = after.split_once('(') {
            let trimmed = ratio.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    m.value.clone()
}

impl<'a> Island<'a> {
    fn new(hwnd: Handle, api: &'a GdiplusApi) -> Self {
        let st = local_time();
        let today = date_key(&st);
        let cfg = settings::get();
        let (bounds, work, base_dpi) = primary_monitor_geometry();
        let (card_top, compact_h) = detect_taskbar_fit(bounds, base_dpi, cfg.taskbar_fit);
        let dpi = effective_dpi(base_dpi, cfg.island_scale_pct);
        let scale = (dpi as f32) / 96.0;
        let px_w = (CANVAS_W * scale).round() as i32;
        let px_h = (CANVAS_H * scale).round() as i32;
        let origin = compute_origin(bounds, work, scale, px_w, px_h, &cfg);
        let mut data = IslandData::load(today);
        data.enabled = cfg.island_enabled;
        let pages = cfg.enabled_pages();
        if !pages.contains(&data.page) {
            data.page = pages[0];
        }
        let initial_slide = pages.iter().position(|&p| p == data.page).unwrap_or(0) as f32;
        let initial_compact_w = match cfg.idle_mode {
            IdleMode::Smart => COMPACT_MINIMAL_W,
            IdleMode::Telemetry => COMPACT_TELEMETRY_W,
            IdleMode::Productivity => COMPACT_PRODUCTIVITY_W,
            IdleMode::Minimal => COMPACT_MINIMAL_W,
        };

        let mut surface = Surface::new(api, px_w, px_h, dpi);
        if let Some(ref mut s) = surface {
            s.scale = scale;
        }
        let hand_cursor = unsafe { LoadCursorW(0, 32649usize as *const u16) };
        let shell_msg = unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) };
        if shell_msg != 0 {
            unsafe {
                ChangeWindowMessageFilterEx(hwnd, shell_msg, 1, null_mut());
            }
        }
        let fs_hook = unsafe { SetWinEventHook(3, 3, 0, Some(fs_event), 0, 0, 2) };
        let move_hook = unsafe { SetWinEventHook(0x000B, 0x000B, 0, Some(fs_event), 0, 0, 2) };
        let media = MediaHandle::start(hwnd);
        let lyrics = LyricsHandle::start(hwnd);
        let plugin_host = PluginHost::start(hwnd, cfg.enabled_plugins.clone());
        let plugin_entries = plugin_host.entries();
        let active_slot = NavSlot::Page(data.page);
        let drop_target = DropTargetGuard::register(hwnd);
        let dev_filter = DevBroadcastDeviceInterface {
            size: size_of::<DevBroadcastDeviceInterface>() as u32,
            dev_type: 5,
            reserved: 0,
            class_guid: Guid::default(),
            name: [0],
        };
        let dev_notify = unsafe {
            RegisterDeviceNotificationW(
                hwnd,
                (&dev_filter as *const DevBroadcastDeviceInterface).cast(),
                0x0000_0004,
            )
        };
        let (pct, chg, saver, _) = battery_power_info();
        let initial_power = (pct, chg, saver);

        let mut island = Self {
            hwnd,
            api,
            surface,
            origin,
            dpi,
            card_top,
            compact_h,
            data,
            active_slot,
            plugin_host,
            plugin_entries,
            media,
            lyrics,
            lyrics_timer: false,
            wave_timer: false,
            last_lyric_idx: None,
            metrics: Vec::new(),
            cpu_hist: [0.0; 28],
            _drop_target: drop_target,
            dev_notify,
            hires: HiResTimer::load(),
            art_id: 0,
            art: None,
            art_tint: None,
            wave: [0.18; 5],
            wave_phase: 0.0,
            toast: None,
            toast_pulse: 0.0,
            last_dev_toast: None,
            last_power: Some(initial_power),
            last_audio_id: String::new(),
            volume_open: false,
            last_vol: 0.5,
            last_muted: false,
            vol_initialized: false,
            vol_hud_until: None,
            copied_until: None,
            track_copied_until: None,
            active_slider: None,
            drag_press: None,
            dragging_out: false,
            expand: Spring::new(0.0),
            compact_w: Spring::new(initial_compact_w),
            slide: Spring::new(initial_slide),
            dock_nav: LiquidNav::new(&[], 24.0, 14.0, 0),
            dock_bounds: RectF::default(),
            press_spring: Spring::new(0.0),
            last_frame: Instant::now(),
            animating: false,
            watch_timer: false,
            outside_since: None,
            pinned: false,
            suspended: false,
            fullscreen_hidden: false,
            shell_msg,
            fs_hook,
            move_hook,
            explorer_hook: 0,
            explorer_pid: 0,
            cached_time: st,
            caret_tick: 0,
            hovered: None,
            prev_hovered: None,
            hover_t: 1.0,
            pressed: None,
            actions: Vec::new(),
            hand_cursor,
        };
        island.watch_explorer();
        island.sync_visibility();
        island
    }

    fn scale(&self) -> f32 {
        (self.dpi.max(72) as f32) / 96.0
    }

    fn refresh_clock(&mut self) {
        self.cached_time = local_time();
    }

    fn watch_explorer(&mut self) {
        unsafe {
            let taskbar = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
            if taskbar == 0 {
                return;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(taskbar, &mut pid);
            if pid == 0 || pid == self.explorer_pid {
                return;
            }
            if self.explorer_hook != 0 {
                UnhookWinEvent(self.explorer_hook);
            }
            self.explorer_hook = SetWinEventHook(0x8002, 0x8004, 0, Some(fs_event), pid, 0, 2);
            self.explorer_pid = pid;
        }
    }

    fn ensure_topmost(&self) {
        if !self.should_show() {
            return;
        }
        unsafe {
            let mut prev = GetWindow(self.hwnd, 3);
            let mut covered = false;
            for _ in 0..32 {
                if prev == 0 {
                    break;
                }
                if IsWindowVisible(prev) != 0 && (GetWindowLongW(prev, -20) & 8) != 0 {
                    covered = true;
                    break;
                }
                prev = GetWindow(prev, 3);
            }
            if covered || (GetWindowLongW(self.hwnd, -20) & 8) == 0 {
                SetWindowPos(self.hwnd, -1, 0, 0, 0, 0, 0x0213);
            }
        }
    }

    fn vol_hud_active(&self) -> bool {
        settings::get().volume_hud_enabled && self.vol_hud_until.is_some_and(|t| Instant::now() < t)
    }

    fn toast_active(&self) -> bool {
        self.toast
            .as_ref()
            .is_some_and(|t| Instant::now() < t.until)
    }

    fn copied_active(&self) -> bool {
        self.copied_until.is_some_and(|t| Instant::now() < t)
    }

    fn track_copied_active(&self) -> bool {
        self.track_copied_until.is_some_and(|t| Instant::now() < t)
    }

    fn measure_pill_text(&self, text: &str, size: f32) -> f32 {
        if let Some(surf) = self.surface.as_ref() {
            surf.measure_text_width(text, size, true)
        } else {
            (text.chars().count() as f32) * size * 0.56
        }
    }

    fn telemetry_text(&self) -> String {
        let cfg = settings::get();
        let cpu = self.metrics.iter().find(|m| m.kind == Kind::Cpu);
        let ram = self.metrics.iter().find(|m| m.kind == Kind::Ram);
        let cpu_temp = cpu.and_then(|c| c.temperature);
        let cpu_txt = cpu
            .map(|m| format!("CPU {}", m.value))
            .unwrap_or_else(|| "CPU --".into());
        let ram_txt = ram.map(|m| m.value.as_str()).unwrap_or("-- GB");
        if let Some(t) = cpu_temp {
            format!(
                "{cpu_txt} · {ram_txt} · {}",
                settings::format_temp(t, cfg.temp_fahrenheit)
            )
        } else {
            format!("{cpu_txt} · {ram_txt}")
        }
    }

    fn cpu_hog(&self) -> Option<(&str, u32, Option<i32>, &str)> {
        let cpu = self.metrics.iter().find(|m| m.kind == Kind::Cpu)?;
        let (name, pct) = cpu.hog.as_ref()?;
        Some((name.as_str(), *pct, cpu.temperature, cpu.value.as_str()))
    }

    fn show_cpu_hog_banner(&self) -> bool {
        if !settings::get().cpu_hog_alert {
            return false;
        }
        let Some((_, _, temp, _)) = self.cpu_hog() else {
            return false;
        };
        !self.media.snapshot.playing || temp.is_some_and(|t| t >= 85)
    }

    fn abnormal_system_alert(&self) -> Option<(String, bool)> {
        let fahr = settings::get().temp_fahrenheit;
        let mut best: Option<(u8, String, bool)> = None;
        for m in &self.metrics {
            let warm = if m.kind == Kind::Disk { 65 } else { 80 };
            if let Some(t) = m.temperature {
                if t >= warm + 10 {
                    let msg = format!("{} {} · Hot", m.label, settings::format_temp(t, fahr));
                    return Some((msg, true));
                } else if t >= warm && best.as_ref().is_none_or(|b| b.0 < 2) {
                    best = Some((
                        2,
                        format!("{} {} · Warm", m.label, settings::format_temp(t, fahr)),
                        false,
                    ));
                }
            }
            if m.kind == Kind::Ram {
                if let Some(cap) = m.capacity {
                    let pct = (cap * 100.0).round() as u32;
                    if pct >= 90 && best.as_ref().is_none_or(|b| b.0 < 3) {
                        best = Some((3, format!("RAM {pct}% · High"), pct >= 95));
                    }
                }
            }
            if m.kind == Kind::Cpu {
                if let Some(pct) = parse_pct(&m.value) {
                    if pct >= 90.0 && best.as_ref().is_none_or(|b| b.0 < 1) {
                        best = Some((1, format!("CPU {:.0}% · High", pct), false));
                    }
                }
            }
        }
        best.map(|(_, msg, crit)| (msg, crit))
    }

    fn current_synced_lyric(&self) -> Option<(usize, &str, Option<&str>)> {
        let cfg = settings::get();
        if !cfg.lyrics_enabled || !self.media.snapshot.has_session {
            return None;
        }
        if let LyricsStatus::Synced(lines) = self.lyrics.status() {
            let (pos_ms, _, _) = self.media.snapshot.timeline_ms();
            active_lyric(lines, pos_ms)
        } else {
            None
        }
    }

    fn pill_showing_lyric(&self) -> bool {
        let cfg = settings::get();
        cfg.lyrics_enabled
            && cfg.lyrics_in_pill
            && self.media.snapshot.playing
            && self
                .current_synced_lyric()
                .is_some_and(|(_, cur, _)| !cur.is_empty())
    }

    fn target_compact_width(&self) -> f32 {
        let cfg = settings::get();
        let small = self.compact_h < 28.0;
        let font_sz = if small { 10.4 } else { 11.1 };

        if let Some(toast) = self.toast.as_ref().filter(|t| Instant::now() < t.until) {
            let tw = self.measure_pill_text(&toast.title, if small { 10.5 } else { 11.2 });
            return match toast.kind {
                ToastKind::Charging
                | ToastKind::Unplugged
                | ToastKind::BatterySaverOn
                | ToastKind::BatterySaverOff
                | ToastKind::LowBattery => (40.0 + tw + 14.0).clamp(132.0, 236.0),
                ToastKind::AudioConnected { .. } => (38.0 + tw + 34.0).clamp(148.0, 260.0),
                ToastKind::DriveConnected | ToastKind::DriveRemoved => {
                    if toast.subtitle.is_empty() {
                        (34.0 + tw + 16.0).clamp(108.0, 180.0)
                    } else {
                        let sw = self
                            .measure_pill_text(&toast.subtitle, if small { 10.0 } else { 10.6 });
                        (34.0 + tw + 12.0 + sw + 14.0).clamp(124.0, 224.0)
                    }
                }
                ToastKind::MouseConnected
                | ToastKind::MouseRemoved
                | ToastKind::KeyboardConnected
                | ToastKind::KeyboardRemoved
                | ToastKind::UsbConnected
                | ToastKind::UsbRemoved => (34.0 + tw + 22.0).clamp(112.0, 210.0),
            };
        }
        if self.vol_hud_active() {
            return 222.0;
        }
        if self.data.pomodoro.running {
            let lw = self.measure_pill_text(
                self.data.pomodoro.phase.label(),
                if small { 10.5 } else { 11.4 },
            );
            return (28.0 + lw + 68.0 + (self.compact_h - 12.0).clamp(13.0, 16.0) + 12.0)
                .clamp(176.0, COMPACT_LIVE_W);
        }
        if self.data.pomodoro.alert_secs > 0 {
            let lw = self.measure_pill_text("Done", 10.8);
            let rw = self.measure_pill_text(self.data.pomodoro.phase.label(), 10.8);
            return (28.0 + lw + 12.0 + rw + 14.0).clamp(156.0, 232.0);
        }
        if self.data.water.due_alert {
            let lw = self.measure_pill_text("Water", 11.0);
            let rw = self.measure_pill_text(&format!("+{} ml", self.data.water.cup_ml), 11.0);
            return (28.0 + lw + 12.0 + rw + 14.0).clamp(158.0, 224.0);
        }
        if self.show_cpu_hog_banner() {
            if let Some((name, pct, temp, val)) = self.cpu_hog() {
                let left_txt = if let Some(t) = temp {
                    format!(
                        "CPU {} · {name}",
                        settings::format_temp(t, cfg.temp_fahrenheit)
                    )
                } else {
                    format!("CPU {val} · {name}")
                };
                let right_txt = format!("{pct}%");
                let lw = self.measure_pill_text(&left_txt, font_sz);
                let rw = self.measure_pill_text(&right_txt, font_sz);
                return (26.0 + lw + 10.0 + rw + 14.0).clamp(160.0, 316.0);
            }
        }
        if let Some((_, entry)) = self.active_plugin_alert() {
            let pw = self.measure_pill_text(&entry.data.pill, font_sz);
            return (26.0 + pw + 14.0).clamp(124.0, 260.0);
        }
        if self.media.snapshot.playing {
            let art_sz = (self.compact_h - 8.0).clamp(16.0, 20.0);
            if self.pill_showing_lyric() {
                let lw = self
                    .current_synced_lyric()
                    .map(|(_, line, _)| {
                        self.measure_pill_text(line, if small { 11.0 } else { 11.5 })
                    })
                    .unwrap_or(140.0);
                return (art_sz + 58.0 + lw).clamp(180.0, COMPACT_LYRICS_W);
            } else {
                let title = if self.media.snapshot.title.is_empty() {
                    "Playing"
                } else {
                    &self.media.snapshot.title
                };
                let tw = self.measure_pill_text(title, if small { 10.8 } else { 11.5 });
                return (art_sz + 58.0 + tw).clamp(166.0, COMPACT_LIVE_W);
            }
        }

        let privacy_active =
            cfg.privacy_dots && (self.media.snapshot.mic_active || self.media.snapshot.cam_active);
        let right_pad = if privacy_active { 25.0 } else { 13.0 };
        let today = date_key(&self.cached_time);
        match cfg.idle_mode {
            IdleMode::Smart => {
                if privacy_active {
                    let lbl = if self.media.snapshot.cam_active && self.media.snapshot.mic_active {
                        "Camera & Mic Active"
                    } else if self.media.snapshot.cam_active {
                        "Camera Active"
                    } else {
                        "Mic Active"
                    };
                    (25.0 + self.measure_pill_text(lbl, font_sz) + 14.0).clamp(124.0, 220.0)
                } else if let Some((msg, _)) = self.abnormal_system_alert() {
                    (25.0 + self.measure_pill_text(&msg, font_sz) + 14.0).clamp(138.0, 236.0)
                } else if let Some(task) = cfg
                    .page_todo
                    .then(|| self.data.todo.top_pending_task(today))
                    .flatten()
                {
                    (26.0 + self.measure_pill_text(task, font_sz) + 14.0).clamp(132.0, 252.0)
                } else if cfg.page_water
                    && self.data.water.reminder_enabled
                    && self.data.water.consumed_ml < self.data.water.goal_ml
                    && self.data.water.next_reminder_secs <= 600
                {
                    let mins = self.data.water.next_reminder_secs.div_ceil(60);
                    let txt = format!("Water in {mins}m");
                    (25.0 + self.measure_pill_text(&txt, font_sz) + 14.0).clamp(126.0, 210.0)
                } else {
                    let txt = self.telemetry_text();
                    (25.0 + self.measure_pill_text(&txt, font_sz) + right_pad).clamp(124.0, 248.0)
                }
            }
            IdleMode::Telemetry => {
                let txt = self.telemetry_text();
                (25.0 + self.measure_pill_text(&txt, font_sz) + right_pad).clamp(124.0, 248.0)
            }
            IdleMode::Productivity => {
                let (done, total) = self.data.todo.counts(today);
                let txt_l = format!("{done}/{total}");
                let txt_r = format!("{}%", self.data.water.percent());
                let lw = self.measure_pill_text(&txt_l, font_sz);
                let rw = self.measure_pill_text(&txt_r, font_sz);
                (24.0 + lw + 22.0 + rw + right_pad).clamp(120.0, 196.0)
            }
            IdleMode::Minimal => {
                if privacy_active {
                    108.0
                } else {
                    COMPACT_MINIMAL_W
                }
            }
        }
    }

    fn update_compact_width(&mut self) {
        let target = self.target_compact_width();
        if (self.compact_w.target - target).abs() > 0.5 {
            self.compact_w.target = target;
            self.start_anim();
        }
    }

    fn sync_lyrics_timer(&mut self) {
        let cfg = settings::get();
        let need = self.should_show()
            && self.media.snapshot.playing
            && cfg.lyrics_enabled
            && matches!(self.lyrics.status(), LyricsStatus::Synced(_));
        if self.lyrics_timer != need {
            self.lyrics_timer = need;
            unsafe {
                if need {
                    SetTimer(self.hwnd, TIMER_LYRICS, 120, 0);
                } else {
                    KillTimer(self.hwnd, TIMER_LYRICS);
                }
            }
        }
    }

    fn sync_wave_timer(&mut self) {
        let playing_visible = self.media.snapshot.playing
            && (self.expand.value < 0.35 || self.active_slot == NavSlot::Page(Page::Media));
        let need = self.should_show() && (playing_visible || self.toast_active());
        if self.wave_timer != need {
            self.wave_timer = need;
            unsafe {
                if need {
                    SetTimer(self.hwnd, TIMER_WAVE, 33, 0);
                } else {
                    KillTimer(self.hwnd, TIMER_WAVE);
                }
            }
        }
    }

    fn should_show(&self) -> bool {
        let cfg = settings::get();
        cfg.island_enabled && !self.suspended && !(cfg.hide_fullscreen && self.fullscreen_hidden)
    }

    fn sync_visibility(&mut self) {
        let show = self.should_show();
        self.media.set_paused(!show);
        self.plugin_host.set_paused(!show);
        self.sync_lyrics_timer();
        self.sync_wave_timer();
        unsafe {
            if show {
                ShowWindow(self.hwnd, 8); // SW_SHOWNA
                SetWindowPos(self.hwnd, -1, 0, 0, 0, 0, 0x0213);
                SetTimer(self.hwnd, TIMER_SECOND, 1000, 0);
                SetTimer(self.hwnd, TIMER_ZORDER, 250, 0);
                self.render();
            } else {
                KillTimer(self.hwnd, TIMER_SECOND);
                KillTimer(self.hwnd, TIMER_ANIM);
                KillTimer(self.hwnd, TIMER_LEAVE);
                KillTimer(self.hwnd, TIMER_ZORDER);
                KillTimer(self.hwnd, TIMER_LYRICS);
                KillTimer(self.hwnd, TIMER_WAVE);
                self.lyrics_timer = false;
                self.wave_timer = false;
                self.animating = false;
                self.watch_timer = false;
                self.outside_since = None;
                if let Some(h) = self.hires.as_mut() {
                    h.set_active(false);
                }
                self.set_focus_mode(false);
                self.expand.set(0.0);
                ShowWindow(self.hwnd, 0);
            }
        }
    }

    fn set_focus_mode(&self, active: bool) {
        unsafe {
            let ex = GetWindowLongW(self.hwnd, -20);
            let next = if active {
                ex & !0x0800_0000
            } else {
                ex | 0x0800_0000
            };
            if next != ex {
                SetWindowLongW(self.hwnd, -20, next);
            }
            if active {
                SetForegroundWindow(self.hwnd);
                SetFocus(self.hwnd);
            }
        }
    }

    fn start_anim(&mut self) {
        if !self.animating && self.should_show() {
            self.animating = true;
            self.last_frame = Instant::now();
            if let Some(h) = self.hires.as_mut() {
                h.set_active(true);
            }
            unsafe {
                SetTimer(self.hwnd, TIMER_ANIM, 8, 0);
            }
        }
    }

    fn set_watch_timer(&mut self, enable: bool) {
        if self.watch_timer != enable {
            self.watch_timer = enable;
            unsafe {
                if enable {
                    SetTimer(self.hwnd, TIMER_LEAVE, 50, 0);
                } else {
                    KillTimer(self.hwnd, TIMER_LEAVE);
                }
            }
        }
    }

    fn expand(&mut self) {
        self.outside_since = None;
        if self.expand.target < 0.5 {
            self.ensure_topmost();
        }
        self.expand.target = 1.0;
        self.set_watch_timer(true);
        self.start_anim();
    }

    fn collapse(&mut self) {
        if self.pinned
            || self.dragging_out
            || self.active_slider.is_some()
            || self.data.shelf.drag_over
            || self.data.todo.editing.is_some()
            || self.data.water.settings_open
        {
            return;
        }
        self.volume_open = false;
        self.outside_since = None;
        self.expand.target = 0.0;
        self.hovered = None;
        self.prev_hovered = None;
        self.hover_t = 1.0;
        self.pressed = None;
        self.press_spring.target = 0.0;
        self.start_anim();
    }

    fn cursor_inside_pill(&self) -> bool {
        let mut pt = Point::default();
        if unsafe { GetCursorPos(&mut pt) } == 0 {
            return false;
        }
        let s = self.scale();
        let px = (pt.x - self.origin.x) as f32 / s;
        let py = (pt.y - self.origin.y) as f32 / s;
        let (pill, _) = self.pill_rect();
        let margin = if self.expand.value >= 0.7 { 6.0 } else { 2.0 };
        RectF::new(
            pill.x - margin,
            pill.y - margin,
            pill.w + margin * 2.0,
            pill.h + margin * 2.0,
        )
        .contains(px, py)
    }

    fn update_cursor_watch(&mut self) {
        if self.pinned
            || self.dragging_out
            || self.active_slider.is_some()
            || self.data.shelf.drag_over
            || self.data.todo.editing.is_some()
            || self.data.water.settings_open
        {
            self.outside_since = None;
            return;
        }
        if self.cursor_inside_pill() {
            self.outside_since = None;
        } else if self.expand.target > 0.5 {
            let now = Instant::now();
            let since = *self.outside_since.get_or_insert(now);
            let grace = if self.expand.value < 0.65 {
                Duration::from_millis(35)
            } else {
                Duration::from_millis(95)
            };
            if now.duration_since(since) >= grace {
                self.collapse();
            }
        }
    }

    fn enabled_slots(&self) -> Vec<NavSlot> {
        let cfg = settings::get();
        let mut slots: Vec<NavSlot> = cfg.enabled_pages().into_iter().map(NavSlot::Page).collect();
        for (idx, entry) in self.plugin_entries.iter().enumerate() {
            if cfg.is_plugin_enabled(&entry.manifest.id) {
                slots.push(NavSlot::Plugin(idx));
            }
        }
        slots
    }

    fn set_slot(&mut self, slot: NavSlot) {
        let slots = self.enabled_slots();
        let target_idx = slots.iter().position(|&s| s == slot).unwrap_or(0);
        let actual = slots[target_idx];
        if self.active_slot != actual || (self.slide.target - target_idx as f32).abs() > 0.01 {
            self.data.todo.cancel();
            self.set_focus_mode(false);
            self.data.water.settings_open = false;
            self.volume_open = false;
            self.active_slider = None;
            self.active_slot = actual;
            if let NavSlot::Page(p) = actual {
                self.data.page = p;
                self.data.save();
            }
            self.slide.target = target_idx as f32;
            self.sync_wave_timer();
            self.start_anim();
            if !self.animating {
                self.render();
            }
        }
    }

    fn set_page(&mut self, page: Page) {
        self.set_slot(NavSlot::Page(page));
    }

    fn step_page(&mut self, delta: isize) {
        let slots = self.enabled_slots();
        if slots.len() <= 1 {
            return;
        }
        let cur = slots
            .iter()
            .position(|&s| s == self.active_slot)
            .unwrap_or(0) as isize;
        let len = slots.len() as isize;
        let next = ((cur + delta).rem_euclid(len)) as usize;
        self.set_slot(slots[next]);
    }

    fn active_plugin_alert(&self) -> Option<(usize, &PluginEntry)> {
        let cfg = settings::get();
        self.plugin_entries.iter().enumerate().find(|(_, e)| {
            cfg.is_plugin_enabled(&e.manifest.id)
                && e.data.updated
                && e.data.alert
                && !e.data.pill.is_empty()
        })
    }

    fn on_plugins_ready(&mut self) {
        self.plugin_entries = self.plugin_host.entries();
        let slots = self.enabled_slots();
        if !slots.contains(&self.active_slot) {
            self.active_slot = slots[0];
            self.slide.set(0.0);
        }
        self.update_compact_width();
        if self.should_show() && !self.animating {
            self.render();
        }
    }

    fn on_metrics(&mut self, metrics: &[Metric]) {
        self.metrics = metrics.to_vec();
        if let Some(cpu) = metrics.iter().find(|m| m.kind == Kind::Cpu) {
            if let Some(pct) = parse_pct(&cpu.value) {
                self.cpu_hist.rotate_left(1);
                self.cpu_hist[self.cpu_hist.len() - 1] = pct;
            }
        }
        let cfg = settings::get();
        let need_redraw = self.should_show()
            && !self.animating
            && ((self.expand.value < 0.25
                && matches!(cfg.idle_mode, IdleMode::Smart | IdleMode::Telemetry))
                || (self.expand.value > 0.25 && self.active_slot == NavSlot::Page(Page::System)));
        if self.expand.value < 0.25 {
            self.update_compact_width();
        }
        if need_redraw {
            self.render();
        }
    }

    fn on_settings_changed_live(&mut self) {
        let cfg = settings::get();
        self.data.enabled = cfg.island_enabled;
        self.update_geometry(&cfg);
        self.fullscreen_hidden = cfg.hide_fullscreen && is_foreground_fullscreen(self.hwnd);
        if self.should_show() {
            self.start_anim();
        } else {
            self.sync_visibility();
        }
    }

    fn on_settings_changed(&mut self) {
        let cfg = settings::get();
        self.data.enabled = cfg.island_enabled;
        self.update_geometry(&cfg);
        self.fullscreen_hidden = cfg.hide_fullscreen && is_foreground_fullscreen(self.hwnd);

        self.plugin_host.sync_enabled(&cfg.enabled_plugins);
        self.plugin_entries = self.plugin_host.entries();

        let slots = self.enabled_slots();
        if let Some(pos) = slots.iter().position(|&s| s == self.active_slot) {
            self.slide.target = pos as f32;
        } else {
            self.active_slot = slots[0];
            if let NavSlot::Page(p) = self.active_slot {
                self.data.page = p;
            }
            self.slide.set(0.0);
        }

        if cfg.lyrics_enabled {
            let snap = &self.media.snapshot;
            if snap.has_session && !snap.title.is_empty() {
                let (_, total_ms, _) = snap.timeline_ms();
                let dur = (total_ms / 1000) as u32;
                self.lyrics
                    .request(&snap.title, &snap.artist, &snap.album, dur);
            }
        } else {
            self.lyrics.clear();
        }

        self.sync_lyrics_timer();
        self.update_compact_width();
        self.sync_visibility();
    }

    fn on_media_ready(&mut self) {
        if !self.media.drain() {
            return;
        }
        let cfg = settings::get();
        let snap = &self.media.snapshot;
        if snap.art_id != self.art_id {
            self.art_id = snap.art_id;
            self.art = snap
                .art_bytes
                .as_ref()
                .and_then(|bytes| GpImage::from_bytes(self.api, bytes));
            self.art_tint = self.art.as_ref().and_then(|img| img.dominant_rgb(self.api));
            self.last_lyric_idx = None;
        }

        if cfg.lyrics_enabled {
            if snap.has_session && !snap.title.is_empty() {
                let (_, total_ms, _) = snap.timeline_ms();
                let dur = (total_ms / 1000) as u32;
                self.lyrics
                    .request(&snap.title, &snap.artist, &snap.album, dur);
            } else {
                self.lyrics.clear();
            }
        }

        if !self.vol_initialized {
            self.last_vol = snap.volume;
            self.last_muted = snap.muted;
            self.vol_initialized = true;
        } else if (snap.volume - self.last_vol).abs() > 0.005 || snap.muted != self.last_muted {
            self.last_vol = snap.volume;
            self.last_muted = snap.muted;
            if cfg.volume_hud_enabled && self.expand.value < 0.25 {
                self.vol_hud_until = Some(Instant::now() + Duration::from_millis(2000));
            }
        }

        self.wave_phase = (self.wave_phase + 0.55) % 62.83;
        let energy = if snap.muted {
            0.0
        } else if snap.peak > 0.01 {
            (snap.peak * 1.45).clamp(0.18, 1.0)
        } else if snap.playing {
            0.34
        } else {
            0.0
        };
        const WEIGHTS: [f32; 5] = [0.58, 0.92, 1.0, 0.80, 0.52];
        for (idx, w) in self.wave.iter_mut().enumerate() {
            let target = if energy > 0.0 {
                let mod_wave = 0.72 + 0.28 * (self.wave_phase + idx as f32 * 1.35).sin();
                (0.20 + energy * WEIGHTS[idx] * mod_wave).clamp(0.18, 1.0)
            } else {
                0.18
            };
            *w = *w * 0.45 + target * 0.55;
        }

        if !snap.audio_device_id.is_empty() {
            if self.last_audio_id.is_empty() {
                self.last_audio_id = snap.audio_device_id.clone();
            } else if snap.audio_device_id != self.last_audio_id {
                self.last_audio_id = snap.audio_device_id.clone();
                if cfg.alerts_enabled && cfg.toast_audio {
                    let headphone = snap.audio_is_headphone;
                    let dev_name = clean_audio_name(&snap.audio_device_name);
                    let title = if dev_name.is_empty() {
                        if headphone {
                            "Headphones".to_string()
                        } else {
                            "Audio".to_string()
                        }
                    } else {
                        dev_name
                    };
                    self.trigger_toast(
                        ToastKind::AudioConnected { headphone },
                        title,
                        "",
                        100,
                        3,
                        false,
                    );
                }
            }
        }

        self.sync_lyrics_timer();
        self.sync_wave_timer();
        self.update_compact_width();
        if self.should_show() && !self.animating {
            self.render();
        }
    }

    fn trigger_toast(
        &mut self,
        kind: ToastKind,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        pct: u8,
        secs: u64,
        force: bool,
    ) {
        if !force && !settings::get().alerts_enabled {
            return;
        }
        self.toast = Some(ActiveToast {
            kind,
            title: title.into(),
            subtitle: subtitle.into(),
            pct,
            until: Instant::now() + Duration::from_secs(secs),
        });
        self.toast_pulse = 0.0;
        self.update_compact_width();
        self.sync_wave_timer();
        if self.should_show() {
            self.ensure_topmost();
            if !self.animating {
                self.render();
            }
        }
    }

    fn check_power_change(&mut self) {
        let cfg = settings::get();
        let (pct, chg, saver, life) = battery_power_info();
        if cfg.alerts_enabled && cfg.toast_power {
            if let Some((prev_pct, prev_chg, prev_saver)) = self.last_power {
                if chg != prev_chg {
                    if chg {
                        let title = if pct >= 100 {
                            "Charged 100%".to_string()
                        } else {
                            format!("Charging {pct}%")
                        };
                        self.trigger_toast(ToastKind::Charging, title, "", pct, 3, false);
                    } else {
                        let title = match life {
                            Some(secs) => format!("{pct}% · {}", format_battery_remaining(secs)),
                            None => format!("{pct}%"),
                        };
                        self.trigger_toast(ToastKind::Unplugged, title, "", pct, 4, false);
                    }
                } else if saver != prev_saver {
                    if saver {
                        self.trigger_toast(
                            ToastKind::BatterySaverOn,
                            format!("Eco {pct}%"),
                            "",
                            pct,
                            3,
                            false,
                        );
                    } else {
                        self.trigger_toast(
                            ToastKind::BatterySaverOff,
                            format!("Standard {pct}%"),
                            "",
                            pct,
                            3,
                            false,
                        );
                    }
                } else if !chg && prev_pct > 20 && pct <= 20 {
                    let title = match life {
                        Some(secs) => format!("Low {pct}% · {}", format_battery_remaining(secs)),
                        None => format!("Low {pct}%"),
                    };
                    self.trigger_toast(ToastKind::LowBattery, title, "", pct, 4, false);
                }
            }
        }
        self.last_power = Some((pct, chg, saver));
    }

    fn handle_device_change(&mut self, event: usize, data: isize) {
        let cfg = settings::get();
        if !cfg.alerts_enabled
            || !cfg.toast_devices
            || data == 0
            || (event != 0x8000 && event != 0x8004)
        {
            return;
        }
        let arrived = event == 0x8000;
        let now = Instant::now();
        let hdr = unsafe { &*(data as *const DevBroadcastHdr) };
        if hdr.dev_type == 5 && hdr.size >= size_of::<DevBroadcastDeviceInterface>() as u32 {
            let iface = unsafe { &*(data as *const DevBroadcastDeviceInterface) };
            let g = &iface.class_guid;
            let specific = if *g == GUID_DEVINTERFACE_MOUSE {
                Some((
                    if arrived {
                        ToastKind::MouseConnected
                    } else {
                        ToastKind::MouseRemoved
                    },
                    if arrived { "Mouse" } else { "Removed" },
                ))
            } else if *g == GUID_DEVINTERFACE_KEYBOARD {
                Some((
                    if arrived {
                        ToastKind::KeyboardConnected
                    } else {
                        ToastKind::KeyboardRemoved
                    },
                    if arrived { "Keyboard" } else { "Removed" },
                ))
            } else if *g == GUID_DEVINTERFACE_USB_DEVICE {
                let recently_toast = self
                    .last_dev_toast
                    .is_some_and(|t| now.saturating_duration_since(t) < Duration::from_millis(900));
                if recently_toast {
                    return;
                }
                Some((
                    if arrived {
                        ToastKind::UsbConnected
                    } else {
                        ToastKind::UsbRemoved
                    },
                    if arrived { "USB Ready" } else { "Removed" },
                ))
            } else {
                None
            };
            if let Some((kind, title)) = specific {
                let is_hid = !matches!(kind, ToastKind::UsbConnected | ToastKind::UsbRemoved);
                if !is_hid
                    && self.toast.as_ref().is_some_and(|t| {
                        now < t.until
                            && !matches!(t.kind, ToastKind::UsbConnected | ToastKind::UsbRemoved)
                    })
                {
                    return;
                }
                self.last_dev_toast = Some(now);
                self.trigger_toast(kind, title, "", 100, 3, false);
            }
        } else if hdr.dev_type == 2 && hdr.size >= size_of::<DevBroadcastVolume>() as u32 {
            let vol = unsafe { &*(data as *const DevBroadcastVolume) };
            let letter = (0..26u32)
                .find(|&bit| (vol.unit_mask & (1 << bit)) != 0)
                .map(|bit| (b'A' + bit as u8) as char)
                .unwrap_or('?');
            self.last_dev_toast = Some(now);
            if arrived {
                let sub = drive_free_label(letter).unwrap_or_default();
                self.trigger_toast(
                    ToastKind::DriveConnected,
                    format!("{letter}:"),
                    sub,
                    100,
                    4,
                    false,
                );
            } else {
                self.trigger_toast(
                    ToastKind::DriveRemoved,
                    format!("{letter}:"),
                    "Ejected",
                    0,
                    3,
                    false,
                );
            }
        }
    }

    fn on_wave_tick(&mut self) {
        let now = Instant::now();
        if self.toast.as_ref().is_some_and(|t| now >= t.until) {
            self.toast = None;
            self.update_compact_width();
        }
        self.toast_pulse = (self.toast_pulse + 0.14) % 62.83;

        let snap = &self.media.snapshot;
        if snap.playing {
            self.wave_phase = (self.wave_phase + 0.26) % 62.83;
            let energy = if snap.muted {
                0.0
            } else {
                (snap.peak * 1.65).clamp(0.44, 1.0)
            };
            const WEIGHTS: [f32; 5] = [0.64, 0.94, 1.0, 0.88, 0.60];
            for (idx, w) in self.wave.iter_mut().enumerate() {
                let target = if energy > 0.0 {
                    let s1 = (self.wave_phase + idx as f32 * 1.15).sin() * 0.5 + 0.5;
                    let s2 = (self.wave_phase * 1.7 - idx as f32 * 0.85).cos() * 0.5 + 0.5;
                    let wave_mod = 0.35 + 0.65 * (0.6 * s1 + 0.4 * s2);
                    (0.18 + energy * WEIGHTS[idx] * wave_mod).clamp(0.16, 1.0)
                } else {
                    0.18
                };
                *w = *w * 0.55 + target * 0.45;
            }
        }
        self.sync_wave_timer();
        if self.should_show() && !self.animating {
            self.render();
        }
    }

    fn on_lyrics_ready(&mut self) {
        if !self.lyrics.drain() {
            return;
        }
        self.last_lyric_idx = None;
        self.sync_lyrics_timer();
        self.update_compact_width();
        if self.should_show() && !self.animating {
            self.render();
        }
    }

    fn on_lyrics_tick(&mut self) {
        if !self.should_show() || !self.media.snapshot.playing {
            return;
        }
        let idx = self.current_synced_lyric().map(|(i, _, _)| i);
        if idx != self.last_lyric_idx {
            self.last_lyric_idx = idx;
            self.update_compact_width();
            if !self.animating {
                self.render();
            }
        }
    }

    fn interaction_state(&self, act: Action) -> (f32, f32) {
        let h = if self.hovered == Some(act) {
            self.hover_t
        } else if self.prev_hovered == Some(act) {
            1.0 - self.hover_t
        } else {
            0.0
        };
        let p =
            if self.pressed == Some(act) || (self.pressed.is_none() && self.hovered == Some(act)) {
                self.press_spring.value.clamp(0.0, 1.0)
            } else {
                0.0
            };
        (h.clamp(0.0, 1.0), p)
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_liquid_button(
        &self,
        surf: &Surface<'_>,
        pal: &Palette,
        rect: RectF,
        r: f32,
        act: Action,
        is_cta: bool,
        alpha: f32,
        clip: Option<(RectF, f32)>,
    ) -> RectF {
        let (ht, pt) = self.interaction_state(act);
        if is_cta {
            let dr = Surface::deform_gel_rect(rect, ht * 0.55, pt * 0.7);
            let dr_r = (r + (dr.h - rect.h) * 0.5).max(2.0);
            surf.fill_round(dr, dr_r, with_alpha(pal.cta_bg, alpha * (0.94 + 0.06 * ht)));
            return dr;
        }
        surf.draw_quartz_pill(rect, r, 0, ht, pt, false, pal.light, alpha, clip)
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_liquid_switch(
        &self,
        surf: &Surface<'_>,
        pal: &Palette,
        rect: RectF,
        on: bool,
        accent: u32,
        act: Action,
        alpha: f32,
        clip: Option<(RectF, f32)>,
    ) {
        let (ht, pt) = self.interaction_state(act);
        let drect = Surface::deform_gel_rect(rect, ht * 0.6, pt * 0.65);
        let tr = drect.h * 0.5;
        let bg = if on {
            accent
        } else {
            mix_color(
                with_alpha(pal.track, alpha),
                with_alpha(pal.btn_hover, alpha),
                ht,
            )
        };
        surf.fill_round(drect, tr, bg);
        let thumb_w = 18.0 + pt * 3.5 + ht * 0.8;
        let thumb_h = (drect.h - 6.0).max(12.0);
        let thumb_x = if on {
            drect.x + drect.w - thumb_w - 3.0
        } else {
            drect.x + 3.0
        };
        let thumb_r = RectF::new(
            thumb_x,
            drect.y + (drect.h - thumb_h) * 0.5,
            thumb_w,
            thumb_h,
        );
        surf.draw_quartz_pill(
            thumb_r,
            thumb_h * 0.5,
            with_alpha(pal.on_accent, alpha),
            ht,
            pt,
            true,
            pal.light,
            alpha,
            clip,
        );
    }

    fn on_anim_tick(&mut self) {
        self.update_cursor_watch();

        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;

        let cfg = settings::get();
        let s_mul = cfg.motion_speed.spring_scale();
        let d_mul = s_mul.sqrt();
        let (stiff, damp) = if self.expand.target > 0.5 {
            (520.0 * s_mul, 36.0 * d_mul)
        } else {
            (640.0 * s_mul, 48.0 * d_mul)
        };
        let m1 = self.expand.step(dt, stiff, damp);
        let m2 = self.slide.step(dt, 520.0 * s_mul, 38.0 * d_mul);
        let m3 = self.compact_w.step(dt, 480.0 * s_mul, 36.0 * d_mul);
        let m4 = self.press_spring.step(dt, 760.0 * s_mul, 32.0 * d_mul);
        self.dock_nav.update(dt * d_mul);
        let m5 = self.dock_nav.animating();

        let mut hover_moving = false;
        if self.hover_t < 1.0 {
            self.hover_t = (self.hover_t + dt * 11.0 * d_mul).min(1.0);
            if self.hover_t >= 1.0 {
                self.prev_hovered = None;
            } else {
                hover_moving = true;
            }
        }

        self.render();

        if !m1 && !m2 && !m3 && !m4 && !m5 && !hover_moving {
            self.animating = false;
            if let Some(h) = self.hires.as_mut() {
                h.set_active(false);
            }
            unsafe {
                KillTimer(self.hwnd, TIMER_ANIM);
            }
            self.sync_wave_timer();
            if self.expand.target <= 0.5 && self.expand.value == 0.0 {
                self.set_watch_timer(false);
            }
        }
    }

    fn on_second_tick(&mut self) {
        self.refresh_clock();
        self.check_power_change();
        let st = self.cached_time;
        let today = date_key(&st);
        let mut dirty = false;

        if self
            .toast
            .as_ref()
            .is_some_and(|t| Instant::now() >= t.until)
        {
            self.toast = None;
            self.update_compact_width();
            self.sync_wave_timer();
            dirty = true;
        }
        if self.vol_hud_until.is_some_and(|t| Instant::now() >= t) {
            self.vol_hud_until = None;
            self.update_compact_width();
            dirty = true;
        }
        if self.copied_until.is_some_and(|t| Instant::now() >= t) {
            self.copied_until = None;
            dirty = true;
        }
        if self.track_copied_until.is_some_and(|t| Instant::now() >= t) {
            self.track_copied_until = None;
            dirty = true;
        }

        if self.data.pomodoro.tick_second().is_some() {
            unsafe {
                MessageBeep(0x40);
            }
            self.set_page(Page::Pomodoro);
            self.expand();
            dirty = true;
        } else if self.data.pomodoro.running || self.data.pomodoro.alert_secs > 0 {
            dirty = true;
        }

        if self.data.water.tick_second(today) {
            unsafe {
                MessageBeep(0x40);
            }
            self.set_page(Page::Water);
            self.expand();
            dirty = true;
        }

        if self.data.todo.editing.is_some() {
            self.caret_tick = self.caret_tick.wrapping_add(1);
            dirty = true;
        }

        if self.media.snapshot.playing
            && self.expand.value > 0.25
            && self.active_slot == NavSlot::Page(Page::Media)
        {
            dirty = true;
        }

        self.update_compact_width();
        if dirty && !self.animating {
            self.render();
        }
    }

    fn update_geometry(&mut self, cfg: &settings::AppSettings) {
        let (bounds, work, base_dpi) = primary_monitor_geometry();
        let (card_top, compact_h) = detect_taskbar_fit(bounds, base_dpi, cfg.taskbar_fit);
        self.card_top = card_top;
        self.compact_h = compact_h;
        let eff_dpi = effective_dpi(base_dpi, cfg.island_scale_pct);
        let scale = (eff_dpi as f32) / 96.0;
        let px_w = (CANVAS_W * scale).round() as i32;
        let px_h = (CANVAS_H * scale).round() as i32;
        self.origin = compute_origin(bounds, work, scale, px_w, px_h, cfg);
        let need_surface = self.dpi != eff_dpi
            || self
                .surface
                .as_ref()
                .is_none_or(|s| s.width != px_w || s.height != px_h);
        if need_surface {
            self.dpi = eff_dpi;
            let mut surf = Surface::new(self.api, px_w, px_h, eff_dpi);
            if let Some(ref mut s) = surf {
                s.scale = scale;
            }
            self.surface = surf;
        }
    }

    fn refresh_geometry(&mut self) {
        let cfg = settings::get();
        self.update_geometry(&cfg);
        self.watch_explorer();
        self.ensure_topmost();
        if self.should_show() {
            self.render();
        }
    }

    fn pill_rect(&self) -> (RectF, f32) {
        let e = self.expand.value;
        let w = lerp(self.compact_w.value, EXPANDED_W, e).max(16.0);
        let h = lerp(self.compact_h, EXPANDED_H, e).max(16.0);
        let r = lerp(self.compact_h * 0.5, EXPANDED_R, e).max(4.0);
        let y = lerp(self.card_top, self.card_top.max(4.0), e);
        let x = match settings::get().island_anchor {
            IslandAnchor::Center => (CANVAS_W - w) * 0.5,
            IslandAnchor::Left => 20.0,
            IslandAnchor::Right => CANVAS_W - 20.0 - w,
        };
        (RectF::new(x, y, w, h), r)
    }

    fn render(&mut self) {
        if !self.should_show() || self.surface.is_none() {
            return;
        }
        let cfg = settings::get();
        let pal = Palette::for_theme(cfg.is_light());
        let st = self.cached_time;
        let today = date_key(&st);
        self.data.water.sync_date(today);
        let (pill, radius) = self.pill_rect();
        let e = self.expand.value;

        let mut actions = Vec::with_capacity(36);
        let Some(surf) = self.surface.take() else {
            return;
        };

        surf.clear();

        let art_bleed = self.art_tint.filter(|_| {
            self.media.snapshot.playing || self.active_slot == NavSlot::Page(Page::Media)
        });
        surf.draw_quartz_shell_styled(
            pill,
            radius,
            pal.light,
            cfg.glass_style.opacity_mul(),
            art_bleed,
        );

        surf.set_round_clip(pill, radius);

        if e < 0.26 {
            let alpha = (1.0 - e / 0.26).clamp(0.0, 1.0);
            self.draw_compact(&surf, &pal, pill, alpha);
            actions.push((pill, self.compact_click_action(today)));
        }

        if e > 0.24 {
            let alpha = ((e - 0.24) / 0.56).clamp(0.0, 1.0);
            self.draw_expanded(&surf, &pal, pill, &st, today, alpha, &mut actions);
        }

        surf.reset_clip();
        surf.present(self.hwnd, self.origin.x, self.origin.y);
        self.surface = Some(surf);
        self.actions = actions;
    }

    fn dock_layout_slots(
        &self,
        surf: &Surface<'_>,
        pill: RectF,
        lift: f32,
    ) -> (RectF, usize, usize, Vec<(usize, RectF)>, bool) {
        let slots = self.enabled_slots();
        let snap = &self.media.snapshot;
        let media_idx = slots.iter().position(|&s| s == NavSlot::Page(Page::Media));
        let media_blend = media_idx
            .map(|idx| (1.0 - (self.slide.value - idx as f32).abs()).clamp(0.0, 1.0))
            .unwrap_or(0.0);

        let dock_y = pill.y + 9.0 + lift;
        let dock_h = 26.0;
        const MAX_DOCK: usize = 6;
        let active_idx = slots
            .iter()
            .position(|&s| s == self.active_slot)
            .unwrap_or(0);
        let vis_count = slots.len().clamp(1, MAX_DOCK);
        let mut vis_start = active_idx.saturating_sub(MAX_DOCK / 2);
        if vis_start + vis_count > slots.len() {
            vis_start = slots.len().saturating_sub(vis_count);
        }
        let vis_end = (vis_start + vis_count).min(slots.len());
        let overflow = slots.len() > MAX_DOCK;

        let mut widths = Vec::with_capacity(vis_count);
        for &slot in &slots[vis_start..vis_end] {
            let lbl = slot.label(&self.plugin_entries);
            let w = (surf.measure_text_width(lbl, 10.6, true) + 16.0).clamp(42.0, 72.0);
            widths.push(w);
        }
        let raw_sum: f32 = widths.iter().sum();
        let cfg = settings::get();
        let privacy_active = cfg.privacy_dots && (snap.mic_active || snap.cam_active);
        let right_reserve = if media_blend > 0.02 || privacy_active {
            62.0
        } else {
            24.0
        };
        let chev_pad = if overflow { 16.0 } else { 4.0 };
        let max_slots_w = (pill.w - 20.0 - right_reserve - chev_pad * 2.0).max(220.0);
        if raw_sum > max_slots_w && raw_sum > 0.0 {
            let k = max_slots_w / raw_sum;
            for w in &mut widths {
                *w *= k;
            }
        }
        let slots_w: f32 = widths.iter().sum();
        let dock_w = slots_w + chev_pad * 2.0;
        let max_dock_right = pill.x + pill.w - right_reserve;
        let ideal_x = pill.x + (pill.w - dock_w) * 0.5;
        let dock_x = ideal_x.clamp(pill.x + 14.0, (max_dock_right - dock_w).max(pill.x + 14.0));
        let dock_rect = RectF::new(dock_x, dock_y, dock_w, dock_h);

        let mut slot_rects = Vec::with_capacity(vis_count);
        let mut sx = dock_x + chev_pad;
        for (idx, w) in widths.into_iter().enumerate() {
            slot_rects.push((
                vis_start + idx,
                RectF::new(sx, dock_y + 2.5, w, dock_h - 5.0),
            ));
            sx += w;
        }
        (dock_rect, active_idx, vis_start, slot_rects, overflow)
    }

    fn compact_click_action(&self, today: u32) -> Action {
        let cfg = settings::get();
        if self.toast_active() {
            return Action::Expand;
        }
        if self.vol_hud_active() {
            return if cfg.page_media {
                Action::ExpandTo(Page::Media)
            } else {
                Action::Expand
            };
        }
        if self.data.pomodoro.running || self.data.pomodoro.alert_secs > 0 {
            return if cfg.page_pomodoro {
                Action::ExpandTo(Page::Pomodoro)
            } else {
                Action::Expand
            };
        }
        if self.data.water.due_alert {
            return if cfg.page_water {
                Action::ExpandTo(Page::Water)
            } else {
                Action::Expand
            };
        }
        if self.show_cpu_hog_banner() && cfg.page_system {
            return Action::ExpandTo(Page::System);
        }
        if let Some((idx, _)) = self.active_plugin_alert() {
            return Action::ExpandToPlugin(idx);
        }
        if self.media.snapshot.playing {
            return if cfg.page_media {
                Action::ExpandTo(Page::Media)
            } else {
                Action::Expand
            };
        }
        if cfg.idle_mode == IdleMode::Smart {
            if self.abnormal_system_alert().is_some() && cfg.page_system {
                return Action::ExpandTo(Page::System);
            }
            if cfg.page_todo && self.data.todo.top_pending_task(today).is_some() {
                return Action::ExpandTo(Page::Todo);
            }
            if cfg.page_water
                && self.data.water.reminder_enabled
                && self.data.water.consumed_ml < self.data.water.goal_ml
                && self.data.water.next_reminder_secs <= 600
            {
                return Action::ExpandTo(Page::Water);
            }
        }
        Action::Expand
    }

    fn draw_compact(&self, surf: &Surface<'_>, pal: &Palette, pill: RectF, alpha: f32) {
        let cfg = settings::get();
        let ink = with_alpha(pal.ink, alpha);
        let ink_soft = with_alpha(pal.ink_soft, alpha);
        let muted = with_alpha(pal.muted, alpha);
        let track = with_alpha(pal.track, alpha);
        let snap = &self.media.snapshot;
        let cy = pill.y + pill.h * 0.5;
        let small = pill.h < 28.0;

        // Priority 1: Hardware / Power Event Toast
        if let Some(toast) = self.toast.as_ref().filter(|t| Instant::now() < t.until) {
            let accent = toast_color(toast.kind, pal);
            let accent_a = with_alpha(accent, alpha);
            let pulse = 0.65 + 0.35 * self.toast_pulse.sin();
            match toast.kind {
                ToastKind::Charging
                | ToastKind::Unplugged
                | ToastKind::BatterySaverOn
                | ToastKind::BatterySaverOff
                | ToastKind::LowBattery => {
                    draw_battery_capsule(
                        surf,
                        pill.x + 12.0,
                        cy,
                        toast.pct,
                        toast.kind,
                        self.toast_pulse,
                        pal,
                        alpha,
                    );
                    surf.draw_text(
                        &toast.title,
                        RectF::new(pill.x + 39.0, pill.y, pill.w - 51.0, pill.h),
                        TextStyle::new(if small { 10.5 } else { 11.2 }, true, ink, 0),
                    );
                }
                ToastKind::AudioConnected { .. } => {
                    let cx = pill.x + 18.5;
                    let phase0 = (self.toast_pulse * 0.28) % 1.0;
                    let phase1 = (phase0 + 0.5) % 1.0;
                    for p in [phase0, phase1] {
                        let rr = 6.5 + p * 6.0;
                        let ra = (1.0 - p) * 0.45 * alpha;
                        surf.stroke_circle(cx, cy, rr, 1.15, with_alpha(accent, ra));
                    }
                    draw_toast_icon(surf, toast.kind, cx, cy, accent_a);
                    surf.draw_text(
                        &toast.title,
                        RectF::new(pill.x + 35.0, pill.y, (pill.w - 71.0).max(48.0), pill.h),
                        TextStyle::new(if small { 10.5 } else { 11.2 }, true, ink, 0),
                    );
                    draw_toast_waveform(
                        surf,
                        pill.x + pill.w - 28.0,
                        cy,
                        self.toast_pulse,
                        (pill.h * 0.42).clamp(8.5, 12.5),
                        with_alpha(accent, 0.92 * alpha),
                    );
                }
                ToastKind::DriveConnected | ToastKind::DriveRemoved => {
                    let cx = pill.x + 18.0;
                    surf.fill_circle(
                        cx,
                        cy,
                        9.0 + 1.1 * pulse,
                        with_alpha(accent, (0.14 + 0.08 * pulse) * alpha),
                    );
                    draw_toast_icon(surf, toast.kind, cx, cy, accent_a);
                    if toast.subtitle.is_empty() {
                        surf.draw_text(
                            &toast.title,
                            RectF::new(pill.x + 33.0, pill.y, pill.w - 45.0, pill.h),
                            TextStyle::new(if small { 10.5 } else { 11.2 }, true, ink, 0),
                        );
                    } else {
                        let sw = (surf.measure_text_width(
                            &toast.subtitle,
                            if small { 10.0 } else { 10.6 },
                            true,
                        ) + 4.0)
                            .min((pill.w - 46.0) * 0.62);
                        let left_w = (pill.w - 46.0 - sw).max(26.0);
                        surf.draw_text(
                            &toast.title,
                            RectF::new(pill.x + 33.0, pill.y, left_w, pill.h),
                            TextStyle::new(if small { 10.5 } else { 11.2 }, true, ink, 0),
                        );
                        surf.draw_text(
                            &toast.subtitle,
                            RectF::new(pill.x + pill.w - 13.0 - sw, pill.y, sw, pill.h),
                            TextStyle::new(if small { 10.0 } else { 10.6 }, true, accent_a, 2),
                        );
                    }
                }
                ToastKind::MouseConnected
                | ToastKind::MouseRemoved
                | ToastKind::KeyboardConnected
                | ToastKind::KeyboardRemoved
                | ToastKind::UsbConnected
                | ToastKind::UsbRemoved => {
                    let cx = pill.x + 18.0;
                    surf.stroke_circle(
                        cx,
                        cy,
                        8.2 + 0.8 * pulse,
                        1.1,
                        with_alpha(accent, 0.34 * alpha),
                    );
                    draw_toast_icon(surf, toast.kind, cx, cy, accent_a);
                    surf.draw_text(
                        &toast.title,
                        RectF::new(pill.x + 33.0, pill.y, (pill.w - 55.0).max(40.0), pill.h),
                        TextStyle::new(if small { 10.5 } else { 11.2 }, true, ink, 0),
                    );
                    let dot_x = pill.x + pill.w - 14.0;
                    surf.fill_circle(dot_x, cy, 5.0, with_alpha(accent, 0.18 * alpha));
                    surf.fill_circle(dot_x, cy, 2.8, accent_a);
                }
            }
            return;
        }

        // Priority 2: Transient Volume HUD
        if self.vol_hud_active() {
            let spk_col = if snap.muted {
                with_alpha(pal.accent_red, alpha)
            } else {
                ink
            };
            draw_speaker_icon(surf, pill.x + 20.0, cy, snap.muted, spk_col);
            let bar = RectF::new(pill.x + 38.0, cy - 2.5, pill.w - 94.0, 5.0);
            surf.fill_round(bar, 2.5, track);
            let vol = if snap.muted { 0.0 } else { snap.volume };
            if vol > 0.01 {
                surf.fill_round(
                    RectF::new(bar.x, bar.y, (bar.w * vol).max(5.0), bar.h),
                    2.5,
                    ink,
                );
            }
            let txt = if snap.muted {
                "Muted".into()
            } else {
                format!("{}%", (snap.volume * 100.0).round() as u32)
            };
            surf.draw_text(
                &txt,
                RectF::new(pill.x + pill.w - 54.0, pill.y, 42.0, pill.h),
                TextStyle::new(if small { 10.2 } else { 11.0 }, true, ink, 2),
            );
            return;
        }

        // Priority 3: Pomodoro / Water live alerts or running Pomodoro
        if self.data.pomodoro.running {
            let dot = with_alpha(pal.ink, alpha);
            surf.fill_circle(pill.x + 17.0, cy, 3.6, dot);
            surf.draw_text(
                self.data.pomodoro.phase.label(),
                RectF::new(pill.x + 26.0, pill.y, 92.0, pill.h),
                TextStyle::new(if small { 10.5 } else { 11.4 }, true, ink, 0),
            );
            let mins = self.data.pomodoro.remaining_secs / 60;
            let secs = self.data.pomodoro.remaining_secs % 60;
            surf.draw_text(
                &format!("{mins:02}:{secs:02}"),
                RectF::new(pill.x + pill.w - 84.0, pill.y, 52.0, pill.h),
                TextStyle::new(if small { 11.0 } else { 12.0 }, true, ink, 2),
            );
            let r_sz = (pill.h - 12.0).clamp(13.0, 16.0);
            let ring = RectF::new(pill.x + pill.w - 10.0 - r_sz, cy - r_sz * 0.5, r_sz, r_sz);
            surf.stroke_circle(ring.x + r_sz * 0.5, cy, r_sz * 0.5 - 1.0, 2.0, track);
            surf.stroke_arc(
                ring,
                2.0,
                -90.0,
                360.0 * self.data.pomodoro.progress().max(0.04),
                dot,
            );
            return;
        }
        if self.data.pomodoro.alert_secs > 0 {
            let green = with_alpha(pal.accent_green, alpha);
            surf.fill_circle(pill.x + 17.0, cy, 3.8, green);
            surf.draw_text(
                "Done",
                RectF::new(pill.x + 26.0, pill.y, 62.0, pill.h),
                TextStyle::new(10.8, true, green, 0),
            );
            surf.draw_text(
                self.data.pomodoro.phase.label(),
                RectF::new(pill.x + pill.w - 110.0, pill.y, 96.0, pill.h),
                TextStyle::new(10.8, true, ink, 2),
            );
            return;
        }
        if self.data.water.due_alert {
            let cyan = with_alpha(pal.accent_blue, alpha);
            draw_droplet(surf, pill.x + 17.0, cy, 5.0, cyan);
            surf.draw_text(
                "Water",
                RectF::new(pill.x + 27.0, pill.y, 68.0, pill.h),
                TextStyle::new(11.0, true, ink, 0),
            );
            surf.draw_text(
                &format!("+{} ml", self.data.water.cup_ml),
                RectF::new(pill.x + pill.w - 86.0, pill.y, 72.0, pill.h),
                TextStyle::new(11.0, true, cyan, 2),
            );
            return;
        }

        // Priority 4: Runaway / High-CPU Process Temporary Stretch Banner
        let font_sz = if small { 10.4 } else { 11.1 };
        if self.show_cpu_hog_banner() {
            if let Some((name, pct, temp, val)) = self.cpu_hog() {
                let is_crit = temp.is_some_and(|t| t >= 88);
                let col = with_alpha(
                    if is_crit {
                        pal.accent_red
                    } else {
                        pal.accent_amber
                    },
                    alpha,
                );
                surf.fill_circle(pill.x + 15.0, cy, 3.6, col);
                let left_txt = if let Some(t) = temp {
                    format!(
                        "CPU {} · {name}",
                        settings::format_temp(t, cfg.temp_fahrenheit)
                    )
                } else {
                    format!("CPU {val} · {name}")
                };
                let right_txt = format!("{pct}%");
                let rw = surf.measure_text_width(&right_txt, font_sz, true) + 4.0;
                surf.draw_text(
                    &left_txt,
                    RectF::new(
                        pill.x + 24.0,
                        pill.y,
                        (pill.w - 38.0 - rw).max(48.0),
                        pill.h,
                    ),
                    TextStyle::new(font_sz, true, ink, 0),
                );
                surf.draw_text(
                    &right_txt,
                    RectF::new(pill.x + pill.w - 13.0 - rw, pill.y, rw, pill.h),
                    TextStyle::new(font_sz, true, col, 2),
                );
                return;
            }
        }

        // Priority 5: Active Plugin Alert Pill
        if let Some((_, entry)) = self.active_plugin_alert() {
            let dot_col = match entry.data.tone {
                PluginTone::Good => pal.accent_green,
                PluginTone::Warn => pal.accent_amber,
                PluginTone::Critical => pal.accent_red,
                PluginTone::Normal => pal.accent_blue,
            };
            surf.fill_circle(pill.x + 15.0, cy, 3.6, with_alpha(dot_col, alpha));
            surf.draw_text(
                &entry.data.pill,
                RectF::new(pill.x + 25.0, pill.y, pill.w - 38.0, pill.h),
                TextStyle::new(font_sz, true, ink, 0),
            );
            return;
        }

        // Priority 6: Media playing (with synced lyrics if enabled)
        if snap.playing {
            let art_sz = (pill.h - 8.0).clamp(16.0, 20.0);
            let art_rect = RectF::new(pill.x + 9.0, cy - art_sz * 0.5, art_sz, art_sz);
            let pill_r = self.compact_h * 0.5;
            if let Some(img) = &self.art {
                surf.draw_image_round(img, art_rect, 4.5, Some((pill, pill_r)));
            } else {
                surf.fill_round(art_rect, 4.5, with_alpha(pal.card_bg, alpha));
                draw_note_icon(surf, art_rect.x + art_sz * 0.5, cy, 0.62, ink);
            }

            let lyric_line = if cfg.lyrics_enabled && cfg.lyrics_in_pill {
                self.current_synced_lyric()
                    .map(|(_, line, _)| line)
                    .filter(|l| !l.is_empty())
            } else {
                None
            };

            if let Some(line) = lyric_line {
                surf.draw_text(
                    line,
                    RectF::new(
                        pill.x + 14.0 + art_sz,
                        pill.y,
                        pill.w - art_sz - 60.0,
                        pill.h,
                    ),
                    TextStyle::new(if small { 11.0 } else { 11.5 }, true, ink, 0),
                );
            } else {
                let title = if snap.title.is_empty() {
                    "Playing"
                } else {
                    &snap.title
                };
                surf.draw_text(
                    title,
                    RectF::new(
                        pill.x + 14.0 + art_sz,
                        pill.y,
                        pill.w - art_sz - 60.0,
                        pill.h,
                    ),
                    TextStyle::new(if small { 10.8 } else { 11.5 }, true, ink, 0),
                );
            }

            draw_waveform(
                surf,
                pill.x + pill.w - 36.0,
                cy,
                &self.wave,
                (pill.h * 0.42).clamp(9.0, 13.0),
                with_alpha(pal.ink_soft, alpha),
            );
            draw_privacy_dots(
                surf,
                pill.x + pill.w - 44.0,
                pill.y + 6.0,
                snap.mic_active,
                snap.cam_active,
                pal,
                alpha,
            );
            return;
        }

        // Priority 6: Contextual & User-configured Idle Pill Mode
        let today = date_key(&self.cached_time);
        let privacy_active = cfg.privacy_dots && (snap.mic_active || snap.cam_active);
        match cfg.idle_mode {
            IdleMode::Smart => {
                if privacy_active {
                    let (label, dot_col) = if snap.cam_active && snap.mic_active {
                        ("Camera & Mic Active", pal.accent_green)
                    } else if snap.cam_active {
                        ("Camera Active", pal.accent_green)
                    } else {
                        ("Mic Active", pal.accent_amber)
                    };
                    surf.fill_circle(pill.x + 16.0, cy, 3.6, with_alpha(dot_col, alpha));
                    surf.draw_text(
                        label,
                        RectF::new(pill.x + 25.0, pill.y, pill.w - 38.0, pill.h),
                        TextStyle::new(font_sz, true, ink, 0),
                    );
                } else if let Some((msg, crit)) = self.abnormal_system_alert() {
                    let col = if crit {
                        with_alpha(pal.accent_red, alpha)
                    } else {
                        with_alpha(pal.accent_amber, alpha)
                    };
                    surf.fill_circle(pill.x + 16.0, cy, 3.6, col);
                    surf.draw_text(
                        &msg,
                        RectF::new(pill.x + 25.0, pill.y, pill.w - 38.0, pill.h),
                        TextStyle::new(font_sz, true, ink, 0),
                    );
                } else if let Some(task) = cfg
                    .page_todo
                    .then(|| self.data.todo.top_pending_task(today))
                    .flatten()
                {
                    surf.stroke_circle(pill.x + 16.0, cy, 4.2, 1.3, ink_soft);
                    surf.draw_text(
                        task,
                        RectF::new(pill.x + 26.0, pill.y, pill.w - 38.0, pill.h),
                        TextStyle::new(font_sz, true, ink, 0),
                    );
                } else if cfg.page_water
                    && self.data.water.reminder_enabled
                    && self.data.water.consumed_ml < self.data.water.goal_ml
                    && self.data.water.next_reminder_secs <= 600
                {
                    let mins = self.data.water.next_reminder_secs.div_ceil(60);
                    draw_droplet(
                        surf,
                        pill.x + 16.0,
                        cy,
                        4.4,
                        with_alpha(pal.accent_blue, alpha),
                    );
                    surf.draw_text(
                        &format!("Water in {mins}m"),
                        RectF::new(pill.x + 25.0, pill.y, pill.w - 38.0, pill.h),
                        TextStyle::new(font_sz, true, ink, 0),
                    );
                } else {
                    let cpu_temp = self
                        .metrics
                        .iter()
                        .find(|m| m.kind == Kind::Cpu)
                        .and_then(|c| c.temperature);
                    let dot_col = with_alpha(thermal_color(cpu_temp, 80, pal), alpha);
                    surf.fill_circle(pill.x + 15.0, cy, 3.4, dot_col);
                    let summary = self.telemetry_text();
                    surf.draw_text(
                        &summary,
                        RectF::new(pill.x + 24.0, pill.y, pill.w - 36.0, pill.h),
                        TextStyle::new(font_sz, true, ink, 0),
                    );
                }
            }
            IdleMode::Telemetry => {
                let cpu_temp = self
                    .metrics
                    .iter()
                    .find(|m| m.kind == Kind::Cpu)
                    .and_then(|c| c.temperature);
                let dot_col = with_alpha(thermal_color(cpu_temp, 80, pal), alpha);
                surf.fill_circle(pill.x + 15.0, cy, 3.4, dot_col);

                let summary = self.telemetry_text();
                let right_pad = if privacy_active { 25.0 } else { 13.0 };
                surf.draw_text(
                    &summary,
                    RectF::new(pill.x + 24.0, pill.y, pill.w - 24.0 - right_pad, pill.h),
                    TextStyle::new(font_sz, true, ink, 0),
                );
                draw_privacy_dots(
                    surf,
                    pill.x + pill.w - 13.0,
                    cy,
                    snap.mic_active,
                    snap.cam_active,
                    pal,
                    alpha,
                );
            }
            IdleMode::Productivity => {
                let (done, total) = self.data.todo.counts(today);
                let green = with_alpha(pal.accent_green, alpha);
                let cyan = with_alpha(pal.accent_blue, alpha);
                let todo_str = format!("{done}/{total}");
                let todo_w = surf.measure_text_width(&todo_str, font_sz, true);
                surf.fill_circle(
                    pill.x + 15.0,
                    cy,
                    3.4,
                    if total > 0 && done == total {
                        green
                    } else {
                        ink_soft
                    },
                );
                surf.draw_text(
                    &todo_str,
                    RectF::new(pill.x + 23.0, pill.y, todo_w + 4.0, pill.h),
                    TextStyle::new(font_sz, true, ink, 0),
                );
                let drop_x = pill.x + 23.0 + todo_w + 11.0;
                draw_droplet(surf, drop_x, cy, 4.2, cyan);
                surf.draw_text(
                    &format!("{}%", self.data.water.percent()),
                    RectF::new(drop_x + 8.0, pill.y, 44.0, pill.h),
                    TextStyle::new(font_sz, true, ink, 0),
                );
                draw_privacy_dots(
                    surf,
                    pill.x + pill.w - 13.0,
                    cy,
                    snap.mic_active,
                    snap.cam_active,
                    pal,
                    alpha,
                );
            }
            IdleMode::Minimal => {
                let cpu_temp = self
                    .metrics
                    .iter()
                    .find(|m| m.kind == Kind::Cpu)
                    .and_then(|c| c.temperature);
                let dot_col = with_alpha(thermal_color(cpu_temp, 80, pal), alpha);
                if privacy_active {
                    surf.fill_circle(pill.x + 17.0, cy, 3.2, dot_col);
                    surf.fill_round(RectF::new(pill.x + 26.0, cy - 1.75, 26.0, 3.5), 1.75, muted);
                    draw_privacy_dots(
                        surf,
                        pill.x + pill.w - 15.0,
                        cy,
                        snap.mic_active,
                        snap.cam_active,
                        pal,
                        alpha,
                    );
                } else {
                    surf.fill_circle(pill.x + 18.0, cy, 3.0, dot_col);
                    surf.fill_round(
                        RectF::new(pill.x + 27.0, cy - 1.75, pill.w - 43.0, 3.5),
                        1.75,
                        track,
                    );
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_expanded(
        &mut self,
        surf: &Surface<'_>,
        pal: &Palette,
        pill: RectF,
        st: &SystemTime,
        today: u32,
        alpha: f32,
        actions: &mut Vec<(RectF, Action)>,
    ) {
        let slots = self.enabled_slots();
        let ink = with_alpha(pal.ink, alpha);
        let muted = with_alpha(pal.muted, alpha);
        let snap = &self.media.snapshot;
        let active_interact = self.expand.value > 0.85;
        let lift = lerp(-6.0, 0.0, alpha);

        let media_idx = slots.iter().position(|&s| s == NavSlot::Page(Page::Media));
        let media_blend = media_idx
            .map(|idx| (1.0 - (self.slide.value - idx as f32).abs()).clamp(0.0, 1.0))
            .unwrap_or(0.0);

        let (dock_rect, active_idx, vis_start, slot_rects, overflow) =
            self.dock_layout_slots(surf, pill, lift);
        self.dock_bounds = dock_rect;
        let dock_x = dock_rect.x;
        let dock_y = dock_rect.y;
        let dock_w = dock_rect.w;
        let dock_h = dock_rect.h;
        let dock_cy = dock_y + dock_h * 0.5;

        let nav_layout: Vec<(f32, f32)> = slot_rects
            .iter()
            .map(|(_, r)| (r.x + r.w * 0.5, r.w))
            .collect();
        let rel = active_idx
            .saturating_sub(vis_start)
            .min(nav_layout.len().saturating_sub(1));
        self.dock_nav
            .sync_layout(&nav_layout, dock_cy, dock_h * 0.5, rel);

        // Frosted QuartzCore Top Floating Glass Dock
        surf.fill_round(dock_rect, 13.0, with_alpha(pal.card_bg, alpha * 0.90));

        // Single Liquid Glass droplet indicator drawn BEFORE labels (zero double-glass overlap).
        if !slot_rects.is_empty() {
            let (center, half_w, drop_shown, velocity, _) = self.dock_nav.state();
            let clamped_hw = (half_w - 1.5).clamp(16.0, ((dock_w - 6.0) * 0.5).max(16.0));
            let clamped_hh = (dock_h - 5.0) * 0.5;
            let clamped_cx = center[0].clamp(
                dock_x + 3.0 + clamped_hw,
                (dock_x + dock_w - 3.0 - clamped_hw).max(dock_x + 3.0 + clamped_hw),
            );
            surf.draw_quartz_droplet(
                dock_rect,
                [clamped_cx, dock_cy],
                [clamped_hw, clamped_hh],
                clamped_hh,
                drop_shown,
                velocity,
                pal.light,
                alpha,
            );
            surf.set_round_clip(pill, EXPANDED_R);
        }

        if overflow {
            let l_rect = RectF::new(dock_x + 2.0, dock_y, 14.0, dock_h);
            let r_rect = RectF::new(dock_x + dock_w - 16.0, dock_y, 14.0, dock_h);
            let (hl, _) = self.interaction_state(Action::PrevPage);
            let (hr, _) = self.interaction_state(Action::NextPage);
            draw_chevron(
                surf,
                l_rect.x + 8.0,
                dock_cy,
                -1.0,
                mix_color(muted, ink, hl),
            );
            draw_chevron(
                surf,
                r_rect.x + 6.0,
                dock_cy,
                1.0,
                mix_color(muted, ink, hr),
            );
            if active_interact {
                actions.push((l_rect, Action::PrevPage));
                actions.push((r_rect, Action::NextPage));
            }
        }

        for &(global_idx, rect) in &slot_rects {
            let slot = slots[global_idx];
            let active = global_idx == active_idx;
            let act = Action::SelectSlot(global_idx);
            let (ht, _) = self.interaction_state(act);
            let col = if active {
                ink
            } else {
                mix_color(muted, ink, ht)
            };
            surf.draw_text(
                slot.label(&self.plugin_entries),
                rect,
                TextStyle::new(10.5, active, col, 1),
            );
            if active_interact {
                actions.push((rect, act));
            }
        }

        draw_privacy_dots(
            surf,
            pill.x + pill.w - if media_blend > 0.02 { 54.0 } else { 20.0 },
            dock_cy,
            snap.mic_active,
            snap.cam_active,
            pal,
            alpha,
        );
        if media_blend > 0.02 {
            let wave_col = if snap.playing {
                with_alpha(pal.ink_soft, alpha * media_blend)
            } else {
                with_alpha(pal.muted, alpha * media_blend)
            };
            draw_waveform(
                surf,
                pill.x + pill.w - 42.0,
                dock_cy,
                &self.wave,
                12.0,
                wave_col,
            );
        }

        let view_clip = RectF::new(
            pill.x + 16.0,
            pill.y + 40.0 + lift,
            pill.w - 32.0,
            pill.h - 48.0,
        );
        surf.set_round_clip(view_clip, 8.0);
        for (slot_idx, &slot) in slots.iter().enumerate() {
            let offset = slot_idx as f32 - self.slide.value;
            if offset.abs() >= 0.99 {
                continue;
            }
            let page_alpha = alpha * (1.0 - offset.abs()).clamp(0.0, 1.0);
            if page_alpha <= 0.02 {
                continue;
            }
            let ox = view_clip.x + offset * (view_clip.w + 18.0);
            let oy = view_clip.y;
            let interact = active_interact && slot == self.active_slot;
            match slot {
                NavSlot::Page(Page::System) => {
                    self.draw_system(surf, pal, (ox, oy), page_alpha, interact, actions)
                }
                NavSlot::Page(Page::Media) => self.draw_media(
                    surf,
                    pal,
                    view_clip,
                    (ox, oy),
                    page_alpha,
                    interact,
                    actions,
                ),
                NavSlot::Page(Page::Shelf) => {
                    self.draw_shelf(surf, pal, (ox, oy), page_alpha, interact, actions)
                }
                NavSlot::Page(Page::Water) => {
                    self.draw_water(surf, pal, (ox, oy), st, page_alpha, interact, actions)
                }
                NavSlot::Page(Page::Pomodoro) => {
                    self.draw_pomodoro(surf, pal, (ox, oy), st, page_alpha, interact, actions)
                }
                NavSlot::Page(Page::Todo) => {
                    self.draw_todo(surf, pal, (ox, oy), today, page_alpha, interact, actions)
                }
                NavSlot::Plugin(plugin_idx) => self.draw_plugin_page(
                    surf,
                    pal,
                    (ox, oy),
                    plugin_idx,
                    page_alpha,
                    interact,
                    actions,
                ),
            }
        }
        surf.reset_clip();
        surf.set_round_clip(pill, EXPANDED_R);
    }

    fn draw_system(
        &self,
        surf: &Surface<'_>,
        pal: &Palette,
        (ox, oy): (f32, f32),
        alpha: f32,
        interact: bool,
        actions: &mut Vec<(RectF, Action)>,
    ) {
        let ink = with_alpha(pal.ink, alpha);
        let ink_soft = with_alpha(pal.ink_soft, alpha);
        let muted = with_alpha(pal.muted, alpha);
        let track = with_alpha(pal.track, alpha);

        let cpu = self.metrics.iter().find(|m| m.kind == Kind::Cpu);
        let ram = self.metrics.iter().find(|m| m.kind == Kind::Ram);

        // Left Top: CPU glass bento card with live sparkline
        let cpu_card = RectF::new(ox, oy + 4.0, 210.0, 90.0);
        draw_glass_tile(surf, pal, cpu_card, 14.0, alpha);
        surf.draw_text(
            "CPU",
            RectF::new(cpu_card.x + 12.0, cpu_card.y + 7.0, 44.0, 16.0),
            TextStyle::new(10.8, true, muted, 0),
        );
        let cpu_val = cpu.map(|c| c.value.as_str()).unwrap_or("--%");
        let cpu_pct = parse_pct(cpu_val).unwrap_or(0.0);
        let cpu_temp = cpu.and_then(|c| c.temperature);
        let cpu_alert_col = if cpu_temp.is_some_and(|t| t >= 90) || cpu_pct >= 92.0 {
            with_alpha(pal.accent_red, alpha)
        } else if cpu_temp.is_some_and(|t| t >= 80) || cpu_pct >= 80.0 {
            with_alpha(pal.accent_amber, alpha)
        } else {
            ink_soft
        };

        surf.draw_text(
            cpu_val,
            RectF::new(cpu_card.x + 12.0, cpu_card.y + 23.0, 82.0, 30.0),
            TextStyle::new(22.0, true, ink, 0),
        );
        if let Some((hog_name, hog_pct)) = cpu.and_then(|c| c.hog.as_ref()) {
            surf.draw_text(
                &format!("Peak · {hog_name} ({hog_pct}%)"),
                RectF::new(
                    cpu_card.x + 12.0,
                    cpu_card.y + 68.0,
                    cpu_card.w - 22.0,
                    16.0,
                ),
                TextStyle::new(9.5, true, cpu_alert_col, 0),
            );
        }
        let fahr = settings::get().temp_fahrenheit;
        if let Some(t) = cpu_temp {
            let is_warm = t >= 80;
            let tcol = if is_warm {
                with_alpha(thermal_color(Some(t), 80, pal), alpha)
            } else {
                ink_soft
            };
            let badge = RectF::new(cpu_card.x + cpu_card.w - 56.0, cpu_card.y + 8.0, 46.0, 18.0);
            surf.fill_round(
                badge,
                9.0,
                if is_warm {
                    with_alpha(tcol, 0.16)
                } else {
                    with_alpha(pal.btn_bg, alpha)
                },
            );
            surf.draw_text(
                &settings::format_temp(t, fahr),
                badge,
                TextStyle::new(10.5, true, tcol, 1),
            );
        } else if crate::ui::needs_temp_setup() {
            let btn = RectF::new(cpu_card.x + cpu_card.w - 86.0, cpu_card.y + 7.0, 76.0, 18.0);
            let dbtn = self.draw_liquid_button(
                surf,
                pal,
                btn,
                9.0,
                Action::EnableTempSensor,
                false,
                alpha,
                None,
            );
            surf.draw_text("Enable Temp", dbtn, TextStyle::new(9.5, true, ink_soft, 1));
            if interact {
                actions.push((btn, Action::EnableTempSensor));
            }
        }

        // CPU Sparkline
        let gx = cpu_card.x + 92.0;
        let gy = cpu_card.y + 30.0;
        let gw = cpu_card.w - 104.0;
        let gh = if cpu.and_then(|c| c.hog.as_ref()).is_some() {
            34.0
        } else {
            48.0
        };
        let step_x = gw / ((self.cpu_hist.len() - 1) as f32);
        let mut pts = Vec::with_capacity(self.cpu_hist.len());
        for (idx, &v) in self.cpu_hist.iter().enumerate() {
            let x = gx + (idx as f32) * step_x;
            let y = gy + gh - (v.clamp(0.0, 100.0) / 100.0) * (gh - 4.0) - 2.0;
            pts.push((x, y));
        }
        let mut area = Vec::with_capacity(pts.len() + 2);
        area.push((gx, gy + gh));
        area.extend_from_slice(&pts);
        area.push((gx + gw, gy + gh));
        surf.fill_polygon(
            &area,
            with_alpha(cpu_alert_col, if pal.light { 0.14 } else { 0.16 }),
        );
        surf.draw_polyline(&pts, 1.7, cpu_alert_col);

        // Left Bottom: RAM glass bento card
        let ram_card = RectF::new(ox, oy + 100.0, 210.0, 42.0);
        draw_glass_tile(surf, pal, ram_card, 12.0, alpha);
        surf.draw_text(
            "Memory",
            RectF::new(ram_card.x + 12.0, ram_card.y + 6.0, 56.0, 16.0),
            TextStyle::new(10.5, true, muted, 0),
        );
        let ram_val = ram
            .map(format_ram_summary)
            .unwrap_or_else(|| "-- GB".into());
        surf.draw_text(
            &ram_val,
            RectF::new(ram_card.x + 68.0, ram_card.y + 6.0, ram_card.w - 80.0, 16.0),
            TextStyle::new(11.0, true, ink, 2),
        );
        let ram_bar = RectF::new(ram_card.x + 12.0, ram_card.y + 28.0, ram_card.w - 24.0, 5.0);
        surf.fill_round(ram_bar, 2.5, track);
        let ram_frac = ram
            .and_then(|r| r.capacity)
            .map(|c| c as f32)
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        let ram_col = if ram_frac >= 0.92 {
            with_alpha(pal.accent_red, alpha)
        } else if ram_frac >= 0.82 {
            with_alpha(pal.accent_amber, alpha)
        } else {
            ink_soft
        };
        if ram_frac > 0.01 {
            surf.fill_round(
                RectF::new(
                    ram_bar.x,
                    ram_bar.y,
                    (ram_bar.w * ram_frac).max(5.0),
                    ram_bar.h,
                ),
                2.5,
                ram_col,
            );
        }

        // Right Column: GPU & Disk glass bento cards (138px height)
        let extras: Vec<&Metric> = self
            .metrics
            .iter()
            .filter(|m| matches!(m.kind, Kind::Gpu | Kind::Disk))
            .take(4)
            .collect();
        if extras.is_empty() {
            let empty = RectF::new(ox + 218.0, oy + 4.0, 214.0, 138.0);
            draw_glass_tile(surf, pal, empty, 14.0, alpha);
            surf.draw_text(
                "No GPU / Disk sensors",
                empty,
                TextStyle::new(11.2, false, muted, 1),
            );
        } else {
            let n = extras.len();
            let gap = if n <= 2 { 7.0 } else { 5.0 };
            let row_h =
                ((138.0 - gap * (n.saturating_sub(1) as f32)) / (n as f32)).clamp(29.0, 65.0);
            for (i, m) in extras.iter().enumerate() {
                let row = RectF::new(
                    ox + 218.0,
                    oy + 4.0 + (i as f32) * (row_h + gap),
                    214.0,
                    row_h,
                );
                draw_glass_tile(
                    surf,
                    pal,
                    row,
                    if row_h >= 40.0 { 12.0 } else { 10.0 },
                    alpha,
                );
                let warm = if m.kind == Kind::Disk { 65 } else { 80 };
                let pct = parse_pct(&m.value).unwrap_or(0.0);
                let status_col = match m.temperature {
                    Some(t) if t >= warm + 10 => with_alpha(pal.accent_red, alpha),
                    Some(t) if t >= warm => with_alpha(pal.accent_amber, alpha),
                    _ if pct >= 90.0 => with_alpha(pal.accent_amber, alpha),
                    _ => ink_soft,
                };
                let label = if m.kind == Kind::Gpu {
                    short_gpu_name(&m.detail)
                } else {
                    m.label.clone()
                };
                surf.fill_circle(row.x + 11.0, row.y + row.h * 0.5, 2.8, status_col);
                surf.draw_text(
                    &label,
                    RectF::new(row.x + 19.0, row.y, 70.0, row.h),
                    TextStyle::new(if row_h >= 40.0 { 11.4 } else { 10.5 }, true, ink, 0),
                );
                let mini = RectF::new(row.x + 92.0, row.y + row.h * 0.5 - 2.25, 40.0, 4.5);
                surf.fill_round(mini, 2.25, track);
                let bar_frac = (pct / 100.0).clamp(0.0, 1.0);
                if bar_frac > 0.01 {
                    surf.fill_round(
                        RectF::new(mini.x, mini.y, (mini.w * bar_frac).max(4.5), mini.h),
                        2.25,
                        status_col,
                    );
                }
                if let Some(t) = m.temperature {
                    surf.draw_text(
                        &m.value,
                        RectF::new(row.x + 134.0, row.y, 34.0, row.h),
                        TextStyle::new(10.8, true, ink, 2),
                    );
                    let tcol = if t >= warm { status_col } else { muted };
                    surf.draw_text(
                        &settings::format_temp(t, fahr),
                        RectF::new(row.x + 168.0, row.y, 38.0, row.h),
                        TextStyle::new(10.4, true, tcol, 2),
                    );
                } else if let Some(cap) = m.capacity {
                    surf.draw_text(
                        &m.value,
                        RectF::new(row.x + 139.0, row.y, 34.0, row.h),
                        TextStyle::new(10.8, true, ink, 2),
                    );
                    surf.draw_text(
                        &format!("{:.0}%", cap * 100.0),
                        RectF::new(row.x + 173.0, row.y, 33.0, row.h),
                        TextStyle::new(10.0, false, muted, 2),
                    );
                } else {
                    surf.draw_text(
                        &m.value,
                        RectF::new(row.x + 139.0, row.y, 67.0, row.h),
                        TextStyle::new(10.8, true, ink, 2),
                    );
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_media(
        &self,
        surf: &Surface<'_>,
        pal: &Palette,
        view_clip: RectF,
        (ox, oy): (f32, f32),
        alpha: f32,
        interact: bool,
        actions: &mut Vec<(RectF, Action)>,
    ) {
        let cfg = settings::get();
        let snap = &self.media.snapshot;
        let ink = with_alpha(pal.ink, alpha);
        let ink_soft = with_alpha(pal.ink_soft, alpha);
        let muted = with_alpha(pal.muted, alpha);
        let track = with_alpha(pal.track, alpha);
        let show_lyrics_bar = cfg.lyrics_enabled && snap.has_session;
        let restore = Some((view_clip, 8.0));

        let art_sz = if show_lyrics_bar { 52.0 } else { 62.0 };
        let art_rect = RectF::new(ox + 6.0, oy + 6.0, art_sz, art_sz);
        if let Some(img) = self.art.as_ref().filter(|_| alpha > 0.05) {
            surf.draw_soft_shadow(art_rect, 12.0, with_alpha(pal.shadow_outer, alpha * 0.75));
            surf.draw_image_round(img, art_rect, 12.0, restore);
        } else {
            surf.fill_round(art_rect, 12.0, with_alpha(pal.card_bg, alpha));
            draw_note_icon(
                surf,
                art_rect.x + art_sz * 0.5,
                art_rect.y + art_sz * 0.5,
                if show_lyrics_bar { 1.05 } else { 1.25 },
                muted,
            );
        }

        let title = if !snap.has_session || snap.title.is_empty() {
            "No Media Playing"
        } else {
            &snap.title
        };
        let sub_text: String = if self.track_copied_active() {
            "Copied track name".into()
        } else if !snap.has_session || snap.artist.is_empty() {
            "Play audio in Spotify, Chrome, or Edge".into()
        } else {
            snap.artist.clone()
        };
        let sub_col = if self.track_copied_active() {
            with_alpha(pal.accent_green, alpha)
        } else {
            muted
        };
        let text_x = art_rect.x + art_sz + 14.0;
        let text_w = 432.0 - (text_x - ox) - 10.0;
        let title_rect = RectF::new(
            text_x,
            oy + if show_lyrics_bar { 8.0 } else { 14.0 },
            text_w,
            22.0,
        );
        let artist_rect = RectF::new(
            text_x,
            oy + if show_lyrics_bar { 31.0 } else { 38.0 },
            text_w,
            18.0,
        );
        surf.draw_text(title, title_rect, TextStyle::new(14.5, true, ink, 0));
        surf.draw_text(
            &sub_text,
            artist_rect,
            TextStyle::new(11.5, false, sub_col, 0),
        );
        if interact && snap.has_session && !snap.title.is_empty() {
            actions.push((
                RectF::new(text_x, title_rect.y, text_w, 38.0),
                Action::MediaCopyTrack,
            ));
        }

        if show_lyrics_bar {
            let lbox = RectF::new(ox + 6.0, oy + 65.0, 420.0, 22.0);
            surf.fill_round(lbox, 9.0, with_alpha(pal.card_bg, alpha));
            let (ltxt, lcol) = match self.lyrics.status() {
                LyricsStatus::Synced(lines) => {
                    let (pos_ms, _, _) = snap.timeline_ms();
                    match active_lyric(lines, pos_ms) {
                        Some((_, cur, _)) if !cur.is_empty() => (cur, ink),
                        Some((_, _, Some(next))) => (next, muted),
                        _ => ("♪ ...", muted),
                    }
                }
                LyricsStatus::Loading => ("Searching synced lyrics...", muted),
                LyricsStatus::Instrumental => ("♪ Instrumental", muted),
                LyricsStatus::NotFound | LyricsStatus::Idle => ("No synced lyrics found", muted),
            };
            surf.draw_text(
                ltxt,
                RectF::new(lbox.x + 10.0, lbox.y, lbox.w - 20.0, lbox.h),
                TextStyle::new(11.2, true, lcol, 1),
            );
        }

        let (elapsed, remaining, frac) = snap.timeline_secs();
        let tl_y = oy + if show_lyrics_bar { 91.0 } else { 79.0 };
        surf.draw_text(
            &format_mmss(elapsed),
            RectF::new(ox + 6.0, tl_y, 44.0, 18.0),
            TextStyle::new(11.5, false, ink_soft, 0),
        );
        let bar = RectF::new(ox + 52.0, tl_y + 7.0, 328.0, 4.5);
        surf.fill_round(bar, 2.25, track);
        if frac > 0.005 {
            surf.fill_round(
                RectF::new(bar.x, bar.y, (bar.w * frac).max(4.5), bar.h),
                2.25,
                ink,
            );
        }
        let (seek_h, _) = self.interaction_state(Action::MediaSeek(bar));
        let seek_drag = matches!(self.active_slider, Some(ActiveSlider::Seek(_)));
        let thumb_r = 3.8 + 1.8 * if seek_drag { 1.0 } else { seek_h };
        surf.fill_circle(bar.x + bar.w * frac, bar.y + 2.25, thumb_r, ink);
        surf.draw_text(
            &format!("-{}", format_mmss(remaining)),
            RectF::new(ox + 382.0, tl_y, 44.0, 18.0),
            TextStyle::new(11.5, false, ink_soft, 2),
        );
        if interact && snap.end_100ns > snap.start_100ns {
            actions.push((
                RectF::new(bar.x - 4.0, tl_y - 4.0, bar.w + 8.0, 24.0),
                Action::MediaSeek(bar),
            ));
        }

        let ctrl_y = oy + if show_lyrics_bar { 112.0 } else { 107.0 };
        if self.volume_open {
            if !snap.audio_device_name.is_empty() {
                let prefix = if snap.audio_is_headphone {
                    "Headphones"
                } else {
                    "Output"
                };
                let dev_lbl = format!("{prefix} · {}", clean_audio_name(&snap.audio_device_name));
                surf.draw_text(
                    &dev_lbl,
                    RectF::new(ox + 68.0, ctrl_y - 3.0, 250.0, 13.0),
                    TextStyle::new(9.2, false, muted, 0),
                );
            }
            let mute_btn = RectF::new(ox + 26.0, ctrl_y + 1.0, 32.0, 30.0);
            let (hm, _) = self.interaction_state(Action::VolMute);
            if hm > 0.02 {
                self.draw_liquid_button(
                    surf,
                    pal,
                    mute_btn,
                    15.0,
                    Action::VolMute,
                    false,
                    alpha * hm,
                    restore,
                );
            }
            let spk_col = if snap.muted {
                with_alpha(pal.accent_red, alpha)
            } else {
                ink
            };
            draw_speaker_icon(
                surf,
                mute_btn.x + 16.0,
                mute_btn.y + 15.0,
                snap.muted,
                spk_col,
            );

            let vbar = RectF::new(ox + 68.0, ctrl_y + 14.0, 250.0, 5.0);
            surf.fill_round(vbar, 2.5, track);
            let vol = if snap.muted { 0.0 } else { snap.volume };
            if vol > 0.01 {
                surf.fill_round(
                    RectF::new(vbar.x, vbar.y, (vbar.w * vol).max(5.0), vbar.h),
                    2.5,
                    ink,
                );
            }
            let (vol_h, _) = self.interaction_state(Action::VolSlider(vbar));
            let vol_drag = matches!(self.active_slider, Some(ActiveSlider::Volume(_)));
            let vthumb_r = 4.6 + 1.8 * if vol_drag { 1.0 } else { vol_h };
            surf.fill_circle(vbar.x + vbar.w * vol, vbar.y + 2.5, vthumb_r, ink);
            surf.draw_text(
                &format!("{}%", (vol * 100.0).round() as u32),
                RectF::new(ox + 328.0, ctrl_y + 6.0, 42.0, 20.0),
                TextStyle::new(11.8, true, ink, 0),
            );
            let close_btn = RectF::new(ox + 374.0, ctrl_y + 2.0, 30.0, 26.0);
            let (hc, _) = self.interaction_state(Action::MediaToggleVol);
            if hc > 0.02 {
                self.draw_liquid_button(
                    surf,
                    pal,
                    close_btn,
                    13.0,
                    Action::MediaToggleVol,
                    false,
                    alpha * hc,
                    restore,
                );
            }
            surf.draw_text(
                "✕",
                close_btn,
                TextStyle::new(13.0, false, mix_color(muted, ink, hc), 1),
            );
            if interact {
                actions.push((mute_btn, Action::VolMute));
                actions.push((
                    RectF::new(vbar.x - 4.0, ctrl_y, vbar.w + 8.0, 32.0),
                    Action::VolSlider(vbar),
                ));
                actions.push((close_btn, Action::MediaToggleVol));
            }
        } else {
            let lyric_btn = RectF::new(ox + 44.0, ctrl_y + 2.0, 34.0, 26.0);
            let (hl, pl) = self.interaction_state(Action::MediaToggleLyrics);
            let lcol = if cfg.lyrics_enabled {
                ink
            } else {
                mix_color(muted, ink, hl)
            };
            if cfg.lyrics_enabled || hl > 0.02 || pl > 0.02 {
                surf.draw_quartz_pill(
                    lyric_btn,
                    9.5,
                    if cfg.lyrics_enabled {
                        pal.seg_active_bg
                    } else {
                        0
                    },
                    hl.max(if cfg.lyrics_enabled { 0.45 } else { 0.0 }),
                    pl,
                    cfg.lyrics_enabled,
                    pal.light,
                    alpha * if cfg.lyrics_enabled { 1.0 } else { hl.max(pl) },
                    restore,
                );
            }
            draw_quote_icon(surf, lyric_btn.x + 17.0, lyric_btn.y + 13.0, lcol);

            let prev_btn = RectF::new(ox + 136.0, ctrl_y, 38.0, 30.0);
            let play_btn = RectF::new(ox + 196.0, ctrl_y - 2.0, 34.0, 34.0);
            let next_btn = RectF::new(ox + 250.0, ctrl_y, 38.0, 30.0);
            let (hp, _) = self.interaction_state(Action::MediaPrev);
            if hp > 0.02 {
                self.draw_liquid_button(
                    surf,
                    pal,
                    RectF::new(prev_btn.x + 4.0, prev_btn.y, 30.0, 30.0),
                    15.0,
                    Action::MediaPrev,
                    false,
                    alpha * hp,
                    restore,
                );
            }
            draw_skip_icon(
                surf,
                prev_btn.x + 19.0,
                prev_btn.y + 15.0,
                -1.0,
                mix_color(ink_soft, ink, hp),
            );
            let dplay = self.draw_liquid_button(
                surf,
                pal,
                play_btn,
                17.0,
                Action::MediaToggle,
                false,
                alpha,
                restore,
            );
            draw_play_pause_icon(
                surf,
                dplay.x + dplay.w * 0.5,
                dplay.y + dplay.h * 0.5,
                snap.playing,
                ink,
            );
            let (hn, _) = self.interaction_state(Action::MediaNext);
            if hn > 0.02 {
                self.draw_liquid_button(
                    surf,
                    pal,
                    RectF::new(next_btn.x + 4.0, next_btn.y, 30.0, 30.0),
                    15.0,
                    Action::MediaNext,
                    false,
                    alpha * hn,
                    restore,
                );
            }
            draw_skip_icon(
                surf,
                next_btn.x + 19.0,
                next_btn.y + 15.0,
                1.0,
                mix_color(ink_soft, ink, hn),
            );

            let vol_btn = RectF::new(ox + 350.0, ctrl_y + 2.0, 34.0, 26.0);
            let (hv, _) = self.interaction_state(Action::MediaToggleVol);
            if hv > 0.02 {
                self.draw_liquid_button(
                    surf,
                    pal,
                    vol_btn,
                    13.0,
                    Action::MediaToggleVol,
                    false,
                    alpha * hv,
                    restore,
                );
            }
            let vcol = if snap.muted {
                with_alpha(pal.accent_red, alpha)
            } else {
                mix_color(ink_soft, ink, hv)
            };
            draw_speaker_icon(surf, vol_btn.x + 17.0, vol_btn.y + 13.0, snap.muted, vcol);

            if interact {
                actions.push((lyric_btn, Action::MediaToggleLyrics));
                actions.push((prev_btn, Action::MediaPrev));
                actions.push((play_btn, Action::MediaToggle));
                actions.push((next_btn, Action::MediaNext));
                actions.push((vol_btn, Action::MediaToggleVol));
            }
        }
    }

    fn draw_shelf(
        &self,
        surf: &Surface<'_>,
        pal: &Palette,
        (ox, oy): (f32, f32),
        alpha: f32,
        interact: bool,
        actions: &mut Vec<(RectF, Action)>,
    ) {
        let shelf = &self.data.shelf;
        let ink = with_alpha(pal.ink, alpha);
        let ink_soft = with_alpha(pal.ink_soft, alpha);
        let muted = with_alpha(pal.muted, alpha);
        let cta_fg = with_alpha(pal.cta_fg, alpha);
        let btn_bg = with_alpha(pal.btn_bg, alpha);
        let zone = RectF::new(ox, oy + 4.0, 432.0, 138.0);

        if shelf.items.is_empty() {
            draw_glass_tile(surf, pal, zone, 16.0, alpha);
            if shelf.drag_over {
                surf.stroke_round(zone, 16.0, 1.4, ink, true);
            }
            draw_tray_icon(
                surf,
                zone.x + zone.w * 0.5,
                zone.y + 30.0,
                if shelf.drag_over { ink } else { ink_soft },
            );
            surf.draw_text(
                if shelf.drag_over {
                    "Drop files to keep on Shelf"
                } else {
                    "Drop files here or pick from disk"
                },
                RectF::new(zone.x + 12.0, zone.y + 52.0, zone.w - 24.0, 20.0),
                TextStyle::new(13.0, true, ink, 1),
            );
            surf.draw_text(
                "Drag out to any app anytime · click to open",
                RectF::new(zone.x + 12.0, zone.y + 72.0, zone.w - 24.0, 16.0),
                TextStyle::new(10.8, false, muted, 1),
            );

            let add_btn = RectF::new(zone.x + 114.0, zone.y + 98.0, 110.0, 26.0);
            let paste_btn = RectF::new(zone.x + 234.0, zone.y + 98.0, 86.0, 26.0);
            let dadd = self.draw_liquid_button(
                surf,
                pal,
                add_btn,
                13.0,
                Action::ShelfAddFiles,
                true,
                alpha,
                None,
            );
            let dpaste = self.draw_liquid_button(
                surf,
                pal,
                paste_btn,
                13.0,
                Action::ShelfPaste,
                false,
                alpha,
                None,
            );
            surf.draw_text("+ Add Files", dadd, TextStyle::new(11.0, true, cta_fg, 1));
            surf.draw_text("Paste", dpaste, TextStyle::new(11.0, false, ink, 1));
            if interact {
                actions.push((zone, Action::ShelfAddFiles));
                actions.push((add_btn, Action::ShelfAddFiles));
                actions.push((paste_btn, Action::ShelfPaste));
            }
            return;
        }

        let status = if self.copied_active() {
            "Copied to clipboard"
        } else {
            "Drag row out · click to open"
        };
        let status_col = if self.copied_active() {
            with_alpha(pal.accent_green, alpha)
        } else {
            muted
        };
        let unit = if shelf.items.len() == 1 {
            "item"
        } else {
            "items"
        };
        surf.draw_text(
            &format!("{} {unit} · {status}", shelf.items.len()),
            RectF::new(ox + 4.0, oy + 4.0, 232.0, 24.0),
            TextStyle::new(11.0, false, status_col, 0),
        );

        let add_btn = RectF::new(ox + 240.0, oy + 4.0, 58.0, 24.0);
        let drag_all_btn = RectF::new(ox + 303.0, oy + 4.0, 70.0, 24.0);
        let clear_btn = RectF::new(ox + 378.0, oy + 4.0, 54.0, 24.0);
        let dadd = self.draw_liquid_button(
            surf,
            pal,
            add_btn,
            12.0,
            Action::ShelfAddFiles,
            false,
            alpha,
            None,
        );
        let ddrag = self.draw_liquid_button(
            surf,
            pal,
            drag_all_btn,
            12.0,
            Action::ShelfDragAll,
            true,
            alpha,
            None,
        );
        let dclr = self.draw_liquid_button(
            surf,
            pal,
            clear_btn,
            12.0,
            Action::ShelfClear,
            false,
            alpha,
            None,
        );
        surf.draw_text("+ Add", dadd, TextStyle::new(10.8, true, ink, 1));
        surf.draw_text("Drag All", ddrag, TextStyle::new(10.8, true, cta_fg, 1));
        surf.draw_text("Clear", dclr, TextStyle::new(10.5, false, muted, 1));
        if interact {
            actions.push((add_btn, Action::ShelfAddFiles));
            actions.push((drag_all_btn, Action::ShelfDragAll));
            actions.push((clear_btn, Action::ShelfClear));
        }

        let box_rect = RectF::new(ox, oy + 32.0, 432.0, 110.0);
        draw_glass_tile(surf, pal, box_rect, 15.0, alpha);
        if shelf.drag_over {
            surf.stroke_round(box_rect, 15.0, 1.3, ink, true);
        }

        let max_scroll = shelf.items.len().saturating_sub(3);
        let start = shelf.scroll.min(max_scroll);
        for (slot, item) in shelf.items[start..].iter().take(3).enumerate() {
            let idx = start + slot;
            let row = RectF::new(
                box_rect.x + 8.0,
                box_rect.y + 9.0 + (slot as f32) * 32.0,
                box_rect.w - 16.0,
                28.0,
            );
            let (hr, _) = self.interaction_state(Action::ShelfItem(idx));
            if hr > 0.02 {
                surf.fill_round(row, 8.0, with_alpha(pal.card_hover, alpha * hr));
            }
            let badge = RectF::new(row.x + 4.0, row.y + 5.0, 38.0, 18.0);
            surf.fill_round(badge, 5.0, btn_bg);
            surf.draw_text(
                &item.ext,
                badge,
                TextStyle::new(
                    9.0,
                    true,
                    if item.is_dir {
                        with_alpha(pal.accent_amber, alpha)
                    } else {
                        ink_soft
                    },
                    1,
                ),
            );

            surf.draw_text(
                &item.name,
                RectF::new(row.x + 48.0, row.y, row.w - 164.0, row.h),
                TextStyle::new(12.0, false, ink, 0),
            );
            surf.draw_text(
                &item.size_label,
                RectF::new(row.x + row.w - 112.0, row.y, 52.0, row.h),
                TextStyle::new(11.0, false, muted, 2),
            );

            let copy_btn = RectF::new(row.x + row.w - 54.0, row.y + 4.0, 24.0, 20.0);
            let del_btn = RectF::new(row.x + row.w - 26.0, row.y + 4.0, 22.0, 20.0);
            let dcopy = self.draw_liquid_button(
                surf,
                pal,
                copy_btn,
                6.0,
                Action::ShelfCopy(idx),
                false,
                alpha,
                None,
            );
            draw_copy_icon(surf, dcopy.x + dcopy.w * 0.5, dcopy.y + dcopy.h * 0.5, ink);
            let (hd, _) = self.interaction_state(Action::ShelfDelete(idx));
            let del_col = mix_color(muted, with_alpha(pal.accent_red, alpha), hd);
            surf.draw_text("×", del_btn, TextStyle::new(13.0, false, del_col, 1));

            if interact {
                actions.push((
                    RectF::new(row.x, row.y, row.w - 58.0, row.h),
                    Action::ShelfItem(idx),
                ));
                actions.push((copy_btn, Action::ShelfCopy(idx)));
                actions.push((del_btn, Action::ShelfDelete(idx)));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_pomodoro(
        &self,
        surf: &Surface<'_>,
        pal: &Palette,
        (ox, oy): (f32, f32),
        st: &SystemTime,
        alpha: f32,
        interact: bool,
        actions: &mut Vec<(RectF, Action)>,
    ) {
        let p = &self.data.pomodoro;
        let ink = with_alpha(pal.ink, alpha);
        let ink_soft = with_alpha(pal.ink_soft, alpha);
        let muted = with_alpha(pal.muted, alpha);
        let track = with_alpha(pal.track, alpha);
        let bar_col = if p.running { ink } else { ink_soft };

        // Left Bento Glass Card: Timer + Concentric Ring + Phase Controls
        let left = RectF::new(ox, oy + 4.0, 212.0, 138.0);
        draw_glass_tile(surf, pal, left, 16.0, alpha);

        let sub = format!(
            "{} · {}/{}",
            p.phase.label(),
            p.session_index,
            p.sessions_per_round
        );
        surf.draw_text(
            &sub,
            RectF::new(left.x + 14.0, left.y + 10.0, 144.0, 18.0),
            TextStyle::new(11.2, true, muted, 0),
        );

        let ring_r = 16.0;
        let ring = RectF::new(
            left.x + left.w - 46.0,
            left.y + 10.0,
            ring_r * 2.0,
            ring_r * 2.0,
        );
        surf.stroke_circle(ring.x + ring_r, ring.y + ring_r, ring_r, 3.0, track);
        let prog = p.progress();
        if prog > 0.01 {
            surf.stroke_arc(ring, 3.0, -90.0, 360.0 * prog, bar_col);
        }

        let mins = p.remaining_secs / 60;
        let secs = p.remaining_secs % 60;
        surf.draw_text(
            &format!("{mins:02}:{secs:02}"),
            RectF::new(left.x + 12.0, left.y + 32.0, 152.0, 44.0),
            TextStyle::new(34.0, true, ink, 0),
        );

        if p.running {
            let fin = format!("Ends {}", p.finish_time_label(st.hour, st.minute));
            surf.draw_text(
                &fin,
                RectF::new(left.x + 14.0, left.y + 84.0, 184.0, 22.0),
                TextStyle::new(12.2, true, ink_soft, 0),
            );
        } else {
            let dec_btn = RectF::new(left.x + 14.0, left.y + 84.0, 22.0, 22.0);
            let inc_btn = RectF::new(left.x + 74.0, left.y + 84.0, 22.0, 22.0);
            let ddec = self.draw_liquid_button(
                surf,
                pal,
                dec_btn,
                11.0,
                Action::PomoAdjustMin(-5),
                false,
                alpha,
                None,
            );
            let dinc = self.draw_liquid_button(
                surf,
                pal,
                inc_btn,
                11.0,
                Action::PomoAdjustMin(5),
                false,
                alpha,
                None,
            );
            surf.draw_text("-", ddec, TextStyle::new(12.0, true, ink, 1));
            surf.draw_text("+", dinc, TextStyle::new(12.0, true, ink, 1));
            surf.draw_text(
                &format!("{}m", p.focus_min),
                RectF::new(left.x + 38.0, left.y + 84.0, 34.0, 22.0),
                TextStyle::new(12.0, true, ink, 1),
            );
            surf.draw_text(
                p.status_text(),
                RectF::new(left.x + 102.0, left.y + 84.0, 96.0, 22.0),
                TextStyle::new(11.8, false, muted, 0),
            );
            if interact {
                actions.push((dec_btn, Action::PomoAdjustMin(-5)));
                actions.push((inc_btn, Action::PomoAdjustMin(5)));
            }
        }

        let bar = RectF::new(left.x + 14.0, left.y + 118.0, left.w - 28.0, 5.0);
        surf.fill_round(bar, 2.5, track);
        if prog > 0.01 {
            surf.fill_round(
                RectF::new(bar.x, bar.y, (bar.w * prog).max(5.0), bar.h),
                2.5,
                bar_col,
            );
        }

        // Right Bento Glass Card: Transport & Round Progress
        let right = RectF::new(ox + 220.0, oy + 4.0, 212.0, 138.0);
        draw_glass_tile(surf, pal, right, 16.0, alpha);

        surf.draw_text(
            "Session",
            RectF::new(right.x + 14.0, right.y + 12.0, 62.0, 24.0),
            TextStyle::new(11.8, true, muted, 0),
        );

        let start_btn = RectF::new(right.x + 76.0, right.y + 10.0, 66.0, 28.0);
        let dstart = self.draw_liquid_button(
            surf,
            pal,
            start_btn,
            14.0,
            Action::PomoToggle,
            true,
            alpha,
            None,
        );
        surf.draw_text(
            if p.running { "Pause" } else { "Start" },
            dstart,
            TextStyle::new(11.5, true, with_alpha(pal.cta_fg, alpha), 1),
        );

        let skip_btn = RectF::new(right.x + 148.0, right.y + 11.0, 26.0, 26.0);
        let dskip = self.draw_liquid_button(
            surf,
            pal,
            skip_btn,
            13.0,
            Action::PomoSkip,
            false,
            alpha,
            None,
        );
        draw_skip_phase_icon(surf, dskip.x + dskip.w * 0.5, dskip.y + dskip.h * 0.5, ink);

        let reset_btn = RectF::new(right.x + 178.0, right.y + 11.0, 26.0, 26.0);
        let dreset = self.draw_liquid_button(
            surf,
            pal,
            reset_btn,
            13.0,
            Action::PomoReset,
            false,
            alpha,
            None,
        );
        draw_reset_icon(
            surf,
            dreset.x + dreset.w * 0.5,
            dreset.y + dreset.h * 0.5,
            ink,
        );

        if interact {
            actions.push((start_btn, Action::PomoToggle));
            actions.push((skip_btn, Action::PomoSkip));
            actions.push((reset_btn, Action::PomoReset));
        }

        let mut sx = right.x + 14.0;
        let sy = right.y + 64.0;
        for i in 1..=4 {
            let active = match p.phase {
                PomodoroPhase::Focus => i < p.session_index || (i == p.session_index && p.running),
                PomodoroPhase::ShortBreak => i <= p.session_index,
                PomodoroPhase::LongBreak => true,
            };
            let seg = RectF::new(sx, sy, 26.0, 6.0);
            surf.fill_round(seg, 3.0, if active { ink } else { track });
            let lbl_col = if i == p.session_index { ink } else { muted };
            surf.draw_text(
                &i.to_string(),
                RectF::new(sx, sy + 12.0, 26.0, 16.0),
                TextStyle::new(10.5, i == p.session_index, lbl_col, 1),
            );
            sx += 29.0;
            surf.fill_circle(sx + 2.5, sy + 3.0, 1.8, track);
            sx += 8.0;
        }
        let long_active = p.phase == PomodoroPhase::LongBreak;
        surf.fill_round(
            RectF::new(sx, sy, 30.0, 6.0),
            3.0,
            if long_active { ink } else { track },
        );
        surf.draw_text(
            "Long",
            RectF::new(sx - 5.0, sy + 12.0, 40.0, 16.0),
            TextStyle::new(10.0, long_active, if long_active { ink } else { muted }, 1),
        );

        let total_m = p.round_total_minutes();
        let hrs = total_m / 60;
        let rem_m = total_m % 60;
        let round_str = if hrs > 0 {
            format!("{hrs}h {rem_m}m · {} sessions", p.sessions_per_round)
        } else {
            format!("{rem_m}m · {} sessions", p.sessions_per_round)
        };
        surf.draw_text(
            &round_str,
            RectF::new(right.x + 14.0, right.y + 108.0, right.w - 28.0, 18.0),
            TextStyle::new(11.0, false, muted, 0),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_water(
        &self,
        surf: &Surface<'_>,
        pal: &Palette,
        (ox, oy): (f32, f32),
        st: &SystemTime,
        alpha: f32,
        interact: bool,
        actions: &mut Vec<(RectF, Action)>,
    ) {
        let w = &self.data.water;
        let ink = with_alpha(pal.ink, alpha);
        let muted = with_alpha(pal.muted, alpha);
        let cyan = with_alpha(pal.accent_blue, alpha);
        let track = with_alpha(pal.track, alpha);

        // Left Bento Glass Card: Hydration Intake + Horizontal Vessel + CTA
        let left = RectF::new(ox, oy + 4.0, 216.0, 138.0);
        draw_glass_tile(surf, pal, left, 16.0, alpha);

        let consumed_l = format!("{:.1}", w.consumed_ml as f32 / 1000.0);
        let goal_l = format!("/ {:.1} L", w.goal_ml as f32 / 1000.0);
        surf.draw_text(
            &consumed_l,
            RectF::new(left.x + 14.0, left.y + 8.0, 62.0, 38.0),
            TextStyle::new(32.0, true, ink, 0),
        );
        surf.draw_text(
            &goal_l,
            RectF::new(left.x + 74.0, left.y + 20.0, 62.0, 22.0),
            TextStyle::new(13.0, false, muted, 0),
        );

        let toggle = RectF::new(left.x + left.w - 54.0, left.y + 14.0, 40.0, 22.0);
        self.draw_liquid_switch(
            surf,
            pal,
            toggle,
            w.reminder_enabled,
            cyan,
            Action::WaterToggleRemind,
            alpha,
            None,
        );

        let pct = w.percent();
        let next_str = if w.reminder_enabled {
            format!("· Next {}", w.next_time_label(st.hour, st.minute))
        } else {
            "· Off".into()
        };
        surf.draw_text(
            "Today ·",
            RectF::new(left.x + 14.0, left.y + 52.0, 48.0, 18.0),
            TextStyle::new(11.0, false, muted, 0),
        );
        surf.draw_text(
            &format!("{pct}%"),
            RectF::new(left.x + 60.0, left.y + 52.0, 36.0, 18.0),
            TextStyle::new(11.0, true, cyan, 0),
        );
        surf.draw_text(
            &next_str,
            RectF::new(left.x + 94.0, left.y + 52.0, 112.0, 18.0),
            TextStyle::new(11.0, false, muted, 0),
        );

        // Translucent Horizontal Hydration Vessel (borderless meniscus)
        let vessel = RectF::new(left.x + 14.0, left.y + 76.0, left.w - 28.0, 12.0);
        surf.fill_round(vessel, 6.0, track);
        if w.fraction() > 0.01 {
            let fill_w = (vessel.w * w.fraction()).max(12.0);
            surf.fill_vertical_gradient(
                RectF::new(vessel.x, vessel.y, fill_w, vessel.h),
                6.0,
                with_alpha(pal.accent_blue, alpha),
                with_alpha(pal.accent_blue, alpha * 0.72),
            );
            surf.draw_line(
                (vessel.x + 5.0, vessel.y + 2.0),
                (vessel.x + fill_w - 5.0, vessel.y + 2.0),
                1.0,
                with_alpha(pal.specular_top, alpha * 0.55),
            );
        }

        let has_consumed = w.consumed_ml > 0;
        let drink_w = if has_consumed { 78.0 } else { 96.0 };
        let drink_btn = RectF::new(left.x + 14.0, left.y + 100.0, drink_w, 28.0);
        let ddrink = self.draw_liquid_button(
            surf,
            pal,
            drink_btn,
            14.0,
            Action::WaterDrink,
            true,
            alpha,
            None,
        );
        surf.draw_text(
            &format!("+{}ml", w.cup_ml),
            ddrink,
            TextStyle::new(11.5, true, with_alpha(pal.cta_fg, alpha), 1),
        );

        let mut set_x = left.x + 14.0 + drink_w + 6.0;
        if has_consumed {
            let undo_btn = RectF::new(set_x, left.y + 100.0, 34.0, 28.0);
            let dundo = self.draw_liquid_button(
                surf,
                pal,
                undo_btn,
                14.0,
                Action::WaterUndo,
                false,
                alpha,
                None,
            );
            surf.draw_text("-1", dundo, TextStyle::new(11.5, true, muted, 1));
            if interact {
                actions.push((undo_btn, Action::WaterUndo));
            }
            set_x += 40.0;
        }

        let set_btn = RectF::new(
            set_x,
            left.y + 100.0,
            (left.x + left.w - 14.0 - set_x).max(66.0),
            28.0,
        );
        let dset = self.draw_liquid_button(
            surf,
            pal,
            set_btn,
            14.0,
            Action::WaterToggleSettings,
            false,
            alpha,
            None,
        );
        draw_sliders_icon(surf, dset.x + 14.0, dset.y + dset.h * 0.5, ink);
        surf.draw_text(
            "Adjust",
            RectF::new(dset.x + 24.0, dset.y, dset.w - 28.0, dset.h),
            TextStyle::new(11.2, false, ink, 0),
        );

        if interact {
            actions.push((toggle, Action::WaterToggleRemind));
            actions.push((drink_btn, Action::WaterDrink));
            actions.push((set_btn, Action::WaterToggleSettings));
        }

        // Right Bento Glass Card: Gauge or Inline Settings
        let right = RectF::new(ox + 224.0, oy + 4.0, 208.0, 138.0);
        draw_glass_tile(surf, pal, right, 16.0, alpha);

        if w.settings_open {
            let rows = [
                (
                    "Goal",
                    format!("{:.1}L", w.goal_ml as f32 / 1000.0),
                    Action::WaterGoal(-250),
                    Action::WaterGoal(250),
                ),
                (
                    "Cup",
                    format!("{}ml", w.cup_ml),
                    Action::WaterCup(-50),
                    Action::WaterCup(50),
                ),
                (
                    "Every",
                    format!("{}m", w.interval_min),
                    Action::WaterInterval(-15),
                    Action::WaterInterval(15),
                ),
            ];
            for (idx, (label, val, dec, inc)) in rows.into_iter().enumerate() {
                let ry = right.y + 12.0 + (idx as f32) * 30.0;
                surf.draw_text(
                    label,
                    RectF::new(right.x + 14.0, ry, 48.0, 22.0),
                    TextStyle::new(11.2, false, muted, 0),
                );
                let minus = RectF::new(right.x + 70.0, ry + 1.0, 24.0, 20.0);
                let plus = RectF::new(right.x + 168.0, ry + 1.0, 24.0, 20.0);
                let dminus =
                    self.draw_liquid_button(surf, pal, minus, 10.0, dec, false, alpha, None);
                let dplus = self.draw_liquid_button(surf, pal, plus, 10.0, inc, false, alpha, None);
                surf.draw_text("-", dminus, TextStyle::new(12.5, true, ink, 1));
                surf.draw_text("+", dplus, TextStyle::new(12.5, true, ink, 1));
                surf.draw_text(
                    &val,
                    RectF::new(right.x + 96.0, ry, 70.0, 22.0),
                    TextStyle::new(12.0, true, ink, 1),
                );
                if interact {
                    actions.push((minus, dec));
                    actions.push((plus, inc));
                }
            }
            let done_btn = RectF::new(right.x + 70.0, right.y + 106.0, 68.0, 22.0);
            let ddone = self.draw_liquid_button(
                surf,
                pal,
                done_btn,
                11.0,
                Action::WaterToggleSettings,
                true,
                alpha,
                None,
            );
            surf.draw_text(
                "Done",
                ddone,
                TextStyle::new(11.5, true, with_alpha(pal.cta_fg, alpha), 1),
            );
            if interact {
                actions.push((done_btn, Action::WaterToggleSettings));
            }
        } else {
            let cx = right.x + right.w * 0.5;
            let cy = right.y + 72.0;
            let r = 42.0;
            let arc_rect = RectF::new(cx - r, cy - r, r * 2.0, r * 2.0);
            surf.stroke_arc(arc_rect, 7.5, 135.0, 270.0, track);
            let sweep = 270.0 * w.fraction();
            if sweep > 1.0 {
                surf.stroke_arc(arc_rect, 7.5, 135.0, sweep, cyan);
            }
            let end_rad = (135.0 + sweep).to_radians();
            surf.fill_circle(cx + r * end_rad.cos(), cy + r * end_rad.sin(), 4.5, cyan);
            surf.draw_text(
                &pct.to_string(),
                RectF::new(cx - 32.0, cy - 20.0, 64.0, 30.0),
                TextStyle::new(23.0, true, ink, 1),
            );
            draw_droplet(surf, cx - 7.0, cy + 16.0, 4.5, cyan);
            surf.draw_text(
                "%",
                RectF::new(cx, cy + 7.0, 18.0, 18.0),
                TextStyle::new(11.5, false, muted, 0),
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_todo(
        &self,
        surf: &Surface<'_>,
        pal: &Palette,
        (ox, oy): (f32, f32),
        today: u32,
        alpha: f32,
        interact: bool,
        actions: &mut Vec<(RectF, Action)>,
    ) {
        let t = &self.data.todo;
        let ink = with_alpha(pal.ink, alpha);
        let muted = with_alpha(pal.muted, alpha);
        let green = with_alpha(pal.accent_green, alpha);
        let track = with_alpha(pal.track, alpha);
        let (done, total) = t.counts(today);

        // Top Frosted Control Strip
        let top_bar = RectF::new(ox, oy + 4.0, 432.0, 34.0);
        draw_glass_tile(surf, pal, top_bar, 12.0, alpha);

        surf.draw_text(
            &format!("{done}/{total}"),
            RectF::new(top_bar.x + 12.0, top_bar.y + 4.0, 44.0, 26.0),
            TextStyle::new(16.0, true, ink, 0),
        );
        let bar = RectF::new(top_bar.x + 56.0, top_bar.y + 15.0, 72.0, 4.5);
        surf.fill_round(bar, 2.25, track);
        if total > 0 && done > 0 {
            let frac = (done as f32 / total as f32).clamp(0.0, 1.0);
            surf.fill_round(
                RectF::new(bar.x, bar.y, (bar.w * frac).max(4.5), bar.h),
                2.25,
                green,
            );
        }

        let cal_btn = RectF::new(top_bar.x + 140.0, top_bar.y + 5.0, 74.0, 24.0);
        let dcal = self.draw_liquid_button(
            surf,
            pal,
            cal_btn,
            12.0,
            Action::TodoFilterToday,
            false,
            alpha,
            None,
        );
        draw_calendar_icon(
            surf,
            dcal.x + 13.0,
            dcal.y + dcal.h * 0.5,
            if t.today_only { ink } else { muted },
        );
        surf.draw_text(
            if t.today_only { "Today" } else { "All" },
            RectF::new(dcal.x + 22.0, dcal.y, dcal.w - 26.0, dcal.h),
            TextStyle::new(
                10.8,
                t.today_only,
                if t.today_only { ink } else { muted },
                0,
            ),
        );

        let add_btn = RectF::new(top_bar.x + 266.0, top_bar.y + 5.0, 88.0, 24.0);
        let dadd =
            self.draw_liquid_button(surf, pal, add_btn, 12.0, Action::TodoAdd, true, alpha, None);
        surf.draw_text(
            "+ Add Task",
            dadd,
            TextStyle::new(11.0, true, with_alpha(pal.cta_fg, alpha), 1),
        );

        let edit_btn = RectF::new(top_bar.x + 360.0, top_bar.y + 5.0, 62.0, 24.0);
        let has_tasks = total > 0;
        let edit_fg = if has_tasks { ink } else { muted };
        let dedit = if has_tasks {
            self.draw_liquid_button(
                surf,
                pal,
                edit_btn,
                12.0,
                Action::TodoEdit,
                false,
                alpha,
                None,
            )
        } else {
            surf.fill_round(edit_btn, 12.0, with_alpha(pal.btn_bg, alpha * 0.5));
            edit_btn
        };
        draw_pencil_icon(surf, dedit.x + 14.0, dedit.y + dedit.h * 0.5, edit_fg);
        surf.draw_text(
            "Edit",
            RectF::new(dedit.x + 22.0, dedit.y, dedit.w - 24.0, dedit.h),
            TextStyle::new(11.0, false, edit_fg, 0),
        );

        if interact {
            actions.push((cal_btn, Action::TodoFilterToday));
            actions.push((add_btn, Action::TodoAdd));
            if has_tasks {
                actions.push((edit_btn, Action::TodoEdit));
            }
        }

        // Full-width Frosted Task List Container
        let panel = RectF::new(ox, oy + 44.0, 432.0, 98.0);
        draw_glass_tile(surf, pal, panel, 14.0, alpha);

        if let Some(mode) = t.editing {
            let label = match mode {
                EditMode::Add => "New Task",
                EditMode::Edit(_) => "Edit Task",
            };
            surf.draw_text(
                label,
                RectF::new(panel.x + 16.0, panel.y + 10.0, panel.w - 32.0, 18.0),
                TextStyle::new(12.0, true, ink, 0),
            );
            let box_rect = RectF::new(panel.x + 14.0, panel.y + 34.0, panel.w - 28.0, 32.0);
            surf.fill_round(box_rect, 10.0, with_alpha(pal.track, alpha));
            let caret = if self.caret_tick.is_multiple_of(2) {
                "|"
            } else {
                ""
            };
            let text = format!("{}{caret}", t.input);
            surf.draw_text(
                &text,
                RectF::new(box_rect.x + 10.0, box_rect.y, box_rect.w - 20.0, box_rect.h),
                TextStyle::new(12.5, false, ink, 0),
            );
            surf.draw_text(
                "Enter to save · Esc to cancel",
                RectF::new(panel.x + 16.0, panel.y + 74.0, panel.w - 32.0, 18.0),
                TextStyle::new(10.5, false, muted, 0),
            );
            return;
        }

        let visible = t.visible_indices(today);
        if visible.is_empty() {
            let badge = RectF::new(panel.x + (panel.w - 36.0) * 0.5, panel.y + 18.0, 36.0, 36.0);
            surf.fill_round(badge, 10.0, with_alpha(pal.btn_bg, alpha));
            draw_checklist_icon(surf, badge.x + 18.0, badge.y + 18.0, muted);
            surf.draw_text(
                "No tasks · click + Add Task",
                RectF::new(panel.x + 12.0, panel.y + 62.0, panel.w - 24.0, 20.0),
                TextStyle::new(11.5, false, muted, 1),
            );
        } else {
            let max_scroll = visible.len().saturating_sub(4);
            let start = t.scroll.min(max_scroll);
            for (slot, &task_idx) in visible[start..].iter().take(4).enumerate() {
                let item = &t.tasks[task_idx];
                let row = RectF::new(
                    panel.x + 8.0,
                    panel.y + 5.0 + (slot as f32) * 22.0,
                    panel.w - 16.0,
                    22.0,
                );
                let is_sel = t.selected == Some(task_idx);
                let row_hovered = matches!(
                    self.hovered,
                    Some(Action::TodoSelect(i) | Action::TodoEditAt(i) | Action::TodoDelete(i) | Action::TodoToggle(i))
                        if i == task_idx
                );
                if is_sel || row_hovered {
                    surf.fill_round(row, 7.0, with_alpha(pal.card_hover, alpha));
                }

                let check_rect = RectF::new(row.x + 4.0, row.y + 1.0, 20.0, 20.0);
                let cx = check_rect.x + 10.0;
                let cy = check_rect.y + 10.0;
                if item.done {
                    surf.fill_circle(cx, cy, 6.5, green);
                    let tick = with_alpha(pal.on_accent, alpha);
                    surf.draw_line((cx - 2.8, cy), (cx - 0.5, cy + 2.3), 1.6, tick);
                    surf.draw_line((cx - 0.5, cy + 2.3), (cx + 3.2, cy - 2.3), 1.6, tick);
                } else {
                    surf.stroke_circle(cx, cy, 6.0, 1.3, muted);
                }

                let text_col = if item.done { muted } else { ink };
                let right_reserve = if row_hovered || is_sel { 44.0 } else { 24.0 };
                surf.draw_text(
                    &item.title,
                    RectF::new(row.x + 28.0, row.y, row.w - 28.0 - right_reserve, row.h),
                    TextStyle::new(11.8, false, text_col, 0),
                );

                let edit_row_rect = RectF::new(row.x + row.w - 42.0, row.y + 1.0, 18.0, 20.0);
                if row_hovered || is_sel {
                    draw_pencil_icon(
                        surf,
                        edit_row_rect.x + 9.0,
                        edit_row_rect.y + 10.0,
                        if self.hovered == Some(Action::TodoEditAt(task_idx)) {
                            ink
                        } else {
                            muted
                        },
                    );
                }

                let del_rect = RectF::new(row.x + row.w - 22.0, row.y + 1.0, 20.0, 20.0);
                let del_col = if self.hovered == Some(Action::TodoDelete(task_idx)) {
                    with_alpha(pal.accent_red, alpha)
                } else {
                    muted
                };
                surf.draw_text("×", del_rect, TextStyle::new(13.0, false, del_col, 1));

                if interact {
                    actions.push((check_rect, Action::TodoToggle(task_idx)));
                    actions.push((edit_row_rect, Action::TodoEditAt(task_idx)));
                    actions.push((del_rect, Action::TodoDelete(task_idx)));
                    actions.push((
                        RectF::new(row.x + 26.0, row.y, row.w - 70.0, row.h),
                        Action::TodoSelect(task_idx),
                    ));
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_plugin_page(
        &self,
        surf: &Surface<'_>,
        pal: &Palette,
        (ox, oy): (f32, f32),
        plugin_idx: usize,
        alpha: f32,
        interact: bool,
        actions: &mut Vec<(RectF, Action)>,
    ) {
        let ink = with_alpha(pal.ink, alpha);
        let ink_soft = with_alpha(pal.ink_soft, alpha);
        let muted = with_alpha(pal.muted, alpha);
        let track = with_alpha(pal.track, alpha);

        let Some(entry) = self.plugin_entries.get(plugin_idx) else {
            let empty = RectF::new(ox, oy + 4.0, 432.0, 138.0);
            draw_glass_tile(surf, pal, empty, 16.0, alpha);
            surf.draw_text(
                "Plugin unavailable",
                empty,
                TextStyle::new(12.0, false, muted, 1),
            );
            return;
        };

        let d = &entry.data;
        let m = &entry.manifest;
        let tone_col = match d.tone {
            PluginTone::Good => with_alpha(pal.accent_green, alpha),
            PluginTone::Warn => with_alpha(pal.accent_amber, alpha),
            PluginTone::Critical => with_alpha(pal.accent_red, alpha),
            PluginTone::Normal => with_alpha(pal.accent_blue, alpha),
        };

        let has_items = !d.items.is_empty();
        let left_w = if has_items { 216.0 } else { 432.0 };
        let hero = RectF::new(ox, oy + 4.0, left_w, 138.0);
        draw_glass_tile(surf, pal, hero, 16.0, alpha);

        surf.fill_circle(hero.x + 16.0, hero.y + 18.0, 3.6, tone_col);
        let title_txt = if d.title.is_empty() {
            m.name.as_str()
        } else {
            d.title.as_str()
        };
        surf.draw_text(
            title_txt,
            RectF::new(hero.x + 25.0, hero.y + 9.0, hero.w - 94.0, 18.0),
            TextStyle::new(11.4, true, ink_soft, 0),
        );
        if !d.badge.is_empty() {
            let bw = (surf.measure_text_width(&d.badge, 9.5, true) + 14.0).clamp(38.0, 68.0);
            let badge = RectF::new(hero.x + hero.w - 12.0 - bw, hero.y + 9.0, bw, 18.0);
            surf.fill_round(badge, 9.0, with_alpha(tone_col, 0.16));
            surf.draw_text(&d.badge, badge, TextStyle::new(9.5, true, tone_col, 1));
        }

        let main_val = if !d.updated {
            "Waiting...".to_string()
        } else if d.value.is_empty() {
            "Ready".to_string()
        } else {
            d.value.clone()
        };
        surf.draw_text(
            &main_val,
            RectF::new(hero.x + 14.0, hero.y + 34.0, hero.w - 28.0, 34.0),
            TextStyle::new(22.0, true, ink, 0),
        );
        if !d.subtitle.is_empty() {
            surf.draw_text(
                &d.subtitle,
                RectF::new(hero.x + 14.0, hero.y + 68.0, hero.w - 28.0, 18.0),
                TextStyle::new(11.0, false, muted, 0),
            );
        }

        if let Some(p) = d.progress {
            let bar = RectF::new(hero.x + 14.0, hero.y + 90.0, hero.w - 28.0, 4.5);
            surf.fill_round(bar, 2.25, track);
            if p > 0.01 {
                surf.fill_round(
                    RectF::new(bar.x, bar.y, (bar.w * p.clamp(0.0, 1.0)).max(4.5), bar.h),
                    2.25,
                    tone_col,
                );
            }
        }

        let ref_btn = RectF::new(hero.x + 14.0, hero.y + 104.0, 74.0, 24.0);
        let dref = self.draw_liquid_button(
            surf,
            pal,
            ref_btn,
            12.0,
            Action::PluginRefresh(plugin_idx),
            false,
            alpha,
            None,
        );
        surf.draw_text("Refresh", dref, TextStyle::new(10.8, true, ink, 1));
        if interact {
            actions.push((ref_btn, Action::PluginRefresh(plugin_idx)));
        }

        if !m.action_command.is_empty() {
            let lbl = if m.action_label.is_empty() {
                "Run"
            } else {
                m.action_label.as_str()
            };
            let act_w = (surf.measure_text_width(lbl, 10.8, true) + 20.0).clamp(58.0, 112.0);
            let act_btn = RectF::new(hero.x + 94.0, hero.y + 104.0, act_w, 24.0);
            let dact = self.draw_liquid_button(
                surf,
                pal,
                act_btn,
                12.0,
                Action::PluginTrigger(plugin_idx),
                true,
                alpha,
                None,
            );
            surf.draw_text(
                lbl,
                dact,
                TextStyle::new(10.8, true, with_alpha(pal.cta_fg, alpha), 1),
            );
            if interact {
                actions.push((act_btn, Action::PluginTrigger(plugin_idx)));
            }
        }

        if has_items {
            let right = RectF::new(ox + 224.0, oy + 4.0, 208.0, 138.0);
            draw_glass_tile(surf, pal, right, 16.0, alpha);
            surf.draw_text(
                "Details",
                RectF::new(right.x + 14.0, right.y + 10.0, 80.0, 16.0),
                TextStyle::new(10.8, true, muted, 0),
            );
            for (i, (k, v)) in d.items.iter().take(3).enumerate() {
                let row = RectF::new(
                    right.x + 10.0,
                    right.y + 32.0 + (i as f32) * 32.0,
                    right.w - 20.0,
                    26.0,
                );
                surf.fill_round(row, 8.0, with_alpha(pal.btn_bg, alpha * 0.72));
                if v.is_empty() {
                    surf.draw_text(
                        k,
                        RectF::new(row.x + 8.0, row.y, row.w - 16.0, row.h),
                        TextStyle::new(11.0, false, ink, 0),
                    );
                } else {
                    surf.draw_text(
                        k,
                        RectF::new(row.x + 8.0, row.y, row.w * 0.48, row.h),
                        TextStyle::new(10.8, false, muted, 0),
                    );
                    surf.draw_text(
                        v,
                        RectF::new(row.x + row.w * 0.48, row.y, row.w * 0.52 - 8.0, row.h),
                        TextStyle::new(11.0, true, ink, 2),
                    );
                }
            }
        }
    }

    fn hit_test(&self, px: f32, py: f32) -> Option<Action> {
        self.actions
            .iter()
            .rev()
            .find(|(rect, _)| rect.contains(px, py))
            .map(|(_, act)| *act)
    }

    fn apply_slider(&mut self, slider: ActiveSlider, px: f32) {
        match slider {
            ActiveSlider::Seek(bar) => {
                let frac = ((px - bar.x) / bar.w.max(1.0)).clamp(0.0, 1.0);
                self.media.send(MediaCmd::SeekFraction(frac));
                self.start_anim();
            }
            ActiveSlider::Volume(bar) => {
                let frac = ((px - bar.x) / bar.w.max(1.0)).clamp(0.0, 1.0);
                self.media.snapshot.volume = frac;
                self.media.snapshot.muted = false;
                self.media.send(MediaCmd::SetVolume(frac));
                self.start_anim();
            }
            ActiveSlider::DockNav => {
                self.dock_nav
                    .move_to([px, self.dock_bounds.y + self.dock_bounds.h * 0.5]);
                self.start_anim();
            }
        }
    }

    fn handle_down(&mut self, px: f32, py: f32) -> ClickEffect {
        let Some(action) = self.hit_test(px, py) else {
            return ClickEffect::None;
        };
        self.pressed = Some(action);
        self.press_spring.target = 1.0;
        self.start_anim();
        let st = self.cached_time;
        let today = date_key(&st);
        match action {
            Action::Expand => self.expand(),
            Action::ExpandTo(page) => {
                self.set_page(page);
                self.expand();
            }
            Action::ExpandToPlugin(idx) => {
                self.set_slot(NavSlot::Plugin(idx));
                self.expand();
            }
            Action::PrevPage => self.step_page(-1),
            Action::NextPage => self.step_page(1),
            Action::SelectSlot(slot_idx) => {
                let cy = self.dock_bounds.y + self.dock_bounds.h * 0.5;
                if self.dock_nav.press([px, cy]) {
                    self.active_slider = Some(ActiveSlider::DockNav);
                    self.start_anim();
                } else {
                    let slots = self.enabled_slots();
                    if let Some(&slot) = slots.get(slot_idx) {
                        self.set_slot(slot);
                    }
                }
            }
            Action::EnableTempSensor => return ClickEffect::EnableTempSensor,
            Action::PluginRefresh(idx) => {
                if let Some(id) = self.plugin_entries.get(idx).map(|e| e.manifest.id.clone()) {
                    self.plugin_host.refresh(&id);
                }
            }
            Action::PluginTrigger(idx) => {
                if let Some(id) = self.plugin_entries.get(idx).map(|e| e.manifest.id.clone()) {
                    self.plugin_host.run_action(&id);
                }
            }
            Action::MediaCopyTrack => {
                let snap = &self.media.snapshot;
                if snap.has_session && !snap.title.is_empty() {
                    let full = if snap.artist.is_empty() {
                        snap.title.clone()
                    } else {
                        format!("{} - {}", snap.title, snap.artist)
                    };
                    if copy_text_to_clipboard(self.hwnd, &full) {
                        self.track_copied_until =
                            Some(Instant::now() + Duration::from_millis(1400));
                        if !self.animating {
                            self.render();
                        }
                    }
                }
            }
            Action::MediaToggleLyrics => return ClickEffect::ToggleLyrics,
            Action::MediaPrev => self.media.send(MediaCmd::Prev),
            Action::MediaToggle => self.media.send(MediaCmd::TogglePlayPause),
            Action::MediaNext => self.media.send(MediaCmd::Next),
            Action::MediaSeek(bar) => {
                let slider = ActiveSlider::Seek(bar);
                self.active_slider = Some(slider);
                self.apply_slider(slider, px);
            }
            Action::MediaToggleVol => {
                self.volume_open = !self.volume_open;
                if !self.animating {
                    self.render();
                }
            }
            Action::VolMute => self.media.send(MediaCmd::ToggleMute),
            Action::VolSlider(bar) => {
                let slider = ActiveSlider::Volume(bar);
                self.active_slider = Some(slider);
                self.apply_slider(slider, px);
            }
            Action::ShelfAddFiles => {
                self.dragging_out = true;
                return ClickEffect::PickFiles;
            }
            Action::ShelfPaste => {
                let paths = read_files_from_clipboard(self.hwnd);
                if self.data.shelf.add_paths(paths) {
                    self.data.save_shelf();
                    if !self.animating {
                        self.render();
                    }
                }
            }
            Action::ShelfDragAll => {
                self.drag_press = Some((ShelfDrag::All, px, py));
            }
            Action::ShelfClear => {
                self.data.shelf.clear();
                self.data.save_shelf();
                if !self.animating {
                    self.render();
                }
            }
            Action::ShelfItem(idx) => {
                self.drag_press = Some((ShelfDrag::Single(idx), px, py));
            }
            Action::ShelfCopy(idx) => {
                if let Some(item) = self.data.shelf.items.get(idx) {
                    if copy_files_to_clipboard(self.hwnd, std::slice::from_ref(&item.path)) {
                        self.copied_until = Some(Instant::now() + Duration::from_millis(1400));
                        if !self.animating {
                            self.render();
                        }
                    }
                }
            }
            Action::ShelfDelete(idx) => {
                self.data.shelf.remove(idx);
                self.data.save_shelf();
                if !self.animating {
                    self.render();
                }
            }
            Action::PomoToggle => {
                self.data.pomodoro.toggle();
                self.update_compact_width();
                if !self.animating {
                    self.render();
                }
            }
            Action::PomoReset => {
                self.data.pomodoro.reset();
                self.update_compact_width();
                if !self.animating {
                    self.render();
                }
            }
            Action::PomoSkip => {
                self.data.pomodoro.skip_phase();
                self.update_compact_width();
                if !self.animating {
                    self.render();
                }
            }
            Action::PomoAdjustMin(d) => {
                self.data.pomodoro.adjust_focus_min(d);
                self.data.save();
                if !self.animating {
                    self.render();
                }
            }
            Action::WaterDrink => {
                self.data.water.drink();
                self.data.save();
                self.update_compact_width();
                if !self.animating {
                    self.render();
                }
            }
            Action::WaterUndo => {
                if self.data.water.undo_drink() {
                    self.data.save();
                    self.update_compact_width();
                    if !self.animating {
                        self.render();
                    }
                }
            }
            Action::WaterToggleRemind => {
                self.data.water.reminder_enabled = !self.data.water.reminder_enabled;
                if !self.data.water.reminder_enabled {
                    self.data.water.due_alert = false;
                }
                self.data.save();
                self.update_compact_width();
                if !self.animating {
                    self.render();
                }
            }
            Action::WaterToggleSettings => {
                self.data.water.settings_open = !self.data.water.settings_open;
                if !self.animating {
                    self.render();
                }
            }
            Action::WaterGoal(d) => {
                self.data.water.adjust_goal(d);
                self.data.save();
                if !self.animating {
                    self.render();
                }
            }
            Action::WaterCup(d) => {
                self.data.water.adjust_cup(d);
                self.data.save();
                if !self.animating {
                    self.render();
                }
            }
            Action::WaterInterval(d) => {
                self.data.water.adjust_interval(d);
                self.data.save();
                if !self.animating {
                    self.render();
                }
            }
            Action::TodoFilterToday => {
                self.data.todo.today_only = !self.data.todo.today_only;
                self.data.todo.scroll = 0;
                self.data.save();
                if !self.animating {
                    self.render();
                }
            }
            Action::TodoAdd => {
                self.data.todo.begin_add();
                self.set_focus_mode(true);
                if !self.animating {
                    self.render();
                }
            }
            Action::TodoEdit => {
                if self.data.todo.begin_edit(today) {
                    self.set_focus_mode(true);
                    if !self.animating {
                        self.render();
                    }
                }
            }
            Action::TodoEditAt(idx) => {
                if self.data.todo.begin_edit_at(idx) {
                    self.set_focus_mode(true);
                    if !self.animating {
                        self.render();
                    }
                }
            }
            Action::TodoToggle(idx) => {
                self.data.todo.toggle_done(idx);
                self.data.save();
                if !self.animating {
                    self.render();
                }
            }
            Action::TodoSelect(idx) => {
                self.data.todo.selected = Some(idx);
                if !self.animating {
                    self.render();
                }
            }
            Action::TodoDelete(idx) => {
                self.data.todo.delete_task(idx);
                self.data.save();
                if !self.animating {
                    self.render();
                }
            }
        }
        ClickEffect::None
    }

    fn handle_up(&mut self) {
        if let Some(ActiveSlider::DockNav) = self.active_slider.take() {
            if let Some(vis_idx) = self.dock_nav.release() {
                let slots = self.enabled_slots();
                let active_idx = slots
                    .iter()
                    .position(|&s| s == self.active_slot)
                    .unwrap_or(0);
                const MAX_DOCK: usize = 6;
                let vis_count = slots.len().clamp(1, MAX_DOCK);
                let mut vis_start = active_idx.saturating_sub(MAX_DOCK / 2);
                if vis_start + vis_count > slots.len() {
                    vis_start = slots.len().saturating_sub(vis_count);
                }
                if let Some(&slot) = slots.get(vis_start + vis_idx) {
                    self.set_slot(slot);
                }
            }
        }
        self.active_slider = None;
        if self.pressed.is_some() {
            self.pressed = None;
            self.press_spring.target = 0.0;
            self.start_anim();
        }
        if let Some((ShelfDrag::Single(idx), _, _)) = self.drag_press.take() {
            if let Some(item) = self.data.shelf.items.get(idx) {
                open_file(&item.path);
            }
        }
    }

    fn popup_menu(&mut self) -> Option<u32> {
        unsafe {
            let menu = CreatePopupMenu();
            if menu == 0 {
                return None;
            }
            let item = |flags, id, label| AppendMenuW(menu, flags, id, wide(label).as_ptr());
            item(if self.pinned { 8 } else { 0 }, 1, "Pin Expanded");
            item(0, 2, "Hide Dynamic Island");
            let mut cursor = Point::default();
            GetCursorPos(&mut cursor);
            SetForegroundWindow(self.hwnd);
            let cmd = TrackPopupMenu(menu, 0x0122, cursor.x, cursor.y, 0, self.hwnd, null());
            DestroyMenu(menu);
            PostMessageW(self.hwnd, 0, 0, 0);
            match cmd {
                1 => {
                    self.pinned = !self.pinned;
                    if self.pinned {
                        self.expand();
                    } else {
                        self.collapse();
                    }
                    None
                }
                2 => Some(cmd),
                _ => None,
            }
        }
    }
}

impl Drop for Island<'_> {
    fn drop(&mut self) {
        self.data.save();
        if let Some(h) = self.hires.as_mut() {
            h.set_active(false);
        }
        unsafe {
            KillTimer(self.hwnd, TIMER_SECOND);
            KillTimer(self.hwnd, TIMER_ANIM);
            KillTimer(self.hwnd, TIMER_LEAVE);
            KillTimer(self.hwnd, TIMER_ZORDER);
            KillTimer(self.hwnd, TIMER_LYRICS);
            KillTimer(self.hwnd, TIMER_WAVE);
            if self.dev_notify != 0 {
                UnregisterDeviceNotification(self.dev_notify);
            }
            if self.fs_hook != 0 {
                UnhookWinEvent(self.fs_hook);
            }
            if self.move_hook != 0 {
                UnhookWinEvent(self.move_hook);
            }
            if self.explorer_hook != 0 {
                UnhookWinEvent(self.explorer_hook);
            }
        }
    }
}

fn draw_glass_tile(surf: &Surface<'_>, pal: &Palette, rect: RectF, r: f32, alpha: f32) {
    // Inner content wells: airy frosted glass cards in Light Mode, smoky borderless wells in Dark Mode.
    if pal.light {
        surf.fill_round(rect, r, with_alpha(argb(124, 255, 255, 255), alpha));
        surf.stroke_round(
            rect,
            r,
            0.85,
            with_alpha(pal.shell_rim, alpha * 0.45),
            false,
        );
    } else {
        surf.fill_round(rect, r, with_alpha(pal.card_bg, alpha));
    }
}

fn toast_color(kind: ToastKind, pal: &Palette) -> u32 {
    match kind {
        ToastKind::Charging | ToastKind::BatterySaverOn | ToastKind::DriveConnected => {
            pal.accent_green
        }
        ToastKind::AudioConnected { .. } => pal.accent_purple,
        ToastKind::MouseConnected | ToastKind::KeyboardConnected | ToastKind::UsbConnected => {
            pal.accent_blue
        }
        ToastKind::LowBattery => pal.accent_red,
        ToastKind::Unplugged
        | ToastKind::BatterySaverOff
        | ToastKind::MouseRemoved
        | ToastKind::KeyboardRemoved
        | ToastKind::UsbRemoved
        | ToastKind::DriveRemoved => pal.accent_amber,
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_battery_capsule(
    surf: &Surface<'_>,
    x: f32,
    cy: f32,
    pct: u8,
    kind: ToastKind,
    phase: f32,
    pal: &Palette,
    alpha: f32,
) {
    let accent = toast_color(kind, pal);
    let pulse = 0.5 + 0.5 * phase.sin();
    let body = RectF::new(x, cy - 5.5, 19.0, 11.0);

    if matches!(
        kind,
        ToastKind::Charging | ToastKind::BatterySaverOn | ToastKind::LowBattery
    ) {
        surf.fill_round(
            RectF::new(body.x - 2.0, body.y - 2.0, body.w + 5.5, body.h + 4.0),
            5.0,
            with_alpha(accent, (0.12 + 0.12 * pulse) * alpha),
        );
    }

    surf.fill_round(body, 3.2, with_alpha(accent, 0.16 * alpha));
    surf.stroke_round(body, 3.2, 1.15, with_alpha(accent, 0.88 * alpha), false);
    surf.fill_round(
        RectF::new(x + 19.6, cy - 2.2, 2.0, 4.4),
        0.9,
        with_alpha(accent, 0.80 * alpha),
    );

    let fill_frac = (pct as f32 / 100.0).clamp(0.10, 1.0);
    let fill_w = (15.4 * fill_frac).max(3.6);
    let fill_col = if kind == ToastKind::Charging {
        mix_color(
            with_alpha(accent, alpha),
            with_alpha(pal.on_accent, alpha),
            0.10 + 0.12 * pulse,
        )
    } else {
        with_alpha(accent, 0.92 * alpha)
    };
    surf.fill_round(RectF::new(x + 1.8, cy - 3.7, fill_w, 7.4), 1.8, fill_col);

    if kind == ToastKind::Charging {
        let bx = x + 9.5;
        let bolt = [
            (bx + 0.9, cy - 3.4),
            (bx - 2.1, cy + 0.3),
            (bx + 0.2, cy + 0.3),
            (bx - 0.9, cy + 3.4),
            (bx + 2.1, cy - 0.3),
            (bx - 0.2, cy - 0.3),
        ];
        surf.fill_polygon(&bolt, with_alpha(pal.on_accent, 0.96 * alpha));
    }
}

fn draw_toast_waveform(surf: &Surface<'_>, x: f32, cy: f32, phase: f32, max_h: f32, color: u32) {
    for i in 0..4 {
        let level = 0.28 + 0.68 * (0.5 + 0.5 * (phase * 1.4 + i as f32 * 1.25).sin());
        let h = (level * max_h).max(2.6);
        let bx = x + (i as f32) * 4.0;
        surf.fill_round(RectF::new(bx, cy - h * 0.5, 2.1, h), 1.05, color);
    }
}

fn draw_toast_icon(surf: &Surface<'_>, kind: ToastKind, cx: f32, cy: f32, color: u32) {
    match kind {
        ToastKind::Charging
        | ToastKind::Unplugged
        | ToastKind::BatterySaverOn
        | ToastKind::BatterySaverOff
        | ToastKind::LowBattery => {}
        ToastKind::AudioConnected { headphone } => {
            if headphone {
                surf.stroke_arc(
                    RectF::new(cx - 5.0, cy - 5.0, 10.0, 10.0),
                    1.6,
                    180.0,
                    180.0,
                    color,
                );
                surf.fill_round(RectF::new(cx - 6.0, cy - 0.5, 2.6, 5.2), 1.2, color);
                surf.fill_round(RectF::new(cx + 3.4, cy - 0.5, 2.6, 5.2), 1.2, color);
            } else {
                draw_speaker_icon(surf, cx, cy, false, color);
            }
        }
        ToastKind::MouseConnected | ToastKind::MouseRemoved => {
            surf.stroke_round(
                RectF::new(cx - 4.0, cy - 5.5, 8.0, 11.0),
                3.8,
                1.4,
                color,
                false,
            );
            surf.draw_line((cx, cy - 3.8), (cx, cy - 1.2), 1.5, color);
        }
        ToastKind::KeyboardConnected | ToastKind::KeyboardRemoved => {
            surf.stroke_round(
                RectF::new(cx - 6.0, cy - 4.0, 12.0, 8.0),
                1.8,
                1.3,
                color,
                false,
            );
            surf.draw_line((cx - 3.5, cy + 1.2), (cx + 3.5, cy + 1.2), 1.3, color);
            surf.fill_circle(cx - 3.0, cy - 1.2, 0.9, color);
            surf.fill_circle(cx, cy - 1.2, 0.9, color);
            surf.fill_circle(cx + 3.0, cy - 1.2, 0.9, color);
        }
        ToastKind::UsbConnected | ToastKind::UsbRemoved => {
            surf.stroke_round(
                RectF::new(cx - 3.2, cy - 5.5, 6.4, 3.6),
                0.8,
                1.2,
                color,
                false,
            );
            surf.fill_round(RectF::new(cx - 4.5, cy - 2.0, 9.0, 5.5), 1.6, color);
            surf.draw_line((cx, cy + 3.5), (cx, cy + 6.2), 1.5, color);
        }
        ToastKind::DriveConnected | ToastKind::DriveRemoved => {
            surf.stroke_round(
                RectF::new(cx - 5.5, cy - 3.8, 11.0, 7.6),
                1.8,
                1.3,
                color,
                false,
            );
            surf.fill_circle(cx + 2.4, cy, 1.2, color);
        }
    }
}

fn draw_privacy_dots(
    surf: &Surface<'_>,
    cx: f32,
    cy: f32,
    mic: bool,
    cam: bool,
    pal: &Palette,
    alpha: f32,
) {
    if !settings::get().privacy_dots {
        return;
    }
    let mut x = cx;
    if cam {
        surf.fill_circle(x, cy, 3.0, with_alpha(pal.accent_green, alpha));
        x -= 8.0;
    }
    if mic {
        surf.fill_circle(x, cy, 3.0, with_alpha(pal.accent_amber, alpha));
    }
}

fn draw_waveform(surf: &Surface<'_>, x: f32, cy: f32, wave: &[f32; 5], max_h: f32, color: u32) {
    for (idx, &level) in wave.iter().enumerate() {
        let h = (level.clamp(0.15, 1.0) * max_h).max(2.5);
        let bx = x + (idx as f32) * 4.5;
        surf.fill_round(RectF::new(bx, cy - h * 0.5, 2.2, h), 1.1, color);
    }
}

fn draw_note_icon(surf: &Surface<'_>, cx: f32, cy: f32, s: f32, color: u32) {
    surf.draw_line(
        (cx - 3.0 * s, cy + 3.0 * s),
        (cx - 3.0 * s, cy - 5.0 * s),
        1.8 * s,
        color,
    );
    surf.draw_line(
        (cx - 3.0 * s, cy - 5.0 * s),
        (cx + 4.0 * s, cy - 6.5 * s),
        2.2 * s,
        color,
    );
    surf.draw_line(
        (cx + 4.0 * s, cy - 6.5 * s),
        (cx + 4.0 * s, cy + 1.5 * s),
        1.8 * s,
        color,
    );
    surf.fill_circle(cx - 5.0 * s, cy + 3.5 * s, 2.5 * s, color);
    surf.fill_circle(cx + 2.0 * s, cy + 2.0 * s, 2.5 * s, color);
}

fn draw_quote_icon(surf: &Surface<'_>, cx: f32, cy: f32, color: u32) {
    surf.stroke_round(
        RectF::new(cx - 9.0, cy - 7.5, 18.0, 14.0),
        4.5,
        1.4,
        color,
        false,
    );
    surf.fill_circle(cx - 2.8, cy - 1.0, 1.8, color);
    surf.fill_circle(cx + 2.8, cy - 1.0, 1.8, color);
    surf.draw_line((cx - 2.0, cy), (cx - 3.2, cy + 2.5), 1.2, color);
    surf.draw_line((cx + 3.6, cy), (cx + 2.4, cy + 2.5), 1.2, color);
}

fn draw_play_pause_icon(surf: &Surface<'_>, cx: f32, cy: f32, playing: bool, color: u32) {
    if playing {
        surf.fill_round(RectF::new(cx - 5.5, cy - 7.0, 3.6, 14.0), 1.5, color);
        surf.fill_round(RectF::new(cx + 1.9, cy - 7.0, 3.6, 14.0), 1.5, color);
    } else {
        surf.fill_polygon(
            &[(cx - 5.0, cy - 7.5), (cx + 7.5, cy), (cx - 5.0, cy + 7.5)],
            color,
        );
    }
}

fn draw_skip_icon(surf: &Surface<'_>, cx: f32, cy: f32, dir: f32, color: u32) {
    let tip = 6.5 * dir;
    let back = -1.5 * dir;
    surf.fill_polygon(
        &[(cx + back, cy - 5.5), (cx + tip, cy), (cx + back, cy + 5.5)],
        color,
    );
    let back2 = -7.0 * dir;
    surf.fill_polygon(
        &[
            (cx + back2, cy - 5.5),
            (cx + back, cy),
            (cx + back2, cy + 5.5),
        ],
        color,
    );
}

fn draw_skip_phase_icon(surf: &Surface<'_>, cx: f32, cy: f32, color: u32) {
    surf.fill_polygon(
        &[(cx - 4.5, cy - 4.5), (cx + 2.0, cy), (cx - 4.5, cy + 4.5)],
        color,
    );
    surf.fill_round(RectF::new(cx + 3.0, cy - 4.5, 1.8, 9.0), 0.9, color);
}

fn draw_speaker_icon(surf: &Surface<'_>, cx: f32, cy: f32, muted: bool, color: u32) {
    surf.fill_polygon(
        &[
            (cx - 6.0, cy - 2.5),
            (cx - 2.5, cy - 2.5),
            (cx + 1.5, cy - 6.0),
            (cx + 1.5, cy + 6.0),
            (cx - 2.5, cy + 2.5),
            (cx - 6.0, cy + 2.5),
        ],
        color,
    );
    if muted {
        surf.draw_line((cx + 3.5, cy - 2.8), (cx + 8.0, cy + 2.8), 1.5, color);
        surf.draw_line((cx + 8.0, cy - 2.8), (cx + 3.5, cy + 2.8), 1.5, color);
    } else {
        surf.stroke_arc(
            RectF::new(cx - 1.5, cy - 3.8, 7.6, 7.6),
            1.4,
            -55.0,
            110.0,
            color,
        );
        surf.stroke_arc(
            RectF::new(cx - 3.5, cy - 6.2, 12.4, 12.4),
            1.4,
            -55.0,
            110.0,
            color,
        );
    }
}

fn draw_tray_icon(surf: &Surface<'_>, cx: f32, cy: f32, color: u32) {
    surf.draw_line((cx, cy - 11.0), (cx, cy + 1.0), 2.0, color);
    surf.fill_polygon(
        &[(cx - 4.0, cy - 2.0), (cx + 4.0, cy - 2.0), (cx, cy + 3.0)],
        color,
    );
    surf.fill_round(
        RectF::new(cx - 12.0, cy + 4.0, 24.0, 9.0),
        3.0,
        with_alpha(color, 0.7),
    );
}

fn draw_copy_icon(surf: &Surface<'_>, cx: f32, cy: f32, color: u32) {
    surf.stroke_round(
        RectF::new(cx - 4.5, cy - 4.5, 6.5, 7.5),
        1.2,
        1.2,
        color,
        false,
    );
    surf.stroke_round(
        RectF::new(cx - 1.5, cy - 2.0, 6.5, 7.5),
        1.2,
        1.2,
        color,
        false,
    );
}

fn draw_droplet(surf: &Surface<'_>, cx: f32, cy: f32, r: f32, color: u32) {
    surf.fill_circle(cx, cy + r * 0.28, r * 0.68, color);
    surf.fill_polygon(
        &[
            (cx, cy - r * 0.95),
            (cx - r * 0.64, cy + r * 0.22),
            (cx + r * 0.64, cy + r * 0.22),
        ],
        color,
    );
}

fn draw_chevron(surf: &Surface<'_>, cx: f32, cy: f32, dir: f32, color: u32) {
    let dx = 3.2 * dir;
    surf.draw_line((cx - dx, cy - 4.5), (cx + dx, cy), 1.6, color);
    surf.draw_line((cx + dx, cy), (cx - dx, cy + 4.5), 1.6, color);
}

fn draw_reset_icon(surf: &Surface<'_>, cx: f32, cy: f32, color: u32) {
    let rect = RectF::new(cx - 5.5, cy - 5.5, 11.0, 11.0);
    surf.stroke_arc(rect, 1.6, -45.0, 270.0, color);
    surf.fill_polygon(
        &[
            (cx - 5.5, cy - 1.0),
            (cx - 8.2, cy - 4.2),
            (cx - 2.6, cy - 4.2),
        ],
        color,
    );
}

fn draw_sliders_icon(surf: &Surface<'_>, cx: f32, cy: f32, color: u32) {
    surf.draw_line((cx - 5.0, cy - 3.0), (cx + 5.0, cy - 3.0), 1.4, color);
    surf.draw_line((cx - 5.0, cy + 3.0), (cx + 5.0, cy + 3.0), 1.4, color);
    surf.fill_circle(cx - 2.0, cy - 3.0, 1.8, color);
    surf.fill_circle(cx + 2.0, cy + 3.0, 1.8, color);
}

fn draw_calendar_icon(surf: &Surface<'_>, cx: f32, cy: f32, color: u32) {
    surf.stroke_round(
        RectF::new(cx - 5.5, cy - 4.5, 11.0, 10.0),
        2.0,
        1.3,
        color,
        false,
    );
    surf.draw_line((cx - 5.0, cy - 1.5), (cx + 5.0, cy - 1.5), 1.2, color);
    surf.draw_line((cx - 2.5, cy - 6.0), (cx - 2.5, cy - 3.8), 1.2, color);
    surf.draw_line((cx + 2.5, cy - 6.0), (cx + 2.5, cy - 3.8), 1.2, color);
}

fn draw_pencil_icon(surf: &Surface<'_>, cx: f32, cy: f32, color: u32) {
    surf.draw_line((cx - 3.5, cy + 3.5), (cx + 3.5, cy - 3.5), 1.8, color);
    surf.fill_polygon(
        &[
            (cx - 4.5, cy + 4.5),
            (cx - 2.0, cy + 4.0),
            (cx - 4.0, cy + 2.0),
        ],
        color,
    );
}

fn draw_checklist_icon(surf: &Surface<'_>, cx: f32, cy: f32, color: u32) {
    surf.draw_line((cx - 8.0, cy - 4.0), (cx - 5.5, cy - 2.0), 1.5, color);
    surf.draw_line((cx - 5.5, cy - 2.0), (cx - 2.5, cy - 6.0), 1.5, color);
    surf.draw_line((cx + 0.5, cy - 4.0), (cx + 8.5, cy - 4.0), 1.5, color);
    surf.draw_line((cx - 8.0, cy + 4.0), (cx - 5.5, cy + 6.0), 1.5, color);
    surf.draw_line((cx - 5.5, cy + 6.0), (cx - 2.5, cy + 2.0), 1.5, color);
    surf.draw_line((cx + 0.5, cy + 4.0), (cx + 8.5, cy + 4.0), 1.5, color);
}

unsafe extern "system" fn fs_event(
    _: Handle,
    _: u32,
    _: Handle,
    object: i32,
    child: i32,
    _: u32,
    _: u32,
) {
    if object == 0 && child == 0 {
        let hwnd = with_island(|i| i.hwnd).unwrap_or(0);
        if hwnd != 0 {
            PostMessageW(hwnd, WM_ISLAND_CHECK_FS, 0, 0);
        }
    }
}

unsafe extern "system" fn island_proc(hwnd: Handle, msg: u32, w: usize, l: isize) -> isize {
    match msg {
        0x0008 => {
            with_island(|i| {
                if i.dock_nav.pressed {
                    i.dock_nav.cancel();
                    i.active_slider = None;
                    i.start_anim();
                }
                if i.data.todo.editing.is_some() {
                    i.data.todo.cancel();
                    i.set_focus_mode(false);
                    if !i.animating {
                        i.render();
                    }
                }
            });
        }
        0x0014 => return 1,
        0x0021 => {
            let editing = with_island(|i| i.data.todo.editing.is_some()).unwrap_or(false);
            return if editing { 1 } else { 3 };
        }
        0x001a | 0x031a => {
            with_island(Island::refresh_geometry);
        }
        0x0100 => {
            with_island(|i| {
                if i.data.todo.editing.is_some() {
                    let st = i.cached_time;
                    let today = date_key(&st);
                    match w {
                        0x0D => {
                            if i.data.todo.commit(today) {
                                i.data.save();
                            }
                            i.set_focus_mode(false);
                            if !i.animating {
                                i.render();
                            }
                        }
                        0x1B => {
                            i.data.todo.cancel();
                            i.set_focus_mode(false);
                            if !i.animating {
                                i.render();
                            }
                        }
                        0x08 => {
                            i.data.todo.backspace();
                            if !i.animating {
                                i.render();
                            }
                        }
                        _ => {}
                    }
                }
            });
        }
        0x0102 => {
            if let Some(ch) = char::from_u32(w as u32) {
                if !ch.is_control() {
                    with_island(|i| {
                        if i.data.todo.editing.is_some() {
                            i.data.todo.push_char(ch);
                            if !i.animating {
                                i.render();
                            }
                        }
                    });
                }
            }
        }
        0x0113 => match w {
            TIMER_SECOND => {
                with_island(Island::on_second_tick);
            }
            TIMER_ANIM => {
                with_island(Island::on_anim_tick);
            }
            TIMER_LEAVE => {
                with_island(Island::update_cursor_watch);
            }
            TIMER_ZORDER => {
                with_island(|i| {
                    i.watch_explorer();
                    let cfg = settings::get();
                    let (bounds, _, dpi) = primary_monitor_geometry();
                    let (top, ch) = detect_taskbar_fit(bounds, dpi, cfg.taskbar_fit);
                    if (top - i.card_top).abs() > 0.5 || (ch - i.compact_h).abs() > 0.5 {
                        i.card_top = top;
                        i.compact_h = ch;
                        if !i.animating {
                            i.render();
                        }
                    }
                    i.ensure_topmost();
                });
            }
            TIMER_LYRICS => {
                with_island(Island::on_lyrics_tick);
            }
            TIMER_WAVE => {
                with_island(Island::on_wave_tick);
            }
            _ => {}
        },
        0x0200 => {
            let mut drag_paths: Option<Vec<String>> = None;
            with_island(|i| {
                i.outside_since = None;
                if i.expand.target < 0.5 && settings::get().expand_on_hover {
                    i.expand();
                }
                let s = i.scale();
                let px = (l as i16) as f32 / s;
                let py = ((l >> 16) as i16) as f32 / s;

                if let Some(slider) = i.active_slider {
                    i.apply_slider(slider, px);
                } else if let Some((kind, sx, sy)) = i.drag_press {
                    let dist_sq = (px - sx) * (px - sx) + (py - sy) * (py - sy);
                    if dist_sq >= 25.0 {
                        i.drag_press = None;
                        i.dragging_out = true;
                        drag_paths = Some(match kind {
                            ShelfDrag::Single(idx) => i
                                .data
                                .shelf
                                .items
                                .get(idx)
                                .map(|item| vec![item.path.clone()])
                                .unwrap_or_default(),
                            ShelfDrag::All => i.data.shelf.paths(),
                        });
                    }
                }

                let next_hover = i.hit_test(px, py);
                if next_hover != i.hovered {
                    i.prev_hovered = i.hovered;
                    i.hovered = next_hover;
                    i.hover_t = 0.0;
                    i.start_anim();
                }
                if next_hover.is_some() && i.hand_cursor != 0 {
                    SetCursor(i.hand_cursor);
                }
            });
            if let Some(paths) = drag_paths {
                start_drag_out(&paths);
                with_island(|i| {
                    i.dragging_out = false;
                    i.update_cursor_watch();
                });
            }
        }
        0x0020 => {
            let has_hover = with_island(|i| {
                if i.hovered.is_some() && i.hand_cursor != 0 {
                    SetCursor(i.hand_cursor);
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false);
            if has_hover {
                return 1;
            }
            return DefWindowProcW(hwnd, msg, w, l);
        }
        0x0201 => {
            let effect = with_island(|i| {
                let s = i.scale();
                let px = (l as i16) as f32 / s;
                let py = ((l >> 16) as i16) as f32 / s;
                i.handle_down(px, py)
            })
            .unwrap_or(ClickEffect::None);
            match effect {
                ClickEffect::PickFiles => {
                    let paths = pick_files_dialog(hwnd);
                    with_island(|i| {
                        i.dragging_out = false;
                        if i.data.shelf.add_paths(paths) {
                            i.data.save_shelf();
                        }
                        i.update_cursor_watch();
                        if !i.animating {
                            i.render();
                        }
                    });
                }
                ClickEffect::ToggleLyrics => {
                    settings::update(|s| s.lyrics_enabled = !s.lyrics_enabled);
                }
                ClickEffect::EnableTempSensor => {
                    crate::ui::trigger_pawnio();
                }
                ClickEffect::None => {}
            }
        }
        0x0202 => {
            with_island(Island::handle_up);
        }
        0x0205 => {
            if with_island(Island::popup_menu).flatten() == Some(2) {
                settings::update(|s| s.island_enabled = false);
            }
        }
        0x0218 if w == 0x000A => {
            with_island(Island::check_power_change);
            return 1;
        }
        0x0219 => {
            with_island(|i| i.handle_device_change(w, l));
            return 1;
        }
        0x0233 => {
            let paths = handle_wm_dropfiles(w as Handle);
            with_island(|i| {
                if !paths.is_empty() && i.data.shelf.add_paths(paths) {
                    i.data.save_shelf();
                }
                i.set_page(Page::Shelf);
                i.expand();
                if !i.animating {
                    i.render();
                }
            });
        }
        0x020A => {
            let delta = (w >> 16) as i16;
            with_island(|i| {
                if delta == 0 {
                    return;
                }
                if i.expand.value <= 0.35 && (i.media.snapshot.playing || i.vol_hud_active()) {
                    let step = if delta > 0 { 0.05 } else { -0.05 };
                    i.media.send(MediaCmd::StepVolume(step));
                    if settings::get().volume_hud_enabled {
                        i.vol_hud_until = Some(Instant::now() + Duration::from_millis(2000));
                    }
                    i.update_compact_width();
                    if !i.animating {
                        i.render();
                    }
                    return;
                }
                if i.expand.value > 0.7 {
                    let st = i.cached_time;
                    let today = date_key(&st);
                    if i.active_slot == NavSlot::Page(Page::Media)
                        && (i.volume_open
                            || matches!(
                                i.hovered,
                                Some(
                                    Action::MediaToggleVol | Action::VolSlider(_) | Action::VolMute
                                )
                            ))
                    {
                        let step = if delta > 0 { 0.05 } else { -0.05 };
                        i.media.send(MediaCmd::StepVolume(step));
                    } else if i.active_slot == NavSlot::Page(Page::Shelf)
                        && i.data.shelf.items.len() > 3
                    {
                        let max_scroll = i.data.shelf.items.len().saturating_sub(3);
                        if delta < 0 {
                            i.data.shelf.scroll = (i.data.shelf.scroll + 1).min(max_scroll);
                        } else {
                            i.data.shelf.scroll = i.data.shelf.scroll.saturating_sub(1);
                        }
                        if !i.animating {
                            i.render();
                        }
                    } else if i.active_slot == NavSlot::Page(Page::Todo)
                        && i.data.todo.visible_indices(today).len() > 4
                    {
                        let max_scroll = i.data.todo.visible_indices(today).len().saturating_sub(4);
                        if delta < 0 {
                            i.data.todo.scroll = (i.data.todo.scroll + 1).min(max_scroll);
                        } else {
                            i.data.todo.scroll = i.data.todo.scroll.saturating_sub(1);
                        }
                        if !i.animating {
                            i.render();
                        }
                    } else if delta < 0 {
                        i.step_page(1);
                    } else {
                        i.step_page(-1);
                    }
                }
            });
        }
        WM_MEDIA_READY => {
            with_island(Island::on_media_ready);
        }
        WM_LYRICS_READY => {
            with_island(Island::on_lyrics_ready);
        }
        WM_PLUGIN_READY => {
            with_island(Island::on_plugins_ready);
        }
        WM_SHELF_DRAG_ENTER => {
            with_island(|i| {
                i.data.shelf.drag_over = true;
                i.set_page(Page::Shelf);
                i.expand();
                if !i.animating {
                    i.render();
                }
            });
        }
        WM_SHELF_DRAG_LEAVE => {
            with_island(|i| {
                if i.data.shelf.drag_over {
                    i.data.shelf.drag_over = false;
                    if !i.animating {
                        i.render();
                    }
                }
            });
        }
        WM_SHELF_DROP => {
            let paths = take_dropped_paths();
            with_island(|i| {
                i.data.shelf.drag_over = false;
                if i.data.shelf.add_paths(paths) {
                    i.data.save_shelf();
                }
                i.set_page(Page::Shelf);
                i.expand();
                if !i.animating {
                    i.render();
                }
            });
        }
        WM_ISLAND_CHECK_FS => {
            with_island(|i| {
                let fs = settings::get().hide_fullscreen && is_foreground_fullscreen(hwnd);
                if i.fullscreen_hidden != fs {
                    i.fullscreen_hidden = fs;
                    i.sync_visibility();
                } else {
                    i.ensure_topmost();
                }
            });
        }
        _ => {
            if with_island(|i| i.shell_msg != 0 && msg == i.shell_msg) == Some(true) {
                with_island(|i| {
                    i.explorer_pid = 0;
                    i.watch_explorer();
                    i.ensure_topmost();
                });
                return 0;
            }
            return DefWindowProcW(hwnd, msg, w, l);
        }
    }
    0
}

pub fn init(instance: Handle) {
    let Some(api) = gdiplus::shared() else {
        return;
    };
    unsafe {
        let class_name = wide("MoLapDynamicIsland");
        let wc = WindowClass {
            size: size_of::<WindowClass>() as u32,
            style: 0,
            callback: Some(island_proc),
            class_extra: 0,
            window_extra: 0,
            instance,
            icon: 0,
            cursor: LoadCursorW(0, 32512usize as *const u16),
            background: 0,
            menu: null(),
            class: class_name.as_ptr(),
            small_icon: 0,
        };
        if RegisterClassExW(&wc) == 0 {
            return;
        }
        let hwnd = CreateWindowExW(
            0x0808_0088, // WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST
            class_name.as_ptr(),
            wide("MoLap Island").as_ptr(),
            0x8000_0000, // WS_POPUP
            -32000,
            -32000,
            1,
            1,
            0,
            0,
            instance,
            0,
        );
        if hwnd != 0 {
            let island = Island::new(hwnd, api);
            ISLAND.with(|cell| *cell.borrow_mut() = Some(island));
        }
    }
}

pub fn shutdown() {
    let hwnd = with_island(|i| i.hwnd).unwrap_or(0);
    ISLAND.with(|cell| {
        cell.borrow_mut().take();
    });
    if hwnd != 0 {
        unsafe {
            DestroyWindow(hwnd);
        }
    }
}

pub fn set_suspended(paused: bool) {
    with_island(|i| {
        if i.suspended != paused {
            i.suspended = paused;
            i.sync_visibility();
        }
    });
}

pub fn on_display_change() {
    with_island(Island::refresh_geometry);
}

pub fn update_metrics(metrics: &[Metric]) {
    with_island(|i| i.on_metrics(metrics));
}

pub fn on_settings_changed() {
    with_island(Island::on_settings_changed);
}

pub fn on_settings_changed_live() {
    with_island(Island::on_settings_changed_live);
}

pub fn on_theme_change() {
    with_island(Island::refresh_geometry);
}

pub fn toggle_pin() {
    let next_pin = !is_pinned();
    if next_pin && !settings::get().island_enabled {
        settings::update(|s| s.island_enabled = true);
    }
    with_island(|i| {
        i.pinned = next_pin;
        if i.pinned {
            i.expand();
        } else {
            i.collapse();
        }
    });
}

pub fn is_pinned() -> bool {
    with_island(|i| i.pinned).unwrap_or(false)
}

pub fn trigger_demo_toast() {
    if !settings::get().island_enabled {
        settings::update(|s| s.island_enabled = true);
    }
    with_island(|i| {
        if i.expand.target > 0.5 {
            i.pinned = false;
            i.collapse();
        }
        i.trigger_toast(
            ToastKind::AudioConnected { headphone: true },
            "AirPods Pro",
            "",
            100,
            3,
            true,
        );
    });
}

pub fn plugin_snapshot() -> Vec<PluginEntry> {
    with_island(|i| i.plugin_entries.clone()).unwrap_or_default()
}

pub fn reload_plugins() {
    with_island(|i| {
        i.plugin_host.reload();
        i.plugin_entries = i.plugin_host.entries();
        let slots = i.enabled_slots();
        if !slots.contains(&i.active_slot) {
            i.active_slot = slots[0];
            i.slide.set(0.0);
        }
        i.update_compact_width();
        if i.should_show() && !i.animating {
            i.render();
        }
    });
}
