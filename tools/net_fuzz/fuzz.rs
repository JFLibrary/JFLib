// net_fuzz -- a mutation fuzzer (no coverage feedback) over JFLib's parsers of untrusted bytes:
// zstd, LZ4, both X.509 parsers, both DNS parsers, both HTTP parsers, a TLS server fed a mutated
// ClientHello, TLS clients fed a mutated first server flight, IPv4 and IPv6 fragment reassembly,
// netpkt's dissector and its application-layer parsers (tools/net_fuzz.py builds and runs it).
//
//   fuzz SECONDS SEEDS OUT [TARGET...]
//   fuzz --dump-seeds SEEDS OUT          the seeds of every target as files (tools/cov_fuzz.py)
//
// The targets and their seeds are in targets.rs (shared with the libFuzzer targets).
// Every target runs on its own thread for SECONDS, from its seeds (built here; SEEDS/ca.pem: the
// system's certificates, SEEDS/*.zst: frames of the reference zstd) and mutations of them.  A
// panic is caught (the build has panic = "unwind", overflow checks and debug assertions), its
// location counted and the first input per location saved to OUT.  A call that takes longer than
// HANG_S is a hang: its input is saved and the program ends with exit code 2.  The report is one
// line per target, "target NAME RUNS SEEDS SLOWEST_US SLOWEST_BYTES PANIC_SITES", followed by
// "panic NAME COUNT LOCATION -- MESSAGE" lines.
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[path = "targets.rs"]
mod targets;
use targets::{targets, Rng};

/// the longest a single call may take before it counts as a hang
const HANG_S: u64 = 10;

thread_local! {
    static LOC: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn mutate(r: &mut Rng, seeds: &[Vec<u8>], d: &[u8]) -> Vec<u8> {
    let mut v = d.to_vec();
    let ops = 1 + r.below(6);
    for _ in 0..ops {
        let len = v.len();
        match r.below(11) {
            0 if len > 0 => {
                let i = r.below(len);
                v[i] ^= 1 << r.below(8);
            }
            1 if len > 0 => {
                let i = r.below(len);
                v[i] = r.next() as u8;
            }
            2 if len > 0 => {
                let i = r.below(len);
                v[i] = [0u8, 1, 0x7f, 0x80, 0xff, 0x40, 0x3f, 0xc0][r.below(8)];
            }
            3 if len > 1 => {
                let i = r.below(len - 1);
                let x: u16 = [0u16, 0xffff, 0x8000, 0x7fff, 1, len as u16, (len as u16).wrapping_add(1)][r.below(7)];
                v[i..i + 2].copy_from_slice(&x.to_be_bytes());
            }
            4 if len > 3 => {
                let i = r.below(len - 3);
                let x: u32 = [0u32, 0xffff_ffff, 0x8000_0000, 0x7fff_ffff, 0x0100_0000][r.below(5)];
                v[i..i + 4].copy_from_slice(&x.to_be_bytes());
            }
            5 if len > 0 => {
                let a = r.below(len);
                let b = (a + 1 + r.below(64)).min(len);
                v.drain(a..b);
            }
            6 if len > 0 => {
                let a = r.below(len);
                let b = (a + 1 + r.below(128)).min(len);
                let chunk = v[a..b].to_vec();
                let at = r.below(len + 1);
                for _ in 0..1 + r.below(4) {
                    v.splice(at..at, chunk.iter().copied());
                }
            }
            7 => {
                let at = r.below(len + 1);
                let n = 1 + r.below(16);
                let ins: Vec<u8> = (0..n).map(|_| r.next() as u8).collect();
                v.splice(at..at, ins);
            }
            8 if len > 0 => {
                v.truncate(r.below(len));
            }
            9 => {
                let o = &seeds[r.below(seeds.len())];
                if !o.is_empty() {
                    let a = r.below(o.len());
                    let b = (a + 1 + r.below(256)).min(o.len());
                    let at = r.below(len + 1);
                    v.splice(at..at, o[a..b].iter().copied());
                }
            }
            10 if len > 0 => {
                let i = r.below(len);
                v[i] = if r.next() & 1 == 0 { v[i].wrapping_add(1) } else { v[i].wrapping_sub(1) };
            }
            _ => {}
        }
    }
    if v.len() > 1 << 20 {
        v.truncate(1 << 20);
    }
    v
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // --dump-seeds SEEDS OUT: every target's seeds as OUT/NAME/N.bin (the corpus of tools/cov_fuzz.py)
    if args.get(1).map(String::as_str) == Some("--dump-seeds") && args.len() == 4 {
        for (name, seeds, _) in targets(&args[2]) {
            let d = format!("{}/{}", args[3], name);
            std::fs::create_dir_all(&d).unwrap();
            for (i, s) in seeds.iter().enumerate() {
                std::fs::write(format!("{}/seed{}.bin", d, i), s).unwrap();
            }
        }
        return;
    }
    if args.len() < 4 {
        eprintln!("usage: fuzz SECONDS SEEDS OUT [TARGET...]");
        std::process::exit(64);
    }
    let secs: u64 = args[1].parse().unwrap_or(30);
    let seeds_dir = args[2].clone();
    let out = args[3].clone();
    let only: Vec<String> = args.iter().skip(4).cloned().collect();
    std::fs::create_dir_all(&out).unwrap();
    panic::set_hook(Box::new(|info| {
        let loc = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() { s.to_string() } else if let Some(s) = info.payload().downcast_ref::<String>() { s.clone() } else { String::new() };
        LOC.with(|c| *c.borrow_mut() = Some(format!("{} -- {}", loc, msg.replace('\n', " "))));
    }));
    let ts = targets(&seeds_dir);
    for o in &only {
        if !ts.iter().any(|t| t.0 == o) {
            eprintln!("unknown target {} ({})", o, ts.iter().map(|t| t.0).collect::<Vec<_>>().join(", "));
            std::process::exit(64);
        }
    }
    let done = AtomicBool::new(false);
    let report = Mutex::new(Vec::<String>::new());
    // per target: when its current call began (us since start, 0: none) and its input
    let begun: Vec<AtomicU64> = ts.iter().map(|_| AtomicU64::new(0)).collect();
    let current: Vec<Mutex<Vec<u8>>> = ts.iter().map(|_| Mutex::new(Vec::new())).collect();
    let t0 = Instant::now();
    std::thread::scope(|sc| {
        // the watchdog: a call past HANG_S saves its input and ends the program
        sc.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(250));
                let now = t0.elapsed().as_micros() as u64;
                for (i, b) in begun.iter().enumerate() {
                    let at = b.load(Ordering::Relaxed);
                    if at != 0 && now.saturating_sub(at) > HANG_S * 1_000_000 {
                        let input = current[i].lock().map(|g| g.clone()).unwrap_or_default();
                        let _ = std::fs::write(format!("{}/{}-hang.bin", out, ts[i].0), &input);
                        println!("hang {} {} bytes, more than {} s", ts[i].0, input.len(), HANG_S);
                        std::process::exit(2);
                    }
                }
            }
        });
        let workers: Vec<(&str, _)> = ts.iter().enumerate().filter(|(_, t)| only.is_empty() || only.iter().any(|o| o == t.0)).map(|(ti, (name, seeds, f))| {
            let (out, report, begun, current) = (out.clone(), &report, &begun[ti], &current[ti]);
            (*name, std::thread::Builder::new().stack_size(64 << 20).spawn_scoped(sc, move || {
                let mut r = Rng(0x9E37_79B9_7F4A_7C15 ^ (ti as u64 + 1).wrapping_mul(0x1234_5678_9abc));
                let end = Instant::now() + Duration::from_secs(secs);
                let mut n = 0u64;
                let mut panics: BTreeMap<String, usize> = BTreeMap::new();
                let mut slow = (Duration::ZERO, 0usize);
                // the seeds as they are first
                let mut queue: Vec<Vec<u8>> = seeds.clone();
                assert!(!seeds.is_empty(), "the target {} has no seeds", name);
                while Instant::now() < end {
                    let input = if let Some(s) = queue.pop() {
                        s
                    } else {
                        let base = &seeds[r.below(seeds.len())];
                        let mut m = mutate(&mut r, seeds, base);
                        if r.below(4) == 0 {
                            m = mutate(&mut r, seeds, &m);
                        }
                        m
                    };
                    if let Ok(mut g) = current.lock() {
                        g.clear();
                        g.extend_from_slice(&input);
                    }
                    let started = Instant::now();
                    begun.store((t0.elapsed().as_micros() as u64).max(1), Ordering::Relaxed);
                    let res = panic::catch_unwind(AssertUnwindSafe(|| f(&input)));
                    begun.store(0, Ordering::Relaxed);
                    let dt = started.elapsed();
                    if dt > slow.0 {
                        slow = (dt, input.len());
                        let _ = std::fs::write(format!("{}/{}-slowest.bin", out, name), &input);
                    }
                    if res.is_err() {
                        let loc = LOC.with(|c| c.borrow_mut().take()).unwrap_or_default();
                        if !panics.contains_key(&loc) {
                            let _ = std::fs::write(format!("{}/{}-panic-{}.bin", out, name, panics.len() + 1), &input);
                        }
                        *panics.entry(loc).or_insert(0) += 1;
                    }
                    n += 1;
                }
                let mut s = format!("target {} {} {} {} {} {}\n", name, n, seeds.len(), slow.0.as_micros(), slow.1, panics.len());
                for (k, v) in &panics {
                    s += &format!("panic {} {} {}\n", name, v, k);
                }
                report.lock().unwrap().push(s);
            }).unwrap())
        }).collect();
        // a worker that died outside the calls it guards (a fault of this program, or a target
        // without seeds) is reported, not lost
        for (name, w) in workers {
            if w.join().is_err() {
                report.lock().unwrap().push(format!("worker-failed {}\n", name));
            }
        }
        done.store(true, Ordering::Relaxed);
    });
    let mut rep = report.into_inner().unwrap();
    rep.sort();
    for s in rep {
        print!("{}", s);
    }
}
