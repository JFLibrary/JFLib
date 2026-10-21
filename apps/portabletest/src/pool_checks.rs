// Exercise actual overlapping WASM workers, borrowed/nested loops, allocator
// contention, cancellation, background completion and shutdown/restart.
use alloc::{format, sync::Arc, vec};
use core::sync::atomic::{AtomicU32, Ordering::SeqCst};

pub fn run() {
    assert_eq!(plat::thr_init(4), 4);
    let entered = AtomicU32::new(0);
    let slots = AtomicU32::new(0);
    let sum = AtomicU32::new(0);
    let deadline = plat::time_us() + 15_000_000;
    plat::parallel_for(64, &|i, _| {
        slots.fetch_or(1 << plat::thr_self(), SeqCst);
        if i < 4 {
            entered.fetch_add(1, SeqCst);
            while entered.load(SeqCst) < 4 {
                assert!(plat::time_us() < deadline, "compute workers did not overlap");
                core::hint::spin_loop();
            }
        }
        let mut bytes = vec![i as u8; 8192];
        plat::parallel_for(5, &|j, _| { sum.fetch_add((i * 5 + j) as u32, SeqCst); }).unwrap();
        bytes.reverse();
        assert!(bytes.iter().all(|&b| b == i as u8));
    }).unwrap();
    assert_eq!(sum.load(SeqCst), (0..320).sum::<u32>());
    assert!(slots.load(SeqCst).count_ones() >= 4);
    plat::evq_clear();
    plat::parallel_for(2048, &|_, _| plat::wake()).unwrap();
    assert_eq!(plat::evq_count(), 0, "worker wakes must not flood the input ring");
    let visits = Arc::new(AtomicU32::new(0));
    let mut job = plat::Job::new();
    let output = visits.clone();
    assert!(job.set(100, move |_, _| { output.fetch_add(1, SeqCst); }));
    assert!(job.start());
    job.wait().unwrap();
    assert!(job.done()); assert_eq!(job.items_done(), 100);
    assert!(job.set_count(23)); assert!(job.start()); job.wait().unwrap();
    assert_eq!(visits.load(SeqCst), 123);
    assert!(job.set(10_000, |i, ctl| { if i == 0 { ctl.cancel(); } }));
    assert!(job.start()); assert!(job.wait().is_err()); assert!(job.done());
    assert!(plat::parallel_for(100, &|_, ctl| ctl.cancel()).is_err());
    drop(job);
    plat::thr_shutdown();
    assert_eq!(plat::thr_init(2), 2);
    plat::parallel_for(16, &|_, _| {}).unwrap();
    plat::thr_shutdown();
    plat::write_stdout(format!("pool ok participants={} overlap=4 nested=320 cancellation restart\n", slots.load(SeqCst).count_ones()).as_bytes());
}
