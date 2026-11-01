// t_touch -- plattest --test touch: the platform's fingers without a touch screen
// Nothing is injected into the system:
//   Windows  a hidden window of our own gets WM_POINTER* / WM_NCPOINTER* through SendMessageW; the pointer
//            API (plat::pen_api_set, touch_api_set) is replaced by fakes that describe fingers: the events
//            of plat::finger, ids and history, cancel, no double events, the window that follows a finger
//   Linux    Wayland: the wl_touch handlers with synthetic events (scale 1 / 2, two fingers, cancel);
//            X11: XI 2.2 GenericEvents through x11_event (two fingers, an event without a window or too
//            short for the fields)
// A real window with real contacts (InjectTouchInput): tools/touchcheck.py.
// Every check prints PASS / FAIL (T::check).

#[cfg(windows)]
pub use win::t_touch;
#[cfg(not(windows))]
pub use lx::t_touch;

/// the event's fields as a finger: phase type, id, x, y, cancelled
#[allow(dead_code)]
fn finger_of(e: &plat::Event) -> (u8, u8, i32, i32, bool) {
    (e.typ, e.btn, e.x, e.y, e.data & plat::PD_CANCEL != 0)
}

fn pop_finger(t: &mut crate::T, typ: u8, id: u8, x: i32, y: i32, cancel: bool, name: &str) {
    let e = plat::evq_pop();
    let ok = e.is_some_and(|e| (e.data >> plat::PD_TYPE_SHIFT) & 3 == plat::PTR_TOUCH && finger_of(&e) == (typ, id, x, y, cancel));
    t.check(ok, name);
    if !ok {
        let e = e.unwrap_or_default();
        crate::out(&alloc::format!("     got type {} id {} x {} y {} data {:x}\n", e.typ, e.btn, e.x, e.y, e.data));
    }
}

fn no_events(t: &mut crate::T, name: &str) {
    t.check(plat::evq_count() == 0, name);
    plat::evq_clear();
}

#[cfg(windows)]
mod win {
    use core::sync::atomic::{AtomicI32, AtomicU32, Ordering::SeqCst};

    use plat::*;

    use super::{no_events, pop_finger};
    use crate::T;

    const WM_NCPOINTERDOWN: u32 = 0x0242;
    const WM_NCPOINTERUPDATE: u32 = 0x0241;
    const WM_NCPOINTERUP: u32 = 0x0243;
    const WM_POINTERUPDATE: u32 = 0x0245;
    const WM_POINTERDOWN: u32 = 0x0246;
    const WM_POINTERUP: u32 = 0x0247;
    const WM_POINTERCAPTURECHANGED: u32 = 0x024C;
    const WM_MOUSEMOVE: u32 = 0x0200;
    const INCONTACT: u32 = 0x4;
    const CANCELED: u32 = 0x8000;
    const HTCAPTION: usize = 2;

    // what the fakes report for pointer ids 20.. (client x, y; flags)
    static X: [AtomicI32; 16] = [const { AtomicI32::new(0) }; 16];
    static Y: [AtomicI32; 16] = [const { AtomicI32::new(0) }; 16];
    static FLAGS: AtomicU32 = AtomicU32::new(INCONTACT);
    static HIST: AtomicU32 = AtomicU32::new(0);
    static REPORT: AtomicU32 = AtomicU32::new(0);
    static TIME: AtomicU32 = AtomicU32::new(0);
    static ORG: [AtomicI32; 2] = [const { AtomicI32::new(0) }; 2];

    fn is_finger(id: u32) -> bool {
        (20..36).contains(&id)
    }

    fn fake_type(id: u32) -> Option<u32> {
        match id {
            7 => Some(3),
            i if is_finger(i) => Some(2),
            _ => None,
        }
    }

    fn sample(id: u32, back: i32) -> TouchInfo {
        let k = (id - 20) as usize;
        TouchInfo {
            flags: FLAGS.load(SeqCst),
            x: X[k].load(SeqCst) + ORG[0].load(SeqCst) - back,
            y: Y[k].load(SeqCst) + ORG[1].load(SeqCst),
            time: TIME.load(SeqCst),
        }
    }

    fn fake_info(id: u32) -> Option<TouchInfo> {
        is_finger(id).then(|| sample(id, 0))
    }

    /// newest first: x back by 1 per sample
    fn fake_hist(id: u32, count: &mut u32, out: &mut [TouchInfo]) -> bool {
        let n = HIST.load(SeqCst);
        if n == 0 || !is_finger(id) {
            return false;
        }
        let room = (*count).min(n) as usize;
        for (k, o) in out.iter_mut().enumerate().take(room) {
            *o = sample(id, k as i32);
        }
        *count = match REPORT.load(SeqCst) {
            0 => n,
            r => r,
        };
        true
    }

    fn fake_now() -> u32 {
        1000
    }

    fn at(id: u32, x: i32, y: i32) {
        X[(id - 20) as usize].store(x, SeqCst);
        Y[(id - 20) as usize].store(y, SeqCst);
    }

    fn send(w: i32, msg: u32, id: usize) {
        win_test_send(w, msg, id, 0);
    }

    fn nc(w: i32, msg: u32, id: usize, ht: usize) -> isize {
        win_test_send(w, msg, id | ht << 16, 0)
    }

    /// Windows: WM_POINTER* to our own hidden windows with a fake pointer API
    pub fn t_touch(t: &mut T) {
        evq_clear();
        let w = win_create("touch test", 300, 200, 0, PW_NOOWNER, 100, 100);
        t.check(w >= 0, "touch: hidden test window");
        if w < 0 {
            return;
        }
        let o = win_get_rect(w).unwrap_or_default();
        ORG[0].store(o[0], SeqCst);
        ORG[1].store(o[1], SeqCst);
        pen_api_set(Some(PenApi { ptype: Some(fake_type), info: None, hist: None, devs: None }));
        touch_api_set(Some(TouchApi { info: Some(fake_info), hist: Some(fake_hist), now: Some(fake_now) }));
        evq_clear();

        // ---- one finger: down, three coalesced samples oldest first, up
        at(20, 50, 60);
        send(w, WM_POINTERDOWN, 20);
        pop_finger(t, E_DOWN, 0, 50, 60, false, "touch: down -> a finger, id 0");
        no_events(t, "touch: down gives one event, no mouse");
        HIST.store(3, SeqCst);
        at(20, 70, 60);
        send(w, WM_POINTERUPDATE, 20);
        pop_finger(t, E_MOVE, 0, 68, 60, false, "touch: history sample 1 (oldest)");
        pop_finger(t, E_MOVE, 0, 69, 60, false, "touch: history sample 2");
        pop_finger(t, E_MOVE, 0, 70, 60, false, "touch: history sample 3 (newest)");
        no_events(t, "touch: three samples, nothing else");
        // an untrusted count: at most the buffer
        REPORT.store(5000, SeqCst);
        HIST.store(40, SeqCst);
        send(w, WM_POINTERUPDATE, 20);
        t.check(evq_count() == 16, "touch: history count clamped to 16");
        evq_clear();
        REPORT.store(0, SeqCst);
        HIST.store(0, SeqCst);
        // no history: the current sample; the age of the sample
        TIME.store(980, SeqCst);
        at(20, 80, 61);
        send(w, WM_POINTERUPDATE, 20);
        let e = evq_pop().unwrap_or_default();
        t.check(e.typ == E_MOVE && e.x == 80 && (e.data >> PD_AGE_SHIFT) & PD_AGE_MASK == 20, "touch: the sample's age from its time");
        evq_clear();
        TIME.store(0, SeqCst);
        // above the screen, not in contact: no move
        FLAGS.store(0, SeqCst);
        at(20, 90, 61);
        send(w, WM_POINTERUPDATE, 20);
        no_events(t, "touch: a sample without contact is nothing");
        FLAGS.store(INCONTACT, SeqCst);

        // ---- a second finger has its own id, the first lifts
        at(21, 200, 100);
        send(w, WM_POINTERDOWN, 21);
        pop_finger(t, E_DOWN, 1, 200, 100, false, "touch: a second finger, id 1");
        at(21, 210, 100);
        send(w, WM_POINTERUPDATE, 21);
        pop_finger(t, E_MOVE, 1, 210, 100, false, "touch: it moves");
        at(20, 91, 62);
        send(w, WM_POINTERUP, 20);
        pop_finger(t, E_UP, 0, 91, 62, false, "touch: the first lifts");
        // the system takes the second: its message says so
        FLAGS.store(INCONTACT | CANCELED, SeqCst);
        send(w, WM_POINTERUPDATE, 21);
        pop_finger(t, E_UP, 1, 210, 100, true, "touch: canceled -> a cancel");
        FLAGS.store(INCONTACT, SeqCst);
        send(w, WM_POINTERUP, 21);
        no_events(t, "touch: its up: nothing more");
        // a capture that is lost
        at(22, 5, 6);
        send(w, WM_POINTERDOWN, 22);
        evq_clear();
        send(w, WM_POINTERCAPTURECHANGED, 22);
        pop_finger(t, E_UP, 0, 5, 6, true, "touch: capture lost -> a cancel");

        // ---- more fingers than there are ids: the rest are nobody's
        for k in 0..12 {
            at(20 + k, 10 + k as i32, 10);
            send(w, WM_POINTERDOWN, 20 + k as usize);
        }
        t.check(evq_count() == 10, "touch: ten fingers at once");
        evq_clear();
        for k in 0..12 {
            send(w, WM_POINTERUP, 20 + k as usize);
        }
        evq_clear();

        // ---- the pen is what it was
        send(w, WM_POINTERUPDATE, 7);
        no_events(t, "touch: a pen pointer does not come here (no pen API in this fake)");

        // ---- the non-client area: a caption of the main (top-level) window is the system's
        at(20, 30, 10);
        nc(w, WM_NCPOINTERDOWN, 20, HTCAPTION);
        no_events(t, "touch: the main window's caption goes to the system");
        // a border under nothing of ours: the system's as well
        nc(w, WM_NCPOINTERDOWN, 20, 15);
        no_events(t, "touch: a border under none of our client areas goes to the system");
        // an owned tool window: its caption is the UI's (press and hold)
        let k = win_create("touch tool", 120, 90, PWF_TOOL, w, 300, 300);
        t.check(k >= 0, "touch: hidden tool window");
        let ko = win_get_rect(k).unwrap_or_default();
        evq_clear();
        at(20, ko[0] - o[0] + 12, ko[1] - o[1] + 8);
        nc(k, WM_NCPOINTERDOWN, 20, HTCAPTION);
        let e = evq_pop().unwrap_or_default();
        t.check(e.typ == E_DOWN && e.win == k as u8 && e.x == 12 && e.y == 8, "touch: a tool window's caption -> a finger of that window");
        no_events(t, "touch: nothing else");

        // ---- from here on the window follows it, no moves for the UI
        win_begin_move(k);
        at(20, ko[0] - o[0] + 52, ko[1] - o[1] + 38);
        send(k, WM_POINTERUPDATE, 20);
        let r = win_get_rect(k).unwrap_or_default();
        t.check(r[0] == ko[0] + 40 && r[1] == ko[1] + 30, "touch: win_begin_move: the tool window follows the finger, every direction");
        let mut moves = 0;
        while let Some(e) = evq_pop() {
            moves += (e.typ == E_MOVE) as i32;
        }
        t.check(moves == 0, "touch: no moves for the UI while it follows");
        nc(k, WM_NCPOINTERUP, 20, HTCAPTION);
        let mut up = false;
        while let Some(e) = evq_pop() {
            up |= e.typ == E_UP && e.btn == 0;
        }
        t.check(up, "touch: the finger's up reaches the UI (in the window's new place)");
        // the mouse's way again without a finger: nothing to follow
        nc(k, WM_NCPOINTERUPDATE, 20, HTCAPTION);
        no_events(t, "touch: updates of a finger that lifted: nothing");

        // ---- the system's mouse move while a finger is the pointer: the cursor did not move
        at(23, 8, 8);
        send(w, WM_POINTERDOWN, 23);
        evq_clear();
        win_test_send(w, WM_MOUSEMOVE, 0, (20 << 16) | 20);
        no_events(t, "touch: a WM_MOUSEMOVE from where the cursor stands is no mouse");
        send(w, WM_POINTERUP, 23);
        evq_clear();

        // back to the real API (and its fresh state): a mouse move is one again
        touch_api_set(None);
        pen_api_set(None);
        win_test_send(w, WM_MOUSEMOVE, 0, (20 << 16) | 20);
        let e = evq_pop().unwrap_or_default();
        t.check(e.typ == E_MOVE && e.x == 20 && e.y == 20 && e.data == 0, "touch: without a finger a WM_MOUSEMOVE is the mouse");
        evq_clear();
        win_destroy(k);
        win_destroy(w);
    }
}

#[cfg(not(windows))]
mod lx {
    use alloc::vec::Vec;

    use plat::*;

    use super::{no_events, pop_finger};
    use crate::T;

    const TOUCH: &str = "wl_touch";

    fn ev(name: &str, a: &[i32]) {
        let v: Vec<u64> = a.iter().map(|&x| x as u32 as u64).collect();
        wl_test_event(TOUCH, name, &v);
    }

    /// an XIDeviceEvent: type, extension, length, evtype, device, time, detail, root, event, child,
    /// root x / y, event x / y (FP1616), buttons_len, valuators_len
    fn xi(op: u8, evtype: u16, time: u32, tid: u32, event: u32, rx: i32, ry: i32, x: i32, y: i32, len: usize) -> Vec<u8> {
        let mut b = alloc::vec![0u8; len.max(32)];
        b[0] = 35;
        b[1] = op;
        let extra = ((len.max(32) - 32) / 4) as u32;
        b[4..8].copy_from_slice(&extra.to_le_bytes());
        if len >= 60 {
            b[8..10].copy_from_slice(&evtype.to_le_bytes());
            b[10..12].copy_from_slice(&2u16.to_le_bytes());
            b[12..16].copy_from_slice(&time.to_le_bytes());
            b[16..20].copy_from_slice(&tid.to_le_bytes());
            b[24..28].copy_from_slice(&event.to_le_bytes());
            b[32..36].copy_from_slice(&(rx << 16).to_le_bytes());
            b[36..40].copy_from_slice(&(ry << 16).to_le_bytes());
            b[40..44].copy_from_slice(&(x << 16).to_le_bytes());
            b[44..48].copy_from_slice(&(y << 16).to_le_bytes());
        } else {
            b[8..10].copy_from_slice(&evtype.to_le_bytes());
        }
        b
    }

    pub fn t_touch(t: &mut T) {
        evq_clear();
        // ---- Wayland: wl_touch, scale 1 (no window: window 0)
        wl_test_scale(120);
        ev("down", &[1, 1000, 0, 5, 10 * 256, 20 * 256]);
        pop_finger(t, E_DOWN, 0, 10, 20, false, "wl: down -> a finger, slot 0");
        ev("motion", &[1010, 5, 15 * 256, 20 * 256]);
        pop_finger(t, E_MOVE, 0, 15, 20, false, "wl: motion");
        ev("frame", &[]);
        no_events(t, "wl: frame: nothing");
        // a second finger, its own slot, the first keeps its place
        ev("down", &[2, 1020, 0, 9, 100 * 256, 50 * 256]);
        pop_finger(t, E_DOWN, 1, 100, 50, false, "wl: a second finger, slot 1");
        ev("motion", &[1030, 9, 120 * 256, 50 * 256]);
        pop_finger(t, E_MOVE, 1, 120, 50, false, "wl: it moves");
        ev("motion", &[1030, 77, 1, 1]);
        no_events(t, "wl: motion of an unknown point: nothing");
        ev("up", &[3, 1040, 5]);
        pop_finger(t, E_UP, 0, 15, 20, false, "wl: the first lifts, at its last place");
        // the compositor takes the rest
        ev("cancel", &[]);
        pop_finger(t, E_UP, 1, 120, 50, true, "wl: cancel -> a cancel");
        ev("up", &[4, 1050, 9]);
        no_events(t, "wl: an up after that: nothing");
        // scale 2: px like the pointer's
        wl_test_scale(240);
        ev("down", &[5, 1060, 0, 1, 3 * 256 + 128, 4 * 256]);
        pop_finger(t, E_DOWN, 0, 7, 8, false, "wl: scale 2 -> 7, 8");
        // a down of a point that never lifted: it ends first
        ev("down", &[6, 1070, 0, 1, 10 * 256, 10 * 256]);
        pop_finger(t, E_UP, 0, 7, 8, true, "wl: the point came down again: the old one is cancelled");
        pop_finger(t, E_DOWN, 0, 20, 20, false, "wl: ... and the new one is there");
        ev("up", &[7, 1080, 1]);
        evq_clear();
        wl_test_scale(120);
        // more than ten: the rest are nobody's
        for k in 0..12 {
            ev("down", &[8 + k, 1090, 0, 100 + k, 256, 256]);
        }
        t.check(evq_count() == 10, "wl: ten fingers at once");
        ev("cancel", &[]);
        evq_clear();

        // ---- X11: XI 2.2 touch events
        let op = 131u8;
        x11t::xi_setup(op as u32, [500, 501, 502]);
        x11t::event_bytes(&xi(op, 18, 1000, 42, 0, 1040, 1050, 40, 50, 80));
        pop_finger(t, E_DOWN, 0, 40, 50, false, "x11: TouchBegin -> a finger, slot 0");
        x11t::event_bytes(&xi(op, 19, 1010, 42, 0, 1045, 1050, 45, 50, 80));
        pop_finger(t, E_MOVE, 0, 45, 50, false, "x11: TouchUpdate");
        x11t::event_bytes(&xi(op, 18, 1020, 43, 0, 1100, 1060, 100, 60, 80));
        pop_finger(t, E_DOWN, 1, 100, 60, false, "x11: a second touch, slot 1");
        x11t::event_bytes(&xi(op, 20, 1030, 42, 0, 1046, 1050, 46, 50, 80));
        pop_finger(t, E_UP, 0, 46, 50, false, "x11: TouchEnd of the first");
        x11t::event_bytes(&xi(op, 19, 1040, 42, 0, 1047, 1050, 47, 50, 80));
        no_events(t, "x11: an update after its end: nothing");
        x11t::event_bytes(&xi(op, 19, 1040, 43, 77777, 1, 1, 1, 1, 80));
        no_events(t, "x11: the window of an event that is not ours: nothing");
        x11t::event_bytes(&xi(op, 19, 1040, 43, 0, 0, 0, 0, 0, 40));
        no_events(t, "x11: an event too short for its fields: nothing");
        x11t::event_bytes(&xi(op, 20, 1050, 43, 0, 1100, 1060, 100, 60, 80));
        pop_finger(t, E_UP, 1, 100, 60, false, "x11: the second lifts");
        x11t::xi_setup(0, [500, 501, 502]);
        evq_clear();
    }
}
