use crate::win::{
    reg_u64, wide, CoCreateInstance, CoTaskMemFree, GetSystemTimeAsFileTime, Guid, Handle, Library,
    PostMessageW, PropVariantClear, RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, HKCU,
};
use std::{
    ffi::c_void,
    mem::transmute,
    ptr::{null, null_mut},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Condvar, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub const WM_MEDIA_READY: u32 = 0x8012;

#[derive(Clone, Default)]
pub struct MediaSnapshot {
    pub has_session: bool,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub art_id: u64,
    pub art_bytes: Option<Arc<Vec<u8>>>,
    pub start_100ns: i64,
    pub end_100ns: i64,
    pub pos_100ns: i64,
    pub last_updated_ft: i64,
    pub volume: f32,
    pub muted: bool,
    pub peak: f32,
    pub mic_active: bool,
    pub cam_active: bool,
    pub audio_device_id: String,
    pub audio_device_name: String,
    pub audio_is_headphone: bool,
}

impl MediaSnapshot {
    pub fn timeline_ms(&self) -> (u64, u64, f32) {
        let mut now_ft = 0u64;
        unsafe {
            GetSystemTimeAsFileTime(&mut now_ft);
        }
        self.timeline_ms_at(now_ft)
    }

    pub fn timeline_ms_at(&self, now_ft: u64) -> (u64, u64, f32) {
        let dur = (self.end_100ns - self.start_100ns).max(0);
        if dur == 0 {
            return (0, 0, 0.0);
        }
        let mut raw_pos = self.pos_100ns;
        if self.playing && self.last_updated_ft > 0 && now_ft > self.last_updated_ft as u64 {
            let delta = (now_ft - self.last_updated_ft as u64).min(86_400 * 10_000_000) as i64;
            raw_pos = raw_pos.saturating_add(delta);
        }
        let pos = (raw_pos - self.start_100ns).clamp(0, dur);
        let elapsed_ms = (pos / 10_000) as u64;
        let total_ms = (dur / 10_000) as u64;
        let frac = (pos as f64 / dur as f64).clamp(0.0, 1.0) as f32;
        (elapsed_ms, total_ms, frac)
    }

    pub fn timeline_secs(&self) -> (u32, u32, f32) {
        let mut now_ft = 0u64;
        unsafe {
            GetSystemTimeAsFileTime(&mut now_ft);
        }
        self.timeline_secs_at(now_ft)
    }

    pub fn timeline_secs_at(&self, now_ft: u64) -> (u32, u32, f32) {
        let (elapsed_ms, total_ms, frac) = self.timeline_ms_at(now_ft);
        let elapsed = (elapsed_ms / 1000) as u32;
        let total = (total_ms / 1000) as u32;
        let remaining = total.saturating_sub(elapsed);
        (elapsed, remaining, frac)
    }
}

pub fn format_mmss(secs: u32) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

pub enum MediaCmd {
    TogglePlayPause,
    Next,
    Prev,
    SeekFraction(f32),
    SetVolume(f32),
    StepVolume(f32),
    ToggleMute,
}

#[derive(Default)]
struct WorkerControl {
    paused: bool,
    quit: bool,
}

pub struct MediaHandle {
    cmd_tx: Sender<MediaCmd>,
    snap_rx: Receiver<MediaSnapshot>,
    control: Arc<(Mutex<WorkerControl>, Condvar)>,
    thread: Option<JoinHandle<()>>,
    pub snapshot: MediaSnapshot,
}

impl MediaHandle {
    pub fn start(hwnd: Handle) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (snap_tx, snap_rx) = mpsc::channel();
        let control = Arc::new((Mutex::new(WorkerControl::default()), Condvar::new()));
        let worker_ctl = Arc::clone(&control);
        let thread = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(move || worker(hwnd, worker_ctl, cmd_rx, snap_tx))
            .ok();
        Self {
            cmd_tx,
            snap_rx,
            control,
            thread,
            snapshot: MediaSnapshot {
                volume: 0.5,
                ..Default::default()
            },
        }
    }

    pub fn send(&self, cmd: MediaCmd) {
        let _ = self.cmd_tx.send(cmd);
        self.control.1.notify_one();
    }

    pub fn set_paused(&self, paused: bool) {
        let (lock, wake) = &*self.control;
        let mut ctl = lock.lock().unwrap();
        if ctl.paused != paused {
            ctl.paused = paused;
            wake.notify_one();
        }
    }

    pub fn drain(&mut self) -> bool {
        let mut updated = false;
        while let Ok(snap) = self.snap_rx.try_recv() {
            self.snapshot = snap;
            updated = true;
        }
        updated
    }
}

impl Drop for MediaHandle {
    fn drop(&mut self) {
        {
            let (lock, wake) = &*self.control;
            let mut ctl = lock.lock().unwrap();
            ctl.quit = true;
            wake.notify_one();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[repr(C)]
struct HStringHeader {
    reserved: [u8; 24],
}

unsafe fn vtable_slot(obj: *mut c_void, slot: usize) -> *const c_void {
    *(*(obj as *mut *const *const c_void)).add(slot)
}

unsafe fn com_release(obj: *mut c_void) {
    if !obj.is_null() {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = transmute(vtable_slot(obj, 2));
        f(obj);
    }
}

unsafe fn com_qi(obj: *mut c_void, iid: &Guid) -> Option<*mut c_void> {
    if obj.is_null() {
        return None;
    }
    let f: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 =
        transmute(vtable_slot(obj, 0));
    let mut out = null_mut();
    (f(obj, iid, &mut out) >= 0 && !out.is_null()).then_some(out)
}

const IID_ASYNC_INFO: Guid = Guid {
    a: 0x0000_0036,
    b: 0x0000,
    c: 0x0000,
    d: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

const IID_ISTREAM: Guid = Guid {
    a: 0x0000_000C,
    b: 0x0000,
    c: 0x0000,
    d: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

const IID_SMTC_STATICS: Guid = Guid {
    a: 0x2050_C4EE,
    b: 0x11A0,
    c: 0x57DE,
    d: [0xAE, 0xD7, 0xC9, 0x7C, 0x70, 0x33, 0x82, 0x45],
};

const CLSID_MM_DEVICE_ENUMERATOR: Guid = Guid {
    a: 0xBCDE_0395,
    b: 0xE52F,
    c: 0x467C,
    d: [0x8E, 0x3D, 0xC4, 0x57, 0x92, 0x91, 0x69, 0x2E],
};

const IID_IMM_DEVICE_ENUMERATOR: Guid = Guid {
    a: 0xA956_64D2,
    b: 0x9614,
    c: 0x4F35,
    d: [0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6],
};

const IID_IAUDIO_ENDPOINT_VOLUME: Guid = Guid {
    a: 0x5CDF_2C82,
    b: 0x841E,
    c: 0x4546,
    d: [0x97, 0x22, 0x0C, 0xF7, 0x40, 0x78, 0x22, 0x9A],
};

const IID_IAUDIO_METER_INFO: Guid = Guid {
    a: 0xC022_16F6,
    b: 0x8C67,
    c: 0x4B5B,
    d: [0x9D, 0x00, 0xD0, 0x08, 0xE7, 0x3E, 0x00, 0x64],
};

struct WinRt {
    _combase: Library,
    _shcore: Option<Library>,
    ro_init: unsafe extern "system" fn(u32) -> i32,
    ro_uninit: unsafe extern "system" fn(),
    ro_get_factory: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32,
    win_str_ref:
        unsafe extern "system" fn(*const u16, u32, *mut HStringHeader, *mut *mut c_void) -> i32,
    win_str_buf: unsafe extern "system" fn(*mut c_void, *mut u32) -> *const u16,
    win_str_del: unsafe extern "system" fn(*mut c_void) -> i32,
    stream_over_ras:
        Option<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>,
}

impl WinRt {
    fn load() -> Option<Self> {
        let combase = Library::system("combase.dll")?;
        let shcore = Library::system("shcore.dll");
        unsafe {
            let stream_over_ras = shcore
                .as_ref()
                .and_then(|lib| lib.symbol(b"CreateStreamOverRandomAccessStream\0"))
                .map(|ptr| {
                    transmute::<
                        *const c_void,
                        unsafe extern "system" fn(
                            *mut c_void,
                            *const Guid,
                            *mut *mut c_void,
                        ) -> i32,
                    >(ptr)
                });
            Some(Self {
                ro_init: transmute::<*const c_void, unsafe extern "system" fn(u32) -> i32>(
                    combase.symbol(b"RoInitialize\0")?,
                ),
                ro_uninit: transmute::<*const c_void, unsafe extern "system" fn()>(
                    combase.symbol(b"RoUninitialize\0")?,
                ),
                ro_get_factory: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32,
                >(combase.symbol(b"RoGetActivationFactory\0")?),
                win_str_ref: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(
                        *const u16,
                        u32,
                        *mut HStringHeader,
                        *mut *mut c_void,
                    ) -> i32,
                >(combase.symbol(b"WindowsCreateStringReference\0")?),
                win_str_buf: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(*mut c_void, *mut u32) -> *const u16,
                >(combase.symbol(b"WindowsGetStringRawBuffer\0")?),
                win_str_del: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(*mut c_void) -> i32,
                >(combase.symbol(b"WindowsDeleteString\0")?),
                stream_over_ras,
                _combase: combase,
                _shcore: shcore,
            })
        }
    }

    unsafe fn hstring_to_rust(&self, hstr: *mut c_void) -> String {
        if hstr.is_null() {
            return String::new();
        }
        let mut len = 0u32;
        let ptr = (self.win_str_buf)(hstr, &mut len);
        let s = if !ptr.is_null() && len > 0 {
            String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len as usize))
        } else {
            String::new()
        };
        (self.win_str_del)(hstr);
        s
    }

    unsafe fn await_op_ptr(&self, op: *mut c_void) -> Option<*mut c_void> {
        let mut out = null_mut();
        if !await_async_ready(op) {
            com_release(op);
            return None;
        }
        let get_results: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 =
            transmute(vtable_slot(op, 8));
        let hr = get_results(op, &mut out);
        com_release(op);
        (hr >= 0 && !out.is_null()).then_some(out)
    }

    unsafe fn await_op_bool(&self, op: *mut c_void) -> bool {
        if !await_async_ready(op) {
            com_release(op);
            return false;
        }
        let get_results: unsafe extern "system" fn(*mut c_void, *mut u8) -> i32 =
            transmute(vtable_slot(op, 8));
        let mut res = 0u8;
        let hr = get_results(op, &mut res);
        com_release(op);
        hr >= 0 && res != 0
    }
}

unsafe fn await_async_ready(op: *mut c_void) -> bool {
    if op.is_null() {
        return false;
    }
    let Some(info) = com_qi(op, &IID_ASYNC_INFO) else {
        return false;
    };
    let get_status: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32 =
        transmute(vtable_slot(info, 7));
    let mut ok = false;
    for _ in 0..160 {
        let mut status = 0i32;
        if get_status(info, &mut status) < 0 {
            break;
        }
        if status == 1 {
            ok = true;
            break;
        }
        if status != 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    com_release(info);
    ok
}

struct Smtc {
    rt: WinRt,
    manager: *mut c_void,
    last_track: (String, String),
    art_id: u64,
    art_bytes: Option<Arc<Vec<u8>>>,
}

impl Smtc {
    fn open() -> Option<Self> {
        let rt = WinRt::load()?;
        unsafe {
            let hr = (rt.ro_init)(1); // RO_INIT_MULTITHREADED
            if hr < 0 && hr != 1 && hr != (-2147417850i32) {
                return None;
            }
            let manager = Self::request_manager(&rt);
            Some(Self {
                rt,
                manager: manager.unwrap_or(null_mut()),
                last_track: (String::new(), String::new()),
                art_id: 0,
                art_bytes: None,
            })
        }
    }

    unsafe fn request_manager(rt: &WinRt) -> Option<*mut c_void> {
        let class = wide("Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager");
        let mut header = HStringHeader { reserved: [0; 24] };
        let mut hstr = null_mut();
        if (rt.win_str_ref)(
            class.as_ptr(),
            (class.len() - 1) as u32,
            &mut header,
            &mut hstr,
        ) < 0
        {
            return None;
        }
        let mut factory = null_mut();
        if (rt.ro_get_factory)(hstr, &IID_SMTC_STATICS, &mut factory) < 0 || factory.is_null() {
            return None;
        }
        let req_async: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 =
            transmute(vtable_slot(factory, 6));
        let mut op = null_mut();
        let hr = req_async(factory, &mut op);
        com_release(factory);
        if hr < 0 {
            return None;
        }
        rt.await_op_ptr(op)
    }

    unsafe fn current_session(&mut self) -> *mut c_void {
        if self.manager.is_null() {
            self.manager = Self::request_manager(&self.rt).unwrap_or(null_mut());
            if self.manager.is_null() {
                return null_mut();
            }
        }
        let get_cur: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 =
            transmute(vtable_slot(self.manager, 6));
        let mut session = null_mut();
        if get_cur(self.manager, &mut session) < 0 {
            com_release(self.manager);
            self.manager = null_mut();
            return null_mut();
        }
        session
    }

    unsafe fn read_thumbnail(&self, props: *mut c_void) -> Option<Arc<Vec<u8>>> {
        let stream_fn = self.rt.stream_over_ras?;
        let get_thumb: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 =
            transmute(vtable_slot(props, 15));
        let mut ras_ref = null_mut();
        if get_thumb(props, &mut ras_ref) < 0 || ras_ref.is_null() {
            return None;
        }
        let open_read: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 =
            transmute(vtable_slot(ras_ref, 6));
        let mut op = null_mut();
        let hr = open_read(ras_ref, &mut op);
        com_release(ras_ref);
        if hr < 0 {
            return None;
        }
        let ras = self.rt.await_op_ptr(op)?;
        let mut istream = null_mut();
        let hr = stream_fn(ras, &IID_ISTREAM, &mut istream);
        com_release(ras);
        if hr < 0 || istream.is_null() {
            return None;
        }
        let read_fn: unsafe extern "system" fn(*mut c_void, *mut u8, u32, *mut u32) -> i32 =
            transmute(vtable_slot(istream, 3));
        let mut data = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let mut got = 0u32;
            let hr = read_fn(istream, chunk.as_mut_ptr(), chunk.len() as u32, &mut got);
            if hr < 0 || got == 0 {
                break;
            }
            if data.len() + got as usize > 2 * 1024 * 1024 {
                data.clear();
                break;
            }
            data.extend_from_slice(&chunk[..got as usize]);
        }
        com_release(istream);
        (!data.is_empty()).then(|| Arc::new(data))
    }

    fn poll(&mut self, snap: &mut MediaSnapshot) {
        unsafe {
            let session = self.current_session();
            if session.is_null() {
                snap.has_session = false;
                snap.playing = false;
                snap.title.clear();
                snap.artist.clear();
                snap.album.clear();
                snap.start_100ns = 0;
                snap.end_100ns = 0;
                snap.pos_100ns = 0;
                snap.last_updated_ft = 0;
                if !self.last_track.0.is_empty() || !self.last_track.1.is_empty() {
                    self.last_track = (String::new(), String::new());
                    self.art_bytes = None;
                    self.art_id += 1;
                }
                snap.art_id = self.art_id;
                snap.art_bytes = None;
                return;
            }
            snap.has_session = true;

            // PlaybackInfo [9] -> get_PlaybackStatus [7] (4 = Playing)
            let get_pb: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 =
                transmute(vtable_slot(session, 9));
            let mut pb = null_mut();
            if get_pb(session, &mut pb) >= 0 && !pb.is_null() {
                let get_status: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32 =
                    transmute(vtable_slot(pb, 7));
                let mut status = 0i32;
                if get_status(pb, &mut status) >= 0 {
                    snap.playing = status == 4;
                }
                com_release(pb);
            }

            // TimelineProperties [8] -> StartTime [6], EndTime [7], Position [10], LastUpdatedTime [11]
            let get_tl: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 =
                transmute(vtable_slot(session, 8));
            let mut tl = null_mut();
            if get_tl(session, &mut tl) >= 0 && !tl.is_null() {
                let get_i64 = |slot: usize| -> i64 {
                    let f: unsafe extern "system" fn(*mut c_void, *mut i64) -> i32 =
                        transmute(vtable_slot(tl, slot));
                    let mut v = 0i64;
                    if f(tl, &mut v) >= 0 {
                        v
                    } else {
                        0
                    }
                };
                snap.start_100ns = get_i64(6);
                snap.end_100ns = get_i64(7);
                snap.pos_100ns = get_i64(10);
                snap.last_updated_ft = get_i64(11);
                com_release(tl);
            }

            // MediaProperties [7]
            let get_media: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 =
                transmute(vtable_slot(session, 7));
            let mut op = null_mut();
            if get_media(session, &mut op) >= 0 {
                if let Some(props) = self.rt.await_op_ptr(op) {
                    let get_str = |slot: usize| -> String {
                        let f: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 =
                            transmute(vtable_slot(props, slot));
                        let mut hs = null_mut();
                        if f(props, &mut hs) >= 0 {
                            self.rt.hstring_to_rust(hs)
                        } else {
                            String::new()
                        }
                    };
                    let title = get_str(6);
                    let artist = get_str(9);
                    let album = get_str(10);
                    let key = (title.clone(), artist.clone());
                    if key != self.last_track {
                        self.last_track = key;
                        self.art_bytes = self.read_thumbnail(props);
                        self.art_id += 1;
                    }
                    snap.title = title;
                    snap.artist = artist;
                    snap.album = album;
                    snap.art_id = self.art_id;
                    snap.art_bytes = self.art_bytes.clone();
                    com_release(props);
                }
            }
            com_release(session);
        }
    }

    fn command(&mut self, cmd: &MediaCmd, snap: &MediaSnapshot) {
        unsafe {
            let session = self.current_session();
            if session.is_null() {
                return;
            }
            match cmd {
                MediaCmd::TogglePlayPause => {
                    let f: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 =
                        transmute(vtable_slot(session, 20));
                    let mut op = null_mut();
                    if f(session, &mut op) >= 0 {
                        self.rt.await_op_bool(op);
                    }
                }
                MediaCmd::Next => {
                    let f: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 =
                        transmute(vtable_slot(session, 16));
                    let mut op = null_mut();
                    if f(session, &mut op) >= 0 {
                        self.rt.await_op_bool(op);
                    }
                }
                MediaCmd::Prev => {
                    let f: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 =
                        transmute(vtable_slot(session, 17));
                    let mut op = null_mut();
                    if f(session, &mut op) >= 0 {
                        self.rt.await_op_bool(op);
                    }
                }
                MediaCmd::SeekFraction(frac) => {
                    let dur = (snap.end_100ns - snap.start_100ns).max(0);
                    if dur > 0 {
                        let target = snap.start_100ns
                            + ((dur as f64) * (*frac as f64).clamp(0.0, 1.0)) as i64;
                        let f: unsafe extern "system" fn(
                            *mut c_void,
                            i64,
                            *mut *mut c_void,
                        ) -> i32 = transmute(vtable_slot(session, 24));
                        let mut op = null_mut();
                        if f(session, target, &mut op) >= 0 {
                            self.rt.await_op_bool(op);
                        }
                    }
                }
                _ => {}
            }
            com_release(session);
        }
    }
}

impl Drop for Smtc {
    fn drop(&mut self) {
        unsafe {
            com_release(self.manager);
            (self.rt.ro_uninit)();
        }
    }
}

#[repr(C)]
struct PropertyKey {
    fmtid: Guid,
    pid: u32,
}

const PKEY_DEVICE_FRIENDLY_NAME: PropertyKey = PropertyKey {
    fmtid: Guid {
        a: 0xa45c_254e,
        b: 0xdf1c,
        c: 0x4efd,
        d: [0x80, 0x20, 0x67, 0xd1, 0x46, 0xa8, 0x50, 0xe0],
    },
    pid: 14,
};

unsafe fn utf16_ptr_to_string(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while len < 1024 && *ptr.add(len) != 0 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
}

fn classify_headphone(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "headphone",
        "headset",
        "earbud",
        "earphone",
        "airpod",
        "buds",
        "bluetooth",
        "tai nghe",
        "wh-1000",
        "wf-1000",
    ]
    .iter()
    .any(|kw| lower.contains(kw))
}

struct AudioEndpoint {
    volume: *mut c_void,
    meter: *mut c_void,
    device_id: String,
    device_name: String,
    is_headphone: bool,
}

impl AudioEndpoint {
    unsafe fn default_device_raw() -> Option<*mut c_void> {
        let mut enumerator = null_mut();
        if CoCreateInstance(
            &CLSID_MM_DEVICE_ENUMERATOR,
            null_mut(),
            23,
            &IID_IMM_DEVICE_ENUMERATOR,
            &mut enumerator,
        ) < 0
            || enumerator.is_null()
        {
            return None;
        }
        let get_def: unsafe extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void) -> i32 =
            transmute(vtable_slot(enumerator, 4));
        let mut device = null_mut();
        let hr = get_def(enumerator, 0, 1, &mut device);
        com_release(enumerator);
        (hr >= 0 && !device.is_null()).then_some(device)
    }

    fn default_device_id() -> Option<String> {
        unsafe {
            let device = Self::default_device_raw()?;
            let get_id: unsafe extern "system" fn(*mut c_void, *mut *mut u16) -> i32 =
                transmute(vtable_slot(device, 5));
            let mut id_ptr = null_mut();
            let hr = get_id(device, &mut id_ptr);
            com_release(device);
            if hr >= 0 && !id_ptr.is_null() {
                let s = utf16_ptr_to_string(id_ptr);
                CoTaskMemFree(id_ptr.cast());
                Some(s)
            } else {
                None
            }
        }
    }

    fn open() -> Option<Self> {
        unsafe {
            let device = Self::default_device_raw()?;
            let mut device_id = String::new();
            let get_id: unsafe extern "system" fn(*mut c_void, *mut *mut u16) -> i32 =
                transmute(vtable_slot(device, 5));
            let mut id_ptr = null_mut();
            if get_id(device, &mut id_ptr) >= 0 && !id_ptr.is_null() {
                device_id = utf16_ptr_to_string(id_ptr);
                CoTaskMemFree(id_ptr.cast());
            }

            let mut device_name = String::new();
            let open_store: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32 =
                transmute(vtable_slot(device, 4));
            let mut store = null_mut();
            if open_store(device, 0, &mut store) >= 0 && !store.is_null() {
                let get_val: unsafe extern "system" fn(
                    *mut c_void,
                    *const PropertyKey,
                    *mut c_void,
                ) -> i32 = transmute(vtable_slot(store, 5));
                let mut pv = [0u8; 24];
                if get_val(store, &PKEY_DEVICE_FRIENDLY_NAME, pv.as_mut_ptr().cast()) >= 0 {
                    let vt = u16::from_le_bytes([pv[0], pv[1]]);
                    if vt == 31 {
                        let ptr = usize::from_le_bytes(pv[8..16].try_into().unwrap()) as *const u16;
                        device_name = utf16_ptr_to_string(ptr);
                    }
                    PropVariantClear(pv.as_mut_ptr().cast());
                }
                com_release(store);
            }

            let activate: unsafe extern "system" fn(
                *mut c_void,
                *const Guid,
                u32,
                *mut c_void,
                *mut *mut c_void,
            ) -> i32 = transmute(vtable_slot(device, 3));
            let mut volume = null_mut();
            let mut meter = null_mut();
            let _ = activate(
                device,
                &IID_IAUDIO_ENDPOINT_VOLUME,
                23,
                null_mut(),
                &mut volume,
            );
            let _ = activate(device, &IID_IAUDIO_METER_INFO, 23, null_mut(), &mut meter);
            com_release(device);
            if volume.is_null() && meter.is_null() {
                None
            } else {
                let is_headphone = classify_headphone(&device_name);
                Some(Self {
                    volume,
                    meter,
                    device_id,
                    device_name,
                    is_headphone,
                })
            }
        }
    }

    fn poll(&self) -> Option<(f32, bool, f32)> {
        unsafe {
            let mut vol = 0.0f32;
            let mut muted = 0i32;
            if !self.volume.is_null() {
                let get_vol: unsafe extern "system" fn(*mut c_void, *mut f32) -> i32 =
                    transmute(vtable_slot(self.volume, 9));
                let get_mute: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32 =
                    transmute(vtable_slot(self.volume, 15));
                if get_vol(self.volume, &mut vol) < 0 || get_mute(self.volume, &mut muted) < 0 {
                    return None;
                }
            }
            let mut peak = 0.0f32;
            if !self.meter.is_null() {
                let get_peak: unsafe extern "system" fn(*mut c_void, *mut f32) -> i32 =
                    transmute(vtable_slot(self.meter, 3));
                if get_peak(self.meter, &mut peak) < 0 {
                    return None;
                }
            }
            Some((vol.clamp(0.0, 1.0), muted != 0, peak.clamp(0.0, 1.0)))
        }
    }

    fn set_volume(&self, level: f32) -> bool {
        if self.volume.is_null() {
            return false;
        }
        unsafe {
            let f: unsafe extern "system" fn(*mut c_void, f32, *const Guid) -> i32 =
                transmute(vtable_slot(self.volume, 7));
            f(self.volume, level.clamp(0.0, 1.0), null()) >= 0
        }
    }

    fn set_mute(&self, mute: bool) -> bool {
        if self.volume.is_null() {
            return false;
        }
        unsafe {
            let f: unsafe extern "system" fn(*mut c_void, i32, *const Guid) -> i32 =
                transmute(vtable_slot(self.volume, 14));
            f(self.volume, i32::from(mute), null()) >= 0
        }
    }
}

impl Drop for AudioEndpoint {
    fn drop(&mut self) {
        unsafe {
            com_release(self.volume);
            com_release(self.meter);
        }
    }
}

fn consent_active(device: &str) -> bool {
    let base = format!(
        r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\{device}"
    );
    check_consent_key(&base, true)
}

fn check_consent_key(path: &str, recurse_non_packaged: bool) -> bool {
    let mut hkey = 0;
    if unsafe { RegOpenKeyExW(HKCU, wide(path).as_ptr(), 0, 0x20019, &mut hkey) } != 0 || hkey == 0
    {
        return false;
    }
    let mut active = false;
    let mut index = 0u32;
    let mut name = [0u16; 256];
    loop {
        let mut len = name.len() as u32;
        let res = unsafe {
            RegEnumKeyExW(
                hkey,
                index,
                name.as_mut_ptr(),
                &mut len,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        if res != 0 {
            break;
        }
        index += 1;
        let sub = String::from_utf16_lossy(&name[..len as usize]);
        let full = format!(r"{path}\{sub}");
        if recurse_non_packaged && sub.eq_ignore_ascii_case("NonPackaged") {
            if check_consent_key(&full, false) {
                active = true;
                break;
            }
        } else if let Some(start) = reg_u64(HKCU, &full, "LastUsedTimeStart") {
            let stop = reg_u64(HKCU, &full, "LastUsedTimeStop").unwrap_or(0);
            if start > 0 && stop == 0 {
                active = true;
                break;
            }
        }
    }
    unsafe {
        RegCloseKey(hkey);
    }
    active
}

fn worker(
    hwnd: Handle,
    control: Arc<(Mutex<WorkerControl>, Condvar)>,
    cmd_rx: Receiver<MediaCmd>,
    snap_tx: Sender<MediaSnapshot>,
) {
    let mut smtc = Smtc::open();
    let mut audio = AudioEndpoint::open();
    let mut snap = MediaSnapshot {
        volume: 0.5,
        ..Default::default()
    };
    if let Some(ep) = &audio {
        snap.audio_device_id = ep.device_id.clone();
        snap.audio_device_name = ep.device_name.clone();
        snap.audio_is_headphone = ep.is_headphone;
    }
    let mut next_privacy = Instant::now();
    let mut next_dev_check = Instant::now();
    let (lock, wake) = &*control;

    loop {
        let mut ctl = lock.lock().unwrap();
        while ctl.paused && !ctl.quit {
            ctl = wake.wait(ctl).unwrap();
        }
        if ctl.quit {
            break;
        }
        drop(ctl);

        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                MediaCmd::SetVolume(v) => {
                    if audio.is_none() {
                        audio = AudioEndpoint::open();
                    }
                    if let Some(ep) = &audio {
                        if !ep.set_volume(v) {
                            audio = AudioEndpoint::open();
                        }
                    }
                }
                MediaCmd::StepVolume(delta) => {
                    if audio.is_none() {
                        audio = AudioEndpoint::open();
                    }
                    if let Some(ep) = &audio {
                        if let Some((cur, _, _)) = ep.poll() {
                            let _ = ep.set_volume(cur + delta);
                        }
                    }
                }
                MediaCmd::ToggleMute => {
                    if audio.is_none() {
                        audio = AudioEndpoint::open();
                    }
                    if let Some(ep) = &audio {
                        if let Some((_, muted, _)) = ep.poll() {
                            let _ = ep.set_mute(!muted);
                        }
                    }
                }
                other => {
                    if let Some(s) = smtc.as_mut() {
                        s.command(&other, &snap);
                    }
                }
            }
        }

        if let Some(s) = smtc.as_mut() {
            s.poll(&mut snap);
        }
        let now = Instant::now();
        if now >= next_dev_check {
            next_dev_check = now + Duration::from_millis(1200);
            if let Some(cur_id) = AudioEndpoint::default_device_id() {
                if audio.as_ref().is_none_or(|ep| ep.device_id != cur_id) {
                    audio = AudioEndpoint::open();
                }
            }
        }
        if audio.is_none() {
            audio = AudioEndpoint::open();
        }
        if let Some(ep) = &audio {
            snap.audio_device_id = ep.device_id.clone();
            snap.audio_device_name = ep.device_name.clone();
            snap.audio_is_headphone = ep.is_headphone;
        }
        if let Some((vol, muted, peak)) = audio.as_ref().and_then(AudioEndpoint::poll) {
            snap.volume = vol;
            snap.muted = muted;
            snap.peak = peak;
        } else {
            audio = AudioEndpoint::open();
            snap.peak = 0.0;
        }

        if now >= next_privacy {
            next_privacy = now + Duration::from_secs(2);
            snap.mic_active = consent_active("microphone");
            snap.cam_active = consent_active("webcam");
        }

        if snap_tx.send(snap.clone()).is_ok() {
            unsafe {
                PostMessageW(hwnd, WM_MEDIA_READY, 0, 0);
            }
        }

        let interval = if snap.playing || snap.peak > 0.01 {
            Duration::from_millis(80)
        } else {
            Duration::from_millis(800)
        };
        let ctl = lock.lock().unwrap();
        if ctl.quit {
            break;
        }
        drop(wake.wait_timeout(ctl, interval).unwrap());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_extrapolates_and_clamps() {
        let mut snap = MediaSnapshot {
            playing: true,
            start_100ns: 0,
            end_100ns: 275 * 10_000_000, // 4m35s
            pos_100ns: 4 * 10_000_000,   // 4s
            last_updated_ft: 1_000_000_000,
            ..Default::default()
        };
        let (elapsed, rem, frac) = snap.timeline_secs_at(1_000_000_000 + 10 * 10_000_000);
        assert_eq!(elapsed, 14);
        assert_eq!(rem, 261);
        assert!(frac > 0.05 && frac < 0.06);

        snap.playing = false;
        let (elapsed2, rem2, _) = snap.timeline_secs_at(1_000_000_000 + 100 * 10_000_000);
        assert_eq!(elapsed2, 4);
        assert_eq!(rem2, 271);
        assert_eq!(format_mmss(271), "04:31");
    }
}
