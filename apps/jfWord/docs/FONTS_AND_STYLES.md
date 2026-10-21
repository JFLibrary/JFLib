# Schriftfamilien und vererbbare Formatvorlagen

Stand: 11. Oktober 2026. Die Etappe ergänzt den vorhandenen Rich-Text-Editor und dieselben
Bibliotheken für Windows, Linux und WebAssembly. Der Build liegt unter
`RustLib/build/wordcraft-styles/{win,linux,web}`.

## Benutzung

Unter **Start → Schriftart** wählt die Schriftliste die Familie für die Auswahl. Ohne
Auswahl gilt der Wechsel für anschließend geschriebenen Text. Die Schriftgröße und die
bisherigen Zeichenformate bleiben erhalten. Formatübertragung nimmt jetzt auch die
Schriftfamilie mit; Zurücksetzen übernimmt wieder die Vorgaben der Absatzvorlage.

Der kleine Dialogstarter in **Start → Formatvorlagen** oder die drei Punkte rechts an
der Galerie öffnen die Vorlagenverwaltung. **Neu** erstellt eine Vorlage auf Grundlage
der aktuellen Absatzvorlage. **Basiert auf** wählt die übergeordnete Vorlage.

- **Zeichen:** Schriftfamilie, Größe, Fett, Kursiv, Unterstreichen, Durchstreichen und Farbe.
  Leere Schrift/Farbe, Größe 0 beziehungsweise „Geerbt“ lassen die Eigenschaft offen.
- **Absatz:** Ausrichtung, Abstände, Zeilenabstand, linke/rechte und Erstzeileneinzüge,
  Seitenumbruch davor sowie Gliederungsebene. Abstände und Einzüge werden in Punkt
  eingegeben; der Erstzeileneinzug darf für hängende Einzüge negativ sein.
- **Speichern** ändert die Definition und alle davon abhängigen Absätze.
  **Anwenden** speichert die Definition und weist sie den ausgewählten Absätzen zu.
  **Schließen** verwirft den Entwurf.

Die Vorschau verwendet die tatsächlich aufgelöste Dokumentfont. Weitere Galerieseiten
zeigen die eigenen Vorlagen. Änderungen unterstützen Undo/Redo; explizite Zeichen- und
Absatzwerte bleiben bei einer Änderung der Basisvorlage erhalten. Eigene Vorlagen mit
Gliederungsebene erscheinen in der Überschriftennavigation.

## Gemeinsame Bibliotheken

`doc_core` enthält Schriftfamilien, stabile Vorlagenkennungen, Definitionen, Vererbung,
Validierung und atomare Editoroperationen. `None` bedeutet bei Zeicheneigenschaften
Vererbung. Absatzwerte besitzen eine Eigenschaftsmaske, damit ausdrücklich „links“,
Einzug 0 und Seitenumbruch „aus“ geerbte Werte überschreiben können. Teiländerungen
machen andere Eigenschaften nicht unbeabsichtigt explizit.

Es gelten höchstens 128 gespeicherte Definitionen und 32 Vererbungsstufen. Zyklen,
unbekannte Basisvorlagen, doppelte Kennungen/Namen und unbrauchbare Textbereiche werden
vor der Änderung abgelehnt. Einzüge werden nach der Vererbung gemeinsam bewertet.
Die vorhandenen vier Vorlagen sind ebenfalls änderbar; alte Dokumente behalten ihre
bisherigen Standarddefinitionen.

`doc_view` stellt Fontauflösung, Messung, Seitendarstellung, Vorschaubilder und
Galerieschriftproben bereit. Drei Familien sind eingebettet: **DejaVu Sans**, **DejaVu
Serif** und **DejaVu Sans Mono**, jeweils normal/fett und mit synthetischen Kursivformen.
Native Apps können zusätzlich installierte, vom bestehenden Textparser unterstützte
Schriften verwenden. Fonts werden bedarfsgerecht geladen und über Dokumentwechsel hinweg
im Cache desselben Text-Engines gehalten. Der GUI-Style wählt keine Dokumentschrift.

Die Webversion verwendet die eingebetteten Familien. Andere Namen bleiben im Dokument
gespeichert, werden aber gegebenenfalls durch eine passende eingebettete Familie
dargestellt. Die Schriftliste weist per Tooltip auf eine Ersatzschrift hin. Vollständiger
Glyphenfallback, komplexes Schrift-Shaping, lokale Browserfont-Freigaben und Einbetten
beliebiger Fonts in DOCX sind weitere Ausbauschritte.

## Dateiformate und Zwischenablage

`doc_io` liest und schreibt im unterstützten DOCX-Profil benannte Absatzvorlagen,
`w:basedOn`, `w:name`, `w:outlineLvl`, Zeichen-/Absatzwerte und `w:rFonts`. Nicht gesetzte
Eigenschaften werden nicht als direkte Formatierung ausgegeben. Schriftfamilien und
Vorlagengraphen überstehen Speichern/Neuladen und die native Rich-Text-Zwischenablage.
Bei identischen Kennungen mit unterschiedlichen Definitionen erhält die eingefügte
Vorlage eine neue Kennung; vorhandene Zielvorlagen werden nicht überschrieben.

RTF und HTML exportieren aufgelöste Familien und Formate. Der RTF-Import übernimmt
Familiennamen einschließlich Unicode. Vorlagenidentitäten werden in diesen beiden
Austauschformaten nicht übertragen; RTF-Felder werden weiterhin als sichtbarer Text
übernommen. Der HTML-Pfad ist eine Ausgabe für externe Programme.

Der allgemeine DOCX-Erhaltungsmodus bleibt erhalten. Nicht repräsentierbare Vorlagen,
Themefont-Verweise, gemischte Schriftzuordnungen und weitere Word-Eigenschaften führen
nicht zu einem behaupteten vollständigen Modellimport. Ein echter Word-Roundtrip und
vollständige Word-Dateikompatibilität bleiben gesonderte Abnahmen.
Die Unterscheidung zwischen direkter Formatierung und Stilauflösung folgt der
[Microsoft-Dokumentation zur Formatierungshierarchie](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/f7130225-2368-48f3-acae-a9d278d0fb25).

## Agentenaktionen

CLI, JSON, MCP und der aktivierte Browserzugang benutzen dieselben Editoroperationen:

| Aktion | Eingabe und Wirkung |
| --- | --- |
| `document.styles.list` | `offset`/`limit`; Definitionen, Basiskennungen und aufgelöste Zeichenwerte |
| `document.styles.put` | `id` optional, `name`, `basedOn`, `format`, `paragraph`, `outlineLevel`; ersetzt eine Definition oder legt sie an |
| `document.paragraph.style` | `paragraph`, `style`; weist eine vorhandene Kennung zu |
| `document.text.format` | Textbereich und `format.fontFamily` neben den vorhandenen Zeichenwerten |
| `document.text.format.inspect` | Liefert aufgelöste Werte einschließlich Schriftfamilie |

Änderungen verlangen `expectedRevision`. Kennungen sind `Normal`, `Title`, `Heading1`,
`Heading2` oder `jfStyleN`. Beim Ersetzen einer Definition bedeutet eine fehlende Eigenschaft
Vererbung, keine Teilaktualisierung. Leeres `basedOn` entfernt die Basisverknüpfung;
ohne Angabe wird für eigene Vorlagen `Normal` verwendet. Absatzmaße in der Schnittstelle
sind Twips, Zeilenabstand 1/240 Zeilen und Schriftgrößen halbe Punkte.

## Reproduzierbare Prüfungen

Vom Verzeichnis `RustLib`:

```powershell
python build_rs.py app jfWord --target all --out build/wordcraft-styles -j 6
python apps/jfWord/tests/check.py --target both --build-dir build/wordcraft-styles
python apps/jfWord/tests/styles.py --target both --out build/wordcraft-styles
python apps/jfWord/tests/agent.py --target both --build-dir build/wordcraft-styles
node apps/jfWord/tests/styles_browser.mjs build/wordcraft-styles
python apps/jfWord/tests/styles.py --target both --out build/wordcraft-styles --web-transfer
node tools/tests/web_apps.mjs build/wordcraft-styles --engine=chromium,firefox --only=jfWord
python tools/tests/web_transfer.py build/wordcraft-styles
```

`src/checks/style_checks.rs` prüft Vererbung, direkte Ausnahmen, Undo, ungültige Graphen,
Einzüge, Clipboard-Kollisionen, DOCX/RTF/HTML und tatsächliche Schriftmetriken.
`tests/styles.uis` bedient Schriftliste, freie Familieneingabe, Dialog, Galerie und Undo
über die vorhandene UI-Runtime. `tests/styles.py` prüft beide nativen Ziele über JSON und
MCP sowie das DOCX-XML unabhängig. Der Browser-Test prüft Chromium und Firefox, hell bei
100 % und dunkel bei 150 %, mit echten Maus-/Tasteneingaben, nativer DOCX-Übernahme und
bestätigtem dauerhaftem Speichern/Neuladen. Artefakte liegen ausschließlich unter `build`.

Prüfstand dieser Etappe:

| Prüfung | Ergebnis |
| --- | --- |
| Builds | Windows, Linux und WASM erfolgreich, ohne Cargo/Fremd-Crates |
| Interne Prüfungen | Je 9.391 bestanden unter Windows/Linux, einschließlich 35 neuer Schrift-/Vorlagenprüfungen |
| Gesamte native Dateiregression | Windows/Linux bestanden: ZIP/XML, Erhaltungsmodus, Ablehnungen, Mutationen, Rich-Text-, Quellen-, Noten- und UI-Abläufe |
| Fachaktionen | Vier neue Vorlagenszenarien über Windows/Linux × JSON/MCP; bestehende vier Agentenszenarien weiterhin erfolgreich |
| Browser | Vier Vorlagenszenarien unter Chromium/Firefox × hell/100 % und dunkel/150 %; zusätzlich sechs allgemeine jfWord-Browserfälle |
| DOCX-Übertragung | Native Vorlagenpakete im Browser bytegleich; acht Browser→native Kombinationen mit eigenen Vorlagen ebenfalls bytegleich (5.225 Bytes) |
| Bestehender Datenaustausch | Vier weitere Browser→native Dokumente mit Unicode, Zeichenformaten und Quellen bytegleich (5.038 Bytes) |
| Lint/API | `build_rs.py check` erfolgreich; gemeinsame API-Dokumentation neu erzeugt |

Nach der vollständigen Dateiregression wurden die Unicode-Namensvalidierung und der
Seiteneinstellungsbezug der Dialogvorschau ergänzt. Die finalen Builds wurden anschließend
erneut mit den internen Prüfungen, dem nativen Vorlagendialog, den vier Vorlagenprotokoll-
und den vier Vorlagenbrowserfällen geprüft. Diese Nachweise ersetzen keine Word-Abnahme.
