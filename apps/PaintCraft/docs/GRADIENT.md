# T3 – Farbverlauf (gradient tool, G)

Spec §2.6 (Gradienten), DRAW-009/010/011/012/013/014.  One file: `src/t_gradient.s`
(`tgrad_init` registers TOOL_GRADIENT, `tgrad_cmd` = test hooks 792–794).  Kernels:
`grad_span` / `grad_alpha_span` of `legacy/ASMLIB/src/gfx/gradient.s` (docs/modules/RESAMPLE.md).

```
python build.py app PaintCraft --target both --out build/t3
build/t3/win/PaintCraft.exe --script apps/PaintCraft/tests/tools_gradient.uis [--verify-dirty]
python tools/pn_gradient_check.py --target both --out build/t3      # bit-exact vs. Python
```

## 1. Behaviour (paint.net 5.1)

| input | effect |
|---|---|
| drag (left) | start = press point, end = pointer; primary → secondary colour.  A click without movement draws nothing (the pending edit is cancelled) |
| drag (right) | same with the colours reversed (secondary → primary) |
| after the drag | LIVE: two end nubs + a middle nub (moves the whole gradient); dragging a nub keeps the grab offset; CUR_MOVE over nubs |
| Shift while dragging (new gradient or an end nub) | angle around the other end point snapped to 15° steps (length kept; nearest of 24 directions) |
| right click on a nub | colours swapped (as in paint.net) |
| press away from the nubs | the live gradient is committed, a new one starts |
| colour / option / selection change while live | re-rendered (TC_VER, `pal_version`, DOC_VER_SEL) |
| Enter / Fertigstellen / tool or image switch / document command | one history step „Farbverlauf“ (icon IC_CONTRAST); an unchanged layer makes no step (hist) |
| Esc / Strg+Z while live | dropped, no step |

Options (tool row):

| widget | id | values |
|---|---|---|
| type | 2160 (rows 2161–2167) | Linear, Linear (gespiegelt), Raute, Radial, Konisch, Spirale rechtsdrehend, Spirale linksdrehend = GT_0..6 |
| mode | 0x80010 (rows +1..+2) | Farbverlauf / Transparenzverlauf (GradientIsAlphaOnly) |
| repeat | 0x80020 (rows +1..+3) | Keine Wiederholung / Periodisch wiederholen / Gespiegelt wiederholen = GR_NONE / WRAP / REFLECT |
| Antialiasing, Füllmethode | `opt_aa`, `opt_blend` | shared TC_AA / TC_BLEND (Normal … XOR, Überschreiben) |

Options live in t_gradient.s globals (default Linear, Farbverlauf, Keine Wiederholung); not
saved in the settings file (tool.s saves no tool options yet).

## 2. Rendering

* Box = image ∩ `sel_bbox` (when a selection exists); the tool layer clips to the selection
  itself (`sel_clip_row`, antialiased edges included).
* Colour mode: `grad_span` into a private straight FMT_ARGB surface of the image size;
  TL = TLK_SOURCE, mode TC_BLEND | 255 << 8 (Normal … Überschreiben via `k_bm_cov_row`).
  Colours with alpha lerp straight per channel (gradient.s: c = (A (16384 − T) + B T + 8192) >> 14).
* Transparenzverlauf: ramp = primary.a → 255 − secondary.a (right button / swapped: reversed);
  this is paint.net's alpha-only rule (defaults black / white = opaque → transparent).  Coverage
  mask (TMASK) = 255 − ramp, TL = TLK_ERASE, strength 255 → a' = ((a · ramp + 128) · 257) >> 16,
  colours kept; the blend mode is ignored.  Tiles of the box are materialised on the UI thread,
  the workers only fill them, `tm_optimize` turns uniform tiles into sentinels.
* Antialiasing (TC_AA) only where the gradient has hard edges: Periodisch wiederholen (every
  type) and the spirals.  4 rotated-grid samples (±0.375, ±0.125) = 4 GRADs with the end
  points shifted; pixel = copy when the 4 samples are equal, else alpha-weighted mean
  a = (Σa + 2) >> 2, c = (Σ c·a + Σa / 2) / Σa.  Transparency: ramp = (Σ + 2) >> 2.
* Worker pool: `thr_parallel_for(tg_band, ctx, bands)`, one item per 64-row band, chunks never
  cross a 64-column tile; only `grad_span` / `grad_alpha_span` and pure code run in workers
  (results independent of `--threads`, checked).  Synchronous: every change re-renders the box
  (4000 × 3000: opening + 3 renders incl. a spiral with AA + commit took 0.44 s in total,
  1.1 s with `--threads 1`).
* Commit = `tl_commit(tl, "Farbverlauf", IC_CONTRAST)` — the same `tl_row` as the preview.

## 3. Test hooks / records

`cmd 792 type | repeat << 8 | transparency << 16` options · `cmd 793 x | y << 16 (| 1 << 31 =
right button)` start point · `cmd 794 x | y << 16` end point: a new live gradient on the active
image (or the live one re-placed); points are pixel centres; only while the tool is current.
Records (emitted in the tool row, i.e. while G is the current tool): `#524288` text
"sx,sy,ex,ey" (image px, floor), value 0 none / 1 live, nothing shown / 2 shown; `#524289` text
"type,repeat,transparency,swapped,aa-samples", value drag state (0 none, 1 middle, 2 start,
3 end, 4 new).

Tests: `tests/tools_gradient.uis` (interaction; Windows + Linux, also `--verify-dirty`);
`tools/pn_gradient_check.py` (85 cases on a 200 × 150 RGBA fixture: 7 types × 3 repeats ×
colour / transparency / Überschreiben with alpha colours, AA, right button, degenerate; plus
1100 × 1200 with default threads and `--threads 1`; FNV-1a of the whole layer (hook 995)
against the Python reference built on `tools/gradient_check.py`).

## 4. Open points / unconfirmed paint.net details

* Icons of the types / modes in paint.net's tool bar are not in the icon font → text
  drop-downs (OMF_NOICON).  (U3: nubs.s draws NK_SQUARE as a square, so the middle nub
  differs from the round end nubs.)
* The dynamic status text while adjusting („Versatz: … Winkel: …°“) is not shown (TD_HINT is
  static).  paint.net 5 records intermediate in-tool history items (for moving the gradient,
  swapping its colours …); here the whole live edit is one step „Farbverlauf“ (tool contract).
* Unconfirmed: whether paint.net 5.1 has a separate move handle (the middle nub is our choice);
  the exact AA algorithm (paint.net supersamples differently); whether the blend mode button is
  disabled in Transparenzverlauf; gamma mode (paint.net 5 „Gamma-Modus“) not implemented.
