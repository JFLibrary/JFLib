// pad -- (G7a part 2) game pads: one snapshot per pad in the W3C standard layout and their connect
// and disconnect events, the part every platform shares
//
//   pad_watch(on) -> bool      start (on) or stop watching for pads -> false: this platform has none
//                              (Android, iOS: a later stage with touch controls) or the system lacks
//                              the means (Windows without an XInput DLL); nothing runs before
//   pad_watching() -> bool     watching now
//   pads() -> Vec<Gamepad>     the connected pads by index, with the buttons that went down and up
//                              since the last call; takes the E_GAMEPAD that waits
//   pad(i) -> Gamepad          slot i as it stands (connected or not), edges left in place
//   E_GAMEPAD                  data = index | PADEV_* << 8: PADEV_STATE (a pad's state changed: one
//                              event until pads() is called, whichever pads change meanwhile),
//                              PADEV_CONNECT, PADEV_DISCONNECT (each its own event)
//
// The layout is the W3C Gamepad API's "standard" mapping, the same on every platform: buttons
// PAD_A (bottom face button: Xbox A, PlayStation cross, Nintendo B) .. PAD_HOME with analog values
// 0 ..= 1 (the triggers PAD_LT / PAD_RT analog, the others 0 or 1 on most pads) and a bit each in
// Gamepad::pressed (a trigger counts as pressed above PAD_TRIGGER_PRESS, XInput's threshold); two
// sticks PAD_LX, PAD_LY, PAD_RX, PAD_RY in -1 ..= 1, right and down positive, raw: no dead zone
// (lib/game applies a radial one).  Gamepad::style says whose labels the buttons carry.
//
// Per platform:
//   Windows  XInput (Xbox-compatible pads, at most four, index = XInput's slot): xinput1_4.dll,
//            else xinput9_1_0.dll, loaded at the first pad_watch(true); its ordinal 100
//            (XInputGetStateEx) where it has it, for the guide button (win_pad.rs)
//   Linux    evdev: the /dev/input/event* nodes the user may read (udev gives the user of the
//            seat game controllers, not keyboards) with the buttons of a game pad (BTN_SOUTH) and
//            two axes; the kernel's gamepad codes mapped to the layout, Sony's and Nintendo's
//            north / west buttons the other way round than xpad's; hotplug through inotify on
//            /dev/input (linux_pad.rs).  Inside a Flatpak only with --device=input
//   macOS    GameController: GCController's extendedGamepad, connect and disconnect
//            notifications (apple_mac_pad.rs)
//   browser  the Gamepad API's pads with the "standard" mapping (others pass their first buttons
//            and axes as the browser numbers them, mapped false): the page polls
//            navigator.getGamepads() every animation frame while the app watches and hands each
//            change to the worker (jfgui_pad; loader.js).  The browser shows a pad only after a
//            button of it was pressed in the page
//   Android, iOS  Unsupported: pad_watch says false
//
// The native backends are polled from the window loop's wait (pad_wait): every 4 ms while a pad is
// connected, new pads every 250 ms (Windows: one empty slot each time, XInput is slow for empty
// ones; Linux: what inotify reported; macOS: its notifications, once a second a look at the list).
// A change of a button wakes the app at once, an axis only when it moved by 1/256 or more since
// the last event (a stick at rest jitters); the snapshot always has the latest values.  Pads speak
// only to the app that has the focus, as the keyboard does: while none of its windows is active
// they report nothing held and their sticks at rest (still connected).

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use rt::sync::SpinLock;

use crate::{E_GAMEPAD, Event, evq_push};

/// slots of pads (Gamepad::index is below this)
pub const PAD_MAX: usize = 8;
/// buttons of the standard layout (Gamepad::buttons)
pub const PAD_BUTTONS: usize = 17;
/// axes of the standard layout (Gamepad::axes)
pub const PAD_AXES: usize = 4;

/// the bottom face button (Xbox A, PlayStation cross, Nintendo B)
pub const PAD_A: usize = 0;
/// the right face button (Xbox B, PlayStation circle, Nintendo A)
pub const PAD_B: usize = 1;
/// the left face button (Xbox X, PlayStation square, Nintendo Y)
pub const PAD_X: usize = 2;
/// the top face button (Xbox Y, PlayStation triangle, Nintendo X)
pub const PAD_Y: usize = 3;
/// the left shoulder button (LB, L1, L)
pub const PAD_LB: usize = 4;
/// the right shoulder button (RB, R1, R)
pub const PAD_RB: usize = 5;
/// the left trigger, analog (LT, L2, ZL)
pub const PAD_LT: usize = 6;
/// the right trigger, analog (RT, R2, ZR)
pub const PAD_RT: usize = 7;
/// the left middle button (View / Back, Share / Create, minus)
pub const PAD_BACK: usize = 8;
/// the right middle button (Menu / Start, Options, plus)
pub const PAD_START: usize = 9;
/// the left stick pressed (LS, L3)
pub const PAD_LS: usize = 10;
/// the right stick pressed (RS, R3)
pub const PAD_RS: usize = 11;
/// the directional pad up
pub const PAD_UP: usize = 12;
/// the directional pad down
pub const PAD_DOWN: usize = 13;
/// the directional pad left
pub const PAD_LEFT: usize = 14;
/// the directional pad right
pub const PAD_RIGHT: usize = 15;
/// the middle button of the system (Xbox, PS, Home)
pub const PAD_HOME: usize = 16;

/// the left stick to the right (Gamepad::axes)
pub const PAD_LX: usize = 0;
/// the left stick down
pub const PAD_LY: usize = 1;
/// the right stick to the right
pub const PAD_RX: usize = 2;
/// the right stick down
pub const PAD_RY: usize = 3;

/// E_GAMEPAD's kind (data >> 8): a pad's state changed (one event until pads() is called)
pub const PADEV_STATE: u32 = 0;
/// E_GAMEPAD's kind: pad data & 0xFF came
pub const PADEV_CONNECT: u32 = 1;
/// E_GAMEPAD's kind: pad data & 0xFF went
pub const PADEV_DISCONNECT: u32 = 2;

/// the value above which a trigger counts as pressed (XInput's XINPUT_GAMEPAD_TRIGGER_THRESHOLD)
pub const PAD_TRIGGER_PRESS: f32 = 30.0 / 255.0;

/// an axis that moved less than this since the last E_GAMEPAD wakes nobody (its value is in the
/// snapshot all the same)
const EVENT_STEP: f32 = 1.0 / 256.0;

/// whose labels a pad's buttons carry (the layout is the same for all)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PadStyle {
    /// not known
    #[default]
    Other,
    /// Xbox and XInput pads: A B X Y, LB RB LT RT, View, Menu
    Xbox,
    /// PlayStation: cross, circle, square, triangle, L1 R1 L2 R2, Share / Create, Options
    PlayStation,
    /// Nintendo: B A Y X by their places (the bottom button is B), L R ZL ZR, minus, plus
    Nintendo,
}

/// one pad's state at a moment, in the standard layout
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PadState {
    /// the buttons, 0 ..= 1 (PAD_A ..= PAD_HOME)
    pub buttons: [f32; PAD_BUTTONS],
    /// the buttons held (bit PAD_*)
    pub pressed: u32,
    /// PAD_LX, PAD_LY, PAD_RX, PAD_RY: -1 ..= 1, right and down positive
    pub axes: [f32; PAD_AXES],
}

impl PadState {
    /// nothing held, the sticks at rest
    pub const NEUTRAL: PadState = PadState { buttons: [0.0; PAD_BUTTONS], pressed: 0, axes: [0.0; PAD_AXES] };

    /// button b (PAD_*) is held
    pub fn held(&self, b: usize) -> bool {
        b < PAD_BUTTONS && self.pressed & (1 << b) != 0
    }

    /// values in range, NaN as 0, the pressed bits of the layout only
    fn clean(&self) -> PadState {
        let mut s = *self;
        for v in &mut s.buttons {
            *v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
        }
        for v in &mut s.axes {
            *v = if v.is_nan() { 0.0 } else { v.clamp(-1.0, 1.0) };
        }
        s.pressed &= (1 << PAD_BUTTONS) - 1;
        s
    }
}

/// a pad as plat::pads reports it
#[derive(Clone, Debug, PartialEq)]
pub struct Gamepad {
    /// its slot, the same while it stays connected (0 .. PAD_MAX; Windows: XInput's slot, the
    /// browser: the Gamepad API's index)
    pub index: u8,
    /// connected (pads() lists only connected ones; pad(i) of an empty slot: false)
    pub connected: bool,
    /// the system's name for it ("Xbox Controller (XInput)", "Sony Interactive Entertainment
    /// Wireless Controller", the browser's id)
    pub name: String,
    /// whose labels its buttons carry
    pub style: PadStyle,
    /// its buttons and axes are the standard layout (false: a browser pad without the "standard"
    /// mapping: its first buttons and axes as the browser numbers them)
    pub mapped: bool,
    /// the standard layout's buttons, 0 ..= 1 (PAD_A ..= PAD_HOME; the triggers analog)
    pub buttons: [f32; PAD_BUTTONS],
    /// the buttons held now (bit PAD_*)
    pub pressed: u32,
    /// the buttons that went down since the last plat::pads (a tap between two reads is in down
    /// and up, but not in pressed)
    pub down: u32,
    /// the buttons that went up since the last plat::pads
    pub up: u32,
    /// PAD_LX, PAD_LY, PAD_RX, PAD_RY: -1 ..= 1, right and down positive, raw (no dead zone)
    pub axes: [f32; PAD_AXES],
}

impl Gamepad {
    /// slot i, empty: not connected, nothing held
    pub const fn empty(i: u8) -> Gamepad {
        Gamepad {
            index: i,
            connected: false,
            name: String::new(),
            style: PadStyle::Other,
            mapped: true,
            buttons: [0.0; PAD_BUTTONS],
            pressed: 0,
            down: 0,
            up: 0,
            axes: [0.0; PAD_AXES],
        }
    }

    /// button b (PAD_*) is held
    pub fn held(&self, b: usize) -> bool {
        b < PAD_BUTTONS && self.pressed & (1 << b) != 0
    }

    /// button b went down since the last read
    pub fn went_down(&self, b: usize) -> bool {
        b < PAD_BUTTONS && self.down & (1 << b) != 0
    }

    /// button b went up since the last read
    pub fn went_up(&self, b: usize) -> bool {
        b < PAD_BUTTONS && self.up & (1 << b) != 0
    }

    /// button b's value, 0 ..= 1 (0 for another index)
    pub fn value(&self, b: usize) -> f32 {
        self.buttons.get(b).copied().unwrap_or(0.0)
    }

    /// axis a's value (PAD_LX ..), -1 ..= 1
    pub fn axis(&self, a: usize) -> f32 {
        self.axes.get(a).copied().unwrap_or(0.0)
    }

    /// stick s (0 left, 1 right): [x, y], right and down positive
    pub fn stick(&self, s: usize) -> [f32; 2] {
        let k = if s == 0 { PAD_LX } else { PAD_RX };
        [self.axes[k], self.axes[k + 1]]
    }

    /// trigger t (0 left, 1 right), 0 ..= 1
    pub fn trigger(&self, t: usize) -> f32 {
        self.buttons[if t == 0 { PAD_LT } else { PAD_RT }]
    }

    /// the state alone
    pub fn state(&self) -> PadState {
        PadState { buttons: self.buttons, pressed: self.pressed, axes: self.axes }
    }
}

/// the short English name of button b of the standard layout ("A", "LB", "Up", "Home"; "" for
/// another index): what a test or a log shows (lib/game has the labels for menus)
pub fn pad_button_name(b: usize) -> &'static str {
    const N: [&str; PAD_BUTTONS] = ["A", "B", "X", "Y", "LB", "RB", "LT", "RT", "Back", "Start", "LS", "RS", "Up", "Down", "Left", "Right", "Home"];
    N.get(b).copied().unwrap_or("")
}

/// whose labels a pad has, from its USB vendor (0: not known) and its name
pub fn pad_style(vendor: u16, name: &str) -> PadStyle {
    match vendor {
        0x045E => return PadStyle::Xbox,
        0x054C => return PadStyle::PlayStation,
        0x057E => return PadStyle::Nintendo,
        _ => {}
    }
    let n = name.to_ascii_lowercase();
    let has = |w: &[&str]| w.iter().any(|s| n.contains(s));
    if has(&["xbox", "x-box", "xinput"]) {
        PadStyle::Xbox
    } else if has(&["playstation", "dualshock", "dualsense", "ps3", "ps4", "ps5", "sony"]) {
        PadStyle::PlayStation
    } else if has(&["nintendo", "switch", "joy-con", "pro controller"]) {
        PadStyle::Nintendo
    } else {
        PadStyle::Other
    }
}

/// the USB vendor in a browser's id of a pad: Chrome's "... (STANDARD GAMEPAD Vendor: 054c Product:
/// 0ce6)", Firefox's "054c-0ce6-DualSense Wireless Controller" (0: none)
pub fn pad_vendor_of_id(id: &str) -> u16 {
    let hex4 = |s: &str| -> Option<u16> {
        let h = s.get(..4)?;
        if h.bytes().all(|c| c.is_ascii_hexdigit()) { u16::from_str_radix(h, 16).ok() } else { None }
    };
    if let Some(k) = id.find("Vendor: ") {
        if let Some(v) = hex4(&id[k + 8..]) {
            return v;
        }
    }
    if id.as_bytes().get(4) == Some(&b'-') {
        if let Some(v) = hex4(id) {
            return v;
        }
    }
    0
}

// ------------------------------------------------------------------ the shared state

struct Slot {
    pad: Gamepad,
    /// the values when the last E_GAMEPAD went out (smaller steps wake nobody)
    sent: PadState,
}

struct Pads {
    slots: [Slot; PAD_MAX],
    /// an E_GAMEPAD of PADEV_STATE waits for pads()
    queued: bool,
    watching: bool,
}

const SLOT0: Slot = Slot { pad: Gamepad::empty(0), sent: PadState::NEUTRAL };

static PADS: SpinLock<Pads> = SpinLock::new(Pads { slots: [SLOT0, SLOT0, SLOT0, SLOT0, SLOT0, SLOT0, SLOT0, SLOT0], queued: false, watching: false });
/// the next time the native backend wants to be polled (time_us)
#[cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(dead_code))]
static NEXT_POLL: AtomicU64 = AtomicU64::new(0);
/// (tests) the focus as a test says it: 0 the windows', 1 focused, 2 not
static FOCUS_TEST: AtomicU8 = AtomicU8::new(0);

fn push(i: usize, kind: u32) {
    evq_push(Event { typ: E_GAMEPAD, data: (i as u32 & 0xFF) | kind << 8, ..Event::default() });
}

/// (backends) pad i came: its name, style and layout; nothing held yet (pad_update follows)
pub(crate) fn pad_connect(i: usize, name: &str, style: PadStyle, mapped: bool) {
    if i >= PAD_MAX {
        return;
    }
    let mut p = PADS.lock();
    let s = &mut p.slots[i];
    let was = s.pad.connected;
    s.pad = Gamepad { index: i as u8, connected: true, name: String::from(name), style, mapped, ..Gamepad::empty(i as u8) };
    s.sent = PadState::NEUTRAL;
    drop(p);
    if !was {
        push(i, PADEV_CONNECT);
    }
}

/// (backends) pad i went (its slot empty again)
pub(crate) fn pad_disconnect(i: usize) {
    if i >= PAD_MAX {
        return;
    }
    let mut p = PADS.lock();
    let s = &mut p.slots[i];
    if !s.pad.connected {
        return;
    }
    s.pad = Gamepad::empty(i as u8);
    s.sent = PadState::NEUTRAL;
    drop(p);
    push(i, PADEV_DISCONNECT);
}

/// (backends) pad i's state now: the edges of its buttons add up until pads() takes them; an
/// E_GAMEPAD when a button changed or an axis moved by EVENT_STEP since the last one (one until
/// pads() is called)
pub(crate) fn pad_update(i: usize, st: &PadState) {
    if i >= PAD_MAX {
        return;
    }
    let st = st.clean();
    let mut p = PADS.lock();
    let s = &mut p.slots[i];
    if !s.pad.connected || s.pad.state() == st {
        return;
    }
    s.pad.down |= st.pressed & !s.pad.pressed;
    s.pad.up |= s.pad.pressed & !st.pressed;
    let wake = st.pressed != s.sent.pressed || moved(&s.sent.buttons, &st.buttons) || moved(&s.sent.axes, &st.axes);
    s.pad.buttons = st.buttons;
    s.pad.pressed = st.pressed;
    s.pad.axes = st.axes;
    if !wake {
        return;
    }
    s.sent = st;
    if p.queued {
        return;
    }
    p.queued = true;
    drop(p);
    push(i, PADEV_STATE);
}

/// an analog value moved enough to wake the app: by EVENT_STEP, or onto a rest or an end
fn moved(a: &[f32], b: &[f32]) -> bool {
    a.iter().zip(b).any(|(&x, &y)| (x - y).abs() >= EVENT_STEP || (x != y && (y == 0.0 || y.abs() == 1.0)))
}

/// (backends) a pad of slot i is connected
#[allow(dead_code)]
pub(crate) fn pad_connected(i: usize) -> bool {
    i < PAD_MAX && PADS.lock().slots[i].pad.connected
}

/// (backends) pads speak to the app now: one of its windows is active (a test may say otherwise)
#[allow(dead_code)]
pub(crate) fn pad_focused() -> bool {
    match FOCUS_TEST.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => crate::win_state(0) & crate::WST_ACTIVE != 0,
    }
}

/// start (on) or stop watching for game pads -> false: this platform has no pads (Android, iOS) or
/// the system lacks the means (Windows without an XInput DLL).  While watching, pads come and go
/// with E_GAMEPAD events and plat::pads has them; stopping lets every pad go (PADEV_DISCONNECT).
/// The browser shows a pad only after one of its buttons was pressed in the page
pub fn pad_watch(on: bool) -> bool {
    if on == PADS.lock().watching {
        return true;
    }
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    {
        if on {
            if !pad_os::start() {
                return false;
            }
            NEXT_POLL.store(0, Ordering::Relaxed);
        } else {
            pad_os::stop();
        }
    }
    #[cfg(target_arch = "wasm32")]
    crate::web::notice(REQUEST_GAMEPAD, &[on as u8]);
    #[cfg(any(target_os = "android", target_os = "ios"))]
    if on {
        return false;
    }
    PADS.lock().watching = on;
    if !on {
        for i in 0..PAD_MAX {
            pad_disconnect(i);
        }
    }
    true
}

/// pads are being watched (pad_watch)
pub fn pad_watching() -> bool {
    PADS.lock().watching
}

/// the connected pads by index, each with the buttons that went down and up since the last call
/// (which go then), and the E_GAMEPAD that waits (the next change sends a new one).  Read it once
/// per frame (ui does: ui.gamepads)
pub fn pads() -> Vec<Gamepad> {
    let mut p = PADS.lock();
    p.queued = false;
    let mut v = Vec::new();
    for s in &mut p.slots {
        if s.pad.connected {
            v.push(s.pad.clone());
        }
        s.pad.down = 0;
        s.pad.up = 0;
    }
    v
}

/// slot i as it stands (Gamepad::connected false when empty), its edges left for pads()
pub fn pad(i: usize) -> Gamepad {
    if i >= PAD_MAX {
        return Gamepad::empty(0);
    }
    PADS.lock().slots[i].pad.clone()
}

// ------------------------------------------------------------------ the window loop

#[cfg(windows)]
use crate::win::pad_os;
#[cfg(target_os = "linux")]
use crate::linux::pad_os;
#[cfg(target_os = "macos")]
use crate::apple::mac::pad_os;

/// (the window loops' wait: win_api.rs, linux_disp.rs, apple_mac.rs) wait as `os_wait` does (until
/// an event or the deadline; 0 polls, u64::MAX forever), the pads polled when their time comes:
/// os_wait runs once per round, with the earlier of the deadline and the pads' next poll
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
pub(crate) fn pad_wait(deadline_us: u64, os_wait: &mut dyn FnMut(u64) -> u32) -> u32 {
    if !PADS.lock().watching {
        return os_wait(deadline_us);
    }
    loop {
        let next = pad_service(crate::time_us());
        let dl = if crate::evq_count() != 0 { 0 } else { deadline_us.min(next) };
        let n = os_wait(dl);
        if n != 0 || dl == deadline_us || dl == 0 {
            return n.max(crate::evq_count());
        }
    }
}

/// poll the backend when its time has come -> the next time
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
fn pad_service(now: u64) -> u64 {
    let due = NEXT_POLL.load(Ordering::Relaxed);
    if now < due {
        return due;
    }
    let next = pad_os::poll(now, pad_focused(), false).max(now + 1000);
    NEXT_POLL.store(next, Ordering::Relaxed);
    next
}

// ------------------------------------------------------------------ the browser

/// (browser) ask the page to watch the pads (one byte, 1 on / 0 off): it polls
/// navigator.getGamepads() every animation frame while one is connected and hands changes over
/// with jfgui_pad
pub const REQUEST_GAMEPAD: u32 = 33;

/// (G7a) the record of a pad the page hands over (jfgui_pad): u32 index, u32 flags (bit 0
/// connected, bit 1 the "standard" mapping), u32 the buttons pressed, f32 x 17 buttons, f32 x 4
/// axes, then the browser's id in UTF-8 -> (index, connected, mapped, state, id); None for a
/// short record or an index beyond PAD_MAX
pub fn web_pad_decode(rec: &[u8]) -> Option<(usize, bool, bool, PadState, String)> {
    const HEAD: usize = 12 + 4 * (PAD_BUTTONS + PAD_AXES);
    if rec.len() < HEAD {
        return None;
    }
    let u = |o: usize| u32::from_le_bytes([rec[o], rec[o + 1], rec[o + 2], rec[o + 3]]);
    let f = |o: usize| f32::from_bits(u(o));
    let i = u(0) as usize;
    if i >= PAD_MAX {
        return None;
    }
    let flags = u(4);
    let mut st = PadState { pressed: u(8), ..PadState::NEUTRAL };
    for (k, b) in st.buttons.iter_mut().enumerate() {
        *b = f(12 + 4 * k);
    }
    for (k, a) in st.axes.iter_mut().enumerate() {
        *a = f(12 + 4 * (PAD_BUTTONS + k));
    }
    let id = String::from_utf8_lossy(&rec[HEAD..]).chars().take(200).collect();
    Some((i, flags & 1 != 0, flags & 2 != 0, st.clean(), id))
}

/// (browser) a pad's state from the page (web_pad_decode's record in the inbox) -> 1 taken
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_pad(len: u32) -> u32 {
    let Some(rec) = crate::web::input(len) else { return 0 };
    let Some((i, connected, mapped, st, id)) = web_pad_decode(&rec) else { return 0 };
    if !pad_watching() {
        return 0;
    }
    if !connected {
        pad_disconnect(i);
        return 1;
    }
    if !pad_connected(i) {
        pad_connect(i, &id, pad_style(pad_vendor_of_id(&id), &id), mapped);
    }
    pad_update(i, &st);
    1
}

// ------------------------------------------------------------------ XInput (every target: tests)

/// XInput's buttons (XINPUT_GAMEPAD::wButtons) in the standard layout's order
const XI_BUTTONS: [(u16, usize); 15] = [
    (0x1000, PAD_A),
    (0x2000, PAD_B),
    (0x4000, PAD_X),
    (0x8000, PAD_Y),
    (0x0100, PAD_LB),
    (0x0200, PAD_RB),
    (0x0020, PAD_BACK),
    (0x0010, PAD_START),
    (0x0040, PAD_LS),
    (0x0080, PAD_RS),
    (0x0001, PAD_UP),
    (0x0002, PAD_DOWN),
    (0x0004, PAD_LEFT),
    (0x0008, PAD_RIGHT),
    (0x0400, PAD_HOME), // (XInputGetStateEx only)
];

/// (G7a) an XINPUT_GAMEPAD in the standard layout: wButtons, the triggers 0..255 (pressed above
/// 30, XINPUT_GAMEPAD_TRIGGER_THRESHOLD), the thumbs -32768..32767 with up positive (the layout
/// has down positive: y turned round)
pub fn xinput_map(buttons: u16, lt: u8, rt: u8, lx: i16, ly: i16, rx: i16, ry: i16) -> PadState {
    let mut st = PadState::NEUTRAL;
    for &(bit, b) in &XI_BUTTONS {
        if buttons & bit != 0 {
            st.buttons[b] = 1.0;
            st.pressed |= 1 << b;
        }
    }
    for (b, v) in [(PAD_LT, lt), (PAD_RT, rt)] {
        st.buttons[b] = v as f32 / 255.0;
        if v > 30 {
            st.pressed |= 1 << b;
        }
    }
    let thumb = |v: i16| if v < 0 { v as f32 / 32768.0 } else { v as f32 / 32767.0 };
    st.axes = [thumb(lx), 0.0 - thumb(ly), thumb(rx), 0.0 - thumb(ry)]; // (0.0 - x: no -0.0 at rest)
    st
}

/// (G7a) an XINPUT_STATE as XInputGetState fills it (16 bytes: dwPacketNumber, then
/// XINPUT_GAMEPAD) -> (packet number, the state in the layout); None for a short record
pub fn xinput_decode(rec: &[u8]) -> Option<(u32, PadState)> {
    if rec.len() < 16 {
        return None;
    }
    let u16_at = |o: usize| u16::from_le_bytes([rec[o], rec[o + 1]]);
    let packet = u32::from_le_bytes([rec[0], rec[1], rec[2], rec[3]]);
    let st = xinput_map(u16_at(4), rec[6], rec[7], u16_at(8) as i16, u16_at(10) as i16, u16_at(12) as i16, u16_at(14) as i16);
    Some((packet, st))
}

// ------------------------------------------------------------------ evdev (every target: tests)

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
const SYN_REPORT: u16 = 0;
const SYN_DROPPED: u16 = 3;
/// struct input_event on 64-bit Linux: struct timeval (16 bytes), type, code, value
pub const EVDEV_EVENT_SIZE: usize = 24;
/// the key codes a pad's bitmap covers (KEY_MAX + 1)
pub const EVDEV_KEYS: usize = 0x300;
/// the absolute axes (ABS_MAX + 1)
pub const EVDEV_ABS: usize = 0x40;

const BTN_SOUTH: u16 = 0x130;
const BTN_EAST: u16 = 0x131;
const BTN_NORTH: u16 = 0x133;
const BTN_WEST: u16 = 0x134;
const BTN_TL: u16 = 0x136;
const BTN_TR: u16 = 0x137;
const BTN_TL2: u16 = 0x138;
const BTN_TR2: u16 = 0x139;
const BTN_SELECT: u16 = 0x13A;
const BTN_START: u16 = 0x13B;
const BTN_MODE: u16 = 0x13C;
const BTN_THUMBL: u16 = 0x13D;
const BTN_THUMBR: u16 = 0x13E;
/// BTN_DPAD_UP; DOWN, LEFT, RIGHT follow
const BTN_DPAD_UP: u16 = 0x220;
const BTN_DPAD_RIGHT: u16 = 0x223;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
const ABS_Z: u16 = 2;
const ABS_RX: u16 = 3;
const ABS_RY: u16 = 4;
const ABS_RZ: u16 = 5;
const ABS_GAS: u16 = 9;
const ABS_BRAKE: u16 = 10;
const ABS_HAT0X: u16 = 16;
const ABS_HAT0Y: u16 = 17;
/// the absolute axes a pad may map (ABS_X ..= ABS_HAT0Y)
pub const EVDEV_ABS_MAPPED: usize = 18;

fn bit(map: &[u8], k: usize) -> bool {
    map.get(k / 8).is_some_and(|b| b & (1 << (k % 8)) != 0)
}

/// (G7a) an evdev node is a game pad: its event types (EVIOCGBIT(0)), keys (EVIOCGBIT(EV_KEY)) and
/// absolute axes (EVIOCGBIT(EV_ABS)) as the kernel's bitmaps -> it has keys, absolute axes, the
/// gamepad's first button (BTN_SOUTH, alias BTN_GAMEPAD) and a stick (ABS_X, ABS_Y).  A pad's
/// touchpad and motion sensors (their own nodes) have no BTN_SOUTH
pub fn evdev_is_pad(types: &[u8], keys: &[u8], abs: &[u8]) -> bool {
    bit(types, EV_KEY as usize) && bit(types, EV_ABS as usize) && bit(keys, BTN_SOUTH as usize) && bit(abs, ABS_X as usize) && bit(abs, ABS_Y as usize)
}

/// an absolute axis as the kernel describes it (EVIOCGABS: value, minimum, maximum)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AbsInfo {
    /// ABS_*
    pub code: u16,
    /// its value now
    pub value: i32,
    pub min: i32,
    pub max: i32,
}

#[derive(Clone, Copy, Debug, Default)]
enum Role {
    #[default]
    None,
    /// a stick's axis (PAD_LX ..)
    Axis(usize),
    /// an analog trigger (PAD_LT / PAD_RT)
    Trigger(usize),
    /// the directional pad as a hat: x / y
    HatX,
    HatY,
}

#[derive(Clone, Copy, Debug, Default)]
struct AbsMap {
    role: Role,
    min: i32,
    max: i32,
}

/// (G7a) one evdev game pad: its codes mapped to the standard layout, the events of the node in,
/// its state out.  The kernel's gamepad codes (Documentation/input/gamepad.rst): BTN_SOUTH and
/// BTN_EAST are A and B; xpad (Xbox pads and most others) reports X as BTN_NORTH (= BTN_X) and Y
/// as BTN_WEST (= BTN_Y), Sony's and Nintendo's drivers by the places (north the top button): the
/// vendor decides.  Sticks ABS_X / ABS_Y and ABS_RX / ABS_RY; triggers ABS_Z / ABS_RZ (without
/// ABS_RX / ABS_RY those are the right stick, as on simple HID pads), else ABS_BRAKE / ABS_GAS,
/// else BTN_TL2 / BTN_TR2 digital; the directional pad ABS_HAT0X / ABS_HAT0Y or BTN_DPAD_*
#[derive(Clone, Debug)]
pub struct EvdevPad {
    /// the button (index + 1) of key codes BTN_SOUTH ..= BTN_THUMBR and BTN_DPAD_UP ..= RIGHT
    keys: [u8; 15],
    dpad: [u8; 4],
    abs: [AbsMap; EVDEV_ABS_MAPPED],
    /// the triggers come as BTN_TL2 / BTN_TR2 (no analog ones)
    digital_triggers: bool,
    /// the state of the last SYN_REPORT, and the one being built
    state: PadState,
    work: PadState,
    /// a SYN_DROPPED: events are ignored up to the next SYN_REPORT, then the device's state must be
    /// read anew (resync)
    dropped: bool,
    resync: bool,
}

impl EvdevPad {
    /// a pad with these keys (EVIOCGBIT(EV_KEY)'s bitmap) and absolute axes (EVIOCGABS of each
    /// one it has) of this USB vendor (0x054C Sony, 0x057E Nintendo: north / west by place)
    pub fn new(vendor: u16, keys: &[u8], abs: &[AbsInfo]) -> EvdevPad {
        let by_place = vendor == 0x054C || vendor == 0x057E;
        let mut p = EvdevPad {
            keys: [0; 15],
            dpad: [0; 4],
            abs: [AbsMap::default(); EVDEV_ABS_MAPPED],
            digital_triggers: false,
            state: PadState::NEUTRAL,
            work: PadState::NEUTRAL,
            dropped: false,
            resync: false,
        };
        let map: [(u16, usize); 13] = [
            (BTN_SOUTH, PAD_A),
            (BTN_EAST, PAD_B),
            (BTN_NORTH, if by_place { PAD_Y } else { PAD_X }),
            (BTN_WEST, if by_place { PAD_X } else { PAD_Y }),
            (BTN_TL, PAD_LB),
            (BTN_TR, PAD_RB),
            (BTN_TL2, PAD_LT),
            (BTN_TR2, PAD_RT),
            (BTN_SELECT, PAD_BACK),
            (BTN_START, PAD_START),
            (BTN_MODE, PAD_HOME),
            (BTN_THUMBL, PAD_LS),
            (BTN_THUMBR, PAD_RS),
        ];
        for (code, b) in map {
            if bit(keys, code as usize) {
                p.keys[(code - BTN_SOUTH) as usize] = b as u8 + 1;
            }
        }
        for (k, b) in [PAD_UP, PAD_DOWN, PAD_LEFT, PAD_RIGHT].into_iter().enumerate() {
            if bit(keys, BTN_DPAD_UP as usize + k) {
                p.dpad[k] = b as u8 + 1;
            }
        }
        let has = |c: u16| abs.iter().any(|a| a.code == c && a.max > a.min);
        let right_stick = has(ABS_RX) && has(ABS_RY);
        let mut set = |c: u16, role: Role| {
            if let Some(a) = abs.iter().find(|a| a.code == c && a.max > a.min) {
                if (c as usize) < EVDEV_ABS_MAPPED {
                    p.abs[c as usize] = AbsMap { role, min: a.min, max: a.max };
                }
            }
        };
        set(ABS_X, Role::Axis(PAD_LX));
        set(ABS_Y, Role::Axis(PAD_LY));
        set(ABS_HAT0X, Role::HatX);
        set(ABS_HAT0Y, Role::HatY);
        let analog = if right_stick {
            set(ABS_RX, Role::Axis(PAD_RX));
            set(ABS_RY, Role::Axis(PAD_RY));
            if has(ABS_Z) && has(ABS_RZ) {
                set(ABS_Z, Role::Trigger(PAD_LT));
                set(ABS_RZ, Role::Trigger(PAD_RT));
                true
            } else if has(ABS_BRAKE) && has(ABS_GAS) {
                set(ABS_BRAKE, Role::Trigger(PAD_LT));
                set(ABS_GAS, Role::Trigger(PAD_RT));
                true
            } else {
                false
            }
        } else {
            // a simple HID pad: the right stick on Z / RZ
            set(ABS_Z, Role::Axis(PAD_RX));
            set(ABS_RZ, Role::Axis(PAD_RY));
            if has(ABS_BRAKE) && has(ABS_GAS) {
                set(ABS_BRAKE, Role::Trigger(PAD_LT));
                set(ABS_GAS, Role::Trigger(PAD_RT));
                true
            } else {
                false
            }
        };
        p.digital_triggers = !analog;
        for a in abs {
            p.abs_value(a.code, a.value);
        }
        p.state = p.work;
        p
    }

    /// the state of the last complete report (SYN_REPORT)
    pub fn state(&self) -> PadState {
        self.state
    }

    /// the events of the node (whole input_event records, EVDEV_EVENT_SIZE bytes each) -> a
    /// SYN_REPORT changed the state.  After a SYN_DROPPED the events up to the next SYN_REPORT go
    /// and needs_resync says to read the device's state anew
    pub fn feed(&mut self, data: &[u8]) -> bool {
        let mut changed = false;
        for rec in data.chunks_exact(EVDEV_EVENT_SIZE) {
            let typ = u16::from_ne_bytes([rec[16], rec[17]]);
            let code = u16::from_ne_bytes([rec[18], rec[19]]);
            let value = i32::from_ne_bytes([rec[20], rec[21], rec[22], rec[23]]);
            match typ {
                EV_SYN if code == SYN_DROPPED => self.dropped = true,
                EV_SYN if code == SYN_REPORT => {
                    if self.dropped {
                        self.dropped = false;
                        self.resync = true;
                    } else if self.work != self.state {
                        self.state = self.work;
                        changed = true;
                    }
                }
                _ if self.dropped => {}
                EV_KEY => self.key(code, value != 0),
                EV_ABS => self.abs_value(code, value),
                _ => {}
            }
        }
        changed
    }

    /// events were dropped (SYN_DROPPED): the caller reads the keys (EVIOCGKEY) and the axes
    /// (EVIOCGABS) and hands them to resync (once true, then false until the next drop)
    pub fn needs_resync(&mut self) -> bool {
        core::mem::take(&mut self.resync)
    }

    /// the device's state read anew: the keys held (EVIOCGKEY's bitmap), the axes' values -> it
    /// changed
    pub fn resync(&mut self, keys: &[u8], abs: &[AbsInfo]) -> bool {
        let mut w = self.state;
        w.pressed = 0;
        w.buttons = [0.0; PAD_BUTTONS];
        self.work = w;
        for code in BTN_SOUTH..=BTN_THUMBR {
            if bit(keys, code as usize) {
                self.key(code, true);
            }
        }
        for code in BTN_DPAD_UP..=BTN_DPAD_RIGHT {
            if bit(keys, code as usize) {
                self.key(code, true);
            }
        }
        for a in abs {
            self.abs_value(a.code, a.value);
        }
        // (a hat or a trigger without a value given stays at rest)
        self.dropped = false;
        let changed = self.work != self.state;
        self.state = self.work;
        changed
    }

    fn press(&mut self, b: usize, v: f32, held: bool) {
        self.work.buttons[b] = v;
        if held {
            self.work.pressed |= 1 << b;
        } else {
            self.work.pressed &= !(1 << b);
        }
    }

    fn key(&mut self, code: u16, down: bool) {
        let b = if (BTN_SOUTH..=BTN_THUMBR).contains(&code) {
            self.keys[(code - BTN_SOUTH) as usize]
        } else if (BTN_DPAD_UP..=BTN_DPAD_RIGHT).contains(&code) {
            self.dpad[(code - BTN_DPAD_UP) as usize]
        } else {
            0
        };
        if b == 0 {
            return;
        }
        let b = b as usize - 1;
        if (b == PAD_LT || b == PAD_RT) && !self.digital_triggers {
            return; // the analog trigger says it
        }
        self.press(b, if down { 1.0 } else { 0.0 }, down);
    }

    fn abs_value(&mut self, code: u16, v: i32) {
        let Some(m) = self.abs.get(code as usize).copied() else { return };
        let (lo, hi) = (m.min as f64, m.max as f64);
        let v = (v as f64).clamp(lo, hi);
        // a stick: around its middle (exactly 0 at 0 for a range around it), else from its minimum
        let centred = if m.min < 0 && m.max > 0 {
            if v < 0.0 { v / -lo } else { v / hi }
        } else {
            (v - (lo + hi) / 2.0) / ((hi - lo) / 2.0)
        };
        match m.role {
            Role::None => {}
            Role::Axis(a) => self.work.axes[a] = centred as f32,
            Role::Trigger(b) => {
                // (in f32 as XInput's v / 255.0: 30 of 255 is exactly PAD_TRIGGER_PRESS, not above)
                let t = (v as f32 - m.min as f32) / (m.max as f32 - m.min as f32);
                self.press(b, t, t > PAD_TRIGGER_PRESS);
            }
            Role::HatX | Role::HatY => {
                let (neg, pos) = if matches!(m.role, Role::HatX) { (PAD_LEFT, PAD_RIGHT) } else { (PAD_UP, PAD_DOWN) };
                self.press(neg, (centred <= -0.5) as u8 as f32, centred <= -0.5);
                self.press(pos, (centred >= 0.5) as u8 as f32, centred >= 0.5);
            }
        }
    }
}

// ------------------------------------------------------------------ test aids (plattest --test pad)

/// (test aid) the shared state without a platform: pad i came (as a backend says it)
pub fn pad_test_connect(i: usize, name: &str, style: PadStyle) {
    pad_connect(i, name, style, true);
}

/// (test aid) pad i's state now (as a backend reports it)
pub fn pad_test_state(i: usize, st: &PadState) {
    pad_update(i, st);
}

/// (test aid) pad i went
pub fn pad_test_disconnect(i: usize) {
    pad_disconnect(i);
}

/// (test aid) every slot empty, no event waiting, not watching (a backend that watched is stopped
/// first: its PADEV_DISCONNECT events are queued)
pub fn pad_test_reset() {
    if pad_watching() {
        pad_watch(false);
    }
    let mut p = PADS.lock();
    for (i, s) in p.slots.iter_mut().enumerate() {
        s.pad = Gamepad::empty(i as u8);
        s.sent = PadState::NEUTRAL;
    }
    p.queued = false;
    p.watching = false;
}

/// (test aid) the focus the backends see: Some(true) focused, Some(false) not, None the windows'
pub fn pad_test_focus(f: Option<bool>) {
    FOCUS_TEST.store(match f {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    }, Ordering::Relaxed);
}

/// (test aid) poll the native backend now, every slot (Windows: also the empty ones), as the window
/// loop does when the time comes -> false: no native backend (the browser, Android, iOS)
pub fn pad_poll_test() -> bool {
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    {
        pad_os::poll(crate::time_us(), pad_focused(), true);
        true
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    false
}
