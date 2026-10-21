// ops -- long actions as operations (202 + polling): each runs on its own thread, at most
// MAX_RUNNING at once (the others wait queued), cancellation is cooperative (the work polls
// OpCtl::cancelled).  Progress lives in memory; the record in the database is written when the
// operation is queued, starts and ends.  Operations that were running when the server stopped
// are marked interrupted at the next start.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use bc_proto::j::Json;
use bc_proto::model::{OpStatus, Operation};
use plat::sync::{Condvar, Mutex};

use crate::ctx::{Ctx, C_OPS};
use crate::util::{new_id, now};

pub const MAX_RUNNING: u32 = 8;

struct Live {
    op: Operation,
    cancel: Arc<AtomicBool>,
}

pub struct Ops {
    live: Mutex<BTreeMap<String, Live>>,
    running: Mutex<u32>,
    slot: Condvar,
}

/// what an operation's work sees
pub struct OpCtl {
    pub id: String,
    cancel: Arc<AtomicBool>,
    ctx: Arc<Ctx>,
}

impl OpCtl {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn progress(&self, done: u64, total: u64) {
        if let Some(l) = self.ctx.ops.live.lock().get_mut(&self.id) {
            l.op.done = done;
            l.op.total = total;
        }
    }

    pub fn ctx(&self) -> &Arc<Ctx> {
        &self.ctx
    }
}

pub type Work = Box<dyn FnOnce(&OpCtl) -> Result<String, String> + Send>;

impl Default for Ops {
    fn default() -> Self {
        Ops::new()
    }
}

impl Ops {
    pub fn new() -> Ops {
        Ops { live: Mutex::new(BTreeMap::new()), running: Mutex::new(0), slot: Condvar::new() }
    }

    /// mark the operations of a previous run that never finished
    pub fn interrupt_stale(db: &datastore::Store) {
        let t = now();
        for (id, v) in db.list(C_OPS) {
            if let Some(mut op) = Operation::from_json(&v) {
                if !op.status.finished() {
                    op.status = OpStatus::Interrupted;
                    op.finished = t;
                    op.error = String::from("Der Server wurde beendet, während der Vorgang lief");
                    let _ = db.put(C_OPS, &id, op.to_json());
                }
            }
        }
    }

    /// start an operation -> its id
    #[allow(clippy::too_many_arguments)]
    pub fn start(ctx: &Arc<Ctx>, kind: &str, title: &str, owner: &str, target_id: &str, session_id: &str, work: Work) -> String {
        let id = new_id();
        let t = now();
        let op = Operation {
            id: id.clone(),
            kind: String::from(kind),
            title: String::from(title),
            owner: String::from(owner),
            status: OpStatus::Queued,
            created: t,
            target_id: String::from(target_id),
            session_id: String::from(session_id),
            ..Operation::default()
        };
        let _ = ctx.db.put(C_OPS, &id, op.to_json());
        let cancel = Arc::new(AtomicBool::new(false));
        ctx.ops.live.lock().insert(id.clone(), Live { op, cancel: cancel.clone() });
        let c = OpCtl { id: id.clone(), cancel, ctx: ctx.clone() };
        let spawned = plat::thread_spawn(1 << 20, Box::new(move || run(c, work)));
        if spawned.is_none() {
            ctx.ops.finish(&ctx.db, &id, Err(String::from("Kein Thread für den Vorgang verfügbar")), false);
        }
        id
    }

    fn finish(&self, db: &datastore::Store, id: &str, r: Result<String, String>, cancelled: bool) {
        let Some(l) = self.live.lock().remove(id) else { return };
        let mut op = l.op;
        op.finished = now();
        match r {
            _ if cancelled => {
                op.status = OpStatus::Cancelled;
                op.error = String::from("Abgebrochen");
            }
            Ok(s) => {
                op.status = OpStatus::Succeeded;
                op.result = s;
                if op.total > 0 {
                    op.done = op.total;
                }
            }
            Err(e) => {
                op.status = OpStatus::Failed;
                op.error = e;
            }
        }
        let _ = db.put(C_OPS, id, op.to_json());
    }

    /// the operation (live progress or the stored record)
    pub fn get(&self, db: &datastore::Store, id: &str) -> Option<Operation> {
        if let Some(l) = self.live.lock().get(id) {
            return Some(l.op.clone());
        }
        db.get(C_OPS, id).as_ref().and_then(Operation::from_json)
    }

    /// recent operations, newest first (running ones with their progress)
    pub fn list(&self, db: &datastore::Store, limit: usize) -> Vec<Operation> {
        let live = self.live.lock();
        let mut v: Vec<Operation> = db.list(C_OPS).iter().filter_map(|(id, x)| live.get(id).map(|l| l.op.clone()).or_else(|| Operation::from_json(x))).collect();
        drop(live);
        v.sort_by(|a, b| b.created.cmp(&a.created));
        v.truncate(limit);
        v
    }

    /// ask an operation to stop -> its record (None: unknown or already finished)
    pub fn cancel(&self, id: &str) -> Option<Operation> {
        let mut live = self.live.lock();
        let l = live.get_mut(id)?;
        l.cancel.store(true, Ordering::Relaxed);
        if !l.op.status.finished() {
            l.op.status = OpStatus::Cancelling;
        }
        Some(l.op.clone())
    }

    pub fn running(&self) -> u32 {
        *self.running.lock()
    }

    /// whether an unfinished operation targets this id (a job that runs already)
    pub fn busy(&self, target_id: &str, kind: &str) -> bool {
        self.live.lock().values().any(|l| l.op.target_id == target_id && l.op.kind == kind)
    }

    /// prune finished operations older than `days`
    pub fn prune(db: &datastore::Store, days: i64) {
        let limit = now() - days * 86_400;
        let old: Vec<String> = db.filter(C_OPS, |_, v| v.get("finished").and_then(|x| x.as_i64()).is_some_and(|f| f > 0 && f < limit)).into_iter().map(|(k, _)| k).collect();
        let _ = db.write(|_, tx| {
            for k in &old {
                tx.delete(C_OPS, k);
            }
            Ok::<(), datastore::StoreError>(())
        });
    }
}

fn run(c: OpCtl, work: Work) {
    let ops = &c.ctx.ops;
    // wait for a slot
    {
        let mut n = ops.slot.wait_while(ops.running.lock(), |n| *n >= MAX_RUNNING);
        *n += 1;
    }
    let t = now();
    let started = {
        let mut live = ops.live.lock();
        live.get_mut(&c.id).map(|l| {
            l.op.status = if c.cancelled() { OpStatus::Cancelling } else { OpStatus::Running };
            l.op.started = t;
            l.op.clone()
        })
    };
    if let Some(op) = started {
        let _ = c.ctx.db.put(C_OPS, &c.id, op.to_json());
    }
    let r = if c.cancelled() { Err(String::new()) } else { work(&c) };
    let cancelled = c.cancelled();
    ops.finish(&c.ctx.db, &c.id, r, cancelled);
    *ops.running.lock() -= 1;
    ops.slot.notify_one();
}
