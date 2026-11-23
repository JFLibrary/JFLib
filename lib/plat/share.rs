// share -- the system's share sheet: hand a text, a link or files to another app (mail, messages,
// AirDrop, Nearby Share, a chat ...)
//
//   share(&Share) -> i32          SHARE_PENDING: the sheet is open, its answer comes through
//                                 share_poll (and wakes the UI); SHARE_NONE: not here -- no sheet on
//                                 this system (Linux), no window, nothing to share, a file the app
//                                 may not read or that is not there, one sheet open already
//   share_poll() -> Option<i32>   None while it is open (or none was), then once: SHARE_DONE (a
//                                 target took the items), SHARE_CANCELLED (closed without one) or
//                                 SHARE_FAILED (the target or the system refused)
//   share_open() -> bool          a sheet is open (its answer not taken yet)
//   CAP_SHARE                     capabilities(): this system has a share sheet
//
// Share { title, text, url, files, anchor }: `title` the subject (a mail's subject line, Windows'
// title of the data; may be empty), `text` plain text, `url` a link (http, https, mailto ...),
// `files` paths of the file system (the browser: of its virtual files) -- at least one of text, url,
// files.  `anchor` [x, y, w, h] in the window's pixels: what the sheet points at where it is a
// popover (macOS, an iPad); empty: the window's middle (macOS, iPad) / its usual place.
//
// What "done" means differs (it says nothing about delivery): iOS and macOS a service was chosen
// and took the items; Windows the target app took them (TargetApplicationChosen); the browser
// navigator.share resolved; Android a target was chosen (the chooser's callback, API 22+).
//
// Windows (win_share.rs): DataTransferManager of the active window through
// IDataTransferManagerInterop (Windows 10), the data package filled in DataRequested (text, a web
// link, storage items of the files).  macOS (apple_mac_share.rs): NSSharingServicePicker at the
// anchor in the key window.  iOS (apple_ios_share.rs): UIActivityViewController over the scene's
// window, on an iPad a popover at the anchor.  Browser and Android (web_share.rs): the host --
// navigator.share (files as File objects; the browser asks for the user's gesture: call it while
// handling input) / Intent.ACTION_SEND(_MULTIPLE) in a chooser (files through the host's provider,
// readable by the target only).  Linux: none.

use alloc::string::String;
use alloc::vec::Vec;

/// the sheet is open; the answer comes through share_poll
pub const SHARE_PENDING: i32 = 2;
/// a target took the items
pub const SHARE_DONE: i32 = 1;
/// closed without a target
pub const SHARE_CANCELLED: i32 = 0;
/// not here: no sheet, no window, nothing to share, a file not readable, one open already
pub const SHARE_NONE: i32 = -1;
/// the target or the system refused
pub const SHARE_FAILED: i32 = -2;

/// what to share (see the module comment)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Share {
    pub title: String,
    pub text: String,
    pub url: String,
    pub files: Vec<String>,
    /// [x, y, w, h] in the window's pixels (all 0: none)
    pub anchor: [i32; 4],
}

impl Share {
    /// a text to share
    pub fn text(text: &str) -> Share {
        Share { text: String::from(text), ..Share::default() }
    }
    /// a link to share
    pub fn url(url: &str) -> Share {
        Share { url: String::from(url), ..Share::default() }
    }
    /// files to share
    pub fn files(files: &[&str]) -> Share {
        Share { files: files.iter().map(|f| String::from(*f)).collect(), ..Share::default() }
    }
    /// nothing to share
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.url.is_empty() && self.files.is_empty()
    }
}

// the answer of the open sheet (backends: answer)
static ANSWER: rt::sync::SpinLock<Option<i32>> = rt::sync::SpinLock::new(None);
static OPEN: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// (backends) the sheet's answer: kept for share_poll, the UI woken
#[allow(dead_code)]
pub(crate) fn answer(r: i32) {
    *ANSWER.lock() = Some(r);
    crate::wake();
}

/// open the system's share sheet with these items -> SHARE_PENDING / SHARE_NONE
pub fn share(s: &Share) -> i32 {
    if s.is_empty() || OPEN.load(core::sync::atomic::Ordering::Acquire) {
        return SHARE_NONE;
    }
    // the files: there, and readable by the app (file_exists asks its permissions, policy.rs)
    let mut files: Vec<String> = Vec::with_capacity(s.files.len());
    for f in &s.files {
        let f = f.replace('\\', "/");
        if f.is_empty() || crate::file_exists(&f) != 1 {
            return SHARE_NONE;
        }
        files.push(f);
    }
    let s = Share { title: clean(&s.title, 256), text: clean(&s.text, 1 << 20), url: clean(&s.url, 8192), files, anchor: s.anchor };
    *ANSWER.lock() = None;
    OPEN.store(true, core::sync::atomic::Ordering::Release);
    let r = native::share(&s);
    if r != SHARE_PENDING {
        OPEN.store(false, core::sync::atomic::Ordering::Release);
        return SHARE_NONE;
    }
    r
}

/// the open sheet's answer: None while open (or none was), then once SHARE_DONE /
/// SHARE_CANCELLED / SHARE_FAILED
pub fn share_poll() -> Option<i32> {
    let r = ANSWER.lock().take()?;
    OPEN.store(false, core::sync::atomic::Ordering::Release);
    Some(r)
}

/// a sheet is open (its answer not taken yet)
pub fn share_open() -> bool {
    OPEN.load(core::sync::atomic::Ordering::Acquire)
}

/// whether this system has a share sheet (CAP_SHARE)
pub(crate) fn available() -> bool {
    native::available()
}

/// no NULs, at most n bytes (on a character boundary)
fn clean(s: &str, n: usize) -> String {
    let mut out: String = s.chars().filter(|&c| c != '\0').collect();
    if out.len() > n {
        let mut k = n;
        while !out.is_char_boundary(k) {
            k -= 1;
        }
        out.truncate(k);
    }
    out
}

#[cfg(windows)]
#[path = "win_share.rs"]
mod native;

#[cfg(target_os = "macos")]
#[path = "apple_mac_share.rs"]
mod native;

#[cfg(target_os = "ios")]
#[path = "apple_ios_share.rs"]
mod native;

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
#[path = "web_share.rs"]
mod native;

#[cfg(not(any(windows, target_os = "macos", target_os = "ios", target_arch = "wasm32", target_os = "android")))]
mod native {
    use super::{SHARE_NONE, Share};
    pub fn share(_s: &Share) -> i32 {
        SHARE_NONE
    }
    pub fn available() -> bool {
        false
    }
}
