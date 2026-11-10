// apple_mac_test -- test aids of the macOS window backend (plattest --test window).  Events made here
// go to our own windows only ([NSWindow sendEvent:] or straight to our view's method); nothing
// reaches the system's event stream.
//
//   win_test(op, w, a) -> i64
//     WT_PIXEL [x, y]          0xRRGGBB of the frame window w shows (its layer's image), -1 none
//     WT_MOVE [x, y, w, h]     the user moves / resizes window w (its client rect, px): reported
//     WT_CURSOR_IS [shape]     1 when the application's current cursor is that shape's
//     WT_VISIBLE, WT_ACTIVE, WT_ENABLED   1 / 0 (shown, the key window, win_enable)
//     WT_POPUP_RECT            popup slot w: x | y << 16 | w << 32 | h << 48 (screen px), -1 hidden
//     WT_POPUP_PIXEL [x, y]    a pixel of popup slot w
//     WT_POPUP_SEND [kind, 0, x, y]   a mouse event (TS_MOVE / TS_DOWN / TS_UP) at popup px x, y
//     WT_CARET                 the input method's caret (firstRectForCharacterRange), client px x | y << 32
//     WT_SEND [kind, a, b, 0]  an event to window w: TS_MOVE / TS_DOWN / TS_UP at client px a, b;
//                              TS_KEY key code a with the character b (keyDown, the input system);
//                              TS_TEXT the character a (insertText:); TS_WHEEL a lines (up > 0);
//                              TS_PINCH a magnification of a / 1000 (magnifyWithEvent:, at client px b, c)
//     WT_PARENT [popup]        the index of the window w (popup = 1: popup slot w) hangs on, -1 none
//     WT_MIN_SIZE              window w's minimum client size (DIP): w | h << 32
//     WT_MENU                  the main menu: its items | the app menu's items << 8 | (1 << 16 when its
//                              Quit is Cmd+Q)
//     WT_TERMINATE             [NSApp terminate:] (Cmd+Q, the Dock's Quit) -> 0 when the app goes on
//     WT_WORKSPACE [k]         NSWorkspace's notification k (0 will sleep, 1 did wake, 2 will power off)
//                              posted in this process
//     WT_SIGTERM               SIGTERM to this process (kill)
//     WT_TRAY [k, a]           the status item: k 0 its menu rows (-1 none), 1 choose row a, 2 a click
//     WT_DOCK [k, a]           the Dock tile: k 0 the badge's number (-1 none), 1 progress shown, 2 the Dock
//                              menu's rows, 3 choose its row a
//     WT_OPEN_URLS             application:openURLs: with build/plattest_open.txt and an https URL
//     WT_NOTE_CLICK [id]       the notification centre's delegate told of a click on notification id
//     WT_BUNDLE                1 when the program runs inside an app bundle
//   file_dialog_autoclose(ms, name)   the next file_dialog is closed after ms: Cancel, or (saving)
//                              OK with name (a full path)

use alloc::string::String;

use super::{CfRef, Id, NIL, NSPoint, NSRange, NSRect, NOT_FOUND, class, ids, msg, nsstring, release, rec, round, with};
use crate::PPW_MAX;

pub const WT_PIXEL: u32 = 1;
pub const WT_MOVE: u32 = 3;
pub const WT_CURSOR_IS: u32 = 5;
pub const WT_VISIBLE: u32 = 6;
pub const WT_POPUP_RECT: u32 = 7;
pub const WT_POPUP_PIXEL: u32 = 8;
pub const WT_ENABLED: u32 = 9;
pub const WT_CARET: u32 = 10;
pub const WT_ACTIVE: u32 = 11;
pub const WT_POPUP_SEND: u32 = 13;
/// a[0]: the dpi as if the screen had that scale: -1, a test cannot change a Mac's scale
pub const WT_DPI: u32 = 4;
/// a[0] = 7 minimised, anything else back again (uigputest)
pub const WT_SHOW: u32 = 18;
/// no window animations (show, hide, resize): screen checks see the end state at once
pub const WT_NOANIM: u32 = 16;
/// whose pixel is at client point (a[0], a[1]) on the screen: 1 this window (its GPU layer is part of
/// it), 0 another one
pub const WT_FROM_POINT: u32 = 17;
pub const WT_SEND: u32 = 32;
pub const WT_PARENT: u32 = 33;
pub const WT_MIN_SIZE: u32 = 34;
pub const WT_MENU: u32 = 35;
pub const WT_TERMINATE: u32 = 36;
pub const WT_WORKSPACE: u32 = 37;
pub const WT_SIGTERM: u32 = 38;
pub const WT_TRAY: u32 = 39;
pub const WT_DOCK: u32 = 40;
pub const WT_OPEN_URLS: u32 = 41;
pub const WT_NOTE_CLICK: u32 = 42;
pub const WT_BUNDLE: u32 = 43;
// WT_SEND / WT_POPUP_SEND kinds
pub const TS_MOVE: i32 = 1;
pub const TS_DOWN: i32 = 2;
pub const TS_UP: i32 = 3;
pub const TS_KEY: i32 = 4;
pub const TS_TEXT: i32 = 5;
pub const TS_WHEEL: i32 = 6;
pub const TS_PINCH: i32 = 7;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGImageGetWidth(i: CfRef) -> usize;
    fn CGImageGetHeight(i: CfRef) -> usize;
    fn CGImageGetBytesPerRow(i: CfRef) -> usize;
    fn CGImageGetDataProvider(i: CfRef) -> CfRef;
    fn CGDataProviderCopyData(p: CfRef) -> CfRef;
    fn CGEventCreateScrollWheelEvent2(src: CfRef, units: u32, count: u32, w1: i32, w2: i32, w3: i32) -> CfRef;
}
#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    static NSWorkspaceWillSleepNotification: Id;
    static NSWorkspaceDidWakeNotification: Id;
    static NSWorkspaceWillPowerOffNotification: Id;
}
unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
    fn getpid() -> i32;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataGetBytePtr(d: CfRef) -> *const u8;
    fn CFDataGetLength(d: CfRef) -> isize;
    fn CFRelease(r: CfRef);
}

/// see the file header
pub fn win_test(op: u32, w: i32, a: [i32; 4]) -> i64 {
    let pop = || with(|s| s.pops.get(w as usize).copied()).flatten().filter(|_| (w as u32) < PPW_MAX);
    match op {
        WT_PIXEL => ids(w).map_or(-1, |(_, view, _)| layer_pixel(view, a[0], a[1])),
        WT_DPI => -1,
        WT_SHOW => {
            crate::win_cmd(w, if a[0] == 7 { crate::WCMD_MINIMIZE } else { crate::WCMD_RESTORE });
            1
        }
        WT_NOANIM => {
            let Some((win, _, _)) = ids(w) else { return -1 };
            // SAFETY: setAnimationBehavior: (NSWindowAnimationBehaviorNone = 2) of our window.
            unsafe { msg!(win, c"setAnimationBehavior:", 2isize => isize; ()) };
            1
        }
        WT_FROM_POINT => {
            let Some((win, view, scale)) = ids(w) else { return -1 };
            // SAFETY: convertPoint:toView: / convertPointToScreen: / windowNumberAtPoint: of our window and view.
            unsafe {
                let pv = NSPoint { x: a[0] as f64 / scale, y: a[1] as f64 / scale };
                let pw: NSPoint = msg!(view, c"convertPoint:toView:", pv => NSPoint, NIL => Id; NSPoint);
                let ps: NSPoint = msg!(win, c"convertPointToScreen:", pw => NSPoint; NSPoint);
                let at: isize = msg!(class(c"NSWindow"), c"windowNumberAtPoint:belowWindowWithWindowNumber:", ps => NSPoint, 0isize => isize; isize);
                let me: isize = msg!(win, c"windowNumber"; isize);
                (at == me) as i64
            }
        }
        WT_MOVE => {
            let Some((win, _, scale)) = ids(w) else { return -1 };
            let (pw, ph) = (a[2].max(1) as f64 / scale, a[3].max(1) as f64 / scale);
            let content = NSRect::new(a[0] as f64 / scale, super::primary_height() - a[1] as f64 / scale - ph, pw, ph);
            // SAFETY: frameRectForContentRect: / setFrame:display: of our window (as a drag would).
            unsafe {
                let frame: NSRect = msg!(win, c"frameRectForContentRect:", content => NSRect; NSRect);
                msg!(win, c"setFrame:display:", frame => NSRect, true => bool; ());
            }
            0
        }
        WT_CURSOR_IS => {
            // SAFETY: NSCursor's current cursor (an object we compare by identity).
            let cur: Id = unsafe { msg!(class(c"NSCursor"), c"currentCursor"; Id) };
            (cur == super::cursor_obj(a[0] as u32) && !cur.is_null()) as i64
        }
        // SAFETY (the three): state queries of our window.
        WT_VISIBLE => ids(w).map_or(-1, |(win, _, _)| unsafe { msg!(win, c"isVisible"; bool) } as i64),
        WT_ACTIVE => ids(w).map_or(-1, |(win, _, _)| unsafe { msg!(win, c"isKeyWindow"; bool) } as i64),
        WT_ENABLED => rec(w, |r| r.enabled as i64).unwrap_or(-1),
        WT_POPUP_RECT => {
            let Some(p) = pop().filter(|p| p.owner >= 0 && !p.win.is_null()) else { return -1 };
            let scale = rec(p.owner, |r| r.scale).unwrap_or(1.0);
            // SAFETY: frame / isVisible of our panel.
            let (vis, f) = unsafe { (msg!(p.win, c"isVisible"; bool), msg!(p.win, c"frame"; NSRect)) };
            if !vis {
                return -1;
            }
            let r = super::to_px(f, scale);
            (r[0] as u16 as i64) | (r[1] as u16 as i64) << 16 | (r[2] as i64) << 32 | (r[3] as i64) << 48
        }
        WT_POPUP_PIXEL => pop().filter(|p| p.owner >= 0).map_or(-1, |p| layer_pixel(p.view, a[0], a[1])),
        WT_POPUP_SEND => {
            let Some(p) = pop().filter(|p| p.owner >= 0 && !p.view.is_null()) else { return -1 };
            let scale = rec(p.owner, |r| r.scale).unwrap_or(1.0);
            mouse(p.win, p.view, a[0], a[2], a[3], scale)
        }
        WT_CARET => {
            let Some((win, view, scale)) = ids(w) else { return -1 };
            // SAFETY: our view's NSTextInputClient method (screen points) and the window's content rect.
            let (r, content) = unsafe {
                let mut actual = NSRange::default();
                let r: NSRect = msg!(view, c"firstRectForCharacterRange:actualRange:", NSRange { location: 0, length: 0 } => NSRange,
                                     &mut actual as *mut NSRange => *mut NSRange; NSRect);
                let fr: NSRect = msg!(win, c"frame"; NSRect);
                (r, msg!(win, c"contentRectForFrameRect:", fr => NSRect; NSRect))
            };
            let x = round((r.origin.x - content.origin.x) * scale) as i64;
            let y = round((content.origin.y + content.size.h - (r.origin.y + r.size.h)) * scale) as i64;
            x | y << 32
        }
        WT_SEND => {
            let Some((win, view, scale)) = ids(w) else { return -1 };
            match a[0] {
                TS_MOVE | TS_DOWN | TS_UP => mouse(win, view, a[0], a[1], a[2], scale),
                TS_KEY => key(win, a[1] as u16, a[2] as u32),
                TS_TEXT => {
                    let t = char::from_u32(a[1] as u32).map(String::from).unwrap_or_default();
                    let ns = nsstring(&t);
                    // SAFETY: our view's insertText:replacementRange: with a string we release after.
                    unsafe { msg!(view, c"insertText:replacementRange:", ns => Id, NSRange { location: NOT_FOUND, length: 0 } => NSRange; ()) };
                    release(ns);
                    0
                }
                TS_PINCH => pinch(win, view, a[1] as f64 / 1000.0, a[2], a[3], scale),
                TS_WHEEL => {
                    // SAFETY: a scroll event of our own (lines), handed to our view, then released.
                    unsafe {
                        let cg = CGEventCreateScrollWheelEvent2(core::ptr::null(), 1, 1, a[1], 0, 0);
                        if cg.is_null() {
                            return -1;
                        }
                        let ev: Id = msg!(class(c"NSEvent"), c"eventWithCGEvent:", cg => CfRef; Id);
                        if !ev.is_null() {
                            msg!(view, c"scrollWheel:", ev => Id; ());
                        }
                        CFRelease(cg);
                    }
                    0
                }
                _ => -1,
            }
        }
        WT_PARENT => {
            let win = if a[0] == 1 { pop().map_or(NIL, |p| p.win) } else { ids(w).map_or(NIL, |x| x.0) };
            if win.is_null() {
                return -1;
            }
            // SAFETY: parentWindow of our window / panel.
            let parent: Id = unsafe { msg!(win, c"parentWindow"; Id) };
            with(|s| s.wins.iter().position(|r| r.as_ref().is_some_and(|r| r.win == parent && !parent.is_null())))
                .flatten()
                .map_or(-1, |k| k as i64)
        }
        WT_MENU => menu_shape(),
        WT_TRAY => super::services::test_tray(a[0], a[1]),
        WT_NOTE_CLICK => super::services::test_note_click(a[0] as u32),
        WT_BUNDLE => super::services::test_bundled(),
        WT_DOCK => super::services::test_dock(a[0], a[1]),
        WT_OPEN_URLS => super::services::test_open(&alloc::format!("{}/build/plattest_open.txt", crate::cwd())),
        WT_TERMINATE => {
            let Some(app) = super::app() else { return -1 };
            // SAFETY: terminate: of the application (its delegate decides).
            unsafe { msg!(app, c"terminate:", NIL => Id; ()) };
            0
        }
        WT_WORKSPACE => {
            // SAFETY: NSWorkspace's centre; a notification of ours in this process.
            unsafe {
                let name = match a[0] {
                    0 => NSWorkspaceWillSleepNotification,
                    1 => NSWorkspaceDidWakeNotification,
                    _ => NSWorkspaceWillPowerOffNotification,
                };
                let ws: Id = msg!(class(c"NSWorkspace"), c"sharedWorkspace"; Id);
                let nc: Id = msg!(ws, c"notificationCenter"; Id);
                msg!(nc, c"postNotificationName:object:", name => Id, ws => Id; ());
            }
            0
        }
        // SAFETY: a signal to this process (the backend's dispatch source takes it).
        WT_SIGTERM => unsafe { kill(getpid(), 15) as i64 },
        WT_MIN_SIZE => {
            let Some((win, _, _)) = ids(w) else { return -1 };
            // SAFETY: contentMinSize of our window (points = DIP).
            let m: super::NSSize = unsafe { msg!(win, c"contentMinSize"; super::NSSize) };
            round(m.w) as i64 | (round(m.h) as i64) << 32
        }
        _ => -1,
    }
}

/// WT_MENU: the main menu's shape
fn menu_shape() -> i64 {
    let Some(app) = super::app() else { return -1 };
    // SAFETY: the main menu and its items (objects of the application).
    unsafe {
        let bar: Id = msg!(app, c"mainMenu"; Id);
        if bar.is_null() {
            return -1;
        }
        let n: isize = msg!(bar, c"numberOfItems"; isize);
        let first: Id = if n > 0 { msg!(bar, c"itemAtIndex:", 0isize => isize; Id) } else { NIL };
        let sub: Id = if first.is_null() { NIL } else { msg!(first, c"submenu"; Id) };
        let m: isize = if sub.is_null() { 0 } else { msg!(sub, c"numberOfItems"; isize) };
        let mut quit = 0i64;
        for k in 0..m {
            let it: Id = msg!(sub, c"itemAtIndex:", k => isize; Id);
            let act: super::Sel = msg!(it, c"action"; super::Sel);
            if act == crate::apple::objc::sel(c"terminate:") {
                let key = super::string_of(msg!(it, c"keyEquivalent"; Id));
                let mask: u64 = msg!(it, c"keyEquivalentModifierMask"; u64);
                quit = (key == "q" && mask & (1 << 20) != 0) as i64;
            }
        }
        n as i64 | (m as i64) << 8 | quit << 16
    }
}

/// the next file_dialog is closed after ms: Cancel, or OK with name (a full path; saving only)
pub fn file_dialog_autoclose(ms: u32, name: Option<&str>) {
    if super::init() {
        with(|s| s.autoclose = Some((ms, name.map(String::from))));
    }
}

/// 0xRRGGBB at x, y of the image a view's layer shows (rows of B, G, R, x), -1 none
fn layer_pixel(view: Id, x: i32, y: i32) -> i64 {
    if view.is_null() {
        return -1;
    }
    // SAFETY: the layer's contents is a CGImage of ours (win_present); its pixels are copied out and
    // the copy released.
    unsafe {
        let layer: Id = msg!(view, c"layer"; Id);
        if layer.is_null() {
            return -1;
        }
        let img = msg!(layer, c"contents"; Id) as CfRef;
        if img.is_null() {
            return -1;
        }
        let (w, h, stride) = (CGImageGetWidth(img), CGImageGetHeight(img), CGImageGetBytesPerRow(img));
        if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
            return -1;
        }
        let data = CGDataProviderCopyData(CGImageGetDataProvider(img));
        if data.is_null() {
            return -1;
        }
        let o = y as usize * stride + x as usize * 4;
        let v = if (CFDataGetLength(data) as usize) < o + 4 {
            -1
        } else {
            let p = CFDataGetBytePtr(data).add(o);
            (*p.add(2) as i64) << 16 | (*p.add(1) as i64) << 8 | *p as i64
        };
        CFRelease(data);
        v
    }
}

/// a left-button mouse event at view px x, y: down / up through the window's sendEvent: (it finds
/// the view under the point), a move straight to the view (AppKit routes moves by tracking area)
fn mouse(win: Id, view: Id, kind: i32, x: i32, y: i32, scale: f64) -> i64 {
    let typ: u64 = match kind {
        TS_MOVE => 5, // NSEventTypeMouseMoved
        TS_DOWN => 1,
        TS_UP => 2,
        _ => return -1,
    };
    // SAFETY: an event of our own for our window, made and sent on the main thread.
    unsafe {
        let p: NSPoint = msg!(view, c"convertPoint:toView:", NSPoint { x: x as f64 / scale, y: y as f64 / scale } => NSPoint, NIL => Id; NSPoint);
        let num: isize = msg!(win, c"windowNumber"; isize);
        let ev: Id = msg!(class(c"NSEvent"), c"mouseEventWithType:location:modifierFlags:timestamp:windowNumber:context:eventNumber:clickCount:pressure:",
                          typ => u64, p => NSPoint, 0u64 => u64, 0.0f64 => f64, num => isize, NIL => Id, 0isize => isize,
                          (kind != TS_MOVE) as isize => isize, if kind == TS_DOWN { 1.0f32 } else { 0.0f32 } => f32; Id);
        if ev.is_null() {
            return -1;
        }
        if kind == TS_MOVE {
            msg!(view, c"mouseMoved:", ev => Id; ());
        } else {
            msg!(win, c"sendEvent:", ev => Id; ());
        }
    }
    0
}

/// a magnify gesture at client px x, y: our view's magnifyWithEvent: with a stand-in event that answers
/// what the method reads (magnification, locationInWindow, modifierFlags)
fn pinch(_win: Id, view: Id, m: f64, x: i32, y: i32, scale: f64) -> i64 {
    let (cls, _) = super::subclass(c"NSObject", c"JFTestMagnify", &[
        (c"magnification", tm_magnification as *const core::ffi::c_void, c"d@:"),
        (c"locationInWindow", tm_location as *const core::ffi::c_void, c"{CGPoint=dd}@:"),
        (c"modifierFlags", tm_flags as *const core::ffi::c_void, c"Q@:"),
    ], &[]);
    // SAFETY: our view's method with a stand-in object of ours (released after).
    unsafe {
        let p: NSPoint = msg!(view, c"convertPoint:toView:", NSPoint { x: x as f64 / scale, y: y as f64 / scale } => NSPoint, NIL => Id; NSPoint);
        *PINCH.0.get() = (m, p);
        let ev: Id = msg!(cls, c"new"; Id);
        msg!(view, c"magnifyWithEvent:", ev => Id; ());
        release(ev);
    }
    0
}

struct Pinch(core::cell::UnsafeCell<(f64, NSPoint)>);
// SAFETY: the main thread only (the test aid and the view's method it calls).
unsafe impl Sync for Pinch {}
static PINCH: Pinch = Pinch(core::cell::UnsafeCell::new((0.0, NSPoint { x: 0.0, y: 0.0 })));

extern "C" fn tm_magnification(_this: Id, _cmd: super::Sel) -> f64 {
    // SAFETY: main thread (see PINCH).
    unsafe { (*PINCH.0.get()).0 }
}
extern "C" fn tm_location(_this: Id, _cmd: super::Sel) -> NSPoint {
    // SAFETY: main thread (see PINCH).
    unsafe { (*PINCH.0.get()).1 }
}
extern "C" fn tm_flags(_this: Id, _cmd: super::Sel) -> u64 {
    0
}

/// a key-down of key code `code` typing the character ch, through the window (its first responder:
/// our view, the input system)
fn key(win: Id, code: u16, ch: u32) -> i64 {
    let t = char::from_u32(ch).map(String::from).unwrap_or_default();
    let ns = nsstring(&t);
    // SAFETY: an event of our own for our window, made and sent on the main thread.
    let r = unsafe {
        let num: isize = msg!(win, c"windowNumber"; isize);
        let ev: Id = msg!(class(c"NSEvent"), c"keyEventWithType:location:modifierFlags:timestamp:windowNumber:context:characters:charactersIgnoringModifiers:isARepeat:keyCode:",
                          10u64 => u64, NSPoint::default() => NSPoint, 0u64 => u64, 0.0f64 => f64, num => isize, NIL => Id, ns => Id, ns => Id,
                          false => bool, code => u16; Id);
        if ev.is_null() {
            -1
        } else {
            msg!(win, c"sendEvent:", ev => Id; ());
            0
        }
    };
    release(ns);
    r
}
