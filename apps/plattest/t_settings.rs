// t_settings -- the system settings the UI follows (lib/plat/settings.rs, linux_settings.rs)
//
//   plattest --test settings       the jfgui_* overrides (and their limits), the defaults, the
//                                  title bar's layout and actions (GNOME's names); Windows: the
//                                  system's values are plausible; Linux without the portal
//                                  (jfgui_PORTAL_SETTINGS=0): the defaults, then GTK's
//                                  settings.ini (gtk-4.0 before gtk-3.0) and KDE's kdeglobals in
//                                  a scratch XDG_CONFIG_HOME
//   plattest --test settings-bus   (Linux; tools/platshared/settings_bus.py starts it on a fake
//                                  session bus) the portal's ReadAll: color-scheme, accent-color
//                                  (ddd), high-contrast, enable-animations, text-scaling-factor
//                                  (d), cursor-blink-time, double-click, a value in two variants,
//                                  GNOME's button-layout and action-*-titlebar; with
//                                  --no-portal-files (the fake answers without appearance keys):
//                                  what the portal does not say comes from the files

use alloc::format;

use plat::{SysSettings, sys_settings, sys_settings_env};
use plat::{Titlebar, titlebar_action_parse, titlebar_layout_parse, TB_ACT_LOWER, TB_ACT_MAXIMIZE, TB_ACT_MENU, TB_ACT_MINIMIZE, TB_ACT_NONE};
use plat::{TB_BTN_CLOSE as C, TB_BTN_MAX as X, TB_BTN_MIN as N};

use crate::T;

const VARS: [&str; 14] = [
    "jfgui_THEME", "jfgui_ACCENT", "jfgui_CONTRAST", "jfgui_REDUCE_MOTION", "jfgui_TEXT_SCALE", "jfgui_DOUBLE_CLICK_MS", "jfgui_CARET_BLINK_MS",
    "jfgui_FONT_AA", "jfgui_FONT_HINTING", "jfgui_FONT_RGBA", "jfgui_BUTTON_LAYOUT", "jfgui_TITLEBAR_DOUBLE_CLICK", "jfgui_TITLEBAR_MIDDLE_CLICK",
    "jfgui_TITLEBAR_RIGHT_CLICK",
];

fn clear() {
    for v in VARS {
        plat::setenv(v, None);
    }
}

pub fn t_settings(t: &mut T) {
    clear();
    t.check(sys_settings_env() == SysSettings::default(), "settings: no overrides: the defaults");
    let d = SysSettings::default();
    t.check(d.text_scale == 100 && d.double_click_ms == 500 && d.caret_blink_ms == 530 && d.dark.is_none() && !d.high_contrast, "settings: the defaults");
    for (k, v) in [
        ("jfgui_THEME", "dark"),
        ("jfgui_ACCENT", "#123456"),
        ("jfgui_CONTRAST", "high"),
        ("jfgui_REDUCE_MOTION", "1"),
        ("jfgui_TEXT_SCALE", "400"),
        ("jfgui_DOUBLE_CLICK_MS", "50"),
        ("jfgui_CARET_BLINK_MS", "0"),
        ("jfgui_FONT_AA", "0"),
        ("jfgui_FONT_HINTING", "slight"),
        ("jfgui_FONT_RGBA", "bgr"),
    ] {
        plat::setenv(k, Some(v));
    }
    let s = sys_settings_env();
    t.check(s.dark == Some(true) && s.accent == Some(0x123456), "settings: jfgui_THEME, jfgui_ACCENT");
    t.check(s.high_contrast && s.reduce_motion, "settings: jfgui_CONTRAST, jfgui_REDUCE_MOTION");
    t.check(s.text_scale == 300 && s.double_click_ms == 100 && s.caret_blink_ms == 0, "settings: scale and times within their limits");
    t.check(s.font_aa == Some(false) && s.font_hinting == Some(plat::FONT_HINT_SLIGHT) && s.font_rgba == Some(plat::FONT_RGBA_BGR),
        "settings: jfgui_FONT_AA, jfgui_FONT_HINTING, jfgui_FONT_RGBA");
    t.check(plat::font_hint_parse("hintfull") == Some(plat::FONT_HINT_FULL) && plat::font_rgba_parse("grayscale") == Some(plat::FONT_RGBA_NONE)
        && plat::font_hint_parse("strong").is_none(), "settings: font names (fontconfig / GNOME)");
    plat::setenv("jfgui_CONTRAST", Some("normal"));
    plat::setenv("jfgui_THEME", Some("light"));
    let s = sys_settings_env();
    t.check(!s.high_contrast && s.dark == Some(false), "settings: jfgui_CONTRAST=normal, jfgui_THEME=light");
    clear();
    titlebar(t);
    #[cfg(windows)]
    {
        let s = sys_settings();
        t.info(&format!("  (system: {s:?})\n"));
        t.check((100..=5000).contains(&s.double_click_ms) && (100..=225).contains(&s.text_scale) && s.caret_blink_ms <= 5000, "settings: the system's values are plausible");
        t.check(!s.high_contrast || s.contrast_colors.is_some(), "settings: high contrast comes with its colours");
        t.check(s.font_aa.is_some() && (s.font_aa == Some(false) || s.font_rgba.is_some()), "settings: font smoothing and its kind");
    }
    #[cfg(target_os = "linux")]
    {
        plat::setenv("jfgui_PORTAL_SETTINGS", Some("0"));
        plat::setenv("jfgui_FONTCONFIG_SETTINGS", Some("0"));
        plat::setenv("jfgui_CONFIG_SETTINGS", Some("0"));
        t.check(sys_settings() == SysSettings::default(), "settings: without the portal, the files and fontconfig the defaults");
        plat::setenv("jfgui_CONFIG_SETTINGS", None);
        files(t);
        plat::setenv("jfgui_FONTCONFIG_SETTINGS", None);
        let s = sys_settings();
        t.info(&format!("  (fontconfig: aa {:?} hinting {:?} rgba {:?})\n", s.font_aa, s.font_hinting, s.font_rgba));
        plat::setenv("jfgui_PORTAL_SETTINGS", None);
    }
}

/// a scratch XDG_CONFIG_HOME (removed by files_done): its path
#[cfg(target_os = "linux")]
pub(crate) fn files_home(t: &mut T, tag: &str) -> alloc::string::String {
    let home = format!("{}/build/plattest/{}-{tag}-{:x}", plat::cwd(), crate::OS, plat::time_us() & 0xFF_FFFF);
    t.check(plat::mkdir(&format!("{home}/gtk-4.0")) && plat::mkdir(&format!("{home}/gtk-3.0")), "settings files: a scratch XDG_CONFIG_HOME");
    home
}

#[cfg(target_os = "linux")]
pub(crate) fn files_done(home: &str) {
    for f in ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini", "kdeglobals"] {
        plat::delete(&format!("{home}/{f}"));
    }
    plat::delete(&format!("{home}/gtk-4.0"));
    plat::delete(&format!("{home}/gtk-3.0"));
    plat::delete(home);
    plat::delete("build/plattest");
}

// without the portal: GTK's settings.ini files, then KDE's kdeglobals (linux_settings.rs)
#[cfg(target_os = "linux")]
fn files(t: &mut T) {
    let home = files_home(t, "settings");
    let old = (plat::env("XDG_CONFIG_HOME"), plat::env("XDG_CURRENT_DESKTOP"));
    plat::setenv("XDG_CONFIG_HOME", Some(&home));
    plat::setenv("XDG_CURRENT_DESKTOP", Some("GNOME"));
    let put = |f: &str, text: &str| {
        let p = format!("{home}/{f}");
        if text.is_empty() { plat::delete(&p) } else { plat::write_file(&p, text.as_bytes()) }
    };
    t.check(sys_settings().dark.is_none(), "settings files: none, no answer");
    put("gtk-3.0/settings.ini", "[Settings]\ngtk-theme-name=Adwaita\ngtk-application-prefer-dark-theme=1\n");
    t.check(sys_settings().dark == Some(true), "settings files: gtk-3.0 gtk-application-prefer-dark-theme=1 -> dark");
    put("gtk-3.0/settings.ini", "[Settings]\ngtk-theme-name = \"Yaru-dark\"\n");
    t.check(sys_settings().dark == Some(true), "settings files: gtk-3.0 gtk-theme-name Yaru-dark -> dark");
    put("gtk-4.0/settings.ini", "# GTK 4\n[Settings]\ngtk-application-prefer-dark-theme=false\ngtk-theme-name=Adwaita\ngtk-decoration-layout=close,minimize:maximize\ngtk-titlebar-double-click=minimize\ngtk-titlebar-right-click=none\n");
    let s = sys_settings();
    t.check(s.dark == Some(false), "settings files: gtk-4.0 decides before gtk-3.0 (light)");
    t.check(s.titlebar == Titlebar { buttons: Some(([C, N, 0], [X, 0, 0])), double: TB_ACT_MINIMIZE, middle: TB_ACT_NONE, right: TB_ACT_NONE },
        "settings files: gtk-decoration-layout, gtk-titlebar-double-click / -right-click");
    put("gtk-4.0/settings.ini", "[Settings]\ngtk-font-name=Cantarell 11\n");
    t.check(sys_settings().dark == Some(true), "settings files: a gtk-4.0 file without theme keys: gtk-3.0 decides");
    put("gtk-4.0/settings.ini", "");
    put("gtk-3.0/settings.ini", "");
    // KDE: the window colour, else the scheme's name; the accent on KDE or without any portal
    put("kdeglobals", "[General]\nColorScheme=Nordic\nAccentColor=61,174,233\n\n[Colors:Window]\nBackgroundNormal=35,38,41\nForegroundNormal=252,252,252\n");
    let s = sys_settings();
    t.check(s.dark == Some(true) && s.accent == Some(0x3DAEE9), "settings files: kdeglobals BackgroundNormal 35,38,41 -> dark, AccentColor 61,174,233");
    put("kdeglobals", "[General]\nColorScheme=BreezeDark\n");
    t.check(sys_settings().dark == Some(true), "settings files: kdeglobals ColorScheme=BreezeDark -> dark");
    put("kdeglobals", "[General]\nColorScheme=BreezeLight\n[Colors:Window]\nBackgroundNormal=239,240,241\n");
    t.check(sys_settings().dark == Some(false), "settings files: kdeglobals BreezeLight, a light window -> light");
    put("gtk-3.0/settings.ini", "[Settings]\ngtk-application-prefer-dark-theme=true\n");
    t.check(sys_settings().dark == Some(true), "settings files: GTK's file before kdeglobals");
    plat::setenv("jfgui_THEME", Some("light"));
    plat::setenv("jfgui_ACCENT", Some("123456"));
    let s = sys_settings();
    t.check(s.dark == Some(false) && s.accent == Some(0x123456), "settings files: the jfgui_* overrides win");
    plat::setenv("jfgui_THEME", None);
    plat::setenv("jfgui_ACCENT", None);
    plat::setenv("jfgui_CONFIG_SETTINGS", Some("0"));
    t.check(sys_settings().dark.is_none(), "settings files: jfgui_CONFIG_SETTINGS=0 skips them");
    plat::setenv("jfgui_CONFIG_SETTINGS", None);
    files_done(&home);
    plat::setenv("XDG_CONFIG_HOME", old.0.as_deref());
    plat::setenv("XDG_CURRENT_DESKTOP", old.1.as_deref());
}

// the title bar: GNOME's names, the defaults, the overrides
fn titlebar(t: &mut T) {
    t.check(Titlebar::default() == Titlebar { buttons: None, double: TB_ACT_MAXIMIZE, middle: TB_ACT_NONE, right: TB_ACT_MENU },
        "titlebar: the defaults (the style's buttons, double click maximizes, right click the window menu)");
    let l = titlebar_layout_parse;
    t.check(l("appmenu:minimize,maximize,close") == ([0; 3], [N, X, C]), "titlebar: appmenu:minimize,maximize,close");
    t.check(l("close,minimize,maximize:") == ([C, N, X], [0; 3]), "titlebar: close,minimize,maximize: (all at the left)");
    t.check(l("appmenu:close") == ([0; 3], [C, 0, 0]), "titlebar: GNOME's default appmenu:close");
    t.check(l("close") == ([C, 0, 0], [0; 3]), "titlebar: without ':' everything is the left side (GTK)");
    t.check(l(" close , spacer,close:maximize ,minimize,icon") == ([C, 0, 0], [X, N, 0]), "titlebar: blanks, unknown names and repeats");
    t.check(l(":") == ([0; 3], [0; 3]) && l("") == ([0; 3], [0; 3]), "titlebar: no buttons");
    let a = titlebar_action_parse;
    t.check(a("toggle-maximize") == Some(TB_ACT_MAXIMIZE) && a("toggle-maximize-horizontally") == Some(TB_ACT_MAXIMIZE)
        && a("toggle-maximize-vertically") == Some(TB_ACT_MAXIMIZE), "titlebar: toggle-maximize (and its variants)");
    t.check(a("minimize") == Some(TB_ACT_MINIMIZE) && a("lower") == Some(TB_ACT_LOWER) && a("menu") == Some(TB_ACT_MENU)
        && a("none") == Some(TB_ACT_NONE) && a("toggle-shade") == Some(TB_ACT_NONE) && a("shade").is_none(), "titlebar: the other actions");
    plat::setenv("jfgui_BUTTON_LAYOUT", Some("close,minimize:maximize"));
    plat::setenv("jfgui_TITLEBAR_DOUBLE_CLICK", Some("minimize"));
    plat::setenv("jfgui_TITLEBAR_MIDDLE_CLICK", Some("lower"));
    plat::setenv("jfgui_TITLEBAR_RIGHT_CLICK", Some("bogus"));
    let s = sys_settings_env();
    t.check(s.titlebar == Titlebar { buttons: Some(([C, N, 0], [X, 0, 0])), double: TB_ACT_MINIMIZE, middle: TB_ACT_LOWER, right: TB_ACT_MENU },
        "titlebar: jfgui_BUTTON_LAYOUT, jfgui_TITLEBAR_*_CLICK (an unknown action keeps the default)");
    clear();
}

#[cfg(not(target_os = "linux"))]
pub fn t_settings_bus(_: &mut T) {
    plat::write_stderr(b"plattest: --test settings-bus is Linux only\n");
    plat::exit(2);
}

#[cfg(target_os = "linux")]
pub fn t_settings_bus(t: &mut T) {
    clear();
    let t0 = plat::time_us();
    let s = sys_settings();
    t.info(&format!("  (portal: {s:?}, {} ms)\n", (plat::time_us() - t0) / 1000));
    t.check(s.dark == Some(true), "settings-bus: color-scheme 1 -> dark");
    t.check(s.accent == Some(0x336699), "settings-bus: accent-color (0.2, 0.4, 0.6) -> #336699");
    t.check(s.high_contrast && s.contrast_colors.is_none(), "settings-bus: a11y high-contrast (in two variants) -> high contrast");
    t.check(s.reduce_motion, "settings-bus: enable-animations false -> reduced motion");
    t.check(s.text_scale == 125, "settings-bus: text-scaling-factor 1.25 -> 125 %");
    t.check(s.caret_blink_ms == 600 && s.double_click_ms == 400, "settings-bus: cursor-blink-time 1200 -> 600 ms, double-click 400 ms");
    t.check(s.font_aa == Some(true) && s.font_hinting == Some(plat::FONT_HINT_MEDIUM) && s.font_rgba == Some(plat::FONT_RGBA_BGR),
        "settings-bus: font-antialiasing rgba, font-hinting medium, font-rgba-order bgr");
    t.check(s.titlebar == Titlebar { buttons: Some(([C, 0, 0], [N, X, 0])), double: TB_ACT_MINIMIZE, middle: TB_ACT_LOWER, right: TB_ACT_NONE },
        "settings-bus: wm.preferences button-layout close:minimize,maximize, action-*-titlebar minimize / lower / none");
    plat::setenv("jfgui_TEXT_SCALE", Some("150"));
    t.check(sys_settings().text_scale == 150, "settings-bus: an override wins over the portal");
    plat::setenv("jfgui_TEXT_SCALE", None);
}

/// --test settings-bus-files: a portal that says nothing about the appearance (an empty
/// org.freedesktop.appearance): GTK's and KDE's files fill in (outside KDE no accent from
/// kdeglobals while a portal answers)
#[cfg(target_os = "linux")]
pub fn t_settings_bus_files(t: &mut T) {
    clear();
    let home = files_home(t, "settings-bus");
    plat::setenv("XDG_CONFIG_HOME", Some(&home));
    plat::setenv("XDG_CURRENT_DESKTOP", Some("GNOME"));
    plat::write_file(&format!("{home}/gtk-3.0/settings.ini"), b"[Settings]\ngtk-application-prefer-dark-theme=1\ngtk-decoration-layout=close:\n");
    plat::write_file(&format!("{home}/kdeglobals"), b"[General]\nAccentColor=61,174,233\n");
    let s = sys_settings();
    t.info(&format!("  (portal + files: {s:?})\n"));
    t.check(s.dark == Some(true), "settings-bus-files: no color-scheme from the portal: gtk-3.0 prefers dark");
    t.check(s.accent.is_none(), "settings-bus-files: GNOME with a portal: no accent from a stale kdeglobals");
    t.check(s.titlebar.buttons == Some(([0; 3], [N, X, C])), "settings-bus-files: the portal's button-layout before the file's");
    plat::setenv("XDG_CURRENT_DESKTOP", Some("KDE"));
    t.check(sys_settings().accent == Some(0x3DAEE9), "settings-bus-files: on KDE the accent of kdeglobals");
    files_done(&home);
}

#[cfg(not(target_os = "linux"))]
pub fn t_settings_bus_files(_: &mut T) {
    plat::write_stderr(b"plattest: --test settings-bus-files is Linux only\n");
    plat::exit(2);
}
