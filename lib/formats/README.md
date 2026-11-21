# jflib-formats

Data formats as text for [JFLib](https://jflib.org): JSON, typed JSON records, a bounded XML
subset and the Unicode text encodings, in plain Rust, without the standard library, without a line
of `unsafe` code and without a dependency. The crate holds what settings and project files,
manifests, protocols and document formats all need, so that a program that only reads JSON or XML
does not have to depend on the crate of the agent interface or on the platform layer.

## What it does

**JSON** (`formats::json`) reads and writes JSON as RFC 8259 defines it, strictly: a duplicate
member name, a lone surrogate, bytes after the value or invalid UTF-8 are errors, each with the
byte offset where it was found and a stable name for the kind (`Error { offset, kind }`). A
`Value` keeps the members of an object in their order, and a `Number` keeps the token as written,
so a 64-bit integer or a long decimal comes back without being rounded through floating point
(`as_i64`, `as_u64`, `as_str`). `stringify` writes the shortest form, `stringify_pretty` an
indented one for files.

**Typed records** (`formats::jmap`) map Rust types to JSON objects and strings without a derive
macro. `record!` declares a struct together with its mapping, `wire_enum!` an enum carried as a
string. A missing or mistyped member reads as its default and unknown members are ignored, so an
old and a new program read each other's data.

**XML** (`formats::xml`) is a namespace-aware subset for interchange formats such as OPC
packages, DOCX parts and manifests: `parse` builds a tree of elements that hold either elements
or text, with names expanded to their namespaces. References are the five predefined entities and
character references. What the subset cannot represent losslessly is refused as `Unsupported`
instead of being read halfway: a DTD, other entities, comments, CDATA sections, processing
instructions and mixed content. `escape` writes text and attribute values that survive the
reader's newline normalization.

**Encodings** (`formats::encoding`) decode bytes to a `String` and back: UTF-8 with or without a
byte-order mark and UTF-16 in both byte orders, told apart by the mark. Nothing is replaced or
guessed: invalid sequences and UTF-32 are errors.

Every reader treats its input as untrusted. It checks the size before it reads and the nesting,
the values or the elements while it reads (`json::Limits`, `xml::Limits`, the `limit` of
`encoding::decode`), and reports a malformed or too large input as an error, never a panic.

```rust
use jflib::formats::{json, record, wire_enum};
use jflib::formats::jmap::Json;

wire_enum! { pub enum Kind { File = "file", Folder = "folder" } }
record! { pub struct Entry { pub name: String, pub kind: Kind, pub size: u64 } }

let v = json::parse(br#"{"name": "notes.txt", "kind": "file", "size": 1234}"#)?;
let e = Entry::from_json(&v).unwrap_or_default();
let text = json::stringify(&e.to_json())?;           // {"name":"notes.txt","kind":"file","size":1234}
```

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::formats`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports.

Until 0.2.1, JSON and the typed records were `agent_core::json` and `agent_core::jmap`, XML and
the encodings `base::xml` and `base::encoding`. Those paths remain as re-exports of this crate's
modules, so existing code keeps compiling.

## What's new in 0.2.1

This crate is new and not part of 0.2.1: it comes with a later release, with the code that was
in `agent_core` and `base` until then. The
[changelog](https://docs.rs/crate/jflib-formats/latest/source/CHANGELOG.md) will say so.

## Documentation

- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#formats): every public
  item of `formats`, also on [docs.rs](https://docs.rs/jflib-formats)

## License

MIT or Apache-2.0, at your choice.
