// render_par -- large redraws on several threads                         (GPU plan, stage 0)
//
//   ui.render_par(n) -> drawn     render_live's hook: the n dirty rectangles of drects (-1 =
//                                 the whole window) drawn here, or false: render_live's loop
//                                 draws them on this thread
//
// When: jfgui_RENDER_PAR=0 never, =1 every render whatever its size (also without worker
// threads), =2 every render twice, on this thread and in bands, a byte that differs is
// warned (the identity tests); else when the worker pool has threads
// (none in the browser without shared memory and on Android: plat::thr_count() == 0) and at
// least PAR_MIN_PX pixels are dirty.
//
// How: the framebuffer's rows are cut into bands of about BAND_BYTES (fewer rows when there
// would be fewer than BANDS_PER_THREAD per thread).  A worker (plat::parallel_for) copies a
// band's dirty part into a buffer of its own (not when an opaque fill is the first command
// over all of a rectangle), draws the band's commands there (Dc::begin_rows: coordinates stay
// the window's, so paths, the rasterizer and glyph positions compute what one thread
// computes) and copies it back past the caches (gfx::blend::copy_stream; the next band then
// still finds its buffer in the cache).  The rasterizer's coverage of a pixel does not
// depend on the clip, panel_fx's mask bands start at the panel, blending is per pixel: a
// band gets the bytes the single-threaded loop gives.
//
// The workers see plain data only.  This thread first walks the commands (scan): a copy of
// each visible one without its Rc (an image's surface is borrowed) with the clip its
// layer's clip records give it, listed in the bands it reaches; every visible text and font
// icon placed with Text::record_n / record_icon (shaping, fallback fonts and glyph masks into
// the cache now; the workers blend from the cache read-only, text::GlyphView).  What only
// this thread may run: custom draws (Draw is not Sync and may write the framebuffer
// directly), icons that an icon hook or the style paints (asked with an empty clip, which
// draws nothing), texts with glyphs the cache cannot hold.  The workers draw such a band up
// to the first of them, this thread the rest afterwards (render_from, the clip cut to the
// bands).  A glyph cache flushed while placing (it filled up) or rectangles that overlap
// (canvas damage) leave the whole render to the loop.  Either way render_live sets the text
// font the passes left set again afterwards.

use alloc::vec::Vec;

use gfx::blend::{copy_stream, stream_fence};
use gfx::path::Path;
use gfx::raster::Raster;
use gfx::shapes::Shapes;
use gfx::{Dc, Surface};
use rt::sync::SpinLock;
use text::{GlyphRec, GlyphView, Text};

use super::cmd::{cmd_bbox, draw_rrect, draw_shadow, Cmd, Gfx, IconHook};
use super::consts::*;
use super::{Style, Ui};

/// auto mode: fewer dirty pixels stay on this thread -- about 1.5 x 1920 x 1080 while the
/// framebuffer fits the caches (a full HD frame takes 0.6 .. 1.7 ms on one thread, the copies
/// and waking the workers eat most of the gain), a quarter of that in a larger one (4K: the
/// thread waits for memory)
const PAR_MIN_PX: i64 = 3_000_000;
const PAR_MIN_PX_BIG: i64 = 750_000;
/// framebuffer bytes from which PAR_MIN_PX_BIG applies
const BIG_FB: usize = 16 << 20;
/// framebuffer bytes of a band (it stays in the worker's cache while its layers are drawn)
const BAND_BYTES: usize = 256 * 1024;
/// bands per thread at least (the pool hands them out as threads get free)
const BANDS_PER_THREAD: usize = 4;
const BAND_MIN_ROWS: usize = 8;
/// framebuffers from this size on (more than the caches keep beside the bands) get their
/// bands back with streaming stores
const STREAM_BYTES: usize = 4 << 20;

const M_UNKNOWN: u8 = 0;
const M_OFF: u8 = 1;
const M_AUTO: u8 = 2;
const M_ALWAYS: u8 = 3;
const M_CHECK: u8 = 4;

/// render_par's state (Mods): the mode, the workers' band buffers, the glyph record
#[derive(Default)]
pub(crate) struct S {
    mode: u8,
    bufs: Vec<Band>,
    rec: GlyphRec,
}

// a worker's band: the buffer, its draw context, path and rasterizer
struct Band {
    sf: Surface,
    dc: Dc,
    path: Path,
    ras: Raster,
}

impl Band {
    fn new() -> Band {
        Band { sf: Surface::default(), dc: Dc::new(0, 0), path: Path::default(), ras: Raster::default() }
    }
}

// a command as the workers replay it (render_layer's drawing cases without Rc)
#[derive(Clone, Copy)]
enum P<'a> {
    Rect { x: i32, y: i32, w: i32, h: i32, c: u32 },
    RRect { b: [i32; 4], r: i32, t: i32, fill: u32, stroke: u32, stroke2: u32 },
    PanelFx { b: [i32; 4], r: i32, t: i32, fill: u32, stroke: u32 },
    /// recorded glyphs from .. to of a text (maxw != 0: clipped to x .. x + maxw) or font icon
    Glyphs { x: i32, maxw: i32, from: u32, to: u32 },
    Image { b: [i32; 4], sf: &'a Surface },
    Circle { x: i32, y: i32, r: i32, t: i32, c: u32 },
    Line { x0: i32, y0: i32, x1: i32, y1: i32, w: i32, c: u32 },
    Shadow { b: [i32; 4], r: i32, d: i32, c: u32 },
}

// a replayed command: its box (cmd_bbox), the clip of its layer's clip records at that point
// (x0, y0, x1, y1; the dc's clip is that cut to the rectangle and the band), the part of the
// window it can change (rows: the bands that list it)
#[derive(Clone, Copy)]
struct Pc<'a> {
    bb: [i32; 4],
    clip: [i32; 4],
    eff: [i32; 4],
    p: P<'a>,
}

// a command only this thread may draw: the part of the window it can change, its place (pos:
// the replayed commands before it in list; layer and index: render_from's start)
#[derive(Clone, Copy)]
struct Serial {
    b: [i32; 4],
    pos: usize,
    layer: usize,
    index: usize,
}

// what render_layer's icons need besides the command (style, accent, dark, disabled, dpi)
type IconStyle = (&'static dyn Style, u32, bool, u32, u32);

impl S {
    // the mode from jfgui_RENDER_PAR (read once)
    fn mode(&mut self) -> u8 {
        if self.mode == M_UNKNOWN {
            self.mode = match plat::getenv("jfgui_RENDER_PAR").as_deref().map(str::as_bytes) {
                Some([b'0', ..]) => M_OFF,
                Some([b'1', ..]) => M_ALWAYS,
                Some([b'2', ..]) => M_CHECK,
                _ => M_AUTO,
            };
        }
        self.mode
    }
}

// x0, y0, x1, y1 boxes
fn meet(a: [i32; 4], b: [i32; 4]) -> Option<[i32; 4]> {
    let r = [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])];
    (r[0] < r[2] && r[1] < r[3]).then_some(r)
}

// cmd_visible: the box touches the clip
fn touches(b: [i32; 4], k: [i32; 4]) -> bool {
    !(b[0] >= k[2] || b[2] <= k[0] || b[1] >= k[3] || b[3] <= k[1])
}

impl Ui {
    /// render_live's hook: the dirty rectangles (n; -1 = the whole window) drawn on several
    /// threads -> false when the work is small, render_par is off, or only this thread may
    /// draw it (render_live's loop draws it then)
    pub(crate) fn render_par(&mut self, n: i32) -> bool {
        match self.m.rpar.mode() {
            M_CHECK => self.render_check(n),
            mode => self.render_bands(n, mode),
        }
    }

    // jfgui_RENDER_PAR=2: render_live's loop, then the bands from the same pixels; warn when
    // a byte differs
    fn render_check(&mut self, n: i32) -> bool {
        if n == 0 {
            return false;
        }
        let before = self.wp.rt.fb.px.clone();
        let font = self.text.get_font();
        if n < 0 {
            self.render_all_to(false);
        } else {
            for i in 0..n as usize {
                let r = self.drects[i];
                self.dc.clip_push(r[0], r[1], r[2], r[3]);
                self.render_all_to(false);
                self.dc.clip_pop();
            }
        }
        let want = core::mem::replace(&mut self.wp.rt.fb.px, before);
        self.text.set_font(font.0, font.1); // as the bands find it
        if !self.render_bands(n, M_ALWAYS) {
            self.wp.rt.fb.px = want; // declined: the loop's pixels
            return true;
        }
        let diff = want.iter().zip(self.wp.rt.fb.px.iter()).filter(|(a, b)| a != b).count();
        if diff != 0 {
            let (w, h) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
            self.warn(&alloc::format!("render_par: {} bytes differ from one thread's render ({}x{}, {} rects)", diff, w, h, n));
        }
        true
    }

    // the bands (mode M_AUTO or M_ALWAYS) -> drawn
    fn render_bands(&mut self, n: i32, mode: u8) -> bool {
        if mode == M_OFF || n == 0 {
            return false;
        }
        let (w, h) = (self.wp.rt.fb.w, self.wp.rt.fb.h);
        // the rectangles as x0, y0, x1, y1 inside the window
        let rects: Vec<[i32; 4]> = if n < 0 {
            alloc::vec![[0, 0, w, h]]
        } else {
            let all = [0, 0, w, h];
            self.drects.iter().take(n as usize).filter_map(|r| meet([r[0], r[1], r[0].wrapping_add(r[2]), r[1].wrapping_add(r[3])], all)).collect()
        };
        let area: i64 = rects.iter().map(|r| (r[2] - r[0]) as i64 * (r[3] - r[1]) as i64).sum();
        let big = h.max(0) as usize * self.wp.rt.fb.stride.max(0) as usize >= BIG_FB;
        let min_px = if big { PAR_MIN_PX_BIG } else { PAR_MIN_PX };
        if rects.is_empty() || (mode == M_AUTO && (area < min_px || plat::thr_count() == 0)) {
            return false;
        }
        // overlapping rectangles: the loop draws the overlap twice, in its order
        for (i, a) in rects.iter().enumerate() {
            if rects[i + 1..].iter().any(|b| meet(*a, *b).is_some()) {
                return false;
            }
        }
        let icon_style: IconStyle =
            (self.style, self.color(T_ACCENT), self.theme_dark, self.color(T_TEXT_DISABLED), self.wp.rt.dpi as u32);
        // ---- bands: about BAND_BYTES each, enough of them for the threads
        let threads = plat::thr_count() as usize + 1;
        let stride = self.wp.rt.fb.stride.max(1) as usize;
        let span = (rects.iter().map(|r| r[3]).max().unwrap_or(0) - rects.iter().map(|r| r[1]).min().unwrap_or(0)) as usize;
        let mut bh = (BAND_BYTES / stride).clamp(BAND_MIN_ROWS, 256);
        if span / bh < threads * BANDS_PER_THREAD {
            bh = (span / (threads * BANDS_PER_THREAD)).max(BAND_MIN_ROWS);
        }
        let nb = (h as usize).div_ceil(bh);
        let band_of = |y: i32| y.max(0) as usize / bh;
        let t0 = plat::time_us();
        let (pbands, first, t1) = {
            let Ui { wp, dc, text, icon_hook, m, .. } = self;
            let st = &mut m.rpar;
            let layers = &wp.cmd.layers;
            let fb = &mut wp.rt.fb;
            // ---- the commands as the workers see them; what stays on this thread
            st.rec.begin(text);
            let (list, sbox) = scan(layers, &rects, text, dc, fb, &mut st.rec, icon_hook, icon_style);
            if !st.rec.valid(text) {
                return false; // the glyph cache filled up while placing: masks are gone
            }
            // bands with a dirty part; first: the earliest command there that only this thread
            // may draw (the workers draw the band up to it, this thread from it on)
            let mut used = alloc::vec![false; nb];
            let mut first: Vec<Option<Serial>> = alloc::vec![None; nb];
            for r in &rects {
                used[band_of(r[1])..=band_of(r[3] - 1)].fill(true);
                for s in &sbox {
                    if let Some(c) = meet(s.b, *r) {
                        for f in &mut first[band_of(c[1])..=band_of(c[3] - 1)] {
                            if f.is_none_or(|f| (s.layer, s.index) < (f.layer, f.index)) {
                                *f = Some(*s);
                            }
                        }
                    }
                }
            }
            // each band's commands (indexes into list, in order): start[k] .. start[k + 1]
            let rows = |c: &Pc| (band_of(c.eff[1]).min(nb - 1), band_of(c.eff[3] - 1).min(nb - 1));
            let mut start = alloc::vec![0usize; nb + 1];
            for c in &list {
                let (b0, b1) = rows(c);
                for s in &mut start[b0 + 1..=b1 + 1] {
                    *s += 1;
                }
            }
            for k in 0..nb {
                start[k + 1] += start[k];
            }
            let mut idx = alloc::vec![0u32; start[nb]];
            let mut fill = start.clone();
            for (i, c) in list.iter().enumerate() {
                let (b0, b1) = rows(c);
                for f in &mut fill[b0..=b1] {
                    idx[*f] = i as u32;
                    *f += 1;
                }
            }
            // what the workers draw of a band: its commands before the first serial one
            let ends: Vec<usize> = (0..nb)
                .map(|k| {
                    let band = &idx[start[k]..start[k + 1]];
                    start[k] + first[k].map_or(band.len(), |s| band.partition_point(|&i| (i as usize) < s.pos))
                })
                .collect();
            let pbands: Vec<usize> = (0..nb).filter(|&k| used[k] && ends[k] > start[k]).collect();
            if pbands.is_empty() {
                return false; // nothing for the workers
            }
            // a rectangle under an opaque fill as the first command needs no copy in
            let bg = layers[0].first();
            let opac = dc.opac;
            let covered: Vec<bool> = rects
                .iter()
                .map(|r| match bg {
                    Some(&Cmd::Rect { x, y, w, h, c }) if c >> 24 == 255 && opac == 255 => {
                        x <= r[0] && y <= r[1] && x.wrapping_add(w) >= r[2] && y.wrapping_add(h) >= r[3]
                    }
                    _ => false,
                })
                .collect();
            // ---- the bands on the pool's threads
            let t1 = plat::time_us();
            let (fw, fh, fmt) = (fb.w, fb.h, fb.fmt);
            let stream = fh as usize * stride >= STREAM_BYTES;
            let px = &mut fb.px[..fh as usize * stride];
            let chunks: Vec<SpinLock<&mut [u8]>> = px.chunks_mut(bh * stride).map(SpinLock::new).collect();
            let pool = SpinLock::new(core::mem::take(&mut st.bufs));
            let gv = text.glyphs();
            let (rec, list, rects, pbands, idx, start, ends) = (&st.rec, &list, &rects, &pbands, &idx, &start, &ends);
            let job = |i: usize, _: &plat::JobCtl| {
                let Some(&k) = pbands.get(i) else { return };
                let Some(cell) = chunks.get(k) else { return };
                let mut chunk = cell.lock();
                let mut b = pool.lock().pop().unwrap_or_else(Band::new);
                if b.sf.w != fw || b.sf.h != bh as i32 || b.sf.fmt != fmt {
                    b.sf = Surface::new(fw, bh as i32, fmt);
                }
                let y0 = (k * bh) as i32;
                let y1 = (y0 + bh as i32).min(fh);
                let bs = b.sf.stride as usize;
                // the band's part of each rectangle: rows, then bytes of a row
                let parts = rects.iter().zip(covered.iter()).filter_map(|(r, &cov)| {
                    meet(*r, [r[0], y0, r[2], y1]).map(|c| (c, cov))
                });
                for (c, cov) in parts.clone() {
                    if cov {
                        continue;
                    }
                    let (x0, x1) = (c[0] as usize * 4, c[2] as usize * 4);
                    for y in (c[1] - y0) as usize..(c[3] - y0) as usize {
                        b.sf.px[y * bs + x0..y * bs + x1].copy_from_slice(&chunk[y * stride + x0..y * stride + x1]);
                    }
                }
                let Band { sf, dc, path, ras } = &mut b;
                dc.begin_rows(sf, y0, fh);
                dc.set_opacity(opac);
                for (c, _) in parts.clone() {
                    replay(list, &idx[start[k]..ends[k]], c, rec, &gv, dc, sf, path, ras);
                }
                dc.end();
                // back; past the caches when the framebuffer does not fit them anyway (they keep
                // the band then, and the framebuffer is not read first)
                for (c, _) in parts {
                    let (x0, x1) = (c[0] as usize * 4, c[2] as usize * 4);
                    for y in (c[1] - y0) as usize..(c[3] - y0) as usize {
                        let (d, s) = (&mut chunk[y * stride + x0..y * stride + x1], &sf.px[y * bs + x0..y * bs + x1]);
                        if stream {
                            copy_stream(d, s);
                        } else {
                            d.copy_from_slice(s);
                        }
                    }
                }
                if stream {
                    stream_fence();
                }
                pool.lock().push(b);
            };
            let _ = plat::parallel_for(pbands.len(), &job);
            st.bufs = core::mem::take(&mut *pool.lock());
            (pbands.len(), first, t1)
        };
        // ---- the rest of the serial bands here, from their first serial command on (runs of
        // bands with the same one at once)
        let t2 = plat::time_us();
        let mut k = 0;
        let mut runs = 0;
        while k < nb {
            let Some(s) = first[k] else {
                k += 1;
                continue;
            };
            let k0 = k;
            while k < nb && first[k].is_some_and(|f| (f.layer, f.index) == (s.layer, s.index)) {
                k += 1;
            }
            let rows = [0, (k0 * bh) as i32, w, ((k * bh) as i32).min(h)];
            for r in &rects {
                if let Some(c) = meet(*r, rows) {
                    self.dc.clip_push(c[0], c[1], c[2] - c[0], c[3] - c[1]);
                    self.render_from(false, s.layer, s.index);
                    self.dc.clip_pop();
                    runs += 1;
                }
            }
        }
        if self.ui_debug {
            let t3 = plat::time_us();
            let sb = first.iter().filter(|f| f.is_some()).count();
            let s = alloc::format!(
                "render: {} rects in {} bands of {} rows on {} threads ({} us; placing {} us), {} serial passes ({} us) over {} of {} bands\n",
                rects.len(), pbands, bh, threads, t2 - t1, t1 - t0, runs, t3 - t2, sb, nb);
            plat::write_stderr(s.as_bytes());
        }
        true
    }
}

// the commands of every layer for the workers -> (the list, boxes only this thread may draw);
// places the visible texts and font icons into rec
#[allow(clippy::too_many_arguments)]
fn scan<'a>(layers: &'a [Vec<Cmd>; L_COUNT], rects: &[[i32; 4]], text: &mut Text, dc: &mut Dc, fb: &mut Surface,
            rec: &mut GlyphRec, hook: &Option<IconHook>, icon_style: IconStyle) -> (Vec<Pc<'a>>, Vec<Serial>) {
    const ALL: [i32; 4] = [i32::MIN, i32::MIN, i32::MAX, i32::MAX];
    let mut list = Vec::new();
    let mut sbox = Vec::new();
    let mut stack: Vec<[i32; 4]> = Vec::new();
    for (l, layer) in layers.iter().enumerate() {
        // the layer's clip records as render_layer pushes them (dc.clip_push; the rectangle
        // and the band are cut in at replay)
        let mut clip = ALL;
        stack.clear();
        for (i, c) in layer.iter().enumerate() {
            let serial = |b: [i32; 4], pos: usize| Serial { b, pos, layer: l, index: i };
            match *c {
                Cmd::Clip { x, y, w, h } => {
                    stack.push(clip);
                    let (x0, y0) = (x.max(clip[0]), y.max(clip[1]));
                    clip = [x0, y0, x.wrapping_add(w).min(clip[2]).max(x0), y.wrapping_add(h).min(clip[3]).max(y0)];
                    continue;
                }
                Cmd::Unclip => {
                    if let Some(k) = stack.pop() {
                        clip = k;
                    }
                    continue;
                }
                _ => {}
            }
            let Some(bb) = cmd_bbox(c) else { continue };
            // the part that can change pixels in the dirty rectangles
            let Some(eff) = meet(bb, clip).filter(|e| rects.iter().any(|r| meet(*e, *r).is_some())) else { continue };
            let p = match *c {
                Cmd::Rect { x, y, w, h, c } => P::Rect { x, y, w, h, c },
                Cmd::RRect { x, y, w, h, r, t, fill, stroke, stroke2 } => {
                    P::RRect { b: [x, y, w, h], r: r as i32, t: t as i32, fill, stroke, stroke2 }
                }
                Cmd::PanelFx { x, y, w, h, r, t, fill, stroke } => P::PanelFx { b: [x, y, w, h], r, t, fill, stroke },
                Cmd::Text { x, y, font, size, c, maxw, ref s } => {
                    dc.set_color(c);
                    text.set_font(font as u32, size);
                    let from = rec.len() as u32;
                    if !text.record_n(rec, dc, fb, x, y, s, s.len()) {
                        sbox.push(serial(grow(eff, (size >> 8) + 2), list.len())); // a glyph too large for the cache
                        continue;
                    }
                    P::Glyphs { x, maxw, from, to: rec.len() as u32 }
                }
                Cmd::Icon { x, y, cp, size, c } => {
                    if icon_painted(hook, icon_style, text, dc, fb, cp, x, y, size, c) {
                        sbox.push(serial(grow(eff, (size >> 10) + 2), list.len())); // drawn by app or style code
                        continue;
                    }
                    dc.set_color(c);
                    let from = rec.len() as u32;
                    if !text.record_icon(rec, dc, fb, cp, x, y, size) {
                        sbox.push(serial(grow(eff, (size >> 8) + 2), list.len()));
                        continue;
                    }
                    P::Glyphs { x: 0, maxw: 0, from, to: rec.len() as u32 }
                }
                Cmd::Image { x, y, w, h, ref sf, .. } => P::Image { b: [x, y, w, h], sf },
                Cmd::Circle { x, y, r, t, c } => P::Circle { x, y, r, t, c },
                Cmd::Line { x0, y0, x1, y1, w, c } => P::Line { x0, y0, x1, y1, w, c },
                Cmd::Shadow { x, y, w, h, r, depth, c } => P::Shadow { b: [x, y, w, h], r, d: depth, c },
                Cmd::Custom { .. } => {
                    sbox.push(serial(eff, list.len())); // clipped to its rectangle by render_layer
                    continue;
                }
                Cmd::Clip { .. } | Cmd::Unclip => continue,
            };
            list.push(Pc { bb, clip, eff, p });
        }
    }
    (list, sbox)
}

// a box grown by m px on every side
fn grow(b: [i32; 4], m: i32) -> [i32; 4] {
    [b[0].saturating_sub(m), b[1].saturating_sub(m), b[2].saturating_add(m), b[3].saturating_add(m)]
}

// does the icon hook or the style paint this icon (render_layer asks them first)?  Asked
// with an empty clip: they draw nothing
#[allow(clippy::too_many_arguments)]
fn icon_painted(hook: &Option<IconHook>, icon_style: IconStyle, text: &mut Text, dc: &mut Dc, fb: &mut Surface, cp: u32, x: i32,
                y: i32, size: i32, c: u32) -> bool {
    dc.clip_push(0, 0, 0, 0);
    let (tx, ty) = (x.wrapping_add(dc.ox), y.wrapping_add(dc.oy));
    let mut painted = false;
    if let Some(f) = hook {
        painted = f(&mut Gfx { dc: &mut *dc, sf: &mut *fb, text: &mut *text }, cp, tx, ty, size, c);
    }
    if !painted {
        let (style, accent, dark, disabled, dpi) = icon_style;
        let mut g = Gfx { dc: &mut *dc, sf: &mut *fb, text: &mut *text };
        painted = style.paint_icon(&mut g, cp, tx, ty, size, c, accent, dark, c == disabled, dpi);
    }
    dc.clip_pop();
    painted
}

// render_layer for the workers: the band's commands (idx into list, in order) inside rb (a
// rectangle cut to the band; dc began the band).  The dc's clip is each command's clip cut to
// rb -- what render_layer's clip stack holds there, wherever it can draw.
#[allow(clippy::too_many_arguments)]
fn replay(list: &[Pc], idx: &[u32], rb: [i32; 4], rec: &GlyphRec, gv: &GlyphView, dc: &mut Dc, sf: &mut Surface, path: &mut Path,
          ras: &mut Raster) {
    for &i in idx {
        let c = &list[i as usize];
        let Some(k) = meet(c.clip, rb) else { continue };
        if !touches(c.bb, k) {
            continue;
        }
        dc.clip = k;
        let mut sh = Shapes { dc: &mut *dc, sf: &mut *sf, path: &mut *path, ras: &mut *ras };
        match c.p {
            P::Rect { x, y, w, h, c } => {
                sh.dc.set_color(c);
                sh.dc.fill(sh.sf, x, y, w, h);
            }
            P::RRect { b, r, t, fill, stroke, stroke2 } => draw_rrect(&mut sh, b, r, t, fill, stroke, stroke2),
            P::PanelFx { b, r, t, fill, stroke } => sh.panel_fx(b[0], b[1], b[2], b[3], r, t, fill, stroke),
            P::Glyphs { x, maxw, from, to } => {
                if maxw != 0 {
                    sh.dc.clip_push(x, -100000, maxw, 200000);
                }
                gv.draw(rec, from as usize, to as usize, sh.dc, sh.sf);
                if maxw != 0 {
                    sh.dc.clip_pop();
                }
            }
            P::Image { b, sf: src } => sh.dc.blit_scaled(sh.sf, src, b[0], b[1], b[2], b[3]),
            P::Circle { x, y, r, t, c } => {
                sh.dc.set_color(c);
                if t != 0 {
                    sh.stroke_circle_fx(x, y, r, t);
                } else {
                    sh.fill_circle_fx(x, y, r);
                }
            }
            P::Line { x0, y0, x1, y1, w, c } => {
                sh.dc.set_color(c);
                sh.line_fx(x0, y0, x1, y1, w);
            }
            P::Shadow { b, r, d, c } => draw_shadow(&mut sh, b, r, d, c),
        }
    }
}

