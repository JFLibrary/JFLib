// dot1xtest -- checks of 802.1X and WPA-Enterprise (group "dot1x" of --selftest: jfn_sim::dot1x,
// dot1xtrace, supp, the EAP part of radiusd, cli/dot1x.rs, jfn_model::wlan's enterprise fields, the
// EAP part of wlan.rs): a PC on an authenticated switch port with EAP-MD5 and with EAP-PWD against
// RADIUS, a wrong password keeps the port closed, quiet period, reauthentication, VLAN assignment,
// force-authorized and force-unauthorized, RADIUS down and the local fallback, a link that goes
// down, the running configuration through a reboot, wpa_cli and its file, a laptop on a
// WPA2-Enterprise SSID that gets its PMK from the MSK and passes the 4-way handshake, a wrong
// password on air, EAP-MD5 on air (no keys), WPA3-Enterprise, the notes, the problems list, the
// English texts and determinism
//
// The LAN (device ids in order): SW1 (switch, Vlan1 10.0.0.2), SRV1 (server-1u, 10.0.0.10, the
// RADIUS server), PC1 (Gi1/0/2), PC2 (Gi1/0/3), R1 (router 10.0.0.1 on Gi1/0/4, an open port the
// PCs ping through once they are in).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::files::File;
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_model::wlan::{Band, Security, Wlan, WlanJoin, WlanNet};
use jfn_sim::dot1x::{PaeState, SesKey};
use jfn_sim::dot1xtrace::DNote;
use jfn_sim::supp::{self, KeyMgmt, NetBlock, SuppState};
use jfn_sim::{Action, Engine, Led, Note, Report, MS, SEC};

use crate::selftest::T;

const BUDGET: usize = 8_000_000;
const SW1: u32 = 1;
const SRV1: u32 = 2;
const PC1: u32 = 3;
const PC2: u32 = 4;
const R1: u32 = 5;
const P_PC1: u16 = 1;
const P_PC2: u16 = 2;

fn s(x: &str) -> String {
    String::from(x)
}

fn def(kind: &str, name: &str, config: &str) -> ServiceDef {
    ServiceDef { kind: s(kind), name: s(name), disabled: false, config: agent_core::json::parse(config.as_bytes()).unwrap_or(Value::Null) }
}

const RADIUS: &str = r#"{"clients": [{"address": "10.0.0.0/24", "secret": "testing123"}],
"users": [{"name": "anna", "password": "geheim"}, {"name": "bob", "password": "bobpw", "vlan": 20},
{"name": "tom", "password": "tompw", "session_timeout": 40}]}"#;

const SERVER_KEY: &str = "radius server S1\n address ipv4 10.0.0.10 auth-port 1812 acct-port 1813\n key testing123\n timeout 1\n retransmit 1\n";

/// the switch's configuration: AAA, the server, the VLANs and `ports` with their lines
fn switch_cfg(methods: &str, ports: &str) -> String {
    format!(
        "hostname SW1\n!\nusername local1 password localpw\n!\naaa new-model\n!\n{}!\naaa authentication dot1x default {}\ndot1x system-auth-control\n!\nvlan 20\n!\n{}interface Vlan1\n ip address 10.0.0.2 255.255.255.0\n no shutdown\n!\nend\n",
        SERVER_KEY, methods, ports
    )
}

/// both PC ports under port-control auto
fn auto_ports(extra: &str) -> String {
    format!("interface GigabitEthernet1/0/2\n authentication port-control auto\n dot1x pae authenticator\n{0}!\ninterface GigabitEthernet1/0/3\n authentication port-control auto\n dot1x pae authenticator\n{0}!\n", extra)
}

/// the file of a wired supplicant
fn wired_conf(eap: &str, user: &str, pass: &str) -> String {
    format!("network={{\n    key_mgmt=IEEE8021X\n    eap={}\n    identity=\"{}\"\n    password=\"{}\"\n}}\n", eap, user, pass)
}

/// the supplicant's file on a device, with its directory
fn add_conf(d: &mut jfn_model::project::Device, text: &str) {
    d.files.push(File { path: s("/etc/wpa_supplicant"), data: Vec::new(), mtime: 1_767_225_600, dir: true });
    d.files.push(File { path: s(supp::CONF), data: Vec::from(text.as_bytes()), mtime: 1_767_225_600, dir: false });
}

/// the LAN; `sw` is the switch's configuration, the PCs' supplicant files (None: no file),
/// `radius` the server's settings (None: no RADIUS server)
fn project(sw: &str, pc1: Option<&str>, pc2: Option<&str>, radius: Option<&str>) -> Project {
    let mut p = Project::new("dot1x", 7);
    for (i, m) in ["jf-s24p", "server-1u", "pc", "pc", "jf-r4"].iter().enumerate() {
        let _ = p.add_device(m, i as i32 * 120, 0);
    }
    let pr = |dev: u32, port: &str| PortRef { dev, port: s(port) };
    let _ = p.connect(&pr(SRV1, "eth0"), &pr(SW1, "gi1/0/1"), None);
    let _ = p.connect(&pr(PC1, "eth0"), &pr(SW1, "gi1/0/2"), None);
    let _ = p.connect(&pr(PC2, "eth0"), &pr(SW1, "gi1/0/3"), None);
    let _ = p.connect(&pr(R1, "gi0/0/0"), &pr(SW1, "gi1/0/4"), None);
    let cfg = [
        (SW1, "SW1", s(sw)),
        (SRV1, "SRV1", s("hostname SRV1\nip addr add 10.0.0.10/24 dev eth0\n")),
        (PC1, "PC1", s("hostname PC1\nip addr add 10.0.0.21/24 dev eth0\n")),
        (PC2, "PC2", s("hostname PC2\nip addr add 10.0.0.22/24 dev eth0\n")),
        (R1, "R1", s("hostname R1\n!\ninterface GigabitEthernet0/0/0\n ip address 10.0.0.1 255.255.255.0\n no shutdown\n!\nend\n")),
    ];
    for (id, name, c) in cfg {
        if let Some(d) = p.device_mut(id) {
            d.name = s(name);
            d.startup_config = c;
        }
    }
    if let Some(d) = p.device_mut(SRV1) {
        if let Some(r) = radius {
            d.services = alloc::vec![def("radius-server", "radius", r)];
        }
    }
    for (id, text) in [(PC1, pc1), (PC2, pc2)] {
        if let (Some(d), Some(t)) = (p.device_mut(id), text) {
            add_conf(d, t);
        }
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

/// what lines print on a console
fn cli(e: &mut Engine, dev: u32, lines: &[&str], d: u64) -> String {
    let _ = e.take_reports();
    for l in lines {
        let _ = e.cli_line(dev, l);
    }
    run_for(e, d);
    let mut o = String::new();
    for r in e.take_reports() {
        if let Report::CliOut { dev: x, bytes, .. } = r
            && x == dev
        {
            o.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    o
}

fn notes(e: &Engine, dev: u32) -> Vec<DNote> {
    e.trace.events().filter(|x| x.dev == dev).filter_map(|x| if let Note::Dot1x(n) = &x.note { Some(n.clone()) } else { None }).collect()
}

fn state_of(e: &Engine, port: u16) -> Option<PaeState> {
    e.dot1x_sessions(SW1).into_iter().find(|x| x.key == SesKey::Port(port)).map(|x| x.state)
}

fn ping(e: &mut Engine, dev: u32, to: &str) -> bool {
    cli(e, dev, &[&format!("ping -c 1 -W 1 {}", to)], 3 * SEC).contains("1 received")
}

pub(crate) fn run(t: &mut T) {
    conf_file_format(t);
    md5_wired(t);
    pwd_wired(t);
    wrong_password(t);
    no_supplicant(t);
    reauth(t);
    vlan_assignment(t);
    force_modes(t);
    server_down(t);
    local_fallback(t);
    link_down(t);
    logoff(t);
    config_roundtrip(t);
    wpa_cli(t);
    frames(t);
    wlan_enterprise(t);
    wlan_wrong(t);
    wlan_md5(t);
    wlan_wpa3(t);
    model_checks(t);
    problems(t);
    texts(t);
    determinism(t);
}

fn conf_file_format(t: &mut T) {
    let text = "# a comment\nctrl_interface=/run/wpa_supplicant\nnetwork={\n  key_mgmt=IEEE8021X\n  eap=MD5\n  identity=\"anna\"\n  password=\"ge heim\"\n}\n\nnetwork = {\n ssid=\"Buero\"\n key_mgmt=WPA-EAP\n eap=PWD\n identity=bob\n password=x\n}\n network={\n key_mgmt=NONE\n}\n";
    let b = supp::parse_conf(text);
    t.eq("three blocks", b.len(), 3);
    t.eq("wired", b[0].clone(), NetBlock { ssid: s(""), key_mgmt: KeyMgmt::Ieee8021x, eap: Some(eapm::Method::Md5), identity: s("anna"), password: s("ge heim") });
    t.eq("enterprise", b[1].clone(), NetBlock { ssid: s("Buero"), key_mgmt: KeyMgmt::WpaEap, eap: Some(eapm::Method::Pwd), identity: s("bob"), password: s("x") });
    t.eq("another word stays", b[2].key_mgmt.clone(), KeyMgmt::Other(s("NONE")));
    let again = supp::parse_conf(&supp::conf_text(&b));
    t.eq("through the file again", again, b);
}

fn md5_wired(t: &mut T) {
    let p = project(&switch_cfg("group radius", &auto_ports("")), Some(&wired_conf("MD5", "anna", "geheim")), None, Some(RADIUS));
    let mut e = engine(&p);
    run_for(&mut e, 200 * MS);
    t.check("closed before it answers or just in", e.dot1x_port_closed(SW1, P_PC2));
    t.eq("PC2's LED: amber", e.port_led(SW1, P_PC2), Led::Unauth);
    run_for(&mut e, 2 * SEC);
    t.eq("PC1 is in", state_of(&e, P_PC1), Some(PaeState::Authenticated));
    t.check("PC1's port is open", !e.dot1x_port_closed(SW1, P_PC1));
    t.eq("PC1's LED", e.port_led(SW1, P_PC1), Led::Up);
    t.check("PC2 has no supplicant: closed", e.dot1x_port_closed(SW1, P_PC2));
    t.eq("PC2 asked", state_of(&e, P_PC2), Some(PaeState::Connecting));
    t.eq("server: Access-Accept", e.metric(SRV1, "radius", "accepts"), Some(1));
    t.eq("server: requests", e.metric(SRV1, "radius", "requests"), Some(2));
    let v = e.dot1x_sessions(SW1).into_iter().find(|x| x.key == SesKey::Port(P_PC1));
    t.eq("user", v.as_ref().map(|x| x.user.clone()), Some(s("anna")));
    t.eq("method", v.as_ref().map(|x| x.method.clone()), Some(s("MD5")));
    let sv = e.supp_views(PC1);
    t.eq("supplicant's state", sv.first().map(|x| x.state), Some(SuppState::Authenticated));
    t.eq("supplicant's method", sv.first().map(|x| x.method.clone()), Some(s("EAP-MD5")));
    t.check("notes of the switch", notes(&e, SW1).iter().any(|n| matches!(n, DNote::Open { user, .. } if user == "anna")));
    t.check("notes of the PC", notes(&e, PC1).iter().any(|n| matches!(n, DNote::SuppOk { .. })));
    // traffic: PC1 passes, PC2 does not
    t.check("PC1 reaches the router", ping(&mut e, PC1, "10.0.0.1"));
    t.check("PC2 does not", !ping(&mut e, PC2, "10.0.0.1"));
    t.check("the closed port dropped it", notes(&e, SW1).iter().any(|n| matches!(n, DNote::Blocked { .. })));
    t.check("the router does not reach PC2", !cli(&mut e, R1, &["ping 10.0.0.22"], 12 * SEC).contains("!!!"));
    // the show commands
    let o = cli(&mut e, SW1, &["enable", "show dot1x all"], 10 * MS);
    t.check("show dot1x: on", o.contains("Port access control: on"));
    t.check("show dot1x: who", o.contains("group radius"));
    t.check("show dot1x: port", o.contains("GigabitEthernet1/0/2: open"));
    t.check("show dot1x: closed", o.contains("GigabitEthernet1/0/3: closed, only EAPOL passes"));
    t.check("show dot1x: state", o.contains("AUTHENTICATED"));
    let o = cli(&mut e, SW1, &["show authentication sessions"], 10 * MS);
    t.check("sessions: table", o.contains("Port") && o.contains("anna") && o.contains("MD5"));
    let o = cli(&mut e, SW1, &["show authentication sessions interface gi1/0/3"], 10 * MS);
    t.check("sessions: one port", o.contains("CONNECTING") && !o.contains("anna"));
    let o = cli(&mut e, SW1, &["show dot1x interface gi1/0/2"], 10 * MS);
    t.check("one port", o.contains("anna") && o.contains("Port control      auto"));
}

fn pwd_wired(t: &mut T) {
    let p = project(&switch_cfg("group radius", &auto_ports("")), Some(&wired_conf("PWD", "anna", "geheim")), None, Some(RADIUS));
    let mut e = engine(&p);
    run_for(&mut e, 2 * SEC);
    t.eq("PC1 is in with EAP-PWD", state_of(&e, P_PC1), Some(PaeState::Authenticated));
    let sv = e.supp_views(PC1);
    t.eq("the method", sv.first().map(|x| x.method.clone()), Some(s("EAP-PWD")));
    let v = e.dot1x_sessions(SW1).into_iter().find(|x| x.key == SesKey::Port(P_PC1));
    t.eq("switch's method", v.map(|x| x.method), Some(s("PWD")));
    // Identity, Nak, PWD-ID, Commit, Confirm: five requests
    t.eq("five requests", e.metric(SRV1, "radius", "requests"), Some(5));
}

fn wrong_password(t: &mut T) {
    let cfg = switch_cfg("group radius", &auto_ports(" dot1x timeout quiet-period 5\n"));
    let p = project(&cfg, Some(&wired_conf("MD5", "anna", "falsch")), Some(&wired_conf("MD5", "nobody", "x")), Some(RADIUS));
    let mut e = engine(&p);
    run_for(&mut e, 2 * SEC);
    t.eq("wrong password: held", state_of(&e, P_PC1), Some(PaeState::Held));
    t.check("still closed", e.dot1x_port_closed(SW1, P_PC1));
    t.eq("unknown user: held", state_of(&e, P_PC2), Some(PaeState::Held));
    t.eq("server: rejects", e.metric(SRV1, "radius", "rejects"), Some(2));
    t.check("the note", notes(&e, SW1).iter().any(|n| matches!(n, DNote::Refused { user, quiet: 5, .. } if user == "anna")));
    t.check("the supplicant's note", notes(&e, PC1).iter().any(|n| matches!(n, DNote::SuppFail { .. })));
    t.check("no traffic", !ping(&mut e, PC1, "10.0.0.1"));
    // after the quiet period the switch asks again, and is refused again
    run_for(&mut e, 5 * SEC);
    t.check("asks again", notes(&e, SW1).iter().any(|n| matches!(n, DNote::Again { .. })));
    // EAP-PWD with the wrong password: the peer stops, the switch's requests run out
    let cfg = switch_cfg("group radius", &auto_ports(" dot1x timeout supp-timeout 2\n dot1x timeout quiet-period 100\n"));
    let p = project(&cfg, Some(&wired_conf("PWD", "anna", "falsch")), None, Some(RADIUS));
    let mut e = engine(&p);
    // (the first Access-Request waits for ARP: the retransmission after 1 s carries it)
    run_for(&mut e, 2 * SEC);
    t.eq("the method ends at the peer", e.supp_views(PC1).first().map(|x| x.state), Some(SuppState::Held));
    t.eq("still authenticating", state_of(&e, P_PC1), Some(PaeState::Authenticating));
    run_for(&mut e, 8 * SEC);
    t.check("retries", notes(&e, SW1).iter().any(|n| matches!(n, DNote::Retry { n: 3, .. })));
    t.check("gives up", notes(&e, SW1).iter().any(|n| matches!(n, DNote::NoAnswer { .. })));
    t.eq("held", state_of(&e, P_PC1), Some(PaeState::Held));
}

fn no_supplicant(t: &mut T) {
    let p = project(&switch_cfg("group radius", &auto_ports(" dot1x timeout tx-period 3\n")), None, None, Some(RADIUS));
    let mut e = engine(&p);
    run_for(&mut e, 10 * SEC);
    t.eq("asks and asks", state_of(&e, P_PC1), Some(PaeState::Connecting));
    let asks = notes(&e, SW1).iter().filter(|n| matches!(n, DNote::TxEap { port, what } if port == "Gi1/0/2" && what == "Request, Identity")).count();
    t.eq("every tx-period", asks, 4);
    // a project without any of it stays as it was
    let p = project("hostname SW1\n!\nend\n", None, None, None);
    let mut e = engine(&p);
    run_for(&mut e, SEC);
    t.check("no sessions", e.dot1x_sessions(SW1).is_empty());
    t.eq("LED", e.port_led(SW1, P_PC1), Led::Up);
    t.check("no frames of EAPOL", e.trace.hops().all(|h| h.mask & netpkt::decode::P_EAPOL == 0));
}

fn reauth(t: &mut T) {
    let cfg = switch_cfg("group radius", &auto_ports(" authentication periodic\n dot1x timeout reauth-period 20\n"));
    let p = project(&cfg, Some(&wired_conf("MD5", "anna", "geheim")), None, Some(RADIUS));
    let mut e = engine(&p);
    run_for(&mut e, 3 * SEC);
    t.eq("in", e.metric(SRV1, "radius", "accepts"), Some(1));
    run_for(&mut e, 19 * SEC);
    t.eq("asked again", e.metric(SRV1, "radius", "accepts"), Some(2));
    t.check("the note", notes(&e, SW1).iter().any(|n| matches!(n, DNote::Reauth { .. })));
    t.check("open all the time", !e.dot1x_port_closed(SW1, P_PC1));
    // the password changes on the server's side: the next round closes the port
    let refused = r#"{"clients": [{"address": "10.0.0.0/24", "secret": "testing123"}], "users": [{"name": "anna", "password": "anders"}]}"#;
    let _ = e.apply(Action::Service { dev: SRV1, def: def("radius-server", "radius", refused) });
    run_for(&mut e, 22 * SEC);
    t.eq("refused now", state_of(&e, P_PC1), Some(PaeState::Held));
    t.check("closed now", e.dot1x_port_closed(SW1, P_PC1));
    // Session-Timeout of the server counts under "authentication periodic"
    let cfg = switch_cfg("group radius", &auto_ports(" authentication periodic\n"));
    let p = project(&cfg, Some(&wired_conf("MD5", "tom", "tompw")), None, Some(RADIUS));
    let mut e = engine(&p);
    run_for(&mut e, 42 * SEC);
    t.eq("twice with Session-Timeout", e.metric(SRV1, "radius", "accepts"), Some(2));
    // by hand
    let p = project(&switch_cfg("group radius", &auto_ports("")), Some(&wired_conf("MD5", "anna", "geheim")), None, Some(RADIUS));
    let mut e = engine(&p);
    run_for(&mut e, 2 * SEC);
    let _ = cli(&mut e, SW1, &["enable", "dot1x re-authenticate interface gi1/0/2"], 2 * SEC);
    t.eq("by hand", e.metric(SRV1, "radius", "accepts"), Some(2));
    let _ = cli(&mut e, SW1, &["clear authentication sessions"], 2 * SEC);
    t.eq("every port", e.metric(SRV1, "radius", "accepts"), Some(3));
}

fn vlan_assignment(t: &mut T) {
    let p = project(&switch_cfg("group radius", &auto_ports("")), Some(&wired_conf("MD5", "bob", "bobpw")), Some(&wired_conf("MD5", "anna", "geheim")), Some(RADIUS));
    let mut e = engine(&p);
    run_for(&mut e, 2 * SEC);
    let v = e.dot1x_sessions(SW1).into_iter().find(|x| x.key == SesKey::Port(P_PC1));
    t.eq("VLAN from the server", v.as_ref().and_then(|x| x.vlan), Some(20));
    t.eq("the bridge follows", e.bridge(SW1).map(|b| b.ports[P_PC1 as usize].mode.clone()), Some(jfn_sim::PortMode::Access(20)));
    t.eq("its setting stays", e.bridge(SW1).map(|b| b.ports[P_PC1 as usize].access_vlan), Some(1));
    t.check("the note", notes(&e, SW1).iter().any(|n| matches!(n, DNote::Open { vlan: Some(20), .. })));
    t.check("VLAN 20 is apart from VLAN 1", !ping(&mut e, PC1, "10.0.0.1"));
    t.check("PC2 in VLAN 1 still reaches", ping(&mut e, PC2, "10.0.0.1"));
    let o = cli(&mut e, SW1, &["enable", "show dot1x interface gi1/0/2"], 10 * MS);
    t.check("show says so", o.contains("VLAN from server  20"));
    let run = cli(&mut e, SW1, &["show running-config"], 10 * MS);
    t.check("not in the configuration", !run.contains("switchport access vlan 20"));
    // the port's own VLAN comes back when the session ends
    let _ = e.apply(Action::PortAdmin { dev: PC1, port: s("eth0"), up: false });
    run_for(&mut e, SEC);
    t.eq("back to its own", e.bridge(SW1).map(|b| b.ports[P_PC1 as usize].mode.clone()), Some(jfn_sim::PortMode::Access(1)));
    // a VLAN the switch does not have: refused
    let cfg = s(&switch_cfg("group radius", &auto_ports(""))).replace("vlan 20\n!\n", "");
    let p = project(&cfg, Some(&wired_conf("MD5", "bob", "bobpw")), None, Some(RADIUS));
    let mut e = engine(&p);
    run_for(&mut e, 2 * SEC);
    t.check("no such VLAN", notes(&e, SW1).iter().any(|n| matches!(n, DNote::NoVlan { vlan: 20, .. })));
    t.check("closed", e.dot1x_port_closed(SW1, P_PC1));
}

fn force_modes(t: &mut T) {
    let ports = "interface GigabitEthernet1/0/2\n authentication port-control force-authorized\n!\ninterface GigabitEthernet1/0/3\n authentication port-control force-unauthorized\n!\n";
    let p = project(&switch_cfg("group radius", ports), Some(&wired_conf("MD5", "anna", "geheim")), Some(&wired_conf("MD5", "anna", "geheim")), Some(RADIUS));
    let mut e = engine(&p);
    run_for(&mut e, 2 * SEC);
    t.check("force-authorized is open", !e.dot1x_port_closed(SW1, P_PC1));
    t.check("force-unauthorized is closed", e.dot1x_port_closed(SW1, P_PC2));
    t.eq("no server asked", e.metric(SRV1, "radius", "requests"), Some(0));
    t.check("PC1 passes", ping(&mut e, PC1, "10.0.0.1"));
    t.check("PC2 does not", !ping(&mut e, PC2, "10.0.0.1"));
    let o = cli(&mut e, SW1, &["enable", "show authentication sessions interface gi1/0/3"], 10 * MS);
    t.check("force-unauth shows", o.contains("FORCE-UNAUTH"));
    // the system switch off: every port open
    let _ = cli(&mut e, SW1, &["configure terminal", "no dot1x system-auth-control", "end"], 10 * MS);
    t.check("off: open", !e.dot1x_port_closed(SW1, P_PC2));
    t.check("off: no sessions", e.dot1x_sessions(SW1).is_empty());
    // configured at run time
    let p = project("hostname SW1\n!\nend\n", Some(&wired_conf("MD5", "anna", "geheim")), None, Some(RADIUS));
    let mut e = engine(&p);
    run_for(&mut e, SEC);
    let lines = ["enable", "configure terminal", "interface vlan 1", "ip address 10.0.0.2 255.255.255.0", "no shutdown", "end", "configure terminal", "aaa new-model", "radius server S1", "address ipv4 10.0.0.10 auth-port 1812 acct-port 1813", "key testing123", "end", "configure terminal", "aaa authentication dot1x default group radius", "dot1x system-auth-control", "interface gi1/0/2", "authentication port-control auto", "end"];
    // (a server with its default timeout of 5 s: the first request waits for ARP)
    let _ = cli(&mut e, SW1, &lines, 7 * SEC);
    t.eq("typed at the console", state_of(&e, P_PC1), Some(PaeState::Authenticated));
    // a trunk takes none of it
    let o = cli(&mut e, SW1, &["configure terminal", "interface gi1/0/5", "switchport mode trunk", "authentication port-control auto", "end"], 10 * MS);
    t.check("a trunk refuses", o.contains("% Port access control needs an access port"));
}

fn server_down(t: &mut T) {
    let p = project(&switch_cfg("group radius", &auto_ports(" dot1x timeout quiet-period 60\n")), Some(&wired_conf("MD5", "anna", "geheim")), None, None);
    let mut e = engine(&p);
    run_for(&mut e, 2 * SEC);
    t.eq("waits for the server", state_of(&e, P_PC1), Some(PaeState::Authenticating));
    run_for(&mut e, 3 * SEC);
    t.check("no server", notes(&e, SW1).iter().any(|n| matches!(n, DNote::NoServer { .. })));
    t.eq("held", state_of(&e, P_PC1), Some(PaeState::Held));
    // no method list at all: refused at once
    let cfg = switch_cfg("group radius", &auto_ports("")).replace("aaa authentication dot1x default group radius\n", "");
    let p = project(&cfg, Some(&wired_conf("MD5", "anna", "geheim")), None, Some(RADIUS));
    let mut e = engine(&p);
    run_for(&mut e, SEC);
    t.eq("no list: held", state_of(&e, P_PC1), Some(PaeState::Held));
    t.eq("no server asked", e.metric(SRV1, "radius", "requests"), Some(0));
    // a wrong shared secret: the answer is dropped, then the server counts as silent
    let wrong = RADIUS.replace("testing123", "other");
    let p = project(&switch_cfg("group radius", &auto_ports("")), Some(&wired_conf("MD5", "anna", "geheim")), None, Some(&wrong));
    let mut e = engine(&p);
    run_for(&mut e, 5 * SEC);
    t.check("server drops it", e.metric(SRV1, "radius", "dropped").unwrap_or(0) > 0);
    t.check("closed", e.dot1x_port_closed(SW1, P_PC1));
}

fn local_fallback(t: &mut T) {
    let p = project(&switch_cfg("group radius local", &auto_ports("")), Some(&wired_conf("MD5", "local1", "localpw")), None, None);
    let mut e = engine(&p);
    run_for(&mut e, 5 * SEC);
    t.eq("the switch checks it itself", state_of(&e, P_PC1), Some(PaeState::Authenticated));
    t.check("open", !e.dot1x_port_closed(SW1, P_PC1));
    // a wrong password locally
    let p = project(&switch_cfg("local", &auto_ports("")), Some(&wired_conf("MD5", "local1", "falsch")), None, None);
    let mut e = engine(&p);
    run_for(&mut e, SEC);
    t.eq("refused locally", state_of(&e, P_PC1), Some(PaeState::Held));
    // a method 802.1X does not take
    let p = project("hostname SW1\n!\nend\n", None, None, None);
    let mut e = engine(&p);
    let o = cli(&mut e, SW1, &["enable", "configure terminal", "aaa authentication dot1x default none", "end"], 10 * MS);
    t.check("none is refused", o.contains("% 802.1X takes"));
}

fn link_down(t: &mut T) {
    let p = project(&switch_cfg("group radius", &auto_ports("")), Some(&wired_conf("MD5", "anna", "geheim")), None, Some(RADIUS));
    let mut e = engine(&p);
    run_for(&mut e, 2 * SEC);
    t.check("open", !e.dot1x_port_closed(SW1, P_PC1));
    let _ = e.apply(Action::PortAdmin { dev: PC1, port: s("eth0"), up: false });
    run_for(&mut e, SEC);
    t.check("link down: closed", e.dot1x_port_closed(SW1, P_PC1));
    t.check("the note", notes(&e, SW1).iter().any(|n| matches!(n, DNote::Closed { .. })));
    t.eq("no session", state_of(&e, P_PC1), None);
    let _ = e.apply(Action::PortAdmin { dev: PC1, port: s("eth0"), up: true });
    run_for(&mut e, 2 * SEC);
    t.eq("link up: in again", state_of(&e, P_PC1), Some(PaeState::Authenticated));
    t.eq("twice", e.metric(SRV1, "radius", "accepts"), Some(2));
    // the switch switched off and on: it starts over, the configuration stays
    let _ = e.apply(Action::Power { dev: SW1, on: false });
    run_for(&mut e, SEC);
    let _ = e.apply(Action::Power { dev: SW1, on: true });
    run_for(&mut e, 200 * MS);
    t.check("closed after the boot", e.dot1x_port_closed(SW1, P_PC2));
    run_for(&mut e, 2 * SEC);
    t.eq("in after the boot", state_of(&e, P_PC1), Some(PaeState::Authenticated));
}

fn logoff(t: &mut T) {
    let cfg = switch_cfg("group radius", &auto_ports(" dot1x timeout tx-period 5\n"));
    let p = project(&cfg, Some(&wired_conf("MD5", "anna", "geheim")), None, Some(RADIUS));
    let mut e = engine(&p);
    run_for(&mut e, 2 * SEC);
    let o = cli(&mut e, PC1, &["wpa_cli logoff"], SEC);
    t.check("OK", o.contains("OK"));
    t.check("logged off: closed", e.dot1x_port_closed(SW1, P_PC1));
    t.check("the switch's note", notes(&e, SW1).iter().any(|n| matches!(n, DNote::Logoff { .. })));
    t.eq("connecting", state_of(&e, P_PC1), Some(PaeState::Connecting));
    let o = cli(&mut e, PC1, &["wpa_cli logon"], 2 * SEC);
    t.check("OK again", o.contains("OK"));
    t.eq("in again", state_of(&e, P_PC1), Some(PaeState::Authenticated));
    t.check("EAPOL-Start came", notes(&e, SW1).iter().any(|n| matches!(n, DNote::Start { .. })));
}

fn config_roundtrip(t: &mut T) {
    let cfg = switch_cfg("group radius local", &auto_ports(" authentication periodic\n dot1x timeout tx-period 7\n dot1x timeout quiet-period 9\n dot1x timeout supp-timeout 11\n dot1x timeout reauth-period 13\n dot1x max-req 4\n"));
    let p = project(&cfg, None, None, Some(RADIUS));
    let e = engine(&p);
    let run = e.running_config(SW1);
    for line in ["aaa authentication dot1x default group radius local", "dot1x system-auth-control", " authentication port-control auto", " authentication periodic", " dot1x pae authenticator", " dot1x timeout tx-period 7", " dot1x timeout quiet-period 9", " dot1x timeout supp-timeout 11", " dot1x timeout reauth-period 13", " dot1x max-req 4"] {
        t.check(&format!("running-config has{}", line), run.lines().any(|l| l == line));
    }
    // through the parser again, the same text
    let mut p2 = p.clone();
    if let Some(d) = p2.device_mut(SW1) {
        d.startup_config = run.clone();
    }
    let e2 = engine(&p2);
    let dot1x = |t: &str| t.lines().filter(|l| l.contains("dot1x") || l.contains("authentication")).map(String::from).collect::<Vec<_>>();
    t.eq("again the same", dot1x(&e2.running_config(SW1)), dot1x(&run));
    // no boot errors, undoing the lines
    t.check("no boot errors", e2.boot_errors(SW1).is_empty());
    let mut e3 = engine(&p);
    let _ = cli(&mut e3, SW1, &["enable", "configure terminal", "interface gi1/0/2", "no authentication port-control", "no authentication periodic", "no dot1x pae authenticator", "no dot1x timeout tx-period", "no dot1x timeout quiet-period", "no dot1x timeout supp-timeout", "no dot1x timeout reauth-period", "no dot1x max-req", "end"], 10 * MS);
    let r = e3.running_config(SW1);
    t.check("gi1/0/3 stays", r.contains("dot1x timeout tx-period 7"));
    t.check("Gi1/0/2 is at its defaults but for pae", r.matches("authentication port-control auto").count() == 1);
    t.check("the port is open", !e3.dot1x_port_closed(SW1, P_PC1));
    // the help has our own words
    let h = e3.cli_help(SW1, "show dot1x ");
    t.check("help of show dot1x", h.contains("The state and every port"));
}

fn wpa_cli(t: &mut T) {
    let p = project(&switch_cfg("group radius", &auto_ports("")), None, None, Some(RADIUS));
    let mut e = engine(&p);
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &["wpa_cli status"], 10 * MS);
    t.check("does not run", o.contains("The supplicant does not run."));
    let o = cli(&mut e, PC1, &["wpa_cli logon"], 10 * MS);
    t.check("cannot log on", o.contains("the supplicant does not run"));
    let o = cli(&mut e, PC1, &["wpa_cli reconfigure"], 10 * MS);
    t.check("no file", o.contains("does not exist"));
    // the file by hand, then the program
    let lines = ["mkdir -p /etc/wpa_supplicant", "echo 'network={' > /tmp/w.conf", "echo 'key_mgmt=IEEE8021X' >> /tmp/w.conf", "echo 'eap=MD5' >> /tmp/w.conf", "echo 'identity=\"anna\"' >> /tmp/w.conf", "echo 'password=\"geheim\"' >> /tmp/w.conf", "echo '}' >> /tmp/w.conf"];
    let _ = cli(&mut e, PC1, &lines, 10 * MS);
    let o = cli(&mut e, PC1, &["wpa_supplicant"], 10 * MS);
    t.check("usage", o.contains("Usage: wpa_supplicant"));
    let o = cli(&mut e, PC1, &["wpa_supplicant -B -i eth0 -c /tmp/none.conf"], 10 * MS);
    t.check("no such file", o.contains("no such file"));
    let o = cli(&mut e, PC1, &["wpa_supplicant -B -i eth0 -c /tmp/w.conf -D wired"], 3 * SEC);
    t.check("it runs", o.contains("The supplicant runs with this file."));
    t.eq("logged in by it", state_of(&e, P_PC1), Some(PaeState::Authenticated));
    let o = cli(&mut e, PC1, &["wpa_cli status"], 10 * MS);
    t.check("status", o.contains("eth0: logged in as anna with EAP-MD5 for"));
    // a Start went out: the program came after the link
    t.check("EAPOL-Start", notes(&e, PC1).iter().any(|n| matches!(n, DNote::SuppStart { .. })));
    let o = cli(&mut e, PC1, &["wpa_cli -i eth0 foo"], 10 * MS);
    t.check("unknown word", o.contains("foo is not known here"));
    let o = cli(&mut e, PC1, &["wpa_cli help"], 10 * MS);
    t.check("help", o.contains("Usage: wpa_cli"));
    // the action of the desktop's form: PC2's file and its login
    let text = supp::conf_text(&[NetBlock { ssid: s(""), key_mgmt: KeyMgmt::Ieee8021x, eap: Some(eapm::Method::Pwd), identity: s("anna"), password: s("geheim") }]);
    let _ = e.take_reports();
    let _ = e.apply(Action::Supplicant { dev: PC2, conf: text.clone() });
    let rep: Vec<(String, bool)> = e.take_reports().into_iter().filter_map(|r| if let Report::Files { path, files, .. } = r { Some((path, files.iter().any(|f| f.path == supp::CONF))) } else { None }).collect();
    t.eq("one step of the project, with the file", rep, alloc::vec![(s(supp::CONF), true)]);
    run_for(&mut e, 3 * SEC);
    t.eq("the form's login", state_of(&e, P_PC2), Some(PaeState::Authenticated));
    t.eq("the file", e.file_store(PC2).and_then(|f| f.read(supp::CONF).ok().map(|b| String::from_utf8_lossy(b).into_owned())), Some(text));
    let o = cli(&mut e, PC2, &["wpa_cli list_networks"], 10 * MS);
    t.check("list", o.contains("(wired)") && o.contains("IEEE8021X") && o.contains("PWD") && o.contains("anna"));
    // a port with nobody at the far end: three Starts, then it goes on without
    let mut p = Project::new("alone", 3);
    let _ = p.add_device("pc", 0, 0);
    let _ = p.add_device("jf-r4", 100, 0);
    let _ = p.connect(&PortRef { dev: 1, port: s("eth0") }, &PortRef { dev: 2, port: s("gi0/0/0") }, None);
    if let Some(d) = p.device_mut(2) {
        d.startup_config = s("interface GigabitEthernet0/0/0\n no shutdown\n");
    }
    if let Some(d) = p.device_mut(1) {
        add_conf(d, &wired_conf("MD5", "anna", "geheim"));
    }
    let mut e = engine(&p);
    run_for(&mut e, 95 * SEC);
    t.eq("nobody answers", e.supp_views(1).first().map(|x| x.state), Some(SuppState::NoPeer));
    t.eq("three Starts", notes(&e, 1).iter().filter(|n| matches!(n, DNote::SuppStart { .. })).count(), 3);
}

fn frames(t: &mut T) {
    let p = project(&switch_cfg("group radius", &auto_ports("")), Some(&wired_conf("PWD", "anna", "geheim")), None, Some(RADIUS));
    let mut e = engine(&p);
    run_for(&mut e, 2 * SEC);
    // the frames between the PC and the switch: EAPOL with the group address, EAP inside
    let eapol: Vec<Vec<u8>> = e.trace.hops().filter(|h| h.mask & netpkt::decode::P_EAPOL != 0 && (h.from.0 == PC1 || h.to.0 == PC1)).map(|h| h.bytes.to_vec()).collect();
    t.check("EAPOL frames", eapol.len() >= 8);
    t.check("to the group address", eapol.iter().all(|f| f[..6] == netpkt::eapol::PAE_GROUP || f[6..12] == [0x02, 0x4A, 0, 0, 0, 2]));
    let infos: Vec<String> = eapol.iter().map(|f| netpkt::decode::info(f).1).collect();
    for want in ["Request, Identity", "Response, Identity (anna)", "Request, MD5-Challenge", "Response, Nak (PWD)", "Request, PWD-ID", "Response, PWD-ID", "Request, PWD-Commit", "Response, PWD-Commit", "Request, PWD-Confirm", "Response, PWD-Confirm", "Success"] {
        t.check(&format!("frame {}", want), infos.iter().any(|i| i == want));
    }
    // RADIUS between the switch and the server: State comes back, EAP-Message and
    // Message-Authenticator in every packet
    let rad: Vec<netpkt::radius::Packet> = e.trace.hops().filter(|h| h.mask & netpkt::decode::P_RADIUS != 0 && h.from.0 == SW1).filter_map(|h| netpkt::eth::L2::parse(&h.bytes).and_then(|l| netpkt::ipv4::Ipv4View::parse(&h.bytes[l.off..]).and_then(|v| netpkt::udp::UdpView::parse(v.payload()).and_then(|u| netpkt::radius::Packet::parse(u.payload()).ok())))).collect();
    t.check("Access-Requests", rad.iter().all(|p| p.code == netpkt::radius::ACCESS_REQUEST));
    t.check("every one with EAP and a Message-Authenticator", !rad.is_empty() && rad.iter().all(|p| p.eap().is_some() && p.attr(netpkt::radius::A_MESSAGE_AUTHENTICATOR).is_some()));
    t.eq("the first without State", rad.first().map(|p| p.attr(netpkt::radius::A_STATE).is_some()), Some(false));
    t.check("the others with State", rad.iter().skip(1).all(|p| p.attr(netpkt::radius::A_STATE).is_some()));
    t.eq("port type", rad.first().and_then(|p| p.int(netpkt::radius::A_NAS_PORT_TYPE)), Some(netpkt::radius::PORT_TYPE_ETHERNET));
    t.eq("the supplicant's address", rad.first().and_then(|p| p.text(netpkt::radius::A_CALLING_STATION_ID)), Some(s("02-4A-00-00-03-01")));
    // debug dot1x prints its notes
    let p = project(&switch_cfg("group radius", &auto_ports(" dot1x timeout tx-period 2\n")), None, None, Some(RADIUS));
    let mut e = engine(&p);
    let o = cli(&mut e, SW1, &["enable", "debug dot1x"], 3 * SEC);
    t.check("debug on", o.contains("Debug output for 802.1X: on"));
    t.check("debug line", o.contains("dot1x: 802.1X: EAP Request, Identity an Gi1/0/2 gesendet."));
    let o = cli(&mut e, SW1, &["show debugging"], 10 * MS);
    t.check("show debugging", o.contains("Debug output for 802.1X: on"));
    let o = cli(&mut e, SW1, &["undebug dot1x"], 3 * SEC);
    t.check("debug off", o.contains("Debug output for 802.1X: off") && !o.contains("gesendet"));
}

// ---------------------------------------------------------------- WPA-Enterprise

const AP1: u32 = 10;
const LAP1: u32 = 11;

fn ent_net(sec: Security, radius: &str, key: &str) -> WlanNet {
    WlanNet { ssid: s("Firma"), band: Band::G5, channel: 0, security: sec, radius: s(radius), radius_key: s(key), ..WlanNet::default() }
}

/// the LAN with an access point (BVI1 10.0.0.3) and a laptop
fn wlan_project(sec: Security, join: WlanJoin) -> Project {
    let mut p = project("hostname SW1\n!\ninterface Vlan1\n ip address 10.0.0.2 255.255.255.0\n no shutdown\n!\nend\n", None, None, Some(RADIUS));
    let ap = p.add_device("jf-ap6e", 0, 150).unwrap_or(0);
    let lap = p.add_device("laptop", 200, 150).unwrap_or(0);
    let _ = p.connect(&PortRef { dev: ap, port: s("TwoGigabitEthernet0") }, &PortRef { dev: SW1, port: s("gi1/0/5") }, None);
    if let Some(d) = p.device_mut(ap) {
        d.name = s("AP1");
        d.startup_config = s("hostname AP1\n!\ninterface BVI1\n ip address 10.0.0.3 255.255.255.0\n no shutdown\n!\nend\n");
        d.wlan = Some(Wlan { nets: alloc::vec![ent_net(sec, "10.0.0.10", "testing123")], join: None });
    }
    if let Some(d) = p.device_mut(lap) {
        d.name = s("Laptop1");
        d.startup_config = s("hostname Laptop1\nip addr add 10.0.0.31/24 dev wlan0\n");
        d.wlan = Some(Wlan { nets: Vec::new(), join: Some(join) });
    }
    p
}

fn joined(user: &str, pass: &str, eap: &str) -> WlanJoin {
    WlanJoin { ssid: s("Firma"), passphrase: s(pass), identity: s(user), eap: s(eap) }
}

fn wlan_enterprise(t: &mut T) {
    let p = wlan_project(Security::Wpa2Ent, joined("anna", "geheim", "PWD"));
    let mut e = engine(&p);
    run_for(&mut e, 5 * SEC);
    let v = e.wlan_station(LAP1);
    t.eq("connected", v.as_ref().map(|x| x.state), Some(jfn_sim::StaState::Connected));
    let a = v.and_then(|x| x.assoc);
    t.eq("authorized after the 4-way handshake", a.as_ref().map(|x| x.authorized), Some(true));
    t.eq("WPA2-Enterprise", a.as_ref().map(|x| x.security), Some(Security::Wpa2Ent));
    t.eq("server accepted", e.metric(SRV1, "radius", "accepts"), Some(1));
    // the frames on the radio link: EAP first, then the four messages, the RSN element says 802.1X
    let frames: Vec<Vec<u8>> = e.trace.hops().filter(|h| h.mask & netpkt::decode::P_EAPOL != 0 && (h.from.0 == LAP1 || h.to.0 == LAP1)).map(|h| h.bytes.to_vec()).collect();
    let infos: Vec<String> = frames.iter().map(|f| netpkt::decode::info(f).1).collect();
    t.check("EAP first", infos.first().is_some_and(|x| x == "Request, Identity"));
    let success = infos.iter().position(|x| x == "Success");
    let m1 = infos.iter().position(|x| x == "Key (Message 1 of 4)");
    t.check("Success before message 1", matches!((success, m1), (Some(a), Some(b)) if a < b));
    for m in 1..=4 {
        t.check(&format!("message {}", m), infos.iter().any(|x| *x == format!("Key (Message {} of 4)", m)));
    }
    let m2 = frames.iter().find(|f| netpkt::decode::info(f).1 == "Key (Message 2 of 4)");
    let rsn = m2.and_then(|f| netpkt::eapol::EapolView::parse(&f[14..]).and_then(|v| netpkt::eapol::Key::parse(v.body()))).and_then(|k| netpkt::eapol::Rsne::parse(&k.data));
    t.eq("AKM 802.1X", rsn.map(|r| r.akm), Some(netpkt::eapol::AKM_8021X));
    // the MSK in RADIUS: the Access-Accept carries it hidden
    let accept = e.trace.hops().filter(|h| h.mask & netpkt::decode::P_RADIUS != 0 && h.to.0 == AP1).filter_map(|h| netpkt::eth::L2::parse(&h.bytes).and_then(|l| netpkt::ipv4::Ipv4View::parse(&h.bytes[l.off..]).and_then(|v| netpkt::udp::UdpView::parse(v.payload()).and_then(|u| netpkt::radius::Packet::parse(u.payload()).ok())))).find(|p| p.code == netpkt::radius::ACCESS_ACCEPT);
    t.check("MS-MPPE keys", accept.is_some_and(|p| p.all(netpkt::radius::A_VENDOR_SPECIFIC).iter().filter(|a| matches!(a.vendor(), Some((netpkt::radius::VENDOR_MICROSOFT, _)))).count() == 2));
    let ses = e.dot1x_sessions(AP1);
    t.eq("AP's session", ses.first().map(|x| (x.user.clone(), x.state)), Some((s("anna"), PaeState::Authenticated)));
    // data flows
    t.check("laptop reaches the router", ping(&mut e, LAP1, "10.0.0.1"));
    // from the file instead of the settings
    let mut p2 = wlan_project(Security::Wpa2Ent, WlanJoin { ssid: s("Firma"), ..WlanJoin::default() });
    if let Some(d) = p2.device_mut(LAP1) {
        add_conf(d, "network={\n ssid=\"Firma\"\n key_mgmt=WPA-EAP\n eap=PWD\n identity=\"anna\"\n password=\"geheim\"\n}\n");
    }
    let mut e = engine(&p2);
    run_for(&mut e, 5 * SEC);
    t.eq("from the file", e.wlan_station(LAP1).map(|x| x.state), Some(jfn_sim::StaState::Connected));
    // no identity anywhere: it waits
    let p3 = wlan_project(Security::Wpa2Ent, WlanJoin { ssid: s("Firma"), ..WlanJoin::default() });
    let mut e = engine(&p3);
    run_for(&mut e, 2 * SEC);
    t.eq("nothing to log in with", e.wlan_station(LAP1).map(|x| x.state), Some(jfn_sim::StaState::NoPassphrase));
    // the file comes later: the waiting station tries again; a station with no network at all takes
    // the one of the file
    let _ = e.apply(Action::Supplicant { dev: LAP1, conf: s("network={\n ssid=\"Firma\"\n key_mgmt=WPA-EAP\n eap=PWD\n identity=\"anna\"\n password=\"geheim\"\n}\n") });
    run_for(&mut e, 5 * SEC);
    t.eq("the form's file wakes it", e.wlan_station(LAP1).map(|x| x.state), Some(jfn_sim::StaState::Connected));
    let mut p4 = wlan_project(Security::Wpa2Ent, WlanJoin::default());
    if let Some(d) = p4.device_mut(LAP1) {
        d.wlan = None;
        add_conf(d, "network={\n ssid=\"Firma\"\n key_mgmt=WPA-EAP\n eap=PWD\n identity=\"anna\"\n password=\"geheim\"\n}\n");
    }
    let mut e = engine(&p4);
    run_for(&mut e, 5 * SEC);
    t.eq("the network of the file", e.wlan_station(LAP1).map(|x| x.state), Some(jfn_sim::StaState::Connected));
    // roaming and disconnects end the session: a disconnect by hand
    let mut e = engine(&p);
    run_for(&mut e, 5 * SEC);
    let _ = cli(&mut e, LAP1, &["iw dev wlan0 disconnect"], SEC);
    t.check("the AP's session went", e.dot1x_sessions(AP1).is_empty());
}

fn wlan_wrong(t: &mut T) {
    let p = wlan_project(Security::Wpa2Ent, joined("anna", "falsch", "PWD"));
    let mut e = engine(&p);
    run_for(&mut e, 20 * SEC);
    t.check("AP sends it away", e.trace.events().any(|x| x.dev == AP1 && matches!(&x.note, Note::Dot1x(DNote::Refused { .. }))));
    let v = e.wlan_station(LAP1);
    t.check("not connected", v.as_ref().is_some_and(|x| x.state != jfn_sim::StaState::Connected));
    t.check("counts as a failure", v.as_ref().is_some_and(|x| x.failures >= 1));
    t.check("reason 23", e.trace.events().any(|x| x.dev == LAP1 && matches!(&x.note, Note::Wlan(jfn_sim::wlantrace::WNote::Deauthed { reason: 23, .. }))));
    // a user the server does not know: Access-Reject at once
    let p = wlan_project(Security::Wpa2Ent, joined("nobody", "x", "PWD"));
    let mut e = engine(&p);
    run_for(&mut e, 3 * SEC);
    t.eq("rejected", e.metric(SRV1, "radius", "rejects"), Some(1));
    // the AP's secret is wrong: the server drops it, the AP gives up after its retries
    let mut p = wlan_project(Security::Wpa2Ent, joined("anna", "geheim", "PWD"));
    if let Some(d) = p.device_mut(AP1) {
        d.wlan = Some(Wlan { nets: alloc::vec![ent_net(Security::Wpa2Ent, "10.0.0.10", "falsch")], join: None });
    }
    let mut e = engine(&p);
    run_for(&mut e, 12 * SEC);
    t.check("no server for the AP", e.trace.events().any(|x| x.dev == AP1 && matches!(&x.note, Note::Dot1x(DNote::NoServer { .. }))));
    t.check("the laptop is out", e.wlan_station(LAP1).is_some_and(|x| x.state != jfn_sim::StaState::Connected));
}

fn wlan_md5(t: &mut T) {
    // a server that only has EAP-MD5 cannot make keys: the laptop's PWD gets a Failure
    let p = wlan_project(Security::Wpa2Ent, joined("anna", "geheim", "MD5"));
    let mut e = engine(&p);
    run_for(&mut e, 5 * SEC);
    t.eq("no common method on air", e.metric(SRV1, "radius", "rejects"), Some(1));
    t.check("not connected", e.wlan_station(LAP1).is_some_and(|x| x.state != jfn_sim::StaState::Connected));
}

fn wlan_wpa3(t: &mut T) {
    let p = wlan_project(Security::Wpa3Ent, joined("anna", "geheim", "PWD"));
    let mut e = engine(&p);
    run_for(&mut e, 5 * SEC);
    t.eq("WPA3-Enterprise connected", e.wlan_station(LAP1).map(|x| x.state), Some(jfn_sim::StaState::Connected));
    let m2 = e.trace.hops().filter(|h| h.mask & netpkt::decode::P_EAPOL != 0).map(|h| h.bytes.to_vec()).find(|f| netpkt::decode::info(f).1 == "Key (Message 2 of 4)");
    let k = m2.and_then(|f| netpkt::eapol::EapolView::parse(&f[14..]).and_then(|v| netpkt::eapol::Key::parse(v.body())));
    t.eq("AES-128-CMAC", k.as_ref().map(|k| k.version()), Some(netpkt::eapol::KDV_CMAC));
    t.eq("AKM 5", k.and_then(|k| netpkt::eapol::Rsne::parse(&k.data)).map(|r| r.akm), Some(netpkt::eapol::AKM_8021X_SHA256));
}

fn model_checks(t: &mut T) {
    use jfn_model::wlan::{check_join, check_net};
    t.check("enterprise is secured", Security::Wpa2Ent.secured() && Security::Wpa2Ent.enterprise() && !Security::Wpa2Ent.personal());
    t.check("personal", Security::Wpa2.personal() && !Security::Wpa2.enterprise());
    t.check("no server", check_net(&ent_net(Security::Wpa2Ent, "", "x")).is_some());
    t.check("no key", check_net(&ent_net(Security::Wpa2Ent, "10.0.0.1", "")).is_some());
    t.check("good", check_net(&ent_net(Security::Wpa2Ent, "10.0.0.1", "x")).is_none());
    let mut n6 = ent_net(Security::Wpa3Ent, "10.0.0.1", "x");
    n6.band = Band::G6;
    t.check("6 GHz takes WPA3-Enterprise", check_net(&n6).is_none());
    n6.security = Security::Wpa2Ent;
    t.check("not WPA2-Enterprise", check_net(&n6).is_some());
    t.check("a short password with an identity", check_join(&joined("a", "x", "PWD")).is_none());
    t.check("a short passphrase without one", check_join(&WlanJoin { ssid: s("n"), passphrase: s("x"), ..WlanJoin::default() }).is_some());
    // the fields reach the project file and come back; an older file has none of them
    let net = ent_net(Security::Wpa2Ent, "10.0.0.10", "k");
    let v = agent_core::json::parse(b"{\"ssid\": \"A\", \"band\": \"5\", \"channel\": 0, \"security\": \"wpa2-personal\", \"passphrase\": \"12345678\", \"vlan\": 0, \"guest\": false, \"isolate\": false}");
    t.check("older settings parse", v.is_ok());
    t.eq("title", Security::Wpa3Ent.title(), "WPA3-Enterprise");
    t.check("wire words", format!("{:?}", net.security) == "Wpa2Ent");
}

fn problems(t: &mut T) {
    use jfn_sim::ProblemKind;
    let kinds = |p: &Project| engine(p).problems().into_iter().map(|x| x.kind).collect::<Vec<_>>();
    // port-control auto without a server
    let p = project("hostname SW1\n!\naaa new-model\ndot1x system-auth-control\n!\ninterface GigabitEthernet1/0/2\n authentication port-control auto\n!\nend\n", None, None, None);
    t.check("no server", kinds(&p).iter().any(|k| *k == ProblemKind::Dot1xNoServer));
    // without the system switch the port stays open: said too
    let p = project("hostname SW1\n!\ninterface GigabitEthernet1/0/2\n authentication port-control auto\n!\nend\n", None, None, None);
    t.check("no system-auth-control", kinds(&p).iter().any(|k| *k == ProblemKind::Dot1xOff));
    // a good one has neither
    let p = project(&switch_cfg("group radius", &auto_ports("")), None, None, Some(RADIUS));
    let k = kinds(&p);
    t.check("good", !k.iter().any(|k| matches!(k, ProblemKind::Dot1xNoServer | ProblemKind::Dot1xOff)));
    // a network without its server is a problem like any network that cannot be offered
    let mut p = wlan_project(Security::Wpa2Ent, joined("anna", "geheim", "PWD"));
    if let Some(d) = p.device_mut(AP1) {
        d.wlan = Some(Wlan { nets: alloc::vec![ent_net(Security::Wpa2Ent, "", "")], join: None });
    }
    t.check("enterprise net without a server", !engine(&p).problems().is_empty());
}

fn texts(t: &mut T) {
    let en = |m: &jfn_model::msg::Msg| crate::app::tr::msg(m);
    let n = DNote::Blocked { port: s("Gi1/0/2") };
    t.check("the note's German", n.message().text().contains("nicht autorisiert"));
    t.eq("layer", Note::Dot1x(n.clone()).layer(), 2);
    t.check("is a drop", Note::Dot1x(n.clone()).is_drop());
    t.eq("mask", n.mask(), netpkt::decode::P_EAPOL);
    let _ = en(&n.message());
}

fn determinism(t: &mut T) {
    let run = || {
        let p = wlan_project(Security::Wpa2Ent, joined("anna", "geheim", "PWD"));
        let mut e = engine(&p);
        run_for(&mut e, 5 * SEC);
        e.trace.hops().map(|h| (h.t_rx, h.bytes.to_vec())).collect::<Vec<_>>()
    };
    let (a, b) = (run(), run());
    t.check("frames", !a.is_empty());
    t.eq("twice the same bytes", a == b, true);
    // a rewind comes back to the same state
    let p = project(&switch_cfg("group radius", &auto_ports("")), Some(&wired_conf("PWD", "anna", "geheim")), None, Some(RADIUS));
    let mut e = engine(&p);
    run_for(&mut e, 2 * SEC);
    let mut e2 = e.clone();
    run_for(&mut e, 2 * SEC);
    run_for(&mut e2, 2 * SEC);
    t.eq("a keyframe's clone goes on the same", jfn_sim::keyframe::digest(&e), jfn_sim::keyframe::digest(&e2));
    t.check("it holds the settings and sessions", jfn_sim::keyframe::footprint(&e) > 0);
}
