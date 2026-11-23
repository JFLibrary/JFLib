// csv -- delimited text: RFC 4180 and what Excel writes
//
//   read(bytes, &Options) -> Csv         decode, find the delimiter, split into records
//   decode(bytes) -> (String, Encoding)  a BOM decides; else valid UTF-8; else Windows-1252
//   detect_delimiter(text) -> u8         ; , TAB or |
//   to_table(&csv, header, infer) -> Table   numbers, booleans and dates typed on request
//
// The grammar: a field in quotes may hold the delimiter, line breaks and "" for a quote; records
// end with CR LF, LF or CR; the last record may end without a line break, and a line break at
// the very end starts no further record; an empty line is a record of one empty field.  Lenient
// where Excel is: a quote inside a field that does not start with one is kept as it is (ab"cd),
// text after a closing quote is kept and the field goes on to the next delimiter ("ab"cd is
// abcd), a quote left open runs to the end of the text.  Line breaks inside quotes are read as
// LF (CR LF and CR too), as Excel keeps them in a cell.  A first line "sep=;" (Excel's hint) sets
// the delimiter unless Options gives one, and is never a record.
//
// Encodings: a BOM (UTF-8, UTF-16 LE / BE: Excel's "Unicode text" is UTF-16 LE with TAB),
// UTF-16 without a BOM when every second byte of the start is zero, valid UTF-8, else
// Windows-1252 as WHATWG defines it (0x80 is the euro sign; the five bytes it leaves open are the
// C1 controls of the same number).  Decoding never fails: a forced encoding turns what it cannot
// read into U+FFFD.
//
// Untrusted input: CsvLimits bounds the bytes, records, fields of a record, fields in all and the
// bytes of one field; nothing recurses.

use alloc::string::String;
use alloc::vec::Vec;

use crate::{column_names, ole_day, parse_iso, parse_time, plain_names, Table, Value};

/// The text encoding of a CSV file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// UTF-8 without a byte-order mark
    Utf8,
    /// UTF-8 with a byte-order mark (Excel's "CSV UTF-8")
    Utf8Bom,
    /// UTF-16 little-endian (Excel's "Unicode text", with BOM)
    Utf16Le,
    /// UTF-16 big-endian
    Utf16Be,
    /// Windows-1252, the ANSI code page of Western European Windows (Excel's plain "CSV")
    Windows1252,
}

/// What a CSV file may hold.  The default: 256 MiB, two million records, Excel's 16384 fields a
/// record, eight million fields in all, 1 MiB a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CsvLimits {
    /// bytes of the file
    pub bytes: usize,
    /// records (lines)
    pub records: usize,
    /// fields of one record
    pub fields: usize,
    /// fields of all records together (each costs memory even when empty)
    pub cells: usize,
    /// bytes of one field (UTF-8)
    pub field_bytes: usize,
}

impl CsvLimits {
    /// The default limits (see `CsvLimits`).
    pub const DEFAULT: CsvLimits = CsvLimits { bytes: 256 << 20, records: 2_000_000, fields: 16_384, cells: 8_000_000, field_bytes: 1 << 20 };
}

impl Default for CsvLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// How a CSV file is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// the delimiter (an ASCII byte); None: a `sep=` line, else `detect_delimiter`
    pub delimiter: Option<u8>,
    /// the encoding; None: `decode` detects it
    pub encoding: Option<Encoding>,
    /// the quote character (an ASCII byte), `"` as a rule
    pub quote: u8,
    /// the bounds of the file
    pub limits: CsvLimits,
}

impl Options {
    /// Detect delimiter and encoding, quote with `"`, the default limits.
    pub const DEFAULT: Options = Options { delimiter: None, encoding: None, quote: b'"', limits: CsvLimits::DEFAULT };
}

impl Default for Options {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A CSV file read: its records of fields, as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Csv {
    /// the delimiter used
    pub delimiter: u8,
    /// the encoding read
    pub encoding: Encoding,
    /// the records, each a list of fields (records may differ in length)
    pub records: Vec<Vec<String>>,
}

/// Why a CSV file was not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// over a limit: the name of the field of `CsvLimits`
    Limit(&'static str),
    /// a delimiter or quote that is no ASCII byte, or both the same
    Options,
}

impl Error {
    /// The error as a sentence in English.
    pub fn text(&self) -> String {
        match self {
            Error::Limit(what) => alloc::format!("CSV: over the limit {what}"),
            Error::Options => String::from("CSV: the delimiter and the quote must be two different ASCII characters other than CR and LF"),
        }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.write_str(&self.text())
    }
}

// Windows-1252 0x80..0x9F (WHATWG: the bytes it leaves open are the C1 controls)
const CP1252: [u16; 32] = [
    0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008D, 0x017D, 0x008F,
    0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178,
];

fn windows1252(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() + b.len() / 8);
    for &c in b {
        let u = if (0x80..0xA0).contains(&c) { CP1252[(c - 0x80) as usize] as u32 } else { c as u32 };
        s.push(char::from_u32(u).unwrap_or('\u{fffd}'));
    }
    s
}

fn utf16(b: &[u8], le: bool) -> String {
    let units = b.chunks(2).map(|p| match (p, le) {
        ([a, b], true) => u16::from_le_bytes([*a, *b]),
        ([a, b], false) => u16::from_be_bytes([*a, *b]),
        _ => 0xFFFD, // an odd byte at the end
    });
    char::decode_utf16(units).map(|c| c.unwrap_or('\u{fffd}')).collect()
}

// UTF-16 without a BOM: in the first 64 code units the high (LE) or low (BE) bytes are zero
// and the others mostly not, as plain text in a Latin script writes it
fn utf16_guess(b: &[u8]) -> Option<bool> {
    if b.len() < 4 || b.len() % 2 != 0 {
        return None;
    }
    let n = (b.len() / 2).min(64);
    let (mut even0, mut odd0) = (0, 0);
    for i in 0..n {
        even0 += (b[2 * i] == 0) as usize;
        odd0 += (b[2 * i + 1] == 0) as usize;
    }
    if odd0 == n && even0 * 4 < n {
        Some(true)
    } else if even0 == n && odd0 * 4 < n {
        Some(false)
    } else {
        None
    }
}

/// Decode a CSV file: a byte-order mark decides (UTF-8, UTF-16 LE, UTF-16 BE); else UTF-16
/// when every second byte of the start is zero; else UTF-8 when it is valid; else
/// Windows-1252.  The BOM is not part of the text.
pub fn decode(bytes: &[u8]) -> (String, Encoding) {
    if let Some(rest) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        return (String::from_utf8_lossy(rest).into_owned(), Encoding::Utf8Bom);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        return (utf16(rest, true), Encoding::Utf16Le);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xfe, 0xff]) {
        return (utf16(rest, false), Encoding::Utf16Be);
    }
    if let Some(le) = utf16_guess(bytes) {
        return (utf16(bytes, le), if le { Encoding::Utf16Le } else { Encoding::Utf16Be });
    }
    match core::str::from_utf8(bytes) {
        Ok(s) => (String::from(s), Encoding::Utf8),
        Err(_) => (windows1252(bytes), Encoding::Windows1252),
    }
}

/// Decode in a given encoding (a matching BOM is skipped; what is not valid becomes U+FFFD).
pub fn decode_as(bytes: &[u8], encoding: Encoding) -> String {
    match encoding {
        Encoding::Utf8 | Encoding::Utf8Bom => String::from_utf8_lossy(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes)).into_owned(),
        Encoding::Utf16Le => utf16(bytes.strip_prefix(&[0xff, 0xfe]).unwrap_or(bytes), true),
        Encoding::Utf16Be => utf16(bytes.strip_prefix(&[0xfe, 0xff]).unwrap_or(bytes), false),
        Encoding::Windows1252 => windows1252(bytes),
    }
}

// Excel's first line "sep=X" (any case): X and the text after the line
fn sep_hint(text: &str) -> Option<(u8, &str)> {
    let b = text.as_bytes();
    if b.len() < 5 || !b[..4].eq_ignore_ascii_case(b"sep=") || !b[4].is_ascii() || matches!(b[4], b'\r' | b'\n') {
        return None;
    }
    let rest = &text[5..];
    let rest = if let Some(r) = rest.strip_prefix("\r\n") {
        r
    } else if let Some(r) = rest.strip_prefix(['\r', '\n']) {
        r
    } else if rest.is_empty() {
        rest
    } else {
        return None;
    };
    Some((b[4], rest))
}

// the delimiters detect_delimiter considers, in the order ties are broken
const CANDIDATES: [u8; 4] = [b'\t', b';', b',', b'|'];

/// The delimiter of a CSV text: a first line `sep=X` names it; else, over the first 20
/// non-empty records (quotes respected, so a quoted delimiter or line break does not count), a
/// candidate that occurs the same number of times, at least once, in every one of them wins, in
/// the order TAB, `;`, `,`, `|` (a TAB is rarely data; German Excel writes `;` and `,` in its
/// numbers, so `;` goes before `,`); when none is that consistent, the one found in the most
/// records (the same order breaks ties); `,` when there is none at all.
pub fn detect_delimiter(text: &str) -> u8 {
    detect(text, b'"')
}

fn detect(text: &str, quote: u8) -> u8 {
    if let Some((d, _)) = sep_hint(text) {
        return d;
    }
    let b = text.as_bytes();
    let mut counts: Vec<[usize; 4]> = Vec::new();
    let mut now = [0usize; 4];
    let mut blank = true;
    let mut quoted = false;
    let mut i = 0;
    while i < b.len() && counts.len() < 20 {
        let c = b[i];
        if c == quote {
            quoted = !quoted;
            blank = false;
        } else if !quoted && (c == b'\n' || c == b'\r') {
            if !blank {
                counts.push(now);
            }
            now = [0; 4];
            blank = true;
            if c == b'\r' && b.get(i + 1) == Some(&b'\n') {
                i += 1;
            }
        } else {
            blank = false;
            if !quoted {
                if let Some(k) = CANDIDATES.iter().position(|&d| d == c) {
                    now[k] += 1;
                }
            }
        }
        i += 1;
    }
    if !blank && counts.len() < 20 {
        counts.push(now);
    }
    if counts.is_empty() {
        return b',';
    }
    for k in 0..CANDIDATES.len() {
        let n = counts[0][k];
        if n > 0 && counts.iter().all(|c| c[k] == n) {
            return CANDIDATES[k];
        }
    }
    let mut best = (0usize, b',');
    for (k, &d) in CANDIDATES.iter().enumerate() {
        let lines = counts.iter().filter(|c| c[k] > 0).count();
        if lines > best.0 {
            best = (lines, d);
        }
    }
    best.1
}

/// Read a CSV file (see the module's head for the grammar).
pub fn read(bytes: &[u8], opts: &Options) -> Result<Csv, Error> {
    let lim = &opts.limits;
    if bytes.len() > lim.bytes {
        return Err(Error::Limit("bytes"));
    }
    let bad = |c: u8| !c.is_ascii() || c == b'\r' || c == b'\n';
    if bad(opts.quote) || opts.delimiter.is_some_and(|d| bad(d) || d == opts.quote) {
        return Err(Error::Options);
    }
    let (text, encoding) = match opts.encoding {
        Some(e) => (decode_as(bytes, e), e),
        None => decode(bytes),
    };
    let mut body = text.as_str();
    let mut delimiter = opts.delimiter;
    if let Some((d, rest)) = sep_hint(body) {
        if delimiter.is_none() && !bad(d) && d != opts.quote {
            delimiter = Some(d);
        }
        body = rest;
    }
    let delimiter = delimiter.unwrap_or_else(|| detect(body, opts.quote));
    let records = split(body, delimiter, opts.quote, lim)?;
    Ok(Csv { delimiter, encoding, records })
}

// a quoted field's text: line breaks as LF
fn push_lf(out: &mut String, s: &str) {
    if s.contains('\r') {
        out.push_str(&s.replace("\r\n", "\n").replace('\r', "\n"));
    } else {
        out.push_str(s);
    }
}

fn split(s: &str, delim: u8, quote: u8, lim: &CsvLimits) -> Result<Vec<Vec<String>>, Error> {
    let b = s.as_bytes();
    let n = b.len();
    let mut records: Vec<Vec<String>> = Vec::new();
    if n == 0 {
        return Ok(records);
    }
    let stop = |c: u8| c == delim || c == b'\r' || c == b'\n';
    let mut record: Vec<String> = Vec::new();
    let mut cells = 0usize;
    let mut i = 0;
    loop {
        let mut field = String::new();
        if i < n && b[i] == quote {
            i += 1;
            loop {
                match b[i..].iter().position(|&c| c == quote) {
                    None => {
                        push_lf(&mut field, &s[i..]);
                        i = n;
                        break;
                    }
                    Some(k) => {
                        push_lf(&mut field, &s[i..i + k]);
                        i += k + 1;
                        if b.get(i) == Some(&quote) {
                            field.push(quote as char);
                            i += 1;
                        } else {
                            break;
                        }
                    }
                }
                if field.len() > lim.field_bytes {
                    return Err(Error::Limit("field_bytes"));
                }
            }
            // text after the closing quote belongs to the field
            let k = b[i..].iter().position(|&c| stop(c)).map_or(n, |k| i + k);
            field.push_str(&s[i..k]);
            i = k;
        } else {
            let k = b[i..].iter().position(|&c| stop(c)).map_or(n, |k| i + k);
            field.push_str(&s[i..k]);
            i = k;
        }
        if field.len() > lim.field_bytes {
            return Err(Error::Limit("field_bytes"));
        }
        cells += 1;
        if cells > lim.cells {
            return Err(Error::Limit("cells"));
        }
        record.push(field);
        if record.len() > lim.fields {
            return Err(Error::Limit("fields"));
        }
        if i >= n {
            break;
        }
        if b[i] == delim {
            i += 1;
            continue;
        }
        // a line break ends the record
        i += if b[i] == b'\r' && b.get(i + 1) == Some(&b'\n') { 2 } else { 1 };
        if records.len() >= lim.records {
            return Err(Error::Limit("records"));
        }
        records.push(core::mem::take(&mut record));
        if i >= n {
            return Ok(records);
        }
    }
    if records.len() >= lim.records {
        return Err(Error::Limit("records"));
    }
    records.push(record);
    Ok(records)
}

/// The records as a Table.  `header`: the first record names the columns (see
/// `Table::columns`).  The table is as wide as the longest record: shorter records are padded
/// with Empty, nothing is cut off; with a header, columns past it that are empty in every
/// record are dropped (a delimiter at the end of the data lines).  Empty records at the end are
/// dropped.  An empty field is Empty.  `infer`: the other fields are typed, else they are Text:
///
/// - numbers: `1.5`, `-3`, `+2`, `.5`, `1e3`; with `;` as delimiter also German ones, `1,5`,
///   `1.234,5`, `-1.234.567,89` (a field with a comma reads `.` as the thousands separator, in
///   groups of three; without a comma `1.234` is 1.234).  A number with a leading zero (`007`,
///   `0123`) stays Text: it is an identifier more often than not.  White space at the ends is
///   ignored.
/// - booleans: TRUE, FALSE, WAHR, FALSCH in any case
/// - dates: ISO 8601 (`2026-11-23`, `2026-11-23T14:05`, `2026-11-23 14:05:00`) and German
///   `d.m.yyyy` (`23.11.2026`, `1.2.2026 14:05`) as `Value::Date`
pub fn to_table(csv: &Csv, header: bool, infer: bool) -> Table {
    let mut records: &[Vec<String>] = &csv.records;
    while records.last().is_some_and(|r| r.iter().all(String::is_empty)) {
        records = &records[..records.len() - 1];
    }
    let (head, data) = match (header, records.split_first()) {
        (true, Some((h, rest))) => (Some(h), rest),
        _ => (None, records),
    };
    let mut width = data.iter().map(Vec::len).max().unwrap_or(0).max(head.map_or(0, Vec::len));
    if let Some(h) = head {
        // the columns past the header that hold nothing
        while width > h.len() && data.iter().all(|r| r.get(width - 1).is_none_or(String::is_empty)) {
            width -= 1;
        }
    }
    let german = csv.delimiter == b';';
    let rows = data
        .iter()
        .map(|r| {
            let mut row: Vec<Value> = r.iter().take(width).map(|f| field_value(f, infer, german)).collect();
            row.resize(width, Value::Empty);
            row
        })
        .collect();
    let columns = match head {
        Some(h) => {
            let mut names: Vec<String> = h.iter().take(width).cloned().collect();
            names.resize(width, String::new());
            column_names(names)
        }
        None => plain_names(width),
    };
    Table { columns, rows }
}

fn field_value(f: &str, infer: bool, german: bool) -> Value {
    if f.is_empty() {
        return Value::Empty;
    }
    if infer {
        if let Some(v) = infer_value(f.trim_matches([' ', '\t']), german) {
            return v;
        }
    }
    Value::Text(String::from(f))
}

fn infer_value(t: &str, german: bool) -> Option<Value> {
    if t.is_empty() {
        return None;
    }
    for (word, v) in [("TRUE", true), ("FALSE", false), ("WAHR", true), ("FALSCH", false)] {
        if t.eq_ignore_ascii_case(word) {
            return Some(Value::Bool(v));
        }
    }
    if let Some(n) = number(t, german) {
        return Some(Value::Number(n));
    }
    if let Some(d) = parse_iso(t, false).or_else(|| german_date(t)) {
        return Some(Value::Date(d));
    }
    None
}

// a number as written above (to_table); None for anything else
fn number(t: &str, german: bool) -> Option<f64> {
    let b = t.as_bytes();
    let body = b.strip_prefix(b"-").or_else(|| b.strip_prefix(b"+")).unwrap_or(b);
    let plain: Vec<u8> = if german && body.contains(&b',') {
        // d{1,3}(.ddd)* or d+, then , and digits
        let comma = body.iter().position(|&c| c == b',')?;
        let (int, frac) = (&body[..comma], &body[comma + 1..]);
        if frac.is_empty() || !frac.iter().all(u8::is_ascii_digit) {
            return None;
        }
        let groups: Vec<&[u8]> = int.split(|&c| c == b'.').collect();
        let grouped = groups.len() > 1 && (1..=3).contains(&groups[0].len()) && groups[1..].iter().all(|g| g.len() == 3);
        if !(groups.len() == 1 || grouped) || !groups.iter().all(|g| !g.is_empty() && g.iter().all(u8::is_ascii_digit)) {
            return None;
        }
        let mut v: Vec<u8> = groups.concat();
        v.push(b'.');
        v.extend_from_slice(frac);
        v
    } else {
        // digits, an optional point and digits, an optional exponent
        let mut i = 0;
        let int = body.iter().take_while(|c| c.is_ascii_digit()).count();
        i += int;
        let mut frac = 0;
        if body.get(i) == Some(&b'.') {
            i += 1;
            frac = body[i..].iter().take_while(|c| c.is_ascii_digit()).count();
            i += frac;
        }
        if int + frac == 0 {
            return None;
        }
        if matches!(body.get(i), Some(b'e' | b'E')) {
            i += 1;
            if matches!(body.get(i), Some(b'+' | b'-')) {
                i += 1;
            }
            let e = body[i..].iter().take_while(|c| c.is_ascii_digit()).count();
            if e == 0 {
                return None;
            }
            i += e;
        }
        if i != body.len() {
            return None;
        }
        body.to_vec()
    };
    // a leading zero before more digits: an identifier (007), not a number
    if plain.len() > 1 && plain[0] == b'0' && plain[1].is_ascii_digit() {
        return None;
    }
    let (v, used) = base::f64_parse(&plain);
    if used != plain.len() || !v.is_finite() {
        return None;
    }
    Some(if b.first() == Some(&b'-') { -v } else { v })
}

// d.m.yyyy with an optional " HH:MM[:SS]"
fn german_date(t: &str) -> Option<f64> {
    let (date, time) = match t.split_once(' ') {
        Some((d, tm)) => (d, Some(tm.trim_start())),
        None => (t, None),
    };
    let mut parts = date.split('.');
    let (d, m, y) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || !(1..=2).contains(&d.len()) || !(1..=2).contains(&m.len()) || y.len() != 4 {
        return None;
    }
    let all = |s: &str| s.bytes().all(|c| c.is_ascii_digit());
    if !all(d) || !all(m) || !all(y) {
        return None;
    }
    let day = ole_day(y.parse().ok()?, m.parse().ok()?, d.parse().ok()?)?;
    match time {
        None => Some(day),
        // a one-digit hour as German writes it (9:05)
        Some(tm) if tm.find(':') == Some(1) => Some(day + parse_time(&alloc::format!("0{tm}"))?),
        Some(tm) => Some(day + parse_time(tm)?),
    }
}
