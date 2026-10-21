# Umsetzung und Abnahme: jfOffice, Agenten und Browser

Stand: 14. Oktober 2026. Der [Featurekatalog](../FEATURES.md) und der vereinbarte gemeinsame
Plan bleiben das Produktziel. Diese Übersicht beschreibt die tatsächlich umgesetzte
Etappe und ihre Nachweise. Sie ersetzt keine vollständige Word-Konformitätsprüfung.

Aktueller Tabellenschritt: [Tabellenformeln und Zahlenformate](TABLE_FORMULAS.md).
Der gemeinsame Feldkern berechnet Zell- und Bereichsbezüge, Richtungsaggregate,
Abhängigkeiten und Zahlenformate. Einfügen, Bearbeiten, DOCX und strukturierte
Zwischenablage verwenden denselben Code nativ und in WASM. Builds unter
`build/wordcraft-table-formulas`; die [Zellnoten](TABLE_CELL_NOTES.md) bleiben integriert.

Die [Umbenennung zu jfgui](../../../docs/JFGUI_RENAME.md) ist für die aktiven native-/Web-
Schnittstellen nachgeprüft. Der gemeinsame Browserhost übernimmt vorhandene Dateien aus
der alten Datenbank, ohne neue Konfliktdateien zu überschreiben. Die CLI heißt jetzt
`tools/jfgui-agent.py` und verwendet die aktuellen MCP-Werkzeugnamen.

## Etappen

| Planbereich | Vorhandener Stand | Weitere Arbeit |
| --- | --- | --- |
| Struktur und Build | jfWord unter `src/{app,ui,workflows,checks}`; neue Bibliotheken mit `src/lib.rs`; gemeinsamer Quellresolver; Konfliktprüfung zweier Einstiegspunkte; Referenzcode ausgeschlossen | Weitere Apps behalten wie vereinbart ihre bestehenden Strukturen |
| Gemeinsame Verträge | Aktionsregistrierung, strikter JSON-Kern, Schemas, Revisionen, strukturierte Fehler, Aufgaben, Ressourcen, Plattformfähigkeiten und schrittweise Runtime | Jede zusätzliche Fachaktion muss ihre eigenen Fähigkeiten und Transaktionsgrenzen angeben |
| Native Agentensteuerung | Opt-in, stdio-JSON/MCP, lokale benutzergebundene IPC, allgemeine Widget-/Fenstersteuerung, CLI und fachliche Adapter für jfWord sowie Grafikfunktionen | Nicht jeder historische GUI-Befehl ist schon als eigene Fachaktion registriert; jfWord bietet noch keine atomare externe Dokumenttransaktion |
| jfOffice und Schreibkern | Eigener blauer Style, integrierter Fensterkopf, alle elf Register, gruppiertes Menüband, direkte vorhandene Rich-Text-Bearbeitung, gemeinsame Dokumentdarstellung, echte Vorschaubilder, Navigation, Fokus, Zoom, Schriftfamilien, vererbbare Absatzvorlagen, interaktive Lineale und individuelle Tabstopps | Komplexe Schriften und weitere Office-Werkzeuge siehe unten |
| Browserport | Direkter WASM-Build für registrierte GUI-Apps, gemeinsamer Worker/Canvas-Host, portable Kernpfade, virtuelle Dateien, asynchrone Dienste, aktivierbarer JSON-/MessagePort-Zugang und optionale lokale Bridge | Betriebssystem-IME, vollständige Fontabdeckung und zusätzliche Browserdienste |
| Gesamtabnahme | Native Gesamtprüfung, Referenzparität, Chromium-/Firefox-Abläufe und portable Vergleichskorpora bestanden; bestehende Formatprüfungen bleiben erhalten | Alle Office-Funktionen, umfassende Browserdienstprüfung und echte Word-Referenzprüfung |

Die Dokumentbibliotheken bleiben `no_std` mit `core`/`alloc`; der gemeinsame Build
startet `rustc` direkt. Der bereits vorhandene Text-Shaping-Unterbau verwendet ein
separates Cargo-Projekt mit festgelegten Abhängigkeiten, beschrieben in
[Text-Shaping-Hinweisen](../../../lib/text_shape/NOTICE.md). Der gemeinsame
JavaScript-Host übersetzt Browserdienste; die Dokumentlogik bleibt im Rust-Kern.

## jfOffice und Dokumentdarstellung

Der aktuelle [Tabellenausbau](EXACT_TABLE_ROW_HEIGHTS.md) ergänzt genaue Zeilenhöhen
neben der bisherigen Mindesthöhe, mit erhaltener Textüberlänge, gemeinsamer
Seitendarstellung und DOCX-/RTF-/HTML-Austausch. Die [Zeilenumbruchregeln](TABLE_ROW_BREAKS.md)
bleiben unabhängig einstellbar. Die vorherigen
[mehrseitigen schwebenden Tabellen und HTML-Positionen](FLOATING_TABLE_PAGES_AND_HTML.md)
bleiben integriert. Dokumentkern, Layout, Codecs und Bedienbausteine sind gemeinsame
Bibliotheksfunktionen. Aktuelle Builds: `build/wordcraft-table-formulas`.

Der Style ist eine eigene Implementierung unter `styles/jfoffice`. Licht- und Dunkelpalette,
Control-Zeichnung, Symbole und Metriken liegen dort; die allgemeinen Interaktionen bleiben
in `lib/ui`. Es gibt keine Quellpfadimporte aus einem anderen Style. Die nachfolgende
[Designetappe](VISUAL_REFERENCE.md) orientiert sich an den konkreten WordCraft-Bildern
und Designtokens. App-/egui-/Widgetcode wird nicht eingebunden. Offen lizenzierte Inter-Fonts
und die nachgezeichneten Symbolmotive sind mit OFL- beziehungsweise MIT-Hinweisen dokumentiert.

Vorhanden sind Datei, Start, Einfügen, Zeichnen, Entwurf, Layout, Referenzen, Sendungen,
Überprüfen, Ansicht und Hilfe. Noch fehlende Funktionen sind sichtbar deaktiviert.
Bestehende Text-, Absatz-, Quellen-, Zitat-, Noten- und Feldabläufe bleiben erreichbar.
Die Startseite zeigt echte, mit demselben Dokumentrenderer erzeugte Vorlagenbilder.
Der aktuelle direkte Screenshotabgleich unter `build/wordcraft-align` korrigiert zusätzlich
Panelkonturen, Suchfeld, Typografie, Galerie, Vorschaukarten, Seitenzentrierung und Status.
Die neuen Vorschau-/Text-/Suchbausteine und die portable logarithmische Sliderabbildung
liegen in `lib/ui`; der [Designnachweis](VISUAL_REFERENCE.md) trennt pixelgleiche Messbereiche
von den noch verbleibenden Rasterisierungs- und Inhaltsunterschieden.

Die gemeinsamen Ribbon-Bausteine umfassen große Aktionen mit Symbol über der Beschriftung,
geteilte Aktionen und Dialogstarter. Beide Hälften einer geteilten Aktion haben eigene
Widgetnamen und Tastaturfokus; das Dropdown übernimmt die normalen Menü-/Escape-Verträge.
jfWord verwendet diese für Einfügen/Nur-Text-Einfügen und den Absatzdialog. Das gleiche Menü
bietet Zeichenformat aufnehmen, anwenden und löschen. Aufnahme verändert weder Dokument
noch Zwischenablage; Anwenden ersetzt die sieben vorhandenen effektiven Zeichenwerte einschließlich Schriftfamilie,
Löschen erbt vom Absatzstil. Text, Felder und Absatzstil bleiben erhalten. Änderungen sind
atomare, rückgängig machbare Dokumentoperationen.
Die direkte Schriftgrößeneingabe ergänzt die bisherige Vorwahlliste: 4–144 pt in halben
Punkten, mit Grenzprüfung und gemeinsamer Zeichenformatierung bei erhaltener Textauswahl.

`lib/doc_view/src` enthält Dokumentfonts, Fontmessung, Seitenzeichnung, Vorschaubilder und
Koordinaten. Die Standardfonts sind von den GUI-Fonts getrennt. Änderungen an UI-Style,
Theme oder DPI verändern keine Dokumenteigenschaften oder Umbruchentscheidung. Die ursprüngliche Auslagerung ließ das Dokumentformat unverändert; der nachfolgende
[Schrift-/Vorlagenausbau](FONTS_AND_STYLES.md) erweitert Modell, Codecs und Renderer gemeinsam.
Die anschließende [Zeichenformatetappe](CHARACTER_FORMATTING.md) ergänzt Hervorhebung,
Hoch-/Tiefstellung, gemeinsame Eigenschaften gemischter Auswahlen und Galerieproben.

Die Navigation zeigt Überschriftentexte mit Ebeneneinzug, paginierte Seitenvorschaubilder
und Suchtreffer. Treffer berücksichtigen Groß-/Kleinschreibung und ganze Wörter, reichen
über Formatwechsel und führen auch in Fuß-/Endnoten zur bestehenden UTF-8-Auswahl.
Generierte Feldresultate werden bei dieser Textsuche nicht als frei editierbarer Text
behandelt. Ohne Überschriften bleibt eine Absatzliste verfügbar.

Fokusmodus blendet Bedienbereiche aus und stellt die vorher gewählten Bereiche wieder
her. Zoom ist auf 10–500 Prozent begrenzt; Vorwahl, direkte Angabe, Plus/Minus, Schieber
und Seitenbreite teilen einen Zustand. Diese Ansichtsaktionen erzeugen keine Undo-Schritte
im Dokument. Der Status nennt die aktuelle Schreib- beziehungsweise Vorschauseite.

Noch offen sind:

- Vollständiger Glyphenfallback, komplexes Shaping, zusätzliche Schriftmetadaten und
  Browserzugriff auf lokale Fonts. Schriftfamilien und vererbbare benannte Absatzvorlagen
  mit Anlegen/Ändern sind umgesetzt. Benannte Tabellenformatvorlagen und wiederholte
  Kopfzeilen sind in [TABLE_HEADERS_AND_STYLES.md](TABLE_HEADERS_AND_STYLES.md) beschrieben;
  [Tabellenstil-Vererbung, Spaltenbänder und Eckzellen](TABLE_STYLE_INHERITANCE.md)
  sind inzwischen umgesetzt; Zeichenformatvorlagen und weitere Word-Tabelleneigenschaften bleiben offen.
- Zusätzliche Zeichenformate jenseits der derzeitigen neun Eigenschaften, beispielsweise
  Kapitälchen, versteckter Text und erweiterte Unterstreichungsarten. Hervorhebung,
  Hoch-/Tiefstellung, Formatübertragung und Zurücksetzen sind vorhanden.
- Weitere Absatz-/Listenfunktionen; interaktive Lineale und individuelle Tabstopps
  sind inzwischen in [RULERS_AND_TABS.md](RULERS_AND_TABS.md) umgesetzt.
- Anzeige von Formatierungszeichen im Seitentext.
- Mehrere Seiten nebeneinander; der Schreibbereich zeigt Seiten untereinander,
  die Vorschau zeigt eine ausgewählte Seite.
- Die weitergehenden Einfügen-, Zeichnen-, Entwurf-, Serienbrief- und Überprüfungsfunktionen
  aus dem Featurekatalog, einschließlich zugehöriger Dateiformateigenschaften.

## Agentensteuerung

Ohne explizite Aktivierung wird kein externer Steuerungsendpunkt geöffnet. `--agent`
aktiviert die lokale Verbindung einer nativen Instanz, `--agent-json` und `--agent-mcp`
eine Headless-Sitzung über Standardstreams. jfWord zeigt den Aktivierungs-/Verbindungszustand
im Fensterkopf beziehungsweise im Fokusmodus in der Statusleiste. Ein Sprachmodell wird
dafür nicht in der App gestartet.

Allgemeine UI-Aktionen und fachliche Dokumentaktionen verwenden die bestehenden
Ereignis-/Dokumentwege. Der Dispatcher prüft Schema, Fähigkeiten, Modalität und erwartete
Revision, bevor er verändernde Fachaktionen ausführt. Datei-/Bilddaten sind Ressourcen.
Die Bibliothek enthält keine allgemeine Shell-Ausführung.

Eine registrierte Undo-Unterstützung ist keine Zusage atomarer externer Stapel: Ohne eine
dafür registrierte Transaktion wird ein atomarer Stapel vor Ausführung abgewiesen.
Aufgabenstatus und Abbruch sind gemeinsame Verträge; konkrete lange App-Abläufe benötigen
weiterhin eigene sichere Unterbrechungs- und Commit-Punkte.

CLI, Protokollversionen, native Transportsicherheit und die optionale Browser-Bridge sind
in [AGENT_WEB.md](../../../docs/AGENT_WEB.md) und
[agent_core/README.md](../../../lib/agent_core/README.md) dokumentiert.

## Browser und Dateisitzung

`--target web` baut dasselbe App-Projekt für `wasm32-unknown-unknown`. `--target all`
umfasst Windows, Linux und Web; `both` bleibt Windows plus Linux. WASM, Loader, Worker,
HTML und benötigte Assets liegen unter `build/<Ausgabe>/web/<App>/` beziehungsweise
`build/web/<App>/`. Sie werden über HTTP(S) ausgeliefert.

Der App-Code läuft im Worker. Die Softwareausgabe wird in Canvas dargestellt;
ein DOM-Eingabeelement übernimmt Texteingabe und Composition-Ereignisse. Bibliotheksdialoge
bleiben innerhalb der Canvas-Fläche. Virtuelle Dateien verbinden die vorhandenen Codecs
mit Browserdaten. Persistente Speicherung hat einen eigenen asynchronen Abschluss.

jfWord meldet erst nach bestätigter Speicherung einen gespeicherten Zustand. Erfolgen
währenddessen neuere Änderungen oder ein Dokumentwechsel, wird die ältere Bestätigung
nicht auf die neue Fassung übertragen. Quota, verweigerter Zugriff, Abbruch und fehlende
Persistenz sind eigene Fehlerzustände. Native Erweiterungen und Betriebssysteminstallation
werden nicht als verfügbare Browserfähigkeiten dargestellt.

Template, Gallery, jfWord, Kritzel und PaintCraft wurden in Chromium und Firefox geprüft. DOM-Eingabe und
simulierte Composition-Ereignisse wurden bis in den Rust-Dokumentzustand geprüft; dies
ersetzt noch keine Abnahme jeder betriebssystemseitigen IME. CJK-Text kann übernommen
werden, die eingebetteten Fonts haben jedoch begrenzte CJK-Glyphendeckung.

Die eigene Browser-Agentenschnittstelle wird ausdrücklich aktiviert. JavaScript und
MessagePort erreichen denselben Dispatcher. Eine separate, ausdrücklich verbundene
Loopback-Bridge ermöglicht CLI/MCP; die gewöhnliche Web-App benötigt keinen Backendserver.

## Nachweise des ursprünglichen gemeinsamen Unterbaus

Die Gesamtzahlen für alle Apps beziehen sich auf den gemeinsamen Unterbau unter
`build/office-universal`. Der anschließende jfWord-Designumbau ist separat unter
`build/wordcraft-design` geprüft; seine aktuellen Messwerte und Nachweise stehen im
[Designbericht](VISUAL_REFERENCE.md).

| Prüfung | Ergebnis und Grenze |
| --- | --- |
| Auslieferbare Builds | Native Apps für Windows/Linux und alle fünf GUI-Apps als Browserbundles unter `build/office-universal` |
| Native Gesamtprüfung | 295/295 Skripte, einschließlich Plugins und API-Prüfung; abschließender Formatkürzel-Fix anschließend separat auf beiden Zielen geprüft |
| Native UI-Skripte | 35 Skripte je Ziel, insgesamt 70 erfolgreiche Läufe mit `--verify-dirty`, ohne `UIWARN` |
| Native Selbstprüfungen | Windows und Linux nach dem Designumbau jeweils 9.411 bestanden, 0 fehlgeschlagen; im UI-Satz durch `core.uis` ausgeführt |
| Dokumentadapter | Unabhängige GUI-/Dokumentfonts, gleiche Layout-/Thumbnailpixel, wiederholte Dokumentwechsel, Grenzen und unveränderte Modelldaten geprüft |
| Darstellung jfWord | Designumbau: 66 Aufnahmen aller Register und der Dateioberfläche, hell/dunkel, 100/150/200 Prozent, 600–1920 DIP; zusätzlich 48 Bilder mit exakt gleicher Dokumentdarstellung bei Register-/Themewechseln und zwischen Windows/Linux |
| Neue Navigation | Überschriften-/Seitensprünge, UTF-8- und formatübergreifende Notensuche, Wort-/Großschreibungsfilter, Zoomgrenzen, Fokus und Bereichswiederherstellung in `navigation.uis` |
| Ribbon-Aktionen | Eigenständiger Dropdown-Fokus, Enter/Space/Alt+Down/F4, Esc-Rückkehr, Einfüge-/Formataktionen und Absatz-Dialogstarter in `ribbon_actions.uis`; 600/1000/1240 DIP und 100/150/200 Prozent einschließlich verschachtelter Überläufe |
| Direkte Schriftgröße | Vorwahlen und freie Zahlen, Halbpunkt-Schritte, ungültige Eingabe, Grenzen, Undo/Redo und Auswahlgrenzen in `font_size.uis` |
| Browser-Basis | Template/Gallery/jfWord/Kritzel/PaintCraft in Chromium und Firefox: Canvas, DPI/Größe, Eingabe, Dialoge, Grafik-/Agentenaktionen und bestätigtes Speichern; Detailfälle in `tools/tests/web_apps.mjs` |
| Finale Browsermatrix | 18/18 Fälle; echte DOM-Tastenereignisse für Formatübertragung/Reset und Widerruf des Agentenzugangs |
| Speicherfehler und Konkurrenz | 8/8 Fälle: IndexedDB-Quota/Denied sowie verzögerte Commit-Bestätigung während Bearbeitung oder Dokumentwechsel; neuerer Zustand bleibt ungespeichert |
| Browser/native DOCX | 4/4 Kombinationen aus Chromium/Firefox und Windows/Linux: Unicode-/Format-/Quellen-Prüfdokument bytegleich importiert und exportiert, jeweils 5038 Bytes |
| Bestehende Dateikompatibilität | Ursprünglich 1017, nach dem Designumbau 1021 native Prozessprüfungen sowie unabhängige ZIP/XML-Prüfungen durch `tests/check.py`; vollständige Word-Kompatibilität ist kein Ergebnis dieser Etappe |
| Native Referenzen | 158/158 UI-Vergleiche identisch für Gallery/Template/Kritzel und 78.414 bytegleiche Rechenkern-/Codec-Ergebnisse; Referenzbuilds unverändert |
| Portabler Korpus | Windows/Linux/WASM: identischer Digest `5a22a1f803872742`; transzendente Funktionen zusätzlich mit expliziten Toleranzen geprüft |

Die bisherigen Mausauswahlprüfungen wurden an die neue Seiten-/Navigationsgeometrie
angepasst. Die Rich-Text-, Feld-, Noten-, Quellen- und Dateiprüfungen bleiben im Satz.
Alle Ergebnisse und Screenshots liegen unter `build`; Tests schreiben nicht in das
persönliche Quellen- oder Einstellungsprofil.

Die anschließende [Schrift-/Vorlagenetappe](FONTS_AND_STYLES.md) wird unter
`build/wordcraft-styles` separat geprüft; die obigen historischen Gesamtzahlen werden
dadurch nicht nachträglich umgedeutet.

Die [Lineal-/Tabulatorenetappe](RULERS_AND_TABS.md) führt den Schreibkern in den
gemeinsamen Bibliotheken fort. Aktuelle Builds liegen unter `build/wordcraft-tabs`;
Details zu Bedienung, Agentenaktionen, Austausch und Prüfungen stehen im Etappenbericht.

Weitere Nachweise sind für echte Betriebssystem-IME, komplexe Schriften, vollständige
Drag-and-drop-/Systemzwischenablage-Abläufe, Systemfonts und Drucken nötig. Reale Microsoft-Word-Roundtrips,
visuelle Word-Vergleiche und die vollständige Office-Funktionsabdeckung bleiben eigene
Ausbau- und Abnahmearbeiten.

Die [Tabellenetappe](TABLES.md) ergänzt Zellstories, Layout, direkte Seitenbedienung,
DOCX/RTF/HTML-Ausgabe und drei registrierte Tabellenaktionen in den gemeinsamen
Bibliotheken. Die Prüfungen und Builds liegen separat unter `build/wordcraft-tables`.
Die bisherigen Referenzbuilds und ihre historischen Prüfergebnisse bleiben erhalten.

Die [Fortsetzung mit verbundenen Zellen und Tabellenbreiten](MERGED_TABLES.md) erfolgt
ausschließlich in JFLib. Sie ergänzt gemeinsame Rasteroperationen, Seitenfortsetzungen,
DOCX-Verbindungen, passende RTF-/HTML-Ausgabe und Schriftmessung für die Breitenanpassung.
Quellenzitate bleiben bei Teilen und erneutem Verbinden strukturierte Felder.
Die damaligen Builds und Prüfprotokolle liegen unter `build/wordcraft-merged`;
ASMGUILib wird als Referenz gelesen und nicht weiter verändert.

Die [Etappe mit wiederholten Tabellenkopfzeilen und Tabellenformatvorlagen](TABLE_HEADERS_AND_STYLES.md)
ergänzt mehrere wiederholte Kopfzeilen, verbundene Kopfzellen, direkt bearbeitbare
Seiteninstanzen und dokumenteigene Vorlagen mit bedingten Bereichen. Modell, Umbruch,
Vorlagendialog und DOCX-/Zwischenablageausgabe liegen in den gemeinsamen Bibliotheken;
jfWord bindet diese über GUI und registrierte Agentenaktionen an.
Die zugehörigen nativen und WASM-Builds sowie Abnahmen liegen unter
`build/wordcraft-table-styles`. Diese Etappe ersetzt keine noch ausstehende Abnahme
der vollständigen Word-Kompatibilität.

Die [Fortsetzung mit vererbbaren Tabellenformatvorlagen](TABLE_STYLE_INHERITANCE.md)
ergänzt mehrstufige Basisbeziehungen, gruppierte Zeilen-/Spaltenbänder und vier
Eckzellbereiche. Zurücksetzen, explizite Überschreibungen und Änderungen an Vorfahren
wirken über gemeinsame Dokumenttransaktionen. Der native/WASM-Dialog und JSON/MCP
verwenden denselben Kern; DOCX erhält den Vorlagengraphen. Aktuelle Ausgaben:
`build/wordcraft-table-graphs`. Vollständige Word-Kompatibilität bleibt gesondert offen.


Die [Etappe für Zellrahmen und vollständige Tabellenfragmente](CELL_BORDERS_AND_TABLE_CLIPBOARD.md)
ergänzt vier unabhängige Rahmenwerte je Zelle, abschnittsweise Kantenauflösung bei
verbundenen Zellen und Quellen-/Vorlagenimport beim Kopieren ganzer Tabellen.
Dokumentkern, Layout, Zeichnung, Dialog und Codecs sind gemeinsame Bibliotheksfunktionen.
Die nativen und WASM-Builds sowie die zugehörigen Prüfungen liegen unter
`build/wordcraft-cell-borders`.
