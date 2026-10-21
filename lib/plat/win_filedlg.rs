// win_filedlg -- the system Open / Save dialog (comdlg32 + ole32, loaded at run time)
//                                                               [src/plat/win/win_filedlg.s]
//
//   file_dialog(flags, path, exts) -> (1 chosen / 0 cancelled / -1 no system dialog, index of
//        the chosen extension entry)
//        flags: FD_SAVE (bit 0), initial extension index in bits 8..15 (FD_EXT_SHIFT);
//        FD_FOLDER answers -1 (the library's own dialog chooses folders).
//        path (UTF-8): initial file / folder in, chosen path out ('/' separators).
//        exts: ["png=PNG-Bild", "jpg;jpeg=JPEG-Bild"] (extensions separated by ';', the first
//        is the default, optional "=description"), empty = all.
//   file_dialog_filter(flags, exts) -> (filter, default extension)   the dialog's filter list
//        ("description\0pattern\0...\0\0", UTF-16) -- for tests
//
// GetOpenFileNameW / GetSaveFileNameW without hooks show the Explorer dialog of the system
// (Common Item Dialog); the call is modal and blocks until the user is done (the window
// procedures of our windows keep running).

use alloc::string::String;
use alloc::vec::Vec;
use core::cell::Cell;

use super::{UiThread, dll_proc, main_hwnd, wide_cap};
use crate::{FD_EXT_SHIFT, FD_SAVE};

const OFN_OVERWRITEPROMPT: u32 = 0x2;
const OFN_HIDEREADONLY: u32 = 0x4;
const OFN_NOCHANGEDIR: u32 = 0x8;
const OFN_PATHMUSTEXIST: u32 = 0x800;
const OFN_FILEMUSTEXIST: u32 = 0x1000;
const OFN_EXPLORER: u32 = 0x80000;
const WPATH: usize = 1040; // wide chars per path buffer
const U8CAP: usize = 4096; // the UTF-8 filter text (str_cat capacity)
const FILTER_W: usize = 2046; // wide chars of the filter (utf8_to_w capacity)
const DEFEXT_W: usize = 16;
const NAME_MAX: usize = 30; // bytes of one extension name

/// OPENFILENAMEW (x64, 152 bytes)
#[repr(C)]
struct Ofn {
    size: u32,
    owner: isize,
    inst: isize,
    filter: *const u16,
    custom_filter: *mut u16,
    max_cust_filter: u32,
    filter_index: u32,
    file: *mut u16,
    max_file: u32,
    file_title: *mut u16,
    max_file_title: u32,
    initial_dir: *const u16,
    title: *const u16,
    flags: u32,
    file_offset: u16,
    file_extension: u16,
    def_ext: *const u16,
    cust_data: isize,
    hook: usize,
    template: *const u16,
    reserved_p: usize,
    reserved: u32,
    flags_ex: u32,
}

const _: () = assert!(core::mem::size_of::<Ofn>() == 152);

struct Fd {
    state: Cell<u8>, // 0 unknown, 1 available, 2 unavailable
    open: Cell<usize>,
    save: Cell<usize>,
}

static FD: UiThread<Fd> = UiThread(Fd { state: Cell::new(0), open: Cell::new(0), save: Cell::new(0) });

/// GetOpenFileNameW / GetSaveFileNameW are available (COM is initialised once for the
/// Explorer dialog)  [fd_load_api]
fn fd_load_api() -> bool {
    let f = &FD.0;
    if f.state.get() == 0 {
        f.state.set(2); // unavailable unless set below
        let co = dll_proc(b"ole32.dll\0", b"CoInitializeEx\0");
        if co != 0 {
            // SAFETY: CoInitializeEx(NULL, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).
            let ci: unsafe extern "system" fn(usize, u32) -> i32 = unsafe { core::mem::transmute(co) };
            unsafe { ci(0, 6) };
        }
        let o = dll_proc(b"comdlg32.dll\0", b"GetOpenFileNameW\0");
        let s = dll_proc(b"comdlg32.dll\0", b"GetSaveFileNameW\0");
        f.open.set(o);
        f.save.set(s);
        if o != 0 && s != 0 {
            f.state.set(1);
        }
    }
    f.state.get() == 1
}

/// the system Open / Save dialog (see the file header)  [plat_file_dialog]
pub fn file_dialog(flags: u32, path: &mut String, exts: &[&str]) -> (i32, u32) {
    // FD_FOLDER: GetOpenFileNameW picks files only -- the library's own dialog chooses folders
    if flags & crate::FD_FOLDER != 0 || !fd_load_api() {
        return (-1, 0);
    }
    let save = flags & FD_SAVE != 0;
    let (filter, defext) = file_dialog_filter(flags, exts);
    let (wdir, wfile) = fd_split_initial(path);
    let mut file = alloc::vec![0u16; WPATH];
    let n = wfile.len().min(WPATH - 1);
    file[..n].copy_from_slice(&wfile[..n]);
    let mut fl = OFN_EXPLORER | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_HIDEREADONLY;
    fl |= if save { OFN_OVERWRITEPROMPT } else { OFN_FILEMUSTEXIST };
    let wdir0 = if wdir.is_empty() { Vec::new() } else { [&wdir[..], &[0]].concat() };
    let mut ofn = Ofn {
        size: 152,
        owner: main_hwnd(),
        inst: 0,
        filter: filter.as_ptr(),
        custom_filter: core::ptr::null_mut(),
        max_cust_filter: 0,
        filter_index: if save { ((flags >> FD_EXT_SHIFT) & 0xFF) + 1 } else { 1 }, // open: "all supported files"
        file: file.as_mut_ptr(),
        max_file: WPATH as u32,
        file_title: core::ptr::null_mut(),
        max_file_title: 0,
        initial_dir: if wdir.is_empty() { core::ptr::null() } else { wdir0.as_ptr() },
        title: core::ptr::null(),
        flags: fl,
        file_offset: 0,
        file_extension: 0,
        def_ext: if defext.first().copied().unwrap_or(0) == 0 { core::ptr::null() } else { defext.as_ptr() },
        cust_data: 0,
        hook: 0,
        template: core::ptr::null(),
        reserved_p: 0,
        reserved: 0,
        flags_ex: 0,
    };
    let p = if save { FD.0.save.get() } else { FD.0.open.get() };
    // SAFETY: GetOpenFileNameW / GetSaveFileNameW(OPENFILENAMEW*) -> BOOL; every buffer the
    // record points to lives until the call returns (file holds max_file wide chars).
    let f: unsafe extern "system" fn(*mut Ofn) -> i32 = unsafe { core::mem::transmute(p) };
    super::wtest::autoclose_arm(); // (test aid: file_dialog_autoclose)
    if unsafe { f(&mut ofn) } == 0 {
        return (0, 0);
    }
    // the chosen path -> UTF-8 with '/' separators
    let len = file.iter().position(|&c| c == 0).unwrap_or(WPATH);
    *path = super::super::from_wide(&file[..len]).replace('\\', "/");
    (1, ofn.filter_index.saturating_sub(1))
}

/// the first extension of "a;b=description" (at most 30 bytes)  [ext_name]
fn ext_name(e: &str) -> &str {
    let b = e.as_bytes();
    let mut n = 0;
    while n < b.len() && b[n] != b'=' && b[n] != b';' && n < NAME_MAX {
        n += 1;
    }
    // (cut at a byte boundary like the assembly; a cut inside a character drops it)
    match e.get(..n) {
        Some(s) => s,
        None => e.get(..e.floor_char_boundary(n)).unwrap_or(""),
    }
}

/// append to the filter text, truncated to U8CAP - 1 bytes at a character boundary  [u8_cat]
fn cat(u: &mut String, s: &str) {
    let room = (U8CAP - 1).saturating_sub(u.len());
    if s.len() <= room {
        u.push_str(s);
    } else {
        u.push_str(&s[..s.floor_char_boundary(room)]);
    }
}

/// "*.a;*.b" for the extensions of "a;b=description"  [cat_entry]
fn cat_entry(u: &mut String, e: &str) {
    let mut rest = e;
    let mut first = true;
    loop {
        if !first {
            cat(u, ";");
        }
        first = false;
        cat(u, "*.");
        cat(u, ext_name(rest));
        // on to the next ';' (or the end / '=')
        match rest.find([';', '=']) {
            Some(i) if rest.as_bytes()[i] == b';' => rest = &rest[i + 1..],
            _ => return,
        }
    }
}

/// "*.a;*.b" of every entry  [cat_patterns]
fn cat_patterns(u: &mut String, exts: &[&str]) {
    for (k, e) in exts.iter().enumerate() {
        if k > 0 {
            cat(u, ";");
        }
        cat_entry(u, e);
    }
}

/// the filter list ("description\0pattern\0...\0\0", UTF-16) and the default extension (UTF-16,
/// NUL-terminated; empty text = none)  [fd_build_filter]
pub fn file_dialog_filter(flags: u32, exts: &[&str]) -> (Vec<u16>, Vec<u16>) {
    let save = flags & FD_SAVE != 0;
    let mut u = String::new();
    let mut defext = alloc::vec![0u16];
    let exts: Vec<&str> = exts.iter().copied().take_while(|e| !e.is_empty()).collect();
    if exts.is_empty() {
        cat(&mut u, "Alle Dateien (*.*)\x01*.*\x01");
    } else {
        // default extension: the one selected initially (save) or the first
        let i = if save { ((flags >> FD_EXT_SHIFT) & 0xFF) as usize } else { 0 };
        let e = exts.get(i).copied().unwrap_or(exts[0]);
        defext = wide_cap(ext_name(e), DEFEXT_W);
        // open: "all supported files" first
        if !save {
            cat(&mut u, "Alle unterstützten Dateien (");
            cat_patterns(&mut u, &exts);
            cat(&mut u, ")\x01");
            cat_patterns(&mut u, &exts);
            cat(&mut u, "\x01");
        }
        for e in &exts {
            // description: after '=', else "EXT-Datei"
            match e.find('=') {
                Some(k) => cat(&mut u, &e[k + 1..]),
                None => {
                    let up = ext_name(e).to_ascii_uppercase();
                    cat(&mut u, &up);
                    cat(&mut u, "-Datei");
                }
            }
            cat(&mut u, " (");
            cat_entry(&mut u, e);
            cat(&mut u, ")\x01");
            cat_entry(&mut u, e);
            cat(&mut u, "\x01");
        }
        if !save {
            cat(&mut u, "Alle Dateien (*.*)\x01*.*\x01");
        }
    }
    // the separators become NULs; the final one plus the terminator end the list
    let mut w = wide_cap(&u, FILTER_W);
    for c in w.iter_mut() {
        if *c == 1 {
            *c = 0;
        }
    }
    w.push(0); // a second terminator
    (w, defext)
}

/// (folder, name) of the initial path as UTF-16 with backslashes (no NULs)  [fd_split_initial]
fn fd_split_initial(path: &str) -> (Vec<u16>, Vec<u16>) {
    let mut wdir = Vec::new();
    let mut wfile = Vec::new();
    if path.is_empty() {
        return (wdir, wfile);
    }
    let mut u = String::new();
    cat(&mut u, path);
    let u = u.replace('/', "\\");
    let strip = |mut v: Vec<u16>| {
        v.pop();
        v
    };
    if crate::file_exists_raw(&u) == 2 {
        wdir = strip(wide_cap(&u, WPATH)); // a folder
    } else {
        match u.rfind('\\') {
            Some(k) => {
                wfile = strip(wide_cap(&u[k + 1..], WPATH));
                wdir = strip(wide_cap(&u[..k], WPATH));
            }
            None => wfile = strip(wide_cap(&u, WPATH)),
        }
    }
    (wdir, wfile)
}
