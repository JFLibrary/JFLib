// apple_ios -- iOS's host (docs/APPLE.md AP5, decision 4): UIKit drives the app as Android's
// activity does.  The app's runtime is web.rs's HostRuntime (ui::launch installed it in jfgui_main);
// this module is to it what android/bridge.c and GalleryActivity.java are on Android.
//
//   before_main()        the canvas the windows start in (the view's real size follows)
//   run() -> !           UIApplicationMain: JFAppDelegate, one scene (JFSceneDelegate) with a window,
//                        JFViewController and its JFView inside the safe area, above the keyboard
//   wake()               (any thread; apple_thread.rs wake) the next tick takes E_WAKE
//   finish() -> !        (plat::app_finish, ui.quit) the process ends
//   open_url(url)        (apple_disp.rs open_url) UIApplication openURL
//   jfgui_host_time_us / jfgui_host_present / jfgui_host_request   what web.rs asks of a host
//
// JFView shows the frames as the contents of its layer (a CGImage of a copy, scaled to the view).
// Fingers go to jfgui_finger (the coalesced samples of a move with their age, for the velocity), the
// Apple Pencil is a pen (pressure, tilt), the pointer of an iPad's trackpad or mouse a mouse.  It is
// the UIKeyInput of the on-screen keyboard (insertText -> jfgui_text, deleteBackward -> Backspace),
// first responder after a gesture reached a widget (jfgui_touch_reached) while the app wants text
// (REQUEST_IME with a height), resigning when the app no longer does or the user taps "Done" on the
// bar above the keyboard (an iPhone's keyboard has no key to close it; the text field keeps the focus,
// a tap into it brings the keyboard back).  The view controller takes the keys of a hardware keyboard
// the text system does not (pressesBegan).
// The ticks: a CADisplayLink while the app animates (jfgui_frame_delay 0), else paused with a timer
// for the delay; input and wake-ups unpause it.  Px per point: the screen's scale, at most 2 and at
// most 2048 px on the longer side (as Android's bitmap); dpi 96 x that.
// The lifecycle: active / inactive -> E_FOCUS_IN / OUT, background / foreground -> E_POWER 2 / 3
// (UIC_SUSPEND), termination -> E_SESSION_END.  The scene's appearance, reduced motion and darker
// colours go to jfgui_system (web_system.rs).
//
// Everything runs on the main thread (UIKit's rule) but wake().

use alloc::format;
use core::cell::Cell;
use core::ffi::{c_char, c_void};
use core::sync::atomic::{AtomicBool, Ordering};

use super::objc::{Id, NIL, NSPoint, NSRect, Sel, class, msg, nsstring, release, string_of, subclass};
use crate::web::{jfgui_buffer, jfgui_complete, jfgui_event, jfgui_finger, jfgui_frame_delay, jfgui_input, jfgui_system, jfgui_text,
                 jfgui_tick, jfgui_touch_reached};
use crate::{E_DOWN, E_DPI, E_FOCUS_IN, E_FOCUS_OUT, E_KEY_DOWN, E_KEY_UP, E_MOVE, E_POWER, E_RESIZE, E_SESSION_END, E_UP, Event,
            MOD_ALT, MOD_CAPS, MOD_CTRL, MOD_SHIFT, MOD_SUPER, PTR_PEN};

type CfRef = *const c_void;

// the system services of AP5b: the clipboard, the file picker, notifications
#[path = "apple_ios_services.rs"]
mod services;
pub(crate) use services::notes;
pub use services::{clip_get, clip_has, clip_set, clip_set_many, file_dialog, file_dialog_poll};
// the motion sensors and the location (sensors.rs through web_sensors.rs)
#[path = "apple_ios_motion.rs"]
pub(crate) mod motion;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CgVector {
    dx: f64,
    dy: f64,
}

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    fn UIApplicationMain(argc: i32, argv: *mut *mut c_char, principal: Id, delegate: Id) -> i32;
    fn UIAccessibilityIsReduceMotionEnabled() -> bool;
    fn UIAccessibilityDarkerSystemColorsEnabled() -> bool;
}
#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {
    fn CACurrentMediaTime() -> f64;
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
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataCreate(alloc: CfRef, bytes: *const u8, n: isize) -> CfRef;
    fn CFRelease(r: CfRef);
}
#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {
    static NSRunLoopCommonModes: Id;
}
// libdispatch and pthread (libSystem)
unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(queue: *const c_void, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
    fn dispatch_after_f(when: u64, queue: *const c_void, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
    fn dispatch_time(when: u64, delta: i64) -> u64;
    fn pthread_main_np() -> i32;
}

const BITMAP_RGBX: u32 = 5; // kCGImageAlphaNoneSkipLast, the default byte order: R, G, B, x
const BITMAP_BGRX: u32 = (2 << 12) | 6; // kCGBitmapByteOrder32Little | kCGImageAlphaNoneSkipFirst
const FINGERS: usize = 10;
// UITouchType
const TOUCH_PENCIL: isize = 2;
const TOUCH_POINTER: isize = 3;
// UIKeyModifierFlags
const KEY_CAPS: isize = 1 << 16;
const KEY_SHIFT: isize = 1 << 17;
const KEY_CONTROL: isize = 1 << 18;
const KEY_OPTION: isize = 1 << 19;
const KEY_COMMAND: isize = 1 << 20;

/// the host's state: main thread only (UIKit's callbacks, the ticks), no borrow outlives a call
struct Host {
    window: Cell<Id>,
    /// the controller's root view (the safe area's surroundings) and JFView in it
    root: Cell<Id>,
    view: Cell<Id>,
    link: Cell<Id>,
    cs: Cell<CfRef>,
    /// the canvas in px and its dpi as last sent (E_RESIZE, E_DPI)
    size: Cell<(i32, i32, i32)>,
    /// px of the canvas per point of the view
    scale: Cell<f64>,
    /// the scene is active: the display link may run
    resumed: Cell<bool>,
    /// the GPU's CAMetalLayer, a sublayer of JFView's (NIL until lib/gpu or the UI's output asks)
    gpu: Cell<Id>,
    /// the app wants text (REQUEST_IME with a height) / the view is first responder (the keyboard)
    ime_visible: Cell<bool>,
    ime_shown: Cell<bool>,
    /// the last finger lifted after a gesture that reached a widget: the keyboard follows after
    /// the next tick (the text field's focus may come with it)
    gesture: Cell<bool>,
    /// the UITouch of each finger slot (0: free), of the pencil, of the trackpad's pointer
    fingers: Cell<[usize; FINGERS]>,
    pen: Cell<usize>,
    pointer: Cell<(usize, u32)>,
    /// the delay timer that counts (dispatch_after, older ones are ignored)
    timer: Cell<u32>,
    epoch_ms: Cell<u64>,
    /// the last traits sent (jfgui_system)
    traits: Cell<u32>,
    /// the canvas's top left colour (0xRRGGBB; u32::MAX: none yet) and the status bar's style
    /// (UIStatusBarStyle: 1 light content, 3 dark content)
    edge: Cell<u32>,
    bar: Cell<isize>,
    /// the bar above the on-screen keyboard (a UIToolbar with "Done")
    accessory: Cell<Id>,
}
struct Main(Host);
// SAFETY: only the main thread touches the host (every function here but wake() runs there).
unsafe impl Sync for Main {}
static HOST: Main = Main(Host {
    window: Cell::new(NIL),
    root: Cell::new(NIL),
    view: Cell::new(NIL),
    link: Cell::new(NIL),
    cs: Cell::new(core::ptr::null()),
    size: Cell::new((0, 0, 0)),
    scale: Cell::new(2.0),
    resumed: Cell::new(false),
    gpu: Cell::new(NIL),
    ime_visible: Cell::new(false),
    ime_shown: Cell::new(false),
    gesture: Cell::new(false),
    fingers: Cell::new([0; FINGERS]),
    pen: Cell::new(0),
    pointer: Cell::new((0, 1)),
    timer: Cell::new(0),
    epoch_ms: Cell::new(0),
    traits: Cell::new(u32::MAX),
    edge: Cell::new(u32::MAX),
    bar: Cell::new(0),
    accessory: Cell::new(NIL),
});
static WAKE_QUEUED: AtomicBool = AtomicBool::new(false);
/// the scene is not active: no GPU work (iOS refuses a background app's command buffers)
static GPU_HOLD: AtomicBool = AtomicBool::new(false);
/// UIKit runs the app (run): wake() goes to the host, not to the pipe of a waiting loop
static HOSTED: AtomicBool = AtomicBool::new(false);

/// (apple_thread.rs wake, any thread) the UIKit host runs the app
pub(crate) fn hosted() -> bool {
    HOSTED.load(Ordering::Acquire)
}

fn host() -> &'static Host {
    &HOST.0
}

fn main_queue() -> *const c_void {
    // SAFETY: the address of libdispatch's main queue object, never read here.
    #[allow(unused_unsafe)]
    unsafe {
        &raw const _dispatch_main_q
    }
}

fn on_main() -> bool {
    // SAFETY: a plain query of the calling thread.
    unsafe { pthread_main_np() != 0 }
}

/// ms since the host's first tick (the runtime's clock)
fn time_ms() -> u32 {
    let h = host();
    let now = crate::time_us() / 1000;
    if h.epoch_ms.get() == 0 {
        h.epoch_ms.set(now.max(1));
    }
    now.saturating_sub(h.epoch_ms.get()) as u32
}

fn rnd(v: f64) -> i32 {
    if v >= 0.0 { (v + 0.5) as i32 } else { (v - 0.5) as i32 }
}

/// bytes into web.rs's inbox (jfgui_buffer) -> false without memory
fn inbox(bytes: &[u8]) -> bool {
    let p = jfgui_buffer(bytes.len() as u32);
    if p == 0 {
        return bytes.is_empty();
    }
    // SAFETY: jfgui_buffer reserved bytes.len() bytes at p.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), p as *mut u8, bytes.len()) };
    true
}

/// the objects of an NSSet / NSArray
fn items(collection: Id) -> impl Iterator<Item = Id> {
    // SAFETY: allObjects (an NSSet) / the array itself; count and objectAtIndex: on a live array
    // that the autorelease pool of the event keeps.
    let array: Id = unsafe {
        if collection.is_null() {
            NIL
        } else if msg!(collection, c"isKindOfClass:", class(c"NSSet") => Id; bool) {
            msg!(collection, c"allObjects"; Id)
        } else {
            collection
        }
    };
    let n = if array.is_null() { 0 } else { unsafe { msg!(array, c"count"; usize) } };
    (0..n).map(move |i| unsafe { msg!(array, c"objectAtIndex:", i => usize; Id) })
}

// ---------------------------------------------------------------- the start

/// (apple.rs main, before jfgui_main) the canvas the windows start in: an iPhone's 390 x 844 points
/// at 2 px per point; the view's layout sends the real size before the first frame shows
pub(crate) fn before_main() {
    crate::web::host_viewport(780, 1688, 192);
}

/// (apple.rs main, an app installed its runtime) UIKit takes the main thread
pub(crate) fn run() -> ! {
    HOSTED.store(true, Ordering::Release);
    register();
    let name = nsstring("JFAppDelegate");
    // SAFETY: the process's own argc / argv (dyld's); UIApplicationMain never returns.
    unsafe {
        let argc = *super::ffi::_NSGetArgc();
        let argv = *super::ffi::_NSGetArgv();
        UIApplicationMain(argc, argv, NIL, name);
    }
    rt::os::exit(0)
}

/// (plat::app_finish) the runtime ended (ui.quit): the process ends, iOS shows the home screen
pub(crate) fn finish() -> ! {
    rt::os::exit(0)
}

fn register() {
    type Imp = *const c_void;
    subclass(c"NSObject", c"JFAppDelegate", &[
        (c"application:didFinishLaunchingWithOptions:", app_launched as Imp, c"B@:@@"),
        (c"application:configurationForConnectingSceneSession:options:", app_scene_config as Imp, c"@@:@@@"),
        (c"applicationWillTerminate:", app_terminate as Imp, c"v@:@"),
    ], &[c"UIApplicationDelegate"]);
    subclass(c"UIResponder", c"JFSceneDelegate", &[
        (c"scene:willConnectToSession:options:", scene_connect as Imp, c"v@:@@@"),
        (c"sceneDidBecomeActive:", scene_active as Imp, c"v@:@"),
        (c"sceneWillResignActive:", scene_inactive as Imp, c"v@:@"),
        (c"sceneDidEnterBackground:", scene_background as Imp, c"v@:@"),
        (c"sceneWillEnterForeground:", scene_foreground as Imp, c"v@:@"),
        (c"window", scene_window as Imp, c"@@:"),
        (c"setWindow:", scene_set_window as Imp, c"v@:@"),
    ], &[c"UIWindowSceneDelegate"]);
    subclass(c"UIViewController", c"JFViewController", &[
        (c"loadView", vc_load_view as Imp, c"v@:"),
        (c"supportedInterfaceOrientations", vc_orientations as Imp, c"Q@:"),
        (c"preferredStatusBarStyle", vc_bar_style as Imp, c"q@:"),
        (c"canBecomeFirstResponder", yes as Imp, c"B@:"),
        (c"pressesBegan:withEvent:", vc_presses_began as Imp, c"v@:@@"),
        (c"pressesEnded:withEvent:", vc_presses_ended as Imp, c"v@:@@"),
        (c"pressesCancelled:withEvent:", vc_presses_ended as Imp, c"v@:@@"),
    ], &[]);
    subclass(c"UIView", c"JFView", &[
        (c"layoutSubviews", view_layout as Imp, c"v@:"),
        (c"step:", view_step as Imp, c"v@:@"),
        (c"touchesBegan:withEvent:", view_touches_began as Imp, c"v@:@@"),
        (c"touchesMoved:withEvent:", view_touches_moved as Imp, c"v@:@@"),
        (c"touchesEnded:withEvent:", view_touches_ended as Imp, c"v@:@@"),
        (c"touchesCancelled:withEvent:", view_touches_cancelled as Imp, c"v@:@@"),
        (c"traitCollectionDidChange:", view_traits_changed as Imp, c"v@:@"),
        (c"canBecomeFirstResponder", yes as Imp, c"B@:"),
        (c"hasText", yes as Imp, c"B@:"),
        (c"insertText:", view_insert_text as Imp, c"v@:@"),
        (c"deleteBackward", view_delete_backward as Imp, c"v@:"),
        (c"inputAccessoryView", view_accessory as Imp, c"@@:"),
        (c"jfDone:", view_done as Imp, c"v@:@"),
        // UITextInputTraits: the keyboard types what it is given, nothing corrected or replaced
        (c"autocorrectionType", trait_no as Imp, c"q@:"),
        (c"spellCheckingType", trait_no as Imp, c"q@:"),
        (c"smartQuotesType", trait_no as Imp, c"q@:"),
        (c"smartDashesType", trait_no as Imp, c"q@:"),
        (c"smartInsertDeleteType", trait_no as Imp, c"q@:"),
        (c"inlinePredictionType", trait_no as Imp, c"q@:"),
        (c"autocapitalizationType", trait_zero as Imp, c"q@:"),
    ], &[c"UIKeyInput"]);
}

extern "C" fn yes(_this: Id, _sel: Sel) -> bool {
    true
}
/// UITextAutocorrectionTypeNo and its siblings (No = 1)
extern "C" fn trait_no(_this: Id, _sel: Sel) -> isize {
    1
}
/// UITextAutocapitalizationTypeNone
extern "C" fn trait_zero(_this: Id, _sel: Sel) -> isize {
    0
}

extern "C" fn app_launched(_this: Id, _sel: Sel, _app: Id, _options: Id) -> bool {
    true
}

/// the one scene's configuration: JFSceneDelegate (Info.plist names it as well)
extern "C" fn app_scene_config(_this: Id, _sel: Sel, _app: Id, session: Id, _options: Id) -> Id {
    // SAFETY: UISceneConfiguration made and handed back autoreleased, as UIKit expects.
    unsafe {
        let role: Id = msg!(session, c"role"; Id);
        let name = nsstring("Default");
        let cfg: Id = msg!(class(c"UISceneConfiguration"), c"alloc"; Id);
        let cfg: Id = msg!(cfg, c"initWithName:sessionRole:", name => Id, role => Id; Id);
        release(name);
        if !cfg.is_null() {
            msg!(cfg, c"setDelegateClass:", class(c"JFSceneDelegate") => Id; ());
            msg!(cfg, c"autorelease"; Id);
        }
        cfg
    }
}

/// iOS ends the app (rare: it usually suspends and ends it later without a word)
extern "C" fn app_terminate(_this: Id, _sel: Sel, _app: Id) {
    crate::evq_push(Event { typ: E_SESSION_END, ..Event::default() });
    jfgui_input(time_ms());
    jfgui_tick(time_ms());
}

// ---------------------------------------------------------------- the scene

extern "C" fn scene_connect(_this: Id, _sel: Sel, scene: Id, _session: Id, _options: Id) {
    let h = host();
    if !h.window.get().is_null() {
        return;
    }
    // SAFETY: UIKit on the main thread; the window and its controller live for the process.
    unsafe {
        let win: Id = msg!(class(c"UIWindow"), c"alloc"; Id);
        let win: Id = msg!(win, c"initWithWindowScene:", scene => Id; Id);
        if win.is_null() {
            return;
        }
        let vc: Id = msg!(class(c"JFViewController"), c"alloc"; Id);
        let vc: Id = msg!(vc, c"init"; Id);
        msg!(win, c"setRootViewController:", vc => Id; ());
        release(vc);
        h.window.set(win);
        send_traits(msg!(scene, c"traitCollection"; Id));
        msg!(win, c"makeKeyAndVisible"; ());
        msg!(vc, c"becomeFirstResponder"; bool);
    }
}

extern "C" fn scene_window(_this: Id, _sel: Sel) -> Id {
    host().window.get()
}
extern "C" fn scene_set_window(_this: Id, _sel: Sel, _window: Id) {}

extern "C" fn scene_active(_this: Id, _sel: Sel, _scene: Id) {
    GPU_HOLD.store(false, Ordering::Release);
    life(E_FOCUS_IN, 0);
    host().resumed.set(true);
    request_frame();
}

extern "C" fn scene_inactive(_this: Id, _sel: Sel, _scene: Id) {
    cancel_gestures();
    // (before the scene goes to the background: the output leaves the GPU in the tick below)
    GPU_HOLD.store(true, Ordering::Release);
    life(E_FOCUS_OUT, 0);
    tick_now();
    host().resumed.set(false);
}

/// UIC_SUSPEND 2: the app saves; one tick takes it before iOS suspends the process
extern "C" fn scene_background(_this: Id, _sel: Sel, _scene: Id) {
    life(E_POWER, 2);
    tick_now();
}

extern "C" fn scene_foreground(_this: Id, _sel: Sel, _scene: Id) {
    life(E_POWER, 3);
}

fn life(typ: u8, data: u32) {
    jfgui_event(typ as u32, 0, 0, 0, 0, 0, data);
    jfgui_input(time_ms());
}

/// dark / light, reduced motion, darker colours -> jfgui_system (a change: E_SETTINGS)
fn send_traits(traits: Id) {
    if traits.is_null() {
        return;
    }
    // SAFETY: userInterfaceStyle of a live UITraitCollection; two plain UIKit queries.
    let (style, motion, contrast) = unsafe {
        (msg!(traits, c"userInterfaceStyle"; isize), UIAccessibilityIsReduceMotionEnabled(), UIAccessibilityDarkerSystemColorsEnabled())
    };
    let key = (style as u32 & 3) | (motion as u32) << 2 | (contrast as u32) << 3;
    if host().traits.replace(key) == key {
        return;
    }
    let dark = match style {
        2 => "1",
        1 => "0",
        _ => "-",
    };
    let text = format!("dark={dark}\nmotion={}\ncontrast={}\n", motion as u8, contrast as u8);
    if inbox(text.as_bytes()) {
        jfgui_system(text.len() as u32);
        jfgui_input(time_ms());
        request_frame();
    }
}

// ---------------------------------------------------------------- the view controller and its view

/// the root view (the safe area's surroundings) and JFView in it: inside the safe area, above the
/// keyboard (keyboardLayoutGuide); the display link that ticks
extern "C" fn vc_load_view(this: Id, _sel: Sel) {
    let h = host();
    // SAFETY: UIKit on the main thread; the root view belongs to the controller, JFView to it.
    unsafe {
        let bg: Id = msg!(class(c"UIColor"), c"systemGroupedBackgroundColor"; Id);
        let root: Id = msg!(class(c"UIView"), c"alloc"; Id);
        let root: Id = msg!(root, c"initWithFrame:", NSRect::default() => NSRect; Id);
        msg!(root, c"setBackgroundColor:", bg => Id; ());
        msg!(this, c"setView:", root => Id; ());
        let view: Id = msg!(class(c"JFView"), c"alloc"; Id);
        let view: Id = msg!(view, c"initWithFrame:", NSRect::default() => NSRect; Id);
        msg!(view, c"setMultipleTouchEnabled:", true => bool; ());
        msg!(view, c"setBackgroundColor:", bg => Id; ());
        msg!(view, c"setTranslatesAutoresizingMaskIntoConstraints:", false => bool; ());
        msg!(root, c"addSubview:", view => Id; ());
        let safe: Id = msg!(root, c"safeAreaLayoutGuide"; Id);
        let keyboard: Id = msg!(root, c"keyboardLayoutGuide"; Id);
        for (a, guide, b) in [(c"topAnchor", safe, c"topAnchor"), (c"leadingAnchor", safe, c"leadingAnchor"),
                              (c"trailingAnchor", safe, c"trailingAnchor"), (c"bottomAnchor", keyboard, c"topAnchor")] {
            let mine: Id = msg!(view, a; Id);
            let theirs: Id = msg!(guide, b; Id);
            let c: Id = msg!(mine, c"constraintEqualToAnchor:", theirs => Id; Id);
            msg!(c, c"setActive:", true => bool; ());
        }
        let link: Id = msg!(class(c"CADisplayLink"), c"displayLinkWithTarget:selector:", view => Id,
                            super::objc::sel(c"step:") => Sel; Id);
        if !link.is_null() {
            msg!(link, c"retain"; Id);
            msg!(link, c"setPaused:", true => bool; ());
            let rl: Id = msg!(class(c"NSRunLoop"), c"mainRunLoop"; Id);
            msg!(link, c"addToRunLoop:forMode:", rl => Id, NSRunLoopCommonModes => Id; ());
        }
        h.root.set(root);
        h.view.set(view);
        h.link.set(link);
        release(view);
        release(root);
    }
}

/// UIInterfaceOrientationMaskAll (Info.plist narrows it on the iPhone)
extern "C" fn vc_orientations(_this: Id, _sel: Sel) -> usize {
    30
}

/// the status bar's text on the app's colour: dark on a light edge, light on a dark one
extern "C" fn vc_bar_style(_this: Id, _sel: Sel) -> isize {
    host().bar.get()
}

/// the canvas's top left pixel (R, G, B): the colour around the safe area (the status bar, the home
/// indicator's strip) and the status bar's style follow the app, not the system's appearance (an app
/// may keep its own theme)
fn edge_color(rgb: [u8; 3]) {
    let h = host();
    let key = (rgb[0] as u32) << 16 | (rgb[1] as u32) << 8 | rgb[2] as u32;
    if h.edge.replace(key) == key {
        return;
    }
    let bright = rgb[0] as u32 * 299 + rgb[1] as u32 * 587 + rgb[2] as u32 * 114 > 140_000;
    let style = if bright { 3 } else { 1 };
    // SAFETY: UIColor and the live views / controller on the main thread.
    unsafe {
        let c = |v: u8| v as f64 / 255.0;
        let color: Id = msg!(class(c"UIColor"), c"colorWithRed:green:blue:alpha:", c(rgb[0]) => f64, c(rgb[1]) => f64,
                             c(rgb[2]) => f64, 1.0f64 => f64; Id);
        for v in [h.root.get(), h.view.get()] {
            if !v.is_null() {
                msg!(v, c"setBackgroundColor:", color => Id; ());
            }
        }
        if h.bar.replace(style) != style {
            let win = h.window.get();
            let vc: Id = if win.is_null() { NIL } else { msg!(win, c"rootViewController"; Id) };
            if !vc.is_null() {
                msg!(vc, c"setNeedsStatusBarAppearanceUpdate"; ());
            }
        }
    }
}

extern "C" fn view_layout(this: Id, _sel: Sel) {
    // SAFETY: bounds and the trait collection of the live view.
    let (bounds, scale) = unsafe {
        let b: NSRect = msg!(this, c"bounds"; NSRect);
        let traits: Id = msg!(this, c"traitCollection"; Id);
        (b, if traits.is_null() { 2.0 } else { msg!(traits, c"displayScale"; f64) })
    };
    resize(bounds.size.w, bounds.size.h, if scale > 0.0 { scale } else { 2.0 });
    gpu_layer_fit();
}

// ---------------------------------------------------------------- the GPU's layer (apple_ios_gpu.rs)

/// a scene that is not active does no GPU work
pub(crate) fn gpu_hold() -> bool {
    GPU_HOLD.load(Ordering::Acquire)
}

/// run f in a Core Animation transaction without animation
fn no_animation(f: impl FnOnce()) {
    // SAFETY: begin / setDisableActions: / commit of the class, on the main thread.
    unsafe {
        let tx = class(c"CATransaction");
        msg!(tx, c"begin"; ());
        msg!(tx, c"setDisableActions:", true => bool; ());
        f();
        msg!(tx, c"commit"; ());
    }
}

/// the layer follows the view's bounds and the canvas's scale (no autoresizing of layers on iOS)
fn gpu_layer_fit() {
    let h = host();
    let (l, view) = (h.gpu.get(), h.view.get());
    if l.is_null() || view.is_null() {
        return;
    }
    no_animation(|| {
        // SAFETY: our layer and our view, on the main thread.
        unsafe {
            let b: NSRect = msg!(view, c"bounds"; NSRect);
            msg!(l, c"setFrame:", b => NSRect; ());
            msg!(l, c"setContentsScale:", h.scale.get() => f64; ());
        }
    });
}

/// the CAMetalLayer of the view, made at the first call and retained for the caller (a NativeWindow);
/// None without UIKit's host
pub(crate) fn gpu_layer() -> Option<usize> {
    let h = host();
    let view = h.view.get();
    if view.is_null() {
        return None;
    }
    if h.gpu.get().is_null() {
        let cls = class(c"CAMetalLayer");
        if cls.is_null() {
            return None;
        }
        let mut made = NIL;
        no_animation(|| {
            // SAFETY: [CAMetalLayer layer] is autoreleased and retained here (the host keeps it for the
            // process); a sublayer of our view's layer, hidden until gpu_layer_show.
            unsafe {
                let l: Id = msg!(cls, c"layer"; Id);
                if l.is_null() {
                    return;
                }
                msg!(l, c"retain"; Id);
                // (not opaque: without a drawable yet the view's picture below shows through)
                msg!(l, c"setOpaque:", false => bool; ());
                msg!(l, c"setHidden:", true => bool; ());
                let host_layer: Id = msg!(view, c"layer"; Id);
                msg!(host_layer, c"addSublayer:", l => Id; ());
                made = l;
            }
        });
        if made.is_null() {
            return None;
        }
        h.gpu.set(made);
        gpu_layer_fit();
    }
    let l = h.gpu.get();
    // SAFETY: a live layer; the caller's reference (gpu_layer_release).
    unsafe { msg!(l, c"retain"; Id) };
    Some(l as usize)
}

/// (any thread) a reference gpu_layer gave out goes
pub(crate) fn gpu_layer_release(layer: usize) {
    release(layer as Id);
}

/// the layer is shown, or hidden with its last drawable gone (the view's picture shows again)
pub(crate) fn gpu_layer_show(on: bool) {
    let l = host().gpu.get();
    if l.is_null() {
        return;
    }
    no_animation(|| {
        // SAFETY: our layer, on the main thread.
        unsafe {
            msg!(l, c"setHidden:", !on => bool; ());
            if !on {
                msg!(l, c"setContents:", NIL => Id; ());
            }
        }
    });
}

/// the view layer's own picture is not needed while the GPU shows the window
pub(crate) fn gpu_release_picture() {
    let view = host().view.get();
    if view.is_null() {
        return;
    }
    no_animation(|| {
        // SAFETY: the layer of our view, on the main thread.
        unsafe {
            let layer: Id = msg!(view, c"layer"; Id);
            msg!(layer, c"setContents:", NIL => Id; ());
        }
    });
}

/// the view's size in points -> the canvas (E_DPI, E_RESIZE) and a frame at once
fn resize(w: f64, h: f64, screen_scale: f64) {
    if w < 1.0 || h < 1.0 {
        return;
    }
    let host = host();
    let mut s = screen_scale.min(2.0);
    let long = w.max(h) * s;
    if long > 2048.0 {
        s *= 2048.0 / long;
    }
    host.scale.set(s);
    let (pw, ph, dpi) = (rnd(w * s).max(1), rnd(h * s).max(1), rnd(96.0 * s).max(48));
    let (ow, oh, od) = host.size.replace((pw, ph, dpi));
    if (ow, oh, od) == (pw, ph, dpi) {
        return;
    }
    if od != dpi {
        jfgui_event(E_DPI as u32, 0, 0, 0, 0, 0, dpi as u32);
    }
    if (ow, oh) != (pw, ph) {
        jfgui_event(E_RESIZE as u32, 0, 0, 0, pw, ph, 0);
    }
    jfgui_input(time_ms());
    tick_now();
}

extern "C" fn view_traits_changed(this: Id, _sel: Sel, _previous: Id) {
    // SAFETY: the trait collection of the live view.
    send_traits(unsafe { msg!(this, c"traitCollection"; Id) });
}

// ---------------------------------------------------------------- the frames

/// the display link: one tick per frame while it runs
extern "C" fn view_step(_this: Id, _sel: Sel, _link: Id) {
    if !host().resumed.get() {
        set_paused(true);
        return;
    }
    tick_now();
}

fn tick_now() {
    jfgui_tick(time_ms());
    after_tick();
}

/// the keyboard follows the app; the display link runs on (the app animates) or rests for the delay
fn after_tick() {
    let h = host();
    if h.gesture.replace(false) {
        keyboard(h.ime_visible.get());
    } else if h.ime_shown.get() && !h.ime_visible.get() {
        keyboard(false); // a text box went away (its dialog closed): the keyboard goes with it
    }
    let delay = jfgui_frame_delay();
    if delay == 0 && h.resumed.get() {
        set_paused(false);
        return;
    }
    set_paused(true);
    if h.resumed.get() {
        let id = h.timer.get().wrapping_add(1);
        h.timer.set(id);
        // SAFETY: a plain function on the main queue; its context is the timer's number.
        unsafe { dispatch_after_f(dispatch_time(0, delay as i64 * 1_000_000), main_queue(), id as usize as *mut c_void, on_timer) };
    }
}

extern "C" fn on_timer(id: *mut c_void) {
    if id as usize as u32 == host().timer.get() {
        request_frame();
    }
}

fn set_paused(paused: bool) {
    let link = host().link.get();
    if !link.is_null() {
        // SAFETY: the live display link.
        unsafe { msg!(link, c"setPaused:", paused => bool; ()) };
    }
}

/// a tick at the next frame
fn request_frame() {
    if host().resumed.get() {
        set_paused(false);
    }
}

/// (any thread; apple_thread.rs wake under the host) the next tick takes E_WAKE
pub(crate) fn wake() {
    crate::web::host_wake();
    if !WAKE_QUEUED.swap(true, Ordering::AcqRel) {
        // SAFETY: a plain function on the main queue (UIKit's loop runs it).
        unsafe { dispatch_async_f(main_queue(), core::ptr::null_mut(), on_wake) };
    }
}

extern "C" fn on_wake(_ctx: *mut c_void) {
    WAKE_QUEUED.store(false, Ordering::Release);
    request_frame();
}

/// R, G, B, A rows (order 1, web.rs) or B, G, R, x: the whole canvas as the layer's contents (a copy;
/// the layer scales it to the view)
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_host_present(src: *const u8, len: usize, width: i32, height: i32, stride: i32, _rects: *const [i32; 4],
                                     _count: usize, order: u32) {
    let t0 = crate::time_us();
    let h = host();
    let view = h.view.get();
    let need = stride.max(0) as usize * height.max(0) as usize;
    if view.is_null() || src.is_null() || width <= 0 || height <= 0 || stride < width * 4 || need > len || !on_main() {
        return;
    }
    // SAFETY: CoreGraphics objects made from a copy of the pixels and released here; the layer
    // retains the image it shows.
    unsafe {
        if h.cs.get().is_null() {
            h.cs.set(CGColorSpaceCreateWithName(kCGColorSpaceSRGB));
        }
        let data = CFDataCreate(core::ptr::null(), src, need as isize);
        let prov = if data.is_null() { core::ptr::null() } else { CGDataProviderCreateWithCFData(data) };
        let info = if order == 1 { BITMAP_RGBX } else { BITMAP_BGRX };
        let img = if prov.is_null() {
            core::ptr::null()
        } else {
            CGImageCreate(width as usize, height as usize, 8, 32, stride as usize, h.cs.get(), info, prov, core::ptr::null(), false, 0)
        };
        let layer: Id = msg!(view, c"layer"; Id);
        if !layer.is_null() && !img.is_null() {
            let tx = class(c"CATransaction");
            msg!(tx, c"begin"; ());
            msg!(tx, c"setDisableActions:", true => bool; ());
            msg!(layer, c"setContents:", img as Id => Id; ());
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
        let px = core::slice::from_raw_parts(src, 4);
        edge_color(if order == 1 { [px[0], px[1], px[2]] } else { [px[2], px[1], px[0]] });
    }
    crate::present_account(t0, true);
}

/// CLOCK_MONOTONIC in µs (plat's time_us)
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_host_time_us() -> f64 {
    crate::time_us() as f64
}

/// web.rs's requests: REQUEST_IME (x, y, height: the app wants text while height > 0); every other
/// one with an id answers Unsupported at once (the clipboard, the pickers, sharing: AP5b)
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_host_request(kind: u32, id: u32, bytes: *const u8, len: usize) {
    if kind == crate::web::REQUEST_IME && len == 12 && !bytes.is_null() && on_main() {
        // SAFETY: the host copies the 12 bytes before returning (web.rs notice).
        let b = unsafe { core::slice::from_raw_parts(bytes, 12) };
        let height = i32::from_le_bytes([b[8], b[9], b[10], b[11]]);
        host().ime_visible.set(height > 0);
    }
    if id != 0 {
        jfgui_complete(id, -3);
    }
}

/// (apple_disp.rs open_url) a web page or another app's URL -> 0 / -1 (not on the main thread)
pub(crate) fn open_url(url: &str) -> i32 {
    if !on_main() {
        return -1;
    }
    // SAFETY: NSURL / UIApplication on the main thread; the string is released here.
    unsafe {
        let s = nsstring(url);
        let u: Id = msg!(class(c"NSURL"), c"URLWithString:", s => Id; Id);
        release(s);
        let app: Id = msg!(class(c"UIApplication"), c"sharedApplication"; Id);
        if u.is_null() || app.is_null() {
            return -1;
        }
        let options: Id = msg!(class(c"NSDictionary"), c"dictionary"; Id);
        msg!(app, c"openURL:options:completionHandler:", u => Id, options => Id, NIL => Id; ());
    }
    0
}

// ---------------------------------------------------------------- fingers, the pencil, a pointer

extern "C" fn view_touches_began(this: Id, _sel: Sel, touches: Id, event: Id) {
    touches_do(this, 0, touches, event);
}
extern "C" fn view_touches_moved(this: Id, _sel: Sel, touches: Id, event: Id) {
    touches_do(this, 1, touches, event);
}
extern "C" fn view_touches_ended(this: Id, _sel: Sel, touches: Id, event: Id) {
    touches_do(this, 2, touches, event);
}
extern "C" fn view_touches_cancelled(this: Id, _sel: Sel, touches: Id, event: Id) {
    touches_do(this, 3, touches, event);
}

/// phase 0 down, 1 move, 2 up, 3 cancel (jfgui_finger's)
fn touches_do(view: Id, phase: u32, touches: Id, event: Id) {
    let h = host();
    // SAFETY: CACurrentMediaTime is UITouch.timestamp's clock.
    let now = unsafe { CACurrentMediaTime() };
    for t in items(touches) {
        // SAFETY: a live UITouch of this event.
        let typ: isize = unsafe { msg!(t, c"type"; isize) };
        if typ == TOUCH_PENCIL || typ == TOUCH_POINTER {
            pointer(view, phase, t, event, typ == TOUCH_PENCIL);
            continue;
        }
        let mut slots = h.fingers.get();
        let slot = if phase == 0 {
            slots.iter().position(|&s| s == 0)
        } else {
            slots.iter().position(|&s| s == t as usize)
        };
        let Some(slot) = slot else { continue };
        if phase == 1 {
            // every sample since the last move, each with its own age: the velocity needs them
            // SAFETY: the coalesced touches of a live touch of this event.
            let samples: Id = unsafe { msg!(event, c"coalescedTouchesForTouch:", t => Id; Id) };
            let mut any = false;
            for c in items(samples) {
                finger(view, 1, slot, c, now);
                any = true;
            }
            if !any {
                finger(view, 1, slot, t, now);
            }
        } else {
            finger(view, phase, slot, t, now);
        }
        slots[slot] = if phase >= 2 { 0 } else { t as usize };
        h.fingers.set(slots);
    }
    jfgui_input(time_ms());
    // the last finger lifted after a gesture that reached a widget: the keyboard follows the app
    if phase >= 2 && h.fingers.get().iter().all(|&s| s == 0) && jfgui_touch_reached() != 0 {
        h.gesture.set(true);
        keyboard_follow_now();
    }
    request_frame();
}

fn finger(view: Id, phase: u32, slot: usize, t: Id, now: f64) {
    let s = host().scale.get();
    // SAFETY: a live UITouch (or one of its coalesced samples).
    let (p, ts): (NSPoint, f64) = unsafe { (msg!(t, c"locationInView:", view => Id; NSPoint), msg!(t, c"timestamp"; f64)) };
    let age = ((now - ts) * 1000.0).clamp(0.0, 60_000.0) as u32;
    jfgui_finger(phase, slot as u32, rnd(p.x * s), rnd(p.y * s), age);
}

/// the Apple Pencil (a pen: pressure, tilt) or the pointer of a trackpad / mouse (a mouse)
fn pointer(view: Id, phase: u32, t: Id, event: Id, pencil: bool) {
    let h = host();
    let s = h.scale.get();
    let samples: alloc::vec::Vec<Id> = if phase == 1 {
        // SAFETY: the coalesced touches of a live touch of this event.
        let all: alloc::vec::Vec<Id> = items(unsafe { msg!(event, c"coalescedTouchesForTouch:", t => Id; Id) }).collect();
        if all.is_empty() { alloc::vec![t] } else { all }
    } else {
        alloc::vec![t]
    };
    let btn = if pencil {
        1
    } else if phase == 0 {
        // SAFETY: buttonMask of the live event (UIEventButtonMaskSecondary = 2)
        let mask: isize = unsafe { msg!(event, c"buttonMask"; isize) };
        let b = if mask & 2 != 0 { 2 } else { 1 };
        h.pointer.set((t as usize, b));
        b
    } else {
        h.pointer.get().1
    };
    for c in samples {
        // SAFETY: a live UITouch (or one of its coalesced samples).
        let (p, data) = unsafe {
            if pencil {
                let p: NSPoint = msg!(c, c"preciseLocationInView:", view => Id; NSPoint);
                let force: f64 = msg!(c, c"force"; f64);
                let most: f64 = msg!(c, c"maximumPossibleForce"; f64);
                let pressure = if most > 0.0 { rnd(force / most * 65535.0) } else { -1 };
                // altitude: pi/2 upright; the azimuth's unit vector points where the pen leans
                let altitude: f64 = msg!(c, c"altitudeAngle"; f64);
                let az: CgVector = msg!(c, c"azimuthUnitVectorInView:", view => Id; CgVector);
                let tilt = 90.0 - altitude * 57.29578;
                (p, crate::evq_pen_data(pressure, PTR_PEN, rnd(tilt * az.dx), rnd(tilt * az.dy)))
            } else {
                (msg!(c, c"locationInView:", view => Id; NSPoint), 0)
            }
        };
        let (x, y) = (rnd(p.x * s), rnd(p.y * s));
        match phase {
            0 => {
                jfgui_event(E_MOVE as u32, 0, 0, 0, x, y, data);
                jfgui_event(E_DOWN as u32, btn, 0, 0, x, y, data);
            }
            1 => {
                jfgui_event(E_MOVE as u32, 0, 0, 0, x, y, data);
            }
            _ => {
                jfgui_event(E_UP as u32, btn, 0, 0, x, y, data);
            }
        }
    }
    if pencil {
        h.pen.set(if phase >= 2 { 0 } else { t as usize });
    } else if phase >= 2 {
        h.pointer.set((0, 1));
    }
}

/// the scene went inactive: fingers and pens the system took (no click, no fling)
fn cancel_gestures() {
    let h = host();
    let mut slots = h.fingers.get();
    for (i, s) in slots.iter_mut().enumerate() {
        if *s != 0 {
            jfgui_finger(3, i as u32, 0, 0, 0);
            *s = 0;
        }
    }
    h.fingers.set(slots);
    if h.pen.replace(0) != 0 {
        jfgui_event(E_UP as u32, 1, 0, 0, 0, 0, 0);
    }
    let (p, b) = h.pointer.replace((0, 1));
    if p != 0 {
        jfgui_event(E_UP as u32, b, 0, 0, 0, 0, 0);
    }
    jfgui_input(time_ms());
}

// ---------------------------------------------------------------- the keyboards

/// the gesture ended: when the app already wants text the keyboard comes now (else after the
/// next tick, in after_tick)
fn keyboard_follow_now() {
    let h = host();
    if h.ime_visible.get() {
        h.gesture.set(false);
        keyboard(true);
    }
}

/// the on-screen keyboard: JFView first responder (UIKeyInput) or the controller again
fn keyboard(show: bool) {
    let h = host();
    let view = h.view.get();
    if view.is_null() || h.ime_shown.get() == show {
        return;
    }
    h.ime_shown.set(show);
    // SAFETY: the live view and the window's root controller.
    unsafe {
        if show {
            msg!(view, c"becomeFirstResponder"; bool);
        } else {
            msg!(view, c"resignFirstResponder"; bool);
            let win = h.window.get();
            let vc: Id = if win.is_null() { NIL } else { msg!(win, c"rootViewController"; Id) };
            if !vc.is_null() {
                msg!(vc, c"becomeFirstResponder"; bool);
            }
        }
    }
}

fn key(vk: u32, mods: u32) {
    jfgui_event(E_KEY_DOWN as u32, 0, mods, 0, 0, 0, vk);
    jfgui_event(E_KEY_UP as u32, 0, mods, 0, 0, 0, vk);
}

/// the keyboard's text: Return is the key, a Tab comes as a key (pressesBegan), the rest is text
extern "C" fn view_insert_text(_this: Id, _sel: Sel, text: Id) {
    let s = string_of(text);
    match s.as_str() {
        "\n" | "\r" => key(13, 0),
        "\t" | "" => {}
        _ => {
            if inbox(s.as_bytes()) {
                jfgui_text(s.len() as u32);
            }
        }
    }
    jfgui_input(time_ms());
    request_frame();
}

/// the bar above the keyboard: "Done" on the right (the system's word for it, in the system's language)
extern "C" fn view_accessory(this: Id, _sel: Sel) -> Id {
    let h = host();
    if h.accessory.get().is_null() {
        // SAFETY: UIKit objects made here on the main thread; the bar is kept for the process.
        unsafe {
            let bar: Id = msg!(class(c"UIToolbar"), c"alloc"; Id);
            let bar: Id = msg!(bar, c"initWithFrame:", NSRect::new(0.0, 0.0, 320.0, 44.0) => NSRect; Id);
            let item = |kind: isize, target: Id, action: Sel| -> Id {
                let i: Id = msg!(class(c"UIBarButtonItem"), c"alloc"; Id);
                msg!(i, c"initWithBarButtonSystemItem:target:action:", kind => isize, target => Id, action => Sel; Id)
            };
            // UIBarButtonSystemItemFlexibleSpace 5, UIBarButtonSystemItemDone 0
            let items = [item(5, NIL, core::ptr::null()), item(0, this, super::objc::sel(c"jfDone:"))];
            let arr: Id = msg!(class(c"NSArray"), c"arrayWithObjects:count:", items.as_ptr() => *const Id, 2usize => usize; Id);
            msg!(bar, c"setItems:", arr => Id; ());
            msg!(bar, c"sizeToFit"; ());
            for i in items {
                release(i);
            }
            h.accessory.set(bar);
        }
    }
    h.accessory.get()
}

/// "Done": the keyboard goes, the text field keeps the focus (a tap into it brings it back)
extern "C" fn view_done(_this: Id, _sel: Sel, _item: Id) {
    host().gesture.set(false);
    keyboard(false);
}

extern "C" fn view_delete_backward(_this: Id, _sel: Sel) {
    key(8, 0);
    jfgui_input(time_ms());
    request_frame();
}

extern "C" fn vc_presses_began(_this: Id, _sel: Sel, presses: Id, _event: Id) {
    presses_do(presses, true);
}
extern "C" fn vc_presses_ended(_this: Id, _sel: Sel, presses: Id, _event: Id) {
    presses_do(presses, false);
}

/// a hardware keyboard's keys: E_KEY_DOWN / UP with the Windows key code; while the on-screen
/// keyboard's text system has the focus, the keys it types (letters, digits, Return, Backspace,
/// Space without Command or Control) are its own
fn presses_do(presses: Id, down: bool) {
    let shown = host().ime_shown.get();
    let mut any = false;
    for press in items(presses) {
        // SAFETY: the UIKey of a live UIPress (nil for a press that is no key).
        let (hid, flags) = unsafe {
            let k: Id = msg!(press, c"key"; Id);
            if k.is_null() {
                continue;
            }
            (msg!(k, c"keyCode"; isize), msg!(k, c"modifierFlags"; isize))
        };
        let vk = vk_of(hid);
        if vk == 0 {
            continue;
        }
        let mods = mods_of(flags);
        let typed = matches!(hid, 4..=40 | 42 | 44) && mods & (MOD_CTRL | MOD_SUPER) == 0;
        if shown && typed {
            continue;
        }
        jfgui_event((if down { E_KEY_DOWN } else { E_KEY_UP }) as u32, 0, mods as u32, 0, 0, 0, vk);
        any = true;
    }
    if any {
        jfgui_input(time_ms());
        request_frame();
    }
}

fn mods_of(flags: isize) -> u8 {
    let mut m = 0;
    if flags & KEY_SHIFT != 0 {
        m |= MOD_SHIFT;
    }
    if flags & KEY_COMMAND != 0 {
        m |= MOD_CTRL;
    }
    if flags & KEY_CONTROL != 0 {
        m |= MOD_SUPER;
    }
    if flags & KEY_OPTION != 0 {
        m |= MOD_ALT;
    }
    if flags & KEY_CAPS != 0 {
        m |= MOD_CAPS;
    }
    m
}

/// a USB HID usage (UIKeyboardHIDUsage) -> the Windows key code (0: none)
fn vk_of(hid: isize) -> u32 {
    match hid {
        4..=29 => 65 + (hid - 4) as u32,    // A..Z
        30..=38 => 49 + (hid - 30) as u32,  // 1..9
        39 => 48,                           // 0
        40 => 13,                           // Return
        41 => 27,                           // Escape
        42 => 8,                            // Backspace
        43 => 9,                            // Tab
        44 => 32,                           // Space
        58..=69 => 112 + (hid - 58) as u32, // F1..F12
        73 => 45,                           // Insert
        74 => 36,                           // Home
        75 => 33,                           // Page Up
        76 => 46,                           // Delete
        77 => 35,                           // End
        78 => 34,                           // Page Down
        79 => 39,                           // Right
        80 => 37,                           // Left
        81 => 40,                           // Down
        82 => 38,                           // Up
        _ => 0,
    }
}
