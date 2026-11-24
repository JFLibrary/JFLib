// xlsx -- Office Open XML workbooks: .xlsx, .xlsm (the macros are never read), .xltx, .xltm, in
// transitional and strict OOXML
//
//   Workbook::open(bytes, Limits)      the package (archive::opc), the workbook part found through
//                                      the package's officeDocument relationship, the sheets
//                                      through the workbook's relationships (never by guessing
//                                      paths), the shared strings and the styles; tables
//                                      (ListObjects) of the worksheets as names
//   wb.sheet(i) -> Grid                one worksheet part, read when asked for
//   wb.table(&area, header)            a rectangle as a Table; wb.sheet_table(i, header) the used
//                                      range of a sheet
//
// Values: the stored ones.  Formulas are never evaluated: a formula's cached <v> counts, a
// formula without one is Empty.  Cell types s (shared string), str (a formula's text),
// inlineStr, b, e, n and d (ISO 8601 text); a number is a Date when its style's number format is
// one: the built-in ids 14-22, 27-36, 45-47, 50-58 (the last two groups are dates in Excel's
// East Asian locales and Excel treats them so everywhere), or a custom code with a date or time
// token (d m y h s e g) outside quotes, brackets and the escapes \x _x *x; [h] [mm] [ss] make an
// elapsed time.
//
// Dates become OLE Automation dates (days since 1899-12-30, as Visio uses them):
//   1900 system   serials from 1 to below 60 (1900-01-01 .. 1900-02-28) move one day up, Excel
//                 counts the 1900-02-29 that never was; serial 60 itself, that day, reads as
//                 1900-02-28 like the serial before it; from 61 (1900-03-01) on serials are OLE
//                 dates already; from 0 to below 1 (Excel's "1900-01-00") a time of day
//                 without a date, as it is
//   1904 system   (workbookPr date1904) serial + 1462; under a format of a time of day alone
//                 (h:mm) a value from 0 to below 1 stays a time without a date
//   both          an elapsed-time format keeps the value as it is (1.5 = 36 hours)
//
// Text: shared strings with rich runs (<r><t>), phonetic runs (<rPh>) left out; a <t> without
// xml:space="preserve" (on it or an element around it) loses its white space at either end, as
// in Excel; _xHHHH_ is decoded (Excel's escape of control characters; _x005F_ is "_", so
// "_x005F_x0041_" reads "_x0041_").
//
// Positions: a row without r follows the previous row, a cell without r the previous cell of its
// row (the first at column A).  Rows and cells out of order are sorted; a cell given twice: the
// last one wins.  Only cells with a value are kept (sparse): one cell at A1 and one at XFD1048576
// are two cells.
//
// Untrusted input: the ZIP within Limits::archive (the compression ratio checked before
// inflating), the XML without DTD and with the nesting bounded (Limits::depth), the rows,
// columns, cells, shared strings and their bytes bounded; <dimension>, count and uniqueCount are
// never read; a shared-string index out of range is Invalid.

use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use archive::opc::{self, Package, Rules};

use crate::xml::{self, Event, Ns, Reader, Start, XmlError};
use crate::{column_names, parse_area, parse_iso, plain_names, quote_sheet, Area, CellError, CellRef, Table, Value};

/// What a workbook may hold.  The default is generous for real workbooks: Excel's own sheet size,
/// four million cells a sheet, two million shared strings of 256 MiB, ZIP parts up to 512 MiB
/// and a compression ratio of at most 200 for parts over 1 MiB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// the ZIP container: parts, sizes, the compression ratio (checked before inflating)
    pub archive: archive::Limits,
    /// rows of a sheet: a row index from 0 to `rows - 1`
    pub rows: u32,
    /// columns of a sheet: a column index from 0 to `cols - 1`
    pub cols: u32,
    /// cells kept of one sheet (those with a value), and values of one Table
    pub cells: usize,
    /// entries of the shared-string table
    pub strings: usize,
    /// bytes of the shared strings together, and of the texts of one sheet together
    pub string_bytes: usize,
    /// nesting of the XML elements of a part
    pub depth: usize,
}

impl Limits {
    /// The default limits (see `Limits`).
    pub const DEFAULT: Limits = Limits {
        archive: archive::Limits { entries: 10_000, part_bytes: 512 << 20, total_bytes: 1 << 30, archive_bytes: 512 << 20, ratio: 200 },
        rows: crate::MAX_ROWS,
        cols: crate::MAX_COLS,
        cells: 4_000_000,
        strings: 2_000_000,
        string_bytes: 256 << 20,
        depth: 64,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Why a workbook or a sheet was not read.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// the ZIP container (archive's reader), its limits included
    Archive(archive::Error),
    /// the OPC package: content types, relationships, part names (archive::opc)
    Package(opc::Error),
    /// not an OOXML workbook: no ZIP, no officeDocument relationship, another main part (a
    /// document, a binary .xlsb)
    NotXlsx,
    /// a part's XML is not well-formed or uses what the reader refuses (a DOCTYPE)
    Xml {
        /// the part's name in the package
        part: String,
        /// the byte offset in the part's text
        offset: usize,
        /// what is wrong
        what: &'static str,
    },
    /// over a limit: the name of the field of `Limits`
    Limit(&'static str),
    /// something the workbook needs is not there
    Missing(&'static str),
    /// a value or structure that breaks the format
    Invalid(&'static str),
}

impl Error {
    /// The error as a sentence in English.
    pub fn text(&self) -> String {
        match self {
            Error::Archive(e) => format!("XLSX: the ZIP container: {}", archive_text(*e)),
            Error::Package(e) => format!("XLSX: the package: {}", package_text(e)),
            Error::NotXlsx => String::from("not an XLSX workbook"),
            Error::Xml { part, offset, what } => format!("XLSX: /{part}: XML {what} (at byte {offset})"),
            Error::Limit(what) => format!("XLSX: over the limit {what}"),
            Error::Missing(what) => format!("XLSX: missing: {what}"),
            Error::Invalid(what) => format!("XLSX: invalid: {what}"),
        }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.write_str(&self.text())
    }
}

fn archive_text(e: archive::Error) -> &'static str {
    match e {
        archive::Error::Invalid => "not a valid ZIP archive",
        archive::Error::Unsupported => "a ZIP feature that is not read (encryption, a method other than DEFLATE, several disks)",
        archive::Error::Checksum => "a part's checksum does not match",
        archive::Error::Limit => "over a limit of Limits::archive (parts, sizes, compression ratio)",
        _ => "refused",
    }
}

fn package_text(e: &opc::Error) -> String {
    match e {
        opc::Error::Zip(z) => String::from(archive_text(*z)),
        opc::Error::Xml(x) => format!("the XML of the content types or a relationship part ({x:?})"),
        opc::Error::Invalid => String::from("its structure (a missing part, content type or relationship target)"),
        opc::Error::Root(r) => format!("an unexpected root element {r}"),
        opc::Error::Address => String::from("a part name or target that is not resolved"),
        other => format!("{other:?}"),
    }
}

fn package_err(e: opc::Error) -> Error {
    match e {
        opc::Error::Zip(z) => Error::Archive(z),
        other => Error::Package(other),
    }
}

/// Whether a sheet is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetState {
    /// shown as a tab
    Visible,
    /// hidden; the user can show it again
    Hidden,
    /// hidden; only a macro or the file can show it again
    VeryHidden,
}

/// What a sheet holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetKind {
    /// cells
    Worksheet,
    /// one chart, no cells (`sheet` gives an empty Grid)
    Chartsheet,
    /// a dialog or macro sheet, or a sheet whose part is missing (`sheet` gives an empty Grid)
    Other,
}

/// A sheet of the workbook, in tab order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetInfo {
    /// the tab's name
    pub name: String,
    /// shown or hidden
    pub state: SheetState,
    /// worksheet, chartsheet or other
    pub kind: SheetKind,
}

/// A defined name of the workbook, or a table (ListObject) of a worksheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinedName {
    /// the name as written (`Daten`, `_xlnm.Print_Area`; a table's display name)
    pub name: String,
    /// the sheet a sheet-local name belongs to (localSheetId); None for a global name
    pub scope: Option<usize>,
    /// hidden from the user (`hidden="1"`)
    pub hidden: bool,
    /// the formula as written (`'Tab 1'!$A$1:$C$10`); for a table its sheet and range
    pub formula: String,
    /// the rectangle when the formula is one (`parse_area`); a reference without a sheet takes
    /// the sheet of the name's scope.  A table's area leaves out its totals row.
    pub area: Option<Area>,
    /// a table (ListObject, xl/tables/tableN.xml) rather than a defined name
    pub table: bool,
}

impl DefinedName {
    /// Whether this is one of Excel's built-in names (`_xlnm.Print_Area`, `_xlnm._FilterDatabase` ...).
    pub fn builtin(&self) -> bool {
        self.name.as_bytes().get(..6).is_some_and(|p| p.eq_ignore_ascii_case(b"_xlnm."))
    }
}

static EMPTY: Value = Value::Empty;

/// The cells of one sheet that hold a value, sparse: rows in ascending order, each with its
/// cells (column, value) in ascending order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Grid {
    sheet: usize,
    rows: Vec<(u32, Vec<(u32, Value)>)>,
}

impl Grid {
    /// The sheet's index.
    pub fn sheet(&self) -> usize {
        self.sheet
    }
    /// The value of a cell (0-based); Empty where there is none.
    pub fn get(&self, row: u32, col: u32) -> &Value {
        let cells = self.row(row);
        match cells.binary_search_by_key(&col, |(c, _)| *c) {
            Ok(i) => &cells[i].1,
            Err(_) => &EMPTY,
        }
    }
    /// The cells of a row with a value: (column, value) in ascending columns.
    pub fn row(&self, row: u32) -> &[(u32, Value)] {
        match self.rows.binary_search_by_key(&row, |(r, _)| *r) {
            Ok(i) => &self.rows[i].1,
            Err(_) => &[],
        }
    }
    /// The rows with a value, ascending: (row, its cells).
    pub fn rows(&self) -> impl Iterator<Item = (u32, &[(u32, Value)])> {
        self.rows.iter().map(|(r, c)| (*r, c.as_slice()))
    }
    /// The cells with a value.
    pub fn cells(&self) -> usize {
        self.rows.iter().map(|(_, c)| c.len()).sum()
    }
    /// Whether no cell has a value.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    /// The used range: from the first to the last row and column that hold a value; None for an
    /// empty sheet.
    pub fn used(&self) -> Option<Area> {
        let first_row = self.rows.first()?.0;
        let last_row = self.rows.last()?.0;
        let first_col = self.rows.iter().filter_map(|(_, c)| c.first().map(|x| x.0)).min()?;
        let last_col = self.rows.iter().filter_map(|(_, c)| c.last().map(|x| x.0)).max()?;
        Some(Area { sheet: Some(self.sheet), first: CellRef::new(first_row, first_col), last: CellRef::new(last_row, last_col) })
    }
    /// A rectangle of this grid as a Table (`header`: its first row names the columns).  Rows
    /// and columns past the used range are left out (a whole-column reference gives the rows
    /// that hold data), the area's first row and column stay; at most `cells` values (rows times
    /// columns), else `Limit("cells")`.  The area's sheet is not looked at.
    pub fn table(&self, area: &Area, header: bool, cells: usize) -> Result<Table, Error> {
        let Some(used) = self.used() else { return Ok(Table::default()) };
        let (r0, c0) = (area.first.row, area.first.col);
        let last_row = area.last.row.min(used.last.row);
        let last_col = area.last.col.min(used.last.col);
        if last_row < r0 || last_col < c0 {
            return Ok(Table::default());
        }
        let nrows = (last_row - r0) as usize + 1;
        let ncols = (last_col - c0) as usize + 1;
        if nrows as u64 * ncols as u64 > cells as u64 {
            return Err(Error::Limit("cells"));
        }
        let mut matrix: Vec<Vec<Value>> = Vec::with_capacity(nrows);
        let start = self.rows.partition_point(|(r, _)| *r < r0);
        for (r, line) in &self.rows[start..] {
            if *r > last_row {
                break;
            }
            while (matrix.len() as u64) < (*r - r0) as u64 {
                matrix.push(vec![Value::Empty; ncols]);
            }
            let mut values = vec![Value::Empty; ncols];
            let s = line.partition_point(|(c, _)| *c < c0);
            for (c, v) in &line[s..] {
                if *c > last_col {
                    break;
                }
                values[(*c - c0) as usize] = v.clone();
            }
            matrix.push(values);
        }
        while matrix.len() < nrows {
            matrix.push(vec![Value::Empty; ncols]);
        }
        if !header {
            return Ok(Table { columns: plain_names(ncols), rows: matrix });
        }
        let mut it = matrix.into_iter();
        let head = it.next().unwrap_or_default();
        Ok(Table { columns: column_names(head.iter().map(Value::text).collect()), rows: it.collect() })
    }
}

// how a style shows its number
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fmt {
    Plain,
    // a date, perhaps with a time
    Date,
    // a time of day alone (h, s, m next to them)
    Time,
    // an elapsed time ([h]:mm)
    Elapsed,
}

// the built-in number formats that are dates and times
fn builtin(id: u32) -> Fmt {
    match id {
        14..=17 | 22 | 27..=36 | 50..=58 => Fmt::Date,
        18..=21 | 45 | 47 => Fmt::Time,
        46 => Fmt::Elapsed,
        _ => Fmt::Plain,
    }
}

// a custom number format code: a date or time token outside quotes, brackets and escapes
fn format_kind(code: &str) -> Fmt {
    let b = code.as_bytes();
    let mut i = 0;
    // tokens of a date (d y e g), of a time (h s), m (a month, or minutes next to h or s)
    let (mut date, mut time, mut m) = (false, false, false);
    while i < b.len() {
        match b[i] {
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    i += 1;
                }
                i += 1;
            }
            b'\\' | b'_' | b'*' => i += 2,
            b'[' => {
                let Some(len) = b[i..].iter().position(|&c| c == b']') else { break };
                let inner = &b[i + 1..i + len];
                let elapsed = |c: u8| !inner.is_empty() && inner.iter().all(|x| x.to_ascii_lowercase() == c);
                if elapsed(b'h') || elapsed(b'm') || elapsed(b's') {
                    return Fmt::Elapsed;
                }
                i += len + 1;
            }
            _ if b.len() - i >= 7 && b[i..i + 7].eq_ignore_ascii_case(b"general") => i += 7,
            c => {
                let next = b.get(i + 1).copied().unwrap_or(0);
                match c.to_ascii_lowercase() {
                    b'd' | b'y' | b'g' => date = true,
                    // e: the era year; E+ / E- is a number's exponent
                    b'e' if next != b'+' && next != b'-' => date = true,
                    b'h' | b's' => time = true,
                    b'm' => m = true,
                    _ => {}
                }
                i += 1;
            }
        }
    }
    if date || (m && !time) {
        Fmt::Date
    } else if time {
        Fmt::Time
    } else {
        Fmt::Plain
    }
}

// a serial of the workbook's date system -> an OLE Automation date (see the module's head);
// `time`: the format shows a time of day alone
fn to_ole(v: f64, date1904: bool, time: bool) -> f64 {
    if date1904 {
        if time && (0.0..1.0).contains(&v) { v } else { v + 1462.0 }
    } else if (1.0..60.0).contains(&v) {
        v + 1.0
    } else {
        v
    }
}

// Excel's escape of characters XML cannot hold: _xHHHH_ is the UTF-16 unit HHHH (a pair of
// surrogates is one character), _x005F_ is "_"; anything else stays as written
pub(crate) fn unescape_x(s: &str) -> Cow<'_, str> {
    if !s.contains("_x") {
        return Cow::Borrowed(s);
    }
    let b = s.as_bytes();
    let unit = |i: usize| -> Option<u16> {
        let t = b.get(i..i + 7)?;
        if t[0] != b'_' || t[1] != b'x' || t[6] != b'_' || !t[2..6].iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        Some(t[2..6].iter().fold(0u16, |a, &c| a << 4 | (c as char).to_digit(16).unwrap_or(0) as u16))
    };
    let mut out = String::with_capacity(s.len());
    let mut run = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'_' {
            i += 1;
            continue;
        }
        let Some(u) = unit(i) else {
            i += 1;
            continue;
        };
        let (c, used) = if (0xD800..0xDC00).contains(&u) {
            match unit(i + 7).filter(|l| (0xDC00..0xE000).contains(l)) {
                Some(l) => (char::from_u32(0x10000 + ((u as u32 - 0xD800) << 10) + (l as u32 - 0xDC00)), 14),
                None => (None, 7),
            }
        } else {
            (char::from_u32(u as u32), 7)
        };
        let Some(c) = c else {
            i += 1;
            continue;
        };
        out.push_str(&s[run..i]);
        out.push(c);
        i += used;
        run = i;
    }
    out.push_str(&s[run..]);
    Cow::Owned(out)
}

fn xml_err(part: &str, (e, at): (XmlError, usize)) -> Error {
    if e == XmlError::Depth {
        Error::Limit("depth")
    } else {
        Error::Xml { part: String::from(part), offset: at, what: e.text() }
    }
}

// the reader of one part with its errors mapped
struct P<'a> {
    r: Reader<'a>,
    part: &'a str,
}

impl<'a> P<'a> {
    fn new(text: &'a str, part: &'a str, depth: usize) -> Self {
        P { r: Reader::new(text, depth), part }
    }
    fn next(&mut self) -> Result<Event<'a>, Error> {
        self.r.next().map_err(|e| xml_err(self.part, e))
    }
    fn skip(&mut self) -> Result<(), Error> {
        self.r.skip().map_err(|e| xml_err(self.part, e))
    }
    fn text(&mut self, limit: usize) -> Result<String, Error> {
        self.r.text(limit).map_err(|e| xml_err(self.part, e))?.ok_or(Error::Limit("string_bytes"))
    }
    // the root element
    fn root(&mut self) -> Result<Start<'a>, Error> {
        loop {
            match self.next()? {
                Event::Start(s) => return Ok(s),
                Event::Text(_) => {}
                _ => return Err(Error::Xml { part: String::from(self.part), offset: 0, what: "without a root element" }),
            }
        }
    }
}

// a part's bytes as text
fn part_text<'p>(pkg: &'p Package, name: &str) -> Result<Cow<'p, str>, Error> {
    let bytes = pkg.part(name).ok_or(Error::Missing("a part a relationship names"))?;
    xml::text_of(bytes).ok_or_else(|| Error::Xml { part: String::from(name), offset: 0, what: "neither UTF-8 nor UTF-16" })
}

// a relationship type of SpreadsheetML (transitional or strict) by its last segment
fn rel_is(kind: &str, what: &str) -> bool {
    [xml::NS_REL, xml::NS_REL_STRICT].iter().any(|base| kind.strip_prefix(base).and_then(|r| r.strip_prefix('/')) == Some(what))
}

fn flag(v: Option<&str>) -> bool {
    v.is_some_and(|v| matches!(xml::trim(v), "1" | "true" | "on"))
}

// text kept: without xml:space="preserve" the white space at the ends goes; _xHHHH_ decoded
fn kept_text(t: String, preserve: bool) -> String {
    let t = if preserve { t } else { String::from(xml::trim(&t)) };
    match unescape_x(&t) {
        Cow::Borrowed(_) => t,
        Cow::Owned(o) => o,
    }
}

// the text of an <si> or <is>: its <t>, the <t> of its runs; phonetic runs (<rPh>) left out
fn rich(p: &mut P, preserve: bool, limit: usize) -> Result<String, Error> {
    let mut out = String::new();
    loop {
        match p.next()? {
            Event::Start(s) if s.is("t") => {
                let keep = s.space().unwrap_or(preserve);
                let t = p.text(limit.saturating_sub(out.len()))?;
                out.push_str(&kept_text(t, keep));
            }
            Event::Start(s) if s.is("r") => {
                let run = s.space().unwrap_or(preserve);
                loop {
                    match p.next()? {
                        Event::Start(t) if t.is("t") => {
                            let keep = t.space().unwrap_or(run);
                            let t = p.text(limit.saturating_sub(out.len()))?;
                            out.push_str(&kept_text(t, keep));
                        }
                        Event::Start(_) => p.skip()?,
                        Event::End => break,
                        Event::Text(_) => {}
                        Event::Eof => return Err(Error::Invalid("a part ends inside a run")),
                    }
                }
            }
            Event::Start(_) => p.skip()?,
            Event::End => return Ok(out),
            Event::Text(_) => {}
            Event::Eof => return Err(Error::Invalid("a part ends inside a string")),
        }
    }
}

// a sheet as workbook.xml lists it
struct RawSheet {
    name: String,
    state: SheetState,
    rid: Option<String>,
}

// what workbook.xml says
struct Book {
    sheets: Vec<RawSheet>,
    // (name, localSheetId, hidden, formula)
    names: Vec<(String, Option<usize>, bool, String)>,
    date1904: bool,
}

fn read_workbook(text: &str, part: &str, limits: &Limits) -> Result<Book, Error> {
    let mut p = P::new(text, part, limits.depth);
    let root = p.root()?;
    if !root.is("workbook") {
        return Err(Error::NotXlsx);
    }
    let mut book = Book { sheets: Vec::new(), names: Vec::new(), date1904: false };
    loop {
        match p.next()? {
            Event::Start(s) if s.is("workbookPr") => {
                book.date1904 = flag(s.attr("date1904"));
                p.skip()?;
            }
            Event::Start(s) if s.is("sheets") => loop {
                match p.next()? {
                    Event::Start(s) if s.is("sheet") => {
                        let name = String::from(s.attr("name").ok_or(Error::Invalid("a sheet without a name"))?);
                        let state = match s.attr("state").map(xml::trim) {
                            Some("hidden") => SheetState::Hidden,
                            Some("veryHidden") => SheetState::VeryHidden,
                            _ => SheetState::Visible,
                        };
                        let rid = s.attr_ns(Ns::Rel, "id").map(String::from);
                        book.sheets.push(RawSheet { name, state, rid });
                        p.skip()?;
                    }
                    Event::Start(_) => p.skip()?,
                    Event::End => break,
                    Event::Text(_) => {}
                    Event::Eof => return Err(Error::Invalid("workbook.xml ends inside <sheets>")),
                }
            },
            Event::Start(s) if s.is("definedNames") => loop {
                match p.next()? {
                    Event::Start(s) if s.is("definedName") => {
                        let name = String::from(s.attr("name").unwrap_or(""));
                        let scope = s.attr("localSheetId").map(|v| xml::trim(v).parse::<usize>().ok());
                        let hidden = flag(s.attr("hidden"));
                        let formula = p.text(limits.string_bytes)?;
                        // a localSheetId that is no number: the name is left out
                        match scope {
                            Some(None) => {}
                            Some(Some(i)) => book.names.push((name, Some(i), hidden, formula)),
                            None => book.names.push((name, None, hidden, formula)),
                        }
                    }
                    Event::Start(_) => p.skip()?,
                    Event::End => break,
                    Event::Text(_) => {}
                    Event::Eof => return Err(Error::Invalid("workbook.xml ends inside <definedNames>")),
                }
            },
            Event::Start(_) => p.skip()?,
            Event::End | Event::Eof => break,
            Event::Text(_) => {}
        }
    }
    Ok(book)
}

fn read_strings(text: &str, part: &str, limits: &Limits) -> Result<Vec<String>, Error> {
    let mut p = P::new(text, part, limits.depth);
    let root = p.root()?;
    if !root.is("sst") {
        return Err(Error::Invalid("the shared strings part's root is no <sst>"));
    }
    let preserve = root.space().unwrap_or(false);
    let mut out: Vec<String> = Vec::new();
    let mut bytes = 0usize;
    loop {
        match p.next()? {
            Event::Start(s) if s.is("si") => {
                if out.len() >= limits.strings {
                    return Err(Error::Limit("strings"));
                }
                let t = rich(&mut p, s.space().unwrap_or(preserve), limits.string_bytes - bytes)?;
                bytes += t.len();
                if bytes > limits.string_bytes {
                    return Err(Error::Limit("string_bytes"));
                }
                out.push(t);
            }
            Event::Start(_) => p.skip()?,
            Event::End | Event::Eof => break,
            Event::Text(_) => {}
        }
    }
    Ok(out)
}

fn read_styles(text: &str, part: &str, limits: &Limits) -> Result<Vec<Fmt>, Error> {
    let mut p = P::new(text, part, limits.depth);
    let root = p.root()?;
    if !root.is("styleSheet") {
        return Err(Error::Invalid("the styles part's root is no <styleSheet>"));
    }
    let mut custom: BTreeMap<u32, Fmt> = BTreeMap::new();
    let mut xfs: Vec<u32> = Vec::new();
    let number = |v: Option<&str>| v.and_then(|v| xml::trim(v).parse::<u32>().ok());
    loop {
        match p.next()? {
            Event::Start(s) if s.is("numFmts") || s.is("cellXfs") => {
                let fmts = s.is("numFmts");
                loop {
                    match p.next()? {
                        Event::Start(f) if fmts && f.is("numFmt") => {
                            if let Some(id) = number(f.attr("numFmtId")) {
                                custom.insert(id, format_kind(f.attr("formatCode").unwrap_or("")));
                            }
                            p.skip()?;
                        }
                        Event::Start(x) if !fmts && x.is("xf") => {
                            xfs.push(number(x.attr("numFmtId")).unwrap_or(0));
                            p.skip()?;
                        }
                        Event::Start(_) => p.skip()?,
                        Event::End => break,
                        Event::Text(_) => {}
                        Event::Eof => return Err(Error::Invalid("the styles part ends early")),
                    }
                }
            }
            Event::Start(_) => p.skip()?,
            Event::End | Event::Eof => break,
            Event::Text(_) => {}
        }
    }
    Ok(xfs.iter().map(|id| custom.get(id).copied().unwrap_or_else(|| builtin(*id))).collect())
}

/// An XLSX workbook: its sheets, names, shared strings and styles; the sheets themselves are read
/// when asked for.
#[derive(Clone)]
pub struct Workbook {
    pkg: Package,
    limits: Limits,
    sheets: Vec<SheetInfo>,
    // the part of each sheet
    parts: Vec<Option<String>>,
    names: Vec<DefinedName>,
    date1904: bool,
    strings: Vec<String>,
    // the number format of each cellXfs entry
    xfs: Vec<Fmt>,
}

impl core::fmt::Debug for Workbook {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Workbook")
            .field("sheets", &self.sheets)
            .field("names", &self.names)
            .field("date1904", &self.date1904)
            .field("strings", &self.strings.len())
            .finish()
    }
}

impl Workbook {
    /// Read a workbook within `limits`: the package, workbook.xml, its relationships, the shared
    /// strings, the styles and the tables of the worksheets; not the cells.
    pub fn open(bytes: &[u8], limits: Limits) -> Result<Workbook, Error> {
        if !bytes.starts_with(b"PK\x03\x04") && !bytes.starts_with(b"PK\x05\x06") {
            return Err(Error::NotXlsx);
        }
        // a macro project is accepted (.xlsm) and never read
        let rules = Rules { macros: true, ..Rules::DEFAULT };
        let pkg = Package::read(bytes, limits.archive, rules).map_err(package_err)?;
        let main = String::from(pkg.main_part().ok_or(Error::NotXlsx)?);
        if pkg.content_type(&main).is_some_and(|t| t.to_ascii_lowercase().contains("binary")) {
            return Err(Error::NotXlsx);
        }
        // the workbook's relationships: id -> (type, the part it names)
        let mut rels: BTreeMap<String, (String, Option<String>)> = BTreeMap::new();
        for r in pkg.relationships(&main).map_err(package_err)? {
            let part = if r.external { None } else { pkg.resolve(&main, &r.target).ok().map(String::from) };
            rels.insert(r.id, (r.kind, part));
        }
        let first_of = |what: &str| rels.values().find(|(k, p)| rel_is(k, what) && p.is_some()).and_then(|(_, p)| p.clone());
        let book = read_workbook(&part_text(&pkg, &main)?, &main, &limits)?;
        let strings = match first_of("sharedStrings") {
            Some(part) => read_strings(&part_text(&pkg, &part)?, &part, &limits)?,
            None => Vec::new(),
        };
        let xfs = match first_of("styles") {
            Some(part) => read_styles(&part_text(&pkg, &part)?, &part, &limits)?,
            None => Vec::new(),
        };
        let mut sheets = Vec::with_capacity(book.sheets.len());
        let mut parts = Vec::with_capacity(book.sheets.len());
        for s in book.sheets {
            let rel = s.rid.as_ref().and_then(|id| rels.get(id));
            let (kind, part) = match rel {
                Some((k, Some(p))) if rel_is(k, "worksheet") => (SheetKind::Worksheet, Some(p.clone())),
                Some((k, Some(p))) if rel_is(k, "chartsheet") => (SheetKind::Chartsheet, Some(p.clone())),
                Some((_, p)) => (SheetKind::Other, p.clone()),
                None => (SheetKind::Other, None),
            };
            sheets.push(SheetInfo { name: s.name, state: s.state, kind });
            parts.push(part);
        }
        let sheet_names: Vec<&str> = sheets.iter().map(|s| s.name.as_str()).collect();
        let mut names = Vec::new();
        for (name, scope, hidden, formula) in book.names {
            if scope.is_some_and(|i| i >= sheets.len()) {
                continue; // a localSheetId past the sheets
            }
            let area = parse_area(&formula, &sheet_names).map(|mut a| {
                a.sheet = a.sheet.or(scope);
                a
            });
            names.push(DefinedName { name, scope, hidden, formula, area, table: false });
        }
        for (i, part) in parts.iter().enumerate() {
            if let (SheetKind::Worksheet, Some(part)) = (sheets[i].kind, part) {
                tables(&pkg, part, i, &sheets[i].name, &limits, &mut names);
            }
        }
        Ok(Workbook { pkg, limits, sheets, parts, names, date1904: book.date1904, strings, xfs })
    }

    /// The sheets in tab order.
    pub fn sheets(&self) -> &[SheetInfo] {
        &self.sheets
    }
    /// The defined names in file order (built-in `_xlnm.*` ones included, see
    /// `DefinedName::builtin`), then the tables of the worksheets (`DefinedName::table`).
    pub fn names(&self) -> &[DefinedName] {
        &self.names
    }
    /// Whether the workbook counts dates from 1904 (its serials are converted either way).
    pub fn date1904(&self) -> bool {
        self.date1904
    }
    /// The index of a sheet by name (ASCII and Unicode case-insensitive, as Excel compares them).
    pub fn sheet_by_name(&self, name: &str) -> Option<usize> {
        let fold = name.to_lowercase();
        self.sheets.iter().position(|s| s.name.to_lowercase() == fold)
    }
    /// `parse_area` with this workbook's sheet names.
    pub fn area(&self, text: &str) -> Option<Area> {
        let names: Vec<&str> = self.sheets.iter().map(|s| s.name.as_str()).collect();
        parse_area(text, &names)
    }
    /// Read one sheet: its stored values.  A chartsheet or another kind of sheet gives an empty
    /// Grid; `Missing("sheet")` for an index past the sheets.
    pub fn sheet(&self, index: usize) -> Result<Grid, Error> {
        let info = self.sheets.get(index).ok_or(Error::Missing("sheet"))?;
        let empty = Grid { sheet: index, rows: Vec::new() };
        if info.kind != SheetKind::Worksheet {
            return Ok(empty);
        }
        let Some(part) = self.parts[index].as_deref() else { return Ok(empty) };
        let text = part_text(&self.pkg, part)?;
        let mut p = P::new(&text, part, self.limits.depth);
        let root = p.root()?;
        if !root.is("worksheet") {
            return Err(Error::Invalid("a worksheet part whose root is no <worksheet>"));
        }
        let mut b = Builder { rows: Vec::new(), messy: false, cells: 0, text: 0, lim: self.limits };
        loop {
            match p.next()? {
                Event::Start(s) if s.is("sheetData") => self.sheet_data(&mut p, &mut b)?,
                Event::Start(_) => p.skip()?,
                Event::End | Event::Eof => break,
                Event::Text(_) => {}
            }
        }
        Ok(b.finish(index))
    }

    fn sheet_data(&self, p: &mut P, b: &mut Builder) -> Result<(), Error> {
        let mut next_row: u64 = 0;
        loop {
            match p.next()? {
                Event::Start(s) if s.is("row") => {
                    let row = match s.attr("r") {
                        Some(v) => xml::trim(v).parse::<u64>().ok().filter(|&r| r >= 1).ok_or(Error::Invalid("a row number"))? - 1,
                        None => next_row,
                    };
                    if row >= self.limits.rows as u64 {
                        return Err(Error::Limit("rows"));
                    }
                    next_row = row + 1;
                    self.row(p, b, row as u32)?;
                }
                Event::Start(_) => p.skip()?,
                Event::End => return Ok(()),
                Event::Text(_) => {}
                Event::Eof => return Err(Error::Invalid("a worksheet ends inside <sheetData>")),
            }
        }
    }

    fn row(&self, p: &mut P, b: &mut Builder, row: u32) -> Result<(), Error> {
        let mut next_col: u64 = 0;
        loop {
            match p.next()? {
                Event::Start(s) if s.is("c") => {
                    let (r, c) = match s.attr("r") {
                        Some(v) => {
                            let at = crate::parse_ref(xml::trim(v)).ok_or(Error::Invalid("a cell reference"))?;
                            (at.row as u64, at.col as u64)
                        }
                        None => (row as u64, next_col),
                    };
                    if r >= self.limits.rows as u64 {
                        return Err(Error::Limit("rows"));
                    }
                    if c >= self.limits.cols as u64 {
                        return Err(Error::Limit("cols"));
                    }
                    next_col = c + 1;
                    let style = s.attr("s").and_then(|v| xml::trim(v).parse::<usize>().ok());
                    // (a number's <v> is read even when the sheet's text is used up)
                    let (v, inline) = cell(p, b.text_left().max(256))?;
                    let value = self.value(s.attr("t").map(xml::trim), style, v, inline)?;
                    b.push(r as u32, c as u32, value)?;
                }
                Event::Start(_) => p.skip()?,
                Event::End => return Ok(()),
                Event::Text(_) => {}
                Event::Eof => return Err(Error::Invalid("a worksheet ends inside <row>")),
            }
        }
    }

    // a cell's value from its type, style, <v> and <is>
    fn value(&self, t: Option<&str>, style: Option<usize>, v: Option<String>, inline: Option<String>) -> Result<Value, Error> {
        let text = |s: String| if s.is_empty() { Value::Empty } else { Value::Text(s) };
        Ok(match t.unwrap_or("n") {
            "s" => {
                let Some(v) = v else { return Ok(Value::Empty) };
                let i = xml::trim(&v).parse::<usize>().map_err(|_| Error::Invalid("a shared string index"))?;
                text(self.strings.get(i).ok_or(Error::Invalid("a shared string index past the table"))?.clone())
            }
            "inlineStr" => match inline {
                Some(s) => text(s),
                None => v.map_or(Value::Empty, |v| text(kept_text(v, true))),
            },
            "str" => v.map_or(Value::Empty, |v| text(kept_text(v, true))),
            "b" => match v.as_deref().map(xml::trim) {
                None | Some("") => Value::Empty,
                Some("1" | "true") => Value::Bool(true),
                Some("0" | "false") => Value::Bool(false),
                Some(other) => Value::Text(String::from(other)),
            },
            "e" => match v {
                Some(v) if !xml::trim(&v).is_empty() => Value::Error(CellError::parse(xml::trim(&v))),
                _ => Value::Empty,
            },
            "d" => match v {
                Some(v) => match parse_iso(xml::trim(&v), true) {
                    Some(d) => Value::Date(d),
                    None => text(v),
                },
                None => Value::Empty,
            },
            _ => {
                let Some(v) = v else { return Ok(Value::Empty) };
                let t = xml::trim(&v);
                if t.is_empty() {
                    return Ok(Value::Empty);
                }
                let (n, used) = base::f64_parse(t.as_bytes());
                if used != t.len() || !n.is_finite() {
                    return Ok(text(v));
                }
                match style.and_then(|s| self.xfs.get(s)).copied().unwrap_or(Fmt::Plain) {
                    Fmt::Plain => Value::Number(n),
                    Fmt::Date => Value::Date(to_ole(n, self.date1904, false)),
                    Fmt::Time => Value::Date(to_ole(n, self.date1904, true)),
                    Fmt::Elapsed => Value::Date(n),
                }
            }
        })
    }

    /// A rectangle of a sheet as a Table (`header`: its first row names the columns, see
    /// `Table::columns`); see `Grid::table` for how it is cut to the used range.  The area needs
    /// a sheet (`Missing("the area's sheet")` without one); at most `Limits::cells` values.
    pub fn table(&self, area: &Area, header: bool) -> Result<Table, Error> {
        let sheet = area.sheet.ok_or(Error::Missing("the area's sheet"))?;
        self.sheet(sheet)?.table(area, header, self.limits.cells)
    }

    /// The used range of a sheet as a Table (from the first to the last row and column that hold
    /// a value); an empty sheet gives an empty Table.
    pub fn sheet_table(&self, index: usize, header: bool) -> Result<Table, Error> {
        let grid = self.sheet(index)?;
        match grid.used() {
            Some(area) => grid.table(&area, header, self.limits.cells),
            None => Ok(Table::default()),
        }
    }
}

// the <v> text and the <is> text of a cell
fn cell(p: &mut P, limit: usize) -> Result<(Option<String>, Option<String>), Error> {
    let mut v = None;
    let mut inline = None;
    loop {
        match p.next()? {
            Event::Start(s) if s.is("v") => v = Some(p.text(limit)?),
            Event::Start(s) if s.is("is") => inline = Some(rich(p, s.space().unwrap_or(false), limit)?),
            Event::Start(_) => p.skip()?,
            Event::End => return Ok((v, inline)),
            Event::Text(_) => {}
            Event::Eof => return Err(Error::Invalid("a worksheet ends inside <c>")),
        }
    }
}

// the tables (ListObjects) of a worksheet as names; a table part that cannot be read is left out
fn tables(pkg: &Package, part: &str, sheet: usize, sheet_name: &str, limits: &Limits, names: &mut Vec<DefinedName>) {
    let Ok(rels) = pkg.relationships(part) else { return };
    for r in rels.iter().filter(|r| !r.external && rel_is(&r.kind, "table")) {
        let Ok(tpart) = pkg.resolve(part, &r.target) else { continue };
        let Ok(text) = part_text(pkg, tpart) else { continue };
        let mut p = P::new(&text, tpart, limits.depth);
        let Ok(root) = p.root() else { continue };
        if !root.is("table") {
            continue;
        }
        let Some(name) = root.attr("displayName").or(root.attr("name")).map(String::from) else { continue };
        let Some(range) = root.attr("ref").map(xml::trim) else { continue };
        let Some(mut area) = parse_area(range, &[] as &[&str]) else { continue };
        let totals = root.attr("totalsRowCount").and_then(|v| xml::trim(v).parse::<u32>().ok()).unwrap_or(0);
        area.sheet = Some(sheet);
        area.last.row = area.last.row.saturating_sub(totals).max(area.first.row);
        let formula = format!("{}!{}", quote_sheet(sheet_name), range);
        names.push(DefinedName { name, scope: None, hidden: false, formula, area: Some(area), table: true });
    }
}

// the cells of a sheet while it is read
struct Builder {
    rows: Vec<(u32, Vec<(u32, Value)>)>,
    // a row or cell came out of order or twice: finish sorts, merges and drops the empty ones
    messy: bool,
    cells: usize,
    text: usize,
    lim: Limits,
}

impl Builder {
    fn text_left(&self) -> usize {
        self.lim.string_bytes.saturating_sub(self.text)
    }

    fn push(&mut self, row: u32, col: u32, v: Value) -> Result<(), Error> {
        let last_row = self.rows.last().map(|(r, _)| *r);
        if last_row.is_some_and(|r| r > row) {
            self.messy = true;
        }
        let same = last_row == Some(row);
        let in_order = !same || self.rows.last().is_some_and(|(_, c)| c.last().is_none_or(|(k, _)| *k < col));
        if !in_order {
            self.messy = true;
        }
        // an empty cell in order holds nothing; out of order it may replace a value (last wins)
        if v.is_empty() && !self.messy {
            return Ok(());
        }
        self.cells += 1;
        if self.cells > self.lim.cells {
            return Err(Error::Limit("cells"));
        }
        if let Value::Text(s) = &v {
            self.text += s.len();
            if self.text > self.lim.string_bytes {
                return Err(Error::Limit("string_bytes"));
            }
        }
        if !same {
            self.rows.push((row, Vec::new()));
        }
        if let Some((_, cells)) = self.rows.last_mut() {
            cells.push((col, v));
        }
        Ok(())
    }

    fn finish(mut self, sheet: usize) -> Grid {
        if self.messy {
            // stable: cells given twice keep their order in the file, and the last one wins
            self.rows.sort_by_key(|(r, _)| *r);
            let mut merged: Vec<(u32, Vec<(u32, Value)>)> = Vec::with_capacity(self.rows.len());
            for (r, cells) in self.rows {
                match merged.last_mut() {
                    Some((m, list)) if *m == r => list.extend(cells),
                    _ => merged.push((r, cells)),
                }
            }
            for (_, cells) in &mut merged {
                cells.sort_by_key(|(c, _)| *c);
                let mut out: Vec<(u32, Value)> = Vec::with_capacity(cells.len());
                for (c, v) in cells.drain(..) {
                    match out.last_mut() {
                        Some(last) if last.0 == c => last.1 = v,
                        _ => out.push((c, v)),
                    }
                }
                out.retain(|(_, v)| !v.is_empty());
                *cells = out;
            }
            merged.retain(|(_, cells)| !cells.is_empty());
            self.rows = merged;
        }
        Grid { sheet, rows: self.rows }
    }
}
