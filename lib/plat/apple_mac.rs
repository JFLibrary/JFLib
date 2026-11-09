// apple_mac -- windows on macOS: AppKit through the Objective-C runtime (docs/APPLE.md AP4)
//
// Each window is an NSWindow with a view of our class JFView (flipped: y grows downwards) and a
// delegate of our class JFWindowDelegate; both carry the window's index in their instance variable.
// The view and the delegate never act: they push events into the ring (evq.rs), as win_api.rs does.
//
//   present   the frame (rows of B, G, R, x) becomes a CGImage, the contents of the view's layer
//             (copied: the layer keeps the last picture for the window server)
//   wait      the own event loop: nextEventMatchingMask until the deadline, sendEvent; wake() from any
//             thread signals a run loop source whose callback posts an application-defined event
//   sizes     points of AppKit times the window's backingScaleFactor = pixels; the dpi 96 * scale
//   keys      NSEvent keyCode -> Windows VK (letters and digits as the layout names them); text
//             through NSTextInputClient (dead keys, input methods: E_PREEDIT, E_IME_COMMIT)
//   modifiers Command is MOD_CTRL (the library's shortcuts are Ctrl+..), Control MOD_SUPER, Option
//             MOD_ALT
//   gestures  the trackpad's pinch becomes Ctrl + wheel (120 per 0.2 of magnification), as Windows
//             makes it of a touchpad's pinch; two-finger scrolling is the wheel with fine steps
//   settings  a change of the appearance or the accent colour (distributed notifications) and of the
//             screens push E_SETTINGS (SET_THEME, SET_MONITORS) for window 0
//   frame     PWF_SYSFRAME: the system's title bar; otherwise the content reaches under a transparent
//             title bar without its buttons (the app draws its own), HB_DRAG drags the window
//   popups    (menus, lists, tooltips that leave the window) borderless non-activating panels, child
//             windows of their owner; they never take the keyboard, their pointer input is pushed as
//             events of the owner in owner client coordinates (as win_popup.rs does)
//   owned     windows with an owner hang on it as child windows while shown: above it, moved with it
//   dialogs   file_dialog: NSOpenPanel / NSSavePanel, modal (FD_FOLDER chooses a folder)
//   system    monitors (NSScreen), the appearance, the accent colour and the accessibility settings
//             (sys_settings_os), custom cursors (NSCursor of an NSBitmapImageRep)
//   dnd       apple_mac_dnd.rs: every view a dragging destination, drag_start a dragging session
//   the app   apple_mac_app.rs: the main menu (Cmd+Q asks the main window), NSApp's delegate,
//             E_POWER / E_SESSION_END (NSWorkspace, SIGTERM), tool windows as panels, printing
//   tests     apple_mac_test.rs: win_test, file_dialog_autoclose (events to our own windows only)
//
// Everything runs on the main thread (AppKit's rule); the state is reached through short borrows
// that never span a call into AppKit (AppKit calls back into the view and the delegate).

use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ffi::{CStr, c_void};
use core::sync::atomic::{AtomicIsize, AtomicPtr, Ordering};

use super::objc::{Id, NIL, NSPoint, NSRange, NSRect, NSSize, NOT_FOUND, Sel, class, msg, nsstring, release, string_of, subclass};
use crate::{
    BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, BTN_X1, BTN_X2, CLIP_DOCX, CLIP_HTML, CLIP_IMAGE, CLIP_PNG, CLIP_RTF, CLIP_TEXT, CUR_ARROW,
    CUR_CROSS, CUR_HAND, CUR_IBEAM, CUR_MAX_PX, CUR_MOVE, CUR_NONE, CUR_SIZE_NESW, CUR_SIZE_NS, CUR_SIZE_NWSE, CUR_SIZE_WE,
    CUR_USER0, CUR_USER_MAX, CUR_WAIT, E_CLOSE, E_DOWN, FD_EXT_SHIFT, FD_FOLDER, FD_SAVE,
    E_DPI, E_FOCUS_IN, E_FOCUS_OUT, E_KEY_DOWN, E_KEY_UP, E_LEAVE, E_MOVE, E_PAINT, E_RESIZE, E_TEXT, E_UP, E_WHEEL, E_WINPOS,
    Event, Frame, HB_CLOSE, HB_DRAG, HB_MAX, HB_MIN, MOD_ALT, MOD_CAPS, MOD_CTRL, MOD_SHIFT, MOD_SUPER, PW_DEFAULT, PW_KEEP,
    PPW_MAX, PW_MAX, PW_NOOWNER, PWF_DIALOG, PWF_NOCLOSE, PWF_RESIZABLE, PWF_SYSFRAME, PWF_TOOL, PWS_HIDE, PWS_SHOWNA, WCMD_CLOSE,
    WCMD_FULLSCREEN, WCMD_LOWER, WCMD_MAXIMIZE, WCMD_MINIMIZE, WCMD_RESTORE, WCMD_WINDOWED, WST_ACTIVE, WST_FULLSCREEN,
    WST_MAXIMIZED, WST_MINIMIZED, evq_count, evq_push, E_SETTINGS, SET_MONITORS, SET_THEME,
};

type CfRef = *const c_void;

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    static NSApplicationDidChangeScreenParametersNotification: Id;
}
#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {
    static NSDefaultRunLoopMode: Id;
    static NSRunLoopCommonModes: Id;
}

#[path = "apple_mac_test.rs"]
mod test;
pub use test::*;
#[path = "apple_mac_dnd.rs"]
mod dnd;
#[path = "apple_mac_app.rs"]
mod sysapp;
pub(crate) use sysapp::print_dialog;
pub use sysapp::print_dialog_autoclose;
pub(crate) use dnd::os_drag_start;
pub use dnd::{dnd_test_drop, dnd_test_formats};
#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {
    static kCAGravityTopLeft: Id;
}
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static kCGColorSpaceSRGB: CfRef;
    fn CGColorSpaceCreateWithName(name: CfRef) -> CfRef;
    fn CGDataProviderCreateWithCFData(data: CfRef) -> CfRef;
    fn CGDataProviderRelease(p: CfRef);
    fn CGImageCreate(w: usize, h: usize, bpc: usize, bpp: usize, stride: usize, cs: CfRef, info: u32, provider: CfRef,
                     decode: *const f64, interpolate: bool, intent: i32) -> CfRef;
    fn CGImageRelease(i: CfRef);
}
#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageSourceCreateWithData(data: CfRef, options: CfRef) -> CfRef;
    fn CGImageSourceCreateImageAtIndex(src: CfRef, index: usize, options: CfRef) -> CfRef;
    fn CGImageDestinationCreateWithData(data: CfRef, ty: CfRef, count: usize, options: CfRef) -> CfRef;
    fn CGImageDestinationAddImage(dst: CfRef, img: CfRef, props: CfRef);
    fn CGImageDestinationFinalize(dst: CfRef) -> bool;
}
#[repr(C)]
struct SourceContext {
    version: isize,
    info: *mut c_void,
    retain: usize,
    release: usize,
    copy_description: usize,
    equal: usize,
    hash: usize,
    schedule: usize,
    cancel: usize,
    perform: extern "C" fn(*mut c_void),
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopCommonModes: CfRef;
    fn CFDataCreate(alloc: CfRef, bytes: *const u8, n: isize) -> CfRef;
    fn CFDataCreateMutable(alloc: CfRef, capacity: isize) -> CfRef;
    fn CFDataGetBytePtr(d: CfRef) -> *const u8;
    fn CFDataGetLength(d: CfRef) -> isize;
    fn CFRelease(r: CfRef);
    fn CFRunLoopGetMain() -> CfRef;
    fn CFRunLoopSourceCreate(alloc: CfRef, order: isize, ctx: *const SourceContext) -> CfRef;
    fn CFRunLoopAddSource(rl: CfRef, src: CfRef, mode: CfRef);
    fn CFRunLoopSourceSignal(src: CfRef);
    fn CFRunLoopWakeUp(rl: CfRef);
}

// NSWindowStyleMask, NSEvent types and modifier flags, tracking area options
const STYLE_TITLED: u64 = 1;
const STYLE_CLOSABLE: u64 = 2;
const STYLE_MINIATURIZABLE: u64 = 4;
const STYLE_RESIZABLE: u64 = 8;
const STYLE_NONACTIVATING: u64 = 1 << 7; // NSWindowStyleMaskNonactivatingPanel
const STYLE_FULLSCREEN: u64 = 1 << 14;
const STYLE_FULL_SIZE_CONTENT: u64 = 1 << 15;
const EV_APP_DEFINED: u64 = 15;
const EV_KEY_UP: u64 = 11;
const EV_LEFT_DRAGGED: u64 = 6;
const WAKE_SUBTYPE: i16 = 0x4a46; // "JF"
const FLAG_CAPS: u64 = 1 << 16;
const FLAG_SHIFT: u64 = 1 << 17;
const FLAG_CONTROL: u64 = 1 << 18;
const FLAG_OPTION: u64 = 1 << 19;
const FLAG_COMMAND: u64 = 1 << 20;
const TRACK_OPTIONS: u64 = 0x01 | 0x02 | 0x80 | 0x200; // entered/exited, moved, always, in the visible rect
const BITMAP_BGRX: u32 = (2 << 12) | 6; // kCGBitmapByteOrder32Little | kCGImageAlphaNoneSkipFirst
const POP_BASE: isize = 64; // a popup view's index: POP_BASE + slot
const POP_RADIUS: f64 = 8.0; // rounded corners of a popup (points = DIP), like the flyout surface

// rounding without std (core has no f64::round / floor); |x| stays far below 2^63 here
fn floor(x: f64) -> f64 {
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}
fn round(x: f64) -> i32 {
    floor(x + 0.5) as i32
}
fn trunc(x: f64) -> f64 {
    x as i64 as f64
}

// ---------------------------------------------------------------- state

struct Win {
    win: Id,
    view: Id,
    del: Id,
    scale: f64,
    w: i32,
    h: i32,
    cursor: u32,
    ime: [i32; 3],
    owner: i32,    // the owner window (-1: none): a child window of it while shown
    enabled: bool, // win_enable: a disabled window takes no input
    pos: [i32; 2], // the client origin last reported (E_WINPOS) or set
}

/// one popup slot: a panel with a view of ours (index POP_BASE + slot), a child window of its
/// owner while shown
#[derive(Clone, Copy)]
struct Pop {
    win: Id,
    view: Id,
    owner: i32, // -1: hidden
    parent: Id, // the owner's NSWindow it hangs on (nil: none)
    dx: i32,    // popup pixel 0, 0 = owner client pixel dx, dy
    dy: i32,
    w: i32,
    h: i32,
}

impl Pop {
    const NONE: Pop = Pop { win: NIL, view: NIL, owner: -1, parent: NIL, dx: 0, dy: 0, w: 0, h: 0 };
}

struct St {
    app: Id,
    view_cls: Id,
    del_cls: Id,
    wins: [Option<Win>; PW_MAX],
    pops: [Pop; PPW_MAX as usize],
    hit: Option<fn(i32, i32, i32) -> u32>,
    mv: Option<fn(i32, &mut [i32; 4])>,
    refresh: Option<fn()>,
    in_refresh: bool,
    min0: (i32, i32),
    flags: u64,
    marked: String,
    cs: CfRef,
    wheel: [f64; 2],
    zoom: f64,                            // the pinch's rest below one wheel step
    own: bool,                            // win_set_rect is moving / sizing: not reported back
    cursors: [Id; CUR_USER_MAX as usize], // cursor_create's NSCursors (CUR_USER0 + k)
    blank: Id,                            // CUR_NONE: an empty cursor
    autoclose: Option<(u32, Option<String>)>, // (test aid) the next file dialog: Cancel / OK after ms
}

struct Main(UnsafeCell<Option<St>>);
// SAFETY: only the main thread touches the state (AppKit's thread); other threads only signal the
// run loop source (SOURCE, MAIN_RL).
unsafe impl Sync for Main {}
static ST: Main = Main(UnsafeCell::new(None));
static VIEW_OFF: AtomicIsize = AtomicIsize::new(0);
static DEL_OFF: AtomicIsize = AtomicIsize::new(0);
static SOURCE: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static MAIN_RL: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

/// a short borrow of the state (never across a call into AppKit)
fn with<R>(f: impl FnOnce(&mut St) -> R) -> Option<R> {
    // SAFETY: main thread only; every borrow ends before AppKit can call back (see the top).
    unsafe { (*ST.0.get()).as_mut().map(f) }
}

fn rec<R>(w: i32, f: impl FnOnce(&mut Win) -> R) -> Option<R> {
    with(|s| s.wins.get_mut(w as usize).and_then(|r| r.as_mut()).map(f)).flatten()
}

/// (window, view, scale) of window w
fn ids(w: i32) -> Option<(Id, Id, f64)> {
    rec(w, |r| (r.win, r.view, r.scale))
}

fn app() -> Option<Id> {
    with(|s| s.app)
}

fn push(w: i32, typ: u8, btn: u8, x: i32, y: i32, data: u32) {
    let mods = with(|s| mods_of(s.flags)).unwrap_or(0);
    evq_push(Event { typ, btn, mods, win: w as u8, x, y, data });
}

fn mods_of(f: u64) -> u8 {
    let mut m = 0;
    if f & FLAG_SHIFT != 0 {
        m |= MOD_SHIFT;
    }
    if f & FLAG_COMMAND != 0 {
        m |= MOD_CTRL;
    }
    if f & FLAG_CONTROL != 0 {
        m |= MOD_SUPER;
    }
    if f & FLAG_OPTION != 0 {
        m |= MOD_ALT;
    }
    if f & FLAG_CAPS != 0 {
        m |= MOD_CAPS;
    }
    m
}

// ---------------------------------------------------------------- the application

/// NSApplication, our two classes, the sRGB colour space, the wake-up source (once)
fn init() -> bool {
    if app().is_some() {
        return true;
    }
    let (view_cls, voff) = subclass(c"NSView", c"JFView", &view_methods(), &[c"NSTextInputClient", c"NSDraggingDestination", c"NSDraggingSource"]);
    let (del_cls, doff) = subclass(c"NSObject", c"JFWindowDelegate", &delegate_methods(), &[c"NSWindowDelegate"]);
    VIEW_OFF.store(voff, Ordering::SeqCst);
    DEL_OFF.store(doff, Ordering::SeqCst);
    // SAFETY: AppKit's start on the main thread; the run loop source lives for the process.
    unsafe {
        let app: Id = msg!(class(c"NSApplication"), c"sharedApplication"; Id);
        if app.is_null() {
            return false;
        }
        msg!(app, c"setActivationPolicy:", 0isize => isize; bool); // regular: a Dock icon, a menu bar
        msg!(app, c"finishLaunching"; ());
        let cs = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        let ctx = SourceContext { version: 0, info: core::ptr::null_mut(), retain: 0, release: 0, copy_description: 0, equal: 0,
                                  hash: 0, schedule: 0, cancel: 0, perform: on_wake_source };
        let rl = CFRunLoopGetMain();
        let src = CFRunLoopSourceCreate(core::ptr::null(), 0, &ctx);
        CFRunLoopAddSource(rl, src, kCFRunLoopCommonModes);
        MAIN_RL.store(rl as *mut c_void, Ordering::SeqCst);
        SOURCE.store(src as *mut c_void, Ordering::SeqCst);
        let obs = observe(del_cls, doff);
        sysapp::observe(app, obs);
        *ST.0.get() = Some(St { app, view_cls, del_cls, wins: Default::default(), pops: [Pop::NONE; PPW_MAX as usize], hit: None, mv: None, refresh: None, in_refresh: false,
                                min0: (0, 0), flags: 0, marked: String::new(), cs, wheel: [0.0; 2], zoom: 0.0, own: false,
                                cursors: [NIL; CUR_USER_MAX as usize], blank: NIL, autoclose: None });
    }
    true
}

/// (apple_recent.rs and others) the application object exists (made here once)
pub(crate) fn app_init() -> bool {
    init()
}

/// the settings' notifications go to a delegate object of ours (index -1): E_SETTINGS -> the object
fn observe(del_cls: Id, doff: isize) -> Id {
    // SAFETY: an observer object kept for the process; the centres retain nothing of it.
    unsafe {
        let obs: Id = msg!(del_cls, c"new"; Id);
        super::objc::ivar_set(obs, doff, -1);
        let dnc: Id = msg!(class(c"NSDistributedNotificationCenter"), c"defaultCenter"; Id);
        for name in ["AppleInterfaceThemeChangedNotification", "AppleColorPreferencesChangedNotification"] {
            let n = nsstring(name);
            msg!(dnc, c"addObserver:selector:name:object:", obs => Id, super::objc::sel(c"jfThemeChanged:") => Sel, n => Id, NIL => Id; ());
            release(n);
        }
        let nc: Id = msg!(class(c"NSNotificationCenter"), c"defaultCenter"; Id);
        msg!(nc, c"addObserver:selector:name:object:", obs => Id, super::objc::sel(c"jfScreensChanged:") => Sel,
             NSApplicationDidChangeScreenParametersNotification => Id, NIL => Id; ());
        obs
    }
}

extern "C" fn d_theme_changed(_this: Id, _cmd: Sel, _n: Id) {
    settings_changed(SET_THEME);
}
extern "C" fn d_screens_changed(_this: Id, _cmd: Sel, _n: Id) {
    settings_changed(SET_MONITORS);
}

/// E_SETTINGS for window 0; the loop may wait for an event: the wake-up lets it return
fn settings_changed(what: u32) {
    push(0, E_SETTINGS, 0, 0, 0, what);
    wake_app();
}

/// (apple_thread.rs wake) any thread: make wait() return: the source's callback runs on the main thread
pub(crate) fn wake_app() {
    let (src, rl) = (SOURCE.load(Ordering::SeqCst), MAIN_RL.load(Ordering::SeqCst));
    if !src.is_null() && !rl.is_null() {
        // SAFETY: both are thread-safe CoreFoundation calls on objects that live for the process.
        unsafe {
            CFRunLoopSourceSignal(src);
            CFRunLoopWakeUp(rl);
        }
    }
}

extern "C" fn on_wake_source(_: *mut c_void) {
    let Some(app) = app() else { return };
    // SAFETY: an application-defined event made and posted on the main thread.
    unsafe {
        let ev: Id = msg!(class(c"NSEvent"), c"otherEventWithType:location:modifierFlags:timestamp:windowNumber:context:subtype:data1:data2:",
                          EV_APP_DEFINED => u64, NSPoint::default() => NSPoint, 0u64 => u64, 0.0f64 => f64, 0isize => isize, NIL => Id,
                          WAKE_SUBTYPE => i16, 0isize => isize, 0isize => isize; Id);
        if !ev.is_null() {
            msg!(app, c"postEvent:atStart:", ev => Id, false => bool; ());
        }
    }
}

/// 0 light, 1 dark: jfgui_THEME=light|dark, else the application's appearance
pub fn theme() -> i32 {
    match crate::env("jfgui_THEME").as_deref() {
        Some("dark") => return 1,
        Some("light") => return 0,
        _ => {}
    }
    if !init() {
        return -1;
    }
    let Some(app) = app() else { return -1 };
    // SAFETY: effectiveAppearance / name of the application object.
    let name = unsafe {
        let ap: Id = msg!(app, c"effectiveAppearance"; Id);
        if ap.is_null() { String::new() } else { string_of(msg!(ap, c"name"; Id)) }
    };
    if name.contains("Dark") { 1 } else { 0 }
}

pub fn pen_devices() -> i32 {
    -1
}

// ---------------------------------------------------------------- windows

/// see win_api.rs (Windows) for the semantics of every function
pub fn win_create(title: &str, w_dip: i32, h_dip: i32, flags: u32, owner: i32, x: i32, y: i32) -> i32 {
    if !init() {
        crate::write_stderr(b"error: no window server (log in on the Mac first)\n");
        return -1;
    }
    sysapp::gui_once(); // the main menu, SIGTERM
    let Some(slot) = with(|s| {
        if owner == PW_NOOWNER && s.wins[0].is_none() {
            Some(0)
        } else {
            (1..PW_MAX).find(|&k| s.wins[k].is_none())
        }
    })
    .flatten() else {
        return -1;
    };
    let (view_cls, del_cls, min0) = with(|s| (s.view_cls, s.del_cls, s.min0)).unwrap_or((NIL, NIL, (0, 0)));
    let mut style = STYLE_TITLED;
    if flags & PWF_NOCLOSE == 0 {
        style |= STYLE_CLOSABLE;
    }
    if flags & (PWF_TOOL | PWF_DIALOG) == 0 {
        style |= STYLE_MINIATURIZABLE;
    }
    if flags & PWF_RESIZABLE != 0 {
        style |= STYLE_RESIZABLE;
    }
    if flags & PWF_SYSFRAME == 0 {
        style |= STYLE_FULL_SIZE_CONTENT;
    }
    let rect = NSRect::new(0.0, 0.0, w_dip.max(1) as f64, h_dip.max(1) as f64);
    // SAFETY: AppKit objects made on the main thread; every message has its exact signature.
    let (win, view, del, scale) = unsafe {
        // a tool window is a panel: the key window only for a click into a text field (sysapp)
        let cls = if flags & PWF_TOOL != 0 { sysapp::panel_class() } else { class(c"NSWindow") };
        let win: Id = msg!(cls, c"alloc"; Id);
        let win: Id = msg!(win, c"initWithContentRect:styleMask:backing:defer:", rect => NSRect, style => u64, 2u64 => u64, false => bool; Id);
        if win.is_null() {
            return -1;
        }
        if flags & PWF_TOOL != 0 {
            msg!(win, c"setBecomesKeyOnlyIfNeeded:", true => bool; ());
            msg!(win, c"setFloatingPanel:", false => bool; ());
            msg!(win, c"setHidesOnDeactivate:", false => bool; ());
        }
        msg!(win, c"setReleasedWhenClosed:", false => bool; ());
        let t = nsstring(title);
        msg!(win, c"setTitle:", t => Id; ());
        release(t);
        if flags & PWF_SYSFRAME == 0 {
            msg!(win, c"setTitlebarAppearsTransparent:", true => bool; ());
            msg!(win, c"setTitleVisibility:", 1isize => isize; ());
            for b in 0..3u64 {
                let button: Id = msg!(win, c"standardWindowButton:", b => u64; Id);
                if !button.is_null() {
                    msg!(button, c"setHidden:", true => bool; ());
                }
            }
        }
        let view: Id = msg!(view_cls, c"alloc"; Id);
        let view: Id = msg!(view, c"initWithFrame:", rect => NSRect; Id);
        super::objc::ivar_set(view, VIEW_OFF.load(Ordering::SeqCst), slot as isize);
        msg!(view, c"setWantsLayer:", true => bool; ());
        msg!(view, c"setLayerContentsRedrawPolicy:", 0isize => isize; ()); // never: our contents stay
        let layer: Id = msg!(view, c"layer"; Id);
        if !layer.is_null() {
            msg!(layer, c"setContentsGravity:", kCAGravityTopLeft => Id; ());
        }
        let ta: Id = msg!(class(c"NSTrackingArea"), c"alloc"; Id);
        let ta: Id = msg!(ta, c"initWithRect:options:owner:userInfo:", NSRect::default() => NSRect, TRACK_OPTIONS => u64, view => Id, NIL => Id; Id);
        msg!(view, c"addTrackingArea:", ta => Id; ());
        release(ta);
        dnd::register(view);
        msg!(win, c"setContentView:", view => Id; ());
        release(view); // the window holds it
        msg!(win, c"makeFirstResponder:", view => Id; bool);
        msg!(win, c"setAcceptsMouseMovedEvents:", true => bool; ());
        let del: Id = msg!(del_cls, c"alloc"; Id);
        let del: Id = msg!(del, c"init"; Id);
        super::objc::ivar_set(del, DEL_OFF.load(Ordering::SeqCst), slot as isize);
        msg!(win, c"setDelegate:", del => Id; ());
        if slot == 0 && min0 != (0, 0) {
            msg!(win, c"setContentMinSize:", NSSize { w: min0.0 as f64, h: min0.1 as f64 } => NSSize; ());
        }
        let scale: f64 = msg!(win, c"backingScaleFactor"; f64);
        (win, view, del, if scale > 0.0 { scale } else { 1.0 })
    };
    let (w, h) = (round(w_dip as f64 * scale), round(h_dip as f64 * scale));
    let owner = if owner >= 0 && (owner as usize) < PW_MAX { owner } else { -1 };
    with(|s| s.wins[slot] = Some(Win { win, view, del, scale, w, h, cursor: CUR_ARROW, ime: [0; 3], owner, enabled: true,
                                                        pos: [i32::MIN; 2] }));
    if x == PW_DEFAULT || y == PW_DEFAULT {
        // an owned window: 32 DIP below and right of the owner's client origin
        let at = if owner >= 0 {
            let d = rec(owner, |r| round(32.0 * r.scale)).unwrap_or(32);
            win_get_rect(owner).map(|r| (r[0] + d, r[1] + d))
        } else {
            None
        };
        match at {
            Some((ox, oy)) => win_set_rect(slot as i32, ox, oy, PW_KEEP, PW_KEEP),
            None => {
                with(|s| s.own = true);
                // SAFETY: center on our window (an own change: not reported).
                unsafe { msg!(win, c"center"; ()) };
                pos_changed(slot as i32);
                with(|s| s.own = false);
            }
        }
    } else {
        win_set_rect(slot as i32, x, y, PW_KEEP, PW_KEEP);
    }
    slot as i32
}

pub fn win_destroy(w: i32) {
    for k in 0..PPW_MAX as usize {
        if with(|s| s.pops[k].owner == w) == Some(true) {
            pop_hide(k);
        }
    }
    let Some(r) = with(|s| s.wins.get_mut(w as usize).and_then(|r| r.take())).flatten() else { return };
    // SAFETY: our window and delegate; the delegate goes after the window let it go.
    unsafe {
        let parent: Id = msg!(r.win, c"parentWindow"; Id);
        if !parent.is_null() {
            msg!(parent, c"removeChildWindow:", r.win => Id; ());
        }
        msg!(r.win, c"setDelegate:", NIL => Id; ());
        msg!(r.win, c"orderOut:", NIL => Id; ());
        msg!(r.win, c"close"; ());
    }
    release(r.win);
    release(r.del);
    if w == 0 {
        push(0, E_CLOSE, 0, 0, 0, 0); // the main window is gone: the loop ends (as on Windows)
    }
}

pub fn win_show(w: i32, mode: u32) {
    let Some((win, _, _)) = ids(w) else { return };
    let Some(app) = app() else { return };
    let parent = rec(w, |r| r.owner).filter(|&o| o >= 0).and_then(ids).map(|p| p.0);
    // SAFETY: our window, its owner's and the application object.
    unsafe {
        match mode {
            PWS_HIDE => {
                if let Some(p) = parent {
                    msg!(p, c"removeChildWindow:", win => Id; ());
                }
                msg!(win, c"orderOut:", NIL => Id; ())
            }
            PWS_SHOWNA => msg!(win, c"orderFront:", NIL => Id; ()),
            _ => {
                msg!(app, c"activateIgnoringOtherApps:", true => bool; ());
                msg!(win, c"makeKeyAndOrderFront:", NIL => Id; ());
            }
        }
        if let Some(p) = parent.filter(|_| mode != PWS_HIDE) {
            if msg!(win, c"parentWindow"; Id) != p {
                msg!(p, c"addChildWindow:ordered:", win => Id, 1isize => isize; ()); // NSWindowAbove
            }
        }
    }
    if mode != PWS_HIDE {
        size_changed(w, true);
        push(w, E_PAINT, 0, 0, 0, 0);
    }
}

pub fn win_activate(w: i32) {
    win_show(w, crate::PWS_SHOW);
}

/// a disabled window takes no input (its owned dialog is modal)
pub fn win_enable(w: i32, on: bool) {
    rec(w, |r| r.enabled = on);
}

fn enabled(w: i32) -> bool {
    rec(w, |r| r.enabled).unwrap_or(true)
}

/// the height of the primary screen in points (Cocoa's y grows upwards from its bottom)
fn primary_height() -> f64 {
    // SAFETY: NSScreen's class methods and the frame of the first screen.
    unsafe {
        let screens: Id = msg!(class(c"NSScreen"), c"screens"; Id);
        let n: usize = if screens.is_null() { 0 } else { msg!(screens, c"count"; usize) };
        if n == 0 {
            return 0.0;
        }
        let first: Id = msg!(screens, c"objectAtIndex:", 0usize => usize; Id);
        let f: NSRect = msg!(first, c"frame"; NSRect);
        f.size.h
    }
}

/// a rectangle of Cocoa's screen points -> [x, y, w, h] px with the origin at the top left
fn to_px(r: NSRect, scale: f64) -> [i32; 4] {
    let top = primary_height() - (r.origin.y + r.size.h);
    [round(r.origin.x * scale), round(top * scale), round(r.size.w * scale), round(r.size.h * scale)]
}

pub fn win_set_rect(w: i32, x: i32, y: i32, cw: i32, ch: i32) {
    let Some((win, _, scale)) = ids(w) else { return };
    let cur = win_get_rect(w).unwrap_or([0, 0, 0, 0]);
    let pick = |v: i32, old: i32| if v == PW_KEEP { old } else { v };
    let (x, y, cw, ch) = (pick(x, cur[0]), pick(y, cur[1]), pick(cw, cur[2]).max(1), pick(ch, cur[3]).max(1));
    let (pw, ph) = (cw as f64 / scale, ch as f64 / scale);
    let content = NSRect::new(x as f64 / scale, primary_height() - y as f64 / scale - ph, pw, ph);
    // SAFETY: frameRectForContentRect: / setFrame:display: of our window.
    with(|s| s.own = true);
    unsafe {
        let frame: NSRect = msg!(win, c"frameRectForContentRect:", content => NSRect; NSRect);
        msg!(win, c"setFrame:display:", frame => NSRect, true => bool; ());
    }
    size_changed(w, false); // the new size and place, quietly (an own change)
    pos_changed(w);
    with(|s| s.own = false);
}

pub fn win_get_rect(w: i32) -> Option<[i32; 4]> {
    let (win, _, scale) = ids(w)?;
    // SAFETY: frame / contentRectForFrameRect: of our window.
    let content = unsafe {
        let frame: NSRect = msg!(win, c"frame"; NSRect);
        msg!(win, c"contentRectForFrameRect:", frame => NSRect; NSRect)
    };
    Some(to_px(content, scale))
}

pub fn win_work_area(w: i32) -> Option<[i32; 4]> {
    let (win, _, scale) = ids(w)?;
    // SAFETY: the window's screen and its visible frame.
    let vis = unsafe {
        let screen: Id = msg!(win, c"screen"; Id);
        let screen: Id = if screen.is_null() { msg!(class(c"NSScreen"), c"mainScreen"; Id) } else { screen };
        if screen.is_null() {
            return None;
        }
        msg!(screen, c"visibleFrame"; NSRect)
    };
    Some(to_px(vis, scale))
}

pub fn win_set_title(w: i32, title: &str) {
    let Some((win, _, _)) = ids(w) else { return };
    let t = nsstring(title);
    // SAFETY: setTitle: of our window with a string we release after.
    unsafe { msg!(win, c"setTitle:", t => Id; ()) };
    release(t);
}

pub fn win_state(w: i32) -> u32 {
    let Some((win, _, _)) = ids(w) else { return 0 };
    let Some(app) = app() else { return 0 };
    // SAFETY: state queries of our window and the application.
    unsafe {
        let mut s = 0;
        if msg!(win, c"isZoomed"; bool) {
            s |= WST_MAXIMIZED;
        }
        if msg!(win, c"isMiniaturized"; bool) {
            s |= WST_MINIMIZED;
        }
        if msg!(win, c"styleMask"; u64) & STYLE_FULLSCREEN != 0 {
            s |= WST_FULLSCREEN;
        }
        if msg!(app, c"isActive"; bool) && !msg!(app, c"keyWindow"; Id).is_null() {
            s |= WST_ACTIVE;
        }
        s
    }
}

pub fn win_cmd(w: i32, cmd: u32) {
    let Some((win, _, _)) = ids(w) else { return };
    let fs = win_state(w) & WST_FULLSCREEN != 0;
    // SAFETY: window commands of our window.
    unsafe {
        match cmd {
            WCMD_MINIMIZE => msg!(win, c"miniaturize:", NIL => Id; ()),
            WCMD_MAXIMIZE => {
                if !msg!(win, c"isZoomed"; bool) {
                    msg!(win, c"zoom:", NIL => Id; ());
                }
            }
            WCMD_RESTORE => {
                if msg!(win, c"isMiniaturized"; bool) {
                    msg!(win, c"deminiaturize:", NIL => Id; ());
                } else if fs {
                    msg!(win, c"toggleFullScreen:", NIL => Id; ());
                } else if msg!(win, c"isZoomed"; bool) {
                    msg!(win, c"zoom:", NIL => Id; ());
                }
            }
            WCMD_CLOSE => push(w, E_CLOSE, 0, 0, 0, 0),
            WCMD_FULLSCREEN if !fs => msg!(win, c"toggleFullScreen:", NIL => Id; ()),
            WCMD_WINDOWED if fs => msg!(win, c"toggleFullScreen:", NIL => Id; ()),
            WCMD_LOWER => msg!(win, c"orderBack:", NIL => Id; ()),
            _ => {}
        }
    }
}

pub fn win_begin_move(w: i32) {
    let (Some((win, _, _)), Some(app)) = (ids(w), app()) else { return };
    // SAFETY: the current mouse-down event drags our window.
    unsafe {
        let ev: Id = msg!(app, c"currentEvent"; Id);
        if !ev.is_null() {
            msg!(win, c"performWindowDragWithEvent:", ev => Id; ());
        }
    }
}

/// the system resizes at the edges itself (a resizable window's frame)
pub fn win_begin_resize(_w: i32, _edges: u32) {}
pub fn win_show_menu(_w: i32, _x: i32, _y: i32) {}

pub fn win_size(w: i32) -> (i32, i32) {
    rec(w, |r| (r.w, r.h)).unwrap_or((0, 0))
}

pub fn win_dpi(w: i32) -> i32 {
    rec(w, |r| round(96.0 * r.scale)).unwrap_or(96)
}

pub fn win_set_min_size(w: i32, w_dip: i32, h_dip: i32) {
    match ids(w) {
        // SAFETY: setContentMinSize: of our window (points = DIP).
        Some((win, _, _)) => unsafe { msg!(win, c"setContentMinSize:", NSSize { w: w_dip as f64, h: h_dip as f64 } => NSSize; ()) },
        None if w == 0 => {
            with(|s| s.min0 = (w_dip, h_dip));
        }
        None => {}
    }
}

/// show the frame in window w (the whole frame; the layer keeps it for the window server)
pub fn win_present(w: i32, f: &Frame, _rects: Option<&[[i32; 4]]>) {
    let t0 = crate::time_us();
    let (Some((_, view, scale)), Some(cs)) = (ids(w), with(|s| s.cs)) else { return };
    if layer_show(view, f, scale, cs) {
        crate::present_account(t0, true);
    }
}

/// the frame as the contents of a view's layer (a CGImage of a copy of the pixels); false: no frame
fn layer_show(view: Id, f: &Frame, scale: f64, cs: CfRef) -> bool {
    let need = f.stride as usize * f.h.max(0) as usize;
    if f.w <= 0 || f.h <= 0 || f.stride < f.w * 4 || f.px.len() < need {
        return false;
    }
    // SAFETY: CoreGraphics objects made from a copy of the pixels and released here; the layer
    // retains the image it shows.
    unsafe {
        let data = CFDataCreate(core::ptr::null(), f.px.as_ptr(), need as isize);
        let prov = CGDataProviderCreateWithCFData(data);
        let img = CGImageCreate(f.w as usize, f.h as usize, 8, 32, f.stride as usize, cs, BITMAP_BGRX, prov, core::ptr::null(), false, 0);
        let layer: Id = msg!(view, c"layer"; Id);
        if !layer.is_null() && !img.is_null() {
            let tx = class(c"CATransaction");
            msg!(tx, c"begin"; ());
            msg!(tx, c"setDisableActions:", true => bool; ());
            msg!(layer, c"setContents:", img as Id => Id; ());
            msg!(layer, c"setContentsScale:", scale => f64; ());
            msg!(tx, c"commit"; ());
        }
        if !img.is_null() {
            CGImageRelease(img);
        }
        if !prov.is_null() {
            CGDataProviderRelease(prov);
        }
        if !data.is_null() {
            CFRelease(data);
        }
    }
    true
}

pub fn win_set_cursor(w: i32, shape: u32) {
    rec(w, |r| r.cursor = shape);
    apply_cursor(shape);
}

fn apply_cursor(shape: u32) {
    let c = cursor_obj(shape);
    if !c.is_null() {
        // SAFETY: set of a cursor object (shared or ours).
        unsafe { msg!(c, c"set"; ()) };
    }
}

/// the NSCursor of a shape: a system cursor, CUR_NONE's empty one, a custom one of cursor_create
fn cursor_obj(shape: u32) -> Id {
    if shape == CUR_NONE {
        return blank_cursor();
    }
    if shape >= CUR_USER0 {
        let c = with(|s| s.cursors.get((shape - CUR_USER0) as usize).copied()).flatten().unwrap_or(NIL);
        if !c.is_null() {
            return c;
        }
    }
    let name: &core::ffi::CStr = match shape {
        CUR_IBEAM => c"IBeamCursor",
        CUR_HAND => c"pointingHandCursor",
        CUR_CROSS | CUR_SIZE_NWSE | CUR_SIZE_NESW => c"crosshairCursor",
        CUR_MOVE => c"openHandCursor",
        CUR_SIZE_NS => c"resizeUpDownCursor",
        CUR_SIZE_WE => c"resizeLeftRightCursor",
        CUR_WAIT | CUR_ARROW => c"arrowCursor",
        _ => c"arrowCursor",
    };
    // SAFETY: an NSCursor class method (a shared cursor).
    unsafe { msg!(class(c"NSCursor"), name; Id) }
}

/// CUR_NONE: a cursor of an empty image (made once)
fn blank_cursor() -> Id {
    if let Some(c) = with(|s| s.blank).filter(|c| !c.is_null()) {
        return c;
    }
    // SAFETY: an empty NSImage of 1 x 1 points and a cursor of it, kept for the process.
    let c = unsafe {
        let img: Id = msg!(class(c"NSImage"), c"alloc"; Id);
        let img: Id = msg!(img, c"initWithSize:", NSSize { w: 1.0, h: 1.0 } => NSSize; Id);
        let c: Id = msg!(class(c"NSCursor"), c"alloc"; Id);
        let c: Id = msg!(c, c"initWithImage:hotSpot:", img => Id, NSPoint::default() => NSPoint; Id);
        release(img);
        c
    };
    with(|s| s.blank = c);
    c
}

pub fn win_hit_hook(f: Option<fn(i32, i32, i32) -> u32>) {
    if !init() {
        return;
    }
    with(|s| s.hit = f);
}

pub fn win_move_hook(f: Option<fn(i32, &mut [i32; 4])>) {
    if !init() {
        return;
    }
    with(|s| s.mv = f);
}

pub fn set_refresh(f: Option<fn()>) {
    if !init() {
        return;
    }
    with(|s| s.refresh = f);
}

/// pump AppKit until an event is queued or the deadline passes (0 = poll, u64::MAX = forever);
/// before any window: sleep until the deadline or a wake() (headless)
pub fn wait(deadline_us: u64) -> u32 {
    let Some(app) = app() else {
        if evq_count() == 0 {
            crate::wait_events(deadline_us, &mut |e| {
                evq_push(*e);
            });
        }
        return evq_count();
    };
    // SAFETY: AppKit's event loop on the main thread, inside an autorelease pool of ours.
    unsafe {
        let pool = super::objc::objc_autoreleasePoolPush();
        dnd::run_pending(); // drag_start's drag: its session runs in this loop
        let mut handled = 0;
        loop {
            if super::thread::wake_pending() {
                evq_push(super::thread::wake_consume());
            }
            // events queued already, or some handled: only what AppKit has at once, no waiting
            let now = crate::time_us();
            let date: Id = if evq_count() != 0 || handled > 0 || deadline_us == 0 || (deadline_us != u64::MAX && deadline_us <= now) {
                msg!(class(c"NSDate"), c"distantPast"; Id)
            } else if deadline_us == u64::MAX {
                msg!(class(c"NSDate"), c"distantFuture"; Id)
            } else {
                msg!(class(c"NSDate"), c"dateWithTimeIntervalSinceNow:", (deadline_us - now) as f64 / 1e6 => f64; Id)
            };
            let ev: Id = msg!(app, c"nextEventMatchingMask:untilDate:inMode:dequeue:", u64::MAX => u64, date => Id,
                              NSDefaultRunLoopMode => Id, true => bool; Id);
            if ev.is_null() {
                if super::thread::wake_pending() {
                    evq_push(super::thread::wake_consume());
                }
                break; // nothing more now, or the deadline passed
            }
            let typ: u64 = msg!(ev, c"type"; u64);
            if typ == EV_APP_DEFINED && msg!(ev, c"subtype"; i16) == WAKE_SUBTYPE {
                continue; // the wake-up: the top of the loop takes it
            }
            // NSApplication keeps a key-up with Command down from the window: it goes there itself
            if typ == EV_KEY_UP && msg!(ev, c"modifierFlags"; u64) & FLAG_COMMAND != 0 {
                let kw: Id = msg!(app, c"keyWindow"; Id);
                if !kw.is_null() {
                    msg!(kw, c"sendEvent:", ev => Id; ());
                    handled += 1;
                    continue;
                }
            }
            msg!(app, c"sendEvent:", ev => Id; ());
            handled += 1;
            if handled >= 64 {
                break;
            }
        }
        super::objc::objc_autoreleasePoolPop(pool);
    }
    evq_count()
}

// ---------------------------------------------------------------- popups

/// show / move / present popup slot `slot` over window owner: rect = x, y, w, h px of the owner's
/// client area (popup pixel 0, 0 = owner pixel x, y); the layer keeps the frame.  rect None hides it.
pub fn popup(slot: u32, owner: i32, rect: Option<[i32; 4]>, f: Option<&Frame>) {
    let k = slot as usize;
    if k >= PPW_MAX as usize {
        return;
    }
    let Some(rect) = rect else {
        pop_hide(k);
        return;
    };
    let (Some((owin, _, scale)), Some(mut p)) = (ids(owner), with(|s| s.pops[k])) else { return };
    if p.win.is_null() {
        let Some((win, view)) = pop_create(k) else { return };
        p.win = win;
        p.view = view;
    }
    let attach = p.parent != owin;
    let sized = attach || (p.w, p.h) != (rect[2], rect[3]);
    // SAFETY: our panel and the owner's window; points = px / the owner's scale.
    unsafe {
        if attach && !p.parent.is_null() {
            msg!(p.parent, c"removeChildWindow:", p.win => Id; ());
        }
        let fr: NSRect = msg!(owin, c"frame"; NSRect);
        let content: NSRect = msg!(owin, c"contentRectForFrameRect:", fr => NSRect; NSRect);
        let (pw, ph) = (rect[2].max(1) as f64 / scale, rect[3].max(1) as f64 / scale);
        let x = content.origin.x + rect[0] as f64 / scale;
        let top = content.origin.y + content.size.h - rect[1] as f64 / scale;
        msg!(p.win, c"setFrame:display:", NSRect::new(x, top - ph, pw, ph) => NSRect, false => bool; ());
    }
    p.owner = owner;
    p.parent = owin;
    p.dx = rect[0];
    p.dy = rect[1];
    p.w = rect[2];
    p.h = rect[3];
    with(|s| s.pops[k] = p);
    if let (Some(f), Some(cs)) = (f, with(|s| s.cs)) {
        layer_show(p.view, f, scale, cs);
    }
    // SAFETY: as above.
    unsafe {
        if attach {
            msg!(owin, c"addChildWindow:ordered:", p.win => Id, 1isize => isize; ()); // NSWindowAbove
            msg!(p.win, c"orderFront:", NIL => Id; ());
        }
        if sized {
            msg!(p.win, c"invalidateShadow"; ());
        }
    }
}

/// (§6) how the next popup() hangs at its anchor: the UI places popups on macOS itself
pub fn popup_anchor(_slot: u32, _anchor: [i32; 4], _kind: u32, _nat: [i32; 2]) {}

/// (§6) the platform never moves a popup on macOS
pub fn popup_placed(_slot: u32) -> Option<[i32; 2]> {
    None
}

/// a new popup panel for slot k: borderless, non-activating, transparent around rounded corners,
/// with a shadow
fn pop_create(k: usize) -> Option<(Id, Id)> {
    let view_cls = with(|s| s.view_cls)?;
    let rect = NSRect::new(0.0, 0.0, 1.0, 1.0);
    // SAFETY: AppKit objects made on the main thread; every message has its exact signature.
    unsafe {
        let win: Id = msg!(class(c"NSPanel"), c"alloc"; Id);
        let win: Id = msg!(win, c"initWithContentRect:styleMask:backing:defer:", rect => NSRect, STYLE_NONACTIVATING => u64, 2u64 => u64,
                           false => bool; Id);
        if win.is_null() {
            return None;
        }
        msg!(win, c"setReleasedWhenClosed:", false => bool; ());
        msg!(win, c"setHidesOnDeactivate:", false => bool; ());
        msg!(win, c"setBecomesKeyOnlyIfNeeded:", true => bool; ());
        msg!(win, c"setOpaque:", false => bool; ());
        let clear: Id = msg!(class(c"NSColor"), c"clearColor"; Id);
        msg!(win, c"setBackgroundColor:", clear => Id; ());
        msg!(win, c"setHasShadow:", true => bool; ());
        let view: Id = msg!(view_cls, c"alloc"; Id);
        let view: Id = msg!(view, c"initWithFrame:", rect => NSRect; Id);
        super::objc::ivar_set(view, VIEW_OFF.load(Ordering::SeqCst), POP_BASE + k as isize);
        msg!(view, c"setWantsLayer:", true => bool; ());
        msg!(view, c"setLayerContentsRedrawPolicy:", 0isize => isize; ());
        let layer: Id = msg!(view, c"layer"; Id);
        if !layer.is_null() {
            msg!(layer, c"setContentsGravity:", kCAGravityTopLeft => Id; ());
            msg!(layer, c"setCornerRadius:", POP_RADIUS => f64; ());
            msg!(layer, c"setMasksToBounds:", true => bool; ());
        }
        let ta: Id = msg!(class(c"NSTrackingArea"), c"alloc"; Id);
        let ta: Id = msg!(ta, c"initWithRect:options:owner:userInfo:", NSRect::default() => NSRect, TRACK_OPTIONS => u64, view => Id, NIL => Id; Id);
        msg!(view, c"addTrackingArea:", ta => Id; ());
        release(ta);
        msg!(win, c"setContentView:", view => Id; ());
        release(view); // the panel holds it
        msg!(win, c"setAcceptsMouseMovedEvents:", true => bool; ());
        Some((win, view))
    }
}

/// hide popup slot k (off its owner's window)
fn pop_hide(k: usize) {
    let Some(p) = with(|s| {
        let p = s.pops[k];
        s.pops[k].owner = -1;
        s.pops[k].parent = NIL;
        p
    }) else {
        return;
    };
    if p.win.is_null() {
        return;
    }
    // SAFETY: our panel and the window it hangs on.
    unsafe {
        if !p.parent.is_null() {
            msg!(p.parent, c"removeChildWindow:", p.win => Id; ());
        }
        msg!(p.win, c"orderOut:", NIL => Id; ());
    }
}

/// the pointer is over a shown popup of window `owner` (owner_too: or over the owner's content)
fn pointer_over(owner: i32, owner_too: bool) -> bool {
    // SAFETY: NSEvent's class method and the frames of our windows.
    unsafe {
        let m: NSPoint = msg!(class(c"NSEvent"), c"mouseLocation"; NSPoint);
        let inside = |r: NSRect| m.x >= r.origin.x && m.x < r.origin.x + r.size.w && m.y >= r.origin.y && m.y < r.origin.y + r.size.h;
        for k in 0..PPW_MAX as usize {
            let Some(p) = with(|s| s.pops[k]) else { return false };
            if p.owner == owner && !p.win.is_null() && inside(msg!(p.win, c"frame"; NSRect)) {
                return true;
            }
        }
        if owner_too && let Some((win, _, _)) = ids(owner) {
            let fr: NSRect = msg!(win, c"frame"; NSRect);
            return inside(msg!(win, c"contentRectForFrameRect:", fr => NSRect; NSRect));
        }
        false
    }
}
/// a cursor from w x h premultiplied 0xAARRGGBB pixels (rows of w * 4 bytes), hot spot in px, drawn
/// for dpi (the window's: its points are px * 96 / dpi) -> CUR_USER0 + k / -1
pub fn cursor_create(px: &[u8], w: i32, h: i32, hot_x: i32, hot_y: i32, dpi: i32) -> i32 {
    let n = (w.max(0) * h.max(0)) as usize;
    if (w - 1) as u32 >= CUR_MAX_PX as u32 || (h - 1) as u32 >= CUR_MAX_PX as u32 || px.len() < n * 4 || !init() {
        return -1;
    }
    let Some(k) = with(|s| s.cursors.iter().position(|c| c.is_null())).flatten() else { return -1 };
    let scale = if dpi > 0 { dpi as f64 / 96.0 } else { 1.0 };
    // SAFETY: a bitmap of our own (it allocates its pixels, w * h * 4 bytes, filled here), an image
    // and a cursor of it; what is not kept is released.
    let cur = unsafe {
        let cs = nsstring("NSDeviceRGBColorSpace");
        let rep: Id = msg!(class(c"NSBitmapImageRep"), c"alloc"; Id);
        let rep: Id = msg!(rep, c"initWithBitmapDataPlanes:pixelsWide:pixelsHigh:bitsPerSample:samplesPerPixel:hasAlpha:isPlanar:colorSpaceName:bitmapFormat:bytesPerRow:bitsPerPixel:",
                           core::ptr::null_mut::<*mut u8>() => *mut *mut u8, w as isize => isize, h as isize => isize, 8isize => isize,
                           4isize => isize, true => bool, false => bool, cs => Id, 0u64 => u64, (w * 4) as isize => isize, 32isize => isize; Id);
        release(cs);
        if rep.is_null() {
            return -1;
        }
        let dst: *mut u8 = msg!(rep, c"bitmapData"; *mut u8);
        if dst.is_null() {
            release(rep);
            return -1;
        }
        // premultiplied R, G, B, A bytes (the bitmap's default format)
        let out = core::slice::from_raw_parts_mut(dst, n * 4);
        for (d, p) in out.chunks_exact_mut(4).zip(px.chunks_exact(4)) {
            d.copy_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
        let img: Id = msg!(class(c"NSImage"), c"alloc"; Id);
        let img: Id = msg!(img, c"initWithSize:", NSSize { w: w as f64 / scale, h: h as f64 / scale } => NSSize; Id);
        msg!(img, c"addRepresentation:", rep => Id; ());
        release(rep);
        let c: Id = msg!(class(c"NSCursor"), c"alloc"; Id);
        let c: Id = msg!(c, c"initWithImage:hotSpot:", img => Id, NSPoint { x: hot_x as f64 / scale, y: hot_y as f64 / scale } => NSPoint; Id);
        release(img);
        c
    };
    if cur.is_null() {
        return -1;
    }
    with(|s| s.cursors[k] = cur);
    (CUR_USER0 as usize + k) as i32
}

/// free a custom cursor; windows showing it fall back to the arrow
pub fn cursor_destroy(id: i32) {
    let k = (id as u32).wrapping_sub(CUR_USER0) as usize;
    let Some(c) = with(|s| s.cursors.get_mut(k).map(|c| core::mem::replace(c, NIL))).flatten() else { return };
    if c.is_null() {
        return;
    }
    let mut shown = false;
    with(|s| {
        for r in s.wins.iter_mut().flatten() {
            if r.cursor == id as u32 {
                r.cursor = CUR_ARROW;
                shown = true;
            }
        }
    });
    if shown {
        apply_cursor(CUR_ARROW);
    }
    release(c);
}
/// the dpi to draw a cursor for a window of this dpi (the window's own)
pub fn cursor_dpi(dpi: i32) -> i32 {
    dpi
}

// ---------------------------------------------------------------- the file dialog

/// the system's Open / Save panel (modal) -> (1 chosen / 0 cancelled / -1 none, index of the chosen
/// extension entry).  flags: FD_SAVE, FD_FOLDER (an Open panel for folders), the selected entry in
/// bits 8..15 (FD_EXT_SHIFT); path: the start (a folder, or a file whose folder and, saving, name
/// are taken) in, the chosen path out; exts: ["png=PNG-Bild", "jpg;jpeg"] (see win_filedlg.rs)
pub fn file_dialog(flags: u32, path: &mut String, exts: &[&str]) -> (i32, u32) {
    if !init() {
        return (-1, 0);
    }
    let (save, folder) = (flags & FD_SAVE != 0, flags & FD_FOLDER != 0);
    let sel = ((flags >> FD_EXT_SHIFT) & 0xFF) as usize;
    let lists: Vec<Vec<&str>> = exts
        .iter()
        .map(|e| e.split('=').next().unwrap_or("").split(';').map(str::trim).filter(|x| !x.is_empty()).collect())
        .collect();
    let start = path.replace('\\', "/");
    let (dir, name) = split_start(&start);
    let auto = with(|s| s.autoclose.take()).flatten();
    // SAFETY: a pool of ours, popped below on this thread.
    let pool = unsafe { super::objc::objc_autoreleasePoolPush() };
    // SAFETY: the shared panels and their setters with objects made here; runModal on the main thread.
    let r = unsafe {
        let panel: Id = if save { msg!(class(c"NSSavePanel"), c"savePanel"; Id) } else { msg!(class(c"NSOpenPanel"), c"openPanel"; Id) };
        if panel.is_null() {
            super::objc::objc_autoreleasePoolPop(pool);
            return (-1, 0);
        }
        if !save {
            msg!(panel, c"setCanChooseFiles:", !folder => bool; ());
            msg!(panel, c"setCanChooseDirectories:", folder => bool; ());
            msg!(panel, c"setAllowsMultipleSelection:", false => bool; ());
        }
        msg!(panel, c"setCanCreateDirectories:", true => bool; ());
        let (mut dir, mut name) = (dir, name);
        if let Some((_, Some(full))) = &auto {
            (dir, name) = split_start(full); // (test aid) the name the dialog is closed with
        }
        if !dir.is_empty() && crate::file_exists(&dir) == 2 {
            let ds = nsstring(&dir);
            let url: Id = msg!(class(c"NSURL"), c"fileURLWithPath:isDirectory:", ds => Id, true => bool; Id);
            msg!(panel, c"setDirectoryURL:", url => Id; ());
            release(ds);
        }
        if save && !name.is_empty() {
            let ns = nsstring(&name);
            msg!(panel, c"setNameFieldStringValue:", ns => Id; ());
            release(ns);
        }
        // the types: opening every listed extension; saving the selected entry's (its first is appended)
        let types: Vec<&str> = if save { lists.get(sel).cloned().unwrap_or_default() } else { lists.concat() };
        if !folder && !types.is_empty() {
            let arr: Id = msg!(class(c"NSMutableArray"), c"array"; Id);
            for t in &types {
                let ts = nsstring(t);
                msg!(arr, c"addObject:", ts => Id; ());
                release(ts);
            }
            msg!(panel, c"setAllowedFileTypes:", arr => Id; ());
            msg!(panel, c"setAllowsOtherFileTypes:", save => bool; ());
        }
        // (test aid) Cancel: the panel's cancel:; OK: NSApp's stopModal (the panel runs in another
        // process, its ok: is not taken from here) -- runModal answers NSModalResponseStop then
        let auto_ok = auto.as_ref().is_some_and(|a| a.1.is_some()) && save;
        if let Some((ms, _)) = &auto {
            let modes: Id = msg!(class(c"NSArray"), c"arrayWithObject:", NSRunLoopCommonModes => Id; Id);
            let (to, close) = if auto_ok { (app().unwrap_or(NIL), super::objc::sel(c"stopModal")) } else { (panel, super::objc::sel(c"cancel:")) };
            msg!(to, c"performSelector:withObject:afterDelay:inModes:", close => Sel, NIL => Id, *ms as f64 / 1000.0 => f64, modes => Id; ());
        }
        let r: isize = msg!(panel, c"runModal"; isize);
        let r = if auto_ok && r == -1000 { 1 } else { r };
        if r != 1 {
            (0, sel as u32)
        } else {
            let url: Id = if save {
                msg!(panel, c"URL"; Id)
            } else {
                let urls: Id = msg!(panel, c"URLs"; Id);
                if urls.is_null() { NIL } else { msg!(urls, c"firstObject"; Id) }
            };
            let chosen = if url.is_null() { String::new() } else { string_of(msg!(url, c"path"; Id)) };
            if chosen.is_empty() {
                (0, sel as u32)
            } else {
                let ext = chosen.rsplit('/').next().and_then(|f| f.rsplit_once('.')).map(|x| x.1).unwrap_or("");
                let idx = lists.iter().position(|l| l.iter().any(|e| e.eq_ignore_ascii_case(ext))).unwrap_or(sel);
                *path = chosen;
                (1, idx as u32)
            }
        }
    };
    // SAFETY: the pool pushed above.
    unsafe { super::objc::objc_autoreleasePoolPop(pool) };
    r
}

/// a start path -> (its folder, the name): an existing folder is the folder itself
fn split_start(p: &str) -> (String, String) {
    if p.is_empty() || crate::file_exists(p) == 2 {
        return (String::from(p), String::new());
    }
    match p.rfind('/') {
        Some(0) => (String::from("/"), String::from(&p[1..])),
        Some(k) => (String::from(&p[..k]), String::from(&p[k + 1..])),
        None => (String::new(), String::from(p)),
    }
}

// ---------------------------------------------------------------- monitors, settings

/// (screen.rs) every screen: desktop units are px (points * the screen's scale) from the top left
/// of the primary screen, which carries the menu bar
pub(crate) fn monitors_os() -> Vec<crate::screen::Monitor> {
    let mut out = Vec::new();
    // NSScreen needs the application object (an app without windows yet makes it here)
    if !init() {
        return out;
    }
    let ph = primary_height();
    // SAFETY: NSScreen's list and the properties of its screens; a pool of ours for the temporaries.
    unsafe {
        let pool = super::objc::objc_autoreleasePoolPush();
        let screens: Id = msg!(class(c"NSScreen"), c"screens"; Id);
        let n: usize = if screens.is_null() { 0 } else { msg!(screens, c"count"; usize) };
        for k in 0..n {
            let sc: Id = msg!(screens, c"objectAtIndex:", k => usize; Id);
            let scale: f64 = msg!(sc, c"backingScaleFactor"; f64);
            let scale = if scale > 0.0 { scale } else { 1.0 };
            let px = |r: NSRect| {
                let top = ph - (r.origin.y + r.size.h);
                [round(r.origin.x * scale), round(top * scale), round(r.size.w * scale), round(r.size.h * scale)]
            };
            let (frame, vis): (NSRect, NSRect) = (msg!(sc, c"frame"; NSRect), msg!(sc, c"visibleFrame"; NSRect));
            let desc: Id = msg!(sc, c"deviceDescription"; Id);
            let key = nsstring("NSScreenNumber");
            let num: Id = if desc.is_null() { NIL } else { msg!(desc, c"objectForKey:", key => Id; Id) };
            release(key);
            let id: u32 = if num.is_null() { k as u32 } else { msg!(num, c"unsignedIntValue"; u32) };
            let name = if msg!(sc, c"respondsToSelector:", super::objc::sel(c"localizedName") => Sel; bool) {
                string_of(msg!(sc, c"localizedName"; Id))
            } else {
                String::new()
            };
            let fps: isize = if msg!(sc, c"respondsToSelector:", super::objc::sel(c"maximumFramesPerSecond") => Sel; bool) {
                msg!(sc, c"maximumFramesPerSecond"; isize)
            } else {
                0
            };
            out.push(crate::screen::Monitor {
                id: id as u64,
                name,
                rect: px(frame),
                work: px(vis),
                dpi: round(96.0 * scale) as u32,
                refresh_mhz: fps.max(0) as u32 * 1000,
                primary: k == 0,
            });
        }
        super::objc::objc_autoreleasePoolPop(pool);
    }
    out
}

/// (settings.rs) the appearance, the accent colour (NSColor.controlAccentColor in sRGB), more
/// contrast, less motion, the double-click time
pub(crate) fn sys_settings_os(s: &mut crate::SysSettings) {
    if !init() {
        return;
    }
    s.dark = Some(theme() == 1);
    // SAFETY: class methods of NSColor / NSWorkspace / NSEvent, a pool of ours for the temporaries.
    unsafe {
        let pool = super::objc::objc_autoreleasePoolPush();
        let nscolor = class(c"NSColor");
        if msg!(nscolor, c"respondsToSelector:", super::objc::sel(c"controlAccentColor") => Sel; bool) {
            let c: Id = msg!(nscolor, c"controlAccentColor"; Id);
            let srgb: Id = msg!(class(c"NSColorSpace"), c"sRGBColorSpace"; Id);
            let c: Id = if c.is_null() { NIL } else { msg!(c, c"colorUsingColorSpace:", srgb => Id; Id) };
            if !c.is_null() {
                let ch = |v: f64| round(v.clamp(0.0, 1.0) * 255.0) as u32;
                let (r, g, b): (f64, f64, f64) = (msg!(c, c"redComponent"; f64), msg!(c, c"greenComponent"; f64), msg!(c, c"blueComponent"; f64));
                s.accent = Some(ch(r) << 16 | ch(g) << 8 | ch(b));
            }
        }
        let ws: Id = msg!(class(c"NSWorkspace"), c"sharedWorkspace"; Id);
        if !ws.is_null() {
            s.high_contrast = msg!(ws, c"accessibilityDisplayShouldIncreaseContrast"; bool);
            s.reduce_motion = msg!(ws, c"accessibilityDisplayShouldReduceMotion"; bool);
        }
        let dc: f64 = msg!(class(c"NSEvent"), c"doubleClickInterval"; f64);
        if dc > 0.0 {
            s.double_click_ms = round(dc * 1000.0) as u32;
        }
        super::objc::objc_autoreleasePoolPop(pool);
    }
}

// ---------------------------------------------------------------- the clipboard

/// the pasteboard type of a CLIP_* format (CLIP_IMAGE: see image_types)
fn clip_type(fmt: u32) -> Option<&'static CStr> {
    Some(match fmt {
        CLIP_TEXT => c"public.utf8-plain-text",
        CLIP_PNG | CLIP_IMAGE => UTI_PNG,
        CLIP_HTML => c"public.html",
        CLIP_RTF => c"public.rtf",
        CLIP_DOCX => c"org.jflib.docx-fragment",
        _ => return None,
    })
}

const UTI_PNG: &CStr = c"public.png";
const UTI_TIFF: &CStr = c"public.tiff";
const UTI_BMP: &CStr = c"com.microsoft.bmp";

fn pasteboard() -> Id {
    // SAFETY: the general pasteboard (a shared object).
    unsafe { msg!(class(c"NSPasteboard"), c"generalPasteboard"; Id) }
}

/// CLIP_TEXT / CLIP_HTML / CLIP_RTF / CLIP_DOCX as clip_set_many; CLIP_PNG: the PNG file;
/// CLIP_IMAGE: a CLIPIMG record (clipimg_pack): its PNG, else a PNG made of its BMP, and the BMP
pub fn clip_set(fmt: u32, data: &[u8]) -> bool {
    match fmt {
        CLIP_PNG => !data.is_empty() && clip_put(&[(UTI_PNG, data)]),
        CLIP_IMAGE => {
            let Some((png, bmp)) = crate::clipimg_split(data) else { return false };
            let made = if png.is_empty() && !bmp.is_empty() { image_convert(bmp, "public.png") } else { None };
            let png = made.as_deref().unwrap_or(png);
            match (png.is_empty(), bmp.is_empty()) {
                (false, false) => clip_put(&[(UTI_PNG, png), (UTI_BMP, bmp)]),
                (false, true) => clip_put(&[(UTI_PNG, png)]),
                (true, false) => clip_put(&[(UTI_BMP, bmp)]),
                (true, true) => false,
            }
        }
        _ => clip_set_many(&[(fmt, data)]),
    }
}

/// several representations of one selection (text formats, cliprich.rs)
pub fn clip_set_many(entries: &[(u32, &[u8])]) -> bool {
    if !crate::clip_many_valid(entries) || entries.iter().any(|(f, _)| clip_type(*f).is_none()) {
        return false;
    }
    let mut v: Vec<(&CStr, &[u8])> = Vec::new();
    for (f, d) in entries {
        v.push((clip_type(*f).unwrap_or(c""), *d));
    }
    clip_put(&v)
}

/// the pasteboard emptied, then each (type, bytes) on it (text as a string)
fn clip_put(entries: &[(&CStr, &[u8])]) -> bool {
    let pb = pasteboard();
    if pb.is_null() {
        return false;
    }
    // SAFETY: a pool of ours, popped below on this thread.
    let pool = unsafe { super::objc::objc_autoreleasePoolPush() };
    // SAFETY: the pasteboard's calls with NSStrings / NSData made here (autoreleased in our pool).
    let ok = unsafe {
        msg!(pb, c"clearContents"; isize);
        entries.iter().all(|(t, d)| {
            let ty = nsstring(t.to_str().unwrap_or(""));
            let ok = if *t == c"public.utf8-plain-text" {
                let s = nsstring(core::str::from_utf8(d).unwrap_or(""));
                let ok = msg!(pb, c"setString:forType:", s => Id, ty => Id; bool);
                release(s);
                ok
            } else {
                let nd: Id = msg!(class(c"NSData"), c"dataWithBytes:length:", d.as_ptr() => *const u8, d.len() => usize; Id);
                msg!(pb, c"setData:forType:", nd => Id, ty => Id; bool)
            };
            release(ty);
            ok
        })
    };
    // SAFETY: the pool pushed above.
    unsafe { super::objc::objc_autoreleasePoolPop(pool) };
    ok
}

/// the bytes of one pasteboard type (text: its UTF-8)
fn clip_data(t: &CStr) -> Option<Vec<u8>> {
    let pb = pasteboard();
    if pb.is_null() {
        return None;
    }
    // SAFETY: a pool of ours, popped below on this thread.
    let pool = unsafe { super::objc::objc_autoreleasePoolPush() };
    let ty = nsstring(t.to_str().unwrap_or(""));
    // SAFETY: the pasteboard's getters; the bytes are copied before the pool ends.
    let out = unsafe {
        if t == c"public.utf8-plain-text" {
            let s: Id = msg!(pb, c"stringForType:", ty => Id; Id);
            (!s.is_null()).then(|| string_of(s).into_bytes())
        } else {
            let d: Id = msg!(pb, c"dataForType:", ty => Id; Id);
            if d.is_null() {
                None
            } else {
                let p: *const u8 = msg!(d, c"bytes"; *const u8);
                let n: usize = msg!(d, c"length"; usize);
                Some(if p.is_null() { Vec::new() } else { core::slice::from_raw_parts(p, n).to_vec() })
            }
        }
    };
    release(ty);
    // SAFETY: the pool pushed above.
    unsafe { super::objc::objc_autoreleasePoolPop(pool) };
    out
}

/// CLIP_IMAGE: the PNG, else a TIFF (screenshots, Preview) as a PNG, else a BMP (clip_bmp_fix)
pub fn clip_get(fmt: u32) -> Option<Vec<u8>> {
    if fmt == CLIP_IMAGE {
        return clip_data(UTI_PNG)
            .filter(|d| !d.is_empty())
            .or_else(|| clip_data(UTI_TIFF).and_then(|t| image_convert(&t, "public.png")))
            .or_else(|| clip_data(UTI_BMP).and_then(|b| crate::clip_bmp_fix(&b)));
    }
    let data = clip_data(clip_type(fmt)?)?;
    if matches!(fmt, CLIP_HTML | CLIP_RTF | CLIP_DOCX) && data.len() > crate::CLIP_RICH_MAX {
        return None;
    }
    if fmt == CLIP_HTML && (data.contains(&0) || core::str::from_utf8(&data).is_err()) {
        return None;
    }
    Some(data)
}

pub fn clip_has(fmt: u32) -> bool {
    match fmt {
        CLIP_IMAGE => [UTI_PNG, UTI_TIFF, UTI_BMP].iter().any(|t| clip_offers(t)),
        _ => clip_type(fmt).is_some_and(clip_offers),
    }
}

/// the pasteboard offers that type (nothing is read)
fn clip_offers(t: &CStr) -> bool {
    let pb = pasteboard();
    if pb.is_null() {
        return false;
    }
    // SAFETY: a pool of ours, popped below on this thread.
    let pool = unsafe { super::objc::objc_autoreleasePoolPush() };
    let ty = nsstring(t.to_str().unwrap_or(""));
    // SAFETY: availableTypeFromArray: with a one-element array of ours.
    let has = unsafe {
        let arr: Id = msg!(class(c"NSArray"), c"arrayWithObject:", ty => Id; Id);
        !msg!(pb, c"availableTypeFromArray:", arr => Id; Id).is_null()
    };
    release(ty);
    // SAFETY: the pool pushed above.
    unsafe { super::objc::objc_autoreleasePoolPop(pool) };
    has
}

/// an image file (PNG, TIFF, BMP, ...) as a file of type `to` ("public.png") through ImageIO
fn image_convert(data: &[u8], to: &str) -> Option<Vec<u8>> {
    if data.is_empty() || data.len() > isize::MAX as usize {
        return None;
    }
    // SAFETY: CoreFoundation / ImageIO objects made here and released here; the result is copied.
    unsafe {
        let src_data = CFDataCreate(core::ptr::null(), data.as_ptr(), data.len() as isize);
        if src_data.is_null() {
            return None;
        }
        let src = CGImageSourceCreateWithData(src_data, core::ptr::null());
        CFRelease(src_data);
        if src.is_null() {
            return None;
        }
        let img = CGImageSourceCreateImageAtIndex(src, 0, core::ptr::null());
        CFRelease(src);
        if img.is_null() {
            return None;
        }
        let out = CFDataCreateMutable(core::ptr::null(), 0);
        let ty = nsstring(to);
        let dst = if out.is_null() { core::ptr::null() } else { CGImageDestinationCreateWithData(out, ty as CfRef, 1, core::ptr::null()) };
        release(ty);
        let mut r = None;
        if !dst.is_null() {
            CGImageDestinationAddImage(dst, img, core::ptr::null());
            if CGImageDestinationFinalize(dst) {
                let (p, n) = (CFDataGetBytePtr(out), CFDataGetLength(out));
                if !p.is_null() && n > 0 {
                    r = Some(core::slice::from_raw_parts(p, n as usize).to_vec());
                }
            }
            CFRelease(dst);
        }
        CGImageRelease(img);
        if !out.is_null() {
            CFRelease(out);
        }
        r
    }
}

/// where the input method's candidate window goes: the caret (px of the current window)
pub fn ime_pos(x: i32, y: i32, h: i32) {
    let w = crate::win_current().max(0);
    rec(w, |r| r.ime = [x, y, h]);
}


// ---------------------------------------------------------------- sizes

/// the view's size in pixels and the window's scale now; push E_DPI / E_RESIZE when they changed
/// (force: E_RESIZE anyway)
fn size_changed(w: i32, force: bool) {
    let Some((win, view, old_scale)) = ids(w) else { return };
    let quiet = with(|s| s.own).unwrap_or(false); // win_set_rect: an own change
    // SAFETY: backingScaleFactor / bounds of our window and view.
    let (scale, b) = unsafe {
        let s: f64 = msg!(win, c"backingScaleFactor"; f64);
        (if s > 0.0 { s } else { 1.0 }, msg!(view, c"bounds"; NSRect))
    };
    let (pw, ph) = (round(b.size.w * scale), round(b.size.h * scale));
    let old = rec(w, |r| {
        let o = (r.w, r.h);
        r.scale = scale;
        r.w = pw;
        r.h = ph;
        o
    })
    .unwrap_or((0, 0));
    if quiet {
        return;
    }
    if scale != old_scale {
        push(w, E_DPI, 0, 0, 0, round(96.0 * scale) as u32);
    }
    if force || old != (pw, ph) || scale != old_scale {
        push(w, E_RESIZE, 0, pw, ph, 0);
    }
}

// ---------------------------------------------------------------- JFView

fn view_win(this: Id) -> i32 {
    super::objc::ivar_get(this, VIEW_OFF.load(Ordering::SeqCst)) as i32
}

fn is_popup(this: Id) -> bool {
    super::objc::ivar_get(this, VIEW_OFF.load(Ordering::SeqCst)) >= POP_BASE
}

/// the window a view's pointer input belongs to, the view's offset in it (px) and the scale: a
/// popup's input is its owner's (owner client coordinates)
fn target(this: Id) -> Option<(i32, i32, i32, f64)> {
    let v = super::objc::ivar_get(this, VIEW_OFF.load(Ordering::SeqCst));
    if v >= POP_BASE {
        let p = with(|s| s.pops.get((v - POP_BASE) as usize).copied()).flatten()?;
        if p.owner < 0 {
            return None;
        }
        Some((p.owner, p.dx, p.dy, rec(p.owner, |r| r.scale)?))
    } else {
        Some((v as i32, 0, 0, rec(v as i32, |r| r.scale).unwrap_or(1.0)))
    }
}

/// the window of the event and its position there: pixels from the top left
fn ev_pos(this: Id, ev: Id) -> Option<(i32, i32, i32)> {
    let (w, dx, dy, scale) = target(this)?;
    // SAFETY: locationInWindow / convertPoint:fromView: of the event and our view.
    let p = unsafe {
        let p: NSPoint = msg!(ev, c"locationInWindow"; NSPoint);
        msg!(this, c"convertPoint:fromView:", p => NSPoint, NIL => Id; NSPoint)
    };
    Some((w, floor(p.x * scale) as i32 + dx, floor(p.y * scale) as i32 + dy))
}

fn ev_flags(ev: Id) {
    // SAFETY: modifierFlags of an event.
    let f: u64 = unsafe { msg!(ev, c"modifierFlags"; u64) };
    with(|s| s.flags = f);
}

extern "C" fn v_yes(_this: Id, _cmd: Sel) -> bool {
    true
}
extern "C" fn v_yes1(_this: Id, _cmd: Sel, _a: Id) -> bool {
    true
}

fn button(this: Id, ev: Id, typ: u8, btn: u8) {
    ev_flags(ev);
    let Some((w, x, y)) = ev_pos(this, ev) else { return };
    if !enabled(w) {
        return;
    }
    if typ == E_DOWN && btn == BTN_LEFT && !is_popup(this) {
        dnd::note_mouse(ev); // a drag out starts from it
        let hit = with(|s| s.hit).flatten();
        if let Some(h) = hit {
            let hb = h(w, x, y);
            if hb & HB_DRAG != 0 && hb & (HB_CLOSE | HB_MIN | HB_MAX) == 0 {
                if let Some((win, _, _)) = ids(w) {
                    // SAFETY: a double click zooms, else the system drags our window with this event.
                    unsafe {
                        if msg!(ev, c"clickCount"; isize) == 2 {
                            msg!(win, c"zoom:", NIL => Id; ());
                        } else {
                            msg!(win, c"performWindowDragWithEvent:", ev => Id; ());
                        }
                    }
                }
                return;
            }
        }
    }
    push(w, typ, btn, x, y, 0);
}

extern "C" fn v_mouse_down(this: Id, _cmd: Sel, ev: Id) {
    button(this, ev, E_DOWN, BTN_LEFT);
}
extern "C" fn v_mouse_up(this: Id, _cmd: Sel, ev: Id) {
    button(this, ev, E_UP, BTN_LEFT);
}
extern "C" fn v_right_down(this: Id, _cmd: Sel, ev: Id) {
    button(this, ev, E_DOWN, BTN_RIGHT);
}
extern "C" fn v_right_up(this: Id, _cmd: Sel, ev: Id) {
    button(this, ev, E_UP, BTN_RIGHT);
}
fn other_button(ev: Id) -> u8 {
    // SAFETY: buttonNumber of a mouse event.
    match unsafe { msg!(ev, c"buttonNumber"; isize) } {
        3 => BTN_X1,
        4 => BTN_X2,
        _ => BTN_MIDDLE,
    }
}
extern "C" fn v_other_down(this: Id, _cmd: Sel, ev: Id) {
    button(this, ev, E_DOWN, other_button(ev));
}
extern "C" fn v_other_up(this: Id, _cmd: Sel, ev: Id) {
    button(this, ev, E_UP, other_button(ev));
}
extern "C" fn v_mouse_moved(this: Id, _cmd: Sel, ev: Id) {
    ev_flags(ev);
    let Some((w, x, y)) = ev_pos(this, ev) else { return };
    if !enabled(w) {
        return;
    }
    // SAFETY: the type of an event.
    if !is_popup(this) && unsafe { msg!(ev, c"type"; u64) } == EV_LEFT_DRAGGED {
        dnd::note_mouse(ev);
    }
    if let Some(c) = rec(w, |r| r.cursor) {
        apply_cursor(c);
    }
    push(w, E_MOVE, 0, x, y, 0);
}
/// onto the owner or another popup of it: no leave
extern "C" fn v_mouse_exited(this: Id, _cmd: Sel, _ev: Id) {
    let Some((w, _, _, _)) = target(this) else { return };
    if is_popup(this) {
        if !pointer_over(w, true) {
            push(w, E_LEAVE, 0, -1, -1, 0);
        }
    } else if !pointer_over(w, false) {
        push(w, E_LEAVE, 0, 0, 0, 0);
    }
}
extern "C" fn v_scroll(this: Id, _cmd: Sel, ev: Id) {
    ev_flags(ev);
    let Some((w, x, y)) = ev_pos(this, ev) else { return };
    if !enabled(w) {
        return;
    }
    // SAFETY: the scroll event's deltas.
    let (precise, dx, dy) = unsafe {
        (msg!(ev, c"hasPreciseScrollingDeltas"; bool), msg!(ev, c"scrollingDeltaX"; f64), msg!(ev, c"scrollingDeltaY"; f64))
    };
    // 120 per notch (a line); a trackpad's points: 40 points a notch; the rest waits for the next event.
    // Up and right are positive: AppKit's deltaY > 0 scrolls up, deltaX > 0 to the left.
    let k = if precise { 3.0 } else { 120.0 };
    let (wy, wx) = with(|s| {
        s.wheel[0] += dy * k;
        s.wheel[1] -= dx * k;
        let (wy, wx) = (trunc(s.wheel[0]), trunc(s.wheel[1]));
        s.wheel[0] -= wy;
        s.wheel[1] -= wx;
        (wy, wx)
    })
    .unwrap_or((0.0, 0.0));
    if wy != 0.0 || wx != 0.0 {
        let data = (wy.clamp(-32768.0, 32767.0) as i16 as u16 as u32) | ((wx.clamp(-32768.0, 32767.0) as i16 as u16 as u32) << 16);
        push(w, E_WHEEL, 0, x, y, data);
    }
}

/// the trackpad's pinch: Ctrl + wheel, 120 per 0.2 of magnification (zoom in: up)
extern "C" fn v_magnify(this: Id, _cmd: Sel, ev: Id) {
    ev_flags(ev);
    let Some((w, x, y)) = ev_pos(this, ev) else { return };
    if !enabled(w) {
        return;
    }
    // SAFETY: the magnification of a gesture event.
    let m: f64 = unsafe { msg!(ev, c"magnification"; f64) };
    let (steps, flags) = with(|s| {
        s.zoom += m * 600.0;
        let t = trunc(s.zoom);
        s.zoom -= t;
        (t, s.flags)
    })
    .unwrap_or((0.0, 0));
    if steps != 0.0 {
        let data = steps.clamp(-32768.0, 32767.0) as i16 as u16 as u32;
        evq_push(Event { typ: E_WHEEL, btn: 0, mods: mods_of(flags) | MOD_CTRL, win: w as u8, x, y, data });
    }
}

extern "C" fn v_key_down(this: Id, _cmd: Sel, ev: Id) {
    ev_flags(ev);
    let w = view_win(this);
    if !enabled(w) {
        return;
    }
    // SAFETY: keyCode / isARepeat / modifierFlags of a key event; interpretKeyEvents: with a
    // one-element array runs the input system (insertText:, setMarkedText:, doCommandBySelector:).
    unsafe {
        let code: u16 = msg!(ev, c"keyCode"; u16);
        let repeat: bool = msg!(ev, c"isARepeat"; bool);
        let vk = vk_of(code, ev);
        if vk != 0 {
            push(w, E_KEY_DOWN, 0, 0, 0, vk | if repeat { 1 << 31 } else { 0 });
        }
        let f: u64 = msg!(ev, c"modifierFlags"; u64);
        if f & FLAG_COMMAND == 0 {
            let arr: Id = msg!(class(c"NSArray"), c"arrayWithObject:", ev => Id; Id);
            msg!(this, c"interpretKeyEvents:", arr => Id; ());
        }
    }
}
extern "C" fn v_key_up(this: Id, _cmd: Sel, ev: Id) {
    ev_flags(ev);
    // SAFETY: keyCode of a key event.
    let code: u16 = unsafe { msg!(ev, c"keyCode"; u16) };
    let vk = vk_of(code, ev);
    if vk != 0 {
        push(view_win(this), E_KEY_UP, 0, 0, 0, vk);
    }
}
extern "C" fn v_flags_changed(this: Id, _cmd: Sel, ev: Id) {
    // SAFETY: keyCode / modifierFlags of a flags-changed event.
    let (code, f) = unsafe { (msg!(ev, c"keyCode"; u16), msg!(ev, c"modifierFlags"; u64)) };
    let old = with(|s| core::mem::replace(&mut s.flags, f)).unwrap_or(0);
    let (vk, bit) = match code {
        0x38 | 0x3C => (0x10, FLAG_SHIFT),
        0x37 | 0x36 => (0x11, FLAG_COMMAND), // Command acts as Ctrl
        0x3A | 0x3D => (0x12, FLAG_OPTION),
        0x3B | 0x3E => (0x5B, FLAG_CONTROL), // Control acts as the Super key
        0x39 => (0x14, FLAG_CAPS),
        _ => return,
    };
    if f & bit != old & bit || bit == FLAG_CAPS {
        let down = f & bit != 0;
        push(view_win(this), if down { E_KEY_DOWN } else { E_KEY_UP }, 0, 0, 0, vk);
    }
}

/// a macOS key code -> Windows VK: letters and digits as the layout names them (Ctrl+Z is the key
/// labelled Z on a German keyboard too), the rest by position
fn vk_of(code: u16, ev: Id) -> u32 {
    let keypad = (0x41..=0x5C).contains(&code) && !matches!(code, 0x48 | 0x49 | 0x4A | 0x4F | 0x50 | 0x5A);
    if !keypad {
        // SAFETY: charactersIgnoringModifiers of a key event.
        let s = string_of(unsafe { msg!(ev, c"charactersIgnoringModifiers"; Id) });
        let mut cs = s.chars();
        if let (Some(c), None) = (cs.next(), cs.next()) {
            if c.is_ascii_alphabetic() {
                return c.to_ascii_uppercase() as u32;
            }
            if c.is_ascii_digit() && !(0x52..=0x5C).contains(&code) {
                return c as u32;
            }
        }
    }
    let vk = match code {
        0x00 => b'A', 0x01 => b'S', 0x02 => b'D', 0x03 => b'F', 0x04 => b'H', 0x05 => b'G', 0x06 => b'Z', 0x07 => b'X',
        0x08 => b'C', 0x09 => b'V', 0x0B => b'B', 0x0C => b'Q', 0x0D => b'W', 0x0E => b'E', 0x0F => b'R', 0x10 => b'Y',
        0x11 => b'T', 0x12 => b'1', 0x13 => b'2', 0x14 => b'3', 0x15 => b'4', 0x16 => b'6', 0x17 => b'5', 0x19 => b'9',
        0x1A => b'7', 0x1C => b'8', 0x1D => b'0', 0x1F => b'O', 0x20 => b'U', 0x22 => b'I', 0x23 => b'P', 0x25 => b'L',
        0x26 => b'J', 0x28 => b'K', 0x2D => b'N', 0x2E => b'M',
        _ => 0,
    } as u32;
    if vk != 0 {
        return vk;
    }
    match code {
        0x18 => 0xBB, // = (VK_OEM_PLUS)
        0x1B => 0xBD, // -
        0x1E => 0xDD, // ]
        0x21 => 0xDB, // [
        0x27 => 0xDE, // '
        0x29 => 0xBA, // ;
        0x2A => 0xDC, // backslash
        0x2B => 0xBC, // ,
        0x2C => 0xBF, // /
        0x2F => 0xBE, // .
        0x32 => 0xC0, // `
        0x0A => 0xE2, // the ISO key left of 1 / Z
        0x24 | 0x4C => 0x0D, // Return, keypad Enter
        0x30 => 0x09,        // Tab
        0x31 => 0x20,        // Space
        0x33 => 0x08,        // Delete (backspace)
        0x35 => 0x1B,        // Escape
        0x75 => 0x2E,        // forward delete
        0x72 => 0x2D,        // Help (Insert's place)
        0x73 => 0x24,        // Home
        0x77 => 0x23,        // End
        0x74 => 0x21,        // Page Up
        0x79 => 0x22,        // Page Down
        0x7B => 0x25,        // left
        0x7E => 0x26,        // up
        0x7C => 0x27,        // right
        0x7D => 0x28,        // down
        0x7A => 0x70, 0x78 => 0x71, 0x63 => 0x72, 0x76 => 0x73, 0x60 => 0x74, 0x61 => 0x75, 0x62 => 0x76, 0x64 => 0x77,
        0x65 => 0x78, 0x6D => 0x79, 0x67 => 0x7A, 0x6F => 0x7B, 0x69 => 0x7C, 0x6B => 0x7D, 0x71 => 0x7E, 0x6A => 0x7F,
        0x40 => 0x80, 0x4F => 0x81, 0x50 => 0x82, 0x5A => 0x83, // F1 .. F20
        0x52 => 0x60, 0x53 => 0x61, 0x54 => 0x62, 0x55 => 0x63, 0x56 => 0x64, 0x57 => 0x65, 0x58 => 0x66, 0x59 => 0x67,
        0x5B => 0x68, 0x5C => 0x69, // keypad 0 .. 9
        0x41 => 0x6E, // keypad .
        0x43 => 0x6A, // keypad *
        0x45 => 0x6B, // keypad +
        0x4E => 0x6D, // keypad -
        0x4B => 0x6F, // keypad /
        0x47 => 0x0C, // keypad clear
        0x51 => 0xBB, // keypad =
        0x4A => 0xAD, 0x49 => 0xAE, 0x48 => 0xAF, // mute, volume down, volume up
        _ => 0,
    }
}

// ---- NSTextInputClient: the input system's text, dead keys and compositions

fn utf16_to_byte(s: &str, units: usize) -> usize {
    let mut n = 0;
    for (i, c) in s.char_indices() {
        if n >= units {
            return i;
        }
        n += c.len_utf16();
    }
    s.len()
}

extern "C" fn v_insert_text(this: Id, _cmd: Sel, text: Id, _range: NSRange) {
    let w = view_win(this);
    let t = string_of(text);
    let composing = with(|s| !core::mem::take(&mut s.marked).is_empty()).unwrap_or(false);
    if composing {
        crate::ime_preedit_set(w as u8, "", 0);
        crate::ime_commit_text(w as u8, &t);
        return;
    }
    for c in t.chars() {
        let cp = c as u32;
        if cp >= 0x20 && cp != 0x7F && !(0xF700..=0xF8FF).contains(&cp) {
            push(w, E_TEXT, 0, 0, 0, cp);
        }
    }
}
extern "C" fn v_set_marked(this: Id, _cmd: Sel, text: Id, sel: NSRange, _repl: NSRange) {
    let w = view_win(this);
    let t = string_of(text);
    let cursor = utf16_to_byte(&t, sel.location);
    with(|s| s.marked = t.clone());
    crate::ime_preedit_set(w as u8, &t, cursor);
}
extern "C" fn v_unmark(this: Id, _cmd: Sel) {
    let w = view_win(this);
    let t = with(|s| core::mem::take(&mut s.marked)).unwrap_or_default();
    if !t.is_empty() {
        crate::ime_preedit_set(w as u8, "", 0);
        crate::ime_commit_text(w as u8, &t);
    }
}
extern "C" fn v_has_marked(_this: Id, _cmd: Sel) -> bool {
    with(|s| !s.marked.is_empty()).unwrap_or(false)
}
extern "C" fn v_marked_range(_this: Id, _cmd: Sel) -> NSRange {
    let n = with(|s| s.marked.encode_utf16().count()).unwrap_or(0);
    if n == 0 { NSRange { location: NOT_FOUND, length: 0 } } else { NSRange { location: 0, length: n } }
}
extern "C" fn v_selected_range(_this: Id, _cmd: Sel) -> NSRange {
    NSRange { location: 0, length: 0 }
}
extern "C" fn v_valid_attributes(_this: Id, _cmd: Sel) -> Id {
    // SAFETY: an empty (autoreleased) array.
    unsafe { msg!(class(c"NSArray"), c"array"; Id) }
}
extern "C" fn v_attributed_substring(_this: Id, _cmd: Sel, _r: NSRange, _actual: *mut NSRange) -> Id {
    NIL
}
extern "C" fn v_character_index(_this: Id, _cmd: Sel, _p: NSPoint) -> usize {
    NOT_FOUND
}
/// the caret's rectangle on the screen (points): where the candidate window goes
extern "C" fn v_first_rect(this: Id, _cmd: Sel, _r: NSRange, _actual: *mut NSRange) -> NSRect {
    let w = view_win(this);
    let Some(((win, _, scale), ime)) = ids(w).zip(rec(w, |r| r.ime)) else { return NSRect::default() };
    let local = NSRect::new(ime[0] as f64 / scale, ime[1] as f64 / scale, 1.0, ime[2].max(1) as f64 / scale);
    // SAFETY: convertRect:toView: (nil: the window) and convertRectToScreen: of our view and window.
    unsafe {
        let in_win: NSRect = msg!(this, c"convertRect:toView:", local => NSRect, NIL => Id; NSRect);
        msg!(win, c"convertRectToScreen:", in_win => NSRect; NSRect)
    }
}
extern "C" fn v_do_command(_this: Id, _cmd: Sel, _sel: Sel) {}

fn view_methods() -> Vec<(&'static core::ffi::CStr, *const c_void, &'static core::ffi::CStr)> {
    let ev = c"v@:@";
    let mut v = alloc::vec![
        (c"isFlipped", v_yes as *const c_void, c"B@:"),
        (c"acceptsFirstResponder", v_yes as *const c_void, c"B@:"),
        (c"acceptsFirstMouse:", v_yes1 as *const c_void, c"B@:@"),
        (c"needsPanelToBecomeKey", sysapp::v_needs_panel_key as *const c_void, c"B@:"),
        (c"mouseDown:", v_mouse_down as *const c_void, ev),
        (c"mouseUp:", v_mouse_up as *const c_void, ev),
        (c"rightMouseDown:", v_right_down as *const c_void, ev),
        (c"rightMouseUp:", v_right_up as *const c_void, ev),
        (c"otherMouseDown:", v_other_down as *const c_void, ev),
        (c"otherMouseUp:", v_other_up as *const c_void, ev),
        (c"mouseMoved:", v_mouse_moved as *const c_void, ev),
        (c"mouseDragged:", v_mouse_moved as *const c_void, ev),
        (c"rightMouseDragged:", v_mouse_moved as *const c_void, ev),
        (c"otherMouseDragged:", v_mouse_moved as *const c_void, ev),
        (c"mouseEntered:", v_mouse_moved as *const c_void, ev),
        (c"mouseExited:", v_mouse_exited as *const c_void, ev),
        (c"scrollWheel:", v_scroll as *const c_void, ev),
        (c"magnifyWithEvent:", v_magnify as *const c_void, ev),
        (c"keyDown:", v_key_down as *const c_void, ev),
        (c"keyUp:", v_key_up as *const c_void, ev),
        (c"flagsChanged:", v_flags_changed as *const c_void, ev),
        (c"insertText:replacementRange:", v_insert_text as *const c_void, c"v@:@{_NSRange=QQ}"),
        (c"setMarkedText:selectedRange:replacementRange:", v_set_marked as *const c_void, c"v@:@{_NSRange=QQ}{_NSRange=QQ}"),
        (c"unmarkText", v_unmark as *const c_void, c"v@:"),
        (c"hasMarkedText", v_has_marked as *const c_void, c"B@:"),
        (c"markedRange", v_marked_range as *const c_void, c"{_NSRange=QQ}@:"),
        (c"selectedRange", v_selected_range as *const c_void, c"{_NSRange=QQ}@:"),
        (c"validAttributesForMarkedText", v_valid_attributes as *const c_void, c"@@:"),
        (c"attributedSubstringForProposedRange:actualRange:", v_attributed_substring as *const c_void, c"@@:{_NSRange=QQ}^{_NSRange=QQ}"),
        (c"characterIndexForPoint:", v_character_index as *const c_void, c"Q@:{CGPoint=dd}"),
        (c"firstRectForCharacterRange:actualRange:", v_first_rect as *const c_void, c"{CGRect={CGPoint=dd}{CGSize=dd}}@:{_NSRange=QQ}^{_NSRange=QQ}"),
        (c"doCommandBySelector:", v_do_command as *const c_void, c"v@::"),
    ];
    v.extend_from_slice(&dnd::view_methods());
    v
}

// ---------------------------------------------------------------- JFWindowDelegate

fn del_win(this: Id) -> i32 {
    super::objc::ivar_get(this, DEL_OFF.load(Ordering::SeqCst)) as i32
}

extern "C" fn d_should_close(this: Id, _cmd: Sel, _sender: Id) -> bool {
    push(del_win(this), E_CLOSE, 0, 0, 0, 0);
    false
}
extern "C" fn d_did_resize(this: Id, _cmd: Sel, _n: Id) {
    let w = del_win(this);
    size_changed(w, false);
    pos_changed(w); // a resize at the top or left edge moves the client origin
    // AppKit's live resize runs its own loop: the UI draws from the refresh callback meanwhile
    // SAFETY: inLiveResize of our window.
    let live = ids(w).is_some_and(|(win, _, _)| unsafe { msg!(win, c"inLiveResize"; bool) });
    let refresh = with(|s| if s.in_refresh || !live { None } else { s.refresh }).flatten();
    if let Some(f) = refresh {
        with(|s| s.in_refresh = true);
        f();
        with(|s| s.in_refresh = false);
    }
}
extern "C" fn d_did_move(this: Id, _cmd: Sel, _n: Id) {
    pos_changed(del_win(this));
}

/// the client origin moved: E_WINPOS (through the move hook) -- not for own changes (win_set_rect),
/// whose new place is only noted
fn pos_changed(w: i32) {
    let Some(r) = win_get_rect(w) else { return };
    let Some(old) = rec(w, |x| core::mem::replace(&mut x.pos, [r[0], r[1]])) else { return };
    if old == [r[0], r[1]] || with(|s| s.own).unwrap_or(false) {
        return;
    }
    let mv = with(|s| s.mv).flatten();
    let mut rr = r;
    if let Some(f) = mv {
        f(w, &mut rr);
    }
    push(w, E_WINPOS, 0, rr[0], rr[1], 0);
}
extern "C" fn d_did_become_key(this: Id, _cmd: Sel, _n: Id) {
    push(del_win(this), E_FOCUS_IN, 0, 0, 0, 0);
}
extern "C" fn d_did_resign_key(this: Id, _cmd: Sel, _n: Id) {
    push(del_win(this), E_FOCUS_OUT, 0, 0, 0, 0);
}
extern "C" fn d_backing_changed(this: Id, _cmd: Sel, _n: Id) {
    size_changed(del_win(this), false);
}

fn delegate_methods() -> Vec<(&'static core::ffi::CStr, *const c_void, &'static core::ffi::CStr)> {
    let n = c"v@:@";
    let mut v = alloc::vec![
        (c"windowShouldClose:", d_should_close as *const c_void, c"B@:@"),
        (c"windowDidResize:", d_did_resize as *const c_void, n),
        (c"windowDidMove:", d_did_move as *const c_void, n),
        (c"windowDidBecomeKey:", d_did_become_key as *const c_void, n),
        (c"windowDidResignKey:", d_did_resign_key as *const c_void, n),
        (c"windowDidChangeBackingProperties:", d_backing_changed as *const c_void, n),
        (c"windowDidEndLiveResize:", d_did_resize as *const c_void, n),
        (c"jfThemeChanged:", d_theme_changed as *const c_void, n),
        (c"jfScreensChanged:", d_screens_changed as *const c_void, n),
    ];
    v.extend_from_slice(&sysapp::delegate_methods());
    v
}
