// Colour spaces (ISO 32000-1, 8.6) and functions (7.10) as drawing needs them: every colour
// becomes sRGB-ish device RGB. DeviceGray, DeviceRGB and DeviceCMYK are taken as they are
// (CMYK as (1 - c)(1 - k), like the JPEG decoder), CalGray and CalRGB as the device spaces,
// Lab through XYZ to sRGB, ICCBased as the device space of its component count (or its
// /Alternate), Indexed through its table, Separation and DeviceN through their tint transform
// (Separation /All as black, /None paints nothing), Pattern with its underlying space.
// Functions: sampled (type 0, multilinear), exponential (2), stitching (3) and the PostScript
// calculator (4, compiled once, a budget per call).

use super::super::object::{Dict, Object};
use super::super::Document;
use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;

const MAX_DEPTH: u32 = 8;
const MAX_SAMPLES: usize = 1 << 22;
const PS_STACK: usize = 100;
const PS_BUDGET: u32 = 10_000;
const PS_MAX_OPS: usize = 1 << 16;

pub(super) fn floor(x: f64) -> f64 {
    if !(x.abs() < 4.0e18) {
        return x;
    }
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

pub(super) fn ceil(x: f64) -> f64 {
    -floor(-x)
}

pub(super) fn sqrt(x: f64) -> f64 {
    if !(x > 0.0) {
        return 0.0;
    }
    rt::fx::sqrtd(x)
}

pub(super) fn pow(x: f64, y: f64) -> f64 {
    if x < 0.0 {
        if floor(y) == y {
            let r = rt::x87::pow(-x, y);
            return if (y as i64) & 1 == 1 { -r } else { r };
        }
        return 0.0;
    }
    if x == 0.0 {
        return if y > 0.0 { 0.0 } else if y == 0.0 { 1.0 } else { f64::INFINITY };
    }
    rt::x87::pow(x, y)
}

fn num(doc: &Document, o: &Object) -> Option<f64> {
    doc.resolve(o).as_f64().filter(|v| v.is_finite())
}

pub(super) fn nums(doc: &Document, o: &Object) -> Vec<f64> {
    match doc.resolve(o) {
        Object::Array(a) => a.iter().filter_map(|v| num(doc, v)).collect(),
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------- functions

#[derive(Clone, Copy, Debug)]
pub(super) enum Ps {
    Num(f64),
    Bool(bool),
    Op(u8),
    /// jump forward by n when the popped value is false
    IfNot(u32),
    Jump(u32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum V {
    R(f64),
    I(i64),
    B(bool),
}

impl V {
    fn f(self) -> f64 {
        match self {
            V::R(v) => v,
            V::I(v) => v as f64,
            V::B(b) => b as u8 as f64,
        }
    }
}

static PS_OPS: [&[u8]; 40] = [
    b"abs", b"add", b"atan", b"ceiling", b"cos", b"cvi", b"cvr", b"div", b"exp", b"floor", b"idiv", b"ln", b"log", b"mod",
    b"mul", b"neg", b"round", b"sin", b"sqrt", b"sub", b"truncate", b"and", b"bitshift", b"eq", b"false", b"ge", b"gt",
    b"le", b"lt", b"ne", b"not", b"or", b"true", b"xor", b"copy", b"dup", b"exch", b"index", b"pop", b"roll",
];

pub(super) enum Func {
    Sampled { m: usize, n: usize, domain: Vec<f64>, range: Vec<f64>, size: Vec<usize>, encode: Vec<f64>, decode: Vec<f64>, samples: Vec<f32> },
    Exp { c0: Vec<f64>, c1: Vec<f64>, e: f64, domain: [f64; 2], range: Vec<f64> },
    Stitch { fns: Vec<Rc<Func>>, bounds: Vec<f64>, encode: Vec<f64>, domain: [f64; 2], n: usize },
    Calc { prog: Vec<Ps>, domain: Vec<f64>, range: Vec<f64> },
    /// an array of functions of one output each
    Many(Vec<Rc<Func>>),
}

fn clampd(v: f64, lo: f64, hi: f64) -> f64 {
    if v.is_nan() {
        lo
    } else if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

fn interp(x: f64, x0: f64, x1: f64, y0: f64, y1: f64) -> f64 {
    if (x1 - x0).abs() < 1e-30 { y0 } else { y0 + (x - x0) * (y1 - y0) / (x1 - x0) }
}

impl Func {
    pub fn parse(doc: &Document, o: &Object) -> Option<Rc<Func>> {
        Func::parse_at(doc, o, 0)
    }

    fn parse_at(doc: &Document, o: &Object, depth: u32) -> Option<Rc<Func>> {
        if depth > MAX_DEPTH {
            return None;
        }
        let o = doc.resolve(o);
        if let Object::Array(a) = &o {
            let fns: Option<Vec<_>> = a.iter().take(32).map(|f| Func::parse_at(doc, f, depth + 1)).collect();
            return Some(Rc::new(Func::Many(fns?)));
        }
        let (d, stream) = match &o {
            Object::Dict(d) => (d.clone(), None),
            Object::Stream(s) => (s.dict.clone(), Some(s.clone())),
            _ => return None,
        };
        let domain = nums(doc, d.get("Domain")?);
        let range = d.get("Range").map(|r| nums(doc, r)).unwrap_or_default();
        if domain.len() < 2 || domain.len() % 2 != 0 {
            return None;
        }
        match doc.key(&d, "FunctionType").as_int()? {
            0 => {
                let s = stream?;
                let m = domain.len() / 2;
                let n = range.len() / 2;
                if n == 0 || m > 8 {
                    return None;
                }
                let size: Vec<usize> = nums(doc, d.get("Size")?).iter().map(|&v| v.max(1.0).min(1e7) as usize).collect();
                if size.len() != m {
                    return None;
                }
                let total = size.iter().try_fold(n, |acc, &s| acc.checked_mul(s).filter(|&t| t <= MAX_SAMPLES))?;
                let bps = doc.key(&d, "BitsPerSample").as_int()?;
                if ![1, 2, 4, 8, 12, 16, 24, 32].contains(&bps) {
                    return None;
                }
                let mut encode = d.get("Encode").map(|e| nums(doc, e)).unwrap_or_default();
                if encode.len() != 2 * m {
                    encode = size.iter().flat_map(|&s| [0.0, (s - 1) as f64]).collect();
                }
                let mut decode = d.get("Decode").map(|e| nums(doc, e)).unwrap_or_default();
                if decode.len() != 2 * n {
                    decode = range.clone();
                }
                let data = doc.decode(&s).ok()?;
                let mut samples = Vec::with_capacity(total);
                let max = ((1u64 << bps) - 1) as f64;
                let mut bit = 0usize;
                for _ in 0..total {
                    let mut v: u64 = 0;
                    for _ in 0..bps {
                        let byte = data.get(bit / 8).copied().unwrap_or(0);
                        v = v << 1 | ((byte >> (7 - bit % 8)) & 1) as u64;
                        bit += 1;
                    }
                    samples.push((v as f64 / max) as f32);
                }
                Some(Rc::new(Func::Sampled { m, n, domain, range, size, encode, decode, samples }))
            }
            2 => {
                let c0 = d.get("C0").map(|v| nums(doc, v)).unwrap_or_else(|| vec![0.0]);
                let c1 = d.get("C1").map(|v| nums(doc, v)).unwrap_or_else(|| vec![1.0]);
                if c0.len() != c1.len() || c0.is_empty() {
                    return None;
                }
                let e = doc.key(&d, "N").as_f64()?;
                Some(Rc::new(Func::Exp { c0, c1, e, domain: [domain[0], domain[1]], range }))
            }
            3 => {
                let fo = doc.key(&d, "Functions");
                let fa = fo.as_array()?;
                if fa.is_empty() || fa.len() > 1024 {
                    return None;
                }
                let fns: Option<Vec<_>> = fa.iter().map(|f| Func::parse_at(doc, f, depth + 1)).collect();
                let fns = fns?;
                let bounds = nums(doc, d.get("Bounds")?);
                let encode = nums(doc, d.get("Encode")?);
                if bounds.len() + 1 != fns.len() || encode.len() < 2 * fns.len() {
                    return None;
                }
                let n = fns[0].outputs();
                Some(Rc::new(Func::Stitch { fns, bounds, encode, domain: [domain[0], domain[1]], n }))
            }
            4 => {
                let s = stream?;
                let data = doc.decode(&s).ok()?;
                let prog = compile(&data)?;
                Some(Rc::new(Func::Calc { prog, domain, range }))
            }
            _ => None,
        }
    }

    pub fn outputs(&self) -> usize {
        match self {
            Func::Sampled { n, .. } => *n,
            Func::Exp { c0, .. } => c0.len(),
            Func::Stitch { n, .. } => *n,
            Func::Calc { range, .. } => (range.len() / 2).max(1),
            Func::Many(f) => f.len(),
        }
    }

    /// the outputs for `input` into out (cleared first)
    pub fn eval(&self, input: &[f64], out: &mut Vec<f64>) {
        out.clear();
        self.eval_at(input, out, 0);
    }

    fn eval_at(&self, input: &[f64], out: &mut Vec<f64>, depth: u32) {
        if depth > MAX_DEPTH {
            return;
        }
        match self {
            Func::Many(fs) => {
                let mut one = Vec::new();
                for f in fs {
                    one.clear();
                    f.eval_at(input, &mut one, depth + 1);
                    out.push(one.first().copied().unwrap_or(0.0));
                }
            }
            Func::Exp { c0, c1, e, domain, range } => {
                let x = clampd(input.first().copied().unwrap_or(0.0), domain[0], domain[1]);
                let xe = pow(x, *e);
                for i in 0..c0.len() {
                    let mut v = c0[i] + xe * (c1[i] - c0[i]);
                    if range.len() >= 2 * i + 2 {
                        v = clampd(v, range[2 * i], range[2 * i + 1]);
                    }
                    out.push(v);
                }
            }
            Func::Stitch { fns, bounds, encode, domain, .. } => {
                let x = clampd(input.first().copied().unwrap_or(0.0), domain[0], domain[1]);
                let mut k = bounds.iter().position(|&b| x < b).unwrap_or(bounds.len());
                k = k.min(fns.len() - 1);
                let lo = if k == 0 { domain[0] } else { bounds[k - 1] };
                let hi = if k == bounds.len() { domain[1] } else { bounds[k] };
                let t = interp(x, lo, hi, encode[2 * k], encode[2 * k + 1]);
                fns[k].eval_at(&[t], out, depth + 1);
            }
            Func::Sampled { m, n, domain, range, size, encode, decode, samples } => {
                // the position in the sample grid of each input, then multilinear
                let mut idx = [0usize; 8];
                let mut frac = [0f64; 8];
                for i in 0..*m {
                    let x = clampd(input.get(i).copied().unwrap_or(0.0), domain[2 * i], domain[2 * i + 1]);
                    let e = interp(x, domain[2 * i], domain[2 * i + 1], encode[2 * i], encode[2 * i + 1]);
                    let e = clampd(e, 0.0, (size[i] - 1) as f64);
                    let f = floor(e);
                    idx[i] = (f as usize).min(size[i] - 1);
                    frac[i] = if idx[i] + 1 < size[i] { e - f } else { 0.0 };
                }
                for j in 0..*n {
                    let mut acc = 0.0;
                    for corner in 0..(1usize << *m) {
                        let mut w = 1.0;
                        let mut off = 0usize;
                        let mut stride = 1usize;
                        for i in 0..*m {
                            let up = corner >> i & 1 == 1;
                            w *= if up { frac[i] } else { 1.0 - frac[i] };
                            let p = if up { (idx[i] + 1).min(size[i] - 1) } else { idx[i] };
                            off += p * stride;
                            stride *= size[i];
                        }
                        if w != 0.0 {
                            acc += w * samples.get(off * n + j).copied().unwrap_or(0.0) as f64;
                        }
                    }
                    let v = decode[2 * j] + acc * (decode[2 * j + 1] - decode[2 * j]);
                    out.push(clampd(v, range[2 * j], range[2 * j + 1]));
                }
            }
            Func::Calc { prog, domain, range } => {
                let mut st: Vec<V> = Vec::with_capacity(16);
                for i in 0..domain.len() / 2 {
                    st.push(V::R(clampd(input.get(i).copied().unwrap_or(0.0), domain[2 * i], domain[2 * i + 1])));
                }
                run(prog, &mut st);
                let n = range.len() / 2;
                let base = st.len().saturating_sub(n);
                for j in 0..n {
                    let v = st.get(base + j).map(|v| v.f()).unwrap_or(0.0);
                    out.push(clampd(v, range[2 * j], range[2 * j + 1]));
                }
            }
        }
    }
}

enum Item {
    P(Ps),
    Proc(Vec<Item>),
}

/// a PostScript calculator program: { ... } with nested { } if and { } { } ifelse, as one list
/// of instructions with forward jumps
fn compile(src: &[u8]) -> Option<Vec<Ps>> {
    use super::super::lex::Tok;
    let mut lx = super::super::lex::Lexer::new(src, 0);
    let mut stack: Vec<Vec<Item>> = Vec::new();
    let mut ops = 0usize;
    let top = loop {
        let t = lx.next();
        lx.reset_budget();
        ops += 1;
        if ops > PS_MAX_OPS || stack.len() > 100 {
            return None;
        }
        let item = match t {
            Tok::Eof => return None,
            Tok::BraceOpen => {
                stack.push(Vec::new());
                continue;
            }
            Tok::BraceClose => {
                let done = stack.pop()?;
                match stack.last_mut() {
                    Some(parent) => {
                        parent.push(Item::Proc(done));
                        continue;
                    }
                    None => break done,
                }
            }
            Tok::Int(v) => Ps::Num(v as f64),
            Tok::Real(v) => Ps::Num(v),
            Tok::Kw(b"true") => Ps::Bool(true),
            Tok::Kw(b"false") => Ps::Bool(false),
            Tok::Kw(b"if") => {
                let cur = stack.last_mut()?;
                let Some(Item::Proc(a)) = cur.pop() else { return None };
                cur.push(Item::Proc(vec![Item::Proc(a)]));
                continue;
            }
            Tok::Kw(b"ifelse") => {
                let cur = stack.last_mut()?;
                let Some(Item::Proc(b)) = cur.pop() else { return None };
                let Some(Item::Proc(a)) = cur.pop() else { return None };
                cur.push(Item::Proc(vec![Item::Proc(a), Item::Proc(b)]));
                continue;
            }
            Tok::Kw(k) => Ps::Op(PS_OPS.iter().position(|&o| o == k)? as u8),
            _ => return None,
        };
        stack.last_mut()?.push(Item::P(item));
    };
    let mut out = Vec::new();
    flatten(&top, &mut out, 0)?;
    Some(out)
}

/// a conditional is a Proc holding one (if) or two (ifelse) Procs; a Proc anywhere else is an error
fn flatten(items: &[Item], out: &mut Vec<Ps>, depth: u32) -> Option<()> {
    if depth > 100 {
        return None;
    }
    for it in items {
        match it {
            Item::P(p) => out.push(*p),
            Item::Proc(c) => match c.as_slice() {
                [Item::Proc(a)] => {
                    let at = out.len();
                    out.push(Ps::IfNot(0));
                    flatten(a, out, depth + 1)?;
                    out[at] = Ps::IfNot((out.len() - at - 1) as u32);
                }
                [Item::Proc(a), Item::Proc(b)] => {
                    let at = out.len();
                    out.push(Ps::IfNot(0));
                    flatten(a, out, depth + 1)?;
                    let j = out.len();
                    out.push(Ps::Jump(0));
                    out[at] = Ps::IfNot((out.len() - at - 1) as u32);
                    flatten(b, out, depth + 1)?;
                    out[j] = Ps::Jump((out.len() - j - 1) as u32);
                }
                _ => return None,
            },
        }
    }
    Some(())
}

fn run(prog: &[Ps], st: &mut Vec<V>) {
    let mut pc = 0usize;
    let mut budget = PS_BUDGET;
    while pc < prog.len() {
        budget -= 1;
        if budget == 0 || st.len() > PS_STACK {
            return;
        }
        let p = prog[pc];
        pc += 1;
        match p {
            Ps::Num(v) => st.push(if floor(v) == v && v.abs() < 1e15 { V::I(v as i64) } else { V::R(v) }),
            Ps::Bool(b) => st.push(V::B(b)),
            Ps::Jump(n) => pc += n as usize,
            Ps::IfNot(n) => {
                let c = match st.pop() {
                    Some(V::B(b)) => b,
                    Some(v) => v.f() != 0.0,
                    None => return,
                };
                if !c {
                    pc += n as usize;
                }
            }
            Ps::Op(o) => {
                if !op(o, st) {
                    return;
                }
            }
        }
    }
}

fn op(o: u8, st: &mut Vec<V>) -> bool {
    let name = PS_OPS[o as usize];
    macro_rules! pop {
        () => {
            match st.pop() {
                Some(v) => v,
                None => return false,
            }
        };
    }
    match name {
        b"abs" | b"neg" | b"ceiling" | b"floor" | b"round" | b"truncate" | b"cvi" | b"cvr" | b"sqrt" | b"sin" | b"cos" | b"ln" | b"log" => {
            let v = pop!();
            let r = match (name, v) {
                (b"abs", V::I(i)) => V::I(i.wrapping_abs()),
                (b"neg", V::I(i)) => V::I(i.wrapping_neg()),
                (b"abs", v) => V::R(v.f().abs()),
                (b"neg", v) => V::R(-v.f()),
                (b"ceiling" | b"floor" | b"round" | b"truncate", V::I(i)) => V::I(i),
                (b"ceiling", v) => V::R(-floor(-v.f())),
                (b"floor", v) => V::R(floor(v.f())),
                (b"round", v) => V::R(floor(v.f() + 0.5)),
                (b"truncate", v) => V::R(if v.f() < 0.0 { -floor(-v.f()) } else { floor(v.f()) }),
                (b"cvi", v) => {
                    let f = v.f();
                    V::I(if f.abs() < 9.0e18 { f as i64 } else { 0 })
                }
                (b"cvr", v) => V::R(v.f()),
                (b"sqrt", v) => V::R(sqrt(v.f())),
                (b"sin", v) => V::R(rt::x87::sincos(v.f().to_radians()).1),
                (b"cos", v) => V::R(rt::x87::sincos(v.f().to_radians()).0),
                (b"ln", v) => V::R(if v.f() > 0.0 { rt::x87::rz_log2(v.f()) * core::f64::consts::LN_2 } else { 0.0 }),
                (_, v) => V::R(if v.f() > 0.0 { rt::x87::rz_log2(v.f()) * core::f64::consts::LOG10_2 } else { 0.0 }),
            };
            st.push(r);
        }
        b"add" | b"sub" | b"mul" | b"div" | b"idiv" | b"mod" | b"atan" | b"exp" => {
            let b = pop!();
            let a = pop!();
            let r = match (name, a, b) {
                (b"add", V::I(x), V::I(y)) => V::I(x.wrapping_add(y)),
                (b"sub", V::I(x), V::I(y)) => V::I(x.wrapping_sub(y)),
                (b"mul", V::I(x), V::I(y)) => match x.checked_mul(y) {
                    Some(v) => V::I(v),
                    None => V::R(x as f64 * y as f64),
                },
                (b"idiv", V::I(x), V::I(y)) => V::I(if y == 0 { 0 } else { x.wrapping_div(y) }),
                (b"mod", V::I(x), V::I(y)) => V::I(if y == 0 { 0 } else { x.wrapping_rem(y) }),
                (b"add", a, b) => V::R(a.f() + b.f()),
                (b"sub", a, b) => V::R(a.f() - b.f()),
                (b"mul", a, b) => V::R(a.f() * b.f()),
                (b"div", a, b) => V::R(if b.f() == 0.0 { 0.0 } else { a.f() / b.f() }),
                (b"atan", a, b) => {
                    let mut d = rt::x87::atan2(a.f(), b.f()).to_degrees();
                    if d < 0.0 {
                        d += 360.0;
                    }
                    V::R(d)
                }
                (b"exp", a, b) => V::R(pow(a.f(), b.f())),
                _ => V::I(0),
            };
            st.push(r);
        }
        b"and" | b"or" | b"xor" => {
            let b = pop!();
            let a = pop!();
            st.push(match (a, b) {
                (V::B(x), V::B(y)) => V::B(match name {
                    b"and" => x & y,
                    b"or" => x | y,
                    _ => x ^ y,
                }),
                (a, b) => {
                    let (x, y) = (a.f() as i64, b.f() as i64);
                    V::I(match name {
                        b"and" => x & y,
                        b"or" => x | y,
                        _ => x ^ y,
                    })
                }
            });
        }
        b"not" => {
            let a = pop!();
            st.push(match a {
                V::B(b) => V::B(!b),
                v => V::I(!(v.f() as i64)),
            });
        }
        b"bitshift" => {
            let s = pop!().f() as i64;
            let a = pop!().f() as i64;
            // (unsigned_abs: -s overflowed for i64::MIN, a shift of -inf)
            st.push(V::I(if s >= 0 { a.checked_shl(s.min(63) as u32).unwrap_or(0) } else { a >> s.unsigned_abs().min(63) }));
        }
        b"eq" | b"ne" | b"ge" | b"gt" | b"le" | b"lt" => {
            let b = pop!().f();
            let a = pop!().f();
            st.push(V::B(match name {
                b"eq" => a == b,
                b"ne" => a != b,
                b"ge" => a >= b,
                b"gt" => a > b,
                b"le" => a <= b,
                _ => a < b,
            }));
        }
        b"true" => st.push(V::B(true)),
        b"false" => st.push(V::B(false)),
        b"dup" => {
            let v = *st.last().unwrap_or(&V::I(0));
            if st.is_empty() {
                return false;
            }
            st.push(v);
        }
        b"pop" => {
            pop!();
        }
        b"exch" => {
            let b = pop!();
            let a = pop!();
            st.push(b);
            st.push(a);
        }
        b"copy" => {
            let n = pop!().f() as i64;
            if n < 0 || n as usize > st.len() || st.len() + n as usize > PS_STACK {
                return false;
            }
            let from = st.len() - n as usize;
            for i in 0..n as usize {
                let v = st[from + i];
                st.push(v);
            }
        }
        b"index" => {
            let n = pop!().f() as i64;
            if n < 0 || n as usize >= st.len() {
                return false;
            }
            let v = st[st.len() - 1 - n as usize];
            st.push(v);
        }
        b"roll" => {
            let j = pop!().f() as i64;
            let n = pop!().f() as i64;
            if n < 0 || n as usize > st.len() {
                return false;
            }
            if n > 0 {
                let from = st.len() - n as usize;
                let k = j.rem_euclid(n) as usize;
                st[from..].rotate_right(k);
            }
        }
        _ => return false,
    }
    true
}

// ---------------------------------------------------------------- colour spaces

pub(super) enum Cs {
    Gray,
    Rgb,
    Cmyk,
    CalGray { gamma: f64 },
    CalRgb { gamma: [f64; 3], matrix: [f64; 9], wp: [f64; 3] },
    Lab { wp: [f64; 3], range: [f64; 4] },
    Indexed { base: Rc<Cs>, hival: usize, lut: Vec<u8> },
    /// Separation and DeviceN: n tints through the function into the alternate space
    Tint { n: usize, alt: Rc<Cs>, f: Rc<Func>, all: bool, none: bool },
    Pattern { under: Option<Rc<Cs>> },
}

/// the sRGB colours of the 16 combinations of full inks in web offset printing (SWOP; index
/// c | m << 1 | y << 2 | k << 3): paper, the inks and their overprints as the common colour
/// references give them
const INKS: [[f64; 3]; 16] = [
    [1.0, 1.0, 1.0],
    [0.0, 0.6784, 0.9373],
    [0.9255, 0.0, 0.5490],
    [0.1804, 0.1922, 0.5725],
    [1.0, 0.9490, 0.0],
    [0.0, 0.6510, 0.3137],
    [0.9294, 0.1098, 0.1412],
    [0.2118, 0.2119, 0.2235],
    [0.1373, 0.1216, 0.1255],
    [0.0, 0.0588, 0.1412],
    [0.1412, 0.0, 0.0],
    [0.0, 0.0, 0.0078],
    [0.1098, 0.1020, 0.0],
    [0.0, 0.0745, 0.0],
    [0.1333, 0.0, 0.0],
    [0.0, 0.0, 0.0],
];

/// CMYK as RGB: the ink combinations weighted by how much of each ink there is (Demichel's
/// weights, the Neugebauer model of halftone printing)
pub(super) fn cmyk_rgb(c: f64, m: f64, y: f64, k: f64) -> [f64; 3] {
    let v = [clampd(c, 0.0, 1.0), clampd(m, 0.0, 1.0), clampd(y, 0.0, 1.0), clampd(k, 0.0, 1.0)];
    let mut out = [0.0; 3];
    for (i, ink) in INKS.iter().enumerate() {
        let mut w = 1.0;
        for (b, &x) in v.iter().enumerate() {
            w *= if i >> b & 1 == 1 { x } else { 1.0 - x };
        }
        if w != 0.0 {
            for c in 0..3 {
                out[c] += w * ink[c];
            }
        }
    }
    out
}

/// XYZ relative to a white point -> sRGB (Bradford adaptation to D65)
fn xyz_srgb(xyz: [f64; 3], wp: [f64; 3]) -> [f64; 3] {
    const BF: [[f64; 3]; 3] = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];
    const BFI: [[f64; 3]; 3] = [[0.9869929, -0.1470543, 0.1599627], [0.4323053, 0.5183603, 0.0492912], [-0.0085287, 0.0400428, 0.9684867]];
    let d65 = [0.95047, 1.0, 1.08883];
    let mv = |m: &[[f64; 3]; 3], v: [f64; 3]| {
        [m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2], m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2], m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2]]
    };
    let (s, d) = (mv(&BF, wp), mv(&BF, d65));
    let lms = mv(&BF, xyz);
    let adapted = [lms[0] * d[0] / s[0].max(1e-9), lms[1] * d[1] / s[1].max(1e-9), lms[2] * d[2] / s[2].max(1e-9)];
    let x = mv(&BFI, adapted);
    let r = 3.2406 * x[0] - 1.5372 * x[1] - 0.4986 * x[2];
    let g = -0.9689 * x[0] + 1.8758 * x[1] + 0.0415 * x[2];
    let b = 0.0557 * x[0] - 0.2040 * x[1] + 1.0570 * x[2];
    [srgb_gamma(r), srgb_gamma(g), srgb_gamma(b)]
}

fn srgb_gamma(v: f64) -> f64 {
    let v = clampd(v, 0.0, 1.0);
    if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * pow(v, 1.0 / 2.4) - 0.055 }
}

impl Cs {
    pub fn n(&self) -> usize {
        match self {
            Cs::Gray | Cs::CalGray { .. } | Cs::Indexed { .. } | Cs::Pattern { .. } => 1,
            Cs::Rgb | Cs::CalRgb { .. } | Cs::Lab { .. } => 3,
            Cs::Cmyk => 4,
            Cs::Tint { n, .. } => *n,
        }
    }

    /// the initial colour of the space (8.6.5)
    pub fn initial(&self) -> Vec<f64> {
        match self {
            Cs::Cmyk => vec![0.0, 0.0, 0.0, 1.0],
            Cs::Tint { n, .. } => vec![1.0; *n],
            Cs::Lab { range, .. } => vec![0.0, clampd(0.0, range[0], range[1]), clampd(0.0, range[2], range[3])],
            c => vec![0.0; c.n()],
        }
    }

    /// the default Decode of an image's samples (8.9.5.2)
    pub fn decode_default(&self, bpc: u32) -> Vec<f64> {
        match self {
            Cs::Indexed { .. } => vec![0.0, ((1u64 << bpc.min(16)) - 1) as f64],
            Cs::Lab { range, .. } => vec![0.0, 100.0, range[0], range[1], range[2], range[3]],
            c => (0..c.n()).flat_map(|_| [0.0, 1.0]).collect(),
        }
    }

    /// paints nothing (Separation /None)
    pub fn paints_nothing(&self) -> bool {
        matches!(self, Cs::Tint { none: true, .. })
    }

    /// the colour as RGB 0..1
    pub fn rgb(&self, c: &[f64]) -> [f64; 3] {
        let g = |i: usize| c.get(i).copied().unwrap_or(0.0);
        match self {
            Cs::Gray => {
                let v = clampd(g(0), 0.0, 1.0);
                [v, v, v]
            }
            Cs::Rgb => [clampd(g(0), 0.0, 1.0), clampd(g(1), 0.0, 1.0), clampd(g(2), 0.0, 1.0)],
            Cs::Cmyk => cmyk_rgb(g(0), g(1), g(2), g(3)),
            Cs::CalGray { gamma } => {
                let v = srgb_gamma(pow(clampd(g(0), 0.0, 1.0), *gamma));
                [v, v, v]
            }
            Cs::CalRgb { gamma, matrix: m, wp } => {
                let a = pow(clampd(g(0), 0.0, 1.0), gamma[0]);
                let b = pow(clampd(g(1), 0.0, 1.0), gamma[1]);
                let c = pow(clampd(g(2), 0.0, 1.0), gamma[2]);
                let xyz = [m[0] * a + m[3] * b + m[6] * c, m[1] * a + m[4] * b + m[7] * c, m[2] * a + m[5] * b + m[8] * c];
                xyz_srgb(xyz, *wp)
            }
            Cs::Lab { wp, range } => {
                let l = clampd(g(0), 0.0, 100.0);
                let a = clampd(g(1), range[0], range[1]);
                let b = clampd(g(2), range[2], range[3]);
                let m = (l + 16.0) / 116.0;
                let ll = m + a / 500.0;
                let nn = m - b / 200.0;
                let gf = |x: f64| if x >= 6.0 / 29.0 { x * x * x } else { 108.0 / 841.0 * (x - 4.0 / 29.0) };
                let (x, y, z) = (wp[0] * gf(ll), wp[1] * gf(m), wp[2] * gf(nn));
                // XYZ (D65-ish white of the space) to linear sRGB
                let r = 3.2406 * x - 1.5372 * y - 0.4986 * z;
                let gg = -0.9689 * x + 1.8758 * y + 0.0415 * z;
                let bb = 0.0557 * x - 0.2040 * y + 1.0570 * z;
                [srgb_gamma(r), srgb_gamma(gg), srgb_gamma(bb)]
            }
            Cs::Indexed { base, hival, lut } => {
                let i = clampd(floor(g(0) + 0.5), 0.0, *hival as f64) as usize;
                let n = base.n();
                let mut comp = [0f64; 32];
                for k in 0..n.min(32) {
                    comp[k] = lut.get(i * n + k).copied().unwrap_or(0) as f64 / 255.0;
                }
                // a Lab base takes its components from its range
                if let Cs::Lab { range, .. } = &**base {
                    comp[0] *= 100.0;
                    comp[1] = range[0] + comp[1] * (range[1] - range[0]);
                    comp[2] = range[2] + comp[2] * (range[3] - range[2]);
                }
                base.rgb(&comp[..n.min(32)])
            }
            Cs::Tint { n, alt, f, all, none } => {
                if *none {
                    return [1.0, 1.0, 1.0];
                }
                if *all {
                    let v = 1.0 - clampd(g(0), 0.0, 1.0);
                    return [v, v, v];
                }
                let mut out = Vec::new();
                f.eval(&c[..(*n).min(c.len())], &mut out);
                out.resize(alt.n(), 0.0);
                alt.rgb(&out)
            }
            Cs::Pattern { .. } => [0.0; 3],
        }
    }

    pub fn parse(doc: &Document, o: &Object, res: &Dict) -> Option<Rc<Cs>> {
        Cs::parse_at(doc, o, res, 0)
    }

    fn parse_at(doc: &Document, o: &Object, res: &Dict, depth: u32) -> Option<Rc<Cs>> {
        if depth > MAX_DEPTH {
            return None;
        }
        let o = doc.resolve(o);
        let name_of = |n: &[u8]| -> Option<Rc<Cs>> {
            Some(Rc::new(match n {
                b"DeviceGray" | b"G" => Cs::Gray,
                b"DeviceRGB" | b"RGB" => Cs::Rgb,
                b"DeviceCMYK" | b"CMYK" => Cs::Cmyk,
                b"Pattern" => Cs::Pattern { under: None },
                _ => return None,
            }))
        };
        match &o {
            Object::Name(n) => {
                if let Some(c) = name_of(&n.0) {
                    return Some(c);
                }
                // a name of the resources' ColorSpace dictionary (also DefaultGray and the others)
                let csd = doc.dict(res.get("ColorSpace")?);
                let v = csd.get_bytes(&n.0)?.clone();
                Cs::parse_at(doc, &v, res, depth + 1)
            }
            Object::Array(a) => {
                let fam = doc.resolve(a.first()?);
                let fam = fam.as_name()?;
                let arg = |i: usize| a.get(i).map(|v| doc.resolve(v)).unwrap_or(Object::Null);
                match fam {
                    b"DeviceGray" | b"G" | b"DeviceRGB" | b"RGB" | b"DeviceCMYK" | b"CMYK" => name_of(fam),
                    b"CalGray" | b"CalRGB" => {
                        let d = doc.dict(&arg(1));
                        let wp = d.get("WhitePoint").map(|w| nums(doc, w)).unwrap_or_default();
                        let wp = if wp.len() == 3 && wp[1] > 0.0 { [wp[0], wp[1], wp[2]] } else { [0.9505, 1.0, 1.089] };
                        if fam == b"CalGray" {
                            let gamma = doc.key(&d, "Gamma").as_f64().filter(|g| *g > 0.0 && g.is_finite()).unwrap_or(1.0);
                            return Some(Rc::new(Cs::CalGray { gamma }));
                        }
                        let gm = d.get("Gamma").map(|w| nums(doc, w)).unwrap_or_default();
                        let gamma = if gm.len() == 3 && gm.iter().all(|g| *g > 0.0) { [gm[0], gm[1], gm[2]] } else { [1.0; 3] };
                        let mt = d.get("Matrix").map(|w| nums(doc, w)).unwrap_or_default();
                        let matrix = if mt.len() == 9 { [mt[0], mt[1], mt[2], mt[3], mt[4], mt[5], mt[6], mt[7], mt[8]] } else { [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] };
                        Some(Rc::new(Cs::CalRgb { gamma, matrix, wp }))
                    }
                    b"Lab" => {
                        let d = doc.dict(&arg(1));
                        let wp = d.get("WhitePoint").map(|w| nums(doc, w)).unwrap_or_default();
                        let wp = if wp.len() == 3 && wp[1] > 0.0 { [wp[0], wp[1], wp[2]] } else { [0.9505, 1.0, 1.089] };
                        let r = d.get("Range").map(|w| nums(doc, w)).unwrap_or_default();
                        let range = if r.len() == 4 { [r[0], r[1], r[2], r[3]] } else { [-100.0, 100.0, -100.0, 100.0] };
                        Some(Rc::new(Cs::Lab { wp, range }))
                    }
                    b"ICCBased" => {
                        let s = arg(1);
                        let d = match &s {
                            Object::Stream(s) => s.dict.clone(),
                            Object::Dict(d) => d.clone(),
                            _ => Dict::new(),
                        };
                        if let Some(alt) = d.get("Alternate") {
                            if let Some(c) = Cs::parse_at(doc, alt, res, depth + 1) {
                                return Some(c);
                            }
                        }
                        Some(Rc::new(match doc.key(&d, "N").as_int() {
                            Some(1) => Cs::Gray,
                            Some(4) => Cs::Cmyk,
                            _ => Cs::Rgb,
                        }))
                    }
                    b"Indexed" | b"I" => {
                        let base = Cs::parse_at(doc, &arg(1), res, depth + 1)?;
                        let hival = arg(2).as_int()?.clamp(0, 255) as usize;
                        let lut = match arg(3) {
                            Object::String(s) => s,
                            Object::Stream(s) => doc.decode(&s).ok()?,
                            _ => return None,
                        };
                        Some(Rc::new(Cs::Indexed { base, hival, lut }))
                    }
                    b"Separation" | b"DeviceN" => {
                        let names: Vec<Vec<u8>> = match arg(1) {
                            Object::Name(n) => vec![n.0],
                            Object::Array(v) => v.iter().filter_map(|x| doc.resolve(x).as_name().map(<[u8]>::to_vec)).collect(),
                            _ => return None,
                        };
                        let n = names.len().clamp(1, 32);
                        let all = fam == b"Separation" && names.first().is_some_and(|x| x == b"All");
                        let none = names.iter().all(|x| x == b"None");
                        let alt = Cs::parse_at(doc, &arg(2), res, depth + 1).unwrap_or_else(|| Rc::new(Cs::Gray));
                        let f = match Func::parse(doc, &arg(3)) {
                            Some(f) => f,
                            None if all || none => Rc::new(Func::Many(Vec::new())),
                            None => return None,
                        };
                        Some(Rc::new(Cs::Tint { n, alt, f, all, none }))
                    }
                    b"Pattern" => {
                        let under = a.get(1).and_then(|u| Cs::parse_at(doc, u, res, depth + 1));
                        Some(Rc::new(Cs::Pattern { under }))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
}
