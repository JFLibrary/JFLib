// dbtest -- the database service (part of --selftest, group db; jfn_sim's db.rs, dbsql.rs, dbstore.rs): the
// models against closed forms, because learners take the figures for the truth; the invariants of concurrency,
// crash and recovery; every refusal with its words; the form and the JSON it writes
//
// Each comparison names its formula:
//
//   CPU-bound      X = cores * speed / (stmt + pages * page + rows * row)
//   IO-bound       X = IOPS / (1 - hit), hit = (pool - tree) / heap for a uniform key
//   index or scan  time(scan) / time(index) from pages and rows; EXPLAIN says what is charged
//   hit rate       uniform: pool / data; zipf: Che's approximation, T from sum(1 - exp(-p T)) = pool,
//                  hit = sum(p (1 - exp(-p T)))
//   hot row        X = 1 / hold time
//   group commit   one client X = 1 / (flush + round trip); many clients above it
//   recovery       time = log bytes / disk rate + latency + log bytes / redo rate
//
// Floating point is fine here: it only compares results, nothing here feeds the simulation.
// "--selftest --only=db --numbers" prints every such comparison with its numbers.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::{empty_object, Value};
use base::rng::Pcg32;
use jfn_model::catalog::{DiskClass, Hw};
use jfn_model::experiment::Experiment;
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_run::run_sweep;
use jfn_sim::db::Db;
use jfn_sim::dbsql;
use jfn_sim::dbstore::{column, Dist, Layout, Pool, Table, TableSpec, Val};
use jfn_sim::experiment::Plan;
use jfn_sim::http::{Agent, Req, Url};
use jfn_sim::keyframe::{digest, Budget};
use jfn_sim::os::{Metric, OsApi, Service, SvcEvent};
use jfn_sim::res::HOST;
use jfn_sim::{Action, Cmd, Engine, Inline, Ns, Report, MS, SEC, US};
use netpkt::http::Headers;

use crate::doc::{edits, Doc};
use crate::selftest::T;
use crate::workspace::devwin::svcform::{fill, merged, takes, Takes};

const BUDGET: usize = 200_000_000;
const GEN_IP: &str = "10.0.0.1";
const DB_IP: &str = "10.0.0.2";

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

fn cfg(text: &str) -> Value {
    agent_core::json::parse(text.as_bytes()).unwrap_or(Value::Null)
}

fn def_v(kind: &str, name: &str, config: Value) -> ServiceDef {
    ServiceDef { kind: String::from(kind), name: String::from(name), disabled: false, config }
}

fn def(kind: &str, name: &str, config: &str) -> ServiceDef {
    def_v(kind, name, cfg(config))
}

fn hw(cores: u32, speed_pm: u32, ram_gb: u64) -> Hw {
    Hw { cores, speed_pm, ram_mb: ram_gb << 10, swap_mb: 1 << 10, disk: DiskClass::Nvme, disk_gb: 400 }
}

/// a client (no hardware, it never pays) and a database on a 10G cable
struct Net {
    p: Project,
    gen_: u32,
    srv: u32,
}

fn net(srv_hw: Option<Hw>, db_cfg: &str, load: Option<Value>) -> Net {
    let mut p = Project::new("db", 21);
    let gen_ = p.add_device("traffic-gen", 0, 0).unwrap_or(0);
    let srv = p.add_device("server-1u", 200, 0).unwrap_or(0);
    let _ = p.connect(&pr(gen_, "eth0"), &pr(srv, "eth1"), None);
    if let Some(d) = p.device_mut(gen_) {
        d.startup_config = format!("ip addr add {}/24 dev eth0\n", GEN_IP);
        if let Some(l) = load {
            d.services.push(def_v("http-load", "load", l));
        }
    }
    if let Some(d) = p.device_mut(srv) {
        d.startup_config = format!("ip addr add {}/24 dev eth1\n", DB_IP);
        d.services = alloc::vec![def("db", "db", db_cfg)];
        d.hw = srv_hw;
    }
    Net { p, gen_, srv }
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
}

fn run_for(e: &mut Engine, d: Ns) {
    let t = e.now() + d;
    e.run_until(t, BUDGET);
    let _ = e.take_reports();
}

/// users in a closed loop, each sending this body to /query
fn closed(users: u32, body: &str, requests: u64) -> Value {
    let mix = Value::object([("method", Value::string("POST")), ("path", Value::string("/query")), ("weight", Value::u64(1)), ("body", Value::string(body))]);
    let mut o = Value::object([("targets", Value::Array(alloc::vec![Value::string(format!("http://{}:7100/", DB_IP))])), ("mode", Value::string("closed")), ("users", Value::u64(users as u64)), ("timeout_ms", Value::u64(600_000)), ("mix", Value::Array(alloc::vec![mix]))]);
    if requests > 0 {
        o.insert("requests", Value::u64(requests));
    }
    o
}

const DRAW_READ: &str = r#"{"sql":"SELECT * FROM t WHERE id = ?","params":[{"draw":true}]}"#;

fn metric(e: &Engine, dev: u32, svc: &str, name: &str) -> i64 {
    e.metric(dev, svc, name).unwrap_or(-1)
}

fn db<'a>(e: &'a Engine, n: &Net) -> Option<&'a Db> {
    e.service::<Db>(n.srv, "db")
}

/// the statements per second after `warm`, over `secs`
fn rate(e: &mut Engine, n: &Net, warm: Ns, span: Ns) -> f64 {
    run_for(e, warm);
    let a = metric(e, n.srv, "db", "statements");
    run_for(e, span);
    let b = metric(e, n.srv, "db", "statements");
    (b - a) as f64 * SEC as f64 / span as f64
}

/// says under --numbers where it is
fn section(name: &str) {
    if plat::args().iter().any(|a| a == "--numbers") {
        plat::write_stdout(format!("-- {}\n", name).as_bytes());
    }
}

fn check_near(t: &mut T, name: &str, got: f64, want: f64, tol: f64) {
    let line = format!("{}: {:.3} against {:.3} (within {:.1} %)", name, got, want, tol * 100.0);
    if plat::args().iter().any(|a| a == "--numbers") {
        plat::write_stdout(format!("{}, off by {:.2} %\n", line, (got - want).abs() / want.abs().max(1e-12) * 100.0).as_bytes());
    }
    t.check(&line, (got - want).abs() <= want.abs() * tol);
}

// ---------------------------------------------------------------- a client with a plan

/// one statement of a plan: at this moment, in this session (its transaction goes along)
#[derive(Clone, Debug)]
struct Step {
    at: Ns,
    ses: usize,
    sql: String,
    /// another path of the same host: a plain GET there instead of a statement
    get: Option<(String, u16)>,
}

/// what came back
#[derive(Clone, Debug)]
struct Got {
    status: u16,
    body: String,
    at: Ns,
}

/// sends statements at given moments, keeps the answers in the order of the plan
#[derive(Clone, Debug)]
struct Driver {
    host: String,
    steps: Vec<Step>,
    next: usize,
    agent: Agent,
    by_agent: BTreeMap<u64, usize>,
    txn: BTreeMap<usize, u64>,
    got: BTreeMap<usize, Got>,
}

fn number_of(body: &str, name: &str) -> Option<u64> {
    let pat = format!("\"{}\":", name);
    let t = &body[body.find(&pat)? + pat.len()..];
    t[..t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len())].parse().ok()
}

impl Driver {
    fn new(host: &str, steps: Vec<Step>) -> Driver {
        Driver { host: String::from(host), steps, next: 0, agent: Agent::new(1), by_agent: BTreeMap::new(), txn: BTreeMap::new(), got: BTreeMap::new() }
    }
    fn send_due(&mut self, os: &mut OsApi) {
        let now = os.now();
        while let Some(s) = self.steps.get(self.next).cloned() {
            if s.at > now {
                os.timer(0, s.at);
                break;
            }
            let i = self.next;
            self.next += 1;
            let mut req = match &s.get {
                Some((path, port)) => Req::get(Url { host: self.host.clone(), port: *port, target: path.clone() }),
                None => {
                    let mut r = Req::get(Url { host: self.host.clone(), port: 7100, target: String::from("/query") });
                    r.method = String::from("POST");
                    let mut b = String::from("{\"sql\":");
                    Val::Text(s.sql.clone()).json(&mut b);
                    if let Some(t) = self.txn.get(&s.ses) {
                        b.push_str(&format!(",\"txn\":{}", t));
                    }
                    b.push('}');
                    r.body = b.into_bytes();
                    r.headers = Headers::new();
                    r.headers.add("Content-Type", "application/json");
                    r
                }
            };
            req.timeout = 600 * SEC;
            let a = self.agent.submit(os, req);
            self.by_agent.insert(a, i);
        }
    }
}

impl Service for Driver {
    fn kind(&self) -> &'static str {
        "test-driver"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.send_due(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        if let SvcEvent::Timer(0) = ev {
            self.send_due(os);
            return;
        }
        if self.agent.on(&ev, os) {
            for d in self.agent.take_done() {
                let Some(i) = self.by_agent.remove(&d.id) else { continue };
                let (status, body) = match d.result {
                    Ok(r) => (r.head.status, String::from_utf8_lossy(&r.body).into_owned()),
                    Err(f) => (0, f.text()),
                };
                let ses = self.steps[i].ses;
                if body.contains("\"committed\"") || body.contains("\"rolled_back\"") || (status == 409 && body.contains("\"retryable\":true")) {
                    self.txn.remove(&ses);
                } else if let Some(t) = number_of(&body, "txn") {
                    self.txn.insert(ses, t);
                }
                self.got.insert(i, Got { status, body, at: os.now() });
            }
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        Vec::new()
    }
    fn status(&self) -> String {
        String::new()
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

fn plan(list: &[(u64, usize, &str)]) -> Vec<Step> {
    list.iter().map(|(ms, ses, sql)| Step { at: *ms * MS, ses: *ses, sql: String::from(*sql), get: None }).collect()
}

/// the answers of a plan: runs it on this net until `until`
fn driven(n: &Net, steps: Vec<Step>, until: Ns) -> (Engine, Vec<Option<Got>>) {
    let mut e = engine(&n.p);
    let _ = e.add_service(n.gen_, "drv", Box::new(Driver::new(DB_IP, steps.clone())), true);
    run_for(&mut e, until);
    let got = got_of(&e, n.gen_, steps.len());
    (e, got)
}

fn got_of(e: &Engine, dev: u32, n: usize) -> Vec<Option<Got>> {
    let d = e.service::<Driver>(dev, "drv");
    (0..n).map(|i| d.and_then(|x| x.got.get(&i).cloned())).collect()
}

fn status_of(g: &[Option<Got>], i: usize) -> u16 {
    g.get(i).and_then(|x| x.as_ref()).map_or(0, |x| x.status)
}

fn body_of(g: &[Option<Got>], i: usize) -> String {
    g.get(i).and_then(|x| x.as_ref()).map_or(String::new(), |x| x.body.clone())
}

fn has(g: &[Option<Got>], i: usize, what: &str) -> bool {
    body_of(g, i).contains(what)
}

/// sessions that move money between accounts, again and again, starting over after a conflict
#[derive(Clone, Debug)]
struct Bank {
    agent: Agent,
    rng: Pcg32,
    sessions: Vec<Teller>,
    by_agent: BTreeMap<u64, usize>,
    until: Ns,
    done: u64,
    conflicts: u64,
}

#[derive(Clone, Debug, Default)]
struct Teller {
    step: u8,
    txn: Option<u64>,
    from: u64,
    to: u64,
}

impl Bank {
    fn new(n: usize, until: Ns) -> Bank {
        Bank { agent: Agent::new(1), rng: Pcg32::new(7, 7), sessions: alloc::vec![Teller::default(); n], by_agent: BTreeMap::new(), until, done: 0, conflicts: 0 }
    }
    fn send(&mut self, os: &mut OsApi, i: usize) {
        if os.now() >= self.until {
            return;
        }
        let s = &mut self.sessions[i];
        if s.step == 0 {
            let a = self.rng.range(1, 8) as u64;
            let mut b = self.rng.range(1, 8) as u64;
            if b == a {
                b = b % 8 + 1;
            }
            (s.from, s.to) = (a, b);
        }
        let sql = match s.step {
            0 => String::from("BEGIN"),
            1 => format!("UPDATE a SET b = b - 7 WHERE id = {}", s.from),
            2 => format!("UPDATE a SET b = b + 7 WHERE id = {}", s.to),
            _ => String::from("COMMIT"),
        };
        let mut r = Req::get(Url { host: String::from(DB_IP), port: 7100, target: String::from("/query") });
        r.method = String::from("POST");
        let mut b = String::from("{\"sql\":");
        Val::Text(sql).json(&mut b);
        if let Some(t) = s.txn {
            b.push_str(&format!(",\"txn\":{}", t));
        }
        b.push('}');
        r.body = b.into_bytes();
        r.timeout = 60 * SEC;
        let id = self.agent.submit(os, r);
        self.by_agent.insert(id, i);
    }
}

impl Service for Bank {
    fn kind(&self) -> &'static str {
        "test-bank"
    }
    fn start(&mut self, os: &mut OsApi) {
        for i in 0..self.sessions.len() {
            self.send(os, i);
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        if self.agent.on(&ev, os) {
            for d in self.agent.take_done() {
                let Some(i) = self.by_agent.remove(&d.id) else { continue };
                let (status, body) = match d.result {
                    Ok(r) => (r.head.status, String::from_utf8_lossy(&r.body).into_owned()),
                    Err(_) => (0, String::new()),
                };
                let s = &mut self.sessions[i];
                if status != 200 {
                    // its transaction went: all over again
                    self.conflicts += 1;
                    s.step = 0;
                    s.txn = None;
                } else if s.step == 0 {
                    s.txn = number_of(&body, "txn");
                    s.step = 1;
                } else if s.step == 3 {
                    self.done += 1;
                    s.step = 0;
                    s.txn = None;
                } else {
                    s.step += 1;
                }
                self.send(os, i);
            }
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        Vec::new()
    }
    fn status(&self) -> String {
        String::new()
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

pub(crate) fn run(t: &mut T) {
    let parts: [(&str, fn(&mut T)); 36] = [("parser", parser), ("config_refusals", config_refusals), ("layout", layout), ("derived_rows", derived_rows), ("pool_unit", pool_unit), ("cpu_bound", cpu_bound), ("io_bound", io_bound), ("index_or_scan", index_or_scan), ("hit_end_to_end", hit_end_to_end), ("hot_row", hot_row), ("group_commit", group_commit), ("counter", counter), ("transfers", transfers), ("isolation", isolation), ("deadlock", deadlock), ("timeouts", timeouts), ("versions", versions), ("limits", limits), ("statements", statements), ("live_index", live_index), ("crash_full", crash_full), ("crash_relaxed", crash_relaxed), ("cold_start", cold_start), ("oom", oom), ("bound", bound), ("determinism", determinism), ("sweep_threads", sweep_threads), ("routes", routes), ("script", script), ("shell", shell), ("form", form), ("preset", preset), ("too_big", too_big), ("live_change", live_change), ("scan_cheap", scan_cheap), ("costs_of_a_clone", costs_of_a_clone)];
    for (name, f) in parts {
        section(name);
        let from = plat::time_us();
        f(t);
        if plat::args().iter().any(|a| a == "--numbers") {
            plat::write_stdout(format!("-- {}: {} ms\n", name, (plat::time_us() - from) / 1000).as_bytes());
        }
    }
}

// ---------------------------------------------------------------- the parser

fn err_of(sql: &str) -> String {
    dbsql::parse(sql).err().map_or(String::new(), |e| e.line())
}

fn parser(t: &mut T) {
    for ok in [
        "SELECT * FROM users WHERE id = 1",
        "select a, b from t where a >= 1 and b < 'x' order by a desc limit 5",
        "SELECT COUNT(*) FROM t;",
        "SELECT * FROM t WHERE a BETWEEN 1 AND 9 FOR UPDATE",
        "SELECT t.a, u.b FROM t JOIN u ON t.a = u.b WHERE t.a > 3 LIMIT 2",
        "INSERT INTO t (a, b) VALUES (1, 'x''y')",
        "UPDATE t SET a = a + 1, b = 'x', c = c - ? WHERE id = ?",
        "DELETE FROM t",
        "BEGIN",
        "BEGIN ISOLATION LEVEL REPEATABLE READ",
        "commit",
        "ROLLBACK",
        "CREATE INDEX ON t (a)",
        "DROP INDEX ON t (a)",
        "EXPLAIN SELECT * FROM t",
    ] {
        t.check(&format!("reads: {}", ok), dbsql::parse(ok).is_ok());
    }
    for (sql, want) in [
        ("", "Zeichen 1: Hier fehlt eine Anweisung (SELECT, INSERT, UPDATE, DELETE, BEGIN, COMMIT, ROLLBACK, CREATE INDEX, DROP INDEX, EXPLAIN), es steht das Ende da."),
        ("SELEKT 1", "Zeichen 1: Hier fehlt eine Anweisung (SELECT, INSERT, UPDATE, DELETE, BEGIN, COMMIT, ROLLBACK, CREATE INDEX, DROP INDEX, EXPLAIN), es steht \u{201e}SELEKT\u{201c} da."),
        ("SELECT * users", "Zeichen 10: Hier fehlt from, es steht \u{201e}users\u{201c} da."),
        ("SELECT * FROM", "Zeichen 14: Hier fehlt ein Tabellenname, es steht das Ende da."),
        ("SELECT * FROM t WHERE", "Zeichen 22: Hier fehlt ein Spaltenname, es steht das Ende da."),
        ("SELECT * FROM t WHERE a", "Zeichen 24: Hier fehlt ein Vergleich (=, <, <=, >, >=, BETWEEN), es steht das Ende da."),
        ("SELECT * FROM t WHERE a = ", "Zeichen 27: Hier fehlt ein Wert (eine Zahl, 'Text' oder ?), es steht das Ende da."),
        ("SELECT * FROM t WHERE a = 1 OR b = 2", "Zeichen 29: OR gibt es hier nicht: Bedingungen hängen nur mit AND zusammen."),
        ("SELECT * FROM t WHERE NOT a = 1", "Zeichen 23: NOT gibt es hier nicht."),
        ("SELECT * FROM t WHERE (a = 1)", "Zeichen 23: Klammern gibt es in WHERE nicht: Bedingungen hängen nur mit AND zusammen."),
        ("SELECT * FROM t WHERE a LIKE 'x'", "Zeichen 25: \u{201e}LIKE\u{201c} gibt es hier nicht: nur =, <, <=, >, >= und BETWEEN."),
        ("SELECT * FROM t WHERE a != 1", "Zeichen 25: Ungleich gibt es hier nicht: nur =, <, <=, >, >= und BETWEEN."),
        ("SELECT * FROM t WHERE a = NULL", "Zeichen 27: NULL gibt es hier nicht: jede Zeile hat in jeder Spalte einen Wert."),
        ("SELECT SUM(a) FROM t", "Zeichen 8: \u{201e}SUM\u{201c} gibt es hier nicht: von den Aggregaten nur COUNT(*)."),
        ("SELECT * FROM t GROUP BY a", "Zeichen 17: GROUP BY und HAVING gibt es hier nicht."),
        ("SELECT * FROM (SELECT 1)", "Zeichen 15: Unterabfragen gibt es hier nicht."),
        ("SELECT * FROM t LEFT JOIN u ON t.a = u.a", "Zeichen 17: Von den Joins gibt es nur JOIN ... ON a.x = b.y."),
        ("SELECT * FROM t JOIN u ON t.a = u.a JOIN v ON t.a = v.a", "Zeichen 37: Mehr als ein JOIN gibt es hier nicht."),
        ("SELECT * FROM t LIMIT x", "Zeichen 23: Hier fehlt eine Zahl hinter LIMIT, es steht \u{201e}x\u{201c} da."),
        ("SELECT * FROM t x", "Zeichen 17: Hinter der Anweisung steht noch \u{201e}x\u{201c}."),
        ("INSERT INTO t (a, b) VALUES (1)", "Zeichen 29: 2 Spalten, aber 1 Werte."),
        ("INSERT INTO t (a) VALUES (1), (2)", "Zeichen 29: Ein INSERT nimmt hier nur eine Zeile."),
        ("UPDATE t SET a = a * 2", "Zeichen 20: Hier fehlt + oder -, es steht \u{201e}*\u{201c} da."),
        ("CREATE TABLE x (a)", "Zeichen 8: CREATE TABLE gibt es hier nicht: Tabellen stehen in der Konfiguration des Dienstes."),
        ("DROP TABLE x", "Zeichen 6: DROP TABLE gibt es hier nicht: Tabellen stehen in der Konfiguration des Dienstes."),
        ("ALTER TABLE x", "Zeichen 1: ALTER gibt es hier nicht: Tabellen stehen in der Konfiguration des Dienstes."),
        ("EXPLAIN EXPLAIN SELECT * FROM t", "Zeichen 9: Ein EXPLAIN in einem EXPLAIN gibt es nicht."),
        ("EXPLAIN BEGIN", "Zeichen 14: EXPLAIN nimmt SELECT, UPDATE und DELETE."),
        ("SELECT 'abc", "Zeichen 8: Hier beginnt ein Text, der nicht endet: es fehlt das schließende '."),
        ("SELECT * FROM t WHERE a = 99999999999999999999", "Zeichen 27: Die Zahl 99999999999999999999 ist zu groß."),
        ("SELECT @", "Zeichen 8: Das Zeichen @ gibt es hier nicht."),
        ("SELECT * FROM t WHERE ä = 1", "Zeichen 23: Das Zeichen ä gibt es hier nicht."),
    ] {
        t.eq(&format!("the error of {:?}", sql), err_of(sql), String::from(want));
    }
    // a long text costs time, not stack: the grammar has no recursion
    let long = alloc::format!("SELECT * FROM t WHERE {}a = 1", "a = 1 AND ".repeat(500));
    t.check("too many tokens are refused", err_of(&long).contains("höchstens 1024"));
    let huge = "x".repeat(dbsql::MAX_LEN + 1);
    t.check("too long a text is refused", err_of(&huge).contains("höchstens 16384 Bytes"));
    let deep = format!("{}SELECT 1", "EXPLAIN ".repeat(100));
    t.check("no recursion", err_of(&deep).contains("Ein EXPLAIN in einem EXPLAIN"));
    t.eq("two ? counted", dbsql::parse("UPDATE t SET a = ? WHERE id = ?").map(|s| dbsql::params_of(&s)).ok(), Some(2));
}

fn config_refusals(t: &mut T) {
    let verdict = |c: &str| jfn_sim::db::from_def_msg(&cfg(c)).err().map(|m| m.text());
    for (c, want) in [
        ("{}", "\u{201e}tables\u{201c} fehlt: eine Datenbank braucht mindestens eine Tabelle."),
        (r#"{"tables": [{"rows": 1}]}"#, "\u{201e}\u{201c} geht nicht als Name einer Tabelle: Buchstaben, Ziffern und _."),
        (r#"{"tables": [{"name": "a b", "rows": 1}]}"#, "\u{201e}a b\u{201c} geht nicht als Name einer Tabelle: Buchstaben, Ziffern und _."),
        (r#"{"tables": [{"name": "t"}]}"#, "Tabelle t: \u{201e}rows\u{201c} nimmt 1 bis 2^40 Zeilen."),
        (r#"{"tables": [{"name": "t", "rows": 1, "row_bytes": 0}]}"#, "Tabelle t: \u{201e}row_bytes\u{201c} nimmt 1 Byte bis 1 MiB."),
        (r#"{"tables": [{"name": "t", "rows": 1, "dist": "x"}]}"#, "Tabelle t: \u{201e}dist\u{201c} kennt x nicht (uniform, zipf)."),
        (r#"{"tables": [{"name": "t", "rows": 1, "columns": ["a:float"]}]}"#, "Tabelle t: \u{201e}a:float\u{201c} ist keine Spalte (name, name:text, name:int:1000)."),
        (r#"{"tables": [{"name": "t", "rows": 1, "indexes": ["z"]}]}"#, "Tabelle t: den Index z gibt es nicht als Spalte."),
        (r#"{"tables": [{"name": "t", "rows": 1}, {"name": "T", "rows": 1}]}"#, "Die Tabelle T steht zweimal da."),
        (r#"{"tables": [{"name": "t", "rows": 1}], "port": 70000}"#, "Port 70000 gibt es nicht."),
        (r#"{"tables": [{"name": "t", "rows": 1}], "page_kb": 3}"#, "\u{201e}page_kb\u{201c} nimmt 1, 2, 4, 8, 16, 32 oder 64."),
        (r#"{"tables": [{"name": "t", "rows": 1}], "durability": "maybe"}"#, "\u{201e}durability\u{201c} kennt maybe nicht (full, relaxed)."),
        (r#"{"tables": [{"name": "t", "rows": 1}], "isolation": "x"}"#, "\u{201e}isolation\u{201c} kennt x nicht (read committed, repeatable read)."),
    ] {
        t.eq(&format!("the refusal of {}", c), verdict(c), Some(String::from(want)));
    }
    t.check("a good one", verdict(r#"{"tables": [{"name": "t", "rows": 1, "columns": ["a", "b:text", "c:int:5", {"name": "d", "type": "text", "distinct": 9}], "indexes": ["a", "id"]}]}"#).is_none());
    t.eq("what it lacks", jfn_sim::svcform::missing("db", &empty_object()).map(|m| m.text()).as_deref(), Some("Noch keine Tabellen: die Datenbank hat nichts, worauf sich eine Anweisung beziehen könnte."));
    // a route that asks the database
    let web = |c: &str| jfn_sim::httpsvc::from_def_msg("http-server", &cfg(c)).err().map(|m| m.text());
    t.eq("a route without db", web(r#"{"routes": [{"path": "/", "backend": "db", "sql": "BEGIN"}]}"#).as_deref(), Some("Eine Route fragt die Datenbank, aber \u{201e}db\u{201c} fehlt."));
    t.eq("a route without sql", web(r#"{"db": {"addr": "10.0.0.1"}, "routes": [{"path": "/", "backend": "db"}]}"#).as_deref(), Some("Eine Route mit \u{201e}backend\u{201c} db braucht \u{201e}sql\u{201c}."));
    t.eq("a route with bad sql", web(r#"{"db": {"addr": "10.0.0.1"}, "routes": [{"path": "/", "backend": "db", "sql": "NIX"}]}"#).as_deref(), Some("Die Anweisung \u{201e}NIX\u{201c}: Zeichen 1: Hier fehlt eine Anweisung (SELECT, INSERT, UPDATE, DELETE, BEGIN, COMMIT, ROLLBACK, CREATE INDEX, DROP INDEX, EXPLAIN), es steht \u{201e}NIX\u{201c} da."));
    t.eq("a route's param", web(r#"{"db": {"addr": "10.0.0.1"}, "routes": [{"path": "/", "backend": "db", "sql": "BEGIN", "param": "x"}]}"#).as_deref(), Some("\u{201e}param\u{201c} kennt x nicht (rest, draw, query:NAME, none)."));
    t.eq("a db without an address", web(r#"{"db": {"addr": "x"}}"#).as_deref(), Some("\u{201e}db.addr\u{201c}: \u{201e}x\u{201c} ist keine IPv4-Adresse."));
}

// ---------------------------------------------------------------- the tables

fn layout(t: &mut T) {
    let l = Layout::new(8192, 0);
    t.eq("fan-out from the page", l.fanout, 512);
    t.eq("rows per page", l.rows_per_page(200), 40);
    t.eq("heap pages", l.heap_pages(1_000_000, 200), 25_000);
    // depth: 1 + ceil(log_512(leaves))
    t.eq("depth of one leaf", l.depth(500), 1);
    t.eq("depth of 512^2 entries", l.depth(512 * 512), 2);
    t.eq("depth of 512^2 + 1 entries", l.depth(512 * 512 + 1), 3);
    t.eq("depth of a million", l.depth(1_000_000), 3);
    t.eq("depth of a billion", l.depth(1_000_000_000), 4);
    t.eq("the pages of a tree", l.tree_pages(1_000_000), 1954 + 4 + 1);
}

fn spec(rows: u64, row_bytes: u64, dist: Dist) -> TableSpec {
    TableSpec { name: String::from("t"), key: String::from("id"), rows, row_bytes, dist, zipf_s: 1000, cols: alloc::vec![column("n", false, 0), column("c", false, 10), column("s", true, 100)], indexes: alloc::vec![String::from("c")] }
}

fn derived_rows(t: &mut T) {
    let tb = Table::new(spec(1000, 100, Dist::Uniform), 0, 5);
    let s = jfn_sim::dbstore::Snap { ts: 0, txn: 0 };
    // what is derived is the same each time, and in range
    t.eq("the same again", tb.derived(1, 7), Table::new(spec(1000, 100, Dist::Uniform), 0, 5).derived(1, 7));
    t.check("another seed, other values", (1..20).any(|k| tb.derived(1, k) != Table::new(spec(1000, 100, Dist::Uniform), 0, 6).derived(1, k)));
    t.check("distinct holds", (1..=1000).all(|k| matches!(tb.derived(2, k), Val::Int(v) if (0..10).contains(&v))));
    t.check("a text names its column", matches!(tb.derived(3, 1), Val::Text(x) if x.starts_with("s-")));
    t.check("no row 0 or 1001", !tb.exists(0, s) && !tb.exists(1001, s) && tb.exists(1000, s));
    // the closed forms against counting them one by one
    use jfn_sim::dbstore::RCond;
    let c = RCond::Int { col: 2, lo: 3, hi: 5 };
    let by_hand = (1..=1000).filter(|k| matches!(tb.derived(2, *k), Val::Int(v) if (3..=5).contains(&v))).count() as u64;
    t.eq("count in closed form", tb.count(1, 1000, core::slice::from_ref(&c), s), by_hand);
    let part = (101..=377).filter(|k| matches!(tb.derived(2, *k), Val::Int(v) if (3..=5).contains(&v))).count() as u64;
    t.eq("count of a key range", tb.count(101, 377, core::slice::from_ref(&c), s), part);
    let keys: Vec<u64> = (1..=1000).filter(|k| matches!(tb.derived(2, *k), Val::Int(v) if (3..=5).contains(&v))).take(20).collect();
    t.eq("the next ones by jumps", tb.by_key(1, 1000, core::slice::from_ref(&c), 20, false, s), keys);
    // every key of a distribution exists, once
    let mut seen: Vec<u64> = (1..=1000).map(|r| tb.key_of_rank(r)).collect();
    seen.sort_unstable();
    t.eq("ranks are a permutation", seen, (1..=1000).collect::<Vec<u64>>());
    let mut g = Pcg32::new(1, 1);
    t.check("draws are keys", (0..1000).all(|_| (1..=1000).contains(&tb.draw(&mut g))));
}

/// exp(x) for x <= 0, near enough for a comparison
fn exp_neg(x: f64) -> f64 {
    if x < -700.0 {
        return 0.0;
    }
    let (mut y, mut n) = (x, 0);
    while y < -0.01 {
        y /= 2.0;
        n += 1;
    }
    let mut r = 1.0 + y * (1.0 + y / 2.0 * (1.0 + y / 3.0 * (1.0 + y / 4.0)));
    for _ in 0..n {
        r *= r;
    }
    r
}

/// Che's approximation of an LRU of `c` pages for these page probabilities
fn che(p: &[f64], c: f64) -> f64 {
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    let filled = |t: f64| p.iter().map(|x| 1.0 - exp_neg(-x * t)).sum::<f64>();
    while filled(hi) < c {
        hi *= 2.0;
    }
    for _ in 0..50 {
        let m = (lo + hi) / 2.0;
        if filled(m) < c {
            lo = m;
        } else {
            hi = m;
        }
    }
    p.iter().map(|x| x * (1.0 - exp_neg(-x * hi))).sum()
}

/// the hit rate of a pool for `n` draws after as many to warm it
fn hit_rate(tb: &Table, l: &Layout, cap: u64, n: u64) -> (f64, u64) {
    let mut p = Pool::new(cap);
    let mut g = Pcg32::new(3, 9);
    let rpp = l.rows_per_page(tb.spec.row_bytes);
    let warm = (cap * 8).max(n);
    for _ in 0..warm {
        let k = tb.draw(&mut g);
        p.access((k - 1) / rpp, false);
    }
    let (h0, m0) = (p.hits, p.misses);
    for _ in 0..n {
        let k = tb.draw(&mut g);
        p.access((k - 1) / rpp, false);
    }
    ((p.hits - h0) as f64 / (p.hits - h0 + p.misses - m0) as f64, p.rate)
}

/// page probabilities of a table's zipf distribution, by its own mapping of ranks
fn zipf_pages(tb: &Table, l: &Layout) -> Vec<f64> {
    let rows = tb.spec.rows;
    let rpp = l.rows_per_page(tb.spec.row_bytes);
    let pages = l.heap_pages(rows, tb.spec.row_bytes) as usize;
    let mut p = alloc::vec![0.0; pages];
    let h: f64 = (1..=rows).map(|r| 1.0 / r as f64).sum();
    for r in 1..=rows {
        p[((tb.key_of_rank(r) - 1) / rpp) as usize] += 1.0 / r as f64 / h;
    }
    p
}

fn pool_unit(t: &mut T) {
    // an exact LRU in a few steps
    let mut p = Pool::new(2);
    t.check("first use misses", !p.access(1, false));
    t.check("the second too", !p.access(2, false));
    t.check("then it is there", p.access(1, false));
    t.check("a third pushes out the oldest", !p.access(3, false));
    t.check("the oldest is gone", !p.access(2, false));
    t.eq("it holds what fits", p.used(), 2);
    t.eq("exact", p.rate, 1);
    // many deletions and insertions keep the table right
    let mut p = Pool::new(1000);
    for i in 0..200_000u64 {
        p.access(i * 7 % 3001, false);
    }
    t.eq("still full", p.used(), 1000);
    t.check("still finds", (0..3001).filter(|x| p.clone().access(*x, false)).count() == 1000);

    let l = Layout::new(8192, 0);
    // uniform: pool / data
    let uni = Table::new(spec(1_000_000, 400, Dist::Uniform), 0, 1);
    for (cap, tol) in [(5000u64, 0.01), (25_000, 0.01)] {
        let (h, _) = hit_rate(&uni, &l, cap, 1_000_000);
        check_near(t, &format!("uniform, pool {} of 50000 pages, an exact or nearly exact LRU: pool / data", cap), h, cap as f64 / 50_000.0, tol);
    }
    let many = Table::new(spec(40_000_000, 400, Dist::Uniform), 0, 1);
    for (cap, tol) in [(200_000u64, 0.03), (1_000_000, 0.03)] {
        let (h, r) = hit_rate(&many, &l, cap, 1_500_000);
        t.check("it samples", r > 1);
        check_near(t, &format!("uniform, pool {} of 2000000 pages, 1 page in {} tracked: pool / data", cap, r), h, cap as f64 / 2_000_000.0, tol);
    }
    // zipf: Che's approximation
    let zt = Table::new(spec(200_000, 400, Dist::Zipf), 0, 1);
    let probs = zipf_pages(&zt, &l);
    for cap in [1000u64, 3000, 6000] {
        let (h, r) = hit_rate(&zt, &l, cap, 1_000_000);
        check_near(t, &format!("zipf 1.0, 200000 rows, pool {} of 10000 pages, 1 in {} tracked: Che", cap, r), h, che(&probs, cap as f64), 0.03);
    }
    let zb = Table::new(spec(2_000_000, 400, Dist::Zipf), 0, 1);
    let probs = zipf_pages(&zb, &l);
    for cap in [20_000u64, 50_000] {
        let (h, r) = hit_rate(&zb, &l, cap, 1_500_000);
        t.check("it samples zipf too", r > 1);
        check_near(t, &format!("zipf 1.0, 2000000 rows, pool {} of 100000 pages, 1 in {} tracked: Che", cap, r), h, che(&probs, cap as f64), 0.05);
    }
}

// ---------------------------------------------------------------- closed forms end to end

fn cpu_bound(t: &mut T) {
    let db_cfg = r#"{"tables": [{"name": "t", "rows": 1000, "row_bytes": 200}], "stmt_us": 1000, "pool_mb": 64}"#;
    // 2 levels of tree (1 upper, 1 leaf) and the heap: 3 pages, 1 row
    let cost = 1000.0 + 3.0 * 2.0 + 0.5;
    for (cores, speed) in [(1u32, 1000u32), (2, 1000), (4, 1500)] {
        let n = net(Some(hw(cores, speed, 8)), db_cfg, Some(closed(4 * cores + 8, DRAW_READ, 0)));
        let mut e = engine(&n.p);
        let got = rate(&mut e, &n, SEC, 4 * SEC);
        check_near(t, &format!("CPU-bound, {} cores at {} %: cores * speed / cost", cores, speed / 10), got, cores as f64 * speed as f64 / 1000.0 / cost * 1e6, 0.01);
        t.eq("no errors", metric(&e, n.srv, "db", "errors"), 0);
    }
}

fn io_bound(t: &mut T) {
    // a tree of one page, so a statement is that page and a heap page; no hardware: only the disk counts
    let table = r#"{"name": "t", "rows": 1000000, "row_bytes": 1000}"#;
    for (pool_mb, warm) in [(100u64, 8 * SEC), (400, 30 * SEC)] {
        let c = format!(r#"{{"tables": [{}], "fanout": 1000000, "pool_mb": {}, "disk": {{"capacity_mb": 100000, "iops": 2000, "mbps": 100000, "latency_us": 10}}}}"#, table, pool_mb);
        let n = net(None, &c, Some(closed(64, DRAW_READ, 0)));
        let mut e = engine(&n.p);
        let got = rate(&mut e, &n, warm, 6 * SEC);
        let hit = ((pool_mb << 20) / 8192 - 1) as f64 / 125_000.0;
        check_near(t, &format!("IO-bound, pool {} MiB: IOPS / (1 - hit), hit {:.3}", pool_mb, hit), got, 2000.0 / (1.0 - hit), 0.04);
    }
}

fn figure(body: &str, name: &str) -> f64 {
    number_of(body, name).unwrap_or(0) as f64
}

fn index_or_scan(t: &mut T) {
    let c = r#"{"tables": [{"name": "t", "rows": 100000, "row_bytes": 200, "columns": ["n"], "indexes": ["n"]}], "pool_mb": 128, "page_kb": 8}"#;
    let n = net(Some(hw(1, 1000, 8)), c, None);
    let steps = plan(&[
        (0, 0, "EXPLAIN SELECT * FROM t WHERE id = 5"),
        (10, 0, "SELECT * FROM t WHERE id = 5"),
        (20, 0, "EXPLAIN SELECT * FROM t WHERE n = 5"),
        (30, 0, "SELECT * FROM t WHERE n = 5"),
        (40, 0, "EXPLAIN SELECT COUNT(*) FROM t"),
        (50, 0, "SELECT COUNT(*) FROM t"),
        // now warm: the same again, without the disk
        (2000, 0, "SELECT * FROM t WHERE id = 5"),
        (2100, 0, "SELECT COUNT(*) FROM t"),
        (2200, 0, "SELECT * FROM t WHERE n = 5"),
        (2300, 0, "EXPLAIN SELECT * FROM t WHERE id BETWEEN 10 AND 19"),
        (2400, 0, "SELECT * FROM t WHERE id BETWEEN 10 AND 19"),
        (2500, 0, "EXPLAIN SELECT * FROM t LIMIT 7"),
        (2600, 0, "EXPLAIN SELECT * FROM t WHERE id = 100001"),
    ]);
    let (_, g) = driven(&n, steps, 3 * SEC);
    // depth 2 upper + leaf + heap: 100000 rows make 196 leaves, so one level above them
    t.check("EXPLAIN names the index", has(&g, 0, "\"plan\":\"index\"") && has(&g, 0, "\"index\":\"id\""));
    t.check("EXPLAIN says why", has(&g, 0, "\"why\":\"Gleichheit auf dem Schlüssel: der Baum führt zur Zeile.\""));
    t.check("EXPLAIN names the other index", has(&g, 2, "\"index\":\"n\""));
    t.check("EXPLAIN says scan", has(&g, 4, "\"plan\":\"scan\"") && has(&g, 4, "Keine Bedingung: alle Seiten der Tabelle."));
    let (ei, ri) = (body_of(&g, 0), body_of(&g, 1));
    t.eq("key: EXPLAIN's pages are what is charged", figure(&ei, "pages"), figure(&ri, "pages_hit") + figure(&ri, "pages_missed"));
    t.eq("key: EXPLAIN's rows too", figure(&ei, "rows_examined"), figure(&ri, "rows_examined"));
    let (es, rs) = (body_of(&g, 4), body_of(&g, 5));
    t.eq("scan: EXPLAIN's pages are what is charged", figure(&es, "pages"), figure(&rs, "pages_hit") + figure(&rs, "pages_missed"));
    t.eq("scan: EXPLAIN's rows too", figure(&es, "rows_examined"), 100_000.0);
    let (ex, rx) = (body_of(&g, 2), body_of(&g, 3));
    t.eq("index: EXPLAIN's pages are what is charged", figure(&ex, "pages"), figure(&rx, "pages_hit") + figure(&rx, "pages_missed"));
    t.check("the count", has(&g, 5, "\"count\":100000"));
    // the time of the warm ones: stmt + pages * page + rows * row, in microseconds
    let key_us = 30.0 + 3.0 * 2.0 + 0.5;
    let scan_us = 30.0 + 2500.0 * 2.0 + 100_000.0 * 0.5;
    check_near(t, "key lookup, warm: EXPLAIN's CPU", figure(&ei, "cpu_us"), key_us, 0.03);
    check_near(t, "scan: EXPLAIN's CPU", figure(&es, "cpu_us"), scan_us, 0.01);
    check_near(t, "key lookup, warm: the time it took", figure(&body_of(&g, 6), "us"), key_us, 0.1);
    check_near(t, "scan, warm: the time it took", figure(&body_of(&g, 7), "us"), scan_us, 0.01);
    check_near(t, "scan against index: the ratio from pages and rows", figure(&body_of(&g, 7), "us") / figure(&body_of(&g, 6), "us"), scan_us / key_us, 0.1);
    t.eq("a range of 10: EXPLAIN's pages", figure(&body_of(&g, 9), "pages"), figure(&body_of(&g, 10), "pages_hit") + figure(&body_of(&g, 10), "pages_missed"));
    t.eq("LIMIT stops a scan", figure(&body_of(&g, 11), "rows_examined"), 7.0);
    t.eq("a key that is not there has no heap page", figure(&body_of(&g, 12), "pages_heap"), 0.0);
    // the cold read costs the real disk: 3 tree pages and a heap page at QD1, or the scan
    let cold = figure(&ri, "pages_missed");
    t.check("cold: its pages came from the disk", cold >= 3.0);
    t.check("warm: from the pool", figure(&body_of(&g, 6), "pages_missed") == 0.0);
}

fn hit_end_to_end(t: &mut T) {
    // uniform over the whole way, with a fast disk: the heap's hits against pool / data
    let c = |mb: u64, dist: &str| format!(r#"{{"tables": [{{"name": "t", "rows": 200000, "row_bytes": 400, "dist": "{}"}}], "fanout": 1000000, "pool_mb": {}, "disk": {{"capacity_mb": 100000, "iops": 5000, "mbps": 100000, "latency_us": 20}}}}"#, dist, mb);
    let span = |e: &mut Engine, n: &Net| -> f64 {
        run_for(e, 3 * SEC);
        let (h0, m0) = db(e, n).map_or((0, 0), |d| d.heap_counts());
        run_for(e, 2 * SEC);
        let (h1, m1) = db(e, n).map_or((0, 0), |d| d.heap_counts());
        (h1 - h0) as f64 / (h1 - h0 + m1 - m0).max(1) as f64
    };
    let n = net(None, &c(40, "uniform"), Some(closed(8, DRAW_READ, 0)));
    let mut e = engine(&n.p);
    let h = span(&mut e, &n);
    check_near(t, "uniform end to end, 40 MiB of 80 MiB: pool / data", h, (5120.0 - 1.0) / 10_000.0, 0.03);
    let n = net(None, &c(16, "zipf"), Some(closed(8, DRAW_READ, 0)));
    let mut e = engine(&n.p);
    let h = span(&mut e, &n);
    let l = Layout::new(8192, 1_000_000);
    let probs = db(&e, &n).and_then(|d| d.table("t")).map_or(Vec::new(), |tb| zipf_pages(tb, &l));
    check_near(t, "zipf end to end, 16 MiB of 80 MiB: Che", h, che(&probs, 2047.0), 0.05);
    t.check("the pool's figure agrees", metric(&e, n.srv, "db", "pool_hit_pm") > 500);
    t.check("what it holds is the pool", metric(&e, n.srv, "db", "pool_bytes") == 16 << 20);
}

fn hot_row(t: &mut T) {
    // every statement waits for the row, holds it through its flush: 2 ms of latency and the transfer
    let c = r#"{"tables": [{"name": "t", "rows": 1000, "row_bytes": 200, "columns": ["n"]}], "disk": {"iops": 0, "mbps": 1000, "latency_us": 2000}, "stmt_us": 1}"#;
    let n = net(None, c, Some(closed(16, "UPDATE t SET n = n + 1 WHERE id = 1", 0)));
    let mut e = engine(&n.p);
    let got = rate(&mut e, &n, SEC, 4 * SEC);
    let hold = 2000.0 + (200.0 + 64.0 + 64.0) * 1e6 / 1e9;
    check_near(t, "a hot row without hardware: 1 / hold time", got, 1e6 / hold, 0.01);
    t.check("they waited", metric(&e, n.srv, "db", "lock_waits") > 1000);
    // the lock is the limit: it had waiters nearly all the time
    let lock = db(&e, &n).map_or(0, |d| d.lock_busy());
    t.check(&format!("the lock is contended: {}", lock), lock as f64 > e.now() as f64 * 0.7);
    // with hardware: the write's CPU comes on top, one row at a time
    let c = r#"{"tables": [{"name": "t", "rows": 1000, "row_bytes": 200, "columns": ["n"]}], "disk": {"iops": 0, "mbps": 1000, "latency_us": 2000}, "write_us": 500}"#;
    let n = net(Some(hw(8, 1000, 8)), c, Some(closed(16, "UPDATE t SET n = n + 1 WHERE id = 1", 0)));
    let mut e = engine(&n.p);
    let got = rate(&mut e, &n, SEC, 4 * SEC);
    check_near(t, "a hot row with 8 cores: 1 / (flush + write)", got, 1e6 / (hold + 500.0), 0.02);
}

fn group_commit(t: &mut T) {
    let c = r#"{"tables": [{"name": "t", "rows": 1000000, "row_bytes": 200, "columns": ["n"]}], "pool_mb": 512, "disk": {"capacity_mb": 100000, "iops": 0, "mbps": 1000, "latency_us": 5000}}"#;
    let body = r#"{"sql":"UPDATE t SET n = n + 1 WHERE id = ?","params":[{"draw":true}]}"#;
    let n = net(None, c, Some(closed(1, body, 0)));
    let mut e = engine(&n.p);
    let one = rate(&mut e, &n, 2 * SEC, 4 * SEC);
    // (each statement reads its tree and heap pages first: they miss while the pool is cold, a 5 ms latency each)
    let (hits, misses) = (metric(&e, n.srv, "db", "pool_hits"), metric(&e, n.srv, "db", "pool_misses"));
    t.check("a commit per flush", metric(&e, n.srv, "db", "wal_flushes") >= 40 && metric(&e, n.srv, "db", "group_pm") == 1000);
    let reads = misses as f64 / (metric(&e, n.srv, "db", "statements") as f64);
    let flush = 5000.0 + (200.0 + 64.0 + 64.0) * 1e6 / 1e9;
    let _ = hits;
    check_near(t, "one client: 1 / (flush + the reads it needs)", one, 1e6 / (flush + reads * 5000.0 + 150.0), 0.05);
    // without reads in the way: a small table, warm
    let c = r#"{"tables": [{"name": "t", "rows": 1000, "row_bytes": 200, "columns": ["n"]}], "pool_mb": 64, "disk": {"capacity_mb": 1000, "iops": 0, "mbps": 1000, "latency_us": 5000}}"#;
    let n = net(None, c, Some(closed(1, body, 0)));
    let mut e = engine(&n.p);
    let one = rate(&mut e, &n, 4 * SEC, 6 * SEC);
    check_near(t, "one client, warm: 1 / flush, the round trip is short", one, 1e6 / flush, 0.01);
    section("group_commit many");
    let n = net(None, c, Some(closed(16, body, 0)));
    let mut e = engine(&n.p);
    let many = rate(&mut e, &n, 4 * SEC, 6 * SEC);
    t.check(&format!("sixteen clients rise above it: {:.0} against {:.0}", many, one), many > one * 5.0);
    t.check(&format!("they share a flush: {} per mille", metric(&e, n.srv, "db", "group_pm")), metric(&e, n.srv, "db", "group_pm") > 5000);
    let flushes = 1e6 / flush;
    t.check("and a flush is a flush", (metric(&e, n.srv, "db", "wal_flushes") as f64) <= flushes * 11.0);
    // relaxed: nobody waits, and it is far faster
    section("group_commit relaxed");
    let c2 = r#"{"tables": [{"name": "t", "rows": 40, "row_bytes": 200, "columns": ["n"]}], "pool_mb": 64, "durability": "relaxed", "disk": {"capacity_mb": 1000, "iops": 0, "mbps": 1000, "latency_us": 5000}}"#;
    let n = net(None, c2, Some(closed(1, body, 0)));
    let mut e = engine(&n.p);
    let rel = rate(&mut e, &n, SEC / 10, SEC / 10);
    t.check(&format!("relaxed is faster: {:.0} against {:.0}", rel, one), rel > one * 10.0);
}

// ---------------------------------------------------------------- invariants

fn counter(t: &mut T) {
    for iso in ["read committed", "repeatable read"] {
        let c = format!(r#"{{"tables": [{{"name": "t", "rows": 10, "row_bytes": 200, "columns": ["n"]}}], "isolation": "{}", "lock_timeout_ms": 600000, "disk": {{"iops": 0, "mbps": 1000, "latency_us": 100}}}}"#, iso);
        let n = net(None, &c, Some(closed(8, "UPDATE t SET n = n + 1 WHERE id = 3", 400)));
        let mut e = engine(&n.p);
        let before = db(&e, &n).map(|d| d.row_now("t", 3));
        run_for(&mut e, 2 * SEC);
        let start = match before.flatten().map(|x| x[1].clone()) {
            Some(Val::Int(v)) => v,
            _ => -1,
        };
        let after = db(&e, &n).and_then(|d| d.row_now("t", 3)).map(|x| x[1].clone());
        // an implicit transaction takes its snapshot before the lock: repeatable read has conflicts to retry...
        if iso == "read committed" {
            t.eq("a counter loses no update (read committed)", after, Some(Val::Int(start + 400)));
            t.eq("no errors", metric(&e, n.srv, "db", "errors"), 0);
        } else {
            let done = metric(&e, n.srv, "db", "commits");
            t.eq("repeatable read: as many as committed, no more and no less", after, Some(Val::Int(start + done)));
        }
    }
}

fn transfers(t: &mut T) {
    let c = r#"{"tables": [{"name": "a", "rows": 8, "row_bytes": 100, "columns": ["b"]}], "lock_timeout_ms": 2000, "txn_idle_ms": 500, "disk": {"iops": 0, "mbps": 1000, "latency_us": 200}}"#;
    let n = net(None, c, None);
    let mut e = engine(&n.p);
    let until = 2 * SEC;
    let _ = e.add_service(n.gen_, "bank", Box::new(Bank::new(6, until)), true);
    let reads: Vec<Step> = (0..20).map(|i| Step { at: (50 + i * 100) * MS, ses: 99, sql: String::from("SELECT * FROM a WHERE id BETWEEN 1 AND 8"), get: None }).collect();
    let _ = e.add_service(n.gen_, "drv", Box::new(Driver::new(DB_IP, reads)), true);
    run_for(&mut e, SEC / 100);
    let sum_of = |e: &Engine| -> i64 { (1..=8).map(|k| db(e, &n).and_then(|d| d.row_now("a", k)).map_or(0, |x| if let Val::Int(v) = x[1] { v } else { 0 })).sum() };
    let start = sum_of(&e);
    run_for(&mut e, 5 * SEC);
    t.eq("the sum stays", sum_of(&e), start);
    let (done, conflicts) = e.service::<Bank>(n.gen_, "bank").map_or((0, 0), |b| (b.done, b.conflicts));
    t.check(&format!("{} transfers went through", done), done > 100);
    t.check(&format!("{} met a conflict", conflicts), conflicts > 0);
    t.check("deadlocks happened", metric(&e, n.srv, "db", "deadlocks") > 0);
    // every statement sees one moment: the sum of its rows is the same
    let got = got_of(&e, n.gen_, 20);
    let mut all = true;
    for x in got.iter().flatten() {
        let v = agent_core::json::parse(x.body.as_bytes()).unwrap_or(Value::Null);
        let sum: i64 = v.get("rows").and_then(|r| r.as_array()).unwrap_or(&[]).iter().map(|r| r.get("b").and_then(|b| b.as_i64()).unwrap_or(0)).sum();
        all &= sum == start;
    }
    t.check("a reader sees the sum at every moment", all && got.iter().flatten().count() == 20);
    t.eq("no row stays locked", db(&e, &n).map(|d| d.lock_list().len()), Some(0));
}

const SMALL: &str = r#"{"tables": [{"name": "t", "rows": 100, "row_bytes": 100, "columns": ["n"]}], "lock_timeout_ms": 300, "txn_idle_ms": 500, "disk": {"iops": 0, "mbps": 1000, "latency_us": 100}}"#;

fn n_of(body: &str) -> i64 {
    let v = agent_core::json::parse(body.as_bytes()).unwrap_or(Value::Null);
    v.get("rows").and_then(|r| r.as_array()).and_then(|r| r.first()).and_then(|r| r.get("n")).and_then(|n| n.as_i64()).unwrap_or(i64::MIN)
}

fn isolation(t: &mut T) {
    let n = net(None, SMALL, None);
    let story = |begin: &'static str| {
        [(0u64, 0usize, begin), (10, 0, "SELECT n FROM t WHERE id = 1"), (20, 1, "UPDATE t SET n = n + 100 WHERE id = 1"), (30, 0, "SELECT n FROM t WHERE id = 1"), (40, 0, "UPDATE t SET n = n + 1 WHERE id = 1"), (50, 0, "COMMIT"), (60, 2, "SELECT n FROM t WHERE id = 1")]
    };
    let (_, g) = driven(&n, plan(&story("BEGIN ISOLATION LEVEL READ COMMITTED")), SEC);
    let v0 = n_of(&body_of(&g, 1));
    t.eq("read committed: sees what another committed", n_of(&body_of(&g, 3)), v0 + 100);
    t.eq("read committed: writes on the newest", status_of(&g, 4), 200);
    t.eq("read committed: nothing lost", n_of(&body_of(&g, 6)), v0 + 101);
    let (_, g) = driven(&n, plan(&story("BEGIN ISOLATION LEVEL REPEATABLE READ")), SEC);
    t.eq("repeatable read: the same value again", n_of(&body_of(&g, 3)), v0);
    t.eq("repeatable read: it may not write over it", status_of(&g, 4), 409);
    t.check("it says serialization, retryable", has(&g, 4, "\"code\":\"serialization\"") && has(&g, 4, "\"retryable\":true"));
    t.check("its transaction is gone", status_of(&g, 5) == 400 && has(&g, 5, "COMMIT ohne Transaktion"));
    t.eq("the other one stands", n_of(&body_of(&g, 6)), v0 + 100);
    // readers do not wait for writers
    let (_, g) = driven(&n, plan(&[(0, 0, "BEGIN"), (10, 0, "UPDATE t SET n = 7 WHERE id = 2"), (20, 1, "SELECT n FROM t WHERE id = 2"), (30, 0, "SELECT n FROM t WHERE id = 2"), (40, 0, "ROLLBACK"), (50, 1, "SELECT n FROM t WHERE id = 2")]), SEC);
    t.check("a reader goes past the lock, at once", g[2].as_ref().is_some_and(|x| x.at < 25 * MS));
    t.eq("and sees the old value", n_of(&body_of(&g, 2)) == 7, false);
    t.eq("its own transaction sees its value", n_of(&body_of(&g, 3)), 7);
    t.eq("after a rollback the old value", n_of(&body_of(&g, 5)), n_of(&body_of(&g, 2)));
}

fn deadlock(t: &mut T) {
    let n = net(None, SMALL, None);
    let (e, g) = driven(&n, plan(&[(0, 0, "BEGIN"), (1, 1, "BEGIN"), (10, 0, "UPDATE t SET n = 1 WHERE id = 1"), (20, 1, "UPDATE t SET n = 2 WHERE id = 2"), (30, 0, "UPDATE t SET n = 3 WHERE id = 2"), (40, 1, "UPDATE t SET n = 4 WHERE id = 1"), (60, 0, "COMMIT"), (70, 2, "SELECT * FROM t WHERE id BETWEEN 1 AND 2")]), SEC);
    t.eq("the one that closes the circle goes", status_of(&g, 5), 409);
    t.check("it says deadlock, retryable", has(&g, 5, "\"code\":\"deadlock\"") && has(&g, 5, "\"retryable\":true"));
    t.eq("the other goes on", status_of(&g, 4), 200);
    t.eq("and commits", status_of(&g, 6), 200);
    t.eq("counted", metric(&e, n.srv, "db", "deadlocks"), 1);
    let v = agent_core::json::parse(body_of(&g, 7).as_bytes()).unwrap_or(Value::Null);
    let rows = v.get("rows").and_then(|r| r.as_array()).map(|a| a.to_vec()).unwrap_or_default();
    t.eq("what the survivor wrote stands", rows.iter().map(|r| r.get("n").and_then(|x| x.as_i64())).collect::<Vec<_>>(), alloc::vec![Some(1), Some(3)]);
}

fn timeouts(t: &mut T) {
    let n = net(None, SMALL, None);
    // a lock that does not come
    let (e, g) = driven(&n, plan(&[(0, 0, "BEGIN"), (10, 0, "SELECT * FROM t WHERE id = 1 FOR UPDATE"), (20, 1, "UPDATE t SET n = 5 WHERE id = 1"), (400, 0, "COMMIT")]), SEC);
    t.eq("the wait ends", status_of(&g, 2), 409);
    t.check("it says lock_timeout", has(&g, 2, "\"code\":\"lock_timeout\""));
    t.check("after its time", g[2].as_ref().is_some_and(|x| (320 * MS..340 * MS).contains(&x.at)));
    t.eq("counted", metric(&e, n.srv, "db", "lock_timeouts"), 1);
    // a client that goes silent while it holds a lock
    let (e, g) = driven(&n, plan(&[(0, 0, "BEGIN"), (10, 0, "UPDATE t SET n = 5 WHERE id = 1"), (20, 1, "BEGIN"), (800, 0, "COMMIT"), (810, 2, "SELECT n FROM t WHERE id = 1"), (820, 2, "UPDATE t SET n = 9 WHERE id = 1")]), 2 * SEC);
    t.eq("its transaction went", status_of(&g, 3), 404);
    t.eq("the words", body_of(&g, 3).contains("Die Transaktion 1 gibt es nicht (mehr)."), true);
    t.check("what it wrote is gone", n_of(&body_of(&g, 4)) != 5);
    t.eq("the lock is free", status_of(&g, 5), 200);
    t.eq("counted", metric(&e, n.srv, "db", "idle_aborts"), 2);
}

fn versions(t: &mut T) {
    let n = net(None, SMALL, None);
    let mut steps = plan(&[(0, 0, "BEGIN ISOLATION LEVEL REPEATABLE READ"), (10, 0, "SELECT n FROM t WHERE id = 1")]);
    for i in 0..5 {
        steps.push(Step { at: (20 + i) * MS, ses: 1, sql: String::from("UPDATE t SET n = n + 1 WHERE id = 1"), get: None });
    }
    steps.push(Step { at: 100 * MS, ses: 0, sql: String::from("SELECT n FROM t WHERE id = 1"), get: None });
    steps.push(Step { at: 200 * MS, ses: 0, sql: String::from("COMMIT"), get: None });
    let mut e = engine(&n.p);
    let _ = e.add_service(n.gen_, "drv", Box::new(Driver::new(DB_IP, steps.clone())), true);
    run_for(&mut e, 150 * MS);
    t.check(&format!("a snapshot keeps them: {}", metric(&e, n.srv, "db", "old_versions")), metric(&e, n.srv, "db", "old_versions") >= 4);
    run_for(&mut e, 150 * MS);
    t.eq("when it ends they go", metric(&e, n.srv, "db", "old_versions"), 0);
    let g = got_of(&e, n.gen_, steps.len());
    t.eq("it saw the same all along", n_of(&body_of(&g, 7)), n_of(&body_of(&g, 1)));
}

fn limits(t: &mut T) {
    let c = r#"{"tables": [{"name": "t", "rows": 100, "row_bytes": 100}], "max_conns": 2, "write_rows_max": 5, "max_rows": 3}"#;
    let n = net(None, c, None);
    // three requests at once from one client make three connections
    let same: Vec<Step> = (0..3).map(|_| Step { at: 0, ses: 0, sql: String::from("SELECT * FROM t WHERE id = 1"), get: None }).collect();
    let mut steps = same;
    steps.extend(plan(&[(50, 1, "UPDATE t SET id = 1"), (60, 1, "DELETE FROM t"), (70, 1, "SELECT * FROM t"), (80, 1, "SELECT * FROM t LIMIT 2")]));
    let (e, g) = driven(&n, steps, SEC);
    let refused = (0..3).filter(|i| status_of(&g, *i) == 503).count();
    t.eq("the third connection is refused", refused, 1);
    t.check("with clear words", (0..3).any(|i| has(&g, i, "Zu viele Verbindungen: höchstens 2.") && has(&g, i, "\"code\":\"too_many_connections\"")));
    t.eq("counted", metric(&e, n.srv, "db", "refused"), 1);
    t.check("no change of a key", has(&g, 3, "Den Schlüssel einer Zeile ändert hier nichts."));
    t.check("a change too large", has(&g, 4, "Diese Anweisung beträfe 100 Zeilen, höchstens 5 gehen auf einmal.") && status_of(&g, 4) == 400);
    t.check("an answer too long says so", has(&g, 5, "\"truncated\":true") && has(&g, 5, "\"count\":3"));
    t.check("a LIMIT is no truncation", !has(&g, 6, "truncated") && has(&g, 6, "\"count\":2"));
}

fn statements(t: &mut T) {
    let c = r#"{"tables": [{"name": "u", "rows": 100, "row_bytes": 100, "key": "uid", "columns": ["mail:text", "age:int:50"], "indexes": ["mail", "age"]}, {"name": "o", "rows": 300, "row_bytes": 100, "key": "oid", "columns": ["uid:int:100", "sum"], "indexes": ["uid"]}]}"#;
    let n = net(None, c, None);
    let (e, g) = driven(&n, plan(&[
        (0, 0, "SELECT * FROM nix"),
        (1, 0, "SELECT nix FROM u"),
        (2, 0, "SELECT * FROM u WHERE mail > 'x'"),
        (3, 0, "SELECT * FROM u WHERE age = 'x'"),
        (4, 0, "SELECT * FROM u WHERE mail = 3"),
        (5, 0, "SELECT * FROM u ORDER BY uid DESC LIMIT 2"),
        (6, 0, "SELECT * FROM u ORDER BY age LIMIT 3"),
        (7, 0, "SELECT * FROM o ORDER BY sum"),
        (8, 0, "INSERT INTO u (mail, age) VALUES ('neu', 7)"),
        (9, 0, "SELECT * FROM u WHERE mail = 'neu'"),
        (10, 0, "INSERT INTO u (uid, mail) VALUES (3, 'x')"),
        (11, 0, "DELETE FROM u WHERE uid = 5"),
        (12, 0, "SELECT * FROM u WHERE uid = 5"),
        (13, 0, "SELECT COUNT(*) FROM u"),
        (14, 0, "UPDATE u SET age = age + 1 WHERE uid = 9"),
        (15, 0, "COMMIT"),
        (16, 0, "SELECT u.uid, o.oid FROM u JOIN o ON u.uid = o.uid WHERE u.uid BETWEEN 1 AND 2 LIMIT 3"),
        (17, 0, "SELECT * FROM u JOIN o ON u.age = o.sum"),
        (18, 0, "UPDATE u SET mail = 'anna' WHERE uid = 2"),
        (19, 0, "SELECT uid FROM u WHERE mail = 'anna'"),
        (20, 0, "SELECT COUNT(*) FROM u WHERE age BETWEEN 0 AND 49"),
        (21, 0, "SELECT * FROM o WHERE sum BETWEEN 3 AND 5 LIMIT 2"),
        (22, 0, "SELECT * FROM u WHERE uid = 1 AND age = 9999"),
        (23, 0, "SELECT * FROM u WHERE uid < 1"),
    ]), SEC);
    t.check("no such table, with its place", has(&g, 0, "Zeichen 15: Die Tabelle \u{201e}nix\u{201c} gibt es nicht.") && has(&g, 0, "\"pos\":15") && status_of(&g, 0) == 400);
    t.check("no such column, with its place", has(&g, 1, "Zeichen 8: Die Spalte \u{201e}nix\u{201c} gibt es in \u{201e}u\u{201c} nicht."));
    t.check("text knows =", has(&g, 2, "Die Spalte \u{201e}mail\u{201c} hält Text: sie kennt nur =."));
    t.check("numbers compare with numbers", has(&g, 3, "hält Zahlen: sie vergleicht sich mit einer Zahl."));
    t.check("text compares with text", has(&g, 4, "hält Text: sie vergleicht sich mit 'Text'."));
    let ids = |b: String, name: &str| -> Vec<i64> { agent_core::json::parse(b.as_bytes()).ok().and_then(|v| v.get("rows").and_then(|r| r.as_array().map(|a| a.iter().filter_map(|x| x.get(name).and_then(|y| y.as_i64())).collect()))).unwrap_or_default() };
    t.eq("ORDER BY the key, descending", ids(body_of(&g, 5), "uid"), alloc::vec![100, 99]);
    let ages = ids(body_of(&g, 6), "age");
    t.check(&format!("ORDER BY an index: {:?}", ages), ages.len() == 3 && ages[0] <= ages[1] && ages[1] <= ages[2]);
    t.check("ORDER BY needs an index", has(&g, 7, "ORDER BY geht nur über einen Index: auf \u{201e}sum\u{201c} liegt keiner."));
    t.check("an insert", status_of(&g, 8) == 200 && has(&g, 8, "\"changed\":1"));
    t.eq("found through its index", ids(body_of(&g, 9), "uid"), alloc::vec![101]);
    t.check("a key that is taken", status_of(&g, 10) == 409 && has(&g, 10, "\"code\":\"duplicate_key\"") && has(&g, 10, "Den Schlüssel 3 gibt es schon."));
    t.eq("a deleted row is gone", ids(body_of(&g, 12), "uid"), Vec::<i64>::new());
    t.check("the count follows", has(&g, 13, "\"count\":100"));
    t.check("a COMMIT outside", status_of(&g, 15) == 400 && has(&g, 15, "COMMIT ohne Transaktion: es fehlt \u{201e}txn\u{201c}."));
    t.check(&format!("a join: {}", body_of(&g, 16)), has(&g, 16, "\"count\":3") && has(&g, 16, "\"u.uid\":") && has(&g, 16, "\"o.oid\":"));
    t.check("a join needs an index", has(&g, 17, "Ein JOIN geht nur über einen Index: auf \u{201e}sum\u{201c} liegt keiner."));
    t.eq("a moved value is found", ids(body_of(&g, 19), "uid"), alloc::vec![2]);
    t.check("a count over an index", has(&g, 20, "\"count\":100"));
    t.check("a scan with a condition jumps and stops", has(&g, 21, "\"count\":2"));
    t.check("no such row", has(&g, 22, "\"count\":0") && has(&g, 23, "\"count\":0"));
    t.eq("updates counted", metric(&e, n.srv, "db", "updates"), 2);
}

fn live_index(t: &mut T) {
    let c = r#"{"tables": [{"name": "t", "rows": 200000, "row_bytes": 200, "columns": ["n"]}], "pool_mb": 512}"#;
    let n = net(Some(hw(4, 1000, 8)), c, None);
    let (mut e, g) = driven(&n, plan(&[(0, 0, "EXPLAIN SELECT * FROM t WHERE n = 5"), (10, 0, "CREATE INDEX ON t (n)"), (20, 0, "EXPLAIN SELECT * FROM t WHERE n = 5"), (21, 0, "CREATE INDEX ON t (n)"), (22, 0, "CREATE INDEX ON t (id)")]), 25 * MS);
    t.check("first it scans", has(&g, 0, "\"plan\":\"scan\""));
    t.check("still building: it scans", has(&g, 2, "\"plan\":\"scan\""));
    t.check("only one at a time", has(&g, 3, "Der Index auf \u{201e}n\u{201c} wird gerade gebaut."));
    t.check("the key has its index", has(&g, 4, "Auf \u{201e}id\u{201c} liegt schon ein Index."));
    run_for(&mut e, 60 * SEC);
    let g = got_of(&e, n.gen_, 5);
    let took = g[1].as_ref().map_or(0, |x| x.at) - 10 * MS;
    // rows * row * log2(rows), the cores take it one second's piece at a time
    let want = 200_000.0 * 0.5e-6 * 18.0;
    check_near(t, "building takes the time of sorting: rows * row * log2(rows)", took as f64 / SEC as f64, want, 0.1);
    t.check("built", status_of(&g, 1) == 200 && has(&g, 1, "\"created\":true"));
    let (ex, sel, drop, after) = (Step { at: 0, ses: 0, sql: String::from("EXPLAIN SELECT * FROM t WHERE n = 5"), get: None }, Step { at: 0, ses: 0, sql: String::from("DROP INDEX ON t (n)"), get: None }, Step { at: 0, ses: 0, sql: String::from("DROP INDEX ON t (n)"), get: None }, Step { at: 0, ses: 0, sql: String::from("EXPLAIN SELECT * FROM t WHERE n = 5"), get: None });
    let at = e.now();
    let list: Vec<Step> = [ex, sel, drop, after].into_iter().enumerate().map(|(i, mut s)| {
        s.at = at + (i as u64 + 1) * 10 * MS;
        s
    }).collect();
    let _ = e.add_service(n.gen_, "drv2", Box::new(Driver::new(DB_IP, list)), true);
    run_for(&mut e, SEC);
    let d2 = e.service::<Driver>(n.gen_, "drv2");
    let b = |i: usize| d2.and_then(|d| d.got.get(&i)).map_or(String::new(), |x| x.body.clone());
    t.check("now the index", b(0).contains("\"plan\":\"index\""));
    t.check("dropped", b(1).contains("\"dropped\":true"));
    t.check("twice: refused", b(2).contains("Auf \u{201e}n\u{201c} liegt kein Index, der sich löschen ließe."));
    t.check("then it scans again", b(3).contains("\"plan\":\"scan\""));
}

// ---------------------------------------------------------------- crash and recovery

const CRASH: &str = r#"{"tables": [{"name": "t", "rows": 1000, "row_bytes": 200, "columns": ["n"]}], "lock_timeout_ms": 60000, "redo_mbps": 1, "checkpoint_s": 1000, "disk": {"iops": 0, "mbps": 100, "latency_us": 500}}"#;

fn base_n(e: &Engine, n: &Net, key: u64) -> i64 {
    match db(e, n).and_then(|d| d.row_now("t", key)).map(|x| x[1].clone()) {
        Some(Val::Int(v)) => v,
        _ => i64::MIN,
    }
}

fn crash_full(t: &mut T) {
    let mut load = closed(8, "UPDATE t SET n = n + 1 WHERE id = 1", 0);
    load.insert("think_ms", Value::u64(5));
    let n = net(None, CRASH, Some(load));
    let mut e = engine(&n.p);
    let v0 = base_n(&e, &n, 1);
    let v5 = base_n(&e, &n, 5);
    let open = plan(&[(0, 0, "BEGIN"), (10, 0, "UPDATE t SET n = 12345 WHERE id = 5"), (11, 1, "BEGIN"), (12, 1, "INSERT INTO t (id, n) VALUES (5000, 1)")]);
    let _ = e.add_service(n.gen_, "drv", Box::new(Driver::new(DB_IP, open)), true);
    run_for(&mut e, 2 * SEC);
    let ok_before = e.metric(n.gen_, "load", "ok").unwrap_or(0);
    t.check("it ran", ok_before > 100);
    t.check("the open one sees its row, nobody else", base_n(&e, &n, 5) == v5);
    let _ = e.apply(Action::Power { dev: n.srv, on: false });
    let ok = e.metric(n.gen_, "load", "ok").unwrap_or(0);
    let commits = metric(&e, n.srv, "db", "commits");
    let lsn_off = db(&e, &n).map_or(0, |d| d.flushed_lsn());
    t.eq("every commit answered is there", base_n(&e, &n, 1) - v0 >= ok, true);
    t.eq("and exactly those that were final", base_n(&e, &n, 1) - v0, commits);
    t.eq("no commit lost", metric(&e, n.srv, "db", "lost_commits"), 0);
    t.eq("the open transaction is gone", base_n(&e, &n, 5), v5);
    t.eq("its new row too", db(&e, &n).and_then(|d| d.row_now("t", 5000)), None);
    t.eq("no locks left", db(&e, &n).map(|d| d.lock_list().len()), Some(0));
    // the way back takes the time of the log
    let _ = e.apply(Action::Power { dev: n.srv, on: true });
    run_for(&mut e, 10 * MS);
    t.check("first it reads its log", db(&e, &n).is_some_and(|d| d.view(e.now()).recovering));
    let since = lsn_off as f64;
    let want = since / 100e6 * 1000.0 + 0.5 + since / 1e6 * 1000.0;
    // (the disk may have been busy with the flush that the crash cut)
    run_for(&mut e, ((want as u64) + 200) * MS);
    let v = db(&e, &n).map(|d| d.view(e.now()));
    t.check("then it runs", v.as_ref().is_some_and(|x| !x.recovering));
    t.eq("once", v.as_ref().map(|x| x.stats.recoveries), Some(1));
    check_near(t, "recovery: log / disk + latency + log / redo rate (ms)", v.as_ref().map_or(0, |x| x.stats.recovery_ms) as f64, want, 0.05);
    t.check("and the load goes on", e.metric(n.gen_, "load", "ok").unwrap_or(0) >= ok);
}

fn crash_relaxed(t: &mut T) {
    let c = r#"{"tables": [{"name": "t", "rows": 1000, "row_bytes": 200, "columns": ["n"]}], "durability": "relaxed", "flush_ms": 200, "lock_timeout_ms": 60000, "stmt_us": 100, "write_us": 100, "disk": {"iops": 0, "mbps": 100, "latency_us": 500}}"#;
    let n = net(Some(hw(1, 1000, 8)), c, Some(closed(4, "UPDATE t SET n = n + 1 WHERE id = 1", 0)));
    let mut e = engine(&n.p);
    let v0 = base_n(&e, &n, 1);
    run_for(&mut e, 1050 * MS);
    let _ = e.apply(Action::Power { dev: n.srv, on: false });
    let commits = metric(&e, n.srv, "db", "commits");
    let lost = metric(&e, n.srv, "db", "lost_commits");
    let kept = base_n(&e, &n, 1) - v0;
    t.check(&format!("{} commits, {} of them lost", commits, lost), commits > 500 && lost > 0);
    t.eq("the lost ones are counted and match", kept + lost, commits);
    let at_most = commits as f64 / 1.05 * 0.21;
    t.check(&format!("it lost at most flush_ms of them: {} of {:.0}", lost, at_most), (lost as f64) <= at_most);
}

fn cold_start(t: &mut T) {
    let c = r#"{"tables": [{"name": "t", "rows": 200000, "row_bytes": 400}], "fanout": 1000000, "pool_mb": 64, "disk": {"capacity_mb": 100000, "iops": 2000, "mbps": 10000, "latency_us": 10}}"#;
    let n = net(None, c, Some(closed(32, DRAW_READ, 0)));
    let mut e = engine(&n.p);
    let first = rate(&mut e, &n, 0, SEC);
    let warm = rate(&mut e, &n, 8 * SEC, SEC);
    t.check(&format!("it warms up: {:.0}, then {:.0}", first, warm), warm > first * 2.0);
    let _ = e.apply(Action::Fault { fault: jfn_sim::Fault::Stop { dev: n.srv, svc: String::from("db") }, dur: None });
    run_for(&mut e, 100 * MS);
    t.eq("a stop empties it", metric(&e, n.srv, "db", "pool_bytes") < 1 << 20, true);
}

fn oom(t: &mut T) {
    // a pool of 2 GiB on 256 MiB of RAM and 64 of swap: as it fills, the machine swaps, then the kill comes
    let h = Hw { cores: 2, speed_pm: 1000, ram_mb: 256, swap_mb: 64, disk: DiskClass::Nvme, disk_gb: 200 };
    let c = r#"{"tables": [{"name": "t", "rows": 10000000, "row_bytes": 800}], "pool_mb": 2048, "mem_mb": 32}"#;
    let n = net(Some(h), c, Some(closed(64, DRAW_READ, 0)));
    let mut e = engine(&n.p);
    run_for(&mut e, 500 * MS);
    let early = metric(&e, n.srv, "db", "pool_bytes");
    t.check(&format!("it counts as it fills: {} bytes", early), early > 0);
    t.eq("no kill yet", metric(&e, n.srv, HOST, "oom_kills"), 0);
    for _ in 0..600 {
        if metric(&e, n.srv, HOST, "oom_kills") > 0 {
            break;
        }
        run_for(&mut e, SEC / 10);
    }
    t.eq("killed", metric(&e, n.srv, HOST, "oom_kills"), 1);
    t.check("the log says 137", e.log_lines(n.srv).iter().any(|l| l.text.contains("Speicher voll") && l.text.contains("137")));
    t.check("it does not run", !e.services(n.srv).iter().find(|s| s.name == "db").is_some_and(|s| s.running));
    // the same pool with enough RAM runs on
    let n2 = net(Some(hw(2, 1000, 8)), c, Some(closed(64, DRAW_READ, 0)));
    let mut e = engine(&n2.p);
    run_for(&mut e, 2 * SEC);
    t.eq("with enough RAM: no kill", metric(&e, n2.srv, HOST, "oom_kills"), 0);
}

fn bound(t: &mut T) {
    let c = r#"{"tables": [{"name": "t", "rows": 100000, "row_bytes": 100, "columns": ["n"]}], "touched_max": 64, "durability": "relaxed", "flush_ms": 1, "disk": {"capacity_mb": 1000, "iops": 0, "mbps": 1000, "latency_us": 1}}"#;
    let body = r#"{"sql":"UPDATE t SET n = 1 WHERE id = ?","params":[{"draw":true}]}"#;
    let n = net(None, c, Some(closed(4, body, 2000)));
    let mut e = engine(&n.p);
    run_for(&mut e, 5 * SEC);
    t.eq("all of them ran", metric(&e, n.srv, "db", "commits"), 2000);
    t.check(&format!("the bound holds: {}", metric(&e, n.srv, "db", "touched_rows")), metric(&e, n.srv, "db", "touched_rows") <= 64);
    t.check(&format!("and it says so: {}", metric(&e, n.srv, "db", "forgotten")), metric(&e, n.srv, "db", "forgotten") > 1000);
    t.check("the status too", db(&e, &n).is_some_and(|d| d.status().contains("vergessen") || d.status().contains("Ohne Hardware")));
}

// ---------------------------------------------------------------- determinism

fn determinism(t: &mut T) {
    let c = r#"{"tables": [{"name": "t", "rows": 5000, "row_bytes": 200, "columns": ["n"], "dist": "zipf"}], "lock_timeout_ms": 1000, "pool_mb": 8, "disk": {"capacity_mb": 1000, "iops": 0, "mbps": 1000, "latency_us": 200}}"#;
    let body = r#"{"sql":"UPDATE t SET n = n + 1 WHERE id = ?","params":[{"draw":true}]}"#;
    let n = net(Some(hw(2, 1000, 8)), c, Some(closed(16, body, 0)));
    let hold = plan(&[(100, 0, "BEGIN"), (110, 0, "UPDATE t SET n = 1 WHERE id = 1"), (1500, 0, "COMMIT")]);
    let start = || {
        let mut e = engine(&n.p);
        let _ = e.add_service(n.gen_, "drv", Box::new(Driver::new(DB_IP, hold.clone())), true);
        e
    };
    let (mut a, mut b) = (start(), start());
    run_for(&mut a, 2 * SEC);
    run_for(&mut b, 2 * SEC);
    t.eq("two runs: equal", digest(&a), digest(&b));
    t.eq("and their numbers", a.metric(n.srv, "db", "commits"), b.metric(n.srv, "db", "commits"));
    // a clone with locks held goes on the same
    let mut c1 = start();
    run_for(&mut c1, SEC);
    t.check("locks are held now", db(&c1, &n).is_some_and(|d| !d.lock_list().is_empty()));
    let mut c2 = c1.clone();
    run_for(&mut c1, SEC);
    run_for(&mut c2, SEC);
    t.eq("a clone of a warm engine with locks: equal", digest(&c1), digest(&c2));
    t.eq("and its rows", db(&c1, &n).and_then(|d| d.row_now("t", 1)), db(&c2, &n).and_then(|d| d.row_now("t", 1)));
    t.eq("and it equals the plain run", digest(&c1), digest(&a));
    // a keyframe taken with locks held replays to the same result
    let mut drv = Inline::new(start());
    drv.rewind_on(Budget { gap_events: 2000, ..Budget::default() });
    for _ in 0..20 {
        let _ = drv.send(Cmd::RunFor(100 * MS), 0);
    }
    let _ = drv.reports();
    let live = digest(&drv.eng);
    let pos = drv.pos();
    let held = drv.rewind().map_or(0, |r| r.keyframes().iter().filter(|k| k.0.t >= 200 * MS && k.0.t <= 1400 * MS).count());
    t.check(&format!("keyframes with locks held: {}", held), held > 0);
    let past = drv.rewind().and_then(|rw| {
        let mut r = drv.restore_pos(pos, None)?;
        if r.run(rw, usize::MAX) { Some(r.finish()) } else { None }
    });
    t.eq("the replay equals the run", past.map(|p| digest(&p.eng)), Some(live));
    // what it adds to a keyframe is told truly: the rows are real
    let before = db(&a, &n).map_or(0, |d| d.footprint());
    t.check(&format!("its footprint counts what it holds: {}", before), before > 16 << 10);
}

/// the sweep of the study with few runs on one and on three threads
fn sweep_threads(t: &mut T) {
    let Ok(exp) = Experiment::from_text(include_str!("../tests/experiments/db_scale_small.jfnex")) else {
        t.check("the small study reads", false);
        return;
    };
    let Ok(p) = Project::from_text(include_str!("../tests/experiments/db_scale.jfnet")) else {
        t.check("its project reads", false);
        return;
    };
    let plan = match Plan::new(&exp, &p, "db_scale_small") {
        Ok(x) => x,
        Err(m) => {
            t.check(&format!("its plan: {:?}", m), false);
            return;
        }
    };
    let one = plan.run_all().table();
    let three = run_sweep(&plan, 3).table();
    t.eq("1 and 3 threads give the same bytes", one, three);
    // a study can say what limits it: a hot row is a lock
    let c = r#"{"tables": [{"name": "t", "rows": 1000, "row_bytes": 200, "columns": ["n"]}], "disk": {"iops": 0, "mbps": 1000, "latency_us": 2000}}"#;
    let n = net(Some(hw(8, 1000, 8)), c, Some(closed(16, "UPDATE t SET n = n + 1 WHERE id = 1", 0)));
    let (g, d) = (n.p.device(n.gen_).map_or(String::new(), |x| x.name.clone()), n.p.device(n.srv).map_or(String::new(), |x| x.name.clone()));
    let text = format!(r#"{{"format": "jfnex", "version": 1, "name": "hot", "project": "", "load": {{"dev": "{}", "svc": "load", "users": [16]}}, "timing": {{"warmup_s": 1, "settle_s": 1, "measure_s": 2, "seeds": [1]}}, "series": [{{"name": "a", "axes": [{{"kind": "config", "name": "Verbindungen", "dev": "{}", "path": "db.max_conns", "values": [100]}}]}}]}}"#, g, d);
    match Experiment::from_text(&text).map(|x| Plan::new(&x, &n.p, "hot")) {
        Ok(Ok(pl)) => {
            let s = pl.run_all();
            let word = s.points.first().map_or(String::new(), |x| x.bottleneck.clone());
            t.check(&format!("a hot row: the study says {}", word), word.starts_with("Sperre"));
        }
        _ => t.check("the hot study reads", false),
    }
}

// ---------------------------------------------------------------- the other ways in

fn routes(t: &mut T) {
    let mut p = Project::new("db-routes", 22);
    let g = p.add_device("traffic-gen", 0, 0).unwrap_or(0);
    let s = p.add_device("jf-s48x", 100, 0).unwrap_or(0);
    let w = p.add_device("server-1u", 200, 0).unwrap_or(0);
    let d = p.add_device("server-1u", 300, 0).unwrap_or(0);
    let _ = p.connect(&pr(g, "eth0"), &pr(s, "TenGigabitEthernet1/0/1"), None);
    let _ = p.connect(&pr(w, "eth1"), &pr(s, "TenGigabitEthernet1/0/2"), None);
    let _ = p.connect(&pr(d, "eth1"), &pr(s, "TenGigabitEthernet1/0/3"), None);
    let web = r#"{"port": 80, "db": {"addr": "10.0.0.3", "port": 7100, "conns": 32}, "routes": [
        {"path": "/u/*", "backend": "db", "sql": "SELECT * FROM t WHERE id = ?"},
        {"path": "/q", "backend": "db", "sql": "SELECT * FROM t WHERE id = ?", "param": "query:k"},
        {"path": "/d", "backend": "db", "sql": "SELECT * FROM t WHERE id = ?", "param": "draw"},
        {"path": "/n", "backend": "db", "sql": "SELECT COUNT(*) FROM t", "param": "none"},
        {"path": "/bad", "backend": "db", "sql": "SELECT * FROM nix", "param": "none"},
        {"path": "/x", "method": "POST", "backend": "db", "retries": 5, "param": "draw", "sql": ["BEGIN", "SELECT * FROM t WHERE id = ? FOR UPDATE", "UPDATE t SET n = n + 1 WHERE id = ?", "COMMIT"]},
        {"path": "/stuck", "backend": "db", "param": "none", "sql": ["BEGIN", "UPDATE t SET n = 1 WHERE id = 1", "SELECT * FROM nix", "COMMIT"]}
    ]}"#;
    if let Some(x) = p.device_mut(g) {
        x.startup_config = String::from("ip addr add 10.0.0.1/24 dev eth0\n");
    }
    if let Some(x) = p.device_mut(w) {
        x.startup_config = String::from("ip addr add 10.0.0.2/24 dev eth1\n");
        x.services = alloc::vec![def("http-server", "web", web)];
    }
    if let Some(x) = p.device_mut(d) {
        x.startup_config = String::from("ip addr add 10.0.0.3/24 dev eth1\n");
        x.services = alloc::vec![def("db", "db", r#"{"tables": [{"name": "t", "rows": 100, "row_bytes": 100, "columns": ["n"], "dist": "zipf"}], "disk": {"iops": 0, "mbps": 1000, "latency_us": 100}}"#)];
    }
    let get = |ms: u64, path: &str| Step { at: ms * MS, ses: 0, sql: String::new(), get: Some((String::from(path), 80)) };
    let steps = alloc::vec![get(0, "/u/7"), get(10, "/q?k=8"), get(20, "/d"), get(30, "/n"), get(40, "/bad"), get(50, "/stuck"), get(60, "/u/nix")];
    let mut e = engine(&p);
    let _ = e.add_service(g, "drv", Box::new(Driver::new("10.0.0.2", steps.clone())), true);
    run_for(&mut e, SEC);
    let got = got_of(&e, g, steps.len());
    t.check(&format!("a parameter from the path: {}", body_of(&got, 0)), status_of(&got, 0) == 200 && has(&got, 0, "\"id\":7"));
    t.check("from the query", has(&got, 1, "\"id\":8"));
    t.check("drawn: the answer says which", has(&got, 2, "\"drawn\":") && has(&got, 2, "\"count\":1"));
    t.check("no parameter", has(&got, 3, "\"count\":100"));
    t.eq("an error comes through", status_of(&got, 4), 400);
    t.eq("a list that fails", status_of(&got, 5), 400);
    t.eq("its transaction is rolled back", e.metric(d, "db", "rollbacks"), Some(1));
    t.eq("no lock stays", e.service::<Db>(d, "db").map(|x| x.lock_list().len()), Some(0));
    t.eq("a word for a number", status_of(&got, 6), 400);
    // a list on a route under load: a hot row, deadlocks retried
    if let Some(x) = p.device_mut(g) {
        let mix = Value::object([("method", Value::string("POST")), ("path", Value::string("/x")), ("weight", Value::u64(1))]);
        x.services = alloc::vec![def_v("http-load", "load", Value::object([("targets", Value::Array(alloc::vec![Value::string("http://10.0.0.2/")])), ("mode", Value::string("closed")), ("users", Value::u64(16)), ("timeout_ms", Value::u64(60000)), ("mix", Value::Array(alloc::vec![mix]))]))];
    }
    let mut e = engine(&p);
    run_for(&mut e, 2 * SEC);
    let ok = e.metric(g, "load", "ok").unwrap_or(0);
    t.check(&format!("transactions over the route: {}", ok), ok > 50);
    t.eq("each one four statements", e.metric(d, "db", "statements").unwrap_or(0) / 4 >= ok, true);
    t.eq("nothing failed", e.metric(g, "load", "http_errors"), Some(0));
    t.check("its worker waited: the call is counted", e.metric(w, "web", "backend_calls").unwrap_or(0) >= ok * 4);
    // without "db" nothing is drawn and no new event comes: an old server goes as it went
    let old = def("http-server", "web", r#"{"routes": [{"path": "/", "body": "x"}]}"#);
    let mut a = Engine::new(1);
    let _ = (&mut a, old);
}

fn script(t: &mut T) {
    let src = r#"
fn yes(x) { if x { return 1 } return 0 }
let c = db.connect("10.0.0.2")
let two = db.connect("http://10.0.0.2:7100/")
metrics.gauge("wrote", 0)
c.begin(fn(err, r) {
    metrics.gauge("in_txn", r.txn)
    c.query("UPDATE t SET n = ? WHERE id = ?", [42, 3], fn(err, r) {
        metrics.gauge("changed", r.changed)
        two.query("SELECT n FROM t WHERE id = 3", fn(err, r) {
            metrics.gauge("other_sees_42", yes(r.rows[0].n == 42))
            c.commit(fn(err, r) {
                two.query("SELECT n FROM t WHERE id = 3", fn(err, r) {
                    metrics.gauge("wrote", r.rows[0].n)
                    two.query("SELECT * FROM nix", fn(err, r) {
                        metrics.gauge("err_ok", yes(err != nil && r.code == "syntax"))
                        c.close()
                        two.close()
                    })
                })
            })
        })
    })
})
"#;
    let n = net(None, SMALL, None);
    let mut p = n.p.clone();
    if let Some(d) = p.device_mut(n.gen_) {
        d.services = alloc::vec![def_v("script", "s", Value::object([("source", Value::string(src))]))];
    }
    let mut e = engine(&p);
    run_for(&mut e, SEC);
    let m = |name: &str| e.metric(n.gen_, "s", name);
    t.check(&format!("begin gave its number: {:?}; {:?}", m("in_txn"), e.services(n.gen_).iter().map(|s| s.status.clone()).collect::<Vec<_>>()), m("in_txn").is_some_and(|x| x > 0));
    t.eq("query with values", m("changed"), Some(1));
    t.eq("another connection does not see it yet", m("other_sees_42"), Some(0));
    t.eq("after the commit it does", m("wrote"), Some(42));
    t.eq("an error comes as err, with what it was", m("err_ok"), Some(1));
    t.eq("the name is known to the editor", script::GLOBALS.contains(&"db"), true);
}

fn shell(t: &mut T) {
    let c = r#"{"tables": [{"name": "users", "rows": 1000, "row_bytes": 200, "columns": ["email:text"], "indexes": ["email"]}]}"#;
    let n = net(None, c, None);
    let mut e = engine(&n.p);
    run_for(&mut e, SEC);
    let sh = |e: &mut Engine, dev: u32, line: &str| -> String {
        let _ = e.take_reports();
        let _ = e.cli_line(dev, line);
        let to = e.now() + SEC;
        e.run_until(to, BUDGET);
        let mut s = String::new();
        for r in e.take_reports() {
            if let Report::CliOut { dev: d, bytes, .. } = r
                && d == dev
            {
                s.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        s
    };
    let o = sh(&mut e, n.srv, "dbctl");
    t.check(&format!("dbctl: {}", o), o.starts_with("Datenbank db: Port 7100, ") && o.contains("Puffer:") && o.contains("Dieses Gerät simuliert keine Hardware"));
    let o = sh(&mut e, n.srv, "dbctl tables");
    t.check(&format!("dbctl tables: {}", o), o.contains("users") && o.contains("1000") && o.contains("email"));
    let o = sh(&mut e, n.srv, "dbctl locks");
    t.check("dbctl locks", o.contains("keine Zeilensperren"));
    let o = sh(&mut e, n.srv, "dbctl explain SELECT * FROM users WHERE id = 1");
    t.check(&format!("dbctl explain: {}", o), o.contains("\"plan\":\"index\""));
    let o = sh(&mut e, n.srv, "dbctl explain SELECT");
    t.check(&format!("dbctl explain with a mistake: {}", o), o.contains("Zeichen 7:"));
    let o = sh(&mut e, n.srv, "dbctl nix");
    t.check("dbctl nix", o.contains("dbctl: nix kenne ich nicht"));
    let o = sh(&mut e, n.gen_, "dbctl");
    t.check("no database here", o.contains("auf diesem Gerät läuft keine Datenbank"));
    let o = sh(&mut e, n.srv, "man dbctl");
    t.check("its page", o.contains("dbctl") && o.contains("explain"));
    let o = sh(&mut e, n.gen_, &format!("curl -s -d \"SELECT * FROM users WHERE id = 5\" http://{}:7100/query", DB_IP));
    t.check(&format!("curl with raw text: {}", o), o.contains("\"ok\":true") && o.contains("\"id\":5"));
    let o = sh(&mut e, n.gen_, &format!("curl -s -d \"SELEKT\" http://{}:7100/query", DB_IP));
    t.check("curl sees the place of a mistake", o.contains("\"code\":\"syntax\"") && o.contains("\"pos\":1"));
    let o = sh(&mut e, n.gen_, &format!("curl -s http://{}:7100/_db/health", DB_IP));
    t.check(&format!("health: {}", o), o.contains("\"status\":\"ok\"") && o.contains("\"role\":\"primary\"") && o.contains("\"lsn\":0"));
    let o = sh(&mut e, n.gen_, &format!("curl -s http://{}:7100/_db/stats", DB_IP));
    t.check(&format!("stats: {}", o), o.contains("\"statements\":") && o.contains("\"name\":\"users\""));
    let o = sh(&mut e, n.gen_, &format!("curl -s -i http://{}:7100/query", DB_IP));
    t.check("a GET of /query", o.contains("405") && o.contains("Allow: POST"));
    let o = sh(&mut e, n.gen_, &format!("curl -s -i http://{}:7100/nix", DB_IP));
    t.check("no such path", o.contains("404"));
    let o = sh(&mut e, n.gen_, &format!("curl -s -d \"{{\" http://{}:7100/query", DB_IP));
    t.check("broken JSON", o.contains("Der Text beginnt mit { und ist doch kein JSON."));
    let o = sh(&mut e, n.gen_, &format!("curl -s -H \"X-Txn: 77\" -d \"SELECT * FROM users\" http://{}:7100/query", DB_IP));
    t.check("a transaction from a header", o.contains("Die Transaktion 77 gibt es nicht (mehr)."));
}

// ---------------------------------------------------------------- the window

fn form(t: &mut T) {
    let base = cfg(r#"{"tables": [{"name": "users", "rows": 10, "columns": ["a"], "indexes": ["a"]}], "pool_mb": 16}"#);
    // nothing typed, nothing changes
    t.eq("nothing changes", merged("db", &base, &fill("db", &base)).ok(), Some(base.clone()));
    let mut v = fill("db", &base);
    t.eq("a table is a row", (v.rows.len(), v.rows[0].name.as_str(), v.rows[0].addr.as_str(), v.rows[0].indexes.as_str()), (1, "users", "10", "a"));
    t.check("a field", v.set("db", "pool_mb", "64"));
    v.rows[0].addr = String::from("20");
    v.rows[0].dist = String::from("zipf");
    v.rows.push(jfn_sim_row("orders", "5"));
    let out = merged("db", &base, &v).map(|c| agent_core::json::stringify(&c).unwrap_or_default());
    t.eq("the form against the JSON it writes", out, Ok(String::from(r#"{"tables":[{"name":"users","rows":20,"columns":["a"],"indexes":["a"],"dist":"zipf"},{"name":"orders","rows":5}],"pool_mb":64}"#)));
    // what it writes is read back the same
    if let Ok(c) = merged("db", &base, &v) {
        t.eq("and reads it back", fill("db", &c).rows.iter().map(|r| r.name.clone()).collect::<Vec<_>>(), alloc::vec![String::from("users"), String::from("orders")]);
        t.check("the service takes it", jfn_sim::db::from_def_msg(&c).is_ok());
    }
    v.rows.clear();
    t.eq("no tables: the member goes", merged("db", &base, &v).map(|c| c.get("tables").is_none()), Ok(true));
    let mut bad = fill("db", &base);
    bad.rows[0].addr = String::from("viele");
    t.check("rows must be a number", merged("db", &base, &bad).is_err());
    // the defaults come from the service
    let d = jfn_sim::svcform::defaults("db").unwrap_or_else(empty_object);
    t.eq("default pool", d.get("pool_mb").and_then(|x| x.as_u64()), Some(128));
    t.eq("default connections", d.get("max_conns").and_then(|x| x.as_u64()), Some(100));
    t.eq("default durability", d.get("durability").and_then(|x| x.as_str()), Some("full"));
    t.eq("its port", jfn_sim::svcform::port_of(&def("db", "db", r#"{"port": 7200}"#)), Some(7200));
    // live or restart, as predicted
    let good = r#"{"tables": [{"name": "t", "rows": 10, "row_bytes": 100}], "pool_mb": 16}"#;
    let mut p = Project::new("p", 1);
    let id = p.add_device("server-1u", 0, 0).unwrap_or(0);
    if let Some(x) = p.device_mut(id) {
        x.services = alloc::vec![def("db", "db", good)];
    }
    let mut doc = Doc::new();
    doc.replace(p, None);
    let _ = doc.sim.send(Cmd::RunFor(SEC));
    let live = [("pool", r#"{"tables": [{"name": "t", "rows": 10, "row_bytes": 100}], "pool_mb": 32}"#), ("connections", r#"{"tables": [{"name": "t", "rows": 10, "row_bytes": 100}], "pool_mb": 16, "max_conns": 5}"#), ("durability", r#"{"tables": [{"name": "t", "rows": 10, "row_bytes": 100}], "pool_mb": 16, "durability": "relaxed"}"#), ("a cost", r#"{"tables": [{"name": "t", "rows": 10, "row_bytes": 100}], "pool_mb": 16, "stmt_us": 99}"#)];
    for (what, c) in live {
        t.eq(&format!("{}: at once", what), takes(&doc, id, 0, &cfg(c)), Takes::Live);
    }
    let again = [("port", r#"{"tables": [{"name": "t", "rows": 10, "row_bytes": 100}], "pool_mb": 16, "port": 7200}"#), ("tables", r#"{"tables": [{"name": "t", "rows": 11, "row_bytes": 100}], "pool_mb": 16}"#), ("page size", r#"{"tables": [{"name": "t", "rows": 10, "row_bytes": 100}], "pool_mb": 16, "page_kb": 16}"#)];
    for (what, c) in again {
        t.eq(&format!("{}: a restart", what), takes(&doc, id, 0, &cfg(c)), Takes::Restart);
    }
    t.eq("the same: nothing", takes(&doc, id, 0, &cfg(good)), Takes::Same);
}

fn jfn_sim_row(name: &str, rows: &str) -> crate::workspace::devwin::svcform::Row {
    crate::workspace::devwin::svcform::Row { name: String::from(name), addr: String::from(rows), ..Default::default() }
}

fn preset(t: &mut T) {
    let mut doc = Doc::new();
    doc.replace(Project::new("p", 1), None);
    let Ok((id, e)) = edits::add_placed(&mut doc.proj, "database", 0, 0) else {
        t.check("placed", false);
        return;
    };
    doc.record(e);
    let d = doc.proj.device(id).cloned().unwrap_or_default();
    t.eq("the tile brings a database", d.services.iter().map(|s| (s.kind.as_str(), s.name.as_str())).collect::<Vec<_>>(), alloc::vec![("db", "db")]);
    t.check("with a table", d.services[0].config.get("tables").and_then(|x| x.as_array()).is_some_and(|a| a.len() == 1));
    t.check("it starts", jfn_sim::db::from_def_msg(&d.services[0].config).is_ok());
    t.eq("nothing is missing", jfn_sim::svcform::missing("db", &d.services[0].config).map(|m| m.text()), None);
    t.check("in one step", doc.undo());
    t.check("undo takes it all", doc.proj.device(id).is_none());
    t.check("redo brings it", doc.redo());
    t.eq("with its table", doc.proj.device(id).map(|x| x.services[0].config.get("tables").is_some()), Some(true));
    // a project file keeps what it says: a database device with its KV store stays so
    let mut p = Project::new("old", 1);
    let db_dev = p.add_device("database", 0, 0).unwrap_or(0);
    if let Some(x) = p.device_mut(db_dev) {
        x.startup_config = String::from("ip addr add 10.0.0.9/24 dev eth0\n");
        x.services = alloc::vec![def("kv", "kv", r#"{"nodes": ["10.0.0.9"], "rf": 1}"#)];
    }
    let text = p.to_text();
    let again = Project::from_text(&text).unwrap_or_else(|_| Project::new("x", 1));
    t.eq("an old project keeps its KV store", again.device(db_dev).map(|x| x.services[0].kind.clone()), Some(String::from("kv")));
    let mut e = engine(&again);
    run_for(&mut e, SEC);
    t.check("and it runs", e.services(db_dev).iter().any(|s| s.kind == "kv" && s.running));
    // the panels
    let names: Vec<&str> = crate::workspace::metrics::groups("db").iter().flat_map(|g| g.names.iter().copied()).collect();
    for n in ["statements", "p95_us", "pool_hit_pm", "pool_use_pm", "lock_waits", "deadlocks", "group_pm", "wal_flushes"] {
        t.check(&format!("the chart of {}", n), names.contains(&n));
    }
    t.check("a pill", crate::workspace::canvas::lab::pill_of("db").is_some());
}

/// tables that do not fit: every statement hears it, the status and the problems say it
fn too_big(t: &mut T) {
    let n = net(None, r#"{"tables": [{"name": "t", "rows": 1000000000, "row_bytes": 200}]}"#, None);
    let (e, g) = driven(&n, plan(&[(10, 0, "SELECT * FROM t WHERE id = 1")]), SEC);
    t.check(&format!("it answers 507: {}", body_of(&g, 0)), status_of(&g, 0) == 507 && has(&g, 0, "\"code\":\"disk_full\"") && has(&g, 0, "Die Tabellen brauchen"));
    t.check("the status says so", db(&e, &n).is_some_and(|d| d.status().starts_with("Die Tabellen brauchen")));
    t.check("the problems too", e.problems().iter().any(|x| x.kind == jfn_sim::problems::ProblemKind::ServiceNotStarted));
}

/// what changes while it runs (Action::ServiceConfig, svc set): the numbers stay, what comes goes by the new
fn live_change(t: &mut T) {
    let c = r#"{"tables": [{"name": "t", "rows": 100000, "row_bytes": 200}], "pool_mb": 16, "disk": {"iops": 20000, "mbps": 1000, "latency_us": 50}}"#;
    let n = net(None, c, Some(closed(4, DRAW_READ, 0)));
    let mut e = engine(&n.p);
    run_for(&mut e, SEC);
    let before = metric(&e, n.srv, "db", "statements");
    let set = |text: &str| Action::ServiceConfig { dev: n.srv, svc: String::from("db"), config: cfg(text) };
    let bigger = r#"{"tables": [{"name": "t", "rows": 100000, "row_bytes": 200}], "pool_mb": 256, "max_conns": 7, "durability": "relaxed", "stmt_us": 5, "disk": {"iops": 20000, "mbps": 1000, "latency_us": 50}}"#;
    t.check("it takes a pool, connections and costs", e.apply(set(bigger)).is_ok());
    t.eq("the new size", db(&e, &n).map(|d| d.pool().cap()), Some(32768));
    t.check("what it held stays", metric(&e, n.srv, "db", "pool_bytes") > 0);
    t.eq("its settings", db(&e, &n).map(|d| (d.cfg.max_conns, d.cfg.durable, d.cfg.stmt_cpu)), Some((7, false, 5 * US)));
    run_for(&mut e, SEC / 2);
    t.check("the numbers go on", metric(&e, n.srv, "db", "statements") > before);
    let other = r#"{"tables": [{"name": "t", "rows": 100000, "row_bytes": 200}], "port": 7200}"#;
    t.check("a port is refused", e.apply(set(other)).is_err());
    let more = r#"{"tables": [{"name": "t", "rows": 100001, "row_bytes": 200}]}"#;
    t.check("other tables are refused", e.apply(set(more)).is_err());
    t.eq("and nothing changed", db(&e, &n).map(|d| d.pool().cap()), Some(32768));
}

/// a scan of a million pages stays cheap to simulate: pieces, no step per page or row
fn scan_cheap(t: &mut T) {
    let c = r#"{"tables": [{"name": "t", "rows": 40000000, "row_bytes": 200}], "pool_mb": 64, "disk": {"capacity_mb": 10000000, "iops": 0, "mbps": 10000, "latency_us": 10}}"#;
    let n = net(Some(Hw { cores: 4, speed_pm: 1000, ram_mb: 8 << 10, swap_mb: 0, disk: DiskClass::Nvme, disk_gb: 100_000 }), c, None);
    let (mut e, _) = driven(&n, Vec::new(), 100 * MS);
    let _ = e.add_service(n.gen_, "scan", Box::new(Driver::new(DB_IP, plan(&[(200, 0, "SELECT COUNT(*) FROM t")]))), true);
    let (before, wall) = (e.events_done(), plat::time_us());
    run_for(&mut e, 40 * SEC);
    let (events, took) = (e.events_done() - before, plat::time_us() - wall);
    let d = e.service::<Driver>(n.gen_, "scan");
    let got = d.and_then(|x| x.got.get(&0)).map_or(String::new(), |x| x.body.clone());
    t.check(&format!("all rows: {}", got), got.contains("\"count\":40000000") && got.contains("\"rows_examined\":40000000"));
    // 1000000 pages in 64s, 20 s of CPU in 4 ms each
    let want = 1_000_000 / 64 + 5000 + 2000;
    t.check(&format!("a scan of a million pages takes {} events", events), events < want * 2);
    if plat::args().iter().any(|a| a == "--numbers") {
        plat::write_stdout(format!("a scan of 1000000 pages and 40000000 rows: {} events, {} ms here, {:.1} s of virtual time\n", events, took / 1000, figure(&got, "us") / 1e6).as_bytes());
    }
}

/// what a clone costs: it must not grow with the table, only with what is touched
fn costs_of_a_clone(t: &mut T) {
    let body = r#"{"sql":"UPDATE t SET n = n + 1 WHERE id = ?","params":[{"draw":true}]}"#;
    let c = r#"{"tables": [{"name": "t", "rows": 100000000, "row_bytes": 200, "columns": ["n"]}], "touched_max": 20000, "pool_mb": 8192, "durability": "relaxed", "disk": {"capacity_mb": 10000000, "iops": 0, "mbps": 100000, "latency_us": 1}}"#;
    let n = net(Some(Hw { cores: 2, speed_pm: 1000, ram_mb: 128 << 10, swap_mb: 0, disk: DiskClass::Nvme, disk_gb: 100_000 }), c, Some(closed(32, body, 0)));
    let mut e = engine(&n.p);
    run_for(&mut e, 3 * SEC);
    let touched = metric(&e, n.srv, "db", "touched_rows");
    t.check(&format!("rows touched: {}", touched), touched >= 1000);
    let fp = db(&e, &n).map_or(0, |d| d.footprint());
    // the seam of replication: positions and a bounded tail
    let (lsn, tail) = db(&e, &n).map_or((0, None), |d| (d.lsn(), d.wal_since(d.lsn() - 2000)));
    t.check(&format!("the tail of the log reaches back: {:?}", tail.as_ref().map(|x| x.len())), tail.is_some_and(|x| !x.is_empty()) && lsn > 0);
    t.eq("not further than it holds", db(&e, &n).map(|d| d.wal_since(0).is_none()), Some(true));
    // the structure of the pool, the touched rows, nothing per row of the table
    t.check(&format!("a table of 100 million rows holds {} bytes", fp), fp < 40 << 20);
    t.check("the pool is bounded", db(&e, &n).map_or(0, |d| d.pool().bytes()) < 1 << 20);
    let (lat, rate) = (db(&e, &n).map_or(0, |d| d.pool().used()), db(&e, &n).map_or(0, |d| d.pool().rate));
    t.check(&format!("it tracks 1 page in {}, holds {} pages", rate, lat), rate > 1);
    // the time of a clone, with and without its database (only said, the machine decides it)
    if plat::args().iter().any(|a| a == "--numbers") {
        let (mut with, mut alone) = (u64::MAX, u64::MAX);
        let mut plain = e.clone();
        let _ = plain.apply(Action::RemoveService { dev: n.srv, name: String::from("db") });
        for _ in 0..5 {
            let a = plat::time_us();
            let c1 = e.clone();
            let b = plat::time_us();
            let c2 = plain.clone();
            let d = plat::time_us();
            with = with.min(b - a);
            alone = alone.min(d - b);
            let _ = (c1.now(), c2.now());
        }
        plat::write_stdout(format!("a clone with {} touched rows, footprint {} KiB: {} us, without the database {} us; keyframe::footprint {} KiB\n", touched, fp >> 10, with, alone, jfn_sim::keyframe::footprint(&e) >> 10).as_bytes());
    }
    let _ = US;
}
