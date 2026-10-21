# Drucken (PR1, Plan M8)

Datei ▸ Drucken… (Strg+P, `CMD_PRINT` 106), aktiv nur mit offenem Bild.  paint.net 5.1 druckt
das zusammengefügte Bild über den Windows-Fotodruck-Assistenten; PaintCraft zeigt stattdessen
einen eigenen modalen Dialog und ruft dann `plat_print` der Bibliothek auf
(`legacy/ASMLIB/docs/modules/PRINT.md`: Windows-Druckdialog + GDI, Linux-Portal + PDF).

## Dialog (`src/print.s`, Fenster `WINID_DLG0 + 48` = 0x6040 = 24640)

| Id | Element | Vorgabe |
|---|---|---|
| 6210 | Vorschau: Seite (weiß), druckbarer Bereich (Linie), Bild (Thumbnail über Weiß, Umriss) | |
| 6201 | Papierformat: A4 / Letter / A5 / Legal / A3 (Vorschlag für den Systemdialog) | A4 |
| 6202 / 6203 | Ausrichtung: Hochformat / Querformat | Hochformat |
| 6204 / 6205 / 6206 | Druckgröße: An Seite anpassen / Originalgröße (Pixeldichte) / Prozentual: | anpassen |
| 6207 | Prozent (1..1000, nur bei „Prozentual:“) | 100 |
| 6208 | Zentrieren (sonst links oben im druckbaren Bereich) | an |
| 6209 | Rand: n mm (0..50; der unbedruckbare Rand des Druckers gilt, wenn größer) | 5 |

Darunter „Bild: W × H Pixel, n dpi“ und „Druckgröße: b × h cm“ (aus `print_layout`, dieselbe
Rechnung wie beim Drucken; im Systemdialog kann das Papier noch wechseln).  „Drucken…“ schließt
den Dialog und druckt; Abbrechen / Esc / Schließen verwerfen nichts (die Optionen bleiben für
die Sitzung).  Pixeldichte = Auflösung des Dokuments (`DOC_DPU_*`, Zentimeter umgerechnet,
ohne Einheit 96 dpi).  Fehler (kein Drucker/Druckdienst, Speicher, Druckfehler) → Fehlerleiste;
Abbruch im Systemdialog → nichts.

Texte: „Drucken“, „Druckgröße“, „Prozentual:“ aus paint.net (`CommonAction.Print`,
`ResizeDialog.PrintSizeHeader`, `ResizeDialog.PercentRB`); die übrigen eigene.

## Einbindung

`fileio.s` (markierte PR1-Stellen): `fio_states` → `prt_states`, `fio_frame` → `prt_frame`,
`fio_cmd` leitet `CMD_PRINT` und `CMD_PRT_DONE` (140, `pn_ids.inc`, Arg 1 = drucken) an
`prt_cmd` weiter.

## Tests

- `tests/print.uis`: Menüzustand, Dialog, alle Optionen mit der Druckgröße, Esc/Abbrechen,
  Optionen bleiben, headless „Drucken…“ ohne Testvariable → Fehlerleiste (nie ein Dialog),
  ohne Bild deaktiviert.  `skeleton.uis`: „Drucken…“ jetzt aktiv.
- `python tools/print_check.py --target both`: echte Ausgabe über die Testvarianten
  (`jfgui_PRINT_TO`, Windows „Microsoft Print to PDF“), Portal mit Fake-Bus.
