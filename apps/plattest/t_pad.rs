// t_pad -- (G7a part 2) game pads without real input: the shared state (events, edges, the
// threshold of the axes, values kept in range), the decoders of every platform (XInput's records,
// the evdev parser with Xbox, PlayStation and simple HID pads, the browser's record, styles and
// vendors), and per platform the backend's own path fed by a test hook: Windows XINPUT_STATE
// records instead of the DLL's, Linux a pipe as a pad's node, macOS a controller of GameController's
// own (controllerWithExtendedGamepad)
//
//   plattest --test pad
//   plattest --test pad-policy   a manifest without `gamepad`: pad_watch refuses, PERMDENY gamepad
//   plattest --test pad-ask      `ask gamepad`: refused while nobody answers, watching after a yes

use alloc::format;
#[cfg(target_os = "macos")]
use alloc::string::String;
use alloc::vec::Vec;

use plat::*;

use crate::T;

pub fn t_pad(t: &mut T) {
    shared(t);
    names(t);
    xinput(t);
    evdev(t);
    web(t);
    backend(t);
}

/// the events in the queue: (index, kind) of each E_GAMEPAD, the others dropped
fn pad_events() -> Vec<(u32, u32)> {
    let mut v = Vec::new();
    while let Some(e) = evq_pop() {
        if e.typ == E_GAMEPAD {
            v.push((e.data & 0xFF, e.data >> 8));
        }
    }
    v
}

fn st(pressed: &[usize], axes: [f32; 4]) -> PadState {
    let mut s = PadState { axes, ..PadState::NEUTRAL };
    for &b in pressed {
        s.buttons[b] = 1.0;
        s.pressed |= 1 << b;
    }
    s
}

// ------------------------------------------------------------------ the shared state

fn shared(t: &mut T) {
    pad_test_reset();
    evq_clear();
    pad_test_connect(2, "Test Pad", PadStyle::Xbox);
    t.check(pad_events() == [(2, PADEV_CONNECT)], "pad: a connect is an event (index 2, PADEV_CONNECT)");
    let v = pads();
    t.check(v.len() == 1 && v[0].index == 2 && v[0].connected && v[0].name == "Test Pad" && v[0].style == PadStyle::Xbox && v[0].mapped && v[0].pressed == 0,
            "pad: pads() lists it, nothing held");
    t.check(!pad(1).connected && pad(2).connected && pad(99).index == 0, "pad: pad(i) of an empty slot, of a connected one, out of range");
    // a press, then its release before anybody read: one event, both edges, not held
    pad_test_state(2, &st(&[PAD_A], [0.0; 4]));
    pad_test_state(2, &st(&[], [0.0; 4]));
    t.check(pad_events() == [(2, PADEV_STATE)], "pad: a press and a release before a read: one PADEV_STATE");
    let v = pads();
    t.check(v[0].down == 1 << PAD_A && v[0].up == 1 << PAD_A && v[0].pressed == 0 && v[0].went_down(PAD_A) && !v[0].held(PAD_A),
            "pad: the tap is in down and up, not in pressed");
    t.check(pads()[0].down == 0 && pads()[0].up == 0, "pad: pads() took the edges");
    // held across reads: the edge once, the hold stays
    pad_test_state(2, &st(&[PAD_RB], [0.0; 4]));
    let v = pads();
    t.check(v[0].held(PAD_RB) && v[0].went_down(PAD_RB) && pad_events().len() == 1, "pad: held: down once");
    let v = pads();
    t.check(v[0].held(PAD_RB) && v[0].down == 0, "pad: still held, no new edge");
    // an axis: jitter below 1/256 wakes nobody but is in the snapshot; a step wakes; back to rest wakes
    evq_clear();
    pad_test_state(2, &st(&[PAD_RB], [0.003, 0.0, 0.0, 0.0]));
    t.check(pad_events().is_empty() && pad(2).axes[PAD_LX] == 0.003, "pad: a stick moving by 0.003: no event, the snapshot has it");
    pad_test_state(2, &st(&[PAD_RB], [0.5, -0.25, 0.0, 0.0]));
    t.check(pad_events() == [(2, PADEV_STATE)], "pad: a stick moving by 0.5: an event");
    pads();
    pad_test_state(2, &st(&[PAD_RB], [0.499, -0.25, 0.0, 0.0]));
    t.check(pad_events().is_empty(), "pad: 0.001 from the last event's value: none");
    pad_test_state(2, &st(&[PAD_RB], [0.0, -0.25, 0.0, 0.0]));
    t.check(pad_events() == [(2, PADEV_STATE)], "pad: back to rest: an event");
    pads();
    pad_test_state(2, &st(&[PAD_RB], [0.0, -0.25, 0.0, 0.0]));
    t.check(pad_events().is_empty(), "pad: the same state again: nothing");
    // a second pad changing while the first's event waits: still one event
    pad_test_connect(5, "Second", PadStyle::PlayStation);
    pad_events();
    pad_test_state(2, &st(&[], [0.0; 4]));
    pad_test_state(5, &st(&[PAD_Y], [0.0; 4]));
    t.check(pad_events() == [(2, PADEV_STATE)], "pad: two pads changing before a read: one event");
    let v = pads();
    t.check(v.len() == 2 && v[0].index == 2 && v[1].index == 5 && v[1].held(PAD_Y) && v[0].went_up(PAD_RB), "pad: both in index order with their changes");
    // values out of range, NaN, bits beyond the layout
    let mut bad = PadState::NEUTRAL;
    bad.buttons[PAD_LT] = 2.0;
    bad.buttons[PAD_RT] = f32::NAN;
    bad.axes = [-3.0, f32::NAN, 1.5, 0.25];
    bad.pressed = 1 << 20 | 1 << PAD_LT;
    pad_test_state(5, &bad);
    let p = pad(5);
    t.check(p.buttons[PAD_LT] == 1.0 && p.buttons[PAD_RT] == 0.0 && p.axes == [-1.0, 0.0, 1.0, 0.25] && p.pressed == 1 << PAD_LT, "pad: values kept in range, NaN as 0, only the layout's bits");
    t.check(p.stick(0) == [-1.0, 0.0] && p.stick(1) == [1.0, 0.25] && p.trigger(0) == 1.0 && p.value(99) == 0.0 && p.axis(9) == 0.0, "pad: stick, trigger, value, axis");
    // a disconnect
    pads();
    pad_events();
    pad_test_disconnect(5);
    t.check(pad_events() == [(5, PADEV_DISCONNECT)] && !pad(5).connected && pads().len() == 1, "pad: a disconnect: its event, the slot empty");
    pad_test_disconnect(5);
    t.check(pad_events().is_empty(), "pad: a second disconnect: nothing");
    pad_test_state(5, &st(&[PAD_A], [0.0; 4]));
    t.check(pad_events().is_empty() && !pad(5).connected, "pad: a state of an empty slot goes nowhere");
    pad_test_connect(9, "beyond", PadStyle::Other);
    t.check(pad_events().is_empty(), "pad: index 9: nothing");
    pad_test_reset();
    evq_clear();
    t.check(pads().is_empty() && !pad_watching(), "pad: reset");
}

fn names(t: &mut T) {
    let n: Vec<&str> = (0..PAD_BUTTONS).map(pad_button_name).collect();
    t.check(n.join(" ") == "A B X Y LB RB LT RT Back Start LS RS Up Down Left Right Home" && pad_button_name(17).is_empty(), "pad: the buttons' names");
    t.check(PAD_HOME == 16 && PAD_BUTTONS == 17 && PAD_RY == 3 && PAD_AXES == 4, "pad: the standard layout's numbers");
    let styles = [(0x045E, "", PadStyle::Xbox), (0x054C, "", PadStyle::PlayStation), (0x057E, "", PadStyle::Nintendo), (0, "Xbox 360 Controller (XInput STANDARD GAMEPAD)", PadStyle::Xbox),
                  (0, "DualSense Wireless Controller", PadStyle::PlayStation), (0, "Pro Controller", PadStyle::Nintendo), (0x2DC8, "8BitDo SN30", PadStyle::Other)];
    let bad: Vec<_> = styles.iter().filter(|(v, n, s)| pad_style(*v, n) != *s).collect();
    t.check(bad.is_empty(), &format!("pad: styles from vendor and name {bad:?}"));
    t.check(pad_vendor_of_id("DualSense Wireless Controller (STANDARD GAMEPAD Vendor: 054c Product: 0ce6)") == 0x054C, "pad: Chrome's id: Vendor: 054c");
    t.check(pad_vendor_of_id("045e-02ea-Microsoft X-Box One S pad") == 0x045E, "pad: Firefox's id: 045e-...");
    t.check(pad_vendor_of_id("xinput") == 0 && pad_vendor_of_id("") == 0 && pad_vendor_of_id("zzzz-1") == 0, "pad: ids without a vendor");
}

// ------------------------------------------------------------------ XInput

fn xinput(t: &mut T) {
    // every button bit to its place
    let bits = [(0x1000u16, PAD_A), (0x2000, PAD_B), (0x4000, PAD_X), (0x8000, PAD_Y), (0x0100, PAD_LB), (0x0200, PAD_RB), (0x0020, PAD_BACK), (0x0010, PAD_START),
                (0x0040, PAD_LS), (0x0080, PAD_RS), (0x0001, PAD_UP), (0x0002, PAD_DOWN), (0x0004, PAD_LEFT), (0x0008, PAD_RIGHT), (0x0400, PAD_HOME)];
    let bad: Vec<_> = bits.iter().filter(|&&(bit, b)| {
        let s = xinput_map(bit, 0, 0, 0, 0, 0, 0);
        s.pressed != 1 << b || s.buttons[b] != 1.0 || s.buttons.iter().filter(|&&v| v != 0.0).count() != 1
    }).collect();
    t.check(bad.is_empty(), &format!("pad xinput: each button bit {bad:x?}"));
    let s = xinput_map(0, 30, 31, 0, 0, 0, 0);
    t.check(s.buttons[PAD_LT] == 30.0 / 255.0 && s.pressed == 1 << PAD_RT, "pad xinput: triggers analog, pressed above 30 (XINPUT_GAMEPAD_TRIGGER_THRESHOLD)");
    let s = xinput_map(0, 255, 0, -32768, 32767, 32767, -32768);
    t.check(s.buttons[PAD_LT] == 1.0 && s.axes == [-1.0, -1.0, 1.0, 1.0], "pad xinput: the thumbs' ends; up is negative y in the layout");
    let s = xinput_map(0, 0, 0, 16384, -16384, 0, 0);
    t.check((s.axes[0] - 0.5).abs() < 1e-4 && s.axes[1] == 0.5 && s.axes[2] == 0.0, "pad xinput: half way");
    // the record XInputGetState fills
    let mut rec = [0u8; 16];
    rec[..4].copy_from_slice(&77u32.to_le_bytes());
    rec[4..6].copy_from_slice(&0x1010u16.to_le_bytes()); // A, Start
    rec[6] = 255;
    rec[8..10].copy_from_slice(&(-32768i16).to_le_bytes());
    rec[14..16].copy_from_slice(&32767i16.to_le_bytes());
    let d = xinput_decode(&rec);
    t.check(d.is_some_and(|(n, s)| n == 77 && s.pressed == (1 << PAD_A | 1 << PAD_START | 1 << PAD_LT) && s.axes == [-1.0, 0.0, 0.0, -1.0]), "pad xinput: XINPUT_STATE decoded");
    t.check(xinput_decode(&rec[..15]).is_none(), "pad xinput: a short record: none");
}

// ------------------------------------------------------------------ evdev

fn bits(n: usize, set: &[u16]) -> Vec<u8> {
    let mut b = alloc::vec![0u8; n / 8];
    for &c in set {
        b[c as usize / 8] |= 1 << (c % 8);
    }
    b
}

fn ev(typ: u16, code: u16, value: i32) -> [u8; EVDEV_EVENT_SIZE] {
    let mut r = [0u8; EVDEV_EVENT_SIZE];
    r[16..18].copy_from_slice(&typ.to_ne_bytes());
    r[18..20].copy_from_slice(&code.to_ne_bytes());
    r[20..24].copy_from_slice(&value.to_ne_bytes());
    r
}

fn evs(list: &[(u16, u16, i32)]) -> Vec<u8> {
    list.iter().flat_map(|&(a, b, c)| ev(a, b, c)).collect()
}

const KEY: u16 = 1;
const ABS: u16 = 3;
const SYN: (u16, u16, i32) = (0, 0, 0);
const TYPES: [u16; 3] = [0, 1, 3];
/// the codes of an xpad pad (Xbox 360): BTN_SOUTH .. BTN_THUMBR without BTN_C / BTN_Z / BTN_TL2 / BTN_TR2
const XPAD_KEYS: [u16; 11] = [0x130, 0x131, 0x133, 0x134, 0x136, 0x137, 0x13A, 0x13B, 0x13C, 0x13D, 0x13E];

fn xpad_abs() -> Vec<AbsInfo> {
    let s = |code, min, max| AbsInfo { code, value: 0, min, max };
    alloc::vec![s(0, -32768, 32767), s(1, -32768, 32767), s(3, -32768, 32767), s(4, -32768, 32767), s(2, 0, 255), s(5, 0, 255), s(16, -1, 1), s(17, -1, 1)]
}

fn evdev(t: &mut T) {
    let types = bits(32, &TYPES);
    let keys = bits(EVDEV_KEYS, &XPAD_KEYS);
    let abs_bits = bits(EVDEV_ABS, &[0, 1, 2, 3, 4, 5, 16, 17]);
    t.check(evdev_is_pad(&types, &keys, &abs_bits), "pad evdev: an xpad node is a pad");
    // a pad's touchpad (BTN_TOUCH, BTN_LEFT, ABS_X/Y, ABS_MT_*), its motion sensors, a keyboard, a mouse
    t.check(!evdev_is_pad(&types, &bits(EVDEV_KEYS, &[0x14A, 0x110]), &bits(EVDEV_ABS, &[0, 1, 0x35, 0x36])), "pad evdev: a touchpad is none");
    t.check(!evdev_is_pad(&bits(32, &[0, 3]), &bits(EVDEV_KEYS, &[]), &bits(EVDEV_ABS, &[0, 1, 2, 3, 4, 5])), "pad evdev: motion sensors are none");
    t.check(!evdev_is_pad(&bits(32, &[0, 1, 17]), &bits(EVDEV_KEYS, &[1, 30, 57]), &bits(EVDEV_ABS, &[])), "pad evdev: a keyboard is none");
    t.check(!evdev_is_pad(&bits(32, &[0, 1, 2]), &bits(EVDEV_KEYS, &[0x110, 0x111]), &bits(EVDEV_ABS, &[])), "pad evdev: a mouse is none");
    t.check(!evdev_is_pad(&types, &keys, &bits(EVDEV_ABS, &[0])), "pad evdev: no ABS_Y: none");

    // xpad (Xbox 360): BTN_NORTH (= BTN_X) is X, BTN_WEST (= BTN_Y) is Y
    let mut p = EvdevPad::new(0x045E, &keys, &xpad_abs());
    t.check(p.state() == PadState::NEUTRAL, "pad evdev: xpad at rest");
    t.check(!p.feed(&evs(&[(KEY, 0x130, 1)])), "pad evdev: no SYN_REPORT: nothing yet");
    t.check(p.state().pressed == 0, "pad evdev: still nothing");
    t.check(p.feed(&evs(&[SYN])) && p.state().pressed == 1 << PAD_A, "pad evdev: BTN_SOUTH + SYN_REPORT: A");
    p.feed(&evs(&[(KEY, 0x130, 0), (KEY, 0x133, 1), (KEY, 0x134, 1), SYN]));
    t.check(p.state().pressed == (1 << PAD_X | 1 << PAD_Y), "pad evdev: xpad's BTN_X / BTN_Y are X and Y");
    p.feed(&evs(&[(KEY, 0x133, 0), (KEY, 0x134, 0), (KEY, 0x136, 1), (KEY, 0x13A, 1), (KEY, 0x13B, 1), (KEY, 0x13C, 1), (KEY, 0x13D, 1), SYN]));
    t.check(p.state().pressed == (1 << PAD_LB | 1 << PAD_BACK | 1 << PAD_START | 1 << PAD_HOME | 1 << PAD_LS), "pad evdev: shoulders, Back, Start, Home, stick");
    p.feed(&evs(&[(KEY, 0x136, 0), (KEY, 0x13A, 0), (KEY, 0x13B, 0), (KEY, 0x13C, 0), (KEY, 0x13D, 0), SYN]));
    // sticks: -32768 .. 32767 around 0 (down positive as the kernel reports it)
    p.feed(&evs(&[(ABS, 0, -32768), (ABS, 1, 32767), (ABS, 3, 16384), (ABS, 4, 0), SYN]));
    let s = p.state();
    t.check(s.axes[0] == -1.0 && s.axes[1] == 1.0 && (s.axes[2] - 0.5).abs() < 1e-4 && s.axes[3] == 0.0, &format!("pad evdev: sticks {:?}", s.axes));
    // triggers on ABS_Z / ABS_RZ (0..255), pressed above 30
    p.feed(&evs(&[(ABS, 2, 255), (ABS, 5, 30), SYN]));
    let s = p.state();
    t.check(s.buttons[PAD_LT] == 1.0 && (s.buttons[PAD_RT] - 30.0 / 255.0).abs() < 1e-7 && s.pressed == 1 << PAD_LT, "pad evdev: triggers analog, pressed above 30/255");
    // the hat as the directional pad
    p.feed(&evs(&[(ABS, 2, 0), (ABS, 5, 0), (ABS, 16, -1), (ABS, 17, 1), SYN]));
    t.check(p.state().pressed == (1 << PAD_LEFT | 1 << PAD_DOWN) && p.state().buttons[PAD_LEFT] == 1.0, "pad evdev: ABS_HAT0X -1, ABS_HAT0Y 1: left, down");
    p.feed(&evs(&[(ABS, 16, 1), (ABS, 17, -1), SYN]));
    t.check(p.state().pressed == (1 << PAD_RIGHT | 1 << PAD_UP), "pad evdev: right, up");
    p.feed(&evs(&[(ABS, 16, 0), (ABS, 17, 0), SYN]));
    t.check(p.state().pressed == 0, "pad evdev: the hat at rest");
    // events that change nothing: no change reported
    t.check(!p.feed(&evs(&[(ABS, 16, 0), SYN])), "pad evdev: a report without a change: false");
    // SYN_DROPPED: the rest up to the next report goes, then a resync
    t.check(!p.feed(&evs(&[(KEY, 0x131, 1), (0, 3, 0), (KEY, 0x130, 1), SYN])), "pad evdev: SYN_DROPPED: nothing applied");
    t.check(p.state().pressed == 0 && p.needs_resync() && !p.needs_resync(), "pad evdev: a resync is needed (once)");
    let held = bits(EVDEV_KEYS, &[0x131, 0x137]);
    let now = [AbsInfo { code: 0, value: 32767, min: -32768, max: 32767 }, AbsInfo { code: 17, value: -1, min: -1, max: 1 }];
    t.check(p.resync(&held, &now) && p.state().pressed == (1 << PAD_B | 1 << PAD_RB | 1 << PAD_UP) && p.state().axes[0] == 1.0, "pad evdev: the state read anew");
    // a short record at the end is left out; codes the pad lacks change nothing
    let mut data = evs(&[(KEY, 0x131, 0), (KEY, 0x2C0, 1), (ABS, 40, 5), SYN]);
    data.extend_from_slice(&[1, 2, 3]);
    p.feed(&data);
    t.check(p.state().pressed == (1 << PAD_RB | 1 << PAD_UP), "pad evdev: unknown codes and a partial record ignored");

    // Sony (hid-playstation): BTN_NORTH is the triangle (Y), BTN_WEST the square (X); sticks and
    // triggers 0..255; BTN_TL2 / BTN_TR2 besides the analog triggers do nothing
    let skeys = bits(EVDEV_KEYS, &[0x130, 0x131, 0x133, 0x134, 0x136, 0x137, 0x138, 0x139, 0x13A, 0x13B, 0x13C, 0x13D, 0x13E]);
    let sabs: Vec<AbsInfo> = [0u16, 1, 3, 4, 2, 5].iter().map(|&c| AbsInfo { code: c, value: if c == 2 || c == 5 { 0 } else { 128 }, min: 0, max: 255 }).collect();
    let mut s = EvdevPad::new(0x054C, &skeys, &sabs);
    s.feed(&evs(&[(KEY, 0x133, 1), (KEY, 0x138, 1), SYN]));
    t.check(s.state().pressed == 1 << PAD_Y, "pad evdev: Sony's BTN_NORTH is the top button; its digital L2 is the analog trigger's");
    s.feed(&evs(&[(KEY, 0x133, 0), (KEY, 0x134, 1), SYN]));
    t.check(s.state().pressed == 1 << PAD_X, "pad evdev: Sony's BTN_WEST is the left button");
    let a = s.state().axes;
    t.check(a.iter().all(|v| (v - 1.0 / 255.0).abs() < 1e-6), &format!("pad evdev: 0..255 sticks at 128: just right of the middle {a:?}"));
    s.feed(&evs(&[(ABS, 0, 0), (ABS, 1, 255), (ABS, 5, 255), SYN]));
    t.check(s.state().axes[0] == -1.0 && s.state().axes[1] == 1.0 && s.state().buttons[PAD_RT] == 1.0 && s.state().held(PAD_RT), "pad evdev: Sony's ends and R2");
    // Nintendo: by places as Sony
    let n = EvdevPad::new(0x057E, &skeys, &sabs);
    let mut n2 = n.clone();
    n2.feed(&evs(&[(KEY, 0x134, 1), SYN]));
    t.check(n2.state().pressed == 1 << PAD_X, "pad evdev: Nintendo's BTN_WEST is the left button");
    // a simple HID pad: no ABS_RX / ABS_RY, the right stick on ABS_Z / ABS_RZ, digital triggers
    let hkeys = bits(EVDEV_KEYS, &[0x130, 0x131, 0x133, 0x134, 0x136, 0x137, 0x138, 0x139]);
    let habs: Vec<AbsInfo> = [0u16, 1, 2, 5].iter().map(|&c| AbsInfo { code: c, value: 127, min: 0, max: 255 }).collect();
    let mut h = EvdevPad::new(0x0079, &hkeys, &habs);
    h.feed(&evs(&[(ABS, 2, 255), (ABS, 5, 0), (KEY, 0x138, 1), SYN]));
    t.check(h.state().axes[2] == 1.0 && h.state().axes[3] == -1.0 && h.state().pressed == 1 << PAD_LT && h.state().buttons[PAD_LT] == 1.0,
            "pad evdev: a simple pad: right stick on Z / RZ, BTN_TL2 the left trigger");
    // BTN_DPAD_* instead of a hat
    let dkeys = bits(EVDEV_KEYS, &[0x130, 0x220, 0x221, 0x222, 0x223]);
    let mut d = EvdevPad::new(0, &dkeys, &habs[..2]);
    d.feed(&evs(&[(KEY, 0x220, 1), (KEY, 0x223, 1), SYN]));
    t.check(d.state().pressed == (1 << PAD_UP | 1 << PAD_RIGHT), "pad evdev: BTN_DPAD_UP / RIGHT");
    // the state at the start comes from the axes' values
    let start: Vec<AbsInfo> = xpad_abs().into_iter().map(|mut a| {
        if a.code == 1 {
            a.value = -32768;
        }
        a
    }).collect();
    t.check(EvdevPad::new(0x045E, &keys, &start).state().axes[1] == -1.0, "pad evdev: the axes' values at the start");
}

// ------------------------------------------------------------------ the browser's record

fn web_rec(index: u32, flags: u32, pressed: u32, buttons: &[f32; 17], axes: &[f32; 4], id: &str) -> Vec<u8> {
    let mut r = Vec::new();
    r.extend_from_slice(&index.to_le_bytes());
    r.extend_from_slice(&flags.to_le_bytes());
    r.extend_from_slice(&pressed.to_le_bytes());
    for v in buttons.iter().chain(axes) {
        r.extend_from_slice(&v.to_le_bytes());
    }
    r.extend_from_slice(id.as_bytes());
    r
}

fn web(t: &mut T) {
    let mut b = [0f32; 17];
    b[PAD_RT] = 0.5;
    b[PAD_A] = 1.0;
    let r = web_rec(3, 3, 1 << PAD_A | 1 << PAD_RT, &b, &[0.25, -1.0, 0.0, 0.5], "Xbox 360 Controller (XInput STANDARD GAMEPAD)");
    let d = web_pad_decode(&r);
    t.check(d.as_ref().is_some_and(|(i, c, m, s, id)| *i == 3 && *c && *m && s.pressed == (1 << PAD_A | 1 << PAD_RT) && s.buttons[PAD_RT] == 0.5 && s.axes == [0.25, -1.0, 0.0, 0.5]
                                  && id.starts_with("Xbox 360")), "pad web: the record decoded");
    let r = web_rec(1, 0, 0, &[0.0; 17], &[0.0; 4], "");
    t.check(web_pad_decode(&r).is_some_and(|(i, c, m, _, _)| i == 1 && !c && !m), "pad web: a disconnect, not mapped");
    t.check(web_pad_decode(&web_rec(8, 1, 0, &[0.0; 17], &[0.0; 4], "x")).is_none(), "pad web: index 8: none");
    t.check(web_pad_decode(&r[..95]).is_none(), "pad web: a short record: none");
    let r = web_rec(0, 1, 0, &[f32::NAN; 17], &[9.0; 4], "x");
    t.check(web_pad_decode(&r).is_some_and(|(_, _, _, s, _)| s.buttons == [0.0; 17] && s.axes == [1.0; 4]), "pad web: values kept in range");
}

// ------------------------------------------------------------------ the backend of this platform

#[cfg(windows)]
fn xrec(packet: u32, buttons: u16, lt: u8, rt: u8, thumbs: [i16; 4]) -> [u8; 16] {
    let mut r = [0u8; 16];
    r[..4].copy_from_slice(&packet.to_le_bytes());
    r[4..6].copy_from_slice(&buttons.to_le_bytes());
    r[6] = lt;
    r[7] = rt;
    for (k, v) in thumbs.iter().enumerate() {
        r[8 + 2 * k..10 + 2 * k].copy_from_slice(&v.to_le_bytes());
    }
    r
}

#[cfg(windows)]
fn backend(t: &mut T) {
    pad_test_reset();
    evq_clear();
    pad_test_focus(Some(true));
    // the XInput DLL of this system (whatever is connected: not looked at)
    let real = pad_watch(true);
    t.check(real && pad_watching(), "pad win: XInput loaded (xinput1_4.dll or xinput9_1_0.dll)");
    pad_watch(false);
    evq_clear();
    // XINPUT_STATE records instead of the DLL's
    xinput_test_feed(1, Some(xrec(1, 0x1000, 0, 0, [0, 0, 0, 0])));
    xinput_test_feed(3, None);
    t.check(pad_watch(true), "pad win: watching with the test's records");
    let e = pad_events();
    t.check(e == [(1, PADEV_CONNECT), (1, PADEV_STATE)], &format!("pad win: slot 1 connects, A held {e:?}"));
    let v = pads();
    t.check(v.len() == 1 && v[0].index == 1 && v[0].name == "Xbox Controller (XInput)" && v[0].style == PadStyle::Xbox && v[0].held(PAD_A), "pad win: the pad as pads() has it");
    // the same packet number: not read again
    xinput_test_feed(1, Some(xrec(1, 0x2000, 0, 0, [0, 0, 0, 0])));
    pad_poll_test();
    t.check(pad_events().is_empty() && pad(1).held(PAD_A), "pad win: the same packet number: no change");
    xinput_test_feed(1, Some(xrec(2, 0x2000, 200, 0, [0, 32767, 0, 0])));
    pad_poll_test();
    let p = pad(1);
    t.check(pad_events() == [(1, PADEV_STATE)] && p.pressed == (1 << PAD_B | 1 << PAD_LT) && p.axes[PAD_LY] == -1.0 && p.went_up(PAD_A), "pad win: a new packet: B, LT, the stick up");
    // the app loses the focus: nothing held; back
    pads();
    pad_test_focus(Some(false));
    pad_poll_test();
    t.check(pad_events() == [(1, PADEV_STATE)] && pad(1).pressed == 0 && pad(1).axes == [0.0; 4] && pad(1).connected, "pad win: not focused: neutral, still connected");
    pads();
    pad_test_focus(Some(true));
    pad_poll_test();
    t.check(pad(1).pressed == (1 << PAD_B | 1 << PAD_LT), "pad win: focused again: the state");
    // a second pad, then the first unplugged
    xinput_test_feed(3, Some(xrec(9, 0x0400, 0, 0, [0, 0, 0, 0])));
    pad_poll_test();
    t.check(pad(3).connected && pad(3).held(PAD_HOME), "pad win: slot 3 with the guide button (XInputGetStateEx)");
    pad_events();
    xinput_test_feed(1, None);
    pad_poll_test();
    t.check(pad_events() == [(1, PADEV_DISCONNECT)] && !pad(1).connected && pads().len() == 1, "pad win: slot 1 unplugged");
    pad_watch(false);
    t.check(pad_events() == [(3, PADEV_DISCONNECT)] && pads().is_empty(), "pad win: stopped: the rest goes");
    xinput_test_end();
    pad_test_focus(None);
}

#[cfg(target_os = "linux")]
fn backend(t: &mut T) {
    pad_test_reset();
    evq_clear();
    pad_test_focus(Some(true));
    // the real backend: inotify on /dev/input, the nodes there (whatever is connected: not looked at)
    t.check(pad_watch(true) && pad_watching(), "pad linux: watching /dev/input");
    let before = pads().len();
    pad_events();
    let keys = bits(EVDEV_KEYS, &XPAD_KEYS);
    let Some((slot, node)) = evdev_test_attach("Microsoft X-Box 360 pad", 0x045E, &keys, &xpad_abs()) else {
        t.check(false, "pad linux: a pipe as a pad's node");
        pad_watch(false);
        return;
    };
    t.check(pad_events() == [(slot as u32, PADEV_CONNECT)] && pad(slot).style == PadStyle::Xbox && pads().len() == before + 1, "pad linux: the pipe's pad connects");
    t.check(node.write(&evs(&[(KEY, 0x130, 1), (ABS, 2, 255), (ABS, 0, 32767), SYN])), "pad linux: records written");
    pad_poll_test();
    let p = pad(slot);
    t.check(pad_events() == [(slot as u32, PADEV_STATE)] && p.pressed == (1 << PAD_A | 1 << PAD_LT) && p.axes[PAD_LX] == 1.0, "pad linux: read when polled: A, LT, the stick right");
    // half a report: nothing until its SYN_REPORT
    pads();
    node.write(&evs(&[(KEY, 0x130, 0)]));
    pad_poll_test();
    t.check(pad(slot).held(PAD_A), "pad linux: no SYN_REPORT yet: A still held");
    node.write(&evs(&[SYN]));
    pad_poll_test();
    t.check(!pad(slot).held(PAD_A) && pad(slot).went_up(PAD_A), "pad linux: the report: A up");
    // more records than one read takes (64): all of them
    let mut many = Vec::new();
    for k in 0..100 {
        many.extend_from_slice(&evs(&[(ABS, 1, k * 300), SYN]));
    }
    node.write(&many);
    pad_poll_test();
    t.check((pad(slot).axes[PAD_LY] - 29700.0 / 32767.0).abs() < 1e-6, &format!("pad linux: 200 records in one round: the last value {}", pad(slot).axes[PAD_LY]));
    // not focused: neutral
    pad_test_focus(Some(false));
    pad_poll_test();
    t.check(pad(slot).pressed == 0 && pad(slot).axes == [0.0; 4] && pad(slot).connected, "pad linux: not focused: neutral, still connected");
    pad_test_focus(Some(true));
    pad_poll_test();
    t.check(pad(slot).buttons[PAD_LT] == 1.0, "pad linux: focused again");
    pad_events();
    // the writer goes: unplugged
    drop(node);
    pad_poll_test();
    t.check(pad_events() == [(slot as u32, PADEV_DISCONNECT)] && !pad(slot).connected, "pad linux: the node ends: the pad went");
    pad_watch(false);
    evq_clear();
    t.check(pads().is_empty() && !pad_watching(), "pad linux: stopped");
    pad_test_focus(None);
}

#[cfg(target_os = "macos")]
fn backend(t: &mut T) {
    pad_test_reset();
    evq_clear();
    pad_test_focus(Some(true));
    let mut want = PadState::NEUTRAL;
    for b in [PAD_A, PAD_Y, PAD_RB, PAD_START, PAD_RIGHT] {
        want.buttons[b] = 1.0;
        want.pressed |= 1 << b;
    }
    want.buttons[PAD_LT] = 0.75;
    want.pressed |= 1 << PAD_LT;
    want.buttons[PAD_RT] = 0.1; // below PAD_TRIGGER_PRESS: not pressed
    want.axes = [0.5, -0.25, -1.0, 0.75];
    let Some((got, slot)) = gc_test_pad(&want) else {
        t.check(false, "pad mac: controllerWithExtendedGamepad (macOS 10.15)");
        return;
    };
    let diff: Vec<String> = (0..PAD_BUTTONS).filter(|&b| (got.buttons[b] - want.buttons[b]).abs() > 1e-6).map(|b| format!("{} {}", pad_button_name(b), got.buttons[b])).collect();
    t.check(diff.is_empty(), &format!("pad mac: the buttons' values read back {diff:?}"));
    t.check(got.pressed == want.pressed, &format!("pad mac: pressed {:#x} (want {:#x})", got.pressed, want.pressed));
    t.check(got.axes.iter().zip(want.axes).all(|(a, b)| (a - b).abs() < 1e-6), &format!("pad mac: the sticks (up positive in GameController) {:?}", got.axes));
    let e = pad_events();
    t.check(e.first() == Some(&(slot as u32, PADEV_CONNECT)), &format!("pad mac: it connects through the backend {e:?}"));
    let p = pad(slot);
    t.check(p.connected && p.pressed == want.pressed && !p.name.is_empty(), &format!("pad mac: the pad: {:?} {:#x}", p.name, p.pressed));
    gc_test_disconnect(slot);
    t.check(pad_events() == [(slot as u32, PADEV_DISCONNECT)] && !pad(slot).connected, "pad mac: disconnected through the backend");
    // the real framework: notifications observed, the controllers there (not looked at)
    t.check(pad_watch(true) && pad_watching(), "pad mac: watching (GameController)");
    pad_watch(false);
    evq_clear();
    pad_test_focus(None);
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn backend(t: &mut T) {
    // the browser watches through its page (tools/tests/web_pad.mjs); Android and iOS: none yet
    let want = cfg!(target_arch = "wasm32");
    t.check(pad_watch(true) == want, "pad: pad_watch here");
    pad_watch(false);
    t.check(!pad_poll_test(), "pad: no native backend");
    evq_clear();
}

/// --test pad-policy: a manifest without `gamepad`: pad_watch refuses and opens nothing, the refusal is recorded
/// (Android and iOS have no pads: false before the manifest is asked)
pub fn t_pad_policy(t: &mut T) {
    t.check(plat::policy::install("fs.user\n").is_ok(), "pad policy: a manifest installed");
    t.check(!pad_watch(true) && !pad_watching(), "pad policy: no gamepad: pad_watch refuses, nothing watches");
    t.check(!plat::policy::gamepad_granted(), "pad policy: not granted");
    let native = !cfg!(any(target_os = "android", target_os = "ios"));
    let d = plat::policy::denials();
    t.check(d.iter().any(|x| x.kind == "gamepad" && x.target.is_empty()) == native, "pad policy: the refusal recorded (PERMDENY gamepad)");
    t.check(plat::policy::check("gamepad\nask gamepad \"Mit dem Pad spielen\"\n").is_ok(), "pad policy: the right parses (also asked)");
    t.check(plat::policy::check("gamepad x\n").is_err(), "pad policy: gamepad takes no arguments");
}

/// --test pad-ask: `ask gamepad`: refused while nobody answers (a console run asks nobody), watching after a yes
pub fn t_pad_ask(t: &mut T) {
    let Ok(key) = plat::policy::install("ask gamepad \"Mit dem Pad spielen\"\n") else {
        t.check(false, "pad ask: the manifest installs");
        return;
    };
    t.check(!pad_watch(true) && !pad_watching(), "pad ask: nobody answers: refused");
    t.check(!plat::policy::gamepad_granted(), "pad ask: not granted before the answer");
    key.set_answer(Some(plat::policy::Answer::Once));
    // (the platforms with pads: the backend starts, as in --test pad)
    let want = cfg!(any(windows, target_os = "linux", target_os = "macos", target_arch = "wasm32"));
    t.check(pad_watch(true) == want && pad_watching() == want, "pad ask: allowed: watching starts");
    t.check(plat::policy::gamepad_granted(), "pad ask: granted after the answer");
    pad_watch(false);
    let d = plat::policy::decisions();
    t.check(d.iter().any(|x| x.rule == "gamepad" && x.reason == "Mit dem Pad spielen"), "pad ask: the decision and its reason recorded");
    evq_clear();
}
