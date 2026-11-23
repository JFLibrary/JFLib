// stencil_src -- the text sources of a stencil (stencil.txt, *.master) as entries (JD9)
//
//   parse(file, text, sections) -> Source { header, sections, parts }   lines, header lines,
//                                       sections, rows with continuation lines, ';'-separated
//                                       cells; (JD26) `[Part NAME]` starts a subshape with its own
//                                       header lines and sections
//   resolve(&entries, measure)       the entries of one variant: a ".metric" / ".us" entry
//                                       replaces the one without suffix (duplicates refused)
//   classify(text) -> Value0         a constant (number with unit, TRUE/FALSE, "string",
//                                       #RRGGBB) or a formula
//   unquote(text)                    the text of a language-suffixed cell
//
// The format is apps/JFDiagrammer/stencils/README.md.  Every error names the file and the line
// (SourceError); nothing here panics, nothing recurses.  The parser keeps the names as written;
// stencil.rs checks them against what the sections may hold.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use diagram_core::instance::lang_index;
use diagram_core::Unit;

use crate::stencil::SourceError;
use crate::vsdx::Measure;

/// the most lines of one source file, cells of one row, rows of one section, sections of one
/// master (trusted repo files, but bounded all the same)
const MAX_LINES: usize = 200_000;
const MAX_ITEMS: usize = 100_000;
const MAX_SECTIONS: usize = 10_000;

/// one `name[.suffixes] = value` or `key[.suffixes]: value`
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    /// the name without suffixes ("Tip", "name", "X")
    pub name: String,
    /// the language suffix (index into instance::LANGS)
    pub lang: Option<usize>,
    /// the variant suffix
    pub variant: Option<Measure>,
    /// the value as written, trimmed at both ends
    pub value: String,
    /// the line (from 1)
    pub line: u32,
    /// the file it comes from when that is not the master's own (`include:`)
    pub file: Option<String>,
}

/// how a section's rows are headed
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) enum SecKind {
    /// [Shape]: single cells only
    Shape,
    /// [Geometry]: rows headed by their type (MoveTo ...), one section per [Geometry]
    Geometry,
    /// rows headed by the word Row (IX 0, 1, ...)
    Indexed,
    /// rows headed by their name and a colon
    Named,
}

/// the sections a master may have: as written, the VSDX name (N), the kind
pub(crate) const SECTIONS: &[(&str, &str, SecKind)] = &[
    ("Shape", "", SecKind::Shape),
    ("Geometry", "Geometry", SecKind::Geometry),
    ("Connection", "Connection", SecKind::Indexed),
    ("Controls", "Control", SecKind::Named),
    ("Control", "Control", SecKind::Named),
    ("User", "User", SecKind::Named),
    ("Property", "Property", SecKind::Named),
    ("Character", "Character", SecKind::Indexed),
    ("Paragraph", "Paragraph", SecKind::Indexed),
    ("Actions", "Actions", SecKind::Named),
    ("Hyperlink", "Hyperlink", SecKind::Named),
    ("Scratch", "Scratch", SecKind::Indexed),
    // (JD15) text fields: the Value formula a "{fld N}" mark in the text shows
    ("Field", "Field", SecKind::Indexed),
];

/// the row types of a Geometry section
pub(crate) const ROW_TYPES: &[&str] = &[
    "MoveTo", "LineTo", "ArcTo", "EllipticalArcTo", "Ellipse", "NURBSTo", "PolylineTo", "RelMoveTo", "RelLineTo", "RelCubBezTo",
    "RelQuadBezTo", "RelEllipticalArcTo", "InfiniteLine", "SplineStart", "SplineKnot",
];

/// a row: its head and its cells
#[derive(Clone, Debug)]
pub(crate) struct SrcRow {
    /// the row type (Geometry), "Row" (indexed) or the row's name (named)
    pub head: String,
    pub line: u32,
    pub cells: Vec<Entry>,
    /// the file it comes from when that is not the master's own (`include:`)
    pub file: Option<String>,
}

/// a section as written
#[derive(Clone, Debug)]
pub(crate) struct SrcSection {
    /// the VSDX name ("" for [Shape])
    pub model: &'static str,
    /// the name as written in the brackets
    pub written: String,
    pub kind: SecKind,
    pub line: u32,
    /// single cells ([Shape]) or the section's own cells (Geometry's NoFill ...)
    pub cells: Vec<Entry>,
    pub rows: Vec<SrcRow>,
    /// (JD26) whose section it is: 0 the master shape, k the k-th `[Part]` (parts[k-1])
    pub part: usize,
}

/// (JD26) a subshape of a group master: `[Part NAME]`, then its own header lines (`text.de:` ...),
/// then its sections up to the next `[Part]`
#[derive(Clone, Debug)]
pub(crate) struct SrcPart {
    /// the name in the brackets
    pub name: String,
    pub line: u32,
    pub header: Vec<Entry>,
}

/// a source file: the header lines, the sections and (JD26) the parts
#[derive(Clone, Debug, Default)]
pub(crate) struct Source {
    pub header: Vec<Entry>,
    pub sections: Vec<SrcSection>,
    pub parts: Vec<SrcPart>,
}

pub(crate) fn err(file: &str, line: u32, message: impl Into<String>) -> SourceError {
    SourceError { file: String::from(file), line, message: message.into() }
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.'
}

// the identifier at the start of `s` (letters, digits, '_', '.') and the rest
fn split_ident(s: &str) -> (&str, &str) {
    let end = s.char_indices().find(|&(_, c)| !is_ident_char(c)).map_or(s.len(), |(i, _)| i);
    s.split_at(end)
}

/// a name with its suffixes -> (name, language, variant)
pub(crate) fn suffixes(file: &str, line: u32, full: &str) -> Result<(String, Option<usize>, Option<Measure>), SourceError> {
    let mut parts = full.split('.');
    let base = parts.next().unwrap_or("");
    if base.is_empty() || !base.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') {
        return Err(err(file, line, format!("ungültiger Name „{full}“")));
    }
    let (mut lang, mut variant) = (None, None);
    for s in parts {
        if let Some(l) = lang_index(s) {
            if lang.replace(l).is_some() {
                return Err(err(file, line, format!("{full}: zwei Sprachendungen")));
            }
        } else if s.eq_ignore_ascii_case("metric") || s.eq_ignore_ascii_case("us") {
            let v = if s.eq_ignore_ascii_case("us") { Measure::Us } else { Measure::Metric };
            if variant.replace(v).is_some() {
                return Err(err(file, line, format!("{full}: zwei Fassungsendungen")));
            }
        } else {
            return Err(err(file, line, format!("{full}: unbekannte Endung .{s} (Sprachen .de .en .es .it .fr, Fassungen .metric .us)")));
        }
    }
    Ok((String::from(base), lang, variant))
}

// split at ';' outside double quotes (a doubled "" stays inside the string)
pub(crate) fn split_cells(s: &str) -> Option<Vec<&str>> {
    let mut out = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if quoted {
        return None;
    }
    out.push(&s[start..]);
    Some(out)
}

/// `a = 1; b.us = 2 in` -> entries
fn parse_cells(file: &str, line: u32, s: &str, out: &mut Vec<Entry>) -> Result<(), SourceError> {
    let parts = split_cells(s).ok_or_else(|| err(file, line, "Anführungszeichen nicht geschlossen"))?;
    for p in parts {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        let Some((name, value)) = p.split_once('=') else {
            return Err(err(file, line, format!("„{p}“: erwartet Zelle = Formel")));
        };
        let name = name.trim();
        let value = value.trim();
        if name.is_empty() || !name.chars().all(is_ident_char) {
            return Err(err(file, line, format!("„{name}“ ist kein Zellname")));
        }
        if value.is_empty() {
            return Err(err(file, line, format!("{name}: Formel fehlt")));
        }
        let (n, lang, variant) = suffixes(file, line, name)?;
        if out.len() >= MAX_ITEMS {
            return Err(err(file, line, "zu viele Zellen"));
        }
        out.push(Entry { name: n, lang, variant, value: String::from(value), line, file: None });
    }
    Ok(())
}

/// parse a source file; `sections`: whether it may have sections (a master) or only header
/// lines (stencil.txt)
pub(crate) fn parse(file: &str, text: &str, sections: bool) -> Result<Source, SourceError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut src = Source::default();
    // the row names of the current section
    let mut names: BTreeSet<String> = BTreeSet::new();
    // (JD26) between `[Part NAME]` and the part's first section: its header lines
    let mut part_head = false;
    for (i, raw) in text.split('\n').enumerate() {
        if i >= MAX_LINES {
            return Err(err(file, i as u32 + 1, "zu viele Zeilen"));
        }
        let line_no = i as u32 + 1;
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let indented = line.starts_with([' ', '\t']);
        if t.starts_with('[') {
            if !sections {
                return Err(err(file, line_no, format!("{file} hat keine Abschnitte ({t})")));
            }
            let Some(name) = t.strip_prefix('[').and_then(|r| r.strip_suffix(']')) else {
                return Err(err(file, line_no, format!("Abschnittskopf {t}: erwartet [Name]")));
            };
            let name = name.trim();
            // (JD26) [Part NAME]: a subshape of the master; what follows is its own
            if let Some(pn) = name.strip_prefix("Part").filter(|r| r.starts_with(char::is_whitespace)).map(str::trim) {
                if pn.is_empty() || !pn.chars().all(is_ident_char) || pn.contains('.') {
                    return Err(err(file, line_no, format!("[Part {pn}]: erwartet [Part Name] mit einem Namen aus Buchstaben, Ziffern und _")));
                }
                if src.parts.iter().any(|p| p.name == pn) {
                    return Err(err(file, line_no, format!("[Part {pn}] doppelt")));
                }
                if src.parts.len() >= MAX_SECTIONS {
                    return Err(err(file, line_no, "zu viele Teile"));
                }
                src.parts.push(SrcPart { name: String::from(pn), line: line_no, header: Vec::new() });
                part_head = true;
                names.clear();
                continue;
            }
            let Some(&(_, model, kind)) = SECTIONS.iter().find(|(w, _, _)| *w == name) else {
                return Err(err(file, line_no, format!("unbekannter Abschnitt [{name}]")));
            };
            let part = src.parts.len();
            if kind != SecKind::Geometry && src.sections.iter().any(|s| s.kind == kind && s.model == model && s.part == part) {
                return Err(err(file, line_no, format!("Abschnitt [{name}] doppelt")));
            }
            if src.sections.len() >= MAX_SECTIONS {
                return Err(err(file, line_no, "zu viele Abschnitte"));
            }
            names.clear();
            part_head = false;
            src.sections.push(SrcSection { model, written: String::from(name), kind, line: line_no, cells: Vec::new(), rows: Vec::new(), part });
            continue;
        }
        let sec = if part_head { None } else { src.sections.last_mut() };
        let Some(sec) = sec else {
            // a header line: key: value
            let Some((key, value)) = t.split_once(':') else {
                return Err(err(file, line_no, format!("Kopfzeile ohne „:“ ({t})")));
            };
            let key = key.trim();
            if key.is_empty() || !key.chars().all(is_ident_char) {
                return Err(err(file, line_no, format!("„{key}“ ist kein Schlüssel")));
            }
            let (name, lang, variant) = suffixes(file, line_no, key)?;
            // (JD26) the header lines of a part
            let header = match src.parts.last_mut() {
                Some(p) if part_head => &mut p.header,
                _ => &mut src.header,
            };
            if header.len() >= MAX_ITEMS {
                return Err(err(file, line_no, "zu viele Kopfzeilen"));
            }
            header.push(Entry { name, lang, variant, value: String::from(value.trim()), line: line_no, file: None });
            continue;
        };
        if sec.kind == SecKind::Shape {
            parse_cells(file, line_no, t, &mut sec.cells)?;
            continue;
        }
        if indented {
            let Some(row) = sec.rows.last_mut() else {
                return Err(err(file, line_no, "eingerückte Folgezeile ohne Zeile davor"));
            };
            parse_cells(file, line_no, t, &mut row.cells)?;
            continue;
        }
        let (ident, rest) = split_ident(t);
        let after = rest.trim_start();
        if after.starts_with('=') {
            // a cell of the section itself (Geometry's NoFill ...)
            parse_cells(file, line_no, t, &mut sec.cells)?;
            continue;
        }
        let (head, cells) = match sec.kind {
            SecKind::Named => {
                let Some(cells) = after.strip_prefix(':') else {
                    return Err(err(file, line_no, format!("Zeile „{ident}“: erwartet Name: Zellen")));
                };
                if ident.is_empty() || ident.contains('.') {
                    return Err(err(file, line_no, format!("„{ident}“ ist kein Zeilenname")));
                }
                if !names.insert(String::from(ident)) {
                    return Err(err(file, line_no, format!("Zeile {ident} doppelt in [{}]", sec.written)));
                }
                (ident, cells)
            }
            SecKind::Geometry => {
                if !ROW_TYPES.contains(&ident) {
                    return Err(err(file, line_no, format!("unbekannte Zeilenart „{ident}“ in [Geometry]")));
                }
                (ident, rest)
            }
            _ => {
                if ident != "Row" {
                    return Err(err(file, line_no, format!("„{ident}“: Zeilen in [{}] beginnen mit Row", sec.written)));
                }
                (ident, rest)
            }
        };
        if sec.rows.len() >= MAX_ITEMS {
            return Err(err(file, line_no, "zu viele Zeilen im Abschnitt"));
        }
        let mut row = SrcRow { head: String::from(head), line: line_no, cells: Vec::new(), file: None };
        parse_cells(file, line_no, cells, &mut row.cells)?;
        sec.rows.push(row);
    }
    Ok(src)
}

/// (JD13) merge the sections of a file a master names with `include:` into the master's: a
/// section the master lacks is added (Geometry sections always, after the master's own); in a
/// section the master has, the included cells and rows come first and the master's own win (a
/// cell of the same name, language and variant is left out; a named row the master has too
/// gets the included cells it does not set itself).  Every included entry and row remembers its
/// file for the error messages.
pub(crate) fn merge_include(src: &mut Source, name: &str, inc: Source) -> Result<(), SourceError> {
    if let Some(e) = inc.header.first() {
        return Err(err(name, e.line, format!("eine eingebundene Datei hat nur Abschnitte, keine Kopfzeilen ({}: …)", e.name)));
    }
    if let Some(p) = inc.parts.first() {
        return Err(err(name, p.line, format!("eine eingebundene Datei hat keine Teile ([Part {}])", p.name)));
    }
    let mark = |es: &mut Vec<Entry>| es.iter_mut().for_each(|e| e.file = Some(String::from(name)));
    for mut s in inc.sections {
        mark(&mut s.cells);
        for r in &mut s.rows {
            r.file = Some(String::from(name));
            mark(&mut r.cells);
        }
        let kind = s.kind;
        let own = if kind == SecKind::Geometry { None } else { src.sections.iter_mut().find(|o| o.kind == kind && o.model == s.model && o.part == 0) };
        let Some(o) = own else {
            if src.sections.len() >= MAX_SECTIONS {
                return Err(err(name, s.line, "zu viele Abschnitte"));
            }
            src.sections.push(s);
            continue;
        };
        let same = |a: &Entry, b: &Entry| a.name == b.name && a.lang == b.lang && a.variant == b.variant;
        let mut cells: Vec<Entry> = s.cells.into_iter().filter(|c| !o.cells.iter().any(|m| same(m, c))).collect();
        cells.append(&mut o.cells);
        o.cells = cells;
        // a named row the master has too: its included cells join the master's row (the master's
        // own cells win)
        let mut rows: Vec<SrcRow> = Vec::new();
        for r in s.rows {
            match o.rows.iter_mut().find(|m| kind == SecKind::Named && m.head == r.head) {
                Some(m) => {
                    let mut cells: Vec<Entry> = r.cells.into_iter().filter(|c| !m.cells.iter().any(|x| same(x, c))).collect();
                    cells.append(&mut m.cells);
                    m.cells = cells;
                }
                None => rows.push(r),
            }
        }
        rows.append(&mut o.rows);
        if rows.len() > MAX_ITEMS {
            return Err(err(name, s.line, "zu viele Zeilen im Abschnitt"));
        }
        o.rows = rows;
    }
    Ok(())
}

/// the entries that hold for variant `m`: per name and language the entry with m's suffix,
/// else the one without; in the order the names first appear.  The same name, language and
/// variant twice is an error.
pub(crate) fn resolve<'e>(file: &str, entries: &'e [Entry], m: Measure) -> Result<Vec<&'e Entry>, SourceError> {
    // per name and language: the position of its group; per group the entry without suffix,
    // the one of m, and whether the other variant had one
    let mut index: BTreeMap<(&str, Option<usize>), usize> = BTreeMap::new();
    let mut groups: Vec<(Option<&'e Entry>, Option<&'e Entry>, bool)> = Vec::new();
    for e in entries {
        let g = *index.entry((e.name.as_str(), e.lang)).or_insert_with(|| {
            groups.push((None, None, false));
            groups.len() - 1
        });
        let Some(group) = groups.get_mut(g) else { continue };
        let taken = match e.variant {
            None => group.0.replace(e).is_some(),
            Some(v) if v == m => group.1.replace(e).is_some(),
            // another variant's entry: only checked for duplicates
            Some(_) => core::mem::replace(&mut group.2, true),
        };
        if taken {
            return Err(err(file, e.line, format!("{} doppelt", show(e))));
        }
    }
    Ok(groups.into_iter().filter_map(|g| g.1.or(g.0)).collect())
}

/// a name with its suffixes as written ("Tip.de.us")
pub(crate) fn show(e: &Entry) -> String {
    let mut s = e.name.clone();
    if let Some(l) = e.lang {
        s.push('.');
        s.push_str(diagram_core::instance::LANGS[l]);
    }
    match e.variant {
        Some(Measure::Metric) => s.push_str(".metric"),
        Some(Measure::Us) => s.push_str(".us"),
        None => {}
    }
    s
}

/// a cell's value as the builder writes it
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value0 {
    /// a number in internal units and the unit it was written in (None: a plain number)
    Num(f64, Option<Unit>),
    /// TRUE / FALSE
    Bool(bool),
    /// a string constant
    Str(String),
    /// a colour #RRGGBB (upper case)
    Color(String),
    /// everything else: a formula in Visio's universal syntax
    Formula(String),
}

/// the text of a string literal "..." ("" inside is one quote); None when `s` is not exactly
/// one string literal
pub(crate) fn string_literal(s: &str) -> Option<String> {
    let rest = s.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            if chars.peek() == Some(&'"') {
                chars.next();
                out.push('"');
            } else {
                // the end: nothing may follow
                return chars.all(char::is_whitespace).then_some(out);
            }
        } else {
            out.push(c);
        }
    }
    None
}

/// the text of a language-suffixed cell: a string literal's content, else the text as written
pub(crate) fn unquote(s: &str) -> String {
    string_literal(s).unwrap_or_else(|| s.to_string())
}

// a number at the start of s (sign, digits, point, exponent) -> (value, rest)
fn number_prefix(s: &str) -> Option<(f64, &str)> {
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
        i += 1;
    }
    let digits_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    let mantissa = &s[digits_start..i];
    if mantissa.is_empty() || mantissa == "." {
        return None;
    }
    // an exponent only when digits follow (else "1 e..." is no number)
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'-' || b[j] == b'+') {
            j += 1;
        }
        let k = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > k {
            i = j;
        }
    }
    let x: f64 = s[..i].parse().ok()?;
    x.is_finite().then_some((x, &s[i..]))
}

/// a constant or a formula (README: number with or without unit, TRUE/FALSE, "string",
/// #RRGGBB are constants)
pub(crate) fn classify(text: &str) -> Value0 {
    let t = text.trim();
    if let Some(s) = string_literal(t) {
        return Value0::Str(s);
    }
    if t.eq_ignore_ascii_case("TRUE") {
        return Value0::Bool(true);
    }
    if t.eq_ignore_ascii_case("FALSE") {
        return Value0::Bool(false);
    }
    if let Some(hex) = t.strip_prefix('#') {
        if hex.len() == 6 && hex.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Value0::Color(format!("#{}", hex.to_ascii_uppercase()));
        }
    }
    if let Some((x, rest)) = number_prefix(t) {
        let rest = rest.trim();
        if rest.is_empty() {
            return Value0::Num(x, None);
        }
        if rest == "%" {
            return Value0::Num(x / 100.0, Some(Unit::Percent));
        }
        if rest.chars().all(|c| c.is_ascii_alphabetic() || c == '_') {
            if let Some(u) = diagram_core::formula::unit_code(rest) {
                let v = u.to_internal(x);
                if v.is_finite() {
                    return Value0::Num(v, Some(u));
                }
            }
        }
    }
    Value0::Formula(String::from(t))
}
