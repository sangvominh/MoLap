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
    pub fn overlaps(self, other: Self) -> bool {
        self.left < other.right
            && self.right > other.left
            && self.top < other.bottom
            && self.bottom > other.top
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
}

#[link(name = "user32")]
extern "system" {
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
    pub fn FindWindowExW(
        parent: Handle,
        after: Handle,
        class: *const u16,
        title: *const u16,
    ) -> Handle;
    pub fn GetWindowRect(window: Handle, rect: *mut Rect) -> Bool;
    pub fn GetClientRect(window: Handle, rect: *mut Rect) -> Bool;
    pub fn GetWindow(window: Handle, command: u32) -> Handle;
    pub fn GetWindowLongW(window: Handle, index: i32) -> i32;
    pub fn SetWindowLongW(window: Handle, index: i32, value: i32) -> i32;
    pub fn SetParent(window: Handle, parent: Handle) -> Handle;
    pub fn GetAncestor(window: Handle, flags: u32) -> Handle;
    pub fn ScreenToClient(window: Handle, point: *mut Point) -> Bool;
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
    pub fn SetLayeredWindowAttributes(window: Handle, key: u32, alpha: u8, flags: u32) -> Bool;
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
    pub fn InvalidateRect(window: Handle, rect: *const Rect, erase: Bool) -> Bool;
    pub fn BeginPaint(window: Handle, paint: *mut Paint) -> Handle;
    pub fn EndPaint(window: Handle, paint: *const Paint) -> Bool;
    pub fn FillRect(dc: Handle, rect: *const Rect, brush: Handle) -> i32;
    pub fn DrawTextW(dc: Handle, text: *const u16, count: i32, rect: *mut Rect, format: u32)
        -> i32;
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
    pub fn CreateCompatibleBitmap(dc: Handle, w: i32, h: i32) -> Handle;
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
    pub fn CreateSolidBrush(color: u32) -> Handle;
    pub fn CreatePen(style: i32, width: i32, color: u32) -> Handle;
    pub fn GetStockObject(index: i32) -> Handle;
    pub fn SelectObject(dc: Handle, item: Handle) -> Handle;
    pub fn DeleteObject(item: Handle) -> Bool;
    pub fn SetBkMode(dc: Handle, mode: i32) -> i32;
    pub fn SetTextColor(dc: Handle, color: u32) -> u32;
    pub fn CreateFontW(
        h: i32,
        w: i32,
        escapement: i32,
        orientation: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strike: u32,
        charset: u32,
        output: u32,
        clip: u32,
        quality: u32,
        pitch: u32,
        face: *const u16,
    ) -> Handle;
    pub fn GetGlyphIndicesW(
        dc: Handle,
        text: *const u16,
        count: i32,
        glyphs: *mut u16,
        flags: u32,
    ) -> u32;
    pub fn Rectangle(dc: Handle, left: i32, top: i32, right: i32, bottom: i32) -> Bool;
    pub fn RoundRect(
        dc: Handle,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        w: i32,
        h: i32,
    ) -> Bool;
    pub fn Ellipse(dc: Handle, left: i32, top: i32, right: i32, bottom: i32) -> Bool;
    pub fn MoveToEx(dc: Handle, x: i32, y: i32, old: *mut Point) -> Bool;
    pub fn LineTo(dc: Handle, x: i32, y: i32) -> Bool;
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
}

#[link(name = "advapi32")]
extern "system" {
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
}

#[link(name = "comctl32")]
extern "system" {
    pub fn InitCommonControls();
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
