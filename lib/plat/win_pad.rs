// win_pad -- (G7a part 2) game pads on Windows: XInput (the shared part: pad.rs)
//
//   start() -> bool             load XInput (xinput1_4.dll, else xinput9_1_0.dll, from System32),
//                               look at all four slots
//   stop()                      every pad goes
//   poll(now, focused, full) -> next   the connected slots' states (pad_update when their packet
//                               number moved or the focus changed), one empty slot every 250 ms
//                               (XInputGetState is slow on an empty one), all with full
//   xinput_test_feed(slot, rec) / xinput_test_end()   (tests) XINPUT_STATE records instead of the
//                               DLL's
//
// XInput knows Xbox-compatible pads only (Xbox 360, One, Series and the pads that act as one; other
// HID pads come later), at most four; the index of a pad is its XInput slot.  XInputGetStateEx
// (ordinal 100 of xinput1_4.dll) also reports the guide button (PAD_HOME); xinput9_1_0.dll has no
// such ordinal, there XInputGetState without it.  XInput reads the pads whoever has the focus: the
// state is neutral while the app is not active (pad.rs).

use rt::sync::SpinLock;

use crate::{PadState, PadStyle, pad_connect, pad_disconnect, pad_update, xinput_decode};

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LoadLibraryExA(name: *const u8, file: isize, flags: u32) -> isize;
    fn GetProcAddress(module: isize, name: *const u8) -> usize;
}

const SLOTS: usize = 4;
const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x800;
const ERROR_SUCCESS: u32 = 0;
const XINPUT_FLAG_GAMEPAD: u32 = 1;
/// XINPUT_DEVSUBTYPE_GAMEPAD
const SUBTYPE_GAMEPAD: u8 = 1;
/// how often a connected pad is polled, how often an empty slot is looked at (µs)
const POLL_US: u64 = 4_000;
const SCAN_US: u64 = 250_000;

struct Xi {
    /// XInputGetState(Ex), XInputGetCapabilities (0: not loaded)
    get: usize,
    caps: usize,
    connected: [bool; SLOTS],
    /// the packet number of the state last reported, and whether the app had the focus then
    packet: [u32; SLOTS],
    focused: bool,
    next_scan: u64,
    scan: usize,
    /// (tests) XINPUT_STATE records instead of the DLL (None: not connected)
    fake: Option<[Option<[u8; 16]>; SLOTS]>,
}

static XI: SpinLock<Xi> = SpinLock::new(Xi { get: 0, caps: 0, connected: [false; SLOTS], packet: [0; SLOTS], focused: false, next_scan: 0, scan: 0, fake: None });

/// the DLL's address of a function by name or ordinal (name: a NUL-terminated name, or the ordinal
/// as a pointer value below 0x10000)
fn proc(m: isize, name: *const u8) -> usize {
    // SAFETY: m is a loaded module; name is NUL-terminated or an ordinal (MAKEINTRESOURCE).
    unsafe { GetProcAddress(m, name) }
}

/// load XInput and look at every slot -> false: no XInput on this system
pub(crate) fn start() -> bool {
    {
        let mut x = XI.lock();
        if x.get == 0 && x.fake.is_none() {
            for dll in [&b"xinput1_4.dll\0"[..], &b"xinput9_1_0.dll\0"[..]] {
                // SAFETY: a NUL-terminated name, from System32 only.
                let m = unsafe { LoadLibraryExA(dll.as_ptr(), 0, LOAD_LIBRARY_SEARCH_SYSTEM32) };
                if m == 0 {
                    continue;
                }
                let ex = proc(m, 100 as *const u8); // XInputGetStateEx
                x.get = if ex != 0 { ex } else { proc(m, b"XInputGetState\0".as_ptr()) };
                x.caps = proc(m, b"XInputGetCapabilities\0".as_ptr());
                if x.get != 0 {
                    break;
                }
            }
            if x.get == 0 {
                return false;
            }
        }
        x.connected = [false; SLOTS];
        x.next_scan = 0;
    }
    poll(crate::time_us(), crate::pad_focused(), true);
    true
}

/// every pad goes
pub(crate) fn stop() {
    let was = core::mem::replace(&mut XI.lock().connected, [false; SLOTS]);
    for (i, c) in was.into_iter().enumerate() {
        if c {
            pad_disconnect(i);
        }
    }
}

/// XINPUT_STATE of slot i as XInputGetState fills it (None: not connected)
fn read(x: &Xi, i: usize) -> Option<[u8; 16]> {
    if let Some(f) = &x.fake {
        return f[i];
    }
    if x.get == 0 {
        return None;
    }
    // XINPUT_STATE is 16 bytes; XInputGetStateEx's record has a reserved DWORD more (20): room for
    // both, the first 16 are the same
    let mut rec = [0u8; 24];
    // SAFETY: XInputGetState(Ex)(DWORD, XINPUT_STATE(_EX)*): 24 writable bytes, more than either
    // writes.
    let r = unsafe {
        let f: unsafe extern "system" fn(u32, *mut u8) -> u32 = core::mem::transmute(x.get);
        f(i as u32, rec.as_mut_ptr())
    };
    let mut out = [0u8; 16];
    out.copy_from_slice(&rec[..16]);
    (r == ERROR_SUCCESS).then_some(out)
}

/// the name of the pad in slot i (XInputGetCapabilities' subtype)
fn name(x: &Xi, i: usize) -> &'static str {
    if x.fake.is_some() || x.caps == 0 {
        return "Xbox Controller (XInput)";
    }
    let mut caps = [0u8; 20]; // XINPUT_CAPABILITIES: Type, SubType, Flags, XINPUT_GAMEPAD, XINPUT_VIBRATION
    // SAFETY: XInputGetCapabilities(DWORD, DWORD, XINPUT_CAPABILITIES*): 20 writable bytes.
    let r = unsafe {
        let f: unsafe extern "system" fn(u32, u32, *mut u8) -> u32 = core::mem::transmute(x.caps);
        f(i as u32, XINPUT_FLAG_GAMEPAD, caps.as_mut_ptr())
    };
    match (r, caps[1]) {
        (ERROR_SUCCESS, SUBTYPE_GAMEPAD) | (ERROR_SUCCESS, 0) => "Xbox Controller (XInput)",
        (ERROR_SUCCESS, 2) => "XInput Wheel",
        (ERROR_SUCCESS, 3) | (ERROR_SUCCESS, 0x13) => "XInput Arcade Stick",
        (ERROR_SUCCESS, 4) => "XInput Flight Stick",
        (ERROR_SUCCESS, 5) => "XInput Dance Pad",
        (ERROR_SUCCESS, 6) | (ERROR_SUCCESS, 7) | (ERROR_SUCCESS, 0x0B) => "XInput Guitar",
        (ERROR_SUCCESS, 8) => "XInput Drum Kit",
        _ => "XInput Controller",
    }
}

/// poll: the connected slots, an empty one when its turn has come (every slot with full) -> the
/// next time to poll
pub(crate) fn poll(now: u64, focused: bool, full: bool) -> u64 {
    let mut x = XI.lock();
    let refocus = focused != x.focused;
    x.focused = focused;
    let scan_due = full || now >= x.next_scan;
    let mut scanned = false;
    let mut events: [Option<(bool, Option<PadState>, &'static str)>; SLOTS] = [None; SLOTS];
    for i in 0..SLOTS {
        let look = x.connected[i] || full || (scan_due && !scanned && i >= x.scan);
        if !look {
            continue;
        }
        if !x.connected[i] {
            scanned = true;
            x.scan = i + 1;
        }
        let rec = read(&x, i);
        match (x.connected[i], rec) {
            (false, None) => {}
            (true, None) => {
                x.connected[i] = false;
                events[i] = Some((false, None, ""));
            }
            (was, Some(rec)) => {
                let Some((packet, st)) = xinput_decode(&rec) else { continue };
                if was && packet == x.packet[i] && !refocus {
                    continue;
                }
                x.packet[i] = packet;
                x.connected[i] = true;
                let st = if focused { st } else { PadState::NEUTRAL };
                events[i] = Some((true, Some(st), if was { "" } else { name(&x, i) }));
            }
        }
    }
    if scan_due {
        // the next empty slot after the one looked at; a round over all four ends where it began
        if !scanned || x.scan >= SLOTS {
            x.scan = 0;
        }
        x.next_scan = now + SCAN_US;
    }
    let any = x.connected.iter().any(|&c| c);
    let next = if any { (now + POLL_US).min(x.next_scan) } else { x.next_scan };
    drop(x);
    for (i, e) in events.into_iter().enumerate() {
        match e {
            Some((false, _, _)) => pad_disconnect(i),
            Some((true, Some(st), name)) => {
                if !name.is_empty() {
                    pad_connect(i, name, PadStyle::Xbox, true);
                }
                pad_update(i, &st);
            }
            _ => {}
        }
    }
    next
}

/// (test aid, plattest --test pad) slot `slot` reports this XINPUT_STATE record (16 bytes: packet
/// number, buttons, triggers, thumbs) from now on instead of the DLL's (None: not connected); the
/// other slots of the first call are empty.  The backend polls as it would the DLL
/// (plat::pad_poll_test, or the window loop)
pub fn xinput_test_feed(slot: usize, rec: Option<[u8; 16]>) {
    if slot >= SLOTS {
        return;
    }
    let mut x = XI.lock();
    let f = x.fake.get_or_insert([None; SLOTS]);
    f[slot] = rec;
}

/// (test aid) the DLL again (pad_watch(false) first: the slots' state is forgotten)
pub fn xinput_test_end() {
    let mut x = XI.lock();
    x.fake = None;
    x.connected = [false; SLOTS];
    x.packet = [0; SLOTS];
}
