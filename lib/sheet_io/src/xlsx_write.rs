// xlsx_write -- typed rows as an XLSX workbook (JFDiagrammer JD19, FEATURES.md EX-12 "Shape-Bericht
// exportieren ... als Excel-Tabelle (.xlsx)", EX-16 "Daten exportieren (Inventar)"); re-exported
// as sheet_io::xlsx::{write, BookOut, SheetOut, WriteOptions, WriteError}
//
//   xlsx::write(&BookOut, &WriteOptions) -> Vec<u8>     the workbook as an OPC package
//   BookOut { sheets }                                  the sheets in tab order, the first active
//   SheetOut { name, rows, header_rows, widths, autofilter }
//   SheetOut::new(name, rows) / SheetOut::from_table(name, &Table)  (the column names as a header
//                                                       row: bold, kept in view)
//
// Values, as xlsx::Workbook reads them back:
//   Number   the shortest text that reads back (-0 as 0).  NaN and the infinities, which Excel
//            cannot store, become the error #NUM! (what Excel shows for a number out of its
//            range) instead of refusing the whole workbook for one cell
//   Text     a shared string (equal texts stored once); "" is no cell.  The characters XML
//            cannot hold (controls but TAB and LF; U+FFFE, U+FFFF) and CR are Excel's escape
//            _xHHHH_, and a "_" that would read as the start of one (_x and four hex digits) is
//            _x005F_, so Excel and the reader give back the text as it was; white space at either
//            end is kept (xml:space="preserve")
//   Bool     t="b"
//   Error    t="e" for the seven errors of ECMA-376 (#NULL! #DIV/0! #VALUE! #REF! #NAME? #NUM!
//            #N/A).  Excel stores the newer ones (#SPILL!, #CALC!, #GETTING_DATA, ...) as #VALUE!
//            with rich-value metadata, so they are written as a text of the same letters
//   Date     a serial of Excel's 1900 date system: OLE dates from 1900-01-01 to 1900-02-28 one
//            day down (Excel counts a 1900-02-29 that never was; the reader moves them up again),
//            a time of day alone (0 to below 1) as it is.  The number format is the one Excel
//            picks when a date is typed in: the viewer's short date (built-in 14) without a time
//            part, short date and h:mm (22) with one (seconds stay in the value, unshown), h:mm:ss
//            (21) for a time alone.  A date Excel cannot show (before 1900-01-01, after
//            9999-12-31) is written as its text (Value::text); NaN and infinities as #NUM!
//   Empty    no cell
// No formula is ever written: a text starting with "=" stays a text, so a workbook written here
// carries no formula injection (csv::WriteOptions::guard_formulas is CSV's matter).
//
// Excel's limits are errors (WriteError), never a file Excel has to repair: values only up to
// row 1 048 576 and column XFD (Empty past them is fine), texts up to 32 767 characters (UTF-16
// units, as Excel counts them), sheet names of 1 to 31 characters without [ ] : * ? / \ and
// control characters, not starting or ending with ', not "History" (Excel's change history),
// unique case-insensitively; at least one sheet; column widths 0 to 255.  WriteOptions bounds the
// cells and the bytes of XML, checked cell by cell, so a caller bounds the memory writing takes,
// and sets the DEFLATE level (6; 1 is several times faster on large sheets).
//
// The package: [Content_Types].xml, _rels/.rels, xl/workbook.xml (bookViews, sheets, a hidden
// _xlnm._FilterDatabase name per filter, as Excel writes them), its relationships,
// xl/worksheets/sheetN.xml (dimension, sheetViews with a frozen pane below the header rows,
// sheetFormatPr, cols, sheetData, autoFilter, pageMargins), xl/styles.xml (fonts regular and
// bold, the fills none and gray125 Excel expects first, one border, the style xf, eight cell xfs:
// general, date, date and time, time, each regular and bold; the Normal cell style),
// xl/sharedStrings.xml when there are texts, docProps/core.xml (empty: no author, no time) and
// docProps/app.xml.  Children come in the order of the schema (ECMA-376 Part 1): Excel repairs
// a part whose elements come in another.  Deterministic: no time stamps, archive's fixed ZIP
// date: the same book and options give the same bytes.
//
// Column widths: a given one as it is; else from the longest line among the column's values (an
// East Asian wide character counts twice, a header cell a tenth more for bold) plus a margin,
// from Excel's default 8.43 up to 80.

use alloc::borrow::Cow;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use archive::opc::{self, Package, Relationship, Rules};
use archive::Method;

use crate::{civil, col_name, number_text, Area, CellError, CellRef, Table, Value, MAX_COLS, MAX_ROWS};

/// Excel's most characters in one cell (UTF-16 units, as Excel counts them)
pub const MAX_TEXT: usize = 32_767;
/// Excel's most characters of a sheet name (UTF-16 units)
pub const MAX_SHEET_NAME: usize = 31;

const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>";
const NS_MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL_CORE: &str = "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";
const REL_APP: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties";
const CT_WORKBOOK: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml";
const CT_SHEET: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml";
const CT_STYLES: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml";
const CT_STRINGS: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml";
const CT_CORE: &str = "application/vnd.openxmlformats-package.core-properties+xml";
const CT_APP: &str = "application/vnd.openxmlformats-officedocument.extended-properties+xml";

// the cell xfs of STYLES: a kind of value, plus BOLD in a header row
const XF_DATE: u32 = 1;
const XF_DATETIME: u32 = 2;
const XF_TIME: u32 = 3;
const XF_BOLD: u32 = 4;

// Calibri 11 as Excel's default font; no theme part, so no theme colours or font schemes
const STYLES: &str = concat!(
    "<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">",
    "<fonts count=\"2\"><font><sz val=\"11\"/><name val=\"Calibri\"/><family val=\"2\"/></font>",
    "<font><b/><sz val=\"11\"/><name val=\"Calibri\"/><family val=\"2\"/></font></fonts>",
    "<fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills>",
    "<borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders>",
    "<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>",
    "<cellXfs count=\"8\">",
    "<xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>",
    "<xf numFmtId=\"14\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>",
    "<xf numFmtId=\"22\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>",
    "<xf numFmtId=\"21\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\"/>",
    "<xf numFmtId=\"0\" fontId=\"1\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyFont=\"1\"/>",
    "<xf numFmtId=\"14\" fontId=\"1\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\" applyFont=\"1\"/>",
    "<xf numFmtId=\"22\" fontId=\"1\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\" applyFont=\"1\"/>",
    "<xf numFmtId=\"21\" fontId=\"1\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\" applyFont=\"1\"/>",
    "</cellXfs>",
    "<cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>",
    "<dxfs count=\"0\"/><tableStyles count=\"0\" defaultTableStyle=\"TableStyleMedium2\" defaultPivotStyle=\"PivotStyleLight16\"/>",
    "</styleSheet>"
);

const CORE: &str = concat!(
    "<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" ",
    "xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" ",
    "xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"/>"
);

const APP: &str = concat!(
    "<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\" ",
    "xmlns:vt=\"http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes\"><Application>JFLib</Application></Properties>"
);

// the page margins Excel writes for a new sheet
const MARGINS: &str = "<pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/>";

/// A workbook to write: its sheets in tab order, the first one the active tab.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BookOut {
    /// the sheets, at least one
    pub sheets: Vec<SheetOut>,
}

/// One worksheet to write.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SheetOut {
    /// the tab's name: 1 to 31 characters, none of `[ ] : * ? / \` and no control character, not
    /// starting or ending with `'`, not "History", unlike the other sheets' names in any case
    pub name: String,
    /// the rows from row 1 on, each from column A on; rows may differ in length, Empty (and an
    /// empty Text) is no cell
    pub rows: Vec<Vec<Value>>,
    /// how many of the first rows are a header: bold and kept in view (a frozen pane below them)
    pub header_rows: u32,
    /// column widths from column A on, in characters of the default font's digits as Excel
    /// counts them (0 to 255; 0 hides the column); None, or no entry, estimates the width from
    /// the column's values
    pub widths: Vec<Option<f64>>,
    /// filter buttons on the last header row over the columns that hold values (needs
    /// `header_rows` of 1 or more)
    pub autofilter: bool,
}

impl SheetOut {
    /// A sheet of rows without header, filter or given widths.
    pub fn new(name: &str, rows: Vec<Vec<Value>>) -> SheetOut {
        SheetOut { name: String::from(name), rows, ..SheetOut::default() }
    }
    /// A table as a sheet: a header row of its column names (bold, kept in view), then its rows;
    /// no filter (set `autofilter`), the widths estimated.
    pub fn from_table(name: &str, table: &Table) -> SheetOut {
        let mut rows = Vec::with_capacity(table.rows.len() + 1);
        rows.push(table.columns.iter().map(|c| Value::Text(c.clone())).collect());
        rows.extend(table.rows.iter().cloned());
        SheetOut { name: String::from(name), rows, header_rows: 1, ..SheetOut::default() }
    }
}

/// How a workbook is written and what writing may take.  The default: DEFLATE level 6, ten
/// million cells, 1 GiB of XML.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteOptions {
    /// DEFLATE level 1 (fastest) to 9 (smallest), 0: stored; 6 by default, as diagram_io's
    /// vsdx::write.  For a large sheet 1 is several times faster and the file about a sixth larger
    pub level: u8,
    /// cells with a value in all sheets together
    pub cells: usize,
    /// bytes of the XML of all parts together, before compression: with the rows given, what
    /// bounds the memory writing takes (the compressed package comes on top)
    pub bytes: usize,
}

impl WriteOptions {
    /// The default (see `WriteOptions`).
    pub const DEFAULT: WriteOptions = WriteOptions { level: 6, cells: 10_000_000, bytes: 1 << 30 };
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Why a workbook was not written.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WriteError {
    /// a workbook without sheets (Excel needs one)
    NoSheets,
    /// a sheet's name Excel does not take (see `SheetOut::name`)
    SheetName {
        /// the sheet, 0-based
        sheet: usize,
        /// what is wrong with the name
        what: &'static str,
    },
    /// a sheet's settings: header rows past Excel's last row, a filter without a header row, a
    /// width outside 0 to 255, widths past column XFD
    Sheet {
        /// the sheet, 0-based
        sheet: usize,
        /// what is wrong
        what: &'static str,
    },
    /// a value Excel cannot hold where it stands: past row 1 048 576 or column XFD, a text over
    /// 32 767 characters
    Cell {
        /// the sheet, 0-based
        sheet: usize,
        /// the cell (row and column saturate at u32::MAX)
        at: CellRef,
        /// what is wrong
        what: &'static str,
    },
    /// over a limit: the name of the field of `WriteOptions`
    Limit(&'static str),
    /// the package (archive::opc) refused what was built: never for a workbook that passed the
    /// checks above
    Package(opc::Error),
}

impl WriteError {
    /// The error as a sentence in English.
    pub fn text(&self) -> String {
        match self {
            WriteError::NoSheets => String::from("XLSX: a workbook needs at least one sheet"),
            WriteError::SheetName { sheet, what } => format!("XLSX: the name of sheet {} {what}", sheet + 1),
            WriteError::Sheet { sheet, what } => format!("XLSX: sheet {}: {what}", sheet + 1),
            WriteError::Cell { sheet, at, what } => format!("XLSX: sheet {}, cell {}: {what}", sheet + 1, at.a1()),
            WriteError::Limit(what) => format!("XLSX: over the write limit {what}"),
            WriteError::Package(e) => format!("XLSX: the package was refused ({e:?})"),
        }
    }
}

impl core::fmt::Display for WriteError {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.write_str(&self.text())
    }
}

// a character XML 1.0 does not allow
struct BadChar;

// becomes formats::xml::escape once the base with U5 is merged
fn esc(s: &str) -> Result<String, BadChar> {
    let mut out = String::with_capacity(s.len() + s.len() / 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\r' => out.push_str("&#13;"),
            '\n' => out.push_str("&#10;"),
            '\t' => out.push_str("&#9;"),
            _ if matches!(c as u32, 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF) => out.push(c),
            _ => return Err(BadChar),
        }
    }
    Ok(out)
}

// "_x" and four hex digits: what Excel and the reader decode as an escape
fn escape_like(rest: &[u8]) -> bool {
    rest.len() >= 5 && rest[0] == b'x' && rest[1..5].iter().all(u8::is_ascii_hexdigit)
}

// a cell text in Excel's escapes: controls but TAB and LF, CR, U+FFFE and U+FFFF as _xHHHH_; a
// "_" before x and four hex digits as _x005F_ (whatever follows them: a "_" of an escape made
// here would complete one)
fn excel_escapes(s: &str) -> Cow<'_, str> {
    let special = |c: char| (c < ' ' && c != '\t' && c != '\n') || c == '\u{fffe}' || c == '\u{ffff}';
    let b = s.as_bytes();
    let needs = s.chars().any(special) || b.iter().enumerate().any(|(i, &c)| c == b'_' && escape_like(&b[i + 1..]));
    if !needs {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 16);
    for (i, c) in s.char_indices() {
        if special(c) {
            out.push_str(&format!("_x{:04X}_", c as u32));
        } else if c == '_' && escape_like(&b[i + 1..]) {
            out.push_str("_x005F_");
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

// a sheet name in single quotes for a formula, each ' doubled
fn quoted(name: &str) -> String {
    format!("'{}'", name.replace('\'', "''"))
}

// what a value becomes in a cell
enum Cell<'a> {
    // a finite number
    Number(f64),
    // a text that is not empty
    Text(Cow<'a, str>),
    Bool(bool),
    // one of ECMA-376's seven errors
    Error(&'a str),
    // a serial and the xf of its number format
    Date(f64, u32),
}

fn cell(v: &Value) -> Option<Cell<'_>> {
    Some(match v {
        Value::Empty => return None,
        Value::Text(s) if s.is_empty() => return None,
        Value::Text(s) => Cell::Text(Cow::Borrowed(s)),
        Value::Number(n) if n.is_finite() => Cell::Number(*n),
        Value::Number(_) => Cell::Error("#NUM!"),
        Value::Bool(b) => Cell::Bool(*b),
        Value::Error(e) => match e {
            CellError::Null | CellError::Div0 | CellError::Value | CellError::Ref | CellError::Name | CellError::Num | CellError::NA => Cell::Error(e.text()),
            _ if e.text().is_empty() => return None,
            _ => Cell::Text(Cow::Borrowed(e.text())),
        },
        Value::Date(d) if !d.is_finite() => Cell::Error("#NUM!"),
        Value::Date(d) => match excel_date(*d) {
            Some((serial, xf)) => Cell::Date(serial, xf),
            None => Cell::Text(Cow::Owned(v.text())),
        },
    })
}

// an OLE date as a serial of Excel's 1900 date system and the xf of its number format; None for
// a date Excel cannot show
fn excel_date(v: f64) -> Option<(f64, u32)> {
    let c = civil(v)?;
    if v >= 0.0 && (c.year, c.month, c.day) == (1899, 12, 30) {
        return Some((v, XF_TIME));
    }
    if v < 0.0 || !(1900..=9999).contains(&c.year) {
        return None;
    }
    let xf = if c.hour == 0 && c.minute == 0 && c.second == 0 && c.nanos == 0 { XF_DATE } else { XF_DATETIME };
    // before 1900-03-01 Excel's serials are a day behind; a value a hair below 1900-01-01 that
    // rounds to it stays on serial 1 (below 1 the reader would take it for a time of day)
    let serial = if v < 61.0 { (v - 1.0).max(1.0) } else { v };
    Some((serial, xf))
}

// an East Asian wide or fullwidth character (two columns wide), roughly by block
fn wide(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0x33FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xA000..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x1F300..=0x1F64F | 0x1F900..=0x1F9FF | 0x20000..=0x3FFFD)
}

// the longest line of a text in character widths, at most 255 (Excel's widest column)
fn text_units(s: &str) -> u32 {
    let (mut best, mut line) = (0u32, 0u32);
    for c in s.chars() {
        if c == '\n' || c == '\r' {
            best = best.max(line);
            line = 0;
        } else {
            line += if wide(c) { 2 } else { 1 };
            if line >= 255 {
                return 255;
            }
        }
    }
    best.max(line)
}

// an estimated width from the widest value in tenths of a character: a margin of 1.5, from
// Excel's default 8.43 to 80 (in hundredths, so the text is short)
fn estimate(tenths: u32) -> f64 {
    let hundredths = (tenths.saturating_mul(10).saturating_add(150)).clamp(843, 8000);
    hundredths as f64 / 100.0
}

fn at(row: usize, col: usize) -> CellRef {
    CellRef::new(row.min(u32::MAX as usize) as u32, col.min(u32::MAX as usize) as u32)
}

// the names and settings Excel refuses, before anything is built
fn check(book: &BookOut) -> Result<(), WriteError> {
    if book.sheets.is_empty() {
        return Err(WriteError::NoSheets);
    }
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (i, s) in book.sheets.iter().enumerate() {
        let name = |what| WriteError::SheetName { sheet: i, what };
        let n = s.name.as_str();
        if n.is_empty() {
            return Err(name("is empty"));
        }
        if n.len() > MAX_SHEET_NAME && n.encode_utf16().count() > MAX_SHEET_NAME {
            return Err(name("is longer than Excel's 31 characters"));
        }
        if n.contains(['[', ']', ':', '*', '?', '/', '\\']) {
            return Err(name("holds one of [ ] : * ? / \\"));
        }
        if n.chars().any(|c| c < ' ' || c == '\u{fffe}' || c == '\u{ffff}') {
            return Err(name("holds a control character"));
        }
        if n.starts_with('\'') || n.ends_with('\'') {
            return Err(name("starts or ends with '"));
        }
        if n.eq_ignore_ascii_case("history") {
            return Err(name("is History, which Excel keeps for its change history"));
        }
        if !seen.insert(n.to_lowercase()) {
            return Err(name("is the name of an earlier sheet (Excel compares them case-insensitively)"));
        }
        let sheet = |what| WriteError::Sheet { sheet: i, what };
        if s.header_rows >= MAX_ROWS {
            return Err(sheet("more header rows than Excel's 1 048 576 rows"));
        }
        if s.autofilter && s.header_rows == 0 {
            return Err(sheet("a filter needs a header row"));
        }
        if s.widths.len() > MAX_COLS as usize {
            return Err(sheet("widths past column XFD"));
        }
        if s.widths.iter().flatten().any(|w| !(0.0..=255.0).contains(w)) {
            return Err(sheet("a column width outside 0 to 255"));
        }
    }
    Ok(())
}

// the state of one write
struct W<'a> {
    opts: &'a WriteOptions,
    // the shared strings: text -> index, and their XML so far
    index: BTreeMap<Cow<'a, str>, u32>,
    sst: String,
    // string cells (sst's count)
    refs: u64,
    cells: usize,
    // bytes of the sheet parts done
    done: usize,
    // column letters, A on
    letters: Vec<String>,
    // (sheet, the filter's range with sheet) for workbook.xml's _xlnm._FilterDatabase
    filters: Vec<(usize, String)>,
}

impl<'a> W<'a> {
    fn letters(&mut self, col: usize) -> &str {
        while self.letters.len() <= col {
            let next = col_name(self.letters.len() as u32);
            self.letters.push(next);
        }
        &self.letters[col]
    }

    // the index of a shared string, adding it when it is new
    fn string(&mut self, t: Cow<'a, str>) -> Result<u32, BadChar> {
        self.refs += 1;
        if let Some(&i) = self.index.get(&t) {
            return Ok(i);
        }
        let i = self.index.len() as u32;
        let keep = t.starts_with([' ', '\t', '\n', '\r']) || t.ends_with([' ', '\t', '\n', '\r']);
        self.sst.push_str(if keep { "<si><t xml:space=\"preserve\">" } else { "<si><t>" });
        self.sst.push_str(&esc(&excel_escapes(&t))?);
        self.sst.push_str("</t></si>");
        self.index.insert(t, i);
        Ok(i)
    }

    fn sheet(&mut self, index: usize, s: &'a SheetOut) -> Result<String, WriteError> {
        let header_rows = s.header_rows as usize;
        let mut data = String::new();
        // per column the widest value in tenths of a character
        let mut widest: Vec<u32> = Vec::new();
        // the used range: first row, first column, last row, last column
        let mut used: Option<(usize, usize, usize, usize)> = None;
        for (r, row) in s.rows.iter().enumerate() {
            let header = r < header_rows;
            let mut number = String::new();
            for (c, v) in row.iter().enumerate() {
                let Some(cell) = cell(v) else { continue };
                if r >= MAX_ROWS as usize {
                    return Err(WriteError::Cell { sheet: index, at: at(r, c), what: "a value past Excel's last row 1 048 576" });
                }
                if c >= MAX_COLS as usize {
                    return Err(WriteError::Cell { sheet: index, at: at(r, c), what: "a value past Excel's last column XFD" });
                }
                self.cells += 1;
                if self.cells > self.opts.cells {
                    return Err(WriteError::Limit("cells"));
                }
                if number.is_empty() {
                    number = format!("{}", r + 1);
                    data.push_str("<row r=\"");
                    data.push_str(&number);
                    data.push_str("\">");
                }
                used = Some(match used {
                    None => (r, c, r, c),
                    Some((r0, c0, _, c1)) => (r0, c0.min(c), r, c1.max(c)),
                });
                data.push_str("<c r=\"");
                data.push_str(self.letters(c));
                data.push_str(&number);
                data.push('"');
                let bold = if header { XF_BOLD } else { 0 };
                let style = |data: &mut String, xf: u32| {
                    if xf != 0 {
                        data.push_str(&format!(" s=\"{xf}\""));
                    }
                };
                let units = match cell {
                    Cell::Number(n) => {
                        let t = number_text(n);
                        style(&mut data, bold);
                        data.push_str("><v>");
                        data.push_str(&t);
                        // General shows at most 11 characters
                        t.len().min(11) as u32
                    }
                    Cell::Date(serial, xf) => {
                        style(&mut data, xf + bold);
                        data.push_str("><v>");
                        data.push_str(&number_text(serial));
                        match xf {
                            XF_DATE => 10,
                            XF_DATETIME => 16,
                            _ => 8,
                        }
                    }
                    Cell::Text(t) => {
                        if t.len() > MAX_TEXT && t.encode_utf16().count() > MAX_TEXT {
                            return Err(WriteError::Cell { sheet: index, at: at(r, c), what: "a text over Excel's 32 767 characters" });
                        }
                        let units = text_units(&t);
                        // (the text was made XML-safe by excel_escapes)
                        let i = self.string(t).map_err(|_| WriteError::Cell { sheet: index, at: at(r, c), what: "a character XML cannot hold" })?;
                        style(&mut data, bold);
                        data.push_str(" t=\"s\"><v>");
                        data.push_str(&format!("{i}"));
                        units
                    }
                    Cell::Bool(b) => {
                        style(&mut data, bold);
                        data.push_str(if b { " t=\"b\"><v>1" } else { " t=\"b\"><v>0" });
                        5
                    }
                    Cell::Error(e) => {
                        style(&mut data, bold);
                        data.push_str(" t=\"e\"><v>");
                        data.push_str(e);
                        e.len() as u32
                    }
                };
                data.push_str("</v></c>");
                // (cell by cell: one row of 16384 long texts is gigabytes)
                if self.done.saturating_add(self.sst.len()).saturating_add(data.len()) > self.opts.bytes {
                    return Err(WriteError::Limit("bytes"));
                }
                if widest.len() <= c {
                    widest.resize(c + 1, 0);
                }
                let tenths = units * if header { 11 } else { 10 };
                widest[c] = widest[c].max(tenths);
            }
            if !number.is_empty() {
                data.push_str("</row>");
            }
        }

        let mut x = String::with_capacity(data.len() + 1024);
        x.push_str(DECL);
        x.push_str(&format!("<worksheet xmlns=\"{NS_MAIN}\" xmlns:r=\"{NS_R}\">"));
        let dimension = match used {
            Some((r0, c0, r1, c1)) => Area::new(None, at(r0, c0), at(r1, c1)).a1(),
            None => String::from("A1"),
        };
        x.push_str(&format!("<dimension ref=\"{dimension}\"/>"));
        let tab = if index == 0 { " tabSelected=\"1\"" } else { "" };
        if header_rows > 0 {
            let top = format!("A{}", header_rows + 1);
            x.push_str(&format!(
                "<sheetViews><sheetView{tab} workbookViewId=\"0\"><pane ySplit=\"{header_rows}\" topLeftCell=\"{top}\" activePane=\"bottomLeft\" state=\"frozen\"/><selection pane=\"bottomLeft\" activeCell=\"{top}\" sqref=\"{top}\"/></sheetView></sheetViews>"
            ));
        } else {
            x.push_str(&format!("<sheetViews><sheetView{tab} workbookViewId=\"0\"/></sheetViews>"));
        }
        x.push_str("<sheetFormatPr defaultRowHeight=\"15\"/>");
        // the columns: runs of equal width in one <col>
        let ncols = widest.len().max(s.widths.len());
        if ncols > 0 {
            x.push_str("<cols>");
            let width = |c: usize| s.widths.get(c).copied().flatten().unwrap_or_else(|| estimate(widest.get(c).copied().unwrap_or(0)));
            let mut c = 0;
            while c < ncols {
                let w = width(c);
                let mut end = c;
                while end + 1 < ncols && width(end + 1) == w {
                    end += 1;
                }
                let hidden = if w == 0.0 { " hidden=\"1\"" } else { "" };
                x.push_str(&format!("<col min=\"{}\" max=\"{}\" width=\"{}\"{hidden} customWidth=\"1\"/>", c + 1, end + 1, number_text(w)));
                c = end + 1;
            }
            x.push_str("</cols>");
        }
        if data.is_empty() {
            x.push_str("<sheetData/>");
        } else {
            x.push_str("<sheetData>");
            x.push_str(&data);
            x.push_str("</sheetData>");
        }
        drop(data);
        if let (true, Some((_, _, last_row, last_col))) = (s.autofilter, used) {
            let first = CellRef::new(header_rows as u32 - 1, 0);
            let last = at(last_row.max(header_rows - 1), last_col);
            let area = format!("{}:{}", first.a1(), last.a1());
            x.push_str(&format!("<autoFilter ref=\"{area}\"/>"));
            let absolute = format!("${}${}:${}${}", col_name(first.col), first.row + 1, col_name(last.col), last.row as u64 + 1);
            self.filters.push((index, format!("{}!{absolute}", quoted(&s.name))));
        }
        x.push_str(MARGINS);
        x.push_str("</worksheet>");
        Ok(x)
    }
}

/// Write a workbook as XLSX (see the module's head for how values, dates and widths are written
/// and what Excel's limits are).  Deterministic: the same book gives the same bytes.
pub fn write(book: &BookOut, opts: &WriteOptions) -> Result<Vec<u8>, WriteError> {
    check(book)?;
    let mut w = W { opts, index: BTreeMap::new(), sst: String::new(), refs: 0, cells: 0, done: 0, letters: Vec::new(), filters: Vec::new() };
    let mut sheets: Vec<String> = Vec::with_capacity(book.sheets.len());
    for (i, s) in book.sheets.iter().enumerate() {
        let x = w.sheet(i, s)?;
        w.done = w.done.saturating_add(x.len());
        sheets.push(x);
    }
    let bad_name = |sheet: usize| WriteError::SheetName { sheet, what: "holds a character XML cannot hold" };

    // workbook.xml: the sheets, the filters' names
    let mut wb = format!("{DECL}<workbook xmlns=\"{NS_MAIN}\" xmlns:r=\"{NS_R}\">");
    wb.push_str("<bookViews><workbookView xWindow=\"240\" yWindow=\"15\" windowWidth=\"16095\" windowHeight=\"9660\"/></bookViews><sheets>");
    for (i, s) in book.sheets.iter().enumerate() {
        let name = esc(&s.name).map_err(|_| bad_name(i))?;
        wb.push_str(&format!("<sheet name=\"{name}\" sheetId=\"{}\" r:id=\"rId{}\"/>", i + 1, i + 1));
    }
    wb.push_str("</sheets>");
    if !w.filters.is_empty() {
        wb.push_str("<definedNames>");
        for (i, area) in &w.filters {
            let area = esc(area).map_err(|_| bad_name(*i))?;
            wb.push_str(&format!("<definedName name=\"_xlnm._FilterDatabase\" localSheetId=\"{i}\" hidden=\"1\">{area}</definedName>"));
        }
        wb.push_str("</definedNames>");
    }
    wb.push_str("</workbook>");

    let n = book.sheets.len();
    let rel = |id: usize, kind: String, target: String| Relationship { id: format!("rId{id}"), kind, target, external: false };
    let mut wb_rels: Vec<Relationship> = (0..n).map(|i| rel(i + 1, format!("{NS_R}/worksheet"), format!("worksheets/sheet{}.xml", i + 1))).collect();
    wb_rels.push(rel(n + 1, format!("{NS_R}/styles"), String::from("styles.xml")));
    let sst = if w.index.is_empty() {
        None
    } else {
        wb_rels.push(rel(n + 2, format!("{NS_R}/sharedStrings"), String::from("sharedStrings.xml")));
        Some(format!("{DECL}<sst xmlns=\"{NS_MAIN}\" count=\"{}\" uniqueCount=\"{}\">{}</sst>", w.refs, w.index.len(), w.sst))
    };
    drop(w.index);
    drop(w.sst);
    let styles = format!("{DECL}{STYLES}");
    let core = format!("{DECL}{CORE}");
    let app = format!("{DECL}{APP}");
    let total = [wb.len(), styles.len(), core.len(), app.len(), sst.as_ref().map_or(0, String::len)].iter().fold(w.done, |a, &b| a.saturating_add(b));
    if total > opts.bytes {
        return Err(WriteError::Limit("bytes"));
    }

    // the package, its parts in Excel's order
    let pkg_err = WriteError::Package;
    let mut pkg = Package::new(Rules::DEFAULT);
    pkg.content_types_mut().set_default("rels", opc::RELS_TYPE).map_err(pkg_err)?;
    pkg.content_types_mut().set_default("xml", "application/xml").map_err(pkg_err)?;
    let package_rels = [
        rel(1, String::from(opc::OFFICE_DOCUMENT), String::from("xl/workbook.xml")),
        rel(2, String::from(REL_CORE), String::from("docProps/core.xml")),
        rel(3, String::from(REL_APP), String::from("docProps/app.xml")),
    ];
    pkg.set_relationships("", &package_rels).map_err(pkg_err)?;
    pkg.set_part("xl/workbook.xml", CT_WORKBOOK, wb.into_bytes()).map_err(pkg_err)?;
    pkg.set_relationships("xl/workbook.xml", &wb_rels).map_err(pkg_err)?;
    for (i, x) in sheets.into_iter().enumerate() {
        pkg.set_part(&format!("xl/worksheets/sheet{}.xml", i + 1), CT_SHEET, x.into_bytes()).map_err(pkg_err)?;
    }
    pkg.set_part("xl/styles.xml", CT_STYLES, styles.into_bytes()).map_err(pkg_err)?;
    if let Some(sst) = sst {
        pkg.set_part("xl/sharedStrings.xml", CT_STRINGS, sst.into_bytes()).map_err(pkg_err)?;
    }
    pkg.set_part("docProps/core.xml", CT_CORE, core.into_bytes()).map_err(pkg_err)?;
    pkg.set_part("docProps/app.xml", CT_APP, app.into_bytes()).map_err(pkg_err)?;
    // the parts are bounded by WriteOptions already; the ZIP itself takes what they need
    let zip = archive::Limits { entries: usize::MAX, part_bytes: usize::MAX, total_bytes: usize::MAX, archive_bytes: usize::MAX, ..archive::Limits::default() };
    let method = if opts.level == 0 { Method::Stored } else { Method::Deflate(opts.level) };
    pkg.write(zip, method).map_err(pkg_err)
}
