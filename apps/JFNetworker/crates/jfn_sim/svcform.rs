// svcform -- what the forms of the window ask of the lab's services (SA3): the settings a service
// takes and what it still lacks, so that no default lives twice
//
//   KINDS                               the kinds that have a form, in the order of the dialog
//   effective(kind, &config) -> Result<Value, Msg>   the settings a service with this config runs
//                                       with, in the units of its config: asked of a service built
//                                       by its own reader (from_def_msg, the same as a project's)
//   defaults(kind) -> Option<Value>     the same for the least config its reader takes
//   missing(kind, &config) -> Option<Msg>   what it lacks before it can do anything: a balancer
//                                       without backends, a generator without a target; such a
//                                       service does not start (Engine::service_def refuses it) and
//                                       sends nothing, problems.rs names it
//   port_of(def) -> Option<u64>         the port a service answers on, by its settings
//
// Only the members a form can show come back; "routes", "mix" and the like are the JSON's.

use alloc::boxed::Box;
use alloc::string::String;

use agent_core::jmap::{empty_object, Value};
use agent_core::json::Number;
use jfn_model::experiment::fixed;
use jfn_model::msg;
use jfn_model::msg::Msg;
use jfn_model::project::ServiceDef;

use crate::kernel::{Ns, MS, US};
use crate::os::Service;

/// the kinds with a form
pub const KINDS: [&str; 8] = ["http-server", "http-load", "lb", "kv", "cache", "mq", "worker", "db"];

/// a number with the fraction it has
fn dec(v: u64, unit: u64) -> Value {
    Number::parse(&fixed(v, unit)).map_or(Value::u64(v / unit), Value::Number)
}

/// a time in milliseconds
pub(crate) fn ms_of(v: Ns) -> Value {
    dec(v, MS)
}

/// a time in microseconds
pub(crate) fn us_of(v: Ns) -> Value {
    dec(v, US)
}

/// a number from thousandths (a rate)
pub(crate) fn thousandths(v: u64) -> Value {
    dec(v, 1000)
}

/// a share of parts per million as the fraction a config writes
pub(crate) fn rate_of(ppm: u32) -> Value {
    dec(ppm as u64, 1_000_000)
}

/// what a service holds, in the units of its config
pub(crate) fn mem_into(o: &mut Value, m: &crate::res::SvcMem) {
    o.insert("mem_mb", Value::u64(m.base >> 20));
    o.insert("conn_kb", Value::u64(m.per_conn >> 10));
    o.insert("data_mb", Value::u64(m.data >> 20));
}

/// a service of a kind with a form, from its config
fn built(kind: &str, c: &Value) -> Result<Box<dyn Service>, Msg> {
    match kind {
        "http-server" | "http-load" => crate::httpsvc::from_def_msg(kind, c),
        "lb" => crate::lb::from_def_msg(c),
        "kv" => crate::kv::from_def_msg(c),
        "cache" => crate::cache::from_def_msg(c),
        "mq" | "worker" => crate::mq::from_def_msg(kind, c),
        "db" => crate::db::from_def_msg(c),
        k => Err(msg!("Unbekannte Dienstart \u{201e}{}\u{201c}.", k)),
    }
}

/// the settings of a service with this config
pub fn effective(kind: &str, c: &Value) -> Result<Value, Msg> {
    let s = built(kind, c)?;
    let a = s.as_any();
    let v = match kind {
        "http-server" => a.downcast_ref::<crate::httpsvc::HttpServer>().map(|x| x.settings()),
        "http-load" => a.downcast_ref::<crate::httpsvc::HttpLoad>().map(|x| x.settings()),
        "lb" => a.downcast_ref::<crate::lb::Lb>().map(|x| x.settings()),
        "kv" => a.downcast_ref::<crate::kv::KvNode>().map(|x| x.settings()),
        "cache" => a.downcast_ref::<crate::cache::Cache>().map(|x| x.settings()),
        "mq" => a.downcast_ref::<crate::mq::Queue>().map(|x| x.settings()),
        "worker" => a.downcast_ref::<crate::mq::Worker>().map(|x| x.settings()),
        "db" => a.downcast_ref::<crate::db::Db>().map(|x| x.settings()),
        _ => None,
    };
    Ok(v.unwrap_or_else(empty_object))
}

/// the least config the reader of this kind takes
fn least(kind: &str) -> Value {
    let local = Value::string("http://127.0.0.1/");
    let ip = Value::string("127.0.0.1");
    match kind {
        "http-load" => Value::object([("targets", Value::Array(alloc::vec![local]))]),
        "lb" => Value::object([("backends", Value::Array(alloc::vec![Value::object([("addr", ip)])]))]),
        "kv" => Value::object([("nodes", Value::Array(alloc::vec![ip]))]),
        "cache" => Value::object([("backend", local)]),
        "worker" => Value::object([("queue", local)]),
        "db" => Value::object([("tables", Value::Array(alloc::vec![Value::object([("name", Value::string("t")), ("rows", Value::u64(1))])]))]),
        _ => empty_object(),
    }
}

/// the settings of a service whose config says nothing
pub fn defaults(kind: &str) -> Option<Value> {
    effective(kind, &least(kind)).ok()
}

/// a member that holds a text or a list with something in it
fn has(c: &Value, key: &str) -> bool {
    match c.get(key) {
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        _ => false,
    }
}

/// what the service lacks before it can do anything
pub fn missing(kind: &str, c: &Value) -> Option<Msg> {
    match kind {
        "lb" if !has(c, "backends") => Some(msg!("Noch keine Backends: der Load Balancer hat niemanden, an den er verteilen kann.")),
        "http-load" if !has(c, "target") && !has(c, "targets") => Some(msg!("Noch kein Ziel: der Lastgenerator sendet nichts.")),
        "kv" if !has(c, "nodes") => Some(msg!("Noch keine Knoten: der KV-Speicher weiß nicht, wer dazugehört.")),
        "cache" if !has(c, "backend") => Some(msg!("Noch kein Backend: der Cache kann nichts nachladen.")),
        "worker" if !has(c, "queue") => Some(msg!("Noch keine Queue: der Worker holt nichts ab.")),
        "db" if !has(c, "tables") => Some(msg!("Noch keine Tabellen: die Datenbank hat nichts, worauf sich eine Anweisung beziehen könnte.")),
        _ => None,
    }
}

/// the port a service answers on
pub fn port_of(def: &ServiceDef) -> Option<u64> {
    if let Ok(v) = effective(&def.kind, &def.config) {
        return v.get("port").and_then(|p| p.as_u64());
    }
    // (a service that does not start yet: what its config says, else its kind's)
    def.config.get("port").and_then(|p| p.as_u64()).or_else(|| defaults(&def.kind)?.get("port")?.as_u64())
}

/// a word of a config as the settings write it
pub(crate) fn word(s: &str) -> Value {
    Value::String(String::from(s))
}
