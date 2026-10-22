// scriptsvc -- scripts as services (kind "script"): a .jfs program (lib/script) runs in a
// virtual host, driven by the events it asked for, and the fuel it uses is the CPU time of a
// single-threaded program
//
//   config (JSON; all but the source may be left out; the config without "source" is the
//   script's `config`, so {"port": 7000} gives config.port):
//     {"source": "…" or ["line", …], "ns_per_op": 5, "slice": 100000, "queue": 1000,
//      "stop_on_error": false, "limits": {"fuel": 10000000, "heap_kb": 16384, "depth": 500,
//      "stack": 100000}, "disk": {"latency_us": 100, "mbps": 500, "iops": 50000,
//      "capacity_mb": 1024}}   (disk: the device's disk, as for kv)
//   from_def(&config) / from_def_msg   the service of a project's definition (svcact.rs); the
//                        refusal says what is wrong, a syntax error with its line and column
//   ScriptService::view(now) / errors() / output() / source() / load(now, secs)
//                        for the shell (script: cli/scripts.rs), top and the app;
//                        Engine::scripts(dev)
//   ScriptService::run_errors() / run_output(from)   the errors and the output of the current
//                        run only (since the script started last; stopped: of its last run),
//                        for an editor's marks and its output pane; view().runs counts the
//                        starts, so a new run is seen; run_output(from) gives what was printed
//                        from byte `from` (of all the script ever printed) on and the end, the
//                        `from` of the next call -- a pane that follows reads only what is new
//   Action::ScriptSource { dev, svc, config }   a new definition for a script that exists
//                        (the config of a project's ServiceDef, its source included): the
//                        service stays -- its disk, its numbers, errors and output, its time
//                        series -- and only the program is new; one that should run (enabled:
//                        running, failed, waiting for the power) starts again with it at once,
//                        a stopped one stays stopped.  A definition that does not compile is
//                        refused (line and column), the old program goes on.  Replacing the
//                        service with Action::Service instead gives a new one: its disk starts
//                        empty.  Action::RenameService (os.rs) renames a script as it runs.
//
// The host API (the names of script::GLOBALS; times in milliseconds, int or float):
//   log(x, ..), log.debug|info|warn|error(..)   a line in the device's log, tagged with the
//                        service's name; print(..) goes to the service's output
//   now()                the virtual time in ms (a float to the nanosecond), CPU time included
//   rand() / rand(n) / rand(a, b)   [0, 1) / 0 .. n-1 / a .. b from the service's own stream
//   config               the configuration (a map)
//   timer.after(ms, f) / timer.every(ms, f) -> id, timer.cancel(id)   every: 10 µs at least
//   net.listen(port, {accept, data, close, binary}) -> listener {kind, id, port}, close()
//   net.connect(addr, port, {connect, data, close, binary}) -> conn
//   conn {kind: "tcp", id, peer, peer_port, port}: send(data), close() (after what is
//                        queued), abort() (RST); close(conn, why) comes once per connection,
//                        whoever ends it: "closed", "reset", "refused", "timeout",
//                        "unreachable", "aborted"; after the peer's FIN the connection closes
//                        once that callback returned
//   net.udp(port, fn(sock, msg), {binary}) -> sock {kind, port}: send(addr, port, data),
//                        close(); msg {data, from, port}; port 0: an ephemeral one
//   dns.resolve(name, fn(err, addrs))   the device's resolver; addrs: a list of str
//   http.serve(port, fn(req, res))      once per script: req {method, path, target, query,
//                        headers, body, peer, peer_port}, res.send(status, body, headers),
//                        res.json(status, value, headers), even from a later callback; a
//                        handler that returns a str, bytes, a list, a map or a status (int)
//                        without using res answers with it, one that fails answers 500
//   http.request(url | {url, method, headers, body, timeout_ms}, fn(err, resp))
//                        resp {status, headers, body}; names go through the resolver, the
//                        timeout is 30 s unless given (0: none)
//   disk.get(key, fn(err, value)), disk.put(key, value, fn(err)), disk.delete(key, fn(err,
//   had)), disk.scan(prefix, fn(err, items))   JSON values on the device's disk (disk.rs:
//                        access times and capacity); they outlive restarts and power cycles
//   metrics.inc(name, n), metrics.gauge(name, v), metrics.observe(name, v)   numbers of
//                        Service::metrics (expect DEV svc NAME metric X); observe gives
//                        NAME_count, NAME_p50, NAME_p95, NAME_p99, NAME_max over five minutes;
//                        in the time series (series.rs) inc is a rate, gauge a level, and
//                        observe's quantiles and maximum those of each interval (the first
//                        call of a name decides its kind)
//   json.encode / json.decode   the standard library, as everything else not listed here
// Data arrives as str (UTF-8; a character split between segments waits for its rest), with
// binary: true as bytes; send takes str or bytes.  Errors reach callbacks as German text.
//
// CPU time: an event the script asked for (a datagram, a connection, data, a request, a
// timer, an answer, a lookup, a disk access) waits in a queue until the CPU is free; then its
// callback runs, fuel × ns_per_op long, and the CPU is busy meanwhile: a single-threaded
// program, so under load events wait and a busy loop delays every answer.  What a callback
// does to the outside (sends, connects, closes, answers, requests, lookups, disk accesses, log
// lines) happens at the moment of its CPU time it was done at; sockets and listeners open at
// once.  A callback runs in slices of `slice` fuel and the simulation goes on between them, so
// an endless loop is 100 % CPU, not a hang, until limits.fuel per callback ends it (0: no
// limit).  TCP data waits in the socket while the CPU is busy (its window fills), connections
// in the backlog; a full queue drops datagrams and answers requests 503 (callbacks the script
// asked for, of timers, the disk or answers, still come).  An error (an
// exception nobody caught, a limit) names line and column in the status, the errors (script
// errors NAME), the device's log and the trace; one in the top-level code, or any with
// stop_on_error, ends the service (OsApi::halt).  Stop and power off end the program and its
// memory; the disk keeps what it stored, the numbers stay.
//
// Metrics: the script's own first, then calls, errors, fuel, busy_us, queued, max_queued,
// dropped, rejected, timers, conns, heap_kb.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value as Json;
use base::metrics::Window;
use base::rng::Pcg32;
use jfn_model::msg;
use jfn_model::msg::{dur, Msg};
use netpkt::syslog::{SEV_DEBUG, SEV_ERR, SEV_INFO, SEV_WARNING};
use netpkt::Ipv4;
use script::{num, Cx, ErrorKind, Fail, Limits, Pin, Program, Ref, Status, TraceLine, Value, Vm};

use crate::http::{Agent, Done, Reply, Req, ReqIn, Resp, Serve, ServeCfg, Url, Wheel};
use crate::kernel::{Engine, Ns, MS, SEC, US};
use crate::action::ActErr;
use crate::os::{Dgram, Job, Metric, OsApi, Service, SvcEvent};
use crate::series::{metric_kind_of, LatStat, LatWindow, MetricKind};
use crate::tcp::{SockOpts, TcpEnd};
use crate::trace::{HopId, Note};

/// timer keys: the service's own deadlines (the CPU, effects, the script's timers, the disk),
/// http.serve, http.request
const K_OWN: u32 = 0;
const K_SERVE: u32 = 1;
const K_AGENT: u32 = 2;
/// errors and bytes of output a script keeps
const ERRORS_KEEP: usize = 20;
const OUTPUT_KEEP: usize = 16 << 10;
/// metrics of one script at most, metric names in the whole program
const METRICS_MAX: usize = 64;
const NAMES_MAX: usize = 4096;
/// the shortest period of timer.every
const EVERY_MIN: Ns = 10 * US;
/// seconds of CPU time kept (top's load averages)
const BUSY_KEEP: usize = 900;
/// http.request gives up after this long unless the script says otherwise
const REQUEST_TIMEOUT: Ns = 30 * SEC;

/// the functions a script may also read as values (`let f = log`)
const FUNCS: &[&str] = &[
    "log", "log.debug", "log.info", "log.warn", "log.error", "now", "rand", "timer.after", "timer.every", "timer.cancel", "net.listen", "net.connect",
    "net.udp", "dns.resolve", "http.serve", "http.request", "disk.get", "disk.put", "disk.delete", "disk.scan", "metrics.inc", "metrics.gauge",
    "metrics.observe",
];

// ---------------------------------------------------------------- what the outside sees

/// how a script runs
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScriptCfg {
    /// the CPU time of one unit of fuel in picoseconds (5000: 5 ns per instruction)
    pub ps_per_op: u64,
    /// fuel of one slice; after it the simulation goes on, then the callback
    pub slice: u64,
    /// heap, stack, depth and fuel per callback (fuel u64::MAX: no limit)
    pub limits: Limits,
    /// events waiting for the CPU at most
    pub queue: usize,
    /// every runtime error ends the service (else only one in the top-level code)
    pub stop_on_error: bool,
}

impl Default for ScriptCfg {
    fn default() -> ScriptCfg {
        ScriptCfg { ps_per_op: 5000, slice: 100_000, limits: Limits { output: 64 << 10, ..Limits::default() }, queue: 1000, stop_on_error: false }
    }
}

/// where a script is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptState {
    /// not started yet, stopped or switched off
    Stopped,
    Running,
    /// an error ended it (in the top-level code, or any with stop_on_error)
    Failed,
}

/// an error of a script
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptErr {
    /// the virtual time, CPU time included
    pub at: Ns,
    pub kind: ErrorKind,
    /// the position (0 when it was not in the script)
    pub line: u32,
    pub col: u32,
    /// German, as the VM words it
    pub msg: String,
    /// the calls, innermost first
    pub trace: Vec<TraceLine>,
    /// what ran: "start", "timer", "udp", "accept", "connect", "data", "close", "http",
    /// "request", "dns", "disk"
    pub during: &'static str,
}

/// a script as the shell and the app show it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptView {
    pub state: ScriptState,
    /// a callback has the CPU now
    pub busy: bool,
    /// when it started last
    pub since: Ns,
    pub calls: u64,
    pub errors: u64,
    pub fuel: u64,
    pub busy_ns: u64,
    pub queued: usize,
    pub max_queued: u64,
    pub dropped: u64,
    pub rejected: u64,
    pub timers: usize,
    pub conns: usize,
    /// what it has open: ("http", 80), ("tcp", 7000), ("udp", 5353)
    pub ports: Vec<(&'static str, u16)>,
    /// bytes on its heap
    pub heap: u64,
    pub cfg: ScriptCfg,
    /// lines of the source
    pub lines: usize,
    /// the script's own metrics as Service::metrics names them
    pub metrics: Vec<(&'static str, i64)>,
    /// how often it started (a new run: run_errors and run_output begin again)
    pub runs: u64,
}

// ---------------------------------------------------------------- the record

#[derive(Clone, Default)]
struct Stats {
    calls: u64,
    errors: u64,
    fuel: u64,
    busy_ns: u64,
    dropped: u64,
    rejected: u64,
    max_queued: u64,
}

/// a metric of the script: a number (inc, gauge) or a histogram (observe) with its names
#[derive(Clone)]
enum UVal {
    Num(i64),
    Hist(Window, [&'static str; 5]),
}

#[derive(Clone)]
struct UMetric {
    name: &'static str,
    /// what the time series make of a number: a rate (inc) or a level (gauge)
    kind: MetricKind,
    v: UVal,
}

/// CPU time per second of virtual time (top)
#[derive(Clone, Default)]
struct Busy {
    /// the second of the newest entry
    head: u64,
    secs: VecDeque<u64>,
}

impl Busy {
    fn add(&mut self, mut from: Ns, to: Ns) {
        while from < to {
            let sec = from / SEC;
            let end = to.min((sec + 1) * SEC);
            self.put(sec, end - from);
            from = end;
        }
    }
    fn put(&mut self, sec: u64, ns: u64) {
        if self.secs.is_empty() {
            self.head = sec;
            self.secs.push_back(0);
        }
        while self.head < sec {
            self.head += 1;
            self.secs.push_back(0);
            if self.secs.len() > BUSY_KEEP {
                self.secs.pop_front();
            }
        }
        let back = (self.head - sec) as usize;
        if back < self.secs.len() {
            let i = self.secs.len() - 1 - back;
            self.secs[i] += ns;
        }
    }
    /// the CPU time of the `secs` seconds that end with the second of `now`
    fn sum(&self, now: Ns, secs: u64) -> u64 {
        let cur = now / SEC;
        let first = (cur + 1).saturating_sub(secs);
        let base = (self.head + 1).saturating_sub(self.secs.len() as u64);
        (first..=cur).filter(|&s| s >= base && s <= self.head).map(|s| self.secs[(s - base) as usize]).sum()
    }
}

/// what a script service keeps over restarts
#[derive(Clone)]
struct Book {
    name: String,
    state: ScriptState,
    since: Ns,
    stats: Stats,
    errors: VecDeque<ScriptErr>,
    output: String,
    metrics: Vec<UMetric>,
    busy: Busy,
    /// the disk's content: key -> JSON text
    store: BTreeMap<String, String>,
    /// bytes printed in all (output holds the last ones)
    printed: u64,
    /// the starts; at the last one: the bytes printed and the errors so far (where the current
    /// run's output and errors begin)
    runs: u64,
    out_mark: u64,
    err_mark: u64,
}

impl Book {
    fn print(&mut self, s: &str) {
        self.printed += s.len() as u64;
        self.output.push_str(s);
        if self.output.len() > OUTPUT_KEEP {
            let mut cut = self.output.len() - OUTPUT_KEEP;
            while !self.output.is_char_boundary(cut) {
                cut += 1;
            }
            self.output.drain(..cut);
        }
    }
}

/// what a script is made of (clones of the service share it)
struct Def {
    source: String,
    prog: Program,
    cfg: ScriptCfg,
    /// the script's config as JSON text
    conf: String,
    /// the device's disk: latency, bytes per second, accesses per second, capacity
    disk: Option<(Ns, u64, u64, u64)>,
}

// ---------------------------------------------------------------- the running program

/// an event waiting for the CPU
#[derive(Clone)]
enum Pending {
    /// the top-level code
    Top,
    Timer(u64),
    Udp(u16, Dgram),
    /// connections wait in the backlog of a listener
    Accept(u64),
    /// a connection was accepted: its accept callback
    Accepted(u64),
    Connected(u64),
    Data(u64),
    Ended(u64, Why),
    Http(ReqIn),
    Answer(Done),
    Dns(u32),
    Disk(u64),
}

/// why a connection ends
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Why {
    /// the peer's FIN
    Peer,
    /// the script closed it
    Local,
    Aborted,
    /// the connect was refused at once (no route)
    Failed,
    End(TcpEnd),
}

impl Why {
    fn word(self) -> &'static str {
        match self {
            Why::Peer | Why::Local | Why::End(TcpEnd::Closed) => "closed",
            Why::Aborted | Why::End(TcpEnd::Aborted) => "aborted",
            Why::Failed | Why::End(TcpEnd::Unreachable) => "unreachable",
            Why::End(TcpEnd::Refused) => "refused",
            Why::End(TcpEnd::Reset) => "reset",
            Why::End(TcpEnd::Timeout) => "timeout",
        }
    }
}

/// a deadline of the service's own timer key
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Due {
    /// the CPU is free again
    Resume,
    /// effects are due
    Flush,
    Timer(u64),
    Disk(u64),
}

/// what a callback does to the outside, at the moment of its CPU time
#[derive(Clone)]
enum Effect {
    Log(u8, String),
    Note(Note),
    /// the script ends (an error)
    Halt,
    UdpSend { port: u16, dst: Ipv4, dport: u16, data: Vec<u8> },
    UdpClose(u16),
    Connect(u64),
    Send(u64, Vec<u8>),
    Close(u64),
    Abort(u64),
    Unlisten(u64),
    Respond(u64, Reply),
    Request(u64, Req),
    Resolve(u64, String),
    Disk(u64),
}

/// what a map the host made stands for
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle {
    Conn(u64),
    Listener(u64),
    Udp(u16),
    Res(u64),
}

/// what follows a callback
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Then {
    Nothing,
    /// close the connection (after the peer's FIN)
    Close(u64),
    /// let the socket go (it ended) and forget the connection
    Release(u64),
    Forget(u64),
}

/// the callback on the CPU
#[derive(Clone, Copy)]
struct Inv {
    start: Ns,
    cause: Option<HopId>,
    during: &'static str,
    /// the request of http.serve it handles
    req: Option<u64>,
    then: Then,
    top: bool,
}

/// a callback to start
struct Call {
    f: Value,
    args: Vec<Value>,
    during: &'static str,
    req: Option<u64>,
    then: Then,
    /// a pin the call makes unnecessary (one-shot callbacks)
    unpin: Option<Pin>,
    top: bool,
}

impl Call {
    fn new(f: Value, args: Vec<Value>, during: &'static str) -> Call {
        Call { f, args, during, req: None, then: Then::Nothing, unpin: None, top: false }
    }
}

#[derive(Clone, Copy)]
struct Tmr {
    pin: Pin,
    period: Option<Ns>,
    at: Ns,
    wheel: u64,
    /// its callback waits in the queue
    queued: bool,
}

#[derive(Clone)]
struct Conn {
    sock: Option<u32>,
    handle: Pin,
    href: Option<Ref>,
    handlers: Pin,
    remote: (Ipv4, u16),
    /// established (sending may begin)
    open: bool,
    /// bytes the socket did not take yet
    out: Vec<u8>,
    /// the script closed or aborted it (send throws)
    closed: bool,
    /// the close is applied: FIN once `out` is sent
    closing: bool,
    /// the socket is given back
    released: bool,
    /// its close callback was started
    ended: bool,
    /// the start of a character the next data completes
    carry: Vec<u8>,
    binary: bool,
    data_queued: bool,
}

impl Conn {
    fn new(handle: Pin, href: Option<Ref>, handlers: Pin, remote: (Ipv4, u16), binary: bool) -> Conn {
        Conn { sock: None, handle, href, handlers, remote, open: false, out: Vec::new(), closed: false, closing: false, released: false, ended: false, carry: Vec::new(), binary, data_queued: false }
    }
}

#[derive(Clone, Copy)]
struct Lst {
    sock: u32,
    handle: Pin,
    handlers: Pin,
    port: u16,
    binary: bool,
    accept_queued: bool,
}

#[derive(Clone, Copy)]
struct Udp {
    handle: Pin,
    handler: Pin,
    binary: bool,
}

#[derive(Clone)]
enum DiskKind {
    Get(String),
    Put(String, String),
    Delete(String),
    Scan(String),
}

#[derive(Clone)]
enum DiskOut {
    Nothing,
    Value(Option<String>),
    Had(bool),
    Items(Vec<(String, String)>),
    Full,
}

#[derive(Clone)]
struct DiskOp {
    kind: DiskKind,
    cb: Option<Pin>,
    out: DiskOut,
}

#[derive(Clone)]
struct Run {
    /// None while it runs (Api has the rest of Run)
    vm: Option<Vm>,
    rng: Pcg32,
    queue: VecDeque<(Pending, Option<HopId>)>,
    inv: Option<Inv>,
    /// the CPU is busy until then
    busy_until: Ns,
    /// an error ends the script: nothing starts any more
    halting: bool,
    effects: VecDeque<(Ns, Option<HopId>, Effect)>,
    wheel: Wheel<Due>,
    resume: Option<(Ns, u64)>,
    flush_at: Option<(Ns, u64)>,
    serve: Option<Serve>,
    serve_fn: Option<Pin>,
    agent: Agent,
    handles: BTreeMap<Ref, Handle>,
    next: u64,
    timers: BTreeMap<u64, Tmr>,
    conns: BTreeMap<u64, Conn>,
    by_sock: BTreeMap<u32, u64>,
    listeners: BTreeMap<u64, Lst>,
    by_listener: BTreeMap<u32, u64>,
    udps: BTreeMap<u16, Udp>,
    /// requests of http.serve not answered yet: their res map
    open: BTreeMap<u64, (Pin, Option<Ref>)>,
    /// http.request: callbacks by the script's id, the Agent's ids
    requests: BTreeMap<u64, Pin>,
    by_agent: BTreeMap<u64, u64>,
    /// dns.resolve: callbacks by the script's id, the lookups
    resolves: BTreeMap<u64, Pin>,
    by_lookup: BTreeMap<u32, u64>,
    disk: BTreeMap<u64, DiskOp>,
}

/// a script as a service
#[derive(Clone)]
pub struct ScriptService {
    def: Arc<Def>,
    run: Option<Box<Run>>,
    book: Book,
    /// the virtual time of its last event (metrics() without a time)
    seen: Ns,
}

// ---------------------------------------------------------------- helpers

static NAMES: rt::sync::SpinLock<BTreeSet<&'static str>> = rt::sync::SpinLock::new(BTreeSet::new());

/// a metric name as Metric holds it (&'static str): each text once per program, at most
/// NAMES_MAX of them
fn intern(name: &str) -> Option<&'static str> {
    let mut set = NAMES.lock();
    if let Some(s) = set.get(name) {
        return Some(*s);
    }
    if set.len() >= NAMES_MAX {
        return None;
    }
    let s: &'static str = Box::leak(String::from(name).into_boxed_str());
    set.insert(s);
    Some(s)
}

static STATS: rt::sync::SpinLock<BTreeMap<&'static str, &'static [(&'static str, LatStat)]>> = rt::sync::SpinLock::new(BTreeMap::new());

/// what the time series take of an observed histogram (Service::windows): its p50, p95, p99
/// and maximum of each interval, kept once per name like the names (at most NAMES_MAX / 5)
fn lat_stats(n: &[&'static str; 5]) -> &'static [(&'static str, LatStat)] {
    let mut map = STATS.lock();
    if let Some(s) = map.get(n[1]) {
        return s;
    }
    let s: &'static [(&'static str, LatStat)] = Box::leak(Box::new([(n[1], LatStat::Quantile(500)), (n[2], LatStat::Quantile(950)), (n[3], LatStat::Quantile(990)), (n[4], LatStat::Max)]));
    map.insert(n[1], s);
    s
}

/// fuel as CPU time
fn cpu(fuel: u64, ps: u64) -> Ns {
    (fuel as u128 * ps as u128 / 1000).min(u64::MAX as u128) as Ns
}

/// milliseconds (int or float, at least 0) as nanoseconds
fn ms_of(v: Value) -> Option<Ns> {
    match v {
        Value::Int(i) if i >= 0 => Some((i as u64).saturating_mul(MS)),
        Value::Float(f) if !num::is_nan(f) && num::le(0.0, f) => {
            let ns = num::floor(num::add(num::mul(f, 1_000_000.0), 0.5));
            Some(num::to_i64(ns).map_or(u64::MAX, |n| n as u64))
        }
        _ => None,
    }
}

/// nanoseconds as milliseconds (float)
fn ms_float(t: Ns) -> f64 {
    num::div(num::from_i64(t.min(i64::MAX as u64) as i64), 1_000_000.0)
}

fn is_fn(v: Value) -> bool {
    matches!(v, Value::Func(_) | Value::Native(_))
}

fn port_of(v: Value) -> Option<u16> {
    match v {
        Value::Int(p) if (0..=65535).contains(&p) => Some(p as u16),
        _ => None,
    }
}

fn ref_of(v: Value) -> Option<Ref> {
    match v {
        Value::Map(r) => Some(r),
        _ => None,
    }
}

/// a field value for cx_map / vm_map
enum F<'a> {
    S(&'a str),
    I(i64),
    V(Value),
}

fn cx_map(cx: &mut Cx, fields: &[(&str, F)]) -> Result<Value, Fail> {
    let m = cx.new_map()?;
    for (k, v) in fields {
        let key = cx.new_str(k)?;
        let val = match v {
            F::S(s) => cx.new_str(s)?,
            F::I(i) => Value::Int(*i),
            F::V(x) => *x,
        };
        cx.map_set(m, key, val)?;
    }
    Ok(m)
}

fn vm_map(vm: &mut Vm, fields: &[(&str, F)]) -> Result<Value, script::Error> {
    let m = vm.new_map()?;
    for (k, v) in fields {
        let key = vm.new_str(k)?;
        let val = match v {
            F::S(s) => vm.new_str(s)?,
            F::I(i) => Value::Int(*i),
            F::V(x) => *x,
        };
        vm.map_set(m, key, val)?;
    }
    Ok(m)
}

/// a str or bytes value as bytes
fn data_of(cx: &Cx, v: Value) -> Option<Vec<u8>> {
    if let Some(s) = cx.str(v) {
        return Some(Vec::from(s.as_bytes()));
    }
    cx.bytes(v).map(Vec::from)
}

fn ip_of(cx: &Cx, v: Value) -> Option<Ipv4> {
    cx.str(v).and_then(|s| Ipv4::parse(s.trim()))
}

/// m.key is truthy
fn flag(cx: &Cx, m: Value, key: &str) -> bool {
    cx.map_get_str(m, key).is_some_and(|v| v.truthy())
}

/// the text of bytes that came in pieces: a character cut at the end waits in `carry` for the
/// rest, broken bytes become U+FFFD
fn utf8_chunk(carry: &mut Vec<u8>, data: Vec<u8>) -> String {
    let data = if carry.is_empty() {
        data
    } else {
        let mut c = core::mem::take(carry);
        c.extend_from_slice(&data);
        c
    };
    let mut out = String::new();
    let mut rest: &[u8] = &data;
    loop {
        match core::str::from_utf8(rest) {
            Ok(s) => {
                out.push_str(s);
                break;
            }
            Err(e) => {
                let (ok, bad) = rest.split_at(e.valid_up_to());
                out.push_str(core::str::from_utf8(ok).unwrap_or_default());
                match e.error_len() {
                    None => {
                        carry.extend_from_slice(bad);
                        break;
                    }
                    Some(n) => {
                        out.push('\u{FFFD}');
                        rest = &bad[n..];
                    }
                }
            }
        }
    }
    out
}

fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

/// %XX and + of a part of a query
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < b.len() => match (hex(b[i + 1]), hex(b[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h << 4 | l);
                    i += 3;
                }
                _ => {
                    out.push(b'%');
                    i += 1;
                }
            },
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// headers as a map: lower-case names, repeated ones joined with ", "
fn headers_map(vm: &mut Vm, h: &netpkt::http::Headers) -> Result<Value, script::Error> {
    let m = vm.new_map()?;
    for (k, v) in &h.0 {
        let k = vm.new_str(&k.to_ascii_lowercase())?;
        let joined = match vm.map_get(m, k).and_then(|old| vm.str(old).map(String::from)) {
            Some(old) => format!("{}, {}", old, v),
            None => v.clone(),
        };
        let v = vm.new_str(&joined)?;
        vm.map_set(m, k, v)?;
    }
    Ok(m)
}

/// a request of http.serve as the script sees it
fn req_map(vm: &mut Vm, r: &ReqIn) -> Result<Value, script::Error> {
    let target = r.head.target.as_str();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let q = vm.new_map()?;
    for part in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        let k = vm.new_str(&unescape(k))?;
        let v = vm.new_str(&unescape(v))?;
        vm.map_set(q, k, v)?;
    }
    let headers = headers_map(vm, &r.head.headers)?;
    let body = vm.new_str(&String::from_utf8_lossy(&r.body))?;
    let peer = format!("{}", r.peer.0);
    vm_map(
        vm,
        &[
            ("method", F::S(&r.head.method)),
            ("path", F::S(path)),
            ("target", F::S(target)),
            ("query", F::V(q)),
            ("headers", F::V(headers)),
            ("body", F::V(body)),
            ("peer", F::S(&peer)),
            ("peer_port", F::I(r.peer.1 as i64)),
        ],
    )
}

/// an answer of http.request as the script sees it
fn resp_map(vm: &mut Vm, r: &Resp) -> Result<Value, script::Error> {
    let headers = headers_map(vm, &r.head.headers)?;
    let body = vm.new_str(&String::from_utf8_lossy(&r.body))?;
    vm_map(vm, &[("status", F::I(r.head.status as i64)), ("headers", F::V(headers)), ("body", F::V(body))])
}

/// the answer a handler gives by returning a value without using res
fn reply_from(vm: &mut Vm, v: Value) -> Option<Reply> {
    match v {
        Value::Int(s) if (100..=999).contains(&s) => Some(Reply::status_page(s as u16)),
        Value::Str(_) => vm.str(v).map(|s| Reply::text(200, s)),
        Value::Bytes(_) => vm.bytes(v).map(|b| {
            let mut r = Reply::text(200, "");
            r.body = Vec::from(b);
            r.headers.set("Content-Type", "application/octet-stream");
            r
        }),
        Value::List(_) | Value::Map(_) => vm.json_encode(v, 0).ok().map(|j| {
            let mut r = Reply::text(200, &j);
            r.headers.set("Content-Type", "application/json");
            r
        }),
        _ => None,
    }
}

// ---------------------------------------------------------------- the host side of a run

/// what a script calls while it runs: the running program without its VM, the record, the
/// device
struct Api<'a, 'b> {
    run: &'a mut Run,
    book: &'a mut Book,
    def: &'a Def,
    os: &'a mut OsApi<'b>,
    inv: Inv,
}

impl Api<'_, '_> {
    /// the moment of CPU time the script is at
    fn at(&self, cx: &Cx) -> Ns {
        self.inv.start.saturating_add(cpu(cx.fuel(), self.def.cfg.ps_per_op))
    }
    fn effect(&mut self, cx: &Cx, e: Effect) {
        let t = self.at(cx);
        self.run.effects.push_back((t, self.inv.cause, e));
    }
    fn id(&mut self) -> u64 {
        self.run.next += 1;
        self.run.next
    }

    fn log(&mut self, cx: &mut Cx, sev: u8, args: &[Value]) -> Result<Value, Fail> {
        let parts: Vec<String> = args.iter().map(|v| cx.display(*v)).collect();
        let text = parts.join(" ");
        cx.charge(text.len() as u64 / 64);
        self.effect(cx, Effect::Log(sev, text));
        Ok(Value::Nil)
    }

    fn rand(&mut self, cx: &mut Cx, args: &[Value]) -> Result<Value, Fail> {
        let a = args.first().copied().unwrap_or(Value::Nil);
        let b = args.get(1).copied().unwrap_or(Value::Nil);
        match (a, b) {
            (Value::Nil, Value::Nil) => {
                let x = self.run.rng.next_u64() >> 11;
                Ok(Value::Float(num::mul(num::from_i64(x as i64), 1.0 / 9_007_199_254_740_992.0)))
            }
            (Value::Int(n), Value::Nil) if n >= 1 => Ok(Value::Int(self.run.rng.range(0, n - 1))),
            (Value::Int(lo), Value::Int(hi)) if lo <= hi => Ok(Value::Int(self.run.rng.range(lo, hi))),
            _ => Err(cx.error("rand() gibt eine Zahl in [0, 1), rand(n) eine ganze Zahl von 0 bis n - 1, rand(a, b) eine von a bis b")),
        }
    }

    fn timer(&mut self, cx: &mut Cx, name: &str, ms: Value, f: Value) -> Result<Value, Fail> {
        let Some(d) = ms_of(ms) else {
            return Err(cx.error(&format!("{}: die Zeit muss eine Zahl ab 0 sein (Millisekunden)", name)));
        };
        if !is_fn(f) {
            return Err(cx.error(&format!("{}: das zweite Argument muss eine Funktion sein", name)));
        }
        let period = if name == "timer.every" { Some(d.max(EVERY_MIN)) } else { None };
        let at = self.at(cx).saturating_add(period.unwrap_or(d));
        let id = self.id();
        let wheel = self.run.wheel.add(at, Due::Timer(id));
        let pin = cx.pin(f);
        self.run.timers.insert(id, Tmr { pin, period, at, wheel, queued: false });
        Ok(Value::Int(id as i64))
    }

    fn cancel(&mut self, cx: &mut Cx, v: Value) -> Result<Value, Fail> {
        let Value::Int(id) = v else {
            return Ok(Value::Bool(false));
        };
        match self.run.timers.remove(&(id as u64)) {
            Some(t) => {
                self.run.wheel.cancel(t.wheel);
                cx.unpin(t.pin);
                Ok(Value::Bool(true))
            }
            None => Ok(Value::Bool(false)),
        }
    }

    fn listen(&mut self, cx: &mut Cx, port: Value, h: Value) -> Result<Value, Fail> {
        let Some(port) = port_of(port) else {
            return Err(cx.error("net.listen: der Port muss eine ganze Zahl von 0 bis 65535 sein"));
        };
        if !matches!(h, Value::Map(_) | Value::Nil) {
            return Err(cx.error("net.listen: die Rückrufe kommen als Map {accept, data, close}"));
        }
        let binary = flag(cx, h, "binary");
        let sock = match self.os.tcp_listen_msg(port, 0, SockOpts::default()) {
            Ok(s) => s,
            Err(m) => return Err(cx.error(&format!("net.listen: {}", m.text()))),
        };
        let port = self.os.tcp_info(sock).map_or(port, |r| r.local.1);
        let id = self.id();
        let m = cx_map(cx, &[("kind", F::S("listener")), ("id", F::I(id as i64)), ("port", F::I(port as i64))])?;
        let handle = cx.pin(m);
        let handlers = cx.pin(h);
        if let Some(r) = ref_of(m) {
            self.run.handles.insert(r, Handle::Listener(id));
        }
        self.run.listeners.insert(id, Lst { sock, handle, handlers, port, binary, accept_queued: false });
        self.run.by_listener.insert(sock, id);
        Ok(m)
    }

    fn connect(&mut self, cx: &mut Cx, addr: Value, port: Value, h: Value) -> Result<Value, Fail> {
        let Some(dst) = ip_of(cx, addr) else {
            return Err(cx.error("net.connect: die Adresse muss eine IPv4-Adresse sein (Namen löst dns.resolve auf)"));
        };
        let Some(port) = port_of(port).filter(|&p| p > 0) else {
            return Err(cx.error("net.connect: der Port muss eine ganze Zahl von 1 bis 65535 sein"));
        };
        if !matches!(h, Value::Map(_) | Value::Nil) {
            return Err(cx.error("net.connect: die Rückrufe kommen als Map {connect, data, close}"));
        }
        let binary = flag(cx, h, "binary");
        let id = self.id();
        let peer = format!("{}", dst);
        let m = cx_map(cx, &[("kind", F::S("tcp")), ("id", F::I(id as i64)), ("peer", F::S(&peer)), ("peer_port", F::I(port as i64)), ("port", F::I(0))])?;
        let handle = cx.pin(m);
        let handlers = cx.pin(h);
        let href = ref_of(m);
        if let Some(r) = href {
            self.run.handles.insert(r, Handle::Conn(id));
        }
        self.run.conns.insert(id, Conn::new(handle, href, handlers, (dst, port), binary));
        self.effect(cx, Effect::Connect(id));
        Ok(m)
    }

    fn udp(&mut self, cx: &mut Cx, port: Value, f: Value, opts: Value) -> Result<Value, Fail> {
        let Some(port) = port_of(port) else {
            return Err(cx.error("net.udp: der Port muss eine ganze Zahl von 0 bis 65535 sein"));
        };
        if !is_fn(f) && f != Value::Nil {
            return Err(cx.error("net.udp: das zweite Argument muss eine Funktion sein: fn(sock, msg)"));
        }
        let binary = flag(cx, opts, "binary");
        let p = match self.os.bind_msg(port, None) {
            Ok(p) => p,
            Err(m) => return Err(cx.error(&format!("net.udp: {}", m.text()))),
        };
        let m = cx_map(cx, &[("kind", F::S("udp")), ("port", F::I(p as i64))])?;
        let handle = cx.pin(m);
        let handler = cx.pin(f);
        if let Some(r) = ref_of(m) {
            self.run.handles.insert(r, Handle::Udp(p));
        }
        self.run.udps.insert(p, Udp { handle, handler, binary });
        Ok(m)
    }

    fn resolve(&mut self, cx: &mut Cx, name: Value, f: Value) -> Result<Value, Fail> {
        let Some(n) = cx.str(name).map(String::from) else {
            return Err(cx.error("dns.resolve: der Name muss ein Text sein"));
        };
        if !is_fn(f) {
            return Err(cx.error("dns.resolve: das zweite Argument muss eine Funktion sein: fn(err, addrs)"));
        }
        let id = self.id();
        let pin = cx.pin(f);
        self.run.resolves.insert(id, pin);
        self.effect(cx, Effect::Resolve(id, n));
        Ok(Value::Nil)
    }

    fn serve(&mut self, cx: &mut Cx, port: Value, f: Value) -> Result<Value, Fail> {
        let Some(port) = port_of(port).filter(|&p| p > 0) else {
            return Err(cx.error("http.serve: der Port muss eine ganze Zahl von 1 bis 65535 sein"));
        };
        if !is_fn(f) {
            return Err(cx.error("http.serve: das zweite Argument muss eine Funktion sein: fn(req, res)"));
        }
        if let Some(s) = &self.run.serve {
            let m = format!("http.serve: dieses Skript bedient schon Port {} (einmal je Skript)", s.cfg.port);
            return Err(cx.error(&m));
        }
        let mut s = Serve::new(ServeCfg { port, server: String::from("jfn-script/1.0"), ..ServeCfg::default() }, K_SERVE);
        if !s.start(self.os, &self.book.name) {
            return Err(cx.error(&format!("http.serve: Port {} ist schon belegt.", port)));
        }
        self.run.serve = Some(s);
        self.run.serve_fn = Some(cx.pin(f));
        Ok(Value::Nil)
    }

    fn request(&mut self, cx: &mut Cx, what: Value, f: Value) -> Result<Value, Fail> {
        if !is_fn(f) {
            return Err(cx.error("http.request: das zweite Argument muss eine Funktion sein: fn(err, resp)"));
        }
        let (url, opts) = match what {
            Value::Map(_) => (cx.map_get_str(what, "url").unwrap_or(Value::Nil), what),
            v => (v, Value::Nil),
        };
        let Some(text) = cx.str(url).map(String::from) else {
            return Err(cx.error("http.request: die URL fehlt"));
        };
        let Ok(u) = Url::parse(&text) else {
            return Err(cx.error(&format!("http.request: {} ist keine http-URL", text)));
        };
        let mut req = Req::get(u);
        req.timeout = REQUEST_TIMEOUT;
        if let Value::Map(_) = opts {
            if let Some(m) = cx.map_get_str(opts, "method").and_then(|v| cx.str(v)) {
                req.method = m.to_ascii_uppercase();
            }
            if let Some(d) = cx.map_get_str(opts, "body").and_then(|b| data_of(cx, b)) {
                req.body = d;
            }
            if let Some(t) = cx.map_get_str(opts, "timeout_ms").and_then(ms_of) {
                req.timeout = t;
            }
            if let Some(h) = cx.map_get_str(opts, "headers") {
                for (k, v) in cx.map_entries(h) {
                    let (k, v) = (cx.display(k), cx.display(v));
                    req.headers.add(&k, &v);
                }
            }
        }
        let id = self.id();
        let pin = cx.pin(f);
        self.run.requests.insert(id, pin);
        self.effect(cx, Effect::Request(id, req));
        Ok(Value::Nil)
    }

    fn disk(&mut self, cx: &mut Cx, name: &str, args: &[Value]) -> Result<Value, Fail> {
        let a = |i: usize| args.get(i).copied().unwrap_or(Value::Nil);
        let Some(key) = cx.str(a(0)).map(String::from) else {
            return Err(cx.error(&format!("{}: der Schlüssel muss ein Text sein", name)));
        };
        let (kind, cb) = match name {
            "disk.get" => (DiskKind::Get(key), a(1)),
            "disk.put" => {
                let v = cx.json_encode(a(1), 0)?;
                (DiskKind::Put(key, v), a(2))
            }
            "disk.delete" => (DiskKind::Delete(key), a(1)),
            _ => (DiskKind::Scan(key), a(1)),
        };
        let needs = matches!(kind, DiskKind::Get(_) | DiskKind::Scan(_));
        if !is_fn(cb) && (needs || cb != Value::Nil) {
            return Err(cx.error(&format!("{}: der Rückruf muss eine Funktion sein: fn(err, ..)", name)));
        }
        let cb = if is_fn(cb) { Some(cx.pin(cb)) } else { None };
        let id = self.id();
        self.run.disk.insert(id, DiskOp { kind, cb, out: DiskOut::Nothing });
        self.effect(cx, Effect::Disk(id));
        Ok(Value::Nil)
    }

    fn metric(&mut self, cx: &mut Cx, name: &str, n: Value, v: Value) -> Result<Value, Fail> {
        let Some(m) = cx.str(n).map(String::from) else {
            return Err(cx.error(&format!("{}: der Name muss ein Text sein", name)));
        };
        if m.is_empty() || m.len() > 64 || !m.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-') {
            return Err(cx.error(&format!("{}: \u{201e}{}\u{201c} taugt nicht als Name (Buchstaben, Ziffern, _ . -)", name, m)));
        }
        let at = self.at(cx);
        let idx = match self.book.metrics.iter().position(|x| x.name == m) {
            Some(i) => i,
            None => {
                if self.book.metrics.len() >= METRICS_MAX {
                    return Err(cx.error(&format!("{}: zu viele Metriken (höchstens {})", name, METRICS_MAX)));
                }
                let full = || format!("{}: zu viele Metriknamen", name);
                let Some(nm) = intern(&m) else {
                    return Err(cx.error(&full()));
                };
                let v = if name == "metrics.observe" {
                    let s = |suffix: &str| intern(&format!("{}_{}", m, suffix));
                    let (Some(a), Some(b), Some(c), Some(d), Some(e)) = (s("count"), s("p50"), s("p95"), s("p99"), s("max")) else {
                        return Err(cx.error(&full()));
                    };
                    UVal::Hist(Window::new(300, SEC), [a, b, c, d, e])
                } else {
                    UVal::Num(0)
                };
                let kind = if name == "metrics.gauge" { MetricKind::Gauge } else { MetricKind::Counter };
                self.book.metrics.push(UMetric { name: nm, kind, v });
                self.book.metrics.len() - 1
            }
        };
        let int = |v: Value| match v {
            Value::Int(i) => Some(i),
            Value::Float(f) => num::to_i64(num::trunc(f)),
            Value::Bool(b) => Some(b as i64),
            _ => None,
        };
        match (&mut self.book.metrics[idx].v, name) {
            (UVal::Num(x), "metrics.inc") => match if v == Value::Nil { Some(1) } else { int(v) } {
                Some(d) => *x = x.saturating_add(d),
                None => return Err(cx.error("metrics.inc: der Schritt muss eine Zahl sein")),
            },
            (UVal::Num(x), "metrics.gauge") => match int(v) {
                Some(g) => *x = g,
                None => return Err(cx.error("metrics.gauge: der Wert muss eine Zahl sein")),
            },
            (UVal::Hist(w, _), "metrics.observe") => match int(v).filter(|&i| i >= 0) {
                Some(o) => {
                    w.record(at, o as u64);
                }
                None => return Err(cx.error("metrics.observe: der Wert muss eine Zahl ab 0 sein")),
            },
            _ => return Err(cx.error(&format!("{}: {} ist eine Metrik anderer Art", name, m))),
        }
        Ok(Value::Nil)
    }

    /// conn.send, res.json, sock.close ..: the methods of the maps the host made
    fn method(&mut self, cx: &mut Cx, m: &str, args: &[Value]) -> Result<Value, Fail> {
        let a = |i: usize| args.get(i).copied().unwrap_or(Value::Nil);
        let obj = a(0);
        let Some(r) = ref_of(obj) else {
            return Err(Fail::Unknown);
        };
        let Some(h) = self.run.handles.get(&r).copied() else {
            // a map of a connection, socket or request that is over
            let kind = cx.map_get_str(obj, "kind").and_then(|k| cx.str(k)).map(String::from);
            return match (kind.as_deref(), m) {
                (Some("tcp"), "send" | "close" | "abort") => Err(cx.error("Die Verbindung ist schon geschlossen.")),
                (Some("udp" | "listener"), "send" | "close") => Err(cx.error("Der Socket ist schon geschlossen.")),
                (Some("response"), "send" | "json") => Err(cx.error("Die Anfrage ist schon beantwortet.")),
                _ => Err(Fail::Unknown),
            };
        };
        match (h, m) {
            (Handle::Conn(c), "send") => {
                let Some(d) = data_of(cx, a(1)) else {
                    return Err(cx.error("send: die Daten müssen str oder bytes sein"));
                };
                if self.run.conns.get(&c).is_none_or(|x| x.closed) {
                    return Err(cx.error("Die Verbindung ist schon geschlossen."));
                }
                cx.charge(d.len() as u64 / 64);
                self.effect(cx, Effect::Send(c, d));
                Ok(Value::Nil)
            }
            (Handle::Conn(c), "close" | "abort") => {
                if let Some(x) = self.run.conns.get_mut(&c)
                    && !x.closed
                {
                    x.closed = true;
                    let e = if m == "close" { Effect::Close(c) } else { Effect::Abort(c) };
                    self.effect(cx, e);
                }
                Ok(Value::Nil)
            }
            (Handle::Listener(l), "close") => {
                self.run.handles.remove(&r);
                self.effect(cx, Effect::Unlisten(l));
                Ok(Value::Nil)
            }
            (Handle::Udp(p), "send") => {
                let Some(dst) = ip_of(cx, a(1)) else {
                    return Err(cx.error("send: die Adresse muss eine IPv4-Adresse sein"));
                };
                let Some(dport) = port_of(a(2)).filter(|&x| x > 0) else {
                    return Err(cx.error("send: der Port muss eine ganze Zahl von 1 bis 65535 sein"));
                };
                let Some(d) = data_of(cx, a(3)) else {
                    return Err(cx.error("send: die Daten müssen str oder bytes sein"));
                };
                cx.charge(d.len() as u64 / 64);
                self.effect(cx, Effect::UdpSend { port: p, dst, dport, data: d });
                Ok(Value::Nil)
            }
            (Handle::Udp(p), "close") => {
                self.run.handles.remove(&r);
                self.effect(cx, Effect::UdpClose(p));
                Ok(Value::Nil)
            }
            (Handle::Res(id), "send" | "json") => {
                let status = match a(1) {
                    Value::Int(s) if (100..=999).contains(&s) => s as u16,
                    _ => return Err(cx.error(&format!("res.{}: der Status muss eine Zahl von 100 bis 999 sein", m))),
                };
                let mut rep = if m == "json" {
                    let body = cx.json_encode(a(2), 0)?;
                    let mut x = Reply::text(status, &body);
                    x.headers.set("Content-Type", "application/json");
                    x
                } else if a(2) == Value::Nil {
                    Reply::text(status, "")
                } else if let Some(s) = cx.str(a(2)) {
                    Reply::text(status, s)
                } else if let Some(b) = cx.bytes(a(2)) {
                    let mut x = Reply::text(status, "");
                    x.body = Vec::from(b);
                    x.headers.set("Content-Type", "application/octet-stream");
                    x
                } else {
                    return Err(cx.error("res.send: der Inhalt muss str oder bytes sein (Daten schickt res.json)"));
                };
                if let Value::Map(_) = a(3) {
                    for (k, v) in cx.map_entries(a(3)) {
                        let (k, v) = (cx.display(k), cx.display(v));
                        rep.headers.set(&k, &v);
                    }
                }
                cx.charge(rep.body.len() as u64 / 64);
                self.run.handles.remove(&r);
                if let Some((pin, _)) = self.run.open.remove(&id) {
                    cx.unpin(pin);
                }
                self.effect(cx, Effect::Respond(id, rep));
                Ok(Value::Nil)
            }
            _ => Err(Fail::Unknown),
        }
    }
}

impl script::Host for Api<'_, '_> {
    fn call(&mut self, cx: &mut Cx, name: &str, args: &[Value]) -> Result<Value, Fail> {
        let a = |i: usize| args.get(i).copied().unwrap_or(Value::Nil);
        match name {
            "log" | "log.info" => self.log(cx, SEV_INFO, args),
            "log.debug" => self.log(cx, SEV_DEBUG, args),
            "log.warn" => self.log(cx, SEV_WARNING, args),
            "log.error" => self.log(cx, SEV_ERR, args),
            "now" => Ok(Value::Float(ms_float(self.at(cx)))),
            "rand" => self.rand(cx, args),
            "timer.after" | "timer.every" => self.timer(cx, name, a(0), a(1)),
            "timer.cancel" => self.cancel(cx, a(0)),
            "net.listen" => self.listen(cx, a(0), a(1)),
            "net.connect" => self.connect(cx, a(0), a(1), a(2)),
            "net.udp" => self.udp(cx, a(0), a(1), a(2)),
            "dns.resolve" => self.resolve(cx, a(0), a(1)),
            "http.serve" => self.serve(cx, a(0), a(1)),
            "http.request" => self.request(cx, a(0), a(1)),
            "disk.get" | "disk.put" | "disk.delete" | "disk.scan" => self.disk(cx, name, args),
            "metrics.inc" | "metrics.gauge" | "metrics.observe" => self.metric(cx, name, a(0), a(1)),
            _ => match name.strip_prefix("map.") {
                Some(m) => self.method(cx, m, args),
                None => Err(Fail::Unknown),
            },
        }
    }
    fn knows(&self, name: &str) -> bool {
        FUNCS.contains(&name)
    }
}

// ---------------------------------------------------------------- the program between runs

impl Run {
    fn new(vm: Vm, rng: Pcg32) -> Run {
        Run {
            vm: Some(vm),
            rng,
            queue: VecDeque::new(),
            inv: None,
            busy_until: 0,
            halting: false,
            effects: VecDeque::new(),
            wheel: Wheel::new(K_OWN),
            resume: None,
            flush_at: None,
            serve: None,
            serve_fn: None,
            agent: Agent::new(K_AGENT),
            handles: BTreeMap::new(),
            next: 0,
            timers: BTreeMap::new(),
            conns: BTreeMap::new(),
            by_sock: BTreeMap::new(),
            listeners: BTreeMap::new(),
            by_listener: BTreeMap::new(),
            udps: BTreeMap::new(),
            open: BTreeMap::new(),
            requests: BTreeMap::new(),
            by_agent: BTreeMap::new(),
            resolves: BTreeMap::new(),
            by_lookup: BTreeMap::new(),
            disk: BTreeMap::new(),
        }
    }

    fn unpin(&mut self, p: Pin) {
        if let Some(vm) = self.vm.as_mut() {
            vm.unpin(p);
        }
    }

    /// an event for the CPU; a full queue drops datagrams and answers requests 503
    fn enqueue(&mut self, book: &mut Book, def: &Def, os: &mut OsApi, p: Pending) {
        if self.queue.len() >= def.cfg.queue {
            match p {
                Pending::Udp(..) => {
                    book.stats.dropped += 1;
                    os.note(Note::ScriptDropped { svc: book.name.clone() });
                    return;
                }
                Pending::Http(r) => {
                    book.stats.rejected += 1;
                    if let Some(s) = self.serve.as_mut() {
                        let mut rep = Reply::status_page(503);
                        rep.headers.set("Retry-After", "1");
                        s.respond(os, r.id, rep);
                    }
                    return;
                }
                _ => {}
            }
        }
        if self.inv.is_some() || os.now() < self.busy_until || !self.queue.is_empty() {
            os.note(Note::ScriptBusy { svc: book.name.clone(), waiting: self.queue.len() as u32 });
        }
        let cause = os.cause;
        self.queue.push_back((p, cause));
        book.stats.max_queued = book.stats.max_queued.max(self.queue.len() as u64);
    }

    /// an event of the OS
    fn event(&mut self, book: &mut Book, def: &Def, os: &mut OsApi, ev: SvcEvent) {
        if let SvcEvent::Timer(K_OWN) = ev {
            self.due(book, def, os);
            return;
        }
        if let Some(s) = self.serve.as_mut()
            && s.on(&ev, os)
        {
            for r in s.take() {
                self.enqueue(book, def, os, Pending::Http(r));
            }
            return;
        }
        if self.agent.on(&ev, os) {
            for d in self.agent.take_done() {
                self.enqueue(book, def, os, Pending::Answer(d));
            }
            return;
        }
        match ev {
            SvcEvent::Udp(d) => {
                if self.udps.contains_key(&d.dport) {
                    let port = d.dport;
                    self.enqueue(book, def, os, Pending::Udp(port, d));
                }
            }
            SvcEvent::TcpAccept { listener, .. } => {
                if let Some(&l) = self.by_listener.get(&listener)
                    && let Some(x) = self.listeners.get_mut(&l)
                    && !x.accept_queued
                {
                    x.accept_queued = true;
                    self.enqueue(book, def, os, Pending::Accept(l));
                }
            }
            SvcEvent::TcpConnected { sock, .. } => {
                if let Some(&c) = self.by_sock.get(&sock) {
                    if let Some(x) = self.conns.get_mut(&c) {
                        x.open = true;
                    }
                    self.pump_out(book, def, os, c);
                    self.enqueue(book, def, os, Pending::Connected(c));
                }
            }
            SvcEvent::TcpData { sock, .. } => {
                if let Some(&c) = self.by_sock.get(&sock)
                    && let Some(x) = self.conns.get_mut(&c)
                    && !x.data_queued
                {
                    x.data_queued = true;
                    self.enqueue(book, def, os, Pending::Data(c));
                }
            }
            SvcEvent::TcpWritable { sock } => {
                if let Some(&c) = self.by_sock.get(&sock) {
                    self.pump_out(book, def, os, c);
                }
            }
            SvcEvent::TcpEof { sock, .. } => {
                if let Some(&c) = self.by_sock.get(&sock) {
                    self.enqueue(book, def, os, Pending::Ended(c, Why::Peer));
                }
            }
            SvcEvent::TcpClosed { sock, end } => {
                if let Some(&c) = self.by_sock.get(&sock) {
                    self.enqueue(book, def, os, Pending::Ended(c, Why::End(end)));
                }
            }
            SvcEvent::Resolved(l) => {
                if self.by_lookup.contains_key(&l) {
                    self.enqueue(book, def, os, Pending::Dns(l));
                }
            }
            _ => {}
        }
    }

    /// the service's own timer key fired
    fn due(&mut self, book: &mut Book, def: &Def, os: &mut OsApi) {
        self.wheel.fired();
        let now = os.now();
        for (wid, d) in self.wheel.due(now) {
            match d {
                Due::Resume => {
                    if self.resume.is_some_and(|r| r.1 == wid) {
                        self.resume = None;
                    }
                }
                Due::Flush => {
                    if self.flush_at.is_some_and(|r| r.1 == wid) {
                        self.flush_at = None;
                    }
                }
                Due::Timer(t) => {
                    let mut queue = false;
                    if let Some(tm) = self.timers.get_mut(&t)
                        && tm.wheel == wid
                    {
                        queue = !tm.queued;
                        tm.queued = true;
                        if let Some(p) = tm.period {
                            // a CPU that was busy for longer skips the missed rounds
                            tm.at = tm.at.saturating_add(p);
                            if tm.at <= now {
                                tm.at = tm.at.saturating_add((now - tm.at) / p * p + p);
                            }
                            tm.wheel = self.wheel.add(tm.at, Due::Timer(t));
                        }
                    }
                    if queue {
                        self.enqueue(book, def, os, Pending::Timer(t));
                    }
                }
                Due::Disk(id) => self.disk_done(book, def, os, id),
            }
        }
    }

    /// runs what the CPU can run now; true when the script ends (an error ended it)
    fn pump(&mut self, book: &mut Book, def: &Def, os: &mut OsApi) -> bool {
        loop {
            if self.flush(book, def, os) {
                return true;
            }
            if self.halting || os.now() < self.busy_until {
                return false;
            }
            if self.inv.is_some() {
                self.slice(book, def, os);
                continue;
            }
            let Some((p, cause)) = self.queue.pop_front() else {
                return false;
            };
            self.dispatch(book, def, os, p, cause);
        }
    }

    /// the service's timer key at the next deadline: the CPU, the effects, timers, the disk
    fn arm(&mut self, os: &mut OsApi) {
        let now = os.now();
        let want = if (self.inv.is_some() || !self.queue.is_empty()) && self.busy_until > now { Some(self.busy_until) } else { None };
        if self.resume.map(|r| r.0) != want {
            if let Some((_, id)) = self.resume.take() {
                self.wheel.cancel(id);
            }
            if let Some(t) = want {
                self.resume = Some((t, self.wheel.add(t, Due::Resume)));
            }
        }
        let want = self.effects.front().map(|e| e.0.max(now));
        if self.flush_at.map(|r| r.0) != want {
            if let Some((_, id)) = self.flush_at.take() {
                self.wheel.cancel(id);
            }
            if let Some(t) = want {
                self.flush_at = Some((t, self.wheel.add(t, Due::Flush)));
            }
        }
        self.wheel.arm(os);
    }

    /// what is due of the effects; true when the script ends
    fn flush(&mut self, book: &mut Book, def: &Def, os: &mut OsApi) -> bool {
        let now = os.now();
        while self.effects.front().is_some_and(|e| e.0 <= now) {
            let Some((_, cause, e)) = self.effects.pop_front() else {
                break;
            };
            if self.apply(book, def, os, cause, e) {
                return true;
            }
        }
        false
    }

    fn apply(&mut self, book: &mut Book, def: &Def, os: &mut OsApi, cause: Option<HopId>, e: Effect) -> bool {
        let saved = os.cause;
        os.cause = cause;
        let mut halt = false;
        match e {
            Effect::Log(sev, text) => os.log(sev, &book.name, &text),
            Effect::Note(n) => os.note(n),
            Effect::Halt => halt = true,
            Effect::UdpSend { port, dst, dport, data } => {
                if self.udps.contains_key(&port) {
                    os.send(port, dst, dport, &data);
                }
            }
            Effect::UdpClose(port) => {
                if let Some(u) = self.udps.remove(&port) {
                    os.close(port);
                    self.unpin(u.handle);
                    self.unpin(u.handler);
                }
            }
            Effect::Connect(c) => self.connect(book, def, os, c),
            Effect::Send(c, data) => {
                if let Some(x) = self.conns.get_mut(&c)
                    && !x.released
                {
                    x.out.extend_from_slice(&data);
                }
                self.pump_out(book, def, os, c);
            }
            Effect::Close(c) => {
                if let Some(x) = self.conns.get_mut(&c) {
                    x.closing = true;
                }
                self.pump_out(book, def, os, c);
            }
            Effect::Abort(c) => self.abort(book, def, os, c),
            Effect::Unlisten(l) => {
                if let Some(x) = self.listeners.remove(&l) {
                    os.tcp_close(x.sock);
                    self.by_listener.remove(&x.sock);
                    self.unpin(x.handle);
                    self.unpin(x.handlers);
                }
            }
            Effect::Respond(id, rep) => {
                if let Some(s) = self.serve.as_mut() {
                    s.respond(os, id, rep);
                }
            }
            Effect::Request(id, req) => {
                let a = self.agent.submit(os, req);
                self.by_agent.insert(a, id);
                for d in self.agent.take_done() {
                    self.enqueue(book, def, os, Pending::Answer(d));
                }
            }
            Effect::Resolve(id, name) => {
                let l = os.resolve(&name, netpkt::dns::TYPE_A);
                self.by_lookup.insert(l, id);
            }
            Effect::Disk(id) => self.disk_start(book, def, os, id),
        }
        os.cause = saved;
        halt
    }

    fn connect(&mut self, book: &mut Book, def: &Def, os: &mut OsApi, c: u64) {
        let Some(x) = self.conns.get(&c) else {
            return;
        };
        match os.tcp_connect_msg(x.remote.0, x.remote.1, SockOpts::default()) {
            Ok(sock) => {
                let local = os.tcp_info(sock).map(|r| r.local.1);
                let handle = x.handle;
                if let Some(x) = self.conns.get_mut(&c) {
                    x.sock = Some(sock);
                }
                self.by_sock.insert(sock, c);
                if let (Some(vm), Some(p)) = (self.vm.as_mut(), local) {
                    let m = vm.pinned(handle);
                    if let Ok(k) = vm.new_str("port") {
                        let _ = vm.map_set(m, k, Value::Int(p as i64));
                    }
                }
            }
            Err(_) => self.enqueue(book, def, os, Pending::Ended(c, Why::Failed)),
        }
    }

    /// sends what waits; closes once everything went and the close came
    fn pump_out(&mut self, book: &mut Book, def: &Def, os: &mut OsApi, c: u64) {
        let Some(x) = self.conns.get_mut(&c) else {
            return;
        };
        let Some(sock) = x.sock else {
            return;
        };
        if x.released || !x.open {
            return;
        }
        while !x.out.is_empty() {
            let n = os.tcp_send(sock, &x.out);
            if n == 0 {
                break;
            }
            x.out.drain(..n);
        }
        if x.out.is_empty() && x.closing {
            os.tcp_close(sock);
            x.released = true;
            let ended = x.ended;
            self.by_sock.remove(&sock);
            if ended {
                self.forget(c);
            } else {
                self.enqueue(book, def, os, Pending::Ended(c, Why::Local));
            }
        }
    }

    fn abort(&mut self, book: &mut Book, def: &Def, os: &mut OsApi, c: u64) {
        let Some(x) = self.conns.get_mut(&c) else {
            return;
        };
        if let Some(sock) = x.sock
            && !x.released
        {
            os.tcp_abort(sock);
            self.by_sock.remove(&sock);
        }
        x.released = true;
        x.out.clear();
        if x.ended {
            self.forget(c);
        } else {
            self.enqueue(book, def, os, Pending::Ended(c, Why::Aborted));
        }
    }

    /// a connection is over for the script
    fn forget(&mut self, c: u64) {
        let Some(x) = self.conns.remove(&c) else {
            return;
        };
        if let Some(s) = x.sock {
            self.by_sock.remove(&s);
        }
        if let Some(r) = x.href {
            self.handles.remove(&r);
        }
        self.unpin(x.handle);
        self.unpin(x.handlers);
    }

    /// the disk access of an operation starts
    fn disk_start(&mut self, book: &mut Book, def: &Def, os: &mut OsApi, id: u64) {
        let Some(op) = self.disk.get(&id) else {
            return;
        };
        let size = |k: &str| book.store.get(k).map_or(0, |v| (k.len() + v.len()) as u64);
        let done = match &op.kind {
            DiskKind::Get(k) => os.disk_read(size(k)),
            DiskKind::Scan(p) => {
                let bytes: u64 = book.store.range::<str, _>((core::ops::Bound::Included(p.as_str()), core::ops::Bound::Unbounded)).take_while(|(k, _)| k.starts_with(p.as_str())).map(|(k, v)| (k.len() + v.len()) as u64).sum();
                os.disk_read(bytes)
            }
            DiskKind::Put(k, v) => os.disk_write((k.len() + v.len()) as u64, 0).unwrap_or(os.now()),
            DiskKind::Delete(k) => os.disk_write(k.len() as u64, 0).unwrap_or(os.now()),
        };
        if done <= os.now() {
            self.disk_done(book, def, os, id);
        } else {
            self.wheel.add(done, Due::Disk(id));
        }
    }

    /// a disk access is done: the store changes now, the callback waits for the CPU
    fn disk_done(&mut self, book: &mut Book, def: &Def, os: &mut OsApi, id: u64) {
        let Some(op) = self.disk.get_mut(&id) else {
            return;
        };
        op.out = match &op.kind {
            DiskKind::Get(k) => DiskOut::Value(book.store.get(k).cloned()),
            DiskKind::Put(k, v) => {
                let new = (k.len() + v.len()) as u64;
                let old = book.store.get(k).map_or(0, |o| (k.len() + o.len()) as u64);
                if new > old && !os.disk_reserve(new - old) {
                    os.note(Note::DiskFull);
                    DiskOut::Full
                } else {
                    if old > new {
                        os.disk_free(old - new);
                    }
                    book.store.insert(k.clone(), v.clone());
                    DiskOut::Nothing
                }
            }
            DiskKind::Delete(k) => match book.store.remove(k) {
                Some(v) => {
                    os.disk_free((k.len() + v.len()) as u64);
                    DiskOut::Had(true)
                }
                None => DiskOut::Had(false),
            },
            DiskKind::Scan(p) => {
                let items = book.store.range::<str, _>((core::ops::Bound::Included(p.as_str()), core::ops::Bound::Unbounded)).take_while(|(k, _)| k.starts_with(p.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
                DiskOut::Items(items)
            }
        };
        if op.cb.is_some() {
            self.enqueue(book, def, os, Pending::Disk(id));
        } else {
            self.disk.remove(&id);
        }
    }

    /// an event gets the CPU: the callback it calls, if any
    fn dispatch(&mut self, book: &mut Book, def: &Def, os: &mut OsApi, p: Pending, cause: Option<HopId>) {
        let during = match &p {
            Pending::Top => "start",
            Pending::Timer(_) => "timer",
            Pending::Udp(..) => "udp",
            Pending::Accept(_) | Pending::Accepted(_) => "accept",
            Pending::Connected(_) => "connect",
            Pending::Data(_) => "data",
            Pending::Ended(..) => "close",
            Pending::Http(_) => "http",
            Pending::Answer(_) => "request",
            Pending::Dns(_) => "dns",
            Pending::Disk(_) => "disk",
        };
        match self.prepare(os, p, cause) {
            Ok(Some(c)) => self.begin(book, def, os, c, cause),
            Ok(None) => {}
            Err(e) => {
                let now = os.now();
                self.fail(book, e, during, now, cause, def.cfg.stop_on_error);
            }
        }
    }

    /// the callback of an event and its arguments
    fn prepare(&mut self, os: &mut OsApi, p: Pending, cause: Option<HopId>) -> Result<Option<Call>, script::Error> {
        let Some(vm) = self.vm.as_mut() else {
            return Ok(None);
        };
        let handler = |vm: &Vm, h: Pin, name: &str| vm.map_get_str(vm.pinned(h), name).unwrap_or(Value::Nil);
        Ok(match p {
            Pending::Top => Some(Call { top: true, ..Call::new(Value::Nil, Vec::new(), "start") }),
            Pending::Timer(t) => {
                let Some(tm) = self.timers.get_mut(&t) else {
                    return Ok(None);
                };
                tm.queued = false;
                let (f, once) = (vm.pinned(tm.pin), tm.period.is_none());
                let unpin = if once { self.timers.remove(&t).map(|x| x.pin) } else { None };
                Some(Call { unpin, ..Call::new(f, Vec::new(), "timer") })
            }
            Pending::Udp(port, d) => {
                let Some(u) = self.udps.get(&port).copied() else {
                    return Ok(None);
                };
                let data = if u.binary { vm.new_bytes(&d.data)? } else { vm.new_str(&String::from_utf8_lossy(&d.data))? };
                let from = format!("{}", d.src);
                let msg = vm_map(vm, &[("data", F::V(data)), ("from", F::S(&from)), ("port", F::I(d.sport as i64))])?;
                Some(Call::new(vm.pinned(u.handler), vec![vm.pinned(u.handle), msg], "udp"))
            }
            Pending::Accept(l) => {
                let Some(x) = self.listeners.get_mut(&l) else {
                    return Ok(None);
                };
                x.accept_queued = false;
                let x = *x;
                let mut fresh = Vec::new();
                while let Some(s) = os.tcp_accept(x.sock) {
                    let row = os.tcp_info(s);
                    let remote = row.as_ref().map_or((Ipv4::UNSPECIFIED, 0), |r| r.remote);
                    let local = row.as_ref().map_or(x.port, |r| r.local.1);
                    self.next += 1;
                    let id = self.next;
                    let peer = format!("{}", remote.0);
                    let m = vm_map(vm, &[("kind", F::S("tcp")), ("id", F::I(id as i64)), ("peer", F::S(&peer)), ("peer_port", F::I(remote.1 as i64)), ("port", F::I(local as i64))])?;
                    let handle = vm.pin(m);
                    let hv = vm.pinned(x.handlers);
                    let handlers = vm.pin(hv);
                    let href = ref_of(m);
                    if let Some(r) = href {
                        self.handles.insert(r, Handle::Conn(id));
                    }
                    let mut conn = Conn::new(handle, href, handlers, remote, x.binary);
                    conn.sock = Some(s);
                    conn.open = true;
                    self.conns.insert(id, conn);
                    self.by_sock.insert(s, id);
                    fresh.push(id);
                }
                for id in fresh.into_iter().rev() {
                    self.queue.push_front((Pending::Accepted(id), cause));
                }
                None
            }
            Pending::Accepted(c) => {
                let Some(x) = self.conns.get(&c) else {
                    return Ok(None);
                };
                Some(Call::new(handler(vm, x.handlers, "accept"), vec![vm.pinned(x.handle)], "accept"))
            }
            Pending::Connected(c) => {
                let Some(x) = self.conns.get(&c) else {
                    return Ok(None);
                };
                Some(Call::new(handler(vm, x.handlers, "connect"), vec![vm.pinned(x.handle)], "connect"))
            }
            Pending::Data(c) => {
                let Some(x) = self.conns.get_mut(&c) else {
                    return Ok(None);
                };
                x.data_queued = false;
                let Some(sock) = x.sock.filter(|_| !x.released) else {
                    return Ok(None);
                };
                let d = os.tcp_recv(sock, usize::MAX);
                if d.is_empty() {
                    return Ok(None);
                }
                self.data_call(c, d)?
            }
            Pending::Ended(c, why) => {
                let Some(x) = self.conns.get_mut(&c) else {
                    return Ok(None);
                };
                if x.ended {
                    return Ok(None);
                }
                // what is still in the socket comes first
                if let Some(sock) = x.sock.filter(|_| !x.released) {
                    let d = os.tcp_recv(sock, usize::MAX);
                    if !d.is_empty() {
                        self.queue.push_front((Pending::Ended(c, why), cause));
                        return self.data_call(c, d);
                    }
                }
                x.ended = true;
                let then = match why {
                    Why::Peer => Then::Close(c),
                    Why::End(_) => Then::Release(c),
                    Why::Local | Why::Aborted | Why::Failed => Then::Forget(c),
                };
                let (handle, handlers) = (x.handle, x.handlers);
                let w = vm.new_str(why.word())?;
                Some(Call { then, ..Call::new(handler(vm, handlers, "close"), vec![vm.pinned(handle), w], "close") })
            }
            Pending::Http(r) => {
                let Some(f) = self.serve_fn.map(|p| vm.pinned(p)) else {
                    return Ok(None);
                };
                let req = req_map(vm, &r)?;
                let res = vm_map(vm, &[("kind", F::S("response")), ("id", F::I(r.id as i64))])?;
                let pin = vm.pin(res);
                let href = ref_of(res);
                if let Some(h) = href {
                    self.handles.insert(h, Handle::Res(r.id));
                }
                self.open.insert(r.id, (pin, href));
                Some(Call { req: Some(r.id), ..Call::new(f, vec![req, res], "http") })
            }
            Pending::Answer(d) => {
                let Some(id) = self.by_agent.remove(&d.id) else {
                    return Ok(None);
                };
                let Some(cb) = self.requests.remove(&id) else {
                    return Ok(None);
                };
                let (err, resp) = match &d.result {
                    Ok(r) => (Value::Nil, resp_map(vm, r)?),
                    Err(f) => (vm.new_str(&f.text())?, Value::Nil),
                };
                Some(Call { unpin: Some(cb), ..Call::new(vm.pinned(cb), vec![err, resp], "request") })
            }
            Pending::Dns(l) => {
                let Some(id) = self.by_lookup.remove(&l) else {
                    return Ok(None);
                };
                let Some(cb) = self.resolves.remove(&id) else {
                    return Ok(None);
                };
                let (err, addrs) = match os.lookup(l).and_then(|x| x.result.clone()) {
                    Some(Ok(a)) => {
                        let mut items = Vec::with_capacity(a.addrs.len());
                        for x in &a.addrs {
                            items.push(vm.new_str(&format!("{}", x))?);
                        }
                        (Value::Nil, vm.new_list(items)?)
                    }
                    Some(Err(e)) => (vm.new_str(e.text())?, Value::Nil),
                    None => (vm.new_str("keine Antwort")?, Value::Nil),
                };
                Some(Call { unpin: Some(cb), ..Call::new(vm.pinned(cb), vec![err, addrs], "dns") })
            }
            Pending::Disk(id) => {
                let Some(op) = self.disk.remove(&id) else {
                    return Ok(None);
                };
                let Some(cb) = op.cb else {
                    return Ok(None);
                };
                let args = match op.out {
                    DiskOut::Full => vec![vm.new_str("Die Disk ist voll.")?],
                    DiskOut::Value(v) => {
                        let v = match v {
                            Some(t) => vm.json_decode(&t).unwrap_or(Value::Nil),
                            None => Value::Nil,
                        };
                        vec![Value::Nil, v]
                    }
                    DiskOut::Had(b) => vec![Value::Nil, Value::Bool(b)],
                    DiskOut::Items(items) => {
                        let m = vm.new_map()?;
                        for (k, t) in items {
                            let k = vm.new_str(&k)?;
                            let v = vm.json_decode(&t).unwrap_or(Value::Nil);
                            vm.map_set(m, k, v)?;
                        }
                        vec![Value::Nil, m]
                    }
                    DiskOut::Nothing => vec![Value::Nil],
                };
                Some(Call { unpin: Some(cb), ..Call::new(vm.pinned(cb), args, "disk") })
            }
        })
    }

    /// the data callback of a connection
    fn data_call(&mut self, c: u64, d: Vec<u8>) -> Result<Option<Call>, script::Error> {
        let (Some(vm), Some(x)) = (self.vm.as_mut(), self.conns.get_mut(&c)) else {
            return Ok(None);
        };
        let v = if x.binary {
            vm.new_bytes(&d)?
        } else {
            let s = utf8_chunk(&mut x.carry, d);
            if s.is_empty() {
                return Ok(None);
            }
            vm.new_str(&s)?
        };
        let f = vm.map_get_str(vm.pinned(x.handlers), "data").unwrap_or(Value::Nil);
        Ok(Some(Call::new(f, vec![vm.pinned(x.handle), v], "data")))
    }

    /// a callback takes the CPU
    fn begin(&mut self, book: &mut Book, def: &Def, os: &mut OsApi, c: Call, cause: Option<HopId>) {
        let now = os.now();
        let Some(vm) = self.vm.as_mut() else {
            return;
        };
        if !c.top {
            if !is_fn(c.f) {
                if let Some(p) = c.unpin {
                    vm.unpin(p);
                }
                if let Some(id) = c.req {
                    // nobody handles it: the server cannot answer
                    self.answer(id, None, now, cause);
                }
                self.then(os, c.then, now);
                return;
            }
            if let Err(e) = vm.call_value(c.f, &c.args) {
                if let Some(p) = c.unpin {
                    vm.unpin(p);
                }
                self.then(os, c.then, now);
                self.fail(book, e, c.during, now, cause, def.cfg.stop_on_error);
                return;
            }
        }
        if let Some(p) = c.unpin {
            vm.unpin(p);
        }
        self.inv = Some(Inv { start: now, cause, during: c.during, req: c.req, then: c.then, top: c.top });
    }

    /// one slice of the callback on the CPU
    fn slice(&mut self, book: &mut Book, def: &Def, os: &mut OsApi) {
        let Some(inv) = self.inv else {
            return;
        };
        let Some(mut vm) = self.vm.take() else {
            self.inv = None;
            return;
        };
        let from = os.now();
        let st = {
            let mut api = Api { run: &mut *self, book: &mut *book, def, os: &mut *os, inv };
            vm.run(&mut api, def.cfg.slice)
        };
        let out = vm.take_output();
        if !out.is_empty() {
            book.print(&out);
        }
        // a slice takes 1 ns at least (time goes on), a callback too
        let at = inv.start.saturating_add(cpu(vm.call_fuel(), def.cfg.ps_per_op));
        let end = match st {
            Status::Yielded => at.max(from + 1),
            _ => at.max(inv.start + 1).max(from),
        };
        book.stats.fuel += vm.last_fuel();
        book.stats.busy_ns += end - from;
        book.busy.add(from, end);
        self.busy_until = end;
        let answer = match &st {
            Status::Done(v) if inv.req.is_some() => reply_from(&mut vm, *v),
            _ => None,
        };
        self.vm = Some(vm);
        match st {
            Status::Yielded => {}
            Status::Done(_) => {
                self.inv = None;
                book.stats.calls += 1;
                if let (Some(id), Some(rep)) = (inv.req, answer) {
                    self.answer(id, Some(rep), end, inv.cause);
                }
                self.then(os, inv.then, end);
            }
            Status::Error(e) => {
                self.inv = None;
                book.stats.calls += 1;
                if let Some(id) = inv.req {
                    self.answer(id, None, end, inv.cause);
                }
                self.then(os, inv.then, end);
                self.fail(book, e, inv.during, end, inv.cause, inv.top || def.cfg.stop_on_error);
            }
        }
    }

    /// answers a request of http.serve that is still open (None: 500)
    fn answer(&mut self, id: u64, rep: Option<Reply>, at: Ns, cause: Option<HopId>) {
        let Some((pin, href)) = self.open.remove(&id) else {
            return;
        };
        self.unpin(pin);
        if let Some(r) = href {
            self.handles.remove(&r);
        }
        let rep = rep.unwrap_or_else(|| Reply::status_page(500));
        self.effects.push_back((at, cause, Effect::Respond(id, rep)));
    }

    /// what follows a callback
    fn then(&mut self, os: &mut OsApi, then: Then, at: Ns) {
        match then {
            Then::Nothing => {}
            Then::Close(c) => {
                if let Some(x) = self.conns.get_mut(&c) {
                    x.closed = true;
                }
                self.effects.push_back((at, None, Effect::Close(c)));
            }
            Then::Release(c) => {
                if let Some(x) = self.conns.get_mut(&c)
                    && let Some(s) = x.sock.filter(|_| !x.released)
                {
                    x.released = true;
                    os.tcp_close(s);
                }
                self.forget(c);
            }
            Then::Forget(c) => self.forget(c),
        }
    }

    /// an error of a callback (or of what was to start one): kept, logged and noted at its
    /// moment; `end`: the script ends then
    fn fail(&mut self, book: &mut Book, e: script::Error, during: &'static str, at: Ns, cause: Option<HopId>, end: bool) {
        let err = ScriptErr { at, kind: e.kind, line: e.line, col: e.col, msg: e.msg, trace: e.trace, during };
        let text = if err.line > 0 { format!("Fehler in Zeile {}, Spalte {} ({}): {}", err.line, err.col, during, err.msg) } else { format!("Fehler ({}): {}", during, err.msg) };
        self.effects.push_back((at, cause, Effect::Log(SEV_ERR, text)));
        self.effects.push_back((at, cause, Effect::Note(Note::ScriptError { svc: book.name.clone(), line: err.line, col: err.col, msg: err.msg.clone() })));
        book.stats.errors += 1;
        if book.errors.len() >= ERRORS_KEEP {
            book.errors.pop_front();
        }
        book.errors.push_back(err);
        if end {
            self.effects.push_back((at, cause, Effect::Halt));
            self.halting = true;
            self.busy_until = self.busy_until.max(at);
        }
    }
}

// ---------------------------------------------------------------- the service

impl ScriptService {
    fn from_parts(source: String, prog: Program, cfg: ScriptCfg, conf: String, disk: Option<(Ns, u64, u64, u64)>) -> ScriptService {
        ScriptService::of_def(Def { source, prog, cfg, conf, disk })
    }

    fn of_def(def: Def) -> ScriptService {
        let book = Book {
            name: String::new(),
            state: ScriptState::Stopped,
            since: 0,
            stats: Stats::default(),
            errors: VecDeque::new(),
            output: String::new(),
            metrics: Vec::new(),
            busy: Busy::default(),
            store: BTreeMap::new(),
            printed: 0,
            runs: 0,
            out_mark: 0,
            err_mark: 0,
        };
        ScriptService { def: Arc::new(def), run: None, book, seen: 0 }
    }

    /// a script of `source` with the settings `cfg` and the script's `config` (a JSON object);
    /// the compiler's errors when it has some
    pub fn new(source: &str, cfg: ScriptCfg, config: &Json) -> Result<ScriptService, Vec<script::Diagnostic>> {
        let prog = script::compile(source)?;
        let conf = agent_core::json::stringify(config).unwrap_or_else(|_| String::from("{}"));
        Ok(ScriptService::from_parts(String::from(source), prog, cfg, conf, None))
    }

    pub fn source(&self) -> &str {
        &self.def.source
    }
    pub fn cfg(&self) -> &ScriptCfg {
        &self.def.cfg
    }
    pub fn state(&self) -> ScriptState {
        self.book.state
    }
    /// the last errors, oldest first
    pub fn errors(&self) -> Vec<ScriptErr> {
        self.book.errors.iter().cloned().collect()
    }
    /// what it printed last (print), at most 16 KiB
    pub fn output(&self) -> &str {
        &self.book.output
    }
    /// the errors of the current run (since its last start; stopped: of its last run), oldest
    /// first, as far as the last ERRORS_KEEP reach
    pub fn run_errors(&self) -> Vec<ScriptErr> {
        let b = &self.book;
        let n = (b.stats.errors.saturating_sub(b.err_mark)).min(b.errors.len() as u64) as usize;
        b.errors.iter().skip(b.errors.len() - n).cloned().collect()
    }
    /// what the current run printed from byte `from` on (counted over all the script ever
    /// printed; 0: the whole run, as far as the kept 16 KiB reach) and where that ends, the
    /// `from` of the next call
    pub fn run_output(&self, from: u64) -> (&str, u64) {
        let b = &self.book;
        let first = b.printed - b.output.len() as u64;
        let start = from.max(b.out_mark).max(first).min(b.printed);
        // (marks and ends fall between two prints, the cut of the kept bytes on a character)
        let mut i = (start - first) as usize;
        while !b.output.is_char_boundary(i) {
            i += 1;
        }
        (&b.output[i..], b.printed)
    }
    /// its CPU time in all
    pub fn busy_ns(&self) -> u64 {
        self.book.stats.busy_ns
    }
    /// a callback has the CPU at `now`
    pub fn busy(&self, now: Ns) -> bool {
        self.run.as_ref().is_some_and(|r| r.inv.is_some() || r.busy_until > now)
    }
    /// its CPU time in the `secs` seconds up to `now` (top: %CPU)
    pub fn load(&self, now: Ns, secs: u64) -> u64 {
        self.book.busy.sum(now, secs)
    }
    /// the shell's and the app's view
    pub fn view(&self, now: Ns) -> ScriptView {
        let s = &self.book.stats;
        let run = self.run.as_deref();
        let mut ports = Vec::new();
        if let Some(r) = run {
            if let Some(sv) = &r.serve {
                ports.push(("http", sv.cfg.port));
            }
            ports.extend(r.listeners.values().map(|l| ("tcp", l.port)));
            ports.extend(r.udps.keys().map(|p| ("udp", *p)));
        }
        ScriptView {
            state: self.book.state,
            busy: self.busy(now),
            since: self.book.since,
            calls: s.calls,
            errors: s.errors,
            fuel: s.fuel,
            busy_ns: s.busy_ns,
            queued: run.map_or(0, |r| r.queue.len()),
            max_queued: s.max_queued,
            dropped: s.dropped,
            rejected: s.rejected,
            timers: run.map_or(0, |r| r.timers.len()),
            conns: run.map_or(0, |r| r.conns.len()),
            ports,
            heap: run.and_then(|r| r.vm.as_ref()).map_or(0, |v| v.heap_bytes()),
            cfg: self.def.cfg,
            lines: self.def.source.lines().count(),
            metrics: self.own_metrics(now).into_iter().map(|m| (m.name, m.value)).collect(),
            runs: self.book.runs,
        }
    }

    fn own_metrics(&self, now: Ns) -> Vec<Metric> {
        let mut m = Vec::new();
        for u in &self.book.metrics {
            match &u.v {
                UVal::Num(x) => m.push(Metric::new(u.name, *x)),
                UVal::Hist(w, n) => {
                    let h = w.hist(now);
                    let q = |v: Option<u64>| v.map_or(0, |x| x.min(i64::MAX as u64) as i64);
                    m.push(Metric::new(n[0], h.count() as i64));
                    m.push(Metric::new(n[1], q(h.p50())));
                    m.push(Metric::new(n[2], q(h.p95())));
                    m.push(Metric::new(n[3], q(h.p99())));
                    m.push(Metric::new(n[4], q(h.max())));
                }
            }
        }
        m
    }

    /// the program ends for good (an error): it cleans up, the OS stops it
    fn end(&mut self, os: &mut OsApi) {
        self.run = None;
        self.book.state = ScriptState::Failed;
        os.halt();
    }

    /// after every call: the CPU, then the timer key
    fn settle(&mut self, os: &mut OsApi) {
        let Some(run) = self.run.as_deref_mut() else {
            return;
        };
        if run.pump(&mut self.book, &self.def, os) {
            self.end(os);
            return;
        }
        run.arm(os);
    }
}

impl Service for ScriptService {
    fn kind(&self) -> &'static str {
        "script"
    }
    fn start(&mut self, os: &mut OsApi) {
        let now = os.now();
        self.seen = now;
        self.book.name = os.svc_name();
        self.book.since = now;
        self.book.state = ScriptState::Running;
        // a new run: its output and errors begin here
        self.book.runs += 1;
        self.book.out_mark = self.book.printed;
        self.book.err_mark = self.book.stats.errors;
        if let Some((lat, bps, iops, cap)) = self.def.disk {
            os.disk_setup(lat, bps, iops, cap);
        }
        let a = os.rand() as u64;
        let b = os.rand() as u64;
        let rng = Pcg32::new((a << 32) | b, os.svc_id() as u64);
        let mut vm = Vm::new(&self.def.prog, self.def.cfg.limits);
        if let Ok(c) = vm.json_decode(&self.def.conf) {
            vm.set_global("config", c);
        }
        let mut run = Run::new(vm, rng);
        run.busy_until = now;
        run.queue.push_back((Pending::Top, None));
        self.run = Some(Box::new(run));
        self.settle(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        self.seen = os.now();
        let Some(run) = self.run.as_deref_mut() else {
            return;
        };
        run.event(&mut self.book, &self.def, os, ev);
        self.settle(os);
    }
    fn stop(&mut self, _os: &mut OsApi) {
        self.run = None;
        if self.book.state == ScriptState::Running {
            self.book.state = ScriptState::Stopped;
        }
    }
    fn renamed(&mut self, name: &str) {
        // the log lines and notes of the running program carry the new name
        self.book.name = String::from(name);
    }
    fn metrics(&self) -> Vec<Metric> {
        self.metrics_at(self.seen)
    }
    fn metric_kind(&self, name: &str) -> MetricKind {
        // the script's own: inc counts, gauge is a level, an observed histogram's count is the
        // level of its last five minutes (its quantiles come from windows()); of the service's
        // numbers the CPU time only grows, timers, connections and the heap are levels
        for u in &self.book.metrics {
            match &u.v {
                UVal::Num(_) if u.name == name => return u.kind,
                UVal::Hist(_, n) if n[0] == name => return MetricKind::Gauge,
                _ => {}
            }
        }
        match name {
            "busy_us" | "fuel" => MetricKind::Counter,
            "timers" | "conns" | "heap_kb" => MetricKind::Gauge,
            n => metric_kind_of(n),
        }
    }
    fn windows(&self) -> Vec<LatWindow<'_>> {
        self.book.metrics.iter().filter_map(|u| match &u.v {
            UVal::Hist(w, n) => Some(LatWindow::raw(w, lat_stats(n))),
            UVal::Num(_) => None,
        }).collect()
    }
    fn metrics_at(&self, now: Ns) -> Vec<Metric> {
        let mut m = self.own_metrics(now);
        let s = &self.book.stats;
        let run = self.run.as_deref();
        let n = |x: u64| x.min(i64::MAX as u64) as i64;
        m.extend([
            Metric::new("calls", n(s.calls)),
            Metric::new("errors", n(s.errors)),
            Metric::new("fuel", n(s.fuel)),
            Metric::new("busy_us", n(s.busy_ns / US)),
            Metric::new("queued", run.map_or(0, |r| r.queue.len()) as i64),
            Metric::new("max_queued", n(s.max_queued)),
            Metric::new("dropped", n(s.dropped)),
            Metric::new("rejected", n(s.rejected)),
            Metric::new("timers", run.map_or(0, |r| r.timers.len()) as i64),
            Metric::new("conns", run.map_or(0, |r| r.conns.len()) as i64),
            Metric::new("heap_kb", n(run.and_then(|r| r.vm.as_ref()).map_or(0, |v| v.heap_bytes() / 1024))),
        ]);
        m
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        let s = &self.book.stats;
        match (self.book.state, self.book.errors.back()) {
            (ScriptState::Failed, Some(e)) => msg!("beendet nach einem Fehler in Zeile {}, Spalte {}: {}", e.line, e.col, e.msg.clone()),
            (ScriptState::Failed, None) => msg!("beendet nach einem Fehler"),
            (ScriptState::Stopped, _) => msg!("gestoppt"),
            (ScriptState::Running, None) => msg!("läuft: Rückrufe {}, CPU-Zeit {}", s.calls, dur(s.busy_ns as i64)),
            (ScriptState::Running, Some(e)) => {
                msg!("läuft: Rückrufe {}, CPU-Zeit {}, Fehler {}, zuletzt in Zeile {}, Spalte {}: {}", s.calls, dur(s.busy_ns as i64), s.errors, e.line, e.col, e.msg.clone())
            }
        }
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
}

// ---------------------------------------------------------------- project definitions

/// a script from its definition in a project file (svcact.rs); the German reason when the
/// configuration does not fit
pub fn from_def(c: &Json) -> Result<Box<dyn Service>, String> {
    from_def_msg(c).map_err(|m| m.text())
}

/// from_def, the reason as a message for the app's catalogue
pub fn from_def_msg(c: &Json) -> Result<Box<dyn Service>, Msg> {
    Ok(Box::new(ScriptService::of_def(def_of(c)?)))
}

/// what a definition makes: the source, the program, the settings, the script's config and
/// the disk; the refusal as a message
fn def_of(c: &Json) -> Result<Def, Msg> {
    let lines = || msg!("\u{201e}source\u{201c} ist ein Text oder eine Liste von Zeilen.");
    let source = match c.get("source") {
        Some(Json::String(s)) => s.clone(),
        Some(Json::Array(list)) => {
            let mut s = String::new();
            for l in list {
                s.push_str(l.as_str().ok_or_else(lines)?);
                s.push('\n');
            }
            s
        }
        Some(_) => return Err(lines()),
        None => return Err(msg!("\u{201e}source\u{201c} fehlt: der Text des Skripts.")),
    };
    let cfg = cfg_of(c)?;
    let prog = match script::compile(&source) {
        Ok(p) => p,
        Err(d) => {
            let Some(first) = d.first() else {
                return Err(msg!("Das Skript lässt sich nicht übersetzen."));
            };
            return Err(if d.len() > 1 {
                msg!("Zeile {}, Spalte {}: {} (und {} weitere Fehler)", first.line, first.col, first.msg.clone(), d.len() - 1)
            } else {
                msg!("Zeile {}, Spalte {}: {}", first.line, first.col, first.msg.clone())
            });
        }
    };
    // the script's config: the definition without its source
    let mut conf = Json::Object(Vec::new());
    for (k, v) in c.as_object().unwrap_or(&[]) {
        if k != "source" {
            conf.insert(k, v.clone());
        }
    }
    let conf = agent_core::json::stringify(&conf).unwrap_or_else(|_| String::from("{}"));
    Ok(Def { source, prog, cfg, conf, disk: crate::disk::setup_of(c) })
}

/// the settings of a definition
fn cfg_of(c: &Json) -> Result<ScriptCfg, Msg> {
    let mut cfg = ScriptCfg::default();
    if let Some(v) = c.get("ns_per_op") {
        cfg.ps_per_op = match crate::httpsvc::decimal(v, 1000) {
            Some(p) if p > 0 => p,
            _ => return Err(msg!("\u{201e}ns_per_op\u{201c} muss eine Zahl größer als 0 sein.")),
        };
    }
    if let Some(n) = c.get("slice").and_then(|v| v.as_u64()) {
        cfg.slice = n.clamp(1000, 100_000_000);
    }
    if let Some(n) = c.get("queue").and_then(|v| v.as_u64()) {
        cfg.queue = n.clamp(1, 1_000_000) as usize;
    }
    if let Some(b) = c.get("stop_on_error").and_then(|v| v.as_bool()) {
        cfg.stop_on_error = b;
    }
    if let Some(l) = c.get("limits") {
        let n = |k: &str| l.get(k).and_then(|v| v.as_u64());
        if let Some(f) = n("fuel") {
            cfg.limits.fuel = if f == 0 { u64::MAX } else { f };
        }
        if let Some(k) = n("heap_kb") {
            cfg.limits.heap = k.clamp(64, 1 << 22) << 10;
        }
        if let Some(d) = n("depth") {
            cfg.limits.depth = d.clamp(1, 100_000) as u32;
        }
        if let Some(s) = n("stack") {
            cfg.limits.stack = s.clamp(256, 10_000_000) as u32;
        }
    }
    Ok(cfg)
}

impl Engine {
    /// the scripts of a device: their names and what they do
    pub fn scripts(&self, dev: u32) -> Vec<(String, ScriptView)> {
        let now = self.now;
        self.service_ids(dev).into_iter().filter_map(|(_, name, _, _)| self.service::<ScriptService>(dev, &name).map(|s| (name.clone(), s.view(now)))).collect()
    }

    /// Action::ScriptSource: script `name` takes the definition `c` and keeps its record (disk,
    /// numbers, errors, output) and its service id (the disk's accounting, the time series);
    /// a running program ends first, one that should run starts with the new one
    pub(crate) fn script_source(&mut self, dev: u32, name: &str, c: &Json) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let os = self.os(ni).ok_or(ActErr::NoIp(dev))?;
        let slot = os.svc(name).filter(|s| !s.console).ok_or_else(|| ActErr::Refused(msg!("Den Dienst \u{201e}{}\u{201c} gibt es nicht.", name)))?;
        let (id, running) = (slot.id, slot.running);
        if !slot.svc.as_ref().is_some_and(|s| s.as_any().downcast_ref::<ScriptService>().is_some()) {
            return Err(ActErr::Refused(msg!("Der Dienst \u{201e}{}\u{201c} ist kein Skript.", name)));
        }
        let def = def_of(c).map_err(|w| ActErr::Refused(msg!("Dienst {}: {}", "script", w)))?;
        // the old program ends as with a stop: its sockets, timers and queued events go
        if running {
            self.svc_stop(ni, id);
        }
        let Some(s) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.id == id)) else {
            return Ok(());
        };
        if let Some(x) = s.svc.as_mut().and_then(|x| x.as_any_mut().downcast_mut::<ScriptService>()) {
            x.def = Arc::new(def);
        }
        if s.enabled {
            self.os_push(ni, Job::Start(id));
        }
        Ok(())
    }
}
