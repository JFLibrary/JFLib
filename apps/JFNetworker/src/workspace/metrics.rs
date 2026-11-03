// metrics -- the Metriken panel: the lab's services as a tree (by device: kind, name, state), the
// KPI tiles of the chosen one and a line chart per group of its time series, with the faults as
// shaded ranges
//
//   Metrics                  the tree's state, the chosen service, the span of the charts, what
//                            was read from the engine for them (kept until it changes)
//   panel(n, ui)             the panel (dock panel P_METRICS, bottom)
//   states(n, ui)            CMD_METRICS checked while the panel shows, CMD_HEAL_ALL enabled
//                            while a fault is active
//   command(n, ui, id, arg)  CMD_METRICS: the panel shows, its tree gets the keyboard;
//                            CMD_HEAL_ALL: every active fault heals (one undo step)
//   kind_name(kind)          "Loadbalancer" for lb, "KV-Knoten" for kv ... (the language)
//   series_label(name)       "Anfragen/s" for requests, "p99" for p99_us ... (the language;
//                            other series keep their technical name)
//   groups(kind) / Group     the charts of a kind of service: a title, how its values read,
//                            the names of its series
//   kpi_tiles(kpi, share, sep)   the tiles of a KPI as (label, value) in the language
//   rebin(buckets) -> Vec<(upper, count)>   a histogram's buckets as bars of 1, 2, 5 times a power of ten
//
// The tree: every configured service of every device (Engine::service_list) in the order of
// the project; a device is a node with its services below ("Loadbalancer lb", its state at the
// right: läuft, gestoppt, aus, ohne Strom).  A click or the arrow keys choose one, a device row
// its first service; a single device chosen on the canvas chooses its first service too.  The
// right side: the service with its state and status line (Service::status_msg), the KPI tiles
// over the last 5 s (Engine::kpi: requests per second, p50 / p95 / p99, error rate, waiting
// work, work in flight, a cache's hits, a KV node's load and share of all KV nodes
// (Engine::shard_load), keys and data), the disk of services that store (Disk::msg), then a
// chart per group of series the service has (throughput, latency quantiles together, errors,
// waiting work ...: groups) over the span chosen (30 s, 1 min, 5 min).  Values read as the
// charts write them (LcUnit: latencies in ms or µs, data in kB or MB).
//
// A device with hardware (SA1, jfn_sim's res.rs) has a row of its own after its services, "Hardware":
// tiles and charts of its CPU, run queue, memory and disk (the series named HOST): how busy the disk is,
// the longest wait for it, what waits for it now.  A service with a latency window has a tile for p99.9
// and bars of its latencies over the chosen span (Engine::lat_hist_between, read from the window: no
// event of its own).  The last row of the tree, "Kabel" with "Alle Kabel": the cables of the project with
// how much of their bit rate they carried each way over the last 5 s, the busiest first, and a chart
// of the busiest (Engine::cable_util).
//
// Time: real time follows the newest samples; the simulation mode ends the charts at the
// moment it shows (also a view of the past), and the tiles cover the 5 s before it
// (Engine::kpi_between).  The faults that overlap a chart (Engine::chart_marks) are shaded
// ranges in the error colour with their text as the tag; an active one reaches the end of the
// view.
//
// Redrawing: the engine's series are read again only when series_gen, the moment shown, the
// span, the faults, the language or the chosen service change; the SeriesBufs stay as they are
// in between, so the charts (which keep their drawing per series version and view) do not
// redraw.  Keyboard: the tree, the span and every chart are Tab stops (a chart's arrow keys
// walk its points, and its record reads them out).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_sim::lab::SvcBrief;
use jfn_sim::{Kpi, Ns, SEC};
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{t, tc, tf};
use crate::app::Net;
use crate::workspace::icons::JN_METRICS;

/// the span of the KPI tiles
pub(crate) const KPI_SPAN: Ns = 5 * SEC;
/// the spans of the charts (s) and their labels
// i18n
pub(crate) const SPANS: [(u64, &str); 3] = [(30, "30 s"), (60, "1 min"), (300, "5 min")];
/// the span when the panel starts (1 min)
const SPAN_START: i32 = 1;
/// rows of the tree at most (its ids)
const ROWS_MAX: usize = 2000;
/// the width of a tile, the least width of a chart, the height of a chart with its legend, the
/// height CMD_METRICS gives the bottom of the dock at least (DIP)
const TILE_W: i32 = 88;
const CHART_MIN_W: i32 = 300;
const CHART_H: i32 = 180;
const PANEL_H: i32 = 400;
/// the cables the row Alle Kabel lists and draws at most
const CABLE_ROWS: usize = 40;
const CABLE_CURVES: usize = 6;

/// how the values of a chart read
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Show {
    /// numbers (rates per second, counts)
    Num,
    /// latencies (the series are µs, the chart reads ms / µs)
    Lat,
    /// a share (the series is per mille, the chart reads percent)
    Pct,
    /// bytes (and bytes per second)
    Bytes,
}

impl Show {
    pub(crate) fn unit(self) -> LcUnit<'static> {
        match self {
            Show::Num => LcUnit::Num(""),
            Show::Lat => LcUnit::Secs,
            Show::Pct => LcUnit::Num("%"),
            Show::Bytes => LcUnit::Si("B"),
        }
    }
}

/// a chart of a kind of service: its title (a context after '|': tc), how its values read,
/// its series
pub(crate) struct Group {
    pub title: &'static str,
    pub show: Show,
    pub names: &'static [&'static str],
}

const LAT3: &[&str] = &["p50_us", "p95_us", "p99_us"];

// i18n: title
static G_HTTP: [Group; 4] = [
    Group { title: "Durchsatz", show: Show::Num, names: &["requests", "responses"] },
    Group { title: "Latenz", show: Show::Lat, names: LAT3 },
    Group { title: "Fehler|Diagramm", show: Show::Num, names: &["status_5xx", "rejected", "backend_errors"] },
    Group { title: "Warteschlange", show: Show::Num, names: &["queued", "inflight"] },
];
// i18n: title
static G_LOAD: [Group; 4] = [
    Group { title: "Durchsatz", show: Show::Num, names: &["sent", "responses"] },
    Group { title: "Latenz", show: Show::Lat, names: LAT3 },
    Group { title: "Fehler|Diagramm", show: Show::Num, names: &["http_errors", "failed", "timeouts", "errors"] },
    Group { title: "In Arbeit", show: Show::Num, names: &["inflight"] },
];
// i18n: title
static G_LB: [Group; 5] = [
    Group { title: "Durchsatz", show: Show::Num, names: &["requests", "responses", "connections"] },
    Group { title: "Latenz", show: Show::Lat, names: LAT3 },
    Group { title: "Fehler|Diagramm", show: Show::Num, names: &["errors", "no_backend", "retries"] },
    Group { title: "In Arbeit", show: Show::Num, names: &["active"] },
    Group { title: "Backends", show: Show::Num, names: &["backends_up", "backends"] },
];
// i18n: title
static G_KV: [Group; 5] = [
    Group { title: "Durchsatz", show: Show::Num, names: &["requests", "reads", "writes"] },
    Group { title: "Latenz", show: Show::Lat, names: LAT3 },
    Group { title: "Fehler|Diagramm", show: Show::Num, names: &["unavailable", "full"] },
    Group { title: "Replikate", show: Show::Num, names: &["replica_reads", "replica_writes", "read_repairs"] },
    Group { title: "Knoten", show: Show::Num, names: &["members_up"] },
];
// i18n: title
static G_CACHE: [Group; 4] = [
    Group { title: "Durchsatz", show: Show::Num, names: &["requests", "hits", "misses"] },
    Group { title: "Latenz", show: Show::Lat, names: LAT3 },
    Group { title: "Fehler|Diagramm", show: Show::Num, names: &["backend_errors"] },
    Group { title: "Inhalt", show: Show::Num, names: &["entries"] },
];
// i18n: title
static G_DB: [Group; 6] = [
    Group { title: "Durchsatz", show: Show::Num, names: &["statements", "commits", "rollbacks"] },
    Group { title: "Latenz", show: Show::Lat, names: LAT3 },
    Group { title: "Puffer", show: Show::Pct, names: &["pool_hit_pm", "pool_use_pm"] },
    Group { title: "Sperren", show: Show::Num, names: &["lock_waits", "deadlocks", "lock_timeouts", "conflicts"] },
    Group { title: "Log", show: Show::Num, names: &["wal_flushes", "group_pm", "lost_commits"] },
    Group { title: "Fehler|Diagramm", show: Show::Num, names: &["errors", "refused"] },
];
// i18n: title
static G_MQ: [Group; 4] = [
    Group { title: "Durchsatz", show: Show::Num, names: &["published", "delivered", "acked"] },
    Group { title: "Latenz", show: Show::Lat, names: LAT3 },
    Group { title: "Fehler|Diagramm", show: Show::Num, names: &["nacked", "dead", "rejected"] },
    Group { title: "Warteschlange", show: Show::Num, names: &["depth", "inflight"] },
];
// i18n: title
static G_WORKER: [Group; 4] = [
    Group { title: "Durchsatz", show: Show::Num, names: &["taken", "done"] },
    Group { title: "Latenz", show: Show::Lat, names: LAT3 },
    Group { title: "Fehler|Diagramm", show: Show::Num, names: &["failed"] },
    Group { title: "Auslastung", show: Show::Num, names: &["busy"] },
];
// i18n: title
static G_DNS: [Group; 2] = [
    Group { title: "Durchsatz", show: Show::Num, names: &["queries", "answers"] },
    Group { title: "Fehler|Diagramm", show: Show::Num, names: &["nxdomain", "servfail", "refused"] },
];
// i18n: title
static G_DHCP: [Group; 3] = [
    Group { title: "Durchsatz", show: Show::Num, names: &["discovers", "offers", "requests", "acks"] },
    Group { title: "Fehler|Diagramm", show: Show::Num, names: &["naks", "declines", "conflicts"] },
    Group { title: "Leases", show: Show::Num, names: &["leases"] },
];
// i18n: title
static G_SCRIPT: [Group; 4] = [
    Group { title: "Durchsatz", show: Show::Num, names: &["calls"] },
    Group { title: "Fehler|Diagramm", show: Show::Num, names: &["errors", "dropped", "rejected"] },
    Group { title: "Warteschlange", show: Show::Num, names: &["queued"] },
    Group { title: "Rechenzeit", show: Show::Num, names: &["busy_us"] },
];
// i18n: title
static G_OTHER: [Group; 4] = [
    Group { title: "Durchsatz", show: Show::Num, names: &["requests", "responses", "queries", "calls", "sent"] },
    Group { title: "Latenz", show: Show::Lat, names: LAT3 },
    Group { title: "Fehler|Diagramm", show: Show::Num, names: &["errors", "failed", "rejected", "dropped"] },
    Group { title: "Warteschlange", show: Show::Num, names: &["queued", "inflight", "active"] },
];

// i18n: title
static G_SMTP: [Group; 3] = [
    Group { title: "Durchsatz", show: Show::Num, names: &["received", "delivered", "relayed"] },
    Group { title: "Fehler|Diagramm", show: Show::Num, names: &["bounced", "deferred", "relay_denied", "rejected"] },
    Group { title: "Warteschlange", show: Show::Num, names: &["queued", "active"] },
];
// i18n: title
static G_POP3: [Group; 3] = [
    Group { title: "Durchsatz", show: Show::Num, names: &["retrieved", "deleted", "logins"] },
    Group { title: "Fehler|Diagramm", show: Show::Num, names: &["failed_logins"] },
    Group { title: "In Arbeit", show: Show::Num, names: &["active"] },
];

// i18n: title
static G_HOST: [Group; 6] = [
    Group { title: "CPU", show: Show::Pct, names: &["cpu_pm"] },
    Group { title: "Warteschlange", show: Show::Num, names: &["runq", "running"] },
    Group { title: "Speicher", show: Show::Bytes, names: &["mem_bytes", "swap_bytes"] },
    Group { title: "Disk", show: Show::Bytes, names: &["disk_bytes"] },
    Group { title: "Disk belegt", show: Show::Pct, names: &["disk_pm"] },
    Group { title: "Warten auf die Disk", show: Show::Lat, names: &["disk_wait_us", "disk_backlog_us"] },
];

static G_IMAP: [Group; 3] = [
    Group { title: "Durchsatz", show: Show::Num, names: &["fetched", "stored", "expunged", "appended", "logins"] },
    Group { title: "Fehler|Diagramm", show: Show::Num, names: &["failed_logins"] },
    Group { title: "In Arbeit", show: Show::Num, names: &["active"] },
];

/// the charts of a kind of service
pub(crate) fn groups(kind: &str) -> &'static [Group] {
    match kind {
        "http-server" => &G_HTTP,
        "http-load" | "http-client" => &G_LOAD,
        "lb" => &G_LB,
        "kv" => &G_KV,
        "db" => &G_DB,
        "cache" => &G_CACHE,
        "mq" => &G_MQ,
        "worker" => &G_WORKER,
        "dns-server" => &G_DNS,
        "dhcp-server" => &G_DHCP,
        "script" => &G_SCRIPT,
        "smtp-server" => &G_SMTP,
        "pop3-server" => &G_POP3,
        "imap-server" => &G_IMAP,
        "host" => &G_HOST,
        _ => &G_OTHER,
    }
}

/// a series' name in the window
struct Lbl {
    key: &'static str,
    label: &'static str,
}

/// the series of the charts in words (rates per second end in "/s")
// i18n: label
static LABELS: [Lbl; 80] = [
    Lbl { key: "requests", label: "Anfragen/s" },
    Lbl { key: "responses", label: "Antworten/s" },
    Lbl { key: "connections", label: "Verbindungen/s" },
    Lbl { key: "errors", label: "Fehler/s" },
    Lbl { key: "status_5xx", label: "5xx/s" },
    Lbl { key: "rejected", label: "Abgewiesen/s" },
    Lbl { key: "backend_errors", label: "Backend-Fehler/s" },
    Lbl { key: "queued", label: "Wartend" },
    Lbl { key: "inflight", label: "In Arbeit" },
    Lbl { key: "active", label: "In Arbeit" },
    Lbl { key: "p50_us", label: "p50" },
    Lbl { key: "p95_us", label: "p95" },
    Lbl { key: "p99_us", label: "p99" },
    Lbl { key: "mean_us", label: "Mittel" },
    Lbl { key: "max_us", label: "Maximum" },
    Lbl { key: "sent", label: "Gesendet/s" },
    Lbl { key: "ok", label: "Erfolgreich/s" },
    Lbl { key: "http_errors", label: "HTTP-Fehler/s" },
    Lbl { key: "failed", label: "Gescheitert/s" },
    Lbl { key: "timeouts", label: "Zeitüberschreitungen/s" },
    Lbl { key: "no_backend", label: "Ohne Backend/s" },
    Lbl { key: "retries", label: "Wiederholt/s" },
    Lbl { key: "backends_up", label: "Verfügbare Backends" },
    Lbl { key: "backends", label: "Backends" },
    Lbl { key: "reads", label: "Gelesen/s" },
    Lbl { key: "writes", label: "Geschrieben/s" },
    Lbl { key: "unavailable", label: "Nicht verfügbar/s" },
    Lbl { key: "full", label: "Disk voll/s" },
    Lbl { key: "replica_reads", label: "Replikate gelesen/s" },
    Lbl { key: "replica_writes", label: "Replikate geschrieben/s" },
    Lbl { key: "read_repairs", label: "Read Repair/s" },
    Lbl { key: "repaired", label: "Repariert/s" },
    Lbl { key: "members_up", label: "Erreichbare Knoten" },
    Lbl { key: "keys", label: "Schlüssel" },
    Lbl { key: "bytes", label: "Daten" },
    Lbl { key: "hits", label: "Treffer/s" },
    Lbl { key: "misses", label: "Fehlschläge/s" },
    Lbl { key: "entries", label: "Einträge" },
    Lbl { key: "published", label: "Veröffentlicht/s" },
    Lbl { key: "delivered", label: "Zugestellt/s" },
    Lbl { key: "acked", label: "Bestätigt/s" },
    Lbl { key: "nacked", label: "Zurückgegeben/s" },
    Lbl { key: "dead", label: "Aussortiert/s" },
    Lbl { key: "depth", label: "Wartend" },
    Lbl { key: "taken", label: "Angenommen/s" },
    Lbl { key: "done", label: "Erledigt/s" },
    Lbl { key: "busy", label: "Beschäftigt" },
    Lbl { key: "queries", label: "Anfragen/s" },
    Lbl { key: "answers", label: "Antworten/s" },
    Lbl { key: "nxdomain", label: "NXDOMAIN/s" },
    Lbl { key: "servfail", label: "SERVFAIL/s" },
    Lbl { key: "refused", label: "Abgelehnt/s" },
    Lbl { key: "discovers", label: "DISCOVER/s" },
    Lbl { key: "offers", label: "OFFER/s" },
    Lbl { key: "acks", label: "ACK/s" },
    Lbl { key: "naks", label: "NAK/s" },
    Lbl { key: "declines", label: "DECLINE/s" },
    Lbl { key: "conflicts", label: "Konflikte/s" },
    Lbl { key: "lost_commits", label: "Verlorene Commits/s" },
    Lbl { key: "leases", label: "Leases" },
    Lbl { key: "calls", label: "Rückrufe/s" },
    Lbl { key: "received", label: "Empfangen/s" },
    Lbl { key: "relayed", label: "Weitergeleitet/s" },
    Lbl { key: "bounced", label: "Unzustellbar/s" },
    Lbl { key: "deferred", label: "Zurückgestellt/s" },
    Lbl { key: "relay_denied", label: "Relay abgelehnt/s" },
    Lbl { key: "retrieved", label: "Abgeholt/s" },
    Lbl { key: "failed_logins", label: "Abgelehnte Anmeldungen/s" },
    Lbl { key: "deleted", label: "Gelöscht/s" },
    Lbl { key: "logins", label: "Anmeldungen/s" },
    Lbl { key: "statements", label: "Anweisungen/s" },
    Lbl { key: "commits", label: "Commits/s" },
    Lbl { key: "rollbacks", label: "Rollbacks/s" },
    Lbl { key: "pool_hit_pm", label: "Treffer im Puffer" },
    Lbl { key: "pool_use_pm", label: "Puffer belegt" },
    Lbl { key: "lock_waits", label: "Warten auf Sperren/s" },
    Lbl { key: "deadlocks", label: "Verklemmungen/s" },
    Lbl { key: "lock_timeouts", label: "Sperren zu spät/s" },
    Lbl { key: "wal_flushes", label: "Log geschrieben/s" },
    Lbl { key: "group_pm", label: "Commits je Schreiben (Tausendstel)" },
];

/// a series' name in the window: "Anfragen/s" for requests, "p99" for p99_us (the language);
/// "busy_us" and the other ones without a word keep their technical name
pub(crate) fn series_label(name: &str) -> String {
    match LABELS.iter().find(|l| l.key == name) {
        Some(l) => String::from(t(l.label)),
        None if name == "busy_us" => String::from(t("Rechenzeit (µs/s)")),
        None if name == "dropped" => String::from(t("Verworfen/s")),
        None if name == "cpu_pm" => String::from(t("Auslastung")),
        None if name == "runq" => String::from(t("Wartende Aufgaben")),
        None if name == "running" => String::from(t("Belegte Kerne")),
        None if name == "mem_bytes" => String::from(t("Im RAM")),
        None if name == "swap_bytes" => String::from(t("Im Swap")),
        None if name == "disk_bytes" => String::from(t("Disk/s")),
        None if name == "disk_pm" => String::from(t("Belegt")),
        None if name == "disk_wait_us" => String::from(t("Längste Wartezeit")),
        None if name == "disk_backlog_us" => String::from(t("Rückstand")),
        None => String::from(name),
    }
}

/// the name of a kind of service (the language)
pub(crate) fn kind_name(kind: &str) -> &str {
    match kind {
        "lb" => t("Loadbalancer"),
        "kv" => t("KV-Knoten"),
        "db" => t("Datenbank"),
        "cache" => t("Cache"),
        "mq" => t("Queue"),
        "worker" => t("Worker"),
        "http-server" => t("Webserver"),
        "http-load" => t("Lastgenerator"),
        "http-client" => t("HTTP-Client"),
        "dns-server" => t("DNS-Server"),
        "dhcp-server" => t("DHCP-Server"),
        "dhcp-relay" => t("DHCP-Relay"),
        "dhcp-client" => t("DHCP-Client"),
        "syslog-server" => t("Syslog-Server"),
        "ntp" => t("Zeit (NTP)"),
        "script" => t("Skript"),
        "tcp-echo" => t("TCP-Echo"),
        "tcp-discard" => t("TCP-Discard"),
        "tcp-client" => t("TCP-Testclient"),
        "rip" => t("Routing (RIP)"),
        "ospf" => t("Routing (OSPF)"),
        "tftp-server" => t("TFTP-Server"),
        "ftp-server" => t("FTP-Server"),
        "smtp-server" => t("Mailserver (SMTP)"),
        "pop3-server" => t("Postfach (POP3)"),
        "imap-server" => t("Postfach (IMAP)"),
        "radius-server" => t("RADIUS-Server"),
        "telnetd" => t("Telnet-Server"),
        "sshd" => t("SSH-Server"),
        "host" => t("Hardware"),
        "cables" => t("Alle Kabel"),
        _ => kind,
    }
}

/// a row of the tree, the head of the charts: "Loadbalancer lb"; a device's hardware has no name
fn svc_label(kind: &str, name: &str) -> String {
    if matches!(kind, "host" | "cables") { String::from(kind_name(kind)) } else { tf("{} {}", &[kind_name(kind), name]) }
}

/// what the requests of a kind of service are called on its first tile
fn rate_label(kind: &str) -> &'static str {
    match kind {
        "http-load" | "http-client" => t("Antworten/s"),
        "mq" => t("Nachrichten/s"),
        "worker" => t("Erledigt/s"),
        _ => t("Anfragen/s"),
    }
}

/// the KPI tiles of a service: (label, value) in the language, the values as the charts write
/// them; share: a KV node's part of the load of all KV nodes (per mille)
pub(crate) fn kpi_tiles(k: &Kpi, share: Option<i64>, sep: u32) -> Vec<(String, String)> {
    let num = |v: f64| LcUnit::Num("").text(v, sep);
    let pct = |pm: i64| LcUnit::Num("%").text(pm as f64 / 10.0, sep);
    let lat = |us: i64| LcUnit::Secs.text(us as f64 * 1e-6, sep);
    let mut out: Vec<(String, String)> = Vec::new();
    if let Some(v) = k.rps_milli {
        out.push((String::from(rate_label(k.kind)), num(v as f64 / 1000.0)));
    }
    for (name, v) in [("p50", k.p50_us), ("p95", k.p95_us), ("p99", k.p99_us), ("p99.9", k.p999_us)] {
        if let Some(v) = v {
            out.push((String::from(name), lat(v)));
        }
    }
    if let Some(v) = k.err_pm {
        out.push((String::from(t("Fehlerquote")), pct(v)));
    }
    if let Some(v) = k.queue {
        out.push((String::from(t("Wartend")), num(v as f64)));
    }
    if let Some(v) = k.inflight {
        out.push((String::from(t("In Arbeit")), num(v as f64)));
    }
    if let Some(v) = k.hit_pm {
        out.push((String::from(t("Trefferquote")), pct(v)));
    }
    if let Some(v) = k.load_milli {
        out.push((String::from(t("Last/s")), num(v as f64 / 1000.0)));
    }
    if let Some(v) = share {
        out.push((String::from(t("Lastanteil")), pct(v)));
    }
    if let Some(v) = k.keys {
        out.push((String::from(t("Schlüssel")), num(v as f64)));
    }
    if let Some(v) = k.bytes {
        out.push((String::from(t("Daten")), LcUnit::Si("B").text(v as f64, sep)));
    }
    out
}

/// a histogram's buckets (smallest value, width, count) as bars: the upper limit of each and its count,
/// limits of 1, 2 and 5 times a power of ten, from the first bucket's to the last one's
pub(crate) fn rebin(buckets: &[(u64, u64, u64)]) -> Vec<(u64, u64)> {
    let Some(last) = buckets.iter().map(|b| b.0 + b.1).max() else { return Vec::new() };
    let first = buckets.iter().map(|b| b.0 + b.1).min().unwrap_or(1);
    let mut limits = Vec::new();
    let mut decade = 1u64;
    'each: loop {
        for m in [1u64, 2, 5] {
            let Some(l) = decade.checked_mul(m) else { break 'each };
            if l >= first {
                limits.push(l);
            }
            if l >= last {
                break 'each;
            }
        }
        decade = match decade.checked_mul(10) {
            Some(d) => d,
            None => break,
        };
    }
    let mut out: Vec<(u64, u64)> = limits.iter().map(|&l| (l, 0)).collect();
    for b in buckets {
        // a bucket counts for the first limit at or above its end
        if let Some(x) = out.iter_mut().find(|x| x.0 >= b.0 + b.1) {
            x.1 += b.2;
        } else if let Some(x) = out.last_mut() {
            x.1 += b.2;
        }
    }
    out
}

/// a cable in the panel: its ends, its bit rate, how much it carried each way lately (thousandths of a
/// percent), the busier way over time
struct CableRow {
    name: String,
    kbps: u32,
    ab: i64,
    ba: i64,
    buf: SeriesBuf,
}

/// what the charts and tiles show of the chosen service, and what it was read for
struct Data {
    key: (u32, String, u64, Option<Ns>, i32, u64, bool),
    /// the bars of the latencies over the span
    bars: Vec<(u64, u64)>,
    /// the cables (the row Alle Kabel)
    cables: Vec<CableRow>,
    /// the series: name, points (x: ns of virtual time; latencies in s)
    series: Vec<(&'static str, SeriesBuf)>,
    kpi: Option<Kpi>,
    share: Option<i64>,
    /// the faults: start, end (ns), text
    marks: Vec<(f64, f64, String)>,
}

pub(crate) struct Metrics {
    tree: TreeState,
    /// the chosen service (device, name)
    sel: Option<(u32, String)>,
    /// the choice the tree's cursor was put on last
    cur_for: Option<(u32, String)>,
    /// the devices the tree has opened once (they start open)
    seen: BTreeSet<u32>,
    /// the single device chosen on the canvas in the last frame
    canvas_dev: Option<u32>,
    /// the span of the charts: index into SPANS
    span: i32,
    /// the width of the tree (DIP)
    split: i32,
    data: Option<Data>,
}

impl Metrics {
    pub(crate) fn new() -> Metrics {
        Metrics { tree: TreeState::default(), sel: None, cur_for: None, seen: BTreeSet::new(), canvas_dev: None, span: SPAN_START, split: 220, data: None }
    }
}

/// enabled / checked of CMD_METRICS and CMD_HEAL_ALL (every frame)
pub(crate) fn states(n: &Net, ui: &mut Ui) {
    ui.cmd_state(CMD_METRICS, if n.shell.dock.visible(P_METRICS) { CF_CHECKED } else { 0 });
    let any = n.doc.sim.engine().active_faults().next().is_some() && !crate::workspace::task::lock(n).faults;
    ui.cmd_state(CMD_HEAL_ALL, if any { 0 } else { CF_DISABLED });
}

/// the commands of the lab
pub(crate) fn command(n: &mut Net, ui: &mut Ui, id: u32, _arg: u32) -> bool {
    match id {
        CMD_METRICS => {
            // the panel comes forward, tall enough for the tiles and a row of charts
            let dock = &mut n.shell.dock;
            dock.show(P_METRICS);
            dock.activate(P_METRICS);
            if dock.side(P_METRICS) == Some(DOCK_BOTTOM) && dock.size(DOCK_BOTTOM) < PANEL_H {
                dock.set_size(DOCK_BOTTOM, PANEL_H);
            }
            ui.focus(ID_MET_TREE);
        }
        CMD_HEAL_ALL => {
            if !crate::workspace::task::lock(n).faults {
                let _ = n.doc.heal_all();
            }
        }
        _ => return false,
    }
    ui.repass();
    true
}

/// the key of a device row and of a service row of the tree
fn dev_key(dev: u32) -> u64 {
    dev as u64
}
fn svc_key(dev: u32, k: usize) -> u64 {
    1 << 40 | (dev as u64) << 8 | (k as u64 & 0xFF)
}

/// the devices the simulator has without power
fn unpowered(n: &Net) -> BTreeSet<u32> {
    n.doc.sim.view.devices.iter().filter(|d| !d.power).map(|d| d.dev).collect()
}

/// the state of a service in words (the language) and as a badge: off with its device (`off`:
/// the devices without power), running, stopped (by a fault or a command), switched off
pub(crate) fn svc_state(off: &BTreeSet<u32>, s: &SvcBrief) -> (&'static str, u32) {
    if off.contains(&s.dev) {
        (t("ohne Strom"), BADGE_NEUTRAL)
    } else if s.running {
        (t("läuft"), BADGE_OK)
    } else if s.enabled {
        (t("gestoppt"), BADGE_WARN)
    } else {
        (t("aus"), BADGE_NEUTRAL)
    }
}

/// the services of the project's devices, in the project's order
fn by_device(n: &Net) -> Vec<(u32, Vec<SvcBrief>)> {
    let mut all: BTreeMap<u32, Vec<SvcBrief>> = BTreeMap::new();
    for s in n.doc.sim.state().service_list() {
        all.entry(s.dev).or_default().push(s);
    }
    // a device with hardware: a row of its own after its services
    for d in &n.doc.proj.devices {
        if n.doc.sim.state().res_of(d.id).is_some() {
            all.entry(d.id).or_default().push(SvcBrief { dev: d.id, name: String::from(jfn_sim::res::HOST), kind: "host", enabled: true, running: true });
        }
    }
    let mut out: Vec<(u32, Vec<SvcBrief>)> = n.doc.proj.devices.iter().filter_map(|d| all.remove(&d.id).map(|v| (d.id, v))).collect();
    // the cables: the last row, beside the services of a project that has any
    if !out.is_empty() && !n.doc.proj.links.is_empty() {
        out.push((CABLES, alloc::vec![SvcBrief { dev: CABLES, name: String::from("_cables"), kind: "cables", enabled: true, running: true }]));
    }
    out
}

/// the row of the cables stands where a device's would (no device has this id)
const CABLES: u32 = u32::MAX;

/// the Metriken panel
pub(crate) fn panel(n: &mut Net, ui: &mut Ui) {
    let devs = by_device(n);
    if devs.is_empty() {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER | pad(12), 0);
        ui.empty_state(
            JN_METRICS,
            t("Keine Dienste"),
            t("Hier stehen die Zahlen der Dienste: Loadbalancer, KV-Speicher, Caches, Queues, Webserver und Lastgeneratoren aus der Projektdatei, mit Kacheln und Verläufen über die Zeit."),
        );
        ui.box_end();
        return;
    }
    follow_canvas(n, &devs);
    // the chosen service must exist (else the first of the lab's own kinds, else the first)
    let ok = n.metrics.sel.as_ref().is_some_and(|(d, s)| devs.iter().any(|(x, v)| x == d && v.iter().any(|y| &y.name == s)));
    if !ok {
        let lab = devs.iter().flat_map(|(d, v)| v.iter().map(move |s| (*d, s))).find(|(_, s)| matches!(s.kind, "lb" | "kv" | "cache" | "mq"));
        n.metrics.sel = lab.or_else(|| devs.first().and_then(|(d, v)| v.first().map(|s| (*d, s)))).map(|(d, s)| (d, s.name.clone()));
    }
    let mut split = n.metrics.split;
    ui.next_size(-1, -1);
    ui.split_begin(ID_MET_SPLIT, split, false);
    tree(n, ui, &devs);
    ui.split_next();
    detail(n, ui, &devs);
    if ui.split_end(&mut split) {
        n.metrics.split = split;
    }
}

// a single device chosen on the canvas: its first service (once per choice)
fn follow_canvas(n: &mut Net, devs: &[(u32, Vec<SvcBrief>)]) {
    let one = match crate::workspace::canvas::single_item(n) {
        Some(crate::doc::edits::Item::Device(d)) => Some(d),
        _ => None,
    };
    if one == n.metrics.canvas_dev {
        return;
    }
    n.metrics.canvas_dev = one;
    let Some(d) = one else { return };
    let mine = n.metrics.sel.as_ref().is_some_and(|s| s.0 == d);
    if let (false, Some((_, v))) = (mine, devs.iter().find(|x| x.0 == d)) {
        if let Some(s) = v.first() {
            n.metrics.sel = Some((d, s.name.clone()));
        }
    }
}

// the tree of the devices and their services
fn tree(n: &mut Net, ui: &mut Ui, devs: &[(u32, Vec<SvcBrief>)]) {
    // new devices start open
    for (d, _) in devs {
        if n.metrics.seen.insert(*d) {
            n.metrics.tree.expanded.insert(dev_key(*d));
        }
    }
    // the rows: key, depth, children, label, detail, device, the service's index
    let off = unpowered(n);
    let mut items: Vec<(u64, u16, bool, String, String, u32, usize)> = Vec::new();
    for (d, v) in devs {
        let name = if *d == CABLES { String::from(t("Kabel")) } else { n.doc.proj.device(*d).map_or_else(|| format!("{}", d), |x| x.name.clone()) };
        // (its hardware is no service)
        let k = v.iter().filter(|s| s.kind != "host").count();
        let count = if *d == CABLES {
            n.doc.proj.links.len().to_string()
        } else if k == 1 {
            String::from(t("1 Dienst"))
        } else {
            tf("{} Dienste", &[&k.to_string()])
        };
        items.push((dev_key(*d), 0, true, name, count, *d, 0));
        if n.metrics.tree.expanded.contains(&dev_key(*d)) {
            for (k, s) in v.iter().enumerate() {
                let state = if s.kind == "cables" { "" } else { svc_state(&off, s).0 };
                items.push((svc_key(*d, k), 1, false, svc_label(s.kind, &s.name), String::from(state), *d, k));
            }
        }
        if items.len() >= ROWS_MAX {
            break;
        }
    }
    items.truncate(ROWS_MAX);
    // a service chosen elsewhere (the canvas, the start): the tree's cursor goes to it (its
    // device's row while the device is closed); the tree's own moves stay where they are
    let m = &mut n.metrics;
    if m.sel != m.cur_for {
        if let Some((d, s)) = &m.sel {
            let k = devs.iter().find(|x| x.0 == *d).and_then(|x| x.1.iter().position(|y| &y.name == s)).unwrap_or(0);
            let key = svc_key(*d, k);
            m.tree.cur = Some(if items.iter().any(|x| x.0 == key) { key } else { dev_key(*d) });
        }
        m.cur_for = m.sel.clone();
    }
    let rows: Vec<TreeRow> = items.iter().map(|x| TreeRow { key: x.0, depth: x.1, has_children: x.2, check: None, icon: 0, label: &x.3, detail: Some(&x.4) }).collect();
    ui.next_size(-1, -1);
    let ev = ui.tree(ID_MET_TREE, &mut m.tree, &rows, 0);
    if ev.sel_changed {
        // a device's row chooses its first service
        if let Some(x) = m.tree.cur.and_then(|c| items.iter().find(|x| x.0 == c)) {
            if let Some(s) = devs.iter().find(|v| v.0 == x.5).and_then(|v| v.1.get(x.6)) {
                m.sel = Some((x.5, s.name.clone()));
                m.cur_for = m.sel.clone();
            }
        }
        ui.repass();
    }
}

// a cheap key of the faults (what the charts' marks depend on)
fn faults_key(n: &Net) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for m in n.doc.sim.engine().faults() {
        for v in [m.id as u64, m.at, m.healed.unwrap_or(u64::MAX), m.until.unwrap_or(u64::MAX)] {
            h = (h ^ v).wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    h
}

// the moment the charts end at in the simulation mode (None: real time, the newest samples)
fn shown_at(n: &Net) -> Option<Ns> {
    if n.sim.sim_mode { Some(n.doc.sim.view_time()) } else { None }
}

// what the charts and tiles of the chosen service show, read again when it changed
fn refresh(n: &mut Net) {
    let Some((dev, svc)) = n.metrics.sel.clone() else {
        n.metrics.data = None;
        return;
    };
    let hide_faults = crate::workspace::task::lock(n).faults;
    let eng = n.doc.sim.engine();
    let at = shown_at(n);
    let key = (dev, svc.clone(), eng.series_gen(), at, n.metrics.span, faults_key(n), crate::app::tr::en());
    if n.metrics.data.as_ref().is_some_and(|d| d.key == key) {
        return;
    }
    let span = SPANS[n.metrics.span.clamp(0, SPANS.len() as i32 - 1) as usize].0 * SEC;
    let mut series = Vec::new();
    for v in eng.series(dev, &svc) {
        let lat = matches!(v.kind, jfn_sim::SeriesKind::Lat(_)) && v.name.ends_with("_us");
        let lat = lat || matches!(v.name, "disk_wait_us" | "disk_backlog_us");
        let mut b = SeriesBuf::new(v.points.len().max(1));
        for &(t, x) in &v.points {
            // (cpu_pm and disk_pm read as percent)
            b.push(t as f64, if lat { x as f64 * 1e-6 } else if matches!(v.name, "cpu_pm" | "disk_pm") { x as f64 / 10.0 } else { x as f64 });
        }
        series.push((v.name, b));
    }
    let (kpi, share) = match at {
        Some(t) => {
            let from = t.saturating_sub(KPI_SPAN);
            (eng.kpi_between(dev, &svc, from, t), eng.shard_load_between(from, t).iter().find(|s| s.dev == dev && s.svc == svc).map(|s| s.share_pm))
        }
        None => (eng.kpi(dev, &svc, KPI_SPAN), eng.shard_load(KPI_SPAN).iter().find(|s| s.dev == dev && s.svc == svc).map(|s| s.share_pm)),
    };
    // the view's end: the moment shown, else the newest sample
    let end = at.unwrap_or_else(|| eng.series_times().last().copied().unwrap_or(eng.now()));
    let from = end.saturating_sub(span);
    let marks = eng
        .chart_marks(from, end)
        .iter()
        .filter(|_| !hide_faults)
        .map(|m| {
            let t1 = m.healed.unwrap_or(end).min(end.max(m.at));
            (m.at as f64, t1 as f64, tf("Fehler {}: {}", &[&m.id.to_string(), &crate::app::tr::msg(&m.msg)]))
        })
        .collect();
    // the bars of the latencies over the span of the charts; the cables
    let bars = eng.lat_hist_between(dev, &svc, from, end).map_or(Vec::new(), |h| rebin(&h.buckets()));
    let mut cables = Vec::new();
    if dev == CABLES {
        for l in &n.doc.proj.links {
            let util = eng.cable_util(l.id);
            let Some(rt) = eng.link_rt(l.id) else { continue };
            let lately: Vec<&(Ns, i64, i64)> = util.iter().filter(|u| u.0 > end.saturating_sub(KPI_SPAN) && u.0 <= end).collect();
            let n_ = lately.len().max(1) as i64;
            let (ab, ba) = (lately.iter().map(|u| u.1).sum::<i64>() / n_, lately.iter().map(|u| u.2).sum::<i64>() / n_);
            let mut buf = SeriesBuf::new(util.len().max(1));
            for u in &util {
                buf.push(u.0 as f64, u.1.max(u.2) as f64 / 1000.0);
            }
            cables.push(CableRow { name: crate::workspace::canvas::cables::ends_text(&n.doc.proj, l), kbps: rt.kbps, ab, ba, buf });
        }
        cables.sort_by_key(|c| core::cmp::Reverse(c.ab.max(c.ba)));
    }
    n.metrics.data = Some(Data { key, bars, cables, series, kpi, share, marks });
}

// the tiles of a device's hardware at moment `to`: its CPU over the last seconds, how many wait, its
// memory
fn host_tiles(e: &jfn_sim::Engine, dev: u32, to: Ns, sep: u32) -> Vec<(String, String)> {
    let from = to.saturating_sub(KPI_SPAN);
    let val = |name: &str| e.series_value(dev, jfn_sim::res::HOST, name, jfn_sim::series::Agg::Natural, from, to).ok().map(|v| v.milli as f64 / 1000.0);
    let num = |v: f64| LcUnit::Num("").text(v, sep);
    let mut out = Vec::new();
    if let Some(v) = val("cpu_pm") {
        out.push((String::from(t("CPU")), LcUnit::Num("%").text(v / 10.0, sep)));
    }
    if let Some(v) = val("runq") {
        out.push((String::from(t("Wartend")), num(v)));
    }
    if let Some(v) = val("mem_bytes") {
        out.push((String::from(t("Im RAM")), jfn_sim::disk::size_text(v as u64)));
    }
    if let Some(v) = val("swap_bytes").filter(|&v| v > 0.0) {
        out.push((String::from(t("Im Swap")), jfn_sim::disk::size_text(v as u64)));
    }
    if let Some(v) = val("disk_pm") {
        out.push((String::from(t("Disk belegt")), LcUnit::Num("%").text(v / 10.0, sep)));
    }
    if let Some(v) = val("disk_backlog_us").filter(|&v| v > 0.0) {
        out.push((String::from(t("Rückstand")), LcUnit::Secs.text(v * 1e-6, sep)));
    }
    if let Some(r) = e.res_of(dev) {
        out.push((String::from(t("Kerne")), num(r.hw.cores as f64)));
    }
    out
}

// the chosen service: head, tiles, charts
fn detail(n: &mut Net, ui: &mut Ui, devs: &[(u32, Vec<SvcBrief>)]) {
    refresh(n);
    let Some((dev, svc)) = n.metrics.sel.clone() else { return };
    let Some(brief) = devs.iter().find(|x| x.0 == dev).and_then(|x| x.1.iter().find(|s| s.name == svc)).cloned() else { return };
    if brief.kind == "cables" {
        cables(n, ui);
        return;
    }
    let sep = ui.decimal_sep();
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_MET_SCROLL, SC_V | pad(8));
    ui.box_begin(0, BOX_V, 8);
    // the head: the service, its state, the span of the charts
    let (state, badge) = svc_state(&unpowered(n), &brief);
    let dname = n.doc.proj.device(dev).map_or_else(|| format!("{}", dev), |d| d.name.clone());
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.status_dot(badge);
    ui.text(TS_BODY_STRONG, &format!("{} · {}", dname, svc_label(brief.kind, &brief.name)));
    ui.text(TS_BODY | TX_SECONDARY, state);
    ui.spacer(-1);
    ui.text(TS_CAPTION | TX_SECONDARY, t("Zeitraum"));
    let labels: Vec<&str> = SPANS.iter().map(|s| t(s.1)).collect();
    let mut span = n.metrics.span;
    ui.next_flags(ST_COMPACT);
    if ui.segmented(ID_MET_SPAN, &mut span, &labels) & EV_ACT != 0 {
        n.metrics.span = span.clamp(0, SPANS.len() as i32 - 1);
        ui.repass();
    }
    ui.box_end();
    // its status line; the span of the tiles
    let status = n.doc.sim.state().services(dev).into_iter().find(|s| s.name == svc).map(|s| s.status_msg).or_else(|| n.doc.sim.state().res_of(dev).filter(|_| brief.kind == "host").map(jfn_sim::res::status_msg));
    ui.box_begin(0, BOX_H, 8);
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &status.filter(|m| !m.is_empty()).map(|m| crate::app::tr::msg(&m)).unwrap_or_default());
    ui.text(TS_CAPTION | TX_TERTIARY, &tf("Kacheln: die letzten {} s", &[&(KPI_SPAN / SEC).to_string()]));
    ui.box_end();
    let w = ui.px2dip(ui.box_rect()[2]).max(TILE_W);
    let Some(data) = n.metrics.data.as_ref() else {
        ui.box_end();
        ui.scroll_end();
        return;
    };
    // the tiles over the last seconds
    let tiles = if brief.kind == "host" { host_tiles(n.doc.sim.state(), dev, shown_at(n).unwrap_or_else(|| n.doc.sim.state().series_times().last().copied().unwrap_or(0)), sep) } else { data.kpi.as_ref().map(|k| kpi_tiles(k, data.share, sep)).unwrap_or_default() };
    if tiles.is_empty() {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Noch keine Messwerte: der Simulator nimmt jede Sekunde einen Punkt jeder Zahl."));
    }
    // rows of tiles
    let per = (w / (TILE_W + 8)).max(1) as usize;
    for chunk in tiles.chunks(per) {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        for (label, value) in chunk {
            ui.next_size(TILE_W, 0);
            ui.box_begin(0, BOX_V | BOX_CARD | pad(6), 0);
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, label);
            ui.text(TS_BODY_STRONG, value);
            ui.box_end();
        }
        ui.box_end();
    }
    // the disk of services that store
    if matches!(brief.kind, "kv" | "db" | "mq" | "script") {
        if let Some(d) = n.doc.sim.state().disk_of(dev) {
            ui.text(TS_CAPTION | TX_SECONDARY, &tf("Disk: {}", &[&crate::app::tr::msg(&d.msg())]));
        }
    }
    // the charts: those groups the service has series of
    let groups = groups(brief.kind);
    let shown: Vec<(&Group, Vec<usize>)> = groups
        .iter()
        .filter_map(|g| {
            let idx: Vec<usize> = g.names.iter().filter_map(|nm| data.series.iter().position(|s| s.0 == *nm)).collect();
            (!idx.is_empty()).then_some((g, idx))
        })
        .collect();
    let cols = (w / CHART_MIN_W).clamp(1, 3);
    let cw = ((w - 12 * (cols - 1)) / cols).max(160);
    let span_ns = (SPANS[n.metrics.span.clamp(0, SPANS.len() as i32 - 1) as usize].0 * SEC) as f64;
    let crit = ui.color(T_CRIT);
    let marks: Vec<LcMarker> = data.marks.iter().map(|(a, b, l)| LcMarker::range(*a, *b, l).color(crit)).collect();
    let at = shown_at(n);
    for (r, row) in shown.chunks(cols as usize).enumerate() {
        ui.box_begin(0, BOX_H, 12);
        for (j, (g, idx)) in row.iter().enumerate() {
            let names: Vec<String> = idx.iter().map(|&i| series_label(data.series[i].0)).collect();
            let series: Vec<LcSeries> = idx.iter().zip(names.iter()).map(|(&i, l)| LcSeries::new(l, &data.series[i].1).unit(g.show.unit())).collect();
            let title = tc(g.title);
            let mut chart = LineChart::new(&series).x_axis(LX_NANOS).name(title).y_range(0.0, f64::NAN).markers(&marks);
            chart = match at {
                Some(e) => chart.x_range((e as f64 - span_ns).max(0.0), (e as f64).max(1.0)),
                None => chart.follow(span_ns),
            };
            ui.next_size(cw, 0);
            ui.box_begin(0, BOX_V, 4);
            ui.text(TS_BODY_STRONG, title);
            ui.next_size(-1, CHART_H);
            let _ = ui.line_chart(ID_MET_CHART0 + (r * cols as usize + j) as u32, &chart);
            ui.box_end();
        }
        ui.box_end();
    }
    // the bars of the latencies
    if !data.bars.is_empty() {
        let labels: Vec<String> = data.bars.iter().map(|b| tf("bis {}", &[&LcUnit::Secs.text(b.0 as f64 * 1e-9, sep)])).collect();
        let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        let counts: Vec<f32> = data.bars.iter().map(|b| b.1 as f32).collect();
        ui.text(TS_BODY_STRONG, t("Verteilung der Latenz"));
        ui.next_size(-1, CHART_H);
        let _ = ui.bar_chart(ID_MET_BARS, &refs, &[BarSeries { label: t("Antworten"), values: &counts, color: 0 }], false);
    }
    ui.box_end();
    ui.scroll_end();
}

// the row Alle Kabel: the busiest first, a chart of the busiest
fn cables(n: &mut Net, ui: &mut Ui) {
    let sep = ui.decimal_sep();
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_MET_SCROLL, SC_V | pad(8));
    ui.box_begin(0, BOX_V, 8);
    ui.text(TS_BODY_STRONG, t("Auslastung der Kabel"));
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &tf("Was jedes Kabel in den letzten {} s von seiner Bitrate trug, je Richtung; die Rahmen ohne Präambel und Lücke.", &[&(KPI_SPAN / SEC).to_string()]));
    let Some(data) = n.metrics.data.as_ref() else {
        ui.box_end();
        ui.scroll_end();
        return;
    };
    let pct = |x: i64| LcUnit::Num("%").text(x as f64 / 1000.0, sep);
    for c in data.cables.iter().take(CABLE_ROWS) {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &c.name);
        ui.next_size(90, 0);
        ui.text(TS_CAPTION | TX_SECONDARY, &LcUnit::Si("bit/s").text(c.kbps as f64 * 1000.0, sep));
        ui.next_size(160, 0);
        ui.text(TS_BODY, &tf("{} / {}", &[&pct(c.ab), &pct(c.ba)]));
        ui.box_end();
    }
    if data.cables.len() > CABLE_ROWS {
        ui.text(TS_CAPTION | TX_TERTIARY, &tf("… und {} weitere", &[&(data.cables.len() - CABLE_ROWS).to_string()]));
    }
    let busiest: Vec<&CableRow> = data.cables.iter().take(CABLE_CURVES).collect();
    let series: Vec<LcSeries> = busiest.iter().map(|c| LcSeries::new(&c.name, &c.buf).unit(LcUnit::Num("%"))).collect();
    let span_ns = (SPANS[n.metrics.span.clamp(0, SPANS.len() as i32 - 1) as usize].0 * SEC) as f64;
    let chart = LineChart::new(&series).x_axis(LX_NANOS).name(t("Auslastung der Kabel")).y_range(0.0, f64::NAN).follow(span_ns);
    ui.next_size(-1, CHART_H);
    let _ = ui.line_chart(ID_MET_CABLES, &chart);
    ui.box_end();
    ui.scroll_end();
}
