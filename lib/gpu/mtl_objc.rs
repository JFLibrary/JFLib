// mtl_objc -- the Objective-C runtime and the frameworks the Metal backend calls (AP6, docs/APPLE.md):
// classes, selectors, objc_msgSend cast to the exact signature of each call (on Apple's arm64 a
// variadic call would put the arguments on the stack), owned objects that release themselves.
//
//   class(c"MTLRenderPassDescriptor") -> Id     sel(c"newBufferWithLength:options:") -> Sel
//   msg!(obj, c"sel:", a => T, ..; R)           one call with that exact signature
//   Obj                                         an object we own (+1): Clone retains, Drop releases
//   Obj::new(id) / Obj::retained(id)            take a +1 object (new..., alloc/init, copy) / retain
//                                               an autoreleased one
//   Pool                                        an autorelease pool for a scope (calls that return
//                                               autoreleased objects run inside one)
//   nsstring(s) -> Obj, string_of(id) -> String, error_text(NSError) -> String
//
// Metal's devices, queues, resources and pipeline states may be used from any thread (Obj is
// Send and Sync); command buffers and encoders stay with the thread that records them.  Kept
// apart from plat's own binding (apple_objc.rs) so plat's API does not grow for the GPU.

use alloc::string::String;
use core::ffi::{CStr, c_char, c_void};

pub type Id = *mut c_void;
pub type Sel = *const c_void;
pub const NIL: Id = core::ptr::null_mut();

#[link(name = "objc")]
unsafe extern "C" {
    pub fn objc_msgSend();
    pub fn objc_getClass(name: *const c_char) -> Id;
    pub fn sel_registerName(name: *const c_char) -> Sel;
    pub fn objc_autoreleasePoolPush() -> *mut c_void;
    pub fn objc_autoreleasePoolPop(pool: *mut c_void);
}
#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {}
#[link(name = "Metal", kind = "framework")]
unsafe extern "C" {
    pub fn MTLCreateSystemDefaultDevice() -> Id;
    #[cfg(target_os = "macos")]
    pub fn MTLCopyAllDevices() -> Id;
}
// CAMetalLayer (mtl_swap.rs)
#[link(name = "QuartzCore", kind = "framework")]
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
        let f: unsafe extern "C" fn($crate::mtl_objc::Id, $crate::mtl_objc::Sel $(, $t)*) -> $r =
            core::mem::transmute($crate::mtl_objc::objc_msgSend as *const ());
        f($obj as $crate::mtl_objc::Id, $crate::mtl_objc::sel($sel) $(, $a)*)
    }};
}
pub(crate) use objc_msg as msg;

/// an object we hold one reference to
pub struct Obj(Id);

// SAFETY: the objects kept in Obj are Metal devices, queues, resources, states and descriptors,
// which Metal allows on any thread; encoders and command buffers are only used by the thread
// that owns the encoder holding them (CommandEncoder is not Send).
unsafe impl Send for Obj {}
// SAFETY: as above.
unsafe impl Sync for Obj {}

impl Obj {
    /// take ownership of a +1 reference (None for nil)
    pub fn new(id: Id) -> Option<Obj> {
        if id.is_null() { None } else { Some(Obj(id)) }
    }
    /// retain an object we do not own (an autoreleased result)
    pub fn retained(id: Id) -> Option<Obj> {
        if id.is_null() {
            return None;
        }
        // SAFETY: a live object (the caller holds it, its pool is still there).
        unsafe { msg!(id, c"retain"; Id) };
        Some(Obj(id))
    }
    pub fn id(&self) -> Id {
        self.0
    }
}

impl Clone for Obj {
    fn clone(&self) -> Obj {
        // SAFETY: we hold a reference, so the object is alive.
        unsafe { msg!(self.0, c"retain"; Id) };
        Obj(self.0)
    }
}

impl Drop for Obj {
    fn drop(&mut self) {
        // SAFETY: the reference this Obj owns.
        unsafe { msg!(self.0, c"release"; ()) };
    }
}

/// an autorelease pool for a scope
pub struct Pool(*mut c_void);

impl Pool {
    pub fn new() -> Pool {
        // SAFETY: a plain push, popped by Drop on the same thread.
        Pool(unsafe { objc_autoreleasePoolPush() })
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        // SAFETY: the pool pushed by new, on this thread, innermost (scoped).
        unsafe { objc_autoreleasePoolPop(self.0) };
    }
}

/// an NSString of s (+1; None when it cannot be made)
pub fn nsstring(s: &str) -> Option<Obj> {
    // SAFETY: alloc / initWithBytes:length:encoding: (NSUTF8StringEncoding = 4) copies the bytes.
    let id = unsafe {
        let o: Id = msg!(class(c"NSString"), c"alloc"; Id);
        msg!(o, c"initWithBytes:length:encoding:", s.as_ptr() => *const u8, s.len() => usize, 4usize => usize; Id)
    };
    Obj::new(id)
}

/// the text of an NSString
pub fn string_of(obj: Id) -> String {
    if obj.is_null() {
        return String::new();
    }
    // SAFETY: UTF8String on a live NSString; the C string lives as long as the object or the
    // current pool and is copied at once.
    unsafe {
        let p: *const c_char = msg!(obj, c"UTF8String"; *const c_char);
        if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
    }
}

/// an NSError's localized description
pub fn error_text(err: Id) -> String {
    if err.is_null() {
        return String::from("(no error object)");
    }
    // SAFETY: localizedDescription of a live NSError (autoreleased: the caller's pool).
    unsafe { string_of(msg!(err, c"localizedDescription"; Id)) }
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct MTLOrigin {
    pub x: usize,
    pub y: usize,
    pub z: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct MTLSize {
    pub w: usize,
    pub h: usize,
    pub d: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct MTLViewport {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub znear: f64,
    pub zfar: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct MTLScissorRect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct MTLClearColor {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct CGSize {
    pub w: f64,
    pub h: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct NSRange {
    pub location: usize,
    pub length: usize,
}

// MTLResourceOptions
pub const STORAGE_SHARED: usize = 0 << 4;
pub const STORAGE_PRIVATE: usize = 2 << 4;
pub const CPU_CACHE_WRITE_COMBINED: usize = 1;
