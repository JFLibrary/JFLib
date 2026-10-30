// ftpt -- checks of netpkt::ftp and its dissector: command lines, replies on one and several
// lines (RFC 959 4.2), the reader of a server's bytes in pieces, the six numbers of PORT and of a
// 227 reply wherever they stand, EPRT and the 229 reply (RFC 2428), and segments of a control
// connection and of an active data connection with their layers and summaries.

use alloc::string::String;
use alloc::vec::Vec;

use netpkt::decode::{self, Proto};
use netpkt::eth::{EthHdr, ET_IPV4};
use netpkt::ftp::{self, Command, Lines, Reply, ReplyReader};
use netpkt::ipv4::{Ipv4Hdr, PROTO_TCP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::tcp::{TcpHdr, TcpOpts, ACK, PSH};
use netpkt::{IpAddr, Ipv4, Mac};

use crate::T;

fn s(x: &str) -> String {
    String::from(x)
}

/// a segment with data from 10.0.0.5:sport to 10.0.0.9:dport
fn segment(sport: u16, dport: u16, data: &[u8]) -> Vec<u8> {
    let (src, dst) = (Ipv4([10, 0, 0, 5]), Ipv4([10, 0, 0, 9]));
    let mut p = Pkt::new(HEADROOM, data);
    TcpHdr { src_port: sport, dst_port: dport, seq: 1000, ack: 2000, flags: ACK | PSH, window: 64240, urgent: 0, opts: TcpOpts::default() }.push_v4(&mut p, src, dst);
    Ipv4Hdr::new(src, dst, PROTO_TCP, 64).push_onto(&mut p);
    EthHdr { dst: Mac([2, 0, 0, 0, 0, 9]), src: Mac([2, 0, 0, 0, 0, 5]), ethertype: ET_IPV4 }.push_onto(&mut p);
    let mut f = p.into_vec();
    netpkt::eth::pad(&mut f);
    f
}

pub fn run(t: &mut T) {
    commands(t);
    replies(t);
    addresses(t);
    segments(t);
}

fn commands(t: &mut T) {
    t.eq("parse", Command::parse("user anna\r\n"), Some(Command { verb: s("USER"), arg: s("anna") }));
    t.eq("an argument with spaces", Command::parse("STOR my file.txt").map(|c| c.arg), Some(s("my file.txt")));
    t.eq("no argument", Command::parse("PASV").map(|c| (c.verb, c.arg)), Some((s("PASV"), String::new())));
    t.eq("empty", Command::parse("\r\n"), None);
    t.eq("line", Command::new("retr", "r1-config").to_line(), s("RETR r1-config\r\n"));
    t.eq("line without argument", Command::new("QUIT", "").to_line(), s("QUIT\r\n"));
    t.check("known verbs", ftp::verb_known("epsv") && ftp::verb_known("XPWD") && ftp::verb_known("XRMD") && !ftp::verb_known("HELO"));
    let mut l = Lines::new();
    t.eq("lines in pieces", l.push(b"USER an"), Vec::<String>::new());
    t.eq("lines complete", l.push(b"na\r\nPASS x\nNO"), alloc::vec![s("USER anna"), s("PASS x")]);
    t.eq("waiting", l.pending(), 2);
}

fn replies(t: &mut T) {
    t.eq("one line", Reply::new(220, "Ready.").to_bytes(), b"220 Ready.\r\n".to_vec());
    let feat = Reply { code: 211, lines: alloc::vec![s("Extensions:"), s(" EPSV"), s(" SIZE"), s("End")] };
    t.eq("several lines", feat.to_bytes(), b"211-Extensions:\r\n EPSV\r\n SIZE\r\n211 End\r\n".to_vec());
    let tricky = Reply { code: 250, lines: alloc::vec![s("Listing:"), s("250 is in the text"), s("Done.")] };
    t.eq("an inner line with digits is indented", tricky.to_bytes(), b"250-Listing:\r\n 250 is in the text\r\n250 Done.\r\n".to_vec());
    let mut r = ReplyReader::new();
    t.eq("a reply in pieces", r.push(b"220 Rea"), Vec::<Reply>::new());
    t.eq("the reply", r.push(b"dy.\r\n331 Pass"), alloc::vec![Reply::new(220, "Ready.")]);
    let got = r.push(b"word?\r\n211-Extensions:\r\n EPSV\r\n211-not the end\r\n211 End\r\n");
    t.eq("two replies", got.iter().map(|x| (x.code, x.lines.len())).collect::<Vec<_>>(), alloc::vec![(331, 1), (211, 4)]);
    t.eq("the multi-line text", got.get(1).map(|x| x.lines.clone()), Some(alloc::vec![s("Extensions:"), s("EPSV"), s("211-not the end"), s("End")]));
    t.check("classes", Reply::new(150, "").is_preliminary() && Reply::new(226, "").is_ok() && Reply::new(331, "").is_intermediate() && !Reply::new(530, "").is_ok());
}

fn addresses(t: &mut T) {
    t.eq("PORT argument", ftp::host_port("10,0,0,5,156,64"), Some((Ipv4([10, 0, 0, 5]), 40000)));
    t.eq("227 text", ftp::host_port("Passive mode on (10,0,0,9,117,49)."), Some((Ipv4([10, 0, 0, 9]), 30001)));
    t.eq("227 without parentheses", ftp::host_port("=10,0,0,9,117,49"), Some((Ipv4([10, 0, 0, 9]), 30001)));
    t.eq("a number too large", ftp::host_port("(300,0,0,9,117,49)"), None);
    t.eq("five numbers", ftp::host_port("(10,0,0,9,117)"), None);
    t.eq("format", ftp::fmt_host_port(Ipv4([10, 0, 0, 9]), 30001), s("10,0,0,9,117,49"));
    t.eq("EPRT v4", ftp::eprt("|1|10.0.0.5|40000|"), Some((IpAddr::V4(Ipv4([10, 0, 0, 5])), 40000)));
    t.eq("EPRT v6", ftp::eprt("|2|2001:db8::5|40000|").map(|x| x.1), Some(40000));
    t.eq("EPRT wrong family", ftp::eprt("|2|10.0.0.5|40000|"), None);
    t.eq("EPRT other delimiter", ftp::eprt("!1!10.0.0.5!21!").map(|x| x.1), Some(21));
    t.eq("EPRT format", ftp::fmt_eprt(IpAddr::V4(Ipv4([10, 0, 0, 5])), 40000), s("|1|10.0.0.5|40000|"));
    t.eq("229 port", ftp::epsv_port("Extended passive mode on (|||30001|)."), Some(30001));
    t.eq("229 format", ftp::fmt_epsv(30001), s("(|||30001|)"));
    t.eq("229 broken", ftp::epsv_port("(||30001|)"), None);
}

fn segments(t: &mut T) {
    let req = segment(40000, ftp::PORT_FTP, b"USER anna\r\n");
    t.eq("request mask", decode::proto_mask(&req) & (decode::P_FTP | decode::P_TCP), decode::P_FTP | decode::P_TCP);
    t.eq("request summary", decode::info(&req), (Proto::Ftp, s("Request: USER anna")));
    let ls = decode::layers(&req);
    let names: Vec<(&str, String)> = ls.iter().find(|l| l.proto == Proto::Ftp).map(|l| l.fields.iter().map(|f| (f.name, f.value.clone())).collect()).unwrap_or_default();
    t.eq("request fields", names, alloc::vec![("Request", s("USER anna")), ("Request command", s("USER")), ("Request arg", s("anna"))]);
    let off = ls.iter().find(|l| l.proto == Proto::Ftp).and_then(|l| l.fields.get(2)).map(|f| (f.off, f.len));
    // Ethernet 14 + IPv4 20 + TCP 20, then "USER "
    t.eq("argument range", off, Some((59, 4)));
    let rep = segment(ftp::PORT_FTP, 40000, b"230-Welcome\r\n230 Logged in.\r\n");
    t.eq("response summary", decode::info(&rep), (Proto::Ftp, s("Response: 230-Welcome")));
    let lr = decode::layers(&rep);
    t.eq("response lines", lr.iter().find(|l| l.proto == Proto::Ftp).map(|l| l.fields.iter().filter(|f| f.name == "Response code").count()), Some(2));
    let bad = segment(40000, ftp::PORT_FTP, b"HELO x\r\n");
    t.eq("unknown command", decode::layers(&bad).iter().find(|l| l.proto == Proto::Ftp).map(|l| l.problems.clone()), Some(alloc::vec![s("a command FTP does not define: HELO")]));
    let data = segment(ftp::PORT_FTP_DATA, 40001, b"hostname R1\n");
    t.eq("active data", decode::info(&data), (Proto::FtpData, s("FTP Data: 12 bytes")));
    t.eq("active data mask", decode::proto_mask(&data) & decode::P_FTP, decode::P_FTP);
    t.eq("names", (Proto::Ftp.name(), Proto::FtpData.name()), ("FTP", "FTP-DATA"));
}
