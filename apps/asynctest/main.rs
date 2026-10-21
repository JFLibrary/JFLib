// asynctest -- async tasks in the event loop: ui.spawn with plat::task and plat::aio.  A copy
// with a progress bar and a cancel button, a fast copy, a timer, a file read with a check sum.
//
//   python build_rs.py run asynctest
//   build/win/asynctest.exe --script apps/asynctest/tests/copy.uis
//
//   "Kopieren"          copies SRC_LEN bytes chunk by chunk (aio::Reader / Writer, CHUNK each,
//                       every chunk a job of the blocking pool) and sleeps SLOW_MS of the task
//                       clock between the chunks.  The progress bar counts the chunks: the
//                       counter is an Rc<Cell> the task shares with the app (UI tasks need not
//                       be Send).  Scripts step through it: `sync` waits for the pool's jobs,
//                       `wait 100` moves the virtual clock past the sleep.
//   "Abbrechen"         Task::cancel: the copy's future is dropped at the next run, its Writer
//                       with it, so the unfinished sibling of the target is removed
//   "Schnell kopieren"  aio::copy_file: one pool job, Progress (bar in per mille) and CancelToken
//   "Warten"            sleep(500) on the task clock
//   "Laden"             aio::read_file, then a check sum in spawn_blocking
//   --selftest          (a real window) starts all of them at once and quits when every one is
//                       done: "asynctest selftest: ok" on stdout, else the states (the event
//                       loop's E_WAKE and its task-timer deadlines with the real clock)
// The app never handles UIC_WAKE: App::frame looks at its Task handles (take, is_done).  The
// files live in a folder of temp_dir, removed when the app is dropped.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::Cell;

use plat::aio::{self, Ctl, IoError, Reader, Writer};
use plat::task::{sleep, spawn_blocking, Task};
use ui::*;

const SRC_LEN: usize = 256 * 1024;
const CHUNK: usize = 64 * 1024;
const CHUNKS: i32 = (SRC_LEN / CHUNK) as i32;
const SLOW_MS: u32 = 100;
const TIMER_MS: u32 = 500;

const ID_BAR: u32 = 300;
const ID_FAST_BAR: u32 = 301;
const ID_COPY: u32 = 310;
const ID_CANCEL: u32 = 311;
const ID_FAST: u32 = 312;
const ID_WAIT: u32 = 313;
const ID_LOAD: u32 = 314;

const DESC: AppDesc = AppDesc { title: "Async-Test", flags: AF_RESIZABLE, w: 560, h: 520, min_w: 420, min_h: 360, ..AppDesc::NONE };

struct AsyncTest {
    dir: String,
    src: String,
    dst: String,
    dst_fast: String,
    copy: Option<Task<Result<u64, IoError>>>,
    chunks: Rc<Cell<u32>>,
    status: String,
    fast: Option<Task<Result<u64, IoError>>>,
    fast_ctl: Ctl,
    fast_status: String,
    timer: Option<Task<()>>,
    timer_status: &'static str,
    load: Option<Task<Result<u32, IoError>>>,
    load_status: String,
    selftest: Option<u64>,
}

// the source's bytes (the same on every run)
fn pattern() -> Vec<u8> {
    (0..SRC_LEN as u32).map(|i| (i.wrapping_mul(31) ^ (i >> 8)) as u8).collect()
}

// Adler-32 (the check sum of "Laden")
fn adler(d: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in d {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    b << 16 | a
}

// the slow copy: chunk by chunk on the pool, SLOW_MS of the task clock between the chunks
async fn copy_slow(src: String, dst: String, chunks: Rc<Cell<u32>>) -> Result<u64, IoError> {
    let mut rd = Reader::open(&src).await?;
    let mut w = Writer::create(&dst).await?;
    loop {
        let c = rd.read(CHUNK).await?;
        if c.is_empty() {
            break;
        }
        w.write(c).await?;
        chunks.set(chunks.get() + 1);
        if rd.pos() < rd.size() {
            sleep(SLOW_MS).await;
        }
    }
    let n = w.written();
    w.commit().await?;
    Ok(n)
}

impl AsyncTest {
    fn new() -> AsyncTest {
        let dir = format!("{}/jfgui-asynctest-{:x}", plat::temp_dir().unwrap_or_default(), plat::time_us() & 0xFFF_FFFF);
        AsyncTest {
            src: format!("{dir}/source.bin"),
            dst: format!("{dir}/copy.bin"),
            dst_fast: format!("{dir}/fast.bin"),
            dir,
            copy: None,
            chunks: Rc::new(Cell::new(0)),
            status: String::from("Status: bereit"),
            fast: None,
            fast_ctl: Ctl::new(),
            fast_status: String::from("Schnell: bereit"),
            timer: None,
            timer_status: "Timer: aus",
            load: None,
            load_status: String::from("Prüfsumme: -"),
            selftest: None,
        }
    }

    fn start_copy(&mut self, ui: &Ui) {
        if self.copy.is_none() {
            self.chunks.set(0);
            self.status = String::from("Status: kopiert …");
            self.copy = Some(ui.spawn(copy_slow(self.src.clone(), self.dst.clone(), self.chunks.clone())));
        }
    }

    fn start_fast(&mut self, ui: &Ui) {
        if self.fast.is_none() {
            self.fast_ctl = Ctl::new();
            self.fast_status = String::from("Schnell: kopiert …");
            self.fast = Some(ui.spawn(aio::copy_file(&self.src, &self.dst_fast, &self.fast_ctl)));
        }
    }

    fn start_timer(&mut self, ui: &Ui) {
        if self.timer.is_none() {
            self.timer_status = "Timer: läuft";
            self.timer = Some(ui.spawn(sleep(TIMER_MS)));
        }
    }

    fn start_load(&mut self, ui: &Ui) {
        if self.load.is_none() {
            self.load_status = String::from("Prüfsumme: lädt …");
            let src = self.src.clone();
            self.load = Some(ui.spawn(async move {
                let d = aio::read_file(&src, &Ctl::new()).await?;
                Ok(spawn_blocking(move || adler(&d)).await)
            }));
        }
    }

    // --selftest: every task finished -> print the result, quit
    fn selftest_end(&mut self, ui: &mut Ui) {
        let Some(t0) = self.selftest else { return };
        if self.copy.is_some() || self.fast.is_some() || self.timer.is_some() || self.load.is_some() {
            return;
        }
        let ms = (plat::time_us() - t0) / 1000;
        let ok = self.status == "Status: fertig (262144 Bytes)" && self.fast_status == "Schnell: fertig (262144 Bytes)"
            && self.timer_status == "Timer: abgelaufen" && self.load_status == "Prüfsumme: 8fd61de3" && ms >= TIMER_MS as u64;
        let s = if ok {
            format!("asynctest selftest: ok ({ms} ms)\n")
        } else {
            format!("asynctest selftest: FAIL ({ms} ms): {} / {} / {} / {}\n", self.status, self.fast_status, self.timer_status, self.load_status)
        };
        plat::write_stdout(s.as_bytes());
        self.selftest = None;
        ui.quit();
    }

    // the results of finished tasks (App::frame polls the handles)
    fn poll_tasks(&mut self) {
        if let Some(t) = self.copy.as_ref().filter(|t| t.is_done()) {
            self.status = match t.take() {
                Some(Ok(n)) => format!("Status: fertig ({n} Bytes)"),
                Some(Err(e)) => format!("Status: Fehler ({e})"),
                None => String::from("Status: abgebrochen"),
            };
            self.copy = None;
        }
        if let Some(t) = self.fast.as_ref().filter(|t| t.is_done()) {
            self.fast_status = match t.take() {
                Some(Ok(n)) => format!("Schnell: fertig ({n} Bytes)"),
                Some(Err(e)) => format!("Schnell: Fehler ({e})"),
                None => String::from("Schnell: abgebrochen"),
            };
            self.fast = None;
        }
        if self.timer.as_ref().is_some_and(|t| t.is_done()) {
            self.timer = None;
            self.timer_status = "Timer: abgelaufen";
        }
        if let Some(t) = self.load.as_ref().filter(|t| t.is_done()) {
            self.load_status = match t.take() {
                Some(Ok(sum)) => format!("Prüfsumme: {sum:08x}"),
                Some(Err(e)) => format!("Prüfsumme: Fehler ({e})"),
                None => String::from("Prüfsumme: abgebrochen"),
            };
            self.load = None;
        }
    }

    // the target folder as the scripts check it
    fn files(&self) -> (bool, u32, bool) {
        let mut tmp = 0;
        plat::dir_list(&self.dir, &mut |n, _, _| tmp += n.contains(".tmp-") as u32);
        let dst = plat::read_file(&self.dst);
        let same = dst.as_deref().is_some_and(|d| d == &pattern()[..]);
        (dst.is_some(), tmp, same)
    }
}

impl App for AsyncTest {
    fn init(&mut self, ui: &mut Ui) -> bool {
        if !(plat::mkdir(&self.dir) && plat::write_file(&self.src, &pattern())) {
            return false;
        }
        if plat::args().iter().any(|a| a == "--selftest") {
            self.selftest = Some(plat::time_us());
            self.start_copy(ui);
            self.start_fast(ui);
            self.start_timer(ui);
            self.start_load(ui);
        }
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        self.poll_tasks();
        self.selftest_end(ui);
        let (exists, tmp, same) = self.files();
        ui.box_begin(0, BOX_V | pad(20), 10);
        ui.text(TS_TITLE, "Asynchrone Aufgaben");
        ui.text(TS_BODY | TX_SECONDARY, &format!("Quelle: {SRC_LEN} Bytes in {CHUNKS} Stücken"));

        ui.progress(ID_BAR, self.chunks.get() as i32, CHUNKS);
        ui.text(TS_BODY, &self.status);
        ui.text(TS_BODY, if exists { "Ziel vorhanden: ja" } else { "Ziel vorhanden: nein" });
        ui.text(TS_BODY, &format!("Temporäre Dateien: {tmp}"));
        ui.text(TS_BODY, if same { "Inhalt gleich: ja" } else { "Inhalt gleich: nein" });
        ui.box_begin(0, BOX_H, 8);
        if ui.accent_button(ID_COPY, "Kopieren") & EV_ACT != 0 {
            self.start_copy(ui);
        }
        if ui.button(ID_CANCEL, "Abbrechen") & EV_ACT != 0 {
            if let Some(t) = &self.copy {
                t.cancel(); // the future goes at the next run; poll_tasks reports it
            }
        }
        ui.box_end();

        ui.separator();
        ui.progress(ID_FAST_BAR, self.fast_ctl.progress.permille() as i32, 1000);
        ui.text(TS_BODY, &self.fast_status);
        ui.box_begin(0, BOX_H, 8);
        if ui.button(ID_FAST, "Schnell kopieren") & EV_ACT != 0 {
            self.start_fast(ui);
        }
        if ui.button(ID_WAIT, "Warten") & EV_ACT != 0 {
            self.start_timer(ui);
        }
        if ui.button(ID_LOAD, "Laden") & EV_ACT != 0 {
            self.start_load(ui);
        }
        ui.box_end();
        ui.text(TS_BODY, self.timer_status);
        ui.text(TS_BODY, &self.load_status);
        ui.text(TS_BODY | TX_SECONDARY, &format!("Aufgaben aktiv: {}", ui.tasks_live()));
        ui.box_end();
    }

}

// the files go with the app (after the Ui and its tasks: launch drops the app last)
impl Drop for AsyncTest {
    fn drop(&mut self) {
        let mut names = Vec::new();
        plat::dir_list(&self.dir, &mut |n, _, _| names.push(format!("{}/{n}", self.dir)));
        for p in names {
            plat::delete(&p);
        }
        plat::delete(&self.dir);
    }
}

ui::main!(ui::launch(&DESC, AsyncTest::new(), &style::STYLE));
