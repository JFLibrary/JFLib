// ops::text -- formula texts as the edits write and rewrite them
//
//   num(x)                       a number as a formula writes it (at most 14 significant digits)
//   items(formula)               the sheet parts and cell references of a formula text, with
//                                their byte ranges (strings, numbers with units and function
//                                names are skipped)
//   replace(formula, edits)      the text with byte ranges replaced
//   proportional(formula)        "Width*0.25" / "Sheet.5!Height*0.5": the reference and factor
//   call_arg(formula, "BOUND")   the range of the first argument of a call around the whole
//                                formula
//
// The edits keep a formula's form where Visio keeps it (a subshape's PinX stays
// "Sheet.<group>!Width*<k>", a control's X stays "Width*<k>"), renumber row references when a
// row is inserted into a section (Geometry1.X3 -> Geometry1.X4) and give pasted shapes their
// new IDs (Sheet.5! -> Sheet.12!).  The parser (formula::parse) keeps no positions, so this is
// a small lexer of its own; it follows the universal syntax of formula/parse.rs: strings in
// double quotes, quoted names in single quotes, numbers with a unit right after them, names
// of letters, digits, '_' and '.', a sheet part ending in '!', an index in brackets.

use alloc::string::String;
use alloc::vec::Vec;

use crate::formula::math;

/// a number as the edits write it into formulas: rounded to 14 significant digits (as Visio
/// writes its factors), no exponent, no trailing zeros
pub(crate) fn num(x: f64) -> String {
    // below a millionth of a millionth: noise of the arithmetic (lengths are inches)
    if !x.is_finite() || x.abs() < 1e-12 {
        return String::from("0");
    }
    let e = math::floor(math::log10(x.abs()));
    let digits = (13.0 - e).clamp(0.0, 20.0) as usize;
    let mut s = alloc::format!("{x:.digits$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = String::from("0");
    }
    s
}

/// one part of a formula text that names a sheet or a cell
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Item<'a> {
    /// a sheet part without its '!': Sheet.5, 'My name', ThePage, Name.7; `foreign` when it
    /// stands behind Pages[..]! or Masters[..]! (a shape of another page or master)
    Sheet { text: &'a str, range: (usize, usize), foreign: bool, quoted: bool },
    /// a cell reference: Width, Geometry1.X3, Char.Size[2]; `sheet` is the index of its sheet
    /// part in the list (None: the formula's own sheet); `bracket` the number in [n] and the
    /// range of the whole reference including the bracket
    Cell { name: &'a str, sheet: Option<usize>, foreign: bool, bracket: Option<u32>, range: (usize, usize) },
}

fn is_name_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b >= 0x80
}

fn is_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b >= 0x80
}

/// the sheet parts and cell references of a formula text in the order they are written
pub(crate) fn items(f: &str) -> Vec<Item<'_>> {
    let b = f.as_bytes();
    let mut out: Vec<Item> = Vec::new();
    // the sheet part in front of the next name (its index), and whether a Pages[..]! or
    // Masters[..]! stands in front
    let mut pending: Option<usize> = None;
    let mut foreign = false;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'"' {
            // a string: "" inside is one quote
            i += 1;
            while i < b.len() {
                if b[i] == b'"' {
                    if b.get(i + 1) == Some(&b'"') {
                        i += 2;
                        continue;
                    }
                    break;
                }
                i += 1;
            }
            i += 1;
            pending = None;
            foreign = false;
        } else if c == b'\'' {
            let start = i;
            i += 1;
            while i < b.len() && b[i] != b'\'' {
                i += 1;
            }
            let end = (i + 1).min(b.len());
            i = end;
            if b.get(i) == Some(&b'!') {
                out.push(Item::Sheet { text: &f[start..end], range: (start, end), foreign, quoted: true });
                pending = Some(out.len() - 1);
                i += 1;
            } else {
                pending = None;
                foreign = false;
            }
        } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            // a number with an optional exponent and a unit right after it (0.25DL, 180DEG)
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                let mut j = i + 1;
                if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                    j += 1;
                }
                if j < b.len() && b[j].is_ascii_digit() {
                    i = j;
                    while i < b.len() && b[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            while i < b.len() && (b[i].is_ascii_alphabetic() || b[i] == b'_') {
                i += 1;
            }
            pending = None;
            foreign = false;
        } else if is_name_start(c) {
            let start = i;
            while i < b.len() && is_name_char(b[i]) {
                i += 1;
            }
            let name_end = i;
            let mut bracket: Option<(usize, usize)> = None;
            if b.get(i) == Some(&b'[') {
                let open = i;
                let mut j = i + 1;
                while j < b.len() && b[j] != b']' {
                    j += 1;
                }
                if j < b.len() {
                    bracket = Some((open + 1, j));
                    i = j + 1;
                }
            }
            let name = &f[start..name_end];
            if b.get(i) == Some(&b'!') {
                if bracket.is_some() && matches!(name, "Pages" | "Masters" | "Styles") {
                    foreign = true;
                    pending = None;
                } else {
                    out.push(Item::Sheet { text: name, range: (start, name_end), foreign, quoted: false });
                    pending = Some(out.len() - 1);
                }
                i += 1;
                continue;
            }
            // a function call is no reference
            let mut j = i;
            while j < b.len() && b[j] == b' ' {
                j += 1;
            }
            if b.get(j) != Some(&b'(') {
                let n = bracket.and_then(|(s, e)| f[s..e].trim().parse::<u32>().ok());
                out.push(Item::Cell { name, sheet: pending, foreign, bracket: n, range: (start, i) });
            }
            pending = None;
            foreign = false;
        } else {
            if c != b' ' {
                pending = None;
                foreign = false;
            }
            i += 1;
        }
    }
    out
}

/// the text with byte ranges replaced (the ranges must not overlap)
pub(crate) fn replace(f: &str, mut edits: Vec<((usize, usize), String)>) -> String {
    edits.sort_by_key(|e| e.0 .0);
    let mut out = String::with_capacity(f.len() + 8);
    let mut at = 0;
    for ((s, e), text) in edits {
        if s < at {
            continue;
        }
        out.push_str(&f[at..s]);
        out.push_str(&text);
        at = e;
    }
    out.push_str(&f[at..]);
    out
}

/// a proportional formula of the form [Sheet.N!]Width*k or [Sheet.N!]Height*k (Visio's form
/// for subshapes of a group, control points, local pins): the sheet ID (None: the own sheet),
/// whether it is Height, and k
pub(crate) fn proportional(f: &str) -> Option<(Option<u32>, bool, f64)> {
    let t = f.trim();
    let (sheet, rest) = match t.strip_prefix("Sheet.") {
        Some(r) => {
            let (id, rest) = r.split_once('!')?;
            (Some(id.parse::<u32>().ok()?), rest)
        }
        None => (None, t),
    };
    let (cell, k) = rest.split_once('*')?;
    let height = match cell.trim() {
        "Width" => false,
        "Height" => true,
        _ => return None,
    };
    let k = k.trim();
    if k.is_empty() || !k.bytes().all(|c| c.is_ascii_digit() || c == b'.' || c == b'-' || c == b'E' || c == b'e' || c == b'+') {
        return None;
    }
    let k: f64 = k.parse().ok()?;
    k.is_finite().then_some((sheet, height, k))
}

/// the byte range of the first argument when the whole formula is a call of `func`
/// (BOUND(<first>, ...)), else None
pub(crate) fn call_arg(f: &str, func: &str) -> Option<(usize, usize)> {
    let lead = f.len() - f.trim_start().len();
    let t = &f[lead..];
    if t.len() <= func.len() || !t.as_bytes()[..func.len()].eq_ignore_ascii_case(func.as_bytes()) {
        return None;
    }
    let mut i = lead + func.len();
    let b = f.as_bytes();
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    if b.get(i) != Some(&b'(') {
        return None;
    }
    let start = i + 1;
    let mut depth = 0i32;
    let mut j = start;
    while j < b.len() {
        match b[j] {
            b'"' => {
                j += 1;
                while j < b.len() && b[j] != b'"' {
                    j += 1;
                }
            }
            b'(' | b'[' => depth += 1,
            b')' | b']' if depth > 0 => depth -= 1,
            b',' | b')' if depth == 0 => return Some((start, j)),
            _ => {}
        }
        j += 1;
    }
    None
}

/// how formulas number the rows of an indexed section ([MS-VSDX] universal syntax, JD6):
/// Geometry<n>.X<k> is row IX k, Connections.X<k> and Scratch.X<k> row IX k - 1, Char.Size[k]
/// row IX k - 1 (Char.Size alone row 0)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) enum Form {
    Geometry,
    Numbered,
    Bracket,
}

/// the prefix formulas use for the rows of a section and how they number them (None: a
/// section whose rows formulas do not number, or one of several names)
pub(crate) fn row_form(section: &str, index: u32) -> Option<(String, Form)> {
    Some(match section {
        "Geometry" => (alloc::format!("Geometry{}", index + 1), Form::Geometry),
        "Connection" => (String::from("Connections"), Form::Numbered),
        "Scratch" => (String::from("Scratch"), Form::Numbered),
        "Character" => (String::from("Char"), Form::Bracket),
        "Paragraph" => (String::from("Para"), Form::Bracket),
        "Tabs" => (String::from("Tabs"), Form::Bracket),
        "Field" => (String::from("Fields"), Form::Bracket),
        "Layer" => (String::from("Layers"), Form::Bracket),
        _ => return None,
    })
}

// "X12" -> ("X", 12)
fn split_digits(s: &str) -> Option<(&str, u32)> {
    let cut = s.trim_end_matches(|c: char| c.is_ascii_digit());
    if cut.is_empty() || cut.len() == s.len() {
        return None;
    }
    s[cut.len()..].parse().ok().map(|n| (cut, n))
}

/// a formula with its references to the rows of one section renumbered: `map` gives a row's
/// new IX (None: the row is gone, the reference becomes `gone(cell, ix)`); references with
/// the sheet part Sheet.<own>! count as the formula's own sheet, other sheets stay
pub(crate) fn renumber_rows(
    f: &str,
    own: Option<u32>,
    prefix: &str,
    form: Form,
    map: &dyn Fn(u32) -> Option<u32>,
    gone: &mut dyn FnMut(&str, u32) -> String,
) -> String {
    let list = items(f);
    let own_name = own.map(|id| alloc::format!("Sheet.{id}"));
    let mut edits: Vec<((usize, usize), String)> = Vec::new();
    for it in &list {
        let Item::Cell { name, sheet, foreign: false, bracket, range } = it else { continue };
        let start = match sheet {
            None => range.0,
            Some(si) => match &list[*si] {
                Item::Sheet { text, range: sr, quoted: false, foreign: false } if own_name.as_deref() == Some(*text) => sr.0,
                _ => continue,
            },
        };
        let Some((head, rest)) = name.split_once('.') else { continue };
        if head != prefix || rest.contains('.') {
            continue;
        }
        let (cell, ix) = match form {
            Form::Geometry | Form::Numbered => {
                if bracket.is_some() {
                    continue;
                }
                let Some((cell, n)) = split_digits(rest) else { continue };
                let ix = if form == Form::Geometry { n } else if n >= 1 { n - 1 } else { continue };
                (cell, ix)
            }
            Form::Bracket => (rest, bracket.map_or(Some(0), |n| n.checked_sub(1)).unwrap_or(0)),
        };
        match map(ix) {
            Some(nx) if nx == ix => {}
            Some(nx) => {
                let text = match form {
                    Form::Geometry => alloc::format!("{head}.{cell}{nx}"),
                    Form::Numbered => alloc::format!("{head}.{cell}{}", nx + 1),
                    Form::Bracket if nx == 0 => alloc::format!("{head}.{cell}"),
                    Form::Bracket => alloc::format!("{head}.{cell}[{}]", nx + 1),
                };
                edits.push(((range.0, range.1), text));
            }
            None => edits.push(((start, range.1), gone(cell, ix))),
        }
    }
    if edits.is_empty() { String::from(f) } else { replace(f, edits) }
}

/// whether a formula text calls `func` anywhere (ASCII case ignored)
pub(crate) fn calls(f: &str, func: &str) -> bool {
    let n = func.len();
    f.as_bytes().windows(n + 1).any(|w| w[..n].eq_ignore_ascii_case(func.as_bytes()) && w[n] == b'(')
}
