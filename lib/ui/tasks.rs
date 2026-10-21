// tasks -- async tasks on the UI thread (plat::task): spawned by App code, polled between
// passes; a task that ran gets the app another pass, so App::frame sees what it changed
//
//   ui.spawn(fut) -> Task<T>    fut runs on the UI thread (it need not be Send); inside it the
//                               blocking work goes to the pool (plat::task::spawn_blocking,
//                               plat::aio, netio's async calls), sleep() waits for the task
//                               clock.  App::frame polls the handle: task.take() (the result,
//                               once), is_done(), cancel(); dropping the handle cancels it
//   ui.spawner() -> Spawner     spawn from inside tasks (a clone moves into them)
//   ui.tasks_live() -> n        tasks not finished yet
//
// Where they run: run_passes polls the woken tasks after the commands of each round (a poll
// counts as a command: another round, at most the round limit; past it the next loop turn
// builds the frame).  Wake-ups from other threads (pool jobs, Progress reports, channels) come
// as E_WAKE (plat::wake) and are taken there.  The window loop waits no longer than the next
// task timer and does not wait while tasks are ready; the browser / Android host gets the same
// as its frame delay.  Headless runs put the task clock on the virtual clock (`wait` fires
// sleep()s), and the scripts' `sync` waits until no task work is in flight (blocking jobs,
// wake-ups not taken yet) -- so scripts with async work stay deterministic.

use core::future::Future;

use plat::task::{Spawner, Task};

use super::Ui;

impl Ui {
    /// run fut on the UI thread between passes; App::frame polls the handle (dropping it
    /// cancels the task, Task::detach lets it run on)
    pub fn spawn<F: Future + 'static>(&self, fut: F) -> Task<F::Output> {
        self.tasks.spawn(fut)
    }

    /// a handle that spawns UI tasks from inside tasks
    pub fn spawner(&self) -> Spawner {
        self.tasks.spawner()
    }

    /// tasks spawned and not finished
    pub fn tasks_live(&self) -> usize {
        self.tasks.live()
    }

    /// (runtime) poll the woken tasks once -> any ran (the frame may change)
    pub(crate) fn tasks_pump(&mut self) -> bool {
        self.tasks.run() > 0
    }

    /// (E_WAKE) take the wake-ups other threads sent
    pub(crate) fn tasks_wake(&mut self) {
        self.tasks.accept_remote();
    }

    /// tasks to poll now: woken, new, or a timer of theirs is due
    pub(crate) fn tasks_ready(&self) -> bool {
        self.tasks.has_ready() || plat::task::next_timer().is_some_and(|t| t <= plat::task::now_us())
    }

    /// task work in flight (the scripts' `sync`)
    pub(crate) fn tasks_busy(&self) -> bool {
        self.tasks_ready() || self.tasks.busy()
    }

    /// the next task timer on the task clock (window mode: plat::time_us)
    pub(crate) fn tasks_deadline(&self) -> Option<u64> {
        plat::task::next_timer()
    }
}
