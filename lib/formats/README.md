# jflib-formats

Data formats as text for [JFLib](https://jflib.org): JSON, typed JSON records, a bounded XML
subset and the text encodings of the web and of old files, in plain Rust, without the standard library, without a line
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

**XML** (`formats::xml`) has one reader in two kinds, and every XML reader of JFLib is one of them:

- `Reader` is strict: well-formed XML 1.0 with namespaces, as a pull reader of events (start
  and end tags, text, CDATA, comments, processing instructions), each name with the URI its
  prefix is bound to, errors with their byte offset, and `skip` for the raw text of an element a
  program keeps as written. A DOCTYPE is refused, so are entities other than the five predefined
  ones; nesting, attributes per element and namespace bindings are bounded, and nothing recurses.
  The VSDX reader of `jflib-diagram-io` runs on it.
- `Lenient` is forgiving, for files from editors (SVG, fontconfig, theme parts): nothing is an
  error, a broken tag or reference is read as far as it goes, and a DOCTYPE's internal entities
  are expanded within a budget of bytes, so no "billion laughs" can grow. It gives tokens, and
  the program builds the tree it needs (`jflib-img-svg`, `jflib-text`, `jflib-diagram-core`).
- `parse` builds a tree on the strict reader for interchange formats such as OPC packages, DOCX
  parts and manifests: elements that hold either elements or text, names expanded to their
  namespaces. What the subset cannot represent losslessly is refused as `Unsupported` instead of
  being read halfway: a DTD, comments, CDATA sections, processing instructions and mixed content.
  `escape` writes text and attribute values that survive the reader's newline normalization.

**Encodings** (`formats::encoding`) decode bytes to a `String` and back. `decode` and `encode` are
the strict Unicode interchange: UTF-8 with or without a byte-order mark and UTF-16 in both byte
orders, told apart by the mark; nothing is replaced or guessed, invalid sequences and UTF-32 are
errors. For text from the web, from mail and from old files there is every encoding of the
[WHATWG Encoding Standard](https://encoding.spec.whatwg.org/), as browsers decode and encode it:

- the single-byte character sets (`Charset`): windows-1252 and the other Windows pages,
  ISO-8859-2 to -16, KOI8-R and KOI8-U, macintosh, IBM866, x-mac-cyrillic; besides them the DOS
  code pages of old files (IBM437, 737, 775, 850 to 869) and an exact ISO-8859-1
- the multi-byte encodings of Chinese, Japanese and Korean (`MultiByte`): Shift_JIS (Windows' code
  page 932), EUC-JP, ISO-2022-JP (mail), GBK, gb18030 (all of Unicode but U+E5E5, GB18030-2022), Big5 with
  HKSCS-2008 and EUC-KR (Windows' 949, every Hangul syllable). `encode` refuses a character the
  encoding lacks (`Error::Unmappable`), `encode_html` writes it as a character reference, as
  browsers submit forms.

`label` reads the standard's labels as browsers do ("latin1" is windows-1252, "sjis" Shift_JIS,
"gb2312" GBK), `from_code_page` takes Windows code page numbers (what RTF's `\ansicpg` and DXF's
`$DWGCODEPAGE` give), and `decode_label` is the standard's decode: a byte-order mark wins over the
label, malformed input becomes U+FFFD. A `Decoder` takes input in pieces, a character cut between
two of them included, and gives the same text as decoding all of it at once. Every decoder and
encoder gives the same output as encoding_rs, Firefox's implementation of the standard, on every
character and every two-byte sequence (the repository's `tools/encoding_check.py`).

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

```rust
use jflib::formats::encoding::{self, Charset, MultiByte};

let text = encoding::decode_label("iso-8859-2", &bytes, 1 << 20)?;   // a mail's charset=
let dos = Charset::from_code_page(850).map(|c| c.decode(&name));     // a ZIP or DBF from DOS
let back = Charset::Windows1252.encode("€100")?;                     // Err: a character it lacks
let sjis = MultiByte::ShiftJis.encode("日本語")?;                      // 93 FA 96 7B 8C EA

let mut d = encoding::Decoder::new(encoding::label("gb18030").unwrap());
let mut text = String::new();
d.decode(&first_piece, false, &mut text);                            // a character may be cut here
d.decode(&last_piece, true, &mut text);
```

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

MIT or Apache-2.0, at your choice. The labels of the WHATWG Encoding Standard and the entries of its
indexes in `charsets.rs`, `cjk.rs` and `cjk.bin` are under the standard's BSD 3-Clause License (Copyright ©
WHATWG: Apple, Google, Mozilla, Microsoft); the text is in the crate's
[`licenses/`](https://github.com/JFLibrary/JFLib/tree/main/lib/formats/licenses) folder.
