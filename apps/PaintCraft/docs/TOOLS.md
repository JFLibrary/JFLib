# T1 – PaintCraft tool framework and painting tools

**Contract frozen** (`inc/pn_tooli.inc`): TOOLDESC, TOOLCFG, TL, NUB, BEP offsets, the
function names below and `sel_clip_row` do not change; later additions only extend.

```
python build.py app PaintCraft --out build/t1
build/t1/win/PaintCraft.exe --script apps/PaintCraft/tests/tools_brush.uis [--verify-dirty]
wsl.exe -d Ubuntu-24.04 --cd <repo> -e ./build/t1/linux/PaintCraft --script apps/PaintCraft/tests/tools_fill.uis
```

## 1. Files

| file | content |
|---|---|
| `inc/pn_tooli.inc` | the contract: TOOLDESC, TF_*, TOOLCFG (TC_*), FS_* / SM_* / SQ_* / SMP_*, TL (tool layer), SCR_* (sel_clip_row), NUB, BEP, OMF_*, option widget ids, test hook ids |
| `src/tool.s` | framework: registry, switching, pending edits, `tool_precmd`, keys (Enter / Strg+Z), cursor, brush outline, shared options + widgets (`opt_*`), tool layer (`tl_*`, `fill_pattern_row`, 53 hatch patterns), test hooks (`t1_diag_*`) |
| `src/nubs.s` | handles (nubs) of live tools: draw, hit, hover, damage, 8-nub frames |
| `src/brush_engine.s` | stroke coverage (TMASK, max of dabs; pencil lines) |
| `src/t_brush.s` | Farbpinsel, Radierer, Zeichenstift |
| `src/t_bucket.s` | Farbeimer (live fill, background job) |
| `src/t_picker.s` | Farbauswahl |
| `src/t_view.s` | Zoomen, Ansicht verschieben |
| `tests/tools_*.uis` | brush, fill, picker, view, options |

Changes in other files (all small, marked "T1"): `inc/pn.inc` includes `pn_tooli.inc`;
`main.s` pn_cmd calls `tool_precmd(id, arg)` before routing a command id; `view.s` view_frame
takes the canvas cursor from `tool_cursor()`; `diag.s` routes 930..949 to `t1_diag_cmd` and
calls `t1_diag_frame`; `sel.s` carried a "T1 temporary" `sel_clip_row` until T2 replaced it;
`tests/skeleton.uis` expects the label "Pinselgröße:" (was "Pinselbreite:").

## 2. Registering a tool

```asm
t_foo_init:                             # called from your module's *_init (any order:
        mov     ecx, TOOL_FOO           #   the table is .bss, tool_init does not clear it)
        lea     rdx, [rip + foo_desc]
        jmp     tool_register

        .section .rodata                # (or .data if you patch it at run time)
        .p2align 3
foo_desc:
        .long   TF_LIVE | TF_AA | TF_BLEND  # TD_FLAGS
        .long   0                       # TD_ICON (0 = command table icon)
        .quad   0                       # TD_NAME (0 = command label, e.g. "Farbeimer")
        .quad   0                       # TD_HINT (0 = the paint.net help text of tool.s)
        .quad   foo_activate, foo_deactivate, foo_input, foo_options, foo_key
        .quad   foo_commit, foo_cancel, foo_cursor, foo_docswitch, foo_overlay, foo_tick
```
Every callback may be 0.  All run on the UI thread:

| field | call | when |
|---|---|---|
| TD_ACTIVATE | `fn(doc)` | the tool became current (doc may be 0) |
| TD_DEACTIVATE | `fn(doc)` | another tool is chosen; a pending edit was committed before |
| TD_INPUT | `fn(doc, ui, x_fx, y_fx) -> eax 1 changed` | while the pointer is over the canvas or a button is held on it (not while the view pans).  `ui` = UIINPUT (UI_PRESSED / UI_BTN / UI_RELEASED, UI_PTS move points relative to the canvas: `view_screen_to_doc_fx((UI_RX + x) << 8 \| 128, …)`), x_fx / y_fx = pointer in image px 24.8.  **Called once per layout pass**: UI_PRESSED / UI_RELEASED only in the first pass of a frame; return 1 only when this call changed something (returning 1 unconditionally makes every pass request another one: "layout did not settle within 4 passes") |
| TD_OPTIONS | `fn()` | tool row (after "Werkzeug:"): call `opt_*` helpers (a horizontal scroll view: they may be wider than the window, U3) |
| TD_KEY | `fn(vk, mods) -> eax 1 handled` | first unconsumed key of the frame (handled = consumed); Enter / Strg+Z with a pending edit are taken by the framework first |
| TD_COMMIT | `fn(doc) -> eax 1 step` | Fertigstellen / Enter / tool or image switch / a document command |
| TD_CANCEL | `fn(doc)` | Esc, Strg+Z, CMD_TOOL_CANCEL (doc 0: the document is gone) |
| TD_CURSOR | `fn(x_fx, y_fx) -> eax CUR_*` | cursor over the canvas (fn 0 → CUR_CROSS); CUR_CROSS / CUR_ARROW show the tool's own cursor, `PNC_BASE + kind` a PaintCraft cursor (cursors.s: PNC_ROTATE) |
| TD_DOCSWITCH | `fn(old, new)` | the active image changed |
| TD_OVERLAY | `fn(doc, &canvas_rect)` | render time: handles etc. (gfx_* in absolute window px); every change must damage its old and new screen area (`tool_damage`, `nubs_*`), `--verify-dirty` checks it |
| TD_TICK | `fn(doc)` | start of every frame: poll jobs, react to option / colour / document changes |

Framework functions: `tool_cur`, `tool_prev`, `tool_set`, `tool_desc`, `tool_cfg`,
`tool_cfg_changed`, `tool_color(btn)`, `tool_pointer`, `tool_damage(sx0, sy0, sx1, sy1)`,
`tool_live_begin/end`, `tool_live`, `tool_commit_live`, `tool_cancel_live`, `tool_precmd`.
Unregistered tools keep the defaults (command label / icon, paint.net help text,
"Pinselgröße:" for Klonen, Neu färben, Linie / Kurve, Formen).

## 3. Pending ("live") edits

`tool_live_begin(doc)` when an edit starts (a brush stroke on press, a fill on click),
`tool_live_end()` when it is committed or cancelled.  While an edit is pending:
* the Fertigstellen button (CMD_TOOL_COMMIT, shown after the options of TF_LIVE tools,
  enabled only now), Enter and CMD_TOOL_COMMIT call TD_COMMIT; Esc (UIC_ESCAPE reaches
  tool.s first), Strg+Z (taken in tool_frame even when the history has nothing to undo) and
  CMD_TOOL_CANCEL call TD_CANCEL (no history entry, HIST-019);
* choosing another tool or image commits it (paint.net behaviour);
* `tool_precmd` (main.s calls it before routing any command id): Rückgängig cancels the edit
  and is consumed; view, tool, palette, window / help / settings commands and test hooks pass;
  every other command commits the edit first (save, close, tabs, image / layer / effect
  commands, clipboard, selection, redo, history jumps).  Modules that change a document
  outside a command should call `tool_commit_live()` first.
* DF_LIVE is set on the document.

A live tool keeps NO history capture open: the preview only goes through the tool layer.
The commit is one `hist_px_begin/touch/end` (one entry, HIST-020).

## 4. Shared options (TOOLCFG, `tool_cfg()`)

| field | type | default | widget (`opt_*`) | paint.net German label |
|---|---|---|---|---|
| TC_WIDTH | f64 1..2000 | 2 | `opt_width` #2000 (− #2001 / + #2002) | Pinselgröße: |
| TC_HARD | u32 % | 75 | `opt_hardness` #2010 | Härte: |
| TC_SPACING | u32 % | 15 | `opt_spacing` #2012 | Abstand: |
| TC_AA | u32 | 1 | `opt_aa` #2020 (icon) | Kantenglättung an / deaktiviert |
| TC_BLEND | u32 BM_* | Normal | `opt_blend` #2030 (icon, tooltip Füllmethode) | Normal, Überschreiben (BM_OVERWRITE), Multiplizieren, Additiv, Farbig nachbelichten, Farbig abwedeln, Reflektieren, Leuchten, Überlagerung, Unterschied, Negierung, Aufhellen, Verdunkeln, Bildschirm, XOR |
| TC_FILL | u32 FS_* | Volltonfarbe | `opt_fill` #2050 (row 2051, swatches 2052..2104) | Fülleffekt: Volltonfarbe + 53 hatch styles (HatchStyle.* names) |
| TC_TOL | u32 % | 50 | `opt_tolerance` #2110 | Toleranz: |
| TC_FLOOD | u32 | 0 | `opt_flood` #2115 (icon) | Füllungsmodus: Zusammenhängend / Global |
| TC_SAMPLE | u32 SMP_* | Ebene | `opt_sampling` #2120 (icon) | Ebene / Bild (flood tools) |
| TC_SELMODE | u32 SM_* | Ersetzen | `opt_selmode` #2130 | Auswahlmodus: Ersetzen, Vereinigen, Subtrahieren, Schneiden, Exklusiv (XOR) |
| TC_SELQUAL | u32 SQ_* | geglättet | `opt_selqual` #2140 (icon) | Auswahl mit Kantenglättung / nicht glätten |
| TC_PRESSURE | u32 | 1 | – | pen pressure scales the width (tool row "Druckempfindlichkeit", PN1 §10) |
| TC_SMOOTH | u32 | 0 | `opt_smooth` #2150 (icon, tooltip Pfadglättung) | Geglätteter Weg / Ungeglätteter Weg |
| TC_VER | u32 | | – | bumped by every change (live tools compare it) |

`[` / `]` (CMD_BRUSH_DEC / INC) change the width by 1, Strg+`[` / `]` by 5 (1..2000).  The
width number box shows 0 decimals; the engine uses the f64 (fractional widths via
pressure / hooks, DRAW-005).  A tool that writes a TC_* field itself calls
`tool_cfg_changed()`.  Per-tool widgets use ids `TOOLOPT_BASE + TOOL_* × 50`;
`opt_menu(id, &u32, n, labels, icons, tooltip, flags)` builds a flat drop-down for any
enumeration (OMF_ICONONLY, OMF_NOICON, OMF_ICON1).  Hatch patterns: 8 × 8 bitmaps in
`hatch_pats` (tool.s), foreground = the button's colour, background = the other colour,
anchored at the image origin (`fill_pattern_row`).

## 5. Tool layer (TL) — the single write path

```
tl_init(tl, doc)                                  empty TL for doc
TL_KIND = TLK_SOLID | TLK_ERASE | TLK_SOURCE, TL_MODE = BM_* | opacity << 8,
TL_MASK = coverage TMASK (0 = full box), TL_ARGB / TL_ARGB2 / TL_FILL, TL_STRENGTH,
TL_SRC / TL_SX / TL_SY, TL_FLAGS (TLF_NOSEL), box TL_X0..TL_Y1
tl_set_paint(tl, btn)                             colour / pattern / blend from the options
tl_show(tl)   tl_mark(tl, x0, y0, x1, y1)   tl_set_box(tl, ...)   tl_hide(tl)
tl_commit(tl, "Farbpinsel", IC_BRUSH) -> eax 1    one history step
```
`tl_row(tl, dst, x, y, n, layer_row)` is the compose hook (`comp_set_tool`, n ≤ 64, worker
threads): coverage = mask (tm_span) × selection (`sel_clip_row`, INT_SCALE), then
* TLK_SOLID: `k_bm_solid_row(dst, layer_row, argb, cov, n, mode)` (solid) or the pattern row
  through `k_bm_cov_row`;
* TLK_ERASE: `k_erase_row` (alpha × (1 − cov·strength));
* TLK_SOURCE: the source surface row through `k_bm_cov_row` (for moved pixels / clone stamp;
  pixels outside the source get coverage 0).

Pixels with coverage 0 stay bit-exact.  `tl_commit` touches (hist_px_touch) only the 64 × 64
tiles whose mask slot is not TM_EMPTY, runs `tl_row` in place on the layer rows of those
tiles on the worker pool (`thr_parallel_for`), removes the hook, `doc_mark_px`,
`hist_px_end`.  Preview ≡ result (tested pixel by pixel).  Note: switching the compose hook
on turns compose.s's single-layer alias off (one full recomposite at the start of a stroke).

### sel_clip_row (sel.s, selection agent)
`sel_clip_row(doc, x, y, n, out) -> eax SCR_NONE | SCR_PART | SCR_ALL` (1 ≤ n ≤ 64, worker
threads, pure).  SCR_ALL also when the document has no selection.  Test hooks 941 / 942
force a rectangular clip in tl_row independent of sel.s.

## 6. Brush engine (brush_engine.s)

A stroke = one coverage TMASK of the image size accumulated with max() (overlapping dabs
never darken; the colour is blended once through the tool layer).  `be_begin(doc, &BEP)`,
`be_dab` (press), `be_to` (every move point: dabs every spacing·width px along the path,
at least 0.25 px, the remainder carried over), `be_dirty` (area for `tl_mark`), `be_box`,
`be_end`; with BEF_SMOOTH (Pfadglättung) the dabs follow a centripetal Catmull-Rom spline
through the points (one point behind the pointer, pieces of ~2 px; points closer than ½ px
are skipped) and `be_finish` draws the last piece at the release.  Dab coverage (AA): d = distance of the pixel centre, r = width / 2 (× pressure),
R = r + ½, edge = clamp(R − d, 0, 1), inner = r · hardness, soft = smoothstep(clamp((R − d) /
(R − inner), 0, 1)), cov = min(edge, soft) (SSE, 4 pixels per step).  AA off: 255 where
d < w / 2, w = max(1, round(2r)), the centre snapped to the pixel grid (odd w: pixel centre,
even w: pixel corner) → w × w round blocks.  Whole 64 × 64 tiles inside the full-coverage
radius become TM_FULL at once (radius ≥ 40).  BEF_PENCIL: 1 px Bresenham lines.

## 7. Tools

| tool | options | behaviour |
|---|---|---|
| Farbpinsel (B) | Pinselgröße, Härte, Abstand, Fülleffekt, Pfadglättung, Antialiasing, Füllmethode | left = primary, right = secondary; stroke opacity = colour alpha; one step per stroke; Esc during the stroke cancels it; brush outline (TC_WIDTH × zoom) at the pointer |
| Radierer (E) | Pinselgröße, Härte, Abstand, Pfadglättung, Antialiasing | alpha × (1 − cov · colour alpha), colour bytes kept |
| Zeichenstift (P) | Füllmethode | 1 px aliased lines, Überschreiben supported |
| Farbeimer (F) | Füllungsmodus, Toleranz, Ebene/Bild, Fülleffekt, Antialiasing, Füllmethode, Fertigstellen | flood_fill_mask (Shift = global) on a private snapshot of the layer / composite; antialiasing = 3 × 3 tent filter of the region (edges of the image repeat); LIVE: origin nub draggable (CUR_MOVE over it), option / colour changes re-run / re-render, the document changing under it (undo in another way, other layer) re-samples; a click elsewhere commits and starts a new fill; images ≥ 4 Mpx fill on the worker pool (a newer request cancels and restarts; commit waits); `sync` waits for it |
| Farbauswahl (K) | Probennahme: Einzelne Pixel / 3 × 3 … 51 × 51 Pixel, Ebene / Bild, Nach dem Klick: Werkzeug beibehalten / Zurück zum vorigen Werkzeug / Zum Zeichenstift wechseln | left = primary, right = secondary, Strg = Bild; one pixel = its exact value, larger sizes average premultiplied (alpha-weighted, pixels inside the image); picks while dragging; the tool switch happens on release |
| Zoomen (Z) | – | left click = next preset around the point, right click = previous; a left drag (> 4 DIP) shows a rectangle, the release zooms so that it fits the canvas, centred |
| Ansicht verschieben (H) | – | drag pans (view_pan_begin / move / end per frame), hand cursor |

**Tool row overflow (U3, chrome.s):** the options after "Werkzeug:" sit in a horizontal scroll
view (#1030, `SC_H | SC_NOBAR`, gap 6); when their width exceeds the row (Formen / Text at
1280 px, every wide tool at 1024 px / 150 %) page buttons "‹" #1031 / "›" #1032 appear at the
row's end (3/4 of the view per click, disabled at the ends; decided from the last measure plus
their own width, so they do not flip).  The wheel over the row scrolls it, Tab focus is
scrolled into view.  Test: `tests/toolrow_overflow.uis`.

## 8. Tests / hooks (diag.s block T1, `cmd 930..949`, records #9930..#9942)

Commands: 930 width × 100, 931 hardness, 932 antialiasing, 933 blend mode, 934 fill style,
935 tolerance, 936 flood mode, 937 sampling, 938 spacing, 939 picker size index, 940 picker
"Nach dem Klick", 941 / 942 test clip rectangle (x0 | y0 << 16, x1 | y1 << 16; 0 = off),
943 / 944 primary / secondary test colour (A1's table), 945 bucket always on the worker pool,
946 selection mode, 947 selection quality, 948 path smoothing, 949 TLK_SOURCE test (layer
pixels [0,10)² written to [100,110)² through a source tool layer + tl_commit, history "Test").
Records: 9930 edit pending, 9931 tool, 9932 width × 100, 9933 / 9934 primary / secondary
"AARRGGBB", 9935 / 9936 history entries / position, 9937 options text, 9938 outline "x,y,r"
(value visible), 9939 bucket origin "x,y" (value = job running), 9940 previous tool,
9941 DF_LIVE, 9942 picker "size,after,sampling".

Scripts (1280 × 800, zoom 100 %: image pixel (x, y) at (240 + x, 148 + y)):
`tools_brush.uis` (AA / aliased dabs, right button, alpha, max coverage, Überschreiben, Esc,
eraser, pencil, test clip and a real rectangle selection, outline, TLK_SOURCE, smoothing), `tools_fill.uis` (live
fill, Esc / Enter / Fertigstellen / Strg+Z / tool switch / document command, tolerance,
nub drag, second click, global + Shift, colour change, pattern, AA edge, clip, Ebene / Bild,
worker pool + `sync`, X / Standardfarben), `tools_picker.uis`, `tools_view.uis`,
`tools_options.uis`.  All pass on Windows and Linux, also with `--verify-dirty`.

## 9. Open points

* paint.net draws the brush outline inverted (XOR-like); here a dark + a light ring.
* The brush type (circle / square) is not implemented; the default of Pfadglättung is
  "Ungeglätteter Weg" (paint.net's default unverified).  Pen pressure: §10.
* The fill's antialiasing is a 3 × 3 tent filter (a 1 px soft edge), not paint.net's
  polygon-based AA; flood tolerance scaling is flood.s's (to be calibrated).
* Own cursors (cursors.s, like paint.net's tool cursors): a black crosshair with a white rim
  at the hot spot (7, 7 DIP) and the tool's colourful icon (pn_icon_paint) with a white halo at
  14, 14 (32 x 32 DIP, drawn for every dpi by the library: ui_cursor_new / plat_cursor_create);
  Text keeps the I-beam, Ansicht verschieben the hand, the move tools show move inside the
  frame, resize arrows on the handles and a rotate cursor (circular arrow) outside.  Test hooks:
  #9920 (canvas cursor, which one), cmd 910 (all cursors at 100 / 150 / 200 % ->
  build/tests/cursors.png); test tests/cursors.uis.
* The single global `ui_sync_hook_set` hook is taken by tool.s (`tool_sync_pending`); other
  modules with background jobs need a chained hook (integrator).

## 10. Pen pressure (agent PN1, plan M8, DRAW-020; library: legacy/ASMLIB/docs/modules/PEN_INPUT.md)

* **Width**: Farbpinsel, Radierer, Klonen (Stempel) and Neu färben pass the pressure of every
  move point (`UI_PPRESS`) and of the pointer (`UI_PRESSURE`) to the brush engine through
  `be_ui_pressure(ui, i)` (65535 → `BE_PRESSURE_ONE` 1024, rounded, at least 1 while there is
  pressure); 0 = mouse = full width.  Without smoothing the dab radius is interpolated along a
  segment from the last point's pressure to the new one and the dab step follows the smaller of
  the two radii (no gaps when the pressure rises).  Opacity does not follow the pressure
  (DRAW-020 asks for the size).  The Zeichenstift ignores pressure.
* **Tool row**: "Druckempfindlichkeit" icon drop-down (ID_OPT_PRESSURE 2155, rows
  "Stiftdruck auswerten" / "… deaktivieren", paint.net's
  ToolConfigStrip.Brush.EnablePressureSensitivity strings) as the last widget of the four tools'
  rows; TC_PRESSURE, saved with the tool defaults (`[Tools] pressure`).  Deviation: it is shown
  only while a pen is known (`pen_known`: pen input seen in this session, `plat_pen_devices` > 0
  — asked once, never in headless runs — or pressure switched off), so mouse-only systems (and
  the 1280 px test rows, which fill the row exactly) keep their layout; with a pen the row may
  overflow and scroll (§7 overflow).
* **Eraser end**: tool.s `tool_input` — pen input of type `PTR_ERASER` over the canvas (no button
  held from an earlier frame) selects the Radierer and remembers the tool before; pen-tip input
  brings it back (only while the Radierer is still selected).  Mouse input changes nothing.
* **Einstellungen ▸ Stift & Tablett**: "Windows-Tinte" (pen input on / off), detected devices
  (docs/SETTINGS.md).
* **Test**: `tests/tools_pressure.uis` (aliased dabs, width 40): 50 % = 20 px, mouse = 40 px,
  rising 10 → 100 %, switch off = 40 px, eraser end → Radierer and back, Radierer with 50 %,
  Windows-Tinte off = 40 px; both targets, also `--verify-dirty`.

## Bitmap brush tips and dynamics

The brush and eraser option rows now expose a bitmap-tip dialog. `src/tools/brushes/custom.rs`
loads PNG/BMP/TIFF coverage and persists the tip path and dynamics; `src/tools/brushes/mod.rs`
applies bilinear tip sampling, rotation, scatter, size/angle variation, flow and
direction tracking. Normal stroke selection, pressure and one-step history remain
in use. Overlapping dabs use maximum coverage rather than repeated airbrush buildup.
The default round brush follows its previous path. See [setup, controls and
Brush Factory compatibility limits](CREATIVE_TOOLS.md).
