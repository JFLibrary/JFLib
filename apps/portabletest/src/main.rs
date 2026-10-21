// Deterministic portability checks: every scalar vector operation against native SSE2
// and AVX2, exact IEEE conversions/square roots/decimal scaling, and math error bounds.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::{format, vec::Vec};
mod vectors;
#[cfg(web_threads)]
mod pool_checks;

#[cfg(not(target_arch = "wasm32"))]
use rt::portable::{fx, math};
#[cfg(target_arch = "wasm32")]
use rt::{fx, x87 as math};

fn next(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

fn digest(hash: &mut u64, bytes: &[u8]) {
    for &v in bytes {
        *hash = (*hash ^ v as u64).wrapping_mul(0x100000001b3);
    }
}

fn compare_vectors(actual: &[u8], expected: &[u8], backend: &str, case: usize) {
    if let Some(at) = actual.iter().zip(expected).position(|(a, b)| a != b) {
        panic!(
            "{backend} case {case} byte {at}: scalar={} native={}",
            actual[at], expected[at]
        );
    }
    assert_eq!(actual.len(), expected.len());
}

fn vector_checks(hash: &mut u64) {
    let mut seed = 0x313273a653871541;
    for case in 0..512 {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        for i in 0..32 {
            a[i] = if case == 0 {
                0
            } else if case == 1 {
                255
            } else {
                next(&mut seed) as u8
            };
            b[i] = if case < 2 {
                255 - a[i]
            } else {
                next(&mut seed) as u8
            };
        }
        let scalar = vectors::run(simd::Scalar, &a, &b);
        let scalar256 = vectors::run(simd::Scalar256, &a, &b);
        compare_vectors(&scalar, &vectors::run(simd::Sse2, &a, &b), "baseline/SIMD128", case);
        #[cfg(not(target_arch = "wasm32"))]
        {
            compare_vectors(&scalar, &vectors::run(simd::Sse2, &a, &b), "SSE2", case);
            if let Some(avx2) = simd::Avx2::new() {
                compare_vectors(&scalar256, &vectors::run(avx2, &a, &b), "AVX2", case);
            }
        }
        digest(hash, &scalar);
        digest(hash, &scalar256);
    }
}

fn float_checks(hash: &mut u64) {
    let edges = [
        0, 1, 0x80000000, 0x007fffff, 0x00800000, 0x3f000000, 0x3fc00000, 0x40200000, 0xbfc00000,
        0xc0200000, 0x4effffff, 0x4f000000, 0xcf000000, 0xcf000001, 0x7f800000, 0xff800000,
        0x7fc01234, 0x7f800001,
    ];
    let mut seed = 0x984813dc02179452;
    for i in 0..20000 {
        let bits = if i < edges.len() {
            edges[i]
        } else {
            next(&mut seed) as u32
        };
        let x = f32::from_bits(bits);
        let results = [fx::cvtt(x) as u32, fx::cvt(x) as u32, fx::sqrt(x).to_bits()];
        #[cfg(not(target_arch = "wasm32"))]
        assert_eq!(
            results,
            [
                rt::fx::cvtt(x) as u32,
                rt::fx::cvt(x) as u32,
                rt::fx::sqrt(x).to_bits()
            ],
            "f32 {bits:08x}"
        );
        for value in results {
            digest(hash, &value.to_le_bytes());
        }
        let bits = next(&mut seed);
        let x = f64::from_bits(bits);
        let results = [
            fx::cvttd(x) as u64,
            fx::cvtd(x) as u64,
            fx::cvttd64(x) as u64,
            fx::cvtd64(x) as u64,
            fx::sqrtd(x).to_bits(),
        ];
        #[cfg(not(target_arch = "wasm32"))]
        assert_eq!(
            results,
            [
                rt::fx::cvttd(x) as u64,
                rt::fx::cvtd(x) as u64,
                rt::fx::cvttd64(x) as u64,
                rt::fx::cvtd64(x) as u64,
                rt::fx::sqrtd(x).to_bits()
            ],
            "f64 {bits:016x}"
        );
        for value in results {
            digest(hash, &value.to_le_bytes());
        }
    }
    assert_eq!(fx::minss(0.0, -0.0).to_bits(), (-0.0f32).to_bits());
    assert_eq!(fx::maxsd(f64::NAN, 3.0), 3.0);
    assert_eq!(fx::cvtd(2147483647.5), i32::MIN);
    assert_eq!(fx::cvtd(-2147483648.5), i32::MIN);
    assert_eq!(fx::nearest(-0.5).to_bits(), (-0.0f64).to_bits());
}

fn decimal_checks(hash: &mut u64) {
    let mut seed = 0x653cdbc417134854;
    for _ in 0..4000 {
        let m = next(&mut seed) & i64::MAX as u64;
        let exponent = (next(&mut seed) % 640) as i32 - 340;
        let value = math::scale(m, exponent);
        #[cfg(not(target_arch = "wasm32"))]
        assert_eq!(
            value.to_bits(),
            rt::x87::scale(m, exponent).to_bits(),
            "scale {m} {exponent}"
        );
        digest(hash, &value.to_bits().to_le_bytes());
        let v = math::scale(m, -12);
        let k = (next(&mut seed) % 28) as i32 - 14;
        let result = math::scale_round(v, k);
        #[cfg(not(target_arch = "wasm32"))]
        assert_eq!(result, rt::x87::scale_round(v, k), "scale_round {v} {k}");
        digest(hash, &result.to_le_bytes());
    }
    for v in [0.0, 0.5, 1.5, 2.5, -0.5, -1.5, -2.5, 0.005, 1e-300, 1e300] {
        for k in [-300, -3, -1, 0, 1, 2, 300] {
            #[cfg(not(target_arch = "wasm32"))]
            assert_eq!(
                math::scale_round(v, k),
                rt::x87::scale_round(v, k),
                "scale edge {v} {k}"
            );
            digest(hash, &math::scale_round(v, k).to_le_bytes());
        }
    }
}

fn math_checks(hash: &mut u64) {
    for i in -1000..=1000 {
        let x = i as f64 / 50.0;
        let exp = math::exp2(x);
        let log = math::log2(exp);
        let (c, s) = math::sincos(x);
        let at = math::atan2(x, 0.75);
        assert!((log - x).abs() < 1e-13);
        assert!((s * s + c * c - 1.0).abs() < 2e-15);
        #[cfg(not(target_arch = "wasm32"))]
        {
            assert!((exp - rt::x87::exp2(x)).abs() <= exp * 2e-15, "exp2 {x}");
            assert!((c - rt::x87::cos(x)).abs() < 2e-15, "cos {x}");
            assert!((s - rt::x87::sin(x)).abs() < 2e-15, "sin {x}");
            assert!((at - rt::x87::atan2(x, 0.75)).abs() < 1e-15, "atan2 {x}");
        }
        for value in [exp, log, c, s, at] {
            digest(hash, &value.to_bits().to_le_bytes());
        }
    }
    assert_eq!(math::atan2(-0.0, 1.0).to_bits(), (-0.0f64).to_bits());
    assert_eq!(math::log2(0.0), f64::NEG_INFINITY);
    assert!(math::log2(-1.0).is_nan());
    assert_eq!(math::pow(-1.0, 2.0), 0.0);
    assert_eq!(math::pow(f64::NAN, 0.0), 1.0);
    assert_eq!(math::exp2(-1074.0).to_bits(), 1);
}

fn allocation_checks(hash: &mut u64) {
    for round in 0..6 {
        let mut blocks: Vec<Vec<u8>> = Vec::new();
        for len in [1, 15, 17, 32, 65, 128, 2047, 2048, 2049, 65537, 1048577] {
            let mut b = alloc::vec![0; len];
            assert!(b.iter().all(|&v| v == 0));
            for (i, v) in b.iter_mut().enumerate() {
                *v = (i + round) as u8;
            }
            blocks.push(b);
        }
        for b in &blocks {
            digest(hash, b);
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn browser_files() {
    assert!(plat::mkdir("/documents/research"));
    assert!(plat::write_file(
        "/documents/research/test.bin",
        b"portable"
    ));
    let file = plat::File::open("/documents/research/test.bin", plat::FO_RW).unwrap();
    assert_eq!(file.size(), Some(8));
    assert!(
        !file.sync(),
        "temporary bytes must not claim durable storage"
    );
    assert_eq!(file.write_at(8, b"-web"), Some(4));
    let mut bytes = [0u8; 12];
    assert_eq!(file.read_at(0, &mut bytes), Some(12));
    assert_eq!(&bytes, b"portable-web");
    assert!(plat::rename(
        "/documents/research/test.bin",
        "/documents/research/renamed.bin"
    ));
    assert_eq!(
        plat::read_file("/documents/research/renamed.bin").unwrap(),
        bytes
    );
    assert!(plat::read_file("/../../outside").is_none());
    let lock = plat::FileLock::acquire("/documents/research/test.lock").unwrap();
    assert!(plat::FileLock::acquire("/documents/research/test.lock").is_none());
    drop(lock);
    assert!(plat::FileLock::acquire("/documents/research/test.lock").is_some());
    assert!(!plat::capabilities().supports(plat::CAP_NATIVE_PLUGINS | plat::CAP_THREADS));
    assert!(plat::lib_open("something.dll").is_err());
    assert!(plat::delete("/documents/research/renamed.bin"));
    assert!(plat::delete("/documents/research"));
}

#[cfg(target_arch = "wasm32")]
fn browser_completions() {
    use plat::web::{RequestStatus as Status, jfgui_complete, forget_request, request_status};
    const PATH: &str = "/documents/Quellen-ä.bin";
    assert!(plat::write_file(PATH, b"first"));
    let file = plat::File::open(PATH, plat::FO_RW).unwrap();
    let request = plat::web::persist(PATH).unwrap();
    assert_eq!(request_status(request), Some(Status::Pending));
    assert!(!file.sync());
    assert!(
        !forget_request(request),
        "pending requests must remain observable"
    );
    assert_eq!(jfgui_complete(request, 0), 1);
    assert_eq!(request_status(request), Some(Status::Durable));
    assert!(file.sync());
    assert_eq!(
        jfgui_complete(request, -1),
        0,
        "duplicate completions are rejected"
    );
    assert!(forget_request(request));
    assert_eq!(request_status(request), None);
    assert_eq!(
        jfgui_complete(request, 0),
        0,
        "forgotten ids cannot be reused"
    );

    let stale = plat::web::persist(PATH).unwrap();
    assert_eq!(file.write_at(0, b"newer"), Some(5));
    assert_eq!(jfgui_complete(stale, 0), 1);
    assert_eq!(request_status(stale), Some(Status::Durable));
    assert!(
        !file.sync(),
        "an old snapshot cannot mark changed bytes as stored"
    );
    assert!(forget_request(stale));

    let deleted = plat::web::persist(PATH).unwrap();
    assert!(plat::delete(PATH));
    assert!(plat::write_file(PATH, b"replacement"));
    let replacement = plat::File::open(PATH, plat::FO_READ).unwrap();
    assert_eq!(jfgui_complete(deleted, 0), 1);
    assert!(
        !replacement.sync(),
        "delete/recreate must not reuse a revision"
    );
    assert!(forget_request(deleted));

    let download = plat::web::download(PATH).unwrap();
    assert_eq!(jfgui_complete(download, 1), 1);
    assert_eq!(request_status(download), Some(Status::Exported));
    assert!(
        !replacement.sync(),
        "a download is not origin-storage persistence"
    );
    assert!(forget_request(download));

    for (code, expected) in [
        (-1, Status::Denied),
        (-2, Status::Quota),
        (-3, Status::Unsupported),
        (-4, Status::Cancelled),
        (-5, Status::Failed),
        (1, Status::Failed),
    ] {
        let request = plat::web::persist(PATH).unwrap();
        assert_eq!(jfgui_complete(request, code), 1);
        assert_eq!(request_status(request), Some(expected));
        assert!(!replacement.sync());
        assert!(forget_request(request));
    }
    let transient = plat::web::pick_file(".docx,.txt").unwrap();
    assert_eq!(jfgui_complete(transient, 2), 1);
    assert_eq!(request_status(transient), Some(Status::Completed));
    assert!(forget_request(transient));
    assert_eq!(jfgui_complete(u32::MAX, 0), 0);

    let first = plat::web::persist(PATH).unwrap();
    assert_eq!(jfgui_complete(first, 0), 1);
    for _ in 0..512 {
        let request = plat::web::persist(PATH).unwrap();
        assert_eq!(jfgui_complete(request, 0), 1);
        assert_eq!(request_status(request), Some(Status::Durable));
    }
    assert_eq!(
        request_status(first),
        None,
        "old terminal task history must expire"
    );

    let pending: Vec<u32> = (0..256)
        .map(|_| plat::web::pick_file(".txt").unwrap())
        .collect();
    assert!(
        plat::web::pick_file(".txt").is_none(),
        "host requests are bounded"
    );
    for id in pending {
        assert_eq!(jfgui_complete(id, -4), 1);
        assert!(forget_request(id));
    }
    assert!(plat::delete(PATH));
}

#[cfg(all(target_arch = "wasm32", not(web_threads)))]
fn browser_jobs() {
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicUsize, Ordering};
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let mut job = plat::Job::new();
    assert!(job.start());
    assert!(job.done());
    assert!(job.set(40, move |i, ctl| {
        assert_eq!(ctl.items_done(), i as u64);
        observed.fetch_add(1, Ordering::Relaxed);
    }));
    assert!(job.start());
    assert_eq!(
        count.load(Ordering::Relaxed),
        0,
        "starting a job must yield"
    );
    assert!(!job.start());
    assert!(!job.set_count(2));
    plat::pump_jobs(3);
    assert_eq!(job.items_done(), 3);
    assert_eq!(count.load(Ordering::Relaxed), 3);
    job.cancel();
    plat::pump_jobs(16);
    assert!(job.done());
    assert_eq!(job.wait(), Err(plat::Cancelled));
    assert_eq!(count.load(Ordering::Relaxed), 3);
    assert!(job.set_count(4));
    assert!(job.start());
    assert_eq!(job.wait(), Ok(()));
    assert_eq!(job.items_done(), 4);
    assert_eq!(count.load(Ordering::Relaxed), 7);
    assert!(job.start());
    drop(job);
    plat::pump_jobs(64);
    assert_eq!(
        count.load(Ordering::Relaxed),
        7,
        "dropped jobs cannot keep running"
    );
    let count = AtomicUsize::new(0);
    assert_eq!(
        plat::parallel_for(100, &|i, ctl| {
            count.fetch_add(1, Ordering::Relaxed);
            if i == 3 {
                ctl.cancel();
            }
        }),
        Err(plat::Cancelled)
    );
    assert_eq!(count.load(Ordering::Relaxed), 4);
    plat::evq_clear();
}

#[cfg(target_arch = "wasm32")]
struct BrowserProbe {
    enabled: bool,
}
#[cfg(target_arch = "wasm32")]
impl plat::web::WebApplication for BrowserProbe {
    fn tick(&mut self, _time_ms: u32) {}
    fn control(&mut self, line: &[u8]) -> Option<alloc::string::String> {
        self.enabled
            .then(|| format!("probe:{}", core::str::from_utf8(line).unwrap()))
    }
    fn set_control_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

// plat::task and plat::aio with the same code on every target: native threads, the browser's
// blocking jobs inline between runs (no threads without shared memory)
#[cfg(not(web_threads))]
fn task_checks() {
    use plat::aio::{self, Ctl, IoError};
    use plat::task::{block_on, join, sleep, spawn_blocking, yield_now, Executor};
    let v = block_on(join(spawn_blocking(|| 2u32), async {
        yield_now().await;
        3u32
    }));
    assert_eq!(v, (2, 3));
    // the UI executor, run step by step like a host's frames
    let mut ex = Executor::ui();
    let h = ex.spawn(async { spawn_blocking(|| 40u32).await + 2 });
    let deadline = plat::time_us() + 5_000_000;
    while !h.is_done() {
        ex.accept_remote();
        ex.run();
        assert!(plat::time_us() < deadline, "UI tasks must finish");
    }
    assert_eq!(h.take(), Some(42));
    let t0 = plat::time_us();
    block_on(sleep(5));
    assert!(plat::time_us() - t0 >= 5000, "sleep must not end early");
    let data: Vec<u8> = (0..10_000u32).map(|i| (i * 7) as u8).collect();
    let ctl = Ctl { chunk: 1000, ..Ctl::new() };
    assert_eq!(block_on(aio::write_file("mem://portable/a.bin", data.clone(), &ctl)), Ok(()));
    assert_eq!(block_on(aio::read_file("mem://portable/a.bin", &ctl)).as_deref(), Ok(&data[..]));
    assert_eq!(ctl.progress.done(), 10_000);
    let stop = Ctl::new();
    stop.cancel.cancel();
    assert_eq!(block_on(aio::copy_file("mem://portable/a.bin", "mem://portable/b.bin", &stop)), Err(IoError::Cancelled));
    assert_eq!(plat::file_exists("mem://portable/b.bin"), 0);
    assert!(plat::delete("mem://portable/a.bin"));
}

fn main() -> i32 {
    #[cfg(web_threads)]
    pool_checks::run();
    #[cfg(not(web_threads))]
    task_checks();
    let _ = plat::args();
    let mut hash = 0xcbf29ce484222325;
    vector_checks(&mut hash);
    float_checks(&mut hash);
    decimal_checks(&mut hash);
    math_checks(&mut hash);
    allocation_checks(&mut hash);
    #[cfg(target_arch = "wasm32")]
    {
        browser_files();
        browser_completions();
        #[cfg(not(web_threads))]
        browser_jobs();
        assert!(plat::web::install(alloc::boxed::Box::new(BrowserProbe {
            enabled: false
        })));
    }
    rt::os::write_stdout(format!("portabletest ok digest={hash:016x}\n").as_bytes());
    0
}

rt::main!(main());
