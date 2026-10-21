// tcp -- the TCP header and its options (RFC 9293, 7323, 2018)
//
//   TcpView::parse(b)     ports, sequence and acknowledgement numbers, flags, window, options,
//                         payload; checksum_ok_v4 / _v6
//   TcpOpts               MSS, window scale, SACK permitted, SACK blocks, timestamps
//   TcpHdr { .. }         push_v4 / push_v6 (&mut Pkt) with the checksum; the options are
//                         written in the usual order and padded with NOPs / EOL
//   flags_str(flags)      "SYN, ACK"
//   seq_lt / seq_le / seq_gt / seq_ge   comparisons in sequence space (modulo 2^32)
//   port_name(port)       well-known names ("http", "ssh", ...)

use alloc::string::String;

use crate::addr::{Ipv4, Ipv6};
use crate::checksum;
use crate::ipv4::PROTO_TCP;
use crate::pkt::Pkt;
use crate::{be16, be32, put16, put32};

pub const TCP_MIN: usize = 20;
pub const TCP_MAX: usize = 60;

pub const FIN: u8 = 0x01;
pub const SYN: u8 = 0x02;
pub const RST: u8 = 0x04;
pub const PSH: u8 = 0x08;
pub const ACK: u8 = 0x10;
pub const URG: u8 = 0x20;
pub const ECE: u8 = 0x40;
pub const CWR: u8 = 0x80;

pub const OPT_EOL: u8 = 0;
pub const OPT_NOP: u8 = 1;
pub const OPT_MSS: u8 = 2;
pub const OPT_WSCALE: u8 = 3;
pub const OPT_SACK_OK: u8 = 4;
pub const OPT_SACK: u8 = 5;
pub const OPT_TS: u8 = 8;

/// a < b in sequence space
pub fn seq_lt(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}
pub fn seq_le(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) <= 0
}
pub fn seq_gt(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) > 0
}
pub fn seq_ge(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) >= 0
}

/// the TCP options this library understands
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TcpOpts {
    pub mss: Option<u16>,
    pub wscale: Option<u8>,
    pub sack_ok: bool,
    /// SACK blocks (left, right edge); `sack_n` of them are valid
    pub sack: [(u32, u32); 4],
    pub sack_n: u8,
    /// timestamp value and echo reply
    pub ts: Option<(u32, u32)>,
}

impl TcpOpts {
    /// parses an option block; unknown options are skipped, a malformed one ends the walk
    pub fn parse(b: &[u8]) -> TcpOpts {
        let mut o = TcpOpts::default();
        let mut i = 0;
        while i < b.len() {
            let k = b[i];
            if k == OPT_EOL {
                break;
            }
            if k == OPT_NOP {
                i += 1;
                continue;
            }
            if i + 1 >= b.len() {
                break;
            }
            let l = b[i + 1] as usize;
            if l < 2 || i + l > b.len() {
                break;
            }
            let d = &b[i + 2..i + l];
            match (k, l) {
                (OPT_MSS, 4) => o.mss = Some(be16(d, 0)),
                (OPT_WSCALE, 3) => o.wscale = Some(d[0].min(14)),
                (OPT_SACK_OK, 2) => o.sack_ok = true,
                (OPT_SACK, _) if (l - 2) % 8 == 0 => {
                    for blk in d.chunks_exact(8).take(4) {
                        o.sack[o.sack_n as usize] = (be32(blk, 0), be32(blk, 4));
                        o.sack_n += 1;
                    }
                }
                (OPT_TS, 10) => o.ts = Some((be32(d, 0), be32(d, 4))),
                _ => {}
            }
            i += l;
        }
        o
    }
    /// the encoded length, padded to a multiple of 4
    pub fn len(&self) -> usize {
        let mut n = 0;
        if self.mss.is_some() {
            n += 4;
        }
        if self.sack_ok {
            n += 2;
        }
        if self.ts.is_some() {
            n += 10;
        }
        if self.wscale.is_some() {
            n += 3;
        }
        if self.sack_n > 0 {
            n += 2 + 8 * self.sack_n as usize;
        }
        (n + 3) & !3
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// writes the options (MSS, SACK-permitted, timestamps, window scale, SACK), NOP padding
    pub fn write(&self, out: &mut [u8]) {
        let mut i = 0;
        if let Some(m) = self.mss {
            out[i] = OPT_MSS;
            out[i + 1] = 4;
            put16(out, i + 2, m);
            i += 4;
        }
        if self.sack_ok {
            out[i] = OPT_SACK_OK;
            out[i + 1] = 2;
            i += 2;
        }
        if let Some((v, e)) = self.ts {
            out[i] = OPT_TS;
            out[i + 1] = 10;
            put32(out, i + 2, v);
            put32(out, i + 6, e);
            i += 10;
        }
        if let Some(w) = self.wscale {
            out[i] = OPT_WSCALE;
            out[i + 1] = 3;
            out[i + 2] = w;
            i += 3;
        }
        if self.sack_n > 0 {
            out[i] = OPT_SACK;
            out[i + 1] = 2 + 8 * self.sack_n;
            i += 2;
            for &(l, r) in &self.sack[..self.sack_n as usize] {
                put32(out, i, l);
                put32(out, i + 4, r);
                i += 8;
            }
        }
        let end = self.len();
        while i < end {
            out[i] = if i + 1 == end { OPT_EOL } else { OPT_NOP };
            i += 1;
        }
    }
}

/// a received TCP segment
#[derive(Clone, Copy, Debug)]
pub struct TcpView<'a> {
    b: &'a [u8],
}

impl<'a> TcpView<'a> {
    pub fn parse(b: &'a [u8]) -> Option<TcpView<'a>> {
        if b.len() < TCP_MIN {
            return None;
        }
        let off = (b[12] >> 4) as usize * 4;
        if off < TCP_MIN || off > b.len() {
            return None;
        }
        Some(TcpView { b })
    }
    pub fn src_port(&self) -> u16 {
        be16(self.b, 0)
    }
    pub fn dst_port(&self) -> u16 {
        be16(self.b, 2)
    }
    pub fn seq(&self) -> u32 {
        be32(self.b, 4)
    }
    pub fn ack(&self) -> u32 {
        be32(self.b, 8)
    }
    pub fn header_len(&self) -> usize {
        (self.b[12] >> 4) as usize * 4
    }
    pub fn flags(&self) -> u8 {
        self.b[13]
    }
    pub fn has(&self, f: u8) -> bool {
        self.b[13] & f == f
    }
    pub fn window(&self) -> u16 {
        be16(self.b, 14)
    }
    pub fn checksum(&self) -> u16 {
        be16(self.b, 16)
    }
    pub fn urgent(&self) -> u16 {
        be16(self.b, 18)
    }
    pub fn options_raw(&self) -> &'a [u8] {
        &self.b[TCP_MIN..self.header_len()]
    }
    pub fn opts(&self) -> TcpOpts {
        TcpOpts::parse(self.options_raw())
    }
    pub fn payload(&self) -> &'a [u8] {
        &self.b[self.header_len()..]
    }
    pub fn bytes(&self) -> &'a [u8] {
        self.b
    }
    /// the sequence space the segment occupies: payload length + SYN + FIN
    pub fn seg_len(&self) -> u32 {
        self.payload().len() as u32 + self.has(SYN) as u32 + self.has(FIN) as u32
    }
    pub fn checksum_ok_v4(&self, src: Ipv4, dst: Ipv4) -> bool {
        checksum::fold(checksum::sum(checksum::pseudo_v4(src, dst, PROTO_TCP, self.b.len() as u16), self.b)) == 0xFFFF
    }
    pub fn checksum_ok_v6(&self, src: Ipv6, dst: Ipv6) -> bool {
        checksum::fold(checksum::sum(checksum::pseudo_v6(src, dst, PROTO_TCP, self.b.len() as u32), self.b)) == 0xFFFF
    }
}

/// a TCP header to write
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TcpHdr {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq: u32,
    pub ack: u32,
    pub flags: u8,
    pub window: u16,
    pub urgent: u16,
    pub opts: TcpOpts,
}

impl TcpHdr {
    pub fn read(v: &TcpView) -> TcpHdr {
        TcpHdr {
            src_port: v.src_port(),
            dst_port: v.dst_port(),
            seq: v.seq(),
            ack: v.ack(),
            flags: v.flags(),
            window: v.window(),
            urgent: v.urgent(),
            opts: v.opts(),
        }
    }
    pub fn len(&self) -> usize {
        TCP_MIN + self.opts.len()
    }
    fn push(&self, p: &mut Pkt) {
        let hl = self.len();
        let h = p.push(hl);
        put16(h, 0, self.src_port);
        put16(h, 2, self.dst_port);
        put32(h, 4, self.seq);
        put32(h, 8, self.ack);
        h[12] = ((hl / 4) as u8) << 4;
        h[13] = self.flags;
        put16(h, 14, self.window);
        put16(h, 18, self.urgent);
        self.opts.write(&mut h[TCP_MIN..hl]);
    }
    /// prepends the header (the packet's content is the payload) with the IPv4 checksum
    pub fn push_v4(&self, p: &mut Pkt, src: Ipv4, dst: Ipv4) {
        self.push(p);
        let ck = checksum::finish(checksum::sum(checksum::pseudo_v4(src, dst, PROTO_TCP, p.len() as u16), p.bytes()));
        put16(p.bytes_mut(), 16, ck);
    }
    /// the same with the IPv6 pseudo header
    pub fn push_v6(&self, p: &mut Pkt, src: Ipv6, dst: Ipv6) {
        self.push(p);
        let ck = checksum::finish(checksum::sum(checksum::pseudo_v6(src, dst, PROTO_TCP, p.len() as u32), p.bytes()));
        put16(p.bytes_mut(), 16, ck);
    }
}

/// "SYN, ACK" (Wireshark order: FIN SYN RST PSH ACK URG ECE CWR)
pub fn flags_str(f: u8) -> String {
    let mut s = String::new();
    for (bit, name) in [(FIN, "FIN"), (SYN, "SYN"), (RST, "RST"), (PSH, "PSH"), (ACK, "ACK"), (URG, "URG"), (ECE, "ECE"), (CWR, "CWR")] {
        if f & bit != 0 {
            if !s.is_empty() {
                s.push_str(", ");
            }
            s.push_str(name);
        }
    }
    s
}

/// a short name of a well-known TCP port
pub fn port_name(port: u16) -> Option<&'static str> {
    Some(match port {
        20 => "ftp-data",
        21 => "ftp",
        22 => "ssh",
        23 => "telnet",
        25 => "smtp",
        53 => "dns",
        80 => "http",
        110 => "pop3",
        143 => "imap",
        179 => "bgp",
        443 => "https",
        587 => "submission",
        3306 => "mysql",
        5432 => "postgresql",
        6379 => "redis",
        8080 => "http-alt",
        _ => return None,
    })
}
