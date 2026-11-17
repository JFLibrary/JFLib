// t_atomics -- (ARM7, docs/ARM64.md) the memory order on weakly ordered CPUs: the library's
// primitives under many threads, and litmus tests that show whether this CPU reorders at all
//
//   plattest --test atomics                  one short pass (the suite: a few seconds)
//   plattest --test atomics --secs N         passes until N seconds are over (devices, runners)
//   --threads N                              contending threads (default: the CPUs, 2 .. 16);
//                                            the pool takes the same number of workers
//   --scale N                                N times the work of one pass
//
// Checks (any error fails): rt::sync::SpinLock, plat::sync::{Mutex, RwLock, Condvar, channel},
// plat::Sem, the pool (parallel_for, nested loops, a Job polled with done()), the heap across
// threads (every size class, page runs, over-aligned blocks), the event ring, the audio ring;
// and the patterns the library relies on: store buffering with SeqCst, the pool's wait protocol
// (fetch_add + load against swap + swap), message passing with Release / Acquire, a seqlock
// with fences.  Plain data is handed over in Relaxed atomics: only the primitive under test
// orders it.  Controls (counted, never a failure): store buffering and message passing with
// Relaxed, a seqlock without fences -- x86-64 shows store buffering only, an ARM CPU may show
// all of them.  The first line names the way the atomics go: LSE or LL/SC (with
// jfgui_ARM_LLSC=1 a static Linux ARM64 program takes LL/SC on a CPU with LSE; rt::outline_lse).
// A lost wake-up shows as a wait of WAIT_MS that runs out, or as a hang.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering::{Acquire, Relaxed, Release, SeqCst}, fence};

use plat::sync::{Condvar, Mutex, RwLock, channel};
use rt::sync::SpinLock;

use crate::{T, out};

const WAIT_MS: u32 = 10_000; // a wait this long without progress counts as lost
const WORDS: usize = 32; // the data under a lock: four cache lines
const MAGIC: u64 = 0x5A5A_C3C3_0F0F_9696;
const K: u64 = 0x9E37_79B9_7F4A_7C15;

struct Cfg {
    threads: usize,
    scale: u64,
    pass: u64,
}

#[derive(Default, Clone, Copy)]
struct Tally {
    ops: u64,
    bad: u64,
    us: u64,
}

/// f's tally with the time it took
fn timed(f: impl FnOnce() -> Tally) -> Tally {
    let t0 = plat::time_us();
    let mut t = f();
    t.us = plat::time_us() - t0;
    t
}

/// one word of a message or block, from who made it, which one and where
fn word(id: u64, seq: u64, i: u64) -> u64 {
    (id << 56) ^ seq.wrapping_mul(K) ^ i.wrapping_mul(0xD6E8_FEB8_6659_FD93)
}

/// f(k) for k in 0..n on n new threads that start together; joined -> threads that ran
fn run(n: usize, f: Arc<dyn Fn(usize) + Send + Sync>) -> usize {
    let gate = Arc::new(AtomicUsize::new(0));
    let mut hs = Vec::new();
    for k in 0..n {
        let (f, gate) = (f.clone(), gate.clone());
        let h = plat::thread_spawn(256 << 10, Box::new(move || {
            gate.fetch_add(1, SeqCst);
            let mut spins = 0u32;
            while gate.load(SeqCst) < n {
                spins += 1;
                if spins & 0xFFF == 0 { plat::thread_yield() } else { core::hint::spin_loop() }
            }
            f(k);
        }));
        if let Some(h) = h {
            hs.push(h);
        }
    }
    if hs.len() < n {
        gate.fetch_add(n - hs.len(), SeqCst); // a thread that did not start: let the others go
    }
    let ran = hs.len();
    for h in hs {
        h.join();
    }
    ran
}

/// a critical section: every word holds the same count (else 1: torn), then one more
fn bump(w: &mut [u64; WORDS]) -> u64 {
    let v = w[0];
    let torn = w.iter().any(|&x| x != v) as u64;
    for x in w.iter_mut() {
        *x = v + 1;
    }
    torn
}

// ---------------------------------------------------------------- locks

fn g_spin(c: &Cfg) -> Tally {
    let iters = 20_000 * c.scale;
    let lock = Arc::new(SpinLock::new([0u64; WORDS]));
    let bad = Arc::new(AtomicU64::new(0));
    let (l, b) = (lock.clone(), bad.clone());
    let ran = run(c.threads, Arc::new(move |_| {
        let mut torn = 0;
        for _ in 0..iters {
            torn += bump(&mut l.lock());
        }
        b.fetch_add(torn, SeqCst);
    }));
    let total = ran as u64 * iters;
    let end_ok = lock.lock().iter().all(|&x| x == total);
    Tally { ops: total, bad: bad.load(SeqCst) + !end_ok as u64 + (ran != c.threads) as u64, us: 0 }
}

fn g_mutex(c: &Cfg) -> Tally {
    let iters = 20_000 * c.scale;
    let lock = Arc::new(Mutex::new([0u64; WORDS]));
    let bad = Arc::new(AtomicU64::new(0));
    let (l, b) = (lock.clone(), bad.clone());
    let ran = run(c.threads, Arc::new(move |_| {
        let mut torn = 0;
        for i in 0..iters {
            if i % 8 == 0 {
                let mut g = loop {
                    if let Some(g) = l.try_lock() {
                        break g;
                    }
                    core::hint::spin_loop();
                };
                torn += bump(&mut g);
            } else {
                torn += bump(&mut l.lock());
            }
        }
        b.fetch_add(torn, SeqCst);
    }));
    let total = ran as u64 * iters;
    let end_ok = lock.lock().iter().all(|&x| x == total);
    Tally { ops: total, bad: bad.load(SeqCst) + !end_ok as u64 + (ran != c.threads) as u64, us: 0 }
}

fn g_rwlock(c: &Cfg) -> Tally {
    let writers = 2usize.min(c.threads - 1);
    let (witers, riters) = (2_000 * c.scale, 8_000 * c.scale);
    let lock = Arc::new(RwLock::new([0u64; WORDS]));
    let bad = Arc::new(AtomicU64::new(0));
    let (l, b) = (lock.clone(), bad.clone());
    let ran = run(c.threads, Arc::new(move |k| {
        let mut torn = 0;
        if k < writers {
            for _ in 0..witers {
                torn += bump(&mut l.write());
            }
        } else {
            for _ in 0..riters {
                let g = l.read();
                let v = g[0];
                torn += g.iter().any(|&x| x != v) as u64;
            }
        }
        b.fetch_add(torn, SeqCst);
    }));
    let end_ok = lock.read().iter().all(|&x| x == writers as u64 * witers);
    let ops = writers as u64 * witers + (c.threads - writers) as u64 * riters;
    Tally { ops, bad: bad.load(SeqCst) + !end_ok as u64 + (ran != c.threads) as u64, us: 0 }
}

struct Queue {
    buf: VecDeque<[u64; 4]>,
    producers: usize,
}

/// producers and consumers on a queue of 8 under a Mutex, two Condvars between them
fn g_condvar(c: &Cfg) -> Tally {
    let p = (c.threads / 2).max(1);
    let q = (c.threads - p).max(1);
    let iters = 5_000 * c.scale;
    let st = Arc::new((Mutex::new(Queue { buf: VecDeque::new(), producers: p }), Condvar::new(), Condvar::new()));
    let (got, bad) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let (s, g2, b2) = (st.clone(), got.clone(), bad.clone());
    let ran = run(p + q, Arc::new(move |k| {
        let (m, not_empty, not_full) = &*s;
        let mut wrong = 0;
        if k < p {
            for i in 0..iters {
                let mut g = m.lock();
                while g.buf.len() >= 8 {
                    let (h, out) = not_full.wait_timeout(g, Some(WAIT_MS));
                    g = h;
                    wrong += (out && g.buf.len() >= 8) as u64;
                }
                g.buf.push_back([k as u64, i, word(k as u64, i, 0), !i]);
                drop(g);
                not_empty.notify_one();
            }
            m.lock().producers -= 1;
            not_empty.notify_all();
        } else {
            let mut last = vec![u64::MAX; p];
            let mut n = 0;
            let mut g = m.lock();
            loop {
                if let Some(e) = g.buf.pop_front() {
                    drop(g);
                    not_full.notify_one();
                    let id = e[0] as usize;
                    let ok = id < p && e[2] == word(e[0], e[1], 0) && e[3] == !e[1] && (last[id] == u64::MAX || e[1] > last[id]);
                    wrong += !ok as u64;
                    if id < p {
                        last[id] = e[1];
                    }
                    n += 1;
                    g = m.lock();
                } else if g.producers == 0 {
                    break;
                } else {
                    let (h, out) = not_empty.wait_timeout(g, Some(WAIT_MS));
                    g = h;
                    wrong += (out && g.buf.is_empty() && g.producers > 0) as u64;
                }
            }
            g2.fetch_add(n, SeqCst);
        }
        b2.fetch_add(wrong, SeqCst);
    }));
    let all = got.load(SeqCst) == p as u64 * iters;
    Tally { ops: p as u64 * iters, bad: bad.load(SeqCst) + !all as u64 + (ran != p + q) as u64, us: 0 }
}

struct Msg {
    id: u64,
    seq: u64,
    data: Vec<u64>,
}

/// senders on all threads but one, the receiver checks order and content and frees the blocks
fn g_channel(c: &Cfg) -> Tally {
    let senders = c.threads - 1;
    let iters = 4_000 * c.scale;
    let ends = Arc::new(channel::<Box<Msg>>(64));
    let bad = Arc::new(AtomicU64::new(0));
    let (e, b) = (ends.clone(), bad.clone());
    let ran = run(senders + 1, Arc::new(move |k| {
        let (tx, rx) = &*e;
        let mut wrong = 0;
        if k == 0 {
            let mut next = vec![0u64; senders];
            let (mut n, mut late) = (0, 0);
            while n < senders as u64 * iters && late < 2 {
                match rx.recv_timeout(WAIT_MS) {
                    Ok(m) => {
                        let id = m.id as usize;
                        let ok = id < senders && m.seq == next[id] && m.data.len() == 1 + (m.seq as usize * 7 + id) % 61
                            && m.data.iter().enumerate().all(|(i, &w)| w == word(m.id, m.seq, i as u64));
                        wrong += !ok as u64;
                        if id < senders {
                            next[id] = m.seq + 1;
                        }
                        n += 1;
                    }
                    Err(_) => {
                        late += 1;
                        wrong += 1;
                    }
                }
            }
            wrong += (n != senders as u64 * iters) as u64;
        } else {
            let id = k - 1;
            for s in 0..iters {
                let len = 1 + (s as usize * 7 + id) % 61;
                let data = (0..len as u64).map(|i| word(id as u64, s, i)).collect();
                if tx.send(Box::new(Msg { id: id as u64, seq: s, data })).is_err() {
                    wrong += 1;
                    break;
                }
            }
        }
        b.fetch_add(wrong, SeqCst);
    }));
    Tally { ops: senders as u64 * iters, bad: bad.load(SeqCst) + (ran != senders + 1) as u64, us: 0 }
}

/// two threads hand a value back and forth through two semaphores
fn g_sem(c: &Cfg) -> Tally {
    let iters = 5_000 * c.scale;
    let (Some(a), Some(b)) = (plat::Sem::new(0), plat::Sem::new(0)) else {
        return Tally { ops: 0, bad: 1, us: 0 };
    };
    let st = Arc::new((a, b, AtomicU64::new(0), AtomicU64::new(0)));
    let bad = Arc::new(AtomicU64::new(0));
    let (s, bd) = (st.clone(), bad.clone());
    let ran = run(2, Arc::new(move |k| {
        let (a, b, x, y) = &*s;
        let mut wrong = 0;
        for i in 1..=iters {
            if k == 0 {
                x.store(i, Relaxed);
                a.post(1);
                if !b.wait(WAIT_MS as i32) {
                    wrong += 1;
                    break;
                }
                wrong += (y.load(Relaxed) != i) as u64;
            } else {
                if !a.wait(WAIT_MS as i32) {
                    wrong += 1;
                    break;
                }
                wrong += (x.load(Relaxed) != i) as u64;
                y.store(i, Relaxed);
                b.post(1);
            }
        }
        bd.fetch_add(wrong, SeqCst);
    }));
    Tally { ops: iters, bad: bad.load(SeqCst) + (ran != 2) as u64, us: 0 }
}

// ---------------------------------------------------------------- the pool

const NMAX: usize = 256;

fn g_pool(c: &Cfg) -> Tally {
    plat::thr_init(c.threads as u32);
    let rounds = 1_500 * c.scale;
    let input: Vec<AtomicU64> = (0..NMAX).map(|_| AtomicU64::new(0)).collect();
    let output: Vec<AtomicU64> = (0..NMAX).map(|_| AtomicU64::new(0)).collect();
    let nest: Vec<AtomicU64> = (0..16 * 4).map(|_| AtomicU64::new(0)).collect();
    let bad_in = AtomicU64::new(0);
    let (mut ops, mut bad) = (0u64, 0u64);
    for r in 0..rounds {
        let n = 1 + (r as usize * 37) % NMAX;
        let tag = c.pass << 40 | r << 12;
        for (i, v) in input.iter().enumerate().take(n) {
            v.store(tag | i as u64, Relaxed); // before the loop: every worker must see it
        }
        let nested = r % 8 == 0;
        let _ = plat::parallel_for(n, &|i, _| {
            let v = input[i].load(Relaxed);
            if v != tag | i as u64 {
                bad_in.fetch_add(1, SeqCst);
            }
            output[i].store(v ^ MAGIC, Relaxed);
            if nested && i < 16 {
                let _ = plat::parallel_for(4, &|j, _| nest[i * 4 + j].store(v + j as u64, Relaxed));
            }
        });
        // after the loop: every item's results
        bad += output.iter().take(n).enumerate().filter(|&(i, v)| v.load(Relaxed) != (tag | i as u64) ^ MAGIC).count() as u64;
        if nested {
            for i in 0..n.min(16) {
                bad += (0..4).filter(|&j| nest[i * 4 + j].load(Relaxed) != (tag | i as u64) + j as u64).count() as u64;
            }
        }
        ops += n as u64;
    }
    Tally { ops, bad: bad + bad_in.load(SeqCst), us: 0 }
}

fn g_job(c: &Cfg) -> Tally {
    let rounds = 400 * c.scale;
    let io: Arc<(Vec<AtomicU64>, Vec<AtomicU64>)> =
        Arc::new(((0..NMAX).map(|_| AtomicU64::new(0)).collect(), (0..NMAX).map(|_| AtomicU64::new(0)).collect()));
    let mut job = plat::Job::new();
    let f = io.clone();
    let set = job.set(NMAX as u64, move |i, _| {
        let v = f.0[i].load(Relaxed);
        f.1[i].store(v ^ MAGIC, Relaxed);
    });
    let (mut ops, mut bad) = (0u64, !set as u64);
    for r in 0..rounds {
        let n = 1 + (r as usize * 53) % NMAX;
        let tag = c.pass << 40 | r << 12;
        bad += !job.set_count(n as u64) as u64;
        for (i, v) in io.0.iter().enumerate().take(n) {
            v.store(tag | i as u64, Relaxed);
        }
        bad += !job.start() as u64;
        if r % 2 == 0 {
            while !job.done() {
                core::hint::spin_loop(); // done() true: the items' results are here
            }
        } else {
            bad += job.wait().is_err() as u64;
        }
        bad += io.1.iter().take(n).enumerate().filter(|&(i, v)| v.load(Relaxed) != (tag | i as u64) ^ MAGIC).count() as u64;
        bad += (job.items_done() != n as u64) as u64;
        ops += n as u64;
    }
    Tally { ops, bad, us: 0 }
}

// ---------------------------------------------------------------- the heap

#[repr(align(64))]
struct A64([u64; 8]);
#[repr(align(128))]
struct A128([u64; 16]);
#[repr(align(4096))]
struct A4k([u64; 512]);

/// a heap block: plain words (size classes and page runs), 64-aligned (a page run from the cache),
/// over-aligned (pages of their own)
enum Blk {
    W(Vec<u64>),
    L(Box<A64>),
    A(Box<[A128; 3]>),
    P(Box<A4k>),
}

// words of the plain blocks: 8 bytes to 1 MiB
const SIZES: [usize; 24] = [1, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 96, 128, 192, 256, 300, 512, 1024, 2048, 4096, 8192, 32768, 131072];

fn rnd(g: &mut u64) -> u64 {
    *g = g.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *g >> 33
}

fn blk_make(g: &mut u64, tag: u64) -> Blk {
    let r = rnd(g);
    let fill = |n: usize| (0..n as u64).map(|i| tag ^ i.wrapping_mul(K)).collect::<Vec<u64>>();
    match r % 64 {
        0 => Blk::A(Box::new([A128([tag; 16]), A128([tag ^ 1; 16]), A128([tag ^ 2; 16])])),
        1 => Blk::P(Box::new(A4k([tag; 512]))),
        2..=5 => Blk::L(Box::new(A64([tag; 8]))),
        _ => {
            let k = (r / 64) as usize % SIZES.len();
            // the page runs (16 and up) one time in four
            let k = if k >= 16 && r % 4 != 0 { k - 16 } else { k };
            Blk::W(fill(SIZES[k]))
        }
    }
}

fn blk_ok(b: &Blk, tag: u64) -> bool {
    match b {
        Blk::W(v) => v.iter().enumerate().all(|(i, &w)| w == tag ^ (i as u64).wrapping_mul(K)),
        Blk::L(x) => (&**x as *const A64 as usize) % 64 == 0 && x.0.iter().all(|&w| w == tag),
        Blk::A(x) => {
            (x.as_ptr() as usize) % 128 == 0 && x.iter().enumerate().all(|(j, a)| a.0.iter().all(|&w| w == tag ^ j as u64))
        }
        Blk::P(x) => (&**x as *const A4k as usize) % 4096 == 0 && x.0.iter().all(|&w| w == tag),
    }
}

/// every thread sends blocks to the next one, which checks and frees them (another shard); each
/// keeps some small blocks of its own and frees them out of order
fn g_heap(c: &Cfg) -> Tally {
    let n = c.threads;
    let iters = 1_500 * c.scale;
    let chans: Arc<Vec<_>> = Arc::new((0..n).map(|_| channel::<(u64, Blk)>(16)).collect());
    let bad = Arc::new(AtomicU64::new(0));
    let (ch, b) = (chans.clone(), bad.clone());
    let pass = c.pass;
    let ran = run(n, Arc::new(move |k| {
        let tx = &ch[(k + 1) % n].0;
        let rx = &ch[k].1;
        let mut g = (k as u64 + 1).wrapping_mul(K) ^ pass;
        let (mut got, mut wrong) = (0u64, 0u64);
        let take = |b: (u64, Blk), got: &mut u64, wrong: &mut u64| {
            *wrong += !blk_ok(&b.1, b.0) as u64;
            *got += 1;
        };
        let mut local: Vec<(u64, Vec<u64>)> = Vec::new();
        for i in 0..iters {
            let tag = (k as u64) << 48 | pass << 32 | i;
            let mut m = (tag, blk_make(&mut g, tag));
            loop {
                match tx.try_send(m) {
                    Ok(()) => break,
                    Err(x) => {
                        m = x;
                        while let Some(b) = rx.try_recv() {
                            take(b, &mut got, &mut wrong);
                        }
                        plat::thread_yield();
                    }
                }
            }
            let s = 1 + rnd(&mut g) as usize % 40;
            local.push((tag, vec![tag; s]));
            if local.len() > 64 {
                let j = rnd(&mut g) as usize % local.len();
                let (t, v) = local.swap_remove(j);
                wrong += v.iter().any(|&w| w != t) as u64;
            }
            if i % 4 == 0 {
                while let Some(b) = rx.try_recv() {
                    take(b, &mut got, &mut wrong);
                }
            }
        }
        while got < iters {
            match rx.recv_timeout(WAIT_MS) {
                Ok(b) => take(b, &mut got, &mut wrong),
                Err(_) => {
                    wrong += 1;
                    break;
                }
            }
        }
        wrong += local.iter().filter(|(t, v)| v.iter().any(|w| w != t)).count() as u64;
        b.fetch_add(wrong, SeqCst);
    }));
    Tally { ops: n as u64 * iters, bad: bad.load(SeqCst) + (ran != n) as u64, us: 0 }
}

// ---------------------------------------------------------------- rings

/// threads push events, one pops them and checks order and content
fn g_evq(c: &Cfg) -> Tally {
    let producers = (c.threads - 1).min(16);
    let iters = 20_000 * c.scale;
    plat::evq_clear();
    let bad = Arc::new(AtomicU64::new(0));
    let b = bad.clone();
    let ran = run(producers + 1, Arc::new(move |k| {
        let mut wrong = 0;
        if k == 0 {
            let mut next = vec![0i32; producers];
            let (mut n, mut idle) = (0u64, plat::time_us());
            while n < producers as u64 * iters {
                match plat::evq_pop() {
                    Some(e) => {
                        let id = e.win as usize;
                        let ok = e.typ == plat::E_TEXT && id < producers && e.x == next[id] && e.y == !e.x
                            && e.data == word(id as u64, e.x as u64, 1) as u32;
                        wrong += !ok as u64;
                        if id < producers {
                            next[id] = e.x + 1;
                        }
                        n += 1;
                        idle = plat::time_us();
                    }
                    None if plat::time_us() - idle > WAIT_MS as u64 * 1000 => {
                        wrong += 1;
                        break;
                    }
                    None => plat::thread_yield(),
                }
            }
        } else {
            let id = k - 1;
            for s in 0..iters as i32 {
                while plat::evq_count() > 4096 {
                    plat::thread_yield();
                }
                let e = plat::Event { typ: plat::E_TEXT, win: id as u8, x: s, y: !s, data: word(id as u64, s as u64, 1) as u32, ..Default::default() };
                wrong += !plat::evq_push(e) as u64;
            }
        }
        b.fetch_add(wrong, SeqCst);
    }));
    plat::evq_clear();
    Tally { ops: producers as u64 * iters, bad: bad.load(SeqCst) + (ran != producers + 1) as u64, us: 0 }
}

/// the audio stream's lock-free ring: a writer thread, this one reads (plat::audio::ring_check)
fn g_ring(c: &Cfg) -> Tally {
    let n = (200_000 * c.scale as usize).min(16_000_000); // samples count up as f32: exact below 2^24
    let mut t = Tally::default();
    for (piece, take) in [(1, 1), (7, 13), (255, 64), (1000, 1000)] {
        t.bad += !plat::audio::ring_check(n, piece, take, true) as u64;
        t.ops += n as u64;
    }
    t
}

// ---------------------------------------------------------------- litmus

#[repr(align(128))]
#[derive(Default)]
struct Line(AtomicU64);

#[derive(Default)]
struct Lit {
    x: [Line; 2],
    y: [Line; 2],
    r0: [Line; 2],
    r1: [Line; 2],
    bar: Line,
    seen: AtomicU64,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    SbSeqCst,
    SbRmw,
    MpAcqRel,
    SbRelaxed,
    SbAcqRel,
    MpRelaxed,
}

/// both threads arrive at round i, then go on together
fn bar_wait(b: &AtomicU64, i: u64) {
    b.fetch_add(1, SeqCst);
    let want = 2 * (i + 1);
    let mut spins = 0u32;
    while b.load(Acquire) < want {
        spins += 1;
        if spins & 0x3FFF == 0 { plat::thread_yield() } else { core::hint::spin_loop() }
    }
}

/// round s of the pair: count the outcome the pattern forbids (or the control looks for), then
/// clear the slot for the round after next
fn judge(kind: Kind, l: &Lit, s: usize) {
    let (r0, r1) = (l.r0[s].0.load(Relaxed), l.r1[s].0.load(Relaxed));
    let seen = match kind {
        Kind::MpAcqRel | Kind::MpRelaxed => r1 == 0b10, // the flag, but not the data
        _ => r0 == 0 && r1 == 0,                        // each missed the other's store
    };
    l.seen.fetch_add(seen as u64, Relaxed);
    for v in [&l.x[s], &l.y[s], &l.r0[s], &l.r1[s]] {
        v.0.store(0, Relaxed);
    }
}

/// a two-thread litmus test, `iters` rounds that start together -> rounds with the outcome
fn litmus(kind: Kind, iters: u64) -> u64 {
    let l = Arc::new(Lit::default());
    let m = l.clone();
    run(2, Arc::new(move |k| {
        let l = &*m;
        for i in 0..iters {
            let s = (i & 1) as usize;
            bar_wait(&l.bar.0, i);
            if k == 1 && matches!(kind, Kind::MpAcqRel | Kind::MpRelaxed) {
                for _ in 0..(i * 7) % 24 {
                    core::hint::spin_loop(); // the reader a little later each round: over the writer's window
                }
            }
            let (x, y) = (&l.x[s].0, &l.y[s].0);
            match (kind, k) {
                (Kind::SbSeqCst, 0) => {
                    x.store(1, SeqCst);
                    l.r0[s].0.store(y.load(SeqCst), Relaxed);
                }
                (Kind::SbSeqCst, _) => {
                    y.store(1, SeqCst);
                    l.r1[s].0.store(x.load(SeqCst), Relaxed);
                }
                // plat::pool's wait_done / finish: dwait.fetch_add + state.load against state.swap + dwait.swap
                (Kind::SbRmw, 0) => {
                    x.fetch_add(1, SeqCst);
                    l.r0[s].0.store(y.load(SeqCst), Relaxed);
                }
                (Kind::SbRmw, _) => {
                    y.swap(1, SeqCst);
                    l.r1[s].0.store(x.swap(0, SeqCst), Relaxed);
                }
                (Kind::SbRelaxed, 0) => {
                    x.store(1, Relaxed);
                    l.r0[s].0.store(y.load(Relaxed), Relaxed);
                }
                (Kind::SbRelaxed, _) => {
                    y.store(1, Relaxed);
                    l.r1[s].0.store(x.load(Relaxed), Relaxed);
                }
                (Kind::SbAcqRel, 0) => {
                    x.store(1, Release);
                    l.r0[s].0.store(y.load(Acquire), Relaxed);
                }
                (Kind::SbAcqRel, _) => {
                    y.store(1, Release);
                    l.r1[s].0.store(x.load(Acquire), Relaxed);
                }
                (Kind::MpAcqRel, 0) => {
                    x.store(1, Relaxed);
                    y.store(1, Release);
                }
                (Kind::MpAcqRel, _) => {
                    let f = y.load(Acquire);
                    l.r1[s].0.store(f << 1 | x.load(Relaxed), Relaxed);
                }
                (Kind::MpRelaxed, 0) => {
                    x.store(1, Relaxed);
                    y.store(1, Relaxed);
                }
                (Kind::MpRelaxed, _) => {
                    let f = y.load(Relaxed);
                    l.r1[s].0.store(f << 1 | x.load(Relaxed), Relaxed);
                }
            }
            if k == 0 && i > 0 {
                judge(kind, l, s ^ 1); // the round before: the barrier saw both threads finish it
            }
        }
        bar_wait(&l.bar.0, iters);
        if k == 0 && iters > 0 {
            judge(kind, l, ((iters - 1) & 1) as usize);
        }
    }));
    l.seen.load(SeqCst)
}

#[derive(Default)]
struct Seq {
    seq: Line,
    data: [Line; 8],
    done: Line,
}

/// a writer updates eight words (eight cache lines) under a sequence count until the reader has
/// `want` consistent copies (or SEQ_US passed) -> (consistent copies, torn ones among them)
fn seqlock(want: u64, fenced: bool) -> (u64, u64) {
    const SEQ_US: u64 = 3_000_000;
    let st = Arc::new(Seq::default());
    let res = Arc::new((AtomicU64::new(0), AtomicU64::new(0)));
    let (s, r) = (st.clone(), res.clone());
    run(2, Arc::new(move |k| {
        if k == 0 {
            let mut v = 0u64;
            while s.done.0.load(Relaxed) == 0 {
                v += 1;
                let q = s.seq.0.load(Relaxed);
                s.seq.0.store(q + 1, Relaxed);
                if fenced {
                    fence(Release); // the odd count before the data
                }
                for d in &s.data {
                    d.0.store(v, Relaxed);
                }
                s.seq.0.store(q + 2, if fenced { Release } else { Relaxed });
                for _ in 0..v % 16 {
                    core::hint::spin_loop(); // pauses of different length: the reader gets whole copies too
                }
            }
        } else {
            let (mut reads, mut torn, mut tries) = (0u64, 0u64, 0u64);
            let t0 = plat::time_us();
            while reads < want {
                tries += 1;
                if tries & 0x3FF == 0 && plat::time_us() - t0 > SEQ_US {
                    break;
                }
                let q1 = s.seq.0.load(if fenced { Acquire } else { Relaxed });
                if q1 & 1 != 0 {
                    continue;
                }
                let first = s.data[0].0.load(Relaxed);
                let same = s.data[1..].iter().all(|d| d.0.load(Relaxed) == first);
                if fenced {
                    fence(Acquire); // the data before the second count
                }
                if s.seq.0.load(Relaxed) == q1 {
                    reads += 1;
                    torn += !same as u64;
                }
            }
            s.done.0.store(1, Relaxed);
            r.0.store(reads, SeqCst);
            r.1.store(torn, SeqCst);
        }
    }));
    (res.0.load(SeqCst), res.1.load(SeqCst))
}

// ---------------------------------------------------------------- the test

/// the way the atomic read-modify-writes go on this CPU and build
fn path() -> &'static str {
    match rt::outline_lse() {
        Some(true) => "LSE (outline atomics, AT_HWCAP)",
        Some(false) => "LL/SC (outline atomics: no LSE in AT_HWCAP, or jfgui_ARM_LLSC=1)",
        None if cfg!(all(target_arch = "aarch64", target_feature = "lse")) => "LSE (inline)",
        None if cfg!(target_arch = "aarch64") => "LL/SC (inline)",
        None => "x86-64 (locked instructions, TSO)",
    }
}

const CHECKS: [&str; 15] = [
    "atomics: rt::sync::SpinLock: no torn or lost update under contention",
    "atomics: plat::sync::Mutex (lock, try_lock): no torn or lost update",
    "atomics: plat::sync::RwLock: readers never see a half-done write",
    "atomics: Mutex + Condvar queue: every item once, in order, no lost wake-up",
    "atomics: channel: every message once, in order per sender, its heap block intact",
    "atomics: plat::Sem ping-pong: the value handed over, no lost post",
    "atomics: pool: parallel_for (nested) sees the caller's data, the caller every item's",
    "atomics: pool: a Job polled with done() or wait(): every item's result",
    "atomics: heap: blocks of every size handed between threads stay intact",
    "atomics: event ring: every event once, in order per thread",
    "atomics: audio ring: every sample once, in order (writer thread)",
    "atomics: SB with SeqCst stores and loads: never both old",
    "atomics: SB as the pool's wait (fetch_add + load / swap + swap): never both old",
    "atomics: MP with Release / Acquire: the flag brings the data",
    "atomics: seqlock with fences: no torn copy",
];
const UNITS: [&str; 15] = [
    "locks", "locks", "locks", "items", "messages", "hand-overs", "items", "items", "blocks", "events", "samples",
    "rounds", "rounds", "rounds", "copies",
];
const CONTROLS: [&str; 4] = ["SB Relaxed", "SB Release/Acquire", "MP Relaxed", "seqlock without fences"];

/// one pass of every group -> the checks' tallies, the controls' (rounds or copies, outcomes)
fn pass(c: &Cfg) -> ([Tally; 15], [Tally; 4]) {
    let lit = 20_000 * c.scale;
    let sb = |kind| timed(|| Tally { ops: lit, bad: litmus(kind, lit), us: 0 });
    let sl = |fenced| timed(|| {
        let (reads, torn) = seqlock(50_000 * c.scale, fenced);
        // a seqlock that never got a consistent copy proves nothing
        Tally { ops: reads, bad: torn + (fenced && reads == 0) as u64, us: 0 }
    });
    let checks = [
        timed(|| g_spin(c)),
        timed(|| g_mutex(c)),
        timed(|| g_rwlock(c)),
        timed(|| g_condvar(c)),
        timed(|| g_channel(c)),
        timed(|| g_sem(c)),
        timed(|| g_pool(c)),
        timed(|| g_job(c)),
        timed(|| g_heap(c)),
        timed(|| g_evq(c)),
        timed(|| g_ring(c)),
        sb(Kind::SbSeqCst),
        sb(Kind::SbRmw),
        sb(Kind::MpAcqRel),
        sl(true),
    ];
    let controls = [sb(Kind::SbRelaxed), sb(Kind::SbAcqRel), sb(Kind::MpRelaxed), sl(false)];
    (checks, controls)
}

pub fn t_atomics(t: &mut T) {
    let args = base::Args(plat::args());
    let secs = args.int("--secs", 0).max(0) as u64;
    let threads = args.int("--threads", plat::cpu_count() as i32).clamp(2, 16) as usize;
    let scale = args.int("--scale", 1).clamp(1, 1000) as u64;
    t.info(&format!("  atomics: {}; {threads} threads, {} CPUs\n", path(), plat::cpu_count()));
    let t0 = plat::time_us();
    let (mut checks, mut controls) = ([Tally::default(); 15], [Tally::default(); 4]);
    let mut passes = 0u64;
    loop {
        let c = Cfg { threads, scale, pass: passes };
        let (a, b) = pass(&c);
        for (s, x) in checks.iter_mut().zip(a).chain(controls.iter_mut().zip(b)) {
            s.ops += x.ops;
            s.bad += x.bad;
            s.us += x.us;
        }
        passes += 1;
        let el = plat::time_us() - t0;
        if secs > 0 {
            let errs: u64 = checks.iter().map(|x| x.bad).sum();
            out(&format!("  atomics pass {passes}: {errs} error(s), {} s\n", el / 1_000_000));
        }
        if secs == 0 || el >= secs * 1_000_000 {
            break;
        }
    }
    for (i, x) in checks.iter().enumerate() {
        t.check(x.bad == 0, CHECKS[i]);
        t.info(&format!("  ({} {}, {} error(s), {} ms)\n", x.ops, UNITS[i], x.bad, x.us / 1000));
    }
    let ctl: Vec<_> = controls.iter().zip(CONTROLS).map(|(x, n)| format!("{n} {} of {} ({} ms)", x.bad, x.ops, x.us / 1000)).collect();
    t.info(&format!("  controls (reorderings seen, not errors): {}\n", ctl.join(", ")));
    t.info(&format!("  {passes} pass(es) in {} ms\n", (plat::time_us() - t0) / 1000));
}
