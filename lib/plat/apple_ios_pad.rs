// apple_ios_pad -- the iPad's parts of iOS's host (docs/APPLE.md AP9): the app's menus in the menu bar
// of iPadOS, the pointer of a trackpad or mouse without a button, its wheel and pinch, and the test
// aids of plattest --test ipad
//
//   is_pad() -> bool                 the device is an iPad (UIUserInterfaceIdiomPad)
//   sysmenu_supported() / sysmenu_window_row() / sysmenu_set()   plat::sysmenu on the iPad
//   app_methods() / view_methods()   (apple_ios.rs register) the app delegate's menu methods, JFView's
//                                    actions of the recognizers
//   add_recognizers(view)            (apple_ios.rs vc_load_view) hover, wheel and pinch on JFView
//   ios_pad_*                        (tests, any thread) window sizes, the keyboard, turning, the menus
//                                    built with a stand-in for UIKit's builder, the pointer's paths
//                                    without a pointer; on the main thread
//
// The menus: UIKit asks the app delegate (a UIResponder) for the main menu (buildMenuWithBuilder:).
// With menus of the app the system's File, Edit, Format and View menus go and the app's follow the
// application menu, before Window, as on the Mac (apple_mac_app.rs).  Every row is a UICommand, with
// a key equivalent a UIKeyCommand, whose action jfSysMenu: goes up the responder chain to the app
// delegate; its propertyList is the row's id.  Separators become inline groups (UIMenu has none).
// sysmenu_set asks for a rebuild; iPadOS builds when it needs the menu (the bar revealed, Command
// held), never without input.  iPadOS 26 shows the menus in its menu bar (a swipe down from the top
// edge or the pointer there reveals it), every iPadOS shows the rows with a key while Command is
// held.  The window keeps its own menu row (sysmenu_window_row, the user's decision of 2026-11-19):
// the bar of iPadOS hides until it is revealed, the row stays the way for the finger.  The iPhone
// has no menu bar: unsupported there.
//
// The pointer: a UIHoverGestureRecognizer moves the mouse without a button (the Apple Pencil's hover
// on the iPads that have it: a pen without pressure), a UIPanGestureRecognizer that takes no touches,
// only scroll events, turns a trackpad's two fingers and a mouse wheel into E_WHEEL (WHEEL_PIXELS,
// as the Mac's trackpad), a UIPinchGestureRecognizer for the indirect pointer alone turns a
// trackpad's pinch into Ctrl + wheel (as macOS).  None of them takes or delays a finger.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};

use rt::sync::SpinLock;

use super::super::objc::{Id, NIL, NSPoint, NSRect, Sel, class, msg, nsstring, release, sel, string_of};
use super::a11y_ios::on_main;
use super::{KEY_COMMAND, KEY_CONTROL, KEY_OPTION, KEY_SHIFT, host, input_now, mods_of, rnd, time_ms};
use crate::sysmenu::{SysMenuItem, TI_CHECKED, TI_DISABLED, TI_SEPARATOR};
use crate::web::{jfgui_event, jfgui_input};
use crate::{E_LEAVE, E_MOVE, E_WHEEL, MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_SUPER, PTR_PEN, WHEEL_PIXELS};

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    static UIMenuApplication: Id;
    static UIMenuFile: Id;
    static UIMenuEdit: Id;
    static UIMenuFormat: Id;
    static UIMenuView: Id;
    static UIMenuWindow: Id;
    static UIMenuHelp: Id;
    static UIKeyInputUpArrow: Id;
    static UIKeyInputDownArrow: Id;
    static UIKeyInputLeftArrow: Id;
    static UIKeyInputRightArrow: Id;
    static UIKeyInputEscape: Id;
    static UIKeyInputHome: Id;
    static UIKeyInputEnd: Id;
    static UIKeyInputPageUp: Id;
    static UIKeyInputPageDown: Id;
    static UIKeyInputDelete: Id;
    static UIKeyInputF1: Id;
    static UIKeyInputF2: Id;
    static UIKeyInputF3: Id;
    static UIKeyInputF4: Id;
    static UIKeyInputF5: Id;
    static UIKeyInputF6: Id;
    static UIKeyInputF7: Id;
    static UIKeyInputF8: Id;
    static UIKeyInputF9: Id;
    static UIKeyInputF10: Id;
    static UIKeyInputF11: Id;
    static UIKeyInputF12: Id;
}

// UIMenuOptionsDisplayInline, UIMenuElementAttributesDisabled, UIMenuElementStateOn
const MENU_INLINE: usize = 1;
const ATTR_DISABLED: usize = 1;
const STATE_ON: isize = 1;
// UIGestureRecognizerState
const GR_BEGAN: isize = 1;
const GR_CHANGED: isize = 2;
// UITouchTypeIndirectPointer, UIScrollTypeMaskAll
const TOUCH_INDIRECT_POINTER: isize = 3;
const SCROLL_ALL: usize = 3;
const ID_PREFIX: &str = "org.jflib.sysmenu.";

/// 0 not asked yet, 1 an iPhone, 2 an iPad
static IDIOM: AtomicU8 = AtomicU8::new(0);
/// the main menu's builds (buildMenuWithBuilder: of the main system)
static BUILDS: AtomicU32 = AtomicU32::new(0);
/// the app's menus as the last build left them in UIKit's main menu (read back from the builder),
/// whether the system's File / Edit / Format / View menus went, the top menus (retained) to choose
/// rows from (tests)
static BUILT: SpinLock<Vec<(String, Vec<IosMenuRow>)>> = SpinLock::new(Vec::new());
static SYSTEM_GONE: AtomicBool = AtomicBool::new(false);
static TOPS: SpinLock<Vec<usize>> = SpinLock::new(Vec::new());
/// the pointer is over the view without a button (the hover recognizer's), the wheel's and the
/// pinch's fractions not sent yet (main thread)
static HOVER: AtomicBool = AtomicBool::new(false);
static WHEEL: SpinLock<(f64, f64, f64)> = SpinLock::new((0.0, 0.0, 0.0));
/// the last E_WHEEL sent: btn, mods, data (tests)
static LAST_WHEEL: SpinLock<(u8, u8, u32)> = SpinLock::new((0, 0, 0));

/// the device is an iPad (UIUserInterfaceIdiomPad): asked once, on the main thread
pub(crate) fn is_pad() -> bool {
    match IDIOM.load(Ordering::Acquire) {
        1 => false,
        2 => true,
        _ => {
            if !super::on_main() {
                return false; // (asked again from the main thread)
            }
            // SAFETY: a plain UIKit query on the main thread.
            let pad = unsafe {
                let dev: Id = msg!(class(c"UIDevice"), c"currentDevice"; Id);
                !dev.is_null() && msg!(dev, c"userInterfaceIdiom"; isize) == 1
            };
            IDIOM.store(if pad { 2 } else { 1 }, Ordering::Release);
            pad
        }
    }
}

// ---------------------------------------------------------------- the menus (sysmenu.rs)

/// an iPad under UIKit's host shows the app's menus in its menu bar (and while Command is held)
pub(crate) fn sysmenu_supported() -> bool {
    super::hosted() && is_pad() && !crate::getenv("jfgui_SYSMENU").is_some_and(|v| v.starts_with('0'))
}

/// the window keeps its own menu row as well: iPadOS's bar hides until it is revealed
pub(crate) fn sysmenu_window_row() -> bool {
    sysmenu_supported()
}

/// (sysmenu_set, the UI thread = the main thread) the menus changed: UIKit builds the main menu anew
pub(crate) fn sysmenu_set() {
    if !super::on_main() {
        return;
    }
    REBUILDS.fetch_add(1, Ordering::AcqRel);
    // SAFETY: the main menu system, on the main thread.
    unsafe {
        let sys: Id = msg!(class(c"UIMenuSystem"), c"mainSystem"; Id);
        if !sys.is_null() {
            msg!(sys, c"setNeedsRebuild"; ());
        }
    }
}

/// the app delegate's methods: the main menu, the rows' action
pub(super) fn app_methods() -> [(&'static core::ffi::CStr, *const c_void, &'static core::ffi::CStr); 2] {
    [
        (c"buildMenuWithBuilder:", app_build_menu as *const c_void, c"v@:@"),
        (c"jfSysMenu:", app_menu_chosen as *const c_void, c"v@:@"),
    ]
}

/// a row was chosen (its menu, its key equivalent): the id in its propertyList
extern "C" fn app_menu_chosen(_this: Id, _sel: Sel, sender: Id) {
    let id = command_id(sender);
    if id > 0 {
        crate::sysmenu::push(id);
        input_now();
    }
}

/// the id a row's command carries (0: none)
fn command_id(cmd: Id) -> u32 {
    if cmd.is_null() {
        return 0;
    }
    // SAFETY: the propertyList of a UICommand (an NSNumber of ours, or nil).
    unsafe {
        if !msg!(cmd, c"respondsToSelector:", sel(c"propertyList") => Sel; bool) {
            return 0;
        }
        let n: Id = msg!(cmd, c"propertyList"; Id);
        if n.is_null() || !msg!(n, c"isKindOfClass:", class(c"NSNumber") => Id; bool) {
            return 0;
        }
        msg!(n, c"unsignedIntValue"; u32)
    }
}

/// UIKit builds the main menu (and context menus: not ours)
extern "C" fn app_build_menu(_this: Id, _sel: Sel, builder: Id) {
    // SAFETY: the builder UIKit hands over, on the main thread, for this call only.
    unsafe {
        let sys: Id = msg!(builder, c"system"; Id);
        if sys.is_null() || sys != msg!(class(c"UIMenuSystem"), c"mainSystem"; Id) {
            return;
        }
        BUILDS.fetch_add(1, Ordering::AcqRel);
        let menus = crate::sysmenu::current();
        let mut tops = TOPS.lock();
        for m in tops.drain(..) {
            release(m as Id);
        }
        if menus.is_empty() || !sysmenu_supported() {
            // the system's menus stay as they are
            BUILT.lock().clear();
            SYSTEM_GONE.store(false, Ordering::Release);
            return;
        }
        for id in [UIMenuFile, UIMenuEdit, UIMenuFormat, UIMenuView] {
            if !msg!(builder, c"menuForIdentifier:", id => Id; Id).is_null() {
                msg!(builder, c"removeMenuForIdentifier:", id => Id; ());
            }
        }
        let after_app = !msg!(builder, c"menuForIdentifier:", UIMenuApplication => Id; Id).is_null();
        let mut keys = Vec::new();
        let mut idents: Vec<Id> = Vec::new();
        for (k, m) in menus.iter().enumerate() {
            let ident = nsstring(&format!("{ID_PREFIX}{k}"));
            let menu = make_menu(&m.label, ident, 0, children(&m.sub, &mut keys));
            if let Some(&prev) = idents.last() {
                msg!(builder, c"insertSiblingMenu:afterMenuForIdentifier:", menu => Id, prev => Id; ());
            } else if after_app {
                msg!(builder, c"insertSiblingMenu:afterMenuForIdentifier:", menu => Id, UIMenuApplication => Id; ());
            } else {
                msg!(builder, c"insertSiblingMenu:beforeMenuForIdentifier:", menu => Id, UIMenuWindow => Id; ());
            }
            idents.push(ident);
        }
        // what UIKit holds now (the tests read it back)
        let mut built = Vec::new();
        for (k, &ident) in idents.iter().enumerate() {
            let menu: Id = msg!(builder, c"menuForIdentifier:", ident => Id; Id);
            if !menu.is_null() {
                msg!(menu, c"retain"; Id);
                tops.push(menu as usize);
                let mut rows = Vec::new();
                describe(menu, &mut rows, false);
                built.push((menus[k].label.clone(), rows));
            }
            release(ident);
        }
        *BUILT.lock() = built;
        let gone = [UIMenuFile, UIMenuEdit, UIMenuFormat, UIMenuView].iter().all(|&id| msg!(builder, c"menuForIdentifier:", id => Id; Id).is_null());
        SYSTEM_GONE.store(gone, Ordering::Release);
    }
}

/// a menu: title, identifier (nil: UIKit makes one), UIMenuOptions, children (an NSArray) -> autoreleased
unsafe fn make_menu(title: &str, ident: Id, options: usize, children: Id) -> Id {
    let t = nsstring(title);
    // SAFETY: UIMenu's class method with its exact signature; the title is released after it.
    let m: Id = unsafe {
        msg!(class(c"UIMenu"), c"menuWithTitle:image:identifier:options:children:", t => Id, NIL => Id, ident => Id, options => usize,
             children => Id; Id)
    };
    release(t);
    m
}

/// objects -> an autoreleased NSArray
fn array(v: &[Id]) -> Id {
    // SAFETY: v holds live objects; the array retains them.
    unsafe { msg!(class(c"NSArray"), c"arrayWithObjects:count:", v.as_ptr() => *const Id, v.len() => usize; Id) }
}

/// the rows of a menu: groups between separators (each an inline menu when there are several),
/// submenus, commands
unsafe fn children(items: &[SysMenuItem], keys: &mut Vec<(String, isize)>) -> Id {
    let mut groups: Vec<Vec<Id>> = alloc::vec![Vec::new()];
    for it in items {
        if it.flags & TI_SEPARATOR != 0 {
            if groups.last().is_some_and(|g| !g.is_empty()) {
                groups.push(Vec::new());
            }
            continue;
        }
        // SAFETY: (the callers' contract) on the main thread.
        let el = unsafe {
            if it.sub.is_empty() { command(it, keys) } else { make_menu(&it.label, NIL, 0, children(&it.sub, keys)) }
        };
        if !el.is_null() {
            groups.last_mut().expect("a group").push(el);
        }
    }
    groups.retain(|g| !g.is_empty());
    if groups.len() <= 1 {
        return array(groups.first().map_or(&[][..], |g| g.as_slice()));
    }
    // SAFETY: as above.
    let inl: Vec<Id> = groups.iter().map(|g| unsafe { make_menu("", NIL, MENU_INLINE, array(g)) }).collect();
    array(&inl)
}

/// a row: a UIKeyCommand with its key equivalent (one per key and modifiers: a second row with the
/// same key gets none), else a UICommand; disabled / checked as the list says -> autoreleased
unsafe fn command(it: &SysMenuItem, keys: &mut Vec<(String, isize)>) -> Id {
    let flags = key_flags(it.mods);
    let input = if it.key.is_empty() || keys.iter().any(|(k, f)| *k == it.key && *f == flags) { None } else { key_input(&it.key) };
    let title = nsstring(&it.label);
    let action = sel(c"jfSysMenu:");
    // SAFETY: the class methods with their exact signatures; NSNumber and the commands come back
    // autoreleased; title and an input of ours are released after them.
    let cmd: Id = unsafe {
        let num: Id = msg!(class(c"NSNumber"), c"numberWithUnsignedInt:", it.id => u32; Id);
        let cmd: Id = match input {
            Some((inp, own)) => {
                keys.push((it.key.clone(), flags));
                let c: Id = msg!(class(c"UIKeyCommand"), c"commandWithTitle:image:action:input:modifierFlags:propertyList:", title => Id,
                                 NIL => Id, action => Sel, inp => Id, flags => isize, num => Id; Id);
                if own {
                    release(inp);
                }
                c
            }
            None => msg!(class(c"UICommand"), c"commandWithTitle:image:action:propertyList:", title => Id, NIL => Id, action => Sel,
                         num => Id; Id),
        };
        if !cmd.is_null() {
            if it.flags & TI_DISABLED != 0 || it.id == 0 {
                msg!(cmd, c"setAttributes:", ATTR_DISABLED => usize; ());
            }
            if it.flags & TI_CHECKED != 0 {
                msg!(cmd, c"setState:", STATE_ON => isize; ());
            }
        }
        cmd
    };
    release(title);
    cmd
}

/// MOD_* of a key equivalent -> UIKeyModifierFlags (MOD_CTRL is Command, MOD_SUPER Control)
fn key_flags(mods: u32) -> isize {
    let m = mods as u8;
    (if m & MOD_CTRL != 0 { KEY_COMMAND } else { 0 })
        | (if m & MOD_ALT != 0 { KEY_OPTION } else { 0 })
        | (if m & MOD_SHIFT != 0 { KEY_SHIFT } else { 0 })
        | (if m & MOD_SUPER != 0 { KEY_CONTROL } else { 0 })
}

/// sysmenu's key (a character; function and cursor keys as AppKit's private-use characters) ->
/// UIKeyCommand's input: (string, made here and to release) or UIKit's constant; None: no key here
fn key_input(key: &str) -> Option<(Id, bool)> {
    let c = key.chars().next()? as u32;
    // SAFETY: UIKit's exported NSString constants.
    let named = unsafe {
        match c {
            0xF700 => UIKeyInputUpArrow,
            0xF701 => UIKeyInputDownArrow,
            0xF702 => UIKeyInputLeftArrow,
            0xF703 => UIKeyInputRightArrow,
            0xF704..=0xF70F => [UIKeyInputF1, UIKeyInputF2, UIKeyInputF3, UIKeyInputF4, UIKeyInputF5, UIKeyInputF6, UIKeyInputF7, UIKeyInputF8,
                                UIKeyInputF9, UIKeyInputF10, UIKeyInputF11, UIKeyInputF12][(c - 0xF704) as usize],
            0xF728 => UIKeyInputDelete,
            0xF729 => UIKeyInputHome,
            0xF72B => UIKeyInputEnd,
            0xF72C => UIKeyInputPageUp,
            0xF72D => UIKeyInputPageDown,
            0x1B => UIKeyInputEscape,
            0xF727 | 0xF600..=0xF8FF => return None, // Insert and the rest: no key on the iPad
            _ => NIL,
        }
    };
    if !named.is_null() {
        return Some((named, false));
    }
    Some((nsstring(key), true))
}

/// one row as UIKit holds it (tests)
#[derive(Clone, Debug, Default)]
pub struct IosMenuRow {
    pub label: String,
    /// the id of its command (0: a submenu or a row without action)
    pub id: u32,
    pub disabled: bool,
    pub checked: bool,
    /// the key equivalent's input (UIKit's names for special keys) and its UIKeyModifierFlags
    pub key: String,
    pub key_flags: isize,
    /// the rows of a submenu (0: none)
    pub sub: usize,
    /// a separator (an inline group's border) comes before it
    pub sep_before: bool,
}

/// the rows of a menu read back, inline groups flattened with sep_before
fn describe(menu: Id, rows: &mut Vec<IosMenuRow>, mut sep: bool) {
    // SAFETY: a live UIMenu and its children (UIMenu, UICommand, UIKeyCommand) on the main thread.
    unsafe {
        let ch: Id = msg!(menu, c"children"; Id);
        let n = if ch.is_null() { 0 } else { msg!(ch, c"count"; usize) };
        for i in 0..n {
            let e: Id = msg!(ch, c"objectAtIndex:", i => usize; Id);
            if msg!(e, c"isKindOfClass:", class(c"UIMenu") => Id; bool) {
                let opts: usize = msg!(e, c"options"; usize);
                if opts & MENU_INLINE != 0 {
                    describe(e, rows, !rows.is_empty());
                    sep = false;
                    continue;
                }
                let sub: Id = msg!(e, c"children"; Id);
                rows.push(IosMenuRow {
                    label: string_of(msg!(e, c"title"; Id)),
                    sub: if sub.is_null() { 0 } else { msg!(sub, c"count"; usize) },
                    sep_before: sep,
                    ..Default::default()
                });
            } else {
                let keyed = msg!(e, c"isKindOfClass:", class(c"UIKeyCommand") => Id; bool);
                rows.push(IosMenuRow {
                    label: string_of(msg!(e, c"title"; Id)),
                    id: command_id(e),
                    disabled: msg!(e, c"attributes"; usize) & ATTR_DISABLED != 0,
                    checked: msg!(e, c"state"; isize) == STATE_ON,
                    key: if keyed { string_of(msg!(e, c"input"; Id)) } else { String::new() },
                    key_flags: if keyed { msg!(e, c"modifierFlags"; isize) } else { 0 },
                    sep_before: sep,
                    sub: 0,
                });
            }
            sep = false;
        }
    }
}

/// the command of row `row` of top menu `menu` (inline groups flattened, as describe), or NIL
fn find_command(menu: Id, row: &mut usize) -> Id {
    // SAFETY: as describe.
    unsafe {
        let ch: Id = msg!(menu, c"children"; Id);
        let n = if ch.is_null() { 0 } else { msg!(ch, c"count"; usize) };
        for i in 0..n {
            let e: Id = msg!(ch, c"objectAtIndex:", i => usize; Id);
            if msg!(e, c"isKindOfClass:", class(c"UIMenu") => Id; bool) {
                if msg!(e, c"options"; usize) & MENU_INLINE != 0 {
                    let found = find_command(e, row);
                    if !found.is_null() {
                        return found;
                    }
                    continue;
                }
                if *row == 0 {
                    return NIL; // a submenu: no command
                }
                *row -= 1;
                continue;
            }
            if *row == 0 {
                return e;
            }
            *row -= 1;
        }
        NIL
    }
}

// ---------------------------------------------------------------- the pointer without a button

/// JFView's actions of the recognizers
pub(super) fn view_methods() -> [(&'static core::ffi::CStr, *const c_void, &'static core::ffi::CStr); 3] {
    [
        (c"jfHover:", view_hover as *const c_void, c"v@:@"),
        (c"jfScroll:", view_scroll as *const c_void, c"v@:@"),
        (c"jfPinch:", view_pinch as *const c_void, c"v@:@"),
    ]
}

/// hover, the wheel and a trackpad's pinch on JFView; none takes, delays or cancels a touch
pub(super) fn add_recognizers(view: Id) {
    // SAFETY: UIKit on the main thread; the view retains its recognizers.
    unsafe {
        let empty = array(&[]);
        let pointer: Id = msg!(class(c"NSNumber"), c"numberWithInteger:", TOUCH_INDIRECT_POINTER => isize; Id);
        let only_pointer = array(&[pointer]);
        for (cls, action) in [(c"UIHoverGestureRecognizer", c"jfHover:"), (c"UIPanGestureRecognizer", c"jfScroll:"), (c"UIPinchGestureRecognizer", c"jfPinch:")] {
            let c = class(cls);
            if c.is_null() {
                continue;
            }
            let r: Id = msg!(c, c"alloc"; Id);
            let r: Id = msg!(r, c"initWithTarget:action:", view => Id, sel(action) => Sel; Id);
            if r.is_null() {
                continue;
            }
            msg!(r, c"setCancelsTouchesInView:", false => bool; ());
            msg!(r, c"setDelaysTouchesBegan:", false => bool; ());
            msg!(r, c"setDelaysTouchesEnded:", false => bool; ());
            if action == c"jfScroll:" {
                // scroll events only (a trackpad's two fingers, a mouse wheel), no finger
                msg!(r, c"setAllowedScrollTypesMask:", SCROLL_ALL => usize; ());
                msg!(r, c"setAllowedTouchTypes:", empty => Id; ());
            } else if action == c"jfPinch:" {
                msg!(r, c"setAllowedTouchTypes:", only_pointer => Id; ());
            }
            msg!(view, c"addGestureRecognizer:", r => Id; ());
            release(r);
        }
    }
}

/// a button of the pointer or the pencil is down (touches_do has them): the hover waits
fn pressed() -> bool {
    let h = host();
    h.pen.get() != 0 || h.pointer.get().0 != 0
}

extern "C" fn view_hover(this: Id, _sel: Sel, rec: Id) {
    // SAFETY: the live recognizer of our view.
    let (state, p, pen) = unsafe {
        let state: isize = msg!(rec, c"state"; isize);
        let p: NSPoint = msg!(rec, c"locationInView:", this => Id; NSPoint);
        // (iPadOS 16.1) the Apple Pencil's height above the screen; 0 for a trackpad's pointer
        let z: f64 = if msg!(rec, c"respondsToSelector:", sel(c"zOffset") => Sel; bool) { msg!(rec, c"zOffset"; f64) } else { 0.0 };
        let pen = if z > 0.0 && msg!(rec, c"respondsToSelector:", sel(c"altitudeAngle") => Sel; bool) {
            let altitude: f64 = msg!(rec, c"altitudeAngle"; f64);
            let az: super::CgVector = msg!(rec, c"azimuthUnitVectorInView:", this => Id; super::CgVector);
            let tilt = 90.0 - altitude * 57.29578;
            Some((rnd(tilt * az.dx), rnd(tilt * az.dy)))
        } else if z > 0.0 {
            Some((0, 0))
        } else {
            None
        };
        (state, p, pen)
    };
    hover(state, p.x, p.y, pen);
}

/// the hover's state (UIGestureRecognizerState), its point in points, the pencil's tilt (None: the
/// pointer of a trackpad or mouse)
fn hover(state: isize, x: f64, y: f64, pen: Option<(i32, i32)>) {
    if pressed() {
        return; // touches_do moves it while a button is down
    }
    let s = host().scale.get();
    if state == GR_BEGAN || state == GR_CHANGED {
        let data = pen.map_or(0, |(tx, ty)| crate::evq_pen_data(0, PTR_PEN, tx, ty));
        HOVER.store(true, Ordering::Relaxed);
        jfgui_event(E_MOVE as u32, 0, 0, 0, rnd(x * s), rnd(y * s), data);
    } else if HOVER.swap(false, Ordering::Relaxed) {
        jfgui_event(E_LEAVE as u32, 0, 0, 0, 0, 0, 0);
    } else {
        return;
    }
    input_now();
}

extern "C" fn view_scroll(this: Id, _sel: Sel, rec: Id) {
    // SAFETY: the live recognizer of our view; its translation goes back to zero after each step.
    let (d, p, flags) = unsafe {
        if msg!(rec, c"numberOfTouches"; usize) != 0 {
            return; // (no finger is allowed; never one either way)
        }
        let d: NSPoint = msg!(rec, c"translationInView:", this => Id; NSPoint);
        msg!(rec, c"setTranslation:inView:", NSPoint::default() => NSPoint, this => Id; ());
        let p: NSPoint = msg!(rec, c"locationInView:", this => Id; NSPoint);
        let flags: isize = if msg!(rec, c"respondsToSelector:", sel(c"modifierFlags") => Sel; bool) { msg!(rec, c"modifierFlags"; isize) } else { 0 };
        (d, p, flags)
    };
    scroll(d.x, d.y, p.x, p.y, flags);
}

/// the pointer is at (x, y) points (a wheel or a pinch there: the UI turns what lies below it)
fn pointer_at(x: f64, y: f64) {
    if pressed() {
        return;
    }
    let s = host().scale.get();
    HOVER.store(true, Ordering::Relaxed);
    jfgui_event(E_MOVE as u32, 0, 0, 0, rnd(x * s), rnd(y * s), 0);
}

/// a scroll step: the content follows the fingers by (dx, dy) points at (x, y) -> E_WHEEL in
/// physical pixels (up and right positive: fingers down scroll up), the rest waits
fn scroll(dx: f64, dy: f64, x: f64, y: f64, flags: isize) {
    let s = host().scale.get();
    let (wy, wx) = {
        let mut w = WHEEL.lock();
        w.0 += dy * s;
        w.1 -= dx * s;
        let (wy, wx) = (w.0 as i32 as f64, w.1 as i32 as f64);
        w.0 -= wy;
        w.1 -= wx;
        (wy, wx)
    };
    if wy == 0.0 && wx == 0.0 {
        return;
    }
    let data = (wy.clamp(-32768.0, 32767.0) as i16 as u16 as u32) | ((wx.clamp(-32768.0, 32767.0) as i16 as u16 as u32) << 16);
    let mods = mods_of(flags);
    *LAST_WHEEL.lock() = (WHEEL_PIXELS, mods, data);
    pointer_at(x, y);
    jfgui_event(E_WHEEL as u32, WHEEL_PIXELS as u32, mods as u32, 0, rnd(x * s), rnd(y * s), data);
    input_now();
}

extern "C" fn view_pinch(this: Id, _sel: Sel, rec: Id) {
    // SAFETY: the live recognizer of our view; its scale goes back to 1 after each step.
    let (scale, p, flags) = unsafe {
        let scale: f64 = msg!(rec, c"scale"; f64);
        msg!(rec, c"setScale:", 1.0f64 => f64; ());
        let p: NSPoint = msg!(rec, c"locationInView:", this => Id; NSPoint);
        let flags: isize = if msg!(rec, c"respondsToSelector:", sel(c"modifierFlags") => Sel; bool) { msg!(rec, c"modifierFlags"; isize) } else { 0 };
        (scale, p, flags)
    };
    pinch(scale - 1.0, p.x, p.y, flags);
}

/// a trackpad's pinch: Ctrl + wheel, 120 per 0.2 of magnification (zoom in: up), as the Mac's
fn pinch(m: f64, x: f64, y: f64, flags: isize) {
    let s = host().scale.get();
    let steps = {
        let mut w = WHEEL.lock();
        w.2 += m * 600.0;
        let t = w.2 as i32 as f64;
        w.2 -= t;
        t
    };
    if steps == 0.0 {
        return;
    }
    let data = steps.clamp(-32768.0, 32767.0) as i16 as u16 as u32;
    let mods = mods_of(flags) | MOD_CTRL;
    *LAST_WHEEL.lock() = (0, mods, data);
    pointer_at(x, y);
    jfgui_event(E_WHEEL as u32, 0, mods as u32, 0, rnd(x * s), rnd(y * s), data);
    input_now();
}

// ---------------------------------------------------------------- test aids (plattest --test ipad)

/// what the host shows (ios_pad_view): sizes in points unless named px
#[derive(Clone, Copy, Debug, Default)]
pub struct IosPadView {
    /// the device is an iPad
    pub pad: bool,
    /// the window scene's bounds (the window iPadOS gave the app), the window's, JFView's
    pub scene: (f64, f64),
    pub window: (f64, f64),
    pub view: (f64, f64),
    /// the safe area's insets of the root view: top, left, bottom, right
    pub safe: (f64, f64, f64, f64),
    /// the canvas in px as last sent (E_RESIZE) and its px per point
    pub canvas: (i32, i32),
    pub scale: f64,
    /// the screen's own scale
    pub screen_scale: f64,
    /// the interface orientation of the scene (UIInterfaceOrientation: 1 portrait .. 4 landscape)
    pub orientation: isize,
    /// the app's menus in the window row stay (sysmenu_window_row)
    pub window_row: bool,
    /// the size of the last picture shown (px)
    pub shown: (i32, i32),
    /// the on-screen keyboard is asked for (JFView first responder)
    pub keyboard: bool,
    /// the last E_WHEEL the wheel or the pinch sent: btn, mods, data
    pub last_wheel: (u8, u8, u32),
}

/// (tests, any thread) the host's sizes now; None without UIKit's host
pub fn ios_pad_view() -> Option<IosPadView> {
    on_main(|| {
        let h = host();
        let (win, root, view) = (h.window.get(), h.root.get(), h.view.get());
        if win.is_null() || view.is_null() || root.is_null() {
            return None;
        }
        // SAFETY: the live window, its scene, the views, on the main thread.
        unsafe {
            let scene: Id = msg!(win, c"windowScene"; Id);
            let sb: NSRect = if scene.is_null() { NSRect::default() } else { msg!(msg!(scene, c"coordinateSpace"; Id), c"bounds"; NSRect) };
            let wb: NSRect = msg!(win, c"bounds"; NSRect);
            let vb: NSRect = msg!(view, c"bounds"; NSRect);
            #[repr(C)]
            #[derive(Clone, Copy)]
            struct Insets {
                top: f64,
                left: f64,
                bottom: f64,
                right: f64,
            }
            let ins: Insets = msg!(root, c"safeAreaInsets"; Insets);
            let screen: Id = msg!(win, c"screen"; Id);
            let (cw, ch, _) = h.size.get();
            Some(IosPadView {
                pad: is_pad(),
                scene: (sb.size.w, sb.size.h),
                window: (wb.size.w, wb.size.h),
                view: (vb.size.w, vb.size.h),
                safe: (ins.top, ins.left, ins.bottom, ins.right),
                canvas: (cw, ch),
                scale: h.scale.get(),
                screen_scale: if screen.is_null() { 0.0 } else { msg!(screen, c"scale"; f64) },
                orientation: if scene.is_null() { 0 } else { msg!(scene, c"interfaceOrientation"; isize) },
                window_row: sysmenu_window_row(),
                shown: h.shown.get(),
                keyboard: h.ime_shown.get(),
                last_wheel: *LAST_WHEEL.lock(),
            })
        }
    })
}

/// (tests, any thread) the window's frame inside its scene in points, as Split View, Slide Over or
/// Stage Manager would give it (w or h 0: the scene's whole bounds again) -> false without the host
pub fn ios_pad_window(w: f64, h: f64) -> bool {
    on_main(|| {
        let win = host().window.get();
        if win.is_null() {
            return false;
        }
        window_frame(win, w, h);
        true
    })
}

/// the window's frame in its scene: w x h points at the top left (0: the scene's bounds)
pub(super) fn window_frame(win: Id, w: f64, h: f64) {
    // SAFETY: the live window and its scene on the main thread.
    unsafe {
        let scene: Id = msg!(win, c"windowScene"; Id);
        let sb: NSRect = if scene.is_null() { NSRect::default() } else { msg!(msg!(scene, c"coordinateSpace"; Id), c"bounds"; NSRect) };
        let r = if w <= 0.0 || h <= 0.0 { sb } else { NSRect::new(0.0, 0.0, w, h) };
        msg!(win, c"setFrame:", r => NSRect; ());
        msg!(win, c"layoutIfNeeded"; ());
    }
}

/// (scene_connect) jfgui_IOS_WINDOW=WxH: the window w x h points inside its scene from the start (a
/// look at Split View, Slide Over or Stage Manager's sizes without dragging; screenshots, tests)
pub(super) fn window_from_env(win: Id) {
    let Some(v) = crate::getenv("jfgui_IOS_WINDOW") else { return };
    let mut it = v.split(['x', 'X']).map(|n| n.trim().parse::<u32>().unwrap_or(0));
    let (w, h) = (it.next().unwrap_or(0), it.next().unwrap_or(0));
    if w > 0 && h > 0 {
        window_frame(win, w as f64, h as f64);
    }
}

/// (tests, any thread) the scene turned to landscape (true) or portrait, as the user turning the
/// iPad would (requestGeometryUpdateWithPreferences:) -> false without the host
pub fn ios_pad_rotate(landscape: bool) -> bool {
    on_main(|| {
        let win = host().window.get();
        if win.is_null() {
            return false;
        }
        // SAFETY: the live window's scene; the preferences are released after the request.
        unsafe {
            let scene: Id = msg!(win, c"windowScene"; Id);
            let cls = class(c"UIWindowSceneGeometryPreferencesIOS");
            if scene.is_null() || cls.is_null() {
                return false;
            }
            // UIInterfaceOrientationMaskPortrait 2, MaskLandscape 24
            let mask: usize = if landscape { 24 } else { 2 };
            let p: Id = msg!(cls, c"alloc"; Id);
            let p: Id = msg!(p, c"initWithInterfaceOrientations:", mask => usize; Id);
            msg!(scene, c"requestGeometryUpdateWithPreferences:errorHandler:", p => Id, NIL => Id; ());
            release(p);
        }
        true
    })
}

/// (tests, any thread) the on-screen keyboard as a tap into a text box brings it (JFView first
/// responder) or "Done" hides it -> false without the host
pub fn ios_pad_keyboard(show: bool) -> bool {
    on_main(|| {
        if host().view.get().is_null() {
            return false;
        }
        super::keyboard(show);
        true
    })
}

/// how often sysmenu_set asked UIKit for a rebuild (tests)
static REBUILDS: AtomicU32 = AtomicU32::new(0);
/// the stand-in builder's top-level menus, in order (retained; main thread)
static FAKE: SpinLock<Vec<usize>> = SpinLock::new(Vec::new());

/// a menu's identifier as a String
fn ident_of(menu: Id) -> String {
    // SAFETY: a live UIMenu.
    string_of(unsafe { msg!(menu, c"identifier"; Id) })
}

/// the menu with this identifier in the stand-in's tree (top level or below), or NIL
fn fake_find(list: &[usize], ident: &str) -> Id {
    for &m in list {
        let m = m as Id;
        if ident_of(m) == ident {
            return m;
        }
        // SAFETY: the children of a live UIMenu.
        let ch: Id = unsafe { msg!(m, c"children"; Id) };
        let n = if ch.is_null() { 0 } else { unsafe { msg!(ch, c"count"; usize) } };
        let subs: Vec<usize> = (0..n)
            .map(|i| unsafe { msg!(ch, c"objectAtIndex:", i => usize; Id) })
            .filter(|&e| unsafe { msg!(e, c"isKindOfClass:", class(c"UIMenu") => Id; bool) })
            .map(|e| e as usize)
            .collect();
        let f = fake_find(&subs, ident);
        if !f.is_null() {
            return f;
        }
    }
    NIL
}

extern "C" fn fb_system(_this: Id, _sel: Sel) -> Id {
    // SAFETY: the main menu system.
    unsafe { msg!(class(c"UIMenuSystem"), c"mainSystem"; Id) }
}
extern "C" fn fb_menu(_this: Id, _sel: Sel, ident: Id) -> Id {
    fake_find(&FAKE.lock(), &string_of(ident))
}
extern "C" fn fb_remove(_this: Id, _sel: Sel, ident: Id) {
    let id = string_of(ident);
    let mut l = FAKE.lock();
    if let Some(i) = l.iter().position(|&m| ident_of(m as Id) == id) {
        release(l.remove(i) as Id);
    }
}
fn fb_insert(menu: Id, ident: Id, after: bool) {
    let id = string_of(ident);
    let mut l = FAKE.lock();
    if let Some(i) = l.iter().position(|&m| ident_of(m as Id) == id) {
        // SAFETY: the stand-in keeps the menu it holds.
        unsafe { msg!(menu, c"retain"; Id) };
        l.insert(if after { i + 1 } else { i }, menu as usize);
    }
}
extern "C" fn fb_after(_this: Id, _sel: Sel, menu: Id, ident: Id) {
    fb_insert(menu, ident, true);
}
extern "C" fn fb_before(_this: Id, _sel: Sel, menu: Id, ident: Id) {
    fb_insert(menu, ident, false);
}

/// (tests, any thread) the main menu built as UIKit would, with a stand-in for its UIMenuBuilder
/// (iPadOS builds its own only when the menu is needed: the bar revealed, Command held, a key of a
/// hardware keyboard -- none of which a test without input can bring): it starts with the system's
/// menus (application, File, Edit, Format, View, Window, Help) and answers what the build asks; the
/// app's menus, rows and commands are UIKit's own objects -> the identifiers of the top level after
/// the build (ios_pad_menus reads the rows), None without the host
pub fn ios_pad_menu_build() -> Option<Vec<String>> {
    on_main(|| {
        if host().view.get().is_null() {
            return None;
        }
        type Imp = *const c_void;
        let (cls, _) = super::super::objc::subclass(c"NSObject", c"JFTestMenuBuilder", &[
            (c"system", fb_system as Imp, c"@@:"),
            (c"menuForIdentifier:", fb_menu as Imp, c"@@:@"),
            (c"removeMenuForIdentifier:", fb_remove as Imp, c"v@:@"),
            (c"insertSiblingMenu:afterMenuForIdentifier:", fb_after as Imp, c"v@:@@"),
            (c"insertSiblingMenu:beforeMenuForIdentifier:", fb_before as Imp, c"v@:@@"),
        ], &[]);
        // SAFETY: menus made here and kept by the stand-in until the next build; our builder object
        // lives for the call.
        unsafe {
            {
                let mut l = FAKE.lock();
                for m in l.drain(..) {
                    release(m as Id);
                }
                for (title, id) in [("App", UIMenuApplication), ("File", UIMenuFile), ("Edit", UIMenuEdit), ("Format", UIMenuFormat),
                                    ("View", UIMenuView), ("Window", UIMenuWindow), ("Help", UIMenuHelp)] {
                    let m = make_menu(title, id, 0, array(&[]));
                    msg!(m, c"retain"; Id);
                    l.push(m as usize);
                }
            }
            let b: Id = msg!(cls, c"new"; Id);
            app_build_menu(NIL, core::ptr::null(), b);
            release(b);
        }
        Some(FAKE.lock().iter().map(|&m| ident_of(m as Id)).collect())
    })
}

/// the main menu after the last build (ios_pad_menus)
#[derive(Clone, Debug, Default)]
pub struct IosPadMenus {
    /// builds so far (UIKit's and ios_pad_menu_build's), rebuilds sysmenu_set asked UIKit for
    pub builds: u32,
    pub rebuilds: u32,
    /// the system's File, Edit, Format and View menus went
    pub system_gone: bool,
    /// the app's menus with their rows as the builder holds them
    pub menus: Vec<(String, Vec<IosMenuRow>)>,
}

/// (tests, any thread) the main menu after the last build
pub fn ios_pad_menus() -> IosPadMenus {
    IosPadMenus {
        builds: BUILDS.load(Ordering::Acquire),
        rebuilds: REBUILDS.load(Ordering::Acquire),
        system_gone: SYSTEM_GONE.load(Ordering::Acquire),
        menus: BUILT.lock().clone(),
    }
}

/// (tests, any thread) choose row `row` of the app's menu `menu` as the menu bar does: its action
/// goes up the responder chain from the first responder -> false: no such row, or no one took it
pub fn ios_pad_menu_choose(menu: usize, row: usize) -> bool {
    on_main(|| {
        let top = TOPS.lock().get(menu).copied().unwrap_or(0) as Id;
        if top.is_null() {
            return false;
        }
        let mut r = row;
        let cmd = find_command(top, &mut r);
        if cmd.is_null() {
            return false;
        }
        // SAFETY: the shared application and a live command of ours.
        unsafe {
            let app: Id = msg!(class(c"UIApplication"), c"sharedApplication"; Id);
            let action: Sel = msg!(cmd, c"action"; Sel);
            msg!(app, c"sendAction:to:from:forEvent:", action => Sel, NIL => Id, cmd => Id, NIL => Id; bool)
        }
    })
}

/// what the tests feed in where a pointer would (ios_pad_pointer)
#[derive(Clone, Copy, Debug)]
pub enum IosPadPointer {
    /// the pointer (pen: the pencil's hover with this tilt) over the view without a button at (x, y)
    /// points; Leave: it left the view
    Hover { x: f64, y: f64, pen: Option<(i32, i32)> },
    Leave,
    /// two fingers on the trackpad move the content by (dx, dy) points at (x, y)
    Scroll { dx: f64, dy: f64, x: f64, y: f64 },
    /// a trackpad's pinch: the scale changed by m (0.1 = 10 % larger)
    Pinch { m: f64, x: f64, y: f64 },
}

/// (tests, any thread) the recognizers' paths with the values a pointer would give (the simulator
/// has no pointer of its own to drive) -> false without the host
pub fn ios_pad_pointer(p: IosPadPointer) -> bool {
    on_main(|| {
        if host().view.get().is_null() {
            return false;
        }
        match p {
            IosPadPointer::Hover { x, y, pen } => hover(GR_CHANGED, x, y, pen),
            IosPadPointer::Leave => hover(3, 0.0, 0.0, None),
            IosPadPointer::Scroll { dx, dy, x, y } => scroll(dx, dy, x, y, 0),
            IosPadPointer::Pinch { m, x, y } => pinch(m, x, y, 0),
        }
        jfgui_input(time_ms());
        true
    })
}
