// auto -- UI automation for headless tests: scripts and the widget dump  [src/ui/ui_auto.s]
//
//   script_run(app, path) -> exit code   0 all expectations met, 1 failures, 2 parse error
//   dump(path)                           widget tree as JSON lines (None = stdout)
//
// Script syntax (one command per line, '#' starts a comment; coordinates in DIP):
//   size W H                      resize the headless window
//   theme dark|light|contrast
//   theme file PATH | none        an app theme from a file (an error is printed and kept for
//                                 `expect theme error`; the theme stays) / no app theme
//   style NAME                    another style linked into the app (Ui::set_style)
//   move S          down S [l|r|m]      up [S] [l|r|m]      click S [l|r]
//   rclick S        dblclick S [shift|ctrl|alt]...
//                   drag S1 S2 [steps] [shift|ctrl|alt|right|middle]...
//                   drag ... pen|eraser P0 P1   (PN1) a pen stroke, pressure P0 -> P1 (%)
//   pointer mouse | pen [P [TX TY]] | eraser [P [TX TY]] | touch   (PN1) pen data of the
//                   following pointer events
//   wheel S DY [shift|ctrl|alt]...   DY in wheel units (120 = one notch); the words hold
//                   those modifiers during the wheel event (e.g. Ctrl + wheel zoom)
//   key CHORD...    e.g. key ctrl+s   key tab   key shift+tab enter esc left f4 apps (menu key)
//                   (also [ ] , . - + / and num0..num9 numadd numsub nummul numdiv numdec)
//   keys [N] CHORD...   the chords N times over, queued for ONE pass of the window with the
//                   keyboard (more keys than frames: what a tool window hands on); `key` gives
//                   every key a pass of its own, as the platform does
//   focus S         give the widget the keyboard focus (focus ring shown)
//   type "text"     ("\n" presses Enter)
//   commit "text" [N]   an input method commits the text (N times over: one long commit)
//   cmd ID [ARG]    run a command as if a menu item was chosen
//   wait MS         advance the virtual clock (16 ms frames; timers, animations and the
//                   sleep()s of async tasks)
//   shot FILE.png   render and save a screenshot
//   dump [FILE]     print / save the widget tree
//   expect S exists | missing | hover | focused | checked | unchecked | disabled |
//            enabled | selected | open | pressed | value N | max N | text "s" | rect X Y W H
//            (rect: '*' takes any value)
//   expect theme "NAME" | none    the app theme;  expect theme error "TEXT" | none  the error
//                                 of the last `theme file` contains TEXT
//   expect style NAME   expect token NAME #RRGGBB|#AARRGGBB (a colour token now)
//   expect pixel X Y #RRGGBB      the main window's pixel at (X, Y), rendered now
//   expect same FILE.png          the main window rendered now has the pixels of that shot
//   print TEXT      echo
//   bench N         time N frames (build, full render, unchanged frame) with the real clock
//   bench N anim [MAX_US]   N frames 16 ms apart on the virtual clock (timers, animations):
//                   each frame's passes and the render of its dirty cells, timed with the real
//                   clock (--verify-dirty compares afterwards, untimed); prints mean, median, max
//   bench N drag S1 S2 [middle|right|shift|ctrl|alt]... [MAX_US]   the same for the N moves of
//                   a drag (press and release untimed; every other run drags back)
//   bench N wheel S DY [shift|ctrl|alt]... [MAX_US]   ... for N wheel events at S, DY and -DY
//                   by turns (with ctrl: zoom in and out)
//   bench N key CHORD [MAX_US]    ... for N presses of a key (e.g. bench 20 key shift+right)
//   bench N type "s" [MAX_US]     ... for N times typing the text (one frame each)
//                   With MAX_US a run passes when its median is at most MAX_US; up to three
//                   runs (the median and the retries ride out a busy machine), FAIL when none
//                   does
//   quit
//   system focus in|out | suspend | resume | session-end | settings theme|locale|contrast|fonts|N
//                       the platform's lifecycle events (UIC_ACTIVE, UIC_SUSPEND, ...)
// Windows (ID = the app's window id, 0 / main = the main window):
//   window ID|main|any            selectors only look into that window (default any)
//   expect window ID open|closed|visible|active|modal|keys   expect window ID rect X Y W H
//                                 (keys: the window keys go to now -- a tool window borrows them
//                                 from its owner after F6 / Ctrl+F6 or a click on a widget)
//   close window ID   movewin ID X Y   resizewin ID W H   activate ID
//   scale P [window ID]           dpi of the main window (default) or of one window
//   shot F.png [window ID|desktop] [X Y W H]   one window, or all visible windows on the
//                                 virtual desktop; a crop rectangle in DIP
//   screen X Y W H | off          (§6) the virtual screen (the monitor's work area, DIP of the
//                                 main window's client area): popups of every window may leave
//                                 it up to there and flip at its edges; off (the default): the
//                                 main window's client area holds all popups
//   sync [MS]                     wait until no background job is pending: the app's sync hook,
//                                 the async tasks' work (blocking jobs, wake-ups) and the spell
//                                 checker's workers (suggestions, edit_spell.rs)
//   expect sound KIND|none        the last sound ui.beep / ui.dialog_sound asked for since the
//                                 last check (default, information, warning, error, question,
//                                 notify; headless runs play none)
//   expect_warn / expect_nowarn   a UIWARN was / was not printed since the last check (a
//                                 script also warns about a widget id used twice in a pass)
// (K1) Files and clipboard, drag and drop:
//   drop X Y FILE...              files dropped at (X, Y) DIP of the scoped window
//   drop X Y [files F...] [text "s"] [html "s"] [url U] [image F.png] [copy|move|link]
//                                 other content dropped there; drop X Y self: the app's own
//                                 drag that left the window comes back
//   dragover X Y [files] [text] [url] [html] [image] [copy|move|link]   another program's
//                                 drag hovers there (default files);  dragleave: it left
//   dragout expect none | text "s" | html "s" | url "s" | files N | image | kinds N
//                                 what the app's drag handed over when it left the window
//   dragout end copy|move|link|none   the platform ended that drag (E_DRAG_END)
//   (widgets dragged onto widgets: drag S1 S2; key esc while held cancels)
//   clip text "s" | clip file F | clip dib F | clip clear | clip save F [bmp] |
//   clip expect none|text|image|png
// Permissions and agent control (auto_perm.rs):
//   perm answer once|always|deny|never|dialog|none   perm timeout MS   perm store FILE|none
//   perm reload   perm wait [MS]   perm reset   perm expect open|none|RULE ANSWER
//   agent on|off   agent call ACTION [JSON]   agent expect ok|error CODE
// The live inspector (inspect.rs):
//   inspect on|off|toggle|arm|disarm|pick|copy|snap   inspect select S|none
//   inspect option outlines|order|dirty on|off   inspect tab widgets|frame|colors
//   inspect expect open|closed|pick on|off|flash on|off|idle|same|excluded|selected S|
//                  hover S|selector "s"|copied "s"
// Selectors S:  #123  (widget id)   "Label"   kind:"Label"   X,Y  (a point in DIP)
// A label must match exactly one widget (plain text is ignored when a control with the
// same label exists; matches in several windows: the topmost window, then the topmost
// layer).  #id selectors must be unique across windows (else a UIWARN, the topmost window
// wins).  Points are in the scoped window (main for any).  Failed expectations print
//   FAIL line N: <command> -> <detail>
// and the script continues.
//
// Scripts are bytes (as the asm reads them): lines of at most 1023 bytes, CR dropped; a NUL
// ends the script.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use super::consts::*;
use super::keys::*;
use super::layout::{dp_at, px2dip_at};
use super::runtime::{parse_hex, parse_int};
use super::{App, Sem, Ui};
use gfx::{Surface, FMT_XRGB};
use plat::{DragData, Event};

const LINE_MAX: usize = 1024;

/// script runner state (ui_auto.s .bss / .data); survives between scripts (scope, the
/// warning mark) and between commands (the pointer)
pub(crate) struct S {
    lineno: u32,
    fails: u32,
    parse_err: bool,
    line: Vec<u8>,
    script_mods: u32,
    sel_err: u32,
    sel_kind: i32,
    sel_wf: i32,
    sel_win: u32,
    sel_x: i32,
    sel_y: i32,
    sel_label: Vec<u8>,
    sc_win: u32,
    sc_x: i32,
    sc_y: i32,
    sc_pdata: u32,
    warn_mark: u32,
    sc_scope: i32,
    /// the agent session of `agent call` (auto_perm.rs)
    pub(crate) agent: super::auto_perm::S,
    /// the error of the last `theme file` (None: it was applied)
    theme_err: Option<String>,
}

impl Default for S {
    fn default() -> S {
        S {
            lineno: 0,
            fails: 0,
            parse_err: false,
            line: Vec::new(),
            script_mods: 0,
            sel_err: 0,
            sel_kind: -1,
            sel_wf: -1,
            sel_win: 0,
            sel_x: 0,
            sel_y: 0,
            sel_label: Vec::new(),
            sc_win: 0,
            sc_x: 0,
            sc_y: 0,
            sc_pdata: 0,
            warn_mark: 0,
            sc_scope: -1, // `window` scope: window id, -1 = any
            agent: Default::default(),
            theme_err: None,
        }
    }
}

/// a resolved selector: a semantic record, or a point (a zero record)
#[derive(Clone, Copy)]
pub(crate) enum Sel {
    Rec(usize),
    Point,
}

// ------------------------------------------------------------------ parsing helpers

// skip_ws: blanks and tabs
pub(crate) fn skip_ws(p: &[u8]) -> &[u8] {
    let i = p.iter().position(|&c| c != b' ' && c != b'\t').unwrap_or(p.len());
    &p[i..]
}

// word_eq(p, word) -> the rest after the word when p starts with it followed by a blank /
// the end
pub(crate) fn word_eq<'a>(p: &'a [u8], w: &str) -> Option<&'a [u8]> {
    let w = w.as_bytes();
    if !p.starts_with(w) {
        return None;
    }
    match p.get(w.len()) {
        None | Some(b' ') | Some(b'\t') => Some(&p[w.len()..]),
        _ => None,
    }
}

// next_word: after the current word and the following blanks
pub(crate) fn next_word(p: &[u8]) -> &[u8] {
    let i = p.iter().position(|&c| c == b' ' || c == b'\t').unwrap_or(p.len());
    skip_ws(&p[i..])
}

// parse_qstring(p, cap) -> (string (at most cap - 1 bytes), rest after it); "\n" and "\t"
// escapes, any other escaped character as itself
pub(crate) fn parse_qstring(p: &[u8], cap: usize) -> Option<(Vec<u8>, &[u8])> {
    if p.first() != Some(&b'"') {
        return None;
    }
    let mut out = Vec::new();
    let mut i = 1;
    loop {
        let c = *p.get(i)?;
        i += 1;
        let ch = match c {
            b'"' => return Some((out, &p[i..])),
            b'\\' => {
                let e = *p.get(i)?; // "\" at the end of the line: no string
                i += 1;
                match e {
                    b'n' => 10,
                    b't' => 9,
                    o => o,
                }
            }
            o => o,
        };
        if out.len() < cap - 1 {
            out.push(ch);
        }
    }
}

// utf8_decode (str.s): codepoint and the next index; invalid bytes, overlong forms,
// surrogates and values above U+10FFFF decode as U+FFFD and advance one byte
fn utf8_decode(s: &[u8], i: usize) -> (u32, usize) {
    let b0 = s.get(i).copied().unwrap_or(0) as u32;
    if b0 < 0x80 {
        return (b0, i + 1);
    }
    let bad = (0xFFFD, i + 1);
    let (mut cp, n, min) = if b0 < 0xC2 {
        return bad;
    } else if b0 < 0xE0 {
        (b0 & 0x1F, 1, 0x80)
    } else if b0 < 0xF0 {
        (b0 & 0x0F, 2, 0x800)
    } else if b0 < 0xF5 {
        (b0 & 7, 3, 0x10000)
    } else {
        return bad;
    };
    let mut j = i + 1;
    for _ in 0..n {
        let c = s.get(j).copied().unwrap_or(0) as u32;
        if c & 0xC0 != 0x80 {
            return bad;
        }
        cp = cp << 6 | (c & 0x3F);
        j += 1;
    }
    if cp < min || cp > 0x10FFFF || cp & 0xFFFF_F800 == 0xD800 {
        return bad;
    }
    (cp, j)
}

// parse_button(p) -> BTN_* (default left)
fn parse_button(p: &[u8]) -> u32 {
    match p.first().map(|c| c | 32) {
        Some(b'r') => BTN_RIGHT,
        Some(b'm') => BTN_MIDDLE,
        _ => BTN_LEFT,
    }
}

// chord(p) -> (modifiers, virtual key, the rest after the chord's word): "ctrl+shift+f3"; None
// = an unknown key
fn chord(p: &[u8]) -> Option<(u32, u32, &[u8])> {
    let mut p = p;
    let mut mods = 0;
    loop {
        if p.starts_with(b"ctrl+") {
            mods |= MOD_CTRL;
            p = &p[5..];
        } else if p.starts_with(b"shift+") {
            mods |= MOD_SHIFT;
            p = &p[6..];
        } else if p.starts_with(b"alt+") {
            mods |= MOD_ALT;
            p = &p[4..];
        } else {
            break;
        }
    }
    // the key name ends at a blank
    let vk = key_lookup(p);
    (vk != 0).then(|| (mods, vk, next_word(p)))
}

// mod_word(p) -> MOD_* of the word shift / ctrl / alt at p (0 = another word): the modifiers
// drag and wheel hold during their events
fn mod_word(p: &[u8]) -> u32 {
    if p.starts_with(b"shift") {
        MOD_SHIFT
    } else if p.starts_with(b"ctrl") {
        MOD_CTRL
    } else if p.starts_with(b"alt") {
        MOD_ALT
    } else {
        0
    }
}

// parse_pct(p) -> (pressure units: % 0..100 -> 0..PRESSURE_ONE, rest)
fn parse_pct(p: &[u8]) -> (i32, &[u8]) {
    let (v, r) = parse_int(p);
    let v = v.clamp(0, 100) as u32;
    ((v * 65535 / 100) as i32, r)
}

// key_lookup(p) -> VK for the word at p (0 = unknown)
fn key_lookup(p: &[u8]) -> u32 {
    let c0 = p.first().copied().unwrap_or(0);
    let c1 = p.get(1).copied().unwrap_or(0);
    if c1 == b' ' || c1 == 0 {
        // single letter / digit
        if c0.is_ascii_lowercase() {
            return (c0 - 32) as u32;
        }
        if c0.is_ascii_uppercase() || c0.is_ascii_digit() {
            return c0 as u32;
        }
    }
    const NAMES: &[(&str, u32)] = &[
        ("enter", VK_RETURN), ("esc", VK_ESCAPE), ("tab", VK_TAB), ("space", VK_SPACE),
        ("backspace", VK_BACK), ("delete", VK_DELETE), ("left", VK_LEFT), ("right", VK_RIGHT),
        ("up", VK_UP), ("down", VK_DOWN), ("home", VK_HOME), ("end", VK_END),
        ("pgup", VK_PRIOR), ("pgdn", VK_NEXT), ("insert", VK_INSERT), ("alt", VK_MENU), ("apps", VK_APPS),
        ("plus", VK_OEM_PLUS), ("minus", VK_OEM_MINUS),
        // (U1) punctuation (US layout keys) and the numeric keypad
        ("[", VK_OEM_4), ("]", VK_OEM_6), (",", VK_OEM_COMMA), (".", VK_OEM_PERIOD),
        ("-", VK_OEM_MINUS), ("+", VK_OEM_PLUS), ("/", VK_OEM_2),
        ("numadd", VK_ADD), ("numsub", VK_SUBTRACT), ("nummul", VK_MULTIPLY), ("numdiv", VK_DIVIDE),
        ("numdec", VK_DECIMAL),
        ("num0", VK_NUMPAD0), ("num1", VK_NUMPAD0 + 1), ("num2", VK_NUMPAD0 + 2), ("num3", VK_NUMPAD0 + 3),
        ("num4", VK_NUMPAD0 + 4), ("num5", VK_NUMPAD0 + 5), ("num6", VK_NUMPAD0 + 6), ("num7", VK_NUMPAD0 + 7),
        ("num8", VK_NUMPAD0 + 8), ("num9", VK_NUMPAD0 + 9),
    ];
    for &(n, vk) in NAMES {
        if word_eq(p, n).is_some() {
            return vk;
        }
    }
    // f1..f12
    if c0 | 32 == b'f' {
        let (n, _) = parse_int(&p[1..]);
        if (1..=12).contains(&n) {
            return VK_F1 + n as u32 - 1;
        }
    }
    0
}

/// json_escape(src, cap): src with " \ escaped and control characters as blanks, at most
/// cap - 8 bytes copied (room for an escape)
fn json_escape(out: &mut Vec<u8>, src: &[u8], cap: usize) {
    let lim = cap - 8;
    let mut n = 0;
    for &c in src {
        if n >= lim || c == 0 {
            break;
        }
        match c {
            b'"' | b'\\' => {
                out.push(b'\\');
                out.push(c);
                n += 2;
            }
            c if c < 32 => {
                out.push(b' '); // control characters become spaces
                n += 1;
            }
            c => {
                out.push(c);
                n += 1;
            }
        }
    }
}

const WFLAG_NAMES: [&str; 9] = ["tool", "dialog", "modal", "resizable", "noclose", "sysframe", "noactivate", "autoheight", "keys"];

// copy_px(dst, src, x, y): gfx_blit_copy -- src at (x, y) of dst, clipped, no blending
fn blit_copy(dst: &mut Surface, src: &Surface, x: i32, y: i32) {
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + src.w).min(dst.w);
    let y1 = (y + src.h).min(dst.h);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let n = (x1 - x0) as usize * 4;
    for yy in y0..y1 {
        let d = yy as usize * dst.stride as usize + x0 as usize * 4;
        let s = (yy - y) as usize * src.stride as usize + (x0 - x) as usize * 4;
        dst.px[d..d + n].copy_from_slice(&src.px[s..s + n]);
    }
}

// crop_surface(sf, dpi, crop) -> a copy of the DIP rectangle crop (x, y, w, h) of sf,
// clipped to it (None when nothing is left)
fn crop_surface(sf: &Surface, dpi: i32, c: [i32; 4]) -> Option<Surface> {
    let x = dp_at(c[0], dpi);
    let y = dp_at(c[1], dpi);
    let mut w = dp_at(c[2], dpi);
    let mut h = dp_at(c[3], dpi);
    if x < 0 || y < 0 {
        return None;
    }
    let ww = sf.w - x;
    if ww <= 0 {
        return None;
    }
    w = w.min(ww);
    let hh = sf.h - y;
    if hh <= 0 {
        return None;
    }
    h = h.min(hh);
    if w <= 0 || h <= 0 {
        return None;
    }
    let mut out = Surface::new(w, h, sf.fmt);
    for r in 0..h as usize {
        let s = (y as usize + r) * sf.stride as usize + x as usize * 4;
        let d = r * out.stride as usize;
        out.px[d..d + w as usize * 4].copy_from_slice(&sf.px[s..s + w as usize * 4]);
    }
    Some(out)
}

impl Ui {
    // ------------------------------------------------------------------ dump

    /// ui_dump(path): JSON lines -- a header, one line per shown secondary window, then one
    /// line per semantic record (None = stdout)
    pub fn dump(&mut self, path: Option<&[u8]>) {
        let mut out: Vec<u8> = Vec::new();
        self.dump_header(&mut out);
        for i in 1..WIN_MAX as u32 {
            if self.uiw_drawable(i) {
                self.dump_window(&mut out, i);
            }
        }
        for k in 0..self.sems.len() {
            let r = self.sems[k].clone();
            self.dump_record(&mut out, &r);
        }
        match path {
            Some(p) => {
                super::perm::trusted(|| plat::write_file(&String::from_utf8_lossy(p), &out));
            }
            None => plat::write_stdout(&out),
        }
    }

    fn dump_header(&self, out: &mut Vec<u8>) {
        let mut s = String::new();
        let scale = ((self.wp.rt.dpi as u32).wrapping_mul(100).wrapping_add(48)) / 96;
        let _ = write!(
            s,
            "{{\"frame\":{},\"size\":[{},{}],\"scale\":{},\"theme\":\"{}\",\"focus\":{},\"hot\":{},\"keys\":{}}}\n",
            self.frame,
            self.px2dip(self.wp.rt.fb.w),
            self.px2dip(self.wp.rt.fb.h),
            scale,
            if self.theme_dark { "dark" } else { "light" },
            self.wp.ctx.focus,
            self.wp.ctx.hot,
            self.uiw_rec_id(self.uiw_keys_window()) // the window the keys go to (0 = main)
        );
        out.extend_from_slice(s.as_bytes());
    }

    // dump_window(i): {"window":ID,"title":"..","flags":"tool","rect":[x,y,w,h],"size":[w,h],
    // "scale":S,"active":0}
    fn dump_window(&self, out: &mut Vec<u8>, i: u32) {
        let rec = &self.m.win.recs[i as usize]; // ui-win: Rec holds shared values
        let mut title = Vec::new();
        json_escape(&mut title, self.uiw_title(i).as_bytes(), 180);
        let mut flags = String::new();
        for (b, n) in WFLAG_NAMES.iter().enumerate() {
            if rec.flags & (1 << b) != 0 {
                if !flags.is_empty() {
                    flags.push(' ');
                }
                flags.push_str(n);
            }
        }
        let dpi = self.win_dpi(i);
        let fb = self.win_fb(i);
        out.extend_from_slice(alloc::format!("{{\"window\":{},\"title\":\"", rec.id).as_bytes());
        out.extend_from_slice(&title);
        let r = rec.rect.get(); // ui-win: the WINRECT (the app's)
        let s = alloc::format!(
            "\",\"flags\":\"{}\",\"rect\":[{},{},{},{}],\"size\":[{},{}],\"scale\":{},\"active\":{},\"focus\":{}}}\n",
            flags,
            r.x,
            r.y,
            r.w,
            r.h,
            px2dip_at(fb.w, dpi),
            px2dip_at(fb.h, dpi),
            ((dpi as u32).wrapping_mul(100).wrapping_add(48)) / 96,
            (i == self.m.win.active) as u32,
            self.uiw_parts(i).map_or(0, |p| p.ctx.focus)
        );
        out.extend_from_slice(s.as_bytes());
    }

    // dump_record(sem): one record line
    fn dump_record(&self, out: &mut Vec<u8>, r: &Sem) {
        let mut st = String::new();
        for (k, n) in STATE_NAMES.iter().enumerate() {
            if r.state & (1 << k) != 0 {
                if !st.is_empty() {
                    st.push(',');
                }
                st.push('"');
                st.push_str(n);
                st.push('"');
            }
        }
        let mut lab = Vec::new();
        json_escape(&mut lab, r.label.as_deref().unwrap_or("").as_bytes(), 180);
        // rect in DIP (at the dpi of the record's window)
        let dpi = self.win_dpi(r.win as u32);
        let kind = if r.kind < K_KIND_COUNT { KIND_NAMES[r.kind as usize] } else { KIND_NAMES[0] };
        out.extend_from_slice(
            alloc::format!("{{\"l\":{},\"d\":{},\"id\":{},\"k\":\"{}\",\"t\":\"", r.layer, r.depth, r.id, kind).as_bytes(),
        );
        out.extend_from_slice(&lab);
        let mut s = alloc::format!(
            "\",\"r\":[{},{},{},{}],\"s\":[{}],\"v\":{},\"max\":{}",
            px2dip_at(r.rect[0], dpi),
            px2dip_at(r.rect[1], dpi),
            px2dip_at(r.rect[2], dpi),
            px2dip_at(r.rect[3], dpi),
            st,
            r.value,
            r.max
        );
        if r.win != 0 {
            // records of other windows: "w":ID
            let _ = write!(s, ",\"w\":{}", self.uiw_rec_id(r.win as u32));
        }
        s.push_str("}\n");
        out.extend_from_slice(s.as_bytes());
    }

    // ------------------------------------------------------------------ script runner

    /// ui_script_run(path) -> exit code
    pub(crate) fn script_run(&mut self, app: &mut dyn App, path: &str) -> i32 {
        let Some(data) = super::perm::trusted(|| plat::read_file(path)) else {
            plat::write_stderr(alloc::format!("error: cannot read script {}\n", path).as_bytes());
            return 2;
        };
        self.sc.fails = 0;
        self.sc.parse_err = false;
        self.sc.lineno = 0;
        self.dup_check = true;
        let mut pos = 0;
        while pos < data.len() && data[pos] != 0 {
            self.sc.lineno += 1;
            // the next line (without CR / LF, at most LINE_MAX - 1 bytes)
            let mut line = Vec::new();
            while pos < data.len() && data[pos] != 0 {
                let c = data[pos];
                pos += 1;
                if c == b'\n' {
                    break;
                }
                if c == b'\r' {
                    continue;
                }
                if line.len() < LINE_MAX - 1 {
                    line.push(c);
                }
            }
            if self.script_exec(app, &line) {
                break; // quit
            }
            if self.sc.parse_err {
                break;
            }
        }
        if self.sc.parse_err {
            2
        } else if self.sc.fails != 0 {
            1
        } else {
            0
        }
    }

    // fail(detail): report a failed command
    pub(crate) fn fail(&mut self, detail: &[u8]) {
        self.fail_more(detail);
        plat::write_stdout(b"\n");
    }

    // fail_more(detail): like fail, but the line is finished by the caller
    fn fail_more(&mut self, detail: &[u8]) {
        self.sc.fails += 1;
        let mut o = alloc::format!("FAIL line {}: ", self.sc.lineno).into_bytes();
        o.extend_from_slice(&self.sc.line);
        o.extend_from_slice(b" -> ");
        o.extend_from_slice(detail);
        plat::write_stdout(&o);
    }

    // sel_fail: report a selector that did not resolve
    pub(crate) fn sel_fail(&mut self) {
        let m: &[u8] = match self.sc.sel_err {
            2 => b"ambiguous selector",
            3 => b"bad selector",
            _ => b"no such widget",
        };
        self.fail(m);
    }

    // script_exec(line) -> true = quit
    fn script_exec(&mut self, app: &mut dyn App, line: &[u8]) -> bool {
        self.sc.line = line.to_vec(); // a copy of the line for messages
        let l = skip_ws(line);
        if l.is_empty() || l[0] == b'#' {
            return false;
        }
        const CMDS: [&str; 44] = [
            "quit", "print", "size", "theme", "move", "down", "up", "click", "rclick", "dblclick", "drag", "wheel",
            "key", "keys", "type", "commit", "cmd", "wait", "shot", "dump", "expect", "bench", "window", "close",
            "movewin", "resizewin", "activate", "scale", "sync", "expect_warn", "expect_nowarn", "focus", "drop",
            "clip", "pointer", "system", "dragover", "dragleave", "dragout", "screen", "perm", "agent", "inspect",
            "style",
        ];
        let Some(&c) = CMDS.iter().find(|c| word_eq(l, c).is_some()) else {
            let mut o = alloc::format!("error: line {}: unknown command: ", self.sc.lineno).into_bytes();
            o.extend_from_slice(l);
            o.push(b'\n');
            plat::write_stderr(&o);
            self.sc.parse_err = true;
            return false;
        };
        let a = next_word(l);
        match c {
            "quit" => return true,
            "system" => self.sc_system(app, a),
            "print" => {
                let mut o = a.to_vec();
                o.push(b'\n');
                plat::write_stdout(&o);
            }
            "size" => {
                let (w, r) = parse_int(a);
                let (h, _) = parse_int(r);
                let (w, h) = (self.dp(w), self.dp(h));
                self.resize(app, w, h);
            }
            "theme" => {
                if let Some(r) = word_eq(a, "file") {
                    // an app theme from a file; an error is remembered (expect theme error)
                    let path = String::from(String::from_utf8_lossy(skip_ws(r)).trim());
                    let res = super::perm::trusted(|| self.load_theme(&path));
                    self.sc.theme_err = res.err().map(|e| {
                        plat::write_stdout(alloc::format!("theme: {}\n", e).as_bytes());
                        e.msg
                    });
                } else if word_eq(a, "none").is_some() {
                    self.clear_theme();
                } else if word_eq(a, "contrast").is_some() {
                    self.set_contrast(Some(true));
                } else {
                    self.set_contrast(Some(false));
                    let dark = word_eq(a, "dark").is_some();
                    self.theme_set(dark as u32);
                }
                self.cmd_run(UIC_THEME, 0);
                self.run_passes(app);
            }
            "move" => match self.resolve(a).0 {
                None => self.sel_fail(),
                Some(_) => {
                    let (x, y) = (self.sc.sel_x, self.sc.sel_y);
                    self.mouse_to(app, x, y);
                }
            },
            "down" => self.sc_button(app, a, plat::E_DOWN),
            "up" => self.sc_button(app, a, plat::E_UP),
            "click" => {
                let (s, rest) = self.resolve(a);
                if s.is_none() {
                    self.sel_fail();
                } else {
                    let (x, y) = (self.sc.sel_x, self.sc.sel_y);
                    self.mouse_to(app, x, y);
                    let b = parse_button(skip_ws(rest));
                    self.click_here(app, b);
                }
            }
            "rclick" => {
                if self.resolve(a).0.is_none() {
                    self.sel_fail();
                } else {
                    let (x, y) = (self.sc.sel_x, self.sc.sel_y);
                    self.mouse_to(app, x, y);
                    self.click_here(app, BTN_RIGHT);
                }
            }
            "dblclick" => {
                let (s, rest) = self.resolve(a);
                if s.is_none() {
                    self.sel_fail();
                } else {
                    // optional words: shift ctrl alt, held during both clicks
                    let saved_mods = self.sc.script_mods;
                    let mut mods = saved_mods;
                    let mut q = skip_ws(rest);
                    while !q.is_empty() {
                        let m = mod_word(q);
                        if m == 0 {
                            let mut d = b"unknown dblclick option ".to_vec();
                            d.extend_from_slice(q);
                            self.fail(&d);
                            return false;
                        }
                        mods |= m;
                        q = next_word(q);
                    }
                    let (x, y) = (self.sc.sel_x, self.sc.sel_y);
                    self.mouse_to(app, x, y);
                    self.sc.script_mods = mods;
                    self.click_here(app, BTN_LEFT);
                    self.advance_time(app, 50);
                    self.click_here(app, BTN_LEFT);
                    self.sc.script_mods = saved_mods;
                }
            }
            "drag" => self.sc_drag(app, a),
            "wheel" => self.sc_wheel(app, a),
            "key" => self.sc_key(app, a),
            "keys" => self.sc_keys(app, a),
            "type" => self.sc_type(app, a),
            "commit" => self.sc_commit(app, a),
            "cmd" => {
                let (id, r) = parse_int(a);
                let (arg, _) = parse_int(r);
                self.cmd_run(id as u32, arg as u32);
                self.run_passes(app);
            }
            "wait" => {
                let (ms, _) = parse_int(a);
                self.advance_time(app, ms);
            }
            "shot" => self.sc_shot(app, a),
            "dump" => self.dump(if a.is_empty() { None } else { Some(a) }),
            "expect" => self.sc_expect(a),
            "bench" => self.sc_bench(app, a),
            "window" => {
                self.sc.sc_scope = if word_eq(a, "any").is_some() {
                    -1
                } else if word_eq(a, "main").is_some() {
                    0
                } else {
                    parse_int(a).0
                };
            }
            "close" => self.sc_close(app, a),
            "movewin" => {
                let (i, r) = self.window_arg(a);
                if i <= 0 {
                    self.fail(b"no such window");
                } else {
                    let (x, r) = parse_int(r);
                    let (y, _) = parse_int(r);
                    self.uiw_user_rect(i as u32, x, y, 0, 0);
                    self.run_passes(app);
                }
            }
            "resizewin" => {
                let (i, r) = self.window_arg(a);
                if i < 0 {
                    self.fail(b"no such window");
                } else {
                    let (w, r) = parse_int(r);
                    let (h, _) = parse_int(r);
                    if i == 0 {
                        // the main window: like `size`
                        let (w, h) = (self.dp(w), self.dp(h));
                        self.resize(app, w, h);
                    } else {
                        self.uiw_user_rect(i as u32, i32::MIN, 0, w, h);
                        self.run_passes(app);
                    }
                }
            }
            "activate" => {
                let (i, _) = self.window_arg(a);
                if i < 0 {
                    self.fail(b"no such window");
                } else {
                    let id = self.uiw_rec_id(i as u32);
                    self.window_activate(id);
                    self.run_passes(app);
                }
            }
            "scale" => self.sc_scale(app, a),
            "screen" => {
                // (§6) the virtual screen: X Y W H (DIP) or off
                self.m.win.screen = if word_eq(a, "off").is_some() {
                    [0; 4]
                } else {
                    let (x, r) = parse_int(a);
                    let (y, r) = parse_int(r);
                    let (w, r) = parse_int(r);
                    let (h, _) = parse_int(r);
                    [x, y, w.max(0), h.max(0)]
                };
                self.invalidate();
                self.run_passes(app);
            }
            "sync" => self.sc_sync(app, a),
            "style" => {
                // another linked style (Ui::set_style)
                let name = String::from_utf8_lossy(skip_ws(a)).into_owned();
                if self.set_style(name.trim()) {
                    self.run_passes(app);
                } else {
                    self.fail(b"no such style");
                }
            }
            "perm" => self.sc_perm(app, a),   // auto_perm.rs
            "agent" => self.sc_agent(app, a), // auto_perm.rs
            "inspect" => self.sc_inspect(app, a), // inspect.rs
            "expect_warn" => {
                let (w, m) = (self.warns, self.sc.warn_mark);
                self.sc.warn_mark = w;
                if w == m {
                    self.fail(b"no warning");
                }
            }
            "expect_nowarn" => {
                let d = self.warns.wrapping_sub(self.sc.warn_mark);
                self.sc.warn_mark = self.warns;
                if d != 0 {
                    self.fail(alloc::format!("{} warnings", d as i32).as_bytes());
                }
            }
            "focus" => match self.resolve(a).0 {
                None => self.sel_fail(),
                Some(s) => {
                    let id = self.sel_rec(s).id;
                    if id == 0 {
                        self.fail(b"the widget has no id");
                    } else {
                        self.focus(id);
                        self.run_passes(app);
                    }
                }
            },
            "drop" => self.sc_drop(app, a),
            "dragover" => self.sc_dragover(app, a, false),
            "dragleave" => self.sc_dragover(app, a, true),
            "dragout" => self.sc_dragout(app, a),
            "clip" => self.sc_clip(app, a),
            "pointer" => self.sc_pointer(a),
            _ => {}
        }
        false
    }

    // the record of a resolved selector (a point: the zero record)
    fn sel_rec(&self, s: Sel) -> Sem {
        match s {
            Sel::Rec(i) => self.sems[i].clone(),
            Sel::Point => Sem::default(),
        }
    }

    // ------------------------------------------------------------------ selectors

    // resolve(p) -> (the record (or a point), the rest after the selector); None: sel_err
    // holds the reason (1 none, 2 ambiguous, 3 syntax).  The point to use is left in
    // sel_x / sel_y (px).
    pub(crate) fn resolve<'a>(&mut self, p: &'a [u8]) -> (Option<Sel>, &'a [u8]) {
        self.sc.sel_err = 3;
        self.sc.sel_kind = -1;
        self.sc.sel_wf = self.scope_index(); // window filter (-1 = any)
        self.sc.sel_win = 0;
        let c0 = p.first().copied().unwrap_or(0);
        if c0 == b'#' {
            return self.resolve_id(p);
        }
        let mut q = p;
        if c0 != b'"' {
            if c0.is_ascii_digit() || c0 == b'-' {
                return self.resolve_point(p);
            }
            // kind:"Label"
            let mut found = false;
            for (k, n) in KIND_NAMES.iter().enumerate() {
                if p.starts_with(n.as_bytes()) && p.get(n.len()) == Some(&b':') {
                    self.sc.sel_kind = k as i32;
                    q = &p[n.len() + 1..];
                    found = true;
                    break;
                }
            }
            if !found || q.first() != Some(&b'"') {
                self.sc.sel_err = 3;
                return (None, p);
            }
        }
        self.resolve_label(p, q)
    }

    // a point of the scoped window (the main window for "any")
    fn resolve_point<'a>(&mut self, p: &'a [u8]) -> (Option<Sel>, &'a [u8]) {
        let (x, r) = parse_int(p);
        if r.first() != Some(&b',') {
            self.sc.sel_err = 3;
            return (None, p);
        }
        let (y, r) = parse_int(&r[1..]);
        let w = self.sc.sel_wf.max(0) as u32;
        self.sc.sel_win = w;
        let dpi = self.win_dpi(w);
        self.sc.sel_x = dp_at(x, dpi);
        self.sc.sel_y = dp_at(y, dpi);
        (Some(Sel::Point), r)
    }

    // sel_win_ok(rec): the record passes the window filter sel_wf
    fn sel_win_ok(&self, r: &Sem) -> bool {
        self.sc.sel_wf < 0 || self.sc.sel_wf as u8 == r.win
    }

    fn resolve_id<'a>(&mut self, p: &'a [u8]) -> (Option<Sel>, &'a [u8]) {
        let q = &p[1..];
        let (id, rest) = if q.len() >= 2 && q[0] == b'0' && q[1] == b'x' {
            parse_hex(q)
        } else {
            let (v, r) = parse_int(q);
            (v as u32, r)
        };
        // windows that have this id: one expected
        let mut mask = 0u32;
        for r in self.sems.iter().rev() {
            if r.id == id && self.sel_win_ok(r) {
                mask |= 1 << (r.win as u32 & (WIN_MAX as u32 - 1));
            }
        }
        if mask == 0 {
            self.sc.sel_err = 1;
            return (None, rest);
        }
        if mask & (mask - 1) != 0 {
            self.warn(&alloc::format!("duplicate id {} in several windows (mask {:x})", id, mask));
            self.sc.sel_wf = self.win_topmost(mask) as i32;
        }
        // last record with this id
        let found = (0..self.sems.len()).rev().find(|&i| self.sems[i].id == id && self.sel_win_ok(&self.sems[i]));
        match found {
            Some(i) => {
                self.sel_found(i);
                (Some(Sel::Rec(i)), rest)
            }
            None => {
                self.sc.sel_err = 1;
                (None, rest)
            }
        }
    }

    fn label_match(&self, r: &Sem) -> bool {
        let Some(l) = r.label.as_deref() else { return false };
        if !self.sel_win_ok(r) {
            return false;
        }
        if self.sc.sel_kind >= 0 && r.kind as i32 != self.sc.sel_kind {
            return false;
        }
        l.as_bytes() == self.sc.sel_label.as_slice()
    }

    fn resolve_label<'a>(&mut self, p: &'a [u8], q: &'a [u8]) -> (Option<Sel>, &'a [u8]) {
        let Some((label, rest)) = parse_qstring(q, 256) else {
            self.sc.sel_err = 3;
            return (None, p);
        };
        self.sc.sel_label = label;
        // matches in several windows: only the topmost window counts
        let mask = self.label_windows();
        if mask & mask.wrapping_sub(1) != 0 {
            self.sc.sel_wf = self.win_topmost(mask) as i32;
        }
        // count matches (all) and non-text matches
        let (mut n, mut nt) = (0, 0);
        let (mut last, mut last_nt) = (0, 0);
        for i in 0..self.sems.len() {
            if !self.label_match(&self.sems[i]) {
                continue;
            }
            n += 1;
            last = i;
            if self.sems[i].kind != K_TEXT {
                nt += 1;
                last_nt = i;
            }
        }
        if n == 0 {
            self.sc.sel_err = 1;
            return (None, rest);
        }
        let pick = if n == 1 {
            Some(last)
        } else if nt == 1 {
            Some(last_nt)
        } else {
            // still ambiguous: a single match on the topmost layer wins (dialogs and popups
            // cover everything below them)
            self.top_layer_match()
        };
        match pick {
            Some(i) => {
                self.sel_found(i);
                (Some(Sel::Rec(i)), rest)
            }
            None => {
                self.sc.sel_err = 2;
                (None, rest)
            }
        }
    }

    fn sel_found(&mut self, i: usize) {
        let r = &self.sems[i];
        self.sc.sel_err = 0;
        self.sc.sel_win = r.win as u32 & (WIN_MAX as u32 - 1);
        self.sc.sel_x = (r.rect[2] >> 1).wrapping_add(r.rect[0]);
        self.sc.sel_y = (r.rect[3] >> 1).wrapping_add(r.rect[1]);
    }

    // top_layer_match -> the only non-text label match on the highest layer that has
    // matches
    fn top_layer_match(&self) -> Option<usize> {
        let mut top = -1;
        let mut n = 0;
        let mut m = 0;
        for i in 0..self.sems.len() {
            let r = &self.sems[i];
            if r.kind == K_TEXT || !self.label_match(r) {
                continue;
            }
            let l = r.layer as i32;
            if l < top {
                continue;
            }
            if l > top {
                top = l;
                n = 0;
            }
            n += 1;
            m = i;
        }
        if n == 1 { Some(m) } else { None }
    }

    // label_windows -> bit mask of the windows with a match (controls; plain text only
    // when there is no control)
    fn label_windows(&self) -> u32 {
        let (mut ctl, mut any) = (0u32, 0u32);
        for r in &self.sems {
            if !self.label_match(r) {
                continue;
            }
            let b = 1 << (r.win as u32 & (WIN_MAX as u32 - 1));
            any |= b;
            if r.kind != K_TEXT {
                ctl |= b;
            }
        }
        if ctl != 0 { ctl } else { any }
    }

    // win_topmost(mask) -> the topmost of the windows: the modal window, else the highest
    // stacking order
    fn win_topmost(&self, mask: u32) -> u32 {
        let modal = self.m.win.modal;
        if modal != 0 && mask & (1 << modal) != 0 {
            return modal;
        }
        let mut best = 0;
        let mut z: i64 = -1;
        for i in 0..WIN_MAX as u32 {
            if mask & (1 << i) == 0 {
                continue;
            }
            let wz = self.m.win.recs[i as usize].z as i32 as i64;
            if wz > z {
                z = wz;
                best = i;
            }
        }
        best
    }

    // scope_index -> window index of the `window` scope, -1 = any, -2 = unknown window
    fn scope_index(&self) -> i32 {
        if self.sc.sc_scope == -1 {
            return -1;
        }
        let i = self.uiw_index(self.sc.sc_scope as u32);
        if i < 0 { -2 } else { i }
    }

    // window_arg(p) -> (window index for "ID" / "main" (-1 = unknown or closed), rest)
    fn window_arg<'a>(&self, p: &'a [u8]) -> (i32, &'a [u8]) {
        let p = skip_ws(p);
        if let Some(r) = word_eq(p, "main") {
            return (0, r);
        }
        let (id, r) = parse_int(p);
        let i = self.uiw_index(id as u32);
        if i < 0 {
            return (i, r);
        }
        if self.m.win.recs[i as usize].state == 0 {
            return (-1, r);
        }
        (i, r)
    }

    // ------------------------------------------------------------------ input injection

    // inject(type, btn, x, y, data): queue one event (current mods) and process it; keys
    // and text go to the keyboard window, pointer events to the script's window
    // system focus in | out, suspend, resume, session-end, settings theme | locale | contrast |
    // fonts | N: the lifecycle events of the platform (UIC_ACTIVE, UIC_SUSPEND, ...)
    fn sc_system(&mut self, app: &mut dyn App, a: &[u8]) {
        let w = |n: &str| word_eq(a, n).is_some();
        let (typ, data) = if w("focus") {
            let r = next_word(a);
            (if word_eq(r, "out").is_some() { plat::E_FOCUS_OUT } else { plat::E_FOCUS_IN }, 0)
        } else if w("suspend") {
            (plat::E_POWER, 1)
        } else if w("resume") {
            (plat::E_POWER, 0)
        } else if w("session-end") {
            (plat::E_SESSION_END, 0)
        } else if w("settings") {
            let r = next_word(a);
            let bits = [("theme", plat::SET_THEME), ("locale", plat::SET_LOCALE), ("contrast", plat::SET_CONTRAST), ("fonts", plat::SET_FONTS)]
                .iter()
                .find(|(n, _)| word_eq(r, n).is_some())
                .map_or_else(|| parse_int(r).0 as u32, |(_, b)| *b);
            (plat::E_SETTINGS, bits)
        } else {
            self.fail(b"system: focus in|out, suspend, resume, session-end or settings WHAT");
            return;
        };
        self.inject(app, typ, 0, 0, 0, data);
    }

    fn inject(&mut self, app: &mut dyn App, typ: u8, btn: u32, x: i32, y: i32, data: u32) {
        let w = if matches!(typ, plat::E_KEY_DOWN | plat::E_KEY_UP | plat::E_TEXT) { self.m.win.active } else { self.sc.sc_win };
        let ev = Event {
            typ,
            btn: btn as u8,
            mods: self.sc.script_mods as u8,
            win: (w & (WIN_MAX as u32 - 1)) as u8,
            x,
            y,
            data,
        };
        plat::evq_push(ev);
        self.process_events(app);
    }

    // mouse_to(x, y): move the pointer (px of window sel_win)
    fn mouse_to(&mut self, app: &mut dyn App, x: i32, y: i32) {
        self.sc.sc_win = self.sc.sel_win;
        self.sc.sc_x = x;
        self.sc.sc_y = y;
        let d = self.sc.sc_pdata;
        self.inject(app, plat::E_MOVE, 0, x, y, d);
    }

    // click_here(btn): press + release at the pointer
    fn click_here(&mut self, app: &mut dyn App, btn: u32) {
        let (x, y, d) = (self.sc.sc_x, self.sc.sc_y, self.sc.sc_pdata);
        self.inject(app, plat::E_DOWN, btn, x, y, d);
        let (x, y, d) = (self.sc.sc_x, self.sc.sc_y, self.sc.sc_pdata);
        self.inject(app, plat::E_UP, btn, x, y, d);
    }

    // down S [btn] / up [S] [btn]
    fn sc_button(&mut self, app: &mut dyn App, a: &[u8], typ: u8) {
        let mut rest = a;
        let c = a.first().map(|c| c | 32);
        if !a.is_empty() && !matches!(c, Some(b'l' | b'r' | b'm')) {
            let (s, r) = self.resolve(a);
            if s.is_none() {
                self.sel_fail();
                return;
            }
            let (x, y) = (self.sc.sel_x, self.sc.sel_y);
            self.mouse_to(app, x, y);
            rest = skip_ws(r);
        }
        let b = parse_button(rest);
        let (x, y, d) = (self.sc.sc_x, self.sc.sc_y, self.sc.sc_pdata);
        self.inject(app, typ, b, x, y, d);
    }

    // drag S1 S2 [steps] [words]
    fn sc_drag(&mut self, app: &mut dyn App, a: &[u8]) {
        let mut pen_type = 0u32;
        let (mut p0, mut p1) = (0i32, 0i32);
        let saved_pdata = self.sc.sc_pdata;
        let (s, r) = self.resolve(a);
        if s.is_none() {
            self.sel_fail();
            return;
        }
        let (bx, by) = (self.sc.sel_x, self.sc.sel_y);
        let press_win = self.sc.sel_win; // the window of the press
        let (s, r) = self.resolve(skip_ws(r));
        if s.is_none() {
            self.sel_fail();
            return;
        }
        // the pointer stays captured by the first window
        self.point_to_window(self.sc.sel_win, press_win);
        self.sc.sel_win = press_win;
        let (ex, ey) = (self.sc.sel_x, self.sc.sel_y);
        let mut q = skip_ws(r);
        let mut steps = 8;
        // optional step count
        if q.first().is_some_and(|c| c.is_ascii_digit()) {
            let (n, r) = parse_int(q);
            if n > 0 {
                steps = n;
            }
            q = r;
        }
        // optional words: shift ctrl alt (held during the drag), right / middle (button)
        let mut btn = BTN_LEFT;
        let saved_mods = self.sc.script_mods;
        let mut mods = saved_mods;
        loop {
            let w = skip_ws(q);
            if w.is_empty() {
                break;
            }
            let mut after = w;
            let m = mod_word(w);
            if m != 0 {
                mods |= m;
            } else if w.starts_with(b"right") {
                btn = BTN_RIGHT;
            } else if w.starts_with(b"middle") {
                btn = BTN_MIDDLE;
            } else if w.starts_with(b"pen") || w.starts_with(b"eraser") {
                // (PN1) pen|eraser P0 P1
                pen_type = if w.starts_with(b"pen") { PTR_PEN } else { PTR_ERASER };
                let (a0, r) = parse_pct(next_word(w));
                let (a1, r) = parse_pct(skip_ws(r));
                p0 = a0;
                p1 = a1;
                after = r;
            } else {
                let mut d = b"unknown drag option ".to_vec();
                d.extend_from_slice(w);
                self.fail(&d);
                return;
            }
            // skip the word
            let i = after.iter().position(|&c| c <= 32).unwrap_or(after.len());
            q = &after[i..];
        }
        self.sc.script_mods = mods;
        if pen_type != 0 {
            self.sc.sc_pdata = plat::evq_pen_data(p0, pen_type, 0, 0); // pressure P0 at the press
        }
        self.mouse_to(app, bx, by);
        let d = self.sc.sc_pdata;
        self.inject(app, plat::E_DOWN, btn, bx, by, d);
        for i in 1..=steps {
            if pen_type != 0 {
                // (PN1) P0 + (P1 - P0) * i / steps
                let pr = (p1 - p0).wrapping_mul(i) / steps + p0;
                self.sc.sc_pdata = plat::evq_pen_data(pr, pen_type, 0, 0);
            }
            // p = a + (b - a) * i / steps
            let x = bx + (ex - bx).wrapping_mul(i) / steps;
            let y = by + (ey - by).wrapping_mul(i) / steps;
            self.mouse_to(app, x, y);
            self.advance_time(app, 16);
        }
        let d = self.sc.sc_pdata;
        self.inject(app, plat::E_UP, btn, ex, ey, d);
        self.sc.script_mods = saved_mods;
        self.sc.sc_pdata = saved_pdata; // (PN1) the pointer's data as before
    }

    // wheel S DY [shift|ctrl|alt]...: the words hold those modifiers during the wheel event
    // (not during the move to S)
    fn sc_wheel(&mut self, app: &mut dyn App, a: &[u8]) {
        let (s, r) = self.resolve(a);
        if s.is_none() {
            self.sel_fail();
            return;
        }
        let (dy, r) = parse_int(r);
        let saved_mods = self.sc.script_mods;
        let mut mods = saved_mods;
        let mut q = skip_ws(r);
        while !q.is_empty() {
            let m = mod_word(q);
            if m == 0 {
                let mut d = b"unknown wheel option ".to_vec();
                d.extend_from_slice(q);
                self.fail(&d);
                return;
            }
            mods |= m;
            q = next_word(q);
        }
        let (x, y) = (self.sc.sel_x, self.sc.sel_y);
        self.mouse_to(app, x, y);
        let (sx, sy) = (self.sc.sc_x, self.sc.sc_y);
        self.sc.script_mods = mods;
        self.inject(app, plat::E_WHEEL, 0, sx, sy, dy as u16 as u32);
        self.sc.script_mods = saved_mods;
    }

    // point_to_window(from, to): sel_x / sel_y (px of window `from`) as px of window `to`
    fn point_to_window(&mut self, from: u32, to: u32) {
        if from == to {
            return;
        }
        let dpi = self.win_dpi(from);
        let (dx, dy) = self.uiw_desk_pos(from);
        let x = dx + px2dip_at(self.sc.sel_x, dpi);
        let y = dy + px2dip_at(self.sc.sel_y, dpi);
        let (tx, ty) = self.uiw_desk_pos(to);
        let tdpi = self.win_dpi(to);
        self.sc.sel_x = dp_at(x - tx, tdpi);
        self.sc.sel_y = dp_at(y - ty, tdpi);
    }

    // pointer mouse | pen [P [TX TY]] | eraser [P [TX TY]] | touch -- (PN1) pen data of the
    // following pointer events
    fn sc_pointer(&mut self, a: &[u8]) {
        let names = ["mouse", "pen", "eraser", "touch"];
        let Some(t) = names.iter().position(|n| word_eq(a, n).is_some()) else {
            let mut d = b"unknown pointer type ".to_vec();
            d.extend_from_slice(a);
            self.fail(&d);
            return;
        };
        let mut pressure = -1;
        let (mut tx, mut ty) = (0, 0);
        let q = next_word(a);
        if q.first().is_some_and(|c| c.is_ascii_digit()) {
            let (pv, r) = parse_pct(q);
            pressure = pv;
            let r = skip_ws(r);
            if r.first().is_some_and(|&c| c == b'-' || c.is_ascii_digit()) {
                let (x, r) = parse_int(r);
                let (y, _) = parse_int(skip_ws(r));
                tx = x;
                ty = y;
            }
        }
        self.sc.sc_pdata = plat::evq_pen_data(pressure, t as u32, tx, ty);
    }

    // key CHORD...
    fn sc_key(&mut self, app: &mut dyn App, a: &[u8]) {
        let Some(keys) = self.chords(a) else { return };
        for k in keys {
            self.press(app, k >> 16, k & 0xFFFF);
        }
    }

    // keys [N] CHORD...: the chords N times over, all queued for ONE pass of the window with
    // the keyboard (input faster than frames, the keys a tool window hands on)
    fn sc_keys(&mut self, app: &mut dyn App, a: &[u8]) {
        let (n, r) = if a.first().is_some_and(|c| c.is_ascii_digit()) { parse_int(a) } else { (1, a) };
        let Some(keys) = self.chords(r) else { return };
        let w = self.m.win.active & (WIN_MAX as u32 - 1);
        if w != 0 && !self.uiw_drawable(w) {
            self.fail(b"the window with the keyboard is not shown");
            return;
        }
        let all: Vec<u32> = (0..n.max(0)).flat_map(|_| keys.iter().copied()).collect();
        self.uiw_use(w);
        self.keys_inject(&all);
        self.m.win.edges |= 1 << w; // cleared after the first pass
        self.uiw_use(0);
        self.run_passes(app);
    }

    // chords(a) -> the keys of "ctrl+s tab shift+f10 ..." (VK | MOD_* << 16), None after a
    // FAIL (an unknown key)
    fn chords(&mut self, a: &[u8]) -> Option<Vec<u32>> {
        let mut out = Vec::new();
        let mut p = a;
        loop {
            p = skip_ws(p);
            if p.is_empty() {
                return Some(out);
            }
            let Some((mods, vk, rest)) = chord(p) else {
                self.fail(b"unknown key");
                return None;
            };
            out.push(vk | mods << 16);
            p = rest;
        }
    }

    // a key pressed and released with modifiers
    fn press(&mut self, app: &mut dyn App, mods: u32, vk: u32) {
        self.sc.script_mods = mods;
        self.inject(app, plat::E_KEY_DOWN, 0, 0, 0, vk);
        self.inject(app, plat::E_KEY_UP, 0, 0, 0, vk);
        self.sc.script_mods = 0;
    }

    // commit "text" [N]: the commits of an input method (plat::ime_commit_text) to the window
    // with the keyboard, as the platform queues them
    fn sc_commit(&mut self, app: &mut dyn App, a: &[u8]) {
        let Some((s, r)) = parse_qstring(a, 1024) else {
            self.fail(b"expected a quoted string");
            return;
        };
        let text = String::from_utf8_lossy(&s).repeat(parse_int(r).0.max(1) as usize);
        let w = (self.m.win.active & (WIN_MAX as u32 - 1)) as u8;
        let mut at = 0;
        while at < text.len() {
            let n = plat::ime_commit_text(w, &text[at..]);
            if n == 0 {
                self.fail(b"the commit was not queued");
                return;
            }
            at += n;
            self.process_events(app);
        }
    }

    // type "text"
    fn sc_type(&mut self, app: &mut dyn App, a: &[u8]) {
        let Some((s, _)) = parse_qstring(a, 256) else {
            self.fail(b"expected a quoted string");
            return;
        };
        let mut i = 0;
        loop {
            let (cp, n) = utf8_decode(&s, i);
            if cp == 0 {
                break;
            }
            i = n;
            if cp == 10 {
                self.inject(app, plat::E_KEY_DOWN, 0, 0, 0, VK_RETURN);
                self.inject(app, plat::E_KEY_UP, 0, 0, 0, VK_RETURN);
            } else {
                self.inject(app, plat::E_TEXT, 0, 0, 0, cp);
            }
        }
    }

    // advance_time(ms): run the virtual clock in 16 ms steps (timers, animations)
    fn advance_time(&mut self, app: &mut dyn App, ms: i32) {
        let mut left = ms;
        while left > 0 {
            let step = left.min(16);
            left -= step;
            let t = self.now_ms.wrapping_add(step as u32);
            self.set_time(t);
            let mut fired = self.tasks_ready(); // a task's sleep() ended (tasks.rs)
            loop {
                let (id, _) = self.timer_due();
                if id == u32::MAX {
                    break;
                }
                self.cmd_run(UIC_TIMER, id);
                fired = true;
            }
            if self.animating || fired {
                self.run_passes(app);
            }
        }
    }

    // shot FILE [window ID|desktop] [X Y W H]
    fn sc_shot(&mut self, app: &mut dyn App, a: &[u8]) {
        let _ = app;
        // the file name is the first word
        let i = a.iter().position(|&c| c == b' ' || c == b'\t').unwrap_or(a.len());
        let file = String::from_utf8_lossy(&a[..i]).into_owned();
        let mut rest = skip_ws(if i < a.len() { &a[i + 1..] } else { &a[i..] });
        let mut win = 0u32;
        let mut desktop = false;
        if let Some(r) = word_eq(rest, "desktop") {
            desktop = true;
            rest = r;
        } else if let Some(r) = word_eq(rest, "window") {
            let (w, r) = self.window_arg(r);
            if w < 0 {
                self.fail(b"no such window");
                return;
            }
            win = w as u32;
            rest = r;
        }
        let mut crop = None;
        let q = skip_ws(rest);
        if !q.is_empty() {
            let (x, r) = parse_int(q);
            let (y, r) = parse_int(r);
            let (w, r) = parse_int(r);
            let (h, _) = parse_int(r);
            crop = Some([x, y, w, h]);
        }
        self.render();
        let ok = if desktop {
            self.shot_desktop(&file, crop)
        } else {
            let sf = self.win_fb(win);
            match crop {
                Some(c) => match crop_surface(sf, self.win_dpi(win), c) {
                    Some(v) => Ui::save_png(&v, &file),
                    None => false,
                },
                None => Ui::save_png(sf, &file),
            }
        };
        if !ok {
            self.fail(b"cannot write the screenshot");
            return;
        }
        plat::write_stdout(alloc::format!("wrote {}\n", file).as_bytes());
    }

    // shot_desktop(path, crop) -> written: every shown window on the virtual desktop (main
    // window at 0, 0; px at the main window's dpi), in stacking order; the virtual screen
    // (`screen`) belongs to it
    fn shot_desktop(&mut self, path: &str, crop: Option<[i32; 4]>) -> bool {
        let dpi = self.wp.rt.dpi; // the main window is live
        let mut bb = [0, 0, px2dip_at(self.wp.rt.fb.w, dpi), px2dip_at(self.wp.rt.fb.h, dpi)];
        let scr = self.m.win.screen;
        if scr[2] > 0 && scr[3] > 0 {
            bb = [bb[0].min(scr[0]), bb[1].min(scr[1]), bb[2].max(scr[0] + scr[2]), bb[3].max(scr[1] + scr[3])];
        }
        for i in 1..WIN_MAX as u32 {
            if !self.uiw_drawable(i) {
                continue;
            }
            let r = self.win_desk_rect(i);
            bb[0] = bb[0].min(r[0]);
            bb[1] = bb[1].min(r[1]);
            bb[2] = bb[2].max(r[0] + r[2]);
            bb[3] = bb[3].max(r[1] + r[3]);
        }
        let mut sf = Surface::new(dp_at(bb[2] - bb[0], dpi), dp_at(bb[3] - bb[1], dpi), FMT_XRGB);
        if sf.px.is_empty() {
            return false;
        }
        let mut dc = gfx::Dc::new(sf.w, sf.h);
        dc.set_color(0xFF5A_6B7C); // desktop
        let (sw, sh) = (sf.w, sf.h);
        dc.fill(&mut sf, 0, 0, sw, sh);
        // windows bottom to top: the main window, then by stacking order
        self.sd_blit(&mut sf, 0, bb[0], bb[1], dpi);
        let mut last_z: i64 = -1;
        loop {
            let mut next = None;
            let mut nz: i64 = i32::MAX as i64;
            for i in 1..WIN_MAX as u32 {
                if !self.uiw_drawable(i) {
                    continue;
                }
                let z = self.m.win.recs[i as usize].z as i32 as i64;
                if z > last_z && z < nz {
                    nz = z;
                    next = Some(i);
                }
            }
            let Some(i) = next else { break };
            last_z = nz;
            self.sd_blit(&mut sf, i, bb[0], bb[1], dpi);
            self.uiw_popups_blit(&mut sf, i, bb[0], bb[1], dpi); // (U3) its popups
        }
        self.uiw_popups_blit(&mut sf, 0, bb[0], bb[1], dpi); // popups of the main window above all
        match crop {
            Some(c) => match crop_surface(&sf, dpi, c) {
                Some(v) => Ui::save_png(&v, path),
                None => false,
            },
            None => Ui::save_png(&sf, path),
        }
    }

    // sd_blit(sf, i, x0, y0, dpi): copy a window's framebuffer to its place on the desktop
    fn sd_blit(&mut self, sf: &mut Surface, i: u32, x0: i32, y0: i32, dpi: i32) {
        let r = self.win_desk_rect(i);
        let x = dp_at(r[0] - x0, dpi);
        let y = dp_at(r[1] - y0, dpi);
        blit_copy(sf, self.win_fb(i), x, y);
    }

    // win_desk_rect(i) -> the window's outer rectangle on the virtual desktop (DIP)
    fn win_desk_rect(&self, i: u32) -> [i32; 4] {
        let (x, y) = self.uiw_desk_pos(i);
        let dpi = self.win_dpi(i);
        let fb = self.win_fb(i);
        [x, y, px2dip_at(fb.w, dpi), px2dip_at(fb.h, dpi)]
    }

    // expect S condition [arg]  |  expect window ID condition
    fn sc_expect(&mut self, a: &[u8]) {
        if let Some(r) = word_eq(a, "window") {
            self.expect_window(r);
            return;
        }
        if let Some(r) = word_eq(a, "sound") {
            // the last sound ui.beep / ui.dialog_sound asked for since the last check
            let want = skip_ws(r);
            let got = self.m.sound.last.take();
            let name = got.map_or("none", plat::audio::sound_name);
            if !want.starts_with(name.as_bytes()) || (want.len() > name.len() && want[name.len()] > b' ') {
                self.fail(alloc::format!("sound {name}").as_bytes());
            }
            return;
        }
        // app themes and styles (theme.rs)
        if let Some(r) = word_eq(a, "theme") {
            self.expect_theme(skip_ws(r));
            return;
        }
        if let Some(r) = word_eq(a, "style") {
            let want = String::from_utf8_lossy(skip_ws(r));
            let have = self.style_name();
            if want.trim() != have {
                self.fail(alloc::format!("style is {}", have).as_bytes());
            }
            return;
        }
        if let Some(r) = word_eq(a, "token") {
            self.expect_token(skip_ws(r));
            return;
        }
        if let Some(r) = word_eq(a, "pixel") {
            self.expect_pixel(r);
            return;
        }
        if let Some(r) = word_eq(a, "same") {
            self.expect_same(skip_ws(r));
            return;
        }
        let (s, rest) = self.resolve(a);
        if s.is_none() && self.sc.sel_err == 3 {
            self.sel_fail(); // bad selector syntax: report and stop
            return;
        }
        let cond = skip_ws(rest);
        // existence tests work with a missing widget
        if word_eq(cond, "missing").is_some() {
            if s.is_some() {
                self.fail(b"widget exists");
            }
            return;
        }
        let Some(s) = s else {
            self.sel_fail();
            return;
        };
        let rec = self.sel_rec(s);
        const STATES: [(&str, u16, bool); 9] = [
            ("hover", SS_HOVER, true),
            ("focused", SS_FOCUSED, true),
            ("checked", SS_CHECKED, true),
            ("unchecked", SS_CHECKED, false),
            ("disabled", SS_DISABLED, true),
            ("enabled", SS_DISABLED, false),
            ("selected", SS_SELECTED, true),
            ("open", SS_OPEN, true),
            ("pressed", SS_PRESSED, true),
        ];
        for (n, mask, want) in STATES {
            if word_eq(cond, n).is_some() {
                let set = rec.state & mask != 0;
                if set != want {
                    self.fail(if set { b"state is set" } else { b"state is not set" });
                }
                return;
            }
        }
        if let Some(r) = word_eq(cond, "value") {
            if parse_int(r).0 != rec.value {
                self.fail(alloc::format!("value is {}", rec.value).as_bytes());
            }
        } else if let Some(r) = word_eq(cond, "max") {
            if parse_int(r).0 != rec.max {
                self.fail(alloc::format!("max is {}", rec.max).as_bytes());
            }
        } else if let Some(r) = word_eq(cond, "text") {
            let Some((t, _)) = parse_qstring(skip_ws(r), 256) else {
                self.fail(b"expected a quoted string");
                return;
            };
            let have = rec.label.as_deref().unwrap_or("");
            if have.as_bytes() != t.as_slice() {
                let mut d = b"text is \"".to_vec();
                d.extend_from_slice(have.as_bytes());
                d.push(b'"');
                self.fail(&d);
            }
        } else if word_eq(cond, "exists").is_some() {
        } else if let Some(r) = word_eq(cond, "rect") {
            self.expect_rect(r, &rec);
        } else {
            self.fail(b"unknown condition");
        }
    }

    // expect_rect(p, rec): "X Y W H" -- the record's rectangle in DIP (its window's dpi); a
    // '*' takes any value (a theme's control height: "rect * * * 36")
    fn expect_rect(&mut self, p: &[u8], rec: &Sem) {
        let mut want = [None; 4];
        let mut q = p;
        for v in want.iter_mut() {
            q = skip_ws(q);
            if q.first() == Some(&b'*') {
                q = &q[1..];
                continue;
            }
            let (n, r) = parse_int(q);
            *v = Some(n);
            q = r;
        }
        let dpi = self.win_dpi(rec.win as u32);
        let have = rec.rect.map(|v| px2dip_at(v, dpi));
        if have.iter().zip(want).any(|(h, w)| w.is_some_and(|w| w != *h)) {
            self.fail_more(alloc::format!("rect is {} {}", have[0], have[1]).as_bytes());
            plat::write_stdout(alloc::format!(" {} {}\n", have[2], have[3]).as_bytes());
        }
    }

    // expect theme "NAME" | none | error "TEXT" | error none: the app theme (theme.rs), the
    // error of the last `theme file`
    fn expect_theme(&mut self, p: &[u8]) {
        if let Some(r) = word_eq(p, "error") {
            let r = skip_ws(r);
            let have = self.sc.theme_err.clone();
            if word_eq(r, "none").is_some() {
                if let Some(h) = have {
                    self.fail(alloc::format!("error: {}", h).as_bytes());
                }
                return;
            }
            let Some((t, _)) = parse_qstring(r, 256) else {
                self.fail(b"expected a quoted string");
                return;
            };
            let t = String::from_utf8_lossy(&t).into_owned();
            match have {
                Some(h) if h.contains(&t) => {}
                Some(h) => self.fail(alloc::format!("error: {}", h).as_bytes()),
                None => self.fail(b"no error"),
            }
            return;
        }
        let have = self.app_theme().map(|t| t.name.clone());
        if word_eq(p, "none").is_some() {
            if let Some(h) = have {
                self.fail(alloc::format!("theme is \"{}\"", h).as_bytes());
            }
            return;
        }
        let want = match parse_qstring(p, 256) {
            Some((t, _)) => String::from_utf8_lossy(&t).into_owned(),
            None => String::from(String::from_utf8_lossy(p).trim()),
        };
        match have {
            Some(h) if h == want => {}
            Some(h) => self.fail(alloc::format!("theme is \"{}\"", h).as_bytes()),
            None => self.fail(b"no theme"),
        }
    }

    // expect token NAME #RRGGBB | #AARRGGBB: a colour token of the theme now in use
    fn expect_token(&mut self, p: &[u8]) {
        let name = String::from_utf8_lossy(&p[..p.iter().position(|&c| c == b' ' || c == b'\t').unwrap_or(p.len())]).into_owned();
        let want = super::theme::parse_color(String::from_utf8_lossy(next_word(p)).trim());
        let (Some(i), Some(want)) = (super::theme::token_index(&name), want) else {
            self.fail(b"expect token NAME #RRGGBB | #AARRGGBB");
            return;
        };
        let have = self.color(i as u32 * 4);
        if have != want {
            self.fail(alloc::format!("token is #{:08X}", have).as_bytes());
        }
    }

    // expect pixel X Y #RRGGBB: the colour of the main window's pixel at (X, Y) DIP, rendered now
    fn expect_pixel(&mut self, p: &[u8]) {
        let (x, r) = parse_int(p);
        let (y, r) = parse_int(r);
        let Some(want) = super::theme::parse_color(String::from_utf8_lossy(skip_ws(r)).trim()) else {
            self.fail(b"expect pixel X Y #RRGGBB");
            return;
        };
        self.render();
        let dpi = self.win_dpi(0);
        let sf = self.win_fb(0);
        let (px, py) = (dp_at(x, dpi), dp_at(y, dpi));
        if px < 0 || py < 0 || px >= sf.w || py >= sf.h {
            self.fail(b"outside the window");
            return;
        }
        let o = py as usize * sf.stride as usize + px as usize * 4;
        let have = u32::from_le_bytes([sf.px[o], sf.px[o + 1], sf.px[o + 2], sf.px[o + 3]]) & 0xFF_FFFF;
        if have != want & 0xFF_FFFF {
            self.fail(alloc::format!("pixel is #{:06X}", have).as_bytes());
        }
    }

    // expect same FILE.png: the main window rendered now is the shot saved in FILE (the same
    // PNG bytes: the same pixels)
    fn expect_same(&mut self, p: &[u8]) {
        let path = String::from(String::from_utf8_lossy(p).trim());
        self.render();
        let sf = self.win_fb(0);
        let png = img_png::encode(&sf.px, sf.w as usize, sf.h as usize, sf.stride as usize, false);
        let file = super::perm::trusted(|| plat::read_file(&path));
        match file {
            None => self.fail(b"no such file"),
            Some(f) if f != png => self.fail(b"the pixels differ"),
            Some(_) => {}
        }
    }

    // expect_window(p): "ID open|closed|visible|active|modal|keys" or "ID rect X Y W H"
    fn expect_window(&mut self, p: &[u8]) {
        let (i, r) = self.window_arg(p);
        let cond = skip_ws(r);
        let mut st = 0u32; // bit 0 visible, 1 active, 2 modal, 3 gets the keys (kbnav.rs)
        if i >= 0 && self.uiw_drawable(i as u32) {
            st |= 1;
            if i as u32 == self.m.win.active {
                st |= 2;
            }
            if i != 0 && i as u32 == self.m.win.modal {
                st |= 4;
            }
            if i as u32 == self.uiw_keys_window() {
                st |= 8;
            }
        }
        const STATES: [(&str, u32, bool); 6] =
            [("open", 1, true), ("closed", 1, false), ("visible", 1, true), ("active", 2, true), ("modal", 4, true), ("keys", 8, true)];
        for (n, mask, want) in STATES {
            if word_eq(cond, n).is_some() {
                let set = st & mask != 0;
                if set != want {
                    self.fail(if set { b"window state is set" } else { b"window state is not set" });
                }
                return;
            }
        }
        if let Some(r) = word_eq(cond, "rect") {
            if i < 0 {
                self.fail(b"no such window");
                return;
            }
            let rect = self.m.win.recs[i as usize].rect.get(); // ui-win: the WINRECT (the app's)
            let have = [rect.x, rect.y, rect.w, rect.h];
            let mut q = r;
            for &h in have.iter() {
                let (n, rr) = parse_int(q);
                q = rr;
                if n != h {
                    self.fail_more(alloc::format!("rect is {} {}", rect.x, rect.y).as_bytes());
                    plat::write_stdout(alloc::format!(" {} {}\n", rect.w, rect.h).as_bytes());
                    return;
                }
            }
            return;
        }
        self.fail(b"unknown window condition");
    }

    // close window ID: the user closes the window (E_CLOSE of that window)
    fn sc_close(&mut self, app: &mut dyn App, a: &[u8]) {
        let Some(r) = word_eq(a, "window") else {
            self.fail(b"expected: close window ID");
            return;
        };
        let (i, _) = self.window_arg(r);
        if i < 0 {
            self.fail(b"no such window");
            return;
        }
        self.sc.sc_win = i as u32;
        self.inject(app, plat::E_CLOSE, 0, 0, 0, 0);
        self.run_passes(app);
    }

    // scale P [window ID]: dpi of the main window (its DIP size is kept) or of one window
    fn sc_scale(&mut self, app: &mut dyn App, a: &[u8]) {
        let (p, r) = parse_int(a);
        let dpi = super::runtime::scale_to_dpi(p);
        let q = skip_ws(r);
        if let Some(r) = word_eq(q, "window") {
            let (i, _) = self.window_arg(r);
            if i < 0 {
                self.fail(b"no such window");
                return;
            }
            if i != 0 {
                self.uiw_dpi(i as u32, dpi);
                self.run_passes(app);
                return;
            }
        }
        // keep the client size in DIP
        let w = self.px2dip(self.wp.rt.fb.w);
        let h = self.px2dip(self.wp.rt.fb.h);
        self.uiw_dpi(0, dpi);
        let (w, h) = (self.dp(w), self.dp(h));
        self.resize(app, w, h);
        self.run_passes(app);
    }

    // sync [MS]: wait (real time, default 10 s) until no background job is pending (the app's
    // hook, async tasks: tasks.rs)
    fn sc_sync(&mut self, app: &mut dyn App, a: &[u8]) {
        let (ms, _) = parse_int(a);
        let ms = if ms > 0 { ms } else { 10000 };
        let deadline = plat::time_us() + ms as u64 * 1000;
        while self.uiw_sync_pending() || self.tasks_busy() || self.spell_busy() {
            // headless: plat::wake() from worker threads arrives through the wait object;
            // queue it like a window's E_WAKE (also keeps the loop from spinning)
            plat::wait_events(plat::time_us() + 2_000, &mut |e| {
                plat::evq_push(*e);
            });
            self.process_events(app);
            self.run_passes(app);
            if plat::time_us() >= deadline {
                self.fail(b"background jobs still running");
                break;
            }
        }
        self.process_events(app); // completions (E_WAKE) reach App::command
        self.spell_poll(); // (their wake-up may still wait in the wait object)
        self.run_passes(app);
    }

    // bench N: time N frames: build (passes), full render, partial render (no change);
    // bench N anim | drag S1 S2 [opts] | wheel S DY [mods] | key CHORD | type "s", each
    // [MAX_US]: frames that move
    fn sc_bench(&mut self, app: &mut dyn App, a: &[u8]) {
        let (n, r) = parse_int(a);
        let w = skip_ws(r);
        if !w.is_empty() {
            self.sc_bench_moving(app, n.max(1) as usize, w);
            return;
        }
        let n = if n > 0 { n as u64 } else { 100 };
        let t0 = plat::time_us();
        for _ in 0..n {
            self.run_passes(app);
        }
        let build = plat::time_us() - t0;
        let t0 = plat::time_us();
        for _ in 0..n {
            self.dirty_force_full();
            self.render();
        }
        let full = plat::time_us() - t0;
        let t0 = plat::time_us();
        for _ in 0..n {
            self.run_passes(app);
            self.render();
        }
        let idle = plat::time_us() - t0;
        let s = alloc::format!(
            "bench: frame build {} us, full render {} us, unchanged frame {} us ({}x{} px)\n",
            build / n,
            full / n,
            idle / n,
            self.wp.rt.fb.w,
            self.wp.rt.fb.h
        );
        plat::write_stdout(s.as_bytes());
    }

    // bench N anim | drag S1 S2 [middle|right|shift|ctrl|alt]... | wheel S DY [shift|ctrl|alt]... |
    // key CHORD | type "s", each with an optional MAX_US: n frames that move per run, up to three
    // runs with a limit (the first whose median is at most MAX_US passes)
    fn sc_bench_moving(&mut self, app: &mut dyn App, n: usize, a: &[u8]) {
        enum Kind {
            Anim,
            Drag { from: (i32, i32), to: (i32, i32), btn: u32, mods: u32, win: u32 },
            Wheel { at: (i32, i32), dy: i32, mods: u32, win: u32 },
            Key { mods: u32, vk: u32 },
            Type(Vec<u32>),
        }
        // the words of a drag / wheel up to the limit (a number)
        let opts = |r: &[u8], drag: bool| -> Result<(u32, u32, usize), Vec<u8>> {
            let (mut btn, mut mods, mut q) = (BTN_LEFT, 0, r);
            loop {
                q = skip_ws(q);
                if q.is_empty() || q[0].is_ascii_digit() {
                    return Ok((btn, mods, r.len() - q.len()));
                }
                let m = mod_word(q);
                if m != 0 {
                    mods |= m;
                } else if drag && word_eq(q, "right").is_some() {
                    btn = BTN_RIGHT;
                } else if drag && word_eq(q, "middle").is_some() {
                    btn = BTN_MIDDLE;
                } else {
                    let mut d = b"unknown bench option ".to_vec();
                    d.extend_from_slice(q);
                    return Err(d);
                }
                let i = q.iter().position(|&c| c == b' ' || c == b'\t').unwrap_or(q.len());
                q = &q[i..];
            }
        };
        let (kind, rest): (Kind, &[u8]) = if let Some(r) = word_eq(a, "anim") {
            (Kind::Anim, r)
        } else if let Some(r) = word_eq(a, "drag") {
            let (s, r) = self.resolve(skip_ws(r));
            if s.is_none() {
                self.sel_fail();
                return;
            }
            let (from, win) = ((self.sc.sel_x, self.sc.sel_y), self.sc.sel_win);
            let (s, r) = self.resolve(skip_ws(r));
            if s.is_none() {
                self.sel_fail();
                return;
            }
            self.point_to_window(self.sc.sel_win, win);
            let to = (self.sc.sel_x, self.sc.sel_y);
            match opts(r, true) {
                Ok((btn, mods, k)) => (Kind::Drag { from, to, btn, mods, win }, &r[k..]),
                Err(d) => {
                    self.fail(&d);
                    return;
                }
            }
        } else if let Some(r) = word_eq(a, "wheel") {
            let (s, r) = self.resolve(skip_ws(r));
            if s.is_none() {
                self.sel_fail();
                return;
            }
            let (at, win) = ((self.sc.sel_x, self.sc.sel_y), self.sc.sel_win);
            let (dy, r) = parse_int(skip_ws(r));
            match opts(r, false) {
                Ok((_, mods, k)) => (Kind::Wheel { at, dy, mods, win }, &r[k..]),
                Err(d) => {
                    self.fail(&d);
                    return;
                }
            }
        } else if let Some(r) = word_eq(a, "key") {
            let Some((mods, vk, r)) = chord(skip_ws(r)) else {
                self.fail(b"unknown key");
                return;
            };
            (Kind::Key { mods, vk }, r)
        } else if let Some(r) = word_eq(a, "type") {
            let Some((s, r)) = parse_qstring(skip_ws(r), 256) else {
                self.fail(b"expected a quoted string");
                return;
            };
            (Kind::Type(String::from_utf8_lossy(&s).chars().map(|c| c as u32).collect()), r)
        } else {
            self.fail(b"expected: bench N [anim | drag S1 S2 [options] | wheel S DY [options] | key CHORD | type \"s\"] [MAX_US]");
            return;
        };
        let (max, _) = parse_int(skip_ws(rest));
        let name = match kind {
            Kind::Anim => "anim",
            Kind::Drag { .. } => "drag",
            Kind::Wheel { .. } => "wheel",
            Kind::Key { .. } => "key",
            Kind::Type(_) => "type",
        };
        let runs = if max > 0 { 3 } else { 1 };
        let mut best = u64::MAX;
        for run in 0..runs {
            // (the frame, its render) in us
            let mut t: Vec<(u64, u64)> = Vec::with_capacity(n);
            match kind {
                Kind::Anim => {
                    for _ in 0..n {
                        t.push(self.bench_frame(app, |ui, app| ui.run_passes(app)));
                    }
                }
                Kind::Drag { from, to, btn, mods, win } => {
                    // back and forth: every other run from the end to the start
                    let (a, b) = if run % 2 == 0 { (from, to) } else { (to, from) };
                    let saved = self.sc.script_mods;
                    self.sc.sel_win = win;
                    self.mouse_to(app, a.0, a.1);
                    self.sc.script_mods = mods;
                    let d = self.sc.sc_pdata;
                    self.inject(app, plat::E_DOWN, btn, a.0, a.1, d);
                    for i in 1..=n as i32 {
                        let (x, y) = (a.0 + (b.0 - a.0) * i / n as i32, a.1 + (b.1 - a.1) * i / n as i32);
                        t.push(self.bench_frame(app, |ui, app| ui.mouse_to(app, x, y)));
                    }
                    let d = self.sc.sc_pdata;
                    self.inject(app, plat::E_UP, btn, b.0, b.1, d);
                    self.sc.script_mods = saved;
                }
                Kind::Wheel { at, dy, mods, win } => {
                    self.sc.sel_win = win;
                    self.mouse_to(app, at.0, at.1);
                    let saved = self.sc.script_mods;
                    self.sc.script_mods = mods;
                    for i in 0..n {
                        let v = if i % 2 == 0 { dy } else { -dy };
                        let (x, y) = (self.sc.sc_x, self.sc.sc_y);
                        t.push(self.bench_frame(app, |ui, app| ui.inject(app, plat::E_WHEEL, 0, x, y, v as u16 as u32)));
                    }
                    self.sc.script_mods = saved;
                }
                Kind::Key { mods, vk } => {
                    for _ in 0..n {
                        t.push(self.bench_frame(app, |ui, app| ui.press(app, mods, vk)));
                    }
                }
                Kind::Type(ref cps) => {
                    for _ in 0..n {
                        t.push(self.bench_frame(app, |ui, app| {
                            for &cp in cps {
                                ui.inject(app, plat::E_TEXT, 0, 0, 0, cp);
                            }
                        }));
                    }
                }
            }
            let mut s: Vec<u64> = t.iter().map(|x| x.0).collect();
            s.sort_unstable();
            let (mean, median, top) = (s.iter().sum::<u64>() / s.len() as u64, s[s.len() / 2], s[s.len() - 1]);
            let render = t.iter().map(|x| x.1).sum::<u64>() / t.len() as u64;
            best = best.min(median);
            let o = alloc::format!(
                "bench {}: {} frames, mean {} us (render {} us), median {} us, max {} us ({}x{} px)\n",
                name,
                s.len(),
                mean,
                render,
                median,
                top,
                self.wp.rt.fb.w,
                self.wp.rt.fb.h
            );
            plat::write_stdout(o.as_bytes());
            if max <= 0 || median <= max as u64 {
                return;
            }
        }
        self.fail(alloc::format!("the median stayed above {} us in {} runs (best {} us)", max, runs, best).as_bytes());
    }

    // one timed frame of bench: the clock 16 ms on (due timers queued), f (the input, the
    // passes), the render of the dirty cells -> (us, the render's us); --verify-dirty's
    // comparison follows untimed
    fn bench_frame(&mut self, app: &mut dyn App, f: impl FnOnce(&mut Ui, &mut dyn App)) -> (u64, u64) {
        let verify = self.verify_dirty;
        self.verify_dirty = false;
        let t0 = plat::time_us();
        let t = self.now_ms.wrapping_add(16);
        self.set_time(t);
        loop {
            let (id, _) = self.timer_due();
            if id == u32::MAX {
                break;
            }
            self.cmd_run(UIC_TIMER, id);
        }
        f(self, app);
        let t1 = plat::time_us();
        self.render();
        let t2 = plat::time_us();
        self.verify_dirty = verify;
        if verify {
            self.verify_dirty();
        }
        (t2.saturating_sub(t0), t2.saturating_sub(t1))
    }

    // ------------------------------------------------------------------ (K1) drop / clip

    // k1_word(p, cap) -> (the word ("quoted" or up to a blank; an over-long plain word is cut),
    // the rest after it and its blanks); None = no word
    fn k1_word(p: &[u8], cap: usize) -> Option<(Vec<u8>, &[u8])> {
        let p = skip_ws(p);
        if p.is_empty() {
            return None;
        }
        if p[0] == b'"' {
            let (w, r) = parse_qstring(p, cap)?;
            return Some((w, skip_ws(r)));
        }
        let e = p.iter().position(|&c| c == b' ' || c == b'\t').unwrap_or(p.len());
        let w = p[..e.min(cap - 1)].to_vec();
        Some((w, skip_ws(&p[e..])))
    }

    // drop X Y FILE... | drop X Y [files F...] [text "s"] [html "s"] [url U]... [image F]
    // [copy|move|link] | drop X Y self: dropped on the scoped window (the main window for
    // "any"); `self`: the app's own drag that left its window (dragout) comes back
    fn sc_drop(&mut self, app: &mut dyn App, a: &[u8]) {
        const BAD: &[u8] = b"expected: drop X Y FILE... | drop X Y [files F...] [text \"s\"] [html \"s\"] [url U] [image F] [copy|move|link] | drop X Y self";
        let (x, r) = parse_int(a);
        let (y, r) = parse_int(r);
        let w = self.scope_index().max(0) as u32;
        let dpi = self.win_dpi(w);
        let (px, py) = (dp_at(x, dpi), dp_at(y, dpi));
        let Some(data) = self.sc_drag_data(r) else {
            self.fail(BAD);
            return;
        };
        if data.types() == 0 && word_eq(skip_ws(r), "self").is_none() {
            self.fail(BAD);
            return;
        }
        let n = data.files.len() as u32;
        self.drop_inject(data);
        self.sc.sc_win = w;
        self.inject(app, plat::E_DROP, 0, px, py, n);
        self.run_passes(app);
    }

    // sc_drag_data(words) -> the content of `drop` / `dragover` (None: a bad word): plain
    // words are paths; files / text / html / url / image name a kind; copy / move / link the
    // effect; self nothing
    fn sc_drag_data(&mut self, a: &[u8]) -> Option<DragData> {
        let mut d = DragData::default();
        let mut r = a;
        let mut kind: &[u8] = b"files";
        plat::drop_reset();
        while let Some((word, rest)) = Ui::k1_word(r, 4096) {
            r = rest;
            let w = word.as_slice();
            match w {
                b"files" | b"text" | b"html" | b"url" | b"image" if !matches!(kind, b"text" | b"html" | b"url" | b"image") => {
                    kind = match w {
                        b"files" => b"files",
                        b"text" => b"text",
                        b"html" => b"html",
                        b"url" => b"url",
                        _ => b"image",
                    };
                    continue;
                }
                b"copy" | b"move" | b"link" if kind == b"files" => {
                    d.effects = match w {
                        b"copy" => plat::DE_COPY,
                        b"move" => plat::DE_MOVE,
                        _ => plat::DE_LINK,
                    };
                    continue;
                }
                b"self" if kind == b"files" => continue,
                _ => {}
            }
            let s = String::from_utf8_lossy(w).into_owned();
            match kind {
                b"text" => d.text = Some(s),
                b"html" => d.html = Some(s),
                b"url" => d.urls.push(s),
                b"image" => d.image = Some(super::perm::trusted(|| plat::read_file(&s))?),
                _ => {
                    plat::drop_add(&s);
                }
            }
            kind = b"files"; // one value per kind word; plain words are paths again
        }
        if matches!(kind, b"text" | b"html" | b"url" | b"image") {
            return None; // a kind without its value
        }
        d.files = plat::drop_take();
        Some(d)
    }

    // dragover X Y [files] [text] [url] [html] [image] [copy|move|link] | dragleave: another
    // program's drag hovers over the scoped window (the kinds it offers; default files)
    fn sc_dragover(&mut self, app: &mut dyn App, a: &[u8], leave: bool) {
        let w = self.scope_index().max(0) as u32;
        self.sc.sc_win = w;
        if leave {
            self.inject(app, plat::E_DRAG, 0, -1, -1, 0);
            return;
        }
        let (x, r) = parse_int(a);
        let (y, mut r) = parse_int(r);
        let dpi = self.win_dpi(w);
        let (px, py) = (dp_at(x, dpi), dp_at(y, dpi));
        let (mut types, mut effects) = (0u32, 0u32);
        while let Some((word, rest)) = Ui::k1_word(r, 16) {
            r = rest;
            match word.as_slice() {
                b"files" => types |= plat::DT_FILES,
                b"text" => types |= plat::DT_TEXT,
                b"url" => types |= plat::DT_URL,
                b"html" => types |= plat::DT_HTML,
                b"image" => types |= plat::DT_IMAGE,
                b"copy" => effects |= plat::DE_COPY,
                b"move" => effects |= plat::DE_MOVE,
                b"link" => effects |= plat::DE_LINK,
                _ => {
                    self.fail(b"expected: dragover X Y [files] [text] [url] [html] [image] [copy|move|link]");
                    return;
                }
            }
        }
        if types == 0 {
            types = plat::DT_FILES;
        }
        if effects == 0 {
            effects = plat::DE_COPY | plat::DE_MOVE;
        }
        self.inject(app, plat::E_DRAG, 0, px, py, types | effects << 8);
    }

    // dragout expect none | text "s" | html "s" | url "s" | files N | image | kinds N, dragout
    // end copy|move|link|none: what the app's drag handed to the platform when it left the
    // window (headless), and the platform's answer (E_DRAG_END)
    fn sc_dragout(&mut self, app: &mut dyn App, a: &[u8]) {
        const BAD: &[u8] = b"expected: dragout expect none|text \"s\"|html \"s\"|url \"s\"|files N|image|kinds N | dragout end copy|move|link|none";
        let Some((verb, r)) = Ui::k1_word(a, 16) else {
            self.fail(BAD);
            return;
        };
        if verb == b"end" {
            let e = match Ui::k1_word(r, 16).map(|w| w.0) {
                Some(w) if w == b"copy" => plat::DE_COPY,
                Some(w) if w == b"move" => plat::DE_MOVE,
                Some(w) if w == b"link" => plat::DE_LINK,
                Some(w) if w == b"none" => 0,
                _ => {
                    self.fail(BAD);
                    return;
                }
            };
            self.inject(app, plat::E_DRAG_END, 0, 0, 0, e);
            return;
        }
        if verb != b"expect" {
            self.fail(BAD);
            return;
        }
        let Some((what, r)) = Ui::k1_word(r, 16) else {
            self.fail(BAD);
            return;
        };
        let out = self.dnd_out_data().cloned();
        let arg = Ui::k1_word(r, 4096).map(|w| w.0);
        let got: Result<(), String> = match (what.as_slice(), &out) {
            (b"none", None) => Ok(()),
            (b"none", Some(_)) => Err(String::from("a drag left the window")),
            (_, None) => Err(String::from("no drag left the window")),
            (b"image", Some(d)) => if d.image.is_some() { Ok(()) } else { Err(String::from("no image")) },
            (b"files" | b"kinds", Some(d)) => {
                let want = parse_int(arg.as_deref().unwrap_or(b"")).0;
                let n = if what == b"files" { d.files.len() as i32 } else { d.types() as i32 };
                if n == want { Ok(()) } else { Err(alloc::format!("{n}")) }
            }
            (b"text" | b"html" | b"url", Some(d)) => {
                let want = String::from_utf8_lossy(arg.as_deref().unwrap_or(b"")).into_owned();
                let have = match what.as_slice() {
                    b"text" => d.text.clone(),
                    b"html" => d.html.clone(),
                    _ => d.urls.first().cloned(),
                };
                if have.as_deref() == Some(want.as_str()) { Ok(()) } else { Err(alloc::format!("{have:?}")) }
            }
            _ => Err(String::from("bad kind")),
        };
        if let Err(e) = got {
            self.fail(e.as_bytes());
        }
    }

    // clip text "s" | file F | dib F | clear | save F [bmp] | expect none|text|image|png
    fn sc_clip(&mut self, app: &mut dyn App, a: &[u8]) {
        const BAD: &[u8] = b"expected: clip text \"s\" | file F | dib F | clear | save F [bmp] | expect none|text|image|png";
        let Some((verb, r)) = Ui::k1_word(a, 16) else {
            self.fail(BAD);
            self.run_passes(app);
            return;
        };
        let v4 = |s: &[u8]| verb.len() >= 4 && &verb[..4] == s;
        let v3 = |s: &[u8]| verb.len() >= 3 && &verb[..3] == s;
        if verb == b"data" {
            // clipboard_platform: deterministic external rich-format fixtures.
            let mut rest = r;
            let mut entries = Vec::new();
            let mut valid = true;
            while !rest.is_empty() && entries.len() < 4 {
                let item = Ui::k1_word(rest, 16).and_then(|(kind, next)| {
                    let fmt = match kind.as_slice() { b"text" => CLIP_TEXT, b"html" => CLIP_HTML, b"rtf" => CLIP_RTF, b"docx" => CLIP_DOCX, _ => return None };
                    let (file, next) = Ui::k1_word(next, 1040)?;
                    Some((fmt, super::perm::trusted(|| plat::read_file(&String::from_utf8_lossy(&file)))?, next))
                });
                match item {
                    Some((fmt, data, next)) => { entries.push((fmt, data)); rest = next; }
                    None => { valid = false; break; }
                }
            }
            let refs: Vec<_> = entries.iter().map(|(fmt, data)| (*fmt, data.as_slice())).collect();
            if !valid || !rest.is_empty() || self.clip_set_many(&refs) != 0 {
                self.fail(b"clip data requires one to four text|html|rtf|docx PATH pairs with valid payloads");
            }
        } else if verb == b"utf8" {
            // Opt-in file payloads exercise exact clipboard bytes, including CR and large
            // text, without the script parser's line-ending removal/string-length limit.
            match Ui::k1_word(r, 1040).and_then(|(name, _)| super::perm::trusted(|| plat::read_file(&String::from_utf8_lossy(&name)))) {
                Some(data) if data.len() <= 4 << 20 && core::str::from_utf8(&data).is_ok() => { self.clip_set(CLIP_TEXT, &data); }
                _ => self.fail(b"clip utf8 requires a readable UTF-8 file of at most 4 MiB"),
            }
        } else if v4(b"text") {
            match parse_qstring(r, 1040) {
                Some((t, _)) => {
                    self.clip_set(CLIP_TEXT, &t);
                }
                None => self.fail(BAD),
            }
        } else if v4(b"file") || v3(b"dib") {
            match Ui::k1_word(r, 1040) {
                None => self.fail(BAD),
                Some((f, _)) => {
                    let name = String::from_utf8_lossy(&f).into_owned();
                    match super::perm::trusted(|| plat::read_file(&name)) {
                        None => {
                            let mut d = b"cannot read ".to_vec();
                            d.extend_from_slice(&f);
                            self.fail(&d);
                        }
                        Some(data) => {
                            // a BMP file: the image/bmp path; dib: the CF_DIB path
                            let bmp = if v3(b"dib") {
                                Some(plat::clip_dib_to_bmp(&data))
                            } else if data.len() >= 2 && data[0] == b'B' && data[1] == b'M' {
                                Some(plat::clip_bmp_fix(&data))
                            } else {
                                None
                            };
                            match bmp {
                                Some(None) => self.fail(b"invalid DIB / BMP data"),
                                Some(Some(b)) => self.hclip_img_set_parts(None, Some(&b)),
                                None => self.hclip_img_set_parts(Some(&data), None),
                            }
                        }
                    }
                }
            }
        } else if v4(b"clea") {
            self.hclip_img_clear();
            self.hclip_text_clear();
        } else if v4(b"save") {
            match Ui::k1_word(r, 1040) {
                None => self.fail(BAD),
                Some((f, rest)) => {
                    let name = String::from_utf8_lossy(&f).into_owned();
                    let data = match rest {
                        b"text" => self.clip_get(CLIP_TEXT),
                        b"html" => self.clip_get(CLIP_HTML),
                        b"rtf" => self.clip_get(CLIP_RTF),
                        b"docx" => self.clip_get(CLIP_DOCX),
                        _ if rest.first() == Some(&b'b') => self.dr.hi_bmp.clone(),
                        _ => self.hclip_img_get(CLIP_PNG),
                    };
                    match data {
                        Some(d) => {
                            super::perm::trusted(|| plat::write_file(&name, &d));
                        }
                        None => self.fail(b"the clipboard holds no image"),
                    }
                }
            }
        } else if v4(b"expe") {
            match Ui::k1_word(r, 16) {
                None => self.fail(BAD),
                Some((w, _)) => {
                    let text = self.hclip_text_has();
                    let png = self.hclip_img_has(CLIP_PNG);
                    let image = self.hclip_img_has(CLIP_IMAGE);
                    let w4 = |s: &[u8]| w.len() >= 4 && &w[..4] == s;
                    let ok = if w4(b"none") {
                        Some(!text && !image && self.m.edit.hclip_rich.is_empty())
                    } else if w == b"text-only" {
                        Some(text && !image && self.m.edit.hclip_rich.is_empty())
                    } else if w4(b"text") {
                        Some(text)
                    } else if w == b"html" {
                        Some(self.clip_has(CLIP_HTML))
                    } else if w == b"rtf" {
                        Some(self.clip_has(CLIP_RTF))
                    } else if w == b"docx" {
                        Some(self.clip_has(CLIP_DOCX))
                    } else if w4(b"imag") {
                        Some(image)
                    } else if w.len() >= 3 && &w[..3] == b"png" {
                        Some(png)
                    } else {
                        None
                    };
                    match ok {
                        None => self.fail(BAD),
                        Some(false) => self.fail(alloc::format!("unexpected clipboard content (text {})", text as u32).as_bytes()),
                        Some(true) => {}
                    }
                }
            }
        } else {
            self.fail(BAD);
        }
        self.run_passes(app);
    }
}
