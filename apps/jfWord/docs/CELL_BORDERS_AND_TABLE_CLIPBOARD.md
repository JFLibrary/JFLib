# Individuelle Zellrahmen und komplette Tabellen in der Zwischenablage

Diese Etappe wird ausschließlich in JFLib implementiert. Gemeinsamer Rust-Kern
und Bedienoberfläche werden für Windows, Linux und WebAssembly gebaut.
jfguiLib bleibt unveränderte Referenz.

## Bedienung

Eine Zelle auf der Seite auswählen und **Zellrahmen …** öffnen. Für oben, rechts,
unten und links lassen sich Linienart, Breite in Punkt und RGB-Farbe unabhängig
einstellen. Durchgezogen, doppelt, gepunktet und gestrichelt sind verfügbar.
**Aus Tabelle erben** entfernt den direkten Wert, **Tabellenstandard** speichert
Words expliziten Standardwert, **Ausblenden** unterdrückt die gemeinsame Kante.
Alle vier Seiten werden gemeinsam übernommen und mit einem Undo rückgängig gemacht.

**Tabelle kopieren** übernimmt die gesamte Tabelle unabhängig von der Textauswahl
in einer Zelle. **Tabelle ausschneiden** kopiert zuerst erfolgreich und entfernt
sie anschließend als einen Dokumentvorgang. **Nach der Tabelle** setzt den Cursor
in den folgenden Haupttext. Dort oder in einem anderen Dokument fügt Strg+V die
komplette Tabelle ein. Strg+C in einer Textauswahl behält seine Bedeutung für Zelltext.

Mitgeführt werden Spaltenbreiten, Mindesthöhen, Zellabstände, verbundene Zellen,
wiederholte Kopfzeilen, Vorlagenbeziehungen und bedingte Bereiche, direkte
Zell-/Zeichen-/Absatzformate sowie strukturierte Quellenfelder. Gleichwertige
Quellen und Vorlagen werden wiederverwendet; kollidierende IDs und Vorlagen werden
neu zugeordnet, ohne vorhandene Definitionen zu überschreiben. Text vor und nach
der Einfügestelle bleibt erhalten. Die ganze Einfügeoperation ist atomar und
umfasst auch importierte Quellen und Vorlagen.

## Gemeinsame Bausteine

| Bibliothek | Verantwortung |
| --- | --- |
| `doc_core/cell_borders.rs` | Vier physische Seiten, Vererbung, Validierung, Auflösung konkurrierender Rahmen |
| `doc_core/table_grid.rs` | Rahmen beim Verbinden, Teilen und bei Rasteränderungen |
| `doc_core/table_fragment.rs` | Ganze Tabellenfragmente, neue Dokument-IDs, Quellen- und Vorlagenimport |
| `doc_layout/table_borders.rs` | Abschnitte an verbundenen Zellen und Seitenfortsetzungen |
| `doc_view/src/cell_borders.rs` | Gemeinsame native/WASM-Zeichnung aller vier Linienarten |
| `doc_view/src/cell_border_editor.rs` | Dialogentwurf mit vier unabhängig bearbeitbaren Seiten |
| `doc_io/cell_borders.rs` | Standardmäßige `w:tcBorders`-Eigenschaften |
| `doc_io/fragment.rs`, `clipboard.rs` | Strukturierte Zwischenablage und RTF-/HTML-Ausgabe |

Direkte Zellformatierung hat Vorrang vor dem Tabellenrahmen. Zwei direkte Werte
werden anhand der unterstützten Word-Regeln für Stilgewicht, Stilreihenfolge und
Farbhelligkeit aufgelöst. `nil` blendet eine gemeinsame Kante aus; `none` verwendet
den Tabellenstandard. Eine verbundene Zelle kann entlang derselben Seite mehrere
unterschiedlich aufgelöste Rahmenabschnitte haben.
[Microsoft beschreibt diese Konfliktregeln in MS-OI29500](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/7c791c5d-44c3-4fe8-abdd-8f136761bb93).

Beim Verbinden bleibt für jede resultierende Außenkante der erste passende äußere
Zellwert erhalten: oben/links vom Ausgangsfeld, rechts vom rechten Feld der ersten
Zeile, unten vom unteren Feld der ersten Spalte. Innere direkte Zellkanten entfallen.
Beim Teilen wird dieser Außenrahmen auf die neuen Randzellen verteilt; neue innere
Kanten erben die Tabelle. Undo stellt alle vorherigen Einzelwerte wieder her.

## Agentenaktionen

Die vorhandene, ausdrücklich aktivierte Steuerung erhält:

- `document.table.change` mit `operation: "borders"`, `row`, `column` und `borders`.
  Nicht angegebene Seiten bleiben unverändert. `width` ist in Achtelpunkten,
  `color` ein RGB-Integer; sichtbare Linien erlauben 2–96 Achtelpunkte (0,25–12 pt).
- `document.table.inspect` liefert direkte Werte als `borders`; `null` bezeichnet
  eine geerbte Seite.
- `document.table.layout` liefert aufgelöste `edges` mit Seite, relativem Versatz,
  Länge und Rahmenwerten in den sichtbaren Zellinstanzen.
- `clipboard.table.copy` und `clipboard.table.cut`, jeweils mit `table`.
  `clipboard.paste` bleibt der gemeinsame Einfügebefehl.

Beispiel für die Eingabe an `document.table.change`:

```json
{
  "table": 2, "operation": "borders", "row": 0, "column": 0,
  "borders": {
    "top": {"style": "double", "width": 12, "color": 2509688},
    "right": {"style": "nil"},
    "bottom": {"style": "dashed", "width": 8, "color": 10242888},
    "left": {"style": "inherit"}
  }
}
```

Mutierende Aufrufe benötigen die erwartete Dokumentrevision. Fehlerhafte Rahmen,
veraltete Revisionen und Einfügungen unterhalb der erlaubten Mindestbreiten werden
vor dem Commit abgewiesen. GUI und Agenten nutzen denselben Editor.

## Austausch und Grenzen

DOCX und das versionierte native Zwischenablageformat führen alle vier Seiten
mit. RTF und HTML enthalten aufgelöste Rahmen, Zellverbindungen, Kopfzeilen und
sichtbare Feldtexte. Sie transportieren keine jfWord-Vorlagenidentitäten oder
Quellenverknüpfungen; diese bleiben im nativen Format erhalten.

Die [folgende Etappe](NESTED_TABLES_AND_IMPORT.md) erweitert strukturelle Fragmente
auf vollständige Tabellenunterbäume und gemischte Absätze/Tabellen. Einfügen ist
im Haupttext und in Tabellenzellen möglich. Schwebende Tabellen, beliebige gemischte
Blockauswahlen im Seiteneditor, diagonale Rahmen, Theme-Rahmenfarben, unterschiedliche direkte
Werte innerhalb einer einzigen Zellseite und weitere Word-Linienstile bleiben
offen. Fremde, nicht modellierbare DOCX-Eigenschaften bleiben im Paketerhaltungsmodus.
Zu breite eingefügte Tabellen werden inzwischen auf die verfügbare Breite eingepasst.
Unterschreitet diese die Mindestmaße, wird die gesamte Transaktion abgewiesen.

RTF-Tabellenimport und HTML-Import sind inzwischen im [begrenzten Importprofil](NESTED_TABLES_AND_IMPORT.md) implementiert. Die
unterschiedlichen Konfliktregeln fremder RTF-/HTML-Empfänger werden nicht
vereinheitlicht. Vollständige Word-Kompatibilität oder ein Rundlauf mit einer
echten Microsoft-Word-Installation werden nicht behauptet.

## Builds und Prüfungen

```powershell
python -X utf8 -B build_rs.py app jfWord --target all --out build/wordcraft-cell-borders
python -X utf8 -B apps/jfWord/tests/check.py --target both --build-dir build/wordcraft-cell-borders
python -X utf8 -B apps/jfWord/tests/cell_borders.py --target both --out build/wordcraft-cell-borders
node apps/jfWord/tests/cell_borders_browser.mjs build/wordcraft-cell-borders
python -X utf8 -B apps/jfWord/tests/cell_borders.py --target both --out build/wordcraft-cell-borders --web-transfer
```

Kernprüfungen: `src/checks/cell_border_clipboard_checks.rs`. Nativer Dialog- und
Tastaturablauf: `tests/cell_borders.uis`. Die Browsermatrix prüft Chromium und
Firefox bei 100/150/200 %, hell/dunkel, echte Dialog-/Tastatureingaben, bestätigtes
dauerhaftes Speichern und Neuladen. Skalares und SIMD-WASM werden ausgeführt;
Threads-WASM wird zusätzlich gebaut.

Abnahme auf dem abschließenden Build:

| Prüfung | Ergebnis |
| --- | --- |
| Vollständiger nativer Dokument-/UI-Satz | 1043 erfolgreiche Prozessprüfungen unter Windows und Linux, mit unabhängigem ZIP/XML-Korpus, Fehlerfällen, Fuzz-Eingaben und Themes |
| Gemeinsamer Tabellenkern | 327 Assertions je Ziel, darunter 52 neue zu Zellrahmen, Seitenfortsetzungen, Tabellenfragmenten, Quellen- und Vorlagenkonflikten |
| Neue JSON-/MCP-Abläufe | 4/4: beide Transporte unter Windows und Linux |
| Bestehende Agentenabläufe | 16/16 Tabellenfälle und 4/4 allgemeine Dokument-/Quellen-/Notenfälle |
| Neue Browsermatrix | 6/6: vierseitiger Rahmendialog, ungültige Eingabe, vollständige Kopie über echte Tasten, Ausschneiden/Undo, Einfügen zwischen Dokumenten, wiederholte Köpfe, dauerhaftes Speichern/Neuladen |
| Bestehende Browsermatrix | 12/12: wiederholte Kopfzeilen sowie Vererbung, gruppierte Bänder und Eckbereiche |
| Browserdateien → native Builds | 12/12 bytegleiche DOCX-Rundläufe |
| Auslieferung | Windows, Linux und skalares/SIMD/Threads-WASM gebaut |
| Quellenstruktur/API | Build-Lint, generierte API-Dokumentation und Diff-Prüfung erfolgreich |

Maschinenlesbare Ergebnisse: `build/wordcraft-cell-borders/cell-borders-results.json`.
Build-Prüfsummen: `build/wordcraft-cell-borders/cell-borders-build-manifest.json`.
Screenshots und übertragene Browserdateien liegen unter `cell-borders-browser` im
gleichen Build-Verzeichnis. Das native Prüfdokument ist `cell-border-tests/win-json.docx`.

Die Vorschau unter `http://127.0.0.1:8765/jfWord/` liefert diese Build-Dateien;
alle drei WASM-Varianten, HTML, Loader und Worker wurden byteweise gegen die Dateien
geprüft. Bereits geöffnete Dokumente wurden nicht automatisch neu geladen.
