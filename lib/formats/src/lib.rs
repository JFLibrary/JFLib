// formats -- data formats as text: JSON, typed JSON records, an XML subset, Unicode encodings
//
//   json   strict bounded JSON (RFC 8259; duplicate member names and lone surrogates are
//          errors): Value, Number (the exact token, no rounding through f64), parse /
//          parse_with_limits, stringify / stringify_with_limits / stringify_pretty, Limits,
//          Error { offset, kind }
//   jmap   typed mapping of records and string enums over json::Value: Field, Json,
//          record! / wire_enum!, list / from_list, str_of / u64_of / obj / empty_object
//   xml    a bounded, namespace-aware XML subset: Node / Name, parse / parse_with(Limits),
//          escape, valid_char; the five predefined entities and character references, and
//          Unsupported for a DTD, other entities, comments, CDATA, processing instructions
//          and mixed content
//   encoding  decode(bytes, limit) -> (String, Encoding) / encode(text, Encoding): UTF-8 with
//          and without BOM, UTF-16 LE / BE; no legacy code pages and no guessing
//
// Every reader takes untrusted input: it checks the size before it reads and the nesting, the
// values or the elements while it reads, and reports a malformed or too large input as an error,
// never a panic.  Only alloc; no I/O.
//
// json and jmap came from agent_core, xml and encoding from base (0.3.0); agent_core::{json,
// jmap, record!, wire_enum!} and base::{xml, encoding} remain as re-exports of these modules.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod encoding;
pub mod jmap;
pub mod json;
pub mod xml;
