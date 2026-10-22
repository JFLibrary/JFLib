// edit -- text input: single-line and multi-line text boxes, number boxes, the clipboard for
// widgets                                                             [src/ui/ui_edit.s]
//
//   textbox(id, placeholder, &mut buf, cap_flags) -> EV
//        WinUI TextBox (32 DIP high).  buf: the app's UTF-8 text; cap_flags = the size of
//        the asm's buffer in bytes incl. the NUL (bits 0..22: the text holds at most cap - 1
//        bytes) | TB_PASSWORD | TB_READONLY | TB_DIGITS | TB_SIGNED.  placeholder (or None)
//        shows while the text is empty (cut with "…" where the box is narrower).
//        ST_DISABLED via next_flags.
//        EV_ACT: the text changed this frame; EV_COMMIT: Enter, Esc or focus lost; EV_FOCUS
//        while it has the keyboard focus.
//        Semantic record K_TEXTBOX: label = the text (one dot per character for passwords),
//        value = caret, max = selection anchor (byte indices; 0 while not focused), min = the
//        kind of box: (TB_MULTI | TB_PASSWORD | TB_READONLY) >> 24.
//        TB_MULTI (U1): several lines (natural height 5 lines): Enter inserts a line break,
//        Ctrl+Enter commits, Up / Down keep the preferred x, PgUp / PgDn move a view, Home /
//        End work on the line (Ctrl: the text), the wheel and the caret scroll it vertically,
//        a thin bar shows the position; TB_WRAP wraps at the width (else it scrolls
//        horizontally; soft hyphens: a hyphen where a line breaks after one), TB_HYPHENATE
//        with it hyphenates words that do not fit (hyph.rs; the hyphen is drawn, not part of
//        the text).  Pasted text keeps its lines.
//        TB_SPELL: spell checking -- misspelled words get a red wavy line, a right click on
//        one offers suggestions, "add to dictionary" and "ignore" (edit_spell.rs: spell_enable,
//        spell_language, spell_sources, spell_set, speller, spell_state, spell_errors).
//   number_box(id, &mut v, &range) -> EV
//        an integer in a text box: digits only ('-' too when the range minimum is negative);
//        Up / Down add -/+ step; typed text is taken on Enter or focus loss; the value is
//        clamped to the range.  EV_ACT when v changed, EV_COMMIT on Enter / focus loss.
//        Semantic value = v, max = range maximum.  PgUp / PgDn: 10 steps.
//   number_box_f(id, &mut v, &range_f) -> EV   (U1)
//        an f64 in a text box: shown with the range's decimals and decimal_sep; ',' and '.'
//        are both accepted; Up / Down = one step (step 0: 1 % of the range), PgUp / PgDn =
//        10 steps; clamped, snapped and rounded like the slider.  Semantic value =
//        v * 10^decimals.
//   (U2) both number boxes follow v while they have the focus as long as the user has not
//        edited their text; Enter / focus loss only store the text when it was edited.
//   nb_live(id, &mut v) / nb_live_f(id, &mut v) -> bool   the number typed so far into box
//        id (being edited) is stored when it differs from v (clamped / fixed to the range),
//        without reformatting the text -- property rows preview while the user types
//   open_folder(path) / open_url(url) -> 0 / -1   (U2) headless runs only print them
//   clip_set(fmt, data) -> 0 / -1, clip_get(fmt) -> data   the clipboard (headless runs:
//        an in-process clipboard, so scripts are deterministic)
//   text_key_filter(k) (crate)  the keys the focused text box consumes are hidden from key()
//
// Keys: Left/Right (+Ctrl words, +Shift select), Home/End, Backspace/Delete (+Ctrl words),
// Ctrl+A/C/X/V, Ctrl+Z undo, Ctrl+Y / Ctrl+Shift+Z redo, Shift+Delete cut, Ctrl+Insert copy,
// Shift+Insert paste, Enter commits, Esc drops the focus.  Plain character keys are consumed
// (their text arrives as typed text).  Tab, Up/Down/PgUp/PgDn, F-keys, Alt chords and other
// Ctrl chords stay for the app.
// Mouse: click places the caret (Shift+click extends), drag selects, double click selects a
// word.  The caret blinks every 530 ms (timer UIT_CARET; always visible in headless runs).
//
// Only one text box has the keyboard focus, so the edit state (caret, anchor, undo history)
// is one block that belongs to the focused box (ed_*); the horizontal / vertical scroll
// offsets are kept per box in its state slot.  The asm edits the app's NUL-terminated buffer
// in place; here the box's text is moved into S::buf for the call (byte for byte the asm's
// buffer up to its NUL) and handed back as a String.
//
// Deviation (Rust ownership): when another number box takes the edit buffer, the asm stores
// the previous owner's edited text into that owner's value right away (through the `&v` it
// kept); here that store waits for the owner's next number_box call (S::pend_text), which
// runs before the owner reads its value.  The events it reports and the settled frame are the
// same; an owner that is never drawn again keeps its old value.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::consts::*;
use crate::keys::*;
use crate::value::{fmt_f64_sep, parse_f64_sep, rf_fix, rf_scaled, RangeF};
use crate::{Sp, Ui};
use text::segment;
#[path = "text_surface.rs"]
mod surface;
pub use surface::TextSurfaceLine;
#[path = "edit_spell.rs"]
mod spellbox;

const TB_NAT_W: i32 = 200; // natural width of a text box (DIP)
const NB_NAT_W: i32 = 120; // ... of a number box
const TB_FLAG_MASK: u32 = 0xFF80_0000;
const ML_MAX: usize = 4096; // visual lines of a multi-line box (more: the rest is one line)
const PW_MAX: usize = 1024; // characters a password box shows
const UD_N: usize = 16; // undo states (power of two)
const UD_SEG: usize = 2048; // bytes per undo state (longer texts are not recorded)
const TXQ_MAX: usize = super::ctx::TEXTQ_MAX; // typed codepoints taken per pass (all of them)
const NB_CAP: u32 = 32; // number box text buffer

/// Opt-in presentation for a multiline editor embedded in document content. Geometry is in
/// physical pixels; callers scale it with the document zoom and DPI. Editing, clipboard,
/// selection, accessibility and IME behavior remain the ordinary textbox implementation.
#[derive(Clone, Copy)]
pub struct TextboxPresentation {
    /// Font identifier registered in the UI text engine.
    pub font: u32,
    /// Font size in 24.8 physical pixels, clamped to 1..512 pixels.
    pub size: i32,
    /// Line advance in physical pixels, clamped to 1..2048.
    pub line_height: i32,
    /// Baseline offset from a line's top, clamped to the line height.
    pub baseline: i32,
    /// Text and caret color, straight ARGB.
    pub color: u32,
    /// Solid editor background, straight ARGB.
    pub background: u32,
    /// Draw an underline beneath every text run.
    pub underline: bool,
}
// Ctrl+key shortcuts a text box takes: A C X V Z Y and the editing keys
const CTRL_KEYS: [u32; 13] = [65, 67, 88, 86, 90, 89, VK_LEFT, VK_RIGHT, VK_HOME, VK_END, VK_BACK, VK_DELETE, VK_INSERT];

/// state shared by all windows
pub(crate) struct S {
    // the headless text clipboard (g_hclip, g_hclip_len, g_hclip_fmt)
    pub(crate) hclip: Option<Vec<u8>>,
    pub(crate) hclip_fmt: u32,
    // clipboard_platform: alternative representations of the same selection.
    pub(crate) hclip_rich: Vec<(u32, Vec<u8>)>,
    // ---- the text box being built (g_tb_*)
    buf: Vec<u8>, // g_tb_buf: its text (moved in for the call)
    disp: Vec<u8>, // g_ed_disp: the dots of a password box (g_tb_disp = buf or disp)
    tb_id: u32,
    tb_max: usize, // capacity - 1
    tb_flags: u32,
    tb_chg: bool,
    tb_ev: u32,
    tb_natw: i32,
    tb_base: i32,
    tb_font: u32,
    tb_size: i32,
    tb_sem: Option<usize>, // its semantic record (None: the table was full)
    presentation: Option<TextboxPresentation>,
    surface: Option<Vec<TextSurfaceLine>>,
    surface_start: Option<usize>,
    surface_change: Option<(usize, usize, String)>,
    // ---- the focused editor (g_ed_*)
    ed_id: u32, // 0 = none
    ed_blur: u32, // box that lost the focus before it was drawn
    ed_caret: usize,
    ed_anchor: usize,
    ed_t0: u32, // blink start
    ed_merge: bool, // typing continues the last undo step
    ed_drag: bool, // drag selection running
    ed_seen: u32, // pass the editor was last drawn in
    ed_self: bool, // the text box reads keys itself
    ed_multi: bool, // (U1) the focused editor is a multi-line box
    ed_prefx: i32, // preferred x of Up / Down (-1 = none)
    // ---- undo history: logical entries 0..count-1 at ring position base
    ud_base: usize,
    ud_pos: usize,
    ud_count: usize,
    ud_hdr: [[usize; 3]; UD_N], // len, caret, anchor
    ud_data: [Vec<u8>; UD_N],
    // ---- number boxes
    nb_id: u32, // box owning nb_buf
    nb_seen: u32,
    nb_pend: u32,
    nb_pend_ev: u32,
    nb_kind: u32, // owner of nb_buf: 0 i32 (Range), 1 f64 (RangeF)
    nb_rg: Range, // the owner's range (g_nb_rg)
    nb_rf: RangeF,
    nb_shown: u64, // (U2) value (i32 / f64 bits) nb_orig shows
    nb_buf: Vec<u8>,
    nb_view: Vec<u8>,
    nb_orig: Vec<u8>, // (U2) text as formatted from the value (unedited when equal)
    pend_text: Option<(u32, u32, Vec<u8>)>, // (port) id, kind, text of a released owner
    // ---- the multi-line layout of the box being built (g_ml_*)
    ml_start: Vec<usize>, // start of every visual line
    ml_hyph: Vec<bool>, // the visual line ends hyphenated (a hyphen drawn after it)
    hyph: Option<alloc::sync::Arc<spell::Hyphenator>>, // TB_HYPHENATE: the box's patterns
    ml_w: i32, // wrap width (px)
    ml_lh: i32, // line height (px)
    ml_page: i32, // lines per view (PgUp / PgDn)
    // ---- spell checking (TB_SPELL, edit_spell.rs)
    spell: spellbox::S,
}

impl Default for S {
    fn default() -> S {
        S {
            hclip: None,
            hclip_fmt: 0,
            hclip_rich: Vec::new(),
            buf: Vec::new(),
            disp: Vec::new(),
            tb_id: 0,
            tb_max: 0,
            tb_flags: 0,
            tb_chg: false,
            tb_ev: 0,
            tb_natw: 0,
            tb_base: 0,
            tb_font: 0,
            tb_size: 0,
            tb_sem: None,
            presentation: None,
            surface: None,
            surface_start: None,
            surface_change: None,
            ed_id: 0,
            ed_blur: 0,
            ed_caret: 0,
            ed_anchor: 0,
            ed_t0: 0,
            ed_merge: false,
            ed_drag: false,
            ed_seen: 0,
            ed_self: false,
            ed_multi: false,
            ed_prefx: -1,
            ud_base: 0,
            ud_pos: 0,
            ud_count: 0,
            ud_hdr: [[0; 3]; UD_N],
            ud_data: core::array::from_fn(|_| Vec::new()),
            nb_id: 0,
            nb_seen: 0,
            nb_pend: 0,
            nb_pend_ev: 0,
            nb_kind: 0,
            nb_rg: Range::default(),
            nb_rf: RangeF::default(),
            nb_shown: 0,
            nb_buf: Vec::new(),
            nb_view: Vec::new(),
            nb_orig: Vec::new(),
            pend_text: None,
            ml_start: Vec::new(),
            ml_hyph: Vec::new(),
            hyph: None,
            ml_w: 0,
            ml_lh: 0,
            ml_page: 0,
            spell: Default::default(),
        }
    }
}

// the display text: the text, or a dot per character (g_tb_disp)
fn disp_of(e: &S) -> &[u8] {
    if e.tb_flags & TB_PASSWORD != 0 { &e.disp } else { &e.buf }
}

// the byte at i, 0 past the end (the asm's NUL)
fn at(s: &[u8], i: usize) -> u8 {
    if i < s.len() { s[i] } else { 0 }
}

// utf8_decode(s, i) -> (codepoint, next) of str.s: 1..4 byte sequences; invalid bytes,
// overlong forms, surrogates and values above U+10FFFF decode as U+FFFD and advance one
// byte; NUL (and the end) is 0
fn u8_decode(s: &[u8], i: usize) -> (u32, usize) {
    let b0 = at(s, i) as u32;
    if b0 < 0x80 {
        return (b0, i + 1);
    }
    let (mut cp, n, min) = if b0 < 0xC2 {
        return (0xFFFD, i + 1);
    } else if b0 < 0xE0 {
        (b0 & 0x1F, 1, 0x80)
    } else if b0 < 0xF0 {
        (b0 & 0x0F, 2, 0x800)
    } else if b0 < 0xF5 {
        (b0 & 7, 3, 0x10000)
    } else {
        return (0xFFFD, i + 1);
    };
    let mut j = i + 1;
    for _ in 0..n {
        let c = at(s, j) as u32;
        if c & 0xC0 != 0x80 {
            return (0xFFFD, i + 1);
        }
        cp = cp << 6 | (c & 0x3F);
        j += 1;
    }
    if cp < min || cp > 0x10FFFF || cp & 0xFFFF_F800 == 0xD800 {
        return (0xFFFD, i + 1);
    }
    (cp, j)
}

// utf8_next(s, i) -> start of the next character (i itself at the NUL / end)
fn u8_next(s: &[u8], i: usize) -> usize {
    if at(s, i) == 0 {
        return i;
    }
    u8_decode(s, i).1
}

// utf8_prev(s, i) -> start of the character before i (0 if i == 0)
fn u8_prev(s: &[u8], i: usize) -> usize {
    let mut p = i;
    if p == 0 {
        return 0;
    }
    loop {
        p -= 1;
        if p == 0 {
            return 0;
        }
        if s[p] & 0xC0 != 0x80 {
            return p;
        }
    }
}

// utf8_clip(s, n) -> largest length <= n that ends on a character boundary
fn u8_clip(s: &[u8], n: usize) -> usize {
    let mut k = n;
    while k != 0 && at(s, k) & 0xC0 == 0x80 {
        k -= 1;
    }
    k
}

// utf8_encode(cp) appended (1..4 bytes; above U+10FFFF: U+FFFD).  (port) A surrogate code
// point becomes U+FFFD too (the asm writes its 3-byte form, which a String cannot hold; the
// platforms never deliver one: Win32 pairs WM_CHAR halves, xkb gives scalar values)
fn u8_push(out: &mut Vec<u8>, cp: u32) {
    let c = if cp > 0x10FFFF || cp & 0xFFFF_F800 == 0xD800 { 0xFFFD } else { cp };
    if c < 0x80 {
        out.push(c as u8);
    } else if c < 0x800 {
        out.push(0xC0 | (c >> 6) as u8);
        out.push(0x80 | (c & 0x3F) as u8);
    } else if c < 0x10000 {
        out.push(0xE0 | (c >> 12) as u8);
        out.push(0x80 | ((c >> 6) & 0x3F) as u8);
        out.push(0x80 | (c & 0x3F) as u8);
    } else {
        out.push(0xF0 | (c >> 18) as u8);
        out.push(0x80 | ((c >> 12) & 0x3F) as u8);
        out.push(0x80 | ((c >> 6) & 0x3F) as u8);
        out.push(0x80 | (c & 0x3F) as u8);
    }
}

// the text up to its first NUL as a String (invalid sequences replaced; the boxes only
// ever insert whole characters)
fn to_string(mut b: Vec<u8>) -> String {
    if let Some(p) = b.iter().position(|&c| c == 0) {
        b.truncate(p);
    }
    match String::from_utf8(b) {
        Ok(s) => s,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    }
}

// tb_owns(entry, multi) -> a focused text box takes this key (entry = VK | mods << 16)
fn tb_owns(entry: u32, multi: bool) -> bool {
    let vk = entry & 0xFF;
    let m = entry >> 16;
    if vk == VK_TAB {
        return false;
    }
    if multi {
        // (U1) multi-line: vertical keys, Ctrl+Enter
        if vk == VK_UP || vk == VK_DOWN || vk.wrapping_sub(VK_PRIOR) <= VK_NEXT - VK_PRIOR {
            return m & (MOD_ALT | MOD_CTRL) == 0;
        }
        if vk == VK_RETURN {
            return m & MOD_ALT == 0;
        }
    }
    // single line: Up / Down / PgUp / PgDn stay for the app (the number box reads Up / Down
    // itself)
    if vk == VK_UP || vk == VK_DOWN || vk.wrapping_sub(VK_PRIOR) <= VK_NEXT - VK_PRIOR {
        return false;
    }
    if vk.wrapping_sub(VK_SHIFT) <= VK_CAPITAL - VK_SHIFT // Shift, Ctrl, Alt, Pause, Caps
        || vk.wrapping_sub(VK_LWIN) <= VK_APPS - VK_LWIN // Windows keys, menu key
        || vk.wrapping_sub(VK_F1) <= 23 // F1..F24
        || vk.wrapping_sub(VK_NUMLOCK) <= 1
        || vk.wrapping_sub(VK_LSHIFT) <= VK_RMENU - VK_LSHIFT
    {
        return false;
    }
    // AltGr (Ctrl+Alt) types characters
    if m & MOD_ALTGR != 0 || m & (MOD_CTRL | MOD_ALT) == MOD_CTRL | MOD_ALT {
        return true;
    }
    if m & MOD_ALT != 0 {
        return false;
    }
    if m & MOD_CTRL == 0 {
        return true;
    }
    // Ctrl: only the editing shortcuts
    CTRL_KEYS.contains(&vk)
}

// tb_accept(cp, flags) -> the box accepts the character
fn tb_accept(cp: u32, flags: u32) -> bool {
    if cp < 32 || cp == 127 {
        return false;
    }
    if flags & TB_DIGITS == 0 || cp.wrapping_sub(48) <= 9 {
        return true;
    }
    if flags & TB_DECIMAL != 0 && (cp == 44 || cp == 46) {
        return true; // (U1) decimal separators
    }
    cp == 45 && flags & TB_SIGNED != 0
}

// nb_format(value): decimal text of a number box
fn nb_format(v: i32) -> Vec<u8> {
    format!("{}", v).into_bytes()
}

// nb_parse(text, stored) -> the value typed (64-bit, saturated), or the stored value when
// there are no digits
fn nb_parse_text(t: &[u8], stored: i32) -> i64 {
    let mut i = 0;
    let neg = at(t, 0) == b'-';
    if neg {
        i = 1;
    }
    let mut v: i64 = 0;
    let mut digits = 0;
    while i < NB_CAP as usize {
        let d = at(t, i).wrapping_sub(48);
        if d > 9 {
            break;
        }
        digits += 1;
        v = v * 10 + d as i64;
        if v as u64 > 0x8000_0000 {
            v = 0x8000_0000;
        }
        i += 1;
    }
    if digits == 0 {
        return stored as i64;
    }
    if neg { -v } else { v }
}

// clamp a 64-bit value to a range (min first, then max)
fn rg_clamp(v: i64, rg: &Range) -> i32 {
    let mut c = v;
    if c < rg.min as i64 {
        c = rg.min as i64;
    }
    if c > rg.max as i64 {
        c = rg.max as i64;
    }
    c as i32
}

impl Ui {
    // ------------------------------------------------------------------ text box

    /// Read the focused textbox's UTF-8 byte selection (anchor, caret), after drawing it.
    /// Opt-in API for document editors; ordinary textbox behavior is unchanged.
    pub fn textbox_selection(&self, id: u32) -> Option<(usize, usize)> {
        if self.m.edit.ed_id == id && self.focused() == id {
            Some((self.m.edit.ed_anchor, self.m.edit.ed_caret))
        } else { None }
    }

    /// Select valid UTF-8 byte boundaries in an already focused/drawn textbox. Pass the
    /// same current text used by textbox(); returns false without changes for invalid ranges.
    pub fn textbox_select(&mut self, id: u32, text: &str, anchor: usize, caret: usize) -> bool {
        if self.m.edit.ed_id != id || self.focused() != id || !text.is_char_boundary(anchor) || !text.is_char_boundary(caret) {
            return false;
        }
        self.m.edit.ed_anchor = anchor;
        self.m.edit.ed_caret = caret;
        self.m.edit.ed_prefx = -1;
        self.tb_blink_reset();
        self.repass();
        true
    }

    /// ui_textbox(id, placeholder, buf, cap_flags) -> EV_* (see the module header)
    pub fn textbox(&mut self, id: u32, placeholder: Option<&str>, buf: &mut String, cap_flags: u32) -> u32 {
        self.textbox_icon(id, 0, placeholder, buf, cap_flags)
    }

    /// Single-line text input with a leading semantic icon. The icon reserves its own
    /// inset, so pointer selection, caret scrolling and IME share the visible text origin.
    /// Multiline inputs ignore the icon and retain their ordinary presentation.
    pub fn textbox_icon(&mut self, id: u32, icon: u32, placeholder: Option<&str>, buf: &mut String, cap_flags: u32) -> u32 {
        self.m.edit.tb_natw = TB_NAT_W;
        let mut b = core::mem::take(buf).into_bytes();
        let ev = self.tb_core(id, placeholder, &mut b, cap_flags, icon);
        *buf = to_string(b);
        ev
    }

    /// Edit multiline text on a caller-provided surface without the style's textbox frame.
    /// Uses zero text insets and the supplied font, line metrics and colors. The supplied
    /// presentation applies only to this call; ordinary textboxes keep their default style.
    /// `TB_MULTI` is implicit; other capacity and textbox flags retain their usual meaning.
    pub fn textbox_presented(&mut self, id: u32, placeholder: Option<&str>, buf: &mut String, cap_flags: u32, mut presentation: TextboxPresentation) -> u32 {
        presentation.size = presentation.size.clamp(256, 512 * 256);
        presentation.line_height = presentation.line_height.clamp(1, 2048);
        presentation.baseline = presentation.baseline.clamp(0, presentation.line_height);
        let previous = self.m.edit.presentation.replace(presentation);
        let old_font = self.text.get_font();
        let ev = self.textbox(id, placeholder, buf, cap_flags | TB_MULTI);
        self.m.edit.presentation = previous;
        self.text.set_font(old_font.0, old_font.1);
        ev
    }

    // tb_core(id, placeholder, buf, cap_flags) -> EV (natural width in tb_natw); the text
    // lives in S::buf for the call
    fn tb_core(&mut self, id: u32, placeholder: Option<&str>, buf: &mut Vec<u8>, cap_flags: u32, icon: u32) -> u32 {
        if let Some(p) = buf.iter().position(|&c| c == 0) {
            buf.truncate(p); // the asm's text ends at its NUL
        }
        core::mem::swap(buf, &mut self.m.edit.buf);
        // a text a screen reader set (a11y.rs): replaces the content, the caret at its end
        let set = self.a11y_text(id).map(|t| {
            let max = ((cap_flags & TB_CAP_MASK) as usize).saturating_sub(1);
            let mut n = if max > 0 { t.len().min(max) } else { t.len() };
            while !t.is_char_boundary(n) {
                n -= 1;
            }
            let e = &mut self.m.edit;
            e.buf = t.as_bytes()[..n].to_vec();
            if e.ed_id == id {
                e.ed_caret = n;
                e.ed_anchor = n;
            }
            self.repass();
        });
        let ph = placeholder.map(|p| p.split('\0').next().unwrap_or(""));
        let ev = if cap_flags & TB_MULTI != 0 {
            self.tbm_core(id, ph, cap_flags)
        } else {
            self.tb_core1(id, ph, cap_flags, icon)
        };
        core::mem::swap(buf, &mut self.m.edit.buf);
        if set.is_some() { ev | EV_ACT | EV_COMMIT } else { ev }
    }

    // tb_start(id, cap_flags, flag_mask) -> style flags: the per-call state of the box
    fn tb_start(&mut self, id: u32, cap_flags: u32, mask: u32) -> u32 {
        let e = &mut self.m.edit;
        e.tb_id = id;
        e.tb_max = ((cap_flags & TB_CAP_MASK) as usize).saturating_sub(1);
        e.tb_flags = cap_flags & mask;
        e.tb_chg = false;
        e.tb_ev = 0;
        self.take_flags()
    }

    // the single-line box (tb_core after the TB_MULTI test)
    fn tb_core1(&mut self, id: u32, placeholder: Option<&str>, cap_flags: u32, icon: u32) -> u32 {
        let stflags = self.tb_start(id, cap_flags, TB_FLAG_MASK);
        // place and behave (style: size and text insets)
        let st = self.style;
        let mut sp = Sp { value: self.m.edit.tb_natw, icon, flags: 0, ..Sp::default() };
        let (w, h) = st.textbox_metrics(self, &mut sp);
        if icon != 0 { sp.s[0] += self.dp(23); }
        self.place(w, h);
        let mut hb = HB_FOCUS | HB_TEXT | CUR_IBEAM << HB_CURSOR_SHIFT;
        if stflags & ST_DISABLED != 0 {
            hb |= HB_DISABLED;
        }
        let ev = self.behave(id, hb);
        let disabled = self.wp.ctx.last_state == SS_DISABLED as u32;
        let mut editing = self.tb_focus();
        if editing {
            self.m.edit.ed_multi = false; // (U1)
        }
        // geometry
        let hair = self.hair();
        let r = self.wp.ctx.last_rect;
        let x0 = sp.s[0].wrapping_add(r[0]);
        let tw = r[2].wrapping_sub(sp.s[0].wrapping_add(sp.s[1])).max(1);
        let mut scroll = self.slot(id).scrollx;
        st.textbox_font(self, &sp);
        self.tb_build_disp();
        // editing: pointer, keys, typed text (not while the box's spell menu is open)
        if editing && !self.spell_menu_open(id) {
            let x = self.rtl_map(self.wp.ctx.mx, x0, tw).wrapping_sub(x0).wrapping_add(scroll);
            self.tb_mouse(ev, x);
            self.tb_keys();
            self.tb_typed();
            #[cfg(target_os = "android")]
            self.tb_android_ime();
            if self.m.edit.tb_chg {
                self.m.edit.tb_ev |= EV_ACT;
                self.repass();
                self.tb_build_disp();
            }
            if self.m.edit.ed_id != id {
                editing = false; // Esc dropped the focus
                self.ime_off();
            }
        }
        // The same face is used for hit-testing, selection, caret and display.
        let (lh, base) = st.textbox_font(self, &sp);
        let top = (r[3].wrapping_sub(lh) >> 1).wrapping_add(r[1]);
        self.m.edit.tb_base = top.wrapping_add(base);
        let (font, size) = self.text.get_font();
        self.m.edit.tb_font = font;
        self.m.edit.tb_size = size;
        // scroll so that the caret stays visible
        let last = tw.wrapping_sub(hair); // last column the caret may use
        let mut s = scroll;
        let mut cx = 0;
        if editing {
            let d = self.tb_to_disp(self.m.edit.ed_caret);
            cx = self.text.caret_x(disp_of(&self.m.edit), d);
            if cx.wrapping_sub(s) > last {
                s = cx.wrapping_sub(last);
            }
            if cx < s {
                s = cx;
            }
        }
        // never further than needed to show the end of the text
        let dlen = disp_of(&self.m.edit).len();
        let ex = self.text.caret_x(disp_of(&self.m.edit), dlen).wrapping_sub(last);
        if s > ex {
            s = ex;
        }
        if s < 0 {
            s = 0;
        }
        scroll = s;
        self.slot(id).scrollx = s;
        // box (style): fill, border, focus edge
        sp.place(r);
        sp.ev = ev;
        sp.value = editing as i32;
        sp.value2 = disabled as i32;
        st.textbox_frame(self, &sp);
        if icon != 0 {
            let size = self.dp(16);
            self.draw_color(self.color(if disabled { T_TEXT_DISABLED } else { T_TEXT2 }));
            self.draw_icon(icon, r[0] + self.dp(8), r[1] + (r[3] - size) / 2, size << 8);
        }
        // text area (clipped to the padding box); right-to-left it keeps the text's order
        self.draw_clip(x0, r[1], tw, r[3]);
        self.ltr_begin(x0, tw);
        let bx = x0.wrapping_sub(scroll); // x of the first character
        let tcol = self.color(if disabled { T_TEXT_DISABLED } else { T_TEXT });
        if dlen == 0 {
            // empty: placeholder
            if let Some(p) = placeholder {
                let c = self.color(if disabled { T_TEXT_DISABLED } else { T_TEXT2 });
                self.tb_emit_hint(x0, p, c, tw);
            }
        } else {
            let (a, b) = self.tb_sel();
            if editing && a != b {
                // selection: highlight, then the text in three clipped runs
                let da = self.tb_to_disp(a);
                let sx0 = self.text.caret_x(disp_of(&self.m.edit), da);
                let db = self.tb_to_disp(b);
                let sx1 = self.text.caret_x(disp_of(&self.m.edit), db);
                let c = self.color(T_ACCENT);
                self.draw_color(c);
                self.draw_rect(bx.wrapping_add(sx0), top, sx1.wrapping_sub(sx0), lh);
                self.tb_clip_x(r[0], bx.wrapping_add(sx0)); // before the selection
                self.tb_emit_disp(bx, tcol);
                self.draw_unclip();
                self.tb_clip_x(bx.wrapping_add(sx0), bx.wrapping_add(sx1)); // inside: on the accent
                let c = self.color(T_ON_ACCENT);
                self.tb_emit_disp(bx, c);
                self.draw_unclip();
                self.tb_clip_x(bx.wrapping_add(sx1), r[0].wrapping_add(r[2])); // after
                self.tb_emit_disp(bx, tcol);
                self.draw_unclip();
            } else {
                self.tb_emit_disp(bx, tcol);
            }
        }
        // spell checking (TB_SPELL): marks under the misspelled words
        let spell = self.spell_scan(id);
        if spell && dlen != 0 {
            self.spell_draw(id, bx, 0, dlen, editing);
        }
        if editing {
            let x = bx.wrapping_add(cx);
            if self.tb_caret_on() {
                let c = self.color(T_TEXT);
                self.draw_color(c);
                self.draw_rect(x, top, hair, lh);
            }
            self.ime_pos(x, top, lh);
        }
        self.ltr_end();
        self.draw_unclip();
        if spell {
            let hit = (ev & EV_RCLICK != 0).then(|| {
                self.text.set_font(self.m.edit.tb_font, self.m.edit.tb_size);
                let x = self.rtl_map(self.wp.ctx.mx, x0, tw).wrapping_sub(x0).wrapping_add(scroll);
                self.tb_hit(x)
            });
            self.spell_input(id, ev, hit, editing);
        }
        self.tb_finish(ev, editing)
    }

    // the semantic record and the events of a text box (the end of tb_core / tbm_core)
    fn tb_finish(&mut self, ev: u32, editing: bool) -> u32 {
        #[cfg(target_os = "android")]
        if editing {
            let e = &self.m.edit;
            plat::android_ime_publish(e.tb_id, &e.buf, e.ed_anchor, e.ed_caret, e.tb_flags);
        }
        if !editing {
            self.wp.ctx.last_state &= !(SS_FOCUSED as u32);
        }
        let (v, mx) = if editing { (self.m.edit.ed_caret as i32, self.m.edit.ed_anchor as i32) } else { (0, 0) };
        let pw = self.m.edit.tb_flags & TB_PASSWORD != 0;
        let d = core::mem::take(if pw { &mut self.m.edit.disp } else { &mut self.m.edit.buf });
        let n0 = self.sems.len();
        {
            let l = String::from_utf8_lossy(&d);
            let id = self.m.edit.tb_id;
            let kind = self.m.edit.tb_flags & (TB_MULTI | TB_PASSWORD | TB_READONLY);
            let s = self.sem(K_TEXTBOX, id, Some(l.as_ref()), v);
            s.max = mx;
            s.min = (kind >> 24) as i32; // the kind of box for the accessibility tree (a11y.rs)
        }
        self.m.edit.tb_sem = if self.sems.len() > n0 { Some(n0) } else { None };
        *(if pw { &mut self.m.edit.disp } else { &mut self.m.edit.buf }) = d;
        // events: the widget's own EV_ACT / EV_COMMIT replace the click ones
        let mut out = ev & (EV_HOVER | EV_HELD | EV_DOUBLE | EV_RCLICK | EV_DRAG);
        if editing {
            out |= EV_FOCUS;
        }
        out | self.m.edit.tb_ev
    }

    // the semantic record of the box just built: value and max (number boxes)
    fn tb_sem_set(&mut self, value: i32, max: i32) {
        let r = match self.m.edit.tb_sem {
            Some(i) if i < self.sems.len() => &mut self.sems[i],
            _ => &mut self.sem_dummy,
        };
        r.value = value;
        r.max = max;
    }

    // ------------------------------------------------------------------ multi-line box (U1)

    // tbm_core(id, placeholder, cap_flags) -> EV: the TB_MULTI text box
    fn tbm_core(&mut self, id: u32, placeholder: Option<&str>, cap_flags: u32) -> u32 {
        let stflags = self.tb_start(id, cap_flags, TB_FLAG_MASK & !TB_PASSWORD);
        let hy = cap_flags & (TB_HYPHENATE | TB_WRAP) == TB_HYPHENATE | TB_WRAP;
        self.m.edit.hyph = self.hyph_take(hy); // (hyph.rs)
        // place: natural width tb_natw, height from the style (lines + insets)
        let presentation = self.m.edit.presentation;
        let lh = presentation.map_or_else(|| self.ts_line(TS_BODY), |p| p.line_height);
        let st = self.style;
        let mut sp = Sp { value: self.m.edit.tb_natw, value2: lh, flags: 1, ..Sp::default() };
        let (w, h) = if presentation.is_some() {
            (self.dp(self.m.edit.tb_natw), lh.saturating_mul(5))
        } else {
            st.textbox_metrics(self, &mut sp)
        };
        self.place(w, h);
        let mut hb = HB_FOCUS | HB_TEXT | CUR_IBEAM << HB_CURSOR_SHIFT;
        if stflags & ST_DISABLED != 0 {
            hb |= HB_DISABLED;
        }
        let ev = self.behave(id, hb);
        let disabled = self.wp.ctx.last_state == SS_DISABLED as u32;
        let mut editing = self.tb_focus();
        if editing {
            self.m.edit.ed_multi = true;
        }
        let hair = self.hair();
        let r = self.wp.ctx.last_rect;
        let x0 = sp.s[0].wrapping_add(r[0]);
        let tw = r[2].wrapping_sub(sp.s[0].wrapping_add(sp.s[1])).max(1);
        self.m.edit.ml_w = tw.wrapping_sub(hair); // wrap width (room for the caret)
        let top = r[1].wrapping_add(sp.s[2]);
        let vh = r[3].wrapping_sub(sp.s[2].wrapping_add(sp.s[2])).max(lh); // view height
        self.m.edit.ml_page = (if lh != 0 { vh / lh } else { 0 }).max(1);
        let boff = if let Some(p) = presentation {
            self.text.set_font(p.font, p.size);
            p.baseline
        } else {
            self.ts_base(TS_BODY) // also selects the body font
        };
        let (font, size) = self.text.get_font();
        self.m.edit.tb_font = font;
        self.m.edit.tb_size = size;
        let (mut sx, mut sy) = {
            let s = self.slot(id);
            (s.scrollx, s.scrolly)
        };
        self.tb_build_disp();
        self.tbm_layout();
        // editing: pointer, keys, typed text (not while the box's spell menu is open)
        if editing && !self.spell_menu_open(id) {
            let x = self.rtl_map(self.wp.ctx.mx, x0, tw).wrapping_sub(x0).wrapping_add(sx);
            let y = self.wp.ctx.my.wrapping_sub(top).wrapping_add(sy);
            self.tbm_mouse(ev, x, y, lh);
            self.m.edit.ml_lh = lh;
            self.tb_keys();
            self.tb_typed();
            #[cfg(target_os = "android")]
            self.tb_android_ime();
            if self.m.edit.tb_chg {
                self.m.edit.tb_ev |= EV_ACT;
                self.repass();
                self.tb_build_disp();
            }
            self.tbm_layout();
            if self.m.edit.ed_id != id {
                editing = false; // Esc dropped the focus
                self.ime_off();
            }
        }
        // content height; the wheel scrolls while there is room
        let ch = (self.m.edit.ml_start.len() as i32).wrapping_mul(lh);
        let clamp_y = |y: i32| -> i32 {
            let m = ch.wrapping_sub(vh).max(0);
            y.min(m).max(0)
        };
        let wy = self.wp.ctx.wheel_y;
        if ev & EV_HOVER != 0 && wy != 0 {
            let d = wy.wrapping_mul(lh.wrapping_mul(3)) / 120;
            let c = clamp_y(sy.wrapping_sub(d));
            if c != sy {
                sy = c;
                self.wp.ctx.wheel_y = 0;
            } // at the end: the page may scroll
        }
        // the caret line and x
        let mut cl = 0;
        let mut cx = 0;
        if editing {
            let caret = self.m.edit.ed_caret;
            cl = self.tbm_line_of(caret);
            let ls = self.m.edit.ml_start[cl];
            cx = self.text.caret_x(&self.m.edit.buf[ls..], caret - ls);
            // scroll the caret into view when it moved or the text changed
            let len = self.m.edit.buf.len() as i32;
            let s = self.slot(id);
            if caret as i32 != s.aux || len != s.fixed {
                s.aux = caret as i32;
                s.fixed = len;
                let ct = (cl as i32).wrapping_mul(lh); // caret line top (content px)
                if ct < sy {
                    sy = ct;
                }
                let cb = ct.wrapping_add(lh).wrapping_sub(vh);
                if cb > sy {
                    sy = cb;
                }
                if self.m.edit.tb_flags & TB_WRAP == 0 {
                    let last = tw.wrapping_sub(hair); // last column the caret may use
                    if cx.wrapping_sub(sx) > last {
                        sx = cx.wrapping_sub(last);
                    }
                    if cx < sx {
                        sx = cx;
                    }
                }
            }
        }
        sy = clamp_y(sy);
        if self.m.edit.tb_flags & TB_WRAP != 0 {
            sx = 0;
        }
        if sx < 0 {
            sx = 0;
        }
        {
            let s = self.slot(id);
            s.scrollx = sx;
            s.scrolly = sy;
        }
        // box (style): fill, border, focus edge
        sp.place(r);
        sp.ev = ev;
        sp.value = editing as i32;
        sp.value2 = disabled as i32;
        if let Some(p) = presentation {
            self.draw_color(p.background);
            self.draw_rect(r[0], r[1], r[2], r[3]);
        } else {
            st.textbox_frame(self, &sp);
        }
        // text area clip (inside the border)
        let inset = if presentation.is_some() { 0 } else { hair };
        self.draw_clip(x0, r[1].wrapping_add(inset), tw, r[3].wrapping_sub(inset).wrapping_sub(inset));
        self.ltr_begin(x0, tw); // right-to-left: the text keeps its order
        let tcol = presentation.map_or_else(|| self.color(if disabled { T_TEXT_DISABLED } else { T_TEXT }), |p| p.color);
        let scol = self.color(T_ON_ACCENT); // selected text colour
        let bx = x0.wrapping_sub(sx); // x of a line's first character
        let spell = self.spell_scan(id); // TB_SPELL
        if self.m.edit.buf.is_empty() {
            // empty: placeholder on the first line
            if let Some(p) = placeholder {
                let c = if presentation.is_some() { 0xff87918f } else { self.color(if disabled { T_TEXT_DISABLED } else { T_TEXT2 }) };
                self.m.edit.tb_base = top.wrapping_sub(sy).wrapping_add(boff);
                self.tb_emit_hint(x0, p, c, tw);
            }
        } else {
            // selection range (empty unless focused)
            let (sa, sb) = if editing { self.tb_sel() } else { (0, 0) };
            // visible lines
            let n = self.m.edit.ml_start.len();
            let first = if lh != 0 { sy / lh } else { 0 };
            let mut last = (if lh != 0 { sy.wrapping_add(vh) / lh } else { 0 }).wrapping_add(1);
            if last > n as i32 {
                last = n as i32;
            }
            let mut l = first;
            while l < last && (l as usize) < ML_MAX {
                let ls = self.m.edit.ml_start[l as usize];
                let le = self.tbm_line_end(l as usize);
                let hy = self.m.edit.ml_hyph.get(l as usize).copied().unwrap_or(false);
                let ly = l.wrapping_mul(lh).wrapping_add(top).wrapping_sub(sy); // line top
                self.m.edit.tb_base = ly.wrapping_add(boff);
                // selection part of this line: [max(a, s), min(b, e)] (+ the line break)
                let s0 = sa.max(ls);
                let s1 = sb.min(le);
                let sel = sa != sb && (s0 < s1 || (s0 == s1 && sb > le));
                if sel {
                    let x0r = self.text.caret_x(&self.m.edit.buf[ls..], s0 - ls);
                    let mut x1r = self.text.caret_x(&self.m.edit.buf[ls..], s1 - ls);
                    if sb > le {
                        x1r = x1r.wrapping_add(self.dp(4)); // the line break is selected
                    }
                    let c = self.color(T_ACCENT);
                    self.draw_color(c);
                    self.draw_rect(bx.wrapping_add(x0r), ly, x1r.wrapping_sub(x0r), lh);
                    // three clipped runs: before, inside (on the accent), after
                    self.tb_clip_x(r[0], bx.wrapping_add(x0r));
                    self.tbm_run(bx, ls, le, tcol, hy);
                    self.draw_unclip();
                    self.tb_clip_x(bx.wrapping_add(x0r), bx.wrapping_add(x1r));
                    self.tbm_run(bx, ls, le, scol, hy);
                    self.draw_unclip();
                    self.tb_clip_x(bx.wrapping_add(x1r), r[0].wrapping_add(r[2]));
                    self.tbm_run(bx, ls, le, tcol, hy);
                    self.draw_unclip();
                } else {
                    self.tbm_run(bx, ls, le, tcol, hy);
                }
                if spell {
                    self.spell_draw(id, bx, ls, le, editing);
                }
                l += 1;
            }
        }
        if editing {
            let ly = (cl as i32).wrapping_mul(lh).wrapping_add(top).wrapping_sub(sy);
            let x = bx.wrapping_add(cx);
            if self.tb_caret_on() {
                let c = presentation.map_or_else(|| self.color(T_TEXT), |p| p.color);
                self.draw_color(c);
                self.draw_rect(x, ly, hair, lh);
            }
            self.ime_pos(x, ly, lh);
        }
        self.ltr_end();
        self.draw_unclip();
        // scroll position bar (2 DIP) when the text is taller than the view
        if ch > vh {
            sp.place(r);
            sp.value = vh;
            sp.value2 = ch;
            sp.s[0] = sy;
            sp.s[1] = top;
            st.textbox_vbar(self, &sp);
        }
        if spell {
            let hit = (ev & EV_RCLICK != 0).then(|| {
                let x = self.rtl_map(self.wp.ctx.mx, x0, tw).wrapping_sub(x0).wrapping_add(sx);
                let y = self.wp.ctx.my.wrapping_sub(top).wrapping_add(sy);
                self.m.edit.ml_lh = lh;
                self.tbm_hit(x, y)
            });
            self.spell_input(id, ev, hit, editing);
        }
        self.tb_finish(ev, editing)
    }

    // .tm_run: the text of the line ls .. le at x, baseline tb_base (hy: and a hyphen)
    fn tbm_run(&mut self, x: i32, ls: usize, le: usize, c: u32, hy: bool) {
        let b = core::mem::take(&mut self.m.edit.buf);
        if hy {
            let mut t = b[ls..le].to_vec();
            t.push(b'-');
            self.tb_emit(x, &t, c);
        } else {
            self.tb_emit(x, &b[ls..le], c);
        }
        self.m.edit.buf = b;
    }

    // tbm_layout: the visual lines of the text (ml_start, ml_hyph); lines break at '\n' and,
    // with TB_WRAP, where the text reaches ml_w px (after the last blank that fits, after a
    // soft hyphen, at a hyphenation point with TB_HYPHENATE, else inside the word).  Uses the
    // current font.
    fn tbm_layout(&mut self) {
        if let Some(lines) = &self.m.edit.surface {
            self.m.edit.ml_start = lines.iter().map(|l| Self::tb_fix_index(&self.m.edit.buf, l.start)).collect();
            self.m.edit.ml_hyph.clear();
            return;
        }
        let len = self.m.edit.buf.len();
        let wrap = self.m.edit.tb_flags & TB_WRAP != 0;
        let mut starts = core::mem::take(&mut self.m.edit.ml_start);
        let mut hyph = core::mem::take(&mut self.m.edit.ml_hyph);
        starts.clear();
        hyph.clear();
        let mut pos = 0;
        'para: loop {
            // paragraph end: the next '\n' or the end
            let mut pe = pos;
            while pe < len && self.m.edit.buf[pe] != b'\n' {
                pe += 1;
            }
            loop {
                if starts.len() >= ML_MAX {
                    break 'para; // too many lines: the rest stays on the last
                }
                starts.push(pos);
                hyph.push(false);
                if !wrap {
                    break;
                }
                let (e, hy) = self.tbm_wrap(pos, pe);
                if e >= pe {
                    break;
                }
                if let Some(h) = hyph.last_mut() {
                    *h = hy;
                }
                pos = e; // soft break: next visual line
            }
            if pe >= len {
                break;
            }
            pos = pe + 1; // after the '\n'
        }
        self.m.edit.ml_start = starts;
        self.m.edit.ml_hyph = hyph;
    }

    // tbm_wrap(pos, pe) -> (where the visual line starting at pos ends (pos < end <= pe),
    // hyphenated): the last line break opportunity (UAX #14, a soft hyphen) whose text fits, a
    // hyphenation point (TB_HYPHENATE), else grapheme clusters
    fn tbm_wrap(&mut self, pos: usize, pe: usize) -> (usize, bool) {
        if self.tw_fits(pos, pe - pos) {
            return (pe, false);
        }
        let w = self.m.edit.ml_w;
        let hw = self.text.width("-");
        let hyph = self.m.edit.hyph.clone();
        let text = &mut self.text;
        match core::str::from_utf8(&self.m.edit.buf) {
            Ok(s) => {
                let mut fits = |a: usize, b: usize, h: bool| text.width_n(&s.as_bytes()[a..], b - a) + if h { hw } else { 0 } <= w;
                match hyph.as_deref() {
                    Some(hy) => segment::wrap_end_hyph(s, pos, pe, &mut fits, Some(&mut |wd: &str, out: &mut Vec<usize>| *out = hy.hyphenate(wd))),
                    None => segment::wrap_end_hyph(s, pos, pe, &mut fits, None),
                }
            }
            Err(_) => {
                // not UTF-8: code points while they fit (at least one)
                let buf = &self.m.edit.buf;
                let mut best = u8_next(buf, pos);
                while best < pe {
                    let n = u8_next(buf, best);
                    if text.width_n(&buf[pos..], n - pos) > w {
                        break;
                    }
                    best = n;
                }
                (best, false)
            }
        }
    }

    // .tw_fits(pos, n): n bytes from pos fit into ml_w
    fn tw_fits(&mut self, pos: usize, n: usize) -> bool {
        self.text.width_n(&self.m.edit.buf[pos..], n) <= self.m.edit.ml_w
    }

    // tbm_line_of(index) -> visual line that shows the index (a soft break index belongs
    // to the next line)
    fn tbm_line_of(&self, i: usize) -> usize {
        let st = &self.m.edit.ml_start;
        let mut l = st.len() as i32;
        loop {
            l -= 1;
            if l <= 0 {
                return 0;
            }
            if st[l as usize] <= i {
                return l as usize;
            }
        }
    }

    // tbm_line_end(line) -> index after the last character of the line (before its '\n')
    fn tbm_line_end(&self, line: usize) -> usize {
        if let Some(lines) = &self.m.edit.surface { return Self::tb_fix_index(&self.m.edit.buf, lines[line].end); }
        let e = &self.m.edit;
        if line + 1 >= e.ml_start.len() {
            return e.buf.len();
        }
        let s = e.ml_start[line + 1];
        if s != 0 && e.buf[s - 1] == b'\n' { s - 1 } else { s }
    }

    // tbm_hit(x, y) -> text index nearest to (x, y) px relative to the text's top-left
    // (content coordinates, scrolling included); line height ml_lh
    fn tbm_hit(&mut self, x: i32, y: i32) -> usize {
        let y = y.max(0);
        let lh = self.m.edit.ml_lh.max(1);
        let n = self.m.edit.ml_start.len() as i32;
        let l = (y / lh).min(n - 1).max(0) as usize;
        let ls = self.m.edit.ml_start[l];
        let le = self.tbm_line_end(l);
        let i = ls + self.text.hit(&self.m.edit.buf[ls..], Some(le - ls), x);
        self.tb_snap(i)
    }

    // tbm_mouse(events, x, y, line_h): click / drag / double click in content coordinates
    fn tbm_mouse(&mut self, ev: u32, x: i32, y: i32, lh: i32) {
        self.m.edit.ml_lh = lh;
        let c = &self.wp.ctx;
        if c.pressed & (1 << BTN_LEFT) != 0 && ev & EV_HOVER != 0 && c.active == self.m.edit.tb_id {
            self.m.edit.ed_prefx = -1;
            let i = self.tbm_hit(x, y);
            self.tb_press(ev, i);
        } else if ev & EV_HELD != 0 {
            if self.m.edit.ed_drag {
                let i = self.tbm_hit(x, y);
                self.tb_drag_to(i);
            }
        } else {
            self.m.edit.ed_drag = false;
        }
    }

    // tbm_vmove(lines, shift): the caret moves up / down by `lines` visual lines at the
    // preferred x (kept over several moves); at the first / last line it goes to its start /
    // end
    fn tbm_vmove(&mut self, lines: i32, shift: bool) {
        if self.m.edit.surface.is_some() { self.surface_vmove(lines, shift); return; }
        self.tbm_layout();
        let caret = self.m.edit.ed_caret;
        let cur = self.tbm_line_of(caret);
        // preferred x: kept, or the caret's x now
        let mut px = self.m.edit.ed_prefx;
        if px < 0 {
            let ls = self.m.edit.ml_start[cur];
            px = self.text.caret_x(&self.m.edit.buf[ls..], caret - ls);
        }
        let t = (cur as i32).wrapping_add(lines);
        let pos = if t < 0 {
            0 // above the first line: its start
        } else if t >= self.m.edit.ml_start.len() as i32 {
            self.m.edit.buf.len() // below the last line: the end
        } else {
            let ls = self.m.edit.ml_start[t as usize];
            let le = self.tbm_line_end(t as usize);
            let i = ls + self.text.hit(&self.m.edit.buf[ls..], Some(le - ls), px);
            self.tb_snap(i)
        };
        self.tb_move(pos, shift);
        self.m.edit.ed_prefx = px;
    }

    // ------------------------------------------------------------------ drawing helpers

    // tb_emit_hint(x, placeholder, colour, width): the placeholder, cut with "…" where it is
    // wider than the text area
    fn tb_emit_hint(&mut self, x: i32, p: &str, c: u32, w: i32) {
        let e = &self.m.edit;
        self.text.set_font(e.tb_font, e.tb_size);
        if w > 0 && self.text.width(p) > w {
            let (cut, _) = self.ellipsize(p.as_bytes(), w);
            self.tb_emit(x, cut.as_bytes(), c);
        } else {
            self.tb_emit(x, p.as_bytes(), c);
        }
    }

    // tb_emit(x, s, colour): a text run at the text baseline
    fn tb_emit(&mut self, x: i32, s: &[u8], c: u32) {
        let e = &self.m.edit;
        let (y, font, size) = (e.tb_base, e.tb_font, e.tb_size);
        self.cmd_text(x, y, s, font, size, c, 0);
        if self.m.edit.presentation.is_some_and(|p| p.underline) {
            self.text.set_font(font, size);
            let width = (self.text.measure(core::str::from_utf8(s).unwrap_or("")) + 128) / 256;
            self.draw_color(c);
            self.draw_rect(x, y + (size / 256 / 12).max(1), width, (size / 256 / 16).max(1));
        }
    }

    // tb_emit(x, disp, dlen, colour): the display text
    fn tb_emit_disp(&mut self, x: i32, c: u32) {
        let pw = self.m.edit.tb_flags & TB_PASSWORD != 0;
        let d = core::mem::take(if pw { &mut self.m.edit.disp } else { &mut self.m.edit.buf });
        self.tb_emit(x, &d, c);
        *(if pw { &mut self.m.edit.disp } else { &mut self.m.edit.buf }) = d;
    }

    // tb_clip_x(x0, x1): clip drawing to the columns x0..x1 of the placed box
    fn tb_clip_x(&mut self, x0: i32, x1: i32) {
        let r = self.wp.ctx.last_rect;
        self.draw_clip(x0, r[1], x1.wrapping_sub(x0), r[3]);
    }

    // tb_caret_on() -> the caret is drawn now (arms the blink timer)
    fn tb_caret_on(&mut self) -> bool {
        let (a, b) = self.tb_sel();
        if a != b {
            return false; // a selection hides the caret
        }
        if self.headless {
            return true; // headless: always visible
        }
        let blink = self.blink_ms; // the system's caret blink time (0: steady)
        if blink == 0 {
            return true;
        }
        let t = self.now_ms.wrapping_sub(self.m.edit.ed_t0);
        let phase = t / blink;
        self.timer(UIT_CARET, blink - t % blink);
        phase & 1 == 0
    }

    // ime_pos(x, y, h): IME windows follow the caret (Windows, window mode only); so do screen
    // magnifiers (the hidden system caret, the accessibility tree: a11y_caret)
    pub fn ime_pos(&mut self, x: i32, y: i32, h: i32) {
        let px = self.phys_x(x);
        self.a11y_caret(px, y, h);
        if !self.headless {
            plat::ime_pos(px, y, h);
        }
    }

    // ime_off: the text box lost the focus
    fn ime_off(&mut self) {
        #[cfg(target_os = "android")]
        plat::android_ime_clear();
        self.ime_pos(0, 0, 0);
    }

    // ------------------------------------------------------------------ focus and edit state

    // tb_focus() -> this box (tb_id) is the focused editor.  Loads the edit state when the
    // focus arrives, reports EV_COMMIT when it leaves.
    fn tb_focus(&mut self) -> bool {
        let id = self.m.edit.tb_id;
        if self.m.edit.ed_blur == id {
            self.m.edit.ed_blur = 0;
            self.m.edit.tb_ev |= EV_COMMIT;
        }
        // (multi-window) only the window with the keyboard edits; the others show their
        // focused text box unfocused
        if self.wp.ctx.focus == id && self.wp.ctx.last_state != SS_DISABLED as u32 && self.uiw_cur == self.m.win.active {
            let fv = self.wp.ctx.focus_visible;
            let e = &mut self.m.edit;
            if e.ed_id != id {
                // a box that still holds the edit state commits when it is drawn
                if e.ed_id != 0 {
                    e.ed_blur = e.ed_id;
                }
                e.ed_id = id;
                e.ed_caret = e.buf.len();
                e.ed_anchor = e.buf.len();
                if fv {
                    e.ed_anchor = 0; // keyboard focus selects everything
                }
                e.ed_drag = false;
                self.ud_reset();
                self.tb_blink_reset();
            }
            self.m.edit.ed_seen = self.frame;
            // the app may have changed the text: keep caret and anchor inside it
            let e = &mut self.m.edit;
            e.ed_caret = Self::tb_fix_index(&e.buf, e.ed_caret);
            e.ed_anchor = Self::tb_fix_index(&e.buf, e.ed_anchor);
            return true;
        }
        if self.m.edit.ed_id == id {
            self.m.edit.ed_id = 0;
            self.m.edit.tb_ev |= EV_COMMIT;
            self.ime_off();
        }
        false
    }

    // tb_fix_index(i) -> i clamped to the text and moved to a character start
    fn tb_fix_index(buf: &[u8], i: usize) -> usize {
        u8_clip(buf, i.min(buf.len()))
    }

    // tb_blink_reset: restart the blink cycle (caret visible now)
    fn tb_blink_reset(&mut self) {
        self.m.edit.ed_t0 = self.now_ms;
    }

    // tb_move(pos, shift): move the caret; without shift the selection collapses
    fn tb_move(&mut self, pos: usize, shift: bool) {
        let e = &mut self.m.edit;
        e.ed_caret = pos;
        if !shift {
            e.ed_anchor = pos;
        }
        e.ed_merge = false;
        self.tb_blink_reset();
    }

    // tb_sel() -> (selection start, end)
    fn tb_sel(&self) -> (usize, usize) {
        let e = &self.m.edit;
        if e.ed_caret <= e.ed_anchor { (e.ed_caret, e.ed_anchor) } else { (e.ed_anchor, e.ed_caret) }
    }

    // ------------------------------------------------------------------ pointer

    // a press on the box this frame at text index i (tb_mouse / tbm_mouse)
    fn tb_press(&mut self, ev: u32, i: usize) {
        self.m.edit.ed_drag = true;
        self.m.edit.ed_merge = false;
        self.tb_blink_reset();
        if ev & EV_DOUBLE != 0 {
            self.tb_word_at(i);
            self.m.edit.ed_drag = false;
            return;
        }
        self.m.edit.ed_caret = i;
        if self.mods & MOD_SHIFT == 0 {
            self.m.edit.ed_anchor = i; // Shift+click extends the selection
        }
    }

    // a drag to text index i
    fn tb_drag_to(&mut self, i: usize) {
        if i != self.m.edit.ed_caret {
            self.m.edit.ed_caret = i;
            self.tb_blink_reset();
        }
    }

    // tb_mouse(events, x): x = pointer relative to the first character (px)
    fn tb_mouse(&mut self, ev: u32, x: i32) {
        let c = &self.wp.ctx;
        if c.pressed & (1 << BTN_LEFT) != 0 && ev & EV_HOVER != 0 && c.active == self.m.edit.tb_id {
            let i = self.tb_hit(x);
            self.tb_press(ev, i);
        } else if ev & EV_HELD != 0 {
            if self.m.edit.ed_drag {
                let i = self.tb_hit(x);
                self.tb_drag_to(i);
            }
        } else {
            self.m.edit.ed_drag = false;
        }
    }

    // tb_hit(x) -> text index closest to x px (relative to the first character)
    fn tb_hit(&mut self, x: i32) -> usize {
        let e = &self.m.edit;
        let d = disp_of(e);
        let i = self.text.hit(d, Some(d.len()), x);
        let i = self.tb_from_disp(i);
        self.tb_snap(i)
    }

    // ------------------------------------------------------------------ display text

    // tb_build_disp: the display text is the text, or a dot per character
    fn tb_build_disp(&mut self) {
        let e = &mut self.m.edit;
        if e.tb_flags & TB_PASSWORD == 0 {
            return;
        }
        e.disp.clear();
        let mut i = 0;
        let mut n = 0;
        while at(&e.buf, i) != 0 && n < PW_MAX {
            e.disp.extend_from_slice(&[0xE2, 0x97, 0x8F]); // U+25CF BLACK CIRCLE
            n += 1;
            i = u8_next(&e.buf, i);
        }
    }

    // tb_to_disp(i) -> index in the display text of text index i
    fn tb_to_disp(&self, i: usize) -> usize {
        let e = &self.m.edit;
        if e.tb_flags & TB_PASSWORD == 0 {
            return i;
        }
        let mut p = 0;
        let mut n = 0;
        while p < i && at(&e.buf, p) != 0 && n < PW_MAX {
            p = u8_next(&e.buf, p);
            n += 1;
        }
        n * 3
    }

    // tb_from_disp(d) -> text index of display index d
    fn tb_from_disp(&self, d: usize) -> usize {
        let e = &self.m.edit;
        if e.tb_flags & TB_PASSWORD == 0 {
            return d;
        }
        let mut n = d / 3;
        let mut p = 0;
        while n != 0 && at(&e.buf, p) != 0 {
            p = u8_next(&e.buf, p);
            n -= 1;
        }
        p
    }

    // ------------------------------------------------------------------ characters and words
    // (Unicode segmentation: text::segment, UAX #29; text that is not UTF-8 by code points)

    // the text as str (None: not UTF-8)
    fn tb_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.m.edit.buf).ok()
    }

    // tb_prev(i) / tb_next(i) -> start of the grapheme cluster before / after index i
    fn tb_prev(&self, i: usize) -> usize {
        match self.tb_str() {
            Some(s) => segment::grapheme_prev(s, i),
            None => u8_prev(&self.m.edit.buf, i),
        }
    }

    fn tb_next(&self, i: usize) -> usize {
        match self.tb_str() {
            Some(s) => segment::grapheme_next(s, i),
            None => u8_next(&self.m.edit.buf, i),
        }
    }

    // tb_back(i) -> what Backspace removes before i: an emoji sequence whole, of other
    // clusters the last code point (an accent can be typed again)
    fn tb_back(&self, i: usize) -> usize {
        let p = self.tb_prev(i);
        match self.tb_str() {
            Some(s) if !segment::pictographic_cluster(s, p, i) => u8_prev(&self.m.edit.buf, i).max(p),
            _ => p,
        }
    }

    // tb_snap(i) -> i moved to the start of its grapheme cluster (hit tests)
    fn tb_snap(&self, i: usize) -> usize {
        match self.tb_str() {
            Some(s) if i < s.len() && !segment::is_grapheme_boundary(s, i) => segment::grapheme_prev(s, i),
            _ => i,
        }
    }

    // tb_word_left(i) -> start of the word before i (Ctrl+Left)
    fn tb_word_left(&self, i: usize) -> usize {
        if self.m.edit.tb_flags & TB_PASSWORD != 0 {
            return 0;
        }
        match self.tb_str() {
            Some(s) => segment::word_prev(s, i),
            None => 0,
        }
    }

    // tb_word_right(i) -> start of the next word after i (Ctrl+Right)
    fn tb_word_right(&self, i: usize) -> usize {
        let len = self.m.edit.buf.len();
        if self.m.edit.tb_flags & TB_PASSWORD != 0 || i >= len {
            return len;
        }
        match self.tb_str() {
            Some(s) => segment::word_next(s, i),
            None => len,
        }
    }

    // tb_word_at(i): select the word at index i (a run of blanks or of punctuation as one)
    fn tb_word_at(&mut self, i: usize) {
        let len = self.m.edit.buf.len();
        if len == 0 {
            self.m.edit.ed_anchor = 0;
            self.m.edit.ed_caret = 0;
            return;
        }
        if self.m.edit.tb_flags & TB_PASSWORD != 0 {
            self.m.edit.ed_anchor = 0;
            self.m.edit.ed_caret = len;
            return;
        }
        let (a, b) = match self.tb_str() {
            Some(s) => {
                let (mut a, mut b) = segment::word_at(s, i);
                let kind = segment::word_kind(s, a, b);
                if kind != segment::WK_WORD {
                    // neighbours of the same kind join (".." , "  "), not across a line
                    while a > 0 {
                        let (pa, pb) = segment::word_at(s, a - 1);
                        if segment::word_kind(s, pa, pb) != kind || s[pa..pb].contains('\n') {
                            break;
                        }
                        a = pa;
                    }
                    while b < s.len() {
                        let (na, nb) = segment::word_at(s, b);
                        if segment::word_kind(s, na, nb) != kind || s[na..nb].contains('\n') {
                            break;
                        }
                        b = nb;
                    }
                }
                (a, b)
            }
            None => (0, len),
        };
        self.m.edit.ed_anchor = a;
        self.m.edit.ed_caret = b;
    }

    // ------------------------------------------------------------------ editing primitives

    // tb_delete(a, b): remove the bytes a..b-1
    fn tb_delete(&mut self, a: usize, b: usize) {
        let e = &mut self.m.edit;
        let b = b.min(e.buf.len());
        if b <= a {
            return;
        }
        e.buf.drain(a..b);
        if e.surface.is_some() { e.surface_start = Some(e.surface_start.map_or(a, |v| v.min(a))); }
        e.tb_chg = true;
    }

    // tb_insert(pos, src) -> bytes inserted (cut at the capacity on a character boundary)
    fn tb_insert(&mut self, pos: usize, src: &[u8]) -> usize {
        let e = &mut self.m.edit;
        if e.tb_max <= e.buf.len() {
            return 0;
        }
        let room = e.tb_max - e.buf.len();
        let n = if src.len() > room { u8_clip(src, room) } else { src.len() };
        if n == 0 {
            return 0;
        }
        e.buf.splice(pos..pos, src[..n].iter().copied());
        if e.surface.is_some() { e.surface_start = Some(e.surface_start.map_or(pos, |v| v.min(pos))); }
        e.tb_chg = true;
        n
    }

    // tb_replace(src): replace the selection with src; caret after it
    fn tb_replace(&mut self, src: &[u8]) {
        let (a, b) = self.tb_sel();
        // Structured text callers must receive the entire replacement or no edit. A
        // truncated paste can otherwise delete a selection before the model can reject it.
        if self.m.edit.surface.is_some()
            && self.m.edit.buf.len().saturating_sub(b - a).saturating_add(src.len()) > self.m.edit.tb_max
        {
            return;
        }
        if a != b {
            self.tb_delete(a, b);
        }
        let mut p = a;
        self.m.edit.ed_caret = p;
        self.m.edit.ed_anchor = p;
        if src.is_empty() {
            return;
        }
        p += self.tb_insert(p, src);
        self.m.edit.ed_caret = p;
        self.m.edit.ed_anchor = p;
    }

    // tb_typed: insert this frame's typed text at the caret
    fn tb_typed(&mut self) {
        let cps = self.text_take(TXQ_MAX);
        let flags = self.m.edit.tb_flags;
        if flags & TB_READONLY != 0 {
            return;
        }
        let mut stage = Vec::new();
        for &c in cps.iter() {
            if tb_accept(c, flags) {
                u8_push(&mut stage, c);
            }
        }
        if stage.is_empty() {
            return;
        }
        if !self.m.edit.ed_merge {
            self.ud_push(); // typing continues one undo step
        }
        self.tb_replace(&stage);
        self.m.edit.ed_merge = true;
        self.tb_blink_reset();
    }

    // ------------------------------------------------------------------ keys

    // Android sends each composing revision immediately, with selection, through the
    // same capacity/filter/undo rules as normal typing. No wait for a word to commit.
    #[cfg(target_os = "android")]
    fn tb_android_ime(&mut self) {
        let Some((bytes, anchor, caret)) = plat::android_ime_take(self.m.edit.tb_id) else { return; };
        if self.m.edit.tb_flags & TB_READONLY != 0 { return; }
        let mut filtered = Vec::new();
        let mut positions = [0; 2];
        for (i, c) in core::str::from_utf8(&bytes).unwrap_or("").char_indices() {
            if tb_accept(c as u32, self.m.edit.tb_flags) { u8_push(&mut filtered, c as u32); }
            if i < anchor { positions[0] = filtered.len(); }
            if i < caret { positions[1] = filtered.len(); }
        }
        if filtered != self.m.edit.buf {
            if !self.m.edit.ed_merge { self.ud_push(); }
            self.m.edit.ed_anchor = 0;
            self.m.edit.ed_caret = self.m.edit.buf.len();
            self.tb_replace(&filtered);
            self.m.edit.ed_merge = true;
        }
        self.m.edit.ed_anchor = Self::tb_fix_index(&self.m.edit.buf, positions[0]);
        self.m.edit.ed_caret = Self::tb_fix_index(&self.m.edit.buf, positions[1]);
        self.m.edit.ed_prefx = -1;
        self.tb_blink_reset();
        self.repass();
    }

    /// ui_text_key_filter(k) -> k, or None when the focused text box consumes that key (so
    /// app-level key() / key_peek() do not see it); k = index of a key queue entry
    pub(crate) fn text_key_filter(&mut self, k: Option<usize>) -> Option<usize> {
        let i = k?;
        let e = &self.m.edit;
        if e.ed_self {
            return k; // the text box itself asks
        }
        if e.ed_id == 0 || e.ed_id != self.wp.ctx.focus {
            return k;
        }
        if self.frame.wrapping_sub(e.ed_seen) > 2 {
            return k; // the box is no longer drawn
        }
        if self.spell_menu_open(e.ed_id) {
            return k; // its spell menu takes the keys (edit_spell.rs)
        }
        let q = self.wp.ctx.keyq[i];
        if tb_owns(q, e.ed_multi) && !self.tb_esc_passes(q) { None } else { k }
    }

    // tb_esc_passes(k): Esc is not the text box's while it holds the focus a dialog window gave
    // its first control (kbnav.rs): Esc closes the dialog at once, as on every system
    fn tb_esc_passes(&self, k: u32) -> bool {
        k & 0xFFFF == VK_ESCAPE && self.wp.ctx.kb.soft != 0 && self.wp.ctx.kb.soft == self.wp.ctx.focus
    }

    // tb_keys: apply this frame's editing keys (stops at the first key it does not own)
    fn tb_keys(&mut self) {
        self.m.edit.ed_self = true;
        loop {
            if self.m.edit.ed_id != self.m.edit.tb_id {
                break;
            }
            let k = self.key_first();
            if k == 0 || !tb_owns(k, self.m.edit.ed_multi) || self.tb_esc_passes(k) {
                break;
            }
            self.key(k & 0xFFFF); // consume it
            let k = self.rtl_text_key(k); // right-to-left: arrows as the text reads
            let vk = k & 0xFFFF;
            let m = k >> 16;
            let shift = m & MOD_SHIFT != 0;
            let ctrl = m & (MOD_ALT | MOD_ALTGR) == 0 && m & MOD_CTRL != 0; // not AltGr
            let flags = self.m.edit.tb_flags;
            if flags & TB_MULTI != 0 {
                // (U1) line keys of a multi-line box
                let page = self.m.edit.ml_page;
                match vk {
                    VK_UP => {
                        self.tbm_vmove(-1, shift);
                        continue;
                    }
                    VK_DOWN => {
                        self.tbm_vmove(1, shift);
                        continue;
                    }
                    VK_PRIOR => {
                        self.tbm_vmove(page.wrapping_neg(), shift);
                        continue;
                    }
                    VK_NEXT => {
                        self.tbm_vmove(page, shift);
                        continue;
                    }
                    VK_RETURN => {
                        if ctrl {
                            self.m.edit.tb_ev |= EV_COMMIT; // Ctrl+Enter commits
                        } else if flags & TB_READONLY == 0 {
                            self.m.edit.ed_prefx = -1;
                            self.ud_push();
                            self.tb_replace(b"\n");
                            self.tb_blink_reset();
                        }
                        continue;
                    }
                    VK_HOME if !ctrl => {
                        self.m.edit.ed_prefx = -1;
                        self.tbm_layout();
                        let l = self.tbm_line_of(self.m.edit.ed_caret);
                        let p = self.m.edit.ml_start[l];
                        self.tb_move(p, shift);
                        continue;
                    }
                    VK_END if !ctrl => {
                        self.m.edit.ed_prefx = -1;
                        self.tbm_layout();
                        let l = self.tbm_line_of(self.m.edit.ed_caret);
                        let p = self.tbm_line_end(l);
                        self.tb_move(p, shift);
                        continue;
                    }
                    _ => {} // Ctrl+Home / End: the whole text
                }
            }
            self.m.edit.ed_prefx = -1;
            let len = self.m.edit.buf.len();
            let caret = self.m.edit.ed_caret;
            match vk {
                VK_LEFT => {
                    let (a, b) = self.tb_sel();
                    let p = if ctrl {
                        self.tb_word_left(caret)
                    } else if !shift && a != b {
                        a // collapse to the selection start
                    } else {
                        self.tb_prev(caret)
                    };
                    self.tb_move(p, shift);
                }
                VK_RIGHT => {
                    let (a, b) = self.tb_sel();
                    let p = if ctrl {
                        self.tb_word_right(caret)
                    } else if !shift && a != b {
                        b // collapse to the selection end
                    } else {
                        self.tb_next(caret)
                    };
                    self.tb_move(p, shift);
                }
                VK_HOME => self.tb_move(0, shift),
                VK_END => self.tb_move(len, shift),
                VK_BACK => {
                    if flags & TB_READONLY != 0 {
                        continue;
                    }
                    let (a, b) = self.tb_sel();
                    if a != b {
                        self.tb_delrange(a, b);
                    } else {
                        let s = if ctrl { self.tb_word_left(a) } else { self.tb_back(a) };
                        self.tb_delrange(s, a);
                    }
                }
                VK_DELETE => {
                    if shift && !ctrl {
                        self.tb_cut(); // Shift+Delete = cut
                        continue;
                    }
                    if flags & TB_READONLY != 0 {
                        continue;
                    }
                    let (a, b) = self.tb_sel();
                    if a != b {
                        self.tb_delrange(a, b);
                    } else {
                        let e = if ctrl { self.tb_word_right(a) } else { self.tb_next(a) };
                        self.tb_delrange(a, e);
                    }
                }
                VK_INSERT => {
                    if ctrl {
                        self.tb_copy(); // Ctrl+Insert = copy
                    } else if shift && flags & TB_READONLY == 0 {
                        self.tb_paste(); // Shift+Insert = paste
                    }
                }
                VK_RETURN => self.m.edit.tb_ev |= EV_COMMIT,
                VK_ESCAPE => {
                    self.wp.ctx.focus = 0;
                    self.m.edit.ed_id = 0;
                    self.m.edit.tb_ev |= EV_COMMIT;
                    self.repass();
                    break;
                }
                _ if !ctrl => {} // character keys: the text comes as typed text
                65 => {
                    // Ctrl+A
                    let e = &mut self.m.edit;
                    e.ed_anchor = 0;
                    e.ed_caret = len;
                    e.ed_merge = false;
                }
                67 => self.tb_copy(),
                88 => self.tb_cut(),
                86 => {
                    if flags & TB_READONLY == 0 {
                        self.tb_paste();
                    }
                }
                90 | 89 => {
                    // Ctrl+Z (Ctrl+Shift+Z = redo), Ctrl+Y
                    if flags & TB_READONLY == 0 {
                        if vk == 90 && !shift {
                            self.ud_undo();
                        } else {
                            self.ud_redo();
                        }
                    }
                }
                _ => {}
            }
        }
        self.m.edit.ed_self = false;
    }

    // .tk_delrange(a, b): a..b is removed, the caret goes to a
    fn tb_delrange(&mut self, a: usize, b: usize) {
        if a >= b {
            return;
        }
        self.ud_push();
        self.tb_delete(a, b);
        self.tb_move(a, false);
    }

    // .tk_cut: copy and delete the selection (not for read-only / password boxes)
    fn tb_cut(&mut self) {
        if self.m.edit.tb_flags & (TB_READONLY | TB_PASSWORD) != 0 {
            return;
        }
        self.tb_copy();
        let (a, b) = self.tb_sel();
        if a != b {
            self.tb_delrange(a, b);
        }
    }

    // ------------------------------------------------------------------ clipboard operations

    // tb_copy: the selection to the clipboard (not for passwords)
    fn tb_copy(&mut self) {
        if self.m.edit.tb_flags & TB_PASSWORD != 0 {
            return;
        }
        let (a, b) = self.tb_sel();
        if a == b {
            return;
        }
        let s = self.m.edit.buf[a..b].to_vec();
        self.clip_set(CLIP_TEXT, &s);
    }

    // tb_paste: the first line of the clipboard text (filtered) replaces the selection
    // (multi-line boxes: every line)
    fn tb_paste(&mut self) {
        let Some(clip) = self.clip_get(CLIP_TEXT) else { return };
        let flags = self.m.edit.tb_flags;
        let cap = clip.len() * 3; // re-encoded: at most 3 bytes per input byte
        let mut out = Vec::with_capacity(cap + 4);
        let mut i = 0;
        loop {
            let (c, n) = u8_decode(&clip, i);
            if c == 0 {
                break;
            }
            i = n;
            if c == 10 {
                if flags & TB_MULTI == 0 || out.len() >= cap {
                    break;
                }
                out.push(10); // (U1) multi-line: keep the break
                continue;
            }
            if c == 13 {
                if flags & TB_MULTI != 0 {
                    if self.m.edit.surface.is_some() && clip.get(i) != Some(&10) {
                        out.push(10); // Rich document paste: a lone CR is a paragraph mark.
                    }
                    continue; // CR of CR LF: dropped
                }
                break;
            }
            if tb_accept(c, flags) {
                u8_push(&mut out, c);
            }
        }
        if out.is_empty() {
            return;
        }
        self.ud_push();
        self.tb_replace(&out);
        self.m.edit.ed_merge = false;
        self.tb_blink_reset();
    }

    /// Publish distinct TEXT/HTML/RTF/DOCX representations of one selection (64 MiB total).
    /// Validation precedes replacement; 0 means every representation was published.
    pub fn clip_set_many(&mut self, entries: &[(u32, &[u8])]) -> i32 {
        if !plat::clip_many_valid(entries) { return -1; }
        if !self.headless { return if plat::clip_set_many(entries) { 0 } else { -1 }; }
        let rich: Vec<_> = entries.iter().filter(|e| e.0 != CLIP_TEXT).map(|e| (e.0, e.1.to_vec())).collect();
        let text = entries.iter().find(|e| e.0 == CLIP_TEXT).map(|e| e.1.to_vec());
        self.hclip_img_clear();
        self.m.edit.hclip = text;
        self.m.edit.hclip_fmt = CLIP_TEXT;
        self.m.edit.hclip_rich = rich;
        0
    }

    /// Replace the clipboard with one text, image or rich representation.
    pub fn clip_set(&mut self, fmt: u32, data: &[u8]) -> i32 {
        if matches!(fmt, CLIP_HTML | CLIP_RTF | CLIP_DOCX) { return self.clip_set_many(&[(fmt, data)]); }
        if !self.headless {
            return if plat::clip_set(fmt, data) { 0 } else { -1 };
        }
        if fmt != CLIP_TEXT {
            return self.hclip_img_set(fmt, data); // K1: images go to the image store
        }
        self.hclip_img_clear(); // K1: one content at a time
        self.m.edit.hclip_rich.clear();
        self.m.edit.hclip = Some(data.to_vec());
        self.m.edit.hclip_fmt = fmt;
        0
    }

    /// ui_clip_get(fmt) -> the clipboard data (text: UTF-8) or None
    pub fn clip_get(&mut self, fmt: u32) -> Option<Vec<u8>> {
        if !self.headless {
            return plat::clip_get(fmt);
        }
        if fmt != CLIP_TEXT {
            if matches!(fmt, CLIP_HTML | CLIP_RTF | CLIP_DOCX) {
                return self.m.edit.hclip_rich.iter().find(|e| e.0 == fmt).map(|e| e.1.clone());
            }
            return self.hclip_img_get(fmt); // K1: images come from the image store
        }
        match &self.m.edit.hclip {
            Some(d) if self.m.edit.hclip_fmt == fmt => Some(d.clone()),
            _ => None,
        }
    }

    /// ui_open_folder(path) -> 0 / -1: (U2) show a folder (or a file selected in its folder)
    /// in the system file manager; headless runs start nothing: they print
    /// "ui_open_folder: PATH" and return 0
    pub fn open_folder(&mut self, path: &str) -> i32 {
        if !self.headless {
            return plat::open_folder(path);
        }
        plat::write_stdout(format!("ui_open_folder: {}\n", path).as_bytes());
        0
    }

    /// ui_open_url(url) -> 0 / -1: a web page / mailto link in the default application;
    /// headless runs print "ui_open_url: URL" and return 0
    pub fn open_url(&mut self, url: &str) -> i32 {
        if !self.headless {
            return plat::open_url(url);
        }
        plat::write_stdout(format!("ui_open_url: {}\n", url).as_bytes());
        0
    }

    /// hclip_text_has: the headless clipboard holds text
    pub(crate) fn hclip_text_has(&self) -> bool {
        self.m.edit.hclip.is_some()
    }

    /// hclip_text_clear: empty the headless text clipboard (an image was set)
    pub(crate) fn hclip_text_clear(&mut self) {
        self.m.edit.hclip = None;
        self.m.edit.hclip_rich.clear();
    }

    // ------------------------------------------------------------------ undo history
    // Logical entries 0..count-1 (ring position ud_base); entries before `pos` are older
    // states, entry `pos` (when it exists) is the current one (after an undo).

    fn ud_reset(&mut self) {
        let e = &mut self.m.edit;
        e.ud_base = 0;
        e.ud_pos = 0;
        e.ud_count = 0;
        e.ed_merge = false;
    }

    // ud_slot(i) -> ring index of logical entry i
    fn ud_slot(&self, i: usize) -> usize {
        (self.m.edit.ud_base + i) & (UD_N - 1)
    }

    // ud_room: make room for one more entry (the oldest is dropped when full)
    fn ud_room(&mut self) {
        let e = &mut self.m.edit;
        if e.ud_count < UD_N {
            return;
        }
        e.ud_base = (e.ud_base + 1) & (UD_N - 1);
        e.ud_count -= 1;
        if e.ud_pos != 0 {
            e.ud_pos -= 1;
        }
    }

    // ud_store(i): snapshot the text, caret and anchor into logical entry i
    fn ud_store(&mut self, i: usize) {
        let k = self.ud_slot(i);
        let e = &mut self.m.edit;
        e.ud_hdr[k] = [e.buf.len(), e.ed_caret, e.ed_anchor];
        e.ud_data[k].clear();
        e.ud_data[k].extend_from_slice(&e.buf);
    }

    // ud_restore(i): make logical entry i the current text
    fn ud_restore(&mut self, i: usize) {
        let k = self.ud_slot(i);
        let e = &mut self.m.edit;
        let [len, caret, anchor] = e.ud_hdr[k];
        let n = len.min(e.tb_max);
        e.buf.clear();
        e.buf.extend_from_slice(&e.ud_data[k][..n]);
        if e.surface.is_some() { e.surface_start = Some(0); }
        e.tb_chg = true;
        e.ed_merge = false;
        e.ed_anchor = Self::tb_fix_index(&e.buf, anchor);
        e.ed_caret = Self::tb_fix_index(&e.buf, caret);
        self.tb_blink_reset();
    }

    // ud_push: remember the state before an edit (drops the redo states)
    fn ud_push(&mut self) {
        self.m.edit.ed_merge = false;
        if self.m.edit.buf.len() >= UD_SEG {
            self.ud_reset(); // too long to record
            return;
        }
        self.m.edit.ud_count = self.m.edit.ud_pos;
        self.ud_room();
        self.ud_store(self.m.edit.ud_pos);
        self.m.edit.ud_pos += 1;
        self.m.edit.ud_count = self.m.edit.ud_pos;
    }

    // ud_undo -> a state was restored
    fn ud_undo(&mut self) -> bool {
        if self.m.edit.ud_pos == 0 {
            return false;
        }
        if self.m.edit.ud_pos == self.m.edit.ud_count && self.m.edit.buf.len() < UD_SEG {
            // at the newest state: keep it for redo
            self.ud_room();
            self.ud_store(self.m.edit.ud_count);
            self.m.edit.ud_count += 1;
        }
        self.m.edit.ud_pos -= 1;
        self.ud_restore(self.m.edit.ud_pos);
        true
    }

    // ud_redo -> a state was restored
    fn ud_redo(&mut self) -> bool {
        let p = self.m.edit.ud_pos + 1;
        if p >= self.m.edit.ud_count {
            return false;
        }
        self.m.edit.ud_pos = p;
        self.ud_restore(p);
        true
    }

    // ------------------------------------------------------------------ number box

    /// ui_number_box(id, &mut v, &range) -> EV_* (see the module header)
    pub fn number_box(&mut self, id: u32, v: &mut i32, rg: &Range) -> u32 {
        self.nb_pend_store(id, Some(&mut *v), None, Some(rg), None);
        let entry = *v;
        let mut deferred = 0;
        let step = if rg.step >= 1 { rg.step } else { 1 };
        let mut flags = NB_CAP | TB_DIGITS;
        if rg.min < 0 {
            flags |= TB_SIGNED;
        }
        // committed earlier in this frame by a box that took over the edit buffer
        if self.m.edit.nb_pend == id {
            self.m.edit.nb_pend = 0;
            deferred = self.m.edit.nb_pend_ev;
        }
        let view;
        // (multi-window) only the keyboard window edits
        if self.wp.ctx.focus == id && self.uiw_cur == self.m.win.active {
            if self.m.edit.nb_id != id {
                self.nb_release(); // the previous owner commits its text
                let e = &mut self.m.edit;
                e.nb_id = id;
                e.nb_kind = 0;
                e.nb_buf = nb_format(*v);
                self.nb_sync(*v as u32 as u64); // (U2) the text as shown for v
            }
            self.m.edit.nb_rg = *rg; // (g_nb_rg: the caller's record)
            self.m.edit.nb_seen = self.frame;
            self.nb_follow(*v as u32 as u64, 0); // (U2) v changed elsewhere, text not edited
            // Up / Down step the value while the editor is active
            if self.m.edit.ed_id == id {
                let n = self.nb_step_keys(); // +1 / -1 (arrows), +10 / -10 (pages)
                if n != 0 {
                    let d = n.wrapping_mul(step);
                    let p = nb_parse_text(&self.m.edit.nb_buf, *v);
                    self.nb_store(p.wrapping_add(d as i64), v);
                    self.repass();
                }
            }
            view = false;
        } else if self.m.edit.nb_id == id {
            self.m.edit.nb_rg = *rg;
            view = false; // losing the focus: committed below
        } else {
            self.m.edit.nb_view = nb_format(*v);
            view = true;
        }
        let ev = self.nb_core(id, flags, view);
        // Enter / focus loss: take the typed number (U2: only when the user edited the text)
        if ev & EV_COMMIT != 0 && self.m.edit.nb_id == id {
            if self.nb_edited() {
                let p = nb_parse_text(&self.m.edit.nb_buf, *v);
                self.nb_store(p, v);
                self.repass();
            }
            if self.wp.ctx.focus != id {
                self.m.edit.nb_id = 0;
            }
        }
        let mut out = (ev & !EV_ACT) | deferred; // EV_ACT = the value changed
        if *v != entry {
            out |= EV_ACT;
        }
        self.tb_sem_set(*v, rg.max);
        out
    }

    // the text box of a number box (tb_core on nb_buf or nb_view)
    fn nb_core(&mut self, id: u32, flags: u32, view: bool) -> u32 {
        self.m.edit.tb_natw = NB_NAT_W;
        let mut b = core::mem::take(if view { &mut self.m.edit.nb_view } else { &mut self.m.edit.nb_buf });
        let ev = self.tb_core(id, None, &mut b, flags, 0);
        *(if view { &mut self.m.edit.nb_view } else { &mut self.m.edit.nb_buf }) = b;
        ev
    }

    // nb_step_keys() -> steps typed into the focused number box: Up / Down 1, PgUp / PgDn
    // 10 (consumed)
    fn nb_step_keys(&mut self) -> i32 {
        self.m.edit.ed_self = true;
        let mut n = self.key(VK_UP) as i32;
        n -= self.key(VK_DOWN) as i32;
        n += self.key(VK_PRIOR) as i32 * 10;
        n -= self.key(VK_NEXT) as i32 * 10;
        self.m.edit.ed_self = false;
        n
    }

    /// ui_number_box_f(id, &mut v, &range_f) -> EV_* (U1; see the module header)
    pub fn number_box_f(&mut self, id: u32, v: &mut f64, rf: &RangeF) -> u32 {
        self.nb_pend_store(id, None, Some(&mut *v), None, Some(rf));
        let entry = v.to_bits();
        let mut deferred = 0;
        let mut flags = NB_CAP | TB_DIGITS | TB_DECIMAL;
        if 0.0 > rf.min {
            flags |= TB_SIGNED;
        }
        if self.m.edit.nb_pend == id {
            self.m.edit.nb_pend = 0;
            deferred = self.m.edit.nb_pend_ev;
        }
        let view;
        if self.wp.ctx.focus == id && self.uiw_cur == self.m.win.active {
            // only the keyboard window edits
            if self.m.edit.nb_id != id {
                self.nb_release();
                let t = self.nbf_format(*v, rf.dec);
                let e = &mut self.m.edit;
                e.nb_id = id;
                e.nb_kind = 1;
                e.nb_buf = t;
                self.nb_sync(v.to_bits()); // (U2)
            }
            self.m.edit.nb_rf = *rf;
            self.m.edit.nb_seen = self.frame;
            self.nb_follow(v.to_bits(), rf.dec); // (U2)
            if self.m.edit.ed_id == id {
                let n = self.nb_step_keys();
                if n != 0 {
                    let x = self.nbf_parse(*v);
                    let mut step = rf.step;
                    if !(step > 0.0) {
                        step = (rf.max - rf.min) * 0.01;
                    }
                    let d = n as f64 * step;
                    self.nbf_store(x + d, v);
                    self.repass();
                }
            }
            view = false;
        } else if self.m.edit.nb_id == id {
            self.m.edit.nb_rf = *rf;
            view = false; // losing the focus: committed below
        } else {
            let t = self.nbf_format(*v, rf.dec);
            self.m.edit.nb_view = t;
            view = true;
        }
        let ev = self.nb_core(id, flags, view);
        if ev & EV_COMMIT != 0 && self.m.edit.nb_id == id {
            if self.nb_edited() {
                // (U2) only an edited text is taken
                let x = self.nbf_parse(*v);
                self.nbf_store(x, v);
                self.repass();
            }
            if self.wp.ctx.focus != id {
                self.m.edit.nb_id = 0;
            }
        }
        let mut out = (ev & !EV_ACT) | deferred;
        if v.to_bits() != entry {
            out |= EV_ACT;
        }
        let sv = rf_scaled(rf, *v);
        let mx = rf_scaled(rf, rf.max);
        self.tb_sem_set(sv, mx);
        out
    }

    /// nb_live(id, &mut v) -> true when number box `id` is being edited and the text typed
    /// so far is a number different from v: the value (clamped to the range) is stored
    /// without reformatting the text -- property rows preview while the user types
    pub fn nb_live(&mut self, id: u32, v: &mut i32) -> bool {
        let e = &self.m.edit;
        if id == 0 || e.nb_id != id || e.ed_id != id || e.nb_kind != 0 {
            return false;
        }
        let x = rg_clamp(nb_parse_text(&e.nb_buf, *v), &e.nb_rg); // (no digits: the value)
        if x == *v {
            return false;
        }
        *v = x;
        true
    }

    /// nb_live_f(id, &mut v) -> nb_live of an f64 number box (fixed to its RangeF)
    pub fn nb_live_f(&mut self, id: u32, v: &mut f64) -> bool {
        let e = &self.m.edit;
        if id == 0 || e.nb_id != id || e.ed_id != id || e.nb_kind != 1 {
            return false;
        }
        let Some(x) = parse_f64_sep(&e.nb_buf, self.m.value.decimal_sep) else { return false };
        let x = rf_fix(&e.nb_rf, x);
        if x.to_bits() == v.to_bits() {
            return false;
        }
        *v = x;
        true
    }

    // nbf_format(v, decimals): the f64 with the range's decimals and the decimal separator
    fn nbf_format(&self, v: f64, dec: u32) -> Vec<u8> {
        fmt_f64_sep(v, dec, self.m.value.decimal_sep).into_bytes()
    }

    // nbf_parse(stored) -> the number typed into nb_buf, or the stored value without digits
    fn nbf_parse(&self, stored: f64) -> f64 {
        parse_f64_sep(&self.m.edit.nb_buf, self.m.value.decimal_sep).unwrap_or(stored)
    }

    // nbf_store(x, &mut v): fix the value to the owner's RangeF, store it, show it
    fn nbf_store(&mut self, x: f64, v: &mut f64) {
        let rf = self.m.edit.nb_rf;
        *v = rf_fix(&rf, x);
        let t = self.nbf_format(*v, rf.dec);
        self.m.edit.nb_buf = t;
        self.nb_sync(v.to_bits()); // (U2)
        self.nb_caret_end();
    }

    // nb_store(x, &mut v): clamp a 64-bit value to the owner's range, store it, show it
    fn nb_store(&mut self, x: i64, v: &mut i32) {
        *v = rg_clamp(x, &self.m.edit.nb_rg);
        self.m.edit.nb_buf = nb_format(*v);
        self.nb_sync(*v as u32 as u64); // (U2)
        self.nb_caret_end();
    }

    // the caret after the new text (when the owner is the editor)
    fn nb_caret_end(&mut self) {
        let e = &mut self.m.edit;
        if e.nb_id != 0 && e.ed_id == e.nb_id {
            e.ed_caret = e.nb_buf.len();
            e.ed_anchor = e.nb_buf.len();
        }
    }

    // (U2) nb_sync(shown): nb_buf now shows the value with these bits: remember that text
    // (nb_orig) and value (nb_shown); the box counts as edited as soon as its text differs
    fn nb_sync(&mut self, shown: u64) {
        let e = &mut self.m.edit;
        e.nb_orig.clear();
        e.nb_orig.extend_from_slice(&e.nb_buf[..e.nb_buf.len().min(NB_CAP as usize - 1)]);
        e.nb_shown = shown;
    }

    // (U2) nb_edited() -> the user changed the text since it was formatted
    fn nb_edited(&self) -> bool {
        self.m.edit.nb_buf != self.m.edit.nb_orig
    }

    // (U2) nb_follow(value bits, decimals): the value of the focused number box changed
    // elsewhere (another widget, a command, a tool) while its text is unedited: show the new
    // value; the caret goes to the end, a selection of the whole text stays a selection of
    // the whole text
    fn nb_follow(&mut self, cur: u64, dec: u32) {
        if cur == self.m.edit.nb_shown || self.nb_edited() {
            return; // the typed text wins
        }
        let e = &self.m.edit;
        let old = e.nb_buf.len();
        let (lo, hi) = if e.ed_caret >= e.ed_anchor { (e.ed_anchor, e.ed_caret) } else { (e.ed_caret, e.ed_anchor) };
        let all = lo == 0 && hi == old && old != 0;
        let t = if e.nb_kind == 0 { nb_format(cur as u32 as i32) } else { self.nbf_format(f64::from_bits(cur), dec) };
        self.m.edit.nb_buf = t;
        self.nb_sync(cur);
        let e = &mut self.m.edit;
        if e.ed_id == e.nb_id {
            e.ed_caret = e.nb_buf.len();
            e.ed_anchor = e.nb_buf.len();
            if all {
                e.ed_anchor = 0;
            }
        }
    }

    // nb_release: the box owning nb_buf commits its text (another box takes it).  The asm
    // stores the text into the owner's value here (through its saved &v); see pend_text.
    fn nb_release(&mut self) {
        let old = self.m.edit.nb_id;
        if old == 0 {
            return;
        }
        self.m.edit.nb_id = 0;
        if self.frame.wrapping_sub(self.m.edit.nb_seen) > 2 {
            return; // not drawn lately: drop the text
        }
        if self.nb_edited() {
            // (U2) not edited: nothing is stored
            let e = &mut self.m.edit;
            e.pend_text = Some((old, e.nb_kind, e.nb_buf.clone()));
            self.repass();
        }
        self.m.edit.nb_pend = old;
        self.m.edit.nb_pend_ev = EV_COMMIT;
    }

    // (port) the store nb_release left for the box (its next call, before it reads v):
    // parse, clamp / fix, store; EV_ACT joins the pending events when the value changed
    fn nb_pend_store(&mut self, id: u32, vi: Option<&mut i32>, vf: Option<&mut f64>, rg: Option<&Range>, rf: Option<&RangeF>) {
        match &self.m.edit.pend_text {
            Some((pid, _, _)) if *pid == id => {}
            _ => return,
        }
        let Some((_, kind, t)) = self.m.edit.pend_text.take() else { return };
        let changed = match (kind, vi, vf, rg, rf) {
            (0, Some(v), _, Some(rg), _) => {
                let old = *v;
                *v = rg_clamp(nb_parse_text(&t, *v), rg);
                *v != old
            }
            (1, _, Some(v), _, Some(rf)) => {
                let old = v.to_bits();
                let x = parse_f64_sep(&t, self.m.value.decimal_sep).unwrap_or(*v);
                *v = rf_fix(rf, x);
                v.to_bits() != old
            }
            _ => false,
        };
        if changed && self.m.edit.nb_pend == id {
            self.m.edit.nb_pend_ev |= EV_ACT;
        }
    }
}

// ------------------------------------------------------------------ editors of their own
// (ROADMAP §6: richtext in rich_edit.rs, code_editor in code_edit.rs; see ed_core.rs)

impl Ui {
    /// an editor with a text model of its own holds the edit state while it has the focus:
    /// its K_TEXTBOX record's text has the selection anchor .. caret (byte offsets).  The key
    /// filter of app-level key(), textbox_selection / textbox_select and a screen reader's
    /// selection then work as for a text box; a text box that held the state commits.
    pub(crate) fn ed_hold(&mut self, id: u32, anchor: usize, caret: usize) {
        let e = &mut self.m.edit;
        if e.ed_id != id {
            if e.ed_id != 0 {
                e.ed_blur = e.ed_id;
            }
            e.ed_id = id;
            e.ed_drag = false;
            self.ud_reset();
        }
        let frame = self.frame;
        let e = &mut self.m.edit;
        if e.ed_blur == id {
            e.ed_blur = 0;
        }
        e.ed_anchor = anchor;
        e.ed_caret = caret;
        e.ed_seen = frame;
        e.ed_multi = true;
        e.ed_prefx = -1;
    }

    /// the editor lost the focus: it gives the edit state back (the IME and magnifiers lose
    /// its caret)
    pub(crate) fn ed_drop(&mut self, id: u32) {
        if self.m.edit.ed_id == id {
            self.m.edit.ed_id = 0;
            self.ime_off();
        }
    }
}
