# Farbprofil / Farbmanagement (PaintCraft, agent I1, plan M8, spec 2.12)

Module `src/colormgmt.s`; library: `legacy/ASMLIB/docs/modules/COLOR_MANAGEMENT.md`.

## Bild ▸ Farbprofil… (CMD_COLOR_PROFILE 278)

Modal dialog window "Farbprofil" (WINID_DLG_PROFILE 0x6038, widgets ID_DLG_PROFILE 6300..):

* **Aktuell:** description of the document profile (DOC_ICC), "sRGB (kein Profil
  eingebettet)" without one.
* **Neu:** list: sRGB, A98 RGB, Display P3, ROMM RGB, Graustufen (Gamma 2,2)
  (built-in, generated profiles), "Monitor: <desc>" when the system has a monitor profile,
  "Importiert: <desc>" after **Import…** (file dialog `*.icc;*.icm` "Farbprofil", id block
  ID_FD_ICC 0x24000; an unusable file -> "Die Datei enthält kein brauchbares Farbprofil.").
  **Export…** writes the document profile (sRGB built-in when none) to a file.
* **Priorität:** Perzeptiv / Relativ farbmetrisch / Sättigung / Absolut farbmetrisch;
  **Tiefenkompensierung verwenden** (default: Perzeptiv + Tiefenkompensierung).
* Buttons **Konvertieren** (accent) / **Zuweisen** / **Abbrechen** (paint.net 5.1:
  "Kon&versieren", "Zu&weisen").

Actions (one history step each, IC_COLOR, names from paint.net's resources):
* **Zuweisen** -> "Farbprofil zuweisen": DOC_ICC = a copy of the chosen profile, pixels
  unchanged (the step is carried by `hist_push_sel`; the selection stays as it is).
* **Konvertieren** -> "In Farbprofil umrechnen": every layer converted from the current
  profile (none = sRGB) with `icc_xform_row` on the worker pool (64-row bands x layers), new
  surfaces installed like imgops.s does (`hist_push_doc`, old surfaces to the history with
  `hist_release`), then DOC_ICC = the new profile.  A profile that cannot be used ->
  the dialog comes back with "In dieses Farbprofil lässt sich nicht umrechnen."
* Undo / redo of DOC_ICC: the history has no DOC_ICC slot, so colormgmt.s records every
  step {document, token in DOC_CSPACE, HE_SEQ of the entry, old profile, new profile} and
  `cm_sync` (every frame) sets DOC_ICC to the state of the history position; records whose
  entries were dropped / truncated / cleared are freed, and so are records of closed
  documents (token mismatch).

## Display (canvas)

The canvas rows (view.s, after `k_view_row` / `k_view_row_pm`, block "I1") go through
`cm_view_row`: document profile -> monitor profile of the main window (`plat_monitor_icc`,
queried again when `plat_monitor_key` changes = the window moved to another monitor; none /
Wayland / headless = sRGB), perceptual intent.  The composite is never changed.  Transforms
are cached per (document profile, monitor profile) by CRC-32 (4 entries); identical or
equivalent profiles cost nothing (no transform).  Colour management is **on** by default
(paint.net 5.1 always colour-manages the canvas).  Not colour-managed: thumbnails (tabs,
layers), the colour picker / palette, rulers and overlays.

## Interfaces

* imgops.s (A5) forwards (block "I1"): `cm_states`, `cm_frame` (in iop_frame), commands 278,
  280..283 and 290..299 -> `cm_cmd` (before its doc check).
* Settings (S1, page "Farbmanagement"): `cm_enabled() -> eax 1 on`, `cm_set_enabled(on)`
  (redraws the canvas with the next frame).
* fileio.s (K1) already does what is needed: open copies IM_ICC into DOC_ICC, save embeds
  DOC_ICC (IOF_EMBED_ICC).  Suggested later: formats without ICC support could offer
  a conversion to sRGB on save, as paint.net does for BMP / GIF / TGA.

## Test hooks (cmd ID ARG) and probes

| cmd | effect |
|---|---|
| 280 / 281 row | Zuweisen / Konvertieren with list row (0..4 built-in, then monitor, then imported) |
| 282 / 283 | Import… / Export… file dialog |
| 290 id | monitor profile = built-in id 0..4; 255 = none (sRGB); 65535 = real query |
| 291 on | colour management on / off |
| 292 i + (b << 8) | dialog intent i, Tiefenkompensierung b |
| 293 x + (y << 16) | display probe: composite pixel (x, y) as shown -> #533249 "AARRGGBB" |
| 294 id | built-in id as the "imported" profile |

Probes: #533248 (0x82300) text document profile description ("" = none), value DOC_ICC_LEN ·
#533249 display probe, value = colour management on · #533250 monitor profile description,
value display transform kind (-1 none, 1 matrix, 2 3D LUT) · #533251 dialog error text,
value = profile history records.

Script: `tests/colorprofile.uis` (assign, convert sRGB -> A98 RGB / Graustufen with pixel
checks against the icc_check reference, undo / redo, simulated P3 monitor, on / off, the
dialog through the combo box, imported row, error path); Windows and Linux, also with
`--verify-dirty`.  A one-off screenshot check confirmed the canvas shows sRGB red as
(234, 51, 35) on a simulated Display P3 monitor and (255, 0, 0) with management off.
