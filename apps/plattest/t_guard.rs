// t_guard -- (plat-shared) plattest --test guard: the crash guard (plat::guard_*) without a
// plugin: Win64 test functions of plat (sum, write to 0x10, ud2, int3, division by zero,
// stack overflow, a register clobber) under guard_call on this thread, nested guards and
// tags, a fault of host code inside a guard (it goes to the enclosing guard), faults on the
// pool's workers, a stack overflow on a thread of its own.
//   plattest --test guardlimit   a full thread table (ERR_LIMIT; the table stays full)
//   plattest --test guardcrash   an unguarded fault after guard_init still reaches the crash
//                                handler (exit code 3, the --test crash checks)
// (The assembly tested the guard through PaintCraft's plugins (tools/plugin_check.py); this
// module and t_lib.rs are their Rust counterpart without PaintCraft.)
//                          [ASMLIB/src/plat/plib.s, win/win_lib.s, linux/lx_guard.s]

use alloc::boxed::Box;
use alloc::format;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering::SeqCst};

use plat::{
    ERR_FAULT, ERR_LIMIT, GK_ACCESS, GK_ARITH, GK_BREAK, GK_ILLEGAL, GK_STACK, GuardInfo, guard_call, guard_init,
    guard_run, guard_tag, guard_test_code, guard_test_state, parallel_for, thread_spawn,
};

use crate::T;

const FAULT: u64 = ERR_FAULT as i64 as u64;
const T_SUM: u32 = 0;
const T_WRITE: u32 = 1;
const T_UD2: u32 = 2;
const T_INT3: u32 = 3;
const T_DIV0: u32 = 4;
const T_STACK: u32 = 5;
const T_CLOBBER: u32 = 6;

#[cfg(windows)]
mod code {
    pub const ACCESS: u32 = 0xC000_0005;
    pub const ILLEGAL: u32 = 0xC000_001D;
    pub const BREAK: u32 = 0x8000_0003;
    pub const DIV0: u32 = 0xC000_0094;
}
#[cfg(not(windows))]
mod code {
    pub const ACCESS: u32 = 11; // SIGSEGV
    pub const ILLEGAL: u32 = 4; // SIGILL
    pub const BREAK: u32 = 5; // SIGTRAP
    pub const DIV0: u32 = 8; // SIGFPE
}

fn tc(k: u32) -> usize {
    guard_test_code(k)
}

// host functions (Win64, as plugins call the host)
extern "win64" fn h_tag(_a: u64) -> u64 {
    guard_tag()
}

/// tag 9 inside: guard_tag there, then ours again -> (inner tag) | (outer tag after) << 32
extern "win64" fn h_nested_tag(_a: u64) -> u64 {
    let mut gi = GuardInfo { tag: 9, ..GuardInfo::default() };
    let (inner, _) = guard_call(h_tag as *const () as usize, 0, 0, 0, 0, Some(&mut gi));
    inner | guard_tag() << 32
}

/// an inner guard catches a fault; this function then returns normally
extern "win64" fn h_inner_fault(_a: u64) -> u64 {
    let mut gi = GuardInfo::default();
    let (r, _) = guard_call(tc(T_WRITE), 0x10, 0, 0, 0, Some(&mut gi));
    if r == FAULT && gi.kind == GK_ACCESS { 42 } else { 1 }
}

/// an inner guard catches ud2, then this host function itself faults (crash_test): the fault
/// must go to the enclosing guard
extern "win64" fn h_host_fault(_a: u64) -> u64 {
    let mut gi = GuardInfo::default();
    guard_call(tc(T_UD2), 0, 0, 0, 0, Some(&mut gi));
    plat::crash_test()
}

pub fn t_guard(t: &mut T) {
    t.check(guard_init() == 0 && guard_init() == 0, "guard: plat_guard_init (twice)");
    let mut gi = GuardInfo::default();
    // ---- a normal call: arguments, rax / rdx, no fault
    let r = guard_call(tc(T_SUM), 1, 2, 3, 4, Some(&mut gi));
    t.check(r == (30, 5) && gi.kind == 0, "guard: a normal call returns rax / rdx of the function");
    let r = guard_call(tc(T_SUM), 5, 0, 0, 7, None);
    t.check(r == (33, 2), "guard: without GUARDINFO");
    // ---- faults of each kind
    let w = tc(T_WRITE);
    gi = GuardInfo { kind: 77, ..GuardInfo::default() };
    let r = guard_call(w, 0x10, 0, 0, 0, Some(&mut gi));
    t.info(&format!("  (write: kind {} code 0x{:x} pc +{} addr 0x{:x})\n", gi.kind, gi.code, gi.pc.wrapping_sub(w as u64), gi.addr));
    t.check(r == (FAULT, FAULT), "guard: a fault returns ERR_FAULT in rax and rdx");
    t.check(gi.kind == GK_ACCESS && gi.code == code::ACCESS, "guard: write to 0x10 -> GK_ACCESS");
    t.check(gi.addr == 0x10 && gi.pc == w as u64, "guard: the data address and the faulting instruction");
    let k = tc(T_UD2);
    let r = guard_call(k, 0, 0, 0, 0, Some(&mut gi));
    t.check(r.0 == FAULT && gi.kind == GK_ILLEGAL && gi.code == code::ILLEGAL && gi.pc == k as u64 && gi.addr == 0,
            "guard: ud2 -> GK_ILLEGAL at the instruction");
    let k = tc(T_INT3);
    let r = guard_call(k, 0, 0, 0, 0, Some(&mut gi));
    t.info(&format!("  (int3: pc +{})\n", gi.pc.wrapping_sub(k as u64)));
    t.check(r.0 == FAULT && gi.kind == GK_BREAK && gi.code == code::BREAK && gi.pc.wrapping_sub(k as u64) <= 1,
            "guard: int3 -> GK_BREAK");
    let k = tc(T_DIV0);
    let r = guard_call(k, 0, 0, 0, 0, Some(&mut gi));
    t.check(r.0 == FAULT && gi.kind == GK_ARITH && gi.code == code::DIV0 && gi.pc == k as u64 + 9,
            "guard: division by zero -> GK_ARITH at the div");
    let r = guard_call(tc(T_SUM), 1, 1, 1, 1, Some(&mut gi));
    t.check(r == (10, 0) && gi.kind == 0, "guard: the next call after faults runs normally (info cleared)");
    // ---- state restored after a fault that clobbered everything
    let before = guard_test_state();
    let (mut x, mut y, mut z) = (0x1234_5678u64, 0x9ABCu64, 3.25f64);
    let mut ok = true;
    for i in 0..64u64 {
        let r = guard_call(tc(T_CLOBBER), 0x10, 0, 0, 0, Some(&mut gi));
        ok &= r.0 == FAULT && gi.kind == GK_ACCESS;
        x = x.rotate_left(3) ^ i;
        y = y.wrapping_mul(31).wrapping_add(x);
        z = z * 1.5 + i as f64;
    }
    let after = guard_test_state();
    t.info(&format!("  (MXCSR / x87 CW / DF: before {:x} {:x} {}, after {:x} {:x} {})\n", before.0, before.1, before.2,
                    after.0, after.1, after.2));
    t.check(ok, "guard: 64 faults after clobbering registers, MXCSR, x87, DF");
    // (MXCSR: the control bits; the sticky exception flags 0..5 change with any arithmetic)
    t.check(after.0 & !0x3F == before.0 & !0x3F && after.1 == before.1 && !after.2,
            "guard: MXCSR, the x87 control word and DF are restored");
    let (mut ex, mut ey, mut ez) = (0x1234_5678u64, 0x9ABCu64, 3.25f64);
    for i in 0..64u64 {
        ex = ex.rotate_left(3) ^ i;
        ey = ey.wrapping_mul(31).wrapping_add(ex);
        ez = ez * 1.5 + i as f64;
    }
    t.check(x == ex && y == ey && z.to_bits() == ez.to_bits(), "guard: values live across the faults are intact");
    // ---- tags and nesting
    t.check(guard_tag() == 0, "guard: plat_guard_tag outside a guard = 0");
    gi = GuardInfo { tag: 0x7A6, ..GuardInfo::default() };
    let r = guard_call(h_tag as *const () as usize, 0, 0, 0, 0, Some(&mut gi));
    t.check(r.0 == 0x7A6, "guard: plat_guard_tag inside = the GUARDINFO tag");
    gi = GuardInfo { tag: 7, ..GuardInfo::default() };
    let r = guard_call(h_nested_tag as *const () as usize, 0, 0, 0, 0, Some(&mut gi));
    t.check(r.0 == (9 | 7 << 32), "guard: nested guards: the innermost tag, then the outer one again");
    let r = guard_call(h_inner_fault as *const () as usize, 0, 0, 0, 0, Some(&mut gi));
    t.check(r.0 == 42 && gi.kind == 0, "guard: a fault inside a nested guard stays there");
    let r = guard_call(h_host_fault as *const () as usize, 0, 0, 0, 0, Some(&mut gi));
    t.check(r.0 == FAULT && gi.kind == GK_ACCESS && gi.addr == 0x10 && gi.pc == plat::crash_test as *const () as u64 + 5,
            "guard: a fault after an inner guard ended goes to the enclosing guard");
    t.check(guard_tag() == 0, "guard: no guard left active");
    // ---- guard_run (Rust closures)
    let v = guard_run(|| 6 * 7, None);
    t.check(v == Ok(42), "guard: plat::guard_run returns the closure's value");
    let v = guard_run(|| guard_call(tc(T_SUM), 2, 0, 0, 0, None).0, None);
    t.check(v == Ok(2), "guard: guard_run with a guarded call inside");
    let v: Result<(), i32> = guard_run(|| { plat::crash_test(); }, Some(&mut gi));
    t.check(v == Err(ERR_FAULT) && gi.kind == GK_ACCESS, "guard: guard_run of a faulting closure -> ERR_FAULT");
    // ---- the pool's workers: every other item faults
    let n = 256usize;
    let res: Arc<Vec<AtomicU64>> = Arc::new((0..n).map(|_| AtomicU64::new(0)).collect());
    let kinds = Arc::new(AtomicU32::new(0));
    {
        let res = res.clone();
        let kinds = kinds.clone();
        let _ = parallel_for(n, &move |i, _| {
            let mut gi = GuardInfo { tag: i as u64, ..GuardInfo::default() };
            let (r, _) = if i % 2 == 0 {
                guard_call(tc(T_SUM), i as u64, 0, 0, 0, Some(&mut gi))
            } else {
                guard_call(tc(if i % 4 == 1 { T_WRITE } else { T_DIV0 }), 0x10, 0, 0, 0, Some(&mut gi))
            };
            let want = if i % 4 == 1 { GK_ACCESS } else { GK_ARITH };
            if i % 2 == 1 && r == FAULT && gi.kind == want {
                kinds.fetch_add(1, SeqCst);
            }
            res[i].store(r, SeqCst);
        });
    }
    let good = (0..n).filter(|&i| if i % 2 == 0 { res[i].load(SeqCst) == i as u64 } else { res[i].load(SeqCst) == FAULT }).count();
    t.info(&format!("  ({} workers)\n", plat::thr_count()));
    t.check(good == n && kinds.load(SeqCst) == n as u32 / 2, "guard: 256 items on the workers, every other one faults");
    // ---- a stack overflow on a thread of its own
    let st = Arc::new(AtomicU64::new(0));
    {
        let st = st.clone();
        let th = thread_spawn(256 * 1024, Box::new(move || {
            let mut gi = GuardInfo::default();
            let (r, _) = guard_call(tc(T_STACK), 0, 0, 0, 0, Some(&mut gi));
            st.store(if r == FAULT { gi.kind as u64 | 1 << 32 } else { 0 }, SeqCst);
        }));
        if let Some(th) = th {
            th.join();
        }
    }
    let s = st.load(SeqCst);
    let want = if cfg!(windows) { GK_STACK } else { GK_ACCESS };
    t.info(&format!("  (stack overflow: kind {})\n", s as u32));
    t.check(s >> 32 == 1 && s as u32 == want, "guard: a stack overflow in a guarded call is caught");
}

/// plattest --test guardlimit: the thread table fills up (128 threads; it stays full, so this
/// is a mode of its own: --test guard can be repeated)
pub fn t_guardlimit(t: &mut T) {
    let r = guard_call(tc(T_SUM), 4, 0, 0, 0, None); // this thread takes its entry first
    t.check(r.0 == 4, "guard: a guarded call on the main thread");
    // (all threads stay alive until every one has made its call: distinct thread ids)
    let limit = Arc::new(AtomicU32::new(0));
    let done = Arc::new(AtomicU32::new(0));
    let go = Arc::new(AtomicU32::new(0));
    let mut ths = Vec::new();
    for _ in 0..140 {
        let (limit, done, go) = (limit.clone(), done.clone(), go.clone());
        if let Some(th) = thread_spawn(64 * 1024, Box::new(move || {
            let (r, _) = guard_call(tc(T_SUM), 1, 0, 0, 0, None);
            if r == ERR_LIMIT as i64 as u64 {
                limit.fetch_add(1, SeqCst);
            }
            done.fetch_add(1, SeqCst);
            while go.load(SeqCst) == 0 {
                plat::sleep_ms(1);
            }
        })) {
            ths.push(th);
        }
    }
    while (done.load(SeqCst) as usize) < ths.len() {
        plat::sleep_ms(1);
    }
    go.store(1, SeqCst);
    for th in ths {
        th.join();
    }
    t.info(&format!("  (ERR_LIMIT for {} of 140 new threads)\n", limit.load(SeqCst)));
    t.check(limit.load(SeqCst) >= 12, "guard: a full thread table -> ERR_LIMIT");
    let r = guard_call(tc(T_SUM), 3, 0, 0, 0, None);
    t.check(r.0 == 3, "guard: threads in the table keep their guard");
}

/// guard_init, then the crash test (an unguarded fault -> the crash handler, exit code 3)
pub fn t_guardcrash(t: &mut T) {
    guard_init();
    let r = guard_call(tc(T_UD2), 0, 0, 0, 0, None);
    t.check(r.0 == FAULT, "guardcrash: a guarded fault first");
    crate::t_crash(t);
}
