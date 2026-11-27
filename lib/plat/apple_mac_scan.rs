// apple_mac_scan -- (scan.rs) macOS's scan dialog: ImageKit over ImageCaptureCore
//
//   available() -> bool            ImageKit's scanner view is there (Quartz.framework, loaded at
//                                  the first use)
//   start(title) -> Result         a window of its own: IKDeviceBrowserView (local and network
//                                  scanners, no cameras) beside IKScannerDeviceView (the dialog
//                                  Preview shows: overview, area, resolution, format, "Scan")
//   mac_scan_shown() / mac_scan_close()   (tests) the window on screen, closed as by its close
//                                  button
//
// The device chosen in the browser goes to the scanner view (its document UTI set to PNG first;
// the view's format menu may change it).  The view scans into a folder of its own below the temp
// folder (file-based transfer); the first file it reports is read, removed and answered, and the
// window closes.  Closing the window without a scan answers Cancelled; an error of the scan
// answers Failed.  Everything runs on the main thread; the delegate is one object for the
// process.

use alloc::format;
use alloc::string::String;
use core::ffi::{c_char, c_void};
use core::sync::atomic::{AtomicPtr, AtomicU8, AtomicU32, Ordering};

use rt::sync::SpinLock;

use super::super::objc::{Id, NIL, NSRect, NSSize, Sel, class, msg, nsstring, release, sel, string_of, subclass};
use crate::scan::{ScanError, Scanned, answer};

unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
}

const RTLD_LAZY: i32 = 1;
/// NSWindowStyleMask: titled, closable, miniaturizable, resizable
const STYLE: u64 = 1 | 2 | 4 | 8;
/// NSViewWidthSizable, NSViewHeightSizable, NSViewMaxXMargin
const SIZE_W: u64 = 2;
const SIZE_H: u64 = 16;
const MARGIN_MAX_X: u64 = 4;
/// the device list keeps its width, the scanner view takes the rest
const LIST_MASK: u64 = SIZE_H | MARGIN_MAX_X;
const VIEW_MASK: u64 = SIZE_W | SIZE_H;
/// the device list's width, the window's size (points)
const LIST_W: f64 = 240.0;
const WIN_W: f64 = 980.0;
const WIN_H: f64 = 620.0;

/// ImageKit loaded: 0 not yet, 1 there, 2 not there
static KIT: AtomicU8 = AtomicU8::new(0);
static DELEGATE: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
/// the open window and its scanner view (retained by us until the window closes)
static WINDOW: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static SCANNER: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static BROWSER: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
/// the folder the scanner view writes into
static DIR: SpinLock<String> = SpinLock::new(String::new());
/// a dialog's answer was given (the window's close then answers nothing more)
static ANSWERED: AtomicU8 = AtomicU8::new(0);
static SERIAL: AtomicU32 = AtomicU32::new(0);

pub fn available() -> bool {
    match KIT.load(Ordering::Acquire) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    // SAFETY: loading a system framework (ImageKit inside Quartz registers its classes).
    unsafe { dlopen(c"/System/Library/Frameworks/Quartz.framework/Quartz".as_ptr(), RTLD_LAZY) };
    let ok = !class(c"IKScannerDeviceView").is_null() && !class(c"IKDeviceBrowserView").is_null();
    KIT.store(if ok { 1 } else { 2 }, Ordering::Release);
    ok
}

fn delegate() -> Id {
    let d = DELEGATE.load(Ordering::Acquire);
    if !d.is_null() {
        return d;
    }
    type Imp = *const c_void;
    let (cls, _) = subclass(c"NSObject", c"JFScanDelegate", &[
        (c"deviceBrowserView:selectionDidChange:", selection as Imp, c"v@:@@"),
        (c"deviceBrowserView:didEncounterError:", browser_error as Imp, c"v@:@@"),
        (c"scannerDeviceView:didScanToURL:error:", scanned as Imp, c"v@:@@@"),
        (c"scannerDeviceView:didEncounterError:", scanner_error as Imp, c"v@:@@"),
        (c"windowWillClose:", closing as Imp, c"v@:@"),
    ], &[c"IKDeviceBrowserViewDelegate", c"IKScannerDeviceViewDelegate", c"NSWindowDelegate"]);
    // SAFETY: an instance of our class, kept for the process.
    let d: Id = unsafe { msg!(msg!(cls, c"alloc"; Id), c"init"; Id) };
    DELEGATE.store(d, Ordering::Release);
    d
}

/// a folder of its own for this dialog's files (below the temp folder) -> its path
fn scan_dir() -> Option<String> {
    let tmp = crate::temp_dir()?;
    let dir = format!("{}/jfgui-scan-{}-{}", tmp.trim_end_matches('/'), crate::process_id(), SERIAL.fetch_add(1, Ordering::Relaxed));
    clear_dir(&dir);
    crate::mkdir_raw(&dir).then_some(dir)
}

/// the folder's files removed, then the folder
fn clear_dir(dir: &str) {
    let mut names = alloc::vec::Vec::new();
    crate::dir_list_raw(dir, &mut |n, is_dir, _| {
        if !is_dir {
            names.push(String::from(n));
        }
    });
    for n in names {
        crate::delete_raw(&format!("{dir}/{n}"));
    }
    crate::delete_raw(dir);
}

pub fn start(title: &str) -> Result<(), ScanError> {
    if !available() {
        return Err(ScanError::Unavailable);
    }
    let Some(dir) = scan_dir() else { return Err(ScanError::Failed(String::from("no folder for the scan"))) };
    let title = String::from(title);
    super::a11y_mac::on_main(move || {
        // the application object (an app without windows of its own yet makes it here)
        if !super::init() {
            clear_dir(&dir);
            return Err(ScanError::Unavailable);
        }
        // a window left from an earlier dialog (closed already: only our references)
        drop_window();
        ANSWERED.store(0, Ordering::Release);
        *DIR.lock() = dir.clone();
        // SAFETY: AppKit and ImageKit objects made on the main thread with their exact signatures;
        // the window, the browser and the scanner view are retained by us until the window closes.
        unsafe {
            let d = delegate();
            let win: Id = msg!(class(c"NSWindow"), c"alloc"; Id);
            let win: Id = msg!(win, c"initWithContentRect:styleMask:backing:defer:", NSRect::new(0.0, 0.0, WIN_W, WIN_H) => NSRect, STYLE => u64,
                               2u64 => u64, false => bool; Id);
            if win.is_null() {
                clear_dir(&dir);
                return Err(ScanError::Failed(String::from("NSWindow")));
            }
            msg!(win, c"setReleasedWhenClosed:", false => bool; ());
            let t = nsstring(&title);
            msg!(win, c"setTitle:", t => Id; ());
            release(t);
            msg!(win, c"setMinSize:", NSSize { w: 720.0, h: 460.0 } => NSSize; ());
            let content: Id = msg!(win, c"contentView"; Id);

            let browser: Id = msg!(msg!(class(c"IKDeviceBrowserView"), c"alloc"; Id), c"initWithFrame:", NSRect::new(0.0, 0.0, LIST_W, WIN_H) => NSRect; Id);
            let scanner: Id = msg!(msg!(class(c"IKScannerDeviceView"), c"alloc"; Id), c"initWithFrame:", NSRect::new(LIST_W, 0.0, WIN_W - LIST_W, WIN_H) => NSRect; Id);
            if browser.is_null() || scanner.is_null() {
                for v in [browser, scanner, win] {
                    if !v.is_null() {
                        release(v);
                    }
                }
                clear_dir(&dir);
                return Err(ScanError::Failed(String::from("ImageKit's views")));
            }
            msg!(browser, c"setAutoresizingMask:", LIST_MASK => u64; ());
            msg!(browser, c"setDisplaysLocalCameras:", false => bool; ());
            msg!(browser, c"setDisplaysNetworkCameras:", false => bool; ());
            msg!(browser, c"setDisplaysLocalScanners:", true => bool; ());
            msg!(browser, c"setDisplaysNetworkScanners:", true => bool; ());
            msg!(browser, c"setDelegate:", d => Id; ());

            msg!(scanner, c"setAutoresizingMask:", VIEW_MASK => u64; ());
            msg!(scanner, c"setTransferMode:", 0isize => isize; ()); // IKScannerDeviceViewTransferModeFileBased
            msg!(scanner, c"setDisplaysDownloadsDirectoryControl:", false => bool; ());
            msg!(scanner, c"setDisplaysPostProcessApplicationControl:", false => bool; ());
            let p = nsstring(&dir);
            let url: Id = msg!(class(c"NSURL"), c"fileURLWithPath:isDirectory:", p => Id, true => bool; Id);
            release(p);
            msg!(scanner, c"setDownloadsDirectory:", url => Id; ());
            let name = nsstring("Scan");
            msg!(scanner, c"setDocumentName:", name => Id; ());
            release(name);
            msg!(scanner, c"setDelegate:", d => Id; ());

            msg!(content, c"addSubview:", browser => Id; ());
            msg!(content, c"addSubview:", scanner => Id; ());
            msg!(win, c"setDelegate:", d => Id; ());
            msg!(win, c"center"; ());
            msg!(win, c"makeKeyAndOrderFront:", NIL => Id; ());
            let app: Id = msg!(class(c"NSApplication"), c"sharedApplication"; Id);
            msg!(app, c"activateIgnoringOtherApps:", true => bool; ());
            BROWSER.store(browser, Ordering::Release);
            SCANNER.store(scanner, Ordering::Release);
            WINDOW.store(win, Ordering::Release);
        }
        Ok(())
    })
}

/// the views and the window let go (main thread; the window is closed already or closes now)
fn drop_window() {
    let win = WINDOW.swap(core::ptr::null_mut(), Ordering::AcqRel);
    let scanner = SCANNER.swap(core::ptr::null_mut(), Ordering::AcqRel);
    let browser = BROWSER.swap(core::ptr::null_mut(), Ordering::AcqRel);
    // SAFETY: our retained objects; their delegates cleared first (no calls into a dialog that is
    // gone), the scanner's device let go (its session closes).
    unsafe {
        if !scanner.is_null() {
            msg!(scanner, c"setDelegate:", NIL => Id; ());
            msg!(scanner, c"setScannerDevice:", NIL => Id; ());
            msg!(scanner, c"autorelease"; Id);
        }
        if !browser.is_null() {
            msg!(browser, c"setDelegate:", NIL => Id; ());
            msg!(browser, c"autorelease"; Id);
        }
        if !win.is_null() {
            msg!(win, c"setDelegate:", NIL => Id; ());
            msg!(win, c"autorelease"; Id);
        }
    }
}

/// answer once, the window closed (unless it is closing already), the folder cleared
fn finish(r: Result<Scanned, ScanError>, close: bool) {
    if ANSWERED.swap(1, Ordering::AcqRel) != 0 {
        return;
    }
    answer(r);
    let win = WINDOW.load(Ordering::Acquire);
    if close && !win.is_null() {
        // SAFETY: our window, closed on the main thread (windowWillClose: finds the answer given).
        unsafe { msg!(win, c"close"; ()) };
    }
    drop_window();
    let dir = core::mem::take(&mut *DIR.lock());
    if !dir.is_empty() {
        clear_dir(&dir);
    }
}

/// a device chosen in the browser: a scanner goes to the scanner view (PNG unless changed there)
extern "C" fn selection(_this: Id, _sel: Sel, _browser: Id, device: Id) {
    let scanner = SCANNER.load(Ordering::Acquire);
    if scanner.is_null() {
        return;
    }
    // SAFETY: the device the browser hands over and our scanner view, on the main thread.
    unsafe {
        let is_scanner = !device.is_null() && {
            let c = class(c"ICScannerDevice");
            !c.is_null() && msg!(device, c"isKindOfClass:", c => Id; bool)
        };
        if is_scanner {
            if msg!(device, c"respondsToSelector:", sel(c"setDocumentUTI:") => Sel; bool) {
                let uti = nsstring("public.png");
                msg!(device, c"setDocumentUTI:", uti => Id; ());
                release(uti);
            }
            msg!(scanner, c"setScannerDevice:", device => Id; ());
        } else {
            msg!(scanner, c"setScannerDevice:", NIL => Id; ());
        }
    }
}

extern "C" fn browser_error(_this: Id, _sel: Sel, _browser: Id, _error: Id) {}

/// a device's trouble that does not end the dialog (busy, gone): the view shows it itself
extern "C" fn scanner_error(_this: Id, _sel: Sel, _view: Id, _error: Id) {}

/// the scan written to url (or an error)
extern "C" fn scanned(_this: Id, _sel: Sel, _view: Id, url: Id, error: Id) {
    // SAFETY: the URL and the error the view hands over (their strings copied), on the main thread.
    let (path, err) = unsafe {
        let path = if url.is_null() { String::new() } else { string_of(msg!(url, c"path"; Id)) };
        let err = if error.is_null() { None } else { Some(string_of(msg!(error, c"localizedDescription"; Id))) };
        (path, err)
    };
    if let Some(e) = err {
        finish(Err(ScanError::Failed(e)), true);
        return;
    }
    let r = match crate::read_file_raw(&path) {
        Some(data) if !data.is_empty() => {
            let name = String::from(path.rsplit('/').next().unwrap_or("Scan"));
            crate::delete_raw(&path);
            Ok(Scanned { name, data })
        }
        _ => Err(ScanError::Failed(format!("the scan is not readable: {path}"))),
    };
    finish(r, true);
}

/// the window closes: without an answer yet that is Cancelled
extern "C" fn closing(_this: Id, _sel: Sel, _note: Id) {
    if ANSWERED.load(Ordering::Acquire) == 0 {
        finish(Err(ScanError::Cancelled), false);
    }
}

/// (tests) the scan window is on screen
pub fn mac_scan_shown() -> bool {
    super::a11y_mac::on_main(|| {
        let win = WINDOW.load(Ordering::Acquire);
        // SAFETY: our window, asked on the main thread.
        !win.is_null() && unsafe { msg!(win, c"isVisible"; bool) }
    })
}

/// (tests) the scan window closed as by its close button -> false when none is open
pub fn mac_scan_close() -> bool {
    super::a11y_mac::on_main(|| {
        let win = WINDOW.load(Ordering::Acquire);
        if win.is_null() {
            return false;
        }
        // SAFETY: our window: performClose: as the close button does (windowWillClose: follows).
        unsafe { msg!(win, c"performClose:", NIL => Id; ()) };
        true
    })
}
