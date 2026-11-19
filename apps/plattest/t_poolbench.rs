// t_poolbench -- plattest --test pool-bench (not in "all": timings): how parallel_for scales with the
// number of threads that take part, 1 (this thread alone) up to cpu_count + 1.  On big.LITTLE phones
// that is the question how many workers the pool should start (docs/ANDROID.md AN1): fast and slow cores
// share the items, and the loop ends with the slowest one.
//
//   arith   ITEMS items of xorshift arithmetic (the cores' speed)
//   memory  ITEMS items that write and read 256 KB each (the caches and the memory bus)
//
// Every count runs REPEAT times, the best one counts; every count must give the same checksum.

use alloc::format;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering::Relaxed};

use plat::sync::Mutex;

use crate::{T, out};

const ITEMS: usize = 512;
const ROUNDS: u32 = 400_000; // xorshift steps per arithmetic item
const WORDS: usize = 32 * 1024; // 256 KB per memory item
const REPEAT: u32 = 3;

fn arith(i: usize) -> u64 {
    let mut x = i as u64 | 1;
    for _ in 0..ROUNDS {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
    }
    x
}

fn memory(i: usize, buf: &mut [u64]) -> u64 {
    let mut s = 0u64;
    for pass in 0..2u64 {
        for (j, w) in buf.iter_mut().enumerate() {
            *w = (j as u64 ^ i as u64).wrapping_mul(pass + 3);
        }
        s = s.wrapping_add(buf.iter().fold(0u64, |a, &w| a.rotate_left(1) ^ w));
    }
    s
}

// one run of a kind with `threads` taking part -> (ms, checksum)
fn run(kind: u32, threads: usize, bufs: &[Mutex<Vec<u64>>]) -> (u64, u64) {
    let sum = AtomicU64::new(0);
    let item = |i: usize| {
        let v = if kind == 0 {
            arith(i)
        } else {
            let mut b = bufs[plat::thr_self().min(bufs.len() - 1)].lock();
            memory(i, &mut b)
        };
        sum.fetch_add(v, Relaxed);
    };
    let t0 = plat::time_us();
    if threads == 1 {
        (0..ITEMS).for_each(item);
    } else {
        let _ = plat::parallel_for(ITEMS, &|i, _| item(i));
    }
    ((plat::time_us() - t0) / 1000, sum.load(Relaxed))
}

pub fn t_poolbench(t: &mut T) {
    let cpus = plat::cpu_count() as usize;
    out(&format!("  pool-bench: {} CPUs, {}; {ITEMS} items, best of {REPEAT}\n", cpus, plat::cpu_brand()));
    let bufs: Vec<Mutex<Vec<u64>>> = (0..=plat::THR_MAX).map(|_| Mutex::new(alloc::vec![0u64; WORDS])).collect();
    let mut same = true;
    for (kind, name) in [(0u32, "arith"), (1, "memory")] {
        let mut first = (0u64, 0u64);
        for threads in 1..=cpus + 1 {
            plat::thr_shutdown();
            if threads > 1 && plat::thr_init(threads as u32 - 1) != threads as u32 - 1 {
                out(&format!("  pool-bench: {name}: the pool did not start {} workers\n", threads - 1));
                same = false;
                break;
            }
            let mut best = u64::MAX;
            let mut check = 0;
            for _ in 0..REPEAT {
                let (ms, c) = run(kind, threads, &bufs);
                best = best.min(ms);
                check = c;
            }
            if threads == 1 {
                first = (best, check);
            }
            same &= check == first.1;
            let x100 = first.0 * 100 / best.max(1); // the speed-up against one thread, in hundredths
            out(&format!("  pool-bench: {name:<6} {threads:>2} threads  {best:>6} ms  {}.{:02}x\n", x100 / 100, x100 % 100));
        }
    }
    plat::thr_shutdown();
    t.check(same, "pool-bench: every number of threads gives the same result");
}
