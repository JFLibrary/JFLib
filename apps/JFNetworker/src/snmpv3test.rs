// snmpv3test -- checks of SNMPv3 in the simulator (group "snmpv3" of --selftest: jfn_sim::snmpd's
// USM and VACM, cli/snmp.rs's commands and managers): get, walk, bulk and set at every level, the
// discovery, each Report (unknown engine ID, unknown user, a level without keys, a wrong digest,
// decryption errors, the time window, unknown contexts), the time window after a reboot, views and
// groups, the IOS-like commands with the running configuration and a boot from it, show snmp user,
// group, view, engineID, snmpd on a Linux host, what is on the wire, the debug lines, determinism
//
// The LAN (device ids in order): R1 (JF-R4, 10.0.0.1/24, the users of R1_V3), SW1 (switch, Vlan1
// 10.0.0.2, a user), SRV1 (server-1u, 10.0.0.10, snmpd with users), PC1 (PC, 10.0.0.20: the manager).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value as Json;
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_sim::os::{Metric, OsApi, Service, SvcEvent};
use jfn_sim::snmptrace::SNote;
use jfn_sim::{Action, Engine, Note, Report, MS, SEC};
use netpkt::ber::Oid;
use netpkt::snmp::{self, Kind, Pdu, VarBind};
use netpkt::snmpv3::{self as v3, AuthProto, Level, Scoped, ScopedPdu, Usm, V3};
use netpkt::{decode, IpAddr};

use crate::selftest::T;

const BUDGET: usize = 4_000_000;
const R1: u32 = 1;
const SW1: u32 = 2;
const SRV1: u32 = 3;
const PC1: u32 = 4;

fn s(x: &str) -> String {
    String::from(x)
}

fn def(kind: &str, name: &str, config: &str) -> ServiceDef {
    ServiceDef { kind: s(kind), name: s(name), disabled: false, config: agent_core::json::parse(config.as_bytes()).unwrap_or(Json::Null) }
}

const R1_V3: &str = "snmp-server view NURSYS 1.3.6.1.2.1.1 included\nsnmp-server view OHNECONTACT 1.3.6.1.2.1.1 included\nsnmp-server view OHNECONTACT 1.3.6.1.2.1.1.4 excluded\nsnmp-server group OFFEN v3 noauth read NURSYS\nsnmp-server group OFFENW v3 noauth read NURSYS write NURSYS\nsnmp-server user otto OFFENW v3\nsnmp-server group SIGNIERT v3 auth read NURSYS write NURSYS\nsnmp-server group GEHEIM v3 priv\nsnmp-server group ALLES v3 priv write NURSYS\nsnmp-server group OHNE v3 auth read OHNECONTACT\nsnmp-server user anna OFFEN v3\nsnmp-server user bert SIGNIERT v3 auth sha bertspass\nsnmp-server user carl GEHEIM v3 auth sha carlspass priv aes 128 carlsgeheim\nsnmp-server user dora ALLES v3 auth md5 dorasauth priv aes 128 dorasgeheim\nsnmp-server user emil OHNE v3 auth sha256 emilspass\nsnmp-server user fred KEINE v3 auth sha fredspass\nsnmp-server user gina GEHEIM v3 auth sha ginaspass\n";
const SRV_SNMPD: &str = r#"{"communities": [{"name": "public"}], "users": [{"name": "root1", "level": "authPriv", "auth": "SHA-256", "auth_password": "rootauth1", "priv": "AES", "priv_password": "rootpriv1", "access": "rw"}, {"name": "leser", "level": "authNoPriv", "auth": "SHA", "auth_password": "leserauth", "access": "ro", "subtree": "1.3.6.1.2.1.1"}]}"#;

fn r1_with(lines: &str) -> String {
    format!("hostname R1\n!\ninterface GigabitEthernet0/0/0\n ip address 10.0.0.1 255.255.255.0\n no shutdown\n!\n{}end\n", lines)
}

fn project(r1: &str) -> Project {
    let mut p = Project::new("snmpv3", 19);
    for (i, m) in ["jf-r4", "jf-s24p", "server-1u", "pc"].iter().enumerate() {
        let _ = p.add_device(m, i as i32 * 120, 0);
    }
    let pr = |dev: u32, port: &str| PortRef { dev, port: s(port) };
    let _ = p.connect(&pr(R1, "gi0/0/0"), &pr(SW1, "gi1/0/1"), None);
    let _ = p.connect(&pr(SRV1, "eth0"), &pr(SW1, "gi1/0/2"), None);
    let _ = p.connect(&pr(PC1, "eth0"), &pr(SW1, "gi1/0/3"), None);
    let cfg = [
        (R1, "R1", s(r1)),
        (SW1, "SW1", s("hostname SW1\n!\ninterface Vlan1\n ip address 10.0.0.2 255.255.255.0\n no shutdown\n!\nsnmp-server group G v3 auth\nsnmp-server user sven G v3 auth sha svenspass\n!\nend\n")),
        (SRV1, "SRV1", s("hostname SRV1\nip addr add 10.0.0.10/24 dev eth0\n")),
        (PC1, "PC1", s("hostname PC1\nip addr add 10.0.0.20/24 dev eth0\n")),
    ];
    for (id, name, c) in cfg {
        if let Some(d) = p.device_mut(id) {
            d.name = s(name);
            d.startup_config = c;
        }
    }
    if let Some(d) = p.device_mut(SRV1) {
        d.services = alloc::vec![def("snmpd", "snmpd", SRV_SNMPD)];
    }
    p
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
}

fn run_for(e: &mut Engine, d: u64) {
    let t = e.now() + d;
    e.run_until(t, BUDGET);
}

fn cli(e: &mut Engine, dev: u32, lines: &[&str], d: u64) -> String {
    let _ = e.take_reports();
    let mut o = String::new();
    for l in lines {
        let _ = e.cli_line(dev, l);
        run_for(e, d);
        for r in e.take_reports() {
            if let Report::CliOut { dev: x, bytes, .. } = r
                && x == dev
            {
                o.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
    }
    o
}

fn notes(e: &Engine, dev: u32) -> Vec<SNote> {
    e.trace.events().filter(|x| x.dev == dev).filter_map(|x| if let Note::Snmp(n) = &x.note { Some(n.clone()) } else { None }).collect()
}

/// the report's notes of an agent
fn reports(e: &Engine, dev: u32) -> Vec<String> {
    notes(e, dev).into_iter().filter_map(|n| if let SNote::V3Report { what, .. } = n { Some(what) } else { None }).collect()
}

/// a manager for the tests: it sends what it is given and keeps what comes back
#[derive(Clone, Debug)]
struct Raw {
    to: IpAddr,
    plan: Vec<Vec<u8>>,
    port: u16,
    got: Vec<Vec<u8>>,
}

impl Service for Raw {
    fn kind(&self) -> &'static str {
        "raw-snmp3"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.port = os.bind(0, None).unwrap_or(0);
        for b in core::mem::take(&mut self.plan) {
            os.send(self.port, self.to, snmp::PORT, &b);
        }
    }
    fn on(&mut self, ev: SvcEvent, _os: &mut OsApi) {
        if let SvcEvent::Udp(d) = ev {
            self.got.push(d.data);
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

fn ask(e: &mut Engine, to: &str, plan: Vec<Vec<u8>>) -> Vec<V3> {
    let _ = e.remove_service(PC1, "raw3");
    let raw = Raw { to: IpAddr::parse(to).unwrap_or(IpAddr::V4(netpkt::Ipv4::UNSPECIFIED)), plan, port: 0, got: Vec::new() };
    let _ = e.add_service(PC1, "raw3", Box::new(raw), true);
    run_for(e, 2 * SEC);
    e.service::<Raw>(PC1, "raw3").map(|r| r.got.iter().filter_map(|b| V3::parse(b).ok()).collect()).unwrap_or_default()
}

fn sys_name(id: i32) -> Pdu {
    Pdu::new(Kind::Get, id, alloc::vec![VarBind::null(Oid::new(&[1, 3, 6, 1, 2, 1, 1, 5, 0]))])
}

/// a message by hand
fn msg(engine_id: &[u8], boots: i32, time: i32, user: &str, flags: u8, auth: Option<(AuthProto, &str)>, context: &str, id: i32) -> Vec<u8> {
    let scoped = ScopedPdu { context_engine: Vec::from(engine_id), context_name: Vec::from(context.as_bytes()), pdu: sys_name(id) };
    let mut usm = Usm { engine_id: Vec::from(engine_id), boots, time, user: Vec::from(user.as_bytes()), auth: Vec::new(), privacy: Vec::new() };
    if let Some((a, _)) = auth {
        usm.auth = alloc::vec![0u8; a.tag_len()];
    }
    let m = V3 { id, max_size: 65507, flags, model: 3, usm, scoped: Scoped::Plain(scoped) };
    let mut b = m.to_bytes();
    if let Some((a, pw)) = auth {
        let k = v3::localize(a, &v3::password_to_key(a, pw.as_bytes()), engine_id);
        v3::sign(&mut b, a, &k);
    }
    b
}

/// the counter a Report names
fn counter(m: &V3) -> &'static str {
    m.plain().and_then(|p| p.pdu.varbinds.first()).map_or("?", |v| v3::report_name(v.oid.arcs()))
}

pub(crate) fn run(t: &mut T) {
    levels(t);
    reports_all(t);
    views(t);
    ios_config(t);
    linux(t);
    tools(t);
    wire(t);
    determinism(t);
}

const CARL: &str = "-v3 -u carl -l authPriv -a SHA -A carlspass -x AES -X carlsgeheim";

/// get, walk, bulk and set at each level, the discovery
fn levels(t: &mut T) {
    let mut e = engine(&project(&r1_with(R1_V3)));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &["snmpget -v3 -u anna 10.0.0.1 sysName.0"], SEC);
    t.eq("noAuthNoPriv", o, s("SNMPv2-MIB::sysName.0 = STRING: R1\n"));
    t.check("the discovery", notes(&e, PC1).iter().any(|n| matches!(n, SNote::V3Discovered { engine, .. } if engine.starts_with("80007ed903"))));
    t.eq("the agent's report of it", reports(&e, R1), alloc::vec![s("usmStatsUnknownEngineIDs")]);
    t.check("its note names the user and level", notes(&e, R1).iter().any(|n| matches!(n, SNote::V3Request { user, level: Level::NoAuthNoPriv, kind: Kind::Get, .. } if user == "anna")));
    let o = cli(&mut e, PC1, &["snmpget -v3 -u bert -l authNoPriv -a SHA -A bertspass 10.0.0.1 sysLocation.0 sysContact.0"], SEC);
    t.check("authNoPriv", o.contains("sysLocation.0") && o.contains("sysContact.0"));
    let o = cli(&mut e, PC1, &[&format!("snmpget {} 10.0.0.1 sysName.0", CARL)], SEC);
    t.eq("authPriv", o, s("SNMPv2-MIB::sysName.0 = STRING: R1\n"));
    let o = cli(&mut e, PC1, &[&format!("snmpgetnext {} 10.0.0.1 sysName.0", CARL)], SEC);
    t.check("GetNext", o.starts_with("SNMPv2-MIB::sysLocation.0"));
    let o = cli(&mut e, PC1, &[&format!("snmpwalk {} 10.0.0.1 system", CARL)], SEC);
    t.check("a walk of system", o.contains("sysDescr.0") && o.contains("sysServices.0"));
    let o = cli(&mut e, PC1, &[&format!("snmpbulkwalk {} 10.0.0.1 ifDescr", CARL)], SEC);
    t.check("a bulk walk", o.contains("IF-MIB::ifDescr.1") && o.contains("GigabitEthernet0/0/0"));
    let o = cli(&mut e, PC1, &["snmpwalk -v3 -u anna 10.0.0.1"], SEC);
    t.check("a walk in a small view ends with it", o.contains("sysName.0") && !o.contains("ifDescr"));
    // Set
    let o = cli(&mut e, PC1, &["snmpset -v3 -u bert -l authNoPriv -a SHA -A bertspass 10.0.0.1 sysLocation.0 s Halle"], SEC);
    t.eq("Set, signed", o, s("SNMPv2-MIB::sysLocation.0 = STRING: Halle\n"));
    t.check("in the configuration", e.running_config(R1).contains("snmp-server location Halle"));
    let o = cli(&mut e, PC1, &["snmpset -v3 -u dora -l authPriv -a MD5 -A dorasauth -x AES -X dorasgeheim 10.0.0.1 sysLocation.0 s Keller"], SEC);
    t.eq("Set, encrypted (MD5)", o, s("SNMPv2-MIB::sysLocation.0 = STRING: Keller\n"));
    let o = cli(&mut e, PC1, &["snmpset -v3 -u otto 10.0.0.1 sysLocation.0 s Tor"], SEC);
    t.eq("Set, noAuthNoPriv", o, s("SNMPv2-MIB::sysLocation.0 = STRING: Tor\n"));
    let o = cli(&mut e, PC1, &["snmpset -v3 -u anna 10.0.0.1 sysLocation.0 s Dach"], SEC);
    t.check("no write view: noAccess", o.contains("noAccess"));
    let o = cli(&mut e, PC1, &[&format!("snmpset {} 10.0.0.1 sysLocation.0 s Dach", CARL)], SEC);
    t.check("a group without a write view", o.contains("noAccess"));
    let o = cli(&mut e, PC1, &["snmpget -v3 -u emil -l authNoPriv -a SHA-256 -A emilspass 10.0.0.1 sysName.0"], SEC);
    t.check("HMAC of SHA-2", o.contains("STRING: R1"));
    // the switch, by its VLAN interface
    let o = cli(&mut e, PC1, &["snmpget -v3 -u sven -l authNoPriv -a SHA -A svenspass 10.0.0.2 sysName.0"], SEC);
    t.check("the switch too", o.contains("STRING: SW1"));
}

/// every Report
fn reports_all(t: &mut T) {
    let mut e = engine(&project(&r1_with(R1_V3)));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &["snmpget -v3 -u niemand 10.0.0.1 sysName.0"], SEC);
    t.check("unknown user", o.contains("the agent knows no user 'niemand'"));
    let o = cli(&mut e, PC1, &["snmpget -v3 -u anna -l authNoPriv -a SHA -A annaspass 10.0.0.1 sysName.0"], SEC);
    t.check("a level the user has no keys for", o.contains("the agent does not take the level authNoPriv for user 'anna'"));
    let o = cli(&mut e, PC1, &["snmpget -v3 -u bert -l authNoPriv -a SHA -A falschfalsch 10.0.0.1 sysName.0"], SEC);
    t.check("a wrong password", o.contains("the agent refused the signature"));
    let o = cli(&mut e, PC1, &["snmpget -v3 -u bert -l authNoPriv -a MD5 -A bertspass 10.0.0.1 sysName.0"], SEC);
    t.check("a wrong protocol", o.contains("the agent refused the signature"));
    let o = cli(&mut e, PC1, &["snmpget -v3 -u carl -l authPriv -a SHA -A carlspass -x AES -X falschfalsch 10.0.0.1 sysName.0"], SEC);
    t.check("a wrong privacy password", o.contains("the agent could not read the request"));
    let o = cli(&mut e, PC1, &["snmpget -v3 -u anna -e 80007ed903ffffffffff 10.0.0.1 sysName.0"], SEC);
    t.check("another engine ID", o.contains("the agent has another engine ID"));
    let o = cli(&mut e, PC1, &["snmpget -v3 -u anna -n fremd 10.0.0.1 sysName.0"], SEC);
    t.check("a context it does not have", o.contains("the agent has no such context"));
    let r = reports(&e, R1);
    for w in ["usmStatsUnknownUserNames", "usmStatsUnsupportedSecLevels", "usmStatsWrongDigests", "usmStatsDecryptionErrors", "usmStatsUnknownEngineIDs", "snmpUnknownContexts"] {
        t.check(&format!("the note {}", w), r.iter().any(|x| x == w));
    }
    t.check("the tool's note", notes(&e, PC1).iter().any(|n| matches!(n, SNote::V3Refused { what, .. } if what == "usmStatsWrongDigests")));
    // by hand: the engine's clock
    let a = ask(&mut e, "10.0.0.1", alloc::vec![msg(&[], 0, 0, "", v3::F_REPORTABLE, None, "", 1)]);
    let Some(rep) = a.first().cloned() else {
        t.check("the discovery by hand", false);
        return;
    };
    t.eq("it is a report", counter(&rep), "usmStatsUnknownEngineIDs");
    t.check("noAuthNoPriv, without a user", rep.flags == 0 && rep.usm.user.is_empty() && rep.usm.auth.is_empty());
    let me = rep.usm.engine_id.clone();
    t.check("format 3 of a MAC", me.len() == 11 && me[..5] == [0x80, 0x00, 0x7E, 0xD9, 3]);
    let (boots, time) = (rep.usm.boots, rep.usm.time);
    t.eq("the first start", boots, 1);
    t.check("its time in seconds", time >= 1);
    let good = msg(&me, boots, time, "bert", v3::F_AUTH | v3::F_REPORTABLE, Some((AuthProto::Sha1, "bertspass")), "", 2);
    let a = ask(&mut e, "10.0.0.1", alloc::vec![good]);
    t.check("a signed request with its clock", a.first().is_some_and(|m| m.flags == v3::F_AUTH && m.plain().is_some_and(|p| p.pdu.kind == Kind::Response)));
    t.check("the answer is signed for that engine", a.first().is_some_and(|m| m.usm.engine_id == me && m.usm.auth.len() == 12));
    let late = msg(&me, boots, time + 200, "bert", v3::F_AUTH | v3::F_REPORTABLE, Some((AuthProto::Sha1, "bertspass")), "", 3);
    let a = ask(&mut e, "10.0.0.1", alloc::vec![late]);
    t.eq("150 s off: not in time", a.first().map(counter), Some("usmStatsNotInTimeWindows"));
    t.check("that report is signed", a.first().is_some_and(|m| m.flags == v3::F_AUTH && m.usm.auth.len() == 12));
    let old = msg(&me, boots + 1, time, "bert", v3::F_AUTH | v3::F_REPORTABLE, Some((AuthProto::Sha1, "bertspass")), "", 4);
    t.eq("other boots: not in time", ask(&mut e, "10.0.0.1", alloc::vec![old]).first().map(counter), Some("usmStatsNotInTimeWindows"));
    let noreq = msg(&me, boots, time, "niemand", 0, None, "", 5);
    t.eq("no report flag, no report", ask(&mut e, "10.0.0.1", alloc::vec![noreq]).len(), 0);
    let priv_only = msg(&me, boots, time, "bert", v3::F_PRIV | v3::F_REPORTABLE, None, "", 6);
    t.eq("privacy without authentication is dropped", ask(&mut e, "10.0.0.1", alloc::vec![priv_only]).len(), 0);
    let plain_priv = msg(&me, boots, time + 3, "carl", v3::F_AUTH | v3::F_PRIV | v3::F_REPORTABLE, Some((AuthProto::Sha1, "carlspass")), "", 7);
    t.eq("a plain PDU at authPriv", ask(&mut e, "10.0.0.1", alloc::vec![plain_priv]).first().map(counter), Some("usmStatsDecryptionErrors"));
    // a reboot: the engine counts, an old message is out of its time; the tool follows
    let before = msg(&me, boots, time + 5, "bert", v3::F_AUTH | v3::F_REPORTABLE, Some((AuthProto::Sha1, "bertspass")), "", 8);
    let _ = e.apply(Action::Power { dev: R1, on: false });
    run_for(&mut e, SEC);
    let _ = e.apply(Action::Power { dev: R1, on: true });
    run_for(&mut e, 5 * SEC);
    // (the router's first answer waits for ARP: a tool's retry fills that in)
    let _ = cli(&mut e, PC1, &["snmpget -v3 -u anna 10.0.0.1 sysName.0"], 3 * SEC);
    let a = ask(&mut e, "10.0.0.1", alloc::vec![before]);
    t.eq("after a reboot: not in time", a.first().map(counter), Some("usmStatsNotInTimeWindows"));
    t.eq("boots counted on", a.first().map(|m| m.usm.boots), Some(2));
    t.check("the time starts again", a.first().is_some_and(|m| m.usm.time < 30));
    let o = cli(&mut e, PC1, &[&format!("snmpget -v3 -u bert -l authNoPriv -a SHA -A bertspass -e {} -Z 1,0 10.0.0.1 sysName.0", v3::engine_id_text(&me))], SEC);
    t.eq("an old clock: the tool takes the agent's and asks again", o, s("SNMPv2-MIB::sysName.0 = STRING: R1\n"));
    let o = cli(&mut e, PC1, &["snmpget -v3 -u bert -l authNoPriv -a SHA -A bertspass 10.0.0.1 sysName.0"], SEC);
    t.check("discovery works again", o.contains("STRING: R1"));
    // a wrong signature counts as an authentication failure
    let mut e = engine(&project(&r1_with(&format!("{}snmp-server enable traps snmp authentication\nsnmp-server host 10.0.0.20 version 2c public\n", R1_V3))));
    run_for(&mut e, SEC);
    let _ = e.take_reports();
    let _ = e.cli_line(PC1, "snmpget -v3 -u bert -l authNoPriv -a SHA -A falschfalsch 10.0.0.1 sysName.0");
    run_for(&mut e, SEC);
    let console: Vec<String> = e.take_reports().into_iter().filter_map(|r| if let Report::CliOut { dev: R1, bytes, .. } = r { Some(String::from_utf8_lossy(&bytes).into_owned()) } else { None }).collect();
    t.check("the console says it", console.iter().any(|l| l.contains("%SNMP-3-AUTHFAIL") && l.contains("wrong digest for user bert")));
    t.check("and a notification goes out", notes(&e, R1).iter().any(|n| matches!(n, SNote::Notify { what, .. } if what == "authenticationFailure")));
}

/// VACM
fn views(t: &mut T) {
    let mut e = engine(&project(&r1_with(R1_V3)));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &["snmpget -v3 -u anna 10.0.0.1 sysName.0 ifNumber.0"], SEC);
    t.check("outside the view: noSuchObject", o.contains("sysName.0 = STRING: R1") && o.contains("IF-MIB::ifNumber.0 = No Such Object"));
    let e1 = "-v3 -u emil -l authNoPriv -a SHA-256 -A emilspass";
    let o = cli(&mut e, PC1, &[&format!("snmpget {} 10.0.0.1 sysName.0 sysContact.0", e1)], SEC);
    t.check("an excluded subtree", o.contains("STRING: R1") && o.contains("sysContact.0 = No Such Object"));
    let o = cli(&mut e, PC1, &[&format!("snmpwalk {} 10.0.0.1 system", e1)], SEC);
    t.check("a walk goes past it", o.contains("sysName.0") && !o.contains("sysContact"));
    let o = cli(&mut e, PC1, &[&format!("snmpgetnext {} 10.0.0.1 sysUpTime.0", e1)], SEC);
    t.check("GetNext goes past it", o.contains("sysName.0"));
    let o = cli(&mut e, PC1, &["snmpget -v3 -u fred -l authNoPriv -a SHA -A fredspass 10.0.0.1 sysName.0"], SEC);
    t.check("a group that is not there: authorizationError", o.contains("authorizationError"));
    let o = cli(&mut e, PC1, &["snmpget -v3 -u gina -l authNoPriv -a SHA -A ginaspass 10.0.0.1 sysName.0"], SEC);
    t.check("a group of a higher level: authorizationError", o.contains("authorizationError"));
    let o = cli(&mut e, PC1, &[&format!("snmpget {} 10.0.0.1 ifNumber.0", CARL)], SEC);
    t.check("a group without a read view reads everything", o.contains("INTEGER:"));
    let o = cli(&mut e, PC1, &["snmpget -v3 -u bert -l authNoPriv -a SHA -A bertspass 10.0.0.1 ifAdminStatus.1"], SEC);
    t.check("write view but read view too", o.contains("No Such Object"));
    let o = cli(&mut e, PC1, &["snmpset -v3 -u bert -l authNoPriv -a SHA -A bertspass 10.0.0.1 ifAlias.1 s x"], SEC);
    t.check("Set outside its view", o.contains("noAccess"));
}

/// the commands, the running configuration, a boot from it
fn ios_config(t: &mut T) {
    let mut e = engine(&project(&r1_with("")));
    run_for(&mut e, SEC);
    let o = cli(&mut e, R1, &["enable", "show snmp user"], 0);
    t.check("no agent yet", o.contains("%SNMP agent not enabled"));
    let o = cli(&mut e, R1, &["configure terminal", "snmp-server engineID local 80007ed90401020304", "snmp-server view V1 system included", "snmp-server group G1 v3 priv read V1 write V1", "snmp-server user u1 G1 v3 auth sha geheim123 priv aes 128 privat123", "snmp-server user u2 G1 v3 auth sha kurz", "snmp-server engineID local 12", "snmp-server view V1 keinobjekt included", "end"], 0);
    t.check("what is refused", o.contains("% A password needs at least 8 characters") && o.contains("% An engine ID has 5 to 32 bytes in hexadecimal") && o.contains("% No object of that name"));
    run_for(&mut e, SEC);
    let run = e.running_config(R1);
    t.check("the configuration", run.contains("snmp-server engineID local 80007ed90401020304\n") && run.contains("snmp-server view V1 1.3.6.1.2.1.1 included\n") && run.contains("snmp-server group G1 v3 priv read V1 write V1\n"));
    t.check("a user with localized keys, no password", run.contains("snmp-server user u1 G1 v3 encrypted auth sha ") && run.contains(" priv aes 128 ") && !run.contains("geheim123") && !run.contains("privat123"));
    let o = cli(&mut e, R1, &["show snmp user"], 0);
    t.check("show snmp user", o.contains("u1") && o.contains("G1") && o.contains("SHA") && o.contains("AES") && !o.contains("u2"));
    let o = cli(&mut e, R1, &["show snmp group"], 0);
    t.check("show snmp group", o.contains("G1") && o.contains("authPriv") && o.contains("V1"));
    let o = cli(&mut e, R1, &["show snmp engineID"], 0);
    t.check("show snmp engineID", o.contains("80007ed90401020304") && o.contains("Starts so far: 1"));
    t.check("show snmp view", cli(&mut e, R1, &["show snmp view"], 0).contains("1.3.6.1.2.1.1"));
    t.check("show snmp says it", cli(&mut e, R1, &["show snmp"], 0).contains("versions 1, 2c and 3"));
    let ask = "snmpget -v3 -u u1 -l authPriv -a SHA -A geheim123 -x AES -X privat123 10.0.0.1 sysName.0";
    t.check("it answers", cli(&mut e, PC1, &[ask], SEC).contains("STRING: R1"));
    // the running configuration as the startup configuration
    let saved = e.running_config(R1);
    let mut e2 = engine(&project(&saved));
    run_for(&mut e2, SEC);
    t.eq("a boot from it has no errors", e2.boot_errors(R1).len(), 0);
    t.check("the same user answers", cli(&mut e2, PC1, &[ask], SEC).contains("STRING: R1"));
    t.eq("and writes the same lines", e2.running_config(R1).lines().filter(|l| l.starts_with("snmp-server")).collect::<Vec<_>>(), saved.lines().filter(|l| l.starts_with("snmp-server")).collect::<Vec<_>>());
    // the no-forms
    let o = cli(&mut e, R1, &["configure terminal", "no snmp-server user u1 G1 v3", "no snmp-server group G1 v3 priv", "no snmp-server view V1", "no snmp-server engineID local", "end"], 0);
    t.check("no complaints", !o.contains("% "));
    let run = e.running_config(R1);
    t.check("they went", !run.contains("snmp-server user") && !run.contains("snmp-server group") && !run.contains("snmp-server view") && !run.contains("engineID"));
    t.check("the user is gone", cli(&mut e, PC1, &[ask], SEC).contains("the agent knows no user"));
    // debug
    let o = cli(&mut e, R1, &["debug snmp packets"], 0);
    t.check("debug", o.contains("on"));
    let _ = e.take_reports();
    let _ = e.cli_line(PC1, "snmpget -v3 -u u1 10.0.0.1 sysName.0");
    run_for(&mut e, SEC);
    let console: String = e.take_reports().into_iter().filter_map(|r| if let Report::CliOut { dev: R1, bytes, .. } = r { Some(String::from_utf8_lossy(&bytes).into_owned()) } else { None }).collect();
    t.check("debug lines of the reports", console.contains("SNMP: Report usmStatsUnknownEngineIDs to 10.0.0.20") && console.contains("SNMP: Report usmStatsUnknownUserNames to 10.0.0.20"));
}

/// snmpd on a Linux host
fn linux(t: &mut T) {
    let mut e = engine(&project(&r1_with("")));
    run_for(&mut e, SEC);
    let r = "-v3 -u root1 -l authPriv -a SHA-256 -A rootauth1 -x AES -X rootpriv1";
    let o = cli(&mut e, PC1, &[&format!("snmpget {} 10.0.0.10 sysName.0 ifNumber.0", r)], SEC);
    t.check("a user with everything", o.contains("STRING: SRV1") && o.contains("INTEGER:"));
    let o = cli(&mut e, PC1, &[&format!("snmpset {} 10.0.0.10 sysLocation.0 s Keller", r)], SEC);
    t.check("rw", o.contains("STRING: Keller"));
    let l = "-v3 -u leser -l authNoPriv -a SHA -A leserauth";
    let o = cli(&mut e, PC1, &[&format!("snmpget {} 10.0.0.10 sysName.0 ifNumber.0", l)], SEC);
    t.check("a subtree of its own", o.contains("STRING: SRV1") && o.contains("No Such Object"));
    let o = cli(&mut e, PC1, &[&format!("snmpset {} 10.0.0.10 sysLocation.0 s Dach", l)], SEC);
    t.check("ro", o.contains("noAccess"));
    let o = cli(&mut e, PC1, &["snmpget -v2c -c public 10.0.0.10 sysName.0"], SEC);
    t.check("version 2c beside it", o.contains("STRING: SRV1"));
    // bad settings
    let mut p = project(&r1_with(""));
    if let Some(d) = p.device_mut(SRV1) {
        d.services = alloc::vec![def("snmpd", "snmpd", r#"{"users": [{"name": "x", "level": "authNoPriv", "auth_password": "kurz"}]}"#)];
    }
    t.check("a short password is refused", engine(&p).services(SRV1).is_empty());
    let mut p = project(&r1_with(""));
    if let Some(d) = p.device_mut(SRV1) {
        d.services = alloc::vec![def("snmpd", "snmpd", r#"{"users": [{"name": "x", "level": "authPriv", "auth_password": "langgenug1", "priv": "DES", "priv_password": "langgenug2"}]}"#)];
    }
    t.check("DES is not here", engine(&p).services(SRV1).is_empty());
}

/// what the tools ask for
fn tools(t: &mut T) {
    let mut e = engine(&project(&r1_with(R1_V3)));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &["snmpget -v3 10.0.0.1 sysName.0"], 0);
    t.check("no user", o.contains("no user given"));
    let o = cli(&mut e, PC1, &["snmpget -v3 -u bert -l authNoPriv 10.0.0.1 sysName.0"], 0);
    t.check("no password", o.contains("needs a password of at least 8 characters"));
    let o = cli(&mut e, PC1, &["snmpget -v3 -u carl -l authPriv -A carlspass 10.0.0.1 sysName.0"], 0);
    t.check("no privacy password", o.contains("needs a privacy password"));
    let o = cli(&mut e, PC1, &["snmpget -v3 -u x -l gut 10.0.0.1 sysName.0"], 0);
    t.check("a level that is none", o.contains("is no level"));
    let o = cli(&mut e, PC1, &["snmpget -v3 -u x -x DES 10.0.0.1 sysName.0"], 0);
    t.check("DES", o.contains("DES is not here"));
    let o = cli(&mut e, PC1, &["snmpget -h"], 0);
    t.check("help", o.contains("-u, -l") && o.contains("-x, -X"));
    let o = cli(&mut e, PC1, &["snmpget -v3 -u anna -r 0 -t 1 10.0.0.77 sysName.0"], 3 * SEC);
    t.check("nobody there", o.contains("did not answer"));
}

/// what the frames show
fn wire(t: &mut T) {
    let mut e = engine(&project(&r1_with(R1_V3)));
    run_for(&mut e, SEC);
    let _ = cli(&mut e, PC1, &[&format!("snmpset {} 10.0.0.1 sysContact.0 s geheimerinhalt", CARL.replace("carl", "dora").replace("-a SHA -A carlspass", "-a MD5 -A dorasauth").replace("carlsgeheim", "dorasgeheim"))], SEC);
    let _ = cli(&mut e, PC1, &[&format!("snmpget {} 10.0.0.1 sysName.0", CARL)], SEC);
    let mut enc = 0;
    let mut report = 0;
    for h in e.trace.hops() {
        if decode::proto_mask(&h.bytes) & decode::P_SNMP == 0 {
            continue;
        }
        let i = decode::info(&h.bytes).1;
        if i.starts_with("SNMPv3 verschlüsselt: user carl (authPriv)") {
            enc += 1;
        }
        if i == "SNMPv3 Report: usmStatsUnknownEngineIDs" {
            report += 1;
        }
    }
    t.check("encrypted messages", enc >= 2);
    t.check("the discovery's reports in the clear", report >= 2);
    t.check("the value nowhere in the clear", !e.trace.hops().any(|h| h.bytes.windows(14).any(|w| w == b"geheimerinhalt")));
    t.check("no password on the wire", !e.trace.hops().any(|h| h.bytes.windows(9).any(|w| w == b"carlspass")));
    let l = e.trace.hops().find(|h| decode::info(&h.bytes).1.starts_with("SNMPv3 verschlüsselt: user carl")).map(|h| decode::layers(&h.bytes));
    t.check("the layer: flags, user, verschlüsselt", l.is_some_and(|ls| ls.iter().any(|x| x.proto == decode::Proto::Snmp && x.fields.iter().any(|f| f.name == "msgFlags" && f.value.contains("auth, priv")) && x.fields.iter().any(|f| f.name == "msgUserName" && f.value == "carl") && x.fields.iter().any(|f| f.value == "verschlüsselt"))));
}

fn determinism(t: &mut T) {
    let go = || {
        let mut e = engine(&project(&r1_with(R1_V3)));
        run_for(&mut e, 200 * MS);
        let o = cli(&mut e, PC1, &[&format!("snmpget {} 10.0.0.1 sysName.0", CARL), "snmpget -v3 -u niemand 10.0.0.1 sysName.0"], SEC);
        let frames: Vec<u8> = e.trace.hops().flat_map(|h| h.bytes.iter().copied().collect::<Vec<u8>>()).collect();
        (o, crypto::sha256(&frames), jfn_sim::keyframe::digest(&e))
    };
    t.check("two runs the same, byte for byte", go() == go());
}
