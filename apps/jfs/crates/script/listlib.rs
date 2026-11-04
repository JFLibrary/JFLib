// listlib -- the list functions of the script library
//
//   l.len()  l.push(x, ..)  l.pop()  l.insert(i, x)  l.remove(i)  l.slice(start, end)
//   l.reverse()  l.sort(cmp)  l.map(f)  l.filter(f)  l.reduce(f, init)  l.contains(x)
//   l.index(x)  l.join(sep)
//
// insert, remove, reverse and sort change the list (reverse and sort return it); slice, map
// and filter make a new one.  insert takes 0..=len, remove 0..len (a negative index is an
// error, as for l[i]).  contains and index compare with ==; index answers nil when x is
// missing.  map and filter call f with the item alone (so l.map(str.trim) works) on the
// items the list had when the call started; reduce without init starts with the first item
// (an empty list then is an error).
//
// sort is a stable merge sort of its own (the order of the comparisons, and so the fuel and
// what a comparison function sees, is the same on every target and with every Rust).
// Without cmp it orders numbers by value (ints and floats exactly against each other, nan
// last), strs and bytes by their bytes; other or mixed types throw.  cmp(a, b) answers a
// number (negative: a before b) or a bool (true: a before b).  The list is replaced at the
// end, so an error or an exception in cmp leaves it as it was.  1 fuel per comparison.

use alloc::format;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::heap::{self, Obj, Value};
use crate::num;
use crate::stdlib::{arg, bad, call, err, is_nan, keep, list_fuel, new_list, num_cmp, span, spend, want_fn, want_int, want_list};
use crate::strlib;
use crate::vm::{Exc, Host, Vm};

pub(crate) fn len(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let l = want_list("list.len", a, 0, "l")?;
    Ok(Value::Int(vm.heap.list(l).len() as i64))
}

/// append the arguments (as l.push(x, ..))
pub(crate) fn push(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let l = want_list("list.push", a, 0, "l")?;
    vm.reserve(16 * (a.len() as u64).saturating_sub(1))?;
    vm.heap.update(l, |o| {
        if let Obj::List(v) = o {
            v.extend_from_slice(&a[1..]);
        }
    });
    Ok(Value::Nil)
}

/// remove and return the last item (nil when the list is empty)
pub(crate) fn pop(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let l = want_list("list.pop", a, 0, "l")?;
    Ok(vm.heap.update(l, |o| match o {
        Obj::List(v) => v.pop(),
        _ => None,
    }).unwrap_or(Value::Nil))
}

fn outside<T>(f: &str, i: i64, n: usize) -> Result<T, Exc> {
    err(format!("{}: Index {} liegt außerhalb der Liste (Länge {})", f, i, n))
}

/// x before index i (0..=len)
pub(crate) fn insert(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    const F: &str = "list.insert";
    let l = want_list(F, a, 0, "l")?;
    let i = want_int(F, a, 1, "i")?;
    let n = vm.heap.list(l).len();
    if i < 0 || i as u64 > n as u64 {
        return outside(F, i, n);
    }
    spend(vm, list_fuel(n - i as usize))?;
    vm.reserve(16)?;
    let x = arg(a, 2);
    vm.heap.update(l, |o| {
        if let Obj::List(v) = o {
            v.insert(i as usize, x);
        }
    });
    Ok(Value::Nil)
}

/// remove and return the item at index i (0..len)
pub(crate) fn remove(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    const F: &str = "list.remove";
    let l = want_list(F, a, 0, "l")?;
    let i = want_int(F, a, 1, "i")?;
    let n = vm.heap.list(l).len();
    if i < 0 || i as u64 >= n as u64 {
        return outside(F, i, n);
    }
    spend(vm, list_fuel(n - i as usize))?;
    Ok(vm.heap.update(l, |o| match o {
        Obj::List(v) => v.remove(i as usize),
        _ => Value::Nil,
    }))
}

/// a new list of the items start..end
pub(crate) fn slice(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let l = want_list("list.slice", a, 0, "l")?;
    let items = vm.heap.list(l);
    let (s, e) = span("list.slice", a, 1, items.len())?;
    let part = items[s..e].to_vec();
    new_list(vm, part)
}

/// reverse in place; the list
pub(crate) fn reverse(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let l = want_list("list.reverse", a, 0, "l")?;
    let n = vm.heap.list(l).len();
    spend(vm, list_fuel(n))?;
    vm.heap.update(l, |o| {
        if let Obj::List(v) = o {
            v.reverse();
        }
    });
    Ok(a[0])
}

/// the first index of an item == x
fn position(vm: &mut Vm, a: &[Value], f: &str) -> Result<Option<usize>, Exc> {
    let l = want_list(f, a, 0, "l")?;
    let x = arg(a, 1);
    let items = vm.heap.list(l);
    let at = items.iter().position(|v| vm.same(*v, x));
    let seen = at.map_or(items.len(), |i| i + 1);
    spend(vm, list_fuel(seen))?;
    Ok(at)
}

pub(crate) fn contains(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    Ok(Value::Bool(position(vm, a, "list.contains")?.is_some()))
}

pub(crate) fn index(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    Ok(position(vm, a, "list.index")?.map_or(Value::Nil, |i| Value::Int(i as i64)))
}

/// the items' texts with sep between them (l.join(sep), sep "" when missing)
pub(crate) fn join(vm: &mut Vm, _: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let l = want_list("list.join", a, 0, "l")?;
    let sep = match arg(a, 1) {
        Value::Nil => None,
        Value::Str(r) => Some(r),
        v => return bad("list.join", "sep", "str", v),
    };
    strlib::join_list(vm, l, sep)
}

/// the items now, kept alive while callbacks run (they may change the list)
fn snapshot(vm: &mut Vm, l: heap::Ref) -> Vec<Value> {
    let items = vm.heap.list(l).clone();
    for v in &items {
        keep(vm, *v);
    }
    items
}

/// a new list of f(item)
pub(crate) fn map(vm: &mut Vm, host: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let l = want_list("list.map", a, 0, "l")?;
    let f = want_fn("list.map", a, 1, "f")?;
    let items = snapshot(vm, l);
    let mut out = Vec::with_capacity(items.len());
    for x in items {
        spend(vm, 1)?;
        out.push(call(vm, host, f, &[x])?);
    }
    new_list(vm, out)
}

/// a new list of the items for which f(item) is truthy
pub(crate) fn filter(vm: &mut Vm, host: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let l = want_list("list.filter", a, 0, "l")?;
    let f = want_fn("list.filter", a, 1, "f")?;
    let items = snapshot(vm, l);
    let mut out = Vec::new();
    for x in items {
        spend(vm, 1)?;
        if call(vm, host, f, &[x])?.truthy() {
            out.push(x);
        }
    }
    new_list(vm, out)
}

/// f(f(f(init, a), b), c) ..
pub(crate) fn reduce(vm: &mut Vm, host: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    const F: &str = "list.reduce";
    let l = want_list(F, a, 0, "l")?;
    let f = want_fn(F, a, 1, "f")?;
    let items = snapshot(vm, l);
    let (mut acc, rest) = if a.len() >= 3 {
        (a[2], &items[..])
    } else {
        match items.split_first() {
            Some((first, rest)) => (*first, rest),
            None => return err(format!("{}: Eine leere Liste braucht einen Startwert (init)", F)),
        }
    };
    for x in rest {
        spend(vm, 1)?;
        acc = call(vm, host, f, &[acc, *x])?;
    }
    Ok(acc)
}

/// the default order of sort
fn order(vm: &Vm, a: Value, b: Value) -> Result<Ordering, Exc> {
    match (a, b) {
        (Value::Int(_) | Value::Float(_), Value::Int(_) | Value::Float(_)) => Ok(match (is_nan(a), is_nan(b)) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            _ => num_cmp(a, b),
        }),
        (Value::Str(x), Value::Str(y)) => Ok(heap::text(&vm.heap, x).cmp(heap::text(&vm.heap, y))),
        (Value::Bytes(x), Value::Bytes(y)) => Ok(vm.heap.bytes_of(x).cmp(vm.heap.bytes_of(y))),
        _ => err(format!(
            "list.sort: {} und {} lassen sich nicht vergleichen – gib eine Vergleichsfunktion an",
            a.type_name(),
            b.type_name()
        )),
    }
}

struct Sorter<'a> {
    vm: &'a mut Vm,
    host: &'a mut dyn Host,
    cmp: Option<Value>,
    /// comparisons not paid for yet
    unpaid: u64,
}

impl Sorter<'_> {
    /// whether b must come before a (false for equal ones: the sort is stable)
    fn before(&mut self, b: Value, a: Value) -> Result<bool, Exc> {
        self.unpaid += 1;
        if self.unpaid == 1024 {
            spend(self.vm, 1024)?;
            self.unpaid = 0;
        }
        let Some(f) = self.cmp else { return Ok(order(self.vm, b, a)? == Ordering::Less) };
        match call(self.vm, self.host, f, &[b, a])? {
            Value::Bool(x) => Ok(x),
            Value::Int(i) => Ok(i < 0),
            Value::Float(x) => Ok(num::lt(x, 0.0)),
            v => err(format!("list.sort: cmp muss eine Zahl oder bool zurückgeben, nicht {}", v.type_name())),
        }
    }

    /// sort v, with tmp (as long as v) as scratch space
    fn sort(&mut self, v: &mut [Value], tmp: &mut [Value]) -> Result<(), Exc> {
        let n = v.len();
        if n <= 8 {
            // insertion sort
            for i in 1..n {
                let x = v[i];
                let mut j = i;
                while j > 0 && self.before(x, v[j - 1])? {
                    v[j] = v[j - 1];
                    j -= 1;
                }
                v[j] = x;
            }
            return Ok(());
        }
        let mid = n / 2;
        self.sort(&mut v[..mid], &mut tmp[..mid])?;
        self.sort(&mut v[mid..], &mut tmp[mid..])?;
        if !self.before(v[mid], v[mid - 1])? {
            return Ok(());
        }
        tmp.copy_from_slice(v);
        let (mut i, mut j) = (0, mid);
        for slot in v.iter_mut() {
            if j < n && (i >= mid || self.before(tmp[j], tmp[i])?) {
                *slot = tmp[j];
                j += 1;
            } else {
                *slot = tmp[i];
                i += 1;
            }
        }
        Ok(())
    }
}

/// sort in place (stable), with an optional comparison function; the list
pub(crate) fn sort(vm: &mut Vm, host: &mut dyn Host, a: &[Value]) -> Result<Value, Exc> {
    let l = want_list("list.sort", a, 0, "l")?;
    let cmp = match arg(a, 1) {
        Value::Nil => None,
        _ => Some(want_fn("list.sort", a, 1, "cmp")?),
    };
    let mut items = if cmp.is_some() { snapshot(vm, l) } else { vm.heap.list(l).clone() };
    let mut tmp = items.clone();
    let mut s = Sorter { vm: &mut *vm, host: &mut *host, cmp, unpaid: 0 };
    s.sort(&mut items, &mut tmp)?;
    let unpaid = s.unpaid;
    spend(vm, unpaid)?;
    let grow = 16 * (items.len() as u64).saturating_sub(vm.heap.list(l).len() as u64);
    vm.reserve(grow)?;
    vm.heap.update(l, |o| {
        if let Obj::List(v) = o {
            *v = items;
        }
    });
    Ok(a[0])
}
