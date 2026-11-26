use crate::{
    gdiplus::argb,
    island_data::Page,
    win::{reg_u32, HKCU},
};
use std::{
    cell::RefCell,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeMode {
    Auto = 0,
    Dark = 1,
    Light = 2,
}

impl ThemeMode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Dark,
            2 => Self::Light,
            _ => Self::Auto,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IdleMode {
    Smart = 0,
    Telemetry = 1,
    Productivity = 2,
    Minimal = 3,
}

impl IdleMode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Telemetry,
            2 => Self::Productivity,
            3 => Self::Minimal,
            _ => Self::Smart,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskbarFit {
    Auto = 0,
    Small = 1,
    Standard = 2,
}

impl TaskbarFit {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Small,
            2 => Self::Standard,
            _ => Self::Auto,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IslandAnchor {
    Center = 0,
    Left = 1,
    Right = 2,
}

impl IslandAnchor {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Left,
            2 => Self::Right,
            _ => Self::Center,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionSpeed {
    Fluid = 0,
    Snappy = 1,
    Gentle = 2,
}

impl MotionSpeed {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Snappy,
            2 => Self::Gentle,
            _ => Self::Fluid,
        }
    }

    pub fn spring_scale(self) -> f32 {
        match self {
            Self::Fluid => 1.0,
            Self::Snappy => 1.38,
            Self::Gentle => 0.72,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlassStyle {
    Frosted = 0,
    Clear = 1,
    Solid = 2,
}

impl GlassStyle {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Clear,
            2 => Self::Solid,
            _ => Self::Frosted,
        }
    }

    pub fn opacity_mul(self) -> f32 {
        match self {
            Self::Frosted => 1.0,
            Self::Clear => 0.78,
            Self::Solid => 1.12,
        }
    }
}

pub fn format_temp(celsius: i32, fahrenheit: bool) -> String {
    if fahrenheit {
        let f = if celsius >= 0 {
            (celsius * 9 + 2) / 5 + 32
        } else {
            (celsius * 9 - 2) / 5 + 32
        };
        format!("{f}°F")
    } else {
        format!("{celsius}°C")
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AppSettings {
    pub theme: ThemeMode,
    pub island_enabled: bool,
    pub idle_mode: IdleMode,
    pub taskbar_fit: TaskbarFit,
    pub alerts_enabled: bool,
    pub lyrics_enabled: bool,
    pub lyrics_in_pill: bool,
    pub page_system: bool,
    pub page_media: bool,
    pub page_shelf: bool,
    pub page_water: bool,
    pub page_pomodoro: bool,
    pub page_todo: bool,
    pub enabled_plugins: Vec<String>,
    pub island_scale_pct: u8,
    pub island_anchor: IslandAnchor,
    pub island_offset_x: i16,
    pub island_offset_y: i16,
    pub expand_on_hover: bool,
    pub hide_fullscreen: bool,
    pub volume_hud_enabled: bool,
    pub toast_power: bool,
    pub toast_audio: bool,
    pub toast_devices: bool,
    pub cpu_hog_alert: bool,
    pub privacy_dots: bool,
    pub motion_speed: MotionSpeed,
    pub glass_style: GlassStyle,
    pub temp_fahrenheit: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: ThemeMode::Auto,
            island_enabled: true,
            idle_mode: IdleMode::Smart,
            taskbar_fit: TaskbarFit::Auto,
            alerts_enabled: true,
            lyrics_enabled: true,
            lyrics_in_pill: true,
            page_system: true,
            page_media: true,
            page_shelf: true,
            page_water: true,
            page_pomodoro: true,
            page_todo: true,
            enabled_plugins: Vec::new(),
            island_scale_pct: 100,
            island_anchor: IslandAnchor::Center,
            island_offset_x: 0,
            island_offset_y: 0,
            expand_on_hover: true,
            hide_fullscreen: true,
            volume_hud_enabled: true,
            toast_power: true,
            toast_audio: true,
            toast_devices: true,
            cpu_hog_alert: true,
            privacy_dots: true,
            motion_speed: MotionSpeed::Fluid,
            glass_style: GlassStyle::Frosted,
            temp_fahrenheit: false,
        }
    }
}

impl AppSettings {
    pub fn is_plugin_enabled(&self, id: &str) -> bool {
        self.enabled_plugins.iter().any(|p| p == id)
    }

    pub fn toggle_plugin(&mut self, id: &str) {
        if let Some(pos) = self.enabled_plugins.iter().position(|p| p == id) {
            self.enabled_plugins.remove(pos);
        } else {
            self.enabled_plugins.push(id.to_string());
        }
    }
    pub fn system_uses_light_theme() -> bool {
        reg_u32(
            HKCU,
            r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
            "SystemUsesLightTheme",
        ) == Some(1)
    }

    pub fn is_light(&self) -> bool {
        match self.theme {
            ThemeMode::Auto => Self::system_uses_light_theme(),
            ThemeMode::Dark => false,
            ThemeMode::Light => true,
        }
    }

    pub fn enabled_pages(&self) -> Vec<Page> {
        let mut pages = Vec::with_capacity(Page::COUNT);
        if self.page_system {
            pages.push(Page::System);
        }
        if self.page_media {
            pages.push(Page::Media);
        }
        if self.page_shelf {
            pages.push(Page::Shelf);
        }
        if self.page_water {
            pages.push(Page::Water);
        }
        if self.page_pomodoro {
            pages.push(Page::Pomodoro);
        }
        if self.page_todo {
            pages.push(Page::Todo);
        }
        if pages.is_empty() {
            pages.push(Page::Media);
        }
        pages
    }

    pub fn needs_sensors(&self) -> bool {
        self.island_enabled
            && (self.page_system || matches!(self.idle_mode, IdleMode::Smart | IdleMode::Telemetry))
    }

    fn storage_path() -> Option<PathBuf> {
        let appdata = std::env::var_os("APPDATA")?;
        Some(PathBuf::from(appdata).join("MoLap").join("settings.txt"))
    }

    pub fn load() -> Self {
        match Self::storage_path() {
            Some(path) => Self::load_from_path(&path),
            None => Self::default(),
        }
    }

    pub fn clamp_values(&mut self) {
        self.island_scale_pct = self.island_scale_pct.clamp(80, 140);
        self.island_offset_x = self.island_offset_x.clamp(-400, 400);
        self.island_offset_y = self.island_offset_y.clamp(-8, 48);
    }

    pub fn load_from_path(path: &Path) -> Self {
        let mut s = Self::default();
        let Ok(raw) = fs::read_to_string(path) else {
            return s;
        };
        for line in raw.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let b = |val: &str| val != "0";
            let u = |val: &str| val.parse::<u8>().unwrap_or(0);
            let i16_val = |val: &str, default: i16| val.parse::<i16>().unwrap_or(default);
            match k {
                "theme" => s.theme = ThemeMode::from_u8(u(v)),
                "island_enabled" => s.island_enabled = b(v),
                "idle_mode" => s.idle_mode = IdleMode::from_u8(u(v)),
                "taskbar_fit" => s.taskbar_fit = TaskbarFit::from_u8(u(v)),
                "alerts_enabled" => s.alerts_enabled = b(v),
                "lyrics_enabled" => s.lyrics_enabled = b(v),
                "lyrics_in_pill" => s.lyrics_in_pill = b(v),
                "page_system" => s.page_system = b(v),
                "page_media" => s.page_media = b(v),
                "page_shelf" => s.page_shelf = b(v),
                "page_water" => s.page_water = b(v),
                "page_pomodoro" => s.page_pomodoro = b(v),
                "page_todo" => s.page_todo = b(v),
                "enabled_plugins" => {
                    s.enabled_plugins = v
                        .split(',')
                        .map(str::trim)
                        .filter(|id| !id.is_empty())
                        .map(ToOwned::to_owned)
                        .collect();
                }
                "island_scale_pct" => {
                    s.island_scale_pct = v.parse::<u8>().unwrap_or(100);
                }
                "island_anchor" => s.island_anchor = IslandAnchor::from_u8(u(v)),
                "island_offset_x" => s.island_offset_x = i16_val(v, 0),
                "island_offset_y" => s.island_offset_y = i16_val(v, 0),
                "expand_on_hover" => s.expand_on_hover = b(v),
                "hide_fullscreen" => s.hide_fullscreen = b(v),
                "volume_hud_enabled" => s.volume_hud_enabled = b(v),
                "toast_power" => s.toast_power = b(v),
                "toast_audio" => s.toast_audio = b(v),
                "toast_devices" => s.toast_devices = b(v),
                "cpu_hog_alert" => s.cpu_hog_alert = b(v),
                "privacy_dots" => s.privacy_dots = b(v),
                "motion_speed" => s.motion_speed = MotionSpeed::from_u8(u(v)),
                "glass_style" => s.glass_style = GlassStyle::from_u8(u(v)),
                "temp_fahrenheit" => s.temp_fahrenheit = b(v),
                _ => {}
            }
        }
        s.clamp_values();
        s
    }

    pub fn save(&self) {
        if let Some(path) = Self::storage_path() {
            self.save_to_path(&path);
        }
    }

    pub fn save_to_path(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let scale = self.island_scale_pct.clamp(80, 140);
        let ox = self.island_offset_x.clamp(-400, 400);
        let oy = self.island_offset_y.clamp(-8, 48);
        let out = format!(
            "theme={}\n\
             island_enabled={}\n\
             idle_mode={}\n\
             taskbar_fit={}\n\
             alerts_enabled={}\n\
             lyrics_enabled={}\n\
             lyrics_in_pill={}\n\
             page_system={}\n\
             page_media={}\n\
             page_shelf={}\n\
             page_water={}\n\
             page_pomodoro={}\n\
             page_todo={}\n\
             enabled_plugins={}\n\
             island_scale_pct={}\n\
             island_anchor={}\n\
             island_offset_x={}\n\
             island_offset_y={}\n\
             expand_on_hover={}\n\
             hide_fullscreen={}\n\
             volume_hud_enabled={}\n\
             toast_power={}\n\
             toast_audio={}\n\
             toast_devices={}\n\
             cpu_hog_alert={}\n\
             privacy_dots={}\n\
             motion_speed={}\n\
             glass_style={}\n\
             temp_fahrenheit={}\n",
            self.theme as u8,
            u8::from(self.island_enabled),
            self.idle_mode as u8,
            self.taskbar_fit as u8,
            u8::from(self.alerts_enabled),
            u8::from(self.lyrics_enabled),
            u8::from(self.lyrics_in_pill),
            u8::from(self.page_system),
            u8::from(self.page_media),
            u8::from(self.page_shelf),
            u8::from(self.page_water),
            u8::from(self.page_pomodoro),
            u8::from(self.page_todo),
            self.enabled_plugins.join(","),
            scale,
            self.island_anchor as u8,
            ox,
            oy,
            u8::from(self.expand_on_hover),
            u8::from(self.hide_fullscreen),
            u8::from(self.volume_hud_enabled),
            u8::from(self.toast_power),
            u8::from(self.toast_audio),
            u8::from(self.toast_devices),
            u8::from(self.cpu_hog_alert),
            u8::from(self.privacy_dots),
            self.motion_speed as u8,
            self.glass_style as u8,
            u8::from(self.temp_fahrenheit),
        );
        let _ = fs::write(path, out);
    }
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub light: bool,
    pub shell_rim: u32,
    pub specular_top: u32,
    pub shadow_outer: u32,
    pub card_bg: u32,
    pub card_hover: u32,
    pub btn_bg: u32,
    pub btn_hover: u32,
    pub seg_active_bg: u32,
    pub cta_bg: u32,
    pub cta_fg: u32,
    pub track: u32,
    pub ink: u32,
    pub ink_soft: u32,
    pub muted: u32,
    pub accent_blue: u32,
    pub accent_purple: u32,
    pub accent_green: u32,
    pub accent_amber: u32,
    pub accent_red: u32,
    pub on_accent: u32,
}

impl Palette {
    pub fn for_theme(light: bool) -> Self {
        if light {
            Self {
                light: true,
                shell_rim: argb(20, 15, 23, 42),
                specular_top: argb(250, 255, 255, 255),
                shadow_outer: argb(34, 15, 23, 42),
                card_bg: argb(18, 15, 23, 42),
                card_hover: argb(30, 15, 23, 42),
                btn_bg: argb(20, 15, 23, 42),
                btn_hover: argb(36, 15, 23, 42),
                seg_active_bg: argb(245, 255, 255, 255),
                cta_bg: argb(242, 20, 23, 31),
                cta_fg: argb(255, 255, 255, 255),
                track: argb(28, 15, 23, 42),
                ink: argb(255, 16, 19, 26),
                ink_soft: argb(255, 62, 68, 82),
                muted: argb(255, 112, 119, 134),
                accent_blue: argb(255, 0, 122, 255),
                accent_purple: argb(255, 110, 86, 207),
                accent_green: argb(255, 36, 154, 78),
                accent_amber: argb(255, 204, 116, 18),
                accent_red: argb(255, 214, 54, 48),
                on_accent: argb(255, 255, 255, 255),
            }
        } else {
            Self {
                light: false,
                shell_rim: argb(18, 255, 255, 255),
                specular_top: argb(190, 255, 255, 255),
                shadow_outer: argb(58, 0, 0, 0),
                card_bg: argb(20, 255, 255, 255),
                card_hover: argb(34, 255, 255, 255),
                btn_bg: argb(26, 255, 255, 255),
                btn_hover: argb(48, 255, 255, 255),
                seg_active_bg: argb(56, 255, 255, 255),
                cta_bg: argb(242, 244, 246, 250),
                cta_fg: argb(255, 14, 16, 22),
                track: argb(32, 255, 255, 255),
                ink: argb(255, 245, 247, 250),
                ink_soft: argb(255, 192, 198, 208),
                muted: argb(255, 134, 140, 152),
                accent_blue: argb(255, 64, 156, 255),
                accent_purple: argb(255, 158, 134, 246),
                accent_green: argb(255, 56, 196, 108),
                accent_amber: argb(255, 242, 168, 59),
                accent_red: argb(255, 238, 92, 84),
                on_accent: argb(255, 255, 255, 255),
            }
        }
    }
}

thread_local! {
    static CURRENT: RefCell<AppSettings> = RefCell::new(AppSettings::default());
}

pub fn init() {
    let loaded = AppSettings::load();
    CURRENT.with(|c| *c.borrow_mut() = loaded);
}

pub fn get() -> AppSettings {
    CURRENT.with(|c| c.borrow().clone())
}

pub fn update(f: impl FnOnce(&mut AppSettings)) {
    CURRENT.with(|c| {
        let mut guard = c.borrow_mut();
        f(&mut guard);
        guard.clamp_values();
        guard.save();
    });
    crate::ui::on_settings_changed();
    crate::island::on_settings_changed();
    crate::settings_ui::on_settings_changed();
}

pub fn update_live(f: impl FnOnce(&mut AppSettings)) {
    CURRENT.with(|c| {
        let mut guard = c.borrow_mut();
        f(&mut guard);
        guard.clamp_values();
    });
    crate::ui::on_settings_changed();
    crate::island::on_settings_changed_live();
    crate::settings_ui::on_settings_changed();
}

pub fn flush_save() {
    CURRENT.with(|c| {
        c.borrow().save();
    });
}

pub fn reset_defaults() {
    update(|s| *s = AppSettings::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_enabled_pages_fallback() {
        let mut path = std::env::temp_dir();
        path.push("molap_settings_test.txt");
        let _ = fs::remove_file(&path);

        let mut s = AppSettings::default();
        assert_eq!(s.enabled_pages().len(), Page::COUNT);
        s.theme = ThemeMode::Light;
        s.idle_mode = IdleMode::Productivity;
        s.taskbar_fit = TaskbarFit::Small;
        s.alerts_enabled = false;
        s.lyrics_enabled = false;
        s.lyrics_in_pill = false;
        s.page_system = false;
        s.page_media = false;
        s.page_shelf = false;
        s.page_water = false;
        s.page_pomodoro = false;
        s.page_todo = false;
        s.island_scale_pct = 125;
        s.island_anchor = IslandAnchor::Right;
        s.island_offset_x = -120;
        s.island_offset_y = 24;
        s.expand_on_hover = false;
        s.hide_fullscreen = false;
        s.volume_hud_enabled = false;
        s.toast_power = false;
        s.toast_audio = false;
        s.toast_devices = false;
        s.cpu_hog_alert = false;
        s.privacy_dots = false;
        s.motion_speed = MotionSpeed::Snappy;
        s.glass_style = GlassStyle::Clear;
        s.temp_fahrenheit = true;
        s.toggle_plugin("network-pulse");
        assert!(s.is_plugin_enabled("network-pulse"));
        s.save_to_path(&path);

        let loaded = AppSettings::load_from_path(&path);
        let _ = fs::remove_file(&path);
        assert_eq!(loaded, s);
        assert!(loaded.is_light());
        assert_eq!(loaded.enabled_pages(), vec![Page::Media]);
        assert!(loaded.is_plugin_enabled("network-pulse"));
    }

    #[test]
    fn load_clamps_ranges_and_keeps_missing_key_defaults() {
        let mut path = std::env::temp_dir();
        path.push("molap_settings_clamp_test.txt");
        let _ = fs::remove_file(&path);

        // Legacy partial config with out-of-bounds values and missing new keys
        let raw = "theme=1\nisland_scale_pct=250\nisland_offset_x=-999\nisland_offset_y=99\n";
        fs::write(&path, raw).expect("write test config");

        let loaded = AppSettings::load_from_path(&path);
        let _ = fs::remove_file(&path);

        assert_eq!(loaded.theme, ThemeMode::Dark);
        assert_eq!(loaded.island_scale_pct, 140);
        assert_eq!(loaded.island_offset_x, -400);
        assert_eq!(loaded.island_offset_y, 48);
        assert_eq!(loaded.island_anchor, IslandAnchor::Center);
        assert_eq!(loaded.motion_speed, MotionSpeed::Fluid);
        assert_eq!(loaded.glass_style, GlassStyle::Frosted);
        assert!(loaded.expand_on_hover);
        assert!(loaded.hide_fullscreen);
        assert!(loaded.volume_hud_enabled);
        assert!(loaded.toast_power);
        assert!(loaded.toast_audio);
        assert!(loaded.toast_devices);
        assert!(loaded.cpu_hog_alert);
        assert!(loaded.privacy_dots);
        assert!(!loaded.temp_fahrenheit);

        // Lower-bound clamping
        fs::write(
            &path,
            "island_scale_pct=10\nisland_offset_x=900\nisland_offset_y=-50\n",
        )
        .expect("write low clamp config");
        let low = AppSettings::load_from_path(&path);
        let _ = fs::remove_file(&path);
        assert_eq!(low.island_scale_pct, 80);
        assert_eq!(low.island_offset_x, 400);
        assert_eq!(low.island_offset_y, -8);
    }

    #[test]
    fn enum_helpers_and_format_temp() {
        assert_eq!(IslandAnchor::from_u8(0), IslandAnchor::Center);
        assert_eq!(IslandAnchor::from_u8(1), IslandAnchor::Left);
        assert_eq!(IslandAnchor::from_u8(2), IslandAnchor::Right);
        assert_eq!(IslandAnchor::from_u8(9), IslandAnchor::Center);

        assert_eq!(MotionSpeed::from_u8(0).spring_scale(), 1.0);
        assert_eq!(MotionSpeed::from_u8(1).spring_scale(), 1.38);
        assert_eq!(MotionSpeed::from_u8(2).spring_scale(), 0.72);

        assert_eq!(GlassStyle::from_u8(0).opacity_mul(), 1.0);
        assert_eq!(GlassStyle::from_u8(1).opacity_mul(), 0.78);
        assert_eq!(GlassStyle::from_u8(2).opacity_mul(), 1.12);

        assert_eq!(format_temp(42, false), "42°C");
        assert_eq!(format_temp(42, true), "108°F");
        assert_eq!(format_temp(0, true), "32°F");
        assert_eq!(format_temp(100, true), "212°F");
    }

    #[test]
    fn needs_sensors_tracks_island() {
        let mut s = AppSettings {
            island_enabled: true,
            page_system: false,
            idle_mode: IdleMode::Minimal,
            ..Default::default()
        };
        assert!(!s.needs_sensors());

        s.idle_mode = IdleMode::Smart;
        assert!(s.needs_sensors());

        s.idle_mode = IdleMode::Telemetry;
        assert!(s.needs_sensors());

        s.idle_mode = IdleMode::Minimal;
        s.page_system = true;
        assert!(s.needs_sensors());

        s.island_enabled = false;
        assert!(!s.needs_sensors());
    }
}
