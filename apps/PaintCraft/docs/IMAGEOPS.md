# A5 – PaintCraft: Bild menu (image operations, M4)

Größe ändern, Leinwandgröße, Horizontal / Vertikal spiegeln, 90° nach rechts / links drehen,
Um 180° drehen for the whole image (every layer), the two dialogs,
"Auf Auswahl zuschneiden" (T2's `sel_crop`) and "Zusammenfassen" (A4's `lay_flatten_cmd`)
routed through the Bild module.  IMAGE-001..020.  Both targets; `image_ops.uis`,
`image_resize.uis`, `canvas_size.uis`, `flatten.uis` pass on Windows and Linux, also with
`--verify-dirty`; asmcheck: 0 errors, 0 warnings in `imgops.s` / `dlg_image.s` (no annotations).

```
python build.py app PaintCraft --out build/a5
build/a5/win/PaintCraft.exe --script apps/PaintCraft/tests/image_resize.uis [--verify-dirty]
wsl.exe -d Ubuntu-24.04 --cd <repo> -e ./build/a5/linux/PaintCraft --script apps/PaintCraft/tests/canvas_size.uis
```

## 1. Files

| file | content |
|---|---|
| `inc/pn_iop.inc` | A5 ids: internal commands 300/301, test hooks 310..319, windows WINID_DLG0 + 24..31, XK_* / AN_* / FILLC_*, the IOPPAR record (IP_*), widget ids (DI_*, ID_RS_*, ID_CS_*), probe records 5190..5199 |
| `src/imgops.s` | module (iop_init / states / frame / cmd), `iop_xform`, `iop_resize`, `iop_canvas`, the worker-pool job, the history install, test hooks + probe records |
| `src/dlg_image.s` | the dialogs "Größe ändern" and "Leinwandgröße" (sub-module of imgops.s) |
| `tests/image_ops.uis`, `image_resize.uis`, `canvas_size.uis`, `flatten.uis` | tests |

## 2. Commands (iop_cmd, CMD_IMG_FIRST..LAST)

| command | key | action | history name (paint.net 5.1 DE) | icon |
|---|---|---|---|---|
| Auf Auswahl zuschneiden | Strg+Umschalt+X | `sel_crop` (T2) | Auf Auswahl zuschneiden | IC_CROP |
| Größe ändern… | Strg+R | dialog → `iop_resize` | Größe ändern (`ResizeAction.Name`) | IC_RESIZE |
| Leinwandgröße… | Strg+Umschalt+R | dialog → `iop_canvas` | Leinwandgröße (`CanvasSizeAction.Name`) | IC_RESIZE |
| Horizontal spiegeln | – | `iop_xform(XK_FLIPH)` | Bild horizontal spiegeln (`FlipDocumentHorizontalAction.Name`) | – |
| Vertikal spiegeln | – | `iop_xform(XK_FLIPV)` | Bild vertikal spiegeln | – |
| 90° nach rechts drehen | Strg+H | `iop_xform(XK_ROTCW)` | 90° nach rechts drehen (`RotateAction.HistoryMementoName.Format` "{1} {0}") | IC_ROTATE_CW |
| 90° nach links drehen | Strg+G | `iop_xform(XK_ROTCCW)` | 90° nach links drehen | IC_ROTATE_CCW |
| Um 180° drehen | Strg+J | `iop_xform(XK_ROT180)` | 180° (alle) Drehen (`RotateAction.180` = "180° (alle)") | IC_ROTATE |
| Zusammenfassen | Strg+Umschalt+F | `lay_flatten_cmd` (A4, layer.s) | Zusammenfassen | IC_LAYERS |

States (`iop_states`): crop = `sel_crop_enabled` (T2), Zusammenfassen = `lay_flat_states`
(A4: two or more layers), the other seven enabled while an image is open.  Farbprofil… (278)
keeps its CF_DISABLED from cmds.s (colour management module).  `main.s` runs
`tool_precmd` before every command, so a pending tool edit (brush stroke, moved pixels, a
pending selection transform) is committed before any Bild operation.

## 3. The operations

API (UI thread, doc = DOC*, one history step each, `eax 1` = step made):
```
iop_xform(doc, kind) -> eax      XK_FLIPH 0, XK_FLIPV 1, XK_ROTCW 2, XK_ROTCCW 3, XK_ROT180 4
                                 (= command id - CMD_FLIP_H)
iop_resize(doc, par) -> eax      par = IOPPAR: IP_W, IP_H (1..65535), IP_FILTER (RS_*),
                                 IP_FLAGS (RS_GAMMA), IP_RESSET / IP_UNIT / IP_DPU
iop_canvas(doc, par) -> eax      IP_W, IP_H, IP_ANCHOR (AN_TL..AN_BR, row-major 3 x 3),
                                 IP_FILL (straight ARGB for background layers), resolution
```
* **New surfaces, never in place**: `hist_push_doc` keeps the old surfaces by reference,
  so every operation allocates one new FMT_ARGB surface per layer (`iop_alloc`: zeroed,
  opacity / blend mode copied), fills it on the worker pool and installs it (`iop_install`).
  Out of memory before the step: nothing changes, the error bar says "Zu wenig
  Arbeitsspeicher, um die Bildgröße zu ändern." (resize / canvas) or "Der Arbeitsspeicher reicht
  für diesen Schritt nicht aus."
* **Worker pool**: `thr_parallel_for(iop_job, g_ij, layers × bands)`, about four row bands
  per thread (≥ 16 rows).  Flip / rotate = one integer map dst(x', y') ← src(ax x' + bx y' +
  cx, ay x' + by y' + cy) per kind (table `xk_tab`); a plain row copy (`k_copy`) when the
  source step is +4 bytes (vertical flip), else a pixel loop over 64-column blocks so quarter
  turns stay cache friendly.  Canvas size = fill / `k_copy` / fill per row.  Resize =
  `rs_prepare` per layer (UI thread), `rs_resize_rows(ctx, y0, y1)` per band, `rs_free`;
  a band that runs out of memory fails the whole operation.  Measured (Ryzen 5 3600,
  4000 × 3000, three layers): resize, gamma resize, rotations, flips and a canvas change —
  seven operations with their undos — ~0.8 s together.
* **History** (`iop_install`, pn_hist.inc rules): `hist_begin_compound`; `hist_push_sel`
  when the image has a selection and the operation moves it; `hist_push_doc`; for every
  layer: tmp = {0}, tmp.LY_SF = old surface, install the new one, `LY_VER`++,
  `hist_release(doc, &tmp)`, `lay_free_rec(&tmp)`; DOC_W / DOC_H; resolution; the selection;
  `doc_resized`; `hist_end_compound(name, icon)` → exactly one entry.  A failed push frees the
  new surfaces and changes nothing.  Undo / redo swap the LAYER arrays (bit-exact, checked
  with `hist_state_hash` in every test).
* **Selection**: the forward matrix goes to `sel_set_xform` + `sel_bake` (T2), so the
  geometry is transformed losslessly: flips / quarter turns map pixel edges to pixel edges
  (a rectangle stays exact, an ellipse keeps its antialiased shape), resize scales it by
  (W'/W, H'/H), canvas size translates it by the anchor offset (a selection moved off the
  canvas clips everything, like T2's crop / move).
* **Resolution**: quarter turns exchange DOC_DPU_X / DOC_DPU_Y; the dialogs set DOC_DPU_UNIT
  and DOC_DPU_X = DOC_DPU_Y only when the user changed the resolution or its unit (recorded
  by `hist_push_doc`, restored by undo).  Same size + new resolution = a step without new
  surfaces; same size + same resolution = no step.
* **View**: after a size change DV_ZOOM = 0 (view.s' "not placed yet"): the view places the
  image again like a new one (100 % when it fits, else Ganzes Bild), + `view_invalidate`.
* **Canvas size**: the old image is placed at (ox, oy): column / row 0 → 0, 1 → (W' − W) / 2,
  2 → W' − W with **C# integer division** (truncation toward zero, like paint.net's
  `(newSize.Width - oldSize.Width) / 2`; shrinking centred by an odd amount keeps the extra
  pixel on the right / bottom).  **Fill rule** (paint.net 5.1, `CanvasSizeDialog` +
  `CanvasSizeFillColor`; older versions always filled with the secondary colour): the new area of **background layers** (LYF_BG, paint.net `IsBackground`) gets
  the chosen colour — Sekundärfarbe by default, or Grundfarbe, Transparent, Schwarz, Weiß —;
  every other layer gets transparent pixels (0x00000000).
* **Resize filters** (`RS_*` of ASMLIB img/resample.s, bit-exact with
  `tools/resample_check.py`): the dialog list maps Adaptiv (scharf) → RS_ADAPTIVE, Lanczos →
  RS_LANCZOS3, Bikubisch → RS_BICUBIC, Bikubisch (glatt) → RS_BICUBIC_SMOOTH, Bilineare →
  RS_BILINEAR, Bilinear (schnell) → RS_BILINEAR_LQ, Fant → RS_FANT, Nächster
  Nachbar → RS_NEAREST; "Gammakorrektur anwenden" → RS_GAMMA.
* **Zusammenfassen** belongs to A4 (layer.s `lay_flatten`): the composite of the visible
  layers becomes the bottom layer (name and background flag stay; Normal, 255, visible), the
  other layers go (one step "Zusammenfassen"); with a hidden layer A4 asks "Ausgeblendete Ebenen
  löschen?" (window WINID_DLG0 + 19).  imgops only routes the command and its state.

## 4. Dialogs (dlg_image.s)

Modal dialog windows (`WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT`, 460 DIP wide, OK / Abbrechen
footer, Esc = Abbrechen): **Größe ändern** = WINID_DLG0 + 24 (0x6028), **Leinwandgröße** =
WINID_DLG0 + 25 (0x6029); + 26..31 are reserved for A5.  Both share the size part (widget
ids base + DI_*, base 5100 / 5200):

| id | widget | behaviour |
|---|---|---|
| +1 / +3 | radios "Prozentual:" / "Absolut:" | percent mode: the absolute part is disabled and shows the computed size; absolute: the percent box is disabled |
| +2 | number box percent (1..10000) | W = round(W₀ · p / 100), H likewise (ties to even, like .NET `Math.Round`) |
| +4 | "Seitenverhältnis beibehalten" | the other side from the ratio of the current image (checking it recomputes the height) |
| +5 / +6 | Breite / Höhe (Pixel, 1..65535) | |
| +7 | Pixeldichte (f64, 2 decimals) | pixel size stays, print size follows |
| +8 / +10 | Pixel/in \| Pixel/cm and Zoll \| Zentimeter | **linked** (one variable): switching converts the resolution by 2.54, the pixel size stays |
| +9 / +11 | print Breite / Höhe (f64) | pixels = round(print · resolution), then the ratio, then the print size is recomputed |

Text lines: "Aktuelle Bildgröße: n MB" and "Neue Größe: n MB" (memory of all layers, 4 bytes
per pixel, paint.net's `Utility.SizeStringFromBytes`: "n,n GB / MB / KB" with one decimal
or "n Bytes").  The dialog starts from the image: size, resolution (Pixel/cm when the image
unit is centimetres; no unit = 96 Pixel/in).

* **Größe ändern** adds "Optionen": Interpolation (#5112: Adaptiv (scharf), Lanczos,
  Bikubisch, Bikubisch (glatt), Bilineare, Bilinear (schnell), Fant, Nächster
  Nachbar) and "Gammakorrektur anwenden" (#5113).  Defaults: Prozentual 100 %, ratio on, Adaptiv (scharf), gamma on.
* **Leinwandgröße** adds "Anker": "Anker:" combo box (#5212, `AnchorEdge.*`: Links oben,
  Oben, Rechts oben, Links, Mitte, Rechts, Links unten, Unten, Rechts unten), the 3 × 3 grid
  (#5220..#5228, tool buttons: the anchor shows the picture icon, its neighbours arrows
  pointing away from it — drawn with `ui_draw_line` —, the other cells a thin frame; grid and
  combo box are one variable) and "Fülleffekt:" (#5213: Sekundärfarbe, Grundfarbe,
  Transparent, Schwarz, Weiß).  Defaults: Absolut (the current size), ratio off, Mitte,
  Sekundärfarbe.
* Every setting except the size is remembered per dialog for the session when OK is pressed.
  OK writes `g_iop_par` and queues CMD_IOP_RESIZE_DO (300) / CMD_IOP_CANVAS_DO (301).
* Enter in a number box commits the value and accepts the dialog (paint.net's AcceptButton;
  detected as EV_COMMIT while the box keeps the focus — Tab / Esc / clicks drop it).

## 5. Test hooks (imgops.s, commands 310..319 in the Bild range; records 5190..5192)

`cmd 310 w|h<<16` target size · `cmd 311 filter|gamma<<8` resize the active image ·
`cmd 312 anchor|fill<<8` canvas size (fill = FILLC_*) · `cmd 314 L` layer for 313 ·
`cmd 313 x|y<<16` pixel of layer L → #5190 text "AARRGGBB" ("none" outside) ·
`cmd 315 kind` iop_xform.  Records: #5190 probe text · #5191 value 1 = every layer surface
has the image size · #5192 text "w,h", value layers.

## 6. Tests

* `image_ops.uis`: 6 × 4 image, two layers, coloured corners; each of the five transforms
  (menu clicks, Strg+H / G / J, commands): size, 8–10 probes over both layers, composite,
  history name, one step, undo = state hash, redo; compositions (180° = H + V, cw + ccw,
  4 × cw = identity); the selection follows every transform (rectangle bounds + coverage
  probes, ellipse), undo restores it.
* `image_resize.uis`: the dialog (percent 50 / 1 / 250 %, "Neue Größe" in KB / Bytes / MB,
  absolute with / without ratio, resolution 300 Pixel/in, Pixel/cm ↔ Zoll / Zentimeter, print
  width in cm, print height in inches with the ratio, Abbrechen, Esc, OK, remembered
  settings, Enter in a number box = OK, resolution-only step, a centimetre image opens in
  Pixel/cm); every filter with
  and without gamma through the hook (8 × 6 → 5 × 3 and → 11 × 9, one probe each = the value
  of `resample_check.ref_resize`), the second layer resized too, every list entry through the
  dialog; undo = state hash.
* `canvas_size.uis`: all nine anchors 6 × 4 → 8 × 6 with probes of both layers (old corners,
  new area: secondary colour in the background layer, transparent in layer 1), the five fill
  choices, odd growth, shrinking (Mitte / Rechts unten / Links oben / wider-but-lower), the
  selection offset (and moved off the canvas), the dialog (defaults, grid ↔ combo box, fill
  combo box, OK, remembered settings, Prozentual 200 %, Abbrechen / Esc).
* `flatten.uis`: three layers (opacity 50 %), Strg+Umschalt+F → one layer "Hintergrund"
  with the composite, menu states, undo / redo, the hidden-layer question (Abbrechen / OK),
  one layer = disabled.

Screenshots: `build/a5/dlg_{resize,canvas}_{light,dark}_{100,150}.png`,
`build/a5/dlg_resize_abs_light_100.png`, `build/a5/dlg_canvas_desk_light_100.png` (the dialog over
the main window), Linux `build/a5/dlg_canvas_linux_dark_100.png`, `build/a5/dlg_resize_linux_light_100.png`.

## 7. Compared with paint.net 5.1

The dialogs follow paint.net's layout and options; their texts are PaintCraft's own German
(short field labels such as "Breite:", "Höhe:", "Pixeldichte:", "Anker:" are the usual words).
Checked against paint.net: the set of options, the eight filters, the nine anchors, the five
fill choices and the size string formats.

**Unverified** (no source without running paint.net): the order of the Interpolation list
and of the Fülleffekt list, the defaults (Adaptiv (scharf), gamma on, Prozentual for Größe
ändern, Absolut for Leinwandgröße), the value shown after "Aktuelle Bildgröße:" (here the
memory size like "Neue Größe"), linked resolution / print units, the placement of
Pixeldichte in the Pixelgröße group (kept like A1's "Neu" dialog), that the 5.1 fill choice
applies only to the background layer (the classic paint.net rule), transparent = 0x00000000
(paint.net 3 used transparent white), the view refit after a size change.

## 8. Changes outside my files

* `tests/skeleton.uis`: the seven A5 Bild items are expected **enabled** now (lines 135–141).
* No change to main.s, cmds.s, chrome.s, pn_ids.inc, pn.inc, diag.s (the probes live in my
  own id ranges; `pn_iop.inc` is included by my two files only).
* Uses (read / call only): DV_ZOOM = 0 (view.s' documented "not placed yet" value),
  `sel_active`, `sel_set_xform`, `sel_bake`, `sel_crop`, `sel_crop_enabled` (T2),
  `lay_flatten_cmd`, `lay_flat_states`, `lay_free_rec` (A4 / A1), `pal_primary`,
  `pal_secondary` (A3), `fio_error` (A1), `hist_*` (A2).

## 9. Open issues

* Anisotropic resolutions (DPU_X ≠ DPU_Y) show and set only one value in the dialogs.
* No progress display: a very large resize runs synchronously on the UI thread (the work
  itself is parallel).  paint.net shows a percentage
  while resizing.
* The dialogs have no tooltips on the anchor cells beyond their names.
