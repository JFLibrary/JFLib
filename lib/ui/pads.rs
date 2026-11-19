// pads -- (G7a part 2) the game pads for the app and the test scripts (plat::pads)
//
//   ui.gamepads()            the pads connected now with the edges of their buttons in this batch;
//                            the first call starts watching (plat::pad_watch; a headless run: the
//                            script's pads only)
//   ui.gamepads_watching()   pads are being watched (false where the platform has none)
//   pads_event(data)         (E_GAMEPAD) the pads anew, the edges added up until the batch's first
//                            pass took them (pads_clear_edges); App::command(UIC_GAMEPAD, data) for a
//                            pad that came or went
//   sc_pad                   the script command `pad ...` (auto.rs)
//
// Pads belong to the app, not to a window: E_GAMEPAD comes for window 0 and every window's pass sees
// the same list.  plat reports pads as nothing held while no window of the app is active.

use alloc::string::String;
use alloc::vec::Vec;

use plat::{Gamepad, PAD_AXES, PAD_BUTTONS, PadState, PadStyle};

use super::consts::*;
use super::{App, Ui};

impl Ui {
    /// (G7a) the game pads connected now (plat::Gamepad: index, name, style, the W3C standard layout's
    /// buttons with analog values, the sticks in -1 ..= 1 right and down positive, the triggers as
    /// buttons PAD_LT / PAD_RT), each with the buttons that went down and up since the last batch
    /// (`down`, `up`: a tap between two frames is in both).  The first call starts watching for pads
    /// (plat::pad_watch): they appear in later frames, each change gives a pass, a pad that comes or
    /// goes gives App::command(UIC_GAMEPAD, index | PADEV_* << 8).  While none of the app's windows
    /// has the focus every pad reports nothing held.  Windows: XInput (Xbox-compatible pads); Linux:
    /// evdev; macOS: GameController; the browser: the Gamepad API (a pad appears after one of its
    /// buttons was pressed in the page); Android and iOS: none.  A headless run has the pads of its
    /// script (`pad ...`)
    pub fn gamepads(&mut self) -> &[Gamepad] {
        if !self.pads_on {
            self.pads_on = true;
            if !self.headless {
                plat::pad_watch(true);
            }
        }
        &self.pads
    }

    /// (G7a) pads are being watched: ui.gamepads was called and the platform has pads (false on
    /// Android and iOS, on Windows without XInput); a headless run: after ui.gamepads
    pub fn gamepads_watching(&self) -> bool {
        self.pads_on && (self.headless || plat::pad_watching())
    }

    /// (runtime, E_GAMEPAD) the pads anew; their edges add up until the batch's first pass took them
    pub(crate) fn pads_event(&mut self, data: u32) {
        let mut fresh = plat::pads();
        for p in &mut fresh {
            if let Some(old) = self.pads.iter().find(|o| o.index == p.index) {
                p.down |= old.down;
                p.up |= old.up;
            }
        }
        self.pads = fresh;
        let kind = data >> 8;
        if kind == plat::PADEV_CONNECT || kind == plat::PADEV_DISCONNECT {
            self.cmd_run(UIC_GAMEPAD, data);
        }
    }

    /// (ctx.rs input_clear_edges) the batch's first pass took the pads' edges
    pub(crate) fn pads_clear_edges(&mut self) {
        for p in &mut self.pads {
            p.down = 0;
            p.up = 0;
        }
    }

    // pad connect N [xbox|playstation|nintendo|other] [NAME...] | disconnect N | press N BUTTON [VALUE] |
    // release N BUTTON | axis N lx|ly|rx|ry VALUE
    pub(crate) fn sc_pad(&mut self, app: &mut dyn App, a: &[u8]) {
        const USE: &[u8] = b"pad connect N [xbox|playstation|nintendo|other] [NAME], disconnect N, press N BUTTON [VALUE], release N BUTTON or axis N lx|ly|rx|ry VALUE";
        let line = String::from_utf8_lossy(a).into_owned();
        let w: Vec<&str> = line.split_whitespace().collect();
        let Some(n) = w.get(1).and_then(|s| s.parse::<usize>().ok()).filter(|&n| n < plat::PAD_MAX) else {
            self.fail(USE);
            return;
        };
        let cur = plat::pad(n);
        match w.first().copied() {
            Some("connect") => {
                let (style, rest) = match w.get(2).copied() {
                    Some("xbox") => (PadStyle::Xbox, 3),
                    Some("playstation") => (PadStyle::PlayStation, 3),
                    Some("nintendo") => (PadStyle::Nintendo, 3),
                    Some("other") => (PadStyle::Other, 3),
                    _ => (PadStyle::Other, 2),
                };
                let name = if w.len() > rest { w[rest..].join(" ") } else { alloc::format!("Pad {n}") };
                plat::pad_test_connect(n, &name, style);
            }
            Some("disconnect") => plat::pad_test_disconnect(n),
            Some(op @ ("press" | "release")) => {
                let Some(b) = w.get(2).and_then(|s| (0..PAD_BUTTONS).find(|&b| plat::pad_button_name(b).eq_ignore_ascii_case(s))) else {
                    self.fail(USE);
                    return;
                };
                let v = if op == "press" { w.get(3).and_then(|s| s.parse::<f32>().ok()).unwrap_or(1.0) } else { 0.0 };
                let mut st: PadState = cur.state();
                st.buttons[b] = v;
                let held = if b == plat::PAD_LT || b == plat::PAD_RT { v > plat::PAD_TRIGGER_PRESS } else { v > 0.0 };
                if held {
                    st.pressed |= 1 << b;
                } else {
                    st.pressed &= !(1 << b);
                }
                plat::pad_test_state(n, &st);
            }
            Some("axis") => {
                let k = match w.get(2).copied() {
                    Some("lx") => 0,
                    Some("ly") => 1,
                    Some("rx") => 2,
                    Some("ry") => 3,
                    _ => PAD_AXES,
                };
                let Some(v) = w.get(3).and_then(|s| s.parse::<f32>().ok()).filter(|_| k < PAD_AXES) else {
                    self.fail(USE);
                    return;
                };
                let mut st = cur.state();
                st.axes[k] = v;
                plat::pad_test_state(n, &st);
            }
            _ => {
                self.fail(USE);
                return;
            }
        }
        // the host would wake the app now: the event, then a frame
        self.process_events(app);
        self.run_passes(app);
    }
}
