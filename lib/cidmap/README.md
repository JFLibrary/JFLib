# jflib-cidmap

The predefined CMaps of Adobe's Chinese, Japanese and Korean character collections, for
[JFLib](https://jflib.org), in plain Rust, without the standard library and without a line of
`unsafe` code. A CMap turns the character codes of a CID-keyed font into CIDs (the glyph numbers of
a collection); the tables here turn a collection's CIDs into Unicode text.

## What it has

- **The 59 CMaps of ISO 32000-1 Table 118:**
  - **Adobe-GB1** (simplified Chinese): GB-EUC, GBpc-EUC, GBK-EUC, GBKp-EUC, GBK2K, UniGB-UCS2 and
    UniGB-UTF16
  - **Adobe-CNS1** (traditional Chinese): B5pc, HKscs-B5, ETen-B5, ETenms-B5, CNS-EUC, UniCNS-UCS2
    and UniCNS-UTF16
  - **Adobe-Japan1** (Japanese): 83pv-RKSJ, 90ms-RKSJ, 90msp-RKSJ, 90pv-RKSJ, Add-RKSJ, EUC, Ext-RKSJ,
    H and V, UniJIS-UCS2, UniJIS-UCS2-HW and UniJIS-UTF16
  - **Adobe-Korea1** (Korean): KSC-EUC, KSCms-UHC, KSCms-UHC-HW, KSCpc-EUC, UniKS-UCS2 and
    UniKS-UTF16

  Each comes horizontal (-H) and, where the standard lists it, vertical (-V).
- **For each CMap:** its code space (codes of one to four bytes), its CID ranges, and its notdef
  ranges. What it takes in by `usecmap` is resolved.
- **For each collection:** CID → Unicode, Adobe's Adobe-GB1-UCS2, -CNS1-UCS2, -Japan1-UCS2 and
  -Korea1-UCS2. Every CID has text, a variant glyph that of its character, a few a sequence of
  characters.

Identity-H and Identity-V are not here: their CID is the two-byte code, with no collection.

## How it is used

```rust
let m = cidmap::predefined(b"90ms-RKSJ-H").unwrap();
let code = [0x82, 0xA0]; // Shift-JIS for あ
let len = m.code_len(&code);
let cid = m.cid(u32::from_be_bytes([0, 0, code[0], code[1]]), len as u8).unwrap();
let text = cidmap::to_unicode(m.collection); // unpack once, keep it
assert_eq!(text.get(cid), Some("あ"));
```

[`jflib-pdf`](https://crates.io/crates/jflib-pdf) reads Chinese, Japanese and Korean fonts with
it: the codes and CIDs of a font's predefined CMap and, without a ToUnicode, its text through the
collection's table. Its `Collection::lang` picks a system font for a font that isn't embedded.

## The data

`data/cidmap.bin` (261 KB) is one zstd frame per collection, unpacked when a CMap or table of it
is asked for. `tools/cidmap_gen.py` makes it from Adobe's files:

- the CMaps of [adobe-type-tools/cmap-resources](https://github.com/adobe-type-tools/cmap-resources)
  at a pinned commit
- the CID → Unicode tables of the poppler-data 0.4.12 release (Adobe's, 2023), which that
  repository no longer holds

`--check` compares every code and CID with Adobe's files.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::cidmap`. All of JFLib's crates share one
version and are released together.

## What's new in 0.2.1

This crate is new and not part of 0.2.1: it comes with the next release. The
[changelog](https://docs.rs/crate/jflib-cidmap/latest/source/CHANGELOG.md) will say so.

## Documentation

- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#cidmap), also on
  [docs.rs](https://docs.rs/jflib-cidmap)

## License

The code: MIT or Apache-2.0, at your choice. The data (`data/cidmap.bin`) comes from Adobe's
files: BSD-3-Clause, Copyright 1990-2023 Adobe
([licenses/adobe-cmap-resources-LICENSE.md](https://github.com/JFLibrary/JFLib/blob/main/lib/cidmap/licenses/adobe-cmap-resources-LICENSE.md)).
