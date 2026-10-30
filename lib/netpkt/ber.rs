// ber -- the Basic Encoding Rules of ASN.1 (X.690) as SNMP uses them (RFC 3417 section 8): the
// universal types INTEGER, OCTET STRING, NULL, OBJECT IDENTIFIER and SEQUENCE, the application
// types of the SMI (RFC 2578: IpAddress, Counter32, Gauge32, TimeTicks, Opaque, Counter64) and
// the exceptions of SNMPv2 (RFC 3416: noSuchObject, noSuchInstance, endOfMibView)
//
//   Oid                   an OBJECT IDENTIFIER as its arcs: parse("1.3.6.1.2.1.1.5.0") (a
//                         leading dot is allowed), Display (dotted), Ord (the lexicographic
//                         order of SNMP: a prefix before what it starts), starts_with, child,
//                         encode() / decode(contents)
//   Value                 what a variable binding holds: Int, Str, Null, Oid, Ip, Counter32,
//                         Gauge32, TimeTicks, Opaque, Counter64, NoSuchObject, NoSuchInstance,
//                         EndOfMibView; tag(), type_name(), is_exception()
//   Tlv                   one element: its tag, where it starts, the length of its header and
//                         its contents (offsets in the buffer the Reader walks)
//   Reader                walks a buffer: peek(), tlv(), int(), uint(tag, max), octets(), null(),
//                         oid(), value(), seq(tag) (a Reader over the contents), done()
//   Writer                builds a message: int, uint, octets, null, oid, value, begin(tag) /
//                         end() around the elements of a SEQUENCE or a PDU (the length goes in at
//                         end()); into_vec()
//   tlv(tag, contents)    one element as bytes
//   Bad                   why bytes are not BER as SNMP wants it
//
// Strict about lengths: every length must fit in what is left of its buffer and of the element
// around it.  The indefinite form (0x80) is refused, a long form of more than four octets too
// (RFC 3417 allows more length octets than needed, so 0x81 0x05 is read as 5); a tag of more than
// one octet (low bits 31) is refused.  An INTEGER has 1 to 8 content octets, an unsigned value 9
// when the first is 0x00; Counter32, Gauge32 and TimeTicks fit 32 bits (RFC 2578: 0 ..
// 4294967295), an Integer32 in a variable binding fits i32; NULL and the exceptions have no
// contents; an IpAddress has four octets.  An OBJECT IDENTIFIER has at least one content octet,
// each subidentifier in its shortest form (no leading 0x80, X.690 8.19.2), at most 2^32 - 1 and
// ended (its last octet below 0x80), at most 128 subidentifiers (RFC 2578 3.5).  A simple type in
// its constructed form (0x24 for an OCTET STRING) is no tag SNMP knows.  Writers always use the
// shortest form of lengths and integers.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::Ipv4;

pub const T_INTEGER: u8 = 0x02;
pub const T_OCTETS: u8 = 0x04;
pub const T_NULL: u8 = 0x05;
pub const T_OID: u8 = 0x06;
pub const T_SEQUENCE: u8 = 0x30;
/// application types of the SMI (RFC 2578 7.1)
pub const T_IPADDRESS: u8 = 0x40;
pub const T_COUNTER32: u8 = 0x41;
pub const T_GAUGE32: u8 = 0x42;
pub const T_TIMETICKS: u8 = 0x43;
pub const T_OPAQUE: u8 = 0x44;
pub const T_COUNTER64: u8 = 0x46;
/// the exceptions of a variable binding in a Response (RFC 3416 3)
pub const T_NO_SUCH_OBJECT: u8 = 0x80;
pub const T_NO_SUCH_INSTANCE: u8 = 0x81;
pub const T_END_OF_MIB_VIEW: u8 = 0x82;

/// the most subidentifiers an OBJECT IDENTIFIER of the SMI has
pub const OID_MAX: usize = 128;

/// why bytes are no BER element as SNMP wants it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bad {
    /// the buffer ends inside a header or before the contents it announces
    Truncated,
    /// the indefinite length form (forbidden by RFC 3417)
    Indefinite,
    /// a long length form of more than four octets
    Length,
    /// a tag of more than one octet, or one that does not belong here
    Tag(u8),
    /// an INTEGER without contents or with more octets than fit
    IntSize,
    /// a value outside its type's range (a negative counter, more than 32 bits)
    Range,
    /// NULL or an exception with contents, an IpAddress that is not four octets
    Size,
    /// an OBJECT IDENTIFIER that is empty, not in its shortest form, too big or unended
    Oid,
    /// bytes after the last element of a SEQUENCE or the message
    Trailing,
}

impl Bad {
    /// what is wrong, for a dissector
    pub fn text(self) -> &'static str {
        match self {
            Bad::Truncated => "the data ends before the element it announces",
            Bad::Indefinite => "an indefinite length (SNMP allows definite ones only)",
            Bad::Length => "a length of more than four octets",
            Bad::Tag(_) => "a type that does not belong here",
            Bad::IntSize => "an integer without contents or too long",
            Bad::Range => "a value outside the range of its type",
            Bad::Size => "contents of the wrong size for the type",
            Bad::Oid => "a malformed object identifier",
            Bad::Trailing => "bytes after the last element",
        }
    }
}

// ---------------------------------------------------------------- object identifiers

/// an OBJECT IDENTIFIER: its arcs ("1.3.6.1.2.1.1.5.0")
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Oid(pub Vec<u32>);

impl Oid {
    pub fn new(arcs: &[u32]) -> Oid {
        Oid(Vec::from(arcs))
    }
    /// dotted decimal, a leading dot allowed; at least two arcs, the first 0, 1 or 2, the
    /// second below 40 under 0 and 1
    pub fn parse(s: &str) -> Option<Oid> {
        let s = s.trim();
        let s = s.strip_prefix('.').unwrap_or(s);
        if s.is_empty() {
            return None;
        }
        let mut v = Vec::new();
        for part in s.split('.') {
            if part.is_empty() || part.len() > 10 || !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            v.push(part.parse::<u32>().ok()?);
        }
        let ok = v.len() >= 2 && v.len() <= OID_MAX && v[0] <= 2 && (v[0] == 2 || v[1] < 40);
        ok.then_some(Oid(v))
    }
    pub fn arcs(&self) -> &[u32] {
        &self.0
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// this OID begins with `prefix` (or is it)
    pub fn starts_with(&self, prefix: &[u32]) -> bool {
        self.0.starts_with(prefix)
    }
    /// this OID with more arcs after it
    pub fn child(&self, more: &[u32]) -> Oid {
        let mut v = self.0.clone();
        v.extend_from_slice(more);
        Oid(v)
    }
    /// the contents octets of its encoding (X.690 8.19): the first two arcs as 40 X + Y, then
    /// each arc in base 128 with the high bit on all octets but its last (one arc: X.0)
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let a = &self.0;
        let first = match a.len() {
            0 => 0u64,
            1 => a[0] as u64 * 40,
            _ => a[0] as u64 * 40 + a[1] as u64,
        };
        push_base128(&mut out, first);
        for &x in a.iter().skip(2) {
            push_base128(&mut out, x as u64);
        }
        out
    }
    /// the OID of contents octets
    pub fn decode(c: &[u8]) -> Result<Oid, Bad> {
        if c.is_empty() {
            return Err(Bad::Oid);
        }
        let mut arcs: Vec<u32> = Vec::new();
        let mut i = 0;
        let mut first = true;
        while i < c.len() {
            if c[i] == 0x80 {
                // not the shortest form (X.690 8.19.2)
                return Err(Bad::Oid);
            }
            let mut v: u64 = 0;
            loop {
                let Some(&b) = c.get(i) else {
                    return Err(Bad::Oid);
                };
                i += 1;
                v = (v << 7) | (b & 0x7F) as u64;
                if v > u32::MAX as u64 + 80 {
                    return Err(Bad::Oid);
                }
                if b & 0x80 == 0 {
                    break;
                }
            }
            if first {
                first = false;
                let (x, y) = if v < 40 {
                    (0, v)
                } else if v < 80 {
                    (1, v - 40)
                } else {
                    (2, v - 80)
                };
                if y > u32::MAX as u64 {
                    return Err(Bad::Oid);
                }
                arcs.push(x);
                arcs.push(y as u32);
            } else {
                if v > u32::MAX as u64 {
                    return Err(Bad::Oid);
                }
                arcs.push(v as u32);
            }
            if arcs.len() > OID_MAX {
                return Err(Bad::Oid);
            }
        }
        Ok(Oid(arcs))
    }
}

fn push_base128(out: &mut Vec<u8>, v: u64) {
    let mut tmp = [0u8; 10];
    let mut n = 0;
    let mut x = v;
    loop {
        tmp[n] = (x & 0x7F) as u8;
        n += 1;
        x >>= 7;
        if x == 0 {
            break;
        }
    }
    for k in (0..n).rev() {
        out.push(tmp[k] | if k > 0 { 0x80 } else { 0 });
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for (i, a) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(".")?;
            }
            write!(f, "{}", a)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- values

/// the value of a variable binding
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// INTEGER (Integer32 in a variable binding)
    Int(i64),
    /// OCTET STRING
    Str(Vec<u8>),
    Null,
    Oid(Oid),
    Ip(Ipv4),
    Counter32(u32),
    Gauge32(u32),
    TimeTicks(u32),
    Opaque(Vec<u8>),
    Counter64(u64),
    NoSuchObject,
    NoSuchInstance,
    EndOfMibView,
}

impl Value {
    /// a text as an OCTET STRING
    pub fn text(s: &str) -> Value {
        Value::Str(Vec::from(s.as_bytes()))
    }
    pub fn tag(&self) -> u8 {
        match self {
            Value::Int(_) => T_INTEGER,
            Value::Str(_) => T_OCTETS,
            Value::Null => T_NULL,
            Value::Oid(_) => T_OID,
            Value::Ip(_) => T_IPADDRESS,
            Value::Counter32(_) => T_COUNTER32,
            Value::Gauge32(_) => T_GAUGE32,
            Value::TimeTicks(_) => T_TIMETICKS,
            Value::Opaque(_) => T_OPAQUE,
            Value::Counter64(_) => T_COUNTER64,
            Value::NoSuchObject => T_NO_SUCH_OBJECT,
            Value::NoSuchInstance => T_NO_SUCH_INSTANCE,
            Value::EndOfMibView => T_END_OF_MIB_VIEW,
        }
    }
    /// the name of its type as the SMI and RFC 3416 write it
    pub fn type_name(&self) -> &'static str {
        type_name(self.tag())
    }
    /// one of the three exceptions of a Response
    pub fn is_exception(&self) -> bool {
        matches!(self, Value::NoSuchObject | Value::NoSuchInstance | Value::EndOfMibView)
    }
    /// the contents octets of its encoding
    pub fn contents(&self) -> Vec<u8> {
        match self {
            Value::Int(v) => int_bytes(*v),
            Value::Str(b) | Value::Opaque(b) => b.clone(),
            Value::Null | Value::NoSuchObject | Value::NoSuchInstance | Value::EndOfMibView => Vec::new(),
            Value::Oid(o) => o.encode(),
            Value::Ip(a) => Vec::from(&a.0[..]),
            Value::Counter32(v) | Value::Gauge32(v) | Value::TimeTicks(v) => uint_bytes(*v as u64),
            Value::Counter64(v) => uint_bytes(*v),
        }
    }
    /// the value of an element read (the contents of `t` in `b`)
    pub fn read(t: &Tlv, b: &[u8]) -> Result<Value, Bad> {
        let c = t.contents(b);
        Ok(match t.tag {
            T_INTEGER => {
                let v = int_of(c)?;
                if v < i32::MIN as i64 || v > i32::MAX as i64 {
                    return Err(Bad::Range);
                }
                Value::Int(v)
            }
            T_OCTETS => Value::Str(Vec::from(c)),
            T_NULL => {
                if !c.is_empty() {
                    return Err(Bad::Size);
                }
                Value::Null
            }
            T_OID => Value::Oid(Oid::decode(c)?),
            T_IPADDRESS => {
                if c.len() != 4 {
                    return Err(Bad::Size);
                }
                Value::Ip(Ipv4([c[0], c[1], c[2], c[3]]))
            }
            T_COUNTER32 => Value::Counter32(uint_of(c, u32::MAX as u64)? as u32),
            T_GAUGE32 => Value::Gauge32(uint_of(c, u32::MAX as u64)? as u32),
            T_TIMETICKS => Value::TimeTicks(uint_of(c, u32::MAX as u64)? as u32),
            T_OPAQUE => Value::Opaque(Vec::from(c)),
            T_COUNTER64 => Value::Counter64(uint_of(c, u64::MAX)?),
            T_NO_SUCH_OBJECT | T_NO_SUCH_INSTANCE | T_END_OF_MIB_VIEW => {
                if !c.is_empty() {
                    return Err(Bad::Size);
                }
                match t.tag {
                    T_NO_SUCH_OBJECT => Value::NoSuchObject,
                    T_NO_SUCH_INSTANCE => Value::NoSuchInstance,
                    _ => Value::EndOfMibView,
                }
            }
            other => return Err(Bad::Tag(other)),
        })
    }
}

/// the name of a type of a variable binding (by its tag)
pub fn type_name(tag: u8) -> &'static str {
    match tag {
        T_INTEGER => "INTEGER",
        T_OCTETS => "OCTET STRING",
        T_NULL => "NULL",
        T_OID => "OBJECT IDENTIFIER",
        T_SEQUENCE => "SEQUENCE",
        T_IPADDRESS => "IpAddress",
        T_COUNTER32 => "Counter32",
        T_GAUGE32 => "Gauge32",
        T_TIMETICKS => "TimeTicks",
        T_OPAQUE => "Opaque",
        T_COUNTER64 => "Counter64",
        T_NO_SUCH_OBJECT => "noSuchObject",
        T_NO_SUCH_INSTANCE => "noSuchInstance",
        T_END_OF_MIB_VIEW => "endOfMibView",
        _ => "unknown",
    }
}

/// the shortest two's complement of `v` (one to eight octets)
pub fn int_bytes(v: i64) -> Vec<u8> {
    let b = v.to_be_bytes();
    let mut i = 0;
    while i < 7 && ((b[i] == 0x00 && b[i + 1] & 0x80 == 0) || (b[i] == 0xFF && b[i + 1] & 0x80 != 0)) {
        i += 1;
    }
    Vec::from(&b[i..])
}

/// the shortest encoding of an unsigned value as an INTEGER's contents (a 0x00 before a first
/// octet with its high bit set)
pub fn uint_bytes(v: u64) -> Vec<u8> {
    let b = v.to_be_bytes();
    let mut i = 0;
    while i < 7 && b[i] == 0 {
        i += 1;
    }
    let mut out = Vec::with_capacity(9);
    if b[i] & 0x80 != 0 {
        out.push(0);
    }
    out.extend_from_slice(&b[i..]);
    out
}

/// the contents of an INTEGER as a signed value
pub fn int_of(c: &[u8]) -> Result<i64, Bad> {
    if c.is_empty() || c.len() > 8 {
        return Err(Bad::IntSize);
    }
    let mut v: i64 = if c[0] & 0x80 != 0 { -1 } else { 0 };
    for &x in c {
        v = (v << 8) | x as i64;
    }
    Ok(v)
}

/// the contents of an unsigned type (at most `max`; 9 octets only with a leading 0x00)
pub fn uint_of(c: &[u8], max: u64) -> Result<u64, Bad> {
    if c.is_empty() || c.len() > 9 || (c.len() == 9 && c[0] != 0) {
        return Err(Bad::IntSize);
    }
    if c[0] & 0x80 != 0 {
        return Err(Bad::Range);
    }
    let mut v: u64 = 0;
    for &x in c {
        v = (v << 8) | x as u64;
    }
    if v > max {
        return Err(Bad::Range);
    }
    Ok(v)
}

// ---------------------------------------------------------------- reading

/// one element: tag, offset of its first octet, header length, contents length
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tlv {
    pub tag: u8,
    pub off: usize,
    pub hdr: usize,
    pub len: usize,
}

impl Tlv {
    /// where its contents start
    pub fn body(&self) -> usize {
        self.off + self.hdr
    }
    /// where it ends
    pub fn end(&self) -> usize {
        self.off + self.hdr + self.len
    }
    /// its contents in the buffer it was read from
    pub fn contents<'a>(&self, b: &'a [u8]) -> &'a [u8] {
        &b[self.body()..self.end()]
    }
    /// a constructed element (a SEQUENCE or a PDU)
    pub fn constructed(&self) -> bool {
        self.tag & 0x20 != 0
    }
}

/// the header of the element at `off` of `b`, which must end by `end`
pub fn header(b: &[u8], off: usize, end: usize) -> Result<Tlv, Bad> {
    let end = end.min(b.len());
    if off + 2 > end {
        return Err(Bad::Truncated);
    }
    let tag = b[off];
    if tag & 0x1F == 0x1F {
        return Err(Bad::Tag(tag));
    }
    let l0 = b[off + 1];
    let (len, hdr) = if l0 < 0x80 {
        (l0 as usize, 2)
    } else if l0 == 0x80 {
        return Err(Bad::Indefinite);
    } else {
        let n = (l0 & 0x7F) as usize;
        if n > 4 {
            return Err(Bad::Length);
        }
        if off + 2 + n > end {
            return Err(Bad::Truncated);
        }
        let mut v: usize = 0;
        for k in 0..n {
            v = (v << 8) | b[off + 2 + k] as usize;
        }
        (v, 2 + n)
    };
    if len > end - off - hdr {
        return Err(Bad::Truncated);
    }
    Ok(Tlv { tag, off, hdr, len })
}

/// a walk over the elements of a buffer (or of one constructed element's contents)
#[derive(Clone, Copy, Debug)]
pub struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
    end: usize,
}

impl<'a> Reader<'a> {
    /// over the whole buffer
    pub fn new(b: &'a [u8]) -> Reader<'a> {
        Reader { b, pos: 0, end: b.len() }
    }
    /// the buffer it reads (offsets of Tlv are in it)
    pub fn buf(&self) -> &'a [u8] {
        self.b
    }
    pub fn pos(&self) -> usize {
        self.pos
    }
    /// nothing is left
    pub fn is_empty(&self) -> bool {
        self.pos >= self.end
    }
    /// Err(Trailing) while bytes are left
    pub fn done(&self) -> Result<(), Bad> {
        if self.is_empty() { Ok(()) } else { Err(Bad::Trailing) }
    }
    /// the header of the next element without taking it
    pub fn peek(&self) -> Result<Tlv, Bad> {
        header(self.b, self.pos, self.end)
    }
    /// the next element
    pub fn tlv(&mut self) -> Result<Tlv, Bad> {
        let t = self.peek()?;
        self.pos = t.end();
        Ok(t)
    }
    /// the next element, which must have tag `tag`
    pub fn expect(&mut self, tag: u8) -> Result<Tlv, Bad> {
        let t = self.peek()?;
        if t.tag != tag {
            return Err(Bad::Tag(t.tag));
        }
        self.pos = t.end();
        Ok(t)
    }
    /// a constructed element of tag `tag`: a reader over its contents
    pub fn seq(&mut self, tag: u8) -> Result<Reader<'a>, Bad> {
        let t = self.expect(tag)?;
        Ok(Reader { b: self.b, pos: t.body(), end: t.end() })
    }
    /// the contents of a constructed element as a reader (any tag)
    pub fn inside(&self, t: &Tlv) -> Reader<'a> {
        Reader { b: self.b, pos: t.body(), end: t.end() }
    }
    /// an INTEGER
    pub fn int(&mut self) -> Result<i64, Bad> {
        let t = self.expect(T_INTEGER)?;
        int_of(t.contents(self.b))
    }
    /// an INTEGER that fits i32 (request ids, error status and index)
    pub fn int32(&mut self) -> Result<i32, Bad> {
        let v = self.int()?;
        i32::try_from(v).map_err(|_| Bad::Range)
    }
    /// an unsigned value of tag `tag` up to `max`
    pub fn uint(&mut self, tag: u8, max: u64) -> Result<u64, Bad> {
        let t = self.expect(tag)?;
        uint_of(t.contents(self.b), max)
    }
    /// an OCTET STRING
    pub fn octets(&mut self) -> Result<&'a [u8], Bad> {
        let t = self.expect(T_OCTETS)?;
        Ok(&self.b[t.body()..t.end()])
    }
    pub fn null(&mut self) -> Result<(), Bad> {
        let t = self.expect(T_NULL)?;
        if t.len != 0 { Err(Bad::Size) } else { Ok(()) }
    }
    pub fn oid(&mut self) -> Result<Oid, Bad> {
        let t = self.expect(T_OID)?;
        Oid::decode(t.contents(self.b))
    }
    /// a value of a variable binding (any of its types)
    pub fn value(&mut self) -> Result<Value, Bad> {
        let t = self.tlv()?;
        Value::read(&t, self.b)
    }
}

// ---------------------------------------------------------------- writing

/// the length octets of `n` in their shortest form
pub fn length_bytes(n: usize) -> Vec<u8> {
    if n < 0x80 {
        return alloc::vec![n as u8];
    }
    let b = (n as u32).to_be_bytes();
    let mut i = 0;
    while b[i] == 0 {
        i += 1;
    }
    let mut v = alloc::vec![0x80 | (4 - i) as u8];
    v.extend_from_slice(&b[i..]);
    v
}

/// one element as bytes
pub fn tlv(tag: u8, contents: &[u8]) -> Vec<u8> {
    let mut v = alloc::vec![tag];
    v.extend_from_slice(&length_bytes(contents.len()));
    v.extend_from_slice(contents);
    v
}

/// builds BER: elements one after the other, constructed ones between begin and end
#[derive(Clone, Debug, Default)]
pub struct Writer {
    buf: Vec<u8>,
    /// the open constructed elements: their tag octet's offset
    open: Vec<usize>,
}

impl Writer {
    pub fn new() -> Writer {
        Writer::default()
    }
    fn put(&mut self, tag: u8, c: &[u8]) {
        self.buf.push(tag);
        self.buf.extend_from_slice(&length_bytes(c.len()));
        self.buf.extend_from_slice(c);
    }
    pub fn int(&mut self, v: i64) {
        self.put(T_INTEGER, &int_bytes(v));
    }
    /// an unsigned value under its application tag (Counter32, Gauge32, TimeTicks, Counter64)
    pub fn uint(&mut self, tag: u8, v: u64) {
        self.put(tag, &uint_bytes(v));
    }
    pub fn octets(&mut self, b: &[u8]) {
        self.put(T_OCTETS, b);
    }
    pub fn null(&mut self) {
        self.put(T_NULL, &[]);
    }
    pub fn oid(&mut self, o: &Oid) {
        self.put(T_OID, &o.encode());
    }
    pub fn value(&mut self, v: &Value) {
        self.put(v.tag(), &v.contents());
    }
    /// bytes that are BER already
    pub fn raw(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    /// opens a constructed element (SEQUENCE 0x30, a PDU 0xA0 ...)
    pub fn begin(&mut self, tag: u8) {
        self.open.push(self.buf.len());
        self.buf.push(tag);
    }
    /// closes the element begin() opened last: its length goes in after its tag
    pub fn end(&mut self) {
        let Some(at) = self.open.pop() else {
            return;
        };
        let n = self.buf.len() - at - 1;
        let l = length_bytes(n);
        let tail = self.buf.split_off(at + 1);
        self.buf.extend_from_slice(&l);
        self.buf.extend_from_slice(&tail);
    }
    /// the bytes (every element begin() opened is closed first)
    pub fn into_vec(mut self) -> Vec<u8> {
        while !self.open.is_empty() {
            self.end();
        }
        self.buf
    }
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

/// an OCTET STRING as text when every octet is printable ASCII (tab and line ends allowed)
pub fn printable(b: &[u8]) -> Option<String> {
    let ok = b.iter().all(|&c| (0x20..0x7F).contains(&c) || c == b'\t' || c == b'\n' || c == b'\r');
    ok.then(|| String::from_utf8_lossy(b).into_owned())
}
