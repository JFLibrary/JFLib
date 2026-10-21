# A6 – PaintCraft: Korrekturen und Effekte (effect framework)

The runner of every adjustment and effect of the library's registry (`legacy/ASMLIB/src/fx`, contract
`legacy/ASMLIB/inc/fx.inc`), the menu mapping with paint.net 5.1's German names, the generic
IndirectUI-like dialog, the Kurven and Manuelle Anpassung (Levels) dialogs, live preview through
the compose tool hook, commit through the selection as one history entry, "<Effekt>
wiederholen" (Strg+F), remembered parameters.  All 54 registered effects run from their menu
rows and give results bit-identical to `efftest` (tested: `fx_all.uis`).

```
python build.py app PaintCraft --out build/a6
build/a6/win/PaintCraft.exe --script apps/PaintCraft/tests/fx_dialog.uis [--verify-dirty]   (#!args: --open …/a6_photo.png)
wsl.exe -d Ubuntu-24.04 --cd <repo> -e ./build/a6/linux/PaintCraft --open apps/PaintCraft/tests/fixtures/a6_photo.png --script apps/PaintCraft/tests/fx_all.uis
```

## 1. Files

| file | content |
|---|---|
| `src/fx.s` | module (fx_init / states / frame / cmd), menu map `fx_map`, label table `fx_labels`, the runner (g_fx), commit, parameter memory, settings hooks, progress window, test hooks |
| `src/fxui.s` | generic effect dialog built from FXPROP lists, `fxui_status` (progress row) |
| `src/adj_curves.s` | Kurven dialog with the curve editor |
| `src/adj_levels.s` | Manuelle Anpassung (Levels) dialog with histograms |
| `inc/pn_fx.inc` | the runner record (FS_*), states, window / command / timer / test ids |
| `tests/fx_menu.uis`, `fx_dialog.uis`, `fx_selection.uis`, `fx_curves.uis`, `fx_levels.uis`, `fx_all.uis` | tests |
| `tests/fixtures/a6_photo.png`, `make_a6.py` | 320 × 240 RGBA test image; reference hash generator (efftest) |

Changes outside my files (all marked "A6"): `inc/pn.inc` includes `pn_fx.inc`; `src/tool.s`
`tool_sync_pending` also asks `fx_jobs_pending` (the single `ui_sync_hook_set` hook);
`src/diag.s` routes 995–999 to `fx_diag_cmd` and calls `fx_diag_frame`;
`tests/skeleton.uis`: the Korrekturen / Effekte rows are `enabled` now (Medianunschärfe and
"Letzten Effekt wiederholen" stay disabled).

## 2. Menu rows → effects

`fx_map` (fx.s) maps every command row of cmds.s to an FX id and the German name (also the
history entry and the dialog title).  `fx_init` calls `fx_use_all()` and looks every id up with `fx_find_id`;
`fx_states` enables a row when its effect is registered and an image is open.  Rows without a
kernel stay disabled (only **Medianunschärfe** — the registry has one `median`, used for
Rauschen ▸ Median).  Menu labels stay those of cmds.s ("Sepia…" keeps its "…" although the
kernel has no dialog).

**New / plugin effects**: a registered effect that no `fx_map` row names gets the next plugin
row (`CMD_ADJ_PLUGIN0 + k` for FXF_ADJ, else `CMD_FX_PLUGIN0 + k`), label = its key, and its key
is appended to the STRLIST `fx_plug_list_adj` / `fx_plug_list_fx` (row k = base + k).  The
rows run like the fixed ones (generic dialog, labels = keys unless `fx_labels` has them).  To
show them the integrator adds `MI_LIST CMD_ADJ_PLUGIN0, fx_plug_list_adj, 40` /
`MI_LIST CMD_FX_PLUGIN0, fx_plug_list_fx, 70` to the menus of cmds.s -- done (P1, M8): the
plugin adjustments follow the last Korrekturen row, the plugin effects the Effekte submenus
(placeholder "": an empty list shows no row).  Plugin effects are registered by plugins.s
(docs/PLUGINS.md) as one fx group after the built-ins; their FXD_NAME is the menu name.  A
failed render with ERR_FAULT (-64, a crashed plugin) shows "Das Plugin ist abgestürzt und
wurde deaktiviert …" instead of the memory message.  A new built-in effect only needs a row
in `fx_map` (and labels).

## 3. Labels (fx_labels)

`fx_label(desc, i)` / `fx_choices(desc, i)`: rows `{FX id, key, label, choice STRLIST}`,
effect-specific rows first, then rows for every effect (`LBL_ANY`: radius Radius, angle
Winkel, quality Qualität, seed Zufällig anordnen, offset Mitte, distance Distanz, amount
Betrag, intensity Intensität, strength Stärke, threshold Schwellenwert, brightness Helligkeit,
contrast Kontrast, scale Skalierung, factor Faktor, roughness Grobheit, color Farbe,
primary_color Primärfarbe, secondary_color Sekundärfarbe, blend_mode Mischmodus: (Normal …
XOR = ContentBlendMode), edge_behavior Kantenverhalten: (Abschneiden, Umlaufend, Spiegeln,
Primärfarbe, Sekundärfarbe, Transparent, Original), color_scheme Farbschema: (Klassisch,
Graustufen, Palette, Regenbogen)).  Unknown keys show the key.
All labels are PaintCraft's own German; most are single words (Helligkeit, Kontrast, Tiefen,
Lichter, Farbton, Sättigung, Tönung, Pinselgröße, Granularität, Wölbung, Zellgröße, Glättung,
Kantenlänge, Kachelgröße, Perzentil, Oktaven, …).  Longer ones: Kontur nachzeichnen, Größter
Streuradius (Frosted Glass scatter_radius), Nur Schatten (Drop Shadow), Automatisch zoomen
(Straighten), Stärke / Richtung (Twist), Transparenz: Beibehalten / Binär (Quantize alpha), and
the two hint lines of Kurven ("Tipp: Ein Rechtsklick auf einen" / "Kurvenpunkt löscht ihn.").
Dents' keys roughness / tension are shown as Detail / Turbulenz (like paint.net 4's dialog).

**Palette colours** (`fs_params_load`): properties named `primary_color` / `secondary_color`
(Clouds, Turbulence, Julia, Mandelbrot, Polar Inversion, Tile Reflection) start with
`pal_primary()` / `pal_secondary()` every time the dialog opens (paint.net uses the palette
colours); other FP_COLOR properties (Drop Shadow, Vignette "color", Levels) keep their
defaults / remembered values.  Strg+F reuses the remembered colours.

## 4. Runner (fx.s, record `g_fx`, pn_fx.inc FS_*)

1. `fx_open(desc, name, repeat)`: `tool_commit_live`; `fs_begin`: current layer of the active
   image; area = `sel_bbox` (a selection that covers nothing: nothing happens) or the layer;
   FXA_SRC = copy of the layer (FMT_ARGB), FXA_DST allocated, FXA_SEL = `tm_clone` of the
   selection mask (the kernels never see the document's mask, which may change later);
   tiles 256 × 256 anchored at the image origin (FXF_BANDS: full-width bands of 128 rows,
   FXF_WHOLE: one tile), sorted visible-first, then by distance from the centre of the visible
   part (`view_canvas_rect` / `view_screen_to_doc`, heap sort of `vis << 62 | d² << 16 | index`).
   Parameters = remembered block of the effect, else `fx_defaults`.
2. Preview hook `fx_row` (`comp_set_tool(doc, fx_row, &g_fx)`), then `fs_start`:
   FS_PARAM → FS_RPARAM (the job's copy); FXD_PREPARE (when not 0) runs as a background job of
   one item (FSP_PREPARE), then the tiles as a background job (FSP_RENDER, JOB_COUNT = tiles).
3. A tile job renders `FS_ORDER[i]` (FXARGS: ROI = tile ∩ area, AUX, JOB, SEL), then sets its
   state byte (bit 0 rendered, bit 1 to show), counts FS_NDONE / FS_UNMARKED and calls
   `plat_wake`.  The UI thread (`fs_poll`: `fx_states` every pass, UIC_WAKE, UIC_TIMER) marks
   them: bit 1 cleared, bit 2 "shown", `comp_mark(tile)` — `fx_row` uses only shown tiles, so a
   tile never appears in a composite without its damage (`--verify-dirty` clean).
4. `fx_row(fs, dst, x, y, n, layer_row)` (worker threads; 64-pixel segments): outside the area
   or an unshown tile → 0 (the layer); else `sel_clip_row`: SCR_NONE → 0, SCR_ALL → copy the
   result row, SCR_PART → `k_lerp_mask_row(dst, layer_row, result, coverage, n)`.
5. Parameter change (`fx_params_changed`, dialogs compare FS_PARAM before / after their
   widgets): FS_DIRTY, `thr_job_cancel`, `ui_timer(TMR_FX_DEBOUNCE, 50)`; the timer sets FS_DUE;
   `fs_poll` restarts when the cancelled job is done.  Old tiles stay shown until the new ones
   replace them (like paint.net).
6. OK (`CMD_FX_OK`): FSM_APPLY; a pending restart starts at once; areas ≤ 1 Mpx wait on the UI
   thread (`fs_finish_now`, helping the pool) and commit now; larger ones show the progress
   window "Rendern... n%" (WINID_FXPROG, Abbrechen) and commit from `fs_poll` when the render
   is complete and every tile shown.  Effects without a dialog (FXF_NO_DIALOG: Automatisch,
   Schwarzweiß, Sepia, Farben / Alpha umkehren) and Strg+F take the same path at once.
7. Commit (`fs_commit`): `hist_px_begin(doc, layer)`, `hist_px_touch` of the 64 × 64 tiles
   of the area that the selection reaches, hook removed, `fx_row` over every row of the area
   **in place on the layer** (`thr_parallel_for`), `doc_mark_px`, `hist_px_end(doc, name,
   IC_SLIDERS (adjustments) / IC_FILTER)` — preview == commit.  The parameters are
   remembered, the effect becomes CMD_FX_REPEAT's, whose label is "<Name> wiederholen"
   (`pn_cmd_label`).
8. Abbrechen / Esc / close button (`CMD_FX_CANCEL`, UIC_WINCLOSE): `fs_end` — job cancelled
   and waited for (kernels poll JOB_CANCEL per row), hook removed (`comp_mark(area)`), buffers
   freed, no history entry.  If the image is closed under a running effect the session ends
   without touching the document.

States: FS_MODE FSM_NONE / FSM_DIALOG / FSM_APPLY, FS_PHASE FSP_IDLE / PREPARE / RENDER / DONE /
FAILED (a kernel error other than ERR_CANCEL → "Der Effekt konnte nicht angewendet werden"
in the error bar).  Out of memory at the start → "Zu wenig Arbeitsspeicher für diesen
Effekt.".

`fx_jobs_pending()` (script `sync`, chained into tool.s's hook): 1 while the effect prepares /
renders, tiles are not shown yet, or an OK'd effect waits for its commit; a debounced restart
is started at once (scripts do not advance the clock while they sync).

Parameter memory: 128 blocks of FX_NSLOTS × 8 bytes by registry index (session).
Persistence hooks for the settings module: `fx_settings_save(ini)` / `fx_settings_load(ini)`
(section "Effekte", key = effect key, value = 512 hex digits; "_last" = the effect of Strg+F).

## 5. Dialogs

All are modal dialog windows (`WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT`) with the WinUI footer
OK / Abbrechen and a status row ("Rendern... n%" + progress bar while rendering; fixed sizes, so
the changing text never changes the layout).  `fx_place_dialog(&rect, w)` puts them right of
the image when there is room, else left of it, else over the right edge of the canvas; top =
canvas top + 16 DIP.  The main window stays drawn (live preview) but inert.

| window | id | content |
|---|---|---|
| generic (fxui.s) | WINID_DLG0 + 0x60 (24688) | property rows (below), 360 DIP |
| Kurven (adj_curves.s) | + 0x61 (24689) | Transferzuordnung, graph, R G B, "(x, y)", tip, Zurücksetzen |
| Manuelle Anpassung (adj_levels.s) | + 0x62 (24690) | histograms, input / output columns, Ausgangsgamma, Automatisch, Zurücksetzen, R G B |
| progress | + 0x63 (24691) | "Rendern... n%", progress bar, Abbrechen |

**Generic rows** (property i: ids ID_FX_PROP0 + 16 i + part): FP_INT `ui_prop_slider` (label,
slider, number box = `ui_part_id(id, 1)`, reset arrow = `ui_part_id(id, 2)`; the widget edits an
i32 copy that is sign-extended back into the i64 slot), FP_DEC `ui_prop_slider_f` (step
10^-decimals), FP_BOOL check box, FP_CHOICE radio buttons (≤ 3 choices, ids id + 4 + k) or a
combo box, FP_ANGLE `ui_angle` (ANG_360 when min ≥ 0; the value is clamped to the property's
range after the dial) + number box (id + 1) + reset (id + 2), FP_PAN `ui_pan` over a thumbnail
of the layer (`thumb_render`, 160 DIP) + X / Y number boxes (id + 3 / id + 4) + reset, FP_COLOR
swatch (click toggles an inline `ui_color_picker_ex` with alpha and fields, id + 8; popups
cannot leave their window) + "#AARRGGBB" + reset, FP_SEED "Randomisieren" (new seed =
fx_hash32(old, clicks) & 0x7FFFFFFF), FP_BLOB not shown.  FPF_LOG is shown linearly (the
library slider has no logarithmic mode; no built-in effect uses it).

**Kurven**: graph 256 × 256 DIP (1 value per DIP at 100 %): left click on a point grabs it,
elsewhere adds a point (≤ 19) and grabs it; dragging keeps x strictly between the neighbours;
right click on a point or dragging it more than 16 DIP out of the graph removes it (≥ 2 stay);
"(x, y)" of the pointer / dragged point.  Curves are drawn through `fx_spline_lut` (the
kernel's table).  Encoding = FXA's blobs (slot 1 luminosity, 6 red, 11 green, 16 blue: u8 n,
n × {x, y}; the straight line (0,0)–(255,255) is stored as n = 0).  RGB mode: an edit goes to the
first checked channel and is copied to the other checked channels; unchecked channels keep
their curves.  Zurücksetzen: every curve straight.

**Manuelle Anpassung**: input histogram = `fx_hist_rgba` over the area / selection at open;
output histogram = input through the kernel's `fx_levels_lut` table (square-root scale, R G B
overlaid, cached images); number boxes 0..255 (in white stays > in black), Ausgangsgamma
0.1..10 (2 decimals); vertical sliders with triangles: input white / black, output white / grey
/ black; grey = out_lo + (out_hi − out_lo) · 2^−gamma, dragging it sets gamma = −log2((v − lo) /
(hi − lo)) rounded to 0.01.  R / G / B work like paint.net's: they choose which channels an
edit changes (the boxes show the first checked channel); the kernel's channel mask slots 7–9
stay 1.  Automatisch = per channel lo / hi = 0.5 % / 99.5 % percentiles, gamma from the mean,
f32 exactly as FXA's auto_level → Levels after Automatisch == Korrekturen ▸ Automatisch
(tested).  Zurücksetzen = defaults, all channels checked.

## 6. Tests (all pass on Windows and Linux, also with `--verify-dirty`)

Reference results are FNV-1a 64 hashes of efftest's output for the same image and parameters
(`python apps/PaintCraft/tests/fixtures/make_a6.py --hash FX k=v`, efftest from
`python build.py app efftest --out build/a6`).  Hooks (diag.s block A6): `cmd 995 0|1|2` hash of
the layer / composite / preview (layer rows through `fx_row`) → #9990; `996 s|v<<8` integer
parameter, `997 s|v<<8` decimal (v / 100); `998 1` always asynchronous (progress window), `998 2`
settings round trip; `999 x|y<<16` pixel of FXA_DST; records #9991 mode | phase << 4 (text =
name), #9992 tiles "done/total", #9993 render starts, #9994 repeat label (value = enabled),
#9995 slot 0.

| script | covers |
|---|---|
| `fx_menu.uis` | menu states, the five immediate adjustments (hash == efftest, history names, Strg+Z), Strg+F twice, no image → disabled |
| `fx_dialog.uis` | Gaußscher Weichzeichner: modal window, preview == efftest (r 2, r 5), typing, 50 ms debounce (virtual clock), Abbrechen (layer, history unchanged), reset arrow, Esc, OK == preview == efftest, entry name, repeat label, Strg+F, remembered radius, settings round trip, async commit with `sync`, Helligkeit / Kontrast via hooks |
| `fx_selection.uis` | antialiased ellipse selection: outside unchanged, inside == efftest, two edge pixels == lerp(layer, result, coverage), preview hash == commit hash, an immediate adjustment through the selection, Esc |
| `fx_curves.uis` | add / move / right-click remove / drag-out remove, "(x, y)", RGB with all and with one checked channel, Zurücksetzen, OK, remembered curve, Strg+F — all hashes == efftest curves blobs |
| `fx_levels.uis` | number box, input triangle drag, grey triangle → gamma 0.41, gamma box, blue-only edit, Automatisch == auto_level, Zurücksetzen, OK |
| `fx_all.uis` | all 54 registered effects from their menu rows with default parameters == efftest (generated by `make_a6.py --fx-all`) |

Large image (4000 × 3000, `cmd 968`): Wolken + restart + OK with the progress window + Gauß r 50
+ commit in < 1 s (script incl. start, 12 threads).  asmcheck: 0 errors in fx.s, fxui.s,
adj_curves.s, adj_levels.s (23 BOUNDS warnings: sentinel-terminated table walks and indexes
bounded by the property count).

Screenshots (`build/a6/shot_{generic,motion,curves,levels}_{light,dark}_{100,150}.png`, desktop
composites `shot_{curves,levels}_desktop_*`): made headless; paint.net itself was not started
(rule), so the look follows paint.net 5 knowledge (IndirectUI rows, footer, Levels layout).

## 7. Open issues

* Medianunschärfe has no kernel (the registry's `median` serves Rauschen ▸ Median).
* paint.net 5.1's Sepia has an intensity (SepiaEffect.Intensity); the kernel has none, so
  "Sepia…" applies at once although its label has "…".
* FPF_LOG not supported by the slider widget (no effect uses it yet).
* Straighten: the kernel's positive angle is clockwise, the dial's counter-clockwise; values
  are passed unchanged.
* The Levels dialog has no colour swatches (paint.net: double-click opens a colour picker for
  the black / white points); R / G / B are the edit mask, not the kernel mask.
* Plugin effects: menu rows done (P1, §2); the settings module should call
  `fx_settings_save / _load`.
* Real OS windows (Win32 / WSLg) were not driven interactively (no input injection); the
  runtime behaviour relies on the multi-window layer (WINDOWS_PLATFORM.md).

## Standalone G’MIC-Qt

Command 445 launches the separately installed G’MIC-Qt application through
`src/integrations/gmic.rs`. The active layer is exchanged as 8-bit PNG. One same-sized output is
blended through the selection and committed once; multiple or differently sized
outputs open a new raster document. A background job handles execution, bounded
output decoding and cancellation. This route is independent of the original
Paint.NET DLL host. See [CREATIVE_TOOLS.md](CREATIVE_TOOLS.md) for installation,
process/format limits and the real-runtime regression checks.
