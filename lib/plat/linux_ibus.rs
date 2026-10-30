// linux_ibus -- input methods on X11: IBus (and fcitx5's IBus frontend) through the session
// bus portal org.freedesktop.portal.IBus, on a thread of its own (linux_dbus.rs).  Wayland has
// its own path (text-input-v3, linux_wl_ime.rs).
//
//   ibus_focus(win, on, x, y, h)    a text field of window win got (on: its caret at root x,
//                                   y, height h) or lost the keyboard focus: FocusIn +
//                                   SetCursorLocation / FocusOut; the input context is made on
//                                   the first focus
//   ibus_key(keyval, keycode, state) -> bool   ProcessKeyEvent (keyval = X keysym, keycode =
//                                   evdev code, state = X modifier bits, IBUS_RELEASE 1 << 30
//                                   for a release): true = the input method took the key; its
//                                   result arrives as ime_commit_set / ime_preedit_set.  false
//                                   without an input method or after 300 ms
//   ibus_available() -> bool        jfgui_IME=0 turns it off
//   ibus_ready() -> bool            the input context exists (keys go to it only then)
//
// Signals of the context: CommitText -> ime_commit_text, UpdatePreeditText (cursor in
// characters -> bytes) / HidePreeditText -> ime_preedit_set, ForwardKeyEvent -> E_TEXT or the
// key's E_KEY_DOWN / E_KEY_UP.  Capabilities: preedit text and focus (IBus draws its own
// candidate window).  Every result wakes the UI thread.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use rt::os::syscall6;
use rt::sync::SpinLock;

use crate::linux_dbus::{Conn, Msg, T_ERROR, T_RETURN, T_SIGNAL, Value, session_available};

const PORTAL: &str = "org.freedesktop.portal.IBus";
const IC: &str = "org.freedesktop.IBus.InputContext";
/// ProcessKeyEvent state bit of a key release
pub const IBUS_RELEASE: u32 = 1 << 30;
const CAPS: u32 = 1 | 8; // IBUS_CAP_PREEDIT_TEXT | IBUS_CAP_FOCUS
const KEY_WAIT_US: u64 = 300_000;

enum Op {
    Focus(u8, bool, i32, i32, i32),
    Key(u32, u32, u32, u32),
}

static OPS: SpinLock<Vec<Op>> = SpinLock::new(Vec::new());
static WAKE: AtomicI32 = AtomicI32::new(-1);
// 0 not started, 1 starting, 2 ready, 3 no input method
static STATE: AtomicU32 = AtomicU32::new(0);
static KEY_ID: AtomicU32 = AtomicU32::new(0);
// (key request id, answer) of the last ProcessKeyEvent
static ANSWER: SpinLock<(u32, bool)> = SpinLock::new((0, false));

/// whether an input method may be asked (a session bus, not turned off)
pub fn ibus_available() -> bool {
    STATE.load(Ordering::Acquire) != 3 && session_available() && !crate::env("jfgui_IME").is_some_and(|v| v.starts_with('0'))
}

fn queue(op: Op) -> bool {
    if !ibus_available() {
        return false;
    }
    OPS.lock().push(op);
    if STATE.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire).is_ok() {
        // SAFETY: eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK).
        let fd = unsafe { syscall6(290, 0, 0x80000 | 0x800, 0, 0, 0, 0) };
        WAKE.store(fd as i32, Ordering::Release);
        if crate::thread_spawn(128 << 10, Box::new(run)).is_none() {
            STATE.store(3, Ordering::Release);
            return false;
        }
    }
    let fd = WAKE.load(Ordering::Acquire);
    if fd >= 0 {
        let one = 1u64;
        // SAFETY: an 8-byte counter increment on our eventfd.
        unsafe { syscall6(1, fd as usize, &one as *const u64 as usize, 8, 0, 0, 0) };
    }
    true
}

/// whether the input context exists (before that keys are not sent)
pub fn ibus_ready() -> bool {
    STATE.load(Ordering::Acquire) == 2
}

/// a text field got or lost the keyboard focus
pub fn ibus_focus(win: u8, on: bool, x: i32, y: i32, h: i32) {
    queue(Op::Focus(win, on, x, y, h));
}

/// let the input method see a key -> it took it
pub fn ibus_key(keyval: u32, keycode: u32, state: u32) -> bool {
    let id = KEY_ID.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
    if !queue(Op::Key(id, keyval, keycode, state)) {
        return false;
    }
    let end = crate::time_us() + KEY_WAIT_US;
    let mut spins = 0u32;
    while crate::time_us() < end {
        let a = *ANSWER.lock();
        if a.0 == id {
            return a.1;
        }
        if STATE.load(Ordering::Acquire) == 3 {
            return false;
        }
        spins += 1;
        if spins < 200 { crate::thread_yield() } else { crate::sleep_ms(1) }
    }
    false
}

// ---------------------------------------------------------------- the thread

struct Ctx {
    path: String,
    win: u8,
    focused: bool,
    /// the input context is gone (ibus restarted, another engine): made anew
    stale: bool,
    /// Key request id of a ProcessKeyEvent serial
    keys: Vec<(u32, u32)>,
}

fn answer(id: u32, took: bool) {
    *ANSWER.lock() = (id, took);
}

fn run() {
    let Some(mut c) = Conn::session() else {
        STATE.store(3, Ordering::Release);
        return;
    };
    let Some(path) = create(&mut c) else {
        STATE.store(3, Ordering::Release);
        return;
    };
    STATE.store(2, Ordering::Release);
    let mut ctx = Ctx { path, win: 0, focused: false, stale: false, keys: Vec::new() };
    loop {
        if ctx.stale {
            // the old context's errors answered its keys; a new one, focused as before
            ctx.stale = false;
            match create(&mut c) {
                Some(p) => {
                    ctx.path = p;
                    if ctx.focused {
                        c.send(&Msg::call(PORTAL, &ctx.path, IC, "FocusIn", Vec::new()));
                    }
                }
                None => {
                    STATE.store(3, Ordering::Release); // ibus is gone: keys go on without it
                    return;
                }
            }
        }
        let ops = core::mem::take(&mut *OPS.lock());
        for op in ops {
            apply(&mut c, &mut ctx, op);
        }
        while let Some(m) = c.read(0) {
            handle(&mut ctx, &m);
        }
        if c.broken {
            STATE.store(3, Ordering::Release);
            for (id, _) in ctx.keys.drain(..) {
                answer(id, false);
            }
            return;
        }
        let wake = WAKE.load(Ordering::Acquire);
        let mut pfd = [c.fd() as u32 as u64 | (1u64 << 32), wake as u32 as u64 | (1u64 << 32)];
        // SAFETY: two pollfd records.
        unsafe { syscall6(7, pfd.as_mut_ptr() as usize, 2, usize::MAX, 0, 0, 0) };
        let mut n = 0u64;
        // SAFETY: drain the eventfd counter (nonblocking).
        unsafe { syscall6(0, wake as usize, &mut n as *mut u64 as usize, 8, 0, 0, 0) };
    }
}

// CreateInputContext, the match for its signals, its capabilities -> its path
fn create(c: &mut Conn) -> Option<String> {
    let create = Msg::call(PORTAL, "/org/freedesktop/IBus", "org.freedesktop.IBus.Portal", "CreateInputContext", alloc::vec![Value::str("jfgui")]);
    let path = c.send(&create).and_then(|s| c.wait_reply(s, 3000)).and_then(|r| r.body.first().and_then(|v| v.as_str()).map(String::from))?;
    let rule = alloc::format!("type='signal',interface='{IC}',path='{path}'");
    c.send(&Msg::call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "AddMatch", alloc::vec![Value::S(rule)]));
    c.send(&Msg::call(PORTAL, &path, IC, "SetCapabilities", alloc::vec![Value::U(CAPS)]));
    Some(path)
}

fn apply(c: &mut Conn, ctx: &mut Ctx, op: Op) {
    match op {
        Op::Focus(win, true, x, y, h) => {
            if !ctx.focused || ctx.win != win {
                ctx.win = win;
                ctx.focused = true;
                c.send(&Msg::call(PORTAL, &ctx.path, IC, "FocusIn", Vec::new()));
            }
            c.send(&Msg::call(PORTAL, &ctx.path, IC, "SetCursorLocation", alloc::vec![Value::I(x), Value::I(y), Value::I(1), Value::I(h)]));
        }
        Op::Focus(win, false, ..) => {
            if ctx.focused {
                ctx.focused = false;
                c.send(&Msg::call(PORTAL, &ctx.path, IC, "FocusOut", Vec::new()));
                crate::ime_preedit_set(win, "", 0);
                crate::wake();
            }
        }
        Op::Key(id, keyval, keycode, state) => {
            let body = alloc::vec![Value::U(keyval), Value::U(keycode), Value::U(state)];
            match c.send(&Msg::call(PORTAL, &ctx.path, IC, "ProcessKeyEvent", body)) {
                Some(serial) => ctx.keys.push((serial, id)),
                None => answer(id, false),
            }
        }
    }
}

// the text of an IBusText variant: (sa{sv}sv) with the text third
fn ibus_text(v: Option<&Value>) -> Option<String> {
    let mut v = v?;
    while let Value::V(inner) = v {
        v = inner;
    }
    match v {
        Value::R(f) => f.get(2).and_then(|t| t.as_str()).map(String::from),
        _ => None,
    }
}

fn handle(ctx: &mut Ctx, m: &Msg) {
    match m.typ {
        T_RETURN | T_ERROR => {
            if let Some(k) = ctx.keys.iter().position(|k| k.0 == m.reply_to) {
                let (_, id) = ctx.keys.remove(k);
                let took = m.typ == T_RETURN && matches!(m.body.first(), Some(Value::B(true)));
                answer(id, took);
                // the context or the portal went away (ibus restart): make a new one
                if m.typ == T_ERROR && [".UnknownObject", ".UnknownMethod", ".ServiceUnknown"]
                    .iter().any(|e| m.error.ends_with(e)) {
                    ctx.stale = true;
                }
            }
        }
        T_SIGNAL if m.iface == IC && m.path == ctx.path => match m.member.as_str() {
            "CommitText" => {
                if let Some(t) = ibus_text(m.body.first()) {
                    crate::ime_preedit_set(ctx.win, "", 0);
                    crate::ime_commit_text(ctx.win, &t); // longer than one commit: in parts
                    crate::wake();
                }
            }
            "UpdatePreeditText" | "UpdatePreeditTextWithMode" => {
                let text = ibus_text(m.body.first()).unwrap_or_default();
                let chars = m.body.get(1).and_then(|v| v.as_u32()).unwrap_or(0) as usize;
                let visible = matches!(m.body.get(2), Some(Value::B(true)));
                let cursor = text.char_indices().nth(chars).map_or(text.len(), |(i, _)| i);
                crate::ime_preedit_set(ctx.win, if visible { &text } else { "" }, if visible { cursor } else { 0 });
                crate::wake();
            }
            "HidePreeditText" => {
                crate::ime_preedit_set(ctx.win, "", 0);
                crate::wake();
            }
            "ForwardKeyEvent" => {
                let keyval = m.body.first().and_then(|v| v.as_u32()).unwrap_or(0);
                let state = m.body.get(2).and_then(|v| v.as_u32()).unwrap_or(0);
                forward(ctx.win, keyval, state & IBUS_RELEASE != 0);
            }
            _ => {}
        },
        _ => {}
    }
}

// a key the input method hands back: text, or the VK_* of the common editing keys
fn forward(win: u8, keyval: u32, release: bool) {
    let vk = match keyval {
        0xFF08 => 8,   // BackSpace
        0xFF09 => 9,   // Tab
        0xFF0D => 13,  // Return
        0xFF1B => 27,  // Escape
        0xFF50 => 36,  // Home
        0xFF51 => 37,  // Left
        0xFF52 => 38,  // Up
        0xFF53 => 39,  // Right
        0xFF54 => 40,  // Down
        0xFF55 => 33,  // Prior
        0xFF56 => 34,  // Next
        0xFF57 => 35,  // End
        0xFFFF => 46,  // Delete
        _ => 0,
    };
    let ev = |typ: u8, data: u32| crate::Event { typ, win, data, ..Default::default() };
    if vk != 0 {
        crate::evq_push(ev(if release { crate::E_KEY_UP } else { crate::E_KEY_DOWN }, vk));
    } else if !release {
        let cp = if (0x20..0x7F).contains(&keyval) || (0xA0..0x100).contains(&keyval) {
            keyval
        } else if keyval & 0xFF00_0000 == 0x0100_0000 && char::from_u32(keyval & 0x00FF_FFFF).is_some() {
            keyval & 0x00FF_FFFF
        } else {
            0
        };
        if cp != 0 {
            crate::evq_push(ev(crate::E_TEXT, cp));
        }
    }
    crate::wake();
}
