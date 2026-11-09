// apple_recent -- (recent.rs) macOS: the app's list of recent documents (NSDocumentController), the
// one the Dock's menu of the app shows
//
//   target() -> Native
//   add(path)      noteNewRecentDocumentURL:
//   list(max)      recentDocumentURLs (newest first): their paths
//   remove(path)   the list without it: clearRecentDocuments:, then the others noted again, oldest first
//   clear()        clearRecentDocuments:
//
// The list belongs to the app (its bundle id; a program outside a bundle gets one named after it).
// AppKit's objects: the UI thread only.

use alloc::string::String;
use alloc::vec::Vec;

use super::Target;
use crate::apple::objc::{Id, NIL, class, msg, nsstring, objc_autoreleasePoolPop, objc_autoreleasePoolPush, release, string_of};

pub fn target() -> Target {
    Target::Native
}

fn controller() -> Id {
    if !crate::apple::mac::app_init() {
        return NIL;
    }
    // SAFETY: the shared document controller (a shared object).
    unsafe { msg!(class(c"NSDocumentController"), c"sharedDocumentController"; Id) }
}

fn note(dc: Id, path: &str) -> bool {
    let s = nsstring(path);
    // SAFETY: a file URL of a string of ours (released after), noted by the controller.
    unsafe {
        let url: Id = msg!(class(c"NSURL"), c"fileURLWithPath:", s => Id; Id);
        release(s);
        if url.is_null() {
            return false;
        }
        msg!(dc, c"noteNewRecentDocumentURL:", url => Id; ());
    }
    true
}

pub fn add(path: &str) -> bool {
    let dc = controller();
    if dc.is_null() {
        return false;
    }
    // SAFETY: a pool of ours around the controller's call.
    let pool = unsafe { objc_autoreleasePoolPush() };
    let ok = note(dc, path);
    // SAFETY: the pool pushed above.
    unsafe { objc_autoreleasePoolPop(pool) };
    ok
}

pub fn list(max: usize) -> Vec<String> {
    let dc = controller();
    let mut out = Vec::new();
    if dc.is_null() {
        return out;
    }
    // SAFETY: a pool of ours; recentDocumentURLs is an array of NSURLs.
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let arr: Id = msg!(dc, c"recentDocumentURLs"; Id);
        let n: usize = if arr.is_null() { 0 } else { msg!(arr, c"count"; usize) };
        for k in 0..n {
            if out.len() >= max {
                break;
            }
            let u: Id = msg!(arr, c"objectAtIndex:", k => usize; Id);
            if msg!(u, c"isFileURL"; bool) {
                let p = string_of(msg!(u, c"path"; Id));
                if !p.is_empty() {
                    out.push(p);
                }
            }
        }
        objc_autoreleasePoolPop(pool);
    }
    out
}

pub fn remove(path: &str) -> bool {
    let all = list(usize::MAX);
    if !all.iter().any(|p| p == path) {
        return false;
    }
    if !clear() {
        return false;
    }
    let dc = controller();
    // SAFETY: a pool of ours around the controller's calls.
    let pool = unsafe { objc_autoreleasePoolPush() };
    for p in all.iter().rev().filter(|p| *p != path) {
        note(dc, p);
    }
    // SAFETY: the pool pushed above.
    unsafe { objc_autoreleasePoolPop(pool) };
    true
}

pub fn clear() -> bool {
    let dc = controller();
    if dc.is_null() {
        return false;
    }
    // SAFETY: the controller's method (the sender may be nil).
    unsafe { msg!(dc, c"clearRecentDocuments:", NIL => Id; ()) };
    true
}
