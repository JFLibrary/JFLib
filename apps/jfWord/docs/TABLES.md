# Tabellen direkt auf der Seite

Stand: 12. Oktober 2026. Fortsetzung von [Linealen und Tabstopps](RULERS_AND_TABS.md).
Historischer Buildname: `build/wordcraft-tables/{win,linux,web}`.
Der anschließende Ausbau [Verbundene Zellen und Tabellenbreiten](MERGED_TABLES.md)
liegt ausschließlich in JFLib unter `build/wordcraft-merged`.

## Bedienung

- **Einfügen → Tabelle** öffnet die Auswahl von Zeilen und Spalten. Die Tabelle wird an
  der Schreibposition im Haupttext eingefügt. Text davor und danach bleibt erhalten.
- Eine Zelle anklicken und direkt schreiben. Zeichenformate, Schriftfamilien,
  Absatzformate, vererbte Vorlagen, Zitate und die Rich-Text-Zwischenablage verwenden
  den vorhandenen Schreibkern. Enter erzeugt einen Absatz innerhalb derselben Zelle.
- Tab wechselt zur nächsten, Umschalt+Tab zur vorherigen Zelle. Tab in der letzten
  Zelle ergänzt eine Zeile. Strg+Tab fügt ein echtes Tabulatorzeichen ein.
- Die Werkzeugleiste über der Seite ergänzt und entfernt Zeilen oder Spalten,
  löscht eine Tabelle und öffnet die Eigenschaften der ausgewählten Zelle.
- Senkrechte innere Kanten verschieben die Breite der beiden benachbarten Spalten.
  Die rechte Außenkante ändert die Gesamtbreite. Untere Zeilenkanten setzen eine
  Mindesthöhe; längerer Inhalt vergrößert die Zeile weiterhin automatisch.
  Das Raster beträgt 90 Twips. Alt hebt es auf, Escape verwirft den Zug. Ein
  abgeschlossener Zug ist ein Undo-Schritt.
- **Eigenschaften …** erlaubt genaue Spaltenbreite, Mindesthöhe, Zellhintergrund
  und vertikale Ausrichtung. Maße werden in Punkt angezeigt; 0,05 Punkt ist ein Twip.
- Das horizontale Lineal zeigt Einzüge und Tabstopps relativ zum Textbereich der
  aktiven Zelle. Ihre Grenzen sind dort gesperrt; Spalten werden an der Tabelle geändert.
- Undo/Redo erhält Zellidentitäten und Inhalte auch nach Strukturänderungen.

## Gemeinsame Umsetzung

`doc_core::Table`, `TableRow` und `TableCell` modellieren rechteckige Tabellen als
Blöcke vor stabilen Haupttextabsätzen. Jede Zelle besitzt gewöhnliche Rich-Text-Absätze
und eine eigene Textstory. Haupttextstories enden an Tabellen. Eine Textauswahl kann
somit keine Nachbarzelle oder dazwischenliegende Tabelle versehentlich zusammenziehen.
`Edit::InsertTable` und `Edit::ChangeTable` validieren und übernehmen jede Änderung
atomar in der bestehenden Editor-Historie. IDs sind dokumentweit eindeutig, auch
gegenüber Quellen, Noten und Absatzkennungen.

`doc_layout` berechnet Zellrechtecke und Text in Twips mit denselben Schriftmetriken
wie den Haupttext. Passende Zeilen bleiben zusammen; überlange Zeilen werden über
Seiten fortgesetzt, ohne eine Textzeile zu zerschneiden. Text wird an der Zellgrenze
beschnitten. Fußnoten im Haupttext und Quellenfelder in Zellen bleiben integriert.

`doc_view` übernimmt Zeichnung, Zelltreffer, Ziehsteuerung und den Eigenschafteneditor.
jfWord verbindet diese Bausteine mit seiner Auswahl und den registrierten Aktionen.
Native Programme und WASM verwenden dieselben Rust-Module; es gibt keinen HTML-
Tabelleneditor als zweite Implementierung.

Grenzen des Modells: maximal 128 Tabellen, 256 Zeilen und 32 Spalten pro Tabelle;
zusätzlich gelten weiterhin die dokumentweiten Absatz-/Speicherbudgets. Breite und
Mindesthöhe müssen in den verfügbaren Seitenbereich passen. Die letzte Zeile oder
Spalte wird über **Tabelle löschen** entfernt. Farben gehören zum Dokument und ändern
sich beim Wechsel zwischen hellem und dunklem App-Thema nicht.

## Formate und Agenten

DOCX verwendet die standardisierten Elemente `w:tbl`, `w:tblGrid`, `w:tr`, `w:tc`,
Zellbreiten, Mindesthöhen, feste Tabellenbreiten, gleichmäßige Ränder, Schattierung und
vertikale Ausrichtung. Ein separater, mit einer Paketbeziehung verknüpfter Custom-XML-
Teil erhält die jfWord-Tabellen-/Zellidentitäten. Standardtabellen ohne diesen Teil
bekommen beim Import neue IDs. Absatzkennungen und die bestehenden Quellen-/Notenteile
bleiben unabhängig davon erhalten.

Primärreferenz: [Microsoft: WordprocessingML-Tabellen](https://learn.microsoft.com/en-us/office/open-xml/word/working-with-wordprocessingml-tables).

HTML- und RTF-Ausgabe enthalten echte Tabellen. Rich-Text-Kopieren innerhalb einer
Zelle erhält Absätze, Formate und Felder. Im Browser gilt das für die instanzlokale
Zwischenablage; der Austausch mit der Systemzwischenablage bleibt dort Klartext und
unterliegt Browserberechtigungen. Ganze Tabellen als Clipboard-Fragmente und
der Import fremder RTF-/HTML-Tabellen sind noch nicht umgesetzt.

| Aktion | Zweck |
| --- | --- |
| `document.table.insert` | Tabelle an `paragraph`/UTF-8-`offset`, mit `rows` und `columns` einfügen |
| `document.table.change` | Zeilen/Spalten, Maße, Zellfüllung/-ausrichtung, Rahmen oder Tabelle ändern |
| `document.table.inspect` | Stabile Zell-/Absatz-IDs und Zeileneigenschaften mit Offset/Limit abfragen |
| `document.inspect` | Tabellensummen und paginierte Tabellendaten zusätzlich zu allen Zellabsätzen |

Änderungen verlangen die erwartete Dokumentrevision. Ungültige Maße, veraltete
Revisionen und Modalität folgen den vorhandenen strukturierten Fehlern. Die gleichen
Aktionen sind im aktivierten CLI-/JSON-/MCP- und Browserzugang registriert.

Beispiel für `document.table.change`:

```json
{"table": 2, "operation": "columns", "widths": [3200, 2000, 2000]}
```

Maße sind Twips, Zeilen und Spalten nullbasiert. `borderWidth` wird entsprechend
DOCX in Achtelpunkten angegeben. `fill` ist ein sechsstelliger RGB-Hexwert, eine leere
Zeichenfolge entfernt die Füllung. Ausrichtungen sind `top`, `center`, `bottom`.

## Prüfungen und verbleibender Ausbau

Tests: `src/checks/table_checks.rs`, `tests/tables.py`, `tests/tables.uis` und
`tests/tables_browser.mjs`. Die Browsermatrix verwendet Chromium/Firefox, 100/150/200 %,
hell/dunkel, echte Maus-/Tastaturereignisse, native DOCX-Dateien und bestätigtes
Speichern mit Neuladen. Dateien und Aufnahmen liegen unter `build/wordcraft-tables`.

Abnahme dieser Etappe:

| Prüfung | Ergebnis |
| --- | --- |
| Builds | Windows, Linux und WASM erstellt; SHA-256-Werte in `tables-build-manifest.json` |
| Native Gesamtprüfung | 1.035 Prozessprüfungen unter Windows/Linux bestanden, zusätzlich interne Bibliotheksprüfungen und unabhängige XML-/ZIP-Prüfungen; UI-Skripte ohne `UIWARN` |
| Gemeinsamer Tabellenkern | 43 neue Prüfungen je nativem Ziel, einschließlich Zellstories, Umbruch über Seiten, Quellen/Noten, IDs und Zelllinealen |
| Native Agentenabläufe | 4/4 Kombinationen aus Windows/Linux und JSON/MCP bestanden; native DOCX-Rundläufe bytegleich |
| Browserbedienung | 6/6 Fälle in Chromium/Firefox bei 100/150/200 %, hell/dunkel; Einfügedialog, Zelltext, Tab-Navigation, Rich-Text-Zwischenablage, Zelllineal, Kantenziehen, Undo und Eigenschaftendialog |
| Dauerhaftes Browserspeichern | 6/6 Fälle nach bestätigtem Abschluss und Neuladen bytegleich |
| Browser → Desktop | 12/12 Rundläufe; alle sechs Browserdateien sind identisch (9.148 Bytes) und werden von Windows/Linux bytegleich ausgegeben |
| Sichtprüfung | Helle Seitenansicht sowie dunkle Seiten-/Dialogansicht geprüft; Tabellenfarben bleiben unabhängig vom App-Thema |

Die Browsermatrix prüft den skalaren Pfad sowie automatische Auswahl ohne
SharedArrayBuffer. Das zusätzlich erzeugte Threads-WASM ist gebaut, aber nicht
Gegenstand dieser Tabellen-Browsermatrix. Ein realer Microsoft-Word-Rundlauf ist
ebenfalls kein Bestandteil dieser Abnahme.

```powershell
python -X utf8 build_rs.py app jfWord --target all --out build/wordcraft-tables -j 6
python -X utf8 apps/jfWord/tests/tables.py --target both --out build/wordcraft-tables
node apps/jfWord/tests/tables_browser.mjs build/wordcraft-tables
python -X utf8 apps/jfWord/tests/tables.py --target both --out build/wordcraft-tables --web-transfer
python -X utf8 apps/jfWord/tests/check.py --target both --build-dir build/wordcraft-tables
```

Verbundene/verschachtelte Zellen, wiederholte Kopfzeilen, Tabellenformeln, AutoFit,
komplexe Tabellenstile, frei schwebende Tabellen und Notenanker innerhalb von Zellen
bleiben Ausbauarbeiten. DOCX-Inhalte außerhalb des editierbaren Profils bleiben im
vorhandenen Paketerhaltungsmodus; sie werden nicht stillschweigend in einfache Tabellen
umgewandelt. Vollständige Word-Kompatibilität ist durch diese Etappe nicht nachgewiesen.

Die folgenden Etappen setzen [verbundene Zellen](MERGED_TABLES.md) sowie
[wiederholte Kopfzeilen und benannte Tabellenformatvorlagen](TABLE_HEADERS_AND_STYLES.md) um.
Die vorstehenden offenen Punkte beschreiben den damaligen Stand der ersten Tabellenetappe.
