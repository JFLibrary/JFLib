// field -- text fields (TX-06): a <fld IX='n'/> mark in a shape's text shows Field row n of the
// shape, the value of its Value cell in the format picture of its Format cell
//
//   Env                        what fields show that the file does not hold (Document::fields,
//                              set by the app): the decimal sign and the digit groups, month and
//                              day names, the date picture of the general format, the clock
//                              (NOW), when the drawing was made, saved and edited (when the
//                              document's properties do not say), the file's path (DIRECTORY,
//                              FILENAME), the zone's offset (the properties' times are UTC)
//   Shown / format(&v, picture, &env) -> String   a value in a picture of Visio's FORMAT
//   shown(doc, sheet, ix) -> String           the text Field row ix of a sheet shows
//   cached(xml) -> Option<String>             the text a field kept as XML (Visio's own
//                                              <fld IX='n'>text</fld>) has cached
//   Seg / segments(doc, shape)                the shape's text as runs of characters and fields,
//                                              each field with the text it shows (editing in
//                                              place, Find and Replace)
//   plain(doc, shape) -> String                the text as it shows, fields included
//   marked("Länge: {fld 0}") -> Text / marks(&text)   (JD15) a text written with field marks as
//                                              characters and fields (the stencil build tool's
//                                              master texts, the instance of such a master) / the
//                                              Field rows a text names
//   Kind / Spec / insert(tx, eng, shape, at, &spec) -> ix   a new field: its Field row (Value
//                                              formula, Format, Type) and its mark in the text,
//                                              computed at once
//   Props / props(doc)                         the document's properties (docProps/core.xml and
//                                              app.xml): title, creator, keywords, dates ...
//   ole_days(y, m, d, h, min, s) / civil(days) dates as Visio keeps them: days since 30 December
//                                              1899, the time of day as the fraction
//
// The Field section (Visio's Text Fields section, [MS-VSDX] Field_Type): Value holds the
// formula (Width, PAGENAME(), NOW(), Prop.Cost, a formula of the user's) and its result in the
// unit the field shows (U="MM" for a length in millimetres, U="DATE" for a date, U="STR" for a
// text); Format holds the format picture as a string.  Type: 0 text, 2 number, 5 date or time,
// 6 duration, 7 currency (Annahme: the numbering of the shape data's Type cell, which the Text
// Fields section shares as far as known here).  The cells UICat, UICod and UIFmt, which only
// Visio's own dialog reads, are not written; Visio's dialog then opens a field of ours as a
// custom formula (Annahme).  A field the file wrote with its text inside
// (<fld IX='0'>19.11.2026</fld>, Visio's way) stays as the reader kept it (TextItem::Other) and
// shows that text (JD4); a field without text inside (ours, TextItem::Field) shows its value,
// which the formula engine computes when what it reads changes.
//
// Format pictures (Visio's FORMAT; Annahmen as far as Visio's own documentation was not at hand):
//   numbers   0 a digit or 0, # a digit or nothing, . the decimal sign, , groups of three in the
//             integer part, % times 100, u / U the unit's abbreviation (uu / UU the same), "..."
//             and \c literal text, anything else literal; "a;b;c": a for positive, b for
//             negative (without its sign), c for zero; a length, angle or duration shows in the
//             unit of its cell (25.4 mm for an inch in a cell with U="MM")
//   dates     {{ ... }} around date and time parts: d dd day, ddd dddd its name, M MM month,
//             MMM MMMM its name, yy yyyy year, H HH hours (24), h hh hours (12), m mm minutes,
//             s ss seconds, t tt AM / PM; '...' literal inside, text outside the braces literal
//   texts     @ the text, @+ in capitals, @- in small letters
//   empty     the general format: a number with up to four decimals (a length with its unit),
//             a date in Env::date, a text as it is

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::cell::Value;
use crate::document::{Document, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::{math, Engine};
use crate::names;
use crate::shape::{Text, TextItem};
use crate::sheet::{CellAddr, RowKey, SecKey};
use crate::units::{Dimension, Unit};

/// what fields show that the file does not hold (see the file header)
#[derive(Clone, Debug)]
pub struct Env {
    /// the decimal sign ('.' in English, ',' in German)
    pub decimal: char,
    /// the sign between groups of three digits
    pub group: char,
    /// January .. December
    pub months: [&'static str; 12],
    /// Sunday .. Saturday
    pub days: [&'static str; 7],
    /// the marks of the morning and the afternoon (t, tt)
    pub am_pm: [&'static str; 2],
    /// the date picture of the general format (without the braces): "M/d/yyyy", "dd.MM.yyyy"
    pub date: &'static str,
    /// now in local time (days since 30 Dec 1899; NaN: unknown, NOW() keeps its stored value)
    pub now: f64,
    /// when the drawing was made (local time; NaN: unknown) when its properties do not say
    pub created: f64,
    /// when it was saved last (the same)
    pub saved: f64,
    /// when it was edited last (NaN: when it was saved)
    pub edited: f64,
    /// the file's path ("": not saved yet)
    pub path: String,
    /// local time minus UTC, in days (the properties' times are UTC)
    pub utc_offset: f64,
}

/// English month names
pub const MONTHS_EN: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
/// English day names, Sunday first
pub const DAYS_EN: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

impl Default for Env {
    fn default() -> Env {
        Env {
            decimal: '.',
            group: ',',
            months: MONTHS_EN,
            days: DAYS_EN,
            am_pm: ["AM", "PM"],
            date: "M/d/yyyy",
            now: f64::NAN,
            created: f64::NAN,
            saved: f64::NAN,
            edited: f64::NAN,
            path: String::new(),
            utc_offset: 0.0,
        }
    }
}

/// a value to show
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shown<'a> {
    /// a number in internal units and the unit of its cell
    Num(f64, Option<Unit>),
    /// a text
    Text(&'a str),
    /// nothing (no value)
    Nothing,
}

/// a value in a format picture (see the file header)
pub fn format(v: &Shown, picture: &str, env: &Env) -> String {
    // (JD19) esc(n) is Visio's own code for a general picture (FIELDPICTURE(0) gives "esc(0)",
    // the Format of Visio's callout fields)
    let picture = if is_esc(picture) { "" } else { picture };
    match *v {
        Shown::Nothing => String::new(),
        Shown::Text(s) => text_picture(s, picture),
        Shown::Num(x, unit) => {
            if !x.is_finite() {
                return String::new();
            }
            let p = picture.trim();
            if p.contains("{{") {
                return date_picture(x, picture, env);
            }
            if p.is_empty() || p.starts_with('@') {
                return general(x, unit, env);
            }
            number_picture(x, unit, picture, env)
        }
    }
}

// "esc(0)", "esc(12)" ...: a picture by Visio's number
fn is_esc(p: &str) -> bool {
    let p = p.trim();
    p.len() > 5 && p[..4].eq_ignore_ascii_case("esc(") && p.ends_with(')') && p[4..p.len() - 1].chars().all(|c| c.is_ascii_digit())
}

// ---- texts

fn text_picture(s: &str, picture: &str) -> String {
    if !picture.contains('@') {
        return String::from(s);
    }
    let mut out = String::new();
    let mut it = picture.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '@' => match it.peek() {
                Some('+') => {
                    it.next();
                    out.push_str(&s.to_uppercase());
                }
                Some('-') => {
                    it.next();
                    out.push_str(&s.to_lowercase());
                }
                _ => out.push_str(s),
            },
            '"' => {
                for d in it.by_ref() {
                    if d == '"' {
                        break;
                    }
                    out.push(d);
                }
            }
            '\\' => {
                if let Some(d) = it.next() {
                    out.push(d);
                }
            }
            c => out.push(c),
        }
    }
    out
}

// literal text outside a date's braces: quotes and backslashes removed
fn literal(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        match c {
            '"' => {}
            '\\' => {
                if let Some(d) = it.next() {
                    out.push(d);
                }
            }
            c => out.push(c),
        }
    }
    out
}

// ---- numbers

/// the abbreviation a picture's u shows for a unit
pub fn unit_abbr(u: Unit) -> &'static str {
    match u {
        Unit::Inches | Unit::InchesFrac | Unit::DrawingLength | Unit::PageLength => "in",
        Unit::Feet | Unit::FeetInches => "ft",
        Unit::Yards => "yd",
        Unit::Miles | Unit::MilesFrac => "mi",
        Unit::NauticalMiles => "nmi",
        Unit::Millimeters => "mm",
        Unit::Centimeters => "cm",
        Unit::Meters => "m",
        Unit::Kilometers => "km",
        Unit::Points | Unit::TextLength => "pt",
        Unit::Picas | Unit::PicasPoints => "p",
        Unit::Didots => "dd",
        Unit::Ciceros | Unit::CicerosDidots => "c",
        Unit::Degrees | Unit::DefaultAngle => "°",
        Unit::Radians => "rad",
        Unit::Days | Unit::DefaultDuration => "d",
        Unit::Hours => "h",
        Unit::Minutes => "min",
        Unit::Seconds => "s",
        Unit::Weeks => "w",
        Unit::Percent => "%",
        _ => "",
    }
}

// the unit a value shows in when it is a length, an angle, a duration or an area
fn converts(u: Option<Unit>) -> Option<Unit> {
    let u = u?;
    matches!(u.dimension(), Dimension::Length | Dimension::Angle | Dimension::Duration | Dimension::Area).then_some(u)
}

// the general format of a number
fn general(x: f64, unit: Option<Unit>, env: &Env) -> String {
    match unit {
        Some(Unit::Date) => return date_parts(x, env.date, env),
        Some(Unit::Bool) => return String::from(if x != 0.0 { "TRUE" } else { "FALSE" }),
        Some(Unit::Percent) => {
            let mut s = digits(x * 100.0, 0, 2, false, env);
            s.push('%');
            return s;
        }
        Some(Unit::Currency) => return digits(x, 2, 2, true, env),
        _ => {}
    }
    match converts(unit) {
        Some(u) => {
            let mut s = digits(u.from_internal(x), 0, 4, false, env);
            let a = unit_abbr(u);
            if !a.is_empty() {
                if a != "°" {
                    s.push(' ');
                }
                s.push_str(a);
            }
            s
        }
        None => digits(x, 0, 4, false, env),
    }
}

// a number with at least `min` and at most `max` decimals, optionally grouped
fn digits(x: f64, min: usize, max: usize, group: bool, env: &Env) -> String {
    let (int, mut frac) = round_split(x.abs(), max);
    while frac.len() > min && frac.ends_with('0') {
        frac.pop();
    }
    let mut out = String::new();
    if x < 0.0 && (int.bytes().any(|b| b != b'0') || frac.bytes().any(|b| b != b'0')) {
        out.push('-');
    }
    out.push_str(&if group { grouped(&int, env.group) } else { int });
    if !frac.is_empty() {
        out.push(env.decimal);
        out.push_str(&frac);
    }
    out
}

// |x| rounded to n decimals: the integer digits and n decimal digits
fn round_split(x: f64, n: usize) -> (String, String) {
    let n = n.min(15);
    let s = alloc::format!("{:.*}", n, x);
    match s.split_once('.') {
        Some((i, f)) => (String::from(i), String::from(f)),
        None => (s, String::new()),
    }
}

fn grouped(int: &str, sep: char) -> String {
    let b = int.as_bytes();
    let mut out = String::new();
    for (k, c) in b.iter().enumerate() {
        if k > 0 && (b.len() - k) % 3 == 0 {
            out.push(sep);
        }
        out.push(*c as char);
    }
    out
}

// a picture's sections: split at ';' outside quotes
fn sections(p: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted, mut esc) = (0, false, false);
    for (i, c) in p.char_indices() {
        if esc {
            esc = false;
            continue;
        }
        match c {
            '\\' => esc = true,
            '"' => quoted = !quoted,
            ';' if !quoted => {
                out.push(&p[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&p[start..]);
    out
}

enum Tok {
    Lit(String),
    Num,
    Unit(bool),
    Percent,
}

fn number_picture(x: f64, unit: Option<Unit>, picture: &str, env: &Env) -> String {
    let secs = sections(picture);
    let (pic, mut v, signed) = if x < 0.0 && secs.len() > 1 {
        (secs[1], -x, false)
    } else if x == 0.0 && secs.len() > 2 {
        (secs[2], x, false)
    } else {
        (secs[0], x, true)
    };
    let shown_unit = converts(unit);
    if let Some(u) = shown_unit {
        v = u.from_internal(v);
    }
    // the picture's parts: literals, the number (at its first digit), units, percent
    let mut toks: Vec<Tok> = Vec::new();
    let (mut int0, mut frac0, mut frac_hash) = (0usize, 0usize, 0usize);
    let (mut group, mut point, mut have_num, mut in_digits) = (false, false, false, false);
    let mut lit = String::new();
    let flush = |lit: &mut String, toks: &mut Vec<Tok>| {
        if !lit.is_empty() {
            toks.push(Tok::Lit(core::mem::take(lit)));
        }
    };
    let mut it = pic.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '0' | '#' => {
                if !have_num {
                    flush(&mut lit, &mut toks);
                    toks.push(Tok::Num);
                    have_num = true;
                }
                in_digits = true;
                match (point, c) {
                    (false, '0') => int0 += 1,
                    (true, '0') => frac0 += 1,
                    (true, _) => frac_hash += 1,
                    _ => {}
                }
            }
            '.' if !point && (in_digits || matches!(it.peek(), Some('0' | '#'))) => {
                if !have_num {
                    flush(&mut lit, &mut toks);
                    toks.push(Tok::Num);
                    have_num = true;
                }
                point = true;
                in_digits = true;
            }
            ',' if in_digits && !point => group = true,
            '%' => {
                flush(&mut lit, &mut toks);
                toks.push(Tok::Percent);
                v *= 100.0;
                in_digits = false;
            }
            'u' | 'U' => {
                flush(&mut lit, &mut toks);
                if it.peek() == Some(&c) {
                    it.next();
                }
                toks.push(Tok::Unit(c == 'U'));
                in_digits = false;
            }
            '"' => {
                for d in it.by_ref() {
                    if d == '"' {
                        break;
                    }
                    lit.push(d);
                }
                in_digits = false;
            }
            '\\' => {
                if let Some(d) = it.next() {
                    lit.push(d);
                }
                in_digits = false;
            }
            c => {
                lit.push(c);
                in_digits = false;
            }
        }
    }
    flush(&mut lit, &mut toks);
    // the number
    let (int, mut frac) = round_split(v.abs(), frac0 + frac_hash);
    while frac.len() > frac0 && frac.ends_with('0') {
        frac.pop();
    }
    let mut int = String::from(int.trim_start_matches('0'));
    while int.len() < int0 {
        int.insert(0, '0');
    }
    let mut num = String::new();
    let nonzero = int.bytes().any(|b| b != b'0') || frac.bytes().any(|b| b != b'0');
    if signed && v < 0.0 && nonzero {
        num.push('-');
    }
    num.push_str(&if group { grouped(&int, env.group) } else { int });
    if !frac.is_empty() {
        num.push(env.decimal);
        num.push_str(&frac);
    }
    let mut out = String::new();
    for t in toks {
        match t {
            Tok::Lit(s) => out.push_str(&s),
            Tok::Num => out.push_str(&num),
            Tok::Percent => out.push('%'),
            Tok::Unit(upper) => {
                let a = shown_unit.map_or("", unit_abbr);
                if upper {
                    out.push_str(&a.to_uppercase());
                } else {
                    out.push_str(a);
                }
            }
        }
    }
    if !have_num {
        out.insert_str(0, &num);
    }
    out
}

// ---- dates

/// the days since 30 December 1899 of a date and time (Visio's date values)
pub fn ole_days(year: i64, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> f64 {
    let days = days_from_civil(year, month, day) + 25_569;
    days as f64 + (hour as f64 * 3600.0 + min as f64 * 60.0 + sec as f64) / 86_400.0
}

/// a date value as (year, month, day, hour, minute, second, weekday with Sunday 0)
pub fn civil(days: f64) -> (i64, u32, u32, u32, u32, u32, u32) {
    let mut d = math::floor(days) as i64;
    let mut secs = math::round((days - d as f64) * 86_400.0) as i64;
    if secs >= 86_400 {
        d += 1;
        secs -= 86_400;
    }
    let (y, m, dd) = civil_from_days(d - 25_569);
    // 30 Dec 1899 was a Saturday
    let wd = (d + 6).rem_euclid(7) as u32;
    (y, m, dd, (secs / 3600) as u32, (secs / 60 % 60) as u32, (secs % 60) as u32, wd)
}

// days since 1970-01-01 (H. Hinnant's algorithms)
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

fn date_picture(x: f64, picture: &str, env: &Env) -> String {
    let mut out = String::new();
    let mut rest = picture;
    while let Some(p) = rest.find("{{") {
        out.push_str(&literal(&rest[..p]));
        let inner = &rest[p + 2..];
        match inner.find("}}") {
            Some(e) => {
                out.push_str(&date_parts(x, &inner[..e], env));
                rest = &inner[e + 2..];
            }
            None => {
                out.push_str(&date_parts(x, inner, env));
                rest = "";
            }
        }
    }
    out.push_str(&literal(rest));
    out
}

fn first_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

// the date and time parts of a picture (inside the braces)
fn date_parts(x: f64, pat: &str, env: &Env) -> String {
    use core::fmt::Write as _;
    let (y, mo, d, h, mi, s, wd) = civil(x);
    let month = env.months[(mo as usize + 11) % 12];
    let day = env.days[wd as usize % 7];
    let mut out = String::new();
    let cs: Vec<char> = pat.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == '\'' || c == '"' {
            // literal up to the closing quote
            let mut j = i + 1;
            while j < cs.len() && cs[j] != c {
                out.push(cs[j]);
                j += 1;
            }
            i = j + 1;
            continue;
        }
        let mut n = 1;
        while i + n < cs.len() && cs[i + n] == c {
            n += 1;
        }
        let two = |out: &mut String, v: u32| {
            let _ = if n >= 2 { write!(out, "{v:02}") } else { write!(out, "{v}") };
        };
        match c {
            'y' => {
                let _ = if n <= 2 { write!(out, "{:02}", y.rem_euclid(100)) } else { write!(out, "{y:04}") };
            }
            'M' if n >= 4 => out.push_str(month),
            'M' if n == 3 => out.push_str(first_chars(month, 3)),
            'M' => two(&mut out, mo),
            'd' if n >= 4 => out.push_str(day),
            'd' if n == 3 => out.push_str(first_chars(day, 3)),
            'd' => two(&mut out, d),
            'H' => two(&mut out, h),
            'h' => two(&mut out, (h + 11) % 12 + 1),
            'm' => two(&mut out, mi),
            's' => two(&mut out, s),
            't' => {
                let m = env.am_pm[(h >= 12) as usize];
                out.push_str(if n >= 2 { m } else { first_chars(m, 1) });
            }
            _ => {
                for _ in 0..n {
                    out.push(c);
                }
            }
        }
        i += n;
    }
    out
}

// ---- fields of a shape

fn field_addr(ix: u32, cell: crate::strings::Sym) -> CellAddr {
    CellAddr::row(SecKey::one(names::FIELD), RowKey::Index(ix), cell)
}

/// the text Field row ix of a sheet shows: its value in its format ("" for an error)
pub fn shown(doc: &Document, sheet: SheetRef, ix: u32) -> String {
    let r = doc.resolve(sheet, &field_addr(ix, names::VALUE));
    if r.error().is_some() {
        return String::new();
    }
    let v = match r.value() {
        Value::Num(x) => Shown::Num(x, r.unit()),
        Value::Text(s) => Shown::Text(doc.str(s)),
        Value::Absent => Shown::Nothing,
    };
    format(&v, picture(doc, sheet, ix), &doc.fields)
}

/// the format picture of Field row ix ("" for none)
pub fn picture(doc: &Document, sheet: SheetRef, ix: u32) -> &str {
    match doc.resolve(sheet, &field_addr(ix, names::FORMAT)).value() {
        Value::Text(s) => doc.str(s),
        _ => "",
    }
}

/// the text inside a <fld ...>text</fld> element (a field as Visio writes it, kept as XML)
pub fn cached(xml: &str) -> Option<String> {
    let t = xml.trim_start();
    if !(t.starts_with("<fld") || t.contains(":fld")) {
        return None;
    }
    let open_end = t.find('>')?;
    if t[..open_end].ends_with('/') {
        return Some(String::new());
    }
    let close = t.rfind("</")?;
    if close < open_end {
        return None;
    }
    Some(crate::render::unescape(&t[open_end + 1..close]))
}

/// a run of a shape's text
#[derive(Clone, Debug, PartialEq)]
pub enum Seg {
    /// characters (the marks between them left out)
    Text(String),
    /// a field: its item in Text::items and the text it shows
    Field {
        /// the index into the text's items
        item: usize,
        /// what it shows
        text: String,
    },
}

impl Seg {
    /// the text the run shows
    pub fn text(&self) -> &str {
        match self {
            Seg::Text(t) => t,
            Seg::Field { text, .. } => text,
        }
    }
}

/// the runs of a text (`sheet`: the shape's, for the values of its fields)
pub fn segments_of(doc: &Document, sheet: SheetRef, text: &Text) -> Vec<Seg> {
    let mut out: Vec<Seg> = Vec::new();
    for (k, it) in text.items.iter().enumerate() {
        match it {
            TextItem::Chars(c) => match out.last_mut() {
                Some(Seg::Text(s)) => s.push_str(c),
                _ => out.push(Seg::Text(String::from(&**c))),
            },
            TextItem::Field(ix) => out.push(Seg::Field { item: k, text: shown(doc, sheet, *ix) }),
            TextItem::Other(x) => {
                if let Some(t) = cached(doc.str(*x)) {
                    out.push(Seg::Field { item: k, text: t });
                }
            }
            TextItem::Cp(_) | TextItem::Pp(_) | TextItem::Tp(_) => {}
        }
    }
    out
}

/// the runs of a shape's text (its master's when it has none of its own)
pub fn segments(doc: &Document, shape: ShapeRef) -> Vec<Seg> {
    match doc.shape(shape).and_then(|s| s.text()) {
        Some(t) => segments_of(doc, SheetRef::Shape(shape), t),
        None => Vec::new(),
    }
}

/// the text of a shape as it shows, fields included
pub fn plain(doc: &Document, shape: ShapeRef) -> String {
    segments(doc, shape).iter().map(|s| s.text()).collect()
}

/// what a field holds (its Type cell)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// a text (Type 0)
    Text,
    /// a number, also a length or an angle (Type 2)
    Number,
    /// a date or a time (Type 5)
    Date,
    /// a duration (Type 6)
    Duration,
    /// an amount of money (Type 7)
    Currency,
}

impl Kind {
    /// the Type cell's value
    pub fn code(self) -> f64 {
        match self {
            Kind::Text => 0.0,
            Kind::Number => 2.0,
            Kind::Date => 5.0,
            Kind::Duration => 6.0,
            Kind::Currency => 7.0,
        }
    }
}

/// a new field
#[derive(Clone, Copy, Debug)]
pub struct Spec<'a> {
    /// the Value cell's formula (Width, PAGENAME(), NOW(), Prop.Cost ...)
    pub formula: &'a str,
    /// the format picture ("" the general format)
    pub format: &'a str,
    /// the unit the value shows in (MM, DEG, DATE ...; None: a plain number; a text: STR)
    pub unit: Option<Unit>,
    /// what it holds
    pub kind: Kind,
}

// the item index and the byte in it where a text's shown position `at` lies (positions inside
// a field go behind it; None: at the end, in front of Visio's end mark)
fn place(doc: &Document, sheet: SheetRef, text: &Text, at: Option<usize>) -> (usize, usize) {
    let end = || {
        for (k, it) in text.items.iter().enumerate().rev() {
            match it {
                TextItem::Chars(c) if c.ends_with('\n') => return (k, c.len() - 1),
                TextItem::Chars(c) if !c.is_empty() => return (k + 1, 0),
                TextItem::Field(_) | TextItem::Other(_) => return (k + 1, 0),
                _ => {}
            }
        }
        (text.items.len(), 0)
    };
    let Some(at) = at else { return end() };
    let mut pos = 0usize;
    for (k, it) in text.items.iter().enumerate() {
        match it {
            TextItem::Chars(c) => {
                let n = c.chars().count();
                if at <= pos + n {
                    let byte = c.char_indices().nth(at - pos).map_or(c.len(), |(b, _)| b);
                    return (k, byte);
                }
                pos += n;
            }
            TextItem::Field(ix) => {
                if at <= pos {
                    return (k, 0);
                }
                pos += shown(doc, sheet, *ix).chars().count();
                if at < pos {
                    return (k + 1, 0);
                }
            }
            TextItem::Other(x) => {
                if let Some(t) = cached(doc.str(*x)) {
                    if at <= pos {
                        return (k, 0);
                    }
                    pos += t.chars().count();
                    if at < pos {
                        return (k + 1, 0);
                    }
                }
            }
            _ => {}
        }
    }
    end()
}

/// the mark of Field row N in a text written as source (MARK_OPEN N MARK_CLOSE: "{fld 0}"), as
/// the stencil build tool reads a master's text and the instance of such a master gets it (JD15)
pub const MARK_OPEN: &str = "{fld ";
/// the end of a field mark (MARK_OPEN)
pub const MARK_CLOSE: char = '}';

/// a text written with field marks ("Länge: {fld 0}") as characters and fields: "{fld N}" with N
/// a row index (decimal digits) is the mark of Field row N, everything else stays characters
/// (also a "{" that starts no such mark)
pub fn marked(s: &str) -> Text {
    let mut items = Vec::new();
    let mut chars = String::new();
    let mut rest = s;
    while let Some(i) = rest.find(MARK_OPEN) {
        let after = &rest[i + MARK_OPEN.len()..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        let ix = after[..digits].parse::<u32>().ok().filter(|_| after[digits..].starts_with(MARK_CLOSE));
        match ix {
            Some(ix) => {
                chars.push_str(&rest[..i]);
                if !chars.is_empty() {
                    items.push(TextItem::Chars(Box::from(chars.as_str())));
                    chars.clear();
                }
                items.push(TextItem::Field(ix));
                rest = &after[digits + MARK_CLOSE.len_utf8()..];
            }
            None => {
                chars.push_str(&rest[..i + 1]);
                rest = &rest[i + 1..];
            }
        }
    }
    chars.push_str(rest);
    if !chars.is_empty() || items.is_empty() {
        items.push(TextItem::Chars(Box::from(chars.as_str())));
    }
    Text { items, extras: crate::Extras::new() }
}

/// the Field rows a text's marks name (the IX of each TextItem::Field, in text order)
pub fn marks(text: &Text) -> Vec<u32> {
    text.items.iter().filter_map(|i| if let TextItem::Field(ix) = i { Some(*ix) } else { None }).collect()
}

/// a text with `item` put in at a shown position (fields count with the text they show; a
/// position inside a field goes behind it; None: at the end, in front of the last line break)
pub fn insert_item(doc: &Document, sheet: SheetRef, text: &Text, at: Option<usize>, item: TextItem) -> Text {
    let (k, byte) = place(doc, sheet, text, at);
    let mut t = text.clone();
    match t.items.get(k) {
        Some(TextItem::Chars(c)) if byte > 0 && byte < c.len() => {
            let (a, b) = (Box::<str>::from(&c[..byte]), Box::<str>::from(&c[byte..]));
            t.items[k] = TextItem::Chars(a);
            t.items.insert(k + 1, item);
            t.items.insert(k + 2, TextItem::Chars(b));
        }
        Some(TextItem::Chars(c)) if byte >= c.len() => t.items.insert(k + 1, item),
        _ => t.items.insert(k.min(t.items.len()), item),
    }
    t
}

/// a new field in a shape's text at the shown position `at` (None: at its end, on a line of
/// its own when the text has words): a Field row with the formula, the format and the type,
/// computed at once, and its mark in the text -> the row's IX
pub fn insert(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, at: Option<usize>, spec: &Spec) -> Result<u32, EditError> {
    let sheet = SheetRef::Shape(shape);
    let old = tx.doc().shape(shape).ok_or(EditError::NoShape)?.text().cloned().unwrap_or_default();
    let key = tx.add_row(sheet, SecKey::one(names::FIELD), None, None)?;
    let RowKey::Index(ix) = key else { return Err(EditError::Invalid) };
    let (first, unit) = match spec.kind {
        Kind::Text => (Value::Text(tx.intern("")), Some(Unit::Str)),
        _ => (Value::Num(0.0), spec.unit),
    };
    tx.set_constant(sheet, &field_addr(ix, names::VALUE), first, unit)?;
    tx.set_formula(sheet, &field_addr(ix, names::VALUE), spec.formula)?;
    tx.set_string(sheet, &field_addr(ix, names::FORMAT), spec.format)?;
    tx.set_constant(sheet, &field_addr(ix, names::TYPE), Value::Num(spec.kind.code()), None)?;
    let mut base = old;
    let text = match at {
        Some(_) => insert_item(tx.doc(), sheet, &base, at, TextItem::Field(ix)),
        None => {
            let now: String = segments_of(tx.doc(), sheet, &base).iter().map(|s| s.text()).collect();
            let end_mark = now.ends_with('\n');
            let body = now.strip_suffix('\n').unwrap_or(&now);
            let new_line = !body.is_empty() && !body.ends_with('\n');
            if end_mark {
                if new_line {
                    base = insert_item(tx.doc(), sheet, &base, None, TextItem::Chars(Box::from("\n")));
                }
                insert_item(tx.doc(), sheet, &base, None, TextItem::Field(ix))
            } else {
                // no end mark: behind everything
                if new_line {
                    base.items.push(TextItem::Chars(Box::from("\n")));
                }
                base.items.push(TextItem::Field(ix));
                base
            }
        }
    };
    eng.recalc_cells(tx, &[(sheet, field_addr(ix, names::VALUE))])?;
    crate::ops::set_text(tx, eng, shape, text)?;
    Ok(ix)
}

// ---- the document's properties

/// the document's properties (docProps/core.xml, docProps/app.xml); dates as days since
/// 30 Dec 1899 in local time (Env::utc_offset), None when the part does not say
#[derive(Clone, Debug, Default)]
pub struct Props {
    pub title: String,
    pub subject: String,
    pub creator: String,
    pub keywords: String,
    pub description: String,
    pub category: String,
    pub manager: String,
    pub company: String,
    pub hyperlink_base: String,
    pub created: Option<f64>,
    pub modified: Option<f64>,
    pub printed: Option<f64>,
}

// the text of the first element with this local name ("" when it is empty, None when missing)
fn elem_text(xml: &str, local: &str) -> Option<String> {
    let mut rest = xml;
    while let Some(p) = rest.find('<') {
        let tag = &rest[p + 1..];
        let end = tag.find('>')?;
        let head = &tag[..end];
        let name = head.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        let lname = name.rsplit_once(':').map_or(name, |(_, l)| l);
        if !head.starts_with('/') && !head.starts_with('?') && lname == local {
            if head.ends_with('/') {
                return Some(String::new());
            }
            let body = &tag[end + 1..];
            let close = body.find("</")?;
            return Some(crate::render::unescape(&body[..close]));
        }
        rest = &tag[end..];
    }
    None
}

// a W3CDTF time (2026-11-19T10:20:30Z, ...+01:00) as days since 30 Dec 1899, UTC
fn w3c_days(s: &str) -> Option<f64> {
    let s = s.trim();
    let num = |a: usize, b: usize| -> Option<u32> { s.get(a..b)?.parse().ok() };
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let (mo, d) = (num(5, 7).unwrap_or(1), num(8, 10).unwrap_or(1));
    let (h, mi, sec) = if s.len() >= 16 { (num(11, 13)?, num(14, 16)?, num(17, 19).unwrap_or(0)) } else { (0, 0, 0) };
    let mut v = ole_days(y, mo, d, h, mi, sec);
    if let Some(tail) = s.get(19..) {
        let tail = tail.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
        if let Some(sign) = tail.chars().next().filter(|c| *c == '+' || *c == '-') {
            let oh: f64 = tail.get(1..3).and_then(|x| x.parse().ok()).unwrap_or(0.0);
            let om: f64 = tail.get(4..6).and_then(|x| x.parse().ok()).unwrap_or(0.0);
            let off = (oh * 60.0 + om) / 1440.0;
            // local = UTC + offset
            v -= if sign == '+' { off } else { -off };
        }
    }
    Some(v)
}

fn part_text<'d>(doc: &'d Document, kind: &str) -> Option<&'d str> {
    doc.parts
        .iter()
        .find(|p| doc.str(p.content_type).contains(kind) || doc.str(p.rel_type).ends_with(kind))
        .and_then(|p| core::str::from_utf8(&p.data).ok())
}

/// the document's properties (see Props)
pub fn props(doc: &Document) -> Props {
    let mut p = Props::default();
    let off = doc.fields.utc_offset;
    if let Some(x) = part_text(doc, "core-properties") {
        let t = |n: &str| elem_text(x, n).unwrap_or_default();
        p.title = t("title");
        p.subject = t("subject");
        p.creator = t("creator");
        p.keywords = t("keywords");
        p.description = t("description");
        p.category = t("category");
        let date = |n: &str| elem_text(x, n).and_then(|s| w3c_days(&s)).map(|v| v + off);
        p.created = date("created");
        p.modified = date("modified");
        p.printed = date("lastPrinted");
    }
    if let Some(x) = part_text(doc, "extended-properties") {
        let t = |n: &str| elem_text(x, n).unwrap_or_default();
        p.manager = t("Manager");
        p.company = t("Company");
        p.hyperlink_base = t("HyperlinkBase");
    }
    p
}

#[cfg(test)]
mod tests;
