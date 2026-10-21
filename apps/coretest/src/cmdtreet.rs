// cmdtreet -- checks of base::cmdtree: IOS-like abbreviations, the "^" position of errors,
// incomplete and ambiguous lines, typed parameters, interface names in one or two words, the
// "no" form, options in any order, "?" help with its IOS-like layout, Tab completion

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use base::cmdtree::{self, Error, Help, ModeId, Names, Tree, Val, F_HIDDEN, F_NO};

use crate::T;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum C {
    Enable,
    ShowRun,
    ShowIpBrief,
    ShowIpRoute,
    SetClock,
    Ping,
    ConfT,
    Hostname,
    Interface,
    IpAddr,
    Shutdown,
    Vlan,
    Descr,
    Arp,
    Route,
    Ipv6,
}

struct Ifs;

impl Names for Ifs {
    fn names(&self, list: u8) -> Vec<String> {
        let v: &[&str] = match list {
            0 => &["GigabitEthernet0/0/0", "GigabitEthernet0/0/1", "TenGigabitEthernet0/1/0", "Loopback*", "GigabitEthernet0/0/0.*"],
            1 => &["TenGigabitEthernet1/1/1", "Tunnel*"],
            _ => &["TwoGigabitEthernet0", "Dot11Radio0", "Dot11Radio1", "BVI1"],
        };
        v.iter().map(|s| String::from(*s)).collect()
    }
    fn type_help(&self, _list: u8, ty: &str) -> String {
        match ty {
            "GigabitEthernet" => String::from("Gigabit Ethernet port"),
            _ => alloc::format!("{} interface", ty),
        }
    }
}

/// exec ">" (no "no"), config "(config)#" and interface "(config-if)#" with "no"
fn tree() -> (Tree<C>, ModeId, ModeId, ModeId) {
    let mut t = Tree::new();
    let ex = t.mode("#", "Commands at the privileged level:", None, None);
    let cf = t.mode("(config)#", "Commands for the whole device:", Some(ex), Some("Undo a setting or restore its default"));
    let ifm = t.mode("(config-if)#", "Commands for this interface:", Some(cf), Some("Undo a setting or restore its default"));
    t.add(ex, "enable", "Switch to the privileged level", Some(C::Enable), 0);
    t.add(ex, "show", "Display the state of the device", None, 0);
    t.add(ex, "show running-config", "Configuration in effect now", Some(C::ShowRun), 0);
    t.add(ex, "show ip", "IPv4 state", None, 0);
    t.add(ex, "show ip interface", "IPv4 settings of the interfaces", None, 0);
    t.add(ex, "show ip interface brief", "One line per interface: address and state", Some(C::ShowIpBrief), 0);
    t.add(ex, "show ip route", "Routes this device knows", Some(C::ShowIpRoute), 0);
    t.add(ex, "set", "Change a setting outside the configuration", None, 0);
    t.add(ex, "set clock", "Date and time of the device", Some(C::SetClock), F_HIDDEN);
    t.add(ex, "ping", "Check whether a host answers", None, 0);
    t.add(ex, "ping WORD=target", "Address or name of the host", Some(C::Ping), 0);
    t.add(ex, "ping WORD=target repeat", "How many echo requests to send", None, 0);
    t.add(ex, "ping WORD=target repeat <1-2147483647>=count", "Number of echo requests", Some(C::Ping), 0);
    t.add(ex, "ping WORD=target size", "Size of each packet", None, 0);
    t.add(ex, "ping WORD=target size <36-18024>=bytes", "Packet size in bytes", Some(C::Ping), 0);
    t.add(ex, "ping WORD=target df-bit", "Forbid fragmentation (set the DF bit)", Some(C::Ping), 0);
    for p in ["ping WORD=target repeat <1-2147483647>=count", "ping WORD=target size <36-18024>=bytes", "ping WORD=target df-bit"] {
        t.also(ex, p, "ping WORD=target");
    }
    t.add(ex, "configure", "Start changing the configuration", None, 0);
    t.add(ex, "configure terminal", "Type configuration commands here", Some(C::ConfT), 0);
    t.add(cf, "hostname", "Give the device a name", None, F_NO);
    t.add(cf, "hostname WORD=name", "The new name", Some(C::Hostname), 0);
    t.add_no(cf, "hostname", C::Hostname);
    t.add(cf, "interface", "Pick an interface to set up", None, F_NO);
    t.add(cf, "interface NAME=if", "", Some(C::Interface), 0);
    t.add(cf, "radio", "Pick a radio to set up (test)", None, 0);
    t.add(cf, "radio NAME2=if", "", Some(C::Interface), 0);
    t.add(cf, "ip", "IPv4 settings of the device", None, F_NO);
    t.add(cf, "ip route", "Add a static route", None, 0);
    t.add(cf, "ip route A.B.C.D=net MASK=mask A.B.C.D=gw", "Next router on the way", Some(C::Route), 0);
    t.add(cf, "ip route A.B.C.D=net MASK=mask NAME1=out", "", Some(C::Route), 0);
    t.add(cf, "ip route A.B.C.D=net MASK=mask A.B.C.D=gw <1-255>=ad", "Preference of this route, lower wins", Some(C::Route), 0);
    t.add(cf, "ipv6", "IPv6 settings of the device", Some(C::Ipv6), 0);
    t.add(cf, "vlan", "Create or change a VLAN", None, F_NO);
    t.add(cf, "vlan <1-4094>=vid", "VLAN number", Some(C::Vlan), 0);
    t.add(cf, "banner", "Text shown before the login", None, 0);
    t.add(cf, "banner WORD=banner", "The text between two equal marks, such as #Hello#", Some(C::Descr), 0);
    t.add(cf, "arp", "Add a fixed ARP entry", None, F_NO);
    t.add(cf, "arp A.B.C.D=ip H.H.H=mac arpa", "Ethernet frames", Some(C::Arp), 0);
    t.add(ifm, "ip", "IPv4 settings of this interface", None, F_NO);
    t.add(ifm, "ip address", "Give the interface an address", None, 0);
    t.add(ifm, "ip address A.B.C.D=addr", "The address", None, 0);
    t.add(ifm, "ip address A.B.C.D=addr MASK=mask", "The network mask", Some(C::IpAddr), 0);
    t.add(ifm, "ip address A.B.C.D=addr MASK=mask secondary", "An extra address next to the first one", Some(C::IpAddr), 0);
    t.add(ifm, "ip address A.B.C.D/nn=pfx", "Address with prefix length", Some(C::IpAddr), F_HIDDEN);
    t.add_no(ifm, "ip address", C::IpAddr);
    t.add(ifm, "shutdown", "Switch the interface off", Some(C::Shutdown), F_NO);
    t.add(ifm, "description", "A note about this interface", None, F_NO);
    t.add(ifm, "description LINE=text", "The note, up to 240 characters", Some(C::Descr), 0);
    (t, ex, cf, ifm)
}

pub fn run(t: &mut T) {
    let (tr, ex, cf, ifm) = tree();
    let n = &Ifs;
    let cmd = |m: ModeId, l: &str| tr.parse(m, l, n).map(|p| p.cmd);
    // abbreviations
    t.eq("sh ip int br", cmd(ex, "sh ip int br"), Ok(C::ShowIpBrief));
    t.eq("case and spaces", cmd(ex, "  SHOW   Ip  ROUTE "), Ok(C::ShowIpRoute));
    t.eq("full words", cmd(ex, "show running-config"), Ok(C::ShowRun));
    t.eq("en", cmd(ex, "en"), Ok(C::Enable));
    t.eq("hidden still parses", cmd(ex, "set clock"), Ok(C::SetClock));
    t.eq("ambiguous s", cmd(ex, "s ip"), Err(Error::Ambiguous(0)));
    t.eq("exact word wins", cmd(cf, "ip route 10.0.0.0 255.0.0.0 10.0.0.1"), Ok(C::Route));
    t.eq("ipv6 by its prefix", cmd(cf, "ipv"), Ok(C::Ipv6));
    // the position of the marker
    t.eq("bad keyword", cmd(ex, "show ip interfacx brief"), Err(Error::Invalid(16)));
    t.eq("extra character", cmd(ex, "show ip routee"), Err(Error::Invalid(13)));
    t.eq("no such word", cmd(ex, "show ip route xyz"), Err(Error::Invalid(14)));
    t.eq("unknown first word", cmd(ex, "frobnicate"), Err(Error::Invalid(0)));
    t.eq("incomplete", cmd(ex, "show ip"), Err(Error::Incomplete));
    t.eq("empty", cmd(ex, "   "), Err(Error::Empty));
    let msg = Error::Invalid(16).render(3, "show ip interfacx brief");
    t.eq("marker text", msg, String::from("                   ^\n% Invalid input detected at '^' marker.\n\n"));
    t.eq("incomplete text", Error::Incomplete.render(3, "show ip"), String::from("% Incomplete command.\n\n"));
    t.eq("ambiguous text", Error::Ambiguous(0).render(3, " s ip "), String::from("% Ambiguous command:  \"s ip\"\n"));
    // typed parameters
    let p = tr.parse(ifm, "ip add 10.0.0.1 255.255.255.0 sec", n);
    t.check("ip address secondary", p.as_ref().is_ok_and(|p| p.cmd == C::IpAddr && p.ip("addr") == Some([10, 0, 0, 1]) && p.mask("mask") == Some(24) && p.has("secondary") && !p.no));
    t.eq("bad octet", cmd(ifm, "ip address 10.0.0.300 255.255.255.0"), Err(Error::Invalid(11)));
    t.eq("mask with holes", cmd(ifm, "ip address 10.0.0.1 255.0.255.0"), Err(Error::Invalid(20)));
    t.check("prefix form", tr.parse(ifm, "ip address 10.1.0.1/16", n).is_ok_and(|p| p.prefix("pfx") == Some(([10, 1, 0, 1], 16))));
    t.check("vlan range", tr.parse(cf, "vlan 4094", n).is_ok_and(|p| p.num("vid") == Some(4094)));
    t.eq("vlan out of range", cmd(cf, "vlan 4095"), Err(Error::Invalid(5)));
    t.eq("not a number", cmd(cf, "vlan 1o"), Err(Error::Invalid(5)));
    let a = tr.parse(cf, "arp 10.0.0.5 024a.0000.0101 arpa", n);
    t.check("mac", a.is_ok_and(|p| p.mac("mac") == Some([2, 0x4A, 0, 0, 1, 1])));
    t.check("mac colons", tr.parse(cf, "arp 10.0.0.5 02:4a:00:00:01:01 ARPA", n).is_ok());
    t.eq("bad mac", cmd(cf, "arp 10.0.0.5 024a.0000.01 arpa"), Err(Error::Invalid(13)));
    let d = tr.parse(ifm, "description  uplink to  core ", n);
    t.eq("line keeps inner spaces", d.ok().and_then(|p| p.text("text").map(String::from)), Some(String::from("uplink to  core")));
    // names: one word, two words, patterns, errors
    let name = |l: &str| tr.parse(cf, l, n).ok().and_then(|p| p.text("if").map(String::from));
    t.eq("gi0/0/0", name("int gi0/0/0"), Some(String::from("GigabitEthernet0/0/0")));
    t.eq("two words", name("interface GigabitEthernet 0/0/1"), Some(String::from("GigabitEthernet0/0/1")));
    t.eq("te", name("int t0/1/0"), Some(String::from("TenGigabitEthernet0/1/0")));
    t.eq("pattern", name("int lo7"), Some(String::from("Loopback7")));
    t.eq("subinterface", name("int g0/0/0.10"), Some(String::from("GigabitEthernet0/0/0.10")));
    t.eq("digits in a type", name("radio do1"), Some(String::from("Dot11Radio1")));
    t.eq("digits in a type, whole", name("radio Dot11Radio0"), Some(String::from("Dot11Radio0")));
    t.check("types with digits in help", matches!(tr.help(cf, "radio ", n), Help::List { items, .. } if items.iter().map(|i| i.0.as_str()).collect::<Vec<_>>() == vec!["BVI", "Dot11Radio", "TwoGigabitEthernet"]));
    t.eq("no such port", cmd(cf, "interface gi0/0/9"), Err(Error::Invalid(12)));
    t.eq("no such type", cmd(cf, "interface fa0/1"), Err(Error::Invalid(10)));
    t.eq("type only", cmd(cf, "interface gi"), Err(Error::Incomplete));
    t.eq("ambiguous type", cmd(cf, "ip route 10.0.0.0 255.0.0.0 t1/1/1"), Err(Error::Ambiguous(28)));
    t.check("other list", tr.parse(cf, "ip route 10.0.0.0 255.0.0.0 te1/1/1", n).is_ok_and(|p| p.text("out") == Some("TenGigabitEthernet1/1/1") && p.ip("gw").is_none()));
    t.check("distance", tr.parse(cf, "ip route 0.0.0.0 0.0.0.0 10.0.0.1 250", n).is_ok_and(|p| p.num("ad") == Some(250) && p.ip("net") == Some([0; 4])));
    // the no-form
    t.check("no shutdown", tr.parse(ifm, "no shut", n).is_ok_and(|p| p.cmd == C::Shutdown && p.no));
    t.check("no ip address ends early", tr.parse(ifm, "no ip address", n).is_ok_and(|p| p.cmd == C::IpAddr && p.no));
    t.eq("ip address needs more", cmd(ifm, "ip address"), Err(Error::Incomplete));
    t.check("no hostname", tr.parse(cf, "no hostname", n).is_ok_and(|p| p.cmd == C::Hostname && p.no));
    t.eq("only F_NO commands", cmd(cf, "no ipv6"), Err(Error::Invalid(5)));
    t.eq("no in exec", cmd(ex, "no shutdown"), Err(Error::Invalid(0)));
    // options in any order
    let pg = tr.parse(ex, "ping 10.0.0.2 size 1500 repeat 10 df-bit", n);
    t.check("options", pg.is_ok_and(|p| p.cmd == C::Ping && p.num("count") == Some(10) && p.num("bytes") == Some(1500) && p.has("df-bit") && p.text("target") == Some("10.0.0.2")));
    t.check("options the other way", tr.parse(ex, "ping r1 rep 3 si 100", n).is_ok_and(|p| p.num("count") == Some(3)));
    t.eq("option without value", cmd(ex, "ping 10.0.0.2 repeat"), Err(Error::Incomplete));
    t.check("a later value wins", tr.parse(ex, "ping 10.0.0.2 repeat 5 repeat 7", n).is_ok_and(|p| p.num("count") == Some(7)));
    t.check("parameter after a keyword of the same key", tr.parse(cf, "banner hello", n).is_ok_and(|p| p.text("banner") == Some("hello")));
    // help
    let h = tr.help(ex, "show ip ", n);
    t.eq("show ip ?", h, Help::List { title: None, items: vec![(String::from("interface"), String::from("IPv4 settings of the interfaces")), (String::from("route"), String::from("Routes this device knows"))], cr: false });
    let txt = tr.help(ex, "show ip interface ", n).render(3, "");
    t.eq("render", txt, String::from("  brief  One line per interface: address and state\n\n"));
    t.eq("render <cr>", tr.help(ex, "show ip route ", n).render(3, ""), String::from("  <cr>\n\n"));
    let top = tr.help(ex, "", n);
    t.check("title and hidden", matches!(&top, Help::List { title: Some(x), items, cr: false } if x == "Commands at the privileged level:" && items.iter().any(|i| i.0 == "set") && items.len() == 5));
    let r = top.render(0, "");
    t.check("aligned", r.contains("\n  configure  Start changing the configuration\n") && r.contains("\n  ping       Check whether a host answers\n"));
    t.eq("partial word", tr.help(ex, "s", n), Help::Words(vec![String::from("set"), String::from("show")]));
    t.eq("partial words text", tr.help(ex, "s", n).render(0, ""), String::from("set  show  \n\n"));
    t.eq("unrecognized", tr.help(ex, "xy", n), Help::Unrecognized);
    t.eq("help after an error", tr.help(ex, "show xyz ", n), Help::Error(Error::Invalid(5)));
    let ih = tr.help(cf, "interface ", n);
    t.check("interface types", matches!(&ih, Help::List { items, .. } if items.iter().map(|i| i.0.as_str()).collect::<Vec<_>>() == vec!["GigabitEthernet", "Loopback", "TenGigabitEthernet"] && items[0].1 == "Gigabit Ethernet port"));
    let nh = tr.help(cf, "no ", n);
    t.check("no lists negatable commands", matches!(&nh, Help::List { items, .. } if items.iter().map(|i| i.0.as_str()).collect::<Vec<_>>() == vec!["arp", "hostname", "interface", "ip", "vlan"]));
    t.check("parameters first", matches!(tr.help(cf, "vlan ", n), Help::List { items, cr: false, .. } if items[0].0 == "<1-4094>"));
    t.check("no-form may end", matches!(tr.help(ifm, "no ip address ", n), Help::List { cr: true, .. }));
    t.eq("partial number", tr.help(cf, "vlan 1", n), Help::Words(vec![String::from("<1-4094>")]));
    t.eq("partial type", tr.help(cf, "interface g", n), Help::Words(vec![String::from("GigabitEthernet")]));
    // Tab
    t.eq("tab show", tr.complete(ex, "sh", n).line, Some(String::from("show ")));
    t.eq("tab later word", tr.complete(ex, "show ip int", n).line, Some(String::from("show ip interface ")));
    t.eq("tab type", tr.complete(cf, "int gi", n).line, Some(String::from("int GigabitEthernet")));
    let c = tr.complete(ex, "s", n);
    t.check("tab ambiguous", c.line.is_none() && c.choices == vec![String::from("set"), String::from("show")]);
    t.eq("tab whole word", tr.complete(ex, "show", n).line, Some(String::from("show ")));
    t.eq("tab after space", tr.complete(ex, "show ", n).line, None);
    // the first word of EXEC (IOS looks unknown ones up as host names)
    t.check("knows first", tr.knows_first(ex, "sh ip") && !tr.knows_first(ex, "frob") && tr.knows_first(cf, "no hostname"));
    t.eq("prompt", tr.prompt(cf, "R1"), String::from("R1(config)#"));
    // the value parsers
    t.eq("ipv4", cmdtree::parse_ipv4("192.168.001.10"), Some([192, 168, 1, 10]));
    t.check("ipv4 bad", cmdtree::parse_ipv4("1.2.3").is_none() && cmdtree::parse_ipv4("1.2.3.4.5").is_none() && cmdtree::parse_ipv4("1.2..4").is_none() && cmdtree::parse_ipv4("+1.2.3.4").is_none());
    t.eq("masks", (cmdtree::parse_mask("255.255.255.252"), cmdtree::parse_mask("0.0.0.0"), cmdtree::parse_mask("255.255.255.255")), (Some(30), Some(0), Some(32)));
    t.eq("mask holes", cmdtree::parse_mask("255.255.0.255"), None);
    t.eq("mac forms", (cmdtree::parse_mac("AABB.CCDD.EEFF"), cmdtree::parse_mac("aa-bb-cc-dd-ee-ff")), (Some([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]), Some([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF])));
    t.eq("mac bad", cmdtree::parse_mac("aa:bb-cc:dd:ee:ff"), None);
    let v = tr.parse(cf, "interface lo1", n).map(|p| p.vals);
    t.eq("values in order", v, Ok(vec![(String::from("interface"), Val::Key(String::from("interface"))), (String::from("if"), Val::Name(String::from("Loopback1")))]));
}
