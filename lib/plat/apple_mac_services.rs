// apple_mac_services -- macOS services of the app (docs/APPLE.md AP7b): the status item (tray.rs),
// notifications, the Dock tile (taskbar.rs), files and URLs the system opens with the app, global
// shortcuts (hotkey.rs)
//
//   tray::*      an NSStatusItem in the menu bar: the icon (TrayIcon, else the app's icon) 18 points
//                high, the tooltip, the menu (NSMenu of the TrayItem tree: TrayEvent::Menu(id)); a symbol
//                without a menu reports a click as Activate.  Notifications through
//                UNUserNotificationCenter -- only in an app bundle (macOS refuses a program without a
//                bundle identifier): elsewhere notify answers 0.  A click on one: NoteClicked(id); one
//                arriving while the app is in front is shown all the same (banner, list).
//   dock::*      the Dock tile: the badge (badgeLabel: the count), progress drawn into the tile (the
//                app's icon with a bar: blue, yellow paused, red error, grey busy), the tasks as the
//                Dock menu (applicationDockMenu:) -- choosing one hands its arguments to the app as a
//                later start would (instance::queue_push -> UIC_OPEN)
//   openURLs     application:openURLs: (a document opened with the app, dropped onto its Dock icon, a
//                URL of its scheme): the URLs reach the app as a later start's arguments (UIC_OPEN)
//   hotkey::*    Carbon's RegisterEventHotKey (no permission needed): Command is MOD_CTRL, Control
//                MOD_SUPER, as for the keys; a chord another program holds is Conflict.  The presses
//                come through the application's event target: HotkeyEvent::Pressed(id)
//
// AppKit's and Carbon's objects: the main (UI) thread only.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ffi::{CStr, c_void};

use crate::apple::objc::{Id, NIL, NSRect, NSSize, Sel, class, msg, nsstring, release, sel, string_of, subclass};

#[link(name = "UserNotifications", kind = "framework")]
unsafe extern "C" {}
unsafe extern "C" {
    static _NSConcreteGlobalBlock: c_void;
}

/// state of the services (main thread only)
struct S {
    target: Id,          // JFServiceTarget: the actions of the status item, its menu and the Dock menu
    item: Id,            // the NSStatusItem
    tasks: Vec<crate::taskbar::JumpTask>,
    progress: crate::taskbar::TaskbarProgress,
    dock_view: Id,       // JFDockView (progress) while shown
    notes: bool,         // the notification centre's delegate set, authorization asked
    hotkeys: Vec<(u32, *mut c_void)>,
    handler: bool,       // the hot key handler installed
}

struct Cell(UnsafeCell<S>);
// SAFETY: the main thread only (AppKit's and Carbon's thread); no borrow spans a call into them.
unsafe impl Sync for Cell {}
static ST: Cell = Cell(UnsafeCell::new(S {
    target: core::ptr::null_mut(),
    item: core::ptr::null_mut(),
    tasks: Vec::new(),
    progress: crate::taskbar::TaskbarProgress::None,
    dock_view: core::ptr::null_mut(),
    notes: false,
    hotkeys: Vec::new(),
    handler: false,
}));

fn st<R>(f: impl FnOnce(&mut S) -> R) -> R {
    // SAFETY: main thread only; f never calls into AppKit.
    unsafe { f(&mut *ST.0.get()) }
}

/// the target object of our actions (made once)
fn target() -> Id {
    let t = st(|s| s.target);
    if !t.is_null() {
        return t;
    }
    let (cls, _) = subclass(c"NSObject", c"JFServiceTarget", &[
        (c"jfTrayItem:", t_tray_item as *const c_void, c"v@:@"),
        (c"jfTrayClick:", t_tray_click as *const c_void, c"v@:@"),
        (c"jfDockTask:", t_dock_task as *const c_void, c"v@:@"),
    ], &[]);
    // SAFETY: an object of our class, kept for the process.
    let t: Id = unsafe { msg!(cls, c"new"; Id) };
    st(|s| s.target = t);
    t
}

extern "C" fn t_tray_item(_this: Id, _cmd: Sel, sender: Id) {
    // SAFETY: the tag of the menu item that sent the action.
    let id = unsafe { msg!(sender, c"tag"; isize) };
    crate::tray::push(crate::tray::TrayEvent::Menu(id as u32));
}
extern "C" fn t_tray_click(_this: Id, _cmd: Sel, _sender: Id) {
    crate::tray::push(crate::tray::TrayEvent::Activate);
}
extern "C" fn t_dock_task(_this: Id, _cmd: Sel, sender: Id) {
    // SAFETY: the tag of the Dock menu's item.
    let k = unsafe { msg!(sender, c"tag"; isize) } as usize;
    if let Some(t) = st(|s| s.tasks.get(k).cloned()) {
        crate::instance::queue_push(crate::instance::Request { cwd: crate::cwd(), args: t.args });
    }
}

/// straight ARGB pixels as an image of `points` height (+1)
fn image_of(icon: &crate::tray::TrayIcon, points: f64) -> Id {
    let (w, h) = (icon.w as usize, icon.h as usize);
    if w == 0 || h == 0 || w > 1024 || h > 1024 || icon.px.len() < w * h {
        return NIL;
    }
    // SAFETY: a bitmap of our own (it allocates w * h * 4 bytes, filled here) in an image of ours.
    unsafe {
        let cs = nsstring("NSDeviceRGBColorSpace");
        let rep: Id = msg!(class(c"NSBitmapImageRep"), c"alloc"; Id);
        let rep: Id = msg!(rep, c"initWithBitmapDataPlanes:pixelsWide:pixelsHigh:bitsPerSample:samplesPerPixel:hasAlpha:isPlanar:colorSpaceName:bitmapFormat:bytesPerRow:bitsPerPixel:",
                           core::ptr::null_mut::<*mut u8>() => *mut *mut u8, w as isize => isize, h as isize => isize, 8isize => isize,
                           4isize => isize, true => bool, false => bool, cs => Id, 2u64 => u64, (w * 4) as isize => isize, 32isize => isize; Id);
        release(cs);
        if rep.is_null() {
            return NIL;
        }
        let dst: *mut u8 = msg!(rep, c"bitmapData"; *mut u8);
        if dst.is_null() {
            release(rep);
            return NIL;
        }
        // straight R, G, B, A bytes (NSBitmapFormatAlphaNonpremultiplied)
        let out = core::slice::from_raw_parts_mut(dst, w * h * 4);
        for (d, &p) in out.chunks_exact_mut(4).zip(icon.px.iter()) {
            d.copy_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]);
        }
        let size = NSSize { w: points * w as f64 / h as f64, h: points };
        let img: Id = msg!(class(c"NSImage"), c"alloc"; Id);
        let img: Id = msg!(img, c"initWithSize:", size => NSSize; Id);
        msg!(img, c"addRepresentation:", rep => Id; ());
        release(rep);
        img
    }
}

/// the app's own icon at `points` (+1)
fn app_icon(points: f64) -> Id {
    let Some(app) = super::app() else { return NIL };
    // SAFETY: a copy of the application's icon, resized.
    unsafe {
        let img: Id = msg!(app, c"applicationIconImage"; Id);
        if img.is_null() {
            return NIL;
        }
        let img: Id = msg!(img, c"copy"; Id);
        msg!(img, c"setSize:", NSSize { w: points, h: points } => NSSize; ());
        img
    }
}

// ---------------------------------------------------------------- the status item, notifications

pub(crate) mod tray {
    use super::*;
    use crate::tray::{Note, TI_CHECKED, TI_DISABLED, TI_SEPARATOR, Tray, TrayItem};

    /// the menu of these rows (+1)
    fn menu(items: &[TrayItem], tgt: Id) -> Id {
        // SAFETY: AppKit objects of ours; each item is kept by its menu.
        unsafe {
            let m: Id = msg!(class(c"NSMenu"), c"new"; Id);
            msg!(m, c"setAutoenablesItems:", false => bool; ());
            for it in items {
                if it.flags & TI_SEPARATOR != 0 {
                    let sep: Id = msg!(class(c"NSMenuItem"), c"separatorItem"; Id);
                    msg!(m, c"addItem:", sep => Id; ());
                    continue;
                }
                let (t, k) = (nsstring(&it.label), nsstring(""));
                let action = if it.sub.is_empty() { sel(c"jfTrayItem:") } else { core::ptr::null() };
                let mi: Id = msg!(class(c"NSMenuItem"), c"alloc"; Id);
                let mi: Id = msg!(mi, c"initWithTitle:action:keyEquivalent:", t => Id, action => Sel, k => Id; Id);
                release(t);
                release(k);
                msg!(mi, c"setTarget:", tgt => Id; ());
                msg!(mi, c"setTag:", it.id as isize => isize; ());
                msg!(mi, c"setState:", (it.flags & TI_CHECKED != 0) as isize => isize; ());
                msg!(mi, c"setEnabled:", it.flags & TI_DISABLED == 0 => bool; ());
                if !it.sub.is_empty() {
                    let sub = menu(&it.sub, tgt);
                    msg!(mi, c"setSubmenu:", sub => Id; ());
                    release(sub);
                }
                msg!(m, c"addItem:", mi => Id; ());
                release(mi);
            }
            m
        }
    }

    pub fn tray_set(t: &Tray) -> bool {
        if !super::super::init() {
            return false;
        }
        let tgt = target();
        // SAFETY: the system status bar's item (kept while shown), its button and menu.
        unsafe {
            let pool = crate::apple::objc::objc_autoreleasePoolPush();
            let mut item = st(|s| s.item);
            if item.is_null() {
                let bar: Id = msg!(class(c"NSStatusBar"), c"systemStatusBar"; Id);
                item = msg!(bar, c"statusItemWithLength:", -1.0f64 => f64; Id); // NSVariableStatusItemLength
                if item.is_null() {
                    crate::apple::objc::objc_autoreleasePoolPop(pool);
                    return false;
                }
                msg!(item, c"retain"; Id);
                st(|s| s.item = item);
            }
            let button: Id = msg!(item, c"button"; Id);
            let img = match &t.icon {
                Some(i) => image_of(i, 18.0),
                None => app_icon(18.0),
            };
            if !button.is_null() {
                msg!(button, c"setImage:", img => Id; ());
                let tip = nsstring(&t.tooltip);
                msg!(button, c"setToolTip:", tip => Id; ());
                release(tip);
                if img.is_null() {
                    let title = nsstring(if t.tooltip.is_empty() { "●" } else { &t.tooltip });
                    msg!(button, c"setTitle:", title => Id; ());
                    release(title);
                }
                // without a menu a click is Activate; with one the menu opens (the Mac's way)
                msg!(button, c"setTarget:", tgt => Id; ());
                msg!(button, c"setAction:", sel(c"jfTrayClick:") => Sel; ());
            }
            release(img);
            if t.menu.is_empty() {
                msg!(item, c"setMenu:", NIL => Id; ());
            } else {
                let m = menu(&t.menu, tgt);
                msg!(item, c"setMenu:", m => Id; ());
                release(m);
            }
            crate::apple::objc::objc_autoreleasePoolPop(pool);
        }
        true
    }

    pub fn tray_remove() {
        let item = st(|s| core::mem::replace(&mut s.item, NIL));
        if item.is_null() {
            return;
        }
        // SAFETY: our status item, taken off the bar and released.
        unsafe {
            let bar: Id = msg!(class(c"NSStatusBar"), c"systemStatusBar"; Id);
            msg!(bar, c"removeStatusItem:", item => Id; ());
        }
        release(item);
    }

    pub fn supported() -> bool {
        true
    }

    /// the bundle identifier (None: a program outside an app bundle -- no notifications then)
    pub(crate) fn bundle_id() -> Option<String> {
        // SAFETY: the main bundle's identifier (nil without one).
        let s = unsafe {
            let b: Id = msg!(class(c"NSBundle"), c"mainBundle"; Id);
            if b.is_null() { NIL } else { msg!(b, c"bundleIdentifier"; Id) }
        };
        (!s.is_null()).then(|| string_of(s)).filter(|s| !s.is_empty())
    }

    fn center() -> Id {
        // SAFETY: (only called with a bundle identifier: macOS raises otherwise) the shared centre.
        unsafe { msg!(class(c"UNUserNotificationCenter"), c"currentNotificationCenter"; Id) }
    }

    fn note_key(id: u32) -> String {
        format!("jflib-note-{id}")
    }

    pub fn notify(id: u32, n: &Note) -> bool {
        if bundle_id().is_none() || !super::super::init() {
            return false;
        }
        let c = center();
        if c.is_null() {
            return false;
        }
        // SAFETY: a pool of ours; the centre's calls with objects made here; blocks that live for the process.
        unsafe {
            let pool = crate::apple::objc::objc_autoreleasePoolPush();
            if !st(|s| core::mem::replace(&mut s.notes, true)) {
                let obs = super::super::observer();
                msg!(c, c"setDelegate:", obs => Id; ());
                // alert | sound; the answer does not matter here (macOS asks the user once)
                msg!(c, c"requestAuthorizationWithOptions:completionHandler:", 6usize => usize, block_noop2() => *const c_void; ());
            }
            let content: Id = msg!(class(c"UNMutableNotificationContent"), c"new"; Id);
            let (t, b) = (nsstring(&n.title), nsstring(&n.body));
            msg!(content, c"setTitle:", t => Id; ());
            msg!(content, c"setBody:", b => Id; ());
            release(t);
            release(b);
            let key = nsstring(&note_key(id));
            let req: Id = msg!(class(c"UNNotificationRequest"), c"requestWithIdentifier:content:trigger:", key => Id, content => Id, NIL => Id; Id);
            release(key);
            release(content);
            let ok = !req.is_null();
            if ok {
                msg!(c, c"addNotificationRequest:withCompletionHandler:", req => Id, core::ptr::null::<c_void>() => *const c_void; ());
            }
            crate::apple::objc::objc_autoreleasePoolPop(pool);
            ok
        }
    }

    pub fn notify_close(id: u32) {
        if bundle_id().is_none() {
            return;
        }
        let c = center();
        if c.is_null() {
            return;
        }
        // SAFETY: a pool of ours; the centre's removals with an array of one identifier.
        unsafe {
            let pool = crate::apple::objc::objc_autoreleasePoolPush();
            let key = nsstring(&note_key(id));
            let arr: Id = msg!(class(c"NSArray"), c"arrayWithObject:", key => Id; Id);
            msg!(c, c"removeDeliveredNotificationsWithIdentifiers:", arr => Id; ());
            msg!(c, c"removePendingNotificationRequestsWithIdentifiers:", arr => Id; ());
            release(key);
            crate::apple::objc::objc_autoreleasePoolPop(pool);
        }
    }

    /// (the centre's delegate) a click on one of ours: NoteClicked(id)
    pub(crate) fn note_response(identifier: &str, dismissed: bool) {
        if let Some(id) = identifier.strip_prefix("jflib-note-").and_then(|s| s.parse::<u32>().ok()) {
            crate::tray::push(if dismissed { crate::tray::TrayEvent::NoteClosed(id) } else { crate::tray::TrayEvent::NoteClicked(id) });
        }
    }
}

// ---- blocks (the notification centre's completion handlers)

#[repr(C)]
struct BlockDesc {
    reserved: usize,
    size: usize,
}

#[repr(C)]
struct Block {
    isa: *const c_void,
    flags: i32,
    reserved: i32,
    invoke: *const c_void,
    desc: *const BlockDesc,
}

static BLOCK_DESC: BlockDesc = BlockDesc { reserved: 0, size: core::mem::size_of::<Block>() };

extern "C" fn noop2(_b: *const c_void, _granted: bool, _err: Id) {}

/// a global block void (^)(BOOL, NSError *) that does nothing (made once, kept for the process)
fn block_noop2() -> *const c_void {
    static B: core::sync::atomic::AtomicPtr<c_void> = core::sync::atomic::AtomicPtr::new(core::ptr::null_mut());
    let b = B.load(core::sync::atomic::Ordering::SeqCst);
    if !b.is_null() {
        return b;
    }
    // (the address of the runtime's global block class: taking it needs no unsafe)
    let isa = core::ptr::addr_of!(_NSConcreteGlobalBlock);
    let blk = alloc::boxed::Box::new(Block { isa, flags: 1 << 28, reserved: 0, invoke: noop2 as *const c_void, desc: &BLOCK_DESC }); // BLOCK_IS_GLOBAL
    let p = alloc::boxed::Box::into_raw(blk) as *mut c_void;
    B.store(p, core::sync::atomic::Ordering::SeqCst);
    p
}

/// call a block the system handed us: void (^)(void) or, with Some(x), void (^)(NSUInteger)
unsafe fn call_block(b: *const c_void, arg: Option<usize>) {
    if b.is_null() {
        return;
    }
    // SAFETY: (the caller's block) its invoke pointer at offset 16 of the block literal.
    unsafe {
        let invoke = *(b as *const u8).add(16).cast::<*const c_void>();
        match arg {
            None => {
                let f: extern "C" fn(*const c_void) = core::mem::transmute(invoke);
                f(b);
            }
            Some(x) => {
                let f: extern "C" fn(*const c_void, usize) = core::mem::transmute(invoke);
                f(b, x);
            }
        }
    }
}

extern "C" fn d_note_present(_this: Id, _cmd: Sel, _center: Id, _note: Id, handler: *const c_void) {
    // banner | list | sound: shown although the app is in front
    // SAFETY: the centre's completion handler, called once.
    unsafe { call_block(handler, Some(1 << 4 | 1 << 3 | 1 << 1)) };
}

extern "C" fn d_note_response(_this: Id, _cmd: Sel, _center: Id, resp: Id, handler: *const c_void) {
    // SAFETY: the response's notification, its request's identifier, the action's identifier.
    let (ident, action) = unsafe {
        let n: Id = msg!(resp, c"notification"; Id);
        let r: Id = if n.is_null() { NIL } else { msg!(n, c"request"; Id) };
        let i = if r.is_null() { String::new() } else { string_of(msg!(r, c"identifier"; Id)) };
        (i, string_of(msg!(resp, c"actionIdentifier"; Id)))
    };
    tray::note_response(&ident, action.ends_with("DismissActionIdentifier"));
    // SAFETY: the centre's completion handler, called once.
    unsafe { call_block(handler, None) };
}

// ---------------------------------------------------------------- the Dock tile

pub(crate) mod dock {
    use super::*;
    use crate::taskbar::{JumpTask, State, TaskbarProgress};

    fn tile() -> Id {
        let Some(app) = super::super::app() else { return NIL };
        // SAFETY: the application's Dock tile.
        unsafe { msg!(app, c"dockTile"; Id) }
    }

    pub fn badge(s: &State) -> bool {
        if !super::super::init() {
            return false;
        }
        let t = tile();
        if t.is_null() {
            return false;
        }
        let label = if s.count == 0 { NIL } else { nsstring(&format!("{}", s.count)) };
        // SAFETY: the tile's badge (nil: none).
        unsafe { msg!(t, c"setBadgeLabel:", label => Id; ()) };
        release(label);
        true
    }

    pub fn progress(s: &State) -> bool {
        if !super::super::init() {
            return false;
        }
        let t = tile();
        if t.is_null() {
            return false;
        }
        st(|x| x.progress = s.progress);
        // SAFETY: the tile's content view: ours while progress shows, none otherwise; display redraws.
        unsafe {
            if matches!(s.progress, TaskbarProgress::None) {
                msg!(t, c"setContentView:", NIL => Id; ());
                let v = st(|x| core::mem::replace(&mut x.dock_view, NIL));
                release(v);
            } else if st(|x| x.dock_view.is_null()) {
                let (cls, _) = subclass(c"NSView", c"JFDockView", &[(c"drawRect:", v_draw_dock as *const c_void, c"v@:{CGRect={CGPoint=dd}{CGSize=dd}}")], &[]);
                let v: Id = msg!(cls, c"alloc"; Id);
                let v: Id = msg!(v, c"initWithFrame:", NSRect::new(0.0, 0.0, 128.0, 128.0) => NSRect; Id);
                msg!(t, c"setContentView:", v => Id; ());
                st(|x| x.dock_view = v);
            }
            msg!(t, c"display"; ());
        }
        true
    }

    /// the tile: the app's icon, below a bar of the progress
    extern "C" fn v_draw_dock(this: Id, _cmd: Sel, _dirty: NSRect) {
        let p = st(|x| x.progress);
        // SAFETY: drawing into our view's own context (AppKit calls drawRect: with it current).
        unsafe {
            let b: NSRect = msg!(this, c"bounds"; NSRect);
            if let Some(app) = super::super::app() {
                let img: Id = msg!(app, c"applicationIconImage"; Id);
                if !img.is_null() {
                    msg!(img, c"drawInRect:fromRect:operation:fraction:", b => NSRect, NSRect::default() => NSRect, 2usize => usize, 1.0f64 => f64; ());
                }
            }
            let (x, y, w, h) = (b.size.w * 0.1, b.size.h * 0.08, b.size.w * 0.8, b.size.h * 0.12);
            let color = |name: &CStr| -> Id { msg!(class(c"NSColor"), name; Id) };
            let back: Id = msg!(class(c"NSColor"), c"colorWithWhite:alpha:", 0.0f64 => f64, 0.55f64 => f64; Id);
            msg!(back, c"setFill"; ());
            let bp: Id = msg!(class(c"NSBezierPath"), c"bezierPathWithRoundedRect:xRadius:yRadius:", NSRect::new(x, y, w, h) => NSRect, h / 2.0 => f64,
                              h / 2.0 => f64; Id);
            msg!(bp, c"fill"; ());
            let (frac, fill) = match p {
                TaskbarProgress::Normal(f) => (f as f64, color(c"systemBlueColor")),
                TaskbarProgress::Paused(f) => (f as f64, color(c"systemYellowColor")),
                TaskbarProgress::Error(f) => (f as f64, color(c"systemRedColor")),
                TaskbarProgress::Indeterminate => (1.0, color(c"systemGrayColor")),
                TaskbarProgress::None => (0.0, NIL),
            };
            if !fill.is_null() && frac > 0.0 {
                msg!(fill, c"setFill"; ());
                let i = 2.0;
                let fw = ((w - 2.0 * i) * frac.clamp(0.0, 1.0)).max(h - 2.0 * i);
                let fp: Id = msg!(class(c"NSBezierPath"), c"bezierPathWithRoundedRect:xRadius:yRadius:", NSRect::new(x + i, y + i, fw, h - 2.0 * i) => NSRect,
                                  (h - 2.0 * i) / 2.0 => f64, (h - 2.0 * i) / 2.0 => f64; Id);
                msg!(fp, c"fill"; ());
            }
        }
    }

    pub fn tasks(t: &[JumpTask]) -> bool {
        st(|s| s.tasks = t.to_vec());
        super::super::init()
    }

    pub fn supported() -> bool {
        true
    }

    pub fn set_app_id(_id: &str) -> bool {
        false
    }

    pub fn app_id() -> Option<String> {
        None
    }

    /// (NSApp's delegate: applicationDockMenu:) the tasks as the Dock menu (autoreleased), nil without
    pub(crate) fn dock_menu() -> Id {
        let tasks = st(|s| s.tasks.clone());
        if tasks.is_empty() {
            return NIL;
        }
        let tgt = target();
        // SAFETY: a menu of ours handed to AppKit autoreleased.
        unsafe {
            let m: Id = msg!(class(c"NSMenu"), c"new"; Id);
            for (k, t) in tasks.iter().enumerate() {
                let (title, key) = (nsstring(&t.title), nsstring(""));
                let mi: Id = msg!(class(c"NSMenuItem"), c"alloc"; Id);
                let mi: Id = msg!(mi, c"initWithTitle:action:keyEquivalent:", title => Id, sel(c"jfDockTask:") => Sel, key => Id; Id);
                release(title);
                release(key);
                msg!(mi, c"setTarget:", tgt => Id; ());
                msg!(mi, c"setTag:", k as isize => isize; ());
                if !t.desc.is_empty() {
                    let d = nsstring(&t.desc);
                    msg!(mi, c"setToolTip:", d => Id; ());
                    release(d);
                }
                msg!(m, c"addItem:", mi => Id; ());
                release(mi);
            }
            msg!(m, c"autorelease"; Id)
        }
    }
}

extern "C" fn d_dock_menu(_this: Id, _cmd: Sel, _app: Id) -> Id {
    dock::dock_menu()
}

/// application:openURLs: -- documents opened with the app, dropped on its Dock icon, URLs of its schemes
extern "C" fn d_open_urls(_this: Id, _cmd: Sel, _app: Id, urls: Id) {
    let mut args = Vec::new();
    // SAFETY: an array of NSURLs.
    unsafe {
        let n: usize = if urls.is_null() { 0 } else { msg!(urls, c"count"; usize) };
        for k in 0..n.min(4096) {
            let u: Id = msg!(urls, c"objectAtIndex:", k => usize; Id);
            let s = if msg!(u, c"isFileURL"; bool) { string_of(msg!(u, c"path"; Id)) } else { string_of(msg!(u, c"absoluteString"; Id)) };
            if !s.is_empty() {
                args.push(s);
            }
        }
    }
    if !args.is_empty() {
        crate::instance::queue_push(crate::instance::Request { cwd: crate::cwd(), args });
    }
}

/// the observer's (NSApp's and the notification centre's delegate's) methods for the services
pub(super) fn delegate_methods() -> [(&'static CStr, *const c_void, &'static CStr); 4] {
    [
        (c"applicationDockMenu:", d_dock_menu as *const c_void, c"@@:@"),
        (c"application:openURLs:", d_open_urls as *const c_void, c"v@:@@"),
        (c"userNotificationCenter:willPresentNotification:withCompletionHandler:", d_note_present as *const c_void, c"v@:@@@?"),
        (c"userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:", d_note_response as *const c_void, c"v@:@@@?"),
    ]
}

// ---------------------------------------------------------------- global shortcuts

pub(crate) mod hotkey {
    use super::*;
    use crate::hotkey::{Chord, HotkeyError};
    use crate::{MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_SUPER};

    #[repr(C)]
    struct EventTypeSpec {
        class: u32,
        kind: u32,
    }
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct EventHotKeyId {
        signature: u32,
        id: u32,
    }
    type Handler = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32;
    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn GetApplicationEventTarget() -> *mut c_void;
        fn InstallEventHandler(target: *mut c_void, handler: Handler, n: usize, list: *const EventTypeSpec, user: *mut c_void, out: *mut *mut c_void) -> i32;
        fn RegisterEventHotKey(code: u32, mods: u32, id: EventHotKeyId, target: *mut c_void, options: u32, out: *mut *mut c_void) -> i32;
        fn UnregisterEventHotKey(r: *mut c_void) -> i32;
        fn GetEventParameter(ev: *mut c_void, name: u32, ty: u32, actual: *mut u32, size: usize, actual_size: *mut usize, data: *mut c_void) -> i32;
        fn CreateEvent(alloc: *const c_void, class: u32, kind: u32, when: f64, flags: u32, out: *mut *mut c_void) -> i32;
        fn SetEventParameter(ev: *mut c_void, name: u32, ty: u32, size: usize, data: *const c_void) -> i32;
        fn SendEventToEventTarget(ev: *mut c_void, target: *mut c_void) -> i32;
        fn ReleaseEvent(ev: *mut c_void);
    }
    const CLASS_KEYBOARD: u32 = 0x6B65_7962; // 'keyb'
    const HOTKEY_PRESSED: u32 = 5; // kEventHotKeyPressed
    const PARAM_DIRECT: u32 = 0x2D2D_2D2D; // '----'
    const TYPE_HOTKEY_ID: u32 = 0x686B_6964; // 'hkid'
    const SIGNATURE: u32 = 0x4A46_4C42; // 'JFLB'
    const EXISTS: i32 = -9878; // eventHotKeyExistsErr

    /// a VK code -> macOS's key code (ANSI positions; the inverse of apple_mac.rs vk_of)
    fn key_code(vk: u32) -> Option<u32> {
        const LETTERS: [u32; 26] = [0x00, 0x0B, 0x08, 0x02, 0x0E, 0x03, 0x05, 0x04, 0x22, 0x26, 0x28, 0x25, 0x2E, 0x2D, 0x1F, 0x23, 0x0C, 0x0F,
                                    0x01, 0x11, 0x20, 0x09, 0x0D, 0x07, 0x10, 0x06];
        const DIGITS: [u32; 10] = [0x1D, 0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1A, 0x1C, 0x19];
        const FKEYS: [u32; 20] = [0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F, 0x69, 0x6B, 0x71, 0x6A, 0x40, 0x4F, 0x50, 0x5A];
        const KEYPAD: [u32; 10] = [0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5B, 0x5C];
        Some(match vk {
            0x41..=0x5A => LETTERS[(vk - 0x41) as usize],
            0x30..=0x39 => DIGITS[(vk - 0x30) as usize],
            0x70..=0x83 => FKEYS[(vk - 0x70) as usize],
            0x60..=0x69 => KEYPAD[(vk - 0x60) as usize],
            0x20 => 0x31,
            0x0D => 0x24,
            0x09 => 0x30,
            0x1B => 0x35,
            0x08 => 0x33,
            0x2E => 0x75,
            0x2D => 0x72,
            0x24 => 0x73,
            0x23 => 0x77,
            0x21 => 0x74,
            0x22 => 0x79,
            0x25 => 0x7B,
            0x26 => 0x7E,
            0x27 => 0x7C,
            0x28 => 0x7D,
            0x6B => 0x45,
            0x6D => 0x4E,
            0x6A => 0x43,
            0x6F => 0x4B,
            0x6E => 0x41,
            0xBD => 0x1B,
            0xBB => 0x18,
            0xBC => 0x2B,
            0xBE => 0x2F,
            _ => return None,
        })
    }

    fn carbon_mods(m: u8) -> u32 {
        let mut c = 0;
        if m & MOD_CTRL != 0 {
            c |= 1 << 8; // cmdKey: Command acts as Ctrl
        }
        if m & MOD_SHIFT != 0 {
            c |= 1 << 9;
        }
        if m & MOD_ALT != 0 {
            c |= 1 << 11;
        }
        if m & MOD_SUPER != 0 {
            c |= 1 << 12; // controlKey
        }
        c
    }

    extern "C" fn on_hotkey(_call: *mut c_void, ev: *mut c_void, _user: *mut c_void) -> i32 {
        let mut hk = EventHotKeyId::default();
        // SAFETY: the event's hot key id parameter into a struct of its size.
        let r = unsafe {
            GetEventParameter(ev, PARAM_DIRECT, TYPE_HOTKEY_ID, core::ptr::null_mut(), core::mem::size_of::<EventHotKeyId>(), core::ptr::null_mut(),
                              (&mut hk as *mut EventHotKeyId).cast())
        };
        if r == 0 && hk.signature == SIGNATURE && crate::hotkey::registered(hk.id) {
            crate::hotkey::push(crate::hotkey::HotkeyEvent::Pressed(hk.id));
            super::super::wake_app();
        }
        0
    }

    fn handler() -> bool {
        if st(|s| s.handler) {
            return true;
        }
        let spec = EventTypeSpec { class: CLASS_KEYBOARD, kind: HOTKEY_PRESSED };
        // SAFETY: a handler of ours on the application's target, kept for the process.
        let ok = unsafe { InstallEventHandler(GetApplicationEventTarget(), on_hotkey, 1, &spec, core::ptr::null_mut(), core::ptr::null_mut()) } == 0;
        st(|s| s.handler = ok);
        ok
    }

    pub fn available() -> bool {
        true
    }

    pub fn register(id: u32, c: Chord, _d: &str) -> Result<(), HotkeyError> {
        let code = key_code(c.vk).ok_or(HotkeyError::Invalid)?;
        if !super::super::init() || !handler() {
            return Err(HotkeyError::Unavailable);
        }
        unregister(id);
        let mut r: *mut c_void = core::ptr::null_mut();
        // SAFETY: a hot key of ours on the application's target.
        let e = unsafe { RegisterEventHotKey(code, carbon_mods(c.mods), EventHotKeyId { signature: SIGNATURE, id }, GetApplicationEventTarget(), 0, &mut r) };
        match e {
            0 if !r.is_null() => {
                st(|s| s.hotkeys.push((id, r)));
                Ok(())
            }
            EXISTS => Err(HotkeyError::Conflict),
            e => Err(HotkeyError::Failed(format!("RegisterEventHotKey: {e}"))),
        }
    }

    pub fn unregister(id: u32) {
        let refs: Vec<*mut c_void> = st(|s| {
            let r = s.hotkeys.iter().filter(|h| h.0 == id).map(|h| h.1).collect();
            s.hotkeys.retain(|h| h.0 != id);
            r
        });
        for r in refs {
            // SAFETY: a hot key this module registered.
            unsafe { UnregisterEventHotKey(r) };
        }
    }

    /// (test aid) a hot key event of id sent to the application's target, as the system sends one
    pub fn hotkey_test_post(id: u32) -> bool {
        if !handler() {
            return false;
        }
        let hk = EventHotKeyId { signature: SIGNATURE, id };
        let mut ev: *mut c_void = core::ptr::null_mut();
        // SAFETY: a Carbon event of ours with the hot key id, sent and released.
        unsafe {
            if CreateEvent(core::ptr::null(), CLASS_KEYBOARD, HOTKEY_PRESSED, 0.0, 0, &mut ev) != 0 || ev.is_null() {
                return false;
            }
            SetEventParameter(ev, PARAM_DIRECT, TYPE_HOTKEY_ID, core::mem::size_of::<EventHotKeyId>(), (&hk as *const EventHotKeyId).cast());
            let r = SendEventToEventTarget(ev, GetApplicationEventTarget());
            ReleaseEvent(ev);
            r == 0
        }
    }
}

// ---------------------------------------------------------------- test aids (apple_mac_test.rs)

/// WT_TRAY: k 0 the status item's menu rows (-1 no item, 0 no menu), 1 choose row a, 2 click the button
pub(super) fn test_tray(k: i32, a: i32) -> i64 {
    let item = st(|s| s.item);
    if item.is_null() {
        return -1;
    }
    // SAFETY: our status item, its menu and button.
    unsafe {
        let m: Id = msg!(item, c"menu"; Id);
        match k {
            0 => if m.is_null() { 0 } else { msg!(m, c"numberOfItems"; isize) as i64 },
            1 if !m.is_null() => {
                msg!(m, c"performActionForItemAtIndex:", a as isize => isize; ());
                0
            }
            2 => {
                let b: Id = msg!(item, c"button"; Id);
                if b.is_null() {
                    return -1;
                }
                msg!(b, c"performClick:", NIL => Id; ());
                0
            }
            _ => -1,
        }
    }
}

/// WT_DOCK: k 0 the badge's number (-1 none), 1 our progress view shown (1 / 0), 2 the Dock menu's rows,
/// 3 choose its row a
pub(super) fn test_dock(k: i32, a: i32) -> i64 {
    let Some(app) = super::app() else { return -1 };
    // SAFETY: the Dock tile and the Dock menu our delegate method makes.
    unsafe {
        let t: Id = msg!(app, c"dockTile"; Id);
        match k {
            0 => {
                let l: Id = if t.is_null() { NIL } else { msg!(t, c"badgeLabel"; Id) };
                if l.is_null() { -1 } else { string_of(l).parse::<i64>().unwrap_or(-2) }
            }
            1 => (!t.is_null() && !msg!(t, c"contentView"; Id).is_null()) as i64,
            2 | 3 => {
                let m = dock::dock_menu();
                if m.is_null() {
                    return 0;
                }
                if k == 2 {
                    msg!(m, c"numberOfItems"; isize) as i64
                } else {
                    msg!(m, c"performActionForItemAtIndex:", a as isize => isize; ());
                    0
                }
            }
            _ => -1,
        }
    }
}

/// WT_OPEN_URLS: the delegate's application:openURLs: with a file URL of path and an https URL
pub(super) fn test_open(path: &str) -> i64 {
    // SAFETY: URLs of ours in an array handed to our delegate method.
    unsafe {
        let pool = crate::apple::objc::objc_autoreleasePoolPush();
        let (p, w) = (nsstring(path), nsstring("https://example.org/jflib"));
        let a: Id = msg!(class(c"NSURL"), c"fileURLWithPath:", p => Id; Id);
        let b: Id = msg!(class(c"NSURL"), c"URLWithString:", w => Id; Id);
        release(p);
        release(w);
        let arr: Id = msg!(class(c"NSMutableArray"), c"array"; Id);
        msg!(arr, c"addObject:", a => Id; ());
        msg!(arr, c"addObject:", b => Id; ());
        d_open_urls(NIL, core::ptr::null(), NIL, arr);
        crate::apple::objc::objc_autoreleasePoolPop(pool);
    }
    0
}
