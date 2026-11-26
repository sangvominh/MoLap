use crate::{
    gdiplus::{self, argb, mix_color, with_alpha, GdiplusApi, RectF, Surface, TextStyle},
    island_data::Spring,
    liquid_glass::LiquidNav,
    plugin::PluginEntry,
    settings::{
        self, GlassStyle, IdleMode, IslandAnchor, MotionSpeed, Palette, TaskbarFit, ThemeMode,
    },
    win::*,
};
use std::{
    cell::RefCell,
    ffi::c_void,
    mem::{size_of, transmute},
    ptr::null,
    time::Instant,
};

const WIN_W: f32 = 600.0;
const WIN_H: f32 = 492.0;
const TIMER_ANIM: usize = 1;

const HEADER_H: f32 = 62.0;
const CONTENT_TOP: f32 = 72.0;
const VIEW_CLIP: RectF = RectF::new(0.0, 0.0, WIN_W, WIN_H);

const NAV_BAR: RectF = RectF::new(138.0, 428.0, 324.0, 50.0);
const NAV_CY: f32 = 453.0;
const NAV_HALF_H: f32 = 22.0;
const NAV_SLOTS: [(f32, f32); 3] = [(194.0, 96.0), (300.0, 96.0), (406.0, 96.0)];
const HIT_CLIP: RectF = RectF::new(16.0, HEADER_H, WIN_W - 32.0, NAV_BAR.y - HEADER_H - 6.0);

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
enum Tab {
    General = 0,
    Island = 1,
    Plugins = 2,
}

impl Tab {
    fn from_idx(i: usize) -> Self {
        match i {
            1 => Self::Island,
            2 => Self::Plugins,
            _ => Self::General,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SliderKind {
    OffsetX,
    OffsetY,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctrl {
    Close,
    TogglePin,
    ShowReadings,
    ResetDefaults,
    Theme(ThemeMode),
    GlassStyle(GlassStyle),
    MotionSpeed(MotionSpeed),
    TempUnit(bool),
    ToggleIsland,
    ToggleStartup,
    TriggerPawnIo,
    ScalePreset(u8),
    ScaleStep(i8),
    IslandAnchor(IslandAnchor),
    TaskbarFit(TaskbarFit),
    OffsetXStep(i16),
    OffsetYStep(i16),
    ResetOffsets,
    Slider(SliderKind),
    ToggleExpandHover,
    ToggleHideFullscreen,
    ToggleVolumeHud,
    IdleMode(IdleMode),
    ToggleAlerts,
    TestAlert,
    ToggleToastPower,
    ToggleToastAudio,
    ToggleToastDevices,
    ToggleCpuHog,
    TogglePrivacyDots,
    ToggleLyrics,
    ToggleLyricsPill,
    TogglePage(usize),
    TogglePlugin(usize),
    ReloadPlugins,
    OpenPluginsDir,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct BgCacheKey {
    px_w: i32,
    px_h: i32,
    dpi: u32,
    light: bool,
    glass_style: GlassStyle,
}

struct SettingsWindow<'a> {
    hwnd: Handle,
    api: &'a GdiplusApi,
    surface: Option<Surface<'a>>,
    bg_cache: Option<Surface<'a>>,
    bg_cache_key: Option<BgCacheKey>,
    content_cache: Option<Surface<'a>>,
    content_dirty: bool,
    content_total_h: f32,
    raw_controls: Vec<(RectF, Ctrl)>,
    cached_plugins: Vec<PluginEntry>,
    origin: Point,
    dpi: u32,
    visible: bool,
    last_light: Option<bool>,
    startup_on: bool,
    needs_temp: bool,
    tab: Tab,
    nav: LiquidNav,
    scroll_y: [Spring; 3],
    max_scroll: [f32; 3],
    hovered: Option<Ctrl>,
    pressed: Option<Ctrl>,
    press_spring: Spring,
    dragging_slider: Option<(SliderKind, RectF)>,
    dirty: bool,
    animating: bool,
    last_frame: Instant,
    hi_res_timer: Option<HiResTimer>,
    controls: Vec<(RectF, Ctrl)>,
    hand_cursor: Handle,
}

thread_local! {
    static SETTINGS_WIN: RefCell<Option<SettingsWindow<'static>>> = const { RefCell::new(None) };
}

fn with_win<T>(f: impl FnOnce(&mut SettingsWindow<'static>) -> T) -> Option<T> {
    SETTINGS_WIN.with(|cell| {
        cell.try_borrow_mut()
            .ok()
            .and_then(|mut opt| opt.as_mut().map(f))
    })
}

fn primary_work_and_dpi() -> (Rect, u32) {
    unsafe {
        let mut info = MonitorInfo {
            size: size_of::<MonitorInfo>() as u32,
            bounds: Rect::default(),
            work: Rect::default(),
            flags: 0,
        };
        let mon = MonitorFromWindow(0, 1);
        if mon != 0 && GetMonitorInfoW(mon, &mut info) != 0 {
            let taskbar = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
            let dpi = if taskbar != 0 {
                match GetDpiForWindow(taskbar) {
                    0 => 96,
                    d => d,
                }
            } else {
                96
            };
            return (info.work, dpi);
        }
    }
    (
        Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        },
        96,
    )
}

fn intersect_rect(a: RectF, b: RectF) -> Option<RectF> {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    (x1 > x0 && y1 > y0).then(|| RectF::new(x0, y0, x1 - x0, y1 - y0))
}

fn push_clipped_ctrl(ctrls: &mut Vec<(RectF, Ctrl)>, rect: RectF, clip: RectF, ctrl: Ctrl) {
    if let Some(clipped) = intersect_rect(rect, clip) {
        ctrls.push((clipped, ctrl));
    }
}

fn slider_track_rect(cx: f32, y: f32, cw: f32) -> RectF {
    RectF::new(cx + 32.0, y + 20.0, cw - 64.0, 24.0)
}

impl<'a> SettingsWindow<'a> {
    fn new(hwnd: Handle, api: &'a GdiplusApi) -> Self {
        let (work, dpi) = primary_work_and_dpi();
        let scale = (dpi.max(96) as f32) / 96.0;
        let px_w = (WIN_W * scale).round() as i32;
        let px_h = (WIN_H * scale).round() as i32;
        let origin = Point {
            x: work.left + (work.width() - px_w) / 2,
            y: work.top + (work.height() - px_h) / 2,
        };
        let surface = Surface::new(api, px_w, px_h, dpi);
        let hand_cursor = unsafe { LoadCursorW(0, 32649usize as *const u16) };
        Self {
            hwnd,
            api,
            surface,
            bg_cache: None,
            bg_cache_key: None,
            content_cache: None,
            content_dirty: true,
            content_total_h: 0.0,
            raw_controls: Vec::new(),
            cached_plugins: Vec::new(),
            origin,
            dpi,
            visible: false,
            last_light: None,
            startup_on: crate::startup::enabled(),
            needs_temp: crate::ui::needs_temp_setup(),
            tab: Tab::General,
            nav: LiquidNav::new(&NAV_SLOTS, NAV_CY, NAV_HALF_H, 0),
            scroll_y: [Spring::new(0.0); 3],
            max_scroll: [0.0; 3],
            hovered: None,
            pressed: None,
            press_spring: Spring::new(0.0),
            dragging_slider: None,
            dirty: false,
            animating: false,
            last_frame: Instant::now(),
            hi_res_timer: HiResTimer::load(),
            controls: Vec::new(),
            hand_cursor,
        }
    }

    fn is_header_ctrl(ctrl: Ctrl) -> bool {
        matches!(
            ctrl,
            Ctrl::Close | Ctrl::TogglePin | Ctrl::ShowReadings | Ctrl::ResetDefaults
        )
    }

    fn refresh_system_flags(&mut self) {
        self.startup_on = crate::startup::enabled();
        self.needs_temp = crate::ui::needs_temp_setup();
        self.content_dirty = true;
    }

    fn refresh_plugins(&mut self) {
        self.cached_plugins = crate::island::plugin_snapshot();
        self.content_dirty = true;
    }

    fn apply_backdrop(&mut self, light: bool) {
        self.last_light = Some(light);
        unsafe {
            let margins = Margins {
                left: -1,
                right: -1,
                top: -1,
                bottom: -1,
            };
            dwm_extend_frame(self.hwnd, &margins);
            let dark: i32 = i32::from(!light);
            dwm_set_window_attribute(
                self.hwnd,
                20, // DWMWA_USE_IMMERSIVE_DARK_MODE
                (&dark as *const i32).cast::<c_void>(),
                4,
            );
            let round: i32 = 2; // DWMWCP_ROUND
            dwm_set_window_attribute(
                self.hwnd,
                33, // DWMWA_WINDOW_CORNER_PREFERENCE
                (&round as *const i32).cast::<c_void>(),
                4,
            );
            let no_border: u32 = 0xFFFF_FFFE; // DWMWA_COLOR_NONE
            dwm_set_window_attribute(
                self.hwnd,
                34, // DWMWA_BORDER_COLOR
                (&no_border as *const u32).cast::<c_void>(),
                4,
            );
            let backdrop: i32 = 3; // DWMSBT_TRANSIENTWINDOW (Windows 11 Desktop Acrylic)
            dwm_set_window_attribute(
                self.hwnd,
                38, // DWMWA_SYSTEMBACKDROP_TYPE
                (&backdrop as *const i32).cast::<c_void>(),
                4,
            );
        }
    }

    fn start_anim(&mut self) {
        if !self.visible {
            return;
        }
        self.dirty = true;
        if !self.animating {
            self.animating = true;
            self.last_frame = Instant::now();
            if let Some(timer) = self.hi_res_timer.as_mut() {
                timer.set_active(true);
            }
            unsafe {
                SetTimer(self.hwnd, TIMER_ANIM, 8, 0);
            }
        }
    }

    fn stop_anim(&mut self) {
        self.animating = false;
        if let Some(timer) = self.hi_res_timer.as_mut() {
            timer.set_active(false);
        }
        unsafe {
            KillTimer(self.hwnd, TIMER_ANIM);
        }
    }

    fn on_anim_tick(&mut self) {
        if !self.visible {
            self.stop_anim();
            return;
        }
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32();
        self.last_frame = now;

        let speed = settings::get().motion_speed.spring_scale();
        let speed_sqrt = speed.sqrt();

        self.nav.update(dt * speed_sqrt);
        let m_nav = self.nav.animating();
        let m_press = self.press_spring.step(dt, 760.0 * speed, 32.0 * speed_sqrt);
        if m_press && !self.pressed.is_some_and(Self::is_header_ctrl) {
            self.content_dirty = true;
        }

        let tab_idx = self.tab as usize;
        let max_s = self.max_scroll[tab_idx];
        self.scroll_y[tab_idx].target = self.scroll_y[tab_idx].target.clamp(0.0, max_s);
        let m_scroll = self.scroll_y[tab_idx].step(dt, 420.0 * speed, 36.0 * speed_sqrt);
        self.scroll_y[tab_idx].value = self.scroll_y[tab_idx].value.clamp(0.0, max_s);

        let mut pt = Point::default();
        if self.hovered.is_some()
            && self.dragging_slider.is_none()
            && unsafe { GetCursorPos(&mut pt) } != 0
        {
            let s = self.scale();
            let px = (pt.x - self.origin.x) as f32 / s;
            let py = (pt.y - self.origin.y) as f32 / s;
            if !VIEW_CLIP.contains(px, py) {
                if let Some(prev) = self.hovered.take() {
                    if !Self::is_header_ctrl(prev) {
                        self.content_dirty = true;
                    }
                    self.dirty = true;
                }
            }
        }

        let still_anim = m_nav || m_press || m_scroll;
        if still_anim || self.dirty || self.content_dirty {
            self.dirty = false;
            self.render();
        }

        if !still_anim {
            self.stop_anim();
        }
    }

    fn interaction_state(&self, ctrl: Ctrl) -> (f32, f32) {
        let h = if self.hovered == Some(ctrl) { 1.0 } else { 0.0 };
        let p = if self.pressed == Some(ctrl)
            || (self.pressed.is_none() && self.hovered == Some(ctrl))
        {
            self.press_spring.value.clamp(0.0, 1.0)
        } else {
            0.0
        };
        (h, p)
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_quartz_control(
        &self,
        surf: &Surface<'_>,
        pal: &Palette,
        rect: RectF,
        r: f32,
        ctrl: Ctrl,
        active: bool,
    ) -> RectF {
        let (ht, pt) = self.interaction_state(ctrl);
        let tint = if active { pal.seg_active_bg } else { 0 };
        surf.draw_quartz_pill(rect, r, tint, ht, pt, active, pal.light, 1.0, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_pearl_knob(&self, surf: &Surface<'_>, pal: &Palette, rect: RectF, a: f32) {
        let r = rect.h * 0.5;
        surf.fill_round(
            RectF::new(rect.x - 0.5, rect.y + 1.2, rect.w + 1.0, rect.h + 1.2),
            r + 0.5,
            with_alpha(pal.shadow_outer, a * if pal.light { 0.65 } else { 0.90 }),
        );
        let (top_c, bot_c) = if pal.light {
            (
                with_alpha(argb(255, 255, 255, 255), a),
                with_alpha(argb(255, 232, 236, 244), a),
            )
        } else {
            (
                with_alpha(argb(255, 248, 250, 255), a),
                with_alpha(argb(255, 212, 218, 230), a),
            )
        };
        surf.fill_vertical_gradient(rect, r, top_c, bot_c);
        surf.stroke_round(rect, r, 0.8, with_alpha(pal.specular_top, 0.72 * a), false);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_quartz_switch(
        &self,
        surf: &Surface<'_>,
        pal: &Palette,
        rect: RectF,
        on: bool,
        accent: u32,
        a: f32,
        ctrl: Ctrl,
    ) {
        let (ht, pt) = self.interaction_state(ctrl);
        let drect = Surface::deform_gel_rect(rect, ht * 0.45, pt * 0.6);
        let tr = drect.h * 0.5;
        let bg = with_alpha(
            if on {
                accent
            } else {
                mix_color(pal.track, pal.btn_hover, ht)
            },
            a,
        );
        surf.fill_round(drect, tr, bg);
        let thumb_w = 18.5 + pt * 3.5 + ht * 0.8;
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
        self.draw_pearl_knob(surf, pal, thumb_r, a);
    }

    fn scale(&self) -> f32 {
        (self.dpi.max(96) as f32) / 96.0
    }

    fn ensure_surface(&mut self) {
        let (work, dpi) = primary_work_and_dpi();
        if self.dpi != dpi || self.surface.is_none() {
            self.dpi = dpi;
            let scale = self.scale();
            let px_w = (WIN_W * scale).round() as i32;
            let px_h = (WIN_H * scale).round() as i32;
            self.surface = Surface::new(self.api, px_w, px_h, dpi);
            self.bg_cache = None;
            self.bg_cache_key = None;
            self.content_cache = None;
            self.content_dirty = true;
            self.origin = Point {
                x: work.left + (work.width() - px_w) / 2,
                y: work.top + (work.height() - px_h) / 2,
            };
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    0,
                    self.origin.x,
                    self.origin.y,
                    px_w,
                    px_h,
                    0x0014,
                );
            }
        }
    }

    fn ensure_bg_cache(&mut self, pal: &Palette, glass_style: GlassStyle) {
        let scale = self.scale();
        let px_w = (WIN_W * scale).round() as i32;
        let px_h = (WIN_H * scale).round() as i32;
        let key = BgCacheKey {
            px_w,
            px_h,
            dpi: self.dpi,
            light: pal.light,
            glass_style,
        };
        if self.bg_cache.is_some() && self.bg_cache_key == Some(key) {
            return;
        }
        let dims_changed = self
            .bg_cache_key
            .map(|k| k.px_w != px_w || k.px_h != px_h || k.dpi != self.dpi)
            .unwrap_or(true);
        if self.bg_cache.is_none() || dims_changed {
            self.bg_cache = Surface::new(self.api, px_w, px_h, self.dpi);
        }
        let Some(cache) = self.bg_cache.as_ref() else {
            return;
        };

        cache.clear();

        let (veil_a_top, veil_a_bot) = match (pal.light, glass_style) {
            (true, GlassStyle::Clear) => (18u8, 22u8),
            (true, GlassStyle::Frosted) => (42u8, 52u8),
            (true, GlassStyle::Solid) => (142u8, 156u8),
            (false, GlassStyle::Clear) => (62u8, 72u8),
            (false, GlassStyle::Frosted) => (98u8, 114u8),
            (false, GlassStyle::Solid) => (178u8, 194u8),
        };
        let (veil_top, veil_bot) = if pal.light {
            (
                argb(veil_a_top, 244, 246, 250),
                argb(veil_a_bot, 234, 238, 246),
            )
        } else {
            (argb(veil_a_top, 14, 15, 20), argb(veil_a_bot, 9, 10, 14))
        };
        cache.fill_vertical_gradient(RectF::new(0.0, 0.0, WIN_W, WIN_H), 0.0, veil_top, veil_bot);

        let (bloom_a, bloom_b, bloom_c) = if pal.light {
            (
                argb(36, 255, 255, 255),
                argb(24, 214, 222, 238),
                argb(28, 255, 255, 255),
            )
        } else {
            (
                argb(26, 200, 210, 228),
                argb(20, 140, 152, 176),
                argb(22, 180, 190, 210),
            )
        };
        cache.fill_radial_glow(130.0, 150.0, 195.0, 145.0, bloom_a, bloom_a & 0x00ff_ffff);
        cache.fill_radial_glow(490.0, 310.0, 205.0, 145.0, bloom_b, bloom_b & 0x00ff_ffff);
        cache.fill_radial_glow(300.0, 430.0, 220.0, 105.0, bloom_c, bloom_c & 0x00ff_ffff);

        cache.stroke_round(
            RectF::new(0.5, 0.5, WIN_W - 1.0, WIN_H - 1.0),
            8.0,
            0.9,
            with_alpha(pal.specular_top, if pal.light { 0.55 } else { 0.22 }),
            false,
        );

        self.bg_cache_key = Some(key);
    }

    fn ensure_content_cache(&mut self, pal: &Palette, s: &settings::AppSettings) {
        let scale = self.scale();
        let px_w = (WIN_W * scale).round() as i32;
        let est_logical_h = (960.0_f32).max(160.0 + self.cached_plugins.len() as f32 * 70.0);
        let min_px_h = (est_logical_h * scale).ceil() as i32;
        let need_alloc = self
            .content_cache
            .as_ref()
            .is_none_or(|c| c.width != px_w || c.height < min_px_h);
        if need_alloc {
            self.content_cache = Surface::new(self.api, px_w, min_px_h, self.dpi);
            self.content_dirty = true;
        }
        if !self.content_dirty {
            return;
        }
        let Some(content) = self.content_cache.as_ref() else {
            return;
        };

        content.clear();
        let cx = 26.0;
        let cw = WIN_W - 52.0;
        let mut raw_ctrls = Vec::with_capacity(48);
        let total_h = match self.tab {
            Tab::General => self.render_general_tab(content, pal, s, cx, 0.0, cw, &mut raw_ctrls),
            Tab::Island => self.render_island_tab(content, pal, s, cx, 0.0, cw, &mut raw_ctrls),
            Tab::Plugins => self.render_plugins_tab(content, pal, s, cx, 0.0, cw, &mut raw_ctrls),
        };

        let needed_px_h = ((total_h + 48.0) * scale).ceil() as i32;
        if needed_px_h > content.height {
            self.content_cache = Surface::new(self.api, px_w, needed_px_h, self.dpi);
            if let Some(grown) = self.content_cache.as_ref() {
                grown.clear();
                raw_ctrls.clear();
                let grown_h = match self.tab {
                    Tab::General => {
                        self.render_general_tab(grown, pal, s, cx, 0.0, cw, &mut raw_ctrls)
                    }
                    Tab::Island => {
                        self.render_island_tab(grown, pal, s, cx, 0.0, cw, &mut raw_ctrls)
                    }
                    Tab::Plugins => {
                        self.render_plugins_tab(grown, pal, s, cx, 0.0, cw, &mut raw_ctrls)
                    }
                };
                self.content_total_h = grown_h;
            }
        } else {
            self.content_total_h = total_h;
        }

        self.raw_controls = raw_ctrls;
        self.content_dirty = false;
    }

    fn show(&mut self) {
        self.ensure_surface();
        self.refresh_system_flags();
        self.refresh_plugins();
        let light = settings::get().is_light();
        self.apply_backdrop(light);
        let scale = self.scale();
        let px_w = (WIN_W * scale).round() as i32;
        let px_h = (WIN_H * scale).round() as i32;
        self.visible = true;
        unsafe {
            SetWindowPos(
                self.hwnd,
                0,
                self.origin.x,
                self.origin.y,
                px_w,
                px_h,
                0x0014,
            );
        }
        self.render();
        unsafe {
            ShowWindow(self.hwnd, 5); // SW_SHOW
            SetForegroundWindow(self.hwnd);
        }
    }

    fn hide(&mut self) {
        self.visible = false;
        self.dragging_slider = None;
        self.nav.cancel();
        self.hovered = None;
        self.pressed = None;
        self.dirty = false;
        self.stop_anim();
        settings::flush_save();
        unsafe {
            ShowWindow(self.hwnd, 0);
        }
    }

    fn present_surface(&self) {
        let Some(surf) = self.surface.as_ref() else {
            return;
        };
        unsafe {
            let dc = GetDC(self.hwnd);
            if dc != 0 {
                surf.blit_to(dc);
                ReleaseDC(self.hwnd, dc);
            }
        }
    }

    fn render(&mut self) {
        if !self.visible {
            return;
        }
        let s = settings::get();
        let pal = Palette::for_theme(s.is_light());
        self.ensure_bg_cache(&pal, s.glass_style);
        self.ensure_content_cache(&pal, &s);
        let (Some(surf), Some(cache), Some(content)) = (
            self.surface.as_ref(),
            self.bg_cache.as_ref(),
            self.content_cache.as_ref(),
        ) else {
            return;
        };

        let tab_idx = self.tab as usize;
        let view_h = WIN_H - CONTENT_TOP - 82.0;
        let max_s = (self.content_total_h - view_h).max(0.0);
        self.max_scroll[tab_idx] = max_s;
        self.scroll_y[tab_idx].target = self.scroll_y[tab_idx].target.clamp(0.0, max_s);
        self.scroll_y[tab_idx].value = self.scroll_y[tab_idx].value.clamp(0.0, max_s);
        let scroll = self.scroll_y[tab_idx].value;

        let mut ctrls = Vec::with_capacity(self.raw_controls.len() + 8);
        surf.blit_from(cache);
        surf.set_round_clip(VIEW_CLIP, 8.0);
        surf.composite_at(content, CONTENT_TOP - scroll);
        surf.reset_clip();

        for &(raw_r, ctrl) in &self.raw_controls {
            let shifted = RectF::new(raw_r.x, raw_r.y + CONTENT_TOP - scroll, raw_r.w, raw_r.h);
            push_clipped_ctrl(&mut ctrls, shifted, HIT_CLIP, ctrl);
        }

        if max_s > 1.0 && self.content_total_h > 0.0 {
            let track_y = HEADER_H + 8.0;
            let track_h = NAV_BAR.y - track_y - 12.0;
            let thumb_h = (track_h * (view_h / self.content_total_h)).clamp(24.0, track_h * 0.72);
            let frac = (scroll / max_s).clamp(0.0, 1.0);
            let thumb_y = track_y + (track_h - thumb_h) * frac;
            surf.fill_round(
                RectF::new(WIN_W - 9.0, thumb_y, 3.0, thumb_h),
                1.5,
                with_alpha(pal.muted, 0.36),
            );
        }

        // Floating Header (crystallizes into refractive glass capsule once content scrolls underneath)
        if scroll > 2.0 {
            let hdr_alpha = (scroll / 20.0).clamp(0.0, 1.0);
            surf.draw_quartz_panel_styled(
                RectF::new(14.0, 8.0, WIN_W - 28.0, 50.0),
                18.0,
                pal.light,
                s.glass_style.opacity_mul(),
                hdr_alpha,
            );
        }

        surf.draw_text(
            "MOLAP",
            RectF::new(26.0, 14.0, 120.0, 12.0),
            TextStyle::new(8.8, true, pal.muted, 0),
        );
        let heading = match self.tab {
            Tab::General => "General",
            Tab::Island => "Dynamic Island",
            Tab::Plugins => "Plugins",
        };
        surf.draw_text(
            heading,
            RectF::new(26.0, 26.0, 240.0, 28.0),
            TextStyle::new(20.5, true, pal.ink, 0),
        );

        let pinned = crate::island::is_pinned();
        let top_utils = [
            (
                RectF::new(WIN_W - 294.0, 19.0, 82.0, 28.0),
                if pinned { "Unpin" } else { "Pin Island" },
                Ctrl::TogglePin,
                pinned,
            ),
            (
                RectF::new(WIN_W - 206.0, 19.0, 78.0, 28.0),
                "Readings",
                Ctrl::ShowReadings,
                false,
            ),
            (
                RectF::new(WIN_W - 122.0, 19.0, 68.0, 28.0),
                "Reset",
                Ctrl::ResetDefaults,
                false,
            ),
        ];
        for (r, lbl, ctrl, active) in top_utils {
            let dr = self.draw_quartz_control(surf, &pal, r, 14.0, ctrl, active);
            let (ht, _) = self.interaction_state(ctrl);
            let fg = if active {
                pal.ink
            } else {
                mix_color(pal.ink_soft, pal.ink, ht)
            };
            surf.draw_text(lbl, dr, TextStyle::new(10.6, active, fg, 1));
            ctrls.push((r, ctrl));
        }

        let close_btn = RectF::new(WIN_W - 46.0, 19.0, 28.0, 28.0);
        let (hc, _) = self.interaction_state(Ctrl::Close);
        if hc > 0.02 {
            surf.fill_round(close_btn, 14.0, with_alpha(pal.accent_red, 0.18 * hc));
        }
        surf.draw_text(
            "✕",
            close_btn,
            TextStyle::new(12.0, false, mix_color(pal.muted, pal.accent_red, hc), 1),
        );
        ctrls.push((close_btn, Ctrl::Close));

        self.draw_bottom_nav(surf, &pal, s.glass_style.opacity_mul());

        self.controls = ctrls;
        self.present_surface();
    }

    #[allow(clippy::too_many_arguments)]
    fn render_general_tab(
        &self,
        surf: &Surface<'_>,
        pal: &Palette,
        s: &settings::AppSettings,
        cx: f32,
        mut y: f32,
        cw: f32,
        ctrls: &mut Vec<(RectF, Ctrl)>,
    ) -> f32 {
        self.draw_section_hdr(surf, pal, cx, y, cw, "APPEARANCE");
        y += 18.0;

        self.draw_choice_row(
            surf,
            pal,
            cx,
            y,
            cw,
            "Theme",
            "Match Windows light or dark mode",
            true,
            186.0,
            &[
                (
                    "Auto",
                    s.theme == ThemeMode::Auto,
                    Ctrl::Theme(ThemeMode::Auto),
                ),
                (
                    "Dark",
                    s.theme == ThemeMode::Dark,
                    Ctrl::Theme(ThemeMode::Dark),
                ),
                (
                    "Light",
                    s.theme == ThemeMode::Light,
                    Ctrl::Theme(ThemeMode::Light),
                ),
            ],
            ctrls,
        );
        y += 48.0;
        self.draw_hairline(surf, pal, cx, y - 2.0, cw);

        self.draw_choice_row(
            surf,
            pal,
            cx,
            y,
            cw,
            "Glass Finish",
            "Backdrop diffusion and translucency",
            true,
            210.0,
            &[
                (
                    "Frosted",
                    s.glass_style == GlassStyle::Frosted,
                    Ctrl::GlassStyle(GlassStyle::Frosted),
                ),
                (
                    "Clear",
                    s.glass_style == GlassStyle::Clear,
                    Ctrl::GlassStyle(GlassStyle::Clear),
                ),
                (
                    "Solid",
                    s.glass_style == GlassStyle::Solid,
                    Ctrl::GlassStyle(GlassStyle::Solid),
                ),
            ],
            ctrls,
        );
        y += 48.0;
        self.draw_hairline(surf, pal, cx, y - 2.0, cw);

        self.draw_choice_row(
            surf,
            pal,
            cx,
            y,
            cw,
            "Motion Feel",
            "Spring physics across controls and pills",
            true,
            206.0,
            &[
                (
                    "Fluid",
                    s.motion_speed == MotionSpeed::Fluid,
                    Ctrl::MotionSpeed(MotionSpeed::Fluid),
                ),
                (
                    "Snappy",
                    s.motion_speed == MotionSpeed::Snappy,
                    Ctrl::MotionSpeed(MotionSpeed::Snappy),
                ),
                (
                    "Gentle",
                    s.motion_speed == MotionSpeed::Gentle,
                    Ctrl::MotionSpeed(MotionSpeed::Gentle),
                ),
            ],
            ctrls,
        );
        y += 48.0;
        self.draw_hairline(surf, pal, cx, y - 2.0, cw);

        self.draw_choice_row(
            surf,
            pal,
            cx,
            y,
            cw,
            "Temperature Unit",
            "Thermal scale across Island and readings",
            true,
            108.0,
            &[
                ("°C", !s.temp_fahrenheit, Ctrl::TempUnit(false)),
                ("°F", s.temp_fahrenheit, Ctrl::TempUnit(true)),
            ],
            ctrls,
        );
        y += 58.0;

        self.draw_section_hdr(surf, pal, cx, y, cw, "WORKSPACE & SYSTEM");
        y += 18.0;

        self.draw_toggle_row(
            surf,
            pal,
            cx,
            y,
            cw,
            "Dynamic Island",
            "Floating Liquid Glass capsule at the top of your primary display",
            s.island_enabled,
            true,
            Ctrl::ToggleIsland,
            ctrls,
        );
        y += 48.0;
        self.draw_hairline(surf, pal, cx, y - 2.0, cw);

        self.draw_toggle_row(
            surf,
            pal,
            cx,
            y,
            cw,
            "Launch at Windows Startup",
            "Automatically start MoLap when you sign in",
            self.startup_on,
            true,
            Ctrl::ToggleStartup,
            ctrls,
        );
        y += 50.0;
        self.draw_hairline(surf, pal, cx, y - 2.0, cw);

        let card_r = RectF::new(cx, y + 2.0, cw, 50.0);
        surf.draw_text(
            "Thermal Sensors",
            RectF::new(card_r.x + 6.0, card_r.y + 5.0, cw - 144.0, 18.0),
            TextStyle::new(12.0, true, pal.ink, 0),
        );
        surf.draw_text(
            if self.needs_temp {
                "CPU & iGPU probes require PawnIO and Administrator mode"
            } else {
                "CPU & iGPU thermal probes active"
            },
            RectF::new(card_r.x + 6.0, card_r.y + 23.0, cw - 144.0, 16.0),
            TextStyle::new(10.2, false, pal.muted, 0),
        );
        let action_r = RectF::new(card_r.x + card_r.w - 118.0, card_r.y + 10.0, 114.0, 28.0);
        if self.needs_temp {
            let (ht, pt) = self.interaction_state(Ctrl::TriggerPawnIo);
            let dr = Surface::deform_gel_rect(action_r, ht * 0.55, pt * 0.7);
            let dr_r = (14.0 + (dr.h - action_r.h) * 0.5).max(2.0);
            surf.fill_round(dr, dr_r, pal.cta_bg);
            surf.draw_text(
                "Enable Sensors",
                dr,
                TextStyle::new(10.8, true, pal.cta_fg, 1),
            );
            ctrls.push((action_r, Ctrl::TriggerPawnIo));
        } else {
            surf.fill_round(action_r, 14.0, with_alpha(pal.accent_green, 0.16));
            surf.draw_text(
                "Active",
                action_r,
                TextStyle::new(10.8, true, pal.accent_green, 1),
            );
        }
        y + 54.0
    }

    #[allow(clippy::too_many_arguments)]
    fn render_island_tab(
        &self,
        surf: &Surface<'_>,
        pal: &Palette,
        s: &settings::AppSettings,
        cx: f32,
        mut y: f32,
        cw: f32,
        ctrls: &mut Vec<(RectF, Ctrl)>,
    ) -> f32 {
        self.draw_section_hdr(surf, pal, cx, y, cw, "SIZE & POSITION");
        y += 18.0;

        surf.draw_text(
            "Island Scale",
            RectF::new(cx + 4.0, y, 220.0, 16.0),
            TextStyle::new(10.5, true, pal.muted, 0),
        );
        surf.draw_text(
            &format!("{}%", s.island_scale_pct),
            RectF::new(cx + cw - 90.0, y, 86.0, 16.0),
            TextStyle::new(11.2, true, pal.accent_blue, 2),
        );

        let presets: [u8; 4] = [85, 100, 115, 130];
        let chip_w = 66.0;
        for (i, pct) in presets.into_iter().enumerate() {
            let r = RectF::new(cx + (i as f32) * (chip_w + 6.0), y + 20.0, chip_w, 28.0);
            self.draw_chip(
                surf,
                pal,
                r,
                &format!("{pct}%"),
                s.island_scale_pct == pct,
                true,
                Ctrl::ScalePreset(pct),
                ctrls,
            );
        }

        let minus_r = RectF::new(cx + cw - 114.0, y + 20.0, 30.0, 28.0);
        let plus_r = RectF::new(cx + cw - 30.0, y + 20.0, 30.0, 28.0);
        let val_r = RectF::new(cx + cw - 80.0, y + 20.0, 46.0, 28.0);
        let dmin = self.draw_quartz_control(surf, pal, minus_r, 14.0, Ctrl::ScaleStep(-5), false);
        surf.draw_text("−", dmin, TextStyle::new(13.0, true, pal.ink, 1));
        ctrls.push((minus_r, Ctrl::ScaleStep(-5)));

        surf.fill_round(val_r, 12.0, pal.card_bg);
        surf.draw_text(
            &format!("{}%", s.island_scale_pct),
            val_r,
            TextStyle::new(10.6, true, pal.ink_soft, 1),
        );

        let dplus = self.draw_quartz_control(surf, pal, plus_r, 14.0, Ctrl::ScaleStep(5), false);
        surf.draw_text("+", dplus, TextStyle::new(13.0, true, pal.ink, 1));
        ctrls.push((plus_r, Ctrl::ScaleStep(5)));
        y += 58.0;
        self.draw_hairline(surf, pal, cx, y - 2.0, cw);

        self.draw_choice_row(
            surf,
            pal,
            cx,
            y,
            cw,
            "Screen Anchor",
            "Horizontal alignment along your display",
            true,
            196.0,
            &[
                (
                    "Left",
                    s.island_anchor == IslandAnchor::Left,
                    Ctrl::IslandAnchor(IslandAnchor::Left),
                ),
                (
                    "Center",
                    s.island_anchor == IslandAnchor::Center,
                    Ctrl::IslandAnchor(IslandAnchor::Center),
                ),
                (
                    "Right",
                    s.island_anchor == IslandAnchor::Right,
                    Ctrl::IslandAnchor(IslandAnchor::Right),
                ),
            ],
            ctrls,
        );
        y += 48.0;
        self.draw_hairline(surf, pal, cx, y - 2.0, cw);

        self.draw_choice_row(
            surf,
            pal,
            cx,
            y,
            cw,
            "Capsule Height",
            "Fit for Windows 11 Standard or 26H2 Small taskbar",
            true,
            224.0,
            &[
                (
                    "Auto",
                    s.taskbar_fit == TaskbarFit::Auto,
                    Ctrl::TaskbarFit(TaskbarFit::Auto),
                ),
                (
                    "Standard",
                    s.taskbar_fit == TaskbarFit::Standard,
                    Ctrl::TaskbarFit(TaskbarFit::Standard),
                ),
                (
                    "Small 26H2",
                    s.taskbar_fit == TaskbarFit::Small,
                    Ctrl::TaskbarFit(TaskbarFit::Small),
                ),
            ],
            ctrls,
        );
        y += 52.0;

        self.draw_slider_row(
            surf,
            pal,
            cx,
            y,
            cw,
            "Horizontal Offset X",
            s.island_offset_x,
            -400,
            400,
            0,
            SliderKind::OffsetX,
            Ctrl::OffsetXStep(-20),
            Ctrl::OffsetXStep(20),
            s.island_offset_x != 0 || s.island_offset_y != 0,
            ctrls,
        );
        y += 50.0;

        self.draw_slider_row(
            surf,
            pal,
            cx,
            y,
            cw,
            "Vertical Offset Y",
            s.island_offset_y,
            -8,
            48,
            0,
            SliderKind::OffsetY,
            Ctrl::OffsetYStep(-2),
            Ctrl::OffsetYStep(2),
            false,
            ctrls,
        );
        y += 56.0;

        self.draw_section_hdr(surf, pal, cx, y, cw, "BEHAVIOR & ALERTS");
        y += 18.0;

        self.draw_toggle_row(
            surf,
            pal,
            cx,
            y,
            cw,
            "Pin Expanded",
            "Keep the full Dynamic Island panel pinned open on screen",
            crate::island::is_pinned(),
            true,
            Ctrl::TogglePin,
            ctrls,
        );
        y += 46.0;
        self.draw_hairline(surf, pal, cx, y - 2.0, cw);

        self.draw_toggle_row(
            surf,
            pal,
            cx,
            y,
            cw,
            "Expand on Hover",
            "Automatically reveal the full Dynamic Island panel on pointer hover",
            s.expand_on_hover,
            true,
            Ctrl::ToggleExpandHover,
            ctrls,
        );
        y += 46.0;
        self.draw_hairline(surf, pal, cx, y - 2.0, cw);

        self.draw_toggle_row(
            surf,
            pal,
            cx,
            y,
            cw,
            "Hide in Fullscreen",
            "Pause and tuck the capsule away when an app or game enters fullscreen",
            s.hide_fullscreen,
            true,
            Ctrl::ToggleHideFullscreen,
            ctrls,
        );
        y += 46.0;
        self.draw_hairline(surf, pal, cx, y - 2.0, cw);

        self.draw_toggle_row(
            surf,
            pal,
            cx,
            y,
            cw,
            "Volume HUD",
            "Display compact Liquid Glass volume feedback on the Island",
            s.volume_hud_enabled,
            true,
            Ctrl::ToggleVolumeHud,
            ctrls,
        );
        y += 48.0;
        self.draw_hairline(surf, pal, cx, y - 2.0, cw);

        let alert_row = RectF::new(cx, y, cw - 72.0, 44.0);
        let (ha, _) = self.interaction_state(Ctrl::ToggleAlerts);
        if ha > 0.02 {
            surf.fill_round(alert_row, 11.0, with_alpha(pal.card_hover, ha));
        }
        surf.draw_text(
            "System Event Toasts",
            RectF::new(
                alert_row.x + 6.0,
                alert_row.y + 4.0,
                alert_row.w - 60.0,
                18.0,
            ),
            TextStyle::new(12.0, true, pal.ink, 0),
        );
        surf.draw_text(
            "Compact hardware, audio, and power alerts",
            RectF::new(
                alert_row.x + 6.0,
                alert_row.y + 22.0,
                alert_row.w - 60.0,
                16.0,
            ),
            TextStyle::new(10.2, false, pal.muted, 0),
        );
        let sw = RectF::new(
            alert_row.x + alert_row.w - 48.0,
            alert_row.y + 11.0,
            38.0,
            22.0,
        );
        self.draw_quartz_switch(
            surf,
            pal,
            sw,
            s.alerts_enabled,
            pal.accent_blue,
            1.0,
            Ctrl::ToggleAlerts,
        );
        ctrls.push((alert_row, Ctrl::ToggleAlerts));

        let test_btn = RectF::new(cx + cw - 62.0, y + 6.0, 62.0, 32.0);
        let dtest = self.draw_quartz_control(surf, pal, test_btn, 16.0, Ctrl::TestAlert, false);
        surf.draw_text("Test", dtest, TextStyle::new(11.0, true, pal.ink, 1));
        ctrls.push((test_btn, Ctrl::TestAlert));
        y += 46.0;

        let toast_chips = [
            ("Power", s.toast_power, Ctrl::ToggleToastPower),
            ("Audio", s.toast_audio, Ctrl::ToggleToastAudio),
            ("Devices", s.toast_devices, Ctrl::ToggleToastDevices),
        ];
        let toast_chip_w = (cw - 16.0) / 3.0;
        for (i, (lbl, on, ctrl)) in toast_chips.into_iter().enumerate() {
            let r = RectF::new(
                cx + (i as f32) * (toast_chip_w + 8.0),
                y,
                toast_chip_w,
                26.0,
            );
            self.draw_chip(surf, pal, r, lbl, on, s.alerts_enabled, ctrl, ctrls);
        }
        y += 34.0;
        self.draw_hairline(surf, pal, cx, y - 2.0, cw);

        self.draw_toggle_row(
            surf,
            pal,
            cx,
            y,
            cw,
            "High-CPU Process Alert",
            "Surface runaway background processes on the System page",
            s.cpu_hog_alert,
            true,
            Ctrl::ToggleCpuHog,
            ctrls,
        );
        y += 46.0;
        self.draw_hairline(surf, pal, cx, y - 2.0, cw);

        self.draw_toggle_row(
            surf,
            pal,
            cx,
            y,
            cw,
            "Camera & Mic Privacy Dots",
            "Show subtle live indicator dots when camera or microphone is active",
            s.privacy_dots,
            true,
            Ctrl::TogglePrivacyDots,
            ctrls,
        );
        y += 54.0;

        self.draw_section_hdr(surf, pal, cx, y, cw, "IDLE & MEDIA");
        y += 18.0;

        self.draw_choice_row(
            surf,
            pal,
            cx,
            y,
            cw,
            "Idle Pill Display",
            "Default resting readout",
            true,
            248.0,
            &[
                (
                    "Smart",
                    s.idle_mode == IdleMode::Smart,
                    Ctrl::IdleMode(IdleMode::Smart),
                ),
                (
                    "Stats",
                    s.idle_mode == IdleMode::Telemetry,
                    Ctrl::IdleMode(IdleMode::Telemetry),
                ),
                (
                    "Tasks",
                    s.idle_mode == IdleMode::Productivity,
                    Ctrl::IdleMode(IdleMode::Productivity),
                ),
                (
                    "Minimal",
                    s.idle_mode == IdleMode::Minimal,
                    Ctrl::IdleMode(IdleMode::Minimal),
                ),
            ],
            ctrls,
        );
        y += 48.0;
        self.draw_hairline(surf, pal, cx, y - 2.0, cw);

        self.draw_toggle_row(
            surf,
            pal,
            cx,
            y,
            cw,
            "Synced Lyrics (LRCLIB)",
            "Fetch and synchronize lyrics for the active track",
            s.lyrics_enabled,
            true,
            Ctrl::ToggleLyrics,
            ctrls,
        );
        y += 46.0;
        self.draw_hairline(surf, pal, cx, y - 2.0, cw);

        self.draw_toggle_row(
            surf,
            pal,
            cx,
            y,
            cw,
            "Show Lyrics in Compact Pill",
            "Display live synced lines while collapsed",
            s.lyrics_in_pill,
            s.lyrics_enabled,
            Ctrl::ToggleLyricsPill,
            ctrls,
        );
        y += 50.0;

        surf.draw_text(
            "VISIBLE PAGES",
            RectF::new(cx + 4.0, y, cw, 14.0),
            TextStyle::new(9.4, true, pal.muted, 0),
        );
        y += 18.0;
        let pages = [
            ("System", s.page_system),
            ("Media", s.page_media),
            ("Shelf", s.page_shelf),
            ("Hydrate", s.page_water),
            ("Focus", s.page_pomodoro),
            ("Tasks", s.page_todo),
        ];
        let chip_w = (cw - 16.0) / 3.0;
        for (i, (lbl, on)) in pages.into_iter().enumerate() {
            let col = (i % 3) as f32;
            let row = (i / 3) as f32;
            let r = RectF::new(cx + col * (chip_w + 8.0), y + row * 30.0, chip_w, 25.0);
            self.draw_chip(surf, pal, r, lbl, on, true, Ctrl::TogglePage(i), ctrls);
        }
        y + 64.0
    }

    #[allow(clippy::too_many_arguments)]
    fn render_plugins_tab(
        &self,
        surf: &Surface<'_>,
        pal: &Palette,
        s: &settings::AppSettings,
        cx: f32,
        mut y: f32,
        cw: f32,
        ctrls: &mut Vec<(RectF, Ctrl)>,
    ) -> f32 {
        let entries = &self.cached_plugins;
        let enabled_cnt = entries
            .iter()
            .filter(|e| s.is_plugin_enabled(&e.manifest.id))
            .count();

        surf.draw_text(
            &format!(
                "{} discovered · {} active (all plugins start disabled for 0% idle cost)",
                entries.len(),
                enabled_cnt
            ),
            RectF::new(cx + 2.0, y, cw, 18.0),
            TextStyle::new(11.0, false, pal.muted, 0),
        );
        y += 24.0;

        let folder_btn = RectF::new(cx, y, 146.0, 28.0);
        let reload_btn = RectF::new(cx + 154.0, y, 108.0, 28.0);
        for (rect, lbl, ctrl) in [
            (folder_btn, "Open Plugins Folder", Ctrl::OpenPluginsDir),
            (reload_btn, "Reload List", Ctrl::ReloadPlugins),
        ] {
            let dr = self.draw_quartz_control(surf, pal, rect, 14.0, ctrl, false);
            surf.draw_text(lbl, dr, TextStyle::new(11.0, true, pal.ink, 1));
            ctrls.push((rect, ctrl));
        }
        y += 38.0;

        if entries.is_empty() {
            surf.draw_text(
                "No plugins installed",
                RectF::new(cx, y + 66.0, cw, 24.0),
                TextStyle::new(14.5, true, pal.ink, 1),
            );
            surf.draw_text(
                "Drop a plugin folder containing plugin.ini into ./plugins or %APPDATA%\\MoLap\\plugins",
                RectF::new(cx + 24.0, y + 96.0, cw - 48.0, 44.0),
                TextStyle::new(11.2, false, pal.muted, 1),
            );
            return y + 150.0;
        }

        for (idx, entry) in entries.iter().enumerate() {
            if idx > 0 {
                self.draw_hairline(surf, pal, cx, y - 3.0, cw);
            }
            let row = RectF::new(cx, y, cw, 60.0);
            let on = s.is_plugin_enabled(&entry.manifest.id);
            let (ht, _) = self.interaction_state(Ctrl::TogglePlugin(idx));
            if ht > 0.02 || on {
                surf.fill_round(
                    row,
                    12.0,
                    with_alpha(pal.card_hover, (if on { 0.45_f32 } else { 0.0 }).max(ht)),
                );
            }

            let title_str = if entry.manifest.version.is_empty() {
                entry.manifest.name.clone()
            } else {
                format!("{} · v{}", entry.manifest.name, entry.manifest.version)
            };
            surf.draw_text(
                &title_str,
                RectF::new(row.x + 10.0, row.y + 6.0, row.w - 80.0, 20.0),
                TextStyle::new(12.5, true, pal.ink, 0),
            );
            let desc = if entry.manifest.description.is_empty() {
                format!(
                    "Runs every {}s · by {}",
                    entry.manifest.interval_secs, entry.manifest.author
                )
            } else {
                entry.manifest.description.clone()
            };
            surf.draw_text(
                &desc,
                RectF::new(row.x + 10.0, row.y + 25.0, row.w - 80.0, 16.0),
                TextStyle::new(10.8, false, pal.muted, 0),
            );
            surf.draw_text(
                &format!(
                    "id: {} · {}s interval",
                    entry.manifest.id, entry.manifest.interval_secs
                ),
                RectF::new(row.x + 10.0, row.y + 41.0, row.w - 80.0, 14.0),
                TextStyle::new(9.6, false, pal.muted, 0),
            );

            let toggle_r = RectF::new(row.x + row.w - 50.0, row.y + 18.0, 42.0, 24.0);
            self.draw_quartz_switch(
                surf,
                pal,
                toggle_r,
                on,
                pal.accent_blue,
                1.0,
                Ctrl::TogglePlugin(idx),
            );
            ctrls.push((toggle_r, Ctrl::TogglePlugin(idx)));
            y += 66.0;
        }
        y
    }

    fn draw_bottom_nav(&self, surf: &Surface<'_>, pal: &Palette, opacity_mul: f32) {
        surf.draw_quartz_panel_styled(NAV_BAR, 25.0, pal.light, opacity_mul, 1.0);

        let (center, half_w, drop_shown, velocity, nearest_idx) = self.nav.state();
        let active_idx = if self.nav.pressed {
            nearest_idx
        } else {
            self.tab as usize
        };

        // Single persistent Liquid Glass droplet lens drawn BEFORE icons/labels
        surf.draw_quartz_droplet(
            NAV_BAR,
            center,
            [(half_w - 4.0).max(28.0), 21.0],
            21.0,
            drop_shown,
            velocity,
            pal.light,
            1.0,
        );

        const LABELS: [&str; 3] = ["General", "Island", "Plugins"];
        for (i, ((slot_cx, slot_w), label)) in NAV_SLOTS.iter().zip(LABELS).enumerate() {
            let selected = i == active_idx;
            let fg = if selected { pal.ink } else { pal.ink_soft };
            draw_nav_icon(surf, i, *slot_cx, NAV_BAR.y + 16.0, fg);
            let lr = RectF::new(*slot_cx - *slot_w * 0.5, NAV_BAR.y + 26.0, *slot_w, 15.0);
            surf.draw_text(label, lr, TextStyle::new(10.6, selected, fg, 1));
            if selected {
                surf.fill_circle(*slot_cx, NAV_BAR.y + 44.5, 1.5, fg);
            }
        }
    }

    fn draw_hairline(&self, surf: &Surface<'_>, pal: &Palette, x: f32, y: f32, w: f32) {
        surf.draw_line(
            (x + 4.0, y),
            (x + w - 4.0, y),
            0.75,
            with_alpha(pal.shell_rim, 0.65),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_slider_row(
        &self,
        surf: &Surface<'_>,
        pal: &Palette,
        x: f32,
        y: f32,
        w: f32,
        label: &str,
        value: i16,
        min_v: i16,
        max_v: i16,
        zero_v: i16,
        kind: SliderKind,
        minus_ctrl: Ctrl,
        plus_ctrl: Ctrl,
        show_reset: bool,
        ctrls: &mut Vec<(RectF, Ctrl)>,
    ) {
        surf.draw_text(
            label,
            RectF::new(x + 4.0, y, 220.0, 16.0),
            TextStyle::new(10.5, true, pal.muted, 0),
        );
        let val_str = if value > 0 {
            format!("+{value} px")
        } else {
            format!("{value} px")
        };
        surf.draw_text(
            &val_str,
            RectF::new(x + w - 96.0, y, 92.0, 16.0),
            TextStyle::new(10.8, true, pal.ink, 2),
        );
        if show_reset {
            let rst_r = RectF::new(x + w - 156.0, y - 1.0, 52.0, 18.0);
            let dr = self.draw_quartz_control(surf, pal, rst_r, 9.0, Ctrl::ResetOffsets, false);
            surf.draw_text("Center", dr, TextStyle::new(9.5, true, pal.accent_blue, 1));
            ctrls.push((rst_r, Ctrl::ResetOffsets));
        }

        let minus_r = RectF::new(x, y + 20.0, 26.0, 24.0);
        let plus_r = RectF::new(x + w - 26.0, y + 20.0, 26.0, 24.0);
        let track_hit = slider_track_rect(x, y, w);

        let dmin = self.draw_quartz_control(surf, pal, minus_r, 12.0, minus_ctrl, false);
        surf.draw_text("−", dmin, TextStyle::new(12.0, true, pal.ink, 1));
        ctrls.push((minus_r, minus_ctrl));

        let dplus = self.draw_quartz_control(surf, pal, plus_r, 12.0, plus_ctrl, false);
        surf.draw_text("+", dplus, TextStyle::new(12.0, true, pal.ink, 1));
        ctrls.push((plus_r, plus_ctrl));

        let bar = RectF::new(
            track_hit.x + 8.0,
            track_hit.y + 9.0,
            track_hit.w - 16.0,
            6.0,
        );
        surf.fill_round(bar, 3.0, pal.track);

        let span = (max_v - min_v).max(1) as f32;
        let t = ((value - min_v) as f32 / span).clamp(0.0, 1.0);
        let tz = ((zero_v - min_v) as f32 / span).clamp(0.0, 1.0);
        let thumb_x = bar.x + bar.w * t;
        let zero_x = bar.x + bar.w * tz;
        let fill_x0 = thumb_x.min(zero_x);
        let fill_w = (thumb_x - zero_x).abs();
        if fill_w > 1.0 {
            surf.fill_round(
                RectF::new(fill_x0, bar.y, fill_w, bar.h),
                3.0,
                pal.accent_blue,
            );
        }
        surf.fill_circle(
            zero_x,
            bar.y + bar.h * 0.5,
            2.0,
            with_alpha(pal.muted, 0.55),
        );

        let (ht, pt) = self.interaction_state(Ctrl::Slider(kind));
        let thumb_r = Surface::deform_gel_rect(
            RectF::new(thumb_x - 8.5, track_hit.y + 3.5, 17.0, 17.0),
            ht * 0.45,
            pt * 0.55,
        );
        self.draw_pearl_knob(surf, pal, thumb_r, 1.0);
        ctrls.push((track_hit, Ctrl::Slider(kind)));
    }

    fn draw_section_hdr(
        &self,
        surf: &Surface<'_>,
        pal: &Palette,
        x: f32,
        y: f32,
        w: f32,
        title: &str,
    ) {
        surf.draw_text(
            title,
            RectF::new(x + 4.0, y, w, 14.0),
            TextStyle::new(9.4, true, pal.muted, 0),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_choice_row(
        &self,
        surf: &Surface<'_>,
        pal: &Palette,
        x: f32,
        y: f32,
        w: f32,
        title: &str,
        sub: &str,
        enabled: bool,
        track_w: f32,
        items: &[(&str, bool, Ctrl)],
        ctrls: &mut Vec<(RectF, Ctrl)>,
    ) {
        let row = RectF::new(x, y, w, 46.0);
        let a = if enabled { 1.0 } else { 0.45 };
        let label_w = (w - track_w - 14.0).max(120.0);
        surf.draw_text(
            title,
            RectF::new(row.x + 6.0, row.y + 5.0, label_w, 18.0),
            TextStyle::new(12.0, true, with_alpha(pal.ink, a), 0),
        );
        surf.draw_text(
            sub,
            RectF::new(row.x + 6.0, row.y + 23.0, label_w, 16.0),
            TextStyle::new(10.2, false, with_alpha(pal.muted, a), 0),
        );

        let track_r = RectF::new(row.x + row.w - track_w, row.y + 8.0, track_w, 30.0);
        surf.fill_round(track_r, 15.0, with_alpha(pal.card_bg, a * 0.95));
        surf.stroke_round(
            track_r,
            15.0,
            0.8,
            with_alpha(pal.shell_rim, a * 0.55),
            false,
        );

        let seg_w = (track_r.w - 6.0) / (items.len().max(1) as f32);
        for (i, &(text, active, ctrl)) in items.iter().enumerate() {
            let r = RectF::new(
                track_r.x + 3.0 + (i as f32) * seg_w,
                track_r.y + 3.0,
                seg_w,
                track_r.h - 6.0,
            );
            let (ht, pt) = if enabled {
                self.interaction_state(ctrl)
            } else {
                (0.0, 0.0)
            };
            let draw_r = if active {
                surf.draw_quartz_pill(
                    r,
                    12.0,
                    with_alpha(pal.seg_active_bg, a),
                    ht,
                    pt,
                    true,
                    pal.light,
                    a,
                    None,
                )
            } else {
                if ht > 0.02 {
                    let dr = Surface::deform_gel_rect(r, ht * 0.45, pt * 0.5);
                    surf.fill_round(dr, 12.0, with_alpha(pal.card_hover, ht * a));
                }
                r
            };
            surf.draw_text(
                text,
                draw_r,
                TextStyle::new(
                    10.8,
                    active,
                    with_alpha(
                        if active {
                            pal.ink
                        } else {
                            mix_color(pal.muted, pal.ink, ht)
                        },
                        a,
                    ),
                    1,
                ),
            );
            if enabled {
                ctrls.push((r, ctrl));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_toggle_row(
        &self,
        surf: &Surface<'_>,
        pal: &Palette,
        x: f32,
        y: f32,
        w: f32,
        title: &str,
        sub: &str,
        on: bool,
        enabled: bool,
        ctrl: Ctrl,
        ctrls: &mut Vec<(RectF, Ctrl)>,
    ) {
        let row = RectF::new(x, y, w, 44.0);
        let a = if enabled { 1.0 } else { 0.45 };
        let (ht, _) = if enabled {
            self.interaction_state(ctrl)
        } else {
            (0.0, 0.0)
        };
        if ht > 0.02 {
            surf.fill_round(row, 11.0, with_alpha(pal.card_hover, ht * a));
        }
        surf.draw_text(
            title,
            RectF::new(row.x + 6.0, row.y + 4.0, row.w - 64.0, 18.0),
            TextStyle::new(12.2, true, with_alpha(pal.ink, a), 0),
        );
        surf.draw_text(
            sub,
            RectF::new(row.x + 6.0, row.y + 22.0, row.w - 64.0, 16.0),
            TextStyle::new(10.3, false, with_alpha(pal.muted, a), 0),
        );
        let sw = RectF::new(row.x + row.w - 44.0, row.y + 11.0, 38.0, 22.0);
        self.draw_quartz_switch(surf, pal, sw, on, pal.accent_blue, a, ctrl);
        if enabled {
            ctrls.push((row, ctrl));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_chip(
        &self,
        surf: &Surface<'_>,
        pal: &Palette,
        r: RectF,
        label: &str,
        on: bool,
        enabled: bool,
        ctrl: Ctrl,
        ctrls: &mut Vec<(RectF, Ctrl)>,
    ) {
        let a = if enabled { 1.0 } else { 0.45 };
        let (ht, pt) = if enabled {
            self.interaction_state(ctrl)
        } else {
            (0.0, 0.0)
        };
        let draw_r = if on {
            surf.draw_quartz_pill(
                r,
                r.h * 0.5,
                with_alpha(pal.seg_active_bg, a),
                ht,
                pt,
                true,
                pal.light,
                a,
                None,
            )
        } else {
            let dr = Surface::deform_gel_rect(r, ht * 0.45, pt * 0.5);
            surf.fill_round(
                dr,
                dr.h * 0.5,
                mix_color(with_alpha(pal.card_bg, a), pal.card_hover, ht),
            );
            dr
        };
        let dot_col = with_alpha(if on { pal.ink } else { pal.track }, a);
        surf.fill_circle(draw_r.x + 11.0, draw_r.y + draw_r.h * 0.5, 2.6, dot_col);
        surf.draw_text(
            label,
            RectF::new(draw_r.x + 17.0, draw_r.y, draw_r.w - 21.0, draw_r.h),
            TextStyle::new(
                10.6,
                on,
                with_alpha(if on { pal.ink } else { pal.muted }, a),
                0,
            ),
        );
        if enabled {
            ctrls.push((r, ctrl));
        }
    }

    fn hit_test_rect(&self, px: f32, py: f32) -> Option<(RectF, Ctrl)> {
        self.controls
            .iter()
            .rev()
            .find(|(r, _)| r.contains(px, py))
            .copied()
    }

    fn hit_test(&self, px: f32, py: f32) -> Option<Ctrl> {
        self.hit_test_rect(px, py).map(|(_, c)| c)
    }

    fn apply_slider(&mut self, kind: SliderKind, rect: RectF, px: f32) {
        let bar_x = rect.x + 8.0;
        let bar_w = (rect.w - 16.0).max(1.0);
        let t = ((px - bar_x) / bar_w).clamp(0.0, 1.0);
        let cur = settings::get();
        match kind {
            SliderKind::OffsetX => {
                let raw = -400.0 + t * 800.0;
                let next = ((raw / 10.0).round() as i16 * 10).clamp(-400, 400);
                if next != cur.island_offset_x {
                    settings::update_live(|s| s.island_offset_x = next);
                }
            }
            SliderKind::OffsetY => {
                let raw = -8.0 + t * 56.0;
                let next = (raw.round() as i16).clamp(-8, 48);
                if next != cur.island_offset_y {
                    settings::update_live(|s| s.island_offset_y = next);
                }
            }
        }
    }

    fn on_click(&mut self, ctrl: Ctrl) {
        match ctrl {
            Ctrl::Close => self.hide(),
            Ctrl::TogglePin => {
                crate::island::toggle_pin();
                self.content_dirty = true;
                self.render();
            }
            Ctrl::ShowReadings => crate::ui::show_readings(),
            Ctrl::ResetDefaults => settings::reset_defaults(),
            Ctrl::Theme(mode) => settings::update(|s| s.theme = mode),
            Ctrl::GlassStyle(style) => settings::update(|s| s.glass_style = style),
            Ctrl::MotionSpeed(speed) => settings::update(|s| s.motion_speed = speed),
            Ctrl::TempUnit(f) => settings::update(|s| s.temp_fahrenheit = f),
            Ctrl::ToggleIsland => settings::update(|s| s.island_enabled = !s.island_enabled),
            Ctrl::ToggleStartup => {
                let cur = self.startup_on;
                if let Err(err) = crate::startup::set_enabled(!cur) {
                    unsafe {
                        MessageBoxW(
                            self.hwnd,
                            wide(&format!("Could not update Windows startup: {err}")).as_ptr(),
                            wide("MoLap").as_ptr(),
                            0x30,
                        );
                    }
                }
                self.refresh_system_flags();
                self.render();
            }
            Ctrl::TriggerPawnIo => {
                if self.needs_temp {
                    crate::ui::trigger_pawnio();
                    self.refresh_system_flags();
                    self.render();
                }
            }
            Ctrl::ScalePreset(pct) => {
                settings::update(|s| s.island_scale_pct = pct.clamp(80, 140));
            }
            Ctrl::ScaleStep(delta) => settings::update(|s| {
                let next = (s.island_scale_pct as i16 + delta as i16).clamp(80, 140) as u8;
                s.island_scale_pct = next;
            }),
            Ctrl::IslandAnchor(anchor) => settings::update(|s| s.island_anchor = anchor),
            Ctrl::TaskbarFit(fit) => settings::update(|s| s.taskbar_fit = fit),
            Ctrl::OffsetXStep(delta) => settings::update(|s| {
                s.island_offset_x = (s.island_offset_x + delta).clamp(-400, 400);
            }),
            Ctrl::OffsetYStep(delta) => settings::update(|s| {
                s.island_offset_y = (s.island_offset_y + delta).clamp(-8, 48);
            }),
            Ctrl::ResetOffsets => settings::update(|s| {
                s.island_offset_x = 0;
                s.island_offset_y = 0;
            }),
            Ctrl::Slider(_) => {}
            Ctrl::ToggleExpandHover => {
                settings::update(|s| s.expand_on_hover = !s.expand_on_hover);
            }
            Ctrl::ToggleHideFullscreen => {
                settings::update(|s| s.hide_fullscreen = !s.hide_fullscreen);
            }
            Ctrl::ToggleVolumeHud => {
                settings::update(|s| s.volume_hud_enabled = !s.volume_hud_enabled);
            }
            Ctrl::IdleMode(mode) => settings::update(|s| s.idle_mode = mode),
            Ctrl::ToggleAlerts => settings::update(|s| s.alerts_enabled = !s.alerts_enabled),
            Ctrl::TestAlert => crate::island::trigger_demo_toast(),
            Ctrl::ToggleToastPower => settings::update(|s| s.toast_power = !s.toast_power),
            Ctrl::ToggleToastAudio => settings::update(|s| s.toast_audio = !s.toast_audio),
            Ctrl::ToggleToastDevices => settings::update(|s| s.toast_devices = !s.toast_devices),
            Ctrl::ToggleCpuHog => settings::update(|s| s.cpu_hog_alert = !s.cpu_hog_alert),
            Ctrl::TogglePrivacyDots => settings::update(|s| s.privacy_dots = !s.privacy_dots),
            Ctrl::ToggleLyrics => settings::update(|s| s.lyrics_enabled = !s.lyrics_enabled),
            Ctrl::ToggleLyricsPill => settings::update(|s| s.lyrics_in_pill = !s.lyrics_in_pill),
            Ctrl::TogglePage(idx) => settings::update(|s| {
                let count = [
                    s.page_system,
                    s.page_media,
                    s.page_shelf,
                    s.page_water,
                    s.page_pomodoro,
                    s.page_todo,
                ]
                .into_iter()
                .filter(|&b| b)
                .count();
                let slot = match idx {
                    0 => &mut s.page_system,
                    1 => &mut s.page_media,
                    2 => &mut s.page_shelf,
                    3 => &mut s.page_water,
                    4 => &mut s.page_pomodoro,
                    _ => &mut s.page_todo,
                };
                if !(*slot && count <= 1) {
                    *slot = !*slot;
                }
            }),
            Ctrl::TogglePlugin(idx) => {
                if let Some(entry) = self.cached_plugins.get(idx) {
                    let id = entry.manifest.id.clone();
                    settings::update(|s| s.toggle_plugin(&id));
                }
            }
            Ctrl::ReloadPlugins => {
                crate::island::reload_plugins();
                self.scroll_y[Tab::Plugins as usize].set(0.0);
                self.refresh_plugins();
                self.render();
            }
            Ctrl::OpenPluginsDir => {
                crate::plugin::open_plugins_folder(self.hwnd);
            }
        }
    }
}

fn draw_nav_icon(surf: &Surface<'_>, idx: usize, cx: f32, cy: f32, col: u32) {
    match idx {
        0 => {
            surf.stroke_circle(cx, cy, 6.2, 1.4, col);
        }
        1 => {
            surf.fill_round(RectF::new(cx - 7.5, cy - 5.0, 15.0, 2.0), 1.0, col);
            surf.fill_round(RectF::new(cx - 5.2, cy - 1.0, 10.4, 2.0), 1.0, col);
            surf.fill_round(RectF::new(cx - 3.0, cy + 3.0, 6.0, 2.0), 1.0, col);
        }
        _ => {
            let d = 6.2f32;
            let pts = [(cx, cy - d), (cx + d, cy), (cx, cy + d), (cx - d, cy)];
            for w in 0..4 {
                surf.draw_line(pts[w], pts[(w + 1) % 4], 1.4, col);
            }
        }
    }
}

unsafe extern "system" fn settings_proc(hwnd: Handle, msg: u32, w: usize, l: isize) -> isize {
    match msg {
        0x0003 => {
            with_win(|win| {
                let mut wr = Rect::default();
                if GetWindowRect(hwnd, &mut wr) != 0 {
                    win.origin = Point {
                        x: wr.left,
                        y: wr.top,
                    };
                }
            });
            return 0;
        }
        0x0010 => {
            with_win(SettingsWindow::hide);
            return 0;
        }
        0x000F => {
            let mut paint = Paint {
                dc: 0,
                erase: 0,
                rect: Rect::default(),
                restore: 0,
                update: 0,
                reserved: [0; 32],
            };
            let hdc = BeginPaint(hwnd, &mut paint);
            with_win(|win| {
                if let Some(surf) = win.surface.as_ref() {
                    surf.blit_to(hdc);
                }
            });
            EndPaint(hwnd, &paint);
            return 0;
        }
        0x0014 => return 1,
        0x0008 => {
            let mut need_flush = false;
            with_win(|win| {
                if win.nav.pressed {
                    win.nav.cancel();
                    ReleaseCapture();
                    win.start_anim();
                }
                if win.dragging_slider.is_some() {
                    win.dragging_slider = None;
                    ReleaseCapture();
                    need_flush = true;
                    win.content_dirty = true;
                    win.start_anim();
                }
            });
            if need_flush {
                settings::flush_save();
            }
        }
        0x0100 => match w {
            0x1B => {
                with_win(SettingsWindow::hide);
                return 0;
            }
            0x31..=0x33 => {
                let idx = w - 0x31;
                with_win(|win| {
                    let next_tab = Tab::from_idx(idx);
                    if win.tab != next_tab {
                        win.tab = next_tab;
                        win.content_dirty = true;
                    }
                    win.hovered = None;
                    win.nav.select(idx, true);
                    win.start_anim();
                    win.render();
                });
                return 0;
            }
            _ => {}
        },
        0x0113 if w == TIMER_ANIM => {
            with_win(SettingsWindow::on_anim_tick);
            return 0;
        }
        0x0200 => {
            with_win(|win| {
                let s = win.scale();
                let px = (l as i16) as f32 / s;
                let py = ((l >> 16) as i16) as f32 / s;
                if let Some((kind, rect)) = win.dragging_slider {
                    win.apply_slider(kind, rect, px);
                    win.start_anim();
                    if win.hand_cursor != 0 {
                        SetCursor(win.hand_cursor);
                    }
                    return;
                }
                if win.nav.pressed {
                    win.nav.move_to([px, py]);
                    win.start_anim();
                    if win.hand_cursor != 0 {
                        SetCursor(win.hand_cursor);
                    }
                    return;
                }
                let over_nav = NAV_BAR.contains(px, py);
                let next = if over_nav { None } else { win.hit_test(px, py) };
                if next != win.hovered {
                    let touches_content = win
                        .hovered
                        .is_some_and(|c| !SettingsWindow::is_header_ctrl(c))
                        || next.is_some_and(|c| !SettingsWindow::is_header_ctrl(c));
                    win.hovered = next;
                    if touches_content {
                        win.content_dirty = true;
                    }
                    win.dirty = true;
                    win.start_anim();
                }
                if (next.is_some() || over_nav) && win.hand_cursor != 0 {
                    SetCursor(win.hand_cursor);
                }
            });
            return 0;
        }
        0x0020 => {
            let hand = with_win(|win| {
                let mut pt = Point::default();
                let over_nav = unsafe { GetCursorPos(&mut pt) } != 0 && {
                    let s = win.scale();
                    let px = (pt.x - win.origin.x) as f32 / s;
                    let py = (pt.y - win.origin.y) as f32 / s;
                    NAV_BAR.contains(px, py)
                };
                if (win.hovered.is_some()
                    || over_nav
                    || win.nav.pressed
                    || win.dragging_slider.is_some())
                    && win.hand_cursor != 0
                {
                    SetCursor(win.hand_cursor);
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false);
            if hand {
                return 1;
            }
        }
        0x020A => {
            let delta = (w >> 16) as i16;
            with_win(|win| {
                if delta != 0 {
                    let idx = win.tab as usize;
                    let max_s = win.max_scroll[idx];
                    if max_s > 0.5 {
                        let step = -(delta as f32 / 120.0) * 52.0;
                        win.scroll_y[idx].target =
                            (win.scroll_y[idx].target + step).clamp(0.0, max_s);
                        win.start_anim();
                    }
                }
            });
            return 0;
        }
        0x0201 => {
            let mut start_drag = false;
            with_win(|win| {
                let s = win.scale();
                let px = (l as i16) as f32 / s;
                let py = ((l >> 16) as i16) as f32 / s;
                if win.nav.press([px, py]) {
                    SetCapture(hwnd);
                    win.start_anim();
                } else if let Some((_, ctrl)) = win.hit_test_rect(px, py) {
                    win.pressed = Some(ctrl);
                    if !SettingsWindow::is_header_ctrl(ctrl) {
                        win.content_dirty = true;
                    }
                    win.press_spring.target = 1.0;
                    win.start_anim();
                    if let Ctrl::Slider(kind) = ctrl {
                        let track = slider_track_rect(26.0, 0.0, WIN_W - 52.0);
                        win.dragging_slider = Some((kind, track));
                        SetCapture(hwnd);
                        win.apply_slider(kind, track, px);
                    } else {
                        win.on_click(ctrl);
                    }
                } else if py <= HEADER_H {
                    start_drag = true;
                }
            });
            if start_drag {
                ReleaseCapture();
                SendMessageW(hwnd, 0x00A1, 2, 0); // WM_NCLBUTTONDOWN, HTCAPTION
            }
            return 0;
        }
        0x0202 => {
            let mut need_flush = false;
            with_win(|win| {
                if win.nav.pressed {
                    if let Some(idx) = win.nav.release() {
                        let next_tab = Tab::from_idx(idx);
                        if win.tab != next_tab {
                            win.tab = next_tab;
                            win.content_dirty = true;
                        }
                        win.hovered = None;
                    }
                    ReleaseCapture();
                    win.start_anim();
                    win.render();
                }
                if win.dragging_slider.is_some() {
                    win.dragging_slider = None;
                    ReleaseCapture();
                    need_flush = true;
                    win.content_dirty = true;
                    win.render();
                }
                if let Some(prev_pressed) = win.pressed.take() {
                    if !SettingsWindow::is_header_ctrl(prev_pressed) {
                        win.content_dirty = true;
                    }
                    win.press_spring.target = 0.0;
                    win.start_anim();
                }
            });
            if need_flush {
                settings::flush_save();
            }
            return 0;
        }
        0x001a | 0x031a => {
            with_win(|win| {
                win.apply_backdrop(settings::get().is_light());
                win.content_dirty = true;
                win.render();
            });
            return 0;
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, w, l)
}

pub fn init(instance: Handle) {
    let Some(api) = gdiplus::shared() else {
        return;
    };
    unsafe {
        let class_name = wide("MoLapSettingsWindow");
        let wc = WindowClass {
            size: size_of::<WindowClass>() as u32,
            style: 0,
            callback: Some(settings_proc),
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
            0x0000_0080, // WS_EX_TOOLWINDOW
            class_name.as_ptr(),
            wide("MoLap Settings").as_ptr(),
            0x8200_0000, // WS_POPUP | WS_CLIPCHILDREN
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
            let mut win = SettingsWindow::new(hwnd, api);
            win.apply_backdrop(settings::get().is_light());
            SETTINGS_WIN.with(|cell| *cell.borrow_mut() = Some(win));
        }
    }
}

pub fn open() {
    with_win(SettingsWindow::show);
}

pub fn shutdown() {
    let hwnd = with_win(|w| w.hwnd).unwrap_or(0);
    SETTINGS_WIN.with(|cell| {
        cell.borrow_mut().take();
    });
    if hwnd != 0 {
        unsafe {
            DestroyWindow(hwnd);
        }
    }
}

pub fn on_theme_change() {
    with_win(|win| {
        win.apply_backdrop(settings::get().is_light());
        win.content_dirty = true;
        win.render();
    });
}

pub fn on_system_flags_changed() {
    with_win(|win| {
        win.refresh_system_flags();
        win.render();
    });
}

pub fn on_settings_changed() {
    with_win(|win| {
        let light = settings::get().is_light();
        if win.last_light != Some(light) {
            win.apply_backdrop(light);
        }
        win.content_dirty = true;
        if win.dragging_slider.is_some() {
            win.start_anim();
        } else {
            win.render();
        }
    });
}
