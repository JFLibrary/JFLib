// der -- the DER subset X.509 needs: a bounded reader (definite lengths, no indefinite
// forms, at most 64 KiB per element) and a writer
//
//   Der::new(bytes).read(tag) -> Option<(content, rest)>, peek_tag, int, oid, ...
//   writer: tlv(tag, content), seq(&[parts]), int_u(bytes), oid(&[arcs]), utf8(s) ...

use alloc::vec::Vec;

pub const BOOLEAN: u8 = 0x01;
pub const INTEGER: u8 = 0x02;
pub const BIT_STRING: u8 = 0x03;
pub const OCTET_STRING: u8 = 0x04;
pub const NULL: u8 = 0x05;
pub const OID: u8 = 0x06;
pub const UTF8: u8 = 0x0c;
pub const PRINTABLE: u8 = 0x13;
pub const IA5: u8 = 0x16;
pub const UTC_TIME: u8 = 0x17;
pub const GEN_TIME: u8 = 0x18;
pub const SEQUENCE: u8 = 0x30;
pub const SET: u8 = 0x31;

/// a cursor over DER bytes
#[derive(Clone, Copy)]
pub struct Der<'a> {
    pub b: &'a [u8],
}

impl<'a> Der<'a> {
    pub fn new(b: &'a [u8]) -> Der<'a> {
        Der { b }
    }

    pub fn is_empty(&self) -> bool {
        self.b.is_empty()
    }

    pub fn peek_tag(&self) -> Option<u8> {
        self.b.first().copied()
    }

    /// the next element: (tag, content, the whole element)
    pub fn next(&mut self) -> Option<(u8, &'a [u8], &'a [u8])> {
        let b = self.b;
        let tag = *b.first()?;
        if tag & 0x1f == 0x1f {
            return None; // multi-byte tags do not occur in X.509
        }
        let l0 = *b.get(1)?;
        let (len, hdr) = if l0 < 0x80 {
            (l0 as usize, 2)
        } else {
            let n = (l0 & 0x7f) as usize;
            if n == 0 || n > 3 {
                return None; // indefinite or over 16 MiB
            }
            let mut len = 0usize;
            for i in 0..n {
                len = len << 8 | *b.get(2 + i)? as usize;
            }
            if len < 0x80 || (n > 1 && len >> ((n - 1) * 8) == 0) {
                return None; // not the shortest form (DER)
            }
            (len, 2 + n)
        };
        let end = hdr.checked_add(len)?;
        let content = b.get(hdr..end)?;
        let whole = &b[..end];
        self.b = &b[end..];
        Some((tag, content, whole))
    }

    /// the next element with this tag -> its content
    pub fn read(&mut self, tag: u8) -> Option<&'a [u8]> {
        let save = *self;
        match self.next() {
            Some((t, c, _)) if t == tag => Some(c),
            _ => {
                *self = save;
                None
            }
        }
    }

    /// the next element with this tag -> the whole element (for signatures over it)
    pub fn read_raw(&mut self, tag: u8) -> Option<&'a [u8]> {
        let save = *self;
        match self.next() {
            Some((t, _, w)) if t == tag => Some(w),
            _ => {
                *self = save;
                None
            }
        }
    }

    /// an optional element with this tag
    pub fn optional(&mut self, tag: u8) -> Option<&'a [u8]> {
        if self.peek_tag() == Some(tag) { self.read(tag) } else { None }
    }

    pub fn seq(&mut self) -> Option<Der<'a>> {
        self.read(SEQUENCE).map(Der::new)
    }

    /// a non-negative INTEGER up to 64 bits
    pub fn u64(&mut self) -> Option<u64> {
        let c = self.read(INTEGER)?;
        if c.is_empty() || c[0] & 0x80 != 0 || c.len() > 9 || (c.len() == 9 && c[0] != 0) {
            return None;
        }
        Some(c.iter().fold(0u64, |a, &x| a << 8 | x as u64))
    }

    pub fn bool(&mut self) -> Option<bool> {
        let c = self.read(BOOLEAN)?;
        (c.len() == 1).then(|| c[0] != 0)
    }

    /// a BIT STRING without unused bits -> its bytes
    pub fn bits(&mut self) -> Option<&'a [u8]> {
        let c = self.read(BIT_STRING)?;
        (c.first() == Some(&0)).then(|| &c[1..])
    }
}

/// the encoded OID content of dotted arcs
pub fn oid_bytes(arcs: &[u64]) -> Vec<u8> {
    let mut out = Vec::new();
    if arcs.len() < 2 {
        return out;
    }
    let mut push = |mut v: u64| {
        let mut tmp = [0u8; 10];
        let mut n = 0;
        loop {
            tmp[n] = (v & 0x7f) as u8;
            n += 1;
            v >>= 7;
            if v == 0 {
                break;
            }
        }
        for i in (0..n).rev() {
            out.push(tmp[i] | if i > 0 { 0x80 } else { 0 });
        }
    };
    push(arcs[0] * 40 + arcs[1]);
    for &a in &arcs[2..] {
        push(a);
    }
    out
}

// ------------------------------------------------------------------ writing

pub fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len() + 5);
    out.push(tag);
    let n = content.len();
    if n < 0x80 {
        out.push(n as u8);
    } else if n < 0x100 {
        out.extend_from_slice(&[0x81, n as u8]);
    } else if n < 0x10000 {
        out.extend_from_slice(&[0x82, (n >> 8) as u8, n as u8]);
    } else {
        out.extend_from_slice(&[0x83, (n >> 16) as u8, (n >> 8) as u8, n as u8]);
    }
    out.extend_from_slice(content);
    out
}

pub fn seq(parts: &[&[u8]]) -> Vec<u8> {
    tlv(SEQUENCE, &parts.concat())
}

pub fn set(parts: &[&[u8]]) -> Vec<u8> {
    tlv(SET, &parts.concat())
}

/// an INTEGER of unsigned big-endian bytes (leading zeros dropped, a zero byte added when
/// the top bit is set)
pub fn int_u(b: &[u8]) -> Vec<u8> {
    let mut i = 0;
    while i + 1 < b.len() && b[i] == 0 {
        i += 1;
    }
    let b = &b[i..];
    let mut c = Vec::with_capacity(b.len() + 1);
    if b.first().is_some_and(|x| x & 0x80 != 0) {
        c.push(0);
    }
    c.extend_from_slice(b);
    if c.is_empty() {
        c.push(0);
    }
    tlv(INTEGER, &c)
}

pub fn small_int(v: u64) -> Vec<u8> {
    int_u(&v.to_be_bytes())
}

pub fn oid(arcs: &[u64]) -> Vec<u8> {
    tlv(OID, &oid_bytes(arcs))
}

pub fn utf8(s: &str) -> Vec<u8> {
    tlv(UTF8, s.as_bytes())
}

pub fn octets(b: &[u8]) -> Vec<u8> {
    tlv(OCTET_STRING, b)
}

pub fn bits(b: &[u8]) -> Vec<u8> {
    let mut c = Vec::with_capacity(b.len() + 1);
    c.push(0);
    c.extend_from_slice(b);
    tlv(BIT_STRING, &c)
}

pub fn boolean(v: bool) -> Vec<u8> {
    tlv(BOOLEAN, &[if v { 0xff } else { 0 }])
}

/// [n] EXPLICIT
pub fn explicit(n: u8, content: &[u8]) -> Vec<u8> {
    tlv(0xa0 | n, content)
}
