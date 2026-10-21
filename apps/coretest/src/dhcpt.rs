// dhcpt -- checks of netpkt::dhcp: a captured DHCP DISCOVER (frame 1 of the Wireshark sample
// capture dhcp.pcap: client 00:0b:82:01:fc:42, transaction 0x3d1d; its IP checksum 0x178b and
// UDP checksum 0x591f check the bytes), the dissector on it, building replies, long options
// (RFC 3396), option overload and malformed messages

use alloc::string::String;
use alloc::vec::Vec;
use alloc::{format, vec};

use netpkt::decode::{self, Proto};
use netpkt::dhcp::{self, Dhcp, DhcpView, MsgType, Opt};
use netpkt::eth::EthView;
use netpkt::ipv4::Ipv4View;
use netpkt::udp::UdpView;
use netpkt::{Ipv4, Mac};

use crate::T;

const CLIENT: Mac = Mac([0x00, 0x0B, 0x82, 0x01, 0xFC, 0x42]);

/// the captured frame: Ethernet, IPv4 0.0.0.0 -> 255.255.255.255, UDP 68 -> 67, 272 bytes of
/// DHCP (the fixed part, cookie, options 53, 61, 50, 55, end, seven bytes of padding)
fn discover_frame() -> Vec<u8> {
    let mut f = vec![0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x0B, 0x82, 0x01, 0xFC, 0x42, 0x08, 0x00];
    f.extend_from_slice(&[0x45, 0x00, 0x01, 0x2C, 0xA8, 0x36, 0x00, 0x00, 0xFA, 0x11, 0x17, 0x8B, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]);
    f.extend_from_slice(&[0x00, 0x44, 0x00, 0x43, 0x01, 0x18, 0x59, 0x1F]);
    f.extend_from_slice(&[0x01, 0x01, 0x06, 0x00, 0x00, 0x00, 0x3D, 0x1D, 0x00, 0x00, 0x00, 0x00]);
    f.extend_from_slice(&[0u8; 16]);
    f.extend_from_slice(&CLIENT.0);
    f.extend_from_slice(&[0u8; 10 + 64 + 128]);
    f.extend_from_slice(&[0x63, 0x82, 0x53, 0x63]);
    f.extend_from_slice(&[0x35, 0x01, 0x01, 0x3D, 0x07, 0x01, 0x00, 0x0B, 0x82, 0x01, 0xFC, 0x42, 0x32, 0x04, 0, 0, 0, 0, 0x37, 0x04, 0x01, 0x03, 0x06, 0x2A, 0xFF]);
    f.extend_from_slice(&[0u8; 7]);
    f
}

pub fn run(t: &mut T) {
    captured(t);
    dissect(t);
    build(t);
    long_and_overload(t);
    malformed(t);
}

fn captured(t: &mut T) {
    let f = discover_frame();
    t.eq("frame length", f.len(), 314);
    let Some(e) = EthView::parse(&f) else {
        t.check("ethernet parses", false);
        return;
    };
    let Some(ip) = Ipv4View::parse(e.payload()) else {
        t.check("ip parses", false);
        return;
    };
    t.check("ip checksum", ip.checksum_ok());
    let Some(u) = UdpView::parse(ip.payload()) else {
        t.check("udp parses", false);
        return;
    };
    t.check("udp checksum", u.checksum_ok_v4(ip.src(), ip.dst()));
    t.eq("ports", (u.src_port(), u.dst_port()), (68, 67));
    let p = u.payload();
    t.eq("dhcp length", p.len(), 272);
    let Some(v) = DhcpView::parse(p) else {
        t.check("dhcp parses", false);
        return;
    };
    t.eq("op htype hlen hops", (v.op(), v.htype(), v.hlen(), v.hops()), (1, 1, 6, 0));
    t.eq("xid", v.xid(), 0x3D1D);
    t.eq("secs flags", (v.secs(), v.flags(), v.broadcast()), (0, 0, false));
    t.eq("addresses", (v.ciaddr(), v.yiaddr(), v.siaddr(), v.giaddr()), (Ipv4::UNSPECIFIED, Ipv4::UNSPECIFIED, Ipv4::UNSPECIFIED, Ipv4::UNSPECIFIED));
    t.eq("mac", v.mac(), Some(CLIENT));
    t.eq("no names", (v.sname().len(), v.file().len()), (0, 0));
    t.eq("message type", v.msg_type(), Some(MsgType::Discover));
    t.eq("option order", v.options().iter().map(|o| (o.0, o.2)).collect::<Vec<_>>(), vec![(53, 240), (61, 243), (50, 252), (55, 258)]);
    let d = Dhcp::read(&v);
    t.eq(
        "options",
        d.opts.clone(),
        vec![Opt::MsgType(MsgType::Discover), Opt::ClientId(vec![1, 0x00, 0x0B, 0x82, 0x01, 0xFC, 0x42]), Opt::RequestedIp(Ipv4::UNSPECIFIED), Opt::Params(vec![1, 3, 6, 42])],
    );
    t.eq("accessors", (d.msg_type(), d.requested_ip(), d.client_id().map(|c| c.len()), d.mac()), (Some(MsgType::Discover), Some(Ipv4::UNSPECIFIED), Some(7), CLIENT));
    // the record writes the same bytes again (with the client's 7 bytes of padding)
    t.check("round trip", d.to_bytes(272) == p);
    t.eq("unpadded length", d.to_bytes(0).len(), 265);
    t.eq("bootp minimum", d.to_bytes(dhcp::BOOTP_MIN).len(), 300);
}

fn dissect(t: &mut T) {
    let f = discover_frame();
    t.eq("info", decode::info(&f), (Proto::Dhcp, String::from("DHCP Discover - Transaction ID 0x3d1d")));
    t.eq("mask", decode::proto_mask(&f), decode::P_ETH | decode::P_IPV4 | decode::P_UDP | decode::P_DHCP);
    let ls = decode::layers(&f);
    let Some(l) = ls.iter().find(|l| l.proto == Proto::Dhcp) else {
        t.check("dhcp layer", false);
        return;
    };
    t.eq("title", l.title.as_str(), "Dynamic Host Configuration Protocol (Discover)");
    t.eq("range", (l.off, l.len), (42, 272));
    t.check("no problems", l.problems.is_empty());
    let field = |name: &str| l.fields.iter().find(|x| x.name == name).map(|x| (x.value.clone(), x.off, x.len));
    t.eq("xid field", field("Transaction ID"), Some((String::from("0x00003d1d"), 46, 4)));
    t.eq("flags field", field("Bootp flags").map(|x| x.0), Some(String::from("0x0000 (Unicast)")));
    t.eq("mac field", field("Client MAC address"), Some((String::from("00:0b:82:01:fc:42"), 70, 6)));
    t.eq("type field", field("DHCP Message Type"), Some((String::from("(53) Discover (1)"), 282, 3)));
    t.eq("client id field", field("Client identifier").map(|x| x.0), Some(String::from("(61) Ethernet 00:0b:82:01:fc:42")));
    t.eq("params field", field("Parameter Request List").map(|x| x.0), Some(String::from("(55) 1 Subnet Mask, 3 Router, 6 Domain Name Server, 42 Network Time Protocol Servers")));
    t.eq("end field", field("End"), Some((String::from("(255)"), 306, 1)));
    t.eq("padding field", field("Padding"), Some((String::from("7 bytes"), 307, 7)));
    t.eq("sname field", field("Server host name").map(|x| x.0), Some(String::from("not given")));
}

fn build(t: &mut T) {
    let server = Ipv4([10, 0, 2, 53]);
    let mut o = Dhcp::new(dhcp::OP_REPLY, 0x1234_5678, CLIENT);
    o.yiaddr = Ipv4([10, 0, 1, 10]);
    o.giaddr = Ipv4([10, 0, 1, 1]);
    o.flags = dhcp::FLAG_BROADCAST;
    o.set(Opt::MsgType(MsgType::Offer));
    o.set(Opt::ServerId(server));
    o.set(Opt::LeaseTime(86_400));
    o.set(Opt::T1(43_200));
    o.set(Opt::T2(75_600));
    o.set(Opt::SubnetMask(Ipv4([255, 255, 255, 0])));
    o.set(Opt::Routers(vec![Ipv4([10, 0, 1, 1])]));
    o.set(Opt::DnsServers(vec![server, Ipv4([10, 0, 2, 54])]));
    o.set(Opt::DomainName(String::from("lab")));
    o.set(Opt::RelayInfo(vec![(dhcp::RELAY_CIRCUIT_ID, b"Gi0/0/0".to_vec()), (dhcp::RELAY_REMOTE_ID, CLIENT.0.to_vec())]));
    let b = o.to_bytes(dhcp::BOOTP_MIN);
    // 240 + 74 bytes of options: longer than the 300 bytes padding would fill up
    t.eq("length", b.len(), 314);
    t.eq("re-read", Dhcp::parse(&b), Some(o.clone()));
    t.check("broadcast flag on the wire", b[10] == 0x80 && b[11] == 0);
    t.eq("first option", &b[240..243], &[53u8, 1, 2][..]);
    t.eq("reply accessors", (o.server_id(), o.lease_time(), o.t1(), o.t2(), o.subnet_mask(), o.domain_name()), (Some(server), Some(86_400), Some(43_200), Some(75_600), Some(Ipv4([255, 255, 255, 0])), Some("lab")));
    t.eq("lists", (o.routers(), o.dns_servers().len()), (vec![Ipv4([10, 0, 1, 1])], 2));
    // set replaces an option in its place; remove takes it out
    o.set(Opt::MsgType(MsgType::Ack));
    t.eq("set replaces", (o.msg_type(), o.opts.len()), (Some(MsgType::Ack), 10));
    o.remove(dhcp::O_RELAY);
    t.eq("remove", o.get(dhcp::O_RELAY), None);
    t.eq("type names", [MsgType::Discover, MsgType::Ack, MsgType::Nak, MsgType::Inform].map(|m| m.name()), ["Discover", "ACK", "NAK", "Inform"]);
    t.eq("option names", [dhcp::option_name(1), dhcp::option_name(82), dhcp::option_name(200)], ["Subnet Mask", "Agent Information Option", "Unknown"]);
}

fn long_and_overload(t: &mut T) {
    // RFC 3396: 300 bytes go into two options with the same code and come back as one
    let mut m = Dhcp::new(dhcp::OP_REQUEST, 7, CLIENT);
    m.set(Opt::MsgType(MsgType::Request));
    let long: Vec<u8> = (0..300u32).map(|i| (i % 251) as u8).collect();
    m.set(Opt::Other(224, long.clone()));
    let b = m.to_bytes(0);
    t.eq("split", (b[243], b[244], b[244 + 256], b[245 + 256]), (224, 255, 224, 45));
    t.eq("joined", Dhcp::parse(&b).and_then(|d| d.get(224).cloned()), Some(Opt::Other(224, long)));
    // option 52: the file field carries options too
    let mut b = Dhcp::new(dhcp::OP_REPLY, 8, CLIENT).to_bytes(0);
    b.pop();
    b.extend_from_slice(&[53, 1, 5, 52, 1, 1, 255]);
    b[108..118].copy_from_slice(&[12, 3, b'p', b'c', b'1', 15, 3, b'l', b'a', b'b']);
    b[118] = 255;
    match Dhcp::parse(&b) {
        Some(d) => {
            t.eq("overload host name", d.host_name(), Some("pc1"));
            t.eq("overload domain", d.domain_name(), Some("lab"));
            t.check("overload file not a name", d.file.is_empty());
        }
        None => t.check("overload parses", false),
    }
}

fn malformed(t: &mut T) {
    let f = discover_frame();
    let p = &f[42..];
    t.check("too short", DhcpView::parse(&p[..239]).is_none());
    let mut bad = p.to_vec();
    bad[236] = 0;
    t.check("no cookie", DhcpView::parse(&bad).is_none());
    // a subnet mask of three bytes stays raw; an option running past the end stops the walk
    let mut m = Dhcp::new(dhcp::OP_REQUEST, 9, CLIENT).to_bytes(0);
    m.pop();
    m.extend_from_slice(&[53, 1, 3, 1, 3, 255, 255, 0, 12, 40, b'x']);
    match Dhcp::parse(&m) {
        Some(d) => t.eq("odd options", d.opts, vec![Opt::MsgType(MsgType::Request), Opt::Other(1, vec![255, 255, 0])]),
        None => t.check("odd options parse", false),
    }
    // the end option turned into padding
    let mut no_end = discover_frame();
    no_end[306] = 0;
    let ls = decode::layers(&no_end);
    t.check("missing end found", ls.iter().any(|l| l.proto == Proto::Dhcp && l.problems.iter().any(|p| p.contains("without end"))));
    t.eq("unknown message type", Opt::decode(53, &[9]), Opt::Other(53, vec![9]));
    t.eq("client id text", format!("{:?}", Opt::decode(61, &[1])), String::from("Other(61, [1])"));
}
