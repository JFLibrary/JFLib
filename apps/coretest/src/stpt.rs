// stpt -- checks of netpkt::stp: Configuration, TCN and RST BPDUs byte for byte (the layouts of
// IEEE 802.1D-1998 9.3 and 802.1D-2004 9.3.3, written out here by hand), bridge and port
// identifiers, the path costs of 802.1D-2004 Table 17-3, the validation of 9.3.4, the frames
// (802.3 length, LLC 42 42 03, padding) and the dissector (fields, ranges, summaries)

use alloc::string::String;
use alloc::vec::Vec;

use netpkt::decode::{self, Proto};
use netpkt::stp::{self, BpduErr, Bpdu, BridgeId, Kind, PortId, Role, Times};
use netpkt::Mac;

use crate::T;

const ROOT_MAC: Mac = Mac([0x02, 0x4A, 0x00, 0x00, 0x01, 0x00]);
const BR_MAC: Mac = Mac([0x02, 0x4A, 0x00, 0x00, 0x02, 0x00]);
const PORT_MAC: Mac = Mac([0x02, 0x4A, 0x00, 0x00, 0x02, 0x02]);
const T20: Times = Times { msg_age: 1, max_age: 20, hello: 2, fwd_delay: 15 };

/// a Configuration BPDU of bridge 32768/02:4a:00:00:02:00, port 128.2, one hop from the root
/// 32768/02:4a:00:00:01:00 over a gigabit link, with the topology change flag
const CONFIG: [u8; 35] = [
    0x00, 0x00, // Protocol Identifier
    0x00, // Protocol Version Identifier: STP
    0x00, // BPDU Type: Configuration
    0x01, // flags: Topology Change
    0x80, 0x00, 0x02, 0x4A, 0x00, 0x00, 0x01, 0x00, // Root Identifier
    0x00, 0x00, 0x4E, 0x20, // Root Path Cost 20000
    0x80, 0x00, 0x02, 0x4A, 0x00, 0x00, 0x02, 0x00, // Bridge Identifier
    0x80, 0x02, // Port Identifier 128.2
    0x01, 0x00, // Message Age 1 s
    0x14, 0x00, // Max Age 20 s
    0x02, 0x00, // Hello Time 2 s
    0x0F, 0x00, // Forward Delay 15 s
];

/// an RST BPDU of the root 4096/02:4a:00:00:01:00 from its port 128.1: designated, learning,
/// forwarding, proposal
const RST: [u8; 36] = [
    0x00, 0x00, 0x02, 0x02, // protocol, version 2, type RST
    0x3E, // flags: proposal, role designated, learning, forwarding
    0x10, 0x00, 0x02, 0x4A, 0x00, 0x00, 0x01, 0x00, // Root Identifier
    0x00, 0x00, 0x00, 0x00, // Root Path Cost 0
    0x10, 0x00, 0x02, 0x4A, 0x00, 0x00, 0x01, 0x00, // Bridge Identifier
    0x80, 0x01, // Port Identifier
    0x00, 0x00, 0x14, 0x00, 0x02, 0x00, 0x0F, 0x00, // 0 s, 20 s, 2 s, 15 s
    0x00, // Version 1 Length
];

pub fn run(t: &mut T) {
    ids_and_costs(t);
    config_bytes(t);
    rst_and_tcn(t);
    validation(t);
    frames(t);
    dissector(t);
}

fn ids_and_costs(t: &mut T) {
    let b = BridgeId::new(32768, ROOT_MAC);
    t.eq("bridge id value", b.0, 0x8000_024A_0000_0100);
    t.eq("bridge id parts", (b.priority(), b.ext(), b.mac()), (32768, 0, ROOT_MAC));
    t.eq("bridge id text", alloc::format!("{}", b), String::from("32768/0/02:4a:00:00:01:00"));
    // the system ID extension (per-VLAN trees put the VLAN there; one tree keeps it 0)
    let v = BridgeId::new(32768 + 10, ROOT_MAC);
    t.eq("system id extension", (v.ext(), alloc::format!("{}", v)), (10, String::from("32768/10/02:4a:00:00:01:00")));
    t.check("lower priority wins", BridgeId::new(4096, BR_MAC) < BridgeId::new(32768, ROOT_MAC));
    t.check("then the lower MAC", BridgeId::new(32768, ROOT_MAC) < BridgeId::new(32768, BR_MAC));
    let p = PortId::new(128, 2);
    t.eq("port id", (p.0, p.priority(), p.number()), (0x8002, 128, 2));
    t.eq("port id text", alloc::format!("{}", PortId::new(240, 4095)), String::from("240.4095"));
    t.eq("port priority steps of 16", PortId::new(130, 1).priority(), 128);
    let costs: Vec<u32> = [10, 1_000, 10_000, 100_000, 1_000_000, 2_500_000, 10_000_000, 25_000_000, 100_000_000, 1_000_000_000].iter().map(|&k| stp::path_cost(k)).collect();
    t.eq("path costs (Table 17-3)", costs, alloc::vec![200_000_000, 20_000_000, 2_000_000, 200_000, 20_000, 8_000, 2_000, 800, 200, 20]);
}

fn config_bytes(t: &mut T) {
    let b = Bpdu::config(stp::F_TC, BridgeId::new(32768, ROOT_MAC), 20_000, BridgeId::new(32768, BR_MAC), PortId::new(128, 2), T20);
    t.eq("config bytes", b.to_bytes(), CONFIG.to_vec());
    let r = Bpdu::parse(&CONFIG);
    t.eq("config parses", r, Ok(b));
    t.eq("config times", b.times(), T20);
    t.check("config flags", b.tc() && !b.tca() && b.role() == Role::Unknown && !b.proposal());
    // only TC and TC acknowledgment exist in a Configuration BPDU
    let a = Bpdu::config(0xFF, BridgeId(0), 0, BridgeId(0), PortId(0), Times::default());
    t.eq("config flags kept", a.flags, stp::F_TC | stp::F_TCA);
    t.check("tca", a.tca());
}

fn rst_and_tcn(t: &mut T) {
    let root = BridgeId::new(4096, ROOT_MAC);
    let r = Bpdu::rst(stp::F_PROPOSAL | stp::F_LEARNING | stp::F_FORWARDING, root, 0, root, PortId::new(128, 1), Times { msg_age: 0, ..T20 }).with_role(Role::Designated);
    t.eq("rst bytes", r.to_bytes(), RST.to_vec());
    t.eq("rst parses", Bpdu::parse(&RST), Ok(r));
    t.check("rst flags", r.proposal() && r.learning() && r.forwarding() && !r.agreement() && !r.tc() && r.role() == Role::Designated);
    t.eq("rst role bits", (Role::AltBackup.bits(), Role::Root.bits(), Role::Designated.bits()), (0x04, 0x08, 0x0C));
    // the root port's answer: agreement, root role, learning and forwarding
    let ag = r.with_role(Role::Root);
    t.eq("role changes only its bits", ag.flags, 0x3A);
    t.eq("tcn bytes", Bpdu::tcn().to_bytes(), alloc::vec![0, 0, 0, 0x80]);
    t.eq("tcn parses", Bpdu::parse(&[0, 0, 0, 0x80]).map(|b| b.kind), Ok(Kind::Tcn));
    // an MST BPDU (version 3) is an RST BPDU to RSTP
    let mut m = RST.to_vec();
    m[2] = 3;
    m.extend_from_slice(&[0x00, 0x40]);
    t.eq("mst as rst", Bpdu::parse(&m).map(|b| (b.kind, b.version)), Ok((Kind::Rst, 3)));
}

fn validation(t: &mut T) {
    t.eq("short config", Bpdu::parse(&CONFIG[..34]), Err(BpduErr::Short));
    t.eq("short rst", Bpdu::parse(&RST[..35]), Err(BpduErr::Short));
    t.eq("short tcn", Bpdu::parse(&[0, 0, 0]), Err(BpduErr::Short));
    let mut p = CONFIG;
    p[1] = 1;
    t.eq("protocol id", Bpdu::parse(&p), Err(BpduErr::Protocol));
    // type 2 needs version 2 or more (802.1D-2004 9.3.4 c)
    let mut v = RST;
    v[2] = 1;
    t.eq("rst with version 1", Bpdu::parse(&v), Err(BpduErr::Type));
    let mut u = CONFIG;
    u[3] = 0x42;
    t.eq("unknown type", Bpdu::parse(&u), Err(BpduErr::Type));
    // a config BPDU with more bytes (a later version) still reads
    let mut long = CONFIG.to_vec();
    long.extend_from_slice(&[0, 0, 0]);
    t.check("longer config", Bpdu::parse(&long).is_ok());
}

fn frames(t: &mut T) {
    let b = Bpdu::parse(&CONFIG).unwrap_or(Bpdu::tcn());
    let f = b.frame(PORT_MAC);
    let mut want: Vec<u8> = alloc::vec![0x01, 0x80, 0xC2, 0x00, 0x00, 0x00, 0x02, 0x4A, 0x00, 0x00, 0x02, 0x02, 0x00, 0x26, 0x42, 0x42, 0x03];
    want.extend_from_slice(&CONFIG);
    want.resize(60, 0);
    t.eq("config frame", f.clone(), want);
    t.eq("frame back", stp::of_frame(&f), Some(b));
    let tcn = Bpdu::tcn().frame(PORT_MAC);
    t.eq("tcn frame", (tcn.len(), tcn[12], tcn[13], tcn[14..21].to_vec()), (60, 0x00, 0x07, alloc::vec![0x42, 0x42, 0x03, 0, 0, 0, 0x80]));
    let rst = Bpdu::parse(&RST).unwrap_or(Bpdu::tcn()).frame(PORT_MAC);
    t.eq("rst frame length field", (rst[12], rst[13]), (0x00, 0x27));
    // tagged, other address, other SAP, Ethernet II: no BPDU
    let tagged = netpkt::eth::tagged(&f, netpkt::eth::VlanTag::new(10));
    t.check("tagged bpdu", stp::of_frame(&tagged) == Some(b));
    let mut other = f.clone();
    other[5] = 0x02;
    t.eq("other group address", stp::of_frame(&other), None);
    let mut sap = f.clone();
    sap[14] = 0x43;
    t.eq("other sap", stp::of_frame(&sap), None);
    let mut e2 = f.clone();
    e2[12] = 0x08;
    t.eq("ethernet ii", stp::of_frame(&e2), None);
}

fn dissector(t: &mut T) {
    let f = Bpdu::parse(&CONFIG).unwrap_or(Bpdu::tcn()).frame(PORT_MAC);
    t.eq("mask", decode::proto_mask(&f), decode::P_ETH | decode::P_LLC | decode::P_STP);
    let ls = decode::layers(&f);
    t.eq("layers", ls.iter().map(|l| l.proto).collect::<Vec<_>>(), alloc::vec![Proto::Eth, Proto::Llc, Proto::Stp]);
    let st = ls.get(2);
    t.eq("stp layer", st.map(|l| (l.off, l.len, l.title.clone())), Some((17, 35, String::from("Spanning Tree Protocol (Configuration BPDU)"))));
    let field = |name: &str| st.and_then(|l| l.fields.iter().find(|x| x.name == name)).map(|x| (x.value.clone(), x.off, x.len));
    t.eq("flags field", field("BPDU flags"), Some((String::from("0x01 (Topology Change)"), 21, 1)));
    t.eq("root field", field("Root Identifier"), Some((String::from("32768 / 0 / 02:4a:00:00:01:00"), 22, 8)));
    t.eq("cost field", field("Root Path Cost"), Some((String::from("20000"), 30, 4)));
    t.eq("port field", field("Port identifier"), Some((String::from("0x8002 (priority 128, port 2)"), 42, 2)));
    t.eq("times", (field("Message Age").map(|x| x.0), field("Max Age").map(|x| x.0), field("Forward Delay").map(|x| x.1)), (Some(String::from("1")), Some(String::from("20")), Some(50)));
    t.check("no problems", st.is_some_and(|l| l.problems.is_empty()));
    t.check("padding", ls.first().is_some_and(|l| l.title.starts_with("IEEE 802.3 Ethernet") && l.fields.iter().any(|x| x.name == "Padding" && x.value == "8 bytes")));
    t.eq("config summary", decode::info(&f), (Proto::Stp, String::from("Conf. TC + Root = 32768/0/02:4a:00:00:01:00  Cost = 20000  Port = 0x8002")));
    let r = Bpdu::parse(&RST).unwrap_or(Bpdu::tcn()).frame(PORT_MAC);
    t.eq("rst summary", decode::info(&r).1, String::from("RST. Root = 4096/0/02:4a:00:00:01:00  Cost = 0  Port = 0x8001"));
    let rl = decode::layers(&r);
    let rf = |name: &str| rl.get(2).and_then(|l| l.fields.iter().find(|x| x.name == name)).map(|x| x.value.clone());
    t.eq("rst flags field", rf("BPDU flags"), Some(String::from("0x3e (Forwarding, Learning, Port Role: Designated, Proposal)")));
    t.eq("rst version field", (rf("Protocol Version Identifier"), rf("Version 1 Length")), (Some(String::from("Rapid Spanning Tree (2)")), Some(String::from("0"))));
    t.eq("tcn summary", decode::info(&Bpdu::tcn().frame(PORT_MAC)).1, String::from("Topology Change Notification"));
    t.eq("agreement text", stp::flags_text(Kind::Rst, 0x78), String::from("Agreement, Forwarding, Learning, Port Role: Root"));
    // a message age at max age is a problem a receiver drops
    let mut old = CONFIG;
    old[27] = 0x14;
    let of = Bpdu::parse(&old).unwrap_or(Bpdu::tcn()).frame(PORT_MAC);
    t.check("message age problem", decode::layers(&of).get(2).is_some_and(|l| l.problems.len() == 1));
    let mut bad = f.clone();
    bad[17 + 3] = 0x42;
    t.check("bad type problem", decode::layers(&bad).get(2).is_some_and(|l| l.problems.first().is_some_and(|p| p.contains("unknown BPDU type"))));
}
