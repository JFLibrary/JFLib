// grid -- a grid box: rows and columns with spans and alignment (a box with its own placing
// mode; data.rs's grid_begin is the data table)
//
//   gridbox_begin(id, cols, flags, gap)   open a grid: one entry per column in DIP, > 0
//                                         fixed, 0 its widest child, < 0 a share of the width
//                                         left (fill weight); rows grow with their tallest
//                                         child.  flags: the box decorations and padding of
//                                         box_begin (BOX_CARD, BOX_CLIP, pad(n), ...) and the
//                                         alignment of the children: GRID_H_START | _CENTER |
//                                         _END (default: stretched across the cell),
//                                         GRID_V_START | _END | _STRETCH (default: centred);
//                                         gap: DIP between cells
//   gridbox_rows(rows)                    (after gridbox_begin) row sizes like cols; rows
//                                         without an entry are 0 (their tallest child); < 0
//                                         share the height left when the grid has a fixed one
//   gridbox_end()
//   next_span(cols, rows)                 the next child covers that many cells
//   next_cell(col, row)                   the next child starts there (later cells follow it)
//   next_align(h, v)                      the next child's alignment: AL_START, AL_CENTER,
//                                         AL_END, AL_STRETCH (AL_DEFAULT: the grid's)
//
// Children fill the cells row by row, skipping cells a row span still covers.  next_size
// still applies: a fixed size is aligned in the cell, a natural one too, a fill size (< 0)
// stretches.  Natural column widths and row heights come from the previous pass, as box
// sizes do; a change asks for another pass.  A spanned child widens the last of its natural
// columns (rows) when it does not fit.  A TX_WRAP text (and a BOX_WRAP box) wraps at the
// width its cell gives it (avail_width), so its row grows with its lines.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use super::consts::*;
use super::Ui;

/// the natural sizes of a grid's columns and rows (previous pass), per window
#[derive(Clone, Default)]
pub(crate) struct Mem {
    colw: Vec<i32>,
    rowh: Vec<i32>,
}

/// a grid being built
#[derive(Clone, Default)]
pub(crate) struct Run {
    id: u32,
    flags: u32,
    gap: i32,
    /// column specs (DIP, as given)
    cols: Vec<i32>,
    /// column x (absolute px) and widths of this pass
    colx: Vec<i32>,
    colw: Vec<i32>,
    /// row specs (DIP, as given) and the rows' heights / tops as laid out so far
    rows: Vec<i32>,
    rowh: Vec<i32>,
    rowy: Vec<i32>,
    /// content origin and size of the grid box
    pos: [i32; 2],
    len: [i32; 2],
    /// the next free cell
    col: usize,
    row: usize,
    /// per column: the first row not covered by a row span
    busy: Vec<usize>,
    /// natural sizes measured in this pass, and the spanned children (start, count, size)
    mcol: Vec<i32>,
    mrow: Vec<i32>,
    spans_c: Vec<(usize, usize, i32)>,
    spans_r: Vec<(usize, usize, i32)>,
}

impl Run {
    // the cell a child of `span` (cols, rows) starts in: `at`, else the next free one, past
    // cells a row span still covers and on the next row where the span does not fit
    fn next_cell(&self, span: (u16, u16), at: Option<(u16, u16)>) -> (usize, usize) {
        let ncol = self.colw.len();
        let cs = (span.0 as usize).min(ncol);
        let (mut col, mut row) = (self.col, self.row);
        if let Some((c, r)) = at {
            col = (c as usize).min(ncol - 1);
            row = r as usize;
        }
        loop {
            if col + cs > ncol {
                col = 0;
                row += 1;
                continue;
            }
            if (col..col + cs).any(|c| self.busy[c] > row) {
                col += 1;
                continue;
            }
            return (col, row);
        }
    }
}

/// the per-window part of the grids (in layout::W)
#[derive(Default)]
pub(crate) struct W {
    pub(crate) runs: Vec<Run>,
    pub(crate) mem: BTreeMap<u32, Mem>,
    /// next_span / next_cell / next_align for the next child
    pub(crate) span: (u16, u16),
    pub(crate) cell: Option<(u16, u16)>,
    pub(crate) align: (u8, u8),
    /// the cell of the last child (col, row, cols, rows) and its grid (run index + 1)
    pub(crate) last: (u16, [u16; 4]),
}

// the alignment fields of the grid flags
fn h_of(flags: u32) -> u8 {
    match (flags >> 1) & 3 {
        1 => AL_START as u8,
        2 => AL_CENTER as u8,
        3 => AL_END as u8,
        _ => AL_STRETCH as u8,
    }
}
fn v_of(flags: u32) -> u8 {
    match (flags >> 3) & 3 {
        1 => AL_START as u8,
        2 => AL_END as u8,
        3 => AL_STRETCH as u8,
        _ => AL_CENTER as u8,
    }
}

// sizes of `specs` (DIP) in `avail` px: fixed, natural (from `nat`), fill shares of the rest
fn sizes(specs: &[i32], nat: &[i32], avail: i32, gap: i32, dpi: i32) -> Vec<i32> {
    let n = specs.len();
    let mut out: Vec<i32> = alloc::vec![0; n];
    let (mut used, mut wsum) = (gap * (n as i32 - 1).max(0), 0i32);
    for (k, &s) in specs.iter().enumerate() {
        if s > 0 {
            out[k] = super::layout::dp_at(s, dpi);
        } else if s == 0 {
            out[k] = nat.get(k).copied().unwrap_or(0);
        } else {
            wsum += -s;
            continue;
        }
        used += out[k];
    }
    if wsum > 0 {
        let spare = (avail - used).max(0);
        let mut left = spare;
        let last = specs.iter().rposition(|&s| s < 0).unwrap_or(0);
        for (k, &s) in specs.iter().enumerate() {
            if s < 0 {
                out[k] = if k == last { left } else { spare * -s / wsum };
                left -= out[k];
            }
        }
    }
    out
}

impl Ui {
    /// gridbox_begin(id, cols, flags, gap): open a grid (see the module comment)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn gridbox_begin(&mut self, id: u32, cols: &[i32], flags: u32, gap: i32) {
        let _src = self.src_scope();
        // the grid's box: vertical, its natural size from gridbox_end; the alignment bits are ours
        self.box_begin(id, BOX_V | (flags & !0x3F), 0);
        let id = self.box_id();
        let b = *self.wp.lay.boxes.last().unwrap_or(&Default::default());
        let gap = self.dp_space(gap); // (theme) spacing
        let dpi = self.wp.rt.dpi;
        let mem = self.wp.lay.grid.mem.get(&id).cloned().unwrap_or_default();
        let cols = if cols.is_empty() { &[0][..] } else { cols };
        let colw = sizes(cols, &mem.colw, b.len[0], gap, dpi);
        let mut colx = Vec::with_capacity(colw.len());
        let mut x = b.pos[0];
        for w in &colw {
            colx.push(x);
            x += w + gap;
        }
        let n = cols.len();
        let run = Run {
            id,
            flags,
            gap,
            cols: cols.to_vec(),
            colx,
            colw,
            rows: Vec::new(),
            rowh: mem.rowh.clone(),
            rowy: Vec::new(),
            pos: b.pos,
            len: b.len,
            busy: alloc::vec![0; n],
            mcol: alloc::vec![0; n],
            mrow: Vec::new(),
            ..Run::default()
        };
        self.wp.lay.grid.runs.push(run);
        let k = self.wp.lay.grid.runs.len() as u16;
        if let Some(top) = self.wp.lay.boxes.last_mut() {
            top.grid = k;
        }
        self.grid_layout_rows();
    }

    /// gridbox_rows(rows): row sizes of the open grid (DIP; > 0 fixed, 0 natural, < 0 fill)
    pub fn gridbox_rows(&mut self, rows: &[i32]) {
        if let Some(r) = self.wp.lay.grid.runs.last_mut() {
            r.rows = rows.to_vec();
        }
        self.grid_layout_rows();
    }

    // the rows' heights and tops from their specs and the previous pass
    fn grid_layout_rows(&mut self) {
        let dpi = self.wp.rt.dpi;
        let mem = {
            let Some(r) = self.wp.lay.grid.runs.last() else { return };
            self.wp.lay.grid.mem.get(&r.id).cloned().unwrap_or_default()
        };
        let Some(r) = self.wp.lay.grid.runs.last_mut() else { return };
        let n = r.rows.len().max(mem.rowh.len());
        let specs: Vec<i32> = (0..n).map(|k| r.rows.get(k).copied().unwrap_or(0)).collect();
        r.rowh = sizes(&specs, &mem.rowh, r.len[1], r.gap, dpi);
        r.rowy.clear();
        let mut y = r.pos[1];
        for h in &r.rowh {
            r.rowy.push(y);
            y += h + r.gap;
        }
    }

    /// next_span(cols, rows): the next grid child covers that many cells
    pub fn next_span(&mut self, cols: u32, rows: u32) {
        self.wp.lay.grid.span = (cols.clamp(1, 1000) as u16, rows.clamp(1, 1000) as u16);
    }

    /// next_cell(col, row): the next grid child starts in that cell
    pub fn next_cell(&mut self, col: u32, row: u32) {
        self.wp.lay.grid.cell = Some((col.min(1000) as u16, row.min(100_000) as u16));
    }

    /// next_align(h, v): the next grid child's alignment (AL_*; AL_DEFAULT: the grid's)
    pub fn next_align(&mut self, h: u32, v: u32) {
        self.wp.lay.grid.align = (h.min(4) as u8, v.min(4) as u8);
    }

    /// (avail_width) the width grid k (run index + 1) gives its next child of natural width
    /// iw, as grid_place will: the cell's width (stretched or fill), a fixed width (at most the
    /// cell's), else (aligned, natural) at most the cell's; 1 when it does not depend on iw
    pub(crate) fn grid_avail_width(&self, k: usize, iw: i32) -> (i32, i32) {
        let l = &self.wp.lay;
        let Some(r) = l.grid.runs.get(k - 1) else { return (iw, 0) };
        let span = (l.grid.span.0.max(1), l.grid.span.1.max(1));
        let (c, _) = r.next_cell(span, l.grid.cell);
        let e = c + (span.0 as usize).min(r.colw.len()) - 1;
        let cw = r.colx[e] + r.colw[e] - r.colx[c];
        let ha = if l.grid.align.0 != 0 { l.grid.align.0 } else { h_of(r.flags) };
        if l.next_w > 0 {
            return (self.dp(l.next_w).min(cw), 1);
        }
        if l.next_w < 0 || ha == AL_STRETCH as u8 {
            return (cw, 1);
        }
        if cw <= 0 {
            return (iw, 0);
        }
        (cw.min(iw), 0)
    }

    /// (place) the next child of grid k (run index + 1): its rectangle
    pub(crate) fn grid_place(&mut self, k: usize, iw: i32, ih: i32) -> [i32; 4] {
        let dpi = self.wp.rt.dpi;
        let l = &mut self.wp.lay;
        let (rw, rh) = (l.next_w, l.next_h);
        l.next_w = 0;
        l.next_h = 0;
        l.place_auto = 0;
        l.place_main = 0;
        let span = core::mem::replace(&mut l.grid.span, (1, 1));
        let span = (span.0.max(1), span.1.max(1));
        let at = l.grid.cell.take();
        let al = core::mem::take(&mut l.grid.align);
        let r = &mut l.grid.runs[k - 1];
        let cs = (span.0 as usize).min(r.colw.len());
        let rs = span.1 as usize;
        // the cell: an explicit one, else the next one free of row spans where it fits
        let (c, row) = r.next_cell(span, at);
        r.row = row;
        // natural size and what this child asks for
        let nat_w = if rw > 0 { super::layout::dp_at(rw, dpi) } else { iw };
        let nat_h = if rh > 0 { super::layout::dp_at(rh, dpi) } else { ih };
        // rows not known yet (new this pass): as tall as their first child
        while r.rowh.len() < row + rs {
            let y = match (r.rowy.last(), r.rowh.last()) {
                (Some(y), Some(h)) => y + h + r.gap,
                _ => r.pos[1],
            };
            r.rowy.push(y);
            r.rowh.push(if rs == 1 { nat_h } else { 0 });
        }
        if r.mrow.len() < row + rs {
            r.mrow.resize(row + rs, 0);
        }
        // record the natural sizes
        if cs == 1 {
            r.mcol[c] = r.mcol[c].max(nat_w);
        } else {
            r.spans_c.push((c, cs, nat_w));
        }
        if rs == 1 {
            r.mrow[row] = r.mrow[row].max(nat_h);
        } else {
            r.spans_r.push((row, rs, nat_h));
        }
        // the cell rectangle
        let cx = r.colx[c];
        let cw = r.colx[c + cs - 1] + r.colw[c + cs - 1] - cx;
        let cy = r.rowy[row];
        let ch = r.rowy[row + rs - 1] + r.rowh[row + rs - 1] - cy;
        // alignment
        let ha = if al.0 != 0 { al.0 } else { h_of(r.flags) };
        let va = if al.1 != 0 { al.1 } else { v_of(r.flags) };
        let place = |a: u8, start: i32, size: i32, want: i32, fill: bool| -> (i32, i32) {
            if fill || (a == AL_STRETCH as u8 && want <= 0) {
                return (start, size);
            }
            let w = if a == AL_STRETCH as u8 { want.min(size) } else { want.min(size).max(0) };
            match a {
                x if x == AL_CENTER as u8 => (start + (size - w) / 2, w),
                x if x == AL_END as u8 => (start + size - w, w),
                _ => (start, w),
            }
        };
        let (x, w) = place(ha, cx, cw, if rw > 0 { nat_w } else if ha == AL_STRETCH as u8 { 0 } else { nat_w }, rw < 0);
        let (y, h) = place(va, cy, ch, if rh > 0 { nat_h } else if va == AL_STRETCH as u8 { 0 } else { nat_h }, rh < 0);
        // the next cell; a row span keeps its columns busy
        if rs > 1 {
            for b in &mut r.busy[c..c + cs] {
                *b = row + rs;
            }
        }
        r.col = c + cs;
        l.grid.last = (k as u16, [c as u16, row as u16, cs as u16, rs as u16]);
        [x, y, w, h]
    }

    /// (box_end) a child box of grid k measured (w, h) this pass in its cell
    pub(crate) fn grid_child_measured(&mut self, k: usize, cell: [u16; 4], w: i32, h: i32) {
        let Some(r) = self.wp.lay.grid.runs.get_mut(k - 1) else { return };
        let [c, row, cs, rs] = cell.map(|v| v as usize);
        if cs == 1 && c < r.mcol.len() {
            r.mcol[c] = r.mcol[c].max(w);
        } else if cs > 1 {
            r.spans_c.push((c, cs, w));
        }
        if rs == 1 && row < r.mrow.len() {
            r.mrow[row] = r.mrow[row].max(h);
        } else if rs > 1 {
            r.spans_r.push((row, rs, h));
        }
    }

    /// gridbox_end: store the measured sizes (another pass when they changed), close the box
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn gridbox_end(&mut self) {
        let _src = self.src_scope();
        let Some(mut r) = self.wp.lay.grid.runs.pop() else {
            self.warn("gridbox_end without gridbox_begin");
            return;
        };
        // spanned children: widen the last natural column / row of their span
        let fit = |m: &mut Vec<i32>, specs: &dyn Fn(usize) -> i32, spans: &[(usize, usize, i32)], gap: i32| {
            for &(s, n, want) in spans {
                if s + n > m.len() {
                    m.resize(s + n, 0);
                }
                let have: i32 = m[s..s + n].iter().sum::<i32>() + gap * (n as i32 - 1);
                if want > have {
                    if let Some(k) = (s..s + n).rev().find(|&k| specs(k) == 0) {
                        m[k] += want - have;
                    }
                }
            }
        };
        let mut mcol = core::mem::take(&mut r.mcol);
        let mut mrow = core::mem::take(&mut r.mrow);
        let (cols_spec, rows_spec) = (r.cols.clone(), r.rows.clone());
        fit(&mut mcol, &|k| cols_spec.get(k).copied().unwrap_or(0), &r.spans_c, r.gap);
        fit(&mut mrow, &|k| rows_spec.get(k).copied().unwrap_or(0), &r.spans_r, r.gap);
        let changed = self.wp.lay.grid.mem.get(&r.id).is_none_or(|m| m.colw != mcol || m.rowh != mrow);
        if changed {
            self.repass = true;
        }
        // the grid's natural size: the columns and rows as laid out (natural ones measured)
        let dpi = self.wp.rt.dpi;
        let gap = r.gap;
        // fixed: its size; natural and fill: the widest child
        let colw: i32 = (0..cols_spec.len())
            .map(|k| match cols_spec[k] {
                s if s > 0 => super::layout::dp_at(s, dpi),
                _ => mcol.get(k).copied().unwrap_or(0),
            })
            .sum::<i32>()
            + gap * (cols_spec.len() as i32 - 1).max(0);
        let nrow = mrow.len().max(rows_spec.len());
        let rowh: i32 = (0..nrow)
            .map(|k| match rows_spec.get(k).copied().unwrap_or(0) {
                s if s > 0 => super::layout::dp_at(s, dpi),
                _ => mrow.get(k).copied().unwrap_or(0),
            })
            .sum::<i32>()
            + gap * (nrow as i32 - 1).max(0);
        self.wp.lay.grid.mem.insert(r.id, Mem { colw: mcol, rowh: mrow });
        if let Some(top) = self.wp.lay.boxes.last_mut() {
            top.grid = 0;
            top.fixed = rowh; // BOX_V: main axis = height
            top.cross = colw;
            top.wsum = 0;
        }
        self.box_end();
    }
}
