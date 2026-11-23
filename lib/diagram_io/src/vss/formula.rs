// formula -- the formulas of the binary format turned back into the text Visio shows
//
// A formula block holds tokens in postfix order, ended by 0xFE:
//   0x20..0x5F  a number: the token is its unit code (0x20 none, 0x21 percent, 0x46 mm ...),
//               a double follows (in internal units)
//   0x60        a string: a byte n, n UTF-16 units, a NUL unit; 0x69 the same with a u32 n
//   0x61        TRUE or FALSE; 0x62 an integer of two bytes (0x63: four bytes)
//   0x75        a cell of the shape's own single-row sections: a byte, the cell's number in the
//               shape's cell table (object_cell), two bytes, 0x60
//   0x74        the same in another shape: those four bytes, then the shape's ID (u32)
//   0x72        a cell of a row: the row (u32), the section (Visio's section index: 10.. the
//               Geometry sections, 240 Actions, 242 User ...), the cell, a flag byte; the row of
//               a named section (User, Prop, Controls, Actions) is the index of its name in the
//               document's name list
//   0x70        the same in another shape: the shape's ID (u32) before
//   0x76        GetRef of such a cell (two bytes, a u32, the shape's ID, row, section, cell, flag)
//   0x80 0x7A   a function with its fixed number of arguments: its ID (u16), two bytes
//   0x81 0x7B   a function with a count of arguments (u32) before its ID
//   0x03..0x0E  + - * / ^ & < <= = >= > <>; 0x13 the unary minus; 0xE4 the parentheses around
//               the value before (Visio keeps them, so no precedence is needed)
//   0x90        IF of the three values before it; 0x91 AND, 0x92 OR, each with a u32 that counts
//               their values and jumps (2n - 1); 0x93 and 0xA0..0xA3 (+ u32) the jumps of the
//               lazy arguments, which the text does not show
// Anything else, or a table without the name a token needs, leaves the whole formula out: the
// cell keeps its value (V1), and the reader warns.  The meanings come from evaluating formulas
// against the values the files store with them and from the same files saved by Visio as .vssx;
// the tables hold what such files showed.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::stream::{f64at, u16at, u32at};

/// what a formula needs from its document: the name list and the ID of the formula's own shape
pub(super) struct Ctx<'a> {
    pub names: &'a [String],
    pub own: u32,
}

// the binary operators by token
fn binary(op: u8) -> Option<&'static str> {
    Some(match op {
        0x03 => "+",
        0x04 => "-",
        0x05 => "*",
        0x06 => "/",
        0x07 => "^",
        0x08 => "&",
        0x09 => "<",
        0x0A => "<=",
        0x0B => "=",
        0x0C => ">=",
        0x0D => ">",
        0x0E => "<>",
        _ => return None,
    })
}

/// the shape cells a token 0x75 / 0x74 names, by their number (those found in files so far)
pub(super) fn object_cell(n: u8) -> Option<&'static str> {
    Some(match n {
        0 => "PinX",
        1 => "PinY",
        2 => "Width",
        3 => "Height",
        4 => "LocPinX",
        5 => "LocPinY",
        6 => "Angle",
        7 => "FlipX",
        8 => "FlipY",
        9 => "ResizeMode",
        10 => "BeginX",
        11 => "BeginY",
        12 => "EndX",
        13 => "EndY",
        14 => "LineWeight",
        15 => "LineColor",
        17 => "FillForegnd",
        31 => "TxtWidth",
        32 => "TxtHeight",
        52 => "LeftMargin",
        53 => "RightMargin",
        75 => "TheData",
        76 => "TheText",
        77 => "EventDblClick",
        78 => "EventXFMod",
        79 => "EventDrop",
        92 => "HideText",
        102 => "BegTrigger",
        103 => "EndTrigger",
        _ => return None,
    })
}

/// a function's name and its number of arguments (used when the token does not count them)
pub(super) fn function(id: u16) -> Option<(&'static str, usize)> {
    Some(match id {
        0x09 => ("MAX", 2),
        0x0A => ("MIN", 2),
        0x0F => ("ABS", 1),
        0x10 => ("POW", 2),
        0x11 => ("SQRT", 1),
        0x12 => ("ATAN2", 2),
        0x1F => ("NOT", 1),
        0x22 => ("BITXOR", 2),
        0x24 => ("COS", 1),
        0x26 => ("SIN", 1),
        0x2D => ("TEXTWIDTH", 1),
        0x2E => ("TEXTHEIGHT", 2),
        0x41 => ("GUARD", 1),
        0x42 => ("MAGNITUDE", 4),
        0x4A => ("NA", 0),
        0x4C => ("OPENTEXTWIN", 0),
        0x5B => ("SIGN", 1),
        0x60 => ("GRAVITY", 1),
        0x64 => ("USERUI", 3),
        0x65 => ("_UCON_C1", 5),
        0x66 => ("_UCON_C2", 2),
        0x67 => ("_UCON_D1", 5),
        0x68 => ("_UCON_D2", 2),
        0x69 => ("_UCON_X1", 10),
        0x6A => ("_UCON_X2", 8),
        0x6B => ("_UCON_Y1", 10),
        0x6C => ("_UCON_Y2", 8),
        0x6D => ("_UCON_SIMPLE", 8),
        0x6E => ("_UCON_BEGTYP", 5),
        0x6F => ("_UCON_ENDTYP", 5),
        0x74 => ("_UCON_C3", 2),
        0x75 => ("_UCON_D3", 2),
        0x76 => ("_UCON_X3", 9),
        0x77 => ("_UCON_Y3", 9),
        0x78 => ("_UCON_GEOTYP", 2),
        0x73 => ("_XFTRIGGER", 1),
        0x7E => ("SETF", 2),
        0x7F => ("LOOKUP", 2),
        0x80 => ("INDEX", 2),
        0x82 => ("DOCMD", 1),
        0x83 => ("RGB", 3),
        0xC1 => ("MASTERNAME", 0),
        0xC2 => ("FIELDPICTURE", 1),
        0xC3 => ("STRSAME", 2),
        0xC5 => ("SHAPETEXT", 1),
        0xD3 => ("BOUND", 5),
        0xDB => ("THEME", 1),
        0xE3 => ("THEMEGUARD", 1),
        0x10F => ("THEMEVAL", 2),
        _ => return None,
    })
}

// a call of `name` with the last `n` values as its arguments
fn call(st: &mut Vec<String>, name: &str, n: usize) -> Option<()> {
    if n > st.len() {
        return None;
    }
    let args = st.split_off(st.len() - n);
    st.push(format!("{name}({})", args.join(",")));
    Some(())
}

/// the section's name in a reference, the names of its cells by number, and whether its rows
/// are named
fn section(sec: u8) -> Option<(&'static str, &'static [&'static str], bool)> {
    Some(match sec {
        3 => ("Char", &["Font", "Color", "Style", "Case", "Pos", "FontScale", "", "Size"], false),
        6 => ("Scratch", &["X", "Y", "A", "B", "C", "D"], false),
        7 => ("Connections", &["X", "Y", "DirX", "DirY", "Type", "AutoGen", "Prompt"], false),
        9 => ("Controls", &["X", "Y", "XDyn", "YDyn", "XCon", "YCon", "CanGlue", "", "Prompt"], true),
        240 => ("Actions", &["Menu", "", "", "Action", "Checked", "Disabled"], true),
        242 => ("User", &["Value", "Prompt"], true),
        243 => ("Prop", &["Value", "Prompt", "Label", "Format", "SortKey", "Type", "Invisible", "Verify"], true),
        _ => return None,
    })
}

fn row_ref(cx: &Ctx, shape: Option<u32>, row: u32, sec: u8, cell: u8) -> Option<String> {
    let prefix = match shape {
        Some(id) if id != cx.own => format!("Sheet.{id}!"),
        _ => String::new(),
    };
    if (10..=239).contains(&sec) {
        let n = sec as u32 - 9;
        if row == 0 {
            let c = ["NoFill", "NoLine", "NoShow", "NoSnap", "NoQuickDrag"].get(cell as usize)?;
            return Some(format!("{prefix}Geometry{n}.{c}"));
        }
        let c = ["X", "Y", "A", "B", "C", "D", "E"].get(cell as usize)?;
        return Some(format!("{prefix}Geometry{n}.{c}{row}"));
    }
    let (name, cells, named) = section(sec)?;
    let c = *cells.get(cell as usize).filter(|c| !c.is_empty())?;
    if named {
        let r = cx.names.get(row as usize).filter(|r| !r.is_empty())?;
        return Some(if cell == 0 { format!("{prefix}{name}.{r}") } else { format!("{prefix}{name}.{r}.{c}") });
    }
    if sec == 3 {
        return Some(if row == 0 { format!("{prefix}{name}.{c}") } else { format!("{prefix}{name}.{c}[{}]", row + 1) });
    }
    Some(format!("{prefix}{name}.{c}{}", row + 1))
}

/// a number in formula text as Visio writes it: at most 15 significant digits, E notation below
/// 1E-5 and from 1E15
pub(super) fn fnum(v: f64) -> String {
    if v == 0.0 || !v.is_finite() {
        return String::from("0");
    }
    let sci = format!("{:e}", v.abs());
    let e: i32 = sci.split_once('e').and_then(|(_, e)| e.parse().ok()).unwrap_or(0);
    if !(-5..15).contains(&e) {
        let m = format!("{:.14e}", v);
        let (mant, ex) = m.split_once('e').unwrap_or((m.as_str(), "0"));
        let mant = trim_zeros(mant);
        let ex: i32 = ex.parse().unwrap_or(0);
        return if ex < 0 { format!("{mant}E-{}", -ex) } else { format!("{mant}E+{ex}") };
    }
    let d = (14 - e).max(0) as usize;
    let s = trim_zeros(&format!("{:.*}", d, v));
    if s == "-0" {
        String::from("0")
    } else {
        s
    }
}

fn trim_zeros(s: &str) -> String {
    if s.contains('.') {
        String::from(s.trim_end_matches('0').trim_end_matches('.'))
    } else {
        String::from(s)
    }
}

// a number token as formula text: its value in the unit it names, the unit's code after it
fn number(code: u8, v: f64) -> Option<String> {
    let (k, unit) = match code {
        0x20 => (1.0, ""),
        0x21 => return Some(format!("{}%", fnum(v * 100.0))),
        0x30 => (72.0, "DT"),
        0x32 => (72.0, "PT"),
        0x3F => (1.0, "DP"),
        0x40 => (1.0, "DL"),
        0x41 => (1.0, "IN"),
        0x45 => (2.54, "CM"),
        0x46 => (25.4, "MM"),
        0x47 => (0.0254, "M"),
        0x50 => (180.0 / core::f64::consts::PI, "DA"),
        0x51 => (180.0 / core::f64::consts::PI, "DEG"),
        0x53 => (1.0, "RAD"),
        _ => return None,
    };
    Some(format!("{}{unit}", fnum(v * k)))
}

/// the formula text of a block's tokens; None when a token or a name is unknown
pub(super) fn decode(t: &[u8], cx: &Ctx) -> Option<String> {
    let mut st: Vec<String> = Vec::new();
    let mut i = 0;
    while i < t.len() {
        let c = t[i];
        i += 1;
        match c {
            0xFE => return if i == t.len() && st.len() == 1 { st.pop() } else { None },
            0x20..=0x5F => {
                let v = f64at(t, i)?;
                i += 8;
                st.push(number(c, v)?);
            }
            0x60 | 0x69 => {
                let (n, w) = if c == 0x60 { (*t.get(i)? as usize, 1) } else { (u32at(t, i)? as usize, 4) };
                let end = n.checked_mul(2).and_then(|b| b.checked_add(i + w))?;
                let b = t.get(i + w..end)?;
                i = end + 2;
                let units: Vec<u16> = b.chunks_exact(2).map(|x| u16::from_le_bytes([x[0], x[1]])).collect();
                let s = String::from_utf16_lossy(&units);
                st.push(format!("\"{}\"", s.replace('"', "\"\"")));
            }
            0x61 => {
                st.push(String::from(if *t.get(i)? != 0 { "TRUE" } else { "FALSE" }));
                i += 1;
            }
            0x62 => {
                st.push(format!("{}", u16at(t, i)?));
                i += 2;
            }
            0x63 => {
                st.push(format!("{}", u32at(t, i)? as i32));
                i += 4;
            }
            0x75 => {
                let n = *t.get(i)?;
                i += 4;
                st.push(String::from(object_cell(n)?));
            }
            0x74 => {
                let n = *t.get(i)?;
                let id = u32at(t, i + 4)?;
                i += 8;
                let c = object_cell(n)?;
                st.push(if id == cx.own { String::from(c) } else { format!("Sheet.{id}!{c}") });
            }
            0x72 => {
                let row = u32at(t, i)?;
                let (sec, cell) = (*t.get(i + 4)?, *t.get(i + 5)?);
                i += 7;
                st.push(row_ref(cx, None, row, sec, cell)?);
            }
            0x70 => {
                let id = u32at(t, i)?;
                let row = u32at(t, i + 4)?;
                let (sec, cell) = (*t.get(i + 8)?, *t.get(i + 9)?);
                i += 11;
                st.push(row_ref(cx, Some(id), row, sec, cell)?);
            }
            0x76 => {
                let id = u32at(t, i + 6)?;
                let row = u32at(t, i + 10)?;
                let (sec, cell) = (*t.get(i + 14)?, *t.get(i + 15)?);
                i += 17;
                let r = row_ref(cx, Some(id), row, sec, cell)?;
                st.push(format!("GetRef({r})"));
            }
            0x80 | 0x7A => {
                let id = u16at(t, i)?;
                i += 4;
                let (name, n) = function(id)?;
                call(&mut st, name, n)?;
            }
            0x81 | 0x7B => {
                let n = u32at(t, i)? as usize;
                let id = u16at(t, i + 4)?;
                i += 8;
                let (name, _) = function(id)?;
                call(&mut st, name, n)?;
            }
            0x90 => call(&mut st, "IF", 3)?,
            0x91 | 0x92 => {
                let n = (u32at(t, i)? as usize).div_ceil(2);
                i += 4;
                call(&mut st, if c == 0x91 { "AND" } else { "OR" }, n)?;
            }
            0x93 => {}
            0xA0..=0xA3 => i += 4,
            0x13 => {
                let x = st.pop()?;
                st.push(format!("-{x}"));
            }
            0xE4 => {
                let x = st.pop()?;
                st.push(format!("({x})"));
            }
            _ => {
                let op = binary(c)?;
                let b = st.pop()?;
                let a = st.pop()?;
                st.push(format!("{a}{op}{b}"));
            }
        }
    }
    None
}

/// whether a formula is a single literal (a number with or without unit, a string, TRUE or
/// FALSE, a colour RGB(r,g,b) of numbers): Visio writes such a cell as its value alone
pub(super) fn is_literal(f: &str) -> bool {
    if f == "TRUE" || f == "FALSE" {
        return true;
    }
    if let Some(args) = f.strip_prefix("RGB(").and_then(|x| x.strip_suffix(')')) {
        let parts: Vec<&str> = args.split(',').collect();
        return parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    }
    if let Some(inner) = f.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
        return !inner.replace("\"\"", "").contains('"');
    }
    let body = f.trim_end_matches(|c: char| c.is_ascii_uppercase() || c == '%');
    let body = body.strip_prefix('-').unwrap_or(body);
    !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit() || b == b'.') || {
        // 8.1204884086868E-15
        let (m, e) = body.split_once('E').unwrap_or(("", ""));
        !m.is_empty() && m.bytes().all(|b| b.is_ascii_digit() || b == b'.') && e.trim_start_matches(['+', '-']).bytes().all(|b| b.is_ascii_digit())
    }
}

/// the text of a string literal ("a ""b""" -> a "b")
pub(super) fn literal_text(f: &str) -> Option<String> {
    let inner = f.strip_prefix('"')?.strip_suffix('"')?;
    Some(inner.replace("\"\"", "\""))
}
