// basic -- the basic widgets                                           [src/ui/ui_basic.s]
//
//   text(style, s)                         TS_* | TX_* flags (TX_ELLIPSIS: cut with "…" at
//                                          the placed width, record value 1 when cut;
//                                          TX_WRAP: lines at the layout width, value = lines;
//                                          TX_HYPHENATE: words that do not fit hyphenated,
//                                          hyph.rs; soft hyphens always)
//   button(id, label) -> EV                standard button
//   accent_button(id, label) -> EV         accent (primary) button
//   icon_button(id, icon, label) -> EV     subtle toolbar button (label = tooltip / name;
//                                          ST_LARGE shows it under a big icon)
//   toggle_button(id, label, &mut v) -> EV checked look while v != 0; toggles v
//   tool_button(id, icon, label, &mut sel, mine) -> EV   selects sel = mine (tool palettes)
//   checkbox(id, label, &mut v) -> EV      v: 0, 1, 2 = indeterminate
//   radio(id, label, &mut v, mine) -> EV   v = mine when clicked
//   switch(id, label, &mut v) -> EV        toggle switch
//   slider(id, &mut v, &range) -> EV       EV_ACT while the value changes, EV_COMMIT on release
//   progress(id, v, max)                   progress bar
//   separator()                            divider line across the current box
//   icon(cp, size_dip)                     an icon in the text colour
//   image(id, &sf, ver) -> EV              a surface at its natural size (or next_size)
//   canvas(id, draw, user, version) -> &UiInput   custom area: draw.draw(g, rect) at render
//                                          time, clipped; bump `version` when its content
//                                          changes
//   hair() -> px                           border width for the current dpi
// Label strings are copied, so they may live in scratch buffers.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use super::cmd::DrawRef;
use super::consts::*;
use super::keys::*;
use super::{Sp, Ui};
use gfx::Surface;

const ELL_MAX: usize = 1024; // ellipsis buffer (bytes)
const WRAP_MAX: usize = 256; // lines of a TX_WRAP text

// a wrapped line: byte offset in the text, bytes drawn (without trailing blanks), width px
// (with the hyphen), hyphenated (a hyphen drawn after it)
#[derive(Clone, Copy)]
struct Wl {
    start: usize,
    len: usize,
    w: i32,
    hy: bool,
}

/// the text up to its first NUL (the asm's strings end there)
pub(crate) fn cstr(s: &str) -> &str {
    match s.find('\0') {
        Some(i) => &s[..i],
        None => s,
    }
}

// utf8_next(s, i) -> index of the next character (i itself at the end)
pub(crate) fn next_char(s: &[u8], i: usize) -> usize {
    if i >= s.len() || s[i] == 0 {
        return i;
    }
    let (_, n) = base::utf8_decode(s, i);
    n
}

impl Ui {
    /// ui_hair: max(1, (dpi + 24) / 96)
    pub fn hair(&self) -> i32 {
        (((self.wp.rt.dpi as u32).wrapping_add(24)) / 96).max(1) as i32
    }

    // the colour of a text style's TX_* flags (later flags win; disabled last)
    pub(crate) fn text_color(&self, style: u32) -> u32 {
        let mut t = T_TEXT;
        if style & TX_SECONDARY != 0 {
            t = T_TEXT2;
        }
        if style & TX_TERTIARY != 0 {
            t = T_TEXT3;
        }
        if style & TX_ACCENT != 0 {
            t = T_ACCENT_TEXT;
        }
        if style & TX_ON_ACCENT != 0 {
            t = T_ON_ACCENT;
        }
        if style & TX_DISABLED != 0 || self.wp.ctx.disabled_depth != 0 {
            t = T_TEXT_DISABLED;
        }
        self.color(t)
    }

    /// ui_focus_ring_draw(ev): the style's focus visual around last_rect when it is shown
    /// (ev: the widget's events; custom widgets built on hit_rect / behave call it last)
    pub fn focus_ring_draw(&mut self, ev: u32) {
        if self.focus_ring(ev) {
            let st = self.style;
            let r = self.wp.ctx.last_rect;
            st.focus_ring(self, r, 4); // corner radius of the controls (DIP)
        }
    }

    // sp_placed(sp, ev): the placed part of an SP record: last_rect, the events and the
    // state index (disabled when the last widget is SS_DISABLED)
    pub(crate) fn sp_placed(&self, sp: &mut Sp, ev: u32) {
        sp.place(self.wp.ctx.last_rect);
        sp.ev = ev;
        sp.state = if self.wp.ctx.last_state == SS_DISABLED as u32 {
            3
        } else if ev & EV_HOVER == 0 {
            0
        } else if ev & EV_HELD == 0 {
            1
        } else {
            2
        };
    }

    // ------------------------------------------------------------------ text

    /// ui_text(style, s): a text in a text style (TS_* | TX_*)
    pub fn text(&mut self, style: u32, s: &str) {
        let s = cstr(s);
        if style & TX_WRAP != 0 {
            self.text_wrap(style, s); // (U2) wrapped text: own function
            return;
        }
        let base = self.ts_base(style & 7); // (also selects the font)
        let (font, size) = self.text.get_font();
        let mut w = self.text.width(s);
        let line = self.ts_line(style & 7);
        let r = self.place(w, line);
        // (U1) ellipsis when the placed width is too small
        let mut cut = 0;
        let mut drawn: Option<String> = None;
        if style & TX_ELLIPSIS != 0 && w > r[2] {
            let (e, ew) = self.ellipsize(s.as_bytes(), r[2]);
            drawn = Some(e);
            w = ew;
            cut = 1;
        }
        let c = self.text_color(style);
        let mut x = r[0];
        let spare = r[2].wrapping_sub(w);
        if style & TX_CENTER != 0 {
            x = x.wrapping_add(spare >> 1);
        } else if style & TX_RIGHT != 0 {
            x = x.wrapping_add(spare);
        }
        let y = r[1].wrapping_add((r[3].wrapping_sub(line)) >> 1).wrapping_add(base);
        let t = drawn.as_deref().unwrap_or(s);
        self.cmd_text(x, y, t.as_bytes(), font, size, c, r[2]); // clipped to the placed width
        self.wp.ctx.last_state = 0;
        self.sem(K_TEXT, 0, Some(s), cut);
    }

    // text_wrap(style, s): text with TX_WRAP: broken into lines at the width the layout
    // gives it (avail_width): after the last blank that fits, inside a word that is wider
    // than the line, and at '\n'.  Natural height = lines x line height; natural width =
    // the unwrapped width where the layout stretches it, else the widest line.  TX_CENTER /
    // TX_RIGHT align each line; TX_ELLIPSIS cuts the text with "…" when a fixed height has
    // room for fewer lines.  Semantic record: the whole text, value = number of lines.
    fn text_wrap(&mut self, style: u32, s: &str) {
        let b = s.as_bytes();
        let hy = self.hyph_take(style & TX_HYPHENATE != 0); // (hyph.rs)
        let hy = hy.as_deref();
        let base = self.ts_base(style & 7); // (also selects the font)
        let (font, size) = self.text.get_font();
        let line = self.ts_line(style & 7);
        // unwrapped width: the widest paragraph
        let mut uw = 0;
        let mut p = 0;
        loop {
            let w = self.text.width(&s[p..]);
            if w > uw {
                uw = w;
            }
            match b[p..].iter().position(|&c| c == b'\n') {
                Some(i) => p += i + 1,
                None => break,
            }
        }
        // the width the layout will give us; <= 0 (unknown yet): no wrapping
        let (aw, fixed) = self.avail_width(uw);
        let mut ww = if aw > 0 { aw } else { 0x7FFF_FFFF };
        let mut lines = Vec::new();
        let mut widest = self.wrap_lines(b, ww, &mut lines, hy);
        // place: natural width = unwrapped (stretched / fixed) or the widest line
        let natw = if fixed != 0 { uw } else { widest };
        let r = self.place(natw, (lines.len() as i32).wrapping_mul(line));
        // placed at another width than the layout announced (should not happen): wrap again
        let pw = r[2];
        if fixed != 0 && pw > 0 && ww != 0x7FFF_FFFF && pw != ww {
            ww = pw;
            widest = self.wrap_lines(b, ww, &mut lines, hy);
        }
        let _ = widest;
        let c = self.text_color(style);
        // vertical position: centred when there is room, else from the top (TX_ELLIPSIS:
        // only the lines that fit, the last one cut with "…")
        let nlines = lines.len() as i32;
        let mut ndraw = nlines;
        let mut cut = false;
        let spare = r[3].wrapping_sub(nlines.wrapping_mul(line));
        let y0 = if spare >= 0 {
            r[1].wrapping_add(spare >> 1)
        } else {
            if style & TX_ELLIPSIS != 0 {
                let fit = if line != 0 { r[3] / line } else { 0 }.max(1);
                if fit < ndraw {
                    ndraw = fit;
                    cut = true;
                }
            }
            r[1]
        };
        for i in 0..ndraw {
            let l = lines[i as usize];
            let mut t: Vec<u8> = b[l.start..l.start + l.len].to_vec();
            if l.hy {
                t.push(b'-'); // hyphenated
            }
            let mut tw = l.w;
            // the last line drawn when lines are cut: the rest of its paragraph + "…"
            if cut && i + 1 == ndraw {
                let rest = &b[l.start..];
                let mut k = 0;
                while k < ELL_MAX - 8 && k < rest.len() && rest[k] != 0 && rest[k] != b'\n' {
                    k += 1;
                }
                // (a UTF-8 sequence cut at ELL_MAX - 8 is dropped by ellipsize's width test)
                let (e, ew) = self.ellipsize(&rest[..k], pw);
                t = e.into_bytes();
                tw = ew;
            }
            // x: aligned inside the placed width
            let mut x = r[0];
            let sp = pw.wrapping_sub(tw);
            if sp > 0 {
                if style & TX_CENTER != 0 {
                    x = x.wrapping_add(sp >> 1);
                } else if style & TX_RIGHT != 0 {
                    x = x.wrapping_add(sp);
                }
            }
            let y = i.wrapping_mul(line).wrapping_add(y0).wrapping_add(base);
            self.cmd_text(x, y, &t, font, size, c, pw); // clipped to the placed width
        }
        self.wp.ctx.last_state = 0;
        self.sem(K_TEXT, 0, Some(s), nlines);
    }

    // wrap_lines(s, width, hyph) -> widest line (px); the lines (at most WRAP_MAX; the rest of
    // a longer text is dropped).  Lines end at '\n' and where the text reaches `width`: after
    // the last blank that fits (the blanks stay on the line but are not measured or drawn),
    // after a soft hyphen, at a hyphenation point (hyph: TX_HYPHENATE) with a hyphen drawn,
    // inside a word that is wider than a line (at a character boundary, at least one
    // character per line).  Current font.
    fn wrap_lines(&mut self, s: &[u8], width: i32, out: &mut Vec<Wl>, hyph: Option<&spell::Hyphenator>) -> i32 {
        out.clear();
        let mut widest = 0;
        let mut pos = 0;
        'para: loop {
            // paragraph end: '\n' or the end
            let pe = pos + s[pos..].iter().position(|&c| c == b'\n' || c == 0).unwrap_or(s.len() - pos);
            loop {
                if out.len() >= WRAP_MAX {
                    break 'para;
                }
                let (end, hy) = self.wrap_break(s, pos, pe, width, hyph);
                // visible part: without the trailing blanks (ASCII and the ideographic space)
                let mut v = end;
                while v > pos && (s[v - 1] == b' ' || s[..v].ends_with("\u{3000}".as_bytes())) {
                    v -= if s[v - 1] == b' ' { 1 } else { 3 };
                }
                let w = self.text.width_n(&s[pos..], v - pos) + if hy { self.text.width("-") } else { 0 };
                out.push(Wl { start: pos, len: v - pos, w, hy });
                if w > widest {
                    widest = w;
                }
                pos = end;
                if pos >= pe {
                    break; // the paragraph is done
                }
            }
            if pe >= s.len() || s[pe] != b'\n' {
                break;
            }
            pos = pe + 1; // after the '\n'
        }
        widest
    }

    // wrap_break(s, pos, pe, width, hyph) -> (where the line starting at pos ends (pos < end
    // <= pe, or pe when pos == pe), hyphenated): the whole rest when it fits, else the last line
    // break opportunity (UAX #14: after spaces, after hyphens, after a soft hyphen, between
    // ideographs ...) whose text fits, a hyphenation point of the next word (hyph), else (one
    // long word) the grapheme clusters that fit (at least one)
    fn wrap_break(&mut self, s: &[u8], pos: usize, pe: usize, width: i32, hyph: Option<&spell::Hyphenator>) -> (usize, bool) {
        let hw = self.text.width("-");
        let t = &mut self.text;
        match core::str::from_utf8(s) {
            Ok(st) => {
                let mut fits = |a: usize, b: usize, h: bool| t.width_n(&s[a..], b - a) + if h { hw } else { 0 } <= width;
                match hyph {
                    Some(hy) => text::segment::wrap_end_hyph(st, pos, pe, &mut fits, Some(&mut |w: &str, out: &mut Vec<usize>| *out = hy.hyphenate(w))),
                    None => text::segment::wrap_end_hyph(st, pos, pe, &mut fits, None),
                }
            }
            Err(_) => {
                // not UTF-8: characters while they fit (at least one)
                let mut best = next_char(s, pos);
                while best < pe {
                    let n = next_char(s, best);
                    if t.width_n(&s[pos..], n - pos) > width {
                        break;
                    }
                    best = n;
                }
                (best.min(pe), false)
            }
        }
    }

    // ellipsize(s, maxw) -> (the longest prefix of s that fits into maxw px together with
    // "…" + "…", its width) (current font; trailing blanks dropped)
    pub(crate) fn ellipsize(&mut self, s: &[u8], maxw: i32) -> (String, i32) {
        let avail = maxw.wrapping_sub(self.text.width("…"));
        let mut n = 0;
        if avail > 0 {
            loop {
                if n >= s.len() || s[n] == 0 {
                    break;
                }
                let m = next_char(s, n); // length with the next character
                if m > ELL_MAX - 4 {
                    break;
                }
                if self.text.width_n(s, m) > avail {
                    break;
                }
                n = m;
            }
        }
        while n > 0 && s[n - 1] == b' ' {
            n -= 1;
        }
        let mut out = String::from_utf8_lossy(&s[..n]).into_owned();
        out.push('…'); // U+2026 HORIZONTAL ELLIPSIS
        let w = self.text.width(&out);
        (out, w)
    }

    // ------------------------------------------------------------------ buttons

    // btn_core(id, label, flags) -> EV: measure (style), place, behave, paint (style), focus
    // ring, semantics, tooltip of icon-only buttons
    fn btn_core(&mut self, id: u32, label: Option<&str>, flags: u32) -> u32 {
        let label = label.map(cstr);
        let flags = flags | self.take_flags();
        let icon = self.take_icon();
        let st = self.style;
        let mut sp = Sp { flags, icon, label, ..Sp::default() };
        let (w, h) = st.button_size(self, &mut sp);
        self.place(w, h);
        let mut hb = if flags & ST_NOFOCUS != 0 { 0 } else { HB_FOCUS };
        if flags & ST_DISABLED != 0 {
            hb |= HB_DISABLED;
        }
        if flags & ST_CHECKED != 0 {
            hb |= HB_CURRENT; // a focus group is entered at its checked button (kbnav.rs)
        }
        let ev = self.behave(id, hb);
        self.sp_placed(&mut sp, ev);
        st.button(self, &sp);
        self.focus_ring_draw(ev);
        if flags & ST_CHECKED != 0 {
            self.wp.ctx.last_state |= SS_CHECKED as u32;
        }
        self.sem(K_BUTTON, id, label, 0);
        // (popup agent) icon-only buttons show their label as a tooltip
        if flags & ST_ICON_ONLY != 0 {
            if let Some(l) = label {
                self.tooltip(l);
            }
        }
        ev
    }

    /// ui_button(id, label) -> EV_* (EV_ACT on click)
    pub fn button(&mut self, id: u32, label: &str) -> u32 {
        self.btn_core(id, Some(label), 0)
    }

    /// A focusable text action with explicit TS_*/TX_* typography and horizontal inset
    /// in DIP. Uses the style's button surface and ordinary button semantics. Long labels
    /// are ellipsized visually; accessibility and agent inspection retain the full label.
    pub fn label_button(&mut self, id: u32, label: &str, typography: u32, inset: i32) -> u32 {
        let label = cstr(label);
        let flags = self.take_flags();
        self.take_icon();
        let inset = self.dp(inset.max(0));
        self.ts_font(typography & 7);
        let width = self.text.width(label) + inset * 2;
        let line = self.ts_line(typography & 7);
        let r = self.place(width, line);
        let hb = if flags & ST_NOFOCUS != 0 { 0 } else { HB_FOCUS }
            | if flags & ST_DISABLED != 0 { HB_DISABLED } else { 0 };
        let ev = self.behave(id, hb);
        let mut sp = Sp { flags, ..Sp::default() };
        self.sp_placed(&mut sp, ev);
        self.style.button(self, &sp);
        let base = self.ts_base(typography & 7);
        let available = (r[2] - inset * 2).max(0);
        let width = self.text.width(label);
        let (display, width) = if width > available {
            self.ellipsize(label.as_bytes(), available)
        } else { (String::from(label), width) };
        let x = r[0] + inset + if typography & TX_CENTER != 0 {
            (available - width) / 2
        } else if typography & TX_RIGHT != 0 { available - width } else { 0 };
        let color = if flags & (ST_ACCENT | ST_ON_ACCENT) != 0 {
            self.color(if sp.state == 3 { T_ON_ACCENT_DIS } else { T_ON_ACCENT })
        } else if sp.state == 3 { self.color(T_TEXT_DISABLED) }
        else { self.text_color(typography) };
        let (font, size) = self.text.get_font();
        self.draw_clip(r[0], r[1], r[2], r[3]);
        self.cmd_text(x, r[1] + (r[3] - line) / 2 + base, display.as_bytes(), font, size, color, available);
        self.draw_unclip();
        self.focus_ring_draw(ev);
        if flags & ST_CHECKED != 0 {
            self.wp.ctx.last_state |= SS_CHECKED as u32;
        }
        self.sem(K_BUTTON, id, Some(label), 0);
        ev
    }

    /// A hyperlink-style text action: accent-coloured text (T_ACCENT_TEXT), underlined while
    /// the pointer is over it, hand cursor, focus ring, no button surface.  TS_*/TX_*
    /// typography as in text(); a narrower slot than the text ellipsizes it.
    pub fn link(&mut self, id: u32, label: &str, typography: u32) -> u32 {
        let label = cstr(label);
        let flags = self.take_flags();
        self.take_icon();
        self.ts_font(typography & 7);
        let line = self.ts_line(typography & 7);
        let natural = self.text.width(label);
        let r = self.place(natural, line);
        let hb = HB_FOCUS | CUR_HAND << HB_CURSOR_SHIFT | if flags & ST_DISABLED != 0 { HB_DISABLED } else { 0 };
        let ev = self.behave(id, hb);
        let (display, width) = if natural > r[2] {
            self.ellipsize(label.as_bytes(), r[2])
        } else { (String::from(label), natural) };
        let color = if flags & ST_DISABLED != 0 {
            self.color(T_TEXT_DISABLED)
        } else if ev & EV_HELD != 0 { self.color(T_TEXT2) } else { self.color(T_ACCENT_TEXT) };
        let base = self.ts_base(typography & 7);
        let y = r[1] + (r[3] - line) / 2;
        let (font, size) = self.text.get_font();
        self.draw_clip(r[0], r[1], r[2], r[3]);
        self.cmd_text(r[0], y + base, display.as_bytes(), font, size, color, r[2]);
        if ev & EV_HOVER != 0 && flags & ST_DISABLED == 0 {
            self.draw_color(color);
            self.draw_rect(r[0], y + base + self.dp(2), width, self.dp(1).max(1));
        }
        self.draw_unclip();
        self.focus_ring_draw(ev);
        self.sem(K_LINK, id, Some(label), 0);
        ev
    }

    /// ui_accent_button(id, label) -> EV_*: the accent (primary) button
    pub fn accent_button(&mut self, id: u32, label: &str) -> u32 {
        self.btn_core(id, Some(label), ST_ACCENT)
    }

    /// ui_icon_button(id, icon, label) -> EV_*: subtle toolbar button
    pub fn icon_button(&mut self, id: u32, icon: u32, label: &str) -> u32 {
        self.next_icon(icon);
        self.btn_icon(id, Some(label), ST_SUBTLE | ST_ICON_ONLY)
    }

    // btn_icon: ST_LARGE requested by the caller keeps the label visible
    fn btn_icon(&mut self, id: u32, label: Option<&str>, flags: u32) -> u32 {
        let f = self.take_flags();
        let mut flags = flags;
        if f & ST_LARGE != 0 {
            flags &= !ST_ICON_ONLY;
        }
        self.btn_core(id, label, flags | f)
    }

    /// ui_toggle_button(id, label, &mut v) -> EV_*: checked look while v != 0; toggles v
    pub fn toggle_button(&mut self, id: u32, label: &str, v: &mut i32) -> u32 {
        let ev = self.btn_core(id, Some(label), if *v != 0 { ST_CHECKED } else { 0 });
        if ev & EV_ACT != 0 {
            *v = (*v ^ 1) & 1;
            self.repass();
        }
        ev
    }

    /// ui_tool_button(id, icon, label, &mut sel, mine) -> EV_*: subtle icon button, checked
    /// when sel == mine; the tool buttons of one variable are one Tab stop (arrows inside)
    pub fn tool_button(&mut self, id: u32, icon: u32, label: &str, sel: &mut i32, mine: i32) -> u32 {
        self.kb_auto_group(&*sel as *const i32 as usize, mine);
        self.next_icon(icon);
        let mut f = ST_SUBTLE | ST_ICON_ONLY;
        if *sel == mine {
            f |= ST_CHECKED;
        }
        let ev = self.btn_icon(id, Some(label), f);
        if ev & EV_ACT != 0 {
            *sel = mine;
            self.repass();
        }
        ev
    }

    // ------------------------------------------------------------------ check box, radio, switch

    /// ui_checkbox(id, label, &mut v) -> EV_*: v 0, 1, 2 = indeterminate (Space toggles,
    /// Enter is the dialog's)
    pub fn checkbox(&mut self, id: u32, label: &str, v: &mut i32) -> u32 {
        let label = cstr(label);
        let f = self.take_flags();
        let st = self.style;
        let mut sp = Sp { flags: f, label: Some(label), ..Sp::default() };
        let (w, h) = st.checkbox_size(self, &mut sp);
        self.place(w, h);
        let hb = HB_FOCUS | HB_SPACE | if f & ST_DISABLED != 0 { HB_DISABLED } else { 0 };
        let mut ev = self.behave(id, hb);
        if f & ST_DISABLED == 0 && core::mem::take(&mut self.m.kbnav.toggle) {
            ev |= EV_ACT; // Space on a focused list: the selected row's check box
        }
        if ev & EV_ACT != 0 {
            *v = if *v == 1 { 0 } else { 1 };
            self.repass();
        }
        self.sp_placed(&mut sp, ev);
        sp.value = *v;
        st.checkbox(self, &sp);
        self.focus_ring_draw(ev);
        if *v != 0 {
            self.wp.ctx.last_state |= SS_CHECKED as u32;
        }
        self.sem(K_CHECKBOX, id, Some(label), *v);
        ev
    }

    /// ui_radio(id, label, &mut v, mine) -> EV_*: v = mine when clicked; the radios of one
    /// variable are one Tab stop, the arrows select inside (kbnav.rs)
    pub fn radio(&mut self, id: u32, label: &str, v: &mut i32, mine: i32) -> u32 {
        let label = cstr(label);
        let f = self.take_flags();
        let st = self.style;
        let mut sp = Sp { flags: f, label: Some(label), ..Sp::default() };
        let (w, h) = st.radio_size(self, &mut sp);
        self.place(w, h);
        let mut hb = HB_FOCUS | HB_SPACE | if f & ST_DISABLED != 0 { HB_DISABLED } else { 0 };
        if *v == mine {
            hb |= HB_CURRENT;
        }
        self.kb_auto_group(&*v as *const i32 as usize, mine);
        let mut ev = self.behave(id, hb);
        if ev & EV_FOCUS != 0 && self.kb_moved(id) && *v != mine {
            ev |= EV_ACT; // the selection follows the arrows
        }
        if ev & EV_ACT != 0 {
            *v = mine;
            self.repass();
        }
        self.sp_placed(&mut sp, ev);
        sp.value = (*v == mine) as i32;
        st.radio(self, &sp);
        self.focus_ring_draw(ev);
        if *v == mine {
            self.wp.ctx.last_state |= SS_CHECKED as u32;
        }
        self.sem(K_RADIO, id, Some(label), mine);
        ev
    }

    /// ui_switch(id, label, &mut v) -> EV_*: toggle switch
    pub fn switch(&mut self, id: u32, label: &str, v: &mut i32) -> u32 {
        let label = cstr(label);
        let f = self.take_flags();
        let st = self.style;
        let mut sp = Sp { flags: f, label: Some(label), ..Sp::default() };
        let (w, h) = st.switch_size(self, &mut sp);
        self.place(w, h);
        let hb = HB_FOCUS | HB_SPACE | if f & ST_DISABLED != 0 { HB_DISABLED } else { 0 };
        let mut ev = self.behave(id, hb);
        if f & ST_DISABLED == 0 && core::mem::take(&mut self.m.kbnav.toggle) {
            ev |= EV_ACT; // Space on a focused list: the selected row's switch
        }
        if ev & EV_ACT != 0 {
            *v = (*v ^ 1) & 1;
            self.repass();
        }
        // knob position animation 0 (off) .. 256 (on)
        let a = self.anim(id, if *v != 0 { 256 } else { 0 }, 167);
        self.sp_placed(&mut sp, ev);
        sp.value = *v;
        sp.value2 = a as i32;
        st.switch(self, &sp);
        self.focus_ring_draw(ev);
        if *v != 0 {
            self.wp.ctx.last_state |= SS_CHECKED as u32;
        }
        self.sem(K_SWITCH, id, Some(label), *v);
        ev
    }

    // ------------------------------------------------------------------ slider

    /// ui_slider(id, &mut v, &range) -> EV_*: EV_ACT while the value changes, EV_COMMIT on
    /// release
    pub fn slider(&mut self, id: u32, v: &mut i32, rg: &Range) -> u32 {
        let old = *v;
        let f = self.take_flags();
        let vert = (rg.flags & RG_VERT) as i32;
        let st = self.style;
        // measure (style): s[0] = thumb px
        let mut sp = Sp { flags: f, value: vert, ..Sp::default() };
        let (w, h) = st.slider_size(self, &mut sp);
        let thumb = sp.s[0];
        let r = self.place(w, h);
        let mut hb = HB_FOCUS | CUR_HAND << HB_CURSOR_SHIFT;
        if f & ST_DISABLED != 0 {
            hb |= HB_DISABLED;
        }
        let mut ev = self.behave(id, hb);
        // track length along the axis (minus the thumb)
        let len = (if vert != 0 { r[3] } else { r[2] }).wrapping_sub(thumb).max(1);
        let snap = |x: i32| {
            if rg.step <= 1 {
                return x;
            }
            (x.wrapping_sub(rg.min).wrapping_add(rg.step >> 1) / rg.step).wrapping_mul(rg.step).wrapping_add(rg.min)
        };
        if ev & EV_HELD != 0 {
            // pointer drag: value from the position (vertical: max at the top)
            let (mx, my) = (self.wp.ctx.mx, self.wp.ctx.my);
            let mut pos = if vert != 0 { r[1].wrapping_add(r[3]).wrapping_sub(my) } else { mx.wrapping_sub(r[0]) };
            pos = pos.wrapping_sub(thumb >> 1); // relative to the track start
            if pos < 0 {
                pos = 0;
            }
            if pos > len {
                pos = len;
            }
            *v = snap(rg.value_at(pos, len));
        } else if ev & EV_FOCUS != 0 {
            let step = if rg.step != 0 { rg.step } else { 1 };
            let a = self.key(VK_RIGHT);
            let b = self.key(VK_UP);
            if a || b {
                *v = v.wrapping_add(step);
                self.wp.ctx.focus_visible = true;
            }
            let a = self.key(VK_LEFT);
            let b = self.key(VK_DOWN);
            if a || b {
                *v = v.wrapping_sub(step);
                self.wp.ctx.focus_visible = true;
            }
            if self.key(VK_HOME) {
                *v = rg.min;
            }
            if self.key(VK_END) {
                *v = rg.max;
            }
        }
        // a value a screen reader set (a11y.rs)
        if let Some(x) = self.a11y_value(id) {
            *v = snap(x);
        }
        if *v < rg.min {
            *v = rg.min;
        }
        if *v > rg.max {
            *v = rg.max;
        }
        if *v != old {
            ev |= EV_ACT;
            self.repass();
        }
        // thumb offset along the track: (v - min) * len / (max - min)
        let t = rg.position(*v, len);
        self.sp_placed(&mut sp, ev);
        sp.value = t; // thumb offset along the track (px)
        sp.value2 = len; // track length (px)
        st.slider(self, &sp);
        self.focus_ring_draw(ev);
        let r = self.sem(K_SLIDER, id, None, *v);
        r.max = rg.max;
        r.min = rg.min;
        ev
    }

    // ------------------------------------------------------------------ progress, separator, icon, image

    /// ui_progress(id, v, max): progress bar
    pub fn progress(&mut self, id: u32, v: i32, max: i32) {
        let st = self.style;
        let mut sp = Sp::default();
        let (w, h) = st.progress_size(self, &mut sp);
        self.place(w, h);
        self.sp_placed(&mut sp, 0);
        sp.value = v;
        sp.value2 = max;
        st.progress(self, &sp);
        self.wp.ctx.last_state = 0;
        self.sem(K_PROGRESS, id, None, v).max = max;
    }

    /// ui_separator: divider across the current box (the style draws it)
    pub fn separator(&mut self) {
        let st = self.style;
        let mut sp = Sp::default();
        let t = st.separator_size(self, &mut sp); // thickness incl. margins
        let horizontal = self.box_flags() & BOX_V != 0;
        if horizontal {
            // in a vertical box: a horizontal line across the full width
            self.next_size(-1, 0);
            self.place(0, t);
        } else {
            self.next_size(0, -1);
            self.place(t, 0);
        }
        self.sp_placed(&mut sp, 0);
        sp.value = horizontal as i32; // 1 = horizontal line
        st.separator(self, &sp);
    }

    /// ui_icon(cp, size_dip): an icon in the text colour
    pub fn icon(&mut self, cp: u32, size: i32) {
        let s = self.dp(size);
        let r = self.place(s, s);
        let c = self.color(if self.wp.ctx.disabled_depth != 0 { T_TEXT_DISABLED } else { T_TEXT });
        self.draw_color(c);
        let x = (r[2].wrapping_sub(s) >> 1).wrapping_add(r[0]);
        let y = (r[3].wrapping_sub(s) >> 1).wrapping_add(r[1]);
        self.draw_icon(cp, x, y, s << 8);
    }

    /// ui_image(id, sf, ver) -> EV_* (clickable when id != 0): a surface at its natural size
    /// (or next_size); ver stands for its SF_AUX version
    pub fn image(&mut self, id: u32, sf: &Rc<Surface>, ver: u32) -> u32 {
        let r = self.place(sf.w, sf.h);
        self.draw_image(sf, ver, r[0], r[1], r[2], r[3]);
        if id == 0 {
            return 0;
        }
        self.behave(id, 0)
    }

    // ------------------------------------------------------------------ canvas

    /// ui_canvas(id, draw, user, version) -> &UiInput: a custom area.  The hit box wants
    /// every pointer move; the cursor comes from next_flags bits 12..15 (CUR_* <<
    /// HB_CURSOR_SHIFT), default crosshair.
    pub fn canvas(&mut self, id: u32, draw: Option<DrawRef>, user: usize, version: u32) -> &UiInput {
        let f = self.take_flags();
        let r = self.place(0, 0);
        let mut cur = f & (15 << HB_CURSOR_SHIFT);
        if cur == 0 {
            cur = CUR_CROSS << HB_CURSOR_SHIFT;
        }
        let ev = self.behave(id, cur | HB_MOVES);
        // custom draw command
        self.cmd_custom(r[0], r[1], r[2], r[3], draw, user, version);
        let key = self.rtl_text_key(self.key_first()); // right-to-left: arrows as drawn
        let (mx, px) = (self.rtl_map(self.wp.ctx.mx, r[0], r[2]), self.rtl_map(self.wp.ctx.press_x, r[0], r[2]));
        let c = &mut self.wp.ctx;
        let inp = &mut self.uiinput;
        inp.ev = ev;
        inp.btn = c.mbtn;
        // behave() clears active on release, including when the pointer is outside
        // the captured canvas. Keep that release visible to gesture controllers.
        let (p, rl) = if ev & (EV_HOVER | EV_COMMIT) != 0 || c.active == id { (c.pressed, c.released) } else { (0, 0) };
        inp.pressed = p;
        inp.released = rl;
        // a press on the canvas takes the keyboard focus (no focus ring), so keys reach the
        // canvas and not the widget focused before
        if p != 0 && ev & EV_HOVER != 0 {
            c.focus = id;
            c.focus_visible = false;
        }
        inp.x = mx.wrapping_sub(r[0]);
        inp.y = c.my.wrapping_sub(r[1]);
        inp.press_x = px.wrapping_sub(r[0]);
        inp.press_y = c.press_y.wrapping_sub(r[1]);
        let (wx, wy) = if ev & EV_HOVER != 0 { (c.wheel_x, c.wheel_y) } else { (0, 0) };
        inp.wheel_x = wx;
        inp.wheel_y = wy;
        inp.mods = self.mods;
        inp.rect = r;
        inp.dpi = self.wp.rt.dpi;
        inp.key = key;
        inp.pressure = self.mpress; // (PN1) pen state
        inp.ptype = self.mptype;
        inp.tilt_x = self.mtilt[0];
        inp.tilt_y = self.mtilt[1];
        // move points relative to the widget (only while it is hovered or active)
        let n = if ev & (EV_HOVER | EV_HELD) != 0 && self.uiw_cur == self.mpts_win { self.mpts.len().min(2048) } else { 0 };
        inp.pts.clear();
        inp.ppress.clear();
        let (rtl, mirror) = (self.rtl, r[0].wrapping_add(r[0]).wrapping_add(r[2]).wrapping_sub(1));
        for i in 0..n {
            let p = self.mpts[i];
            let x = if rtl { mirror.wrapping_sub(p[0]) } else { p[0] };
            inp.pts.push([x.wrapping_sub(r[0]), p[1].wrapping_sub(r[1])]);
            inp.ppress.push(self.mprs[i]);
        }
        self.dmg_canvas_seen(id); // (U1) rectangle for canvas_damage
        self.sem(K_CANVAS, id, None, 0);
        &self.uiinput
    }
}
