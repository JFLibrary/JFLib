// stdlib -- the standard library of the scripts: what every host gets, under the names and
// with the conventions of host functions (Host::call), looked up before the host is asked
//
//   lookup(type, name)  the function type.name.  Methods of str, list, map and bytes get the
//                       value as args[0], so s.split(",") and str.split(s, ",") are the same
//                       call; math, json, int and float hold module functions; find("bytes")
//                       is the constructor
//   constant(name)      math.pi math.e math.tau math.inf math.nan int.max int.min
//   NAMES, CONSTANTS    every name ("str.split"), for script::library()
//   spend / cap         fuel by the work; a text that is built must stay below the heap limit
//
// The functions are in strlib.rs, listlib.rs, jsonlib.rs and here (int, float, bytes, map,
// math).  Fuel, on top of the 1 of the call: 1 per new object (str, bytes, list, map), 1 per
// 64 bytes of text read or written, 1 per 8 list values and 1 per 4 map entries copied or
// visited, 1 per comparison of a sort, 2 for sqrt, 4 for exp log log2 log10 sin cos, 8 for
// pow, plus what callbacks cost.  Long operations pay as they go (spend), so the fuel limit
// of the invocation stops them in the middle.  Results are allocated through the VM, so the
// heap limit holds for them, and a text whose size is known in advance is checked before it
// is built.  Wrong arguments throw a catchable str ("str.split: sep muss str sein, nicht
// int"); the limits end the invocation.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::heap::{self, Map, Obj, Ref, Value};
use crate::vm::{ErrorKind, Exc, Fail, Host, Vm};
use crate::{jsonlib, listlib, math, num, strlib};

/// a library function: the VM, the host (for callbacks), the arguments (the object first)
pub(crate) type LibFn = fn(&mut Vm, &mut dyn Host, &[Value]) -> Result<Value, Exc>;

macro_rules! table {
    ($($ty:literal { $($name:literal => $f:path),* $(,)? })*) => {
        /// the library function ty.name
        pub(crate) fn lookup(ty: &str, name: &str) -> Option<LibFn> {
            match ty {
                $($ty => match name {
                    $($name => Some($f as LibFn),)*
                    _ => None,
                },)*
                _ => None,
            }
        }

        /// every library function as "type.name"
        pub(crate) const NAMES: &[&str] = &[$($(concat!($ty, ".", $name),)*)*];
    };
}

table! {
    "str" {
        "len" => strlib::len, "slice" => strlib::slice, "split" => strlib::split, "join" => strlib::join,
        "trim" => strlib::trim, "trim_start" => strlib::trim_start, "trim_end" => strlib::trim_end,
        "find" => strlib::find, "find_last" => strlib::find_last, "contains" => strlib::contains,
        "starts_with" => strlib::starts_with, "ends_with" => strlib::ends_with, "count" => strlib::count,
        "replace" => strlib::replace, "upper" => strlib::upper, "lower" => strlib::lower,
        "repeat" => strlib::repeat, "chars" => strlib::chars, "bytes" => strlib::bytes, "code" => strlib::code,
        "from_code" => strlib::from_code, "format" => strlib::format, "hash" => strlib::hash,
    }
    "list" {
        "len" => listlib::len, "push" => listlib::push, "pop" => listlib::pop, "insert" => listlib::insert,
        "remove" => listlib::remove, "slice" => listlib::slice, "reverse" => listlib::reverse,
        "sort" => listlib::sort, "map" => listlib::map, "filter" => listlib::filter, "reduce" => listlib::reduce,
        "contains" => listlib::contains, "index" => listlib::index, "join" => listlib::join,
    }
    "map" {
        "len" => map_len, "keys" => map_keys, "values" => map_values, "entries" => map_entries,
        "has" => map_has, "get" => map_get, "delete" => map_delete, "merge" => map_merge,
    }
    "bytes" {
        "len" => bytes_len, "slice" => bytes_slice, "hex" => bytes_hex, "from_hex" => bytes_from_hex,
        "text" => bytes_text, "hash" => bytes_hash,
    }
    "math" {
        "abs" => math_abs, "min" => math_min, "max" => math_max, "clamp" => math_clamp,
        "floor" => math_floor, "ceil" => math_ceil, "round" => math_round, "trunc" => math_trunc,
        "sqrt" => math_sqrt, "pow" => math_pow, "exp" => math_exp, "log" => math_log, "log2" => math_log2,
        "log10" => math_log10, "sin" => math_sin, "cos" => math_cos, "is_nan" => math_is_nan,
        "is_finite" => math_is_finite,
    }
    "json" {
        "encode" => jsonlib::encode_fn, "decode" => jsonlib::decode_fn,
    }
    "int" {
        "parse" => int_parse,
    }
    "float" {
        "parse" => float_parse,
    }
}

/// the library function of a host name ("str.split", "math.sqrt"; "bytes" alone)
pub(crate) fn find(name: &str) -> Option<LibFn> {
    if name == "bytes" {
        return Some(bytes_new);
    }
    let (ty, f) = name.split_once('.')?;
    lookup(ty, f)
}

/// the constants of the library
pub(crate) const CONSTANTS: &[&str] = &["math.pi", "math.e", "math.tau", "math.inf", "math.nan", "int.max", "int.min"];

/// the value of a constant ("math.pi")
pub(crate) fn constant(name: &str) -> Option<Value> {
    Some(match name {
        "math.pi" => Value::Float(core::f64::consts::PI),
        "math.e" => Value::Float(core::f64::consts::E),
        "math.tau" => Value::Float(core::f64::consts::TAU),
        "math.inf" => Value::Float(f64::INFINITY),
        "math.nan" => Value::Float(num::NAN),
        "int.max" => Value::Int(i64::MAX),
        "int.min" => Value::Int(i64::MIN),
        _ => return None,
    })
}

// ---- arguments

pub(crate) fn arg(a: &[Value], i: usize) -> Value {
    a.get(i).copied().unwrap_or(Value::Nil)
}

pub(crate) fn err<T>(m: String) -> Result<T, Exc> {
    Err(Exc::Msg(m))
}

/// "f: p muss want sein, nicht <type>"
pub(crate) fn bad<T>(f: &str, p: &str, want: &str, got: Value) -> Result<T, Exc> {
    err(format!("{}: {} muss {} sein, nicht {}", f, p, want, got.type_name()))
}

pub(crate) fn want_int(f: &str, a: &[Value], i: usize, p: &str) -> Result<i64, Exc> {
    match arg(a, i) {
        Value::Int(n) => Ok(n),
        v => bad(f, p, "int", v),
    }
}

/// an optional int (nil: None)
pub(crate) fn opt_int(f: &str, a: &[Value], i: usize, p: &str) -> Result<Option<i64>, Exc> {
    match arg(a, i) {
        Value::Nil => Ok(None),
        Value::Int(n) => Ok(Some(n)),
        v => bad(f, p, "int", v),
    }
}

pub(crate) fn want_str(f: &str, a: &[Value], i: usize, p: &str) -> Result<Ref, Exc> {
    match arg(a, i) {
        Value::Str(r) => Ok(r),
        v => bad(f, p, "str", v),
    }
}

pub(crate) fn want_list(f: &str, a: &[Value], i: usize, p: &str) -> Result<Ref, Exc> {
    match arg(a, i) {
        Value::List(r) => Ok(r),
        v => bad(f, p, "list", v),
    }
}

pub(crate) fn want_map(f: &str, a: &[Value], i: usize, p: &str) -> Result<Ref, Exc> {
    match arg(a, i) {
        Value::Map(r) => Ok(r),
        v => bad(f, p, "map", v),
    }
}

pub(crate) fn want_bytes(f: &str, a: &[Value], i: usize, p: &str) -> Result<Ref, Exc> {
    match arg(a, i) {
        Value::Bytes(r) => Ok(r),
        v => bad(f, p, "bytes", v),
    }
}

pub(crate) fn want_fn(f: &str, a: &[Value], i: usize, p: &str) -> Result<Value, Exc> {
    match arg(a, i) {
        v @ (Value::Func(_) | Value::Native(_)) => Ok(v),
        v => err(format!("{}: {} muss eine Funktion sein, nicht {}", f, p, v.type_name())),
    }
}

/// a number as a float
pub(crate) fn want_num(f: &str, a: &[Value], i: usize, p: &str) -> Result<f64, Exc> {
    match arg(a, i) {
        Value::Int(n) => Ok(num::from_i64(n)),
        Value::Float(x) => Ok(x),
        v => bad(f, p, "eine Zahl", v),
    }
}

/// start..end for slice(start, end) over len items: a negative index counts from the end,
/// nil is the start / the end, indexes outside are clamped, end before start is empty
pub(crate) fn span(f: &str, a: &[Value], i: usize, len: usize) -> Result<(usize, usize), Exc> {
    let n = len as i64;
    let at = |v: Value, p: &str, dflt: i64| -> Result<i64, Exc> {
        match v {
            Value::Nil => Ok(dflt),
            Value::Int(k) => Ok(if k < 0 { (n + k).max(0) } else { k.min(n) }),
            v => bad(f, p, "int", v),
        }
    };
    let s = at(arg(a, i), "start", 0)?;
    let e = at(arg(a, i + 1), "end", n)?;
    Ok((s as usize, e.max(s) as usize))
}

/// the message of a value that cannot be a map key
pub(crate) fn key_error<T>(f: &str, k: Value) -> Result<T, Exc> {
    err(format!("{}: Ein Map-Schlüssel muss str, int oder bool sein, nicht {}", f, k.type_name()))
}

// ---- fuel, heap, callbacks

pub(crate) fn text_fuel(bytes: usize) -> u64 {
    bytes as u64 / 64
}

pub(crate) fn list_fuel(items: usize) -> u64 {
    items as u64 / 8
}

pub(crate) fn map_fuel(entries: usize) -> u64 {
    entries as u64 / 4
}

/// charge fuel; the fuel error of the invocation once Limits::fuel is used up
pub(crate) fn spend(vm: &mut Vm, fuel: u64) -> Result<(), Exc> {
    vm.used += fuel;
    if vm.call_fuel + vm.used > vm.lim.fuel {
        return Err(Exc::Fatal(ErrorKind::Fuel, crate::vm::fuel_msg(vm.lim.fuel)));
    }
    Ok(())
}

pub(crate) fn heap_full(vm: &Vm) -> Exc {
    Exc::Fatal(ErrorKind::Heap, format!("Speichergrenze erreicht ({} Bytes)", vm.lim.heap))
}

/// a text of len bytes that is being built stays below the heap limit
pub(crate) fn cap(vm: &Vm, len: u64) -> Result<(), Exc> {
    if len.saturating_add(32) > vm.lim.heap { Err(heap_full(vm)) } else { Ok(()) }
}

/// keep v alive until the native returns (values held while calling back)
pub(crate) fn keep(vm: &mut Vm, v: Value) {
    if let Some(r) = v.heap_ref() {
        vm.nroots.push(r);
    }
}

/// call a function value back to its end
pub(crate) fn call(vm: &mut Vm, host: &mut dyn Host, f: Value, args: &[Value]) -> Result<Value, Exc> {
    match vm.nested(host, f, args) {
        Ok(v) => Ok(v),
        Err(Fail::Throw(v)) => Err(Exc::Throw(v)),
        Err(Fail::Fatal(m)) => Err(Exc::Fatal(vm.fatal_kind.take().unwrap_or(ErrorKind::Host), m)),
        Err(Fail::Unknown) => err(String::from("Unbekannte Funktion")),
    }
}

// ---- new objects (1 fuel each, plus copying)

pub(crate) fn new_str(vm: &mut Vm, s: String) -> Result<Value, Exc> {
    spend(vm, 1 + text_fuel(s.len()))?;
    vm.alloc_str(s)
}

pub(crate) fn new_list(vm: &mut Vm, items: Vec<Value>) -> Result<Value, Exc> {
    spend(vm, 1 + list_fuel(items.len()))?;
    vm.alloc(Obj::List(items)).map(Value::List)
}

pub(crate) fn new_map(vm: &mut Vm, m: Map) -> Result<Value, Exc> {
    spend(vm, 1 + map_fuel(m.entries.len()))?;
    vm.alloc(Obj::Map(Box::new(m))).map(Value::Map)
}

pub(crate) fn new_bytes(vm: &mut Vm, b: Vec<u8>) -> Result<Value, Exc> {
    spend(vm, 1 + text_fuel(b.len()))?;
    vm.alloc(Obj::Bytes(b.into_boxed_slice())).map(Value::Bytes)
}

// ---- int, float

/// an integer in text: optional spaces around, a sign, 0x / 0b / 0o (when base is None or
/// matches), digits of the base; like literals, other bases than 10 take up to 64 bits in
/// two's complement (0xFFFF_FFFF_FFFF_FFFF is -1); None when it is no integer or too big
pub(crate) fn parse_int(s: &str, base: Option<u32>) -> Option<i64> {
    let s = s.trim();
    let (neg, s) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let prefix = match s.as_bytes() {
        [b'0', b'x' | b'X', ..] => Some(16),
        [b'0', b'b' | b'B', ..] => Some(2),
        [b'0', b'o' | b'O', ..] => Some(8),
        _ => None,
    };
    let (base, digits) = match (base, prefix) {
        (None, Some(p)) => (p, &s[2..]),
        (Some(b), Some(p)) if b == p => (p, &s[2..]),
        (None, None) => (10, s),
        (Some(b), _) => (b, s),
    };
    if digits.is_empty() {
        return None;
    }
    let mut v: u64 = 0;
    for c in digits.chars() {
        let d = c.to_digit(base)? as u64;
        v = v.checked_mul(base as u64)?.checked_add(d)?;
    }
    if neg {
        return if v <= 1 << 63 { Some((v as i64).wrapping_neg()) } else { None };
    }
    if base == 10 && v > i64::MAX as u64 {
        return None;
    }
    Some(v as i64)
}

fn int_parse(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    const F: &str = "int.parse";
    let r = want_str(F, a, 0, "s")?;
    let base = match opt_int(F, a, 1, "base")? {
        None => None,
        Some(b @ 2..=36) => Some(b as u32),
        Some(b) => return err(format!("{}: base muss zwischen 2 und 36 liegen, nicht {}", F, b)),
    };
    let s = heap::text(&vm.heap, r);
    let v = parse_int(s, base).map_or(Value::Nil, Value::Int);
    let fuel = text_fuel(s.len());
    spend(vm, fuel)?;
    Ok(v)
}

/// a float in text (also inf, -inf, nan, 1e5, .5); None when it is none
pub(crate) fn parse_float(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().map(|f| if num::is_nan(f) { num::NAN } else { f })
}

fn float_parse(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let r = want_str("float.parse", a, 0, "s")?;
    let s = heap::text(&vm.heap, r);
    let v = parse_float(s).map_or(Value::Nil, Value::Float);
    let fuel = text_fuel(s.len());
    spend(vm, fuel)?;
    Ok(v)
}

// ---- bytes

fn byte_of(v: Value) -> Result<u8, Exc> {
    match v {
        Value::Int(i @ 0..=255) => Ok(i as u8),
        Value::Int(i) => err(format!("bytes: {} ist kein Byte (0 bis 255)", i)),
        v => err(format!("bytes: Ein Byte muss int sein, nicht {}", v.type_name())),
    }
}

/// bytes(), bytes(str) (UTF-8), bytes(list of ints), bytes(b), bytes(int, int, ..)
fn bytes_new(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let data: Vec<u8> = match a {
        [] => Vec::new(),
        [Value::Bytes(_)] => return Ok(a[0]),
        [Value::Str(r)] => heap::text(&vm.heap, *r).as_bytes().to_vec(),
        [Value::List(r)] => {
            let n = vm.heap.list(*r).len();
            spend(vm, list_fuel(n))?;
            vm.heap.list(*r).iter().map(|v| byte_of(*v)).collect::<Result<_, _>>()?
        }
        [v] if !matches!(v, Value::Int(_)) => {
            return err(format!("bytes braucht str, list, bytes oder Bytes als int, nicht {}", v.type_name()));
        }
        _ => a.iter().map(|v| byte_of(*v)).collect::<Result<_, _>>()?,
    };
    new_bytes(vm, data)
}

fn bytes_len(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let b = want_bytes("bytes.len", a, 0, "b")?;
    Ok(Value::Int(vm.heap.bytes_of(b).len() as i64))
}

fn bytes_slice(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let b = want_bytes("bytes.slice", a, 0, "b")?;
    let all = vm.heap.bytes_of(b);
    let (s, e) = span("bytes.slice", a, 1, all.len())?;
    if s == 0 && e == all.len() {
        return Ok(a[0]);
    }
    let part = all[s..e].to_vec();
    new_bytes(vm, part)
}

fn bytes_hex(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let b = want_bytes("bytes.hex", a, 0, "b")?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let data = vm.heap.bytes_of(b);
    let mut s = String::with_capacity(2 * data.len());
    for &x in data {
        s.push(HEX[(x >> 4) as usize] as char);
        s.push(HEX[(x & 15) as usize] as char);
    }
    let fuel = text_fuel(data.len());
    spend(vm, fuel)?;
    new_str(vm, s)
}

fn bytes_from_hex(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    const F: &str = "bytes.from_hex";
    let r = want_str(F, a, 0, "hex")?;
    let s = heap::text(&vm.heap, r);
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut hi: Option<u8> = None;
    for (i, c) in s.chars().enumerate() {
        let Some(d) = c.to_digit(16) else {
            return err(format!("{}: {} ist keine Hex-Ziffer (Index {})", F, c, i));
        };
        match hi.take() {
            Some(h) => out.push(h << 4 | d as u8),
            None => hi = Some(d as u8),
        }
    }
    if hi.is_some() {
        return err(format!("{}: Die Zahl der Hex-Ziffern ist ungerade", F));
    }
    let fuel = text_fuel(s.len());
    spend(vm, fuel)?;
    new_bytes(vm, out)
}

fn bytes_text(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    const F: &str = "bytes.text";
    let b = want_bytes(F, a, 0, "b")?;
    let lossy = arg(a, 1).truthy();
    let data = vm.heap.bytes_of(b);
    let s = match core::str::from_utf8(data) {
        Ok(s) => String::from(s),
        Err(_) if lossy => String::from_utf8_lossy(data).into_owned(),
        Err(e) => return err(format!("{}: Die Bytes sind kein gültiges UTF-8 (ab Index {})", F, e.valid_up_to())),
    };
    let fuel = text_fuel(data.len());
    spend(vm, fuel)?;
    new_str(vm, s)
}

/// FNV-1a (32 bits) over bytes, the hash of str.hash too
pub(crate) fn fnv(data: &[u8]) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    for &b in data {
        h = (h ^ b as u32).wrapping_mul(0x0100_0193);
    }
    h
}

fn bytes_hash(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let b = want_bytes("bytes.hash", a, 0, "b")?;
    let data = vm.heap.bytes_of(b);
    let h = fnv(data);
    let fuel = text_fuel(data.len());
    spend(vm, fuel)?;
    Ok(Value::Int(h as i64))
}

// ---- map

fn map_len(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let m = want_map("map.len", a, 0, "m")?;
    Ok(Value::Int(vm.heap.map(m).live as i64))
}

fn map_keys(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let m = want_map("map.keys", a, 0, "m")?;
    let keys: Vec<Value> = vm.heap.map(m).iter().map(|e| e.key).collect();
    spend(vm, map_fuel(keys.len()))?;
    new_list(vm, keys)
}

fn map_values(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let m = want_map("map.values", a, 0, "m")?;
    let vals: Vec<Value> = vm.heap.map(m).iter().map(|e| e.val).collect();
    spend(vm, map_fuel(vals.len()))?;
    new_list(vm, vals)
}

/// [[key, value], ..]
fn map_entries(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let m = want_map("map.entries", a, 0, "m")?;
    let pairs: Vec<(Value, Value)> = vm.heap.map(m).iter().map(|e| (e.key, e.val)).collect();
    spend(vm, map_fuel(pairs.len()))?;
    let mut out = Vec::with_capacity(pairs.len());
    for (k, v) in pairs {
        out.push(new_list(vm, alloc::vec![k, v])?);
    }
    new_list(vm, out)
}

/// the entry position of key k in map m
fn position(vm: &Vm, f: &str, m: Ref, k: Value) -> Result<Option<usize>, Exc> {
    match vm.heap.key(k) {
        Some(key) => Ok(vm.heap.map_find(m, key)),
        None => key_error(f, k),
    }
}

fn map_has(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let m = want_map("map.has", a, 0, "m")?;
    Ok(Value::Bool(position(vm, "map.has", m, arg(a, 1))?.is_some()))
}

/// m[k], or the default when the key is missing
fn map_get(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let m = want_map("map.get", a, 0, "m")?;
    Ok(match position(vm, "map.get", m, arg(a, 1))? {
        Some(i) => vm.heap.map(m).entries[i].val,
        None => arg(a, 2),
    })
}

/// remove m[k]; whether it was there
fn map_delete(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let m = want_map("map.delete", a, 0, "m")?;
    let Some(i) = position(vm, "map.delete", m, arg(a, 1))? else { return Ok(Value::Bool(false)) };
    vm.heap.update(m, |o| {
        if let Obj::Map(map) = o {
            map.remove_at(i);
        }
    });
    Ok(Value::Bool(true))
}

/// a new map: the entries of m, then those of the other maps (later values win, the order of
/// the first appearance stays)
fn map_merge(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    const F: &str = "map.merge";
    want_map(F, a, 0, "m")?;
    let mut out: Map = Map::default();
    let mut visited = 0;
    for (i, v) in a.iter().enumerate() {
        let Value::Map(r) = *v else { return bad(F, if i == 0 { "m" } else { "other" }, "map", *v) };
        let entries: Vec<(Value, Value, u32)> = vm.heap.map(r).iter().map(|e| (e.key, e.val, e.hash)).collect();
        visited += entries.len();
        for (k, val, hash) in entries {
            let pos = vm.heap.key(k).and_then(|key| out.find(&vm.heap.objs, key));
            match pos {
                Some(p) => out.entries[p].val = val,
                None => out.push(k, hash, val),
            }
        }
    }
    spend(vm, map_fuel(visited))?;
    new_map(vm, out)
}

// ---- math

/// the exact order of two numbers that are no NaN
pub(crate) fn num_cmp(a: Value, b: Value) -> Ordering {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x.cmp(&y),
        (Value::Int(x), Value::Float(y)) => num::cmp_int(x, y).unwrap_or(Ordering::Equal),
        (Value::Float(x), Value::Int(y)) => num::cmp_int(y, x).map_or(Ordering::Equal, Ordering::reverse),
        (Value::Float(x), Value::Float(y)) => num::cmp(x, y).unwrap_or(Ordering::Equal),
        _ => Ordering::Equal,
    }
}

pub(crate) fn is_nan(v: Value) -> bool {
    matches!(v, Value::Float(x) if num::is_nan(x))
}

fn number(f: &str, v: Value) -> Result<Value, Exc> {
    match v {
        Value::Int(_) | Value::Float(_) => Ok(v),
        v => bad(f, "x", "eine Zahl", v),
    }
}

fn math_abs(_: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    match number("math.abs", arg(a, 0))? {
        Value::Int(i) => Ok(Value::Int(i.wrapping_abs())),
        Value::Float(x) => Ok(Value::Float(num::abs(x))),
        _ => unreachable!(),
    }
}

/// the smallest or largest of the arguments (or of one list); nan when one is nan
fn extreme(vm: &mut Vm, a: &[Value], f: &str, max: bool) -> Result<Value, Exc> {
    let items: Vec<Value> = match a {
        [Value::List(r)] => vm.heap.list(*r).clone(),
        _ => a.to_vec(),
    };
    spend(vm, list_fuel(items.len()))?;
    let mut best: Option<Value> = None;
    for v in items {
        let x = number(f, v)?;
        if is_nan(x) {
            return Ok(Value::Float(num::NAN));
        }
        let better = |b: Value| num_cmp(x, b) == if max { Ordering::Greater } else { Ordering::Less };
        best = Some(match best {
            Some(b) if !better(b) => b,
            _ => x,
        });
    }
    match best {
        Some(b) => Ok(b),
        None => err(format!("{} braucht mindestens einen Wert", f)),
    }
}

fn math_min(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    extreme(vm, a, "math.min", false)
}

fn math_max(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    extreme(vm, a, "math.max", true)
}

/// x limited to lo..hi (the values themselves, ints stay ints)
fn math_clamp(_: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    const F: &str = "math.clamp";
    let x = number(F, arg(a, 0))?;
    let lo = match arg(a, 1) {
        v @ (Value::Int(_) | Value::Float(_)) if !is_nan(v) => v,
        v => return bad(F, "lo", "eine Zahl (nicht nan)", v),
    };
    let hi = match arg(a, 2) {
        v @ (Value::Int(_) | Value::Float(_)) if !is_nan(v) => v,
        v => return bad(F, "hi", "eine Zahl (nicht nan)", v),
    };
    if num_cmp(lo, hi) == Ordering::Greater {
        return err(format!("{}: lo ist größer als hi", F));
    }
    Ok(if is_nan(x) {
        x
    } else if num_cmp(x, lo) == Ordering::Less {
        lo
    } else if num_cmp(x, hi) == Ordering::Greater {
        hi
    } else {
        x
    })
}

/// floor, ceil, round, trunc: an int stays the same int, a float gives the integral float
fn integral(a: &[Value], f: &str, op: fn(f64) -> f64) -> Result<Value, Exc> {
    match number(f, arg(a, 0))? {
        Value::Float(x) => Ok(Value::Float(op(x))),
        v => Ok(v),
    }
}

fn math_floor(_: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    integral(a, "math.floor", num::floor)
}

fn math_ceil(_: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    integral(a, "math.ceil", num::ceil)
}

fn math_round(_: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    integral(a, "math.round", math::round)
}

fn math_trunc(_: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    integral(a, "math.trunc", num::trunc)
}

/// a float function of one argument with a fixed cost
fn unary(vm: &mut Vm, a: &[Value], f: &str, fuel: u64, op: fn(f64) -> f64) -> Result<Value, Exc> {
    let x = want_num(f, a, 0, "x")?;
    spend(vm, fuel)?;
    Ok(Value::Float(op(x)))
}

fn math_sqrt(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    unary(vm, a, "math.sqrt", 2, num::sqrt)
}

fn math_exp(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    unary(vm, a, "math.exp", 4, math::exp)
}

fn math_log(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    unary(vm, a, "math.log", 4, math::log)
}

fn math_log2(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    unary(vm, a, "math.log2", 4, math::log2)
}

fn math_log10(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    unary(vm, a, "math.log10", 4, math::log10)
}

fn math_sin(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    unary(vm, a, "math.sin", 4, math::sin)
}

fn math_cos(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    unary(vm, a, "math.cos", 4, math::cos)
}

fn math_pow(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let x = want_num("math.pow", a, 0, "x")?;
    let y = want_num("math.pow", a, 1, "y")?;
    spend(vm, 8)?;
    Ok(Value::Float(math::pow(x, y)))
}

fn math_is_nan(_: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let x = number("math.is_nan", arg(a, 0))?;
    Ok(Value::Bool(is_nan(x)))
}

fn math_is_finite(_: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    Ok(Value::Bool(match number("math.is_finite", arg(a, 0))? {
        Value::Float(x) => x.to_bits() & 0x7FF0_0000_0000_0000 != 0x7FF0_0000_0000_0000,
        _ => true,
    }))
}
