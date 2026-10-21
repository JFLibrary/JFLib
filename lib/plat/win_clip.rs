// win_clip -- Windows clipboard, file drops (WM_DROPFILES) and the IME caret position
//                                          [src/plat/win/win_clip.s; docs/modules/CLIPBOARD_DND.md]
//
//   clip_set(fmt, data) -> false on failure
//        CLIP_TEXT: UTF-8 text (converted to CF_UNICODETEXT, "\n" -> "\r\n")
//        CLIP_PNG:  PNG file bytes (registered clipboard format "PNG", the one browsers and
//                   Office exchange; no CF_DIBV5 rendering)
//        CLIP_IMAGE (K1): data = a CLIPIMG record (clipimg_pack: PNG + BMP file): "PNG" +
//                   CF_DIBV5 (the BMP's V5 header and pixels) + CF_DIB (BITMAPINFOHEADER,
//                   32 bit BI_RGB)
//   clip_get(fmt) -> bytes   text as UTF-8 with "\r\n" -> "\n"; CLIP_IMAGE (K1): "PNG", else
//        "image/png", else CF_DIBV5, else CF_DIB as a BMP file (clip_dib_to_bmp: header checks,
//        paint.net's alpha rule)
//   clip_has(fmt)            the clipboard offers the format (no data is read; K1)
//   win_drop_files(hdrop) -> (files, x, y)   (K1) WM_DROPFILES: the paths go to the pending
//        drop list (drop_*); shell32 (DragQueryFileW, DragQueryPoint, DragFinish) is loaded on
//        the first drop.  The windows get WS_EX_ACCEPTFILES.
//   win_drop_selftest(hwnd)  (K1, test aid) jfgui_TEST_DROPFILES="path|path..." posts a
//        WM_DROPFILES with a real HDROP (GlobalAlloc'd DROPFILES, wide paths, point 100, 100) to
//        the new main window, so the whole shell32 path runs in a real window
//   ime_pos(x, y, h)         place a hidden system caret at client px (x, y) with height h in
//        the window that has the keyboard, so IME composition / candidate windows open next to
//        the text caret and screen magnifiers follow it (Magnifier, ZoomText: GetGUIThreadInfo
//        and the caret's location events); h <= 0 removes it, so does the window's WM_KILLFOCUS
//        (caret_focus_lost)
//
// The clipboard is opened for the main window; opening is retried for a moment while another
// program holds it.  Every block of another program is checked (sizes, NUL, surrogates).

use alloc::vec::Vec;
use core::cell::Cell;
use core::sync::atomic::{AtomicUsize, Ordering};

use super::{PostMessageW, UiThread, dll_proc, main_hwnd, wide_cap, win_hwnd};
use crate::{CLIP_DOCX, CLIP_HTML, CLIP_RTF, CLIP_IMAGE, CLIP_PNG, CLIP_TEXT, clip_dib_to_bmp, clipimg_split, drop_add, drop_reset};

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn OpenClipboard(h: isize) -> i32;
    fn CloseClipboard() -> i32;
    fn EmptyClipboard() -> i32;
    fn SetClipboardData(fmt: u32, h: isize) -> isize;
    fn GetClipboardData(fmt: u32) -> isize;
    fn IsClipboardFormatAvailable(fmt: u32) -> i32;
    fn RegisterClipboardFormatW(name: *const u16) -> u32;
    fn CreateCaret(h: isize, bmp: isize, w: i32, h: i32) -> i32;
    fn DestroyCaret() -> i32;
    fn SetCaretPos(x: i32, y: i32) -> i32;
    fn GetFocus() -> isize;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GlobalAlloc(flags: u32, n: usize) -> isize;
    fn GlobalFree(h: isize) -> isize;
    fn GlobalLock(h: isize) -> *mut u8;
    fn GlobalUnlock(h: isize) -> i32;
    fn GlobalSize(h: isize) -> usize;
    fn Sleep(ms: u32);
}

const CF_UNICODETEXT: u32 = 13;

#[link(name = "imm32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ImmGetContext(hwnd: isize) -> isize;
    fn ImmReleaseContext(hwnd: isize, context: isize) -> i32;
    fn ImmGetCompositionStringW(context: isize, index: u32, buffer: *mut u16, bytes: u32) -> i32;
}
pub(super) fn ime_composition(hwnd: isize, win: u8, flags: u32) {
    // Attribute-only notifications retain the existing composition. Cursor-only
    // notifications still need to query its text; a result ends the preedit.
    if flags!=0 && flags & (8|0x80|0x800)==0 { return; }
    // SAFETY: the input context is acquired/released for this HWND. The UTF-16
    // output is bounded by the exact allocated buffer length, then decoded safely.
    unsafe {
        let context=ImmGetContext(hwnd); if context==0 { return; }
        let mut result=alloc::string::String::new(); let mut cursor=0usize;
        if flags & (8|0x80) != 0 {
            let bytes=ImmGetCompositionStringW(context,8,core::ptr::null_mut(),0);
            if bytes>0 && bytes<=8192 && bytes%2==0 {
                let mut wide=alloc::vec![0u16;bytes as usize/2];
                let read=ImmGetCompositionStringW(context,8,wide.as_mut_ptr(),bytes as u32);
                if read>=0 && read<=bytes && read%2==0 {
                    wide.truncate(read as usize/2);
                    let pos=ImmGetCompositionStringW(context,0x80,core::ptr::null_mut(),0).max(0) as usize;
                    result=alloc::string::String::from_utf16_lossy(&wide);
                    cursor=alloc::string::String::from_utf16_lossy(&wide[..pos.min(wide.len())]).len();
                }
            }
        }
        ImmReleaseContext(hwnd,context);
        crate::ime_preedit_set(win,&result,cursor);
    }
}
const CF_DIB: u32 = 8;
const CF_DIBV5: u32 = 17;
const GMEM_MOVEABLE: u32 = 2;
const GMEM_ZEROINIT: u32 = 0x40;
const CLIP_MAX: usize = 1 << 30; // larger clipboard blocks are refused (K1)
const DROP_FILES_MAX: u32 = 4096;
const DROP_WCH_MAX: u32 = 32767;
const DROPFILES_SIZE: usize = 20; // K1: win_drop_selftest
const SELFTEST_W: usize = 4096;
const OPEN_TRIES: u32 = 10;
const OPEN_WAIT_MS: u32 = 5;
const WM_DROPFILES: u32 = 0x0233;
static W_PNG: [u16; 4] = [0x50, 0x4E, 0x47, 0]; // "PNG"
static W_IMPNG: [u16; 10] = [0x69, 0x6D, 0x61, 0x67, 0x65, 0x2F, 0x70, 0x6E, 0x67, 0]; // "image/png"

struct Clip {
    cf_png: Cell<u32>,
    cf_impng: Cell<u32>,
    ime_h: Cell<i32>,
    ime_x: Cell<i32>,
    ime_y: Cell<i32>,
    /// the window the caret was created in
    ime_w: Cell<isize>,
}

static CLIP: UiThread<Clip> = UiThread(Clip {
    cf_png: Cell::new(0),
    cf_impng: Cell::new(0),
    ime_h: Cell::new(0),
    ime_x: Cell::new(0),
    ime_y: Cell::new(0),
    ime_w: Cell::new(0),
});

// shell32's drop functions (0 = not loaded); SHELL_TRIED: 1 after the first attempt
static P_DQF: AtomicUsize = AtomicUsize::new(0);
static P_DQP: AtomicUsize = AtomicUsize::new(0);
static P_DFIN: AtomicUsize = AtomicUsize::new(0);
static SHELL_TRIED: AtomicUsize = AtomicUsize::new(0);

fn clip() -> &'static Clip {
    &CLIP.0
}

/// the clipboard is open (retried while another program holds it)  [clip_open]
fn clip_open() -> bool {
    for k in 0..OPEN_TRIES {
        // SAFETY: plain call; the main window (or 0 = this task).
        if unsafe { OpenClipboard(main_hwnd()) } != 0 {
            return true;
        }
        if k + 1 < OPEN_TRIES {
            // SAFETY: no memory involved.
            unsafe { Sleep(OPEN_WAIT_MS) };
        }
    }
    false
}

/// the registered "PNG" clipboard format (0 if it cannot be registered)
fn png_format() -> u32 {
    let c = clip();
    if c.cf_png.get() == 0 {
        // SAFETY: a static NUL-terminated name.
        c.cf_png.set(unsafe { RegisterClipboardFormatW(W_PNG.as_ptr()) });
    }
    c.cf_png.get()
}

/// the registered "image/png" format (K1; 0 if it cannot be registered)
fn impng_format() -> u32 {
    let c = clip();
    if c.cf_impng.get() == 0 {
        // SAFETY: a static NUL-terminated name.
        c.cf_impng.set(unsafe { RegisterClipboardFormatW(W_IMPNG.as_ptr()) });
    }
    c.cf_impng.get()
}

// clipboard_platform: registered standard formats plus a private, length-delimited record.
fn rich_format(fmt: u32) -> u32 {
    let name = match fmt {
        CLIP_HTML => "HTML Format",
        CLIP_RTF => "Rich Text Format",
        CLIP_DOCX => "application/x-jfgui-docx-fragment",
        _ => return 0,
    };
    let mut wide: Vec<u16> = name.encode_utf16().collect();
    wide.push(0);
    // SAFETY: the UTF-16 name is NUL-terminated and lives through the call.
    unsafe { RegisterClipboardFormatW(wide.as_ptr()) }
}

/// Publish TEXT/HTML/RTF/DOCX together, allocating every handle before replacing the clipboard.
pub fn clip_set_many(entries: &[(u32, &[u8])]) -> bool {
    if !crate::clip_many_valid(entries) { return false; }
    let mut blocks = Vec::new();
    for &(fmt, data) in entries {
        let format = if fmt == CLIP_TEXT { CF_UNICODETEXT } else { rich_format(fmt) };
        let bytes = match fmt {
            CLIP_TEXT => {
                let text = core::str::from_utf8(data).unwrap_or("");
                let mut bytes = Vec::new();
                let mut prev = 0u16;
                for ch in text.encode_utf16() {
                    if ch == 10 && prev != 13 { bytes.extend_from_slice(&13u16.to_le_bytes()); }
                    bytes.extend_from_slice(&ch.to_le_bytes());
                    prev = ch;
                }
                bytes.extend_from_slice(&[0, 0]);
                bytes
            }
            CLIP_HTML => crate::clip_html_encode(data).unwrap_or_default(),
            CLIP_RTF => { let mut bytes = data.to_vec(); bytes.push(0); bytes }
            _ => {
                let mut bytes = b"ASMCLIP1".to_vec();
                bytes.extend_from_slice(&(data.len() as u64).to_le_bytes());
                bytes.extend_from_slice(data);
                bytes
            }
        };
        let h = if format == 0 { 0 } else { gblock(&bytes) };
        if h == 0 {
            for (_, handle) in blocks {
                // SAFETY: these allocated blocks have not been handed to Windows.
                unsafe { GlobalFree(handle) };
            }
            return false;
        }
        blocks.push((format, h));
    }
    let mut taken = 0;
    if clip_open() {
        // SAFETY: the clipboard is open and every handle is an owned movable block.
        unsafe {
            if EmptyClipboard() != 0 {
                for (format, handle) in &mut blocks {
                    if SetClipboardData(*format, *handle) != 0 { *handle = 0; taken += 1; }
                }
            }
            CloseClipboard();
        }
    }
    for (_, handle) in &blocks {
        if *handle != 0 {
            // SAFETY: SetClipboardData did not take these blocks.
            unsafe { GlobalFree(*handle) };
        }
    }
    taken == entries.len()
}

fn available(fmt: u32) -> bool {
    // SAFETY: plain call.
    unsafe { IsClipboardFormatAvailable(fmt) != 0 }
}

/// one code point of UTF-8 at s[i] -> (cp, next); invalid bytes give U+FFFD and advance by one;
/// a NUL (or the end) gives 0  [utf8_decode]
fn utf8_decode(s: &[u8], i: usize) -> (u32, usize) {
    let Some(&b) = s.get(i) else { return (0, i) };
    let c = b as u32;
    if c < 0x80 {
        return (c, i + 1);
    }
    let (mut v, n, min) = match c {
        0xC2..=0xDF => (c & 0x1F, 1, 0x80),
        0xE0..=0xEF => (c & 0x0F, 2, 0x800),
        0xF0..=0xF4 => (c & 7, 3, 0x10000),
        _ => return (0xFFFD, i + 1), // continuation byte, overlong lead, > U+10FFFF
    };
    for k in 1..=n {
        match s.get(i + k) {
            Some(&t) if t & 0xC0 == 0x80 => v = v << 6 | (t as u32 & 0x3F),
            _ => return (0xFFFD, i + 1),
        }
    }
    if v < min || v > 0x10FFFF || v & 0xFFFF_F800 == 0xD800 {
        return (0xFFFD, i + 1); // overlong, too large, surrogate
    }
    (v, i + 1 + n)
}

/// a moveable global block holding a copy of the bytes (at least 1 byte)  [gblock]
fn gblock(data: &[u8]) -> isize {
    // SAFETY: a new block of at least data.len() bytes, locked for the copy.
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1));
        if h == 0 {
            return 0;
        }
        let p = GlobalLock(h);
        if p.is_null() {
            GlobalFree(h);
            return 0;
        }
        core::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
        GlobalUnlock(h);
        h
    }
}

/// hand one block to the clipboard (freed when it is not taken)
fn put_one(fmt: u32, h: isize) -> bool {
    if h == 0 {
        return false;
    }
    if !clip_open() {
        // SAFETY: the block is still ours.
        unsafe { GlobalFree(h) };
        return false;
    }
    // SAFETY: the clipboard is open; SetClipboardData takes the block on success.
    let taken = unsafe {
        EmptyClipboard();
        let t = SetClipboardData(fmt, h);
        CloseClipboard();
        t
    };
    if taken == 0 {
        // SAFETY: not taken: the block is still ours.
        unsafe { GlobalFree(h) };
        return false;
    }
    true
}

/// CLIP_TEXT / CLIP_PNG / CLIP_IMAGE (a CLIPIMG record) -> false on failure  [plat_clip_set]
pub fn clip_set(fmt: u32, data: &[u8]) -> bool {
    if matches!(fmt, CLIP_HTML | CLIP_RTF | CLIP_DOCX) { return clip_set_many(&[(fmt, data)]); }
    match fmt {
        CLIP_TEXT => {
            // UTF-8 -> UTF-16 (at most 2 wchars per input byte, + NUL), "\n" -> "\r\n"
            let mut w: Vec<u16> = Vec::with_capacity(data.len() * 2 + 1);
            let (mut i, mut prev) = (0, 0u32);
            while i < data.len() {
                let (c, next) = utf8_decode(data, i);
                if c == 0 {
                    break;
                }
                i = next;
                if c == 10 && prev != 13 {
                    w.push(13);
                }
                prev = c;
                let mut b = [0u16; 2];
                w.extend_from_slice(char::from_u32(c).unwrap_or('\u{FFFD}').encode_utf16(&mut b));
            }
            w.push(0);
            // SAFETY: the u16 buffer viewed as its bytes.
            let bytes = unsafe { core::slice::from_raw_parts(w.as_ptr() as *const u8, w.len() * 2) };
            put_one(CF_UNICODETEXT, gblock(bytes))
        }
        CLIP_PNG => {
            // the bytes as they are (GlobalSize = length)
            let f = png_format();
            f != 0 && put_one(f, gblock(data))
        }
        CLIP_IMAGE => match clipimg_split(data) {
            Some((png, bmp)) => cs_image(png, bmp),
            None => false,
        },
        _ => false,
    }
}

/// "PNG" + CF_DIBV5 + CF_DIB in one clipboard change  [cs_image]
fn cs_image(png: &[u8], bmp: &[u8]) -> bool {
    let mut hs = [0isize; 3]; // PNG, V5, DIB
    if !png.is_empty() && png.len() <= CLIP_MAX {
        hs[0] = gblock(png);
    }
    // the BMP file: 14-byte file header + BITMAPV5HEADER (124) + pixels
    let n = bmp.len();
    if n >= 14 + 124 && n <= CLIP_MAX && u32::from_le_bytes([bmp[14], bmp[15], bmp[16], bmp[17]]) == 124 {
        let off = u32::from_le_bytes([bmp[10], bmp[11], bmp[12], bmp[13]]) as usize; // bfOffBits
        if (14 + 124..=n).contains(&off) {
            hs[1] = gblock(&bmp[14..]);
            // CF_DIB: a 40-byte header (BI_RGB, 32 bit) + the same pixels
            let mut dib = Vec::with_capacity(40 + n - off);
            dib.extend_from_slice(&bmp[14..54]);
            dib[0..4].copy_from_slice(&40u32.to_le_bytes());
            dib[16..20].copy_from_slice(&0u32.to_le_bytes()); // BI_RGB
            dib[32..40].copy_from_slice(&[0; 8]); // colours used, important
            dib.extend_from_slice(&bmp[off..]);
            hs[2] = gblock(&dib);
        }
    }
    if hs == [0; 3] {
        return false;
    }
    let mut taken = 0;
    if clip_open() {
        // SAFETY: the clipboard is open; SetClipboardData takes a block on success.
        unsafe {
            EmptyClipboard();
            let pf = if hs[0] != 0 { png_format() } else { 0 };
            for (k, fmt) in [pf, CF_DIBV5, CF_DIB].into_iter().enumerate() {
                if hs[k] != 0 && fmt != 0 && SetClipboardData(fmt, hs[k]) != 0 {
                    hs[k] = 0; // taken
                    taken += 1;
                }
            }
            CloseClipboard();
        }
    }
    // blocks the clipboard did not take are still ours
    for h in hs {
        if h != 0 {
            // SAFETY: our block.
            unsafe { GlobalFree(h) };
        }
    }
    taken != 0
}

/// the clipboard data of a format as bytes (None: not there / too large / not readable);
/// f(bytes) converts it while the block is locked
fn get_block(fmt: u32, f: impl FnOnce(&[u8]) -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    if !clip_open() {
        return None;
    }
    // SAFETY: the clipboard is open; the block stays locked while f reads it.
    let r = unsafe {
        let h = GetClipboardData(fmt);
        let mut r = None;
        if h != 0 {
            let n = GlobalSize(h);
            let p = GlobalLock(h);
            if !p.is_null() {
                if n <= CLIP_MAX {
                    r = f(core::slice::from_raw_parts(p, n));
                }
                GlobalUnlock(h);
            }
        }
        r
    };
    // SAFETY: the clipboard was opened above.
    unsafe { CloseClipboard() };
    r
}

/// the clipboard contents (text as UTF-8 with "\n"; CLIP_IMAGE: PNG or BMP file bytes)  [plat_clip_get]
pub fn clip_get(fmt: u32) -> Option<Vec<u8>> {
    match fmt {
        CLIP_TEXT => {
            if !available(CF_UNICODETEXT) {
                return None;
            }
            get_block(CF_UNICODETEXT, |b| Some(text_from_wide(b)))
        }
        CLIP_PNG => {
            let f = png_format();
            if f == 0 || !available(f) {
                return None;
            }
            get_block(f, |b| Some(b.to_vec()))
        }
        CLIP_IMAGE => cg_image(),
        CLIP_HTML | CLIP_RTF | CLIP_DOCX => {
            let f = rich_format(fmt);
            if f == 0 || !available(f) { return None; }
            get_block(f, |b| {
                if b.len() > crate::CLIP_RICH_MAX + 65536 { return None; }
                match fmt {
                    CLIP_HTML => crate::clip_html_decode(b),
                    CLIP_RTF => {
                        let mut end = b.len();
                        while end > 0 && b[end - 1] == 0 { end -= 1; }
                        if end > crate::CLIP_RICH_MAX { return None; }
                        Some(b[..end].to_vec())
                    }
                    _ => {
                        if b.len() < 16 || &b[..8] != b"ASMCLIP1" { return None; }
                        let len = usize::try_from(u64::from_le_bytes(b[8..16].try_into().ok()?)).ok()?;
                        if len > crate::CLIP_RICH_MAX { return None; }
                        Some(b.get(16..16usize.checked_add(len)?)?.to_vec())
                    }
                }
            })
        },
        _ => None,
    }
}

/// UTF-16 up to the NUL (or the end of the block) -> UTF-8, "\r\n" -> "\n", bad surrogates ->
/// U+FFFD
fn text_from_wide(b: &[u8]) -> Vec<u8> {
    let w: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    let mut out = Vec::with_capacity(w.len() * 3);
    let mut i = 0;
    while i < w.len() {
        let c = w[i] as u32;
        if c == 13 && i + 1 < w.len() && w[i + 1] == 10 {
            i += 1; // "\r\n" -> "\n"
            continue;
        }
        let cp;
        if c & 0xFC00 == 0xD800 {
            if i + 1 >= w.len() {
                cp = 0xFFFD; // high surrogate at the very end
                i += 1;
            } else if w[i + 1] as u32 & 0xFC00 == 0xDC00 {
                cp = 0x10000 + ((c - 0xD800) << 10) + (w[i + 1] as u32 - 0xDC00);
                i += 2;
            } else {
                cp = 0xFFFD;
                i += 1;
            }
        } else {
            cp = if c & 0xFC00 == 0xDC00 { 0xFFFD } else { c };
            i += 1;
        }
        let mut b4 = [0u8; 4];
        out.extend_from_slice(char::from_u32(cp).unwrap_or('\u{FFFD}').encode_utf8(&mut b4).as_bytes());
    }
    out
}

/// CLIP_IMAGE: "PNG", "image/png", CF_DIBV5, CF_DIB  [cg_image]
fn cg_image() -> Option<Vec<u8>> {
    let pf = png_format();
    let (fmt, dib) = if pf != 0 && available(pf) {
        (pf, false)
    } else {
        let ip = impng_format();
        if ip != 0 && available(ip) {
            (ip, false)
        } else if available(CF_DIBV5) {
            (CF_DIBV5, true)
        } else if available(CF_DIB) {
            (CF_DIB, true)
        } else {
            return None;
        }
    };
    get_block(fmt, |b| if dib { clip_dib_to_bmp(b) } else { Some(b.to_vec()) })
}

/// the clipboard offers fmt (CLIP_TEXT / CLIP_PNG / CLIP_IMAGE)  [plat_clip_has]
pub fn clip_has(fmt: u32) -> bool {
    if matches!(fmt, CLIP_HTML | CLIP_RTF | CLIP_DOCX) { let f = rich_format(fmt); return f != 0 && available(f); }
    if fmt == CLIP_TEXT {
        return available(CF_UNICODETEXT);
    }
    let pf = png_format();
    if pf != 0 && available(pf) {
        return true; // (CLIP_PNG and CLIP_IMAGE; other formats only ask "PNG" as the asm)
    }
    if fmt != CLIP_IMAGE {
        return false;
    }
    let ip = impng_format();
    (ip != 0 && available(ip)) || available(CF_DIBV5) || available(CF_DIB)
}

// ---------------------------------------------------------------- drops

/// the shell32 drop functions are there  [shell_load]
fn shell_load() -> bool {
    if P_DFIN.load(Ordering::Relaxed) != 0 {
        return true;
    }
    if SHELL_TRIED.swap(1, Ordering::Relaxed) != 0 {
        return false;
    }
    let s = b"shell32.dll\0";
    let (q, p, f) = (dll_proc(s, b"DragQueryFileW\0"), dll_proc(s, b"DragQueryPoint\0"), dll_proc(s, b"DragFinish\0"));
    if q == 0 || p == 0 || f == 0 {
        return false;
    }
    P_DQF.store(q, Ordering::Relaxed);
    P_DQP.store(p, Ordering::Relaxed);
    P_DFIN.store(f, Ordering::Relaxed);
    true
}

/// WM_DROPFILES -> (files added, x, y client px)  [win_drop_files]
pub(super) fn win_drop_files(hdrop: isize) -> (u32, i32, i32) {
    if !shell_load() {
        // without shell32 the drop is ignored (and freed: DragFinish = GlobalFree)
        // SAFETY: the HDROP is a global block handed to us.
        unsafe { GlobalFree(hdrop) };
        return (0, 0, 0);
    }
    // SAFETY: the shell32 functions with their signatures; the HDROP comes from the system
    // (or win_drop_selftest); every buffer holds what the calls are told.
    unsafe {
        let dqf: unsafe extern "system" fn(isize, u32, *mut u16, u32) -> u32 = core::mem::transmute(P_DQF.load(Ordering::Relaxed));
        let dqp: unsafe extern "system" fn(isize, *mut [i32; 2]) -> i32 = core::mem::transmute(P_DQP.load(Ordering::Relaxed));
        let dfin: unsafe extern "system" fn(isize) = core::mem::transmute(P_DFIN.load(Ordering::Relaxed));
        drop_reset();
        let files = dqf(hdrop, 0xFFFF_FFFF, core::ptr::null_mut(), 0).min(DROP_FILES_MAX);
        let mut added = 0;
        for i in 0..files {
            let n = dqf(hdrop, i, core::ptr::null_mut(), 0); // wchars without the NUL
            if n == 0 || n > DROP_WCH_MAX {
                continue;
            }
            let mut w = alloc::vec![0u16; n as usize + 1];
            dqf(hdrop, i, w.as_mut_ptr(), n + 1);
            w[n as usize] = 0;
            let len = w.iter().position(|&c| c == 0).unwrap_or(n as usize);
            let path = super::super::from_wide(&w[..len]);
            if drop_add(&path) {
                added += 1;
            }
        }
        let mut pt = [0i32; 2];
        dqp(hdrop, &mut pt);
        dfin(hdrop);
        (added, pt[0], pt[1])
    }
}

/// jfgui_TEST_DROPFILES: post WM_DROPFILES with our own HDROP to hwnd  [win_drop_selftest]
pub(super) fn win_drop_selftest(hwnd: isize) {
    let Some(v) = crate::env("jfgui_TEST_DROPFILES") else { return };
    if v.is_empty() {
        return;
    }
    // '|' separates the paths: each becomes NUL-terminated, one more NUL ends the list
    let mut w = wide_cap(&v, SELFTEST_W);
    w.pop();
    for c in w.iter_mut() {
        if *c == b'|' as u16 {
            *c = 0;
        }
    }
    let n = w.len();
    // SAFETY: a zeroed DROPFILES block with the wide list behind it (two more NULs from the
    // zero fill); posted to our window, which frees it (DragFinish) -- or freed here.
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, n * 2 + DROPFILES_SIZE + 4);
        if h == 0 {
            return;
        }
        let p = GlobalLock(h);
        if p.is_null() {
            GlobalFree(h);
            return;
        }
        let hdr: [u32; 5] = [DROPFILES_SIZE as u32, 100, 100, 0, 1]; // pFiles, pt, fNC, fWide
        core::ptr::copy_nonoverlapping(hdr.as_ptr() as *const u8, p, DROPFILES_SIZE);
        core::ptr::copy_nonoverlapping(w.as_ptr() as *const u8, p.add(DROPFILES_SIZE), n * 2);
        GlobalUnlock(h);
        if PostMessageW(hwnd, WM_DROPFILES, h as usize, 0) == 0 {
            GlobalFree(h);
        }
    }
}

// ---------------------------------------------------------------- IME

/// place a hidden system caret at client px (x, y) with height h in the window of ours that
/// has the keyboard (h <= 0, or none of ours has it: removed)
pub fn ime_pos(x: i32, y: i32, h: i32) {
    // SAFETY: no arguments: the focus window of this (the UI) thread.
    let f = unsafe { GetFocus() };
    let hwnd = if f != 0 && (0..crate::PW_MAX as i32).any(|w| win_hwnd(w) == f) { f } else { 0 };
    let c = clip();
    if h <= 0 || hwnd == 0 {
        caret_off();
        return;
    }
    if h != c.ime_h.get() || hwnd != c.ime_w.get() {
        c.ime_h.set(h);
        c.ime_w.set(hwnd);
        // SAFETY: our window with the keyboard; (re)create (the old caret goes): 1 px wide,
        // never shown.
        unsafe { CreateCaret(hwnd, 0, 1, h) };
        c.ime_x.set(i32::MIN);
    }
    if x != c.ime_x.get() || y != c.ime_y.get() {
        c.ime_x.set(x);
        c.ime_y.set(y);
        // SAFETY: plain call.
        unsafe { SetCaretPos(x, y) };
    }
}

// the system caret goes
fn caret_off() {
    let c = clip();
    if c.ime_h.get() != 0 {
        c.ime_h.set(0);
        c.ime_w.set(0);
        // SAFETY: no arguments.
        unsafe { DestroyCaret() };
    }
}

/// (WM_KILLFOCUS) window hwnd loses the keyboard: its caret goes (the UI places it again in the
/// window that gets the keyboard)
pub(super) fn caret_focus_lost(hwnd: isize) {
    if clip().ime_w.get() == hwnd {
        caret_off();
    }
}
