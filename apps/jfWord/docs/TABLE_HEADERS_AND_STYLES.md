# Wiederholte Tabellenkopfzeilen und Tabellenformatvorlagen

Stand: 12. Oktober 2026. Implementiert in **JFLib**, als Fortsetzung von
[verbundenen Zellen und Tabellenbreiten](MERGED_TABLES.md). Native Anwendung und WASM
verwenden dasselbe Dokumentmodell, denselben Umbruch und denselben Vorlageneditor.
Ausgaben dieser Etappe: `build/wordcraft-table-styles/{win,linux,web/jfWord}`.

## Bedienung

Eine Tabellenzelle anklicken. Die Tabellenwerkzeuge oberhalb der Seite bieten:

- **Kopfzeilen:** Anzahl der ersten Zeilen, die auf Folgeseiten wiederholt werden.
  `0` schaltet Wiederholungen aus. Mehrere Kopfzeilen und darin horizontal oder
  vertikal verbundene Zellen werden gemeinsam wiederholt.
- **Formatvorlage:** Blau, Petrol, Grau, Schlicht oder eine eigene benannte Vorlage.
  **Ohne Vorlage** entfernt die Zuordnung. Tabellenfarben gehören zum Dokument;
  ein Wechsel zwischen heller und dunkler Anwendung verändert sie nicht.
- **Erste Zeile**, **Ergebniszeile**, **Erste Spalte**, **Letzte Spalte** und
  **Zeilenbänder:** Die zugehörigen bedingten Bereiche einzeln einschalten.
  Erste-Zeile-Formatierung gilt auch für weitere als Kopfzeilen markierte Zeilen.
- **Neue Vorlage …:** Die aktuelle Vorlage als Ausgangspunkt verwenden, benennen,
  ändern und der Tabelle zuweisen. Definition und Zuordnung bilden einen Undo-Schritt.
- **Ändern …:** Eine vorhandene Definition bearbeiten. Änderungen wirken auf alle
  Tabellen des Dokuments, die dieselbe Vorlagen-ID verwenden.

Der gemeinsame Editor bietet Name, Grundfläche, Kopfzeilenfläche, Zeilenband,
Kopfzeilenschriftfarbe, Fettdruck sowie Rahmenfarbe und -stärke. Farbwerte sind sechs
RGB-Hexziffern; ein leeres Flächenfeld erbt die Grundformatierung. Weitere bereits
definierte Bereichs- und Schriftattribute bleiben bei der Dialogbearbeitung erhalten.

Direkte Zellfüllungen überlagern die Vorlage. **Aus Tabellenformatvorlage** im
Zelleigenschaftendialog entfernt diese Überschreibung; **Weiß** ist eine ausdrückliche
weiße Fläche. Zeichenformate werden in der Reihenfolge Tabellenbereich, Absatzvorlage,
direkte Zeichenformatierung aufgelöst. Das Auswählen einer Tabellenvorlage übernimmt
deren Rahmen; danach gesetzte direkte Rahmenwerte haben wiederum Vorrang.

Bedingte Bereiche werden nach Zeilenbändern, erster/letzter Spalte und erster/letzter
Zeile aufgelöst. Bei Überschneidungen gewinnt der spätere Bereich; eine einzelne Zeile
mit gleichzeitig aktivierter erster und letzter Zeile erhält deshalb die Formatierung
der letzten Zeile. Direkte Zellformate bleiben darüber wirksam.
[Microsoft: Vorrang bedingter Tabellenformate](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/2ac331d4-cf1e-4fa0-8bca-6da74411e284)

Auch eine wiederholte Kopfzelle auf einer Folgeseite lässt sich direkt anklicken und
bearbeiten. Cursor, Scrollposition, Lineal und Seitenanzeige beziehen sich auf diese
Seiteninstanz. Text, Felder, Zell- und Absatz-IDs existieren weiterhin nur einmal im
Dokument. Die übrigen Wiederholungen werden nach einer Änderung neu dargestellt.

## Gemeinsamer Unterbau

| Bereich | Umsetzung |
| --- | --- |
| `lib/doc_core/table_styles.rs` | `TableStyle`, `TableRegion`, `TableLook`, vier Ausgangsvorlagen, Namens-/ID-Prüfung, Formatvorrang und effektive Zeichenformate |
| `lib/doc_core/tables.rs` | Führender Kopfzeilenbereich, Vorlagenzuordnung, bedingte Bereiche, transaktionale Strukturänderungen |
| `lib/doc_layout/notes.rs` | Gemeinsame Kopfzeilenmessung, Wiederholung bei tatsächlichen Seitenwechseln, Reservierung vor Tabelleninhalt, verbundene Fortsetzungsbereiche |
| `lib/doc_layout::CellBox` | `repeated` unterscheidet eine Darstellungskopie von der ursprünglichen Zelle |
| `lib/doc_view/src/table_style_editor.rs` | Derselbe Vorlagendialog nativ und im Canvas des Browsers |
| `lib/doc_io/table_styles.rs` und `tables.rs` | Standard-DOCX-Vorlagen, bedingte Formatierung und Kopfzeilenmarker |
| `lib/doc_io/clipboard.rs` | Effektive Vorlagenfarben/-fonts, RTF-Kopfzeilen und HTML-Tabellenkopf |
| jfWord `src/ui` und `src/app` | Auswahl und Fokus einer Seiteninstanz, dünne GUI- und Agentenadapter |

Einfügen innerhalb der Kopfzeilen erweitert den Kopfbereich. Einfügen direkt dahinter
erzeugt eine Körperzeile. Entfernen einer Kopfzeile verkleinert den Bereich. Eine
Zellverbindung darf die Grenze zwischen Kopf und Körper nicht schneiden; ungültige
Änderungen werden einschließlich Historie und Revision vollständig verworfen.

Der Kopf bleibt mit dem ersten Körperbereich zusammen. Auch ein verbundener Körperbereich,
der selbst mehrere Seiten benötigt, reserviert auf jeder Fortsetzung erneut den
Kopfbereich. Textzeilen werden nicht am Seitenrand abgeschnitten. Wiederholungen zählen
gegen die bestehenden Layoutbudgets. Ein Kopf, der keinen Platz für eine Körperzeile
auf einer frischen Seite lässt, liefert einen begrenzten Layoutfehler; er führt nicht
zu einer Endlosschleife oder stillschweigendem Textverlust. Kleinere Kopfzeilen, größere
Seiten oder `Kopfzeilen = 0` beheben einen solchen Fall.

## DOCX, RTF und HTML

- `w:tblHeader` steht in den tatsächlichen Kopfzeilen. Der Standard verlangt einen
  zusammenhängenden Bereich am Tabellenanfang; jfWord prüft diesen Bereich ausdrücklich.
  [Microsoft: TableHeader](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.tableheader?view=openxml-3.0.1)
- `w:tblStyle` verweist auf eine Definition mit `w:type="table"` in `word/styles.xml`.
  `w:tblStylePr` beschreibt `firstRow`, `lastRow`, `firstCol`, `lastCol`, `band1Horz`
  und `band2Horz`; `w:tblLook` schaltet diese Bereiche ein oder aus. Direkte Zellfarben
  werden weiterhin als `w:shd` geschrieben. Die Vorlagenzuordnung bleibt bearbeitbar.
  [Microsoft: TableStyleProperties](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.tablestyleproperties?view=openxml-3.0.1)
- Tabellen ohne Vorlagen und Wiederholungen behalten ihre bisherigen DOCX-Ausgaben.
  Private Tabellen-IDs sind für das Erkennen der Standard-Kopfzeilen und Vorlagen nicht
  erforderlich. Nicht unterstützte Vorlageneigenschaften bleiben im vorhandenen
  Paketerhaltungsmodus erhalten.
- RTF-Ausgabe verwendet `\trhdr`; HTML-Ausgabe verwendet `thead`, `th` und `tbody`
  mit den aufgelösten Farben und Fonts. Diese Ausgaben übertragen die Darstellung;
  benannte, weiter editierbare Vorlagen werden über DOCX übertragen. Kopierter Zelltext
  übernimmt die effektive Zeichenformatierung auch außerhalb der ursprünglichen Tabelle.

## Agentenschnittstelle

Alle folgenden verändernden Aktionen verlangen `expectedRevision` und verwenden
dieselben validierten Undo-Transaktionen wie die GUI. Sie sind über JSON, MCP und den
aktivierten Browserkanal erreichbar.

```json
{"action":"document.table.change","input":{"table":2,"operation":"headerRows","headerRows":2},"expectedRevision":10}
{"action":"document.table.change","input":{"table":2,"operation":"style","style":"jfTableBlue"},"expectedRevision":11}
{"action":"document.table.change","input":{"table":2,"operation":"look","look":{"bandedRows":true,"lastRow":true}},"expectedRevision":12}
{"action":"document.table.styles.put","input":{"id":"ResearchTable","name":"Messreihe","cloneFrom":"jfTableTeal","applyTo":2,"firstRow":{"fill":"345A68","format":{"bold":true,"color":16777215}},"band1":{"fill":"D5E9F3"}},"expectedRevision":13}
```

`document.table.styles.list` liefert Definitionen mit `offset`/`limit`.
`cloneFrom` kopiert eine Ausgangsdefinition einmalig; es erzeugt keine
Vererbungsbeziehung. Beim Ändern bleiben ausgelassene Bereiche erhalten. Ein angegebenes
`format` ersetzt die Zeichenattribute dieses Bereichs; `{}` entfernt sie. `fill: ""`
entfernt die Bereichsfüllung. `applyTo` wendet eine Definition in derselben Transaktion an.

`document.table.inspect` ergänzt `headerRows`, `style`, `look`, `borderOverride` und
`effectiveFill`. `document.table.layout` liefert paginierte Seiteninstanzen mit
Zell-IDs, `repeated` und Grenzen in Twips; maximal acht Seiten pro Antwort. Bei dieser
Aktion bezeichnet `offset` die Seitennummer ab null. Diese Inspektion misst mit den
echten Dokumentfonts und benötigt keine Bildschirmkoordinaten als fachliche Eingabe.

## Prüfung und Grenzen

Prüfprogramme:

```powershell
python -X utf8 -B build_rs.py app jfWord --target all --out build/wordcraft-table-styles -j 6
python -X utf8 -B apps/jfWord/tests/table_styles.py --target both --out build/wordcraft-table-styles
node apps/jfWord/tests/table_styles_browser.mjs build/wordcraft-table-styles
python -X utf8 -B apps/jfWord/tests/table_styles.py --target both --out build/wordcraft-table-styles --web-transfer
python -X utf8 -B apps/jfWord/tests/check.py --target both --build-dir build/wordcraft-table-styles
```

Die Browsermatrix deckt Chromium/Firefox bei 100/150/200 %, hell/dunkel, skalare und
automatisch gewählte Berechnung, echte Dialogbedienung und Eingabe in wiederholte
Kopfzellen sowie bestätigtes dauerhaftes Speichern/Neuladen ab. Ergebnisse, DOCX-Dateien,
Bildnachweise und Build-Prüfsummen liegen im oben angegebenen Build-Verzeichnis.

Abnahme dieser Etappe:

| Prüfung | Ergebnis |
| --- | --- |
| Gesamter nativer Regressionssatz | 1039 Prozessprüfungen unter Windows/Linux, einschließlich unabhängigem ZIP/XML-Korpus, ungültigen Eingaben, Fuzz-Fällen und UI-Skripten |
| Gemeinsamer Tabellenkern | 247 Assertions je Ziel; davon 67 neue zu Wiederholungen, Vorlagen, Vorrangregeln, Austausch und ungültigen Änderungen |
| Neue native Agentenabläufe | 4/4: Windows/Linux jeweils über JSON und MCP |
| Bisherige native Tabellenabläufe | 8/8: Tabellenbasis und Zellverbindungen, jeweils Windows/Linux × JSON/MCP |
| Betroffene UI-Abläufe | 24/24 nach der abschließenden Navigations-/Formatauflösung; anschließend Vorlagen-UI und Kern erneut mit korrigiertem Sonderfall für überlappende Bereiche geprüft |
| Neue Browserabläufe | 6/6: Chromium/Firefox × 100/150/200 %, einschließlich Vorlagendialogen, Bearbeitung der wiederholten Kopfzelle und dauerhafter Speicherung |
| Bisherige Browserabläufe | 12/12: Tabellenbasis und Zellverbindungen unter beiden Browsern und drei Skalierungen |
| Browserdateien → native Anwendung | 36/36 bytegleiche DOCX-Rundläufe unter Windows/Linux, davon 12 mit Kopfzeilen und eigenen Vorlagen |
| Auslieferbare Ziele | Windows, Linux sowie skalares, SIMD- und Threads-WASM gebaut |
| Strukturprüfungen | Build-Lint erfolgreich; generierte API-Dokumentation aktuell |

Der vollständige native Regressionssatz wurde vor der letzten Formatprioritätskorrektur
ausgeführt. Danach wurden Kern, Vorlagen-UI, Agenten- und neue Browserabläufe sowie
deren native Dateirundläufe gezielt erneut geprüft. Maschinenlesbare Nachweise:
`build/wordcraft-table-styles/table-styles-results.json` und
`build/wordcraft-table-styles/table-styles-build-manifest.json`.

Tabellenstil-Vererbung, vertikale Bänder und Eckzellsonderformate sind inzwischen in
der [folgenden Etappe](TABLE_STYLE_INHERITANCE.md) umgesetzt. Auch [individuelle Zellrahmen](CELL_BORDERS_AND_TABLE_CLIPBOARD.md)
und [verschachtelte Tabellen](NESTED_TABLES_AND_IMPORT.md) sind inzwischen vorhanden.
Weiter offen sind frei schwebende Tabellen, Tabellenformeln und Notenanker innerhalb von Zellen. Word-eigene
Vorlagen mit weiteren Eigenschaften werden deshalb gegebenenfalls im Erhaltungsmodus
geöffnet. Die Browsermatrix verwendet kein SharedArrayBuffer; das zusätzlich erzeugte
Threads-WASM ist gebaut, aber hier nicht zur Laufzeit abgenommen. Ein Rundlauf mit
einer echten Microsoft-Word-Installation und vollständige Word-Kompatibilität sind
durch diese Tests nicht nachgewiesen.
