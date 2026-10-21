// t_pen -- (plat-win) plattest --test pen (PN1): the platform pen paths without a pen
//                                     [apps/plattest/src/pen.s; docs/modules/PEN_INPUT.md]
// Nothing is injected into the system:
//   Windows  a hidden window of our own gets WM_POINTER* through SendMessageW; the pointer API
//            (plat::pen_api_set) is replaced by fakes that describe a pen, a touch point and an
//            unknown pointer -> E_MOVE (history, oldest first) / E_DOWN (barrel = right) / E_UP /
//            E_LEAVE with pen data, eraser, tilt clamp, history clamp, pen input off,
//            pen_devices
//   Linux    (main agent) Wayland: the tablet tool handlers with synthetic events (scale 1 /
//            2) -> E_* with pen data, barrel buttons, eraser tool; X11: a synthetic
//            XIQueryDevice reply (pen + eraser slave devices), XI2 GenericEvents through
//            x11_event before core events of the same time, a truncated event, a mouse event
//            in between
// Every check prints PASS / FAIL (T::check).

#[cfg(not(windows))]
pub use lx::t_pen;

#[cfg(not(windows))]
mod lx {
    use alloc::format;
    use alloc::vec::Vec;

    use plat::*;

    use crate::{T, out};

    const TOOL: &str = "zwp_tablet_tool_v2";
    const SEAT: &str = "zwp_tablet_seat_v2";

    /// the next event must match (type, btn, x, y, data; x = i32::MAX: position not compared)
    fn pop_expect(t: &mut T, typ: u8, btn: u8, x: i32, y: i32, data: u32, name: &str) {
        let e = evq_pop();
        let ok = e.is_some_and(|e| e.typ == typ && e.btn == btn && e.data == data && (x == i32::MAX || (e.x == x && e.y == y)));
        t.check(ok, name);
        if !ok {
            let e = e.unwrap_or_default();
            out(&format!("     got type {} btn {} x {} y {} data {:x}\n", e.typ, e.btn, e.x, e.y, e.data));
        }
    }

    /// the ring must be empty
    fn no_events(t: &mut T, name: &str) {
        t.check(evq_count() == 0, name);
        evq_clear();
    }

    /// a Wayland event handler with (32-bit, zero-extended) args and the tool slot as udata
    /// [tcall / TC]
    fn tc(iface: &str, ev: &str, slot: u64, a: &[i32]) {
        let v: Vec<u64> = a.iter().map(|&x| x as u32 as u64).collect();
        wl_test_event_u(iface, ev, &v, slot);
    }

    fn tool(ev: &str, slot: u64, a: &[i32]) {
        tc(TOOL, ev, slot, a);
    }

    fn sh(v: &mut Vec<u8>, xs: &[i32]) {
        for &x in xs {
            v.extend_from_slice(&(x as u16).to_le_bytes());
        }
    }

    fn lg(v: &mut Vec<u8>, xs: &[i64]) {
        for &x in xs {
            v.extend_from_slice(&(x as u32).to_le_bytes());
        }
    }

    /// the reply length field (4-byte units after the first 32 bytes)
    fn fix_len(mut v: Vec<u8>) -> Vec<u8> {
        let n = ((v.len() - 32) / 4) as u32;
        v[4..8].copy_from_slice(&n.to_le_bytes());
        v
    }

    /// a valuator class of device `src`: number, label, min, max
    fn valuator(v: &mut Vec<u8>, src: i32, num: i32, label: i64, min: i64, max: i64) {
        sh(v, &[2, 11, src, num]);
        lg(v, &[label, min, 0, max, 0, 0, 0, 0]);
        v.extend_from_slice(&[0; 4]);
    }

    /// XIQueryDevice reply: a master pointer, a pen (valuators 2 pressure 0..2047, 3 / 4 tilt),
    /// an eraser (valuator 2 pressure 0..1023), a mouse (slave, no pressure)  [xi_reply]
    fn xi_reply() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&[1, 0]);
        sh(&mut v, &[1]);
        lg(&mut v, &[0]);
        sh(&mut v, &[4]);
        v.extend_from_slice(&[0; 22]);
        sh(&mut v, &[2, 1, 3, 0, 20]);
        v.extend_from_slice(&[1, 0]);
        v.extend_from_slice(b"Virtual core pointer");
        sh(&mut v, &[11, 3, 2, 3, 16]);
        v.extend_from_slice(&[1, 0]);
        v.extend_from_slice(b"Wacom Pen stylus");
        valuator(&mut v, 11, 2, 500, 0, 2047);
        valuator(&mut v, 11, 3, 501, -64, 63);
        valuator(&mut v, 11, 4, 502, -64, 63);
        sh(&mut v, &[12, 3, 2, 1, 17]);
        v.extend_from_slice(&[1, 0]);
        v.extend_from_slice(b"Wacom Pen ERASER\0\0\0\0");
        valuator(&mut v, 12, 2, 500, 0, 1023);
        sh(&mut v, &[13, 3, 2, 1, 8]);
        v.extend_from_slice(&[1, 0]);
        v.extend_from_slice(b"USBmouse");
        valuator(&mut v, 13, 0, 0, 0, 1000);
        fix_len(v)
    }

    /// an XI2 device event (GenericEvent of extension 131): evtype, device, time, detail,
    /// buttons_len 0, valuators_len `vlen`, then the mask word and the FP3232 values
    fn xi_event(evtype: i32, dev: i32, time: i64, detail: i64, vlen: i32, mask: i64, vals: &[i64]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&[35, 131]);
        sh(&mut v, &[0]);
        lg(&mut v, &[0]);
        sh(&mut v, &[evtype, dev]);
        lg(&mut v, &[time, detail, 0, 0, 0, 0, 0, 0, 0]);
        sh(&mut v, &[0, vlen, dev, 0]);
        lg(&mut v, &[0, 0, 0, 0, 0]);
        v.extend_from_slice(&[0; 4]);
        lg(&mut v, &[mask]);
        lg(&mut v, vals);
        fix_len(v)
    }

    /// a synthetic core pointer event via x11_event  [core_ev]
    fn core_ev(typ: u8, detail: u8, x: i32, y: i32, time: u32) {
        let mut p = [0u8; 32];
        p[0] = typ;
        p[1] = detail;
        p[4..8].copy_from_slice(&time.to_le_bytes());
        p[20..22].copy_from_slice(&(x as u16).to_le_bytes());
        p[24..26].copy_from_slice(&(x as u16).to_le_bytes());
        p[22..24].copy_from_slice(&(y as u16).to_le_bytes());
        p[26..28].copy_from_slice(&(y as u16).to_le_bytes());
        x11t::event(&p);
    }

    /// Linux: Wayland tablet handlers, then XInput 2
    pub fn t_pen(t: &mut T) {
        evq_clear();
        wl_test_scale(120); // scale 1 (no window)
        // ---- Wayland: a pen with pressure and tilt (slot 0), an eraser (slot 1)
        tc(SEAT, "tool_added", 0, &[0xFF000100u32 as i32]);
        tc(SEAT, "tool_added", 0, &[0xFF000101u32 as i32]);
        tool("type", 0, &[0x140]);
        tool("capability", 0, &[2]);
        tool("capability", 0, &[1]);
        tool("type", 1, &[0x141]);
        tool("capability", 1, &[2]);
        tool("proximity_in", 0, &[5, 0, 0]);
        tool("motion", 0, &[10 * 256, 20 * 256]);
        tool("pressure", 0, &[32768]);
        tool("tilt", 0, &[30 * 256, -10 * 256]);
        tool("frame", 0, &[1]);
        pop_expect(t, E_MOVE, 0, 10, 20, evq_pen_data(32768, PTR_PEN, 30, -10),
                   "wl: proximity + motion -> E_MOVE, pressure 32768, tilt 30 / -10");
        tool("down", 0, &[6]);
        tool("pressure", 0, &[70000]);
        tool("frame", 0, &[2]);
        pop_expect(t, E_DOWN, BTN_LEFT, 10, 20, evq_pen_data(65535, PTR_PEN, 30, -10),
                   "wl: down -> E_DOWN left (pressure 70000 clamped)");
        tool("motion", 0, &[15 * 256, 20 * 256]);
        tool("up", 0, &[]);
        tool("frame", 0, &[3]);
        pop_expect(t, E_MOVE, 0, 15, 20, evq_pen_data(65535, PTR_PEN, 30, -10), "wl: motion + up in one frame: E_MOVE first");
        pop_expect(t, E_UP, BTN_LEFT, 15, 20, evq_pen_data(65535, PTR_PEN, 30, -10), "wl: ... then E_UP");
        tool("button", 0, &[7, 0x14B, 1]);
        tool("down", 0, &[8]);
        tool("frame", 0, &[4]);
        pop_expect(t, E_DOWN, BTN_RIGHT, 15, 20, evq_pen_data(65535, PTR_PEN, 30, -10),
                   "wl: BTN_STYLUS held -> E_DOWN right");
        tool("button", 0, &[9, 0x14B, 0]);
        tool("button", 0, &[9, 0x110, 1]);
        tool("proximity_out", 0, &[]);
        tool("frame", 0, &[5]);
        pop_expect(t, E_UP, BTN_RIGHT, 15, 20, 0, "wl: proximity_out while down -> E_UP right");
        pop_expect(t, E_LEAVE, 0, 0, 0, 0, "wl: ... and E_LEAVE");
        tool("motion", 0, &[99 * 256, 99 * 256]);
        tool("frame", 0, &[6]);
        no_events(t, "wl: motion out of proximity: nothing");
        // the eraser, scale 2
        wl_test_scale(240);
        tool("proximity_in", 1, &[10, 0, 0]);
        tool("motion", 1, &[3 * 256 + 128, 4 * 256]);
        tool("pressure", 1, &[0]);
        tool("frame", 1, &[7]);
        pop_expect(t, E_MOVE, 0, 7, 8, evq_pen_data(0, PTR_ERASER, 0, 0), "wl: eraser tool, scale 2 -> E_MOVE 7,8 PTR_ERASER");
        tool("frame", 5, &[8]);
        tool("motion", 9, &[0, 0]);
        no_events(t, "wl: unknown tool slots are ignored");
        // pen input off: plain mouse events
        pen_enable(false);
        tool("down", 1, &[11]);
        tool("frame", 1, &[9]);
        pop_expect(t, E_DOWN, BTN_LEFT, 7, 8, 0, "wl: pen input off -> E_DOWN without pen data");
        pen_enable(true);
        tool("up", 1, &[]);
        tool("frame", 1, &[10]);
        evq_clear();

        // ---- X11: XInput 2 devices from a synthetic XIQueryDevice reply
        x11t::xi_setup(131, [500, 501, 502]);
        let n = x11t::xi_parse_devices(&xi_reply());
        t.check(n == 2, "x11: XIQueryDevice -> 2 pen devices (master, mouse skipped)");
        t.check(x11t::pen_devices() == 2, "x11: x11_pen_devices");
        // a pen sample (XI_Motion, device 11), then the core motion of the same time
        x11t::event_bytes(&xi_event(6, 11, 1000, 0, 1, 0x1C, &[1023, 0x80000000, -20, 0, 15, 0]));
        core_ev(6, 0, 40, 50, 1000); // MotionNotify
        pop_expect(t, E_MOVE, 0, 40, 50, evq_pen_data(32751, PTR_PEN, -20, 15),
                   "x11: XI_Motion + core motion -> E_MOVE with pressure / tilt");
        core_ev(6, 0, 41, 50, 1001);
        pop_expect(t, E_MOVE, 0, 41, 50, 0, "x11: a core motion of another time is a mouse");
        // the eraser device presses (pressure 512 of 0..1023)
        x11t::event_bytes(&xi_event(4, 12, 1002, 1, 1, 0x4, &[512, 0]));
        core_ev(4, 1, 42, 51, 1002); // ButtonPress 1
        pop_expect(t, E_DOWN, BTN_LEFT, 42, 51, evq_pen_data(32799, PTR_ERASER, 0, 0),
                   "x11: eraser device press -> E_DOWN, PTR_ERASER");
        core_ev(5, 1, 42, 51, 1003);
        pop_expect(t, E_UP, BTN_LEFT, 42, 51, 0, "x11: release");
        // a truncated event (valuators beyond its length) changes nothing
        x11t::event_bytes(&xi_event(6, 11, 1004, 0, 200, 0x4, &[7, 0]));
        core_ev(6, 0, 43, 52, 1004);
        pop_expect(t, E_MOVE, 0, 43, 52, 0, "x11: truncated XI event ignored");
        x11t::xi_setup(0, [500, 501, 502]);
        evq_clear();
    }
}

#[cfg(windows)]
pub use win::t_pen;

#[cfg(windows)]
mod win {
    use alloc::format;
    use core::sync::atomic::{AtomicI32, AtomicU32, Ordering::SeqCst};

    use plat::*;

    use crate::{T, out};

    const WM_POINTERUPDATE: u32 = 0x0245;
    const WM_POINTERDOWN: u32 = 0x0246;
    const WM_POINTERUP: u32 = 0x0247;
    const WM_POINTERLEAVE: u32 = 0x024A;
    const WM_POINTERCAPTURECHANGED: u32 = 0x024C;
    const FK_N: usize = 20; // fake samples

    // fake sample: client x, y, pressure (0..1024), pen flags, pen mask, tilt x, tilt y,
    // pointer flags
    type Sample = [i32; 8];
    static SAMP_A: [Sample; 3] = [[10, 20, 0, 0, 1, 0, 0, 0], [20, 30, 512, 0, 1, 0, 0, 0], [30, 40, 1024, 0, 1, 0, 0, 0]];
    static SAMP_BARREL: [Sample; 1] = [[30, 40, 1024, 1, 1, 0, 0, 0x20]];
    static SAMP_ERASER: [Sample; 1] = [[50, 5, 256, 2, 13, -70, 20, 0]];
    static SAMP_NOP: [Sample; 1] = [[1, 2, 700, 0, 0, 0, 0, 0]];
    static SAMP_MANY: [Sample; FK_N] = [[5, 5, 600, 0, 1, 0, 0, 0]; FK_N];

    // the samples the fakes report (set: 0 a, 1 barrel, 2 eraser, 3 nop, 4 many), the client
    // origin on the screen, a history count to report instead of the real one
    static FK_SET: AtomicU32 = AtomicU32::new(0);
    static FK_REPORT: AtomicU32 = AtomicU32::new(0);
    static ORG_X: AtomicI32 = AtomicI32::new(0);
    static ORG_Y: AtomicI32 = AtomicI32::new(0);

    fn samples() -> &'static [Sample] {
        match FK_SET.load(SeqCst) {
            0 => &SAMP_A,
            1 => &SAMP_BARREL,
            2 => &SAMP_ERASER,
            3 => &SAMP_NOP,
            _ => &SAMP_MANY,
        }
    }

    /// a POINTER_PEN_INFO from a fake sample (screen position)
    fn fill(s: &Sample) -> PenInfo {
        PenInfo {
            flags: s[7] as u32,
            x: s[0] + ORG_X.load(SeqCst),
            y: s[1] + ORG_Y.load(SeqCst),
            pen_flags: s[3] as u32,
            pen_mask: s[4] as u32,
            pressure: s[2] as u32,
            tilt_x: s[5],
            tilt_y: s[6],
        }
    }

    /// GetPointerType: 7 pen, 8 touch, else failure
    fn fake_type(id: u32) -> Option<u32> {
        match id {
            7 => Some(3),
            8 => Some(2),
            _ => None,
        }
    }

    /// GetPointerPenInfo: the newest sample
    fn fake_info(_id: u32) -> Option<PenInfo> {
        samples().last().map(fill)
    }

    /// GetPointerPenInfoHistory: newest first, at most *count records; *count = the samples
    /// available (or FK_REPORT)
    fn fake_hist(_id: u32, count: &mut u32, out: &mut [PenInfo]) -> bool {
        let s = samples();
        let room = (*count as usize).min(s.len()).min(out.len());
        for (k, o) in out.iter_mut().enumerate().take(room) {
            *o = fill(&s[s.len() - 1 - k]);
        }
        *count = match FK_REPORT.load(SeqCst) {
            0 => s.len() as u32,
            r => r,
        };
        true
    }

    /// GetPointerDevices: an external pen and a touch screen
    fn fake_devs(count: &mut u32, types: Option<&mut [u32]>) -> bool {
        *count = 2;
        if let Some(t) = types {
            if !t.is_empty() {
                t[0] = 2; // POINTER_DEVICE_TYPE_EXTERNAL_PEN
            }
            if t.len() > 1 {
                t[1] = 3; // touch
            }
        }
        true
    }

    /// the next event must match (type, btn, x, y, data; x = i32::MAX: position not compared)
    fn pop_expect(t: &mut T, typ: u8, btn: u8, x: i32, y: i32, data: u32, name: &str) {
        let e = evq_pop();
        let ok = e.is_some_and(|e| e.typ == typ && e.btn == btn && e.data == data && (x == i32::MAX || (e.x == x && e.y == y)));
        t.check(ok, name);
        if !ok {
            let e = e.unwrap_or_default();
            out(&format!("     got type {} btn {} x {} y {} data {:x}\n", e.typ, e.btn, e.x, e.y, e.data));
        }
    }

    /// the ring must be empty
    fn no_events(t: &mut T, name: &str) {
        t.check(evq_count() == 0, name);
        evq_clear();
    }

    /// SendMessageW to the test window (wParam = id | INRANGE << 16)
    fn send(w: i32, msg: u32, id: usize) {
        win_test_send(w, msg, id | 0x20000, 0);
    }

    fn set(k: u32) {
        FK_SET.store(k, SeqCst);
    }

    /// Windows: WM_POINTER* to our own hidden window with a fake pointer API
    pub fn t_pen(t: &mut T) {
        evq_clear();
        let w = win_create("pen test", 200, 100, 0, PW_NOOWNER, 100, 100);
        t.check(w >= 0, "pen: hidden test window");
        if w < 0 {
            return;
        }
        // client (0, 0) on the screen: the fakes report screen positions
        let o = win_get_rect(w).unwrap_or_default();
        ORG_X.store(o[0], SeqCst);
        ORG_Y.store(o[1], SeqCst);
        pen_api_set(Some(PenApi { ptype: Some(fake_type), info: Some(fake_info), hist: Some(fake_hist), devs: Some(fake_devs) }));
        evq_clear();

        // ---- three coalesced samples: oldest first, pressure 0 / 512 / 1024
        set(0);
        send(w, WM_POINTERUPDATE, 7);
        pop_expect(t, E_MOVE, 0, 10, 20, evq_pen_data(0, PTR_PEN, 0, 0), "pen: history sample 1 (oldest, pressure 0 -> 1)");
        pop_expect(t, E_MOVE, 0, 20, 30, evq_pen_data(32767, PTR_PEN, 0, 0), "pen: history sample 2 (pressure 512 -> 32767)");
        pop_expect(t, E_MOVE, 0, 30, 40, evq_pen_data(65535, PTR_PEN, 0, 0), "pen: history sample 3 (newest, full pressure)");
        no_events(t, "pen: three samples, nothing else");
        // ---- the barrel button held at the contact -> right button
        set(1);
        send(w, WM_POINTERDOWN, 7);
        pop_expect(t, E_DOWN, BTN_RIGHT, 30, 40, evq_pen_data(65535, PTR_PEN, 0, 0),
                   "pen: contact with the barrel button -> E_DOWN right");
        send(w, WM_POINTERDOWN, 7);
        no_events(t, "pen: a second down while down is ignored");
        send(w, WM_POINTERUP, 7);
        pop_expect(t, E_UP, BTN_RIGHT, 30, 40, evq_pen_data(65535, PTR_PEN, 0, 0), "pen: lift -> E_UP right");
        // ---- the eraser end, tilt (clamped), capture lost
        set(2);
        send(w, WM_POINTERDOWN, 7);
        pop_expect(t, E_DOWN, BTN_LEFT, 50, 5, evq_pen_data(16383, PTR_ERASER, -63, 20),
                   "pen: eraser end -> E_DOWN left, PTR_ERASER, tilt -70 clamped");
        send(w, WM_POINTERCAPTURECHANGED, 7);
        pop_expect(t, E_UP, BTN_LEFT, -1, -1, 0, "pen: capture lost -> E_UP at the last position");
        send(w, WM_POINTERUP, 7);
        no_events(t, "pen: the lift after a lost capture adds nothing");
        // ---- a pen without pressure axis
        set(3);
        send(w, WM_POINTERUPDATE, 7);
        pop_expect(t, E_MOVE, 0, 1, 2, evq_pen_data(-1, PTR_PEN, 0, 0), "pen: no pressure axis -> pressure 0 (none)");
        // ---- an untrusted history count: at most 16 samples
        set(4);
        FK_REPORT.store(1000, SeqCst);
        send(w, WM_POINTERUPDATE, 7);
        FK_REPORT.store(0, SeqCst);
        t.check(evq_count() == 16, "pen: history count 1000 -> 16 samples (PEN_HIST)");
        evq_clear();
        // ---- touch / unknown pointers / pen input off: DefWindowProc, no events
        send(w, WM_POINTERUPDATE, 8);
        no_events(t, "pen: touch pointer -> DefWindowProc (no events)");
        send(w, WM_POINTERDOWN, 9);
        no_events(t, "pen: GetPointerType fails -> DefWindowProc");
        pen_enable(false);
        send(w, WM_POINTERDOWN, 7);
        no_events(t, "pen: pen input off -> DefWindowProc (mouse emulation)");
        pen_enable(true);
        send(w, WM_POINTERLEAVE, 7);
        pop_expect(t, E_LEAVE, 0, 0, 0, 0, "pen: out of range -> E_LEAVE");
        // ---- devices: one external pen, one touch screen
        t.check(pen_devices() == 1, "pen: plat_pen_devices counts pens only");
        // back to the real API (nothing else of this process uses it)
        pen_api_set(None);
        win_destroy(w);
    }
}
