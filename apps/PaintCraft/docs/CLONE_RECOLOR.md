# T6 – Klonen (Stempel) and Neu färben

File `src/t_clone.s` (`tclone_init` registers TOOL_CLONE / TOOL_RECOLOR, `tclone_cmd` = test
hooks 845–848).  Tests `tests/tools_clone.uis`, `tests/tools_recolor.uis` (both targets, also
`--verify-dirty`).  Spec DRAW-013..019.  No changes to other files.

## Shared stroke machinery

Both tools use the brush engine (`be_begin/dab/to/dirty/end`: Pinselgröße, Härte, Abstand,
Antialiasing, pen pressure; no path smoothing) and ONE `TLK_SOURCE` tool layer (`tl_*`, clipped
by `sel_clip_row`).  Press → `tool_live_begin`; release → `tool_commit_live` → `tl_commit` = one
history step named like the tool ("Klonen (Stempel)", "Neu färben"; icons IC_PICTURES /
IC_CRAYON).  Esc / Strg+Z during the stroke cancel it (no step).  A stroke that changes no pixel
leaves no step (hist_px).

## Klonen (Stempel) (L)

| | |
|---|---|
| options | Pinselgröße · Härte, Abstand · Antialiasing, Füllmethode (all BM_*, incl. Überschreiben) |
| Strg + left press | anchor = image pixel + the current layer (kept by `LY_UID`, so the source can be another layer, DRAW-017); clears the offset |
| left press | no anchor for this image → an error text in the error bar (`fio_error`), no stroke.  Otherwise the first stroke after Strg+click fixes the offset = anchor − press pixel (integer, "aligned": later strokes keep it) |
| painting | dest (x, y) = source layer (x + dx, y + dy) through `TLK_SOURCE` with TC_BLEND; source pixels outside the layer = coverage 0 (unchanged) |
| no self-copy | the preview reads the unchanged source layer; the commit (in place on worker threads) first copies the source area of the stroke box (`cl_snapshot`) when source = target layer |
| crosshair | TD_OVERLAY: dark bars + white lines, ±8 px, at the anchor, after the first stroke at pointer + offset (follows the pointer); damaged via `tool_damage` |
| gone layer | anchor layer deleted → the current layer is the source |

## Neu färben (R)

| | |
|---|---|
| options | Pinselgröße · Härte, Abstand · Toleranz, Alpha-Modus der Toleranz (Vormultipliziert = default / Gerade), Probennahme (einmalig / Sekundärfarbe = default) · Antialiasing |
| buttons | left: old = secondary (or the colour under the press point), new = primary; right: roles swapped (old = primary / clicked colour, new = secondary) |
| match | flood.s CheckColor: Σ_RGB ⌊(1 + d²)·cmp.A / 256⌋ + (cmp.A − px.A)² ≤ tol²·4, tol = round(TC_TOL · 2.55); in "Vormultipliziert" cmp and px are premultiplied first (transparent pixels become alike) |
| recolour | a match becomes clamp(px.c + new.c − old.c) per colour channel, alpha kept (shading preserved; the classic paint.net shift); written with Überschreiben = lerp(px, result, coverage), so non-matching pixels stay bit-exact |
| implementation | a recoloured copy (`g_tc_sf`, image size, `surf_alloc` per stroke) is filled for every dirty area of the engine before it is marked; the tool layer lerps it in |

## Hooks / records

`cmd 845 x|y<<16` anchor on the current layer · `846 0` clear anchor + offset, `846 1` offset
only · `847 m` Probennahme (0 einmalig, 1 Sekundärfarbe) · `848 m` alpha mode (0 Vormultipliziert,
1 Gerade).  Records (tool row): #532736 anchor "x,y,layer" (value set), #532737 offset "dx,dy"
(value fixed), #532738 crosshair "sx,sy" (value shown), #532739 "sampling,alpha" (value stroke
running), #532740 "OLD,NEW".  Option ids: Probennahme 2950 (rows 2951–2952), Alpha-Modus 2955
(rows 2956–2957).  Scripts set the anchor with `drag P P 1 ctrl` (Strg+click).

## Unconfirmed paint.net details (chosen behaviour)

* Default Probennahme = Sekundärfarbe (classic behaviour); paint.net 5.1's default unverified.
* Abstand is shown for both tools; Clone has no Ebene/Bild sampling (paint.net copies from the
  anchor's layer); right button does nothing for Klonen.
* paint.net shows the no-anchor error as a message box, here the error bar.
* Crosshair look (paint.net draws its own source marker); source outside the image = unchanged.
* Recolour formula of paint.net 5 is not documented (book: only "replaces one colour with
  another"); hard tolerance threshold, no soft edge; premultiplied mode = the same CheckColor on
  premultiplied bytes.
* The Alpha-Modus der Toleranz is private to Neu färben here; in paint.net it is shared with
  Zauberstab / Farbeimer (flood.s only implements "Gerade").
