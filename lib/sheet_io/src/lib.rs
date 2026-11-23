// sheet_io -- XLSX and CSV import: workbooks and delimited text as typed tables (JFDiagrammer JD18,
// FEATURES.md IM-05 / IM-10, decision F6: "CSV und .xlsx in P2 mit XLSX-Leser in JFLib")
//
//   xlsx::Workbook::open(bytes, Limits) -> Workbook     the workbook part, its relationships,
//        the shared strings and the styles (which number formats are dates); not the sheets yet
//   wb.sheets() / names() / date1904()                   sheets with state and kind, defined
//        names (global and sheet-local, _xlnm.* included) and tables (ListObjects) as names
//   wb.sheet(i) -> Grid                                  one worksheet, sparse: stored values
//        (a formula's cached result, formulas are never evaluated)
//   wb.table(&area, header) / wb.sheet_table(i, header) -> Table   a rectangle as a table
//   csv::read(bytes, &Options) -> Csv                    RFC 4180 and what Excel writes: the
//        encoding (BOM, UTF-8, else Windows-1252) and the delimiter (; , TAB |) detected
//   csv::to_table(&csv, header, infer) -> Table          with numbers, booleans and dates typed
//   Value, CellError, CellRef / parse_ref / col_name, Area / parse_area, civil / serial
//
// Why a crate of its own: one crate per foreign format family, like dxf, gltf and pdf.  Not
// doc_io: that is jfWord's document model, and every user of a spreadsheet reader would pull in
// doc_core, doc_layout and text.  Not archive: that holds containers (ZIP, OPC), not their
// content.  Not base: base::encoding is strict Unicode by design ("no guessed legacy code
// pages"), while CSV files from Excel come in Windows-1252.  CSV sits next to XLSX because both
// deliver the same typed Table.
//
// Every file is untrusted input: the ZIP container is bounded before anything is inflated
// (archive::Limits, the compression ratio included), the XML reader takes no DTD and no entity
// but the five predefined ones and bounds the nesting, rows, columns, cells, shared strings and
// their bytes are bounded, no size a file advertises (<dimension>, count, uniqueCount) sizes an
// allocation, cells are stored sparsely, nothing recurses.  A broken file is an error, never a
// panic.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub mod csv;
pub mod xlsx;
mod xml;

/// A calendar date and wall time (datetime's), what `civil` and `serial` convert.
pub use datetime::Civil;

/// Excel's rows per sheet: the last row of a whole-column reference (`A:C`)
pub const MAX_ROWS: u32 = 1_048_576;
/// Excel's columns per sheet (A to XFD): the last column of a whole-row reference (`1:3`)
pub const MAX_COLS: u32 = 16_384;

/// A cell value with its type.
///
/// `Date` holds an OLE Automation date, as Visio and Excel's 1900 date system use it: days since
/// 1899-12-30, the fraction is the time of day (values are taken as linear, also below zero).
/// Values from 0 to below 1 are a time of day without a date.  The XLSX reader converts the
/// 1904 date system and Excel's fake 1900-02-29 (see `xlsx`).
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    /// no value (an empty cell, a formula without a cached result)
    #[default]
    Empty,
    /// a number
    Number(f64),
    /// a text
    Text(String),
    /// TRUE or FALSE
    Bool(bool),
    /// a date and time as an OLE Automation date (days since 1899-12-30)
    Date(f64),
    /// an error value of a cell (#N/A, #DIV/0! ...)
    Error(CellError),
}

impl Value {
    /// The display form: numbers as the shortest text that reads back (`base::f64_format`,
    /// `-0` as `0`), dates as ISO 8601 (`2026-11-23`, `2026-11-23 14:05:00`, a time of day alone
    /// as `14:05:00`, milliseconds only when there are some: `14:05:00.250`), booleans as
    /// `TRUE` / `FALSE`, errors as Excel writes them (`#N/A`), Empty as "".
    pub fn text(&self) -> String {
        match self {
            Value::Empty => String::new(),
            Value::Number(v) => number_text(*v),
            Value::Text(s) => s.clone(),
            Value::Bool(b) => String::from(if *b { "TRUE" } else { "FALSE" }),
            Value::Date(v) => date_text(*v),
            Value::Error(e) => String::from(e.text()),
        }
    }
    /// Whether this is `Empty`.
    pub fn is_empty(&self) -> bool {
        matches!(self, Value::Empty)
    }
    /// The number of a Number or a Date (its serial), else None.
    pub fn number(&self) -> Option<f64> {
        match self {
            Value::Number(v) | Value::Date(v) => Some(*v),
            _ => None,
        }
    }
}

fn number_text(v: f64) -> String {
    if v == 0.0 { String::from("0") } else { base::f64_format(v) }
}

fn date_text(v: f64) -> String {
    let Some(c) = civil(v) else { return number_text(v) };
    let mut time = format!("{:02}:{:02}:{:02}", c.hour, c.minute, c.second);
    if c.nanos != 0 {
        time.push_str(&format!(".{:03}", c.nanos / 1_000_000));
    }
    if v >= 0.0 && (c.year, c.month, c.day) == (1899, 12, 30) {
        return time;
    }
    let date = format!("{:04}-{:02}-{:02}", c.year, c.month, c.day);
    if c.hour == 0 && c.minute == 0 && c.second == 0 && c.nanos == 0 { date } else { format!("{date} {time}") }
}

// days since 1970-01-01 of 1899-12-30, the day 0 of OLE dates
const OLE_EPOCH: i64 = -25_569;

/// The date and time of an OLE Automation date (days since 1899-12-30, the fraction the time of
/// day), rounded to the millisecond; None for NaN, infinities and values beyond 10^8 days.
pub fn civil(serial: f64) -> Option<Civil> {
    if !serial.is_finite() || serial.abs() > 1.0e8 {
        return None;
    }
    let x = serial * 86_400_000.0;
    let ms = if x >= 0.0 { (x + 0.5) as i64 } else { -((-x + 0.5) as i64) };
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let (year, month, day) = datetime::civil_from_days(days + OLE_EPOCH);
    Some(Civil {
        year,
        month,
        day,
        hour: (rem / 3_600_000) as u8,
        minute: (rem / 60_000 % 60) as u8,
        second: (rem / 1000 % 60) as u8,
        nanos: (rem % 1000) as u32 * 1_000_000,
    })
}

/// The OLE Automation date of a date and time (days since 1899-12-30, the fraction the time of
/// day); `civil` reads it back.
pub fn serial(c: &Civil) -> f64 {
    let days = datetime::days_from_civil(c.year, c.month, c.day) - OLE_EPOCH;
    let secs = c.hour as f64 * 3600.0 + c.minute as f64 * 60.0 + c.second as f64 + c.nanos as f64 / 1.0e9;
    days as f64 + secs / 86_400.0
}

// the OLE day of a valid date, None for an invalid one
pub(crate) fn ole_day(y: i32, m: u8, d: u8) -> Option<f64> {
    if !(1..=12).contains(&m) || d == 0 || d > datetime::days_in_month(y, m) {
        return None;
    }
    Some((datetime::days_from_civil(y, m, d) - OLE_EPOCH) as f64)
}

// "HH:MM", "HH:MM:SS", "HH:MM:SS.fff" (hours 0..23, a second of 60 refused) -> fraction of a day
pub(crate) fn parse_time(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let two = |i: usize| -> Option<u32> {
        let (x, y) = (*b.get(i)?, *b.get(i + 1)?);
        (x.is_ascii_digit() && y.is_ascii_digit()).then(|| ((x - b'0') * 10 + (y - b'0')) as u32)
    };
    let h = two(0)?;
    if b.get(2) != Some(&b':') {
        return None;
    }
    let m = two(3)?;
    let mut secs = 0.0;
    let mut i = 5;
    if b.get(5) == Some(&b':') {
        secs = two(6)? as f64;
        i = 8;
        if b.get(8) == Some(&b'.') {
            let frac = &s[9..];
            if frac.is_empty() || frac.len() > 9 || !frac.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let (f, used) = base::f64_parse(&b[8..]);
            if used != b.len() - 8 {
                return None;
            }
            secs += f;
            i = b.len();
        }
    }
    if i != b.len() || h > 23 || m > 59 || secs >= 60.0 {
        return None;
    }
    Some((h as f64 * 3600.0 + m as f64 * 60.0 + secs) / 86_400.0)
}

// ISO 8601 as Excel and CSV exports write it: "2026-11-23", "2026-11-23T14:05",
// "2026-11-23 14:05:00.5", a trailing "Z" (the wall time kept); `time_only`: also "14:05:00"
pub(crate) fn parse_iso(s: &str, time_only: bool) -> Option<f64> {
    let s = s.strip_suffix('Z').unwrap_or(s);
    let b = s.as_bytes();
    if time_only && b.get(2) == Some(&b':') {
        return parse_time(s);
    }
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let digits = |r: core::ops::Range<usize>| -> Option<u32> {
        let t = b.get(r)?;
        t.iter().all(u8::is_ascii_digit).then(|| t.iter().fold(0, |a, &c| a * 10 + (c - b'0') as u32))
    };
    let (y, m, d) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    let day = ole_day(y as i32, m as u8, d as u8)?;
    match b.get(10) {
        None => Some(day),
        Some(b'T' | b' ') => Some(day + parse_time(&s[11..])?),
        _ => None,
    }
}

/// An error value of a cell.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CellError {
    /// `#NULL!`
    Null,
    /// `#DIV/0!`
    Div0,
    /// `#VALUE!`
    Value,
    /// `#REF!`
    Ref,
    /// `#NAME?`
    Name,
    /// `#NUM!`
    Num,
    /// `#N/A`
    NA,
    /// `#GETTING_DATA`
    GettingData,
    /// `#SPILL!`
    Spill,
    /// `#CALC!`
    Calc,
    /// another error text, as written (`#BUSY!`, `#FIELD!` ...)
    Other(String),
}

impl CellError {
    /// The error as Excel writes it (`#N/A`).
    pub fn text(&self) -> &str {
        match self {
            CellError::Null => "#NULL!",
            CellError::Div0 => "#DIV/0!",
            CellError::Value => "#VALUE!",
            CellError::Ref => "#REF!",
            CellError::Name => "#NAME?",
            CellError::Num => "#NUM!",
            CellError::NA => "#N/A",
            CellError::GettingData => "#GETTING_DATA",
            CellError::Spill => "#SPILL!",
            CellError::Calc => "#CALC!",
            CellError::Other(s) => s,
        }
    }
    /// The error of a text (ASCII case-insensitive, white space at the ends ignored); any other
    /// text is `Other` with the text as written.
    pub fn parse(s: &str) -> CellError {
        const ALL: [CellError; 10] = [
            CellError::Null,
            CellError::Div0,
            CellError::Value,
            CellError::Ref,
            CellError::Name,
            CellError::Num,
            CellError::NA,
            CellError::GettingData,
            CellError::Spill,
            CellError::Calc,
        ];
        let t = s.trim();
        ALL.into_iter().find(|e| e.text().eq_ignore_ascii_case(t)).unwrap_or_else(|| CellError::Other(String::from(s)))
    }
}

/// A cell position, 0-based (`B3` is row 2, column 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct CellRef {
    /// the row, 0-based
    pub row: u32,
    /// the column, 0-based
    pub col: u32,
}

impl CellRef {
    /// A cell position from a 0-based row and column.
    pub const fn new(row: u32, col: u32) -> CellRef {
        CellRef { row, col }
    }
    /// The A1 form without `$` (`B3`).
    pub fn a1(&self) -> String {
        format!("{}{}", col_name(self.col), self.row as u64 + 1)
    }
}

/// The letters of a 0-based column (0 is `A`, 27 is `AB`, 16383 is `XFD`).
pub fn col_name(col: u32) -> String {
    let mut n = col as u64 + 1;
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        out.push(b'A' + (n % 26) as u8);
        n /= 26;
    }
    out.reverse();
    out.into_iter().map(char::from).collect()
}

// a column's letters (ASCII, either case) -> its 0-based index
fn letters(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut v: u64 = 0;
    for c in s.bytes() {
        v = v * 26 + (c.to_ascii_uppercase() - b'A') as u64 + 1;
        if v > u32::MAX as u64 + 1 {
            return None;
        }
    }
    Some((v - 1) as u32)
}

// a row's digits (1-based) -> its 0-based index
fn row_digits(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 10 || !s.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let v: u64 = s.parse().ok()?;
    if v == 0 || v > u32::MAX as u64 + 1 {
        return None;
    }
    Some((v - 1) as u32)
}

/// A cell reference in A1 form: column letters (either case), then the 1-based row, each with
/// an optional `$` (`B3`, `$B$3`, `b$3`); None for anything else.
pub fn parse_ref(s: &str) -> Option<CellRef> {
    let s = s.strip_prefix('$').unwrap_or(s);
    let split = s.find(|c: char| !c.is_ascii_alphabetic())?;
    let col = letters(&s[..split])?;
    let rest = &s[split..];
    let row = row_digits(rest.strip_prefix('$').unwrap_or(rest))?;
    Some(CellRef { row, col })
}

/// A rectangle of cells, `first` and `last` included, on a sheet (an index into the workbook's
/// sheets) or on no sheet in particular.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Area {
    /// the sheet, an index into `xlsx::Workbook::sheets`; None when the reference named none
    pub sheet: Option<usize>,
    /// the top left cell
    pub first: CellRef,
    /// the bottom right cell
    pub last: CellRef,
}

impl Area {
    /// The rectangle between two corners in any order.
    pub fn new(sheet: Option<usize>, a: CellRef, b: CellRef) -> Area {
        Area {
            sheet,
            first: CellRef { row: a.row.min(b.row), col: a.col.min(b.col) },
            last: CellRef { row: a.row.max(b.row), col: a.col.max(b.col) },
        }
    }
    /// Its rows.
    pub fn rows(&self) -> u64 {
        self.last.row as u64 - self.first.row as u64 + 1
    }
    /// Its columns.
    pub fn cols(&self) -> u64 {
        self.last.col as u64 - self.first.col as u64 + 1
    }
    /// Whether a cell lies inside.
    pub fn contains(&self, at: CellRef) -> bool {
        (self.first.row..=self.last.row).contains(&at.row) && (self.first.col..=self.last.col).contains(&at.col)
    }
    /// The A1 form without sheet and `$`: `B3` for one cell, `A1:C10` for more.
    pub fn a1(&self) -> String {
        if self.first == self.last { self.first.a1() } else { format!("{}:{}", self.first.a1(), self.last.a1()) }
    }
}

/// A sheet name as a formula writes it before `!`: as it is when it is a plain name (letters,
/// digits, `_`, `.`, not starting with a digit, no cell reference), else in single quotes with
/// each `'` doubled (`'Tab 1'`, `'O''Brien'`).
pub fn quote_sheet(name: &str) -> String {
    let plain = !name.is_empty()
        && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && parse_ref(name).is_none();
    if plain { String::from(name) } else { format!("'{}'", name.replace('\'', "''")) }
}

// one side of a range: a cell, a whole column (letters) or a whole row (digits)
#[derive(Clone, Copy, PartialEq)]
enum Side {
    Cell(CellRef),
    Col(u32),
    Row(u32),
}

fn side(s: &str) -> Option<Side> {
    if let Some(c) = parse_ref(s) {
        return Some(Side::Cell(c));
    }
    let t = s.strip_prefix('$').unwrap_or(s);
    if let Some(c) = letters(t) {
        return Some(Side::Col(c));
    }
    row_digits(t).map(Side::Row)
}

/// A reference to one rectangle as formulas and defined names write it: `B3`, `$A$1:$C$10`,
/// whole columns `A:C` (rows 0 to MAX_ROWS - 1), whole rows `1:3` (columns 0 to MAX_COLS - 1),
/// with an optional sheet before `!` (`Sheet1!A1`, `'Tab 1'!$A$1:$C$10`, `''` for a quote in a
/// quoted name) and an optional leading `=`.  The sheet is resolved by name among `sheets`
/// (ASCII and Unicode case-insensitive, as Excel compares sheet names); without a sheet the
/// area's `sheet` is None.  None for an unknown sheet, a 3-D reference (`Sheet1:Sheet3!A1`), an
/// external one (`[1]Sheet1!A1`), a union (`A1,B2`), a function or `#REF!`.
pub fn parse_area<S: AsRef<str>>(text: &str, sheets: &[S]) -> Option<Area> {
    let t = text.trim();
    let t = t.strip_prefix('=').unwrap_or(t).trim();
    let (sheet, rest) = if let Some(q) = t.strip_prefix('\'') {
        // a quoted name: '' is a quote, a single ' ends it
        let mut name = String::new();
        let mut it = q.char_indices();
        let end = loop {
            let (i, c) = it.next()?;
            if c == '\'' {
                if q[i + 1..].starts_with('\'') {
                    name.push('\'');
                    it.next();
                } else {
                    break i + 1;
                }
            } else {
                name.push(c);
            }
        };
        (Some(name), q[end..].strip_prefix('!')?)
    } else if let Some((name, rest)) = t.split_once('!') {
        (Some(String::from(name)), rest)
    } else {
        (None, t)
    };
    let sheet = match sheet {
        Some(name) => {
            if name.is_empty() || name.contains(['[', ']', ':']) {
                return None;
            }
            let fold = name.to_lowercase();
            Some(sheets.iter().position(|s| s.as_ref().to_lowercase() == fold)?)
        }
        None => None,
    };
    let (a, b) = match rest.split_once(':') {
        Some((a, b)) => (side(a)?, side(b)?),
        None => match side(rest)? {
            Side::Cell(c) => (Side::Cell(c), Side::Cell(c)),
            _ => return None,
        },
    };
    let (first, last) = match (a, b) {
        (Side::Cell(x), Side::Cell(y)) => (x, y),
        (Side::Col(x), Side::Col(y)) => (CellRef::new(0, x), CellRef::new(MAX_ROWS - 1, y)),
        (Side::Row(x), Side::Row(y)) => (CellRef::new(x, 0), CellRef::new(y, MAX_COLS - 1)),
        _ => return None,
    };
    Some(Area::new(sheet, first, last))
}

/// A rectangular table: every row has `columns.len()` values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    /// the column names: trimmed, an empty one is `F<n>` (n the 1-based column, as Excel's OLE
    /// DB driver names them), a repeated one gets a number (`Name`, `Name1`, `Name2`; compared
    /// case-insensitively); without a header row all are `F1` ... `Fn`
    pub columns: Vec<String>,
    /// the rows of values
    pub rows: Vec<Vec<Value>>,
}

impl Table {
    /// The index of a column by name (case-insensitive).
    pub fn column(&self, name: &str) -> Option<usize> {
        let fold = name.to_lowercase();
        self.columns.iter().position(|c| c.to_lowercase() == fold)
    }
}

// the column names of a header row (see Table::columns)
pub(crate) fn column_names(raw: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    // folded names taken -> the next number to try for that base (no quadratic search)
    let mut taken: BTreeMap<String, u32> = BTreeMap::new();
    for (i, n) in raw.into_iter().enumerate() {
        let t = n.trim();
        let base = if t.is_empty() { format!("F{}", i + 1) } else { t.to_string() };
        let mut name = base.clone();
        let fold = base.to_lowercase();
        if taken.contains_key(&fold) {
            let mut k = taken.get(&fold).copied().unwrap_or(1);
            loop {
                name = format!("{base}{k}");
                k += 1;
                if !taken.contains_key(&name.to_lowercase()) {
                    break;
                }
            }
            taken.insert(fold, k);
        }
        taken.entry(name.to_lowercase()).or_insert(1);
        out.push(name);
    }
    out
}

// the names F1 .. Fn of a table without a header row
pub(crate) fn plain_names(n: usize) -> Vec<String> {
    (1..=n).map(|i| format!("F{i}")).collect()
}

// the test binary of the unit tests: plat's start of a std program refers to the jfgui_main a
// program defines
#[cfg(test)]
rt::test_main!();

#[cfg(test)]
mod tests;
