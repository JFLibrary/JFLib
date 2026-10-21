//! Persistent editor preferences and localization.
pub(crate) mod localization;

// settings -- PaintCraft: the settings file (docs/SETTINGS.md).
// settings.ini lives in plat::config_dir()/PaintCraft/; `--config DIR` or $PaintCraft_CONFIG_DIR
// use DIR/settings.ini instead; headless runs (--shot / --script) without either have no file
// at all (nothing is read or written).  The INI object stays loaded, so comments and unknown
// keys survive a save (base::ini; temp file + rename).  The file is input: every value is
// range checked; malformed / out-of-range values are ignored (window sizes are clamped),
// strings are length checked.
//                                                               [apps/PaintCraft/src/settings.s]
//
//   st_init()                (set_init) path, factory values, read the file, apply what works
//                            before the first frame (theme, colours, view, tool options,
//                            recent files, history, threads, main window, effects)
//   st_states()              (set_states) the first call applies the tool windows and the
//                            default tool (their modules initialise after set_init); every
//                            call tracks the main window's normal rectangle
//   st_save() -> 0 written, 1 no persistence, -1 write error
//   set_save_quit()          app/lifecycle.rs: Beenden / closing the window (= st_save)
//   st_reload()              factory state, then read the file again (test hook)
//   st_factory()             factory state of everything the file holds (test hook)
//   st_check_screens()       windows outside every monitor go to their default place
//   st_timer(id) -> handled  TMR_SET_*
//   st_tools_from_bar()      tool defaults = the current tool bar options
//   st_tools_reset()         tool defaults = the factory defaults
//   st_budget_apply()        ST_BUDGET_MB -> hist_budget_set
//   st_tempdir_apply()       ST_TEMPDIR -> hist_temp_dir_set
//   st_hex(str) -> value     6 (opaque) or 8 hex digits, nothing else
//   st_tc_text(&TOOLCFG) -> compact text of tool options (probes)
// Data for the dialog: St::st (ST_* of pn_set.inc), file, on, bauto, mrect, mmax, err,
// PN_VERSION, TOOL_NAMES / THEME_NAMES.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::ini::Ini;
use gfx::blendmode::BM_COUNT;
use gfx::flood::FF_GLOBAL;
use ui::*;

use crate::workspace::chrome::chrome_wnd_open;
use crate::app::constants::*;
use crate::tools::ToolCfg;
use crate::workspace::panels::dock::{dock_rec, dock_reset};
use crate::app::formatting::fmt_cap;
use crate::app::Pn;

const FILE_CAP: usize = ST_PATH_CAP as usize;
const TXT_CAP: usize = 1024; // palette text (96 x 9 bytes)
const PAL_N: usize = 96; // palette.rs PAL_MAX
const WIN_N: usize = 4; // tool windows
const POS_LIM: i32 = 20000; // tool window offsets (DIP) beyond: default place
const MPOS_LIM: i32 = 100000; // main window position (screen px)
const NOVAL: i32 = i32::MIN; // "missing" marker of st_geti
const NEED_X: i32 = 48; // px of a window that must be on a monitor
const NEED_Y: i32 = 24;

const SEC_PN: &str = "PaintCraft";
const SEC_UI: &str = "UI";
const SEC_MAIN: &str = "MainWindow";
const SEC_COL: &str = "Colors";
const SEC_VIEW: &str = "View";
const SEC_TOOLS: &str = "Tools";
const SEC_RECENT: &str = "RecentFiles";
const SEC_HIST: &str = "History";
const SEC_PERF: &str = "Performance";

/// s_pn_version
pub(crate) const PN_VERSION: &str = "0.7";

/// tool_names: TOOL_* order (pn_tool.inc)
pub(crate) static TOOL_NAMES: [&str; TOOL_COUNT as usize] = [
    "RectangleSelect",
    "MoveSelectedPixels",
    "LassoSelect",
    "MoveSelection",
    "EllipseSelect",
    "Zoom",
    "MagicWand",
    "Pan",
    "PaintBucket",
    "Gradient",
    "Paintbrush",
    "Eraser",
    "Pencil",
    "ColorPicker",
    "CloneStamp",
    "Recolor",
    "Text",
    "LineCurve",
    "Shapes",
];

/// theme_names
pub(crate) static THEME_NAMES: [&str; 3] = ["light", "dark", "system"];
static UNIT_NAMES: [&str; 3] = ["pixel", "inch", "cm"];

/// the tool windows (win_tab): section, sizeable
static WIN_SEC: [(&str, bool); WIN_N] =
    [("ToolsWindow", false), ("HistoryWindow", true), ("LayersWindow", true), ("ColorsWindow", false)];

/// the integer tool options (tc_tab): key, lo, hi
static TC_TAB: [(&str, i32, i32); 12] = [
    ("hardness", 0, 100),
    ("spacing", 1, 100),
    ("antialias", 0, 1),
    ("blend", 0, BM_COUNT as i32 - 1),
    ("fill", 0, FS_COUNT - 1),
    ("tolerance", 0, 100),
    ("flood", 0, FF_GLOBAL as i32),
    ("sampling", 0, SMP_IMAGE),
    ("selection_mode", 0, crate::app::constants::SM_COUNT - 1),
    ("selection_quality", 0, SQ_ALIASED),
    ("pressure", 0, 1),
    ("smoothing", 0, 1),
];

// the TOOLCFG field of tc_tab row k
fn tc_field(c: &mut ToolCfg, k: usize) -> &mut u32 {
    match k {
        0 => &mut c.hard,
        1 => &mut c.spacing,
        2 => &mut c.aa,
        3 => &mut c.blend,
        4 => &mut c.fill,
        5 => &mut c.tol,
        6 => &mut c.flood,
        7 => &mut c.sample,
        8 => &mut c.selmode,
        9 => &mut c.selqual,
        10 => &mut c.pressure,
        _ => &mut c.smooth,
    }
}

/// g_st: the values the settings dialog edits (ST_* of pn_set.inc)
#[derive(Clone, Default)]
pub(crate) struct StRec {
    pub(crate) language: u32,
    pub(crate) update_auto: u32,
    pub(crate) update_beta: u32,
    pub(crate) workspace: u32,
    pub(crate) checker0: u32,
    pub(crate) checker1: u32,
    pub(crate) canvas_shadow: u32,
    /// ST_THEME: 0 light, 1 dark, 2 system
    pub(crate) theme: u32,
    /// ST_ACCENT: 0xRRGGBB, 0 = the default accent
    pub(crate) accent: u32,
    /// ST_ANIM: animations on
    pub(crate) anim: u32,
    /// ST_THREADS: worker threads, 0 = automatic
    pub(crate) threads: u32,
    /// ST_BUDGET_MB: history budget in MB, 0 = automatic
    pub(crate) budget_mb: u32,
    /// ST_DEFTOOL: TOOL_* active at start-up
    pub(crate) deftool: u32,
    /// ST_SAVES: files written (probe)
    pub(crate) saves: u32,
    /// ST_LOADED: a settings file was read
    pub(crate) loaded: u32,
    /// ST_TOOLDEF: defaults of the tool bar
    pub(crate) tooldef: ToolCfg,
    /// ST_TEMPDIR: spill folder ("" = temp dir; < ST_PATH_CAP bytes)
    pub(crate) tempdir: String,
    /// ST_PEN: (PN1) pen input
    pub(crate) pen: u32,
}

/// the state of settings.s
#[derive(Default)]
pub(crate) struct St {
    /// g_st
    pub(crate) st: StRec,
    /// g_st_file: the settings file ("" = no persistence)
    pub(crate) file: String,
    /// g_st_on: a settings file is used
    pub(crate) on: bool,
    /// g_st_bauto: automatic history budget (bytes)
    pub(crate) bauto: u64,
    /// g_st_mrect: main window: normal rect (x, y px; w, h DIP)
    pub(crate) mrect: WinRect,
    /// g_st_mmax: main window maximized
    pub(crate) mmax: bool,
    /// g_st_err: result of the last ini save (0 / -1)
    pub(crate) err: i32,
    /// g_st_ini: the INI (None before st_init)
    ini: Option<Ini>,
    /// g_st_late: st_apply_late still to do
    late: bool,
    /// g_st_mvalid: g_st_mrect holds a rectangle
    mvalid: bool,
    /// g_st_maxwait: the restored maximize is pending (TMR_SET_MAX)
    maxwait: bool,
    /// g_st_restored: a window position came from the file
    restored: bool,
    /// g_st_moved: windows moved back onto a monitor (probe)
    pub(crate) moved: u32,
    /// g_st_fcfg: factory TOOLCFG (tool_init)
    fcfg: ToolCfg,
}

// ------------------------------------------------------------------ INI helpers

// st_geti(key, def, lo, hi) -> integer of key in section sec; def when it is missing,
// malformed or outside lo..hi
fn st_geti(s: &St, sec: &str, key: &str, def: i32, lo: i32, hi: i32) -> i32 {
    let Some(ini) = &s.ini else { return def };
    let v = ini.get_int(sec, key, def as i64);
    if v < lo as i64 || v > hi as i64 { def } else { v as i32 }
}

// st_getc(key, def, lo, hi) -> like st_geti, but clamped to lo..hi
fn st_getc(s: &St, sec: &str, key: &str, def: i32, lo: i32, hi: i32) -> i32 {
    let Some(ini) = &s.ini else { return def };
    ini.get_int(sec, key, def as i64).max(lo as i64).min(hi as i64) as i32
}

// st_gets(key) -> value string or None
fn st_gets<'a>(s: &'a St, sec: &str, key: &str) -> Option<&'a str> {
    s.ini.as_ref()?.get(sec, key)
}

// st_seti / st_sets / st_del -- write into section sec
fn st_seti(s: &mut St, sec: &str, key: &str, v: i32) {
    if let Some(ini) = &mut s.ini {
        ini.set_int(sec, key, v as i64);
    }
}

fn st_sets(s: &mut St, sec: &str, key: &str, v: &str) {
    if let Some(ini) = &mut s.ini {
        ini.set(sec, key, v);
    }
}

fn st_del(s: &mut St, sec: &str, key: &str) {
    if let Some(ini) = &mut s.ini {
        ini.remove(sec, key);
    }
}

// st_getenum(key, list, def) -> index of the value in list, def when unknown
fn st_getenum(s: &St, sec: &str, key: &str, list: &[&str], def: u32) -> u32 {
    let Some(v) = st_gets(s, sec, key) else { return def };
    list.iter().position(|&e| e == v).map_or(def, |i| i as u32)
}

// st_setenum(key, list, index) -- write entry `index` of list (nothing when out of range)
fn st_setenum(s: &mut St, sec: &str, key: &str, list: &[&str], i: u32) {
    if let Some(&v) = list.get(i as usize) {
        st_sets(s, sec, key, v);
    }
}

/// st_hex(str) -> value: exactly 8 hex digits (AARRGGBB) or 6 (RRGGBB, opaque) and nothing
/// else; None otherwise
pub(crate) fn st_hex(s: &str) -> Option<u32> {
    let mut v = 0u32;
    let mut n = 0;
    for &c in s.as_bytes() {
        if n >= 8 {
            return None;
        }
        let d = match c {
            b'0'..=b'9' => (c - b'0') as u32,
            _ => {
                let l = c | 0x20;
                if !(b'a'..=b'f').contains(&l) {
                    return None;
                }
                (l - b'a') as u32 + 10
            }
        };
        v = v << 4 | d;
        n += 1;
    }
    match n {
        8 => Some(v),
        6 => Some(v | 0xFF00_0000),
        _ => None,
    }
}

// arg_int(name, def)
fn arg_int(name: &str, def: i32) -> i32 {
    match plat::arg_value(name) {
        Some(v) => base::num::parse_i64(v.as_bytes()).unwrap_or(0) as i32,
        None => def,
    }
}

// ------------------------------------------------------------------ start-up

// st_find_path() -- file / on: --config DIR, $PaintCraft_CONFIG_DIR, else (only with a window)
// plat::config_dir()/PaintCraft; + "/settings.ini"
fn st_find_path(s: &mut St, ui: &Ui) {
    s.file.clear();
    s.on = false;
    let dir = match plat::arg_value("--config").filter(|v| !v.is_empty()) {
        Some(v) => {
            if v.len() >= FILE_CAP - 32 {
                return; // too long: no persistence
            }
            v
        }
        None => match plat::env("PaintCraft_CONFIG_DIR").filter(|v| !v.is_empty() && v.len() < FILE_CAP - 32) {
            Some(v) => v,
            None => {
                if ui.headless() {
                    return; // tests never touch the user's file
                }
                match plat::config_dir().filter(|v| !v.is_empty() && v.len() < FILE_CAP - 32) {
                    Some(v) => fmt_cap(format!("{}/PaintCraft", v), FILE_CAP),
                    None => return,
                }
            }
        },
    };
    s.file = fmt_cap(format!("{}/settings.ini", dir), FILE_CAP);
    s.on = true;
}

/// st_init() -- see the header (set_init: after tool_init, before the tool windows' inits)
pub(crate) fn st_init(p: &mut Pn, ui: &mut Ui) {
    let cfg = *crate::tools::tool_cfg(&p.tool); // factory tool options
    let s = &mut p.st;
    s.fcfg = cfg;
    s.st.tooldef = cfg;
    s.st.deftool = TOOL_DEFAULT as u32;
    s.st.theme = THEME_SYSTEM as u32;
    s.st.pen = 1; // (PN1)
    s.st.checker0 = 0xFFFFFFFF;
    s.st.checker1 = 0xFFCCCCCC;
    s.st.canvas_shadow = 1;
    s.st.anim = !ui.anim_off() as u32;
    s.bauto = crate::document::history::memory::hist_budget(&p.hm);
    s.late = true;
    st_find_path(s, ui);
    s.ini = Some(if s.on { Ini::load(&s.file) } else { Ini::new() }); // a missing file = an empty INI
    if s.on {
        st_load_early(p, ui);
    }
    // test hook for real windows (--auto-close quits without UIC_CLOSE): save after MS
    let ms = arg_int("--settings-save-at", 0);
    if ms > 0 {
        ui.timer(TMR_SET_SAVE, ms as u32);
    }
}

// st_tools_apply() -- the tool defaults into the tool bar (TOOLCFG, TC_VER kept + bumped)
fn st_tools_apply(p: &mut Pn) {
    let ver = p.tool.cfg.ver;
    p.tool.cfg = p.st.st.tooldef;
    p.tool.cfg.ver = ver;
    crate::tools::tool_cfg_changed(p);
}

/// st_budget_apply() -- ST_BUDGET_MB (0 = automatic) -> hist_budget_set
pub(crate) fn st_budget_apply(p: &mut Pn) {
    let b = (p.st.st.budget_mb as u64) << 20;
    crate::document::history::memory::hist_budget_set(p, b);
}

/// st_tempdir_apply() -- ST_TEMPDIR ("" = the system temp folder) -> hist_temp_dir_set
pub(crate) fn st_tempdir_apply(p: &mut Pn) {
    let d = &p.st.st.tempdir;
    crate::document::history::memory::hist_temp_dir_set(&mut p.hm, if d.is_empty() { None } else { Some(d.as_str()) });
}

// st_pal_parse(str) -> the colours of "AARRGGBB,AARRGGBB,..." (6 digits = opaque; malformed
// entries are skipped, at most PAL_N)
fn st_pal_parse(s: &str) -> Vec<u32> {
    let mut v = Vec::new();
    for tok in s.split(',') {
        if v.len() >= PAL_N {
            break;
        }
        // spaces are ignored; 9 or more characters = too long
        let t: String = tok.chars().filter(|&c| c != ' ').collect();
        if t.len() > 8 {
            continue;
        }
        if let Some(c) = st_hex(&t) {
            v.push(c);
        }
    }
    v
}

// st_color(which, key) -- pal_set_color(which, hex value of key) when it is valid
fn st_color(p: &mut Pn, which: i32, key: &str) {
    if let Some(c) = st_gets(&p.st, SEC_COL, key).and_then(st_hex) {
        crate::workspace::palette::pal_set_color(&mut p.pal, which, c);
    }
}

// st_load_early() -- everything that can be applied before the first frame
fn st_load_early(p: &mut Pn, ui: &mut Ui) {
    let v = st_geti(&p.st, SEC_PN, "version", 0, 0, 0x7FFF_FFFF);
    p.st.st.loaded = (v != 0) as u32;
    // ---- UI (--theme / --accent / --anim on the command line win)
    let s = &mut p.st;
    s.st.language = st_getenum(s, SEC_UI, "language", &["de", "en", "fr"], 0);
    s.st.theme = st_getenum(s, SEC_UI, "theme", &THEME_NAMES, s.st.theme);
    if plat::arg_value("--theme").is_none() {
        ui.theme_set(s.st.theme);
    }
    if let Some(c) = st_gets(s, SEC_UI, "accent").and_then(st_hex) {
        s.st.accent = c & 0xFF_FFFF;
        if plat::arg_value("--accent").is_none() {
            ui.accent_set(s.st.accent);
        }
    }
    s.st.pen = st_geti(s, SEC_UI, "peninput", s.st.pen as i32, 0, 1) as u32; // (PN1) Windows-Tinte
    plat::pen_enable(s.st.pen != 0);
    s.st.anim = st_geti(s, SEC_UI, "animations", s.st.anim as i32, 0, 1) as u32;
    if plat::arg_value("--anim").is_none() {
        ui.set_anim_off(s.st.anim == 0);
    }
    // ---- colours
    st_color(p, 0, "primary");
    st_color(p, 1, "secondary");
    if let Some(t) = st_gets(&p.st, SEC_COL, "palette") {
        let cols = st_pal_parse(t);
        if !cols.is_empty() {
            crate::workspace::palette::pal_set_all(&mut p.pal, &cols);
        }
    }
    // ---- Ansicht
    p.st.st.workspace = st_gets(&p.st,SEC_VIEW,"workspace").and_then(st_hex).unwrap_or(0);
    p.st.st.checker0 = st_gets(&p.st,SEC_VIEW,"checker0").and_then(st_hex).unwrap_or(0xFFFFFFFF);
    p.st.st.checker1 = st_gets(&p.st,SEC_VIEW,"checker1").and_then(st_hex).unwrap_or(0xFFCCCCCC);
    p.st.st.canvas_shadow = st_geti(&p.st,SEC_VIEW,"shadow",1,0,1) as u32;
    p.st.st.update_auto = st_geti(&p.st,"Updates","automatic",0,0,1) as u32;
    p.st.st.update_beta = st_geti(&p.st,"Updates","beta",0,0,1) as u32;
    let v = &p.view;
    let units = st_getenum(&p.st, SEC_VIEW, "units", &UNIT_NAMES, crate::workspace::view::view_units(v));
    let grid = st_geti(&p.st, SEC_VIEW, "grid", crate::workspace::view::view_grid(v) as i32, 0, 1);
    let rulers = st_geti(&p.st, SEC_VIEW, "rulers", crate::workspace::view::view_rulers(v) as i32, 0, 1);
    crate::workspace::view::view_set_opts(p, units, grid != 0, rulers != 0);
    // ---- tool defaults (the tool itself: st_apply_late)
    let s = &mut p.st;
    s.st.deftool = st_getenum(s, SEC_TOOLS, "default", &TOOL_NAMES, s.st.deftool);
    if let Some(ini) = &s.ini {
        let w = ini.get_f64(SEC_TOOLS, "width", s.st.tooldef.width);
        if w >= 1.0 && w <= 2000.0 {
            s.st.tooldef.width = w; // (NaN: unordered = below)
        }
    }
    for (k, &(key, lo, hi)) in TC_TAB.iter().enumerate() {
        let cur = *tc_field(&mut s.st.tooldef, k) as i32;
        let v = st_geti(s, SEC_TOOLS, key, cur, lo, hi);
        *tc_field(&mut s.st.tooldef, k) = v as u32;
    }
    st_tools_apply(p);
    // ---- recent files: file10 .. file1 (the oldest first, so file1 ends on top)
    for i in (1..=RECENT_MAX).rev() {
        let key = format!("file{}", i);
        if let Some(path) = st_gets(&p.st, SEC_RECENT, &key) {
            let path = String::from(path);
            crate::integrations::file_io::fio_recent_add(p, &path);
        }
    }
    // ---- Verlauf
    p.st.st.budget_mb = st_geti(&p.st, SEC_HIST, "budget_mb", 0, 0, BUDGET_MB_MAX) as u32;
    st_budget_apply(p);
    if let Some(d) = st_gets(&p.st, SEC_HIST, "temp_dir") {
        if d.len() < ST_PATH_CAP as usize {
            p.st.st.tempdir = String::from(d);
            st_tempdir_apply(p);
        }
    }
    // ---- threads (the pool starts lazily: only effective before its first use)
    let s = &mut p.st;
    s.st.threads = st_geti(s, SEC_PERF, "threads", 0, 0, THREADS_MAX) as u32;
    if s.st.threads != 0 && plat::arg_value("--threads").is_none() {
        plat::thr_init(s.st.threads);
    }
    // ---- main window (only with a window and without --size)
    if !ui.headless() && plat::arg_value("--size").is_none() && st_gets(s, SEC_MAIN, "width").is_some() {
        s.mrect.w = st_getc(s, SEC_MAIN, "width", 1280, 640, 16384);
        s.mrect.h = st_getc(s, SEC_MAIN, "height", 800, 400, 16384);
        s.mrect.flags = WRF_ABS;
        let x = st_geti(s, SEC_MAIN, "x", NOVAL, -MPOS_LIM, MPOS_LIM);
        let y = st_geti(s, SEC_MAIN, "y", NOVAL, -MPOS_LIM, MPOS_LIM);
        if x != NOVAL && y != NOVAL {
            s.mrect.x = x;
            s.mrect.y = y;
            s.mrect.flags |= WRF_POS;
        }
        s.mvalid = true;
        s.restored = true;
        s.mmax = st_geti(s, SEC_MAIN, "maximized", 0, 0, 1) != 0;
        // placed once the window runs (TMR_SET_PLACE); st_track waits for it
        s.maxwait = true;
        ui.timer(TMR_SET_PLACE, 1);
    }
    // ---- effects (fx.rs: section "Effekte")
    if let Some(ini) = &p.st.ini {
        crate::effects::fx_settings_load(&mut p.fx, ini);
    }
}

// the WINRECT of tool window k (win_tab: the *_rect getters)
fn win_rect(p: &Pn, k: usize) -> Shared<WinRect> {
    match k {
        0 => crate::workspace::panels::tools::wtools_rect(&p.wtools),
        1 => crate::workspace::panels::history::whist_rect(&p.whist),
        2 => crate::workspace::panels::layers::wlay_rect(&p.wlay),
        _ => crate::workspace::panels::colors::wcol_rect(&p.wcol),
    }
}

// the default place of tool window k (win_tab: the *_reset_pos functions)
fn win_reset_pos(p: &mut Pn, k: usize) {
    match k {
        0 => crate::workspace::panels::tools::wtools_reset_pos(p), // pn-ui
        1 => crate::workspace::panels::history::whist_reset_pos(p), // pn-ui
        2 => crate::workspace::panels::layers::wlay_reset_pos(p), // pn-ui
        _ => crate::workspace::panels::colors::wcol_reset_pos(p), // pn-ui
    }
}

// st_windows_load() -- the four tool windows: visibility, position, size, Mehr
fn st_windows_load(p: &mut Pn, ui: &mut Ui) {
    for (k, &(sec, sizeable)) in WIN_SEC.iter().enumerate() {
        let rs = win_rect(p, k);
        let mut r = rs.get();
        let flag = chrome_wnd_open(&p.chrome, k as i32);
        flag.set(st_geti(&p.st, sec, "visible", flag.get(), 0, 1));
        if k == WND_COLORS as usize {
            let m = crate::workspace::panels::colors::wcol_more(&p.wcol) as i32;
            let v = st_geti(&p.st, sec, "expanded", m, 0, 1);
            crate::workspace::panels::colors::wcol_set_more(p, v != 0); // pn-ui
        }
        if sizeable {
            r.w = st_getc(&p.st, sec, "width", r.w, 120, 4000);
            r.h = st_getc(&p.st, sec, "height", r.h, 80, 4000);
        }
        // placement (wnd_dock.rs): "dock" 0 automatic, 1 anchored (ax dx ay dy), 2 free (x y);
        // files without "dock" (older versions) get the automatic layout
        let mode = st_geti(&p.st, sec, "dock", NOVAL, 0, DM_FREE);
        if mode != NOVAL {
            let s = &p.st;
            let (ax, dx, ay, dy) = (
                st_geti(s, sec, "ax", 0, 0, 1),
                st_geti(s, sec, "dx", 0, -POS_LIM, POS_LIM),
                st_geti(s, sec, "ay", 0, 0, 1),
                st_geti(s, sec, "dy", 0, -POS_LIM, POS_LIM),
            );
            let (x, y) = (st_geti(s, sec, "x", NOVAL, -POS_LIM, POS_LIM), st_geti(s, sec, "y", NOVAL, -POS_LIM, POS_LIM));
            let d = dock_rec(&mut p.dock, k as i32);
            d.mode = mode;
            if mode == DM_ANCHOR {
                d.ax = ax;
                d.dx = dx;
                d.ay = ay;
                d.dy = dy;
            } else if mode == DM_FREE {
                if x != NOVAL && y != NOVAL {
                    r.x = x;
                    r.y = y;
                    d.apx = x;
                    d.apy = y;
                    r.flags |= WRF_POS;
                    p.st.restored = true;
                } else {
                    d.mode = DM_AUTO;
                }
            }
        }
        rs.set(r);
    }
    ui.repass();
}

// st_apply_late() -- tool windows and the default tool (their modules are initialised now)
fn st_apply_late(p: &mut Pn, ui: &mut Ui) {
    p.st.late = false;
    if !p.st.on || p.st.ini.is_none() {
        return;
    }
    p.custom_brush.path = st_gets(&p.st, "BrushTip", "file").unwrap_or("").into();
    let b = &mut p.custom_brush.dynamics;
    b.angle = st_geti(&p.st, "BrushTip", "angle", 0, 0, 359);
    b.scatter = st_geti(&p.st, "BrushTip", "scatter", 0, 0, 200);
    b.size_jitter = st_geti(&p.st, "BrushTip", "size_jitter", 0, 0, 95);
    b.angle_jitter = st_geti(&p.st, "BrushTip", "angle_jitter", 0, 0, 180);
    b.flow = st_geti(&p.st, "BrushTip", "flow", 100, 0, 100);
    b.follow = st_geti(&p.st, "BrushTip", "follow", 0, 0, 1);
    b.accumulate = st_geti(&p.st, "BrushTip", "accumulate", 0, 0, 1) != 0;
    b.flow_byte = u8::try_from(st_geti(&p.st, "BrushTip", "flow_byte", -1, -1, 255)).ok();
    b.opacity_cap = st_geti(&p.st, "BrushTip", "opacity_cap", 255, 0, 255) as u8;
    b.density = st_geti(&p.st, "BrushTip", "density", 0, 0, 100) as u32;
    if !crate::tools::brushes::custom::load(&mut p.custom_brush) { p.custom_brush.path.clear(); }
    st_windows_load(p, ui);
    if crate::tools::tool_cur(&p.tool) != p.st.st.deftool as i32 {
        let t = p.st.st.deftool as i32;
        crate::tools::tool_set(p, ui, t);
    }
    if !ui.headless() && p.st.restored {
        ui.timer(TMR_SET_SCREENS, 800);
    }
}

/// st_states() -- the first call applies the late part; every call tracks the main window
pub(crate) fn st_states(p: &mut Pn, ui: &mut Ui) {
    if p.st.late {
        st_apply_late(p, ui);
    }
    st_track(p, ui);
}

// st_track() -- the main window's normal rectangle (position px, client size DIP) and whether
// it is maximized; nothing while minimized, in headless runs, or while the restored maximized
// state is still pending
fn st_track(p: &mut Pn, ui: &mut Ui) {
    if ui.headless() {
        return;
    }
    let ws = plat::win_state(0);
    if ws & plat::WST_MINIMIZED != 0 {
        return;
    }
    let s = &mut p.st;
    if ws & (plat::WST_MAXIMIZED | plat::WST_FULLSCREEN) != 0 {
        s.mmax = true;
        return;
    }
    if s.maxwait {
        return;
    }
    s.mmax = false;
    // where Windows put it (it may have moved a requested position); X11 / Wayland: the
    // library's record
    let r = if cfg!(windows) { plat::win_get_rect(0) } else { None };
    match r {
        Some(r) => {
            s.mrect.x = r[0];
            s.mrect.y = r[1];
            s.mrect.flags |= WRF_POS | WRF_ABS;
        }
        None => {
            if let Some(w) = ui.window_rect(0) {
                if w.flags & WRF_POS != 0 {
                    s.mrect.x = w.x;
                    s.mrect.y = w.y;
                    s.mrect.flags |= WRF_POS | WRF_ABS;
                }
            }
        }
    }
    let (w, h) = ui.client_size();
    if w > 0 && h > 0 {
        s.mrect.w = w;
        s.mrect.h = h;
        s.mvalid = true;
    }
}

/// st_timer(id) -> handled: TMR_SET_MAX (maximize after the normal rect was placed),
/// TMR_SET_SCREENS (windows outside every monitor), TMR_SET_SAVE (--settings-save-at)
pub(crate) fn st_timer(p: &mut Pn, ui: &mut Ui, id: u32) -> bool {
    match id {
        TMR_SET_SCREENS => st_check_screens(p, ui),
        TMR_SET_SAVE => {
            st_save(p, ui);
        }
        TMR_SET_PLACE => {
            let r = p.st.mrect;
            ui.window_place(0, &r); // (the library resizes the framebuffer, U3)
            if p.st.mmax {
                ui.timer(TMR_SET_MAX, 250); // maximize once the normal rect is applied
            } else {
                p.st.maxwait = false;
            }
        }
        TMR_SET_MAX => {
            p.st.maxwait = false;
            let mut r = p.st.mrect;
            r.flags |= WRF_MAX;
            ui.window_place(0, &r);
        }
        _ => return false,
    }
    true
}

// st_screen_ok(pw) -> 1 platform window pw is on a monitor (at least NEED_X x NEED_Y px, or
// its whole size when smaller, inside the work area of the nearest monitor), 0 outside, -1
// unknown; with the client rect and the work area
fn st_screen_ok(pw: i32) -> (i32, [i32; 4], [i32; 4]) {
    let Some(r) = plat::win_get_rect(pw) else { return (-1, [0; 4], [0; 4]) };
    let Some(a) = plat::win_work_area(pw) else { return (-1, r, [0; 4]) };
    if a[2] <= 0 || a[3] <= 0 {
        return (-1, r, a);
    }
    // x: min(right) - max(left) >= min(w, NEED_X)
    let ox = (r[0] + r[2]).min(a[0] + a[2]) - r[0].max(a[0]);
    if ox < r[2].min(NEED_X) {
        return (0, r, a);
    }
    let oy = (r[1] + r[3]).min(a[1] + a[3]) - r[1].max(a[1]);
    if oy < r[3].min(NEED_Y) {
        return (0, r, a);
    }
    (1, r, a)
}

/// st_check_screens() -- a window outside every monitor goes to its default place: the main
/// window centred on the nearest monitor's work area, tool windows to their default place
/// (only with a window; moved counts the moves, probe)
pub(crate) fn st_check_screens(p: &mut Pn, ui: &mut Ui) {
    if !cfg!(windows) || ui.headless() {
        return; // X11 / Wayland: the window manager does it
    }
    let (ok, r, a) = st_screen_ok(0);
    if ok == 0 {
        p.st.moved += 1;
        let x = ((a[2] - r[2]) >> 1).max(0) + a[0];
        let y = ((a[3] - r[3]) >> 1).max(0) + a[1];
        // 0 = keep the size
        ui.window_place(0, &WinRect::new(x, y, 0, 0, WRF_POS | WRF_ABS));
    }
    for k in 0..WIN_N {
        let id = WINID_TOOLS + k as u32;
        if ui.window_state(id) & UWS_VISIBLE == 0 {
            continue;
        }
        let (i, pw) = ui.window_pw(id);
        if i <= 0 || pw < 0 {
            continue;
        }
        if st_screen_ok(pw).0 == 0 {
            p.st.moved += 1;
            win_reset_pos(p, k);
            ui.repass();
        }
    }
}

// ------------------------------------------------------------------ saving

// st_pal_text() -> "AARRGGBB,..." of the current palette
fn st_pal_text(p: &Pn) -> String {
    let mut t = String::new();
    let n = crate::workspace::palette::pal_count(&p.pal).max(0) as usize;
    for i in 0..n.min(PAL_N) {
        if t.len() + 9 >= TXT_CAP {
            break;
        }
        t.push_str(&format!("{:08X},", crate::workspace::palette::pal_entry(&p.pal, i as i32)));
    }
    t.pop();
    t
}

// st_windows_save() -- the four tool windows
fn st_windows_save(p: &mut Pn) {
    for (k, &(sec, sizeable)) in WIN_SEC.iter().enumerate() {
        let r = win_rect(p, k).get();
        let vis = chrome_wnd_open(&p.chrome, k as i32).get();
        let d = *dock_rec(&mut p.dock, k as i32);
        let more = crate::workspace::panels::colors::wcol_more(&p.wcol) as i32;
        let s = &mut p.st;
        st_seti(s, sec, "visible", vis);
        // placement (wnd_dock.rs)
        st_seti(s, sec, "dock", d.mode);
        if d.mode == DM_ANCHOR {
            st_seti(s, sec, "ax", d.ax);
            st_seti(s, sec, "dx", d.dx);
            st_seti(s, sec, "ay", d.ay);
            st_seti(s, sec, "dy", d.dy);
        } else {
            for key in ["ax", "dx", "ay", "dy"] {
                st_del(s, sec, key);
            }
        }
        if d.mode == DM_FREE && r.flags & WRF_POS != 0 {
            st_seti(s, sec, "x", r.x);
            st_seti(s, sec, "y", r.y);
        } else {
            st_del(s, sec, "x");
            st_del(s, sec, "y");
        }
        if sizeable {
            st_seti(s, sec, "width", r.w);
            st_seti(s, sec, "height", r.h);
        }
        if k == WND_COLORS as usize {
            st_seti(s, sec, "expanded", more);
        }
    }
}

// st_collect() -- every value into the INI
fn st_collect(p: &mut Pn, ui: &mut Ui) {
    let s = &mut p.st;
    st_seti(s, SEC_PN, "version", SET_VERSION);
    let b = &p.custom_brush;
    st_sets(s, "BrushTip", "file", &b.path);
    for (key,value) in [("angle",b.dynamics.angle),("scatter",b.dynamics.scatter),("size_jitter",b.dynamics.size_jitter),("angle_jitter",b.dynamics.angle_jitter),("flow",b.dynamics.flow),("follow",b.dynamics.follow)] { st_seti(s,"BrushTip",key,value); }
    st_seti(s, "BrushTip", "accumulate", b.dynamics.accumulate as i32);
    st_seti(s, "BrushTip", "flow_byte", b.dynamics.flow_byte.map_or(-1, |v| v as i32));
    st_seti(s, "BrushTip", "opacity_cap", b.dynamics.opacity_cap as i32);
    st_seti(s, "BrushTip", "density", b.dynamics.density as i32);
    // ---- UI
    st_setenum(s, SEC_UI, "language", &["de", "en", "fr"], s.st.language);
    st_setenum(s, SEC_UI, "theme", &THEME_NAMES, s.st.theme);
    let acc = if s.st.accent == 0 { String::new() } else { format!("{:06X}", s.st.accent) };
    st_sets(s, SEC_UI, "accent", &acc);
    st_seti(s, SEC_UI, "animations", s.st.anim as i32);
    st_seti(s, SEC_UI, "peninput", s.st.pen as i32); // (PN1)
    // ---- main window (headless runs keep the file's values)
    if !ui.headless() && s.mvalid {
        let m = s.mrect;
        if m.flags & WRF_POS != 0 {
            st_seti(s, SEC_MAIN, "x", m.x);
            st_seti(s, SEC_MAIN, "y", m.y);
        } else {
            st_del(s, SEC_MAIN, "x");
            st_del(s, SEC_MAIN, "y");
        }
        st_seti(s, SEC_MAIN, "width", m.w);
        st_seti(s, SEC_MAIN, "height", m.h);
        st_seti(s, SEC_MAIN, "maximized", s.mmax as i32);
    }
    st_windows_save(p);
    // ---- colours
    let prim = format!("{:08X}", crate::workspace::palette::pal_primary(&p.pal));
    let sec = format!("{:08X}", crate::workspace::palette::pal_secondary(&p.pal));
    let pal = st_pal_text(p);
    let (units, grid, rulers) = (crate::workspace::view::view_units(&p.view), crate::workspace::view::view_grid(&p.view), crate::workspace::view::view_rulers(&p.view));
    let s = &mut p.st;
    st_sets(s, SEC_COL, "primary", &prim);
    st_sets(s, SEC_COL, "secondary", &sec);
    st_sets(s, SEC_COL, "palette", &pal);
    // ---- Ansicht
    st_setenum(s, SEC_VIEW, "units", &UNIT_NAMES, units);
    st_seti(s, SEC_VIEW, "grid", grid as i32);
    st_seti(s, SEC_VIEW, "rulers", rulers as i32);
    for (key,value) in [("workspace",s.st.workspace),("checker0",s.st.checker0),("checker1",s.st.checker1)] {
        st_sets(s,SEC_VIEW,key,&format!("{value:08X}"));
    }
    st_seti(s,SEC_VIEW,"shadow",s.st.canvas_shadow as i32);
    st_seti(s,"Updates","automatic",s.st.update_auto as i32);
    st_seti(s,"Updates","beta",s.st.update_beta as i32);
    // ---- tool defaults
    st_setenum(s, SEC_TOOLS, "default", &TOOL_NAMES, s.st.deftool);
    let w = s.st.tooldef.width;
    if let Some(ini) = &mut s.ini {
        ini.set_f64(SEC_TOOLS, "width", w);
    }
    for (k, &(key, _, _)) in TC_TAB.iter().enumerate() {
        let v = *tc_field(&mut s.st.tooldef, k) as i32;
        st_seti(s, SEC_TOOLS, key, v);
    }
    // ---- recent files file1..file10 (missing ones removed)
    for i in 0..RECENT_MAX {
        let key = format!("file{}", i + 1);
        match crate::integrations::file_io::fio_recent_path(&p.fio, i).map(String::from) {
            Some(path) => st_sets(&mut p.st, SEC_RECENT, &key, &path),
            None => st_del(&mut p.st, SEC_RECENT, &key),
        }
    }
    let s = &mut p.st;
    st_seti(s, SEC_HIST, "budget_mb", s.st.budget_mb as i32);
    let td = s.st.tempdir.clone();
    st_sets(s, SEC_HIST, "temp_dir", &td);
    st_seti(s, SEC_PERF, "threads", s.st.threads as i32);
    // ---- effects (fx.rs)
    if let Some(ini) = &mut p.st.ini {
        crate::effects::fx_settings_save(&p.fx, ini);
    }
}

/// st_save() -> 0 written, 1 no persistence, -1 write error
pub(crate) fn st_save(p: &mut Pn, ui: &mut Ui) -> i32 {
    if !p.st.on || p.st.ini.is_none() {
        return 1;
    }
    st_collect(p, ui);
    let s = &mut p.st;
    let ok = s.ini.as_ref().is_some_and(|ini| ini.save(&s.file));
    s.err = if ok { 0 } else { -1 };
    if ok {
        s.st.saves += 1;
    }
    s.err
}

/// set_save_quit() -- app/lifecycle.rs: Beenden / closing the main window
pub(crate) fn set_save_quit(p: &mut Pn, ui: &mut Ui) {
    st_save(p, ui);
}

// ------------------------------------------------------------------ factory state / reload

/// st_factory() -- colours, palette, view options, tool defaults + tool bar + tool, tool
/// windows, recent files, history budget / folder, threads, UI values (not applied)
pub(crate) fn st_factory(p: &mut Pn, ui: &mut Ui) {
    p.custom_brush.dynamics = Default::default(); p.custom_brush.path.clear();
    crate::workspace::palette::pal_reset(&mut p.pal);
    crate::workspace::palette::pal_default(&mut p.pal);
    crate::workspace::view::view_set_opts(p, UNIT_PX as u32, false, false);
    st_tools_reset(p);
    st_tools_apply(p);
    if crate::tools::tool_cur(&p.tool) != TOOL_DEFAULT {
        crate::tools::tool_set(p, ui, TOOL_DEFAULT);
    }
    crate::workspace::panels::tools::wtools_init(p, ui);
    crate::workspace::panels::history::whist_init(p, ui);
    crate::workspace::panels::layers::wlay_init(p, ui);
    crate::workspace::panels::colors::wcol_init(p, ui);
    for k in 0..WND_COUNT {
        dock_reset(&mut p.dock, k); // placement: the automatic layout
    }
    crate::workspace::panels::colors::wcol_set_more(p, false); // pn-ui
    for k in 0..WIN_N {
        chrome_wnd_open(&p.chrome, k as i32).set(1);
    }
    crate::integrations::file_io::fio_cmd(p, ui, CMD_RECENT_CLEAR, 0);
    let s = &mut p.st.st;
    s.language = 0;
    s.budget_mb = 0;
    s.threads = 0;
    s.accent = 0;
    s.loaded = 0;
    s.tempdir.clear();
    s.theme = THEME_SYSTEM as u32;
    s.anim = 1;
    s.pen = 1; // (PN1)
    s.workspace = 0;
    s.update_auto = 0;
    s.update_beta = 0;
    s.checker0 = 0xFFFFFFFF;
    s.checker1 = 0xFFCCCCCC;
    s.canvas_shadow = 1;
    crate::preferences::localization::apply(p,ui,0);
    plat::pen_enable(true);
    st_budget_apply(p);
    st_tempdir_apply(p);
    ui.repass();
}

/// st_reload() -- factory state, then the file is read again and applied (early + late)
pub(crate) fn st_reload(p: &mut Pn, ui: &mut Ui) {
    st_factory(p, ui);
    if !p.st.on {
        return;
    }
    p.st.ini = Some(Ini::load(&p.st.file));
    st_load_early(p, ui);
    st_apply_late(p, ui);
    crate::preferences::localization::init(p,ui);
}

/// st_tools_from_bar() -- "Aktuelle Werkzeugleiste übernehmen": defaults = current tool bar
pub(crate) fn st_tools_from_bar(p: &mut Pn) {
    p.st.st.tooldef = p.tool.cfg;
    p.repass();
}

/// st_tools_reset() -- "Zurücksetzen": defaults = factory (Farbpinsel, tool_init's options)
pub(crate) fn st_tools_reset(p: &mut Pn) {
    p.st.st.tooldef = p.st.fcfg;
    p.st.st.deftool = TOOL_DEFAULT as u32;
    p.repass();
}

/// st_tc_text(&TOOLCFG) -> "w=2 h=75 s=15 aa=1 b=0 f=0 t=50 fl=0 smp=0 sel=0 q=0 p=0 sm=0"
/// (probes; width with '.' as decimal point)
pub(crate) fn st_tc_text(tc: &ToolCfg) -> String {
    let w = base::num::f64_format(tc.width);
    format!(
        "w={} h={} s={} aa={} b={} f={} t={} fl={} smp={} sel={} q={} p={} sm={}",
        w, tc.hard as i32, tc.spacing as i32, tc.aa as i32, tc.blend as i32, tc.fill as i32, tc.tol as i32,
        tc.flood as i32, tc.sample as i32, tc.selmode as i32, tc.selqual as i32, tc.pressure as i32, tc.smooth as i32
    )
}
