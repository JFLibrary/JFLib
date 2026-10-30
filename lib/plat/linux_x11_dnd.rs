// linux_x11_dnd -- (dnd.rs) drag and drop through XDND 5: every window is a target, the
// app's drags leave as a source                               [src/plat/linux/x11_dnd.s]
//
//   dnd_window(win)                 XdndAware = 5 on every new window (the first call interns
//                                   the atoms, also image/bmp for the clipboard)
//   dnd_client(ev, index) -> consumed   ClientMessages: XdndEnter / Position / Leave / Drop
//                                   (target), XdndStatus / Finished (source)
//   dnd_deferred()                  (wait, after the event handlers: round trips are allowed
//                                   there) the work the messages and the pointer asked for
//   dnd_next() -> deadline          when dnd_deferred must run again (a drop that waits)
//   drag_start(data) -> bool        (Display) our drag, the left button held: XdndSelection,
//                                   XdndTypeList; dnd_motion / dnd_release / dnd_escape of the
//                                   input handlers drive it
//   dnd_request(ev)                 SelectionRequest of XdndSelection: our drag's content
//
// Target: Enter takes the source's first three types, or XdndTypeList when it has more
// (read in dnd_deferred).  Every Position is answered there: dnd::drag_query (the UI's hook)
// at the point in window pixels, with the source's action as the modifier keys (move = Shift,
// link = Ctrl+Shift), then XdndStatus (accept, "send more positions", the action) and E_DRAG
// when the point changed.  Leave: E_DRAG with data 0.  Drop: the kinds the UI took are read
// from XdndSelection with the drop's time (one ConvertSelection at a time through the
// clipboard's sel_convert: INCR, 3 s each, bounded): text/uri-list (paths: drop_uri_list,
// other URLs: drop_url_list), text/plain;charset=utf-8 / UTF8_STRING / text/plain / STRING /
// TEXT, text/html (UTF-16 with a BOM is converted), image/png, else image/bmp
// (clip_bmp_fix); then E_DROP and XdndFinished (success, the action).  Every message field is
// untrusted: the source window must match the one of XdndEnter.
// Source: drag_start owns XdndSelection and puts the types into XdndTypeList; the pointer on
// the root (motion of the implicit grab) is followed in dnd_deferred: the window under it
// (TranslateCoordinates from the root down, the first with XdndAware; XdndProxy honoured)
// gets XdndEnter / XdndPosition (the next one only after its XdndStatus) / XdndLeave; the
// release sends XdndDrop when the target accepted (else XdndLeave), XdndFinished ends it
// (10 s without: failed); Esc cancels (the key does not reach the UI).  E_DRAG_END carries
// the effect.  The content stays offered after the drag (a target may ask late):
// text/uri-list (file:// URIs of the paths, the URLs), the text types (the text, else the
// URLs), text/html, image/png, image/bmp, TARGETS, TIMESTAMP.  Our own windows are targets
// like any other.

use alloc::string::String;
use alloc::vec::Vec;

use super::*;
use crate::{DE_ALL, DE_COPY, DE_LINK, DE_MOVE, DT_FILES, DT_HTML, DT_IMAGE, DT_TEXT, DT_URL, DragData,
            E_DRAG, E_DROP, MOD_CTRL, MOD_SHIFT};

const XDND_VERSION: u32 = 5;
const DROP_TIMEOUT: u64 = 10_000_000; // us without XdndFinished
const STATUS_TIMEOUT: u64 = 2_000_000; // us after the release without the target's XdndStatus
const TYPES_MAX: u32 = 64; // atoms of an XdndTypeList we read
const FIND_DEPTH: usize = 16;
pub(super) const D_AWARE: usize = 0;
const D_ENTER: usize = 1;
const D_POSITION: usize = 2;
const D_STATUS: usize = 3;
const D_LEAVE: usize = 4;
const D_DROP: usize = 5;
const D_FINISHED: usize = 6;
pub(super) const D_SELECTION: usize = 7;
const D_COPY: usize = 8;
const D_URI: usize = 9;
const D_BMP: usize = 11;
const D_TYPELIST: usize = 12;
const D_MOVE: usize = 13;
const D_LINK: usize = 14;
const D_PROXY: usize = 15;
const D_HTML: usize = 16;
const D_HTML8: usize = 17;
pub(super) const D_COUNT: usize = 18;
const XD_NAMES: [&str; D_COUNT] = [
    "XdndAware",
    "XdndEnter",
    "XdndPosition",
    "XdndStatus",
    "XdndLeave",
    "XdndDrop",
    "XdndFinished",
    "XdndSelection",
    "XdndActionCopy",
    "text/uri-list",
    "jfgui_DND",
    "image/bmp",
    "XdndTypeList",
    "XdndActionMove",
    "XdndActionLink",
    "XdndProxy",
    "text/html",
    "text/html;charset=utf-8",
];

/// another program's drag over one of our windows
#[derive(Default)]
pub(super) struct Target {
    src: u32,
    win: i32,
    types: Vec<u32>,
    list: bool,
    offer: u32,
    /// a Position to answer: root x, y, the source's action
    pos: Option<(i32, i32, u32)>,
    x: i32,
    y: i32,
    shown: Option<(i32, i32)>,
    /// a Drop to do: its time
    drop: Option<u32>,
}

/// the app's drag
#[derive(Default)]
pub(super) struct Source {
    on: bool,
    /// what XdndSelection serves (kept after the drag)
    data: Vec<(u32, Vec<u8>)>,
    time: u32,
    win: u8,
    target: u32,
    dest: u32,
    ver: u32,
    accept: bool,
    action: u32,
    waiting: bool,
    moved: bool,
    rx: i32,
    ry: i32,
    release: bool,
    cancel: bool,
    dropped: u64,
    /// when the button was released while a status was still awaited (time_us; 0 = not)
    released_at: u64,
}

// %XX for every byte of a path that is not unreserved (or '/')
fn uri_of(path: &str) -> Vec<u8> {
    let mut u = b"file://".to_vec();
    for &c in path.as_bytes() {
        if c.is_ascii_alphanumeric() || matches!(c, b'/' | b'-' | b'.' | b'_' | b'~') {
            u.push(c);
        } else {
            u.extend_from_slice(alloc::format!("%{c:02X}").as_bytes());
        }
    }
    u
}

// the absolute path of p (relative ones from the working folder)
fn abs_path(p: &str) -> String {
    if p.starts_with('/') {
        return String::from(p);
    }
    let mut d = crate::cwd();
    if !d.ends_with('/') {
        d.push('/');
    }
    d.push_str(p.trim_start_matches("./"));
    d
}

// text/html of other programs: UTF-8, or UTF-16 LE with its BOM (Firefox)
fn html_bytes(b: &[u8]) -> Vec<u8> {
    if b.len() >= 2 && b[0] == 0xFF && b[1] == 0xFE {
        return crate::drop_text_from(&b[2..], true);
    }
    b.to_vec()
}

impl X11 {
    /// x11_dnd_window(win): XdndAware on a new window (interning the atoms once)
    pub(super) fn dnd_window(&mut self, win: u32) {
        if !self.xd_init {
            self.xd_init = true;
            let mut seqs = [0u32; D_COUNT];
            for (i, n) in XD_NAMES.iter().enumerate() {
                seqs[i] = self.intern_send(n.as_bytes(), false);
            }
            for i in 0..D_COUNT {
                self.xd_atoms[i] = self.atom_reply(seqs[i]);
            }
            self.xd_bmp_atom = self.xd_atoms[D_BMP];
        }
        let aware = self.xd_atoms[D_AWARE];
        if aware != 0 {
            self.change_prop32(win, aware, XA_ATOM, &[XDND_VERSION]);
        }
    }

    // the DT_* kinds of a list of target atoms
    fn kinds_of(&self, atoms: &[u32]) -> u32 {
        let (a, d) = (&self.atoms, &self.xd_atoms);
        let mut t = 0;
        for &x in atoms.iter().filter(|&&x| x != 0) {
            if x == d[D_URI] {
                t |= DT_FILES | DT_URL; // decided when the list is read
            } else if [a[A_UTF8_STRING], a[A_TEXT_PLAIN_UTF8], a[A_TEXT_PLAIN], a[A_TEXT], XA_STRING].contains(&x) {
                t |= DT_TEXT;
            } else if x == d[D_HTML] || x == d[D_HTML8] {
                t |= DT_HTML;
            } else if x == a[A_IMAGE_PNG] || x == d[D_BMP] {
                t |= DT_IMAGE;
            }
        }
        t
    }

    // the DE_* effect of an action atom (0: none / unknown)
    fn action_effect(&self, a: u32) -> u32 {
        let d = &self.xd_atoms;
        if a == 0 {
            0
        } else if a == d[D_MOVE] {
            DE_MOVE
        } else if a == d[D_LINK] {
            DE_LINK
        } else {
            DE_COPY // copy, ask, private
        }
    }

    // the action atom of an effect
    fn effect_action(&self, e: u32) -> u32 {
        match e {
            DE_MOVE => self.xd_atoms[D_MOVE],
            DE_LINK => self.xd_atoms[D_LINK],
            DE_COPY => self.xd_atoms[D_COPY],
            _ => 0,
        }
    }

    /// xd_send(dest, window, type, l0..l4): an XDND ClientMessage
    fn xd_send(&mut self, dest: u32, win: u32, typ: u32, l: [u32; 5]) {
        let mut ev = [0u8; 32];
        ev[0] = XE_CLIENT_MESSAGE;
        ev[1] = 32;
        wr32(&mut ev, 4, win);
        wr32(&mut ev, 8, typ);
        for (k, v) in l.iter().enumerate() {
            wr32(&mut ev, 12 + 4 * k, *v);
        }
        self.send_event(dest, 0, &ev, false);
        self.flush();
    }

    // our window of index w (0 = none)
    fn xd_ourwin(&self, w: i32) -> u32 {
        self.xw[(w as u32 as usize) & (PW_MAX - 1)].win
    }

    /// x11_dnd_client(ev, index) -> true when it was an XDND message
    pub(super) fn dnd_client(&mut self, b: &[u8], idx: i32) -> bool {
        let typ = rd32(b, 8);
        if typ == 0 || b[1] != 32 {
            return false;
        }
        let a = self.xd_atoms;
        let l = |k: usize| rd32(b, 12 + 4 * k);
        if typ == a[D_ENTER] {
            if self.xd.drop.is_some() {
                return true; // still reading the last drop
            }
            self.xd = Target::default();
            if l(1) >> 24 < 3 || l(0) == 0 {
                return true; // the source's version (or no source)
            }
            self.xd.src = l(0);
            self.xd.win = idx;
            self.xd.types = [l(2), l(3), l(4)].into_iter().filter(|&t| t != 0).collect();
            self.xd.list = l(1) & 1 != 0; // more than three types: XdndTypeList
            self.xd.offer = self.kinds_of(&self.xd.types.clone());
        } else if typ == a[D_POSITION] {
            if l(0) == 0 || l(0) != self.xd.src || self.xd.drop.is_some() {
                return true;
            }
            self.xd.win = idx;
            let p = l(2);
            self.xd.pos = Some(((p >> 16) as u16 as i16 as i32, p as u16 as i16 as i32, l(4)));
            self.time = l(3).max(self.time);
        } else if typ == a[D_LEAVE] {
            if l(0) != 0 && l(0) == self.xd.src && self.xd.drop.is_none() {
                self.evwin = self.xd.win;
                self.push(E_DRAG, 0, -1, -1, 0);
                self.xd = Target::default();
            }
        } else if typ == a[D_DROP] {
            if l(0) == 0 || l(0) != self.xd.src || self.xd.drop.is_some() {
                return true;
            }
            self.xd.win = idx;
            self.xd.drop = Some(l(2));
        } else if typ == a[D_STATUS] {
            let s = &mut self.xs;
            if s.on && l(0) == s.target && s.target != 0 {
                s.accept = l(1) & 1 != 0;
                s.action = l(4);
                s.waiting = false;
            }
        } else if typ == a[D_FINISHED] {
            if self.xs.on && l(0) == self.xs.target && self.xs.dropped != 0 {
                let ok = self.xs.ver < 5 || l(1) & 1 != 0;
                let act = if self.xs.ver >= 5 { l(2) } else { self.xs.action };
                let e = if !ok { 0 } else if act == 0 { DE_COPY } else { self.action_effect(act) };
                self.xs_finish(e);
            }
        } else {
            return false;
        }
        true
    }

    /// x11_dnd_deferred: what the XDND messages and the pointer asked for (round trips)
    pub(super) fn dnd_deferred(&mut self) {
        self.xd_deferred();
        self.xs_deferred();
    }

    /// the next time dnd_deferred must run (u64::MAX: none)
    pub(super) fn dnd_next(&self) -> u64 {
        if self.xs.on && self.xs.dropped != 0 {
            self.xs.dropped + DROP_TIMEOUT
        } else if self.xs.on && self.xs.released_at != 0 {
            self.xs.released_at + STATUS_TIMEOUT
        } else {
            u64::MAX
        }
    }

    // ------------------------------------------------------------------ target

    fn xd_deferred(&mut self) {
        if self.xd.src == 0 {
            return;
        }
        if self.xd.list {
            self.xd.list = false;
            let (src, tl) = (self.xd.src, self.xd_atoms[D_TYPELIST]);
            if let Some(r) = self.get_property(src, tl, XA_ATOM, 0, TYPES_MAX, false) {
                let n = (rd32(&r, 16) as usize).min((r.len() - 32) / 4);
                if rd32(&r, 8) == XA_ATOM && r[1] == 32 {
                    self.xd.types = (0..n).map(|k| rd32(&r, 32 + 4 * k)).filter(|&t| t != 0).collect();
                    self.xd.offer = self.kinds_of(&self.xd.types.clone());
                }
            }
        }
        if let Some((rx, ry, action)) = self.xd.pos.take() {
            let w = self.xd.win;
            let r = self.xw[(w as u32 as usize) & (PW_MAX - 1)];
            let (x, y) = if r.posok { (rx - r.rx, ry - r.ry) } else { (rx, ry) };
            self.xd.x = x;
            self.xd.y = y;
            let mods = match self.action_effect(action) {
                DE_MOVE => MOD_SHIFT,
                DE_LINK => MOD_SHIFT | MOD_CTRL,
                _ => 0,
            };
            let offer = self.xd.offer;
            let (_, e) = crate::drag_query(w, x, y, offer, DE_ALL, mods);
            let (src, ours, st) = (self.xd.src, self.xd_ourwin(w), self.xd_atoms[D_STATUS]);
            let act = self.effect_action(e);
            // accept, "send every position" (the targets differ by place), no rectangle
            self.xd_send(src, src, st, [ours, (e != 0) as u32 | 2, 0, 0, act]);
            if self.xd.shown != Some((x, y)) && offer != 0 {
                self.xd.shown = Some((x, y));
                self.evwin = w;
                self.push(E_DRAG, 0, x, y, offer | DE_ALL << 8);
            }
        }
        if let Some(time) = self.xd.drop {
            self.xd_drop(time);
        }
    }

    // the drop: read what the UI takes, E_DROP, XdndFinished
    fn xd_drop(&mut self, time: u32) {
        let (w, x, y, offer) = (self.xd.win, self.xd.x, self.xd.y, self.xd.offer);
        let (types, e) = crate::drag_query(w, x, y, offer, DE_ALL, 0);
        let (src, ours, fin) = (self.xd.src, self.xd_ourwin(w), self.xd_atoms[D_FINISHED]);
        self.evwin = w;
        if e == 0 {
            self.push(E_DRAG, 0, -1, -1, 0);
            self.xd_send(src, src, fin, [ours, 0, 0, 0, 0]);
            self.xd = Target::default();
            return;
        }
        let sel = self.xd_atoms[D_SELECTION];
        let has = |s: &X11, a: u32| a != 0 && s.xd.types.contains(&a);
        crate::drop_reset();
        let mut files = 0;
        // a source that does not answer (3 s) is not asked for its other types: every one
        // would wait as long again
        let mut dead = false;
        if types & (DT_FILES | DT_URL) != 0 && has(self, self.xd_atoms[D_URI]) {
            if let Some(d) = self.xd_conv(sel, self.xd_atoms[D_URI], time, crate::CLIP_RICH_MAX, &mut dead) {
                if types & DT_FILES != 0 {
                    files = crate::drop_uri_list(&d);
                }
                if types & DT_URL != 0 {
                    crate::drop_url_list(&d);
                }
            }
        }
        if types & DT_TEXT != 0 {
            let a = self.atoms;
            for t in [a[A_TEXT_PLAIN_UTF8], a[A_UTF8_STRING], a[A_TEXT_PLAIN], XA_STRING, a[A_TEXT]] {
                if !has(self, t) {
                    continue;
                }
                if let Some(d) = self.xd_conv(sel, t, time, crate::CLIP_RICH_MAX, &mut dead) {
                    let d = if t == XA_STRING { super::clip::from_latin1(&d) } else { d };
                    crate::drop_set_text(&d);
                    break;
                }
            }
        }
        if types & DT_HTML != 0 {
            for t in [self.xd_atoms[D_HTML], self.xd_atoms[D_HTML8]] {
                if has(self, t) {
                    if let Some(d) = self.xd_conv(sel, t, time, crate::CLIP_RICH_MAX, &mut dead) {
                        crate::drop_set_html(&html_bytes(&d));
                        break;
                    }
                }
            }
        }
        if types & DT_IMAGE != 0 {
            let png = self.atoms[A_IMAGE_PNG];
            let mut img = if has(self, png) { self.xd_conv(sel, png, time, super::clip::CLIP_MAX, &mut dead) } else { None };
            if img.is_none() {
                let bmp = self.xd_atoms[D_BMP];
                if has(self, bmp) {
                    img = self.xd_conv(sel, bmp, time, super::clip::CLIP_MAX, &mut dead).and_then(|d| crate::clip_bmp_fix(&d));
                }
            }
            if let Some(d) = img {
                crate::drop_set_image(d);
            }
        }
        crate::drop_set_effect(e);
        self.evwin = w;
        self.push(E_DROP, 0, x, y, files);
        let act = self.effect_action(e);
        self.xd_send(src, src, fin, [ours, 1, act, 0, 0]);
        self.xd = Target::default();
    }

    /// one type of the drop's data (None: refused or not answered; dead: the source stopped
    /// answering -- nothing more is asked)
    fn xd_conv(&mut self, sel: u32, target: u32, time: u32, limit: usize, dead: &mut bool) -> Option<Vec<u8>> {
        if *dead {
            return None;
        }
        match self.sel_convert(sel, target, time, limit) {
            Ok(d) => Some(d),
            Err(e) => {
                *dead |= e == -2;
                None
            }
        }
    }

    // ------------------------------------------------------------------ source

    /// (Display) drag `d` out of the app: the left button must be held -> started
    pub(super) fn drag_start(&mut self, d: DragData) -> bool {
        let win0 = self.xw[0].win;
        let (sel, tl) = (self.xd_atoms[D_SELECTION], self.xd_atoms[D_TYPELIST]);
        if win0 == 0 || sel == 0 || self.btn & (1 << crate::BTN_LEFT) == 0 || self.xs.on {
            return false;
        }
        // the content per target type
        let (a, x) = (self.atoms, self.xd_atoms);
        let mut data: Vec<(u32, Vec<u8>)> = Vec::new();
        if !d.files.is_empty() || !d.urls.is_empty() {
            let mut l = Vec::new();
            for f in &d.files {
                l.extend_from_slice(&uri_of(&abs_path(f)));
                l.extend_from_slice(b"\r\n");
            }
            for u in &d.urls {
                l.extend_from_slice(u.as_bytes());
                l.extend_from_slice(b"\r\n");
            }
            data.push((x[D_URI], l));
        }
        let text = d.text.clone().or_else(|| if d.urls.is_empty() { None } else { Some(d.urls.join("\n")) });
        if let Some(t) = text {
            let b = t.into_bytes();
            for at in [a[A_TEXT_PLAIN_UTF8], a[A_UTF8_STRING], a[A_TEXT_PLAIN], a[A_TEXT]] {
                data.push((at, b.clone()));
            }
            data.push((XA_STRING, super::clip::to_latin1(&b)));
        }
        if let Some(h) = &d.html {
            data.push((x[D_HTML], h.clone().into_bytes()));
        }
        if let Some(p) = &d.image {
            data.push((a[A_IMAGE_PNG], p.clone()));
        }
        if let Some(b) = &d.bmp {
            data.push((x[D_BMP], b.clone()));
        }
        data.retain(|e| e.0 != 0);
        if data.is_empty() {
            return false;
        }
        let time = self.time;
        // CurrentTime: an older event time than the last owner's would be ignored by the server
        let (o, _) = self.req(XR_SET_SELECTION_OWNER, 16);
        wr32(&mut self.out, o, win0);
        wr32(&mut self.out, o + 4, sel);
        wr32(&mut self.out, o + 8, 0);
        let types: Vec<u32> = data.iter().map(|e| e.0).collect();
        self.change_prop32(win0, tl, XA_ATOM, &types);
        self.flush();
        let w = self.btn_win;
        let r = self.xw[(w as u32 as usize) & (PW_MAX - 1)];
        let (rx, ry) = if r.posok { (r.rx + self.mx, r.ry + self.my) } else { (self.mx, self.my) };
        self.xs = Source { on: true, data, time, win: (w as u32 & (PW_MAX as u32 - 1)) as u8, moved: true, rx, ry, ..Source::default() };
        true
    }

    /// the pointer moved (root px) while our drag runs
    pub(super) fn dnd_motion(&mut self, rx: i32, ry: i32) {
        if self.xs.on {
            self.xs.rx = rx;
            self.xs.ry = ry;
            self.xs.moved = true;
        }
    }

    /// the left button was released while our drag runs
    pub(super) fn dnd_release(&mut self) {
        if self.xs.on {
            self.xs.release = true;
        }
    }

    /// Esc while our drag runs -> true: it was taken (the UI does not see the key)
    pub(super) fn dnd_escape(&mut self) -> bool {
        if self.xs.on {
            self.xs.cancel = true;
        }
        self.xs.on
    }

    // the drag ended with this effect
    fn xs_finish(&mut self, effect: u32) {
        self.xs.on = false;
        self.xs.target = 0;
        crate::dnd::drag_finished(self.xs.win, effect);
    }

    // a message to the target of our drag
    fn xs_send(&mut self, k: usize, l: [u32; 5]) {
        let (dest, target, typ) = (self.xs.dest, self.xs.target, self.xd_atoms[k]);
        self.xd_send(dest, target, typ, l);
    }

    // XdndAware of a window -> its version (None: not a target)
    fn xs_aware(&mut self, w: u32) -> Option<u32> {
        let aw = self.xd_atoms[D_AWARE];
        let r = self.get_property(w, aw, XA_ATOM, 0, 1, false)?;
        if rd32(&r, 8) == XA_ATOM && rd32(&r, 16) >= 1 && r.len() >= 36 { Some(rd32(&r, 32)) } else { None }
    }

    // the XDND window under the root point -> (target, where messages go, version)
    fn xs_find(&mut self, rx: i32, ry: i32) -> (u32, u32, u32) {
        let root = self.root;
        let mut w = root;
        for _ in 0..FIND_DEPTH {
            let (o, seq) = self.req(XR_TRANSLATE_COORDS, 16);
            wr32(&mut self.out, o, root);
            wr32(&mut self.out, o + 4, w);
            wr16(&mut self.out, o + 8, rx as u32);
            wr16(&mut self.out, o + 10, ry as u32);
            let Some(r) = self.reply(seq) else { break };
            let child = rd32(&r, 8);
            if child == 0 {
                break;
            }
            w = child;
            // a proxy (its own XdndProxy must point to itself)
            let px = self.xd_atoms[D_PROXY];
            let proxy = self.get_property(w, px, XA_WINDOW, 0, 1, false)
                .filter(|r| rd32(r, 8) == XA_WINDOW && rd32(r, 16) >= 1 && r.len() >= 36)
                .map(|r| rd32(&r, 32));
            if let Some(p) = proxy.filter(|&p| p != 0) {
                if let Some(v) = self.xs_aware(p) {
                    return (w, p, v);
                }
            }
            if let Some(v) = self.xs_aware(w) {
                return (w, w, v);
            }
        }
        (0, 0, 0)
    }

    fn xs_deferred(&mut self) {
        if !self.xs.on {
            return;
        }
        let win0 = self.xw[0].win;
        if self.xs.cancel {
            if self.xs.target != 0 {
                self.xs_send(D_LEAVE, [win0, 0, 0, 0, 0]);
            }
            self.xs_finish(0);
            return;
        }
        if self.xs.dropped != 0 {
            if crate::time_us() >= self.xs.dropped + DROP_TIMEOUT {
                self.xs_finish(0); // no XdndFinished: failed
            }
            return;
        }
        if self.xs.release {
            if self.xs.target != 0 && self.xs.waiting {
                // its answer to the last position decides -- unless it does not come (a hung
                // target): then it is no drop
                let now = crate::time_us();
                if self.xs.released_at == 0 {
                    self.xs.released_at = now.max(1);
                }
                if now < self.xs.released_at + STATUS_TIMEOUT {
                    return;
                }
                self.xs.waiting = false;
                self.xs.accept = false;
            }
            if self.xs.target != 0 && self.xs.accept {
                let t = self.time;
                self.xs_send(D_DROP, [win0, 0, t, 0, 0]);
                self.xs.dropped = crate::time_us().max(1);
            } else {
                if self.xs.target != 0 {
                    self.xs_send(D_LEAVE, [win0, 0, 0, 0, 0]);
                }
                self.xs_finish(0);
            }
            return;
        }
        if !self.xs.moved {
            return;
        }
        let (rx, ry) = (self.xs.rx, self.xs.ry);
        let (t, dest, ver) = self.xs_find(rx, ry);
        if t != self.xs.target {
            if self.xs.target != 0 {
                self.xs_send(D_LEAVE, [win0, 0, 0, 0, 0]);
            }
            self.xs.target = t;
            self.xs.dest = dest;
            self.xs.ver = ver.min(XDND_VERSION);
            self.xs.accept = false;
            self.xs.waiting = false;
            if t != 0 && self.xs.ver >= 3 {
                let types: Vec<u32> = self.xs.data.iter().map(|e| e.0).collect();
                let more = (types.len() > 3) as u32;
                let g = |k: usize| types.get(k).copied().unwrap_or(0);
                let v = self.xs.ver;
                self.xs_send(D_ENTER, [win0, v << 24 | more, g(0), g(1), g(2)]);
            } else {
                self.xs.target = 0;
            }
        }
        if self.xs.target == 0 || self.xs.waiting {
            return; // the next position after the target's status
        }
        self.xs.moved = false;
        let e = crate::drag_effect_pick(DE_ALL, self.mods as u8);
        let act = self.effect_action(e);
        let time = self.time;
        self.xs_send(D_POSITION, [win0, 0, (rx as u32 & 0xFFFF) << 16 | (ry as u32 & 0xFFFF), time, act]);
        self.xs.waiting = true;
    }

    /// x11_dnd_request(ev): SelectionRequest of XdndSelection -- our drag's content
    pub(super) fn dnd_request(&mut self, b: &[u8]) {
        let req = rd32(b, 12);
        let target = rd32(b, 20);
        let mut prop = rd32(b, 24);
        if prop == 0 {
            prop = target;
        }
        let mut answered = 0u32;
        let a = self.atoms;
        if target == a[A_TARGETS] {
            let mut list = Vec::from([a[A_TARGETS], a[A_TIMESTAMP]]);
            list.extend(self.xs.data.iter().map(|e| e.0));
            self.change_prop32(req, prop, XA_ATOM, &list);
            answered = prop;
        } else if target == a[A_TIMESTAMP] {
            let t = self.xs.time;
            self.change_prop32(req, prop, XA_INTEGER, &[t]);
            answered = prop;
        } else if let Some(d) = self.xs.data.iter().find(|e| e.0 == target).map(|e| e.1.clone()) {
            if self.clip_send(req, prop, target, d) {
                answered = prop;
            }
        }
        let mut ev = [0u8; 32];
        ev[0] = XE_SELECTION_NOTIFY;
        wr32(&mut ev, 4, rd32(b, 4)); // time
        wr32(&mut ev, 8, req);
        wr32(&mut ev, 12, rd32(b, 16)); // selection
        wr32(&mut ev, 16, target);
        wr32(&mut ev, 20, answered);
        self.send_event(req, 0, &ev, false);
        self.flush();
    }
}
