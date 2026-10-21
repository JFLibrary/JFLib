// ntpt -- checks of netpkt::ntp and netpkt::syslog: NTP timestamps (the 1900 epoch offset
// 0x83aa7e80, halves of a second, round trips to the nanosecond), calendar dates, packets,
// the on-wire offset and delay, syslog priorities and the IOS and BSD message forms, and the
// dissector for both

use alloc::string::String;
use alloc::vec::Vec;

use netpkt::decode::{self, Proto};
use netpkt::eth::{EthHdr, ET_IPV4};
use netpkt::ipv4::{Ipv4Hdr, PROTO_UDP};
use netpkt::ntp::{self, Ntp, NtpView, Ts};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::syslog;
use netpkt::udp::UdpHdr;
use netpkt::{Ipv4, Mac};

use crate::T;

/// 2026-01-01 00:00:00 UTC
const Y2026: i64 = 1_767_225_600;

fn frame(sport: u16, dport: u16, payload: &[u8]) -> Vec<u8> {
    let (a, b) = (Ipv4([10, 0, 0, 1]), Ipv4([10, 0, 0, 2]));
    let mut p = Pkt::new(HEADROOM, payload);
    UdpHdr::new(sport, dport).push_v4(&mut p, a, b);
    Ipv4Hdr::new(a, b, PROTO_UDP, 64).push_onto(&mut p);
    EthHdr { dst: Mac([2, 0, 0, 0, 0, 2]), src: Mac([2, 0, 0, 0, 0, 1]), ethertype: ET_IPV4 }.push_onto(&mut p);
    p.into_vec()
}

pub fn run(t: &mut T) {
    timestamps(t);
    packets(t);
    syslog_msgs(t);
}

fn timestamps(t: &mut T) {
    t.eq("unix epoch", Ts::from_unix_ns(0), Ts(0x83AA_7E80u64 << 32));
    t.eq("half a second", Ts::from_unix_ns(1_500_000_000), Ts((0x83AA_7E81u64 << 32) | 0x8000_0000));
    for ns in [0i64, 1, 123_456_789, 999_999_999, Y2026 * 1_000_000_000 + 987_654_321, -1, -1_000_000_001] {
        t.eq(&alloc::format!("round trip {}", ns), Ts::from_unix_ns(ns).to_unix_ns(), ns);
    }
    t.eq("seconds field", Ts::from_unix_ns(Y2026 * 1_000_000_000).secs(), (Y2026 + 2_208_988_800) as u32);
    t.eq("civil 1970", ntp::civil(0), (1970, 1, 1, 0, 0, 0));
    t.eq("civil leap day", ntp::civil(951_782_400), (2000, 2, 29, 0, 0, 0));
    t.eq("civil 2026", ntp::civil(Y2026 + 291 * 86_400 + 12 * 3600 + 5), (2026, 10, 19, 12, 0, 5));
    t.eq("civil before 1970", ntp::civil(-1), (1969, 12, 31, 23, 59, 59));
    // client behind by 5000 ns, 400 ns each way, the server takes 100 ns
    t.eq("offset and delay", ntp::offset_delay(0, 5400, 5500, 900), (5000, 800));
}

fn packets(t: &mut T) {
    let at = Y2026 * 1_000_000_000;
    let c = Ntp::client(4, Ts::from_unix_ns(at));
    let cb = c.to_bytes();
    t.eq("client packet", (cb.len(), cb[0], &cb[40..44]), (48, 0x23, &((Y2026 + 2_208_988_800) as u32).to_be_bytes()[..]));
    let s = Ntp {
        li: ntp::LI_NONE,
        vn: 4,
        mode: ntp::MODE_SERVER,
        stratum: 1,
        poll: 6,
        precision: -20,
        root_delay: 0x0000_8000,
        root_disp: 0x0001_0000,
        refid: *b"GPS\0",
        reference: Ts::from_unix_ns(at - 16_000_000_000),
        origin: c.transmit,
        receive: Ts::from_unix_ns(at + 400),
        transmit: Ts::from_unix_ns(at + 500),
    };
    let sb = s.to_bytes();
    match NtpView::parse(&sb) {
        Some(v) => {
            t.eq("server fields", (v.li(), v.vn(), v.mode(), v.stratum(), v.poll(), v.precision()), (0, 4, 4, 1, 6, -20));
            t.eq("server record", Ntp::read(&v), s);
        }
        None => t.check("server parses", false),
    }
    t.check("short packet", NtpView::parse(&sb[..47]).is_none());
    let f = frame(123, 123, &sb);
    t.eq("ntp info", decode::info(&f), (Proto::Ntp, String::from("NTP Version 4, server")));
    let ls = decode::layers(&f);
    let field = |name: &str| ls.iter().find(|l| l.proto == Proto::Ntp).and_then(|l| l.fields.iter().find(|x| x.name == name)).map(|x| x.value.clone());
    t.eq("refid", field("Reference ID"), Some(String::from("GPS")));
    t.eq("root delay", field("Root Delay"), Some(String::from("0.500000 s")));
    t.eq("transmit", field("Transmit Timestamp"), Some(String::from("2026-01-01 00:00:00.000000500 UTC")));
    t.eq("origin zero", field("Origin Timestamp").map(|_| ()), Some(()));
    t.eq("zero timestamp", decode::layers(&frame(123, 123, &cb)).iter().find(|l| l.proto == Proto::Ntp).and_then(|l| l.fields.iter().find(|x| x.name == "Origin Timestamp")).map(|x| x.value.clone()), Some(String::from("(0)")));
    let mut s2 = s;
    s2.stratum = 2;
    s2.refid = [10, 0, 2, 53];
    let f2 = frame(123, 123, &s2.to_bytes());
    t.eq("refid address", decode::layers(&f2).iter().find(|l| l.proto == Proto::Ntp).and_then(|l| l.fields.iter().find(|x| x.name == "Reference ID")).map(|x| x.value.clone()), Some(String::from("10.0.2.53")));
}

fn syslog_msgs(t: &mut T) {
    let p = syslog::pri(syslog::FAC_LOCAL7, syslog::SEV_NOTICE);
    t.eq("pri", (p, syslog::facility_of(p), syslog::severity_of(p)), (189, 23, 5));
    t.eq("names", (syslog::facility_name(23), syslog::severity_name(5), syslog::facility_name(3), syslog::severity_name(3)), ("local7", "notice", "daemon", "err"));
    let ios = syslog::ios(p, 12, "*Oct 19 12:00:05.123", "%LINK-3-UPDOWN: Interface GigabitEthernet0/0/0, changed state to up");
    t.eq("ios form", String::from_utf8_lossy(&ios).into_owned(), String::from("<189>12: *Oct 19 12:00:05.123: %LINK-3-UPDOWN: Interface GigabitEthernet0/0/0, changed state to up"));
    let ts = syslog::bsd_time(10, 9, 8, 0, 5);
    t.eq("bsd time", ts.as_str(), "Oct  9 08:00:05");
    let bsd = syslog::bsd(syslog::pri(syslog::FAC_DAEMON, syslog::SEV_INFO), &ts, "pc1", "dhclient", "bound to 10.0.1.10");
    t.eq("bsd form", String::from_utf8_lossy(&bsd).into_owned(), String::from("<30>Oct  9 08:00:05 pc1 dhclient: bound to 10.0.1.10"));
    match syslog::parse(&bsd) {
        Some(m) => t.eq("bsd parsed", (m.pri, m.time, m.host, m.tag, m.text), (30, Some(ts.clone()), Some(String::from("pc1")), Some(String::from("dhclient")), String::from("bound to 10.0.1.10"))),
        None => t.check("bsd parses", false),
    }
    match syslog::parse(&ios) {
        Some(m) => t.eq("ios parsed", (m.pri, m.time, m.rest.starts_with("12: *Oct"), m.text == m.rest), (189, None, true, true)),
        None => t.check("ios parses", false),
    }
    t.check("refused", syslog::parse(b"<192>x").is_none() && syslog::parse(b"hello").is_none() && syslog::parse(b"<1x>a").is_none());
    t.eq("empty rest", syslog::parse(b"<13>\n").map(|m| (m.pri, m.rest)), Some((13, String::new())));
    let f = frame(51000, 514, &ios);
    t.eq("syslog info", decode::info(&f), (Proto::Syslog, String::from("LOCAL7.NOTICE: 12: *Oct 19 12:00:05.123: %LINK-3-UPDOWN: Interface GigabitEthernet0/0/0, changed state to up")));
    let ls = decode::layers(&f);
    let lv: Vec<(&str, String)> = ls.iter().filter(|l| l.proto == Proto::Syslog).flat_map(|l| l.fields.iter().map(|x| (x.name, x.value.clone()))).collect();
    t.eq("syslog fields", lv.iter().take(2).cloned().collect::<Vec<_>>(), alloc::vec![("Facility", String::from("local7 (23)")), ("Level", String::from("notice (5)"))]);
}
