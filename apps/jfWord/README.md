# jfWord – ausführbarer Entwicklungsstand

Aktuelle Etappe in **JFLib** (Arbeitsordner `GitHub/JFLib`): [Tabellenformeln und Zahlenformate](docs/TABLE_FORMULAS.md).
Live-Felder wie `=SUM(ABOVE)`, `=A2*B2` und `=IF(A2>0,B2/A2,0)` verwenden den
aktuellen Tabellenkontext. Der gemeinsame Dialog zeigt die Vorschau; Zelländerungen und
Undo aktualisieren abhängige Ergebnisse. DOCX und native Tabellenkopien erhalten die
Formeln. Gemeinsame Bibliotheken für Windows, Linux und WASM; Builds unter
`build/wordcraft-table-formulas`.
Die vorherigen [Zellnoten](docs/TABLE_CELL_NOTES.md), [exakten Zeilenhöhen](docs/EXACT_TABLE_ROW_HEIGHTS.md)
und [mehrseitigen schwebenden Tabellen](docs/FLOATING_TABLE_PAGES_AND_HTML.md) bleiben integriert.
Das frühere ASMGUILib-Repository wird ausschließlich als Referenz gelesen.

Die [Prüfung der Umbenennung zu jfgui](../../docs/JFGUI_RENAME.md) dokumentiert die
aktuellen CLI-/MCP-/WASM-Namen und die automatische Übernahme vorhandener Browserdateien.

Stand: 13. Oktober 2026. Der [Featurekatalog](FEATURES.md) bleibt das vollständige
Produktziel. Dieser Stand setzt einen ersten Teil von **P1 (Grundlagen)** und eine
erste Oberfläche um. Der aktuelle Ausbau umfasst Quellenverwaltung mit Sources.xml-, RIS-
und BibTeX-Austausch, Importprüfung, bearbeitbare Mehrfachzitate, Quellenzusammenführung
und Vollständigkeitshinweise, eine dauerhaft gespeicherte persönliche Quellenbibliothek,
bearbeitbare Fuß-/Endnoten einschließlich darin enthaltener Zitate,
Zeichen-/Absatzformatierung mit Schriftfamilien und vererbbaren Vorlagen, Suchen/Ersetzen sowie einen eigenen Seitenumbruch mit Vorschau.
P1 und die Word-Funktionsabdeckung sind noch nicht abgeschlossen.
Die erste strukturelle [Designüberarbeitung](DESIGN.md) ist umgesetzt: Start-/Dateiansicht,
eigene Vorlagen, kompaktes gruppiertes Menüband und direkte Rich-Text-Bearbeitung auf
weißen Papierseiten. Eine gemeinsame Auswahl reicht über Formatwechsel, Absätze, Zeilen und Seiten des
Haupttexts beziehungsweise einer Note; die Strukturansicht ist optional.
Formatierte Zwischenablage und Feldaktionen sind in die Papieransicht integriert:
Absätze, Quellenverweise und Notenkörper lassen sich zwischen jfWord-Dokumenten kopieren.
Fremde DOCX-Pakete lassen sich zusätzlich im Erhaltungsmodus öffnen und unverändert kopieren;
nachweisbar einzeln zuordenbare Textbereiche können ohne Modellkonvertierung geändert werden.

Der aktuelle gemeinsame Ausbau ergänzt eine modulare `src`-Struktur, einen eigenständig
implementierten blauen jfOffice-Style und explizit aktivierbare Agentensteuerung.
Die [aktuelle Designetappe](docs/VISUAL_REFERENCE.md) gleicht Farben, Typografie,
Symbole und Abstände an die WordCraft-Referenz an. Widgets und Renderer bleiben eigene
Library-Implementierungen. Die offen lizenzierten Inter-Fonts und abgeleiteten Symbolmotive
werden mit ihren Lizenzhinweisen ausgeliefert; fremder App-/egui-Code wird nicht eingebunden.
[Agenten- und Web-Unterbau](../../docs/AGENT_WEB.md)
baut auf denselben Bibliotheken auf. Vollständige Browserabnahme und vollständige
Word-Dateikompatibilität sind getrennte, noch offene Ausbauziele.
Alle fünf GUI-Apps sind als WASM gebaut; die gemeinsame Chromium-/Firefox-Matrix besteht
18 Fälle plus acht Speicherfehler-/Konkurrenzfälle. Native und Web-Builds
des gemeinsamen Unterbaus wurden ursprünglich unter `build/office-universal` geprüft.
Die aktuelle jfWord-Featureversion liegt in JFLib unter `build/wordcraft-table-formulas/{win,linux,web}`.
Der direkte PNG-Vergleich und die Korrekturen an Navigation, Vorlagen, Suche,
Formatvorlagengalerie und Zoom sind im [Designnachweis](docs/VISUAL_REFERENCE.md) beschrieben.
Der neueste Abgleich ergänzt die Inter-GPOS-Zeichenabstände, eine eigene Rastereinstellung
für UI-Schriften und gemeinsam geglättete Füllungen/Rahmen auf halben DIP-Positionen.
Direkte Schriftproben und Kantenvergleiche dokumentieren die verbleibenden Abweichungen.
Die neue [Schrift- und Vorlagenetappe](docs/FONTS_AND_STYLES.md) ergänzt auswählbare
Schriftfamilien, vererbbare benannte Absatzvorlagen und deren Bearbeitungsdialog.
Die folgende [Zeichenformatetappe](docs/CHARACTER_FORMATTING.md) ergänzt Hervorhebung,
Hoch-/Tiefstellung und die Anzeige gemeinsamer Formate gemischter Auswahlen.
Die [Lineal- und Tabulatorenetappe](docs/RULERS_AND_TABS.md) ergänzt ziehbare Einzüge
und Seitenränder, vererbbare individuelle Tabstopps, Füllzeichen und genaue Positionseingabe.
Die gemeinsame Implementierung ist nativ und im WASM-Build nutzbar.
Die [Zellrahmen- und Zwischenablageetappe](docs/CELL_BORDERS_AND_TABLE_CLIPBOARD.md) ergänzt
individuelle Zellrahmen und das Kopieren/Ausschneiden/Einfügen vollständiger Tabellen,
einschließlich verbundener Zellen, Quellenfeldern und vererbten Formatvorlagen.
Der [vorherige Tabellenausbau](docs/NESTED_TABLES_AND_IMPORT.md) ergänzt bis zu acht
Verschachtelungsebenen, innere Kopfzeilen auf Folgeseiten und RTF-/HTML-Import.
Die dort dokumentierten Importgrenzen sind kein Nachweis vollständiger Formatkompatibilität.
Die [Etappen- und Abnahmeübersicht](docs/IMPLEMENTATION.md) ordnet den vorhandenen Stand
dem gemeinsamen Plan zu und benennt die noch offenen Office- und Browserarbeiten.

## Starten und bauen

Vom Repository-Stamm:

```powershell
python -X utf8 -B build_rs.py app jfWord --target all --out build/wordcraft-table-formulas
```

Ergebnisse: `build/wordcraft-table-formulas/win/jfWord.exe`, `build/wordcraft-table-formulas/linux/jfWord`
und `build/wordcraft-table-formulas/web/jfWord/`.
Wenn eine geöffnete Instanz die EXE sperrt, kann parallel in ein separates Build-Verzeichnis
gebaut und dort geprüft werden; die geöffnete Anwendung muss dafür nicht beendet werden:

```powershell
python build_rs.py app jfWord --target both --out build/jfword-next
python apps/jfWord/tests/check.py --target both --build-dir build/jfword-next
```

Der Style `jfoffice` wird über `STYLE` gewählt. Es gelten weiterhin die Projektregeln:
`no_std`, `core`/`alloc`, keine Cargo- oder Fremd-Crate-Abhängigkeiten.

`both` bezeichnet weiterhin Windows und Linux. Die zusätzlichen Ziele `web` und `all`
gehören zum gemeinsamen Browserport; dessen Voraussetzungen und tatsächlicher Stand stehen
in [AGENT_WEB.md](../../docs/AGENT_WEB.md).

```powershell
python -X utf8 -B build_rs.py app jfWord --target web --out build/wordcraft-table-formulas
# Browserbundle lokal über HTTP bereitstellen, danach /jfWord/ öffnen:
python -m http.server 8765 --bind 127.0.0.1 --directory build/wordcraft-table-formulas/web
```

Der Server wird hier nur für die Bereitstellung gestartet. Die Web-App benötigt keinen
Backenddienst; Agentensteuerung bleibt auch im Browser bis zur ausdrücklichen Aktivierung aus.

```powershell
# Mit einem unterstützten Dokument starten:
python build_rs.py run jfWord --target win -- --file build/jfword-check/win/sample.docx
# Eigenen Style mit allen vorhandenen GUI-Controls ansehen:
python build_rs.py app gallery --style jfoffice --out build/jfoffice-check
```

## Quellstruktur

```text
jfWord/
  src/
    main.rs             # Einstieg und Modulverbindungen
    app/                # Zustand, Lebenszyklus, Dokumentabläufe, Befehle, Agentenadapter
    ui/                 # Fensterkopf, Dateioberfläche, Menüband, Papier- und Feldansicht
    workflows/          # Datei-, Quellen-, Bibliotheks-, Zitat- und Notendialoge
    checks/             # native Selbstprüfungen
  tests/                # Python- und UI-Prüfungen
  docs/                 # Etappen, tatsächliche Nachweise und verbleibende Ausbauziele
  context/              # externe Referenzen; keine App-Quellen
  books/                # lokale Referenzen; keine App-Quellen
  README.md
  FEATURES.md
  DESIGN.md
  STYLE
  TARGETS               # win, linux, web
```

Dokumentmodell, Dateicodecs, Seitendarstellung und Agentenprotokolle liegen in gemeinsamen
Bibliotheken. Der Build erfasst Module und eingebettete Assets ausgehend von `src/main.rs`;
Referenzprojekte unter `context` gehören weder zum App-Lint noch zu den App-Abhängigkeiten.
Die bestehenden flachen Projekte bleiben unterstützt.

## Agentensteuerung

Ohne ausdrückliche Aktivierung öffnet jfWord keinen Steuerungsendpunkt. `--agent` aktiviert
die lokale Verbindung einer laufenden Anwendung; der Fensterkopf zeigt die Aktivierung und
eine aktive Verbindung. `--agent-json` beziehungsweise `--agent-mcp` startet eine steuerbare
Headless-Sitzung über Standardstreams. Dafür läuft kein Modell und keine KI-Erweiterung in
jfWord. Das gemeinsame Werkzeug ist `tools/jfgui-agent.py`.

```powershell
# Eigene, ausdrücklich aktivierte Headless-Sitzung:
python tools/jfgui-agent.py headless build/win/jfWord.exe --protocol json
# Bereits aktivierte lokale Instanzen finden:
python tools/jfgui-agent.py list
```

Neben Widgets, Fokus, Tasteneingabe, bestehenden Befehlen und PNG-Ressourcen werden
Dokumentinspektion, Text-/Zeichenformatierung, Absatzvorlagen, Quellen, Zitate, Felder,
Noten, Zwischenablage und DOCX-Dateisitzung fachlich angeboten. Der aktuelle Katalog wird
über `actions/list` und `actions/describe` abgefragt. Ändernde Fachaktionen verlangen die
zuletzt gelesene `expectedRevision`; ein Konflikt verändert das Dokument nicht.
Die Adapter benutzen die vorhandenen Editor- und Dateisitzungsfunktionen. Sie erweitern
damit nicht automatisch das unterstützte DOCX-Profil. Atomare externe Stapel sind für
jfWord gegenwärtig nicht registriert und werden vor der ersten Änderung abgelehnt.

## Bereits benutzbar

- Start-/Dateiansicht mit eigenen Vorlagen für leere Dokumente, wissenschaftliche Arbeiten und
  Berichte; zuletzt verwendete/angeheftete Dateien mit Suche. Die Liste speichert das Datum des
  letzten Öffnens/Speicherns, kein behauptetes Dateisystem-Änderungsdatum.
- Integrierter Fensterkopf mit Schnellzugriff, Befehlssuche und Fenstersteuerung. Vollständige
  Registerstruktur: Datei, Start, Einfügen, Zeichnen, Entwurf, Layout, Referenzen, Sendungen,
  Überprüfen, Ansicht und Hilfe; noch nicht implementierte Funktionen sind sichtbar deaktiviert.
  Gruppiertes Menüband mit Formatgalerie und Überlauf bei geringer Breite. Gemeinsame
  große Aktionen zeigen Symbol und Beschriftung; die geteilte Einfügeaktion hat eine
  getrennt fokussierbare Optionsliste. Der Absatz-Dialogstarter besitzt einen eigenen
  semantischen Namen und ist mit Tastatur beziehungsweise Agenten erreichbar.
  Navigation und Quellenbereich sind standardmäßig ausgeblendet und direkt zuschaltbar.
- **Einfügeoptionen** bietet formatiertes Einfügen, Nur-Text-Einfügen und Zeichenformat-
  Übertragung. **Format aufnehmen** merkt die effektiven sieben vorhandenen Zeichenwerte einschließlich Schriftfamilie,
  **Format anwenden** ersetzt die Zielformatierung; **Formatierung löschen** übernimmt
  wieder die Vorgaben des Absatzstils. Text, Felder, Absatzstil und Zwischenablage bleiben
  erhalten; Änderungen sind rückgängig machbar. Tastenkürzel: `Strg+Umschalt+C`,
  `Strg+Umschalt+Alt+V`, `Strg+Leertaste`. `Strg+Umschalt+V` bleibt Nur-Text-Einfügen.
- Schriftfamilien lassen sich direkt auswählen: eingebettete DejaVu Sans/Serif/Mono auf
  allen Plattformen, zusätzlich unterstützte installierte Fonts nativ. Eigene Absatzvorlagen
  lassen sich unter **Start → Formatvorlagen** anlegen, ändern und vererben. Direkte
  Formatierung bleibt vorrangig; DOCX und die native Zwischenablage erhalten den Vorlagengraphen.
- Die Schriftgröße kann direkt von 4 bis 144 pt in halben Punkten eingegeben oder über
  die bisherigen Vorwahlen ausgewählt werden. Beide Eingaben benutzen dieselbe
  Zeichenformatierung und erhalten die gewählte UTF-8-Textauswahl.
- Textabsätze anlegen, auswählen, bearbeiten und entfernen; Text, Titel und zwei
  Überschriftenebenen zuweisen. Gemischte Formate, Ausrichtung, Tabulatoren und mehrseitige
  Absätze werden direkt auf dem gesetzten Papier bearbeitet, einschließlich Notentext.
- Enter teilt einen Absatz an der Schreibposition bzw. ersetzt die Auswahl durch eine
  Absatzgrenze. Umschalt+Enter fügt einen Zeilenwechsel ein. Rücktaste am Absatzanfang und
  Entf am Absatzende verbinden benachbarte Absätze, auch innerhalb derselben Note.
  Formate und strukturierte Verweise bleiben erhalten; Undo/Redo stellt Cursor und Auswahl wieder her.
- `Strg+C`/`Strg+X`/`Strg+V` übertragen formatierte Auswahlen über Absatzgrenzen und zwischen
  Dokumenten. Zeichenformate, Absatzwerte und harte/weiche Umbrüche bleiben erhalten.
  Vollständige Zitate nehmen ihre Quellen mit; identische Quellen werden wiederverwendet.
  Kopierte Fuß-/Endnoten erhalten eigene IDs samt formatiertem Inhalt und darin enthaltenen Zitaten.
  Einfügen und Ausschneiden sind jeweils eine Undo-Transaktion. Verschachtelte Noten und
  teilweise ausgeschnittene Felder werden ohne Dokumentänderung abgelehnt.
- Die Zwischenablage bietet gleichzeitig ein versioniertes natives Fragment, RTF, HTML und
  Klartext an. `Strg+Umschalt+V` fügt ausdrücklich nur Text ein. Ein teilweise markiertes Feld
  wird beim Kopieren als Klartext übertragen und entsprechend gemeldet.
- Eine Auswahl oder Schreibposition in einem Feld zeigt kontextbezogene Aktionen über der Seite:
  Zitat bearbeiten, Quelle zeigen, Anmerkung öffnen oder Feld entfernen. `F11`/`Umschalt+F11`
  wechseln zwischen Feldern des aktuellen Haupttext-/Notenbereichs. Das Menüband „Referenzen“
  enthält dieselben Navigationsaktionen. Auch leere Literaturverzeichnisse sind erreichbar.
- Markierten Text über Format- und Absatzgrenzen hinweg fett, kursiv, unterstrichen oder durchgestrichen
  formatieren, Schriftgröße und direkte Schriftfarbe ändern, hervorheben sowie hoch-/tiefstellen. Ohne Markierung gilt die Änderung
  im Seiteneditor für nachfolgend eingefügten Text. Die optionale Strukturansicht behält ihre
  bisherige Formatierung des gesamten aktiven Segments ohne Auswahl.
  Tastenkürzel: `Strg+B`, `Strg+I`, `Strg+U`. Formate sind in der Seitenvorschau sichtbar.
- Absätze linksbündig, zentriert, rechtsbündig oder im Blocksatz darstellen; Abstände davor/
  danach, linken/rechten sowie Erstzeilen-/hängenden Einzug und einfachen bis doppelten
  Zeilenabstand einstellen. Absatzbefehle und Vorlagen gelten für alle von einer Auswahl
  berührten Absätze, mit einer gemeinsamen Undo-Transaktion.
- A4/US Letter, Hoch-/Querformat und individuelle Seitenränder für das gesamte Dokument;
  manuellen Seitenwechsel vor einem Absatz ein-/ausschalten.
- Papieransicht und Seitenvorschau mit automatischem Zeilen-/Seitenumbruch und Zoom von
  10 bis 500 % sowie Seitenbreite in der Statusleiste. Schieberegler, Plus/Minus und
  Vorwahl teilen denselben Zoomzustand; unter **Ansicht** ist eine direkte Prozentangabe
  möglich. Klick auf eine Textzeile führt zum Absatz.
- Navigation bietet Überschriften mit Ebeneneinzug, echte Seitenvorschaubilder und
  Suchergebnisse. Ein Seitensprung setzt die Schreibposition an die erste gesetzte Zeile
  der gewählten Seite, auch bei einem Absatz über mehrere Seiten. Suchen umfasst
  formatübergreifenden Text und Fuß-/Endnoten, mit Groß-/Kleinschreibung und ganzen Wörtern;
  erzeugte Felder bleiben ausgeschlossen. Treffer werden direkt im Text markiert.
  Große Listen sind paginiert. Fehlen Überschriften, bleiben die Absätze direkt erreichbar.
- **Fokus** in der Statusleiste oder `Strg+Umschalt+F11` blendet Fensterkopf, Menüband und
  Seitenbereiche vorübergehend aus. **Fokus beenden** beziehungsweise `Esc` stellt sie wieder
  her; die gewählten Seitenbereiche bleiben erhalten. Zoom und Schreibposition bleiben bedienbar.
- Fuß- und Endnoten an einer Textposition einfügen, ihre Absätze bearbeiten und formatieren,
  zum Haupttextverweis zurückkehren sowie zwischen beiden Notenarten umwandeln. Entfernen
  löscht nach Bestätigung Verweis und Noteninhalt gemeinsam; Undo stellt beides wieder her.
- Beide Notenarten werden unabhängig nach ihrer Verweisreihenfolge fortlaufend nummeriert.
  Die Vorschau setzt Fußnoten mit Trennlinie an den Seitenfuß, setzt lange Noten auf
  Folgeseiten fort und führt Endnoten hinter dem Haupttext auf.
- Literales Suchen mit `Strg+F`/`Strg+H`, Groß-/Kleinschreibung und ganzen Wörtern;
  nächster Treffer mit Markierung, alle Treffer als einen Undo-Schritt ersetzen.
  Die Suche überwindet Formatgrenzen, verändert aber keine erzeugten Zitatfelder.
- Dokumentänderungen einschließlich Quellen, Verweisen und Absatzstruktur rückgängig
  machen und wiederholen. Die Anzeige ungespeicherter Änderungen folgt dem gespeicherten
  Dokumentzustand, auch über Undo/Redo hinweg.
- Bücher, Zeitschriftenartikel und Webseiten anlegen, ändern und entfernen. Geordnete
  Autorenlisten mit Vor-, weiteren Vor- und Nachnamen oder eine Körperschaft erfassen;
  Personen hinzufügen, entfernen und umordnen. Titel ist Pflicht. Quellen mit
  Zitatverweisen lassen sich nicht versehentlich löschen.
- Publikationsangaben einschließlich Zeitschrift, Band, Heft, Seiten, Verlagsort,
  Auflage, Website, URL/DOI-Link, ISBN/ISSN, Zugriffsdatum, Kurztitel und Notizen erhalten.
  Der Quelldialog trennt Publikation, Autoren und scrollbare Details.
- Unterstützte Quellenlisten im `Sources.xml`-Format, RIS- und BibTeX-Dateien über den
  Quellenbereich importieren/exportieren. Eine Vorschau zeigt Einträge, Auswahl und
  Dublettenhinweise; Abbrechen verändert das Dokument nicht. Identische Inhalte werden
  übersprungen. Der gewählte Stapel bildet einen Undo-Schritt.
- Dokumentquellen nach Titel, Personen oder Metadaten filtern. Die Auswahl verweist auf
  die richtige Quelle auch bei gleichen Titeln und nach Undo-bedingten Indexänderungen.
  Exportieren schreibt alle Dokumentquellen, unabhängig vom Suchfilter, und ersetzt
  keinen Dokument-Speichervorgang.
- Persönliche Quellenbibliothek über „Referenzen / Quellenbibliothek“ oder den Quellenbereich
  öffnen; Quellen suchen, erstellen, bearbeiten und entfernen. Änderungen einschließlich
  Rückgängig/Wiederholen werden unmittelbar gespeichert. Die Bibliothek bleibt bei „Neu“
  und „Öffnen“ erhalten; ihre Historie ist von der Dokumenthistorie getrennt.
- Quellen in beide Richtungen zwischen Dokument und Bibliothek kopieren. Identische Angaben
  werden beim Import dedupliziert; IDs werden unabhängig zugeteilt. Dokumente enthalten
  weiterhin alle benötigten Quelldaten und benötigen die persönliche Bibliothek nicht.
- Zwei ausgewählte Quellen feldweise abgleichen, mit Richtungsauswahl und Ergebnisvorschau.
  Vorauswahl ergänzt leere Zielfelder. Belegte Angaben ändern sich nur nach expliziter Auswahl;
  die Zielquelle behält ihre ID und alle Zitatverweise. Andere Dokumente ändern sich nicht.
- Letzte Bibliothekssicherung prüfen und ausdrücklich wiederherstellen. Eine inzwischen
  geänderte Bibliothek oder Sicherung wird erkannt und nicht mit einer veralteten Vorschau ersetzt.
- Zitate an der Schreibposition eines Textsegments einfügen; ohne gewählte Textposition
  an den Absatz anhängen. Text davor und danach behält seine Formatierung.
- Bestehende Zitate bearbeiten: bis zu 32 geordnete Quellen pro Zitatfeld, einzelne
  Fundstellen, Präfix/Suffix und Ausblenden von Autor, Jahr oder anonymem Titelfallback.
  Übernehmen bildet einen Undo-Schritt; Abbrechen verändert das Dokument nicht.
- Verwendungen einer Quelle auflisten und zum zugehörigen Zitat springen. Quellenänderungen
  aktualisieren die Feldanzeige und das Literaturverzeichnis unmittelbar. Zitate in
  Fuß-/Endnoten nehmen an Quellenverwendung, Löschschutz und Zusammenführung teil.
- Zwei Dokumentquellen feldweise vergleichen und bewusst zusammenführen: Angaben je Feld
  wählen, Ergebnis prüfen und sämtliche Zitatbezüge auf die verbleibende ID umhängen.
  Die gesamte Änderung ist ein Undo-Schritt; Abbrechen erhält beide Quellen unverändert.
- Quellen auf fehlende Autoren/Körperschaft, Jahr und typabhängige Angaben prüfen;
  Hinweise nach zitierten Quellen filtern und direkt zur betroffenen Quelle wechseln.
  Diese Prüfung verändert weder Quelldaten noch den Gespeichert-Status.
- Literaturverzeichnis als dynamisches Feld einfügen; zitierte Quellen erscheinen einmal
  in der Reihenfolge ihrer ersten Verwendung. Zusätzlich gibt es eine Live-Vorschau.
- DOCX erstellen, speichern und im unten beschriebenen Profil wieder öffnen. Die Datei
  enthält native Word-Bibliografiedaten, `CITATION`- und `BIBLIOGRAPHY`-Felder.
- Speichern unter, Nachfrage bei ungespeicherten Änderungen und beim Ersetzen einer
  anderen bestehenden Datei. Ein fehlgeschlagenes Öffnen erhält das aktuelle Dokument;
  fehlgeschlagenes Speichern setzt weder Dateiname noch Gespeichert-Markierung um.
- Heller und dunkler jfOffice-Style mit blauer Palette, Inter als UI-Schrift und eigenen
  geometrischen Symbolen. Alle Style-Zeichenfunktionen liegen lokal in `styles/jfoffice`;
  die gemeinsame UI-Library liefert Interaktion, Layout und Fenstersteuerung.
- Dokumentfonts, Seitenzeichnung, Koordinaten und echte Dokument-Vorschaubilder kommen aus
  `lib/doc_view/src`. Vorlagen zeigen die erste Seite ihres tatsächlichen Dokumentmodells.
  Ein Wechsel des Anwendungsstyles verändert weder Dokumentfarben noch Zeilenumbruch.

Der Office-Ausbau ist noch nicht vollständig: Vollständiger Glyphenfallback, komplexes
Shaping, interaktive Lineale und individuelle Tabstopps
sind weitere Etappen. Schriftfamilien sowie benannte/vererbbare Absatzvorlagen mit eigenen
Definitionen und Vorschauen sind vorhanden; Zeichen- und Tabellenformatvorlagen folgen später.
Die Papierbearbeitung zeigt Seiten untereinander; die Vorschau zeigt eine gewählte Seite.
Eine Ansicht mehrerer Seiten nebeneinander ist noch nicht umgesetzt. Sichtbar deaktivierte
Office-Aktionen geben keinen vorhandenen Funktionsumfang vor.

Die Zitatausgabe ist ein ausdrücklich einfacher **Autor/Jahr-Basisstil**. Sie ist kein
APA-, Harvard- oder CSL-Konformitätsversprechen. Wortzählung bedeutet derzeit durch
Whitespace getrennte Wörter einschließlich dargestellter Felder.

## Bibliotheksaufteilung

| Ort | Verantwortung |
|---|---|
| `lib/base/encoding.rs` | Striktes UTF-8, BOM, UTF-16 LE/BE; fehlerhafte Sequenzen werden abgelehnt |
| `lib/base/xml.rs` | Begrenzter XML-Teilparser mit aufgelösten Namensräumen und sicherem Escaping |
| `lib/archive` | ZIP32 lesen/schreiben, gespeicherte und DEFLATE-komprimierte Einträge lesen, CRC, Daten-Deskriptoren und Ressourcenlimits |
| `lib/doc_core` | Stabile Dokument-/Quellen-/Noten-IDs, Absätze, Zitat-/Literaturfelder, Validierung, atomare Änderungen, Undo/Redo |
| `lib/doc_core/notes.rs` | Einmalige Haupttextanker, mehrabsätzige Noten, Navigation, Nummerierung und atomare Notenänderungen |
| `lib/doc_core/story_range.rs` | Gemeinsame UTF-8-Koordinaten eines Haupttexts oder einer Note, atomare Auswahlbearbeitung und Formatierung über Absatzgrenzen |
| `lib/doc_core/paragraph_edit.rs` | Atomare Absatzteilung/-verbindung mit UTF-8-Positionen und Erhaltung strukturierter Verweise |
| `lib/doc_core/source.rs` | Quellentypen, Personen/Körperschaften, Metadaten, Größenlimits und Autor/Jahr-Basisdarstellung |
| `lib/doc_core/format.rs`, `search.rs` | Vererbte Zeichenformate, typografische Absatz-/Seiteneinstellungen, Unicode-Textsuche und atomare Ersetzungen |
| `lib/doc_core/source_import.rs` | Quellenvergleich, Importklassifikation und Unicode-Filterung ohne UI-Abhängigkeit |
| `lib/doc_core/source_merge.rs` | Dublettenkandidaten, feldweise Vergleichsentwürfe und atomare Zusammenführung mit Umhängen aller Zitate |
| `lib/doc_core/source_audit.rs` | Nicht verändernde Vollständigkeitshinweise mit stabilen Quellen-IDs |
| `lib/doc_core/source_library.rs` | Dokumentunabhängige Quellensammlung, monotoner ID-Zähler, atomare Importe und eigene begrenzte Historie |
| `lib/doc_core/citation.rs` | Strukturierte Mehrfachzitate, Darstellung, Verwendungsstellen und begrenzte Textexpansion |
| `lib/doc_layout` | Zeilen- und Seitenumbruch in Twips mit austauschbarer Fontmessung, unabhängig von UI, Zoom und DPI |
| `lib/doc_layout/notes.rs` | Reservierter Fußnotenraum, Trennlinien, begrenzte Notenfortsetzung und Endnoten nach dem Haupttext |
| `lib/doc_view/src` | Style-unabhängige Dokumentfonts, gemeinsame Seitendarstellung, A4-Vorschaubilder und Koordinatenumrechnung |
| `lib/doc_core/text_range.rs` | UTF-8-Absatzprojektion mit geschützten Feldern, atomare Ersetzung/Formatierung über Textsegmente |
| `lib/doc_io` | OPC/DOCX-Profil, Feldcodes, begrenztes Dateilesen und Speichern über `plat` |
| `lib/doc_io/preservation.rs`, `preserved_text.rs`, `preserved_stories.rs` | OPC-Inspektion, unveränderte Originalkopie und gezielte Änderungen zugeordneter XML-Textbereiche mehrerer Dokumentteile mit gemeinsamer Historie |
| `lib/doc_io/bibliography.rs` | Gemeinsamer Word-Bibliografie-XML-Codec für DOCX und separate Quellenlisten |
| `lib/doc_io/ris.rs`, `bibtex.rs`, `exchange.rs` | Begrenzte RIS-/BibTeX-Profile, Formatwahl und vollständige Prüfung vor Dateiänderungen |
| `lib/doc_io/format.rs` | Word-Zeichen-/Absatz-/Seiteneigenschaften lesen und schreiben |
| `lib/doc_io/citations.rs` | Strikte Word-CITATION-Feldcodes mit mehreren Quellen und quellbezogenen Schaltern |
| `lib/doc_io/notes.rs` | Standard-Notenparts, Nummerierungsprofil, Notenzeichen und Abbildung fremder Notenkennungen |
| `lib/doc_io/library.rs` | Eigenes Bibliotheksformat, geprüfte Speicherung, Konflikterkennung, Sicherung und Wiederherstellung |
| `lib/plat/{win,linux}_sys.rs` | Wiederverwendbare nicht blockierende Dateisperre und Dateiprüfung mit unterscheidbaren Fehlerfällen |
| `lib/agent_core/src`, `lib/agent_io/src` | Gemeinsame Aktionsverträge, Schemas, JSON/MCP, Aufgaben und aktivierbare native Transporte |
| `styles/jfoffice` | Eigene blaue Farben, Typografie, Fonts, Symbole und lokale Control-Zeichenfunktionen |
| `lib/ui/ribbon.rs`, `lib/ui/edit.rs`, `lib/ui/text_surface.rs` | Adaptive Menübandgruppen, Formatgalerie und gemeinsame Texteingabe über extern gesetzten Zeilen mit Cursor, Auswahl und IME-Anbindung |
| `apps/jfWord/src` | Fenster, Bedienaktionen, Dialoge und Zusammenführung der Bibliotheksdienste |

Die Navigationsansicht verändert keine Dokumenteigenschaften. Die Statusleiste bezeichnet
mit „Seite“ die Seite der aktuellen Schreibposition beziehungsweise die gewählte Vorschauseite.
Fokus und Zoom sind reine Ansichtszustände und bilden keine zusätzlichen Undo-Schritte.

Kompression und CRC werden aus den vorhandenen `img_core`-Primitiven wiederverwendet.
Ein späterer gemeinsamer Kompressionsbaustein kann diese Abhängigkeit ablösen; es gibt
keine zweite DEFLATE-Implementierung in der App.

## Dokument bearbeiten und ansehen

Im Textfeld einen Bereich markieren und unter **Start** Fett/Kursiv/Unterstrichen oder
eine Schriftgröße wählen. Dabei entstehen getrennte Textsegmente mit eigenen Formaten;
Präfix und Suffix behalten ihre bisherigen Eigenschaften. Ein leerer formatierter
Textabschnitt behält sein Format auch beim anschließenden Schreiben. Die vier
Absatzvorlagen liefern die Grundwerte, Zeichenformate überschreiben sie ausdrücklich.
Der Standardeditor verwendet `Ui::textbox_surface` über den Fragmenten von `doc_layout`.
Cursor, Mausauswahl, Zwischenablage und vertikale Navigation folgen denselben gesetzten Zeilen
wie die Ausgabe. Der Haupttext beziehungsweise jede einzelne Note hat auch bei Schriftwechseln,
Blocksatz, Tabulatoren und mehreren Seiten nur eine durchgängige Auswahl. Umschalt+Pfeile,
Mausauswahl und Strg+A umfassen mehrere Absätze; Strg+Pos1/Ende springen im aktuellen Bereich.
Schriftfarbe und Durchstreichen sind auf der Seite sichtbar.
Die gesamte Papierfläche bleibt auch im dunklen Anwendungsthema weiß.

`Ansicht → Strukturansicht` beziehungsweise `--structure` öffnet weiterhin die Segment-/
Feldbedienung. Zitate, Anmerkungszeichen und Verzeichnisse erscheinen im Seiteneditor als
geschützte Feldtexte: Freie Texteingabe und Zeichenformatierungen über Feldtexte werden
atomar abgewiesen. Vollständig ausgewählte Felder lassen sich dagegen strukturiert kopieren,
ausschneiden und durch ein Dokumentfragment ersetzen. F11/Umschalt+F11 wählt das nächste/
vorherige Feld; die Kontextleiste öffnet dessen Bearbeitung direkt auf der Seite.
Die Strukturansicht sowie Quellen- und Anmerkungsnavigation bleiben weitere Zugänge.
Die Zwischenablage überträgt zwischen jfWord-Dokumenten Zeichen-/Absatzformate, Quellen
und Notenkörper; andere Programme erhalten zusätzlich RTF, HTML und Klartext.
Strg+Umschalt+V fügt ausdrücklich Klartext ein. Mehrzeiliger Klartext erzeugt echte Absätze;
LF, CRLF und CR werden normalisiert. Der gemeinsame [RTF-/HTML-Import](docs/NESTED_TABLES_AND_IMPORT.md)
erkennt außerdem Tabellen und deren verschachtelte Zellinhalte.

Auch absatzübergreifendes Ersetzen, Ausschneiden, Formatieren und Einfügen bildet jeweils eine
atomare Transaktion. Nicht ausgewählte Textläufe und Verweise behalten ihre Struktur. Haupttext
und unterschiedliche Noten sind getrennte Bearbeitungsbereiche. Ein Auswahlende genau am Anfang
des nächsten Absatzes bezieht dessen Absatzformat nicht mit ein.

**Enter** erzeugt eine Absatzgrenze; **Umschalt+Enter** bleibt ein Zeilenwechsel im Absatz.
Am Ende einer Überschrift beginnt der neue Absatz mit der Vorlage Text, innerhalb einer
Überschrift setzt er deren Vorlage fort. Ein Seitenwechsel vor dem Absatz wird nicht auf
beide Hälften übertragen. Beim Verbinden gewinnt die Absatzvorlage des ersten Absatzes;
explizite Zeichenformate bleiben bestehen. Notenabsätze werden ausschließlich innerhalb
derselben Note verbunden. Eine markierte Auswahl kann mehrere Absätze und Textsegmente
umfassen. Aus der Zwischenablage eingefügte Zeilenwechsel erzeugen neue Absätze. **Strg+Enter** behält den bisherigen Befehl „Absatz hinzufügen“.

**Start** bietet Ausrichtung und **Absatz**, **Layout** Seitenwechsel und **Seite einrichten**.
**Start → Absatz**
öffnet Abstände und Einzüge. Ungültige Ränder/Einzüge lassen das bisherige Dokument
unverändert. **Ansicht → Quellenbereich** blendet die Quellenleiste ein oder aus.
Die Vorschau zeigt jeweils eine Seite mit Vor/Zurück-Navigation und Scrollleisten.
Zoom und Bildschirm-Skalierung verändern keine Umbruchentscheidung und keine Quelldaten.
Es handelt sich um den jfWord-Seitenumbruch, nicht um eine Word-identische Satzberechnung.

Die Suche behandelt benachbarte Textsegmente als gemeinsamen Text. Absatz- und Feldgrenzen
werden nicht übersprungen. Groß-/Kleinschreibung nutzt Unicode-Kleinschreibung pro Zeichen,
keine sprachspezifischen Gleichsetzungen wie `ß` und `ss`; Wortgrenzen orientieren sich an
Buchstaben, Zahlen und Unterstrich. Bei einem Treffer über mehrere Formatsegmente markiert
der Seiteneditor den gesamten Treffer; die optionale Strukturansicht markiert den Anteil
im ersten Segment. **Alle ersetzen** verarbeitet den gesamten Treffer.
Ersatztext übernimmt das Format am Trefferanfang. Regex und Suche in
Quellenmetadaten sind noch nicht Teil dieses Befehls.

Der Layoutkern verarbeitet explizite Zeilenwechsel, feste Tabstopps alle 36 pt,
Absatzabstände und Einzüge, Blocksatz sowie Überschriften mit der ersten Folgezeile,
sofern beide auf eine Seite passen. Lange Wörter werden erforderlichenfalls an
Unicode-Zeichengrenzen getrennt. Grenzen: 2.048 Seiten, 100.000 Textfragmente und
4 MiB expandierter Text; unpassende Einzelglyphen oder übergroße Zeilen werden gemeldet.
Ein Fontmessungsadapter verbindet ihn mit der vorhandenen Textbibliothek. Kursiv wird
gegenwärtig aus den eingebetteten regulären/fetten Fonts synthetisch abgeleitet.

## Fuß- und Endnoten

Im Haupttext eine Textposition setzen und **Referenzen → Fußnote einfügen** oder
**Endnote einfügen** wählen. Ohne aktive Textposition wird das Ende des letzten
bearbeitbaren Textsegments im gewählten Absatz verwendet. Die neue Note öffnet sich im
Absatzeditor. **Absatz hinzufügen**, Zeichen-/Absatzformatierung und die vorhandenen
Zitatbefehle funktionieren dort ebenfalls. Enter erzeugt neue Notenabsätze;
Umschalt+Enter bleibt ein Zeilenwechsel innerhalb des Notenabsatzes.

**Zur Textstelle** führt zum Verweis zurück. Die Verweisschaltfläche im Haupttext und
der Bereich **Anmerkungen** in der Navigation öffnen den zugehörigen Notentext;
ein Klick auf eine Notenzeile in der Vorschau wählt ihren Absatz aus. Suche und Ersetzen
beziehen Notenabsätze ein. In der Notenbearbeitung stehen **In Endnote umwandeln** bzw.
**In Fußnote umwandeln** und **Entfernen …** bereit. Umwandeln erhält Inhalt, Formate,
IDs und Zitate. Entfernen zeigt den Inhalt zur Bestätigung; Quelleinträge bleiben erhalten.
Diese Aktionen gehören zur gemeinsamen Dokumenthistorie und werden mit DOCX gespeichert.

Fußnoten und Endnoten zählen jeweils dezimal ab 1, fortlaufend für das ganze Dokument.
Die Reihenfolge der Haupttextverweise bestimmt die Nummern. Die aktuelle Oberfläche und
Vorschau unterscheiden ihre Textmarken als `[1]` und `[E1]`; DOCX schreibt automatische,
hochgestellte Notenzeichen mit jeweils eigener dezimaler Folge. Eigene Startwerte,
Seiten-/Abschnittsneustarts, römische Zahlen und benutzerdefinierte Symbole sind noch offen.
Jede Note hat genau einen Haupttextverweis. Verschachtelte Noten und ein
Literaturverzeichnis innerhalb einer Note werden nicht unterstützt.

Ein Zitat in einer Note ist dasselbe strukturierte Zitatfeld wie im Haupttext.
**Verwendungen anzeigen** nennt Notenart, Nummer und Absatz und springt zum Feld.
Auch ausschließlich in Noten zitierte Quellen erscheinen im dynamischen Literaturverzeichnis;
dessen erste Verwendung wird am Haupttextanker der jeweiligen Note berücksichtigt.
Quellenkorrekturen, Zusammenführungen und der Schutz vor dem Löschen verwendeter Quellen
wirken über Haupttext und Noten hinweg. Die Ausgabe bleibt der einfache Autor/Jahr-Basisstil;
automatische wissenschaftliche Fußnotenstile mit Erst-/Folgezitat oder CSL fehlen weiterhin.

Der eigene Layoutkern reserviert Fußnotenraum vor dem Setzen der zugehörigen Haupttextzeile.
Lange Fußnoten können auf Folgeseiten weiterlaufen; Endnoten stehen mit eigener Überschrift
nach dem Haupttext. Fortsetzungen dürfen derzeit eigene Seiten beanspruchen, bevor weiterer
Haupttext folgt. Die Trennlinie ist fest vorgegeben, einschließlich auf Fortsetzungsseiten.
Bei mehreren Fußnotenverweisen in derselben Haupttextzeile müssen die vorhergehenden Noten
dieser Gruppe und die erste Zeile der letzten Note zusammen mit der Verweiszeile auf eine
Seite passen. Zu große Gruppen oder eine zu große Kombination aus Verweis- und erster
Notenzeile melden eine Layoutgrenze. Inhalte werden dabei nicht weggelassen.
Dies ist ein begrenzter jfWord-Umbruch; eine Word-identische Verteilung und frei gestaltbare
Noten-/Fortsetzungstrennlinien sind noch nicht umgesetzt.

## Quellenlisten und wissenschaftlicher Arbeitsablauf

1. Im Quellenbereich **Importieren** wählen und eine unterstützte `.xml`, `.ris`, `.bib`
   oder `.bibtex` öffnen, oder **Quelle anlegen** verwenden. In **Quellenimport prüfen**
   einen Eintrag auswählen, Angaben vergleichen und **Diesen Eintrag übernehmen** setzen.
   Erst **Auswahl importieren** übernimmt den Stapel ins Dokument.
2. In **Publikation** Typ, Titel, Jahr und Verlag/Zeitschrift/Website erfassen. Unter
   **Autoren** zwischen Personen und Körperschaft wählen; die Reihenfolge steuert die
   Zitatdarstellung. Unter **Details** stehen die übrigen Publikationsangaben.
3. Im Text die Einfügestelle setzen, die Quelle wählen und **Im Absatz zitieren** anklicken.
   Eine Textmarkierung wird dabei nicht gelöscht; eingefügt wird am aktiven Cursorende.
   **Zitat bearbeiten** öffnet einen Entwurf mit Quellenreihenfolge und Einzelangaben.
   Präfix und Suffix sind wörtlich; gewünschte Abstände deshalb mit eingeben.
   Änderungen an Quelldaten aktualisieren Zitate und Literaturverzeichnis sofort.
4. Das Dokument als DOCX speichern; **Exportformat** und **Exportieren** schreiben separat
   alle Dokumentquellen, auch unzitierte. Die Dateiendung bestimmt den tatsächlichen Codec.
   Sources.xml unterstützt alle gegenwärtigen Modellfelder; RIS und BibTeX haben engere Profile.
5. **Verwendungen anzeigen** listet Zitatfelder der gewählten Quelle in Dokumentreihenfolge.
   Ein Klick führt zum Absatz und setzt den Tastaturfokus auf **Zitat bearbeiten**.
   Mehrere Nennungen derselben Quelle innerhalb eines Feldes ergeben einen Listeneintrag.

Die Zitatvorschau verbindet mehrere Nachweise im Basisstil mit Semikolon. Autorunterdrückung
betrifft Personen/Körperschaften; bei anonymen Quellen steuert die Titelunterdrückung den
Titelfallback. Fundstellen werden mit „S.“ ausgegeben. Es gibt noch keine frei wählbaren
Locator-Typen, automatische Jahreszusätze oder einen vollständigen wissenschaftlichen Zitierstil.
Je Fundstelle/Präfix/Suffix gelten maximal 128 UTF-8-Bytes; Anführungszeichen, Backslashes
und Zeilenumbrüche werden im aktuellen Feldcodeprofil abgelehnt.

Der XML-Codec versteht `Book`, `JournalArticle` und `InternetSite`, geordnete `Person`-
Einträge (`First`, `Middle`, `Last`) oder `Corporate`. Unterstützte skalare Felder:
`Title`, `Year`, `Publisher`, `City`, `Edition`, `JournalName`, `Volume`, `Issue`, `Pages`,
`InternetSiteTitle`, `URL`, `StandardNumber`, `YearAccessed`, `MonthAccessed`, `DayAccessed`,
`ShortTitle`, `Comments`, `Guid`, `LCID`. GUID und Sprachkennung werden erhalten, aktuell
aber nicht im Dialog bearbeitet und nicht zur Formatierung herangezogen. DOI-Adressen
werden als URL gespeichert; es gibt noch keinen DOI-Metadatenabruf.

Externe Tags werden beim Import auf neue dokumentlokale IDs abgebildet; der Export
schreibt `jf<ID>`-Tags. `RefOrder` sowie die Listenattribute `SelectedStyle`, `StyleName`
und `URI` werden nicht übernommen, da Reihenfolge und Zitierstil zum Ursprungsdokument
gehören. Es werden keine Stylesheets oder URLs geladen. Die Quelldaten bleiben erhalten;
der Export ist keine bytegleiche Kopie einer fremden Quellenliste.

Ein identischer Eintrag bedeutet Gleichheit aller modellierten Angaben einschließlich
Autorenreihenfolge, GUID und Sprachkennung, unabhängig von seiner eingehenden ID.
Geänderte Einträge werden zusätzlich aufgenommen; vorhandene Quellen und Zitate werden
nicht anhand ähnlicher Titel, Namen oder GUIDs überschrieben. Nicht unterstützte
Quellentypen, Rollen (z. B. Herausgeber), Felder, doppelte Tags/Felder oder beschädigte
XML-Dateien brechen den vollständigen Import ab. Die Importvorschau kennzeichnet identische
Angaben im Dokument und im eingehenden Stapel und wählt sie zunächst ab. Gleicher Typ,
normalisierter Titel und Jahr oder eine identische URL ergeben einen Hinweis auf eine
mögliche Dublette. Solche Einträge werden mit Vergleichsanzeige angeboten und bleiben
getrennt. Eine anschließende manuelle Zusammenführung ist möglich. Die persönliche
Quellenbibliothek übernimmt den dokumentübergreifenden Bestand; ein Abgleich erfolgt ausdrücklich.
Abbrechen und globale Tastenkürzel während der Vorschau verändern das Dokument nicht.

### Quellen vergleichen und zusammenführen

Im Quellenbereich die **verbleibende Quelle** wählen, dann **Zusammenführen** anklicken;
alternativ **Referenzen → Quellen zusammenführen**. Der Dialog bietet passende Kandidaten
an. **Nur mögliche Dubletten anzeigen** ausschalten, um eine beliebige andere
Dokumentquelle zu vergleichen. Gleicher Typ/Titel/Jahr oder eine identische URL sind
Hinweise, keine Feststellung, dass beide Einträge dasselbe Werk beschreiben.

Der Entwurf ergänzt tatsächlich leere Felder aus der zusätzlichen Quelle. Bereits belegte
Angaben bleiben zunächst erhalten; Leerzeichen gelten dabei als eingegebene Daten.
Im **Feldvergleich** eines von 21 Feldern wählen und festlegen, welche Seite gelten soll.
Autorenlisten und Körperschaft werden als eine gemeinsame Angabe übernommen, damit die
beiden Autorenmodi nicht vermischt werden. Die Auswahl eines leeren Werts löscht die
entsprechende Angabe ausdrücklich. **Ergebnis** zeigt sämtliche resultierenden Felder,
einschließlich Notizen, externer Kennung und Sprachkennung.

Erst **Zusammenführen** ersetzt die Angaben der verbleibenden Quelle und entfernt die
zusätzliche Quelle. Alle Einzel- und Mehrfachzitate wechseln auf die verbleibende ID;
Reihenfolge, Fundstellen, Präfix/Suffix und Ausblendungsoptionen bleiben erhalten.
Mehrfache Nennungen derselben Quelle innerhalb eines Clusters bleiben bestehen, weil sie
verschiedene Fundstellen haben können. Das Literaturverzeichnis führt die Quelle nur einmal.
Ein Undo stellt beide Quellen und ihre ursprünglichen Verknüpfungen vollständig wieder her.
Ungültige Ergebnisse oder überschrittene Text-/Dokumentgrenzen verändern das Dokument nicht.

### Fehlende Quellenangaben prüfen

**Prüfen** im Quellenbereich bzw. **Referenzen → Quellen prüfen** öffnet die Hinweise.
Erfasst werden fehlende Autoren/Körperschaft und Erscheinungsjahre, fehlender Verlag bei
Büchern, fehlende Zeitschrift bei Artikeln und fehlende URL bei Webseiten. Ein Zugriffsmonat
ohne Jahr bzw. ein Zugriffstag ohne Jahr oder Monat wird ebenfalls gemeldet. Jahr allein
und Jahr/Monat bleiben zulässige Teilangaben; Kalendergültigkeit wird noch nicht geprüft.

Der Bericht berücksichtigt zunächst auch unzitierte Quellen; **Nur zitierte Quellen prüfen**
grenzt ihn ein. **Quelle bearbeiten** öffnet den gewählten Eintrag; ein erneuter Prüflauf
verwendet die geänderten Angaben. Die Prüfung selbst ändert keine Daten und erstellt keinen
Undo-Schritt. Anonyme oder undatierte Werke bleiben speicherbar. Die Hinweise sind keine
APA-/CSL-/Hochschul-Konformitätsprüfung und prüfen keine Kennungen oder Online-Erreichbarkeit.

| Katalogbezug | Nachgewiesener Teilumfang | Noch offen |
|---|---|---|
| SRC-012 | Dublettenkandidaten, manuelle Feldwahl, Umhängen aller modellierten Zitate und Undo; feldweiser Bibliotheksabgleich | Weitergehender Kennungenvergleich und dokumentübergreifende Dublettenbearbeitung |
| SRC-024 | Typabhängige Vollständigkeitshinweise und Navigation geprüft; verwaiste Zitate werden bereits vom Modell abgelehnt | Prüfung je Zitierstil, vollständige Datums-/Kennungsvalidierung und Platzhalter |

Grenzen: 1.024 Quellen im Dokument, 64 Personen je Quelle, 4.096 UTF-8-Bytes je
Quellenfeld; weiterhin gelten Modell- und XML-Gesamtbudgets. Die einfache Ausgabe nennt
bei zwei Autoren beide Namen, ab drei Autoren den ersten mit „et al.“. Das Verzeichnis
nennt alle Autoren. Die Reihenfolge ist weiterhin die erste Verwendung im Dokument.

## RIS und BibTeX: tatsächlicher Umfang

Beide Codecs sind eigenständige `no_std`-Implementierungen in `doc_io`. Unterstützt werden
UTF-8, UTF-8-BOM und UTF-16 mit BOM. Ein nicht darstellbares Feld, ein unbekannter Typ oder
eine mehrdeutige Struktur bricht die gesamte Datei mit Datensatz-/Feldmeldung ab. Auch beim
Export wird zuerst vollständig kodiert und wieder eingelesen; abgesehen von lokalen IDs
muss der Modellinhalt gleich bleiben. Ein fehlgeschlagener Export überschreibt keine Datei.
Es handelt sich um dokumentierte Teilprofile, keine vollständige RIS-/BibLaTeX-Kompatibilität.

| Format | Unterstützt | Bewusst abgelehnt |
|---|---|---|
| RIS | `BOOK`, `JOUR`, `ELEC`; geordnete `AU`/`A1`; Titel `TI`/`T1` (Buch auch `BT`), `PY`/`Y1` als Jahr; `PB`, `CY`, `ET`, `JF`/`JO`/`T2`, `VL`, `IS`, `SP`/`EP`, `UR`, `SN`, `ST`, `N1`, `LA`, Zugriff `Y2`; mehrzeilige Werte | Weitere Typen/Rollen, Abstracts, Schlüsselwörter, Anhänge und unbekannte Felder; volle Publikationsdaten, Körperschaften, getrennte Mittelnamen, GUIDs und unpassende typabhängige Angaben |
| BibTeX | `@book`, `@article`, `@online`, `@misc` als Website mit URL; `author`, `title`, `year`, `publisher`, `address`, `edition`, `journal`, `volume`, `number`, `pages`, `url`, `note`, typabhängig `isbn`/`issn` und `howpublished`; `urldate` als `YYYY[-MM[-DD]]` | Weitere Typen/Felder, `@string`, `@preamble`, Verkettung, TeX-Befehle, geschützte Großschreibung, Namenssuffixe, getrennte Mittelnamen, GUID/Sprachkennung/Kurztitel und sonstige nicht abbildbare Werte |

RIS-Personen verwenden `Nachname[, Vorname]`; BibTeX-Personen `Nachname, Vorname` oder
einen einzelnen Nachnamen. Mehrere Vornamen bleiben ein gemeinsames Vorname-Feld.
Eine BibTeX-Körperschaft steht als alleiniger Autor in zusätzlicher Klammerung:
`author = {{Institut für Forschung}}`. Sichere einfache TeX-Escapes wie `\&`, `\%` und
`\_` werden verstanden; ASCII-Leerraum in BibTeX-Feldern wird entsprechend der Feldsyntax
normalisiert. Unbekannte Befehle werden nicht ausgeführt oder still entfernt.

Ein RIS-`DO` kann nur als eindeutige `https://doi.org/...`-URL übernommen werden. Ein
abweichender zusätzlicher `UR`-Wert wird abgelehnt, da DOI und URL im aktuellen Modell
noch kein getrenntes Feldpaar bilden. Externe RIS-IDs und BibTeX-Schlüssel werden geprüft,
aber auf lokale IDs abgebildet. Ihr Originaltext wird derzeit nicht archiviert. Der RIS-
Export lässt IDs weg; BibTeX erzeugt lokale Schlüssel. Für die dauerhafte Erhaltung aller
aktuellen Modellangaben bleibt Sources.xml das umfassendere Austauschformat.

Grenzen: 4 MiB Eingabe, 1.024 Quellen, 64 Autoren pro Quelle, 4.096 UTF-8-Bytes je
Modellfeld, 2 MiB Quellenmodell; RIS zusätzlich höchstens 4.096 Tags. Beide Parser laden
keine externen Dateien und rufen keine Netzwerkressourcen ab.

## Tatsächlicher DOCX-Umfang

**Keine vollständige Word-Dateikompatibilität.** Die Anwendung unterscheidet zwei Modi:

1. **Modellbearbeitung:** Das ganze Dokument passt in das nachfolgend beschriebene Profil.
   Text, Formate, Quellen, Zitate und Noten können mit den bestehenden Werkzeugen bearbeitet werden.
2. **Erhaltungsmodus:** Ein geprüftes konventionelles DOCX enthält nicht modellierte Eigenschaften.
   Die Originaldatei wird vollständig gehalten; unverändertes Speichern liefert exakt dieselben
   Archivbytes. Das Hauptdokument wird über seine Paketbeziehung gefunden. Eigene Formatvorlagen,
   Tabellen, Bilder, Kopf-/Fußzeilen, Metadaten, Noten, externe Links und unbekannte Teile bleiben
   erhalten. Die Oberfläche zeigt ausdrücklich nur einen Textauszug, keine originalgetreue Seite.

Im Erhaltungsmodus können geeignete direkte `w:t`-Textbereiche des Haupttexts, von Tabellenzellen,
Kopf-/Fußzeilen und normalen Fuß-/Endnoten einzeln ersetzt werden. Die Seitenleiste filtert nach
Dokumentbereich. Verwandte Teile werden über ihre OPC-Beziehung, ihren Inhaltstyp und XML-Wurzelknoten
zugeordnet; Dateinamen werden nicht geraten. Gemeinsame Kopfzeilen erscheinen nur einmal.
Nur der betreffende XML-Text samt erforderlichem `xml:space` wird angepasst;
alle übrigen XML-Bytes und Nutzdaten der Paketteile bleiben identisch. Das ZIP wird nach einer Änderung
neu aufgebaut; Kompression und ZIP-Metadaten sind dann nicht byteidentisch. UTF-8/BOM und UTF-16 LE/BE
werden erhalten. Undo/Redo und Speichern-Markierungen funktionieren auch in diesem Modus.

Feldresultate, Hyperlinktexte, Inhaltssteuerelemente und andere nicht eindeutig zugeordnete Bereiche
sind nicht einzeln bearbeitbar. Komplexe Felder oder Revisionen sperren den gesamten betroffenen
Dokumentteil; andere zugeordnete Teile können weiterhin bearbeitbar sein. Trenn-/Fortsetzungsnoten
werden nicht bearbeitet. Erkannter Dokumentschutz, Änderungsverfolgung oder Signaturen sperren
sämtliche selektiven Textänderungen. Links und eingebettete
Dateien werden nicht geöffnet/ausgeführt. Der Auszug enthält keine separaten Kopf-/Fußzeilen oder
Notentexte; Feldanzeigen sind gespeicherte Ergebnisse und werden nicht aktualisiert. Daher können
auch Verzeichnisse oder Verweise nach einer Textänderung bis zur Aktualisierung in Word veraltet sein.
`Dateiinformationen` zeigt Modus und erkannte Grenzen. Neue Absätze, Format-/Layoutänderungen und
wissenschaftliche Modellwerkzeuge sind in diesem Modus noch nicht verfügbar.

Die folgenden Details betreffen den streng geprüften **Modellmodus**:

Das Profil umfasst acht grundlegende OPC-Teile: Content Types, Paketbeziehungen, Hauptdokument,
Formatvorlagen, Dokumentbeziehungen, Bibliografiedaten, deren Eigenschaften und Beziehung.
Bei vorhandenen Noten kommen `word/footnotes.xml` und/oder `word/endnotes.xml` sowie
`word/settings.xml` mit passenden Content Types und Dokumentbeziehungen hinzu. Die
Einstellungen listen sämtliche speziellen Trenn-/Fortsetzungsnoten auf. Damit entstehen
zehn Teile bei einer Notenart oder elf bei beiden; Dokumente ohne Noten behalten das
bisherige achtteilige Profil.
Unterstützt sind die vier mitgelieferten und eigene vererbbare Absatzvorlagen, ein dokumentweiter Seitenabschnitt
mit Papiermaßen und Rändern, Text, Zeilenumbrüche/Tab-Zeichen, Fett/Kursiv/einfache
Unterstreichung, Durchstreichen, direkte RGB-/Auto-Schriftfarben, Schriftfamilien, Schriftgrößen in halben Punkten,
16 Hervorhebungsfarben mit expliziter Abschaltung, Hoch-/Tiefstellung, Absatzabstände/Einzüge/Ausrichtung und
Seitenwechsel vor Absätzen. Hinzu kommen die drei Quellentypen und Metadaten sowie Zitat-
und Literaturverzeichnisfelder sowie strukturierte Fuß-/Endnoten mit formatierten Absätzen
und Zitatfeldern. Absatz-, Quellen- und Noten-IDs bleiben im eigenen Roundtrip erhalten.
Feldanzeigen werden aus den strukturierten Quellen neu berechnet.
Optionale fehlende Absatzkennungen werden kollisionsfrei neben Quellen-, Noten- und
vorhandenen Absatzkennungen vergeben. Themefarben bleiben im Erhaltungsmodus, damit ihre
Abhängigkeiten von Dokumentdesigns nicht durch eine feste Ersatzfarbe verloren gehen.

Noten verwenden `w:footnoteReference`/`w:endnoteReference` im Haupttext und genau ein
passendes `w:footnoteRef`/`w:endnoteRef` am Anfang ihres ersten Absatzes. Unterstützt sind
automatische, ausdrücklich hochgestellte Notenzeichen, dezimale Nummerierung ab 1 ohne
Neustart sowie Seitenfuß bzw. Dokumentende als Position. Der Export schreibt diese
Eigenschaften ausdrücklich. Fremde Endnoten ohne explizites `w:numFmt w:val="decimal"`
werden abgelehnt, da Words Standard römische
Zahlen verwendet. Andere Nummerierungen, eigene Zeichen, verschachtelte Noten, zusätzliche
Notenbeziehungen, nicht definierte Zeichenstyles und individuelle Trennlinien liegen
außerhalb dieses Profils. Beide Standard-Trenn-/Fortsetzungseinträge mit IDs `-1`/`0`
samt passenden Verweisen in `settings.xml` sind erforderlich; diese IDs stehen deshalb
nicht für normale Noten zur Verfügung. Fehlende, doppelte, verwaiste oder falsch
zugeordnete Notenverweise verhindern den Import. Unformatierte oder nur über einen
Zeichenstyle formatierte Notenzeichen werden nicht still zu hochgestellten Zeichen umgewandelt.

Fremde Notenkennungen gelten in OOXML getrennt für Fuß- und Endnoten. Kollidieren sie mit
anderen Modell-IDs, werden sie samt Verweisen deterministisch auf freie IDs abgebildet;
Notenabsätze ohne optionale `w14:paraId` erhalten ebenfalls eigene IDs. Das ignorierbare
Attribut `jf:order` im Namensraum `urn:jfword:notes:1` bewahrt im eigenen DOCX-Roundtrip
zusätzlich die interne Reihenfolge gemischter Notenarten. Es bestimmt keine sichtbare Nummer.

`CITATION` unterstützt `\m` für weitere Quellen, `\p` für Fundstellen, `\f`/`\s` für
Präfix/Suffix und `\n`/`\y`/`\t` für Autor-/Jahr-/Titelunterdrückung. Schalter beziehen
sich auf die jeweils vorhergehende Quelle. Doppelte Schalter je Quelle und ungültige
Referenzen werden abgelehnt. Noch nicht unterstützt sind unter anderem `\l` (Sprache),
`\v` (Band) und allgemeine Formatierungsschalter. Neben `w:fldSimple` werden komplexe Felder
mit `w:fldChar` und auf mehrere `w:instrText`-Läufe verteilten Anweisungen gelesen, sofern sie
innerhalb eines Absatzes liegen; das gilt auch in Noten. Ergebnistext wird aus den Quellen
neu erzeugt. `dirty` und ausdrücklich nicht gesperrte Felder werden unterstützt. Verschachtelte,
absatzübergreifende, gesperrte oder direkt formatierte Feldresultate verbleiben im Erhaltungsmodus,
weil ihr zusätzlicher Zustand noch nicht im Modell abgebildet ist. Der Export verwendet
kanonische `w:fldSimple`-Felder. Das übrige beschriebene Paketprofil bleibt Voraussetzung.

Der Modellleser prüft feste unterstützte Teile und jedes verarbeitete Element. Nicht modellierte
Inhalte verhindern die Modellkonvertierung; die Anwendung nutzt dann bei gültiger Paketstruktur
den Erhaltungsmodus. `--check-docx` prüft weiterhin ausschließlich das strenge Modellprofil.
Makrodateien, alte binäre DOC/DOT-Dateien und Verschlüsselung werden weiterhin abgelehnt.
Die geplante inaktive Erhaltung von VBA-Streams ist offen. Es wird kein VBA ausgeführt.
Keiner der Codecs holt externe Beziehungen oder XML-Ressourcen aus dem Netz.

Das Schreiben nutzt zunächst unkomprimierte ZIP-Einträge. Lesen unterstützt zusätzlich
DEFLATE, Daten-Deskriptoren und UTF-16-kodierte XML-Teile. Verschlüsselung, ZIP64,
mehrteilige ZIPs und nicht eindeutige Pfade werden abgelehnt. XML-Kommentare, CDATA, DTDs,
weitere Processing Instructions und gemischter Element-/Textinhalt sind im Teilparser
nicht freigegeben. Dieser Baustein ist kein vollständiger XML-1.0-Prozessor.

Aktuelle Grenzen: maximal 32 MiB Paket, 8 MiB pro ZIP-Teil, 32 MiB entpackte Gesamtdaten,
512 Teile; XML 4 MiB, 30.000 Elemente, Verschachtelung 48; Modell 2 MiB geschätzte Nutzdaten,
2.048 Absätze über Haupttext und Noten zusammen, 1.024 Quellen und 1.024 Noten.
Vollständig expandierter Text einschließlich Noteninhalten und wiederholter
Zitat-/Literaturfelder ist auf 4 MiB begrenzt. Undo hält höchstens 64 Schritte und etwa 8 MiB Modellkopien.
Für Dokumentdateien gibt es noch kein Journal/Recovery und keine Sperrung gegen konkurrierende
externe Änderungen. Die persönliche Quellenbibliothek besitzt eigene Sicherungs- und Konfliktregeln.
Der Erhaltungsmodus begrenzt den Auszug auf 4.096 Absätze/1 MiB und einen Textaustausch auf
64 KiB ohne neue Zeilen-/Tabzeichen; die gesonderte Historie hält bis zu 64 Schritte/4 MiB.

Die Liste zuletzt verwendeter Dateien liegt bei normalem Start neben der Quellenbibliothek
in `recent.ini`. `--settings-dir ORDNER` wählt einen Test-/Alternativordner. Ohne expliziten
Ordner schreiben Headless-Läufe keine Benutzerprofildaten. Bis zu 24 Einträge werden gespeichert;
die Suche durchsucht Dateiname und Pfad. `--editor`, `--sources` und `--navigation` steuern den
Anfangszustand, während ein normaler Start die Startseite zeigt.

## Persönliche Quellenbibliothek

Standardpfad bei normalem Start: `%APPDATA%/jfWord/sources.jflib` auf Windows und
`$XDG_CONFIG_HOME/jfWord/sources.jflib` beziehungsweise `$HOME/.config/jfWord/sources.jflib`
auf Linux. `--library-dir ORDNER` wählt einen anderen Ordner. Headless-Läufe verwenden
ohne diesen Parameter keine Profilbibliothek. Öffnen allein legt keine Dateien an.

Das eigene XML-Format trägt den Namensraum `urn:jfword:source-library:1`, Version 1 und
den nächsten noch nicht vergebenen Bezeichner. Die Quellen verwenden das bereits
unterstützte Bibliografieprofil. Unbekannte Versionen/Felder werden abgelehnt; eine
beschädigte vorhandene Datei wird niemals stillschweigend als leere Bibliothek geöffnet.
Grenzen: 1.024 Quellen, 2 MiB Modell und 4 MiB Datei. IDs gelten jeweils nur innerhalb
einer Bibliothek oder eines Dokuments; numerische Gleichheit verbindet keine Quellen.

Speichern validiert das gesamte Ergebnis, erwirbt eine Betriebssystemsperre auf der
benachbarten `.lock`-Datei und vergleicht den aktuellen Inhalt mit dem zuvor gelesenen
Dateistand. Bei einem Konflikt muss „Neu laden“ erfolgen; der fehlgeschlagene Vorgang
ändert weder Datei noch Bibliotheksmodell. Neuladen setzt die Sitzungshistorie zurück.
Kooperierende jfWord-Instanzen werden serialisiert; andere Programme, die diese
Sperre ignorieren, und verteilte Dateisysteme besitzen keine weitergehende Garantie.

Vor einer Ersetzung wird der vorherige gültige Stand atomar nach `.bak` geschrieben.
Scheitert die Sicherung, bleibt die Hauptdatei unverändert. Wiederherstellen zeigt die
Sicherung zuerst an und prüft beim Bestätigen Hauptdatei und Sicherung erneut. Ersetzte
Bytes bleiben unter `.damaged` erhalten; eine spätere Wiederherstellung ersetzt diese
einzelne Rettungskopie. Die `.lock`-Datei bleibt bestehen, ihre Sperre endet mit dem
Dateihandle beziehungsweise Prozess. Normale Bearbeitung und Undo verwenden keine
alten IDs erneut. Bei Wiederherstellung über eine gültige Hauptdatei bleibt der höhere
ID-Zähler erhalten; bei beschädigter Hauptdatei ist nur der Zähler der Sicherung bekannt.
Dies ist eine lokale Sicherung, noch kein mehrstufiges Journal oder Cloud-Sync.

## Zwischenablage und Austauschprofil

Die wiederverwendbaren Grundlagen liegen in `doc_core::DocumentFragment`, `doc_io` und
`Ui::clip_set_many`; die App verbindet sie mit Auswahl, Menüband und Undo. Das native Format
`application/x-jfgui-docx-fragment` trägt den Versionskopf `JFWCLIP` mit Version 1 und ein
validiertes DOCX-Paket. IDs werden beim Einfügen neu zugeordnet. Fremde oder beschädigte
Fragmente können weder Verweise einschleusen noch die aktuelle Auswahl teilweise ersetzen.

RTF-Import/-Export unterstützt direkte Zeichenformate, RGB-Farbe, halbe Punktgrößen,
Absatzausrichtung, Abstände, Einzüge, Absatz-/Zeilenwechsel, Unicode und Windows-Einbyte-
Codepages 874/1250–1258. Eingabe ist auf 16 MiB, expandierter Text auf 2 MiB begrenzt.
Schriftfamilien und effektive Zeichenformate einschließlich Hervorhebung und Hoch-/Tiefstellung bleiben erhalten.
Sprachmetadaten und Vorlagenidentitäten werden vereinheitlicht; implizite
Normal-Stylesheet-/Default-Zeichenwerte werden nicht aufgelöst. Bilder, Objekte,
Listen und weitere nicht darstellbare Eigenschaften werden abgelehnt. Das erweiterte
[Tabellenprofil](docs/NESTED_TABLES_AND_IMPORT.md) ergänzt gemeinsame Spaltenraster,
Verbünde und Verschachtelung; vollständige RTF-Dateibearbeitung bleibt offen.

HTML wird mit Inline-CSS exportiert und im dokumentierten begrenzten Profil importiert. Bei nicht unterstütztem
Rich-Text-Inhalt bleibt die Auswahl erhalten; Klartext lässt sich ausdrücklich mit
`Strg+Umschalt+V` einfügen. Im externen RTF/HTML werden Felder zu sichtbarem Ergebnistext;
Quellenmetadaten und Notenkörper reisen ausschließlich im nativen jfWord-Fragment mit.
Ein allein kopiertes Literaturverzeichnis verwendet extern den aktuellen Ursprungstext,
intern bleibt es ein dynamisches Feld für die Zitate des Zieldokuments.

Win32 veröffentlicht Unicode-Text, CF_HTML, registriertes RTF und das private Binärformat
gemeinsam. X11/Wayland bieten entsprechende MIME-Typen einschließlich üblicher HTML-/RTF-
Aliase an. Einfacher Text oder ein Bild ersetzt alle vorherigen Rich-Formate.

## Noch ausstehend

Die direkte Rich-Text-Eingabe deckt Format-, Absatz-, Zeilen- und Seitengrenzen in einem
Haupttext oder Notenbereich ab. HTML-Import, erweiterte RTF-Inhalte, weitere Feldtypen sowie
das strukturierte Kopieren/Einfügen von Tabellen und Objekten fehlen noch.
Druck/PDF sind offen. Enter erzeugt neue Absätze, Umschalt+Enter interne Zeilenwechsel.
Individuelle Tabstopps, Silbentrennung, vollständiges Unicode-
Shaping/Bidi, Ligaturen, Font-Fallback und Word-identischer Umbruch sind noch offen. Unicode bleibt
im Modell und DOCX erhalten, auch wenn der eingebettete Font ein Zeichen nicht abdeckt.

Insbesondere folgen weitere Quellentypen und Autorenrollen, Sortierung und weitere Bibliotheksfunktionen,
CSL, vollständige BibTeX/BibLaTeX-/RIS-Abdeckung, CSL-JSON, erweiterte Notennummerierung
und Notenlayouts, Querverweise, Formeln, Inhaltsverzeichnis,
Revisionen und die übrigen Funktionen aus `FEATURES.md`. Die eigene Add-on-Schnittstelle
und das Paketformat sind spezifiziert, aber noch nicht implementiert. Die vollständige
Registerstruktur ist vorhanden; noch fehlende Aktionen sind sichtbar deaktiviert.

## Prüfung

Geprüfter Stand vom 11. Oktober 2026: Beide Builds unter `build/jfword-next` erfolgreich
(die geöffnete Standard-EXE wurde nicht ersetzt); vollständiger Lauf mit
**1.015 nativen Prozessprüfungen** auf Windows und Linux bestanden, zusätzlich die darin
ausgeführten Modell-/Bibliotheksprüfungen und unabhängigen XML-/ZIP-Vergleiche. Lint und
generierte API-Dokumentation sind aktuell. Die bestehenden Gallery-Textbox-/Mehrzeilentests
bestehen; das Mehrzeilen-Referenzbild ist pixelidentisch zur vorhandenen ASM-Referenz.
Diese Ergebnisse belegen den beschriebenen Teilumfang, keine vollständige Word-Konformität.

Die anschließende jfOffice-/Navigationsetappe wurde separat unter `build/jfoffice-view`
geprüft: **35 UI-Skripte je nativem Ziel, insgesamt 70 erfolgreiche Prozessläufe**, jeweils
mit `--verify-dirty` und ohne `UIWARN`. Die darin enthaltenen Selbstprüfungen melden auf
Windows und Linux jeweils **9.410 bestanden, 0 fehlgeschlagen**. `navigation.uis` ergänzt
Überschriften-/Seitensprünge, genaue UTF-8-Suchauswahl einschließlich formatierter Noten,
Groß-/Kleinschreibung und Wortgrenzen, Zoomgrenzen sowie den reversiblen Fokusmodus.
Zusätzlich bestehen 16 Darstellungsfälle für Start-/Papieransicht, hell/dunkel,
100/150/200 Prozent und 1000/1240/1920 DIP Breite; repräsentative Bilder wurden angesehen.
Die neuen `view_checks.rs` prüfen style-unabhängige Dokumentfonts, unveränderte Modellwerte,
Layout-/Thumbnailpixel, Koordinatengrenzen und wiederholten Dokumentwechsel.
`ribbon_actions.uis` prüft getrennten Dropdown-Fokus, Enter/Space/Alt+Down/F4, Esc-Rückkehr,
echte Einfügeoptionen, Formatübertragung/Reset/Undo, Absatz-Dialogstarter und verschachtelte
Gruppenüberläufe. Bilder decken 600/1000/1240 DIP, hell/dunkel und 100/150/200 Prozent ab.
`font_size.uis` prüft direkte und vorgewählte Größen, halbe Punkte, ungültige Eingabe,
Grenzen, Undo/Redo und unveränderte Auswahlgrenzen.

```powershell
python build_rs.py app jfWord --target both
python apps/jfWord/tests/check.py --target both
python build_rs.py check
python tools/gendoc.py --check
```

`check.py` prüft die echten Windows-/Linux-Binaries mit unabhängig durch Pythons
Standardbibliothek erzeugten ZIP-Varianten und untersucht OPC/XML separat. Enthalten sind
Modell-/Unicode-/XML-Prüfungen, Quellen-/Feld-Roundtrip, fremde und beschädigte Pakete,
Ressourcenlimits, ein deterministischer Mutationskorpus und GUI-Skripte mit `--verify-dirty`.
`tests/science.py` erzeugt unabhängig mehrsprachige Quellenlisten im Sources.xml-Format (UTF-8/UTF-16,
Namensraum-Aliase, Personen/Körperschaften, Metadaten) und prüft XML-Export sowie DOCX-
Feldverknüpfungen. Fehlerfälle prüfen auch, dass vorhandene Ausgabedateien erhalten
bleiben. Native Modelltests decken atomare Stapelimporte, Dubletten, Grenzen und Undo ab.
`editor_checks.rs` prüft UTF-8-Auswahlgrenzen, Suche über Formatwechsel, atomare Ersetzungen,
Feldschutz, Ausrichtung, Seitenwechsel und Umbruch mit einer unabhängigen festen Fontmetrik.
`rich_checks.rs` prüft jede UTF-8-Auswahlgrenze eines gemischt formatierten Absatzes,
Einfügeformate, Feldschutz, atomare Absatzteilung, Farben und Herkunft der Layoutfragmente
über mehrere Seiten. `rich.uis`, `rich_pages.uis` und `tests/rich_text.py` prüfen die gemeinsame
Seitenbearbeitung, gespeicherte Zeichenformate und unveränderte Notenverweise unabhängig im XML.
`rich_citations.uis` prüft Textbearbeitung um geschützte Zitate, Wechsel zum Felddialog,
Änderung der Fundstelle und erneutes Einfügen an der Rich-Text-Schreibposition.
`fragment_checks.rs`, `clipboard_codec_checks.rs`, `rich_clipboard.uis`, `rich_fields.uis`,
`rich_field_remove.uis` und `tests/clipboard.py` prüfen mehrabsätzige native Zwischenablage,
unabhängig erzeugtes RTF, XML-Formatwerte, Noten-/Quellenverknüpfungen, Dialogfokus,
Undo/Redo, Leerfelder und abgelehnte Teilfelder/fehlerhafte Formate. Die Cursorposition wird
auch geprüft, wenn das Entfernen eines Zitats ein davor stehendes Literaturverzeichnis verkürzt.
`tests/fields.py` erzeugt komplexe Word-Felder unabhängig vom Exporter, einschließlich Noten,
aufgeteilten Anweisungen, ungültigen Grenzen, Sperren und formatierten Resultaten.
Die Feldleiste ist zusätzlich bei 1000×700 DIP, dunklem Thema und 150 Prozent Skalierung geprüft.
Die Feldstruktur folgt der [Microsoft-Referenz für FieldChar](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.fieldchar?view=openxml-3.0.1).

Zusätzlich wurden RTF-Daten über Windows RichEdit eingelesen und wieder ausgegeben:
Text, Zeichenformate, Punktgrößen, Farben und Ausrichtung des Prüfdokuments stimmen überein.
Das ist kein Microsoft-Word-Roundtrip. Die gemeinsame Plattformschicht besteht 54 CF_HTML-/
Mehrformatprüfungen und 31 native X11-Transferprüfungen mit privater Testauswahl, einschließlich
großer INCR-Daten und eines unabhängigen Anbieters mit MIME-Aliasnamen. Die Systemzwischenablage
wurde dabei nicht verändert. Win32-/Wayland-Transfers sind gebaut und über die Headless-Schnittstelle
geprüft, aber noch nicht praktisch gegen einen zweiten nativen GUI-Prozess abgenommen.
Die bisherigen Segment-/Felddialogtests laufen ausdrücklich mit `--structure`; `paper.uis`
und `paragraphs.uis` prüfen weiterhin den regulären Seiteneditor.
`story_edit_checks.rs`, `rich_story.uis` und `tests/rich_text.py` prüfen mehrabsätzige
Auswahlen in beide Richtungen, Ersetzen und Absatzbefehle, Einfügeformate in leeren Absätzen,
Mausauswahl, Clipboard-Zeilenumbrüche, Notenisolierung und die gespeicherte OOXML-Struktur.
`indent_checks.rs`, `indents.uis` und `tests/formatting.py` prüfen Erstzeilen- und hängende
Einzüge über Zeilen-/Seitengrenzen, in Noten und mit allen vier Ausrichtungen sowie
unabhängig erzeugte OOXML-Eigenschaften und abgelehnte doppelte/falsche Attribute.
Die Einzüge verwenden standardkonforme `w:ind`-Attribute `firstLine` beziehungsweise `hanging`
([Microsoft-Referenz](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.indentation?view=openxml-3.0.1));
zeichenbasierte Einzüge und zusätzliche nicht modellierte Attribute bleiben im Erhaltungsmodus.
`editor.uis` führt Auswahlformatierung, Such-/Ersetzung, Seitenlayout, Vorschau und DOCX-
Neuladen durch; Screenshots decken helle/dunkle Darstellung und 150-%-Skalierung ab.
Tests und Screenshots schreiben ausschließlich nach `build/`.
`compat_checks.rs` und `tests/compatibility.py` prüfen zusätzlich fremde DOCX-Pakete,
OPC-Beziehungen, Strict-Namensräume, exakt erhaltene Originalarchive, selektive Textänderungen,
UTF-16 und unveränderte Bilder, Tabellenstruktur, Kopf-/Fußzeilen, Noten und unbekannte Teile.
Das unabhängige Python-Korpus ist synthetisch und ersetzt keinen Test gegen Microsoft Word.
`shell.uis`, `tests/shell.py` und `paper.uis` prüfen Startseite, Vorlagen, persistente angeheftete
Dateien, fehlgeschlagenes Öffnen, direktes Schreiben und Menü-/Dialogwege.
`paragraph_checks.rs`, `paragraphs.uis` und `paragraphs_rich.uis` prüfen atomare Absatzoperationen,
Unicode, Auswahl/Cursor nach Undo, Zeichenformate, Notenkontext und Speichern/Neuladen.
`story_checks.rs`, `tests/stories.py` und `stories.uis` prüfen Textänderungen in zugeordneten
Kopf-/Fußzeilen und Noten, gemeinsame Historie, Filter sowie byteidentische übrige Paketteile.
Die unabhängigen Varianten umfassen UTF-8, UTF-16 LE/BE, Strict-OOXML und abweichende Paketpfade.
`ris_checks.rs`, `bibtex_checks.rs` und `workflow_checks.rs` ergänzen native Codec-,
Grenz- und Importtransaktionstests. `tests/exchange.py` erzeugt RIS/BibTeX unabhängig,
prüft XML-/DOCX-Metadaten und beschädigte Daten einschließlich bestehender Zieldateien.
`exchange.uis` deckt Abbrechen, Teilauswahl, Dubletten, Quellenfilter, Undo-Auswahlstabilität,
gesperrte Dokumentkürzel während der Vorschau und Quellenexport ab.
`citation_checks.rs` prüft Mehrfachzitate, UTF-8-Einfügegrenzen, Formaterhaltung,
Verwendungsstellen, Undo und Expansionsgrenzen. `tests/citations.py` erzeugt Feldcodes
unabhängig, prüft Schalterzuordnung, Grenzen, Ablehnungen und stabilen DOCX-Roundtrip.
`z_citations.uis` deckt Cursorposition, Entwürfe, Umordnung, Verwendungsnavigation,
Modalität und Speichern/Neuladen einschließlich Dialog-Screenshots ab.
`merge_checks.rs` prüft alle Feldentscheidungen, Autorenmodi, Referenzumschreibung,
Undo/Redo, Auswahlfehler und Expansionsgrenzen. `audit_checks.rs` prüft die typabhängigen
Hinweise, partielle Datumsangaben und unveränderte Historie. `tests/merging.py` untersucht
alle XML-Quelldaten, Feldschalter und unveränderten Paketbestandteile unabhängig.
`source_tools.uis` vergleicht Quellen, prüft Abbruch, Umhängen, Undo, DOCX-Neuladen,
Quellenprüfung und den Übergang zur Quellenbearbeitung. Sein gespeichertes Dokument
muss dem unabhängig untersuchten Modellresultat entsprechen. Dialog-Screenshots prüfen
1000×700 DIP, hell/dunkel und 150-Prozent-Skalierung.

`library_checks.rs` prüft unabhängige Identitäten, Kopiertransaktionen und begrenzte Historie;
`library_io_checks.rs` prüft das Bibliotheksformat, Speicherkonflikte, fehlgeschlagene
Sicherungen/Ersetzungen und Wiederherstellung. `tests/library.py` erzeugt XML unabhängig
und startet getrennte native Prozesse für Sperr- und Konflikttests. `library.uis` prüft
Übernahme, Bearbeitung, Abgleich, getrenntes Undo, neue Dokumente und Wiederherstellung.
`library_reopen.uis` öffnet die gespeicherte Bibliothek in einem neuen Prozess;
`library_failures.uis` stellt eine zunächst beschädigte Bibliothek nach Vorschau wieder her;
`library_disabled.uis` prüft die Trennung automatisierter Läufe vom Benutzerprofil.
Testbibliotheken liegen ausschließlich unter `build/`; `.lock`-Dateien dürfen beim Betrieb
nicht entfernt werden. `--check-library DATEI --output ZIEL` dient dem Codec-Roundtrip;
`--expect-reject` schreibt dabei keine Ausgabe.

`notes_checks.rs` deckt Notenanker, Nummerierung, Umwandlung, Undo und wissenschaftliche
Quellenbezüge ab. `notes_io_checks.rs` prüft reichhaltige Noten, stabile IDs und Reihenfolge,
fremde Kennungskollisionen sowie abgelehnte Notenparts, Verweise und Nummerierungsvarianten.
`notes_layout_checks.rs` prüft Fußnotenraum, Fortsetzungen, Endnoten, vollständige Textausgabe
und Layoutgrenzen mit fester Fontmetrik. `tests/notes.py` untersucht OPC/XML unabhängig;
`notes.uis` führt Einfügen, Formatieren, Zitieren, Navigation, Umwandeln, Entfernen/Undo,
Suche und Speichern/Neuladen durch. Noten-Screenshots ergänzen die Vorschauprüfung.
`notes_context.uis` prüft die Rückkehr zum richtigen Haupttextabsatz nach Undo und
das gemeinsame Entfernen von Absatz und Noten. `notes_empty.uis` prüft importierte
leere Noten und erneutes Schreiben nach Undo, ohne beim Öffnen das Dokument zu verändern.

Die `.uis`-Skripte werden auch vom normalen `build_rs.py test --quick` gefunden.
`core.uis` erzeugt die Dateien für die anschließenden Datei- und Quellenlistentests.
`--self-test`, `--fixture`, `--check-docx`, `--output` und `--expect-reject` ermöglichen
zusätzliche Headless-Prüfungen. `--output` beim Probe-Befehl schreibt nur nach erfolgreichem Import.
`--check-preserved DATEI.docx` prüft dagegen die allgemeine Erhaltungssitzung; `--output`
schreibt deren Ergebnis. `--edit-index N --expected-file ALT.txt --replacement-file NEU.txt`
prüft eine gezielte Textänderung mit explizitem Vorzustand. Diese UTF-8-Dateien sind Testeingaben.
`--check-sources DATEI.xml|.ris|.bib` prüft Quellenlisten; optional schreibt `--output`
eine normalisierte Liste im Format ihrer Dateiendung und `--docx-output DATEI.docx` ein
Prüfdokument mit Zitatfeldern. Alle angeforderten Kodierungen werden vor dem ersten
Schreibvorgang geprüft. `--expect-reject` prüft auch Exportgrenzen und schreibt niemals.
`--preview --no-sources --file DATEI.docx` startet direkt mit einer größeren Seitenvorschau.

Die Paketdatei wurde zusätzlich mit `python-docx` gelesen (Absätze, Formatvorlagen,
Seitenabschnitt, Feldknoten). Ein echter Microsoft-Word-Roundtrip und eine visuelle
Word-Referenzprüfung wurden noch nicht durchgeführt; sie bleiben eigene Abnahmeschritte.

Technische Referenzen: [PKWARE ZIP-Spezifikation](https://pkware.cachefly.net/webdocs/casestudies/APPNOTE.TXT),
[Microsoft: Bibliografiedaten und Zitatfelder](https://learn.microsoft.com/en-us/office/vba/word/concepts/working-with-word/working-with-bibliographies),
[Open-XML-Bibliografieelemente](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.bibliography.source).
Die VBA-Beispiele der Microsoft-Dokumentation dienen nur zur Erläuterung der Dateistruktur.
Weitere Primärreferenzen: [BibTeXing von Oren Patashnik](https://texdoc.org/serve/btxdoc/0),
[Zoteros öffentliche RIS-Feldzuordnung](https://raw.githubusercontent.com/zotero/translators/master/RIS.js).
Für Mehrfachzitate: [Microsoft: CITATION-Feld](https://support.microsoft.com/en-gb/word/field-codes-citation-field),
[MS-OI29500: Zuordnung der CITATION-Schalter](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/70651763-1bd2-4fa2-ae94-420b4a980998)
und ECMA-376 Teil 1, §17.16.5.8.
Für Noten: [Microsoft: Zuordnung der Fuß-/Endnotenkennungen](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.footnoteendnoteseparatorreferencetype.id),
[Noteneigenschaften und Nummerierung](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.footnoteproperties),
[Deklaration spezieller Fußnoten](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.footnotespecialreference),
[MS-OI29500: abweichender Word-Standard für Endnotennummern](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-oi29500/a910e9f7-f95f-49ad-ad23-68ea9a7922cd).
Für getrennte Dokumentteile: [Microsoft: WordprocessingML-Dokumentstruktur](https://learn.microsoft.com/en-us/office/open-xml/word/structure-of-a-wordprocessingml-document)
und [HeaderReference und Paketbeziehung](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.headerreference).
Für Zeichenformate: [Microsoft: direkte und automatische Schriftfarbe](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.color)
und [einfaches Durchstreichen](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.strike).

## Marken

Microsoft, Word und Office sind Marken der Microsoft Corporation. jfWord ist ein
eigenständiges Projekt und steht in keiner Verbindung zu Microsoft; es wird von Microsoft
weder unterstützt noch empfohlen. Die Namen stehen hier nur, um Dateiformate und
vergleichbare Funktionen zu beschreiben.
