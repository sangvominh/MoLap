use crate::win::{
    wide, ChangeWindowMessageFilterEx, CloseClipboard, DoDragDrop, DragAcceptFiles, DragFinish,
    DragQueryFileW, EmptyClipboard, GetClipboardData, GlobalAlloc, GlobalFree, GlobalLock,
    GlobalUnlock, Guid, Handle, Library, OleInitialize, OleUninitialize, OpenClipboard,
    PostMessageW, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop, SetClipboardData,
    ShellExecuteW,
};
use std::{
    cell::RefCell,
    ffi::c_void,
    mem::size_of,
    ptr::{null, null_mut},
    sync::atomic::{AtomicU32, Ordering},
};

pub const WM_SHELF_DRAG_ENTER: u32 = 0x8013;
pub const WM_SHELF_DRAG_LEAVE: u32 = 0x8014;
pub const WM_SHELF_DROP: u32 = 0x8015;

const CF_HDROP: u32 = 15;
const TYMED_HGLOBAL: u32 = 1;
const DVASPECT_CONTENT: u32 = 1;
const DROPEFFECT_COPY: u32 = 1;
const DROPEFFECT_LINK: u32 = 4;
const S_OK: i32 = 0;
const S_FALSE: i32 = 1;
const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
const E_NOTIMPL: i32 = 0x8000_4001u32 as i32;
const DV_E_FORMATETC: i32 = 0x8004_0064u32 as i32;
const DRAGDROP_S_DROP: i32 = 0x0004_0100;
const DRAGDROP_S_CANCEL: i32 = 0x0004_0101;
const DRAGDROP_S_USEDEFAULTCURSORS: i32 = 0x0004_0102;

const IID_IUNKNOWN: Guid = Guid {
    a: 0x0000_0000,
    b: 0x0000,
    c: 0x0000,
    d: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};
const IID_IDROP_TARGET: Guid = Guid {
    a: 0x0000_0122,
    b: 0x0000,
    c: 0x0000,
    d: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};
const IID_IDATA_OBJECT: Guid = Guid {
    a: 0x0000_010E,
    b: 0x0000,
    c: 0x0000,
    d: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};
const IID_IDROP_SOURCE: Guid = Guid {
    a: 0x0000_0121,
    b: 0x0000,
    c: 0x0000,
    d: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};
const IID_IENUM_FORMATETC: Guid = Guid {
    a: 0x0000_0103,
    b: 0x0000,
    c: 0x0000,
    d: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

#[repr(C)]
#[derive(Clone, Copy)]
struct FormatEtc {
    cf_format: u16,
    ptd: *mut c_void,
    aspect: u32,
    lindex: i32,
    tymed: u32,
}

#[repr(C)]
struct StgMedium {
    tymed: u32,
    data: Handle,
    unk_for_release: *mut c_void,
}

#[repr(C)]
struct DropFiles {
    files_offset: u32,
    pt_x: i32,
    pt_y: i32,
    nc: i32,
    wide: i32,
}

thread_local! {
    static DROPPED_PATHS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

pub fn take_dropped_paths() -> Vec<String> {
    DROPPED_PATHS.with(|cell| std::mem::take(&mut *cell.borrow_mut()))
}

fn hdrop_format() -> FormatEtc {
    FormatEtc {
        cf_format: CF_HDROP as u16,
        ptd: null_mut(),
        aspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL,
    }
}

unsafe fn data_has_hdrop(data_obj: *mut c_void) -> bool {
    if data_obj.is_null() {
        return false;
    }
    let query_get: unsafe extern "system" fn(*mut c_void, *const FormatEtc) -> i32 =
        std::mem::transmute(*(*(data_obj as *mut *const *const c_void)).add(5));
    let fmt = hdrop_format();
    query_get(data_obj, &fmt) == S_OK
}

unsafe fn extract_hdrop(hdrop: Handle) -> Vec<String> {
    let count = DragQueryFileW(hdrop, u32::MAX, null_mut(), 0);
    let mut out = Vec::with_capacity(count as usize);
    for idx in 0..count {
        let len = DragQueryFileW(hdrop, idx, null_mut(), 0);
        if len == 0 || len > 32_768 {
            continue;
        }
        let mut buf = vec![0u16; len as usize + 1];
        let copied = DragQueryFileW(hdrop, idx, buf.as_mut_ptr(), buf.len() as u32);
        if copied > 0 {
            out.push(String::from_utf16_lossy(&buf[..copied as usize]));
        }
    }
    out
}

pub fn build_hdrop_bytes(paths: &[String]) -> Vec<u8> {
    let header = DropFiles {
        files_offset: size_of::<DropFiles>() as u32,
        pt_x: 0,
        pt_y: 0,
        nc: 0,
        wide: 1,
    };
    let mut buf = Vec::with_capacity(size_of::<DropFiles>() + paths.len() * 64);
    let hdr_bytes = unsafe {
        std::slice::from_raw_parts(
            (&header as *const DropFiles).cast::<u8>(),
            size_of::<DropFiles>(),
        )
    };
    buf.extend_from_slice(hdr_bytes);
    for path in paths {
        for u in path.encode_utf16() {
            buf.extend_from_slice(&u.to_le_bytes());
        }
        buf.extend_from_slice(&0u16.to_le_bytes());
    }
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf
}

fn alloc_hglobal(bytes: &[u8]) -> Handle {
    unsafe {
        let hmem = GlobalAlloc(0x0042, bytes.len()); // GMEM_MOVEABLE | GMEM_ZEROINIT
        if hmem == 0 {
            return 0;
        }
        let ptr = GlobalLock(hmem);
        if ptr.is_null() {
            GlobalFree(hmem);
            return 0;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast(), bytes.len());
        GlobalUnlock(hmem);
        hmem
    }
}

#[repr(transparent)]
struct Vtbl<const N: usize>([*const c_void; N]);
unsafe impl<const N: usize> Sync for Vtbl<N> {}

#[repr(C)]
struct ComDropTarget {
    vtbl: *const Vtbl<7>,
    refs: AtomicU32,
    hwnd: Handle,
}

static DROP_TARGET_VTBL: Vtbl<7> = Vtbl([
    dt_qi as *const c_void,
    dt_addref as *const c_void,
    dt_release as *const c_void,
    dt_drag_enter as *const c_void,
    dt_drag_over as *const c_void,
    dt_drag_leave as *const c_void,
    dt_drop as *const c_void,
]);

unsafe extern "system" fn dt_qi(
    this: *mut ComDropTarget,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> i32 {
    if out.is_null() || iid.is_null() {
        return E_NOINTERFACE;
    }
    let iid = &*iid;
    if *iid == IID_IUNKNOWN || *iid == IID_IDROP_TARGET {
        dt_addref(this);
        *out = this.cast();
        S_OK
    } else {
        *out = null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn dt_addref(this: *mut ComDropTarget) -> u32 {
    (*this).refs.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn dt_release(this: *mut ComDropTarget) -> u32 {
    let prev = (*this).refs.fetch_sub(1, Ordering::Release);
    if prev == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
        drop(Box::from_raw(this));
        0
    } else {
        prev - 1
    }
}

unsafe extern "system" fn dt_drag_enter(
    this: *mut ComDropTarget,
    data_obj: *mut c_void,
    _key_state: u32,
    _pt: u64,
    effect: *mut u32,
) -> i32 {
    let has = data_has_hdrop(data_obj);
    if !effect.is_null() {
        *effect = if has { DROPEFFECT_COPY } else { 0 };
    }
    if has {
        PostMessageW((*this).hwnd, WM_SHELF_DRAG_ENTER, 0, 0);
    }
    S_OK
}

unsafe extern "system" fn dt_drag_over(
    _this: *mut ComDropTarget,
    _key_state: u32,
    _pt: u64,
    effect: *mut u32,
) -> i32 {
    if !effect.is_null() {
        *effect &= DROPEFFECT_COPY;
        if *effect == 0 {
            *effect = DROPEFFECT_COPY;
        }
    }
    S_OK
}

unsafe extern "system" fn dt_drag_leave(this: *mut ComDropTarget) -> i32 {
    PostMessageW((*this).hwnd, WM_SHELF_DRAG_LEAVE, 0, 0);
    S_OK
}

unsafe extern "system" fn dt_drop(
    this: *mut ComDropTarget,
    data_obj: *mut c_void,
    _key_state: u32,
    _pt: u64,
    effect: *mut u32,
) -> i32 {
    PostMessageW((*this).hwnd, WM_SHELF_DRAG_LEAVE, 0, 0);
    if data_obj.is_null() {
        if !effect.is_null() {
            *effect = 0;
        }
        return S_OK;
    }
    let get_data: unsafe extern "system" fn(*mut c_void, *const FormatEtc, *mut StgMedium) -> i32 =
        std::mem::transmute(*(*(data_obj as *mut *const *const c_void)).add(3));
    let fmt = hdrop_format();
    let mut medium = StgMedium {
        tymed: 0,
        data: 0,
        unk_for_release: null_mut(),
    };
    if get_data(data_obj, &fmt, &mut medium) == S_OK
        && medium.tymed == TYMED_HGLOBAL
        && medium.data != 0
    {
        let paths = extract_hdrop(medium.data);
        ReleaseStgMedium((&mut medium as *mut StgMedium).cast());
        if !paths.is_empty() {
            DROPPED_PATHS.with(|cell| cell.borrow_mut().extend(paths));
            PostMessageW((*this).hwnd, WM_SHELF_DROP, 0, 0);
            if !effect.is_null() {
                *effect = DROPEFFECT_COPY;
            }
            return S_OK;
        }
    }
    if !effect.is_null() {
        *effect = 0;
    }
    S_OK
}

pub struct DropTargetGuard {
    hwnd: Handle,
    ole_init: bool,
    registered: bool,
}

impl DropTargetGuard {
    pub fn register(hwnd: Handle) -> Self {
        unsafe {
            if hwnd != 0 {
                DragAcceptFiles(hwnd, 1);
                for msg in [0x0233u32, 0x004A, 0x0049] {
                    ChangeWindowMessageFilterEx(hwnd, msg, 1, null_mut());
                }
            }
            let hr = OleInitialize(null_mut());
            let ole_init = hr == S_OK || hr == S_FALSE;
            let mut registered = false;
            if ole_init && hwnd != 0 {
                let target = Box::into_raw(Box::new(ComDropTarget {
                    vtbl: &DROP_TARGET_VTBL,
                    refs: AtomicU32::new(1),
                    hwnd,
                }));
                if RegisterDragDrop(hwnd, target.cast()) == S_OK {
                    registered = true;
                }
                dt_release(target);
            }
            Self {
                hwnd,
                ole_init,
                registered,
            }
        }
    }
}

impl Drop for DropTargetGuard {
    fn drop(&mut self) {
        unsafe {
            if self.registered && self.hwnd != 0 {
                RevokeDragDrop(self.hwnd);
            }
            if self.ole_init {
                OleUninitialize();
            }
        }
    }
}

#[repr(C)]
struct ComEnumFormatEtc {
    vtbl: *const Vtbl<7>,
    refs: AtomicU32,
    index: u32,
}

static ENUM_VTBL: Vtbl<7> = Vtbl([
    ef_qi as *const c_void,
    ef_addref as *const c_void,
    ef_release as *const c_void,
    ef_next as *const c_void,
    ef_skip as *const c_void,
    ef_reset as *const c_void,
    ef_clone as *const c_void,
]);

unsafe extern "system" fn ef_qi(
    this: *mut ComEnumFormatEtc,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> i32 {
    if out.is_null() || iid.is_null() {
        return E_NOINTERFACE;
    }
    let iid = &*iid;
    if *iid == IID_IUNKNOWN || *iid == IID_IENUM_FORMATETC {
        ef_addref(this);
        *out = this.cast();
        S_OK
    } else {
        *out = null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn ef_addref(this: *mut ComEnumFormatEtc) -> u32 {
    (*this).refs.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn ef_release(this: *mut ComEnumFormatEtc) -> u32 {
    let prev = (*this).refs.fetch_sub(1, Ordering::Release);
    if prev == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
        drop(Box::from_raw(this));
        0
    } else {
        prev - 1
    }
}

unsafe extern "system" fn ef_next(
    this: *mut ComEnumFormatEtc,
    celt: u32,
    out: *mut FormatEtc,
    fetched: *mut u32,
) -> i32 {
    if celt > 0 && (*this).index == 0 && !out.is_null() {
        *out = hdrop_format();
        (*this).index = 1;
        if !fetched.is_null() {
            *fetched = 1;
        }
        if celt == 1 {
            S_OK
        } else {
            S_FALSE
        }
    } else {
        if !fetched.is_null() {
            *fetched = 0;
        }
        S_FALSE
    }
}

unsafe extern "system" fn ef_skip(this: *mut ComEnumFormatEtc, celt: u32) -> i32 {
    (*this).index = (*this).index.saturating_add(celt).min(1);
    if (*this).index <= 1 && celt <= 1 {
        S_OK
    } else {
        S_FALSE
    }
}

unsafe extern "system" fn ef_reset(this: *mut ComEnumFormatEtc) -> i32 {
    (*this).index = 0;
    S_OK
}

unsafe extern "system" fn ef_clone(this: *mut ComEnumFormatEtc, out: *mut *mut c_void) -> i32 {
    if out.is_null() {
        return E_NOINTERFACE;
    }
    let clone = Box::into_raw(Box::new(ComEnumFormatEtc {
        vtbl: &ENUM_VTBL,
        refs: AtomicU32::new(1),
        index: (*this).index,
    }));
    *out = clone.cast();
    S_OK
}

#[repr(C)]
struct ComDataObject {
    vtbl: *const Vtbl<12>,
    refs: AtomicU32,
    hdrop_bytes: Vec<u8>,
}

static DATA_OBJ_VTBL: Vtbl<12> = Vtbl([
    do_qi as *const c_void,
    do_addref as *const c_void,
    do_release as *const c_void,
    do_get_data as *const c_void,
    do_not_impl as *const c_void,
    do_query_get_data as *const c_void,
    do_not_impl as *const c_void,
    do_not_impl as *const c_void,
    do_enum_format_etc as *const c_void,
    do_not_impl as *const c_void,
    do_not_impl as *const c_void,
    do_not_impl as *const c_void,
]);

unsafe extern "system" fn do_qi(
    this: *mut ComDataObject,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> i32 {
    if out.is_null() || iid.is_null() {
        return E_NOINTERFACE;
    }
    let iid = &*iid;
    if *iid == IID_IUNKNOWN || *iid == IID_IDATA_OBJECT {
        do_addref(this);
        *out = this.cast();
        S_OK
    } else {
        *out = null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn do_addref(this: *mut ComDataObject) -> u32 {
    (*this).refs.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn do_release(this: *mut ComDataObject) -> u32 {
    let prev = (*this).refs.fetch_sub(1, Ordering::Release);
    if prev == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
        drop(Box::from_raw(this));
        0
    } else {
        prev - 1
    }
}

fn matches_hdrop(fmt: &FormatEtc) -> bool {
    fmt.cf_format as u32 == CF_HDROP
        && (fmt.aspect & DVASPECT_CONTENT) != 0
        && (fmt.tymed & TYMED_HGLOBAL) != 0
}

unsafe extern "system" fn do_get_data(
    this: *mut ComDataObject,
    fmt: *const FormatEtc,
    medium: *mut StgMedium,
) -> i32 {
    if fmt.is_null() || medium.is_null() || !matches_hdrop(&*fmt) {
        return DV_E_FORMATETC;
    }
    let hmem = alloc_hglobal(&(*this).hdrop_bytes);
    if hmem == 0 {
        return DV_E_FORMATETC;
    }
    (*medium).tymed = TYMED_HGLOBAL;
    (*medium).data = hmem;
    (*medium).unk_for_release = null_mut();
    S_OK
}

unsafe extern "system" fn do_query_get_data(
    _this: *mut ComDataObject,
    fmt: *const FormatEtc,
) -> i32 {
    if !fmt.is_null() && matches_hdrop(&*fmt) {
        S_OK
    } else {
        DV_E_FORMATETC
    }
}

unsafe extern "system" fn do_enum_format_etc(
    _this: *mut ComDataObject,
    direction: u32,
    out: *mut *mut c_void,
) -> i32 {
    if direction == 1 && !out.is_null() {
        let e = Box::into_raw(Box::new(ComEnumFormatEtc {
            vtbl: &ENUM_VTBL,
            refs: AtomicU32::new(1),
            index: 0,
        }));
        *out = e.cast();
        S_OK
    } else {
        E_NOTIMPL
    }
}

unsafe extern "system" fn do_not_impl() -> i32 {
    E_NOTIMPL
}

#[repr(C)]
struct ComDropSource {
    vtbl: *const Vtbl<5>,
    refs: AtomicU32,
}

static DROP_SOURCE_VTBL: Vtbl<5> = Vtbl([
    ds_qi as *const c_void,
    ds_addref as *const c_void,
    ds_release as *const c_void,
    ds_query_continue as *const c_void,
    ds_give_feedback as *const c_void,
]);

unsafe extern "system" fn ds_qi(
    this: *mut ComDropSource,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> i32 {
    if out.is_null() || iid.is_null() {
        return E_NOINTERFACE;
    }
    let iid = &*iid;
    if *iid == IID_IUNKNOWN || *iid == IID_IDROP_SOURCE {
        ds_addref(this);
        *out = this.cast();
        S_OK
    } else {
        *out = null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn ds_addref(this: *mut ComDropSource) -> u32 {
    (*this).refs.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn ds_release(this: *mut ComDropSource) -> u32 {
    let prev = (*this).refs.fetch_sub(1, Ordering::Release);
    if prev == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
        drop(Box::from_raw(this));
        0
    } else {
        prev - 1
    }
}

unsafe extern "system" fn ds_query_continue(
    _this: *mut ComDropSource,
    esc_pressed: i32,
    key_state: u32,
) -> i32 {
    if esc_pressed != 0 {
        DRAGDROP_S_CANCEL
    } else if (key_state & 1) == 0 {
        DRAGDROP_S_DROP
    } else {
        S_OK
    }
}

unsafe extern "system" fn ds_give_feedback(_this: *mut ComDropSource, _effect: u32) -> i32 {
    DRAGDROP_S_USEDEFAULTCURSORS
}

pub fn start_drag_out(paths: &[String]) {
    if paths.is_empty() {
        return;
    }
    let hdrop_bytes = build_hdrop_bytes(paths);
    unsafe {
        let data_obj = Box::into_raw(Box::new(ComDataObject {
            vtbl: &DATA_OBJ_VTBL,
            refs: AtomicU32::new(1),
            hdrop_bytes,
        }));
        let drop_src = Box::into_raw(Box::new(ComDropSource {
            vtbl: &DROP_SOURCE_VTBL,
            refs: AtomicU32::new(1),
        }));
        let mut effect = 0u32;
        DoDragDrop(
            data_obj.cast(),
            drop_src.cast(),
            DROPEFFECT_COPY | DROPEFFECT_LINK,
            &mut effect,
        );
        ds_release(drop_src);
        do_release(data_obj);
    }
}

pub fn copy_files_to_clipboard(hwnd: Handle, paths: &[String]) -> bool {
    if paths.is_empty() {
        return false;
    }
    let bytes = build_hdrop_bytes(paths);
    let hmem = alloc_hglobal(&bytes);
    if hmem == 0 {
        return false;
    }
    unsafe {
        if OpenClipboard(hwnd) == 0 {
            GlobalFree(hmem);
            return false;
        }
        EmptyClipboard();
        let ok = SetClipboardData(CF_HDROP, hmem) != 0;
        CloseClipboard();
        if !ok {
            GlobalFree(hmem);
        }
        ok
    }
}

pub fn handle_wm_dropfiles(hdrop: Handle) -> Vec<String> {
    if hdrop == 0 {
        return Vec::new();
    }
    unsafe {
        let paths = extract_hdrop(hdrop);
        DragFinish(hdrop);
        paths
    }
}

pub fn read_files_from_clipboard(hwnd: Handle) -> Vec<String> {
    unsafe {
        if OpenClipboard(hwnd) == 0 {
            return Vec::new();
        }
        let hdrop = GetClipboardData(CF_HDROP);
        let paths = if hdrop != 0 {
            extract_hdrop(hdrop)
        } else {
            Vec::new()
        };
        CloseClipboard();
        paths
    }
}

#[repr(C)]
struct OpenFileNameW {
    l_struct_size: u32,
    hwnd_owner: Handle,
    h_instance: Handle,
    lpstr_filter: *const u16,
    lpstr_custom_filter: *mut u16,
    n_max_cust_filter: u32,
    n_filter_index: u32,
    lpstr_file: *mut u16,
    n_max_file: u32,
    lpstr_file_title: *mut u16,
    n_max_file_title: u32,
    lpstr_initial_dir: *const u16,
    lpstr_title: *const u16,
    flags: u32,
    n_file_offset: u16,
    n_file_extension: u16,
    lpstr_def_ext: *const u16,
    l_cust_data: isize,
    lpfn_hook: *const c_void,
    lp_template_name: *const u16,
    pv_reserved: *mut c_void,
    dw_reserved: u32,
    flags_ex: u32,
}

pub fn parse_ofn_buffer(buf: &[u16]) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    while start < buf.len() {
        let len = buf[start..].iter().position(|&c| c == 0).unwrap_or(0);
        if len == 0 {
            break;
        }
        parts.push(String::from_utf16_lossy(&buf[start..start + len]));
        start += len + 1;
    }
    match parts.len() {
        0 => Vec::new(),
        1 => parts,
        _ => {
            let dir = parts[0].trim_end_matches('\\');
            parts[1..]
                .iter()
                .map(|name| format!("{dir}\\{name}"))
                .collect()
        }
    }
}

pub fn pick_files_dialog(owner: Handle) -> Vec<String> {
    let Some(lib) = Library::system("comdlg32.dll") else {
        return Vec::new();
    };
    let Some(ptr) = lib.symbol(b"GetOpenFileNameW\0") else {
        return Vec::new();
    };
    unsafe {
        let get_open: unsafe extern "system" fn(*mut OpenFileNameW) -> i32 =
            std::mem::transmute(ptr);
        let mut buf = vec![0u16; 32_768];
        let mut filter: Vec<u16> = "All Files (*.*)\0*.*\0\0".encode_utf16().collect();
        let title = wide("Add files to MoLap Shelf");
        let mut ofn = OpenFileNameW {
            l_struct_size: size_of::<OpenFileNameW>() as u32,
            hwnd_owner: owner,
            h_instance: 0,
            lpstr_filter: filter.as_mut_ptr(),
            lpstr_custom_filter: null_mut(),
            n_max_cust_filter: 0,
            n_filter_index: 1,
            lpstr_file: buf.as_mut_ptr(),
            n_max_file: buf.len() as u32,
            lpstr_file_title: null_mut(),
            n_max_file_title: 0,
            lpstr_initial_dir: null(),
            lpstr_title: title.as_ptr(),
            flags: 0x0008_1A08, // ALLOWMULTISELECT | EXPLORER | FILEMUSTEXIST | PATHMUSTEXIST | NOCHANGEDIR
            n_file_offset: 0,
            n_file_extension: 0,
            lpstr_def_ext: null(),
            l_cust_data: 0,
            lpfn_hook: null(),
            lp_template_name: null(),
            pv_reserved: null_mut(),
            dw_reserved: 0,
            flags_ex: 0,
        };
        if get_open(&mut ofn) != 0 {
            parse_ofn_buffer(&buf)
        } else {
            Vec::new()
        }
    }
}

pub fn open_file(path: &str) {
    unsafe {
        ShellExecuteW(
            0,
            wide("open").as_ptr(),
            wide(path).as_ptr(),
            null(),
            null(),
            1,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ofn_buffer_parses_single_and_multi_selection() {
        let single: Vec<u16> = "C:\\docs\\a.txt\0\0".encode_utf16().collect();
        assert_eq!(parse_ofn_buffer(&single), vec![r"C:\docs\a.txt"]);

        let multi: Vec<u16> = "C:\\docs\0a.txt\0b.png\0\0".encode_utf16().collect();
        assert_eq!(
            parse_ofn_buffer(&multi),
            vec![r"C:\docs\a.txt", r"C:\docs\b.png"]
        );
    }

    #[test]
    fn hdrop_bytes_encodes_dropfiles_header_and_double_nul_paths() {
        let paths = vec![r"C:\a.txt".to_string(), r"D:\b.png".to_string()];
        let bytes = build_hdrop_bytes(&paths);
        assert_eq!(u32::from_le_bytes(bytes[0..4].try_into().unwrap()), 20);
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 1);
        let units: Vec<u16> = bytes[20..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&[lo, hi]| u16::from_le_bytes([lo, hi]))
            .collect();
        let mut expected: Vec<u16> = r"C:\a.txt".encode_utf16().collect();
        expected.push(0);
        expected.extend(r"D:\b.png".encode_utf16());
        expected.push(0);
        expected.push(0);
        assert_eq!(units, expected);
    }
}
