// json -- the report's JSON: the top level one member per line, nested objects one member per line,
// arrays of numbers and objects marked compact on one line (a vector, a part, the timing), so the report
// reads well and a line filter can leave out the parts that change from run to run (selftest.rs compares
// the examples' reports without "files", "views" and "timing").  Numbers through geom::fnum (nine
// decimals: a nanometre).

use alloc::string::String;
use alloc::vec::Vec;

/// a JSON value
#[derive(Clone, Debug)]
pub enum J {
    Null,
    Bool(bool),
    Int(i64),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
    /// written on one line wherever it is
    Compact(alloc::boxed::Box<J>),
}

impl J {
    pub fn obj(fields: Vec<(&str, J)>) -> J {
        J::Obj(fields.into_iter().map(|(k, v)| (String::from(k), v)).collect())
    }

    pub fn compact(self) -> J {
        J::Compact(alloc::boxed::Box::new(self))
    }

    pub fn str(s: &str) -> J {
        J::Str(String::from(s))
    }

    pub fn vec3(v: geom::Vec3) -> J {
        J::Arr(alloc::vec![J::Num(v.x), J::Num(v.y), J::Num(v.z)])
    }

    pub fn nums(v: &[f64]) -> J {
        J::Arr(v.iter().map(|&x| J::Num(x)).collect())
    }

    /// an array of scalars only (written on one line); an array of compact values gets a line per value
    fn flat(&self) -> bool {
        match self {
            J::Arr(a) => a.iter().all(|x| !matches!(x, J::Arr(_) | J::Obj(_) | J::Compact(_))),
            J::Obj(_) => false,
            _ => true,
        }
    }
}

fn escape(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 32 => out.push_str(&alloc::format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn inline(out: &mut String, v: &J) {
    match v {
        J::Null => out.push_str("null"),
        J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        J::Int(i) => out.push_str(&alloc::format!("{}", i)),
        J::Num(x) => out.push_str(&geom::fnum(*x)),
        J::Str(s) => escape(out, s),
        J::Arr(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                inline(out, x);
            }
            out.push(']');
        }
        J::Obj(o) => {
            out.push('{');
            for (i, (k, x)) in o.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                escape(out, k);
                out.push_str(": ");
                inline(out, x);
            }
            out.push('}');
        }
        J::Compact(b) => inline(out, b),
    }
}

fn pad(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn pretty(out: &mut String, v: &J, depth: usize) {
    match v {
        J::Obj(o) if !o.is_empty() => {
            out.push_str("{\n");
            for (i, (k, x)) in o.iter().enumerate() {
                pad(out, depth + 1);
                escape(out, k);
                out.push_str(": ");
                pretty(out, x, depth + 1);
                if i + 1 < o.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            pad(out, depth);
            out.push('}');
        }
        J::Arr(a) if !a.is_empty() && !v.flat() => {
            out.push_str("[\n");
            for (i, x) in a.iter().enumerate() {
                pad(out, depth + 1);
                pretty(out, x, depth + 1);
                if i + 1 < a.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            pad(out, depth);
            out.push(']');
        }
        _ => inline(out, v),
    }
}

/// the value as text with a final line break
pub fn write(v: &J) -> String {
    let mut s = String::new();
    pretty(&mut s, v, 0);
    s.push('\n');
    s
}
