// linux_xkb -- minimal XKB keymap parser and key translation (Wayland keyboard; the X11
// backend fills the same table from the core keyboard mapping)        [src/plat/linux/xkb.s]
//
//   xkb_parse(text) -> 0 ok / -1           parse an xkbcommon text keymap (format 1)
//   xkb_use_fallback()                     built-in evdev -> US English table
//   xkb_set_mods(depressed, latched, locked, group) -> MOD_* bits (also wl_mods())
//   xkb_key(evdev) -> (VK, text codepoint)  for the current modifiers (text 0 = none)
//   xkb_is_modifier(evdev) -> bool         the key is a modifier (never auto-repeats)
//   xkb_mod_bit(evdev) -> MOD_* bit the key holds (Shift / Ctrl / Alt / Super / AltGr) or 0
//   xkb_dump()                             print the parsed table to stdout (tests)
//   wl_mods() / wl_mods_set(m)             the MOD_* bits of the events (g_wl_mods)
//   X11 backend: xkb_begin(), xkb_set_key(evdev, levels, nlev), xkb_set_roles(alt, num,
//   super, level3)
//
// Parsed: xkb_keycodes (`<NAME> = code;`, `alias <A> = <B>;`, names <= 4 chars, codes
// < 264), xkb_symbols (first level list of group 1 per key, `type=`, `modifier_map`).
// xkb_types / xkb_compatibility are skipped; key types are classified by name (ONE_LEVEL,
// CTRL+ALT, PC_* -> one level; *KEYPAD* -> keypad; *ALPHABETIC* -> Caps Lock applies) or,
// without an explicit type, like xkbcomp does (1 symbol -> one level, KP_* -> keypad, a/A
// pair -> alphabetic).
//
// Level: ONE -> 1; KEYPAD -> (Shift ^ NumLock) ? 2 : 1;
//        else s = Shift ^ (ALPHA && Caps && !Level3), level = (Level3 ? 3 : 1) + s.
// Text is produced for codepoints >= 0x20 (not 0x7F) unless Ctrl / Alt / Super is held
// without Level3 (AltGr may arrive as Ctrl + Mod5 from Windows hosts).  Dead keys produce
// their spacing character at once (no compose).  VK codes: layout-following for a-z / 0-9
// (level-1 symbol, like Win32) and for + , - . (VK_OEM_PLUS/COMMA/MINUS/PERIOD); otherwise
// by key position; keypad keys give navigation VKs while Num Lock is off.
//
// The table is process state behind a lock (both display backends and the tests use it).

use alloc::format;
use core::sync::atomic::{AtomicU32, Ordering};

use rt::sync::SpinLock;

use crate::{MOD_ALT, MOD_ALTGR, MOD_CAPS, MOD_CTRL, MOD_NUM, MOD_SHIFT, MOD_SUPER};

// tokens
const T_EOF: u32 = 0;
const T_IDENT: u32 = 1;
const T_NUM: u32 = 2;
const T_KEY: u32 = 3; // <NAME>  (tk_p / tk_len: inside the brackets)
const T_STR: u32 = 4; // "text"  (inside the quotes)
const T_PUNCT: u32 = 5; // tk_num = the character

// key flags
const KF_DEF: u8 = 1;
const KF_ONE: u8 = 2;
const KF_KEYPAD: u8 = 4;
const KF_ALPHA: u8 = 8;

// symbol values: codepoint | flags
pub const KV_CP: u32 = 0x1FFFFF;
pub const KV_KP: u32 = 0x2000_0000;
pub const KV_DEAD: u32 = 0x4000_0000;
pub const KV_MODSHIFT: u32 = 24;

// modifier symbols (value >> 24 & 15)
const MS_SHIFT: u8 = 1;
const MS_CTRL: u8 = 2;
const MS_CAPS: u8 = 3;
const MS_ALT: u8 = 4;
const MS_META: u8 = 5;
const MS_SUPER: u8 = 6;
const MS_L3: u8 = 8;
const MS_NUM: u8 = 10;
const MS_OTHER: u8 = 12;

// explicit key types
const TY_ONE: u8 = 1;
const TY_KEYPAD: u8 = 2;
const TY_ALPHA: u8 = 3;
const TY_OTHER: u8 = 4;

const NAMES_MAX: usize = 1024;
const REAL_SHIFT: u32 = 0x01;
const REAL_LOCK: u32 = 0x02;
const REAL_CTRL: u32 = 0x04;

const VK_OEM_PLUS: u32 = 0xBB;
const VK_OEM_COMMA: u32 = 0xBC;
const VK_OEM_MINUS: u32 = 0xBD;
const VK_OEM_PERIOD: u32 = 0xBE;

/// one key record (per evdev code 0..255)
#[derive(Clone, Copy)]
struct Key {
    cp: [u32; 4], // value per level 1..4
    flags: u8,    // KF_*
    md: u8,       // MS_* of the level-1 symbol
    nlev: u8,     // number of levels listed
    ty: u8,       // TY_* of an explicit type
}

const KEY0: Key = Key { cp: [0; 4], flags: 0, md: 0, nlev: 0, ty: 0 };

struct Xkb {
    keys: [Key; 256],
    scratch: Key, // keys with an unknown name
    names: [(u32, u32); NAMES_MAX],
    nnames: usize,
    nkeys: u32,
    mods: u32, // depressed | latched | locked real modifiers (g_xkb_mods)
    m_alt: u32,
    m_num: u32,
    m_super: u32,
    m_l3: u32,
}

static XKB: SpinLock<Xkb> = SpinLock::new(Xkb {
    keys: [KEY0; 256],
    scratch: KEY0,
    names: [(0, 0); NAMES_MAX],
    nnames: 0,
    nkeys: 0,
    mods: 0,
    m_alt: 0,
    m_num: 0,
    m_super: 0,
    m_l3: 0,
});

static WL_MODS: AtomicU32 = AtomicU32::new(0);

/// the MOD_* bits the events carry (g_wl_mods)
pub fn wl_mods() -> u32 {
    WL_MODS.load(Ordering::Relaxed)
}

/// set the MOD_* bits of the events (a modifier key's own event, wl_input)
pub fn wl_mods_set(m: u32) {
    WL_MODS.store(m, Ordering::Relaxed);
}

// ------------------------------------------------------------------ tokenizer

struct Tok<'a> {
    t: &'a [u8],
    p: usize, // read position (g_xp)
    tk_p: usize,
    tk_len: usize,
    tk_num: u64,
    tk_type: u32,
    back: bool,
}

fn is_id(c: u8) -> bool {
    c == b'_' || c.is_ascii_digit() || (c | 0x20).wrapping_sub(b'a') <= 25
}

impl<'a> Tok<'a> {
    fn new(t: &'a [u8]) -> Self {
        Tok { t, p: 0, tk_p: 0, tk_len: 0, tk_num: 0, tk_type: T_EOF, back: false }
    }

    /// the next token -> T_*
    fn tok(&mut self) -> u32 {
        if self.back {
            self.back = false;
            return self.tk_type;
        }
        let t = self.t;
        let e = t.len();
        let mut p = self.p;
        let typ;
        'scan: loop {
            // white space and comments
            loop {
                if p >= e || t[p] == 0 {
                    self.tk_p = p;
                    self.tk_len = 0;
                    typ = T_EOF;
                    break 'scan;
                }
                let c = t[p];
                if c <= b' ' {
                    p += 1;
                    continue;
                }
                let line = c == b'#' || (c == b'/' && p + 1 < e && t[p + 1] == b'/');
                if line {
                    loop {
                        p += 1;
                        if p >= e {
                            self.tk_p = p;
                            self.tk_len = 0;
                            typ = T_EOF;
                            break 'scan;
                        }
                        if t[p] == b'\n' {
                            break;
                        }
                    }
                    continue;
                }
                if c == b'/' && p + 1 < e && t[p + 1] == b'*' {
                    p += 2;
                    loop {
                        if p + 1 >= e {
                            self.tk_p = p;
                            self.tk_len = 0;
                            typ = T_EOF;
                            break 'scan;
                        }
                        if t[p] == b'*' && t[p + 1] == b'/' {
                            p += 2;
                            break;
                        }
                        p += 1;
                    }
                    continue;
                }
                break;
            }
            let c = t[p];
            self.tk_p = p;
            let start = p;
            if c == b'_' || (c | 0x20).wrapping_sub(b'a') <= 25 {
                p += 1;
                while p < e && is_id(t[p]) {
                    p += 1;
                }
                self.tk_len = p - start;
                typ = T_IDENT;
            } else if c.is_ascii_digit() {
                let mut v = 0u64;
                let hex = p + 1 < e && (t[p + 1] | 0x20) == b'x' && t[p] == b'0';
                if hex {
                    p += 2;
                    while p < e {
                        let c = t[p];
                        let d = if c.is_ascii_digit() {
                            (c - b'0') as u64
                        } else if (c | 0x20).wrapping_sub(b'a') <= 5 {
                            ((c | 0x20) - b'a' + 10) as u64
                        } else {
                            break;
                        };
                        v = (v << 4).wrapping_add(d);
                        p += 1;
                    }
                } else {
                    while p < e && t[p].is_ascii_digit() {
                        v = v.wrapping_mul(10).wrapping_add((t[p] - b'0') as u64);
                        p += 1;
                    }
                }
                while p < e && is_id(t[p]) {
                    p += 1; // trailing identifier characters
                }
                self.tk_num = v;
                self.tk_len = p - start;
                typ = T_NUM;
            } else if c == b'<' {
                p += 1;
                self.tk_p = p;
                let s = p;
                while p < e && t[p] != b'>' && t[p] != b'\n' {
                    p += 1;
                }
                self.tk_len = p - s;
                if p < e && t[p] == b'>' {
                    p += 1;
                }
                typ = T_KEY;
            } else if c == b'"' {
                p += 1;
                self.tk_p = p;
                let s = p;
                while p < e && t[p] != b'"' {
                    if t[p] == b'\\' {
                        p += 1;
                    }
                    p += 1;
                }
                if p > e {
                    p = e;
                }
                self.tk_len = p - s;
                if p < e {
                    p += 1; // closing quote
                }
                typ = T_STR;
            } else {
                self.tk_num = c as u64;
                self.tk_len = 1;
                p += 1;
                typ = T_PUNCT;
            }
            break;
        }
        self.p = p;
        self.tk_type = typ;
        typ
    }

    fn text(&self) -> &'a [u8] {
        let t = self.t;
        let end = (self.tk_p + self.tk_len).min(t.len());
        &t[self.tk_p.min(end)..end]
    }

    /// the current token's text equals s
    fn eq(&self, s: &str) -> bool {
        self.text() == s.as_bytes()
    }

    /// s occurs inside the current token's text
    fn has(&self, s: &str) -> bool {
        let s = s.as_bytes();
        self.text().windows(s.len()).any(|w| w == s)
    }

    /// the current token is the punctuation character ch
    fn punct(&self, ch: u8) -> bool {
        self.tk_type == T_PUNCT && self.tk_num as u32 == ch as u32
    }

    /// key name packed into a u32 (0 if longer than 4 bytes or empty)
    fn pack(&self) -> u32 {
        let n = self.tk_len;
        if n == 0 || n > 4 {
            return 0;
        }
        let mut v = 0u32;
        for i in (0..n).rev() {
            v = v << 8 | self.t.get(self.tk_p + i).copied().unwrap_or(0) as u32;
        }
        v
    }

    /// skip to the punctuation ch -> found
    fn skip_until(&mut self, ch: u8) -> bool {
        loop {
            if self.tok() == T_EOF {
                return false;
            }
            if self.punct(ch) {
                return true;
            }
        }
    }

    /// the opening bracket was consumed: skip to its match
    fn skip_nested(&mut self, open: u8, close: u8) {
        let mut depth = 1;
        loop {
            let t = self.tok();
            if t == T_EOF {
                return;
            }
            if t != T_PUNCT {
                continue;
            }
            let c = self.tk_num as u32;
            if c == open as u32 {
                depth += 1;
            } else if c == close as u32 {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
        }
    }

    /// skip to the next '{' and past its matching '}'
    fn skip_block(&mut self) {
        if self.skip_until(b'{') {
            self.skip_nested(b'{', b'}');
        }
    }

    /// skip to ';' (consumed) at nesting depth 0; a '}' at depth 0 (and ',' if requested)
    /// ends the statement and is pushed back (',' is consumed)
    fn skip_stmt(&mut self, stop_at_comma: bool) {
        let mut depth = 0u32;
        loop {
            let t = self.tok();
            if t == T_EOF {
                return;
            }
            if t != T_PUNCT {
                continue;
            }
            match self.tk_num as u32 {
                0x7B | 0x5B | 0x28 => depth += 1, // { [ (
                0x5D | 0x29 => {
                    // ] )
                    if depth != 0 {
                        depth -= 1;
                    }
                }
                0x7D => {
                    // }
                    if depth == 0 {
                        self.back = true;
                        return;
                    }
                    depth -= 1;
                }
                c => {
                    if depth != 0 {
                        continue;
                    }
                    if c == b';' as u32 || (c == b',' as u32 && stop_at_comma) {
                        return;
                    }
                }
            }
        }
    }

    /// the current token as a keysym value (codepoint | KV_* flags), 0 = none
    fn ks_value(&self) -> u32 {
        let s = self.text();
        let n = self.tk_len;
        match self.tk_type {
            T_NUM => {
                let v = self.tk_num;
                if n == 1 {
                    return (v as u32).wrapping_add(b'0' as u32);
                }
                let x = self.t.get(self.tk_p + 1).copied().unwrap_or(0);
                if x != b'x' && x != b'X' {
                    return 0;
                }
                if v < 0x100_0100 {
                    if v < 0x20 {
                        0
                    } else if v <= 0x7E {
                        v as u32
                    } else if v < 0xA0 {
                        0
                    } else if v <= 0xFF {
                        v as u32
                    } else {
                        0
                    }
                } else if v > 0x110_FFFF {
                    0
                } else {
                    v as u32 - 0x100_0000
                }
            }
            T_IDENT => {
                if n == 1 {
                    return s[0] as u32;
                }
                if s[0] == b'U' && (5..=7).contains(&n) {
                    let mut v = 0u32;
                    let mut ok = true;
                    for &c in &s[1..] {
                        let d = if c.is_ascii_digit() {
                            (c - b'0') as u32
                        } else if (c | 0x20).wrapping_sub(b'a') <= 5 {
                            ((c | 0x20) - b'a' + 10) as u32
                        } else {
                            ok = false;
                            break;
                        };
                        v = v << 4 | d;
                    }
                    if ok {
                        return if v >= 0x110000 { 0 } else { v };
                    }
                }
                for &(name, val) in KS_TABLE {
                    if name.as_bytes() == s {
                        return val;
                    }
                }
                // other KP_* names: KP_0..KP_9, or keypad keys without text
                if n <= 3 || &s[..3] != b"KP_" {
                    return 0;
                }
                if n == 4 && s[3].is_ascii_digit() {
                    return KV_KP | s[3] as u32;
                }
                KV_KP
            }
            _ => 0,
        }
    }
}

/// v1 is a lower-case letter and v2 its upper case
fn case_pair(v1: u32, v2: u32) -> bool {
    let c = v1 & KV_CP;
    let u = v2 & KV_CP;
    if c == 0 {
        return false;
    }
    let d = c.wrapping_sub(u) as i32;
    if c.wrapping_sub(b'a' as u32) <= 25 {
        return d == 32;
    }
    if c.wrapping_sub(0xE0) <= 0x1E {
        return c != 0xF7 && d == 32;
    }
    if c.wrapping_sub(0x100) <= 0x7F {
        return d == 1 || d == -1;
    }
    if c.wrapping_sub(0x3B1) <= 0x18 || c.wrapping_sub(0x430) <= 0x1F {
        return d == 32;
    }
    if c.wrapping_sub(0x450) <= 0x0F {
        return d == 80;
    }
    false
}

// ------------------------------------------------------------------ parser

impl Xkb {
    /// empty tables, modifier roles unknown
    fn reset(&mut self) {
        self.keys = [KEY0; 256];
        self.nnames = 0;
        self.nkeys = 0;
        self.m_alt = 0;
        self.m_num = 0;
        self.m_super = 0;
        self.m_l3 = 0;
    }

    /// fill in modifier roles the keymap did not name (Wayland defaults)
    fn defaults(&mut self) {
        if self.m_alt == 0 {
            self.m_alt = 0x08;
        }
        if self.m_num == 0 {
            self.m_num = 0x10;
        }
        if self.m_super == 0 {
            self.m_super = 0x40;
        }
        if self.m_l3 == 0 {
            self.m_l3 = 0x80;
        }
    }

    fn name_add(&mut self, name: u32, code: u32) {
        if name == 0 || self.nnames >= NAMES_MAX {
            return;
        }
        self.names[self.nnames] = (name, code);
        self.nnames += 1;
    }

    fn name_code(&self, name: u32) -> u32 {
        if name == 0 {
            return 0;
        }
        self.names[..self.nnames].iter().find(|e| e.0 == name).map_or(0, |e| e.1)
    }

    /// the key record (index 0..255, 256 = scratch) of the current <NAME> token
    fn key_rec(&self, tk: &Tok) -> usize {
        let code = self.name_code(tk.pack());
        match code.checked_sub(8) {
            Some(i) if i < 256 => i as usize,
            _ => 256,
        }
    }

    fn key(&mut self, k: usize) -> &mut Key {
        if k < 256 { &mut self.keys[k] } else { &mut self.scratch }
    }

    /// xkb_keycodes { <NAME> = code; alias <A> = <B>; ... };
    fn parse_keycodes(&mut self, tk: &mut Tok) {
        if !tk.skip_until(b'{') {
            return;
        }
        loop {
            let t = tk.tok();
            if t == T_EOF {
                return;
            }
            if t == T_KEY {
                let name = tk.pack();
                tk.tok();
                if tk.punct(b'=') && tk.tok() == T_NUM && tk.tk_num < 264 {
                    self.name_add(name, tk.tk_num as u32);
                }
                tk.skip_stmt(false);
            } else if t == T_IDENT {
                if tk.eq("alias") && tk.tok() == T_KEY {
                    let a = tk.pack();
                    tk.tok(); // '='
                    if tk.tok() == T_KEY {
                        let code = self.name_code(tk.pack());
                        if code != 0 {
                            self.name_add(a, code);
                        }
                    }
                }
                tk.skip_stmt(false);
            } else if tk.punct(b'}') {
                tk.skip_stmt(false); // the ';' after '}'
                return;
            }
        }
    }

    /// xkb_symbols { key <K> { ... }; modifier_map M { ... }; ... };
    fn parse_symbols(&mut self, tk: &mut Tok) {
        if !tk.skip_until(b'{') {
            return;
        }
        loop {
            let t = tk.tok();
            if t == T_EOF {
                return;
            }
            if t == T_IDENT {
                if tk.eq("key") {
                    self.parse_key(tk);
                } else if tk.eq("modifier_map") || tk.eq("modmap") {
                    self.parse_modmap(tk);
                } else {
                    tk.skip_stmt(false);
                }
            } else if tk.punct(b'}') {
                tk.skip_stmt(false);
                return;
            }
        }
    }

    /// '[' consumed: read up to 4 level symbols into the record
    fn parse_levels(&mut self, tk: &mut Tok, k: usize) {
        let mut n = 0usize;
        loop {
            let t = tk.tok();
            if t == T_EOF {
                break;
            }
            let v;
            if t == T_PUNCT {
                let c = tk.tk_num as u32;
                if c == b']' as u32 {
                    break;
                }
                if c != b'{' as u32 {
                    continue; // ','
                }
                // { sym, sym }: several symbols on one level -- keep the first
                tk.tok();
                v = tk.ks_value();
                tk.skip_nested(b'{', b'}');
            } else {
                v = tk.ks_value();
            }
            if n < 4 {
                self.key(k).cp[n] = v;
            }
            n += 1;
        }
        self.key(k).nlev = n.min(255) as u8;
    }

    /// after "key": <NAME> { items };
    fn parse_key(&mut self, tk: &mut Tok) {
        if tk.tok() != T_KEY {
            tk.skip_stmt(false);
            return;
        }
        let k = self.key_rec(tk);
        tk.tok();
        if !tk.punct(b'{') {
            tk.skip_stmt(false);
            return;
        }
        *self.key(k) = KEY0;
        let mut seen = false; // a level list was read
        loop {
            let t = tk.tok();
            if t == T_EOF {
                return;
            }
            if t == T_PUNCT {
                let c = tk.tk_num as u32;
                if c == b'}' as u32 {
                    tk.skip_stmt(false); // ';'
                    self.key_finish(k);
                    return;
                }
                if c != b'[' as u32 {
                    continue; // ','
                }
                if seen {
                    tk.skip_nested(b'[', b']');
                } else {
                    self.parse_levels(tk, k);
                    seen = true;
                }
                continue;
            }
            if t == T_IDENT && tk.eq("symbols") {
                let g = group_spec(tk);
                tk.tok(); // '='
                tk.tok();
                if !tk.punct(b'[') {
                    tk.skip_stmt(true);
                } else if g != 1 || seen {
                    tk.skip_nested(b'[', b']');
                } else {
                    self.parse_levels(tk, k);
                    seen = true;
                }
                continue;
            }
            if t == T_IDENT && tk.eq("type") {
                let g = group_spec(tk);
                tk.tok(); // '='
                if tk.tok() != T_STR {
                    tk.skip_stmt(true);
                } else if g == 1 {
                    self.key(k).ty = type_class(tk);
                }
                continue;
            }
            tk.skip_stmt(true); // actions=, repeat=, vmods=, ...
        }
    }

    /// derive the key flags from its symbols and explicit type
    fn key_finish(&mut self, k: usize) {
        if k >= 256 {
            return;
        }
        self.nkeys += 1;
        let r = &mut self.keys[k];
        let ty = r.ty;
        let mut f = KF_DEF;
        r.md = ((r.cp[0] >> KV_MODSHIFT) & 15) as u8;
        if r.nlev <= 1 || ty == TY_ONE {
            f |= KF_ONE;
        } else if ty == TY_KEYPAD || r.cp[0] & KV_KP != 0 || r.cp[1] & KV_KP != 0 {
            f |= KF_KEYPAD;
        } else if ty == TY_ALPHA || (ty == 0 && case_pair(r.cp[0], r.cp[1])) {
            f |= KF_ALPHA;
        }
        r.flags = f;
    }

    /// after "modifier_map": Name { <KEY> | keysym, ... };
    fn parse_modmap(&mut self, tk: &mut Tok) {
        if tk.tok() != T_IDENT {
            tk.skip_stmt(false);
            return;
        }
        let Some(i) = MODMAP_NAMES.iter().position(|n| tk.eq(n)) else {
            tk.skip_stmt(false);
            return;
        };
        let bit = 1u32 << i;
        tk.tok();
        if !tk.punct(b'{') {
            tk.skip_stmt(false);
            return;
        }
        loop {
            let t = tk.tok();
            if t == T_EOF {
                return;
            }
            let role = if t == T_KEY {
                let k = self.key_rec(tk);
                self.key(k).md
            } else if t == T_IDENT || t == T_NUM {
                ((tk.ks_value() >> KV_MODSHIFT) & 15) as u8
            } else {
                if tk.punct(b'}') {
                    tk.skip_stmt(false);
                    return;
                }
                continue;
            };
            if bit < 8 {
                continue; // roles only for Mod1..Mod5
            }
            match role {
                MS_ALT | MS_META => self.m_alt |= bit,
                MS_SUPER => self.m_super |= bit,
                MS_L3 => self.m_l3 |= bit,
                MS_NUM => self.m_num |= bit,
                _ => {}
            }
        }
    }

    /// evdev -> (VK, text) for the current modifiers
    fn key_of(&self, evdev: u32) -> (u32, u32) {
        if evdev >= 256 {
            return (0, 0);
        }
        let r = &self.keys[evdev as usize];
        let m = self.mods;
        let f = r.flags;
        let level = if f & KF_ONE != 0 {
            0
        } else if f & KF_KEYPAD != 0 {
            (m & REAL_SHIFT) ^ (m & self.m_num != 0) as u32
        } else {
            let mut s = m & REAL_SHIFT;
            if f & KF_ALPHA != 0 && m & REAL_LOCK != 0 && m & self.m_l3 == 0 {
                s ^= 1;
            }
            if m & self.m_l3 != 0 && r.nlev > 2 {
                s += 2;
            }
            s
        };
        let mut text = r.cp[level as usize] & KV_CP;
        if text < 0x20 || text == 0x7F || (m & (self.m_alt | self.m_super | REAL_CTRL) != 0 && m & self.m_l3 == 0) {
            text = 0;
        }
        // virtual key
        let table = EVDEV_VK[evdev as usize] as u32;
        if f & KF_KEYPAD != 0 {
            if m & self.m_num != 0 {
                return (table, text);
            }
            let i = evdev.wrapping_sub(71);
            if i <= 12 && KP_NAV[i as usize] != 0 {
                return (KP_NAV[i as usize] as u32, text);
            }
            return (table, text);
        }
        let c0 = r.cp[0];
        if c0 & (KV_KP | KV_DEAD) != 0 {
            return (table, text);
        }
        let c = c0 & KV_CP;
        let vk = match c {
            0x61..=0x7A => c - 32,
            0x41..=0x5A | 0x30..=0x39 => c,
            0x2B => VK_OEM_PLUS,
            0x2C => VK_OEM_COMMA,
            0x2D => VK_OEM_MINUS,
            0x2E => VK_OEM_PERIOD,
            _ => table,
        };
        (vk, text)
    }
}

/// an optional "[GroupN]" / "[N]" -> the group number (1 if absent)
fn group_spec(tk: &mut Tok) -> u32 {
    tk.tok();
    if !tk.punct(b'[') {
        tk.back = true;
        return 1;
    }
    let t = tk.tok();
    let mut g = 1;
    if t == T_NUM {
        g = tk.tk_num as u32;
    } else if t == T_IDENT {
        g = if tk.eq("Group1") { 1 } else { 2 };
    }
    tk.skip_nested(b'[', b']');
    g
}

/// TY_* of the current string token (a key type name)
fn type_class(tk: &Tok) -> u8 {
    if tk.has("KEYPAD") {
        return TY_KEYPAD;
    }
    if tk.has("ONE_LEVEL") || tk.eq("CTRL+ALT") {
        return TY_ONE;
    }
    let s = tk.text();
    if s.len() >= 3 && &s[..3] == b"PC_" {
        return TY_ONE;
    }
    if tk.has("ALPHABETIC") {
        return TY_ALPHA;
    }
    TY_OTHER
}

// ------------------------------------------------------------------ API

/// parse an xkbcommon text keymap -> 0 ok / -1 (fewer than 20 keys found)  [xkb_parse]
pub fn xkb_parse(text: &[u8]) -> i32 {
    let mut x = XKB.lock();
    x.reset();
    let mut tk = Tok::new(text);
    loop {
        let t = tk.tok();
        if t == T_EOF {
            break;
        }
        if t != T_IDENT {
            continue;
        }
        if tk.eq("xkb_keycodes") {
            x.parse_keycodes(&mut tk);
        } else if tk.eq("xkb_symbols") {
            x.parse_symbols(&mut tk);
        } else if tk.eq("xkb_types") || tk.eq("xkb_compatibility") || tk.eq("xkb_compat") || tk.eq("xkb_geometry") {
            tk.skip_block();
        }
    }
    x.defaults();
    if x.nkeys < 20 { -1 } else { 0 }
}

/// evdev -> US English symbols (used when no keymap parses)  [xkb_use_fallback]
pub fn xkb_use_fallback() {
    let mut x = XKB.lock();
    x.reset();
    for e in US_TABLE.chunks_exact(4) {
        let k = e[0] as usize;
        let mut l1 = e[1] as u32;
        let mut l2 = e[2] as u32;
        let fl = e[3] as u32;
        if fl & 0x80 != 0 {
            l1 |= KV_KP;
            l2 |= KV_KP;
        }
        let md = fl & 15;
        let r = &mut x.keys[k];
        r.cp[0] = l1 | md << KV_MODSHIFT;
        r.cp[1] = l2;
        r.nlev = if md != 0 { 1 } else { 2 };
        x.key_finish(k);
    }
    x.defaults();
}

/// (X11 backend) empty key table and modifier roles, before xkb_set_key  [xkb_begin]
pub fn xkb_begin() {
    XKB.lock().reset();
}

/// (X11 backend) define one key: 4 symbol values (as the keymap parser makes them), nlev =
/// number of levels listed  [xkb_set_key]
pub fn xkb_set_key(evdev: u32, levels: &[u32; 4], nlev: u32) {
    if evdev >= 256 {
        return;
    }
    let mut x = XKB.lock();
    let r = &mut x.keys[evdev as usize];
    *r = KEY0;
    r.cp = *levels;
    r.nlev = nlev as u8;
    x.key_finish(evdev as usize);
}

/// (X11 backend) real-modifier masks of the modifier roles (0 = the Wayland default)
/// [xkb_set_roles]
pub fn xkb_set_roles(alt: u32, num: u32, sup: u32, level3: u32) {
    let mut x = XKB.lock();
    x.m_alt = alt;
    x.m_num = num;
    x.m_super = sup;
    x.m_l3 = level3;
    x.defaults();
}

/// the real modifier state -> MOD_* bits (also the events' modifiers)  [xkb_set_mods]
pub fn xkb_set_mods(depressed: u32, latched: u32, locked: u32, _group: u32) -> u32 {
    let mut x = XKB.lock();
    let m = depressed | latched | locked;
    x.mods = m;
    let mut r = 0u32;
    if m & REAL_SHIFT != 0 {
        r |= MOD_SHIFT as u32;
    }
    if m & REAL_CTRL != 0 {
        r |= MOD_CTRL as u32;
    }
    if m & x.m_alt != 0 {
        r |= MOD_ALT as u32;
    }
    if m & x.m_super != 0 {
        r |= MOD_SUPER as u32;
    }
    if m & x.m_l3 != 0 {
        r |= MOD_ALTGR as u32;
    }
    if m & REAL_LOCK != 0 {
        r |= MOD_CAPS as u32;
    }
    if m & x.m_num != 0 {
        r |= MOD_NUM as u32;
    }
    wl_mods_set(r);
    r
}

/// the key's level-1 symbol is a modifier  [xkb_is_modifier]
pub fn xkb_is_modifier(evdev: u32) -> bool {
    evdev < 256 && XKB.lock().keys[evdev as usize].md != 0
}

/// MOD_* bit of a held modifier key (0 for other keys and locks)  [xkb_mod_bit]
pub fn xkb_mod_bit(evdev: u32) -> u32 {
    if evdev >= 256 {
        return 0;
    }
    let md = XKB.lock().keys[evdev as usize].md;
    if md > MS_OTHER { 0 } else { MODSYM_BITS[md as usize] as u32 }
}

/// evdev code -> (VK code, text codepoint (0 = none))  [xkb_key]
pub fn xkb_key(evdev: u32) -> (u32, u32) {
    XKB.lock().key_of(evdev)
}

/// print every defined key: evdev, VK, flags, modifier id, 4 levels  [xkb_dump]
pub fn xkb_dump() {
    let mut x = XKB.lock();
    crate::write_stdout(
        format!("xkb: {} keys, masks alt={:x} num={:x} super={:x} level3={:x}\n", x.nkeys, x.m_alt, x.m_num, x.m_super,
                x.m_l3)
        .as_bytes(),
    );
    for k in 0..256u32 {
        let r = x.keys[k as usize];
        if r.flags & KF_DEF == 0 {
            continue;
        }
        x.mods = 0;
        let (vk, _) = x.key_of(k);
        let mut s = format!("key {:3} vk {:02X} fl {:x} mod {:2} n {}:", k, vk, r.flags, r.md, r.nlev);
        for v in r.cp {
            s.push_str(&format!(" {v:06X}"));
        }
        s.push('\n');
        crate::write_stdout(s.as_bytes());
    }
}

// ------------------------------------------------------------------ data

const MODMAP_NAMES: [&str; 8] = ["Shift", "Lock", "Control", "Mod1", "Mod2", "Mod3", "Mod4", "Mod5"];

// MS_* -> MOD_* while the key is held (locks and group switches: 0)
static MODSYM_BITS: [u8; 13] = [0, MOD_SHIFT, MOD_CTRL, 0, MOD_ALT, MOD_ALT, MOD_SUPER, MOD_SUPER, MOD_ALTGR, MOD_ALTGR, 0, 0, 0];

// keypad keys 71..83 with Num Lock off -> navigation VKs
static KP_NAV: [u8; 13] = [0x24, 0x26, 0x21, 0, 0x25, 0x0C, 0x27, 0, 0x23, 0x28, 0x22, 0x2D, 0x2E];

// US fallback: evdev, level 1, level 2, flags (modifier id | 0x80 keypad)
static US_TABLE: &[u8] = &[
    1, 0, 0, 0, 2, b'1', b'!', 0, 3, b'2', b'@', 0, 4, b'3', b'#', 0, 5, b'4', b'$', 0,
    6, b'5', b'%', 0, 7, b'6', b'^', 0, 8, b'7', b'&', 0, 9, b'8', b'*', 0, 10, b'9', b'(', 0,
    11, b'0', b')', 0, 12, b'-', b'_', 0, 13, b'=', b'+', 0, 14, 0, 0, 0, 15, 0, 0, 0,
    16, b'q', b'Q', 0, 17, b'w', b'W', 0, 18, b'e', b'E', 0, 19, b'r', b'R', 0, 20, b't', b'T', 0,
    21, b'y', b'Y', 0, 22, b'u', b'U', 0, 23, b'i', b'I', 0, 24, b'o', b'O', 0, 25, b'p', b'P', 0,
    26, b'[', b'{', 0, 27, b']', b'}', 0, 28, 0, 0, 0, 29, 0, 0, MS_CTRL,
    30, b'a', b'A', 0, 31, b's', b'S', 0, 32, b'd', b'D', 0, 33, b'f', b'F', 0, 34, b'g', b'G', 0,
    35, b'h', b'H', 0, 36, b'j', b'J', 0, 37, b'k', b'K', 0, 38, b'l', b'L', 0, 39, b';', b':', 0,
    40, 0x27, b'"', 0, 41, b'`', b'~', 0, 42, 0, 0, MS_SHIFT, 43, 0x5C, b'|', 0,
    44, b'z', b'Z', 0, 45, b'x', b'X', 0, 46, b'c', b'C', 0, 47, b'v', b'V', 0, 48, b'b', b'B', 0,
    49, b'n', b'N', 0, 50, b'm', b'M', 0, 51, b',', b'<', 0, 52, b'.', b'>', 0, 53, b'/', b'?', 0,
    54, 0, 0, MS_SHIFT, 55, b'*', b'*', 0x80, 56, 0, 0, MS_ALT, 57, b' ', b' ', 0,
    58, 0, 0, MS_CAPS, 69, 0, 0, MS_NUM,
    71, 0, b'7', 0x80, 72, 0, b'8', 0x80, 73, 0, b'9', 0x80, 74, b'-', b'-', 0x80,
    75, 0, b'4', 0x80, 76, 0, b'5', 0x80, 77, 0, b'6', 0x80, 78, b'+', b'+', 0x80,
    79, 0, b'1', 0x80, 80, 0, b'2', 0x80, 81, 0, b'3', 0x80, 82, 0, b'0', 0x80,
    83, 0, b'.', 0x80, 86, b'<', b'>', 0, 96, 0, 0, 0x80, 97, 0, 0, MS_CTRL,
    98, b'/', b'/', 0x80, 100, 0, 0, MS_ALT, 125, 0, 0, MS_SUPER, 126, 0, 0, MS_SUPER,
];

// evdev code -> VK (by key position, US-style names; layout-following keys are handled above)
static EVDEV_VK: [u8; 256] = [
    0x00, 0x1B, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x30, 0xBD, 0xBB, 0x08, 0x09, // 0..15
    0x51, 0x57, 0x45, 0x52, 0x54, 0x59, 0x55, 0x49, 0x4F, 0x50, 0xDB, 0xDD, 0x0D, 0x11, 0x41, 0x53, // 16..31
    0x44, 0x46, 0x47, 0x48, 0x4A, 0x4B, 0x4C, 0xBA, 0xDE, 0xC0, 0x10, 0xDC, 0x5A, 0x58, 0x43, 0x56, // 32..47
    0x42, 0x4E, 0x4D, 0xBC, 0xBE, 0xBF, 0x10, 0x6A, 0x12, 0x20, 0x14, 0x70, 0x71, 0x72, 0x73, 0x74, // 48..63
    0x75, 0x76, 0x77, 0x78, 0x79, 0x90, 0x91, 0x67, 0x68, 0x69, 0x6D, 0x64, 0x65, 0x66, 0x6B, 0x61, // 64..79
    0x62, 0x63, 0x60, 0x6E, 0x00, 0x00, 0xE2, 0x7A, 0x7B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 80..95
    0x0D, 0x11, 0x6F, 0x2C, 0x12, 0x00, 0x24, 0x26, 0x21, 0x25, 0x27, 0x23, 0x28, 0x22, 0x2D, 0x2E, // 96..111
    0x00, 0xAD, 0xAE, 0xAF, 0x00, 0x00, 0x00, 0x13, 0x00, 0x00, 0x00, 0x00, 0x00, 0x5B, 0x5C, 0x5D, // 112..127
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 128..143
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 144..159
    0x00, 0x00, 0x00, 0xB0, 0xB3, 0xB1, 0xB2, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 160..175
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7C, 0x7D, 0x7E, 0x7F, 0x80, 0x81, 0x82, 0x83, 0x84, // 176..191
    0x85, 0x86, 0x87, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 192..207
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 208..223
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 224..239
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 240..255
];

// keysym names: the Latin-1 set, ASCII punctuation, common European extras, keypad names
// with KV_KP, dead keys with KV_DEAD (value = spacing character) and modifier keysyms
// (MS_* << 24)
static KS_TABLE: &[(&str, u32)] = &[
    // ASCII punctuation
    ("space", 0x20), ("exclam", 0x21), ("quotedbl", 0x22), ("numbersign", 0x23), ("dollar", 0x24),
    ("percent", 0x25), ("ampersand", 0x26), ("apostrophe", 0x27), ("quoteright", 0x27), ("parenleft", 0x28),
    ("parenright", 0x29), ("asterisk", 0x2A), ("plus", 0x2B), ("comma", 0x2C), ("minus", 0x2D),
    ("period", 0x2E), ("slash", 0x2F), ("colon", 0x3A), ("semicolon", 0x3B), ("less", 0x3C), ("equal", 0x3D),
    ("greater", 0x3E), ("question", 0x3F), ("at", 0x40), ("bracketleft", 0x5B), ("backslash", 0x5C),
    ("bracketright", 0x5D), ("asciicircum", 0x5E), ("underscore", 0x5F), ("grave", 0x60), ("quoteleft", 0x60),
    ("braceleft", 0x7B), ("bar", 0x7C), ("braceright", 0x7D), ("asciitilde", 0x7E),
    // Latin-1
    ("nobreakspace", 0xA0), ("exclamdown", 0xA1), ("cent", 0xA2), ("sterling", 0xA3), ("currency", 0xA4),
    ("yen", 0xA5), ("brokenbar", 0xA6), ("section", 0xA7), ("diaeresis", 0xA8), ("copyright", 0xA9),
    ("ordfeminine", 0xAA), ("guillemotleft", 0xAB), ("notsign", 0xAC), ("hyphen", 0xAD), ("registered", 0xAE),
    ("macron", 0xAF), ("degree", 0xB0), ("plusminus", 0xB1), ("twosuperior", 0xB2), ("threesuperior", 0xB3),
    ("acute", 0xB4), ("mu", 0xB5), ("paragraph", 0xB6), ("periodcentered", 0xB7), ("cedilla", 0xB8),
    ("onesuperior", 0xB9), ("masculine", 0xBA), ("guillemotright", 0xBB), ("onequarter", 0xBC),
    ("onehalf", 0xBD), ("threequarters", 0xBE), ("questiondown", 0xBF), ("Agrave", 0xC0), ("Aacute", 0xC1),
    ("Acircumflex", 0xC2), ("Atilde", 0xC3), ("Adiaeresis", 0xC4), ("Aring", 0xC5), ("AE", 0xC6),
    ("Ccedilla", 0xC7), ("Egrave", 0xC8), ("Eacute", 0xC9), ("Ecircumflex", 0xCA), ("Ediaeresis", 0xCB),
    ("Igrave", 0xCC), ("Iacute", 0xCD), ("Icircumflex", 0xCE), ("Idiaeresis", 0xCF), ("ETH", 0xD0),
    ("Ntilde", 0xD1), ("Ograve", 0xD2), ("Oacute", 0xD3), ("Ocircumflex", 0xD4), ("Otilde", 0xD5),
    ("Odiaeresis", 0xD6), ("multiply", 0xD7), ("Oslash", 0xD8), ("Ugrave", 0xD9), ("Uacute", 0xDA),
    ("Ucircumflex", 0xDB), ("Udiaeresis", 0xDC), ("Yacute", 0xDD), ("THORN", 0xDE), ("ssharp", 0xDF),
    ("agrave", 0xE0), ("aacute", 0xE1), ("acircumflex", 0xE2), ("atilde", 0xE3), ("adiaeresis", 0xE4),
    ("aring", 0xE5), ("ae", 0xE6), ("ccedilla", 0xE7), ("egrave", 0xE8), ("eacute", 0xE9),
    ("ecircumflex", 0xEA), ("ediaeresis", 0xEB), ("igrave", 0xEC), ("iacute", 0xED), ("icircumflex", 0xEE),
    ("idiaeresis", 0xEF), ("eth", 0xF0), ("ntilde", 0xF1), ("ograve", 0xF2), ("oacute", 0xF3),
    ("ocircumflex", 0xF4), ("otilde", 0xF5), ("odiaeresis", 0xF6), ("division", 0xF7), ("oslash", 0xF8),
    ("ugrave", 0xF9), ("uacute", 0xFA), ("ucircumflex", 0xFB), ("udiaeresis", 0xFC), ("yacute", 0xFD),
    ("thorn", 0xFE), ("ydiaeresis", 0xFF), ("guillemetleft", 0xAB), ("guillemetright", 0xBB),
    ("ordmasculine", 0xBA), ("Eth", 0xD0), ("Ooblique", 0xD8), ("Thorn", 0xDE), ("ooblique", 0xF8),
    // other text keysyms
    ("EuroSign", 0x20AC), ("endash", 0x2013), ("emdash", 0x2014), ("ellipsis", 0x2026),
    ("leftsinglequotemark", 0x2018), ("rightsinglequotemark", 0x2019), ("singlelowquotemark", 0x201A),
    ("leftdoublequotemark", 0x201C), ("rightdoublequotemark", 0x201D), ("doublelowquotemark", 0x201E),
    ("leftarrow", 0x2190), ("uparrow", 0x2191), ("rightarrow", 0x2192), ("downarrow", 0x2193),
    ("lstroke", 0x142), ("Lstroke", 0x141), ("dstroke", 0x111), ("Dstroke", 0x110), ("tslash", 0x167),
    ("Tslash", 0x166), ("hstroke", 0x127), ("Hstroke", 0x126), ("eng", 0x14B), ("ENG", 0x14A), ("kra", 0x138),
    ("idotless", 0x131), ("oe", 0x153), ("OE", 0x152), ("Greek_OMEGA", 0x3A9), ("oneeighth", 0x215B),
    ("threeeighths", 0x215C), ("fiveeighths", 0x215D), ("seveneighths", 0x215E), ("trademark", 0x2122),
    ("enfilledcircbullet", 0x2022), ("notequal", 0x2260), ("similarequal", 0x2243), ("dagger", 0x2020),
    ("doubledagger", 0x2021), ("permille", 0x2030), ("schwa", 0x259), ("SCHWA", 0x18F), ("abovedot", 0x2D9),
    ("breve", 0x2D8), ("caron", 0x2C7), ("doubleacute", 0x2DD), ("ogonek", 0x2DB), ("prime", 0x2032),
    ("doubleprime", 0x2033), ("lessthanequal", 0x2264), ("greaterthanequal", 0x2265), ("infinity", 0x221E),
    ("Ydiaeresis", 0x178), ("zcaron", 0x17E), ("Zcaron", 0x17D), ("scaron", 0x161), ("Scaron", 0x160),
    ("ccaron", 0x10D), ("Ccaron", 0x10C), ("cacute", 0x107), ("Cacute", 0x106), ("ecaron", 0x11B),
    ("Ecaron", 0x11A), ("rcaron", 0x159), ("Rcaron", 0x158), ("nacute", 0x144), ("Nacute", 0x143),
    ("sacute", 0x15B), ("Sacute", 0x15A), ("zacute", 0x17A), ("Zacute", 0x179), ("zabovedot", 0x17C),
    ("Zabovedot", 0x17B), ("aogonek", 0x105), ("Aogonek", 0x104), ("eogonek", 0x119), ("Eogonek", 0x118),
    ("abreve", 0x103), ("Abreve", 0x102), ("gbreve", 0x11F), ("Gbreve", 0x11E), ("Iabovedot", 0x130),
    ("scedilla", 0x15F), ("Scedilla", 0x15E), ("tcedilla", 0x163), ("Tcedilla", 0x162), ("odoubleacute", 0x151),
    ("Odoubleacute", 0x150), ("udoubleacute", 0x171), ("Udoubleacute", 0x170), ("uring", 0x16F),
    ("Uring", 0x16E), ("dcaron", 0x10F), ("Dcaron", 0x10E), ("lcaron", 0x13E), ("Lcaron", 0x13D),
    ("ncaron", 0x148), ("Ncaron", 0x147), ("tcaron", 0x165), ("Tcaron", 0x164), ("amacron", 0x101),
    ("Amacron", 0x100), ("emacron", 0x113), ("Emacron", 0x112), ("imacron", 0x12B), ("Imacron", 0x12A),
    ("omacron", 0x14D), ("Omacron", 0x14C), ("umacron", 0x16B), ("Umacron", 0x16A), ("lacute", 0x13A),
    ("Lacute", 0x139), ("racute", 0x155), ("Racute", 0x154), ("gcedilla", 0x123), ("Gcedilla", 0x122),
    ("kcedilla", 0x137), ("Kcedilla", 0x136), ("lcedilla", 0x13C), ("Lcedilla", 0x13B), ("ncedilla", 0x146),
    ("Ncedilla", 0x145), ("iogonek", 0x12F), ("Iogonek", 0x12E), ("uogonek", 0x173), ("Uogonek", 0x172),
    // keypad (KP flag)
    ("KP_Decimal", 0x2000_002E), ("KP_Separator", 0x2000_002C), ("KP_Add", 0x2000_002B),
    ("KP_Subtract", 0x2000_002D), ("KP_Multiply", 0x2000_002A), ("KP_Divide", 0x2000_002F),
    ("KP_Equal", 0x2000_003D), ("KP_Space", 0x2000_0020),
    // dead keys (DEAD flag, spacing char)
    ("dead_grave", 0x4000_0060), ("dead_acute", 0x4000_00B4), ("dead_circumflex", 0x4000_005E),
    ("dead_tilde", 0x4000_007E), ("dead_perispomeni", 0x4000_007E), ("dead_macron", 0x4000_00AF),
    ("dead_breve", 0x4000_02D8), ("dead_abovedot", 0x4000_02D9), ("dead_diaeresis", 0x4000_00A8),
    ("dead_abovering", 0x4000_00B0), ("dead_doubleacute", 0x4000_02DD), ("dead_caron", 0x4000_02C7),
    ("dead_cedilla", 0x4000_00B8), ("dead_ogonek", 0x4000_02DB), ("dead_belowdot", 0x4000_002E),
    ("dead_belowmacron", 0x4000_005F), ("dead_iota", 0x4000_037A), ("dead_hook", 0x4000_02C0),
    ("dead_horn", 0x4000_031B), ("dead_stroke", 0x4000_002F), ("dead_belowcomma", 0x4000_002C),
    ("dead_currency", 0x4000_00A4), ("dead_greek", 0x4000_03BC),
    // modifiers (id << 24)
    ("Shift_L", 0x100_0000), ("Shift_R", 0x100_0000), ("Control_L", 0x200_0000), ("Control_R", 0x200_0000),
    ("Caps_Lock", 0x300_0000), ("Shift_Lock", 0x300_0000), ("Alt_L", 0x400_0000), ("Alt_R", 0x400_0000),
    ("Meta_L", 0x500_0000), ("Meta_R", 0x500_0000), ("Super_L", 0x600_0000), ("Super_R", 0x600_0000),
    ("Hyper_L", 0x700_0000), ("Hyper_R", 0x700_0000), ("ISO_Level3_Shift", 0x800_0000),
    ("ISO_Level3_Latch", 0x800_0000), ("ISO_Level3_Lock", 0x800_0000), ("Mode_switch", 0x900_0000),
    ("script_switch", 0x900_0000), ("Num_Lock", 0xA00_0000), ("ISO_Level5_Shift", 0xB00_0000),
    ("ISO_Level5_Latch", 0xB00_0000), ("ISO_Level5_Lock", 0xB00_0000), ("ISO_Next_Group", 0xC00_0000),
    ("ISO_Prev_Group", 0xC00_0000), ("ISO_Group_Shift", 0xC00_0000), ("ISO_Group_Latch", 0xC00_0000),
    ("ISO_Group_Lock", 0xC00_0000), ("ISO_First_Group", 0xC00_0000), ("ISO_Last_Group", 0xC00_0000),
];
