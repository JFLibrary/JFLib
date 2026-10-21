# Exakte Tabellenzeilenhöhen

Stand: 13. Oktober 2026. Ausbau von **TAB-007**, aufbauend auf den
[Zeilenumbruchregeln](TABLE_ROW_BREAKS.md). Die gemeinsame JFLib liegt in diesem
Arbeitsstand unter `GitHub/JFLib`; jfguiLib bleibt unveränderte Referenz.

## Bedienung

Eine Tabellenzelle auswählen, **Eigenschaften …** öffnen und bei **Höhenregel** wählen:

- **Mindestens:** Die angegebene Höhe ist die Untergrenze. Inhalt kann die Zeile
  vergrößern. `0` lässt die Höhe automatisch aus dem Inhalt berechnen.
- **Genau:** Die positive Höhe bleibt fest. Überstehende Texte und innere Tabellen
  bleiben vollständig im Dokument, sind auf der Seite aber abgeschnitten. Vergrößern
  der Höhe oder Zurückstellen auf „Mindestens“ macht sie wieder sichtbar.

Der Dialog übernimmt Maße, Regel, Zellfüllung, Ausrichtung und Zeilenumbruch gemeinsam
als einen Undo-Schritt. Null oder ungültige Maße bei „Genau“ halten den Dialog offen
und ändern das Dokument nicht. Der Dialoginhalt scrollt bei kleinen Fenstern oder hoher DPI.
Der vorhandene Griff an der unteren Zeilenkante erhält beim Ziehen die Höhenregel.
Auch eine Zeile, deren Text ganz verborgen ist, bleibt für ihre Eigenschaften auswählbar.

## Gemeinsame Implementierung

| Bereich | Ort und Verhalten |
| --- | --- |
| Modell | `doc_core::RowHeightRule::{AtLeast, Exact}`, `TableRow.height_rule`; `min_height` bleibt aus API-Kompatibilitätsgründen der Zahlenwert in Twips |
| Bearbeitung | `TableChange::RowSizing` setzt Höhe und Regel atomar; `RowHeight` behält die bestehende Regel, ebenso direkte Größenänderungen |
| Seitenumbruch | `lib/doc_layout/notes.rs` und `row_height.rs`; feste Zeilen wachsen nicht durch Text oder Untertabellen |
| Anzeige/Eingabe | Gemeinsame Zellgeometrie begrenzt Zeichnung, Auswahl und Treffertests; verborgene Untertabellen erhalten keine falschen Rahmen oder Größenänderungsgriffe |
| Dialog | `lib/doc_view/src/table_editor.rs`; jfWord bindet den gemeinsamen Editor ein |
| Austausch | Gemeinsame DOCX-, RTF- und HTML-Codecs einschließlich strukturierter Ganz-Tabellen-Zwischenablage |

Bei vertikal verbundenen Zellen wird zusätzlicher Platz der letzten Zeile mit
Mindesthöhe im verbundenen Bereich zugeteilt. Sind alle betroffenen Zeilen fest,
bleibt deren Summe unverändert. Inhalt, der nicht hineinpasst, wird nur in der
Darstellung beschnitten. Zu großer Inhalt beginnt auch bei mittiger/unterer
Ausrichtung oben, damit der Textanfang sichtbar bleibt.

Seitenteilung und Zusammenhalten verwenden weiterhin die separat gespeicherte
Zeilenumbruchregel. Eine überhohe feste Zeile darf sich über Seiten fortsetzen;
die Summe ihrer ursprünglichen Zellfragmente entspricht weiterhin der eingestellten
Höhe. Wiederholte Kopfzeilen sind zusätzliche Anzeigeinstanzen derselben Zellen.
Windows, Linux und WASM benutzen denselben Dokument-, Codec- und Layout-Code.

## Dateiformate

- **DOCX:** `w:trHeight` schreibt `w:hRule="exact"` oder `"atLeast"`. Importiert werden
  auch `auto` und eine fehlende Höhenregel; diese verwenden automatische Höhe.
  Ein bei `auto` wirkungsloser Zahlenwert wird im Modell zu `0` normalisiert.
  Unbekannte Regeln, ungültige Werte und eine genaue Höhe von `0` werden abgelehnt.
- **RTF:** Ein negatives `\trrhN` bezeichnet eine genaue Höhe, ein positiver Wert
  eine Mindesthöhe, `0` automatische Höhe. `\trowd` setzt die Regel zurück. Beträge
  außerhalb des gemeinsamen Maßbereichs werden ohne vorzeichenbedingten Überlauf abgelehnt.
- **HTML:** Gewöhnliches `height` an `tr` bleibt eine Mindesthöhe.
  `mso-height-rule:exactly` wird lokal an der Zeile erkannt. Der Export erhält genaue
  Twips zusätzlich als `data-jf-row-height="1 exact N"`; diese versionierten Metadaten
  haben beim Wiederimport Vorrang vor möglicherweise gerundeten CSS-Maßen.
  Bei vollständig festen Zellen liefert ein Block mit `max-height` und `overflow:hidden`
  eine visuelle CSS-Annäherung. Er enthält weiterhin den gesamten Inhalt.

Die Word-Eigenschaft ist in der [Microsoft-Dokumentation zu TableRowHeight](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.tablerowheight?view=openxml-3.0.1)
beschrieben. Die unterschiedliche Bedeutung gewöhnlicher CSS-Zeilenhöhen folgt aus
[CSS 2.2, Tabellenhöhen](https://www.w3.org/TR/CSS22/tables.html#height-layout).
Beliebiges HTML außerhalb dieses statischen Importprofils und eine identische
Darstellung in jedem fremden HTML-Renderer werden damit nicht zugesagt.

## Agentenschnittstelle

```json
{
  "action": "document.table.change",
  "expectedRevision": 12,
  "input": {
    "table": 7,
    "operation": "rowHeight",
    "row": 0,
    "height": 480,
    "heightRule": "exact"
  }
}
```

480 Twips sind 24 Punkt. `heightRule` ist optional; ohne Angabe bleibt die bestehende
Regel erhalten. `document.table.inspect` liefert `height` und `heightRule` je Zeile.
Die vorhandenen Schemata, Revisionskonflikte, Modalitätsprüfungen und Undo-Verträge gelten weiter.

## Reproduzierbare Abnahme

```powershell
python -X utf8 -B build_rs.py app jfWord --target all --out build/wordcraft-exact-rows -j 4
python -X utf8 -B apps/jfWord/tests/exact_rows.py --target both --out build/wordcraft-exact-rows
node apps/jfWord/tests/exact_rows_browser.mjs build/wordcraft-exact-rows
python -X utf8 -B apps/jfWord/tests/exact_rows.py --target both --out build/wordcraft-exact-rows --web-transfer
python -X utf8 -B apps/jfWord/tests/isolated_check.py --target both --build-dir build/wordcraft-exact-rows
python -X utf8 -B lib/ui/tests/hit_regions.py --target both --out build/wordcraft-exact-rows
```

`row_height_checks.rs` enthält Modell-, Geometrie-, Codec- und echte Rasterprüfungen.
`exact_rows.uis` bedient den gemeinsamen Dialog, Undo/Redo und DPI-/Fensterwechsel.
Die Browserprüfung bedient denselben Dialog, tippt in die sichtbare Zeile,
stellt eine vollständig beschnittene Zeile wieder her und prüft Export sowie
bestätigtes dauerhaftes Speichern mit Neuladen. `fixtures/exact-row-heights.html`
zeigt den Unterschied zwischen genauer Höhe und Mindesthöhe.

Der isolierte Runner führt die unveränderte native Gesamtsuite in einem frischen
Arbeitsverzeichnis unter dem angegebenen Buildordner aus. So können gleichzeitige
Buildprüfungen die von den Tests geschriebenen DOCX-Dateien nicht überschreiben.
Skripte, Dokumente und Aufnahmen bleiben dort für die Fehlersuche erhalten.

Die Abnahme deckte außerdem zwei UI-Probleme auf: Die Trefferflächen mehrerer Zeilen
eines Rich-Text-Felds zählen nun als ein Widget, während tatsächlich mehrfach vergebene
Widget-IDs weiterhin gewarnt werden. Einzelne Anmerkungsabsätze erhalten in der Navigation
eigene IDs, unabhängig vom gemeinsamen Eingabefeld ihrer Textgeschichte.

Abgenommen am 13. Oktober 2026 mit dem Build unter `build/wordcraft-exact-rows`:

- Windows und Linux: jeweils **10.444 Kernprüfungen, 0 Fehler**; davon 98 zusätzliche
  Prüfungen zu Zeilenhöhen, Geometrie, Austausch und Rasterung.
- **1.053 native Prozessprüfungen** einschließlich 51 Oberflächenskripten je Plattform,
  unabhängiger XML-/ZIP-Prüfung, beschädigter Dateien und Themen-/DPI-Wechsel.
- **24 JSON-/MCP-Szenarien** für Zeilenhöhen und die bisherigen Tabellenfunktionen.
- **24 Browser-Szenarien** in Chromium und Firefox bei 100, 150 und 200 Prozent;
  die neue Höhenprüfung bestätigt das tatsächlich geladene skalare beziehungsweise SIMD-Backend.
- **48 DOCX-Übertragungen** von den Browserprüfungen nach Windows/Linux mit bytegleichem
  erneutem Export. Dazu ein unabhängiger UI-Regressionstest für logische Treffergruppen
  und echte doppelte Widget-IDs auf beiden nativen Plattformen.
- Bibliotheks-Lint ohne Beanstandungen; API-Dokumentation mit 5.086 Einträgen aktuell.

Alle drei WASM-Varianten sind gebaut. Die Threads-Variante gehört zum Buildnachweis;
die Browserabläufe dieser Etappe testen das skalare und das SIMD-Backend.

Die endgültigen Prüfergebnisse und Builds liegen unter `build/wordcraft-exact-rows`.
Notenanker in Tabellenzellen, Tabellenformeln und vollständige Word-Kompatibilität
bleiben gesonderte Ausbauarbeiten.
