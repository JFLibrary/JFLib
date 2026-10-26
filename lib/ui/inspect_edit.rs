// inspect_edit -- the live inspector's editing: values changed in the tool window, the selected
// widget operated, the changes taken over (ROADMAP §11 "Hot-Reload und beschreibbarer
// Inspektor", stages 2 and 3; GUIDE.md §30)
//
// Only in development builds: build_rs.py passes --cfg jfgui_dev there; a --release build has
// the read-only inspector (inspect_edit_off.rs).  Nothing of the app changes for it: what is
// edited exists already, and the app's own values are never written.
//   * the app theme (ui.set_theme, theme_app.rs) over the app's own (or none): the T_* colour
//     tokens and the accent of the colours in use (light, dark or high contrast), the corner
//     radius, spacing, control height, the sizes of the text styles and a font family
//   * light / dark / high contrast (as theme_set / set_contrast), the linked style
//     (set_style), the UI language (i18n::set_language_code), the main window's scale (its
//     dpi; the window keeps its size in pixels) and the text scale (set_text_scale)
//   * the selected widget, through the synthetic input a script gives (auto.rs): a click, a
//     drag, text typed into a text box (click, Ctrl+A, the text), a command (cmd_run); a
//     slider's value the way a screen reader sets it (plat::a11y::request: the widget takes
//     it as input).  Events go into the platform's queue and are handled like the platform's.
// Every change overrides one key (Key) over the value it had before its first change (its
// base): marked in the tool window, reset one by one or all together, undone and redone (a
// History of the overrides, Snapshot steps; the moves of one drag of a slider or the colour
// picker are one step).  They hold for the running session, also after the inspector closed,
// until they are taken over: "Als Theme speichern" writes the app theme in use as a theme
// file, "Liste kopieren" copies "Das hast du geändert" (the changed theme keys as lines of a
// theme file, the rest as lines of code).  Layout and texts are changed in the code.
//
// The tool window's controls ask for edits during its pass (want); ied_after applies them
// once the window is built (no window's build sees the look change half-way), and the frame
// is built again.  The tool window itself is built in the style's own look (ied_plain): the
// app theme and a text scale edited here do not reach it (the style, light / dark / contrast
// and the language do), so it stays usable whatever is edited.
//
// Hooks (inspect.rs): ied_plain (around the tool window's build), ied_colors (tab "Farben"),
// ied_look (tab "Darstellung"), ied_operate (tab "Widgets", below the properties), ied_keys
// (Ctrl+Z / Ctrl+Y in the tool window), ied_after (after the tool window's pass), ied_script /
// ied_expect (the script commands).
//
// Script commands (auto.rs `inspect ...`; KEY = token NAME | accent | radius | spacing |
// height | size STYLE | font | mode | style | lang | scale | textscale):
//   inspect set token NAME #RRGGBB|#AARRGGBB   a token of the colours in use (NAME as in
//                              theme files: bg, ctrl_stroke; or T_TEXT2)
//   inspect set accent #RRGGBB   inspect set radius DIP|PERCENT%|none   inspect set spacing
//   PERCENT   inspect set height DIP   inspect set size STYLE DIP   inspect set font
//   FAMILY|none   inspect set mode light|dark|contrast   inspect set style NAME
//   inspect set lang CODE   inspect set scale PERCENT   inspect set textscale PERCENT
//   inspect reset [KEY]        one change back to its base (without KEY: every change)
//   inspect undo | redo        inspect save PATH (the app theme in use as a theme file)
//   inspect list               copy "Das hast du geändert" to the clipboard
//   inspect do click | drag DX DY | value N | type "text" | cmd ID [ARG]   the selected widget
//   inspect expect changes N | list "TEXT" (the change list contains it) | undo on|off |
//                  redo on|off | message "TEXT"|none (the last message contains it) |
//                  value KEY "TEXT" (the key's value now, as the tool window shows it)

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::auto::{next_word, parse_qstring, skip_ws, word_eq};
use super::consts::*;
use super::keys::*;
use super::layout::{dp_at, px2dip_at};
use super::runtime::{parse_int, scale_to_dpi};
use super::theme::{parse_color, token_index, Radius, Theme, THEME_CONTRAST, THEME_DARK, THEME_LIGHT, TOKEN_NAMES, TS_NAMES};
use super::{App, History, Sem, Snapshot, Ui};
use crate::i18n::{fill, tr};

/// the inspector can edit (a development build)
pub(crate) const EDIT: bool = true;

// widget ids inside the inspector's window (inspect.rs uses 0x07FE_....; below the hashed
// range, bits 29 and 30 clear)
const ID_BASE: u32 = 0x07FD_0000;
const ID_UNDO: u32 = ID_BASE + 1;
const ID_REDO: u32 = ID_BASE + 2;
const ID_RESET_ALL: u32 = ID_BASE + 3;
const ID_TOKF: u32 = ID_BASE + 0x10; // + 0..2: the token's hex box, picker switch, picker
const ID_ACCF: u32 = ID_BASE + 0x14; // + 0..2: the accent's
const ID_LOOK: u32 = ID_BASE + 0x20;
const ID_STYLE: u32 = ID_BASE + 0x21;
const ID_MODE: u32 = ID_BASE + 0x22;
const ID_LANG: u32 = ID_BASE + 0x23;
const ID_SCALE: u32 = ID_BASE + 0x24;
const ID_TSCALE: u32 = ID_BASE + 0x25;
const ID_RADIUS: u32 = ID_BASE + 0x26;
const ID_SPACING: u32 = ID_BASE + 0x27;
const ID_HEIGHT: u32 = ID_BASE + 0x28;
const ID_FAMILY: u32 = ID_BASE + 0x29;
const ID_PATH: u32 = ID_BASE + 0x2A;
const ID_SAVE: u32 = ID_BASE + 0x2B;
const ID_COPY: u32 = ID_BASE + 0x2C;
const ID_SIZE: u32 = ID_BASE + 0x30; // + TS_*
const ID_OPS: u32 = ID_BASE + 0x40;
const ID_CLICK: u32 = ID_BASE + 0x41;
const ID_DX: u32 = ID_BASE + 0x42;
const ID_DY: u32 = ID_BASE + 0x43;
const ID_DRAG: u32 = ID_BASE + 0x44;
const ID_VALUE: u32 = ID_BASE + 0x45;
const ID_SET: u32 = ID_BASE + 0x46;
const ID_TEXT: u32 = ID_BASE + 0x47;
const ID_TYPE: u32 = ID_BASE + 0x48;
const ID_CMD: u32 = ID_BASE + 0x49;
const ID_RUN: u32 = ID_BASE + 0x4A;
const ID_RESET: u32 = ID_BASE + 0x100; // + Key::ord: a value's reset (the tabs' rows)
const ID_RESET2: u32 = ID_BASE + 0x800; // + Key::ord: ... in the change list
const ID_TOKENS: u32 = ID_BASE + 0x1000; // + 1 + i: the token rows

const LABEL_W: i32 = 130; // the names of the rows (DIP)
const OP_W: i32 = 64; // ... in the flyout of the widget operations
const ROW_H: u32 = 28; // the token rows (DIP)
const RESET_W: i32 = 32; // a row's reset button (DIP)
const HIST_MAX: usize = 200;

// asked for in the tool window's pass (S::act)
const A_UNDO: u8 = 1;
const A_REDO: u8 = 2;
const A_RESET: u8 = 3;

const RG_RADIUS: Range = Range::new(0, 32, 1, 0);
const RG_SPACING: Range = Range::new(25, 200, 5, 0);
const RG_HEIGHT: Range = Range::new(0, 96, 2, 0);
const RG_SIZE: Range = Range::new(4, 200, 1, 0);
const RG_SCALE: Range = Range::new(50, 400, 25, 0);
const RG_TSCALE: Range = Range::new(50, 300, 10, 0);
const RG_DRAG: Range = Range::new(-4000, 4000, 1, 0);
const RG_CMD: Range = Range::new(0, i32::MAX, 1, 0);

// the languages of the library's texts by their own names (language_codes: the columns)
const LANG_NAMES: [(&str, &str); 9] = [
    ("de", "Deutsch"),
    ("en", "English"),
    ("fr", "Français"),
    ("es", "Español"),
    ("it", "Italiano"),
    ("nl", "Nederlands"),
    ("pl", "Polski"),
    ("ar", "العربية"),
    ("he", "עברית"),
];

/// what a change overrides
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Key {
    /// a colour token of the colours THEME_LIGHT / _DARK / _CONTRAST (T_* / 4)
    Token(u8, u8),
    /// the accent of those colours (T_ACCENT's row edits it: the style derives the ramp)
    Accent(u8),
    Radius,
    Spacing,
    Height,
    /// the size of a text style (TS_*)
    Size(u8),
    Family,
    Mode,
    Style,
    Lang,
    Scale,
    TextScale,
}

impl Key {
    // a key of the app theme (set_theme)
    fn theme(self) -> bool {
        matches!(self, Key::Token(..) | Key::Accent(_) | Key::Radius | Key::Spacing | Key::Height | Key::Size(_) | Key::Family)
    }

    // a small number of its own (the ids of the reset buttons)
    fn ord(self) -> u32 {
        match self {
            Key::Token(m, t) => 0x100 + m as u32 * 64 + t as u32,
            Key::Accent(m) => 0x80 + m as u32,
            Key::Size(i) => 0x90 + i as u32,
            Key::Radius => 1,
            Key::Spacing => 2,
            Key::Height => 3,
            Key::Family => 4,
            Key::Mode => 5,
            Key::Style => 6,
            Key::Lang => 7,
            Key::Scale => 8,
            Key::TextScale => 9,
        }
    }
}

/// the value of a key
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Val {
    /// a colour 0xAARRGGBB (None: the style's)
    Color(Option<u32>),
    Radius(Option<Radius>),
    /// spacing in percent, control height in DIP (0: the style's), a text size 24.8 (0: the
    /// style's), the main window's dpi
    Num(i32),
    /// a font family ("": the style's), a language code
    Text(String),
    Style(&'static str),
    /// the system's light / dark is followed, dark, the contrast mode (0 the system's, 1 on,
    /// 2 off), high contrast on
    Mode(bool, bool, u8, bool),
    /// the text scale in percent, fixed by the app, the command line or here (else the system's)
    Scale(u32, bool),
}

type Over = BTreeMap<Key, Val>;

/// a widget operation, run after the tool window's pass
enum Op {
    /// window, the point (px of the window)
    Click(u32, i32, i32),
    /// window, from, to
    Drag(u32, i32, i32, i32, i32),
    /// window, the point to click, the text, a text box (Ctrl+A selects its text first)
    Type(u32, i32, i32, String, bool),
    /// window, id, the n-th record of the id (1..), the value
    Value(u32, u32, u32, i32),
    /// command id, argument
    Cmd(u32, u32),
}

/// the app's look while the tool window is built in the style's own (ied_plain)
struct Plain {
    rad_mode: u8,
    spacing: i32,
    ctrl_h: i32,
    text: bool,
    fonts: [u32; 3],
    text_scale: u32,
    tokens: [u32; T_COUNT],
}

/// a colour field: its hex text, what it was filled from, the picker shown
#[derive(Default)]
struct Field {
    hex: String,
    of: u64,
    pick: i32,
}

/// the editing state (ui.m.ied)
pub(crate) struct S {
    /// the changes (overrides) and their history
    over: Over,
    hist: History<Over>,
    /// a key's value before its first change and how it showed then
    base: BTreeMap<Key, (Val, String)>,
    /// the app had no theme before the first change of a theme key: none again when every
    /// theme key is back
    theme_none: bool,
    /// asked for in the tool window's pass, done by ied_after: edits (key, value or None for
    /// the base, part of a drag), the end of a drag, undo / redo / reset all, operations
    want: Vec<(Key, Option<Val>, bool)>,
    want_end: bool,
    act: u8,
    ops: Vec<Op>,
    /// the release of a drag: queued once the drag's moves were handled (a pass while held)
    later: Vec<plat::Event>,
    /// the edits of one drag merge into one step
    gesture: u32,
    /// "Farben": the token edited (T_* / 4); the colour fields of the token and the accent
    tok: i32,
    tokf: Field,
    accf: Field,
    /// text fields: the family, the theme file's path
    family: String,
    family_of: String,
    path: String,
    /// the widget operations: their fields, the widget they were filled for
    dx: i32,
    dy: i32,
    value: i32,
    text: String,
    cmd: i32,
    op_of: (u8, u32, u16),
    /// the last operation as a script line, the last message (saved, copied, an error)
    last: String,
    msg: String,
    /// the app's look while the tool window is built (ied_plain)
    plain: Option<Plain>,
}

impl Default for S {
    fn default() -> S {
        S {
            over: Over::new(),
            hist: History::new(HIST_MAX),
            base: BTreeMap::new(),
            theme_none: false,
            want: Vec::new(),
            want_end: false,
            act: 0,
            ops: Vec::new(),
            later: Vec::new(),
            gesture: 1,
            tok: 0,
            tokf: Field::default(),
            accf: Field::default(),
            family: String::new(),
            family_of: String::from("\u{1}"),
            path: String::new(),
            dx: 40,
            dy: 0,
            value: 0,
            text: String::new(),
            cmd: 0,
            op_of: (0xFF, 0, 0),
            last: String::new(),
            msg: String::new(),
            plain: None,
        }
    }
}

// ------------------------------------------------------------------ helpers

// #RRGGBB (opaque) or #AARRGGBB
fn color_text(c: u32) -> String {
    if c >> 24 == 0xFF { format!("#{:06X}", c & 0xFF_FFFF) } else { format!("#{:08X}", c) }
}

// a size 24.8 in DIP ("15", "14.5")
fn size_text(fx: i32) -> String {
    if fx & 255 == 0 { format!("{}", fx >> 8) } else { base::num::f64_format(fx as f64 / 256.0) }
}

fn mode_name(m: usize) -> &'static str {
    match m {
        THEME_LIGHT => tr("Hell"),
        THEME_DARK => tr("Dunkel"),
        _ => tr("Hoher Kontrast"),
    }
}

// the colours a mode value shows: THEME_LIGHT, _DARK, _CONTRAST
fn mode_of(v: &Val) -> usize {
    match *v {
        Val::Mode(_, _, _, true) => THEME_CONTRAST,
        Val::Mode(_, true, _, _) => THEME_DARK,
        _ => THEME_LIGHT,
    }
}

// a language code by its own name
fn lang_name(code: &str) -> String {
    let p = code.split(['-', '_']).next().unwrap_or(code).to_ascii_lowercase();
    match LANG_NAMES.iter().find(|l| l.0 == p) {
        Some(l) => format!("{} ({})", l.1, code),
        None => String::from(code),
    }
}

// the name of a key in the tool window and the change list
fn key_name(k: Key) -> String {
    match k {
        Key::Token(m, t) => format!("T_{} ({})", TOKEN_NAMES[t as usize], mode_name(m as usize)),
        Key::Accent(m) => format!("{} ({})", tr("Akzent"), mode_name(m as usize)),
        Key::Radius => String::from(tr("Eckenradius")),
        Key::Spacing => String::from(tr("Abstände")),
        Key::Height => String::from(tr("Steuerelementhöhe")),
        Key::Size(i) => format!("{}.size", TS_NAMES[i as usize]),
        Key::Family => String::from(tr("Familie")),
        Key::Mode => String::from(tr("Modus")),
        Key::Style => String::from(tr("Stil")),
        Key::Lang => String::from(tr("Sprache")),
        Key::Scale => String::from(tr("Skalierung")),
        Key::TextScale => String::from(tr("Textskalierung")),
    }
}

// the key's name in a theme file ("bg", "accent"; empty for the keys that are no colour)
fn file_key(k: Key) -> String {
    match k {
        Key::Token(_, t) => TOKEN_NAMES[t as usize].to_ascii_lowercase(),
        Key::Accent(_) => String::from("accent"),
        _ => String::new(),
    }
}

// a value as the tool window shows it
fn val_text(k: Key, v: &Val) -> String {
    let style = || String::from(tr("vom Stil"));
    match (k, v) {
        (_, Val::Color(Some(c))) => color_text(*c),
        (_, Val::Color(None)) | (_, Val::Radius(None)) => style(),
        (_, Val::Radius(Some(Radius::Dip(d)))) => format!("{} DIP", d),
        (_, Val::Radius(Some(Radius::Percent(p)))) => format!("{} %", p),
        (Key::Spacing, Val::Num(n)) => format!("{} %", n),
        (Key::Scale, Val::Num(dpi)) => format!("{} %", (dpi * 100 + 48) / 96),
        (_, Val::Num(0)) => style(),
        (Key::Size(_), Val::Num(fx)) => format!("{} DIP", size_text(*fx)),
        (_, Val::Num(n)) => format!("{} DIP", n),
        (Key::Lang, Val::Text(c)) => lang_name(c),
        (_, Val::Text(t)) if t.is_empty() => style(),
        (_, Val::Text(t)) => t.clone(),
        (_, Val::Style(n)) => String::from(*n),
        (_, Val::Mode(..)) => String::from(mode_name(mode_of(v))),
        (_, Val::Scale(p, _)) => format!("{} %", p),
    }
}

// a theme key's value into a theme
fn theme_put(th: &mut Theme, k: Key, v: &Val) {
    match (k, v) {
        (Key::Token(m, t), Val::Color(c)) => th.colors[m as usize][t as usize] = *c,
        (Key::Accent(m), Val::Color(c)) => th.accent[m as usize] = c.map(|c| c & 0xFF_FFFF),
        (Key::Radius, Val::Radius(r)) => th.radius = *r,
        (Key::Spacing, Val::Num(n)) => th.spacing = (*n).clamp(25, 400) as u16,
        (Key::Height, Val::Num(n)) => th.control_height = if *n <= 0 { 0 } else { (*n).clamp(12, 96) as u16 },
        (Key::Size(i), Val::Num(n)) => th.text[i as usize].size_fx = if *n <= 0 { None } else { Some((*n).clamp(4 << 8, 200 << 8) as u16) },
        (Key::Family, Val::Text(f)) => th.family = f.clone(),
        _ => {}
    }
}

// a change that is no theme key, as a line of code (the change list)
fn code_line(k: Key, v: &Val) -> String {
    match v {
        Val::Mode(_, _, 1, _) | Val::Mode(_, _, 0, true) => String::from("ui.set_contrast(Some(true));"),
        Val::Mode(true, ..) => String::from("ui.theme_set(2); // the system's"),
        Val::Mode(false, dark, ..) => format!("ui.theme_set({});", *dark as u32),
        Val::Style(n) => format!("ui.set_style(\"{}\");", n),
        Val::Text(c) if k == Key::Lang => format!("ui::i18n::set_language_code(\"{}\");", c),
        Val::Num(dpi) if k == Key::Scale => format!("--scale {}", (dpi * 100 + 48) / 96),
        Val::Scale(p, _) => format!("ui.set_text_scale({});", p),
        _ => format!("{} = {}", key_name(k), val_text(k, v)),
    }
}

// a path that names itself: absolute, or with a scheme (app://, mem://)
fn is_abs(f: &str) -> bool {
    let b = f.as_bytes();
    f.contains("://") || f.starts_with('/') || f.starts_with('\\') || (b.len() > 2 && b[1] == b':')
}

// a font of the theme as the theme file `target` names it (Theme::load takes a relative font
// file from the file's folder): a file named relative to the working folder becomes relative
// to the theme file's folder, or absolute where that cannot be said (a scheme, ".."); the
// installed families and absolute names stay
fn font_for(f: &str, target: &str) -> String {
    let l = f.to_ascii_lowercase();
    let file = l.contains('/') || l.contains('\\') || [".ttf", ".otf", ".ttc", ".otc"].iter().any(|e| l.ends_with(e));
    if f.is_empty() || !file || is_abs(f) {
        return String::from(f);
    }
    let dir = target.rsplit_once(['/', '\\']).map_or("", |(d, _)| d);
    if !is_abs(target) && !dir.split(['/', '\\']).any(|s| s == "..") {
        let up = dir.split(['/', '\\']).filter(|s| !s.is_empty() && *s != ".").count();
        return format!("{}{}", "../".repeat(up), f);
    }
    let cwd = plat::cwd().replace('\\', "/");
    if cwd.is_empty() { String::from(f) } else { format!("{}/{}", cwd.trim_end_matches('/'), f) }
}

// a script string: " \ and control characters escaped
fn quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

// the rest of a script line as a string: quoted, or the bare words
fn arg_text(r: &[u8]) -> String {
    let r = skip_ws(r);
    if r.first() == Some(&b'"') {
        if let Some((s, _)) = parse_qstring(r, 1024) {
            return String::from_utf8_lossy(&s).into_owned();
        }
    }
    String::from(String::from_utf8_lossy(r).trim())
}

// the first word of a script argument
fn word(r: &[u8]) -> String {
    let r = skip_ws(r);
    let n = r.iter().position(|&c| c <= b' ').unwrap_or(r.len());
    String::from_utf8_lossy(&r[..n]).into_owned()
}

impl Ui {
    // ------------------------------------------------------------------ the model

    // the colours in use: THEME_LIGHT, _DARK, _CONTRAST
    fn ied_mode(&self) -> usize {
        if self.high_contrast {
            THEME_CONTRAST
        } else if self.theme_dark {
            THEME_DARK
        } else {
            THEME_LIGHT
        }
    }

    /// (inspect.rs inspect_frame, around the tool window's build) on: the tool window is built
    /// in the style's own look -- the app theme (the app's own and the edits here) and a text
    /// scale edited here do not reach it, so it stays usable whatever is edited; off: the
    /// app's look again (the app's windows are built before it, the edits are applied after)
    pub(crate) fn ied_plain(&mut self, on: bool) {
        if !on {
            if let Some(p) = self.m.ied.plain.take() {
                let l = &mut self.look;
                (l.rad_mode, l.spacing, l.ctrl_h, l.text, l.fonts) = (p.rad_mode, p.spacing, p.ctrl_h, p.text, p.fonts);
                self.text_scale = p.text_scale;
                self.theme[..T_COUNT].copy_from_slice(&p.tokens);
            }
            return;
        }
        let mut tokens = [0u32; T_COUNT];
        tokens.copy_from_slice(&self.theme[..T_COUNT]);
        let l = &self.look;
        let p = Plain { rad_mode: l.rad_mode, spacing: l.spacing, ctrl_h: l.ctrl_h, text: l.text, fonts: l.fonts, text_scale: self.text_scale, tokens };
        // the style's tokens: theme_build without an app theme
        let t = if self.high_contrast {
            let c = self.hc_sys.unwrap_or(if self.theme_base_dark { plat::HcColors::BLACK } else { plat::HcColors::WHITE });
            super::theme::contrast_tokens(&c)
        } else {
            let st = self.style;
            let mut t = st.tokens(self.theme_dark);
            st.accent(&mut t, self.theme_dark, self.accent);
            t
        };
        self.theme[..T_COUNT].copy_from_slice(&t);
        let l = &mut self.look;
        (l.rad_mode, l.spacing, l.ctrl_h, l.text, l.fonts) = (0, 100, 0, false, [FONT_REG, FONT_SEMI, FONT_BOLD]);
        if self.m.ied.over.contains_key(&Key::TextScale) {
            if let Some((Val::Scale(s, _), _)) = self.m.ied.base.get(&Key::TextScale) {
                self.text_scale = *s; // the text scale before the edit
            }
        }
        self.m.ied.plain = Some(p);
    }

    // the app's colour token t (also while the tool window is built in the style's look)
    fn ied_tok(&self, t: usize) -> u32 {
        match &self.m.ied.plain {
            Some(p) => p.tokens[t],
            None => self.theme[t],
        }
    }

    // the app's text scale (also while the tool window is built)
    fn ied_text_scale(&self) -> u32 {
        self.m.ied.plain.as_ref().map_or(self.text_scale, |p| p.text_scale)
    }

    // the key a token's row edits (T_ACCENT: the accent the style derives it from)
    fn ied_tok_key(&self, t: usize) -> Key {
        let m = self.ied_mode() as u8;
        if t == (T_ACCENT / 4) as usize { Key::Accent(m) } else { Key::Token(m, t as u8) }
    }

    // the value of a key now
    fn ied_get(&self, k: Key) -> Val {
        let th = self.look.theme.as_deref();
        match k {
            Key::Token(m, t) => Val::Color(th.and_then(|th| th.colors[m as usize][t as usize])),
            Key::Accent(m) => Val::Color(th.and_then(|th| th.accent[m as usize]).map(|c| c | 0xFF00_0000)),
            Key::Radius => Val::Radius(th.and_then(|th| th.radius)),
            Key::Spacing => Val::Num(th.map_or(100, |th| th.spacing as i32)),
            Key::Height => Val::Num(th.map_or(0, |th| th.control_height as i32)),
            Key::Size(i) => Val::Num(th.and_then(|th| th.text[i as usize].size_fx).map_or(0, i32::from)),
            Key::Family => Val::Text(th.map_or(String::new(), |th| th.family.clone())),
            Key::Mode => Val::Mode(self.theme_sys, self.theme_base_dark, self.contrast_mode, self.high_contrast),
            Key::Style => Val::Style(self.style.name()),
            Key::Lang => Val::Text(crate::i18n::language_code()),
            Key::Scale => Val::Num(self.win_dpi(0)),
            Key::TextScale => Val::Scale(self.ied_text_scale(), self.text_scale_forced),
        }
    }

    // the colour of a token row now (T_ACCENT's row: the accent it comes from)
    fn ied_color_now(&self, k: Key, t: usize) -> u32 {
        match (k, self.ied_get(k)) {
            (Key::Accent(_), Val::Color(Some(c))) => c,
            (Key::Accent(_), _) if self.accent != 0 => self.accent | 0xFF00_0000,
            _ => self.ied_tok(t),
        }
    }

    // how a key shows now (its base in the change list): a colour the style gives is shown
    fn ied_show_now(&self, k: Key) -> String {
        let v = self.ied_get(k);
        match (k, &v) {
            (Key::Token(m, t), Val::Color(None)) if m as usize == self.ied_mode() => color_text(self.ied_tok(t as usize)),
            (Key::Accent(m), Val::Color(None)) if m as usize == self.ied_mode() => {
                color_text(self.ied_color_now(k, (T_ACCENT / 4) as usize))
            }
            _ => val_text(k, &v),
        }
    }

    // values into the look: the theme keys together (one set_theme), then the rest -> the
    // error of set_theme (a font that cannot be loaded: the theme stays as it was)
    fn ied_put(&mut self, vals: &[(Key, Val)]) -> Result<(), String> {
        if vals.iter().any(|(k, _)| k.theme()) {
            if self.m.ied.theme_none && !self.m.ied.over.keys().any(|k| k.theme()) {
                self.clear_theme(); // every theme key is back: no theme, as before
            } else {
                let mut th = self.app_theme().cloned().unwrap_or_default();
                for (k, v) in vals.iter().filter(|(k, _)| k.theme()) {
                    theme_put(&mut th, *k, v);
                }
                super::perm::trusted(|| self.set_theme(&th)).map_err(|e| e.msg)?;
            }
        }
        for (k, v) in vals.iter().filter(|(k, _)| !k.theme()) {
            match (k, v) {
                (Key::Mode, Val::Mode(sys, dark, c, hc)) => {
                    self.theme_sys = *sys;
                    self.theme_base_dark = *dark;
                    self.contrast_mode = *c;
                    self.high_contrast = *hc;
                    self.theme_build();
                    self.cmd_run(UIC_THEME, 0); // as the script's `theme`
                }
                (Key::Style, Val::Style(n)) => {
                    self.set_style(n);
                }
                (Key::Lang, Val::Text(code)) => {
                    crate::i18n::set_language_code(code);
                    // every text again: laid out and drawn (rtl_update mirrors with the next frame)
                    self.look.serial = self.look.serial.wrapping_add(1);
                    self.dirty_force_full();
                    for r in self.m.win.recs.iter_mut() {
                        if let Some(p) = r.save.as_deref_mut() {
                            p.dirty.full = true;
                        }
                    }
                }
                (Key::Scale, Val::Num(dpi)) => self.uiw_dpi(0, *dpi),
                (Key::TextScale, Val::Scale(p, forced)) => {
                    self.set_text_scale(*p);
                    self.text_scale_forced = *forced;
                }
                _ => {}
            }
        }
        self.repass();
        self.invalidate();
        Ok(())
    }

    // the overrides changed from `old`: each key that differs gets its new value, or its base
    // when its change is gone
    fn ied_sync(&mut self, old: &Over) -> Result<(), String> {
        let e = &self.m.ied;
        let mut vals: Vec<(Key, Val)> = Vec::new();
        for k in old.keys().chain(e.over.keys()) {
            if old.get(k) == e.over.get(k) || vals.iter().any(|(x, _)| x == k) {
                continue;
            }
            let v = match (e.over.get(k), e.base.get(k)) {
                (Some(v), _) => v.clone(),
                (None, Some(b)) => b.0.clone(),
                (None, None) => continue,
            };
            vals.push((*k, v));
        }
        self.ied_put(&vals)
    }

    // key k changes to v (None: back to its base) as a step of the history; drag: a step of
    // a drag (its steps are one)
    fn ied_edit(&mut self, k: Key, v: Option<Val>, drag: bool) {
        if !self.m.ied.base.contains_key(&k) {
            let b = (self.ied_get(k), self.ied_show_now(k));
            self.m.ied.base.insert(k, b);
        }
        if k.theme() && !self.m.ied.over.keys().any(|k| k.theme()) {
            self.m.ied.theme_none = self.look.theme.is_none();
        }
        let old = self.m.ied.over.clone();
        let mut new = old.clone();
        let base = &self.m.ied.base[&k].0;
        // a mode that shows the base's colours is the base (the system's settings stay)
        let same = |v: &Val| v == base || (k == Key::Mode && mode_of(v) == mode_of(base));
        match v {
            Some(v) if !same(&v) => {
                new.insert(k, v);
            }
            _ => {
                new.remove(&k);
            }
        }
        if new == old {
            return;
        }
        self.m.ied.over = new.clone();
        match self.ied_sync(&old) {
            Ok(()) => {
                let e = &mut self.m.ied;
                let key = if drag { (k.ord() << 16 ^ e.gesture) | 0x8000_0000 } else { 0 };
                e.hist.record(Box::new(Snapshot::new(&key_name(k), old, new).merging(key)));
                if !drag {
                    e.gesture = e.gesture.wrapping_add(1);
                }
                e.msg.clear();
            }
            Err(m) => {
                self.m.ied.over = old;
                self.m.ied.msg = m;
            }
        }
    }

    // undo, redo, reset all -> done
    fn ied_step(&mut self, act: u8) -> bool {
        let old = self.m.ied.over.clone();
        let e = &mut self.m.ied;
        let done = match act {
            A_UNDO => e.hist.undo(&mut e.over),
            A_REDO => e.hist.redo(&mut e.over),
            _ if old.is_empty() => false,
            _ => {
                e.over.clear();
                e.hist.record(Box::new(Snapshot::new(tr("Alles zurücksetzen"), old.clone(), Over::new())));
                true
            }
        };
        if done {
            e.gesture = e.gesture.wrapping_add(1);
            e.msg.clear();
            if let Err(m) = self.ied_sync(&old) {
                self.m.ied.msg = m;
            }
        }
        done
    }

    // (the tool window's pass) an edit for ied_after
    fn ied_want(&mut self, k: Key, v: Option<Val>, drag: bool) {
        self.m.ied.want.push((k, v, drag));
    }

    /// (inspect.rs inspect_frame, after the tool window was built) what its controls asked
    /// for: steps, edits, widget operations
    pub(crate) fn ied_after(&mut self) {
        self.ied_later();
        let act = core::mem::take(&mut self.m.ied.act);
        let want = core::mem::take(&mut self.m.ied.want);
        let ops = core::mem::take(&mut self.m.ied.ops);
        if act != 0 {
            self.ied_step(act);
        }
        for (k, v, drag) in want {
            self.ied_edit(k, v, drag);
        }
        if core::mem::take(&mut self.m.ied.want_end) {
            self.m.ied.gesture = self.m.ied.gesture.wrapping_add(1); // a drag ended
        }
        for op in ops {
            self.ied_run(op);
        }
    }

    // ------------------------------------------------------------------ taking over

    // write the app theme in use as a theme file -> written
    fn ied_save(&mut self, path: &str) -> bool {
        let mut th = self.app_theme().cloned().unwrap_or_default();
        for f in [&mut th.family, &mut th.family_semibold, &mut th.family_bold] {
            *f = font_for(f, path);
        }
        let ok = !path.is_empty() && super::perm::trusted(|| th.save(path));
        self.m.ied.msg = if ok { fill(tr("Gespeichert: {}"), &[path]) } else { fill(tr("Nicht gespeichert: {}"), &[path]) };
        ok
    }

    // "Das hast du geändert": the changed theme keys as a theme file's lines, the rest as
    // lines of code (comments: the whole text can go into a theme file)
    fn ied_list(&self) -> String {
        let mut th = Theme::default();
        let mut code: Vec<String> = Vec::new();
        for (k, v) in &self.m.ied.over {
            if k.theme() {
                theme_put(&mut th, *k, v);
            } else {
                code.push(code_line(*k, v));
            }
        }
        let mut s = format!("# {}\n", tr("Das hast du geändert"));
        if th != Theme::default() {
            s.push_str(&format!("# {}:\n", tr("Theme-Datei")));
            for l in th.to_ini().lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
                s.push_str(l);
                s.push('\n');
            }
        }
        if !code.is_empty() {
            s.push_str(&format!("# {}:\n", tr("Im Code")));
            for c in code {
                s.push_str("#   ");
                s.push_str(&c);
                s.push('\n');
            }
        }
        s
    }

    // the change list to the clipboard
    fn ied_copy(&mut self) {
        let s = self.ied_list();
        self.clip_set(CLIP_TEXT, s.as_bytes());
        self.m.ied.msg = String::from(tr("Kopiert"));
    }

    // ------------------------------------------------------------------ widget operations

    // a synthetic event for window w (the platform's window index outside headless runs)
    fn ied_event(&self, typ: u8, w: u32, x: i32, y: i32, btn: u32, mods: u32, data: u32) -> Option<plat::Event> {
        let win = if self.headless { w as i32 } else { self.uiw_pw(w) };
        (win >= 0).then_some(plat::Event { typ, btn: btn as u8, mods: mods as u8, win: win as u8, x, y, data })
    }

    // ... into the platform's queue
    fn ied_ev(&mut self, typ: u8, w: u32, x: i32, y: i32, btn: u32, mods: u32, data: u32) {
        if let Some(e) = self.ied_event(typ, w, x, y, btn, mods, data) {
            plat::evq_push(e);
        }
    }

    // the release of a drag, once the queue holds none of its moves any more (they were
    // handled, with a pass while the button was held) -> queued
    fn ied_later(&mut self) -> bool {
        if self.m.ied.later.is_empty() || plat::evq_count() != 0 {
            return false;
        }
        for e in core::mem::take(&mut self.m.ied.later) {
            plat::evq_push(e);
        }
        if !self.headless {
            plat::wake();
        }
        true
    }

    // a click at (x, y) of window w
    fn ied_click(&mut self, w: u32, x: i32, y: i32) {
        self.ied_ev(plat::E_MOVE, w, x, y, 0, 0, 0);
        self.ied_ev(plat::E_DOWN, w, x, y, BTN_LEFT, 0, 0);
        self.ied_ev(plat::E_UP, w, x, y, BTN_LEFT, 0, 0);
    }

    // run an operation: events into the platform's queue (the events the platform queues next
    // come after them), a screen reader's value, a command
    fn ied_run(&mut self, op: Op) {
        match op {
            Op::Click(w, x, y) => self.ied_click(w, x, y),
            Op::Drag(w, x0, y0, x1, y1) => {
                // the moves come in one batch (their last position counts, as when the
                // platform delivers them faster than frames); the release after a pass
                self.ied_ev(plat::E_MOVE, w, x0, y0, 0, 0, 0);
                self.ied_ev(plat::E_DOWN, w, x0, y0, BTN_LEFT, 0, 0);
                for i in 1..=8 {
                    let (x, y) = (x0 + (x1 - x0) * i / 8, y0 + (y1 - y0) * i / 8);
                    self.ied_ev(plat::E_MOVE, w, x, y, 0, 0, 0);
                }
                self.m.ied.later.extend(self.ied_event(plat::E_UP, w, x1, y1, BTN_LEFT, 0, 0));
            }
            Op::Type(w, x, y, s, text_box) => {
                self.ied_click(w, x, y);
                if text_box {
                    self.ied_ev(plat::E_KEY_DOWN, w, 0, 0, 0, MOD_CTRL, VK_A);
                    self.ied_ev(plat::E_KEY_UP, w, 0, 0, 0, MOD_CTRL, VK_A);
                }
                for c in s.chars() {
                    if c == '\n' {
                        self.ied_ev(plat::E_KEY_DOWN, w, 0, 0, 0, 0, VK_RETURN);
                        self.ied_ev(plat::E_KEY_UP, w, 0, 0, 0, 0, VK_RETURN);
                    } else {
                        self.ied_ev(plat::E_TEXT, w, 0, 0, 0, 0, c as u32);
                    }
                }
            }
            Op::Value(w, id, nth, v) => {
                let act = plat::a11y::Act::Value(v as f64);
                plat::a11y::request(plat::a11y::Request { win: w as u8, key: id as u64 | (nth as u64) << 32, act });
                self.a11y_poll();
                self.repass();
            }
            Op::Cmd(id, arg) => self.cmd_run(id, arg),
        }
        self.invalidate();
        if !self.headless {
            plat::wake(); // the window loop handles the queued events now
        }
    }

    // the operation `what` on the selected widget r: "click", "drag", "value", "type", "cmd"
    // -> the operation and its script line
    fn ied_op(&self, r: &Sem, what: &str) -> Option<(Op, String)> {
        let w = r.win as u32;
        let (cx, cy) = (r.rect[0] + r.rect[2] / 2, r.rect[1] + r.rect[3] / 2);
        let sel = self.ins_selector(r);
        let e = &self.m.ied;
        Some(match what {
            "click" => (Op::Click(w, cx, cy), format!("click {}", sel)),
            "drag" => {
                let dpi = self.win_dpi(w);
                let (x1, y1) = (cx + dp_at(e.dx, dpi), cy + dp_at(e.dy, dpi));
                (Op::Drag(w, cx, cy, x1, y1), format!("drag {} {},{}", sel, px2dip_at(x1, dpi), px2dip_at(y1, dpi)))
            }
            "value" => {
                let i = self.sems.iter().position(|s| s.win == r.win && s.id == r.id && s.rect == r.rect && s.kind == r.kind)?;
                let nth = self.sems[..i].iter().filter(|s| s.win == r.win && s.id == r.id).count() as u32 + 1;
                (Op::Value(w, r.id, nth, e.value), format!("inspect do value {}", e.value))
            }
            "type" => {
                let tb = r.kind == K_TEXTBOX;
                let line = format!("click {}{}  type {}", sel, if tb { "  key ctrl+a" } else { "" }, quote(&e.text));
                (Op::Type(w, cx, cy, e.text.clone(), tb), line)
            }
            _ => (Op::Cmd(e.cmd as u32, 0), format!("cmd {}", e.cmd)),
        })
    }

    // the selected widget's operation `what` for ied_after
    fn ied_op_want(&mut self, r: &Sem, what: &str) {
        if let Some((op, line)) = self.ied_op(r, what) {
            self.m.ied.ops.push(op);
            self.m.ied.last = line;
        }
    }

    // ------------------------------------------------------------------ the tool window

    // the bar above the editable tabs: undo, redo, reset all, the number of changes, the last
    // message
    fn ied_bar(&mut self) {
        let (u, r, n) = (self.m.ied.hist.can_undo(), self.m.ied.hist.can_redo(), self.m.ied.over.len());
        self.box_begin(0, BOX_H | BOX_XCENTER, 4);
        if !u {
            self.next_flags(ST_DISABLED);
        }
        if self.icon_button(ID_UNDO, IC_UNDO, tr("Rückgängig")) & EV_ACT != 0 {
            self.m.ied.act = A_UNDO;
        }
        if !r {
            self.next_flags(ST_DISABLED);
        }
        if self.icon_button(ID_REDO, IC_REDO, tr("Wiederholen")) & EV_ACT != 0 {
            self.m.ied.act = A_REDO;
        }
        if n == 0 {
            self.next_flags(ST_DISABLED);
        }
        if self.button(ID_RESET_ALL, tr("Alles zurücksetzen")) & EV_ACT != 0 {
            self.m.ied.act = A_RESET;
        }
        self.box_end();
        // the status line (always there: nothing below moves): changes, the last message
        self.box_begin(0, BOX_H | BOX_XCENTER, 8);
        self.text(TS_CAPTION | TX_SECONDARY, &fill(tr("Änderungen: {}"), &[&format!("{}", n)]));
        if !self.m.ied.msg.is_empty() {
            let m = self.m.ied.msg.clone();
            self.next_size(-1, 0);
            self.text(TS_CAPTION | TX_ACCENT | TX_ELLIPSIS, &m);
        }
        self.box_end();
    }

    // a swatch: the colour over white and black halves (its alpha shows)
    fn ied_well(&mut self, c: u32, w: i32, h: i32) {
        let stroke = self.color(T_CTRL_STROKE);
        self.next_size(w, h);
        let r = self.place(0, 0);
        let h2 = r[2] / 2;
        self.draw_color(stroke);
        self.draw_rect(r[0], r[1], r[2], r[3]);
        self.draw_color(0xFFFF_FFFF);
        self.draw_rect(r[0] + 1, r[1] + 1, h2 - 1, r[3] - 2);
        self.draw_color(0xFF00_0000);
        self.draw_rect(r[0] + h2, r[1] + 1, r[2] - h2 - 1, r[3] - 2);
        self.draw_color(c);
        self.draw_rect(r[0] + 1, r[1] + 1, r[2] - 2, r[3] - 2);
    }

    // a value's reset button (only for a changed one; its place is kept)
    fn ied_reset_button(&mut self, k: Key, id: u32) {
        if self.m.ied.over.contains_key(&k) {
            self.next_size(RESET_W, 0);
            let label = fill(tr("Zurücksetzen: {}"), &[&key_name(k)]);
            if self.icon_button(id, IC_CANCEL, &label) & EV_ACT != 0 {
                self.ied_want(k, None, false);
            }
        } else {
            self.spacer(RESET_W);
        }
    }

    // a row of the tab "Darstellung": its name (accent: changed); the control follows
    fn ied_row(&mut self, k: Key, label: &str) {
        let changed = self.m.ied.over.contains_key(&k);
        self.box_begin(0, BOX_H | BOX_XCENTER, 8);
        self.next_size(LABEL_W, 0);
        self.text(TS_BODY | TX_ELLIPSIS | if changed { TX_ACCENT } else { TX_SECONDARY }, label);
    }

    // ... its end: the reset button
    fn ied_row_end(&mut self, k: Key) {
        self.ied_reset_button(k, ID_RESET + k.ord());
        self.box_end();
    }

    // a colour key's fields in a row: swatch, hex text box (taken on Enter), the picker's switch
    fn ied_color_row(&mut self, k: Key, c: u32, acc: bool) {
        let id = if acc { ID_ACCF } else { ID_TOKF };
        let mut f = core::mem::take(if acc { &mut self.m.ied.accf } else { &mut self.m.ied.tokf });
        // the text follows the colour unless it is being edited
        let of = (k.ord() as u64) << 32 | c as u64;
        if self.wp.ctx.focus != id && f.of != of {
            f.hex = color_text(c);
            f.of = of;
        }
        self.ied_well(c, 28, 22);
        self.next_size(-1, 0);
        if self.textbox(id, Some("#RRGGBB"), &mut f.hex, 16) & EV_COMMIT != 0 {
            match parse_color(f.hex.trim()) {
                Some(n) if n != c => self.ied_want(k, Some(Val::Color(Some(n))), false),
                _ => f.of = 0, // (shown again)
            }
        }
        self.next_flags(ST_SUBTLE | ST_ICON_ONLY);
        self.next_icon(IC_COLOR);
        self.toggle_button(id + 1, tr("Farbwähler"), &mut f.pick);
        if acc {
            self.m.ied.accf = f;
        } else {
            self.m.ied.tokf = f;
        }
    }

    // ... the picker below it, when it is switched on (its moves while held are one step)
    fn ied_color_picker(&mut self, k: Key, c: u32, acc: bool) {
        let (id, on) = if acc { (ID_ACCF + 2, self.m.ied.accf.pick) } else { (ID_TOKF + 2, self.m.ied.tokf.pick) };
        if on == 0 {
            return;
        }
        let mut v = c;
        let ev = self.color_picker_ex(id, &mut v, if acc { 0 } else { CP_ALPHA });
        if ev & EV_ACT != 0 && v != c {
            self.ied_want(k, Some(Val::Color(Some(v))), true);
        }
        if ev & EV_COMMIT != 0 {
            self.m.ied.want_end = true;
        }
    }

    /// (inspect.rs, tab "Farben") the theme's tokens, editable: the selected token's colour
    /// (hex text, picker, reset), the list (changed ones marked) -> true (the tab is drawn)
    pub(crate) fn ied_colors(&mut self) -> bool {
        self.ied_bar();
        let mode = self.ied_mode();
        let t = self.m.ied.tok.clamp(0, T_COUNT as i32 - 1) as usize;
        let k = self.ied_tok_key(t);
        let c = self.ied_color_now(k, t);
        let changed = self.m.ied.over.contains_key(&k);
        self.box_begin(0, BOX_H | BOX_XCENTER, 8);
        self.next_size(-1, 0);
        let name = format!("T_{}  ·  {}  ({})", TOKEN_NAMES[t], file_key(k), mode_name(mode));
        self.text(TS_BODY_STRONG | TX_ELLIPSIS | if changed { TX_ACCENT } else { 0 }, &name);
        if changed {
            self.text(TS_CAPTION | TX_ACCENT, tr("geändert"));
        }
        self.ied_reset_button(k, ID_RESET + k.ord());
        self.box_end();
        self.box_begin(0, BOX_H | BOX_XCENTER, 8);
        self.ied_color_row(k, c, false);
        self.box_end();
        self.ied_color_picker(k, c, false);
        // the tokens
        self.next_size(-1, -1);
        let mut sel = t as i32;
        let (first, end) = self.list_begin(ID_TOKENS, &mut sel, T_COUNT as i32, ROW_H);
        for i in first..end {
            let iu = i as usize;
            let changed = self.m.ied.over.contains_key(&self.ied_tok_key(iu));
            let c = self.ied_tok(iu);
            self.list_item_begin(i);
            self.ied_well(c, 32, 18);
            self.next_size(150, 0);
            self.text(TS_CAPTION | if changed { TX_ACCENT } else { 0 }, &format!("T_{}", TOKEN_NAMES[iu]));
            self.next_size(-1, 0);
            self.text(TS_CAPTION | TX_SECONDARY, &format!("#{:08X}", c));
            if changed {
                self.text(TS_CAPTION | TX_ACCENT, tr("geändert"));
            }
            self.list_item_end();
        }
        let (_, s) = self.list_end(&mut sel);
        if s >= 0 {
            self.m.ied.tok = s;
        }
        true
    }

    /// (inspect.rs, tab "Darstellung") style, mode, accent, language, scales; corners, spacing,
    /// control height; font family and text sizes; the changes and taking them over
    pub(crate) fn ied_look(&mut self) {
        self.ied_bar();
        self.next_size(-1, -1);
        self.scroll_begin(ID_LOOK, SC_V);
        self.box_begin(0, BOX_V | pad(4), 6);
        self.ied_look_main();
        self.separator();
        self.ied_look_metrics();
        self.separator();
        self.ied_look_text();
        self.separator();
        self.ied_changes();
        self.box_end();
        self.scroll_end();
    }

    // style, mode, accent, language, scale, text scale
    fn ied_look_main(&mut self) {
        self.text(TS_BODY_STRONG, tr("Darstellung"));
        // the linked styles
        let names = self.style_names();
        let cur = self.style_name();
        let mut s = names.iter().position(|n| *n == cur).unwrap_or(0) as i32;
        self.ied_row(Key::Style, tr("Stil"));
        self.next_size(-1, 0);
        if self.combobox(ID_STYLE, &mut s, &names) & EV_ACT != 0 {
            if let Some(n) = names.get(s as usize) {
                self.ied_want(Key::Style, Some(Val::Style(n)), false);
            }
        }
        self.ied_row_end(Key::Style);
        // light / dark / high contrast
        let mut m = self.ied_mode() as i32;
        self.ied_row(Key::Mode, tr("Modus"));
        self.next_size(-1, 0);
        if self.combobox(ID_MODE, &mut m, &[mode_name(0), mode_name(1), mode_name(2)]) & EV_ACT != 0 {
            let dark = self.theme_base_dark;
            let v = match m as usize {
                THEME_LIGHT => Val::Mode(false, false, 2, false),
                THEME_DARK => Val::Mode(false, true, 2, false),
                _ => Val::Mode(false, dark, 1, true),
            };
            self.ied_want(Key::Mode, Some(v), false);
        }
        self.ied_row_end(Key::Mode);
        // the accent of these colours
        let mode = self.ied_mode();
        let k = Key::Accent(mode as u8);
        let c = self.ied_color_now(k, (T_ACCENT / 4) as usize);
        self.ied_row(k, &format!("{} ({})", tr("Akzent"), mode_name(mode)));
        self.ied_color_row(k, c, true);
        self.ied_row_end(k);
        self.ied_color_picker(k, c, true);
        // the UI language: the columns of the app's catalogue (else German and English)
        let codes = crate::i18n::language_codes();
        let labels: Vec<String> = codes.iter().map(|c| lang_name(c)).collect();
        let list: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        let now = crate::i18n::language_code();
        let mut l = codes.iter().position(|c| c.split(['-', '_']).next().unwrap_or(c).eq_ignore_ascii_case(&now)).unwrap_or(0) as i32;
        self.ied_row(Key::Lang, tr("Sprache"));
        self.next_size(-1, 0);
        if self.combobox(ID_LANG, &mut l, &list) & EV_ACT != 0 {
            if let Some(c) = codes.get(l as usize) {
                self.ied_want(Key::Lang, Some(Val::Text(c.clone())), false);
            }
        }
        self.ied_row_end(Key::Lang);
        // the main window's scale, the text scale
        let mut p = (self.win_dpi(0) * 100 + 48) / 96;
        self.ied_row(Key::Scale, tr("Skalierung"));
        self.next_size(-1, 0);
        if self.number_box(ID_SCALE, &mut p, &RG_SCALE) & EV_ACT != 0 {
            self.ied_want(Key::Scale, Some(Val::Num(scale_to_dpi(p))), false);
        }
        self.text(TS_BODY | TX_SECONDARY, "%");
        self.ied_row_end(Key::Scale);
        let mut p = self.ied_text_scale() as i32;
        self.ied_row(Key::TextScale, tr("Textskalierung"));
        self.next_size(-1, 0);
        if self.number_box(ID_TSCALE, &mut p, &RG_TSCALE) & EV_ACT != 0 {
            self.ied_want(Key::TextScale, Some(Val::Scale(p as u32, true)), false);
        }
        self.text(TS_BODY | TX_SECONDARY, "%");
        self.ied_row_end(Key::TextScale);
    }

    // corners, spacing, control height
    fn ied_look_metrics(&mut self) {
        let th = self.app_theme().cloned().unwrap_or_default();
        self.text(TS_BODY_STRONG, tr("Maße"));
        let mut v = match th.radius {
            Some(Radius::Dip(d)) => d as i32,
            _ => 4,
        };
        self.ied_row(Key::Radius, tr("Eckenradius"));
        self.next_size(-1, 0);
        let ev = self.slider(ID_RADIUS, &mut v, &RG_RADIUS);
        if ev & EV_ACT != 0 {
            self.ied_want(Key::Radius, Some(Val::Radius(Some(Radius::Dip(v as u16)))), true);
        }
        if ev & EV_COMMIT != 0 {
            self.m.ied.want_end = true;
        }
        self.next_size(64, 0);
        self.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &val_text(Key::Radius, &Val::Radius(th.radius)));
        self.ied_row_end(Key::Radius);
        let mut v = th.spacing as i32;
        self.ied_row(Key::Spacing, tr("Abstände"));
        self.next_size(-1, 0);
        let ev = self.slider(ID_SPACING, &mut v, &RG_SPACING);
        if ev & EV_ACT != 0 {
            self.ied_want(Key::Spacing, Some(Val::Num(v)), true);
        }
        if ev & EV_COMMIT != 0 {
            self.m.ied.want_end = true;
        }
        self.next_size(64, 0);
        self.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &format!("{} %", th.spacing));
        self.ied_row_end(Key::Spacing);
        let mut v = th.control_height as i32;
        self.ied_row(Key::Height, tr("Steuerelementhöhe"));
        self.next_size(-1, 0);
        if self.number_box(ID_HEIGHT, &mut v, &RG_HEIGHT) & EV_ACT != 0 {
            self.ied_want(Key::Height, Some(Val::Num(if v > 0 { v.max(12) } else { 0 })), false);
        }
        self.next_size(64, 0);
        self.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &val_text(Key::Height, &Val::Num(th.control_height as i32)));
        self.ied_row_end(Key::Height);
    }

    // the font family, the sizes of the text styles
    fn ied_look_text(&mut self) {
        let th = self.app_theme().cloned().unwrap_or_default();
        self.text(TS_BODY_STRONG, tr("Schrift"));
        // the family's text follows the theme unless it is being edited
        if self.wp.ctx.focus != ID_FAMILY && self.m.ied.family_of != th.family {
            self.m.ied.family = th.family.clone();
            self.m.ied.family_of = th.family.clone();
        }
        self.ied_row(Key::Family, tr("Familie"));
        self.next_size(-1, 0);
        let mut f = core::mem::take(&mut self.m.ied.family);
        if self.textbox(ID_FAMILY, Some(tr("vom Stil")), &mut f, 256) & EV_COMMIT != 0 && f.trim() != th.family {
            self.ied_want(Key::Family, Some(Val::Text(String::from(f.trim()))), false);
        }
        self.m.ied.family = f;
        self.ied_row_end(Key::Family);
        let sizes = self.style.ts_size_fx();
        for (i, name) in TS_NAMES.iter().enumerate() {
            let k = Key::Size(i as u8);
            let fx = th.text[i].size_fx.map_or(sizes[i] as i32, i32::from);
            let mut v = (fx + 128) >> 8;
            self.ied_row(k, &format!("{}.size", name));
            self.next_size(-1, 0);
            if self.number_box(ID_SIZE + i as u32, &mut v, &RG_SIZE) & EV_ACT != 0 {
                self.ied_want(k, Some(Val::Num(v << 8)), false);
            }
            self.text(TS_BODY | TX_SECONDARY, "DIP");
            self.ied_row_end(k);
        }
    }

    // the changes (old -> new, each with its reset) and taking them over
    fn ied_changes(&mut self) {
        self.text(TS_BODY_STRONG, tr("Änderungen"));
        let list: Vec<(Key, Val)> = self.m.ied.over.iter().map(|(k, v)| (*k, v.clone())).collect();
        if list.is_empty() {
            self.text(TS_BODY | TX_SECONDARY, tr("Keine Änderungen"));
        }
        for (k, v) in list {
            let old = self.m.ied.base.get(&k).map_or(String::new(), |b| b.1.clone());
            self.box_begin(0, BOX_H | BOX_XCENTER, 8);
            self.next_size(LABEL_W, 0);
            self.text(TS_BODY | TX_ELLIPSIS, &key_name(k));
            self.next_size(-1, 0);
            self.text(TS_BODY | TX_SECONDARY | TX_ELLIPSIS, &format!("{}  →  {}", old, val_text(k, &v)));
            self.ied_reset_button(k, ID_RESET2 + k.ord());
            self.box_end();
        }
        // take them over: a theme file, the list
        if self.m.ied.path.is_empty() {
            let p = if self.headless { "mem://inspector/inspector.theme" } else { "config://themes/inspector.theme" };
            self.m.ied.path = String::from(p);
        }
        self.box_begin(0, BOX_H | BOX_XCENTER, 8);
        self.next_size(-1, 0);
        let mut p = core::mem::take(&mut self.m.ied.path);
        self.textbox(ID_PATH, Some(tr("Pfad der Theme-Datei")), &mut p, 1024);
        self.m.ied.path = p;
        self.next_icon(IC_SAVE);
        if self.button(ID_SAVE, tr("Als Theme speichern")) & EV_ACT != 0 {
            let p = String::from(self.m.ied.path.trim());
            self.ied_save(&p);
        }
        self.box_end();
        self.box_begin(0, BOX_H | BOX_XCENTER, 8);
        self.next_icon(IC_COPY);
        if self.button(ID_COPY, tr("Liste kopieren")) & EV_ACT != 0 {
            self.ied_copy();
        }
        self.box_end();
    }

    /// (inspect.rs ins_props) the selected widget operated through synthetic input: a click,
    /// a drag, a value (sliders), typed text (text boxes), a command -- in a flyout of the
    /// button "Bedienen" (the tree keeps its room); the last one as a script line beside it
    pub(crate) fn ied_operate(&mut self, r: &Sem) {
        if r.kind == K_BOX {
            return; // (a layout box or a window)
        }
        // the fields follow the selection
        let of = (r.win, r.id, r.kind);
        if self.m.ied.op_of != of {
            let e = &mut self.m.ied;
            e.op_of = of;
            e.value = r.value;
            e.cmd = r.id as i32;
            e.text = if r.kind == K_TEXTBOX { r.label.clone().unwrap_or_default() } else { String::new() };
        }
        self.box_begin(0, BOX_H | BOX_XCENTER, 8);
        self.next_flags(ST_COMPACT);
        self.next_icon(IC_EDIT);
        self.dropdown_button(ID_OPS, tr("Bedienen"));
        if self.flyout_begin(ID_OPS, 0) {
            self.next_size(320, 0);
            self.box_begin(0, BOX_V | pad(8), 8);
            self.ied_operate_rows(r);
            self.box_end();
        }
        self.flyout_end();
        if !self.m.ied.last.is_empty() {
            let l = fill(tr("Skript: {}"), &[&self.m.ied.last]);
            self.next_size(-1, 0);
            self.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &l);
        }
        self.box_end();
    }

    // the flyout's rows: click; drag by dx, dy; a slider's value; text; a command
    fn ied_operate_rows(&mut self, r: &Sem) {
        if self.button(ID_CLICK, tr("Klicken")) & EV_ACT != 0 {
            self.ied_op_want(r, "click");
        }
        // a drag from its centre by dx, dy (DIP)
        self.box_begin(0, BOX_H | BOX_XCENTER, 8);
        for (id, label, v) in [(ID_DX, "dx", 0), (ID_DY, "dy", 1)] {
            self.text(TS_BODY | TX_SECONDARY, label);
            self.next_size(-1, 0);
            let mut n = if v == 0 { self.m.ied.dx } else { self.m.ied.dy };
            self.number_box(id, &mut n, &RG_DRAG);
            if v == 0 {
                self.m.ied.dx = n;
            } else {
                self.m.ied.dy = n;
            }
        }
        if self.button(ID_DRAG, tr("Ziehen")) & EV_ACT != 0 {
            self.ied_op_want(r, "drag");
        }
        self.box_end();
        // a slider's value
        if r.kind == K_SLIDER && r.max > r.min {
            self.box_begin(0, BOX_H | BOX_XCENTER, 8);
            self.next_size(OP_W, 0);
            self.text(TS_BODY | TX_SECONDARY, tr("Wert"));
            self.next_size(-1, 0);
            let mut v = self.m.ied.value;
            self.number_box(ID_VALUE, &mut v, &Range::new(r.min, r.max, 1, 0));
            self.m.ied.value = v;
            if self.button(ID_SET, tr("Setzen")) & EV_ACT != 0 {
                self.ied_op_want(r, "value");
            }
            self.box_end();
        }
        // text typed into a text box
        if r.kind == K_TEXTBOX {
            self.box_begin(0, BOX_H | BOX_XCENTER, 8);
            self.next_size(OP_W, 0);
            self.text(TS_BODY | TX_SECONDARY, tr("Text"));
            self.next_size(-1, 0);
            let mut t = core::mem::take(&mut self.m.ied.text);
            self.textbox(ID_TEXT, None, &mut t, 1024);
            self.m.ied.text = t;
            if self.button(ID_TYPE, tr("Tippen")) & EV_ACT != 0 {
                self.ied_op_want(r, "type");
            }
            self.box_end();
        }
        // a command (cmd ID)
        self.box_begin(0, BOX_H | BOX_XCENTER, 8);
        self.next_size(OP_W, 0);
        self.text(TS_BODY | TX_SECONDARY, tr("Befehl"));
        self.next_size(-1, 0);
        let mut c = self.m.ied.cmd;
        self.number_box(ID_CMD, &mut c, &RG_CMD);
        self.m.ied.cmd = c;
        if self.button(ID_RUN, tr("Ausführen")) & EV_ACT != 0 {
            self.ied_op_want(r, "cmd");
        }
        self.box_end();
    }

    /// (inspect.rs ins_ui) Ctrl+Z / Ctrl+Y in the tool window undo and redo the changes (a
    /// text box of the window keeps its own)
    pub(crate) fn ied_keys(&mut self) {
        let h = &self.m.ied.hist;
        if !h.can_undo() && !h.can_redo() {
            return; // (the keys go on to the app)
        }
        match self.undo_keys() {
            Some(true) => self.m.ied.act = A_UNDO,
            Some(false) => self.m.ied.act = A_REDO,
            None => {}
        }
    }

    // ------------------------------------------------------------------ script commands

    // KEY [VALUE] of `inspect set` / `inspect reset` / `inspect expect value` -> (the key, its
    // value (None without one), what follows the key)
    fn ied_parse<'a>(&self, a: &'a [u8], value: bool) -> Result<(Key, Option<Val>, &'a [u8]), &'static [u8]> {
        let r = skip_ws(next_word(a));
        let color = |p: &[u8]| parse_color(&word(p)).map(|c| Some(Val::Color(Some(c)))).ok_or(&b"expected #RRGGBB or #AARRGGBB"[..]);
        let num = |p: &[u8]| {
            let p = skip_ws(p);
            if p.first().is_some_and(|c| c.is_ascii_digit()) { Ok(parse_int(p).0) } else { Err(&b"expected a number"[..]) }
        };
        let mode = self.ied_mode() as u8;
        let (k, rest) = if word_eq(a, "token").is_some() {
            let name = word(r);
            let Some(t) = token_index(&name) else { return Err(b"no such token") };
            (self.ied_tok_key(t), skip_ws(next_word(r)))
        } else if word_eq(a, "accent").is_some() {
            (Key::Accent(mode), r)
        } else if word_eq(a, "size").is_some() {
            let name = word(r).to_ascii_lowercase();
            let Some(i) = TS_NAMES.iter().position(|n| *n == name) else { return Err(b"size caption|body|body_strong|subtitle|title|title_large|display") };
            (Key::Size(i as u8), skip_ws(next_word(r)))
        } else {
            const KEYS: [(&str, Key); 9] = [
                ("radius", Key::Radius),
                ("spacing", Key::Spacing),
                ("height", Key::Height),
                ("font", Key::Family),
                ("mode", Key::Mode),
                ("style", Key::Style),
                ("lang", Key::Lang),
                ("scale", Key::Scale),
                ("textscale", Key::TextScale),
            ];
            let Some(&(_, k)) = KEYS.iter().find(|(n, _)| word_eq(a, n).is_some()) else {
                return Err(b"KEY: token NAME|accent|radius|spacing|height|size STYLE|font|mode|style|lang|scale|textscale");
            };
            (k, r)
        };
        if !value {
            return Ok((k, None, rest));
        }
        let none = word_eq(rest, "none").is_some();
        let v = match k {
            Key::Token(..) | Key::Accent(_) => color(rest)?,
            Key::Radius if none => Some(Val::Radius(None)),
            Key::Radius => {
                let n = num(rest)?;
                let pct = word(rest).ends_with('%');
                Some(Val::Radius(Some(if pct { Radius::Percent(n.clamp(0, 400) as u16) } else { Radius::Dip(n.clamp(0, 64) as u16) })))
            }
            Key::Spacing | Key::Height => Some(Val::Num(num(rest)?)),
            Key::Size(_) => {
                let (x, used) = base::num::f64_parse(word(rest).as_bytes());
                if used == 0 || !(4.0..=200.0).contains(&x) {
                    return Err(b"expected a size from 4 to 200 (DIP)");
                }
                Some(Val::Num((x * 256.0 + 0.5) as i32))
            }
            Key::Family => Some(Val::Text(if none { String::new() } else { arg_text(rest) })),
            Key::Mode => {
                let dark = self.theme_base_dark;
                Some(match word(rest).as_str() {
                    "light" => Val::Mode(false, false, 2, false),
                    "dark" => Val::Mode(false, true, 2, false),
                    "contrast" => Val::Mode(false, dark, 1, true),
                    _ => return Err(b"mode light|dark|contrast"),
                })
            }
            Key::Style => {
                let name = word(rest);
                match self.style_names().into_iter().find(|n| *n == name) {
                    Some(n) => Some(Val::Style(n)),
                    None => return Err(b"no such style"),
                }
            }
            Key::Lang => {
                let code = word(rest);
                if code.is_empty() {
                    return Err(b"expected a language code");
                }
                Some(Val::Text(code))
            }
            Key::Scale => Some(Val::Num(scale_to_dpi(num(rest)?))),
            Key::TextScale => Some(Val::Scale(num(rest)?.clamp(50, 300) as u32, true)),
        };
        Ok((k, v, rest))
    }

    /// (inspect.rs sc_inspect) `inspect set|reset|undo|redo|save|list|do ...` -> handled
    pub(crate) fn ied_script(&mut self, app: &mut dyn App, a: &[u8]) -> bool {
        const CMDS: [&str; 7] = ["set", "reset", "undo", "redo", "save", "list", "do"];
        let Some(&c) = CMDS.iter().find(|c| word_eq(a, c).is_some()) else { return false };
        if !self.m.ins.on {
            self.fail(b"the inspector is closed");
            return true;
        }
        let r = skip_ws(next_word(a));
        match c {
            "set" => match self.ied_parse(r, true) {
                Ok((k, v, _)) => self.ied_edit(k, v, false),
                Err(m) => {
                    self.fail(m);
                    return true;
                }
            },
            "reset" if r.is_empty() || word_eq(r, "all").is_some() => {
                self.ied_step(A_RESET);
            }
            "reset" => match self.ied_parse(r, false) {
                Ok((k, _, _)) => self.ied_edit(k, None, false),
                Err(m) => {
                    self.fail(m);
                    return true;
                }
            },
            "undo" | "redo" => {
                if !self.ied_step(if c == "undo" { A_UNDO } else { A_REDO }) {
                    self.fail(if c == "undo" { b"nothing to undo" } else { b"nothing to redo" });
                }
            }
            "save" => {
                let p = arg_text(r);
                if !self.ied_save(&p) {
                    self.fail(b"not saved");
                }
            }
            "list" => self.ied_copy(),
            _ => {
                // the selected widget (cmd: any)
                let what = word(r);
                let sel = self.m.ins.sel_rec.clone();
                let op = match sel {
                    _ if what == "cmd" => {
                        let (id, rest) = parse_int(next_word(r));
                        let (arg, _) = parse_int(rest);
                        Some((Op::Cmd(id as u32, arg as u32), format!("cmd {}", id)))
                    }
                    None => {
                        self.fail(b"nothing selected");
                        return true;
                    }
                    Some(s) => {
                        let rest = next_word(r);
                        match what.as_str() {
                            "drag" => {
                                let (dx, rest) = parse_int(rest);
                                let (dy, _) = parse_int(rest);
                                self.m.ied.dx = dx;
                                self.m.ied.dy = dy;
                            }
                            "value" => self.m.ied.value = parse_int(rest).0,
                            "type" => match parse_qstring(skip_ws(rest), 1024) {
                                Some((t, _)) => self.m.ied.text = String::from_utf8_lossy(&t).into_owned(),
                                None => {
                                    self.fail(b"expected a quoted string");
                                    return true;
                                }
                            },
                            "click" => {}
                            _ => {
                                self.fail(b"inspect do click|drag DX DY|value N|type \"text\"|cmd ID [ARG]");
                                return true;
                            }
                        }
                        self.ied_op(&s, &what)
                    }
                };
                match op {
                    Some((op, line)) => {
                        self.m.ied.last = line;
                        self.ied_run(op);
                        self.process_events(app);
                        self.ied_later();
                        if plat::evq_count() != 0 {
                            self.process_events(app); // (a drag's release)
                        }
                    }
                    None => self.fail(b"the widget is not in this frame"),
                }
            }
        }
        if !self.m.ied.msg.is_empty() {
            plat::write_stdout(format!("inspect: {}\n", self.m.ied.msg).as_bytes());
        }
        self.run_passes(app);
        true
    }

    /// (inspect.rs sc_inspect_expect) `inspect expect changes N|list "TEXT"|undo on|off|redo
    /// on|off|message "TEXT"|none|value KEY "TEXT"` -> handled
    pub(crate) fn ied_expect(&mut self, a: &[u8]) -> bool {
        let r = skip_ws(next_word(a));
        if word_eq(a, "value").is_some() {
            // the key's value now, as the tool window shows it
            let (k, rest) = match self.ied_parse(r, false) {
                Ok((k, _, rest)) => (k, rest),
                Err(m) => {
                    self.fail(m);
                    return true;
                }
            };
            let have = self.ied_show_now(k);
            match parse_qstring(skip_ws(rest), 256) {
                Some((want, _)) if want == have.as_bytes() => {}
                Some(_) => self.fail(format!("value is {:?}", have).as_bytes()),
                None => self.fail(b"expected a quoted string"),
            }
        } else if word_eq(a, "changes").is_some() {
            let n = self.m.ied.over.len() as i32;
            if parse_int(r).0 != n {
                self.fail(format!("{} changes", n).as_bytes());
            }
        } else if word_eq(a, "undo").is_some() || word_eq(a, "redo").is_some() {
            let h = &self.m.ied.hist;
            let have = if word_eq(a, "undo").is_some() { h.can_undo() } else { h.can_redo() };
            if have != word_eq(r, "on").is_some() {
                self.fail(if have { b"there is a step" } else { b"there is no step" });
            }
        } else if word_eq(a, "list").is_some() || word_eq(a, "message").is_some() {
            let have = if word_eq(a, "list").is_some() { self.ied_list() } else { self.m.ied.msg.clone() };
            if word_eq(a, "message").is_some() && word_eq(r, "none").is_some() {
                if !have.is_empty() {
                    self.fail(format!("message: {}", have).as_bytes());
                }
                return true;
            }
            let Some((want, _)) = parse_qstring(r, 1024) else {
                self.fail(b"expected a quoted string");
                return true;
            };
            if !have.contains(String::from_utf8_lossy(&want).as_ref()) {
                self.fail(format!("{:?}", have).as_bytes());
            }
        } else {
            return false;
        }
        true
    }
}
