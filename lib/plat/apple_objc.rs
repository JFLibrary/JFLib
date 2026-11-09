// apple_objc -- the Objective-C runtime from Rust (docs/APPLE.md, decision 3): classes, selectors,
// typed objc_msgSend calls, classes of our own with methods and an instance variable, NSString.
//
//   class("NSWindow") -> Id        sel("setTitle:") -> Sel (NUL-terminated names: c"..")
//   msg!(obj, c"sel:", a => T, ..; R)  objc_msgSend cast to exactly fn(Id, Sel, T..) -> R: on Apple's
//                                  arm64 a variadic call would put the arguments on the stack, so
//                                  every call has its own exact signature (docs/APPLE.md, Fallen)
//   subclass(super, name, methods, ivar) -> (class, ivar offset)   a class of ours: an isize
//                                  instance variable (the window's index), the methods' IMPs
//   ivar_get / ivar_set            that variable of an object
//   nsstring(s) -> Id (+1)         string_of(nsstring) -> String
//
// Everything here runs on the main thread (AppKit's rule); nothing is retained across threads.

use alloc::string::String;
use core::ffi::{CStr, c_char, c_void};

pub type Id = *mut c_void;
pub type Sel = *const c_void;
pub const NIL: Id = core::ptr::null_mut();

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct NSPoint {
    pub x: f64,
    pub y: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct NSSize {
    pub w: f64,
    pub h: f64,
}
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct NSRect {
    pub origin: NSPoint,
    pub size: NSSize,
}
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct NSRange {
    pub location: usize,
    pub length: usize,
}
/// NSNotFound
pub const NOT_FOUND: usize = isize::MAX as usize;

impl NSRect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> NSRect {
        NSRect { origin: NSPoint { x, y }, size: NSSize { w, h } }
    }
}

#[link(name = "objc")]
unsafe extern "C" {
    pub fn objc_msgSend();
    pub fn objc_getClass(name: *const c_char) -> Id;
    pub fn objc_getProtocol(name: *const c_char) -> Id;
    pub fn sel_registerName(name: *const c_char) -> Sel;
    pub fn objc_allocateClassPair(superclass: Id, name: *const c_char, extra: usize) -> Id;
    pub fn objc_registerClassPair(cls: Id);
    pub fn class_addMethod(cls: Id, sel: Sel, imp: *const c_void, types: *const c_char) -> bool;
    pub fn class_addProtocol(cls: Id, protocol: Id) -> bool;
    pub fn class_addIvar(cls: Id, name: *const c_char, size: usize, align: u8, types: *const c_char) -> bool;
    pub fn class_getInstanceVariable(cls: Id, name: *const c_char) -> *const c_void;
    pub fn ivar_getOffset(ivar: *const c_void) -> isize;
    pub fn objc_autoreleasePoolPush() -> *mut c_void;
    pub fn objc_autoreleasePoolPop(pool: *mut c_void);
}
// the frameworks the backend calls into (their classes are found through objc_getClass)
#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {}

/// the class of that name (nil when there is none)
pub fn class(name: &CStr) -> Id {
    // SAFETY: a NUL-terminated name.
    unsafe { objc_getClass(name.as_ptr()) }
}

/// the selector of that name
pub fn sel(name: &CStr) -> Sel {
    // SAFETY: a NUL-terminated name; selectors live for the process.
    unsafe { sel_registerName(name.as_ptr()) }
}

/// objc_msgSend with the exact signature of the call: msg!(obj, c"sel:", a => T, b => U; R)
macro_rules! objc_msg {
    ($obj:expr, $sel:expr $(, $a:expr => $t:ty)* ; $r:ty) => {{
        let f: unsafe extern "C" fn($crate::apple::objc::Id, $crate::apple::objc::Sel $(, $t)*) -> $r =
            core::mem::transmute($crate::apple::objc::objc_msgSend as *const ());
        f($obj as $crate::apple::objc::Id, $crate::apple::objc::sel($sel) $(, $a)*)
    }};
}
pub(crate) use objc_msg as msg;

/// a class of ours: super, name, the methods (selector, IMP, type encoding), one isize instance
/// variable "jf" -> (class, offset of the variable).  An existing class of that name is reused.
pub fn subclass(superclass: &CStr, name: &CStr, methods: &[(&CStr, *const c_void, &CStr)], protocols: &[&CStr]) -> (Id, isize) {
    // SAFETY: the runtime's own calls on a class pair made here and registered once.
    unsafe {
        let mut cls = objc_allocateClassPair(class(superclass), name.as_ptr(), 0);
        if cls.is_null() {
            cls = class(name); // registered before (a second backend start in one process)
        } else {
            class_addIvar(cls, c"jf".as_ptr(), 8, 3, c"q".as_ptr());
            for (s, imp, types) in methods {
                class_addMethod(cls, sel(s), *imp, types.as_ptr());
            }
            for p in protocols {
                let proto = objc_getProtocol(p.as_ptr());
                if !proto.is_null() {
                    class_addProtocol(cls, proto);
                }
            }
            objc_registerClassPair(cls);
        }
        let iv = class_getInstanceVariable(cls, c"jf".as_ptr());
        (cls, if iv.is_null() { 0 } else { ivar_getOffset(iv) })
    }
}

/// the instance variable "jf" of an object of one of our classes
pub fn ivar_get(obj: Id, off: isize) -> isize {
    if obj.is_null() || off == 0 {
        return -1;
    }
    // SAFETY: obj is an instance of a class made by subclass, which has the isize at off.
    unsafe { *((obj as *mut u8).offset(off) as *const isize) }
}

pub fn ivar_set(obj: Id, off: isize, v: isize) {
    if obj.is_null() || off == 0 {
        return;
    }
    // SAFETY: as in ivar_get.
    unsafe { *((obj as *mut u8).offset(off) as *mut isize) = v };
}

/// an NSString of s, retained (+1): release it when done
pub fn nsstring(s: &str) -> Id {
    // SAFETY: alloc / initWithBytes:length:encoding: (NSUTF8StringEncoding = 4) copies the bytes.
    unsafe {
        let o: Id = msg!(class(c"NSString"), c"alloc"; Id);
        msg!(o, c"initWithBytes:length:encoding:", s.as_ptr() => *const u8, s.len() => usize, 4usize => usize; Id)
    }
}

/// the text of an NSString (or of an NSAttributedString's string)
pub fn string_of(obj: Id) -> String {
    if obj.is_null() {
        return String::new();
    }
    // SAFETY: isKindOfClass: / string / UTF8String on a live object; the C string lives as long
    // as the autorelease pool, it is copied at once.
    unsafe {
        let s = if msg!(obj, c"isKindOfClass:", class(c"NSAttributedString") => Id; bool) { msg!(obj, c"string"; Id) } else { obj };
        let p: *const c_char = msg!(s, c"UTF8String"; *const c_char);
        if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
    }
}

/// release an object we own (+1)
pub fn release(obj: Id) {
    if !obj.is_null() {
        // SAFETY: the caller owns one reference.
        unsafe { msg!(obj, c"release"; ()) };
    }
}
