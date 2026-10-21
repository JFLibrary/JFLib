// fxtest -- headless test runner for the graphics kernels of gfx::straight, gfx::blendmode,
// gfx::tmask and gfx::flood                                              [apps/fxtest/src/main.s]
//
// Driven by tools/blend_check.py, tools/tmask_check.py and tools/flood_check.py (the same
// protocol as the assembly version, so the tools run unchanged against this build):
//
//   fxtest --shot x.png --batch IN --result OUT
//   fxtest --selftest            known-answer checks of every kernel family (exit code 0 =
//                                passed; tests/selftest.uis)
//
// Batch file: "FXB1", then records {u32 op, u32 payload bytes, payload}; payload = u32 np,
// i32 p[np] (up to 16, missing ones are 0), u32 nb (up to 8), nb x {u32 len, u32 align,
// u8 data[len]}.  Result file: "FXR1", then per record {i32 result, u32 len, data}: data =
// every buffer after the op (in order), then the op's extra output.  -100: unknown op.
// p[7] = repetitions: the extra output then ends with the u64 elapsed microseconds.
// The assembly version places every buffer at the requested alignment between 0xCD guard
// bytes (result -200 when they change); Rust slices cannot write outside their buffer, so
// the alignment is ignored and a buffer too short for the op's parameters gives -200 too.
//
// Ops (b = buffers, p = params) -- see apps/fxtest/src/main.s for the full list:
//   1 premul_row  2 unpremul_row  3 unpremul_row (png reference)  4 lerp_mask_row
//   5 erase_row  6 view_row  7 view_row_pm  10 bm_row  11 bm_cov_row  12 bm_solid_row
//   20..44 mask ops (16 slots), 50 flood_fill_mask
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use base::Args;
use gfx::blendmode::{self, BM_COLORBURN, BM_MULTIPLY, BM_NORMAL, BM_OVERLAY, BM_OVERWRITE, BM_SCREEN};
use gfx::flood::{FF_GLOBAL, flood_fill_mask};
use gfx::path::Path;
use gfx::raster::Raster;
use gfx::straight::{self, ViewCfg};
use gfx::tmask::{Block, Span, TM_ADD, TM_REPLACE, TMask};
use gfx::{FMT_ARGB, RULE_NONZERO, Surface};

const MAXPAR: usize = 16;
const MAXBUF: usize = 8;
const NMASK: usize = 16;
const P_REPS: usize = 7;
const R_GUARD: i32 = -200; // a buffer too short for the op (the guard bytes would change)
const R_UNKNOWN: i32 = -100;

ui::main!(fxtest());

fn fxtest() -> i32 {
    let args = Args(plat::args());
    if args.flag("--selftest") {
        return selftest();
    }
    let Some(inp) = args.value("--batch") else {
        plat::write_stderr(b"usage: fxtest --shot x.png (--batch IN --result OUT | --selftest)\n");
        return 1;
    };
    run_batch(&args, inp)
}

// ---------------------------------------------------------------- batch

struct Rec {
    p: [i32; MAXPAR],
    bufs: Vec<Vec<u8>>,
}

/// the runner state: mask slots, the extra output of the current record, the repetition
struct Fx {
    masks: Vec<Option<TMask>>,
    ex: Vec<u8>,
    rep: u32,
    ncont: u32,
    path: Path,
    ras: Raster,
}

impl Fx {
    fn ex_u32(&mut self, v: u32) {
        self.ex.extend_from_slice(&v.to_le_bytes());
    }

    fn slot(p: &[i32; MAXPAR], i: usize) -> usize {
        (p[i & (MAXPAR - 1)] as usize) & (NMASK - 1)
    }
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn run_batch(args: &Args, inp: &str) -> i32 {
    let Some(outp) = args.value("--result") else {
        plat::write_stderr(b"error: --batch needs --result OUT\n");
        return 1;
    };
    let Some(file) = plat::read_file(inp) else {
        plat::write_stderr(b"error: cannot read the batch file\n");
        return 1;
    };
    let bad = || {
        plat::write_stderr(b"error: malformed batch file\n");
        1
    };
    if file.len() < 4 || &file[..4] != b"FXB1" {
        return bad();
    }
    let mut fx = Fx {
        masks: (0..NMASK).map(|_| None).collect(),
        ex: Vec::new(),
        rep: 0,
        ncont: 0,
        path: Path::default(),
        ras: Raster::default(),
    };
    let mut out: Vec<u8> = b"FXR1".to_vec();
    let mut pos = 4;
    let mut nrec = 0u32;
    while file.len() - pos >= 8 {
        let op = u32_at(&file, pos);
        let plen = u32_at(&file, pos + 4) as usize;
        if plen > file.len() - pos - 8 {
            return bad();
        }
        pos += 8;
        let end = pos + plen;
        let need4 = |pos: usize| end - pos >= 4;
        let mut rec = Rec { p: [0; MAXPAR], bufs: Vec::new() };
        if !need4(pos) {
            return bad();
        }
        let np = u32_at(&file, pos) as usize;
        pos += 4;
        if np > MAXPAR {
            return bad();
        }
        for i in 0..np {
            if !need4(pos) {
                return bad();
            }
            rec.p[i] = u32_at(&file, pos) as i32;
            pos += 4;
        }
        if !need4(pos) {
            return bad();
        }
        let nb = u32_at(&file, pos) as usize;
        pos += 4;
        if nb > MAXBUF {
            return bad();
        }
        for _ in 0..nb {
            if end - pos < 8 {
                return bad();
            }
            let len = u32_at(&file, pos) as usize;
            if len > end - pos - 8 {
                return bad();
            }
            rec.bufs.push(file[pos + 8..pos + 8 + len].to_vec());
            pos += 8 + len;
        }
        pos = end;
        fx.ex.clear();
        let r = run_op(&mut fx, op, &mut rec);
        let total: usize = rec.bufs.iter().map(|b| b.len()).sum::<usize>() + fx.ex.len();
        out.extend_from_slice(&r.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        for b in &rec.bufs {
            out.extend_from_slice(b);
        }
        out.extend_from_slice(&fx.ex);
        nrec += 1;
    }
    if !plat::write_file(outp, &out) {
        plat::write_stderr(b"error: cannot write the result file\n");
        return 1;
    }
    plat::write_stdout(format!("batch: {nrec} records\n").as_bytes());
    0
}

type Thunk = fn(&mut Fx, &mut Rec) -> i32;

/// look the op up and run it p[7] times (at least once); with p[7] > 1 the elapsed
/// microseconds are appended to the extra output
fn run_op(fx: &mut Fx, op: u32, rec: &mut Rec) -> i32 {
    let f: Thunk = match op {
        1 => t_premul,
        2 | 3 => t_unpremul,
        4 => t_lerp,
        5 => t_erase,
        6 => t_view,
        7 => t_viewpm,
        10 => t_bm,
        11 => t_bmcov,
        12 => t_bmsolid,
        20 => t_alloc,
        21 => t_free,
        22 => t_load,
        23 => t_store,
        24 => t_combine,
        25 => t_invert,
        26 => t_bbox,
        27 => t_thresh,
        28 => t_fillrect,
        29 => t_rpoly,
        30 => t_rpath,
        31 => t_trace,
        32 => t_count,
        33 => t_span,
        34 => t_row,
        35 => t_clear,
        36 => t_copy,
        37 => t_clone,
        38 => t_get,
        39 => t_roww,
        40 => t_optimize,
        41 => t_tileset,
        42 => t_cblock,
        43 => t_tilew,
        44 => t_rctx,
        50 => t_flood,
        _ => return R_UNKNOWN,
    };
    let reps = rec.p[P_REPS].max(1) as u32;
    fx.rep = 0;
    let t0 = plat::time_us();
    let mut r = 0;
    while fx.rep < reps {
        r = f(fx, rec);
        fx.rep += 1;
    }
    let dt = plat::time_us() - t0;
    if reps > 1 {
        fx.ex.extend_from_slice(&dt.to_le_bytes());
    }
    r
}

// ---------------------------------------------------------------- kernel ops

// n pixels: the byte count, or None when a buffer is too short
fn px_bytes(rec: &Rec, n: i32, bufs: &[usize]) -> Option<usize> {
    let nb = (n.max(0) as usize) * 4;
    bufs.iter().all(|&i| rec.bufs.get(i).is_some_and(|b| b.len() >= nb)).then_some(nb)
}

// b dst, src; p n, in-place
fn t_premul(_: &mut Fx, rec: &mut Rec) -> i32 {
    let d = if rec.p[1] != 0 { 1 } else { 0 };
    let Some(nb) = px_bytes(rec, rec.p[0], &[d, 1]) else { return R_GUARD };
    if d == 1 {
        straight::premul_inplace(&mut rec.bufs[1][..nb]);
    } else {
        let (o, s) = rec.bufs.split_at_mut(1);
        straight::premul_row(&mut o[0][..nb], &s[0][..nb]);
    }
    0
}

fn t_unpremul(_: &mut Fx, rec: &mut Rec) -> i32 {
    let d = if rec.p[1] != 0 { 1 } else { 0 };
    let Some(nb) = px_bytes(rec, rec.p[0], &[d, 1]) else { return R_GUARD };
    if d == 1 {
        straight::unpremul_inplace(&mut rec.bufs[1][..nb]);
    } else {
        let (o, s) = rec.bufs.split_at_mut(1);
        straight::unpremul_row(&mut o[0][..nb], &s[0][..nb]);
    }
    0
}

// b dst, a, b, m; p n, alias (1: dst = a, 2: dst = b)
fn t_lerp(_: &mut Fx, rec: &mut Rec) -> i32 {
    let n = rec.p[0].max(0) as usize;
    let d = match rec.p[1] {
        1 => 1,
        2 => 2,
        _ => 0,
    };
    let Some(nb) = px_bytes(rec, rec.p[0], &[d, 1, 2]) else { return R_GUARD };
    if rec.bufs.get(3).is_none_or(|b| b.len() < n) {
        return R_GUARD;
    }
    let (o, s) = rec.bufs.split_at_mut(1);
    let (a, s) = s.split_at_mut(1);
    let (bb, m) = s.split_at_mut(1);
    match d {
        1 => straight::lerp_mask_into_a(&mut a[0][..nb], &bb[0][..nb], &m[0][..n]),
        2 => straight::lerp_mask_into_b(&a[0][..nb], &mut bb[0][..nb], &m[0][..n]),
        _ => straight::lerp_mask_row(&mut o[0][..nb], &a[0][..nb], &bb[0][..nb], &m[0][..n]),
    }
    0
}

// b dst, cov; p n, strength
fn t_erase(_: &mut Fx, rec: &mut Rec) -> i32 {
    let n = rec.p[0].max(0) as usize;
    let Some(nb) = px_bytes(rec, rec.p[0], &[0]) else { return R_GUARD };
    if rec.bufs.get(1).is_none_or(|b| b.len() < n) {
        return R_GUARD;
    }
    let (d, c) = rec.bufs.split_at_mut(1);
    straight::erase_row(&mut d[0][..nb], &c[0][..n], rec.p[1] as u32);
    0
}

fn view_cfg(rec: &Rec) -> Option<ViewCfg> {
    let b = rec.bufs.get(2)?;
    (b.len() >= 12).then(|| ViewCfg { cell: u32_at(b, 0) as i32, c0: u32_at(b, 4), c1: u32_at(b, 8) })
}

// b dst, src, cfg; p n, x0, y0
fn t_view(_: &mut Fx, rec: &mut Rec) -> i32 {
    let Some(nb) = px_bytes(rec, rec.p[0], &[0, 1]) else { return R_GUARD };
    let Some(cfg) = view_cfg(rec) else { return R_GUARD };
    let (d, s) = rec.bufs.split_at_mut(1);
    straight::view_row(&mut d[0][..nb], &s[0][..nb], rec.p[1], rec.p[2], &cfg);
    0
}

fn t_viewpm(_: &mut Fx, rec: &mut Rec) -> i32 {
    let Some(nb) = px_bytes(rec, rec.p[0], &[0, 1]) else { return R_GUARD };
    let Some(cfg) = view_cfg(rec) else { return R_GUARD };
    let (d, s) = rec.bufs.split_at_mut(1);
    straight::view_row_pm(&mut d[0][..nb], &s[0][..nb], rec.p[1], rec.p[2], &cfg);
    0
}

// b dst, lhs, rhs; p n, modeop, alias (dst = lhs)
fn t_bm(_: &mut Fx, rec: &mut Rec) -> i32 {
    let d = if rec.p[2] != 0 { 1 } else { 0 };
    let Some(nb) = px_bytes(rec, rec.p[0], &[d, 1, 2]) else { return R_GUARD };
    let mo = rec.p[1] as u32;
    let (o, s) = rec.bufs.split_at_mut(1);
    if d == 1 {
        let (l, r) = s.split_at_mut(1);
        blendmode::bm_row_inplace(&mut l[0][..nb], &r[0][..nb], mo);
    } else {
        blendmode::bm_row(&mut o[0][..nb], &s[0][..nb], &s[1][..nb], mo);
    }
    0
}

// b dst, lhs, rhs, cov; p n, modeop, alias, nocov
fn t_bmcov(_: &mut Fx, rec: &mut Rec) -> i32 {
    let n = rec.p[0].max(0) as usize;
    let d = if rec.p[2] != 0 { 1 } else { 0 };
    let Some(nb) = px_bytes(rec, rec.p[0], &[d, 1, 2]) else { return R_GUARD };
    let nocov = rec.p[3] != 0;
    if !nocov && rec.bufs.get(3).is_none_or(|b| b.len() < n) {
        return R_GUARD;
    }
    let mo = rec.p[1] as u32;
    let (o, s) = rec.bufs.split_at_mut(1);
    let (l, rc) = s.split_at_mut(1);
    let cov = if nocov { None } else { Some(&rc[1][..n]) };
    if d == 1 {
        blendmode::bm_cov_row_inplace(&mut l[0][..nb], &rc[0][..nb], cov, n, mo);
    } else {
        blendmode::bm_cov_row(&mut o[0][..nb], &l[0][..nb], &rc[0][..nb], cov, n, mo);
    }
    0
}

// b dst, lhs, cov; p n, modeop, alias, nocov, argb
fn t_bmsolid(_: &mut Fx, rec: &mut Rec) -> i32 {
    let n = rec.p[0].max(0) as usize;
    let d = if rec.p[2] != 0 { 1 } else { 0 };
    let Some(nb) = px_bytes(rec, rec.p[0], &[d, 1]) else { return R_GUARD };
    let nocov = rec.p[3] != 0;
    if !nocov && rec.bufs.get(2).is_none_or(|b| b.len() < n) {
        return R_GUARD;
    }
    let (mo, argb) = (rec.p[1] as u32, rec.p[4] as u32);
    let (o, s) = rec.bufs.split_at_mut(1);
    let (l, c) = s.split_at_mut(1);
    let cov = if nocov { None } else { Some(&c[0][..n]) };
    if d == 1 {
        blendmode::bm_solid_row_inplace(&mut l[0][..nb], argb, cov, n, mo);
    } else {
        blendmode::bm_solid_row(&mut o[0][..nb], &l[0][..nb], argb, cov, n, mo);
    }
    0
}

// ---------------------------------------------------------------- mask ops

macro_rules! mask {
    ($fx:expr, $rec:expr, $i:expr) => {
        match $fx.masks[Fx::slot(&$rec.p, $i)].as_mut() {
            Some(m) => m,
            None => return -1,
        }
    };
}

// p slot, w, h (result 0 / -1)
fn t_alloc(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let s = Fx::slot(&rec.p, 0);
    fx.masks[s] = TMask::new(rec.p[1] as u32, rec.p[2] as u32);
    if fx.masks[s].is_some() { 0 } else { -1 }
}

fn t_free(fx: &mut Fx, rec: &mut Rec) -> i32 {
    fx.masks[Fx::slot(&rec.p, 0)] = None;
    0
}

// p slot, stride; b data
fn t_load(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let m = mask!(fx, rec, 0);
    let stride = rec.p[1];
    let Some(b) = rec.bufs.first() else { return R_GUARD };
    if stride < 0 || m.h != 0 && b.len() < (m.h as usize - 1) * stride as usize + m.w as usize {
        return R_GUARD;
    }
    m.load_a8(b, stride as usize)
}

// p slot -> extra w*h bytes
fn t_store(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let m = mask!(fx, rec, 0);
    let mut d = vec![0u8; m.w as usize * m.h as usize];
    m.store_a8(&mut d, m.w as usize);
    fx.ex.extend_from_slice(&d);
    0
}

// p dst, src, op
fn t_combine(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let (d, s) = (Fx::slot(&rec.p, 0), Fx::slot(&rec.p, 1));
    let op = rec.p[2] as u32;
    if d == s {
        return mask!(fx, rec, 0).combine_self(op);
    }
    let Some(src) = fx.masks[s].take() else { return -1 };
    let r = match fx.masks[d].as_mut() {
        Some(m) => m.combine(&src, op),
        None => -1,
    };
    fx.masks[s] = Some(src);
    r
}

fn t_invert(fx: &mut Fx, rec: &mut Rec) -> i32 {
    mask!(fx, rec, 0).invert();
    0
}

// p slot -> extra 16 bytes
fn t_bbox(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let b = mask!(fx, rec, 0).bbox();
    for v in b.unwrap_or([0; 4]) {
        fx.ex.extend_from_slice(&v.to_le_bytes());
    }
    b.is_some() as i32
}

// p slot, level
fn t_thresh(fx: &mut Fx, rec: &mut Rec) -> i32 {
    mask!(fx, rec, 0).threshold(rec.p[1]);
    0
}

// p slot, x0, y0, x1, y1, value
fn t_fillrect(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let p = rec.p;
    mask!(fx, rec, 0).fill_rect(p[1], p[2], p[3], p[4], p[5] as u8)
}

fn f32_pts(b: &[u8]) -> Vec<(f32, f32)> {
    b.chunks_exact(8).map(|c| (f32::from_bits(u32_at(c, 0)), f32::from_bits(u32_at(c, 4)))).collect()
}

// p slot, rule, op; b f32 points
fn t_rpoly(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let pts = f32_pts(rec.bufs.first().map_or(&[][..], |b| &b[..]));
    let (rule, op) = (rec.p[1] as u32, rec.p[2] as u32);
    let s = Fx::slot(&rec.p, 0);
    let Fx { masks, path, ras, .. } = fx;
    match masks[s].as_mut() {
        Some(m) => m.raster_polygon(path, ras, &pts, rule, op),
        None => -1,
    }
}

// p slot, rule, op; b {u32 nsub, nsub x {u32 n, f32 pts[2n]}}
fn t_rpath(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let b = rec.bufs.first().map_or(&[][..], |b| &b[..]);
    let rd = |o: usize| if o + 4 <= b.len() { u32_at(b, o) } else { 0 };
    let path = &mut fx.path;
    path.reset();
    let mut o = 4;
    for _ in 0..rd(0) {
        let n = rd(o);
        o += 4;
        for k in 0..n {
            let (x, y) = (f32::from_bits(rd(o)), f32::from_bits(rd(o + 4)));
            if k == 0 {
                path.move_f(x, y);
            } else {
                path.line_f(x, y);
            }
            o += 8;
        }
        path.close();
    }
    let (rule, op) = (rec.p[1] as u32, rec.p[2] as u32);
    let s = Fx::slot(&rec.p, 0);
    let Fx { masks, path, ras, .. } = fx;
    match masks[s].as_mut() {
        Some(m) => m.raster_path(path, ras, rule, op),
        None => -1,
    }
}

// p slot, stop_after -> extra per contour {u32 n, i32 pts[2n]} (first repetition only)
fn t_trace(fx: &mut Fx, rec: &mut Rec) -> i32 {
    fx.ncont = 0;
    let stop = rec.p[1] as u32;
    let s = Fx::slot(&rec.p, 0);
    let Fx { masks, ex, rep, ncont, .. } = fx;
    let Some(m) = masks[s].as_ref() else { return -1 };
    m.trace(&mut |pts: &[(i32, i32)]| {
        if *rep == 0 {
            ex.extend_from_slice(&(pts.len() as u32).to_le_bytes());
            for &(x, y) in pts {
                ex.extend_from_slice(&x.to_le_bytes());
                ex.extend_from_slice(&y.to_le_bytes());
            }
        }
        *ncont += 1;
        if stop != 0 && *ncont >= stop { 7 } else { 0 }
    })
}

// p slot -> extra u32 tiles, u32 full
fn t_count(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let (t, f) = mask!(fx, rec, 0).count_tiles();
    fx.ex_u32(t);
    fx.ex_u32(f);
    0
}

// p slot, x, y, maxn -> extra u32 kind (0 empty, 1 full, 2 pointer), u32 n, [bytes]
fn t_span(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let p = rec.p;
    let s = Fx::slot(&p, 0);
    let Fx { masks, ex, .. } = fx;
    let Some(m) = masks[s].as_ref() else { return -1 };
    let (sp, n) = m.span(p[1], p[2], p[3]);
    let kind: u32 = match sp {
        Span::Empty => 0,
        Span::Full => 1,
        Span::Bytes(_) => 2,
    };
    ex.extend_from_slice(&kind.to_le_bytes());
    ex.extend_from_slice(&(n as u32).to_le_bytes());
    if let Span::Bytes(b) = sp {
        ex.extend_from_slice(b);
    }
    0
}

// p slot, x, y, n -> extra u32 (1 = returned the caller's buffer), n bytes
fn t_row(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let p = rec.p;
    let n = p[3];
    if n < 0 {
        return -1;
    }
    let s = Fx::slot(&p, 0);
    let Fx { masks, ex, .. } = fx;
    let Some(m) = masks[s].as_ref() else { return -1 };
    let mut buf = vec![0u8; n as usize + 1];
    let (b, own) = m.row(p[1], p[2], n, &mut buf);
    let b: Vec<u8> = b.to_vec();
    ex.extend_from_slice(&(own as u32).to_le_bytes());
    ex.extend_from_slice(&b);
    0
}

// p slot, value
fn t_clear(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let v = rec.p[1] as u8;
    mask!(fx, rec, 0).clear(v)
}

// p dst, src
fn t_copy(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let (d, s) = (Fx::slot(&rec.p, 0), Fx::slot(&rec.p, 1));
    if d == s {
        return mask!(fx, rec, 0).combine_self(TM_REPLACE);
    }
    let Some(src) = fx.masks[s].take() else { return -1 };
    let r = match fx.masks[d].as_mut() {
        Some(m) => m.copy(&src),
        None => -1,
    };
    fx.masks[s] = Some(src);
    r
}

// p dst, src
fn t_clone(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let c = fx.masks[Fx::slot(&rec.p, 1)].clone();
    let ok = c.is_some();
    fx.masks[Fx::slot(&rec.p, 0)] = c;
    if ok { 0 } else { -1 }
}

// p slot, x, y -> result
fn t_get(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let p = rec.p;
    mask!(fx, rec, 0).get(p[1], p[2]) as i32
}

// p slot, x, y, n, value: write n bytes from (x, y) through row_w
fn t_roww(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let p = rec.p;
    let m = mask!(fx, rec, 0);
    let (mut x, mut left) = (p[1], p[3]);
    while left > 0 && x < m.w as i32 {
        let Some(row) = m.row_w(x, p[2]) else { return -1 };
        let k = row.len().min(left as usize);
        row[..k].fill(p[4] as u8);
        x += k as i32;
        left -= k as i32;
    }
    0
}

fn t_optimize(fx: &mut Fx, rec: &mut Rec) -> i32 {
    mask!(fx, rec, 0).optimize();
    0
}

// p slot, tx, ty, kind
fn t_tileset(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let p = rec.p;
    mask!(fx, rec, 0).tile_set(p[1] as u32, p[2] as u32, p[3] != 0);
    0
}

// p slot, tx, ty, stride, op, srcmode (0 buffer, 1 empty, 2 full); b block
fn t_cblock(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let p = rec.p;
    let s = Fx::slot(&p, 0);
    let stride = p[3].max(0) as usize;
    let data: Vec<u8>;
    let blk = match p[5] {
        1 => Block::Empty,
        2 => Block::Full,
        _ => {
            let b = rec.bufs.first().map_or(&[][..], |b| &b[..]);
            if b.len() < 63 * stride + 64 {
                return R_GUARD;
            }
            data = b.to_vec();
            Block::Data(&data, stride)
        }
    };
    match fx.masks[s].as_mut() {
        Some(m) => m.combine_block(p[1] as u32, p[2] as u32, blk, p[4] as u32),
        None => -1,
    }
}

// p slot, tx, ty (result 0 / -1)
fn t_tilew(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let p = rec.p;
    if mask!(fx, rec, 0).tile_w(p[1] as u32, p[2] as u32).is_some() { 0 } else { -1 }
}

// p slot; b f32 points: raster_polygon (non-zero, replace) "inside a draw context" -- the
// Rust masks never touch a draw context, so the context check is always 1
fn t_rctx(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let pts = f32_pts(rec.bufs.first().map_or(&[][..], |b| &b[..]));
    let s = Fx::slot(&rec.p, 0);
    let Fx { masks, path, ras, ex, .. } = fx;
    let r = match masks[s].as_mut() {
        Some(m) => m.raster_polygon(path, ras, &pts, RULE_NONZERO, TM_REPLACE),
        None => return -1,
    };
    ex.extend_from_slice(&1u32.to_le_bytes());
    r
}

// p slot, w, h, x, y, tol, flags, reps, pad; b pixels ((w + pad) * h * 4, FMT_ARGB)
fn t_flood(fx: &mut Fx, rec: &mut Rec) -> i32 {
    let p = rec.p;
    let s = Fx::slot(&p, 0);
    let stride = p[1].wrapping_add(p[8]).wrapping_mul(4);
    // the pixels move into the surface and back (no copy inside the timed repetitions)
    let px = rec.bufs.first_mut().map(core::mem::take).unwrap_or_default();
    if p[1] > 0 && p[2] > 0 && (stride < p[1] * 4 || px.len() < (p[2] as usize - 1) * stride as usize + p[1] as usize * 4) {
        if let Some(b) = rec.bufs.first_mut() {
            *b = px;
        }
        return R_GUARD;
    }
    let src = Surface { px, w: p[1], h: p[2], stride, fmt: FMT_ARGB, ..Surface::default() };
    let r = match fx.masks[s].as_mut() {
        Some(m) => flood_fill_mask(&src, p[3], p[4], p[5], p[6] as u32, m),
        None => -1,
    };
    if let Some(b) = rec.bufs.first_mut() {
        *b = src.px;
    }
    r
}

// ---------------------------------------------------------------- self test

const STC_LHS: [u32; 5] = [0x80FF0000, 0xFF123456, 0x00000000, 0x40A0B0C0, 0xFF808080];
const STC_RHS: [u32; 5] = [0x800000FF, 0x80FFFFFF, 0x7F102030, 0xFF000000, 0xC8FF4020];
const STC_PM: [u32; 5] = [0x80000080, 0x80808080, 0x7F081018, 0xFF000000, 0xC8C83219];
const STC_COV: [u8; 5] = [255, 128, 0, 200, 77];
// modeop, coverage row, expected (numpy reference, tools/blend_check.py ref_blend)
const STC_CASES: [(u32, bool, [u32; 5]); 12] = [
    (BM_NORMAL | 200 << 8, false, [0xb26f008f, 0xff6e8398, 0x64102030, 0xd60a0b0c, 0xffce5844]),
    (BM_NORMAL | 255 << 8, true, [0xc05500aa, 0xff4d6680, 0x00000000, 0xd60a0b0c, 0xff9d7069]),
    (BM_MULTIPLY | 200 << 8, false, [0xb26f0047, 0xff123456, 0x64102030, 0xd60a0b0c, 0xff80443b]),
    (BM_MULTIPLY | 255 << 8, true, [0xc0550055, 0xff123456, 0x00000000, 0xd60a0b0c, 0xff806965]),
    (BM_COLORBURN | 200 << 8, false, [0xb26f0047, 0xff123456, 0x64102030, 0xd60a0b0c, 0xff803131]),
    (BM_COLORBURN | 255 << 8, true, [0xc0550055, 0xff123456, 0x00000000, 0xd60a0b0c, 0xff806161]),
    (BM_OVERLAY | 200 << 8, false, [0xb2b70047, 0xff194877, 0x64102030, 0xd619222a, 0xffce5945]),
    (BM_OVERLAY | 255 << 8, true, [0xc0aa0055, 0xff16416b, 0x00000000, 0xd619222a, 0xff9d7169]),
    (BM_SCREEN | 200 << 8, false, [0xb2b7008f, 0xff6e8398, 0x64102030, 0xd62f3439, 0xffce9389]),
    (BM_SCREEN | 255 << 8, true, [0xc0aa00aa, 0xff4d6680, 0x00000000, 0xd62f3439, 0xff9d8783]),
    (BM_OVERWRITE | 200 << 8, false, [0x803700c8, 0x9bccd3db, 0x640d1926, 0xd6232629, 0xd4e44e35]),
    (BM_OVERWRITE | 255 << 8, true, [0x800000ff, 0xbf899aab, 0x00000000, 0xd6232629, 0xeea66d63]),
];
const STC_TRI: [(f32, f32); 3] = [(150.5, 30.5), (290.0, 60.0), (200.0, 190.0)];
const STC_BBOX: [i32; 4] = [10, 20, 290, 190];
const STC_FBOX: [i32; 4] = [0, 0, 5, 16];

fn bytes(px: &[u32]) -> Vec<u8> {
    px.iter().flat_map(|p| p.to_le_bytes()).collect()
}

/// known answers (from the numpy references of tools/blend_check.py, tmask_check.py,
/// flood_check.py) for blend modes, straight kernels, masks and the flood fill
fn selftest() -> i32 {
    let mut err = 0u32;
    let mut fail = |msg: &str| {
        err += 1;
        plat::write_stderr(msg.as_bytes());
    };
    // ---- blend modes: 5 pixels, bm_row and bm_cov_row (in place)
    for (i, &(modeop, cov, want)) in STC_CASES.iter().enumerate() {
        let mut w = bytes(&STC_LHS);
        let rhs = bytes(&STC_RHS);
        if cov {
            blendmode::bm_cov_row_inplace(&mut w, &rhs, Some(&STC_COV), 5, modeop);
        } else {
            blendmode::bm_row_inplace(&mut w, &rhs, modeop);
        }
        if w != bytes(&want) {
            fail(&format!("selftest: blend case {i} failed\n"));
        }
    }
    // ---- premultiply / unpremultiply
    let mut w = vec![0u8; 20];
    straight::premul_row(&mut w, &bytes(&STC_RHS));
    if w != bytes(&STC_PM) {
        fail("selftest: k_premul_row failed\n");
    }
    straight::unpremul_inplace(&mut w);
    if w != bytes(&STC_RHS) {
        fail("selftest: k_unpremul_row failed\n");
    }
    // ---- masks: rectangle + anti-aliased triangle, bbox, trace, get
    let mask_ok = (|| {
        let mut m = TMask::new(300, 200)?;
        m.fill_rect(10, 20, 110, 70, 255);
        let (mut path, mut ras) = (Path::default(), Raster::default());
        if m.raster_polygon(&mut path, &mut ras, &STC_TRI, RULE_NONZERO, TM_ADD) != 0 {
            return None;
        }
        if m.bbox()? != STC_BBOX {
            return None;
        }
        let mut n = 0;
        if m.trace(&mut |_| {
            n += 1;
            0
        }) != 0
            || n != 2
        {
            return None;
        }
        (m.get(200, 100) == 255).then_some(())
    })();
    if mask_ok.is_none() {
        fail("selftest: tmask failed\n");
    }
    // ---- flood: 16 x 16 white image with a black column at x = 5
    let flood_ok = (|| {
        let mut px = vec![0xFFu8; 16 * 16 * 4];
        for y in 0..16 {
            px[y * 64 + 20..y * 64 + 24].copy_from_slice(&0xFF000000u32.to_le_bytes());
        }
        let src = Surface { px, w: 16, h: 16, stride: 64, fmt: FMT_ARGB, ..Surface::default() };
        let mut m = TMask::new(16, 16)?;
        if flood_fill_mask(&src, 0, 0, 0, 0, &mut m) != 0 || m.bbox()? != STC_FBOX {
            return None;
        }
        if flood_fill_mask(&src, 9, 9, 0, FF_GLOBAL, &mut m) != 0 || m.get(5, 3) != 0 {
            return None;
        }
        (m.get(2, 3) == 255).then_some(())
    })();
    if flood_ok.is_none() {
        fail("selftest: flood_fill_mask failed\n");
    }
    plat::write_stdout(format!("selftest: {err} failures\n").as_bytes());
    (err != 0) as i32
}
