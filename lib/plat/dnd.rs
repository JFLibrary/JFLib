// dnd -- drag and drop between programs: what a drag carries, the target hook, the pending
// drop and the app's own outgoing drag (the parts every OS shares)       (docs/GUIDE.md §7)
//
//   DT_FILES / DT_TEXT / DT_URL / DT_HTML / DT_IMAGE   kinds of content (bits, DT_STD = all);
//                                DT_USER0 << k: the app's own kinds (internal drags only)
//   DE_COPY / DE_MOVE / DE_LINK  effects (bits)
//   DragData                     the content of one drag (paths, text, URLs, HTML, an image, the
//                                app's own bytes) and the effects
//   drag_hook(Some(f))           f(pw, x, y, types) -> (accepted types, accepted effects): what the
//                                UI takes at a point (px) of platform window pw.  The OS layers
//                                ask it synchronously while something is dragged over a window
//                                (IDropTarget::DragOver, XdndPosition, wl_data_device.motion).
//   drag_query(pw, x, y, types, allowed, mods) -> (types, effect)   (OS layers) the hook's answer
//                                with one effect picked; without a hook: files, copy (as before).
//                                Files are never moved: a source deletes what it moved, and
//                                the app gets only the paths (copy or link instead)
//   drag_effect_pick(allowed, mods) -> effect   Ctrl copy, Shift move, Ctrl+Shift link; else
//                                the first of copy / move / link the mask allows
//   drop_data_take() -> DragData the pending drop of E_DROP: the paths of drop_take and the rest
//   drop_set_text / drop_set_html / drop_add_url / drop_set_image / drop_set_effect
//                                (OS layers) fill the pending drop (drop_reset empties it)
//   drop_text_from(bytes, utf16) / drop_url_list(text)   decoding helpers of the OS layers
//   drag_start(data) -> bool     drag data out of the app while the left button is held (the UI
//                                calls it when an internal drag leaves its window); false when
//                                the platform cannot (headless, browser) or a drag runs.
//                                E_DRAG_END follows (data = the effect, 0 = cancelled / refused).
//   drag_running() -> bool       an outgoing drag runs
//
// Events: E_DRAG (x, y = position in window px, data = DT_* offered | DE_* allowed << 8; data 0:
// the drag left the window, was dropped or cancelled) while another program's drag (or our
// own outgoing one) is over a window; E_DROP (x, y, data = the number of paths) when it is
// dropped there; E_DRAG_END when our own outgoing drag ended.
// Windows: OLE (win_dnd.rs: IDropTarget per window, DoDragDrop with our IDataObject / IDropSource);
// X11: Xdnd 5 target and source (linux_x11_dnd.rs); Wayland: wl_data_device (linux_wl_clip.rs).
// Everything that comes from another program is bounded (sizes, counts) and checked.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use rt::sync::SpinLock;

// ---- kinds of content (bits)
pub const DT_FILES: u32 = 1; // paths (file:// URIs, CF_HDROP)
pub const DT_TEXT: u32 = 2; // UTF-8 text
pub const DT_URL: u32 = 4; // URLs that are not local files
pub const DT_HTML: u32 = 8; // a UTF-8 HTML fragment
pub const DT_IMAGE: u32 = 16; // an image file (PNG; a DIB arrives as BMP)
pub const DT_STD: u32 = 31; // every kind other programs know
pub const DT_USER0: u32 = 1 << 8; // the app's own kinds: DT_USER0 << k, k = 0..23

// ---- effects (bits)
pub const DE_COPY: u32 = 1; // the target copies (DROPEFFECT_COPY, XdndActionCopy)
pub const DE_MOVE: u32 = 2; // the target moves: the source deletes its original
pub const DE_LINK: u32 = 4; // the target links to it (not on Wayland)
pub const DE_ALL: u32 = 7; // every effect

const TEXT_MAX: usize = crate::CLIP_RICH_MAX; // text / HTML of one drop
const IMAGE_MAX: usize = 1 << 28; // one image file
const URL_MAX: usize = 32768; // bytes of one URL
const URLS_MAX: usize = 4096;
const URL_BYTES_MAX: usize = 1 << 24;

/// the content of one drag (a drop that arrives, or what the app drags)
#[derive(Clone, Default, Debug, PartialEq)]
pub struct DragData {
    /// paths of files and folders
    pub files: Vec<String>,
    /// plain text
    pub text: Option<String>,
    /// URLs (not local files)
    pub urls: Vec<String>,
    /// an HTML fragment (UTF-8)
    pub html: Option<String>,
    /// an image file: PNG (a drop of a DIB / image/bmp arrives as a BMP file)
    pub image: Option<Vec<u8>>,
    /// (drag out) the same image as a BMP file, offered as DIB / image/bmp too (optional)
    pub bmp: Option<Vec<u8>>,
    /// the app's own kind (DT_USER0 << k bits) and bytes: internal drags only
    pub app: u32,
    pub bytes: Vec<u8>,
    /// what the drag shows next to the pointer inside the app (empty: from the content)
    pub label: String,
    /// drag out: the DE_* effects the source allows (0 = copy | move); a drop: the effect chosen
    pub effects: u32,
}

impl DragData {
    /// the DT_* kinds it carries (the app's own kinds included)
    pub fn types(&self) -> u32 {
        let mut t = self.app & !DT_STD;
        if !self.files.is_empty() {
            t |= DT_FILES;
        }
        if self.text.is_some() {
            t |= DT_TEXT;
        }
        if !self.urls.is_empty() {
            t |= DT_URL;
        }
        if self.html.is_some() {
            t |= DT_HTML;
        }
        if self.image.is_some() {
            t |= DT_IMAGE;
        }
        t
    }

    /// nothing to drag
    pub fn is_empty(&self) -> bool {
        self.types() == 0
    }

    /// text only
    pub fn of_text(s: &str) -> DragData {
        DragData { text: Some(String::from(s)), ..DragData::default() }
    }

    /// paths only
    pub fn of_files(paths: &[&str]) -> DragData {
        DragData { files: paths.iter().map(|p| String::from(*p)).collect(), ..DragData::default() }
    }

    /// the app's own kind (DT_USER0 << k) and bytes
    pub fn of_app(kind: u32, bytes: &[u8]) -> DragData {
        DragData { app: kind & !DT_STD, bytes: bytes.to_vec(), ..DragData::default() }
    }

    /// the effects a drag out allows (0 means copy | move)
    pub fn allowed(&self) -> u32 {
        if self.effects & DE_ALL == 0 { DE_COPY | DE_MOVE } else { self.effects & DE_ALL }
    }
}

// ---------------------------------------------------------------- the target hook

/// what the UI takes at a point: f(platform window, x, y px, offered DT_*) -> (accepted DT_*,
/// accepted DE_*)
pub type DragHook = fn(i32, i32, i32, u32) -> (u32, u32);

static HOOK: AtomicUsize = AtomicUsize::new(0);

/// install (Some) or remove (None) the drop target hook (the UI does it for its windows)
pub fn drag_hook(f: Option<DragHook>) {
    HOOK.store(f.map_or(0, |f| f as usize), Ordering::Release);
}

/// the effect of a drop: the modifier keys choose (Ctrl copy, Shift move, Ctrl+Shift link),
/// else the first of copy / move / link that `allowed` has (0: none)
pub fn drag_effect_pick(allowed: u32, mods: u8) -> u32 {
    let a = allowed & DE_ALL;
    let ctrl = mods & crate::MOD_CTRL != 0;
    let shift = mods & crate::MOD_SHIFT != 0;
    let want = match (ctrl, shift) {
        (true, true) => DE_LINK,
        (true, false) => DE_COPY,
        (false, true) => DE_MOVE,
        _ => 0,
    };
    if want & a != 0 {
        return want;
    }
    [DE_COPY, DE_MOVE, DE_LINK].into_iter().find(|e| a & e != 0).unwrap_or(0)
}

/// (OS layers) what platform window pw takes at (x, y) px of a drag offering `types` with the
/// effects `allowed` (0 = copy | move) -> (accepted types, the effect); (0, 0) = refused
pub fn drag_query(pw: i32, x: i32, y: i32, types: u32, allowed: u32, mods: u8) -> (u32, u32) {
    let h = HOOK.load(Ordering::Acquire);
    let (t, e) = if h == 0 {
        (types & DT_FILES, DE_ALL) // no UI: files, as before
    } else {
        // SAFETY: HOOK only ever holds 0 or a DragHook stored by drag_hook.
        let f: DragHook = unsafe { core::mem::transmute::<usize, DragHook>(h) };
        f(pw, x, y, types)
    };
    let t = t & types;
    let mut allowed = if allowed & DE_ALL == 0 { DE_COPY | DE_MOVE } else { allowed & DE_ALL };
    if t & DT_FILES != 0 {
        // a move would make the source delete the files: the app gets only their paths
        allowed &= !DE_MOVE;
    }
    let eff = drag_effect_pick(e & allowed, mods);
    if t == 0 || eff == 0 { (0, 0) } else { (t, eff) }
}

// ---------------------------------------------------------------- the pending drop

struct Rich {
    text: Option<String>,
    html: Option<String>,
    urls: Vec<String>,
    url_bytes: usize,
    image: Option<Vec<u8>>,
    effect: u32,
}

static RICH: SpinLock<Rich> =
    SpinLock::new(Rich { text: None, html: None, urls: Vec::new(), url_bytes: 0, image: None, effect: 0 });

/// (drop_reset) forget the pending text, HTML, URLs and image
pub(crate) fn rich_reset() {
    let mut r = RICH.lock();
    r.text = None;
    r.html = None;
    r.urls = Vec::new();
    r.url_bytes = 0;
    r.image = None;
    r.effect = 0;
}

// text of another program: UTF-8 (invalid bytes replaced), up to the first NUL, bounded
fn text_of(bytes: &[u8]) -> Option<String> {
    let end = bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len());
    if end > TEXT_MAX {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
}

/// (OS layers) the pending drop's text (UTF-8 bytes; cut at a NUL) -> false when too large
pub fn drop_set_text(bytes: &[u8]) -> bool {
    let Some(s) = text_of(bytes) else { return false };
    RICH.lock().text = Some(s);
    true
}

/// (OS layers) the pending drop's HTML fragment (UTF-8 bytes) -> false when too large
pub fn drop_set_html(bytes: &[u8]) -> bool {
    let Some(s) = text_of(bytes) else { return false };
    RICH.lock().html = Some(s);
    true
}

/// (OS layers) one URL of the pending drop ("scheme:...", no blanks or control characters,
/// bounded, no duplicates) -> added
pub fn drop_add_url(url: &str) -> bool {
    let url = url.trim();
    if url.is_empty() || url.len() > URL_MAX || url.chars().any(|c| c.is_control() || c.is_whitespace()) || !has_scheme(url) {
        return false;
    }
    let mut r = RICH.lock();
    if r.urls.len() >= URLS_MAX || r.url_bytes + url.len() > URL_BYTES_MAX || r.urls.iter().any(|u| u == url) {
        return false;
    }
    r.url_bytes += url.len();
    r.urls.push(String::from(url));
    true
}

/// (OS layers) the pending drop's image file (PNG or BMP bytes) -> false when too large
pub fn drop_set_image(bytes: Vec<u8>) -> bool {
    if bytes.is_empty() || bytes.len() > IMAGE_MAX {
        return false;
    }
    RICH.lock().image = Some(bytes);
    true
}

/// (OS layers) the effect the drop was made with (DE_*)
pub fn drop_set_effect(effect: u32) {
    RICH.lock().effect = effect & DE_ALL;
}

/// the pending drop (E_DROP): its paths (drop_take) and the rest; both start empty again
pub fn drop_data_take() -> DragData {
    let files = crate::drop_take();
    let mut r = RICH.lock();
    let d = DragData {
        files,
        text: r.text.take(),
        urls: core::mem::take(&mut r.urls),
        html: r.html.take(),
        image: r.image.take(),
        effects: core::mem::take(&mut r.effect),
        ..DragData::default()
    };
    r.url_bytes = 0;
    d
}

// "scheme:" in front (RFC 3986: a letter, then letters, digits, + - .)
fn has_scheme(u: &str) -> bool {
    let b = u.as_bytes();
    let Some(colon) = b.iter().position(|&c| c == b':') else { return false };
    colon > 0
        && b[0].is_ascii_alphabetic()
        && b[..colon].iter().all(|&c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'))
}

/// (OS layers) text in other programs' encodings -> UTF-8 bytes: UTF-16 LE (utf16, up to a
/// NUL unit) or bytes as they are (up to a NUL)
pub fn drop_text_from(bytes: &[u8], utf16: bool) -> Vec<u8> {
    if !utf16 {
        let end = bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len());
        return bytes[..end].to_vec();
    }
    let units = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0);
    let mut s = String::new();
    for c in core::char::decode_utf16(units) {
        s.push(c.unwrap_or('\u{FFFD}'));
    }
    s.into_bytes()
}

/// (OS layers) the lines of a URL list (text/uri-list, text/x-moz-url, UniformResourceLocator):
/// '#' lines skipped, every other line one URL; local files are left to drop_uri_list -> URLs
/// added (also used for the URL a browser drags)
pub fn drop_url_list(text: &[u8]) -> u32 {
    let mut n = 0;
    for line in text.split(|&c| c == b'\n') {
        let l = line.strip_suffix(b"\r").unwrap_or(line);
        if l.is_empty() || l[0] == b'#' || (l.len() >= 5 && l[..5].eq_ignore_ascii_case(b"file:")) {
            continue;
        }
        if let Ok(s) = core::str::from_utf8(l) {
            if drop_add_url(s) {
                n += 1;
            }
        }
    }
    n
}

// ---------------------------------------------------------------- the outgoing drag

static RUNNING: AtomicBool = AtomicBool::new(false);

/// drag `data` out of the app (call while the left button is held) -> started; E_DRAG_END
/// (data = the effect, 0 = cancelled or refused) follows.  Only the standard kinds leave
/// the app.
pub fn drag_start(data: DragData) -> bool {
    if data.types() & DT_STD == 0 || RUNNING.load(Ordering::Acquire) {
        return false;
    }
    RUNNING.store(true, Ordering::Release);
    let ok = crate::os_drag_start(data);
    if !ok {
        RUNNING.store(false, Ordering::Release);
    }
    ok
}

/// an outgoing drag runs (until its E_DRAG_END is queued)
pub fn drag_running() -> bool {
    RUNNING.load(Ordering::Acquire)
}

/// (OS layers) the outgoing drag ended: E_DRAG_END for window w with the effect (0 = none)
#[cfg_attr(any(target_arch = "wasm32", target_os = "android"), allow(dead_code))] // no drags out there yet
pub(crate) fn drag_finished(w: u8, effect: u32) {
    RUNNING.store(false, Ordering::Release);
    crate::evq_push(crate::Event { typ: crate::E_DRAG_END, win: w, data: effect & DE_ALL, ..crate::Event::default() });
}
