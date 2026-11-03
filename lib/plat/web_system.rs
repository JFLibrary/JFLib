// web_system -- what the browser / Android host says about the system: the user's settings
// (settings.rs), the permission for notifications and whether the screen can be kept on
// (web_notify.rs, web_power.rs), the answers of the notifications
//
//   jfgui_system(len) -> count   the inbox: "key=value" lines -> how many keys were taken; a
//                                key that is missing keeps its value.  The host sends all of
//                                them before jfgui_start and the changed ones after every
//                                change; once the app runs, a change of what sys_settings()
//                                answers is E_SETTINGS with its SET_* bits (dark, accent:
//                                SET_THEME; contrast, forced, colors: SET_CONTRAST; scale:
//                                SET_FONTS; motion: SET_OTHER).  Keys:
//     dark=1|0|-        a dark / light scheme, "-" no preference (prefers-color-scheme;
//                       Android: Configuration.uiMode, UI_MODE_NIGHT_YES)
//     contrast=1|0      more contrast (prefers-contrast: more; Android: high-contrast text, the
//                       contrast level of API 34 at "high")
//     forced=1|0        the browser's forced colours (forced-colors: active, Windows' contrast
//                       themes in the browser): high contrast with the colours of `colors`
//     colors=RRGGBB,..  eight system colours of the forced scheme: Canvas, CanvasText,
//                       Highlight, HighlightText, ButtonFace, ButtonText, GrayText, LinkText
//                       (HcColors in that order); empty: none
//     motion=1|0        fewer animations (prefers-reduced-motion: reduce; Android:
//                       Settings.Global.ANIMATOR_DURATION_SCALE 0)
//     scale=PERCENT     the text size (the browser's default font size against 16 px;
//                       Android: Configuration.fontScale)
//     accent=RRGGBB|-   the system's accent colour (Android 12+: the dynamic colours,
//                       system_accent1_600); "-" none
//     notify=granted|default|denied|unsupported   (browser) the Notification permission:
//                       default = asked at the first notification (from the user's gesture)
//     wakelock=1|0      (browser) navigator.wakeLock exists (Screen Wake Lock)
//   jfgui_note(id, clicked) -> 1   a notification of tray::notify was clicked (1; the host
//                                brought the tab / the activity to the front) or went away
//                                unclicked (0: closed, swiped away, refused) -> TrayEvent
//   sys_settings_host(&mut s)    (settings.rs) the host's values; the jfgui_* overrides follow
//   dark() -> Option<bool>       (web.rs theme())
//   notify_state() / wakelock()  (web_notify.rs, web_power.rs)
//   back_trap(on)                (Android) the back key is Esc for the app while on (lib.rs)
//   jfgui_touch_reached() -> 0|1  (Android) the last finger's gesture reached a widget (a tap, a
//                                drag), not only a scroll: the keyboard may follow it
//
// The lifecycle needs no export of its own: the host sends E_POWER through jfgui_event (data 2:
// the tab was hidden / Android's onStop, 3: visible / started again) and the runtime turns it
// into UIC_SUSPEND.

use core::sync::atomic::{AtomicU8, Ordering};

use rt::sync::SpinLock;

use crate::{HcColors, SysSettings, SET_CONTRAST, SET_FONTS, SET_OTHER, SET_THEME};

/// notify=..: the browser's permission for notifications
pub(crate) const NOTIFY_UNSUPPORTED: u8 = 0;
pub(crate) const NOTIFY_DEFAULT: u8 = 1;
pub(crate) const NOTIFY_GRANTED: u8 = 2;
pub(crate) const NOTIFY_DENIED: u8 = 3;

#[derive(Clone, Copy, PartialEq)]
struct Host {
    dark: Option<bool>,
    contrast: bool,
    forced: bool,
    colors: Option<HcColors>,
    motion: bool,
    scale: u32,
    accent: Option<u32>,
}

static HOST: SpinLock<Host> =
    SpinLock::new(Host { dark: None, contrast: false, forced: false, colors: None, motion: false, scale: 100, accent: None });
// the browser tells (notify=); Android's host asks at every notification itself
static NOTIFY: AtomicU8 = AtomicU8::new(if cfg!(target_os = "android") { NOTIFY_DEFAULT } else { NOTIFY_UNSUPPORTED });
static WAKELOCK: AtomicU8 = AtomicU8::new(0);

fn hex(v: &str) -> Option<u32> {
    let v = v.trim().trim_start_matches('#');
    if v.len() != 6 {
        return None;
    }
    u32::from_str_radix(v, 16).ok()
}

// colors=: eight RRGGBB values in HcColors' order
fn colors(v: &str) -> Option<HcColors> {
    let mut c = [0u32; 8];
    let mut it = v.split(',');
    for x in c.iter_mut() {
        *x = hex(it.next()?)?;
    }
    if it.next().is_some() {
        return None;
    }
    Some(HcColors { bg: c[0], text: c[1], highlight: c[2], highlight_text: c[3], button: c[4], button_text: c[5], disabled: c[6], link: c[7] })
}

// the SET_* bits of the difference
fn bits(a: &Host, b: &Host) -> u32 {
    let mut m = 0;
    if a.dark != b.dark || a.accent != b.accent {
        m |= SET_THEME;
    }
    if a.contrast != b.contrast || a.forced != b.forced || (b.forced && a.colors != b.colors) {
        m |= SET_CONTRAST;
    }
    if a.scale != b.scale {
        m |= SET_FONTS;
    }
    if a.motion != b.motion {
        m |= SET_OTHER;
    }
    m
}

/// The host's system settings and states (inbox: "key=value" lines, see the module header)
/// -> the keys taken.  A change once the app runs is E_SETTINGS.
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_system(len: u32) -> u32 {
    let Some(bytes) = super::input(len) else { return 0 };
    let Ok(text) = core::str::from_utf8(&bytes) else { return 0 };
    let mut taken = 0;
    let (before, after) = {
        let mut h = HOST.lock();
        let before = *h;
        for line in text.lines() {
            let Some((key, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            let flag = || match v {
                "1" => Some(true),
                "0" => Some(false),
                _ => None,
            };
            let ok = match key.trim() {
                "dark" => {
                    h.dark = flag();
                    flag().is_some() || v == "-" || v.is_empty()
                }
                "contrast" => flag().map(|f| h.contrast = f).is_some(),
                "forced" => flag().map(|f| h.forced = f).is_some(),
                "colors" => {
                    h.colors = colors(v);
                    h.colors.is_some() || v.is_empty()
                }
                "motion" => flag().map(|f| h.motion = f).is_some(),
                "scale" => v.parse::<u32>().ok().filter(|p| (25..=500).contains(p)).map(|p| h.scale = p).is_some(),
                "accent" => {
                    h.accent = hex(v);
                    h.accent.is_some() || v == "-" || v.is_empty()
                }
                "notify" => {
                    let s = match v {
                        "granted" => Some(NOTIFY_GRANTED),
                        "default" | "prompt" => Some(NOTIFY_DEFAULT),
                        "denied" => Some(NOTIFY_DENIED),
                        "unsupported" => Some(NOTIFY_UNSUPPORTED),
                        _ => None,
                    };
                    s.map(|s| NOTIFY.store(s, Ordering::Relaxed)).is_some()
                }
                "wakelock" => flag().map(|f| WAKELOCK.store(f as u8, Ordering::Relaxed)).is_some(),
                _ => false,
            };
            taken += ok as u32;
        }
        (before, *h)
    };
    let changed = bits(&before, &after);
    if changed != 0 && super::STARTED.load(Ordering::Relaxed) {
        let _ = crate::evq_push(crate::Event { typ: crate::E_SETTINGS, data: changed, ..crate::Event::default() });
        super::wake();
    }
    taken
}

/// A notification of tray::notify was clicked (clicked 1) or went away without a click (0)
/// -> 1 taken (0: the app has not started).
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_note(id: u32, clicked: u32) -> u32 {
    if id == 0 || !super::STARTED.load(Ordering::Relaxed) {
        return 0;
    }
    crate::tray::push(if clicked != 0 { crate::tray::TrayEvent::NoteClicked(id) } else { crate::tray::TrayEvent::NoteClosed(id) });
    1
}

/// (settings.rs) the host's settings: scheme, accent, contrast (the forced colours with their
/// scheme), animations, text scale
pub(crate) fn sys_settings_host(s: &mut SysSettings) {
    let h = *HOST.lock();
    s.dark = h.dark;
    s.accent = h.accent;
    s.high_contrast = h.contrast || h.forced;
    s.contrast_colors = if h.forced { h.colors } else { None };
    s.reduce_motion = h.motion;
    s.text_scale = h.scale.clamp(50, 300);
}

// (Android) the host call of GallerySystem (android_host.rs: CALL_*)
#[cfg(target_os = "android")]
const CALL_BACK: u32 = 55;
#[cfg(target_os = "android")]
const CALL_LONG_PRESS: u32 = 56;
#[cfg(target_os = "android")]
const CALL_FINISH: u32 = 57;

/// (Android) while on, the back key reaches the app as Esc instead of ending the activity
#[cfg(target_os = "android")]
pub(crate) fn back_trap(on: bool) {
    let _ = crate::web::android_host::call(CALL_BACK, &[on as u8]);
}

/// (Android) the runtime ended (ui.quit): the activity goes and its process with it, a new start
/// needs a new runtime
#[cfg(target_os = "android")]
pub(crate) fn app_finish() {
    let _ = crate::web::android_host::call(CALL_FINISH, &[]);
}

// (Android) the finger's gesture reached a widget: plat::touch_reached, jfgui_touch_reached
#[cfg(target_os = "android")]
static TOUCH_REACHED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

#[cfg(target_os = "android")]
pub(crate) fn touch_reached(reached: bool) {
    TOUCH_REACHED.store(reached, Ordering::Relaxed);
}

/// (Android) the last finger's gesture reached a widget -> 1
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_touch_reached() -> u32 {
    TOUCH_REACHED.load(Ordering::Relaxed) as u32
}

/// (Android) a finger rested long enough: GalleryActivity gives the system's feedback
#[cfg(target_os = "android")]
pub(crate) fn touch_long_press() {
    let _ = crate::web::android_host::call(CALL_LONG_PRESS, &[]);
}

/// the host's scheme: Some(true) dark, Some(false) light, None no preference
pub(crate) fn dark() -> Option<bool> {
    HOST.lock().dark
}

/// NOTIFY_*: what the browser's permission for notifications is (Android: NOTIFY_DEFAULT,
/// the host asks itself)
#[cfg_attr(target_os = "android", allow(dead_code))]
pub(crate) fn notify_state() -> u8 {
    NOTIFY.load(Ordering::Relaxed)
}

/// (browser) the host can keep the screen on (Screen Wake Lock)
#[allow(dead_code)]
pub(crate) fn wakelock() -> bool {
    WAKELOCK.load(Ordering::Relaxed) != 0
}
