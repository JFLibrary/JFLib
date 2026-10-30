// tftpt -- checks of netpkt::tftp and its dissector: every packet kind byte for byte (RFC
// 1350), options in requests and OACKs (RFC 2347, 2348, 2349), the error codes, what parse
// refuses, netascii, the shape of a transfer's packets away from port 69, and whole frames with
// their layers and summaries.  The expected bytes follow the RFC's figures.

use alloc::string::String;
use alloc::vec::Vec;

use netpkt::decode::{self, Proto};
use netpkt::eth::{EthHdr, ET_IPV4};
use netpkt::ipv4::{Ipv4Hdr, PROTO_UDP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::tftp::{self, Bad, Packet, TftpView};
use netpkt::udp::{UdpHdr, PORT_TFTP};
use netpkt::{Ipv4, Mac};

use crate::T;

fn s(x: &str) -> String {
    String::from(x)
}

/// a TFTP packet in a frame from 10.0.0.5:sport to 10.0.0.9:dport
fn frame(sport: u16, dport: u16, msg: &[u8]) -> Vec<u8> {
    let (src, dst) = (Ipv4([10, 0, 0, 5]), Ipv4([10, 0, 0, 9]));
    let mut p = Pkt::new(HEADROOM, msg);
    UdpHdr::new(sport, dport).push_v4(&mut p, src, dst);
    Ipv4Hdr::new(src, dst, PROTO_UDP, 64).push_onto(&mut p);
    EthHdr { dst: Mac([2, 0, 0, 0, 0, 9]), src: Mac([2, 0, 0, 0, 0, 5]), ethertype: ET_IPV4 }.push_onto(&mut p);
    let mut f = p.into_vec();
    netpkt::eth::pad(&mut f);
    f
}

pub fn run(t: &mut T) {
    packets(t);
    options(t);
    refused(t);
    shapes(t);
    frames(t);
}

fn packets(t: &mut T) {
    let rrq = Packet::Rrq { file: s("r1-config"), mode: s("octet"), opts: Vec::new() };
    t.eq("RRQ bytes", rrq.to_bytes(), b"\x00\x01r1-config\x00octet\x00".to_vec());
    let wrq = Packet::Wrq { file: s("a"), mode: s("netascii"), opts: Vec::new() };
    t.eq("WRQ bytes", wrq.to_bytes(), b"\x00\x02a\x00netascii\x00".to_vec());
    let data = Packet::Data { block: 258, data: b"hostname R1\n".to_vec() };
    t.eq("DATA bytes", data.to_bytes(), b"\x00\x03\x01\x02hostname R1\n".to_vec());
    t.eq("ACK bytes", Packet::Ack { block: 65535 }.to_bytes(), alloc::vec![0, 4, 0xFF, 0xFF]);
    let err = Packet::error_of(tftp::ERR_NOT_FOUND);
    t.eq("ERROR bytes", err.to_bytes(), b"\x00\x05\x00\x01File not found\x00".to_vec());
    for p in [&rrq, &wrq, &data, &err, &Packet::Ack { block: 7 }, &Packet::Data { block: 1, data: Vec::new() }] {
        t.eq(&alloc::format!("round trip of opcode {}", p.opcode()), Packet::parse(&p.to_bytes()), Ok(p.clone()));
    }
    match TftpView::parse(&data.to_bytes()) {
        Ok(v) => t.eq("DATA view", (v.opcode(), v.block(), v.data()), (tftp::OP_DATA, 258, &b"hostname R1\n"[..])),
        Err(_) => t.check("DATA view", false),
    }
    match TftpView::parse(&err.to_bytes()) {
        Ok(v) => t.eq("ERROR view", (v.error_code(), v.error_msg()), (1, s("File not found"))),
        Err(_) => t.check("ERROR view", false),
    }
    t.eq("op names", [tftp::op_name(1), tftp::op_name(4), tftp::op_name(6), tftp::op_name(9)], ["Read Request", "Acknowledgement", "Option Acknowledgement", "Unknown"]);
    t.eq("error names", [tftp::error_name(0), tftp::error_name(3), tftp::error_name(5), tftp::error_name(8)], ["Not defined", "Disk full or allocation exceeded", "Unknown transfer ID", "Option negotiation failed"]);
    t.check("modes", tftp::mode_known("OCTET") && tftp::mode_known("netascii") && tftp::mode_known("mail") && !tftp::mode_known("binary"));
    t.eq("netascii out", tftp::to_netascii(b"a\nb\rc"), b"a\r\nb\r\0c".to_vec());
    t.eq("netascii in", tftp::from_netascii(b"a\r\nb\r\0c\r"), b"a\nb\rc\r".to_vec());
}

fn options(t: &mut T) {
    let rrq = Packet::Rrq { file: s("big.bin"), mode: s("octet"), opts: alloc::vec![(s("blksize"), s("1428")), (s("tsize"), s("0"))] };
    let b = rrq.to_bytes();
    t.eq("RRQ with options", b.clone(), b"\x00\x01big.bin\x00octet\x00blksize\x001428\x00tsize\x000\x00".to_vec());
    let back = Packet::parse(&b);
    t.eq("options read back", back.as_ref().ok().and_then(|p| p.opt("BLKSIZE")), Some("1428"));
    t.eq("tsize read back", back.as_ref().ok().and_then(|p| p.opt("tsize")), Some("0"));
    let oack = Packet::Oack { opts: alloc::vec![(s("blksize"), s("1428")), (s("tsize"), s("70000"))] };
    t.eq("OACK bytes", oack.to_bytes(), b"\x00\x06blksize\x001428\x00tsize\x0070000\x00".to_vec());
    t.eq("OACK round trip", Packet::parse(&oack.to_bytes()), Ok(oack.clone()));
    t.eq("no option of a DATA", Packet::Data { block: 1, data: Vec::new() }.opt("blksize"), None);
    match TftpView::parse(&b) {
        Ok(v) => t.eq("field offsets", v.fields(), alloc::vec![(2, 7), (10, 5), (16, 7), (24, 4), (29, 5), (35, 1)]),
        Err(_) => t.check("field offsets", false),
    }
}

fn refused(t: &mut T) {
    t.eq("too short", Packet::parse(&[0]).err(), Some(Bad::Short));
    t.eq("ack too short", Packet::parse(&[0, 4, 1]).err(), Some(Bad::Short));
    t.eq("unknown opcode", Packet::parse(&[0, 9, 0, 0]).err(), Some(Bad::Opcode(9)));
    t.eq("file without NUL", Packet::parse(b"\x00\x01file").err(), Some(Bad::Unterminated));
    t.eq("no mode", Packet::parse(b"\x00\x01file\x00").err(), Some(Bad::Unterminated));
    t.eq("option without value", Packet::parse(b"\x00\x01f\x00octet\x00blksize\x00").err(), Some(Bad::OddOptions));
    t.eq("error without NUL", Packet::parse(b"\x00\x05\x00\x01oops").err(), Some(Bad::Unterminated));
}

fn shapes(t: &mut T) {
    t.check("data block", tftp::looks_like(&Packet::Data { block: 1, data: alloc::vec![7; 512] }.to_bytes()));
    t.check("ack", tftp::looks_like(&[0, 4, 0, 1]));
    t.check("ack too long", !tftp::looks_like(&[0, 4, 0, 1, 0]));
    t.check("error", tftp::looks_like(&Packet::error_of(2).to_bytes()));
    t.check("error code too high", !tftp::looks_like(b"\x00\x05\x00\x09x\x00"));
    t.check("oack", tftp::looks_like(b"\x00\x06blksize\x001428\x00"));
    t.check("oack with a text value", !tftp::looks_like(b"\x00\x06blksize\x00big\x00"));
    t.check("a request is no transfer packet", !tftp::looks_like(b"\x00\x01f\x00octet\x00"));
    t.check("text", !tftp::looks_like(b"hello world"));
}

fn frames(t: &mut T) {
    let rrq = Packet::Rrq { file: s("r1-config"), mode: s("octet"), opts: alloc::vec![(s("blksize"), s("1428"))] };
    let f = frame(40000, PORT_TFTP, &rrq.to_bytes());
    t.eq("request mask", decode::proto_mask(&f) & (decode::P_TFTP | decode::P_UDP), decode::P_TFTP | decode::P_UDP);
    t.eq("request summary", decode::info(&f), (Proto::Tftp, s("Read Request, File: r1-config, Transfer type: octet, blksize=1428")));
    let ls = decode::layers(&f);
    let l = ls.iter().find(|l| l.proto == Proto::Tftp);
    t.eq("request title", l.map(|l| l.title.clone()), Some(s("Trivial File Transfer Protocol, Read Request")));
    let names: Vec<(&str, String)> = l.map(|l| l.fields.iter().map(|f| (f.name, f.value.clone())).collect()).unwrap_or_default();
    t.eq("request fields", names, alloc::vec![("Opcode", s("Read Request (1)")), ("Source File", s("r1-config")), ("Type", s("octet")), ("Option", s("blksize = 1428"))]);
    // the field of the file covers its bytes and the NUL (Ethernet 14 + IPv4 20 + UDP 8 + 2)
    t.eq("file range", l.and_then(|l| l.fields.get(1)).map(|f| (f.off, f.len)), Some((44, 10)));
    // a transfer between two free ports: known by its shape
    let d = frame(51234, 40000, &Packet::Data { block: 3, data: alloc::vec![b'x'; 100] }.to_bytes());
    t.eq("data summary", decode::info(&d), (Proto::Tftp, s("Data Packet, Block: 3 (last)")));
    let a = frame(40000, 51234, &Packet::Ack { block: 3 }.to_bytes());
    t.eq("ack summary", decode::info(&a), (Proto::Tftp, s("Acknowledgement, Block: 3")));
    let e = frame(51234, 40000, &Packet::Error { code: tftp::ERR_DISK_FULL, msg: s("quota") }.to_bytes());
    t.eq("error summary", decode::info(&e).1, s("Error Code, Code: Disk full or allocation exceeded, Message: quota"));
    let le = decode::layers(&e);
    t.eq("error fields", le.iter().find(|l| l.proto == Proto::Tftp).map(|l| l.fields.iter().map(|f| f.value.clone()).collect::<Vec<_>>()), Some(alloc::vec![s("Error Code (5)"), s("Disk full or allocation exceeded (3)"), s("quota")]));
    // the same shape below port 1024 is no TFTP
    let other = frame(500, 40000, &[0, 4, 0, 1]);
    t.eq("not below 1024", decode::info(&other).0, Proto::Data);
    // a bad request names its problem
    let bad = frame(40000, PORT_TFTP, b"\x00\x01f\x00binary\x00");
    let lb = decode::layers(&bad);
    t.eq("bad mode", lb.iter().find(|l| l.proto == Proto::Tftp).map(|l| l.problems.clone()), Some(alloc::vec![s("unknown transfer mode \"binary\"")]));
}
