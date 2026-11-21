// formats -- data formats as text: JSON, typed JSON records, an XML subset, Unicode encodings
//
//   json   strict bounded JSON (RFC 8259; duplicate member names and lone surrogates are
//          errors): Value, Number (the exact token, no rounding through f64), parse /
//          parse_with_limits, stringify / stringify_with_limits / stringify_pretty, Limits,
//          Error { offset, kind }
//   jmap   typed mapping of records and string enums over json::Value: Field, Json,
//          record! / wire_enum!, list / from_list, str_of / u64_of / obj / empty_object
//   xml    Reader: the strict pull reader (well-formed XML 1.0 with namespaces, events, offsets,
//          skip; DOCTYPE refused); Lenient: the forgiving reader of files from editors (tokens,
//          a DOCTYPE's entities within a budget); parse / parse_with(Limits) -> Node: a bounded
//          tree of the strict subset (Unsupported for comments, CDATA, processing instructions
//          and mixed content); escape, valid_char
//   encoding  decode(bytes, limit) -> (String, Encoding) / encode(text, Encoding): UTF-8 with
//          and without BOM, UTF-16 LE / BE, strict and without guessing; Charset: the single-byte
//          sets of the web (WHATWG: windows-1252, ISO-8859-x, KOI8 ...) and of old files (the DOS
//          code pages); label / decode_label: the WHATWG Encoding Standard's labels and decode
//
// Every reader takes untrusted input: it checks the size before it reads and the nesting, the
// values or the elements while it reads, and reports a malformed or too large input as an error,
// never a panic.  Only alloc; no I/O.
//
// The single-byte tables and the WHATWG labels are generated (charsets.rs, tools/gen_encodings.py).
//
// json and jmap came from agent_core, xml and encoding from base (0.3.0); agent_core::{json,
// jmap, record!, wire_enum!} and base::{xml, encoding} remain as re-exports of these modules.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod charsets;
pub mod encoding;
pub mod jmap;
pub mod json;
pub mod xml;
