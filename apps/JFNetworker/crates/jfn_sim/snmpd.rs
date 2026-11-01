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
//     SNMPv3: {"engine_id": "80007ed903...", "users": [{"name": "admin", "level": "authPriv",
//              "auth": "SHA", "auth_password": "...", "priv": "AES", "priv_password": "...",
//              "access": "rw", "subtree": "1.3.6.1.2.1.1"}]}: a user with a group and a view of
//              their own
//   SnmpAgent::from_config / SnmpCfg / Community / TrapHost / SnmpSet / N_* (notifications)
//   UsmUser / GroupRow / ViewRow: SNMPv3's users, groups and views
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
// SNMPv3 (netpkt::snmpv3; RFC 3412 7.2, RFC 3414 3.2, RFC 3415): a message goes through the steps
// of the USM in the order of the RFC, each failure with its Report PDU (noAuthNoPriv, unless the
// sender proved who it is: not-in-time-window is signed): an engine ID that is not ours or empty
// (usmStatsUnknownEngineIDs: that is the discovery), a user the agent does not know
// (usmStatsUnknownUserNames), a level the user has no keys for (usmStatsUnsupportedSecLevels), a wrong
// digest (usmStatsWrongDigests), boots or a time more than 150 s off (usmStatsNotInTimeWindows), an
// encrypted PDU it cannot read (usmStatsDecryptionErrors); a context of another name gets
// snmpUnknownContexts.  Then VACM: the user's group at this level names the read and write views
// (no group: authorizationError; no read view named: all of 1.3.6.1; no write view: Set gets noAccess).
// In a view a Get of something else gives noSuchObject and GetNext and GetBulk go past it.  The engine's
// boots count every start of the agent (they stay as long as the project is open, the power does
// not reset them), its time the seconds since then.  The engine ID: "snmp-server engineID local",
// else format 3 of the first interface's MAC under the enterprise number for documentation.
// Keys are localized when a user is configured (IOS-like) or the agent starts (Linux); DES is not
// here (lib/crypto has none); notifications stay with versions 1 and 2c.  A message of version 3
// that cannot be read as SNMPv3 counts as a version the agent does not speak.
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
use netpkt::snmpv3::{self as v3, AuthProto, Level, PrivProto, Scoped, ScopedPdu, Usm, V3};
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

/// a row of a view ("snmp-server view NAME OID included|excluded")
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewRow {
    pub name: String,
    pub oid: Oid,
    pub included: bool,
}

/// "snmp-server group NAME v3 noauth|auth|priv [read VIEW] [write VIEW]"
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupRow {
    pub name: String,
    pub level: Level,
    /// None: everything below 1.3.6.1 ("v1default")
    pub read: Option<String>,
    /// None: no writing
    pub write: Option<String>,
}

/// a key of a user: as a password (what a user's definition holds until an engine ID localizes it)
/// or localized
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UserKey {
    Password(String),
    Localized(Vec<u8>),
}

/// a user of SNMPv3: "snmp-server user NAME GROUP v3 ...", or from a Linux agent's settings
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsmUser {
    pub name: String,
    pub group: String,
    pub auth: Option<(AuthProto, UserKey)>,
    pub privacy: Option<(PrivProto, UserKey)>,
}

impl UsmUser {
    /// the level its keys allow
    pub fn level(&self) -> Level {
        match (&self.auth, &self.privacy) {
            (Some(_), Some(_)) => Level::AuthPriv,
            (Some(_), None) => Level::AuthNoPriv,
            _ => Level::NoAuthNoPriv,
        }
    }
    /// its keys for one engine
    pub fn localize(&mut self, engine: &[u8]) {
        let Some((a, k)) = &mut self.auth else {
            return;
        };
        let proto = *a;
        if let UserKey::Password(p) = k {
            *k = UserKey::Localized(v3::localize(proto, &v3::password_to_key(proto, p.as_bytes()), engine));
        }
        if let Some((_, k)) = &mut self.privacy
            && let UserKey::Password(p) = k
        {
            *k = UserKey::Localized(v3::localize(proto, &v3::password_to_key(proto, p.as_bytes()), engine));
        }
    }
}

/// the key bytes, once localized
fn key_of(k: &UserKey) -> &[u8] {
    match k {
        UserKey::Localized(b) => b,
        UserKey::Password(_) => &[],
    }
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
    /// "snmp-server engineID local" (None: from the first interface's MAC)
    pub engine_id: Option<Vec<u8>>,
    pub views: Vec<ViewRow>,
    pub groups: Vec<GroupRow>,
    pub users: Vec<UsmUser>,
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
            engine_id: None,
            views: Vec::new(),
            groups: Vec::new(),
            users: Vec::new(),
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
    /// "[no] snmp-server engineID local HEX"
    EngineId(Option<Vec<u8>>),
    /// "snmp-server view NAME OID included|excluded"
    View(ViewRow),
    /// "no snmp-server view NAME [OID]"
    NoView { name: String, oid: Option<Oid> },
    /// "snmp-server group NAME v3 LEVEL [read VIEW] [write VIEW]"
    Group(GroupRow),
    /// "no snmp-server group NAME v3 LEVEL"
    NoGroup { name: String, level: Level },
    /// "snmp-server user NAME GROUP v3 ..."
    User(UsmUser),
    /// "no snmp-server user NAME GROUP v3"
    NoUser { name: String },
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
    /// snmpEngineBoots: every start of the agent
    pub boots: i32,
    /// the counters of the reports, by the last but one arc of usmStats (1 to 6); 7 snmpUnknownContexts
    pub usm: [u32; 8],
    /// the read and write view of the SNMPv3 request that runs (None: no such limit)
    limit: Option<Vec<(Oid, bool)>>,
    /// messages of version 3 it took (requests that reached VACM)
    pub v3_requests: u64,
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
            boots: 0,
            usm: [0; 8],
            limit: None,
            v3_requests: 0,
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
        if let Some(t) = text_of(c, "engine_id") {
            cfg.engine_id = Some(v3::parse_engine_id(&t).ok_or_else(|| msg!("„{}“ ist keine Engine-ID (5 bis 32 Bytes hexadezimal).", t))?);
        }
        for x in c.get("users").and_then(|v| v.as_array()).unwrap_or(&[]) {
            cfg.linux_user(x)?;
        }
        Ok(a)
    }

    /// the views that let a request read or write, by what VACM says; Err: no group at this level
    fn vacm(&self, user: &UsmUser, level: Level, write: bool) -> Result<Vec<(Oid, bool)>, ()> {
        let row = self.cfg.groups.iter().filter(|g| g.name == user.group && g.level <= level).max_by_key(|g| g.level).ok_or(())?;
        let name = if write { &row.write } else { &row.read };
        Ok(match name {
            None if write => Vec::new(),
            None => alloc::vec![(Oid::new(&[1, 3, 6, 1]), true)],
            Some(n) if n == "v1default" => alloc::vec![(Oid::new(&[1, 3, 6, 1]), true)],
            Some(n) => self.cfg.views.iter().filter(|v| v.name == *n).map(|v| (v.oid.clone(), v.included)).collect(),
        })
    }

    /// the view of the request that runs shows this object: the longest subtree decides
    fn visible(&self, arcs: &[u32]) -> bool {
        let Some(rows) = &self.limit else {
            return true;
        };
        rows.iter().filter(|(o, _)| arcs.starts_with(o.arcs())).max_by_key(|(o, _)| o.len()).is_some_and(|(_, inc)| *inc)
    }

    /// the next object in the view of the request
    fn next_in_view(&self, view: &View, arcs: &[u32], v1: bool) -> Option<(Oid, Value)> {
        let (o, v) = view.next(arcs, v1)?;
        if self.visible(o.arcs()) {
            return Some((o, v));
        }
        let mut cur = o;
        loop {
            let (o, v) = view.next(cur.arcs(), v1)?;
            if self.visible(o.arcs()) {
                return Some((o, v));
            }
            cur = o;
        }
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
                // SNMPv3 (what cannot be read as that counts like any other version)
                if v == 3
                    && let Ok(m) = V3::parse(&d.data)
                {
                    self.request_v3(os, d, m);
                    return;
                }
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
            let v = if self.visible(vb.oid.arcs()) { view.get(vb.oid.arcs()) } else { Value::NoSuchObject };
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
            match self.next_in_view(&view, vb.oid.arcs(), v1) {
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
            out.push(match self.next_in_view(&view, vb.oid.arcs(), false) {
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
                match self.next_in_view(&view, o.arcs(), false) {
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

    // ---------------------------------------------------------------- SNMPv3

    /// a message of version 3: the steps of the USM, then VACM
    fn request_v3(&mut self, os: &mut OsApi, d: Dgram, m: V3) {
        let me = os.eng.snmp_engine_id(os.ni, &self.cfg.engine_id);
        let rid = m.plain().map_or(0, |s| s.pdu.request_id);
        let Some(level) = m.level().filter(|_| m.model as i64 == v3::MODEL_USM) else {
            // privacy without authentication, another security model: dropped (RFC 3412 7.2)
            os.note(Note::Snmp(SNote::Malformed { from: d.src }));
            return;
        };
        if m.usm.engine_id != me {
            self.report(os, &d, &m, rid, 4, &v3::USM_STATS_UNKNOWN_ENGINE_IDS, None);
            return;
        }
        let Some(user) = self.cfg.users.iter().find(|u| u.name.as_bytes() == m.usm.user.as_slice()).cloned() else {
            self.report(os, &d, &m, rid, 3, &v3::USM_STATS_UNKNOWN_USER_NAMES, None);
            return;
        };
        if level > user.level() {
            self.report(os, &d, &m, rid, 1, &v3::USM_STATS_UNSUPPORTED_SEC_LEVELS, None);
            return;
        }
        let (time, boots) = (self.engine_time(os), self.boots);
        if level >= Level::AuthNoPriv {
            let Some((proto, key)) = &user.auth else {
                return;
            };
            if !v3::check(&d.data, *proto, key_of(key)) {
                self.report(os, &d, &m, rid, 5, &v3::USM_STATS_WRONG_DIGESTS, None);
                self.v3_auth_failure(os, d.src, &user.name);
                return;
            }
            if m.usm.boots != boots || boots == i32::MAX || (m.usm.time as i64 - time as i64).abs() > 150 {
                self.report(os, &d, &m, rid, 2, &v3::USM_STATS_NOT_IN_TIME_WINDOWS, Some(&user));
                return;
            }
        }
        let scoped = match (&m.scoped, level) {
            (Scoped::Plain(s), Level::NoAuthNoPriv | Level::AuthNoPriv) => s.clone(),
            (Scoped::Encrypted(e), Level::AuthPriv) => {
                let plain = user.privacy.as_ref().and_then(|(_, k)| v3::decrypt(&v3::priv_key(key_of(k)), m.usm.boots, m.usm.time, &m.usm.privacy, e));
                match plain.and_then(|p| ScopedPdu::parse(&p).ok()) {
                    Some(s) => s,
                    None => {
                        self.report(os, &d, &m, rid, 6, &v3::USM_STATS_DECRYPTION_ERRORS, Some(&user));
                        return;
                    }
                }
            }
            _ => {
                self.report(os, &d, &m, rid, 6, &v3::USM_STATS_DECRYPTION_ERRORS, None);
                return;
            }
        };
        if !scoped.context_name.is_empty() || !(scoped.context_engine.is_empty() || scoped.context_engine == me) {
            self.report(os, &d, &m, scoped.pdu.request_id, 7, &v3::UNKNOWN_CONTEXTS, if level == Level::NoAuthNoPriv { None } else { Some(&user) });
            return;
        }
        let pdu = scoped.pdu;
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
        self.v3_requests += 1;
        os.note(Note::Snmp(SNote::V3Request { from: d.src, kind: pdu.kind, user: user.name.clone(), level, id: pdu.request_id, vars: pdu.varbinds.len().min(u16::MAX as usize) as u16 }));
        // VACM: what this user may see or change at this level
        let resp = match self.vacm(&user, level, pdu.kind == Kind::Set) {
            Err(()) => error_of(&pdu, snmp::E_AUTHORIZATION_ERROR, 1),
            Ok(rows) => {
                self.limit = Some(rows);
                let r = match pdu.kind {
                    Kind::Get => {
                        self.count(C_IN_GET);
                        self.get(os, &pdu, false)
                    }
                    Kind::GetNext => {
                        self.count(C_IN_GETNEXT);
                        self.next(os, &pdu, false)
                    }
                    Kind::GetBulk => self.bulk(os, &pdu, &[0u8; 180]),
                    _ => {
                        self.count(C_IN_SET);
                        match pdu.varbinds.iter().position(|vb| !self.visible(vb.oid.arcs())) {
                            Some(i) => error_of(&pdu, snmp::E_NO_ACCESS, i + 1),
                            None => self.set(os, &pdu, false),
                        }
                    }
                };
                self.limit = None;
                r
            }
        };
        self.respond_v3(os, &d, &m, &user, level, &me, resp);
    }

    /// the seconds since the agent started
    fn engine_time(&self, os: &OsApi) -> i32 {
        (os.now().saturating_sub(self.started) / SEC).min(i32::MAX as u64) as i32
    }

    /// a message of the agent itself: USM parameters, encrypted and signed by the level
    fn seal_v3(&self, os: &mut OsApi, m: &V3, user: Option<&UsmUser>, level: Level, me: &[u8], scoped: ScopedPdu) -> Vec<u8> {
        let (time, boots) = (self.engine_time(os), self.boots);
        let mut usm = Usm { engine_id: Vec::from(me), boots, time, user: m.usm.user.clone(), auth: Vec::new(), privacy: Vec::new() };
        let mut body = Scoped::Plain(scoped.clone());
        if level == Level::AuthPriv
            && let Some((_, k)) = user.and_then(|u| u.privacy.as_ref())
        {
            let mut salt = [0u8; 8];
            salt[..4].copy_from_slice(&os.rand().to_be_bytes());
            salt[4..].copy_from_slice(&os.rand().to_be_bytes());
            if let Some(c) = v3::encrypt(&v3::priv_key(key_of(k)), boots, time, &salt, &scoped.to_bytes()) {
                usm.privacy = Vec::from(&salt[..]);
                body = Scoped::Encrypted(c);
            }
        }
        let auth = user.filter(|_| level >= Level::AuthNoPriv).and_then(|u| u.auth.as_ref());
        if let Some((p, _)) = auth {
            usm.auth = alloc::vec![0u8; p.tag_len()];
        }
        let msg = V3 { id: m.id, max_size: v3::MAX_SIZE as i32, flags: level.flags(), model: v3::MODEL_USM as i32, usm, scoped: body };
        let mut bytes = msg.to_bytes();
        if let Some((p, k)) = auth {
            v3::sign(&mut bytes, *p, key_of(k));
        }
        bytes
    }

    /// a Report PDU: `counter` counts one more; signed when `user` is given
    #[allow(clippy::too_many_arguments)]
    fn report(&mut self, os: &mut OsApi, d: &Dgram, m: &V3, rid: i32, which: usize, oid: &[u32], user: Option<&UsmUser>) {
        self.usm[which] = self.usm[which].wrapping_add(1);
        self.refused += 1;
        let what = v3::report_name(oid);
        os.note(Note::Snmp(SNote::V3Report { to: d.src, what: String::from(what) }));
        // (only a message that asks for reports gets one: RFC 3412 7.1)
        if !m.reportable() {
            return;
        }
        let me = os.eng.snmp_engine_id(os.ni, &self.cfg.engine_id);
        let scoped = ScopedPdu { context_engine: me.clone(), context_name: Vec::new(), pdu: v3::report_pdu(rid, oid, self.usm[which]) };
        let level = if user.is_some() { Level::AuthNoPriv } else { Level::NoAuthNoPriv };
        let bytes = self.seal_v3(os, m, user, level, &me, scoped);
        self.count(C_OUT_PKTS);
        os.send_from(d.dst, self.cfg.port, d.src, d.sport, &bytes);
    }

    /// the response of a v3 request
    #[allow(clippy::too_many_arguments)]
    fn respond_v3(&mut self, os: &mut OsApi, d: &Dgram, m: &V3, user: &UsmUser, level: Level, me: &[u8], mut p: Pdu) {
        let mut bytes = self.seal_v3(os, m, Some(user), level, me, ScopedPdu { context_engine: Vec::from(me), context_name: Vec::new(), pdu: p.clone() });
        if bytes.len() > MAX_MSG {
            p.error_status = snmp::E_TOO_BIG;
            p.error_index = 0;
            p.varbinds = Vec::new();
            bytes = self.seal_v3(os, m, Some(user), level, me, ScopedPdu { context_engine: Vec::from(me), context_name: Vec::new(), pdu: p.clone() });
        }
        if p.error_status != 0 {
            self.errors += 1;
        }
        match snmp::v1_error(p.error_status) {
            snmp::E_TOO_BIG => self.count(C_OUT_TOO_BIGS),
            snmp::E_GEN_ERR => self.count(C_OUT_GEN_ERRS),
            _ => {}
        }
        self.count(C_OUT_PKTS);
        self.count(C_OUT_GET_RESPONSES);
        self.responses += 1;
        os.note(Note::Snmp(SNote::Response { to: d.src, id: p.request_id, status: p.error_status, index: p.error_index, vars: p.varbinds.len().min(u16::MAX as usize) as u16 }));
        os.send_from(d.dst, self.cfg.port, d.src, d.sport, &bytes);
    }

    /// a wrong digest: the log line and an authenticationFailure where it is on
    fn v3_auth_failure(&mut self, os: &mut OsApi, from: IpAddr, user: &str) {
        if self.ios {
            let ni = os.ni;
            os.eng.ios_log(ni, &format!("%SNMP-3-AUTHFAIL: refused a request from {}: wrong digest for user {}", from, user));
        } else {
            os.log(SEV_WARN, "snmpd", &format!("refused a request from {}: wrong digest for user {}", from, user));
        }
        self.notify(os, Notif::AuthFailure);
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
        self.boots = self.boots.saturating_add(1);
        let me = os.eng.snmp_engine_id(os.ni, &self.cfg.engine_id);
        for u in &mut self.cfg.users {
            u.localize(&me);
        }
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
        if self.cfg.communities.is_empty() && self.cfg.users.is_empty() {
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
        512 + self.cfg.views.len() * 96 + self.cfg.groups.len() * 96 + self.cfg.users.len() * 160 + self.cfg.communities.len() * 96 + self.cfg.hosts.len() * 96 + self.changes.len() * 16 + self.pending.iter().map(|p| 96 + p.bytes.len()).sum::<usize>() + self.cfg.location.len() + self.cfg.contact.len()
    }
}

// ---------------------------------------------------------------- the engine side

impl SnmpCfg {
    /// a user of a Linux agent's settings, with a group and a view of their own
    fn linux_user(&mut self, x: &Json) -> Result<(), Msg> {
        let name = text_of(x, "name").unwrap_or_default();
        if name.is_empty() || name.contains(char::is_whitespace) {
            return Err(msg!("Ein SNMPv3-Benutzer braucht „name“ ohne Leerzeichen."));
        }
        let level = match text_of(x, "level") {
            Some(l) => Level::parse(&l).ok_or_else(|| msg!("„{}“ ist keine Stufe (noAuthNoPriv, authNoPriv, authPriv).", l))?,
            None => Level::NoAuthNoPriv,
        };
        let pw = |k: &str| -> Result<String, Msg> {
            let p = text_of(x, k).unwrap_or_default();
            if p.len() < 8 {
                return Err(msg!("„{}“ von {} braucht mindestens 8 Zeichen.", k, name));
            }
            Ok(p)
        };
        let auth = if level >= Level::AuthNoPriv {
            let a = text_of(x, "auth").unwrap_or_else(|| String::from("SHA"));
            let proto = AuthProto::parse(&a).ok_or_else(|| msg!("„{}“ ist kein Verfahren (MD5, SHA, SHA-256, SHA-384, SHA-512).", a))?;
            Some((proto, UserKey::Password(pw("auth_password")?)))
        } else {
            None
        };
        let privacy = if level == Level::AuthPriv {
            let a = text_of(x, "priv").unwrap_or_else(|| String::from("AES"));
            let proto = PrivProto::parse(&a).ok_or_else(|| msg!("„{}“ ist kein Verfahren (AES; DES gibt es hier nicht).", a))?;
            Some((proto, UserKey::Password(pw("priv_password")?)))
        } else {
            None
        };
        let rw = match text_of(x, "access").as_deref() {
            None | Some("ro") => false,
            Some("rw") => true,
            Some(o) => return Err(msg!("„{}“ ist kein Zugriff (ro oder rw).", o)),
        };
        let group = format!("u:{}", name);
        let mut view = None;
        if let Some(sub) = text_of(x, "subtree") {
            let oid = mib::lookup(&sub).ok_or_else(|| msg!("„{}“ ist kein Objekt.", sub))?;
            let v = format!("v:{}", name);
            self.views.push(ViewRow { name: v.clone(), oid, included: true });
            view = Some(v);
        }
        let all = String::from("v1default");
        let (read, write) = (view.clone(), if rw { Some(view.unwrap_or(all)) } else { None });
        self.groups.push(GroupRow { name: group.clone(), level, read, write });
        self.users.retain(|u| u.name != name);
        self.users.push(UsmUser { name, group, auth, privacy });
        Ok(())
    }
}

impl Engine {
    /// the engine ID of a device's agent: the one configured, else format 3 of the first
    /// interface's MAC (of the device id where there is no interface)
    pub(crate) fn snmp_engine_id(&self, ni: usize, set: &Option<Vec<u8>>) -> Vec<u8> {
        if let Some(e) = set {
            return e.clone();
        }
        let mac = self.stack(ni).and_then(|s| s.ifaces.iter().find(|f| !f.hidden).map(|f| f.mac));
        let dev = self.dev_of(ni);
        v3::engine_id_mac(mac.unwrap_or(netpkt::Mac([2, 0, 0, 0, (dev >> 8) as u8, dev as u8])))
    }

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
            if matches!(set, SnmpSet::Off | SnmpSet::NoCommunity { .. } | SnmpSet::NoHost { .. } | SnmpSet::Notify { on: false, .. } | SnmpSet::Location(None) | SnmpSet::Contact(None) | SnmpSet::EngineId(None) | SnmpSet::NoView { .. } | SnmpSet::NoGroup { .. } | SnmpSet::NoUser { .. }) {
                return Ok(());
            }
            self.add_service(dev, IOS_NAME, Box::new(SnmpAgent::new(true)), true)?;
        }
        let (ni, id) = self.snmp_slot(dev).ok_or(ActErr::NoIp(dev))?;
        if set == SnmpSet::Off {
            let name = self.os(ni).and_then(|o| o.svcs.iter().find(|s| s.id == id)).map(|s| s.name.clone()).unwrap_or_default();
            return self.remove_service(dev, &name);
        }
        // a user's keys are for the engine the device has now
        let engine = {
            let set_id = match &set {
                SnmpSet::EngineId(e) => e.clone(),
                _ => self.snmp_agent(dev).and_then(|a| a.cfg.engine_id.clone()),
            };
            self.snmp_engine_id(ni, &set_id)
        };
        let Some(a) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.id == id)).and_then(|s| s.svc.as_mut()).and_then(|x| x.as_any_mut().downcast_mut::<SnmpAgent>()) else {
            return Err(ActErr::NoIp(dev));
        };
        let c = &mut a.cfg;
        match set {
            SnmpSet::EngineId(e) => c.engine_id = e,
            SnmpSet::View(v) => {
                c.views.retain(|x| !(x.name == v.name && x.oid == v.oid));
                c.views.push(v);
            }
            SnmpSet::NoView { name, oid } => c.views.retain(|x| !(x.name == name && oid.as_ref().is_none_or(|o| *o == x.oid))),
            SnmpSet::Group(g) => {
                c.groups.retain(|x| !(x.name == g.name && x.level == g.level));
                c.groups.push(g);
            }
            SnmpSet::NoGroup { name, level } => c.groups.retain(|x| !(x.name == name && x.level == level)),
            SnmpSet::User(mut u) => {
                u.localize(&engine);
                c.users.retain(|x| x.name != u.name);
                c.users.push(u);
            }
            SnmpSet::NoUser { name } => c.users.retain(|x| x.name != name),
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
