# Hervorhebung und Hoch-/Tiefstellung

Stand: 12. Oktober 2026. Fortsetzung der [Schrift- und Vorlagenetappe](FONTS_AND_STYLES.md).
Build: `RustLib/build/wordcraft-character/{win,linux,web}`.

## Bedienung

Im Register **Start** sind die Knöpfe **Tiefgestellt** und **Hochgestellt** sowie das
Menü **Texthervorhebung** aktiv. Die Markierung umfasst 16 Dokumentfarben und **Keine Farbe**.
Die Eigenschaften gelten für die Auswahl über Format- und Absatzgrenzen hinweg; an einer
Schreibposition gelten sie für neu eingegebenen Text. Hoch- und Tiefstellung schließen sich
gegenseitig aus. Ein zweiter Klick auf den aktiven Knopf stellt auf die normale Grundlinie zurück.
Die Befehle sind auch als `Strg+=` und `Strg+Umschalt+=` registriert (physische Plus-/Gleich-Taste;
die konkrete Taste hängt vom Tastaturlayout ab).

Eine gemischte Auswahl erscheint nicht als einheitlich hoch-/tiefgestellt. Ein Klick wendet
den gewählten Zustand auf die gesamte Auswahl an. Auch die bisherigen Fett-/Kursiv-/
Unterstreichen-/Durchstreichen-Schalter berücksichtigen jetzt alle ausgewählten Textläufe.
Die Knopfzustände werden nach einer Tastaturauswahl noch vor der Bildausgabe aktualisiert.
Das Lineal zeichnet beim Wechsel zwischen Absätzen mit unterschiedlichen Einzügen alle
Markierungen neu; der gemeinsame Darstellungsbaustein berücksichtigt dafür auch Änderungen
der Auswahl ohne neue Dokumentrevision.

Der Formatvorlagendialog bietet für beide Eigenschaften **Geerbt** und explizite Werte an.
**Keine Farbe** und **Normal** überschreiben eine entsprechende Elternvorlage dauerhaft.
Galerie und Dialogvorschau zeigen Hintergrundfarbe, relative Zeichenposition und vorhandene
Unterstreichung/Durchstreichung. Dokumentbefehle werden während eines offenen Vorlagendialogs
zurückgehalten, damit Tastenkürzel dessen Entwurf nicht unbemerkt auf das Dokument anwenden.

Formatpinsel, Formatierung zurücksetzen, Undo/Redo und das native Rich-Text-Clipboard
berücksichtigen die neuen Eigenschaften. Quellen, Zitate und erzeugte Felder behalten ihre
bisherigen Bearbeitungsgrenzen. Notentext verwendet dasselbe Formatmodell und dieselbe Darstellung.

## Gemeinsame Implementierung

- `doc_core::Highlight`: feste Palette mit stabilen Namen und exakten RGB-Werten.
- `doc_core::TextPositioning`: `Baseline`, `Superscript`, `Subscript`.
- `TextFormat` enthält optionale Overrides. Ein fehlender Wert erbt; ein explizites
  `Highlight::None` beziehungsweise `Baseline` schaltet die geerbte Eigenschaft aus.
- `Document::story_character_common` liefert gemeinsame effektive Werte einer gesamten
  UTF-8-Auswahl. Fehlende Werte in dieser Zusammenfassung bedeuten unterschiedliche Formate.
  Das bisherige `story_character_format` für Formatpinsel und einzelne Einfügepositionen bleibt bestehen.
- `doc_layout` berücksichtigt Grundlinienverschiebungen bei Zeilenhöhe, Seitenumbruch und
  Notenumbruch. `doc_view` und die Caret-Messung verwenden dieselbe dargestellte Zeichengröße.
  Hintergründe werden vor allen Glyphen einer Zeile gezeichnet, damit sie keine kursiven
  Überhänge benachbarter Textläufe überdecken.
- `ui::RibbonSample` ist ein wiederverwendbarer Galeriebaustein mit Schrift, Größe,
  Hintergrund, Grundlinienverschiebung und Textdekoration. Er kennt kein jfWord-Dokumentmodell.

Die nominelle Schriftgröße bleibt gespeichert. Die gemeinsame Darstellung verwendet für
Skripte zwei Drittel dieser Größe, auf halbe Punkte gerundet. Hochstellung verschiebt die
Grundlinie um ein Drittel des nominellen Gevierts nach oben, Tiefstellung um ein Fünftel
nach unten. Dies ist eine ausdrücklich definierte, plattformgleiche Darstellungspolitik;
schriftspezifische OpenType-Skriptmetriken und Word-identischer Umbruch sind noch nicht zugesichert.
Anwendungsthemen und Zoom verändern die Dokumentfarben oder gespeicherten Größen nicht.

## Dateiformate und Agenten

DOCX verwendet `w:highlight` und `w:vertAlign`, auch in Vorlagen und Notenteilen. Unbekannte
Werte werden nicht angenähert. Die vorhandene Erhaltung nicht unterstützter DOCX-Strukturen
bleibt bestehen. Die Trennung von nomineller Größe und Skriptposition entspricht
[Microsofts Beschreibung von vertAlign](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.verticaltextalignment).
Die Palette folgt den [WordprocessingML-Hervorhebungswerten](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.highlightcolorvalues).

RTF übernimmt `highlight`, `super`, `sub`, `nosupersub` sowie Gruppenrückkehr und `plain`-Reset.
Diese Steuerwörter sind in [Microsofts RTF-Zeichenformaten](https://learn.microsoft.com/en-us/windows/win32/controls/about-text-object-model)
beschrieben. Importierte Hervorhebungsfarben außerhalb der Word-Palette werden ausdrücklich
abgelehnt. HTML-Ausgabe verwendet `background-color`, eine verkleinerte Schriftgröße und
eine numerische Grundlinienverschiebung. Ein HTML-Importer ist damit nicht umgesetzt.

`document.text.format`, `document.text.format.replace` und `document.styles.put` akzeptieren:

```json
{"highlight":"yellow","positioning":"subscript"}
```

`highlight` erlaubt `none`, `yellow`, `green`, `cyan`, `magenta`, `blue`, `red`, `darkBlue`,
`darkCyan`, `darkGreen`, `darkMagenta`, `darkRed`, `darkYellow`, `darkGray`, `lightGray`, `black`,
`white`. `positioning` erlaubt `baseline`, `superscript`, `subscript`.

Die neue lesende Aktion `document.text.format.summary` nimmt dieselbe Auswahl wie
`document.text.format.inspect` entgegen und liefert gemeinsame effektive Eigenschaften;
`null` bedeutet gemischt. Verändernde Aktionen verwenden weiterhin eine erwartete Revision.
Dieselben Aktionen stehen nativ über JSON/MCP und im Browser über den aktivierten Dispatcher bereit.

## Nachweise

`src/checks/character_checks.rs` prüft alle 51 Palette-/Positionskombinationen in DOCX und RTF,
Vorlagenvererbung, explizites Löschen, gemischte Auswahl, Undo/Redo, Clipboard-Nahtstellen,
Noten, CSS-Ausgabe sowie echte Glyphen- und Zeilengrenzen von 4 bis 144 Punkt.
Die Seitenprüfung prüft die tatsächlich ausgegebenen Farbwerte.

`tests/characters.py` führt den Fachablauf auf Windows/Linux über JSON und MCP aus und liest
das erzeugte DOCX-XML unabhängig. `tests/characters.uis` bedient Auswahl, Menüband, Palette,
Einfügeformat und Undo über die native UI-Runtime. `tests/characters_browser.mjs` verwendet
echte Pointer-/DOM-Eingabe in Chromium und Firefox, hell bei 100 % und dunkel bei 150 %;
native Dateien, beständig gespeicherte Browserdateien und erneutes Öffnen werden verglichen.
Beide neuen Vorlageneigenschaften werden auch über die tatsächlichen Dialogfelder geändert;
direkte Ausnahmen und Undo werden danach geprüft.
Aufnahmen und Dateien liegen unter `build/wordcraft-character/character-browser`.

Prüfstand dieser Etappe:

| Prüfung | Ergebnis |
| --- | --- |
| Builds | Windows, Linux und WASM erfolgreich, weiterhin ohne Cargo/Fremd-Crates |
| Interne Prüfungen mit Dateifixtures | Je 9.639 bestanden unter Windows/Linux; 164 neue Zeichenformatprüfungen gegenüber der vorherigen Etappe |
| Gesamte native Regression | 1.029 Prozessprüfungen unter Windows/Linux bestanden: ZIP/OPC/XML, Erhaltungsmodus, Ablehnungen, Mutationen, Quellen-/Noten-/Feld-/Clipboard-Abläufe, Oberfläche und Themes |
| Fachaktionen | Vier Zeichenformatszenarien über Windows/Linux × JSON/MCP; bisherige vier Vorlagen- und vier Agentenszenarien ebenfalls bestanden |
| Native Oberfläche | Neue Zeichenformate einschließlich Tastenkürzeln und Dialogmodalität; Absatz-/Feldnavigation nach Korrektur der Linealaktualisierung ohne Darstellungswarnungen |
| Browser | Vier Zeichenformatfälle unter Chromium/Firefox × hell/100 % und dunkel/150 %, einschließlich tatsächlicher Vorlagendialogeingaben und dauerhaftem Speichern/Neuladen |
| DOCX-Übertragung | Native Zeichenformatpakete im Browser bytegleich; alle acht Browser→Windows/Linux-Kombinationen bytegleich (5.740 Bytes) |
| Bisherige Browserfälle | Vier Vorlagenfälle, sechs allgemeine jfWord-Fälle und vier weitere DOCX-Übertragungen mit Quellen bestanden |
| Lint/API | `build_rs.py check` und `git diff --check` erfolgreich; API-Dokumentation mit 3.998 Einträgen neu erzeugt |

Die letzten Änderungen an der Linealaktualisierung wurden mit dem vollständigen Neubau,
den nativen Absatz-/Feld-/Zeichenformatabläufen sowie den vier Zeichenformatbrowserfällen
erneut geprüft. `character-build-manifest.json` im Buildordner enthält die SHA-256-Werte
der ausgelieferten Windows-, Linux- und WASM-Dateien. Die vollständige native Regression
lief anschließend mit diesen Builds erfolgreich durch (`native-acceptance.log`, 340,8 Sekunden;
das Protokoll enthält auch die erwarteten Ablehnungen beschädigter Testdateien).
Der Server unter
`http://127.0.0.1:8765/jfWord/` liefert nachweislich dieses WASM; ein bereits offener Tab
verwendet den neuen Stand nach dem Neuladen.

Diese Etappe ersetzt keine vollständige Word-Abnahme. Weitere Zeichenattribute,
Zeichen-/Tabellenvorlagen, interaktive Lineale, Tabstopps und Listen bleiben im Ausbauplan.
