# T2 – PaintCraft: Auswahl (selection model, selection tools, move tools)

The selection of every image, the Bearbeiten selection commands, "Auf Auswahl zuschneiden",
marching ants, the selection history, the tools Rechteckige Auswahl, Lasso-Auswahl, Elliptische
Auswahl, Zauberstab, Pixel verschieben and Auswahl verschieben.  SEL-001..020 and
TRANS-001..020 (copy / paste of the geometry through the clipboard is the clipboard module's
command; the text format is here).  Builds on both targets; `sel_basic.uis`, `sel_wand.uis`,
`sel_move.uis`, `sel_clip.uis` pass on Windows and Linux, also with `--verify-dirty` (and at
150 % / 300 % / 50 % zoom checks); asmcheck: 0 errors, 0 warnings in the T2 files (4 `assume`
annotations: loops bounded by n <= 64 / a nubs_hit index).

```
python build.py app PaintCraft --out build/t2
build/t2/win/PaintCraft.exe --script apps/PaintCraft/tests/sel_basic.uis [--verify-dirty]
wsl.exe -d Ubuntu-24.04 --cd <repo> -e ./build/t2/linux/PaintCraft --script apps/PaintCraft/tests/sel_move.uis
```

## 1. Files

| file | content |
|---|---|
| `inc/pn_sel.inc` | contract: DOC_SEL fields (SEL_*), POLY layout, API list, T2 ids (included by pn.inc) |
| `src/sel.s` | module (sel_init/states/frame/cmd, sel_size/offset), model, rendering, `sel_clip_row`, history hooks, commands, erase / fill / crop, marching ants, `sel_line`, clipboard text, test hooks `sel_diag_*` |
| `src/t_select.s` | Rechteckige Auswahl, Lasso-Auswahl, Elliptische Auswahl (+ `tsel_mode` shared with the wand) |
| `src/t_wand.s` | Zauberstab |
| `src/t_move.s` | Pixel verschieben, Auswahl verschieben |
| `tests/sel_basic.uis`, `sel_wand.uis`, `sel_move.uis`, `sel_clip.uis` | tests (generated from templates; coordinates: image px (x, y) = screen (240 + x, 148 + y) at 1280 × 800, 100 %) |

## 2. Model

* A selection is a list of **ops** `{POLY, mode SM_*, quality SQ_*}` kept in a per-image state
  record (`g_ss`, pointed to by `SEL_STATE`; freed by `sel_sweep` when the image is closed, so
  doc.s needs no hook).  "No selection" = no ops (every tool works everywhere).  A selection
  whose coverage is empty (moved off the canvas) clips everything away.
* **POLY** = refcounted polygon set (f32 image px, contours closed implicitly, even-odd fill).
  Ops share polygons with history snapshots (a step costs 16 bytes per op).  Constructors:
  rectangle (i32 grid), ellipse (5·π·√r points, error < 0.02 px), one contour (lasso), traced
  mask (magic wand: pixel-edge contours of `tm_trace`), transformed copy.
* **Rendering** (`sel_render`): the ops from the last SM_REPLACE op on are replayed into the
  coverage TMASK `SEL_MASK` of the image size — antialiased ops with `tm_raster_path`
  (combine op = SM_* = TM_*), pixelated ops rasterized alone, thresholded at 128 and combined
  (`tm_combine`).  A first op in Subtrahieren / Schneiden mode gives nothing.  Then
  `SEL_X0..Y1` = `tm_bbox`, the **outline** for the ants (the polygon itself for a single
  antialiased op, else the traced mask) with its box `SEL_OX0..OY1`, damage of the old and
  the new outline, `DOC_VER_SEL`++.  A combine whose result is empty ends as no selection.
* A **preview op** (a drag in progress) and a **pending transform** (move tools, f64 matrix)
  take part in rendering only; `sel_bake` turns the transform into new polygons (lossless:
  moving a selection off the image and back gives the same coverage).
* The mask follows image size changes by itself (sel_states re-renders when TM_W/H differ
  from DOC_W/H: Leinwandgröße, undo of a crop).

## 3. API for other modules (inc/pn_sel.inc)

Reading (any thread for `sel_clip_row`):
```
sel_clip_row(doc, x, y, n, out) -> eax SCR_NONE / SCR_PART / SCR_ALL   (T1 contract, n <= 64)
sel_active(doc)  sel_mask(doc) -> TMASK* | 0  sel_bbox(doc, &rect) -> eax 1 non-empty
sel_version(doc) -> DOC_VER_SEL   sel_size() / sel_offset() (active image, status bar)
```
`sel_clip_row` reads only `SEL_MASK` (tm_span); SCR_ALL without a selection, SCR_NONE for rows /
pixels outside the mask, `out` written only for SCR_PART (a run of materialised tiles may be
SCR_PART with 255s).  The mask is replaced / re-rendered only on the UI thread; background jobs
must not overlap selection changes (the tool framework commits live edits first).

Changing (UI thread; record `hist_push_sel(doc, name, icon)` BEFORE, except for previews):
```
sel_poly_new(pts, n) / sel_poly_rect(x0, y0, x1, y1) / sel_poly_ellipse(...) /
sel_poly_trace(tm) / sel_poly_xform(poly, m) / sel_poly_bbox / sel_poly_ref / sel_poly_unref
sel_combine(doc, poly, mode, qual)   sel_apply(doc, pts, n, mode, qual)
sel_set_rect(doc, x0, y0, x1, y1)    sel_set_ellipse(...)      sel_edit_last(doc, poly)
sel_set_preview(doc, poly | 0, mode, qual)   sel_clear(doc)
sel_set_xform(doc, m | 0)   sel_bake(doc)   sel_translate(doc, dx, dy)
```
Pixels / document (one history step each, eax 1 = step):
```
sel_erase(doc)        "Auswahlinhalt löschen" (Entf): alpha *= 1 - coverage (k_erase_row)
sel_fill(doc, argb)   "Auswahl füllen" (Rücktaste): argb Normal through the coverage
sel_crop(doc)         "Auf Auswahl zuschneiden" (Strg+Umschalt+X)
sel_crop_enabled()    CMD_CROP state for the Bild module
```
Erase / fill save only the 64 × 64 tiles the coverage touches (`hist_px_touch`) and write the
rows on the worker pool through `sel_clip_row`.  Crop: new surfaces of the selection bounds for
every layer; for a non-rectangular selection the pixels get alpha × coverage; one compound
step (hist_push_sel + hist_push_doc, the old surfaces released with hist_release /
lay_free_rec like A2's canvas-size reference); the selection moves along (it stays, translated,
as in paint.net: an elliptical crop keeps its ants); a rectangle over the whole image does
nothing.

Clipboard (for Auswahl kopieren / Auswahl einfügen ▸ Ersetzen / Hinzufügen / Ausschließen /
Schneiden / Umkehren, SEL-011/012 — the commands belong to the clipboard module):
```
sel_geom_text(doc) -> rax text (mem_alloc, NUL-terminated) | 0, rdx length
sel_geom_parse(text) -> rax POLY | 0      then hist_push_sel + sel_combine(doc, poly, mode, SQ_AA)
```
Text: `PaintCraft Auswahl 1`, then one line `p N x0 y0 ... ` per contour, coordinates in 1/256
image px.  A single antialiased op is exported as its polygon (lossless), anything else as
the traced coverage (pixel edges).  History names (paint.net 5.1 DE): "Auswahl einfügen:
Ersetzen" … see `PasteSelectionFromClipboardAction.HistoryMemento.*`.

Drawing: `sel_line(x0, y0, x1, y1, argb)` — render time, aliased 1 px line clipped to the gfx
clip, written directly (exact under every dirty rectangle).

## 4. Commands, history names

| command | key | history (paint.net 5.1 DE) | icon |
|---|---|---|---|
| Alles auswählen | Strg+A | Alles auswählen | IC_SELECT_ALL |
| Auswahl aufheben | Strg+D | Auswahl aufheben | IC_CLEAR_SELECTION |
| Auswahl umkehren | Strg+I | Auswahl umkehren (xor with the image rectangle) | IC_SELECT_ALL |
| Auswahl löschen | Entf | Auswahlinhalt löschen | IC_DELETE |
| Auswahl füllen | Rücktaste | Auswahl füllen (primary colour) | IC_DROP |
| Auf Auswahl zuschneiden | Strg+Umschalt+X | Auf Auswahl zuschneiden | IC_CROP |
| tools | | Rechteckige Auswahl, Lasso-Auswahl, Elliptische Auswahl, Zauberstab, Auswahl aufheben (click), Pixel verschieben, Auswahl verschieben, Auswahlgröße ändern, Auswahl drehen | |

Everything except Alles auswählen is disabled without a selection.  Selection steps are
`hist_push_sel` records (the blob = the op list; restore re-renders).

## 5. Tools

**Rechteckige / Elliptische Auswahl, Lasso** (`t_select.s`): options Auswahlmodus, Auswahl-
Qualität (T1 widgets), for rectangle / ellipse also Zeichenmodus (#2200 / #2400: Alle Größen,
Festes Seitenverhältnis, Feste Größe) with Breite / Höhe (#2210 / #2211, #2410 / #2411; ratio
4 : 3 and size 100 × 100 by default).  A drag shows the shape as preview op (ants and every
clip follow at once); the release records one step and combines.  Modes by mouse / keys like
paint.net: Strg + left = Hinzufügen, right = Subtrahieren, Strg + right = Invertieren ("xor"),
Alt + left = Schneiden; else the Auswahlmodus option.  Corners on the pixel grid (nearest
grid line); Umschalt = square / circle; a click without dragging deselects (Ersetzen).
After drawing, rectangle / ellipse keep 8 handles (paint.net 5) until the selection changes
otherwise or another tool is chosen; dragging a handle edits the last op in place (the same
history step).  Lasso: every pointer move point (sub-pixel), even-odd.

**Zauberstab** (`t_wand.s`): options Auswahlmodus, Füllungsmodus, Toleranz, Ebene / Bild.
`flood_fill_mask` on the current layer or the composite, Umschalt = global, the region traced to
a pixel-exact polygon op, step "Zauberstab".  The result stays editable: an origin handle can
be dragged and option changes re-run it (same step) until the selection, the pixels or the
tool change.

**Pixel verschieben / Auswahl verschieben** (`t_move.s`, TF_LIVE): a frame with 8
handles and the pivot on the selection bounds (without a selection Pixel verschieben
uses the whole layer and leaves the moved rectangle selected).  A press starts a
live session (Fertigstellen button, DF_LIVE); the session transform M accumulates:
drag inside = move (whole pixels), handle = scale (Umschalt at a corner: aspect ratio, Alt:
about the centre), outside or right button = rotate about the pivot (Umschalt: 15°), pivot
handle = move the pivot, arrow keys 1 px / Strg 10 px, buttons Horizontal / Vertikal
spiegeln (#2260 / #2261; Auswahl verschieben: #2360 / #2361), Probenahme
(#2250: Nächster Nachbar, Bilineare, Bikubisch, Bilinear mit Multisampling = RS_* of rs_affine).
Fertigstellen / Enter / another tool / another image / any document command (Auswahl
aufheben, Rückgängig…) commits ONE step; Esc / Strg+Z drop it; a session without a change
makes no step.  Pixels: the selected pixels are lifted into F (alpha × coverage); unless
Strg was held at the first press (copy) the hole gets alpha × (1 − original coverage); the
preview is the compose hook `mv_row` (hole, then P = rs_affine(F, M) blended Normal where P
is not transparent, so untouched pixels stay bit-exact); the commit runs `mv_row` on the
layer rows (compound: selection + pixels, "Pixel verschieben").  The selection
follows M as pending transform and is baked at the commit.

## 6. Marching ants

Overlay `sel_overlay` (registered in sel_init, before the tool overlay): every outline edge
as 1 px dashes (4 DIP black / 4 DIP white) written straight into the framebuffer, clipped per
edge (Liang–Barsky-like range for axis-aligned edges, per-pixel test else), so huge zooms and
huge outlines stay cheap.  Animation: timer TMR_ANTS every 100 ms (not with `--anim off`),
phase − 1, damage only the outline (edge by edge for up to 256 points, else its box).

## 7. Test hooks (diag.s block T2, ids 980–984, records #9980–#9984)

`cmd 980 x|y<<16` coverage probe → #9980 value 0..255, text none / part / all ·
`cmd 981 op`: 0 rectangle / 1 ellipse from `982` / `983` corners and `984 mode|qual<<8`
(step "Test"), 2 deselect without history, 3 fill (primary), 4 erase, 5 crop, 6 ants step,
7 invert, 8 count partially covered pixels → #9980, 9 geometry through the clipboard text ·
records: #9981 text bounds "x0,y0,x1,y1", value ops · #9982 text outline "contours,points",
value SEL_FLAGS · #9983 text handles of the editable rectangle / ellipse or the wand origin,
value DOC_VER_SEL · #9984 text M "tx,ty,sx%,sy%,deg", value live | frame << 1 | drag << 2.

## 8. Changes outside my files

* `inc/pn.inc`: `.include "pn_sel.inc"` (the one allowed line).
* `src/diag.s`: T2 routing block (980–984 → sel_diag_cmd, sel_diag_frame).
* `src/imgops.s`: a marked T2 forwarding block (CMD_CROP state + command → sel_crop) added
  while it was a stub; the imgops agent A5 owns the file now and routes CMD_CROP itself.
* `tests/skeleton.uis` line 85: "Alles auswählen" is enabled now.

## 9. Open issues / requests

* Chrome status bar: `sel_offset()` exists for a "Auswahl oben links" field (paint.net 5 shows
  offset + size); chrome.s shows the size only.
* View: CMD_ZOOM_SEL (Auf Auswahl zoomen) can use `sel_bbox`.
* History (A2): selection steps do not set DF_MODIFIED (paint.net marks the image changed).
* T1: at brush widths ≥ ~400 px the brush outline circle misses single pixels under
  `--verify-dirty` (1 px damage margin).
* Not done: the dynamic status text "Auswahl oben links … Fläche …" while dragging, the lasso's
  polygon mode, anisotropic resampling.  (The rotate cursor outside the frame exists since the
  custom cursors: cursors.s PNC_ROTATE.)
* Library: `tm_raster_path` is main-thread only, so selections render on the UI thread; on a
  4000 × 3000 image a whole-image op / trace / move step costs tens to ~200 ms (a script with
  select all, 2 × invert, wand, ellipse, a 10-step pixel move, commit and undo runs in 1.6 s).
