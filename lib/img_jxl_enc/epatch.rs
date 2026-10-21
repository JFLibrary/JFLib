// epatch -- patches for the new writer: repeated small content (text glyphs, icons) found in the
// image goes once into a reference-only frame and is added back at every place it occurs
// (libjxl 0.11.1 enc_patch_dictionary.cc: FindTextLikePatches, FindBestPatchDictionary's bin
// packing, PatchDictionaryEncoder::Encode / SubtractFrom)                [jxl-tools: no asm]
//
//   find(planes, s, w, h, xyb) -> Vec<Found>   FindTextLikePatches on three float planes (row
//                             stride s, image w x h): XYB (lossy) or the colour values / 255
//                             (lossless); the deduplicated patches that occur twice or more
//   pack(found, nec) -> Packed  the reference frame (bin packing as libjxl: largest first) and
//                             the dictionary (colour kAdd, extra channels kNone)
//   Packed::subtract(planes, s)   SubtractFrom with the reference frame's (decoded) values
//   Dict / dict.write(jw)     the dictionary in the frame's LfGlobal section
//
// libjxl uses patches from effort 5 (lossless) / 7 (lossy) on; enew.rs / evdct.rs call this
// at those efforts.  Detection: 4x4 blocks of one colour with a mostly equal neighbourhood
// ("screenshot-like") seed a flood fill of the background (similar colours within 50 pixels);
// what is left are small connected components on a uniform background: those of at most 32 x
// 32 pixels that stand out from it become patches, identical ones (after a coarse
// quantization) share one rectangle of the reference frame.

use alloc::vec;
use alloc::vec::Vec;

use crate::eans::{self, pack_signed, HParams, Token};
use crate::ewr::Jw;

const CTX_NREF: u32 = 0;
const CTX_REF: u32 = 1;
const CTX_SIZE: u32 = 2;
const CTX_REFPOS: u32 = 3;
const CTX_POS: u32 = 4;
const CTX_BLEND: u32 = 5;
const CTX_OFFSET: u32 = 6;
const CTX_COUNT: u32 = 7;
const NUM_CTX: usize = 10;
const PB_NONE: u32 = 0;
const PB_ADD: u32 = 2;

const PATCH_SIDE: usize = 4;
const EXTRA_SIDE: i64 = 4;
const SEARCH_RADIUS: i64 = 1;
const DISTANCE_LIMIT: i64 = 50;
const SIMILAR: f32 = 0.8;
const VERY_SIMILAR: f32 = 0.03;
const HAS_SIMILAR: f32 = 0.03;
const HAS_SIMILAR_RADIUS: i64 = 2;
const MIN_PEAK: i32 = 2;
const MAX_PATCH: usize = 32;
const MIN_OCCURRENCES: usize = 2;
const MIN_MAX_PATCH_PIXELS: usize = 20;
const SLACK: f32 = 1.05;

/// the reference frame slot of the patches
pub const PATCH_REF: u32 = 0;

/// PatchColorspaceInfo
struct Space {
    dequant: [f32; 3],
    weights: [f32; 3],
}

impl Space {
    fn new(xyb: bool) -> Space {
        if xyb {
            Space { dequant: [0.01615, 0.08875, 0.1922], weights: [30.0, 3.0, 1.0] }
        } else {
            Space { dequant: [20.0 / 255.0, 22.0 / 255.0, 20.0 / 255.0], weights: [0.017 * 255.0, 0.02 * 255.0, 0.017 * 255.0] }
        }
    }
    #[inline]
    fn quantize(&self, v: f32, c: usize) -> i32 {
        (v / self.dequant[c]) as i32
    }
    #[inline]
    fn similar(&self, a: [f32; 3], b: [f32; 3], thr: f32) -> bool {
        let mut d = 0f32;
        for c in 0..3 {
            d += (a[c] - b[c]).abs() * self.weights[c];
        }
        d <= thr
    }
}

/// a deduplicated patch: size, quantized pixels (its identity), the pixels minus the background
/// (of its first occurrence), every position
pub struct Found {
    pub w: usize,
    pub h: usize,
    q: [Vec<i8>; 3],
    pub f: [Vec<f32>; 3],
    pub pos: Vec<(u32, u32)>,
}

impl Found {
    /// a patch at one position without an identity key (dots, edots.rs)
    pub(crate) fn single(w: usize, h: usize, f: [Vec<f32>; 3], pos: (u32, u32)) -> Found {
        Found { w, h, q: [Vec::new(), Vec::new(), Vec::new()], f, pos: vec![pos] }
    }

    fn key_cmp(&self, o: &Found) -> core::cmp::Ordering {
        (self.w, self.h).cmp(&(o.w, o.h)).then_with(|| {
            for c in 0..3 {
                // memcmp: bytes as unsigned
                let a = self.q[c].iter().map(|&v| v as u8);
                let b = o.q[c].iter().map(|&v| v as u8);
                let r = a.cmp(b);
                if r != core::cmp::Ordering::Equal {
                    return r;
                }
            }
            core::cmp::Ordering::Equal
        })
    }
}

/// FindTextLikePatches (see the module header)
pub fn find(p: [&[f32]; 3], s: usize, w: usize, h: usize, xyb: bool) -> Vec<Found> {
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let sp = Space::new(xyb);
    let px = |i: usize| -> [f32; 3] { [p[0][i], p[1][i], p[2][i]] };
    let at = |x: usize, y: usize| y * s + x;
    let same = |a: usize, b: usize| p[0][a] == p[0][b] && p[1][a] == p[1][b] && p[2][a] == p[2][b];
    // 4x4 blocks of one colour whose 12x12 neighbourhood is mostly that colour
    let sw = w.div_ceil(PATCH_SIDE);
    let sh = h.div_ceil(PATCH_SIDE);
    let mut shot = vec![0u8; sw * sh];
    let mut any = false;
    for by in 0..h / PATCH_SIDE {
        for bx in 0..w / PATCH_SIDE {
            let c0 = at(bx * PATCH_SIDE, by * PATCH_SIDE);
            let mut all = true;
            'b: for iy in 0..PATCH_SIDE {
                for ix in 0..PATCH_SIDE {
                    if !same(at(bx * PATCH_SIDE + ix, by * PATCH_SIDE + iy), c0) {
                        all = false;
                        break 'b;
                    }
                }
            }
            if !all {
                continue;
            }
            let (mut num, mut nsame) = (0usize, 0usize);
            for iy in -EXTRA_SIDE..EXTRA_SIDE + PATCH_SIDE as i64 {
                for ix in -EXTRA_SIDE..EXTRA_SIDE + PATCH_SIDE as i64 {
                    let cx = (bx * PATCH_SIDE) as i64 + ix;
                    let cy = (by * PATCH_SIDE) as i64 + iy;
                    if cx < 0 || cx >= w as i64 || cy < 0 || cy >= h as i64 {
                        continue;
                    }
                    num += 1;
                    if same(at(cx as usize, cy as usize), c0) {
                        nsame += 1;
                    }
                }
            }
            if nsame * 8 < num * 7 {
                continue;
            }
            shot[by * sw + bx] = 1;
            any = true;
        }
    }
    if !any {
        return Vec::new();
    }
    let is_shot = |x: usize, y: usize| shot[(y / PATCH_SIDE) * sw + x / PATCH_SIDE] != 0;
    // the background: flood fill from the screenshot-like areas with similar colours
    let n = w * h;
    let mut isbg = vec![0u8; n];
    let mut bg: [Vec<f32>; 3] = [vec![0.0; n], vec![0.0; n], vec![0.0; n]];
    let mut queue: Vec<(u32, u32)> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if is_shot(x, y) {
                let i = (y * w + x) as u32;
                queue.push((i, i));
            }
        }
    }
    let mut front = 0usize;
    while front < queue.len() {
        let (cur, src) = queue[front];
        front += 1;
        let (cur, src) = (cur as usize, src as usize);
        if isbg[cur] != 0 {
            continue;
        }
        isbg[cur] = 1;
        let (cx, cy) = (cur % w, cur / w);
        let (sx, sy) = (src % w, src / w);
        let spx = px(at(sx, sy));
        for c in 0..3 {
            bg[c][cur] = spx[c];
        }
        for dx in -SEARCH_RADIUS..=SEARCH_RADIUS {
            for dy in -SEARCH_RADIUS..=SEARCH_RADIUS {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let nx = cx as i64 + dx;
                let ny = cy as i64 + dy;
                if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                    continue;
                }
                if (nx - sx as i64).abs() + (ny - sy as i64).abs() > DISTANCE_LIMIT {
                    continue;
                }
                let (nx, ny) = (nx as usize, ny as usize);
                let ni = at(nx, ny);
                if sp.similar(spx, px(ni), SIMILAR) && (!is_shot(nx, ny) || same(at(sx, sy), ni)) {
                    let nl = ny * w + nx;
                    if isbg[nl] == 0 {
                        queue.push((nl as u32, src as u32));
                    }
                }
            }
        }
    }
    drop(queue);
    // small connected components on a uniform background
    let mut info: Vec<Found> = Vec::new();
    let mut visited = vec![0u8; n];
    let mut stack: Vec<u32> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let l = y * w + x;
            if isbg[l] != 0 || visited[l] != 0 {
                continue;
            }
            stack.clear();
            stack.push(l as u32);
            let (mut x0, mut x1, mut y0, mut y1) = (x, x, y, y);
            let mut reference = usize::MAX;
            let mut all_similar = true;
            while let Some(c) = stack.pop() {
                let c = c as usize;
                if visited[c] != 0 {
                    continue;
                }
                visited[c] = 1;
                let (cx, cy) = (c % w, c / w);
                x0 = x0.min(cx);
                x1 = x1.max(cx);
                y0 = y0.min(cy);
                y1 = y1.max(cy);
                for dx in -SEARCH_RADIUS..=SEARCH_RADIUS {
                    for dy in -SEARCH_RADIUS..=SEARCH_RADIUS {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let nx = cx as i64 + dx;
                        let ny = cy as i64 + dy;
                        if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                            continue;
                        }
                        let nl = ny as usize * w + nx as usize;
                        if isbg[nl] == 0 {
                            stack.push(nl as u32);
                        } else if reference == usize::MAX {
                            reference = nl;
                        } else {
                            let a = [bg[0][nl], bg[1][nl], bg[2][nl]];
                            let b = [bg[0][reference], bg[1][reference], bg[2][reference]];
                            if !sp.similar(a, b, VERY_SIMILAR) {
                                all_similar = false;
                            }
                        }
                    }
                }
            }
            if reference == usize::MAX || !all_similar || x1 - x0 >= MAX_PATCH || y1 - y0 >= MAX_PATCH {
                continue;
            }
            let rc = [bg[0][reference], bg[1][reference], bg[2][reference]];
            let mut has_similar = false;
            let ya = (y0 as i64 - HAS_SIMILAR_RADIUS).max(0) as usize;
            let xa = (x0 as i64 - HAS_SIMILAR_RADIUS).max(0) as usize;
            'h: for iy in ya..(y1 + HAS_SIMILAR_RADIUS as usize + 1).min(h) {
                for ix in xa..(x1 + HAS_SIMILAR_RADIUS as usize + 1).min(w) {
                    if sp.similar(rc, px(at(ix, iy)), HAS_SIMILAR) {
                        has_similar = true;
                        break 'h;
                    }
                }
            }
            if !has_similar {
                continue;
            }
            let pw = x1 - x0 + 1;
            let ph = y1 - y0 + 1;
            let mut f = [vec![0f32; pw * ph], vec![0f32; pw * ph], vec![0f32; pw * ph]];
            let mut q = [vec![0i8; pw * ph], vec![0i8; pw * ph], vec![0i8; pw * ph]];
            let mut maxv = 0i32;
            for c in [1usize, 0, 2] {
                for iy in y0..=y1 {
                    for ix in x0..=x1 {
                        let o = (iy - y0) * pw + ix - x0;
                        let v = p[c][at(ix, iy)] - rc[c];
                        f[c][o] = v;
                        let qv = sp.quantize(v, c);
                        q[c][o] = qv as i8;
                        maxv = maxv.max(qv.abs());
                    }
                }
            }
            if maxv < MIN_PEAK {
                continue;
            }
            info.push(Found { w: pw, h: ph, q, f, pos: vec![(x0 as u32, y0 as u32)] });
        }
    }
    if info.is_empty() {
        return info;
    }
    // identical patches (quantized) share one; keep those found twice or more (libjxl sorts
    // the (patch, positions) pairs: equal patches by their position)
    info.sort_by(|a, b| a.key_cmp(b).then_with(|| a.pos.cmp(&b.pos)));
    let mut out: Vec<Found> = Vec::new();
    for fd in info {
        if let Some(last) = out.last_mut()
            && last.key_cmp(&fd) == core::cmp::Ordering::Equal
        {
            last.pos.extend_from_slice(&fd.pos);
            continue;
        }
        if out.last().is_some_and(|l| l.pos.len() < MIN_OCCURRENCES) {
            out.pop();
        }
        out.push(fd);
    }
    if out.last().is_some_and(|l| l.pos.len() < MIN_OCCURRENCES) {
        out.pop();
    }
    if out.iter().map(|f| f.w * f.h).max().unwrap_or(0) < MIN_MAX_PATCH_PIXELS {
        return Vec::new();
    }
    out
}

// ------------------------------------------------------------------ libstdc++ std::sort

/// std::sort as libstdc++ implements it (introsort: median of three, unguarded partitions,
/// heap sort past 2 log2 n levels, insertion sort of runs up to 16), so elements that compare
/// equal end up where they do in libjxl
pub fn cxx_sort<T: Copy, F: FnMut(&T, &T) -> bool>(v: &mut [T], mut less: F) {
    let n = v.len();
    if n < 2 {
        return;
    }
    let depth = 2 * (usize::BITS - 1 - n.leading_zeros()) as usize;
    introsort_loop(v, 0, n, depth, &mut less);
    final_insertion_sort(v, 0, n, &mut less);
}

const S_THRESHOLD: usize = 16;

fn introsort_loop<T: Copy, F: FnMut(&T, &T) -> bool>(v: &mut [T], first: usize, mut last: usize, mut depth: usize, less: &mut F) {
    while last - first > S_THRESHOLD {
        if depth == 0 {
            heap_sort(v, first, last, less);
            return;
        }
        depth -= 1;
        let mid = first + (last - first) / 2;
        move_median_to_first(v, first, first + 1, mid, last - 1, less);
        let cut = unguarded_partition(v, first + 1, last, first, less);
        introsort_loop(v, cut, last, depth, less);
        last = cut;
    }
}

fn move_median_to_first<T: Copy, F: FnMut(&T, &T) -> bool>(v: &mut [T], r: usize, a: usize, b: usize, c: usize, less: &mut F) {
    let m = if less(&v[a], &v[b]) {
        if less(&v[b], &v[c]) {
            b
        } else if less(&v[a], &v[c]) {
            c
        } else {
            a
        }
    } else if less(&v[a], &v[c]) {
        a
    } else if less(&v[b], &v[c]) {
        c
    } else {
        b
    };
    v.swap(r, m);
}

fn unguarded_partition<T: Copy, F: FnMut(&T, &T) -> bool>(v: &mut [T], mut first: usize, mut last: usize, pivot: usize, less: &mut F) -> usize {
    loop {
        while less(&v[first], &v[pivot]) {
            first += 1;
        }
        last -= 1;
        while less(&v[pivot], &v[last]) {
            last -= 1;
        }
        if first >= last {
            return first;
        }
        v.swap(first, last);
        first += 1;
    }
}

fn final_insertion_sort<T: Copy, F: FnMut(&T, &T) -> bool>(v: &mut [T], first: usize, last: usize, less: &mut F) {
    if last - first > S_THRESHOLD {
        insertion_sort(v, first, first + S_THRESHOLD, less);
        for i in first + S_THRESHOLD..last {
            unguarded_linear_insert(v, i, less);
        }
    } else {
        insertion_sort(v, first, last, less);
    }
}

fn insertion_sort<T: Copy, F: FnMut(&T, &T) -> bool>(v: &mut [T], first: usize, last: usize, less: &mut F) {
    if first == last {
        return;
    }
    for i in first + 1..last {
        if less(&v[i], &v[first]) {
            let val = v[i];
            v.copy_within(first..i, first + 1);
            v[first] = val;
        } else {
            unguarded_linear_insert(v, i, less);
        }
    }
}

fn unguarded_linear_insert<T: Copy, F: FnMut(&T, &T) -> bool>(v: &mut [T], mut last: usize, less: &mut F) {
    let val = v[last];
    let mut next = last - 1;
    while less(&val, &v[next]) {
        v[last] = v[next];
        last = next;
        next -= 1;
    }
    v[last] = val;
}

fn heap_sort<T: Copy, F: FnMut(&T, &T) -> bool>(v: &mut [T], first: usize, last: usize, less: &mut F) {
    // __partial_sort(first, last, last) = make_heap + sort_heap
    let len = last - first;
    if len >= 2 {
        let mut parent = (len - 2) / 2;
        loop {
            let val = v[first + parent];
            adjust_heap(v, first, parent, len, val, less);
            if parent == 0 {
                break;
            }
            parent -= 1;
        }
    }
    let mut l = last;
    while l - first > 1 {
        l -= 1;
        let val = v[l];
        v[l] = v[first];
        adjust_heap(v, first, 0, l - first, val, less);
    }
}

fn adjust_heap<T: Copy, F: FnMut(&T, &T) -> bool>(v: &mut [T], first: usize, mut hole: usize, len: usize, val: T, less: &mut F) {
    let top = hole;
    let mut second = hole;
    while second < (len - 1) / 2 {
        second = 2 * (second + 1);
        if less(&v[first + second], &v[first + second - 1]) {
            second -= 1;
        }
        v[first + hole] = v[first + second];
        hole = second;
    }
    if len & 1 == 0 && second == (len - 2) / 2 {
        second = 2 * (second + 1);
        v[first + hole] = v[first + second - 1];
        hole = second - 1;
    }
    // __push_heap
    let mut parent = hole.wrapping_sub(1) / 2;
    while hole > top && less(&v[first + parent], &val) {
        v[first + hole] = v[first + parent];
        hole = parent;
        parent = hole.wrapping_sub(1) / 2;
    }
    v[first + hole] = val;
}

/// the packed patches: the reference frame (three planes, w x h, the patch pixels on zero)
/// and the dictionary
pub struct Packed {
    pub w: usize,
    pub h: usize,
    pub planes: [Vec<f32>; 3],
    pub dict: Dict,
}

/// FindBestPatchDictionary: largest patches first into the smallest square-ish frame found by
/// growing it 5 % at a time
pub fn pack(found: Vec<Found>, nec: u32) -> Packed {
    // largest first; equal areas in the order libstdc++'s std::sort leaves them (libjxl 0.7)
    let mut perm: Vec<u32> = (0..found.len() as u32).collect();
    cxx_sort(&mut perm, |a, b| found[*a as usize].w * found[*a as usize].h > found[*b as usize].w * found[*b as usize].h);
    let mut slots: Vec<Option<Found>> = found.into_iter().map(Some).collect();
    let found: Vec<Found> = perm.iter().map(|&i| slots[i as usize].take().unwrap_or_else(|| unreachable!())).collect();
    let (mut mx, mut my, mut total) = (0usize, 0usize, 0usize);
    for f in &found {
        mx = mx.max(f.w);
        my = my.max(f.h);
        total += f.w * f.h;
    }
    let st = rt::fx::sqrt(total as f32);
    let mut rw = (mx as f32).max(st) as usize;
    let mut rh = (my as f32).max(st) as usize;
    let mut placed: Vec<(usize, usize)> = vec![(0, 0); found.len()];
    let mut max_y;
    loop {
        max_y = 0usize;
        rw = (rw as f32 * SLACK + 1.0) as usize;
        rh = (rh as f32 * SLACK + 1.0) as usize;
        let mut occ = vec![0u8; rw * rh];
        let mut ok = true;
        for (k, f) in found.iter().enumerate() {
            let (mut x0, mut y0) = (0usize, 0usize);
            let mut hit = false;
            while y0 + f.h <= rh {
                x0 = 0;
                while x0 + f.w <= rw {
                    let mut occupied = false;
                    let mut x = x0;
                    for y in y0..y0 + f.h {
                        x = x0;
                        while x < x0 + f.w {
                            if occ[y * rw + x] != 0 {
                                occupied = true;
                                break;
                            }
                            x += 1;
                        }
                    }
                    if !occupied {
                        hit = true;
                        break;
                    }
                    x0 = x + 1; // past the occupied pixel (libjxl's jump)
                }
                if hit {
                    break;
                }
                y0 += 1;
            }
            if !hit {
                ok = false;
                break;
            }
            placed[k] = (x0, y0);
            for y in y0..y0 + f.h {
                for x in x0..x0 + f.w {
                    occ[y * rw + x] = 1;
                }
            }
            max_y = max_y.max(y0 + f.h);
        }
        if ok {
            break;
        }
    }
    let rh = max_y;
    let mut planes = [vec![0f32; rw * rh], vec![0f32; rw * rh], vec![0f32; rw * rh]];
    let mut dict = Dict { refs: Vec::with_capacity(found.len()), pos: Vec::with_capacity(found.len()), nec };
    for (f, &(x0, y0)) in found.iter().zip(&placed) {
        for y in 0..f.h {
            for x in 0..f.w {
                for c in 0..3 {
                    planes[c][(y0 + y) * rw + x0 + x] = f.f[c][y * f.w + x];
                }
            }
        }
        dict.refs.push(RefPos { rf: PATCH_REF, x0: x0 as u32, y0: y0 as u32, w: f.w as u32, h: f.h as u32 });
        dict.pos.push(f.pos.clone());
    }
    Packed { w: rw, h: rh, planes, dict }
}

impl Packed {
    /// SubtractFrom: planes (row stride s) minus the reference frame's values `r` (rw x rh,
    /// usually the decoded reference frame) at every position, in the dictionary's order
    pub fn subtract(&self, r: &[Vec<f32>; 3], planes: &mut [Vec<f32>; 3], s: usize) {
        for (rp, ps) in self.dict.refs.iter().zip(&self.dict.pos) {
            for &(x, y) in ps {
                for iy in 0..rp.h as usize {
                    for ix in 0..rp.w as usize {
                        let o = (y as usize + iy) * s + x as usize + ix;
                        let ro = (rp.y0 as usize + iy) * self.w + rp.x0 as usize + ix;
                        for c in 0..3 {
                            planes[c][o] -= r[c][ro];
                        }
                    }
                }
            }
        }
    }
}

/// a rectangle of a reference frame
#[derive(Clone, Copy, Default, Debug)]
pub struct RefPos {
    pub rf: u32,
    pub x0: u32,
    pub y0: u32,
    pub w: u32,
    pub h: u32,
}

/// the patch dictionary: the rectangles and, per rectangle, where it goes (in this order)
#[derive(Clone, Default)]
pub struct Dict {
    pub refs: Vec<RefPos>,
    pub pos: Vec<Vec<(u32, u32)>>,
    pub nec: u32,
}

impl Dict {
    /// PatchDictionaryEncoder::Encode
    pub fn write(&self, jw: &mut Jw) {
        let mut t: Vec<Token> = Vec::new();
        let n = self.refs.iter().zip(&self.pos).filter(|(_, p)| !p.is_empty()).count();
        t.push(Token::new(CTX_NREF, n as u32));
        for (r, ps) in self.refs.iter().zip(&self.pos) {
            if ps.is_empty() {
                continue;
            }
            t.push(Token::new(CTX_REF, r.rf));
            t.push(Token::new(CTX_REFPOS, r.x0));
            t.push(Token::new(CTX_REFPOS, r.y0));
            t.push(Token::new(CTX_SIZE, r.w - 1));
            t.push(Token::new(CTX_SIZE, r.h - 1));
            t.push(Token::new(CTX_COUNT, ps.len() as u32 - 1));
            for (j, &(x, y)) in ps.iter().enumerate() {
                if j == 0 {
                    t.push(Token::new(CTX_POS, x));
                    t.push(Token::new(CTX_POS, y));
                } else {
                    let (px, py) = ps[j - 1];
                    t.push(Token::new(CTX_OFFSET, pack_signed(x as i32 - px as i32)));
                    t.push(Token::new(CTX_OFFSET, pack_signed(y as i32 - py as i32)));
                }
                t.push(Token::new(CTX_BLEND, PB_ADD));
                for _ in 0..self.nec {
                    t.push(Token::new(CTX_BLEND, PB_NONE));
                }
            }
        }
        let mut st = vec![t];
        let code = eans::build(&HParams::default(), NUM_CTX, &mut st, Some(jw));
        eans::write_tokens(&st[0], &code, jw);
    }

    /// the number of positions
    pub fn count(&self) -> usize {
        self.pos.iter().map(|p| p.len()).sum()
    }
}
