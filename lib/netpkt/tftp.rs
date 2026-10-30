// tftp -- Trivial File Transfer Protocol packets (UDP port 69, RFC 1350) with the option
// extension of RFC 2347 (OACK, error 8) and the options blksize (RFC 2348), timeout and tsize
// (RFC 2349)
//
//   TftpView::parse(b)        the opcode and the fields of each kind: file / mode / options
//                             (RRQ, WRQ, OACK as NUL-terminated strings with their offsets),
//                             block and data (DATA), block (ACK), code and message (ERROR)
//   Packet                    Rrq / Wrq { file, mode, opts } | Data { block, data } |
//                             Ack { block } | Error { code, msg } | Oack { opts }:
//                             read(view) / parse(b) / to_bytes / opcode / opt(name)
//   Packet::error_of(code)    an ERROR with the RFC's text of the code
//   OP_* (1 .. 6), ERR_* (0 .. 8), op_name, error_name, BLOCK (512), BLKSIZE_MIN / _MAX,
//   mode_known ("octet", "netascii", "mail"), to_netascii / from_netascii (RFC 764 line ends)
//   looks_like(b)             the shape of a packet of a running transfer (DATA, ACK, ERROR,
//                             OACK): RFC 1350 moves a transfer to new ports at once, so a
//                             dissector without state knows it only by its bytes
//
// A transfer is lock-step: every DATA block waits for its ACK, a block shorter than the block
// size ends it (an empty one when the size is a multiple), block numbers count from 1 and roll
// over after 65535.  Option names compare case-insensitively; values are decimal text.  Errors
// of parse name what is wrong for a dissector (Bad).

use alloc::string::String;
use alloc::vec::Vec;

use crate::{be16, put16};

pub const OP_RRQ: u16 = 1;
pub const OP_WRQ: u16 = 2;
pub const OP_DATA: u16 = 3;
pub const OP_ACK: u16 = 4;
pub const OP_ERROR: u16 = 5;
/// option acknowledgment (RFC 2347)
pub const OP_OACK: u16 = 6;

/// opcode and block number (DATA, ACK) or error code (ERROR)
pub const TFTP_HDR: usize = 4;
/// data bytes in a block without the blksize option
pub const BLOCK: usize = 512;
/// the block sizes RFC 2348 allows
pub const BLKSIZE_MIN: u16 = 8;
pub const BLKSIZE_MAX: u16 = 65464;

/// "Not defined, see error message (if any)."
pub const ERR_UNDEFINED: u16 = 0;
pub const ERR_NOT_FOUND: u16 = 1;
pub const ERR_ACCESS: u16 = 2;
pub const ERR_DISK_FULL: u16 = 3;
pub const ERR_ILLEGAL: u16 = 4;
pub const ERR_UNKNOWN_TID: u16 = 5;
pub const ERR_EXISTS: u16 = 6;
pub const ERR_NO_USER: u16 = 7;
/// option negotiation failed (RFC 2347)
pub const ERR_OPTION: u16 = 8;

/// the name of an opcode as capture tools show it
pub fn op_name(op: u16) -> &'static str {
    match op {
        OP_RRQ => "Read Request",
        OP_WRQ => "Write Request",
        OP_DATA => "Data Packet",
        OP_ACK => "Acknowledgement",
        OP_ERROR => "Error Code",
        OP_OACK => "Option Acknowledgement",
        _ => "Unknown",
    }
}

/// the meaning of an error code (RFC 1350 appendix, RFC 2347)
pub fn error_name(code: u16) -> &'static str {
    match code {
        ERR_UNDEFINED => "Not defined",
        ERR_NOT_FOUND => "File not found",
        ERR_ACCESS => "Access violation",
        ERR_DISK_FULL => "Disk full or allocation exceeded",
        ERR_ILLEGAL => "Illegal TFTP operation",
        ERR_UNKNOWN_TID => "Unknown transfer ID",
        ERR_EXISTS => "File already exists",
        ERR_NO_USER => "No such user",
        ERR_OPTION => "Option negotiation failed",
        _ => "Unknown error",
    }
}

/// a transfer mode RFC 1350 defines (case-insensitive)
pub fn mode_known(mode: &str) -> bool {
    ["octet", "netascii", "mail"].iter().any(|m| m.eq_ignore_ascii_case(mode))
}

/// bytes as netascii: LF becomes CR LF, a lone CR becomes CR NUL (RFC 764)
pub fn to_netascii(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 16);
    for &c in data {
        match c {
            b'\n' => out.extend_from_slice(b"\r\n"),
            b'\r' => out.extend_from_slice(b"\r\0"),
            c => out.push(c),
        }
    }
    out
}

/// netascii back to bytes: CR LF is LF, CR NUL is CR
pub fn from_netascii(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i] == b'\r' && i + 1 < data.len() && (data[i + 1] == b'\n' || data[i + 1] == 0) {
            out.push(if data[i + 1] == b'\n' { b'\n' } else { b'\r' });
            i += 2;
        } else {
            out.push(data[i]);
            i += 1;
        }
    }
    out
}

/// why bytes are no TFTP packet
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bad {
    /// shorter than its opcode and fixed fields
    Short,
    /// an opcode RFC 1350 and 2347 do not define
    Opcode(u16),
    /// a string field without its NUL
    Unterminated,
    /// an option name without a value
    OddOptions,
}

/// a NUL-terminated string field: its offset, its length without the NUL
pub type Span = (usize, usize);

/// a received TFTP packet
#[derive(Clone, Copy, Debug)]
pub struct TftpView<'a> {
    b: &'a [u8],
}

impl<'a> TftpView<'a> {
    /// at least an opcode RFC 1350 or 2347 defines and the fields its kind has
    pub fn parse(b: &'a [u8]) -> Result<TftpView<'a>, Bad> {
        if b.len() < 2 {
            return Err(Bad::Short);
        }
        let v = TftpView { b };
        match v.opcode() {
            OP_RRQ | OP_WRQ => {
                let s = strings(&b[2..])?;
                if s.len() < 2 {
                    return Err(Bad::Unterminated);
                }
                if s.len() % 2 != 0 {
                    return Err(Bad::OddOptions);
                }
            }
            OP_OACK => {
                if strings(&b[2..])?.len() % 2 != 0 {
                    return Err(Bad::OddOptions);
                }
            }
            OP_DATA | OP_ACK => {
                if b.len() < TFTP_HDR {
                    return Err(Bad::Short);
                }
            }
            OP_ERROR => {
                if b.len() < TFTP_HDR + 1 {
                    return Err(Bad::Short);
                }
                if !b[TFTP_HDR..].contains(&0) {
                    return Err(Bad::Unterminated);
                }
            }
            op => return Err(Bad::Opcode(op)),
        }
        Ok(v)
    }
    pub fn bytes(&self) -> &'a [u8] {
        self.b
    }
    pub fn opcode(&self) -> u16 {
        be16(self.b, 0)
    }
    /// the block number of DATA and ACK
    pub fn block(&self) -> u16 {
        if self.b.len() >= TFTP_HDR { be16(self.b, 2) } else { 0 }
    }
    /// the data of a DATA packet
    pub fn data(&self) -> &'a [u8] {
        if self.opcode() == OP_DATA { &self.b[TFTP_HDR.min(self.b.len())..] } else { &[] }
    }
    /// the code of an ERROR
    pub fn error_code(&self) -> u16 {
        self.block()
    }
    /// the message of an ERROR (up to its NUL)
    pub fn error_msg(&self) -> String {
        if self.opcode() != OP_ERROR || self.b.len() <= TFTP_HDR {
            return String::new();
        }
        let m = &self.b[TFTP_HDR..];
        let n = m.iter().position(|&c| c == 0).unwrap_or(m.len());
        String::from_utf8_lossy(&m[..n]).into_owned()
    }
    /// the string fields after the opcode (RRQ, WRQ: file, mode, then option pairs; OACK: the
    /// pairs), offsets relative to the packet
    pub fn fields(&self) -> Vec<Span> {
        match self.opcode() {
            OP_RRQ | OP_WRQ | OP_OACK => strings(&self.b[2..]).map(|v| v.into_iter().map(|(o, l)| (o + 2, l)).collect()).unwrap_or_default(),
            _ => Vec::new(),
        }
    }
    /// a field as text
    pub fn text(&self, s: Span) -> String {
        String::from_utf8_lossy(&self.b[s.0..s.0 + s.1]).into_owned()
    }
}

/// the NUL-terminated strings of `b`: every byte must belong to one
fn strings(b: &[u8]) -> Result<Vec<Span>, Bad> {
    let mut v = Vec::new();
    let mut start = 0;
    for (i, &c) in b.iter().enumerate() {
        if c == 0 {
            v.push((start, i - start));
            start = i + 1;
        }
    }
    if start != b.len() {
        return Err(Bad::Unterminated);
    }
    Ok(v)
}

/// a TFTP packet to write (or one read)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Packet {
    Rrq { file: String, mode: String, opts: Vec<(String, String)> },
    Wrq { file: String, mode: String, opts: Vec<(String, String)> },
    Data { block: u16, data: Vec<u8> },
    Ack { block: u16 },
    Error { code: u16, msg: String },
    Oack { opts: Vec<(String, String)> },
}

impl Packet {
    pub fn read(v: &TftpView) -> Packet {
        let f = v.fields();
        let pairs = |from: usize| -> Vec<(String, String)> { f[from..].chunks(2).filter(|c| c.len() == 2).map(|c| (v.text(c[0]), v.text(c[1]))).collect() };
        match v.opcode() {
            OP_RRQ | OP_WRQ => {
                let file = f.first().map(|&s| v.text(s)).unwrap_or_default();
                let mode = f.get(1).map(|&s| v.text(s)).unwrap_or_default();
                let opts = if f.len() > 2 { pairs(2) } else { Vec::new() };
                if v.opcode() == OP_RRQ { Packet::Rrq { file, mode, opts } } else { Packet::Wrq { file, mode, opts } }
            }
            OP_DATA => Packet::Data { block: v.block(), data: v.data().to_vec() },
            OP_ACK => Packet::Ack { block: v.block() },
            OP_ERROR => Packet::Error { code: v.error_code(), msg: v.error_msg() },
            _ => Packet::Oack { opts: pairs(0) },
        }
    }
    /// bytes as a packet
    pub fn parse(b: &[u8]) -> Result<Packet, Bad> {
        TftpView::parse(b).map(|v| Packet::read(&v))
    }
    /// an ERROR with the meaning of its code as the message
    pub fn error_of(code: u16) -> Packet {
        Packet::Error { code, msg: String::from(error_name(code)) }
    }
    pub fn opcode(&self) -> u16 {
        match self {
            Packet::Rrq { .. } => OP_RRQ,
            Packet::Wrq { .. } => OP_WRQ,
            Packet::Data { .. } => OP_DATA,
            Packet::Ack { .. } => OP_ACK,
            Packet::Error { .. } => OP_ERROR,
            Packet::Oack { .. } => OP_OACK,
        }
    }
    /// an option of a request or an OACK (the name ignores case)
    pub fn opt(&self, name: &str) -> Option<&str> {
        let opts = match self {
            Packet::Rrq { opts, .. } | Packet::Wrq { opts, .. } | Packet::Oack { opts } => opts,
            _ => return None,
        };
        opts.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = alloc::vec![0u8; 2];
        put16(&mut b, 0, self.opcode());
        let put_str = |b: &mut Vec<u8>, s: &str| {
            b.extend_from_slice(s.as_bytes());
            b.push(0);
        };
        match self {
            Packet::Rrq { file, mode, opts } | Packet::Wrq { file, mode, opts } => {
                put_str(&mut b, file);
                put_str(&mut b, mode);
                for (k, v) in opts {
                    put_str(&mut b, k);
                    put_str(&mut b, v);
                }
            }
            Packet::Data { block, data } => {
                b.extend_from_slice(&block.to_be_bytes());
                b.extend_from_slice(data);
            }
            Packet::Ack { block } => b.extend_from_slice(&block.to_be_bytes()),
            Packet::Error { code, msg } => {
                b.extend_from_slice(&code.to_be_bytes());
                put_str(&mut b, msg);
            }
            Packet::Oack { opts } => {
                for (k, v) in opts {
                    put_str(&mut b, k);
                    put_str(&mut b, v);
                }
            }
        }
        b
    }
}

/// the bytes of a packet of a running transfer: a DATA block (at most BLKSIZE_MAX bytes), an
/// ACK of exactly four bytes, an ERROR with a known code and a printable message, an OACK of
/// option names and values
pub fn looks_like(b: &[u8]) -> bool {
    if b.len() < TFTP_HDR {
        return false;
    }
    match be16(b, 0) {
        OP_DATA => b.len() <= TFTP_HDR + BLKSIZE_MAX as usize,
        OP_ACK => b.len() == TFTP_HDR,
        OP_ERROR => {
            let m = &b[TFTP_HDR..];
            be16(b, 2) <= ERR_OPTION && m.last() == Some(&0) && m[..m.len() - 1].iter().all(|&c| c >= 0x20 && c != 0x7F)
        }
        OP_OACK => match strings(&b[2..]) {
            Ok(s) => !s.is_empty() && s.len() % 2 == 0 && s.chunks(2).all(|c| c[0].1 > 0 && b[2 + c[0].0..2 + c[0].0 + c[0].1].iter().all(|x| x.is_ascii_alphanumeric()) && b[2 + c[1].0..2 + c[1].0 + c[1].1].iter().all(|x| x.is_ascii_digit())),
            Err(_) => false,
        },
        _ => false,
    }
}
