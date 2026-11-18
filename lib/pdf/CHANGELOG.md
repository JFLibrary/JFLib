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

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
