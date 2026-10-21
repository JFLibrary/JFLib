// par -- the worker-pool helpers of the JPEG XL reader and writer (jxl-plus / jxl-speed; moved
// out of the writer's emod.rs by jxl-module: the decoder uses them too)    [jxl-plus: no asm]
//
//   par_map(n, threads, f)              f(0..n) on the worker pool, results in index order
//   par_map_with(n, threads, init, f)   par_map with a scratch value per worker
//   par_rows(planes, h, band, threads, f)   bands of rows of planes, each band on one worker
//   join(threads, a, b)                 two parts at the same time
//
// Work depends only on the index, so results never depend on the number of threads.

use alloc::vec::Vec;

use rt::sync::SpinLock;

/// run f(i) for i in 0..n on the pool (or here with one thread); results in order
pub fn par_map<T: Send>(n: usize, threads: bool, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    if !threads || n <= 1 {
        return (0..n).map(f).collect();
    }
    let outs: Vec<SpinLock<Option<T>>> = (0..n).map(|_| SpinLock::new(None)).collect();
    let item = |i: usize, _job: &plat::JobCtl| {
        let r = f(i);
        *outs[i].lock() = Some(r);
    };
    let _ = plat::parallel_for(n, &item);
    outs.into_iter().map(|o| o.lock().take().expect("parallel item")).collect()
}

/// a() and b() at the same time on the pool (one after the other without threads)  [jxl-speed]
pub fn join<A: Send, B: Send>(threads: bool, a: impl FnOnce() -> A + Send, b: impl FnOnce() -> B + Send) -> (A, B) {
    if !threads {
        let x = a();
        let y = b();
        return (x, y);
    }
    let fa = SpinLock::new(Some(a));
    let fb = SpinLock::new(Some(b));
    let ra: SpinLock<Option<A>> = SpinLock::new(None);
    let rb: SpinLock<Option<B>> = SpinLock::new(None);
    par_map(2, true, |i| {
        if i == 0 {
            let f = fa.lock().take();
            if let Some(f) = f {
                let v = f();
                *ra.lock() = Some(v);
            }
        } else {
            let f = fb.lock().take();
            if let Some(f) = f {
                let v = f();
                *rb.lock() = Some(v);
            }
        }
    });
    let x = ra.lock().take().expect("join a");
    let y = rb.lock().take().expect("join b");
    (x, y)
}

/// par_map with scratch: f(scratch, i); the scratch values are made by init when no idle one
/// is left and handed on to later items, so items do not allocate their large buffers again
/// (blocks above 2 KB are page runs from the OS: allocating them per item costs system calls
/// and page faults that do not scale over threads).  f must not depend on what an earlier
/// item left in the scratch.  [jxl-speed]
pub fn par_map_with<S: Send, T: Send>(n: usize, threads: bool, init: impl Fn() -> S + Sync, f: impl Fn(&mut S, usize) -> T + Sync) -> Vec<T> {
    if !threads || n <= 1 {
        let mut s = init();
        return (0..n).map(|i| f(&mut s, i)).collect();
    }
    let idle: SpinLock<Vec<S>> = SpinLock::new(Vec::new());
    par_map(n, threads, |i| {
        let got = idle.lock().pop();
        let mut s = match got {
            Some(s) => s,
            None => init(),
        };
        let r = f(&mut s, i);
        idle.lock().push(s);
        r
    })
}

/// f(y0, rows, the planes' rows y0 .. y0 + rows) for bands of `band` rows, on the worker pool
/// when threads (each band writes only its own rows)  [jxl-speed]
pub fn par_rows<T: Send>(planes: Vec<(&mut [T], usize)>, h: usize, band: usize, threads: bool, f: impl Fn(usize, usize, &mut [&mut [T]]) + Sync) {
    let nb = h.div_ceil(band);
    if nb == 0 {
        return;
    }
    let k = planes.len();
    let mut per: Vec<Vec<&mut [T]>> = (0..nb).map(|_| Vec::with_capacity(k)).collect();
    for (d, s) in planes {
        let total = (h * s).min(d.len());
        for (b, ch) in d[..total].chunks_mut((band * s).max(1)).enumerate().take(nb) {
            per[b].push(ch);
        }
    }
    let cells: Vec<SpinLock<Vec<&mut [T]>>> = per.into_iter().map(SpinLock::new).collect();
    par_map(nb, threads, |b| {
        let mut g = cells[b].lock();
        if g.len() == k {
            f(b * band, band.min(h - b * band), &mut g[..]);
        }
    });
}
