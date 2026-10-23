// linux_x11_clip -- the clipboard through the X11 CLIPBOARD selection (drag and drop:
// linux_x11_dnd.rs, which reads its drops through sel_convert)  [src/plat/linux/x11_clip.s]
//
//   clip_set(fmt, data) -> bool      CLIP_TEXT (UTF-8) / CLIP_PNG / CLIP_IMAGE (CLIPIMG bytes,
//        clipimg_pack: targets image/png + image/bmp).  The data is copied; we own CLIPBOARD
//        (timestamp of the last event) until another client takes it (SelectionClear).
//   clip_get(fmt) -> data            our own selection directly; otherwise ConvertSelection
//        into a property of our window, wait for SelectionNotify (other events keep being
//        handled, 3 s timeout), read it (INCR transfers included).  Text: UTF8_STRING,
//        text/plain;charset=utf-8, STRING (Latin-1, converted).  Images: image/png;
//        CLIP_IMAGE: TARGETS first, then image/png, else image/bmp (through clip_bmp_fix:
//        paint.net's alpha rule).  At most CLIP_MAX bytes.
//   clip_has(fmt) -> bool            the CLIPBOARD owner offers fmt: our own data directly;
//        else GetSelectionOwner at most every 250 ms and TARGETS (300 ms timeout) when the
//        owner changed or the answer is older than 2 s
//   clip_request(ev)                 SelectionRequest: TARGETS, TIMESTAMP, UTF8_STRING, TEXT,
//        text/plain;charset=utf-8, text/plain, STRING (UTF-8 -> Latin-1), image/png,
//        image/bmp (CLIP_IMAGE); MULTIPLE and others are refused.  Data that does not fit
//        into one request (min(maximum request, request buffer) - 64 bytes: ~1 MB with
//        BIG-REQUESTS, 256 KB without) goes out with INCR in 64 KB chunks, driven by
//        PropertyNotify (Deleted) on the requestor.
//   clip_notify / clip_clear / clip_property (event handlers), clip_init (the selection)
//   sel_convert(selection, target, time, limit) -> data   ConvertSelection of any selection
//        (XdndSelection of a drop), waited for as clip_get does
//   clip_send(requestor, property, type, data)   one answer, INCR for large data (also the
//        answers of our drag, linux_x11_dnd.rs)
//
// Test aids: jfgui_X11SELECTION=NAME uses that selection instead of CLIPBOARD (WSLg's Weston
// mirrors CLIPBOARD and crashes after reading any INCR selection); jfgui_X11INCR=N sends
// everything above N bytes with INCR in N / 2 chunks.

use alloc::vec::Vec;

use super::*;
use crate::{CLIP_DOCX, CLIP_HTML, CLIP_RTF, CLIP_IMAGE, CLIP_PNG, CLIP_TEXT};

pub(super) const XF_MAX: usize = 4; // concurrent INCR transfers
const INCR_CHUNK: u32 = 65536;
const INCR_MIN: u32 = 256;
const CLIP_TIMEOUT: u64 = 3000000; // us
pub(super) const CLIP_MAX: usize = 1 << 29; // largest paste we read
const HAS_POLL: u64 = 250000; // us between two GetSelectionOwner of clip_has
const HAS_FRESH: u64 = 2000000; // us an owner's TARGETS answer is kept
const HAS_TIMEOUT: u64 = 300000; // us for that TARGETS answer
const HB_TEXT: u32 = 1; // clip_has cache bits
const HB_PNG: u32 = 2;
const HB_BMP: u32 = 4;

/// utf8_decode of core/str.s at d[i] (bytes past the end read as NUL) -> (codepoint, next);
/// invalid bytes, overlong forms, surrogates and values above U+10FFFF decode as U+FFFD and
/// advance one byte
fn utf8_decode(d: &[u8], i: usize) -> (u32, usize) {
    let at = |k: usize| d.get(k).copied().unwrap_or(0) as u32;
    let c = at(i);
    if c < 0x80 {
        return (c, i + 1);
    }
    let (mut v, n, min) = match c {
        0xC2..=0xDF => (c & 0x1F, 1, 0x80),
        0xE0..=0xEF => (c & 0x0F, 2, 0x800),
        0xF0..=0xF4 => (c & 7, 3, 0x10000),
        _ => return (0xFFFD, i + 1),
    };
    let mut p = i + 1;
    for _ in 0..n {
        let b = at(p);
        if b & 0xC0 != 0x80 {
            return (0xFFFD, i + 1);
        }
        v = v << 6 | (b & 0x3F);
        p += 1;
    }
    if v < min || v > 0x10FFFF || v & 0xFFFFF800 == 0xD800 {
        return (0xFFFD, i + 1);
    }
    (v, p)
}

/// to_latin1(utf8): characters above U+00FF become '?'
pub(super) fn to_latin1(d: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(d.len());
    let mut i = 0;
    while i < d.len() {
        let (c, n) = utf8_decode(d, i);
        i = n;
        out.push(if c <= 0xFF { c as u8 } else { b'?' });
    }
    out
}

/// from_latin1: Latin-1 -> UTF-8
pub(super) fn from_latin1(d: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(d.len() * 2);
    for &b in d {
        if b < 0x80 {
            out.push(b);
        } else {
            out.push(0xC0 | b >> 6);
            out.push(0x80 | (b & 0x3F));
        }
    }
    out
}

/// own_has(cb_fmt, fmt): our own selection serves fmt
fn own_has(own: u32, fmt: u32) -> bool {
    if fmt == CLIP_TEXT {
        return own == CLIP_TEXT;
    }
    (own == CLIP_PNG || own == CLIP_IMAGE) && (fmt == CLIP_PNG || fmt == CLIP_IMAGE)
}

impl X11 {
    // ------------------------------------------------------------------ copy side

    /// x11_plat_clip_set(fmt, data) -> false on failure
    pub(super) fn clip_set(&mut self, fmt: u32, data: &[u8]) -> bool {
        if matches!(fmt, CLIP_HTML | CLIP_RTF | CLIP_DOCX) { return self.clip_set_many(&[(fmt, data)]); }
        let win0 = self.xw[0].win;
        if win0 == 0 {
            return false;
        }
        let (d, bmp) = match fmt {
            CLIP_TEXT | CLIP_PNG => (data.to_vec(), None),
            CLIP_IMAGE => match crate::clipimg_split(data) {
                Some((png, bmp)) if !png.is_empty() => (png.to_vec(), if bmp.is_empty() { None } else { Some(bmp.to_vec()) }),
                _ => return false,
            },
            _ => return false,
        };
        self.clip_drop();
        self.cb_data = Some(d);
        self.cb_bmp = bmp;
        self.cb_fmt = fmt;
        self.clip_claim()
    }

    // clipboard_platform: TEXT and rich representations share a single selection owner.
    pub(super) fn clip_set_many(&mut self, entries: &[(u32, &[u8])]) -> bool {
        if self.xw[0].win == 0 || !crate::clip_many_valid(entries) { return false; }
        if self.cb_rich_atoms.contains(&0) { return false; }
        let text = entries.iter().find(|e| e.0 == CLIP_TEXT).map(|e| e.1.to_vec());
        let rich = entries.iter().filter(|e| e.0 != CLIP_TEXT).map(|e| (e.0, e.1.to_vec())).collect();
        self.clip_drop();
        self.cb_fmt = if text.is_some() { CLIP_TEXT } else { 0 };
        self.cb_data = text;
        self.cb_rich = rich;
        self.clip_claim()
    }

    fn clip_claim(&mut self) -> bool {
        let win0 = self.xw[0].win;
        self.cb_time = self.time;
        let (sel, t) = (self.cb_sel, self.cb_time);
        let (o, _) = self.req(XR_SET_SELECTION_OWNER, 16);
        wr32(&mut self.out, o, win0);
        wr32(&mut self.out, o + 4, sel);
        wr32(&mut self.out, o + 8, t);
        let (o, seq) = self.req(XR_GET_SELECTION_OWNER, 8);
        wr32(&mut self.out, o, sel);
        match self.reply(seq) {
            Some(r) if rd32(&r, 8) == win0 => {
                self.cb_own = true;
                true
            }
            _ => {
                self.clip_drop();
                false
            }
        }
    }

    /// clip_drop: forget our selection data, end transfers to other clients
    fn clip_drop(&mut self) {
        self.cb_own = false;
        for k in 0..XF_MAX {
            if self.xfers[k].req != 0 {
                self.xfer_end(k);
            }
        }
        self.cb_data = None;
        self.cb_bmp = None;
        self.cb_rich.clear();
    }

    /// x11_clip_clear(ev): SelectionClear(time, owner, selection) -- someone else owns it
    pub(super) fn clip_clear(&mut self, b: &[u8]) {
        if rd32(b, 8) == self.xw[0].win && rd32(b, 12) == self.cb_sel {
            self.clip_drop();
        }
    }

    /// x11_clip_request(ev): SelectionRequest(time, owner, requestor, selection, target,
    /// property) -- convert our data, answer with SelectionNotify
    pub(super) fn clip_request(&mut self, b: &[u8]) {
        let xsel = self.xd_atoms[super::dnd::D_SELECTION];
        if xsel != 0 && rd32(b, 16) == xsel {
            self.dnd_request(b); // the content of our drag
            return;
        }
        let req = rd32(b, 12);
        let target = rd32(b, 20);
        let mut prop = rd32(b, 24);
        if prop == 0 {
            prop = target; // obsolete clients: property = target
        }
        let mut answered = 0u32;
        if self.cb_own && rd32(b, 16) == self.cb_sel {
            let a = self.atoms;
            if target == a[A_TARGETS] {
                let mut list = Vec::from([a[A_TARGETS], a[A_TIMESTAMP]]);
                if self.cb_fmt == CLIP_TEXT {
                    list.push(a[A_UTF8_STRING]);
                    list.extend_from_slice(&[a[A_TEXT_PLAIN_UTF8], a[A_TEXT_PLAIN], XA_STRING, a[A_TEXT]]);
                } else if matches!(self.cb_fmt, CLIP_PNG | CLIP_IMAGE) {
                    list.push(a[A_IMAGE_PNG]);
                    if self.cb_bmp.is_some() && self.xd_bmp_atom != 0 {
                        list.push(self.xd_bmp_atom);
                    }
                }
                for &(fmt, _) in &self.cb_rich {
                    list.push(self.cb_rich_atoms[(fmt - CLIP_HTML) as usize]);
                    if fmt == CLIP_HTML { list.push(self.cb_rich_atoms[3]); }
                    if fmt == CLIP_RTF { list.push(self.cb_rich_atoms[4]); }
                }
                self.change_prop32(req, prop, XA_ATOM, &list);
                answered = prop;
            } else if target == a[A_TIMESTAMP] {
                let t = self.cb_time;
                self.change_prop32(req, prop, XA_INTEGER, &[t]);
                answered = prop;
            } else {
                let mut send: Option<(u32, Vec<u8>)> = None;
                if self.cb_fmt == CLIP_TEXT {
                    let d = self.cb_data.clone().unwrap_or_default();
                    if target == a[A_UTF8_STRING] || target == a[A_TEXT] {
                        send = Some((a[A_UTF8_STRING], d)); // TEXT: answered as UTF8_STRING
                    } else if target == a[A_TEXT_PLAIN_UTF8] || target == a[A_TEXT_PLAIN] {
                        send = Some((target, d));
                    } else if target == XA_STRING {
                        send = Some((XA_STRING, to_latin1(&d))); // STRING: a Latin-1 copy
                    }
                } else if target == a[A_IMAGE_PNG] && matches!(self.cb_fmt, CLIP_PNG | CLIP_IMAGE) {
                    send = Some((target, self.cb_data.clone().unwrap_or_default()));
                } else if target != 0 && target == self.xd_bmp_atom {
                    if let Some(bmp) = self.cb_bmp.clone() {
                        send = Some((target, bmp)); // image/bmp of CLIP_IMAGE
                    }
                }
                if send.is_none() {
                    if let Some((_, data)) = self.cb_rich.iter().find(|(fmt, _)| self.cb_rich_atoms[(fmt - CLIP_HTML) as usize] == target
                        || (*fmt == CLIP_HTML && target == self.cb_rich_atoms[3])
                        || (*fmt == CLIP_RTF && target == self.cb_rich_atoms[4])) {
                        send = Some((target, data.clone()));
                    }
                }
                if let Some((typ, d)) = send {
                    if self.clip_send(req, prop, typ, d) {
                        answered = prop;
                    }
                }
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

    /// clip_send(requestor, property, type, data) -> false refused: store the data in the
    /// property, or start an INCR transfer for large data
    pub(super) fn clip_send(&mut self, req: u32, prop: u32, typ: u32, d: Vec<u8>) -> bool {
        if d.len() <= self.incr_limit() as usize {
            self.change_property(req, prop, typ, 8, 0, &d);
            return true;
        }
        let Some(k) = self.xfers.iter().position(|x| x.req == 0) else { return false }; // too many at once
        let len = d.len() as u32;
        self.xfers[k] = Xfer { req, prop, typ, data: d, off: 0 };
        self.select_events(req, XM_PROPERTY); // PropertyNotify of the requestor
        let incr = self.atoms[A_INCR];
        self.change_prop32(req, prop, incr, &[len]); // INCR: lower bound of the size
        true
    }

    /// incr_limit -> bytes above which data goes out with INCR: jfgui_X11INCR, else what
    /// fits into one ChangeProperty (maximum request and our request buffer)
    fn incr_limit(&mut self) -> u32 {
        if self.incr_limit != 0 {
            return self.incr_limit;
        }
        self.incr_limit = self.maxreq.min(X11_OUT_CAP as u32) - 64;
        if let Some(v) = crate::env("jfgui_X11INCR") {
            let n = parse_int(v.as_bytes()).0 as u32;
            if n >= INCR_MIN && n <= self.incr_limit {
                self.incr_limit = n;
                self.incr_chunk = n >> 1;
            }
        }
        self.incr_limit
    }

    /// incr_chunk -> bytes per INCR chunk (64 KB, or jfgui_X11INCR / 2)
    fn incr_chunk(&mut self) -> u32 {
        self.incr_limit();
        if self.incr_chunk != 0 { self.incr_chunk } else { INCR_CHUNK }
    }

    /// x11_clip_init: the selection atom -- CLIPBOARD, or jfgui_X11SELECTION (tests)
    pub(super) fn clip_init(&mut self) {
        self.cb_sel = self.atoms[A_CLIPBOARD];
        for (i, name) in [b"text/html".as_slice(), b"text/rtf", b"application/x-jfgui-docx-fragment", b"text/html;charset=utf-8", b"application/rtf"].iter().enumerate() {
            let seq = self.intern_send(name, false);
            self.cb_rich_atoms[i] = self.atom_reply(seq);
        }
        if let Some(n) = crate::env("jfgui_X11SELECTION") {
            if !n.is_empty() {
                let seq = self.intern_send(n.as_bytes(), false);
                if let Some(r) = self.reply(seq) {
                    self.cb_sel = rd32(&r, 8);
                }
            }
        }
    }

    /// select_events(window, mask): our event mask on another client's window
    fn select_events(&mut self, win: u32, mask: u32) {
        let (o, _) = self.req(XR_CHANGE_WINDOW_ATTR, 16);
        wr32(&mut self.out, o, win);
        wr32(&mut self.out, o + 4, XCW_EVENT_MASK);
        wr32(&mut self.out, o + 8, mask);
    }

    /// xfer_end(k): free a transfer record; stop watching the requestor unless another
    /// transfer goes there
    fn xfer_end(&mut self, k: usize) {
        let req = self.xfers[k].req;
        self.xfers[k].req = 0;
        self.xfers[k].data = Vec::new();
        if self.xfers.iter().any(|x| x.req == req) {
            return;
        }
        self.select_events(req, 0);
    }

    /// x11_clip_property(ev): PropertyNotify(window, atom, time, state) of our paste property
    /// (NewValue during an INCR receive) or of an INCR requestor (Deleted: send the next
    /// chunk; an empty chunk ends the transfer)
    pub(super) fn clip_property(&mut self, b: &[u8]) {
        let (win, atom, state) = (rd32(b, 4), rd32(b, 8), b[16]);
        if win == self.xw[0].win {
            if atom == self.atoms[A_JFGUI_SEL] && state == 0 {
                self.cb_newval = true;
            }
            return;
        }
        if state != 1 {
            return;
        }
        let Some(k) = self.xfers.iter().position(|x| x.req == win && x.prop == atom) else { return };
        let left = self.xfers[k].data.len() - self.xfers[k].off;
        let n = left.min(self.incr_chunk() as usize);
        let (req, prop, typ, off) = (self.xfers[k].req, self.xfers[k].prop, self.xfers[k].typ, self.xfers[k].off);
        let chunk = self.xfers[k].data[off..off + n].to_vec();
        self.change_property(req, prop, typ, 8, 0, &chunk);
        self.xfers[k].off += n;
        if n == 0 {
            self.xfer_end(k); // the empty chunk went out: done
        }
        self.flush();
    }

    // ------------------------------------------------------------------ paste side

    /// x11_clip_notify(ev): SelectionNotify(time, requestor, selection, target, property)
    pub(super) fn clip_notify(&mut self, b: &[u8]) {
        if rd32(b, 8) != self.xw[0].win {
            return;
        }
        if rd32(b, 12) != self.cv_sel {
            return; // not the selection sel_convert waits for
        }
        self.cb_prop = rd32(b, 20);
        self.cb_got = true;
    }

    /// x11_plat_clip_get(fmt) -> the data, None when there is none
    pub(super) fn clip_get(&mut self, fmt: u32) -> Option<Vec<u8>> {
        if self.cb_own {
            // CLIP_PNG / CLIP_IMAGE share the PNG
            return if own_has(self.cb_fmt, fmt) { self.cb_data.clone() }
                else { self.cb_rich.iter().find(|e| e.0 == fmt).map(|e| e.1.clone()) };
        }
        if self.xw[0].win == 0 {
            return None;
        }
        match fmt {
            CLIP_HTML | CLIP_RTF | CLIP_DOCX => {
                let atom = self.cb_rich_atoms[(fmt - CLIP_HTML) as usize];
                if atom == 0 { return None; }
                match self.clip_convert(atom) {
                    Ok(data) => Some(data),
                    Err(-2) => None,
                    Err(_) => {
                        let alternate = match fmt { CLIP_HTML => self.cb_rich_atoms[3], CLIP_RTF => self.cb_rich_atoms[4], _ => 0 };
                        if alternate == 0 { None } else { self.clip_convert(alternate).ok() }
                    }
                }
            }
            CLIP_IMAGE => self.get_image(),
            CLIP_PNG => self.clip_convert(self.atoms[A_IMAGE_PNG]).ok(),
            CLIP_TEXT => {
                match self.clip_convert(self.atoms[A_UTF8_STRING]) {
                    Ok(d) => return Some(d),
                    Err(-2) => return None, // timeout: do not wait again
                    Err(_) => {}
                }
                match self.clip_convert(self.atoms[A_TEXT_PLAIN_UTF8]) {
                    Ok(d) => return Some(d),
                    Err(-2) => return None,
                    Err(_) => {}
                }
                self.clip_convert(XA_STRING).ok().map(|d| from_latin1(&d)) // Latin-1 -> UTF-8
            }
            _ => None,
        }
    }

    /// clip_convert(target) -> the data; Err(-1) refused / Err(-2) timeout
    fn clip_convert(&mut self, target: u32) -> Result<Vec<u8>, i32> {
        let limit = if self.cb_rich_atoms.contains(&target) { crate::CLIP_RICH_MAX } else { CLIP_MAX };
        let (sel, t) = (self.cb_sel, self.time);
        self.sel_convert(sel, target, t, limit)
    }

    /// sel_convert(selection, target, time, limit) -> the data of any selection (CLIPBOARD,
    /// XdndSelection); Err(-1) refused / Err(-2) timeout
    pub(super) fn sel_convert(&mut self, sel: u32, target: u32, t: u32, limit: usize) -> Result<Vec<u8>, i32> {
        self.rb = Vec::new();
        self.cv_sel = sel;
        let (win0, prop) = (self.xw[0].win, self.atoms[A_JFGUI_SEL]);
        let (o, _) = self.req(XR_DELETE_PROPERTY, 12);
        wr32(&mut self.out, o, win0);
        wr32(&mut self.out, o + 4, prop);
        self.cb_got = false;
        self.cb_newval = false;
        let (o, _) = self.req(XR_CONVERT_SELECTION, 24);
        let b = &mut self.out;
        wr32(b, o, win0);
        wr32(b, o + 4, sel);
        wr32(b, o + 8, target);
        wr32(b, o + 12, prop);
        wr32(b, o + 16, t);
        if !self.clip_wait(|s| s.cb_got) {
            return self.convert_timeout();
        }
        if self.cb_prop == 0 {
            return Err(-1);
        }
        let typ = self.prop_read(limit);
        if typ == 0 || (typ == u32::MAX && limit == crate::CLIP_RICH_MAX) {
            self.rb = Vec::new();
            return Err(-1);
        }
        // (an error of the first read keeps what was read, as the assembly)
        if typ == self.atoms[A_INCR] {
            // INCR: the owner writes chunks after each delete; an empty one ends it
            self.rb.clear();
            loop {
                if !self.clip_wait(|s| s.cb_newval) {
                    return self.convert_timeout();
                }
                self.cb_newval = false;
                let old = self.rb.len();
                let t = self.prop_read(limit);
                if t == u32::MAX {
                    self.rb = Vec::new();
                    return Err(-1);
                }
                if old != self.rb.len() || t == 0 {
                    continue; // data came: wait for the next chunk (or the property is not there yet)
                }
                break;
            }
        }
        Ok(core::mem::take(&mut self.rb))
    }

    fn convert_timeout(&mut self) -> Result<Vec<u8>, i32> {
        if self.debug {
            eprint("x11: clipboard: no answer within 3 s\n");
        }
        self.rb = Vec::new();
        Err(-2)
    }

    /// prop_read -> property type (0 = none, u32::MAX = error): append the whole paste property
    /// to rb (reads delete it at the end)
    fn prop_read(&mut self, limit: usize) -> u32 {
        let mut off = 0u32; // offset in 4-byte units
        loop {
            let (win0, prop) = (self.xw[0].win, self.atoms[A_JFGUI_SEL]);
            let Some(r) = self.get_property(win0, prop, 0, off, X11_PROP_CHUNK, true) else { return u32::MAX };
            let typ = rd32(&r, 8);
            if typ == 0 {
                return 0; // no such property
            }
            let bytes = (rd32(&r, 16) as usize).wrapping_mul((r[1] >> 3) as usize) as u32 as usize;
            let after = rd32(&r, 12);
            if bytes == 0 && after != 0 {
                return u32::MAX; // no progress: the loop would never end
            }
            if self.rb.len() + bytes + usize::from(limit == CLIP_MAX) > limit {
                return u32::MAX; // other programs' data is bounded
            }
            self.rb.extend_from_slice(&r[32..32 + bytes.min(r.len() - 32)]);
            off = off.wrapping_add(bytes as u32 >> 2);
            if after == 0 {
                return typ;
            }
        }
    }

    /// clip_wait(flag) -> true when the flag became set, false after 3 s (clip_has: 300 ms);
    /// every other event is handled meanwhile
    fn clip_wait(&mut self, flag: fn(&X11) -> bool) -> bool {
        let now = crate::time_us();
        let deadline = now + if self.cb_wait != 0 { self.cb_wait } else { CLIP_TIMEOUT };
        loop {
            self.dispatch();
            if flag(self) {
                return true;
            }
            self.flush();
            let now = crate::time_us();
            if now >= deadline {
                return false;
            }
            let ms = ((deadline - now) / 1000) as i32 + 1;
            if self.poll(POLLIN, ms) & POLLIN != 0 {
                self.recv();
            }
        }
    }

    /// targets_mask(list) -> HB_* bits of an ATOM list
    fn targets_mask(&self, d: &[u8]) -> u32 {
        let mut m = 0;
        for c in d.chunks_exact(4) {
            let a = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            if a == 0 {
                continue;
            }
            if a == self.atoms[A_IMAGE_PNG] {
                m |= HB_PNG;
            } else if a == self.xd_bmp_atom {
                m |= HB_BMP;
            } else if a == self.atoms[A_UTF8_STRING]
                || a == self.atoms[A_TEXT_PLAIN_UTF8]
                || a == self.atoms[A_TEXT_PLAIN]
                || a == self.atoms[A_TEXT]
                || a == XA_STRING
            {
                m |= HB_TEXT;
            }
            for (i, &atom) in self.cb_rich_atoms.iter().enumerate() {
                if atom != 0 && a == atom { m |= match i { 3 => 8, 4 => 16, _ => 8 << i }; }
            }
        }
        m
    }

    /// get_image: CLIP_IMAGE of another client -- TARGETS, then image/png, else image/bmp
    /// (clip_bmp_fix)
    fn get_image(&mut self) -> Option<Vec<u8>> {
        let mut mask = HB_PNG | HB_BMP; // unknown: try both
        match self.clip_convert(self.atoms[A_TARGETS]) {
            Ok(list) => mask = self.targets_mask(&list),
            Err(-2) => return None, // the owner does not answer
            Err(_) => {}
        }
        if mask & HB_PNG != 0 {
            match self.clip_convert(self.atoms[A_IMAGE_PNG]) {
                Ok(d) => return Some(d),
                Err(-2) => return None,
                Err(_) => {}
            }
        }
        if mask & HB_BMP == 0 || self.xd_bmp_atom == 0 {
            return None;
        }
        let d = self.clip_convert(self.xd_bmp_atom).ok()?;
        crate::clip_bmp_fix(&d)
    }

    /// x11_plat_clip_has(fmt) -> the clipboard offers fmt
    pub(super) fn clip_has(&mut self, fmt: u32) -> bool {
        if self.cb_own {
            return own_has(self.cb_fmt, fmt) || self.cb_rich.iter().any(|e| e.0 == fmt);
        }
        if self.xw[0].win == 0 {
            return false;
        }
        let now = crate::time_us();
        if now.wrapping_sub(self.hs_time) >= HAS_POLL {
            self.hs_time = now;
            let sel = self.cb_sel;
            let (o, seq) = self.req(XR_GET_SELECTION_OWNER, 8);
            wr32(&mut self.out, o, sel);
            let owner = self.reply(seq).map_or(0, |r| rd32(&r, 8));
            if owner == 0 {
                self.hs_owner = 0;
                self.hs_mask = 0;
            } else if owner != self.hs_owner || now.wrapping_sub(self.hs_full) >= HAS_FRESH {
                self.hs_owner = owner;
                self.hs_full = now;
                self.hs_mask = 0;
                self.cb_wait = HAS_TIMEOUT;
                let r = self.clip_convert(self.atoms[A_TARGETS]);
                self.cb_wait = 0;
                match r {
                    Ok(list) => self.hs_mask = self.targets_mask(&list),
                    // a silent owner: ask again in 30 s (as the assembly: the unsigned age
                    // test above asks again at once while hs_full lies in the future)
                    Err(-2) => self.hs_full = self.hs_full.wrapping_add(30000000),
                    Err(_) => {}
                }
            }
        }
        let bits = match fmt {
            CLIP_TEXT => HB_TEXT,
            CLIP_PNG => HB_PNG,
            CLIP_IMAGE => HB_PNG | HB_BMP,
            CLIP_HTML | CLIP_RTF | CLIP_DOCX => 8 << (fmt - CLIP_HTML),
            _ => 0,
        };
        self.hs_mask & bits != 0
    }

}
