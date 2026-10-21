# Seitenumbrüche innerhalb von Tabellenzeilen

Stand: 13. Oktober 2026. Fortsetzung von TAB-012 nach den
[mehrseitigen schwebenden Tabellen](FLOATING_TABLE_PAGES_AND_HTML.md).
Die gemeinsame JFLib liegt in diesem Arbeitsstand unter `GitHub/JFLib`.

## Bedienung

Eine Tabellenzelle auswählen und **Eigenschaften …** öffnen. Die Option
**Zeilenwechsel auf Seiten zulassen** gilt für die ausgewählte Tabellenzeile:

- Eingeschaltet: Zeilen dürfen den verbleibenden Seitenraum nutzen und auf der
  nächsten Seite weiterlaufen. Geteilt wird zwischen Textzeilen.
- Ausgeschaltet: Eine passende Zeile rückt als Ganzes auf die nächste Seite.
  Eine Zeile, die selbst auf einer frischen Seite zu hoch ist, läuft dort weiter.
  Text wird weder abgeschnitten noch verworfen.
- Wiederholte Kopfzeilen werden vor dem Fortsetzungstext berücksichtigt.
  Verschachtelte Tabellen bringen ihre eigenen Zeilenregeln mit; die äußere
  Tabelle erbt die Einstellung nicht als globale Eigenschaft.
- Fortsetzungsfragmente behalten Zell- und Absatz-IDs. Texteingabe und bestehende
  Tabellenbefehle bleiben verfügbar. Nur die tatsächliche letzte Zeilenkante
  bietet den Griff für die Zeilenhöhe; Seitenwechsel erzeugen keine neuen Rahmen.
- Ändern der Option ist eine rückgängig machbare Dokumentoperation. Die gemeinsame
  Eigenschaftsmaske übernimmt Maße, Ausrichtung, Füllung und Zeilenregel gemeinsam.

Bei schwebenden Tabellen bleibt der vorhandene Vorrang vollständiger Zeilengruppen
erhalten. „Zulassen“ erzwingt keinen Schnitt, wenn eine vollständige Gruppe sinnvoll
platziert werden kann. Muss eine zusammengehaltene erste Zeile über die Ankerseite
hinaus verschoben werden, beginnt ihr erstes sichtbares Fragment am oberen Textrand
der Folgeseite; der Ankerabsatz bleibt erhalten.

Bei versetzt angeordneten Untertabellen können sich mehrere Regeln zum Zusammenhalten
so überlappen, dass es keinen gemeinsamen Umbruchpunkt innerhalb einer Seite gibt.
Erst auf einer frischen Seite wird diese nicht erfüllbare Gruppe gelockert; Textzeilen
bleiben ungeteilt. Der eigene Test `interleaved-row-breaks.html` deckt diesen Fall ab.

Die Ausnahme für überhohe Zeilen entspricht der dokumentierten Bedeutung von
[WordprocessingML `cantSplit`](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.cantsplit).
Ein identischer Umbruch für beliebige Word-Dokumente ist damit nicht nachgewiesen.

## Gemeinsame Implementierung

| Aufgabe | Bibliothek |
| --- | --- |
| Zeileneigenschaft `TableRow.cant_split`, `TableChange::RowBreak` und Undo | `lib/doc_core/tables.rs` |
| Verschachtelte Sperrbereiche und fortsetzbare Inline-Geometrie | `lib/doc_layout/row_breaks.rs`, `notes.rs` |
| Schwebende Tabellen und Fortsetzungen | `lib/doc_layout/floating.rs`, `floating_fragment.rs` |
| Gemeinsame Eigenschaftsmaske | `lib/doc_view/src/table_editor.rs` |
| DOCX, RTF, HTML und strukturierte Zwischenablage | `lib/doc_io` und vorhandener Fragmentkern |

jfWord registriert die Aktion und bindet die Eigenschaftsmaske ein. Windows, Linux
und WASM verwenden denselben Modell-, Umbruch- und Codec-Code. Es gibt keine
JavaScript-Implementierung der Tabellenlogik.

## Austausch und Agenten

DOCX schreibt und liest `w:trPr/w:cantSplit`, einschließlich expliziter wahr/falsch-Werte.
Ohne Eigenschaft bleiben Umbrüche erlaubt. RTF verwendet `\trkeep` beziehungsweise
`\trkeep0`; `\trowd` setzt die Zeileneigenschaft zurück. HTML verwendet lokale
`break-inside:avoid`- und `page-break-inside:avoid`-Angaben an `tr`. Der Import
unterstützt auch `avoid-page`; die moderne Eigenschaft hat Vorrang vor der alten.
Sie wird nicht an innere Tabellen oder Absatztexte vererbt.
Siehe [CSS Fragmentation, Umbrüche innerhalb von Boxen](https://www.w3.org/TR/css-break-3/#break-within).

Ungültige Word-/RTF-Schalter und nicht unterstützte HTML-Umbruchausdrücke werden
zurückgewiesen. Externe Stylesheets und beliebige CSS-Vererbungsregeln bleiben außerhalb
des unterstützten statischen HTML-Profils. Ganze Tabellen behalten ihre Zeilenoptionen
in der nativen Rich-Text-Zwischenablage und beim Einfügen in ein anderes Dokument.

```json
{
  "action": "document.table.change",
  "expectedRevision": 12,
  "input": {"table": 7, "operation": "rowBreak", "row": 0, "allowSplit": false}
}
```

`document.table.inspect` liefert `allowSplit` je Zeile.
`document.table.layout` ergänzt `rowEnd` je Seiteninstanz einer Zelle, damit Agenten
Fortsetzungen von der tatsächlichen letzten Zeilenkante unterscheiden können.
Die vorhandenen Schema-, Modalitäts- und Revisionsprüfungen gelten weiter.

## Reproduzierbare Prüfung

```powershell
python -X utf8 -B build_rs.py app jfWord --target all --out build/wordcraft-row-breaks -j 4
python -X utf8 -B apps/jfWord/tests/row_breaks.py --target both --out build/wordcraft-row-breaks
node apps/jfWord/tests/row_breaks_browser.mjs build/wordcraft-row-breaks
python -X utf8 -B apps/jfWord/tests/row_breaks.py --target both --out build/wordcraft-row-breaks --web-transfer
python -X utf8 -B apps/jfWord/tests/check.py --target both --build-dir build/wordcraft-row-breaks
```

`src/checks/row_break_checks.rs` prüft die Modell- und Umbruchregeln,
`tests/row_breaks.uis` den gemeinsamen Dialog samt Undo/Redo und DPI-Wechsel.
`fixtures/row-breaks.html` zeigt eine teilbare Zeile nach einem längeren Seitenanfang.
Die Browserprüfung bedient den Dialog und die verschobene Zeile tatsächlich auf dem
Canvas, prüft HTML-Wiederöffnung und bestätigtes dauerhaftes Speichern mit Neuladen.

Die konkreten Ergebnisse des Builds werden unter `build/wordcraft-row-breaks`
protokolliert. Vollständige Word-Kompatibilität, exakte Zeilenhöhen und Notenanker
innerhalb von Tabellenzellen bleiben gesonderte Ausbauarbeiten.


## Abnahme des endgültigen Builds

- Windows und Linux: jeweils **10.346 Kernprüfungen**, keine Fehler.
- Vollständige native Suite auf den endgültigen Dateien: **1.051 Prozessprüfungen**,
  einschließlich **50 GUI-Skripten je Plattform**, Formatmutationen und unabhängigen
  XML-/ZIP-Prüfungen.
- **20 JSON-/MCP-Szenarien** bestanden: Zeilenumbrüche, mehrseitige schwebende Tabellen,
  Tabellenpositionen/RTF-Raster, verschachtelte Tabellen sowie Zellrahmen/Zwischenablage.
- **24 Browser-Szenarien** bestanden: Chromium und Firefox bei 100 % hell, 150 % dunkel
  und 200 % hell, für diese und die drei vorherigen Tabellenetappen.
- **48 Browser-zu-Windows/Linux-Transfers** bestanden; alle DOCX-Dateien bytegleich
  nativ wieder ausgegeben.
- Die nativen Dialogaufnahmen sind für Windows/Linux jeweils bytegleich:
  `native-*-dialog-light.png` und `native-*-dialog-dark-150.png`.
- Lint bestanden; API-Dokumentation aktuell. Native Builds und drei WASM-Varianten
  erstellt. Die Threads-Variante ist gebaut; Browserprüfungen verwenden skalar/SIMD.
- Lokale Vorschau über HTTP gegen die auslieferbaren Dateien geprüft: HTML, Loader,
  Worker und alle drei WASM-Dateien stimmen bytegenau überein.

`build/wordcraft-row-breaks/verification.json` verknüpft die Nachweise.
`manifest.json` erfasst die 55 Anwendungsdateien mit SHA-256-Prüfsummen;
`feature-sources.json` und `changes.patch` dokumentieren die 24 Featuredateien.
