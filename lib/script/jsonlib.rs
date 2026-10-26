// jsonlib -- json.encode and json.decode of the script library, on agent_core::json; maps
// keep their order both ways
//
//   json.encode(v, indent)  nil bool int float str list map as JSON text: compact, or with
//                           indent (1 to 8) spaces per level for people; int and bool keys
//                           become their text; nan and inf, bytes, functions, keys that
//                           collide (1 and "1"), data that contains itself and more than 64
//                           levels throw
//   json.decode(text)       JSON in a str or in bytes (UTF-8) as values: objects become maps
//                           in the order of their members, numbers without a point or an
//                           exponent ints (floats when they are too big for an int), the
//                           others floats; errors throw with line and column ("json.decode:
//                           Hier fehlt ein Doppelpunkt (Zeile 1, Spalte 6)")
//   encode / decode         the same for hosts (Vm::json_encode, Cx::json_decode ..)
//
// Fuel: 1 per 64 bytes of text, 1 per 8 values, 1 per new object.  The text is paid for
// before it is read and the encoder counts the values as it visits them, so a huge job stops
// in the middle.  The heap limit caps the number of values (each takes 16 bytes or more)
// before anything is built.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use agent_core::json::{self as js, Number, Value as J};

use crate::heap::{self, Map, Ref, Value};
use crate::num;
use crate::stdlib::{arg, bad, cap, err, heap_full, list_fuel, new_list, new_map, new_str, opt_int, parse_float, spend, text_fuel};
use crate::vm::{fuel_msg, ErrorKind, Exc, Host, Vm};

/// the most levels: no value deeper than this below the outermost one
pub(crate) const DEPTH: usize = 64;

/// the most values the heap could hold
fn most(vm: &Vm) -> u64 {
    (vm.lim.heap / 16).max(16)
}

struct Enc<'a> {
    vm: &'a Vm,
    /// the lists and maps being encoded (a cycle comes back to one of them)
    seen: Vec<Ref>,
    count: u64,
    most: u64,
    /// the fuel the invocation has left
    budget: u64,
    /// the bytes of text copied into the tree so far (a shared string is copied per visit)
    strings: u64,
}

impl Enc<'_> {
    fn charge(&mut self, len: usize) -> Result<(), Exc> {
        self.strings = self.strings.saturating_add(len as u64);
        cap(self.vm, self.strings)
    }

    fn value(&mut self, v: Value, depth: usize) -> Result<J, Exc> {
        let vm = self.vm;
        self.count += 1;
        if self.count > self.most {
            return Err(heap_full(vm));
        }
        if self.count / 8 > self.budget {
            return Err(Exc::Fatal(ErrorKind::Fuel, fuel_msg(vm.lim.fuel)));
        }
        if depth > DEPTH {
            return err(format!("json.encode: Die Daten sind zu tief verschachtelt (mehr als {} Ebenen)", DEPTH));
        }
        Ok(match v {
            Value::Nil | Value::Cell(_) => J::Null,
            Value::Bool(b) => J::Bool(b),
            Value::Int(i) => J::i64(i),
            Value::Float(x) => {
                let mut s = String::new();
                num::fmt(x, &mut s);
                match Number::parse(&s) {
                    Some(n) => J::Number(n),
                    None => return err(format!("json.encode: {} lässt sich nicht als JSON darstellen", s)),
                }
            }
            Value::Str(r) => {
                let s = heap::text(&vm.heap, r);
                self.charge(s.len())?;
                J::String(s.to_string())
            }
            Value::Bytes(_) => return err(String::from("json.encode: bytes lassen sich nicht als JSON darstellen – erst b.hex() oder b.text()")),
            Value::Func(_) | Value::Native(_) => return err(String::from("json.encode: Funktionen lassen sich nicht als JSON darstellen")),
            Value::List(r) => {
                self.enter(r)?;
                let items = vm.heap.list(r);
                let mut out = Vec::with_capacity(items.len());
                for x in items {
                    out.push(self.value(*x, depth + 1)?);
                }
                self.seen.pop();
                J::Array(out)
            }
            Value::Map(r) => {
                self.enter(r)?;
                let m = vm.heap.map(r);
                // only keys of different types can collide
                let mixed = m.iter().any(|e| !matches!(e.key, Value::Str(_)));
                let mut names: BTreeSet<String> = BTreeSet::new();
                let mut out = Vec::with_capacity(m.live as usize);
                for e in m.iter() {
                    let k = match e.key {
                        Value::Str(s) => heap::text(&vm.heap, s).to_string(),
                        Value::Int(i) => i.to_string(),
                        Value::Bool(b) => String::from(if b { "true" } else { "false" }),
                        _ => String::new(),
                    };
                    self.charge(k.len())?;
                    if mixed && !names.insert(k.clone()) {
                        return err(format!("json.encode: Der Schlüssel \"{}\" kommt doppelt vor", k));
                    }
                    out.push((k, self.value(e.val, depth + 1)?));
                }
                self.seen.pop();
                J::Object(out)
            }
        })
    }

    fn enter(&mut self, r: Ref) -> Result<(), Exc> {
        if self.seen.contains(&r) {
            return err(String::from("json.encode: Die Daten enthalten sich selbst"));
        }
        self.seen.push(r);
        Ok(())
    }
}

/// v as JSON text (indent 0: compact)
pub(crate) fn encode(vm: &mut Vm, v: Value, indent: u32) -> Result<String, Exc> {
    let budget = vm.lim.fuel.saturating_sub(vm.call_fuel + vm.used);
    let mut enc = Enc { vm: &*vm, seen: Vec::new(), count: 0, most: most(vm), budget, strings: 0 };
    let tree = enc.value(v, 0)?;
    let count = enc.count;
    let lim = js::Limits {
        bytes: usize::try_from(vm.lim.heap.saturating_sub(32)).unwrap_or(usize::MAX),
        depth: DEPTH,
        values: usize::MAX,
        string_bytes: usize::MAX,
        members: usize::MAX,
    };
    let text = if indent == 0 { js::stringify_with_limits(&tree, lim) } else { js::stringify_pretty(&tree, lim, indent as usize) };
    let text = match text {
        Ok(t) => t,
        Err(e) if e.kind == "byte_limit" => return Err(heap_full(vm)),
        Err(e) => return err(format!("json.encode: {}", e.kind)),
    };
    spend(vm, list_fuel(count as usize) + text_fuel(text.len()))?;
    Ok(text)
}

/// json.encode(v, indent)
pub(crate) fn encode_fn(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let indent = match opt_int("json.encode", a, 1, "indent")? {
        None => 0,
        Some(n @ 0..=8) => n as u32,
        Some(n) => return err(format!("json.encode: indent muss zwischen 0 und 8 liegen, nicht {}", n)),
    };
    let text = encode(vm, arg(a, 0), indent)?;
    spend(vm, 1)?;
    vm.alloc_str(text)
}

/// the German message of a parse error, with line and column (characters)
fn syntax(vm: &Vm, text: &[u8], e: js::Error) -> Exc {
    let what = match e.kind {
        "value_limit" | "member_limit" | "byte_limit" | "string_limit" => return heap_full(vm),
        "invalid_utf8" => "Der Text ist kein gültiges UTF-8",
        "trailing_data" => "Nach dem Wert folgt noch etwas",
        "depth_limit" => "Die Daten sind zu tief verschachtelt (mehr als 64 Ebenen)",
        "expected_value" => "Hier fehlt ein Wert",
        "invalid_literal" => "Unbekanntes Wort (erlaubt sind true, false und null)",
        "invalid_number" => "Ungültige Zahl",
        "number_limit" => "Die Zahl ist zu lang",
        "invalid_escape" => "Ungültige Escape-Sequenz",
        "unpaired_surrogate" => "Ungültiges \\u-Zeichen (ein Surrogat ohne Partner)",
        "unescaped_control" => "Ein Steuerzeichen im Text muss als Escape-Sequenz stehen",
        "unterminated_string" => "Der Text endet ohne schließendes \"",
        "expected_string" => "Hier fehlt ein Schlüssel in Anführungszeichen",
        "expected_comma" => "Hier fehlt ein Komma oder eine schließende Klammer",
        "expected_colon" => "Hier fehlt ein Doppelpunkt",
        "duplicate_key" => "Der Schlüssel kommt doppelt vor",
        _ => "Ungültiges JSON",
    };
    let head = &text[..e.offset.min(text.len())];
    let line = 1 + head.iter().filter(|&&b| b == b'\n').count();
    let start = head.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let col = 1 + String::from_utf8_lossy(&head[start..]).chars().count();
    Exc::Msg(format!("json.decode: {} (Zeile {}, Spalte {})", what, line, col))
}

/// the syntax tree of a JSON text
fn parse(vm: &Vm, text: &[u8]) -> Result<J, Exc> {
    let lim = js::Limits {
        bytes: usize::MAX,
        depth: DEPTH,
        values: usize::try_from(most(vm)).unwrap_or(usize::MAX),
        string_bytes: usize::MAX,
        members: usize::MAX,
    };
    js::parse_with_limits(text, lim).map_err(|e| syntax(vm, text, e))
}

fn number(n: &Number) -> Value {
    let s = n.as_str();
    if !s.contains(['.', 'e', 'E']) {
        if let Some(i) = n.as_i64() {
            return Value::Int(i);
        }
    }
    Value::Float(parse_float(s).unwrap_or(num::NAN))
}

/// the values of a syntax tree
fn build(vm: &mut Vm, j: &J) -> Result<Value, Exc> {
    Ok(match j {
        J::Null => Value::Nil,
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => number(n),
        J::String(s) => new_str(vm, s.clone())?,
        J::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for x in items {
                out.push(build(vm, x)?);
            }
            new_list(vm, out)?
        }
        J::Object(fields) => {
            let mut m = Map::default();
            for (k, x) in fields {
                let key = new_str(vm, k.clone())?;
                let val = build(vm, x)?;
                m.push(key, heap::hash_str(k), val);
            }
            new_map(vm, m)?
        }
    })
}

/// the value of a JSON text
pub(crate) fn decode(vm: &mut Vm, text: &[u8]) -> Result<Value, Exc> {
    spend(vm, text_fuel(text.len()))?;
    let tree = parse(vm, text)?;
    build(vm, &tree)
}

/// json.decode(text)
pub(crate) fn decode_fn(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let len = match arg(a, 0) {
        Value::Str(r) => heap::text(&vm.heap, r).len(),
        Value::Bytes(r) => vm.heap.bytes_of(r).len(),
        v => return bad("json.decode", "text", "str oder bytes", v),
    };
    spend(vm, text_fuel(len))?;
    let tree = match arg(a, 0) {
        Value::Str(r) => parse(vm, heap::text(&vm.heap, r).as_bytes())?,
        Value::Bytes(r) => parse(vm, vm.heap.bytes_of(r))?,
        _ => J::Null,
    };
    build(vm, &tree)
}
