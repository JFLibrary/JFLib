// t_gradient -- PaintCraft: Farbverlauf (gradient tool G, agent T3; spec §2.6, DRAW-013/014).
//                                              [apps/PaintCraft/src/t_gradient.s]
//
// Dragging draws a gradient from the press point to the pointer: left button primary ->
// secondary colour, right button reversed.  7 types (Linear, Linear (gespiegelt), Raute,
// Radial, Konisch, Spirale rechts- / linksdrehend), 3 repeat modes (Keine Wiederholung /
// Periodisch / Gespiegelt wiederholen), Farbverlauf (colours with alpha, blend mode incl.
// Überschreiben) or Transparenzverlauf (only the layer's alpha is scaled by the ramp
// primary.a -> 255 - secondary.a, paint.net's alpha-only rule).  Antialiasing: 4 samples per
// pixel (rotated grid, alpha-weighted mean) where the gradient has hard edges (zyklische
// Wiederholung, spirals).  The gradient stays LIVE (TF_LIVE): the end points and a middle
// nub (moves the whole gradient) can be dragged, Shift snaps the angle to 15° steps, a right
// click on a nub swaps the colours, colour / option / selection changes re-render -- until
// Fertigstellen / Enter / another tool / image / document command (one history step
// "Farbverlauf"); Esc / Strg+Z drop it.  Rendering: the gradient of the box (image ∩
// selection bounds) is computed with grad_span / grad_alpha_span (gfx::gradient) on the
// worker pool (one item per 64-row band) into a private straight surface (TLK_SOURCE) or a
// coverage mask 255 - ramp (TLK_ERASE, strength 255); the tool layer is the only write path,
// so the preview is the result.
//
//   tgrad_init()                          registers TOOL_GRADIENT (tool_register)
//   tgrad_cmd(id, arg) -> 1 handled       test hooks CMD_TGRAD_FIRST..LAST:
//       792 type | repeat << 8 | transparency << 16      options
//       793 x | y << 16 (| 1 << 31 = right button)       start point (pixel centre) of 794
//       794 x | y << 16                                  end point: new live gradient (or the
//                                                        live one moved) on the active image
//   Hidden records (in the tool row): ID_TGRAD_FIRST text "sx,sy,ex,ey" (image px, floor),
//   value 0 none / 1 live, nothing shown yet / 2 shown; ID_TGRAD_FIRST + 1 text
//   "type,repeat,transparency,swapped,aa-samples", value drag state (DRG_*)
//
// Rust model: the colour surface and the transparency mask stay in TGrad (src / mask); the
// tool layer refers to them (TlSrc::Grad / TlMask::Grad).  The alpha bands are computed into
// band buffers on the workers and copied into the mask's tiles afterwards (same bytes).

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gfx::blendmode::{BM_COUNT, BM_OPAC_SHIFT};
use gfx::gradient::{grad_alpha_span, grad_span, Grad, GR_WRAP, GT_SPIRAL_CW};
use gfx::tmask::TMask;
use gfx::{Surface, FMT_ARGB};
use rt::fx::{cvtd, sqrtd};
use rt::sync::SpinLock;
use ui::*;

use crate::app::constants::*;
use crate::document::model::{doc_active, DocId};
use crate::workspace::nubs::{nub_set, nubs_damage, nubs_draw, nubs_hit, nubs_hover, Nub};
use crate::workspace::palette::{pal_primary, pal_secondary, pal_version};
use crate::document::selection::{sel_active, sel_bbox};
use crate::tools::*;
use crate::workspace::view::view_cursor;
use crate::app::Pn;

const TG_NTYPES: u32 = 7;
const TG_NREP: u32 = 3;
const TG_NNUBS: usize = 3;
const NB_MID: usize = 0; // nub order: the move nub first, end points on top
const NB_S: usize = 1;
const NB_E: usize = 2;
const DRG_NONE: u32 = 0;
const DRG_MID: u32 = 1; // = nub index + 1
const DRG_S: u32 = 2;
const DRG_E: u32 = 3;
const DRG_NEW: u32 = 4; // the first drag: start fixed, end at the pointer
const ID_TG_REC: u32 = ID_TGRAD_FIRST; // record "sx,sy,ex,ey"
const ID_TG_REC2: u32 = ID_TGRAD_FIRST + 1; // record options
const ID_TG_MODE: u32 = ID_TGRAD_FIRST + 0x10; // Farbverlauf / Transparenzverlauf (rows +1 .. +2)
const ID_TG_REP: u32 = ID_TGRAD_FIRST + 0x20; // repeat mode (rows +1 .. +3)
const C_INV256: f64 = 0.00390625;

/// render context (g_tg_ctx; the workers only read it)
#[derive(Clone, Copy, Default)]
struct TgCtx {
    /// TGC_G: sampled at the pixel centres
    g: Grad,
    /// TGC_SS: the supersamples (end points shifted)
    ss: [Grad; 4],
    /// TGC_AA: 4 samples per pixel
    aa: bool,
    /// TGC_MODE: 0 colour / 1 transparency
    mode: u32,
    /// TGC_X0 .. TGC_Y1: box (image px, x1 / y1 exclusive)
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

/// the state of t_gradient.s (its g_* globals)
#[derive(Default)]
pub(crate) struct TGrad {
    ctx: TgCtx,
    /// colour mode: the gradient (straight ARGB, image size)
    pub(crate) src: Surface,
    /// transparency mode: coverage 255 - ramp
    pub(crate) mask: Option<TMask>,
    /// middle, start, end
    nubs: [Nub; TG_NNUBS],
    doc: Option<DocId>,
    live: bool,
    /// DRG_*
    drag: u32,
    dragbtn: u32,
    /// BTN_LEFT primary -> secondary, BTN_RIGHT reversed
    btn: u32,
    /// end points (image px, 24.8)
    sx: i32,
    sy: i32,
    ex: i32,
    ey: i32,
    /// end points when a nub drag started
    osx: i32,
    osy: i32,
    oex: i32,
    oey: i32,
    /// pointer when a nub drag started
    ax: i32,
    ay: i32,
    /// options: GT_*, GR_*, 1 = Transparenzverlauf
    typ: u32,
    rep: u32,
    alpha: u32,
    /// TC_VER / pal_version / DOC_VER_SEL rendered
    pver: u32,
    palv: u32,
    vsel: u32,
    nrender: u32,
    /// hook 793: start point, button
    hx: i32,
    hy: i32,
    hbtn: u32,
}

static DESC_GRAD: ToolDesc = ToolDesc {
    flags: (TF_LIVE | TF_CLIP_SEL | TF_AA | TF_BLEND) as u32,
    input: Some(tg_input),
    options: Some(tg_options),
    commit: Some(tg_commit),
    cancel: Some(tg_cancel),
    cursor: Some(tg_cursor),
    overlay: Some(tg_overlay),
    tick: Some(tg_tick),
    ..ToolDesc::NONE
};
static TYPE_LABELS: [&str; 7] =
    ["Linear", "Linear (gespiegelt)", "Raute", "Radial", "Konisch", "Spirale rechtsdrehend", "Spirale linksdrehend"];
static MODE_LABELS: [&str; 2] = ["Farbverlauf", "Transparenzverlauf"];
static REP_LABELS: [&str; 3] = ["Keine Wiederholung", "Periodisch wiederholen", "Gespiegelt wiederholen"];
/// rotated-grid supersample offsets (x, y)
static SS_OFF: [[f64; 2]; 4] = [[-0.375, -0.125], [0.125, -0.375], [0.375, 0.125], [-0.125, 0.375]];
/// (cos, sin) of k * 15 degrees, k = 0 .. 23 (the asm's table bits)
static SNAP_DIRS: [[u64; 2]; 24] = [
    [0x3FF0000000000000, 0x0000000000000000],
    [0x3FEEE8DD4748BF15, 0x3FD0907DC1930690],
    [0x3FEBB67AE8584CAB, 0x3FDFFFFFFFFFFFFF],
    [0x3FE6A09E667F3BCD, 0x3FE6A09E667F3BCD],
    [0x3FE0000000000001, 0x3FEBB67AE8584CAA],
    [0x3FD0907DC1930690, 0x3FEEE8DD4748BF15],
    [0x0000000000000000, 0x3FF0000000000000],
    [0xBFD0907DC1930692, 0x3FEEE8DD4748BF15],
    [0xBFDFFFFFFFFFFFFC, 0x3FEBB67AE8584CAB],
    [0xBFE6A09E667F3BCC, 0x3FE6A09E667F3BCD],
    [0xBFEBB67AE8584CAB, 0x3FDFFFFFFFFFFFFF],
    [0xBFEEE8DD4748BF14, 0x3FD0907DC1930695],
    [0xBFF0000000000000, 0x0000000000000000],
    [0xBFEEE8DD4748BF15, 0xBFD0907DC1930691],
    [0xBFEBB67AE8584CAA, 0xBFE0000000000001],
    [0xBFE6A09E667F3BCE, 0xBFE6A09E667F3BCC],
    [0xBFE0000000000004, 0xBFEBB67AE8584CA9],
    [0xBFD0907DC193068E, 0xBFEEE8DD4748BF15],
    [0x0000000000000000, 0xBFF0000000000000],
    [0x3FD0907DC1930688, 0xBFEEE8DD4748BF16],
    [0x3FE0000000000001, 0xBFEBB67AE8584CAA],
    [0x3FE6A09E667F3BCB, 0xBFE6A09E667F3BCE],
    [0x3FEBB67AE8584CA8, 0xBFE0000000000004],
    [0x3FEEE8DD4748BF15, 0xBFD0907DC193068F],
];

/// tgrad_init() -- register the tool
pub(crate) fn tgrad_init(p: &mut Pn, _ui: &mut Ui) {
    tool_register(&mut p.tool, TOOL_GRADIENT, Some(&DESC_GRAD));
}

// ------------------------------------------------------------------ geometry

// tg_snap(ox, oy, px, py) -> (x, y): p rotated around o to the nearest multiple of 15° (same
// length); all 24.8 image coordinates
fn tg_snap(ox: i32, oy: i32, px: i32, py: i32) -> (i32, i32) {
    let vx = px.wrapping_sub(ox) as f64;
    let vy = py.wrapping_sub(oy) as f64;
    let len = sqrtd(vx * vx + vy * vy);
    let mut best = 0usize;
    let mut bdot = vx; // v . dir_0 (1, 0)
    for (k, d) in SNAP_DIRS.iter().enumerate().skip(1) {
        let dot = f64::from_bits(d[0]) * vx + f64::from_bits(d[1]) * vy;
        if dot > bdot {
            bdot = dot;
            best = k;
        }
    }
    let d = SNAP_DIRS[best];
    (cvtd(f64::from_bits(d[0]) * len).wrapping_add(ox), cvtd(f64::from_bits(d[1]) * len).wrapping_add(oy))
}

// tg_place(): the nubs at the end points and the middle (damaged)
fn tg_place(p: &mut Pn, ui: &mut Ui) {
    let t = &mut p.tgrad;
    nub_set(&p.view, ui, &mut t.nubs[NB_S], t.sx, t.sy);
    nub_set(&p.view, ui, &mut t.nubs[NB_E], t.ex, t.ey);
    let (mx, my) = (t.sx.wrapping_add(t.ex) >> 1, t.sy.wrapping_add(t.ey) >> 1);
    nub_set(&p.view, ui, &mut t.nubs[NB_MID], mx, my);
}

// tg_show_nubs(): the nubs become visible (first time) and follow the end points
fn tg_show_nubs(p: &mut Pn, ui: &mut Ui) {
    if p.tgrad.nubs[0].flags & NBF_HIDDEN as u32 != 0 {
        for n in p.tgrad.nubs.iter_mut() {
            n.flags &= !(NBF_HIDDEN as u32);
        }
        tg_place(p, ui);
        nubs_damage(&p.view, ui, &mut p.tgrad.nubs);
    } else {
        tg_place(p, ui);
    }
}

// tg_begin(doc, btn): a new live gradient on doc (both end points at the pointer / hook
// start, nothing shown yet)
fn tg_begin(p: &mut Pn, doc: DocId, btn: u32) {
    let t = &mut p.tgrad;
    t.doc = Some(doc);
    t.live = true;
    t.btn = btn;
    t.drag = DRG_NONE;
    let (sx, sy) = (t.sx, t.sy);
    for (i, n) in t.nubs.iter_mut().enumerate() {
        n.x = sx;
        n.y = sy;
        n.kind = if i == 0 { NK_SQUARE as u32 } else { NK_CIRCLE as u32 }; // the move nub is square
        n.flags = NBF_HIDDEN as u32;
    }
    tl_init(p, TL_GRADIENT, doc);
    tool_live_begin(p, doc);
}

// tg_end(): the live gradient is over: nubs hidden, buffers freed
fn tg_end(p: &mut Pn, ui: &mut Ui) {
    let t = &mut p.tgrad;
    t.live = false;
    t.drag = DRG_NONE;
    for n in t.nubs.iter_mut() {
        n.flags = (n.flags & !((NBF_HOVER | NBF_DRAG) as u32)) | NBF_HIDDEN as u32;
    }
    nubs_damage(&p.view, ui, &mut t.nubs);
    t.src = Surface::default();
    t.mask = None;
    let tl = &mut p.tool.tls[TL_GRADIENT];
    tl.mask = TlMask::None;
    tl.src = TlSrc::None;
}

// ------------------------------------------------------------------ rendering

// tg_setup() -> true when the box is not empty: the context from the end points, the
// colours, the options and the selection bounds of the document
fn tg_setup(p: &mut Pn) -> bool {
    let Some(doc) = p.tgrad.doc else { return false };
    let t = &p.tgrad;
    let typ = if t.typ < TG_NTYPES { t.typ as u8 } else { 0 };
    let rep = if t.rep < TG_NREP { t.rep as u8 } else { 0 };
    // supersampling only where the gradient has hard edges (wrapped repeat, spirals)
    let mut aa = rep == GR_WRAP || typ >= GT_SPIRAL_CW;
    if p.tool.cfg.aa == 0 {
        aa = false;
    }
    let mode = (t.alpha != 0) as u32;
    let (ca, cb) = if mode == 0 {
        let a = tool_color(p, t.btn);
        let b = tool_color(p, if t.btn == BTN_RIGHT { BTN_LEFT } else { BTN_RIGHT });
        (a, b)
    } else {
        // transparency: ramp primary.a -> 255 - secondary.a (right button: reversed)
        let mut a = pal_primary(&p.pal) >> 24;
        let mut b = 255 - (pal_secondary(&p.pal) >> 24);
        if t.btn == BTN_RIGHT {
            core::mem::swap(&mut a, &mut b);
        }
        (a << 24, b << 24)
    };
    let g = Grad {
        typ,
        repeat: rep,
        flags: 0,
        col_a: ca,
        col_b: cb,
        sx: t.sx as f64 * C_INV256,
        sy: t.sy as f64 * C_INV256,
        ex: t.ex as f64 * C_INV256,
        ey: t.ey as f64 * C_INV256,
    };
    // the supersamples: sampling at p + o = the end points shifted by -o
    let mut ss = [g; 4];
    for (k, s) in ss.iter_mut().enumerate() {
        let o = SS_OFF[k];
        s.sx = g.sx - o[0];
        s.sy = g.sy - o[1];
        s.ex = g.ex - o[0];
        s.ey = g.ey - o[1];
    }
    // box: the image, cut to the selection bounds
    let d = p.doc.d(doc);
    let mut b = [0, 0, d.w, d.h];
    let mut ok = true;
    if sel_active(d) {
        match sel_bbox(d) {
            Some(r) => {
                b[0] = b[0].max(r[0]);
                b[1] = b[1].max(r[1]);
                b[2] = b[2].min(r[2]);
                b[3] = b[3].min(r[3]);
            }
            None => ok = false,
        }
    }
    if !ok || b[0] >= b[2] || b[1] >= b[3] {
        b = [0; 4];
        ok = false;
    }
    p.tgrad.ctx = TgCtx { g, ss, aa, mode, x0: b[0], y0: b[1], x1: b[2], y1: b[3] };
    ok
}

// tg_span_c(ctx, dst, n, x, y): n (1..64) straight gradient pixels of row y from column x;
// with aa the alpha-weighted mean of the 4 supersamples: 4 equal samples are copied, else
// a = (sa + 2) >> 2, c = (sum c_k a_k + sa / 2) / sa (0 when sa = 0).  Pure (workers).
fn tg_span_c(c: &TgCtx, dst: &mut [u8], n: usize, x: i32, y: i32) {
    let n = n.min(64);
    if n == 0 {
        return;
    }
    if !c.aa {
        grad_span(&mut dst[..n * 4], x, y, &c.g);
        return;
    }
    let mut s = [[0u8; 256]; 4];
    for k in 0..4 {
        grad_span(&mut s[k][..n * 4], x, y, &c.ss[k]);
    }
    for i in 0..n {
        let o = i * 4;
        let px = |k: usize| u32::from_le_bytes([s[k][o], s[k][o + 1], s[k][o + 2], s[k][o + 3]]);
        let p0 = px(0);
        let out = if p0 == px(1) && p0 == px(2) && p0 == px(3) {
            p0
        } else {
            let a = [s[0][o + 3] as u32, s[1][o + 3] as u32, s[2][o + 3] as u32, s[3][o + 3] as u32];
            let sa = a[0] + a[1] + a[2] + a[3];
            let mut v = 0u32;
            if sa != 0 {
                for ch in 0..3 {
                    let sum = s[0][o + ch] as u32 * a[0] + s[1][o + ch] as u32 * a[1] + s[2][o + ch] as u32 * a[2] + s[3][o + ch] as u32 * a[3];
                    v |= ((sum + (sa >> 1)) / sa) << (ch * 8);
                }
                v |= ((sa + 2) >> 2) << 24;
            }
            v
        };
        dst[o..o + 4].copy_from_slice(&out.to_le_bytes());
    }
}

// tg_span_a(ctx, dst, n, x, y): n (1..64) coverage bytes 255 - ramp (transparency mode: the
// erase then keeps alpha * ramp); with aa the ramp is the mean of the 4 supersamples
// ((s0 + s1 + s2 + s3 + 2) >> 2).  Pure (workers).
fn tg_span_a(c: &TgCtx, dst: &mut [u8], n: usize, x: i32, y: i32) {
    let n = n.min(64);
    if n == 0 {
        return;
    }
    if !c.aa {
        grad_alpha_span(&mut dst[..n], x, y, &c.g);
        for b in &mut dst[..n] {
            *b = !*b;
        }
        return;
    }
    let mut s = [[0u8; 64]; 4];
    for k in 0..4 {
        grad_alpha_span(&mut s[k][..n], x, y, &c.ss[k]);
    }
    for i in 0..n {
        let v = (s[0][i] as u32 + s[1][i] as u32 + s[2][i] as u32 + s[3][i] as u32 + 2) >> 2;
        dst[i] = !(v as u8);
    }
}

// tg_band(ctx, y0, y1, row(y) -> dst): the rows y0..y1 of a band, in chunks that never cross
// a 64-column tile
fn tg_band_rows(c: &TgCtx, ya: i32, yb: i32, mut row: impl FnMut(i32) -> (usize, usize), out: &mut [u8]) {
    for y in ya..yb {
        let (o, bpp) = row(y);
        let mut x = c.x0;
        while x < c.x1 {
            let n = (((x | 63) + 1).min(c.x1) - x) as usize;
            let a = o + (x - c.x0) as usize * bpp;
            if c.mode == 0 {
                tg_span_c(c, &mut out[a..a + n * 4], n, x, y);
            } else {
                tg_span_a(c, &mut out[a..a + n], n, x, y);
            }
            x += n as i32;
        }
    }
}

// tg_render(): compute the gradient (worker pool) and show it through the tool layer
fn tg_render(p: &mut Pn) {
    let Some(doc) = p.tgrad.doc else { return };
    p.tgrad.pver = p.tool.cfg.ver;
    p.tgrad.palv = pal_version(&p.pal);
    p.tgrad.vsel = p.doc.d(doc).ver_sel;
    p.tgrad.nrender = p.tgrad.nrender.wrapping_add(1);
    if tg_setup(p) && !tg_compute(p, doc) {
        // out of memory: nothing shown
        let tl = &mut p.tool.tls[TL_GRADIENT];
        tl.kind = TLK_NONE as u32;
        tl.mask = TlMask::None;
        let c = &mut p.tgrad.ctx;
        c.x0 = 0;
        c.y0 = 0;
        c.x1 = 0;
        c.y1 = 0;
    }
    let c = p.tgrad.ctx;
    tl_set_box(p, TL_GRADIENT, c.x0, c.y0, c.x1, c.y1);
    if !p.tool.tls[TL_GRADIENT].shown {
        tl_show(p, TL_GRADIENT);
    }
}

// tg_compute(doc) -> false = out of memory: the bands of the box into the colour surface or
// the transparency mask, the tool layer set up for them
fn tg_compute(p: &mut Pn, doc: DocId) -> bool {
    let (w, h) = {
        let d = p.doc.d(doc);
        (d.w, d.h)
    };
    let c = p.tgrad.ctx;
    let ty0 = (c.y0 >> 6) as usize;
    let nb = (((c.y1 - 1) >> 6) - (c.y0 >> 6) + 1) as usize; // 64-row bands
    if c.mode == 0 {
        // colour: a straight surface of the image size
        let t = &mut p.tgrad;
        if t.src.w != w || t.src.h != h || t.src.px.is_empty() {
            t.src = Surface::new(w, h, FMT_ARGB);
            if t.src.px.is_empty() {
                return false;
            }
        }
        let stride = t.src.stride as usize;
        let bands: Vec<SpinLock<&mut [u8]>> = t.src.px.chunks_mut(stride * 64).skip(ty0).take(nb).map(SpinLock::new).collect();
        let item = |i: usize, _: &plat::JobCtl| {
            let by = ((ty0 + i) << 6) as i32;
            let mut b = bands[i].lock();
            let ya = by.max(c.y0);
            let yb = (by + 64).min(c.y1);
            tg_band_rows(&c, ya, yb, |y| ((y - by) as usize * stride + c.x0 as usize * 4, 4), &mut b);
        };
        let _ = plat::parallel_for(nb, &item);
        let mode = if p.tool.cfg.blend < BM_COUNT { p.tool.cfg.blend } else { 0 };
        let tl = &mut p.tool.tls[TL_GRADIENT];
        tl.kind = TLK_SOURCE as u32;
        tl.src = TlSrc::Grad;
        tl.sx = 0;
        tl.sy = 0;
        tl.mask = TlMask::None;
        tl.mode = mode | 255 << BM_OPAC_SHIFT;
        return true;
    }
    // transparency: a coverage mask of the image size, the box's tiles materialised
    let t = &mut p.tgrad;
    if !matches!(&t.mask, Some(m) if m.w as i32 == w && m.h as i32 == h) {
        t.mask = None;
        p.tool.tls[TL_GRADIENT].mask = TlMask::None;
        t.mask = TMask::new(w as u32, h as u32);
    }
    let Some(m) = t.mask.as_mut() else { return false };
    m.clear(0);
    let bw = (c.x1 - c.x0) as usize;
    let bufs: Vec<SpinLock<Vec<u8>>> = (0..nb).map(|_| SpinLock::new(Vec::new())).collect();
    let item = |i: usize, _: &plat::JobCtl| {
        let by = ((ty0 + i) << 6) as i32;
        let ya = by.max(c.y0);
        let yb = (by + 64).min(c.y1);
        let mut b = vec![0u8; (yb - ya) as usize * bw];
        tg_band_rows(&c, ya, yb, |y| ((y - ya) as usize * bw, 1), &mut b);
        *bufs[i].lock() = b;
    };
    let _ = plat::parallel_for(nb, &item);
    for ty in (c.y0 >> 6)..=((c.y1 - 1) >> 6) {
        for tx in (c.x0 >> 6)..=((c.x1 - 1) >> 6) {
            let Some(tile) = m.tile_w(tx as u32, ty as u32) else { return false };
            let ya = (ty << 6).max(c.y0);
            let yb = ((ty << 6) + 64).min(c.y1);
            let xa = (tx << 6).max(c.x0);
            let xb = ((tx << 6) + 64).min(c.x1);
            let b = bufs[ty as usize - ty0].lock();
            let band_y0 = (ty << 6).max(c.y0);
            for y in ya..yb {
                let src = (y - band_y0) as usize * bw + (xa - c.x0) as usize;
                let dst = ((y & 63) << 6) as usize + (xa & 63) as usize;
                let n = (xb - xa) as usize;
                tile[dst..dst + n].copy_from_slice(&b[src..src + n]);
            }
        }
    }
    m.optimize();
    let tl = &mut p.tool.tls[TL_GRADIENT];
    tl.kind = TLK_ERASE as u32;
    tl.strength = 255;
    tl.mask = TlMask::Grad;
    tl.src = TlSrc::None;
    true
}

// ------------------------------------------------------------------ TOOLDESC callbacks

// tg_input(doc, ui, x_fx, y_fx) -> changed
fn tg_input(p: &mut Pn, ui: &mut Ui, doc: DocId, inp: &UiInput, x: i32, y: i32) -> bool {
    let msx = inp.x + inp.rect[0];
    let msy = inp.y + inp.rect[1];
    let t = &p.tgrad;
    if t.drag != DRG_NONE && t.live {
        if Some(doc) != t.doc {
            return false;
        }
        if inp.btn & (1 << t.dragbtn) == 0 {
            // released
            let d = t.drag;
            let t = &mut p.tgrad;
            t.drag = DRG_NONE;
            for n in t.nubs.iter_mut() {
                n.flags &= !(NBF_DRAG as u32);
            }
            nubs_damage(&p.view, ui, &mut t.nubs);
            if d == DRG_NEW && !p.tool.tls[TL_GRADIENT].shown {
                tool_cancel_live(p, ui); // a click without a drag: no gradient
            }
            return true;
        }
        // dragging: the new end points
        let (mut nsx, mut nsy, mut nex, mut ney) = (t.sx, t.sy, t.ex, t.ey);
        let dx = x.wrapping_sub(t.ax); // pointer delta
        let dy = y.wrapping_sub(t.ay);
        let shift = inp.mods & MOD_SHIFT != 0;
        match t.drag {
            DRG_NEW => {
                let (mut px, mut py) = (x, y);
                if shift {
                    (px, py) = tg_snap(nsx, nsy, px, py);
                }
                nex = px;
                ney = py;
                // nothing is shown while the end is still at the start point
                if !p.tool.tls[TL_GRADIENT].shown && px == nsx && py == nsy {
                    return false;
                }
            }
            DRG_MID => {
                nsx = t.osx.wrapping_add(dx);
                nsy = t.osy.wrapping_add(dy);
                nex = t.oex.wrapping_add(dx);
                ney = t.oey.wrapping_add(dy);
            }
            DRG_S => {
                let (mut px, mut py) = (t.osx.wrapping_add(dx), t.osy.wrapping_add(dy));
                if shift {
                    (px, py) = tg_snap(nex, ney, px, py);
                }
                nsx = px;
                nsy = py;
            }
            _ => {
                // end point (Shift: angle around the start point)
                let (mut px, mut py) = (t.oex.wrapping_add(dx), t.oey.wrapping_add(dy));
                if shift {
                    (px, py) = tg_snap(nsx, nsy, px, py);
                }
                nex = px;
                ney = py;
            }
        }
        if nsx == t.sx && nsy == t.sy && nex == t.ex && ney == t.ey && p.tool.tls[TL_GRADIENT].shown {
            return false;
        }
        let t = &mut p.tgrad;
        t.sx = nsx;
        t.sy = nsy;
        t.ex = nex;
        t.ey = ney;
        tg_show_nubs(p, ui);
        tg_render(p);
        return true;
    }
    let btn = if inp.pressed & (1 << BTN_LEFT) != 0 {
        BTN_LEFT
    } else if inp.pressed & (1 << BTN_RIGHT) != 0 {
        BTN_RIGHT
    } else {
        // hover
        let t = &mut p.tgrad;
        if !t.live || Some(doc) != t.doc {
            return false;
        }
        return nubs_hover(&p.view, ui, &mut t.nubs, msx, msy);
    };
    if p.tgrad.live {
        if Some(doc) == p.tgrad.doc {
            let k = nubs_hit(&p.view, ui, &p.tgrad.nubs, msx, msy);
            if k >= 0 {
                let t = &mut p.tgrad;
                if btn == BTN_RIGHT {
                    // right click on a nub: the colours swap
                    t.btn ^= BTN_LEFT ^ BTN_RIGHT;
                    tg_render(p);
                    return true;
                }
                t.drag = k as u32 + 1;
                t.dragbtn = btn;
                t.ax = x;
                t.ay = y;
                t.osx = t.sx;
                t.osy = t.sy;
                t.oex = t.ex;
                t.oey = t.ey;
                t.nubs[k as usize].flags |= NBF_DRAG as u32;
                nubs_damage(&p.view, ui, &mut t.nubs);
                return true;
            }
        }
        // a press away from the nubs: the gradient is finished, a new one starts
        tool_commit_live(p, ui);
    }
    let t = &mut p.tgrad;
    t.sx = x;
    t.sy = y;
    t.ex = x;
    t.ey = y;
    tg_begin(p, doc, btn);
    p.tgrad.drag = DRG_NEW;
    p.tgrad.dragbtn = btn;
    true
}

// tg_tick(doc): hover left the canvas; colour / option / selection changes re-render
fn tg_tick(p: &mut Pn, ui: &mut Ui, doc: Option<DocId>) {
    let Some(doc) = doc else { return };
    if !p.tgrad.live || Some(doc) != p.tgrad.doc {
        return;
    }
    let t = &mut p.tgrad;
    let f = t.nubs.iter().fold(0, |a, n| a | n.flags);
    if f & NBF_HOVER as u32 != 0 && !view_cursor(&p.view).2 {
        for n in t.nubs.iter_mut() {
            n.flags &= !(NBF_HOVER as u32);
        }
        nubs_damage(&p.view, ui, &mut t.nubs);
    }
    if !p.tool.tls[TL_GRADIENT].shown {
        return;
    }
    let t = &p.tgrad;
    if p.tool.cfg.ver != t.pver || pal_version(&p.pal) != t.palv || p.doc.d(doc).ver_sel != t.vsel {
        tg_render(p);
    }
}

// tg_commit(doc) -> step: the gradient into the layer ("Farbverlauf")
fn tg_commit(p: &mut Pn, ui: &mut Ui, _doc: DocId) -> bool {
    if !p.tgrad.live {
        return false;
    }
    let mut r = false;
    if p.tool.tls[TL_GRADIENT].shown {
        r = tl_commit(p, TL_GRADIENT, "Farbverlauf", IC_CONTRAST);
    }
    tg_end(p, ui);
    r
}

// tg_cancel(doc): drop the live gradient (no history; doc None = the image is gone)
fn tg_cancel(p: &mut Pn, ui: &mut Ui, doc: Option<DocId>) {
    if !p.tgrad.live {
        return;
    }
    if doc.is_some() && p.tool.tls[TL_GRADIENT].shown {
        tl_hide(p, TL_GRADIENT);
    }
    p.tool.tls[TL_GRADIENT].shown = false;
    tg_end(p, ui);
}

// tg_cursor(x_fx, y_fx) -> CUR_MOVE over / while dragging a nub, else the crosshair
fn tg_cursor(p: &mut Pn, _x: i32, _y: i32) -> u32 {
    let t = &p.tgrad;
    if !t.live || t.drag == DRG_NEW {
        return CUR_CROSS;
    }
    let f = t.nubs.iter().fold(0, |a, n| a | n.flags);
    if f & (NBF_HOVER | NBF_DRAG) as u32 != 0 { CUR_MOVE } else { CUR_CROSS }
}

// tg_overlay(doc, &rect): the nubs of the live gradient
fn tg_overlay(p: &mut Pn, g: &mut Gfx, doc: DocId, _rect: [i32; 4]) {
    if p.tgrad.live && Some(doc) == p.tgrad.doc {
        nubs_draw(&p.view, g, &mut p.tgrad.nubs);
    }
}

// tg_options(): type, Farbverlauf / Transparenzverlauf, repeat, Füllmethode, Antialiasing
// (the framework adds Fertigstellen); the hidden test records
fn tg_options(p: &mut Pn, ui: &mut Ui) {
    let mut v = p.tgrad.typ;
    opt_menu(p, ui, ID_OPT_GRAD0, &mut v, &TYPE_LABELS, &[], None, OMF_NOICON as u32);
    p.tgrad.typ = v;
    let mut v = p.tgrad.alpha;
    opt_menu(p, ui, ID_TG_MODE, &mut v, &MODE_LABELS, &[], None, OMF_NOICON as u32);
    p.tgrad.alpha = v;
    let mut v = p.tgrad.rep;
    opt_menu(p, ui, ID_TG_REP, &mut v, &REP_LABELS, &[], None, OMF_NOICON as u32);
    p.tgrad.rep = v;
    opt_sep(ui);
    opt_blend(p, ui);
    opt_aa(p, ui);
    tg_records(p, ui);
}

// tg_records(): ID_TG_REC "sx,sy,ex,ey" (value state), ID_TG_REC2 options (value drag)
fn tg_records(p: &Pn, ui: &mut Ui) {
    let t = &p.tgrad;
    let s = format!("{},{},{},{}", t.sx >> 8, t.sy >> 8, t.ex >> 8, t.ey >> 8);
    let v = if t.live { 1 + p.tool.tls[TL_GRADIENT].shown as i32 } else { 0 };
    t1_sem(ui, ID_TG_REC, &s, v);
    let s = format!("{},{},{},{},{}", t.typ as i32, t.rep as i32, t.alpha as i32, (t.btn == BTN_RIGHT) as i32, t.ctx.aa as i32);
    t1_sem(ui, ID_TG_REC2, &s, t.drag as i32);
}

// ------------------------------------------------------------------ test hooks

/// tgrad_cmd(id, arg) -> 1 handled -- 792 options, 793 start point, 794 end point
pub(crate) fn tgrad_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    if id == CMD_TGRAD_FIRST {
        let t = &mut p.tgrad;
        let ty = arg & 255;
        t.typ = if ty < TG_NTYPES { ty } else { 0 };
        let r = (arg >> 8) & 255;
        t.rep = if r < TG_NREP { r } else { 0 };
        t.alpha = (arg >> 16) & 1;
        tool_cfg_changed(p);
        return true;
    }
    if id == CMD_TGRAD_FIRST + 1 {
        let t = &mut p.tgrad;
        t.hx = (arg & 0xFFFF) as i32;
        t.hy = ((arg >> 16) & 0x7FFF) as i32;
        t.hbtn = if (arg as i32) < 0 { BTN_RIGHT } else { BTN_LEFT };
        return true;
    }
    if id != CMD_TGRAD_FIRST + 2 {
        return false;
    }
    if tool_cur(&p.tool) != TOOL_GRADIENT {
        return true;
    }
    let Some(doc) = doc_active(&p.doc) else { return true };
    let mut new = !p.tgrad.live;
    if p.tgrad.live && Some(doc) != p.tgrad.doc {
        tool_commit_live(p, ui);
        new = true;
    }
    if new {
        let hb = p.tgrad.hbtn;
        tg_begin(p, doc, hb);
    }
    let t = &mut p.tgrad;
    t.btn = t.hbtn;
    t.sx = (t.hx << 8) + 128;
    t.sy = (t.hy << 8) + 128;
    t.ex = (((arg & 0xFFFF) as i32) << 8) + 128;
    t.ey = ((((arg >> 16) & 0x7FFF) as i32) << 8) + 128;
    tg_show_nubs(p, ui);
    tg_render(p);
    ui.repass();
    true
}
