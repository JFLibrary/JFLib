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
  - a document without `windows.xml` gets one of its own: desktop Visio refuses a file without
    it (error 271), although [MS-VSDX] calls the part unused
  - the macro project of a `.vsdm`, `.vstm` or `.vssm` goes back unchanged and is never run;
    into a format without macros it is `WriteError::Macros`
  - no recursion, no panic: a part kept as bytes without a valid name or content type is
    `WriteError::Part`, raw XML from the model that is not well-formed where it stands
    `WriteError::Xml`
- New documents of JFLib's own (decision V3): `vsdx::new_document(kind, Measure::Metric |
  Measure::Us)` makes a drawing, template or stencil with own styles (No Style, Normal,
  Connector, Guide, Text Only in NetLab colours and Inter), the document sheet, the 24 colour
  indexes [MS-VSDX] fixes, one page (A4 or Letter landscape; a stencil's without contents), an
  empty `windows.xml` and the own DrawingML theme "NetLab" from the NetLab style's palette;
  nothing comes from Visio's files
- The stencil build tool (stage JD9): `stencil::build(&files, Measure)` compiles the text
  sources of a stencil (a `stencil.txt`, one `.master` file per master, optional SVG drawings;
  the format is apps/JFDiagrammer/stencils/README.md) into a stencil document,
  `stencil::build_vssx` also into `.vssx` bytes; `Built { doc, stencil, file, category,
  measure }`, `Built::vssx_name`, `SourceError { file, line, message }`, `CATEGORIES`:
  - header lines, ShapeSheet sections, rows with continuation lines, `;`-separated cells,
    language suffixes (`.de` `.en` `.es` `.it` `.fr`, all five or an error) and measure
    suffixes (`.metric`, `.us`); section, row and cell names checked against what each section
    holds
  - constants as values with their unit, everything else as formulas whose values the formula
    engine of jflib-diagram-core computes; an unknown function, a reference into nothing, an
    error value or a cycle is an error naming the source line
  - the Master attributes Visio writes (Name and Prompt in the build language, IsCustomName(U),
    IconSize, AlignName, MatchByName, IconUpdate, PatternFlags, Hidden, MasterType),
    name-based BaseID and UniqueID (SHA-1, RFC 4122 version 5; the UniqueID follows the
    content), the master page sized to the shape at 1:1, `ShapeKeywords`, and JFDiagrammer's
    JFD_ User cells with the names, QuickInfos, keywords, texts and translated cells in five
    languages (`diagram_core::instance` reads them)
  - icons rendered from the shapes themselves: 32x32 ICO with 16 colours and a mask, the kind
    Visio writes
  - SVG drawings as Geometry: path (M L H V C S Q T A Z), rect, circle, ellipse, line,
    polyline, polygon, g, transforms; lines as `LineTo`, curves as exact cubic `NURBSTo` rows,
    arcs in pieces of at most 90 degrees; one format on the master shape, several as a group
    of full-size subshapes
  - the same sources give the same bytes; no panic, bounded counts, no recursion
- (JD10) The stencil build tool takes `hidden: true` in a master's header (the master's `Hidden="1"`: in the file, not in the Shapes panel; JFDiagrammer's flowchart frame and lists)
- (JD13) The stencil build tool builds 1-D masters: a master whose `[Shape]` sets `BeginX`, `BeginY`, `EndX`, `EndY` (lengths from 0) gets Visio's 1-D formulas where it writes none (`PinX = GUARD((BeginX+EndX)/2)`, `Width = GUARD(SQRT(...))`, `Angle = GUARD(ATAN2(...))`, `LocPin = GUARD(...)`), its master page is `BeginX+EndX` by `BeginY+EndY`, `Height` may be 0 or a formula; 2-D masters are written as before
- (JD13) `stencil::file_name` / `parse_file_name` and `FILE_PREFIX`: JFDiagrammer's own stencils are named `jfd_<stem>_m.vssx` / `_u.vssx` (`Built::vssx_name`, the stencil windows of templates), so that Visio never opens one of its own stencils of the same name (ARROWS_M.VSSX) for ours and Visio's names are never taken for ours
- (JD13) The template build tool `template` (apps/JFDiagrammer/templates/README.md): `build(&files, Measure, &stencils)` and `build_vstx` compile `template.txt`, `*.starter` and `rules.txt` into the forms of a template (`BuiltTemplate`: the blank one and one per starter diagram) and `.vstx` bytes, with the stencil windows in `windows.xml` as Visio writes them, the rule set in `visio/validation.xml` and the catalogue's JFD_ cells; `stencil_windows`, `set_stencil_windows`, `info`, `registers` and `to_drawing` for templates at run time; unit tests in the crate (`cargo test -p jflib-diagram-io --lib`)

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
- Stencil sources (stage JD13): the master keys `include:` (sections shared by many masters from .inc files, merged under the master's own cells, errors name the included file), `scale: 1:N` (a master page drawn to scale, line weights on paper) and `size:` (the size of a master whose Width or Height is a formula); apps/JFDiagrammer/stencils/README.md
- Reading Visio's XML formats of 2003-2010 (stage JD16 of apps/JFDiagrammer/PLAN.md): `vdx::read(bytes, kind, &Limits)` takes .vdx (drawing), .vsx (stencil) and .vtx (template), DatadiagramML after Microsoft's public schema reference, into the same `diagram_core::Document` the VSDX reader fills, so that `vsdx::write` saves it as .vsdx, .vssx or .vstx: cells with formula, value, unit and error (nothing computed), single-row sections flattened, Char/Para/Prop/Act/SmartTagDef as Character/Paragraph/Property/Actions/ActionTag rows (named by NameU, Name or Row_<ID>), Geom as Geometry sections, FaceName IDs of the font cells as font names, base64 pictures as parts (PNG, JPEG, GIF, TIFF, BMP, EMF, WMF; a bitmap without file header gets one), masters with icons, pages, connects, DocumentProperties as docProps, Windows as windows.xml, EventList and HeaderFooter, VBProjectData as vbaProject.bin (a .vsdm, never run); tab stops, solution XML, data connections and other namespaces left out (`Warning::LeftOut`). Bounded like a VSDX file by `vsdx::Limits` (the file's size by `zip.part_bytes`), no recursion; `vdx::kind_of`, `vdx::sniff`
- Document properties and the window state (stage JD16, DF-09, DF-10): `props::read(&doc)` / `props::write(&mut doc, &Props)` read and write docProps core.xml, app.xml and custom.xml (title, subject, author, keywords, comments, category, language, dates, template, company, manager, hyperlink base, custom properties; a part is only touched when one of its entries changed, app.xml keeps everything else); `windows::drawing_window` / `set_drawing_window` the drawing window of windows.xml (page, zoom, centre, rulers, grid, guides, page breaks; the other windows stay as written); `vsdx::remove_macros` takes a macro project out (for saving as .vsdx after the user's yes), and the writer hangs a vbaProject part without relationship on the main part

### Changed

- (JD11) New documents are themed: a style "Theme" (ID 5) with THEMEVAL() cells, "Normal" based on it, "Connector" on the connector scheme's style 1, "Text Only" with the dark font style 1 and Visio's text contrast rule; the page shows the design NetLab, variant 1 (its theme part now carries Visio's extensions); the root style holds the NetLab look without a design
- The stencil tool's SVG import keeps the document's order as JFNetworker's tools/vsym.py does: consecutive elements of one format share a subshape, a format that comes back later is a subshape of its own (before, every format was one layer where it first appeared and later parts vanished under the body); overlapping filled parts of one format get subshapes of their own (no holes from the even-odd rule); sub-paths of one kind share a Geometry section; an SVG group gets SelectMode 0; SVG masters get new UniqueIDs

### Fixed

- The stencil tool rendered each master's icon after a recalculation of the whole stencil: a stencil of 150 masters took minutes; the icon instance now inherits the values computed for the master


### Fixed

- (JD13) Rows of a shape's Geometry and Connection sections are written with their type `T`, as Visio writes them: a row that overrides an inherited one (a resized instance of a master) gets the inherited row's type, a Connection row without one `T='Connection'`. Without it Office's previewer (`VPREVIEW.EXE`, the preview pane of Windows Explorer) hung, and Explorer with it, and desktop Visio passed over the rows and drew such shapes with the master's geometry
