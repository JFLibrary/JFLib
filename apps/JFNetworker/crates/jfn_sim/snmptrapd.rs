// snmptrapd -- the receiver of SNMP notifications on Linux hosts (as net-snmp's snmptrapd):
// "snmptrapd" on UDP port 162 takes traps of version 1, SNMPv2-Traps and InformRequests,
// acknowledges each inform with its Response, and logs every notification
//
//   config (JSON, every member may be left out)
//     {"port": 162, "communities": ["public"], "log": "/var/log/snmptrapd.log", "keep": 100}
//   TrapReceiver::from_config(&config) / TrapReceiver::traps() / Engine::snmp_traps(dev)
//
// A notification whose community is not in "communities" is dropped (an empty list takes every
// community, as net-snmp with "disableAuthorization yes").  Each one becomes a log line of the
// device (tag "snmptrapd", the Protokoll panel), lines in the log file (net-snmp's layout: the
// device's clock, the sender, then the bindings as snmptrapd prints them; "" writes no file),
// a note, and a row of traps() (the last `keep` of them) that the status line shows.  An
// InformRequest is answered with a Response of the same request id and bindings from port 162
// to where it came from, also when it came again (the sender did not get the first answer).
//
// Metrics: traps, informs, refused, logged.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value as Json;
use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::mib;
use netpkt::snmp::{self, Body, Kind, Message, Pdu};
use netpkt::IpAddr;

use crate::kernel::{Engine, Ns};
use crate::os::{Dgram, Metric, OsApi, Service, SvcEvent};
use crate::snmptrace::SNote;
use crate::trace::Note;

pub const KIND: &str = "snmptrapd";
const SEV_NOTICE: u8 = 5;

/// a notification the receiver logged
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrapRow {
    /// virtual time it arrived
    pub at: Ns,
    pub from: IpAddr,
    pub version: snmp::Version,
    pub community: String,
    /// an InformRequest (acknowledged)
    pub inform: bool,
    /// the notification as its object's name ("IF-MIB::linkDown"; a v1 trap: its generic name
    /// as SNMPv2 calls it)
    pub what: String,
    /// the bindings as "name = value" (the v1 trap's own, SNMPv2's after snmpTrapOID.0)
    pub vars: Vec<String>,
}

/// the trap receiver
#[derive(Clone, Debug)]
pub struct TrapReceiver {
    pub port: u16,
    /// the communities it takes (empty: all)
    pub communities: Vec<String>,
    /// the log file ("" none)
    pub log: String,
    pub keep: usize,
    rows: VecDeque<TrapRow>,
    pub bound: bool,
    pub traps: u64,
    pub informs: u64,
    pub refused: u64,
    pub logged: u64,
}

impl TrapReceiver {
    pub fn new() -> TrapReceiver {
        TrapReceiver { port: snmp::PORT_TRAP, communities: Vec::new(), log: String::from("/var/log/snmptrapd.log"), keep: 100, rows: VecDeque::new(), bound: true, traps: 0, informs: 0, refused: 0, logged: 0 }
    }

    /// the receiver of a project's definition (svcact.rs)
    pub fn from_config(c: &Json) -> Result<TrapReceiver, Msg> {
        let mut r = TrapReceiver::new();
        if let Some(p) = c.get("port").and_then(|v| v.as_u64()) {
            r.port = u16::try_from(p).ok().filter(|&p| p > 0).ok_or_else(|| msg!("Port {} gibt es nicht.", p))?;
        }
        for x in c.get("communities").and_then(|v| v.as_array()).unwrap_or(&[]) {
            let name = x.as_str().map(str::trim).unwrap_or("");
            if name.is_empty() || name.contains(char::is_whitespace) {
                return Err(msg!("Eine Community ist ein Wort ohne Leerzeichen."));
            }
            r.communities.push(String::from(name));
        }
        if let Some(l) = c.get("log") {
            let l = l.as_str().ok_or_else(|| msg!("„log“ ist der Pfad einer Datei."))?;
            r.log = String::from(l.trim());
        }
        if let Some(k) = c.get("keep").and_then(|v| v.as_u64()) {
            r.keep = k.clamp(1, 10_000) as usize;
        }
        Ok(r)
    }

    /// the notifications it logged, oldest first
    pub fn traps(&self) -> Vec<TrapRow> {
        self.rows.iter().cloned().collect()
    }

    fn take(&mut self, os: &mut OsApi, d: Dgram) {
        let Ok(m) = Message::parse(&d.data) else { return };
        let community = m.community_text();
        if !self.communities.is_empty() && !self.communities.iter().any(|c| *c == community) {
            self.refused += 1;
            os.note(Note::Snmp(SNote::TrapRefused { from: d.src, community }));
            return;
        }
        let (inform, what, vars, head) = match &m.body {
            Body::Trap(t) => {
                let what = mib::name_of(snmp::trap_oid(t.generic, &t.enterprise, t.specific).arcs(), false);
                let vars: Vec<String> = t.varbinds.iter().map(|v| format!("{} = {}", mib::name_of(v.oid.arcs(), false), mib::show(v.oid.arcs(), &v.value, false))).collect();
                let head = format!("TRAP, SNMP v1, community {}\n\t{} {} ({}) Uptime: {}", community, mib::name_of(t.enterprise.arcs(), false), snmp::generic_name(t.generic), t.specific, mib::bare(&snmp::SYS_UPTIME_0, &netpkt::ber::Value::TimeTicks(t.timestamp), false));
                (false, what, vars, head)
            }
            Body::Pdu(p) if matches!(p.kind, Kind::TrapV2 | Kind::Inform) => {
                let what = snmp::notification(&p.varbinds).map(|o| mib::name_of(o.arcs(), false)).unwrap_or_else(|| String::from("?"));
                let vars: Vec<String> = p.varbinds.iter().map(|v| format!("{} = {}", mib::name_of(v.oid.arcs(), false), mib::show(v.oid.arcs(), &v.value, false))).collect();
                (p.kind == Kind::Inform, what, vars, String::new())
            }
            _ => return,
        };
        if let Body::Pdu(p) = &m.body
            && p.kind == Kind::Inform
        {
            // the acknowledgement: the same request id and bindings (RFC 3416 4.2.7)
            let ack = Message { version: m.version, community: m.community.clone(), body: Body::Pdu(Pdu { kind: Kind::Response, request_id: p.request_id, error_status: 0, error_index: 0, varbinds: p.varbinds.clone() }) };
            os.send_from(d.dst, self.port, d.src, d.sport, &ack.to_bytes());
            self.informs += 1;
        } else {
            self.traps += 1;
        }
        let short = what.rsplit("::").next().map(String::from).unwrap_or_else(|| what.clone());
        os.note(Note::Snmp(SNote::Received { from: d.src, what: short.clone(), inform }));
        // the device's log and the Protokoll panel: one line
        let kind = if inform { "inform" } else { "trap" };
        let shown: Vec<&String> = if head.is_empty() { vars.iter().skip(2).collect() } else { vars.iter().collect() };
        let tail = shown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
        let line = if tail.is_empty() { format!("{} {} from {} (v{}, community {})", short, kind, d.src, m.version.word(), community) } else { format!("{} {} from {} (v{}, community {}): {}", short, kind, d.src, m.version.word(), community, tail) };
        os.log(SEV_NOTICE, KIND, &line);
        // the log file in net-snmp's layout
        if !self.log.is_empty() {
            let clock = os.clock().div_euclid(1_000_000_000);
            let c = datetime::Civil::from_unix(clock);
            let stamp = format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", c.year, c.month, c.day, c.hour, c.minute, c.second);
            let mut text = if head.is_empty() {
                format!("{} {} [UDP: [{}]:{}->[{}]:{}]:\n{}\n", stamp, d.src, d.src, d.sport, d.dst, d.dport, vars.join("\t"))
            } else {
                format!("{} {}(via UDP: [{}]:{}->[{}]:{}) {}\n", stamp, d.src, d.src, d.sport, d.dst, d.dport, head)
            };
            if !head.is_empty() && !vars.is_empty() {
                text.push('\t');
                text.push_str(&vars.join("\t"));
                text.push('\n');
            }
            if os.file_append(&self.log, text.as_bytes()).is_ok() {
                self.logged += 1;
            }
        }
        if self.rows.len() >= self.keep {
            self.rows.pop_front();
        }
        let vars = if head.is_empty() { vars.into_iter().skip(2).collect() } else { vars };
        self.rows.push_back(TrapRow { at: os.now(), from: d.src, version: m.version, community, inform, what, vars });
    }
}

impl Default for TrapReceiver {
    fn default() -> TrapReceiver {
        TrapReceiver::new()
    }
}

impl Service for TrapReceiver {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn start(&mut self, os: &mut OsApi) {
        self.bound = os.bind_any(self.port, None).is_ok();
        if !self.bound {
            os.note(Note::PortInUse { port: self.port, svc: String::from(KIND) });
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        if let SvcEvent::Udp(d) = ev
            && d.dport == self.port
        {
            self.take(os, d);
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![Metric::new("traps", self.traps as i64), Metric::new("informs", self.informs as i64), Metric::new("refused", self.refused as i64), Metric::new("logged", self.logged as i64)]
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        if !self.bound {
            return msg!("Port {} ist belegt: keine Meldungen", self.port);
        }
        match self.rows.back() {
            Some(r) => msg!("{} Traps und {} Informs empfangen; zuletzt {} von {}", self.traps, self.informs, r.what, r.from),
            None => msg!("wartet auf Meldungen an Port {}", self.port),
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
    fn footprint(&self) -> usize {
        256 + self.communities.len() * 32 + self.rows.iter().map(|r| 96 + r.what.len() + r.vars.iter().map(|v| v.len() + 24).sum::<usize>()).sum::<usize>()
    }
}

impl Engine {
    /// the notifications the trap receivers of a device logged, oldest first
    pub fn snmp_traps(&self, dev: u32) -> Vec<TrapRow> {
        let Some(os) = self.node_index(dev).and_then(|ni| self.os(ni)) else { return Vec::new() };
        let mut v: Vec<TrapRow> = Vec::new();
        for s in os.svcs.iter().filter(|s| !s.console) {
            if let Some(r) = s.svc.as_ref().and_then(|x| x.as_any().downcast_ref::<TrapReceiver>()) {
                v.extend(r.traps());
            }
        }
        v.sort_by_key(|r| r.at);
        v
    }
}
