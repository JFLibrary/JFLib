# A2 – PaintCraft: Verlauf (history), Verlauf window, Werkzeuge window

Per-document undo/redo with **swap semantics** (an entry holds the "other" state; undo and redo
exchange it with the document), a memory budget over all documents (deflate → spill file →
drop), the "Verlauf" and "Werkzeuge" tool windows, test hooks and tests.  HIST-001..020 are
covered as far as they do not depend on later tools (see §8).

```
python build.py app PaintCraft --out build/a2
build/a2/win/PaintCraft.exe --script apps/PaintCraft/tests/history.uis [--verify-dirty]
build/a2/win/PaintCraft.exe --script apps/PaintCraft/tests/toolswnd.uis [--verify-dirty]
wsl.exe -d Ubuntu-24.04 --cd <repo> -e ./build/a2/linux/PaintCraft --script apps/PaintCraft/tests/history.uis
```

## 1. Files

| file | content |
|---|---|
| `inc/pn_hist.inc` | the contract (API, resource ownership rules, sel hooks) + internal records (HD_/HE_/HT_) |
| `src/hist.s` | module (hist_modinit/states/frame/cmd), entries, pixel capture, apply (swap), navigation, compound, budget policy |
| `src/hist_mem.s` | small-block allocator, 16 KB tile slabs, tile kernels (SSE2), deflate on the pool, zpool, spill file, parallel entry loader |
| `src/hist_diag.s` | test hooks `cmd 950..978`, probe window "Verlauf-Test" (test mode only), `hist_state_hash` |
| `src/wnd_history.s` | Verlauf window |
| `src/wnd_tools.s` | Werkzeuge window |
| `tests/history.uis`, `tests/toolswnd.uis` | tests (both targets, also `--verify-dirty`) |

## 2. How tools / effects / commands record history

All calls: UI thread, `doc` = DOC*.  Names are `.rodata` strings (the German command names,
e.g. "Farbeimer", "Farbverlauf", "Neue Ebene", "Ebene löschen", "Größe ändern",
"Leinwandgröße", "Auswahl aufheben"), icons IC_* (0 = IC_EDIT).

**Pixels** (brush, fill, effect, anything that writes layer pixels):
```asm
        mov     rcx, rdi                # doc
        mov     edx, -1                 # layer index (-1 = DOC_CUR)
        call    hist_px_begin           # eax 0 / ERR_*
        ...for every region before writing it:
        mov     rcx, rdi
        mov     edx, x0                 # [x0, x1) x [y0, y1), layer pixels, clipped
        mov     r8d, y0
        mov     r9d, x1
        mov     [rsp + 32], eax         # y1
        call    hist_px_touch           # saves the 64x64 tiles not saved yet (cheap after)
        ...write the pixels, doc_mark_px(doc, layer, x0, y0, x1, y1)...
        mov     rcx, rdi
        LSTR    rdx, "Farbpinsel"
        mov     r8d, IC_BRUSH
        call    hist_px_end             # eax 1 = step recorded (0: nothing changed)
```
Esc / cancel: `hist_px_cancel(doc)` puts the saved tiles back (no step, HIST-019).
Unchanged tiles are dropped at `hist_px_end`, one-colour tiles shrink to 4 bytes, zero changed
tiles = no step.  A second `hist_px_begin` while a capture is open commits it as "Bearbeiten".
While a capture or a compound is open, undo/redo/jump/clear are refused (commands disabled).

**Other state** — call BEFORE the change; `eax 0` = out of memory: do not make the change (the
message is shown already):
* `hist_push_layer_props(doc, i, name, icon)` — name / opacity / blend / visibility of layer i
* `hist_push_layers(doc, name, icon)` — layer list (add, delete, duplicate, move, merge)
* `hist_push_doc(doc, name, icon)` — image size / resolution / all layers (resize, canvas size,
  rotate, flip image, crop, flatten): the old surfaces move into the entry, nothing is copied
* `hist_push_sel(doc, name, icon)` — selection (sel.s registers `hist_sel_hooks`)

**Live tools / multi-part operations** (HIST-020): `hist_begin_compound(doc)` … any records …
`hist_end_compound(doc, name, icon)` → one step (nestable; one record inside = that record
renamed; none = no step; an open capture is committed into it).

**Resource ownership** (the one rule every layer-changing module must follow): structure/doc
entries keep LAYER records by value and share surface pixels / names / metadata with the
document.  When the document stops using such a resource (removed layer, replaced surface,
old name), take the record out of `DOC_LAYERS` first, then `hist_release(doc, &rec)` (zeroes
what the history still uses), then free the rest (`lay_free_rec`).  layer.s already does this
in `lay_delete`, `lay_set_name`, `lay_merge_down`.  Caches (LY_THUMB, LY_TDIRTY) never go into
entries; layers returning by undo/redo get them from the same uid or 0.  `DOC_LAYERS` must be a
mem_alloc block (arrays are swapped).  After a size change the history calls `doc_resized`.

**Document life cycle**: `hist_init(doc, "Neues Bild", IC_PAGE)` for new images,
`hist_init(doc, "Datei öffnen", IC_FOLDER_OPEN)` after loading (paint.net strings
`NewImageAction.Name` / `OpenImageAction.Name`); without it the first frame creates a
"Neues Bild" base.  `hist_mark_clean(doc)` after saving (undo back to that state clears
DF_MODIFIED).  `hist_free(doc)` in doc_close before the layers are freed (doc.s does it).

## 3. Data model (inc/pn_hist.inc)

* DOC_HIST block: entry pointer array (`HD_ENT`, mem_alloc, grows ×2), `HD_N`, `HD_POS`
  (entries 0..pos-1 applied; entry 0 = the base), `HD_CLEAN` (saved pos), capture record,
  open compound, depth, flags.
* Entry (128 B, small-block pool): kind (BASE / PX / PROPS / LAYERS / SEL / COMP), name, icon,
  global sequence number (oldest first across documents), RAM / disk bytes.
  PX: layer index + uid, bbox, layer size, tile array (32 B per tile: tx, ty, state, flags,
  length, pointer, slab/block).  Tile states: RAW (16 KB, pitch 256), SOLID (colour only),
  Z (zlib stream in 1 MB zpool blocks), DISK (spill file offset; zlib or raw).
  PROPS: name pointer (shared), opacity, blend, visibility.  LAYERS: LAYER array + capacity,
  DOC_CUR, DOC_W/H, resolution.  SEL: opaque blob.  COMP: child entries (applied backwards
  for undo; a failing child swaps the done ones back).
* Swap: undo and redo run the same exchange; a new step after undo destroys the redo branch
  (HIST-008); resources are freed only when neither the document nor another entry uses them
  (`hist_ref` scan).  `HIST_MAX_DEFAULT` = 1000 entries per document (the oldest is dropped).
* Navigation: `hist_jump` steps entry by entry with one batch of `doc_mark_px` per layer
  (structure changes flush first) and one `doc_mark_layers` / `doc_resized` at the end, then
  `doc_set_modified(pos != clean)`.  `CMD_HIST_JUMP` arg = entry index (0 = first entry).
* Pixel entries are loaded completely before any swap (`hm_load_entry`: destination tiles
  first, spill-file runs read at once, inflate on the worker pool), so a failed load changes
  nothing.

## 4. Memory budget (HIST-012..014)

* Budget = min(40 % of `plat_mem_total`, 4 GB) (1 GB when unknown) over all documents;
  `hist_budget_set(bytes)` (0 = default) — for the settings dialog ("Verlaufsbudget");
  `hist_temp_dir_set(path)` for "Temp-Ordner" (default `plat_temp_dir`).
* Counted: raw tiles (16 KB), deflate payloads, records, arrays, and surfaces that only a
  structure/doc entry still keeps (recounted before every enforcement).
* After every new step and navigation `hist_enforce`: (1) deflate level 1 the raw tiles of the
  oldest pixel entries (`thr_parallel_for`, tiles independent; tiles that do not shrink by 1 KB
  stay raw), (2) move the RAM tiles of the oldest entries into the spill file
  (`plat_file_open_temp`, deleted on close / process exit; append with a first-fit free-extent
  list; one gathered write per entry), (3) drop the oldest steps (oldest applied step of any
  document, else newest redo step; never the step just added) and show "Ältere
  Verlaufsschritte wurden verworfen, um Arbeitsspeicher freizugeben."
* Allocation failure never crashes: a tile that cannot be saved first triggers
  `hist_reclaim` (squeeze to half), then the capture fails → the history of that document is
  cleared (the image stays) with "Arbeitsspeicher knapp: Der Verlauf wurde geleert.";
  `hist_push_*` return 0 with "Der Arbeitsspeicher reicht für diesen Schritt nicht aus."
* Messages: `hist_notify_set(fn)` (fn(IB_*, msg); for a main-window info bar) and always in the
  Verlauf window above the list (word-wrapped, close button).
* Measured (Ryzen, 12 threads, 4000×3000 layer, 2961 tiles = 48 MB, nearly incompressible test
  pattern): undo/redo with raw tiles 22 ms; with deflated tiles 74 ms (1 thread 240 ms); with
  spilled tiles 100 ms (184 ms); deflate of all tiles 380 ms (1 thread 1240 ms); deflate +
  spill 410 ms.  Deflate scales only ~3.3× because every `zlib_deflate` call maps and frees
  ~340 KB of state (see §7).

## 5. Windows

**Verlauf** (`wnd_history.s`, WINID_HISTORY, `WF_TOOL | WF_RESIZABLE`, 208 × 260 DIP, default
top right below the tool bars): virtualised `ui_list` (`ID_HIST_LIST`, rows `+1+i`, 28 DIP) of
the active document (HIST-005/010): icon + name, the current entry selected, redo entries greyed
(BOX_DISABLED), a click jumps (HIST-006/007), auto-scroll to the current entry after every
history change; bottom bar `4101` Zurück zum Anfang (IC_PREVIOUS), `4102` Einen Schritt
zurück, `4103` Einen Schritt vor, `4104` Vor bis zum Ende (IC_NEXT) — the tooltips are these
labels.  Keys stay with the
main window (Strg+Z / Strg+Y), as the library gives tool windows the keyboard only for text boxes.

**Werkzeuge** (`wnd_tools.s`, WINID_TOOLS, `WF_TOOL | WF_AUTOHEIGHT`, 112 DIP wide, default
left below the tool bars): 19 buttons (51 × 32 DIP, 2 columns, ids `4000 + TOOL_*`, icons from
the command table), the current tool checked (`ui_tool_button` on a copy of `tool_cur()`), a
click runs `CMD_TOOL0 + i` (tool.s), tooltips "Rechteckige Auswahl (S)" etc.  (The width is
dictated by the title bar: see §7.)

Both windows follow `chrome_wnd_open(WND_*)` (F5 / F6 / title-row toggles / close button).
`whist_reset_pos()` / `wtools_reset_pos()` restore the default place (for CMD_WND_RESET).

## 6. Tests

`history.uis` (≈ 370 lines): pixel steps, Strg+Z/Y, list clicks, all four buttons,
`CMD_HIST_JUMP`, redo branch discarded, unchanged edit = no step, cancel, add/delete layer,
opacity, rename, canvas size (400 × 300) and back across the size change, compound, selection
step, saved state ↔ DF_MODIFIED, Verlauf löschen, two documents (HIST-004/011), 42 steps + list
scroll, budget 512 KB (deflate only), 12 KB (all 43 tiles in the spill file), no spill file +
1 KB (oldest dropped + message), a compound of two structure steps undone and then discarded (shared layers freed once), out-of-memory (history cleared + message, a property push is
refused, no crash), the real layer.s commands (add, duplicate, merge down, visibility, to
bottom, delete) undone/redone exactly, F6.  Every state check compares an FNV hash of the whole
document (`hist_state_hash`: size, layers, properties, names, pixels) with saved slots.
`toolswnd.uis`: layout (two columns, rects), default tool, clicks → tool.s (status bar hint,
tool menu), tool keys, tooltips, F5, close button, title-row toggle.
Results: Windows and Linux exit 0, also with `--verify-dirty` (skeleton, docs, layers_basic
pass too).  asmcheck: 0 errors, 0 warnings in my files (3 `assume` annotations for table
counters).

Hooks (`cmd 950 + k`, hist_diag.s): 0 probes on/off · 1/2 rectangle · 3 colour · 4 fill ·
5 pattern · 6 fill+cancel · 7 add layer · 8 delete layer (contract way) · 9 opacity · 10 canvas
size · 11/12 save/compare state hash · 13 budget KB (+enforce) · 14 fail from the n-th allocation
on · 15 spill on/off · 16 compound · 17 selection step · 18 new document · 19 activate document ·
20 enforce · 21 mark clean · 22 rename · 23 n small steps · 24 clear message · 25/26/27 timed
undo/redo/enforce (`t.us=`) · 28 budget without enforcing · 29 compound of two layer additions.  Probes (window 0x60FE, only in test
mode): `t.n= t.pos= t.clean= t.layers= t.cur= t.size= t.opac= t.name= t.mod= t.undo=… redo=…
t.raw= t.solid= t.z= t.disk= t.docs= t.cmp= t.msg= t.file= t.us=`.

Screenshots: `build/a2/{desk,hist,tools,hist_msg}_{light,dark,dark150}.png`,
`build/a2/*_linux_light.png`.

## 7. Changes outside my files / requests

Done (small, additive):
* `main.s`: `mod_init` calls `hist_modinit` instead of `hist_init` (the contract's
  `hist_init(doc, name, icon)` is the per-document function).
* `diag.s`: A2 block routing `CMD_HIST_TEST0..1` to `hist_diag_cmd` (A1 kept it).
* `tools/icons.txt` section "PaintCraft (agent A2)": `IC_PREVIOUS` E892, `IC_NEXT` E893;
  `legacy/ASMLIB/assets/icons.bin` + `legacy/ASMLIB/inc/icons.inc` regenerated with
  `ttf2bin.py --font SegoeIcons.ttf --icons tools/icons.txt --inc …` (icons only; diff additive).

Requests:
* A1 / fileio: call `hist_init(doc, "Neues Bild" | "Datei öffnen", IC_PAGE | IC_FOLDER_OPEN)`
  in Neu / after loading, `hist_mark_clean(doc)` after saving.  Every future imgops / clip / fx
  code must record (§2) and follow the ownership rule.
* chrome.s: `CMD_WND_RESET` → `wtools_reset_pos()` / `whist_reset_pos()`; optionally
  `hist_notify_set` → a main-window `ui_infobar`.
* Settings agent: "Verlaufsbudget" → `hist_budget_set`, "Temp-Ordner" → `hist_temp_dir_set`;
  persist `g_wt_rect` / `g_wh_rect` (WINRECTs in wnd_tools.s / wnd_history.s).
* sel.s: `hist_sel_hooks(snapshot, restore, free, size)` in sel_init.
* Library (ui): (1) `TX_WRAP` is not implemented by `ui_text` (the Verlauf window wraps its
  message itself); (2) a tool window's title text never shrinks, so a narrow window loses its
  close button — the Werkzeuge window is 112 DIP wide instead of the 76 DIP a 2 × 32 DIP grid needs (title with
  ellipsis + fixed close button would fix it); (3) tooltips are clipped to their tool window.
* Library (img): `zlib_deflate` allocates ~340 KB state per call (VirtualAlloc/mmap + page
  faults + unmap); a variant with caller-owned, reusable state (per worker) would make the
  history deflate ~3× faster and scale with threads.

## 8. Open issues

* No keyboard navigation inside the Verlauf list (library focus model for tool windows).
* Selection history works only once sel.s registers its hooks (until then "Auswahl …" steps
  are no-ops).
* Budget enforcement runs synchronously on the UI thread (parallel inside); a background job
  would hide the ~0.4 s for a 48 MB step.
* Doc/structure entries are counted against the budget but cannot be deflated or spilled
  (only dropped).
* HIST-019/020 for real live tools (gradient, shapes, text, moved pixels) are ready in the API
  (cancel, compound) but untested until those tools exist.
