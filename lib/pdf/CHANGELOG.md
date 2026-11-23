# Changelog of jflib-pdf

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- The crate: a vector PDF writer (decision V5 of
  [apps/JFDiagrammer/PLAN.md](https://github.com/JFLibrary/JFLib/blob/main/apps/JFDiagrammer/PLAN.md)).
  `Document` holds pages of their own size, fonts, images, outlines and the document information
  and writes PDF 1.7 with a classic xref table, the same bytes for the same document.
- `Canvas`, one per page: origin top left with y down or PDF's bottom left; paths of lines,
  cubic and quadratic curves, rectangles, rounded rectangles, ellipses and arcs; fills with the
  non-zero or even-odd rule, strokes with width, dashes, caps, joins and miter limit; transforms,
  clipping, alpha for fills and strokes, RGB colours; state written only when a paint needs it.
  `Path` keeps a path to draw it again.
- `Image`: JPEG files passed through (DCTDecode, progressive and Adobe CMYK too); gray, RGB, RGBA
  and `gfx::Surface` pixels with FlateDecode and PNG predictors, their alpha as a soft mask.
- Text in embedded font subsets: TrueType and OpenType (glyf, name-keyed and CID-keyed CFF) as
  Type0 fonts with Identity-H encoding, widths from hmtx and a ToUnicode CMap; lines shaped with
  JFLib's shaper (kerning, ligatures, marks, Arabic, fallback fonts) and placed glyph by glyph
  (`Canvas::text`, `text_fallback`, `glyphs`), filled, stroked or invisible. Fonts whose OS/2
  `fsType` forbids embedding are refused.
- Links to URIs and to places in the document (`Dest`), nested outlines, the document
  information with XMP metadata, the document's language.
- Gradients: `Shading` (`ShadingKind::Axial` / `Radial`, PDF shading types 2 and 3) with any
  number of stops (sorted, clamped, NaN dropped; two at one place make a hard edge) and alpha per
  stop, painted with `Canvas::shade` (the whole clip) or `Canvas::fill_shading` (a path). One
  alpha for all stops is the fill alpha, varying alpha a luminosity soft mask; degenerate
  geometry paints the last stop's colour.
- Hatches: `Hatch` and `Canvas::fill_hatch` fill a path with parallel lines (angle, spacing,
  width, colour, alpha, a point on a line), drawn as clipped strokes rather than tiling patterns,
  which pdfium shows with seams.
- A PDF reader, pdf::read: Document::open / open_with(bytes, &Options { password, max_stream }) for PDF 1.0 to 2.0 with classic cross-reference tables and cross-reference streams, object streams, incremental updates and hybrid files; a damaged, cut or tableless file is rebuilt from its objects (was_repaired). The standard security handler: RC4 40 to 128 bit, AES-128 and AES-256 (revisions 2 to 6) with the user or the owner password (is_encrypted, encryption_revision, permissions, opened_as_owner). get / resolve and the objects (Object, Dict, Name, Ref, Stream, text_string); decode and decode_until_image with FlateDecode (raw DEFLATE too, damaged data up to the damage), LZWDecode, ASCIIHexDecode, ASCII85Decode, RunLengthDecode and the PNG and TIFF predictors. Pages with their boxes, rotation and inherited resources (page, pages, contents, page_label); content streams as operators (Ops). The text: page_text gives every glyph with its characters, position, advance, width and size, text(i) the page in reading order as pdftotext -raw lays it out (ToUnicode, the base encodings and Differences through the Adobe Glyph List, Type 0 fonts with their CMaps, Type 3 fonts, the 14 standard fonts with their widths, form XObjects, invisible text marked, Hebrew and Arabic in reading order, fake bold once). info, metadata, language, outlines, named_dest, annotations and links with their Action, fields (the interactive form's values, options and widgets; signature fields as stated, not verified), attachments (embedded files with name, MIME type, size, relationship). Hostile input gives an error, never a panic or a hang; every decoded stream is at most max_stream bytes. About as fast as pdftotext, with a third of its memory

### Changed

- jflib-pdf depends on jflib-crypto (MD5, SHA-2 and AES for the reader's decryption); its license is (MIT OR Apache-2.0) AND BSD-3-Clause AND OFL-1.1 for the Adobe Glyph List and the widths of the Liberation fonts in the reader's tables (lib/pdf/licenses/)

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD

### Changed

- The zlib of the streams comes from jflib-compress instead of img_core; the crate no longer depends on img_core.

