// pcapng -- capture files in the pcapng format (IETF draft-ietf-opsawg-pcapng), as Wireshark
// reads them
//
//   Writer::new(app)                 section header block (little endian, "app" as shb_userappl)
//   add_interface(name, link, snap)  interface description block with nanosecond timestamps;
//                                    returns the interface id
//   packet(iface, t_ns, data)        enhanced packet block (with an optional comment)
//   finish() -> Vec<u8>              the file
//   read(bytes) -> Result<Capture>   section header, interfaces and enhanced / simple packet
//                                    blocks of either byte order; other blocks are skipped
//
// Timestamps are whatever the caller counts in nanoseconds (a simulator's virtual clock).
// Link types: LINKTYPE_ETHERNET, LINKTYPE_C_HDLC (serial lines), LINKTYPE_RAW (bare IP).

use alloc::string::String;
use alloc::vec::Vec;

pub const LINKTYPE_ETHERNET: u16 = 1;
/// bare IPv4 / IPv6 packets without a link header
pub const LINKTYPE_RAW: u16 = 101;
/// Cisco HDLC (address, control, an ethertype, the packet), as on serial lines
pub const LINKTYPE_C_HDLC: u16 = 104;

const BT_SHB: u32 = 0x0A0D_0D0A;
const BT_IDB: u32 = 0x0000_0001;
const BT_SPB: u32 = 0x0000_0003;
const BT_EPB: u32 = 0x0000_0006;
const MAGIC: u32 = 0x1A2B_3C4D;

const OPT_END: u16 = 0;
const OPT_COMMENT: u16 = 1;
const OPT_SHB_USERAPPL: u16 = 4;
const OPT_IF_NAME: u16 = 2;
const OPT_IF_TSRESOL: u16 = 9;

/// writes a pcapng file into memory
#[derive(Clone, Debug)]
pub struct Writer {
    buf: Vec<u8>,
    ifaces: u32,
}

fn opt(body: &mut Vec<u8>, code: u16, val: &[u8]) {
    body.extend_from_slice(&code.to_le_bytes());
    body.extend_from_slice(&(val.len() as u16).to_le_bytes());
    body.extend_from_slice(val);
    while body.len() % 4 != 0 {
        body.push(0);
    }
}

fn opt_end(body: &mut Vec<u8>) {
    body.extend_from_slice(&OPT_END.to_le_bytes());
    body.extend_from_slice(&0u16.to_le_bytes());
}

impl Writer {
    /// a new file; `app` names the program that wrote it
    pub fn new(app: &str) -> Writer {
        let mut w = Writer { buf: Vec::new(), ifaces: 0 };
        let mut body = Vec::new();
        body.extend_from_slice(&MAGIC.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&u64::MAX.to_le_bytes()); // section length unknown
        if !app.is_empty() {
            opt(&mut body, OPT_SHB_USERAPPL, app.as_bytes());
            opt_end(&mut body);
        }
        w.block(BT_SHB, &body);
        w
    }
    fn block(&mut self, typ: u32, body: &[u8]) {
        let len = (12 + body.len()) as u32;
        self.buf.extend_from_slice(&typ.to_le_bytes());
        self.buf.extend_from_slice(&len.to_le_bytes());
        self.buf.extend_from_slice(body);
        self.buf.extend_from_slice(&len.to_le_bytes());
    }
    /// an interface; `link` is a LINKTYPE_* value, `snaplen` 0 = unlimited
    pub fn add_interface(&mut self, name: &str, link: u16, snaplen: u32) -> u32 {
        let mut body = Vec::new();
        body.extend_from_slice(&link.to_le_bytes());
        body.extend_from_slice(&0u16.to_le_bytes());
        body.extend_from_slice(&snaplen.to_le_bytes());
        if !name.is_empty() {
            opt(&mut body, OPT_IF_NAME, name.as_bytes());
        }
        opt(&mut body, OPT_IF_TSRESOL, &[9]); // 10^-9 s
        opt_end(&mut body);
        self.block(BT_IDB, &body);
        self.ifaces += 1;
        self.ifaces - 1
    }
    /// a packet seen on interface `iface` at `t_ns` nanoseconds
    pub fn packet(&mut self, iface: u32, t_ns: u64, data: &[u8]) {
        self.packet_with_comment(iface, t_ns, data, "");
    }
    pub fn packet_with_comment(&mut self, iface: u32, t_ns: u64, data: &[u8], comment: &str) {
        let mut body = Vec::with_capacity(20 + data.len() + 4);
        body.extend_from_slice(&iface.to_le_bytes());
        body.extend_from_slice(&((t_ns >> 32) as u32).to_le_bytes());
        body.extend_from_slice(&(t_ns as u32).to_le_bytes());
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(data);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        if !comment.is_empty() {
            opt(&mut body, OPT_COMMENT, comment.as_bytes());
            opt_end(&mut body);
        }
        self.block(BT_EPB, &body);
    }
    pub fn interfaces(&self) -> u32 {
        self.ifaces
    }
    pub fn bytes(&self) -> &[u8] {
        &self.buf
    }
    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
}

/// an interface of a read capture
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    pub link: u16,
    pub name: String,
    /// timestamp units per second
    pub units: u64,
}

/// a packet of a read capture
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub iface: u32,
    /// nanoseconds (converted from the interface's resolution)
    pub t_ns: u64,
    pub data: Vec<u8>,
    pub orig_len: u32,
    pub comment: String,
}

/// a read capture
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capture {
    pub app: String,
    pub interfaces: Vec<Interface>,
    pub packets: Vec<Packet>,
}

/// why a capture could not be read
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadErr {
    NotPcapng,
    Truncated,
    BadBlock,
}

struct Rd<'a> {
    b: &'a [u8],
    le: bool,
}

impl Rd<'_> {
    fn u16(&self, o: usize) -> u16 {
        let a = [self.b[o], self.b[o + 1]];
        if self.le { u16::from_le_bytes(a) } else { u16::from_be_bytes(a) }
    }
    fn u32(&self, o: usize) -> u32 {
        let a = [self.b[o], self.b[o + 1], self.b[o + 2], self.b[o + 3]];
        if self.le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) }
    }
    /// options in b[from..to]: (code, value)
    fn opts(&self, mut o: usize, to: usize) -> Vec<(u16, &[u8])> {
        let mut v = Vec::new();
        while o + 4 <= to {
            let code = self.u16(o);
            let len = self.u16(o + 2) as usize;
            if code == OPT_END || o + 4 + len > to {
                break;
            }
            v.push((code, &self.b[o + 4..o + 4 + len]));
            o += 4 + ((len + 3) & !3);
        }
        v
    }
}

/// reads a pcapng file (one or more sections)
pub fn read(bytes: &[u8]) -> Result<Capture, ReadErr> {
    if bytes.len() < 28 || bytes[..4] != BT_SHB.to_le_bytes() {
        return Err(ReadErr::NotPcapng);
    }
    let mut cap = Capture::default();
    let mut r = Rd { b: bytes, le: true };
    let mut base = 0u32; // interface ids restart per section
    let mut o = 0;
    while o < bytes.len() {
        if o + 12 > bytes.len() {
            return Err(ReadErr::Truncated);
        }
        let raw_typ = u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        if raw_typ == BT_SHB {
            let m = &bytes[o + 8..o + 12];
            r.le = if m == MAGIC.to_le_bytes() {
                true
            } else if m == MAGIC.to_be_bytes() {
                false
            } else {
                return Err(ReadErr::NotPcapng);
            };
            base = cap.interfaces.len() as u32;
        }
        let typ = r.u32(o);
        let len = r.u32(o + 4) as usize;
        if len < 12 || len % 4 != 0 || o + len > bytes.len() {
            return Err(ReadErr::BadBlock);
        }
        let body = o + 8;
        let end = o + len - 4;
        match typ {
            BT_SHB => {
                if end >= body + 16 {
                    for (c, v) in r.opts(body + 16, end) {
                        if c == OPT_SHB_USERAPPL {
                            cap.app = String::from_utf8_lossy(v).into_owned();
                        }
                    }
                }
            }
            BT_IDB => {
                if end < body + 8 {
                    return Err(ReadErr::BadBlock);
                }
                let mut i = Interface { link: r.u16(body), name: String::new(), units: 1_000_000 };
                for (c, v) in r.opts(body + 8, end) {
                    match c {
                        OPT_IF_NAME => i.name = String::from_utf8_lossy(v).into_owned(),
                        OPT_IF_TSRESOL if !v.is_empty() => {
                            let e = (v[0] & 0x7F) as u32;
                            i.units = if v[0] & 0x80 != 0 { 1u64.checked_shl(e).unwrap_or(0) } else { 10u64.checked_pow(e).unwrap_or(0) };
                        }
                        _ => {}
                    }
                }
                cap.interfaces.push(i);
            }
            BT_EPB => {
                if end < body + 20 {
                    return Err(ReadErr::BadBlock);
                }
                let iface = base + r.u32(body);
                let ts = ((r.u32(body + 4) as u64) << 32) | r.u32(body + 8) as u64;
                let caplen = r.u32(body + 12) as usize;
                let orig_len = r.u32(body + 16);
                if body + 20 + caplen > end {
                    return Err(ReadErr::BadBlock);
                }
                let units = cap.interfaces.get(iface as usize).map(|i| i.units).unwrap_or(1_000_000);
                let t_ns = if units == 0 { ts } else { (ts as u128 * 1_000_000_000 / units as u128) as u64 };
                let mut p = Packet { iface, t_ns, data: bytes[body + 20..body + 20 + caplen].to_vec(), orig_len, comment: String::new() };
                for (c, v) in r.opts(body + 20 + ((caplen + 3) & !3), end) {
                    if c == OPT_COMMENT {
                        p.comment = String::from_utf8_lossy(v).into_owned();
                    }
                }
                cap.packets.push(p);
            }
            BT_SPB => {
                if end < body + 4 {
                    return Err(ReadErr::BadBlock);
                }
                let orig_len = r.u32(body);
                let n = (orig_len as usize).min(end - body - 4);
                cap.packets.push(Packet { iface: base, t_ns: 0, data: bytes[body + 4..body + 4 + n].to_vec(), orig_len, comment: String::new() });
            }
            _ => {}
        }
        o += len;
    }
    Ok(cap)
}
