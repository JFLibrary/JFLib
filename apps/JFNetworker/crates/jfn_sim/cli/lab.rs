// lab -- the console commands that show the lab's services (our own tools, our own words):
//
//   lbctl [status] [-s NAME]                 a load balancer: settings, numbers, every backend
//   kvctl [status|ring|where KEY|get KEY|keys] [-s NAME]
//                                            a KV node: members as it sees them, the ring, the
//                                            replicas of a key and what each one holds, its
//                                            own entries
//   cachectl [status|keys] [-s NAME]         a cache: hits, misses, its entries
//   mqctl [status] [-s NAME]                 a queue: its topics
//   workerctl [status] [-s NAME]             a worker: its slots
//
// Without -s a command takes the first service of its kind on the device.  The texts are
// German like the rest of the lab; times are virtual ("seit 6,512 s").

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::hashring::hash64;

use crate::cache::Cache;
use crate::disk::size_text;
use crate::kernel::{Engine, Ns};
use crate::kv::KvNode;
use crate::lb::{Admin, CheckKind, Lb, LbMode};
use crate::mq::{Queue, SlotState, Worker};
use crate::os::Service;
use crate::trace::fmt_ns;

/// columns: every one as wide as its widest cell (+ 2), the last one not padded
pub(crate) fn table(rows: &[Vec<String>]) -> String {
    let n = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut w = alloc::vec![0usize; n];
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            w[i] = w[i].max(c.chars().count());
        }
    }
    let mut s = String::new();
    for r in rows {
        let mut line = String::new();
        for (i, c) in r.iter().enumerate() {
            line.push_str(c);
            if i + 1 < r.len() {
                for _ in c.chars().count()..w[i] + 2 {
                    line.push(' ');
                }
            }
        }
        s.push_str(line.trim_end());
        s.push('\n');
    }
    s
}

pub(crate) fn row(cells: &[&str]) -> Vec<String> {
    cells.iter().map(|c| String::from(*c)).collect()
}

fn opt_ns(v: Option<Ns>) -> String {
    match v {
        Some(x) => fmt_ns(x as i64),
        None => String::from("-"),
    }
}

/// "80,0 %" from per mille
fn pm(v: u64) -> String {
    format!("{},{} %", v / 10, v % 10)
}

/// "33,4 %" from parts per million
fn ppm(v: u32) -> String {
    format!("{},{} %", v / 10_000, v % 10_000 / 1000)
}

const LB_HELP: &str = "lbctl [status] [-s DIENST]   zeigt den Loadbalancer: Einstellungen, Zahlen und jedes Backend\n";
const KV_HELP: &str = "kvctl [status] [-s DIENST]   zeigt den KV-Knoten: Mitglieder, wie er sie sieht, und seine Zahlen\nkvctl ring                   die Tokens des Rings und ihre Knoten\nkvctl where SCHLÜSSEL        die Replikate eines Schlüssels und was jedes davon hält\nkvctl get SCHLÜSSEL          der Eintrag dieses Knotens\nkvctl keys                   die Schlüssel dieses Knotens\n";
const CACHE_HELP: &str = "cachectl [status|keys] [-s DIENST]   zeigt den Cache: Treffer, Fehlschläge, Einträge\n";
const MQ_HELP: &str = "mqctl [status] [-s DIENST]   zeigt die Queue mit ihren Topics\n";
const WORKER_HELP: &str = "workerctl [status] [-s DIENST]   zeigt den Worker und seine Plätze\n";

impl Engine {
    fn lab_err(&mut self, ni: usize, text: &str) -> Result<(), String> {
        self.out(ni, text);
        Err(String::from(text.lines().next().unwrap_or("")))
    }

    /// "-s NAME" and the other words of a command; Err: the words were not understood
    fn lab_args(args: &[&str]) -> Result<(Option<String>, Vec<String>), ()> {
        let mut svc = None;
        let mut rest = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-s" | "--service" => {
                    i += 1;
                    svc = Some(String::from(*args.get(i).ok_or(())?));
                }
                "-h" | "--help" | "help" => return Err(()),
                a => rest.push(String::from(a)),
            }
            i += 1;
        }
        Ok((svc, rest))
    }

    /// the name of the service of type T a command shows
    fn lab_pick<T: Service + 'static>(&self, ni: usize, name: Option<&str>) -> Option<String> {
        let dev = self.dev_of(ni);
        match name {
            Some(n) => self.service::<T>(dev, n).map(|_| String::from(n)),
            None => self.service_ids(dev).into_iter().map(|s| s.1).find(|n| self.service::<T>(dev, n).is_some()),
        }
    }

    // ---------------------------------------------------------------- lbctl

    pub(crate) fn sh_lbctl(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let Ok((svc, rest)) = Engine::lab_args(args) else {
            self.out(ni, LB_HELP);
            return Ok(());
        };
        if rest.iter().any(|w| w != "status") {
            return self.lab_err(ni, &format!("lbctl: {} kenne ich nicht\n{}", rest.join(" "), LB_HELP));
        }
        let Some(name) = self.lab_pick::<Lb>(ni, svc.as_deref()) else {
            return self.lab_err(ni, "lbctl: auf diesem Gerät läuft kein Loadbalancer\n");
        };
        let dev = self.dev_of(ni);
        let Some(v) = self.lb_view(dev, &name) else {
            return Ok(());
        };
        let lb = self.service::<Lb>(dev, &name);
        let health = match lb.map(|l| l.health.clone()) {
            Some(h) if h.kind != CheckKind::None => format!("Health Checks {} alle {} (Timeout {}, verfügbar nach {}, ausgefallen nach {} Prüfungen)", if h.kind == CheckKind::Http { format!("http GET {}", h.path) } else { String::from("tcp") }, fmt_ns(h.interval as i64), fmt_ns(h.timeout as i64), h.rise, h.fall),
            _ => String::from("ohne Health Checks"),
        };
        let mut s = format!("Loadbalancer {}: {} auf Port {}{}, Verfahren {}, {}\n", name, if v.mode == LbMode::Http { "HTTP" } else { "TCP" }, v.port, if v.listening { "" } else { " (nicht offen)" }, v.algo.name(), health);
        let what = if v.mode == LbMode::Http { "Anfragen" } else { "Verbindungen" };
        s.push_str(&format!("{} {}, beendet {}, offen {}, Fehler {} (ohne Backend {}), Wiederholungen {}\n", what, v.requests, v.responses, v.active, v.errors, v.no_backend, v.retried));
        s.push_str(&format!("{} p50 {}, p95 {}, p99 {}\n\n", if v.mode == LbMode::Http { "Antwortzeit" } else { "Verbindungsaufbau" }, opt_ns(v.p50), opt_ns(v.p95), opt_ns(v.p99)));
        let mut rows = alloc::vec![row(&["Backend", "Adresse", "Gewicht", "Zustand", "seit", "offen", "gesamt", "Fehler", "p99"])];
        for b in &v.backends {
            let state = match b.admin {
                Admin::Active if b.up => "verfügbar",
                Admin::Active => "ausgefallen",
                Admin::Draining { .. } => "leert sich",
                Admin::Disabled => "abgeschaltet",
                Admin::Removed => "entfernt",
            };
            let at = if b.addr.is_v4() { format!("{}:{}", b.addr, b.port) } else { format!("[{}]:{}", b.addr, b.port) };
            rows.push(alloc::vec![b.name.clone(), at, format!("{}", b.weight), String::from(state), fmt_ns(b.since as i64), format!("{}", b.active), format!("{}", b.total), format!("{}", b.errors), opt_ns(b.p99)]);
        }
        s.push_str(&table(&rows));
        self.out(ni, &s);
        Ok(())
    }

    // ---------------------------------------------------------------- kvctl

    pub(crate) fn sh_kvctl(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let Ok((svc, rest)) = Engine::lab_args(args) else {
            self.out(ni, KV_HELP);
            return Ok(());
        };
        let Some(name) = self.lab_pick::<KvNode>(ni, svc.as_deref()) else {
            return self.lab_err(ni, "kvctl: auf diesem Gerät läuft kein KV-Knoten\n");
        };
        let dev = self.dev_of(ni);
        let words: Vec<&str> = rest.iter().map(String::as_str).collect();
        let text = match words.as_slice() {
            [] | ["status"] => self.kv_status(dev, &name),
            ["ring"] => {
                let mut rows = alloc::vec![row(&["Token", "Knoten"])];
                for (t, m) in self.kv_ring(dev, &name) {
                    rows.push(alloc::vec![format!("0x{:016x}", t), format!("{}", m)]);
                }
                table(&rows)
            }
            ["where", key] => self.kv_where_text(dev, &name, key),
            ["get", key] => match self.service::<KvNode>(dev, &name).and_then(|k| k.entry(key)) {
                Some(e) if e.deleted => format!("{}: gelöscht (Version {})\n", key, e.ver.text()),
                Some(e) => format!("{} = {} (Version {})\n", key, String::from_utf8_lossy(&e.value), e.ver.text()),
                None => format!("{}: nicht auf diesem Knoten\n", key),
            },
            ["keys"] => {
                let mut rows = alloc::vec![row(&["Schlüssel", "Version", "Bytes"])];
                let mut more = 0usize;
                if let Some(k) = self.service::<KvNode>(dev, &name) {
                    for (i, (key, e)) in k.stored().enumerate() {
                        if i >= 100 {
                            more += 1;
                            continue;
                        }
                        rows.push(alloc::vec![key.clone(), format!("{}{}", e.ver.text(), if e.deleted { " gelöscht" } else { "" }), format!("{}", e.value.len())]);
                    }
                }
                let mut t = table(&rows);
                if more > 0 {
                    t.push_str(&format!("... und {} weitere\n", more));
                }
                t
            }
            _ => return self.lab_err(ni, &format!("kvctl: {} kenne ich nicht\n{}", rest.join(" "), KV_HELP)),
        };
        self.out(ni, &text);
        Ok(())
    }

    fn kv_status(&self, dev: u32, name: &str) -> String {
        let (Some(v), Some(k)) = (self.kv_view(dev, name), self.service::<KvNode>(dev, name)) else {
            return String::new();
        };
        let me = match v.me {
            Some(a) => format!("{}", a),
            None => String::from("nur Koordinator"),
        };
        let mut s = format!("KV-Knoten {}: {}, Port {}, Heartbeats UDP {} alle {} (ausgefallen nach {})\n", name, me, v.port, k.hb_port, fmt_ns(k.beat as i64), fmt_ns(k.down_after as i64));
        let strong = (v.w + v.r) as usize > v.rf;
        s.push_str(&format!("Ring: {} Knoten mit je {} vnodes, RF {}, Konsistenz {} (W {}, R {}{}), Read Repair {}\n\n", v.members.len(), v.vnodes, v.rf, v.consistency.name(), v.w, v.r, if strong { ", W + R > RF" } else { ", W + R \u{2264} RF" }, if v.read_repair { "an" } else { "aus" }));
        let mut rows = alloc::vec![row(&["Knoten", "Zustand", "seit", "Ring-Anteil"])];
        for m in &v.members {
            let share = v.shares.iter().find(|x| x.0 == m.addr).map(|x| ppm(x.1)).unwrap_or_default();
            let (state, since) = if Some(m.addr) == v.me {
                ("dieser Knoten", String::from("-"))
            } else if m.up {
                ("erreichbar", fmt_ns(m.since as i64))
            } else {
                ("ausgefallen", fmt_ns(m.since as i64))
            };
            rows.push(alloc::vec![format!("{}", m.addr), String::from(state), since, share]);
        }
        s.push_str(&table(&rows));
        let disk = self.node_index(dev).and_then(|ni| self.os(ni)).map(|o| o.disk.text()).unwrap_or_default();
        let st = v.stats;
        s.push_str(&format!("\nSchlüssel {} ({}), Disk {}\n", v.keys, size_text(v.bytes), disk));
        s.push_str(&format!("Anfragen {}: gelesen {}, geschrieben {}, nicht gefunden {}, nicht verfügbar {}, Disk voll {}\n", st.requests, st.reads, st.writes, st.not_found, st.unavailable, st.full));
        s.push_str(&format!("Weitergereicht {}, selbst geführt {}; Replikate geschrieben {}, gelesen {}\n", st.forwards, st.led, st.replica_writes, st.replica_reads));
        s.push_str(&format!("Veraltete Antworten {}, Read Repair gesendet {}, erhalten {}; Ausfälle erkannt {}\n", st.stale_reads, st.read_repairs, st.repaired, st.member_downs));
        s.push_str(&format!("Antwortzeit p50 {}, p95 {}, p99 {}\n", opt_ns(v.p50), opt_ns(v.p95), opt_ns(v.p99)));
        s
    }

    fn kv_where_text(&self, dev: u32, name: &str, key: &str) -> String {
        let reps = self.kv_where(dev, name, key);
        let rf = self.service::<KvNode>(dev, name).map(|k| k.rf).unwrap_or(0);
        let mut s = format!("Schlüssel {}: Hash 0x{:016x}, Replikate (RF {}):\n", key, hash64(key.as_bytes()), rf);
        let mut rows = Vec::new();
        for r in reps {
            let held = match &r.entry {
                Some(e) if e.deleted => format!("gelöscht, Version {}", e.ver.text()),
                Some(e) => format!("Version {}, {} Bytes", e.ver.text(), e.value.len()),
                None if r.dev.is_none() => String::from("kein KV-Knoten unter der Adresse"),
                None => String::from("nichts gespeichert"),
            };
            rows.push(alloc::vec![format!("  {}", r.addr), String::from(if r.leader { "Leader" } else { "Follower" }), String::from(if r.up { "erreichbar" } else { "ausgefallen" }), held]);
        }
        s.push_str(&table(&rows));
        s
    }

    // ---------------------------------------------------------------- cachectl

    pub(crate) fn sh_cachectl(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let Ok((svc, rest)) = Engine::lab_args(args) else {
            self.out(ni, CACHE_HELP);
            return Ok(());
        };
        let Some(name) = self.lab_pick::<Cache>(ni, svc.as_deref()) else {
            return self.lab_err(ni, "cachectl: auf diesem Gerät läuft kein Cache\n");
        };
        let dev = self.dev_of(ni);
        let now = self.now;
        let words: Vec<&str> = rest.iter().map(String::as_str).collect();
        let text = match words.as_slice() {
            [] | ["status"] => {
                let Some(v) = self.cache_view(dev, &name) else {
                    return Ok(());
                };
                let mut s = format!("Cache {}: Port {} vor {}, Lebensdauer {}, höchstens {} Einträge und {}\n", name, v.port, v.backend, fmt_ns(v.ttl as i64), v.max_entries, size_text(v.max_bytes));
                s.push_str(&format!("Einträge {} ({}), Anfragen {}: Treffer {}, Fehlschläge {} ({} Treffer)\n", v.entries, size_text(v.bytes), v.requests, v.hits, v.misses, pm(v.hit_pm)));
                s.push_str(&format!("Zusammengelegt {}, abgelaufen {}, verdrängt {}, ungültig gemacht {}\n", v.collapsed, v.expired, v.evictions, v.invalidations));
                s.push_str(&format!("Backend: {} Anfragen, {} Fehler; Antwortzeit p50 {} (Treffer {}, Fehlschlag {}), p99 {}\n", v.backend_requests, v.backend_errors, opt_ns(v.p50), opt_ns(v.hit_p50), opt_ns(v.miss_p50), opt_ns(v.p99)));
                s
            }
            ["keys"] => {
                let mut rows = alloc::vec![row(&["Ziel", "Status", "Bytes", "Alter", "frisch bis"])];
                if let Some(c) = self.service::<Cache>(dev, &name) {
                    for k in c.keys().into_iter().rev().take(100) {
                        if let Some(e) = c.peek(&k) {
                            rows.push(alloc::vec![k.clone(), format!("{}", e.status), format!("{}", e.body.len()), fmt_ns(now.saturating_sub(e.stored) as i64), fmt_ns(e.expires as i64)]);
                        }
                    }
                }
                table(&rows)
            }
            _ => return self.lab_err(ni, &format!("cachectl: {} kenne ich nicht\n{}", rest.join(" "), CACHE_HELP)),
        };
        self.out(ni, &text);
        Ok(())
    }

    // ---------------------------------------------------------------- mqctl, workerctl

    pub(crate) fn sh_mqctl(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let Ok((svc, rest)) = Engine::lab_args(args) else {
            self.out(ni, MQ_HELP);
            return Ok(());
        };
        if rest.iter().any(|w| w != "status") {
            return self.lab_err(ni, &format!("mqctl: {} kenne ich nicht\n{}", rest.join(" "), MQ_HELP));
        }
        let Some(name) = self.lab_pick::<Queue>(ni, svc.as_deref()) else {
            return self.lab_err(ni, "mqctl: auf diesem Gerät läuft keine Queue\n");
        };
        let dev = self.dev_of(ni);
        let Some(v) = self.mq_view(dev, &name) else {
            return Ok(());
        };
        let mut s = format!("Queue {}: Port {}, Sichtbarkeit {}, höchstens {} Zustellungen, {}\n", name, v.port, fmt_ns(v.visibility as i64), v.max_deliveries, if v.durable { "auf der Disk" } else { "nur im Speicher" });
        s.push_str(&format!("Angenommen bis bestätigt: p50 {}, p99 {}\n\n", opt_ns(v.p50), opt_ns(v.p99)));
        let mut rows = alloc::vec![row(&["Topic", "wartend", "unterwegs", "angenommen", "ausgeliefert", "erneut", "bestätigt", "zurück", "aussortiert", "älteste"])];
        for t in &v.topics {
            rows.push(alloc::vec![t.name.clone(), format!("{}", t.ready), format!("{}", t.inflight), format!("{}", t.published), format!("{}", t.delivered), format!("{}", t.redelivered), format!("{}", t.acked), format!("{}", t.nacked), format!("{}", t.dead), opt_ns(t.oldest)]);
        }
        s.push_str(&table(&rows));
        self.out(ni, &s);
        Ok(())
    }

    pub(crate) fn sh_workerctl(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let Ok((svc, rest)) = Engine::lab_args(args) else {
            self.out(ni, WORKER_HELP);
            return Ok(());
        };
        if rest.iter().any(|w| w != "status") {
            return self.lab_err(ni, &format!("workerctl: {} kenne ich nicht\n{}", rest.join(" "), WORKER_HELP));
        }
        let Some(name) = self.lab_pick::<Worker>(ni, svc.as_deref()) else {
            return self.lab_err(ni, "workerctl: auf diesem Gerät läuft kein Worker\n");
        };
        let dev = self.dev_of(ni);
        let (Some(v), Some(w)) = (self.worker_view(dev, &name), self.service::<Worker>(dev, &name)) else {
            return Ok(());
        };
        let mut s = format!("Worker {}: {} Topic {}, {} Plätze, {} je Nachricht, Fehlerquote {}\n", name, v.queue, v.topic, v.concurrency, fmt_ns(w.time as i64), crate::fault::ppm_text(w.error_ppm));
        s.push_str(&format!("Genommen {}, erledigt {}, zurückgegeben {}, Abfragen {} (leer {}), Fehler {}; Arbeit p50 {}, p99 {}\n\n", v.taken, v.done, v.failed, v.polls, v.empty, v.errors, opt_ns(v.p50), opt_ns(v.p99)));
        let mut rows = alloc::vec![row(&["Platz", "Zustand"])];
        for (i, st) in v.slots.iter().enumerate() {
            let t = match st {
                SlotState::Idle => String::from("pausiert"),
                SlotState::Asking => String::from("wartet auf eine Nachricht"),
                SlotState::Working { id, until } => format!("arbeitet an Nachricht {} bis {}", id, fmt_ns(*until as i64)),
                SlotState::Acking { id } => format!("meldet Nachricht {} zurück", id),
            };
            rows.push(alloc::vec![format!("{}", i + 1), t]);
        }
        s.push_str(&table(&rows));
        self.out(ni, &s);
        Ok(())
    }
}
