# A1 – PaintCraft document core (M1, Strang A)

Multi-document model, layers, compositing, canvas view, rulers, image tabs, file open/save,
"Neu" dialog, Ebenen basics, test hooks.  Builds on both targets; `docs.uis`, `view.uis`,
`files.uis`, `layers_basic.uis` and `skeleton.uis` pass on Windows and Linux (WSLg), also with
`--verify-dirty`; asmcheck: 0 errors in the A1 files (warnings: count / sentinel bounded table
walks only).

```
python build.py app PaintCraft --out build/a1
build/a1/win/PaintCraft.exe --script apps/PaintCraft/tests/docs.uis [--verify-dirty]
build/a1/win/PaintCraft.exe --open apps/PaintCraft/tests/fixtures/a1_rgba.png --script apps/PaintCraft/tests/files.uis
wsl.exe -d Ubuntu-24.04 --cd <repo> -e ./build/a1/linux/PaintCraft --script apps/PaintCraft/tests/view.uis
```

## 1. Files

| file | content |
|---|---|
| `inc/pn_doc.inc` | DOC / LAYER records (contract), DV_* (view block), CO_* (composite block), RP_* (ruler record), DNF_*, LY_META_LEN (new, in the reserved gap) |
| `src/doc.s` | document list (DOC_MAX = 64), active document, "Unbenannt n", paths, change marking, tab order |
| `src/layer.s` | layer records, Ebenen menu (lay_init/states/frame/cmd) |
| `src/compose.s` | composite (tiles, worker threads, alias fast path, tool hook), display mips, thumbnails |
| `src/view.s` | canvas view (replaces `canvas_stub.s`, deleted), transforms, zoom / pan / scroll, rendering |
| `src/rulers.s` | ruler bands (sub-module of view.s) |
| `src/tabs.s` | image list in the title row |
| `src/fileio.s` | Datei menu, questions, recent files, window title, error bar |
| `src/dlg_new.s` | "Neu" dialog window (sub-module of fileio.s) |
| `src/diag.s` | A1 test hook block (900–929, widgets 9900–9929); A2's routing kept |
| `src/tool.s` | `tool_input` no-op at the end, block "A1 temporary" (the tool agent replaces it) |
| `tests/docs.uis`, `view.uis`, `files.uis`, `layers_basic.uis` | tests; `tests/fixtures/` (+ `make_fixtures.py`) |

Integrator files touched (minimal): `chrome.s` `unit_str` uses `view_dpi100()` (the image
resolution) instead of a fixed 96 dpi for Zoll / Zentimeter; `tests/skeleton.uis` where
behaviour intentionally changed (file / layer commands enabled, Strg+N / Strg+O / Strg+S /
Strg+Umschalt+N removed from the "disabled accelerators" line).  `main.s` needs no change
(A2 already renamed the history module init to `hist_modinit`).

## 2. Documents (doc.s)

```
doc_count() -> eax          doc_cur() -> eax (-1 = none)     doc_at(i) -> rax DOC* | 0
doc_active() -> rax         doc_index(doc) -> eax (-1)       doc_set_active(doc)
doc_size(i) -> eax w, edx h doc_name(i) -> rax               doc_dirty(i)   doc_version(i)
doc_new(w, h, bg_argb, flags) -> rax DOC* | 0     one layer "Hintergrund" (LYF_BG) filled with
                            bg (0 = transparent); DNF_EMPTY: no layer; DNF_NOACTIVATE
doc_close(doc)              hist_free first, then everything; the tab to the right becomes active
doc_layer(doc, i) -> rax    (clamped)      doc_cur_layer(doc) -> rax
doc_mark_px(doc, i, x0, y0, x1, y1)   pixels of layer i (-1 any) changed ([x0,x1) x [y0,y1)):
                            LY_VER / DOC_VER_PX++, DF_MODIFIED, composite + mips + view damage
doc_mark_layers(doc)        structure / properties: DOC_VER_LAY++, DF_MODIFIED, full recomposite
doc_set_modified(doc, on)   doc_resized(doc) (DOC_W/H changed: composite rebuilt)
doc_set_path(doc, path)     DOC_PATH + DOC_NAME (file name), clears DF_UNTITLED / DOC_SERIAL
doc_find_path(path) -> rax  open document with that file (ASCII case / separator insensitive)
doc_move(from, to)          tab order        pn_strdup(s) -> rax
```
Names: new images get the smallest free n: "Unbenannt", "Unbenannt 2", …  A document starts
with DPU inch / 96.0.  The composite size follows DOC_W/H automatically (comp_update detects a
new size, e.g. after an undo of a resize).

## 3. Layers (layer.s)

```
lay_new(doc, index, name) -> rax LAYER*   transparent, name 0 = "Ebene n" (n = layer count)
lay_add_sf(doc, index, sf, name) -> rax   takes over the FMT_ARGB surface (loaders)
lay_delete(doc, i) -> eax 0/-1            never the last layer; hist_release + lay_free_rec
lay_move(doc, from, to)   lay_dup(doc, i) -> rax ("<Name> Kopie", above i)
lay_merge_down(doc, i) -> eax             new surface = lower ⊕ upper (upper's mode / opacity,
                                          k_bm_row on worker threads); old surface through
                                          hist_release; upper deleted; DOC_CUR = i - 1
lay_set_name / lay_set_opacity / lay_set_blend / lay_set_visible (doc, i, v)
lay_free_rec(layer)                       pixels, name, thumbnail, bitset, metadata
```
DOC_CUR follows its layer on insert / move / delete (a deleted current layer selects the one
below).  Follows A2's ownership rules (pn_hist.inc): records leave DOC_LAYERS before
`hist_release`; every `hist_push_*` result is checked (0 = no change).
Ebenen menu (lay_cmd): Neue Ebene, Ebene löschen, Ebene verdoppeln, Nach unten vereinen,
Oberste / Unterste Ebene wählen … (no history), Ebene ganz nach oben / eine Stufe höher / eine
Stufe tiefer / ganz nach unten, Ebene ein-/ausblenden, and the arg commands CMD_LAYER_SELECT (arg =
index), CMD_LAYER_SET_VIS (arg = index, toggles), CMD_LAYER_REORDER (arg = from | to << 16).
History names are the German labels ("Neue Ebene", "Ebene löschen", "Ebene
verdoppeln", "Nach unten vereinen", "Ebene eine Stufe höher", …, "Ebene
Sichtbarkeit").  M3 items (import, flip, rotate, rotozoom, properties) keep CF_DISABLED.

## 4. Composite (compose.s)

```
comp_surface(doc) -> rax SURFACE*   straight FMT_ARGB composite, up to date (SF_PX 0 = no memory)
comp_update(doc)                    recomposite the dirty 64 x 64 tiles (thr_parallel_for over
                                    tile rows; every visible layer bottom-up with k_bm_row
                                    starting from transparent)
comp_mark(doc, x0, y0, x1, y1)      preview-only change: tiles + mips dirty, view damage, no
                                    DF_MODIFIED (use doc_mark_px for real edits)
comp_mark_all(doc)   comp_pixel(doc, x, y) -> eax   comp_damage_take(doc, &rect) -> eax
comp_set_tool(doc, fn, ctx)         live tool layer (below)
comp_mip(doc, level) -> rax SURFACE*, edx level      premultiplied 2^-level, levels 1..6,
                                    refreshed from the stale tiles (level by level, parallel)
thumb_render(dst, src, cell)        4 x 4 supersampled, over a checkerboard, opaque PARGB
doc_thumb(doc, w, h, cell) -> rax   &DOC_THUMB now;  lay_thumb(doc, i, w, h, cell) -> rax
                                    LY_THUMB, re-rendered when LY_VER changed (layers window)
```
Fast path: exactly one visible layer, Normal, opacity 255, no tool hook → CO_SF aliases the
layer's pixels (CO_ALIAS = 1, nothing copied); any other state allocates the composite and
marks everything.

**Tool layer hook** (`comp_set_tool(doc, fn, ctx)`): while set, compositing calls
`fn(ctx, dst, x, y, n, layer_row) -> eax` for every row segment (≤ 64 px) of the active layer
on the WORKER threads (pure code only: no gfx_*/ui_*/text_*); return 1 after writing the
pixels as they should appear into dst (n straight pixels for x..x+n-1 of line y), 0 to use
layer_row unchanged.  Mark the preview area with `comp_mark` whenever it changes, and again
after `comp_set_tool(doc, 0, 0)`.  Commit = write the layer pixels (hist_px_touch first) with
the same function and `doc_mark_px`.

## 5. View (view.s, rulers.s)

Zoom semantics: 100 % = one image pixel per physical screen pixel.  Per document (DOC_VIEW):
DV_ZOOM, DV_FIT (Ganzes Bild follows the window), DV_CX/CY = image point at the canvas centre
(f64), DV_PREV* for Strg+B toggling back.  A new document fits the window (Ganzes Bild, ≤ 100 %)
when it is larger than the canvas, else 100 %.  The image is centred while it fits; otherwise
it can scroll 32 DIP beyond each edge.

Functions for tools and other modules (screen = absolute window px of the frame's transform):
```
view_screen_to_doc(sx, sy) -> eax x, edx y          floor
view_doc_to_screen(x, y) -> eax sx, edx sy          top-left corner of pixel (x, y)
view_screen_to_doc_fx / view_doc_to_screen_fx       24.8 fixed point in and out
view_screen_to_doc_f64(&pt) / view_doc_to_screen_f64(&pt)   pt = {f64 x, f64 y} in place
view_canvas_rect() -> rax &{x, y, w, h}   view_zoom() -> eax (1/100 %)   view_units()
view_cursor() -> eax x, edx y, r8d valid   view_origin() -> eax, edx (relative to the canvas)
view_zoom_at(zoom, sx, sy)   view_zoom_step(dir, sx, sy)      (Zoom tool)
view_pan_begin(sx, sy)  view_pan_move(sx, sy)  view_pan_end()  (Pan tool)
view_damage_doc(doc, x0, y0, x1, y1)   redraw that image area (overlays that changed)
view_invalidate()                      redraw the whole canvas
view_overlay_add(fn, ctx) -> eax 0/-1  view_overlay_remove(fn, ctx)   (8 slots)
view_dpi100() -> eax                   resolution of the active image, dpi * 100
view_test_keys(flags)                  test hook (1 Space held)
```
**Overlays**: `fn(ctx, doc, &canvas_rect)` runs at render time after the image and the grid,
on the framebuffer with the gfx clip = canvas ∩ dirty area.  Draw with gfx_* using
view_doc_to_screen*; call view_damage_doc / view_invalidate when an overlay changes (marching
ants, handles) — the canvas version does not change by itself.

**Tool input** (`tool_input(doc, uiinput, x_fx, y_fx) -> eax`): called every frame while the
pointer is over the canvas or a button is held on it, except while the view pans (Space held,
middle drag).  x_fx / y_fx = pointer position in image pixels, 24.8 fixed point (pixel centre
of screen pixel = +128).  UIINPUT move points (UI_PTS, relative to the canvas) can be
converted with view_screen_to_doc_fx((UI_RX + x) << 8 + 128, …).  Return 1 when something
changed (the view requests another pass).

Input handled by the view: wheel scrolls 48 DIP per notch (Shift: horizontal, horizontal wheel
too), Strg + wheel zooms one preset per notch around the pointer, Space + left drag and middle
drag pan, scroll bars (library ui_scrollbar placed over the canvas edges, ids ID_HSCROLL /
ID_VSCROLL).  Commands: CMD_ZOOM_IN / OUT (presets around the canvas centre), CMD_ZOOM_WINDOW
(Ganzes Bild; again = back to the zoom before), CMD_ZOOM_ACTUAL, CMD_ZOOM_SET (arg 0 = Ganzes
Bild), CMD_GRID, CMD_RULERS, CMD_UNIT_*.  Zoom / scroll never touch the composite.

Rendering (`view_draw`, the ui_canvas callback): the visible part of the image in bands of
rows on the worker pool; zoom ≥ 100 % = nearest samples of the straight composite over the
checkerboard (k_view_row); below 100 % the premultiplied mip level L with zoom·2^L ≤ 100 %:
exactly 1:1 (k_view_row_pm) for 50 %, 25 %, … else bilinear (7-bit weights, pmaddwd) from
that level (level 0 is premultiplied per row first).  Checkerboard 8 DIP, white / #CCCCCC,
anchored at the image origin.  Pixel grid (Pixel-Raster, ≥ 400 %): 1 px lines between pixels.
Pixel changes only damage their screen area (`comp_damage_take` → `ui_canvas_damage`); the
canvas version hashes zoom, origin, size, grid, theme, dpi and view_invalidate.
Workspace #DADADA / #181818 with a 6-step soft drop shadow (stronger in the dark theme).
Measured (Ryzen 5 3600, Windows, 4000 x 3000 two-layer image, whole 1920 x 1080 window render):
100 % 13.8 ms, 66 % 11.0 ms, 50 % 9.7 ms, 33 % 8.2 ms, 25 % 6.9 ms, 200 % 16 ms.

Rulers (Lineale): 20 DIP bands, px / Zoll / Zentimeter from the image resolution (DOC_DPU_*);
major steps 1, 2, 5, 10 … units ≥ 48 px apart, divided into 10 / 5 / 2 (px, cm) or 16 / 8 / 4 /
2 (Zoll) parts ≥ 5 px apart; numbers at the major ticks (vertical ruler: characters stacked),
the image extent highlighted, the pointer position as an accent line.

## 6. Tabs (tabs.s)

Tab i = ID_TAB0 + i (K_TAB, label = name, value = i, "selected"), close = ID_TAB_CLOSE0 + i
(K_BUTTON "Schließen", on the active / hovered tab), list button ID 1392 (only when the tabs do
not fit; flyout 1393 with one row per image, ids 1264+i / 1364+i-36), tooltip = file path (or
name).  Thumbnail = DOC_THUMB (image aspect inside 44 x 30 DIP, 4 DIP checkerboard), made at
once for new tabs and refreshed 250 ms after the last change (TMR_TABS).  A dot marks unsaved
changes.  Click / drag start activates; drag moves the tab (accent insertion bar, CMD_TAB_MOVE
arg = from | to << 16 on release); middle click closes (CMD_TAB_CLOSE, with the question);
Strg+Tab / Strg+Umschalt+Tab; CMD_IMG_LIST opens the image list; CMD_TAB_COPY_PATH copies the
path.

## 7. Files (fileio.s, dlg_new.s)

* **Neu** (Strg+N): window "Neu" (WINID_DLG0 + 0, modal, 460 DIP): "Neue Größe: n MB",
  Seitenverhältnis beibehalten (#5001), Breite / Höhe (#5002 / #5003, 1..65535), Pixeldichte
  (#5004 text, #5005 Pixel/in | Pixel/cm), Druckgröße Breite / Höhe (#5006 / #5008 texts,
  #5007 Zoll | Zentimeter).  Starts with the clipboard image size (ui_clip_get CLIP_IMAGE +
  img_probe), else the last size (800 x 600, 96 dpi).  OK → white image "Unbenannt n", history
  "Neues Bild".
* **Öffnen** (Strg+O): ui_filedlg with "Alle Bildtypen" + every format whose codec decodes
  (img_format_info), starting at the active image's file or the folder used last (initially
  the current directory).  The file is read once; "PDN3" / gzip → pdn_decode (every layer:
  name, opacity, blend, visibility, isBackground, metadata; resolution from the paint.net
  unit), else img_decode_ex(IMGD_STRAIGHT | ICC | EXIF | XMP) → layer "Hintergrund"; DPI from
  IMGMETA (per-metre PNG / cm → unit cm), ICC → DOC_ICC, EXIF / XMP → DOC_META (IMGMETA).
  Paths are made absolute and canonical; an open file is only activated. Linux and browser
  paths keep case distinctions; Windows paths compare ASCII case-insensitively. Errors → error bar
  (ID_FIO_FIRST + 40, title = file name) with PaintCraft's German messages.  Tests of a HEIC
  and an AVIF (lossless 4:4:4, alpha item, irot): tests/heic.uis, tests/avif.uis; AVIF is also
  written (avif_enc.s: quality, lossless, chroma subsampling in the save dialog, tests/saveopt.uis).
  JPEG XL is read (jxl*.s, modular and VarDCT, first displayed frame of an animation):
  tests/jxl.uis (lossless with alpha); written lossy (VarDCT, jxl_venc.s, Qualität) or lossless
  (modular, jxl_enc.s, Verlustfrei): tests/saveopt.uis.  (Rust: the new writer -- Bemühung,
  Filmkörnung -- is the module jxlenc.dll / jxlenc.so next to the exe, loaded on the first JPEG
  XL save; without it the assembly's writer saves and the dialog says so:
  apps/PaintCraft/tests/jxlmod.uis, jxlmod_off.uis; `--jxlenc=FILE|off`.)
* **Speichern** / **Speichern unter**: to the file (its format) when possible, else the save
  dialog (types = every encoder; preselected .pdn for layered images, else the file's format,
  else PNG; untitled images start in the folder used last).  Format from the extension;
  layered image + flat format → window "Speichern" (WINID_DLG0 + 2): "Dieses Dateiformat
  kennt keine Ebenen. …" Zusammenfassen / Abbrechen — the flattened composite is saved, the document
  keeps its layers.  .pdn → pdn_encode (all layers, borrowed surfaces).  Others → img_save_opt
  with DOC_SAVEOPT (img_opt_init defaults per format) and ICC / EXIF / XMP embedded when
  present (`fio_prep_opt`).  Success: path / name / DOC_FMT, DF_MODIFIED cleared,
  hist_mark_clean, recent files, last folder.
  In the browser, encoded bytes are persisted to browser storage before reporting success.
  A modal progress window blocks edits while this request is pending; a failure keeps the
  document modified and stops any save/close chain. A successful request resumes the chain.
* **Speicheroptionen** (dlg_saveopt.s, paint.net's Save Configuration
  dialog, window WINID_DLG0 + 3, modal, 820 × 620 DIP, resizable): shown by Speichern unter and
  by the first Speichern of a document in a format (DF_OPTOK + DOC_SAVEOPT.IO_FMT remember a
  confirmed choice); never for .pdn or a format whose encoder has no options.  Left
  "Einstellungen": exactly the controls of the encoder's IFI_IOM — Bit-Tiefe (radio buttons:
  Automatisch / 32 / 24 / 8 Bit; TGA and JPEG XR without 8), Dithering-Level 0..8 and
  Schwelle für Transparenz 0..255 (disabled for 24 / 32 bit), Qualität 1..100 (disabled while
  Verlustfrei), Verlustfrei, Chroma-Subsampling (disabled while Verlustfrei; HEIC writes 4:2:2 as
  4:2:0), Preset (WebP), Bemühung 1..9
  (PNG: Komprimierungsstufe = deflate level; DDS: Tempo der BC6H/BC7-Kompression
  Schnell / Mittel / Langsam), Komprimieren (RLE), Komprimierung (TIFF: LZW / Keine / Deflate /
  PackBits), DDS Format (27 entries incl. the sRGB variants) / Fehlermetrik / Cubemap aus
  Kreuz-Layout / Mip Maps generieren + Resampling, Zeilensprung; "Standardwerte" =
  img_opt_init.  Right "Vorschau – Dateigröße 1,23 MB" ("(wird berechnet)", "(Fehler)"): a
  worker job encodes the composite with the options *and* the document's metadata
  (fio_prep_opt, so the size is the file's) and decodes the bytes again; an option change
  cancels it (IO_CANCEL) and starts a new one.  The preview is drawn over a checkerboard, fitted
  (Fant) or at "Originalgröße" (radio buttons; a click on the image toggles and centres the
  clicked point, a drag pans).  OK (CMD_FIO_SOPT 127) writes exactly the previewed bytes
  (saveopt_take waits for a running job, or encodes when the result is stale) and stores the
  options in DOC_SAVEOPT; Abbrechen / Esc / × save nothing and stop a question chain.  The
  script `sync` waits for the preview (tool_sync_pending → saveopt_pending).  Probes: #6100
  value = file size, text busy / ok / err / none; #6101 value = previous size, text kleiner /
  größer / gleich.  Test: tests/saveopt.uis.
* **Alles speichern**, **Schließen** (Strg+W, tab close button, middle click), **Beenden**
  (Strg+Q) and UIC_CLOSE: a question chain; "Änderungen speichern?" (WINID_DLG0 + 1,
  modal): „Name“ wurde geändert und noch nicht gespeichert. Wie möchten Sie fortfahren? with the thumbnail —
  Speichern / Nicht speichern / Abbrechen (Esc / close = Abbrechen, stops the chain).
  Beenden / UIC_CLOSE walk every modified image, then quit.
* **Zuletzt geöffnet**: in memory, 10 entries, newest first, labels "&1 name" on the
  reserved rows CMD_RECENT0.. (pn_cmd_label); (Keine) hidden / Liste leeren enabled when there
  are entries.
* Window title "Name – PaintCraft", "*Name – PaintCraft" while modified, "PaintCraft" without images.
* Command line: `--open FILE` and plain file arguments (values of --size, --scale, --theme,
  --accent, --shot, --script, --auto-close, --anim, --threads, --simd, --config,
  --settings-save-at, --plugins, --crash-dir are skipped) open files
  on the first frame (every module is initialised then); without any, "Unbenannt" 800 x 600.
* Desktop opens from the file dialog, recent files and file drops use `fio_queue_path`:
  a serial background job reads in chunks and decodes flat images or PDN layers. The UI
  imports only complete results, checks the document limit again, and skips duplicate
  paths. The progress dialog (0x60EA) cancels the active read/decoder and queued files;
  no document is created for a cancelled or failed load. `sync` waits for this queue.
  Original FileType plugin cancellation terminates its helper process. Command-line
  startup files and scalar WebAssembly still use synchronous `fio_open_path`.
* `fio_open_path(path)`, `fio_queue_path(path)`, `fio_close_doc(doc)`, `fio_error(title, msg)`, `fio_error_text()`,
  `fio_title_text()`, `fio_infobar()` (view.s draws it above the canvas).

Dialog windows used by A1: WINID_DLG0 + 0 (Neu), + 1 (question), + 2 (flatten), + 3 (save
options), + 4 (browser save progress); + 5..7 are reserved. Internal fileio commands 121..127.

## 8. Test hooks (diag.s, A1 block)

Commands (`cmd ID ARG`): 900 composite pixel (arg x | y << 16) → #9900 text "AARRGGBB";
901 current layer pixel; 902 test colour (0 transparent, 1 red, 2 green, 3 blue, 4 red 50 %,
5 white, 6 black, 7 grey, 8 black 50 %, 9 yellow); 903 corner x0 | y0 << 16; 904 fill up to
x1 | y1 << 16 in the current layer (doc_mark_px, no history); 905 opacity / 906 blend mode of
the current layer (with hist_push_layer_props); 907 view key flags (1 Space held); 908 layer
thumbnail pixel; 909 test tool hook on / off (yellow 10 x 10 preview).
Hidden records (every frame): 9900 probe text · 9901 documents · 9902 active index · 9903
layers · 9904 current layer · 9905 modified · 9906 layer name (value = visible) · 9907 opacity
· 9908 blend · 9909 text "x,y" image origin in the canvas, value zoom · 9910 path · 9911 name
(value DOC_FMT) · 9912 Ganzes Bild mode · 9913 / 9914 size · 9915 resolution × 100 · 9916
resolution unit · 9917 tab thumbnail current · 9918 error bar message · 9919 window title.

## 9. Tests

* `docs.uis`: Neu dialog (ratio, print size, units, resolution in Pixel/cm, cancel, Esc),
  names, switching (click, Strg+Tab, Bilderliste), tooltip, drag reorder, closing (unchanged,
  Abbrechen, Esc, Nicht speichern, tab close button, middle click, Speichern → save dialog
  cancel / save), reopening, Beenden chain, window title, tab overflow.
* `view.uis`: cursor position, zoom presets / limits, Strg+wheel around the pointer, wheel
  scrolling and clamping, Space + drag and middle drag panning, scroll bars, Ganzes Bild
  (large image, toggle back, window resize), per-image view, rulers and units, pixel grid.
* `files.uis` (`#!args --open …/a1_rgba.png`): PNG with alpha and 300 dpi, status bar units in
  the image resolution, JPEG / GIF / BMP, activation of an open file, recent files, Speichern
  unter PNG / BMP / GIF / JPEG and reopening, flatten question (Abbrechen, Zusammenfassen),
  .pdn round trip of a layered image, paint.net's c_layers.pdn (5 layers, hidden, 50 %),
  error bar, missing file, Alles speichern.
* `layers_basic.uis`: tool hook, grey background, add, Normal / Multiply / Screen / opacity
  (values = UserBlendOps formulas, checked by hand), visibility, duplicate, go to, move,
  reorder, select, merge down with Multiply, undo / redo (A2), delete.

## 10. Open issues / notes for other agents

* Layer thumbnails for the layers window: `lay_thumb(doc, i, w, h, cell)`.
* The tool agent replaces `tool_input` in tool.s (block "A1 temporary") and uses
  comp_set_tool / view_overlay_* / view_pan_* / view_zoom_at.
* Cursor over the canvas: crosshair, hand while panning; tools may want their own cursor
  (a hook in view_frame, e.g. `tool_cursor()`, would be the next step).
* The built-in file dialog (headless) has no file type selector; the save type follows the
  typed extension (system dialogs show the list).
* `hist_init` is called by fileio for every new / opened image ("Neues Bild" / "Datei öffnen").
* Tabs: no context menu yet (Pfad kopieren exists as CMD_TAB_COPY_PATH arg = index).
* Library: `ui_draw_text` sizes are 24.8 (`px << 16 | font`); the A0 ruler stub used px << 8
  (invisible numbers) — fixed in rulers.s.

## PSD/PSB raster document interchange

`lib/img_all/psd/document.rs` exposes a layer-aware document API in addition to the
composite codec. `src/integrations/file_io.rs` installs supported raster layers and exports the stack
as 8-bit PSD without the flatten dialog. Names, visibility, opacity and mapped
blend modes are preserved. Simple masks are applied to alpha. PSB, high-depth data,
masks and lossy Photoshop-specific imports become new unsaved documents. Other
codecs reporting high-depth/HDR sources use the same source-path protection.

Groups, clipping, vector masks, effects and unsupported blend modes can use the
saved composite; editable Photoshop structures are not preserved. See the
[current compatibility and precision limits](MANUAL_PARITY.md). The editor's
internal pixels and history remain 8-bit SDR.
