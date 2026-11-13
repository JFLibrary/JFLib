// apple_assoc -- (assoc.rs) macOS: the default web browser through Launch Services
//
//   browser_status(pkg)   LSCopyDefaultHandlerForURLScheme of http and https is this bundle's id
//   browser_request(pkg)  LSSetDefaultHandlerForURLScheme(http, https -> the bundle's id): macOS
//                         asks the user itself ("Do you want to change your default web browser
//                         to ... or keep ...?") and sets both when the user agrees
//
// The bundle declares http and https as its URL types and the web documents it views (Info.plist
// from PACKAGE browser=1: cargo jflib package, tools/apple.py), so Launch Services lists it as a
// browser.  Both functions are deprecated since macOS 12 (NSWorkspace's
// setDefaultApplicationAtURL:toOpenURLsWithScheme: is the newer form) and still work.  A program
// outside a bundle has no id: Unknown, NotRegistered.

use super::{AssocError, BrowserStatus, Package};
use crate::apple::objc::{nsstring, release};
use core::ffi::c_void;

type CfRef = *const c_void;

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn LSCopyDefaultHandlerForURLScheme(scheme: CfRef) -> CfRef;
    fn LSSetDefaultHandlerForURLScheme(scheme: CfRef, handler: CfRef) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFBundleGetMainBundle() -> CfRef;
    fn CFBundleGetIdentifier(bundle: CfRef) -> CfRef;
    fn CFStringCompare(a: CfRef, b: CfRef, flags: usize) -> isize;
    fn CFRelease(r: CfRef);
}

/// kCFCompareCaseInsensitive (Launch Services keeps bundle ids in lower case)
const CASE_INSENSITIVE: usize = 1;

/// the main bundle's identifier (not ours to release), null outside a bundle
fn bundle_id() -> CfRef {
    // SAFETY: the main bundle and its identifier follow the get rule: nothing to release.
    unsafe {
        let b = CFBundleGetMainBundle();
        if b.is_null() { core::ptr::null() } else { CFBundleGetIdentifier(b) }
    }
}

pub fn browser_status(_: &Package) -> BrowserStatus {
    let me = bundle_id();
    if me.is_null() {
        return BrowserStatus::Unknown;
    }
    for s in ["http", "https"] {
        let scheme = nsstring(s);
        // SAFETY: an NSString of ours (toll-free a CFString); the handler is a copy (+1),
        // released after the comparison.
        let mine = unsafe {
            let h = LSCopyDefaultHandlerForURLScheme(scheme as CfRef);
            let eq = !h.is_null() && CFStringCompare(h, me, CASE_INSENSITIVE) == 0;
            if !h.is_null() {
                CFRelease(h);
            }
            eq
        };
        release(scheme);
        if !mine {
            return BrowserStatus::NotDefault;
        }
    }
    BrowserStatus::Default
}

pub fn browser_request(_: &Package) -> Result<(), AssocError> {
    let me = bundle_id();
    if me.is_null() {
        return Err(AssocError::NotRegistered);
    }
    for s in ["http", "https"] {
        let scheme = nsstring(s);
        // SAFETY: two CFStrings that live across the call (ours, the bundle's).
        let st = unsafe { LSSetDefaultHandlerForURLScheme(scheme as CfRef, me) };
        release(scheme);
        if st != 0 {
            return Err(AssocError::Failed(alloc::format!("LSSetDefaultHandlerForURLScheme({s}): {st}")));
        }
    }
    Ok(())
}
