// textdraw -- (JD18, IM-08) a connected drawing from a text file: records of the types "Shape"
// and "Link" (Visio's text import filter; Visio 5.0 user guide, chapter 21, "Creating connected
// drawings from text files") read into a plan, which then becomes shapes and dynamic connectors
// on a page
//
//   parse(text, &Options, &Limits) -> Plan   the records: shapes (ID, master, text, position,
//                       size, shape data), links (ID, master, text, from, to), the master names
//                       they use, the problems with their line numbers; it never fails (a
//                       record it cannot use is a problem and left out)
//   detect_sep(text, fallback)              the field separator the records use
//   layout(&sizes, &edges, gap) -> centres  the automatic arrangement of the shapes without a
//                       position (top to bottom in layers, see below)
//   apply(tx, eng, page, &plan, &masters, &Build) -> Built   the plan on a page inside the
//                       caller's transaction: instances of the chosen masters (else plain
//                       rectangles), text, size, shape data, laid out; the links as dynamic
//                       connectors (or instances of a chosen 1-D master) glued to the whole
//                       shapes and routed.  The caller takes stencil masters into the document
//                       first (instance::import_master) and passes one master (or None) per
//                       Plan::masters entry.
//
// The format as the guide describes it, and what is decided here where it says nothing:
// * One record per line (CR, LF or CR LF ends it); the first field names the record type,
//   "Shape" or "Link" (any case).  Another type is a warning and the line is left out.
// * Fields are separated by a comma (the guide: the default of .csv files), a tab (.txt files)
//   or -- decided here, the CSV of a German Excel -- a semicolon.  The separator is the
//   character right after the type word of the first record or field heading (";Shape,ID,..."),
//   else the caller's fallback (the file's extension).
// * A field in double quotes is text as it stands: separators, line breaks (kept as paragraph
//   breaks, '\n') and doubled quotes ("" for ") inside; an unquoted field is trimmed of spaces.
//   A quote left open at the end of the file is an error and its record is left out.
// * A line that starts with a semicolon (spaces before it allowed) is a comment.  Decided here:
//   a comment that lists the field headings of Shape records (";Shape,ID,Master,Text,X,Y,Width,
//   Height,Cost,Owner") names the shape data fields of the records after it by the headings
//   past Height (a heading "Property" names nothing).
// * Shape: ID, Master, Text, X, Y, Width, Height, then shape data fields.  Link: ID, Master,
//   Text, From, To (the IDs of two shapes; further fields are ignored).  Empty fields are
//   allowed (the guide's example leaves Master and the link IDs empty).
// * IDs are compared trimmed and without regard to ASCII case.  A shape without an ID is drawn
//   but cannot be linked (a warning); a second shape with the same ID is an error and left out;
//   a link whose end is missing or names no shape is an error, a link from a shape to itself as
//   well (both left out); two links with the same ID only a warning.
// * Masters are named by the master's name (NameU or the name in any language; the caller maps
//   each name to a master of its stencils or of the document).  An empty master is the default:
//   a plain rectangle for a shape, the dynamic connector for a link.  A chosen master that does
//   not fit (a 1-D master for shapes, a 2-D one for links) is replaced by the default
//   (Built::unfit names it).
// * Numbers (X, Y, Width, Height) are in the unit the caller gives (Options::per_number, drawing
//   inches per number: the guide says nothing about units; the wizard asks), or carry their own
//   unit: "25 mm", "2.5cm", "1 m", "1.5 in", "1.5\"", "2 ft", "36 pt".  A decimal comma is
//   taken when there is no point ("2,5").  X and Y are the middle of the shape (its pin for
//   nearly every master) in page coordinates as Visio has them: from the page's bottom left
//   corner, y up.  A number that does not read, X without Y and the like are warnings: the
//   record is drawn without that value.  Lengths beyond 100 000 inches and sizes <= 0 are
//   refused.  Width and Height replace the master's size (around the same middle).
// * A shape data field "Name=Value" sets field Name (Name: letters, digits, spaces, '_', '-',
//   '.' only, at most 64 characters, else the whole field is the value); otherwise the field's
//   heading names it, else "Property1", "Property2" ... by its place.  An empty field sets
//   nothing.  A value of the form -12 / 12.5 / 12,5 (no leading zeros) becomes a number (Type 2),
//   everything else text.  A field the shape has already (by name or by label, any case) gets
//   the value; otherwise a new Property row (its name made of letters, digits and '_').
//
// The automatic layout (layout()), for the shapes without X and Y: each connected group of shapes
// (the links between them) is laid out in layers from top to bottom -- the guide's example, an
// organisation chart, comes out as its picture shows.  Cycles are broken where a depth-first walk
// meets a shape on its own path; a shape's layer is one below the lowest layer of the shapes
// linking to it (longest path), a shape that no link reaches sits right above its highest
// successor; the order within a layer starts as the file's order, then follows the mean place of
// the neighbours (barycentre sweeps down and up); the x places are where the neighbours want
// them, the overlaps taken out with the least movement (isotonic regression: order kept, the gap
// between neighbours), in alternating passes up and down, so children centre under their parents
// and parents over their children.  The groups (and single shapes) are then packed left to right
// in rows no wider than about 1.5 times the square root of their total area (a grid for shapes
// without links).  The whole block starts at the page's top left corner (Build::margin inside
// it), right of the page's earlier shapes and of the shapes placed by X and Y.
//
// Every text is untrusted input: its size, the number of records, of fields per record, of shape
// data fields per shape, the length of each field and of an ID and the number of problems are
// bounded (Limits); the walks of the layout use explicit stacks (no recursion over the file).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use diagram_core::formula::math::sqrt;
use diagram_core::formula::Engine;
use diagram_core::geometry::Xform;
use diagram_core::{glue, instance, ops, pagesetup, CellAddr, EditError, MasterRef, Owner, PageRef, RowKey, SecKey, ShapeRef, SheetRef, Text, Transaction, Unit, Value};

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------- the plan

/// how to read the numbers and the fields
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// the field separator; None: detected (detect_sep), else `fallback_sep`
    pub sep: Option<char>,
    /// the separator when none is detected (',' for .csv, '\t' for .txt)
    pub fallback_sep: char,
    /// drawing inches per number without a unit (1/25.4: the numbers are millimetres)
    pub per_number: f64,
}

impl Default for Options {
    fn default() -> Options {
        Options { sep: None, fallback_sep: ',', per_number: 1.0 }
    }
}

/// the bounds of a text (untrusted input)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// the text's size in bytes
    pub max_bytes: usize,
    /// Shape and Link records together
    pub max_records: usize,
    /// fields of one record (more are left out)
    pub max_fields: usize,
    /// characters of one field (more are cut)
    pub max_field_chars: usize,
    /// characters of an ID (a longer one is an error)
    pub max_id_chars: usize,
    /// shape data fields of one shape
    pub max_props: usize,
    /// problems kept (then one TooManyProblems)
    pub max_problems: usize,
}

impl Limits {
    /// what the app reads: 16 MiB, 10 000 records, 1 000 fields of up to 32 768 characters
    pub const DEFAULT: Limits = Limits { max_bytes: 16 << 20, max_records: 10_000, max_fields: 1_000, max_field_chars: 32_768, max_id_chars: 256, max_props: 256, max_problems: 1_000 };
}

/// a shape data field of a shape record
#[derive(Clone, Debug, PartialEq)]
pub struct Prop {
    /// the row's name (Prop.<name>): letters, digits, '_'
    pub name: String,
    /// the label as the file names it
    pub label: String,
    /// the value as written
    pub value: String,
}

/// a Shape record
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeRec {
    /// the line it starts on (from 1)
    pub line: u32,
    /// its ID (empty: none, it cannot be linked)
    pub id: String,
    /// index into Plan::masters
    pub master: usize,
    /// the text ('\n' between paragraphs)
    pub text: String,
    /// X and Y: the middle (drawing inches, y up; None: laid out)
    pub pos: Option<[f64; 2]>,
    /// Width and Height (drawing inches; None: the master's or the default)
    pub size: [Option<f64>; 2],
    /// shape data
    pub props: Vec<Prop>,
}

/// a Link record
#[derive(Clone, Debug, PartialEq)]
pub struct LinkRec {
    /// the line it starts on (from 1)
    pub line: u32,
    /// its ID (may be empty)
    pub id: String,
    /// index into Plan::masters
    pub master: usize,
    /// the connector's text
    pub text: String,
    /// the shapes it connects (indices into Plan::shapes)
    pub from: usize,
    pub to: usize,
}

/// a master name as the records use it: one entry per name and kind of record
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MasterUse {
    /// the name as first written ("" for the default)
    pub name: String,
    /// used by Link records (a 1-D master is wanted), else by Shape records
    pub link: bool,
    /// how many records use it
    pub count: usize,
}

/// a number field of a Shape record
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumField {
    X,
    Y,
    Width,
    Height,
}

/// what is wrong with a record or the text
#[derive(Clone, Debug, PartialEq)]
pub enum Issue {
    /// the text is larger than Limits::max_bytes: nothing read
    TooBig,
    /// more records than Limits::max_records: the rest is left out
    TooManyRecords,
    /// more problems than Limits::max_problems: the rest is not listed
    TooManyProblems,
    /// a record type other than Shape and Link (the type as written, cut to 40 characters)
    UnknownRecord(String),
    /// a quote left open at the end of the text
    UnclosedQuote,
    /// more fields than Limits::max_fields (the rest left out)
    TooManyFields,
    /// a field longer than Limits::max_field_chars (cut)
    FieldTooLong,
    /// an ID longer than Limits::max_id_chars
    IdTooLong,
    /// a shape without an ID (drawn, but no link can reach it)
    NoId,
    /// a second shape with this ID
    DuplicateId(String),
    /// a second link with this ID (kept)
    DuplicateLinkId(String),
    /// a link without From or To
    NoEnd,
    /// a link to an ID no shape has
    UnknownEnd(String),
    /// a link from a shape to itself
    SelfLink(String),
    /// a number field that does not read (the field, the text)
    BadNumber(NumField, String),
    /// a number out of range (beyond 100 000 inches, a size <= 0)
    OutOfRange(NumField, String),
    /// X without Y or Y without X: laid out instead
    HalfPosition,
    /// more shape data fields than Limits::max_props (the rest left out)
    TooManyProps,
}

/// a problem of the text, at its line
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    /// the line (from 1; 0: the whole text)
    pub line: u32,
    pub issue: Issue,
    /// the record was left out (an error), else only a warning
    pub skipped: bool,
}

/// what a text says
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Plan {
    /// the separator used
    pub sep: char,
    /// it was detected from the records (not the fallback, not given)
    pub sep_detected: bool,
    pub shapes: Vec<ShapeRec>,
    pub links: Vec<LinkRec>,
    /// the master names the records use, shapes' first, each in the order it first appears
    pub masters: Vec<MasterUse>,
    pub problems: Vec<Problem>,
    /// comment lines
    pub comments: usize,
}

impl Plan {
    /// the problems that left a record out
    pub fn errors(&self) -> usize {
        self.problems.iter().filter(|p| p.skipped).count()
    }

    /// the problems that did not
    pub fn warnings(&self) -> usize {
        self.problems.len() - self.errors()
    }
}

// ---------------------------------------------------------------- reading

/// the separator of the records: the character right after "Shape" or "Link" in the first
/// record or comment heading that has ',', a tab or ';' there -> (it, detected); else
/// (`fallback`, false)
pub fn detect_sep(text: &str, fallback: char) -> (char, bool) {
    for line in text.split(['\n', '\r']) {
        let mut l = line.trim_start_matches([' ', '\u{feff}']);
        if let Some(rest) = l.strip_prefix(';') {
            l = rest.trim_start_matches(' ');
        }
        let quoted = l.starts_with('"');
        if quoted {
            l = &l[1..];
        }
        for kw in ["shape", "link"] {
            let Some(head) = l.get(..kw.len()) else { continue };
            if !head.eq_ignore_ascii_case(kw) {
                continue;
            }
            let mut rest = &l[kw.len()..];
            if quoted {
                match rest.strip_prefix('"') {
                    Some(r) => rest = r,
                    None => continue,
                }
            }
            let rest = rest.trim_start_matches(' ');
            if let Some(c @ (',' | '\t' | ';')) = rest.chars().next() {
                return (c, true);
            }
        }
    }
    (fallback, false)
}

// one record as the tokenizer gives it
struct Raw {
    line: u32,
    fields: Vec<String>,
    unclosed: bool,
    too_long: bool,
    too_many: bool,
}

// what the tokenizer gives next
enum Item {
    Record(Raw),
    Comment(String),
}

// the records of a text, one at a time
struct Tok<'a> {
    chars: core::iter::Peekable<core::str::Chars<'a>>,
    line: u32,
    sep: char,
    max_fields: usize,
    max_chars: usize,
}

impl Tok<'_> {
    // a line break at the current place (CR LF counts once) -> consumed
    fn eol(&mut self) -> bool {
        match self.chars.peek() {
            Some('\n') => {
                self.chars.next();
                self.line = self.line.saturating_add(1);
                true
            }
            Some('\r') => {
                self.chars.next();
                if self.chars.peek() == Some(&'\n') {
                    self.chars.next();
                }
                self.line = self.line.saturating_add(1);
                true
            }
            _ => false,
        }
    }

    // the rest of the line (a comment) -> its text, cut at max_chars
    fn rest_of_line(&mut self) -> String {
        let mut s = String::new();
        let mut n = 0usize;
        while let Some(&c) = self.chars.peek() {
            if c == '\n' || c == '\r' {
                break;
            }
            self.chars.next();
            if n < self.max_chars {
                s.push(c);
                n += 1;
            }
        }
        self.eol();
        s
    }

    // the next record or comment (None at the end)
    fn next_item(&mut self) -> Option<Item> {
        // empty lines and the spaces before a record
        loop {
            while self.chars.peek() == Some(&' ') {
                self.chars.next();
            }
            if !self.eol() {
                break;
            }
        }
        self.chars.peek()?;
        if self.chars.peek() == Some(&';') {
            self.chars.next();
            return Some(Item::Comment(self.rest_of_line()));
        }
        let mut raw = Raw { line: self.line, fields: Vec::new(), unclosed: false, too_long: false, too_many: false };
        loop {
            let (field, end) = self.field(&mut raw);
            if raw.fields.len() < self.max_fields {
                raw.fields.push(field);
            } else {
                raw.too_many = true;
            }
            if end {
                break;
            }
        }
        Some(Item::Record(raw))
    }

    // one field -> (its text, the record ended after it)
    fn field(&mut self, raw: &mut Raw) -> (String, bool) {
        let mut s = String::new();
        let mut n = 0usize;
        let max = self.max_chars;
        let mut push = |s: &mut String, c: char, raw: &mut Raw| {
            if n < max {
                s.push(c);
                n += 1;
            } else {
                raw.too_long = true;
            }
        };
        while self.chars.peek() == Some(&' ') {
            self.chars.next();
        }
        if self.chars.peek() == Some(&'"') {
            self.chars.next();
            loop {
                match self.chars.next() {
                    None => {
                        raw.unclosed = true;
                        return (s, true);
                    }
                    Some('"') => {
                        if self.chars.peek() == Some(&'"') {
                            self.chars.next();
                            push(&mut s, '"', raw);
                        } else {
                            break;
                        }
                    }
                    Some('\r') => {
                        if self.chars.peek() == Some(&'\n') {
                            self.chars.next();
                        }
                        self.line = self.line.saturating_add(1);
                        push(&mut s, '\n', raw);
                    }
                    Some('\n') => {
                        self.line = self.line.saturating_add(1);
                        push(&mut s, '\n', raw);
                    }
                    Some(c) => push(&mut s, c, raw),
                }
            }
            // after the closing quote: what stands before the separator is kept (lenient)
            let mut end = false;
            let mut tail = String::new();
            loop {
                match self.chars.peek() {
                    None => {
                        end = true;
                        break;
                    }
                    Some(&c) if c == self.sep => {
                        self.chars.next();
                        break;
                    }
                    Some('\n' | '\r') => {
                        self.eol();
                        end = true;
                        break;
                    }
                    Some(&c) => {
                        self.chars.next();
                        if tail.len() < max {
                            tail.push(c);
                        }
                    }
                }
            }
            for c in tail.trim_end().chars() {
                push(&mut s, c, raw);
            }
            return (s, end);
        }
        let mut end = true;
        loop {
            match self.chars.peek() {
                None => break,
                Some(&c) if c == self.sep => {
                    self.chars.next();
                    end = false;
                    break;
                }
                Some('\n' | '\r') => {
                    self.eol();
                    break;
                }
                Some(&c) => {
                    self.chars.next();
                    push(&mut s, c, raw);
                }
            }
        }
        let t = s.trim_end().len();
        s.truncate(t);
        (s, end)
    }
}

/// a length "number [unit]" -> drawing inches (`per_number` inches per number without a unit;
/// None: no number, an unknown unit)
pub fn length(s: &str, per_number: f64) -> Option<f64> {
    let s = s.trim();
    // the number: digits, sign, decimal marks and an exponent with digits after it
    let b = s.as_bytes();
    let mut k = 0;
    while k < b.len() {
        let c = b[k];
        let exp = (c == b'e' || c == b'E') && k > 0 && b.get(k + 1).is_some_and(|d| d.is_ascii_digit() || *d == b'-' || *d == b'+');
        if c.is_ascii_digit() || matches!(c, b'.' | b',' | b'-' | b'+') || exp {
            k += 1;
        } else {
            break;
        }
    }
    let (num, unit) = s.split_at(k);
    let v = number(num)?;
    let factor = match unit.trim().to_ascii_lowercase().as_str() {
        "" => per_number,
        "mm" => 1.0 / 25.4,
        "cm" => 1.0 / 2.54,
        "m" => 1.0 / 0.0254,
        "in" | "\"" => 1.0,
        "ft" => 12.0,
        "pt" => 1.0 / 72.0,
        _ => return None,
    };
    let x = v * factor;
    x.is_finite().then_some(x)
}

// a number with '.' or a single ',' as the decimal mark
fn number(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() || s.len() > 64 {
        return None;
    }
    let owned;
    let s = if !s.contains('.') && s.matches(',').count() == 1 {
        owned = s.replace(',', ".");
        owned.as_str()
    } else {
        s
    };
    if s.contains(',') {
        return None;
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// a shape data value that is a number: -12, 12.5, 12,5 (no leading zeros, no exponent, no
/// plus sign: "0049", "+49" and "1e3" stay text)
pub fn plain_number(s: &str) -> Option<f64> {
    let t = s.strip_prefix('-').unwrap_or(s);
    let (int, frac) = match t.find(['.', ',']) {
        Some(k) => (&t[..k], Some(&t[k + 1..])),
        None => (t, None),
    };
    let digits = |x: &str| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit());
    if t.len() > 30 || !digits(int) || (int.len() > 1 && int.starts_with('0')) || frac.is_some_and(|f| !digits(f)) {
        return None;
    }
    number(s)
}

// "Name=Value" when the part before '=' is a name (see the header) -> (name, value)
fn prop_name(field: &str) -> Option<(&str, &str)> {
    let (n, v) = field.split_once('=')?;
    let n = n.trim();
    let ok = !n.is_empty() && n.chars().count() <= 64 && n.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '-' | '.'));
    ok.then(|| (n, v.trim()))
}

/// a name as a shape data row name: letters, digits and '_', not starting with a digit
pub fn row_name(s: &str) -> String {
    let mut out: String = s.trim().chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

// an ID as compared (trimmed, ASCII lower case)
fn id_key(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

// a heading that names no field
fn generic_heading(h: &str) -> bool {
    let h = h.trim().to_ascii_lowercase();
    h.is_empty() || matches!(h.as_str(), "property" | "properties" | "eigenschaft" | "eigenschaften")
}

// a Link record until every shape is known
struct PendingLink {
    line: u32,
    id: String,
    master: String,
    text: String,
    from: String,
    to: String,
}

struct Reader<'a> {
    plan: Plan,
    lim: &'a Limits,
    // (lower case name, link) -> index into plan.masters
    masters: BTreeMap<(String, bool), usize>,
    full: bool,
}

impl Reader<'_> {
    fn problem(&mut self, line: u32, issue: Issue, skipped: bool) {
        if self.full {
            return;
        }
        if self.plan.problems.len() >= self.lim.max_problems {
            self.plan.problems.push(Problem { line, issue: Issue::TooManyProblems, skipped: false });
            self.full = true;
            return;
        }
        self.plan.problems.push(Problem { line, issue, skipped });
    }

    fn master(&mut self, name: &str, link: bool) -> usize {
        let name = name.trim();
        let key = (name.to_lowercase(), link);
        if let Some(&k) = self.masters.get(&key) {
            self.plan.masters[k].count += 1;
            return k;
        }
        let k = self.plan.masters.len();
        self.plan.masters.push(MasterUse { name: String::from(name), link, count: 1 });
        self.masters.insert(key, k);
        k
    }

    // a number field of a shape record (None: empty or refused, a warning noted)
    fn num(&mut self, line: u32, s: &str, which: NumField, per_number: f64) -> Option<f64> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        let shown = || s.chars().take(40).collect::<String>();
        match length(s, per_number) {
            None => {
                self.problem(line, Issue::BadNumber(which, shown()), false);
                None
            }
            Some(v) if v.abs() > 100_000.0 || (matches!(which, NumField::Width | NumField::Height) && v <= 0.0) => {
                self.problem(line, Issue::OutOfRange(which, shown()), false);
                None
            }
            Some(v) => Some(v),
        }
    }
}

/// read a text (see the file header) -> the plan; never fails
pub fn parse(text: &str, opts: &Options, lim: &Limits) -> Plan {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let (sep, detected) = match opts.sep {
        Some(s) => (s, false),
        None => detect_sep(text, opts.fallback_sep),
    };
    let mut r = Reader { plan: Plan { sep, sep_detected: detected, ..Plan::default() }, lim, masters: BTreeMap::new(), full: false };
    if text.len() > lim.max_bytes {
        r.problem(0, Issue::TooBig, true);
        return r.plan;
    }
    let mut tok = Tok { chars: text.chars().peekable(), line: 1, sep, max_fields: lim.max_fields.max(1), max_chars: lim.max_field_chars };
    // the shape data headings of the last ";Shape,..." comment (index 0: the 9th field)
    let mut headings: Vec<String> = Vec::new();
    // shape ID (id_key) -> index; links are resolved at the end (they may come first)
    let mut ids: BTreeMap<String, usize> = BTreeMap::new();
    let mut pending: Vec<PendingLink> = Vec::new();
    let mut records = 0usize;
    while let Some(item) = tok.next_item() {
        let raw = match item {
            Item::Comment(c) => {
                r.plan.comments += 1;
                let fields: Vec<&str> = c.split(sep).map(|f| f.trim().trim_matches('"').trim()).collect();
                if fields.first().is_some_and(|f| f.eq_ignore_ascii_case("shape")) {
                    headings = fields.iter().skip(8).take(lim.max_props).map(|h| if generic_heading(h) { String::new() } else { String::from(*h) }).collect();
                }
                continue;
            }
            Item::Record(raw) => raw,
        };
        let line = raw.line;
        if raw.fields.iter().all(|f| f.is_empty()) && !raw.unclosed {
            continue;
        }
        if raw.unclosed {
            r.problem(line, Issue::UnclosedQuote, true);
            continue;
        }
        let kind = raw.fields[0].trim().to_ascii_lowercase();
        if kind != "shape" && kind != "link" {
            let shown: String = raw.fields[0].chars().take(40).collect();
            r.problem(line, Issue::UnknownRecord(shown), true);
            continue;
        }
        if records >= lim.max_records {
            r.problem(line, Issue::TooManyRecords, true);
            break;
        }
        records += 1;
        if raw.too_many {
            r.problem(line, Issue::TooManyFields, false);
        }
        if raw.too_long {
            r.problem(line, Issue::FieldTooLong, false);
        }
        let f = |k: usize| raw.fields.get(k).map_or("", |s| s.as_str());
        let id = String::from(f(1).trim());
        if id.chars().count() > lim.max_id_chars {
            r.problem(line, Issue::IdTooLong, true);
            continue;
        }
        if kind == "link" {
            pending.push(PendingLink { line, id, master: String::from(f(2)), text: String::from(f(3)), from: String::from(f(4).trim()), to: String::from(f(5).trim()) });
            continue;
        }
        if id.is_empty() {
            r.problem(line, Issue::NoId, false);
        } else if ids.contains_key(&id_key(&id)) {
            r.problem(line, Issue::DuplicateId(id), true);
            continue;
        }
        let per = opts.per_number;
        let x = r.num(line, f(4), NumField::X, per);
        let y = r.num(line, f(5), NumField::Y, per);
        let w = r.num(line, f(6), NumField::Width, per);
        let h = r.num(line, f(7), NumField::Height, per);
        let pos = match (x, y) {
            (Some(x), Some(y)) => Some([x, y]),
            (None, None) => None,
            _ => {
                r.problem(line, Issue::HalfPosition, false);
                None
            }
        };
        let mut props: Vec<Prop> = Vec::new();
        for (k, field) in raw.fields.iter().enumerate().skip(8) {
            let field = field.trim_matches(' ');
            if field.is_empty() {
                continue;
            }
            if props.len() >= lim.max_props {
                r.problem(line, Issue::TooManyProps, false);
                break;
            }
            let (label, value) = match (prop_name(field), headings.get(k - 8).filter(|h| !h.is_empty())) {
                (Some((n, v)), _) => (String::from(n), String::from(v)),
                (None, Some(h)) => (h.clone(), String::from(field)),
                (None, None) => (format!("Property{}", k - 7), String::from(field)),
            };
            // a second field of the same name gets a number
            let base = row_name(&label);
            let mut name = base.clone();
            let mut n = 2u32;
            while props.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
                name = format!("{base}{n}");
                n += 1;
            }
            props.push(Prop { name, label, value });
        }
        let master = r.master(f(2), false);
        if !id.is_empty() {
            ids.insert(id_key(&id), r.plan.shapes.len());
        }
        r.plan.shapes.push(ShapeRec { line, id, master, text: String::from(f(3)), pos, size: [w, h], props });
    }
    // the links, now that every shape is known
    let mut link_ids: BTreeSet<String> = BTreeSet::new();
    for l in pending {
        if l.from.is_empty() || l.to.is_empty() {
            r.problem(l.line, Issue::NoEnd, true);
            continue;
        }
        let a = ids.get(&id_key(&l.from)).copied();
        let b = ids.get(&id_key(&l.to)).copied();
        let (Some(a), Some(b)) = (a, b) else {
            let missing = if a.is_none() { l.from } else { l.to };
            r.problem(l.line, Issue::UnknownEnd(missing.chars().take(40).collect()), true);
            continue;
        };
        if a == b {
            r.problem(l.line, Issue::SelfLink(l.from), true);
            continue;
        }
        if !l.id.is_empty() && !link_ids.insert(id_key(&l.id)) {
            r.problem(l.line, Issue::DuplicateLinkId(l.id.clone()), false);
        }
        let master = r.master(&l.master, true);
        r.plan.links.push(LinkRec { line: l.line, id: l.id, master, text: l.text, from: a, to: b });
    }
    r.plan.problems.sort_by_key(|p| p.line);
    r.plan
}

// ---------------------------------------------------------------- layout

/// the arrangement of shapes of `sizes` (width, height) linked by `edges` (from, to; indices into
/// sizes): their middles, x to the right and y down from the block's top left corner (see the
/// file header); `gap`: the space across and down between shapes
pub fn layout(sizes: &[[f64; 2]], edges: &[(usize, usize)], gap: [f64; 2]) -> Vec<[f64; 2]> {
    let n = sizes.len();
    let mut out = vec![[0.0; 2]; n];
    if n == 0 {
        return out;
    }
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut pred: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut seen: BTreeSet<(usize, usize)> = BTreeSet::new();
    for &(a, b) in edges {
        if a < n && b < n && a != b && seen.insert((a, b)) {
            succ[a].push(b);
            pred[b].push(a);
        }
    }
    // the connected groups, in the order of their first shape
    let mut comp = vec![usize::MAX; n];
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for s in 0..n {
        if comp[s] != usize::MAX {
            continue;
        }
        let c = groups.len();
        comp[s] = c;
        let mut members = Vec::new();
        let mut stack = vec![s];
        while let Some(v) = stack.pop() {
            members.push(v);
            for &w in succ[v].iter().chain(pred[v].iter()) {
                if comp[w] == usize::MAX {
                    comp[w] = c;
                    stack.push(w);
                }
            }
        }
        members.sort_unstable();
        groups.push(members);
    }
    let mut local = vec![usize::MAX; n];
    let boxes: Vec<[f64; 2]> = groups.iter().map(|g| layered(g, sizes, &succ, &mut local, gap, &mut out)).collect();
    // the groups in rows
    let area: f64 = boxes.iter().map(|b| (b[0] + gap[0]) * (b[1] + gap[1])).sum();
    let widest = boxes.iter().fold(0.0f64, |m, b| m.max(b[0]));
    let limit = widest.max(sqrt(area) * 1.5);
    let (mut x, mut y, mut row_h) = (0.0f64, 0.0f64, 0.0f64);
    for (g, b) in groups.iter().zip(&boxes) {
        if x > 0.0 && x + b[0] > limit + 1e-9 {
            x = 0.0;
            y += row_h + gap[1];
            row_h = 0.0;
        }
        for &v in g {
            out[v][0] += x;
            out[v][1] += y;
        }
        x += b[0] + gap[0];
        row_h = row_h.max(b[1]);
    }
    out
}

// one connected group in layers: the middles into `out` (from the group's top left corner) ->
// the group's width and height
fn layered(members: &[usize], sizes: &[[f64; 2]], succ: &[Vec<usize>], local: &mut [usize], gap: [f64; 2], out: &mut [[f64; 2]]) -> [f64; 2] {
    let m = members.len();
    for (k, &v) in members.iter().enumerate() {
        local[v] = k;
    }
    let size = |k: usize| sizes[members[k]];
    let next: Vec<Vec<usize>> = members.iter().map(|&v| succ[v].iter().map(|&w| local[w]).collect()).collect();
    // cycles broken: a depth-first walk drops the links back to a shape on its own path; the
    // walk starts at the shapes no link reaches, in file order
    let mut has_pred = vec![false; m];
    for list in &next {
        for &w in list {
            has_pred[w] = true;
        }
    }
    let starts: Vec<usize> = (0..m).filter(|&k| !has_pred[k]).chain((0..m).filter(|&k| has_pred[k])).collect();
    let mut state = vec![0u8; m];
    let mut post: Vec<usize> = Vec::with_capacity(m);
    let mut keep: Vec<Vec<usize>> = vec![Vec::new(); m];
    for s in starts {
        if state[s] != 0 {
            continue;
        }
        state[s] = 1;
        let mut stack: Vec<(usize, usize)> = vec![(s, 0)];
        while let Some(top) = stack.last_mut() {
            let (v, i) = *top;
            if i < next[v].len() {
                top.1 += 1;
                let w = next[v][i];
                match state[w] {
                    0 => {
                        keep[v].push(w);
                        state[w] = 1;
                        stack.push((w, 0));
                    }
                    // a shape on the path: the link closes a cycle and is left out
                    1 => {}
                    _ => keep[v].push(w),
                }
            } else {
                state[v] = 2;
                post.push(v);
                stack.pop();
            }
        }
    }
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); m];
    for (v, list) in keep.iter().enumerate() {
        for &w in list {
            preds[w].push(v);
        }
    }
    // layers: longest path from the top (reverse post order is a topological order)
    let mut layer = vec![0usize; m];
    for &v in post.iter().rev() {
        for &w in &keep[v] {
            layer[w] = layer[w].max(layer[v] + 1);
        }
    }
    // a shape no link reaches sits right above its highest successor
    for &v in &post {
        if preds[v].is_empty() && !keep[v].is_empty() {
            layer[v] = keep[v].iter().map(|&w| layer[w]).min().unwrap_or(1).saturating_sub(1);
        }
    }
    let depth = layer.iter().copied().max().unwrap_or(0) + 1;
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); depth];
    for v in 0..m {
        layers[layer[v]].push(v);
    }
    // the order within the layers: barycentre sweeps down and up
    let mut place = vec![0usize; m];
    let renumber = |layers: &[Vec<usize>], place: &mut [usize]| {
        for l in layers {
            for (i, &v) in l.iter().enumerate() {
                place[v] = i;
            }
        }
    };
    renumber(&layers, &mut place);
    let bary = |v: usize, nb: &[usize], place: &[usize]| -> f64 {
        if nb.is_empty() {
            place[v] as f64
        } else {
            nb.iter().map(|&u| place[u] as f64).sum::<f64>() / nb.len() as f64
        }
    };
    for _ in 0..4 {
        for k in 1..depth {
            let mut l = core::mem::take(&mut layers[k]);
            let keys: BTreeMap<usize, f64> = l.iter().map(|&v| (v, bary(v, &preds[v], &place))).collect();
            l.sort_by(|a, b| keys[a].partial_cmp(&keys[b]).unwrap_or(core::cmp::Ordering::Equal));
            layers[k] = l;
            renumber(&layers[k..k + 1], &mut place);
        }
        for k in (0..depth.saturating_sub(1)).rev() {
            let mut l = core::mem::take(&mut layers[k]);
            let keys: BTreeMap<usize, f64> = l.iter().map(|&v| (v, bary(v, &keep[v], &place))).collect();
            l.sort_by(|a, b| keys[a].partial_cmp(&keys[b]).unwrap_or(core::cmp::Ordering::Equal));
            layers[k] = l;
            renumber(&layers[k..k + 1], &mut place);
        }
    }
    // x: packed, then where the neighbours want each shape (passes up and down)
    let mut x = vec![0.0f64; m];
    for l in &layers {
        let mut at = 0.0;
        for &v in l {
            let w = size(v)[0];
            x[v] = at + w * 0.5;
            at += w + gap[0];
        }
    }
    for pass in 0..5 {
        let up = pass % 2 == 0;
        let order: Vec<usize> = if up { (0..depth).rev().collect() } else { (0..depth).collect() };
        for k in order {
            let l = &layers[k];
            let want: Vec<f64> = l
                .iter()
                .map(|&v| {
                    let nb = if up { &keep[v] } else { &preds[v] };
                    if nb.is_empty() {
                        x[v]
                    } else {
                        nb.iter().map(|&u| x[u]).sum::<f64>() / nb.len() as f64
                    }
                })
                .collect();
            let widths: Vec<f64> = l.iter().map(|&v| size(v)[0]).collect();
            let placed = spread(&want, &widths, gap[0]);
            for (i, &v) in l.iter().enumerate() {
                x[v] = placed[i];
            }
        }
    }
    let left = (0..m).map(|v| x[v] - size(v)[0] * 0.5).fold(f64::INFINITY, f64::min);
    let mut width = 0.0f64;
    for v in 0..m {
        x[v] -= left;
        width = width.max(x[v] + size(v)[0] * 0.5);
    }
    // y: the layers' bands, each as high as its highest shape
    let mut top = 0.0;
    let mut height = 0.0;
    for l in &layers {
        let h = l.iter().map(|&v| size(v)[1]).fold(0.0f64, f64::max);
        for &v in l {
            out[members[v]] = [x[v], top + h * 0.5];
        }
        height = top + h;
        top += h + gap[1];
    }
    [width, height]
}

// the places nearest to `want` (squared distance) that keep the order and leave `gap` between
// the boxes of `widths`: isotonic regression (pool adjacent violators) of want - offset
fn spread(want: &[f64], widths: &[f64], gap: f64) -> Vec<f64> {
    let n = want.len();
    let mut offset = vec![0.0f64; n];
    for i in 1..n {
        offset[i] = offset[i - 1] + (widths[i - 1] + widths[i]) * 0.5 + gap;
    }
    // blocks: (sum, count)
    let mut blocks: Vec<(f64, usize)> = Vec::with_capacity(n);
    for i in 0..n {
        blocks.push((want[i] - offset[i], 1));
        while blocks.len() >= 2 {
            let (s1, c1) = blocks[blocks.len() - 1];
            let (s0, c0) = blocks[blocks.len() - 2];
            if s0 / c0 as f64 <= s1 / c1 as f64 {
                break;
            }
            blocks.pop();
            let last = blocks.len() - 1;
            blocks[last] = (s0 + s1, c0 + c1);
        }
    }
    let mut out = Vec::with_capacity(n);
    for (s, c) in blocks {
        let mean = s / c as f64;
        for _ in 0..c {
            out.push(mean + offset[out.len()]);
        }
    }
    out
}

// ---------------------------------------------------------------- building

/// how apply builds (lengths in drawing inches)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Build {
    /// the language of new instances' names (instance::LANGS)
    pub lang: usize,
    /// a plain rectangle's size when the record gives none
    pub default_size: [f64; 2],
    /// the space between laid-out shapes: across, down
    pub gap: [f64; 2],
    /// the laid-out block's distance from the page's top left corner
    pub margin: f64,
}

impl Default for Build {
    fn default() -> Build {
        Build { lang: 0, default_size: [1.0, 0.5], gap: [0.4, 0.6], margin: 0.5 }
    }
}

/// what apply built
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Built {
    /// the shape of each Plan::shapes entry
    pub shapes: Vec<ShapeRef>,
    /// the connector of each Plan::links entry
    pub connectors: Vec<ShapeRef>,
    /// the Plan::masters entries whose master did not fit (a 1-D master for shapes, a 2-D one
    /// for links): drawn as the default instead
    pub unfit: Vec<usize>,
    /// the box of all of it on the page (x0, y0, x1, y1; None: nothing built)
    pub bounds: Option<[f64; 4]>,
}

// whether a master's shape is 1-D
fn master_one_d(doc: &diagram_core::Document, m: MasterRef) -> bool {
    doc.master(m).and_then(|x| x.master_shape()).is_some_and(|slot| glue::is_one_d(doc, ShapeRef { owner: Owner::Master(m), slot }))
}

// a shape and its subshapes as sheets
fn sheets_of(doc: &diagram_core::Document, s: ShapeRef) -> Vec<SheetRef> {
    let mut out = Vec::new();
    let mut stack = vec![s];
    while let Some(x) = stack.pop() {
        out.push(SheetRef::Shape(x));
        if let Some(sh) = doc.shape(x) {
            stack.extend(sh.children());
        }
    }
    out
}

// a text into a shape; a shape that refuses text (LockTextEdit) keeps its own
fn put_text(tx: &mut Transaction, eng: &mut Engine, s: ShapeRef, text: &str) -> Result<(), EditError> {
    if text.is_empty() {
        return Ok(());
    }
    match ops::set_text(tx, eng, s, Text::plain(text)) {
        Err(EditError::Invalid) => Ok(()),
        r => r,
    }
}

// the shape data of a record -> the Value cells written into `written`
fn put_props(tx: &mut Transaction, sheet: SheetRef, props: &[Prop], written: &mut Vec<(SheetRef, CellAddr)>) -> Result<(), EditError> {
    for p in props {
        let (value, unit) = match plain_number(&p.value) {
            Some(x) => (Value::Num(x), None),
            None => (Value::Text(tx.intern(&p.value)), Some(Unit::Str)),
        };
        // a field the shape has already, by name or by label
        let label = p.label.trim();
        let existing = tx.doc().shape_data(sheet).into_iter().find(|r| r.name().is_some_and(|n| n.eq_ignore_ascii_case(&p.name)) || r.text("Label").is_some_and(|l| l.trim().eq_ignore_ascii_case(label))).map(|r| (r.section, r.key()));
        let value_sym = tx.intern("Value");
        let r = match existing {
            Some((sk, key)) => {
                let addr = CellAddr::row(sk, key, value_sym);
                written.push((sheet, addr.clone()));
                tx.set_constant(sheet, &addr, value, unit)
            }
            None => {
                tx.set_shape_data(sheet, &p.name, &p.label, value).and_then(|()| {
                    let prop = tx.intern("Property");
                    let name = tx.intern(&p.name);
                    let (sk, key) = (SecKey::one(prop), RowKey::Named(name));
                    written.push((sheet, CellAddr::row(sk, key, value_sym)));
                    match value {
                        Value::Num(_) => {
                            let ty = tx.intern("Type");
                            tx.set_constant(sheet, &CellAddr::row(sk, key, ty), Value::Num(2.0), None)
                        }
                        _ => Ok(()),
                    }
                })
            }
        };
        match r {
            // a guarded value stays (Visio leaves a GUARDed cell alone too)
            Err(EditError::Guarded) => {}
            r => r?,
        }
    }
    Ok(())
}

// the union of two boxes
fn union(a: Option<[f64; 4]>, b: Option<[f64; 4]>) -> Option<[f64; 4]> {
    match (a, b) {
        (Some(a), Some(b)) => Some([a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]),
        (a, None) => a,
        (None, b) => b,
    }
}

// move a shape so that its middle is at `to`
fn centre_at(tx: &mut Transaction, eng: &mut Engine, s: ShapeRef, to: [f64; 2]) -> Result<(), EditError> {
    let c = ops::frame(tx.doc(), s).centre();
    let d = [to[0] - c[0], to[1] - c[1]];
    if d[0].abs() > 1e-9 || d[1].abs() > 1e-9 {
        ops::move_shapes(tx, eng, &[s], d)?;
    }
    Ok(())
}

/// build the plan on page `page` inside the caller's transaction (see the file header):
/// `masters[k]` is the master (of tx's document) for Plan::masters[k], None or missing: the
/// default (a plain rectangle, the dynamic connector) -> what was built
pub fn apply(tx: &mut Transaction, eng: &mut Engine, page: PageRef, plan: &Plan, masters: &[Option<MasterRef>], opts: &Build) -> Result<Built, EditError> {
    tx.doc().page(page).ok_or(EditError::NoPage)?;
    let owner = Owner::Page(page);
    let before = pagesetup::content(tx.doc(), page);
    let mut built = Built::default();
    // the chosen masters that fit their records
    let fits: Vec<Option<MasterRef>> = plan
        .masters
        .iter()
        .enumerate()
        .map(|(k, u)| {
            let m = masters.get(k).copied().flatten()?;
            if master_one_d(tx.doc(), m) == u.link {
                Some(m)
            } else {
                built.unfit.push(k);
                None
            }
        })
        .collect();
    let chosen = |k: usize| fits.get(k).copied().flatten();
    let mut written: Vec<(SheetRef, CellAddr)> = Vec::new();
    // the shapes: where the record puts them (else at the origin until the layout)
    for rec in &plan.shapes {
        let at = rec.pos.unwrap_or([0.0, 0.0]);
        let s = match chosen(rec.master) {
            Some(m) => {
                let s = instance::place(tx, page, m, at[0], at[1], opts.lang)?;
                let sheets = sheets_of(tx.doc(), s);
                eng.recalc(tx, &sheets)?;
                s
            }
            None => {
                let w = rec.size[0].unwrap_or(opts.default_size[0]);
                let h = rec.size[1].unwrap_or(opts.default_size[1]);
                ops::new_shape(tx, eng, owner, ops::Basic::Rectangle, [at[0] - w * 0.5, at[1] - h * 0.5], [at[0] + w * 0.5, at[1] + h * 0.5])?
            }
        };
        // the record's size around the same middle
        if rec.size.iter().any(Option::is_some) {
            let f = ops::frame(tx.doc(), s);
            let (w, h) = (rec.size[0].unwrap_or(f.w), rec.size[1].unwrap_or(f.h));
            if (w - f.w).abs() > 1e-9 || (h - f.h).abs() > 1e-9 {
                let to = ops::Frame { xf: Xform::translate((f.w - w) * 0.5, (f.h - h) * 0.5).then(&f.xf), w, h };
                ops::set_frame(tx, eng, s, &to)?;
            }
        }
        put_text(tx, eng, s, &rec.text)?;
        if let Some(p) = rec.pos {
            centre_at(tx, eng, s, p)?;
        }
        put_props(tx, SheetRef::Shape(s), &rec.props, &mut written)?;
        built.shapes.push(s);
    }
    if !written.is_empty() {
        eng.recalc_cells(tx, &written)?;
    }
    // the shapes without a position, laid out
    let auto: Vec<usize> = (0..plan.shapes.len()).filter(|&i| plan.shapes[i].pos.is_none()).collect();
    if !auto.is_empty() {
        let mut ix = vec![usize::MAX; plan.shapes.len()];
        for (k, &i) in auto.iter().enumerate() {
            ix[i] = k;
        }
        let sizes: Vec<[f64; 2]> = auto
            .iter()
            .map(|&i| {
                let b = ops::frame(tx.doc(), built.shapes[i]).bounds();
                [b[2] - b[0], b[3] - b[1]]
            })
            .collect();
        let edges: Vec<(usize, usize)> = plan.links.iter().filter_map(|l| Some((*ix.get(l.from)?, *ix.get(l.to)?))).filter(|&(a, b)| a != usize::MAX && b != usize::MAX).collect();
        let centres = layout(&sizes, &edges, opts.gap);
        // the block at the page's top left corner, right of what is there already
        let placed: Vec<ShapeRef> = (0..plan.shapes.len()).filter(|&i| plan.shapes[i].pos.is_some()).map(|i| built.shapes[i]).collect();
        let avoid = union(before, ops::bounds(tx.doc(), &placed));
        let [_, ph] = pagesetup::size(tx.doc(), page);
        let left = avoid.map_or(opts.margin, |b| opts.margin.max(b[2] + opts.gap[0]));
        let top = ph - opts.margin;
        for (k, &i) in auto.iter().enumerate() {
            centre_at(tx, eng, built.shapes[i], [left + centres[k][0], top - centres[k][1]])?;
        }
    }
    // the links
    for l in &plan.links {
        let (a, b) = (built.shapes[l.from], built.shapes[l.to]);
        let (pa, pb) = (ops::frame(tx.doc(), a).centre(), ops::frame(tx.doc(), b).centre());
        let c = match chosen(l.master) {
            Some(m) => {
                let c = instance::place(tx, page, m, (pa[0] + pb[0]) * 0.5, (pa[1] + pb[1]) * 0.5, opts.lang)?;
                let sheets = sheets_of(tx.doc(), c);
                eng.recalc(tx, &sheets)?;
                glue::glue(tx, eng, c, glue::End::Begin, pa, glue::Target::Shape(a))?;
                glue::glue(tx, eng, c, glue::End::End, pb, glue::Target::Shape(b))?;
                c
            }
            None => glue::new_connector(tx, eng, owner, (pa, glue::Target::Shape(a)), (pb, glue::Target::Shape(b)))?,
        };
        put_text(tx, eng, c, &l.text)?;
        built.connectors.push(c);
    }
    if !built.connectors.is_empty() {
        glue::update(tx, eng, &built.connectors)?;
    }
    let all: Vec<ShapeRef> = built.shapes.iter().chain(built.connectors.iter()).copied().collect();
    built.bounds = ops::bounds(tx.doc(), &all);
    Ok(built)
}
