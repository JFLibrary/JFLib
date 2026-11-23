// apple_mac_share -- (share.rs) macOS's share sheet: NSSharingServicePicker at the anchor in the key
// window (the main window when none is key)
//
//   share(&Share) -> SHARE_PENDING / SHARE_NONE   the picker shown relative to the anchor (points =
//                                px over the window's backingScaleFactor; our views are flipped, so
//                                y counts from the top), without one in the content view's middle
//   available() -> bool          true (AppKit has the picker since 10.8)
//   mac_share_shown() / mac_share_close()   (tests) the picker's window on screen, closed again
//
// Items: the text (NSString), the link (NSURL), the files (file URLs).  The picker's delegate answers:
// a service chosen -> SHARE_DONE (the service gets the title as its subject, Mail's subject line);
// none (the picker closed) -> SHARE_CANCELLED.  Whether the service then delivers is its own
// business (Android's chooser says as little).

use alloc::string::String;
use core::ffi::c_void;
use core::sync::atomic::{AtomicPtr, Ordering};

use rt::sync::SpinLock;

use super::super::objc::{Id, NIL, NSPoint, NSRect, NSSize, Sel, class, msg, nsstring, objc_autoreleasePoolPop, objc_autoreleasePoolPush,
                         release, subclass};
use crate::share::{SHARE_CANCELLED, SHARE_DONE, SHARE_NONE, SHARE_PENDING, Share, answer};

/// the subject of the open picker (set on the chosen service)
static SUBJECT: SpinLock<String> = SpinLock::new(String::new());
/// the picker's delegate (one for the process) and the open picker (retained until its answer)
static DELEGATE: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static PICKER: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

pub fn available() -> bool {
    true
}

fn delegate() -> Id {
    let d = DELEGATE.load(Ordering::Acquire);
    if !d.is_null() {
        return d;
    }
    type Imp = *const c_void;
    let (cls, _) = subclass(c"NSObject", c"JFSharePickerDelegate", &[(c"sharingServicePicker:didChooseSharingService:", chose as Imp, c"v@:@@")],
                            &[c"NSSharingServicePickerDelegate"]);
    // SAFETY: an instance of our class, kept for the process.
    let d: Id = unsafe { msg!(msg!(cls, c"alloc"; Id), c"init"; Id) };
    DELEGATE.store(d, Ordering::Release);
    d
}

extern "C" fn chose(_this: Id, _sel: Sel, _picker: Id, service: Id) {
    if !service.is_null() {
        let s = SUBJECT.lock().clone();
        if !s.is_empty() {
            let n = nsstring(&s);
            // SAFETY: the chosen service's subject (copied by it); our string released after.
            unsafe { msg!(service, c"setSubject:", n => Id; ()) };
            release(n);
        }
    }
    let p = PICKER.swap(core::ptr::null_mut(), Ordering::AcqRel);
    if !p.is_null() {
        release(p);
    }
    answer(if service.is_null() { SHARE_CANCELLED } else { SHARE_DONE });
}

/// the key window, else the main window, else the first visible one of the app
///
/// SAFETY: the main thread; AppKit's application object exists.
unsafe fn front_window() -> Id {
    // SAFETY: (caller) plain queries of NSApp.
    unsafe {
        let app: Id = msg!(class(c"NSApplication"), c"sharedApplication"; Id);
        let w: Id = msg!(app, c"keyWindow"; Id);
        if !w.is_null() {
            return w;
        }
        let w: Id = msg!(app, c"mainWindow"; Id);
        if !w.is_null() {
            return w;
        }
        let ws: Id = msg!(app, c"orderedWindows"; Id);
        let n: usize = if ws.is_null() { 0 } else { msg!(ws, c"count"; usize) };
        for i in 0..n {
            let w: Id = msg!(ws, c"objectAtIndex:", i => usize; Id);
            if msg!(w, c"isVisible"; bool) {
                return w;
            }
        }
        NIL
    }
}

pub fn share(s: &Share) -> i32 {
    if !super::app_init() || !PICKER.load(Ordering::Acquire).is_null() {
        return SHARE_NONE;
    }
    *SUBJECT.lock() = s.title.clone();
    // SAFETY: Foundation / AppKit objects made here (autoreleased in the pool; the picker retained
    // until its answer); the main thread (app_init).
    let ok = unsafe {
        let pool = objc_autoreleasePoolPush();
        let win = front_window();
        let view: Id = if win.is_null() { NIL } else { msg!(win, c"contentView"; Id) };
        let items: Id = msg!(class(c"NSMutableArray"), c"array"; Id);
        let add = |obj: Id| {
            if !obj.is_null() {
                msg!(items, c"addObject:", obj => Id; ());
            }
        };
        if !s.text.is_empty() {
            let t = nsstring(&s.text);
            add(t);
            release(t);
        }
        if !s.url.is_empty() {
            let u = nsstring(&s.url);
            add(msg!(class(c"NSURL"), c"URLWithString:", u => Id; Id));
            release(u);
        }
        for f in &s.files {
            let p = nsstring(f);
            add(msg!(class(c"NSURL"), c"fileURLWithPath:", p => Id; Id));
            release(p);
        }
        let n: usize = msg!(items, c"count"; usize);
        let ok = !view.is_null() && n > 0 && {
            let picker: Id = msg!(class(c"NSSharingServicePicker"), c"alloc"; Id);
            let picker: Id = msg!(picker, c"initWithItems:", items => Id; Id);
            if picker.is_null() {
                false
            } else {
                msg!(picker, c"setDelegate:", delegate() => Id; ());
                PICKER.store(picker, Ordering::Release);
                let sc: f64 = msg!(win, c"backingScaleFactor"; f64);
                let sc = if sc > 0.0 { sc } else { 1.0 };
                let b: NSRect = msg!(view, c"bounds"; NSRect);
                let a = s.anchor;
                let r = if a[2] > 0 && a[3] > 0 {
                    // (the content view is flipped when it is ours: y from the top; else from the bottom)
                    let flipped: bool = msg!(view, c"isFlipped"; bool);
                    let (x, w, h) = (a[0] as f64 / sc, a[2] as f64 / sc, a[3] as f64 / sc);
                    let y = if flipped { a[1] as f64 / sc } else { b.size.h - (a[1] + a[3]) as f64 / sc };
                    NSRect::new(x, y, w, h)
                } else {
                    NSRect { origin: NSPoint { x: b.size.w / 2.0, y: b.size.h / 2.0 }, size: NSSize { w: 1.0, h: 1.0 } }
                };
                // NSMinYEdge 1: below the anchor in a flipped view
                msg!(picker, c"showRelativeToRect:ofView:preferredEdge:", r => NSRect, view => Id, 1usize => usize; ());
                true
            }
        };
        objc_autoreleasePoolPop(pool);
        ok
    };
    if ok { SHARE_PENDING } else { SHARE_NONE }
}

/// (tests) the picker waits for the user (its answer not given yet)
pub fn mac_share_shown() -> bool {
    !PICKER.load(Ordering::Acquire).is_null()
}

/// (tests, any thread) the open picker closed without a service, as the user's click outside it would
/// (NSSharingServicePicker close, macOS 13; before that the delegate's answer is sent directly)
pub fn mac_share_close() -> bool {
    super::a11y_mac::on_main(|| {
        let p = PICKER.load(Ordering::Acquire);
        if p.is_null() {
            return false;
        }
        // SAFETY: the open picker (retained until its answer); the main thread.
        unsafe {
            if msg!(p, c"respondsToSelector:", super::super::objc::sel(c"close") => Sel; bool) {
                msg!(p, c"close"; ());
            }
        }
        // the delegate answered when closing worked; else as if it had
        if !PICKER.load(Ordering::Acquire).is_null() {
            chose(NIL, core::ptr::null(), p, NIL);
        }
        true
    })
}
