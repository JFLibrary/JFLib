// gfxtest -- rasterizer self tests and benchmarks          [apps/gfxtest/src/main.s, selftest.s]
//
//   gfxtest --shot x.png --selftest
//        a8:   a8_over against the scalar formula for every (dst, coverage, alpha) triple and
//              rows of n = 0..67 (the assembly also varies the alignments; the random
//              sequence is consumed the same way, so the later checks see the same data)
//        cpop: a clip pushed on a large target, then begin of a small one and an unbalanced
//              clip_pop: the clip must stay inside the small target
//        clip: 400 random paths (fills with both rules, strokes, ellipses, rounded
//              rectangles, far-away coordinates) per target format (PARGB, A8), rendered once
//              unclipped and through 24 random clip rectangles each: every pixel inside the
//              clip must equal the unclipped rendering, every pixel outside the background
//   gfxtest --shot x.png --bench     timings: fill (PARGB / A8 circle), tmask raster_path
//                                    (4000 x 3000 lasso), a8_over (Mpx/s)
// The output lines are those of the assembly version (tools/parity.py compares them).
// The test pages (shapes, text) need the UI canvas (R5) and the text features of R3; --open
// (plat_open_folder) comes with R4.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use base::Args;
use gfx::blend::a8_over;
use gfx::path::Path;
use gfx::raster::Raster;
use gfx::tmask::{TM_REPLACE, TMask};
use gfx::xform::sincos;
use gfx::{Dc, FMT_A8, FMT_PARGB, Surface};

const TW: i32 = 192; // test surfaces
const TH: i32 = 160;
const NCASE: u32 = 400; // random paths per target format
const NCLIP: u32 = 24; // clip rectangles per path
const AL_N: u32 = 68; // a8_over rows 0..67
const FXS: i32 = 256; // 24.8

ui::main!(gfxtest());

fn gfxtest() -> i32 {
    let args = Args(plat::args());
    if args.flag("--selftest") {
        return selftest();
    }
    if args.flag("--bench") {
        bench();
        return 0;
    }
    plat::write_stderr(b"gfxtest: the test pages need the UI canvas (R5); use --selftest or --bench\n");
    1
}

fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

fn err(s: &str) {
    plat::write_stderr(s.as_bytes());
}

/// xorshift32 and its scaled form (rnd * n >> 32)
struct Rng(u32);

impl Rng {
    fn rnd(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    fn rndn(&mut self, n: u32) -> u32 {
        ((self.rnd() as u64 * n as u64) >> 32) as u32
    }
}

struct T {
    rng: Rng,
    fails: u32,
    dc: Dc,
    path: Path,
    ras: Raster,
    // the random path's paint
    col: u32,
    rule: u32,
    stroke: bool,
    width: i32,
    style: u32,
}

fn selftest() -> i32 {
    let mut t = T {
        rng: Rng(0x2545_F491),
        fails: 0,
        dc: Dc::new(TW, TH),
        path: Path::default(),
        ras: Raster::default(),
        col: 0,
        rule: 0,
        stroke: false,
        width: 0,
        style: 0,
    };
    a8_test(&mut t);
    cpop_test(&mut t);
    let bg = clip_setup(&mut t);
    clip_test(&mut t, bg.0, FMT_PARGB);
    clip_test(&mut t, bg.1, FMT_A8);
    if t.fails == 0 {
        out("selftest: all checks passed\n");
        0
    } else {
        err(&format!("selftest: {} check(s) FAILED\n", t.fails));
        1
    }
}

// ---- clip stack across targets

fn cpop_test(t: &mut T) {
    let big = Surface::new(TW, TH, FMT_PARGB);
    let small = Surface::new(16, 16, FMT_PARGB);
    t.dc.begin(&big);
    t.dc.clip_push(4, 4, 100, 100);
    t.dc.begin(&small);
    t.dc.clip_pop(); // (unbalanced in the small target)
    let c = t.dc.clip;
    if c[2] > 16 || c[3] > 16 || c[0] < 0 || c[1] < 0 {
        err(&format!("cpop: clip {},{}..{},{} outside the 16 x 16 target\n", c[0], c[1], c[2], c[3]));
        t.fails += 1;
    } else {
        out("cpop: clip stays inside the target after an unbalanced pop\n");
    }
    t.dc.end();
    t.dc.clip_pop();
    t.dc.end();
}

// ---- a8_over

// c' = c*a/255, d + d*(255 - c')/255 -- the scalar formula of raster.s
fn a8_ref(d: u32, c: u32, a: u32) -> u32 {
    let x = c * a + 128;
    let c2 = (x + (x >> 8)) >> 8;
    let y = (255 - c2) * d + 128;
    ((y + (y >> 8)) >> 8) + c2
}

fn a8_test(t: &mut T) {
    let mut bad = 0u32;
    let cov: Vec<u8> = (0..=255).collect();
    for alpha in 0..256u32 {
        for dv in 0..256u32 {
            let mut dst = [dv as u8; 256];
            a8_over(&mut dst, &cov, alpha);
            for (c, &got) in dst.iter().enumerate() {
                let want = a8_ref(dv, c as u32, alpha);
                if got as u32 != want {
                    bad += 1;
                    if bad <= 4 {
                        err(&format!("a8: dst {dv} cov {c} alpha {alpha} -> {got}, expected {want}\n"));
                    }
                }
            }
        }
    }
    // rows of every length (and, in the assembly, every alignment: the same random numbers)
    for n in 0..AL_N {
        for ad in 0..16u32 {
            for ac in 0..16u32 {
                let style = t.rng.rnd();
                let (mut dst, mut rf, mut cv) = (vec![0u8; n as usize], vec![0u8; n as usize], vec![0u8; n as usize]);
                for b in 0..n as usize {
                    let v = t.rng.rnd() as u8;
                    dst[b] = v;
                    rf[b] = v;
                    let r = t.rng.rnd();
                    let k = (b >> 3) & 3; // runs of 8
                    cv[b] = match (style >> (2 * k)) & 3 {
                        1 => 0,
                        2 => 255,
                        _ => r as u8,
                    };
                }
                let r = t.rng.rnd();
                let alpha = match (r >> 8) & 3 {
                    1 => 255,
                    2 => 0,
                    _ => r & 255,
                };
                a8_over(&mut dst, &cv, alpha);
                if let Some(b) = (0..n as usize).find(|&b| dst[b] as u32 != a8_ref(rf[b] as u32, cv[b] as u32, alpha)) {
                    bad += 1;
                    if bad <= 8 {
                        err(&format!("a8: row n {n} dst align {ad} cov align {ac} wrong at byte {}\n", b + 16 + ad as usize));
                    }
                }
            }
        }
    }
    if bad != 0 {
        err(&format!("a8: {bad} mismatches\n"));
        t.fails += 1;
    } else {
        out("a8: k_a8_over exhaustive + alignments ok\n");
    }
}

// ---- clipped vs unclipped rendering

// backgrounds: PARGB random premultiplied (half opaque), A8 random
fn clip_setup(t: &mut T) -> (Surface, Surface) {
    let mut p = Surface::new(TW, TH, FMT_PARGB);
    let mut a = Surface::new(TW, TH, FMT_A8);
    for y in 0..TH as usize {
        for x in 0..TW as usize {
            let c = t.rng.rnd();
            let r = t.rng.rnd();
            let al = if r & 1 == 0 { 255 } else { (r >> 8) & 255 };
            let v = ((c & 255) * al) >> 8 | ((c >> 8 & 255) * al) & 0xFF00 | (((c >> 16 & 255) * al) << 8) & 0xFF_0000 | al << 24;
            let o = y * p.stride as usize + x * 4;
            p.px[o..o + 4].copy_from_slice(&v.to_le_bytes());
            a.px[y * a.stride as usize + x] = c as u8;
        }
    }
    (p, a)
}

// a 24.8 coordinate: mostly around the surface (48 px margin), sometimes far away (+-3000 px)
fn rp_coord(rng: &mut Rng, extent: i32) -> i32 {
    if rng.rndn(16) == 0 {
        rng.rndn((6000 * FXS) as u32) as i32 - 3000 * FXS
    } else {
        rng.rndn(((extent + 96) << 8) as u32) as i32 - 48 * FXS
    }
}

// a random path plus colour, rule and mode
fn rp_build(t: &mut T) {
    t.path.begin(&t.dc);
    let r = &mut t.rng;
    t.col = r.rnd() | 0x0100_0000; // never fully transparent
    let m = r.rnd();
    t.rule = m & 1;
    t.stroke = m & 6 == 0; // 1 of 4: width 0.3 .. 16 px, any join / cap
    t.width = r.rndn((16 * FXS) as u32) as i32 + 77;
    let join = r.rndn(3);
    t.style = r.rndn(3) << 4 | join;
    match r.rndn(8) {
        0 => {
            let cx = rp_coord(r, TW);
            let cy = rp_coord(r, TH);
            let rx = r.rndn((120 * FXS) as u32) as i32 + 64;
            let ry = r.rndn((120 * FXS) as u32) as i32 + 64;
            t.path.ellipse_fx(cx, cy, rx, ry);
        }
        1 => {
            let x = rp_coord(r, TW);
            let y = rp_coord(r, TH);
            let w = r.rndn((200 * FXS) as u32) as i32 + 32;
            let h = r.rndn((200 * FXS) as u32) as i32 + 32;
            let rr = r.rndn((40 * FXS) as u32) as i32;
            t.path.rrect_fx(x, y, w, h, rr);
        }
        _ => {
            // polygons and curves
            for _ in 0..r.rndn(3) + 1 {
                let x = rp_coord(r, TW);
                let y = rp_coord(r, TH);
                t.path.move_fx(x, y);
                for _ in 0..r.rndn(10) + 1 {
                    match r.rndn(4) {
                        2 => {
                            let (a, b, c, d) = (rp_coord(r, TW), rp_coord(r, TH), rp_coord(r, TW), rp_coord(r, TH));
                            t.path.quad_fx(a, b, c, d);
                        }
                        3 => {
                            let (a, b, c, d) = (rp_coord(r, TW), rp_coord(r, TH), rp_coord(r, TW), rp_coord(r, TH));
                            let (e, f) = (rp_coord(r, TW), rp_coord(r, TH));
                            t.path.cubic_fx(a, b, c, d, e, f);
                        }
                        _ => {
                            let (a, b) = (rp_coord(r, TW), rp_coord(r, TH));
                            t.path.line_fx(a, b);
                        }
                    }
                }
                if r.rnd() & 1 != 0 {
                    t.path.close();
                }
            }
        }
    }
}

// the path into sf through the clip rectangle
fn render(t: &mut T, sf: &mut Surface, x: i32, y: i32, w: i32, h: i32) {
    t.dc.begin(sf);
    t.dc.clip_push(x, y, w, h);
    t.dc.set_color(t.col);
    if t.stroke {
        t.path.stroke(&mut t.ras, &t.dc, sf, t.width, t.style);
    } else {
        t.path.fill(&mut t.ras, &t.dc, sf, t.rule);
    }
    t.dc.clip_pop();
    t.dc.end();
}

fn clip_test(t: &mut T, bg: Surface, fmt: u8) {
    let bpp = if fmt == FMT_A8 { 1 } else { 4 };
    let copy = |s: &Surface| Surface { px: s.px.clone(), w: s.w, h: s.h, stride: s.stride, fmt: s.fmt, ..Surface::default() };
    let (mut full, mut clipped) = (copy(&bg), copy(&bg)); // reset from bg before each render
    let mut bad_cases = 0u32;
    for case in 0..NCASE {
        rp_build(t);
        full.px.copy_from_slice(&bg.px);
        render(t, &mut full, 0, 0, TW, TH);
        for _ in 0..NCLIP {
            // every other clip a run of 32 px dirty cells, else any rectangle
            let r = &mut t.rng;
            let (x, y, w, h) = if r.rnd() & 1 != 0 {
                let x = r.rndn((TW / 32) as u32) as i32 * 32;
                let y = r.rndn((TH / 32) as u32) as i32 * 32;
                let w = (r.rndn(3) as i32 + 1) * 32;
                let h = (r.rndn(2) as i32 + 1) * 32;
                (x, y, w, h)
            } else {
                let x = r.rndn(TW as u32) as i32;
                let y = r.rndn(TH as u32) as i32;
                let w = r.rndn(TW as u32) as i32 + 1;
                let h = r.rndn(TH as u32) as i32 + 1;
                (x, y, w, h)
            };
            clipped.px.copy_from_slice(&bg.px);
            render(t, &mut clipped, x, y, w, h);
            // inside the clip = full rendering, outside = background; row by row in three
            // parts (perf-raster: slice compares instead of a pixel loop; the first
            // difference in row order is reported as before)
            if let Some((px, py, g, e)) = first_diff(&clipped, &full, &bg, (x, y, w, h), bpp) {
                if bad_cases < 3 {
                    err(&format!("clip: case {case} pixel {px},{py} got {g:x} expected {e:x} (clip at {x},{y})\n"));
                }
                bad_cases += 1;
                break; // one failing clip per case is enough
            }
        }
    }
    let name = if fmt == FMT_A8 { "A8" } else { "PARGB" };
    if bad_cases == 0 {
        out(&format!("clip: {name} {NCASE} paths x {NCLIP} clips bit-identical\n"));
    } else {
        err(&format!("clip: {name} {bad_cases} of the paths differ under a clip\n"));
        t.fails += 1;
    }
}

// a == b for slices of the same length, 8 bytes at a time without an early exit (the
// compiler vectorizes it; the byte loop of memcmp would dominate the self test)
fn same(a: &[u8], b: &[u8]) -> bool {
    let (ac, at) = a.as_chunks::<8>();
    let (bc, bt) = b.as_chunks::<8>();
    let d = ac.iter().zip(bc).fold(0u64, |d, (x, y)| d | (u64::from_ne_bytes(*x) ^ u64::from_ne_bytes(*y)));
    d == 0 && at.iter().zip(bt).all(|(x, y)| x == y)
}

// the first pixel (row order) of `got` that differs from `full` inside the clip rectangle
// (x, y, w, h) or from `bg` outside it: (x, y, got, expected)
fn first_diff(got: &Surface, full: &Surface, bg: &Surface, clip: (i32, i32, i32, i32), bpp: usize) -> Option<(i32, i32, u32, u32)> {
    let (x, y, w, h) = clip;
    let (cx0, cx1) = (x.clamp(0, TW) as usize, (x + w).clamp(0, TW) as usize);
    let stride = bg.stride as usize;
    let rowb = TW as usize * bpp;
    for py in 0..TH {
        let o = py as usize * stride;
        let g = &got.px[o..o + rowb];
        let inside = py >= y && py < y + h;
        let (fr, br) = (&full.px[o..o + rowb], &bg.px[o..o + rowb]);
        let (a, b) = if inside { (cx0 * bpp, cx1 * bpp) } else { (rowb, rowb) };
        // [0, a) and [b, rowb) against the background, [a, b) against the full rendering
        if same(&g[..a], &br[..a]) && same(&g[a..b], &fr[a..b]) && same(&g[b..], &br[b..]) {
            continue;
        }
        for px in 0..TW as usize {
            let e = if px * bpp >= a && px * bpp < b { fr } else { br };
            let k = px * bpp;
            if g[k..k + bpp] != e[k..k + bpp] {
                let val = |s: &[u8]| if bpp == 1 { s[k] as u32 } else { u32::from_le_bytes([s[k], s[k + 1], s[k + 2], s[k + 3]]) };
                return Some((px as i32, py, val(g), val(e)));
            }
        }
    }
    None
}

// ---- benchmarks

fn bench() {
    let mut rng = Rng(12345);
    let mut dc = Dc::new(64, 64);
    let mut path = Path::default();
    let mut ras = Raster::default();
    // fill: a circle of radius 480 into a 1024 x 1024 PARGB and A8 target
    for fmt in [FMT_PARGB, FMT_A8] {
        let mut sf = Surface::new(1024, 1024, fmt);
        dc.begin(&sf);
        dc.set_color(0x8033_6699);
        path.begin(&dc);
        path.ellipse_fx(512 * FXS + 77, 512 * FXS + 33, 480 * FXS, 470 * FXS);
        let mut best = u64::MAX; // best of 9 rounds of 10 fills
        for _ in 0..9 {
            let t0 = plat::time_us();
            for _ in 0..10 {
                path.fill(&mut ras, &dc, &mut sf, 0);
            }
            best = best.min(plat::time_us() - t0);
        }
        dc.end();
        let name = if fmt == FMT_A8 { "A8" } else { "PARGB" };
        out(&format!("bench: ras_fill circle r 480 into {name} 1024x1024: {} us\n", best / 10));
    }
    // tmask raster_path: 4000 x 3000 lasso with 3000 points
    if let Some(mut tm) = TMask::new(4000, 3000) {
        path.reset();
        for i in 0..3000u32 {
            let (s, c) = sincos(i as f64 * 0.0020943951023931952); // 2 pi / 3000
            let r = (1300 + 40 * (i * 37 % 11)) as f64;
            let (x, y) = ((c * r + 2000.3) as f32, (s * r + 1500.6) as f32);
            if i == 0 {
                path.move_f(x, y);
            } else {
                path.line_f(x, y);
            }
        }
        let mut best = u64::MAX; // best of 7 rounds of 2
        for _ in 0..7 {
            let t0 = plat::time_us();
            for _ in 0..2 {
                tm.raster_path(&path, &mut ras, 0, TM_REPLACE);
            }
            best = best.min(plat::time_us() - t0);
        }
        out(&format!("bench: tm_raster_path 4000x3000 lasso (3000 points): {} us\n", best / 2));
    }
    // a8_over: 4096 px rows, alpha 200, random coverage
    let cov: Vec<u8> = (0..4096).map(|_| rng.rnd() as u8).collect();
    let mut dst = vec![0u8; 4096];
    let mut best = u64::MAX; // best of 9 rounds of 2000 rows
    for _ in 0..9 {
        let t0 = plat::time_us();
        for _ in 0..2000 {
            a8_over(&mut dst, &cov, 200);
        }
        best = best.min(plat::time_us() - t0);
    }
    if best != 0 {
        out(&format!("bench: k_a8_over {} Mpx/s\n", 4096 * 2000 / best));
    }
}
