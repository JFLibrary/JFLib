# K1 – PaintCarft: Zwischenablage and file drops (`src/clip.s`)

Library side: `legacy/ASMLIB/docs/modules/CLIPBOARD_DND.md` (CLIP_IMAGE, drops, headless `clip` /
`drop` script commands).

## 1. Commands (160–171) and states

| command | key | action | history (paint.net 5.1 DE) |
|---|---|---|---|
| Ausschneiden | Strg+X | Kopieren, then the selected pixels become transparent (no selection: the whole layer) | Ausschneiden (IC_CUT) |
| Kopieren | Strg+C | current layer inside the selection bounds (no selection: the whole layer), alpha × coverage, CLIP_IMAGE | – |
| Sichtbares kopieren | Strg+Umschalt+C | the same from the composite (`comp_surface`) | – |
| Einfügen | Strg+V | floating pixels (`tmove_paste`, Pixel verschieben) | Einfügen (IC_PASTE) at the commit |
| Einfügen als neue Ebene | Strg+Umschalt+V | new layer above the current one, then Einfügen | Neue Ebene + Einfügen |
| Einfügen als neues Bild | Strg+Alt+V | new image of the clipboard size (white background), Einfügen at 0, 0 | Neues Bild + Einfügen |
| Auswahl kopieren | Strg+Alt+Umschalt+C | `sel_geom_text` as CLIP_TEXT | – |
| Auswahl einfügen ▸ Ersetzen / Hinzufügen (Union) / Ausschließen / Schneiden / Umkehren ("xor") | Strg+Alt+Umschalt+V (Ersetzen) | `sel_geom_parse` of the clipboard text, else the clipboard image's rectangle at 0, 0; `sel_combine` SM_* | Auswahl einfügen / …: Vereinigen / …: Subtrahieren / …: Überschneiden / …: Umkehren ("xor") |

States (`clip_states`, every frame): Ausschneiden / Kopieren / Sichtbares kopieren need an
image; Einfügen / Einfügen als neue Ebene an image and `ui_clip_has(CLIP_IMAGE)`; Einfügen als
neues Bild only the clipboard image; Auswahl kopieren a selection; Auswahl einfügen ▸ * an image
and clipboard text or an image.  `ui_clip_has` reads no data (Windows: IsClipboardFormatAvailable,
Wayland: offer MIME mask, X11: cached TARGETS).

## 2. Einfügen

* Position: the top left of the visible canvas area (0, 0 while the image origin is visible),
  moved back so that the pasted image stays inside the canvas when it fits.
* Larger than the canvas: modal question "Einfügen" (window WINID_DLG0 + 0x70) with the
  paint.net texts (ExpandCanvasQuestion.*): **Leinwand vergrößern** (`iop_canvas`, anchor top
  left, the new area of background layers in the secondary colour; own step "Leinwandgröße"),
  **Leinwand nicht ändern**, **Abbrechen** / Esc / close button.
* `tmove_paste(doc, sf, x, y)` (K1 block in `t_move.s`): switches to Pixel verschieben,
  saves the old selection (`sel_hsnap`), selects the pasted rectangle, makes the
  image the floating pixels F (nothing lifted, no hole) and starts the live session.  Moving,
  scaling, rotating, Probenahme and the handles work as for moved pixels.  Fertigstellen / Enter
  / another tool / any document command commits ONE compound step "Einfügen" (selection before =
  the old selection, after = the transformed rectangle; pixels through `mv_row`); Esc / Strg+Z
  drop the paste and the old selection comes back.
* Errors (error bar, `fio_error`): "In der Zwischenablage liegt kein Bild." (Einfügen), "In der
  Zwischenablage liegt kein Bild, aus dem ein neues entstehen kann." (Einfügen als neues Bild),
  "Nichts zum Einfügen: Die Zwischenablage hat weder Auswahl noch Bild." (Auswahl einfügen),
  "Die Zwischenablage hat die Daten nicht angenommen." (Kopieren).

## 3. Drag and drop (UIC_DROP)

No image open: every file is opened (`fio_open_path`).  Otherwise the modal question "Drag &
Drop" (WINID_DLG0 + 0x71; DragDrop.OpenOrImport.* texts, singular / plural): **Öffnen** (every
file), **Ebene(n) hinzufügen** (`lay_import_paths`: A4's import with its own canvas question),
**Abbrechen** / Esc.  The dropped list is copied (the library's list is replaced by the next drop).

## 4. Test hooks, ids

* Commands 172–174 (they commit a pending edit first, like every command): `cmd 172 X|Y<<16` →
  #0x82200 "W,H,AARRGGBB" of the decoded clipboard image (value 1; "none" / 0 without one);
  `cmd 173` → #0x82200 "docs,layers,cur,W,H", value = enabled commands (1 Einfügen, 2 In neue
  Ebene, 4 In neues Bild, 8 Auswahl kopieren, 16 Auswahl einfügen, 32 Kopieren); `cmd 174 A`
  answers an open question (paste 0 / 1 enlarge / 2 keep; drop 0 / 1 open / 2 layers).
  Internal 175 / 176 = the answers of the two questions.
* Record #0x82201: value = open questions (1 Einfügen, 2 Drag & Drop).
* Widget ids 0x82200..0x822FF (K1 block of ID_FREE_FIRST): 0x82210 Leinwand vergrößern,
  0x82211 beibehalten, 0x82220 Öffnen, 0x82221 Ebene(n) hinzufügen.  Windows WINID_DLG0 + 0x70
  / + 0x71.
* Tests: `tests/clipboard.uis`, `tests/clipboard_dib.uis`, `tests/drop.uis`;
  `tools/clip_check.py` (headless variants; `real-win`, `real-x11`, `real-wl` with real windows).

## 5. Unconfirmed paint.net details

* Paste position rule (top left of the visible area, clamped into the canvas) and "Einfügen als
  neues Bild" with a white background are from paint.net 3.x knowledge, not from the book.
* Esc drops a paste here (the brief: like the normal move); paint.net keeps the pasted pixels.
* Auswahl einfügen from an image uses its rectangle at 0, 0 (the error text suggests images
  count; the position is a guess).
* Drag and drop without an open image opens the files directly (paint.net has an "add as layers
  to a new image" variant for several files: not done).
* Copy / paste keep only the rectangle: paint.net's own "MaskedSurface" format (the selection
  shape) is not exchanged; the shape travels as alpha.
