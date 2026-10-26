//! Interactive canvas tools and their shared brush/shape engines.
pub(crate) mod brush;
pub(crate) mod brushes;
pub(crate) mod bucket;
pub(crate) mod clone;
pub(crate) mod gradient;
pub(crate) mod move_pixels;
pub(crate) mod picker;
pub(crate) mod select;
pub(crate) mod shapes;
pub(crate) mod shapes_library;
pub(crate) mod text;
pub(crate) mod view;
pub(crate) mod wand;

// tool -- PaintCraft: the tool framework (agent T1).  Contract: inc/pn_tooli.inc, details
// docs/TOOLS.md.
//                                              [apps/PaintCraft/src/tool.s]
//
// Module contract (main.rs / chrome.rs / view.rs):
//   tool_init()  tool_states()  tool_frame()  tool_cmd(id, arg) -> bool
//   tool_cur() -> TOOL_*   tool_options()   tool_hint() -> "Name: Hilfetext"
//   tool_input(doc, inp, x_fx, y_fx) -> bool   (view.rs: canvas input)
//   tool_cursor() -> CUR_*                     (view.rs: cursor over the canvas)
//   tool_precmd(id, arg) -> bool consumed      (app/lifecycle.rs: before a command id is routed)
// Registry / switching: tool_register, tool_desc, tool_set, tool_prev
// Options: tool_cfg (p.tool.cfg), tool_cfg_changed, opt_width, opt_hardness, opt_spacing,
//   opt_fill, opt_aa, opt_blend, opt_tolerance, opt_flood, opt_sampling, opt_selmode,
//   opt_selqual, opt_smooth, opt_pressure, opt_sep, opt_label, opt_menu
// Pending edits: tool_live_begin, tool_live_end, tool_live, tool_commit_live,
//   tool_cancel_live; helpers tool_color, tool_pointer, tool_damage, tool_sync_pending
// Tool layer: tl_init, tl_set_paint, tl_row, tl_show, tl_hide, tl_mark, tl_set_box,
//   tl_commit, tl_hook, fill_pattern_row
// Test hooks (diag.rs block T1): t1_diag_cmd (930..949), t1_diag_frame (records 9930..9949)
//
// Tool keys: CMD_TOOLKEY_S .. CMD_TOOLKEY_O select the first tool of their group or, when a
// tool of the group is active, the next one (S: Rechteck, Lasso, Ellipse, Zauberstab;
// M: Pixel verschieben, Auswahl verschieben; O: Linie / Kurve, Formen).
//
// Rust model: the asm's TL records are `p.tool.tls[TL_*]`.  TL_MASK / TL_SRC are `TlMask` /
// `TlSrc`: a mask or surface owned by the record (`Own`), the brush engine's stroke mask
// (`TlMask::Be`) or a layer of the target document found by its uid (`TlSrc::Layer`).
// tl_hook / tl_commit resolve them into a `TlView` (plain references) for tl_row, which runs
// on the worker threads.  The commit takes the target layer's surface out of the document
// while the workers write it (so tl_row can still read the document's selection).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use gfx::blendmode::{bm_cov_row, bm_cov_row_inplace, bm_solid_row, bm_solid_row_inplace, BM_COUNT, BM_NORMAL, BM_OPAC_SHIFT, BM_OVERWRITE};
use gfx::straight::erase_row;
use gfx::tmask::{Span, TMask};
use gfx::Surface;
use rt::fx::{cvtd, cvttd, maxsd, minsd};
use rt::sync::SpinLock;
use ui::*;

use crate::document::composite::{comp_mark, comp_mark_all, comp_set_tool, CompHook, HookKey};
use crate::app::constants::*;
use crate::document::model::{doc_active, doc_index, doc_mark_px, Doc, DocId};
use crate::document::history::{hist_count, hist_pos, hist_px_begin, hist_px_cancel, hist_px_end, hist_px_touch};
use crate::document::layers::lay_find_uid;
use crate::workspace::palette::{pal_primary, pal_reset, pal_secondary, pal_set_primary, pal_set_secondary, pal_swap};
use crate::document::selection::sel_clip_row;
use crate::workspace::view::{view_canvas_rect, view_cursor, view_invalidate, view_overlay_add, View};
use crate::app::Pn;

const KEYGRP_N: u32 = 14; // CMD_TOOLKEY_S .. CMD_TOOLKEY_O
const HINT_CAP: usize = 512; // status bar hint buffer
const OL_PAD: i32 = 4; // px around the brush outline (damage)
const HATCH_N: u32 = FS_COUNT as u32 - 1; // 53 hatch styles
const BLEND_N: u32 = 15; // rows of the blend mode menu
const FILL_COLS: u32 = 3; // Fülleffekt flyout: names in three columns
const FILL_COL_W: i32 = 272; //   of this width (DIP)
const C_RSCALE: f64 = 0.0128; // 256 / 20000: width * zoom -> radius 24.8 px
const RF_WIDTH: RangeF = RangeF::new(1.0, 2000.0, 1.0, 0, 0);
const RANGE_PCT: Range = Range::new(0, 100, 1, 0);
const RANGE_SPC: Range = Range::new(1, 100, 1, 0);

/// a tool's callbacks (TOOLDESC; None = nothing / default).  UI thread.
pub(crate) struct ToolDesc {
    /// TD_FLAGS: TF_*
    pub(crate) flags: u32,
    /// TD_ICON: IC_* (0 = icon of the command CMD_TOOL0 + tool)
    pub(crate) icon: u32,
    /// TD_NAME: German name ("" = label of CMD_TOOL0 + tool)
    pub(crate) name: &'static str,
    /// TD_HINT: German status bar help text ("" = paint.net default)
    pub(crate) hint: &'static str,
    /// TD_ACTIVATE fn(doc): the tool became current (doc may be None)
    pub(crate) activate: Option<fn(&mut Pn, &mut Ui, Option<DocId>)>,
    /// TD_DEACTIVATE fn(doc): it stops being current (pending edit committed already)
    pub(crate) deactivate: Option<fn(&mut Pn, &mut Ui, Option<DocId>)>,
    /// TD_INPUT fn(doc, ui, x_fx, y_fx) -> 1 changed: canvas pointer input (once per pass)
    pub(crate) input: Option<fn(&mut Pn, &mut Ui, DocId, &UiInput, i32, i32) -> bool>,
    /// TD_OPTIONS fn(): option widgets (opt_* helpers)
    pub(crate) options: Option<fn(&mut Pn, &mut Ui)>,
    /// TD_KEY fn(vk, mods) -> 1 handled
    pub(crate) key: Option<fn(&mut Pn, &mut Ui, u32, u32) -> bool>,
    /// TD_COMMIT fn(doc) -> 1 step
    pub(crate) commit: Option<fn(&mut Pn, &mut Ui, DocId) -> bool>,
    /// TD_CANCEL fn(doc): None = the document is gone, free private state only
    pub(crate) cancel: Option<fn(&mut Pn, &mut Ui, Option<DocId>)>,
    /// TD_CURSOR fn(x_fx, y_fx) -> CUR_* or PNC_BASE + kind
    pub(crate) cursor: Option<fn(&mut Pn, i32, i32) -> u32>,
    /// TD_DOCSWITCH fn(old, new)
    pub(crate) docswitch: Option<fn(&mut Pn, &mut Ui, Option<DocId>, Option<DocId>)>,
    /// TD_OVERLAY fn(doc, &canvas_rect): render time, after the image (absolute window px)
    pub(crate) overlay: Option<fn(&mut Pn, &mut Gfx, DocId, [i32; 4])>,
    /// TD_TICK fn(doc): start of every frame (tool_states)
    pub(crate) tick: Option<fn(&mut Pn, &mut Ui, Option<DocId>)>,
}

impl ToolDesc {
    /// a descriptor without callbacks (fill the fields with `..ToolDesc::NONE`)
    pub(crate) const NONE: ToolDesc = ToolDesc {
        flags: 0,
        icon: 0,
        name: "",
        hint: "",
        activate: None,
        deactivate: None,
        input: None,
        options: None,
        key: None,
        commit: None,
        cancel: None,
        cursor: None,
        docswitch: None,
        overlay: None,
        tick: None,
    };
}

/// the shared tool options (TOOLCFG, tool_cfg())
#[derive(Clone, Copy, Default)]
pub(crate) struct ToolCfg {
    /// TC_WIDTH: brush width in image px, 1 .. 2000 (fractions allowed)
    pub(crate) width: f64,
    /// TC_HARD: hardness 0 .. 100 %
    pub(crate) hard: u32,
    /// TC_SPACING: dab spacing 1 .. 100 % of the width
    pub(crate) spacing: u32,
    /// TC_AA: 1 = antialiasing on
    pub(crate) aa: u32,
    /// TC_BLEND: BM_* (BM_OVERWRITE = Überschreiben)
    pub(crate) blend: u32,
    /// TC_FILL: FS_* fill style
    pub(crate) fill: u32,
    /// TC_TOL: tolerance 0 .. 100 %
    pub(crate) tol: u32,
    /// TC_FLOOD: 0 zusammenhängend / FF_GLOBAL global
    pub(crate) flood: u32,
    /// TC_SAMPLE: SMP_LAYER / SMP_IMAGE
    pub(crate) sample: u32,
    /// TC_SELMODE: SM_*
    pub(crate) selmode: u32,
    /// TC_SELQUAL: SQ_*
    pub(crate) selqual: u32,
    /// TC_PRESSURE: 1 = pen pressure scales the width
    pub(crate) pressure: u32,
    /// TC_VER: bumped on every change of an option
    pub(crate) ver: u32,
    /// TC_SMOOTH: 1 = Pfadglättung
    pub(crate) smooth: u32,
}

impl ToolCfg {
    /// paint.net 5 defaults (g_tcfg in .data)
    pub(crate) const DEFAULTS: ToolCfg = ToolCfg {
        width: 2.0,
        hard: 75,
        spacing: 15,
        aa: 1,
        blend: BM_NORMAL,
        fill: FS_SOLID as u32,
        tol: 50,
        flood: 0,
        sample: SMP_LAYER as u32,
        selmode: SM_REPLACE as u32,
        selqual: SQ_AA as u32,
        pressure: 1,
        ver: 0,
        smooth: 0, // Ungeglätteter Weg
    };

    // the TOOLCFG fields of the hooks / records by asm offset (t1_fields, t1_cfgf)
    fn field(&mut self, off: u32) -> Option<&mut u32> {
        Some(match off {
            8 => &mut self.hard,
            12 => &mut self.spacing,
            16 => &mut self.aa,
            20 => &mut self.blend,
            24 => &mut self.fill,
            28 => &mut self.tol,
            32 => &mut self.flood,
            36 => &mut self.sample,
            40 => &mut self.selmode,
            44 => &mut self.selqual,
            48 => &mut self.pressure,
            52 => &mut self.ver,
            56 => &mut self.smooth,
            _ => return None,
        })
    }
}

/// tool layer slots (the asm's TL records g_br_tl, g_bk_tl, ...; p.tool.tls[TL_*])
pub(crate) const TL_BRUSH: usize = 0;
pub(crate) const TL_BUCKET: usize = 1;
pub(crate) const TL_CLONE: usize = 2;
pub(crate) const TL_GRADIENT: usize = 3;
pub(crate) const TL_SHAPES: usize = 4;
pub(crate) const TL_TEXT: usize = 5;
pub(crate) const TL_T1: usize = 6;
pub(crate) const TL_COUNT: usize = 7;

/// TL_MASK: the coverage of a tool layer
#[derive(Default)]
pub(crate) enum TlMask {
    /// 255 everywhere inside the box
    #[default]
    None,
    /// a mask owned by the record
    Own(TMask),
    /// the brush engine's stroke mask (p.be: be_mask)
    Be,
    /// the bucket's shown mask p.tbucket.mf[i] (0 region, 1 softened region)
    Bucket(usize),
    /// the gradient's transparency mask (p.tgrad.mask)
    Grad,
}

/// TL_SRC: the straight source surface of TLK_SOURCE
#[derive(Default)]
pub(crate) enum TlSrc {
    #[default]
    None,
    /// a surface owned by the record
    Own(Surface),
    /// the target document's layer with this uid (LY_UID)
    Layer(u64),
    /// the gradient's colour surface (p.tgrad.src)
    Grad,
    /// the clone / recolor tools' private surface (p.tclone.sf)
    Clone,
}

/// a tool layer (TL): the pending pixels of the current layer as a row function.  It must stay
/// Sync (tl_row runs on worker threads through the compose hook).
#[derive(Default)]
pub(crate) struct Tl {
    /// TL_KIND: TLK_*
    pub(crate) kind: u32,
    /// TL_MODE: BM_* | opacity << 8
    pub(crate) mode: u32,
    /// TL_MASK: the coverage
    pub(crate) mask: TlMask,
    /// TL_ARGB / TL_ARGB2: colour (pattern foreground) / pattern background
    pub(crate) argb: u32,
    pub(crate) argb2: u32,
    /// TL_FILL: FS_*
    pub(crate) fill: u32,
    /// TL_STRENGTH: TLK_ERASE 0 .. 255
    pub(crate) strength: u32,
    /// TL_DOC: the target document (its selection clips)
    pub(crate) doc: Option<DocId>,
    /// TL_X0 .. TL_Y1: box of the affected pixels (x1 / y1 exclusive)
    pub(crate) bx: [i32; 4],
    /// TL_SRC: the source (TLK_SOURCE)
    pub(crate) src: TlSrc,
    /// TL_SX / TL_SY: image position of the source's pixel (0, 0) (TLK_SOURCE)
    pub(crate) sx: i32,
    pub(crate) sy: i32,
    /// TL_FLAGS: TLF_*
    pub(crate) flags: u32,
    /// TL_SHOWN: installed as the compose hook
    pub(crate) shown: bool,
}

/// a tool layer with its mask and source resolved (what tl_row reads; worker threads)
pub(crate) struct TlView<'a> {
    pub(crate) tl: &'a Tl,
    pub(crate) mask: Option<&'a TMask>,
    pub(crate) src: Option<&'a Surface>,
    /// the test clip rectangle of hooks 941 / 942 (instead of sel_clip_row)
    pub(crate) tclip: Option<[i32; 4]>,
}

/// the background jobs `sync` waits for (the UI's sync hook has no context): the bucket's
/// fill (t_bucket.rs), the effects (fx.rs), the save options preview (dlg_saveopt.rs)
pub(crate) static SYNC_BK: AtomicBool = AtomicBool::new(false);
pub(crate) static SYNC_FX: AtomicBool = AtomicBool::new(false);
pub(crate) static SYNC_SAVEOPT: AtomicBool = AtomicBool::new(false);

/// the tool framework (tool.s): current tool, descriptors, options, tool layers
pub(crate) struct Tool {
    /// tool_cfg(): the shared options
    pub(crate) cfg: ToolCfg,
    /// the tool layers (TL records), TL_* slots
    pub(crate) tls: [Tl; TL_COUNT],
    /// TOOLDESC per tool (tool_register; never cleared)
    tools: [Option<&'static ToolDesc>; TOOL_COUNT as usize],
    /// active image of the last frame
    last_doc: Option<DocId>,
    /// image with a pending edit
    live_doc: Option<DocId>,
    cur: i32,
    prev: i32,
    /// (PN1) tool before the pen's eraser end + 1 (0 = none)
    pen_tool: i32,
    /// (PN1) a pen is known (Druckempfindlichkeit shown) / plat::pen_devices was asked
    pen_seen: bool,
    pen_probed: bool,
    /// inside TD_COMMIT / TD_CANCEL
    in_live: bool,
    /// last tool_input: image position 24.8, screen position (px)
    ptr_fx: i32,
    ptr_fy: i32,
    ptr_sx: i32,
    ptr_sy: i32,
    ptr_ok: bool,
    ptr_hover: bool,
    /// brush outline as drawn: visible, x, y, r
    ol_vis: bool,
    ol_x: i32,
    ol_y: i32,
    ol_r: i32,
    /// test clip rectangle (hooks 941 / 942)
    tclip_on: bool,
    tclip: [i32; 4],
}

impl Default for Tool {
    fn default() -> Tool {
        Tool {
            cfg: ToolCfg::DEFAULTS,
            tls: Default::default(),
            tools: [None; TOOL_COUNT as usize],
            last_doc: None,
            live_doc: None,
            cur: 0,
            prev: 0,
            pen_tool: 0,
            pen_seen: false,
            pen_probed: false,
            in_live: false,
            ptr_fx: 0,
            ptr_fy: 0,
            ptr_sx: 0,
            ptr_sy: 0,
            ptr_ok: false,
            ptr_hover: false,
            ol_vis: false,
            ol_x: 0,
            ol_y: 0,
            ol_r: 0,
            tclip_on: false,
            tclip: [0; 4],
        }
    }
}

// ================================================================== registry

/// tool_register(tool, desc) -- desc = TOOLDESC (None removes it)
pub(crate) fn tool_register(t: &mut Tool, tool: i32, desc: Option<&'static ToolDesc>) {
    if (tool as u32) < TOOL_COUNT as u32 {
        t.tools[tool as usize] = desc;
    }
}

/// tool_desc(tool) -> TOOLDESC | None
pub(crate) fn tool_desc(t: &Tool, tool: i32) -> Option<&'static ToolDesc> {
    if (tool as u32) < TOOL_COUNT as u32 { t.tools[tool as usize] } else { None }
}

// cur_desc: the TOOLDESC of the current tool
fn cur_desc(t: &Tool) -> Option<&'static ToolDesc> {
    tool_desc(t, t.cur)
}

/// flags of unregistered tools (width tools show "Pinselgröße:")
static DEF_FLAGS: [u8; TOOL_COUNT as usize] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 4, 0, 0, 4, 4, 0, 4, 4];

// cur_flags() -> TD_FLAGS of the current tool (defaults for unregistered tools)
fn cur_flags(t: &Tool) -> u32 {
    if t.cur as u32 >= TOOL_COUNT as u32 {
        return 0;
    }
    match t.tools[t.cur as usize] {
        Some(d) => d.flags,
        None => DEF_FLAGS[t.cur as usize] as u32,
    }
}

/// tool_cur() -> current tool
pub(crate) fn tool_cur(t: &Tool) -> i32 {
    t.cur
}

/// tool_prev() -> the tool before the current one
pub(crate) fn tool_prev(t: &Tool) -> i32 {
    t.prev
}

/// tool_cfg() -> &TOOLCFG (read the fields directly: p.tool.cfg)
pub(crate) fn tool_cfg(t: &Tool) -> &ToolCfg {
    &t.cfg
}

/// tool_cfg_changed() -- an option changed: TC_VER++, another pass
pub(crate) fn tool_cfg_changed(p: &mut Pn) {
    p.tool.cfg.ver = p.tool.cfg.ver.wrapping_add(1);
    p.repass();
}

/// tool_color(btn) -> straight ARGB: BTN_RIGHT = secondary colour, else primary
pub(crate) fn tool_color(p: &Pn, btn: u32) -> u32 {
    if btn == BTN_RIGHT { pal_secondary(&p.pal) } else { pal_primary(&p.pal) }
}

/// tool_pointer() -> (x_fx, y_fx, valid) of the last tool_input
pub(crate) fn tool_pointer(t: &Tool) -> (i32, i32, bool) {
    (t.ptr_fx, t.ptr_fy, t.ptr_ok)
}

/// tool_live() -> document with a pending edit | None
pub(crate) fn tool_live(t: &Tool) -> Option<DocId> {
    t.live_doc
}

// ================================================================== module contract

/// tool_init() -- default tool, the T1 tools, the overlay, the script `sync` hook
pub(crate) fn tool_init(p: &mut Pn, ui: &mut Ui) {
    p.tool.cur = TOOL_DEFAULT;
    p.tool.prev = TOOL_DEFAULT;
    crate::tools::brush::t_brush_init(p, ui);
    crate::tools::bucket::t_bucket_init(p, ui);
    crate::tools::picker::t_picker_init(p, ui);
    crate::tools::view::t_view_init(p, ui);
    view_overlay_add(p, tool_ovl, 0);
    ui.sync_hook_set(Some(tool_sync_hook));
    if let Some(f) = cur_desc(&p.tool).and_then(|d| d.activate) {
        let d = doc_active(&p.doc);
        f(p, ui, d);
    }
}

// the UI's `sync` hook: background jobs of the tools, effects and the save options dialog
fn tool_sync_hook() -> bool {
    crate::effects::FX_SYNC_ASK.store(SYNC_FX.load(Ordering::SeqCst), Ordering::SeqCst); // pn-sys: fx_jobs_pending's restart
    SYNC_FX.load(Ordering::SeqCst) || SYNC_SAVEOPT.load(Ordering::SeqCst) || SYNC_BK.load(Ordering::SeqCst) || crate::integrations::gmic::BUSY.load(Ordering::Acquire) || crate::integrations::paint_dot_net::host::BUSY.load(Ordering::Acquire) || crate::integrations::services::BUSY.load(Ordering::Acquire) || crate::integrations::file_io::OPEN_BUSY.load(Ordering::Acquire)
}

/// tool_sync_pending() -> 1 while a tool computes in the background (script `sync`)
pub(crate) fn tool_sync_pending(_p: &mut Pn) -> bool {
    tool_sync_hook()
}

/// tool_states() -- image switches (commit), the current tool's tick, command states
pub(crate) fn tool_states(p: &mut Pn, ui: &mut Ui) {
    let d = doc_active(&p.doc);
    let old = p.tool.last_doc;
    if d != old {
        p.tool.last_doc = d;
        // another image: a pending edit of the old one is committed (paint.net)
        if let Some(o) = old {
            if Some(o) == p.tool.live_doc {
                if doc_index(&p.doc, o) >= 0 {
                    tool_commit_live(p, ui);
                } else {
                    live_finish_cancel(p, ui, None);
                }
            }
        }
        if let Some(f) = cur_desc(&p.tool).and_then(|t| t.docswitch) {
            f(p, ui, old, d);
        }
    }
    if let Some(f) = cur_desc(&p.tool).and_then(|t| t.tick) {
        f(p, ui, d);
    }
    for t in 0..TOOL_COUNT {
        crate::app::commands::pn_cmd_set(ui, CMD_TOOL0 + t as u32, if t == p.tool.cur { CF_CHECKED } else { 0 });
    }
    crate::app::commands::pn_cmd_set(ui, CMD_SWAP_COLORS, 0);
    crate::app::commands::pn_cmd_set(ui, CMD_RESET_COLORS, 0);
    let f = if p.tool.live_doc.is_some() { 0 } else { CF_DISABLED };
    crate::app::commands::pn_cmd_set(ui, CMD_TOOL_COMMIT, f);
    crate::app::commands::pn_cmd_set(ui, CMD_TOOL_CANCEL, f);
}

/// tool_frame() -- keys for the tool (Enter commits a pending edit), brush outline state
pub(crate) fn tool_frame(p: &mut Pn, ui: &mut Ui) {
    let k = ui.key_first();
    if k != 0 {
        let vk = k & 0xFFFF;
        let mods = k >> 16;
        if ui.key_peek(vk) {
            let mut to_tool = true;
            if p.tool.live_doc.is_some() {
                let m = mods & (MOD_CTRL | MOD_ALT | MOD_SHIFT);
                if vk == VK_RETURN {
                    // Enter: Fertigstellen (T5: TF_TEXTKEYS tools get it as a key)
                    if m == 0 && cur_flags(&p.tool) & TF_TEXTKEYS as u32 == 0 {
                        ui.key(VK_RETURN);
                        tool_commit_live(p, ui);
                        to_tool = false;
                    }
                } else if vk == b'Z' as u32 && m == MOD_CTRL {
                    // Strg+Z: drop the pending edit (also when the history has nothing to undo)
                    ui.key(b'Z' as u32);
                    tool_cancel_live(p, ui);
                    ui.repass();
                    to_tool = false;
                }
            }
            if to_tool {
                if let Some(f) = cur_desc(&p.tool).and_then(|t| t.key) {
                    if f(p, ui, vk, mods) {
                        ui.key(vk);
                        ui.repass();
                    }
                }
            }
        }
    }
    p.tool.ptr_hover = view_cursor(&p.view).2;
    outline_update(p, ui);
}

/// tool groups of the tool keys (CMD_TOOLKEY_S + i), 0xFF = unused
static KEYGRP: [[u8; 4]; KEYGRP_N as usize] = [
    [TOOL_RECT_SELECT as u8, TOOL_LASSO as u8, TOOL_ELLIPSE_SELECT as u8, TOOL_MAGIC_WAND as u8], // S
    [TOOL_MOVE_PIXELS as u8, TOOL_MOVE_SEL as u8, 0xFF, 0xFF], // M
    [TOOL_ZOOM as u8, 0xFF, 0xFF, 0xFF],     // Z
    [TOOL_PAN as u8, 0xFF, 0xFF, 0xFF],      // H
    [TOOL_BUCKET as u8, 0xFF, 0xFF, 0xFF],   // F
    [TOOL_GRADIENT as u8, 0xFF, 0xFF, 0xFF], // G
    [TOOL_BRUSH as u8, 0xFF, 0xFF, 0xFF],    // B
    [TOOL_ERASER as u8, 0xFF, 0xFF, 0xFF],   // E
    [TOOL_PENCIL as u8, 0xFF, 0xFF, 0xFF],   // P
    [TOOL_PICKER as u8, 0xFF, 0xFF, 0xFF],   // K
    [TOOL_CLONE as u8, 0xFF, 0xFF, 0xFF],    // L
    [TOOL_RECOLOR as u8, 0xFF, 0xFF, 0xFF],  // R
    [TOOL_TEXT as u8, 0xFF, 0xFF, 0xFF],     // T
    [TOOL_LINE as u8, TOOL_SHAPES as u8, 0xFF, 0xFF], // O
];

/// tool_cmd(id, arg) -> true handled
pub(crate) fn tool_cmd(p: &mut Pn, ui: &mut Ui, id: u32, _arg: u32) -> bool {
    let t = id.wrapping_sub(CMD_TOOL0);
    if t < TOOL_COUNT as u32 {
        tool_set(p, ui, t as i32);
    } else if id.wrapping_sub(CMD_TOOLKEY_S) < KEYGRP_N {
        // tool key: next tool of the group, or its first tool
        let g = &KEYGRP[(id - CMD_TOOLKEY_S) as usize];
        let cur = p.tool.cur;
        let mut nt = g[0] as i32;
        if let Some(i) = g.iter().position(|&x| x as i32 == cur) {
            if i + 1 < 4 && g[i + 1] != 0xFF {
                nt = g[i + 1] as i32;
            }
        }
        tool_set(p, ui, nt);
    } else if id == CMD_SWAP_COLORS {
        pal_swap(&mut p.pal);
    } else if id == CMD_RESET_COLORS {
        pal_reset(&mut p.pal);
    } else if id == CMD_BRUSH_DEC || id == CMD_BRUSH_INC || id == CMD_BRUSH_DEC5 || id == CMD_BRUSH_INC5 {
        // [ ] brush width (Strg: steps of 5)
        let d = match id {
            CMD_BRUSH_DEC => -1,
            CMD_BRUSH_INC => 1,
            CMD_BRUSH_DEC5 => -5,
            _ => 5,
        };
        width_step(p, ui, d);
    } else if id == CMD_TOOL_COMMIT {
        tool_commit_live(p, ui);
    } else if id == CMD_TOOL_CANCEL {
        tool_cancel_live(p, ui);
    } else if id == UIC_ESCAPE {
        if p.tool.live_doc.is_none() {
            return false;
        }
        tool_cancel_live(p, ui);
    } else {
        if id == UIC_WAKE {
            crate::tools::bucket::bk_poll(p, ui); // background fill finished?
        }
        return false; // (UIC_WAKE reaches every module anyway)
    }
    ui.repass();
    true
}

// width_step(delta): TC_WIDTH += delta, clamped to 1 .. 2000
fn width_step(p: &mut Pn, ui: &mut Ui, d: i32) {
    let w = d as f64 + p.tool.cfg.width;
    p.tool.cfg.width = minsd(maxsd(w, 1.0), 2000.0);
    tool_cfg_changed(p);
    outline_update(p, ui);
}

/// tool_set(tool) -- select a tool: the pending edit is committed first
pub(crate) fn tool_set(p: &mut Pn, ui: &mut Ui, tool: i32) {
    if tool as u32 >= TOOL_COUNT as u32 || tool == p.tool.cur {
        return;
    }
    tool_commit_live(p, ui);
    let d = doc_active(&p.doc);
    if let Some(f) = cur_desc(&p.tool).and_then(|t| t.deactivate) {
        f(p, ui, d);
    }
    p.tool.prev = p.tool.cur;
    p.tool.cur = tool;
    if let Some(f) = cur_desc(&p.tool).and_then(|t| t.activate) {
        f(p, ui, d);
    }
    outline_update(p, ui);
    view_invalidate(p); // cursor and overlays of the new tool
    ui.repass();
}

// ================================================================== pending edits

/// tool_live_begin(doc) -- an edit is pending on doc
pub(crate) fn tool_live_begin(p: &mut Pn, doc: DocId) {
    p.tool.live_doc = Some(doc);
    if let Some(d) = p.doc.get_mut(doc) {
        d.flags |= DF_LIVE as u32;
    }
    p.repass();
}

/// tool_live_end() -- no edit pending any more
pub(crate) fn tool_live_end(p: &mut Pn) {
    if let Some(doc) = p.tool.live_doc.take() {
        if let Some(d) = p.doc.get_mut(doc) {
            d.flags &= !(DF_LIVE as u32);
        }
        p.repass();
    }
}

/// tool_commit_live() -> 1 when a history step was made
pub(crate) fn tool_commit_live(p: &mut Pn, ui: &mut Ui) -> bool {
    let Some(d) = p.tool.live_doc else { return false };
    if p.tool.in_live {
        return false;
    }
    p.tool.in_live = true;
    let mut r = false;
    if let Some(f) = cur_desc(&p.tool).and_then(|t| t.commit) {
        r = f(p, ui, d);
    }
    p.tool.in_live = false;
    tool_live_end(p);
    r
}

/// tool_cancel_live() -- drop the pending edit (no history)
pub(crate) fn tool_cancel_live(p: &mut Pn, ui: &mut Ui) {
    if let Some(d) = p.tool.live_doc {
        live_finish_cancel(p, ui, Some(d));
    }
}

// live_finish_cancel(doc) -- TD_CANCEL(doc) of the current tool, then no edit pending
fn live_finish_cancel(p: &mut Pn, ui: &mut Ui, d: Option<DocId>) {
    if p.tool.in_live {
        return;
    }
    p.tool.in_live = true;
    if let Some(f) = cur_desc(&p.tool).and_then(|t| t.cancel) {
        f(p, ui, d);
    }
    p.tool.in_live = false;
    if d.is_none() {
        p.tool.live_doc = None; // the document is gone: no flags to clear
    }
    tool_live_end(p);
    view_invalidate(p);
}

/// tool_precmd(id, arg) -> 1 consumed -- main.rs calls it before routing a command: with a
/// pending edit Strg+Z cancels it (consumed); view / tool / window / help / palette /
/// settings commands and test hooks pass; every other command commits the edit first
pub(crate) fn tool_precmd(p: &mut Pn, ui: &mut Ui, id: u32, _arg: u32) -> bool {
    if p.tool.live_doc.is_none() {
        return false;
    }
    if id == CMD_UNDO {
        tool_cancel_live(p, ui);
        ui.repass();
        return true;
    }
    let pass = id.wrapping_sub(CMD_TOOL_FIRST) <= CMD_TOOL_LAST - CMD_TOOL_FIRST
        || id.wrapping_sub(CMD_VIEW_FIRST) <= CMD_VIEW_LAST - CMD_VIEW_FIRST
        || id.wrapping_sub(CMD_PAL_FIRST) <= CMD_PAL_LAST - CMD_PAL_FIRST
        || id.wrapping_sub(CMD_WND_TOOLS) <= CMD_WND_LAST - CMD_WND_TOOLS // windows, help, settings, image switching
        || id.wrapping_sub(CMD_TTEXT_FIRST) <= CMD_TEST_LAST - CMD_TTEXT_FIRST // M5 tool hooks, settings dialog, test hooks
        || id >= 0x7FFF_0000; // UIC_* (not routed through here)
    if !pass {
        tool_commit_live(p, ui);
    }
    false
}

// ================================================================== canvas input, cursor

/// tool_input(doc, ui, x_fx, y_fx) -> 1 when something changed (inp = the canvas UIINPUT)
pub(crate) fn tool_input(p: &mut Pn, ui: &mut Ui, doc: DocId, inp: &UiInput, x_fx: i32, y_fx: i32) -> bool {
    let t = &mut p.tool;
    t.ptr_fx = x_fx;
    t.ptr_fy = y_fx;
    t.ptr_sx = inp.x + inp.rect[0];
    t.ptr_sy = inp.y + inp.rect[1];
    t.ptr_ok = true;
    t.ptr_hover = inp.ev & EV_HOVER != 0;
    // (PN1) the eraser end of a pen selects the Radierer, the tip brings the tool used
    // before back (over the canvas, not while a button of an earlier frame is held)
    if !inp.pressed & inp.btn == 0 && inp.ev & EV_HOVER != 0 {
        if inp.ptype.wrapping_sub(PTR_PEN) <= PTR_ERASER - PTR_PEN {
            t.pen_seen = true; // a pen: the tool row shows its switch
        }
        if inp.ptype == PTR_ERASER {
            if t.cur != TOOL_ERASER {
                t.pen_tool = t.cur + 1; // the tool before + 1
                tool_set(p, ui, TOOL_ERASER);
            }
        } else if inp.ptype == PTR_PEN && t.pen_tool != 0 {
            let back = t.pen_tool - 1;
            t.pen_tool = 0;
            if t.cur == TOOL_ERASER {
                tool_set(p, ui, back); // (not when the user chose another tool meanwhile)
            }
        }
    }
    let mut r = false;
    if let Some(f) = cur_desc(&p.tool).and_then(|t| t.input) {
        r = f(p, ui, doc, inp, x_fx, y_fx);
    }
    outline_update(p, ui);
    r
}

/// tool_cursor() -> CUR_* for the canvas (TD_CURSOR, default crosshair): a crosshair or arrow
/// becomes the tool's own cursor, PNC_BASE + kind that PaintCraft cursor (cursors.rs; both as
/// CUR_CUSTOM with ui.cursor_custom); CUR_CROSS when the cursor cannot be made
pub(crate) fn tool_cursor(p: &mut Pn, ui: &mut Ui) -> u32 {
    let mut c = CUR_CROSS;
    if let Some(f) = cur_desc(&p.tool).and_then(|t| t.cursor) {
        let (x, y) = (p.tool.ptr_fx, p.tool.ptr_fy);
        c = f(p, x, y);
    }
    let kind = if c >= PNC_BASE as u32 {
        c - PNC_BASE as u32
    } else if c == CUR_CROSS || c == CUR_ARROW {
        p.tool.cur as u32
    } else {
        return c;
    };
    let h = crate::workspace::cursors::pn_cursor(p, ui, kind);
    if h == 0 {
        return CUR_CROSS;
    }
    ui.cursor_custom(h);
    CUR_CUSTOM
}

/// tool_damage(sx0, sy0, sx1, sy1) -- redraw that screen rectangle (absolute px) of the canvas
pub(crate) fn tool_damage(v: &View, ui: &mut Ui, sx0: i32, sy0: i32, sx1: i32, sy1: i32) {
    if sx0 >= sx1 || sy0 >= sy1 {
        return;
    }
    let r = view_canvas_rect(v);
    ui.canvas_damage(ID_CANVAS, sx0 - r[0], sy0 - r[1], sx1 - sx0, sy1 - sy0);
}

// ol_damage(sx, sy, r_fx): the screen area of an outline circle
fn ol_damage(v: &View, ui: &mut Ui, x: i32, y: i32, r: i32) {
    let r = (r >> 8) + OL_PAD;
    tool_damage(v, ui, x - r, y - r, x + r + 1, y + r + 1);
}

// outline_update(): show / move / hide the brush outline of TF_OUTLINE tools (damages the old
// and the new place)
fn outline_update(p: &mut Pn, ui: &mut Ui) {
    let mut vis = false;
    let mut r = 0;
    if cur_flags(&p.tool) & TF_OUTLINE as u32 != 0 && p.tool.ptr_hover && doc_active(&p.doc).is_some() && !ui.key_down(VK_SPACE) {
        let v = cvttd(crate::workspace::view::view_zoom(p) as i32 as f64 * p.tool.cfg.width * C_RSCALE);
        if v >= 512 {
            // (radius < 2 px: the cursor is enough)
            r = v;
            vis = true;
        }
    }
    let t = &p.tool;
    let (x, y) = (t.ptr_sx, t.ptr_sy);
    if vis == t.ol_vis && (!vis || (x == t.ol_x && y == t.ol_y && r == t.ol_r)) {
        return;
    }
    if t.ol_vis {
        ol_damage(&p.view, ui, t.ol_x, t.ol_y, t.ol_r);
    }
    let t = &mut p.tool;
    t.ol_vis = vis;
    t.ol_x = x;
    t.ol_y = y;
    t.ol_r = r;
    if vis {
        ol_damage(&p.view, ui, x, y, r);
    }
}

// tool_ovl(ctx, doc, &canvas_rect): view overlay: the tool's TD_OVERLAY, the brush outline
fn tool_ovl(p: &mut Pn, g: &mut Gfx, _ctx: usize, doc: DocId, rect: [i32; 4]) {
    if let Some(f) = cur_desc(&p.tool).and_then(|t| t.overlay) {
        f(p, g, doc, rect);
    }
    if p.tool.ol_vis {
        let x = (p.tool.ol_x << 8) + 128;
        let y = (p.tool.ol_y << 8) + 128;
        let r = p.tool.ol_r;
        // dark ring outside, light ring inside
        g.dc.set_color(0xA000_0000);
        g.shapes().stroke_circle_fx(x, y, r + 128, 256);
        g.dc.set_color(0xC0FF_FFFF);
        g.shapes().stroke_circle_fx(x, y, r - 128, 256);
    }
}

// ================================================================== tool row, status bar

/// tool_options() -- the options of the current tool (+ Fertigstellen for TF_LIVE tools)
pub(crate) fn tool_options(p: &mut Pn, ui: &mut Ui) {
    let desc = tool_desc(&p.tool, p.tool.cur);
    match desc.and_then(|t| t.options) {
        Some(f) => f(p, ui),
        None => {
            if cur_flags(&p.tool) & TF_USES_WIDTH as u32 != 0 {
                opt_width(p, ui);
            }
        }
    }
    if desc.is_some_and(|t| t.flags & TF_LIVE as u32 != 0) {
        opt_sep(ui);
        ui.cmd_button(CMD_TOOL_COMMIT, 0);
    }
}

/// tool_hint() -> "Name: Hilfetext" of the current tool
pub(crate) fn tool_hint(p: &Pn) -> String {
    let t = if (p.tool.cur as u32) < TOOL_COUNT as u32 { p.tool.cur } else { 0 };
    let desc = tool_desc(&p.tool, t);
    let name = match desc.map(|d| d.name).filter(|n| !n.is_empty()) {
        Some(n) => String::from(n),
        None => cmd_label(p, CMD_TOOL0 + t as u32),
    };
    let hint = desc.map(|d| d.hint).filter(|h| !h.is_empty()).unwrap_or(HELP_TAB[t as usize]);
    fmt_cap(format!("{}: {}", name, hint), HINT_CAP)
}

// ui_cmd_label(id): the label of a command without the mnemonic marks ("&&" = "&")
fn cmd_label(p: &Pn, id: u32) -> String {
    let l = p.cmds.iter().find(|c| c.id == id).map_or("", |c| c.label);
    let mut s = String::new();
    let mut it = l.chars().peekable();
    while let Some(c) = it.next() {
        if c == '&' {
            if it.peek() == Some(&'&') {
                it.next();
                s.push('&');
            }
            continue;
        }
        s.push(c);
    }
    s
}

/// fmt_buf(buf, cap, ...): the text cut to cap - 1 bytes (never inside a UTF-8 sequence)
pub(crate) fn fmt_cap(s: String, cap: usize) -> String {
    if s.len() < cap {
        return s;
    }
    let mut n = cap - 1;
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    String::from(&s[..n])
}

// ================================================================== option widgets

/// opt_label(str) -- "Label:" in the option row
pub(crate) fn opt_label(ui: &mut Ui, s: &str) {
    ui.text(TS_BODY, crate::preferences::localization::tr(s));
}

/// opt_sep() -- vertical divider between option groups
pub(crate) fn opt_sep(ui: &mut Ui) {
    ui.spacer(4);
    ui.next_size(1, 20);
    ui.box_begin(0, BOX_H | BOX_FIXED, 0);
    let c = ui.color(T_CTRL_STROKE2);
    ui.draw_color(c);
    let r = ui.box_rect();
    ui.draw_rect(r[0], r[1], r[2], r[3]);
    ui.box_end();
    ui.spacer(4);
}

/// opt_width() -> 1 changed -- "Pinselgröße:" [-] [number] [+]
pub(crate) fn opt_width(p: &mut Pn, ui: &mut Ui) -> bool {
    let mut ch = false;
    opt_label(ui, "Pinselgröße:");
    if ui.icon_button(ID_OPT_WDEC, IC_REMOVE, "Pinsel verkleinern") & EV_ACT != 0 {
        width_step(p, ui, -1);
        ch = true;
    }
    ui.next_size(64, 0);
    if ui.number_box_f(ID_OPT_WIDTH, &mut p.tool.cfg.width, &RF_WIDTH) & EV_ACT != 0 {
        tool_cfg_changed(p);
        outline_update(p, ui);
        ch = true;
    }
    if ui.icon_button(ID_OPT_WINC, IC_ADD, "Pinsel vergrößern") & EV_ACT != 0 {
        width_step(p, ui, 1);
        ch = true;
    }
    ch
}

// opt_pct(id, label, &u32, range) -> 1 changed: label, slider, "75%"
fn opt_pct(p: &mut Pn, ui: &mut Ui, id: u32, label: &str, v: &mut u32, rg: &Range) -> bool {
    opt_label(ui, label);
    // The bitmap-tip button shares the brush row; keep the remaining controls
    // visible at the default window width by tightening its two sliders.
    ui.next_size(if matches!(tool_cur(&p.tool),TOOL_BRUSH|TOOL_ERASER) {72} else {90}, 0);
    let mut x = *v as i32;
    let r = ui.slider(id, &mut x, rg);
    *v = x as u32;
    let s = format!("{}%", x);
    ui.next_size(36, 0);
    ui.text(TS_BODY, &s);
    if r & EV_ACT != 0 {
        tool_cfg_changed(p);
        return true;
    }
    false
}

/// opt_hardness() -> 1 changed
pub(crate) fn opt_hardness(p: &mut Pn, ui: &mut Ui) -> bool {
    let mut v = p.tool.cfg.hard;
    let r = opt_pct(p, ui, ID_OPT_HARD, "Härte:", &mut v, &RANGE_PCT);
    p.tool.cfg.hard = v;
    r
}

/// opt_spacing() -> 1 changed
pub(crate) fn opt_spacing(p: &mut Pn, ui: &mut Ui) -> bool {
    let mut v = p.tool.cfg.spacing;
    let r = opt_pct(p, ui, ID_OPT_SPACING, "Abstand:", &mut v, &RANGE_SPC);
    p.tool.cfg.spacing = v;
    r
}

/// opt_tolerance() -> 1 changed
pub(crate) fn opt_tolerance(p: &mut Pn, ui: &mut Ui) -> bool {
    let mut v = p.tool.cfg.tol;
    let r = opt_pct(p, ui, ID_OPT_TOL, "Toleranz:", &mut v, &RANGE_PCT);
    p.tool.cfg.tol = v;
    r
}

/// opt_menu(id, &u32, n, labels, icons, tooltip, flags) -> 1 changed -- flat drop-down with the
/// current entry (icon / label), flyout rows id + 1 + i (current one checked); icons empty =
/// none
pub(crate) fn opt_menu(p: &mut Pn, ui: &mut Ui, id: u32, cur: &mut u32, labels: &[&str], icons: &[u32], tooltip: Option<&str>, flags: u32) -> bool {
    let n = labels.len() as u32;
    if n == 0 {
        return false;
    }
    let c = if *cur <= n - 1 { *cur } else { 0 };
    let mut chosen = c;
    if !icons.is_empty() && flags & OMF_NOICON as u32 == 0 {
        ui.next_icon(icons[if flags & OMF_ICON1 as u32 != 0 { 0 } else { c as usize }]);
    }
    let mut fl = ST_SUBTLE;
    if flags & OMF_ICONONLY as u32 != 0 {
        fl |= ST_ICON_ONLY;
    }
    ui.next_flags(fl);
    let mut label = labels[c as usize];
    if flags & OMF_ICONONLY as u32 != 0 {
        if let Some(t) = tooltip {
            label = t;
        }
    }
    ui.dropdown_button(id, crate::preferences::localization::tr(label));
    if flags & OMF_ICONONLY as u32 == 0 {
        if let Some(t) = tooltip {
            ui.tooltip(crate::preferences::localization::tr(t));
        }
    }
    if ui.flyout_begin(id, FL_BELOW) {
        for i in 0..n {
            if i == c {
                ui.next_flags(ST_CHECKED);
            }
            if !icons.is_empty() && flags & (OMF_ICON1 | OMF_NOICON) as u32 == 0 {
                ui.next_icon(icons[i as usize]);
            }
            if ui.menu_item(id + 1 + i, crate::preferences::localization::tr(labels[i as usize])) & EV_ACT != 0 {
                chosen = i;
            }
        }
    }
    ui.flyout_end();
    if chosen == c {
        return false;
    }
    *cur = chosen;
    tool_cfg_changed(p);
    true
}

static AA_LABELS: [&str; 2] = ["Kantenglättung an", "Kantenglättung aus"];
static AA_ICONS: [u32; 2] = [IC_CIRCLE_FILL, IC_GRID];
static SMOOTH_LABELS: [&str; 2] = ["Geglätteter Weg", "Ungeglätteter Weg"];
static SMOOTH_ICONS: [u32; 2] = [IC_CURVE, IC_LINE];
static PRESSURE_LABELS: [&str; 2] = ["Stiftdruck auswerten", "Stiftdruck ignorieren"]; // (PN1)
static PRESSURE_ICONS: [u32; 2] = [IC_INKING_TOOL, IC_EDIT];
static SELQUAL_LABELS: [&str; 2] = ["Auswahl mit Kantenglättung", "Auswahl ohne Kantenglättung"];
static FLOOD_LABELS: [&str; 2] = ["Zusammenhängend", "Global"];
static FLOOD_ICONS: [u32; 2] = [IC_DROP, IC_GLOBE];
static SAMPLE_LABELS: [&str; 2] = ["Ebene", "Bild"];
static SAMPLE_ICONS: [u32; 2] = [IC_LAYERS, IC_IMAGE];
static SELMODE_LABELS: [&str; 5] = ["Ersetzen", "Vereinigen", "Subtrahieren", "Schneiden", "Exklusiv (XOR)"];
static SELMODE_ICONS: [u32; 5] = [IC_SELECT_RECT, IC_ADD, IC_REMOVE, IC_CROP, IC_SWITCH];
static BLEND_ICON: [u32; 1] = [IC_LAYERS];
/// blend mode menu: row -> BM_* and BM_* -> row (Normal, Überschreiben, then the others)
static BLEND_MODE: [u8; 15] = [BM_NORMAL as u8, BM_OVERWRITE as u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13];
static BLEND_ROW: [u8; 15] = [0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 1];

/// opt_aa() -> 1 changed -- antialiasing on / off (icon drop-down)
pub(crate) fn opt_aa(p: &mut Pn, ui: &mut Ui) -> bool {
    let mut v = (p.tool.cfg.aa == 0) as u32; // 0 = aktiviert, 1 = deaktiviert
    if !opt_menu(p, ui, ID_OPT_AA, &mut v, &AA_LABELS, &AA_ICONS, Some("Antialiasing"), OMF_ICONONLY as u32) {
        return false;
    }
    p.tool.cfg.aa = (v == 0) as u32;
    true
}

/// opt_blend() -> 1 changed -- Füllmethode (Normal, Überschreiben, the 13 other modes)
pub(crate) fn opt_blend(p: &mut Pn, ui: &mut Ui) -> bool {
    let b = if p.tool.cfg.blend < BM_COUNT { p.tool.cfg.blend } else { 0 };
    let mut v = BLEND_ROW[b as usize] as u32;
    if !opt_menu(p, ui, ID_OPT_BLEND, &mut v, &BLEND_LABELS, &BLEND_ICON, Some("Füllmethode"), (OMF_ICONONLY | OMF_ICON1) as u32) {
        return false;
    }
    if v <= BLEND_N - 1 {
        p.tool.cfg.blend = BLEND_MODE[v as usize] as u32;
    }
    true
}

/// opt_flood() -> 1 changed -- "Füllungsmodus:" Zusammenhängend / Global
pub(crate) fn opt_flood(p: &mut Pn, ui: &mut Ui) -> bool {
    opt_label(ui, "Füllungsmodus:");
    let mut v = p.tool.cfg.flood;
    let r = opt_menu(p, ui, ID_OPT_FLOOD, &mut v, &FLOOD_LABELS, &FLOOD_ICONS, None, OMF_ICONONLY as u32);
    p.tool.cfg.flood = v;
    r
}

/// opt_sampling() -> 1 changed -- Ebene / Bild (flood tools)
pub(crate) fn opt_sampling(p: &mut Pn, ui: &mut Ui) -> bool {
    let mut v = p.tool.cfg.sample;
    let r = opt_menu(p, ui, ID_OPT_SAMPLE, &mut v, &SAMPLE_LABELS, &SAMPLE_ICONS, None, OMF_ICONONLY as u32);
    p.tool.cfg.sample = v;
    r
}

/// opt_selmode() -> 1 changed -- "Auswahlmodus:" (selection tools)
pub(crate) fn opt_selmode(p: &mut Pn, ui: &mut Ui) -> bool {
    opt_label(ui, "Auswahlmodus:");
    let mut v = p.tool.cfg.selmode;
    let r = opt_menu(p, ui, ID_OPT_SELMODE, &mut v, &SELMODE_LABELS, &SELMODE_ICONS, None, 0);
    p.tool.cfg.selmode = v;
    r
}

/// opt_smooth() -> 1 changed -- Pfadglättung: Geglätteter / Ungeglätteter Weg (icon)
pub(crate) fn opt_smooth(p: &mut Pn, ui: &mut Ui) -> bool {
    let mut v = (p.tool.cfg.smooth == 0) as u32; // 0 = geglättet
    if !opt_menu(p, ui, ID_OPT_SMOOTH, &mut v, &SMOOTH_LABELS, &SMOOTH_ICONS, Some("Pfadglättung"), OMF_ICONONLY as u32) {
        return false;
    }
    p.tool.cfg.smooth = (v == 0) as u32;
    true
}

/// opt_pressure() -> 1 changed -- (PN1) Stiftdruck auswerten / deaktivieren (icon
/// drop-down, paint.net's ToolConfigStrip.Brush.EnablePressureSensitivity), at the end of the
/// row and only while a pen is known (pen_known): without a pen the row keeps its layout (it
/// fills a 1280 px window exactly)
pub(crate) fn opt_pressure(p: &mut Pn, ui: &mut Ui) -> bool {
    if !pen_known(p, ui) {
        return false;
    }
    let mut v = (p.tool.cfg.pressure == 0) as u32; // 0 = aktiviert
    if !opt_menu(p, ui, ID_OPT_PRESSURE, &mut v, &PRESSURE_LABELS, &PRESSURE_ICONS, Some("Druckempfindlichkeit"), OMF_ICONONLY as u32) {
        return false;
    }
    p.tool.cfg.pressure = (v == 0) as u32;
    true
}

// pen_known() -> (PN1) a pen was used in this session, the system reports one (asked once; not
// in headless runs, so scripts do not depend on the machine) or pressure is switched off (the
// switch must stay reachable)
fn pen_known(p: &mut Pn, ui: &mut Ui) -> bool {
    if p.tool.pen_seen || p.tool.cfg.pressure == 0 {
        return true;
    }
    if p.tool.pen_probed {
        return false;
    }
    p.tool.pen_probed = true;
    if ui.headless() || plat::pen_devices() <= 0 {
        return false;
    }
    p.tool.pen_seen = true;
    true
}

/// opt_selqual() -> 1 changed -- Auswahl mit Kantenglättung / nicht glätten (icon drop-down)
pub(crate) fn opt_selqual(p: &mut Pn, ui: &mut Ui) -> bool {
    let mut v = p.tool.cfg.selqual;
    let r = opt_menu(p, ui, ID_OPT_SELQUAL, &mut v, &SELQUAL_LABELS, &AA_ICONS, Some("Auswahl-Qualität"), OMF_ICONONLY as u32);
    p.tool.cfg.selqual = v;
    r
}

/// opt_fill() -> 1 changed -- "Fülleffekt:" drop-down: Volltonfarbe, then the 53 hatch styles
/// by name in three columns (the current one checked)
pub(crate) fn opt_fill(p: &mut Pn, ui: &mut Ui) -> bool {
    opt_label(ui, "Fülleffekt:");
    let cur = if p.tool.cfg.fill < FS_COUNT as u32 { p.tool.cfg.fill } else { 0 };
    let mut chosen = cur;
    ui.next_flags(ST_SUBTLE);
    ui.dropdown_button(ID_OPT_FILL, FILL_NAMES[cur as usize]);
    if ui.flyout_begin(ID_OPT_FILL, FL_BELOW) {
        if cur == FS_SOLID as u32 {
            ui.next_flags(ST_CHECKED);
        }
        if ui.menu_item(ID_OPT_FILL + 1, FILL_NAMES[0]) & EV_ACT != 0 {
            chosen = FS_SOLID as u32;
        }
        ui.menu_sep();
        let mut h = 0u32; // hatch index
        while h < HATCH_N {
            ui.next_size(-1, 0);
            ui.box_begin(0, BOX_H, 0);
            let mut col = 0;
            while col < FILL_COLS && h < HATCH_N {
                let fs = h + FS_HATCH0 as u32;
                if fs == cur {
                    ui.next_flags(ST_CHECKED);
                }
                ui.next_size(FILL_COL_W, 0);
                if ui.menu_item(ID_OPT_FILL + 2 + h, FILL_NAMES[fs as usize]) & EV_ACT != 0 {
                    chosen = fs;
                }
                h += 1;
                col += 1;
            }
            ui.box_end();
        }
    }
    ui.flyout_end();
    if chosen == cur {
        return false;
    }
    p.tool.cfg.fill = chosen;
    tool_cfg_changed(p);
    true
}

// ================================================================== tool layer

/// tl_init(tl, doc) -- empty tool layer of doc
pub(crate) fn tl_init(p: &mut Pn, tl: usize, doc: DocId) {
    p.tool.tls[tl] = Tl { doc: Some(doc), ..Default::default() };
}

/// tl_set_paint(tl, btn) -- TLK_SOLID with the colour of btn, the fill style (pattern
/// foreground = that colour, background = the other one) and the blend mode (opacity 255)
pub(crate) fn tl_set_paint(p: &mut Pn, tl: usize, btn: u32) {
    let c = tool_color(p, btn);
    let c2 = tool_color(p, if btn == BTN_RIGHT { BTN_LEFT } else { BTN_RIGHT });
    let fill = if p.tool.cfg.fill < FS_COUNT as u32 { p.tool.cfg.fill } else { 0 };
    let mode = if p.tool.cfg.blend < BM_COUNT { p.tool.cfg.blend } else { 0 };
    let t = &mut p.tool.tls[tl];
    t.argb = c;
    t.argb2 = c2;
    t.kind = TLK_SOLID as u32;
    t.fill = fill;
    t.mode = mode | 255 << BM_OPAC_SHIFT;
}

// tl_mark_box(tl): comp_mark of the current box (when not empty)
fn tl_mark_box(p: &mut Pn, tl: usize) {
    let t = &p.tool.tls[tl];
    let b = t.bx;
    if b[0] >= b[2] || b[1] >= b[3] {
        return;
    }
    if let Some(d) = t.doc {
        comp_mark(p, d, b[0], b[1], b[2], b[3]);
    }
}

/// tl_show(tl) -- install tl_row as the compose hook of the document
pub(crate) fn tl_show(p: &mut Pn, tl: usize) {
    if p.tool.tls[tl].shown {
        return;
    }
    let Some(d) = p.tool.tls[tl].doc else { return };
    comp_set_tool(p, d, Some(HookKey::Tl(tl)));
    p.tool.tls[tl].shown = true;
    tl_mark_box(p, tl);
}

/// tl_hide(tl) -- remove the compose hook (the box is recomposited without it)
pub(crate) fn tl_hide(p: &mut Pn, tl: usize) {
    if !p.tool.tls[tl].shown {
        return;
    }
    p.tool.tls[tl].shown = false;
    let Some(d) = p.tool.tls[tl].doc else { return };
    comp_set_tool(p, d, None);
    tl_mark_box(p, tl);
}

/// tl_mark(tl, x0, y0, x1, y1) -- the preview changed in that area: the box grows, the area
/// is recomposited while shown
pub(crate) fn tl_mark(p: &mut Pn, tl: usize, x0: i32, y0: i32, x1: i32, y1: i32) {
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let t = &mut p.tool.tls[tl];
    let b = &mut t.bx;
    if b[0] >= b[2] {
        *b = [x0, y0, x1, y1]; // empty box: take the area
    } else {
        b[0] = b[0].min(x0);
        b[1] = b[1].min(y0);
        b[2] = b[2].max(x1);
        b[3] = b[3].max(y1);
    }
    if !t.shown {
        return;
    }
    if let Some(d) = t.doc {
        comp_mark(p, d, x0, y0, x1, y1);
    }
}

/// tl_set_box(tl, x0, y0, x1, y1) -- replace the box; old and new box are recomposited while
/// shown
pub(crate) fn tl_set_box(p: &mut Pn, tl: usize, x0: i32, y0: i32, x1: i32, y1: i32) {
    if p.tool.tls[tl].shown {
        tl_mark_box(p, tl);
    }
    p.tool.tls[tl].bx = [x0, y0, x1, y1];
    if p.tool.tls[tl].shown {
        tl_mark_box(p, tl);
    }
}

#[inline]
fn int_scale(a: u32, b: u32) -> u32 {
    let t = a * b + 0x80;
    (t + (t >> 8)) >> 8
}

/// tl_row(tl, dst, x, y, n, layer_row) -> 1 written / 0 unchanged -- the tool layer (compose
/// hook and commit; worker threads: pure code only).  n <= 64 (larger values are cut to 64).
/// layer_row None: dst holds the layer row and is changed in place (the commit).
pub(crate) fn tl_row(v: &TlView, doc: &Doc, dst: &mut [u8], x: i32, y: i32, n: i32, layer_row: Option<&[u8]>) -> bool {
    let t = v.tl;
    if n <= 0 {
        return false;
    }
    let n = n.min(64);
    if y < t.bx[1] || y >= t.bx[3] {
        return false;
    }
    let xa = t.bx[0].wrapping_sub(x).max(0); // run offsets inside the box
    let xb = t.bx[2].wrapping_sub(x).min(n);
    if xa >= xb {
        return false;
    }
    let (xa, xb, nu) = (xa as usize, xb as usize, n as usize);
    let mut cov = [0u8; 64];
    match v.mask {
        None => cov[xa..xb].fill(255), // no mask: 255 inside the box
        Some(m) => {
            let mut any = false;
            let mut o = xa;
            while o < xb {
                let (sp, k) = m.span(x + o as i32, y, (xb - o) as i32);
                if k <= 0 {
                    break;
                }
                let k = (k as usize).min(xb - o);
                match sp {
                    Span::Empty => {}
                    Span::Full => {
                        any = true;
                        cov[o..o + k].fill(255);
                    }
                    Span::Bytes(b) => {
                        any = true;
                        cov[o..o + k].copy_from_slice(&b[..k]);
                    }
                }
                o += k;
            }
            if !any {
                return false;
            }
        }
    }
    // the selection clips (or the test rectangle of hook 941 / 942)
    if t.flags & TLF_NOSEL as u32 == 0 && t.doc.is_some() {
        if let Some(c) = v.tclip {
            if y < c[1] || y >= c[3] {
                return false;
            }
            for (i, b) in cov.iter_mut().enumerate().take(xb).skip(xa) {
                let px = x + i as i32;
                if px < c[0] || px >= c[2] {
                    *b = 0;
                }
            }
        } else {
            let mut sel = [0u8; 64];
            match sel_clip_row(doc, x, y, n, &mut sel) as i32 {
                SCR_NONE => return false,
                SCR_PART => {
                    for i in xa..xb {
                        cov[i] = int_scale(cov[i] as u32, sel[i] as u32) as u8;
                    }
                }
                _ => {}
            }
        }
    }
    let dst = &mut dst[..nu * 4];
    let cov = &cov[..nu];
    let mut row = [0u8; 256]; // source / pattern row
    match t.kind as i32 {
        TLK_SOLID => {
            if t.fill == FS_SOLID as u32 {
                match layer_row {
                    Some(l) => bm_solid_row(dst, &l[..nu * 4], t.argb, Some(cov), nu, t.mode),
                    None => bm_solid_row_inplace(dst, t.argb, Some(cov), nu, t.mode),
                }
                return true;
            }
            fill_pattern_row(&mut row, n, x, y, t.fill, t.argb, t.argb2);
            cov_row(dst, layer_row, &row[..nu * 4], cov, nu, t.mode);
            true
        }
        TLK_ERASE => {
            if let Some(l) = layer_row {
                dst.copy_from_slice(&l[..nu * 4]);
            }
            erase_row(dst, cov, t.strength);
            true
        }
        TLK_SOURCE => {
            // the source row; coverage 0 where the source has no pixel
            let Some(s) = v.src else { return false };
            let sy = y.wrapping_sub(t.sy);
            if sy < 0 || sy >= s.h {
                return false;
            }
            // the pixels i0 .. i1 lie inside the source (x + i - sx in 0 .. w)
            let mut cv = [0u8; 64];
            let d0 = t.sx as i64 - x as i64;
            let i0 = d0.clamp(0, n as i64) as usize;
            let i1 = (d0 + s.w as i64).clamp(0, n as i64) as usize;
            if i0 < i1 {
                cv[i0..i1].copy_from_slice(&cov[i0..i1]);
                let o = sy as usize * s.stride as usize + (x as i64 + i0 as i64 - t.sx as i64) as usize * 4;
                row[i0 * 4..i1 * 4].copy_from_slice(&s.px[o..o + (i1 - i0) * 4]);
            }
            cov_row(dst, layer_row, &row[..nu * 4], &cv[..nu], nu, t.mode);
            true
        }
        _ => false,
    }
}

// k_bm_cov_row with the layer row or in place
fn cov_row(dst: &mut [u8], layer_row: Option<&[u8]>, top: &[u8], cov: &[u8], n: usize, mode: u32) {
    match layer_row {
        Some(l) => bm_cov_row(dst, &l[..n * 4], top, Some(cov), n, mode),
        None => bm_cov_row_inplace(dst, top, Some(cov), n, mode),
    }
}

/// fill_pattern_row(dst, n, x, y, fs, fg, bg) -- n straight pixels of fill style fs for the
/// pixels (x .. x+n-1, y), anchored at the image origin (FS_SOLID: fg).  Pure.
pub(crate) fn fill_pattern_row(dst: &mut [u8], n: i32, x: i32, y: i32, fs: u32, fg: u32, bg: u32) {
    if n <= 0 {
        return;
    }
    let n = n as usize;
    if (FS_HATCH0 as u32..FS_COUNT as u32).contains(&fs) {
        let bits = HATCH_PATS[(fs - 1) as usize][(y & 7) as usize] as u32;
        for i in 0..n {
            let b = 7 - (x.wrapping_add(i as i32) & 7) as u32; // bit 7 = column 0
            let c = if bits >> b & 1 != 0 { fg } else { bg };
            dst[i * 4..i * 4 + 4].copy_from_slice(&c.to_le_bytes());
        }
    } else {
        for i in 0..n {
            dst[i * 4..i * 4 + 4].copy_from_slice(&fg.to_le_bytes());
        }
    }
}

// tl_view(p, tl) -> the record with its mask and source resolved (None: the source layer of a
// TLK_SOURCE record is gone -- it shows nothing then, as a missing TL_SRC in the asm)
fn tl_view<'a>(p: &'a Pn, tl: usize) -> TlView<'a> {
    let t = &p.tool.tls[tl];
    let mask = match &t.mask {
        TlMask::None => None,
        TlMask::Own(m) => Some(m),
        TlMask::Be => crate::tools::brushes::be_mask(&p.be),
        TlMask::Bucket(i) => p.tbucket.mf.get(*i).and_then(|m| m.as_ref()),
        TlMask::Grad => p.tgrad.mask.as_ref(),
    };
    let src = match &t.src {
        TlSrc::None => None,
        TlSrc::Own(s) => Some(s),
        TlSrc::Layer(uid) => t.doc.and_then(|d| p.doc.get(d)).and_then(|d| lay_find_uid(d, *uid).map(|i| &d.layers[i as usize].sf)),
        TlSrc::Grad => Some(&p.tgrad.src),
        TlSrc::Clone => crate::tools::clone::tc_surface(&p.tclone),
    };
    TlView { tl: t, mask, src, tclip: if p.tool.tclip_on { Some(p.tool.tclip) } else { None } }
}

// the compose hook of a tool layer
struct TlHook<'a>(TlView<'a>);

impl CompHook for TlHook<'_> {
    fn row(&self, doc: &Doc, dst: &mut [u8], x: i32, y: i32, n: i32, layer_row: &[u8]) -> bool {
        tl_row(&self.0, doc, dst, x, y, n, Some(layer_row))
    }
}

/// the compose hook of tool layer `tl` (HookKey::Tl(tl)): resolved by compose at update time
pub(crate) fn tl_hook(p: &Pn, tl: usize) -> Option<Box<dyn CompHook + '_>> {
    if tl >= TL_COUNT {
        return None;
    }
    Some(Box::new(TlHook(tl_view(p, tl))))
}

/// tl_commit(tl, name, icon) -> 1 history step -- write the current layer with tl_row (only the
/// non-empty 64 x 64 tiles of TL_MASK), history "name", doc_mark_px, hook removed
pub(crate) fn tl_commit(p: &mut Pn, tl: usize, name: &'static str, icon: u32) -> bool {
    let Some(doc) = p.tool.tls[tl].doc else { return false };
    let Some(d) = p.doc.get(doc) else {
        tl_hide(p, tl);
        return false;
    };
    let cur = d.cur;
    let Some(ly) = (if cur >= 0 { d.layers.get(cur as usize) } else { None }) else {
        tl_hide(p, tl);
        return false;
    };
    if ly.sf.px.is_empty() {
        tl_hide(p, tl);
        return false;
    }
    let b = p.tool.tls[tl].bx;
    let x0 = b[0].max(0);
    let y0 = b[1].max(0);
    let x1 = b[2].min(ly.sf.w);
    let y1 = b[3].min(ly.sf.h);
    if x0 >= x1 || y0 >= y1 {
        tl_hide(p, tl);
        return false;
    }
    // the tile grid of the box: the tiles whose mask slot is not empty
    let (ty0, ty1, tx0, tx1) = (y0 >> 6, (y1 - 1) >> 6, x0 >> 6, (x1 - 1) >> 6);
    let mut list: Vec<(i32, i32)> = Vec::new();
    {
        let v = tl_view(p, tl);
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                if let Some(m) = v.mask {
                    // (a slot is empty when its first pixel is a run of the empty sentinel)
                    if ty as u32 >= m.tiles_y() || tx as u32 >= m.tiles_x() || matches!(m.span(tx << 6, ty << 6, 1).0, Span::Empty) {
                        continue;
                    }
                }
                list.push((tx, ty));
            }
        }
    }
    if list.is_empty() {
        tl_hide(p, tl);
        return false;
    }
    // no pixel is written that the history could not save (out of memory / budget): then no step
    if hist_px_begin(p, doc, -1) != 0 {
        tl_hide(p, tl);
        return false;
    }
    for &(tx, ty) in &list {
        let ax = (tx << 6).max(x0);
        let ay = (ty << 6).max(y0);
        let bx = ((tx << 6) + 64).min(x1);
        let by = ((ty << 6) + 64).min(y1);
        if hist_px_touch(p, doc, ax, ay, bx, by) != 0 {
            hist_px_cancel(p, doc);
            tl_hide(p, tl);
            return false;
        }
    }
    // tl_row in place over the rows of the listed tiles (worker pool); the layer's surface is
    // taken out of the document meanwhile (tl_row reads the document's selection)
    let mut sf = core::mem::take(&mut p.doc.dm(doc).layers[cur as usize].sf);
    {
        let v = tl_view(p, tl);
        let d = p.doc.d(doc);
        let stride = sf.stride as usize;
        let rows: Vec<SpinLock<&mut [u8]>> = sf.px.chunks_mut(stride).skip(y0 as usize).take((y1 - y0) as usize).map(SpinLock::new).collect();
        let list = &list;
        let item = |i: usize, _: &plat::JobCtl| {
            let y = y0 + i as i32;
            let ty = y >> 6;
            let mut r = rows[i].lock();
            for &(tx, lty) in list.iter() {
                if lty != ty {
                    continue;
                }
                let a = (tx << 6).max(x0);
                let e = ((tx << 6) + 64).min(x1);
                if a < e {
                    tl_row(&v, d, &mut r[a as usize * 4..e as usize * 4], a, y, e - a, None);
                }
            }
        };
        let _ = plat::parallel_for((y1 - y0) as usize, &item);
    }
    p.doc.dm(doc).layers[cur as usize].sf = sf;
    tl_hide(p, tl);
    doc_mark_px(p, doc, cur, x0, y0, x1, y1);
    hist_px_end(p, doc, name, icon)
}

// ================================================================== test hooks (diag.rs block T1)
// Commands (cmd 930 + k ARG):
//   0 width * 100 (f64 TC_WIDTH = ARG / 100)   1 hardness %   2 antialiasing 0 / 1
//   3 blend mode BM_*   4 fill style FS_*   5 tolerance %   6 flood mode 0 / 1 (global)
//   7 sampling 0 Ebene / 1 Bild   8 spacing %   9 picker sample size index 0..5
//   10 picker "Nach dem Klick" 0..2   11 test clip corner x0 | y0 << 16
//   12 test clip x1 | y1 << 16 (on; 0 = off): the tool layer clips to that rectangle
//      instead of sel_clip_row   13 primary colour / 14 secondary colour = test colour
//      (0 transparent, 1 red, 2 green, 3 blue, 4 red 50 %, 5 white, 6 black, 7 grey,
//      8 black 50 %, 9 yellow)   15 bucket: 1 = always fill on the worker pool
//   16 selection mode   17 selection quality   18 path smoothing on / off
//   19 TLK_SOURCE test: layer pixels [0,10)^2 -> [100,110)^2 (tl_commit, history "Test")
// Records (hidden, every frame):
//   9930 value 1 while an edit is pending      9931 value current tool
//   9932 value width * 100                     9933 text primary "AARRGGBB"
//   9934 text secondary                        9935 value history entries (active image)
//   9936 value history position                9937 text options "hard,spacing,aa,blend,
//                                              fill,tol,flood,sample,selmode,selqual", value TC_VER
//   9938 text outline "x,y,r" (px), value visible   9939 text bucket "x,y" seed (live), value
//                                              1 while a background fill runs
//   9940 value previous tool                   9941 value DF_LIVE of the active image
//   9942 text picker state "size,after"

static T1_COLORS: [u32; 10] =
    [0x00000000, 0xFFFF0000, 0xFF00FF00, 0xFF0000FF, 0x80FF0000, 0xFFFFFFFF, 0xFF000000, 0xFF808080, 0x80000000, 0xFFFFFF00];
/// hooks 931..938: TOOLCFG field offsets (index 0 unused)
static T1_FIELDS: [u8; 9] = [0, 8, 16, 20, 24, 28, 32, 36, 12];
/// record 9937: TC_HARD, TC_SPACING, TC_AA, TC_BLEND, TC_FILL, TC_TOL, TC_FLOOD, TC_SAMPLE,
/// TC_SELMODE, TC_SELQUAL
static T1_CFGF: [u8; 10] = [8, 12, 16, 20, 24, 28, 32, 36, 40, 44];

/// t1_diag_cmd(id, arg) -> 1
pub(crate) fn t1_diag_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    let k = id.wrapping_sub(CMD_T1_TEST0);
    let mut changed = false;
    match k {
        0 => {
            let w = arg as i32 as f64 / 100.0;
            p.tool.cfg.width = minsd(maxsd(w, 1.0), 2000.0);
            changed = true;
        }
        1..=8 => {
            if let Some(f) = p.tool.cfg.field(T1_FIELDS[k as usize] as u32) {
                *f = arg;
            }
            changed = true;
        }
        9 | 10 => {
            crate::tools::picker::pk_test(p, k - 9, arg as i32);
            changed = true;
        }
        11 => {
            p.tool.tclip[0] = (arg & 0xFFFF) as i32;
            p.tool.tclip[1] = (arg >> 16) as i32;
        }
        12 => {
            p.tool.tclip_on = arg != 0;
            p.tool.tclip[2] = (arg & 0xFFFF) as i32;
            p.tool.tclip[3] = (arg >> 16) as i32;
            if let Some(d) = doc_active(&p.doc) {
                comp_mark_all(p, d);
            }
        }
        13 | 14 => {
            if arg < 10 {
                let c = T1_COLORS[arg as usize];
                if k == 13 {
                    pal_set_primary(&mut p.pal, c);
                } else {
                    pal_set_secondary(&mut p.pal, c);
                }
            }
        }
        15 => crate::tools::bucket::bk_test_async(&mut p.tbucket, arg != 0),
        16 => {
            p.tool.cfg.selmode = arg;
            changed = true;
        }
        17 => {
            p.tool.cfg.selqual = arg;
            changed = true;
        }
        18 => {
            p.tool.cfg.smooth = arg;
            changed = true;
        }
        19 => t1_source_test(p),
        _ => {}
    }
    if changed {
        tool_cfg_changed(p);
        outline_update(p, ui);
    }
    ui.repass();
    true
}

// t1_source_test() -- hook 949: the current layer's pixels [0,10) x [0,10) are written to
// [100,110) x [100,110) through a TLK_SOURCE tool layer (tl_commit, history "Test").  The
// source is a copy of the layer (tl_commit writes the layer while the workers read the
// source; the regions do not overlap, so the copy gives the same pixels).
fn t1_source_test(p: &mut Pn) {
    let Some(doc) = doc_active(&p.doc) else { return };
    let d = p.doc.d(doc);
    if d.cur < 0 || d.cur as usize >= d.layers.len() {
        return;
    }
    let src = sf_copy(&d.layers[d.cur as usize].sf);
    tl_init(p, TL_T1, doc);
    let t = &mut p.tool.tls[TL_T1];
    t.kind = TLK_SOURCE as u32;
    t.mode = BM_NORMAL | 255 << BM_OPAC_SHIFT;
    t.src = TlSrc::Own(src);
    t.sx = 100;
    t.sy = 100;
    t.bx = [100, 100, 110, 110];
    tl_commit(p, TL_T1, "Test", 0);
    p.tool.tls[TL_T1].src = TlSrc::None;
}

/// a copy of a surface (same size, format and stride)
pub(crate) fn sf_copy(s: &Surface) -> Surface {
    let mut c = Surface::new(s.w, s.h, s.fmt);
    if c.px.len() == s.px.len() {
        c.px.copy_from_slice(&s.px);
    }
    c
}

// t1_sem(id, label, value) -- a hidden K_TEXT record (empty rectangle)
pub(crate) fn t1_sem(ui: &mut Ui, id: u32, label: &str, value: i32) {
    let s = ui.sem(K_TEXT, id, Some(label), value);
    s.rect = [0; 4];
    s.state = 0;
}

/// t1_diag_frame() -- the hidden records 9930..9942
pub(crate) fn t1_diag_frame(p: &mut Pn, ui: &mut Ui) {
    let t = &p.tool;
    t1_sem(ui, ID_T1_TEST0, "", t.live_doc.is_some() as i32);
    t1_sem(ui, ID_T1_TEST0 + 1, "", t.cur);
    t1_sem(ui, ID_T1_TEST0 + 2, "", cvtd(t.cfg.width * 100.0));
    t1_sem(ui, ID_T1_TEST0 + 3, &format!("{:08X}", pal_primary(&p.pal)), 0);
    t1_sem(ui, ID_T1_TEST0 + 4, &format!("{:08X}", pal_secondary(&p.pal)), 0);
    if let Some(doc) = doc_active(&p.doc) {
        let d = p.doc.d(doc);
        t1_sem(ui, ID_T1_TEST0 + 5, "", hist_count(d));
        t1_sem(ui, ID_T1_TEST0 + 6, "", hist_pos(d));
        t1_sem(ui, ID_T1_TEST0 + 11, "", ((d.flags & DF_LIVE as u32) >> 3) as i32);
    }
    // options text
    let mut c = p.tool.cfg;
    let mut s = String::new();
    for (i, &f) in T1_CFGF.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!("{}", c.field(f as u32).map_or(0, |v| *v as i32)));
    }
    t1_sem(ui, ID_T1_TEST0 + 7, &s, p.tool.cfg.ver as i32);
    // outline "x,y,r"
    let t = &p.tool;
    t1_sem(ui, ID_T1_TEST0 + 8, &format!("{},{},{}", t.ol_x, t.ol_y, t.ol_r >> 8), t.ol_vis as i32);
    // bucket
    let (bs, brun) = crate::tools::bucket::bk_state(&p.tbucket);
    t1_sem(ui, ID_T1_TEST0 + 9, &bs, brun as i32);
    t1_sem(ui, ID_T1_TEST0 + 10, "", p.tool.prev);
    t1_sem(ui, ID_T1_TEST0 + 12, &crate::tools::picker::pk_state(&p.tpicker), 0);
}

// ================================================================== data

/// fill styles: Volltonfarbe + System.Drawing HatchStyle 0..52 (own German names)
pub(crate) static FILL_NAMES: [&str; 54] = [
    "Volltonfarbe",
    "Horizontal",
    "Vertikal",
    "Diagonal fallend",
    "Diagonal steigend",
    "Kreuz",
    "Diagonale Kreuze",
    "05 Prozent",
    "10 Prozent",
    "20 Prozent",
    "25 Prozent",
    "30 Prozent",
    "40 Prozent",
    "50 Prozent",
    "60 Prozent",
    "70 Prozent",
    "75 Prozent",
    "80 Prozent",
    "90 Prozent",
    "Diagonal fallend, fein",
    "Diagonal steigend, fein",
    "Diagonal fallend, dicht",
    "Diagonal steigend, dicht",
    "Diagonal fallend, breit",
    "Diagonal steigend, breit",
    "Vertikal, fein",
    "Horizontal, fein",
    "Vertikal, eng",
    "Horizontal, eng",
    "Vertikal, dicht",
    "Horizontal, dicht",
    "Diagonal fallend, gestrichelt",
    "Diagonal steigend, gestrichelt",
    "Horizontal, gestrichelt",
    "Vertikal, gestrichelt",
    "Konfetti (klein)",
    "Konfetti (groß)",
    "Zickzack",
    "Wellen",
    "Diagonale Ziegel",
    "Horizontale Ziegel",
    "Gewebe",
    "Karo",
    "Rasen",
    "Gepunktetes Gitter",
    "Gepunktete Rauten",
    "Dachziegel",
    "Gitter",
    "Kugeln",
    "Kariert (klein)",
    "Schachbrett (klein)",
    "Schachbrett (groß)",
    "Rauten",
    "Gefüllte Rauten",
];

/// status bar help texts per tool (TOOL_* order)
static HELP_TAB: [&str; 19] = [
    "Rechteckige Auswahl mit gedrückter Maustaste aufziehen. Mit Umschalt wird sie quadratisch.",
    "Ziehen bewegt den ausgewählten Inhalt, die Anfasser skalieren ihn. Mit der rechten Maustaste gezogen dreht er sich.",
    "Mit gedrückter Maustaste eine freie Auswahlform umfahren.",
    "Ziehen bewegt die Auswahlkontur, die Anfasser skalieren oder drehen sie. Umschalt hält beim Skalieren die Proportionen und lässt den Winkel einrasten; Alt skaliert in alle Richtungen gleich.",
    "Elliptische Auswahl mit gedrückter Maustaste aufziehen. Mit Umschalt wird sie kreisrund.",
    "Links: hineinzoomen, rechts: herauszoomen, Mitte: Ansicht schieben",
    "Ein Klick wählt zusammenhängende Pixel ähnlicher Farbe aus. Mit Umschalt zählen passende Pixel im ganzen Bild.",
    "Mit gedrückter Maustaste ziehen, um die Ansicht des Bildes zu verschieben",
    "Linksklick füllt eine zusammenhängende Fläche mit der Primärfarbe, Rechtsklick mit der Sekundärfarbe. Mit Umschalt wird jede passende Stelle im ganzen Bild gefüllt.",
    "Mit gedrückter Maustaste Richtung und Länge des Verlaufs ziehen. Umschalt lässt den Winkel in Stufen einrasten.",
    "Mit der linken Maustaste malt der Pinsel in der Primärfarbe, mit der rechten in der Sekundärfarbe.",
    "Entfernt die Pixel unter der Werkzeugspitze",
    "Zeichnet pixelgenaue Freihandlinien: links in der Primärfarbe, rechts in der Sekundärfarbe",
    "Linksklick nimmt die Farbe unter dem Mauszeiger als Primärfarbe auf, Rechtsklick als Sekundärfarbe. Mit Strg zählt das Gesamtbild aller Ebenen.",
    "Strg+Klick legt die Quelle fest. Danach mit gedrückter linker Maustaste malen, um von dort zu kopieren.",
    "Ersetzt beim Malen Farben: mit der linken Maustaste durch die Primärfarbe, mit der rechten durch die Sekundärfarbe.",
    "Ein Klick setzt die Schreibmarke, danach einfach lostippen. Der Text erscheint in der Primärfarbe.",
    "Linke Maustaste zeichnet in der Primärfarbe, rechte in der Sekundärfarbe. Umschalt rastet die Richtung ein, Alt zeichnet von der Mitte aus.",
    "Mit gedrückter Maustaste eine Form aufziehen. Umschalt hält die Proportionen fest, Alt zeichnet von der Mitte aus.",
];

/// tool blend modes (paint.net ContentBlendMode names), in menu order
static BLEND_LABELS: [&str; 15] = [
    "Normal",
    "Überschreiben",
    "Multiplizieren",
    "Additiv",
    "Farbig nachbelichten",
    "Farbig abwedeln",
    "Reflektieren",
    "Leuchten",
    "Überlagerung",
    "Unterschied",
    "Negierung",
    "Aufhellen",
    "Verdunkeln",
    "Bildschirm",
    "XOR",
];

/// 8 x 8 hatch bitmaps, one byte per row, bit 7 = column 0 (1 = foreground)
static HATCH_PATS: [[u8; 8]; 53] = [
    [0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // 0 Horizontal
    [0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80], // 1 Vertical
    [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01], // 2 ForwardDiagonal
    [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80], // 3 BackwardDiagonal
    [0xFF, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80], // 4 Cross
    [0x81, 0x42, 0x24, 0x18, 0x18, 0x24, 0x42, 0x81], // 5 DiagonalCross
    [0x80, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00], // 6 Percent05
    [0x80, 0x00, 0x08, 0x00, 0x80, 0x00, 0x08, 0x00], // 7 Percent10
    [0x88, 0x00, 0x22, 0x00, 0x88, 0x00, 0x22, 0x00], // 8 Percent20
    [0x88, 0x22, 0x88, 0x22, 0x88, 0x22, 0x88, 0x22], // 9 Percent25
    [0xAA, 0x44, 0xAA, 0x11, 0xAA, 0x44, 0xAA, 0x11], // 10 Percent30
    [0xAA, 0x44, 0xAA, 0x55, 0xAA, 0x44, 0xAA, 0x55], // 11 Percent40
    [0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55], // 12 Percent50
    [0x55, 0xBB, 0x55, 0xFF, 0x55, 0xBB, 0x55, 0xFF], // 13 Percent60
    [0x55, 0xFF, 0xDD, 0xFF, 0x55, 0xFF, 0xDD, 0xFF], // 14 Percent70
    [0x77, 0xDD, 0x77, 0xDD, 0x77, 0xDD, 0x77, 0xDD], // 15 Percent75
    [0x77, 0xFF, 0xDD, 0xFF, 0x77, 0xFF, 0xDD, 0xFF], // 16 Percent80
    [0x7F, 0xFF, 0xF7, 0xFF, 0x7F, 0xFF, 0xF7, 0xFF], // 17 Percent90
    [0x88, 0x44, 0x22, 0x11, 0x88, 0x44, 0x22, 0x11], // 18 LightDownwardDiagonal
    [0x11, 0x22, 0x44, 0x88, 0x11, 0x22, 0x44, 0x88], // 19 LightUpwardDiagonal
    [0xCC, 0x66, 0x33, 0x99, 0xCC, 0x66, 0x33, 0x99], // 20 DarkDownwardDiagonal
    [0x33, 0x66, 0xCC, 0x99, 0x33, 0x66, 0xCC, 0x99], // 21 DarkUpwardDiagonal
    [0xC1, 0xE0, 0x70, 0x38, 0x1C, 0x0E, 0x07, 0x83], // 22 WideDownwardDiagonal
    [0x83, 0x07, 0x0E, 0x1C, 0x38, 0x70, 0xE0, 0xC1], // 23 WideUpwardDiagonal
    [0x88, 0x88, 0x88, 0x88, 0x88, 0x88, 0x88, 0x88], // 24 LightVertical
    [0xFF, 0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00], // 25 LightHorizontal
    [0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA], // 26 NarrowVertical
    [0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00], // 27 NarrowHorizontal
    [0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC, 0xCC], // 28 DarkVertical
    [0xFF, 0xFF, 0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00], // 29 DarkHorizontal
    [0x00, 0x00, 0x88, 0x44, 0x22, 0x11, 0x00, 0x00], // 30 DashedDownwardDiagonal
    [0x00, 0x00, 0x11, 0x22, 0x44, 0x88, 0x00, 0x00], // 31 DashedUpwardDiagonal
    [0xF0, 0x00, 0x00, 0x00, 0x0F, 0x00, 0x00, 0x00], // 32 DashedHorizontal
    [0x80, 0x80, 0x80, 0x80, 0x08, 0x08, 0x08, 0x08], // 33 DashedVertical
    [0x80, 0x08, 0x40, 0x02, 0x10, 0x01, 0x20, 0x04], // 34 SmallConfetti
    [0xB1, 0x30, 0x03, 0x1B, 0xD8, 0xC0, 0x0C, 0x8D], // 35 LargeConfetti
    [0x81, 0x42, 0x24, 0x18, 0x81, 0x42, 0x24, 0x18], // 36 ZigZag
    [0x00, 0x18, 0xA4, 0x03, 0x00, 0x18, 0xA4, 0x03], // 37 Wave
    [0x01, 0x02, 0x04, 0x08, 0x18, 0x24, 0x42, 0x81], // 38 DiagonalBrick
    [0xFF, 0x80, 0x80, 0x80, 0xFF, 0x08, 0x08, 0x08], // 39 HorizontalBrick
    [0x88, 0x54, 0x22, 0x45, 0x88, 0x14, 0x22, 0x51], // 40 Weave
    [0xAA, 0x55, 0xAA, 0x55, 0xF0, 0xF0, 0xF0, 0xF0], // 41 Plaid
    [0x80, 0x40, 0x80, 0x00, 0x08, 0x04, 0x08, 0x00], // 42 Divot
    [0xAA, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00], // 43 DottedGrid
    [0x80, 0x00, 0x22, 0x00, 0x08, 0x00, 0x22, 0x00], // 44 DottedDiamond
    [0x03, 0x84, 0x48, 0x30, 0x0C, 0x02, 0x01, 0x01], // 45 Shingle
    [0xFF, 0x66, 0xFF, 0x99, 0xFF, 0x66, 0xFF, 0x99], // 46 Trellis
    [0x77, 0x89, 0x8F, 0x8F, 0x77, 0x98, 0xF8, 0xF8], // 47 Sphere
    [0xFF, 0x88, 0x88, 0x88, 0xFF, 0x88, 0x88, 0x88], // 48 SmallGrid
    [0xCC, 0xCC, 0x33, 0x33, 0xCC, 0xCC, 0x33, 0x33], // 49 SmallCheckerBoard
    [0xF0, 0xF0, 0xF0, 0xF0, 0x0F, 0x0F, 0x0F, 0x0F], // 50 LargeCheckerBoard
    [0x41, 0x22, 0x14, 0x08, 0x14, 0x22, 0x41, 0x80], // 51 OutlinedDiamond
    [0x10, 0x38, 0x7C, 0xFE, 0x7C, 0x38, 0x10, 0x00], // 52 SolidDiamond
];
