# jflib-doc-io

The file formats of [JFLib](https://jflib.org)'s document model: DOCX in both directions, rich
copy and paste as RTF, HTML and a native fragment, and bibliographies as Word source lists, RIS
and BibTeX. It reads and writes the documents of
[`jflib-doc-core`](https://crates.io/crates/jflib-doc-core). It is plain Rust, without the
standard library and without a line of `unsafe` code.

The crate is deliberately conservative. Its DOCX and bibliography readers either represent a file
completely or refuse it with an error that names what they could not represent, and never hand
out a half-converted document; a Word document beyond that is kept as it is instead of being
converted. Every file is treated as untrusted input: sizes, element counts and nesting are
bounded, external links are kept as data and never followed, and nothing in a document is ever
run.

## DOCX

`save_docx` writes a `doc_core::Document` as a deterministic DOCX package: paragraphs with their
styles and formatting, tables, footnotes and endnotes in their standard parts, sources as Word's
bibliography XML and citations as fields with their display text. `load_docx` reads exactly this
profile back. A package with anything else, another part, a layout feature, a kind of field, is
refused with `Error::Unsupported` naming it. Full Word compatibility is not claimed.

Documents from Word and other programs go through `DocxFile`. It opens any conventional,
unencrypted DOCX up to 32 MiB (macro-enabled documents are refused) and tells you what it found.
If the document passes the editable profile, `document()` returns it as a model. If not, the file
stays as it is: you get a plain-text preview and the individual text runs of the main text,
headers, footers and notes that can be changed safely, each edit checked against the text it
replaces and undoable.
Saving without edits writes the original bytes exactly, down to compression, images, fields and
parts the crate does not know; after edits, everything but the changed text stays as it was.

```rust
use jflib::doc_io::{self, DocxFile};

let doc = doc_io::load_docx("documents://Report.docx")?;     // the model's own profile
doc_io::save_docx("documents://Report copy.docx", &doc)?;    // written in full, then put in place

let mut file = DocxFile::load("documents://Contract.docx")?; // any Word document
let hit = file.text_runs().iter().find(|r| r.text.contains("2025")).map(|r| (r.index, r.text.clone()));
if let Some((index, old)) = hit {
    file.replace_text(index, &old, &old.replace("2025", "2026"))?;
}
file.save("documents://Contract.docx", None)?;               // the rest keeps its bytes
```

Saving always encodes the whole file first and then writes it atomically through JFLib's
platform layer, so a failed save never damages the file that was there.

## Copy and paste

A rich selection travels in several forms at once. `encode_document_fragment` packs a
`doc_core::DocumentFragment` in a versioned native format that keeps sources, citations and note
bodies, for pasting into another JFLib document. `clipboard_rtf_encode` and
`clipboard_html_encode` give other programs the text with its resolved formatting, and
`clipboard_rtf_decode` and `clipboard_html_decode` turn what they offer back into paragraphs and
tables. The RTF reader takes a defined profile, direct formatting, tables, Unicode and the
single-byte code pages of Windows, and refuses pictures, lists and objects rather than losing
them; the HTML reader ignores active content and never requests an external resource.

## Bibliographies

`load_bibliography` and `save_bibliography` read and write a list of `doc_core::Source` records,
choosing the format by the file's extension: Word's source list XML, RIS or BibTeX. Each codec
has an explicit subset; a field it cannot represent faithfully aborts the whole import or export
with a diagnostic, so a list is never shortened without notice. `LibraryStore` keeps a
`doc_core::SourceLibrary` in a file of its own: it notices when another program changed the file
since it was opened, works under a lock, keeps the previous version as a backup and can restore
it after a review.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::doc_io`. All of JFLib's crates share one
version and are released together. File names may be paths of the operating system or the
names with a scheme that JFLib's platform layer knows, such as `documents://`.

## What's new in 0.2.0

Long formatted documents can be saved. `encode_docx`, and with it `save_docx`, checks its own
output with the XML reader, which stopped at 30000 elements and 4 MiB a part, a size some dozen
pages of formatted text reach. The parts that grow with a document's content are now written and
read up to 250000 elements and 8 MiB. The
[changelog](https://docs.rs/crate/jflib-doc-io/latest/source/CHANGELOG.md) says the same.

## Documentation

- [Document sources and notes](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images)
  and [the rich clipboard](https://docs.rs/jflib/latest/jflib/guide/index.html#5-widgets), in
  JFLib's guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#doc_io): every public
  item of `doc_io`, also on [docs.rs](https://docs.rs/jflib-doc-io)

## License

MIT or Apache-2.0, at your choice.
