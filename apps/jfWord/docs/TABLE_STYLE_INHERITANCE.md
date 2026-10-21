# Vererbbare Tabellenformatvorlagen und bedingte Bereiche

Stand: 12. Oktober 2026. Fortsetzung der
[wiederholten Tabellenkopfzeilen und Tabellenformatvorlagen](TABLE_HEADERS_AND_STYLES.md)
in **JFLib**. Ausgabe: `build/wordcraft-table-graphs/{win,linux,web/jfWord}`.
Modell, Formatauflösung, Dateicodecs und Vorlagendialog sind gemeinsame Bibliotheksfunktionen.
GUI, JSON, MCP und Browser verwenden dieselben Dokumenttransaktionen.

## Bedienung

Eine Tabellenzelle auswählen und in den Tabellenwerkzeugen **Neue Vorlage …** oder
**Ändern …** öffnen. Neue Vorlagen erben nun von der aktuellen Tabellenvorlage;
Änderungen an dieser Basis wirken auch auf die abgeleiteten Vorlagen und deren Tabellen.
Eigene Eigenschaften überschreiben nur die jeweils gesetzten Werte.

Der Dialog hat drei Register:

- **Grundlagen:** Name, häufige Flächen- und Schriftfarben sowie Rahmen und Schriftgewicht.
- **Vererbung:** Basisvorlage, geerbter oder eigener Rahmen, Zeilen beziehungsweise Spalten
  pro Band und Zurücksetzen sämtlicher Formatüberschreibungen. `0` bei einer Bandgröße
  bedeutet Vererbung; ohne geerbten Wert gilt eine Zeile beziehungsweise Spalte pro Band.
- **Bereiche:** Gesamtfläche, erste/letzte Zeile, erste/letzte Spalte, beide Zeilenbänder,
  beide Spaltenbänder und alle vier Eckzellen. Fläche, Schriftfarbe und Schriftgewicht
  lassen sich je Bereich ändern oder gemeinsam zurücksetzen. Weitere vorhandene
  Zeichenattribute bleiben bei gewöhnlicher Bearbeitung erhalten; Bereich zurücksetzen
  entfernt auch diese lokalen Attribute.

Leere Farbwerte erben; `FFFFFF` ist ausdrücklich Weiß. Beim Schriftgewicht unterscheiden
**Geerbt**, **Normal** und **Fett** zwischen fehlender Überschreibung und explizitem Wert.
Zurücksetzen erhält Name und Basisbeziehung. Eine Vorlage kann bis zu 32 Definitionen
einschließlich sich selbst in ihrer Vererbungskette haben. Fehlende Basen, Kreise und
ungültige Werte werden vor dem Übernehmen abgelehnt. Der Dialog bietet sich selbst
und abhängige Vorlagen nicht als Eltern an.

**Spaltenbänder** ergänzt die bisherigen Formatoptionen oberhalb der Seite. In schmalen
Fenstern stehen diese Optionen auf zwei Zeilen. Kopfzeilenwiederholung und die
bedingten Formatoptionen bleiben voneinander unabhängig.

## Formatauflösung

Zuerst werden die lokalen Eigenschaften vom ältesten Vorfahren bis zur gewählten
Vorlage zusammengeführt. Anschließend gelten die eingeschalteten Bereiche in dieser
Reihenfolge: Gesamtfläche, Zeilenbänder, Spaltenbänder, erste/letzte Spalte,
erste/letzte Zeile, oben links, oben rechts, unten links, unten rechts. Spätere
gesetzte Eigenschaften haben Vorrang. Direkte Zellfüllungen und direkte Zeichenformate
bleiben wirksam. Die Reihenfolge folgt den dokumentierten
[Word-Regeln für bedingte Tabellenformate](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/2ac331d4-cf1e-4fa0-8bca-6da74411e284).

Eckzellen benötigen die zugehörige Zeilen- und Spaltenoption. Aktive Randzeilen und
Randspalten werden beim jeweiligen Bandmuster ausgespart. Verbundene Zellen werden
an ihrer Ursprungszelle mit ihrer gesamten Ausdehnung ausgewertet. Eine über die volle
Breite verbundene erste Zelle kann daher beide oberen Eckbedingungen erfüllen.
[Microsoft: Tabellenformatoptionen und Eckbedingungen](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/814bc319-1027-4d3c-9cc5-2276e470e4a2)

Wie in der vorherigen Etappe gilt Erste-Zeile-Formatierung für den gesamten markierten
Kopfzeilenbereich. Die oberen Eckbereiche beziehen sich auf dessen tatsächliche erste
Zeile. Auf Folgeseiten werden dieselben aufgelösten Kopfzellen dargestellt und bearbeitet.

## DOCX und Zwischenablage

- `w:basedOn` erhält die Beziehung zwischen Vorlagen. Der Export enthält auch benötigte
  eingebaute Vorfahren, die keiner Tabelle direkt zugeordnet sind. Geerbte Eigenschaften
  werden nicht als lokale Werte festgeschrieben.
- `w:tblStyleRowBandSize` und `w:tblStyleColBandSize` erhalten die Bandgrößen. Ein fehlender
  Wert wird geerbt; ohne Vorgabe gilt eins.
  [Microsoft: TableStyleRowBandSize](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.tablestylerowbandsize?view=openxml-3.0.1)
- `band1Vert`, `band2Vert`, `nwCell`, `neCell`, `swCell` und `seCell` ergänzen die bereits
  unterstützten `w:tblStylePr`-Bereiche.
- `w:tblLook` liest sowohl die sechs Einzelattribute als auch die ältere `w:val`-Bitmaske.
  Sobald ein Einzelattribut vorhanden ist, wird die Bitmaske gemäß Word-Verhalten
  ignoriert. Beim Schreiben werden alle sechs Einzelattribute angegeben.
- RTF und HTML erhalten die effektiv aufgelösten Farben und Schriftattribute. Diese
  Ausgabe überträgt die Darstellung, während DOCX die bearbeitbaren Vererbungsbeziehungen
  erhält. Beim Kopieren von Zelltext werden die effektiven Zeichenwerte in das Fragment
  übernommen. Kopieren kompletter Tabellen als strukturierte Einfügefragmente bleibt offen.

## Gemeinsame Bibliotheken und Agentenaktionen

| Ort | Verantwortung |
| --- | --- |
| `lib/doc_core/table_styles.rs` | `TableRegionKind`, Vorlagengraph, Validierung, Bandgrößen, Vorrang und Auflösung |
| `lib/doc_io/table_styles.rs` | DOCX-Lesen und -Schreiben der lokalen Eigenschaften und Beziehungen |
| `lib/doc_view/src/table_style_editor.rs` | Gemeinsamer Vorlageneditor für native und WASM-Oberfläche |
| jfWord `src/app/table_style_actions.rs` | Schema und dünner Adapter für registrierte Dokumentaktionen |
| jfWord `src/ui/table_ui.rs` | Vorlagenauswahl, neue abgeleitete Vorlage und Bereichsschalter |

`document.table.styles.put` ergänzt `basedOn`, `inheritBorder`, `rowBandSize`,
`columnBandSize`, `band1Vertical`, `band2Vertical`, `topLeft`, `topRight`, `bottomLeft`
und `bottomRight`. `document.table.change` mit `operation: "look"` ergänzt
`bandedColumns`. `document.table.styles.list` liefert lokale Definitionen und zusätzlich
`effective` mit aufgelösten Eigenschaften; die Liste bleibt paginiert.

```json
{"action":"document.table.styles.put","input":{"id":"ResearchBase","name":"Forschung","basedOn":"jfTableBlue","rowBandSize":2,"columnBandSize":2,"band1Vertical":{"fill":"D5E9F3"},"topLeft":{"fill":"285942"}},"expectedRevision":10}
{"action":"document.table.styles.put","input":{"id":"ResearchChild","name":"Messwerte","basedOn":"ResearchBase","applyTo":2,"firstRow":{"format":{"bold":false}}},"expectedRevision":11}
{"action":"document.table.change","input":{"table":2,"operation":"look","look":{"firstRow":true,"firstColumn":true,"bandedColumns":true}},"expectedRevision":12}
```

`basedOn: ""` entfernt eine Basisbeziehung. `cloneFrom` erzeugt dagegen eine unabhängige,
aufgelöste Kopie und kann nicht gleichzeitig mit `basedOn` angegeben werden.
`inheritBorder: false` friert die derzeit geerbten Rahmenwerte ein; einzelne explizite
Rahmenwerte erhalten den jeweils anderen aufgelösten Wert. `inheritBorder: true`
zusammen mit expliziten Rahmenwerten wird als widersprüchlich abgelehnt.
Bandgröße `0`, `fill: ""` und `format: {}` entfernen die jeweilige Überschreibung.
Ausgelassene Eigenschaften bleiben unverändert. Jede Änderung erfordert die aktuelle
`expectedRevision` und unterstützt Undo/Redo.

## Reproduzierbare Prüfung

```powershell
python -X utf8 -B build_rs.py app jfWord --target all --out build/wordcraft-table-graphs -j 6
python -X utf8 -B apps/jfWord/tests/table_style_graphs.py --target both --out build/wordcraft-table-graphs
node apps/jfWord/tests/table_style_graphs_browser.mjs build/wordcraft-table-graphs
python -X utf8 -B apps/jfWord/tests/table_style_graphs.py --target both --out build/wordcraft-table-graphs --web-transfer
python -X utf8 -B apps/jfWord/tests/check.py --target both --build-dir build/wordcraft-table-graphs
```

Die Kernprüfungen decken mehrstufige Vererbung, explizites `false` und Weiß, Bandgrößen,
Ecken, direkte Zellwerte, verbundene Zellen, Undo/Redo, fehlerhafte Graphen, DOCX und
RTF-/HTML-/Zelltext-Zwischenablage ab. Die Agentenprüfung kontrolliert zusätzlich XML
unabhängig vom Rust-Codec und die Interpretation älterer Tabellenformat-Bitmasken.
Die Browsermatrix verwendet Chromium und Firefox bei 100/150/200 %, hell/dunkel,
echte Dialog- und Canvas-Eingaben sowie bestätigte dauerhafte Speicherung und Neuladen.

Abnahme auf dem abschließenden Build:

| Prüfung | Ergebnis |
| --- | --- |
| Vollständiger nativer Regressionssatz | 1041 bestandene Prozessprüfungen unter Windows/Linux, einschließlich ZIP/XML-Korpus, Fehlerfällen, Fuzz-Eingaben, UI-Skripten und Themes |
| Gemeinsamer Tabellenkern | 275 Assertions je Ziel, davon 28 neue zu Vorlagengraphen, bedingten Bereichen und Austausch |
| Neue Agentenabläufe | 4/4: Windows/Linux jeweils JSON und MCP |
| Bisherige Tabellen-Agentenabläufe | 12/12: Grundtabellen, Zellverbindungen und wiederholte Köpfe/Vorlagen |
| Allgemeine Dokument-Agentenabläufe | 4/4: Bearbeitung, Quellen, Zitate, Noten, Undo, Dateien und Darstellung |
| Neue Browserabläufe | 6/6: Vererbung, Bereichsdialog, Bandgrößen, Eckzellen zurücksetzen, Undo/Redo, Canvas-Eingabe und dauerhafte Speicherung |
| Bisherige Browserabläufe | 6/6: Wiederholte Kopfzeilen und Vorlagen, einschließlich direkter Bearbeitung auf Folgeseiten |
| Browserdateien → nativ | 24/24 bytegleiche DOCX-Rundläufe unter Windows/Linux |
| Builds | Windows, Linux, skalares/SIMD/Threads-WASM erfolgreich erstellt |
| Struktur und API | Build-Lint und Prüfung der generierten API-Dokumentation erfolgreich |

Maschinenlesbare Ergebnisse und Build-Prüfsummen:
`build/wordcraft-table-graphs/table-style-graphs-results.json` und
`build/wordcraft-table-graphs/table-style-graphs-build-manifest.json`.
Die Screenshot- und DOCX-Nachweise liegen unter `table-graphs-browser` und
`table-styles-browser` im selben Build-Verzeichnis. Der lokale Vorschaupfad
`http://127.0.0.1:8765/jfWord/` wurde byteweise gegen WASM, Loader und Worker dieses
Builds geprüft. Ein bereits geöffnetes Dokument wird nicht automatisch neu geladen.

## Verbleibende Grenzen

Die Vererbung betrifft die bereits unterstützten Tabellen-, Flächen- und Zeichenwerte.
Individuelle Zellrahmen und komplette Tabellenfragmente folgen in der
[anschließenden Etappe](CELL_BORDERS_AND_TABLE_CLIPBOARD.md). Bedingte
Absatz-/Zellgeometrie, Theme-Farben,
verschachtelte oder schwebende Tabellen, Tabellenformeln und Notenanker in Zellen
bleiben offen. Nicht unterstützte Eigenschaften fremder Word-Vorlagen werden weiterhin
über den vorhandenen Paketerhaltungsmodus behandelt. Die Anzahl expliziter
Vorlagendefinitionen bleibt auf 128 begrenzt; Bandgrößen sind auf 256 Zeilen und 32
Spalten begrenzt.

Diese Abnahme beweist weder vollständige Word-Kompatibilität noch einen Rundlauf mit
einer tatsächlichen Microsoft-Word-Installation. Das Threads-WASM wird gebaut;
die Browsermatrix ohne SharedArrayBuffer prüft skalare und SIMD-Ausführung.
