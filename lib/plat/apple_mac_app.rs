// apple_mac_app -- the app in macOS (docs/APPLE.md AP7a): its menu, its delegate, the system's events,
// tool windows as panels, the print dialog
//
//   gui_once()                (win_create, the first window) the main menu: the app menu (About, Hide,
//                             Hide Others, Show All, Quit: Cmd+Q) and the Window menu (Minimize: Cmd+M,
//                             Zoom, Bring All to Front), in German or English after the system's first
//                             language; SIGTERM becomes E_SESSION_END (a dispatch source on the main
//                             queue: the run loop delivers it)
//   observe(obs)              (init) the observer (a delegate object of ours, index -1) is NSApp's
//                             delegate and hears NSWorkspace: will sleep / did wake -> E_POWER 1 / 0,
//                             will power off -> E_SESSION_END
//   delegate methods          applicationShouldTerminate: (Cmd+Q, the Dock's Quit, logging out): E_CLOSE
//                             of the main window, the app decides (NSTerminateCancel); without a main
//                             window the app ends.  applicationShouldHandleReopen:hasVisibleWindows:
//                             (a click on the Dock icon) brings the main window back
//   panel_class()             PWF_TOOL windows are panels of our class JFPanel: they become the key
//                             window only for a click into a text field (the hit hook's HB_TEXT), as on
//                             Windows (MA_NOACTIVATE); JFPanel's sendEvent: notes the event
//                             needsPanelToBecomeKey looks at
//   print_dialog(job)         (apple_disp.rs print) NSPrintPanel: printer, paper, orientation, copies;
//                             the job laid out on the chosen paper and its printable area as a one-page
//                             PDF (print.rs), printed through PDFKit without a second panel
//   print_dialog_autoclose(ms) (test aid) the next print panel is cancelled after ms (NSApp abortModal)

use core::ffi::{CStr, c_void};
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};

use crate::apple::objc::{Id, NIL, NSRect, NSSize, Sel, class, msg, nsstring, release, sel, string_of};
use super::{app, ev_pos, ids, push, wake_app, with};
use crate::{E_CLOSE, E_POWER, E_SESSION_END, HB_TEXT, PJF_LANDSCAPE, PWS_SHOW};

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    static NSWorkspaceWillSleepNotification: Id;
    static NSWorkspaceDidWakeNotification: Id;
    static NSWorkspaceWillPowerOffNotification: Id;
}
#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {
    static NSRunLoopCommonModes: Id;
}
#[link(name = "PDFKit", kind = "framework")]
unsafe extern "C" {}
#[repr(C)]
struct ObjcSuper {
    receiver: Id,
    class: Id,
}
#[link(name = "objc")]
unsafe extern "C" {
    fn objc_msgSendSuper();
}
// libdispatch (libSystem): a signal source on the main queue
unsafe extern "C" {
    static _dispatch_source_type_signal: c_void;
    static _dispatch_main_q: c_void;
    fn dispatch_source_create(ty: *const c_void, handle: usize, mask: usize, queue: *const c_void) -> *mut c_void;
    fn dispatch_source_set_event_handler_f(src: *mut c_void, handler: extern "C" fn(*mut c_void));
    fn dispatch_resume(obj: *mut c_void);
    fn signal(sig: i32, handler: usize) -> usize;
}

const COMMAND: u64 = 1 << 20;
const OPTION: u64 = 1 << 19;
const SIGTERM: i32 = 15;
const SIG_IGN: usize = 1;
const TERMINATE_CANCEL: usize = 0;
const TERMINATE_NOW: usize = 1;
const PANEL_OPTIONS: usize = 1 | 4 | 8 | 1 << 17; // NSPrintPanel: copies, paper size, orientation, preview

static GUI: AtomicBool = AtomicBool::new(false);
static PANEL_CLS: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static PANEL_EV: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static PRINT_AUTOCLOSE: AtomicU32 = AtomicU32::new(0);

// ---------------------------------------------------------------- the menu, SIGTERM

/// (win_create) once: the main menu and SIGTERM
pub(super) fn gui_once() {
    if GUI.swap(true, Ordering::SeqCst) {
        return;
    }
    menus();
    // SAFETY: SIGTERM ignored by the default action; a dispatch source on the main queue reports it.
    unsafe {
        signal(SIGTERM, SIG_IGN);
        let src = dispatch_source_create(&_dispatch_source_type_signal, SIGTERM as usize, 0, &_dispatch_main_q);
        if !src.is_null() {
            dispatch_source_set_event_handler_f(src, on_sigterm);
            dispatch_resume(src);
        }
    }
}

extern "C" fn on_sigterm(_: *mut c_void) {
    push(0, E_SESSION_END, 0, 0, 0, 0);
    wake_app();
}

/// the words of the menus: German or English after the system's first language
fn words() -> [&'static str; 9] {
    let de = crate::system_languages().first().is_some_and(|l| l.to_ascii_lowercase().starts_with("de"));
    if de {
        ["Über %", "% ausblenden", "Andere ausblenden", "Alle einblenden", "% beenden", "Fenster", "Im Dock ablegen", "Zoomen",
         "Alle nach vorne bringen"]
    } else {
        ["About %", "Hide %", "Hide Others", "Show All", "Quit %", "Window", "Minimize", "Zoom", "Bring All to Front"]
    }
}

/// a menu item (+1): title, action (nil target: the responder chain), key equivalent and its modifiers
fn item(title: &str, action: &CStr, key: &str, mods: u64) -> Id {
    let (t, k) = (nsstring(title), nsstring(key));
    // SAFETY: a menu item of ours with strings released after.
    let it = unsafe {
        let it: Id = msg!(class(c"NSMenuItem"), c"alloc"; Id);
        let it: Id = msg!(it, c"initWithTitle:action:keyEquivalent:", t => Id, sel(action) => Sel, k => Id; Id);
        if mods != 0 && !it.is_null() {
            msg!(it, c"setKeyEquivalentModifierMask:", mods => u64; ());
        }
        it
    };
    release(t);
    release(k);
    it
}

/// a menu of these items (None: a separator) under its title, as an item of the bar
fn submenu(bar: Id, title: &str, items: &[Option<Id>]) -> Id {
    let t = nsstring(title);
    // SAFETY: AppKit objects of ours; the menus keep what they hold.
    let menu = unsafe {
        let menu: Id = msg!(class(c"NSMenu"), c"alloc"; Id);
        let menu: Id = msg!(menu, c"initWithTitle:", t => Id; Id);
        for it in items {
            match it {
                Some(it) => {
                    msg!(menu, c"addItem:", *it => Id; ());
                    release(*it);
                }
                None => {
                    let sep: Id = msg!(class(c"NSMenuItem"), c"separatorItem"; Id);
                    msg!(menu, c"addItem:", sep => Id; ());
                }
            }
        }
        let top: Id = msg!(class(c"NSMenuItem"), c"new"; Id);
        msg!(top, c"setSubmenu:", menu => Id; ());
        msg!(bar, c"addItem:", top => Id; ());
        release(top);
        menu
    };
    release(t);
    menu
}

fn menus() {
    let Some(app) = app() else { return };
    let w = words();
    // SAFETY: the process name; the menus made here are kept by the application.
    unsafe {
        let pool = crate::apple::objc::objc_autoreleasePoolPush();
        let pi: Id = msg!(class(c"NSProcessInfo"), c"processInfo"; Id);
        let name = string_of(msg!(pi, c"processName"; Id));
        let n = |s: &str| s.replace('%', &name);
        let bar: Id = msg!(class(c"NSMenu"), c"new"; Id);
        let app_menu = submenu(bar, &name, &[
            Some(item(&n(w[0]), c"orderFrontStandardAboutPanel:", "", 0)),
            None,
            Some(item(&n(w[1]), c"hide:", "h", COMMAND)),
            Some(item(w[2], c"hideOtherApplications:", "h", COMMAND | OPTION)),
            Some(item(w[3], c"unhideAllApplications:", "", 0)),
            None,
            Some(item(&n(w[4]), c"terminate:", "q", COMMAND)),
        ]);
        let win_menu = submenu(bar, w[5], &[
            Some(item(w[6], c"performMiniaturize:", "m", COMMAND)),
            Some(item(w[7], c"performZoom:", "", 0)),
            None,
            Some(item(w[8], c"arrangeInFront:", "", 0)),
        ]);
        msg!(app, c"setMainMenu:", bar => Id; ());
        msg!(app, c"setWindowsMenu:", win_menu => Id; ());
        release(app_menu);
        release(win_menu);
        release(bar);
        crate::apple::objc::objc_autoreleasePoolPop(pool);
    }
}

// ---------------------------------------------------------------- the delegate, the system's events

/// (init) the observer becomes NSApp's delegate and hears NSWorkspace
pub(super) fn observe(app: Id, obs: Id) {
    // SAFETY: NSApp keeps its delegate weakly: the observer lives for the process.
    unsafe {
        msg!(app, c"setDelegate:", obs => Id; ());
        let ws: Id = msg!(class(c"NSWorkspace"), c"sharedWorkspace"; Id);
        let nc: Id = if ws.is_null() { NIL } else { msg!(ws, c"notificationCenter"; Id) };
        if nc.is_null() {
            return;
        }
        for (name, s) in [(NSWorkspaceWillSleepNotification, c"jfWillSleep:"), (NSWorkspaceDidWakeNotification, c"jfDidWake:"),
                          (NSWorkspaceWillPowerOffNotification, c"jfWillPowerOff:")] {
            msg!(nc, c"addObserver:selector:name:object:", obs => Id, sel(s) => Sel, name => Id, NIL => Id; ());
        }
    }
}

extern "C" fn d_will_sleep(_this: Id, _cmd: Sel, _n: Id) {
    push(0, E_POWER, 0, 0, 0, 1);
    wake_app();
}
extern "C" fn d_did_wake(_this: Id, _cmd: Sel, _n: Id) {
    push(0, E_POWER, 0, 0, 0, 0);
    wake_app();
}
extern "C" fn d_will_power_off(_this: Id, _cmd: Sel, _n: Id) {
    push(0, E_SESSION_END, 0, 0, 0, 0);
    wake_app();
}

/// Cmd+Q, the Dock's Quit, logging out: the main window is asked to close; the app decides
extern "C" fn d_should_terminate(_this: Id, _cmd: Sel, _app: Id) -> usize {
    if ids(0).is_none() {
        return TERMINATE_NOW;
    }
    push(0, E_CLOSE, 0, 0, 0, 0);
    wake_app();
    TERMINATE_CANCEL
}

/// a click on the Dock icon with no window shown: the main window comes back
extern "C" fn d_reopen(_this: Id, _cmd: Sel, _app: Id, visible: bool) -> bool {
    if !visible && ids(0).is_some() {
        super::win_show(0, PWS_SHOW);
    }
    true
}

/// the observer's methods (apple_mac.rs delegate_methods)
pub(super) fn delegate_methods() -> [(&'static CStr, *const c_void, &'static CStr); 5] {
    let n = c"v@:@";
    [
        (c"jfWillSleep:", d_will_sleep as *const c_void, n),
        (c"jfDidWake:", d_did_wake as *const c_void, n),
        (c"jfWillPowerOff:", d_will_power_off as *const c_void, n),
        (c"applicationShouldTerminate:", d_should_terminate as *const c_void, c"Q@:@"),
        (c"applicationShouldHandleReopen:hasVisibleWindows:", d_reopen as *const c_void, c"B@:@B"),
    ]
}

// ---------------------------------------------------------------- tool windows as panels

/// JFPanel: an NSPanel whose sendEvent: notes the event (needsPanelToBecomeKey reads it)
pub(super) fn panel_class() -> Id {
    let c = PANEL_CLS.load(Ordering::SeqCst);
    if !c.is_null() {
        return c;
    }
    let (cls, _) = crate::apple::objc::subclass(c"NSPanel", c"JFPanel", &[(c"sendEvent:", p_send_event as *const c_void, c"v@:@")], &[]);
    PANEL_CLS.store(cls, Ordering::SeqCst);
    cls
}

extern "C" fn p_send_event(this: Id, cmd: Sel, ev: Id) {
    PANEL_EV.store(ev, Ordering::SeqCst);
    let sup = ObjcSuper { receiver: this, class: class(c"NSPanel") };
    // SAFETY: [super sendEvent:ev] through objc_msgSendSuper with its exact signature.
    unsafe {
        let f: unsafe extern "C" fn(*const ObjcSuper, Sel, Id) = core::mem::transmute(objc_msgSendSuper as *const ());
        f(&sup, cmd, ev);
    }
    PANEL_EV.store(core::ptr::null_mut(), Ordering::SeqCst);
}

/// (the view's needsPanelToBecomeKey) a click into a text field (HB_TEXT) makes the panel the key
/// window, other clicks leave the keyboard where it is
pub(super) extern "C" fn v_needs_panel_key(this: Id, _cmd: Sel) -> bool {
    let ev = PANEL_EV.load(Ordering::SeqCst);
    // SAFETY: the type of the event being sent (alive during the panel's sendEvent:).
    let down = !ev.is_null() && matches!(unsafe { msg!(ev, c"type"; u64) }, 1 | 3 | 25);
    if !down {
        return false;
    }
    let Some((w, x, y)) = ev_pos(this, ev) else { return false };
    let hit = with(|s| s.hit).flatten();
    hit.is_some_and(|h| h(w, x, y) & HB_TEXT != 0)
}

// ---------------------------------------------------------------- printing

/// (test aid) the next print panel is cancelled after ms
pub fn print_dialog_autoclose(ms: u32) {
    PRINT_AUTOCLOSE.store(ms.max(1), Ordering::SeqCst);
}

/// (apple_disp.rs print) the system's print panel, then the job on the chosen page as a PDF printed by
/// PDFKit -> 0 printed / ERR_CANCEL / ERR_UNSUPPORTED / ERR_IO / print_pdf's errors
pub(crate) fn print_dialog(job: &mut crate::PrintJob<'_>) -> i32 {
    if !super::init() {
        return crate::ERR_UNSUPPORTED;
    }
    let suggest = crate::page_init(job.paper, job.flags);
    // SAFETY: a pool of ours; NSPrintInfo / NSPrintPanel / PDFDocument objects made here.
    unsafe {
        let pool = crate::apple::objc::objc_autoreleasePoolPush();
        let r = print_in_pool(job, &suggest);
        crate::apple::objc::objc_autoreleasePoolPop(pool);
        r
    }
}

/// print_dialog's work inside its pool
unsafe fn print_in_pool(job: &mut crate::PrintJob<'_>, suggest: &crate::PPage) -> i32 {
    // SAFETY: (the caller's pool) AppKit's print objects, each message with its exact signature.
    unsafe {
        let shared: Id = msg!(class(c"NSPrintInfo"), c"sharedPrintInfo"; Id);
        let info: Id = msg!(shared, c"copy"; Id); // +1, released below
        if info.is_null() {
            return crate::ERR_UNSUPPORTED;
        }
        msg!(info, c"setPaperSize:", NSSize { w: suggest.w, h: suggest.h } => NSSize; ());
        msg!(info, c"setOrientation:", (suggest.flags & PJF_LANDSCAPE != 0) as isize => isize; ());
        let panel: Id = msg!(class(c"NSPrintPanel"), c"printPanel"; Id);
        // copies, page range off; paper size, orientation, preview
        msg!(panel, c"setOptions:", PANEL_OPTIONS => usize; ());
        let ms = PRINT_AUTOCLOSE.swap(0, Ordering::SeqCst);
        if ms != 0 {
            let modes: Id = msg!(class(c"NSArray"), c"arrayWithObject:", NSRunLoopCommonModes => Id; Id);
            let app: Id = msg!(class(c"NSApplication"), c"sharedApplication"; Id);
            msg!(app, c"performSelector:withObject:afterDelay:inModes:", sel(c"abortModal") => Sel, NIL => Id, ms as f64 / 1000.0 => f64,
                 modes => Id; ());
        }
        let r: isize = msg!(panel, c"runModalWithPrintInfo:", info => Id; isize);
        if r != 1 {
            release(info);
            return crate::ERR_CANCEL;
        }
        let size: NSSize = msg!(info, c"paperSize"; NSSize);
        let ib: NSRect = msg!(info, c"imageablePageBounds"; NSRect);
        let landscape = size.w > size.h;
        let page = crate::PPage {
            w: size.w,
            h: size.h,
            l: ib.origin.x.max(0.0),
            b: ib.origin.y.max(0.0),
            r: (size.w - ib.origin.x - ib.size.w).max(0.0),
            t: (size.h - ib.origin.y - ib.size.h).max(0.0),
            token: 0,
            flags: if landscape { PJF_LANDSCAPE } else { 0 },
        };
        let pdf = match crate::print_pdf(job, &page) {
            Ok(p) => p,
            Err(e) => {
                release(info);
                return e;
            }
        };
        let data: Id = msg!(class(c"NSData"), c"dataWithBytes:length:", pdf.as_ptr() => *const u8, pdf.len() => usize; Id);
        let doc: Id = msg!(class(c"PDFDocument"), c"alloc"; Id);
        let doc: Id = if doc.is_null() { NIL } else { msg!(doc, c"initWithData:", data => Id; Id) };
        if doc.is_null() {
            release(info);
            return crate::ERR_IO;
        }
        // the PDF has the page's size: no scaling, no rotation; the panel was shown already
        msg!(info, c"setTopMargin:", 0.0f64 => f64; ());
        msg!(info, c"setBottomMargin:", 0.0f64 => f64; ());
        msg!(info, c"setLeftMargin:", 0.0f64 => f64; ());
        msg!(info, c"setRightMargin:", 0.0f64 => f64; ());
        let op: Id = msg!(doc, c"printOperationForPrintInfo:scalingMode:autoRotate:", info => Id, 0isize => isize, false => bool; Id);
        let ok = !op.is_null() && {
            msg!(op, c"setShowsPrintPanel:", false => bool; ());
            msg!(op, c"setShowsProgressPanel:", true => bool; ());
            msg!(op, c"runOperation"; bool)
        };
        let copies = {
            let dict: Id = msg!(info, c"dictionary"; Id);
            let key = nsstring("NSCopies");
            let v: Id = if dict.is_null() { NIL } else { msg!(dict, c"objectForKey:", key => Id; Id) };
            release(key);
            if v.is_null() { 1 } else { msg!(v, c"integerValue"; isize).max(1) as u32 }
        };
        release(doc);
        release(info);
        if !ok {
            return crate::ERR_IO;
        }
        job.copies = copies;
        0
    }
}
