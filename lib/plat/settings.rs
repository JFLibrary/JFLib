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
//
// The title bar the library draws (titlebar: the buttons and what clicks on the bar do): Linux
// GNOME's org.gnome.desktop.wm.preferences button-layout, action-double-click-titlebar,
// action-middle-click-titlebar, action-right-click-titlebar (the portal), else GTK's
// gtk-decoration-layout / gtk-titlebar-*-click (settings.ini); Windows has none (its own title
// bar and the system's snap / caption behaviour).  Overrides: jfgui_BUTTON_LAYOUT=LAYOUT (GNOME's
// syntax, e.g. "close,minimize,maximize:"), jfgui_TITLEBAR_DOUBLE_CLICK / _MIDDLE_CLICK /
// _RIGHT_CLICK=toggle-maximize|minimize|lower|menu|none.

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

/// Titlebar::buttons: the caption buttons of a title bar the library draws
pub const TB_BTN_CLOSE: u8 = 1;
pub const TB_BTN_MIN: u8 = 2;
pub const TB_BTN_MAX: u8 = 3;
/// Titlebar::double / middle / right: what a click on the empty part of the bar does
pub const TB_ACT_NONE: u8 = 0;
/// maximize, or restore a maximized window
pub const TB_ACT_MAXIMIZE: u8 = 1;
pub const TB_ACT_MINIMIZE: u8 = 2;
/// below the other windows (X11; Wayland has no request for it: nothing)
pub const TB_ACT_LOWER: u8 = 3;
/// the window manager's window menu (move, resize, always on top, ...)
pub const TB_ACT_MENU: u8 = 4;

/// the buttons and click actions of a title bar the library draws (SysSettings::titlebar)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Titlebar {
    /// the buttons at the left end and at the right end, each side left to right (TB_BTN_*,
    /// 0 ends a side; each button at most once); None: the style's places
    pub buttons: Option<([u8; 3], [u8; 3])>,
    /// TB_ACT_* of a double click, a middle click and a right click
    pub double: u8,
    pub middle: u8,
    pub right: u8,
}

impl Default for Titlebar {
    /// the style's buttons, double click maximizes, right click the window menu
    fn default() -> Self {
        Titlebar { buttons: None, double: TB_ACT_MAXIMIZE, middle: TB_ACT_NONE, right: TB_ACT_MENU }
    }
}

/// a button layout in GNOME's button-layout syntax (GTK's gtk-decoration-layout): the left
/// side, ':', the right side, names separated by ','.  close, minimize and maximize are kept,
/// other names (appmenu, icon, menu, spacer) and repeats skipped; without ':' everything is
/// the left side (as GTK reads it)
pub fn titlebar_layout_parse(s: &str) -> ([u8; 3], [u8; 3]) {
    let (l, r) = s.split_once(':').unwrap_or((s, ""));
    let mut seen = 0u8;
    let mut side = |part: &str| {
        let mut out = [0u8; 3];
        let mut n = 0;
        for name in part.split(',') {
            let b = match name.trim() {
                "close" => TB_BTN_CLOSE,
                "minimize" => TB_BTN_MIN,
                "maximize" => TB_BTN_MAX,
                _ => continue,
            };
            if seen & (1 << b) == 0 && n < 3 {
                seen |= 1 << b;
                out[n] = b;
                n += 1;
            }
        }
        out
    };
    let left = side(l);
    (left, side(r))
}

/// a title bar action as GNOME names it (GDesktopTitlebarAction): toggle-maximize (also
/// -horizontally / -vertically, which xdg-shell can not do: GTK treats them alike), minimize,
/// lower, menu, none; toggle-shade is none (no compositor shades client-side windows)
pub fn titlebar_action_parse(s: &str) -> Option<u8> {
    match s.trim() {
        "toggle-maximize" | "toggle-maximize-horizontally" | "toggle-maximize-vertically" => Some(TB_ACT_MAXIMIZE),
        "minimize" => Some(TB_ACT_MINIMIZE),
        "lower" => Some(TB_ACT_LOWER),
        "menu" => Some(TB_ACT_MENU),
        "none" | "toggle-shade" => Some(TB_ACT_NONE),
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
    /// the title bar the library draws (Linux client-side decorations, AF_CUSTOM_TITLEBAR)
    pub titlebar: Titlebar,
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
            titlebar: Titlebar::default(),
        }
    }
}

/// the system's settings and the jfgui_* overrides
pub fn sys_settings() -> SysSettings {
    let mut s = SysSettings::default();
    #[cfg(any(windows, target_os = "linux"))]
    crate::sys_settings_os(&mut s);
    // the browser's media queries / Android's configuration, as the host sent them (web_system.rs)
    #[cfg(any(target_arch = "wasm32", target_os = "android"))]
    crate::web::sys_settings_host(&mut s);
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
    if let Some(v) = crate::env("jfgui_BUTTON_LAYOUT") {
        s.titlebar.buttons = Some(titlebar_layout_parse(&v));
    }
    let act = |n: &str| crate::env(n).as_deref().and_then(titlebar_action_parse);
    if let Some(a) = act("jfgui_TITLEBAR_DOUBLE_CLICK") {
        s.titlebar.double = a;
    }
    if let Some(a) = act("jfgui_TITLEBAR_MIDDLE_CLICK") {
        s.titlebar.middle = a;
    }
    if let Some(a) = act("jfgui_TITLEBAR_RIGHT_CLICK") {
        s.titlebar.right = a;
    }
}
