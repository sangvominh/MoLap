use crate::{
    liquid_glass::{self, BackdropPyramid, GlassSpec},
    win::{
        wide, BitBlt, BitmapInfoHeader, BlendFunction, CreateCompatibleDC, CreateDIBSection,
        CreateStreamOnHGlobal, DeleteDC, DeleteObject, GetDC, GlobalAlloc, GlobalFree, GlobalLock,
        GlobalUnlock, Handle, Library, Point, ReleaseDC, SelectObject, Size, UpdateLayeredWindow,
    },
};
use std::{
    cell::RefCell,
    ffi::c_void,
    mem::{size_of, transmute},
    ptr::{null, null_mut},
};

#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RectF {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PointF {
    x: f32,
    y: f32,
}

#[derive(Clone, Copy)]
pub struct TextStyle {
    pub size: f32,
    pub bold: bool,
    pub color: u32,
    pub align: i32,
}

impl TextStyle {
    pub const fn new(size: f32, bold: bool, color: u32, align: i32) -> Self {
        Self {
            size,
            bold,
            color,
            align,
        }
    }
}

pub const fn argb(a: u8, r: u8, g: u8, b: u8) -> u32 {
    ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

pub fn with_alpha(color: u32, alpha_scale: f32) -> u32 {
    let a = ((color >> 24) & 0xff) as f32;
    let scaled = (a * alpha_scale.clamp(0.0, 1.0)).round() as u32;
    (color & 0x00ff_ffff) | ((scaled.min(255)) << 24)
}

pub fn mix_color(a: u32, b: u32, t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    let ch = |shift: u32| -> u32 {
        let ca = ((a >> shift) & 0xff) as f32;
        let cb = ((b >> shift) & 0xff) as f32;
        ((ca * inv + cb * t).round() as u32).min(255) << shift
    };
    ch(24) | ch(16) | ch(8) | ch(0)
}

#[repr(C)]
struct StartupInput {
    version: u32,
    debug_callback: *const c_void,
    suppress_background_thread: i32,
    suppress_external_codecs: i32,
}

type Gp = *mut c_void;

pub struct GdiplusApi {
    _library: Library,
    token: usize,
    shutdown: unsafe extern "system" fn(usize),
    flush: unsafe extern "system" fn(Gp, i32) -> i32,
    create_bitmap_from_scan0:
        unsafe extern "system" fn(i32, i32, i32, u32, *mut u8, *mut Gp) -> i32,
    create_bitmap_from_stream: unsafe extern "system" fn(*mut c_void, *mut Gp) -> i32,
    dispose_image: unsafe extern "system" fn(Gp) -> i32,
    draw_image_rect: unsafe extern "system" fn(Gp, Gp, f32, f32, f32, f32) -> i32,
    get_image_graphics: unsafe extern "system" fn(Gp, *mut Gp) -> i32,
    delete_graphics: unsafe extern "system" fn(Gp) -> i32,
    set_smoothing_mode: unsafe extern "system" fn(Gp, i32) -> i32,
    set_pixel_offset_mode: unsafe extern "system" fn(Gp, i32) -> i32,
    set_interpolation_mode: unsafe extern "system" fn(Gp, i32) -> i32,
    set_text_rendering_hint: unsafe extern "system" fn(Gp, i32) -> i32,
    graphics_clear: unsafe extern "system" fn(Gp, u32) -> i32,
    set_clip_path: unsafe extern "system" fn(Gp, Gp, i32) -> i32,
    reset_clip: unsafe extern "system" fn(Gp) -> i32,
    create_solid_fill: unsafe extern "system" fn(u32, *mut Gp) -> i32,
    set_solid_fill_color: unsafe extern "system" fn(Gp, u32) -> i32,
    create_line_brush:
        unsafe extern "system" fn(*const PointF, *const PointF, u32, u32, i32, *mut Gp) -> i32,
    create_path_gradient_from_path: unsafe extern "system" fn(Gp, *mut Gp) -> i32,
    set_path_gradient_center_point: unsafe extern "system" fn(Gp, *const PointF) -> i32,
    set_path_gradient_center_color: unsafe extern "system" fn(Gp, u32) -> i32,
    set_path_gradient_surround_colors: unsafe extern "system" fn(Gp, *const u32, *mut i32) -> i32,
    set_path_gradient_sigma_blend: unsafe extern "system" fn(Gp, f32, f32) -> i32,
    delete_brush: unsafe extern "system" fn(Gp) -> i32,
    create_pen1: unsafe extern "system" fn(u32, f32, i32, *mut Gp) -> i32,
    set_pen_color: unsafe extern "system" fn(Gp, u32) -> i32,
    set_pen_width: unsafe extern "system" fn(Gp, f32) -> i32,
    set_pen_start_cap: unsafe extern "system" fn(Gp, i32) -> i32,
    set_pen_end_cap: unsafe extern "system" fn(Gp, i32) -> i32,
    set_pen_dash_style: unsafe extern "system" fn(Gp, i32) -> i32,
    delete_pen: unsafe extern "system" fn(Gp) -> i32,
    create_path: unsafe extern "system" fn(i32, *mut Gp) -> i32,
    add_path_arc: unsafe extern "system" fn(Gp, f32, f32, f32, f32, f32, f32) -> i32,
    add_path_ellipse: unsafe extern "system" fn(Gp, f32, f32, f32, f32) -> i32,
    add_path_line: unsafe extern "system" fn(Gp, f32, f32, f32, f32) -> i32,
    close_path_figure: unsafe extern "system" fn(Gp) -> i32,
    fill_path: unsafe extern "system" fn(Gp, Gp, Gp) -> i32,
    draw_path: unsafe extern "system" fn(Gp, Gp, Gp) -> i32,
    delete_path: unsafe extern "system" fn(Gp) -> i32,
    draw_arc: unsafe extern "system" fn(Gp, Gp, f32, f32, f32, f32, f32, f32) -> i32,
    fill_ellipse: unsafe extern "system" fn(Gp, Gp, f32, f32, f32, f32) -> i32,
    draw_ellipse: unsafe extern "system" fn(Gp, Gp, f32, f32, f32, f32) -> i32,
    draw_line: unsafe extern "system" fn(Gp, Gp, f32, f32, f32, f32) -> i32,
    fill_polygon: unsafe extern "system" fn(Gp, Gp, *const PointF, i32, i32) -> i32,
    create_font_family: unsafe extern "system" fn(*const u16, Gp, *mut Gp) -> i32,
    delete_font_family: unsafe extern "system" fn(Gp) -> i32,
    create_font: unsafe extern "system" fn(Gp, f32, i32, i32, *mut Gp) -> i32,
    delete_font: unsafe extern "system" fn(Gp) -> i32,
    create_string_format: unsafe extern "system" fn(i32, u16, *mut Gp) -> i32,
    set_string_format_align: unsafe extern "system" fn(Gp, i32) -> i32,
    set_string_format_line_align: unsafe extern "system" fn(Gp, i32) -> i32,
    set_string_format_trimming: unsafe extern "system" fn(Gp, i32) -> i32,
    set_string_format_flags: unsafe extern "system" fn(Gp, i32) -> i32,
    delete_string_format: unsafe extern "system" fn(Gp) -> i32,
    draw_string: unsafe extern "system" fn(Gp, *const u16, i32, Gp, *const RectF, Gp, Gp) -> i32,
    measure_string: unsafe extern "system" fn(
        Gp,
        *const u16,
        i32,
        Gp,
        *const RectF,
        Gp,
        *mut RectF,
        *mut i32,
        *mut i32,
    ) -> i32,
}

impl GdiplusApi {
    pub fn load() -> Option<Self> {
        let library = Library::system("gdiplus.dll")?;
        unsafe {
            let startup = transmute::<
                *const c_void,
                unsafe extern "system" fn(*mut usize, *const StartupInput, *mut c_void) -> i32,
            >(library.symbol(b"GdiplusStartup\0")?);
            let shutdown = transmute::<*const c_void, unsafe extern "system" fn(usize)>(
                library.symbol(b"GdiplusShutdown\0")?,
            );
            let flush = transmute::<*const c_void, unsafe extern "system" fn(Gp, i32) -> i32>(
                library.symbol(b"GdipFlush\0")?,
            );
            let input = StartupInput {
                version: 1,
                debug_callback: null(),
                suppress_background_thread: 0,
                suppress_external_codecs: 0,
            };
            let mut token = 0;
            if startup(&mut token, &input, null_mut()) != 0 {
                return None;
            }
            let api = Self {
                token,
                shutdown,
                flush,
                create_bitmap_from_scan0: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(i32, i32, i32, u32, *mut u8, *mut Gp) -> i32,
                >(
                    library.symbol(b"GdipCreateBitmapFromScan0\0")?
                ),
                create_bitmap_from_stream: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(*mut c_void, *mut Gp) -> i32,
                >(
                    library.symbol(b"GdipCreateBitmapFromStream\0")?
                ),
                dispose_image: transmute::<*const c_void, unsafe extern "system" fn(Gp) -> i32>(
                    library.symbol(b"GdipDisposeImage\0")?,
                ),
                draw_image_rect: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, Gp, f32, f32, f32, f32) -> i32,
                >(library.symbol(b"GdipDrawImageRect\0")?),
                get_image_graphics: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, *mut Gp) -> i32,
                >(
                    library.symbol(b"GdipGetImageGraphicsContext\0")?
                ),
                delete_graphics: transmute::<*const c_void, unsafe extern "system" fn(Gp) -> i32>(
                    library.symbol(b"GdipDeleteGraphics\0")?,
                ),
                set_smoothing_mode: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, i32) -> i32,
                >(library.symbol(b"GdipSetSmoothingMode\0")?),
                set_pixel_offset_mode: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, i32) -> i32,
                >(
                    library.symbol(b"GdipSetPixelOffsetMode\0")?
                ),
                set_interpolation_mode: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, i32) -> i32,
                >(
                    library.symbol(b"GdipSetInterpolationMode\0")?
                ),
                set_text_rendering_hint: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, i32) -> i32,
                >(
                    library.symbol(b"GdipSetTextRenderingHint\0")?
                ),
                graphics_clear: transmute::<*const c_void, unsafe extern "system" fn(Gp, u32) -> i32>(
                    library.symbol(b"GdipGraphicsClear\0")?,
                ),
                set_clip_path: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, Gp, i32) -> i32,
                >(library.symbol(b"GdipSetClipPath\0")?),
                reset_clip: transmute::<*const c_void, unsafe extern "system" fn(Gp) -> i32>(
                    library.symbol(b"GdipResetClip\0")?,
                ),
                create_solid_fill: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(u32, *mut Gp) -> i32,
                >(library.symbol(b"GdipCreateSolidFill\0")?),
                set_solid_fill_color: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, u32) -> i32,
                >(library.symbol(b"GdipSetSolidFillColor\0")?),
                create_line_brush: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(
                        *const PointF,
                        *const PointF,
                        u32,
                        u32,
                        i32,
                        *mut Gp,
                    ) -> i32,
                >(library.symbol(b"GdipCreateLineBrush\0")?),
                create_path_gradient_from_path: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, *mut Gp) -> i32,
                >(
                    library.symbol(b"GdipCreatePathGradientFromPath\0")?,
                ),
                set_path_gradient_center_point: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, *const PointF) -> i32,
                >(
                    library.symbol(b"GdipSetPathGradientCenterPoint\0")?,
                ),
                set_path_gradient_center_color: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, u32) -> i32,
                >(
                    library.symbol(b"GdipSetPathGradientCenterColor\0")?,
                ),
                set_path_gradient_surround_colors: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, *const u32, *mut i32) -> i32,
                >(
                    library.symbol(b"GdipSetPathGradientSurroundColorsWithCount\0")?,
                ),
                set_path_gradient_sigma_blend: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, f32, f32) -> i32,
                >(
                    library.symbol(b"GdipSetPathGradientSigmaBlend\0")?,
                ),
                delete_brush: transmute::<*const c_void, unsafe extern "system" fn(Gp) -> i32>(
                    library.symbol(b"GdipDeleteBrush\0")?,
                ),
                create_pen1: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(u32, f32, i32, *mut Gp) -> i32,
                >(library.symbol(b"GdipCreatePen1\0")?),
                set_pen_color: transmute::<*const c_void, unsafe extern "system" fn(Gp, u32) -> i32>(
                    library.symbol(b"GdipSetPenColor\0")?,
                ),
                set_pen_width: transmute::<*const c_void, unsafe extern "system" fn(Gp, f32) -> i32>(
                    library.symbol(b"GdipSetPenWidth\0")?,
                ),
                set_pen_start_cap: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, i32) -> i32,
                >(library.symbol(b"GdipSetPenStartCap\0")?),
                set_pen_end_cap: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, i32) -> i32,
                >(library.symbol(b"GdipSetPenEndCap\0")?),
                set_pen_dash_style: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, i32) -> i32,
                >(library.symbol(b"GdipSetPenDashStyle\0")?),
                delete_pen: transmute::<*const c_void, unsafe extern "system" fn(Gp) -> i32>(
                    library.symbol(b"GdipDeletePen\0")?,
                ),
                create_path: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(i32, *mut Gp) -> i32,
                >(library.symbol(b"GdipCreatePath\0")?),
                add_path_arc: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, f32, f32, f32, f32, f32, f32) -> i32,
                >(library.symbol(b"GdipAddPathArc\0")?),
                add_path_ellipse: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, f32, f32, f32, f32) -> i32,
                >(library.symbol(b"GdipAddPathEllipse\0")?),
                add_path_line: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, f32, f32, f32, f32) -> i32,
                >(library.symbol(b"GdipAddPathLine\0")?),
                close_path_figure: transmute::<*const c_void, unsafe extern "system" fn(Gp) -> i32>(
                    library.symbol(b"GdipClosePathFigure\0")?,
                ),
                fill_path: transmute::<*const c_void, unsafe extern "system" fn(Gp, Gp, Gp) -> i32>(
                    library.symbol(b"GdipFillPath\0")?,
                ),
                draw_path: transmute::<*const c_void, unsafe extern "system" fn(Gp, Gp, Gp) -> i32>(
                    library.symbol(b"GdipDrawPath\0")?,
                ),
                delete_path: transmute::<*const c_void, unsafe extern "system" fn(Gp) -> i32>(
                    library.symbol(b"GdipDeletePath\0")?,
                ),
                draw_arc: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, Gp, f32, f32, f32, f32, f32, f32) -> i32,
                >(library.symbol(b"GdipDrawArc\0")?),
                fill_ellipse: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, Gp, f32, f32, f32, f32) -> i32,
                >(library.symbol(b"GdipFillEllipse\0")?),
                draw_ellipse: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, Gp, f32, f32, f32, f32) -> i32,
                >(library.symbol(b"GdipDrawEllipse\0")?),
                draw_line: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, Gp, f32, f32, f32, f32) -> i32,
                >(library.symbol(b"GdipDrawLine\0")?),
                fill_polygon: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, Gp, *const PointF, i32, i32) -> i32,
                >(library.symbol(b"GdipFillPolygon\0")?),
                create_font_family: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(*const u16, Gp, *mut Gp) -> i32,
                >(
                    library.symbol(b"GdipCreateFontFamilyFromName\0")?
                ),
                delete_font_family: transmute::<*const c_void, unsafe extern "system" fn(Gp) -> i32>(
                    library.symbol(b"GdipDeleteFontFamily\0")?,
                ),
                create_font: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, f32, i32, i32, *mut Gp) -> i32,
                >(library.symbol(b"GdipCreateFont\0")?),
                delete_font: transmute::<*const c_void, unsafe extern "system" fn(Gp) -> i32>(
                    library.symbol(b"GdipDeleteFont\0")?,
                ),
                create_string_format: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(i32, u16, *mut Gp) -> i32,
                >(
                    library.symbol(b"GdipCreateStringFormat\0")?
                ),
                set_string_format_align: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, i32) -> i32,
                >(
                    library.symbol(b"GdipSetStringFormatAlign\0")?
                ),
                set_string_format_line_align: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, i32) -> i32,
                >(
                    library.symbol(b"GdipSetStringFormatLineAlign\0")?
                ),
                set_string_format_trimming: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, i32) -> i32,
                >(
                    library.symbol(b"GdipSetStringFormatTrimming\0")?
                ),
                set_string_format_flags: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, i32) -> i32,
                >(
                    library.symbol(b"GdipSetStringFormatFlags\0")?
                ),
                delete_string_format: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp) -> i32,
                >(
                    library.symbol(b"GdipDeleteStringFormat\0")?
                ),
                draw_string: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(Gp, *const u16, i32, Gp, *const RectF, Gp, Gp) -> i32,
                >(library.symbol(b"GdipDrawString\0")?),
                measure_string: transmute::<
                    *const c_void,
                    unsafe extern "system" fn(
                        Gp,
                        *const u16,
                        i32,
                        Gp,
                        *const RectF,
                        Gp,
                        *mut RectF,
                        *mut i32,
                        *mut i32,
                    ) -> i32,
                >(library.symbol(b"GdipMeasureString\0")?),
                _library: library,
            };
            Some(api)
        }
    }
}

impl Drop for GdiplusApi {
    fn drop(&mut self) {
        if self.token != 0 {
            unsafe {
                (self.shutdown)(self.token);
            }
        }
    }
}

thread_local! {
    static SHARED_API: RefCell<Option<Box<GdiplusApi>>> = const { RefCell::new(None) };
}

pub fn shared() -> Option<&'static GdiplusApi> {
    SHARED_API.with(|cell| {
        let mut opt = cell.borrow_mut();
        if opt.is_none() {
            *opt = GdiplusApi::load().map(Box::new);
        }
        opt.as_deref()
            .map(|api| unsafe { &*(api as *const GdiplusApi) })
    })
}

pub fn shutdown_shared() {
    SHARED_API.with(|cell| {
        cell.borrow_mut().take();
    });
}

pub struct GpImage<'a> {
    api: &'a GdiplusApi,
    raw: Gp,
}

impl<'a> GpImage<'a> {
    pub fn from_bytes(api: &'a GdiplusApi, bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() {
            return None;
        }
        unsafe {
            let hglobal = GlobalAlloc(0x0002, bytes.len());
            if hglobal == 0 {
                return None;
            }
            let ptr = GlobalLock(hglobal);
            if ptr.is_null() {
                GlobalFree(hglobal);
                return None;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.cast(), bytes.len());
            GlobalUnlock(hglobal);

            let mut stream = null_mut();
            if CreateStreamOnHGlobal(hglobal, 1, &mut stream) != 0 || stream.is_null() {
                GlobalFree(hglobal);
                return None;
            }
            let mut bitmap = null_mut();
            let status = (api.create_bitmap_from_stream)(stream, &mut bitmap);
            let release: unsafe extern "system" fn(*mut c_void) -> u32 =
                transmute(*(*(stream as *mut *const *const c_void)).add(2));
            release(stream);
            if status != 0 || bitmap.is_null() {
                return None;
            }
            Some(Self { api, raw: bitmap })
        }
    }

    pub fn dominant_rgb(&self, api: &GdiplusApi) -> Option<(u8, u8, u8)> {
        let surf = Surface::new(api, 8, 8, 96)?;
        surf.clear();
        unsafe {
            (api.draw_image_rect)(surf.graphics, self.raw, 0.0, 0.0, 8.0, 8.0);
            let len = (surf.width * surf.height * 4) as usize;
            let bytes = std::slice::from_raw_parts(surf.bits, len);
            extract_dominant_bgra(bytes)
        }
    }
}

fn extract_dominant_bgra(bytes: &[u8]) -> Option<(u8, u8, u8)> {
    let mut sum_r = 0.0f32;
    let mut sum_g = 0.0f32;
    let mut sum_b = 0.0f32;
    let mut sum_w = 0.0f32;
    for px in bytes.as_chunks::<4>().0 {
        let a = px[3] as f32;
        if a < 32.0 {
            continue;
        }
        let inv = 255.0 / a;
        let b = (px[0] as f32 * inv).min(255.0);
        let g = (px[1] as f32 * inv).min(255.0);
        let r = (px[2] as f32 * inv).min(255.0);
        let max_c = r.max(g).max(b);
        let min_c = r.min(g).min(b);
        if max_c < 28.0 || min_c > 235.0 {
            continue;
        }
        let w = (max_c - min_c) + 16.0;
        sum_r += r * w;
        sum_g += g * w;
        sum_b += b * w;
        sum_w += w;
    }
    if sum_w <= 0.0 {
        return None;
    }
    let avg_r = sum_r / sum_w;
    let avg_g = sum_g / sum_w;
    let avg_b = sum_b / sum_w;
    let lum = 0.299 * avg_r + 0.587 * avg_g + 0.114 * avg_b;
    let boost = |c: f32| -> u8 { (lum + (c - lum) * 1.18).clamp(0.0, 255.0).round() as u8 };
    Some((boost(avg_r), boost(avg_g), boost(avg_b)))
}

impl Drop for GpImage<'_> {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                (self.api.dispose_image)(self.raw);
            }
        }
    }
}

pub struct Surface<'a> {
    api: &'a GdiplusApi,
    pub width: i32,
    pub height: i32,
    pub scale: f32,
    mem_dc: Handle,
    dib: Handle,
    old_bmp: Handle,
    bits: *mut u8,
    bitmap: Gp,
    graphics: Gp,
    brush: Gp,
    pen: Gp,
    font_display: Gp,
    font_text: Gp,
    font_semibold: Gp,
    font_family: Gp,
    formats: [Gp; 3],
    fonts: RefCell<Vec<(u16, u8, Gp)>>,
    pyramid: RefCell<BackdropPyramid>,
}

impl<'a> Surface<'a> {
    pub fn new(api: &'a GdiplusApi, width: i32, height: i32, dpi: u32) -> Option<Self> {
        let width = width.max(1);
        let height = height.max(1);
        unsafe {
            let screen_dc = GetDC(0);
            let mem_dc = CreateCompatibleDC(screen_dc);
            if screen_dc != 0 {
                ReleaseDC(0, screen_dc);
            }
            if mem_dc == 0 {
                return None;
            }
            let bmi = BitmapInfoHeader {
                size: size_of::<BitmapInfoHeader>() as u32,
                width,
                height: -height,
                planes: 1,
                bit_count: 32,
                compression: 0,
                ..Default::default()
            };
            let mut bits = null_mut();
            let dib = CreateDIBSection(mem_dc, &bmi, 0, &mut bits, 0, 0);
            if dib == 0 || bits.is_null() {
                if dib != 0 {
                    DeleteObject(dib);
                }
                DeleteDC(mem_dc);
                return None;
            }
            let old_bmp = SelectObject(mem_dc, dib);
            let mut bitmap = null_mut();
            if (api.create_bitmap_from_scan0)(
                width,
                height,
                width * 4,
                0x000E_200B, // PixelFormat32bppPARGB
                bits.cast(),
                &mut bitmap,
            ) != 0
                || bitmap.is_null()
            {
                SelectObject(mem_dc, old_bmp);
                DeleteObject(dib);
                DeleteDC(mem_dc);
                return None;
            }
            let mut graphics = null_mut();
            if (api.get_image_graphics)(bitmap, &mut graphics) != 0 || graphics.is_null() {
                (api.dispose_image)(bitmap);
                SelectObject(mem_dc, old_bmp);
                DeleteObject(dib);
                DeleteDC(mem_dc);
                return None;
            }
            (api.set_smoothing_mode)(graphics, 4); // SmoothingModeAntiAlias
            (api.set_pixel_offset_mode)(graphics, 2); // PixelOffsetModeHighQuality
            (api.set_interpolation_mode)(graphics, 7); // InterpolationModeHighQualityBicubic
            (api.set_text_rendering_hint)(graphics, 3); // TextRenderingHintAntiAliasGridFit

            let mut brush = null_mut();
            (api.create_solid_fill)(0xff00_0000, &mut brush);

            let mut pen = null_mut();
            (api.create_pen1)(0xff00_0000, 1.0, 2, &mut pen);

            let load_family = |name: &str| -> Gp {
                let mut fam = null_mut();
                if (api.create_font_family)(wide(name).as_ptr(), null_mut(), &mut fam) == 0
                    && !fam.is_null()
                {
                    fam
                } else {
                    null_mut()
                }
            };
            let font_family = load_family("Segoe UI");
            let font_display = load_family("Segoe UI Variable Display");
            let font_text = load_family("Segoe UI Variable Text");
            let font_semibold = load_family("Segoe UI Semibold");

            let mut formats = [null_mut(); 3];
            for (align, slot) in formats.iter_mut().enumerate() {
                let mut fmt = null_mut();
                if (api.create_string_format)(0x1000, 0, &mut fmt) == 0 && !fmt.is_null() {
                    (api.set_string_format_align)(fmt, align as i32);
                    (api.set_string_format_line_align)(fmt, 1); // StringAlignmentCenter
                    (api.set_string_format_trimming)(fmt, 3); // StringTrimmingEllipsisCharacter
                    (api.set_string_format_flags)(fmt, 0x1000); // StringFormatFlagsNoWrap
                    *slot = fmt;
                }
            }

            Some(Self {
                api,
                width,
                height,
                scale: (dpi.max(96) as f32) / 96.0,
                mem_dc,
                dib,
                old_bmp,
                bits: bits.cast(),
                bitmap,
                graphics,
                brush,
                pen,
                font_display,
                font_text,
                font_semibold,
                font_family,
                formats,
                fonts: RefCell::new(Vec::with_capacity(24)),
                pyramid: RefCell::new(BackdropPyramid::default()),
            })
        }
    }

    fn sx(&self, rect: RectF) -> RectF {
        let s = self.scale;
        RectF {
            x: rect.x * s,
            y: rect.y * s,
            w: rect.w * s,
            h: rect.h * s,
        }
    }

    unsafe fn rounded_path(&self, rect: RectF, radius: f32) -> Gp {
        let mut path = null_mut();
        if (self.api.create_path)(0, &mut path) != 0 || path.is_null() {
            return null_mut();
        }
        let r = radius.min(rect.w * 0.5).min(rect.h * 0.5).max(0.0);
        if r < 0.5 {
            (self.api.add_path_line)(path, rect.x, rect.y, rect.x + rect.w, rect.y);
            (self.api.add_path_line)(
                path,
                rect.x + rect.w,
                rect.y,
                rect.x + rect.w,
                rect.y + rect.h,
            );
            (self.api.add_path_line)(
                path,
                rect.x + rect.w,
                rect.y + rect.h,
                rect.x,
                rect.y + rect.h,
            );
            (self.api.close_path_figure)(path);
            return path;
        }
        let d = r * 2.0;
        (self.api.add_path_arc)(path, rect.x, rect.y, d, d, 180.0, 90.0);
        (self.api.add_path_arc)(path, rect.x + rect.w - d, rect.y, d, d, 270.0, 90.0);
        (self.api.add_path_arc)(
            path,
            rect.x + rect.w - d,
            rect.y + rect.h - d,
            d,
            d,
            0.0,
            90.0,
        );
        (self.api.add_path_arc)(path, rect.x, rect.y + rect.h - d, d, d, 90.0, 90.0);
        (self.api.close_path_figure)(path);
        path
    }

    pub fn clear(&self) {
        unsafe {
            (self.api.graphics_clear)(self.graphics, 0);
        }
    }

    pub fn blit_to(&self, dst_dc: Handle) {
        unsafe {
            BitBlt(
                dst_dc,
                0,
                0,
                self.width,
                self.height,
                self.mem_dc,
                0,
                0,
                0x00cc_0020,
            );
        }
    }

    pub fn blit_from(&self, src: &Surface<'_>) {
        if self.bits.is_null() || src.bits.is_null() || self.width <= 0 || self.height <= 0 {
            return;
        }
        src.flush();
        self.flush();
        if self.width == src.width && self.height == src.height {
            let len = (self.width as usize) * (self.height as usize) * 4;
            unsafe {
                std::ptr::copy_nonoverlapping(src.bits, self.bits, len);
            }
            return;
        }
        self.clear();
        let copy_w = self.width.min(src.width).max(0) as usize;
        let copy_h = self.height.min(src.height).max(0) as usize;
        let row_bytes = copy_w * 4;
        let src_stride = (src.width.max(0) as usize) * 4;
        let dst_stride = (self.width.max(0) as usize) * 4;
        unsafe {
            for y in 0..copy_h {
                std::ptr::copy_nonoverlapping(
                    src.bits.add(y * src_stride),
                    self.bits.add(y * dst_stride),
                    row_bytes,
                );
            }
        }
    }

    /// Alpha-composites `src` onto `self` vertically positioned at `dst_y_logical`.
    pub fn composite_at(&self, src: &Surface<'_>, dst_y_logical: f32) {
        if self.bits.is_null() || src.bits.is_null() || self.width <= 0 || self.height <= 0 {
            return;
        }
        src.flush();
        self.flush();
        let dst_y0 = (dst_y_logical * self.scale).round() as i32;
        let y_start = dst_y0.max(0);
        let y_end = (dst_y0 + src.height).min(self.height);
        if y_end <= y_start {
            return;
        }
        let px_w = self.width.min(src.width).max(0) as usize;
        let src_stride = src.width.max(0) as usize;
        let dst_stride = self.width.max(0) as usize;
        unsafe {
            let src_u32 = src.bits.cast::<u32>();
            let dst_u32 = self.bits.cast::<u32>();
            for dy in y_start..y_end {
                let sy = (dy - dst_y0) as usize;
                let s_row = src_u32.add(sy * src_stride);
                let d_row = dst_u32.add((dy as usize) * dst_stride);
                for x in 0..px_w {
                    let s_px = *s_row.add(x);
                    if s_px == 0 {
                        continue;
                    }
                    let sa = s_px >> 24;
                    if sa == 255 {
                        *d_row.add(x) = s_px;
                        continue;
                    }
                    let d_px = *d_row.add(x);
                    let inv = 255 - sa;
                    let a = (sa + (((d_px >> 24) * inv + 127) / 255)).min(255);
                    let b = ((s_px & 0xff) + (((d_px & 0xff) * inv + 127) / 255)).min(a);
                    let g =
                        (((s_px >> 8) & 0xff) + ((((d_px >> 8) & 0xff) * inv + 127) / 255)).min(a);
                    let r = (((s_px >> 16) & 0xff) + ((((d_px >> 16) & 0xff) * inv + 127) / 255))
                        .min(a);
                    *d_row.add(x) = b | (g << 8) | (r << 16) | (a << 24);
                }
            }
        }
    }

    pub fn set_round_clip(&self, rect: RectF, r: f32) {
        let rect = self.sx(rect);
        let r = r * self.scale;
        unsafe {
            let path = self.rounded_path(rect, r);
            if !path.is_null() {
                (self.api.set_clip_path)(self.graphics, path, 0);
                (self.api.delete_path)(path);
            }
        }
    }

    pub fn intersect_round_clip(&self, rect: RectF, r: f32) {
        let rect = self.sx(rect);
        let r = r * self.scale;
        unsafe {
            let path = self.rounded_path(rect, r);
            if !path.is_null() {
                (self.api.set_clip_path)(self.graphics, path, 1);
                (self.api.delete_path)(path);
            }
        }
    }

    pub fn draw_image_round(
        &self,
        img: &GpImage<'_>,
        rect: RectF,
        r: f32,
        restore_clip: Option<(RectF, f32)>,
    ) {
        self.intersect_round_clip(rect, r);
        let scaled = self.sx(rect);
        unsafe {
            (self.api.draw_image_rect)(
                self.graphics,
                img.raw,
                scaled.x,
                scaled.y,
                scaled.w,
                scaled.h,
            );
        }
        if let Some((clip_rect, clip_r)) = restore_clip {
            self.set_round_clip(clip_rect, clip_r);
        } else {
            self.reset_clip();
        }
    }

    pub fn reset_clip(&self) {
        unsafe {
            (self.api.reset_clip)(self.graphics);
        }
    }

    pub fn fill_round(&self, rect: RectF, r: f32, color: u32) {
        if (color >> 24) == 0 || rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let rect = self.sx(rect);
        let r = r * self.scale;
        unsafe {
            let path = self.rounded_path(rect, r);
            if !path.is_null() {
                (self.api.set_solid_fill_color)(self.brush, color);
                (self.api.fill_path)(self.graphics, self.brush, path);
                (self.api.delete_path)(path);
            }
        }
    }

    pub fn fill_vertical_gradient(&self, rect: RectF, r: f32, top_color: u32, bottom_color: u32) {
        if ((top_color >> 24) == 0 && (bottom_color >> 24) == 0) || rect.w <= 0.5 || rect.h <= 0.5 {
            return;
        }
        let srect = self.sx(rect);
        let sr = r * self.scale;
        unsafe {
            let path = self.rounded_path(srect, sr);
            if !path.is_null() {
                let p1 = PointF {
                    x: srect.x,
                    y: srect.y,
                };
                let p2 = PointF {
                    x: srect.x,
                    y: srect.y + srect.h.max(1.0),
                };
                let mut brush = null_mut();
                if (self.api.create_line_brush)(&p1, &p2, top_color, bottom_color, 3, &mut brush)
                    == 0
                    && !brush.is_null()
                {
                    (self.api.fill_path)(self.graphics, brush, path);
                    (self.api.delete_brush)(brush);
                }
                (self.api.delete_path)(path);
            }
        }
    }

    pub fn flush(&self) {
        unsafe {
            (self.api.flush)(self.graphics, 1); // FlushIntentionSync
        }
    }

    fn draw_quartz_shell_inner(
        &self,
        rect: RectF,
        r: f32,
        light: bool,
        opaque_backdrop: bool,
        opacity_mul: f32,
        bleed_rgb: Option<(u8, u8, u8)>,
    ) {
        self.flush();
        let mut spec = GlassSpec::shell(light, bleed_rgb);
        spec.face_opacity = (spec.face_opacity * opacity_mul).clamp(0.08, 1.0);
        let pyr = if opaque_backdrop {
            let mut p = self.pyramid.borrow_mut();
            let ok = p.capture(self.bits, self.width, self.height, self.scale, rect, 12, 5);
            drop(p);
            ok
        } else {
            false
        };
        let pyr_ref = pyr.then(|| self.pyramid.borrow());
        if pyr_ref.is_some() {
            spec.refract_scale = 0.85;
        }
        liquid_glass::shade_rounded_glass(
            self.bits,
            self.width,
            self.height,
            self.scale,
            rect,
            r,
            &spec,
            pyr_ref.as_deref(),
            None,
            1.0,
        );
    }

    #[allow(dead_code)]
    pub fn draw_quartz_shell(
        &self,
        rect: RectF,
        r: f32,
        light: bool,
        opaque_backdrop: bool,
        bleed_rgb: Option<(u8, u8, u8)>,
    ) {
        self.draw_quartz_shell_inner(rect, r, light, opaque_backdrop, 1.0, bleed_rgb);
    }

    pub fn draw_quartz_shell_styled(
        &self,
        rect: RectF,
        r: f32,
        light: bool,
        opacity_mul: f32,
        bleed_rgb: Option<(u8, u8, u8)>,
    ) {
        self.draw_quartz_shell_inner(rect, r, light, false, opacity_mul, bleed_rgb);
    }

    #[allow(dead_code)]
    pub fn draw_quartz_panel(&self, rect: RectF, r: f32, light: bool, alpha: f32) {
        self.draw_quartz_panel_styled(rect, r, light, 1.0, alpha);
    }

    pub fn draw_quartz_panel_styled(
        &self,
        rect: RectF,
        r: f32,
        light: bool,
        opacity_mul: f32,
        alpha: f32,
    ) {
        self.flush();
        let mut spec = GlassSpec::panel(light);
        spec.face_opacity = (spec.face_opacity * opacity_mul).clamp(0.08, 1.0);
        let mut pyr = self.pyramid.borrow_mut();
        let ok = pyr.capture(self.bits, self.width, self.height, self.scale, rect, 18, 5);
        liquid_glass::shade_rounded_glass(
            self.bits,
            self.width,
            self.height,
            self.scale,
            rect,
            r,
            &spec,
            ok.then_some(&*pyr),
            None,
            alpha,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_quartz_pill(
        &self,
        rect: RectF,
        r: f32,
        tint: u32,
        hover_t: f32,
        press_t: f32,
        active: bool,
        light: bool,
        alpha: f32,
        restore_clip: Option<(RectF, f32)>,
    ) -> RectF {
        self.flush();
        let drect = Self::deform_gel_rect(rect, hover_t, press_t);
        let dr = (r + (drect.h - rect.h) * 0.5).max(2.0);
        let tint_rgb = ((tint >> 24) > 8).then(|| {
            [
                ((tint >> 16) & 0xff) as f32 / 255.0,
                ((tint >> 8) & 0xff) as f32 / 255.0,
                (tint & 0xff) as f32 / 255.0,
            ]
        });
        let spec = GlassSpec::pill(light, tint_rgb, active, press_t);
        let mut pyr = self.pyramid.borrow_mut();
        let ok = pyr.capture(self.bits, self.width, self.height, self.scale, drect, 12, 3);
        liquid_glass::shade_rounded_glass(
            self.bits,
            self.width,
            self.height,
            self.scale,
            drect,
            dr,
            &spec,
            ok.then_some(&*pyr),
            restore_clip,
            alpha,
        );
        drect
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_quartz_droplet(
        &self,
        bar_bounds: RectF,
        center: [f32; 2],
        half_size: [f32; 2],
        r: f32,
        shown: f32,
        velocity: [f32; 2],
        light: bool,
        alpha: f32,
    ) {
        if alpha <= 0.01 || half_size[0] <= 1.0 || half_size[1] <= 1.0 {
            return;
        }
        self.flush();
        let mut pyr = self.pyramid.borrow_mut();
        if !pyr.capture(
            self.bits,
            self.width,
            self.height,
            self.scale,
            bar_bounds,
            16,
            3,
        ) {
            return;
        }
        liquid_glass::shade_liquid_droplet(
            self.bits,
            self.width,
            self.height,
            self.scale,
            bar_bounds,
            center,
            half_size,
            r,
            shown,
            velocity,
            &pyr,
            light,
            alpha,
        );
    }

    pub fn draw_soft_shadow(&self, rect: RectF, r: f32, base_col: u32) {
        if (base_col >> 24) == 0 || rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        const LAYERS: [(f32, f32, f32); 4] = [
            (9.5, 3.2, 0.28),
            (6.0, 2.2, 0.44),
            (3.4, 1.3, 0.65),
            (1.4, 0.5, 0.90),
        ];
        for &(pad, dy, alpha_mul) in &LAYERS {
            self.fill_round(
                RectF::new(
                    rect.x - pad,
                    rect.y - pad * 0.2 + dy,
                    rect.w + pad * 2.0,
                    rect.h + pad * 1.15,
                ),
                r + pad * 0.72,
                with_alpha(base_col, alpha_mul),
            );
        }
    }

    pub fn fill_radial_glow(
        &self,
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        center_color: u32,
        edge_color: u32,
    ) {
        if rx < 1.0 || ry < 1.0 || ((center_color >> 24) == 0 && (edge_color >> 24) == 0) {
            return;
        }
        let s = self.scale;
        let scx = cx * s;
        let scy = cy * s;
        let srx = rx * s;
        let sry = ry * s;
        unsafe {
            let mut path = null_mut();
            if (self.api.create_path)(0, &mut path) != 0 || path.is_null() {
                return;
            }
            (self.api.add_path_ellipse)(path, scx - srx, scy - sry, srx * 2.0, sry * 2.0);
            let mut brush = null_mut();
            if (self.api.create_path_gradient_from_path)(path, &mut brush) == 0 && !brush.is_null()
            {
                let center = PointF { x: scx, y: scy };
                let mut count = 1i32;
                (self.api.set_path_gradient_center_point)(brush, &center);
                (self.api.set_path_gradient_center_color)(brush, center_color);
                (self.api.set_path_gradient_surround_colors)(brush, &edge_color, &mut count);
                (self.api.set_path_gradient_sigma_blend)(brush, 1.0, 1.0);
                (self.api.fill_path)(self.graphics, brush, path);
                (self.api.delete_brush)(brush);
            }
            (self.api.delete_path)(path);
        }
    }

    pub fn deform_gel_rect(rect: RectF, hover_t: f32, press_t: f32) -> RectF {
        let h = hover_t.clamp(0.0, 1.0);
        let p = press_t.clamp(0.0, 1.0);
        let (dx, dy) = if rect.w <= rect.h * 1.35 {
            let scale = 1.0 + 0.04 * h - 0.08 * p;
            (rect.w * (scale - 1.0) * 0.5, rect.h * (scale - 1.0) * 0.5)
        } else {
            let dw = 0.9 * h + rect.w * 0.024 * p;
            let dh = 0.6 * h - rect.h * 0.072 * p;
            (dw * 0.5, dh * 0.5)
        };
        RectF::new(
            rect.x - dx,
            rect.y - dy,
            (rect.w + dx * 2.0).max(4.0),
            (rect.h + dy * 2.0).max(4.0),
        )
    }

    unsafe fn prepare_pen(&self, width: f32, color: u32, dashed: bool, round_caps: bool) -> bool {
        if self.pen.is_null() {
            return false;
        }
        (self.api.set_pen_color)(self.pen, color);
        (self.api.set_pen_width)(self.pen, (width * self.scale).max(1.0));
        (self.api.set_pen_dash_style)(self.pen, i32::from(dashed));
        let cap = if round_caps { 2 } else { 0 };
        (self.api.set_pen_start_cap)(self.pen, cap);
        (self.api.set_pen_end_cap)(self.pen, cap);
        true
    }

    fn needs_fallback(text: &str) -> bool {
        text.chars().any(|c| {
            let u = c as u32;
            u > 0x024F
                && !(0x1E00..=0x1EFF).contains(&u)
                && !matches!(c, '–' | '—' | '…' | '•' | '°' | '·' | '×' | '€' | '™')
        })
    }

    fn has_font_family(&self) -> bool {
        !self.font_family.is_null()
            || !self.font_text.is_null()
            || !self.font_display.is_null()
            || !self.font_semibold.is_null()
    }

    unsafe fn cached_font(&self, size: f32, bold: bool, use_fallback: bool) -> Gp {
        let px = (size * self.scale).max(6.0);
        let key = (px * 10.0).round() as u16;
        let (family, style, bucket) = if use_fallback {
            (
                self.font_family,
                i32::from(bold),
                if bold { 5u8 } else { 4u8 },
            )
        } else if size >= 15.0 {
            let fam = if !self.font_display.is_null() {
                self.font_display
            } else {
                self.font_family
            };
            (fam, i32::from(bold), if bold { 3u8 } else { 2u8 })
        } else if bold {
            if !self.font_semibold.is_null() {
                (self.font_semibold, 0, 1u8)
            } else {
                (self.font_family, 1, 1u8)
            }
        } else {
            let fam = if !self.font_text.is_null() {
                self.font_text
            } else {
                self.font_family
            };
            (fam, 0, 0u8)
        };
        if family.is_null() {
            return null_mut();
        }
        let mut fonts = self.fonts.borrow_mut();
        if let Some(&(_, _, f)) = fonts.iter().find(|&&(k, b, _)| k == key && b == bucket) {
            return f;
        }
        let em = key as f32 * 0.1;
        let mut font = null_mut();
        let mut ok =
            (self.api.create_font)(family, em, style, 2, &mut font) == 0 && !font.is_null();
        if !ok && family != self.font_family && !self.font_family.is_null() {
            font = null_mut();
            ok = (self.api.create_font)(self.font_family, em, style, 2, &mut font) == 0
                && !font.is_null();
        }
        if !ok && style != 0 {
            let fallback_fam = if !self.font_family.is_null() {
                self.font_family
            } else {
                family
            };
            font = null_mut();
            ok = (self.api.create_font)(fallback_fam, em, 0, 2, &mut font) == 0 && !font.is_null();
        }
        if ok {
            fonts.push((key, bucket, font));
            font
        } else {
            null_mut()
        }
    }

    pub fn stroke_round(&self, rect: RectF, r: f32, width: f32, color: u32, dashed: bool) {
        if (color >> 24) == 0 || rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let rect = self.sx(rect);
        let r = r * self.scale;
        unsafe {
            let path = self.rounded_path(rect, r);
            if !path.is_null() {
                if self.prepare_pen(width, color, dashed, false) {
                    (self.api.draw_path)(self.graphics, self.pen, path);
                }
                (self.api.delete_path)(path);
            }
        }
    }

    pub fn fill_circle(&self, cx: f32, cy: f32, r: f32, color: u32) {
        if (color >> 24) == 0 || r <= 0.0 {
            return;
        }
        let s = self.scale;
        let rs = r * s;
        unsafe {
            (self.api.set_solid_fill_color)(self.brush, color);
            (self.api.fill_ellipse)(
                self.graphics,
                self.brush,
                cx * s - rs,
                cy * s - rs,
                rs * 2.0,
                rs * 2.0,
            );
        }
    }

    pub fn stroke_circle(&self, cx: f32, cy: f32, r: f32, width: f32, color: u32) {
        if (color >> 24) == 0 || r <= 0.0 {
            return;
        }
        let s = self.scale;
        let rs = r * s;
        unsafe {
            if self.prepare_pen(width, color, false, false) {
                (self.api.draw_ellipse)(
                    self.graphics,
                    self.pen,
                    cx * s - rs,
                    cy * s - rs,
                    rs * 2.0,
                    rs * 2.0,
                );
            }
        }
    }

    pub fn stroke_arc(&self, rect: RectF, width: f32, start_deg: f32, sweep_deg: f32, color: u32) {
        if (color >> 24) == 0 || sweep_deg.abs() < 0.5 || rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let rect = self.sx(rect);
        unsafe {
            if self.prepare_pen(width, color, false, true) {
                (self.api.draw_arc)(
                    self.graphics,
                    self.pen,
                    rect.x,
                    rect.y,
                    rect.w,
                    rect.h,
                    start_deg,
                    sweep_deg,
                );
            }
        }
    }

    pub fn draw_line(&self, p1: (f32, f32), p2: (f32, f32), width: f32, color: u32) {
        if (color >> 24) == 0 {
            return;
        }
        let s = self.scale;
        unsafe {
            if self.prepare_pen(width, color, false, true) {
                (self.api.draw_line)(
                    self.graphics,
                    self.pen,
                    p1.0 * s,
                    p1.1 * s,
                    p2.0 * s,
                    p2.1 * s,
                );
            }
        }
    }

    pub fn draw_polyline(&self, pts: &[(f32, f32)], width: f32, color: u32) {
        if (color >> 24) == 0 || pts.len() < 2 {
            return;
        }
        let s = self.scale;
        unsafe {
            if self.prepare_pen(width, color, false, true) {
                for pair in pts.windows(2) {
                    (self.api.draw_line)(
                        self.graphics,
                        self.pen,
                        pair[0].0 * s,
                        pair[0].1 * s,
                        pair[1].0 * s,
                        pair[1].1 * s,
                    );
                }
            }
        }
    }

    pub fn fill_polygon(&self, pts: &[(f32, f32)], color: u32) {
        if (color >> 24) == 0 || pts.len() < 3 {
            return;
        }
        let s = self.scale;
        let scaled: Vec<PointF> = pts
            .iter()
            .map(|&(x, y)| PointF { x: x * s, y: y * s })
            .collect();
        unsafe {
            (self.api.set_solid_fill_color)(self.brush, color);
            (self.api.fill_polygon)(
                self.graphics,
                self.brush,
                scaled.as_ptr(),
                scaled.len() as i32,
                0,
            );
        }
    }

    pub fn measure_text_width(&self, text: &str, size: f32, bold: bool) -> f32 {
        if text.is_empty() || !self.has_font_family() {
            return 0.0;
        }
        let fmt = self.formats[0];
        if fmt.is_null() {
            return (text.chars().count() as f32) * size * 0.56;
        }
        let mut stack = [0u16; 96];
        let mut len = 0;
        let mut overflow = false;
        for ch in text.encode_utf16() {
            if len == stack.len() {
                overflow = true;
                break;
            }
            stack[len] = ch;
            len += 1;
        }
        let layout = RectF::new(0.0, 0.0, 2000.0 * self.scale, 200.0 * self.scale);
        let mut bbox = RectF::default();
        unsafe {
            let font = self.cached_font(size, bold, Self::needs_fallback(text));
            if font.is_null() {
                return (text.chars().count() as f32) * size * 0.56;
            }
            let status = if !overflow {
                (self.api.measure_string)(
                    self.graphics,
                    stack.as_ptr(),
                    len as i32,
                    font,
                    &layout,
                    fmt,
                    &mut bbox,
                    null_mut(),
                    null_mut(),
                )
            } else {
                let wide_text = wide(text);
                (self.api.measure_string)(
                    self.graphics,
                    wide_text.as_ptr(),
                    (wide_text.len() as i32) - 1,
                    font,
                    &layout,
                    fmt,
                    &mut bbox,
                    null_mut(),
                    null_mut(),
                )
            };
            if status == 0 && bbox.w > 0.0 {
                (bbox.w / self.scale).ceil()
            } else {
                (text.chars().count() as f32) * size * 0.56
            }
        }
    }

    pub fn draw_text(&self, text: &str, rect: RectF, style: TextStyle) {
        if (style.color >> 24) == 0 || text.is_empty() || !self.has_font_family() {
            return;
        }
        let rect = self.sx(rect);
        let fmt = self.formats[style.align.clamp(0, 2) as usize];
        if fmt.is_null() {
            return;
        }
        let mut stack = [0u16; 96];
        let mut len = 0;
        let mut overflow = false;
        for ch in text.encode_utf16() {
            if len == stack.len() {
                overflow = true;
                break;
            }
            stack[len] = ch;
            len += 1;
        }
        unsafe {
            let font = self.cached_font(style.size, style.bold, Self::needs_fallback(text));
            if font.is_null() {
                return;
            }
            (self.api.set_solid_fill_color)(self.brush, style.color);
            if !overflow {
                (self.api.draw_string)(
                    self.graphics,
                    stack.as_ptr(),
                    len as i32,
                    font,
                    &rect,
                    fmt,
                    self.brush,
                );
            } else {
                let wide_text = wide(text);
                (self.api.draw_string)(
                    self.graphics,
                    wide_text.as_ptr(),
                    (wide_text.len() as i32) - 1,
                    font,
                    &rect,
                    fmt,
                    self.brush,
                );
            }
        }
    }

    pub fn present(&self, hwnd: Handle, x: i32, y: i32) {
        unsafe {
            let screen_dc = GetDC(0);
            let dst_pt = Point { x, y };
            let src_pt = Point { x: 0, y: 0 };
            let size = Size {
                cx: self.width,
                cy: self.height,
            };
            let blend = BlendFunction {
                op: 0,
                flags: 0,
                alpha: 255,
                format: 1, // AC_SRC_ALPHA
            };
            UpdateLayeredWindow(
                hwnd,
                screen_dc,
                &dst_pt,
                &size,
                self.mem_dc,
                &src_pt,
                0,
                &blend,
                2, // ULW_ALPHA
            );
            if screen_dc != 0 {
                ReleaseDC(0, screen_dc);
            }
        }
    }
}

impl Drop for Surface<'_> {
    fn drop(&mut self) {
        unsafe {
            if !self.graphics.is_null() {
                (self.api.delete_graphics)(self.graphics);
            }
            if !self.bitmap.is_null() {
                (self.api.dispose_image)(self.bitmap);
            }
            for &(_, _, font) in self.fonts.borrow().iter() {
                if !font.is_null() {
                    (self.api.delete_font)(font);
                }
            }
            if !self.pen.is_null() {
                (self.api.delete_pen)(self.pen);
            }
            if !self.brush.is_null() {
                (self.api.delete_brush)(self.brush);
            }
            for fmt in self.formats {
                if !fmt.is_null() {
                    (self.api.delete_string_format)(fmt);
                }
            }
            for fam in [
                self.font_display,
                self.font_text,
                self.font_semibold,
                self.font_family,
            ] {
                if !fam.is_null() {
                    (self.api.delete_font_family)(fam);
                }
            }
            if self.mem_dc != 0 {
                if self.old_bmp != 0 {
                    SelectObject(self.mem_dc, self.old_bmp);
                }
                if self.dib != 0 {
                    DeleteObject(self.dib);
                }
                DeleteDC(self.mem_dc);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dominant_color_skips_neutrals_and_extracts_chroma() {
        let bytes = [
            250, 250, 250, 255, // near-white (ignored)
            10, 10, 10, 255, // near-black (ignored)
            70, 90, 220, 255, // coral
            70, 90, 220, 255, // coral
        ];
        let rgb = extract_dominant_bgra(&bytes).expect("dominant rgb");
        assert!(rgb.0 > rgb.1 && rgb.0 > rgb.2, "got {rgb:?}");
    }

    #[test]
    fn optical_liquid_glass_primitives_smoke() {
        let api = GdiplusApi::load().expect("gdiplus.dll loads all optical symbols");
        let surf = Surface::new(&api, 160, 80, 96).expect("surface creates");
        let rect = RectF::new(10.0, 10.0, 140.0, 60.0);
        surf.draw_soft_shadow(rect, 18.0, argb(40, 0, 0, 0));
        surf.draw_quartz_shell_styled(rect, 18.0, false, 1.0, None);
        surf.draw_quartz_panel_styled(RectF::new(18.0, 14.0, 120.0, 50.0), 14.0, false, 1.0, 1.0);
        let pill = surf.draw_quartz_pill(
            RectF::new(20.0, 20.0, 80.0, 28.0),
            14.0,
            argb(66, 46, 74, 60),
            0.8,
            0.35,
            true,
            false,
            1.0,
            Some((rect, 18.0)),
        );
        assert!(pill.w > 78.0);
        surf.draw_text(
            "Hello 123 · 45°",
            rect,
            TextStyle::new(12.0, true, argb(255, 255, 255, 255), 1),
        );
        surf.draw_text(
            "Control Center",
            rect,
            TextStyle::new(18.0, true, argb(255, 255, 255, 255), 0),
        );
        assert!(surf.has_font_family(), "Segoe UI font family must load");
        let copy = Surface::new(&api, 160, 80, 96).expect("copy surface creates");
        copy.blit_from(&surf);
        assert!(surf.measure_text_width("Segoe UI", 12.0, false) > 0.0);
    }
}
