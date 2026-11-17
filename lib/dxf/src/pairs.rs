// pairs -- a DXF file as group pairs (a code line, a value line), read from a source piece by piece: lines end in LF,
// CR LF or a lone CR; a code line is a decimal integer between spaces; a value keeps its leading spaces (text), loses
// its line end.  A line longer than Limits::line_bytes is an error, so the window never grows past it.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use crate::{Error, Result};

/// the window the source is read into, and how much it grows at most beyond a line
const CHUNK: usize = 1 << 20;

pub(crate) struct Pairs<'s> {
    src: &'s mut dyn FnMut(&mut [u8]) -> usize,
    buf: Vec<u8>,
    pos: usize,
    end: usize,
    eof: bool,
    max_line: usize,
    /// lines and bytes taken
    pub lines: u64,
    pub bytes: u64,
    /// the current pair: its code and value (valid until the next `next`)
    pub code: i32,
    vstart: usize,
    vend: usize,
}

impl<'s> Pairs<'s> {
    pub fn new(src: &'s mut dyn FnMut(&mut [u8]) -> usize, max_line: usize) -> Pairs<'s> {
        Pairs { src, buf: vec![0u8; CHUNK], pos: 0, end: 0, eof: false, max_line: max_line.max(16), lines: 0, bytes: 0, code: 0, vstart: 0, vend: 0 }
    }

    /// the first bytes of the file (for the binary sentinel and a byte order mark), not consumed
    pub fn peek(&mut self, n: usize) -> &[u8] {
        while self.end - self.pos < n && !self.eof {
            self.fill();
        }
        &self.buf[self.pos..(self.pos + n).min(self.end)]
    }

    pub fn skip(&mut self, n: usize) {
        let n = n.min(self.end - self.pos);
        self.pos += n;
        self.bytes += n as u64;
    }

    /// more bytes into the window: what is left moves to its start, the window grows only for a line longer than it
    fn fill(&mut self) {
        if self.eof {
            return;
        }
        if self.pos > 0 {
            self.buf.copy_within(self.pos..self.end, 0);
            self.end -= self.pos;
            // (the value of the current pair moves with it: a line is read before the next one, see `line`)
            self.vstart = self.vstart.saturating_sub(self.pos);
            self.vend = self.vend.saturating_sub(self.pos);
            self.pos = 0;
        }
        if self.end == self.buf.len() {
            let grown = (self.buf.len() * 2).min(self.max_line + CHUNK);
            if grown <= self.buf.len() {
                return;
            }
            self.buf.resize(grown, 0);
        }
        let end = self.end;
        let n = (self.src)(&mut self.buf[end..]);
        let n = n.min(self.buf.len() - end);
        if n == 0 {
            self.eof = true;
        }
        self.end += n;
    }

    /// the next line as a range of the window (without its line end); None at the end of the file
    fn line(&mut self) -> Result<Option<(usize, usize)>> {
        let mut from = self.pos;
        loop {
            // the line end after `from`
            let mut i = from;
            while i < self.end {
                let b = self.buf[i];
                if b == b'\n' || b == b'\r' {
                    break;
                }
                i += 1;
            }
            if i < self.end {
                let b = self.buf[i];
                if b == b'\r' && i + 1 == self.end && !self.eof {
                    // a CR at the window's end: LF may follow
                    let shift = self.pos;
                    self.fill();
                    from = i - shift;
                    continue;
                }
                let start = self.pos;
                let next = if b == b'\r' && i + 1 < self.end && self.buf[i + 1] == b'\n' { i + 2 } else { i + 1 };
                if i - start > self.max_line {
                    return Err(Error::Limit(format!("line {}: longer than {} bytes", self.lines + 1, self.max_line)));
                }
                self.bytes += (next - start) as u64;
                self.lines += 1;
                self.pos = next;
                return Ok(Some((start, i)));
            }
            if self.eof {
                if self.pos < self.end {
                    // the last line without a line end
                    let start = self.pos;
                    let i = self.end;
                    if i - start > self.max_line {
                        return Err(Error::Limit(format!("line {}: longer than {} bytes", self.lines + 1, self.max_line)));
                    }
                    self.bytes += (i - start) as u64;
                    self.lines += 1;
                    self.pos = i;
                    return Ok(Some((start, i)));
                }
                return Ok(None);
            }
            if self.end - self.pos > self.max_line {
                return Err(Error::Limit(format!("line {}: longer than {} bytes", self.lines + 1, self.max_line)));
            }
            let shift = self.pos;
            self.fill();
            from = i - shift;
        }
    }

    /// the next pair; false at the end of the file (also after a code line without its value)
    pub fn next(&mut self) -> Result<bool> {
        let Some((a, b)) = self.line()? else { return Ok(false) };
        let code = match parse_code(&self.buf[a..b]) {
            Some(c) => c,
            None => {
                // blank lines at the very end of a file are no pair
                if trim(&self.buf[a..b]).is_empty() {
                    let at = self.lines;
                    loop {
                        match self.line()? {
                            None => return Ok(false),
                            Some((x, y)) if trim(&self.buf[x..y]).is_empty() => continue,
                            Some(_) => return Err(Error::Invalid(format!("line {at}: an empty group code"))),
                        }
                    }
                }
                return Err(Error::Invalid(format!("line {}: a group code that is no number", self.lines)));
            }
        };
        let Some((c, d)) = self.line()? else { return Ok(false) };
        self.code = code;
        self.vstart = c;
        self.vend = d;
        Ok(true)
    }

    /// the current value's bytes
    pub fn value(&self) -> &[u8] {
        &self.buf[self.vstart..self.vend]
    }
}

/// a group code: optional spaces, an optional sign, digits, optional spaces
pub(crate) fn parse_code(s: &[u8]) -> Option<i32> {
    let s = trim(s);
    let (neg, d) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    if d.is_empty() || d.len() > 9 || !d.iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut v = 0i32;
    for &b in d {
        v = v * 10 + (b - b'0') as i32;
    }
    Some(if neg { -v } else { v })
}

/// without ASCII white space at both ends
pub(crate) fn trim(s: &[u8]) -> &[u8] {
    let mut a = 0;
    let mut b = s.len();
    while a < b && s[a].is_ascii_whitespace() {
        a += 1;
    }
    while b > a && s[b - 1].is_ascii_whitespace() {
        b -= 1;
    }
    &s[a..b]
}
