// term -- a terminal / console widget: a scrollback ring with an ANSI subset, the input line on
// textbox_surface (IME, clipboard, selection), history, completion, IOS-style `?` help, a
// --More-- pager, busy mode with type-ahead, raw keys, mouse selection and search
//
//   TermState::new(scrollback) -> st     a console (the app keeps one per console); scrollback
//                                        = lines kept (at least 100)
//   st.write(&[u8]) / st.write_str(s)    output: UTF-8 with an ANSI subset -- SGR 0 1 2 4 7 22
//                                        24 27, colours 30-37 / 90-97 / 39, backgrounds 40-47 /
//                                        100-107 / 49 (38;5 / 48;5 / 38;2 mapped to the 16);
//                                        \r \n \t BS, BEL (the console flashes), ESC[K (0 1 2),
//                                        ESC[2J (the screen scrolls clear), ESC[3J (the
//                                        scrollback too), ESC[H, ESC[nC / nD / nG; OSC skipped
//   st.clear()                           drop the scrollback and the current line
//   st.set_prompt(s)                     the prompt before the input (SGR allowed)
//   st.set_mode(TM_*)                    TM_LINE the input line; TM_BUSY a command runs: no
//                                        input line, typed lines wait (type-ahead) and come
//                                        back one Submit at a time in TM_LINE; TM_RAW every key
//                                        goes to the app (TermEv::Raw); TM_SECRET a line typed
//                                        without echo or history (passwords)
//   st.input_set(s, caret)               replace the input line (caret: a byte index)
//   st.complete_reply(line, list)        the answer to TermEv::Complete: the new line, and
//                                        candidates listed in columns below the old one
//   st.pager(on)                         on: the following output pages -- a screen minus one
//                                        line (st.page_len lines when set, as "terminal
//                                        length"), then pager_text (" --More-- "): Space a page,
//                                        Enter a line, q / Esc / Ctrl+C TermEv::PagerQuit (later
//                                        output is dropped until pager(false)); off: the app
//                                        wrote everything (a held rest still pages)
//   st.grid() -> (cols, rows)            the character grid of the last layout
//   st.text() -> String                  scrollback and the current line as plain text
//   ui.terminal(id, &mut st, TF_*) -> Vec<TermEv>   the console, filling the place it gets
//                                        (natural 480 x 240 DIP; next_size(-1, -1) fills);
//                                        ids id .. id + 7 (id + 1: the input line)
//   ui.mono_font_set(font)               the monospace font of every console (an id from
//                                        ui.text.load_mem; apps embed DejaVu Sans Mono)
//   ui.mono_font(size_pt) -> (font, cell_w)   that font, else font_load("monospace"), else
//                                        FONT_REG drawn cell by cell; the size is adjusted so
//                                        that a cell is a whole number of pixels
//
// Keys while focused: Enter submits (the line is echoed after the prompt and kept in the
// history), Tab TermEv::Complete, Up / Down the history, Home / End; TF_QHELP: a typed `?` is
// not inserted but gives TermEv::Help (the line up to the caret); Ctrl+C copies a selection,
// else TermEv::Interrupt (echoes ^C); TF_CTRL_SHIFT6: Ctrl+Shift+6 interrupts too; Ctrl+D on an
// empty line TermEv::Eof, Ctrl+Z on an empty line and Ctrl+L TermEv::Chord; Ctrl+U clears the
// line before the caret; Ctrl+Shift+C copies; Ctrl+V / Ctrl+Shift+V paste (every pasted line
// is one Submit); PgUp / PgDn (also with Shift), Ctrl+Up / Down, Ctrl+Home / End scroll;
// Ctrl+F the search bar (F3 / Enter: the next older match, Shift+F3 / Shift+Enter: newer, Esc
// closes it).  While busy or paging the console holds the keyboard as a text box would: plain
// keys are no accelerators, the undo keys stay with it (the app's undo does not run).  Mouse:
// a drag selects in the scrollback, a double click a word, the wheel scrolls; scrolled up,
// new output shows a "Neue Ausgabe" button that jumps down.  Every
// visible line is a K_TEXT record (`expect text:"..." exists`), the input line the K_TEXTBOX
// id + 1.  Colours: a 16-colour palette from the theme tokens (light / dark).  Every character
// takes one cell (wide CJK characters overlap the next); right-to-left the rows stay LTR.
//
// The visible rows are drawn by two cached Draw objects (scrollback, current line + input),
// replaced only when what they show changes, so typing redraws only the input rows.

use alloc::collections::VecDeque;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use crate::cmd::{Draw, DrawRef, Gfx};
use crate::consts::*;
use crate::keys::*;
use crate::theme::color_mix;
use crate::{FontSel, TextSurfaceLine, Ui};

/// TermState::mode: the input line
pub const TM_LINE: u8 = 0;
/// TermState::mode: a command runs -- no input line, typed text waits (type-ahead)
pub const TM_BUSY: u8 = 1;
/// TermState::mode: every key and character goes to the app (TermEv::Raw)
pub const TM_RAW: u8 = 2;
/// TermState::mode: a line typed without echo or history (passwords)
pub const TM_SECRET: u8 = 3;

pub const TF_QHELP: u32 = 1; // a typed `?` asks for help (TermEv::Help, IOS)
pub const TF_CTRL_SHIFT6: u32 = 2; // Ctrl+Shift+6 interrupts too (IOS)
pub const TF_NO_INPUT: u32 = 4; // output only: no input line
pub const TF_NO_FRAME: u32 = 8; // no border, a small padding

pub const TK_KEY: u32 = 0x8000_0000; // TermEv::Raw: a key (VK | MOD_* << 16), not a character

const DEF: u8 = 255; // the default colour of Attr
const A_BOLD: u8 = 1;
const A_DIM: u8 = 2;
const A_UNDER: u8 = 4;
const A_INV: u8 = 8;
const LINE_MAX: usize = 32768; // cells of a line before a forced break
const INPUT_CAP: u32 = 0x1_0000; // bytes of the input line
const HIST_MAX: usize = 500;
const PARAMS_MAX: usize = 16;
const UIT_TERM: u32 = 0x7FFE_0040; // timer: type-ahead goes on
const UIT_TERM_BELL: u32 = 0x7FFE_0041; // timer: the bell flash ends
const BELL_MS: u32 = 150;

/// what ui.terminal reports
#[derive(Clone, Debug, PartialEq)]
pub enum TermEv {
    /// Enter: the line (already echoed after the prompt and kept in the history)
    Submit(String),
    /// Tab: the input line and the caret (byte index); answer with complete_reply
    Complete { line: String, caret: usize },
    /// TF_QHELP: `?` was typed; the line up to the caret (echoed with the `?`)
    Help(String),
    /// Ctrl+C without a selection (TF_CTRL_SHIFT6: also Ctrl+Shift+6)
    Interrupt,
    /// Ctrl+D on an empty line
    Eof,
    /// Ctrl+Z on an empty line, Ctrl+L: VK | MOD_* << 16
    Chord(u32),
    /// the pager was quit (q, Esc, Ctrl+C)
    PagerQuit,
    /// the character grid changed (also at the first layout): columns, rows
    Resize(u16, u16),
    /// TM_RAW: characters, and keys as TK_KEY | VK | MOD_* << 16; Enter 13, Tab 9,
    /// Backspace 127, Esc 27, Ctrl+letter 1..26
    Raw(Vec<u32>),
}

// ------------------------------------------------------------------ the screen

// SGR state of a cell: colours 0..15 or DEF, A_* flags
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Attr {
    fg: u8,
    bg: u8,
    fl: u8,
}

const ATTR0: Attr = Attr { fg: DEF, bg: DEF, fl: 0 };

#[derive(Clone, Copy, PartialEq)]
struct Cell {
    ch: char,
    a: Attr,
}

// a finished line: its text (one char per cell), the attribute changes (column, attributes;
// ATTR0 before the first), its cells
struct Line {
    text: String,
    spans: Vec<(u32, Attr)>,
    cells: u32,
}

impl Line {
    fn of(cells: &[Cell]) -> Line {
        let mut text = String::with_capacity(cells.len());
        let mut spans = Vec::new();
        let mut a = ATTR0;
        for (i, c) in cells.iter().enumerate() {
            if c.a != a {
                spans.push((i as u32, c.a));
                a = c.a;
            }
            text.push(c.ch);
        }
        Line { text, spans, cells: cells.len() as u32 }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Esc {
    No,
    Esc,
    Csi,
    Osc,
    OscEsc,
}

// rows of a line wrapped at cols (an empty line takes one)
fn line_rows(cells: u32, cols: usize) -> u64 {
    if cells == 0 {
        1
    } else {
        (cells as u64).div_ceil(cols.max(1) as u64)
    }
}

// the scrollback ring, the line being written and the ANSI parser
struct Screen {
    lines: VecDeque<Line>,
    row0: VecDeque<u64>, // the first wrapped row of each line (absolute)
    row_end: u64,        // the row after the last line (the current line starts there)
    base: u64,           // the absolute index of lines[0]
    cap: usize,
    cols: usize,
    cur: Vec<Cell>,
    col: usize,
    attr: Attr,
    esc: Esc,
    params: Vec<u32>,
    param: Option<u32>,
    utf: u32,
    utf_need: u8,
    utf_min: u32,
    floor: u64, // the screen starts at this line (ESC[2J)
    bell: bool,
    added: u64, // lines added so far
    ver: u64,   // bumps with every change
}

impl Screen {
    fn new(cap: usize) -> Screen {
        Screen {
            lines: VecDeque::new(),
            row0: VecDeque::new(),
            row_end: 0,
            base: 0,
            cap: cap.max(100),
            cols: 80,
            cur: Vec::new(),
            col: 0,
            attr: ATTR0,
            esc: Esc::No,
            params: Vec::new(),
            param: None,
            utf: 0,
            utf_need: 0,
            utf_min: 0,
            floor: 0,
            bell: false,
            added: 0,
            ver: 0,
        }
    }

    fn first_row(&self) -> u64 {
        self.row0.front().copied().unwrap_or(self.row_end)
    }

    // the absolute index after the last line
    fn end(&self) -> u64 {
        self.base + self.lines.len() as u64
    }

    fn line(&self, abs: u64) -> Option<&Line> {
        abs.checked_sub(self.base).and_then(|i| self.lines.get(i as usize))
    }

    // the first row of line abs (lines before the ring: the first row; after: the end)
    fn row_of(&self, abs: u64) -> u64 {
        match abs.checked_sub(self.base) {
            None => self.first_row(),
            Some(i) => self.row0.get(i as usize).copied().unwrap_or(self.row_end),
        }
    }

    // the line that shows absolute row r (None: r is past the lines)
    fn line_at_row(&self, r: u64) -> Option<u64> {
        if r >= self.row_end || self.lines.is_empty() {
            return None;
        }
        let i = self.row0.partition_point(|&x| x <= r);
        Some(self.base + i.max(1) as u64 - 1)
    }

    // wrap at another width: the rows are counted again from 0
    fn set_cols(&mut self, cols: usize) {
        let cols = cols.max(1);
        if cols == self.cols {
            return;
        }
        self.cols = cols;
        let mut r = 0;
        for (i, l) in self.lines.iter().enumerate() {
            self.row0[i] = r;
            r += line_rows(l.cells, cols);
        }
        self.row_end = r;
        self.ver += 1;
    }

    fn push(&mut self, l: Line) {
        self.row0.push_back(self.row_end);
        self.row_end += line_rows(l.cells, self.cols);
        self.lines.push_back(l);
        self.added += 1;
        while self.lines.len() > self.cap {
            self.lines.pop_front();
            self.row0.pop_front();
            self.base += 1;
        }
    }

    fn newline(&mut self) {
        let cells = core::mem::take(&mut self.cur);
        self.col = 0;
        self.push(Line::of(&cells));
    }

    fn clear(&mut self) {
        self.base = self.end();
        self.lines.clear();
        self.row0.clear();
        self.cur.clear();
        self.col = 0;
        self.floor = self.base;
        self.ver += 1;
    }

    fn write(&mut self, s: &[u8]) {
        if s.is_empty() {
            return;
        }
        for &b in s {
            self.feed(b);
        }
        self.ver += 1;
    }

    fn put(&mut self, ch: char) {
        if self.col >= LINE_MAX {
            self.newline();
        }
        let cell = Cell { ch, a: self.attr };
        if self.col < self.cur.len() {
            self.cur[self.col] = cell;
        } else {
            while self.cur.len() < self.col {
                self.cur.push(Cell { ch: ' ', a: ATTR0 });
            }
            self.cur.push(cell);
        }
        self.col += 1;
    }

    fn feed(&mut self, b: u8) {
        match self.esc {
            Esc::No => {}
            Esc::Esc => {
                self.esc = match b {
                    b'[' => {
                        self.params.clear();
                        self.param = None;
                        Esc::Csi
                    }
                    b']' => Esc::Osc,
                    _ => Esc::No, // other escapes (charsets, keypad modes) are skipped
                };
                return;
            }
            Esc::Csi => {
                match b {
                    b'0'..=b'9' => {
                        let v = self.param.unwrap_or(0) * 10 + (b - b'0') as u32;
                        self.param = Some(v.min(65535));
                    }
                    b';' | b':' => {
                        if self.params.len() < PARAMS_MAX {
                            self.params.push(self.param.unwrap_or(0));
                        }
                        self.param = None;
                    }
                    0x40..=0x7E => {
                        if let Some(p) = self.param.take() {
                            if self.params.len() < PARAMS_MAX {
                                self.params.push(p);
                            }
                        }
                        self.esc = Esc::No;
                        self.csi(b);
                    }
                    0x1B => self.esc = Esc::Esc,
                    _ => {} // private markers, intermediates
                }
                return;
            }
            Esc::Osc => {
                if b == 7 {
                    self.esc = Esc::No;
                } else if b == 0x1B {
                    self.esc = Esc::OscEsc;
                }
                return;
            }
            Esc::OscEsc => {
                self.esc = if b == b'\\' { Esc::No } else { Esc::Osc };
                return;
            }
        }
        if self.utf_need != 0 {
            if b & 0xC0 == 0x80 {
                self.utf = self.utf << 6 | (b & 0x3F) as u32;
                self.utf_need -= 1;
                if self.utf_need == 0 {
                    let c = self.utf;
                    let ok = c >= self.utf_min && c <= 0x10FFFF && c & 0xFFFF_F800 != 0xD800;
                    self.put(if ok { char::from_u32(c).unwrap_or('\u{FFFD}') } else { '\u{FFFD}' });
                }
                return;
            }
            self.utf_need = 0; // a cut sequence
            self.put('\u{FFFD}');
        }
        match b {
            0x1B => self.esc = Esc::Esc,
            b'\n' => self.newline(),
            b'\r' => self.col = 0,
            b'\t' => {
                let to = (self.col / 8 + 1) * 8;
                while self.col < to {
                    self.put(' ');
                }
            }
            8 => self.col = self.col.saturating_sub(1),
            7 => self.bell = true,
            0..=0x1F | 0x7F => {}
            0x20..=0x7E => self.put(b as char),
            0xC2..=0xDF => self.utf_begin(b & 0x1F, 1, 0x80),
            0xE0..=0xEF => self.utf_begin(b & 0x0F, 2, 0x800),
            0xF0..=0xF4 => self.utf_begin(b & 7, 3, 0x10000),
            _ => self.put('\u{FFFD}'),
        }
    }

    fn utf_begin(&mut self, bits: u8, need: u8, min: u32) {
        self.utf = bits as u32;
        self.utf_need = need;
        self.utf_min = min;
    }

    fn arg(&self, i: usize, default: u32) -> u32 {
        match self.params.get(i) {
            Some(&v) if v != 0 => v,
            _ => default,
        }
    }

    fn csi(&mut self, f: u8) {
        match f {
            b'm' => self.sgr(),
            b'K' => match self.params.first().copied().unwrap_or(0) {
                0 => self.cur.truncate(self.col),
                1 => {
                    let n = (self.col + 1).min(self.cur.len());
                    for c in &mut self.cur[..n] {
                        *c = Cell { ch: ' ', a: ATTR0 };
                    }
                }
                _ => self.cur.clear(),
            },
            b'J' => match self.params.first().copied().unwrap_or(0) {
                2 => {
                    // the screen scrolls clear: what was on it stays in the scrollback
                    if !self.cur.is_empty() {
                        self.newline();
                    }
                    self.col = 0;
                    self.floor = self.end();
                }
                3 => self.clear(),
                _ => {}
            },
            b'H' | b'f' => self.col = self.arg(1, 1) as usize - 1,
            b'G' => self.col = self.arg(0, 1) as usize - 1,
            b'C' => self.col = (self.col + self.arg(0, 1) as usize).min(LINE_MAX - 1),
            b'D' => self.col = self.col.saturating_sub(self.arg(0, 1) as usize),
            _ => {}
        }
    }

    fn sgr(&mut self) {
        if self.params.is_empty() {
            self.attr = ATTR0;
            return;
        }
        let p = core::mem::take(&mut self.params);
        let mut i = 0;
        while i < p.len() {
            let a = &mut self.attr;
            match p[i] {
                0 => *a = ATTR0,
                1 => a.fl |= A_BOLD,
                2 => a.fl |= A_DIM,
                4 => a.fl |= A_UNDER,
                7 => a.fl |= A_INV,
                22 => a.fl &= !(A_BOLD | A_DIM),
                24 => a.fl &= !A_UNDER,
                27 => a.fl &= !A_INV,
                v @ 30..=37 => a.fg = (v - 30) as u8,
                39 => a.fg = DEF,
                v @ 40..=47 => a.bg = (v - 40) as u8,
                49 => a.bg = DEF,
                v @ 90..=97 => a.fg = (v - 90 + 8) as u8,
                v @ 100..=107 => a.bg = (v - 100 + 8) as u8,
                v @ (38 | 48) => {
                    // 38;5;n / 38;2;r;g;b: the nearest of the 16 colours
                    let c = match p.get(i + 1) {
                        Some(5) => {
                            i += 2;
                            p.get(i).map(|&n| ansi256(n))
                        }
                        Some(2) => {
                            i += 4;
                            match (p.get(i - 2), p.get(i - 1), p.get(i)) {
                                (Some(&r), Some(&g), Some(&b)) => Some(rgb16(r.min(255) / 51, g.min(255) / 51, b.min(255) / 51)),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    if let Some(c) = c {
                        if v == 38 {
                            a.fg = c;
                        } else {
                            a.bg = c;
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
        self.params = p;
    }
}

// a colour of the 6x6x6 cube (levels 0..5) as one of the 16
fn rgb16(r: u32, g: u32, b: u32) -> u8 {
    let bits = (r > 2) as u8 | ((g > 2) as u8) << 1 | ((b > 2) as u8) << 2;
    let hi = r.max(g).max(b);
    if bits == 0 {
        return if hi >= 2 { 8 } else { 0 };
    }
    if hi >= 5 { bits + 8 } else { bits }
}

// xterm's 256 colours as one of the 16
fn ansi256(n: u32) -> u8 {
    match n {
        0..=15 => n as u8,
        16..=231 => {
            let v = n - 16;
            rgb16(v / 36, v / 6 % 6, v % 6)
        }
        232..=243 => 8,
        _ => 7,
    }
}

// ------------------------------------------------------------------ the state

/// the state of one console (see the module header); the app keeps it between frames
pub struct TermState {
    /// the text before the input (SGR allowed)
    pub prompt: String,
    /// TM_LINE, TM_BUSY, TM_RAW, TM_SECRET
    pub mode: u8,
    /// TF_* added to the flags of ui.terminal
    pub flags: u32,
    /// a font id for this console (None: ui.mono_font)
    pub font: Option<u32>,
    /// the text size in points (default 10)
    pub size_pt: f32,
    /// the pager's line (default " --More-- ")
    pub pager_text: String,
    /// lines of output per page of the pager (0, the default: the screen minus one line)
    pub page_len: u16,
    scr: Screen,
    input: String,
    caret_want: Option<usize>,
    history: Vec<String>,
    hist_pos: usize,
    hist_draft: String,
    ta: String, // type-ahead: text typed while busy, '\n' = Enter
    ta_at: Option<u32>, // when the last line was submitted (ms)
    pg_open: bool,
    pg_quit: bool,
    pg_held: Vec<u8>,
    pg_left: u32,
    find_open: bool,
    find_text: String,
    find_hits: Vec<(u64, u32, u32)>, // line, first column, end column
    find_idx: usize,
    find_key: Option<(String, u64)>, // what find_hits were made for: the text, Screen::ver
    sel: Option<((u64, u32), (u64, u32))>, // anchor, end: (line, column)
    sel_drag: bool,
    at_bottom: bool,
    top: u64,
    unseen: bool,
    seen_added: u64,
    cols: u16,
    rows: u16,
    scroll_req: i64,
    bell_until: u32,
    prompt_cells: Option<(String, Vec<Cell>)>,
    body: Option<(Rc<Snap>, u32)>,
    tail: Option<(Rc<Snap>, u32)>,
}

impl TermState {
    /// a console keeping `scrollback` lines (at least 100)
    pub fn new(scrollback: usize) -> TermState {
        TermState {
            prompt: String::new(),
            mode: TM_LINE,
            flags: 0,
            font: None,
            size_pt: 10.0,
            pager_text: String::from(" --More-- "),
            page_len: 0,
            scr: Screen::new(scrollback),
            input: String::new(),
            caret_want: None,
            history: Vec::new(),
            hist_pos: 0,
            hist_draft: String::new(),
            ta: String::new(),
            ta_at: None,
            pg_open: false,
            pg_quit: false,
            pg_held: Vec::new(),
            pg_left: 0,
            find_open: false,
            find_text: String::new(),
            find_hits: Vec::new(),
            find_idx: 0,
            find_key: None,
            sel: None,
            sel_drag: false,
            at_bottom: true,
            top: 0,
            unseen: false,
            seen_added: 0,
            cols: 0,
            rows: 0,
            scroll_req: 0,
            bell_until: 0,
            prompt_cells: None,
            body: None,
            tail: None,
        }
    }

    /// output: UTF-8 with the ANSI subset of the module header (paged while the pager is on)
    pub fn write(&mut self, s: &[u8]) {
        if self.pg_open && self.pg_quit {
            return; // the pager was quit: the rest of the paged output is dropped
        }
        if self.pg_open || !self.pg_held.is_empty() {
            self.pg_feed(s);
        } else {
            self.scr.write(s);
        }
    }

    /// write(s.as_bytes())
    pub fn write_str(&mut self, s: &str) {
        self.write(s.as_bytes());
    }

    /// drop the scrollback and the current line (the history stays)
    pub fn clear(&mut self) {
        self.scr.clear();
        self.sel = None;
        self.at_bottom = true;
        self.find_key = None;
    }

    /// the prompt before the input (SGR allowed)
    pub fn set_prompt(&mut self, s: &str) {
        if self.prompt != s {
            self.prompt = String::from(s);
        }
    }

    /// TM_LINE, TM_BUSY, TM_RAW or TM_SECRET
    pub fn set_mode(&mut self, mode: u8) {
        self.mode = mode;
    }

    /// replace the input line; caret: a byte index (clamped to a character start)
    pub fn input_set(&mut self, s: &str, caret: usize) {
        self.input = String::from(s);
        let mut c = caret.min(s.len());
        while !s.is_char_boundary(c) {
            c -= 1;
        }
        self.caret_want = Some(c);
    }

    /// the input line as typed so far
    pub fn input(&self) -> &str {
        &self.input
    }

    /// the lines submitted so far (oldest first)
    pub fn history(&self) -> &[String] {
        &self.history
    }

    /// the answer to TermEv::Complete: the new input line (caret at its end); a non-empty
    /// list is shown in columns below the line as typed
    pub fn complete_reply<S: AsRef<str>>(&mut self, line: &str, list: &[S]) {
        if !list.is_empty() {
            let typed = self.input.clone();
            self.echo(&typed, "");
            let w = list.iter().map(|s| s.as_ref().chars().count()).max().unwrap_or(0) + 2;
            let per = (self.cols.max(1) as usize / w).max(1);
            let mut out = String::new();
            for (i, s) in list.iter().enumerate() {
                let s = s.as_ref();
                out.push_str(s);
                if (i + 1) % per == 0 || i + 1 == list.len() {
                    out.push('\n');
                } else {
                    for _ in s.chars().count()..w {
                        out.push(' ');
                    }
                }
            }
            self.scr.write(out.as_bytes());
        }
        self.input_set(line, line.len());
        self.at_bottom = true;
    }

    /// on: the following output pages (see the module header); off: the app wrote all of it
    pub fn pager(&mut self, on: bool) {
        if on {
            self.pg_open = true;
            self.pg_quit = false;
            let held = core::mem::take(&mut self.pg_held);
            self.scr.write(&held);
            self.pg_left = self.page();
        } else {
            self.pg_open = false;
            if self.pg_quit {
                self.pg_held.clear();
                self.pg_quit = false;
            }
        }
    }

    /// the pager holds output and waits for a key
    pub fn pager_waiting(&self) -> bool {
        !self.pg_held.is_empty()
    }

    /// the character grid of the last layout (columns, rows; 0 before the first)
    pub fn grid(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }

    /// the scrollback and the current line as plain text (lines joined with '\n')
    pub fn text(&self) -> String {
        let mut s = String::new();
        for l in &self.scr.lines {
            s.push_str(&l.text);
            s.push('\n');
        }
        s.extend(self.scr.cur.iter().map(|c| c.ch));
        s
    }

    /// lines kept in the scrollback
    pub fn line_count(&self) -> usize {
        self.scr.lines.len()
    }

    // the lines of a page: page_len, else the screen minus one line (23 before the first layout)
    fn page(&self) -> u32 {
        match (self.page_len, self.rows) {
            (0, 0) => 23,
            (0, r) => (r.max(2) as u32 - 1).max(1),
            (n, _) => n as u32,
        }
    }

    // paged output: lines pass while the page has room, the rest is held
    fn pg_feed(&mut self, s: &[u8]) {
        if !self.pg_held.is_empty() {
            self.pg_held.extend_from_slice(s);
            return;
        }
        let mut i = 0;
        while i < s.len() {
            if self.pg_left == 0 {
                self.pg_held.extend_from_slice(&s[i..]);
                break;
            }
            let end = s[i..].iter().position(|&b| b == b'\n').map_or(s.len(), |p| i + p + 1);
            self.scr.write(&s[i..end]);
            if s[end - 1] == b'\n' {
                self.pg_left -= 1;
            }
            i = end;
        }
    }

    // the pager goes on by n lines
    fn pg_release(&mut self, n: u32) {
        self.pg_left = n.max(1);
        let held = core::mem::take(&mut self.pg_held);
        self.pg_feed(&held);
        self.at_bottom = true;
    }

    fn pg_quit_now(&mut self, evs: &mut Vec<TermEv>) {
        self.pg_held.clear();
        if self.pg_open {
            self.pg_quit = true;
        }
        evs.push(TermEv::PagerQuit);
        self.at_bottom = true;
    }

    // the current line, the prompt, the line (not in TM_SECRET) and a suffix, then a line
    // break, with the SGR state of the output kept aside
    fn echo(&mut self, line: &str, suffix: &str) {
        let attr = self.scr.attr;
        self.scr.attr = ATTR0;
        self.scr.col = self.scr.cur.len();
        let mut s = String::with_capacity(self.prompt.len() + line.len() + suffix.len() + 1);
        s.push_str(&self.prompt);
        s.push_str("\x1b[0m");
        if self.mode != TM_SECRET {
            s.push_str(line);
        }
        s.push_str(suffix);
        s.push('\n');
        self.scr.write(s.as_bytes());
        self.scr.attr = attr;
    }

    fn submit(&mut self, line: String, evs: &mut Vec<TermEv>) {
        self.echo(&line, "");
        if self.mode != TM_SECRET && !line.trim().is_empty() && self.history.last() != Some(&line) {
            self.history.push(line.clone());
            if self.history.len() > HIST_MAX {
                self.history.remove(0);
            }
        }
        self.hist_pos = self.history.len();
        self.hist_draft.clear();
        self.at_bottom = true;
        evs.push(TermEv::Submit(line));
    }

    fn hist_move(&mut self, older: bool) {
        let n = self.history.len();
        if older {
            if self.hist_pos == 0 || n == 0 {
                return;
            }
            if self.hist_pos >= n {
                self.hist_pos = n;
                self.hist_draft = self.input.clone();
            }
            self.hist_pos -= 1;
            self.input = self.history[self.hist_pos].clone();
        } else {
            if self.hist_pos >= n {
                return;
            }
            self.hist_pos += 1;
            self.input = if self.hist_pos == n { self.hist_draft.clone() } else { self.history[self.hist_pos].clone() };
        }
        self.caret_want = Some(self.input.len());
        self.at_bottom = true;
    }

    // one waiting line of the type-ahead (TM_LINE): -> it was submitted
    fn ta_release(&mut self, evs: &mut Vec<TermEv>) -> bool {
        if self.ta.is_empty() {
            return false;
        }
        match self.ta.find('\n') {
            Some(i) => {
                let mut line = core::mem::take(&mut self.input);
                line.push_str(&self.ta[..i]);
                self.ta.drain(..=i);
                self.submit(line, evs);
                true
            }
            None => {
                let rest = core::mem::take(&mut self.ta);
                self.input.push_str(&rest);
                self.caret_want = Some(self.input.len());
                false
            }
        }
    }

    // the selection, start before end
    fn sel_range(&self) -> Option<((u64, u32), (u64, u32))> {
        let (a, b) = self.sel?;
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        (a != b).then_some((a, b))
    }

    fn sel_text(&self) -> String {
        let Some((a, b)) = self.sel_range() else { return String::new() };
        let mut out = String::new();
        for l in a.0..=b.0 {
            let Some(line) = self.scr.line(l) else { continue };
            let c0 = if l == a.0 { a.1 } else { 0 } as usize;
            let c1 = if l == b.0 { b.1 } else { line.cells } as usize;
            let part: String = line.text.chars().skip(c0).take(c1.saturating_sub(c0)).collect();
            if c1 >= line.cells as usize {
                out.push_str(part.trim_end());
            } else {
                out.push_str(&part);
            }
            if l != b.0 {
                out.push('\n');
            }
        }
        out
    }

    // the word (or run of blanks / other characters) at a position
    fn word_at(&self, p: (u64, u32)) -> Option<((u64, u32), (u64, u32))> {
        let line = self.scr.line(p.0)?;
        let chars: Vec<char> = line.text.chars().collect();
        if chars.is_empty() {
            return None;
        }
        let k = (p.1 as usize).min(chars.len() - 1);
        let class = |c: char| -> u8 {
            if c.is_alphanumeric() || "_-./:@".contains(c) {
                1
            } else if c == ' ' {
                0
            } else {
                2
            }
        };
        let cl = class(chars[k]);
        let mut s = k;
        while s > 0 && class(chars[s - 1]) == cl {
            s -= 1;
        }
        let mut e = k + 1;
        while e < chars.len() && class(chars[e]) == cl {
            e += 1;
        }
        Some(((p.0, s as u32), (p.0, e as u32)))
    }

    // the matches of the search text (case-insensitive), redone when text or output changed
    fn find_update(&mut self) {
        let key = (self.find_text.clone(), self.scr.ver);
        if self.find_key.as_ref() == Some(&key) {
            return;
        }
        let fresh = self.find_key.as_ref().is_none_or(|k| k.0 != key.0);
        self.find_key = Some(key);
        let cur = self.find_hits.get(self.find_idx).copied();
        self.find_hits.clear();
        let needle: Vec<char> = self.find_text.chars().map(fold).collect();
        if needle.is_empty() {
            return;
        }
        for (i, l) in self.scr.lines.iter().enumerate() {
            let hay: Vec<char> = l.text.chars().map(fold).collect();
            let mut k = 0;
            while k + needle.len() <= hay.len() {
                if hay[k..k + needle.len()] == needle[..] {
                    self.find_hits.push((self.scr.base + i as u64, k as u32, (k + needle.len()) as u32));
                    k += needle.len();
                } else {
                    k += 1;
                }
            }
        }
        // a new text: the newest match; new output: the same match as before
        self.find_idx = match cur {
            Some(c) if !fresh => self.find_hits.iter().position(|h| *h == c).unwrap_or(self.find_hits.len().saturating_sub(1)),
            _ => self.find_hits.len().saturating_sub(1),
        };
        if fresh {
            self.find_show();
        }
    }

    // step to the next older (or newer) match
    fn find_step(&mut self, older: bool) {
        let n = self.find_hits.len();
        if n == 0 {
            return;
        }
        self.find_idx = if older { (self.find_idx + n - 1) % n } else { (self.find_idx + 1) % n };
        self.find_show();
    }

    // scroll the current match into view
    fn find_show(&mut self) {
        let Some(&(l, c, _)) = self.find_hits.get(self.find_idx) else { return };
        let row = self.scr.row_of(l) + c as u64 / self.scr.cols.max(1) as u64;
        let rows = self.rows.max(1) as u64;
        let top = if self.at_bottom { u64::MAX } else { self.top };
        if top == u64::MAX || row < top || row >= top + rows {
            self.at_bottom = false;
            self.top = row.saturating_sub(rows / 3);
        }
    }
}

// a character folded for the case-insensitive search
fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

// ------------------------------------------------------------------ drawing

// cells of one colour: the first column, cells, colours (bg 0: none), A_BOLD | A_UNDER
#[derive(Clone, PartialEq)]
struct Run {
    col: u32,
    n: u32,
    fg: u32,
    bg: u32,
    fl: u8,
    text: String,
}

// the visible rows of one part (scrollback, or current line + input) with their font
#[derive(PartialEq)]
struct Snap {
    font: u32,
    size: i32,
    cw: i32,
    lh: i32,
    base: i32,
    ul: (i32, i32),
    bold_dx: i32,
    per_cell: bool,
    rows: Vec<Vec<Run>>,
}

impl Draw for Snap {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        g.text.set_font(self.font, self.size);
        for (i, row) in self.rows.iter().enumerate() {
            let y = r[1] + i as i32 * self.lh;
            for run in row.iter().filter(|run| run.bg != 0) {
                g.dc.set_color(run.bg);
                g.dc.fill(g.sf, r[0] + run.col as i32 * self.cw, y, run.n as i32 * self.cw, self.lh);
            }
            for run in row {
                let x0 = r[0] + run.col as i32 * self.cw;
                g.dc.set_color(run.fg);
                let passes = if run.fl & A_BOLD != 0 { 2 } else { 1 };
                for p in 0..passes {
                    let dx = p * self.bold_dx;
                    let b = run.text.as_bytes();
                    let (mut i, mut col) = (0, 0);
                    while i < b.len() {
                        let s = i;
                        if !self.per_cell && b[i] < 0x80 {
                            // ASCII runs at once (the cell width is the font's advance)
                            while i < b.len() && b[i] < 0x80 {
                                i += 1;
                            }
                        } else {
                            // other characters cell by cell (fallback fonts may be wider)
                            i += match b[i] {
                                0..=0x7F => 1,
                                0x80..=0xDF => 2,
                                0xE0..=0xEF => 3,
                                _ => 4,
                            };
                            i = i.min(b.len());
                        }
                        if b[s..i] != *b" " {
                            g.text.draw(&mut *g.dc, &mut *g.sf, x0 + col * self.cw + dx, y + self.base, &b[s..i]);
                        }
                        col += if i - s > 1 && b[s] < 0x80 { (i - s) as i32 } else { 1 };
                    }
                }
                if run.fl & A_UNDER != 0 {
                    g.dc.fill(g.sf, x0, y + self.ul.0, run.n as i32 * self.cw, self.ul.1);
                }
            }
        }
    }
}

// builds the runs of a row cell by cell
#[derive(Default)]
struct Rb {
    runs: Vec<Run>,
}

impl Rb {
    fn push(&mut self, col: u32, ch: char, (fg, bg, fl): (u32, u32, u8)) {
        if let Some(r) = self.runs.last_mut() {
            if r.fg == fg && r.bg == bg && r.fl == fl && r.col + r.n == col {
                r.text.push(ch);
                r.n += 1;
                return;
            }
        }
        self.runs.push(Run { col, n: 1, fg, bg, fl, text: String::from(ch) });
    }
}

// the palette: 16 colours, default text and background, selection, search matches
struct Pal {
    c: [u32; 16],
    fg: u32,
    bg: u32,
    sel: u32,
    sel_fg: u32,
    hit: u32,
    hit_cur: u32,
}

// the brightness of 0x..RRGGBB (0..255)
fn luma(c: u32) -> u32 {
    ((c >> 16 & 255) * 299 + (c >> 8 & 255) * 587 + (c & 255) * 114) / 1000
}

// c over an opaque background
fn over(bg: u32, c: u32) -> u32 {
    let a = (c >> 24) as i32;
    color_mix(bg | 0xFF00_0000, c | 0xFF00_0000, a + (a >> 7))
}

impl Pal {
    // the colours of a cell (fg, bg or 0, the flags the drawing needs)
    fn cell(&self, a: Attr) -> (u32, u32, u8) {
        let mut fg = if a.fg == DEF { self.fg } else { self.c[(a.fg & 15) as usize] };
        let mut bg = if a.bg == DEF { 0 } else { self.c[(a.bg & 15) as usize] };
        if a.fg == DEF && bg != 0 && luma(fg).abs_diff(luma(bg)) < 110 {
            fg = self.bg; // the default text stays readable on a background of its brightness
        }
        if a.fl & A_DIM != 0 {
            fg = color_mix(fg, self.bg, 110);
        }
        if a.fl & A_INV != 0 {
            let b = if bg == 0 { self.bg } else { bg };
            bg = fg;
            fg = b;
        }
        (fg, bg, a.fl & (A_BOLD | A_UNDER))
    }
}

// font, size (24.8), cell width, line height, baseline; per_cell: a proportional fallback
#[derive(Clone, Copy)]
struct Metrics {
    font: u32,
    size: i32,
    cw: i32,
    lh: i32,
    base: i32,
    per_cell: bool,
}

/// shared state (Ui::m.term): the monospace font of the consoles
#[derive(Default)]
pub(crate) struct S {
    pub(crate) mono: Option<u32>, // (svg) imgview.rs: the face of "monospace" in SVG text
}

impl Ui {
    /// the monospace font of every console (a font id, e.g. from ui.text.load_mem)
    pub fn mono_font_set(&mut self, font: u32) {
        self.m.term.mono = Some(font);
    }

    /// the consoles' monospace font at size_pt points -> (font id, cell width px): the font of
    /// mono_font_set, else the system's "monospace", else FONT_REG (drawn cell by cell)
    pub fn mono_font(&mut self, size_pt: f32) -> (u32, i32) {
        let m = self.term_metrics(None, size_pt);
        (m.font, m.cw)
    }

    fn term_metrics(&mut self, font: Option<u32>, size_pt: f32) -> Metrics {
        let (font, per_cell) = match font.or(self.m.term.mono) {
            Some(f) if self.text.valid(f) => (f, false),
            _ => match self.font_load(&FontSel::new("monospace", size_pt as f64)) {
                Some(f) => (f, false),
                None => (text::FONT_REG, true),
            },
        };
        let dpi = self.wp.rt.dpi as f32;
        let scale = self.text_scale as f32 / 100.0;
        let mut size = ((size_pt.clamp(4.0, 96.0) * 4.0 / 3.0 * dpi / 96.0 * scale * 256.0) as i32).max(256);
        let prev = self.text.get_font();
        self.text.set_font(font, size);
        let cw = if per_cell {
            self.text.width("M").max(self.text.width("W")).max(1)
        } else {
            // ten cells, so that the rounding of the advance does not add up along a row
            let adv = self.text.measure("0000000000");
            let cw = ((adv + 1280) / 2560).max(1);
            if adv > 0 {
                size = (size as i64 * (cw as i64 * 2560) / adv as i64) as i32;
                self.text.set_font(font, size);
            }
            cw
        };
        let (asc, desc) = (self.text.ascent(), self.text.descent());
        let lh = self.text.line_h().max(asc + desc - 1).max(1);
        let base = ((lh - asc - desc) >> 1) + asc;
        self.text.set_font(prev.0, prev.1);
        Metrics { font, size, cw, lh, base, per_cell }
    }

    fn term_palette(&self) -> Pal {
        let dark = self.theme_dark();
        let bg = over(self.color(T_BG), self.color(T_INPUT_ACTIVE));
        let fg = over(bg, self.color(T_TEXT));
        let red = over(bg, self.color(T_CRIT));
        let green = over(bg, self.color(T_OK));
        let yellow = over(bg, self.color(T_WARN));
        let blue = over(bg, self.color(T_INFO));
        let magenta = color_mix(red, blue, 128);
        let cyan = color_mix(green, blue, 140);
        let (black, white, gray) = if dark {
            (color_mix(bg, fg, 70), over(bg, self.color(T_TEXT2)), over(bg, self.color(T_TEXT3)))
        } else {
            (fg, color_mix(fg, bg, 150), over(bg, self.color(T_TEXT3)))
        };
        let lift = |c: u32| color_mix(c, if dark { 0xFFFF_FFFF } else { 0xFF00_0000 }, if dark { 70 } else { 40 });
        let base = [black, red, green, yellow, blue, magenta, cyan, white];
        let mut c = [0; 16];
        for i in 0..8 {
            c[i] = base[i];
            c[i + 8] = lift(base[i]);
        }
        c[8] = gray;
        c[15] = if dark { fg } else { color_mix(white, bg, 120) };
        Pal {
            c,
            fg,
            bg,
            sel: over(bg, self.color(T_ACCENT)),
            sel_fg: self.color(T_ON_ACCENT) | 0xFF00_0000,
            hit: over(bg, self.color(T_WARN_BG)),
            hit_cur: yellow,
        }
    }

    // take an unconsumed key with exactly these modifiers (Shift, Ctrl, Alt), past the
    // reservation of the focused text box
    fn term_key(&mut self, vk: u32, mods: u32) -> bool {
        let m = MOD_SHIFT | MOD_CTRL | MOD_ALT;
        let q = &mut self.wp.ctx.keyq;
        match q.iter().position(|&k| (k as i32) >= 0 && k & 0xFFFF == vk && (k >> 16) & m == mods && k >> 16 & MOD_ALTGR == 0) {
            Some(i) => {
                q[i] |= 0x8000_0000;
                true
            }
            None => false,
        }
    }

    // take the first unconsumed key: VK | MOD_* << 16
    fn term_key_next(&mut self) -> Option<u32> {
        let q = &mut self.wp.ctx.keyq;
        let i = q.iter().position(|&k| (k as i32) >= 0)?;
        let k = q[i];
        q[i] |= 0x8000_0000;
        Some(k)
    }

    /// ui_terminal(id, st, flags) -> events: the console (see the module header)
    pub fn terminal(&mut self, id: u32, st: &mut TermState, flags: u32) -> Vec<TermEv> {
        let flags = flags | st.flags;
        let mut evs = Vec::new();
        let sid = id.wrapping_add(1); // the input line
        let (bar_id, find_id, more_id) = (id.wrapping_add(2), id.wrapping_add(3), id.wrapping_add(7));
        let r = self.place(self.dp(480), self.dp(240));
        let m = self.term_metrics(st.font, st.size_pt);
        let hair = self.hair();
        let style = self.style;
        let sbw = style.scrollbar_thickness(self).max(self.dp(10));
        let frame = flags & TF_NO_FRAME == 0;
        let pad = if frame { self.dp(6) } else { self.dp(2) };
        let (tx, ty) = (r[0] + pad, r[1] + pad);
        let tw = (r[2] - pad * 2 - sbw).max(m.cw);
        let th = (r[3] - pad * 2).max(m.lh);
        let cols = (tw / m.cw).max(1) as usize;
        let vrows = (th / m.lh).max(1) as usize;
        if (cols as u16, vrows as u16) != (st.cols, st.rows) {
            // keep the first visible line when the width changes
            let anchor = (!st.at_bottom).then(|| {
                let l = st.scr.line_at_row(st.top);
                (l, l.map_or(0, |l| st.top - st.scr.row_of(l)))
            });
            st.scr.set_cols(cols);
            if let Some((Some(l), sub)) = anchor {
                let rows = st.scr.line(l).map_or(1, |x| line_rows(x.cells, cols));
                st.top = st.scr.row_of(l) + sub.min(rows - 1);
            }
            st.cols = cols.min(65535) as u16;
            st.rows = vrows.min(65535) as u16;
            evs.push(TermEv::Resize(st.cols, st.rows));
        }
        // focus: the console's id stands for its input line
        if self.wp.ctx.focus == id {
            self.wp.ctx.focus = sid;
        }
        let fids = sid..=id.wrapping_add(6);
        let active_win = self.uiw_cur == self.m.win.active;
        let focused = active_win && fids.contains(&self.wp.ctx.focus);
        let on_input = active_win && self.wp.ctx.focus == sid;
        let on_find = active_win && self.wp.ctx.focus == find_id;
        let line_mode = matches!(st.mode, TM_LINE | TM_SECRET) && flags & TF_NO_INPUT == 0;
        let waiting = st.pager_waiting();
        let show_input = line_mode && !waiting && st.mode == TM_LINE;

        // ---- keys
        let mut help_rest: Option<Vec<u32>> = None;
        if focused {
            self.term_keys_common(st, &mut evs, id, on_find);
        }
        if on_input {
            if waiting {
                self.term_keys_pager(st, &mut evs);
            } else if flags & TF_NO_INPUT != 0 {
                let _ = self.text_take(usize::MAX);
            } else {
                match st.mode {
                    TM_RAW => self.term_keys_raw(st, &mut evs),
                    TM_BUSY | TM_SECRET => self.term_keys_typed(st, &mut evs, flags),
                    _ => help_rest = self.term_keys_line(st, &mut evs, sid, flags),
                }
            }
        }
        // type-ahead: a waiting line every 16 ms while the app reads lines (the app may go
        // busy on it); the timer brings the next
        let due = st.ta_at.is_none_or(|t| self.now_ms.wrapping_sub(t) >= 16);
        if line_mode && !st.pager_waiting() && due && st.ta_release(&mut evs) {
            st.ta_at = Some(self.now_ms);
            self.repass();
        }
        if line_mode && !st.pager_waiting() && !st.ta.is_empty() {
            self.timer(UIT_TERM, 16);
        }

        // ---- the current line, prompt and input (the tail)
        let waiting = st.pager_waiting();
        let show_input = show_input && !st.pager_waiting();
        let mut tail: Vec<Cell> = st.scr.cur.clone();
        let mut p0 = tail.len(); // the cell of the input's first character
        if waiting {
            tail.extend(st.pager_text.chars().map(|ch| Cell { ch, a: Attr { fg: DEF, bg: DEF, fl: A_INV } }));
        } else if line_mode {
            if st.prompt_cells.as_ref().is_none_or(|p| p.0 != st.prompt) {
                let mut s = Screen::new(100);
                s.write(st.prompt.as_bytes());
                st.prompt_cells = Some((st.prompt.clone(), s.cur));
            }
            if let Some((_, pc)) = &st.prompt_cells {
                tail.extend_from_slice(pc);
            }
            p0 = tail.len();
            if st.mode == TM_LINE {
                tail.extend(st.input.chars().map(|ch| Cell { ch, a: ATTR0 }));
            }
        }
        let n_in = if show_input { st.input.chars().count() } else { 0 };
        // the cursor's last possible cell: after the input / in the current line
        let no_cursor = waiting || flags & TF_NO_INPUT != 0;
        let cursor_max = if no_cursor {
            0
        } else if line_mode {
            p0 + n_in + 1
        } else {
            st.scr.col + 1
        };
        let tail_cells = tail.len().max(cursor_max).max(1);
        let tail_rows = tail_cells.div_ceil(cols) as u64;

        // ---- scrolling
        let first = st.scr.first_row();
        let end_all = st.scr.row_end + tail_rows;
        let floor_row = if st.scr.floor >= st.scr.end() { st.scr.row_end } else { st.scr.row_of(st.scr.floor) };
        let bottom = end_all.saturating_sub(vrows as u64).max(floor_row).max(first);
        let clamp_top = |t: i64| -> u64 { (t.max(first as i64) as u64).min(bottom) };
        let mut top = if st.at_bottom { bottom } else { st.top.clamp(first, bottom) };
        if st.scroll_req != 0 {
            top = clamp_top(top as i64 + st.scroll_req);
            st.scroll_req = 0;
        }

        // ---- pointer: selection, wheel
        let ev = self.hit_rect(id, r[0], r[1], r[2], r[3], CUR_IBEAM << HB_CURSOR_SHIFT);
        // a focused console takes the keyboard as a text box does, also while it shows no
        // input line (busy, the pager): no plain-key accelerators, no app undo (HB_TEXT);
        // output only (TF_NO_INPUT) it does not
        if on_input && !show_input && flags & TF_NO_INPUT == 0 {
            let _ = self.hit_rect(sid, r[0], r[1], 0, 0, HB_TEXT);
        }
        // (right-to-left: the rows keep their order inside the text columns)
        let (mx, my) = (self.rtl_map(self.wp.ctx.mx, tx, cols as i32 * m.cw), self.wp.ctx.my);
        let hit_pos = |st: &TermState, top: u64| -> Option<(u64, u32)> {
            let v = (my - ty).div_euclid(m.lh);
            let row = (top as i64 + v as i64).max(first as i64) as u64;
            match st.scr.line_at_row(row) {
                Some(l) => {
                    let line = st.scr.line(l)?;
                    let sub = row - st.scr.row_of(l);
                    let c = ((mx - tx + m.cw / 2).div_euclid(m.cw)).clamp(0, cols as i32) as u64;
                    Some((l, (sub * cols as u64 + c).min(line.cells as u64) as u32))
                }
                None => {
                    let l = st.scr.end().checked_sub(1)?;
                    Some((l, st.scr.line(l)?.cells))
                }
            }
        };
        let c = &self.wp.ctx;
        if c.pressed & (1 << BTN_LEFT) != 0 && ev & EV_HOVER != 0 && c.active == id {
            self.wp.ctx.focus = sid; // a click takes the keyboard focus (no ring)
            self.wp.ctx.focus_visible = false;
            if let Some(p) = hit_pos(st, top) {
                if ev & EV_DOUBLE != 0 {
                    st.sel = st.word_at(p);
                    st.sel_drag = false;
                } else if self.mods & MOD_SHIFT != 0 && st.sel.is_some() {
                    st.sel = st.sel.map(|(a, _)| (a, p));
                    st.sel_drag = true;
                } else {
                    st.sel = Some((p, p));
                    st.sel_drag = true;
                }
            }
            self.repass();
        } else if ev & EV_HELD != 0 && st.sel_drag {
            if my < ty {
                top = clamp_top(top as i64 - 1);
            } else if my >= ty + vrows as i32 * m.lh {
                top = clamp_top(top as i64 + 1);
            }
            if let Some(p) = hit_pos(st, top) {
                st.sel = st.sel.map(|(a, _)| (a, p));
            }
        } else if st.sel_drag {
            st.sel_drag = false;
            if st.sel_range().is_none() {
                st.sel = None;
            }
        }
        let wy = self.wp.ctx.wheel_y;
        if (ev & EV_HOVER != 0 || self.wp.ctx.hot == sid) && wy != 0 {
            let t = clamp_top(top as i64 - (wy as i64 * 3) / 120);
            if t != top {
                top = t;
                self.wp.ctx.wheel_y = 0;
            }
        }
        st.at_bottom = top >= bottom;
        st.top = top;
        if st.at_bottom {
            st.unseen = false;
            st.seen_added = st.scr.added;
        } else if st.scr.added != st.seen_added {
            st.unseen = true;
        }

        // ---- frame
        let pal = self.term_palette();
        if frame {
            let stroke = self.color(if focused { T_ACCENT } else { T_CTRL_STROKE });
            self.cmd_rrect(r[0], r[1], r[2], r[3], self.dp(4), hair, pal.bg, stroke, stroke);
        } else {
            self.draw_color(pal.bg);
            self.draw_rect(r[0], r[1], r[2], r[3]);
        }
        self.clip_push(r[0] + hair, r[1] + hair, r[2] - hair * 2, r[3] - hair * 2);
        self.ltr_begin(tx, cols as i32 * m.cw);
        self.wp.ctx.last_state = 0;
        self.sem(K_CANVAS, id, None, st.scr.lines.len() as i32);

        // ---- the scrollback rows
        if st.find_open {
            st.find_update();
        }
        let sel = st.sel_range();
        let body_vis = (st.scr.row_end.saturating_sub(top) as usize).min(vrows);
        let mut rows: Vec<Vec<Run>> = Vec::with_capacity(body_vis);
        let mut sems: Vec<(i32, i32, String)> = Vec::new();
        let mut row = top;
        while rows.len() < body_vis {
            let Some(l) = st.scr.line_at_row(row) else { break };
            let Some(line) = st.scr.line(l) else { break };
            let r0 = st.scr.row_of(l);
            let sub0 = (row - r0) as usize;
            let nrows = line_rows(line.cells, cols) as usize;
            let sub_end = nrows.min(sub0 + body_vis - rows.len());
            let mut rbs: Vec<Rb> = (sub0..sub_end).map(|_| Rb::default()).collect();
            let hits_from = st.find_hits.partition_point(|h| h.0 < l);
            let hits = st.find_hits[hits_from..].iter().take_while(|h| h.0 == l);
            let cur_hit = st.find_hits.get(st.find_idx).copied();
            let mut sp = line.spans.iter().peekable();
            let mut a = ATTR0;
            for (k, ch) in line.text.chars().enumerate() {
                while let Some(&&(c, at)) = sp.peek() {
                    if c as usize > k {
                        break;
                    }
                    a = at;
                    sp.next();
                }
                let rr = k / cols;
                if rr < sub0 {
                    continue;
                }
                if rr >= sub_end {
                    break;
                }
                let mut col = pal.cell(a);
                let kk = k as u32;
                if sel.is_some_and(|(s, e)| (l, kk) >= s && (l, kk) < e) {
                    col = (pal.sel_fg, pal.sel, col.2);
                } else if let Some(h) = hits.clone().find(|h| h.1 <= kk && kk < h.2) {
                    col = if Some(*h) == cur_hit { (pal.bg, pal.hit_cur, col.2) } else { (pal.fg, pal.hit, col.2) };
                }
                rbs[rr - sub0].push((k % cols) as u32, ch, col);
            }
            let y = ty + rows.len() as i32 * m.lh;
            sems.push((y, (sub_end - sub0) as i32 * m.lh, line.text.clone()));
            rows.extend(rbs.into_iter().map(|rb| rb.runs));
            row = r0 + sub_end as u64;
        }
        let snap = |rows: Vec<Vec<Run>>| Snap {
            font: m.font,
            size: m.size,
            cw: m.cw,
            lh: m.lh,
            base: m.base,
            ul: (m.base + 1 + (m.size >> 8) / 12, ((m.size >> 8) / 14).max(1)),
            bold_dx: (m.size >> 8) / 16 + 1,
            per_cell: m.per_cell,
            rows,
        };
        let w = cols as i32 * m.cw;
        if body_vis > 0 {
            let (d, ver) = term_cached(&mut st.body, snap(rows));
            self.cmd_custom(tx, ty, w, body_vis as i32 * m.lh, Some(d), 0, ver);
        }
        for (y, h, label) in sems.into_iter().filter(|s| !s.2.is_empty()) {
            self.wp.ctx.last_rect = [tx, y, w, h];
            self.sem(K_TEXT, 0, Some(&label), 0);
        }

        // ---- the tail rows
        let tail_y0 = ty + (st.scr.row_end as i64 - top as i64) as i32 * m.lh; // its first row
        let t_first = top.saturating_sub(st.scr.row_end) as usize; // first visible tail row
        let t_end = ((top + vrows as u64).saturating_sub(st.scr.row_end) as usize).min(tail_rows as usize);
        if t_first < t_end {
            let mut rbs: Vec<Rb> = (t_first..t_end).map(|_| Rb::default()).collect();
            for (k, cell) in tail.iter().enumerate() {
                let rr = k / cols;
                if rr >= t_first && rr < t_end {
                    rbs[rr - t_first].push((k % cols) as u32, cell.ch, pal.cell(cell.a));
                }
            }
            let y = tail_y0 + t_first as i32 * m.lh;
            let (d, ver) = term_cached(&mut st.tail, snap(rbs.into_iter().map(|rb| rb.runs).collect()));
            self.cmd_custom(tx, y, w, (t_end - t_first) as i32 * m.lh, Some(d), 0, ver);
            let label: String = tail.iter().map(|c| c.ch).collect();
            if !label.is_empty() {
                self.wp.ctx.last_rect = [tx, y, w, (t_end - t_first) as i32 * m.lh];
                self.sem(K_TEXT, 0, Some(&label), 0);
            }
        }

        // ---- the input line: a text surface over the input's cells
        if show_input {
            if let Some(c) = st.caret_want {
                if self.textbox_select(sid, &st.input, c, c) {
                    st.caret_want = None;
                }
            }
            let starts: Vec<usize> = st.input.char_indices().map(|(i, _)| i).chain(core::iter::once(st.input.len())).collect();
            let mut lines = Vec::new();
            let (r_first, r_last) = (p0 / cols, (p0 + n_in) / cols);
            for rr in r_first..=r_last {
                let i0 = (rr * cols).saturating_sub(p0).min(n_in);
                let i1 = ((rr + 1) * cols).saturating_sub(p0).min(n_in);
                let y = tail_y0 + rr as i32 * m.lh;
                let carets = (i0..=i1).map(|i| (starts[i], tx + ((p0 + i) - rr * cols) as i32 * m.cw)).collect();
                lines.push(TextSurfaceLine { start: starts[i0], end: starts[i1], rect: [tx, y, w.max(1), m.lh], carets });
            }
            let before = st.input.clone();
            self.textbox_surface(sid, &mut st.input, INPUT_CAP, &lines);
            if st.input != before {
                st.at_bottom = true;
                st.hist_pos = st.history.len();
            }
            if let Some(c) = st.caret_want.take() {
                self.textbox_select(sid, &st.input, c.min(st.input.len()), c.min(st.input.len()));
            }
            // a pasted line break: one Submit per line, the rest waits as type-ahead
            if let Some(i) = st.input.find('\n') {
                let rest = String::from(&st.input[i + 1..]);
                let line = String::from(&st.input[..i]);
                st.input.clear();
                st.ta.insert_str(0, &rest);
                st.submit(line, &mut evs);
                st.ta_at = Some(self.now_ms);
                self.timer(UIT_TERM, 16);
                self.repass();
            }
            // TF_QHELP: the `?` asks for help about the line up to the caret
            if let Some(rest) = help_rest {
                let caret = self.textbox_selection(sid).map_or(st.input.len(), |s| s.1).min(st.input.len());
                let line = String::from(&st.input[..caret]);
                st.echo(&line, "?");
                st.ta.insert_str(0, &rest.iter().filter_map(|&c| char::from_u32(c)).collect::<String>());
                st.at_bottom = true;
                evs.push(TermEv::Help(line));
            }
        }

        // ---- the cursor (drawn over the text box's own caret, after the box moved it)
        let sel_in = self.textbox_selection(sid);
        let cursor = if no_cursor || sel_in.is_some_and(|s| s.0 != s.1) {
            None
        } else if show_input {
            let caret = sel_in.map_or(st.input.len(), |s| s.1).min(st.input.len());
            Some(p0 + st.input[..caret].chars().count().min(n_in))
        } else if line_mode {
            Some(p0)
        } else {
            Some(st.scr.col)
        };
        if let Some(k) = cursor {
            let rr = k / cols;
            if rr >= t_first && rr < t_end {
                let (x, y) = (tx + (k % cols) as i32 * m.cw, tail_y0 + rr as i32 * m.lh);
                if on_input {
                    self.draw_color(pal.fg);
                    self.draw_rect(x, y, m.cw, m.lh);
                    if let Some(cell) = tail.get(k).filter(|c| c.ch != ' ') {
                        let mut b = [0u8; 4];
                        let s = cell.ch.encode_utf8(&mut b);
                        self.cmd_text(x, y + m.base, s.as_bytes(), m.font, m.size, pal.bg, 0);
                    }
                } else {
                    self.draw_color(color_mix(pal.bg, pal.fg, 160));
                    self.draw_frame(x, y, m.cw, m.lh, 0, hair);
                }
            }
        }
        self.ltr_end();

        // ---- scroll bar, "Neue Ausgabe", search bar, bell
        let mut pos = (top - first).min(i32::MAX as u64) as i32;
        // (a cleared screen may show less than a screenful at the bottom)
        let content = (end_all.max(bottom + vrows as u64) - first).min(i32::MAX as u64) as i32;
        self.next_rect_px(r[0] + r[2] - sbw - hair, r[1] + hair, sbw, r[3] - hair * 2);
        if self.scrollbar(bar_id, &mut pos, content, vrows as i32, true) & EV_ACT != 0 {
            let t = (first + pos as u64).min(bottom);
            st.top = t;
            st.at_bottom = t >= bottom;
        }
        if st.unseen && !st.at_bottom {
            let label = crate::i18n::tr("Neue Ausgabe");
            self.ts_font(TS_BODY);
            let bw = self.text.width(label) + self.dp(48);
            let bh = self.dp(30);
            self.next_rect_px(r[0] + (r[2] - bw) / 2, r[1] + r[3] - bh - self.dp(10), bw, bh);
            self.next_icon(IC_DOWN);
            if self.accent_button(more_id, label) & EV_ACT != 0 {
                st.at_bottom = true;
                self.wp.ctx.focus = sid;
                self.wp.ctx.focus_visible = false;
            }
        }
        if st.find_open {
            self.term_find_bar(id, st, r, sbw);
        }
        if st.scr.bell {
            st.scr.bell = false;
            st.bell_until = self.now_ms.wrapping_add(BELL_MS);
            self.timer(UIT_TERM_BELL, BELL_MS + 1);
        }
        if (st.bell_until.wrapping_sub(self.now_ms) as i32) > 0 {
            self.draw_color((self.color(T_ACCENT) & 0x00FF_FFFF) | 0x30 << 24);
            self.draw_rect(r[0], r[1], r[2], r[3]);
        }
        self.clip_pop();
        self.wp.ctx.last_rect = r;
        if !evs.is_empty() {
            self.repass();
        }
        evs
    }

    // keys of every mode: scrolling, copying, the search bar
    fn term_keys_common(&mut self, st: &mut TermState, evs: &mut Vec<TermEv>, id: u32, on_find: bool) {
        let page = (st.rows.max(2) - 1) as i64;
        for (vk, mods, d) in [
            (VK_PRIOR, 0, -page),
            (VK_NEXT, 0, page),
            (VK_PRIOR, MOD_SHIFT, -page),
            (VK_NEXT, MOD_SHIFT, page),
            (VK_UP, MOD_CTRL, -1),
            (VK_DOWN, MOD_CTRL, 1),
            (VK_HOME, MOD_CTRL, i64::MIN / 2),
            (VK_END, MOD_CTRL, i64::MAX / 2),
        ] {
            if st.mode == TM_RAW && mods == 0 {
                continue; // raw: PgUp / PgDn go to the app
            }
            if self.term_key(vk, mods) {
                st.scroll_req = st.scroll_req.saturating_add(d);
            }
        }
        if st.sel_range().is_some() && (self.term_key(VK_C, MOD_CTRL | MOD_SHIFT) || self.term_key(VK_C, MOD_CTRL) || self.term_key(VK_INSERT, MOD_CTRL)) {
            let s = st.sel_text();
            self.clip_set(CLIP_TEXT, s.as_bytes());
        }
        if (st.mode != TM_RAW || on_find) && self.term_key(0x46, MOD_CTRL) {
            // Ctrl+F: the search bar (again: select its text; raw: the app's key)
            st.find_open = true;
            self.focus(id.wrapping_add(3));
        }
        if st.find_open {
            let older = self.term_key(VK_F3, 0) || (on_find && self.term_key(VK_RETURN, 0));
            let newer = self.term_key(VK_F3, MOD_SHIFT) || (on_find && self.term_key(VK_RETURN, MOD_SHIFT));
            if older || newer {
                st.find_update();
                st.find_step(older);
                self.repass();
            }
            if on_find && self.term_key(VK_ESCAPE, 0) {
                st.find_open = false;
                self.wp.ctx.focus = id.wrapping_add(1);
                self.wp.ctx.focus_visible = false;
                self.repass();
            }
        }
        // Esc: the search bar closes, else the selection goes, else the pager quits (else it
        // stays for the text box / the app)
        if !on_find && (st.find_open || st.sel.is_some() || st.pager_waiting()) && self.term_key(VK_ESCAPE, 0) {
            if st.find_open {
                st.find_open = false;
            } else if st.sel.is_some() {
                st.sel = None;
            } else {
                st.pg_quit_now(evs);
            }
            self.repass();
        }
    }

    // the --More-- pager: Space a page, Enter a line, q / Ctrl+C quit (Esc: common keys)
    fn term_keys_pager(&mut self, st: &mut TermState, evs: &mut Vec<TermEv>) {
        let _ = self.text_take(usize::MAX); // the characters of these keys
        self.term_undo_keys();
        let page = st.page();
        if self.term_key(VK_SPACE, 0) {
            st.pg_release(page);
        } else if self.term_key(VK_RETURN, 0) {
            st.pg_release(1);
        } else if self.term_key(0x51, 0) || self.term_key(0x51, MOD_SHIFT) || self.term_key(VK_C, MOD_CTRL) {
            st.pg_quit_now(evs);
        } else {
            return;
        }
        self.repass();
    }

    // TM_RAW: characters and keys to the app
    fn term_keys_raw(&mut self, st: &mut TermState, evs: &mut Vec<TermEv>) {
        let mut out: Vec<u32> = self.text_take(usize::MAX).into_iter().filter(|&c| c >= 32 && c != 127).collect();
        if self.term_key(0x56, MOD_CTRL | MOD_SHIFT) {
            if let Some(clip) = self.clip_get(CLIP_TEXT) {
                out.extend(String::from_utf8_lossy(&clip).chars().map(|c| if c == '\n' { 13 } else { c as u32 }).filter(|&c| c >= 32 || c == 13));
            }
        }
        while let Some(k) = self.term_key_next() {
            let vk = k & 0xFFFF;
            let mods = k >> 16 & (MOD_SHIFT | MOD_CTRL | MOD_ALT);
            let ctrl = mods & (MOD_CTRL | MOD_ALT) == MOD_CTRL && k >> 16 & MOD_ALTGR == 0;
            let typing = vk == VK_SPACE || (0x30..=0x5A).contains(&vk) || (0x60..=0x6F).contains(&vk) || (0xBA..=0xE2).contains(&vk);
            out.push(match vk {
                VK_RETURN => 13,
                VK_TAB if mods == 0 => 9,
                VK_BACK => 127,
                VK_ESCAPE => 27,
                0x41..=0x5A if ctrl => vk - 0x40, // Ctrl+letter: its control code
                _ if typing && mods & (MOD_CTRL | MOD_ALT) == 0 => continue, // the text follows
                VK_SHIFT | VK_CONTROL | VK_MENU => continue,
                _ => TK_KEY | k & 0x7FFF_FFFF,
            });
        }
        if !out.is_empty() {
            st.at_bottom = true;
            evs.push(TermEv::Raw(out));
        }
    }

    // TM_BUSY (type-ahead) and TM_SECRET (a hidden line): typed text without a text box
    fn term_keys_typed(&mut self, st: &mut TermState, evs: &mut Vec<TermEv>, flags: u32) {
        let secret = st.mode == TM_SECRET;
        let buf = if secret { &mut st.input } else { &mut st.ta };
        for c in self.text_take(usize::MAX) {
            if c >= 32 && c != 127 {
                buf.push(char::from_u32(c).unwrap_or('\u{FFFD}'));
            }
        }
        if self.term_key(VK_BACK, 0) && !buf.ends_with('\n') {
            buf.pop();
        }
        if self.term_key(VK_RETURN, 0) || self.term_key(VK_RETURN, MOD_SHIFT) {
            if secret {
                let line = core::mem::take(&mut st.input);
                st.submit(line, evs);
            } else {
                st.ta.push('\n');
            }
        }
        let ctrl_c = self.term_key(VK_C, MOD_CTRL);
        if ctrl_c || (flags & TF_CTRL_SHIFT6 != 0 && (self.term_key(0x36, MOD_CTRL | MOD_SHIFT) || self.term_key(0x36, MOD_CTRL))) {
            st.ta.clear();
            if secret {
                st.input.clear();
                st.echo("", "^C");
            } else if ctrl_c {
                st.scr.write(b"^C"); // the program ends the line (Ctrl+Shift+6: no echo)
            }
            evs.push(TermEv::Interrupt);
            st.at_bottom = true;
        }
        let empty = if secret { st.input.is_empty() } else { st.ta.is_empty() || st.ta.ends_with('\n') };
        if empty && self.term_key(0x44, MOD_CTRL) {
            evs.push(TermEv::Eof);
        }
        if (empty || !secret) && self.term_key(0x5A, MOD_CTRL) {
            evs.push(TermEv::Chord(0x5A | MOD_CTRL << 16));
        }
        self.term_undo_keys();
    }

    // the undo keys (Ctrl+Z, Ctrl+Y, Ctrl+Shift+Z) belong to the console as to a text box:
    // without an input line it swallows them, so the app's undo does not take them
    fn term_undo_keys(&mut self) {
        for (vk, mods) in [(0x5A, MOD_CTRL), (0x59, MOD_CTRL), (0x5A, MOD_CTRL | MOD_SHIFT)] {
            let _ = self.term_key(vk, mods);
        }
    }

    // TM_LINE: the keys the input line's text box must not see -> the characters typed
    // after a `?` (TF_QHELP), handled after the text box
    fn term_keys_line(&mut self, st: &mut TermState, evs: &mut Vec<TermEv>, sid: u32, flags: u32) -> Option<Vec<u32>> {
        let caret = self.textbox_selection(sid).map_or(st.input.len(), |s| s.1).min(st.input.len());
        let has_sel = self.textbox_selection(sid).is_some_and(|s| s.0 != s.1);
        if !self.wp.ctx.textq.is_empty() {
            st.at_bottom = true;
        }
        if self.term_key(VK_RETURN, 0) || self.term_key(VK_RETURN, MOD_SHIFT) || self.term_key(VK_RETURN, MOD_CTRL) {
            let line = core::mem::take(&mut st.input);
            st.submit(line, evs);
        }
        if self.term_key(VK_TAB, 0) {
            st.at_bottom = true;
            evs.push(TermEv::Complete { line: st.input.clone(), caret });
        }
        if self.term_key(VK_UP, 0) {
            st.hist_move(true);
        }
        if self.term_key(VK_DOWN, 0) {
            st.hist_move(false);
        }
        let _ = self.term_key(VK_UP, MOD_SHIFT) || self.term_key(VK_DOWN, MOD_SHIFT);
        if self.term_key(VK_HOME, 0) {
            st.caret_want = Some(0);
        }
        if self.term_key(VK_END, 0) {
            st.caret_want = Some(st.input.len());
        }
        if !has_sel && self.term_key(VK_C, MOD_CTRL) {
            let line = core::mem::take(&mut st.input);
            st.echo(&line, "^C");
            st.at_bottom = true;
            evs.push(TermEv::Interrupt);
        }
        if flags & TF_CTRL_SHIFT6 != 0 && (self.term_key(0x36, MOD_CTRL | MOD_SHIFT) || self.term_key(0x36, MOD_CTRL)) {
            evs.push(TermEv::Interrupt);
        }
        if st.input.is_empty() {
            if self.term_key(0x44, MOD_CTRL) {
                evs.push(TermEv::Eof);
            }
            if self.term_key(0x5A, MOD_CTRL) {
                evs.push(TermEv::Chord(0x5A | MOD_CTRL << 16));
            }
        }
        if self.term_key(0x4C, MOD_CTRL) {
            evs.push(TermEv::Chord(0x4C | MOD_CTRL << 16));
        }
        if self.term_key(0x55, MOD_CTRL) {
            // Ctrl+U: the line before the caret goes
            st.input.drain(..caret);
            st.caret_want = Some(0);
        }
        if flags & TF_QHELP != 0 {
            let q = &mut self.wp.ctx.textq;
            if let Some(k) = q.iter().position(|&c| c == '?' as u32) {
                let rest = q.split_off(k);
                return Some(rest[1..].to_vec());
            }
        }
        None
    }

    // the search bar at the top right: text, "n/m", older, newer, close
    fn term_find_bar(&mut self, id: u32, st: &mut TermState, r: [i32; 4], sbw: i32) {
        let (fid, up_id, down_id, close_id) = (id.wrapping_add(3), id.wrapping_add(4), id.wrapping_add(5), id.wrapping_add(6));
        let h = self.dp(30);
        let g = self.dp(4);
        let tb_w = self.dp(160);
        let count = if st.find_hits.is_empty() {
            String::from(if st.find_text.is_empty() { "" } else { "0/0" })
        } else {
            alloc::format!("{}/{}", st.find_idx + 1, st.find_hits.len())
        };
        self.ts_font(TS_CAPTION);
        let cw = self.text.width(&count).max(self.dp(36));
        let pw = g + tb_w + g + cw + g + h * 3 + g;
        let ph = h + g * 2;
        let (px, py) = (r[0] + r[2] - sbw - pw - self.dp(8), r[1] + self.dp(6));
        let (fill, stroke) = (self.color(T_FLYOUT), self.color(T_FLYOUT_STROKE));
        self.cmd_rrect(px, py, pw, ph, self.dp(6), self.hair(), fill | 0xFF00_0000, stroke, stroke);
        self.wp.ctx.last_rect = [px, py, pw, ph];
        self.hit(0, HB_BLOCK); // clicks between the parts do not select
        let mut x = px + g;
        self.next_rect_px(x, py + g, tb_w, h);
        let before = st.find_text.clone();
        self.textbox(fid, Some(crate::i18n::tr("Suchen")), &mut st.find_text, 256);
        if st.find_text != before {
            st.find_update();
            self.repass();
        }
        x += tb_w + g;
        let base = self.ts_base(TS_CAPTION);
        let line = self.ts_line(TS_CAPTION);
        let c = self.color(T_TEXT2);
        let tw = self.text.width(&count);
        self.emit_text(x + (cw - tw) / 2, py + g + (h - line) / 2 + base, &count, c);
        if !count.is_empty() {
            self.wp.ctx.last_rect = [x, py + g, cw, h];
            self.sem(K_TEXT, 0, Some(&count), 0);
        }
        x += cw + g;
        for (bid, icon, tip, older) in [
            (up_id, IC_UP, "Vorheriger Treffer", Some(true)),
            (down_id, IC_DOWN, "Nächster Treffer", Some(false)),
            (close_id, IC_CHROME_CLOSE, "Schließen", None),
        ] {
            self.next_rect_px(x, py + g, h, h);
            self.next_flags(ST_SUBTLE | ST_ICON_ONLY);
            if self.icon_button(bid, icon, crate::i18n::tr(tip)) & EV_ACT != 0 {
                match older {
                    Some(o) => {
                        st.find_update();
                        st.find_step(o);
                    }
                    None => {
                        st.find_open = false;
                        self.wp.ctx.focus = id.wrapping_add(1);
                        self.wp.ctx.focus_visible = false;
                    }
                }
                self.repass();
            }
            x += h;
        }
    }
}

// the drawing object of one part: the cached one while it shows the same, else a new one
fn term_cached(slot: &mut Option<(Rc<Snap>, u32)>, snap: Snap) -> (DrawRef, u32) {
    if let Some((d, ver)) = slot {
        if **d == snap {
            let r: DrawRef = d.clone();
            return (r, *ver);
        }
    }
    let ver = slot.as_ref().map_or(1, |s| s.1.wrapping_add(1));
    let d = Rc::new(snap);
    *slot = Some((d.clone(), ver));
    let r: DrawRef = d;
    (r, ver)
}
