// t_async -- plattest --test async: async execution and files (lib/plat/task.rs, aio.rs)
//
//   executors   spawn order, yield_now interleaving, a task spawned from a task, join / join3 /
//               join_all / select, block_on
//   wake-ups    a CancelToken cancelled on another thread wakes block_on (its semaphore); the UI
//               executor takes another thread's wake-up only after accept_remote and posts
//               plat::wake (wait_events sees E_WAKE)
//   pool        40 blocking jobs (more than BLOCKING_MAX); a queued job whose future was dropped
//               never runs; spawn_cancellable's token when the future goes
//   cancel      Task::cancel drops the future at the next run, awaiting -> Err(Cancelled); a
//               dropped handle cancels, detach runs on; CancelToken::run; a dropped executor
//   timers      sleeps end in deadline order, never early; the virtual clock (clock_set)
//   progress    permille, changed() woken by another thread
//   files       aio read / write / copy (OS files, mem://, a DirFs mount) with progress;
//               cancelled between chunks through a gated mount (gated://: reads and writes past
//               the first chunk wait at a gate; the test cancels once the job waits there), the
//               target unchanged, no sibling left; a dropped future; Reader / Writer;
//               AtomicFile; Linux: access bits kept
//   panics      a panic in a task / a blocking job ends the child with exit code 3
//   permissions (a child: the manifest stays installed) refused reads and writes fail without
//               leftovers; a trusted() caller stays trusted on the pool
//
//   plattest --test async-panic | async-panic-job | async-policy   the children

use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::future::Future;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::SeqCst};

use plat::aio::{self, Ctl, IoError, Reader, Writer};
use plat::task::{
    self, block_on, join, join3, join_all, select, sleep, spawn_blocking, spawn_cancellable, yield_now, CancelToken, Either, Executor,
    Progress,
};
use plat::vfs::{self, MemFs, Opened, Provider, Stream};
use plat::{AtomicFile, Cancelled, Sem};

use crate::T;

pub fn t_async(t: &mut T) {
    executors(t);
    wakeups(t);
    pool(t);
    cancellation(t);
    timers(t);
    progress(t);
    t.scratch_init();
    files(t);
    gated(t);
    children(t);
    t.scratch_done();
}

fn time() -> u64 {
    plat::time_us()
}

// until no blocking job is in flight (at most 10 s)
fn pool_idle() -> bool {
    let dl = time() + 10_000_000;
    while task::blocking_pending() > 0 && time() < dl {
        plat::sleep_ms(1);
    }
    task::blocking_pending() == 0
}

// ---------------------------------------------------------------- executors

fn executors(t: &mut T) {
    let log = Rc::new(RefCell::new(Vec::<u32>::new()));
    let mut ex = Executor::new();
    let mut hs = Vec::new();
    for i in 1..=3u32 {
        let l = log.clone();
        hs.push(ex.spawn(async move {
            l.borrow_mut().push(i);
            i * 10
        }));
    }
    t.check(log.borrow().is_empty() && ex.live() == 3, "async: spawned tasks wait for the executor's run");
    ex.run();
    t.check(*log.borrow() == [1, 2, 3], "async: tasks run in spawn order");
    t.check(hs.iter().all(|h| h.is_done()) && ex.live() == 0, "async: finished tasks leave the executor");
    t.check(hs.iter().map(|h| h.take().unwrap_or(0)).sum::<u32>() == 60, "async: Task::take gives the results");
    t.check(hs[0].take().is_none(), "async: a result is taken once");

    log.borrow_mut().clear();
    for i in [1u32, 2] {
        let l = log.clone();
        ex.spawn(async move {
            l.borrow_mut().push(i);
            yield_now().await;
            l.borrow_mut().push(i + 10);
        })
        .detach();
    }
    while ex.has_ready() {
        ex.run();
    }
    t.check(*log.borrow() == [1, 2, 11, 12], "async: yield_now lets the other task run first");

    let sp = ex.spawner();
    let v = ex.block_on(async move { sp.spawn(async { 7 }).await });
    t.check(v == Ok(7), "async: a task spawned from inside a task, awaited through its handle");

    let (a, b) = block_on(join(async { 1 }, async {
        yield_now().await;
        2
    }));
    t.check((a, b) == (1, 2), "async: join");
    let (a, b, c) = block_on(join3(async { 1 }, async { 2 }, async { 3 }));
    t.check((a, b, c) == (1, 2, 3), "async: join3");
    let v = block_on(join_all((0..5u32).map(|i| async move {
        if i % 2 == 0 {
            yield_now().await;
        }
        i
    })));
    t.check(v == [0, 1, 2, 3, 4], "async: join_all keeps the order");
    let s = block_on(select(sleep(60_000), async { 9 }));
    t.check(s == Either::Right(9), "async: select gives the first to finish");
    t.check(task::next_timer().is_none(), "async: the losing sleep went with its timer");
}

// ---------------------------------------------------------------- wake-ups

fn wakeups(t: &mut T) {
    // the other thread cancels 30 ms after block_on's future ran (it waits for that: a main
    // thread that comes late cannot make the cancel come first); block_on returns after the
    // cancel, within 5 s
    let tok = CancelToken::new();
    let (entered, at) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU64::new(0)));
    let (t2, e2, a2) = (tok.clone(), entered.clone(), at.clone());
    let th = plat::thread_spawn(0, Box::new(move || {
        let dl = plat::time_us() + 30_000_000;
        while !e2.load(SeqCst) && plat::time_us() < dl {
            plat::sleep_ms(1);
        }
        plat::sleep_ms(30);
        a2.store(plat::time_us(), SeqCst); // when it cancelled
        t2.cancel();
    }));
    let t0 = time();
    block_on(async {
        entered.store(true, SeqCst);
        tok.cancelled().await
    });
    let t1 = time();
    if let Some(th) = th {
        th.join();
    }
    let at = at.load(SeqCst);
    t.check(at != 0 && t1 >= at && t1 - at < 5_000_000 && t1 - t0 >= 20_000, "async: block_on sleeps until another thread wakes it");

    // the UI executor: a pool job's wake-up waits for accept_remote; plat::wake was posted.  The
    // job waits until the task's first poll is over: had it ended before, that poll would have
    // found its value (no waker, no wake-up)
    let Some(go) = Sem::new(0).map(Arc::new) else {
        t.check(false, "async: Sem::new");
        return;
    };
    let mut ui = Executor::ui();
    while plat::wait_events(0, &mut |_| {}) > 0 {}
    let g = go.clone();
    let job = move || {
        g.wait(30_000);
        5u32
    };
    let h = ui.spawn(async move { spawn_blocking(job).await + 1 });
    ui.run();
    let waits = !h.is_done();
    go.post(1);
    t.check(waits && pool_idle(), "async: the UI task waits for its blocking job");
    ui.run();
    t.check(!h.is_done() && ui.busy(), "async: another thread's wake-up waits for accept_remote (E_WAKE)");
    // posted already: the job wakes the task before it leaves blocking_pending
    let mut woke = false;
    plat::wait_events(time() + 2_000_000, &mut |e| woke |= e.typ == plat::E_WAKE);
    t.check(woke, "async: the wake-up posted plat::wake (E_WAKE)");
    t.check(ui.accept_remote(), "async: accept_remote takes it");
    ui.run();
    t.check(h.take() == Some(6) && !ui.busy(), "async: the UI task finished after E_WAKE");
}

// ---------------------------------------------------------------- the blocking pool

fn pool(t: &mut T) {
    let v = block_on(join_all((0..40u32).map(|i| {
        spawn_blocking(move || {
            plat::sleep_ms(2);
            i * 2
        })
    })));
    t.check(v.len() == 40 && v.iter().enumerate().all(|(i, x)| *x == i as u32 * 2), "async: 40 blocking jobs (more than BLOCKING_MAX) finish");

    // every worker blocked; a job queued behind them whose future is dropped never runs
    let Some(gate) = Sem::new(0).map(Arc::new) else {
        t.check(false, "async: Sem::new");
        return;
    };
    let started = Arc::new(AtomicU32::new(0));
    let busy: Vec<_> = (0..task::BLOCKING_MAX)
        .map(|_| {
            let (g, s) = (gate.clone(), started.clone());
            spawn_blocking(move || {
                s.fetch_add(1, SeqCst);
                g.wait(10_000);
            })
        })
        .collect();
    let dl = time() + 5_000_000;
    while started.load(SeqCst) < task::BLOCKING_MAX && time() < dl {
        plat::sleep_ms(1);
    }
    t.check(started.load(SeqCst) == task::BLOCKING_MAX, "async: BLOCKING_MAX jobs run at once");
    let ran = Arc::new(AtomicBool::new(false));
    let r = ran.clone();
    drop(spawn_blocking(move || r.store(true, SeqCst)));
    gate.post(task::BLOCKING_MAX);
    block_on(join_all(busy));
    t.check(pool_idle() && !ran.load(SeqCst), "async: a queued job whose future was dropped is skipped");

    let (seen, go) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let (s, g) = (seen.clone(), go.clone());
    let f = spawn_cancellable(move |tok| {
        g.store(true, SeqCst);
        let dl = plat::time_us() + 5_000_000;
        while !tok.is_cancelled() && plat::time_us() < dl {
            plat::sleep_ms(1);
        }
        s.store(tok.is_cancelled(), SeqCst);
    });
    let dl = time() + 5_000_000;
    while !go.load(SeqCst) && time() < dl {
        plat::sleep_ms(1);
    }
    drop(f);
    t.check(pool_idle() && seen.load(SeqCst), "async: spawn_cancellable's token tells the job its future was dropped");
}

// ---------------------------------------------------------------- cancellation

struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, SeqCst);
    }
}

fn cancellation(t: &mut T) {
    let mut ex = Executor::new();
    let dropped = Arc::new(AtomicBool::new(false));
    let d = DropFlag(dropped.clone());
    let tok = CancelToken::new();
    let tk = tok.clone();
    let h = ex.spawn(async move {
        let _d = d;
        tk.cancelled().await;
        1
    });
    ex.run();
    t.check(!h.is_done(), "async: a task waits");
    h.cancel();
    t.check(!dropped.load(SeqCst), "async: cancel takes effect at the executor's next run");
    ex.run();
    t.check(dropped.load(SeqCst) && h.is_done() && h.is_cancelled() && h.take().is_none(), "async: a cancelled task's future is dropped");
    t.check(ex.block_on(h) == Err(Cancelled), "async: awaiting a cancelled task gives Err(Cancelled)");

    let a = Arc::new(AtomicU32::new(0));
    let (a1, a2) = (a.clone(), a.clone());
    drop(ex.spawn(async move {
        a1.fetch_add(1, SeqCst);
    }));
    ex.spawn(async move {
        a2.fetch_add(10, SeqCst);
    })
    .detach();
    ex.run();
    t.check(a.load(SeqCst) == 10 && ex.live() == 0, "async: a dropped handle cancels its task, a detached one runs");

    let tok = CancelToken::new();
    let t2 = tok.clone();
    let r = block_on(async move {
        join(tok.run(sleep(60_000)), async move {
            yield_now().await;
            t2.cancel();
        })
        .await
        .0
    });
    t.check(r == Err(Cancelled) && task::next_timer().is_none(), "async: CancelToken::run ends with Err(Cancelled) and drops the future");
    let tok = CancelToken::new();
    t.check(block_on(tok.run(async { 3 })) == Ok(3) && tok.check().is_ok(), "async: CancelToken::run passes the value on");
    tok.cancel();
    t.check(tok.check() == Err(Cancelled) && tok.is_cancelled(), "async: CancelToken::check after cancel");

    let ex2 = Executor::new();
    let h = ex2.spawn(async { 5 });
    drop(ex2);
    t.check(h.is_done() && h.is_cancelled(), "async: the tasks of a dropped executor end as cancelled");
}

// ---------------------------------------------------------------- timers

fn timers(t: &mut T) {
    let order = Rc::new(RefCell::new(Vec::new()));
    let mut ex = Executor::new();
    for ms in [30u32, 10, 20] {
        let o = order.clone();
        ex.spawn(async move {
            sleep(ms).await;
            o.borrow_mut().push(ms);
        })
        .detach();
    }
    let t0 = time();
    ex.block_on(sleep(40));
    let dt = time() - t0;
    t.check(*order.borrow() == [10, 20, 30], "async: sleeps end in deadline order");
    t.check(dt >= 40_000, "async: sleep(40) does not end early");

    task::clock_set(Some(1_000_000));
    let mut ui = Executor::ui();
    let h = ui.spawn(async {
        sleep(100).await;
        1
    });
    ui.run();
    t.check(task::next_timer() == Some(1_100_000), "async: a timer on the virtual clock");
    task::clock_set(Some(1_099_999));
    ui.run();
    t.check(!h.is_done(), "async: the virtual clock: not before the deadline");
    task::clock_set(Some(1_100_000));
    ui.run();
    t.check(h.take() == Some(1) && task::next_timer().is_none(), "async: the virtual clock fires the timer");
    task::clock_set(None);
    t.check(task::now_us().abs_diff(time()) < 1_000_000, "async: clock_set(None): back to time_us");
}

// ---------------------------------------------------------------- progress

fn progress(t: &mut T) {
    let p = Progress::new();
    t.check(p.permille() == 0, "async: Progress without a total: 0");
    p.set_total(200);
    p.set(50);
    t.check(p.permille() == 250 && p.done() == 50 && p.total() == 200, "async: Progress::permille");
    p.add(150);
    t.check(p.permille() == 1000, "async: Progress at the end");
    let ch = p.changed();
    let p2 = p.clone();
    let th = plat::thread_spawn(0, Box::new(move || {
        plat::sleep_ms(10);
        p2.add(1);
    }));
    block_on(ch);
    if let Some(th) = th {
        th.join();
    }
    t.check(p.done() == 201, "async: Progress::changed wakes after another thread's report");
}

// ---------------------------------------------------------------- files

fn pattern(n: usize) -> Vec<u8> {
    (0..n as u32).map(|i| (i.wrapping_mul(7) ^ (i >> 9)) as u8).collect()
}

// no "NAME.tmp-.." siblings in dir
fn no_siblings(dir: &str) -> bool {
    let mut ok = true;
    plat::dir_list(dir, &mut |n, _, _| ok &= !n.contains(".tmp-"));
    ok
}

fn files(t: &mut T) {
    let base = format!("{}/{}", plat::cwd().replace('\\', "/"), t.subp("aio"));
    plat::mkdir(&base);
    let data = pattern(1_000_000);
    let src = format!("{base}/src.bin");
    let ctl = Ctl::new();
    t.check(block_on(aio::write_file(&src, data.clone(), &ctl)) == Ok(()), "aio: write_file");
    t.check(plat::read_file(&src).as_deref() == Some(&data[..]), "aio: write_file wrote every byte");
    t.check(ctl.progress.done() == data.len() as u64 && ctl.progress.total() == data.len() as u64, "aio: write_file's progress");
    t.check(no_siblings(&base), "aio: write_file left no sibling");

    let ctl = Ctl { chunk: 100_000, ..Ctl::new() };
    let r = block_on(aio::read_file(&src, &ctl));
    t.check(r.as_ref().is_ok_and(|v| *v == data), "aio: read_file in chunks");
    t.check(ctl.progress.reports() >= 11 && ctl.progress.permille() == 1000, "aio: read_file reports every chunk");

    let dst = format!("{base}/dst.bin");
    plat::write_file(&dst, b"old");
    let ctl = Ctl::new();
    t.check(block_on(aio::copy_file(&src, &dst, &ctl)) == Ok(data.len() as u64), "aio: copy_file");
    t.check(plat::read_file(&dst).as_deref() == Some(&data[..]) && no_siblings(&base), "aio: copy_file replaced the target");

    let r = block_on(async {
        let mut rd = Reader::open(&src).await?;
        let mut w = Writer::create(&format!("{base}/chunked.bin")).await?;
        loop {
            let c = rd.read(65_536).await?;
            if c.is_empty() {
                break;
            }
            w.write(c).await?;
        }
        let n = w.written();
        w.commit().await?;
        Ok::<(u64, u64), IoError>((n, rd.pos()))
    });
    t.check(r == Ok((data.len() as u64, data.len() as u64)), "aio: Reader / Writer chunk by chunk");
    t.check(plat::read_file(&format!("{base}/chunked.bin")).as_deref() == Some(&data[..]), "aio: Writer::commit put the file in place");
    let gone = format!("{base}/dropped.bin");
    let r = block_on(async {
        let mut w = Writer::create(&gone).await?;
        w.write(vec![1, 2, 3]).await?;
        Ok::<(), IoError>(()) // w dropped without commit
    });
    t.check(r.is_ok() && plat::file_exists(&gone) == 0 && no_siblings(&base), "aio: a Writer dropped without commit leaves nothing");

    // mem:// and a folder mount
    let ctl = Ctl::new();
    t.check(block_on(aio::copy_file(&src, "mem://aio/m.bin", &ctl)) == Ok(data.len() as u64), "aio: copy into mem://");
    t.check(plat::read_file("mem://aio/m.bin").as_deref() == Some(&data[..]), "aio: mem:// has the copy");
    vfs::mount("aiotest", Arc::new(vfs::DirFs::new(&base)));
    t.check(block_on(aio::copy_file("mem://aio/m.bin", "aiotest://sub/d.bin", &ctl)) == Ok(data.len() as u64), "aio: copy into a folder mount");
    t.check(plat::read_file(&format!("{base}/sub/d.bin")).as_deref() == Some(&data[..]), "aio: the folder mount's OS file");
    t.check(no_siblings(&format!("{base}/sub")) && no_siblings("mem://aio"), "aio: the mounts keep no siblings");
    vfs::unmount("aiotest");
    plat::delete("mem://aio/m.bin");

    t.check(block_on(aio::read_file(&format!("{base}/missing.bin"), &Ctl::new())) == Err(IoError::Open), "aio: a missing source");
    t.check(block_on(aio::write_file(&base, vec![1], &Ctl::new())) == Err(IoError::Create), "aio: a folder is no target");
    let ctl = Ctl::new();
    ctl.cancel.cancel();
    let never = format!("{base}/never.bin");
    t.check(block_on(aio::write_file(&never, data.clone(), &ctl)) == Err(IoError::Cancelled), "aio: cancelled before the start");
    t.check(plat::file_exists(&never) == 0 && no_siblings(&base), "aio: ... nothing was created");

    // AtomicFile
    let p = format!("{base}/atomic.txt");
    plat::write_file(&p, b"before");
    match AtomicFile::create(&p) {
        Some(a) => {
            t.check(a.write_at(0, b"after!") == Some(6), "aio: AtomicFile::write_at");
            t.check(plat::read_file(&p).as_deref() == Some(&b"before"[..]), "aio: AtomicFile: the old file until commit");
        }
        None => t.check(false, "aio: AtomicFile::create"),
    }
    t.check(plat::read_file(&p).as_deref() == Some(&b"before"[..]) && no_siblings(&base), "aio: AtomicFile dropped without commit");
    let ok = AtomicFile::create(&p).is_some_and(|a| a.write_at(0, b"after") == Some(5) && a.commit());
    t.check(ok && plat::read_file(&p).as_deref() == Some(&b"after"[..]), "aio: AtomicFile::commit");
    t.check(AtomicFile::create(&base).is_none(), "aio: AtomicFile of a folder refused");
    #[cfg(target_os = "linux")]
    {
        // in temp_dir: the scratch tree may be on a file system without access bits (drvfs)
        let m = format!("{}/jfgui-aio-mode-{:x}.bin", plat::temp_dir().unwrap_or_default(), time() & 0xFF_FFFF);
        let made = plat::write_file(&m, b"x") && plat::fs::set_mode(&m, 0o640);
        let ok = made && block_on(aio::write_file(&m, b"mode".to_vec(), &Ctl::new())) == Ok(());
        let mode = plat::fs::stat(&m).map_or(0, |s| s.mode & 0o777);
        t.check(ok && mode == 0o640, "aio: (Linux) the replaced file keeps its access bits");
        plat::delete(&m);
    }
}

// ---------------------------------------------------------------- cancel between chunks

// a MemFs whose streams wait at the gate for reads and writes at offsets of `from` and beyond
// (once per operation: it is cancelled meanwhile)
struct Gated {
    inner: MemFs,
    gate: Arc<Gate>,
    from: u64,
}

// arrived counts the reads / writes that came to the gate; each waits for one post of sem
// (at most 30 s: a broken test fails instead of hanging)
struct Gate {
    sem: Sem,
    arrived: Progress,
}

impl Gate {
    fn pass(&self) {
        self.arrived.add(1);
        self.sem.wait(30_000);
    }

    // until more than n came to the gate
    async fn reached(&self, n: u64) {
        loop {
            let ch = self.arrived.changed(); // before the test: an arrival in between is not missed
            if self.arrived.done() > n {
                return;
            }
            ch.await;
        }
    }
}

struct GatedStream {
    s: Box<dyn Stream>,
    gate: Arc<Gate>,
    from: u64,
}

impl Stream for GatedStream {
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Option<usize> {
        if off >= self.from {
            self.gate.pass();
        }
        self.s.read_at(off, buf)
    }
    fn write_at(&self, off: u64, buf: &[u8]) -> Option<usize> {
        if off >= self.from {
            self.gate.pass();
        }
        self.s.write_at(off, buf)
    }
    fn size(&self) -> Option<u64> {
        self.s.size()
    }
    fn set_len(&self, len: u64) -> bool {
        self.s.set_len(len)
    }
    fn sync(&self) -> bool {
        self.s.sync()
    }
}

impl Provider for Gated {
    fn kind(&self, path: &str) -> u32 {
        self.inner.kind(path)
    }
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.inner.read(path)
    }
    fn list(&self, path: &str, f: &mut dyn FnMut(&str, bool, u64)) -> bool {
        self.inner.list(path, f)
    }
    fn write(&self, path: &str, data: &[u8]) -> bool {
        self.inner.write(path, data)
    }
    fn mkdir(&self, path: &str) -> bool {
        self.inner.mkdir(path)
    }
    fn delete(&self, path: &str) -> bool {
        self.inner.delete(path)
    }
    fn rename(&self, old: &str, new: &str) -> bool {
        self.inner.rename(old, new)
    }
    fn open(&self, path: &str, mode: u32) -> Opened {
        match self.inner.open(path, mode) {
            Opened::File(s) => Opened::File(Box::new(GatedStream { s, gate: self.gate.clone(), from: self.from })),
            o => o,
        }
    }
}

// op() with a watcher: once its job waits at the gate (in the second chunk, past the cancel check
// before it), cancel, then open the gate.  op is made after the gate's count is read: its job
// starts at once.  (Cancelling at the first progress report would race with that check.)
fn cancel_at_gate<F: Future>(op: impl FnOnce() -> F, ctl: &Ctl, gate: &Arc<Gate>) -> F::Output {
    let (c, g) = (ctl.clone(), gate.clone());
    let n = g.arrived.done();
    let op = op();
    let watch = async move {
        select(g.reached(n), sleep(30_000)).await;
        c.cancel.cancel();
        g.sem.post(1);
    };
    block_on(join(op, watch)).0
}

const CH: usize = 64 * 1024;

fn gated(t: &mut T) {
    let Some(sem) = Sem::new(0) else { return };
    let gate = Arc::new(Gate { sem, arrived: Progress::new() });
    let data = pattern(16 * CH);
    let g = Gated { inner: MemFs::new(), gate: gate.clone(), from: CH as u64 };
    g.inner.write("src.bin", &data);
    vfs::mount("gated", Arc::new(g));
    let dir = t.subp("aio");

    // copy: the second chunk's read waits; cancelled meanwhile, the copy stops after it
    let dst = format!("{dir}/gated-dst.bin");
    plat::write_file(&dst, b"old");
    let ctl = Ctl { chunk: CH, ..Ctl::new() };
    let r = cancel_at_gate(|| aio::copy_file("gated://src.bin", &dst, &ctl), &ctl, &gate);
    t.check(r == Err(IoError::Cancelled), "aio: copy_file cancelled between chunks");
    t.check(ctl.progress.done() == 2 * CH as u64, "aio: ... after the chunk that was under way");
    t.check(plat::read_file(&dst).as_deref() == Some(&b"old"[..]) && no_siblings(&dir), "aio: ... the target unchanged, the sibling removed");

    // write into the gated mount: its sibling there is removed too
    let ctl = Ctl { chunk: CH, ..Ctl::new() };
    let r = cancel_at_gate(|| aio::write_file("gated://out.bin", data.clone(), &ctl), &ctl, &gate);
    let ok = r == Err(IoError::Cancelled) && ctl.progress.done() == 2 * CH as u64;
    t.check(ok && plat::file_exists("gated://out.bin") == 0, "aio: write_file cancelled between chunks");
    t.check(no_siblings("gated://"), "aio: ... the mount's sibling removed");

    // read
    let ctl = Ctl { chunk: CH, ..Ctl::new() };
    let r = cancel_at_gate(|| aio::read_file("gated://src.bin", &ctl), &ctl, &gate);
    t.check(r == Err(IoError::Cancelled) && ctl.progress.done() == 2 * CH as u64, "aio: read_file cancelled between chunks");

    // the future dropped while its job waits: the job stops, cleans up
    let ctl = Ctl { chunk: CH, ..Ctl::new() };
    let mut ex = Executor::new();
    let n = gate.arrived.done();
    let h = ex.spawn(aio::copy_file("gated://src.bin", &dst, &ctl));
    ex.run();
    let dl = time() + 30_000_000;
    while gate.arrived.done() == n && time() < dl {
        plat::sleep_ms(1);
    }
    drop(h);
    ex.run(); // drops the future: the job's token is cancelled
    gate.sem.post(1);
    t.check(pool_idle() && ex.live() == 0 && ctl.progress.done() == 2 * CH as u64, "aio: a dropped copy future ends its job");
    t.check(plat::read_file(&dst).as_deref() == Some(&b"old"[..]) && no_siblings(&dir), "aio: ... target unchanged, no sibling");
    t.check(!ctl.cancel.is_cancelled(), "aio: ... without the caller's token");
    t.check(gate.arrived.done() == 4 && !gate.sem.wait(0), "aio: each job waited at the gate once and took its post");
    while gate.sem.wait(0) {}
    vfs::unmount("gated");
}

// ---------------------------------------------------------------- children

// run plattest with args -> (exit code, stdout + stderr)
fn child(args: &[&str]) -> (Option<i32>, String) {
    let exe = plat::current_exe().unwrap_or_default();
    let cmd = plat::process::Command { program: &exe, args, cwd: None, env: &[] };
    let Ok(mut c) = cmd.spawn() else { return (None, String::new()) };
    let mut out: Vec<u8> = Vec::new();
    let mut buf = [0u8; 4096];
    let mut code = None;
    for _ in 0..3000 {
        while let Ok(Some(n)) = c.read_stderr(&mut buf) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        while let Ok(Some(n)) = c.read_stdout(&mut buf) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        if code.is_some() {
            break;
        }
        code = c.try_wait().ok().flatten();
        if code.is_none() {
            plat::sleep_ms(10);
        }
    }
    (code, String::from_utf8_lossy(&out).into_owned())
}

fn children(t: &mut T) {
    let (code, out) = child(&["--test", "async-panic"]);
    t.check(code == Some(3) && out.contains("panic: ") && out.contains("task panic test"), "async: a panic in a task: exit code 3");
    let (code, out) = child(&["--test", "async-panic-job"]);
    t.check(code == Some(3) && out.contains("job panic test"), "async: a panic in a blocking job: exit code 3");
    let base = format!("{}/{}", plat::cwd().replace('\\', "/"), t.subp("policy"));
    let ok = plat::mkdir(&format!("{base}/rw")) && plat::mkdir(&format!("{base}/no")) && plat::write_file(&format!("{base}/no/secret.bin"), b"secret");
    t.check(ok, "async: the permission test's folders");
    let (code, out) = child(&["--test", "async-policy", "--base", &base]);
    t.check(code == Some(0), "async: permissions apply to aio (the child)");
    if code != Some(0) {
        crate::out(&out);
    }
    let mut names = Vec::new();
    plat::dir_list(&format!("{base}/no"), &mut |n, _, _| names.push(String::from(n)));
    t.check(names == ["secret.bin"], "async: the refused writes left nothing behind");
}

/// (the child) a panic inside a task
pub fn t_async_panic(_: &mut T) {
    let mut ex = Executor::new();
    ex.spawn(async {
        yield_now().await;
        panic!("task panic test");
    })
    .detach();
    while ex.has_ready() {
        ex.run();
    }
}

/// (the child) a panic inside a blocking job
pub fn t_async_panic_job(_: &mut T) {
    block_on(spawn_blocking(|| -> u32 { panic!("job panic test") }));
}

/// (the child) a manifest allows base/rw only: aio follows it, trusted() callers stay trusted
pub fn t_async_policy(t: &mut T) {
    let Some(base) = plat::arg_value("--base") else {
        t.check(false, "async-policy: --base");
        return;
    };
    let key = match plat::policy::install(&format!("fs.write {base}/rw/**\n")) {
        Ok(k) => k,
        Err(e) => {
            t.check(false, &format!("async-policy: install ({e})"));
            return;
        }
    };
    let ok = format!("{base}/rw/a.bin");
    t.check(block_on(aio::write_file(&ok, vec![7; 1000], &Ctl::new())) == Ok(()), "async-policy: aio writes where the manifest allows");
    let no = format!("{base}/no/b.bin");
    t.check(block_on(aio::write_file(&no, vec![7; 1000], &Ctl::new())) == Err(IoError::Create), "async-policy: a refused write fails");
    let secret = format!("{base}/no/secret.bin");
    t.check(block_on(aio::read_file(&secret, &Ctl::new())) == Err(IoError::Open), "async-policy: a refused read fails");
    t.check(block_on(aio::copy_file(&secret, &ok, &Ctl::new())) == Err(IoError::Open), "async-policy: a refused copy source");
    t.check(block_on(aio::copy_file(&ok, &no, &Ctl::new())) == Err(IoError::Create), "async-policy: a refused copy target");
    let f = key.trusted(|| aio::read_file(&secret, &Ctl::new()));
    t.check(block_on(f).as_deref() == Ok(&b"secret"[..]), "async-policy: a trusted() caller stays trusted on the pool");
}
