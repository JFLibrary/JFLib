# T4 – Linie / Kurve and Formen

Files: `src/t_shapes.s` (both tools, `tshp_init` / `tshp_cmd`), `src/shapes_lib.s` (shape
library, XAML loader, gallery), library `gfx/dash.s` + `gfx/pathdata.s`
(`legacy/ASMLIB/docs/modules/DASH_PATHDATA.md`), tests `tests/tools_line.uis`,
`tests/tools_shapes.uis`, fixtures `tests/fixtures/shapes/*.xaml`.

## Behaviour

Both tools are live tools (TF_LIVE, TOOLS.md §3): the drag draws, the result stays editable
until Fertigstellen / Enter / tool or image switch / a document command → ONE history step
("Linie / Kurve", icon IC_CURVE; "Formen", IC_SHAPES); Esc / Strg+Z drop it (no step).  A
press away from the live object finishes it and starts a new one.  Rendering goes only
through the tool layer (preview == result): the centre line is stroked with `dash_stroke`
(width = Pinselgröße, Linienart, caps) and filled into a coverage mask with
`tm_raster_path`; the fill uses the shape's own rule.  Antialiasing off = threshold 128 on
the exact coverage with a 1/64 px offset to +x/+y (a top-left rule, so a w px line through
pixel centres is exactly w px).  Selection clip, Füllmethode (incl. Überschreiben),
Fülleffekt (hatch) come from tl_row.

**Linie / Kurve** (O): drag = straight line (Umschalt 15° steps, Alt centred on the press
point), 4 control points at 0, ⅓, ⅔, 1.  Dragging a point bends the line: Spline-Pfad
(uniform Catmull-Rom = cardinal spline with tension ½, through all points); the right
button switches the line to Bézierpfad (P1, P2 become controls; grey arms are drawn).  The
square handle after the end (22 DIP along P0→P3) moves the line; right-dragging it rotates
the line about the centroid (Umschalt 15°).  Left = primary colour, right = secondary.
Options: Linientyp (Kurvenzug (Spline) / Bézierpfad), Pinselgröße, Stil:
Linienanfang / Linienart / Linienende (Flach, Pfeil, Ausgefüllter Pfeil, Abgerundet;
Durchgezogen, Striche, Punktiert, Strich, Punkt, Strich-Punkt-Punkt), Fülleffekt,
Antialiasing, Füllmethode.  Joins round.

**Formen** (O O): drag = the chosen shape fitted into the rectangle between the pixel
corners under the pointer (Umschalt: equal sides, Alt: centred).  Then: 8 handles resize
(Umschalt at a corner keeps the ratio, Alt about the centre, dragging an edge over the
opposite one mirrors), a drag inside moves (whole pixels), a drag in the 24 DIP band
around the frame or a right drag inside rotates about the centre (Umschalt 15°),
Horizontal / Vertikal spiegeln buttons mirror; arrow keys move 1 px (Strg 10 px).  Model:
local frame box B (any corner order = mirror) + rigid transform M (rotation, translation);
the shape geometry is mapped from its bounds onto B, then M.  Formfüllmodus: Umriss
zeichnen (primary), Nur Füllung (primary), Füllung und Umriss
(outline primary over fill secondary); the right button swaps the colours.  The combined
mode renders a straight source S and a union coverage U with S·U = outline over fill
(TLK_SOURCE), so every blend mode applies to the shape as a whole.  Ecke Größe (px,
default 20) for Abgerundetes Rechteck.  Joins mitred (limit 10), caps flat.

## Shape library (29 built-in, paint.net 5.1 names)

Grundlegende Formen: Rechteck, Abgerundetes Rechteck (radius option), Ellipse, Raute,
Parallelogramm, Trapez, Dreieck, Rechtwinkliges Dreieck · Polygone und Sterne: Fünfeck,
Sechseck, Heptagon, Achteck, Stern (3 / 4 / 5 / 6 Zacken) · Pfeile: Pfeil,
Gekerbter Pfeil, Richtungspfeil, Chevron Pfeil · Sprechblasen: Sprechblase (eckig),
Sprechblase (abgerundet), Sprechblase (oval), Sprechblase (Wolke) · Symbole: Häkchen, Multiplizieren,
Zahnrad, Herz, Blitz.  Geometry = path data in a 0..100 box (ported from
apps/paint/src/shapes_geom.s where it existed, the rest drawn), fitted by its tight bounds.
Gallery: "Art der Form" drop-down (ID_SHAPE_LIST), flyout with a heading per category and
8 cells per row (ID_SHAPE_LIST + 1 + i, icon drawn at render time).

**Plugin shapes** (P1, M8; docs/PLUGINS.md): shapes of plugin libraries arrive as one XAML
document per plugin through `shl_add_xaml(buf, len, name)`; `shl_load_dir` calls
`plug_shapes_feed` after the folder, so they survive a reload and count as custom shapes.

**Custom shapes** (SHAPE-020): every `*.xaml` of `<exe folder>/Shapes` (folder of argv[0];
sorted by name) at start-up.  Accepted: `<[prefix:]SimpleGeometryShape DisplayName="…"
Geometry="[F0|F1] path data"/>` or the element with content: all `Figures="…"` attributes
inside (PathGeometry, GeometryGroup …) are appended, `FillRule="EvenOdd"` switches the rule.
Several shapes per file are possible; entities (&amp; &lt; &gt; &quot; &apos; &#N; &#xH;)
in DisplayName are decoded (missing: the file name).  A file with a parse error, a shape
without area or no shape at all is skipped as a whole; the gallery shows "N
eigene Formen; nicht geladen: a.xaml, …".  Limits: 1 MB per file, 128 files,
256 shapes, names 63 bytes.

## Test hooks / records

`cmd 795 sel << 16 | value`: 0 Linientyp, 1 Linienart, 2 Linienanfang, 3 Linienende (menu
indices), 4 shape, 5 Formfüllmodus, 6 radius, 7 mirror (0 H, 1 V), 8 load custom shapes
(0 = `apps/PaintCraft/tests/fixtures/shapes`, 1 = exe folder again, 2 = folder in the
environment variable PDN_SHAPES_DIR), 9 rotate by (i16) degrees.  `tools/pathdata_check.py
--xaml N` loads N mutated XAML files that way and draws every shape (robustness).  Records (drawn with the options): #524544 state ("L S|B x,y ×4" / "F shape x0,y0
x1,y1 degrees", value = live), #524545 custom shape status (value = shapes), #524546
current shape (value = index), #524547 "dash,cap0,cap1,mode,radius,ctype".

## Unverified / deviations

* History names: the tool names "Linie / Kurve" / "Formen" (paint.net 5 shows its own
  "{0} zeichnen / abschließen" chain while editing; here one step).
* Dash proportions (3 1 / 1 1 …, flat dash ends), arrow head size (L = 3w + 4, half width
  L/2), default corner radius 20, Nur Füllung = primary colour, the rotation
  band and the move handle position are own choices.
* Geometry of Heptagon, Achteck, Stern (3 Zacken), Gekerbter Pfeil, Chevron,
  Sprechblasen, Häkchen, Multiplizieren, Zahnrad drawn after the paint.net icons.
* Custom shapes: only path geometry (no EllipseGeometry / RectangleGeometry children).
* raster.s adds partial coverages of overlapping contours per pixel (arrow junctions are
  a little darker at single pixels).
* The tool row of Abgerundetes Rechteck (with Ecke Größe) is wider than 1280 px: at the
  default window size Füllmethode, the mirror buttons and Fertigstellen are clipped
  (Enter still finishes).  Linienart / Formfüllmodus / caps are compact (drawn pattern,
  icons) to fit the other rows.
