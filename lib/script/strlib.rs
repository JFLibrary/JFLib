// strlib -- the str functions of the script library; lengths and indexes count characters
//
//   s.len()  s.slice(start, end)  s.split(sep, n)  sep.join(list)  s.trim(chars)
//   s.trim_start(chars)  s.trim_end(chars)  s.find(sub, from)  s.find_last(sub)
//   s.contains(sub)  s.starts_with(p)  s.ends_with(p)  s.count(sub)  s.replace(from, to, n)
//   s.upper()  s.lower()  s.repeat(n)  s.chars()  s.bytes()  s.code(i)  str.from_code(n, ..)
//   s.format(args..)  s.hash()
//
// Indexes count characters (Unicode scalar values) like len(s) and s[i]; a negative index of
// slice counts from the end, an index outside is clamped.  find and find_last answer nil when
// there is no match.  split without sep splits at runs of white space (and drops it at the
// ends), with "" into characters; n caps the number of parts, the last one keeps the rest.
// format: {} the next argument, {0} one by position, {name} a field of the first argument (a
// map), {{ and }} for the braces themselves; after a colon a spec
// [[fill]align][+][#][0][width][.precision][type]: align < > ^, + a sign also for positive
// numbers, # the prefix 0x 0b 0o, 0 zeros after the sign, width at most 10000, precision at
// most 100 (digits after the point of a number, characters of anything else), type x X b o d
// (ints), f e (numbers; f is the default once a precision is given), s.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::heap::{self, Ref, StrObj, Value};
use crate::num;
use crate::stdlib::{
    arg, bad, cap, err, heap_full, list_fuel, new_bytes, new_list, new_str, opt_int, span, spend, text_fuel, want_int,
    want_list, want_str,
};
use crate::vm::{Exc, Host, Vm};

fn ascii(s: &StrObj) -> bool {
    s.chars as usize == s.s.len()
}

/// the byte offset of character i (i <= chars)
fn byte_at(s: &StrObj, i: usize) -> usize {
    if ascii(s) { i.min(s.s.len()) } else { s.s.char_indices().nth(i).map_or(s.s.len(), |(b, _)| b) }
}

/// the character index of byte offset b
fn char_at(s: &StrObj, b: usize) -> usize {
    if ascii(s) { b } else { s.s[..b].chars().count() }
}

/// fail before the work when the invocation cannot pay `fuel` (spend charges afterwards)
fn afford(vm: &Vm, fuel: u64) -> Result<(), Exc> {
    if vm.call_fuel + vm.used + fuel > vm.lim.fuel {
        return Err(Exc::Fatal(crate::ErrorKind::Fuel, crate::vm::fuel_msg(vm.lim.fuel)));
    }
    Ok(())
}

pub(crate) fn len(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let r = want_str("str.len", a, 0, "s")?;
    Ok(Value::Int(vm.heap.str(r).chars as i64))
}

pub(crate) fn slice(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    const F: &str = "str.slice";
    let r = want_str(F, a, 0, "s")?;
    let o = vm.heap.str(r);
    let (s, e) = span(F, a, 1, o.chars as usize)?;
    if s == 0 && e == o.chars as usize {
        return Ok(a[0]);
    }
    let scan = if ascii(o) { 0 } else { byte_at(o, e) };
    let out = String::from(&o.s[byte_at(o, s)..byte_at(o, e)]);
    spend(vm, text_fuel(scan))?;
    new_str(vm, out)
}

/// the parts at runs of white space (Python's split()), at most n
fn words(s: &str, n: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut it = s.char_indices().peekable();
    loop {
        while it.peek().is_some_and(|(_, c)| c.is_whitespace()) {
            it.next();
        }
        let Some(&(start, _)) = it.peek() else { break };
        if out.len() + 1 == n {
            out.push((start, s.len()));
            break;
        }
        let mut end = s.len();
        while let Some(&(i, c)) = it.peek() {
            if c.is_whitespace() {
                end = i;
                break;
            }
            it.next();
        }
        out.push((start, end));
    }
    out
}

/// the characters, at most n parts
fn letters(s: &str, n: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (i, c) in s.char_indices() {
        if out.len() + 1 == n {
            out.push((i, s.len()));
            break;
        }
        out.push((i, i + c.len_utf8()));
    }
    out
}

pub(crate) fn split(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    const F: &str = "str.split";
    let r = want_str(F, a, 0, "s")?;
    let sep = match arg(a, 1) {
        Value::Nil => None,
        Value::Str(x) => Some(x),
        v => return bad(F, "sep", "str", v),
    };
    let n = match opt_int(F, a, 2, "n")? {
        None => usize::MAX,
        Some(k) if k >= 1 => k as usize,
        Some(k) => return err(format!("{}: n muss mindestens 1 sein, nicht {}", F, k)),
    };
    // a part takes 32 bytes of heap at least: more than this many cannot be stored, and
    // the list of ranges must not grow beyond the limit before new_str notices
    let n = n.min(usize::try_from((vm.lim.heap / 32).saturating_add(1)).unwrap_or(usize::MAX));
    let s = heap::text(&vm.heap, r);
    let ranges = match sep.map(|x| heap::text(&vm.heap, x)) {
        None => words(s, n),
        Some("") => letters(s, n),
        Some(p) => {
            let mut out = Vec::new();
            let mut start = 0;
            for (i, m) in s.match_indices(p) {
                if out.len() + 1 == n {
                    break;
                }
                out.push((start, i));
                start = i + m.len();
            }
            out.push((start, s.len()));
            out
        }
    };
    let fuel = text_fuel(s.len());
    spend(vm, fuel)?;
    let mut items = Vec::with_capacity(ranges.len());
    for (b, e) in ranges {
        let part = String::from(&heap::text(&vm.heap, r)[b..e]);
        items.push(new_str(vm, part)?);
    }
    new_list(vm, items)
}

/// the texts of the items of l (str as they are, other values as print shows them) with sep
/// between them
pub(crate) fn join_list(vm: &mut Vm, l: Ref, sep: Option<Ref>) -> Result<Value, Exc> {
    let n = vm.heap.list(l).len();
    let mut out = String::new();
    for i in 0..n {
        if let (true, Some(sp)) = (i > 0, sep) {
            out.push_str(heap::text(&vm.heap, sp));
        }
        match vm.heap.list(l)[i] {
            Value::Str(r) => out.push_str(heap::text(&vm.heap, r)),
            v => vm.write_value(&mut out, v, false, 0, &mut Vec::new()),
        }
        cap(vm, out.len() as u64)?;
    }
    spend(vm, list_fuel(n))?;
    new_str(vm, out)
}

/// sep.join(list)
pub(crate) fn join(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let sep = want_str("str.join", a, 0, "sep")?;
    let l = want_list("str.join", a, 1, "list")?;
    join_list(vm, l, Some(sep))
}

/// trim at the start and / or the end: white space, or the characters of `chars`
fn trim_with(vm: &mut Vm, a: &[Value], f: &str, start: bool, end: bool) -> Result<Value, Exc> {
    let r = want_str(f, a, 0, "s")?;
    let set: Option<Vec<char>> = match arg(a, 1) {
        Value::Nil => None,
        Value::Str(c) => Some(heap::text(&vm.heap, c).chars().collect()),
        v => return bad(f, "chars", "str", v),
    };
    let s = heap::text(&vm.heap, r);
    let cut = |c: char| match &set {
        None => c.is_whitespace(),
        Some(v) => v.contains(&c),
    };
    let t = match (start, end) {
        (true, true) => s.trim_matches(cut),
        (true, false) => s.trim_start_matches(cut),
        _ => s.trim_end_matches(cut),
    };
    if t.len() == s.len() {
        return Ok(a[0]);
    }
    let out = String::from(t);
    spend(vm, text_fuel(s.len() - out.len()))?;
    new_str(vm, out)
}

pub(crate) fn trim(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    trim_with(vm, a, "str.trim", true, true)
}

pub(crate) fn trim_start(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    trim_with(vm, a, "str.trim_start", true, false)
}

pub(crate) fn trim_end(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    trim_with(vm, a, "str.trim_end", false, true)
}

/// the character index of the first match at or after from, or nil
pub(crate) fn find(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    const F: &str = "str.find";
    let r = want_str(F, a, 0, "s")?;
    let p = want_str(F, a, 1, "sub")?;
    let from = opt_int(F, a, 2, "from")?.unwrap_or(0);
    let o = vm.heap.str(r);
    let n = o.chars as i64;
    let from = (if from < 0 { (n + from).max(0) } else { from.min(n) }) as usize;
    let bs = byte_at(o, from);
    let sub = heap::text(&vm.heap, p);
    let (res, scanned) = match o.s[bs..].find(sub) {
        Some(b) => (Value::Int((from + o.s[bs..bs + b].chars().count()) as i64), bs + b + sub.len()),
        None => (Value::Nil, o.s.len()),
    };
    spend(vm, text_fuel(scanned))?;
    Ok(res)
}

/// the character index of the last match, or nil
pub(crate) fn find_last(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let r = want_str("str.find_last", a, 0, "s")?;
    let p = want_str("str.find_last", a, 1, "sub")?;
    let o = vm.heap.str(r);
    let res = match o.s.rfind(heap::text(&vm.heap, p)) {
        Some(b) => Value::Int(char_at(o, b) as i64),
        None => Value::Nil,
    };
    let fuel = text_fuel(o.s.len());
    spend(vm, fuel)?;
    Ok(res)
}

/// a test of s against a second str (contains, starts_with, ends_with)
fn test(vm: &mut Vm, a: &[Value], f: &str, p: &str, op: fn(&str, &str) -> bool) -> Result<Value, Exc> {
    let r = want_str(f, a, 0, "s")?;
    let q = want_str(f, a, 1, p)?;
    let s = heap::text(&vm.heap, r);
    let yes = op(s, heap::text(&vm.heap, q));
    let fuel = text_fuel(s.len());
    spend(vm, fuel)?;
    Ok(Value::Bool(yes))
}

pub(crate) fn contains(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    test(vm, a, "str.contains", "sub", |s, p| s.contains(p))
}

pub(crate) fn starts_with(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    test(vm, a, "str.starts_with", "p", |s, p| s.starts_with(p))
}

pub(crate) fn ends_with(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    test(vm, a, "str.ends_with", "p", |s, p| s.ends_with(p))
}

/// the matches of sub that do not overlap ("" matches between all characters)
pub(crate) fn count(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let r = want_str("str.count", a, 0, "s")?;
    let p = want_str("str.count", a, 1, "sub")?;
    let s = heap::text(&vm.heap, r);
    let k = s.matches(heap::text(&vm.heap, p)).count();
    let fuel = text_fuel(s.len());
    spend(vm, fuel)?;
    Ok(Value::Int(k as i64))
}

/// every match of from (or the first n) replaced by to
pub(crate) fn replace(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    const F: &str = "str.replace";
    let r = want_str(F, a, 0, "s")?;
    let fr = want_str(F, a, 1, "from")?;
    let to = want_str(F, a, 2, "to")?;
    let limit = match opt_int(F, a, 3, "n")? {
        None => usize::MAX,
        Some(k) if k >= 0 => k as usize,
        Some(k) => return err(format!("{}: n muss mindestens 0 sein, nicht {}", F, k)),
    };
    let (s, f, t) = (heap::text(&vm.heap, r), heap::text(&vm.heap, fr), heap::text(&vm.heap, to));
    let k = s.matches(f).take(limit).count();
    if k == 0 {
        let fuel = text_fuel(s.len());
        spend(vm, fuel)?;
        return Ok(a[0]);
    }
    let len = (s.len() - k * f.len()) as u64 + (k as u64).saturating_mul(t.len() as u64);
    cap(vm, len)?;
    let fuel = 1 + text_fuel(s.len()) + len / 64;
    afford(vm, fuel)?;
    let out = if limit == usize::MAX { s.replace(f, t) } else { s.replacen(f, t, limit) };
    spend(vm, fuel)?;
    vm.alloc_str(out)
}

/// upper or lower case (full Unicode mapping: "ß".upper() is "SS")
fn case(vm: &mut Vm, a: &[Value], f: &str, upper: bool) -> Result<Value, Exc> {
    let r = want_str(f, a, 0, "s")?;
    let s = heap::text(&vm.heap, r);
    let fuel = text_fuel(2 * s.len());
    afford(vm, fuel)?;
    let out = if upper { s.to_uppercase() } else { s.to_lowercase() };
    let same = out == s;
    spend(vm, fuel)?;
    if same {
        return Ok(a[0]);
    }
    cap(vm, out.len() as u64)?;
    new_str(vm, out)
}

pub(crate) fn upper(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    case(vm, a, "str.upper", true)
}

pub(crate) fn lower(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    case(vm, a, "str.lower", false)
}

/// s n times
pub(crate) fn repeat(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    const F: &str = "str.repeat";
    let r = want_str(F, a, 0, "s")?;
    let k = want_int(F, a, 1, "n")?;
    if k < 0 {
        return err(format!("{}: n muss mindestens 0 sein, nicht {}", F, k));
    }
    let s = heap::text(&vm.heap, r);
    let Some(len) = (s.len() as u64).checked_mul(k as u64) else { return Err(heap_full(vm)) };
    if len == 0 {
        return new_str(vm, String::new());
    }
    cap(vm, len)?;
    let fuel = 1 + len / 64;
    afford(vm, fuel)?;
    let out = s.repeat(k as usize);
    spend(vm, fuel)?;
    vm.alloc_str(out)
}

/// a list of one-character strings
pub(crate) fn chars(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let r = want_str("str.chars", a, 0, "s")?;
    // every character becomes a string of 33 bytes of heap: check before the buffers are built
    cap(vm, (vm.heap.str(r).chars as u64).saturating_mul(33))?;
    let cs: Vec<char> = heap::text(&vm.heap, r).chars().collect();
    let mut items = Vec::with_capacity(cs.len());
    for c in cs {
        let mut t = String::new();
        t.push(c);
        items.push(new_str(vm, t)?);
    }
    new_list(vm, items)
}

/// the UTF-8 bytes
pub(crate) fn bytes(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let r = want_str("str.bytes", a, 0, "s")?;
    let b = heap::text(&vm.heap, r).as_bytes().to_vec();
    new_bytes(vm, b)
}

/// the code point of character i (default 0)
pub(crate) fn code(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    const F: &str = "str.code";
    let r = want_str(F, a, 0, "s")?;
    let i = opt_int(F, a, 1, "i")?.unwrap_or(0);
    let o = vm.heap.str(r);
    let (c, n, scan) = (heap::char_at(o, i), o.chars, if ascii(o) { 0 } else { i.max(0) as u64 });
    spend(vm, scan / 64)?;
    match c {
        Some(c) => Ok(Value::Int(c as i64)),
        None => err(format!("{}: Index {} liegt außerhalb des Textes (Länge {})", F, i, n)),
    }
}

/// the text of these code points
pub(crate) fn from_code(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    const F: &str = "str.from_code";
    let mut s = String::new();
    for v in a {
        match *v {
            Value::Int(i) => match u32::try_from(i).ok().and_then(char::from_u32) {
                Some(c) => s.push(c),
                None => return err(format!("{}: {} ist kein gültiges Zeichen", F, i)),
            },
            v => return bad(F, "n", "int", v),
        }
    }
    spend(vm, list_fuel(a.len()))?;
    new_str(vm, s)
}

/// FNV-1a (32 bits) of the UTF-8 bytes, as an int from 0 to 2^32 - 1
pub(crate) fn hash(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let r = want_str("str.hash", a, 0, "s")?;
    Ok(Value::Int(vm.heap.str(r).hash as i64))
}

// ---- format

const MAX_WIDTH: usize = 10_000;
const MAX_PREC: usize = 100;

/// the part after the colon of {..:spec}
struct Spec {
    fill: char,
    fill_given: bool,
    align: Option<char>,
    plus: bool,
    alt: bool,
    zero: bool,
    width: usize,
    prec: Option<usize>,
    ty: Option<char>,
}

impl Spec {
    fn parse(s: &str) -> Option<Spec> {
        let cs: Vec<char> = s.chars().collect();
        let mut sp = Spec { fill: ' ', fill_given: false, align: None, plus: false, alt: false, zero: false, width: 0, prec: None, ty: None };
        let is_align = |c: char| matches!(c, '<' | '>' | '^');
        let mut i = 0;
        if cs.len() >= 2 && is_align(cs[1]) {
            (sp.fill, sp.fill_given, sp.align, i) = (cs[0], true, Some(cs[1]), 2);
        } else if cs.first().is_some_and(|c| is_align(*c)) {
            (sp.align, i) = (Some(cs[0]), 1);
        }
        if cs.get(i) == Some(&'+') {
            (sp.plus, i) = (true, i + 1);
        }
        if cs.get(i) == Some(&'#') {
            (sp.alt, i) = (true, i + 1);
        }
        if cs.get(i) == Some(&'0') {
            (sp.zero, i) = (true, i + 1);
        }
        let number = |i: &mut usize| -> Option<usize> {
            let start = *i;
            let mut v: usize = 0;
            while let Some(d) = cs.get(*i).and_then(|c| c.to_digit(10)) {
                v = v.saturating_mul(10).saturating_add(d as usize);
                *i += 1;
            }
            (*i > start).then_some(v)
        };
        if let Some(w) = number(&mut i) {
            if w > MAX_WIDTH {
                return None;
            }
            sp.width = w;
        }
        if cs.get(i) == Some(&'.') {
            i += 1;
            let p = number(&mut i)?;
            if p > MAX_PREC {
                return None;
            }
            sp.prec = Some(p);
        }
        if let Some(&c) = cs.get(i) {
            if matches!(c, 'x' | 'X' | 'b' | 'o' | 'd' | 'f' | 'e' | 's') {
                (sp.ty, i) = (Some(c), i + 1);
            }
        }
        (i == cs.len()).then_some(sp)
    }
}

/// a number without its sign: (negative, prefix, digits)
fn render(vm: &Vm, v: Value, sp: &Spec, spec: &str) -> Result<(bool, &'static str, String), Exc> {
    let numeric = matches!(v, Value::Int(_) | Value::Float(_));
    if let Some(t @ ('x' | 'X' | 'b' | 'o' | 'd')) = sp.ty {
        let Value::Int(i) = v else { return err(format!("str.format: {{:{}}} braucht int, nicht {}", spec, v.type_name())) };
        let m = i.unsigned_abs();
        let body = match t {
            'x' => format!("{:x}", m),
            'X' => format!("{:X}", m),
            'b' => format!("{:b}", m),
            'o' => format!("{:o}", m),
            _ => format!("{}", m),
        };
        let prefix = match (sp.alt, t) {
            (true, 'x') => "0x",
            (true, 'X') => "0X",
            (true, 'b') => "0b",
            (true, 'o') => "0o",
            _ => "",
        };
        return Ok((i < 0, prefix, body));
    }
    let fixed = matches!(sp.ty, Some('f' | 'e')) || (sp.ty.is_none() && sp.prec.is_some() && numeric);
    if fixed {
        return match v {
            Value::Int(i) if sp.ty != Some('e') => {
                let mut body = format!("{}", i.unsigned_abs());
                let p = sp.prec.unwrap_or(6);
                if p > 0 {
                    body.push('.');
                    body.extend(core::iter::repeat_n('0', p));
                }
                Ok((i < 0, "", body))
            }
            Value::Int(_) | Value::Float(_) => {
                let x = if let Value::Int(i) = v { num::from_i64(i) } else if let Value::Float(x) = v { x } else { num::NAN };
                let m = num::abs(x);
                let body = if num::is_nan(x) {
                    String::from("nan")
                } else if m.to_bits() == 0x7FF0_0000_0000_0000 {
                    String::from("inf")
                } else {
                    match (sp.ty, sp.prec) {
                        (Some('e'), Some(p)) => format!("{:.*e}", p, m),
                        (Some('e'), None) => format!("{:e}", m),
                        (_, p) => format!("{:.*}", p.unwrap_or(6), m),
                    }
                };
                Ok((x.to_bits() >> 63 != 0 && !num::is_nan(x), "", body))
            }
            v => err(format!("str.format: {{:{}}} braucht eine Zahl, nicht {}", spec, v.type_name())),
        };
    }
    let mut body = String::new();
    let neg = match v {
        Value::Str(r) => {
            body.push_str(heap::text(&vm.heap, r));
            false
        }
        Value::Int(i) => {
            body.push_str(&format!("{}", i.unsigned_abs()));
            i < 0
        }
        Value::Float(x) => {
            num::fmt(num::abs(x), &mut body);
            x.to_bits() >> 63 != 0 && !num::is_nan(x)
        }
        v => {
            vm.write_value(&mut body, v, false, 0, &mut Vec::new());
            false
        }
    };
    if let (Some(p), false) = (sp.prec, numeric) {
        if let Some((cut, _)) = body.char_indices().nth(p) {
            body.truncate(cut);
        }
    }
    Ok((neg, "", body))
}

/// the text of one placeholder
fn put(vm: &Vm, out: &mut String, v: Value, spec: &str) -> Result<(), Exc> {
    if spec.is_empty() {
        match v {
            Value::Str(r) => out.push_str(heap::text(&vm.heap, r)),
            v => vm.write_value(out, v, false, 0, &mut Vec::new()),
        }
        return Ok(());
    }
    let Some(sp) = Spec::parse(spec) else { return err(format!("str.format: Ungültige Formatangabe {{:{}}}", spec)) };
    let numeric = matches!(v, Value::Int(_) | Value::Float(_));
    let (neg, prefix, body) = render(vm, v, &sp, spec)?;
    let sign = if neg { "-" } else if sp.plus && numeric { "+" } else { "" };
    let len = sign.len() + prefix.len() + body.chars().count();
    let pad = sp.width.saturating_sub(len);
    if sp.zero && numeric && sp.align.is_none() {
        out.push_str(sign);
        out.push_str(prefix);
        out.extend(core::iter::repeat_n('0', pad));
        out.push_str(&body);
        return Ok(());
    }
    let align = sp.align.unwrap_or(if numeric { '>' } else { '<' });
    let (left, right) = match align {
        '<' => (0, pad),
        '>' => (pad, 0),
        _ => (pad / 2, pad - pad / 2),
    };
    let fill = if sp.zero && !sp.fill_given { '0' } else { sp.fill };
    out.extend(core::iter::repeat_n(fill, left));
    out.push_str(sign);
    out.push_str(prefix);
    out.push_str(&body);
    out.extend(core::iter::repeat_n(fill, right));
    Ok(())
}

/// fmt with its placeholders filled in (see the module comment)
pub(crate) fn format(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    const F: &str = "str.format";
    let r = want_str(F, a, 0, "fmt")?;
    let args = &a[1..];
    let mut out = String::new();
    let fmt = heap::text(&vm.heap, r);
    let b = fmt.as_bytes();
    let (mut i, mut next) = (0, 0);
    while i < b.len() {
        match b[i] {
            b'{' if b.get(i + 1) == Some(&b'{') => {
                out.push('{');
                i += 2;
            }
            b'}' if b.get(i + 1) == Some(&b'}') => {
                out.push('}');
                i += 2;
            }
            b'{' => {
                let Some(len) = fmt[i + 1..].find('}') else { return err(format!("{}: Nach {{ fehlt die schließende }}", F)) };
                let body = &fmt[i + 1..i + 1 + len];
                i += len + 2;
                let (field, spec) = body.split_once(':').unwrap_or((body, ""));
                let v = if field.is_empty() {
                    next += 1;
                    match args.get(next - 1) {
                        Some(v) => *v,
                        None => return err(format!("{}: Es gibt kein Argument {}", F, next - 1)),
                    }
                } else if let Ok(k) = field.parse::<usize>() {
                    match args.get(k) {
                        Some(v) => *v,
                        None => return err(format!("{}: Es gibt kein Argument {}", F, k)),
                    }
                } else {
                    match args.first() {
                        Some(m @ Value::Map(_)) => match vm.map_get_str(*m, field) {
                            Some(v) => v,
                            None => return err(format!("{}: Die Map hat kein Feld {}", F, field)),
                        },
                        v => {
                            let t = v.map_or("nichts", |v| v.type_name());
                            return err(format!("{}: Für {{{}}} muss das erste Argument eine Map sein, nicht {}", F, field, t));
                        }
                    }
                };
                put(vm, &mut out, v, spec)?;
            }
            b'}' => return err(format!("{}: Eine }} ohne öffnende {{", F)),
            _ => {
                let j = fmt[i..].find(['{', '}']).map_or(b.len(), |k| i + k);
                out.push_str(&fmt[i..j]);
                i = j;
            }
        }
        cap(vm, out.len() as u64)?;
    }
    let fuel = text_fuel(fmt.len());
    spend(vm, fuel)?;
    new_str(vm, out)
}
