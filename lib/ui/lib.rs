// ui -- the immediate-mode runtime: one Ui context instead of ASMLIB's g_* globals  [src/ui]
//
//   run(desc, app, style) -> exit code   command line, headless (--shot / --script) or window
//   App: init(ui) / frame(ui) builds the UI every pass / command(ui, id, arg) -> handled
//   Style: how widgets look (style.rs, the contract of ASMLIB/inc/style.inc); the app
//   passes the style crate it was linked with (build_rs.py --style)
//   main!(expr): the app's entry point (jfgui_main, called by plat's process entry)
//
// Modules (one per ASMLIB file): ctx (ui_ctx.s: input, hit testing, behaviour, slots,
// timers, commands, semantics, clip, layers, passes), layout (ui_layout.s), cmd (ui_cmd.s:
// draw command lists and their replay), dirty (ui_dirty.s), theme (ui_theme.s), basic
// (ui_basic.s), auto (ui_auto.s: scripts and the widget dump), runtime (ui_main.s), cursor
// (ui_cursor.s), drop (ui_drop.s), dnd (drag and drop between widgets and programs), dock
// (docking: panels at the sides in tab groups, floating tool windows), kbnav (keyboard
// navigation between widgets, panes and windows), clipimg
// (ui_clipimg.s), style (style.inc); the widget families popup, edit, list, value, color, win,
// chrome, filedlg (R5 agents); rtl (the right-to-left mirror of the recorded commands and the
// input); ids (widget ids from names), settings_page (the page of a base::settings schema),
// winstate (window placement kept between runs, AF_SAVE_STATE); fontpick (system fonts: loading
// a choice, the font picker and dialog); tasks (async tasks on the UI thread: ui.spawn, run
// between passes; plat::task).
//
// State: ASMLIB keeps every window's UI state in per-module blocks (`*_wb0` .. `*_wb1`)
// that ui_win.s swaps when it builds another window.  Here that state is `WinParts` (the
// live window's part of every module: `ui.wp.ctx`, `ui.wp.lay`, `ui.wp.cmd`, `ui.wp.dirty`,
// `ui.wp.rt`, `ui.wp.popup`, ...); win.rs swaps it with a window's saved WinParts
// (core::mem::swap).  State shared by all windows lives in `Ui` itself (core) and in
// `ui.m` (the widget families' shared state).
//
// For the widget families (R5): the asm globals map to fields -- g_mx / g_my / g_focus /
// g_hot / g_active / g_last_rect / g_keyq ... are `self.wp.ctx.*`, g_boxes / g_next_* are
// `self.wp.lay.*`, g_dcol / the command lists `self.wp.cmd.*`, g_fb / g_ui_dpi
// `self.wp.rt.fb` / `.dpi`; g_frame / g_now_ms / g_repass / g_mods / g_sems / slots / timers /
// g_uiw_cur are fields of Ui.  A family's per-window state goes into its `W` (in WinParts),
// its shared state into its `S` (in Mods).  Style calls: `let st = self.style; let mut sp =
// Sp { .. }; let (w, h) = st.xxx_size(self, &mut sp); self.place(w, h); let ev =
// self.behave(id, hb); self.sp_placed(&mut sp, ev); st.xxx(self, &sp);
// self.focus_ring_draw(ev); self.sem(K_*, id, label, value)` (basic.rs shows every pattern).
// The core calls into the families through these functions (all ported, R5): win.rs uiw_*,
// chrome.rs titlebar_begin / titlebar_end / edge_grips (ui-win), popup.rs accel_dispatch /
// tooltip / popup_count (ui-popup), edit.rs text_key_filter, clip_set / clip_get /
// hclip_text_* / open_folder / open_url (ui-edit).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod consts;
pub mod keys;
mod a11y;
mod auto;
mod auto_perm;
mod agent;
// the app's file types at start-up (--register / --unregister)
mod assoc;
mod basic;
mod chart;
mod chrome;
mod clipimg;
mod cmd;
mod color;
mod ctx;
mod cursor;
mod data;
mod datepick;
mod diagram; // node / edge canvas (diagram.rs)
pub use diagram::{
    dg_edge_points, dg_hairline, dg_length, dg_paint_grid, dg_point_at, dg_stroke_edge, DEdge, DEv, DHit, DModel, DNode, DPort, DgFrame, DgGeo,
    DgPaint, DgRoute, DiagramState, Viewport, DG_CONNECT, DG_PAN, DG_PLACE, DG_SELECT, DN_LOCKED, DN_NOSELECT, DPT_HIDDEN,
    DPT_USED, DR_BEZIER, DR_ORTHO, DR_STRAIGHT,
};
mod dirty;
mod dnd;
// (ROADMAP §6) the editors with a text model of their own: the shared core, rich text, code
mod ed_core;
pub use ed_core::{TextPos, TextSel};
mod rich_doc;
pub use rich_doc::{
    list_marks, Fmt, Para, RichDoc, RA_CENTER, RA_JUSTIFY, RA_LEFT, RA_RIGHT, LEVEL_MAX, LS_BULLET, LS_NONE, LS_NUMBER, PS_H1, PS_H2,
    PS_H3, PS_NORMAL, PS_QUOTE, RF_ALL, RF_BOLD, RF_ITALIC, RF_STRIKE, RF_UNDER,
};
mod rich_edit;
pub use rich_edit::{
    RTB_ALIGN, RTB_ALL, RTB_CHARS, RTB_CLEAR, RTB_COLOR, RTB_HISTORY, RTB_LINK, RTB_LISTS, RTB_STYLE, RT_NO_FRAME, RT_OPEN_LINKS, RT_PLAIN_PASTE,
    RT_READONLY, RT_TAB,
};
mod rich_html;
// (ROADMAP §6) the Markdown / HTML viewer: its document (md_doc.rs) and the view (md_view.rs)
mod md_doc;
pub use md_doc::{
    MdBlock, MdDoc, MdImage, MdLinkSpot, MdSpan, MdTable, MA_CENTER, MA_LEFT, MA_NONE, MA_RIGHT, MB_CODE, MB_HEADING, MB_PARA, MB_ROW,
    MB_RULE, MK_BULLET, MK_NONE, MK_NUMBER, MS_BOLD, MS_CODE, MS_ITALIC, MS_STRIKE, MS_UNDER,
};
mod md_view;
pub use md_view::{MdEv, MdViewState, MV_LTR, MV_NO_FRAME, MV_OPEN_LINKS, MV_RTL};
// (ROADMAP §6) lists whose rows have different heights (ui.vlist_begin)
mod vlist;
pub use vlist::VListState;
mod code_doc;
pub use code_doc::{CodeDoc, LineChange, FIND_CASE, FIND_MAX, FIND_WORD};
pub use code_doc::{Diag, DIAG_ERROR, DIAG_INFO, DIAG_WARNING}; // (L6) a code editor's diagnostics
mod highlight;
pub use highlight::{
    CodeTheme, Grammar, Highlighter, Rule, Token, INI, JSON, MARKDOWN, PLAIN, RUST, SY_CODE, SY_COMMENT, SY_CONST, SY_COUNT, SY_EMPH, SY_ESCAPE,
    SY_FUNC, SY_HEADING, SY_KEY, SY_KEYWORD, SY_LINK, SY_META, SY_NUMBER, SY_PUNCT, SY_STRING, SY_STRONG, SY_TEXT, SY_TYPE, TOML,
};
pub use highlight::JFS; // (L6) the grammar of lib/script's .jfs
mod code_edit;
mod code_long;
pub use code_edit::{CodeStatus, CE_NO_FIND, CE_NO_FRAME, CE_NO_GUTTER, CE_NO_LINE, CE_READONLY};
pub use code_edit::CE_MARKS; // (L6) the gutter's column of diagnostic markers
pub use code_long::code_long_check; // (tests) what the code editor keeps of long lines
mod dock;
mod drop;
mod edit;
mod filedlg;
mod fontpick;
pub use fontpick::{font_language, FontSel};
mod grid;
mod hexview;
pub use hexview::{HexEv, HexMark, HexState};
// (ROADMAP §6) SVG pictures (ui.svg) and the image viewer (ui.image_view)
mod imgview;
pub use imgview::{ImageViewState, SvgFonts, SvgText, ViewImage, IV_NO_BARS, IV_NO_CHECKER, IV_NO_GRID, IV_SMOOTH, IV_TEXT_COLOR, IV_WHEEL_SCROLL};
pub use img_svg::Svg;
mod history;
pub use history::{Edit, History, Snapshot};
// hyphenation of wrapped texts and text boxes (TX_HYPHENATE, TB_HYPHENATE)
mod hyph;
pub mod i18n;
mod ids;
// the live inspector: overlay and tool window (--inspect, F12 / Ctrl+Shift+I)
mod inspect;
// keyboard navigation: focus groups, panes and windows (F6), splitters (F8), the menu key
mod kbnav;
pub use ids::{ID_HASH_BASE, wid, wid_path};
mod layout;
mod list;
mod notify;
mod office_gallery;
mod perm;
mod popup;
// the app's recent files and the system's (ui.recent_add / recent_files)
mod recent;
mod ribbon;
mod settings_page;
mod shell;
mod slider;
// the system's sounds: ui.beep, the sound of a message dialog, the app's switch (sound.rs)
mod sound;
mod rtl;
mod runtime;
// the monitors, global shortcuts, the colour pick, pictures of the screen (plat::screen, hotkey)
mod screen;
mod stock;
mod style;
mod symbol;
mod taskbar;
mod tasks;
mod term;
pub use term::{TermEv, TermState, TF_CTRL_SHIFT6, TF_NO_FRAME, TF_NO_INPUT, TF_QHELP, TK_KEY, TM_BUSY, TM_LINE, TM_RAW, TM_SECRET};
pub mod theme;
mod timeline;
pub use timeline::{TimelineState, TlTrack};
mod tray;
mod value;
mod vicon;
mod win;
mod winstate;

use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;

use gfx::Dc;
use text::Text;

pub use clipimg::{clip_bmp_from_sf, clipimg_parse, clipimg_record};
pub use cmd::{Cmd, Draw, DrawRef, Gfx, IconHook};
pub use consts::*;
pub use ctx::{Sem, Slot, ease};
pub use cursor::CursorDraw;
pub use edit::TextboxPresentation;
pub use edit::TextSurfaceLine;
pub use keys::*;
pub use layout::{dp_at, px2dip_at};
pub use perm::boot;
pub use assoc::{assoc_switch, desk_boot};
pub use recent::RECENT_KEEP;
// the library's version (lib/VERSION, docs/VERSIONING.md) and the app's own (apps/NAME/PACKAGE)
pub use rt::{app_version, VERSION, VERSION_MAJOR, VERSION_MINOR, VERSION_PATCH};
// drag and drop (dnd.rs): what a drag carries, its kinds and effects
pub use plat::{DE_ALL, DE_COPY, DE_LINK, DE_MOVE, DT_FILES, DT_HTML, DT_IMAGE, DT_STD, DT_TEXT, DT_URL, DT_USER0, DragData};
pub use popup::{Command, Mi}; // ui-popup: the command table and menu records
pub use ribbon::{RibbonGroup, RibbonGroupMode, RibbonLayout, RibbonMetrics, RibbonSample};
pub use runtime::{Runtime, launch, run};
pub use style::{Sp, Style, StyleBasic, StyleChrome, StyleData, StyleEdit, StyleList, StyleMisc, StylePopup, StyleShell, StyleValue};
pub use data::{GridCol, GridEv, GridState, TreeEv, TreeRow, TreeState};
pub use chart::{BarSeries, DonutSlice};
// (L5) line charts of time series (chart.rs): the ring of points, what a chart shows, the axis maths
pub use chart::{
    lc_nice, lc_time_step, lc_time_text, LcMarker, LcSeries, LcThreshold, LcUnit, LineChart, SeriesBuf, SeriesCol, LC_NO_GRID, LC_NO_HOVER,
    LC_NO_LEGEND, LX_NANOS, LX_NUM, LX_SECS,
};
pub use shell::{CmdItem, NavState};
// docking (dock.rs): the panels, the layout, the sides and flags
pub use dock::{
    DockPanel, DockState, DOCK_BOTTOM, DOCK_CENTER, DOCK_FLOAT, DOCK_FLOAT_MAX, DOCK_LEFT, DOCK_MK_BOTTOM, DOCK_MK_CENTER, DOCK_MK_LEFT,
    DOCK_MK_RIGHT, DOCK_MK_TOP, DOCK_RIGHT, DP_HIDDEN, DP_NOCLOSE, DP_NOFLOAT,
};
// the taskbar button's progress and jump list tasks (ui.taskbar_progress / taskbar_tasks)
pub use plat::taskbar::{JumpTask, TaskbarProgress};
pub use plat::hotkey::{Chord, HotkeyError};
pub use plat::screen::{Monitor, ScreenError};
pub use stock::{card_opaque, kind_colors, Stock};
pub use vicon::{paint_svg_icon, paint_svg_icon2};
pub use symbol::{SymPalette, Symbol, SymbolCache, SYM_BODY, SYM_CACHE_MAX_W, SYM_CRIT, SYM_HI, SYM_INK, SYM_OK, SYM_PORT, SYM_ROLES,
    SYM_SCREEN, SYM_SHADE, SYM_TINT, SYM_WARN}; // multi-colour vector symbols (tools/vsym.py)
pub use theme::color_mix;
pub use theme::{contrast_ratio, luminance};
pub use win::Shared; // ui-win: the app's open flags / WINRECTs the library keeps
// ui-list: the value / colour / list API beside the Ui methods
pub use color::{hsv_to_rgb, rgb_to_hsv};
pub use value::{cvtd, deg_atan2, deg_sincos, grad_at, part_id, px_mix, RangeF, Track, NO_DEFAULT};

/// the app's entry point: `ui::main!(ui::run(&DESC, &mut MyApp::default(), &style::STYLE));`
/// (expands to the `jfgui_main` symbol the platform entry calls; the app crate denies
/// unsafe code, this one item is the exception).  It first sets up what build_rs.py built in
/// (perm.rs): the app's name for its folders (JFGUI_APP), its assets as app://
/// (apps/NAME/assets, packed to JFGUI_ASSETS with --cfg jfgui_assets) and its permissions
/// (apps/NAME/PERMISSIONS as JFGUI_PERMISSIONS), then its file types and URL schemes
/// (apps/NAME/PACKAGE as JFGUI_PACKAGE: the switches --register / --register-replace /
/// --unregister, assoc.rs).
#[macro_export]
macro_rules! main {
    ($body:expr) => {
        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        fn jfgui_main() -> i32 {
            #[cfg(jfgui_assets)]
            static ASSETS: &[u8] = include_bytes!(env!("JFGUI_ASSETS"));
            #[cfg(not(jfgui_assets))]
            static ASSETS: &[u8] = &[];
            if let Err(code) = $crate::boot(option_env!("JFGUI_APP"), option_env!("JFGUI_PERMISSIONS"), ASSETS) {
                return code;
            }
            if let Some(code) = $crate::desk_boot(option_env!("JFGUI_PACKAGE")) {
                return code;
            }
            $body
        }
    };
}

/// the app descriptor (APP_DESC): title, AF_* flags, sizes in DIP, accent 0xRRGGBB.  Write
/// `AppDesc { title: "..", w: .., ..AppDesc::NONE }` so later fields keep their defaults.
#[derive(Clone, Copy)]
pub struct AppDesc {
    pub title: &'static str,
    pub flags: u32,
    pub w: u16,
    pub h: u16,
    pub min_w: u16,
    pub min_h: u16,
    pub accent: u32,
    /// (ui-popup) the command table (AD_CMDS: ids, labels, icons, accelerators, CF_* flags)
    pub cmds: &'static [Command],
    /// (ui-popup) the menu bar of menubar(None) (AD_MENUBAR: Mi::Menu records)
    pub menubar: &'static [Mi],
}

impl AppDesc {
    /// every field empty / zero
    pub const NONE: AppDesc =
        AppDesc { title: "", flags: 0, w: 0, h: 0, min_w: 0, min_h: 0, accent: 0, cmds: &[], menubar: &[] };
}

/// the app's callbacks (APP_DESC on_init, on_frame, on_command)
pub trait App {
    /// Optional domain actions, in addition to the universal UI action catalogue.
    fn actions(&self) -> &'static [agent_core::ActionSpec] { &[] }
    /// Domain revision; None uses the UI runtime's monotonically increasing frame revision.
    fn action_revision(&self) -> Option<u64> { None }
    /// Dynamic domain constraints such as read-only files or an absent selection.
    fn action_available(&self, _spec: &agent_core::ActionSpec) -> Result<(), agent_core::ActionError> { Ok(()) }
    /// Domain action handler; adapters should call the same editing functions as the GUI.
    fn action(&mut self, _ui: &mut Ui, _spec: &agent_core::ActionSpec, _input: &agent_core::Value,
        _context: &agent_core::ActionContext) -> Result<agent_core::ActionResult, agent_core::ActionError> {
        Err(agent_core::ActionError::new(agent_core::ErrorCode::NotFound, "No domain action with this name"))
    }
    /// Filtered domain state; the default universal inspection includes the widget tree.
    fn action_inspect(&self, _query: &agent_core::Value) -> Result<agent_core::Value, agent_core::ActionError> {
        Ok(agent_core::Value::object([]))
    }
    /// Describe an application-owned binary resource returned by one of its actions.
    fn action_resource_info(&self, _uri: &str) -> Result<agent_core::Resource, agent_core::ActionError> {
        Err(agent_core::ActionError::new(agent_core::ErrorCode::NotFound, "Resource not found"))
    }
    /// Read a bounded byte range of an application-owned resource, never an arbitrary path.
    fn action_resource_read(&self, _uri: &str, _offset: u64, _limit: usize) -> Result<Vec<u8>, agent_core::ActionError> {
        Err(agent_core::ActionError::new(agent_core::ErrorCode::NotFound, "Resource not found"))
    }
    /// called before the first frame; false = initialisation failed (exit code 2)
    fn init(&mut self, _ui: &mut Ui) -> bool {
        true
    }
    /// builds the UI (every pass; no side effects except through widgets)
    fn frame(&mut self, ui: &mut Ui);
    /// commands queued with ui.cmd_run and the built-in UIC_* ids; true = handled
    fn command(&mut self, _ui: &mut Ui, _id: u32, _arg: u32) -> bool {
        false
    }
}

/// the per-window state of the live window: the core's per-window blocks (ui_ctx_wb0,
/// ui_layout_wb0, ui_cmd_wb0, ui_dirty_wb0, ui_main_wb0) and the families' (ui_popup,
/// ui_list, ui_chrome).  win.rs keeps one of these per window and swaps it with `Ui::wp`
/// when another window is built (core::mem::swap); a new window starts from `WinParts::new`.
pub struct WinParts {
    pub(crate) ctx: ctx::W,
    pub(crate) lay: layout::W,
    pub(crate) cmd: cmd::W,
    pub(crate) dirty: dirty::W,
    pub(crate) rt: runtime::W,
    #[allow(dead_code)] // R5: filled by the widget-family ports
    pub(crate) popup: popup::W,
    #[allow(dead_code)]
    pub(crate) list: list::W,
    #[allow(dead_code)]
    pub(crate) chrome: chrome::W,
    /// right-to-left: the regions that keep their LTR content (rtl.rs)
    pub(crate) rtl: rtl::W,
}

impl WinParts {
    /// the state of a window before its first pass (the main window: ui_init)
    pub(crate) fn new() -> WinParts {
        WinParts {
            ctx: ctx::W::new(),
            lay: layout::W::default(),
            cmd: cmd::W::new(&cmd::CAPS_MAIN),
            dirty: dirty::W::default(),
            rt: runtime::W::new(),
            popup: Default::default(),
            list: Default::default(),
            chrome: Default::default(),
            rtl: Default::default(),
        }
    }
}

/// the shared state of the widget-family modules
#[derive(Default)]
#[allow(dead_code)] // R5: filled by the widget-family ports
pub(crate) struct Mods {
    pub(crate) popup: popup::S,
    pub(crate) edit: edit::S,
    pub(crate) list: list::S,
    pub(crate) value: value::S,
    pub(crate) color: color::S,
    pub(crate) win: win::S,
    pub(crate) chrome: chrome::S,
    pub(crate) filedlg: filedlg::S,
    pub(crate) chart: chart::S,
    pub(crate) shell: shell::S,
    pub(crate) notify: notify::S,
    pub(crate) a11y: a11y::S,
    pub(crate) dock: dock::S,
    pub(crate) kbnav: kbnav::S,
    pub(crate) fonts: fontpick::S,
    pub(crate) term: term::S,
    pub(crate) perm: perm::S,
    /// (ROADMAP §6) the rich-text editors' views (rich_edit.rs)
    pub(crate) rich: rich_edit::S,
    /// (ROADMAP §6) the code editors' views, theme and font size (code_edit.rs)
    pub(crate) code: code_edit::S,
    /// (ROADMAP §12) the live inspector (inspect.rs)
    pub(crate) ins: inspect::S,
    /// (ROADMAP §6) ui.svg's draw objects, the faces of SVG text (imgview.rs)
    pub(crate) svg: imgview::S,
    /// (ROADMAP §4) hyphenation: languages, dictionaries (hyph.rs)
    pub(crate) hyph: hyph::S,
    /// (ROADMAP §6) the open lists of ui.vlist_begin (vlist.rs)
    pub(crate) vlist: vlist::S,
    /// (ROADMAP §3) the app's sound switch, the last sound asked for (sound.rs)
    pub(crate) sound: sound::S,
    /// (ROADMAP §3) the colour pick, the last picture, dropped shortcuts (screen.rs)
    pub(crate) screen: screen::S,
}

/// the UI context: everything ASMLIB keeps in g_* variables
pub struct Ui {
    pub(crate) agent: agent::State,
    pub desc: AppDesc,
    pub style: &'static dyn Style,
    /// fonts, glyph cache, text runs (shared: the asm's font module)
    pub text: Text,
    /// the gfx draw context (the asm's g_dc; render time only)
    pub dc: Dc,
    /// the live window's per-window state (see WinParts)
    pub(crate) wp: WinParts,
    /// the widget families' shared state
    pub(crate) m: Mods,

    // ---- ctx: shared by all windows (ui_ctx.s after ui_ctx_wb1)
    pub(crate) sems: Vec<Sem>,
    pub(crate) semtext_len: usize,
    pub(crate) sem_dummy: Sem,
    pub(crate) slots: BTreeMap<u32, Slot>,
    pub(crate) keydown: [u8; 256],
    pub(crate) mpts: Vec<[i32; 2]>,
    pub(crate) mprs: Vec<u32>,
    pub(crate) mtilt: [i32; 2],
    pub(crate) mpress: u32,
    pub(crate) mptype: u32,
    pub(crate) mpts_win: u32,
    /// commands for App::command in the order queued (ctx.rs cmd_run, CMDQ_MAX)
    pub(crate) cmdq: VecDeque<(u32, u32)>,
    /// typed text that waits for the next passes (beyond a pass's TEXTQ_MAX, or left by a
    /// text_take cap): (window, codepoints) in typing order (ctx.rs text_refill)
    pub(crate) text_more: Vec<(u32, Vec<u32>)>,
    /// (id, deadline ms) in the order set (ctx.rs timer, TIMERS_MAX)
    pub(crate) timers: Vec<(u32, u32)>,
    /// what each ceiling held back (ctx.rs limit_hit, LIM_*)
    pub(crate) limits: [u32; ctx::LIM_N],
    pub(crate) mods: u32,
    pub(crate) frame: u32,
    pub(crate) now_ms: u32,
    pub(crate) repass: bool,
    pub(crate) animating: bool,
    pub(crate) anim_off: bool,
    /// the windows are mirrored this frame (rtl.rs); rtl_mode 0 = from the UI language,
    /// 1 = on, 2 = off
    pub(crate) rtl: bool,
    pub(crate) rtl_mode: u8,
    /// lifecycle: the windows that have the keyboard (bit per window), UIC_ACTIVE's state
    pub(crate) app_focus: u32,
    pub(crate) app_active: bool,
    /// the theme follows the system (theme_set(2))
    pub(crate) theme_sys: bool,
    /// system settings (theme.rs): the light / dark choice before high contrast, high
    /// contrast (mode 0 system, 1 on, 2 off) and its colours, the accent follows the system,
    /// --anim / --text-scale given, the text scale (%), double-click and caret blink times
    pub(crate) theme_base_dark: bool,
    pub(crate) contrast_mode: u8,
    pub(crate) high_contrast: bool,
    pub(crate) hc_sys: Option<plat::HcColors>,
    pub(crate) accent_sys: bool,
    pub(crate) anim_forced: bool,
    /// the system's reduced motion as last applied (set_reduced_motion(None) returns to it)
    pub(crate) anim_sys: bool,
    pub(crate) text_scale_forced: bool,
    pub(crate) text_scale: u32,
    pub(crate) dbl_ms: u32,
    /// the system's font rendering: smoothing, hinting, subpixel order (SysSettings)
    pub(crate) font_prefs: (Option<bool>, Option<u8>, Option<u8>),
    pub(crate) blink_ms: u32,
    pub(crate) warns: u32,
    pub(crate) ui_debug: bool,
    /// scripts and jfgui_DEBUG: warn about an id two widgets of one pass share
    pub(crate) dup_check: bool,
    pub(crate) dup_warned: alloc::collections::BTreeSet<u32>,
    /// widget ids from names: the scope stack (ids.rs)
    pub(crate) ids: ids::Ids,
    pub(crate) focus_req: u32,
    pub(crate) focus_req_win: u32,
    pub(crate) focus_req_old: u32,
    pub(crate) focus_req_frame: u32,
    /// window being built / live (g_uiw_cur of ui_win.s; 0 = the main window)
    pub(crate) uiw_cur: u32,

    // ---- theme (ui_theme.s)
    pub(crate) theme: [u32; theme::THEME_N],
    pub(crate) theme_dark: bool,
    pub(crate) accent: u32,

    // ---- runtime (ui_main.s, shared part)
    pub(crate) title: String,
    pub(crate) t0_us: u64,
    pub(crate) headless: bool,
    pub(crate) need: bool,
    pub(crate) need_render: bool,
    pub(crate) quit: bool,
    pub(crate) log_events: bool,
    /// AF_SINGLE_INSTANCE: the requests open_take has not taken yet
    pub(crate) open_reqs: Vec<plat::instance::Request>,
    pub(crate) autoclose_ms: u32,
    pub(crate) accent_raw: u32,
    pub(crate) settle_warned: bool,
    pub(crate) verify_dirty: bool,
    pub(crate) frames_rendered: u32,
    pub(crate) close_at: u32,

    // ---- dirty regions (ui_dirty.s, shared part)
    pub(crate) drects: Vec<[i32; 4]>,
    pub(crate) dmg: Vec<dirty::Dmg>,
    pub(crate) cvs: Vec<dirty::Cvs>,

    // ---- commands (ui_cmd.s, shared part)
    pub(crate) icon_hook: Option<IconHook>,

    // ---- basic widgets (ui_basic.s)
    pub(crate) uiinput: UiInput,

    // ---- scripts (ui_auto.s), cursors (ui_cursor.s), drops and the headless image
    // clipboard (ui_drop.s)
    pub(crate) sc: auto::S,
    pub(crate) ucur: cursor::S,
    pub(crate) dr: drop::S,
    /// drag and drop: the app's drags, the drag over a window, drops onto targets (dnd.rs)
    pub(crate) dnd: dnd::S,
    /// async tasks on the UI thread (tasks.rs: ui.spawn; run between passes)
    pub(crate) tasks: plat::task::Executor,
}
