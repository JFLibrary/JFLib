// apple_mac_screen -- (screen.rs) macOS: the colour pick and pictures of the screen
//
//   pick_available() -> bool         NSColorSampler is there (macOS 10.15 and later)
//   pick_start(test) -> Result       the system's loupe (NSColorSampler): the user clicks a pixel
//                                    anywhere, Esc cancels; the colour comes in sRGB.  It needs no
//                                    right: the system takes the picture, the app gets one colour.
//                                    A test point (pick_test_point) answers the pixel there from a
//                                    picture of the screen instead -- which needs the right below
//   capture_available() -> bool      true (the right is asked when taking)
//   capture_start(rect) -> Result    a picture of rect (desktop px; None: every monitor together)
//   mac_screen_access() -> bool      (tests) the process may record the screen
//
// Pictures need the user's "Screen & System Audio Recording" right (TCC) for the program (the
// app, or the terminal that started it): without it capture_start asks the system once
// (CGRequestScreenCaptureAccess shows its dialog, which leads to the System Settings) and answers
// Denied; the right counts from the next start of the program.  macOS 15.2 and later take the
// picture through ScreenCaptureKit (SCScreenshotManager captureImageInRect:, the answer on a queue
// of the system); before, CGWindowListCreateImage (looked up at run time: newer SDKs no longer
// offer it).  Desktop px are points times the screen's scale from the top left of the primary
// screen (monitors_os): a rectangle becomes points by the scale of the monitor at its corner.  The
// picture is drawn into B, G, R, x rows in sRGB (colours match what the app drew in sRGB, up to
// the conversion through the display's profile).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use core::ffi::{c_char, c_void};
use core::sync::atomic::{AtomicPtr, AtomicU8, Ordering};

use rt::sync::SpinLock;

use super::super::block;
use super::super::objc::{Id, NIL, NSPoint, NSRect, NSSize, Sel, class, msg, objc_autoreleasePoolPop, objc_autoreleasePoolPush, release, sel,
                         string_of};
use crate::screen::{Monitor, ScreenError, Shot, push_color, push_shot};

type CfRef = *const c_void;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static kCGColorSpaceSRGB: CfRef;
    fn CGColorSpaceCreateWithName(name: CfRef) -> CfRef;
    fn CGColorSpaceRelease(cs: CfRef);
    fn CGBitmapContextCreate(data: *mut c_void, w: usize, h: usize, bpc: usize, bpr: usize, cs: CfRef, info: u32) -> CfRef;
    fn CGContextSetInterpolationQuality(ctx: CfRef, q: i32);
    fn CGContextDrawImage(ctx: CfRef, r: NSRect, img: CfRef);
    fn CGContextRelease(ctx: CfRef);
    fn CGImageRelease(img: CfRef);
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

const BITMAP_BGRX: u32 = (2 << 12) | 6; // kCGBitmapByteOrder32Little | kCGImageAlphaNoneSkipFirst
const RTLD_LAZY: i32 = 1;
const RTLD_DEFAULT: isize = -2;
/// SCStreamErrorUserDeclined: the user (or the system) refused the right
const SC_DECLINED: isize = -3801;

/// what a running request answers: x, y, w, h in desktop px (the picture's size is w x h)
static SHOT_REQ: SpinLock<Option<[i32; 4]>> = SpinLock::new(None);
static PICK_REQ: SpinLock<Option<[i32; 4]>> = SpinLock::new(None);
static SHOT_BLOCK: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static PICK_BLOCK: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static SAMPLER_BLOCK: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
/// the last colour sampler (kept until the next pick: it calls its handler and may still need itself)
static SAMPLER: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
/// ScreenCaptureKit looked up: 0 not yet, 1 there (captureImageInRect:), 2 not there
static SC: AtomicU8 = AtomicU8::new(0);
/// asked the system for the right once in this process
static ASKED: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Copy)]
enum Dest {
    Shot,
    Pick,
}

pub fn pick_available() -> bool {
    !class(c"NSColorSampler").is_null()
}

pub fn capture_available() -> bool {
    true
}

/// (tests) the process may record the screen (asks nothing)
pub fn mac_screen_access() -> bool {
    // SAFETY: no arguments; CoreGraphics answers from the TCC database.
    unsafe { CGPreflightScreenCaptureAccess() }
}

pub fn capture_start(rect: Option<[i32; 4]>) -> Result<(), ScreenError> {
    let r = match rect {
        Some(r) => r,
        None => union_px(&super::monitors_os()).ok_or(ScreenError::Unavailable)?,
    };
    capture(Dest::Shot, r)
}

pub fn pick_start(test: Option<(i32, i32)>) -> Result<(), ScreenError> {
    if let Some((x, y)) = test {
        return capture(Dest::Pick, [x, y, 1, 1]);
    }
    if !pick_available() {
        return Err(ScreenError::Unavailable);
    }
    super::a11y_mac::on_main(|| {
        // SAFETY: a sampler made and shown on the main thread; the handler is a global block of ours;
        // the previous sampler (done) released here.
        unsafe {
            let s: Id = msg!(msg!(class(c"NSColorSampler"), c"alloc"; Id), c"init"; Id);
            if s.is_null() {
                return Err(ScreenError::Failed(String::from("NSColorSampler")));
            }
            let old = SAMPLER.swap(s, Ordering::AcqRel);
            if !old.is_null() {
                release(old);
            }
            let b = block::global(&SAMPLER_BLOCK, sampled as *const c_void);
            msg!(s, c"showSamplerWithSelectionHandler:", b => *const c_void; ());
        }
        Ok(())
    })
}

/// void (^)(NSColor *selectedColor): the user's colour, nil when cancelled (main thread)
extern "C" fn sampled(_block: *const c_void, color: Id) {
    if color.is_null() {
        push_color(Err(ScreenError::Cancelled));
        return;
    }
    // SAFETY: the colour converted to sRGB (an autoreleased object of AppKit); its components read.
    let rgb = unsafe {
        let pool = objc_autoreleasePoolPush();
        let cs: Id = msg!(class(c"NSColorSpace"), c"sRGBColorSpace"; Id);
        let c: Id = if cs.is_null() { NIL } else { msg!(color, c"colorUsingColorSpace:", cs => Id; Id) };
        let v = if c.is_null() {
            None
        } else {
            let ch = |v: f64| ((v.clamp(0.0, 1.0) * 255.0) + 0.5) as u32;
            Some((ch(msg!(c, c"redComponent"; f64)) << 16) | (ch(msg!(c, c"greenComponent"; f64)) << 8) | ch(msg!(c, c"blueComponent"; f64)))
        };
        objc_autoreleasePoolPop(pool);
        v
    };
    push_color(rgb.ok_or_else(|| ScreenError::Failed(String::from("the colour has no sRGB form"))));
}

/// every monitor together (desktop px)
fn union_px(v: &[Monitor]) -> Option<[i32; 4]> {
    let first = v.first()?;
    let (mut l, mut t, mut r, mut b) = (first.rect[0], first.rect[1], first.rect[0] + first.rect[2], first.rect[1] + first.rect[3]);
    for m in v {
        l = l.min(m.rect[0]);
        t = t.min(m.rect[1]);
        r = r.max(m.rect[0] + m.rect[2]);
        b = b.max(m.rect[1] + m.rect[3]);
    }
    Some([l, t, r - l, b - t])
}

/// a rectangle of desktop px in the global display points of Quartz (top left of the primary screen)
fn to_points(r: [i32; 4]) -> NSRect {
    let scale = crate::screen::monitor_at(r[0], r[1]).map_or(1.0, |m| (m.dpi.max(1) as f64) / 96.0);
    NSRect { origin: NSPoint { x: r[0] as f64 / scale, y: r[1] as f64 / scale }, size: NSSize { w: r[2] as f64 / scale, h: r[3] as f64 / scale } }
}

/// ScreenCaptureKit's captureImageInRect: is there (macOS 15.2 and later)
fn screencapturekit() -> bool {
    match SC.load(Ordering::Acquire) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    // SAFETY: loading a system framework (its classes register themselves); then a class method asked.
    let ok = unsafe {
        dlopen(c"/System/Library/Frameworks/ScreenCaptureKit.framework/ScreenCaptureKit".as_ptr(), RTLD_LAZY);
        let c = class(c"SCScreenshotManager");
        !c.is_null() && msg!(c, c"respondsToSelector:", sel(c"captureImageInRect:completionHandler:") => Sel; bool)
    };
    SC.store(if ok { 1 } else { 2 }, Ordering::Release);
    ok
}

fn capture(dest: Dest, r: [i32; 4]) -> Result<(), ScreenError> {
    if r[2] <= 0 || r[3] <= 0 {
        return Err(ScreenError::Failed(String::from("empty rectangle")));
    }
    if !mac_screen_access() {
        // the system's dialog once per process (macOS shows it once per program anyway)
        if ASKED.swap(1, Ordering::AcqRel) == 0 {
            // SAFETY: no arguments.
            unsafe { CGRequestScreenCaptureAccess() };
        }
        return Err(ScreenError::Denied);
    }
    let slot = match dest {
        Dest::Shot => &SHOT_REQ,
        Dest::Pick => &PICK_REQ,
    };
    {
        let mut s = slot.lock();
        if s.is_some() {
            return Err(ScreenError::Busy);
        }
        *s = Some(r);
    }
    let pts = to_points(r);
    if screencapturekit() {
        let (bslot, invoke) = match dest {
            Dest::Shot => (&SHOT_BLOCK, shot_done as *const c_void),
            Dest::Pick => (&PICK_BLOCK, pick_done as *const c_void),
        };
        let b = block::global(bslot, invoke);
        // SAFETY: a class method of ScreenCaptureKit (loaded above) with a CGRect and a global block of ours.
        unsafe { msg!(class(c"SCScreenshotManager"), c"captureImageInRect:completionHandler:", pts => NSRect, b => *const c_void; ()) };
        return Ok(());
    }
    // before macOS 15.2: CGWindowListCreateImage of everything on screen in the rectangle
    // SAFETY: dlsym in the process's images (CoreGraphics is linked); the function has this signature.
    let f = unsafe { dlsym(RTLD_DEFAULT as *mut c_void, c"CGWindowListCreateImage".as_ptr()) };
    if f.is_null() {
        *slot.lock() = None;
        return Err(ScreenError::Unavailable);
    }
    // SAFETY: CGWindowListCreateImage(CGRect, kCGWindowListOptionOnScreenOnly, kCGNullWindowID,
    // kCGWindowImageDefault) -> a CGImage of ours (released in deliver).
    let img = unsafe {
        let f: extern "C" fn(NSRect, u32, u32, u32) -> CfRef = core::mem::transmute(f);
        f(pts, 1, 0, 0)
    };
    deliver(dest, img, NIL);
    if !img.is_null() {
        // SAFETY: the image made above.
        unsafe { CGImageRelease(img) };
    }
    Ok(())
}

/// void (^)(CGImageRef, NSError *) of a picture (a queue of the system)
extern "C" fn shot_done(_block: *const c_void, img: CfRef, err: Id) {
    deliver(Dest::Shot, img, err);
}

/// the same for a pick at a test point
extern "C" fn pick_done(_block: *const c_void, img: CfRef, err: Id) {
    deliver(Dest::Pick, img, err);
}

/// the answer of a request: the picture drawn into w x h pixels, else why not
fn deliver(dest: Dest, img: CfRef, err: Id) {
    let req = match dest {
        Dest::Shot => SHOT_REQ.lock().take(),
        Dest::Pick => PICK_REQ.lock().take(),
    };
    let Some(r) = req else { return };
    // SAFETY: a pool for the error's strings.
    let pool = unsafe { objc_autoreleasePoolPush() };
    let res = if !img.is_null() {
        draw(img, r[2], r[3]).map(|px| Shot { rect: r, w: r[2], h: r[3], px, file: None })
    } else if !err.is_null() {
        // SAFETY: the NSError the system handed over: its code and description.
        let (code, text) = unsafe { (msg!(err, c"code"; isize), string_of(msg!(err, c"localizedDescription"; Id))) };
        Err(if code == SC_DECLINED { ScreenError::Denied } else { ScreenError::Failed(format!("{text} ({code})")) })
    } else {
        Err(ScreenError::Failed(String::from("no picture")))
    };
    // SAFETY: the pool pushed above.
    unsafe { objc_autoreleasePoolPop(pool) };
    match dest {
        Dest::Shot => push_shot(res),
        Dest::Pick => push_color(res.and_then(|s| crate::screen::shot_pixel(&s, 0, 0).ok_or(ScreenError::Failed(String::from("no pixel"))))),
    }
}

/// the image drawn into w x h pixels of B, G, R, x in sRGB
fn draw(img: CfRef, w: i32, h: i32) -> Result<alloc::vec::Vec<u8>, ScreenError> {
    let (w, h) = (w.max(1) as usize, h.max(1) as usize);
    let mut px = vec![0u8; w * h * 4];
    // SAFETY: a bitmap context over our buffer (w * 4 bytes a row), the image drawn into it and the
    // context released before the buffer is handed on.
    unsafe {
        let cs = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        let ctx = CGBitmapContextCreate(px.as_mut_ptr() as *mut c_void, w, h, 8, w * 4, cs, BITMAP_BGRX);
        if !cs.is_null() {
            CGColorSpaceRelease(cs);
        }
        if ctx.is_null() {
            return Err(ScreenError::Failed(String::from("CGBitmapContextCreate")));
        }
        CGContextSetInterpolationQuality(ctx, 1); // kCGInterpolationNone: pixels stay pixels
        CGContextDrawImage(ctx, NSRect::new(0.0, 0.0, w as f64, h as f64), img);
        CGContextRelease(ctx);
    }
    for p in px.chunks_exact_mut(4) {
        p[3] = 255;
    }
    Ok(px)
}
