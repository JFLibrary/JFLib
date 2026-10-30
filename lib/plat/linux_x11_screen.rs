// linux_x11_screen -- X11: the monitor list, global shortcuts, pictures of the screen and the
// colour pick (screen.rs, hotkey.rs through linux_screen.rs / linux_hotkey.rs)
//
//   screen_init()            (the main window's set-up) RandR's first event and RRSelectInput
//                            (screen, CRTC and output changes), _NET_WORKAREA /
//                            _NET_CURRENT_DESKTOP interned
//   monitors() -> [Monitor]  RandR 1.5 GetMonitors (active ones: name atom -> GetAtomName,
//                            primary, rectangle); the refresh rate of the first output's CRTC
//                            mode (GetScreenResourcesCurrent, GetOutputInfo, GetCrtcInfo: dot
//                            clock / (htotal * vtotal), interlace x 2, double scan / 2); the
//                            work area = monitor ^ _NET_WORKAREA of the current desktop; the
//                            backend's dpi for every monitor.  Without RandR 1.5: the root.
//   hotkey_grab(id, chord) / hotkey_ungrab(id)   GrabKey on the root window, owner_events off
//                            (the press goes to the root even while a window of ours has the
//                            keyboard), for every keycode giving the chord's VK (xkb_vk_keys)
//                            and every combination of Lock and the Num Lock modifier; each
//                            grab is checked (BadAccess: another client has it -> Conflict, the
//                            grabs made are taken back)
//   capture(rect) -> Shot    GetImage (ZPixmap) of the root window in strips of at most 1 MB;
//                            a rootless X server (Xwayland, WSLg) refuses it (BadMatch): Failed
//   pick_start(test)         GrabPointer (crosshair) + GrabKeyboard on the root; a left click's
//                            root position, Esc / a right click cancels; the pixel is read
//                            after the event (deferred: GetImage of the root, else of the
//                            top-level window under the pointer, the way rootless servers
//                            allow); test = Some(x, y): that pixel at once, no grab
//   screen_event(packet) -> handled   (event()) KeyPress / KeyRelease / ButtonPress on the root
//                            window (shortcuts, the pick), RandR notifies and _NET_WORKAREA
//                            changes (-> E_SETTINGS SET_MONITORS once, from screen_deferred)
//   test aids: fake_input(type, detail, x, y) (XTEST FakeInput), image_of(win index, rect)
//                            (the GetImage path on one of our windows)
//
// The pixels: depth 24 / 32 with 32 bits per pixel (B, G, R, x in memory), the formats the
// presentation code supports too; anything else is Failed.

use alloc::string::String;
use alloc::vec::Vec;

use super::*;
use crate::hotkey::{Chord, HotkeyError, HotkeyEvent};
use crate::screen::{Monitor, ScreenError, Shot};
use crate::{CUR_CROSS, E_SETTINGS, MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_SUPER, SET_MONITORS};

const XR_GRAB_POINTER: u32 = 26;
const XR_GRAB_KEYBOARD: u32 = 31;
const XR_UNGRAB_KEYBOARD: u32 = 32;
const XR_GRAB_KEY: u32 = 33;
const XR_UNGRAB_KEY: u32 = 34;
const XR_GET_ATOM_NAME: u32 = 17;
const XR_GET_IMAGE: u32 = 73;
const RR_QUERY_VERSION: u32 = 0;
const RR_SELECT_INPUT: u32 = 4;
const RR_GET_OUTPUT_INFO: u32 = 9;
const RR_GET_CRTC_INFO: u32 = 20;
const RR_GET_SCREEN_RESOURCES_CURRENT: u32 = 25;
const RR_GET_MONITORS: u32 = 42;
const XTEST_FAKE_INPUT: u32 = 2;
const ZPIXMAP: u32 = 2;
const MASK_SHIFT: u32 = 1;
const MASK_LOCK: u32 = 2;
const MASK_CTRL: u32 = 4;
const BAD_ACCESS: u32 = 10;
const STRIP_BYTES: usize = 1 << 20; // GetImage reply per strip (the receive buffer holds 4 MB)

/// one passive grab of a shortcut
#[derive(Clone, Copy)]
pub(super) struct Grab {
    id: u32,
    kc: u8,
    mods: u32,
}

/// the pick: running, a click or a cancel waiting for screen_deferred
#[derive(Clone, Copy, Default)]
pub(super) struct Pick {
    on: bool,
    /// 1 = a click at x, y (child: the top-level window there), 2 = cancelled
    done: u8,
    x: i32,
    y: i32,
    child: u32,
}

impl X11 {
    /// (main window set-up) RandR change events, the work area atoms
    pub(super) fn screen_init(&mut self) {
        let seq = self.query_ext_send(b"RANDR");
        if let Some(r) = self.reply(seq) {
            if r[8] != 0 {
                self.sc_rr_op = r[9] as u32;
                self.sc_rr_ev = r[10] as u32;
                let (o, seq) = self.req(self.sc_rr_op | RR_QUERY_VERSION << 8, 12);
                wr32(&mut self.out, o, 1);
                wr32(&mut self.out, o + 4, 5);
                let v15 = self.reply(seq).is_some_and(|r| rd32(&r, 8) > 1 || rd32(&r, 12) >= 5);
                if !v15 {
                    self.sc_rr_op = 0; // GetMonitors needs 1.5
                }
                // screen change, CRTC change, output change
                let (o, _) = self.req(r[9] as u32 | RR_SELECT_INPUT << 8, 12);
                let root = self.root;
                wr32(&mut self.out, o, root);
                wr16(&mut self.out, o + 4, 7);
            }
        }
        let s1 = self.intern_send(b"_NET_WORKAREA", false);
        let s2 = self.intern_send(b"_NET_CURRENT_DESKTOP", false);
        self.sc_wa_atom = self.atom_reply(s1);
        self.sc_desk_atom = self.atom_reply(s2);
        self.flush();
    }

    // ------------------------------------------------------------------ monitors

    // a property of the root window as 32-bit values
    fn root_cards(&mut self, atom: u32) -> Vec<u32> {
        if atom == 0 {
            return Vec::new();
        }
        let root = self.root;
        match self.get_property(root, atom, XA_CARDINAL, 0, 256, false) {
            Some(r) if r[1] == 32 => {
                let n = rd32(&r, 16) as usize;
                (0..n).filter(|k| 32 + k * 4 + 4 <= r.len()).map(|k| rd32(&r, 32 + k * 4)).collect()
            }
            _ => Vec::new(),
        }
    }

    // the work area of the current desktop (None: the window manager does not say)
    fn work_area(&mut self) -> Option<[i32; 4]> {
        let wa = self.root_cards(self.sc_wa_atom);
        let d = self.root_cards(self.sc_desk_atom).first().copied().unwrap_or(0) as usize;
        let d = if wa.len() >= (d + 1) * 4 { d } else { 0 };
        (wa.len() >= (d + 1) * 4).then(|| [wa[d * 4] as i32, wa[d * 4 + 1] as i32, wa[d * 4 + 2] as i32, wa[d * 4 + 3] as i32])
    }

    fn atom_name(&mut self, atom: u32) -> String {
        let (o, seq) = self.req(XR_GET_ATOM_NAME, 8);
        wr32(&mut self.out, o, atom);
        match self.reply(seq) {
            Some(r) => {
                let n = (rd16(&r, 8) as usize).min(r.len().saturating_sub(32));
                String::from_utf8_lossy(&r[32..32 + n]).into_owned()
            }
            None => String::new(),
        }
    }

    // mHz of the first output of a monitor (0 = unknown)
    fn output_refresh(&mut self, output: u32, cfg_ts: u32, modes: &[(u32, u32)]) -> u32 {
        let op = self.sc_rr_op;
        let (o, seq) = self.req(op | RR_GET_OUTPUT_INFO << 8, 12);
        wr32(&mut self.out, o, output);
        wr32(&mut self.out, o + 4, cfg_ts);
        let Some(r) = self.reply(seq) else { return 0 };
        let crtc = rd32(&r, 12);
        if crtc == 0 {
            return 0;
        }
        let (o, seq) = self.req(op | RR_GET_CRTC_INFO << 8, 12);
        wr32(&mut self.out, o, crtc);
        wr32(&mut self.out, o + 4, cfg_ts);
        let Some(r) = self.reply(seq) else { return 0 };
        let mode = rd32(&r, 20);
        modes.iter().find(|m| m.0 == mode).map_or(0, |m| m.1)
    }

    /// every monitor (screen.rs sorts them)
    pub(super) fn monitors(&mut self) -> Vec<Monitor> {
        let dpi = self.dpi.max(96) as u32;
        let wa = self.work_area();
        let op = self.sc_rr_op;
        let mut out = Vec::new();
        if op != 0 {
            // the modes' refresh rates and the configuration timestamp
            let (o, seq) = self.req(op | RR_GET_SCREEN_RESOURCES_CURRENT << 8, 8);
            let root = self.root;
            wr32(&mut self.out, o, root);
            let mut modes: Vec<(u32, u32)> = Vec::new();
            let mut cfg_ts = 0;
            if let Some(r) = self.reply(seq) {
                cfg_ts = rd32(&r, 12);
                let (nc, no, nm) = (rd16(&r, 16) as usize, rd16(&r, 18) as usize, rd16(&r, 20) as usize);
                let mut p = 32 + (nc + no) * 4;
                for _ in 0..nm {
                    if p + 32 > r.len() {
                        break;
                    }
                    let (id, clock, ht, vt, fl) = (rd32(&r, p), rd32(&r, p + 8) as u64, rd16(&r, p + 16) as u64, rd16(&r, p + 24) as u64, rd32(&r, p + 28));
                    let mut mhz = if ht * vt != 0 { clock * 1000 / (ht * vt) } else { 0 };
                    if fl & 0x10 != 0 {
                        mhz *= 2; // interlaced
                    }
                    if fl & 0x20 != 0 {
                        mhz /= 2; // double scan
                    }
                    modes.push((id, mhz as u32));
                    p += 32;
                }
            }
            let (o, seq) = self.req(op | RR_GET_MONITORS << 8, 12);
            wr32(&mut self.out, o, root);
            wr32(&mut self.out, o + 4, 1); // active monitors only
            let mut raw: Vec<(u32, bool, [i32; 4], u32)> = Vec::new();
            if let Some(r) = self.reply(seq) {
                let count = rd32(&r, 12) as usize;
                let mut p = 32;
                for _ in 0..count {
                    if p + 24 > r.len() {
                        break;
                    }
                    let nout = rd16(&r, p + 6) as usize;
                    let first = if nout > 0 && p + 28 <= r.len() { rd32(&r, p + 24) } else { 0 };
                    raw.push((rd32(&r, p), r[p + 4] != 0, [rds16(&r, p + 8), rds16(&r, p + 10), rd16(&r, p + 12) as i32, rd16(&r, p + 14) as i32], first));
                    p += 24 + nout * 4;
                }
            }
            for (atom, primary, rect, first) in raw {
                let name = self.atom_name(atom);
                let refresh_mhz = if first != 0 { self.output_refresh(first, cfg_ts, &modes) } else { 0 };
                out.push(Monitor { id: atom as u64, name, rect, work: clip(rect, wa), dpi, refresh_mhz, primary });
            }
        }
        if out.is_empty() {
            let rect = [0, 0, self.scr_w as i32, self.scr_h as i32];
            out.push(Monitor { id: 0, name: String::new(), rect, work: clip(rect, wa), dpi, refresh_mhz: 0, primary: true });
        }
        out
    }

    // ------------------------------------------------------------------ global shortcuts

    // the real modifier mask of MOD_* bits (Alt / Super: their roles, else Mod1 / Mod4)
    fn real_mods(&self, m: u8) -> u32 {
        let mut r = 0;
        if m & MOD_SHIFT != 0 {
            r |= MASK_SHIFT;
        }
        if m & MOD_CTRL != 0 {
            r |= MASK_CTRL;
        }
        if m & MOD_ALT != 0 {
            r |= if self.role_alt != 0 { self.role_alt & self.role_alt.wrapping_neg() } else { 8 };
        }
        if m & MOD_SUPER != 0 {
            r |= if self.role_super != 0 { self.role_super & self.role_super.wrapping_neg() } else { 64 };
        }
        r
    }

    // the lock modifiers ignored for a match: Lock and Num Lock's mask
    fn lock_mask(&self) -> u32 {
        MASK_LOCK | self.role_num
    }

    fn grab_key(&mut self, kc: u8, mods: u32, on: bool) -> u32 {
        let root = self.root;
        if on {
            let (o, seq) = self.req(XR_GRAB_KEY, 16); // owner_events 0
            wr32(&mut self.out, o, root);
            wr16(&mut self.out, o + 4, mods);
            self.out[o + 6] = kc;
            self.out[o + 7] = 1; // pointer mode async
            self.out[o + 8] = 1; // keyboard mode async
            self.watch(seq);
            self.sync();
            self.watch_error()
        } else {
            let (o, _) = self.req(XR_UNGRAB_KEY | (kc as u32) << 8, 12);
            wr32(&mut self.out, o, root);
            wr16(&mut self.out, o + 4, mods);
            0
        }
    }

    // the modifier sets one shortcut grabs: mods with every combination of the locks
    fn lock_combos(&self, mods: u32) -> Vec<u32> {
        let mut v = Vec::new();
        for l in [0, MASK_LOCK, self.role_num, MASK_LOCK | self.role_num] {
            if !v.contains(&(mods | l)) {
                v.push(mods | l);
            }
        }
        v
    }

    pub(super) fn hotkey_grab(&mut self, id: u32, chord: Chord) -> Result<(), HotkeyError> {
        if self.fd < 0 || !self.kb_ready {
            return Err(HotkeyError::Unavailable);
        }
        let kcs: Vec<u8> = crate::xkb_vk_keys(chord.vk).into_iter().map(|e| e + 8).filter(|&k| k <= 255).map(|k| k as u8).collect();
        if kcs.is_empty() {
            return Err(HotkeyError::Invalid); // no key of this keyboard gives it
        }
        let mods = self.real_mods(chord.mods);
        let mut made: Vec<(u8, u32)> = Vec::new();
        for kc in kcs.iter().copied() {
            for m in self.lock_combos(mods) {
                let err = self.grab_key(kc, m, true);
                if err != 0 {
                    for (k, mm) in made {
                        self.grab_key(k, mm, false);
                    }
                    self.flush();
                    return Err(if err == BAD_ACCESS { HotkeyError::Conflict } else { HotkeyError::Failed(alloc::format!("GrabKey: X error {err}")) });
                }
                made.push((kc, m));
            }
        }
        for kc in kcs {
            self.sc_grabs.push(Grab { id, kc, mods });
        }
        Ok(())
    }

    pub(super) fn hotkey_ungrab(&mut self, id: u32) {
        let gone: Vec<Grab> = self.sc_grabs.iter().filter(|g| g.id == id).copied().collect();
        self.sc_grabs.retain(|g| g.id != id);
        for g in gone {
            for m in self.lock_combos(g.mods) {
                self.grab_key(g.kc, m, false);
            }
            if self.sc_key_down == g.kc as u32 {
                self.sc_key_down = 0;
            }
        }
        self.flush();
    }

    // ------------------------------------------------------------------ pictures

    /// GetImage of drawable at x, y, w x h (ZPixmap, strips) -> B, G, R, x rows
    fn drawable_image(&mut self, drawable: u32, r: [i32; 4]) -> Result<Vec<u8>, ScreenError> {
        let (w, h) = (r[2].max(0) as usize, r[3].max(0) as usize);
        let mut px = Vec::new();
        if px.try_reserve_exact(w * h * 4).is_err() {
            return Err(ScreenError::Failed(String::from("no memory for the picture")));
        }
        let rows = (STRIP_BYTES / (w * 4).max(1)).max(1);
        let mut y = 0;
        while y < h {
            let n = rows.min(h - y);
            let (o, seq) = self.req(XR_GET_IMAGE | ZPIXMAP << 8, 20);
            wr32(&mut self.out, o, drawable);
            wr16(&mut self.out, o + 4, r[0] as u32);
            wr16(&mut self.out, o + 6, (r[1] + y as i32) as u32);
            wr16(&mut self.out, o + 8, w as u32);
            wr16(&mut self.out, o + 10, n as u32);
            wr32(&mut self.out, o + 12, 0xFFFF_FFFF); // plane mask
            let Some(rep) = self.reply(seq) else {
                return Err(ScreenError::Failed(String::from("the X server gave no picture (GetImage refused: a rootless server such as Xwayland?)")));
            };
            let depth = rep[1];
            let data = &rep[32..];
            if !(depth == 24 || depth == 32) || data.len() < w * n * 4 {
                return Err(ScreenError::Failed(alloc::format!("unsupported pixel format (depth {depth})")));
            }
            for row in data[..w * n * 4].chunks_exact(4) {
                px.extend_from_slice(&[row[0], row[1], row[2], 255]);
            }
            y += n;
        }
        Ok(px)
    }

    pub(super) fn capture(&mut self, rect: Option<[i32; 4]>) -> Result<Shot, ScreenError> {
        if self.fd < 0 {
            return Err(ScreenError::Unavailable);
        }
        let mut r = rect.unwrap_or([0, 0, self.scr_w as i32, self.scr_h as i32]);
        // inside the root (GetImage outside it is a BadMatch)
        let (sw, sh) = (self.scr_w as i32, self.scr_h as i32);
        let (x0, y0) = (r[0].clamp(0, sw), r[1].clamp(0, sh));
        let (x1, y1) = (r[0].saturating_add(r[2]).clamp(x0, sw), r[1].saturating_add(r[3]).clamp(y0, sh));
        r = [x0, y0, x1 - x0, y1 - y0];
        if r[2] <= 0 || r[3] <= 0 {
            return Err(ScreenError::Failed(String::from("the rectangle lies outside the screen")));
        }
        let root = self.root;
        let px = self.drawable_image(root, r)?;
        Ok(Shot { rect: r, w: r[2], h: r[3], px, file: None })
    }

    /// (test aid) the GetImage path on window w (window px)
    pub(super) fn image_of(&mut self, w: i32, r: [i32; 4]) -> Result<Shot, ScreenError> {
        let i = self.rec(w).ok_or(ScreenError::Unavailable)?;
        let win = self.xw[i].win;
        let px = self.drawable_image(win, r)?;
        Ok(Shot { rect: r, w: r[2], h: r[3], px, file: None })
    }

    // the top-level window at root x, y (0: none)
    fn top_at(&mut self, x: i32, y: i32) -> u32 {
        let root = self.root;
        let (o, seq) = self.req(XR_TRANSLATE_COORDS, 16);
        wr32(&mut self.out, o, root);
        wr32(&mut self.out, o + 4, root);
        wr16(&mut self.out, o + 8, x as u32);
        wr16(&mut self.out, o + 10, y as u32);
        self.reply(seq).map_or(0, |t| rd32(&t, 8))
    }

    // the colour at root x, y: the root's picture, else the top-level window's there (child:
    // as the event named it, 0 = ask)
    fn pixel_at(&mut self, x: i32, y: i32, child: u32) -> Result<u32, ScreenError> {
        let root = self.root;
        let r = match self.drawable_image(root, [x, y, 1, 1]) {
            Ok(p) => Ok(p),
            Err(e) => {
                let child = if child != 0 { child } else { self.top_at(x, y) };
                if child == 0 {
                    return Err(e);
                }
                // rootless servers: the window's own picture (TranslateCoordinates root -> child)
                let (o, seq) = self.req(XR_TRANSLATE_COORDS, 16);
                wr32(&mut self.out, o, root);
                wr32(&mut self.out, o + 4, child);
                wr16(&mut self.out, o + 8, x as u32);
                wr16(&mut self.out, o + 10, y as u32);
                match self.reply(seq) {
                    Some(t) => self.drawable_image(child, [rds16(&t, 12), rds16(&t, 14), 1, 1]),
                    None => Err(e),
                }
            }
        }?;
        Ok((r[2] as u32) << 16 | (r[1] as u32) << 8 | r[0] as u32)
    }

    pub(super) fn pick_start(&mut self, test: Option<(i32, i32)>) -> Result<(), ScreenError> {
        if self.fd < 0 {
            return Err(ScreenError::Unavailable);
        }
        if let Some((x, y)) = test {
            let c = self.pixel_at(x, y, 0);
            crate::screen::push_color(c);
            return Ok(());
        }
        let root = self.root;
        let cursor = self.cursor_get(CUR_CROSS);
        let (o, seq) = self.req(XR_GRAB_POINTER, 24); // owner_events 0
        wr32(&mut self.out, o, root);
        wr16(&mut self.out, o + 4, XM_BUTTON_PRESS | XM_BUTTON_RELEASE);
        self.out[o + 6] = 1; // pointer mode async
        self.out[o + 7] = 1; // keyboard mode async
        wr32(&mut self.out, o + 8, 0); // confine to: none
        wr32(&mut self.out, o + 12, cursor);
        wr32(&mut self.out, o + 16, 0); // CurrentTime
        let st = self.reply(seq).map_or(255, |r| r[1]);
        if st != 0 {
            return Err(if st == 1 { ScreenError::Busy } else { ScreenError::Failed(alloc::format!("GrabPointer: status {st}")) });
        }
        let (o, seq) = self.req(XR_GRAB_KEYBOARD, 16);
        wr32(&mut self.out, o, root);
        wr32(&mut self.out, o + 4, 0);
        self.out[o + 8] = 1;
        self.out[o + 9] = 1;
        let _ = self.reply(seq); // Esc cancels only with the keyboard; the click works anyway
        self.sc_pick = Pick { on: true, ..Pick::default() };
        Ok(())
    }

    fn pick_ungrab(&mut self) {
        let (o, _) = self.req(XR_UNGRAB_POINTER, 8);
        wr32(&mut self.out, o, 0);
        let (o, _) = self.req(XR_UNGRAB_KEYBOARD, 8);
        wr32(&mut self.out, o, 0);
        self.flush();
    }

    // ------------------------------------------------------------------ events

    /// (event()) shortcuts and the pick on the root window, RandR and work area changes ->
    /// true when the event was handled here
    pub(super) fn screen_event(&mut self, b: &[u8]) -> bool {
        let typ = b[0] & 0x7F;
        let root = self.root;
        if root == 0 {
            return false; // no server: linuxtest's synthetic events
        }
        if self.sc_rr_ev != 0 && (typ as u32 == self.sc_rr_ev || typ as u32 == self.sc_rr_ev + 1) {
            if typ as u32 == self.sc_rr_ev && rd32(b, 12) == root {
                // RRScreenChangeNotify: the root's new size (width / height at 24 / 26, before
                // the rotation in byte 1: 90 / 270 degrees swap them, as Xlib does)
                let (w, h) = (rd16(b, 24) as u32, rd16(b, 26) as u32);
                if w > 0 && h > 0 {
                    (self.scr_w, self.scr_h) = if b[1] & (2 | 8) != 0 { (h, w) } else { (w, h) };
                }
            }
            self.sc_changed = true;
            self.xm_valid = false; // monitor_rect asks again
            return true;
        }
        if typ == XE_PROPERTY && rd32(b, 4) == root && rd32(b, 8) == self.sc_wa_atom && self.sc_wa_atom != 0 {
            self.sc_changed = true;
            return false; // the backend may look at it too
        }
        if !(XE_KEY_PRESS..=XE_BUTTON_RELEASE).contains(&typ) || rd32(b, 12) != root {
            return false;
        }
        // the user's input: its time stamps the next selection claim / activation (an older
        // one would be refused)
        self.time = rd32(b, 4);
        let kc = b[1] as u32;
        if self.sc_pick.on {
            match typ {
                XE_BUTTON_PRESS if kc == 1 => {
                    self.sc_pick = Pick { on: true, done: 1, x: rds16(b, XEV_ROOT_X), y: rds16(b, XEV_ROOT_Y), child: rd32(b, 16) };
                }
                XE_BUTTON_PRESS if kc == 3 => self.sc_pick.done = 2,
                XE_KEY_PRESS => {
                    if crate::xkb_key(kc.wrapping_sub(8)).0 == 0x1B {
                        self.sc_pick.done = 2; // Esc
                    }
                }
                _ => {}
            }
            return true;
        }
        let state = rd16(b, XEV_STATE) as u32 & 0xFF & !self.lock_mask();
        match typ {
            XE_KEY_PRESS => {
                if let Some(g) = self.sc_grabs.iter().find(|g| g.kc as u32 == kc && g.mods == state).copied() {
                    if self.sc_key_down != kc {
                        self.sc_key_down = kc; // held keys repeat: one press only
                        crate::hotkey::push(HotkeyEvent::Pressed(g.id));
                    }
                }
                true
            }
            XE_KEY_RELEASE => {
                if self.sc_key_down == kc {
                    self.sc_key_down = 0;
                }
                true
            }
            _ => false,
        }
    }

    /// (deferred()) the pick's pixel (round trips: never inside event()), one SET_MONITORS
    pub(super) fn screen_deferred(&mut self) {
        if self.sc_changed {
            self.sc_changed = false;
            let ev = self.evwin;
            self.evwin = 0;
            self.push(E_SETTINGS, 0, 0, 0, SET_MONITORS);
            self.evwin = ev;
        }
        let p = self.sc_pick;
        if p.on && p.done != 0 {
            self.sc_pick = Pick::default();
            self.pick_ungrab();
            let r = if p.done == 1 { self.pixel_at(p.x, p.y, p.child) } else { Err(ScreenError::Cancelled) };
            crate::screen::push_color(r);
        }
    }

    /// (test aid) XTEST FakeInput: type 2 / 3 key press / release (detail = keycode), 4 / 5
    /// button press / release, 6 motion to the root point x, y -> false without XTEST
    pub(super) fn fake_input(&mut self, typ: u32, detail: u32, x: i32, y: i32) -> bool {
        if self.sc_xt_op == 0 {
            let seq = self.query_ext_send(b"XTEST");
            self.sc_xt_op = match self.reply(seq) {
                Some(r) if r[8] != 0 => r[9] as u32,
                _ => u32::MAX,
            };
        }
        if self.sc_xt_op == u32::MAX {
            return false;
        }
        let root = self.root;
        let (o, _) = self.req(self.sc_xt_op | XTEST_FAKE_INPUT << 8, 36);
        self.out[o] = typ as u8;
        self.out[o + 1] = detail as u8;
        wr32(&mut self.out, o + 4, 0); // CurrentTime
        wr32(&mut self.out, o + 8, if typ == 6 { root } else { 0 });
        wr16(&mut self.out, o + 20, x as u32);
        wr16(&mut self.out, o + 22, y as u32);
        self.flush();
        true
    }
}

// a ^ b (b None: a; an empty intersection: a)
fn clip(a: [i32; 4], b: Option<[i32; 4]>) -> [i32; 4] {
    let Some(b) = b else { return a };
    let (l, t) = (a[0].max(b[0]), a[1].max(b[1]));
    let (r, d) = ((a[0] + a[2]).min(b[0] + b[2]), (a[1] + a[3]).min(b[1] + b[3]));
    if r > l && d > t { [l, t, r - l, d - t] } else { a }
}
