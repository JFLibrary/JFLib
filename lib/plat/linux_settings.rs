// linux_settings -- the system settings from the settings portal (settings.rs)
//
//   sys_settings_os(&mut s)   org.freedesktop.portal.Settings.ReadAll over the session bus,
//                             then the configuration files for what it did not say
//   files_used()              (linux_life.rs) the last read took something from the files:
//                             their changes matter
//
//   org.freedesktop.appearance       color-scheme (1 dark, 2 light, 0 no preference: light --
//                                    GNOME's "Default" style -- unless the GTK theme is a
//                                    dark one, e.g. Yaru-dark), accent-color ((ddd) 0..1),
//                                    contrast (1 high), reduced-motion (1)
//   org.gnome.desktop.interface      enable-animations, text-scaling-factor, cursor-blink,
//                                    cursor-blink-time (a whole cycle), gtk-theme
//                                    (HighContrast*), font-antialiasing (none / grayscale /
//                                    rgba), font-hinting, font-rgba-order
//   org.gnome.desktop.a11y.interface high-contrast
//   org.gnome.desktop.peripherals.mouse double-click
//   org.gnome.desktop.wm.preferences button-layout, action-double-click-titlebar,
//                                    action-middle-click-titlebar, action-right-click-titlebar
//                                    (the title bar the library draws; GTK reads the same)
//
// Without color-scheme a GTK theme named *-dark means dark.  Without the portal (or for what
// it did not say) the files, in this order: GTK's $XDG_CONFIG_HOME/gtk-4.0/settings.ini, then
// gtk-3.0/settings.ini ([Settings] gtk-application-prefer-dark-theme, gtk-theme-name *-dark /
// *:dark: the first file with either decides; gtk-decoration-layout, gtk-titlebar-double-click
// / -middle-click / -right-click), then KDE's $XDG_CONFIG_HOME/kdeglobals ([Colors:Window]
// BackgroundNormal "r,g,b": dark below half luminance, else [General] ColorScheme naming
// "Dark"; [General] AccentColor "r,g,b" -- the accent only on KDE (XDG_CURRENT_DESKTOP) or
// without any portal answer, so a stale kdeglobals does not colour a GNOME session).  The
// answer is kept for portal_dark() (plat::theme: the UI's "follow the system" choice; nothing
// before the first read, so headless runs, which never read the portal, stay independent of
// the desktop).  The portal gets up to 1 s the first time (it may still be starting), 300 ms
// later on; jfgui_PORTAL_SETTINGS=0 skips it, jfgui_CONFIG_SETTINGS=0 the files.  Font
// rendering the portal does not give comes from fontconfig's conf.d (/etc/fonts/conf.d,
// $XDG_CONFIG_HOME/fontconfig/conf.d): the links 10-hinting-{none,slight,medium,full}.conf,
// 10-sub-pixel-{rgb,bgr,vrgb,vbgr,none}.conf, 10-no-sub-pixel.conf, 10-no-antialias.conf /
// 10-yes-antialias.conf (jfgui_FONTCONFIG_SETTINGS=0 skips them).

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use crate::SysSettings;
use crate::linux_dbus::{Conn, Msg, Value};

const NAMESPACES: [&str; 5] = [
    "org.freedesktop.appearance",
    "org.gnome.desktop.interface",
    "org.gnome.desktop.a11y.interface",
    "org.gnome.desktop.peripherals.mouse",
    "org.gnome.desktop.wm.preferences",
];
const WM: &str = "org.gnome.desktop.wm.preferences";

// what the portal said (GOT_* bits): the files fill in the rest
const GOT_ANSWER: u32 = 1; // ReadAll was answered
const GOT_DARK: u32 = 2;
const GOT_ACCENT: u32 = 4;
const GOT_BUTTONS: u32 = 8;
const GOT_DOUBLE: u32 = 16;
const GOT_MIDDLE: u32 = 32;
const GOT_RIGHT: u32 = 64;
const GOT_ALL: u32 = 127;

/// the system's light / dark choice from the last read: 1 dark, 0 light, -1 unknown, -2 not read
static DARK: AtomicI32 = AtomicI32::new(-2);
/// the last read consulted the configuration files
static FILES: AtomicBool = AtomicBool::new(false);

/// (plat::theme) 1 dark, 0 light, -1 unknown: the answer of the last read of the system
/// settings, the portal's or the files' (-1 before the first)
pub(crate) fn portal_dark() -> i32 {
    DARK.load(Ordering::Relaxed).max(-1)
}

/// (linux_life.rs) the last read of the settings consulted the configuration files: a change
/// of them is a change of the settings
pub(crate) fn files_used() -> bool {
    FILES.load(Ordering::Relaxed)
}

/// the portal's settings into s, then the files for what it did not say
pub(crate) fn sys_settings_os(s: &mut SysSettings) {
    let got = portal(s);
    let files = got & GOT_ALL != GOT_ALL && !crate::env("jfgui_CONFIG_SETTINGS").is_some_and(|v| v.starts_with('0'));
    if files {
        config_files(s, got);
    }
    FILES.store(files, Ordering::Relaxed);
    DARK.store(s.dark.map_or(-1, |d| d as i32), Ordering::Relaxed);
    if !crate::env("jfgui_FONTCONFIG_SETTINGS").is_some_and(|v| v.starts_with('0')) {
        fontconfig_links(s);
    }
}

// what fontconfig's conf.d turns on (the portal's values stay)
fn fontconfig_links(s: &mut SysSettings) {
    let mut dirs = alloc::vec![String::from("/etc/fonts/conf.d")];
    match crate::env("XDG_CONFIG_HOME") {
        Some(c) if c.starts_with('/') => dirs.push(alloc::format!("{}/fontconfig/conf.d", c)),
        _ => {
            if let Some(h) = crate::env("HOME") {
                dirs.push(alloc::format!("{}/.config/fontconfig/conf.d", h));
            }
        }
    }
    let (mut aa, mut hint, mut rgba) = (None, None, None);
    for d in dirs {
        crate::dir_list(&d, &mut |name, _, _| {
            let Some(rest) = name.get(3..).filter(|_| name.as_bytes().first().is_some_and(u8::is_ascii_digit)) else { return };
            let Some(rest) = rest.strip_suffix(".conf") else { return };
            if let Some(h) = rest.strip_prefix("hinting-") {
                hint = crate::font_hint_parse(h).or(hint);
            } else if let Some(o) = rest.strip_prefix("sub-pixel-") {
                rgba = crate::font_rgba_parse(o).or(rgba);
            } else if rest == "no-sub-pixel" {
                rgba = Some(crate::FONT_RGBA_NONE);
            } else if rest == "no-antialias" {
                aa = Some(false);
            } else if rest == "yes-antialias" {
                aa = aa.or(Some(true));
            }
        });
    }
    s.font_aa = s.font_aa.or(aa);
    s.font_hinting = s.font_hinting.or(hint);
    s.font_rgba = s.font_rgba.or(rgba);
}

// the settings portal -> GOT_* bits of what it said
fn portal(s: &mut SysSettings) -> u32 {
    if crate::env("jfgui_PORTAL_SETTINGS").is_some_and(|v| v.starts_with('0')) {
        return 0;
    }
    let Some(mut c) = Conn::session() else { return 0 };
    let ns: Vec<Value> = NAMESPACES.iter().map(|n| Value::str(n)).collect();
    let call = Msg::call(
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
        "ReadAll",
        alloc::vec![Value::A(String::from("s"), ns)],
    );
    let Some(serial) = c.send(&call) else { return 0 };
    let first = DARK.load(Ordering::Relaxed) == -2;
    let Some(r) = c.wait_reply(serial, if first { 1000 } else { 300 }) else { return 0 };
    let Some(Value::A(_, spaces)) = r.body.first() else { return 0 };
    let (mut scheme, mut gtk_dark) = (None, false);
    let mut got = GOT_ANSWER;
    for e in spaces {
        let Value::E(k, v) = e else { continue };
        let (Some(ns), Value::A(_, keys)) = (k.as_str(), &**v) else { continue };
        for kv in keys {
            let Value::E(key, val) = kv else { continue };
            if let Some(key) = key.as_str() {
                let val = unwrap(val);
                match (ns, key) {
                    ("org.freedesktop.appearance", "color-scheme") => scheme = val.as_u32().or(scheme),
                    ("org.gnome.desktop.interface", "gtk-theme") => gtk_dark = val.as_str().is_some_and(theme_is_dark),
                    ("org.freedesktop.appearance", "accent-color") => got |= GOT_ACCENT,
                    (WM, "button-layout") => got |= GOT_BUTTONS,
                    (WM, "action-double-click-titlebar") => got |= GOT_DOUBLE,
                    (WM, "action-middle-click-titlebar") => got |= GOT_MIDDLE,
                    (WM, "action-right-click-titlebar") => got |= GOT_RIGHT,
                    _ => {}
                }
                apply(s, ns, key, val);
            }
        }
    }
    dark_choice(s, scheme, gtk_dark);
    if s.dark.is_some() {
        got |= GOT_DARK;
    }
    if s.accent.is_none() {
        got &= !GOT_ACCENT; // a value out of range
    }
    got
}

// ---------------------------------------------------------------- without the portal

// the [section] key=value pairs of an ini-style file (GTK's settings.ini, KDE's kdeglobals):
// f(section, key, value), the value trimmed and unquoted; keys with a locale or a KDE
// modifier ("Name[de]", "Key[$e]") come as they are
fn ini_each(path: &str, f: &mut dyn FnMut(&str, &str, &str)) -> bool {
    let Some(b) = crate::read_file_raw(path) else { return false };
    let text = String::from_utf8_lossy(&b);
    let mut sec = "";
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') || l.starts_with(';') {
            continue;
        }
        if let Some(n) = l.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            sec = n;
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        let v = v.trim();
        let v = v.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(v);
        f(sec, k.trim(), v);
    }
    true
}

// "r,g,b" (KDE; a fourth number, alpha, is ignored) or "#rrggbb" -> 0xRRGGBB
fn rgb_parse(v: &str) -> Option<u32> {
    if let Some(h) = v.strip_prefix('#') {
        return if h.len() == 6 { u32::from_str_radix(h, 16).ok() } else { None };
    }
    let c = v.split(',').map(|x| x.trim().parse::<u32>().ok().filter(|n| *n <= 255)).collect::<Option<Vec<u32>>>()?;
    if c.len() == 3 || c.len() == 4 { Some(c[0] << 16 | c[1] << 8 | c[2]) } else { None }
}

fn bool_parse(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

// one title bar action of a GTK file, unless the portal (or an earlier file) gave it
fn tb_action(val: &str, bit: u32, got: &mut u32, slot: &mut u8) {
    if *got & bit == 0 {
        if let Some(a) = crate::titlebar_action_parse(val) {
            *slot = a;
            *got |= bit;
        }
    }
}

/// what GTK's settings.ini and KDE's kdeglobals say about what the portal did not (got:
/// GOT_* bits), see the top of this file
pub(crate) fn config_files(s: &mut SysSettings, mut got: u32) {
    let Some(home) = crate::config_dir() else { return };
    for v in ["gtk-4.0", "gtk-3.0"] {
        let (mut prefer, mut theme): (Option<bool>, Option<bool>) = (None, None);
        let tb = &mut s.titlebar;
        let mut g = got;
        ini_each(&alloc::format!("{home}/{v}/settings.ini"), &mut |sec, k, val| {
            if sec != "Settings" {
                return;
            }
            match k {
                "gtk-application-prefer-dark-theme" => prefer = bool_parse(val).or(prefer),
                "gtk-theme-name" => theme = Some(theme_is_dark(val)),
                "gtk-decoration-layout" if g & GOT_BUTTONS == 0 => {
                    tb.buttons = Some(crate::titlebar_layout_parse(val));
                    g |= GOT_BUTTONS;
                }
                "gtk-titlebar-double-click" => tb_action(val, GOT_DOUBLE, &mut g, &mut tb.double),
                "gtk-titlebar-middle-click" => tb_action(val, GOT_MIDDLE, &mut g, &mut tb.middle),
                "gtk-titlebar-right-click" => tb_action(val, GOT_RIGHT, &mut g, &mut tb.right),
                _ => {}
            }
        });
        got = g;
        if got & GOT_DARK == 0 && (prefer.is_some() || theme.is_some()) {
            s.dark = Some(prefer == Some(true) || theme == Some(true));
            got |= GOT_DARK;
        }
    }
    if got & (GOT_DARK | GOT_ACCENT) == GOT_DARK | GOT_ACCENT {
        return;
    }
    // KDE: the window colour, else the scheme's name; the accent only where it belongs
    let (mut bg, mut scheme, mut accent) = (None, None, None);
    ini_each(&alloc::format!("{home}/kdeglobals"), &mut |sec, k, val| match (sec, k) {
        ("Colors:Window", "BackgroundNormal") => bg = rgb_parse(val),
        ("General", "ColorScheme") => scheme = Some(val.to_ascii_lowercase().contains("dark")),
        ("General", "AccentColor") => accent = rgb_parse(val),
        _ => {}
    });
    if got & GOT_DARK == 0 {
        if let Some(c) = bg {
            let l = (c >> 16 & 255) * 299 + (c >> 8 & 255) * 587 + (c & 255) * 114;
            s.dark = Some(l < 128 * 1000);
        } else if let Some(d) = scheme {
            s.dark = Some(d);
        }
    }
    let kde = crate::env("XDG_CURRENT_DESKTOP").is_some_and(|d| d.to_ascii_uppercase().split(':').any(|p| p == "KDE"));
    if got & GOT_ACCENT == 0 && (kde || got & GOT_ANSWER == 0) {
        if let Some(a) = accent {
            s.accent = Some(a);
        }
    }
}

/// a GTK theme name of a dark variant ("Yaru-dark", "Adwaita-dark", "Name:dark")
pub(crate) fn theme_is_dark(t: &str) -> bool {
    let l = t.to_ascii_lowercase();
    l.ends_with("-dark") || l.ends_with(":dark")
}

/// light or dark from color-scheme (None: the portal has none) and the GTK theme: 0 (no
/// preference) is light -- GNOME's "Default" style -- unless the theme is a dark one
pub(crate) fn dark_choice(s: &mut SysSettings, scheme: Option<u32>, gtk_dark: bool) {
    match scheme {
        Some(1) => s.dark = Some(true),
        Some(2) => s.dark = Some(false),
        Some(_) => s.dark = Some(gtk_dark || s.dark == Some(true)),
        None if gtk_dark => s.dark = Some(true),
        None => {}
    }
}

// a value inside its variant(s)
fn unwrap(mut v: &Value) -> &Value {
    while let Value::V(inner) = v {
        v = inner;
    }
    v
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::D(x) => Some(*x),
        Value::I(x) => Some(*x as f64),
        Value::U(x) => Some(*x as f64),
        _ => None,
    }
}

/// one setting into s
pub(crate) fn apply(s: &mut SysSettings, ns: &str, key: &str, v: &Value) {
    match (ns, key) {
        ("org.freedesktop.appearance", "color-scheme") => match v.as_u32() {
            Some(1) => s.dark = Some(true),
            Some(2) => s.dark = Some(false),
            _ => {}
        },
        ("org.freedesktop.appearance", "accent-color") => {
            if let Value::R(f) = v {
                let c: Vec<f64> = f.iter().filter_map(num).collect();
                if c.len() == 3 && c.iter().all(|x| (0.0..=1.0).contains(x)) {
                    let b = |x: f64| ((x * 255.0 + 0.5) as u32).min(255);
                    s.accent = Some(b(c[0]) << 16 | b(c[1]) << 8 | b(c[2]));
                }
            }
        }
        ("org.freedesktop.appearance", "contrast") => s.high_contrast |= v.as_u32() == Some(1),
        ("org.freedesktop.appearance", "reduced-motion") => s.reduce_motion |= v.as_u32() == Some(1),
        ("org.gnome.desktop.interface", "enable-animations") => s.reduce_motion |= matches!(v, Value::B(false)),
        ("org.gnome.desktop.interface", "text-scaling-factor") => {
            if let Some(f) = num(v) {
                s.text_scale = ((f * 100.0 + 0.5) as u32).clamp(50, 300);
            }
        }
        ("org.gnome.desktop.interface", "cursor-blink") => {
            if matches!(v, Value::B(false)) {
                s.caret_blink_ms = 0;
            }
        }
        ("org.gnome.desktop.interface", "cursor-blink-time") => {
            if let Some(ms) = num(v) {
                if s.caret_blink_ms != 0 {
                    s.caret_blink_ms = ((ms / 2.0) as u32).clamp(100, 2500);
                }
            }
        }
        ("org.gnome.desktop.interface", "gtk-theme") => {
            if let Some(t) = v.as_str() {
                if t.starts_with("HighContrast") {
                    s.high_contrast = true;
                    if t.ends_with("Inverse") {
                        s.dark = Some(true);
                    }
                }
            }
        }
        ("org.gnome.desktop.interface", "font-antialiasing") => match v.as_str() {
            Some("none") => s.font_aa = Some(false),
            Some("grayscale") => {
                s.font_aa = Some(true);
                s.font_rgba = Some(crate::FONT_RGBA_NONE);
            }
            Some("rgba") => {
                s.font_aa = Some(true);
                if s.font_rgba.is_none_or(|r| r == crate::FONT_RGBA_NONE) {
                    s.font_rgba = Some(crate::FONT_RGBA_RGB);
                }
            }
            _ => {}
        },
        ("org.gnome.desktop.interface", "font-hinting") => {
            if let Some(h) = v.as_str().and_then(crate::font_hint_parse) {
                s.font_hinting = Some(h);
            }
        }
        // the order counts only with subpixel smoothing (font-antialiasing rgba)
        ("org.gnome.desktop.interface", "font-rgba-order") => {
            if let Some(o) = v.as_str().and_then(crate::font_rgba_parse) {
                if s.font_rgba != Some(crate::FONT_RGBA_NONE) || o == crate::FONT_RGBA_NONE {
                    s.font_rgba = Some(o);
                }
            }
        }
        ("org.gnome.desktop.a11y.interface", "high-contrast") => s.high_contrast |= matches!(v, Value::B(true)),
        ("org.gnome.desktop.peripherals.mouse", "double-click") => {
            if let Some(ms) = num(v) {
                s.double_click_ms = (ms as u32).clamp(100, 5000);
            }
        }
        (WM, "button-layout") => {
            if let Some(l) = v.as_str() {
                s.titlebar.buttons = Some(crate::titlebar_layout_parse(l));
            }
        }
        (WM, "action-double-click-titlebar") => {
            if let Some(a) = v.as_str().and_then(crate::titlebar_action_parse) {
                s.titlebar.double = a;
            }
        }
        (WM, "action-middle-click-titlebar") => {
            if let Some(a) = v.as_str().and_then(crate::titlebar_action_parse) {
                s.titlebar.middle = a;
            }
        }
        (WM, "action-right-click-titlebar") => {
            if let Some(a) = v.as_str().and_then(crate::titlebar_action_parse) {
                s.titlebar.right = a;
            }
        }
        _ => {}
    }
}
