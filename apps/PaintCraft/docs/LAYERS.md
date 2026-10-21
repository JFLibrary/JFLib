# A4 – PaintCraft: Ebenen (layers window, layer properties, import, flip, Rotationszoom, flatten)

The layers package of the paint.net 5.1 clone: the "Ebenen" tool window, the
"Ebeneneigenschaften" and "Rotationszoom" dialog windows, Als Ebene importieren (images and
.pdn documents, with the "larger than the canvas" question), Horizontal / Vertikal spiegeln,
Um 180° rotieren, Zusammenfassen (Bild menu) and the test hooks.  LAYER-001..020 work on both
targets (the layer model itself is A1's, see DOCUMENT.md §3).

```
python build.py app PaintCraft --out build/a4
build/a4/win/PaintCraft.exe --script apps/PaintCraft/tests/layers_window.uis [--verify-dirty]
build/a4/win/PaintCraft.exe --script apps/PaintCraft/tests/layer_props.uis
build/a4/win/PaintCraft.exe --script apps/PaintCraft/tests/layer_import.uis
build/a4/win/PaintCraft.exe --script apps/PaintCraft/tests/layer_rotzoom.uis
wsl.exe -d Ubuntu-24.04 --cd <repo> -e ./build/a4/linux/PaintCraft --script apps/PaintCraft/tests/layer_props.uis
```

## 1. Files

| file | content |
|---|---|
| `inc/pn_lay.inc` | ids of the package: internal commands 350..355, test hooks 370..379, windows, widgets, timer |
| `src/layer.s` | A1's layer model (unchanged API) + A4 section: lay_init / lay_frame, import, flips, flatten, questions, test hooks and probes |
| `src/wnd_layers.s` | the Ebenen tool window (replaces the A0 stub) |
| `src/dlg_layerprops.s` | Ebeneneigenschaften (new, sub-module of layer.s) |
| `src/dlg_rotzoom.s` | Rotationszoom (new, sub-module of layer.s) |
| `tests/layers_window.uis`, `layer_props.uis`, `layer_import.uis`, `layer_rotzoom.uis` | tests |
| `tests/fixtures/layers/` | `make_fixtures.py` → l4_small / l4_wide / l4_tall / l4_lo / l4_hi .png |

Ids (pn_lay.inc): dialog windows **WINID_DLG0 + 16 .. 23** (A4 block: +16 Ebeneneigenschaften
0x6020, +17 Rotationszoom 0x6021, +18 import question 0x6022, +19 hidden-layers question
0x6023, +20..23 reserved); widgets: layers window 4200..4389 (bar 4201..4207), list
`ID_LAYER_LIST` 0x40000 (row r = 0x40001 + r, row 0 = top layer; its visibility check box =
`ui_part_id(row id, 1)`), probes 4390..4399, layer properties 5400..5449, questions
5450..5469, Rotationszoom 5300..5399, import file dialog `ID_FD_IMPORT` 0x18000..;
timer `TMR_WLAY` = TMR_WND + 0x20.  Internal commands (lay_cmd, not in the command table):
350 CMD_LAY_PROPS_DONE, 351 CMD_LAY_RZ_DONE, 352 CMD_LAY_RZ_UPDATE, 353 CMD_LAY_IMP_FILE,
354 CMD_LAY_IMP_ANSWER, 355 CMD_LAY_FLAT_ANSWER; test hooks 370..379.

## 2. Ebenen window (wnd_layers.s, LAYER-007/011..015/019)

`WF_TOOL | WF_RESIZABLE`, 208 × 240 DIP, default place bottom right below the Verlauf window
(shorter when the main window is low), visibility `chrome_wnd_open(WND_LAYERS)` (F7, title
row toggle, close button); `wlay_reset_pos()` for "Fensteranordnung zurücksetzen".

* Virtualised `ui_list` with `LF_REORDER`, 44 DIP rows, **top layer first**: thumbnail
  (image aspect in a 44 × 34 DIP box, over a checkerboard, hairline frame), name
  (`TX_ELLIPSIS`), visibility check box at the right.  The current layer is the selected row
  (accent pill + shade, LAYER-011); after a change of the current layer / image the row is
  scrolled into view.
* Click = `CMD_LAYER_SELECT`; double click = select + `CMD_LAYER_PROPS`; the check box =
  `CMD_LAYER_SET_VIS` (history "Ebene Sichtbarkeit"); dragging a row = `CMD_LAYER_REORDER`
  (one history step "Ebene verschieben") and the moved layer becomes the current one
  (a drop on its own place is a click); right click = the layer menu for that row (it
  becomes the current layer): Neue Ebene, Ebene löschen, Duplizieren, Zusammenführen, Nach
  oben, Nach unten, Eigenschaften… -- since U3 with the command table's labels and shortcuts
  ("Neue Ebene", "Ebene verdoppeln", "Nach unten vereinen", "Ebene
  eine Stufe höher", "Ebeneneigenschaften…"): the menu may leave the tool window.
* Bottom bar (subtle 28 DIP icon buttons, states from the command table): Neue Ebene,
  Ebene löschen, Ebene duplizieren, Nach unten vereinen, Ebene eine Stufe höher, Ebene eine
  Stufe tiefer, Eigenschaften — the command label + the shortcut from the command table ("Neue Ebene
  (Strg+Umschalt+N)", "Eigenschaften (F4)").
* **Keyboard** (ROADMAP §11, library `kbnav.rs`; test `tests/keyboard.uis`): Ctrl+F6 reaches
  the window (F7 only shows / hides it; F6 is paint.net's Verlauf toggle), the list is
  focused: Up / Down / Home / End / PgUp / PgDn select the layer, Ctrl+Up / Ctrl+Down /
  Ctrl+Home / Ctrl+End move it (the list's keyboard drop = `CMD_LAYER_REORDER`, one history
  step), Space toggles its visibility (the selected row's check box), Enter opens
  Ebenen-Eigenschaften (opacity, blend mode, name), the menu key / Shift+F10 the layer menu at
  the row; Tab: list, the selected row's check box, the bottom bar (one stop, arrows inside);
  Esc gives the keys back to the main window. Keys the window does not use (Ctrl+Shift+N, F4,
  Ctrl+Z ...) still go to the main window.
* **Lazy thumbnails**: only rows in view are rendered (`lay_thumb`, cached in LY_THUMB); a
  missing or resized thumbnail is made at once, a stale one (pixels changed) is kept until
  the image has not changed for 250 ms (`TMR_WLAY` wakes the loop), so a brush stroke does
  not re-render thumbnails every frame.

## 3. Ebeneneigenschaften (dlg_layerprops.s, LAYER-008..010)

F4 / Ebenen ▸ Ebeneneigenschaften… / double click / the bar button: modal dialog window
"Ebeneneigenschaften" (400 DIP): **Allgemein** — Name (text box), Sichtbar (check box);
**Mischmodus:** — combo box with paint.net's 14 `LayerBlendMode.*` names in .pdn order
(Normal, Multiplizieren, Additiv, Farbig nachbelichten, Farbig abwedeln, Reflektieren,
Leuchten, Überlagern, Differenz, Negierung, Aufhellen, Abdunkeln, Raster, Xor);
**Deckfähigkeit** — 0..255 slider + number box.  Every change is shown at once on the layer
(live preview through `lay_set_*`, no history).  **OK** = ONE history step
"Ebeneneigenschaften" whose other state is the values from before the dialog: the layer is
set back, `hist_push_layer_props` captures it, the new values are applied (the old name goes
through `hist_release` in `lay_set_name`); nothing changed = no step.  **Abbrechen** / Esc /
close restore every value and the image's modified flag.  The dialog keeps the layer by uid
(`lay_find_uid`).  (U3: no extra height below the controls any more -- the combo list of the
14 blend modes may leave the dialog.)

## 4. Als Ebene importieren (layer.s, LAYER-006)

Ebenen ▸ Als Ebene importieren…: `ui_filedlg(ID_FD_IMPORT)` (system dialog on Windows, the
library's dialog headless / on Linux), types "Alle Bildtypen" + every registered decoder,
starting in the folder of the last import (initially the working directory).
* Images (`img_decode_ex`, straight) → one layer named after the file ("foto.png");
  `.pdn` (`pdn_decode`) → one layer per document layer, named "datei.pdn: Ebenenname"
  (paint.net's `ImportFromFileAction.ImportOneFile.NewLayer.Format` "{0}: {1}"), with
  opacity, blend mode, visibility and metadata; not a background layer.
* The new layers go **above the current layer** (in file / document order); the topmost
  new one becomes current.  Images are placed at the top left; smaller ones are padded
  transparent.
* An image larger than the canvas asks (modal "Als Ebene importieren", paint.net's
  `ExpandCanvasQuestion.*` texts): **Leinwand vergrößern** (the canvas grows to hold every
  imported image, anchored top left; the background layer's new area in the secondary
  colour, other layers transparent), **Leinwand nicht ändern** (the image is cut),
  **Abbrechen** / Esc.
* One history step "Als Ebene importieren": `hist_push_layers`, or `hist_push_doc` when the
  canvas grows (the old surfaces go through `hist_release`); undo restores size and layers.
  All new surfaces are allocated before the history push (out of memory = no change and
  paint.net's message in the error bar).
* Errors: the error bar with paint.net's messages (file not found, unknown format, out of
  memory, corrupt).
* `lay_import_paths(doc, list)` imports several files at once (one step, one question) —
  for drag and drop (fileio / UIC_DROP) and tests; the file dialog chooses one file (the
  library dialog has no multi-select).

## 5. Spiegeln, 180°, Rotationszoom (LAYER-016..018)

* Horizontal spiegeln / Vertikal spiegeln / Um 180° rotieren: the whole current layer in
  place (`img_flip_h` / `img_flip_v` / `img_rotate180` after `hist_px_touch` of every tile),
  history "Horizontal kippen" / "Vertikal kippen" / "Um 180° rotieren"
  (`FlipLayer*Action.Name`, `RotateLayer180Action.Name`).
* **Rotationszoom…** (Strg+Umschalt+Z, dlg_rotzoom.s): modal dialog like paint.net's
  RotateZoomEffect: **Kippen / Rotieren** — Winkel: dial (`ui_angle`) + number box (°,
  counter-clockwise); **Verschieben** — `ui_pan` over a thumbnail of the layer: where the
  layer's centre goes (−1..1 = the canvas edges; Home centres, arrows 1 %); **Zoom** — a
  logarithmic slider (1/16..16) + number box (0,06..16); **Kachelmodus:** Keine / Wiederholen
  / Spiegeln (`XF_TRANSPARENT` / `XF_WRAP` / `XF_REFLECT`); **Original als Hintergrund** (the
  result drawn Normal over the untransformed layer); **Qualität** — Probenahme: Nächster
  Nachbar / Bilineare / Bikubisch (default) / Bilinear mit Multisampling (`rs_affine` filters).
  Transform: dst = C + P + R_ccw(a)·Z·(src − C) (C = canvas centre, P = pan·C); `rs_affine`
  gets T(C)·S(1/Z)·R(a)·T(−(C+P)).
* **Live preview**: whenever a parameter changed (hash of the parameter block), one
  `CMD_LAY_RZ_UPDATE` per frame computes a preview surface of the layer's size on the worker
  pool (`rs_affine_prepare` + `rs_affine_rows` per 64-row band, then keep-background and
  selection passes with `k_bm_row` / `sel_clip_row` / `k_lerp_mask_row`); the compose tool
  hook (`comp_set_tool(doc, rz_hook, &preview)`) shows it instead of the layer's rows.  With
  a selection only the selected part changes (antialiased edges blend by the coverage).
* **OK** writes the preview into the layer inside `hist_px_begin / touch / end` (one step
  "Rotationszoom"; unchanged tiles are dropped) and removes the hook; **Abbrechen** / Esc
  remove the hook only.  paint.net's 3-D roll ("Kippen" amount / direction) is not
  implemented: the section has the angle only.

## 6. Zusammenfassen (LAYER-020)

`lay_flatten(doc)`: the composite of the visible layers (`comp_surface`) becomes the bottom
layer (Normal, 255, visible; its name and background flag stay), every other layer is
deleted (`lay_delete`: record out, `hist_release`, free) — one step "Zusammenfassen"
(`hist_push_layers`).  `lay_flatten_cmd()` asks "Ausgeblendete Ebenen löschen?" (OK /
Abbrechen, window 0x6023) when a layer is hidden.  `lay_flat_states()` enables CMD_FLATTEN
with more than one layer.  Both are called by A5's imgops.s (the Bild menu); A5's
`tests/flatten.uis` covers it.

## 7. Test hooks and probes (layer.s)

The diag.s command block 900..999 is fully assigned, so A4's hooks live in the Ebenen range
(lay_cmd) and its hidden records in the layers window block:

| cmd | meaning |
|---|---|
| 370 i | layer i → #4390 text "name\|visible\|opacity\|blend" ("-" = none) |
| 371 mask | import the fixtures of the bits at once (0 l4_small, 1 l4_wide, 2 l4_tall, 3 apps/PaintCraft/tests/fixtures/pdn/c_layers.pdn, 4 build/a4_layers.pdn, 5 a missing file, 6 l4_lo, 7 l4_hi) |
| 372 x \| y<<12 \| i<<24 | pixel of layer i → #4390 "AARRGGBB" |
| 373 op<<16 \| v | Rotationszoom parameter: 0 angle °, 1 zoom %, 2 pan x %, 3 pan y %, 4 Kachelmodus, 5 Original als Hintergrund, 6 Probenahme (v signed 16 bit) |
| 374 kind | lay_flip of the current layer |

Records (every frame, main window): #4390 text probe · #4391 value history position, text the
current entry's name · #4392 value history entries · #4393 value thumbnails rendered by the
layers window · #4394 value Rotationszoom previews computed, text "angle\|zoom\|panx\|pany"
(× 100) · #4395 value open A4 dialogs (1 properties, 2 Rotationszoom, 4 canvas question,
8 hidden-layers question, 16 import file dialog).

## 8. Tests

* `layers_window.uis`: default place, rows = layers, bar states and tooltips, add / select /
  visibility (with undo) / drag reorder (both directions, undo / redo, no-op drop) /
  duplicate / move / merge / delete through the bar, double click and the bar button →
  properties, context menu (select, Duplizieren, Eigenschaften…), flips and 180° with pixel
  probes, lazy thumbnails (render count before / after 250 ms), scrolling to the current row
  with 9 layers, F7 / close button / title-row toggle, dark theme shot.
* `layer_props.uis`: Abbrechen keeps an unchanged image unmodified, every blend mode through
  the combo box with the live preview checked on 4 composite pixels (values from
  `tools/blend_check.py` `ref_blend`, paint.net UserBlendOps; the fixture colours give 14
  distinct results), opacity 128 / 51 (typed), name with umlauts, visibility, OK = one step,
  one Strg+Z / Strg+Y restores / reapplies everything, Abbrechen and Esc restore, unchanged
  OK = no step, menu item and bar button.
* `layer_import.uis`: menu → dialog → PNG (pixels, name, composite, undo / redo), the start
  folder of the next import, the question (Abbrechen, Esc, beibehalten = cut, vergrößern =
  96 × 48 with the secondary colour, undo restores the size), three files at once (one
  question, one step), paint.net's c_layers.pdn (5 layers: names, hidden, 128), a .pdn
  written by the app (Speichern unter) imported into a new image (opacity 200, Multiply),
  errors (missing file, unknown format).
* `layer_rotzoom.uis`: defaults, 180° (preview only, layer unchanged), Abbrechen, pan with
  the three tiling modes, Original als Hintergrund, pan keys, zoom 0,5 through the number
  box and the log slider, Probenahme, OK = one step + undo / redo, Esc, unchanged OK = no
  step, a rectangular selection limits the change.
* Updated A0 / A1 tests (behaviour changed on purpose): `skeleton.uis` (the six A4 Ebenen
  items are enabled; F4 removed from the "disabled accelerators" line because it opens the
  properties now), `layers_basic.uis` (Ebeneneigenschaften… enabled).

Results (2026-09-30, build/a4): the four A4 scripts and every other PaintCraft script pass on
Windows and Linux (WSLg), the A4 scripts and layers_basic / flatten / history also with
`--verify-dirty`.  asmcheck on the four A4 files: 0 errors, 2 BOUNDS warnings (the
zero-terminated command tables of lay_states), 2 `assume` annotations.
Screenshots: `build/a4/shot_layers_{light,dark}_{100,150}.png`, `shot_props_{light,dark}.png`,
`shot_rotzoom_{light,dark,desk}.png`, `shot_ctx.png`; real windows `build/a4/cap_win_layers.png`
(Win32, owned WS_EX_TOOLWINDOW), `build/a4/cap_linux_layers.png` (WSLg).

## 9. Notes / open issues

* (done, U3) popups (menus, combo lists) may leave their window: full row menu labels, no
  reserved room in the properties dialog; `ui_filedlg` has no
  multi-select (paint.net imports several files at once: `lay_import_paths` is ready);
  combo lists do not scroll.
* paint.net's Rotationszoom "roll" (tilt amount / direction, a 3-D look) is not implemented;
  Bilinear mit Multisampling previews of very large layers take ~0.5 s per change.
* Layer names of new layers stay A1's "Ebene n" (n = layer count).
* (done 2026-10-20) keyboard navigation of the layers window: see section 2.

## U3 notes
* Thumbnails: `wl_thumb` gives every rendered thumbnail a version that never repeats
  (`g_wl_tver`, high word): a thumbnail SURFACE freed with its layer and allocated again at the
  same address restarted SF_AUX at 1, so the image command looked unchanged and
  `history.uis --verify-dirty` missed 29 pixels after a history jump.
* Keys: after a click into the list, Up / Down move the selection (the Layers window borrows
  the keys, library WINDOWS.md §7.10); other keys still reach the main window.  Test:
  `tests/toolkeys.uis`.
