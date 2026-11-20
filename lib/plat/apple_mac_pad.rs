// apple_mac_pad -- (G7a part 2) game pads on macOS: GameController (the shared part: pad.rs)
//
//   start() -> bool             an observer of GCControllerDidConnectNotification and
//                               ...DidDisconnectNotification, then the controllers there already
//   stop()                      every pad goes, the observer is removed
//   poll(now, focused, full) -> next   the pads' extendedGamepad values (pad_update), once a second
//                               (and with full) GCController.controllers against the slots: a
//                               notification that never came still connects or disconnects
//   gc_test_pad(state) / gc_test_disconnect(slot)   (tests) a controller of GameController's own
//                               (controllerWithExtendedGamepad) with these values, through the
//                               same paths as a real one
//
// Only controllers with the extended gamepad profile count (Xbox, PlayStation, Switch Pro, MFi
// pads); a Siri remote or a single Joy-Con (micro gamepad) does not.  GameController names the
// buttons by their places as the standard layout does (buttonA the bottom one); its sticks have up
// positive (the layout: down).  It hands input to the frontmost app only (shouldMonitorBackgroundEvents
// stays off), as pad.rs wants it anyway.  Everything on the main thread: the notifications come on
// the main queue, inside the window loop's run loop, which a wake-up (wake_app) lets return.

use core::ffi::c_void;

use rt::sync::SpinLock;

use super::super::objc::{Id, NIL, Sel, class, msg, string_of, subclass};
use crate::{PAD_A, PAD_B, PAD_BACK, PAD_DOWN, PAD_HOME, PAD_LB, PAD_LEFT, PAD_LS, PAD_LT, PAD_MAX, PAD_RB, PAD_RIGHT, PAD_RS, PAD_RT, PAD_START, PAD_TRIGGER_PRESS, PAD_UP,
            PAD_X, PAD_Y, PadState, pad_connect, pad_disconnect, pad_style, pad_update};

#[link(name = "GameController", kind = "framework")]
unsafe extern "C" {
    static GCControllerDidConnectNotification: Id;
    static GCControllerDidDisconnectNotification: Id;
}

const POLL_US: u64 = 4_000;
const LIST_US: u64 = 1_000_000;

struct Mp {
    /// the observer (an Id as usize: 0 none)
    observer: usize,
    /// the controller of each slot, retained (0: empty)
    ctrls: [usize; PAD_MAX],
    next_list: u64,
}

static MP: SpinLock<Mp> = SpinLock::new(Mp { observer: 0, ctrls: [0; PAD_MAX], next_list: 0 });

/// [obj sel] answers: a selector the object may lack (optional elements of later macOS versions)
fn responds(obj: Id, s: &core::ffi::CStr) -> bool {
    // SAFETY: respondsToSelector: on a live object.
    !obj.is_null() && unsafe { msg!(obj, c"respondsToSelector:", super::super::objc::sel(s) => Sel; bool) }
}

/// an element of the gamepad by its getter (NIL: the profile has none)
fn element(gp: Id, s: &core::ffi::CStr) -> Id {
    if !responds(gp, s) {
        return NIL;
    }
    // SAFETY: a getter of GCExtendedGamepad that returns an object (or nil).
    unsafe { msg!(gp, s; Id) }
}

/// a GCControllerButtonInput's value and pressed state ((0, false) for none)
fn button(b: Id) -> (f32, bool) {
    if b.is_null() {
        return (0.0, false);
    }
    // SAFETY: value / isPressed of a GCControllerButtonInput.
    unsafe { (msg!(b, c"value"; f32), msg!(b, c"isPressed"; bool)) }
}

/// a GCControllerDirectionPad's x and y (up positive), (0, 0) for none
fn direction(d: Id) -> (f32, f32) {
    if d.is_null() {
        return (0.0, 0.0);
    }
    // SAFETY: xAxis / yAxis (GCControllerAxisInput) and their value.
    unsafe {
        let x: Id = msg!(d, c"xAxis"; Id);
        let y: Id = msg!(d, c"yAxis"; Id);
        let v = |a: Id| if a.is_null() { 0.0 } else { msg!(a, c"value"; f32) };
        (v(x), v(y))
    }
}

/// the getters of the standard layout's buttons on GCExtendedGamepad
const BUTTONS: [(&core::ffi::CStr, usize); 13] = [
    (c"buttonA", PAD_A),
    (c"buttonB", PAD_B),
    (c"buttonX", PAD_X),
    (c"buttonY", PAD_Y),
    (c"leftShoulder", PAD_LB),
    (c"rightShoulder", PAD_RB),
    (c"leftTrigger", PAD_LT),
    (c"rightTrigger", PAD_RT),
    (c"buttonOptions", PAD_BACK),
    (c"buttonMenu", PAD_START),
    (c"leftThumbstickButton", PAD_LS),
    (c"rightThumbstickButton", PAD_RS),
    (c"buttonHome", PAD_HOME),
];

/// a GCExtendedGamepad's values in the standard layout (the triggers pressed above
/// PAD_TRIGGER_PRESS, as on the other platforms)
fn read(gp: Id) -> PadState {
    let mut st = PadState::NEUTRAL;
    if gp.is_null() {
        return st;
    }
    let mut set = |b: usize, (v, p): (f32, bool)| {
        st.buttons[b] = v;
        if p {
            st.pressed |= 1 << b;
        }
    };
    for (s, b) in BUTTONS {
        let (v, p) = button(element(gp, s));
        set(b, (v, if b == PAD_LT || b == PAD_RT { v > PAD_TRIGGER_PRESS } else { p }));
    }
    let dpad = element(gp, c"dpad");
    if !dpad.is_null() {
        for (s, b) in [(c"up", PAD_UP), (c"down", PAD_DOWN), (c"left", PAD_LEFT), (c"right", PAD_RIGHT)] {
            set(b, button(element(dpad, s)));
        }
    }
    let (lx, ly) = direction(element(gp, c"leftThumbstick"));
    let (rx, ry) = direction(element(gp, c"rightThumbstick"));
    st.axes = [lx, 0.0 - ly, rx, 0.0 - ry]; // (up positive in GameController; 0.0 - y: no -0.0)
    st
}

/// the controller's extended gamepad (NIL: it has none)
fn gamepad(ctrl: Id) -> Id {
    element(ctrl, c"extendedGamepad")
}

/// a controller came: the lowest free slot, its name and style -> the slot (None: no extended
/// gamepad, known already, or no slot free)
fn connect(ctrl: Id, focused: bool) -> Option<usize> {
    if ctrl.is_null() || gamepad(ctrl).is_null() {
        return None;
    }
    let slot = {
        let mut m = MP.lock();
        if m.ctrls.contains(&(ctrl as usize)) {
            return None;
        }
        let slot = m.ctrls.iter().position(|&c| c == 0)?;
        // SAFETY: the slot keeps the controller alive (released when it goes).
        unsafe { msg!(ctrl, c"retain"; Id) };
        m.ctrls[slot] = ctrl as usize;
        slot
    };
    let vendor = string_of(element(ctrl, c"vendorName"));
    let category = string_of(element(ctrl, c"productCategory"));
    let name = if vendor.is_empty() { if category.is_empty() { alloc::string::String::from("Controller") } else { category.clone() } } else { vendor };
    pad_connect(slot, &name, pad_style(0, &alloc::format!("{category} {name}")), true);
    pad_update(slot, &if focused { read(gamepad(ctrl)) } else { PadState::NEUTRAL });
    Some(slot)
}

/// a controller went
fn disconnect(ctrl: Id) {
    let slot = {
        let mut m = MP.lock();
        let Some(slot) = m.ctrls.iter().position(|&c| c == ctrl as usize && c != 0) else { return };
        m.ctrls[slot] = 0;
        slot
    };
    pad_disconnect(slot);
    super::super::objc::release(ctrl);
}

extern "C" fn on_connect(_this: Id, _cmd: Sel, note: Id) {
    // SAFETY: the notification's object is the GCController.
    let ctrl: Id = unsafe { msg!(note, c"object"; Id) };
    if connect(ctrl, crate::pad_focused()).is_some() {
        super::wake_app(); // the loop may wait in AppKit: let it return
    }
}

extern "C" fn on_disconnect(_this: Id, _cmd: Sel, note: Id) {
    // SAFETY: as on_connect.
    let ctrl: Id = unsafe { msg!(note, c"object"; Id) };
    disconnect(ctrl);
    super::wake_app();
}

/// the controllers GameController knows now against the slots
fn sync_list(focused: bool) {
    // SAFETY: +[GCController controllers] (an NSArray), its count and objects.
    let list: alloc::vec::Vec<usize> = unsafe {
        let a: Id = msg!(class(c"GCController"), c"controllers"; Id);
        let n: usize = if a.is_null() { 0 } else { msg!(a, c"count"; usize) };
        (0..n).map(|i| msg!(a, c"objectAtIndex:", i => usize; Id) as usize).collect()
    };
    let ours = MP.lock().ctrls;
    for c in ours {
        if c != 0 && !list.contains(&c) {
            disconnect(c as Id);
        }
    }
    for c in list {
        connect(c as Id, focused);
    }
}

/// observe the notifications, take the controllers there already
pub(crate) fn start() -> bool {
    if class(c"GCController").is_null() {
        return false;
    }
    let methods = [(c"jfPadConnected:", on_connect as *const c_void, c"v@:@"), (c"jfPadDisconnected:", on_disconnect as *const c_void, c"v@:@")];
    let (cls, _) = subclass(c"NSObject", c"JFPadObserver", &methods, &[]);
    // SAFETY: an observer object of ours, kept until stop; the centre retains nothing of it.
    unsafe {
        let pool = super::super::objc::objc_autoreleasePoolPush();
        let obs: Id = msg!(cls, c"new"; Id);
        let nc: Id = msg!(class(c"NSNotificationCenter"), c"defaultCenter"; Id);
        msg!(nc, c"addObserver:selector:name:object:", obs => Id, super::super::objc::sel(c"jfPadConnected:") => Sel, GCControllerDidConnectNotification => Id, NIL => Id; ());
        msg!(nc, c"addObserver:selector:name:object:", obs => Id, super::super::objc::sel(c"jfPadDisconnected:") => Sel, GCControllerDidDisconnectNotification => Id,
             NIL => Id; ());
        MP.lock().observer = obs as usize;
        sync_list(crate::pad_focused());
        super::super::objc::objc_autoreleasePoolPop(pool);
    }
    true
}

/// every pad goes, the observer with them
pub(crate) fn stop() {
    let (obs, ctrls) = {
        let mut m = MP.lock();
        (core::mem::take(&mut m.observer), m.ctrls)
    };
    for c in ctrls {
        if c != 0 {
            disconnect(c as Id);
        }
    }
    if obs != 0 {
        // SAFETY: our observer, made in start.
        unsafe {
            let nc: Id = msg!(class(c"NSNotificationCenter"), c"defaultCenter"; Id);
            msg!(nc, c"removeObserver:", obs as Id => Id; ());
        }
        super::super::objc::release(obs as Id);
    }
}

/// the pads' values; the list of controllers when its time has come (with full at once) -> the
/// next time to poll
pub(crate) fn poll(now: u64, focused: bool, full: bool) -> u64 {
    // SAFETY: an autorelease pool around the GameController calls of this round.
    let pool = unsafe { super::super::objc::objc_autoreleasePoolPush() };
    let list_due = full || now >= MP.lock().next_list;
    if list_due {
        sync_list(focused);
        MP.lock().next_list = now + LIST_US;
    }
    // (every connected pad is read each round: a change of the focus needs nothing more)
    let ctrls = MP.lock().ctrls;
    for (slot, c) in ctrls.into_iter().enumerate() {
        if c != 0 {
            pad_update(slot, &if focused { read(gamepad(c as Id)) } else { PadState::NEUTRAL });
        }
    }
    // SAFETY: the pool pushed above.
    unsafe { super::super::objc::objc_autoreleasePoolPop(pool) };
    let m = MP.lock();
    if m.ctrls.iter().any(|&c| c != 0) { (now + POLL_US).min(m.next_list) } else { m.next_list }
}

/// (test aid, plattest --test pad) a controller of GameController's own
/// (+[GCController controllerWithExtendedGamepad], macOS 10.15) with the buttons and sticks of
/// `st` set on its elements (setValue:, setValueForXAxis:yAxis:), read back through the backend's
/// mapping and connected through its path (watching or not) -> (what the mapping read, the slot);
/// None: no such controller here
pub fn gc_test_pad(st: &PadState) -> Option<(PadState, usize)> {
    let gc = class(c"GCController");
    if gc.is_null() || !responds(gc, c"controllerWithExtendedGamepad") {
        return None;
    }
    // SAFETY: a controller made for tests, its elements' setters (macOS 10.15 and newer).
    unsafe {
        let pool = super::super::objc::objc_autoreleasePoolPush();
        let ctrl: Id = msg!(gc, c"controllerWithExtendedGamepad"; Id);
        let gp = gamepad(ctrl);
        if gp.is_null() {
            super::super::objc::objc_autoreleasePoolPop(pool);
            return None;
        }
        for (s, b) in BUTTONS {
            let e = element(gp, s);
            if !e.is_null() && responds(e, c"setValue:") {
                msg!(e, c"setValue:", st.buttons[b] => f32; ());
            }
        }
        let b = |k: usize| st.buttons[k];
        let dpad = element(gp, c"dpad");
        if responds(dpad, c"setValueForXAxis:yAxis:") {
            msg!(dpad, c"setValueForXAxis:yAxis:", b(PAD_RIGHT) - b(PAD_LEFT) => f32, b(PAD_UP) - b(PAD_DOWN) => f32; ());
        }
        for (s, k) in [(c"leftThumbstick", 0usize), (c"rightThumbstick", 2usize)] {
            let d = element(gp, s);
            if responds(d, c"setValueForXAxis:yAxis:") {
                msg!(d, c"setValueForXAxis:yAxis:", st.axes[k] => f32, -st.axes[k + 1] => f32; ());
            }
        }
        let got = read(gp);
        let slot = connect(ctrl, true);
        super::super::objc::objc_autoreleasePoolPop(pool);
        Some((got, slot?))
    }
}

/// (test aid) the test controller of `slot` goes through the backend's path (pad_disconnect)
pub fn gc_test_disconnect(slot: usize) {
    let c = MP.lock().ctrls.get(slot).copied().unwrap_or(0);
    if c != 0 {
        disconnect(c as Id);
    }
}
