#![allow(non_snake_case)]

use std::ffi::c_void;

pub type Handle = isize;
pub type Bool = i32;
pub type Callback = unsafe extern "system" fn(Handle, u32, usize, isize) -> isize;
pub type EventCallback = unsafe extern "system" fn(Handle, u32, Handle, i32, i32, u32, u32);

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(self) -> i32 {
        self.right - self.left
    }
    pub fn height(self) -> i32 {
        self.bottom - self.top
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
pub struct WindowClass {
    pub size: u32,
    pub style: u32,
    pub callback: Option<Callback>,
    pub class_extra: i32,
    pub window_extra: i32,
    pub instance: Handle,
    pub icon: Handle,
    pub cursor: Handle,
    pub background: Handle,
    pub menu: *const u16,
    pub class: *const u16,
    pub small_icon: Handle,
}

#[repr(C)]
#[derive(Default)]
pub struct Message {
    pub window: Handle,
    pub message: u32,
    pub w: usize,
    pub l: isize,
    pub time: u32,
    pub point: Point,
    pub private: u32,
}

#[repr(C)]
pub struct Paint {
    pub dc: Handle,
    pub erase: Bool,
    pub rect: Rect,
    pub restore: Bool,
    pub update: Bool,
    pub reserved: [u8; 32],
}

#[repr(C)]
pub struct MonitorInfo {
    pub size: u32,
    pub bounds: Rect,
    pub work: Rect,
    pub flags: u32,
}

#[repr(C)]
#[derive(Default, PartialEq, Eq)]
pub struct Guid {
    pub a: u32,
    pub b: u16,
    pub c: u16,
    pub d: [u8; 8],
}

#[repr(C)]
pub struct NotifyIcon {
    pub size: u32,
    pub window: Handle,
    pub id: u32,
    pub flags: u32,
    pub message: u32,
    pub icon: Handle,
    pub tip: [u16; 128],
    pub state: u32,
    pub state_mask: u32,
    pub info: [u16; 256],
    pub version: u32,
    pub title: [u16; 64],
    pub info_flags: u32,
    pub guid: Guid,
    pub balloon_icon: Handle,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Size {
    pub cx: i32,
    pub cy: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BlendFunction {
    pub op: u8,
    pub flags: u8,
    pub alpha: u8,
    pub format: u8,
}

#[repr(C)]
#[derive(Default)]
pub struct BitmapInfoHeader {
    pub size: u32,
    pub width: i32,
    pub height: i32,
    pub planes: u16,
    pub bit_count: u16,
    pub compression: u32,
    pub size_image: u32,
    pub x_ppm: i32,
    pub y_ppm: i32,
    pub clr_used: u32,
    pub clr_important: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SystemTime {
    pub year: u16,
    pub month: u16,
    pub day_of_week: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub millis: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ProcessEntry32W {
    pub size: u32,
    pub usage: u32,
    pub process_id: u32,
    pub default_heap_id: usize,
    pub module_id: u32,
    pub threads: u32,
    pub parent_process_id: u32,
    pub pri_class_base: i32,
    pub flags: u32,
    pub exe_file: [u16; 260],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct SystemPowerStatus {
    pub ac_line: u8,
    pub battery_flag: u8,
    pub battery_percent: u8,
    pub system_status: u8,
    pub battery_life_time: u32,
    pub battery_full_life_time: u32,
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

#[link(name = "kernel32")]
extern "system" {
    pub fn GetModuleHandleW(name: *const u16) -> Handle;
    pub fn CreateMutexW(attributes: Handle, owner: Bool, name: *const u16) -> Handle;
    pub fn WaitForSingleObject(handle: Handle, millis: u32) -> u32;
    pub fn ReleaseMutex(handle: Handle) -> Bool;
    pub fn CloseHandle(handle: Handle) -> Bool;
    pub fn GetSystemTimes(idle: *mut u64, kernel: *mut u64, user: *mut u64) -> Bool;
    pub fn GlobalMemoryStatusEx(status: *mut c_void) -> Bool;
    pub fn QueryDosDeviceW(name: *const u16, out: *mut u16, count: u32) -> u32;
    pub fn GetDiskFreeSpaceExW(
        path: *const u16,
        available: *mut u64,
        total: *mut u64,
        free: *mut u64,
    ) -> Bool;
    pub fn GetWindowsDirectoryW(out: *mut u16, count: u32) -> u32;
    pub fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: Handle,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    pub fn DeviceIoControl(
        handle: Handle,
        code: u32,
        input: *const c_void,
        input_size: u32,
        output: *mut c_void,
        output_size: u32,
        returned: *mut u32,
        overlapped: Handle,
    ) -> Bool;
    pub fn LoadLibraryExW(name: *const u16, file: Handle, flags: u32) -> Handle;
    pub fn GetProcAddress(module: Handle, name: *const u8) -> *const c_void;
    pub fn FreeLibrary(module: Handle) -> Bool;
    pub fn OpenFileMappingW(access: u32, inherit: Bool, name: *const u16) -> Handle;
    pub fn MapViewOfFile(
        mapping: Handle,
        access: u32,
        high: u32,
        low: u32,
        size: usize,
    ) -> *const u8;
    pub fn UnmapViewOfFile(view: *const u8) -> Bool;
    pub fn GetLocalTime(time: *mut SystemTime);
    pub fn GetSystemPowerStatus(status: *mut SystemPowerStatus) -> Bool;
    pub fn GetSystemTimeAsFileTime(file_time: *mut u64);
    pub fn GlobalAlloc(flags: u32, bytes: usize) -> Handle;
    pub fn GlobalLock(mem: Handle) -> *mut c_void;
    pub fn GlobalUnlock(mem: Handle) -> Bool;
    pub fn GlobalFree(mem: Handle) -> Handle;
    pub fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> Handle;
    pub fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry32W) -> Bool;
    pub fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry32W) -> Bool;
    pub fn OpenProcess(access: u32, inherit: Bool, process_id: u32) -> Handle;
    pub fn GetProcessTimes(
        process: Handle,
        creation: *mut u64,
        exit: *mut u64,
        kernel: *mut u64,
        user: *mut u64,
    ) -> Bool;
}

#[link(name = "user32")]
extern "system" {
    pub fn UpdateLayeredWindow(
        window: Handle,
        dst_dc: Handle,
        dst_pt: *const Point,
        size: *const Size,
        src_dc: Handle,
        src_pt: *const Point,
        key: u32,
        blend: *const BlendFunction,
        flags: u32,
    ) -> Bool;
    pub fn GetForegroundWindow() -> Handle;
    pub fn IsIconic(window: Handle) -> Bool;
    pub fn GetClassNameW(window: Handle, buffer: *mut u16, max: i32) -> i32;
    pub fn SetFocus(window: Handle) -> Handle;
    pub fn GetDC(window: Handle) -> Handle;
    pub fn ReleaseDC(window: Handle, dc: Handle) -> i32;
    pub fn SetCursor(cursor: Handle) -> Handle;
    pub fn SetCapture(window: Handle) -> Handle;
    pub fn ReleaseCapture() -> Bool;
    pub fn MessageBeep(kind: u32) -> Bool;
    pub fn OpenClipboard(window: Handle) -> Bool;
    pub fn EmptyClipboard() -> Bool;
    pub fn SetClipboardData(format: u32, mem: Handle) -> Handle;
    pub fn GetClipboardData(format: u32) -> Handle;
    pub fn CloseClipboard() -> Bool;
    pub fn RegisterClassExW(class: *const WindowClass) -> u16;
    pub fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: Handle,
        menu: Handle,
        instance: Handle,
        param: Handle,
    ) -> Handle;
    pub fn DefWindowProcW(window: Handle, message: u32, w: usize, l: isize) -> isize;
    pub fn GetMessageW(message: *mut Message, window: Handle, min: u32, max: u32) -> Bool;
    pub fn TranslateMessage(message: *const Message) -> Bool;
    pub fn DispatchMessageW(message: *const Message) -> isize;
    pub fn PostMessageW(window: Handle, message: u32, w: usize, l: isize) -> Bool;
    pub fn PostQuitMessage(code: i32);
    pub fn DestroyWindow(window: Handle) -> Bool;
    pub fn SetTimer(window: Handle, id: usize, period: u32, callback: Handle) -> usize;
    pub fn KillTimer(window: Handle, id: usize) -> Bool;
    pub fn FindWindowW(class: *const u16, title: *const u16) -> Handle;
    pub fn GetWindowRect(window: Handle, rect: *mut Rect) -> Bool;
    pub fn GetWindow(window: Handle, command: u32) -> Handle;
    pub fn GetWindowLongW(window: Handle, index: i32) -> i32;
    pub fn SetWindowLongW(window: Handle, index: i32, value: i32) -> i32;
    pub fn GetWindowThreadProcessId(window: Handle, process: *mut u32) -> u32;
    pub fn IsWindowVisible(window: Handle) -> Bool;
    pub fn ShowWindow(window: Handle, command: i32) -> Bool;
    pub fn SetWindowPos(
        window: Handle,
        after: Handle,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        flags: u32,
    ) -> Bool;
    pub fn SetProcessDpiAwarenessContext(context: Handle) -> Bool;
    pub fn GetDpiForWindow(window: Handle) -> u32;
    pub fn MonitorFromWindow(window: Handle, flags: u32) -> Handle;
    pub fn GetMonitorInfoW(monitor: Handle, info: *mut MonitorInfo) -> Bool;
    pub fn SetWinEventHook(
        min: u32,
        max: u32,
        module: Handle,
        callback: Option<EventCallback>,
        process: u32,
        thread: u32,
        flags: u32,
    ) -> Handle;
    pub fn UnhookWinEvent(hook: Handle) -> Bool;
    pub fn RegisterWindowMessageW(name: *const u16) -> u32;
    pub fn BeginPaint(window: Handle, paint: *mut Paint) -> Handle;
    pub fn EndPaint(window: Handle, paint: *const Paint) -> Bool;
    pub fn LoadCursorW(module: Handle, name: *const u16) -> Handle;
    pub fn LoadIconW(module: Handle, name: *const u16) -> Handle;
    pub fn CreatePopupMenu() -> Handle;
    pub fn AppendMenuW(menu: Handle, flags: u32, item: usize, title: *const u16) -> Bool;
    pub fn TrackPopupMenu(
        menu: Handle,
        flags: u32,
        x: i32,
        y: i32,
        reserved: i32,
        window: Handle,
        rect: *const Rect,
    ) -> u32;
    pub fn DestroyMenu(menu: Handle) -> Bool;
    pub fn GetCursorPos(point: *mut Point) -> Bool;
    pub fn SetForegroundWindow(window: Handle) -> Bool;
    pub fn MessageBoxW(window: Handle, text: *const u16, title: *const u16, flags: u32) -> i32;
    pub fn ChangeWindowMessageFilterEx(
        window: Handle,
        message: u32,
        action: u32,
        change: *mut c_void,
    ) -> Bool;
}

#[link(name = "gdi32")]
extern "system" {
    pub fn CreateCompatibleDC(dc: Handle) -> Handle;
    pub fn DeleteDC(dc: Handle) -> Bool;
    pub fn BitBlt(
        dest: Handle,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        src: Handle,
        sx: i32,
        sy: i32,
        op: u32,
    ) -> Bool;
    pub fn SelectObject(dc: Handle, item: Handle) -> Handle;
    pub fn DeleteObject(item: Handle) -> Bool;
    pub fn CreateDIBSection(
        dc: Handle,
        bmi: *const BitmapInfoHeader,
        usage: u32,
        bits: *mut *mut c_void,
        section: Handle,
        offset: u32,
    ) -> Handle;
}

#[link(name = "shell32")]
extern "system" {
    pub fn Shell_NotifyIconW(message: u32, icon: *mut NotifyIcon) -> Bool;
    pub fn ShellExecuteW(
        window: Handle,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> Handle;
    pub fn IsUserAnAdmin() -> Bool;
    pub fn DragQueryFileW(drop: Handle, index: u32, out: *mut u16, count: u32) -> u32;
    pub fn DragAcceptFiles(window: Handle, accept: Bool);
    pub fn DragFinish(drop: Handle);
}

#[link(name = "ole32")]
extern "system" {
    pub fn OleInitialize(reserved: *mut c_void) -> i32;
    pub fn OleUninitialize();
    pub fn RegisterDragDrop(window: Handle, target: *mut c_void) -> i32;
    pub fn RevokeDragDrop(window: Handle) -> i32;
    pub fn DoDragDrop(
        data: *mut c_void,
        source: *mut c_void,
        ok_effects: u32,
        effect: *mut u32,
    ) -> i32;
    pub fn ReleaseStgMedium(medium: *mut c_void);
    pub fn CoTaskMemFree(pv: *mut c_void);
    pub fn PropVariantClear(pvar: *mut c_void) -> i32;
    pub fn CoCreateInstance(
        clsid: *const Guid,
        outer: *mut c_void,
        clsctx: u32,
        riid: *const Guid,
        out: *mut *mut c_void,
    ) -> i32;
    pub fn CreateStreamOnHGlobal(
        hglobal: Handle,
        delete_on_release: Bool,
        stream: *mut *mut c_void,
    ) -> i32;
}

#[link(name = "advapi32")]
extern "system" {
    pub fn RegOpenKeyExW(
        key: Handle,
        subkey: *const u16,
        options: u32,
        sam: u32,
        result: *mut Handle,
    ) -> i32;
    pub fn RegEnumKeyExW(
        key: Handle,
        index: u32,
        name: *mut u16,
        name_len: *mut u32,
        reserved: *mut u32,
        class: *mut u16,
        class_len: *mut u32,
        last_write: *mut u64,
    ) -> i32;
    pub fn RegCloseKey(key: Handle) -> i32;
    pub fn RegGetValueW(
        key: Handle,
        subkey: *const u16,
        value: *const u16,
        flags: u32,
        kind: *mut u32,
        data: *mut c_void,
        bytes: *mut u32,
    ) -> i32;
}

pub fn reg_u64(key: Handle, path: &str, name: &str) -> Option<u64> {
    let (mut data, mut bytes) = (0u64, 8u32);
    (unsafe {
        RegGetValueW(
            key,
            wide(path).as_ptr(),
            wide(name).as_ptr(),
            0x40,
            std::ptr::null_mut(),
            (&mut data as *mut u64).cast(),
            &mut bytes,
        )
    } == 0)
        .then_some(data)
}

pub const HKLM: Handle = 0x80000002u32 as i32 as isize;
pub const HKCU: Handle = 0x80000001u32 as i32 as isize;

pub fn reg_u32(key: Handle, path: &str, name: &str) -> Option<u32> {
    let (mut data, mut bytes) = (0, 4);
    (unsafe {
        RegGetValueW(
            key,
            wide(path).as_ptr(),
            wide(name).as_ptr(),
            0x10,
            std::ptr::null_mut(),
            (&mut data as *mut u32).cast(),
            &mut bytes,
        )
    } == 0)
        .then_some(data)
}

pub fn reg_string(key: Handle, path: &str, name: &str) -> Option<String> {
    let mut data = [0u16; 512];
    let mut bytes = (data.len() * 2) as u32;
    if unsafe {
        RegGetValueW(
            key,
            wide(path).as_ptr(),
            wide(name).as_ptr(),
            2,
            std::ptr::null_mut(),
            data.as_mut_ptr().cast(),
            &mut bytes,
        )
    } != 0
    {
        return None;
    }
    let count = (bytes as usize / 2).min(data.len());
    let end = data[..count].iter().position(|&c| c == 0).unwrap_or(count);
    Some(String::from_utf16_lossy(&data[..end]))
}

pub struct OwnedHandle(pub Handle);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

#[link(name = "user32")]
extern "system" {
    pub fn SendMessageW(window: Handle, message: u32, w: usize, l: isize) -> isize;
    pub fn RegisterPowerSettingNotification(
        window: Handle,
        setting: *const Guid,
        flags: u32,
    ) -> Handle;
    pub fn UnregisterPowerSettingNotification(handle: Handle) -> Bool;
    pub fn RegisterDeviceNotificationW(
        recipient: Handle,
        filter: *const c_void,
        flags: u32,
    ) -> Handle;
    pub fn UnregisterDeviceNotification(handle: Handle) -> Bool;
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Margins {
    pub left: i32,
    pub right: i32,
    pub top: i32,
    pub bottom: i32,
}

struct DwmApi {
    _lib: Library,
    extend_frame: unsafe extern "system" fn(Handle, *const Margins) -> i32,
    set_window_attr: unsafe extern "system" fn(Handle, u32, *const c_void, u32) -> i32,
}

unsafe impl Send for DwmApi {}
unsafe impl Sync for DwmApi {}

fn dwm_api() -> Option<&'static DwmApi> {
    static DWM: std::sync::OnceLock<Option<DwmApi>> = std::sync::OnceLock::new();
    DWM.get_or_init(|| unsafe {
        let lib = Library::system("dwmapi.dll")?;
        let extend_frame = std::mem::transmute::<
            *const c_void,
            unsafe extern "system" fn(Handle, *const Margins) -> i32,
        >(lib.symbol(b"DwmExtendFrameIntoClientArea\0")?);
        let set_window_attr = std::mem::transmute::<
            *const c_void,
            unsafe extern "system" fn(Handle, u32, *const c_void, u32) -> i32,
        >(lib.symbol(b"DwmSetWindowAttribute\0")?);
        Some(DwmApi {
            _lib: lib,
            extend_frame,
            set_window_attr,
        })
    })
    .as_ref()
}

pub fn dwm_extend_frame(hwnd: Handle, margins: &Margins) {
    if let Some(api) = dwm_api() {
        unsafe {
            (api.extend_frame)(hwnd, margins);
        }
    }
}

pub unsafe fn dwm_set_window_attribute(hwnd: Handle, attr: u32, value: *const c_void, size: u32) {
    if let Some(api) = dwm_api() {
        (api.set_window_attr)(hwnd, attr, value, size);
    }
}

pub struct Library(Handle);

impl Library {
    pub fn load(path: &std::path::Path) -> Option<Self> {
        use std::os::windows::ffi::OsStrExt;
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        wide.push(0);
        let module = unsafe { LoadLibraryExW(wide.as_ptr(), 0, 0x900) };
        (module != 0).then_some(Self(module))
    }
    pub fn system(name: &str) -> Option<Self> {
        let module = unsafe { LoadLibraryExW(wide(name).as_ptr(), 0, 0x800) };
        (module != 0).then_some(Self(module))
    }

    pub fn symbol(&self, name: &[u8]) -> Option<*const c_void> {
        debug_assert_eq!(name.last(), Some(&0));
        let pointer = unsafe { GetProcAddress(self.0, name.as_ptr()) };
        (!pointer.is_null()).then_some(pointer)
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe {
                FreeLibrary(self.0);
            }
        }
    }
}
