// t_settings -- the system settings the UI follows (lib/plat/settings.rs, linux_settings.rs)
//
//   plattest --test settings       the jfgui_* overrides (and their limits), the defaults;
//                                  Windows: the system's values are plausible; Linux without
//                                  the portal (jfgui_PORTAL_SETTINGS=0): the defaults
//   plattest --test settings-bus   (Linux; tools/platshared/settings_bus.py starts it on a fake
//                                  session bus) the portal's ReadAll: color-scheme, accent-color
//                                  (ddd), high-contrast, enable-animations, text-scaling-factor
//                                  (d), cursor-blink-time, double-click, a value in two variants

use alloc::format;

use plat::{SysSettings, sys_settings, sys_settings_env};

use crate::T;

const VARS: [&str; 10] = [
    "jfgui_THEME", "jfgui_ACCENT", "jfgui_CONTRAST", "jfgui_REDUCE_MOTION", "jfgui_TEXT_SCALE", "jfgui_DOUBLE_CLICK_MS", "jfgui_CARET_BLINK_MS",
    "jfgui_FONT_AA", "jfgui_FONT_HINTING", "jfgui_FONT_RGBA",
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
        t.check(sys_settings() == SysSettings::default(), "settings: without the portal and fontconfig the defaults");
        plat::setenv("jfgui_FONTCONFIG_SETTINGS", None);
        let s = sys_settings();
        t.info(&format!("  (fontconfig: aa {:?} hinting {:?} rgba {:?})\n", s.font_aa, s.font_hinting, s.font_rgba));
        plat::setenv("jfgui_PORTAL_SETTINGS", None);
    }
}

#[cfg(windows)]
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
    plat::setenv("jfgui_TEXT_SCALE", Some("150"));
    t.check(sys_settings().text_scale == 150, "settings-bus: an override wins over the portal");
    plat::setenv("jfgui_TEXT_SCALE", None);
}
