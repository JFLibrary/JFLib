// pct -- JPEG XR sample reconstruction: inverse PCT and overlap post filters (ITU-T T.832 9.9)
//                                                                    [src/img/jxr_pct.s]
//
//   ict4x4(v)        ICT4x4 of 16 i32 in place (v = ICT input in 4x4 raster order after
//                    InvPermute)
//   recon(jx)        first level overlap filtering (OVERLAP_MODE 2) of the DC grids, second
//                    level inverse transform of every 4x4 block (DC taken from the grid) and
//                    second level overlap filtering (OVERLAP_MODE 1 / 2) of the sample planes of
//                    all image planes of the codestream
//
// All arithmetic is the assembly's 32-bit two's complement (wrapping) arithmetic.

use crate::ent::Cs;

#[inline(always)]
fn ad(a: i32, b: i32) -> i32 {
    a.wrapping_add(b)
}

#[inline(always)]
fn sb(a: i32, b: i32) -> i32 {
    a.wrapping_sub(b)
}

/// (3 x + r) >> s
#[inline(always)]
fn m3(x: i32, r: i32, s: u32) -> i32 {
    x.wrapping_mul(3).wrapping_add(r) >> s
}

/// (x + r) >> s
#[inline(always)]
fn rs(x: i32, r: i32, s: u32) -> i32 {
    x.wrapping_add(r) >> s
}

/// T2x2h a, b, c, d, rnd -- 9.9.7.2 (its own inverse)
#[inline(always)]
pub fn t2(v: &mut [i32; 16], a: usize, b: usize, c: usize, d: usize, rnd: i32) {
    let (mut x0, mut x1, x2, x3) = (v[a], v[b], v[c], v[d]);
    x0 = ad(x0, x3);
    x1 = sb(x1, x2);
    let t = ad(sb(x0, x1), rnd) >> 1;
    let c2 = sb(t, x3);
    let d2 = sb(t, x2);
    x0 = sb(x0, d2);
    x1 = ad(x1, c2);
    v[a] = x0;
    v[b] = x1;
    v[c] = c2;
    v[d] = d2;
}

/// InvTodd -- 9.9.7.3
#[inline(always)]
fn todd(v: &mut [i32; 16], a: usize, b: usize, c: usize, d: usize) {
    let (mut c0, mut c1, mut c2, mut c3) = (v[a], v[b], v[c], v[d]);
    c1 = ad(c1, c3);
    c0 = sb(c0, c2);
    c3 = sb(c3, c1 >> 1);
    c2 = ad(c2, rs(c0, 1, 1));
    c0 = sb(c0, m3(c1, 4, 3));
    c1 = ad(c1, m3(c0, 4, 3));
    c2 = sb(c2, m3(c3, 4, 3));
    c3 = ad(c3, m3(c2, 4, 3));
    c2 = sb(c2, rs(c1, 1, 1));
    c3 = sb(rs(c0, 1, 1), c3);
    c1 = ad(c1, c2);
    c0 = sb(c0, c3);
    v[a] = c0;
    v[b] = c1;
    v[c] = c2;
    v[d] = c3;
}

/// InvToddodd -- 9.9.7.4
#[inline(always)]
fn toddodd(v: &mut [i32; 16], a: usize, b: usize, c: usize, d: usize) {
    let (mut c0, mut c1, mut c2, mut c3) = (v[a], v[b], v[c], v[d]);
    c3 = ad(c3, c0);
    c2 = sb(c2, c1);
    let t1 = c3 >> 1;
    let t2 = c2 >> 1;
    c0 = sb(c0, t1);
    c1 = ad(c1, t2);
    c0 = sb(c0, m3(c1, 3, 3));
    c1 = ad(c1, m3(c0, 3, 2));
    c0 = sb(c0, m3(c1, 4, 3));
    c1 = sb(c1, t2);
    c0 = ad(c0, t1);
    c2 = ad(c2, c1);
    c3 = sb(c3, c0);
    v[a] = c0;
    v[b] = c1.wrapping_neg();
    v[c] = c2.wrapping_neg();
    v[d] = c3;
}

/// ICT4x4 (9.9.7.1) of 16 i32 in place  [jxp_ict4x4]
#[inline]
pub fn ict4x4(v: &mut [i32; 16]) {
    t2(v, 0, 1, 4, 5, 1);
    todd(v, 2, 3, 6, 7);
    todd(v, 8, 12, 9, 13);
    toddodd(v, 10, 11, 14, 15);
    t2(v, 0, 3, 12, 15, 0);
    t2(v, 5, 6, 9, 10, 0);
    t2(v, 1, 2, 13, 14, 0);
    t2(v, 4, 7, 8, 11, 0);
}

// ---------------------------------------------------------------- overlap post filters

/// InvScale -- 9.9.8.6
#[inline(always)]
fn invscale(a: &mut i32, b: &mut i32) {
    *a = ad(*a, *b);
    *b = sb(*a >> 1, *b);
    *a = ad(*a, b.wrapping_mul(3) >> 3);
    *b = ad(*b, a.wrapping_mul(3) >> 4);
    *b = ad(*b, *a >> 7);
    *b = sb(*b, *a >> 10);
}

/// InvRotate -- 9.9.8.5
#[inline(always)]
fn invrot(a: &mut i32, b: &mut i32) {
    *a = sb(*a, rs(*b, 1, 1));
    *b = ad(*b, rs(*a, 1, 1));
}

#[inline(always)]
fn rotm(v: &mut [i32; 16], a: usize, b: usize) {
    let (mut x, mut y) = (v[a], v[b]);
    invrot(&mut x, &mut y);
    v[a] = x;
    v[b] = y;
}

#[inline(always)]
fn scalem(v: &mut [i32; 16], a: usize, b: usize) {
    let (mut x, mut y) = (v[a], v[b]);
    invscale(&mut x, &mut y);
    v[a] = x;
    v[b] = y;
}

/// T2x2hPOST -- 9.9.8.7
#[inline(always)]
fn t2post(v: &mut [i32; 16], a: usize, b: usize, c: usize, d: usize) {
    let (mut c0, mut c1, c2, mut c3) = (v[a], v[b], v[c], v[d]);
    c1 = sb(c1, c2);
    c0 = ad(c0, m3(c3, 4, 3));
    c3 = sb(c3, c1 >> 1);
    let t = sb(sb(c0, c1) >> 1, c2);
    let n2 = c3; // swap c2, c3
    let n3 = t;
    c0 = sb(c0, n3);
    c1 = ad(c1, n2);
    v[a] = c0;
    v[b] = c1;
    v[c] = n2;
    v[d] = n3;
}

/// OverlapPostFilter4x4 (9.9.8.1) on a 4x4 raster region  [post4x4]
#[inline]
fn post4x4(v: &mut [i32; 16]) {
    t2(v, 0, 3, 12, 15, 0);
    t2(v, 1, 2, 13, 14, 0);
    t2(v, 4, 7, 8, 11, 0);
    t2(v, 5, 6, 9, 10, 0);
    rotm(v, 13, 12);
    rotm(v, 9, 8);
    rotm(v, 7, 3);
    rotm(v, 6, 2);
    // InvToddoddPOST 10, 11, 14, 15
    let (mut c0, mut c1, mut c2, mut c3) = (v[10], v[11], v[14], v[15]);
    c3 = ad(c3, c0);
    c2 = sb(c2, c1);
    let t1 = c3 >> 1;
    let t2v = c2 >> 1;
    c0 = sb(c0, t1);
    c1 = ad(c1, t2v);
    c0 = sb(c0, m3(c1, 6, 3));
    c1 = ad(c1, m3(c0, 2, 2));
    c0 = sb(c0, m3(c1, 4, 3));
    c1 = sb(c1, t2v);
    c0 = ad(c0, t1);
    c2 = ad(c2, c1);
    c3 = sb(c3, c0);
    v[10] = c0;
    v[11] = c1;
    v[14] = c2;
    v[15] = c3;
    scalem(v, 0, 15);
    scalem(v, 1, 14);
    scalem(v, 4, 11);
    scalem(v, 5, 10);
    t2post(v, 0, 3, 12, 15);
    t2post(v, 1, 2, 13, 14);
    t2post(v, 4, 7, 8, 11);
    t2post(v, 5, 6, 9, 10);
}

/// OverlapPostFilter4 (9.9.8.2)  [post4]
#[inline]
fn post4(v: &mut [i32; 4]) {
    let (mut c0, mut c1, mut c2, mut c3) = (v[0], v[1], v[2], v[3]);
    c0 = ad(c0, c3);
    c1 = ad(c1, c2);
    c3 = sb(c3, rs(c0, 1, 1));
    c2 = sb(c2, rs(c1, 1, 1));
    invscale(&mut c0, &mut c3);
    invscale(&mut c1, &mut c2);
    c0 = ad(c0, m3(c3, 4, 3));
    c1 = ad(c1, m3(c2, 4, 3));
    c3 = sb(c3, c0 >> 1);
    c2 = sb(c2, c1 >> 1);
    c0 = ad(c0, c3);
    c1 = ad(c1, c2);
    c3 = c3.wrapping_neg();
    c2 = c2.wrapping_neg();
    invrot(&mut c2, &mut c3);
    c3 = ad(c3, rs(c0, 1, 1));
    c2 = ad(c2, rs(c1, 1, 1));
    c0 = sb(c0, c3);
    c1 = sb(c1, c2);
    *v = [c0, c1, c2, c3];
}

/// OverlapPostFilter2 (9.9.8.4)  [post2]
#[inline]
fn post2(mut c0: i32, mut c1: i32) -> (i32, i32) {
    c1 = ad(c1, rs(c0, 2, 2));
    c0 = ad(c0, rs(c1, 1, 1));
    c0 = ad(c0, c1 >> 5);
    c0 = ad(c0, c1 >> 9);
    c0 = ad(c0, c1 >> 13);
    c1 = ad(c1, rs(c0, 2, 2));
    (c0, c1)
}

/// OverlapPostFilter2x2 (9.9.8.3) of TL TR BL BR  [post2x2]
#[inline]
fn post2x2(v: &mut [i32; 4]) {
    let (mut c0, mut c1, mut c2, mut c3) = (v[0], v[1], v[2], v[3]);
    c0 = ad(c0, c3);
    c1 = ad(c1, c2);
    c3 = sb(c3, rs(c0, 1, 1));
    c2 = sb(c2, rs(c1, 1, 1));
    (c0, c1) = post2(c0, c1);
    c3 = ad(c3, rs(c0, 1, 1));
    c2 = ad(c2, rs(c1, 1, 1));
    c0 = sb(c0, c3);
    c1 = sb(c1, c2);
    *v = [c0, c1, c2, c3];
}

// ---------------------------------------------------------------- whole planes

/// overlap filtering of a whole plane at block granularity 4 (second level of the sample
/// planes, first level of 4 x 4 DC grids): 4x4 at interior corners, 4-point filters across
/// block boundaries on the two outermost rows / columns, 4-point filters on the four 2x2
/// corners; stride = w  [post_plane]
fn post_plane(p: &mut [i32], w: usize, h: usize) {
    // interior 4x4
    let mut y = 4;
    while y + 4 <= h {
        let mut x = 4;
        while x + 4 <= w {
            let o = (y - 2) * w + x - 2;
            let mut v = [0i32; 16];
            for r in 0..4 {
                v[r * 4..r * 4 + 4].copy_from_slice(&p[o + r * w..o + r * w + 4]);
            }
            post4x4(&mut v);
            for r in 0..4 {
                p[o + r * w..o + r * w + 4].copy_from_slice(&v[r * 4..r * 4 + 4]);
            }
            x += 4;
        }
        y += 4;
    }
    // rows 0, 1, h - 2, h - 1 across vertical block boundaries
    for sel in 0..4 {
        let row = if sel < 2 { sel } else { h - 4 + sel };
        let mut x = 4;
        while x + 4 <= w {
            let o = row * w + x - 2;
            let mut v = [p[o], p[o + 1], p[o + 2], p[o + 3]];
            post4(&mut v);
            p[o..o + 4].copy_from_slice(&v);
            x += 4;
        }
    }
    // columns 0, 1, w - 2, w - 1 across horizontal block boundaries
    for sel in 0..4 {
        let col = if sel < 2 { sel } else { w - 4 + sel };
        let mut y = 4;
        while y + 4 <= h {
            let o = (y - 2) * w + col;
            let mut v = [p[o], p[o + w], p[o + 2 * w], p[o + 3 * w]];
            post4(&mut v);
            p[o] = v[0];
            p[o + w] = v[1];
            p[o + 2 * w] = v[2];
            p[o + 3 * w] = v[3];
            y += 4;
        }
    }
    // corners: 4-point filter on (r, c), (r, c + 1), (r + 1, c), (r + 1, c + 1)
    for sel in 0..4 {
        let r = if sel & 2 != 0 { h - 2 } else { 0 };
        let c = if sel & 1 != 0 { w - 2 } else { 0 };
        let o = r * w + c;
        let mut v = [p[o], p[o + 1], p[o + w], p[o + w + 1]];
        post4(&mut v);
        p[o] = v[0];
        p[o + 1] = v[1];
        p[o + w] = v[2];
        p[o + w + 1] = v[3];
    }
}

/// OverlapPostFilter1 at the four image corners: the corner cell minus / plus its horizontal
/// neighbour (TL, TR, BL, BR)  [uv_corner_sub / uv_corner_add]
fn uv_corner(p: &mut [i32], w: usize, h: usize, add: bool) {
    for row in [0, h - 1] {
        let o = row * w;
        let n = p[o + 1];
        p[o] = if add { ad(p[o], n) } else { sb(p[o], n) };
        let n = p[o + w - 2];
        p[o + w - 1] = if add { ad(p[o + w - 1], n) } else { sb(p[o + w - 1], n) };
    }
}

/// first level overlap filtering of a 4:2:0 / 4:2:2 chroma DC grid (9.9.3.3 / 9.9.3.4, soft
/// tiles): corner residual differences, 2x2 filters at every second row / column crossing,
/// 2-point filters along the outer rows / columns, corner residual additions  [post_uv]
fn post_uv(p: &mut [i32], w: usize, h: usize) {
    uv_corner(p, w, h, false);
    let mut y = 2;
    while y < h {
        let mut x = 2;
        while x < w {
            let o = (y - 1) * w + x - 1;
            let mut v = [p[o], p[o + 1], p[o + w], p[o + w + 1]];
            post2x2(&mut v);
            p[o] = v[0];
            p[o + 1] = v[1];
            p[o + w] = v[2];
            p[o + w + 1] = v[3];
            x += 2;
        }
        y += 2;
    }
    // left / right columns: pairs (y - 1, y)
    let mut y = 2;
    while y < h {
        let mut col = 0;
        loop {
            let o = (y - 1) * w + col;
            let (a, b) = post2(p[o], p[o + w]);
            p[o] = a;
            p[o + w] = b;
            if col == w - 1 {
                break;
            }
            col = w - 1;
        }
        y += 2;
    }
    // top / bottom rows: pairs (x - 1, x)
    let mut row = 0;
    loop {
        let o = row * w;
        let mut x = 2;
        while x < w {
            let (a, b) = post2(p[o + x - 1], p[o + x]);
            p[o + x - 1] = a;
            p[o + x] = b;
            x += 2;
        }
        if row == h - 1 {
            break;
        }
        row = h - 1;
    }
    uv_corner(p, w, h, true);
}

/// second level inverse transform: every 4x4 block gets its DC from the grid (w / 4 per row),
/// ICT4x4 in place  [ict_plane]
fn ict_plane(p: &mut [i32], w: usize, h: usize, grid: &[i32]) {
    let mut gi = 0;
    let mut y = 0;
    while y < h {
        let row = &mut p[y * w..(y + 4) * w];
        let mut x = 0;
        while x < w {
            let mut v = [0i32; 16];
            for r in 0..4 {
                v[r * 4..r * 4 + 4].copy_from_slice(&row[r * w + x..r * w + x + 4]);
            }
            v[0] = grid[gi];
            gi += 1;
            ict4x4(&mut v);
            for r in 0..4 {
                row[r * w + x..r * w + x + 4].copy_from_slice(&v[r * 4..r * 4 + 4]);
            }
            x += 4;
        }
        y += 4;
    }
}

/// see the module header  [jxp_recon]
pub fn recon(jx: &mut Cs) {
    let ovl = jx.ovl;
    for p in 0..jx.npl as usize {
        let pl = &mut jx.pl[p];
        for c in 0..pl.nch as usize {
            let (gw, gh) = (pl.gw[c] as usize, pl.gh[c] as usize);
            if ovl == 2 {
                if c != 0 && (pl.cf == 1 || pl.cf == 2) {
                    post_uv(&mut pl.dcg[c], gw, gh);
                } else {
                    post_plane(&mut pl.dcg[c], gw, gh);
                }
            }
            let (pw, ph) = (pl.pw[c] as usize, pl.ph[c] as usize);
            ict_plane(&mut pl.buf[c], pw, ph, &pl.dcg[c]);
            if ovl != 0 {
                post_plane(&mut pl.buf[c], pw, ph);
            }
        }
    }
}
