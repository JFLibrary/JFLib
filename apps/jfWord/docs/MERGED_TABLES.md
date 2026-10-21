# Verbundene Zellen und Tabellenbreiten

Stand: 12. Oktober 2026. Fortsetzung der [Tabellenbearbeitung auf der Seite](TABLES.md),
entwickelt und gebaut ausschließlich in **JFLib**. jfguiLib bleibt eine unveränderte
Referenz. Der aktuelle Build liegt unter `build/wordcraft-merged/{win,linux,web}`.

## Bedienung

- Eine Zelle anklicken und **Verbinden …** wählen. Zeilen- und Spaltenzahl beschreiben
  den rechteckigen Bereich ab dieser Zelle nach unten und rechts. Bereits verbundene
  Zellen müssen vollständig enthalten sein; ein angeschnittener Bereich wird abgelehnt.
- Alle Absätze, Zeichenformate, Vorlagen, Quellenfelder und IDs des Bereichs bleiben
  erhalten. Die Absätze werden zeilenweise in der linken oberen Zelle zusammengeführt.
  Hintergrund und vertikale Ausrichtung stammen aus dieser ersten Zelle.
- **Teilen** löst die ausgewählte Verbindung in ihre ursprünglichen Rasterplätze auf.
  Der gesamte Inhalt bleibt in der linken oberen Zelle; die weiteren Zellen bekommen
  leere, bearbeitbare Absätze. Undo stellt Inhalt und Raster exakt wieder her.
- Tab und Umschalt+Tab wechseln ausschließlich zwischen sichtbaren Zellen. Ein Klick
  im überdeckten Teil einer verbundenen Zelle bearbeitet dieselbe Zellstory. Enter,
  Rich-Text-Auswahl, Zwischenablage, Formatierung und individuelle Tabstopps bleiben verfügbar.
- Zeilen oder Spalten innerhalb einer Verbindung einzufügen vergrößert ihren Bereich.
  Das Entfernen eines Rasterstreifens verkleinert ihn. Solange ein Teil der verbundenen
  Zelle verbleibt, bleiben ihre ID und Inhalte auch beim Löschen des Ursprungsstreifens erhalten.
- **Breite anpassen …** bietet **An Inhalt**, **An Seitenbreite** und **Gleich verteilen**.
  Die Inhaltsmessung berücksichtigt die tatsächlichen Dokumentfonts, Schriftgrößen,
  Einzüge, individuellen Tabstopps und verbundene Zellen. Überbreite Inhalte werden
  innerhalb der verfügbaren Seite umgebrochen. Gleichmäßige Verteilung behält die
  Gesamtbreite einschließlich ganzzahliger Rest-Twips bei.
- Diese Anpassungen setzen feste Spaltenbreiten. Eine wirkliche Änderung ist jeweils
  ein Undo-Schritt; eine bereits passende Breite erzeugt keinen zusätzlichen Eintrag.
  Es handelt sich um eine einmalige Anpassung, nicht um einen dauerhaft eingeschalteten
  Word-AutoFit-Modus.
- Der Eigenschaftendialog zeigt die gesamte Breite der verbundenen Zelle. Eine neue
  Breite wird proportional auf ihre Rasterspalten verteilt. Ziehen an der rechten Kante
  verändert die letzte enthaltene Spalte; die untere Kante verändert die letzte enthaltene
  Zeile. Eine durch Seitenumbruch entstandene Fortsetzungskante ist kein Zeilengriff.
- In schmalen Fenstern verteilen sich die Tabellenwerkzeuge auf drei Zeilen.

Verbindungen können mehrere Seiten überdecken. Vertikal zusammenhängende Zeilen bleiben
nach Möglichkeit gemeinsam auf einer Seite; überlange Bereiche werden fortgesetzt,
ohne Textzeilen zu verlieren oder zu duplizieren. Inhalte und Hintergründe bleiben an
die jeweilige sichtbare Zellfläche gebunden.

## Gemeinsame Bibliotheken

| Modul | Verantwortung |
| --- | --- |
| `lib/doc_core/table_grid.rs` | Rastervalidierung, Auflösung überdeckter Plätze, Verbinden, Teilen und Strukturänderungen |
| `lib/doc_core/tables.rs` | Atomare Dokumentoperationen, Zellstories, IDs und Formatgrenzen |
| `lib/doc_layout/notes.rs` | Gemeinsame Seitengeometrie, Zeilenhöhen, verbundene Bereiche und Seitenfortsetzungen |
| `lib/doc_layout/table_fit.rs` | Plattformunabhängige Inhalts-/Seiten-/Gleichverteilung mit `Measure` |
| `lib/doc_view/src/{measure,table,table_editor}.rs` | Dokumentfontauflösung, Zelltreffer, Ziehen und genauer Eigenschafteneditor |
| `lib/doc_io/{tables,clipboard}.rs` | DOCX sowie RTF-/HTML-Ausgabe |
| `apps/jfWord/src/{ui/table_ui,app/table_actions}.rs` | Dünne Bedienungs- und Agentenadapter |

`TableCell.row_span` und `column_span` sind an sichtbaren Ursprungszellen positiv.
Überdeckte Rasterplätze haben beide Werte null und keinen eigenen Text. Das physische
Raster behält eindeutige Zell-IDs; `Table::origin`, `cell`, `cell_width` und `anchors`
stellen die fachliche Sicht darauf bereit. Validierung weist Überschneidungen,
Rasterlöcher, ungültige Spannen und Inhalte in überdeckten Plätzen zurück.

Das Modell und alle Berechnungen bleiben `no_std`, ohne Cargo und Fremd-Crates.
Native Programme und Browser verwenden denselben Rust-Code und dieselben Aktionen.

## DOCX, RTF und HTML

DOCX schreibt horizontale Verbindungen mit `w:gridSpan` und vertikale Verbindungen mit
`w:vMerge`. Die erste Zeile enthält `restart`, weitere Zeilen eine Fortsetzung mit
gleicher Rasterspanne. Der Import akzeptiert sowohl `w:vMerge` ohne Wert als auch
explizites `continue`. Zellbreiten entsprechen der Summe der abgedeckten Rasterspalten.
Die Inhalte stehen nur im Ursprungsabschnitt; die Fortsetzung enthält einen leeren Absatz.
Das separate Custom-XML mit jfWord-IDs ist für das Erkennen der Verbindung nicht erforderlich.

Primärquellen: [Microsoft: GridSpan](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.gridspan?view=openxml-3.0.1)
und [Microsoft: VerticalMerge](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.verticalmerge?view=openxml-3.0.1).

HTML schreibt eine Ursprungszelle mit `rowspan`/`colspan`, RTF die horizontalen und
vertikalen Merge-Steuerwörter. Zellinhalte werden dabei nicht vervielfältigt. Der
Rich-Text-Clipboardkern innerhalb einer Zelle behält seine vorhandenen Verträge.

Das editierbare DOCX-Profil setzt weiterhin ein regelmäßiges Raster mit festen Breiten
voraus. Zusätzlicher Inhalt oder abweichende Formatierung einer vertikalen Fortsetzung,
komplexe Tabellenstile und nicht unterstützte Eigenschaften bleiben im Paketerhaltungsmodus.
Sie werden nicht stillschweigend entfernt. Der aktuelle Fortsetzungsparser unterstützt
leere, attributfreie Absätze; zusätzliche Absatzmetadaten oder Formatierung dort erfordern
weiteren Ausbau. Ein tatsächlicher Microsoft-Word-Rundlauf ist noch gesondert nachzuweisen.

## Agentenaktionen

Die bestehende Aktion `document.table.change` erhält `merge` und `split`.
`document.table.inspect` ergänzt `rowSpan`, `columnSpan` und `covered` pro Rasterplatz.
Die neue Aktion `document.table.fit` verwendet dieselbe Schriftmessung wie die GUI.
Zeilen und Spalten sind nullbasiert, Maße Twips. Verändernde Aufrufe verlangen die
aktuelle `expectedRevision`; Schemafehler, veraltete Revisionen und ungültige Bereiche
ändern weder Dokument noch Undo-Historie.

```json
{"action":"document.table.change","input":{"table":2,"operation":"merge","row":0,"column":0,"rows":2,"columns":2},"expectedRevision":10}
{"action":"document.table.change","input":{"table":2,"operation":"split","row":1,"column":1},"expectedRevision":11}
{"action":"document.table.fit","input":{"table":2,"mode":"contents"},"expectedRevision":12}
```

Weitere Fit-Modi sind `page` und `equal`. Ein `split` darf auch einen überdeckten
Rasterplatz angeben; der gemeinsame Kern löst diesen zur Ursprungszelle auf. Der
Dispatcher ist über aktivierte native JSON-/MCP-Sitzungen und den Browserzugang erreichbar.

## Prüfungen und Builds

Die Testprogramme befinden sich unter `src/checks/table_merge_checks.rs`,
`tests/table_merges.py`, `tests/table_merges.uis` und `tests/table_merges_browser.mjs`.
Sie ergänzen die bestehenden rechteckigen Tabellen-, Quellen-, Feld-, Noten-,
Rich-Text- und Formatprüfungen. Ausgaben und Protokolle stehen unter
`build/wordcraft-merged`; UI-Skripte schreiben ihre Aufnahmen zusätzlich unter `build/`.

```powershell
python -X utf8 -B build_rs.py app jfWord --target all --out build/wordcraft-merged -j 6
python -X utf8 -B apps/jfWord/tests/tables.py --target both --out build/wordcraft-merged
python -X utf8 -B apps/jfWord/tests/table_merges.py --target both --out build/wordcraft-merged
node apps/jfWord/tests/tables_browser.mjs build/wordcraft-merged
node apps/jfWord/tests/table_merges_browser.mjs build/wordcraft-merged
python -X utf8 -B apps/jfWord/tests/tables.py --target both --out build/wordcraft-merged --web-transfer
python -X utf8 -B apps/jfWord/tests/table_merges.py --target both --out build/wordcraft-merged --web-transfer
python -X utf8 -B apps/jfWord/tests/check.py --target both --build-dir build/wordcraft-merged
python -m http.server 8765 --bind 127.0.0.1 --directory build/wordcraft-merged/web
```

Die Browsermatrix verwendet Chromium/Firefox, 100/150/200 %, hell/dunkel, den skalaren
Pfad beziehungsweise automatische Auswahl ohne SharedArrayBuffer. Das Threads-WASM
wird gebaut; dessen paralleler Ausführungspfad ist nicht Gegenstand dieser Tabellenmatrix.

Abnahme des aktuellen JFLib-Builds:

| Prüfung | Ergebnis |
| --- | --- |
| Auslieferbare Builds | Windows, Linux, skalares WASM, SIMD-WASM und Threads-WASM gebaut; SHA-256-Werte in `merged-tables-build-manifest.json` |
| Gesamte native jfWord-Regression | **1.037 Prozessprüfungen bestanden**, Windows/Linux einschließlich UI-Skripten mit `--verify-dirty`, ohne `UIWARN` |
| Gemeinsamer Tabellenkern | **180/180 Prüfungen je nativem Ziel**, einschließlich 137 zusätzlicher Prüfungen für Verbindungen, Strukturänderungen, Umbruch und Breitenberechnung |
| Native Agentenabläufe | **4/4 neue** Kombinationen aus Windows/Linux und JSON/MCP, zusätzlich **4/4 bestehende** Tabellenabläufe bestanden |
| Wissenschaftliche Felder | Quellenzitat nach Teilen, erneutem Verbinden, Speichern und Neuladen in allen vier nativen Agentenkombinationen erhalten |
| Browserbedienung | **6/6 neue** Fälle für Zellverbindungen, zusätzlich **6/6 bestehende** Tabellenfälle, Chromium/Firefox und alle drei Skalierungen |
| GUI-/Agentenparität | Alle drei Breitenaktionen liefern innerhalb jeder Browserinstanz über GUI und API identische Spaltenbreiten |
| Dauerhaftes Browserspeichern | Alle zwölf Browserfälle erst nach bestätigtem Abschluss gespeichert und nach Neuladen bytegleich exportiert |
| Browser → Desktop | **24/24 Rundläufe**: zwölf Browserdateien jeweils mit Windows und Linux bytegleich verarbeitet |
| Browservergleich | Alle sechs neuen Dateien identisch, jeweils 8.305 Bytes; alle sechs bisherigen Tabellendateien identisch, jeweils 9.148 Bytes |
| Code und Dokumentation | Bibliotheks-/App-Lint bestanden, generierte API-Dokumentation aktuell |
| Sichtprüfung | Helle verbundene Tabelle, dunkle Ansicht mit 150 %, Dialog und kompakte native Werkzeugleiste geprüft |

`native-checks.log`, `browser-merged.log`, `browser-tables.log` und die jeweiligen
`results.json`-Dateien liegen im Build-Verzeichnis. Die Vorschau unter
`http://127.0.0.1:8765/jfWord/` liefert dieses JFLib-Bundle; Loader und WASM wurden
gegen die gebauten Dateien bytegleich geprüft.

## Verbleibende Tabellenarbeiten

Verschachtelte Tabellen, wiederholte Kopfzeilen, Tabellenformeln, dauerhafter AutoFit,
komplexe Tabellenstile und frei schwebende Tabellen bleiben offen. Ebenfalls noch offen
sind Notenanker innerhalb von Zellen, das Kopieren ganzer Tabellen als strukturierte
Fragmente und der Import fremder RTF-/HTML-Tabellen. Die Verbindungsauswahl erfolgt
über den Bereichsdialog; eine Zellbereichsauswahl per Maus ist noch nicht implementiert.
Vollständige Word-Dateikompatibilität ist durch diese Etappe nicht nachgewiesen.

Fortsetzung: [Wiederholte Tabellenkopfzeilen und Tabellenformatvorlagen](TABLE_HEADERS_AND_STYLES.md)
erweitert diesen Stand im gemeinsamen Kern und in beiden Plattformoberflächen.
