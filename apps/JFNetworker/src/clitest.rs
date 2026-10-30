// clitest -- checks of the command lines of jfn_sim (part of --selftest): IOS modes and
// prompts, abbreviations, "?" help and Tab, the error messages with their "^", the
// configuration commands (interfaces, addresses, switchports, VLANs, routes, ARP), show
// output, ping and traceroute with IOS and iputils output, Ctrl+C, lines typed while a command
// runs, debug and console logs, the Linux shell (ip, ping, traceroute, hostname, sysctl), the
// running configuration read back as the startup configuration, and the reports of the driver
//
// Devices of the small projects get ids in order: PC1 = 1, R1 = 2, PC2 = 3 (MAC 02:4a:00:00:id:port+1).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::project::{PortRef, Project};
use jfn_sim::{Cmd, Engine, Inline, Report, MS, SEC};

use crate::selftest::T;

const BUDGET: usize = 2_000_000;

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
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

/// a line typed at a device (a second of virtual time follows)
fn cli(e: &mut Engine, dev: u32, line: &str) -> String {
    cli_run(e, dev, line, SEC)
}

/// lines typed one after another; the output of the last
fn lines(e: &mut Engine, dev: u32, ls: &[&str]) -> String {
    let mut o = String::new();
    for l in ls {
        o = cli(e, dev, l);
    }
    o
}

/// PC1 (10.0.1.10) - Gi0/0/0 R1 Gi0/0/1 - PC2 (10.0.2.10), all set up by startup configurations
fn routed() -> (Project, u32, u32, u32) {
    let mut p = Project::new("routed", 21);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let r1 = p.add_device("jf-r4", 100, 0).unwrap_or(0);
    let pc2 = p.add_device("pc", 200, 0).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(r1, "gi0/0/0"), None);
    let _ = p.connect(&pr(r1, "gi0/0/1"), &pr(pc2, "eth0"), None);
    let cfg = [
        (pc1, "hostname PC1\nip addr add 10.0.1.10/24 dev eth0\nip route add default via 10.0.1.1\n"),
        (r1, "hostname R1\n!\ninterface GigabitEthernet0/0/0\n ip address 10.0.1.1 255.255.255.0\n no shutdown\n!\ninterface GigabitEthernet0/0/1\n ip address 10.0.2.1 255.255.255.0\n no shutdown\n!\nend\n"),
        (pc2, "hostname PC2\nip addr add 10.0.2.10/24 dev eth0\nip route add default via 10.0.2.1\n"),
    ];
    for (id, c) in cfg {
        if let Some(d) = p.device_mut(id) {
            d.startup_config = String::from(c);
        }
    }
    (p, pc1, r1, pc2)
}

/// SW1 (jf-s24p) with PC1 on Gi1/0/1 and MLS1 (jf-s24l3) on Te1/1/1
fn switched() -> (Project, u32, u32, u32) {
    let mut p = Project::new("switched", 22);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let sw = p.add_device("jf-s24p", 100, 0).unwrap_or(0);
    let mls = p.add_device("jf-s24l3", 200, 0).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(sw, "gi1/0/1"), None);
    let _ = p.connect(&pr(sw, "te1/1/1"), &pr(mls, "te1/1/1"), None);
    (p, pc1, sw, mls)
}

pub fn run(t: &mut T) {
    prompts_and_errors(t);
    help_and_tab(t);
    interface_config(t);
    routes_and_arp(t);
    switching(t);
    ios_ping(t);
    ios_traceroute_debug(t);
    saving(t);
    linux_shell(t);
    linux_ping(t);
    round_trip(t);
    driver_reports(t);
}

fn prompts_and_errors(t: &mut T) {
    let (p, pc1, r1, _pc2) = routed();
    let mut e = engine(&p);
    t.eq("boot errors", e.boot_errors(r1).len() + e.boot_errors(pc1).len(), 0);
    t.eq("user prompt", e.cli_prompt(r1), String::from("R1>"));
    t.eq("linux prompt", e.cli_prompt(pc1), String::from("root@PC1:~# "));
    t.eq("enable", cli(&mut e, r1, "enable"), String::new());
    t.eq("privileged prompt", e.cli_prompt(r1), String::from("R1#"));
    t.eq("conf t", cli(&mut e, r1, "conf t"), String::from("Configuration mode: one command per line; end or Ctrl+Z leaves it.\n"));
    t.eq("config prompt", e.cli_prompt(r1), String::from("R1(config)#"));
    let o = cli(&mut e, r1, "interfac gi0/0/0");
    t.check("abbreviated interface", o.is_empty() && e.cli_prompt(r1) == "R1(config-if)#");
    // a global command in interface mode works and ends the interface mode (IOS)
    t.eq("fallback to global", cli(&mut e, r1, "ip route 192.0.2.0 255.255.255.0 10.0.2.10"), String::new());
    t.eq("prompt after fallback", e.cli_prompt(r1), String::from("R1(config)#"));
    t.eq("marker", cli(&mut e, r1, "hostnme R2"), String::from("                ^\n% Invalid input detected at '^' marker.\n\n"));
    t.eq("marker after a whole word", cli(&mut e, r1, "ip routee"), String::from("                   ^\n% Invalid input detected at '^' marker.\n\n"));
    t.eq("incomplete", cli(&mut e, r1, "ip route 10.0.0.0"), String::from("% Incomplete command.\n\n"));
    t.eq("ambiguous", cli(&mut e, r1, "i"), String::from("% Ambiguous command:  \"i\"\n"));
    t.check("end logs", cli(&mut e, r1, "end").ends_with("%SYS-5-CONFIG_I: configuration changed at the console\n") && e.cli_prompt(r1) == "R1#");
    t.eq("unknown exec word", cli(&mut e, r1, "shw ip route"), String::from("Resolving \"shw\" via DNS (255.255.255.255) ...\n% Unknown command or host name\n"));
    t.eq("show needs more", cli(&mut e, r1, "show"), String::from("% Incomplete command.\n\n"));
    t.eq("several keywords", cli(&mut e, r1, "c"), String::from("% Ambiguous command:  \"c\"\n"));
    t.eq("config only in privileged", lines(&mut e, r1, &["disable", "conf t"]), String::from("Resolving \"conf\" via DNS (255.255.255.255) ...\n% Unknown command or host name\n"));
    t.check("no domain lookup", {
        let _ = lines(&mut e, r1, &["enable", "conf t", "no ip domain-lookup", "end"]);
        cli(&mut e, r1, "frob") == "% Unknown command or host name\n"
    });
    // logging out and back in
    let bye = cli(&mut e, r1, "exit");
    t.check("logout", bye.contains("R1: session closed.") && bye.contains("Console ready, press Enter to log in.") && e.cli_prompt(r1).is_empty());
    t.eq("press return", cli(&mut e, r1, ""), String::new());
    t.eq("back in", e.cli_prompt(r1), String::from("R1>"));
    t.check("no console on a hub", {
        let mut q = Project::new("hub", 1);
        let h = q.add_device("hub", 0, 0).unwrap_or(0);
        engine(&q).cli_line(h, "show").is_err()
    });
}

fn help_and_tab(t: &mut T) {
    let (p, _pc1, r1, _pc2) = routed();
    let mut e = engine(&p);
    let h = e.cli_help(r1, "");
    t.check("exec help", h.starts_with("Commands at the user level:\n") && h.contains("\n  enable      Switch to the privileged level\n") && h.ends_with("\n\n"));
    let _ = cli(&mut e, r1, "enable");
    t.eq("show ip ?", e.cli_help(r1, "show ip "), String::from("  access-lists  The access lists with their hit counters\n  arp           Learned IPv4-to-MAC mappings\n  bgp           The BGP table: every path to every network\n  dhcp          State of the DHCP server\n  interface     IPv4 settings of the interfaces\n  nat           Address translation\n  ospf          State of the OSPF process\n  protocols     The routing processes and their settings\n  rip           State of RIP\n  route         Routes this device knows\n\n"));
    t.eq("partial word", e.cli_help(r1, "sh"), String::from("show  \n\n"));
    t.eq("help with <cr>", e.cli_help(r1, "show ip route "), String::from("  bgp        Only the routes BGP chose\n  connected  Only the networks of the interfaces\n  ospf       Only the routes OSPF found\n  rip        Only the routes RIP learned\n  static     Only the static routes\n  <cr>\n\n"));
    t.eq("help marker", e.cli_help(r1, "show ipx "), String::from("          ^\n% Invalid input detected at '^' marker.\n\n"));
    let _ = cli(&mut e, r1, "conf t");
    let ih = e.cli_help(r1, "interface ");
    t.check("interface types", ih.contains("  GigabitEthernet     Gigabit Ethernet port (1 Gbit/s)\n") && ih.contains("  Loopback            Virtual interface that is always up\n") && ih.contains("  Serial              Serial WAN port\n"));
    // X1d added several "no" forms (aaa, username, radius-server ...); the column is wider now
    t.check("no ?", { let h = e.cli_help(r1, "no "); h.contains("hostname") && h.contains("Give the device a name") });
    t.check("do ?", e.cli_help(r1, "do sh").contains("show"));
    t.eq("? as a typed line", cli(&mut e, r1, "ip route ?"), String::from("  A.B.C.D  Destination network\n\n"));
    t.eq("tab", e.cli_complete(r1, "int").0, Some(String::from("interface ")));
    t.eq("tab type", e.cli_complete(r1, "interface gi").0, Some(String::from("interface GigabitEthernet")));
    // X1d added "enable secret / password" in the global configuration
    t.check("tab choices", e.cli_complete(r1, "e").0.is_none() && e.cli_complete(r1, "e").1 == ["enable", "end", "exit"]);
    t.eq("linux tab", e.cli_complete(1, "tracer").0, Some(String::from("traceroute ")));
}

fn interface_config(t: &mut T) {
    let (p, _pc1, r1, _pc2) = routed();
    let mut e = engine(&p);
    let _ = lines(&mut e, r1, &["enable", "conf t", "interface gi0/0/2", "description to the lab"]);
    t.eq("no address on a shut port", cli(&mut e, r1, "ip address 10.0.3.1 255.255.255.0"), String::new());
    t.eq("bad mask", cli(&mut e, r1, "ip address 10.0.3.0 255.255.255.0"), String::from("Bad mask /24 for address 10.0.3.0\n"));
    t.eq("not a host", cli(&mut e, r1, "ip address 127.0.0.1 255.0.0.0"), String::from("Not a valid host address - 127.0.0.1\n"));
    t.eq("secondary", cli(&mut e, r1, "ip address 10.0.4.1 255.255.255.0 secondary"), String::new());
    t.eq("primary with secondaries", cli(&mut e, r1, "no ip address 10.0.3.1 255.255.255.0"), String::from("Must delete secondary before deleting primary\n"));
    t.eq("replace the primary", cli(&mut e, r1, "ip address 10.0.5.1 255.255.255.0"), String::new());
    let addrs: Vec<String> = e.ifaces(r1).into_iter().find(|f| f.name == "GigabitEthernet0/0/2").map(|f| f.addrs.iter().map(|a| format!("{}", a)).collect()).unwrap_or_default();
    t.eq("addresses in order", addrs.join(" "), String::from("10.0.5.1/24 10.0.4.1/24"));
    t.eq("overlap", lines(&mut e, r1, &["interface gi0/0/3", "ip address 10.0.5.9 255.255.255.0"]), String::from("% 10.0.5.0 overlaps with GigabitEthernet0/0/2\n"));
    let up = cli(&mut e, r1, "no shutdown");
    t.eq("no shutdown without cable", up, String::new());
    t.check("shut logs", cli(&mut e, r1, "shutdown").contains("%LINK-5-CHANGED: GigabitEthernet0/0/3 was shut down\n"));
    t.check("port down logs on both ends", {
        let a = cli(&mut e, r1, "interface gi0/0/1");
        let b = cli(&mut e, r1, "shutdown");
        a.is_empty() && b.contains("%LINK-5-CHANGED: GigabitEthernet0/0/1 was shut down") && b.contains("%LINEPROTO-5-UPDOWN: line protocol on GigabitEthernet0/0/1 is down")
    });
    t.check("and up again", cli(&mut e, r1, "no shut").contains("%LINK-3-UPDOWN: GigabitEthernet0/0/1 is up\n"));
    let lo = cli(&mut e, r1, "interface loopback 7");
    t.check("loopback made", lo.contains("%LINEPROTO-5-UPDOWN: line protocol on Loopback7 is up") && e.cli_prompt(r1) == "R1(config-if)#");
    t.eq("loopback /32", cli(&mut e, r1, "ip address 192.0.2.7 255.255.255.255"), String::new());
    t.eq("/32 only on loopbacks", lines(&mut e, r1, &["int gi0/0/3", "ip address 10.0.9.1 255.255.255.255"]), String::from("Bad mask /32 for address 10.0.9.1\n"));
    t.eq("mtu", lines(&mut e, r1, &["ip mtu 1400", "no ip proxy-arp"]), String::new());
    let f3 = e.ifaces(r1).into_iter().find(|f| f.name == "GigabitEthernet0/0/3");
    t.check("mtu and proxy arp", f3.is_some_and(|f| f.mtu == 1400 && !f.proxy_arp));
    t.eq("no physical removal", cli(&mut e, r1, "no interface gi0/0/3"), String::from("% Physical interfaces cannot be removed\n"));
    t.eq("no loopback", cli(&mut e, r1, "no interface Loopback7"), String::new());
    t.check("loopback gone", !e.ifaces(r1).iter().any(|f| f.name == "Loopback7"));
    t.eq("no such loopback", cli(&mut e, r1, "no interface Loopback7"), String::from("% Interface Loopback7 does not exist\n"));
    // subinterfaces: no address before encapsulation, one VLAN each
    let _ = cli(&mut e, r1, "interface gi0/0/2.10");
    t.eq("subif prompt", e.cli_prompt(r1), String::from("R1(config-subif)#"));
    t.check("needs encapsulation", cli(&mut e, r1, "ip address 10.0.10.1 255.255.255.0").starts_with("% Give the subinterface its VLAN first"));
    t.eq("encapsulation", cli(&mut e, r1, "encapsulation dot1Q 10"), String::new());
    t.eq("subif address", cli(&mut e, r1, "ip address 10.0.10.1 255.255.255.0"), String::new());
    let dup = lines(&mut e, r1, &["interface gi0/0/2.11", "encapsulation dot1Q 10"]);
    t.check("one vlan per subif", dup.contains("% VLAN 10 is already in use on GigabitEthernet0/0/2.10."));
    let run = e.running_config(r1);
    t.check("running config interfaces", run.contains("interface GigabitEthernet0/0/2\n description to the lab\n ip address 10.0.5.1 255.255.255.0\n ip address 10.0.4.1 255.255.255.0 secondary\n shutdown\n!\ninterface GigabitEthernet0/0/2.10\n encapsulation dot1Q 10\n ip address 10.0.10.1 255.255.255.0\n!\ninterface GigabitEthernet0/0/2.11\n!\ninterface GigabitEthernet0/0/3\n no ip address\n ip mtu 1400\n no ip proxy-arp\n shutdown\n!"));
    t.check("hostname", lines(&mut e, r1, &["end", "conf t", "hostname Core-1"]).is_empty() && e.cli_prompt(r1) == "Core-1(config)#" && e.node(r1).is_some_and(|n| n.name == "Core-1"));
    t.eq("bad hostname", cli(&mut e, r1, "hostname 9lives"), String::from("% Hostname contains one or more illegal characters.\n"));
    t.check("do", cli(&mut e, r1, "do show ip interface brief").starts_with("Interface              IP-Address      OK? Method Status                Protocol\n"));
    t.eq("do with a typo", cli(&mut e, r1, "do show ipx"), String::from("                         ^\n% Invalid input detected at '^' marker.\n\n"));
}

fn routes_and_arp(t: &mut T) {
    let (p, _pc1, r1, _pc2) = routed();
    let mut e = engine(&p);
    let _ = lines(&mut e, r1, &["en", "conf t"]);
    t.eq("inconsistent mask", cli(&mut e, r1, "ip route 10.9.0.1 255.255.0.0 10.0.2.10"), String::from("%Inconsistent address and mask\n"));
    t.eq("own next hop", cli(&mut e, r1, "ip route 10.9.0.0 255.255.0.0 10.0.2.1"), String::from("%Invalid next hop address (it's this router)\n"));
    let ok = lines(&mut e, r1, &["ip route 10.9.0.0 255.255.0.0 10.0.2.10", "ip route 0.0.0.0 0.0.0.0 10.0.1.10", "ip route 172.16.0.0 255.255.0.0 gi0/0/1 10.0.2.10 200", "ip route 10.0.0.0 255.0.0.0 10.0.2.10 250"]);
    t.eq("routes", ok, String::new());
    let rt = cli(&mut e, r1, "do show ip route");
    t.check("gateway of last resort", rt.contains("\nGateway of last resort is 10.0.1.10 to network 0.0.0.0\n\n"));
    t.check("default route line", rt.contains("\nS*    0.0.0.0/0 [1/0] via 10.0.1.10\n"));
    t.check("variably subnetted", rt.contains("\n      10.0.0.0/8 is variably subnetted, 6 subnets, 4 masks\nS        10.0.0.0/8 [250/0] via 10.0.2.10\nC        10.0.1.0/24 is directly connected, GigabitEthernet0/0/0\n"));
    t.check("static in group", rt.contains("\nS        10.9.0.0/16 [1/0] via 10.0.2.10\n"));
    t.check("a classful network alone", rt.contains("\nS     172.16.0.0/16 [200/0] via 10.0.2.10, GigabitEthernet0/0/1\n"));
    t.check("configured routes", {
        let run = e.running_config(r1);
        run.contains("ip route 0.0.0.0 0.0.0.0 10.0.1.10\nip route 10.0.0.0 255.0.0.0 10.0.2.10 250\nip route 10.9.0.0 255.255.0.0 10.0.2.10\nip route 172.16.0.0 255.255.0.0 GigabitEthernet0/0/1 10.0.2.10 200\n!")
    });
    t.eq("no ip route", cli(&mut e, r1, "no ip route 10.9.0.0 255.255.0.0"), String::new());
    t.check("route gone", !e.routes(r1).iter().any(|r| format!("{}", r.prefix) == "10.9.0.0/16"));
    t.eq("no such route", cli(&mut e, r1, "no ip route 10.9.0.0 255.255.0.0"), String::from("%No matching route to delete\n"));
    t.eq("static arp", cli(&mut e, r1, "arp 10.0.1.77 024a.0000.7701 arpa"), String::new());
    t.eq("static arp off net", cli(&mut e, r1, "arp 192.0.2.77 024a.0000.7701 arpa"), String::from("% Invalid ARP entry - 192.0.2.77 is not on a connected network\n"));
    let arp = cli(&mut e, r1, "do show arp");
    t.check("show arp", arp.starts_with("Protocol  Address          Age (min)  Hardware Addr   Type   Interface\nInternet  10.0.1.1                -   024a.0000.0201  ARPA   GigabitEthernet0/0/0\nInternet  10.0.1.77               -   024a.0000.7701  ARPA   GigabitEthernet0/0/0\n"));
    t.check("arp in the config", e.running_config(r1).contains("\narp 10.0.1.77 024a.0000.7701 ARPA\n!"));
    t.eq("clear arp", cli(&mut e, r1, "do clear arp-cache"), String::new());
    t.eq("no arp", cli(&mut e, r1, "no arp 10.0.1.77"), String::new());
    t.check("ip routing", {
        let _ = cli(&mut e, r1, "no ip routing");
        let off = e.running_config(r1).contains("\nno ip routing\n");
        let _ = cli(&mut e, r1, "ip routing");
        off && !e.running_config(r1).contains("ip routing")
    });
}

fn switching(t: &mut T) {
    let (p, _pc1, sw, mls) = switched();
    let mut e = engine(&p);
    let _ = lines(&mut e, sw, &["enable", "configure terminal", "vlan 10", "name Sales"]);
    t.eq("vlan prompt", e.cli_prompt(sw), String::from("SW1(config-vlan)#"));
    t.check("vlan named", e.bridge(sw).is_some_and(|b| b.vlans.get(&10).is_some_and(|n| n == "Sales")));
    t.eq("vlan exits by fallback", cli(&mut e, sw, "interface gi1/0/5"), String::new());
    t.eq("access vlan made", cli(&mut e, sw, "switchport access vlan 30"), String::from("% Access VLAN does not exist. Creating vlan 30\n"));
    t.eq("existing access vlan", cli(&mut e, sw, "switchport access vlan 10"), String::new());
    let _ = lines(&mut e, sw, &["switchport mode access", "interface te1/1/1", "switchport mode trunk", "switchport trunk native vlan 99", "switchport trunk allowed vlan 10,20", "switchport trunk allowed vlan add 30-31", "switchport trunk allowed vlan remove 31"]);
    let b = e.bridge(sw);
    t.check("trunk", b.is_some_and(|b| matches!(&b.ports[24].mode, jfn_sim::bridge::PortMode::Trunk { native: 99, allowed } if allowed.to_text() == "10,20,30")));
    t.check("access", b.is_some_and(|b| b.ports[4].mode == jfn_sim::bridge::PortMode::Access(10)));
    let run = e.running_config(sw);
    t.check("vlans in the config", run.contains("!\nvlan 10\n name Sales\n!\nvlan 30\n!\n"));
    t.check("switchports in the config", run.contains("interface GigabitEthernet1/0/5\n switchport access vlan 10\n switchport mode access\n!") && run.contains("interface TenGigabitEthernet1/1/1\n switchport trunk native vlan 99\n switchport trunk allowed vlan 10,20,30\n switchport mode trunk\n!"));
    t.check("vlan 1 stays", cli(&mut e, sw, "no vlan 1") == "%Default VLAN 1 may not be deleted.\n");
    t.check("no ip address on L2 ports", cli(&mut e, sw, "int gi1/0/2").is_empty() && cli(&mut e, sw, "ip address 10.0.0.9 255.255.255.0") == "% IP addresses may not be configured on L2 links.\n");
    let vb = cli(&mut e, sw, "do show vlan brief");
    t.check("show vlan brief", vb.starts_with("\nVLAN Name                             Status    Ports\n---- -------------------------------- --------- -------------------------------\n1    default                          active    Gi1/0/1, Gi1/0/2, Gi1/0/3\n                                                Gi1/0/4, Gi1/0/6, Gi1/0/7\n") && vb.contains("\n10   Sales                            active    Gi1/0/5\n") && vb.contains("\n30   VLAN0030                         active    \n1002 fddi-default                     act/unsup \n"));
    let tr = cli(&mut e, sw, "do show interfaces trunk");
    t.check("show interfaces trunk", tr.starts_with("\nPort        Mode             Encapsulation  Status        Native vlan\nTe1/1/1     on               802.1q         trunking      99\n\nPort        Vlans allowed on trunk\nTe1/1/1     10,20,30\n"));
    // an L3 switch: routed ports, SVIs, ip routing
    let _ = lines(&mut e, mls, &["enable", "conf t", "interface te1/1/2", "no switchport", "ip address 192.168.99.1 255.255.255.252", "interface vlan 10"]);
    t.check("routed port", e.ifaces(mls).iter().any(|f| f.name == "TenGigabitEthernet1/1/2" && f.addrs.len() == 1));
    t.eq("svi prompt", e.cli_prompt(mls), String::from("MLS1(config-if)#"));
    let _ = lines(&mut e, mls, &["ip address 10.0.10.1 255.255.255.0", "no shutdown", "exit", "ip routing"]);
    let run = e.running_config(mls);
    t.check("l3 config", run.contains("hostname MLS1\n!\nip routing\n!\n") && run.contains("interface TenGigabitEthernet1/1/2\n no switchport\n ip address 192.168.99.1 255.255.255.252\n!") && run.contains("interface Vlan10\n ip address 10.0.10.1 255.255.255.0\n!\n") && run.contains("interface Vlan1\n no ip address\n shutdown\n!"));
    t.check("switchport again", cli(&mut e, mls, "interface te1/1/2").is_empty() && cli(&mut e, mls, "switchport").is_empty() && !e.ifaces(mls).iter().any(|f| f.name == "TenGigabitEthernet1/1/2"));
    t.eq("no switchport on an L2 switch", cli(&mut e, sw, "no switchport"), String::from("% Incomplete command.\n\n"));
    // an access point: radios (digits in the type name) and its bridge interface BVI1
    let mut q = Project::new("ap", 23);
    let ap = q.add_device("jf-ap6e", 0, 0).unwrap_or(0);
    let mut ea = engine(&q);
    let _ = lines(&mut ea, ap, &["enable", "conf t"]);
    t.check("radio types", ea.cli_help(ap, "interface ").contains("  Dot11Radio          WLAN radio\n"));
    t.check("radio by abbreviation", cli(&mut ea, ap, "interface do1").is_empty() && ea.cli_prompt(ap) == "AP1(config-if)#");
    let _ = lines(&mut ea, ap, &["interface BVI1", "ip address 10.0.0.5 255.255.255.0"]);
    t.check("bvi address", ea.running_config(ap).contains("interface BVI1\n ip address 10.0.0.5 255.255.255.0\n!"));
    t.eq("no routing command on L2", cli(&mut e, sw, "ip routing"), String::from("                  ^\n% Invalid input detected at '^' marker.\n\n"));
}

fn ios_ping(t: &mut T) {
    let (p, _pc1, r1, _pc2) = routed();
    let mut e = engine(&p);
    let _ = cli(&mut e, r1, "enable");
    let o = cli_run(&mut e, r1, "ping 10.0.2.10", 20 * SEC);
    t.eq("ping", o, String::from("Ctrl+Shift+6 stops the ping.\nPinging 10.0.2.10 with 5 echo requests of 100 bytes, 2 s timeout each:\n.!!!!\nSuccess rate is 80 percent (4/5), round-trip min/avg/max = 0/0/0 ms\n"));
    let o = cli_run(&mut e, r1, "ping ip 10.0.1.10 repeat 2 size 1500 source gi0/0/0 df-bit", 20 * SEC);
    t.eq("ping options", o, String::from("Ctrl+Shift+6 stops the ping.\nPinging 10.0.1.10 with 2 echo requests of 1500 bytes, 2 s timeout each:\nSource address: 10.0.1.1\nFragmentation forbidden (DF bit set)\n.!\nSuccess rate is 50 percent (1/2), round-trip min/avg/max = 0/0/0 ms\n"));
    t.eq("ping too big for df", cli_run(&mut e, r1, "ping 10.0.1.10 repeat 1 size 1501 df-bit", 5 * SEC).lines().nth(3).map(String::from), Some(String::from("M")));
    // no route: every probe waits its timeout
    let t0 = e.now();
    let _ = e.take_reports();
    let _ = e.cli_line(r1, "ping 192.0.2.1");
    t.check("busy", e.cli_busy(r1));
    e.run_until(t0 + 9 * SEC, BUDGET);
    t.check("still busy after 9 s", e.cli_busy(r1));
    e.run_until(t0 + 11 * SEC, BUDGET);
    t.eq("no route", out(&mut e, r1), String::from("Ctrl+Shift+6 stops the ping.\nPinging 192.0.2.1 with 5 echo requests of 100 bytes, 2 s timeout each:\n.....\nSuccess rate is 0 percent (0/5)\n"));
    t.check("done after 10 s", !e.cli_busy(r1));
    t.eq("unknown host", cli(&mut e, r1, "ping nowhere"), String::from("Resolving \"nowhere\" via DNS (255.255.255.255) ...\n% Unrecognized host or address, or protocol not running.\n\n"));
    // Ctrl+Shift+6 stops it; what was typed meanwhile is dropped
    let _ = e.take_reports();
    let _ = e.cli_line(r1, "ping 10.0.2.99 repeat 100");
    let t0 = e.now();
    e.run_until(t0 + 5 * SEC, BUDGET);
    let _ = e.cli_line(r1, "show ip route");
    t.check("typed while busy", e.cli_busy(r1));
    let _ = e.cli_interrupt(r1);
    e.run_until(e.now() + SEC, BUDGET);
    t.eq("interrupted", out(&mut e, r1), String::from("Ctrl+Shift+6 stops the ping.\nPinging 10.0.2.99 with 100 echo requests of 100 bytes, 2 s timeout each:\n..\nSuccess rate is 0 percent (0/2)\n"));
    t.check("idle again", !e.cli_busy(r1));
    // a line typed while a command runs comes after it, with its echo
    let _ = e.cli_line(r1, "ping 10.0.2.99 repeat 2");
    let _ = e.cli_line(r1, "show arp");
    e.run_until(e.now() + 10 * SEC, BUDGET);
    let o = out(&mut e, r1);
    t.check("type-ahead", o.contains("..\nSuccess rate is 0 percent (0/2)\nR1#show arp\nProtocol  Address"));
}

fn ios_traceroute_debug(t: &mut T) {
    let (p, pc1, r1, _pc2) = routed();
    let mut e = engine(&p);
    let _ = cli(&mut e, r1, "enable");
    let o = cli_run(&mut e, r1, "traceroute 10.0.2.10", 30 * SEC);
    t.eq("traceroute", o, String::from("Ctrl+Shift+6 stops the traceroute.\nRoute to 10.0.2.10, up to 30 hops:\n  1  * \n    10.0.2.10 0 msec 0 msec\n"));
    let o = cli_run(&mut e, r1, "traceroute 10.0.2.10 numeric probe 2", 30 * SEC);
    t.check("traceroute options", o.ends_with("  1 10.0.2.10 0 msec 0 msec\n"));
    t.eq("debug arp", cli(&mut e, r1, "debug arp"), String::from("Debug output for ARP: on\n"));
    t.eq("debug icmp", cli(&mut e, r1, "debug ip icmp"), String::from("Debug output for ICMP: on\n"));
    t.check("show debugging", cli(&mut e, r1, "show debugging").contains("Debug output for ICMP: on"));
    let _ = cli(&mut e, r1, "clear arp-cache");
    let _ = cli(&mut e, pc1, "ip neigh flush all");
    let _ = cli_run(&mut e, pc1, "ping -c 1 10.0.1.1", 15 * SEC);
    let (p2, pc1b, r1b, _) = routed();
    let mut e2 = engine(&p2);
    let _ = lines(&mut e2, r1b, &["enable", "debug arp", "debug ip icmp"]);
    let _ = e2.take_reports();
    let _ = e2.cli_line(pc1b, "ping -c 1 10.0.1.1");
    e2.run_until(e2.now() + 15 * SEC, BUDGET);
    let d = out(&mut e2, r1b);
    t.check("rcvd req", d.contains(": ARP: got request from 10.0.1.10 (024a.0000.0101) for 10.0.1.1 on GigabitEthernet0/0/0\n"));
    t.check("creating entry", d.contains(": ARP: learned 10.0.1.10 is at 024a.0000.0101\n"));
    t.check("sent rep", d.contains(": ARP: sent reply 10.0.1.1 is at 024a.0000.0201 to 10.0.1.10 (024a.0000.0101) on GigabitEthernet0/0/0\n"));
    t.check("echo reply sent", d.contains(": ICMP: sent echo reply 10.0.1.1 -> 10.0.1.10\n"));
    t.check("stamps", d.starts_with("*Mar  1 00:00:03.000: ARP: got request"));
    let _ = e2.cli_line(pc1b, "ping -c 1 -t 1 10.0.2.10");
    e2.run_until(e2.now() + 15 * SEC, BUDGET);
    let d = out(&mut e2, r1b);
    t.check("time exceeded sent", d.contains("ICMP: sent time exceeded to 10.0.1.10 (packet for 10.0.2.10)\n"));
    t.eq("undebug", cli(&mut e2, r1b, "undebug all"), String::from("Debug output: all off\n"));
    let _ = e2.cli_line(pc1b, "ping -c 1 10.0.1.1");
    e2.run_until(e2.now() + 15 * SEC, BUDGET);
    t.eq("quiet again", out(&mut e2, r1b), String::new());
}

fn saving(t: &mut T) {
    let (p, _pc1, r1, _pc2) = routed();
    let mut e = engine(&p);
    let _ = lines(&mut e, r1, &["enable", "conf t", "interface gi0/0/2", "ip address 10.0.3.1 255.255.255.0", "end"]);
    let _ = e.take_reports();
    let _ = e.cli_line(r1, "copy running-config startup-config");
    t.eq("asks", e.cli_prompt(r1), String::from("Save to which file [startup-config]? "));
    let _ = e.cli_line(r1, "");
    let mut saved = None;
    let mut text = String::new();
    for r in e.take_reports() {
        match r {
            Report::Startup { dev, text: s } if dev == r1 => saved = Some(s),
            Report::CliOut { dev, bytes, .. } if dev == r1 => text.push_str(&String::from_utf8_lossy(&bytes)),
            _ => {}
        }
    }
    t.eq("copy run start", text, String::from("Saving the configuration ...\n[OK]\n"));
    t.check("saved text", saved.as_ref().is_some_and(|s| *s == e.running_config(r1) && s.starts_with("!\nversion 1.0\n!\nhostname R1\n!\n") && s.ends_with("!\nend\n")));
    t.eq("prompt back", e.cli_prompt(r1), String::from("R1#"));
    let st = cli(&mut e, r1, "show startup-config");
    t.check("show startup-config", st.starts_with("! startup configuration, ") && st.contains(" bytes\n!\nversion 1.0\n") && st.contains(" ip address 10.0.3.1 255.255.255.0"));
    t.eq("write memory", cli(&mut e, r1, "write"), String::from("Saving the configuration ...\n[OK]\n"));
    let sr = cli(&mut e, r1, "show running-config");
    t.check("show running-config header", sr.starts_with(&format!("Collecting the configuration ...\n\n! configuration in effect, {} bytes\n!\n", e.running_config(r1).len())));
    t.eq("other file", lines(&mut e, r1, &["copy running-config startup-config", "flash:x"]), String::from("%Error opening flash:x (Invalid argument)\n"));
    let mut q = Project::new("empty", 1);
    let r = q.add_device("jf-r4", 0, 0).unwrap_or(0);
    let mut eq = engine(&q);
    t.eq("no startup", lines(&mut eq, r, &["en", "show startup-config"]), String::from("No startup configuration saved yet\n"));
}

fn linux_shell(t: &mut T) {
    let (p, pc1, _r1, _pc2) = routed();
    let mut e = engine(&p);
    let a = cli(&mut e, pc1, "ip a s eth0");
    t.eq("ip addr show", a, String::from("2: eth0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc fq_codel state UP group default qlen 1000\n    link/ether 02:4a:00:00:01:01 brd ff:ff:ff:ff:ff:ff\n    inet 10.0.1.10/24 brd 10.0.1.255 scope global eth0\n       valid_lft forever preferred_lft forever\n"));
    t.eq("ip -br a", cli(&mut e, pc1, "ip -br a"), String::from("lo               UNKNOWN        127.0.0.1/8\neth0             UP             10.0.1.10/24\n"));
    t.eq("add again", cli(&mut e, pc1, "ip addr add 10.0.1.10/24 dev eth0"), String::from("RTNETLINK answers: File exists\n"));
    t.eq("no such device", cli(&mut e, pc1, "ip addr add 10.0.7.1/24 dev eth9"), String::from("Cannot find device \"eth9\"\n"));
    t.eq("bad prefix", cli(&mut e, pc1, "ip addr add 10.0.7.300/24 dev eth0"), String::from("Error: any valid prefix is expected rather than \"10.0.7.300/24\".\n"));
    t.eq("needs dev", cli(&mut e, pc1, "ip addr add 10.0.7.1/24"), String::from("Not enough information: \"dev\" argument is required.\n"));
    t.eq("secondary address", cli(&mut e, pc1, "ip addr add 10.0.1.11/24 dev eth0"), String::new());
    t.check("secondary shown", cli(&mut e, pc1, "ip addr show dev eth0").contains("    inet 10.0.1.11/24 brd 10.0.1.255 scope global secondary eth0\n"));
    t.eq("del", cli(&mut e, pc1, "ip addr del 10.0.1.11/24 dev eth0"), String::new());
    t.eq("del again", cli(&mut e, pc1, "ip addr del 10.0.1.11/24 dev eth0"), String::from("RTNETLINK answers: Cannot assign requested address\n"));
    t.eq("link down", cli(&mut e, pc1, "ip link set eth0 down"), String::new());
    t.eq("ip link", cli(&mut e, pc1, "ip link show eth0"), String::from("2: eth0: <BROADCAST,MULTICAST> mtu 1500 qdisc fq_codel state DOWN mode DEFAULT group default qlen 1000\n    link/ether 02:4a:00:00:01:01 brd ff:ff:ff:ff:ff:ff\n"));
    let _ = cli(&mut e, pc1, "ip link set dev eth0 up");
    t.check("link up", cli(&mut e, pc1, "ip link").contains("2: eth0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc fq_codel state UP"));
    t.eq("ip route", cli(&mut e, pc1, "ip r"), String::from("default via 10.0.1.1 dev eth0\n10.0.1.0/24 dev eth0 proto kernel scope link src 10.0.1.10\n"));
    t.eq("route add", cli(&mut e, pc1, "ip route add 10.9.0.0/16 via 10.0.1.1"), String::new());
    t.eq("route add twice", cli(&mut e, pc1, "ip route add 10.9.0.0/16 via 10.0.1.1"), String::from("RTNETLINK answers: File exists\n"));
    t.eq("bad gateway", cli(&mut e, pc1, "ip route add 10.8.0.0/16 via 192.0.2.1"), String::from("Error: Nexthop has invalid gateway.\n"));
    t.eq("route dev", cli(&mut e, pc1, "ip route add 10.7.0.0/16 dev eth0"), String::new());
    t.check("routes shown", cli(&mut e, pc1, "ip route show").contains("10.7.0.0/16 dev eth0 scope link\n10.9.0.0/16 via 10.0.1.1 dev eth0\n"));
    t.eq("route get", cli(&mut e, pc1, "ip route get 10.0.2.10"), String::from("10.0.2.10 via 10.0.1.1 dev eth0 src 10.0.1.10 uid 0 \n    cache \n"));
    t.eq("route del", cli(&mut e, pc1, "ip route del 10.9.0.0/16"), String::new());
    t.eq("route del again", cli(&mut e, pc1, "ip route del 10.9.0.0/16"), String::from("RTNETLINK answers: No such process\n"));
    t.eq("neigh add", cli(&mut e, pc1, "ip neigh add 10.0.1.99 lladdr 02:00:00:00:00:99 dev eth0"), String::new());
    t.check("neigh shown", cli(&mut e, pc1, "ip neigh").contains("10.0.1.99 dev eth0 lladdr 02:00:00:00:00:99 PERMANENT\n"));
    t.eq("unknown object", cli(&mut e, pc1, "ip foo"), String::from("Object \"foo\" is unknown, try \"ip help\".\n"));
    t.check("usage texts", cli(&mut e, pc1, "ip help").starts_with("Usage: ip [-4] [-br] OBJECT") && cli(&mut e, pc1, "ip route help").starts_with("Usage: ip route [show]\n") && cli(&mut e, pc1, "ip -help").starts_with("Usage: ip "));
    t.eq("unknown command", cli(&mut e, pc1, "ip addr frob"), String::from("Command \"frob\" is unknown, try \"ip address help\".\n"));
    t.eq("hostname", cli(&mut e, pc1, "hostname"), String::from("PC1\n"));
    t.check("set hostname", cli(&mut e, pc1, "hostname client-7").is_empty() && e.cli_prompt(pc1) == "root@client-7:~# ");
    t.eq("bad hostname", cli(&mut e, pc1, "hostname -x-"), String::from("hostname: the specified hostname is invalid\n"));
    t.eq("sysctl", cli(&mut e, pc1, "sysctl net.ipv4.ip_forward"), String::from("net.ipv4.ip_forward = 0\n"));
    t.check("sysctl -w", cli(&mut e, pc1, "sysctl -w net.ipv4.ip_forward=1") == "net.ipv4.ip_forward = 1\n" && e.node(pc1).is_some_and(|n| n.ip.as_ref().is_some_and(|s| s.forwarding)));
    t.eq("not found", cli(&mut e, pc1, "frobnicate --all"), String::from("frobnicate: command not found\n"));
    t.eq("sudo and quotes", cli(&mut e, pc1, "sudo echo 'a  b' \"c\""), String::from("a  b c\n"));
    t.eq("subinterface", cli(&mut e, pc1, "ip link add link eth0 name eth0.10 type vlan id 10"), String::new());
    t.check("subinterface shown", cli(&mut e, pc1, "ip link show eth0.10").starts_with("3: eth0.10@eth0: <BROADCAST,MULTICAST> mtu 1500 qdisc noqueue state DOWN"));
    t.eq("dummy", cli(&mut e, pc1, "ip link add dummy0 type dummy"), String::new());
    t.check("dummy shown", cli(&mut e, pc1, "ip link").contains(": dummy0: <BROADCAST,NOARP,UP,LOWER_UP> mtu 1500 qdisc noqueue state UNKNOWN"));
}

fn linux_ping(t: &mut T) {
    let (p, pc1, _r1, pc2) = routed();
    let mut e = engine(&p);
    let o = cli_run(&mut e, pc1, "ping -c 3 10.0.2.10", 30 * SEC);
    t.eq("ping", o, String::from("PING 10.0.2.10 (10.0.2.10) 56(84) bytes of data.\n64 bytes from 10.0.2.10: icmp_seq=2 ttl=63 time=0.004 ms\n64 bytes from 10.0.2.10: icmp_seq=3 ttl=63 time=0.004 ms\n\n--- 10.0.2.10 ping statistics ---\n3 packets transmitted, 2 received, 33.3333% packet loss, time 2000ms\nrtt min/avg/max/mdev = 0.004/0.004/0.004/0.000 ms\n"));
    let o = cli_run(&mut e, pc1, "ping -c2 -s 1472 -M do 10.0.2.10", 30 * SEC);
    t.check("big df ping", o.starts_with("PING 10.0.2.10 (10.0.2.10) 1472(1500) bytes of data.\n1480 bytes from 10.0.2.10: icmp_seq=1 ttl=63 time=") && o.contains("2 packets transmitted, 2 received, 0% packet loss"));
    t.check("too big", cli_run(&mut e, pc1, "ping -c 1 -s 1473 -M do 10.0.2.10", 15 * SEC).contains("\nping: local error: message too long, mtu=1500\n"));
    t.check("ttl exceeded", cli_run(&mut e, pc1, "ping -c 1 -t 1 10.0.2.10", 15 * SEC).contains("\nFrom 10.0.1.1 icmp_seq=1 Time to live exceeded\n\n--- 10.0.2.10 ping statistics ---\n1 packets transmitted, 0 received, +1 errors, 100% packet loss, time 0ms\n"));
    t.check("host unreachable", cli_run(&mut e, pc1, "ping -c 1 10.0.1.99", 15 * SEC).contains("\nFrom 10.0.1.10 icmp_seq=1 Destination Host Unreachable\n"));
    t.eq("no route", cli(&mut e, pc2, "ip route del default"), String::new());
    t.eq("network unreachable", cli(&mut e, pc2, "ping -c 1 10.0.1.10"), String::from("ping: connect: Network is unreachable\n"));
    t.eq("route back", cli(&mut e, pc2, "ip route add default via 10.0.2.1"), String::new());
    t.eq("usage", cli(&mut e, pc1, "ping"), String::from("ping: usage error: Destination address required\n"));
    t.eq("bad count", cli(&mut e, pc1, "ping -c x 10.0.2.10"), String::from("ping: invalid argument: 'x'\n"));
    t.eq("name", cli(&mut e, pc1, "ping server.lab"), String::from("ping: server.lab: Temporary failure in name resolution\n"));
    // until Ctrl+C
    let _ = e.take_reports();
    // (more than a minute of virtual time went by: Linux asks ARP again before the first reply)
    let _ = e.cli_line(pc1, "ping -i 0.5 10.0.2.10");
    e.run_until(e.now() + 1200 * MS, BUDGET);
    let _ = e.cli_interrupt(pc1);
    e.run_until(e.now() + SEC, BUDGET);
    let o = out(&mut e, pc1);
    t.eq("ctrl+c", o, String::from("PING 10.0.2.10 (10.0.2.10) 56(84) bytes of data.\n64 bytes from 10.0.2.10: icmp_seq=1 ttl=63 time=0.005 ms\n64 bytes from 10.0.2.10: icmp_seq=2 ttl=63 time=0.004 ms\n64 bytes from 10.0.2.10: icmp_seq=3 ttl=63 time=0.004 ms\n\n--- 10.0.2.10 ping statistics ---\n3 packets transmitted, 3 received, 0% packet loss, time 1000ms\nrtt min/avg/max/mdev = 0.004/0.004/0.005/0.000 ms\n"));
    t.check("ctrl+c ends it", !e.cli_busy(pc1));
    t.eq("ctrl+c at the prompt", {
        let _ = e.cli_interrupt(pc1);
        out(&mut e, pc1)
    }, String::from("\n"));
    let o = cli_run(&mut e, pc1, "traceroute -n 10.0.2.10", 60 * SEC);
    t.eq("traceroute", o, String::from("traceroute to 10.0.2.10 (10.0.2.10), 30 hops max, 60 byte packets\n 1  10.0.1.1  0.002 ms  0.002 ms  0.002 ms\n 2  10.0.2.10  0.004 ms  0.004 ms  0.004 ms\n"));
    let o = cli_run(&mut e, pc1, "traceroute -q 1 -m 1 10.0.2.10", 60 * SEC);
    t.eq("traceroute names", o, String::from("traceroute to 10.0.2.10 (10.0.2.10), 1 hops max, 60 byte packets\n 1  10.0.1.1 (10.0.1.1)  0.003 ms\n"));
    t.eq("traceroute usage", cli(&mut e, pc1, "traceroute"), String::from("Specify \"host\" missing argument.\n"));
}

fn round_trip(t: &mut T) {
    // a configuration made at the console comes back the same from the startup configuration
    let (p, pc1, sw, mls) = switched();
    let mut e = engine(&p);
    let _ = lines(&mut e, sw, &["en", "conf t", "hostname Access-1", "vlan 10", "name Sales", "vlan 20", "interface gi1/0/1", "switchport access vlan 10", "switchport mode access", "description PC1", "interface te1/1/1", "switchport mode trunk", "switchport trunk allowed vlan 1,10,20", "interface gi1/0/9", "shutdown", "interface vlan 10", "ip address 10.0.10.2 255.255.255.0", "no shutdown", "exit", "ip route 0.0.0.0 0.0.0.0 10.0.10.1", "no ip domain-lookup", "line vty 0 4", "password secret", "logging synchronous", "end"]);
    let _ = lines(&mut e, mls, &["en", "conf t", "ip routing", "interface te1/1/2", "no switchport", "ip address 192.168.99.1 255.255.255.252", "ip address 192.168.98.1 255.255.255.0 secondary", "interface loopback 0", "ip address 10.255.0.1 255.255.255.255", "interface vlan 10", "ip address 10.0.10.1 255.255.255.0", "no shutdown", "end"]);
    let _ = lines(&mut e, pc1, &["ip addr add 10.0.10.10/24 dev eth0", "ip route add default via 10.0.10.1", "ip link add link eth0 name eth0.30 type vlan id 30", "ip addr add 10.0.30.10/24 dev eth0.30", "ip neigh add 10.0.10.1 lladdr 02:4a:00:00:03:19 dev eth0", "sysctl -w net.ipv4.ip_forward=1"]);
    let mut q = p.clone();
    for id in [pc1, sw, mls] {
        let c = e.running_config(id);
        if let Some(d) = q.device_mut(id) {
            d.startup_config = c;
        }
    }
    let e2 = engine(&q);
    for (id, what) in [(sw, "switch"), (mls, "l3 switch"), (pc1, "linux")] {
        t.eq(&format!("{} boots clean", what), e2.boot_errors(id), Vec::new());
        t.eq(&format!("{} round trip", what), e2.running_config(id), e.running_config(id));
    }
    t.check("hostname from the startup config", e2.node(sw).is_some_and(|n| n.name == "Access-1"));
    t.eq("linux config", e.running_config(pc1), String::from("hostname PC1\nsysctl -w net.ipv4.ip_forward=1\nip addr add 10.0.10.10/24 dev eth0\nip link add link eth0 name eth0.30 type vlan id 30\nip addr add 10.0.30.10/24 dev eth0.30\nip route add default via 10.0.10.1\nip neigh add 10.0.10.1 lladdr 02:4a:00:00:03:19 dev eth0\n"));
    t.check("line settings", e.running_config(sw).contains("line con 0\nline vty 0 4\n login\n password secret\n logging synchronous\n!\nend\n"));
    let ib = {
        let mut e3 = engine(&q);
        cli(&mut e3, sw, "show ip interface brief")
    };
    t.check("nvram method", ib.contains("\nVlan10                 10.0.10.2       YES NVRAM  up                    up\n"));
    // a broken startup configuration: the console says so, boot_errors keeps the lines
    let mut b = p.clone();
    if let Some(d) = b.device_mut(sw) {
        d.startup_config = String::from("hostname SW1\ninterface gi1/0/3\n switchport acess vlan 10\n ip route 1.2.3.4\n!\nend\n");
    }
    if let Some(d) = b.device_mut(pc1) {
        d.startup_config = String::from("ip addr add 10.0.0.1/24 dev eth7\n");
    }
    let mut eb = engine(&b);
    let errs = eb.boot_errors(sw);
    t.eq("boot error lines", errs.iter().map(|x| x.line).collect::<Vec<_>>(), alloc::vec![3, 4]);
    t.eq("boot error text", errs.first().map(|x| x.msg.clone()), Some(String::from("% Invalid input detected at '^' marker.")));
    t.check("boot error on the console", out(&mut eb, sw).contains("switchport acess vlan 10\n             ^\n% Invalid input detected at '^' marker.\n"));
    t.eq("linux boot error", eb.boot_errors(pc1).iter().map(|x| x.msg.clone()).collect::<Vec<_>>(), alloc::vec![String::from("Cannot find device \"eth7\"")]);
    t.eq("startup kept", eb.startup_config(pc1), String::from("ip addr add 10.0.0.1/24 dev eth7\n"));
}

fn driver_reports(t: &mut T) {
    let (p, pc1, r1, _pc2) = routed();
    let mut d = Inline::new(engine(&p));
    let _ = d.reports();
    t.check("cli through the driver", d.send(Cmd::Cli { dev: r1, line: String::from("enable") }, 0).is_ok());
    let r = d.reports();
    t.eq("state report", r, alloc::vec![Report::CliState { dev: r1, prompt: String::from("R1#"), busy: false }]);
    let _ = d.send(Cmd::Cli { dev: pc1, line: String::from("ping 10.0.2.10") }, 0);
    let r = d.reports();
    t.check("busy report", r.iter().any(|x| *x == Report::CliState { dev: pc1, prompt: String::from("root@PC1:~# "), busy: true }));
    let _ = d.send(Cmd::RunFor(2500 * MS), 0);
    let _ = d.send(Cmd::Interrupt { dev: pc1 }, 0);
    let r = d.reports();
    let text: String = r
        .iter()
        .filter_map(|x| match x {
            Report::CliOut { dev, bytes, .. } if *dev == pc1 => Some(String::from_utf8_lossy(bytes).into_owned()),
            _ => None,
        })
        .collect();
    t.check("output and times", text.contains("--- 10.0.2.10 ping statistics ---\n3 packets transmitted, 2 received") && r.iter().any(|x| matches!(x, Report::CliOut { dev, at, .. } if *dev == pc1 && *at == SEC + 4104)));
    t.check("idle report", r.iter().any(|x| *x == Report::CliState { dev: pc1, prompt: String::from("root@PC1:~# "), busy: false }));
    t.check("unknown device", d.send(Cmd::Cli { dev: 99, line: String::from("x") }, 0).is_err());
    let _ = d.send(Cmd::Cli { dev: r1, line: String::from("conf t") }, 0);
    let _ = d.send(Cmd::Cli { dev: r1, line: String::from("hostname Edge") }, 0);
    t.check("hostname report", d.reports().contains(&Report::Hostname { dev: r1, name: String::from("Edge") }));
    // the engine copies with its consoles
    let c = d.eng.clone();
    t.eq("clone keeps the console", c.cli_prompt(r1), String::from("Edge(config)#"));
    let _ = d.send(Cmd::Cli { dev: r1, line: String::from("end") }, 0);
    t.eq("power off", d.send(Cmd::Apply(jfn_sim::Action::Power { dev: r1, on: false }), 0).is_ok() && d.send(Cmd::Cli { dev: r1, line: String::from("show arp") }, 0).is_err(), true);
    // the capture of a run: one interface per cable, every frame
    let (bytes, n) = jfn_sim::runner::capture(&d.eng);
    let cap = netpkt::pcapng::read(&bytes);
    t.check("pcapng", n > 10 && cap.is_ok_and(|c| c.packets.len() == n && c.interfaces.len() == 2));
    // a device added while it runs boots its startup configuration
    let mut q = p.clone();
    let mut nd = q.device(pc1).cloned().unwrap_or_default();
    nd.id = 77;
    nd.name = String::from("PC77");
    nd.startup_config = String::from("hostname PC77\nip addr add 10.0.1.77/24 dev eth0\n");
    q.devices.clear();
    let added = d.send(Cmd::Apply(jfn_sim::Action::AddDevice(nd)), 0).is_ok();
    t.check("added device boots", added && d.eng.ifaces(77).iter().any(|f| f.name == "eth0" && f.addrs.len() == 1));
}
