// settings -- the user's system settings the UI follows
//
//   sys_settings() -> SysSettings       the system's (Windows: system calls and the registry;
//                                       Linux: the settings portal, at most 300 ms; elsewhere
//                                       the defaults), then the jfgui_* overrides
//   sys_settings_env() -> SysSettings   the defaults and the overrides only (headless runs)
//
// Overrides (tests, shots, users): jfgui_THEME=dark|light, jfgui_ACCENT=RRGGBB,
// jfgui_CONTRAST=high|normal, jfgui_REDUCE_MOTION=1|0, jfgui_TEXT_SCALE=PERCENT,
// jfgui_DOUBLE_CLICK_MS=N, jfgui_CARET_BLINK_MS=N (0: a steady caret),
// jfgui_FONT_AA=1|0, jfgui_FONT_HINTING=none|slight|medium|full,
// jfgui_FONT_RGBA=none|rgb|bgr|vrgb|vbgr.
//
// Font rendering (font_aa, font_hinting, font_rgba; None: the system says nothing): Windows
// SPI_GETFONTSMOOTHING, SPI_GETFONTSMOOTHINGTYPE (ClearType: subpixels) and
// SPI_GETFONTSMOOTHINGORIENTATION; Linux the portal's org.gnome.desktop.interface
// font-antialiasing / font-hinting / font-rgba-order, else what fontconfig's conf.d turns on
// (10-hinting-*.conf, 10-sub-pixel-*.conf, 10-no-antialias.conf, 10-yes-antialias.conf).

/// the colours of a high-contrast scheme (0xRRGGBB)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HcColors {
    pub bg: u32,
    pub text: u32,
    /// selection, accent fills
    pub highlight: u32,
    pub highlight_text: u32,
    pub button: u32,
    pub button_text: u32,
    pub disabled: u32,
    pub link: u32,
}

impl HcColors {
    /// white on black (Windows "Night sky" / "HC Black")
    pub const BLACK: HcColors = HcColors {
        bg: 0x000000,
        text: 0xFFFFFF,
        highlight: 0x1AEBFF,
        highlight_text: 0x000000,
        button: 0x000000,
        button_text: 0xFFFFFF,
        disabled: 0x3FF23F,
        link: 0xFFFF00,
    };
    /// black on white (Windows "Desert" / "HC White")
    pub const WHITE: HcColors = HcColors {
        bg: 0xFFFFFF,
        text: 0x000000,
        highlight: 0x37006E,
        highlight_text: 0xFFFFFF,
        button: 0xFFFFFF,
        button_text: 0x000000,
        disabled: 0x600000,
        link: 0x00009F,
    };
}

/// SysSettings::font_hinting
pub const FONT_HINT_NONE: u8 = 0;
pub const FONT_HINT_SLIGHT: u8 = 1;
pub const FONT_HINT_MEDIUM: u8 = 2;
pub const FONT_HINT_FULL: u8 = 3;
/// SysSettings::font_rgba: the order of a pixel's subpixels (none: greyscale smoothing)
pub const FONT_RGBA_NONE: u8 = 0;
pub const FONT_RGBA_RGB: u8 = 1;
pub const FONT_RGBA_BGR: u8 = 2;
pub const FONT_RGBA_VRGB: u8 = 3;
pub const FONT_RGBA_VBGR: u8 = 4;

/// a FONT_HINT_* name ("none", "slight", "medium", "full"; also "hintnone" ... of fontconfig)
pub fn font_hint_parse(s: &str) -> Option<u8> {
    match s.trim().trim_start_matches("hint") {
        "none" => Some(FONT_HINT_NONE),
        "slight" => Some(FONT_HINT_SLIGHT),
        "medium" => Some(FONT_HINT_MEDIUM),
        "full" => Some(FONT_HINT_FULL),
        _ => None,
    }
}

/// a FONT_RGBA_* name ("none", "rgb", "bgr", "vrgb", "vbgr"; "grayscale" is none)
pub fn font_rgba_parse(s: &str) -> Option<u8> {
    match s.trim() {
        "none" | "grayscale" | "greyscale" => Some(FONT_RGBA_NONE),
        "rgb" => Some(FONT_RGBA_RGB),
        "bgr" => Some(FONT_RGBA_BGR),
        "vrgb" => Some(FONT_RGBA_VRGB),
        "vbgr" => Some(FONT_RGBA_VBGR),
        _ => None,
    }
}

/// what the system says (see the module comment)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SysSettings {
    /// None: no preference
    pub dark: Option<bool>,
    /// 0xRRGGBB
    pub accent: Option<u32>,
    pub high_contrast: bool,
    /// Windows: the colours of the active high-contrast scheme
    pub contrast_colors: Option<HcColors>,
    pub reduce_motion: bool,
    /// percent, 100 = normal
    pub text_scale: u32,
    pub double_click_ms: u32,
    /// one phase of the caret (on or off); 0 = steady
    pub caret_blink_ms: u32,
    /// font smoothing: Some(false) = off (sharp black-and-white glyphs)
    pub font_aa: Option<bool>,
    /// FONT_HINT_*
    pub font_hinting: Option<u8>,
    /// FONT_RGBA_* (subpixel smoothing and its order)
    pub font_rgba: Option<u8>,
}

impl Default for SysSettings {
    fn default() -> Self {
        SysSettings {
            dark: None,
            accent: None,
            high_contrast: false,
            contrast_colors: None,
            reduce_motion: false,
            text_scale: 100,
            double_click_ms: 500,
            caret_blink_ms: 530,
            font_aa: None,
            font_hinting: None,
            font_rgba: None,
        }
    }
}

/// the system's settings and the jfgui_* overrides
pub fn sys_settings() -> SysSettings {
    let mut s = SysSettings::default();
    #[cfg(any(windows, target_os = "linux"))]
    crate::sys_settings_os(&mut s);
    env_overrides(&mut s);
    s
}

/// the defaults and the jfgui_* overrides only (headless runs stay the same everywhere)
pub fn sys_settings_env() -> SysSettings {
    let mut s = SysSettings::default();
    env_overrides(&mut s);
    s
}

fn env_overrides(s: &mut SysSettings) {
    let num = |n: &str| crate::env(n).and_then(|v| v.trim().parse::<u32>().ok());
    match crate::env("jfgui_THEME").as_deref() {
        Some("dark") => s.dark = Some(true),
        Some("light") => s.dark = Some(false),
        _ => {}
    }
    if let Some(v) = crate::env("jfgui_ACCENT") {
        if let Ok(c) = u32::from_str_radix(v.trim().trim_start_matches('#'), 16) {
            s.accent = Some(c & 0xFF_FFFF);
        }
    }
    match crate::env("jfgui_CONTRAST").as_deref() {
        Some("high") | Some("1") => s.high_contrast = true,
        Some("normal") | Some("0") => {
            s.high_contrast = false;
            s.contrast_colors = None;
        }
        _ => {}
    }
    if let Some(v) = num("jfgui_REDUCE_MOTION") {
        s.reduce_motion = v != 0;
    }
    if let Some(v) = num("jfgui_TEXT_SCALE") {
        s.text_scale = v.clamp(50, 300);
    }
    if let Some(v) = num("jfgui_DOUBLE_CLICK_MS") {
        s.double_click_ms = v.clamp(100, 5000);
    }
    if let Some(v) = num("jfgui_CARET_BLINK_MS") {
        s.caret_blink_ms = v.min(5000);
    }
    if let Some(v) = num("jfgui_FONT_AA") {
        s.font_aa = Some(v != 0);
    }
    if let Some(v) = crate::env("jfgui_FONT_HINTING").as_deref().and_then(font_hint_parse) {
        s.font_hinting = Some(v);
    }
    if let Some(v) = crate::env("jfgui_FONT_RGBA").as_deref().and_then(font_rgba_parse) {
        s.font_rgba = Some(v);
    }
}
