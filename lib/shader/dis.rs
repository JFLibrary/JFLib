// dis -- SPIR-V words as text in the style of spirv-dis (friendly names, the result column
// at 15 characters)
//
//   disassemble(words) -> Result<String, String>
//   fmt_float(f32) -> String    a float as spirv-dis prints it (%.9g, hex for inf / NaN)
//
// Names follow spirv-dis' FriendlyNameMapper: OpName first, then generated names for types
// (void, uint, v4float, mat4v4float, _arr_float_uint_4, _runtimearr_X, _ptr_Uniform_X,
// _struct_N) and constants (uint_4, int_n1, float_0_5, true); a taken name gets _0, _1 ...;
// everything else is its number.  Opcodes shader does not know print as OpUnknown(N) with
// their words.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::parse::{self, Inst};
use crate::spv::{self, op};

#[derive(Clone, Copy)]
enum Num {
    Int(u32, bool),
    Float(u32),
}

struct Names {
    map: BTreeMap<u32, String>,
    used: BTreeSet<String>,
}

fn sanitize(s: &str) -> String {
    if s.is_empty() {
        return String::from("_");
    }
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

impl Names {
    fn save(&mut self, id: u32, suggested: &str) {
        if self.map.contains_key(&id) {
            return;
        }
        let base = sanitize(suggested);
        let mut name = base.clone();
        let mut k = 0;
        while self.used.contains(&name) {
            name = format!("{}_{}", base, k);
            k += 1;
        }
        self.used.insert(name.clone());
        self.map.insert(id, name);
    }
    fn of(&self, id: u32) -> String {
        self.map.get(&id).cloned().unwrap_or_else(|| id.to_string())
    }
}

/// %.Ng of C (N significant digits, exponent form outside 1e-4 .. 1e N)
fn fmt_g(x: f64, prec: usize) -> String {
    if x == 0.0 {
        return String::from(if x.is_sign_negative() { "-0" } else { "0" });
    }
    let (sign, a) = if x < 0.0 { ("-", -x) } else { ("", x) };
    let e = format!("{:.*e}", prec - 1, a);
    let (mant, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let trim = |s: &str| -> String {
        if s.contains('.') {
            String::from(s.trim_end_matches('0').trim_end_matches('.'))
        } else {
            String::from(s)
        }
    };
    if exp < -4 || exp >= prec as i32 {
        format!("{}{}e{}{:02}", sign, trim(mant), if exp < 0 { '-' } else { '+' }, exp.unsigned_abs())
    } else {
        let decimals = (prec as i32 - 1 - exp).max(0) as usize;
        format!("{}{}", sign, trim(&format!("{:.*}", decimals, a)))
    }
}

fn hex_float(bits: u64, frac_bits: u32, exp_bias: u32, neg: bool) -> String {
    let frac = bits & ((1u64 << frac_bits) - 1);
    let digits = frac_bits.div_ceil(4);
    let f = frac << (digits * 4 - frac_bits);
    let mut s = format!("{:0w$x}", f, w = digits as usize);
    while s.ends_with('0') {
        s.pop();
    }
    let dot = if s.is_empty() { String::new() } else { format!(".{}", s) };
    format!("{}0x1{}p+{}", if neg { "-" } else { "" }, dot, exp_bias + 1)
}

/// a 32-bit float as spirv-dis prints it
pub fn fmt_float(x: f32) -> String {
    if x.is_finite() {
        fmt_g(x as f64, 9)
    } else {
        hex_float(x.to_bits() as u64 & 0x7f_ffff, 23, 127, x.is_sign_negative())
    }
}

fn fmt_double(x: f64) -> String {
    if x.is_finite() {
        fmt_g(x, 17)
    } else {
        hex_float(x.to_bits() & 0xf_ffff_ffff_ffff, 52, 1023, x.is_sign_negative())
    }
}

fn fmt_num(n: Option<Num>, w: &[u32]) -> String {
    let lo = w.first().copied().unwrap_or(0);
    let wide = (w.get(1).copied().unwrap_or(0) as u64) << 32 | lo as u64;
    match n {
        Some(Num::Int(64, true)) => (wide as i64).to_string(),
        Some(Num::Int(64, false)) => wide.to_string(),
        Some(Num::Int(width, true)) => {
            let sh = 32 - width.min(32);
            (((lo << sh) as i32) >> sh).to_string()
        }
        Some(Num::Float(64)) => fmt_double(f64::from_bits(wide)),
        Some(Num::Float(32)) => fmt_float(f32::from_bits(lo)),
        _ => lo.to_string(),
    }
}

fn mask(table: &[(u32, &str, &str)], v: u32) -> String {
    if v == 0 {
        return String::from("None");
    }
    let mut parts = Vec::new();
    let mut rest = v;
    for &(bit, name, _) in table {
        if v & bit != 0 {
            parts.push(String::from(name));
            rest &= !bit;
        }
    }
    if rest != 0 {
        parts.push(format!("0x{:x}", rest));
    }
    parts.join("|")
}

fn enum_name(table: &[(u32, &'static str)], v: u32) -> String {
    spv::name(table, v).map_or_else(|| v.to_string(), String::from)
}

fn quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        if c == '"' || c == '\\' {
            o.push('\\');
        }
        o.push(c);
    }
    o.push('"');
    o
}

/// the names of the ids, the number kinds of the scalar types, the extended instruction sets
fn scan(insts: &[Inst]) -> (Names, BTreeMap<u32, Num>, BTreeMap<u32, String>) {
    let mut n = Names { map: BTreeMap::new(), used: BTreeSet::new() };
    let mut nums = BTreeMap::new();
    let mut sets = BTreeMap::new();
    for i in insts {
        let o = i.ops;
        match i.op {
            op::Name if !o.is_empty() => n.save(o[0], &parse::string(&o[1..]).0),
            op::ExtInstImport => {
                sets.insert(i.res, parse::string(o).0);
            }
            op::TypeVoid => n.save(i.res, "void"),
            op::TypeBool => n.save(i.res, "bool"),
            op::TypeInt if o.len() >= 2 => {
                nums.insert(i.res, Num::Int(o[0], o[1] != 0));
                let root = match o[0] {
                    8 => String::from("char"),
                    16 => String::from("short"),
                    32 => String::from("int"),
                    64 => String::from("long"),
                    w => format!("i{}", w),
                };
                n.save(i.res, &format!("{}{}", if o[1] == 0 { "u" } else { "" }, root));
            }
            op::TypeFloat if !o.is_empty() => {
                nums.insert(i.res, Num::Float(o[0]));
                let s = match o[0] {
                    16 => String::from("half"),
                    32 => String::from("float"),
                    64 => String::from("double"),
                    w => format!("fp{}", w),
                };
                n.save(i.res, &s);
            }
            op::TypeVector if o.len() >= 2 => n.save(i.res, &format!("v{}{}", o[1], n.of(o[0]))),
            op::TypeMatrix if o.len() >= 2 => n.save(i.res, &format!("mat{}{}", o[1], n.of(o[0]))),
            op::TypeArray if o.len() >= 2 => n.save(i.res, &format!("_arr_{}_{}", n.of(o[0]), n.of(o[1]))),
            op::TypeRuntimeArray if !o.is_empty() => n.save(i.res, &format!("_runtimearr_{}", n.of(o[0]))),
            op::TypePointer if o.len() >= 2 => {
                n.save(i.res, &format!("_ptr_{}_{}", enum_name(spv::SCS, o[0]), n.of(o[1])))
            }
            op::TypeStruct => n.save(i.res, &format!("_struct_{}", i.res)),
            op::ConstantTrue => n.save(i.res, "true"),
            op::ConstantFalse => n.save(i.res, "false"),
            op::Constant => {
                let v = fmt_num(nums.get(&i.ty).copied(), o).replace('-', "n");
                n.save(i.res, &format!("{}_{}", n.of(i.ty), v));
            }
            _ => {}
        }
    }
    (n, nums, sets)
}

/// the module as text
pub fn disassemble(words: &[u32]) -> Result<String, String> {
    if words.len() < 5 || words[0] != spv::MAGIC {
        return Err(String::from("not a SPIR-V module (magic number)"));
    }
    let insts = parse::parse(words)?;
    let (names, nums, sets) = scan(&insts);
    let generator = words[2];
    let vendor = if generator >> 16 == 0 { String::from("Khronos") } else { format!("Unknown({})", generator >> 16) };
    let mut out = format!(
        "; SPIR-V\n; Version: {}.{}\n; Generator: {}; {}\n; Bound: {}\n; Schema: {}\n",
        words[1] >> 16 & 0xff, words[1] >> 8 & 0xff, vendor, generator & 0xffff, words[3], words[4]
    );
    for i in &insts {
        let mut line = String::new();
        if i.res != 0 {
            let id = format!("%{}", names.of(i.res));
            for _ in id.len()..12 {
                line.push(' ');
            }
            line.push_str(&id);
            line.push_str(" = ");
        } else {
            line.push_str("               ");
        }
        match i.info {
            Some(info) => {
                line.push_str("Op");
                line.push_str(info.name);
            }
            None => line.push_str(&format!("OpUnknown({})", i.op)),
        }
        if i.ty != 0 {
            line.push_str(" %");
            line.push_str(&names.of(i.ty));
        }
        let mut last_id = 0;
        let num = nums.get(&i.ty).copied();
        parse::operands(i, &mut |k, w| {
            let s = match k {
                'i' => {
                    last_id = w[0];
                    format!("%{}", names.of(w[0]))
                }
                's' => quote(&parse::string(w).0),
                'v' => fmt_num(num, w),
                'x' => match sets.get(&last_id).map(String::as_str) {
                    Some("GLSL.std.450") => {
                        spv::GLSL_NAMES.get(w[0] as usize).filter(|s| !s.is_empty()).map_or_else(|| w[0].to_string(), |s| String::from(*s))
                    }
                    _ => w[0].to_string(),
                },
                'C' => enum_name(spv::CAPS, w[0]),
                'A' => enum_name(spv::ADDRESSING, w[0]),
                'Y' => enum_name(spv::MEMORY_MODELS, w[0]),
                'M' => enum_name(spv::MODELS, w[0]),
                'E' => enum_name(spv::MODES, w[0]),
                'd' => enum_name(spv::DECS, w[0]),
                'B' => enum_name(spv::BIS, w[0]),
                'S' => enum_name(spv::SCS, w[0]),
                'D' => enum_name(spv::DIMS, w[0]),
                'F' => enum_name(spv::FMTS, w[0]),
                'Q' => enum_name(spv::ACCESS, w[0]),
                'k' => enum_name(spv::LANGS, w[0]),
                'f' => mask(spv::FN_CTL, w[0]),
                'e' => mask(spv::SEL_CTL, w[0]),
                'l' => mask(spv::LOOP_CTL, w[0]),
                'm' => mask(spv::MEM_ACCESS, w[0]),
                'I' => mask(spv::IMG_OPS, w[0]),
                _ => w[0].to_string(),
            };
            line.push(' ');
            line.push_str(&s);
        });
        out.push_str(&line);
        out.push('\n');
    }
    Ok(out)
}
