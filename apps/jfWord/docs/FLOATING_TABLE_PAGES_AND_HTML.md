# Mehrseitige schwebende Tabellen und HTML-Positionen

Stand: 12. Oktober 2026. Implementierung ausschließlich in JFLib, mit demselben
Dokumentkern, Layout, Eingabeadapter und Codec unter Windows, Linux und WASM.
Build-Verzeichnis: `build/wordcraft-floating-pages`.

## Mehrseitige Tabellen

Schwebende Tabellen werden zwischen Zeilen aufgeteilt und auf Folgeseiten fortgesetzt.
Passt eine vollständige Zeilengruppe in den verfügbaren Bereich, bleibt sie zusammen.
Überhohe Zeilen und vertikal verbundene Zellen dürfen über mehrere Seiten laufen;
Textzeilen werden dabei nicht abgeschnitten. Die eingestellte Mindesthöhe darf bis
31.680 Twips reichen. Ein Seitenwechsel erzeugt keine zusätzliche Tabellenzeile.

- Die Wunschposition gilt für den Anfang. Fortsetzungen behalten den horizontalen
  Bezug und beginnen am oberen Textrand der nächsten Seite. Die gespeicherten
  Positionswerte werden durch den Seitenumbruch nicht verändert.
- Führende Kopfzeilen erscheinen auf Folgeseiten erneut. Enthaltene Untertabellen
  wiederholen ihre eigenen Kopfzeilen innerhalb der umgebenden Zellrahmen.
- Zell- und Absatzidentitäten bleiben stabil. Eingabe, Auswahl, Tab/Shift+Tab und
  Rückgängig funktionieren auch in einer Fortsetzung; wiederholte Kopfzeilen
  bearbeiten dieselben ursprünglichen Zellen.
- Vertikale Zellrahmen werden an Fortsetzungen gekürzt, horizontale Rahmen nur an
  tatsächlichen Zellgrenzen gezeichnet. Der Größenänderungsgriff für eine Zeile
  liegt weiterhin an ihrem wirklichen Ende; auch überhohe Zeilen lassen sich dort
  innerhalb der gemeinsamen Modellgrenzen vergrößern und verkleinern.
- Der Fließtext umbricht neben jedem Seitenfragment. Nach dem letzten Fragment
  steht die gesamte Absatzbreite wieder zur Verfügung. Auch die Trefferflächen
  des Fließtexts halten die Tabellenfläche frei.
- Fußnoten verkleinern vor dem Übernehmen ihrer Ankerzeile den verfügbaren
  Tabellenbereich. Ihre Fortsetzungen und Tabellenfortsetzungen werden gemeinsam
  abgearbeitet. Tabelleninhalt überschreibt keine Fußnoten.
- Lange Tabellen erzeugen ihre Folgeseiten auch bei einem kurzen oder leeren
  Ankerabsatz. Breite Tabellen dürfen ganze Seiten belegen, bevor der Absatz folgt.
  Weitere Tabellen ohne Überlappung werden bei Bedarf auf folgende Seiten verschoben.

Es bleibt eine Tabelle im Dokumentmodell und in DOCX; Seitenfragmente und wiederholte
Kopfzeilen sind ausschließlich Darstellungsobjekte. Gemessene Ausgangsgeometrie wird
zwischen den Fragmenten geteilt. Die vorhandenen Grenzen für Seiten, Zeilen,
Textfragmente und Verschachtelung gelten weiter.

## HTML-Austausch

Die gemeinsame HTML-Ausgabe enthält normale Tabellen, Spaltenbreiten, Zeilenhöhen,
Verbünde, Kopfzeilen, Zellrahmen und formatierte Absätze. Schwebende Tabellen bekommen
zusätzlich eine CSS-Float-Darstellung und diese versionierten Attribute:

```html
<div data-jf-page="1 8640 5760 720 720 720 720">
  <table data-jf-table-position="1 margin paragraph 180 270 60 120 90 150 0">
    ...
  </table>
</div>
```

`data-jf-page` enthält Version, Seitenbreite, Seitenhöhe und die Ränder **oben, rechts,
unten, links**. `data-jf-table-position` enthält Version, horizontalen Bezug
(`page`, `margin`, `column`), vertikalen Bezug (`page`, `margin`, `paragraph`), X, Y,
die vier Textabstände in derselben Reihenfolge sowie Überlappung (`0`/`1`). Alle Zahlen
sind ganze Twips. Negative X-Werte relativ zum Rand bleiben erhalten, sofern die
Tabelle innerhalb der physischen Seite liegt.

Beim Öffnen einer HTML-Datei werden deren Seiteneinstellungen übernommen. Beim
Einfügen in ein bestehendes Dokument bleibt dessen Seiteneinstellung maßgeblich.
Die Positionsattribute haben Vorrang vor der zusätzlichen CSS-Darstellung; unbekannte
Versionen, fehlende Werte und ungültige Geometrie werden ausdrücklich abgelehnt.
Ein fehlgeschlagener Import verändert das aktuelle Dokument nicht.

Fremdes HTML ohne diese Attribute unterstützt für Tabellen:

- `float:left` und `float:right`, einschließlich numerischer Margin-Werte;
- das ältere `align=left/right` als Float-Angabe;
- `position:absolute` mit numerischem `left` oder `right` und `top`, bezogen auf
  die Dokumentseite; Prozentbreiten werden nach dem jeweiligen Seitenbezug aufgelöst;
- Längen in px, pt, in, cm und mm, einschließlich signierter absoluter X-Offsets.

Rechte Positionen werden erst nach Ermittlung der Tabellenbreite berechnet. Tabellen-
Margins werden nicht als Absatzeinzüge an die Zelltexte vererbt. Das ältere `TABLE align`
verschiebt die Tabelle, ohne die Absatzausrichtung der Zellen zu ändern; explizites
`text-align` bleibt wirksam. Siehe [HTML 4, Tabellenattribute](https://www.w3.org/TR/html401/struct/tables.html#h-11.2.1).
Externe Stylesheets,
beliebige CSS-Containing-Blocks, `fixed`/`sticky`, `calc()` und CSS-Transformationen
gehören nicht zum unterstützten HTML-Profil. Der Decoder führt kein HTML/JavaScript
aus und lädt keine externen Ressourcen.

CSS-Floats beschreiben einen kontinuierlichen Dokumentfluss. Sie ersetzen keinen
Word-Seitenanker. Deshalb erhält nur ein Importer, der die Austauschattribute
berücksichtigt, alle Anker- und Überlappungswerte exakt. Entfernt ein fremdes Programm
die Attribute, bleibt die CSS-Näherung; identischer Word-Seitenumbruch ist damit nicht
zugesichert. Grundlagen: [W3C, Float-Layout](https://www.w3.org/TR/CSS22/visuren.html#floats)
und [HTML-Standard, eigene Datenattribute](https://html.spec.whatwg.org/multipage/dom.html#embedding-custom-non-visible-data-with-the-data-*-attributes).

## Bibliotheks- und App-Grenzen

| Aufgabe | Gemeinsame Implementierung |
| --- | --- |
| Position, Zeilenhöhen und Dokumentvalidierung | `lib/doc_core/table_position.rs`, `tables.rs` |
| Unveränderliche Tabellenmessung und Fortsetzungswarteschlange | `lib/doc_layout/floating.rs` |
| Seitenfragmente, Kopfzeilen und Rahmen | `lib/doc_layout/floating_fragment.rs` |
| Haupttext und Fußnotenreservierung | `lib/doc_layout/notes.rs` |
| Trefferflächen und Verschieben | `lib/doc_view/src/table.rs`, `text_line_bounds` |
| HTML-Position und Seiteneinstellung | `lib/doc_io/html_position.rs`, `html_import.rs`, `import_blocks.rs` |
| HTML-/RTF-/DOCX-Ausgabe | bestehende Codecs in `lib/doc_io` |

jfWord bindet diese Funktionen an Dateien, die vorhandene Rich-Text-Oberfläche und
Agentenaktionen. Der Browser verwendet weiterhin Worker, Software-Renderer und Canvas.
Es gibt keinen zweiten Tabellenkern in JavaScript.

## Agenten und Verwendung

Der bestehende Positionsdialog, das Verschieben am Griff und `document.table.change`
funktionieren weiterhin. Zusätzlich nimmt `document.export` ein optionales Format an:

```json
{"action":"document.export","input":{"format":"html"}}
```

Erlaubt sind `docx` (Standard), `html` und `rtf`. Die Antwort verweist auf eine separat
lesbare Ressource mit passendem MIME-Typ. Export ändert weder Revision noch
Speicherstatus. HTML/RTF-Ausgabe aus einer nur im Erhaltungsmodus geöffneten DOCX wird
abgelehnt; die unveränderte DOCX-Ausgabe bleibt verfügbar.

Die Beispieldatei `apps/jfWord/fixtures/floating-pages.html` erzeugt vier Tabellenseiten
und lässt sich über **Datei öffnen** nativ oder im Browser laden.

## Prüfungen

```powershell
python -X utf8 -B build_rs.py app jfWord --target all --out build/wordcraft-floating-pages -j 4
python -X utf8 -B apps/jfWord/tests/check.py --target both --build-dir build/wordcraft-floating-pages
python -X utf8 -B apps/jfWord/tests/floating_pages.py --target both --out build/wordcraft-floating-pages
node apps/jfWord/tests/floating_pages_browser.mjs build/wordcraft-floating-pages
python -X utf8 -B apps/jfWord/tests/floating_pages.py --target both --out build/wordcraft-floating-pages --web-transfer
```

Kernfälle stehen in `src/checks/floating_pages_checks.rs` und den erweiterten
`floating_table_checks.rs`. Die native GUI-Prüfung `tests/floating_pages.uis` gehört
zum normalen Testlauf. Browserfälle prüfen Chromium/Firefox bei 100 % hell, 150 %
dunkel und 200 % hell: reale Canvas-Eingabe auf einer Folgeseite und in einer wiederholten
Kopfzeile, externes HTML-Paste, Positionsfehler, Dateiöffnung und dauerhaftes Speichern.

### Abnahme des ausgelieferten Builds

- Windows und Linux: jeweils **10.265 Kernprüfungen**, keine Fehler.
- Vollständige native Suite unter beiden Betriebssystemen bestanden, einschließlich
  **49 GUI-Skripten je Plattform**, unabhängiger XML-/ZIP-Prüfung und Mutationskorpus.
  Der separat protokollierte Linux-Lauf umfasst 525 Prozessprüfungen.
- **16 JSON-/MCP-Szenarien** für mehrseitige und bisherige schwebende Tabellen,
  verschachtelte Tabellen sowie Zellrahmen/Zwischenablage bestanden.
- **18 Browser-Szenarien** in Chromium und Firefox bestanden: die drei Tabellenetappen
  jeweils bei 100 % hell, 150 % dunkel und 200 % hell. Skalarer und SIMD-Pfad wurden benutzt.
- **36 Transfers Browser → Windows/Linux** bestanden. Die gespeicherten DOCX-Dateien
  wurden nativ geöffnet und bytegenau wieder ausgegeben.
- Native direkte Zellauswahl, Tab-Navigation zur Fortsetzung, Texteingabe,
  Rückgängig/Wiederholen und Positionsdialog zusätzlich unter Windows und Linux geprüft.
- Lint ohne Fehler; API-Dokumentation aktuell. Windows, Linux und alle drei WASM-Varianten
  gebaut. Die Threads-Variante wurde gebaut; die Browserfälle verwenden skalar beziehungsweise SIMD.

Ein erster kombinierter Schlusslauf bestand den vollständigen Windows-Abschnitt,
traf unter Linux aber auf eine bereits durch den Quellen-GUI-Test überschriebene
gemeinsame Merge-Eingabedatei. Der vollständige Linux-Lauf wurde deshalb mit frisch
erzeugten Testdateien separat wiederholt und bestand ohne Codeänderung. Der ursprüngliche
Fehllauf und ein Änderungsprotokoll der Fixture bleiben als Diagnose erhalten.

Ergebnisse, Quellenprüfsummen und Anwendungsmanifest liegen unter
`build/wordcraft-floating-pages/{verification.json,feature-sources.json,manifest.json}`.
Das Manifest erfasst 55 Anwendungsdateien einschließlich der Browser-Lizenzhinweise;
Compiler-Caches gehören nicht zur Auslieferung. Die Vorschau unter
`http://127.0.0.1:8765/jfWord/` liefert diese Dateien; HTML, Loader, Worker und alle
WASM-Varianten wurden über HTTP gegen die lokalen Prüfsummen verglichen.

Vollständige Word-Kompatibilität bleibt ein eigenes Ausbau- und Nachweisziel. Schwebende
Untertabellen, beliebige HTML/CSS-Seitenmodelle und Word-identische Paginierung sind
damit nicht abgeschlossen. Für HTML bleiben Quellen- und Notenmetadaten weiterhin
dem nativen DOCX-/Zwischenablageformat vorbehalten.
