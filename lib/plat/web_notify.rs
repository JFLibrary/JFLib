// web_notify -- (tray.rs) system notifications in the browser and on Android; the symbol in a
// notification area does not exist there (tray_set: false)
//
//   notify(id, &Note) -> bool      show it -> false: not possible (no permission, no API)
//   notify_close(id)               take it away
//
// Browser (web/notify.js): REQUEST_NOTIFY to the page, which shows it with the Notification
// API.  Without the API or with the permission denied (jfgui_system notify=..) nothing is
// sent and notify answers false; while the user was not asked yet (notify=default) the page asks
// first -- Notification.requestPermission from the user's gesture that led to the call (the
// request arrives within the gesture's activation) -- and shows it once allowed; refused, the
// note goes away at once (NoteClosed).  A click focuses the tab (window.focus) and reports
// NoteClicked, closing it NoteClosed (jfgui_note, web_system.rs).
// Android (android/GalleryNotify.java): CALL_NOTIFY over jfgui_host_call; NotificationManager
// with the channel "jfgui", POST_NOTIFICATIONS asked for on API 33+ (the note waits for the
// answer, refused: NoteClosed); a tap opens the activity (onNewIntent) and reports NoteClicked,
// swiping it away NoteClosed.
//
// REQUEST_NOTIFY / CALL_NOTIFY payload (little endian): u32 op (1 show, 2 close), u32 id; show:
// title and body (u32 length + UTF-8 each), u32 w, u32 h (0 x 0: the app's icon), then w * h
// pixels (browser: R, G, B, A bytes; Android: 0xAARRGGBB words), straight alpha.

use alloc::vec::Vec;

use crate::tray::{Note, Tray};

/// (browser) a notification for the page: see the module header; no completion
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const REQUEST_NOTIFY: u32 = 24;
// (Android) the host call of GalleryNotify (android_host.rs: CALL_*)
#[cfg(target_os = "android")]
const CALL_NOTIFY: u32 = 50;

const OP_SHOW: u32 = 1;
const OP_CLOSE: u32 = 2;
// what a notification may carry
const TITLE_MAX: usize = 1024;
const BODY_MAX: usize = 8192;

fn cut(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut n = max;
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    &s[..n]
}

fn payload(op: u32, id: u32, n: Option<&Note>) -> Vec<u8> {
    let mut p = Vec::new();
    p.extend_from_slice(&op.to_le_bytes());
    p.extend_from_slice(&id.to_le_bytes());
    let Some(n) = n else { return p };
    for (s, max) in [(n.title.as_str(), TITLE_MAX), (n.body.as_str(), BODY_MAX)] {
        let s = cut(s, max);
        p.extend_from_slice(&(s.len() as u32).to_le_bytes());
        p.extend_from_slice(s.as_bytes());
    }
    let icon = n.icon.as_ref().filter(|i| i.w > 0 && i.h > 0 && i.w <= 256 && i.h <= 256 && i.px.len() == (i.w * i.h) as usize);
    let (w, h) = icon.map_or((0, 0), |i| (i.w, i.h));
    p.extend_from_slice(&w.to_le_bytes());
    p.extend_from_slice(&h.to_le_bytes());
    if let Some(i) = icon {
        for &c in &i.px {
            if cfg!(target_os = "android") {
                p.extend_from_slice(&c.to_le_bytes());
            } else {
                p.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8, (c >> 24) as u8]);
            }
        }
    }
    p
}

pub fn tray_set(_t: &Tray) -> bool {
    false
}

pub fn tray_remove() {}

pub fn supported() -> bool {
    false
}

#[cfg(target_arch = "wasm32")]
pub fn notify(id: u32, n: &Note) -> bool {
    use crate::web::system_host::{NOTIFY_DEFAULT, NOTIFY_GRANTED, notify_state};
    if !matches!(notify_state(), NOTIFY_GRANTED | NOTIFY_DEFAULT) {
        return false;
    }
    crate::web::notice(REQUEST_NOTIFY, &payload(OP_SHOW, id, Some(n)));
    true
}

#[cfg(target_arch = "wasm32")]
pub fn notify_close(id: u32) {
    crate::web::notice(REQUEST_NOTIFY, &payload(OP_CLOSE, id, None));
}

#[cfg(target_os = "android")]
pub fn notify(id: u32, n: &Note) -> bool {
    crate::web::android_host::call(CALL_NOTIFY, &payload(OP_SHOW, id, Some(n))).is_some_and(|r| r.first() == Some(&1))
}

#[cfg(target_os = "android")]
pub fn notify_close(id: u32) {
    let _ = crate::web::android_host::call(CALL_NOTIFY, &payload(OP_CLOSE, id, None));
}
