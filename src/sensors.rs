use crate::win::*;
use std::{
    collections::BTreeMap,
    ffi::c_void,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::{mpsc::SyncSender, Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

pub const SAMPLE_READY: u32 = 0x8001;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Cpu,
    Ram,
    Gpu,
    Disk,
}

#[derive(Clone, PartialEq)]
pub struct Metric {
    pub kind: Kind,
    pub label: String,
    pub value: String,
    pub temperature: Option<i32>,
    pub detail: String,
    pub capacity: Option<f64>,
    pub hog: Option<(String, u32)>,
}

fn percent(value: Option<f64>) -> String {
    value
        .map(|v| format!("{}%", v.clamp(0., 100.).round() as u32))
        .unwrap_or_else(|| "--".into())
}

pub fn worker(
    window: Handle,
    control: Arc<(Mutex<Control>, Condvar)>,
    tx: SyncSender<Vec<Metric>>,
) {
    let mut monitor = Monitor::new();
    let (lock, wake) = &*control;
    loop {
        let mut state = lock.lock().unwrap();
        let mut resumed = false;
        while state.paused && !state.quit {
            resumed = true;
            state = wake.wait(state).unwrap();
        }
        if state.quit {
            break;
        }
        drop(state);
        if resumed {
            monitor.reset_baseline();
            let state = lock.lock().unwrap();
            if state.quit {
                break;
            }
            if state.paused {
                continue;
            }
            let (state, _) = wake
                .wait_timeout(state, Duration::from_millis(250))
                .unwrap();
            if state.quit {
                break;
            }
            if state.paused {
                continue;
            }
        }
        if tx.try_send(monitor.sample()).is_ok() {
            unsafe {
                PostMessageW(window, SAMPLE_READY, 0, 0);
            }
        }
        let state = lock.lock().unwrap();
        if state.quit {
            break;
        }
        drop(
            wake.wait_timeout(state, Duration::from_millis(1000))
                .unwrap(),
        );
    }
}

#[derive(Default)]
pub struct Control {
    pub paused: bool,
    pub quit: bool,
}

#[repr(C)]
struct Memory {
    size: u32,
    load: u32,
    total: u64,
    available: u64,
    other: [u64; 5],
}

#[repr(C)]
struct CounterItem {
    name: *const u16,
    status: u32,
    value: f64,
}

struct Pdh {
    _library: Library,
    open: unsafe extern "system" fn(*const u16, usize, *mut Handle) -> u32,
    add: unsafe extern "system" fn(Handle, *const u16, usize, *mut Handle) -> u32,
    collect: unsafe extern "system" fn(Handle) -> u32,
    array: unsafe extern "system" fn(Handle, u32, *mut u32, *mut u32, *mut c_void) -> u32,
    close: unsafe extern "system" fn(Handle) -> u32,
}

impl Pdh {
    fn load() -> Option<Self> {
        let library = Library::system("pdh.dll")?;
        unsafe {
            Some(Self {
                open: std::mem::transmute::<
                    *const c_void,
                    unsafe extern "system" fn(*const u16, usize, *mut Handle) -> u32,
                >(library.symbol(b"PdhOpenQueryW\0")?),
                add: std::mem::transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Handle, *const u16, usize, *mut Handle) -> u32,
                >(library.symbol(b"PdhAddEnglishCounterW\0")?),
                collect: std::mem::transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Handle) -> u32,
                >(library.symbol(b"PdhCollectQueryData\0")?),
                array: std::mem::transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Handle, u32, *mut u32, *mut u32, *mut c_void) -> u32,
                >(library.symbol(b"PdhGetFormattedCounterArrayW\0")?),
                close: std::mem::transmute::<*const c_void, unsafe extern "system" fn(Handle) -> u32>(
                    library.symbol(b"PdhCloseQuery\0")?,
                ),
                _library: library,
            })
        }
    }
}

struct Counters {
    api: Option<Pdh>,
    query: Handle,
    gpu: Handle,
    disk: Handle,
    thermal: Handle,
    buffer: Vec<u64>,
    name: String,
}

impl Counters {
    fn new() -> Self {
        let (mut query, mut gpu, mut disk, mut thermal) = (0, 0, 0, 0);
        let api = Pdh::load();
        if let Some(pdh) = &api {
            unsafe {
                if (pdh.open)(null(), 0, &mut query) == 0 {
                    for (path, slot) in [
                        (r"\GPU Engine(*)\Utilization Percentage", &mut gpu),
                        (r"\PhysicalDisk(*)\% Idle Time", &mut disk),
                        (r"\Thermal Zone Information(*)\Temperature", &mut thermal),
                    ] {
                        if (pdh.add)(query, wide(path).as_ptr(), 0, slot) != 0 {
                            *slot = 0;
                        }
                    }
                    (pdh.collect)(query);
                }
            }
        }
        Self {
            api,
            query,
            gpu,
            disk,
            thermal,
            buffer: Vec::new(),
            name: String::new(),
        }
    }

    fn collect(&self) -> bool {
        self.query != 0
            && self
                .api
                .as_ref()
                .is_some_and(|api| unsafe { (api.collect)(self.query) == 0 })
    }

    fn read(&mut self, counter: Handle, mut item: impl FnMut(&str, f64)) -> bool {
        if counter == 0 {
            return false;
        }
        let Some(array) = self.api.as_ref().map(|api| api.array) else {
            return false;
        };
        const MORE: u32 = 0x800007d2;
        for _ in 0..3 {
            let mut bytes = (self.buffer.len() * 8) as u32;
            let mut count = 0;
            let buffer = if bytes == 0 {
                null_mut()
            } else {
                self.buffer.as_mut_ptr().cast()
            };
            let status = unsafe { array(counter, 0x8200, &mut bytes, &mut count, buffer) };
            if status == MORE && bytes > 0 && bytes <= 16 * 1024 * 1024 {
                self.buffer.resize((bytes as usize).div_ceil(8), 0);
                continue;
            }
            if status != 0 || count as usize > self.buffer.len() * 8 / size_of::<CounterItem>() {
                return false;
            }
            let mut valid = false;
            let buffer = &self.buffer;
            let name = &mut self.name;
            for index in 0..count as usize {
                let raw = unsafe { &*buffer.as_ptr().cast::<CounterItem>().add(index) };
                if raw.name.is_null() || raw.status > 1 || !raw.value.is_finite() || raw.value < 0.
                {
                    continue;
                }
                let mut end = 0;
                unsafe {
                    while end < 512 && *raw.name.add(end) != 0 {
                        end += 1;
                    }
                    name.clear();
                    for ch in char::decode_utf16(
                        std::slice::from_raw_parts(raw.name, end).iter().copied(),
                    ) {
                        name.push(ch.unwrap_or(char::REPLACEMENT_CHARACTER));
                    }
                    item(name, raw.value);
                }
                valid = true;
            }
            return valid;
        }
        false
    }
}

impl Drop for Counters {
    fn drop(&mut self) {
        if self.query != 0 {
            if let Some(api) = &self.api {
                unsafe {
                    (api.close)(self.query);
                }
            }
        }
    }
}

#[repr(C)]
struct AdapterDesc {
    name: [u16; 128],
    vendor: u32,
    device: u32,
    subsystem: u32,
    revision: u32,
    video: usize,
    dedicated: usize,
    shared: usize,
    luid: u64,
    flags: u32,
}

unsafe fn method(object: *mut c_void, slot: usize) -> *const c_void {
    *(*(object as *mut *const *const c_void)).add(slot)
}

unsafe fn release(object: *mut c_void) {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = std::mem::transmute(method(object, 2));
    f(object);
}

struct Adapter {
    name: String,
    vendor: u32,
    device: u32,
    luid: u64,
    thermal: Option<i32>,
}

fn adapters() -> Vec<Adapter> {
    let mut result = Vec::new();
    unsafe {
        let iid = Guid {
            a: 0x770aae78,
            b: 0xf26f,
            c: 0x4dba,
            d: [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87],
        };
        let Some(dxgi) = Library::system("dxgi.dll") else {
            return result;
        };
        let Some(address) = dxgi.symbol(b"CreateDXGIFactory1\0") else {
            return result;
        };
        let create: unsafe extern "system" fn(*const Guid, *mut *mut c_void) -> i32 =
            std::mem::transmute(address);
        let mut factory = null_mut();
        if create(&iid, &mut factory) < 0 || factory.is_null() {
            return result;
        }
        let enumerate: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32 =
            std::mem::transmute(method(factory, 12));
        for index in 0..32 {
            let mut adapter = null_mut();
            if enumerate(factory, index, &mut adapter) < 0 || adapter.is_null() {
                break;
            }
            let describe: unsafe extern "system" fn(*mut c_void, *mut AdapterDesc) -> i32 =
                std::mem::transmute(method(adapter, 10));
            let mut desc: AdapterDesc = zeroed();
            if describe(adapter, &mut desc) >= 0 && desc.flags & 2 == 0 && desc.vendor != 0x1414 {
                let end = desc.name.iter().position(|&c| c == 0).unwrap_or(128);
                result.push(Adapter {
                    name: String::from_utf16_lossy(&desc.name[..end]),
                    vendor: desc.vendor,
                    device: desc.device,
                    luid: desc.luid,
                    thermal: None,
                });
            }
            release(adapter);
        }
        release(factory);
    }
    result
}

#[repr(C)]
struct AdapterOpen {
    luid: u64,
    handle: u32,
}
#[repr(C)]
struct AdapterQuery {
    handle: u32,
    kind: u32,
    data: *mut c_void,
    size: u32,
}
#[repr(C)]
struct AdapterPerf {
    index: u32,
    clocks: [u64; 5],
    fan: u32,
    power: u32,
    temperature: u32,
    state: u8,
}

#[link(name = "gdi32")]
extern "system" {
    fn D3DKMTOpenAdapterFromLuid(args: *mut AdapterOpen) -> i32;
    fn D3DKMTQueryAdapterInfo(args: *mut AdapterQuery) -> i32;
    fn D3DKMTCloseAdapter(handle: *const u32) -> i32;
}

fn gpu_temperature(luid: u64) -> Option<i32> {
    unsafe {
        let mut open = AdapterOpen { luid, handle: 0 };
        if D3DKMTOpenAdapterFromLuid(&mut open) != 0 {
            return None;
        }
        let mut perf: AdapterPerf = zeroed();
        let mut query = AdapterQuery {
            handle: open.handle,
            kind: 62,
            data: (&mut perf as *mut AdapterPerf).cast(),
            size: size_of::<AdapterPerf>() as u32,
        };
        let status = D3DKMTQueryAdapterInfo(&mut query);
        D3DKMTCloseAdapter(&open.handle);
        (status == 0 && (1..1500).contains(&perf.temperature))
            .then_some(((perf.temperature + 5) / 10) as i32)
    }
}

type NvTemp = unsafe extern "C" fn(Handle, u32, *mut u32) -> i32;
struct Nvml {
    _library: Library,
    gpu: Handle,
    temperature: NvTemp,
    shutdown: unsafe extern "C" fn() -> i32,
}

impl Nvml {
    fn open() -> Option<Self> {
        let library = Library::system("nvml.dll").or_else(|| {
            let dir = std::env::var_os("ProgramFiles")?;
            Library::load(&std::path::PathBuf::from(dir).join(r"NVIDIA Corporation\NVSMI\nvml.dll"))
        })?;
        unsafe {
            let init: unsafe extern "C" fn() -> i32 =
                std::mem::transmute(library.symbol(b"nvmlInit_v2\0")?);
            let shutdown: unsafe extern "C" fn() -> i32 =
                std::mem::transmute(library.symbol(b"nvmlShutdown\0")?);
            let count: unsafe extern "C" fn(*mut u32) -> i32 =
                std::mem::transmute(library.symbol(b"nvmlDeviceGetCount_v2\0")?);
            let handle: unsafe extern "C" fn(u32, *mut Handle) -> i32 =
                std::mem::transmute(library.symbol(b"nvmlDeviceGetHandleByIndex_v2\0")?);
            let temperature: NvTemp =
                std::mem::transmute(library.symbol(b"nvmlDeviceGetTemperature\0")?);
            if init() != 0 {
                return None;
            }
            let (mut gpu, mut devices) = (0, 0);
            if count(&mut devices) != 0 || devices != 1 || handle(0, &mut gpu) != 0 {
                shutdown();
                return None;
            }
            Some(Self {
                _library: library,
                gpu,
                temperature,
                shutdown,
            })
        }
    }

    fn read(&self) -> Option<i32> {
        let mut value = 0;
        (unsafe { (self.temperature)(self.gpu, 0, &mut value) } == 0 && (1..150).contains(&value))
            .then_some(value as i32)
    }
}

impl Drop for Nvml {
    fn drop(&mut self) {
        unsafe {
            (self.shutdown)();
        }
    }
}

fn open_device(path: &str, access: u32) -> Option<OwnedHandle> {
    let file = unsafe { CreateFileW(wide(path).as_ptr(), access, 7, 0, 3, 0, 0) };
    if file == -1 {
        None
    } else {
        Some(OwnedHandle(file))
    }
}

fn ioctl(handle: Handle, code: u32, input: &[u8], output: &mut [u8]) -> Option<usize> {
    let mut bytes = 0;
    let src = if input.is_empty() {
        null()
    } else {
        input.as_ptr().cast()
    };
    let dst = if output.is_empty() {
        null_mut()
    } else {
        output.as_mut_ptr().cast()
    };
    if unsafe {
        DeviceIoControl(
            handle,
            code,
            src,
            input.len() as u32,
            dst,
            output.len() as u32,
            &mut bytes,
            0,
        )
    } == 0
    {
        None
    } else {
        Some((bytes as usize).min(output.len()))
    }
}

struct Pawn {
    name: &'static str,
    module: &'static [u8],
    function: &'static [u8],
    target: u64,
    status: u64,
    handle: Option<OwnedHandle>,
    tjmax: Option<i32>,
}

impl Pawn {
    fn new(
        name: &'static str,
        module: &'static [u8],
        function: &'static [u8],
        target: u64,
        status: u64,
    ) -> Self {
        Self {
            name,
            module,
            function,
            target,
            status,
            handle: None,
            tjmax: None,
        }
    }

    fn register(&self, register: u64) -> Option<u64> {
        let mut request = [0; 40];
        request[..self.function.len()].copy_from_slice(self.function);
        request[32..].copy_from_slice(&register.to_le_bytes());
        let mut reply = [0; 8];
        (ioctl(self.handle.as_ref()?.0, 0xA1B22104, &request, &mut reply)? == 8)
            .then_some(u64::from_le_bytes(reply))
    }

    fn read(&mut self) -> Option<i32> {
        if self.handle.is_none() {
            let handle = open_device(r"\\?\GLOBALROOT\Device\PawnIO", 0xC0000000)?;
            let replacement = std::env::current_exe().ok().and_then(|path| {
                std::fs::read(
                    path.parent()?
                        .join("third-party")
                        .join("PawnIO")
                        .join(self.name),
                )
                .ok()
            });
            ioctl(
                handle.0,
                0xA1B22084,
                replacement.as_deref().unwrap_or(self.module),
                &mut [],
            )?;
            self.handle = Some(handle);
            self.tjmax = self
                .register(self.target)
                .map(|x| ((x >> 16) & 255) as i32)
                .filter(|value| (60..=130).contains(value));
            if self.tjmax.is_none() {
                self.handle = None;
                return None;
            }
        }
        let status = match self.register(self.status) {
            Some(value) => value,
            None => {
                self.handle = None;
                return None;
            }
        };
        let value = self.tjmax? - ((status >> 16) & 127) as i32;
        (status & (1 << 31) != 0 && value > 0).then_some(value)
    }
}

fn core_temp() -> Option<i32> {
    let mapping = unsafe { OpenFileMappingW(4, 0, wide("CoreTempMappingObjectEx").as_ptr()) };
    if mapping == 0 {
        return None;
    }
    let mapping = OwnedHandle(mapping);
    let ptr = unsafe { MapViewOfFile(mapping.0, 4, 0, 0, 2686) };
    if ptr.is_null() {
        return None;
    }
    let data = unsafe { std::slice::from_raw_parts(ptr, 2686) };
    let u32at = |i| u32::from_le_bytes(data[i..i + 4].try_into().unwrap());
    let (cores, cpus) = (u32at(1536) as usize, u32at(1540) as usize);
    let mut hottest = None;
    if cores > 0 && cpus > 0 && cpus <= 128 && cores <= 256 / cpus {
        for i in 0..cores * cpus {
            let mut value = f32::from_bits(u32at(1544 + i * 4)) as f64;
            let relative = data[2685] != 0;
            if data[2684] != 0 {
                value = (value - if relative { 0. } else { 32. }) * 5. / 9.;
            }
            if relative {
                let limit = u32at(1024 + 4 * (i / cores));
                if !(50..=150).contains(&limit) {
                    continue;
                }
                value = limit as f64 - value;
            }
            if value > 0. && value < 150. {
                hottest = Some(hottest.unwrap_or(0).max(value.round() as i32));
            }
        }
    }
    unsafe {
        UnmapViewOfFile(ptr);
    }
    hottest
}

struct Disk {
    number: u32,
    kind: &'static str,
    volumes: String,
    activity: Option<f64>,
    temperature: Option<i32>,
    capacity: (u64, u64),
    missing_ticks: u8,
}

impl Disk {
    fn open(number: u32) -> Option<OwnedHandle> {
        open_device(&format!(r"\\.\PhysicalDrive{number}"), 0)
    }

    fn new(number: u32) -> Self {
        let handle = Self::open(number);
        let mut reply = [0; 12];
        let kind = match handle.as_ref().and_then(|h| {
            ioctl(
                h.0,
                0x2D1400,
                &[7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                &mut reply,
            )
        }) {
            Some(n) if n >= 9 => {
                if reply[8] == 0 {
                    "SSD"
                } else {
                    "HDD"
                }
            }
            _ => "Disk",
        };
        Self {
            number,
            kind,
            volumes: String::new(),
            activity: None,
            temperature: None,
            capacity: (0, 0),
            missing_ticks: 0,
        }
    }

    fn temperature(&self) -> Option<i32> {
        let handle = Self::open(self.number)?;
        let h = handle.0;
        let mut reply = [0; 516];
        for property in [52, 51] {
            let request = [property, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            if let Some(bytes) =
                ioctl(h, 0x2D1400, &request, &mut reply).filter(|&bytes| bytes >= 24)
            {
                let count =
                    (u16::from_le_bytes([reply[12], reply[13]]) as usize).min((bytes - 24) / 16);
                let result = (0..count)
                    .map(|i| i * 16 + 26)
                    .map(|i| i16::from_le_bytes([reply[i], reply[i + 1]]) as i32)
                    .filter(|t| (1..150).contains(t))
                    .max();
                if result.is_some() {
                    return result;
                }
            }
        }
        if ioctl(h, 0x2D1100, &[], &mut reply)? < 516 {
            return None;
        }
        let mut fallback = None;
        for i in (6..366).step_by(12) {
            let value = reply[i + 5] as i32;
            if !(1..=120).contains(&value) {
                continue;
            }
            match reply[i] {
                194 => return Some(value),
                190 => fallback = Some(value),
                _ => {}
            }
        }
        fallback
    }

    fn read_capacity(&self) -> (u64, u64) {
        let (mut used, mut total) = (0u64, 0u64);
        let mut seen = 0u32;
        for volume in self.volumes.split_whitespace() {
            let bytes = volume.as_bytes();
            if bytes.len() != 2 || bytes[1] != b':' || !bytes[0].is_ascii_alphabetic() {
                continue;
            }
            let bit = 1 << (bytes[0].to_ascii_uppercase() - b'A');
            if seen & bit != 0 {
                continue;
            }
            seen |= bit;
            let (mut t, mut f) = (0, 0);
            let path = [bytes[0] as u16, b':' as u16, b'\\' as u16, 0];
            if unsafe { GetDiskFreeSpaceExW(path.as_ptr(), null_mut(), &mut t, &mut f) } != 0
                && t >= f
            {
                used = used.saturating_add(t - f);
                total = total.saturating_add(t);
            }
        }
        (used, total)
    }
}

struct Monitor {
    counters: Counters,
    disks: BTreeMap<u32, Disk>,
    adapters: Vec<Adapter>,
    system_volume: String,
    cpu_name: String,
    previous: Option<(u64, u64)>,
    next_thermal: Instant,
    next_disk: Instant,
    cpu_temp: Option<i32>,
    cpu: Option<Pawn>,
    graphics: Option<Pawn>,
    nvidia: Option<Nvml>,
    tried_nvidia: bool,
    proc_prev: BTreeMap<u32, (u64, String)>,
    proc_sys_total: Option<u64>,
    active_hog: Option<(String, u32)>,
    hog_hold: u8,
}

fn gib(bytes: u64) -> String {
    let tenth = (bytes as f64 / 107_374_182.4).round() as u64;
    if tenth.is_multiple_of(10) {
        (tenth / 10).to_string()
    } else {
        format!("{}.{}", tenth / 10, tenth % 10)
    }
}

impl Monitor {
    fn new() -> Self {
        let mut devices = vec![0u16; 65536];
        let n = (unsafe { QueryDosDeviceW(null(), devices.as_mut_ptr(), devices.len() as u32) }
            as usize)
            .min(devices.len());
        let mut disks = BTreeMap::new();
        for name in devices[..n].split(|&c| c == 0) {
            if let Some(number) = String::from_utf16_lossy(name)
                .strip_prefix("PhysicalDrive")
                .and_then(|x| x.parse::<u32>().ok())
            {
                disks.insert(number, Disk::new(number));
            }
        }
        let mut path = [0u16; 260];
        let len = unsafe { GetWindowsDirectoryW(path.as_mut_ptr(), path.len() as u32) };
        let system_volume = if len >= 2 {
            String::from_utf16_lossy(&path[..2])
        } else {
            "C:".into()
        };
        const CPU: &str = r"HARDWARE\DESCRIPTION\System\CentralProcessor\0";
        let cpu_name = reg_string(HKLM, CPU, "ProcessorNameString")
            .map(|raw| raw.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Processor".into());
        let intel = reg_string(HKLM, CPU, "VendorIdentifier").as_deref() == Some("GenuineIntel");
        let ice = intel
            && reg_string(HKLM, CPU, "Identifier")
                .is_some_and(|s| s.starts_with("Intel64 Family 6 Model 126 Stepping "));
        let cpu = intel.then(|| {
            Pawn::new(
                "IntelMSR.bin",
                include_bytes!("../third-party/PawnIO/IntelMSR.bin"),
                b"ioctl_read_msr",
                0x1A2,
                0x1B1,
            )
        });
        let graphics = ice.then(|| {
            Pawn::new(
                "IntelMCHBAR.bin",
                include_bytes!("../third-party/PawnIO/IntelMCHBAR.bin"),
                b"ioctl_read_dword",
                0x599C,
                0x59C0,
            )
        });
        let mut monitor = Self {
            counters: Counters::new(),
            disks,
            adapters: adapters(),
            system_volume,
            cpu_name,
            previous: None,
            next_thermal: Instant::now(),
            next_disk: Instant::now(),
            cpu_temp: None,
            cpu,
            graphics,
            nvidia: None,
            tried_nvidia: false,
            proc_prev: BTreeMap::new(),
            proc_sys_total: None,
            active_hog: None,
            hog_hold: 0,
        };
        monitor.cpu_usage();
        monitor
    }

    fn reset_baseline(&mut self) {
        self.previous = None;
        self.proc_prev.clear();
        self.proc_sys_total = None;
        self.active_hog = None;
        self.hog_hold = 0;
        self.cpu_usage();
        self.counters.collect();
        let now = Instant::now();
        self.next_thermal = now;
        self.next_disk = now;
    }

    fn cpu_usage(&mut self) -> (Option<f64>, u64) {
        let (mut idle, mut kernel, mut user) = (0, 0, 0);
        if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
            return (None, 0);
        }
        let total = kernel + user;
        let previous = self.previous.replace((idle, total));
        let usage = previous.and_then(|(i, t)| {
            (total > t && idle >= i).then(|| 100. * (1. - (idle - i) as f64 / (total - t) as f64))
        });
        (usage, total)
    }

    fn update_cpu_hog(&mut self, cpu_pct: Option<f64>, sys_total: u64) -> Option<(String, u32)> {
        let load = cpu_pct.unwrap_or(0.0);
        let temp = self.cpu_temp;
        let warmup = load >= 55.0 || temp.is_some_and(|t| t >= 75);
        let hot = load >= 75.0
            || (load >= 50.0 && temp.is_some_and(|t| t >= 80))
            || temp.is_some_and(|t| t >= 85);
        let calm = load < 62.0 && temp.is_none_or(|t| t < 77);

        if !warmup && self.hog_hold == 0 {
            if !self.proc_prev.is_empty() || self.active_hog.is_some() {
                self.proc_prev.clear();
                self.proc_sys_total = None;
                self.active_hog = None;
            }
            return None;
        }

        let snap = snapshot_process_times();
        if let Some(prev_sys) = self.proc_sys_total {
            if sys_total > prev_sys && !self.proc_prev.is_empty() {
                let sys_delta = (sys_total - prev_sys) as f64;
                let mut grouped: BTreeMap<String, u64> = BTreeMap::new();
                for (&pid, &(cur_time, ref name)) in &snap {
                    if let Some(&(prev_time, ref prev_name)) = self.proc_prev.get(&pid) {
                        if prev_name == name && cur_time >= prev_time {
                            *grouped.entry(name.clone()).or_default() += cur_time - prev_time;
                        }
                    }
                }
                let top =
                    grouped
                        .into_iter()
                        .max_by_key(|(_, delta)| *delta)
                        .map(|(name, delta)| {
                            let pct = ((delta as f64 / sys_delta) * 100.0)
                                .round()
                                .clamp(0.0, 99.0) as u32;
                            (name, pct)
                        });
                if let Some((best_name, pct)) = top {
                    if hot && pct >= 10 {
                        self.active_hog = Some((best_name, pct));
                        self.hog_hold = 4;
                    } else if self.hog_hold > 0 {
                        if calm {
                            self.hog_hold -= 1;
                            if self.hog_hold == 0 {
                                self.active_hog = None;
                            } else if pct >= 8 {
                                self.active_hog = Some((best_name, pct));
                            }
                        } else if pct >= 8 {
                            self.active_hog = Some((best_name, pct));
                        }
                    }
                } else if self.hog_hold > 0 && calm {
                    self.hog_hold -= 1;
                    if self.hog_hold == 0 {
                        self.active_hog = None;
                    }
                }
            }
        }
        self.proc_prev = snap;
        self.proc_sys_total = (sys_total > 0).then_some(sys_total);
        self.active_hog.clone()
    }

    fn sample(&mut self) -> Vec<Metric> {
        let (cpu, sys_total) = self.cpu_usage();
        let mut memory: Memory = unsafe { zeroed() };
        memory.size = size_of::<Memory>() as u32;
        let (ram, ram_cap, ram_detail) =
            if unsafe { GlobalMemoryStatusEx((&mut memory as *mut Memory).cast()) } != 0
                && memory.total > 0
            {
                let used = memory.total.saturating_sub(memory.available);
                let frac = (used as f64 / memory.total as f64).clamp(0.0, 1.0);
                (
                    format!("{} GB", gib(used)),
                    Some(frac),
                    format!(
                        "Physical memory: {} / {} GB ({:.0}%)",
                        gib(used),
                        gib(memory.total),
                        frac * 100.0
                    ),
                )
            } else {
                ("--".into(), None, "Physical memory".into())
            };
        let collected = self.counters.collect();
        let mut engines = BTreeMap::<(u64, u32, u32), f64>::new();
        let gpu_ready = collected
            && self.counters.read(self.counters.gpu, |name, value| {
                if let Some(key) = engine_id(name) {
                    *engines.entry(key).or_default() += value;
                }
            });
        for disk in self.disks.values_mut() {
            disk.activity = None;
        }
        let mut disk_ready = false;
        if collected {
            self.counters.read(self.counters.disk, |name, idle| {
                let mut fields = name.splitn(2, char::is_whitespace);
                let Some(number) = fields.next().and_then(|x| x.parse().ok()) else {
                    return;
                };
                disk_ready = true;
                let disk = self.disks.entry(number).or_insert_with(|| {
                    let mut disk = Disk::new(number);
                    disk.temperature = disk.temperature();
                    disk
                });
                disk.activity = Some(100. - idle);
                disk.missing_ticks = 0;
                if let Some(volumes) = fields.next() {
                    if disk.volumes != volumes.trim() {
                        disk.volumes = volumes.trim().into();
                        disk.capacity = disk.read_capacity();
                    }
                }
            });
            if disk_ready {
                self.disks.retain(|_, disk| {
                    if disk.activity.is_some() {
                        true
                    } else {
                        disk.missing_ticks = disk.missing_ticks.saturating_add(1);
                        disk.missing_ticks < 3
                    }
                });
            }
        }
        let now = Instant::now();
        if now >= self.next_thermal {
            self.next_thermal = now + Duration::from_secs(5);
            self.cpu_temp = self.cpu.as_mut().and_then(Pawn::read).or_else(core_temp);
            if self.cpu_temp.is_none() && collected {
                self.counters.read(self.counters.thermal, |name, kelvin| {
                    if name.eq_ignore_ascii_case("CPU")
                        || name.eq_ignore_ascii_case("CPU Package")
                        || (name.len() >= 4
                            && name.as_bytes()[name.len() - 4..].eq_ignore_ascii_case(b".CPU"))
                    {
                        let c = (kelvin - 273.15).round() as i32;
                        if (1..150).contains(&c) {
                            self.cpu_temp = Some(self.cpu_temp.unwrap_or(0).max(c));
                        }
                    }
                });
            }
            let nvidia_count = self.adapters.iter().filter(|a| a.vendor == 0x10de).count();
            for adapter in &mut self.adapters {
                adapter.thermal = gpu_temperature(adapter.luid);
                if adapter.thermal.is_none()
                    && adapter.vendor == 0x8086
                    && adapter.device & 0xff00 == 0x8a00
                {
                    adapter.thermal = self.graphics.as_mut().and_then(Pawn::read);
                }
                if adapter.thermal.is_none() && adapter.vendor == 0x10de && nvidia_count == 1 {
                    if !self.tried_nvidia {
                        self.nvidia = Nvml::open();
                        self.tried_nvidia = true;
                    }
                    adapter.thermal = self.nvidia.as_ref().and_then(Nvml::read);
                }
            }
        }
        if now >= self.next_disk {
            self.next_disk = now + Duration::from_secs(15);
            for disk in self.disks.values_mut() {
                disk.temperature = disk.temperature();
                disk.capacity = disk.read_capacity();
            }
        }

        let cpu_hog = self.update_cpu_hog(cpu, sys_total);
        let mut result = Vec::with_capacity(2 + self.adapters.len() + self.disks.len());
        result.push(Metric {
            kind: Kind::Cpu,
            label: "CPU".into(),
            value: percent(cpu),
            detail: self.cpu_name.clone(),
            temperature: self.cpu_temp,
            capacity: None,
            hog: cpu_hog,
        });
        result.push(Metric {
            kind: Kind::Ram,
            label: "RAM".into(),
            value: ram,
            detail: ram_detail,
            temperature: None,
            capacity: ram_cap,
            hog: None,
        });
        for (index, adapter) in self.adapters.iter().enumerate() {
            let busiest = engines
                .range((adapter.luid, 0, 0)..=(adapter.luid, u32::MAX, u32::MAX))
                .map(|(_, &busy)| busy)
                .fold(0., f64::max);
            result.push(Metric {
                kind: Kind::Gpu,
                label: format!("GPU{index}"),
                value: percent(gpu_ready.then_some(busiest)),
                temperature: adapter.thermal,
                detail: adapter.name.clone(),
                capacity: None,
                hog: None,
            });
        }
        let is_system = |disk: &Disk| {
            disk.volumes
                .split_whitespace()
                .any(|v| v.eq_ignore_ascii_case(&self.system_volume))
        };
        for disk in self
            .disks
            .values()
            .filter(|d| is_system(d))
            .chain(self.disks.values().filter(|d| !is_system(d)))
        {
            let (used, total) = disk.capacity;
            let capacity = (total != 0).then(|| used as f64 / total as f64);
            let mut detail = if disk.volumes.is_empty() {
                format!("Disk {} - active time", disk.number)
            } else {
                format!("Disk {} ({}) - active time", disk.number, disk.volumes)
            };
            if let Some(frac) = capacity {
                let free = total.saturating_sub(used);
                detail.push_str(&format!(
                    "\n{}/{} GB used ({:.0}%) · {} GB free",
                    gib(used),
                    gib(total),
                    frac * 100.,
                    gib(free)
                ));
            }
            result.push(Metric {
                kind: Kind::Disk,
                label: format!("{}{}", disk.kind, disk.number),
                value: percent(disk.activity),
                temperature: disk.temperature,
                detail,
                capacity,
                hog: None,
            });
        }
        result
    }
}

fn clean_exe_name(raw: &str) -> String {
    let base = raw
        .trim()
        .strip_suffix(".exe")
        .or_else(|| raw.trim().strip_suffix(".EXE"))
        .unwrap_or_else(|| raw.trim());
    let pretty = match base.to_ascii_lowercase().as_str() {
        "chrome" => "Chrome",
        "msedge" => "Edge",
        "code" => "VS Code",
        "firefox" => "Firefox",
        "explorer" => "Explorer",
        "dwm" => "DWM",
        "svchost" => "System",
        "antigravity" => "Antigravity",
        "rust-analyzer" => "rust-analyzer",
        "rustc" => "rustc",
        "cargo" => "Cargo",
        _ => "",
    };
    let name = if !pretty.is_empty() {
        pretty.to_string()
    } else if base.chars().all(|c| !c.is_ascii_uppercase()) {
        let mut chars = base.chars();
        match chars.next() {
            Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
            None => "Process".into(),
        }
    } else {
        base.to_string()
    };
    name.chars().take(16).collect()
}

fn snapshot_process_times() -> BTreeMap<u32, (u64, String)> {
    let mut map = BTreeMap::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(2, 0); // TH32CS_SNAPPROCESS
        if snap == 0 || snap == -1 {
            return map;
        }
        let mut entry = ProcessEntry32W {
            size: size_of::<ProcessEntry32W>() as u32,
            usage: 0,
            process_id: 0,
            default_heap_id: 0,
            module_id: 0,
            threads: 0,
            parent_process_id: 0,
            pri_class_base: 0,
            flags: 0,
            exe_file: [0; 260],
        };
        if Process32FirstW(snap, &mut entry) != 0 {
            loop {
                let pid = entry.process_id;
                if pid != 0 && pid != 4 {
                    let h = OpenProcess(0x1000, 0, pid); // PROCESS_QUERY_LIMITED_INFORMATION
                    if h != 0 {
                        let (mut c, mut e, mut k, mut u) = (0u64, 0u64, 0u64, 0u64);
                        if GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) != 0 {
                            let len = entry
                                .exe_file
                                .iter()
                                .position(|&ch| ch == 0)
                                .unwrap_or(entry.exe_file.len());
                            let raw = String::from_utf16_lossy(&entry.exe_file[..len]);
                            map.insert(pid, (k + u, clean_exe_name(&raw)));
                        }
                        CloseHandle(h);
                    }
                }
                if Process32NextW(snap, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    map
}

fn engine_id(name: &str) -> Option<(u64, u32, u32)> {
    let (_, tail) = name.split_once("_luid_0x")?;
    let (high, tail) = tail.split_once("_0x")?;
    let (low, tail) = tail.split_once("_phys_")?;
    let (physical, tail) = tail.split_once("_eng_")?;
    let engine = tail.split('_').next()?;
    let luid = ((u32::from_str_radix(high, 16).ok()? as u64) << 32)
        | u32::from_str_radix(low, 16).ok()? as u64;
    Some((luid, physical.parse().ok()?, engine.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_names_group_processes_by_physical_engine() {
        let first = "pid_3170_luid_0x00000000_0x00010332_phys_0_eng_2_engtype_Copy";
        let second = "pid_5271_luid_0x00000000_0x00010332_phys_0_eng_2_engtype_Copy";
        assert_eq!(engine_id(first), Some((0x10332, 0, 2)));
        assert_eq!(engine_id(first), engine_id(second));
        assert_eq!(engine_id("_Total"), None);
    }

    #[test]
    fn clean_exe_name_and_snapshot_work() {
        assert_eq!(clean_exe_name("chrome.exe"), "Chrome");
        assert_eq!(clean_exe_name("Code.exe"), "VS Code");
        assert_eq!(clean_exe_name("mytool.exe"), "Mytool");
        let snap = snapshot_process_times();
        assert!(!snap.is_empty());
    }

    #[test]
    #[ignore = "Requires the two GPUs, two disks and installed PawnIO on the MoLap laptop"]
    fn sample_molap_laptop() {
        let mut monitor = Monitor::new();
        monitor.sample();
        std::thread::sleep(Duration::from_millis(1100));
        let readings = monitor.sample();
        for metric in &readings {
            println!(
                "{} {} {:?} {:?} {}",
                metric.label, metric.value, metric.temperature, metric.capacity, metric.detail
            );
        }
        assert_eq!(readings.len(), 6);
        assert!(readings.iter().all(|metric| metric.value != "--"));
        assert!(readings[0].temperature.is_some(), "CPU temperature");
        assert!(
            readings[2].detail.contains("Intel") && readings[2].temperature.is_some(),
            "Intel UHD temperature"
        );
        assert!(
            readings[3].detail.contains("NVIDIA") && readings[3].temperature.is_some(),
            "NVIDIA temperature"
        );
        assert!(
            readings[4].detail.contains("(C:)") && readings[5].detail.contains("(D:)"),
            "disk order"
        );
        assert!(
            readings[4..]
                .iter()
                .all(|disk| disk.capacity.is_some() && disk.temperature.is_some()),
            "disk capacity and temperature"
        );
    }
}
