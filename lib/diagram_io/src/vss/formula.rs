// formula -- the formulas of the binary format turned back into the text Visio shows
//
// A formula block holds tokens in postfix order, ended by 0xFE:
//   0x20..0x5F  a number: the token is its unit code (0x20 none, 0x21 percent, 0x46 mm ...),
//               a double follows
//   0x60        a string: a byte n, n UTF-16 units, a NUL unit
//   0x61 0x62   an integer of one / two bytes (0x63: four bytes)
//   0x75        a cell of the shape's own single-row sections: a byte, the cell's number in the
//               shape's cell table (OBJECT_CELLS), two bytes, 0x60
//   0x74        the same in another shape: those four bytes, then the shape's ID (u32)
//   0x72        a cell of a row: the row (u32), the section (Visio's section index: 10.. the
//               Geometry sections, 240 Actions, 242 User ...), the cell, a flag byte
//   0x70        the same in another shape: the shape's ID (u32) before
//   0x76        a row of another sheet (17 bytes)
//   0x80        a function with its usual number of arguments: its ID (u16), two bytes
//   0x81 0x7B   a function with a count of arguments (u32) before its ID
//   0x7A        a function of one string
//   0x00..0x1F  operators; 0xE4 the parentheses around the value before
//   0x90..0x93  the ends of IF, AND, OR ...; 0xA0..0xA3 (+ u32) the jumps of their lazy
//               arguments, which the text does not show
// Anything else, or a table without the name a token needs, leaves the whole formula out: the
// cell keeps its value (V1), and the reader warns.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::cells::num;
use super::stream::{f64at, u16at, u32at};

/// what a formula's references need: the names of rows by (shape ID or None for the own shape,
/// section, row)
pub(super) trait Names {
    fn row(&self, shape: Option<u32>, section: u8, row: u32) -> Option<String>;
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

/// the shape cells a token 0x75 / 0x74 names, by their number
pub(super) fn object_cell(n: u16) -> Option<&'static str> {
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
        _ => return None,
    })
}

/// a function's name by its ID
pub(super) fn function(id: u16) -> Option<&'static str> {
    Some(match id {
        0x11 => "SQRT",
        _ => return None,
    })
}

/// the section's name in a reference and the names of its cells by number
fn section(sec: u8) -> Option<(&'static str, &'static [&'static str], bool)> {
    // (name in a formula, cells, named rows)
    Some(match sec {
        3 => ("Char", &["Font", "Color", "Style", "Case", "Pos", "FontScale", "Size"], false),
        6 => ("Scratch", &["X", "Y", "A", "B", "C", "D"], false),
        7 => ("Connections", &["X", "Y", "DirX", "DirY", "Type", "AutoGen", "Prompt"], false),
        9 => ("Controls", &["X", "Y", "XDyn", "YDyn", "XCon", "YCon", "CanGlue", "Prompt"], true),
        10..=239 => ("Geometry", &["X", "Y", "A", "B", "C", "D", "E"], false),
        240 => ("Actions", &["Menu", "Prompt", "Help", "Action", "Checked", "Disabled", "", "ReadOnly", "Invisible", "BeginGroup"], true),
        242 => ("User", &["Value", "Prompt"], true),
        243 => ("Prop", &["Value", "Prompt", "Label", "Format", "SortKey", "Type", "Invisible", "Verify"], true),
        _ => return None,
    })
}

fn row_ref(names: &dyn Names, shape: Option<u32>, row: u32, sec: u8, cell: u8) -> Option<String> {
    let (name, cells, named) = section(sec)?;
    let c = *cells.get(cell as usize).filter(|c| !c.is_empty())?;
    let prefix = shape.map(|id| format!("Sheet.{id}!")).unwrap_or_default();
    if (10..=239).contains(&sec) {
        return Some(format!("{prefix}{name}{}.{c}{row}", sec as u32 - 9));
    }
    if named {
        let r = names.row(shape, sec, row)?;
        return Some(if c == "Value" { format!("{prefix}{name}.{r}") } else { format!("{prefix}{name}.{r}.{c}") });
    }
    Some(format!("{prefix}{name}.{c}[{}]", row + 1))
}

// a number token as formula text: its value in the unit it names
fn number(code: u8, v: f64) -> String {
    let unit = match code {
        0x21 => return format!("{}%", num(v * 100.0)),
        0x46 => " mm",
        0x45 => " cm",
        0x47 => " m",
        0x41 => " in",
        0x32 => " pt",
        0x51 => " deg",
        _ => "",
    };
    let shown = match code {
        0x46 => v * 25.4,
        0x45 => v * 2.54,
        0x47 => v * 0.0254,
        0x32 => v * 72.0,
        0x51 => v.to_degrees(),
        _ => v,
    };
    format!("{}{unit}", num(shown))
}

/// the formula text of a block's tokens; None when a token or a name is unknown
pub(super) fn decode(t: &[u8], names: &dyn Names) -> Option<String> {
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
                st.push(number(c, v));
            }
            0x60 => {
                let n = *t.get(i)? as usize;
                let b = t.get(i + 1..i + 1 + 2 * n)?;
                i += 1 + 2 * n + 2;
                let units: Vec<u16> = b.chunks_exact(2).map(|x| u16::from_le_bytes([x[0], x[1]])).collect();
                let s = String::from_utf16_lossy(&units);
                st.push(format!("\"{}\"", s.replace('"', "\"\"")));
            }
            0x61 => {
                st.push(format!("{}", *t.get(i)?));
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
                let n = *t.get(i)? as u16;
                i += 4;
                st.push(String::from(object_cell(n)?));
            }
            0x74 => {
                let n = *t.get(i)? as u16;
                let id = u32at(t, i + 4)?;
                i += 8;
                st.push(format!("Sheet.{id}!{}", object_cell(n)?));
            }
            0x72 => {
                let row = u32at(t, i)?;
                let (sec, cell) = (*t.get(i + 4)?, *t.get(i + 5)?);
                i += 7;
                st.push(row_ref(names, None, row, sec, cell)?);
            }
            0x70 => {
                let id = u32at(t, i)?;
                let row = u32at(t, i + 4)?;
                let (sec, cell) = (*t.get(i + 8)?, *t.get(i + 9)?);
                i += 11;
                st.push(row_ref(names, Some(id), row, sec, cell)?);
            }
            0x80 => {
                let id = u16at(t, i)?;
                i += 4;
                let _ = function(id)?;
                return None;
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
