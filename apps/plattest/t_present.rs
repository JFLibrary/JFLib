// t_present -- plattest --test present-bench [--frames N] [--size WxH]: what presenting a frame
// costs in a real window (not in "all": timings).  The frame is W x H (default 3840 x 2160, a web
// page at 4K) and larger than the window (640 x 400 DIP): the window system clips what it shows,
// plat copies all of the dirty rectangles into the window's own pixels (Windows: the kept DIB
// section; X11: an MIT-SHM segment; Wayland: a wl_shm buffer).  Two frames by turns, as an engine
// hands them over (neither stays in the caches); between presents the loop pumps for 16 ms, so
// the segments and buffers are free again.  Variants: the whole frame (rects None), and damage
// only -- a caret, a line of text, 20 blocks, a band.  Per variant: win_present's own time per
// call (best, median) and present_time per frame (with what the platform defers: X11 a segment
// that frees up, Wayland the frame callback).  Windows also checks a pixel the window shows.
// First, without the window, the copy alone: the plain row copies of before charge 9 against
// plat's present copy (plat::present_copy_bench: row bands on the pool, streaming at 4K), the
// same bytes checked.
// The window is shown without activation and closes itself; no input is sent.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use base::Args;
use plat::*;

use crate::T;

/// pump the OS until the deadline (plat::time_us)
fn pump_until(dl: u64) {
    loop {
        wait(dl);
        while evq_pop().is_some() {}
        if time_us() >= dl {
            return;
        }
    }
}

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
}

pub fn t_present_bench(t: &mut T) {
    let args = Args(args());
    let n = args.int("--frames", 40).clamp(3, 1000) as usize;
    let (fw, fh) = args
        .value("--size")
        .and_then(|s| s.split_once('x'))
        .and_then(|(a, b)| Some((a.parse::<i32>().ok()?, b.parse::<i32>().ok()?)))
        .unwrap_or((3840, 2160));
    let (fw, fh) = (fw.clamp(16, 8192), fh.clamp(16, 8192));
    setenv("jfgui_TEST_NOACTIVATE", Some("1"));
    evq_clear();
    let w = win_create("plattest present-bench", 640, 400, PWF_RESIZABLE, PW_NOOWNER, PW_DEFAULT, PW_DEFAULT);
    t.check(w == 0, "present-bench: win_create");
    if w != 0 {
        return;
    }
    #[cfg(windows)]
    win_test(WT_NOANIM, 0, [0; 4]);
    win_show(0, PWS_SHOWNA);
    pump_until(time_us() + 300_000);
    // two frames of noise (the bytes do not matter, the caches do)
    let stride = fw as usize * 4;
    let mut frames: [Vec<u8>; 2] = [vec![0u8; stride * fh as usize], vec![0u8; stride * fh as usize]];
    let mut rng = Rng(0x2545_F491);
    for f in frames.iter_mut() {
        for c in f.chunks_exact_mut(4) {
            c.copy_from_slice(&rng.next().to_le_bytes());
        }
    }
    let same = copy_alone(&frames, fw, fh, n);
    t.check(same, "present-bench: plat's present copy gives the bytes of the row copies");
    let frame = |k: usize| Frame { px: &frames[k], w: fw, h: fh, stride: stride as i32 };
    out_line(&format!("present-bench: frame {fw} x {fh} ({} KB) in a window of {} x {} px, {} worker threads, {n} frames per variant\n",
                      stride * fh as usize / 1024, win_size(0).0, win_size(0).1, thr_count()));
    // warm up: the window's buffers at the frame's size
    for k in 0..3 {
        win_present(0, &frame(k & 1), None);
        pump_until(time_us() + 30_000);
    }
    let variants: [(&str, i32, i32, i32); 5] =
        [("whole frame", fw, fh, 0), ("caret 2x20", 2, 20, 1), ("text line 1200x80", 1200, 80, 1), ("20 blocks 256x64", 256, 64, 20), ("band 3840x400", fw, 400, 1)];
    for (name, rw, rh, count) in variants {
        let (rw, rh) = (rw.min(fw), rh.min(fh));
        let mut times: Vec<u64> = Vec::with_capacity(n);
        let mut rects: Vec<[i32; 4]> = Vec::new();
        let (n0, us0) = present_time();
        for i in 0..n {
            rects.clear();
            for j in 0..count {
                let (x, y) = ((j * 173 + i as i32 * 7) % (fw - rw).max(1), (j * 97 + i as i32 * 3) % (fh - rh).max(1));
                rects.push([x, y, rw, rh]);
            }
            let f = frame(i & 1);
            let t0 = time_us();
            win_present(0, &f, if count == 0 { None } else { Some(&rects) });
            times.push(time_us() - t0);
            pump_until(time_us() + 16_000);
        }
        pump_until(time_us() + 100_000); // what waits for a free segment / the frame callback
        let (n1, us1) = present_time();
        times.sort_unstable();
        let kb = if count == 0 { stride * fh as usize / 1024 } else { (rw * rh * count * 4) as usize / 1024 };
        out_line(&format!("present-bench: {name} ({kb} KB): win_present best {} us, median {} us; present_time {} us per frame\n",
                          times[0], times[n / 2], (us1 - us0) / (n1 - n0).max(1)));
    }
    // the window shows the frame (Windows: a pixel of its DC after a whole present)
    #[cfg(windows)]
    {
        win_present(0, &frame(0), None);
        pump_until(time_us() + 50_000);
        let px = |x: usize, y: usize| {
            let o = y * stride + x * 4;
            u32::from_le_bytes([frames[0][o], frames[0][o + 1], frames[0][o + 2], 0]) as i64
        };
        let ok = [(5usize, 5usize), (100, 37), (300, 200)].iter().all(|&(x, y)| win_test(WT_PIXEL, 0, [x as i32, y as i32, 0, 0]) == px(x, y));
        t.check(ok, "present-bench: the window shows the presented frame");
    }
    win_destroy(0);
    pump_until(time_us() + 50_000);
    setenv("jfgui_TEST_NOACTIVATE", None);
    t.check(true, "present-bench: done");
}

fn out_line(s: &str) {
    write_stdout(s.as_bytes());
}

// the damage of the variants: (name, w, h, count; 0 = the whole frame)
fn boxes_of(fw: i32, fh: i32, rw: i32, rh: i32, count: i32, i: usize, out: &mut Vec<[i32; 4]>) {
    out.clear();
    if count == 0 {
        out.push([0, 0, fw, fh]);
    }
    for j in 0..count {
        let (x, y) = ((j * 173 + i as i32 * 7) % (fw - rw).max(1), (j * 97 + i as i32 * 3) % (fh - rh).max(1));
        out.push([x, y, x + rw, y + rh]);
    }
}

/// the present's copy alone, without a window (best / median of n): the plain row copies of
/// before charge 9 against plat's present copy (row bands on the pool, streaming at 4K), the
/// same bytes checked
fn copy_alone(frames: &[Vec<u8>; 2], fw: i32, fh: i32, n: usize) -> bool {
    #[cfg(any(windows, target_os = "linux"))]
    {
        let mut all = true;
        let stride = fw as usize * 4;
        let mut a = vec![1u8; stride * fh as usize];
        let mut b = vec![2u8; stride * fh as usize];
        let mut bx: Vec<[i32; 4]> = Vec::new();
        for (name, rw, rh, count) in [("whole frame", fw, fh, 0), ("text line 1200x80", 1200, 80, 1), ("20 blocks 256x64", 256, 64, 20), ("band 3840x400", fw, 400, 1)] {
            let (rw, rh) = (rw.min(fw), rh.min(fh));
            let (mut t_rows, mut t_plat) = (Vec::with_capacity(n), Vec::with_capacity(n));
            for i in 0..n {
                boxes_of(fw, fh, rw, rh, count, i, &mut bx);
                let src = &frames[i & 1];
                let t0 = time_us();
                for q in &bx {
                    let (o, len) = (q[0] as usize * 4, (q[2] - q[0]) as usize * 4);
                    for y in q[1] as usize..q[3] as usize {
                        a[y * stride + o..y * stride + o + len].copy_from_slice(&src[y * stride + o..y * stride + o + len]);
                    }
                }
                t_rows.push(time_us() - t0);
                let t0 = time_us();
                present_copy_bench(&mut b, stride, src, stride, &bx, fw, fh);
                t_plat.push(time_us() - t0);
            }
            t_rows.sort_unstable();
            t_plat.sort_unstable();
            let same = a == b;
            all &= same;
            out_line(&format!("present-bench: copy alone, {name}: row copies best {} us, median {} us; plat's present copy best {} us, median {} us{}\n",
                              t_rows[0], t_rows[n / 2], t_plat[0], t_plat[n / 2], if same { "" } else { " -- THE BYTES DIFFER" }));
        }
        all
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (frames, fw, fh, n);
        true
    }
}
