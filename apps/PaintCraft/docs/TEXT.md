# T5 – Text tool (T)

`src/tools/text.rs` (TOOL_TEXT, spec TEXT-001..020), tests `tests/tools_text.uis`,
`tests/manual_parity.uis` and `tools/paintcraft_manual_check.py`.

## Behaviour (paint.net 5.1)
* Left click on the canvas places the text cursor; the anchor is the top of the first line
  (its left end / centre / right end by the alignment).  The edit is **live** (TF_LIVE):
  typing inserts UTF-8 text (TEXT-017), Enter = line break (TEXT-018), Rücktaste / Entf /
  ← → ↑ ↓ / Pos1 / Ende (Strg+Pos1 / Strg+Ende: whole text) (TEXT-019), Strg+V and
  Umschalt+Einfg paste the text clipboard (CR LF → LF, tab → space, controls dropped).
  While live, character keys are text (no tool shortcuts), Entf / Rücktaste do not reach
  the selection commands.
* Caret (dark + light line, blinks 530 ms, always on in headless runs), handle (square nub
  at the lower right corner of the text) drags the whole text (TEXT-014); Strg held hides
  caret and handle.  Status bar while live: "Am Anfasser lässt sich der Text verschieben. …".
* A click inside the text moves the caret; a click outside commits and starts a new text.
  Fertigstellen / a click outside / another tool or image / a document command = one
  history step "Text" (icon IC_FONT) written into the active layer (TEXT-015, TEXT-020);
  an empty text makes no step.  Esc and Strg+Z drop the live text (no step).
* Colour = primary colour (TEXT-012); the Fülleffekt hatch uses the secondary colour as
  background; Füllmethode incl. Überschreiben (TEXT-013); selection clip via the tool layer.
* Options apply to the live text immediately.

## Tool row
`Schriftart:` [family ▾] [size] [▾] | **B** *I* U S  [Glatt ▾] [≡ ▾]  Fülleffekt, Antialiasing,
Füllmethode, Fertigstellen.

| widget | id | notes |
|---|---|---|
| family drop-down | `ID_FONT_LIST` 0x50000 | flyout: search box 0x50001 (case-insensitive substring; the first match is taken at once), virtualized list 0x50002 (rows 0x50003 + i); "Schriften werden eingelesen…" while the scan runs |
| size box | 3002 | points 1..2000, two decimal places |
| size drop-down | 3003 | flyout: [A-] 3001 / [A+] 3004 (Strg: ±5), list 0x8200F of 8…288 (rows 0x82010 + i), metric rows 3006 "Punkte (Bild DPI)" / 3007 "Fest (96 DPI)" |
| Fett / Kursiv / Unterstrichen / Durchgestrichen | 3008..3011 | toggles (real bold / italic faces via fontdb, synthetic otherwise) |
| Textdarstellung | 3012 (rows 3013..3015) | Glatt, Scharf (Modern), Scharf (Klassisch) |
| Ausrichtung | 3016 (rows 3017..3019) | Linksbündig, Zentrieren, Rechtsbündig |
| Fülleffekt, Antialiasing, Füllmethode | tool.s `opt_fill` / `opt_aa` / `opt_blend` | shared TOOLCFG |

After a click on an option button while live the keyboard focus returns to the canvas
(`ui_focus(ID_CANVAS)`).  Typed text and keys go to the text only while the canvas, a
tool-row button or nothing has the focus (not the size box / search box).

## Implementation
* Size in px = points × dpi / 72 (`view_dpi100`; Fest: 96 dpi).
* Layout: lines split at LF; `text_measure_n` per line; x by the alignment (rounded to
  whole px), line height `text_line_h`, baseline = top + `text_ascent`.
* Raster: the text rectangle + margin (size/2 + 4) clipped to the image, in 64-row bands
  (one TMASK tile row): `text_draw_n` (+ `text_decor` underline / strikeout) into an A8
  surface, `tm_combine_block` into the image-size coverage TMASK; antialiasing off =
  coverage ≥ 128 → 255.  TL = `tl_set_paint` (TLK_SOLID).  Colour fonts (`font_is_color`:
  COLR / CBDT, e.g. Segoe UI Emoji) render into PARGB bands: coverage = alpha, a straight
  colour surface (`TLK_SOURCE`, alpha = primary alpha) keeps the glyph colours.
  Only the tiles of the previous rendering are cleared per update; `tl_set_box` marks old
  and new box.  Commit: `tm_optimize`, `tl_commit(tl, "Text", IC_FONT)`.
* Fonts: `fontdb_load(family, 400/700, italic)` (+ `font_style` FS_SHARP for Scharf);
  all text / font / glyph-cache calls on the UI thread.  The fontdb scan runs once as a
  worker job when the tool is first activated (`fontdb_load_cache` → `fontdb_scan` →
  `fontdb_save_cache`, cache `<plat_config_dir>/PaintCraft/fontcache.fdb`, not written in
  headless runs); fontdb is not touched by the UI thread before the job finished.  Until
  then (or when loading fails) the embedded JFLib Sans (Selawik; FONT_REG / FONT_BOLD,
  synthetic italic) renders – identical outlines to Selawik's TTF files.  Default family after the scan:
  Segoe UI, else DejaVu Sans, Noto Sans, Liberation Sans, Ubuntu, Cantarell, Arial, else the
  first family containing "Sans".
* Keys: TD_KEY consumes the frame's keys in queue order (editing keys, Enter, Strg+V,
  character keys); typed text (`ui_text_take`) is taken in TD_TICK.  Framework change (T5):
  `TF_TEXTKEYS` (pn_tooli.inc) – tool.s leaves Enter to TD_KEY for such tools.
* Hooks 840..844 (tool_precmd lets 840..849 pass without committing): 840 test font id
  (-1 = family), 841 size pt, 842 options (bits 0-3 B I U S, 4-5 align, 6-7 render, 8
  metric), 843 anchor x | y << 16, 844: 1 wait for the scan, 2 | i << 8 caret, 3 | i << 8
  family i; 4 synthetic preedit, 5 clear preedit, 6 a 200-character IME commit.
  Records `ID_TTEXT_FIRST` + 0..9 (0x82000 = 532480): text (value live),
  "caret,line,ax,ay", family (value pt), "style,align,render,metric" (value font id), TL box
  (value scan state), "families,filtered" (value filtered > 0), font from file, text rect.

## Rendering and navigation additions
* Scharf (Klassisch) combines grid fitting with integer device-pixel kerning and advances
  (`FS_CLASSIC`); Scharf (Modern) preserves fractional advances. Measurement and drawing
  share this behavior. Pixel tests distinguish all three modes on Windows and Linux.
* Strg+←/→ jumps over words, whitespace and punctuation without splitting UTF-8 sequences.

* Fractional point sizes retain their precision through the 24.8-pixel conversion.
* Shift+navigation and mouse dragging select live text. Ctrl+A selects all; Ctrl+C/X/V
  copy/cut/paste; typing, Backspace and Delete replace/remove the selection. Ctrl+Shift+
  arrows select whole words. Highlight rectangles are overlays and never enter the image.

## OpenType shaping, bidi and fallback

The text tool scopes `Text::set_shaping(true)` around measurement, rasterization,
carets and selection. Ordinary app widgets retain their previous text metrics.
`lib/text_shape` uses pinned HarfRust 0.14.0, Unicode Bidi and Unicode Script in a
`no_std` Cargo subproject. GSUB/GPOS shaping handles joining, ligatures and mark
placement; Unicode bidi runs are laid out in visual order. UTF-8 clusters are
retained for hit testing and editing. A logical selection may produce several
visual rectangles across a bidi boundary. Multi-character ligature carets are
interpolated within their cluster; these are not font-provided ligature caret tables.

After the font scan, non-ASCII text enables a fallback list of installed families
including Noto, Segoe UI, Arial, Nirmala UI, Yu Gothic, Microsoft YaHei and DejaVu.
A fallback must cover a complete script run so that Arabic joining is not split
character by character. Coverage depends on installed fonts. This is a finite
family list, not the operating system's full font-fallback service. Synthetic
fallback styles, all emoji sequences, arbitrary language-specific OpenType features
and every Indic script/font combination have not been exhaustively validated.
`--text-font FILE` selects a TrueType/OpenType fixture for deterministic UI tests.

## IME composition

Unconfirmed composition text is a separate overlay beside the text caret and is
not painted into the layer or inserted into the edit buffer. Windows uses
`WM_IME_COMPOSITION` / IMM; Linux Wayland binds text-input-v3 version 1 and handles
enter/leave, preedit, commit and candidate positioning. Each Wayland commit uses
a bounded UTF-8 payload event, avoiding the per-character event-ring limit.
The payload limit is 4096 bytes; the application text buffer has its own limit.
Escape/end of editing clears the overlay. A completed text edit remains one Undo step.

The native candidate chooser belongs to the operating system/input method. These
bindings have been compiled, and synthetic preedit/commit behavior is tested on
both desktop targets. No real Japanese/Chinese IME candidate session has yet been
accepted on Windows or a Wayland compositor. X11 XIM/IBus integration, surrounding-
text deletion/reconversion and detailed composition segment styling remain open.
The generic UI edit controls receive committed text but do not yet share the text
tool's custom preedit overlay.

## Tests

```powershell
$env:PAINTCRAFT_SHAPING_FONT = 'C:\Windows\Fonts\arial.ttf'
cargo test --locked --manifest-path lib/text_shape/Cargo.toml --target-dir build/paintcraft-next/cargo-shaping-tests
python tools/paintcraft_text_language_check.py --out build/paintcraft-next --target both
```

On Linux, set `PAINTCRAFT_SHAPING_FONT` to an installed font covering Latin, Arabic
and Hebrew. The UI script also accepts `--font FILE`. Test fonts are copied only
into ignored build directories and are not redistributed. Five shaping tests cover
Arabic joining/lam-alef, mixed Hebrew/Latin bidi, combining marks, forced font
fallback and malformed input. Sixteen desktop UI cases cover English/French UI
labels and saved language choice, synthetic preedit, a 200-character atomic IME
commit, disjoint bidi selection, clipboard editing and text Undo/Redo.

## Deviations / open points

* Arrow keys still navigate logical UTF-8 order; complete visual bidi navigation,
  grapheme-cluster deletion and both caret affinities at direction boundaries are open.
* Font list rows show names in the UI font, not each family's own typeface.
* The size list is 8…288 as in the book; the exact Paint.NET list is unverified.
  Size Inc/Dec buttons live in the size drop-down.
* Hatch fill does not apply to color-font glyphs. Color fallback in an otherwise
  monochrome run and synthetic-bold shaped advances need further visual coverage.
* No editable text layers: committing text rasterizes it, as before.
* Handle position and caret appearance have not been checked against Paint.NET.
