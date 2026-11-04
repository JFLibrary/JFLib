// db -- the database service of the lab: SQL-shaped statements over modelled tables, with a buffer pool, row
// locks, versions, a log on the real disk, group commit and recovery.  It speaks HTTP, is no SQL engine and
// no real product's wire protocol.  The statements are dbsql.rs, what it stores dbstore.rs
//
//   kind  config (JSON; every member but "tables" may be left out)
//   db    {"port": 7100, "tables": [{"name": "users", "rows": 1000000, "row_bytes": 200, "key": "id",
//          "dist": "uniform", "zipf_s": 1.0, "columns": ["email:text", {"name": "balance", "type": "int",
//          "distinct": 1000}], "indexes": ["email"]}], "seed": 1, "pool_mb": 128, "page_kb": 8, "fanout": 0,
//          "max_conns": 100, "durability": "full", "flush_ms": 200, "isolation": "read committed",
//          "lock_timeout_ms": 5000, "txn_idle_ms": 30000, "checkpoint_s": 30, "touched_max": 100000,
//          "stmt_us": 30, "page_us": 2, "row_us": 0.5, "write_us": 10, "redo_mbps": 200, "max_rows": 1000,
//          "write_rows_max": 10000, "wal_tail": 1024, "mem_mb": 64, "conn_kb": 256, "disk": {...}}
//   a column in short: "name" (a number per key), "name:text", "name:int:1000" (1000 different numbers)
//
//   HTTP   POST /query    the body: the statement, or JSON {"sql": "..", "params": [1, "x", {"draw": true}],
//                         "txn": 7}; "txn" also as ?txn=7 or the header X-Txn.  {"draw": true}: a key by the
//                         table's distribution, the answer says which ("drawn")
//          GET /_db/health  state, role and log positions (the seam of replication)
//          GET /_db/stats   the numbers and the tables
//   answer JSON: {"ok": true, "rows": [{..}], "count": 1, "txn": 7, "drawn": 5, "truncated": false, "figures":
//          {"rows_examined": 1, "pages_hit": 3, "pages_missed": 1, "lock_wait_us": 0, "us": 180}}, an error
//          {"ok": false, "code": "syntax", "error": "Zeichen 8: ..", "pos": 8, "retryable": false}
//   status 200; 400 syntax and what the schema does not have; 404 no such transaction; 409 conflict (deadlock,
//          serialization, lock_timeout, duplicate_key, txn_busy), retryable but for the duplicate; 503 overload
//          (too_many_connections, recovering); 507 the disk is full
//
//   Db::view / Engine::db_view        what the window and dbctl show
//   Db::wal_since(lsn), lsn(), flushed_lsn()   log positions and a bounded tail of records: what a replica
//                                     will fetch and apply (SA5 builds on it; there is no replication here)
//   Db::tables(), Db::explain(sql)    for dbctl and the tests
//   Db::rows_of(...)                  a table's values as of now (the tests)
//
// One statement.  Its parsing costs stmt_us of CPU.  What it reads it reads from its snapshot when it starts,
// so the answer is known at once and only the time still has to pass: pages through the pool (a miss is a read
// on the real disk, one by one when scattered, in pieces of 64 when neighbours), page_us per page and row_us
// per row in pieces of res::QUANTUM.  No event per page or row.  A write then takes its row locks in key order,
// re-reads under each lock what it needs and writes (write_us per row).
//
// Versions (dbstore.rs).  Readers never block: a snapshot is a commit stamp.  "read committed" takes one per
// statement, "repeatable read" one per transaction.  A write that waited for a lock goes on with the newest
// committed values under read committed (so a counter loses no update) and fails with "serialization" under
// repeatable read when the row changed after its snapshot.  Versions no snapshot needs are dropped.
//
// Locks.  One exclusive lock per row, also for rows that are only derived; FIFO queues; lock_timeout_ms; the
// wait-for graph is walked when a transaction would wait, and the one that closes a cycle is the victim
// (409 deadlock, retryable).  An explicit transaction lives on the id BEGIN gave, and goes after txn_idle_ms
// without a statement; what it held is released.  Service::lock_busy is the time some row lock had waiters.
//
// The log.  Positions in bytes (LSN); a commit is durable when the disk has it.  "durability": "full": the
// answer waits for the flush, and commits that come during a flush share the next one (group commit).
// "relaxed": the answer comes at once and a flush runs every flush_ms; a crash loses what was answered since
// the last flush, and counts it (lost_commits).  Locks go when a commit is final.  A checkpoint (every
// checkpoint_s while something changed) writes the dirty pages and frees the log before it.
//
// A stop counts as a kill: power off, kill and the OOM killer all end it at once; there is no clean shutdown.
// What the log did not keep goes, open transactions too, the pool is empty.  The next start takes the time of
// reading and replaying the log since the last checkpoint ("recovery_ms"); until then statements get 503.
//
// Costs are assumptions of this lab (the README lists them): stmt_us, page_us, row_us, write_us and the
// page size.  Without hardware on the device CPU costs nothing and the status says so; the disk and locks
// still take their time.
//
// Metrics: statements, selects, inserts, updates, deletes, commits, rollbacks, errors, syntax_errors, conflicts,
// deadlocks, lock_waits, lock_timeouts, idle_aborts, refused, requests, active, inflight, txns, p50_us, p95_us,
// p99_us, pool_hits, pool_misses, pool_hit_pm, pool_use_pm, pool_bytes, dirty_pages, rows_examined, pages_read,
// wal_flushes, group_pm (commits per flush, thousandths), wal_bytes, lsn, flushed_lsn, lost_commits, recoveries,
// recovery_ms, touched_rows, forgotten, old_versions.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value;
use base::metrics::Window;
use base::rng::Pcg32;
use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::http::Headers;

use crate::dbsql::{self, Cmp, Cond, Iso, Join, Lit, Select, Set, SetTo, SqlErr, Stmt, What};
use crate::dbstore::{self, best_access, column, cost_of, walk, Access, ColDef, Cost, Dist, Layout, Pool, RCond, Snap, Table, TableSpec, Touch, Val};
use crate::http::{Reply, ReqIn, Serve, ServeCfg, Wheel};
use crate::httpsvc::decimal;
use crate::kernel::{Engine, Ns, MS, SEC, US};
use crate::os::{Metric, OsApi, Service, SvcEvent};
use crate::series::{metric_kind_of, LatWindow, MetricKind, LAT};

/// timer keys: its own deadlines, the server side
const K_OWN: u32 = 0;
const K_SERVE: u32 = 1;
/// the default port
pub const DB_PORT: u16 = 7100;
/// pages read at once: scattered ones keep their latency each, neighbours come together
const RAND_BATCH: u64 = 16;
const SEQ_BATCH: u64 = 64;
/// the piece of a checkpoint
const CKPT_CHUNK: u64 = 1 << 20;
/// the log's record of a row: its image and a head
const REC_HEAD: u64 = 64;
/// statements it keeps parsed
const CACHE_MAX: usize = 256;

#[derive(Clone, Copy, Debug)]
enum DTimer {
    /// a statement's next step
    Step(u64),
    /// a lock wait of a statement ran out
    LockOut(u64),
    /// an explicit transaction was silent too long
    Idle(u64),
    FlushDone,
    /// the next flush of "relaxed"
    FlushTick,
    Checkpoint,
    CkptStep,
    Recovered,
    /// an index is built
    Built(u64),
}

/// its settings
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbCfg {
    pub port: u16,
    pub seed: u64,
    pub pool_bytes: u64,
    pub layout: Layout,
    pub max_conns: u32,
    pub durable: bool,
    pub flush: Ns,
    pub iso: Iso,
    pub lock_timeout: Ns,
    pub txn_idle: Ns,
    pub checkpoint: Ns,
    pub touched_max: u64,
    pub stmt_cpu: Ns,
    pub page_cpu: Ns,
    pub row_cpu: Ns,
    pub write_cpu: Ns,
    pub redo_bps: u64,
    pub max_rows: u64,
    pub write_rows_max: u64,
    pub wal_tail: usize,
}

impl Default for DbCfg {
    fn default() -> DbCfg {
        DbCfg {
            port: DB_PORT,
            seed: 1,
            pool_bytes: 128 << 20,
            layout: Layout::new(8192, 0),
            max_conns: 100,
            durable: true,
            flush: 200 * MS,
            iso: Iso::ReadCommitted,
            lock_timeout: 5 * SEC,
            txn_idle: 30 * SEC,
            checkpoint: 30 * SEC,
            touched_max: 100_000,
            stmt_cpu: 30 * US,
            page_cpu: 2 * US,
            row_cpu: 500,
            write_cpu: 10 * US,
            redo_bps: 200_000_000,
            max_rows: 1000,
            write_rows_max: 10_000,
            wal_tail: 1024,
        }
    }
}

/// what it counted
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DbStats {
    pub requests: u64,
    pub statements: u64,
    pub selects: u64,
    pub inserts: u64,
    pub updates: u64,
    pub deletes: u64,
    pub commits: u64,
    pub rollbacks: u64,
    pub errors: u64,
    pub syntax_errors: u64,
    pub conflicts: u64,
    pub deadlocks: u64,
    pub lock_waits: u64,
    pub lock_timeouts: u64,
    pub lock_wait_ns: u64,
    pub idle_aborts: u64,
    pub refused: u64,
    pub rows_examined: u64,
    pub pages_read: u64,
    pub heap_hits: u64,
    pub heap_miss: u64,
    pub wal_flushes: u64,
    pub commits_flushed: u64,
    pub wal_bytes: u64,
    pub lost_commits: u64,
    pub recoveries: u64,
    pub recovery_ms: u64,
    pub forgotten: u64,
    pub checkpoints: u64,
    pub indexes_built: u64,
}

/// the last seconds of two counters: a rate that follows what happens now
#[derive(Clone, Copy, Debug, Default)]
struct Recent {
    slots: [(u64, u64, u64); 8],
}

impl Recent {
    fn add(&mut self, now: Ns, a: u64, b: u64) {
        let sec = now / SEC;
        let s = &mut self.slots[(sec % 8) as usize];
        if s.0 != sec + 1 {
            *s = (sec + 1, 0, 0);
        }
        s.1 += a;
        s.2 += b;
    }
    fn sums(&self, now: Ns) -> (u64, u64) {
        let sec = now / SEC + 1;
        let (mut a, mut b) = (0, 0);
        for s in &self.slots {
            if s.0 != 0 && s.0 + 8 > sec && s.0 <= sec {
                a += s.1;
                b += s.2;
            }
        }
        (a, b)
    }
}

/// an open transaction
#[derive(Clone, Debug)]
struct Txn {
    iso: Iso,
    /// repeatable read: its snapshot
    snap: Option<u64>,
    writes: Vec<(u16, u64)>,
    locks: Vec<(u16, u64)>,
    /// the lock it waits for
    waits: Option<(u16, u64)>,
    implicit: bool,
    /// a statement of it runs
    busy: bool,
    idle_timer: Option<u64>,
}

/// who holds a row's lock and who waits for it
#[derive(Clone, Debug)]
struct RowLock {
    holder: u64,
    queue: VecDeque<Waiter>,
}

#[derive(Clone, Copy, Debug)]
struct Waiter {
    txn: u64,
    exec: u64,
    since: Ns,
    timer: u64,
}

/// a log record: what a replica would apply
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalRec {
    pub lsn: u64,
    pub txn: u64,
    pub kind: WalKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WalKind {
    Row { table: u16, key: u64, vals: Vec<(u16, Val)>, gone: bool },
    Commit,
}

/// where a statement is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Parse,
    /// pages and CPU of what it read
    Work,
    /// taking its locks, writing
    Locks,
    /// the CPU of what it wrote
    Written,
    /// its commit waits for the flush
    Flush,
}

/// what a statement resolved to
#[derive(Clone, Debug)]
enum SetVal {
    Lit(Val),
    Add(i64),
}

#[derive(Clone, Debug)]
enum WriteKind {
    None,
    /// SELECT .. FOR UPDATE: only the locks, then the rows
    Lock,
    Insert { sets: Vec<(u16, Val)> },
    Update { sets: Vec<(u16, SetVal)>, conds: Vec<RCond> },
    Delete { conds: Vec<RCond> },
}

/// a statement on its way
#[derive(Clone, Debug)]
struct Exec {
    sid: u64,
    stmt: Arc<Stmt>,
    params: Vec<Val>,
    drawn: Option<i64>,
    /// its transaction (0: none, a plain read)
    txn: u64,
    implicit: bool,
    start: Ns,
    phase: Phase,
    /// what it may still do on the CPU and the disk
    cpu_left: Ns,
    miss_rand: u64,
    miss_seq: u64,
    turn: bool,
    /// the snapshot it holds until its end (0: none of its own)
    snap_held: Option<u64>,
    snap: u64,
    table: usize,
    targets: Vec<u64>,
    next: usize,
    write: WriteKind,
    wrote: u64,
    /// what it answers when it ends
    body: String,
    count: Option<u64>,
    truncated: bool,
    examined: u64,
    hits: u64,
    misses: u64,
    lock_wait: Ns,
}

/// an index being built
#[derive(Clone, Debug)]
struct Build {
    sid: u64,
    table: usize,
    col: usize,
    start: Ns,
    /// the CPU work it has left, one piece after the other; when its disk work is done
    left: Ns,
    disk_done: Ns,
}

/// the service
#[derive(Clone, Debug)]
pub struct Db {
    pub cfg: DbCfg,
    /// its config's tables, as Service::reconfigure compares them
    tables_cfg: Option<Value>,
    disk_cfg: Option<(Ns, u64, u64, u64)>,
    tables: Vec<Table>,
    pool: Pool,
    serve: Serve,
    wheel: Wheel<DTimer>,
    execs: BTreeMap<u64, Exec>,
    next_exec: u64,
    ready: VecDeque<u64>,
    txns: BTreeMap<u64, Txn>,
    next_txn: u64,
    locks: BTreeMap<(u16, u64), RowLock>,
    /// open snapshots: stamp -> how many
    snaps: BTreeMap<u64, u32>,
    /// the commit stamp
    clock: u64,
    /// rows with old versions
    old: BTreeSet<(u16, u64)>,
    // the log
    lsn: u64,
    flushed: u64,
    ckpt: u64,
    /// its bytes on the disk
    wal_disk: u64,
    wal_tail: VecDeque<WalRec>,
    /// the tail holds every record after this position
    tail_floor: u64,
    flushing: Option<(u64, Vec<u64>)>,
    /// commits waiting for the next flush
    pending: Vec<u64>,
    /// "relaxed": answered commits the disk does not have yet
    acked: VecDeque<(u64, Vec<(u16, u64)>)>,
    tick_armed: bool,
    ckpt_armed: bool,
    ckpt_left: u64,
    ckpt_to: u64,
    /// it replays the log until then
    recovering: Option<Ns>,
    recover_began: Ns,
    started: bool,
    reserved: bool,
    /// the tables do not fit on the disk
    no_disk: Option<(u64, u64)>,
    metered: bool,
    builds: BTreeMap<u64, Build>,
    next_build: u64,
    cache: BTreeMap<String, Arc<Stmt>>,
    rng: Pcg32,
    lat: Window,
    recent_pool: Recent,
    recent_flush: Recent,
    /// locks with waiters now, since when, the time there were any
    contended: u32,
    contended_since: Ns,
    contended_ns: Ns,
    pub mem: crate::res::SvcMem,
    pub stats: DbStats,
    seen: Ns,
    /// bound: hand over the tables
    forget_at: usize,
    close_next: bool,
}

/// a table as the lab shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableView {
    pub name: String,
    pub rows: u64,
    pub touched: u64,
    pub heap_pages: u64,
    pub held_pages: u64,
    pub depth: u64,
    pub key: String,
    pub dist: Dist,
    pub indexes: Vec<String>,
    pub columns: Vec<String>,
}

/// a lock as the lab shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockView {
    pub table: String,
    pub key: u64,
    pub holder: u64,
    pub waiting: u64,
}

/// the database as the lab shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbView {
    pub port: u16,
    pub listening: bool,
    pub metered: bool,
    pub recovering: bool,
    pub no_disk: Option<(u64, u64)>,
    pub conns: u64,
    pub max_conns: u32,
    pub txns: u64,
    pub running: u64,
    pub pool_bytes: u64,
    pub pool_used: u64,
    pub pool_rate: u64,
    pub hit_pm: u64,
    pub durable: bool,
    pub iso: Iso,
    pub lsn: u64,
    pub flushed: u64,
    pub ckpt: u64,
    pub touched: u64,
    pub stats: DbStats,
    pub p50: Option<Ns>,
    pub p95: Option<Ns>,
    pub p99: Option<Ns>,
    pub tables: Vec<TableView>,
}

impl Db {
    pub fn new(cfg: DbCfg, tables: Vec<TableSpec>) -> Db {
        let srv = ServeCfg { port: cfg.port, server: String::from("jfn-db/1.0"), max_requests: 0, max_body: 1 << 20, ..ServeCfg::default() };
        let pages = cfg.pool_bytes / cfg.layout.page;
        let seed = cfg.seed;
        let tables: Vec<Table> = tables.into_iter().enumerate().map(|(i, s)| Table::new(s, i as u16, seed)).collect();
        let mut pool = Pool::new(pages);
        for t in &tables {
            pool.set_heap(t.no, cfg.layout.heap_pages(t.spec.rows, t.spec.row_bytes));
        }
        Db {
            pool,
            tables_cfg: None,
            disk_cfg: None,
            tables,
            serve: Serve::new(srv, K_SERVE),
            wheel: Wheel::new(K_OWN),
            execs: BTreeMap::new(),
            next_exec: 0,
            ready: VecDeque::new(),
            txns: BTreeMap::new(),
            next_txn: 0,
            locks: BTreeMap::new(),
            snaps: BTreeMap::new(),
            clock: 0,
            old: BTreeSet::new(),
            lsn: 0,
            flushed: 0,
            ckpt: 0,
            wal_disk: 0,
            wal_tail: VecDeque::new(),
            tail_floor: 0,
            flushing: None,
            pending: Vec::new(),
            acked: VecDeque::new(),
            tick_armed: false,
            ckpt_armed: false,
            ckpt_left: 0,
            ckpt_to: 0,
            recovering: None,
            recover_began: 0,
            started: false,
            reserved: false,
            no_disk: None,
            metered: false,
            builds: BTreeMap::new(),
            next_build: 0,
            cache: BTreeMap::new(),
            rng: Pcg32::derive(seed ^ 0x4442_5F44, 1),
            lat: Window::new(300, SEC),
            recent_pool: Recent::default(),
            recent_flush: Recent::default(),
            contended: 0,
            contended_since: 0,
            contended_ns: 0,
            mem: crate::res::SvcMem::default(),
            stats: DbStats::default(),
            seen: 0,
            forget_at: 0,
            close_next: false,
            cfg,
        }
    }

    /// the log's write position and what the disk has
    pub fn lsn(&self) -> u64 {
        self.lsn
    }
    pub fn flushed_lsn(&self) -> u64 {
        self.flushed
    }
    /// the records from `from` on; None when the bounded tail no longer reaches back that far: a replica would
    /// start from a copy then
    pub fn wal_since(&self, from: u64) -> Option<Vec<WalRec>> {
        (from >= self.tail_floor).then(|| self.wal_tail.iter().filter(|r| r.lsn > from).cloned().collect())
    }
    /// its tables do not fit on the disk: it says so to every statement (problems.rs)
    pub fn disk_problem(&self) -> Option<Msg> {
        self.no_disk.map(|(need, cap)| msg!("Die Tabellen brauchen {}, die Platte hat {}", crate::disk::size_text(need), crate::disk::size_text(cap)))
    }
    /// the tables' state
    pub fn tables(&self) -> Vec<TableView> {
        let l = self.cfg.layout;
        self.tables
            .iter()
            .map(|t| TableView {
                name: t.spec.name.clone(),
                rows: t.count(1, u64::MAX, &[], Snap { ts: self.clock, txn: 0 }),
                touched: t.rows.len() as u64,
                heap_pages: l.heap_pages(t.spec.rows.max(t.next_key), t.spec.row_bytes),
                held_pages: self.pool.held_of(t.no),
                depth: l.depth(t.spec.rows.max(t.next_key)),
                key: t.spec.key.clone(),
                dist: t.spec.dist,
                indexes: t.indexes.iter().map(|c| String::from(t.spec.col_name(*c))).collect(),
                columns: (1..=t.spec.cols.len()).map(|c| String::from(t.spec.col_name(c))).collect(),
            })
            .collect()
    }
    /// the row locks now
    pub fn lock_list(&self) -> Vec<LockView> {
        self.locks.iter().map(|(k, l)| LockView { table: self.tables.get(k.0 as usize).map_or(String::new(), |t| t.spec.name.clone()), key: k.1, holder: l.holder, waiting: l.queue.len() as u64 }).collect()
    }
    /// what it shows
    pub fn view(&self, now: Ns) -> DbView {
        let (h, m) = self.recent_pool.sums(now);
        DbView {
            port: self.cfg.port,
            listening: self.serve.listening(),
            metered: self.metered,
            recovering: self.recovering.is_some(),
            no_disk: self.no_disk,
            conns: self.serve.conns() as u64,
            max_conns: self.cfg.max_conns,
            txns: self.txns.len() as u64,
            running: self.execs.len() as u64,
            pool_bytes: self.cfg.pool_bytes,
            pool_used: self.pool.used() * self.cfg.layout.page,
            pool_rate: self.pool.rate,
            hit_pm: if h + m == 0 { 0 } else { h * 1000 / (h + m) },
            durable: self.cfg.durable,
            iso: self.cfg.iso,
            lsn: self.lsn,
            flushed: self.flushed,
            ckpt: self.ckpt,
            touched: self.touched(),
            stats: self.stats,
            p50: self.lat.p50(now),
            p95: self.lat.p95(now),
            p99: self.lat.p99(now),
            tables: self.tables(),
        }
    }
    /// hits and misses of heap pages so far (the tests' closed forms)
    pub fn heap_counts(&self) -> (u64, u64) {
        (self.stats.heap_hits, self.stats.heap_miss)
    }
    /// the pool (the tests)
    pub fn pool(&self) -> &Pool {
        &self.pool
    }
    /// a table's values at the newest commit, the key first (the tests)
    pub fn row_now(&self, table: &str, key: u64) -> Option<Vec<Val>> {
        let t = self.tables.iter().find(|t| t.spec.name.eq_ignore_ascii_case(table))?;
        t.image(key, Snap { ts: self.clock, txn: 0 })
    }
    /// the table (the tests: its distribution)
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.spec.name.eq_ignore_ascii_case(name))
    }
    /// how a statement would go, as dbctl and EXPLAIN say it
    pub fn explain(&self, sql: &str) -> Result<String, SqlErr> {
        let s = dbsql::parse(sql)?;
        let inner = match &s {
            Stmt::Explain(i) => &**i,
            x => x,
        };
        self.explain_text(inner, &[])
    }
    fn touched(&self) -> u64 {
        self.tables.iter().map(|t| t.rows.len() as u64).sum()
    }
    fn table_at(&self, name: &str, pos: usize) -> Result<usize, SqlErr> {
        self.tables.iter().position(|t| t.spec.name.eq_ignore_ascii_case(name)).ok_or_else(|| SqlErr { pos, text: format!("Die Tabelle \u{201e}{}\u{201c} gibt es nicht.", name) })
    }

    // ---------------------------------------------------------------- what a statement means

    fn lit_val(lit: &Lit, params: &[Val], pos: usize) -> Result<Val, SqlErr> {
        match lit {
            Lit::Int(v) => Ok(Val::Int(*v)),
            Lit::Text(t) => Ok(Val::Text(t.clone())),
            Lit::Param(i) => params.get(*i as usize).cloned().ok_or_else(|| SqlErr { pos, text: format!("Der Wert zu ? Nummer {} fehlt.", *i + 1) }),
        }
    }
    fn col_of(t: &Table, name: &dbsql::ColRef) -> Result<usize, SqlErr> {
        if let Some(q) = &name.table
            && !q.eq_ignore_ascii_case(&t.spec.name)
        {
            return Err(SqlErr { pos: name.pos, text: format!("Hier gilt nur die Tabelle \u{201e}{}\u{201c}, nicht \u{201e}{}\u{201c}.", t.spec.name, q) });
        }
        t.spec.col(&name.name).ok_or_else(|| SqlErr { pos: name.pos, text: format!("Die Spalte \u{201e}{}\u{201c} gibt es in \u{201e}{}\u{201c} nicht.", name.name, t.spec.name) })
    }
    /// a WHERE in the table's terms
    fn conds_of(t: &Table, filter: &[Cond], params: &[Val]) -> Result<Vec<RCond>, SqlErr> {
        let mut out = Vec::with_capacity(filter.len());
        for f in filter {
            let col = Db::col_of(t, &f.col)?;
            let v = Db::lit_val(&f.val, params, f.col.pos)?;
            if t.spec.is_text(col) {
                let Val::Text(text) = v else {
                    return Err(SqlErr { pos: f.col.pos, text: format!("Die Spalte \u{201e}{}\u{201c} hält Text: sie vergleicht sich mit 'Text'.", f.col.name) });
                };
                if f.op != Cmp::Eq {
                    return Err(SqlErr { pos: f.col.pos, text: format!("Die Spalte \u{201e}{}\u{201c} hält Text: sie kennt nur =.", f.col.name) });
                }
                let derived = t.derived_of_text(col, &text);
                out.push(RCond::Text { col, val: text, derived });
                continue;
            }
            let Val::Int(n) = v else {
                return Err(SqlErr { pos: f.col.pos, text: format!("Die Spalte \u{201e}{}\u{201c} hält Zahlen: sie vergleicht sich mit einer Zahl.", f.col.name) });
            };
            let (lo, hi) = match f.op {
                Cmp::Eq => (n, n),
                Cmp::Lt => (i64::MIN, n.saturating_sub(1)),
                Cmp::Le => (i64::MIN, n),
                Cmp::Gt => (n.saturating_add(1), i64::MAX),
                Cmp::Ge => (n, i64::MAX),
                Cmp::Between => {
                    let h = f.hi.as_ref().map(|h| Db::lit_val(h, params, f.col.pos)).transpose()?;
                    let Some(Val::Int(h)) = h else {
                        return Err(SqlErr { pos: f.col.pos, text: String::from("BETWEEN nimmt zwei Zahlen.") });
                    };
                    (n, h)
                }
            };
            out.push(RCond::Int { col, lo, hi });
        }
        Ok(out)
    }

    /// the rows of a read at a snapshot, and what it touches
    #[allow(clippy::too_many_arguments)]
    fn run_find(t: &Table, l: &Layout, cs: &[RCond], order: Option<(usize, bool)>, limit: Option<u64>, count_only: bool, cap: u64, s: Snap) -> (Access, &'static str, dbstore::Found, Cost) {
        let (access, why) = best_access(t, cs, order.map(|o| o.0));
        let desc = order.is_some_and(|o| o.1);
        let want = if count_only { 0 } else { limit.unwrap_or(u64::MAX).min(cap) };
        let mut f = dbstore::Found::default();
        let top = t.spec.rows.max(t.next_key);
        let (lo, hi) = match &access {
            Access::Key { lo, hi } => (*lo, (*hi).min(top)),
            _ => (1, top),
        };
        let rest: Vec<RCond> = cs.iter().filter(|c| c.col() != 0 || matches!(access, Access::Index { .. })).cloned().collect();
        match &access {
            Access::Key { .. } | Access::Scan => {
                let plain = rest.is_empty();
                f.keys = t.by_key(lo, hi, &rest, want, desc, s);
                let enough = limit.is_some_and(|n| f.keys.len() as u64 >= n);
                f.matches = if count_only || (want > 0 && f.keys.len() as u64 >= want && !enough) { t.count(lo, hi, &rest, s) } else { f.keys.len() as u64 };
                f.examined = if plain {
                    f.matches.min(if enough { f.keys.len() as u64 } else { u64::MAX })
                } else if enough {
                    // up to the last row it needed
                    let last = f.keys.last().copied().unwrap_or(lo);
                    if desc { hi.saturating_sub(last) + 1 } else { last.saturating_sub(lo) + 1 }
                } else {
                    t.count(lo, hi, &[], s)
                };
            }
            Access::Index { cond } => {
                let others: Vec<RCond> = cs.iter().filter(|c| *c != cond).cloned().collect();
                if count_only {
                    f.matches = t.count(1, top, cs, s);
                    f.examined = t.count(1, top, core::slice::from_ref(cond), s);
                } else {
                    let (keys, looked) = t.by_index(cond, &others, want, desc, s);
                    f.keys = keys;
                    let enough = limit.is_some_and(|n| f.keys.len() as u64 >= n);
                    let cut = f.keys.len() as u64 >= want;
                    f.matches = if cut && !enough { t.count(1, top, cs, s) } else { f.keys.len() as u64 };
                    f.examined = if cut && !enough { t.count(1, top, core::slice::from_ref(cond), s) } else { looked };
                }
            }
        }
        let found = if count_only { f.matches.max(f.examined) } else { f.keys.len() as u64 };
        let cost = cost_of(t, l, &access, f.examined, found);
        (access, why, f, cost)
    }

    fn explain_text(&self, s: &Stmt, params: &[Val]) -> Result<String, SqlErr> {
        let (table, filter, order, limit, count_only) = match s {
            Stmt::Select(x) => (x.table.as_str(), &x.filter, x.order.as_ref(), x.limit, matches!(x.what, What::Count)),
            Stmt::Update { table, filter, .. } | Stmt::Delete { table, filter, .. } => (table.as_str(), filter, None, None, false),
            _ => return Err(SqlErr { pos: 1, text: String::from("EXPLAIN nimmt SELECT, UPDATE und DELETE.") }),
        };
        let pos = s.table().map_or(1, |x| x.1);
        let ti = self.table_at(table, pos)?;
        let t = &self.tables[ti];
        let cs = Db::conds_of(t, filter, params)?;
        let ord = match order {
            Some((c, d)) => Some((self.order_col(t, c)?, *d)),
            None => None,
        };
        let snap = Snap { ts: self.clock, txn: 0 };
        let (access, why, f, cost) = Db::run_find(t, &self.cfg.layout, &cs, ord, limit, count_only, self.cfg.max_rows, snap);
        let (way, index) = match &access {
            Access::Key { .. } => ("index", String::from(t.spec.key.as_str())),
            Access::Index { cond } => ("index", String::from(t.spec.col_name(cond.col()))),
            Access::Scan => ("scan", String::new()),
        };
        let cpu = self.cfg.stmt_cpu + self.cfg.page_cpu * cost.pages() + self.cfg.row_cpu * f.examined;
        let mut out = format!("{{\"plan\":\"{}\",", way);
        if !index.is_empty() {
            out.push_str(&format!("\"index\":\"{}\",", index));
        }
        if !index.is_empty() {
            out.push_str(&format!("\"depth\":{},", cost.upper + 1));
        }
        out.push_str(&format!("\"est_rows\":{},\"rows_examined\":{},\"pages\":{},\"pages_tree\":{},\"pages_heap\":{},\"cpu_us\":{},\"why\":\"{}\"}}", f.matches, f.examined, cost.pages(), cost.upper + cost.leaves, cost.heap, cpu / US, why));
        Ok(out)
    }

    fn order_col(&self, t: &Table, c: &dbsql::ColRef) -> Result<usize, SqlErr> {
        let col = Db::col_of(t, c)?;
        if col != 0 && !t.indexes.contains(&col) {
            return Err(SqlErr { pos: c.pos, text: format!("ORDER BY geht nur über einen Index: auf \u{201e}{}\u{201c} liegt keiner.", c.name) });
        }
        Ok(col)
    }

    // ---------------------------------------------------------------- answers

    fn reply(&mut self, os: &mut OsApi, sid: u64, status: u16, body: String, start: Option<Ns>) {
        let now = os.now();
        let mut h = Headers::new();
        h.add("Content-Type", "application/json");
        // a connection over the limit goes with its refusal
        let close = core::mem::take(&mut self.close_next);
        let rep = Reply { status, headers: h, body: body.into_bytes(), chunk: 0, close };
        if self.serve.respond(os, sid, rep)
            && let Some(t) = start
        {
            self.lat.record(now, now - t);
        }
    }
    fn fail_body(code: &str, text: &str, pos: usize, retry: bool) -> String {
        let mut e = String::new();
        Val::Text(String::from(text)).json(&mut e);
        if pos > 0 {
            format!("{{\"ok\":false,\"code\":\"{}\",\"error\":{},\"pos\":{},\"retryable\":{}}}\n", code, e, pos, retry)
        } else {
            format!("{{\"ok\":false,\"code\":\"{}\",\"error\":{},\"retryable\":{}}}\n", code, e, retry)
        }
    }
    fn refuse(&mut self, os: &mut OsApi, sid: u64, status: u16, code: &str, text: &str, pos: usize, retry: bool, start: Option<Ns>) {
        self.stats.errors += 1;
        if status == 409 {
            self.stats.conflicts += 1;
        }
        self.reply(os, sid, status, Db::fail_body(code, text, pos, retry), start);
    }

    // ---------------------------------------------------------------- requests

    fn request(&mut self, os: &mut OsApi, r: ReqIn) {
        let path = r.head.path();
        match (r.head.method.as_str(), path) {
            ("POST", "/query") => self.query(os, r),
            ("GET" | "HEAD", "/_db/health") => {
                let b = self.health();
                self.reply(os, r.id, 200, b, None);
            }
            ("GET" | "HEAD", "/_db/stats") => {
                let b = self.stats_json(os.now());
                self.reply(os, r.id, 200, b, None);
            }
            (_, "/query") | (_, "/_db/health") | (_, "/_db/stats") => {
                let mut h = Headers::new();
                h.add("Allow", if path == "/query" { "POST" } else { "GET, HEAD" });
                h.add("Content-Type", "text/plain");
                self.serve.respond(os, r.id, Reply { status: 405, headers: h, body: Vec::from(&b"405 Method Not Allowed\n"[..]), chunk: 0, close: false });
            }
            _ => {
                self.serve.respond(os, r.id, Reply::status_page(404));
            }
        }
    }

    fn query(&mut self, os: &mut OsApi, r: ReqIn) {
        self.stats.requests += 1;
        let start = r.at;
        if self.recovering.is_some() {
            self.stats.refused += 1;
            self.refuse(os, r.id, 503, "recovering", "Die Datenbank liest ihr Log noch: sie nimmt noch keine Anweisungen an.", 0, true, Some(start));
            return;
        }
        if let Some((need, cap)) = self.no_disk {
            let t = format!("Die Tabellen brauchen {}, die Platte hat {}.", crate::disk::size_text(need), crate::disk::size_text(cap));
            self.refuse(os, r.id, 507, "disk_full", &t, 0, false, Some(start));
            return;
        }
        // the connections beyond the limit are refused, the oldest stay
        if self.serve.conns() > self.cfg.max_conns as usize && self.serve.rank(r.sock) >= self.cfg.max_conns as usize {
            self.stats.refused += 1;
            let t = format!("Zu viele Verbindungen: höchstens {}.", self.cfg.max_conns);
            self.close_next = true;
            self.refuse(os, r.id, 503, "too_many_connections", &t, 0, true, Some(start));
            return;
        }
        let (sql, mut params, mut txn) = match Db::body_of(&r.body) {
            Ok(x) => x,
            Err(t) => {
                self.stats.syntax_errors += 1;
                self.refuse(os, r.id, 400, "syntax", &t, 0, false, Some(start));
                return;
            }
        };
        if txn.is_none() {
            txn = r.head.header("x-txn").and_then(|v| v.trim().parse().ok()).or_else(|| crate::kv::query_value(&r.head.target, "txn").and_then(|v| v.parse().ok()));
        }
        let stmt = match self.prepared(&sql) {
            Ok(s) => s,
            Err(e) => {
                self.stats.syntax_errors += 1;
                let t = e.line();
                self.refuse(os, r.id, 400, "syntax", &t, e.pos, false, Some(start));
                return;
            }
        };
        // drawn keys: by the distribution of the statement's table
        let mut drawn = None;
        let table = stmt.table().and_then(|(n, _)| self.tables.iter().position(|t| t.spec.name.eq_ignore_ascii_case(n)));
        for p in params.iter_mut() {
            if p.1 {
                let Some(ti) = table else {
                    self.stats.syntax_errors += 1;
                    self.refuse(os, r.id, 400, "syntax", "Einen Schlüssel ziehen geht nur für eine Anweisung mit einer Tabelle.", 0, false, Some(start));
                    return;
                };
                let k = match drawn {
                    Some(k) => k,
                    None => {
                        let k = self.tables[ti].draw(&mut self.rng) as i64;
                        drawn = Some(k);
                        k
                    }
                };
                p.0 = Val::Int(k);
            }
        }
        let params: Vec<Val> = params.into_iter().map(|p| p.0).collect();
        self.next_exec += 1;
        let id = self.next_exec;
        let e = Exec {
            sid: r.id,
            stmt,
            params,
            drawn,
            txn: txn.unwrap_or(0),
            implicit: false,
            start,
            phase: Phase::Parse,
            cpu_left: 0,
            miss_rand: 0,
            miss_seq: 0,
            turn: false,
            snap_held: None,
            snap: 0,
            table: 0,
            targets: Vec::new(),
            next: 0,
            write: WriteKind::None,
            wrote: 0,
            body: String::new(),
            count: None,
            truncated: false,
            examined: 0,
            hits: 0,
            misses: 0,
            lock_wait: 0,
        };
        self.execs.insert(id, e);
        // its parsing first
        let (now, at) = (os.now(), os.cpu(self.cfg.stmt_cpu));
        if at > now {
            self.wheel.add(at, DTimer::Step(id));
        } else {
            self.ready.push_back(id);
        }
    }

    /// what a request's body asks: raw text, or JSON
    #[allow(clippy::type_complexity)]
    fn body_of(body: &[u8]) -> Result<(String, Vec<(Val, bool)>, Option<u64>), String> {
        let first = body.iter().copied().find(|b| !b.is_ascii_whitespace());
        if first != Some(b'{') {
            return match core::str::from_utf8(body) {
                Ok(s) => Ok((String::from(s), Vec::new(), None)),
                Err(_) => Err(String::from("Die Anweisung ist kein UTF-8-Text.")),
            };
        }
        let v = agent_core::json::parse(body).map_err(|_| String::from("Der Text beginnt mit { und ist doch kein JSON."))?;
        let Some(sql) = v.get("sql").and_then(|x| x.as_str()) else {
            return Err(String::from("Im JSON fehlt \u{201e}sql\u{201c}."));
        };
        let mut params = Vec::new();
        for p in v.get("params").and_then(|x| x.as_array()).unwrap_or(&[]) {
            params.push(match p {
                Value::String(s) => (Val::Text(s.clone()), false),
                Value::Number(n) => match n.as_str().parse::<i64>() {
                    Ok(x) => (Val::Int(x), false),
                    Err(_) => return Err(format!("{} ist keine ganze Zahl.", n.as_str())),
                },
                Value::Object(_) if p.get("draw").and_then(|d| d.as_bool()) == Some(true) => (Val::Int(0), true),
                _ => return Err(String::from("Ein Wert in \u{201e}params\u{201c} ist eine Zahl, ein Text oder {\"draw\": true}.")),
            });
        }
        Ok((String::from(sql), params, v.get("txn").and_then(|x| x.as_u64())))
    }

    fn prepared(&mut self, sql: &str) -> Result<Arc<Stmt>, SqlErr> {
        if let Some(s) = self.cache.get(sql) {
            return Ok(s.clone());
        }
        let s = Arc::new(dbsql::parse(sql)?);
        if self.cache.len() >= CACHE_MAX {
            self.cache.pop_first();
        }
        self.cache.insert(String::from(sql), s.clone());
        Ok(s)
    }

    // ---------------------------------------------------------------- a statement's way

    /// it goes on until it waits or ends
    fn advance(&mut self, os: &mut OsApi, id: u64) {
        let Some(mut e) = self.execs.remove(&id) else {
            return;
        };
        loop {
            match e.phase {
                Phase::Parse => {
                    if !self.begin(os, id, &mut e) {
                        return;
                    }
                }
                Phase::Work => {
                    if let Some(at) = self.work(os, &mut e) {
                        self.wheel.add(at, DTimer::Step(id));
                        self.execs.insert(id, e);
                        return;
                    }
                    if matches!(e.write, WriteKind::None) {
                        self.finish(os, id, e);
                        return;
                    }
                    e.phase = Phase::Locks;
                }
                Phase::Locks => match self.locks_go(os, id, &mut e) {
                    Go::On => {
                        e.cpu_left = self.cfg.write_cpu * e.wrote;
                        e.phase = Phase::Written;
                    }
                    Go::Wait => {
                        self.execs.insert(id, e);
                        return;
                    }
                    Go::Fail => return,
                },
                Phase::Written => {
                    if let Some(at) = self.work(os, &mut e) {
                        self.wheel.add(at, DTimer::Step(id));
                        self.execs.insert(id, e);
                        return;
                    }
                    self.finish(os, id, e);
                    return;
                }
                Phase::Flush => {
                    self.execs.insert(id, e);
                    return;
                }
            }
        }
    }

    /// CPU and disk still due: when the next piece is done; None when nothing is left
    fn work(&mut self, os: &mut OsApi, e: &mut Exec) -> Option<Ns> {
        let now = os.now();
        let page = self.cfg.layout.page;
        if !self.metered {
            e.cpu_left = 0;
        }
        loop {
            e.turn = !e.turn;
            if e.miss_rand > 0 && (e.turn || e.cpu_left == 0) {
                let n = e.miss_rand.min(RAND_BATCH);
                e.miss_rand -= n;
                self.stats.pages_read += n;
                // read one after the other: each with its latency
                let mut at = now;
                for _ in 0..n {
                    at = os.disk_read(page);
                }
                let lat = os.disk().map_or(0, |d| d.latency + d.slow_extra);
                return Some(at + lat * (n - 1));
            }
            if e.miss_seq > 0 && (e.turn || e.cpu_left == 0) {
                let n = e.miss_seq.min(SEQ_BATCH);
                e.miss_seq -= n;
                self.stats.pages_read += n;
                return Some(os.disk_read(n * page));
            }
            if e.cpu_left == 0 {
                if e.miss_rand == 0 && e.miss_seq == 0 {
                    return None;
                }
                continue;
            }
            let at = os.cpu_slice(&mut e.cpu_left);
            if at > now {
                return Some(at);
            }
        }
    }

    /// what the statement means, after its parsing: false when it is over
    fn begin(&mut self, os: &mut OsApi, id: u64, e: &mut Exec) -> bool {
        self.stats.statements += 1;
        let stmt = e.stmt.clone();
        let r = match &*stmt {
            Stmt::Begin(iso) => self.do_begin(os, id, e, *iso),
            Stmt::Commit => self.do_commit(os, id, e),
            Stmt::Rollback => self.do_rollback(os, e),
            Stmt::Explain(inner) => match self.explain_text(inner, &e.params) {
                Ok(b) => {
                    let body = format!("{{\"ok\":true,\"explain\":{}}}\n", b);
                    self.reply(os, e.sid, 200, body, Some(e.start));
                    Ok(false)
                }
                Err(x) => Err(x),
            },
            Stmt::CreateIndex { table, table_pos, col } => self.do_create_index(os, e, table, *table_pos, col),
            Stmt::DropIndex { table, table_pos, col } => self.do_drop_index(os, e, table, *table_pos, col),
            Stmt::Select(s) => self.do_select(os, id, e, s),
            Stmt::Insert { table, table_pos, cols, vals } => self.do_insert(os, id, e, table, *table_pos, cols, vals),
            Stmt::Update { table, table_pos, sets, filter } => self.do_update(os, id, e, table, *table_pos, sets, filter),
            Stmt::Delete { table, table_pos, filter } => self.do_delete(os, id, e, table, *table_pos, filter),
        };
        match r {
            Ok(goes_on) => goes_on,
            Err(x) => {
                self.stats.syntax_errors += 1;
                let t = x.line();
                self.refuse(os, e.sid, 400, "syntax", &t, x.pos, false, Some(e.start));
                self.statement_over(os, e);
                false
            }
        }
    }

    /// the transaction a statement runs in; Err: there is none or it is busy (answered)
    fn enter(&mut self, os: &mut OsApi, e: &mut Exec) -> Result<(), ()> {
        if e.txn == 0 {
            return Ok(());
        }
        let Some(t) = self.txns.get_mut(&e.txn) else {
            let text = format!("Die Transaktion {} gibt es nicht (mehr).", e.txn);
            self.refuse(os, e.sid, 404, "no_transaction", &text, 0, false, Some(e.start));
            return Err(());
        };
        if t.busy {
            let text = format!("In der Transaktion {} läuft schon eine Anweisung.", e.txn);
            self.refuse(os, e.sid, 409, "txn_busy", &text, 0, true, Some(e.start));
            return Err(());
        }
        t.busy = true;
        if let Some(timer) = t.idle_timer.take() {
            self.wheel.cancel(timer);
        }
        Ok(())
    }

    /// a statement that wrote or locks needs a transaction: one of its own if it has none
    fn implicit_txn(&mut self, e: &mut Exec) {
        if e.txn == 0 {
            self.next_txn += 1;
            e.txn = self.next_txn;
            e.implicit = true;
            let iso = self.cfg.iso;
            self.txns.insert(e.txn, Txn { iso, snap: None, writes: Vec::new(), locks: Vec::new(), waits: None, implicit: true, busy: true, idle_timer: None });
        }
    }

    /// the snapshot of a statement
    fn snapshot(&mut self, e: &mut Exec) -> u64 {
        let clock = self.clock;
        if let Some(t) = self.txns.get_mut(&e.txn)
            && t.iso == Iso::RepeatableRead
        {
            if let Some(s) = t.snap {
                return s;
            }
            t.snap = Some(clock);
            *self.snaps.entry(clock).or_insert(0) += 1;
            return clock;
        }
        e.snap_held = Some(clock);
        *self.snaps.entry(clock).or_insert(0) += 1;
        clock
    }

    fn snap_drop(&mut self, ts: u64) {
        let first = self.snaps.keys().next().copied();
        if let Some(n) = self.snaps.get_mut(&ts) {
            *n -= 1;
            if *n == 0 {
                self.snaps.remove(&ts);
            }
        }
        // the oldest snapshot went: old versions may go now
        if first != self.snaps.keys().next().copied() {
            self.prune_some(256);
        }
    }

    fn horizon(&self) -> u64 {
        self.snaps.keys().next().copied().unwrap_or(self.clock).min(self.clock)
    }

    fn prune_some(&mut self, n: usize) {
        let (h, d) = (self.horizon(), self.flushed);
        let list: Vec<(u16, u64)> = self.old.iter().take(n).copied().collect();
        for (t, k) in list {
            if let Some(tb) = self.tables.get_mut(t as usize) {
                tb.prune_row(k, h, d);
                if tb.rows.get(&k).is_none_or(|r| r.vers.len() < 2) {
                    self.old.remove(&(t, k));
                }
            }
        }
    }

    fn do_begin(&mut self, os: &mut OsApi, _id: u64, e: &mut Exec, iso: Option<Iso>) -> Result<bool, SqlErr> {
        if e.txn != 0 {
            let text = format!("Die Transaktion {} läuft schon.", e.txn);
            self.refuse(os, e.sid, 409, "txn_busy", &text, 0, false, Some(e.start));
            return Ok(false);
        }
        self.next_txn += 1;
        let id = self.next_txn;
        let iso = iso.unwrap_or(self.cfg.iso);
        let now = os.now();
        let idle = self.wheel.add(now + self.cfg.txn_idle, DTimer::Idle(id));
        self.txns.insert(id, Txn { iso, snap: None, writes: Vec::new(), locks: Vec::new(), waits: None, implicit: false, busy: false, idle_timer: Some(idle) });
        let body = format!("{{\"ok\":true,\"txn\":{},\"isolation\":\"{}\"}}\n", id, iso.name());
        self.reply(os, e.sid, 200, body, Some(e.start));
        Ok(false)
    }

    fn do_commit(&mut self, os: &mut OsApi, id: u64, e: &mut Exec) -> Result<bool, SqlErr> {
        if e.txn == 0 {
            self.refuse(os, e.sid, 400, "no_transaction", "COMMIT ohne Transaktion: es fehlt \u{201e}txn\u{201c}.", 0, false, Some(e.start));
            return Ok(false);
        }
        if self.enter(os, e).is_err() {
            return Ok(false);
        }
        e.body = format!("{{\"ok\":true,\"committed\":{}}}\n", e.txn);
        self.commit(os, id, core::mem::replace(e, placeholder()));
        Ok(false)
    }

    fn do_rollback(&mut self, os: &mut OsApi, e: &mut Exec) -> Result<bool, SqlErr> {
        if e.txn == 0 {
            self.refuse(os, e.sid, 400, "no_transaction", "ROLLBACK ohne Transaktion: es fehlt \u{201e}txn\u{201c}.", 0, false, Some(e.start));
            return Ok(false);
        }
        if self.enter(os, e).is_err() {
            return Ok(false);
        }
        let txn = e.txn;
        self.abort(os, txn);
        self.stats.rollbacks += 1;
        let body = format!("{{\"ok\":true,\"rolled_back\":{}}}\n", txn);
        self.reply(os, e.sid, 200, body, Some(e.start));
        Ok(false)
    }

    fn do_create_index(&mut self, os: &mut OsApi, e: &mut Exec, table: &str, pos: usize, col: &dbsql::ColRef) -> Result<bool, SqlErr> {
        let ti = self.table_at(table, pos)?;
        let t = &self.tables[ti];
        let c = Db::col_of(t, col)?;
        if c == 0 || t.indexes.contains(&c) {
            return Err(SqlErr { pos: col.pos, text: format!("Auf \u{201e}{}\u{201c} liegt schon ein Index.", col.name) });
        }
        if self.builds.values().any(|b| b.table == ti && b.col == c) {
            return Err(SqlErr { pos: col.pos, text: format!("Der Index auf \u{201e}{}\u{201c} wird gerade gebaut.", col.name) });
        }
        // it reads all rows and sorts them, then writes its pages: it takes that time on the real CPU and disk,
        // in the background of other statements
        let l = self.cfg.layout;
        let rows = t.spec.rows.max(t.next_key);
        let log2 = 64 - rows.max(2).leading_zeros() as u64;
        let index_bytes = l.tree_pages(rows) * l.page;
        let heap_bytes = l.heap_pages(rows, t.spec.row_bytes) * l.page;
        let left = if self.metered { self.cfg.row_cpu.saturating_mul(rows).saturating_mul(log2) } else { 0 };
        let read = os.disk_read(heap_bytes);
        let wrote = os.disk_write(index_bytes, index_bytes as i64);
        let Some(wrote) = wrote else {
            self.refuse(os, e.sid, 507, "disk_full", "Für diesen Index ist die Platte zu voll.", 0, false, Some(e.start));
            return Ok(false);
        };
        self.next_build += 1;
        let id = self.next_build;
        self.builds.insert(id, Build { sid: e.sid, table: ti, col: c, start: e.start, left, disk_done: read.max(wrote) });
        self.build_step(os, id);
        Ok(false)
    }

    /// an index's next piece of work (one core at a time, other statements in between), then it is there
    fn build_step(&mut self, os: &mut OsApi, id: u64) {
        let now = os.now();
        let Some(b) = self.builds.get_mut(&id) else {
            return;
        };
        if b.left > 0 {
            let piece = b.left.min(100 * MS);
            b.left -= piece;
            let at = os.cpu(piece);
            self.wheel.add(at, DTimer::Built(id));
            return;
        }
        if b.disk_done > now {
            let at = b.disk_done;
            self.wheel.add(at, DTimer::Built(id));
            return;
        }
        let Some(b) = self.builds.remove(&id) else {
            return;
        };
        if !self.tables[b.table].indexes.contains(&b.col) {
            self.tables[b.table].indexes.push(b.col);
            self.tables[b.table].index_added(b.col);
        }
        self.stats.indexes_built += 1;
        self.reply(os, b.sid, 200, String::from("{\"ok\":true,\"created\":true}\n"), Some(b.start));
    }

    fn do_drop_index(&mut self, os: &mut OsApi, e: &mut Exec, table: &str, pos: usize, col: &dbsql::ColRef) -> Result<bool, SqlErr> {
        let ti = self.table_at(table, pos)?;
        let t = &self.tables[ti];
        let c = Db::col_of(t, col)?;
        let Some(i) = t.indexes.iter().position(|x| *x == c) else {
            return Err(SqlErr { pos: col.pos, text: format!("Auf \u{201e}{}\u{201c} liegt kein Index, der sich löschen ließe.", col.name) });
        };
        let l = self.cfg.layout;
        os.disk_free(l.tree_pages(t.spec.rows.max(t.next_key)) * l.page);
        self.tables[ti].indexes.remove(i);
        self.reply(os, e.sid, 200, String::from("{\"ok\":true,\"dropped\":true}\n"), Some(e.start));
        Ok(false)
    }

    /// counts what a walk through the pool gave, writes back what it pushed out
    fn charged(&mut self, os: &mut OsApi, e: &mut Exec, cost: &Cost, touch: Touch) {
        e.examined += cost.examined;
        e.hits += touch.hits;
        e.misses += touch.misses();
        e.miss_rand += touch.miss_rand;
        e.miss_seq += touch.miss_seq;
        e.cpu_left += self.cfg.page_cpu * cost.pages() + self.cfg.row_cpu * cost.examined;
        self.stats.rows_examined += cost.examined;
        self.stats.heap_hits += touch.heap_hits;
        self.stats.heap_miss += touch.heap_miss;
        self.recent_pool.add(os.now(), touch.hits, touch.misses());
        if self.pool.evicted_dirty > 0 {
            let n = core::mem::take(&mut self.pool.evicted_dirty) * self.pool.rate * self.cfg.layout.page;
            let _ = os.disk_write(n, 0);
        }
    }

    fn do_select(&mut self, os: &mut OsApi, _id: u64, e: &mut Exec, s: &Select) -> Result<bool, SqlErr> {
        self.stats.selects += 1;
        let ti = self.table_at(&s.table, s.table_pos)?;
        let cs = Db::conds_of(&self.tables[ti], &s.filter, &e.params)?;
        let order = match &s.order {
            Some((c, d)) => {
                let t = &self.tables[ti];
                Some((self.order_col(t, c)?, *d))
            }
            None => None,
        };
        if self.enter(os, e).is_err() {
            return Ok(false);
        }
        if s.for_update {
            self.implicit_txn(e);
        }
        e.table = ti;
        let ts = self.snapshot(e);
        e.snap = ts;
        let snap = Snap { ts, txn: e.txn };
        let count_only = matches!(s.what, What::Count);
        let l = self.cfg.layout;
        let cap = if s.for_update { self.cfg.write_rows_max + 1 } else { self.cfg.max_rows };
        let (access, _, found, cost) = Db::run_find(&self.tables[ti], &l, &cs, order, s.limit, count_only, cap, snap);
        let touch = walk(&mut self.pool, &self.tables[ti], &l, &access, &cost, &found.keys, false);
        self.charged(os, e, &cost, touch);
        e.phase = Phase::Work;
        if s.for_update {
            if count_only {
                return Err(SqlErr { pos: s.table_pos, text: String::from("COUNT(*) und FOR UPDATE passen nicht zusammen.") });
            }
            if found.matches > self.cfg.write_rows_max {
                return self.too_many(os, e, found.matches);
            }
            e.targets = found.keys;
            e.write = WriteKind::Lock;
            return Ok(true);
        }
        if count_only {
            e.count = Some(found.matches);
            return Ok(true);
        }
        e.truncated = (found.keys.len() as u64) < found.matches && s.limit.is_none_or(|n| n > found.keys.len() as u64);
        if let Some(j) = &s.join {
            return self.do_join(os, e, s, j, found.keys, snap);
        }
        let cols = self.columns_of(ti, &s.what)?;
        let mut b = String::new();
        self.rows_json(ti, &found.keys, snap, &cols, &mut b);
        e.body = b;
        e.count = Some(found.keys.len() as u64);
        Ok(true)
    }

    fn columns_of(&self, ti: usize, w: &What) -> Result<Vec<usize>, SqlErr> {
        let t = &self.tables[ti];
        Ok(match w {
            What::Cols(list) => list.iter().map(|c| Db::col_of(t, c)).collect::<Result<_, _>>()?,
            _ => (0..=t.spec.cols.len()).collect(),
        })
    }

    fn rows_json(&self, ti: usize, keys: &[u64], s: Snap, cols: &[usize], out: &mut String) {
        let t = &self.tables[ti];
        let mut first = true;
        for k in keys {
            // (a row that went while FOR UPDATE waited for its lock is not there any more)
            let Some(img) = t.image(*k, s) else { continue };
            if !first {
                out.push(',');
            }
            first = false;
            out.push('{');
            for (j, c) in cols.iter().enumerate() {
                if j > 0 {
                    out.push(',');
                }
                out.push('"');
                out.push_str(t.spec.col_name(*c));
                out.push_str("\":");
                img[*c].json(out);
            }
            out.push('}');
        }
    }

    /// a join: each row of the first table with its partners, over the second table's key or index
    fn do_join(&mut self, os: &mut OsApi, e: &mut Exec, s: &Select, j: &Join, outer: Vec<u64>, snap: Snap) -> Result<bool, SqlErr> {
        let ta = e.table;
        let tb = self.table_at(&j.table, j.pos)?;
        // ON a.x = b.y in either order
        let on_first = |c: &dbsql::ColRef| c.table.as_deref().is_none_or(|t| t.eq_ignore_ascii_case(&s.table));
        let (l, r) = if on_first(&j.left) { (&j.left, &j.right) } else { (&j.right, &j.left) };
        let lc = self.tables[ta].spec.col(&l.name).ok_or_else(|| SqlErr { pos: l.pos, text: format!("Die Spalte \u{201e}{}\u{201c} gibt es in \u{201e}{}\u{201c} nicht.", l.name, s.table) })?;
        let rc = self.tables[tb].spec.col(&r.name).ok_or_else(|| SqlErr { pos: r.pos, text: format!("Die Spalte \u{201e}{}\u{201c} gibt es in \u{201e}{}\u{201c} nicht.", r.name, j.table) })?;
        if rc != 0 && !self.tables[tb].indexes.contains(&rc) {
            return Err(SqlErr { pos: r.pos, text: format!("Ein JOIN geht nur über einen Index: auf \u{201e}{}\u{201c} liegt keiner.", r.name) });
        }
        let cap = self.cfg.max_rows;
        let layout = self.cfg.layout;
        let mut out = String::new();
        let mut n = 0u64;
        let names_a: Vec<usize> = (0..=self.tables[ta].spec.cols.len()).collect();
        let names_b: Vec<usize> = (0..=self.tables[tb].spec.cols.len()).collect();
        let picks: Option<Vec<(bool, usize)>> = match &s.what {
            What::Cols(list) => {
                let mut v = Vec::new();
                for c in list {
                    let second = c.table.as_deref().is_some_and(|t| t.eq_ignore_ascii_case(&j.table));
                    let t = &self.tables[if second { tb } else { ta }];
                    v.push((second, t.spec.col(&c.name).ok_or_else(|| SqlErr { pos: c.pos, text: format!("Die Spalte \u{201e}{}\u{201c} gibt es in \u{201e}{}\u{201c} nicht.", c.name, t.spec.name) })?));
                }
                Some(v)
            }
            _ => None,
        };
        for k in &outer {
            let Some(img_a) = self.tables[ta].image(*k, snap) else { continue };
            let cond = match &img_a[lc] {
                Val::Int(v) => RCond::Int { col: rc, lo: *v, hi: *v },
                Val::Text(t) => RCond::Text { col: rc, val: t.clone(), derived: self.tables[tb].derived_of_text(rc, t) },
            };
            let (access, _, found, cost) = Db::run_find(&self.tables[tb], &layout, core::slice::from_ref(&cond), None, None, false, cap, snap);
            let touch = walk(&mut self.pool, &self.tables[tb], &layout, &access, &cost, &found.keys, false);
            self.charged(os, e, &cost, touch);
            for kb in &found.keys {
                let Some(img_b) = self.tables[tb].image(*kb, snap) else { continue };
                if n >= cap || s.limit.is_some_and(|m| n >= m) {
                    break;
                }
                if n > 0 {
                    out.push(',');
                }
                out.push('{');
                let mut first = true;
                let mut put = |out: &mut String, tn: &str, name: &str, v: &Val| {
                    if !first {
                        out.push(',');
                    }
                    first = false;
                    out.push('"');
                    out.push_str(tn);
                    out.push('.');
                    out.push_str(name);
                    out.push_str("\":");
                    v.json(out);
                };
                match &picks {
                    Some(list) => {
                        for (second, c) in list {
                            let (t, img) = if *second { (&self.tables[tb], &img_b) } else { (&self.tables[ta], &img_a) };
                            put(&mut out, &t.spec.name, t.spec.col_name(*c), &img[*c]);
                        }
                    }
                    None => {
                        for c in &names_a {
                            put(&mut out, &self.tables[ta].spec.name, self.tables[ta].spec.col_name(*c), &img_a[*c]);
                        }
                        for c in &names_b {
                            put(&mut out, &self.tables[tb].spec.name, self.tables[tb].spec.col_name(*c), &img_b[*c]);
                        }
                    }
                }
                out.push('}');
                n += 1;
            }
        }
        e.body = out;
        e.count = Some(n);
        Ok(true)
    }

    fn too_many(&mut self, os: &mut OsApi, e: &mut Exec, n: u64) -> Result<bool, SqlErr> {
        let text = format!("Diese Anweisung beträfe {} Zeilen, höchstens {} gehen auf einmal.", n, self.cfg.write_rows_max);
        self.refuse(os, e.sid, 400, "too_many_rows", &text, 0, false, Some(e.start));
        self.statement_over(os, e);
        Ok(false)
    }

    #[allow(clippy::too_many_arguments)]
    fn do_insert(&mut self, os: &mut OsApi, _id: u64, e: &mut Exec, table: &str, pos: usize, cols: &[dbsql::ColRef], vals: &[Lit]) -> Result<bool, SqlErr> {
        self.stats.inserts += 1;
        let ti = self.table_at(table, pos)?;
        let t = &self.tables[ti];
        let mut key = None;
        let mut sets = Vec::new();
        for (c, v) in cols.iter().zip(vals.iter()) {
            let col = Db::col_of(t, c)?;
            let val = Db::lit_val(v, &e.params, c.pos)?;
            if t.spec.is_text(col) != matches!(val, Val::Text(_)) {
                return Err(SqlErr { pos: c.pos, text: format!("Der Wert passt nicht zur Spalte \u{201e}{}\u{201c}.", c.name) });
            }
            if col == 0 {
                match val {
                    Val::Int(n) if n >= 1 => key = Some(n as u64),
                    _ => return Err(SqlErr { pos: c.pos, text: String::from("Ein Schlüssel ist eine Zahl ab 1.") }),
                }
            } else {
                sets.push((col as u16, val));
            }
        }
        if self.enter(os, e).is_err() {
            return Ok(false);
        }
        self.implicit_txn(e);
        e.table = ti;
        e.snap = self.snapshot(e);
        let key = key.unwrap_or_else(|| {
            let t = &mut self.tables[ti];
            t.next_key += 1;
            t.next_key
        });
        if key > self.tables[ti].next_key {
            self.tables[ti].next_key = key;
        }
        let row_bytes = self.tables[ti].spec.row_bytes;
        if !os.disk_reserve(row_bytes) {
            self.refuse(os, e.sid, 507, "disk_full", "Die Platte ist voll.", 0, false, Some(e.start));
            self.statement_over(os, e);
            return Ok(false);
        }
        let l = self.cfg.layout;
        let access = Access::Key { lo: key, hi: key };
        let cost = cost_of(&self.tables[ti], &l, &access, 1, 1);
        let touch = walk(&mut self.pool, &self.tables[ti], &l, &access, &cost, &[key], true);
        let extra = self.index_writes(ti, key, None);
        self.charged(os, e, &cost, touch);
        e.hits += extra.hits;
        e.misses += extra.misses();
        e.miss_rand += extra.misses();
        e.targets = alloc::vec![key];
        e.write = WriteKind::Insert { sets };
        e.phase = Phase::Work;
        Ok(true)
    }

    /// a write's way into the other indexes: one leaf each (the columns it changes; None: all)
    fn index_writes(&mut self, ti: usize, key: u64, cols: Option<&[u16]>) -> Touch {
        let mut o = Touch::default();
        let t = &self.tables[ti];
        for (i, c) in t.indexes.iter().enumerate() {
            if cols.is_some_and(|l| !l.contains(&(*c as u16))) {
                continue;
            }
            let id = ((t.no as u64 & 0xFFF) << 51) | ((i as u64 + 2) << 45) | ((key - 1) / self.cfg.layout.fanout);
            if self.pool.access(id, true) {
                o.hits += 1;
            } else {
                o.miss_rand += 1;
            }
        }
        o
    }

    #[allow(clippy::too_many_arguments)]
    fn do_update(&mut self, os: &mut OsApi, _id: u64, e: &mut Exec, table: &str, pos: usize, sets: &[Set], filter: &[Cond]) -> Result<bool, SqlErr> {
        self.stats.updates += 1;
        let ti = self.table_at(table, pos)?;
        let t = &self.tables[ti];
        let cs = Db::conds_of(t, filter, &e.params)?;
        let mut rs = Vec::with_capacity(sets.len());
        for s in sets {
            let col = Db::col_of(t, &s.col)?;
            if col == 0 {
                return Err(SqlErr { pos: s.col.pos, text: String::from("Den Schlüssel einer Zeile ändert hier nichts.") });
            }
            let v = match &s.to {
                SetTo::Lit(l) => {
                    let val = Db::lit_val(l, &e.params, s.col.pos)?;
                    if t.spec.is_text(col) != matches!(val, Val::Text(_)) {
                        return Err(SqlErr { pos: s.col.pos, text: format!("Der Wert passt nicht zur Spalte \u{201e}{}\u{201c}.", s.col.name) });
                    }
                    SetVal::Lit(val)
                }
                SetTo::Add(l, minus) => {
                    if t.spec.is_text(col) {
                        return Err(SqlErr { pos: s.col.pos, text: format!("Die Spalte \u{201e}{}\u{201c} hält Text: rechnen geht nur mit Zahlen.", s.col.name) });
                    }
                    let Val::Int(n) = Db::lit_val(l, &e.params, s.col.pos)? else {
                        return Err(SqlErr { pos: s.col.pos, text: String::from("Rechnen geht nur mit Zahlen.") });
                    };
                    SetVal::Add(if *minus { -n } else { n })
                }
            };
            rs.push((col as u16, v));
        }
        if self.enter(os, e).is_err() {
            return Ok(false);
        }
        self.implicit_txn(e);
        e.table = ti;
        e.snap = self.snapshot(e);
        let snap = Snap { ts: e.snap, txn: e.txn };
        let l = self.cfg.layout;
        let (access, _, found, cost) = Db::run_find(&self.tables[ti], &l, &cs, None, None, false, self.cfg.write_rows_max + 1, snap);
        let touch = walk(&mut self.pool, &self.tables[ti], &l, &access, &cost, &found.keys, true);
        let changed: Vec<u16> = rs.iter().map(|x| x.0).collect();
        let mut extra = Touch::default();
        for k in found.keys.iter().take(16) {
            let x = self.index_writes(ti, *k, Some(&changed));
            extra.hits += x.hits;
            extra.miss_rand += x.miss_rand;
        }
        self.charged(os, e, &cost, touch);
        e.hits += extra.hits;
        e.misses += extra.misses();
        e.miss_rand += extra.miss_rand;
        if found.matches > self.cfg.write_rows_max {
            return self.too_many(os, e, found.matches);
        }
        e.targets = found.keys;
        e.write = WriteKind::Update { sets: rs, conds: cs };
        e.phase = Phase::Work;
        Ok(true)
    }

    fn do_delete(&mut self, os: &mut OsApi, _id: u64, e: &mut Exec, table: &str, pos: usize, filter: &[Cond]) -> Result<bool, SqlErr> {
        self.stats.deletes += 1;
        let ti = self.table_at(table, pos)?;
        let cs = Db::conds_of(&self.tables[ti], filter, &e.params)?;
        if self.enter(os, e).is_err() {
            return Ok(false);
        }
        self.implicit_txn(e);
        e.table = ti;
        e.snap = self.snapshot(e);
        let snap = Snap { ts: e.snap, txn: e.txn };
        let l = self.cfg.layout;
        let (access, _, found, cost) = Db::run_find(&self.tables[ti], &l, &cs, None, None, false, self.cfg.write_rows_max + 1, snap);
        let touch = walk(&mut self.pool, &self.tables[ti], &l, &access, &cost, &found.keys, true);
        self.charged(os, e, &cost, touch);
        if found.matches > self.cfg.write_rows_max {
            return self.too_many(os, e, found.matches);
        }
        e.targets = found.keys;
        e.write = WriteKind::Delete { conds: cs };
        e.phase = Phase::Work;
        Ok(true)
    }

    // ---------------------------------------------------------------- locks and writes

    /// takes the rest of its locks and writes: Go::On when all are done
    fn locks_go(&mut self, os: &mut OsApi, id: u64, e: &mut Exec) -> Go {
        while e.next < e.targets.len() {
            let key = (e.table as u16, e.targets[e.next]);
            let txn = e.txn;
            let holds = match self.locks.get(&key) {
                None => {
                    self.locks.insert(key, RowLock { holder: txn, queue: VecDeque::new() });
                    if let Some(t) = self.txns.get_mut(&txn) {
                        t.locks.push(key);
                    }
                    true
                }
                Some(l) => l.holder == txn,
            };
            if !holds {
                if self.closes_cycle(txn, key) {
                    self.stats.deadlocks += 1;
                    os.log(4, "db", &format!("Verklemmung: Transaktion {} gibt auf", txn));
                    self.fail_txn(os, core::mem::replace(e, placeholder()), "deadlock", "Verklemmung: diese Transaktion wurde abgebrochen, die andere läuft weiter. Bitte wiederholen.");
                    return Go::Fail;
                }
                let now = os.now();
                self.stats.lock_waits += 1;
                let timer = self.wheel.add(now + self.cfg.lock_timeout, DTimer::LockOut(id));
                if let Some(l) = self.locks.get_mut(&key) {
                    if l.queue.is_empty() {
                        if self.contended == 0 {
                            self.contended_since = now;
                        }
                        self.contended += 1;
                    }
                    l.queue.push_back(Waiter { txn, exec: id, since: now, timer });
                }
                if let Some(t) = self.txns.get_mut(&txn) {
                    t.waits = Some(key);
                }
                return Go::Wait;
            }
            // the row under its lock
            match self.write_row(os, e) {
                Ok(()) => e.next += 1,
                Err((code, text, retry)) => {
                    if retry {
                        self.fail_txn(os, core::mem::replace(e, placeholder()), code, &text);
                    } else {
                        // the statement fails, its transaction goes on
                        self.refuse(os, e.sid, 409, code, &text, 0, false, Some(e.start));
                        self.statement_over(os, e);
                    }
                    return Go::Fail;
                }
            }
        }
        // the rows of FOR UPDATE, as they are under the locks
        if matches!(e.write, WriteKind::Lock) {
            let stmt = e.stmt.clone();
            if let Stmt::Select(s) = &*stmt {
                let snap = Snap { ts: u64::MAX, txn: e.txn };
                match self.columns_of(e.table, &s.what) {
                    Ok(cols) => {
                        let mut b = String::new();
                        self.rows_json(e.table, &e.targets, snap, &cols, &mut b);
                        e.body = b;
                        e.count = Some(e.targets.len() as u64);
                    }
                    Err(x) => {
                        let t = x.line();
                        self.refuse(os, e.sid, 400, "syntax", &t, x.pos, false, Some(e.start));
                        self.statement_over(os, e);
                        return Go::Fail;
                    }
                }
            }
        }
        Go::On
    }

    /// waiting for this lock would close a cycle: who it waits for, who they wait for
    fn closes_cycle(&self, me: u64, key: (u16, u64)) -> bool {
        let mut seen = BTreeSet::new();
        let mut todo: Vec<(u16, u64)> = alloc::vec![key];
        while let Some(k) = todo.pop() {
            let Some(l) = self.locks.get(&k) else { continue };
            for t in core::iter::once(l.holder).chain(l.queue.iter().map(|w| w.txn)) {
                if t == me {
                    return true;
                }
                if seen.insert(t)
                    && let Some(w) = self.txns.get(&t).and_then(|x| x.waits)
                {
                    todo.push(w);
                }
            }
        }
        false
    }

    /// the row at e.next, its lock held: Err(code, text, its transaction ends)
    #[allow(clippy::type_complexity)]
    fn write_row(&mut self, os: &mut OsApi, e: &mut Exec) -> Result<(), (&'static str, String, bool)> {
        let k = e.targets[e.next];
        let ti = e.table;
        let txn = e.txn;
        let iso = self.txns.get(&txn).map_or(self.cfg.iso, |t| t.iso);
        let snap_of_txn = self.txns.get(&txn).and_then(|t| t.snap);
        let t = &self.tables[ti];
        // repeatable read: it may not write over what it could not see
        if iso == Iso::RepeatableRead
            && let Some(s) = snap_of_txn
            && t.newest_ts(k) > s
        {
            return Err(("serialization", String::from("Die Zeile hat sich seit dem Beginn dieser Transaktion geändert: bitte die Transaktion wiederholen."), true));
        }
        let (delta, gone) = match &e.write {
            WriteKind::None | WriteKind::Lock => return Ok(()),
            WriteKind::Insert { sets, .. } => {
                if t.exists(k, Snap { ts: u64::MAX, txn }) {
                    return Err(("duplicate_key", format!("Den Schlüssel {} gibt es schon.", k), false));
                }
                (sets.clone(), false)
            }
            WriteKind::Update { sets, conds } => {
                // read committed: the newest committed values, if they still fit
                if !t.holds_now(k, txn, conds) {
                    return Ok(());
                }
                let cur = t.image(k, Snap { ts: u64::MAX, txn });
                let mut out = Vec::with_capacity(sets.len());
                for (c, v) in sets {
                    out.push((
                        *c,
                        match v {
                            SetVal::Lit(x) => x.clone(),
                            SetVal::Add(n) => match cur.as_ref().map(|i| &i[*c as usize]) {
                                Some(Val::Int(x)) => Val::Int(x.saturating_add(*n)),
                                _ => Val::Int(*n),
                            },
                        },
                    ));
                }
                (out, false)
            }
            WriteKind::Delete { conds } => {
                if !t.holds_now(k, txn, conds) {
                    return Ok(());
                }
                (Vec::new(), true)
            }
        };
        // into the log (the disk gets it with the next flush), then a version
        let bytes = self.tables[ti].spec.row_bytes + REC_HEAD;
        self.lsn += bytes;
        let lsn = self.lsn;
        self.stats.wal_bytes += bytes;
        self.wal_rec(WalRec { lsn, txn, kind: WalKind::Row { table: ti as u16, key: k, vals: delta.clone(), gone } });
        self.tables[ti].write(k, txn, lsn, delta, gone);
        if self.tables[ti].rows.get(&k).is_some_and(|r| r.vers.len() >= 2) {
            self.old.insert((ti as u16, k));
        }
        if let Some(tx) = self.txns.get_mut(&txn) {
            tx.writes.push((ti as u16, k));
        }
        e.wrote += 1;
        if !self.ckpt_armed {
            self.ckpt_armed = true;
            let at = os.now() + self.cfg.checkpoint;
            self.wheel.add(at, DTimer::Checkpoint);
        }
        Ok(())
    }

    fn wal_rec(&mut self, r: WalRec) {
        if self.wal_tail.len() >= self.cfg.wal_tail.max(1)
            && let Some(old) = self.wal_tail.pop_front()
        {
            self.tail_floor = old.lsn;
        }
        self.wal_tail.push_back(r);
    }

    /// a statement is over without a commit of its own
    fn statement_over(&mut self, os: &mut OsApi, e: &mut Exec) {
        if let Some(s) = e.snap_held.take() {
            self.snap_drop(s);
        }
        if e.txn == 0 {
            return;
        }
        if e.implicit {
            let t = e.txn;
            self.abort(os, t);
            return;
        }
        let now = os.now();
        let idle = self.cfg.txn_idle;
        let txn = e.txn;
        if let Some(old) = self.txns.get_mut(&txn).map(|t| t.idle_timer.take()) {
            if let Some(o) = old {
                self.wheel.cancel(o);
            }
            let timer = self.wheel.add(now + idle, DTimer::Idle(txn));
            if let Some(t) = self.txns.get_mut(&txn) {
                t.busy = false;
                t.idle_timer = Some(timer);
            }
        }
    }

    /// a statement ran to its end
    fn finish(&mut self, os: &mut OsApi, id: u64, mut e: Exec) {
        if e.implicit {
            // its own transaction ends with it: its answer waits for that
            self.commit(os, id, e);
            return;
        }
        let body = self.answer_of(&e, os.now());
        self.reply(os, e.sid, 200, body, Some(e.start));
        self.statement_over(os, &mut e);
    }

    /// the JSON of a statement's end
    fn answer_of(&self, e: &Exec, now: Ns) -> String {
        let mut b = String::with_capacity(e.body.len() + 160);
        b.push_str("{\"ok\":true");
        match &*e.stmt {
            Stmt::Select(s) if !matches!(s.what, What::Count) => {
                b.push_str(",\"rows\":[");
                b.push_str(&e.body);
                b.push(']');
            }
            Stmt::Select(_) => {}
            _ => b.push_str(&format!(",\"changed\":{}", e.wrote)),
        }
        if let Some(c) = e.count {
            b.push_str(&format!(",\"count\":{}", c));
        }
        if e.txn != 0 && !e.implicit {
            b.push_str(&format!(",\"txn\":{}", e.txn));
        }
        if let Some(d) = e.drawn {
            b.push_str(&format!(",\"drawn\":{}", d));
        }
        if e.truncated {
            b.push_str(",\"truncated\":true");
        }
        b.push_str(&format!(",\"figures\":{{\"rows_examined\":{},\"pages_hit\":{},\"pages_missed\":{},\"lock_wait_us\":{},\"us\":{}}}}}\n", e.examined, e.hits, e.misses, e.lock_wait / US, now.saturating_sub(e.start) / US));
        b
    }

    // ---------------------------------------------------------------- commit

    /// what a commit answers: COMMIT its own words, a statement on its own its rows and figures
    fn final_body(&self, e: &mut Exec, now: Ns) -> String {
        if e.implicit { self.answer_of(e, now) } else { core::mem::take(&mut e.body) }
    }

    /// a transaction ends well: `e` answers with its body when that is final
    fn commit(&mut self, os: &mut OsApi, id: u64, mut e: Exec) {
        if let Some(s) = e.snap_held.take() {
            self.snap_drop(s);
        }
        let txn = e.txn;
        let wrote = self.txns.get(&txn).is_some_and(|t| !t.writes.is_empty());
        if !wrote {
            self.finalize(os, txn, 0);
            let body = self.final_body(&mut e, os.now());
            self.reply(os, e.sid, 200, body, Some(e.start));
            return;
        }
        self.lsn += REC_HEAD;
        let lsn = self.lsn;
        self.stats.wal_bytes += REC_HEAD;
        self.wal_rec(WalRec { lsn, txn, kind: WalKind::Commit });
        if self.cfg.durable {
            e.phase = Phase::Flush;
            self.execs.insert(id, e);
            self.pending.push(id);
            if self.flushing.is_none() {
                self.start_flush(os);
            }
            return;
        }
        // "relaxed": it counts at once, the disk follows
        let writes = self.txns.get(&txn).map_or(Vec::new(), |t| t.writes.clone());
        self.finalize(os, txn, lsn);
        self.acked.push_back((lsn, writes));
        let body = self.final_body(&mut e, os.now());
        self.reply(os, e.sid, 200, body, Some(e.start));
        if !self.tick_armed {
            self.tick_armed = true;
            let at = os.now() + self.cfg.flush;
            self.wheel.add(at, DTimer::FlushTick);
        }
    }

    /// the log up to now goes to the disk, with the commits waiting for it
    fn start_flush(&mut self, os: &mut OsApi) {
        let group = core::mem::take(&mut self.pending);
        let to = self.lsn;
        let bytes = to.saturating_sub(self.flushed).max(1);
        match os.disk_write(bytes, bytes as i64) {
            Some(at) => {
                self.wal_disk += bytes;
                self.stats.wal_flushes += 1;
                self.flushing = Some((to, group));
                self.wheel.add(at, DTimer::FlushDone);
            }
            None => {
                // the disk is full: their commits fail
                for id in group {
                    if let Some(mut e) = self.execs.remove(&id) {
                        let t = e.txn;
                        self.abort(os, t);
                        self.refuse(os, e.sid, 507, "disk_full", "Die Platte ist voll: das Log passt nicht mehr darauf.", 0, false, Some(e.start));
                        e.body.clear();
                    }
                }
            }
        }
    }

    fn flush_done(&mut self, os: &mut OsApi) {
        let Some((to, group)) = self.flushing.take() else {
            return;
        };
        self.flushed = to;
        // what was answered without waiting is safe now
        while self.acked.front().is_some_and(|a| a.0 <= to) {
            self.acked.pop_front();
        }
        let now = os.now();
        self.recent_flush.add(now, 1, group.len() as u64);
        self.stats.commits_flushed += group.len() as u64;
        for id in group {
            if let Some(mut e) = self.execs.remove(&id) {
                let txn = e.txn;
                self.finalize(os, txn, to);
                let body = self.final_body(&mut e, now);
                self.reply(os, e.sid, 200, body, Some(e.start));
            }
        }
        if !self.pending.is_empty() {
            self.start_flush(os);
        }
        // what waited for the disk may go now
        self.prune_some(256);
        self.bound();
    }

    /// its versions count from here on, its locks go
    fn finalize(&mut self, os: &mut OsApi, txn: u64, lsn: u64) {
        let Some(t) = self.txns.remove(&txn) else {
            return;
        };
        if let Some(timer) = t.idle_timer {
            self.wheel.cancel(timer);
        }
        if let Some(s) = t.snap {
            self.snap_drop(s);
        }
        if !t.writes.is_empty() {
            self.clock += 1;
            self.stats.commits += 1;
            let ts = self.clock;
            let horizon = self.horizon();
            let durable = self.flushed;
            for (ti, k) in &t.writes {
                if let Some(tb) = self.tables.get_mut(*ti as usize) {
                    tb.commit_row(*k, txn, ts);
                    // (a crash keeps the transaction as a whole or not at all: all of it at its commit's place)
                    if let Some(v) = tb.rows.get_mut(k).and_then(|r| r.vers.last_mut())
                        && v.ts == ts
                    {
                        v.lsn = lsn;
                    }
                    tb.prune_row(*k, horizon, durable);
                    if tb.rows.get(k).is_none_or(|r| r.vers.len() < 2) {
                        self.old.remove(&(*ti, *k));
                    }
                }
            }
        } else if !t.implicit {
            self.stats.commits += 1;
        }
        self.release(os, &t.locks);
        self.bound();
    }

    /// a transaction ends badly: what it wrote goes, its locks too
    fn abort(&mut self, os: &mut OsApi, txn: u64) {
        let Some(t) = self.txns.remove(&txn) else {
            return;
        };
        if let Some(timer) = t.idle_timer {
            self.wheel.cancel(timer);
        }
        if let Some(s) = t.snap {
            self.snap_drop(s);
        }
        for (ti, k) in &t.writes {
            if let Some(tb) = self.tables.get_mut(*ti as usize) {
                tb.undo_row(*k, txn);
                if tb.rows.get(k).is_none_or(|r| r.vers.len() < 2) {
                    self.old.remove(&(*ti, *k));
                }
            }
        }
        self.release(os, &t.locks);
    }

    /// a statement's transaction ends with an error that says so
    fn fail_txn(&mut self, os: &mut OsApi, mut e: Exec, code: &str, text: &str) {
        if let Some(s) = e.snap_held.take() {
            self.snap_drop(s);
        }
        let t = e.txn;
        self.abort(os, t);
        self.refuse(os, e.sid, 409, code, text, 0, true, Some(e.start));
    }

    /// these locks go: the next waiter of each gets its lock
    fn release(&mut self, os: &mut OsApi, list: &[(u16, u64)]) {
        let now = os.now();
        for key in list {
            let Some(l) = self.locks.get_mut(key) else { continue };
            let had_waiters = !l.queue.is_empty();
            match l.queue.pop_front() {
                None => {
                    self.locks.remove(key);
                }
                Some(w) => {
                    l.holder = w.txn;
                    let empty = l.queue.is_empty();
                    self.wheel.cancel(w.timer);
                    let waited = now.saturating_sub(w.since);
                    self.stats.lock_wait_ns += waited;
                    if let Some(t) = self.txns.get_mut(&w.txn) {
                        t.locks.push(*key);
                        t.waits = None;
                    }
                    if let Some(x) = self.execs.get_mut(&w.exec) {
                        x.lock_wait += waited;
                    }
                    self.ready.push_back(w.exec);
                    if had_waiters && empty {
                        self.uncontend(now);
                    }
                }
            }
        }
    }

    fn uncontend(&mut self, now: Ns) {
        self.contended = self.contended.saturating_sub(1);
        if self.contended == 0 {
            self.contended_ns += now.saturating_sub(self.contended_since);
        }
    }

    fn lock_out(&mut self, os: &mut OsApi, id: u64) {
        let Some(e) = self.execs.remove(&id) else {
            return;
        };
        let now = os.now();
        let key = self.txns.get(&e.txn).and_then(|t| t.waits);
        if let Some(k) = key
            && let Some(l) = self.locks.get_mut(&k)
        {
            let before = l.queue.len();
            l.queue.retain(|w| w.exec != id);
            if before > 0 && l.queue.is_empty() {
                self.uncontend(now);
            }
        }
        self.stats.lock_timeouts += 1;
        self.fail_txn(os, e, "lock_timeout", "Die Sperre kam nicht rechtzeitig: die Transaktion wurde abgebrochen. Bitte wiederholen.");
    }

    fn idle_out(&mut self, os: &mut OsApi, txn: u64) {
        if !self.txns.get(&txn).is_some_and(|t| !t.busy) {
            return;
        }
        if let Some(t) = self.txns.get_mut(&txn) {
            t.idle_timer = None;
        }
        self.stats.idle_aborts += 1;
        os.log(4, "db", &format!("Transaktion {} war zu lange still und wurde abgebrochen", txn));
        self.abort(os, txn);
    }

    /// the bound on touched rows
    fn bound(&mut self) {
        let max = self.cfg.touched_max;
        if self.tables.is_empty() {
            return;
        }
        let (h, d) = (self.horizon(), self.flushed);
        let locks = core::mem::take(&mut self.locks);
        let mut tries = 0;
        while self.touched() > max && tries < 64 {
            tries += 1;
            let i = self.forget_at % self.tables.len();
            self.forget_at += 1;
            let locked = |t: u16, k: u64| locks.contains_key(&(t, k));
            if self.tables[i].forget_one(h, d, &locked) {
                self.stats.forgotten += 1;
                tries = 0;
            }
        }
        self.locks = locks;
    }

    // ---------------------------------------------------------------- the checkpoint

    fn checkpoint(&mut self, os: &mut OsApi) {
        self.ckpt_armed = false;
        self.ckpt_to = self.flushed;
        self.ckpt_left = self.pool.dirty_pages() * self.cfg.layout.page;
        self.pool.clean_all();
        self.ckpt_step(os);
    }

    fn ckpt_step(&mut self, os: &mut OsApi) {
        if self.ckpt_left > 0 {
            let n = self.ckpt_left.min(CKPT_CHUNK);
            self.ckpt_left -= n;
            if let Some(at) = os.disk_write(n, 0) {
                self.wheel.add(at, DTimer::CkptStep);
                return;
            }
        }
        // all it had is on the disk: the log before it is free
        let free = self.ckpt_to.saturating_sub(self.ckpt).min(self.wal_disk);
        os.disk_free(free);
        self.wal_disk -= free;
        self.ckpt = self.ckpt_to;
        self.stats.checkpoints += 1;
    }

    // ---------------------------------------------------------------- texts

    fn health(&self) -> String {
        format!("{{\"status\":\"{}\",\"role\":\"primary\",\"lsn\":{},\"flushed_lsn\":{},\"checkpoint_lsn\":{},\"recoveries\":{},\"recovery_ms\":{}}}\n", if self.recovering.is_some() { "recovering" } else { "ok" }, self.lsn, self.flushed, self.ckpt, self.stats.recoveries, self.stats.recovery_ms)
    }

    fn stats_json(&self, now: Ns) -> String {
        let mut b = String::from("{");
        for (i, m) in self.metrics_at(now).iter().enumerate() {
            if i > 0 {
                b.push(',');
            }
            b.push_str(&format!("\"{}\":{}", m.name, m.value));
        }
        b.push_str(",\"tables\":[");
        for (i, t) in self.tables().iter().enumerate() {
            if i > 0 {
                b.push(',');
            }
            b.push_str(&format!("{{\"name\":\"{}\",\"rows\":{},\"touched\":{},\"heap_pages\":{},\"pool_pages\":{}}}", t.name, t.rows, t.touched, t.heap_pages, t.held_pages));
        }
        b.push_str("]}\n");
        b
    }

    /// its settings in the units of its config (svcform.rs)
    pub(crate) fn settings(&self) -> Value {
        use crate::svcform::{mem_into, ms_of, us_of, word};
        let c = &self.cfg;
        let mut o = agent_core::jmap::empty_object();
        o.insert("port", Value::u64(c.port as u64));
        o.insert("pool_mb", Value::u64(c.pool_bytes >> 20));
        o.insert("page_kb", Value::u64(c.layout.page >> 10));
        o.insert("fanout", Value::u64(c.layout.fanout));
        o.insert("max_conns", Value::u64(c.max_conns as u64));
        o.insert("durability", word(if c.durable { "full" } else { "relaxed" }));
        o.insert("flush_ms", ms_of(c.flush));
        o.insert("isolation", word(c.iso.name()));
        o.insert("lock_timeout_ms", ms_of(c.lock_timeout));
        o.insert("txn_idle_ms", ms_of(c.txn_idle));
        o.insert("checkpoint_s", Value::u64(c.checkpoint / SEC));
        o.insert("touched_max", Value::u64(c.touched_max));
        o.insert("stmt_us", us_of(c.stmt_cpu));
        o.insert("page_us", us_of(c.page_cpu));
        o.insert("row_us", us_of(c.row_cpu));
        o.insert("write_us", us_of(c.write_cpu));
        o.insert("redo_mbps", Value::u64(c.redo_bps / 1_000_000));
        o.insert("max_rows", Value::u64(c.max_rows));
        o.insert("write_rows_max", Value::u64(c.write_rows_max));
        o.insert("seed", Value::u64(c.seed));
        mem_into(&mut o, &self.mem);
        o
    }

    /// a start: what was lost is gone, the pool is empty; the log since the last checkpoint is read and replayed
    fn start_up(&mut self, os: &mut OsApi) {
        self.metered = os.metered();
        self.pool.clear();
        self.recovering = None;
        let since = self.flushed.saturating_sub(self.ckpt);
        if self.started && since > 0 {
            let read = os.disk_read(since);
            let replay = (since as u128 * SEC as u128 / self.cfg.redo_bps.max(1) as u128) as Ns;
            let at = read + replay;
            self.recovering = Some(at);
            self.wheel.add(at, DTimer::Recovered);
            self.stats.recoveries += 1;
            os.log(5, "db", &format!("liest {} Log seit dem letzten Prüfpunkt", crate::disk::size_text(since)));
        } else if self.started {
            self.stats.recoveries += 1;
            self.stats.recovery_ms = 0;
        }
        self.started = true;
    }

    fn recovered(&mut self, os: &mut OsApi) {
        if let Some(_at) = self.recovering.take() {
            let free = self.flushed.saturating_sub(self.ckpt).min(self.wal_disk);
            os.disk_free(free);
            self.wal_disk -= free;
            self.ckpt = self.flushed;
            let began = self.recover_began;
            self.stats.recovery_ms = os.now().saturating_sub(began) / MS;
            os.log(5, "db", "Wiederherstellung fertig");
        }
    }
}

/// how taking locks went
enum Go {
    On,
    Wait,
    Fail,
}

/// what stands in the place of a statement handed on
fn placeholder() -> Exec {
    Exec {
        sid: 0,
        stmt: Arc::new(Stmt::Commit),
        params: Vec::new(),
        drawn: None,
        txn: 0,
        implicit: false,
        start: 0,
        phase: Phase::Flush,
        cpu_left: 0,
        miss_rand: 0,
        miss_seq: 0,
        turn: false,
        snap_held: None,
        snap: 0,
        table: 0,
        targets: Vec::new(),
        next: 0,
        write: WriteKind::None,
        wrote: 0,
        body: String::new(),
        count: None,
        truncated: false,
        examined: 0,
        hits: 0,
        misses: 0,
        lock_wait: 0,
    }
}

impl Service for Db {
    fn kind(&self) -> &'static str {
        "db"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.seen = os.now();
        self.serve.cfg.port = self.cfg.port;
        self.serve.stop();
        self.wheel.clear();
        if let Some((lat, bps, iops, cap)) = self.disk_cfg {
            os.disk_setup(lat, bps, iops, cap);
        }
        // the tables stand on the disk from the first start on
        if !self.reserved {
            let l = self.cfg.layout;
            let need: u64 = self.tables.iter().map(|t| t.disk_bytes(&l)).sum();
            if os.disk_reserve(need) {
                self.reserved = true;
                self.no_disk = None;
            } else {
                let cap = os.disk().map_or(0, |d| d.capacity);
                self.no_disk = Some((need, cap));
                os.log(3, "db", &format!("Die Tabellen brauchen {}, die Platte hat {}", crate::disk::size_text(need), crate::disk::size_text(cap)));
            }
        }
        self.recover_began = self.seen;
        self.start_up(os);
        self.serve.start(os, "db");
        self.wheel.arm(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        self.seen = os.now();
        self.metered = os.metered();
        if let SvcEvent::Timer(K_OWN) = ev {
            self.wheel.fired();
            let now = os.now();
            for (_, t) in self.wheel.due(now) {
                match t {
                    DTimer::Step(id) => self.advance(os, id),
                    DTimer::LockOut(id) => self.lock_out(os, id),
                    DTimer::Idle(id) => self.idle_out(os, id),
                    DTimer::FlushDone => self.flush_done(os),
                    DTimer::FlushTick => {
                        self.tick_armed = false;
                        // the relaxed flush: the log so far, nobody waits for it
                        if self.flushing.is_none() && self.lsn > self.flushed {
                            self.start_flush(os);
                        }
                        if self.lsn > self.flushed || !self.acked.is_empty() {
                            self.tick_armed = true;
                            self.wheel.add(now + self.cfg.flush, DTimer::FlushTick);
                        }
                    }
                    DTimer::Checkpoint => self.checkpoint(os),
                    DTimer::CkptStep => self.ckpt_step(os),
                    DTimer::Recovered => self.recovered(os),
                    DTimer::Built(id) => self.build_step(os, id),
                }
                self.pump(os);
            }
        } else if self.serve.on(&ev, os) {
            for r in self.serve.take() {
                self.request(os, r);
            }
        }
        self.pump(os);
        self.wheel.arm(os);
    }
    fn stop(&mut self, _os: &mut OsApi) {
        // a kill: open transactions and what the log did not keep go
        let durable = self.flushed;
        let mut rows: Vec<(u16, u64)> = Vec::new();
        for t in self.txns.values() {
            rows.extend_from_slice(&t.writes);
        }
        self.stats.lost_commits += self.acked.iter().filter(|a| a.0 > durable).count() as u64;
        for (_, w) in &self.acked {
            rows.extend_from_slice(w);
        }
        rows.sort_unstable();
        rows.dedup();
        for (ti, k) in rows {
            if let Some(t) = self.tables.get_mut(ti as usize) {
                t.crash_row(k, durable);
            }
        }
        self.lsn = self.flushed;
        self.wal_tail.retain(|r| r.lsn <= durable);
        self.pool.clear();
        self.acked.clear();
        self.txns.clear();
        self.execs.clear();
        self.ready.clear();
        self.locks.clear();
        self.snaps.clear();
        self.old.clear();
        self.pending.clear();
        self.flushing = None;
        self.tick_armed = false;
        self.ckpt_armed = false;
        self.ckpt_left = 0;
        self.builds.clear();
        self.contended = 0;
        self.recovering = None;
        self.serve.stop();
        self.wheel.clear();
    }
    fn metrics(&self) -> Vec<Metric> {
        self.metrics_at(self.seen)
    }
    fn metrics_at(&self, now: Ns) -> Vec<Metric> {
        let s = &self.stats;
        let v = |x: u64| x as i64;
        let (h, m) = self.recent_pool.sums(now);
        let (fl, gr) = self.recent_flush.sums(now);
        let used = self.pool.used();
        alloc::vec![
            Metric::new("requests", v(s.requests)),
            Metric::new("statements", v(s.statements)),
            Metric::new("selects", v(s.selects)),
            Metric::new("inserts", v(s.inserts)),
            Metric::new("updates", v(s.updates)),
            Metric::new("deletes", v(s.deletes)),
            Metric::new("commits", v(s.commits)),
            Metric::new("rollbacks", v(s.rollbacks)),
            Metric::new("errors", v(s.errors)),
            Metric::new("syntax_errors", v(s.syntax_errors)),
            Metric::new("conflicts", v(s.conflicts)),
            Metric::new("deadlocks", v(s.deadlocks)),
            Metric::new("lock_waits", v(s.lock_waits)),
            Metric::new("lock_timeouts", v(s.lock_timeouts)),
            Metric::new("idle_aborts", v(s.idle_aborts)),
            Metric::new("refused", v(s.refused)),
            Metric::new("active", self.serve.conns() as i64),
            Metric::new("inflight", self.execs.len() as i64),
            Metric::new("txns", self.txns.len() as i64),
            Metric::new("p50_us", (self.lat.p50(now).unwrap_or(0) / 1000) as i64),
            Metric::new("p95_us", (self.lat.p95(now).unwrap_or(0) / 1000) as i64),
            Metric::new("p99_us", (self.lat.p99(now).unwrap_or(0) / 1000) as i64),
            Metric::new("pool_hits", v(self.pool.hits)),
            Metric::new("pool_misses", v(self.pool.misses)),
            Metric::new("pool_hit_pm", if h + m == 0 { 0 } else { (h * 1000 / (h + m)) as i64 }),
            Metric::new("pool_use_pm", (used * 1000 / self.pool.cap().max(1)) as i64),
            Metric::new("pool_bytes", v(used * self.cfg.layout.page)),
            Metric::new("dirty_pages", v(self.pool.dirty_pages())),
            Metric::new("rows_examined", v(s.rows_examined)),
            Metric::new("pages_read", v(s.pages_read)),
            Metric::new("wal_flushes", v(s.wal_flushes)),
            Metric::new("group_pm", if fl == 0 { 0 } else { (gr * 1000 / fl) as i64 }),
            Metric::new("wal_bytes", v(s.wal_bytes)),
            Metric::new("lsn", v(self.lsn)),
            Metric::new("flushed_lsn", v(self.flushed)),
            Metric::new("lost_commits", v(s.lost_commits)),
            Metric::new("recoveries", v(s.recoveries)),
            Metric::new("recovery_ms", v(s.recovery_ms)),
            Metric::new("touched_rows", v(self.touched())),
            Metric::new("forgotten", v(s.forgotten)),
            Metric::new("old_versions", self.old.iter().map(|(t, k)| self.tables.get(*t as usize).and_then(|x| x.rows.get(k)).map_or(0, |r| r.vers.len().saturating_sub(1))).sum::<usize>() as i64),
        ]
    }
    fn metric_kind(&self, name: &str) -> MetricKind {
        match name {
            "txns" | "pool_bytes" | "dirty_pages" | "touched_rows" | "old_versions" | "lsn" | "flushed_lsn" => MetricKind::Gauge,
            n => metric_kind_of(n),
        }
    }
    fn windows(&self) -> Vec<LatWindow<'_>> {
        alloc::vec![LatWindow::new(&self.lat, LAT)]
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        if !self.serve.listening() {
            return msg!("Port {} ist nicht offen", self.cfg.port);
        }
        if let Some((need, cap)) = self.no_disk {
            return msg!("Die Tabellen brauchen {}, die Platte hat {}", crate::disk::size_text(need), crate::disk::size_text(cap));
        }
        if self.recovering.is_some() {
            return msg!("Port {}: liest das Log seit dem letzten Prüfpunkt", self.cfg.port);
        }
        let s = &self.stats;
        let (h, m) = self.recent_pool.sums(self.seen);
        let hit = if h + m == 0 { 0 } else { h * 1000 / (h + m) };
        if !self.metered {
            return msg!("Port {}: {} Anweisungen, Treffer im Puffer {} ‰, {} Transaktionen offen. Ohne Hardware kostet die CPU nichts: diese Zahlen sagen wenig.", self.cfg.port, s.statements, hit, self.txns.len());
        }
        if s.forgotten > 0 {
            return msg!("Port {}: {} Anweisungen, Treffer im Puffer {} ‰, {} Transaktionen offen, {} geänderte Zeilen vergessen", self.cfg.port, s.statements, hit, self.txns.len(), s.forgotten);
        }
        msg!("Port {}: {} Anweisungen, Treffer im Puffer {} ‰, {} Transaktionen offen", self.cfg.port, s.statements, hit, self.txns.len())
    }
    fn clone_box(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn footprint(&self) -> usize {
        // the pool's structure, the touched rows, what is on its way; the table's descriptions are shared
        4096 + self.pool.bytes() + self.tables.iter().map(|t| t.bytes()).sum::<usize>() + self.execs.values().map(|e| 256 + e.body.len() + e.targets.len() * 8).sum::<usize>() + (self.txns.len() + self.locks.len()) * 128 + self.wal_tail.len() * 96 + self.lat.bytes()
    }
    fn mem(&self) -> u64 {
        self.mem.bytes(self.serve.conns() as u64, self.pool.used() * self.cfg.layout.page)
    }
    fn lock_busy(&self) -> Ns {
        self.contended_ns + if self.contended > 0 { self.seen.saturating_sub(self.contended_since) } else { 0 }
    }
    fn reconfigure(&mut self, c: &Value, os: Option<&mut OsApi>) -> Result<(), Msg> {
        let fresh = from_def_msg(c)?;
        let Some(f) = fresh.as_any().downcast_ref::<Db>() else {
            return Err(msg!("Der Dienst \u{201e}{}\u{201c} ist von anderer Art.", "db"));
        };
        if self.cfg.port != f.cfg.port || self.cfg.layout != f.cfg.layout || self.cfg.seed != f.cfg.seed || self.tables_cfg != f.tables_cfg || self.disk_cfg != f.disk_cfg {
            return Err(msg!("Port, Seitengröße und Tabellen ändern sich nur mit einem Neustart."));
        }
        let pool_was = self.cfg.pool_bytes;
        let keep = self.cfg.clone();
        self.cfg = DbCfg { port: keep.port, seed: keep.seed, layout: keep.layout, ..f.cfg.clone() };
        self.mem = f.mem;
        if self.cfg.pool_bytes != pool_was {
            self.pool.resize(self.cfg.pool_bytes / self.cfg.layout.page);
        }
        if let Some(os) = os {
            self.pump(os);
            self.wheel.arm(os);
        }
        Ok(())
    }
}

impl Db {
    /// statements that may go on now
    fn pump(&mut self, os: &mut OsApi) {
        while let Some(id) = self.ready.pop_front() {
            self.advance(os, id);
        }
    }
}

// ---------------------------------------------------------------- from a project's definition

fn word_of(c: &Value, key: &str) -> Option<String> {
    c.get(key).and_then(|v| v.as_str()).map(String::from)
}

/// a column of a config
fn column_of(t: &str, v: &Value) -> Result<ColDef, Msg> {
    let parts = |s: &str| -> Result<ColDef, Msg> {
        let mut p = s.split(':');
        let name = p.next().unwrap_or("").trim();
        let ty = p.next().unwrap_or("int").trim();
        let distinct = match p.next() {
            Some(n) => n.trim().parse::<u64>().map_err(|_| msg!("Tabelle {}: \u{201e}{}\u{201c} ist keine Spalte (name, name:text, name:int:1000).", t, s))?,
            None => 0,
        };
        if name.is_empty() || (ty != "int" && ty != "text") {
            return Err(msg!("Tabelle {}: \u{201e}{}\u{201c} ist keine Spalte (name, name:text, name:int:1000).", t, s));
        }
        Ok(column(name, ty == "text", distinct))
    };
    match v {
        Value::String(s) => parts(s),
        Value::Object(_) => {
            let name = word_of(v, "name").unwrap_or_default();
            let ty = word_of(v, "type").unwrap_or_else(|| String::from("int"));
            if name.is_empty() || (ty != "int" && ty != "text") {
                return Err(msg!("Tabelle {}: eine Spalte braucht einen Namen und den Typ int oder text.", t));
            }
            Ok(column(&name, ty == "text", v.get("distinct").and_then(|x| x.as_u64()).unwrap_or(0)))
        }
        _ => Err(msg!("Tabelle {}: eine Spalte ist ein Text oder ein Objekt.", t)),
    }
}

/// a table of a config
fn table_of(v: &Value) -> Result<TableSpec, Msg> {
    let name = word_of(v, "name").unwrap_or_default();
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') || name.as_bytes()[0].is_ascii_digit() {
        return Err(msg!("\u{201e}{}\u{201c} geht nicht als Name einer Tabelle: Buchstaben, Ziffern und _.", name));
    }
    let rows = v.get("rows").and_then(|x| x.as_u64()).unwrap_or(0);
    if rows == 0 || rows > 1 << 40 {
        return Err(msg!("Tabelle {}: \u{201e}rows\u{201c} nimmt 1 bis 2^40 Zeilen.", name));
    }
    let row_bytes = v.get("row_bytes").and_then(|x| x.as_u64()).unwrap_or(200);
    if row_bytes == 0 || row_bytes > 1 << 20 {
        return Err(msg!("Tabelle {}: \u{201e}row_bytes\u{201c} nimmt 1 Byte bis 1 MiB.", name));
    }
    let dist = match word_of(v, "dist").as_deref().unwrap_or("uniform") {
        "uniform" => Dist::Uniform,
        "zipf" => Dist::Zipf,
        d => return Err(msg!("Tabelle {}: \u{201e}dist\u{201c} kennt {} nicht (uniform, zipf).", name, d)),
    };
    let zipf_s = v.get("zipf_s").and_then(|x| decimal(x, 1000)).unwrap_or(1000);
    if zipf_s > 5000 {
        return Err(msg!("Tabelle {}: \u{201e}zipf_s\u{201c} nimmt 0 bis 5.", name));
    }
    let key = word_of(v, "key").unwrap_or_else(|| String::from("id"));
    let mut cols = Vec::new();
    for c in v.get("columns").and_then(|x| x.as_array()).unwrap_or(&[]) {
        let d = column_of(&name, c)?;
        if !d.name.eq_ignore_ascii_case(&key) {
            cols.push(d);
        }
    }
    let mut indexes = Vec::new();
    for i in v.get("indexes").and_then(|x| x.as_array()).unwrap_or(&[]) {
        let n = i.as_str().unwrap_or("");
        if !n.eq_ignore_ascii_case(&key) && !cols.iter().any(|c: &ColDef| c.name.eq_ignore_ascii_case(n)) {
            return Err(msg!("Tabelle {}: den Index {} gibt es nicht als Spalte.", name, n));
        }
        indexes.push(String::from(n));
    }
    Ok(TableSpec { name, key, rows, row_bytes, dist, zipf_s: zipf_s as u32, cols, indexes })
}

/// a database from its definition in a project file (svcact.rs); the reason as a message
pub fn from_def_msg(c: &Value) -> Result<Box<dyn Service>, Msg> {
    let num = |k: &str, d: u64| c.get(k).and_then(|v| v.as_u64()).unwrap_or(d);
    let ms = |k: &str, d: u64| c.get(k).and_then(|v| decimal(v, MS)).unwrap_or(d * MS);
    let us = |k: &str, d: Ns| c.get(k).and_then(|v| decimal(v, US)).unwrap_or(d);
    let def = DbCfg::default();
    let list = c.get("tables").and_then(|v| v.as_array()).unwrap_or(&[]);
    if list.is_empty() {
        return Err(msg!("\u{201e}tables\u{201c} fehlt: eine Datenbank braucht mindestens eine Tabelle."));
    }
    let mut tables = Vec::with_capacity(list.len());
    for t in list {
        let s = table_of(t)?;
        if tables.iter().any(|x: &TableSpec| x.name.eq_ignore_ascii_case(&s.name)) {
            return Err(msg!("Die Tabelle {} steht zweimal da.", s.name.clone()));
        }
        tables.push(s);
    }
    let port = match num("port", DB_PORT as u64) {
        p @ 1..=65535 => p as u16,
        p => return Err(msg!("Port {} gibt es nicht.", p)),
    };
    let page_kb = num("page_kb", 8);
    if !matches!(page_kb, 1 | 2 | 4 | 8 | 16 | 32 | 64) {
        return Err(msg!("\u{201e}page_kb\u{201c} nimmt 1, 2, 4, 8, 16, 32 oder 64."));
    }
    let durable = match word_of(c, "durability").as_deref().unwrap_or("full") {
        "full" => true,
        "relaxed" => false,
        d => return Err(msg!("\u{201e}durability\u{201c} kennt {} nicht (full, relaxed).", d)),
    };
    let iso = match word_of(c, "isolation") {
        Some(w) => Iso::parse(&w).ok_or_else(|| msg!("\u{201e}isolation\u{201c} kennt {} nicht (read committed, repeatable read).", w))?,
        None => def.iso,
    };
    let cfg = DbCfg {
        port,
        seed: num("seed", def.seed),
        pool_bytes: c.get("pool_mb").and_then(|v| decimal(v, 1 << 20)).unwrap_or(def.pool_bytes).max(1 << 20),
        layout: Layout::new(page_kb << 10, num("fanout", 0)),
        max_conns: num("max_conns", def.max_conns as u64).clamp(1, 1_000_000) as u32,
        durable,
        flush: ms("flush_ms", 200).max(MS),
        iso,
        lock_timeout: ms("lock_timeout_ms", 5000).max(MS),
        txn_idle: ms("txn_idle_ms", 30_000).max(MS),
        checkpoint: num("checkpoint_s", 30).max(1) * SEC,
        touched_max: num("touched_max", def.touched_max).max(16),
        stmt_cpu: us("stmt_us", def.stmt_cpu),
        page_cpu: us("page_us", def.page_cpu),
        row_cpu: us("row_us", def.row_cpu),
        write_cpu: us("write_us", def.write_cpu),
        redo_bps: c.get("redo_mbps").and_then(|v| decimal(v, 1_000_000)).unwrap_or(def.redo_bps).max(1),
        max_rows: num("max_rows", def.max_rows).clamp(1, 1_000_000),
        write_rows_max: num("write_rows_max", def.write_rows_max).clamp(1, 1_000_000),
        wal_tail: num("wal_tail", def.wal_tail as u64).clamp(16, 1 << 20) as usize,
    };
    let mut d = Db::new(cfg, tables);
    d.mem = crate::res::SvcMem::of(c, 64, 256);
    d.disk_cfg = crate::disk::setup_of(c);
    d.tables_cfg = c.get("tables").cloned();
    Ok(Box::new(d))
}

// ---------------------------------------------------------------- the engine side

impl Engine {
    /// a database's view for the lab
    pub fn db_view(&self, dev: u32, svc: &str) -> Option<DbView> {
        Some(self.service::<Db>(dev, svc)?.view(self.now))
    }
}
