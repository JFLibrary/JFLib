// palette -- PaintCraft: the colour model (primary / secondary colour) and the palette
// (COLOR-001..003, 016..020).  Owner: agent A3 (colours).  The Colors window itself is
// wnd_colors.rs; the palette grid and the palette buttons it shows are drawn here (widget
// block 4700-4999).
//                                              [apps/PaintCraft/src/palette.s]
//
// Colours are straight 0xAARRGGBB.  Tools read them when they paint and set them (picker):
//   pal_primary() / pal_secondary() / pal_color(which)   which 0 = primary, 1 = secondary
//   pal_set_primary(argb) / pal_set_secondary / pal_set_color(which, argb)   every window is
//                                   rebuilt when the colour changed
//   pal_swap()                      X: primary <-> secondary
//   pal_reset()                     black / white
//   pal_version()                   bumped by every change of the colours or of the palette
// Palette (at most PAL_MAX = 96 colours, like paint.net):
//   pal_count() / pal_entry(i) / pal_set_entry(i, argb) / pal_add(argb) / pal_add_primary()
//   pal_default()                   the default palette (64 colours + 32 free slots)
//   pal_load_file(path) -> (0 ok / -1, lines ignored)   (palette unchanged on -1)
//   pal_save_file(path) -> 0 ok / -1 (paint.net format, atomic write)
//   pal_folder() -> (exists, path)  Windows: <home>/Documents/paint.net App Files/Palettes
//                                   (shared with paint.net) when it exists, else
//                                   <config>/PaintCraft/Palettes.  Never created by the app.
// Colors window parts (called inside its layout): pal_grid(), pal_buttons(), pal_checker().
// Module contract (app/lifecycle.rs): pal_init, pal_states, pal_frame (file dialog, error dialog),
//   pal_cmd(id, arg).  Test hooks (diag.rs block A3): pal_diag_cmd (985..994), pal_diag_frame
//   (records 9985..9989).
// The setters take only the Pal: a change sets `repass`, which Pn::flush turns into
// ui.repass() at the end of the callback (the asm calls ui_repass at once).
//
// Palette file (paint.net *.txt, COLOR-020): one colour per line as AARRGGBB (8 hex digits;
// 6 digits RRGGBB = opaque; an optional leading '#'), ';' starts a comment (whole lines or
// the rest of a line), blanks / CR ignored, a UTF-8 BOM is skipped, at most 96 colours are
// taken; malformed lines and colours beyond 96 are ignored.  Saved with a German comment
// header, one line per colour, CRLF on Windows / LF on Linux.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

use crate::app::constants::*;
use crate::app::commands::pn_cmd_set;
use crate::app::Pn;

const PAL_MAX: usize = 96; // colours in a palette (paint.net: 96)
const PAL_COLS: i32 = 16; // grid in the Colors window: 16 x 6
const PAL_ROWS: i32 = 6;
const PAL_DEF_N: usize = 64; // colours of the default palette
const PAL_CELL: i32 = 16; // DIP per grid cell (square + gap)
const PAL_READ_MAX: u64 = 256 * 1024; // bytes of a palette file that are parsed
const PAL_PATH_CAP: usize = 1040;
const LIST_MAX: usize = 32; // palette files collected from the folder
const LIST_SHOW: usize = LIST_MAX; // rows in the palette menu
const NAME_CAP: usize = 128; // bytes per collected file name
const LBL_CAP: usize = 256;
const BLACK: u32 = 0xFF00_0000;
const WHITE: u32 = 0xFFFF_FFFF;
// widget ids (palette block 4700-4999)
const ID_PAL_SW0: u32 = ID_PAL_FIRST; // 4700 + i: swatch i (i < PAL_MAX)
const ID_PAL_ADD: u32 = ID_PAL_FIRST + 96; // 4796 Farbe hinzufügen
const ID_PAL_MENU: u32 = ID_PAL_FIRST + 97; // 4797 palette menu (drop-down)
const ID_PAL_GRID: u32 = ID_PAL_FIRST + 98; // 4798 the grid box
const ID_PAL_LIST0: u32 = ID_PAL_FIRST + 100; // 4800 + k: palette files in the menu
const ID_PAL_MORE: u32 = ID_PAL_FIRST + 140; // 4840 Weitere Paletten…
const ID_PAL_ERR: u32 = ID_PAL_FIRST + 200; // 4900 error dialog (content dialog, 4900..4949)
// test hooks (diag.rs block A3)
const CMD_PAL_TEST0: u32 = 985; // 985..994
const ID_PAL_PROBE: u32 = ID_TEST_FIRST + 85; // 9985..9989
const PAL_TDIR_N: u32 = 3; // test palette folders (0 = none)

// the header of saved palette files (paint.net ignores ';' lines)
static PAL_HDR: [&str; 5] = [
    "; PaintCraft-Palettendatei (paint.net-Format)",
    "; Mit Semikolon beginnende Zeilen sind Kommentare.",
    "; Jede Zeile hält eine Farbe als AARRGGBB in Hex (8 Ziffern), etwa FF00FF00 für Grün.",
    "; AA steht für die Deckkraft von 00 (unsichtbar) bis FF (deckend).",
    "; Höchstens 96 Farben; paint.net füllt fehlende Farbfelder mit Weiß (FFFFFFFF).",
];
const EOL: &str = if cfg!(windows) { "\r\n" } else { "\n" };

// paint.net's default palette (see the file header): 4 rows of 16
static PAL_DEFAULT_TAB: [u32; PAL_DEF_N] = [
    0xFF000000, 0xFF404040, 0xFFFF0000, 0xFFFF6A00, 0xFFFFD800, 0xFFB6FF00, 0xFF4CFF00, 0xFF00FF21,
    0xFF00FF90, 0xFF00FFFF, 0xFF0094FF, 0xFF0026FF, 0xFF4800FF, 0xFFB200FF, 0xFFFF00DC, 0xFFFF006E,
    0xFFFFFFFF, 0xFF808080, 0xFF7F0000, 0xFF7F3300, 0xFF7F6A00, 0xFF5B7F00, 0xFF267F00, 0xFF007F0E,
    0xFF007F46, 0xFF007F7F, 0xFF004A7F, 0xFF00137F, 0xFF21007F, 0xFF57007F, 0xFF7F006E, 0xFF7F0037,
    0xFFA0A0A0, 0xFF303030, 0xFFFF7F7F, 0xFFFFB27F, 0xFFFFE97F, 0xFFDAFF7F, 0xFFA5FF7F, 0xFF7FFF8E,
    0xFF7FFFC5, 0xFF7FFFFF, 0xFF7FC9FF, 0xFF7F92FF, 0xFFA17FFF, 0xFFD67FFF, 0xFFFF7FED, 0xFFFF7FB6,
    0xFFC0C0C0, 0xFF606060, 0xFF7F3F3F, 0xFF7F593F, 0xFF7F743F, 0xFF6D7F3F, 0xFF527F3F, 0xFF3F7F47,
    0xFF3F7F62, 0xFF3F7F7F, 0xFF3F647F, 0xFF3F497F, 0xFF503F7F, 0xFF6B3F7F, 0xFF7F3F76, 0xFF7F3F5B,
];
// commands enabled by pal_states.  X (CMD_SWAP_COLORS) and the reset command
// (CMD_RESET_COLORS) belong to tool.rs, which enables them and calls pal_swap / pal_reset.
static PAL_CMD_IDS: [u32; 4] = [CMD_PAL_FOLDER, CMD_PAL_OPEN, CMD_PAL_SAVE, CMD_PAL_RESET];
static PAL_TDIRS: [&str; 3] = ["", "build", "apps/PaintCraft/tests/fixtures/palettes"];

const S_ERR_LOAD: &str = "Die Palette ließ sich nicht laden.";
const S_ERR_SAVE: &str = "Die Palette ließ sich nicht speichern.";

/// the state of palette.s (its g_* globals)
pub(crate) struct Pal {
    /// g_err_msg: message of the error dialog (None = none)
    err_msg: Option<&'static str>,
    /// g_pri / g_sec
    pri: u32,
    sec: u32,
    /// g_ver: pal_version
    ver: u32,
    /// g_pal / g_pal_n
    pal: [u32; PAL_MAX],
    n: usize,
    /// g_pal_mode: 1 = replace mode
    mode: bool,
    /// g_pal_ignored: lines ignored by the last load
    ignored: i32,
    /// g_pal_tdir: test folder (cmd 985)
    tdir: u32,
    /// g_fd_open / g_fd_mode (0 Öffnen, 1 Speichern unter) / g_fd_path
    fd_open: Shared<i32>,
    fd_mode: i32,
    fd_path: String,
    /// g_err_open: error dialog shown
    err_open: Shared<i32>,
    /// g_list / g_list_dir: palette files of the folder (names with ".txt")
    list: Vec<String>,
    list_dir: String,
    /// g_menu_open: the palette menu was open last frame
    menu_open: bool,
    /// g_pal_last: the last palette file loaded / saved
    last: String,
    /// ui.repass() wanted (Pn::flush)
    pub(crate) repass: bool,
}

impl Default for Pal {
    fn default() -> Pal {
        Pal {
            err_msg: None,
            pri: 0,
            sec: 0,
            ver: 0,
            pal: [0; PAL_MAX],
            n: 0,
            mode: false,
            ignored: 0,
            tdir: 0,
            fd_open: Shared::new(0),
            fd_mode: 0,
            fd_path: String::new(),
            err_open: Shared::new(0),
            list: Vec::new(),
            list_dir: String::new(),
            menu_open: false,
            last: String::new(),
            repass: false,
        }
    }
}

// ================================================================== colours

/// pal_init() -- black / white, the default palette
pub(crate) fn pal_init(p: &mut Pn, _ui: &mut Ui) {
    let pal = &mut p.pal;
    pal.pri = BLACK;
    pal.sec = WHITE;
    pal.ver = 1;
    pal_default(pal);
}

/// pal_primary()
pub(crate) fn pal_primary(pal: &Pal) -> u32 {
    pal.pri
}

/// pal_secondary()
pub(crate) fn pal_secondary(pal: &Pal) -> u32 {
    pal.sec
}

/// pal_color(which) -> (0 primary, else secondary)
pub(crate) fn pal_color(pal: &Pal, which: i32) -> u32 {
    if which == 0 { pal.pri } else { pal.sec }
}

/// pal_version() -> change counter of the colours and the palette
pub(crate) fn pal_version(pal: &Pal) -> u32 {
    pal.ver
}

// pal_changed() -- something changed: bump the version, rebuild every window
fn pal_changed(pal: &mut Pal) {
    pal.ver = pal.ver.wrapping_add(1);
    pal.repass = true;
}

/// pal_set_primary(argb)
pub(crate) fn pal_set_primary(pal: &mut Pal, argb: u32) {
    if pal.pri != argb {
        pal.pri = argb;
        pal_changed(pal);
    }
}

/// pal_set_secondary(argb)
pub(crate) fn pal_set_secondary(pal: &mut Pal, argb: u32) {
    if pal.sec != argb {
        pal.sec = argb;
        pal_changed(pal);
    }
}

/// pal_set_color(which, argb)
pub(crate) fn pal_set_color(pal: &mut Pal, which: i32, argb: u32) {
    if which == 0 {
        pal_set_primary(pal, argb);
    } else {
        pal_set_secondary(pal, argb);
    }
}

/// pal_swap() -- primary <-> secondary (X)
pub(crate) fn pal_swap(pal: &mut Pal) {
    if pal.pri != pal.sec {
        core::mem::swap(&mut pal.pri, &mut pal.sec);
        pal_changed(pal);
    }
}

/// pal_reset() -- primary black, secondary white
pub(crate) fn pal_reset(pal: &mut Pal) {
    if pal.pri != BLACK || pal.sec != WHITE {
        pal.pri = BLACK;
        pal.sec = WHITE;
        pal_changed(pal);
    }
}

// ================================================================== palette

/// pal_count()
pub(crate) fn pal_count(pal: &Pal) -> i32 {
    pal.n as i32
}

/// pal_entry(i) -> colour i, 0 when i >= count
pub(crate) fn pal_entry(pal: &Pal, i: i32) -> u32 {
    let i = i as u32 as usize;
    if i < pal.n && i < PAL_MAX { pal.pal[i] } else { 0 }
}

/// pal_set_entry(i, argb) -> true done: i < count replaces colour i, i == count appends
pub(crate) fn pal_set_entry(pal: &mut Pal, i: i32, argb: u32) -> bool {
    let i = i as u32 as usize;
    if i > pal.n || i >= PAL_MAX {
        return false;
    }
    pal.pal[i] = argb;
    if i >= pal.n {
        pal.n += 1;
    }
    pal_changed(pal);
    true
}

/// pal_add(argb) -> index of the new colour, -1 when the palette is full
pub(crate) fn pal_add(pal: &mut Pal, argb: u32) -> i32 {
    if pal.n >= PAL_MAX {
        return -1;
    }
    pal.pal[pal.n] = argb;
    pal.n += 1;
    pal_changed(pal);
    pal.n as i32 - 1
}

/// pal_add_primary() -- "Farbe hinzufügen": append the primary colour; a full palette (or a
/// second click) toggles the replace mode: the next click on a swatch replaces it
pub(crate) fn pal_add_primary(pal: &mut Pal) {
    if pal.mode {
        pal.mode = false;
        pal.repass = true;
        return;
    }
    if pal_add(pal, pal.pri) == -1 {
        pal.mode = true;
        pal.repass = true;
    }
}

/// pal_default() -- paint.net's default palette
pub(crate) fn pal_default(pal: &mut Pal) {
    pal.pal[..PAL_DEF_N].copy_from_slice(&PAL_DEFAULT_TAB);
    pal.n = PAL_DEF_N;
    pal.mode = false;
    pal_changed(pal);
}

// ------------------------------------------------------------------ palette files

// pal_line(line) -> 1 colour, 0 empty / comment, -1 malformed
fn pal_line(l: &[u8]) -> (i32, u32) {
    // a ';' starts a comment
    let mut e = l.iter().position(|&c| c == b';').unwrap_or(l.len());
    let mut s = 0;
    let blank = |c: u8| c == b' ' || c == b'\t' || c == b'\r';
    while s < e && blank(l[s]) {
        s += 1;
    }
    if s >= e {
        return (0, 0);
    }
    while blank(l[e - 1]) {
        e -= 1;
    }
    if l[s] == b'#' {
        s += 1;
    }
    let n = e.wrapping_sub(s);
    if n != 6 && n != 8 {
        return (-1, 0);
    }
    let mut v = 0u32;
    for &c in &l[s..e] {
        let d = match c {
            b'0'..=b'9' => c - b'0',
            _ => {
                let c = c | 32;
                if (b'a'..=b'f').contains(&c) { c - b'a' + 10 } else { return (-1, 0) }
            }
        };
        v = v << 4 | d as u32;
    }
    if n == 6 {
        v |= 0xFF00_0000;
    }
    (1, v)
}

// pal_parse(text) -> (colours (<= PAL_MAX), lines ignored)
fn pal_parse(t: &[u8]) -> (Vec<u32>, i32) {
    let mut cols = Vec::new();
    let mut bad = 0;
    // the text ends at its first NUL (the asm terminates the buffer)
    let t = &t[..t.iter().position(|&c| c == 0).unwrap_or(t.len())];
    let mut s = if t.starts_with(&[0xEF, 0xBB, 0xBF]) { 3 } else { 0 };
    while s < t.len() {
        let e = t[s..].iter().position(|&c| c == b'\n').map_or(t.len(), |k| s + k);
        match pal_line(&t[s..e]) {
            (0, _) => {}
            (1, c) if cols.len() < PAL_MAX => cols.push(c),
            _ => bad += 1, // malformed, or beyond 96 colours
        }
        if e >= t.len() {
            break;
        }
        s = e + 1; // past the LF
    }
    (cols, bad)
}

/// pal_load_file(path) -> (0 ok / -1, lines ignored).  The palette stays unchanged when the
/// file cannot be read or has no colour.
pub(crate) fn pal_load_file(pal: &mut Pal, path: &str) -> (i32, i32) {
    let Some(f) = plat::File::open(path, plat::FO_READ) else { return (-1, 0) };
    let Some(size) = f.size() else { return (-1, 0) };
    let size = size.min(PAL_READ_MAX) as usize;
    let mut buf = alloc::vec![0u8; size];
    let Some(got) = f.read_at(0, &mut buf) else { return (-1, 0) };
    drop(f);
    buf.truncate(got.min(size));
    let (cols, bad) = pal_parse(&buf);
    if cols.is_empty() {
        return (-1, bad);
    }
    pal.pal[..cols.len()].copy_from_slice(&cols);
    pal.n = cols.len();
    pal.mode = false;
    pal_changed(pal);
    (0, bad)
}

/// pal_save_file(path) -> 0 ok / -1: header + one AARRGGBB line per colour
pub(crate) fn pal_save_file(pal: &Pal, path: &str) -> i32 {
    let mut t = String::new();
    for l in PAL_HDR.iter() {
        t.push_str(l);
        t.push_str(EOL);
    }
    for &c in pal.pal[..pal.n.min(PAL_MAX)].iter() {
        t.push_str(&format!("{:08X}{}", c, EOL));
    }
    if plat::write_file(path, t.as_bytes()) { 0 } else { -1 }
}

/// pal_folder() -> (exists, the palette folder)
pub(crate) fn pal_folder(pal: &Pal) -> (bool, String) {
    // test hook: a fixed folder below the working directory
    if pal.tdir != 0 && pal.tdir < PAL_TDIR_N {
        let d = String::from(PAL_TDIRS[pal.tdir as usize]);
        return (plat::file_exists(&d) == 2, d);
    }
    if cfg!(windows) {
        // paint.net's own palette folder when it exists
        if let Some(h) = plat::home().filter(|h| !h.is_empty() && h.len() < PAL_PATH_CAP - 64) {
            let d = h + "/Documents/paint.net App Files/Palettes";
            if plat::file_exists(&d) == 2 {
                return (true, d);
            }
        }
    }
    match plat::config_dir().filter(|c| !c.is_empty() && c.len() < PAL_PATH_CAP - 64) {
        Some(c) => {
            let d = c + "/PaintCraft/Palettes";
            (plat::file_exists(&d) == 2, d)
        }
        None => (false, String::new()),
    }
}

// ------------------------------------------------------------------ palette menu list

// pal_scan() -- collect the *.txt files of the palette folder (sorted, at most LIST_MAX)
fn pal_scan(pal: &mut Pal) {
    pal.list.clear();
    let (ok, dir) = pal_folder(pal);
    pal.list_dir = dir;
    if !ok {
        return;
    }
    let mut list = Vec::new();
    plat::dir_list(&pal.list_dir, &mut |name: &str, is_dir: bool, _size: u32| {
        if is_dir || list.len() >= LIST_MAX {
            return;
        }
        if !name.ends_with(".txt") || name.len() >= NAME_CAP - 1 {
            return;
        }
        list.push(String::from(name));
    });
    // insertion sort, ASCII case-insensitive
    for i in 1..list.len() {
        let mut j = i;
        while j > 0 && pal_icmp(&list[j - 1], &list[j]) > 0 {
            list.swap(j - 1, j);
            j -= 1;
        }
    }
    pal.list = list;
}

// pal_icmp(a, b) -> < 0, 0, > 0  ASCII case-insensitive
fn pal_icmp(a: &str, b: &str) -> i32 {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut i = 0;
    loop {
        let x = a.get(i).copied().unwrap_or(0).to_ascii_lowercase() as i32;
        let y = b.get(i).copied().unwrap_or(0).to_ascii_lowercase() as i32;
        if x != y || y == 0 {
            return x - y;
        }
        i += 1;
    }
}

// pal_list_label(k) -> "name" of list entry k without ".txt", '&' doubled
fn pal_list_label(pal: &Pal, k: usize) -> String {
    let mut s = String::new();
    let Some(n) = pal.list.get(k) else { return s };
    let b = &n.as_bytes()[..n.len() - 4];
    let mut out: Vec<u8> = Vec::new();
    for &c in b {
        if out.len() >= LBL_CAP - 3 {
            break;
        }
        out.push(c);
        if c == b'&' {
            out.push(c);
        }
    }
    s.push_str(&String::from_utf8_lossy(&out));
    s
}

// ------------------------------------------------------------------ commands

/// pal_states() -- the palette commands work
pub(crate) fn pal_states(_p: &mut Pn, ui: &mut Ui) {
    for &c in PAL_CMD_IDS.iter() {
        pn_cmd_set(ui, c, 0);
    }
}

// pal_error(msg) -- show msg in the error dialog (main window)
fn pal_error(pal: &mut Pal, msg: &'static str) {
    pal.err_msg = Some(msg);
    pal.err_open.set(1);
    pal.repass = true;
}

// pal_dialog(mode, which) -- open the file dialog: mode 0 Öffnen, 1 Speichern unter; which 0 =
// start at the last palette file, 1 = in the palette folder
fn pal_dialog(pal: &mut Pal, mode: i32, which: i32) {
    pal.fd_mode = mode;
    pal.fd_path.clear();
    if which == 0 && !pal.last.is_empty() {
        pal.fd_path = pal.last.clone();
    } else {
        let (ok, dir) = pal_folder(pal);
        if ok {
            pal.fd_path = dir;
            if mode != 0 {
                pal.fd_path.push_str("/Palette.txt");
            }
        } else {
            pal.fd_path.clear();
            if mode != 0 {
                pal.fd_path = String::from("Palette.txt"); // save: a name only (last folder / home)
            }
        }
    }
    pal.fd_path = crate::app::formatting::fmt_cap(core::mem::take(&mut pal.fd_path), PAL_PATH_CAP);
    pal.fd_open.set(1);
    pal.repass = true;
}

// pal_load_path(path) -- load a palette chosen by the user (error dialog on failure)
fn pal_load_path(pal: &mut Pal, path: &str) {
    let (r, ign) = pal_load_file(pal, path);
    pal.ignored = ign;
    if r != 0 {
        pal_error(pal, S_ERR_LOAD);
        return;
    }
    pal.err_msg = None;
    pal.last = crate::app::formatting::fmt_cap(String::from(path), PAL_PATH_CAP);
}

/// pal_cmd(id, arg) -> 1 handled
pub(crate) fn pal_cmd(p: &mut Pn, _ui: &mut Ui, id: u32, arg: u32) -> bool {
    let pal = &mut p.pal;
    match id {
        CMD_PAL_OPEN => pal_dialog(pal, 0, 0),
        CMD_PAL_FOLDER => pal_dialog(pal, 0, 1),
        CMD_PAL_SAVE => pal_dialog(pal, 1, 0),
        CMD_PAL_RESET => pal_default(pal),
        CMD_PAL_LOAD => {
            let k = arg as usize;
            if k < pal.list.len() && k < LIST_MAX {
                let path = crate::app::formatting::fmt_cap(format!("{}/{}", pal.list_dir, pal.list[k]), PAL_PATH_CAP);
                pal.fd_path = path.clone();
                pal_load_path(pal, &path);
            }
        }
        CMD_PAL_ADD => pal_add_primary(pal),
        UIC_ESCAPE => {
            // Esc ends the replace mode (only then it is consumed)
            if !pal.mode {
                return false;
            }
            pal.mode = false;
            pal.repass = true;
        }
        _ => return false,
    }
    true
}

/// pal_frame() -- the file dialog (Öffnen / Speichern unter) and the error dialog
pub(crate) fn pal_frame(p: &mut Pn, ui: &mut Ui) {
    let pal = &mut p.pal;
    let flags = if pal.fd_mode == 0 { FD_OPEN } else { FD_SAVE };
    let open = pal.fd_open.clone();
    if ui.filedlg(ID_FD_PALETTE, flags, &open, &mut pal.fd_path, PAL_PATH_CAP as u32, &["txt=Palette"]) & EV_ACT != 0 {
        let path = pal.fd_path.clone();
        if pal.fd_mode == 0 {
            pal_load_path(pal, &path);
        } else if pal_save_file(pal, &path) != 0 {
            pal_error(pal, S_ERR_SAVE);
        } else {
            pal.err_msg = None;
            pal.last = crate::app::formatting::fmt_cap(path, PAL_PATH_CAP);
        }
    }
    let eo = pal.err_open.clone();
    if ui.dialog_begin(ID_PAL_ERR, "Palette", &eo) {
        // the message and the hint below it (two lines: TX_WRAP is not implemented)
        ui.text(TS_BODY, pal.err_msg.unwrap_or(""));
        let hint = if pal.err_msg == Some(S_ERR_SAVE) {
            "Der Ordner ist nicht beschreibbar."
        } else {
            "Die Datei ist nicht lesbar oder enthält keine Farbe."
        };
        ui.text(TS_BODY | TX_SECONDARY, hint);
        ui.dialog_buttons(Some(crate::preferences::localization::tr("OK")), None, None);
    }
    ui.dialog_end();
}

// ================================================================== Colors window parts

/// pal_checker(x, y, w, h) -- checkerboard (white / light grey) under transparent colours (px)
pub(crate) fn pal_checker(ui: &mut Ui, x: i32, y: i32, w: i32, h: i32) {
    ui.draw_color(0xFFFF_FFFF);
    ui.draw_rect(x, y, w, h);
    let cell = ui.dp(4).max(2);
    ui.draw_color(0xFFCC_CCCC);
    let mut ry = 0;
    let mut rp = 0u32; // row parity
    while ry < h {
        let mut cp = rp; // column parity
        let mut cx = 0;
        while cx < w {
            if cp & 1 != 0 {
                ui.draw_rect(x + cx, y + ry, cell.min(w - cx), cell.min(h - ry));
            }
            cp += 1;
            cx += cell;
        }
        rp += 1;
        ry += cell;
    }
}

/// pal_grid() -- the palette: 16 x 6 swatches (click = primary, right click = secondary;
/// replace mode: click = replace that swatch with the primary colour, an empty slot appends)
pub(crate) fn pal_grid(p: &mut Pn, ui: &mut Ui) {
    ui.next_size(PAL_COLS * PAL_CELL, PAL_ROWS * PAL_CELL);
    ui.box_begin(ID_PAL_GRID, BOX_H | BOX_FIXED, 0);
    let br = ui.box_rect();
    let (x0, y0) = (br[0], br[1]);
    let cell = ui.dp(PAL_CELL);
    let hair = ui.hair();
    let sq = cell - hair;
    let pal = &mut p.pal;
    // replace mode: an accent frame around the grid
    if pal.mode {
        let c = ui.color(T_ACCENT);
        ui.draw_color(c);
        let h2 = hair + hair;
        ui.draw_frame(x0 - h2, y0 - h2, cell * PAL_COLS + h2 + h2, cell * PAL_ROWS + h2 + h2, 0, h2);
    }
    // the keyboard: the swatches are one Tab stop, the arrows move in the grid, Space / Enter
    // = a click, the menu key = a right click (secondary colour)
    ui.focus_group_begin(ID_PAL_GRID);
    for i in 0..PAL_MAX {
        // cell position
        let x = (i as i32 & (PAL_COLS - 1)) * cell + x0;
        let y = (i as i32 >> 4) * cell + y0;
        let hb = if i < pal.n || pal.mode { HB_FOCUS } else { 0 };
        let ev = ui.hit_rect(ID_PAL_SW0 + i as u32, x, y, sq, sq, hb);
        if i < pal.n {
            let c = pal.pal[i];
            // colour (over a checkerboard when it is transparent), hairline edge
            if c < 0xFF00_0000 {
                pal_checker(ui, x, y, sq, sq);
            }
            ui.draw_color(c);
            ui.draw_rect(x, y, sq, sq);
            let e = ui.color(if ev & EV_HOVER != 0 { T_TEXT } else { T_CTRL_STROKE2 });
            ui.draw_color(e);
            ui.draw_frame(x, y, sq, sq, 0, hair);
            ui.focus_ring_draw(ev);
            // record (label AARRGGBB) and tooltip (#RRGGBB, #AARRGGBB when transparent)
            ui.sem(K_SWATCH, ID_PAL_SW0 + i as u32, Some(&format!("{:08X}", c)), 1);
            if ev & EV_HOVER != 0 {
                let tip = if c >= 0xFF00_0000 { format!("#{:06X}", c & 0xFF_FFFF) } else { format!("#{:08X}", c) };
                ui.tooltip(&tip);
            }
            if ev & (EV_ACT | EV_RCLICK) == 0 {
                continue;
            }
            if !pal.mode {
                if ev & EV_ACT != 0 {
                    pal_set_primary(pal, c);
                } else {
                    pal_set_secondary(pal, c);
                }
                continue;
            }
        } else {
            // a free slot: an empty well
            let e = ui.color(if ev & EV_HOVER != 0 { T_TEXT3 } else { T_CTRL_STROKE2 });
            ui.draw_color(e);
            ui.draw_frame(x, y, sq, sq, 0, hair);
            ui.focus_ring_draw(ev);
            ui.sem(K_SWATCH, ID_PAL_SW0 + i as u32, Some(""), 0);
            if ev & (EV_ACT | EV_RCLICK) == 0 || !pal.mode {
                continue;
            }
        }
        // replace: left click the primary colour, right click the secondary colour
        let c = if ev & EV_ACT != 0 { pal.pri } else { pal.sec };
        let k = if i > pal.n { pal.n } else { i }; // an empty slot: append
        pal.mode = false;
        pal_set_entry(pal, k as i32, c);
    }
    ui.focus_group_end();
    ui.box_end();
}

/// pal_buttons() -- "Farbe hinzufügen" (+) and the palette menu (drop-down with the palette
/// commands and the palette files of the palette folder)
pub(crate) fn pal_buttons(p: &mut Pn, ui: &mut Ui) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 0);
    ui.next_size(28, 28);
    if ui.icon_button(ID_PAL_ADD, IC_ADD, "Farbe hinzufügen") & EV_ACT != 0 {
        pal_add_primary(&mut p.pal);
    }
    ui.next_flags(ST_SUBTLE | ST_ICON_ONLY);
    ui.next_icon(IC_COLOR);
    ui.next_size(0, 28);
    let ev = ui.dropdown_button(ID_PAL_MENU, crate::preferences::localization::tr("Paletten"));
    // the menu just opened: read the palette folder
    let pal = &mut p.pal;
    let mut was = false;
    if ev & EV_OPEN != 0 {
        if !pal.menu_open {
            pal.menu_open = true;
            pal_scan(pal);
        }
        was = true;
    }
    pal.menu_open = was;
    if ui.flyout_begin(ID_PAL_MENU, FL_BELOW) {
        ui.menu_cmd(CMD_PAL_FOLDER);
        ui.menu_cmd(CMD_PAL_OPEN);
        ui.menu_cmd(CMD_PAL_SAVE);
        ui.menu_cmd(CMD_PAL_RESET);
        if !pal.list.is_empty() {
            ui.menu_sep();
            for k in 0..pal.list.len() {
                if k >= LIST_SHOW {
                    if ui.menu_item(ID_PAL_MORE, "Weitere Paletten…") & EV_ACT != 0 {
                        ui.cmd_run(CMD_PAL_FOLDER, 0);
                    }
                    break;
                }
                let l = pal_list_label(pal, k);
                if ui.menu_item(ID_PAL_LIST0 + k as u32, &l) & EV_ACT != 0 {
                    ui.cmd_run(CMD_PAL_LOAD, k as u32);
                }
            }
        }
    }
    ui.flyout_end();
    ui.box_end();
}

// ================================================================== test hooks (A3)
// cmd 985 k      palette folder for the tests: 0 = the real one, 1 = build/, 2 = the fixtures
//                apps/PaintCraft/tests/fixtures/palettes
// cmd 986 argb   primary colour          cmd 987 argb   secondary colour
// cmd 988 n      palette = n generated colours (FF000000 + i * 010203), n <= 96
// Records (hidden K_TEXT, refreshed every frame):
//   9985 text primary AARRGGBB, value pal_version     9986 text secondary AARRGGBB
//   9987 value colours in the palette, text the name of the last palette file
//   9988 value lines ignored by the last load, text the last error ("" = none)
//   9989 value 1 while the replace mode ("Farbe hinzufügen" on a full palette) is on

/// pal_diag_cmd(id, arg) -> 1 handled
pub(crate) fn pal_diag_cmd(p: &mut Pn, _ui: &mut Ui, id: u32, arg: u32) -> bool {
    let pal = &mut p.pal;
    match id {
        CMD_PAL_TEST0 => pal.tdir = if arg < PAL_TDIR_N { arg } else { 0 },
        _ if id == CMD_PAL_TEST0 + 1 => pal_set_primary(pal, arg),
        _ if id == CMD_PAL_TEST0 + 2 => pal_set_secondary(pal, arg),
        _ if id == CMD_PAL_TEST0 + 3 => {
            let n = (arg as usize).min(PAL_MAX);
            for i in 0..n {
                pal.pal[i] = (i as u32).wrapping_mul(0x010203) | 0xFF00_0000;
            }
            pal.n = n;
            pal.mode = false;
            pal_changed(pal);
        }
        _ => return false,
    }
    true
}

// pal_probe(id, label, value) -- a hidden K_TEXT record (empty rectangle)
fn pal_probe(ui: &mut Ui, id: u32, label: &str, value: i32) {
    ui.last_reset();
    ui.sem(K_TEXT, id, Some(label), value);
}

/// pal_diag_frame() -- the probe records 9985..9989
pub(crate) fn pal_diag_frame(p: &mut Pn, ui: &mut Ui) {
    let pal = &p.pal;
    pal_probe(ui, ID_PAL_PROBE, &format!("{:08X}", pal.pri), pal.ver as i32);
    pal_probe(ui, ID_PAL_PROBE + 1, &format!("{:08X}", pal.sec), 0);
    // name of the last palette file (after the last '/')
    let name = match pal.last.rfind('/') {
        Some(k) => &pal.last[k + 1..],
        None => &pal.last[..],
    };
    pal_probe(ui, ID_PAL_PROBE + 2, name, pal.n as i32);
    pal_probe(ui, ID_PAL_PROBE + 3, pal.err_msg.unwrap_or(""), pal.ignored);
    pal_probe(ui, ID_PAL_PROBE + 4, "", pal.mode as i32);
}

/// pal_set_all(colors) -- replace the whole palette (settings.rs): n clamped to 1..PAL_MAX
/// (empty: nothing happens)
pub(crate) fn pal_set_all(pal: &mut Pal, colors: &[u32]) {
    if colors.is_empty() {
        return;
    }
    let n = colors.len().min(PAL_MAX);
    pal.pal[..n].copy_from_slice(&colors[..n]);
    pal.n = n;
    pal.mode = false;
    pal_changed(pal);
}
