// present_copy -- the copy of a present's dirty rectangles into the window's own pixels:
// Windows' kept DIB section (win_api.rs), X11's MIT-SHM segments, shadow and staging images,
// Wayland's wl_shm buffers and staging image (linux_damage.rs)
//
//   copy_boxes(dst, ds, src, ss, boxes, w, h)   the boxes x0, y0, x1, y1 (exclusive; clipped to
//                     w x h, they may overlap) of src (rows of ss bytes) into dst (rows of ds
//                     bytes), 4 bytes a pixel; rows outside either slice are skipped.  Below
//                     PAR_MIN bytes plain row copies on this thread (no pool round trip); from
//                     PAR_MIN on in row bands on the worker pool (parallel_for; a band is a run of
//                     the target's rows, so no two threads write one row, overlapping boxes
//                     included); from STREAM_MIN on with streaming stores (x86-64: movntdq), fenced
//                     on every thread before the loop returns.  The same bytes either way.
//
// The thresholds are those of gfx's blit (gfx::COPY_PAR_MIN / COPY_STREAM_MIN, measured there): a
// 4K frame is 32 MB, source and target together no longer fit the caches, and the target is read
// by someone else next (GDI's BitBlt, the X server, the compositor).  plat has its own streaming
// copy (stream_copy, simd::copy_stream's loop): plat comes before simd in the build order (the
// workspace's members, which build_rs.py's LIBS and the publication follow), so depending on
// simd would move simd below plat and give the published jflib-plat a new dependency, with
// simd's kernels and its compute-crate build, for one loop of 30 lines.

use alloc::vec::Vec;

use rt::sync::SpinLock;

/// a present's copy from this many bytes on runs in row bands on the worker pool
pub(crate) const PAR_MIN: usize = 1 << 20;
/// ... and from this many on with streaming stores
pub(crate) const STREAM_MIN: usize = 12 << 20;
// bytes of a band (the pool hands the bands out as threads get free)
const BAND: usize = 256 * 1024;

// a box clipped to w x h (None: empty)
fn clipped(b: &[i32; 4], w: i32, h: i32) -> Option<[i32; 4]> {
    let c = [b[0].max(0), b[1].max(0), b[2].min(w), b[3].min(h)];
    (c[0] < c[2] && c[1] < c[3]).then_some(c)
}

/// copy the pixel rows of the boxes (x0, y0, x1, y1; clipped to w x h) from src (rows of ss bytes)
/// into dst (rows of ds bytes); rows outside either slice are skipped.  Large copies run in row
/// bands on the worker pool, from STREAM_MIN bytes on past the caches (see the head of the file)
pub(crate) fn copy_boxes(dst: &mut [u8], ds: usize, src: &[u8], ss: usize, boxes: &[[i32; 4]], w: i32, h: i32) {
    if ds == 0 {
        return;
    }
    let (mut total, mut ylo, mut yhi) = (0usize, i32::MAX, i32::MIN);
    for b in boxes.iter().filter_map(|b| clipped(b, w, h)) {
        total = total.saturating_add((b[2] - b[0]) as usize * 4 * (b[3] - b[1]) as usize);
        ylo = ylo.min(b[1]);
        yhi = yhi.max(b[3]);
    }
    if total == 0 {
        return;
    }
    let stream = total >= STREAM_MIN;
    let rows = (yhi - ylo) as usize;
    if total < PAR_MIN || rows < 2 || crate::thr_count() == 0 {
        let mut nt = false;
        for b in boxes.iter().filter_map(|b| clipped(b, w, h)) {
            nt |= copy_rows(dst, ds, 0, src, ss, &b, b[1], b[3], stream);
        }
        if nt {
            stream_fence();
        }
        return;
    }
    // bands of bh rows from row ylo on: chunks of the target (each band locks its own)
    let bh = rows.div_ceil((total / BAND).clamp(2, rows));
    let Some(tail) = dst.get_mut(ylo as usize * ds..) else { return };
    let chunks: Vec<SpinLock<&mut [u8]>> = tail.chunks_mut(bh * ds).take(rows.div_ceil(bh)).map(SpinLock::new).collect();
    let _ = crate::parallel_for(chunks.len(), &|i, _| {
        let Some(cell) = chunks.get(i) else { return };
        let mut c = cell.lock();
        let y0 = ylo + (i * bh) as i32;
        let y1 = (y0 + bh as i32).min(yhi);
        let mut nt = false;
        for b in boxes.iter().filter_map(|b| clipped(b, w, h)) {
            let (a, z) = (b[1].max(y0), b[3].min(y1));
            if a < z {
                nt |= copy_rows(&mut c[..], ds, y0, src, ss, &b, a, z, stream);
            }
        }
        if nt {
            stream_fence(); // (this thread's stores, before the loop counts the band done)
        }
    });
}

/// rows a..z of box b: src's row y at y * ss, dst's at (y - base) * ds (dst starts at row base)
/// -> streaming stores were used (the caller fences them)
#[allow(clippy::too_many_arguments)]
fn copy_rows(dst: &mut [u8], ds: usize, base: i32, src: &[u8], ss: usize, b: &[i32; 4], a: i32, z: i32, stream: bool) -> bool {
    let (o, n) = (b[0] as usize * 4, (b[2] - b[0]) as usize * 4);
    let mut nt = false;
    for y in a..z {
        let (s, d) = (y as usize * ss + o, (y - base) as usize * ds + o);
        if let (Some(sr), Some(dr)) = (src.get(s..s + n), dst.get_mut(d..d + n)) {
            if stream {
                // SAFETY: copy_boxes fences this thread's stores before anything reads dst.
                nt |= unsafe { stream_copy(dr, sr) };
            } else {
                dr.copy_from_slice(sr);
            }
        }
    }
    nt
}

/// copy src into dst (equal lengths) with non-temporal stores (the 16-byte aligned part of dst)
/// -> whether such stores were used (simd::copy_stream's loop without its fence)
///
/// # Safety
/// When it returns true, stream_fence() must run on this thread before anything (this thread
/// included) reads or writes dst again: Rust's memory model does not cover the stores before it.
#[cfg(target_arch = "x86_64")]
unsafe fn stream_copy(dst: &mut [u8], src: &[u8]) -> bool {
    use core::arch::x86_64::{__m128i, _mm_loadu_si128, _mm_stream_si128};
    let n = dst.len().min(src.len());
    let (dst, src) = (&mut dst[..n], &src[..n]);
    let head = (dst.as_ptr() as usize).wrapping_neg() & 15;
    if n < head + 64 {
        dst.copy_from_slice(src);
        return false;
    }
    dst[..head].copy_from_slice(&src[..head]);
    let body = (n - head) & !63;
    let (d, s) = (&mut dst[head..head + body], &src[head..head + body]);
    let mut i = 0;
    while i < body {
        // SAFETY: i + 64 <= body: 64 readable bytes of s and 64 writable bytes of d at i; d's
        // start is 16-byte aligned (head), so every store address is (movntdq's alignment).
        unsafe {
            let (sp, dp) = (s.as_ptr().add(i) as *const __m128i, d.as_mut_ptr().add(i) as *mut __m128i);
            let (a, b, c, e) = (_mm_loadu_si128(sp), _mm_loadu_si128(sp.add(1)), _mm_loadu_si128(sp.add(2)), _mm_loadu_si128(sp.add(3)));
            _mm_stream_si128(dp, a);
            _mm_stream_si128(dp.add(1), b);
            _mm_stream_si128(dp.add(2), c);
            _mm_stream_si128(dp.add(3), e);
        }
        i += 64;
    }
    dst[head + body..].copy_from_slice(&src[head + body..]);
    true
}

/// stream_copy without streaming stores (ARM64: a copy of whole lines in a row is noticed by the
/// cores, which then stop allocating in their caches; see simd::copy_stream)
///
/// # Safety
/// None here (it is unsafe for the x86-64 version's sake).
#[cfg(not(target_arch = "x86_64"))]
unsafe fn stream_copy(dst: &mut [u8], src: &[u8]) -> bool {
    let n = dst.len().min(src.len());
    dst[..n].copy_from_slice(&src[..n]);
    false
}

/// the streaming stores of this thread become visible to others (sfence)
#[inline]
fn stream_fence() {
    // SAFETY: sfence is part of the x86-64 baseline (SSE); it only orders stores.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::x86_64::_mm_sfence()
    };
}

/// (benchmarks: plattest --test present-bench) copy_boxes for a bench outside plat: the boxes
/// (x0, y0, x1, y1) of src into dst as a present copies them
#[doc(hidden)]
pub fn present_copy_bench(dst: &mut [u8], ds: usize, src: &[u8], ss: usize, boxes: &[[i32; 4]], w: i32, h: i32) {
    copy_boxes(dst, ds, src, ss, boxes, w, h);
}
