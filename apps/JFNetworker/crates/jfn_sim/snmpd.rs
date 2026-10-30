// snmpd -- the SNMP agent (SNMPv1 of RFC 1157, SNMPv2c of RFC 1901 and RFC 3416): "snmpd" on UDP
// port 161 answers Get, GetNext, GetBulk and Set over the device's state (snmpmib.rs) and sends
// notifications (coldStart, linkDown, linkUp, authenticationFailure) as traps or informs
//
//   config of Linux hosts (JSON, every member may be left out; svcact.rs)
//     {"port": 161, "communities": [{"name": "public", "access": "ro", "source": "10.0.0.0/24"}],
//      "location": "Serverraum", "contact": "admin@lab",
//      "sinks": [{"host": "10.0.0.50", "type": "trap", "version": "2c", "community": "public",
//                 "port": 162}],
//      "notify": ["coldstart", "linkdown", "linkup", "authentication"],
//      "inform_timeout_ms": 1000, "inform_retries": 5}
//   IOS-like devices: the commands snmp-server ... (cli/snmp.rs) through Action::Snmp (SnmpSet);
//   the service "snmp" comes with the first of them and runs from then on, "no snmp-server" ends
//   it; the running configuration is written from SnmpCfg
//   SnmpAgent::from_config / SnmpCfg / Community / TrapHost / SnmpSet / N_* (notifications)
//   Engine::snmp_apply(dev, set), snmp_agent(dev), snmp_link(ni, port, up) (link.rs),
//   snmp_problems (problems.rs)
//
// A request goes through the steps of RFC 1157 4.1 and RFC 3584: a message that is no BER SNMP
// can read counts as snmpInASNParseErrs, another version (SNMPv3) as snmpInBadVersions, both are
// dropped; an unknown community, or a sender its access list (IOS: a standard list by number or
// name, the deciding line counts it) or its network (Linux: "source") keeps out, counts as
// snmpInBadCommunityNames, gets no answer and an authenticationFailure notification where that
// is on.  A Set with a read-only community counts as snmpInBadCommunityUses and gets noAccess
// (version 1: noSuchName).  Responses and notifications that come to the agent's port are
// counted and dropped.
//
// Get, GetNext and GetBulk follow RFC 3416 4.2.1 to 4.2.3: in 2c an object the agent does not
// serve gives noSuchObject, a missing instance noSuchInstance, the end of the view endOfMibView,
// each in its binding; in version 1 the first such binding ends the request with noSuchName and
// its index (the response carries the request's bindings), GetNext skips Counter64 values (RFC
// 3584 4.2.2.1).  GetBulk answers the non-repeaters once and the repeaters up to max-repetitions
// times, stops early when every repeater reached the end, and gives fewer bindings rather than
// more than 1472 bytes (one Ethernet frame without fragments); a Get or GetNext that would not
// fit gets tooBig.  Set checks every binding first (RFC 3416 4.2.5: notWritable, wrongType,
// wrongLength, wrongValue, noCreation; version 1 maps them to noSuchName and badValue, RFC 3584
// 4.4) and changes nothing unless all of them pass.  Writable are sysContact, sysLocation and
// sysName (the device's name, as hostname: the app renames the device), ifAdminStatus (up or
// down: the interface shuts or opens as with shutdown), ifAlias (the description) and
// snmpEnableAuthenTraps.  On IOS-like devices these changes are in the running configuration
// and reach the project with copy run start, as with the console; on Linux hosts they last as
// long as the service runs.
//
// Notifications go to every target from a port of their own (an ephemeral one, as net-snmp and
// IOS send them), version 2c as SNMPv2-Trap or InformRequest (sysUpTime.0, snmpTrapOID.0 and
// the objects of the notification), version 1 as Trap (enterprise sysObjectID, the address the
// trap leaves from, generic and specific trap, time stamp).  coldStart goes out when the agent
// starts (with the device, or when it is switched on); linkDown and linkUp with ifIndex,
// ifAdminStatus and ifOperStatus of the port when a port's link changes (not in the agent's
// first moment: what comes up with the device is no news after its coldStart);
// authenticationFailure for a refused community.  An inform waits for its Response with the same
// request id and goes again after the timeout (IOS-like 15 s, 3 times; Linux 1 s, 5 times) until
// it is acknowledged or given up.  An IOS-like device loses a notification whose next hop has no
// ARP entry yet, as its first ping (an inform tries again).
//
// Metrics: requests, responses, errors (responses with an error), refused, sets, traps, informs,
// informs_acked, informs_lost, informs_pending.  Notes (snmptrace.rs), log lines (IOS-like
// %SNMP-3-AUTHFAIL on the console, "snmpd" on Linux) and "debug snmp packets" (cli/snmp.rs).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value as Json;
use jfn_model::catalog::CliStyle;
use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::ber::{Oid, Value};
use netpkt::mib::{self, Syntax};
use netpkt::nat::Flow;
use netpkt::snmp::{self, Body, Kind, Message, Pdu, TrapV1, VarBind, Version};
use netpkt::{IpAddr, Ipv4, Prefix4};

use crate::action::{ActErr, Action};
use crate::driver::Report;
use crate::http::Wheel;
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::os::{Dgram, Job, Metric, OsApi, Service, SvcEvent};
use crate::snmpmib::{AgentPart, Counters, View};
use crate::snmptrace::SNote;
use crate::trace::Note;

/// the kind of the service, and its name on IOS-like devices
pub const KIND: &str = "snmpd";
pub const IOS_NAME: &str = "snmp";
/// the timer key of the informs' deadlines
const K_WHEEL: u32 = 0;
/// the largest message the agent sends (one Ethernet frame without fragments)
pub const MAX_MSG: usize = 1472;
/// the most bindings a GetBulk answer has
const MAX_BULK: usize = 1000;
const SEV_WARN: u8 = 4;

/// the notifications (SnmpCfg::notify)
pub const N_COLDSTART: u8 = 1;
pub const N_LINKDOWN: u8 = 2;
pub const N_LINKUP: u8 = 4;
pub const N_AUTH: u8 = 8;
pub const N_ALL: u8 = 15;

/// the counters of the snmp group by the last arc of their OID (snmpmib::Counters)
const C_IN_PKTS: usize = 1;
const C_OUT_PKTS: usize = 2;
const C_BAD_VERSIONS: usize = 3;
const C_BAD_COMMUNITY_NAMES: usize = 4;
const C_BAD_COMMUNITY_USES: usize = 5;
const C_ASN_PARSE_ERRS: usize = 6;
const C_TOTAL_REQ_VARS: usize = 13;
const C_TOTAL_SET_VARS: usize = 14;
const C_IN_GET: usize = 15;
const C_IN_GETNEXT: usize = 16;
const C_IN_SET: usize = 17;
const C_IN_GET_RESPONSES: usize = 18;
const C_IN_TRAPS: usize = 19;
const C_OUT_TOO_BIGS: usize = 20;
const C_OUT_NO_SUCH_NAMES: usize = 21;
const C_OUT_BAD_VALUES: usize = 22;
const C_OUT_GEN_ERRS: usize = 24;
const C_OUT_GET_RESPONSES: usize = 28;
const C_OUT_TRAPS: usize = 29;

/// a community: its name, whether it may write, who may use it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Community {
    pub name: String,
    pub rw: bool,
    /// IOS-like: a standard access list by number or name
    pub acl: Option<String>,
    /// Linux: the network it is for (None: everyone)
    pub source: Option<Prefix4>,
}

/// a receiver of notifications
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrapHost {
    pub addr: Ipv4,
    pub port: u16,
    /// informs (acknowledged) rather than traps
    pub inform: bool,
    pub version: Version,
    pub community: String,
}

/// what the agent is set to
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnmpCfg {
    pub communities: Vec<Community>,
    pub location: String,
    pub contact: String,
    pub hosts: Vec<TrapHost>,
    /// N_* bits
    pub notify: u8,
    pub port: u16,
    pub inform_timeout: Ns,
    pub inform_retries: u8,
}

impl SnmpCfg {
    pub fn new(ios: bool) -> SnmpCfg {
        SnmpCfg {
            communities: Vec::new(),
            location: String::new(),
            contact: String::new(),
            hosts: Vec::new(),
            notify: if ios { 0 } else { N_COLDSTART },
            port: snmp::PORT,
            inform_timeout: if ios { 15 * SEC } else { SEC },
            inform_retries: if ios { 3 } else { 5 },
        }
    }
}

/// a change of an IOS-like agent's configuration (Action::Snmp)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnmpSet {
    /// "snmp-server community NAME [ro|rw] [ACL]": a new one or a changed one
    Community { name: String, rw: bool, acl: Option<String> },
    /// "no snmp-server community NAME"
    NoCommunity { name: String },
    /// "[no] snmp-server location|contact TEXT"
    Location(Option<String>),
    Contact(Option<String>),
    /// "snmp-server host ADDR [traps|informs] [version 1|2c] COMMUNITY" (one per address and
    /// community)
    Host(TrapHost),
    /// "no snmp-server host ADDR [COMMUNITY]"
    NoHost { addr: Ipv4, community: Option<String> },
    /// "[no] snmp-server enable traps [snmp [authentication] [linkdown] [linkup] [coldstart]]"
    Notify { bits: u8, on: bool },
    /// "no snmp-server": the agent stops and forgets its configuration
    Off,
}

/// a notification of the agent
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Notif {
    ColdStart,
    LinkDown(u16),
    LinkUp(u16),
    AuthFailure,
}

impl Notif {
    fn bit(self) -> u8 {
        match self {
            Notif::ColdStart => N_COLDSTART,
            Notif::LinkDown(_) => N_LINKDOWN,
            Notif::LinkUp(_) => N_LINKUP,
            Notif::AuthFailure => N_AUTH,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Notif::ColdStart => "coldStart",
            Notif::LinkDown(_) => "linkDown",
            Notif::LinkUp(_) => "linkUp",
            Notif::AuthFailure => "authenticationFailure",
        }
    }
    fn generic(self) -> i32 {
        match self {
            Notif::ColdStart => snmp::G_COLD_START,
            Notif::LinkDown(_) => snmp::G_LINK_DOWN,
            Notif::LinkUp(_) => snmp::G_LINK_UP,
            Notif::AuthFailure => snmp::G_AUTH_FAILURE,
        }
    }
}

/// an inform that waits for its acknowledgement
#[derive(Clone, Debug)]
struct Pending {
    id: i32,
    to: (Ipv4, u16),
    what: &'static str,
    bytes: Vec<u8>,
    tries: u8,
    due: u64,
}

/// the agent
#[derive(Clone, Debug)]
pub struct SnmpAgent {
    pub cfg: SnmpCfg,
    pub ios: bool,
    /// when it started (sysUpTime counts from here)
    pub started: Ns,
    /// port 161 was free
    pub bound: bool,
    /// the port notifications leave from (and inform acknowledgements come to)
    trap_port: u16,
    pub counters: Counters,
    /// (port, virtual time) of the last link changes it saw
    changes: Vec<(u16, Ns)>,
    pending: Vec<Pending>,
    wheel: Wheel<i32>,
    pub requests: u64,
    pub responses: u64,
    pub errors: u64,
    pub refused: u64,
    pub sets: u64,
    pub traps: u64,
    pub informs: u64,
    pub informs_acked: u64,
    pub informs_lost: u64,
}

fn text_of(c: &Json, k: &str) -> Option<String> {
    c.get(k).and_then(|v| v.as_str()).map(|s| String::from(s.trim()))
}

/// N_* of a word of "notify" and "snmp-server enable traps snmp ..."
pub fn notify_bit(w: &str) -> Option<u8> {
    Some(match w.trim().to_ascii_lowercase().as_str() {
        "coldstart" => N_COLDSTART,
        "linkdown" => N_LINKDOWN,
        "linkup" => N_LINKUP,
        "authentication" => N_AUTH,
        _ => return None,
    })
}

/// the words of N_* bits in the order of the running configuration
pub fn notify_words(bits: u8) -> Vec<&'static str> {
    [(N_AUTH, "authentication"), (N_LINKDOWN, "linkdown"), (N_LINKUP, "linkup"), (N_COLDSTART, "coldstart")].iter().filter(|(b, _)| bits & b != 0).map(|(_, w)| *w).collect()
}

impl SnmpAgent {
    pub fn new(ios: bool) -> SnmpAgent {
        SnmpAgent {
            cfg: SnmpCfg::new(ios),
            ios,
            started: 0,
            bound: true,
            trap_port: 0,
            counters: [0; 33],
            changes: Vec::new(),
            pending: Vec::new(),
            wheel: Wheel::new(K_WHEEL),
            requests: 0,
            responses: 0,
            errors: 0,
            refused: 0,
            sets: 0,
            traps: 0,
            informs: 0,
            informs_acked: 0,
            informs_lost: 0,
        }
    }

    /// the agent of a project's definition (Linux hosts; svcact.rs)
    pub fn from_config(c: &Json, ios: bool) -> Result<SnmpAgent, Msg> {
        let mut a = SnmpAgent::new(ios);
        let cfg = &mut a.cfg;
        if let Some(p) = c.get("port").and_then(|v| v.as_u64()) {
            cfg.port = u16::try_from(p).ok().filter(|&p| p > 0).ok_or_else(|| msg!("Port {} gibt es nicht.", p))?;
        }
        cfg.location = text_of(c, "location").unwrap_or_default();
        cfg.contact = text_of(c, "contact").unwrap_or_default();
        for x in c.get("communities").and_then(|v| v.as_array()).unwrap_or(&[]) {
            let name = text_of(x, "name").unwrap_or_default();
            if name.is_empty() || name.contains(char::is_whitespace) {
                return Err(msg!("Eine Community braucht „name“ ohne Leerzeichen."));
            }
            let rw = match text_of(x, "access").as_deref() {
                None | Some("ro") => false,
                Some("rw") => true,
                Some(o) => return Err(msg!("„{}“ ist kein Zugriff (ro oder rw).", o)),
            };
            let source = match text_of(x, "source").as_deref() {
                None | Some("") | Some("default") => None,
                Some(s) => Some(Prefix4::parse(s).or_else(|| Ipv4::parse(s).map(|a| Prefix4 { addr: a, len: 32 })).ok_or_else(|| msg!("„{}“ ist kein Netz wie 10.0.0.0/24.", s))?.trunc()),
            };
            cfg.communities.retain(|k| k.name != name);
            cfg.communities.push(Community { name, rw, acl: None, source });
        }
        for x in c.get("sinks").and_then(|v| v.as_array()).unwrap_or(&[]) {
            let host = text_of(x, "host").unwrap_or_default();
            let addr = Ipv4::parse(&host).ok_or_else(|| msg!("„{}“ ist keine IPv4-Adresse.", host))?;
            let inform = match text_of(x, "type").as_deref() {
                None | Some("trap") => false,
                Some("inform") => true,
                Some(o) => return Err(msg!("„{}“ ist keine Art (trap oder inform).", o)),
            };
            let version = match text_of(x, "version").as_deref() {
                None => Version::V2c,
                Some(v) => Version::parse(v).ok_or_else(|| msg!("Version „{}“ gibt es hier nicht (1 oder 2c).", v))?,
            };
            if inform && version == Version::V1 {
                return Err(msg!("Informs gibt es erst ab Version 2c."));
            }
            let community = text_of(x, "community").unwrap_or_else(|| String::from("public"));
            let port = match x.get("port").and_then(|v| v.as_u64()) {
                Some(p) => u16::try_from(p).ok().filter(|&p| p > 0).ok_or_else(|| msg!("Port {} gibt es nicht.", p))?,
                None => snmp::PORT_TRAP,
            };
            cfg.hosts.push(TrapHost { addr, port, inform, version, community });
        }
        if let Some(list) = c.get("notify").and_then(|v| v.as_array()) {
            cfg.notify = 0;
            for w in list {
                let w = w.as_str().unwrap_or("");
                cfg.notify |= notify_bit(w).ok_or_else(|| msg!("„{}“ ist keine Meldung (coldstart, linkdown, linkup, authentication).", w))?;
            }
        }
        if let Some(t) = c.get("inform_timeout_ms").and_then(|v| v.as_u64()) {
            cfg.inform_timeout = t.clamp(10, 600_000) * MS;
        }
        if let Some(r) = c.get("inform_retries").and_then(|v| v.as_u64()) {
            cfg.inform_retries = r.min(20) as u8;
        }
        Ok(a)
    }

    fn part(&self) -> AgentPart<'_> {
        AgentPart { started: self.started, contact: &self.cfg.contact, location: &self.cfg.location, counters: &self.counters, auth_traps: self.cfg.notify & N_AUTH != 0, changes: &self.changes }
    }

    /// the device as the agent serves it now
    pub fn view(&self, os: &OsApi) -> View {
        View::of(os.eng, os.ni, &self.part())
    }

    /// the number of informs that wait
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    fn count(&mut self, c: usize) {
        if let Some(x) = self.counters.get_mut(c) {
            *x = x.wrapping_add(1);
        }
    }

    // ---------------------------------------------------------------- requests

    /// a datagram on port 161
    fn request(&mut self, os: &mut OsApi, d: Dgram) {
        self.count(C_IN_PKTS);
        let m = match Message::parse(&d.data) {
            Ok(m) => m,
            Err(snmp::Bad::Version(v)) => {
                self.count(C_BAD_VERSIONS);
                os.note(Note::Snmp(SNote::BadVersion { from: d.src, version: v }));
                return;
            }
            Err(_) => {
                self.count(C_ASN_PARSE_ERRS);
                os.note(Note::Snmp(SNote::Malformed { from: d.src }));
                return;
            }
        };
        let community = m.community_text();
        let Some(c) = self.cfg.communities.iter().find(|c| c.name.as_bytes() == m.community.as_slice()).cloned() else {
            self.count(C_BAD_COMMUNITY_NAMES);
            self.refused += 1;
            os.note(Note::Snmp(SNote::BadCommunity { from: d.src, community: community.clone() }));
            self.auth_failure(os, d.src, None);
            return;
        };
        if let Err(acl) = self.allowed(os, &c, &d) {
            self.count(C_BAD_COMMUNITY_NAMES);
            self.refused += 1;
            os.note(Note::Snmp(SNote::Denied { from: d.src, community: community.clone(), acl: acl.clone() }));
            self.auth_failure(os, d.src, Some(acl));
            return;
        }
        let pdu = match m.body {
            Body::Pdu(p) => p,
            Body::Trap(_) => {
                self.count(C_IN_TRAPS);
                return;
            }
        };
        match pdu.kind {
            Kind::Get | Kind::GetNext | Kind::GetBulk | Kind::Set => {}
            Kind::Response => {
                self.count(C_IN_GET_RESPONSES);
                return;
            }
            _ => {
                self.count(C_IN_TRAPS);
                return;
            }
        }
        self.requests += 1;
        os.note(Note::Snmp(SNote::Request { from: d.src, kind: pdu.kind, community: community.clone(), id: pdu.request_id, vars: pdu.varbinds.len().min(u16::MAX as usize) as u16 }));
        let v1 = m.version == Version::V1;
        let resp = match pdu.kind {
            Kind::Get => {
                self.count(C_IN_GET);
                self.get(os, &pdu, v1)
            }
            Kind::GetNext => {
                self.count(C_IN_GETNEXT);
                self.next(os, &pdu, v1)
            }
            Kind::GetBulk => self.bulk(os, &pdu, &m.community),
            _ => {
                self.count(C_IN_SET);
                if !c.rw {
                    self.count(C_BAD_COMMUNITY_USES);
                    os.note(Note::Snmp(SNote::ReadOnly { from: d.src, community: community.clone() }));
                    error_of(&pdu, if v1 { snmp::E_NO_SUCH_NAME } else { snmp::E_NO_ACCESS }, 1)
                } else {
                    self.set(os, &pdu, v1)
                }
            }
        };
        self.respond(os, &d, m.version, &m.community, &pdu, resp);
    }

    /// the access list or network of a community lets the sender in; Err: what kept it out
    fn allowed(&self, os: &mut OsApi, c: &Community, d: &Dgram) -> Result<(), String> {
        if let Some(acl) = &c.acl {
            let Some(src) = d.src.v4() else {
                return Err(format!("Zugriffsliste {}", acl));
            };
            let f = Flow { proto: netpkt::ipv4::PROTO_UDP, src, dst: d.dst4(), sport: d.sport, dport: d.dport, ports: true, icmp: None, flags: 0, later: false, mf: false, id: 0 };
            return match os.eng.acl_match(os.ni, acl, &f) {
                Some(false) => Err(format!("Zugriffsliste {}", acl)),
                _ => Ok(()),
            };
        }
        if let Some(net) = c.source {
            let ok = d.src.v4().is_some_and(|a| net.contains(a));
            if !ok {
                return Err(format!("nur {}", net));
            }
        }
        Ok(())
    }

    /// the response goes back to where the request came from, from the address it went to
    fn respond(&mut self, os: &mut OsApi, d: &Dgram, version: Version, community: &[u8], req: &Pdu, mut p: Pdu) {
        let mut bytes = Message { version, community: Vec::from(community), body: Body::Pdu(p.clone()) }.to_bytes();
        if bytes.len() > MAX_MSG {
            // tooBig: version 1 with the request's bindings (RFC 1157 4.1.2), 2c without any
            // (RFC 3416 4.2.1)
            p.error_status = snmp::E_TOO_BIG;
            p.error_index = 0;
            p.varbinds = if version == Version::V1 { req.varbinds.clone() } else { Vec::new() };
            bytes = Message { version, community: Vec::from(community), body: Body::Pdu(p.clone()) }.to_bytes();
        }
        match snmp::v1_error(p.error_status) {
            snmp::E_TOO_BIG => self.count(C_OUT_TOO_BIGS),
            snmp::E_NO_SUCH_NAME => self.count(C_OUT_NO_SUCH_NAMES),
            snmp::E_BAD_VALUE => self.count(C_OUT_BAD_VALUES),
            snmp::E_GEN_ERR => self.count(C_OUT_GEN_ERRS),
            _ => {}
        }
        if p.error_status != 0 {
            self.errors += 1;
        }
        self.count(C_OUT_PKTS);
        self.count(C_OUT_GET_RESPONSES);
        self.responses += 1;
        os.note(Note::Snmp(SNote::Response { to: d.src, id: p.request_id, status: p.error_status, index: p.error_index, vars: p.varbinds.len().min(u16::MAX as usize) as u16 }));
        os.send_from(d.dst, self.cfg.port, d.src, d.sport, &bytes);
    }

    fn get(&mut self, os: &OsApi, p: &Pdu, v1: bool) -> Pdu {
        let view = self.view(os);
        let mut out = Vec::with_capacity(p.varbinds.len());
        for (i, vb) in p.varbinds.iter().enumerate() {
            let v = view.get(vb.oid.arcs());
            if v1 && (v.is_exception() || matches!(v, Value::Counter64(_))) {
                return error_of(p, snmp::E_NO_SUCH_NAME, i + 1);
            }
            out.push(VarBind::new(vb.oid.clone(), v));
        }
        self.counters[C_TOTAL_REQ_VARS] = self.counters[C_TOTAL_REQ_VARS].wrapping_add(out.iter().filter(|v| !v.value.is_exception()).count() as u32);
        Pdu { kind: Kind::Response, request_id: p.request_id, error_status: 0, error_index: 0, varbinds: out }
    }

    fn next(&mut self, os: &OsApi, p: &Pdu, v1: bool) -> Pdu {
        let view = self.view(os);
        let mut out = Vec::with_capacity(p.varbinds.len());
        for (i, vb) in p.varbinds.iter().enumerate() {
            match view.next(vb.oid.arcs(), v1) {
                Some((o, v)) => out.push(VarBind::new(o, v)),
                None if v1 => return error_of(p, snmp::E_NO_SUCH_NAME, i + 1),
                None => out.push(VarBind::new(vb.oid.clone(), Value::EndOfMibView)),
            }
        }
        self.counters[C_TOTAL_REQ_VARS] = self.counters[C_TOTAL_REQ_VARS].wrapping_add(out.iter().filter(|v| !v.value.is_exception()).count() as u32);
        Pdu { kind: Kind::Response, request_id: p.request_id, error_status: 0, error_index: 0, varbinds: out }
    }

    /// GetBulk (RFC 3416 4.2.3): fewer bindings rather than more than the message takes
    fn bulk(&mut self, os: &OsApi, p: &Pdu, community: &[u8]) -> Pdu {
        let view = self.view(os);
        let n = p.non_repeaters().min(p.varbinds.len());
        let m = p.max_repetitions();
        let r = p.varbinds.len() - n;
        let mut out: Vec<VarBind> = Vec::new();
        for vb in &p.varbinds[..n] {
            out.push(match view.next(vb.oid.arcs(), false) {
                Some((o, v)) => VarBind::new(o, v),
                None => VarBind::new(vb.oid.clone(), Value::EndOfMibView),
            });
        }
        let fits = |vbs: &[VarBind]| Message { version: Version::V2c, community: Vec::from(community), body: Body::Pdu(Pdu::new(Kind::Response, p.request_id, Vec::from(vbs))) }.to_bytes().len() <= MAX_MSG;
        let mut last: Vec<Oid> = p.varbinds[n..].iter().map(|v| v.oid.clone()).collect();
        'rep: for _ in 0..m {
            if r == 0 {
                break;
            }
            let mut ended = 0;
            let mut row: Vec<VarBind> = Vec::with_capacity(r);
            for o in last.iter_mut() {
                match view.next(o.arcs(), false) {
                    Some((x, v)) => {
                        *o = x.clone();
                        row.push(VarBind::new(x, v));
                    }
                    None => {
                        ended += 1;
                        row.push(VarBind::new(o.clone(), Value::EndOfMibView));
                    }
                }
            }
            for vb in row {
                out.push(vb);
                if out.len() > MAX_BULK || !fits(&out) {
                    out.pop();
                    break 'rep;
                }
            }
            if ended == r {
                break;
            }
        }
        self.counters[C_TOTAL_REQ_VARS] = self.counters[C_TOTAL_REQ_VARS].wrapping_add(out.iter().filter(|v| !v.value.is_exception()).count() as u32);
        Pdu { kind: Kind::Response, request_id: p.request_id, error_status: 0, error_index: 0, varbinds: out }
    }

    /// Set: every binding is checked, then all are made (RFC 3416 4.2.5)
    fn set(&mut self, os: &mut OsApi, p: &Pdu, v1: bool) -> Pdu {
        let view = self.view(os);
        let fail = |e: i32, i: usize| error_of(p, if v1 { snmp::v1_error(e) } else { e }, i + 1);
        for (i, vb) in p.varbinds.iter().enumerate() {
            if let Err(e) = check_set(&view, &vb.oid, &vb.value) {
                return fail(e, i);
            }
        }
        for vb in &p.varbinds {
            self.apply_set(os, &view, &vb.oid, &vb.value);
        }
        self.sets += p.varbinds.len() as u64;
        self.counters[C_TOTAL_SET_VARS] = self.counters[C_TOTAL_SET_VARS].wrapping_add(p.varbinds.len() as u32);
        Pdu { kind: Kind::Response, request_id: p.request_id, error_status: 0, error_index: 0, varbinds: p.varbinds.clone() }
    }

    /// makes one checked binding
    fn apply_set(&mut self, os: &mut OsApi, view: &View, oid: &Oid, v: &Value) {
        let a = oid.arcs();
        let shown = mib::show(a, v, false);
        let text = match v {
            Value::Str(b) => String::from_utf8_lossy(b).into_owned(),
            _ => String::new(),
        };
        let dev = os.dev();
        let ni = os.ni;
        match a {
            [1, 3, 6, 1, 2, 1, 1, 4, 0] => self.cfg.contact = text,
            [1, 3, 6, 1, 2, 1, 1, 6, 0] => self.cfg.location = text,
            [1, 3, 6, 1, 2, 1, 1, 5, 0] => {
                // the device's name, as "hostname" (the app renames the device in the project)
                if text != os.hostname() && os.eng.apply(Action::Hostname { dev, name: text.clone() }).is_ok() {
                    os.eng.cli.reports.push(Report::Hostname { dev, name: text });
                }
            }
            [1, 3, 6, 1, 2, 1, 11, 30, 0] => {
                if matches!(v, Value::Int(1)) {
                    self.cfg.notify |= N_AUTH;
                } else {
                    self.cfg.notify &= !N_AUTH;
                }
            }
            [1, 3, 6, 1, 2, 1, 2, 2, 1, 7, idx] => {
                let up = matches!(v, Value::Int(1));
                if let Some(row) = view.if_row(*idx) {
                    let r = match (row.iface, row.port) {
                        (Some(i), _) => {
                            let name = os.ifaces().get(i as usize).map(|f| f.name.clone()).unwrap_or_default();
                            os.eng.apply(Action::IfUp { dev, iface: name, up })
                        }
                        (None, Some(_)) => os.eng.apply(Action::PortAdmin { dev, port: row.descr.clone(), up }),
                        _ => Ok(()),
                    };
                    let _ = r;
                    // the IOS-like console reports the change as after "shutdown"
                    os.eng.ios_snap(ni);
                }
            }
            [1, 3, 6, 1, 2, 1, 31, 1, 1, 1, 18, idx] => {
                if let Some(row) = view.if_row(*idx)
                    && let Some(c) = os.eng.cdev_mut(ni)
                {
                    if text.is_empty() {
                        c.cfg.descr.remove(&row.descr);
                    } else {
                        c.cfg.descr.insert(row.descr.clone(), text);
                    }
                }
            }
            _ => return,
        }
        os.note(Note::Snmp(SNote::Set { var: mib::name_of(a, false), value: shown }));
    }

    // ---------------------------------------------------------------- notifications

    /// an authenticationFailure where it is on, and the log line
    fn auth_failure(&mut self, os: &mut OsApi, from: IpAddr, acl: Option<String>) {
        if self.ios {
            let why = match acl {
                Some(a) => format!("{} keeps it out", a.replace("Zugriffsliste", "access list")),
                None => String::from("unknown community"),
            };
            let ni = os.ni;
            os.eng.ios_log(ni, &format!("%SNMP-3-AUTHFAIL: refused a request from {}: {}", from, why));
        } else {
            let why = if acl.is_some() { "community not for this address" } else { "unknown community" };
            os.log(SEV_WARN, "snmpd", &format!("refused a request from {}: {}", from, why));
        }
        self.notify(os, Notif::AuthFailure);
    }

    /// a notification to every receiver, if it is on
    fn notify(&mut self, os: &mut OsApi, what: Notif) {
        if self.cfg.notify & what.bit() == 0 || self.cfg.hosts.is_empty() {
            return;
        }
        if self.trap_port == 0 {
            match os.bind(0, None) {
                Ok(p) => self.trap_port = p,
                Err(_) => return,
            }
        }
        let view = self.view(os);
        // the objects of linkDown and linkUp: ifIndex, ifAdminStatus, ifOperStatus (RFC 2863)
        let mut objs: Vec<VarBind> = Vec::new();
        if let Notif::LinkDown(p) | Notif::LinkUp(p) = what
            && let Some(r) = view.if_of_port(p)
        {
            let i = r.index;
            objs.push(VarBind::new(Oid::new(&snmp::IF_INDEX).child(&[i]), Value::Int(i as i64)));
            objs.push(VarBind::new(Oid::new(&snmp::IF_ADMIN_STATUS).child(&[i]), Value::Int(if r.admin_up { 1 } else { 2 })));
            objs.push(VarBind::new(Oid::new(&snmp::IF_OPER_STATUS).child(&[i]), Value::Int(if r.oper_up { 1 } else { 2 })));
        }
        let trap_oid = snmp::trap_oid(what.generic(), &view.object_id, 0);
        for h in self.cfg.hosts.clone() {
            let id = (os.rand() & 0x7FFF_FFFF) as i32;
            let msg = match h.version {
                Version::V1 => {
                    let agent = os.source_for(h.addr).unwrap_or(Ipv4::UNSPECIFIED);
                    Message { version: Version::V1, community: Vec::from(h.community.as_bytes()), body: Body::Trap(TrapV1 { enterprise: view.object_id.clone(), agent_addr: agent, generic: what.generic(), specific: 0, timestamp: view.uptime, varbinds: objs.clone() }) }
                }
                Version::V2c => {
                    let mut vbs = alloc::vec![VarBind::new(Oid::new(&snmp::SYS_UPTIME_0), Value::TimeTicks(view.uptime)), VarBind::new(Oid::new(&snmp::SNMP_TRAP_OID_0), Value::Oid(trap_oid.clone()))];
                    vbs.extend(objs.iter().cloned());
                    let kind = if h.inform { Kind::Inform } else { Kind::TrapV2 };
                    Message::new(Version::V2c, &h.community, Pdu::new(kind, id, vbs))
                }
            };
            let bytes = msg.to_bytes();
            let inform = h.inform && h.version == Version::V2c;
            if !os.send(self.trap_port, h.addr, h.port, &bytes) {
                os.note(Note::Snmp(SNote::NoRoute { to: h.addr }));
                continue;
            }
            os.note(Note::Snmp(SNote::Notify { to: h.addr, what: String::from(what.name()), inform, version: h.version }));
            self.count(C_OUT_PKTS);
            self.count(C_OUT_TRAPS);
            if inform {
                self.informs += 1;
                let due = self.wheel.add(os.now() + self.cfg.inform_timeout, id);
                self.pending.push(Pending { id, to: (h.addr, h.port), what: what.name(), bytes, tries: 1, due });
            } else {
                self.traps += 1;
            }
        }
        self.wheel.arm(os);
    }

    /// a datagram to the notification port: the acknowledgement of an inform
    fn on_ack(&mut self, os: &mut OsApi, d: Dgram) {
        let Ok(m) = Message::parse(&d.data) else { return };
        let Some(p) = m.pdu().filter(|p| p.kind == Kind::Response) else { return };
        let Some(i) = self.pending.iter().position(|x| x.id == p.request_id && IpAddr::V4(x.to.0) == d.src && x.to.1 == d.sport) else { return };
        let x = self.pending.remove(i);
        self.wheel.cancel(x.due);
        self.informs_acked += 1;
        os.note(Note::Snmp(SNote::InformAcked { to: x.to.0, what: String::from(x.what) }));
        self.wheel.arm(os);
    }

    /// informs whose time ran out: again, or given up
    fn deadlines(&mut self, os: &mut OsApi) {
        self.wheel.fired();
        let now = os.now();
        for (_, id) in self.wheel.due(now) {
            let Some(i) = self.pending.iter().position(|x| x.id == id) else { continue };
            if self.pending[i].tries > self.cfg.inform_retries {
                let x = self.pending.remove(i);
                self.informs_lost += 1;
                os.note(Note::Snmp(SNote::InformLost { to: x.to.0, what: String::from(x.what) }));
                continue;
            }
            let x = &mut self.pending[i];
            x.tries += 1;
            let (to, bytes, n, what) = (x.to, x.bytes.clone(), x.tries - 1, x.what);
            os.note(Note::Snmp(SNote::InformAgain { to: to.0, what: String::from(what), n }));
            os.send(self.trap_port, to.0, to.1, &bytes);
            self.count(C_OUT_PKTS);
            let due = self.wheel.add(now + self.cfg.inform_timeout, id);
            self.pending[i].due = due;
        }
        self.wheel.arm(os);
    }

    /// a port's link changed (link.rs through the inbox)
    fn port_changed(&mut self, os: &mut OsApi, port: u16, up: bool) {
        let now = os.now();
        self.changes.retain(|c| c.0 != port);
        self.changes.push((port, now));
        // what comes up with the device in the agent's first moment is no news after coldStart
        if now == self.started {
            return;
        }
        self.notify(os, if up { Notif::LinkUp(port) } else { Notif::LinkDown(port) });
    }
}

/// the error response to a request: its bindings, the status, the index (from 1)
fn error_of(p: &Pdu, status: i32, index: usize) -> Pdu {
    Pdu { kind: Kind::Response, request_id: p.request_id, error_status: status, error_index: index as i32, varbinds: p.varbinds.clone() }
}

/// may a Set give this binding its value? Err: the error of RFC 3416 4.2.5
fn check_set(view: &View, oid: &Oid, v: &Value) -> Result<(), i32> {
    let a = oid.arcs();
    let Some((obj, rest)) = mib::object_of(a) else {
        return Err(snmp::E_NOT_WRITABLE);
    };
    let writable = matches!(obj.name, "sysContact" | "sysName" | "sysLocation" | "ifAdminStatus" | "ifAlias" | "snmpEnableAuthenTraps");
    if !matches!(obj.kind, mib::ObjKind::Scalar | mib::ObjKind::Column) || !writable {
        return Err(snmp::E_NOT_WRITABLE);
    }
    if v.tag() != obj.syntax.tag() {
        return Err(snmp::E_WRONG_TYPE);
    }
    match (obj.syntax, v) {
        (Syntax::DisplayString, Value::Str(b)) => {
            if b.len() > 255 {
                return Err(snmp::E_WRONG_LENGTH);
            }
            let text = core::str::from_utf8(b).map_err(|_| snmp::E_WRONG_VALUE)?;
            if text.chars().any(|c| c.is_control()) {
                return Err(snmp::E_WRONG_VALUE);
            }
            if obj.name == "sysName" && !jfn_model::project::valid_name(text) {
                return Err(snmp::E_WRONG_VALUE);
            }
        }
        (Syntax::Integer, Value::Int(i)) => {
            if !(1..=2).contains(i) {
                return Err(snmp::E_WRONG_VALUE);
            }
        }
        _ => return Err(snmp::E_WRONG_TYPE),
    }
    // the instance: .0 of a scalar, an interface that is there for the columns
    let exists = match obj.kind {
        mib::ObjKind::Scalar => rest == [0],
        _ => rest.len() == 1 && view.if_row(rest[0]).is_some(),
    };
    if !exists {
        return Err(snmp::E_NO_CREATION);
    }
    Ok(())
}

impl Service for SnmpAgent {
    fn kind(&self) -> &'static str {
        KIND
    }
    fn start(&mut self, os: &mut OsApi) {
        self.started = os.now();
        self.changes.clear();
        self.trap_port = 0;
        self.bound = os.bind_any(self.cfg.port, None).is_ok();
        if !self.bound {
            os.note(Note::PortInUse { port: self.cfg.port, svc: String::from(KIND) });
        }
        self.notify(os, Notif::ColdStart);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Udp(d) if d.dport == self.cfg.port => self.request(os, d),
            SvcEvent::Udp(d) if d.dport == self.trap_port && self.trap_port != 0 => self.on_ack(os, d),
            SvcEvent::Timer(K_WHEEL) => self.deadlines(os),
            SvcEvent::Port { port, up } => self.port_changed(os, port, up),
            _ => {}
        }
    }
    fn stop(&mut self, _os: &mut OsApi) {
        self.pending.clear();
        self.wheel.clear();
        self.trap_port = 0;
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![
            Metric::new("requests", self.requests as i64),
            Metric::new("responses", self.responses as i64),
            Metric::new("errors", self.errors as i64),
            Metric::new("refused", self.refused as i64),
            Metric::new("sets", self.sets as i64),
            Metric::new("traps", self.traps as i64),
            Metric::new("informs", self.informs as i64),
            Metric::new("informs_acked", self.informs_acked as i64),
            Metric::new("informs_lost", self.informs_lost as i64),
            Metric::new("informs_pending", self.pending.len() as i64),
        ]
    }
    fn metric_kind(&self, name: &str) -> crate::series::MetricKind {
        if name == "informs_pending" { crate::series::MetricKind::Gauge } else { crate::series::metric_kind_of(name) }
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        if !self.bound {
            return msg!("Port {} ist belegt: der Agent antwortet nicht", self.cfg.port);
        }
        if self.cfg.communities.is_empty() {
            return msg!("ohne Community: beantwortet nichts, {} Meldungen gesendet", self.traps + self.informs);
        }
        msg!("{} Anfragen beantwortet, {} abgelehnt, {} Meldungen gesendet", self.responses, self.refused, self.traps + self.informs)
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
        512 + self.cfg.communities.len() * 96 + self.cfg.hosts.len() * 96 + self.changes.len() * 16 + self.pending.iter().map(|p| 96 + p.bytes.len()).sum::<usize>() + self.cfg.location.len() + self.cfg.contact.len()
    }
}

// ---------------------------------------------------------------- the engine side

impl Engine {
    /// the slot of a device's agent: (node slot, service id); by its kind, since the service
    /// is out of its slot while it runs (a Set that shuts a port hears of it all the same)
    fn snmp_slot(&self, dev: u32) -> Option<(usize, u32)> {
        let ni = self.node_index(dev)?;
        let os = self.os(ni)?;
        os.svcs.iter().find(|s| !s.console && s.kind == KIND).map(|s| (ni, s.id))
    }

    /// the agent of a device (None while it has none)
    pub fn snmp_agent(&self, dev: u32) -> Option<&SnmpAgent> {
        self.service_of::<SnmpAgent>(dev)
    }

    /// the agent runs on the device
    pub fn snmp_running(&self, dev: u32) -> bool {
        self.snmp_slot(dev).is_some_and(|(ni, id)| self.os(ni).is_some_and(|o| o.running(id)))
    }

    /// the view an agent of the device would serve now (show snmp, the tests)
    pub fn snmp_view(&self, dev: u32) -> Option<View> {
        let ni = self.node_index(dev)?;
        let a = self.snmp_agent(dev)?;
        Some(View::of(self, ni, &a.part()))
    }

    /// a port's link changed: the device's agent hears of it (linkDown, linkUp, ifLastChange)
    pub(crate) fn snmp_link(&mut self, ni: usize, port: u16, up: bool) {
        let dev = self.dev_of(ni);
        if let Some((_, id)) = self.snmp_slot(dev)
            && self.os(ni).is_some_and(|o| o.running(id))
        {
            self.os_push(ni, Job::Ev(id, SvcEvent::Port { port, up }));
        }
    }

    /// Action::Snmp: a change of an IOS-like agent's configuration; the service "snmp" comes with
    /// the first one and runs, "no snmp-server" stops it
    pub(crate) fn snmp_apply(&mut self, dev: u32, set: SnmpSet) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let ios = matches!(self.nodes.get(ni), Some(Some(n)) if n.model.cli == CliStyle::Ios);
        if self.os(ni).is_none() {
            return Err(ActErr::NoIp(dev));
        }
        if !ios {
            return Err(ActErr::Refused(msg!("Den SNMP-Agenten eines Linux-Hosts stellt der Dienst snmpd ein.")));
        }
        if self.snmp_slot(dev).is_none() {
            if matches!(set, SnmpSet::Off | SnmpSet::NoCommunity { .. } | SnmpSet::NoHost { .. } | SnmpSet::Notify { on: false, .. } | SnmpSet::Location(None) | SnmpSet::Contact(None)) {
                return Ok(());
            }
            self.add_service(dev, IOS_NAME, Box::new(SnmpAgent::new(true)), true)?;
        }
        let (ni, id) = self.snmp_slot(dev).ok_or(ActErr::NoIp(dev))?;
        if set == SnmpSet::Off {
            let name = self.os(ni).and_then(|o| o.svcs.iter().find(|s| s.id == id)).map(|s| s.name.clone()).unwrap_or_default();
            return self.remove_service(dev, &name);
        }
        let Some(a) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.id == id)).and_then(|s| s.svc.as_mut()).and_then(|x| x.as_any_mut().downcast_mut::<SnmpAgent>()) else {
            return Err(ActErr::NoIp(dev));
        };
        let c = &mut a.cfg;
        match set {
            SnmpSet::Community { name, rw, acl } => match c.communities.iter_mut().find(|k| k.name == name) {
                Some(k) => {
                    k.rw = rw;
                    k.acl = acl;
                }
                None => c.communities.push(Community { name, rw, acl, source: None }),
            },
            SnmpSet::NoCommunity { name } => {
                let before = c.communities.len();
                c.communities.retain(|k| k.name != name);
                if c.communities.len() == before {
                    return Err(ActErr::Refused(msg!("Die Community „{}“ gibt es nicht.", name)));
                }
            }
            SnmpSet::Location(t) => c.location = t.unwrap_or_default(),
            SnmpSet::Contact(t) => c.contact = t.unwrap_or_default(),
            SnmpSet::Host(h) => match c.hosts.iter_mut().find(|x| x.addr == h.addr && x.community == h.community) {
                Some(x) => *x = h,
                None => c.hosts.push(h),
            },
            SnmpSet::NoHost { addr, community } => {
                let before = c.hosts.len();
                c.hosts.retain(|x| !(x.addr == addr && community.as_ref().is_none_or(|k| *k == x.community)));
                if c.hosts.len() == before {
                    return Err(ActErr::Refused(msg!("{} ist kein Empfänger von Meldungen.", addr)));
                }
            }
            SnmpSet::Notify { bits, on } => {
                if on {
                    c.notify |= bits;
                } else {
                    c.notify &= !bits;
                }
            }
            SnmpSet::Off => {}
        }
        Ok(())
    }

    /// the problems of SNMP: a receiver of notifications without a route, a community that may
    /// write and that everyone may use (problems.rs)
    pub(crate) fn snmp_problems(&self, out: &mut Vec<crate::Problem>) {
        use crate::problems::{Problem, ProblemKind, Severity};
        for dev in self.devices() {
            let Some(a) = self.snmp_agent(dev) else { continue };
            let Some(ni) = self.node_index(dev) else { continue };
            if !self.os(ni).is_some_and(|o| o.svcs.iter().any(|s| s.enabled && s.kind == KIND)) {
                continue;
            }
            let name = self.node(dev).map(|n| n.name.clone()).unwrap_or_default();
            for h in &a.cfg.hosts {
                if self.source_for(ni, h.addr).is_none() {
                    let msg = msg!("{} schickt SNMP-Meldungen an {}, hat aber keinen Weg dorthin.", name, h.addr);
                    out.push(Problem { kind: ProblemKind::SnmpTrapHost, severity: Severity::Warning, dev, port: None, link: None, others: Vec::new(), text: msg.text(), msg });
                }
            }
            for c in a.cfg.communities.iter().filter(|c| c.rw && c.acl.is_none() && c.source.is_none()) {
                let msg = msg!("Auf {} darf jeder mit der Community „{}“ schreiben: eine Zugriffsliste fehlt.", name, c.name);
                out.push(Problem { kind: ProblemKind::SnmpOpenRw, severity: Severity::Warning, dev, port: None, link: None, others: Vec::new(), text: msg.text(), msg });
            }
        }
    }
}
