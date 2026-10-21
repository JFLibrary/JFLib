// linux_settings -- the system settings from the settings portal (settings.rs)
//
//   sys_settings_os(&mut s)   org.freedesktop.portal.Settings.ReadAll over the session bus
//                             (at most 300 ms; without the portal the defaults stay)
//
//   org.freedesktop.appearance       color-scheme (1 dark, 2 light), accent-color ((ddd)
//                                    0..1), contrast (1 high), reduced-motion (1)
//   org.gnome.desktop.interface      enable-animations, text-scaling-factor, cursor-blink,
//                                    cursor-blink-time (a whole cycle), gtk-theme
//                                    (HighContrast*), font-antialiasing (none / grayscale /
//                                    rgba), font-hinting, font-rgba-order
//   org.gnome.desktop.a11y.interface high-contrast
//   org.gnome.desktop.peripherals.mouse double-click
//
// jfgui_PORTAL_SETTINGS=0 skips the portal.  Font rendering the portal does not give comes from
// fontconfig's conf.d (/etc/fonts/conf.d, $XDG_CONFIG_HOME/fontconfig/conf.d): the links
// 10-hinting-{none,slight,medium,full}.conf, 10-sub-pixel-{rgb,bgr,vrgb,vbgr,none}.conf,
// 10-no-sub-pixel.conf, 10-no-antialias.conf / 10-yes-antialias.conf (jfgui_FONTCONFIG_SETTINGS=0
// skips them).

use alloc::string::String;
use alloc::vec::Vec;

use crate::SysSettings;
use crate::linux_dbus::{Conn, Msg, Value};

const NAMESPACES: [&str; 4] =
    ["org.freedesktop.appearance", "org.gnome.desktop.interface", "org.gnome.desktop.a11y.interface", "org.gnome.desktop.peripherals.mouse"];

/// the portal's settings into s
pub(crate) fn sys_settings_os(s: &mut SysSettings) {
    portal(s);
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

// the settings portal
fn portal(s: &mut SysSettings) {
    if crate::env("jfgui_PORTAL_SETTINGS").is_some_and(|v| v.starts_with('0')) {
        return;
    }
    let Some(mut c) = Conn::session() else { return };
    let ns: Vec<Value> = NAMESPACES.iter().map(|n| Value::str(n)).collect();
    let call = Msg::call(
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
        "ReadAll",
        alloc::vec![Value::A(String::from("s"), ns)],
    );
    let Some(serial) = c.send(&call) else { return };
    let Some(r) = c.wait_reply(serial, 300) else { return };
    let Some(Value::A(_, spaces)) = r.body.first() else { return };
    for e in spaces {
        let Value::E(k, v) = e else { continue };
        let (Some(ns), Value::A(_, keys)) = (k.as_str(), &**v) else { continue };
        for kv in keys {
            let Value::E(key, val) = kv else { continue };
            if let Some(key) = key.as_str() {
                apply(s, ns, key, unwrap(val));
            }
        }
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
        _ => {}
    }
}
