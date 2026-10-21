// j -- JSON mapping of the wire types (agent_core::json): the Field and Json traits of
// agent_core::jmap, and the record! / wire_enum! macros that declare a type and its
// mapping in one place.  Missing or mistyped fields read as their default (the server checks
// what it needs); unknown fields are ignored, so old and new programs understand each other.

use alloc::string::String;

// the traits and their implementations live in the library (agent_core::jmap); the macros
// below keep their `$crate::j::` paths, so bc_proto's records and enums implement those traits
pub use agent_core::jmap::{from_list, list, obj, str_of, u64_of, Field, Json, Value};

/// declare a record: the struct (Clone, Debug, Default, PartialEq) and its Json / Field
/// mapping; the JSON member names are the field names
#[macro_export]
macro_rules! record {
    ($(#[$m:meta])* pub struct $name:ident { $( $(#[$fm:meta])* pub $f:ident : $t:ty ),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug, Default, PartialEq)]
        pub struct $name { $( $(#[$fm])* pub $f: $t ),* }
        impl $crate::j::Json for $name {
            fn to_json(&self) -> $crate::j::Value {
                let mut o = $crate::j::Value::Object(alloc::vec::Vec::new());
                $( o.insert(stringify!($f), $crate::j::Field::put(&self.$f)); )*
                o
            }
            fn from_json(v: &$crate::j::Value) -> Option<Self> {
                v.as_object()?;
                Some($name { $( $f: $crate::j::Field::take(v.get(stringify!($f))) ),* })
            }
        }
        impl $crate::j::Field for $name {
            fn put(&self) -> $crate::j::Value {
                $crate::j::Json::to_json(self)
            }
            fn take(v: Option<&$crate::j::Value>) -> Self {
                v.and_then(<$name as $crate::j::Json>::from_json).unwrap_or_default()
            }
        }
    };
}

/// declare an enum carried as a string: Default is the first variant, as_str / parse, Field
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
        impl $crate::j::Field for $name {
            fn put(&self) -> $crate::j::Value {
                $crate::j::Value::string(self.as_str())
            }
            fn take(v: Option<&$crate::j::Value>) -> Self {
                v.and_then($crate::j::Value::as_str).and_then($name::parse).unwrap_or_default()
            }
        }
    };
}

/// the encoded text of a value (empty on a limit violation)
pub fn text(v: &Value) -> String {
    agent_core::json::stringify_with_limits(v, big()).unwrap_or_default()
}

/// parse with the limits of API bodies (8 MiB, 200 000 values)
pub fn parse(bytes: &[u8]) -> Option<Value> {
    agent_core::json::parse_with_limits(bytes, big()).ok()
}

/// the limits of API bodies and stored records
pub fn big() -> agent_core::json::Limits {
    agent_core::json::Limits { bytes: 8 << 20, depth: 32, values: 200_000, string_bytes: 1 << 20, members: 100_000 }
}
