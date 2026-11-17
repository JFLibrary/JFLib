# Changelog of jflib-diagram-io

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- The crate, for Visio's file formats (the VSDX family) and diagram exports; the plan is
  [apps/JFDiagrammer/PLAN.md](https://github.com/JFLibrary/JFLib/blob/main/apps/JFDiagrammer/PLAN.md).
- Reading the VSDX family (stage JD3): `vsdx::read(bytes, &Limits)` takes `.vsdx`, `.vsdm`,
  `.vstx`, `.vstm`, `.vssx` and `.vssm` (`Format`, by the main part's content type or the
  extension) into a `diagram_core::Document`:
  - DocumentSettings, colours, fonts, styles and the document sheet; pages and masters with
    their sheets, shapes (groups, foreign objects, deleted subshapes), texts with their marks and
    `Connect` entries; master icons; themes kept raw
  - `windows.xml`, properties, pictures, comments, data recordsets, solutions and unknown parts
    as `Document::parts`, every relationship in `Document::rels`
  - unknown attributes, elements and text in the `Extras` where they stood; nothing recomputed
    or rerouted
  - the macro project of a macro-enabled file is kept and never run; one in a file that may not
    carry macros is `Error::Macros`
  - every file is untrusted: `Limits::VSDX` bounds the ZIP container (100000 parts, 512 MiB a
    part, 2 GiB in all, no part denser than 200:1), XML nesting, attributes and namespace
    declarations, shapes, cells, rows, sections, pages/masters/styles and every other item kept,
    formula and text length; an XML reader of its own without recursion (mixed content, no
    DOCTYPE, no entities); cycles of styles, masters and background pages are `Error::Cycle`;
    references that name nothing are kept and reported (`Warning`)
- Writing the VSDX family (stage JD5): `vsdx::write(&doc, &WriteOptions)` writes a
  `diagram_core::Document` as any of the six files (`WriteOptions::format`: a drawing as a
  template and so on, EX-11; the DEFLATE level; the ZIP bounds, `Limits::VSDX.zip` by default):
  - document.xml, pages.xml, masters.xml and their parts, the themes and every part kept as
    bytes (unknown packages pass through)
  - the unknown attributes, elements and text of every element at their place; every
    relationship with its id (new ones `rIdN`, the main part's of Visio's own type); deleted
    subshapes with ID 4294967295; children in [MS-VSDX]'s order; numbers in the shortest form
    that reads back the same
  - deterministic: a file read and written unchanged reads back as the same model and writes
    the same bytes again
  - the macro project of a `.vsdm`, `.vstm` or `.vssm` goes back unchanged and is never run;
    into a format without macros it is `WriteError::Macros`
  - no recursion, no panic: a part kept as bytes without a valid name or content type is
    `WriteError::Part`, raw XML from the model that is not well-formed where it stands
    `WriteError::Xml`
- New documents of JFLib's own (decision V3): `vsdx::new_document(kind, Measure::Metric |
  Measure::Us)` makes a drawing, template or stencil with own styles (No Style, Normal,
  Connector, Guide, Text Only in NetLab colours and Inter), the document sheet, the 24 colour
  indexes [MS-VSDX] fixes, one page (A4 or Letter landscape; a stencil's without contents) and
  the own DrawingML theme "NetLab" from the NetLab style's palette; nothing comes from Visio's
  files

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
