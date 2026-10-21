# Schwebende Tabellen und unterschiedliche RTF-Zeilenraster

Historischer Etappenstand. Die folgende [Fortsetzungs- und HTML-Etappe](FLOATING_TABLE_PAGES_AND_HTML.md)
ersetzt die hier beschriebenen Einseiten- und HTML-Positionsgrenzen; die RTF-Rasterfunktionen bleiben bestehen.

Stand: 12. Oktober 2026. Fortsetzung ausschließlich in JFLib. Die gemeinsame
Implementierung wird für Windows, Linux und WASM gebaut; jfWord verwendet dieselben
Modelltransaktionen, Codecs, Seitenkoordinaten und Dialogbausteine.

## Bedienung

Eine Zelle auswählen und in den Tabellenwerkzeugen **Position …** öffnen. **Im Textfluss**
behält die normale Blocktabelle bei. **Schwebend** aktiviert den Textumfluss und bietet:

- horizontalen Bezug zur Seite, zum Seitenrand oder zur Textspalte;
- vertikalen Bezug zur Seite, zum Seitenrand oder zum Ankerabsatz;
- genaue Positionen und vier unabhängige Textabstände in Punkt;
- die ausdrückliche Erlaubnis, andere schwebende Tabellen zu überlappen.

Der kleine Griff links oberhalb der Tabelle verschiebt sie direkt auf der Seite.
Das Raster beträgt 4,5 pt; Alt erlaubt freie Bewegung. Escape und Änderungen der
Dokumentrevision brechen einen laufenden Zug ab. Loslassen erzeugt eine einzige
Änderung für Rückgängig/Wiederholen. Die Zellen bleiben direkt beschreibbar; Rahmen,
Vorlagen, verbundene Zellen und enthaltene Untertabellen bleiben erhalten.

Text nutzt pro Zeile den breiteren freien Bereich neben der Tabelle. Unterhalb des
Textabstands steht wieder die gesamte Absatzbreite zur Verfügung. Auf Folgeseiten
werden die Zeilen ohne die vorausgehende Tabelle neu umbrochen. Fußnotenreservierung,
Absatzeinzüge, Tabstopps und Überschriftenbindungen werden weiterhin berücksichtigt.
Inline-Tabellen weichen belegten Flächen nach unten aus. Eine absolute Position, die
bereits gesetzten Text überschreiben würde, beginnt zusammen mit ihrem Anker auf einer
neuen Seite. Bei ausgeschalteter Überlappung weichen weitere schwebende Tabellen nach
unten aus; die gespeicherte Wunschposition bleibt erhalten.

Die Positionsdialoge, Tabellenänderungen, direkte Texteingabe, Rich-Text-Paste,
Zeichen-/Absatzformatierung und fachlichen Agentenänderungen prüfen das Layout mit den
tatsächlichen Dokumentfonts vor dem Übernehmen. Nicht passende Transaktionen
lassen Dokument, Revision, Identitätszähler und Undo/Redo-Historie unverändert.

## RTF-Zeilenraster

Der Import bildet die Vereinigung aller physischen Zellgrenzen. Eine sichtbare Zelle
überspannt die dazugehörigen Rasterabschnitte. Deshalb bleiben beispielsweise zwei
Zellen oben und drei anders aufgeteilte Zellen darunter in ihrer tatsächlichen Breite
erhalten. Schmale interne Rasterabschnitte sind erlaubt; die Mindestbreite gilt für
die sichtbare Zelle einschließlich ihres Innenabstands.

Unterschiedliche `trleft`-Werte und unterschiedlich lange Zeilen werden durch ausgelassene
Rasterbereiche vor beziehungsweise hinter der Zeile dargestellt. Sie sind freie Fläche,
keine zusätzlich eingefügten Zellen. Der gemeinsame linke Ursprung wird auf den
Tabellenursprung normalisiert. Vertikale Verbünde müssen über identische physische
Intervalle laufen. Uneindeutige Verbünde, Text in abgedeckten Fortsetzungen und übergroße
Raster werden vor der Übernahme abgelehnt.

## Gemeinsame Zuständigkeiten

| Bereich | Implementierung |
| --- | --- |
| Position, Grenzen, Undo | `lib/doc_core/table_position.rs`, `tables.rs`, `table_grid.rs`, `Editor::apply_checked` |
| Import und Austausch | `lib/doc_io/rtf_tables.rs`, `import_blocks.rs`, `tables.rs`, `table_position.rs` |
| Zeilenumfluss und Seitenplanung | `lib/doc_layout/floating.rs`, `notes.rs`, gemeinsamer Zeilenumbruch |
| Messung vor Transaktionen | `lib/doc_view/src/measure.rs` mit `apply_paginated` und `replace_fragment_paginated` |
| Positionsdialog und Ziehen | `lib/doc_view/src/table_position_editor.rs`, `table.rs` |
| App-Anbindung | `apps/jfWord/src/ui/table_ui.rs`, `app/table_actions.rs` |

DOCX verwendet `tblpPr`/`tblOverlap`, `gridSpan`, `gridBefore` und `gridAfter`. Private
Tabellenfragmente erhalten dieselben Eigenschaften durch den DOCX-Codec. RTF verwendet
die numerischen Positionierungs- und Textabstandssteuerwörter sowie die tatsächlichen
Zeilenkanten. HTML erhält die Rasterbreiten und Verbünde; leere Randintervalle verwenden
markierte, rahmenlose Abstandszellen, die der eigene Import wieder als Freiraum erkennt.
Die HTML-Ausgabe stellt schwebende Tabellen derzeit im normalen Blockfluss dar.

## Agentensteuerung

`document.table.change` unterstützt die Operation `position`. Werte sind Twips,
die vier Abstände stehen in der Reihenfolge oben, rechts, unten, links:

```json
{
  "table": 2,
  "operation": "position",
  "floating": true,
  "horizontal": "margin",
  "vertical": "paragraph",
  "x": 0,
  "y": 360,
  "distance": [120, 180, 120, 180],
  "overlap": false
}
```

Bei bereits schwebenden Tabellen bleiben weggelassene Felder erhalten. `floating:false`
schaltet auf den normalen Textfluss zurück und nimmt keine Positionsparameter an.
Der vorhandene Revisionsschutz gilt auch hier. `document.table.inspect` zeigt Position,
Zeilenlücken und ausgelassene Rasterplätze. `document.table.layout` liefert neben den
Zellrechtecken bis zu 512 Haupttext-/Notenzeilen je abgefragter Seite mit Quellbereichen
und Geometrie; die Gesamtzahl steht in `totalBodyLines`. Seiten bleiben paginiert.

## Grenzen

Schwebende Tabellen müssen mit ihrem Anker auf eine Seite passen. Mehrseitige schwebende
Tabellen und schwebende Untertabellen sind noch nicht unterstützt. Inline-Untertabellen
innerhalb einer schwebenden Tabelle sind möglich. Native absolute Maße werden erhalten;
symbolische Word-Ausrichtungen wie `tblpXSpec` und negative vertikale Positionen sind
noch nicht im bearbeitbaren Profil. Nicht unterstützte DOCX-Inhalte bleiben über den
bestehenden Paketerhaltungsmodus zugänglich.

Der Browser und die native App benutzen denselben Layoutkern. Das beweist weder einen
identischen Microsoft-Word-Seitenumbruch noch vollständige Word-Dateikompatibilität.
Ein echter Microsoft-Word-Roundtrip bleibt eine eigene Abnahme. Die bisherigen Importlimits
gelten weiter, insbesondere 32 gemeinsame Rasterintervalle, 256 Zeilen, 128 Tabellen und
acht Verschachtelungsebenen.

## Prüfungen und Builds

```powershell
python -X utf8 -B build_rs.py app jfWord --target all --out build/wordcraft-floating-tables -j 4
python -X utf8 -B apps/jfWord/tests/check.py --target both --build-dir build/wordcraft-floating-tables
python -X utf8 -B apps/jfWord/tests/floating_tables.py --target both --out build/wordcraft-floating-tables
node apps/jfWord/tests/floating_tables_browser.mjs build/wordcraft-floating-tables
python -X utf8 -B apps/jfWord/tests/floating_tables.py --target both --out build/wordcraft-floating-tables --web-transfer
```

Die neuen Kernprüfungen stehen in `src/checks/floating_table_checks.rs`; sie gehören zu
`--self-test-tables` und `--self-test`. Die GUI-Prüfung `tests/floating_tables.uis` wird
vom normalen Testsystem gefunden. Browserprüfungen verwenden Chromium/Firefox bei
100/150/200 %, hell/dunkel, echte Canvas-Eingabe, Drag/Escape, den gemeinsamen Dialog,
externe RTF-Paste-Daten, Dateiauswahl, dauerhaftes Speichern und Neuladen.

Abnahme des Builds `build/wordcraft-floating-tables`:

- **1.047 native Prozessprüfungen** unter Windows und Linux bestanden, einschließlich
  aller **48 UI-Skripte pro Plattform**, unabhängiger ZIP-/XML-Prüfungen und Fehlerkorpus.
- **462 Tabellen-Kernprüfungen pro Plattform** bestanden; davon 83 neue Prüfungen für
  schwebende Tabellen, Textumfluss, abweichende Zeilenraster und ausgelassene Intervalle.
- **12 JSON-/MCP-Szenarien** bestanden: jeweils Windows/Linux und JSON/MCP für die neue
  Etappe sowie die Regressionen für Verschachtelung und Zellrahmen.
- **12 Browserfälle** bestanden: sechs neue Abläufe und sechs Verschachtelungsregressionen,
  jeweils Chromium/Firefox bei 100 % hell, 150 % dunkel und 200 % hell. Überprüft wurden
  auch das Ablehnen eines zu großen Rich-Text-Paste-Inhalts ohne Änderung der Undo-Historie
  und der Positionsdialog im schmalen Fenster.
- **24 Browser-zu-Nativ-Übertragungen** bestanden: zwölf gespeicherte Browserdokumente
  jeweils unter Windows und Linux geöffnet und bytegleich wieder ausgegeben.
- Windows-, Linux- und WASM-Ausgaben (scalar, SIMD, Threads) gebaut. Die Browserfälle
  verwenden scalar beziehungsweise automatische Auswahl ohne SharedArrayBuffer.
  Dieser Durchlauf enthält keinen gesonderten Threads-Browserfall.
- API-Dokumentation aktualisiert; Repository-Lint und Prüfung der geänderten Diffs bestanden.

Die gebündelten Ergebnisse stehen in
[`floating-tables-results.json`](../../../build/wordcraft-floating-tables/floating-tables-results.json).
[`floating-tables-build-manifest.json`](../../../build/wordcraft-floating-tables/floating-tables-build-manifest.json)
enthält Größe und SHA-256 von 55 ausgelieferten Dateien. Die lokale Vorschau unter
`http://127.0.0.1:8765/jfWord/` wurde auf diesen Build umgestellt; Loader, Worker und
alle drei WASM-Dateien wurden über HTTP gegen die Build-Dateien geprüft. Der laufende
Benutzertab wurde dabei nicht neu geladen.

Primärreferenzen: [Microsoft/ISO: TablePositionProperties](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.tablepositionproperties?view=openxml-3.0.1),
[Microsofts RTF-Spezifikation 1.6](https://learn.microsoft.com/en-us/previous-versions/office/developer/office2000/aa140277(v=office.10)),
Abschnitt „Table Definitions“ (Positionierungs- und Textabstandssteuerwörter seit Word 2000).
