// apple_ios_share -- (share.rs) iOS's share sheet: UIActivityViewController over the scene's window
//
//   share(&Share) -> SHARE_PENDING / SHARE_NONE   the sheet, presented by the topmost controller (as
//                                the file picker); on an iPad a popover at the anchor (points = px over
//                                the canvas's scale), without one in the view's middle and without an
//                                arrow.  SHARE_NONE without UIKit's host or off the main thread.
//   available() -> bool          (any thread) UIKit's host runs the app
//   ios_share_shown() / ios_share_dismiss()   (tests) the sheet on screen, closed again
//
// Items: the text (NSString), the link (NSURL), the files (file URLs).  A title goes in as the
// subject (Mail's subject line) through an item source (JFShareItem) wrapping the first item.  The
// completion handler (a global block: one sheet at a time) answers SHARE_DONE when an activity
// completed, SHARE_FAILED with an error, SHARE_CANCELLED otherwise (the sheet or the activity closed).

use alloc::string::String;
use core::ffi::c_void;
use core::sync::atomic::{AtomicPtr, Ordering};

use rt::sync::SpinLock;

use super::super::objc::{Id, NIL, NSPoint, NSRect, NSSize, ObjcSuper, Sel, class, ivar_get, ivar_set, msg, msg_super, nsstring,
                         objc_autoreleasePoolPop, objc_autoreleasePoolPush, release, string_of, subclass};
use super::{host, on_main};
use crate::share::{SHARE_CANCELLED, SHARE_DONE, SHARE_FAILED, SHARE_NONE, SHARE_PENDING, Share, answer};

/// the subject of the open sheet (JFShareItem's subjectForActivityType:)
static SUBJECT: SpinLock<String> = SpinLock::new(String::new());
/// the completion handler (a global block, made once)
static DONE_BLOCK: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
/// JFShareItem's class and the offset of its item
static ITEM_CLASS: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static ITEM_IVAR: core::sync::atomic::AtomicIsize = core::sync::atomic::AtomicIsize::new(0);

/// (any thread) UIKit's host runs the app
pub fn available() -> bool {
    super::hosted()
}

/// void (^)(UIActivityType, BOOL completed, NSArray *returned, NSError *error)
extern "C" fn done(_block: *const c_void, _activity: Id, completed: bool, _returned: Id, error: Id) {
    answer(if completed {
        SHARE_DONE
    } else if !error.is_null() {
        SHARE_FAILED
    } else {
        SHARE_CANCELLED
    });
}

/// JFShareItem: an item source handing out its item, with the sheet's subject
fn item_class() -> (Id, isize) {
    let c = ITEM_CLASS.load(Ordering::Acquire);
    if !c.is_null() {
        return (c, ITEM_IVAR.load(Ordering::Acquire));
    }
    type Imp = *const c_void;
    let (cls, off) = subclass(c"NSObject", c"JFShareItem", &[
        (c"activityViewControllerPlaceholderItem:", item_placeholder as Imp, c"@@:@"),
        (c"activityViewController:itemForActivityType:", item_for as Imp, c"@@:@@"),
        (c"activityViewController:subjectForActivityType:", item_subject as Imp, c"@@:@@"),
        (c"dealloc", item_dealloc as Imp, c"v@:"),
    ], &[c"UIActivityItemSource"]);
    ITEM_IVAR.store(off, Ordering::Release);
    ITEM_CLASS.store(cls, Ordering::Release);
    (cls, off)
}

fn item_of(this: Id) -> Id {
    ivar_get(this, ITEM_IVAR.load(Ordering::Acquire)) as Id
}

extern "C" fn item_placeholder(this: Id, _sel: Sel, _vc: Id) -> Id {
    item_of(this)
}

extern "C" fn item_for(this: Id, _sel: Sel, _vc: Id, _activity: Id) -> Id {
    item_of(this)
}

extern "C" fn item_subject(_this: Id, _sel: Sel, _vc: Id, _activity: Id) -> Id {
    let s = nsstring(&SUBJECT.lock());
    // SAFETY: a string of ours handed back autoreleased, as a getter's result.
    unsafe { msg!(s, c"autorelease"; Id) }
}

extern "C" fn item_dealloc(this: Id, cmd: Sel) {
    release(item_of(this));
    let sup = ObjcSuper { receiver: this, class: class(c"NSObject") };
    // SAFETY: [super dealloc] of this object (cmd is dealloc), last.
    unsafe { msg_super!(&sup, cmd; ()) };
}

/// the topmost presented controller of the window (the root's when nothing is presented)
///
/// SAFETY: win is the live window; the main thread.
unsafe fn top(win: Id) -> Id {
    // SAFETY: (caller)
    unsafe {
        let mut vc: Id = msg!(win, c"rootViewController"; Id);
        while !vc.is_null() {
            let next: Id = msg!(vc, c"presentedViewController"; Id);
            if next.is_null() {
                break;
            }
            vc = next;
        }
        vc
    }
}

pub fn share(s: &Share) -> i32 {
    let h = host();
    let (win, view) = (h.window.get(), h.view.get());
    if win.is_null() || view.is_null() || !on_main() {
        return SHARE_NONE;
    }
    *SUBJECT.lock() = s.title.clone();
    // SAFETY: Foundation / UIKit objects made here (autoreleased in the pool or released after the
    // presenting controller took them); the main thread.
    let ok = unsafe {
        let pool = objc_autoreleasePoolPush();
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
        let ok = n > 0 && {
            // the subject rides on the first item
            if !s.title.is_empty() {
                let (cls, off) = item_class();
                let first: Id = msg!(items, c"objectAtIndex:", 0usize => usize; Id);
                let src: Id = msg!(msg!(cls, c"alloc"; Id), c"init"; Id);
                if !src.is_null() {
                    msg!(first, c"retain"; Id);
                    ivar_set(src, off, first as isize);
                    msg!(items, c"replaceObjectAtIndex:withObject:", 0usize => usize, src => Id; ());
                    release(src);
                }
            }
            let avc: Id = msg!(class(c"UIActivityViewController"), c"alloc"; Id);
            let avc: Id = msg!(avc, c"initWithActivityItems:applicationActivities:", items => Id, NIL => Id; Id);
            let vc = top(win);
            if avc.is_null() || vc.is_null() {
                if !avc.is_null() {
                    release(avc);
                }
                false
            } else {
                let block = super::super::block::global(&DONE_BLOCK, done as *const c_void);
                msg!(avc, c"setCompletionWithItemsHandler:", block => *const c_void; ());
                // an iPad shows it as a popover: at the anchor, else in the view's middle
                let pop: Id = msg!(avc, c"popoverPresentationController"; Id);
                if !pop.is_null() {
                    let sc = h.scale.get().max(1.0);
                    let vb: NSRect = msg!(view, c"bounds"; NSRect);
                    let a = s.anchor;
                    let r = if a[2] > 0 && a[3] > 0 {
                        NSRect::new(a[0] as f64 / sc, a[1] as f64 / sc, a[2] as f64 / sc, a[3] as f64 / sc)
                    } else {
                        msg!(pop, c"setPermittedArrowDirections:", 0usize => usize; ());
                        NSRect { origin: NSPoint { x: vb.size.w / 2.0, y: vb.size.h / 2.0 }, size: NSSize::default() }
                    };
                    msg!(pop, c"setSourceView:", view => Id; ());
                    msg!(pop, c"setSourceRect:", r => NSRect; ());
                }
                msg!(vc, c"presentViewController:animated:completion:", avc => Id, true => bool, NIL => Id; ());
                release(avc);
                true
            }
        };
        objc_autoreleasePoolPop(pool);
        ok
    };
    if ok { SHARE_PENDING } else { SHARE_NONE }
}

/// (tests, any thread) the class of the controller presented over the window now (the share sheet:
/// "UIActivityViewController"), None when nothing is presented or without the host
pub fn ios_share_shown() -> Option<String> {
    super::a11y_ios::on_main(|| {
        let win = host().window.get();
        if win.is_null() {
            return None;
        }
        // SAFETY: the live window's controllers; the class's name copied; the main thread.
        unsafe {
            let root: Id = msg!(win, c"rootViewController"; Id);
            let vc = top(win);
            if vc.is_null() || vc == root {
                return None;
            }
            let name: Id = msg!(msg!(vc, c"class"; Id), c"description"; Id);
            Some(string_of(name))
        }
    })
}

/// (tests, any thread) the presented sheet closed as its Close button does: the controller's
/// completion handler with nothing chosen (UIKit calls it for the user's Close, not for a
/// dismissal by code), then the dismissal -> false when none
pub fn ios_share_dismiss() -> bool {
    super::a11y_ios::on_main(|| {
        let win = host().window.get();
        if win.is_null() {
            return false;
        }
        // SAFETY: the live window's controllers; the handler is a block (its invoke pointer after
        // isa, flags and reserved), called with its own signature; the main thread.
        unsafe {
            let root: Id = msg!(win, c"rootViewController"; Id);
            let vc = top(win);
            if vc.is_null() || vc == root {
                return false;
            }
            let block: *const c_void = msg!(vc, c"completionWithItemsHandler"; *const c_void);
            if !block.is_null() {
                let invoke = *(block as *const u8).add(16).cast::<*const c_void>();
                let f: extern "C" fn(*const c_void, Id, bool, Id, Id) = core::mem::transmute(invoke);
                f(block, NIL, false, NIL, NIL);
            }
            msg!(vc, c"dismissViewControllerAnimated:completion:", true => bool, NIL => Id; ());
        }
        true
    })
}
