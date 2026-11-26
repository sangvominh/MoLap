use crate::{
    sensors::{self, Control, Kind, Metric},
    settings,
    win::*,
};
use std::{
    cell::RefCell,
    mem::{size_of, zeroed},
    os::windows::{ffi::OsStrExt, process::CommandExt},
    process::Command,
    ptr::{null, null_mut},
    sync::{
        mpsc::{self, Receiver},
        Arc, Condvar, Mutex,
    },
};

const TRAY: u32 = 0x8002;
const INSTALL_DONE: u32 = 0x8004;
const DISPLAY_POWER: Guid = Guid {
    a: 0x6fe69556,
    b: 0x704a,
    c: 0x47a0,
    d: [0x8f, 0x24, 0xc2, 0x8d, 0x93, 0x6f, 0xda, 0x47],
};
#[repr(C)]
struct PowerBroadcast {
    setting: Guid,
    length: u32,
}
const HELP: &str = "MoLap";

thread_local! {
    static UI: RefCell<Option<State>> = const { RefCell::new(None) };
}

struct State {
    window: Handle,
    events: Receiver<Vec<Metric>>,
    control: Arc<(Mutex<Control>, Condvar)>,
    worker: Option<std::thread::JoinHandle<()>>,
    metrics: Vec<Metric>,
    locked: bool,
    screen_off: bool,
    tray: bool,
    opening_menu: bool,
    installing: bool,
    shell_message: u32,
    power: Handle,
    session: Option<Library>,
}

fn state<T>(f: impl FnOnce(&mut State) -> T) -> Option<T> {
    UI.with(|ui| {
        ui.try_borrow_mut()
            .ok()
            .and_then(|mut ui| ui.as_mut().map(f))
    })
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
            locked: false,
            screen_off: false,
            tray: false,
            opening_menu: false,
            installing: false,
            shell_message: 0,
            power: 0,
            session: Library::system("wtsapi32.dll"),
        };
        unsafe {
            state.shell_message = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
            if state.shell_message != 0 {
                ChangeWindowMessageFilterEx(window, state.shell_message, 1, null_mut());
            }
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
        }
        state.show_tray(true);
        state.suspend();
        Some(state)
    }

    fn suspend(&self) {
        let cfg = settings::get();
        let (lock, wake) = &*self.control;
        let mut control = lock.lock().unwrap();
        let sensor_pause = !cfg.needs_sensors() || self.locked || self.screen_off;
        if control.paused != sensor_pause {
            control.paused = sensor_pause;
            wake.notify_one();
        }
        crate::island::set_suspended(self.locked || self.screen_off);
    }

    fn reads_ready(&mut self) {
        if let Ok(next) = self.events.try_recv() {
            crate::island::update_metrics(&next);
            self.metrics = next;
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
        }
        self.show_tray(false);
    }
}

fn information(text: &str, title: &str, flags: u32) -> i32 {
    unsafe {
        MessageBoxW(
            0,
            wide(text).as_ptr(),
            wide(title).as_ptr(),
            flags | 0x00010000,
        )
    }
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
                DestroyWindow(window);
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
        "Install the official PawnIO driver to enable CPU and Intel UHD temperatures?",
        "MoLap - Enable Temperatures",
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
        information("Could not start the installer.", HELP, 0x30);
    }
}

pub fn show_readings() {
    let fahr = settings::get().temp_fahrenheit;
    if let Some(readings) = state(|s| {
        if s.metrics.is_empty() {
            "No sensor readings available yet.".into()
        } else {
            s.metrics
                .iter()
                .map(|m| {
                    let temp = if m.kind != Kind::Ram {
                        m.temperature
                            .map(|v| format!(" · {}", settings::format_temp(v, fahr)))
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };
                    format!("{}  {}{}  —  {}", m.label, m.value, temp, m.detail)
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
    }) {
        information(&readings, "MoLap - Hardware Readings", 0x40);
    }
}

pub fn needs_temp_setup() -> bool {
    unsafe { IsUserAnAdmin() == 0 || !driver_installed() }
}

pub fn trigger_pawnio() {
    if let Some(win) = state(|s| s.window) {
        enable_pawnio(win);
    }
}

pub fn on_settings_changed() {
    state(|s| s.suspend());
}

unsafe fn popup(window: Handle) {
    let Some(installing) = state(|s| {
        if s.opening_menu {
            None
        } else {
            s.opening_menu = true;
            Some(s.installing)
        }
    })
    .flatten() else {
        return;
    };
    let menu = CreatePopupMenu();
    if menu == 0 {
        state(|s| s.opening_menu = false);
        return;
    }
    let cfg = settings::get();
    let item = |flags, id, label| AppendMenuW(menu, flags, id, wide(label).as_ptr());
    item(0, 7, "Control Center...");
    item(if cfg.island_enabled { 8 } else { 0 }, 6, "Dynamic Island");
    item(
        if crate::island::is_pinned() { 8 } else { 0 },
        8,
        "Pin Island Expanded",
    );
    item(0, 2, "Hardware Readings");
    if needs_temp_setup() {
        item(
            if installing { 1 } else { 0 },
            3,
            if IsUserAnAdmin() != 0 {
                "Install PawnIO for Temperatures"
            } else {
                "Run as Administrator for Temperatures"
            },
        );
    }
    let start_with_windows = crate::startup::enabled();
    item(
        if start_with_windows { 8 } else { 0 },
        5,
        "Start with Windows",
    );
    item(0, 4, "Quit MoLap");
    let mut cursor = Point::default();
    GetCursorPos(&mut cursor);
    SetForegroundWindow(window);
    let command = TrackPopupMenu(menu, 0x0122, cursor.x, cursor.y, 0, window, null());
    DestroyMenu(menu);
    PostMessageW(window, 0, 0, 0);
    state(|s| s.opening_menu = false);
    match command {
        7 => crate::settings_ui::open(),
        6 => settings::update(|s| s.island_enabled = !s.island_enabled),
        8 => {
            crate::island::toggle_pin();
            crate::settings_ui::on_settings_changed();
        }
        2 => show_readings(),
        3 => enable_pawnio(window),
        5 => {
            if let Err(error) = crate::startup::set_enabled(!start_with_windows) {
                information(
                    &format!("Could not update Windows startup: {error}"),
                    HELP,
                    0x30,
                );
            }
            crate::settings_ui::on_system_flags_changed();
        }
        4 => {
            DestroyWindow(window);
        }
        _ => {}
    }
}

unsafe extern "system" fn window_proc(window: Handle, message: u32, w: usize, l: isize) -> isize {
    match message {
        0x0002 => {
            UI.with(|ui| {
                ui.borrow_mut().take();
            });
            PostQuitMessage(0);
        }
        0x0010 => {
            DestroyWindow(window);
        }
        0x0016 if w != 0 => {
            DestroyWindow(window);
        }
        0x007e => {
            crate::island::on_display_change();
        }
        0x0218 if w == 0x8013 && l != 0 => {
            let header = &*(l as *const PowerBroadcast);
            if header.setting == DISPLAY_POWER && header.length >= 4 {
                let value = (l as *const u8)
                    .add(size_of::<PowerBroadcast>())
                    .cast::<u32>()
                    .read_unaligned();
                state(|s| {
                    s.screen_off = value == 0;
                    s.suspend();
                });
            }
            return 1;
        }
        0x0218 if w == 0x0012 => {
            crate::island::on_display_change();
            return 1;
        }
        0x02b1 if w == 7 || w == 8 => {
            state(|s| {
                s.locked = w == 7;
                s.suspend();
            });
        }
        0x001a | 0x031a => {
            crate::island::on_theme_change();
            crate::settings_ui::on_theme_change();
        }
        sensors::SAMPLE_READY => {
            state(State::reads_ready);
        }
        TRAY => match l as u32 {
            0x0202 => crate::settings_ui::open(),
            0x0205 => popup(window),
            _ => {}
        },
        INSTALL_DONE => {
            state(|s| s.installing = false);
            if driver_installed() {
                restart_elevated(window);
            } else {
                information(
                    if w != 0 {
                        "Could not launch WinGet. Download the official installer at https://pawnio.eu/"
                    } else {
                        "PawnIO installation did not finish. You can retry from the menu."
                    },
                    HELP,
                    0x40,
                );
            }
            crate::settings_ui::on_system_flags_changed();
        }
        _ => {
            if state(|s| message == s.shell_message) == Some(true) {
                state(|s| {
                    s.tray = false;
                    s.show_tray(true);
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
        crate::settings::init();
        let class = wide("MoLapTrayHost");
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
            crate::island::init(instance);
            crate::settings_ui::init(instance);
            let window = CreateWindowExW(
                0x0000_0080,
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
                    let mut message = Message::default();
                    while GetMessageW(&mut message, 0, 0, 0) > 0 {
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                } else {
                    DestroyWindow(window);
                }
            }
            UI.with(|ui| {
                ui.borrow_mut().take();
            });
            crate::settings_ui::shutdown();
            crate::island::shutdown();
            crate::gdiplus::shutdown_shared();
        }
        ReleaseMutex(mutex.0);
    }
}
