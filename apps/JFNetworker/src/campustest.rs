// campustest -- checks of the campus of the acceptance (tests/fixtures/campus.jfnet, part of
// --selftest): the fixture boots without complaints; the four PCs get their leases from R1
// over MLS1's relay on both VLAN interfaces; names resolve with the domain from DHCP for ping
// and traceroute (canonical names, search domain, NXDOMAIN, a silent name server, Ctrl+C while
// looking up); dhclient renews straight from R1 over two routers, releases and binds again;
// the files of the resolver; the IOS DHCP commands (the pool mode with pools that wait for
// their network, lease, excluded addresses, helpers, show and clear, the running configuration
// read back as the startup configuration) and NTP (R1 and MLS1 set their clocks from SRV1,
// which also tells them SRV1's MAC address before the first name lookup); a ping of a PC that
// never talked shows ARP first, then ICMP with correct checksums on every cable.
//
// Device ids of the fixture: R1 = 1, MLS1 = 2, SRV1 = 3, PC1 = 4, PC2 = 5, PC3 = 6, PC4 = 7.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::project::Project;
use jfn_sim::{Action, BindState, Engine, Report, SEC};
use netpkt::decode::{P_ARP, P_ICMP};
use netpkt::{Ipv4, Prefix4};

use crate::selftest::T;

const BUDGET: usize = 5_000_000;
const CAMPUS: &str = include_str!("../tests/fixtures/campus.jfnet");

const R1: u32 = 1;
const MLS1: u32 = 2;
const SRV1: u32 = 3;
const PC1: u32 = 4;
const PC2: u32 = 5;
const PC3: u32 = 6;
const PC4: u32 = 7;

fn ip(s: &str) -> Ipv4 {
    Ipv4::parse(s).unwrap_or(Ipv4::UNSPECIFIED)
}

fn pf(s: &str) -> Prefix4 {
    Prefix4::parse(s).unwrap_or(Prefix4 { addr: Ipv4::UNSPECIFIED, len: 0 })
}

fn project() -> Project {
    Project::from_text(CAMPUS).unwrap_or_else(|_| Project::new("campus", 5))
}

/// the campus, `secs` seconds after the start
fn campus(secs: u64) -> Engine {
    let mut e = Engine::from_project(&project()).unwrap_or_else(|_| Engine::new(0));
    e.run_until(secs * SEC, BUDGET);
    let _ = e.take_reports();
    e
}

/// what a device printed since the last call
fn out(e: &mut Engine, dev: u32) -> String {
    let mut s = String::new();
    for r in e.take_reports() {
        if let Report::CliOut { dev: d, bytes, .. } = r
            && d == dev
        {
            s.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    s
}

/// a line typed at a device, then `run` of virtual time: what the device printed
fn cli_run(e: &mut Engine, dev: u32, line: &str, run: u64) -> String {
    let _ = e.take_reports();
    let _ = e.cli_line(dev, line);
    let t = e.now() + run;
    e.run_until(t, BUDGET);
    out(e, dev)
}

/// a line typed at a device (no time passes)
fn cli(e: &mut Engine, dev: u32, line: &str) -> String {
    cli_run(e, dev, line, 0)
}

/// the first address of eth0
fn eth0(e: &Engine, dev: u32) -> Option<Prefix4> {
    e.ifaces(dev).into_iter().find(|f| f.name == "eth0").and_then(|f| f.addrs.first().copied())
}

pub fn run(t: &mut T) {
    fixture(t);
    leases(t);
    names(t);
    dhclient(t);
    files(t);
    ios_dhcp(t);
    ios_ntp(t);
    first_ping(t);
}

fn fixture(t: &mut T) {
    let p = project();
    t.eq("devices", p.devices.len(), 7);
    t.eq("links", p.links.len(), 6);
    t.eq("notes", p.notes.len(), 3);
    let names: Vec<&str> = p.devices.iter().map(|d| d.name.as_str()).collect();
    t.eq("names", names, alloc::vec!["R1", "MLS1", "SRV1", "PC1", "PC2", "PC3", "PC4"]);
    let e = campus(0);
    for d in [R1, MLS1, SRV1, PC1, PC2, PC3, PC4] {
        t.check(&format!("{} boots cleanly", d), e.boot_errors(d).is_empty());
    }
    // what the startup configurations give, written back
    let r1 = e.running_config(R1);
    t.check("R1 pool", r1.contains("ip dhcp pool STAFF\n network 10.10.10.0 255.255.255.0\n default-router 10.10.10.1\n dns-server 10.10.100.10\n domain-name lab\n!\n"));
    t.check("R1 excluded", r1.contains("ip dhcp excluded-address 10.10.10.1 10.10.10.99\nip dhcp excluded-address 10.10.20.1 10.10.20.99\n!\n"));
    t.check("R1 ntp at the end", r1.ends_with("line vty 0 4\n login\n!\nntp server 10.10.100.10\n!\nend\n"));
    let mls = e.running_config(MLS1);
    t.check("MLS1 helper 10", mls.contains("interface Vlan10\n description gateway of VLAN 10 Staff\n ip address 10.10.10.1 255.255.255.0\n ip helper-address 10.10.0.1\n!\n"));
    t.check("MLS1 helper 20", mls.contains("interface Vlan20\n description gateway of VLAN 20 Students\n ip address 10.10.20.1 255.255.255.0\n ip helper-address 10.10.0.1\n!\n"));
    t.check("MLS1 routed uplink", mls.contains("interface TenGigabitEthernet1/1/1\n description uplink to R1\n no switchport\n ip address 10.10.0.2 255.255.255.252\n!\n"));
    t.eq("MLS1 helpers", e.dhcp_helpers(MLS1), alloc::vec![(String::from("Vlan10"), ip("10.10.0.1")), (String::from("Vlan20"), ip("10.10.0.1"))]);
    // read back: a device booted from its running configuration writes the same again
    for d in [R1, MLS1] {
        let text = e.running_config(d);
        let mut p2 = project();
        if let Some(x) = p2.device_mut(d) {
            x.startup_config = text.clone();
        }
        let e2 = Engine::from_project(&p2).unwrap_or_else(|_| Engine::new(0));
        t.eq(&format!("{} round trip", d), e2.running_config(d), text);
        t.check(&format!("{} round trip clean", d), e2.boot_errors(d).is_empty());
    }
}

fn leases(t: &mut T) {
    let mut e = campus(6);
    let want = [(PC1, "10.10.10.100/24", "10.10.10.1"), (PC2, "10.10.10.101/24", "10.10.10.1"), (PC3, "10.10.20.100/24", "10.10.20.1"), (PC4, "10.10.20.101/24", "10.10.20.1")];
    for (pc, a, gw) in want {
        t.eq(&format!("{} lease", pc), eth0(&e, pc), Some(pf(a)));
        let def = e.routes(pc).into_iter().find(|r| r.prefix.len == 0).and_then(|r| r.via);
        t.eq(&format!("{} gateway", pc), def, Some(ip(gw)));
        let rc = e.resolv_conf(pc);
        t.eq(&format!("{} name server", pc), rc.as_ref().map(|c| c.servers.clone()), Some(alloc::vec![ip("10.10.100.10")]));
        t.eq(&format!("{} search", pc), rc.map(|c| c.search), Some(alloc::vec![String::from("lab")]));
    }
    // R1 handed them out from the pool of the relay's VLAN interface
    let b: Vec<(Ipv4, String)> = e.dhcp_bindings(R1).into_iter().filter(|b| b.state == BindState::Bound).map(|b| (b.ip, b.pool)).collect();
    t.eq(
        "bindings",
        b,
        alloc::vec![(ip("10.10.10.100"), String::from("STAFF")), (ip("10.10.10.101"), String::from("STAFF")), (ip("10.10.20.100"), String::from("STUDENTS")), (ip("10.10.20.101"), String::from("STUDENTS"))]
    );
    t.check("relayed", e.metric(MLS1, "dhcp-server", "relayed").is_some_and(|n| n >= 4));
    t.check("relayed back", e.metric(MLS1, "dhcp-server", "relayed_back").is_some_and(|n| n >= 4));
    t.eq("R1 leases", e.metric(R1, "dhcp-server", "leases"), Some(4));
    // show ip dhcp binding (a day from the start of the clock: March 2nd)
    let _ = cli(&mut e, R1, "enable");
    let s = cli(&mut e, R1, "show ip dhcp binding");
    t.check("binding head", s.starts_with("Addresses this server has handed out:\nIP address          Client-ID/              Lease expiration        Type\n                    Hardware address/\n                    User name\n"));
    t.check("binding row", s.contains("10.10.10.100        0102.4a00.0004.01       Mar 02 1993 12:00 AM    Automatic\n"));
    t.eq("binding rows", s.lines().count(), 8);
    let s = cli(&mut e, R1, "show ip dhcp pool");
    t.check("pool staff", s.contains("\nPool STAFF :\n Network                        : 10.10.10.0/24\n Total addresses                : 254\n Excluded addresses             : 99\n Leased addresses               : 2\n"));
    t.check("pool lease", s.contains(" Lease                          : 1 day\n"));
    // MLS1 learned the four PCs in their VLANs
    let _ = cli(&mut e, MLS1, "enable");
    let s = cli(&mut e, MLS1, "show mac address-table");
    for (vlan, mac, port) in [(10, "024a.0000.0401", "Gi1/0/1"), (10, "024a.0000.0501", "Gi1/0/2"), (20, "024a.0000.0601", "Gi1/0/3"), (20, "024a.0000.0701", "Gi1/0/4")] {
        t.check(&format!("mac {}", port), s.contains(&format!("  {}    {}    DYNAMIC     {}\n", vlan, mac, port)));
    }
    t.check("mac total", s.contains("Total Mac Addresses for this criterion: 4\n"));
}

fn names(t: &mut T) {
    let mut e = campus(6);
    // by name: the first line names the address, the statistics the name
    let s = cli_run(&mut e, PC1, "ping -c 2 server.lab", 3 * SEC);
    t.check("ping head", s.contains("PING server.lab (10.10.100.10) 56(84) bytes of data.\n"));
    t.check("ping reply", s.contains("64 bytes from 10.10.100.10: icmp_seq=1 ttl=62 time="));
    t.check("ping stats", s.contains("--- server.lab ping statistics ---\n2 packets transmitted, 2 received, 0% packet loss, time 1000ms\n"));
    t.check("ping done", !e.cli_busy(PC1));
    // a CNAME: the canonical name; a short name: the search domain
    let s = cli_run(&mut e, PC2, "ping -c 1 srv1.lab", 2 * SEC);
    t.check("cname", s.contains("PING server.lab (10.10.100.10) 56(84) bytes of data.\n") && s.contains("--- server.lab ping statistics ---\n"));
    let s = cli_run(&mut e, PC2, "ping -c 1 server", 2 * SEC);
    t.check("search", s.contains("PING server.lab (10.10.100.10)") && s.contains("1 packets transmitted, 1 received, 0% packet loss"));
    // NXDOMAIN
    let s = cli_run(&mut e, PC1, "ping -c 1 nosuch.lab", 2 * SEC);
    t.eq("nxdomain", s.as_str(), "ping: nosuch.lab: Name or service not known\n");
    t.check("nxdomain done", !e.cli_busy(PC1));
    // traceroute by name over the L3 switch and the router
    let s = cli_run(&mut e, PC3, "traceroute server.lab", 2 * SEC);
    t.check("trace head", s.starts_with("traceroute to server.lab (10.10.100.10), 30 hops max, 60 byte packets\n"));
    t.check("trace 1", s.contains("\n 1  10.10.20.1 (10.10.20.1)  "));
    t.check("trace 2", s.contains("\n 2  10.10.0.1 (10.10.0.1)  "));
    t.check("trace 3", s.contains("\n 3  10.10.100.10 (10.10.100.10)  "));
    t.eq("trace lines", s.lines().count(), 4);
    let s = cli_run(&mut e, PC3, "traceroute -n server.lab", 2 * SEC);
    t.check("trace numeric", s.contains("\n 1  10.10.20.1  ") && s.contains("\n 3  10.10.100.10  "));
    let s = cli_run(&mut e, PC3, "traceroute nosuch.lab", 2 * SEC);
    t.eq("trace nxdomain", s.as_str(), "nosuch.lab: Name or service not known\nCannot handle \"host\" cmdline arg `nosuch.lab' on position 1 (argc 1)\n");
    // a name server that does not answer: glibc gives up after 2 x 5 s
    let _ = e.apply(Action::Resolver { dev: PC4, servers: Some(alloc::vec![ip("10.10.100.99")]), search: None });
    let s = cli_run(&mut e, PC4, "ping -c 1 server.lab", 5 * SEC);
    t.eq("lookup waits", s.as_str(), "");
    t.check("lookup busy", e.cli_busy(PC4));
    let t1 = e.now() + 6 * SEC;
    e.run_until(t1, BUDGET);
    t.eq("lookup gives up", out(&mut e, PC4).as_str(), "ping: server.lab: Temporary failure in name resolution\n");
    t.check("lookup done", !e.cli_busy(PC4));
    // Ctrl+C while looking up: no statistics
    let _ = e.apply(Action::Resolver { dev: PC4, servers: Some(alloc::vec![ip("10.10.100.98")]), search: None });
    let _ = cli_run(&mut e, PC4, "traceroute server.lab", SEC);
    let _ = e.cli_interrupt(PC4);
    t.eq("interrupted", out(&mut e, PC4).as_str(), "\n");
    t.check("interrupted done", !e.cli_busy(PC4));
}

fn dhclient(t: &mut T) {
    let mut e = campus(6);
    let before = e.dhcp_bindings(R1).into_iter().find(|b| b.ip == ip("10.10.10.100")).map(|b| b.until).unwrap_or(0);
    // a renewal goes straight to R1 (unicast over MLS1); R1 finds the pool by the client's
    // address, as there is no relay in between
    let s = cli_run(&mut e, PC1, "dhclient -v eth0", SEC);
    t.check("renew request", s.contains("DHCPREQUEST for 10.10.10.100 on eth0 to 10.10.0.1 port 67 (xid=0x"));
    t.check("renew ack", s.contains("DHCPACK of 10.10.10.100 from 10.10.0.1 (xid=0x"));
    t.check("renew bound", s.contains("bound to 10.10.10.100 -- renewal in "));
    t.eq("renew lines", s.lines().count(), 3);
    t.check("renew done", !e.cli_busy(PC1));
    let after = e.dhcp_bindings(R1).into_iter().find(|b| b.ip == ip("10.10.10.100")).map(|b| b.until).unwrap_or(0);
    t.check("lease extended", after > before);
    t.eq("no pool misses", e.metric(R1, "dhcp-server", "dropped"), Some(0));
    // release: the address goes, R1 forgets the binding
    let s = cli_run(&mut e, PC1, "dhclient -r -v eth0", SEC);
    t.check("release line", s.starts_with("DHCPRELEASE of 10.10.10.100 on eth0 to 10.10.0.1 port 67 (xid=0x"));
    t.eq("released address", eth0(&e, PC1), None);
    t.check("binding gone", !e.dhcp_bindings(R1).iter().any(|b| b.ip == ip("10.10.10.100")));
    t.eq("release again", cli_run(&mut e, PC1, "dhclient -r eth0", SEC).as_str(), "");
    // without -v: silent, the console waits until the client is bound
    let _ = e.take_reports();
    let _ = e.cli_line(PC1, "dhclient eth0");
    t.check("waits", e.cli_busy(PC1));
    let t1 = e.now() + 10 * SEC;
    e.run_until(t1, BUDGET);
    t.eq("silent", out(&mut e, PC1).as_str(), "");
    t.check("bound again", !e.cli_busy(PC1) && eth0(&e, PC1) == Some(pf("10.10.10.100/24")));
    t.eq("bad option", cli(&mut e, PC1, "dhclient -x eth0").as_str(), "dhclient: unknown option -x\nUsage: dhclient [-v] [-r] [INTERFACE]\n  -v  show the DHCP messages while they happen\n  -r  give the address back (DHCPRELEASE)\n");
    t.eq("bad interface", cli(&mut e, PC1, "dhclient eth9").as_str(), "dhclient: there is no interface eth9\n");
}

fn files(t: &mut T) {
    let mut e = campus(6);
    t.eq("resolv.conf", cli(&mut e, PC2, "cat /etc/resolv.conf").as_str(), "domain lab\nsearch lab\nnameserver 10.10.100.10\n");
    t.eq("hosts", cli(&mut e, PC2, "cat /etc/hosts").as_str(), "127.0.0.1\tlocalhost\n127.0.1.1\tPC2\n");
    t.eq("hostname", cli(&mut e, PC2, "cat /etc/hostname").as_str(), "PC2\n");
    t.eq("missing", cli(&mut e, PC2, "cat /etc/shadow").as_str(), "cat: /etc/shadow: No such file or directory\n");
    // set by hand: no "domain" line, the options that differ
    let _ = e.apply(Action::Resolver { dev: SRV1, servers: Some(alloc::vec![ip("10.10.100.10"), ip("10.10.100.1")]), search: Some(alloc::vec![String::from("lab"), String::from("example")]) });
    t.eq("static resolv.conf", cli(&mut e, SRV1, "cat /etc/resolv.conf").as_str(), "search lab example\nnameserver 10.10.100.10\nnameserver 10.10.100.1\n");
    let _ = e.apply(Action::HostEntry { dev: SRV1, name: String::from("gw"), ip: Some(ip("10.10.100.1")) });
    t.eq("hosts entry", cli(&mut e, SRV1, "cat /etc/hosts").as_str(), "127.0.0.1\tlocalhost\n127.0.1.1\tSRV1\n10.10.100.1\tgw\n");
}

fn ios_dhcp(t: &mut T) {
    let mut e = campus(6);
    for l in ["enable", "configure terminal", "ip dhcp pool TEST"] {
        let _ = cli(&mut e, R1, l);
    }
    t.eq("pool prompt", e.cli_prompt(R1).as_str(), "R1(dhcp-config)#");
    t.check("pool help", cli(&mut e, R1, "?").starts_with("Commands for this address pool:\n"));
    // without a network the pool waits on the command line
    let _ = cli(&mut e, R1, "default-router 192.168.5.1 192.168.5.2");
    let _ = cli(&mut e, R1, "lease 0 12");
    t.check("pending", !e.dhcp_pools(R1).iter().any(|p| p.name == "TEST"));
    t.check("pending config", e.running_config(R1).contains("ip dhcp pool TEST\n default-router 192.168.5.1 192.168.5.2\n lease 0 12\n!\n"));
    t.eq("host bits", cli(&mut e, R1, "network 192.168.5.7 255.255.255.0").as_str(), "% 192.168.5.7 is not a network address (192.168.5.0 is)\n");
    t.eq("network /nn", cli(&mut e, R1, "network 192.168.6.0 /24").as_str(), "");
    let p = e.dhcp_pools(R1).into_iter().find(|p| p.name == "TEST");
    t.eq("live network", p.as_ref().map(|p| p.cfg.network), Some(pf("192.168.6.0/24")));
    t.eq("live routers", p.as_ref().map(|p| p.cfg.routers.clone()), Some(alloc::vec![ip("192.168.5.1"), ip("192.168.5.2")]));
    t.eq("live lease", p.map(|p| p.cfg.lease), Some(Some(43_200)));
    for (l, want) in [("lease infinite", " lease infinite\n"), ("lease 7", " lease 7\n"), ("lease 0 0 30", " lease 0 0 30\n")] {
        let _ = cli(&mut e, R1, l);
        t.check(l, e.running_config(R1).contains(&format!("ip dhcp pool TEST\n network 192.168.6.0 255.255.255.0\n default-router 192.168.5.1 192.168.5.2\n{}!\n", want)));
    }
    let _ = cli(&mut e, R1, "no lease");
    let _ = cli(&mut e, R1, "dns-server 192.168.6.53");
    let _ = cli(&mut e, R1, "domain-name test.lab");
    t.check("pool lines", e.running_config(R1).contains("ip dhcp pool TEST\n network 192.168.6.0 255.255.255.0\n default-router 192.168.5.1 192.168.5.2\n dns-server 192.168.6.53\n domain-name test.lab\n!\n"));
    // no network: back to the command line
    let _ = cli(&mut e, R1, "no network");
    t.check("no network", !e.dhcp_pools(R1).iter().any(|p| p.name == "TEST") && e.running_config(R1).contains("ip dhcp pool TEST\n default-router 192.168.5.1 192.168.5.2\n dns-server"));
    // a global command leaves the pool mode
    let _ = cli(&mut e, R1, "ip dhcp excluded-address 10.10.10.200");
    t.eq("fell back", e.cli_prompt(R1).as_str(), "R1(config)#");
    t.check("single excluded", e.running_config(R1).contains("ip dhcp excluded-address 10.10.20.1 10.10.20.99\nip dhcp excluded-address 10.10.10.200\n!\n"));
    let _ = cli(&mut e, R1, "no ip dhcp excluded-address 10.10.10.200");
    t.check("excluded gone", !e.running_config(R1).contains("10.10.10.200"));
    t.eq("backwards range", cli(&mut e, R1, "ip dhcp excluded-address 10.10.10.9 10.10.10.1").as_str(), "% The range ends before it starts\n");
    let _ = cli(&mut e, R1, "no ip dhcp pool TEST");
    t.check("pool gone", !e.running_config(R1).contains("TEST"));
    t.eq("no such pool", cli(&mut e, R1, "no ip dhcp pool TEST").as_str(), "% There is no pool TEST\n");
    let h = cli(&mut e, R1, "ip dhcp ?");
    t.check("dhcp help", h.contains("  excluded-address  ") && h.contains("  pool  "));
    let _ = cli(&mut e, R1, "end");
    // clear ip dhcp binding
    let _ = cli(&mut e, R1, "clear ip dhcp binding 10.10.10.100");
    t.check("cleared one", !e.dhcp_bindings(R1).iter().any(|b| b.ip == ip("10.10.10.100")) && e.dhcp_bindings(R1).len() == 3);
    let _ = cli(&mut e, R1, "clear ip dhcp binding *");
    t.eq("cleared all", e.dhcp_bindings(R1).len(), 0);
    t.eq("no conflicts", cli(&mut e, R1, "show ip dhcp conflict").as_str(), "IP address        Detection method   Detection time          VRF\n");
    // helper addresses on MLS1's VLAN interfaces
    for l in ["enable", "configure terminal", "interface Vlan10", "no ip helper-address 10.10.0.1"] {
        let _ = cli(&mut e, MLS1, l);
    }
    t.eq("helper gone", e.dhcp_helpers(MLS1), alloc::vec![(String::from("Vlan20"), ip("10.10.0.1"))]);
    t.check("helper line gone", e.running_config(MLS1).contains("interface Vlan10\n description gateway of VLAN 10 Staff\n ip address 10.10.10.1 255.255.255.0\n!\n"));
    let _ = cli(&mut e, MLS1, "ip helper-address 10.10.0.1");
    t.eq("helper back", e.dhcp_helpers(MLS1).len(), 2);
    let _ = cli(&mut e, MLS1, "interface gi1/0/5");
    t.eq("helper on a switch port", cli(&mut e, MLS1, "ip helper-address 10.10.0.1").as_str(), "% IP addresses may not be configured on L2 links.\n");
}

fn ios_ntp(t: &mut T) {
    // R1 and MLS1 set their clocks from SRV1 (its first answers come with the second poll:
    // the first request of each waits for ARP)
    let mut e = campus(70);
    for d in [R1, MLS1] {
        let c = e.clock_state(d);
        t.check(&format!("{} synced", d), c.as_ref().is_some_and(|c| c.synced));
        t.eq(&format!("{} source", d), c.and_then(|c| c.source), Some(ip("10.10.100.10")));
    }
    for l in ["enable", "configure terminal", "ntp master 3"] {
        let _ = cli(&mut e, R1, l);
    }
    t.check("master line", e.running_config(R1).ends_with("!\nntp master 3\nntp server 10.10.100.10\n!\nend\n"));
    let _ = cli(&mut e, R1, "ntp master");
    t.check("master default", e.running_config(R1).ends_with("!\nntp master\nntp server 10.10.100.10\n!\nend\n"));
    let _ = cli(&mut e, R1, "no ntp master");
    let _ = cli(&mut e, R1, "no ntp server 10.10.100.10");
    t.check("ntp gone", e.running_config(R1).ends_with("line vty 0 4\n login\n!\nend\n"));
    t.eq("no ntp server again", cli(&mut e, R1, "no ntp server 10.10.100.10").as_str(), "");
}

fn first_ping(t: &mut T) {
    let mut e = campus(6);
    // the first name lookup is answered at once (R1 knows SRV1's MAC from the NTP at the start)
    let _ = e.take_reports();
    let t0 = e.now();
    let _ = e.cli_line(PC1, "ping -c 1 server.lab");
    e.run_until(t0 + SEC / 1000, BUDGET);
    t.check("no lookup delay", out(&mut e, PC1).contains("PING server.lab (10.10.100.10)"));
    // PC2 has never talked: ARP for its gateway first, then the echo; every cable carries the
    // echo with correct checksums (MLS1 and R1 lower the TTL and fix the header checksum)
    e.run_until(e.now() + 2 * SEC, BUDGET);
    let from = e.trace.hops().last().map(|h| h.id).unwrap_or(0);
    let _ = cli_run(&mut e, PC2, "ping -c 1 10.10.100.10", 2 * SEC);
    let hops: Vec<_> = e.trace.hops_after(from).filter(|h| h.mask & (P_ARP | P_ICMP) != 0).cloned().collect();
    let first: Vec<(u32, u32, bool)> = hops.iter().take(3).map(|h| (h.from.0, h.to.0, h.mask & P_ARP != 0)).collect();
    t.eq("arp first", first, alloc::vec![(PC2, MLS1, true), (MLS1, PC1, true), (MLS1, PC2, true)]);
    let echoes: Vec<(u32, u32)> = hops.iter().filter(|h| h.mask & P_ICMP != 0).map(|h| (h.from.0, h.to.0)).collect();
    t.eq("echo path", echoes, alloc::vec![(PC2, MLS1), (MLS1, R1), (R1, SRV1), (SRV1, R1), (R1, MLS1), (MLS1, PC2)]);
    let mut good = 0;
    for h in hops.iter().filter(|h| h.mask & P_ICMP != 0) {
        let ls = jfn_sim::view::hop_layers(h);
        let ok = |name: &str| ls.iter().any(|l| l.fields.iter().any(|f| f.name == name && f.value.ends_with("[correct]")));
        if ok("Header Checksum") && ok("Checksum") {
            good += 1;
        }
    }
    t.eq("checksums", good, 6);
}
