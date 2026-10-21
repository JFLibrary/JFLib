// svctest -- checks of the services of jfn_sim (part of --selftest): the campus scenario (two
// PCs get their addresses over R1's relay from SRV1 in another network, resolve "www" with
// the search domain they got by DHCP, follow the CNAME and ping by name) with exact virtual
// times; the IOS DHCP server on a router (ping check, excluded addresses, lowest address
// first, the IOS client with its broadcast flag, a NAK after renumbering); conflicts found by
// the server's ping and by the client's ARP probe (DECLINE); renewal, rebinding, expiry,
// release (the address goes 10 ms after the RELEASE) and INIT-REBOOT; the DNS server and the
// resolver (CNAME, NXDOMAIN, NODATA, MX, PTR, referral, search order, glibc's choice of the
// error, cache and TTL, failover after the timeout, forwarding, host table, the latency window);
// syslog from IOS and Linux; NTP stepping (with the error an asymmetric path brings), slewing
// and correcting the drift; service views, project definitions, cloning and determinism
//
// Times: 1 Gbit/s copper with 50 ns of cable, (max(n, 60) + 24) * 8 + 50 ns per frame of n
// bytes.  DHCP messages are padded to 300 bytes: 342-byte frames, 2978 ns per cable.  An ARP
// frame takes 722 ns, the query for www.lab 778 ns, its answer 1050 ns, an echo 1026 ns.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_sim::node::port_mac;
use jfn_sim::{
    ActErr, Action, BindState, ClientEv, ClientState, Detect, DhcpReq, Engine, IfKind, JobKind, Lease, LookupErr, Note, PingEv, PingOpts, PoolCfg, RouteSrc, Source, TryOut, MS, SEC, US,
};
use netpkt::dns::{self, RData, Rr};
use netpkt::{Ipv4, Mac, Prefix4};

use crate::selftest::T;

const BUDGET: usize = 2_000_000;

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

fn ip(s: &str) -> Ipv4 {
    Ipv4::parse(s).unwrap_or(Ipv4::UNSPECIFIED)
}

fn pf(s: &str) -> Prefix4 {
    Prefix4::parse(s).unwrap_or(Prefix4 { addr: Ipv4::UNSPECIFIED, len: 0 })
}

fn addr(e: &mut Engine, dev: u32, iface: &str, p: &str) -> Result<(), ActErr> {
    e.apply(Action::IfAddr { dev, iface: String::from(iface), prefix: pf(p), add: true })
}

fn if_up(e: &mut Engine, dev: u32, iface: &str) -> Result<(), ActErr> {
    e.apply(Action::IfUp { dev, iface: String::from(iface), up: true })
}

fn gw(e: &mut Engine, dev: u32, via: &str) -> Result<(), ActErr> {
    e.apply(Action::Route { dev, prefix: pf("0.0.0.0/0"), via: Some(ip(via)), iface: None, ad: 1, add: true })
}

fn client(e: &mut Engine, dev: u32, iface: &str, on: bool) -> Result<(), ActErr> {
    e.apply(Action::DhcpClient { dev, iface: String::from(iface), on })
}

fn json(s: &str) -> Value {
    agent_core::json::parse(s.as_bytes()).unwrap_or(Value::Null)
}

fn def(kind: &str, name: &str, config: &str) -> ServiceDef {
    ServiceDef { kind: String::from(kind), name: String::from(name), disabled: false, config: json(config) }
}

fn mac(dev: u32) -> Mac {
    port_mac(dev, 0)
}

/// the notes of a device, in order
fn notes(e: &Engine, dev: u32) -> Vec<Note> {
    e.trace.events().filter(|x| x.dev == dev).map(|x| x.note.clone()).collect()
}

/// the time of the first note of a device that matches
fn when(e: &Engine, dev: u32, f: impl Fn(&Note) -> bool) -> Option<u64> {
    e.trace.events().find(|x| x.dev == dev && f(&x.note)).map(|x| x.t)
}

/// the lease of a device's first DHCP client
fn lease(e: &Engine, dev: u32) -> Option<Lease> {
    e.dhcp_leases(dev).first().and_then(|l| l.lease.clone())
}

fn state(e: &Engine, dev: u32) -> Option<ClientState> {
    e.dhcp_leases(dev).first().map(|l| l.state)
}

/// when a device's DHCP client bound an address (every time)
fn bound_at(e: &Engine, dev: u32) -> Vec<(u64, Ipv4)> {
    e.dhcp_leases(dev).first().map(|l| l.log.iter().filter_map(|x| if let ClientEv::Bound { ip, .. } = x.ev { Some((x.t, ip)) } else { None }).collect()).unwrap_or_default()
}

/// the addresses of an interface
fn addrs(e: &Engine, dev: u32, iface: &str) -> Vec<Prefix4> {
    e.ifaces(dev).into_iter().find(|f| f.name == iface).map(|f| f.addrs).unwrap_or_default()
}

fn ping_ev(e: &Engine, id: u32) -> Vec<PingEv> {
    match e.job(id).map(|j| &j.kind) {
        Some(JobKind::Ping(p)) => p.events.clone(),
        _ => Vec::new(),
    }
}

fn reply(seq: u16, from: &str, ttl: u8, rtt: u64) -> PingEv {
    PingEv::Reply { seq, from: ip(from), bytes: 84, ttl, rtt }
}

pub fn run(t: &mut T) {
    relay_scenario(t);
    ios_server(t);
    conflicts(t);
    lifecycle(t);
    dns_server(t);
    resolver(t);
    syslog_ntp(t);
    views_defs(t);
    determinism(t);
}

// ---------------------------------------------------------------- the campus scenario

/// the services of SRV1: DHCP for the relayed 10.0.1.0/24, DNS for "lab"
fn srv1_services() -> Vec<ServiceDef> {
    vec![
        def(
            "dhcp-server",
            "isc-dhcp-server",
            r#"{"pools": [{"name": "lan1", "network": "10.0.1.0/24", "range": ["10.0.1.100", "10.0.1.199"], "routers": ["10.0.1.1"], "dns": ["10.0.2.10"], "domain": "lab", "lease": 600}]}"#,
        ),
        def("dns-server", "bind9", r#"{"zones": [{"origin": "lab", "ttl": 300, "records": ["ns A 10.0.2.10", "srv1 A 10.0.2.10", "web A 10.0.2.20", "www CNAME web"]}]}"#),
    ]
}

/// PC1, PC2 - SW1 - Gi0/0/0 R1 Gi0/0/1 - SW2 - SRV1 (DHCP, DNS from the project file), SRV2:
/// (engine, [pc1, pc2], r1, srv1, srv2, set-up ok)
fn campus(seed: u64) -> (Engine, Vec<u32>, u32, u32, u32, bool) {
    let mut p = Project::new("campus", seed);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let pc2 = p.add_device("pc", 100, 0).unwrap_or(0);
    let sw1 = p.add_device("jf-s24p", 50, 100).unwrap_or(0);
    let r1 = p.add_device("jf-r4", 50, 200).unwrap_or(0);
    let sw2 = p.add_device("jf-s24p", 50, 300).unwrap_or(0);
    let srv1 = p.add_device("server-1u", 0, 400).unwrap_or(0);
    let srv2 = p.add_device("server-1u", 100, 400).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(sw1, "gi1/0/1"), None);
    let _ = p.connect(&pr(pc2, "eth0"), &pr(sw1, "gi1/0/2"), None);
    let _ = p.connect(&pr(sw1, "gi1/0/24"), &pr(r1, "gi0/0/0"), None);
    let _ = p.connect(&pr(r1, "gi0/0/1"), &pr(sw2, "gi1/0/24"), None);
    let _ = p.connect(&pr(srv1, "eth0"), &pr(sw2, "gi1/0/1"), None);
    let _ = p.connect(&pr(srv2, "eth0"), &pr(sw2, "gi1/0/2"), None);
    if let Some(d) = p.devices.iter_mut().find(|d| d.id == srv1) {
        d.services = srv1_services();
    }
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    let ok = [
        addr(&mut e, r1, "Gi0/0/0", "10.0.1.1/24"),
        addr(&mut e, r1, "Gi0/0/1", "10.0.2.1/24"),
        if_up(&mut e, r1, "Gi0/0/0"),
        if_up(&mut e, r1, "Gi0/0/1"),
        e.apply(Action::DhcpHelper { dev: r1, iface: String::from("gi0/0/0"), server: ip("10.0.2.10"), add: true }),
        addr(&mut e, srv1, "eth0", "10.0.2.10/24"),
        gw(&mut e, srv1, "10.0.2.1"),
        addr(&mut e, srv2, "eth0", "10.0.2.20/24"),
        gw(&mut e, srv2, "10.0.2.1"),
        client(&mut e, pc1, "eth0", true),
        client(&mut e, pc2, "eth0", true),
    ];
    (e, vec![pc1, pc2], r1, srv1, srv2, ok.iter().all(|r| r.is_ok()))
}

fn relay_scenario(t: &mut T) {
    let (mut e, pcs, r1, srv1, _srv2, ok) = campus(21);
    let (pc1, pc2) = (pcs[0], pcs[1]);
    t.check("campus set-up", ok);
    e.run_until(20 * SEC, BUDGET);
    // t = 0: both PCs broadcast a DISCOVER.  PC1's reaches R1 after two cables (5956 ns); R1
    // relays it but has to ask for SRV1's MAC first and drops it (IOS keeps nothing back while
    // ARP asks).  PC2's waited behind it on SW1's uplink (8884 ns); by then R1 knows SRV1, so it
    // reaches SRV1 at 14840 ns.  dhcpd pings the highest free address, 10.0.1.199, for a second
    // (R1 cannot resolve it either), then offers it: four cables back to PC2 (1 s + 26752 ns),
    // the REQUEST four cables to SRV1, the ACK four cables back (1 s + 50576 ns).  PC2 probes
    // the address twice a second apart and binds it 2 s after the ACK.
    t.eq("relay dropped the first discover", when(&e, r1, |n| *n == Note::ArpIncomplete { ip: ip("10.0.2.10") }), Some(5956));
    t.eq("ping check of .199", when(&e, srv1, |n| *n == Note::DhcpdPing { ip: ip("10.0.1.199") }), Some(14_840));
    t.eq("offer after one second", when(&e, srv1, |n| *n == Note::DhcpdOffer { ip: ip("10.0.1.199"), mac: mac(pc2) }), Some(SEC + 14_840));
    t.eq("pc2 sees the offer", when(&e, pc2, |n| matches!(n, Note::DhcpOffered { .. })), Some(SEC + 26_752));
    t.eq("pc2 ack", when(&e, pc2, |n| matches!(n, Note::DhcpAcked { .. })), Some(SEC + 50_576));
    t.eq("pc2 probes", e.trace.events().filter(|x| x.dev == pc2 && x.note == Note::DhcpProbe { ip: ip("10.0.1.199") }).map(|x| x.t).collect::<Vec<_>>(), vec![SEC + 50_576]);
    t.eq("pc2 bound", bound_at(&e, pc2), vec![(3 * SEC + 50_576, ip("10.0.1.199"))]);
    // PC1 tries again after 3 s and a random fraction (dhclient); now R1 knows SRV1
    t.eq("pc1 second discover", e.trace.events().filter(|x| x.dev == pc1 && matches!(x.note, Note::DhcpDiscover { .. })).map(|x| x.t).collect::<Vec<_>>(), vec![0, 3_419_092_000]);
    t.eq("pc1 bound", bound_at(&e, pc1), vec![(6_419_139_648, ip("10.0.1.198"))]);
    let l = lease(&e, pc1);
    t.eq(
        "pc1 lease",
        l.as_ref().map(|l| (l.addr, l.routers.clone(), l.dns.clone(), l.domain.clone(), l.server, l.lease, l.obtained, l.expiry)),
        Some((pf("10.0.1.198/24"), vec![ip("10.0.1.1")], vec![ip("10.0.2.10")], Some(String::from("lab")), ip("10.0.2.10"), 600, 4_419_139_648, 604_419_139_648)),
    );
    // dhclient renews between 3/4 and 4/4 of T1 = 300 s
    t.check("pc1 renewal time", l.as_ref().is_some_and(|l| (225 * SEC..=300 * SEC).contains(&(l.t1 - l.obtained)) && l.t2 == l.obtained + 525 * SEC));
    t.eq("pc1 address", addrs(&e, pc1, "eth0"), vec![pf("10.0.1.198/24")]);
    t.check("pc1 default route", e.routes(pc1).iter().any(|r| r.prefix == pf("0.0.0.0/0") && r.via == Some(ip("10.0.1.1")) && r.src == RouteSrc::Static));
    t.eq("pc1 resolv.conf", e.resolv_conf(pc1).map(|c| (c.servers, c.search)), Some((vec![ip("10.0.2.10")], vec![String::from("lab")])));
    let b: Vec<(Ipv4, Mac, BindState, u64)> = e.dhcp_bindings(srv1).iter().map(|b| (b.ip, b.mac, b.state, b.until)).collect();
    // the server counts the lease from its ACK
    t.eq("bindings", b, vec![(ip("10.0.1.198"), mac(pc1), BindState::Bound, 604_419_127_736), (ip("10.0.1.199"), mac(pc2), BindState::Bound, 601_000_038_664)]);
    t.eq("relay counts", (e.metric(r1, "dhcp-server", "relayed"), e.metric(r1, "dhcp-server", "relayed_back")), (Some(5), Some(4)));
    t.eq("server counts", ["discovers", "offers", "requests", "acks"].map(|m| e.metric(srv1, "isc-dhcp-server", m)), [Some(2), Some(2), Some(2), Some(2)]);
    // t = 20 s: PC1 pings "www".  The resolver asks for www.lab (search domain), PC1 first
    // asks for its gateway's MAC (4 x 722 ns), the query takes four cables (4 x 778 ns), the
    // answer (CNAME web.lab, A 10.0.2.20) four more (4 x 1050 ns): the address is known at
    // 20 s + 10200 ns.  The first echo is lost at R1 (ARP for SRV2), the others take 8 x 1026
    // ns, and the reply lost a TTL at R1.
    let id = e.ping_host(pc1, "www", PingOpts { count: 3, ..PingOpts::default() }).unwrap_or(0);
    e.run_until(30 * SEC, BUDGET);
    let lid = match e.job(id).map(|j| &j.kind) {
        Some(JobKind::Ping(p)) => (p.lookup, p.host.clone(), p.dst),
        _ => (None, None, Ipv4::UNSPECIFIED),
    };
    t.eq("ping by name", (lid.1.as_deref(), lid.2), (Some("www"), ip("10.0.2.20")));
    let look = lid.0.and_then(|l| e.lookup_state(pc1, l)).cloned();
    t.eq(
        "lookup",
        look.as_ref().and_then(|l| l.result.clone()).map(|r| r.map(|a| (a.name, a.canonical, a.addrs, a.from, a.ttl))),
        Some(Ok((String::from("www.lab"), String::from("web.lab"), vec![ip("10.0.2.20")], Source::Server(ip("10.0.2.10")), 300))),
    );
    t.eq("lookup done", look.as_ref().and_then(|l| l.done), Some(20 * SEC + 10_200));
    t.eq("one question", look.as_ref().map(|l| l.tries.iter().map(|x| (x.name.clone(), x.server, x.outcome, x.rtt)).collect::<Vec<_>>()), Some(vec![(String::from("www.lab"), ip("10.0.2.10"), Some(TryOut::Answer), 10_200)]));
    t.eq("echoes", ping_ev(&e, id), vec![reply(2, "10.0.2.20", 63, 8208), PingEv::Timeout { seq: 1 }, reply(3, "10.0.2.20", 63, 8208)]);
    t.check("server answered", notes(&e, srv1).contains(&Note::DnsAnswer { name: String::from("www.lab"), qtype: dns::TYPE_A, n: 2 }));
    // PC2 pings SRV1 by its short name: R1 knows SRV1 already, nothing is lost
    let id2 = e.ping_host(pc2, "srv1", PingOpts { count: 1, ..PingOpts::default() }).unwrap_or(0);
    e.run_until(35 * SEC, BUDGET);
    t.check("pc2 pings srv1", matches!(ping_ev(&e, id2).as_slice(), [PingEv::Reply { from, ttl: 63, .. }] if *from == ip("10.0.2.10")));
    t.eq("dns counts", (e.metric(srv1, "bind9", "queries"), e.metric(srv1, "bind9", "answers")), (Some(2), Some(2)));
}

// ---------------------------------------------------------------- the IOS DHCP server

/// PC1 and R2 - SW1 - Gi0/0/0 R1 (10.0.1.1/24, pool LAN), with or without DHCP clients on
/// PC1 and R2: (engine, pc1, r2, r1, set-up ok)
fn ios_lan(clients: bool) -> (Engine, u32, u32, u32, bool) {
    let mut p = Project::new("ios", 5);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let r1 = p.add_device("jf-r4", 50, 200).unwrap_or(0);
    let sw1 = p.add_device("jf-s24p", 50, 100).unwrap_or(0);
    let r2 = p.add_device("jf-r4", 100, 0).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(sw1, "gi1/0/1"), None);
    let _ = p.connect(&pr(r2, "gi0/0/0"), &pr(sw1, "gi1/0/2"), None);
    let _ = p.connect(&pr(sw1, "gi1/0/24"), &pr(r1, "gi0/0/0"), None);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    let mut pool = PoolCfg::new(pf("10.0.1.0/24"));
    pool.routers = vec![ip("10.0.1.1")];
    pool.dns = vec![ip("10.0.1.53")];
    pool.domain = Some(String::from("lab"));
    let ok = [
        addr(&mut e, r1, "Gi0/0/0", "10.0.1.1/24"),
        if_up(&mut e, r1, "Gi0/0/0"),
        e.apply(Action::DhcpExclude { dev: r1, from: ip("10.0.1.1"), to: ip("10.0.1.9"), add: true }),
        e.apply(Action::DhcpPool { dev: r1, name: String::from("LAN"), pool: Some(pool) }),
    ];
    let c = if clients { [client(&mut e, pc1, "eth0", true), client(&mut e, r2, "Gi0/0/0", true)] } else { [Ok(()), Ok(())] };
    (e, pc1, r2, r1, ok.iter().chain(c.iter()).all(|r| r.is_ok()))
}

fn ios_server(t: &mut T) {
    let (mut e, pc1, r2, r1, ok) = ios_lan(true);
    t.check("ios set-up", ok);
    // R2's interface is shut down: its client waits
    e.run_until(2 * SEC, BUDGET);
    t.eq("ios client waits for the interface", state(&e, r2), Some(ClientState::Init));
    t.check("if up", if_up(&mut e, r2, "Gi0/0/0").is_ok());
    e.run_until(10 * SEC, BUDGET);
    // IOS pings twice, 500 ms each; the lowest address after the excluded ones goes first;
    // the address being checked for PC1 is not offered to R2
    let pings: Vec<(u64, Ipv4)> = e.trace.events().filter(|x| x.dev == r1).filter_map(|x| if let Note::DhcpdPing { ip } = x.note { Some((x.t, ip)) } else { None }).collect();
    t.eq("ping checks", pings, vec![(5956, ip("10.0.1.10")), (2 * SEC + 5956, ip("10.0.1.11"))]);
    t.eq("offers", e.trace.events().filter(|x| x.dev == r1).filter_map(|x| if let Note::DhcpdOffer { ip, .. } = x.note { Some((x.t, ip)) } else { None }).collect::<Vec<_>>(), vec![(SEC + 5956, ip("10.0.1.10")), (3 * SEC + 5956, ip("10.0.1.11"))]);
    t.eq("pc1 lease", lease(&e, pc1).map(|l| (l.addr, l.lease, l.dns)), Some((pf("10.0.1.10/24"), 86_400, vec![ip("10.0.1.53")])));
    let l2 = lease(&e, r2);
    t.eq("r2 lease", l2.as_ref().map(|l| (l.addr, l.t1 - l.obtained, l.t2 - l.obtained)), Some((pf("10.0.1.11/24"), 43_200 * SEC, 75_600 * SEC)));
    // the IOS client binds at once (no ARP check) and installs its default route with distance 254
    t.check("r2 bound without probing", !notes(&e, r2).iter().any(|n| matches!(n, Note::DhcpProbe { .. })) && bound_at(&e, r2).len() == 1);
    t.check("r2 default route 254", e.routes(r2).iter().any(|r| r.prefix == pf("0.0.0.0/0") && r.ad == 254 && r.via == Some(ip("10.0.1.1"))));
    t.check("r2 log", e.log_lines(r2).iter().any(|l| l.tag == "%DHCP-6-ADDRESS_ASSIGN" && l.text == "Interface GigabitEthernet0/0/0 assigned DHCP address 10.0.1.11, mask 255.255.255.0, hostname R2"));
    // the IOS client asks for broadcasts and names itself with "cisco-<mac>-<interface>"
    let offer_to_r2 = e.trace.hops().find(|h| h.from.0 == r1 && netpkt::decode::info(&h.bytes).1.starts_with("DHCP Offer") && h.t_tx > 2 * SEC).map(|h| h.bytes.to_vec());
    let b = offer_to_r2.as_deref().and_then(|f| netpkt::dhcp::Dhcp::parse(&f[42..]));
    t.check("broadcast offer", b.as_ref().is_some_and(|m| m.broadcast()) && offer_to_r2.is_some_and(|f| f[..6] == [0xFF; 6]));
    let bind: Vec<(Ipv4, Mac, Option<String>)> = e.dhcp_bindings(r1).into_iter().map(|b| (b.ip, b.mac, b.client_id.map(|c| String::from_utf8_lossy(&c[1..]).into_owned()))).collect();
    t.eq("bindings", bind, vec![(ip("10.0.1.10"), mac(pc1), None), (ip("10.0.1.11"), mac(r2), Some(format!("cisco-{}-Gi0/0/0", mac(r2).cisco())))]);
    t.eq("pool use", e.dhcp_pools(r1).iter().map(|p| (p.name.clone(), p.leased, p.total)).collect::<Vec<_>>(), vec![(String::from("LAN"), 2, 254)]);
    // renumbering: R1 moves to 10.0.5.0/24; PC1 restarts and asks for its old address
    // (INIT-REBOOT), IOS answers with a NAK, PC1 starts over and gets 10.0.5.10
    let mut pool = PoolCfg::new(pf("10.0.5.0/24"));
    pool.routers = vec![ip("10.0.5.1")];
    let moved = [
        e.apply(Action::IfAddr { dev: r1, iface: String::from("Gi0/0/0"), prefix: pf("10.0.1.1/24"), add: false }),
        addr(&mut e, r1, "Gi0/0/0", "10.0.5.1/24"),
        e.apply(Action::DhcpExclude { dev: r1, from: ip("10.0.5.1"), to: ip("10.0.5.9"), add: true }),
        e.apply(Action::DhcpPool { dev: r1, name: String::from("LAN"), pool: Some(pool) }),
        e.apply(Action::Power { dev: pc1, on: false }),
        e.apply(Action::Power { dev: pc1, on: true }),
    ];
    t.check("renumbered", moved.iter().all(|r| r.is_ok()));
    e.run_until(30 * SEC, BUDGET);
    t.check("init-reboot", notes(&e, pc1).contains(&Note::DhcpRequest { iface: 1, ip: ip("10.0.1.10"), kind: DhcpReq::Reboot }));
    t.check("nak", notes(&e, r1).contains(&Note::DhcpdNak { ip: ip("10.0.1.10"), mac: mac(pc1) }) && notes(&e, pc1).contains(&Note::DhcpNaked { server: ip("10.0.5.1") }));
    t.eq("new lease", lease(&e, pc1).map(|l| l.addr), Some(pf("10.0.5.10/24")));
    // the pool moved: bindings outside its new network are gone (R2 learns it at its renewal)
    t.eq("old bindings gone with the pool", e.dhcp_bindings(r1).iter().map(|b| b.ip).collect::<Vec<_>>(), vec![ip("10.0.5.10")]);
    t.eq("removing an unknown pool", e.apply(Action::DhcpPool { dev: r1, name: String::from("NOPE"), pool: None }).is_err(), true);
}

// ---------------------------------------------------------------- conflicts

fn conflicts(t: &mut T) {
    // the server's ping finds a device that uses 10.0.1.10 without a lease
    let (mut e, pc1, r2, r1, _) = ios_lan(false);
    // R2 takes 10.0.1.10 by hand; PC1 asks a little later
    let _ = addr(&mut e, r2, "Gi0/0/0", "10.0.1.10/24");
    let _ = if_up(&mut e, r2, "Gi0/0/0");
    e.run_until(SEC, BUDGET);
    let _ = client(&mut e, pc1, "eth0", true);
    e.run_until(10 * SEC, BUDGET);
    t.check("ping conflict", notes(&e, r1).contains(&Note::DhcpdPingConflict { ip: ip("10.0.1.10") }));
    t.check("conflict logged", e.log_lines(r1).iter().any(|l| l.tag == "%DHCPD-4-PING_CONFLICT" && l.text.contains("10.0.1.10")));
    t.eq("next address", lease(&e, pc1).map(|l| l.addr.addr), Some(ip("10.0.1.11")));
    let c: Vec<(Ipv4, BindState, Option<Detect>)> = e.dhcp_bindings(r1).into_iter().map(|b| (b.ip, b.state, b.detect)).collect();
    t.eq("conflict binding", c, vec![(ip("10.0.1.10"), BindState::Conflict, Some(Detect::Ping)), (ip("10.0.1.11"), BindState::Bound, None)]);
    t.check("clear conflicts", e.apply(Action::DhcpClear { dev: r1, ip: None, conflicts: true }).is_ok() && e.dhcp_bindings(r1).len() == 1);

    // without the ping check the client's ARP probe finds the device: DECLINE, 10 s, again
    let mut p = Project::new("decline", 9);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let pc0 = p.add_device("pc", 100, 0).unwrap_or(0);
    let srv = p.add_device("server-1u", 50, 200).unwrap_or(0);
    let sw = p.add_device("jf-s24p", 50, 100).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(sw, "gi1/0/1"), None);
    let _ = p.connect(&pr(pc0, "eth0"), &pr(sw, "gi1/0/2"), None);
    let _ = p.connect(&pr(srv, "eth0"), &pr(sw, "gi1/0/3"), None);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    let ok = [
        addr(&mut e, srv, "eth0", "10.0.0.53/24"),
        addr(&mut e, pc0, "eth0", "10.0.0.110/24"),
        e.apply(Action::Service { dev: srv, def: def("dhcp-server", "dhcpd", r#"{"pools": [{"name": "lan", "network": "10.0.0.0/24", "range": ["10.0.0.100", "10.0.0.110"]}], "ping_packets": 0}"#) }),
        client(&mut e, pc1, "eth0", true),
    ];
    t.check("decline set-up", ok.iter().all(|r| r.is_ok()));
    e.run_until(30 * SEC, BUDGET);
    t.check("probe answered", notes(&e, pc1).contains(&Note::DhcpConflict { ip: ip("10.0.0.110"), mac: mac(pc0) }));
    t.check("server saw the decline", notes(&e, srv).contains(&Note::DhcpdDecline { ip: ip("10.0.0.110"), mac: mac(pc1) }));
    let decl = when(&e, pc1, |n| matches!(n, Note::DhcpConflict { .. })).unwrap_or(0);
    let again = e.trace.events().filter(|x| x.dev == pc1 && matches!(x.note, Note::DhcpDiscover { .. }) && x.t > decl).map(|x| x.t).next();
    t.eq("10 s later again", again.map(|a| a - decl), Some(10 * SEC));
    t.eq("other address", lease(&e, pc1).map(|l| l.addr.addr), Some(ip("10.0.0.109")));
    t.eq("declined address", e.dhcp_bindings(srv).iter().map(|b| (b.ip, b.state, b.detect)).collect::<Vec<_>>(), vec![(ip("10.0.0.109"), BindState::Bound, None), (ip("10.0.0.110"), BindState::Conflict, Some(Detect::Decline))]);
    t.eq("decline counted", e.metric(srv, "dhcpd", "declines"), Some(1));
}

// ---------------------------------------------------------------- renewal, expiry, release

/// PC1 - SW - SRV (dhcpd: 60 s leases, no ping check): (engine, pc1, srv, link of srv, ok)
fn short_leases() -> (Engine, u32, u32, u32, bool) {
    let mut p = Project::new("leases", 13);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let srv = p.add_device("server-1u", 50, 200).unwrap_or(0);
    let sw = p.add_device("jf-s24p", 50, 100).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(sw, "gi1/0/1"), None);
    let link = p.connect(&pr(srv, "eth0"), &pr(sw, "gi1/0/2"), None).unwrap_or(0);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    let ok = [
        addr(&mut e, srv, "eth0", "10.0.0.53/24"),
        e.apply(Action::Service { dev: srv, def: def("dhcp-server", "dhcpd", r#"{"pools": [{"name": "lan", "network": "10.0.0.0/24", "range": ["10.0.0.100", "10.0.0.150"], "dns": ["10.0.0.53"], "lease": 60}], "ping_packets": 0}"#) }),
        client(&mut e, pc1, "eth0", true),
    ];
    (e, pc1, srv, link, ok.iter().all(|r| r.is_ok()))
}

fn lifecycle(t: &mut T) {
    let (mut e, pc1, srv, link, ok) = short_leases();
    t.check("lease set-up", ok);
    e.run_until(10 * SEC, BUDGET);
    let first = lease(&e, pc1);
    t.eq("bound", first.as_ref().map(|l| (l.addr.addr, l.lease, l.t2 - l.obtained, l.expiry - l.obtained)), Some((ip("10.0.0.150"), 60, 52 * SEC, 60 * SEC)));
    let t1 = first.as_ref().map(|l| l.t1).unwrap_or(0);
    // renewing: a unicast REQUEST to the server at T1, the ACK extends the lease
    e.run_until(t1 + SEC, BUDGET);
    t.eq("renew at t1", when(&e, pc1, |n| matches!(n, Note::DhcpRequest { kind: DhcpReq::Renew, .. })), Some(t1));
    let renewed = lease(&e, pc1);
    t.check("renewed", renewed.as_ref().is_some_and(|l| l.obtained > t1 - 1 && l.obtained < t1 + MS && l.addr.addr == ip("10.0.0.150")) && state(&e, pc1) == Some(ClientState::Bound));
    let renew_frame = e.trace.hops().find(|h| h.from.0 == pc1 && h.t_tx >= t1 && netpkt::decode::info(&h.bytes).1.starts_with("DHCP Request")).map(|h| h.bytes.to_vec());
    t.check("renewal is unicast", renew_frame.is_some_and(|f| f[..6] == port_mac(srv, 0).0));
    // the server goes away: RENEWING, REBINDING (broadcast after T2), expiry
    let _ = e.apply(Action::Unplug { link, down: true });
    let l2 = renewed.unwrap_or_else(|| Lease { addr: pf("0.0.0.0/0"), routers: Vec::new(), dns: Vec::new(), domain: None, server: Ipv4::UNSPECIFIED, lease: 0, obtained: 0, t1: 0, t2: 0, expiry: 0 });
    e.run_until(l2.expiry + SEC, BUDGET);
    t.eq("rebind at t2", when(&e, pc1, |n| matches!(n, Note::DhcpRequest { kind: DhcpReq::Rebind, .. })), Some(l2.t2));
    t.eq("expired", when(&e, pc1, |n| *n == Note::DhcpExpired { ip: ip("10.0.0.150") }), Some(l2.expiry));
    t.eq("address gone", addrs(&e, pc1, "eth0"), Vec::new());
    t.eq("resolver forgot dhcp", e.resolv_conf(pc1).map(|c| c.servers), Some(Vec::new()));
    t.eq("looking again", state(&e, pc1), Some(ClientState::Selecting));
    // back: the same address again (dhcpd remembers the client)
    let _ = e.apply(Action::Unplug { link, down: false });
    e.run_until(l2.expiry + 40 * SEC, BUDGET);
    t.eq("same address again", lease(&e, pc1).map(|l| l.addr.addr), Some(ip("10.0.0.150")));
    // a power cycle: INIT-REBOOT asks for the address of the lease file
    let _ = e.apply(Action::Power { dev: pc1, on: false });
    t.eq("power off drops the address", addrs(&e, pc1, "eth0"), Vec::new());
    let on = e.now();
    let _ = e.apply(Action::Power { dev: pc1, on: true });
    e.run_until(on + 10 * SEC, BUDGET);
    t.eq("reboot request", when(&e, pc1, |n| matches!(n, Note::DhcpRequest { kind: DhcpReq::Reboot, .. })), Some(on));
    t.eq("rebooted", lease(&e, pc1).map(|l| l.addr.addr), Some(ip("10.0.0.150")));
    // release ("dhclient -r"): the RELEASE goes to the server (after ARP, the power cycle
    // emptied the cache); the script takes the address away 10 ms later; the client stays idle
    t.check("release", client(&mut e, pc1, "eth0", false).is_ok());
    let rel = e.now();
    e.run_until(rel + 5 * MS, BUDGET);
    t.eq("address kept for the release", addrs(&e, pc1, "eth0"), vec![pf("10.0.0.150/24")]);
    t.check("released", notes(&e, pc1).contains(&Note::DhcpReleased { ip: ip("10.0.0.150"), server: ip("10.0.0.53") }) && notes(&e, srv).contains(&Note::DhcpdRelease { ip: ip("10.0.0.150") }));
    t.eq("no binding left", e.dhcp_bindings(srv).len(), 0);
    e.run_until(rel + SEC, BUDGET);
    t.eq("client idle", (state(&e, pc1), addrs(&e, pc1, "eth0")), (Some(ClientState::Stopped), Vec::new()));
    t.eq("renew without client", e.apply(Action::DhcpRenew { dev: pc1, iface: String::from("lo") }).is_err(), true);
    // switched on again, the client starts over
    t.check("client again", client(&mut e, pc1, "eth0", true).is_ok());
    e.run_until(e.now() + 10 * SEC, BUDGET);
    t.eq("leased again", lease(&e, pc1).map(|l| l.addr.addr), Some(ip("10.0.0.150")));
}

// ---------------------------------------------------------------- DNS

/// PC1 (10.0.0.10) - SW - NS (10.0.0.53, zones lab, corp and 0.0.10.in-addr.arpa) and R1
/// (10.0.0.1, a forwarder): (engine, pc1, ns, r1, ok)
fn dns_lan() -> (Engine, u32, u32, u32, bool) {
    let mut p = Project::new("dns", 17);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let ns = p.add_device("server-1u", 50, 200).unwrap_or(0);
    let r1 = p.add_device("jf-r4", 100, 200).unwrap_or(0);
    let sw = p.add_device("jf-s24p", 50, 100).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(sw, "gi1/0/1"), None);
    let _ = p.connect(&pr(ns, "eth0"), &pr(sw, "gi1/0/2"), None);
    let _ = p.connect(&pr(r1, "gi0/0/0"), &pr(sw, "gi1/0/3"), None);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    let zones = r#"{"zones": [
        {"origin": "lab", "ttl": 300, "records": ["@ MX 10 mail", "mail A 10.0.0.25", "www CNAME web", "web A 10.0.0.80", "web A 10.0.0.81", "info TXT \"hello lab\"", "sub NS ns.sub", "ns.sub A 10.0.0.60", "x.y A 10.0.0.90"]},
        {"origin": "corp", "records": ["host 60 A 10.0.0.70"]},
        {"origin": "0.0.10.in-addr.arpa", "records": ["80 PTR web.lab."]}]}"#;
    let ok = [
        addr(&mut e, pc1, "eth0", "10.0.0.10/24"),
        addr(&mut e, ns, "eth0", "10.0.0.53/24"),
        addr(&mut e, r1, "Gi0/0/0", "10.0.0.1/24"),
        if_up(&mut e, r1, "Gi0/0/0"),
        e.apply(Action::Service { dev: ns, def: def("dns-server", "named", zones) }),
        e.apply(Action::Resolver { dev: pc1, servers: Some(vec![ip("10.0.0.53")]), search: Some(vec![String::from("lab"), String::from("corp")]) }),
    ];
    (e, pc1, ns, r1, ok.iter().all(|r| r.is_ok()))
}

/// looks a name up and runs until it is done: (result, done at)
fn look(e: &mut Engine, dev: u32, name: &str, qtype: u16) -> (Option<Result<jfn_sim::Answer, LookupErr>>, u64) {
    let start = e.now();
    let id = e.lookup(dev, name, qtype).unwrap_or(0);
    e.run_until(start + 30 * SEC, BUDGET);
    let l = e.lookup_state(dev, id);
    (l.and_then(|l| l.result.clone()), l.and_then(|l| l.done).map(|d| d - start).unwrap_or(0))
}

fn addrs_of(r: &Option<Result<jfn_sim::Answer, LookupErr>>) -> Option<Result<Vec<Ipv4>, LookupErr>> {
    r.clone().map(|x| x.map(|a| a.addrs))
}

fn dns_server(t: &mut T) {
    let (mut e, pc1, ns, r1, ok) = dns_lan();
    t.check("dns set-up", ok);
    e.run_until(SEC, BUDGET);
    let (www, _) = look(&mut e, pc1, "www", dns::TYPE_A);
    t.eq("cname and two addresses", www.clone().map(|r| r.map(|a| (a.name, a.canonical, a.addrs))), Some(Ok((String::from("www.lab"), String::from("web.lab"), vec![ip("10.0.0.80"), ip("10.0.0.81")]))));
    let (mx, _) = look(&mut e, pc1, "lab.", dns::TYPE_MX);
    t.eq("mx", mx.map(|r| r.map(|a| a.records.iter().map(|r| r.data.clone()).collect::<Vec<_>>())), Some(Ok(vec![RData::Mx { pref: 10, host: String::from("mail.lab") }])));
    let (txt, _) = look(&mut e, pc1, "info.lab", dns::TYPE_TXT);
    t.eq("txt", txt.map(|r| r.map(|a| a.records.first().map(|r| r.data.text()))), Some(Ok(Some(String::from("\"hello lab\"")))));
    let (ptr, _) = look(&mut e, pc1, &dns::reverse_v4(ip("10.0.0.80")), dns::TYPE_PTR);
    t.eq("ptr", ptr.map(|r| r.map(|a| a.records.first().map(|r| r.data.clone()))), Some(Ok(Some(RData::Ptr(String::from("web.lab"))))));
    let (aaaa, _) = look(&mut e, pc1, "web.lab", dns::TYPE_AAAA);
    t.eq("no data", addrs_of(&aaaa), Some(Err(LookupErr::NoData)));
    t.check("no data noted", notes(&e, ns).contains(&Note::DnsNoData { name: String::from("web.lab"), qtype: dns::TYPE_AAAA }));
    // "y.lab" has no records but a name below it (an empty non-terminal): no data, not NXDOMAIN
    let (y, _) = look(&mut e, pc1, "y.lab.", dns::TYPE_A);
    t.eq("empty non-terminal", addrs_of(&y), Some(Err(LookupErr::NoData)));
    let (deleg, _) = look(&mut e, pc1, "a.sub.lab.", dns::TYPE_A);
    t.check("referral", notes(&e, ns).contains(&Note::DnsReferral { name: String::from("a.sub.lab"), zone: String::from("sub.lab") }) && deleg.is_some());
    // outside every zone: REFUSED (twice, two attempts), the lookup fails
    let (out, _) = look(&mut e, pc1, "example.com.", dns::TYPE_A);
    t.eq("refused", addrs_of(&out), Some(Err(LookupErr::Refused)));
    t.eq("refused twice", notes(&e, ns).iter().filter(|n| **n == Note::DnsRefused { name: String::from("example.com") }).count(), 2);
    // the zone can change while the server runs
    let r = e.apply(Action::DnsRecord { dev: ns, rr: Rr::new("new.lab", 60, RData::A(ip("10.0.0.99"))), add: true });
    let (new, _) = look(&mut e, pc1, "new", dns::TYPE_A);
    t.check("record added", r.is_ok() && addrs_of(&new) == Some(Ok(vec![ip("10.0.0.99")])));
    t.check("no zone for it", e.apply(Action::DnsRecord { dev: ns, rr: Rr::new("x.example", 60, RData::A(ip("10.0.0.1"))), add: true }).is_err());
    t.eq("zones", e.dns_zones(ns).iter().map(|z| z.origin.clone()).collect::<Vec<_>>(), vec![String::from("lab"), String::from("corp"), String::from("0.0.10.in-addr.arpa")]);
    // R1 forwards names it has no zone for to its own resolver, which asks NS
    let fw = [
        e.apply(Action::DnsForward { dev: r1, on: true }),
        e.apply(Action::Resolver { dev: r1, servers: Some(vec![ip("10.0.0.53")]), search: None }),
        e.apply(Action::Resolver { dev: pc1, servers: Some(vec![ip("10.0.0.1")]), search: None }),
    ];
    t.check("forwarder set-up", fw.iter().all(|r| r.is_ok()));
    let _ = e.apply(Action::ClearDnsCache { dev: pc1 });
    let (via, _) = look(&mut e, pc1, "mail.lab", dns::TYPE_A);
    t.eq("forwarded", addrs_of(&via), Some(Ok(vec![ip("10.0.0.25")])));
    t.check("forward noted", notes(&e, r1).contains(&Note::DnsForward { name: String::from("mail.lab") }));
    t.eq("forwarder cached it", e.resolver_cache(r1).iter().map(|c| (c.name.clone(), c.qtype, c.negative)).collect::<Vec<_>>(), vec![(String::from("mail.lab"), dns::TYPE_A, false)]);
    t.eq("forwarder counts", e.metric(r1, "dns-server", "forwarded"), Some(1));
}

fn resolver(t: &mut T) {
    let (mut e, pc1, ns, _r1, _) = dns_lan();
    e.run_until(SEC, BUDGET);
    // search order: "host" is tried as host.lab (NXDOMAIN), then host.corp
    let (host, _) = look(&mut e, pc1, "host", dns::TYPE_A);
    t.eq("search domains", host.clone().map(|r| r.map(|a| (a.name, a.addrs, a.ttl))), Some(Ok((String::from("host.corp"), vec![ip("10.0.0.70")], 60))));
    let asked: Vec<String> = notes(&e, pc1).iter().filter_map(|n| if let Note::ResolveQuery { name, .. } = n { Some(name.clone()) } else { None }).collect();
    t.eq("questions", asked, vec![String::from("host.lab"), String::from("host.corp")]);
    // the cache answers again, negative answers too; after the TTL the server is asked again
    let before = e.metric(ns, "named", "queries");
    let (again, took) = look(&mut e, pc1, "host", dns::TYPE_A);
    t.eq("from the cache", (again.map(|r| r.map(|a| a.from)), took, e.metric(ns, "named", "queries")), (Some(Ok(Source::Cache)), 0, before));
    let cache: Vec<(String, bool, u32)> = e.resolver_cache(pc1).iter().map(|c| (c.name.clone(), c.negative, c.ttl)).collect();
    t.check("cache rows", cache.iter().any(|c| c.0 == "host.corp" && !c.1) && cache.iter().any(|c| c.0 == "host.lab" && c.1));
    e.run_until(e.now() + 61 * SEC, BUDGET);
    let (later, _) = look(&mut e, pc1, "host.corp", dns::TYPE_A);
    t.eq("ttl over: asked again", later.map(|r| r.map(|a| a.from)), Some(Ok(Source::Server(ip("10.0.0.53")))));
    // nothing anywhere: every candidate NXDOMAIN or REFUSED
    let (none, _) = look(&mut e, pc1, "nosuch", dns::TYPE_A);
    t.check("unknown name", matches!(none, Some(Err(LookupErr::NotFound | LookupErr::Refused))));
    // host table and literals need no server
    let _ = e.apply(Action::HostEntry { dev: pc1, name: String::from("printer"), ip: Some(ip("10.0.0.7")) });
    let (pr_, took) = look(&mut e, pc1, "printer", dns::TYPE_A);
    t.eq("hosts", (pr_.map(|r| r.map(|a| (a.addrs, a.from))), took), (Some(Ok((vec![ip("10.0.0.7")], Source::Hosts))), 0));
    let (lit, _) = look(&mut e, pc1, "10.9.8.7", dns::TYPE_A);
    t.eq("literal", lit.map(|r| r.map(|a| (a.addrs, a.from))), Some(Ok((vec![ip("10.9.8.7")], Source::Literal))));
    let (lh, _) = look(&mut e, pc1, "localhost", dns::TYPE_A);
    t.eq("localhost", addrs_of(&lh), Some(Ok(vec![Ipv4::LOCALHOST])));
    let (badname, _) = look(&mut e, pc1, "a..b", dns::TYPE_A);
    t.eq("bad name", addrs_of(&badname), Some(Err(LookupErr::BadName)));
    // failover: the first server does not exist, after 5 s the second one answers
    let _ = e.apply(Action::Resolver { dev: pc1, servers: Some(vec![ip("10.0.0.99"), ip("10.0.0.53")]), search: None });
    let _ = e.apply(Action::ClearDnsCache { dev: pc1 });
    let (fo, took) = look(&mut e, pc1, "mail.lab", dns::TYPE_A);
    t.eq("failover", addrs_of(&fo), Some(Ok(vec![ip("10.0.0.25")])));
    t.check("after the timeout", (5 * SEC..5 * SEC + 20_000).contains(&took));
    t.check("timeout noted", notes(&e, pc1).contains(&Note::ResolveTimeout { name: String::from("mail.lab"), server: ip("10.0.0.99") }));
    // nobody answers: two rounds of 5 s, then the lookup gives up
    let _ = e.apply(Action::Resolver { dev: pc1, servers: Some(vec![ip("10.0.0.99")]), search: None });
    let (dead, took) = look(&mut e, pc1, "web.lab", dns::TYPE_A);
    t.eq("timeout", (addrs_of(&dead), took), (Some(Err(LookupErr::Timeout)), 10 * SEC));
    let _ = e.apply(Action::Resolver { dev: pc1, servers: Some(Vec::new()), search: None });
    let (no, _) = look(&mut e, pc1, "web.lab", dns::TYPE_A);
    t.eq("no server", addrs_of(&no), Some(Err(LookupErr::NoServer)));
    // the durations of the lookups of the last five minutes (base::metrics::Window)
    let lat = e.resolver_latency(pc1);
    t.check("lookup latency", lat.count >= 8 && lat.max == 10 * SEC && lat.p50 < SEC && lat.p99 >= 5 * SEC);
    // ping by a name that does not resolve: no probes, the lookup tells why
    let _ = e.apply(Action::Resolver { dev: pc1, servers: Some(vec![ip("10.0.0.53")]), search: Some(Vec::new()) });
    let id = e.ping_host(pc1, "gone.lab", PingOpts::default()).unwrap_or(0);
    e.run_until(e.now() + 5 * SEC, BUDGET);
    let (sent, lookup) = match e.job(id).map(|j| (j.done, &j.kind)) {
        Some((true, JobKind::Ping(p))) => (p.sent, p.lookup),
        _ => (99, None),
    };
    t.eq("ping without address", (sent, lookup.and_then(|l| e.lookup_state(pc1, l)).and_then(|l| l.result.clone()).map(|r| r.err())), (0, Some(Some(LookupErr::NotFound))));
}

// ---------------------------------------------------------------- syslog and NTP

fn syslog_ntp(t: &mut T) {
    let mut p = Project::new("logs", 23);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let srv = p.add_device("server-1u", 50, 200).unwrap_or(0);
    let r1 = p.add_device("jf-r4", 100, 200).unwrap_or(0);
    let sw = p.add_device("jf-s24p", 50, 100).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(sw, "gi1/0/1"), None);
    let _ = p.connect(&pr(srv, "eth0"), &pr(sw, "gi1/0/2"), None);
    let _ = p.connect(&pr(r1, "gi0/0/0"), &pr(sw, "gi1/0/3"), None);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    let ok = [
        addr(&mut e, pc1, "eth0", "10.0.0.10/24"),
        addr(&mut e, srv, "eth0", "10.0.0.53/24"),
        addr(&mut e, r1, "Gi0/0/0", "10.0.0.1/24"),
        if_up(&mut e, r1, "Gi0/0/0"),
        e.apply(Action::Service { dev: srv, def: def("syslog-server", "rsyslog", "{}") }),
        e.apply(Action::NtpMaster { dev: srv, stratum: Some(3) }),
        e.apply(Action::LogHost { dev: r1, host: ip("10.0.0.53"), add: true }),
        e.apply(Action::LogHost { dev: pc1, host: ip("10.0.0.53"), add: true }),
    ];
    t.check("log set-up", ok.iter().all(|r| r.is_ok()));
    // SRV pings both first, so every ARP cache is warm (IOS would drop the first message
    // while it asks, and the first NTP exchange would wait for ARP on one way only)
    let _ = e.ping(srv, ip("10.0.0.1"), PingOpts { count: 1, ..PingOpts::default() });
    let _ = e.ping(srv, ip("10.0.0.10"), PingOpts { count: 1, ..PingOpts::default() });
    e.run_until(SEC, BUDGET);
    let _ = e.apply(Action::Log { dev: r1, sev: 5, tag: String::from("%SYS-5-CONFIG_I"), text: String::from("Configured from console by console") });
    let _ = e.apply(Action::Log { dev: pc1, sev: 6, tag: String::from("logger"), text: String::from("hello") });
    let _ = e.apply(Action::Log { dev: r1, sev: 7, tag: String::from("%DEBUG"), text: String::from("too much") });
    e.run_until(2 * SEC, BUDGET);
    let got: Vec<(Ipv4, u8, String)> = e.syslog_messages(srv).into_iter().map(|m| (m.from, m.pri, m.text)).collect();
    t.eq("two messages", got.len(), 2);
    // the device clocks start up to 2 s off: the seconds are not checked
    let from = |a: &str| got.iter().find(|m| m.0 == ip(a)).map(|m| (m.1, m.2.clone())).unwrap_or_default();
    let (ios_pri, ios_text) = from("10.0.0.1");
    t.check("ios form", ios_pri == 189 && ios_text.starts_with("1: *Jan  1 00:00:0") && ios_text.ends_with(": %SYS-5-CONFIG_I: Configured from console by console"));
    let (lin_pri, lin_text) = from("10.0.0.10");
    t.check("linux form", lin_pri == 30 && lin_text.starts_with("Jan  1 00:00:0") && lin_text.ends_with(" PC1 logger: hello"));
    t.eq("debug stays local", (e.log_lines(r1).len(), e.metric(srv, "rsyslog", "messages")), (2, Some(2)));
    // NTP: SRV serves its own clock at stratum 3; R1 and PC1 follow it
    let _ = e.apply(Action::NtpServer { dev: r1, server: ip("10.0.0.53"), add: true });
    let _ = e.apply(Action::NtpServer { dev: pc1, server: ip("10.0.0.53"), add: true });
    let start = e.now();
    e.run_until(start + 10 * US, BUDGET);
    let off = |e: &Engine, d: u32| e.clock(d).unwrap_or(0) - e.clock(srv).unwrap_or(0);
    // the first answer steps the clock to the server's time; PC1's request waited 912 ns at
    // the switch behind R1's (both left at the same moment): that asymmetry puts PC1 half of
    // it, 456 ns, ahead
    t.eq("stepped", (off(&e, r1), off(&e, pc1)), (0, 456));
    e.run_until(start + SEC, BUDGET);
    let st = e.clock_state(r1);
    t.eq("synced", st.as_ref().map(|c| (c.synced, c.stratum, c.source)), Some((true, 4, Some(ip("10.0.0.53")))));
    t.check("step noted", notes(&e, r1).iter().any(|n| matches!(n, Note::NtpStep { .. })));
    t.check("peer sync logged", e.log_lines(r1).iter().any(|l| l.tag == "%NTP-5-PEERSYNC" && l.text == "NTP synced to peer 10.0.0.53"));
    // the oscillators drift apart (up to 60 ppm: 3.8 ms in 64 s); later answers slew the
    // clock and correct the rate, so the offset shrinks
    e.run_until(start + 64 * SEC - MS, BUDGET);
    let drift = off(&e, r1).abs();
    e.run_until(start + 6 * 64 * SEC + SEC, BUDGET);
    let late = off(&e, r1).abs();
    t.check("drift before the second poll", drift > 0 && drift < 4 * MS as i64);
    t.check("slewed", notes(&e, r1).iter().any(|n| matches!(n, Note::NtpSlew { .. })));
    t.check("rate corrected", late < drift / 50);
    // R1's oscillator plus the correction runs exactly as fast as SRV's
    let (rc, sc) = (e.clock_state(r1), e.clock_state(srv));
    t.eq("rates match", rc.map(|c| c.drift_ppb + c.freq_ppb), sc.map(|c| c.drift_ppb));
    t.eq("served", e.metric(srv, "ntp", "served"), Some(14));
    let peers = e.ntp_peers(r1);
    t.eq("reach", peers.iter().map(|p| (p.addr, p.reach, p.stratum)).collect::<Vec<_>>(), vec![(ip("10.0.0.53"), 0x7F, 3)]);
    // an unsynchronised device does not answer
    let _ = e.apply(Action::NtpServer { dev: srv, server: ip("10.0.0.1"), add: true });
    let _ = e.apply(Action::NtpMaster { dev: srv, stratum: None });
    let _ = e.apply(Action::ServiceRun { dev: srv, name: String::from("ntp"), on: false });
    let _ = e.apply(Action::ServiceRun { dev: srv, name: String::from("ntp"), on: true });
    let _ = e.apply(Action::ServiceRun { dev: r1, name: String::from("ntp"), on: false });
    let _ = e.apply(Action::ServiceRun { dev: r1, name: String::from("ntp"), on: true });
    e.run_until(e.now() + 2 * SEC, BUDGET);
    t.check("no time from an unsynchronised server", notes(&e, srv).iter().any(|n| matches!(n, Note::NtpNoTime { .. })) || notes(&e, r1).iter().any(|n| matches!(n, Note::NtpNoTime { .. })));
    let set = e.apply(Action::ClockSet { dev: pc1, unix_ns: 1_800_000_000 * SEC as i64 });
    t.check("clock set", set.is_ok() && e.clock(pc1) == Some(1_800_000_000 * SEC as i64));
}

// ---------------------------------------------------------------- views and definitions

fn views_defs(t: &mut T) {
    let (mut e, pcs, r1, srv1, _srv2, _) = campus(21);
    e.run_until(10 * SEC, BUDGET);
    let v = e.services(srv1);
    t.eq("services", v.iter().map(|s| (s.name.as_str(), s.kind, s.running)).collect::<Vec<_>>(), vec![("isc-dhcp-server", "dhcp-server", true), ("bind9", "dns-server", true)]);
    t.check("status lines", v[0].status.contains("Leases") && v[1].status.contains("lab"));
    t.eq("client service", e.services(pcs[0]).iter().map(|s| (s.name.clone(), s.kind)).collect::<Vec<_>>(), vec![(String::from("dhcp-client@eth0"), "dhcp-client")]);
    let mut ports: Vec<u16> = e.sockets(srv1).iter().map(|s| s.port).collect();
    ports.sort();
    t.eq("server sockets", ports, vec![53, 67]);
    t.eq("relay helpers", e.dhcp_helpers(r1), vec![(String::from("GigabitEthernet0/0/0"), ip("10.0.2.10"))]);
    t.check("relay status", e.services(r1).first().is_some_and(|s| s.status.contains("Relay")));
    // a stopped service frees its port; a running one keeps the engine busy
    t.check("stop", e.apply(Action::ServiceRun { dev: srv1, name: String::from("bind9"), on: false }).is_ok());
    t.eq("port freed", e.sockets(srv1).iter().any(|s| s.port == 53), false);
    t.check("unknown service", e.apply(Action::ServiceRun { dev: srv1, name: String::from("nginx"), on: true }).is_err());
    t.check("unknown kind", e.apply(Action::Service { dev: srv1, def: def("ftp", "ftp", "{}") }).is_err());
    t.check("bad zone line", e.apply(Action::Service { dev: srv1, def: def("dns-server", "dns2", r#"{"zones": [{"origin": "x", "records": ["a QQ b"]}]}"#) }).is_err());
    t.check("client needs an interface", e.apply(Action::Service { dev: srv1, def: def("dhcp-client", "c", "{}") }).is_err());
    t.check("hub has no os", e.apply(Action::Log { dev: 999, sev: 1, tag: String::new(), text: String::new() }).is_err());
    // Linux: a relay and a server cannot share port 67
    t.check("relay beside server refused", e.apply(Action::DhcpHelper { dev: srv1, iface: String::from("eth0"), server: ip("10.0.9.9"), add: true }).is_err());
    let z = jfn_sim::parse_record("@ 600 IN SOA ns hostmaster 7 1 2 3 4", "lab", 300);
    t.check("soa line", z.is_some_and(|r| r.ttl == 600 && r.name == "lab" && matches!(r.data, RData::Soa(ref s) if s.serial == 7 && s.mname == "ns.lab")));
    t.eq("absolute name", jfn_sim::parse_record("www CNAME web.example.", "lab", 300).map(|r| (r.name, r.data)), Some((String::from("www.lab"), RData::Cname(String::from("web.example")))));
    // the engine clones with its services: both copies go on alike
    let mut c = e.clone();
    e.run_until(20 * SEC, BUDGET);
    c.run_until(20 * SEC, BUDGET);
    t.check("clone runs alike", e.trace.hops().count() == c.trace.hops().count() && e.dhcp_bindings(srv1) == c.dhcp_bindings(srv1));
    // IfKind is re-exported for the views of DHCP interfaces
    t.check("pc interface kind", e.ifaces(pcs[0]).iter().any(|f| f.kind == IfKind::Port(0)));
}

fn determinism(t: &mut T) {
    let run = |seed: u64| {
        let (mut e, pcs, _, _, _, _) = campus(seed);
        e.run_until(10 * SEC, BUDGET);
        let _ = e.ping_host(pcs[1], "www", PingOpts { count: 2, ..PingOpts::default() });
        e.run_until(20 * SEC, BUDGET);
        e.trace.hops().map(|h| (h.t_tx, h.from, h.to, h.bytes.to_vec())).collect::<Vec<_>>()
    };
    let a = run(21);
    t.check("a busy run", a.len() > 60);
    t.check("two runs alike", a == run(21));
    t.check("another seed, other xids", a != run(22));
}
