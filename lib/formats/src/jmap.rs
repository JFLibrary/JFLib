// jmap -- typed JSON mapping for records and string enums over json::Value: the Field trait
// for member values, the Json trait for records, and the record! / wire_enum! macros that
// declare a type and its mapping in one place.  Missing or mistyped members read as their
// default; unknown members are ignored, so old and new programs read each other's data
// (protocols, project files, settings).
//
//   record! { pub struct P { pub a: String, pub n: u32, pub items: Vec<Item> } }
//   wire_enum! { pub enum Kind { Pc = "pc", Router = "router" } }   // Default = first variant
//   p.to_json() / P::from_json(&v) / Field::put / Field::take
//   list(&[T]) / from_list(&v) / str_of / u64_of / obj([...]) / empty_object()
//
// Field is implemented for String, bool, the integer types, Value (kept as is), Vec<T>,
// Option<T> (null = None) and BTreeMap<String, T> (an object).  Floating point has no exact
// JSON token here: store fixed-point integers instead.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

pub use crate::json::Value;

/// a value that maps to one JSON member
pub trait Field: Sized + Default {
    fn put(&self) -> Value;
    fn take(v: Option<&Value>) -> Self;
}

/// a record (a JSON object)
pub trait Json: Sized {
    fn to_json(&self) -> Value;
    fn from_json(v: &Value) -> Option<Self>;
}

impl Field for String {
    fn put(&self) -> Value {
        Value::string(self.as_str())
    }
    fn take(v: Option<&Value>) -> Self {
        v.and_then(Value::as_str).map(String::from).unwrap_or_default()
    }
}

impl Field for bool {
    fn put(&self) -> Value {
        Value::Bool(*self)
    }
    fn take(v: Option<&Value>) -> Self {
        v.and_then(Value::as_bool).unwrap_or(false)
    }
}

impl Field for u64 {
    fn put(&self) -> Value {
        Value::u64(*self)
    }
    fn take(v: Option<&Value>) -> Self {
        v.and_then(Value::as_u64).unwrap_or(0)
    }
}

impl Field for i64 {
    fn put(&self) -> Value {
        Value::i64(*self)
    }
    fn take(v: Option<&Value>) -> Self {
        v.and_then(Value::as_i64).unwrap_or(0)
    }
}

impl Field for u32 {
    fn put(&self) -> Value {
        Value::u64(*self as u64)
    }
    fn take(v: Option<&Value>) -> Self {
        v.and_then(Value::as_u64).map_or(0, |x| x.min(u32::MAX as u64) as u32)
    }
}

impl Field for i32 {
    fn put(&self) -> Value {
        Value::i64(*self as i64)
    }
    fn take(v: Option<&Value>) -> Self {
        v.and_then(Value::as_i64).map_or(0, |x| x.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
    }
}

impl Field for u16 {
    fn put(&self) -> Value {
        Value::u64(*self as u64)
    }
    fn take(v: Option<&Value>) -> Self {
        v.and_then(Value::as_u64).map_or(0, |x| x.min(u16::MAX as u64) as u16)
    }
}

impl Field for u8 {
    fn put(&self) -> Value {
        Value::u64(*self as u64)
    }
    fn take(v: Option<&Value>) -> Self {
        v.and_then(Value::as_u64).map_or(0, |x| x.min(255) as u8)
    }
}

impl Default for Value {
    fn default() -> Value {
        Value::Null
    }
}

/// a free-form member (configuration of a plugin, a service, ...)
impl Field for Value {
    fn put(&self) -> Value {
        self.clone()
    }
    fn take(v: Option<&Value>) -> Self {
        v.cloned().unwrap_or(Value::Null)
    }
}

impl<T: Field> Field for Vec<T> {
    fn put(&self) -> Value {
        Value::Array(self.iter().map(Field::put).collect())
    }
    fn take(v: Option<&Value>) -> Self {
        v.and_then(Value::as_array).map(|a| a.iter().map(|x| T::take(Some(x))).collect()).unwrap_or_default()
    }
}

impl<T: Field> Field for Option<T> {
    fn put(&self) -> Value {
        match self {
            Some(x) => x.put(),
            None => Value::Null,
        }
    }
    fn take(v: Option<&Value>) -> Self {
        match v {
            None | Some(Value::Null) => None,
            Some(x) => Some(T::take(Some(x))),
        }
    }
}

/// a map with string keys as a JSON object (members in key order)
impl<T: Field> Field for BTreeMap<String, T> {
    fn put(&self) -> Value {
        Value::Object(self.iter().map(|(k, v)| (k.clone(), v.put())).collect())
    }
    fn take(v: Option<&Value>) -> Self {
        v.and_then(Value::as_object).map(|o| o.iter().map(|(k, x)| (k.clone(), T::take(Some(x)))).collect()).unwrap_or_default()
    }
}

/// an empty JSON object (used by record!)
pub fn empty_object() -> Value {
    Value::Object(Vec::new())
}

/// declare a record: the struct (Clone, Debug, Default, PartialEq) and its Json / Field
/// mapping; the JSON member names are the field names
#[macro_export]
macro_rules! record {
    ($(#[$m:meta])* pub struct $name:ident { $( $(#[$fm:meta])* pub $f:ident : $t:ty ),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug, Default, PartialEq)]
        pub struct $name { $( $(#[$fm])* pub $f: $t ),* }
        impl $crate::jmap::Json for $name {
            fn to_json(&self) -> $crate::jmap::Value {
                #[allow(unused_mut)]
                let mut o = $crate::jmap::empty_object();
                $( o.insert(stringify!($f), $crate::jmap::Field::put(&self.$f)); )*
                o
            }
            fn from_json(v: &$crate::jmap::Value) -> Option<Self> {
                v.as_object()?;
                Some($name { $( $f: $crate::jmap::Field::take(v.get(stringify!($f))) ),* })
            }
        }
        impl $crate::jmap::Field for $name {
            fn put(&self) -> $crate::jmap::Value {
                $crate::jmap::Json::to_json(self)
            }
            fn take(v: Option<&$crate::jmap::Value>) -> Self {
                v.and_then(<$name as $crate::jmap::Json>::from_json).unwrap_or_default()
            }
        }
    };
}

/// declare an enum carried as a string: Default is the first variant, ALL, as_str / parse,
/// Field
#[macro_export]
macro_rules! wire_enum {
    ($(#[$m:meta])* pub enum $name:ident { $first:ident = $fs:literal $(, $v:ident = $s:literal)* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name { #[default] $first $(, $v)* }
        impl $name {
            /// every variant in declaration order
            pub const ALL: &'static [$name] = &[$name::$first $(, $name::$v)*];
            /// the wire name
            pub fn as_str(&self) -> &'static str {
                match self { $name::$first => $fs $(, $name::$v => $s)* }
            }
            /// the variant of a wire name
            pub fn parse(s: &str) -> Option<$name> {
                match s { $fs => Some($name::$first) $(, $s => Some($name::$v))*, _ => None }
            }
        }
        impl $crate::jmap::Field for $name {
            fn put(&self) -> $crate::jmap::Value {
                $crate::jmap::Value::string(self.as_str())
            }
            fn take(v: Option<&$crate::jmap::Value>) -> Self {
                v.and_then($crate::jmap::Value::as_str).and_then($name::parse).unwrap_or_default()
            }
        }
    };
}

/// a list of records as a JSON array
pub fn list<T: Json>(items: &[T]) -> Value {
    Value::Array(items.iter().map(Json::to_json).collect())
}

/// the records of a JSON array (or of the array under `items` of an object)
pub fn from_list<T: Json>(v: &Value) -> Vec<T> {
    let arr = v.as_array().or_else(|| v.get("items").and_then(Value::as_array));
    arr.map(|a| a.iter().filter_map(T::from_json).collect()).unwrap_or_default()
}

/// a string member of an object ("" when missing)
pub fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// an unsigned member of an object (0 when missing)
pub fn u64_of(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

/// an object built from (name, value) pairs
pub fn obj<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::object(fields)
}
