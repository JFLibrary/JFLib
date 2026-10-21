# jfWord - Feature- und Architekturkatalog

Stand: 2026-10-11. Phase 1: Anforderungsspezifikation mit ausgewiesenen Implementierungsteilen.
Sprache der Anwendung: Deutsch.

Der erste ausführbare Entwicklungsstand und seine konkreten Grenzen stehen in
[README.md](README.md). Die folgenden Anforderungen beschreiben weiterhin das Gesamtziel.

Dieses Dokument beschreibt das vollständige Produktziel einer eigenständig entwickelten,
Word-kompatiblen wissenschaftlichen Textverarbeitung auf jfguiLib. Es ist kein Nachweis,
dass die aufgeführten Funktionen bereits implementiert sind. Im App-Verzeichnis lagen bei
der Bestandsaufnahme ausschließlich die lokalen Referenzbücher. Prioritäten bestimmen die
Umsetzungsreihenfolge, nicht die Streichung von Anforderungen.

Implementierter Ausbau vom 11. Oktober 2026 (jeweils **Teilumfang**, keine Gesamtabnahme):

| Bezug | Vorhanden | Verbleibende Grenze / Nachweis |
|---|---|---|
| UI-001, UI-002, UI-003, UI-005, UI-009 | Start-/Dateiansicht, eigene Vorlagen, lokale letzte/angeheftete Dateien, Befehlssuche, kompakter Kopf und adaptive Menübandgruppen; Quellen/Navigation bei Bedarf | Drucken, Einstellungen, alle Zielregister und andockbare Bereiche fehlen; `shell.uis`, `tests/shell.py`, `ribbon_checks.rs` |
| UI-011, EDIT-003, EDIT-004, VIEW-001, VIEW-003 | Gemeinsame direkte Rich-Text-Eingabe über Format-/Absatz-/Zeilen-/Seitengrenzen desselben Haupttexts oder Notenbereichs, auch in Noten und bei Ausrichtung/Tabulatoren; Zoom und Seitenbreite | Direkte Feldaktionen für Zitate, Bibliografie und Noten; weitere Feldtypen offen; `paper.uis`, `rich.uis`, `rich_pages.uis`, `rich_story.uis`, `rich_fields.uis` |
| EDIT-007, EDIT-008, EDIT-010, Dateikompatibilität §8 | Native Rich-Text-Zwischenablage über Dokumentgrenzen mit Absatzwerten, Quellen und Notenkörpern; RTF-Import/-Export und HTML-Export; atomare Historie, explizites Klartext-Einfügen | HTML-Import, Tabellen/Objekte, volle RTF-Schrift-/Style-Abbildung und externe strukturierte Felder offen; `fragment_checks.rs`, `clipboard_codec_checks.rs`, `rich_clipboard.uis`, `tests/clipboard.py` |
| CITE-001, REF-001, Dateikompatibilität §8 | Zitat-/Notenfeldaktionen direkt auf der Seite; unterstützte absatzlokale komplexe DOCX-Zitatfelder auch in Noten | Unbekannte, verschachtelte, gesperrte oder direkt formatierte Feldresultate im Erhaltungsmodus; `rich_fields.uis`, `rich_field_remove.uis`, `tests/fields.py` |
| EDIT-003, EDIT-004, EDIT-010 | Enter teilt Absätze und ersetzt formatübergreifende Auswahl; Umschalt+Enter fügt Zeilenwechsel ein; Rücktaste/Entf verbinden Absätze im selben Haupttext-/Notenkontext; Undo/Redo stellt Cursor/Auswahl wieder her | Gemeinsame Bereichsänderungen und Absatzbefehle, getrennte Haupttext-/Notenauswahl; `story_edit_checks.rs`, `rich_story.uis`, `rich_checks.rs`, `paragraph_checks.rs`, `paragraphs.uis`, `paragraphs_rich.uis` |
| FMT-001, FMT-002, Dateikompatibilität §8 | Direkte RGB-/Auto-Schriftfarbe und Durchstreichen im Modell, Seitenrenderer und DOCX; optionale Absatzkennungen konfliktfrei vergeben | Themefarben, weitere Schrifteigenschaften und Word-Referenzabnahme offen; `tests/formatting.py`, `tests/rich_text.py` |
| FMT-005, REF-004, Dateikompatibilität §8 | Erstzeilen-/hängender Einzug im Modell, DOCX, Absatzdialog und Seitenlayout, auch in mehrseitigen Fuß-/Endnoten; Absatzbefehle für ganze Auswahlen | Linealbedienung, zeichenbasierte Einzüge und Word-Referenzabnahme offen; `indent_checks.rs`, `indents.uis`, `tests/formatting.py` |
| SEC-006, Dateikompatibilität §8 | Originalarchive fremder gültiger DOCX-Dateien unverändert speichern; zugeordnete Texte in Haupttext, Tabellenzellen, Kopf-/Fußzeilen und normalen Noten austauschen, übrige XML-/Objektteile erhalten | Keine vollständige Darstellung oder Modellbearbeitung, kein DOC/DOCM/Verschlüsselungsprofil; `compat_checks.rs`, `story_checks.rs`, `tests/compatibility.py`, `tests/stories.py`, `preservation.uis`, `stories.uis` |

Weiterer implementierter Teilumfang vom 13. Oktober 2026: **TAB-012** –
[Zeilen zusammenhalten oder über Seiten teilen](docs/TABLE_ROW_BREAKS.md), mit
verschachtelten Zeilenregeln, DOCX/RTF/HTML und nativer/WASM-Bedienung.

Anschließender Teilumfang von **TAB-007**: [Genaue Zeilenhöhe oder Mindesthöhe](docs/EXACT_TABLE_ROW_HEIGHTS.md),
mit begrenzter Darstellung bei vollständig erhaltenem Inhalt, verbundenen/verschachtelten
Zellen, Seitenfortsetzungen und DOCX-/RTF-/HTML-Austausch. Gemeinsamer Eigenschaftsdialog,
Undo und Agentenaktion für Windows, Linux und WASM.

Das [Designziel](DESIGN.md) und der tatsächliche [DOCX-Umfang](README.md#tatsächlicher-docx-umfang)
beschreiben die offenen Arbeiten genauer. Erhaltene Daten sind nicht automatisch darstellbar,
berechenbar oder bearbeitbar.

## Inhalt

1. [Produktziel und verbindliche Entscheidungen](#produktziel)
2. [Quellen und Umgang mit Referenzen](#quellen)
3. [Bestand und Bibliotheksarchitektur](#architektur)
4. [Bedienstruktur und eigener Office-Style](#oberflaeche)
5. [Leseschlüssel des Funktionskatalogs](#leseschluessel)
6. [Funktionskatalog](#funktionen)
7. [Wissenschaftliche Arbeitsabläufe und Datenfluss](#wissenschaft)
8. [Dateiformate und Kompatibilitätsvertrag](#kompatibilitaet)
9. [Eigene Erweiterungsschnittstelle](#erweiterungen)
10. [Rechtliche Einordnung und Gestaltungsänderungen](#recht)
11. [Umsetzungsreihenfolge](#reihenfolge)
12. [Abnahme und Prüfszenarien](#abnahme)
13. [Abgleich mit den Buchabschnitten](#buchabgleich)

<a id="produktziel"></a>
## 1. Produktziel und verbindliche Entscheidungen

- Referenz ist **Word 2024 für Windows**, ergänzt um die in den fünf Fachbüchern
  beschriebenen Microsoft-365-Funktionen. Versionsabhängige Dienste werden gekennzeichnet.
  Die Bücher bilden keinen vollständigen normativen Katalog aller jemals erschienenen
  Word-Versionen; technische Querschnittsanforderungen und Formatstandards ergänzen sie.
- Wissenschaftliche Nutzung ist Kernumfang: Quellenverwaltung, Zitation, Literatur,
  strukturierte Formeln, stabile Nummerierungen, Verweise, Revisionen und lange Dokumente.
  Dafür ist kein Add-on erforderlich.
- Vertraute funktionale Anordnung mit Menüband, Dokumentfläche, Seitenbereichen und
  Statusleiste; eigenständige Gestaltung, Assets und Implementierung.
- Word-Quellenverwaltung plus CSL, BibTeX/BibLaTeX, RIS und CSL-JSON sind verbindlich.
  Hochschul- und Journalvorgaben dürfen eigene Zitierstile verwenden.
- Keine VBA-Laufzeit, kein VBA-Editor, keine VBA-Quelltextbearbeitung. Makrobestandteile
  in geeigneten Dateiformaten werden inaktiv erhalten; eigenes Erweiterungssystem.
- Der Desktop-Kern arbeitet lokal und ohne Benutzerkonto. Zusammenarbeit, KI, Übersetzung,
  Onlinerecherche und andere Dienste gehören zum Zielumfang in späteren Ausbaustufen.
- Windows und Linux mit Wayland/X11 über die vorhandene Plattformschicht. Keine zweite
  Browser-/Web-GUI, keine Abhängigkeit von installiertem Microsoft Office zur Laufzeit.
- Rust mit `#![no_std]`, `core`/`alloc`, ohne Cargo und Fremd-Crates. Builds über
  `../../build_rs.py`; Ausgaben unter `../../build/`. Wissenschaftliche Engines,
  Formatparser und weitere Grundlagen werden eigene Bibliotheksmodule.
- Bücher, Originalgrafiken und extrahierte Buchtexte bleiben lokale Referenzen und werden
  nicht eingecheckt. Code unter `legacy/` bleibt unverändert. Bestehende Änderungen anderer
  Arbeiten werden nicht zurückgesetzt.
- `jfWord` ist der interne Arbeitsname, `jfoffice` die technische Bezeichnung des geplanten
  Styles. Eine öffentliche Markenfreigabe wird damit nicht behauptet.
- **100 % Dateikompatibilität ist das Ziel**, nicht eine schon erfüllte Zusage. Es umfasst
  definierte Formate und Versionen; Abweichungen müssen sichtbar und messbar sein.
  VBA-Ausführung und fremde Add-on-Laufzeiten bleiben ausdrücklich ausgeschlossen.

<a id="quellen"></a>
## 2. Quellen und Umgang mit Referenzen

### 2.1 Lokale Referenzen

Alle sechs Dateien liegen in `books/` und sind durch die Repository-Regeln git-ignoriert.
Die Kurzkennungen dienen den Featurezeilen und dem Abschnittsabgleich. Titel werden nur
zur Identifikation genannt; Anforderungen sind eigenständig formuliert.

| Kennung | Referenz | Fundstellenkonvention |
|---|---|---|
| T | G. O. Tuhls: Wissenschaftliche Arbeiten schreiben mit Microsoft Word 365, 2024, 2021, 2019, EPUB | Kapitel/Abschnitt, z. B. T 5.6.2; EPUB-Anker `u05.6.2` |
| B | Inge Baumeister / Christian Immler: Word 2024 Stufe 1 Grundlagen, PDF, 371 PDF-Seiten | Nummerierte Kapitel/Abschnitte; bei Seitenangaben gedruckte Seiten |
| P | Dimitar Popmintchev: Engineering and Scientific Manuscripts with Microsoft Word 2024, PDF, 223 PDF-Seiten | Kapitel/Abschnitt; gedruckte Seiten nach römischem Vorspann |
| S | Charles Sherer: MICROSOFT WORD 2024, EPUB | Kapitel 1-6; EPUB-Abschnittsanker im Abgleich |
| E | Eric Edward: Become a Microsoft Word 2024 Pro in 7 Days, PDF, 68 PDF-Seiten | PDF-Seiten, da die Inhaltsübersicht keine Kapitelnummern verwendet |
| L | Executive Summary-1.pdf, 7 PDF-Seiten | PDF-Seiten; rechtliches Ausgangsmemo mit nicht auflösbaren internen Zitiermarken |
| U | Direkt bestätigte Nutzeranforderungen vom 2026-10-10 | Projektvorgabe, keine Behauptung über Word |
| A | Aus Architektur, Datenintegrität und Prüfbarkeit abgeleitete Anforderung | Eigene Ergänzung, keine Buchbehauptung |

Methodik: vollständige Inhaltsübersichten der Bücher als Abdeckungsgerüst; Detailauswertung
insbesondere für Quellen, Felder, Formeln, Dokumentverbünde, Tabellen, Ausgabe und
Bedienwege. Pädagogische Wiederholungen und Übungen erzeugen keine doppelten Features.
Fehler, Meinungsaussagen und versionsabhängige Bedienpfade aus Büchern werden nicht zu
normativen technischen Wahrheiten erhoben. Die Abschnittsmatrix dokumentiert die Zuordnung,
nicht die Implementierung oder eine pixelgenaue Rekonstruktion der Buchabbildungen.

### 2.2 Primärquellen und Spezifikationen

Abruf-/Prüfstand für die verlinkten Grundlagen: 2026-10-10. Für spätere Implementierungen
werden tatsächlich verwendete Spezifikationsrevision, Testkorpus und Referenzbuild separat
festgehalten. Eine neuere Spezifikationsrevision erweitert nicht automatisch den Produktumfang.

| Kennung | Primärquelle | Verwendung |
|---|---|---|
| W-OOXML | [ECMA-376](https://ecma-international.org/publications-and-standards/standards/ecma-376/) | WordprocessingML, OPC, Markup Compatibility, Transitional |
| W-DOCX | [MS-DOCX](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-docx/b839fe1f-e1ca-4fa6-8c26-5954d0abbccd) | Word-Erweiterungen zu OOXML; Standard allein genügt nicht |
| W-FIELDS | [Word-Feldübersicht](https://support.microsoft.com/en-us/word/list-of-field-codes-in-word) | Zusätzlicher Abgleich der dokumentierten Feldfamilien |
| W-2024 | [Word-2024-Neuerungen](https://support.microsoft.com/en-us/word/what-s-new-in-word-2024-for-windows-and-mac) | Sitzungswiederherstellung, Bearbeitungsmodi, erledigte Kommentare und aktuelle ODF-Angaben; Build-/Editionsunterschiede beachten |
| W-FORMAT | [Microsoft Office File Formats](https://learn.microsoft.com/en-us/openspecs/office_file_formats/) | Einstieg zu MS-DOC, MS-CFB, MS-OFFCRYPTO und weiteren Formatspezifikationen |
| W-BIB | [Create Custom Bibliography Styles](https://learn.microsoft.com/en-us/office/vba/word/Concepts/Objects-Properties-Methods/create-custom-bibliography-styles) | Quellen-XML und XSL-basierte Word-Zitierausgabe; keine VBA-Laufzeit nötig, um das Dateimodell zu implementieren |
| W-CSL | [CSL 1.0.2](https://docs.citationstyles.org/en/stable/specification.html) | Stilmodell, Lokalisierung, Disambiguierung, Zitat-/Literaturformatierung |
| W-OSP | [Microsoft Open Specification Promise](https://learn.microsoft.com/en-us/openspecs/dev_center/ms-devcentlp/1c24c7c8-28b0-4ce1-a47d-95fe1ff504bc) | Umfang und Grenzen des Patentversprechens, einschließlich DOC und OVBA |
| W-COPY | [§ 69a UrhG](https://www.gesetze-im-internet.de/urhg/__69a.html) | Schutz von Programmausdruck gegenüber Ideen/Grundsätzen |
| W-SAS | [EuGH C-406/10](https://eur-lex.europa.eu/legal-content/EN/TXT/PDF/?uri=celex%3A62010CJ0406) | Funktionalität, Sprache, Dateiformat und Schutz von Handbuchinhalten |
| W-GUI | [EuGH C-393/09](https://eur-lex.europa.eu/legal-content/en/TXT/?uri=CELEX%3A62009CJ0393) | Möglicher eigenständiger urheberrechtlicher Schutz einer GUI |
| W-UWG | [§ 4 UWG](https://www.gesetze-im-internet.de/uwg_2004/__4.html) | Nachahmung und Herkunftstäuschung, insbesondere Nummer 3 |
| W-FONT | [Microsoft Font Redistribution FAQ](https://learn.microsoft.com/en-us/typography/fonts/font-faq) | Nutzung installierter Fonts, Dokumenteinbettung und Weitergabe auseinanderhalten |

Weitere normative Detailquellen für ODF, PDF, Unicode, MathML, BibLaTeX und RIS sind beim
jeweiligen Codec-Meilenstein mit Revision festzuschreiben. Diese Spezifikation behauptet
keine bereits durchgeführte Konformitätsprüfung dieser Implementierungen.

<a id="architektur"></a>
## 3. Bestand und Bibliotheksarchitektur

### 3.1 Verifizierter Ausgangsbestand

| Bestand | Nachweis im Repository | Verwendbar als / noch fehlend |
|---|---|---|
| App-Skelett und Immediate-Mode-UI | `../template/main.rs`, `../gallery/`, `../../docs/GUIDE.md` | App-Lebenszyklus, Commands, Widgets, Headless-Skripte; kein Dokumenteditor |
| Stylevertrag | `../../lib/ui/style.rs`, `../../docs/STYLES.md` | Tokens, Fontrollen, Größen-/Zeichenmethoden; Menüband und Dokumentlineale ergänzen |
| Text/Fonts | `../../lib/text/lib.rs`, `ttf.rs`, `fontdb.rs` | Glyphen, Messung, Hit-Tests, Fontregister, eingeschränktes Kerning; vollständiges Shaping, Bidi, Absatz-/Seitensatz neu |
| Textbox | `../../lib/ui/edit.rs` | Eingabe-/Zwischenablagegrundlage; kein Rich-Text-Modell, keine akademische Seitendarstellung |
| Raster/Vektoren/Bilder | `../../lib/gfx/`, `../../lib/img_core/io.rs`, `../../lib/img_*` | Oberflächen, Pfade, vorhandene Bildcodecs und Registry; kein DOCX-Codec und kein vollständiger SVG-Dokumentparser |
| Plattform | `../../lib/plat/` | Dateien, Fenster, Threads, Clipboard, Stift und Druck; Dokument-Clipboard und Zugänglichkeitsbrücken ausbauen |
| Druck/PDF | `../../lib/plat/print.rs` | Bildorientierte Druck-/PDF-Pfade; kein Nachweis für mehrseitiges, strukturiertes, durchsuchbares Dokument-PDF |
| Plugins/Module | `../../lib/plugin/`, `../../plugins/README.md`, `../../lib/plat/modabi.rs` | PNAPI und bestehender Lader als Referenz; keine jfWord-API, keine nachgewiesene Sandbox |
| Build | `../../build_rs.py` | Explizite Bibliotheksabhängigkeiten, App-Erkennung über `main.rs`, Style-Erkennung über `lib.rs`; neue Libs registrieren |
| jfWord | Dieses Verzeichnis | Nur Referenzmaterial vor Phase 1; kein implementiertes Feature anzunehmen |

Bestandsstatus: **V** = genau diese Grundlage vorhanden; **E** = vorhandenen Baustein
erweitern; **N** = neue Fähigkeit. Eine Featurezeile ist immer geplant. V bedeutet niemals,
dass die komplette jfWord-Funktion bereits existiert.

### 3.2 Zuständigkeiten und Abhängigkeiten

Die Kürzel sind feste Architekturgrenzen dieser Spezifikation, keine bereits existierenden
Rust-Crates oder endgültigen ABI-Symbole. Neue Komponenten leben in der Bibliothek,
sobald sie ohne jfWord-Bedienlogik nutzbar sind. Die UI darf keine Voraussetzung für einen
Formatdecoder, Zitierprozessor oder Layouttest sein.

| Kürzel | Zielkomponente / Zustand | Aufgaben und direkte Grundlagen |
|---|---|---|
| BAS | Basis/Serialisierung E+N | UTF-8/16, Codepages, XML mit Namespaces, JSON, ZIP/OPC, CFB, Ressourcenlimits; `base`, `rt`, vorhandene Kompressionsalgorithmen wiederverwenden |
| DOC | Dokumentkern N | Typisierte Inhalte, stabile IDs, Abschnitte, Stories, Styles, Beziehungen, Range-Anker, Transaktionen, Undo; BAS |
| IO | Dokumentcodecs N | Format-Erkennung, Import/Export, Ressourcen und Erhalt unbekannter Bestandteile; BAS, DOC; keine Fenster |
| TXT | Schrift-/Sprachgrundlagen E+N | Unicode-Segmentierung, Shaping, Bidi, Fontfallback, Hyphenation, Wörterbücher; `text`, BAS |
| LAY | Textsatz N | Zeilen, Absätze, Seiten, Spalten, Tabellen, Anker, Fußnoten; DOC, TXT, Grafikgrundlagen |
| BIB | Bibliografie N | Quellenmodell, Austausch, CSL, Zitatcluster, Sortierung/Disambiguierung; BAS, DOC-Feldanbindung |
| FLD | Felder/Referenzen N | Feldsyntax, Werte, Abhängigkeitsgraph, Nummerierung, Verzeichnisse; DOC, BIB, LAY-Rückmeldungen |
| MAT | Mathematik N | Formelbaum, Parser, Layout, OMML/MathML; BAS, TXT, LAY-Anbindung |
| GFX | Dokumentgrafik E+N | Bild-/Vektorressourcen, Formen, Diagramm-/Zeichnungsmodelle; vorhandene gfx/img-Module |
| GUI | Wiederverwendbare GUI E+N | Menüband, Galerien, Lineale, Dokumentoberfläche, semantische Zugänglichkeit; ui, TXT, LAY |
| STY | Eigenständiger Style N | Aussehen und Größen; bestehender Stylevertrag, eigene/verteilbare Assets |
| PLAT | Betriebssystem E | Datei-/Druck-/Clipboard-/IME-/Zugänglichkeitsadapter; keine Dokumentsemantik |
| OUT | Dokumentausgabe N | Mehrseitiges Vektor-/Text-PDF, Tags, Schriftuntergruppen und Drucklisten; DOC, LAY, GFX, PLAT |
| SEC | Sicherheit E+N | Sichere Parsergrenzen, Dokumentkrypto, Vertrauenszustände, Signaturprüfung; BAS, PLAT |
| EXT | Erweiterungssystem N | Eigene API, Paketprüfung, Hostrechte, Isolation und Anbieterprotokolle; DOC, BAS, SEC, PLAT |
| SVC | Optionale Dienste N | Netz, Authentifizierung, Synchronisation, Anbieter für Sprach-/KI-/Metadatendienste; SEC, EXT |
| APP | jfWord N | Benutzerabläufe, Commands, Auswahlkontext, Dokumentfenster, Einstellungen; verwendet obige APIs |

Datenfluss: Datei -> BAS/IO -> DOC -> TXT/LAY -> GUI oder OUT. Dokumentänderungen laufen
als Transaktionen in DOC; BIB/FLD/MAT liefern strukturierte Inhalte, nicht bloß gerasterte
Darstellungen. APP verdrahtet die Funktionen. STY zeichnet Bedienelemente und verändert
keine Dokumentsemantik. SVC und EXT arbeiten über kontrollierte Aufträge/Transaktionen.

### 3.3 Geplante öffentliche Schnittstellen

Phase 1 ändert keine Rust-API. Für die Umsetzung sind diese Verträge erforderlich:

- **Codec:** Erkennen, Prüfen, Lesen, Schreiben; Optionen, Abbruch, Limits, Warnungen mit
  Fundstelle, Formatfähigkeiten und Import-/Exportbericht. Fehler erzeugen kein halbes Dokument.
- **Dokument:** Dokument-/Objekt-/Quellen-IDs, stabile Bereiche, atomare Änderungspakete,
  Ereignisse und Snapshots. Kein app-spezifischer globaler Zustand in der Bibliothek.
- **Layout:** versionierter Snapshot, Seiten-/Zeilen-/Glyphenpositionen, Hit-Test,
  Navigation und Invalidierung. Veraltete Worker-Ergebnisse ersetzen keinen neueren Stand.
- **Bibliografie:** Quellenbibliothek, Dokumentquellen, Zitiercluster, Stil/Locale,
  gerenderte strukturierte Runs und Diagnostik. Datenmodell unabhängig vom Word-XML-Codec.
- **Felder:** Originalcode, geparste Struktur, zwischengespeichertes Ergebnis,
  Aktualisierungszustand und Rückverweise auf abhängige Bereiche.
- **Ausgabe:** dieselbe Layoutquelle für Bildschirm, Druck und PDF; keine Screenshot-PDFs
  als Ersatz für Text, Suchbarkeit, mathematische Struktur und Barrierefreiheit.
- **GUI/Style:** neue Controls besitzen semantische IDs und Testzugänge. Größen, Eingabe und
  Zeichnung bleiben konsistent; bestehende Styles und Apps müssen weiter funktionieren.
- **Erweiterungen:** versioniertes eigenes Nachrichten-/Hostprotokoll; keine Rust-internen
  Speicherlayouts, Referenzen oder `Vec` über eine unversionierte Fremdgrenze exportieren.

Bestehende Safe-/Unsafe-Grenzen gelten weiterhin. Parser, Satz, Bibliografie und App bleiben
safe. Neue Plattformoperationen gehören in die bereits dafür vorgesehenen Schichten.

<a id="oberflaeche"></a>
## 4. Bedienstruktur und eigener Office-Style

### 4.1 Fensteraufbau

Von oben nach unten: Titelleiste mit Dateiname/Speicherzustand und Schnellzugriff;
Registerleiste mit Befehlssuche; Menüband; Dokumentarbeitsfläche mit optionaler Navigation
links und aufgabenbezogenen Bereichen rechts; Statusleiste mit Seite, Wörtern, Sprache,
Prüfstatus, Ansicht und Zoom. Horizontales/vertikales Lineal folgen der Dokumentausrichtung.
Seiten bleiben von der Farbe der Anwendung unabhängig. Ein dunkler Arbeitsbereich darf
nicht versehentlich schwarze Druckseiten erzeugen.

| Register / Zugang | Gruppen -> Dialoge und Seitenbereiche |
|---|---|
| Datei | Neu/Vorlagen, Öffnen, Zuletzt, Speichern/Speichern unter, Informationen, Export, Drucken, Einstellungen, Schließen |
| Start | Zwischenablage, Schrift, Absatz, Listen, Formatvorlagen, Suchen/Ersetzen/Auswählen |
| Einfügen | Seiten, Tabellen, Illustrationen, Links/Textmarken, Kopf-/Fußzeile, Textbausteine/Felder, Symbole/Formeln, Objekte |
| Zeichnen | Stifte, Radierer, Auswahl, Umwandlung, Zeichenbereich |
| Entwurf | Dokumentdesign, Schrift-/Farbsätze, Absatzsätze, Wasserzeichen, Seitenfarbe/-rahmen |
| Layout | Seite/Abschnitt, Ränder, Spalten, Umbrüche, Nummerierung, Silbentrennung, Einzüge/Abstände, Anordnen |
| Referenzen | Inhaltsverzeichnis, Fuß-/Endnoten, Quellen/Zitate/Literatur, Beschriftungen, Querverweise, Index, Rechtsgrundlagen, Glossar/Abkürzungen |
| Sendungen | Dokumenttyp, Datenquelle/Empfänger, Felder/Regeln, Vorschau, Zusammenführen, Umschläge/Etiketten |
| Überprüfen | Sprache/Korrektur, Dokumentstatistik, Barrierefreiheit, Kommentare, Änderungen, Vergleichen, Schutz |
| Ansicht | Dokumentansichten, Navigation/Lineale, Zoom, Fenster/Teilung, Fokus, Dokumentverbund |
| Entwickeln | Formularfelder, Datenbindung, eigene Aktionen, Erweiterungsverwaltung; standardmäßig ausblendbar, kein VBA |
| Hilfe | Offlinehilfe, Tastaturübersicht, Diagnose, Version, Lizenzen; keine Microsoft-Kontoseite |
| Kontext: Tabelle | Tabellenentwurf und Tabellenlayout; Zell-/Zeilen-/Spaltenoperationen |
| Kontext: Bild/Form/Diagramm | Format, Größe, Umbruch, Effekte, Daten/Struktur, Alternativtext |
| Kontext: Formel | Strukturen, Symbole, Eingabesyntax, Konvertierung, Ausrichtung |
| Kontext: Kopf-/Fußzeile | Position, Varianten, Verknüpfung mit vorherigem Abschnitt, Navigation |
| Kontext: Zitat/Feld | Quelle, Fundstelle, Darstellung, Code/Ergebnis, Aktualisierung |

Schnellzugriff, Kontextmenü, Tastatur und Befehlssuche verweisen auf dieselben Commands.
Bei kleiner Breite klappen Gruppen in beschriftete Menüs zusammen. Funktionen verschwinden
nicht ersatzlos. Dialoge bewahren Eingaben bei Validierungsfehlern. Modale Bestätigungen
werden auf Datenverlust, irreversible Aktionen und bewusst erteilte Dienstrechte beschränkt.

### 4.2 Stilvorgabe `jfoffice`

Eigenständige, sachliche Bürooberfläche: kompakte beschriftete Gruppen, ruhige Flächen,
dezente Gruppengrenzen, präzise Fokusrahmen und eine petrolfarbene Akzentfamilie. Startwerte
für die spätere visuelle Ausarbeitung: helles Umfeld `#F3F5F5`, weiße Bedienelementflächen,
Text `#202A30`, Akzent `#176B70`; dunkel `#20262C`, Flächen `#2A333B`, Text `#EEF3F5`,
Akzent `#77C8C8`. Farben sind vor Abnahme auf Kontrast und Zustände zu prüfen.

Abstände auf einem 4-DIP-Raster, zurückhaltende 4-DIP-Ecken, Standardtext 14 DIP,
beschriftete große Befehle und kompakte Nebenbefehle. Für Touch größere Ziele anbieten.
Keine Übernahme von Microsoft-Logos, Fluent-Icon-Fontdateien, proprietären Vorlagengalerien
oder charakteristischen Illustrationen. Eigene semantische Icons decken bestehende und
neue `IC_*`-Rollen ab. Frei weitergebbare UI-/Mathematikfonts einschließlich Lizenzdateien
werden als eigene Assets ausgewählt; Systemschriften können zusätzlich verwendet werden.

Die Styleumsetzung umfasst alle Widgetfamilien, Hell/Dunkel, Hover, Druckzustand,
Auswahl, Fokus, deaktivierte Zustände, Fehler, hohe Skalierung und Kontrastmodus.
Anwendungsstyle und im Dokument gespeichertes Design sind voneinander unabhängig.

<a id="leseschluessel"></a>
## 5. Leseschlüssel des Funktionskatalogs

Jede Zeile ist eine Anforderung mit stabiler ID. Unterfunktionen in einer Zeile teilen
denselben Daten- und Prüfvorgang; beim Implementieren darf keine davon still entfallen.
Die Tabelle legt Zweck/Aufbau, Bedienort, Eigentümer/Abhängigkeiten, Austausch und Abnahme fest.
`+` in der Architekturspalte bedeutet Zusammenarbeit, der erste Eintrag besitzt die Semantik.
Die Bestandszustände ergeben sich aus Abschnitt 3; App-Funktionen sind durchgehend geplant.

Austauschcodes gelten zusätzlich zur formatspezifischen Matrix in Abschnitt 8:

- **D:** als editierbare Dokumentstruktur erhalten; Verhalten und Layout gesondert prüfen.
- **F:** Feldcode, Quellen-/Zielbezug und Ergebnis erhalten; Aktualisierung separat prüfen.
- **R:** Ressource samt Beziehungen erhalten; Vorschau allein ist keine Editierbarkeit.
- **M:** Metadaten/Revisionen/Schutzinformationen erhalten, formatbedingte Verluste melden.
- **P:** lokale Programmeinstellung; verändert keine fremden Dokumentdateien.
- **X:** kontrollierter Import/Export oder Dienst-/Erweiterungsgrenze; Abbildungsbericht nötig.

Universelle Abnahme: Erstellen, Ändern, Undo/Redo und Speichern/Öffnen müssen für persistente
Funktionen funktionieren; Tastaturbedienung, Mehrsprachigkeit und Zugänglichkeit gehören zur
Bedienabnahme. Die jeweilige Zeile nennt darüber hinaus den kennzeichnenden Prüffall.
Fehlende Formatfähigkeiten werden gemeldet; unbekannte Daten dürfen nicht still verschwinden.

<a id="funktionen"></a>
## 6. Funktionskatalog

### 6.1 Anwendung und Bedienung

| ID | Funktion und Aufbau | Bedienort | Architektur | Austausch | Kennzeichnende Abnahme | Quelle |
|---|---|---|---|---|---|---|
| UI-001 | Startansicht mit leerem Dokument, eigenen Vorlagen, zuletzt verwendeten und angehefteten Dateien | Start / Datei | APP+GUI | P | Ohne Konto ein neues Dokument erstellen | B 1, 3; S 1 |
| UI-002 | Dokumentfenster mit Dateiname, Pfad, Änderungs-/Speicherstatus und eindeutigem aktiven Dokument | Titelleiste | APP+GUI | P | Zwei gleichnamige Dateien unterscheiden | B 1.3, 3.7 |
| UI-003 | Menüband mit Registern, Gruppen, Dialogstartern, geteilten Schaltflächen und Überlauf | Registerleiste | GUI+APP | P | Alle Befehle auch bei schmalem Fenster erreichbar | B 1.4; T Einleitung |
| UI-004 | Kontextregister abhängig von Auswahl und Bearbeitungsmodus | Über der Dokumentfläche | APP+GUI | P | Wechsel Tabelle -> Formel -> Text zeigt richtige Befehle | B 7-9; T 15-17 |
| UI-005 | Anpassbarer Schnellzugriff mit Reihenfolge und Wiederherstellung | Titelleiste / Einstellungen | GUI+APP | P | Gespeicherte Anordnung nach Neustart erhalten | B 1.4; P 1.3 |
| UI-006 | Kontextmenüs und abschaltbare Miniwerkzeugleiste für Auswahl und Objekte | Rechtsklick / Auswahl | GUI+APP | P | Tastatur-Kontextmenü bietet dieselben Aktionen | B 1.4; E S. 19-25 |
| UI-007 | Befehlssuche mit Synonymen, aktivem Kontext und verständlichen deaktivierten Zuständen | Obere Suchleiste | GUI+APP | P | Suche nach Quellen führt direkt zur Verwaltung | T 21.1; B 1 |
| UI-008 | Tastaturkürzel, konfliktgeprüfte Anpassung, Zugriffstasten und Menübandnavigation | Einstellungen / Tastatur | GUI+APP | P | Alle Kernabläufe ohne Maus; Konflikt vor Neubelegung anzeigen | P 1.3, 16; U |
| UI-009 | Andockbare Bereiche für Navigation, Styles, Quellen, Kommentare, Auswahl und Formatierung | Ansicht / Seitenbereiche | GUI+APP | P | Bereiche schließen/wiederherstellen ohne Dokumentänderung | T 6, 8, 23 |
| UI-010 | Statusleiste mit Seiten, Wörtern, Sprache, Änderungs-/Prüfstatus und Zoom | Fensterfuß | APP+LAY+GUI | P | Statistik folgt aktiver Auswahl und aktuellem Layout | B 2.11; T 20.1 |
| UI-011 | Live-Vorschau in Galerien mit Abbruch und atomarer Übernahme | Style-/Formatgalerien | GUI+DOC | D | Hover erzeugt keinen permanenten Undo-Schritt | B 4, 6; A |
| UI-012 | Einstellungen für Anzeige, Bearbeitung, Speichern, Sprache, Schutz und Erweiterungen | Datei / Einstellungen | APP+GUI | P | Ungültige Eingaben mit Feldhinweis; bisherige Werte bleiben nutzbar | B 3.9; T 7 |
| UI-013 | Offlinehilfe, kontextbezogene Erklärung und Kürzelübersicht | Hilfe / F1 | APP+GUI | P | Kernhilfe ohne Internet erreichbar | B 1.4; E S. 67-68 |
| UI-014 | Dokumentfenster wechseln, nebeneinander anordnen und synchron scrollen | Ansicht / Fenster | APP+GUI+LAY | P | Unterschiedliche Dokumentlängen verursachen keinen Positionsverlust | B 3.7; T 23.4 |
| UI-015 | Versions-/Lizenzinformationen und exportierbare, inhaltsarme Diagnose | Hilfe / Informationen | APP+PLAT | X | Diagnose kopierbar, ohne automatisch Dokumenttext zu übertragen | A; U |
| UI-016 | Eigenständiger vollständiger Office-Style mit Hell/Dunkel, DPI und Touchdichte | Einstellungen / Darstellung | STY+GUI | P | Gallery und Dokumentfenster bei 100/150/200 % bedienbar | U; Repository STYLES |
| UI-017 | Eigene Register/Gruppen/Befehle im Menüband anordnen, benennen, ausblenden und zurücksetzen | Einstellungen / Menüband | GUI+APP | P | Pflichtzugänge bleiben über Befehlssuche erreichbar | T Anhang B.3 |
| UI-018 | Menüband-, Schnellzugriff- und Tastaturprofile importieren/exportieren | Einstellungen / Anpassungen | APP+BAS | P | Unbekannte Befehle melden; bestehendes Profil bei Fehler erhalten | T Anhang B.1-B.3 |
| UI-019 | Eindeutiger Wechsel zwischen Anzeigen, Überprüfen und Bearbeiten | Dokumentmodus / Titelleiste | APP+DOC+SEC | P | Moduswechsel respektiert Rechte und Revisionsvorgaben | W-2024; U |

### 6.2 Dokumentverwaltung und Dateiarbeit

| ID | Funktion und Aufbau | Bedienort | Architektur | Austausch | Kennzeichnende Abnahme | Quelle |
|---|---|---|---|---|---|---|
| FILE-001 | Leeres Dokument und Dokument aus Vorlage mit eigener Dokumentidentität | Datei / Neu | DOC+APP | D | Vorlage bleibt beim Erstellen unverändert | B 3.2, 10; S 3 |
| FILE-002 | Öffnen per Dialog, Dateipfad, Drag-and-drop und Shell-Zuordnung | Datei / Öffnen | IO+PLAT+APP | X | Format an Inhalt prüfen, nicht nur Endung | B 3.4; T 22.2 |
| FILE-003 | Speichern, Speichern unter, Kopie und Formatwahl mit Verlustbericht | Datei / Speichern | IO+DOC+APP | X | Fehlgeschlagener Schreibvorgang lässt vorherige Datei intakt | B 3.3, 3.8; A |
| FILE-004 | Zuletzt verwendet, Anheften und Wiederaufnahme an letzter Bearbeitungsstelle | Datei / Zuletzt | APP+DOC | P | Verschobene/gelöschte Datei verständlich behandeln | B 3.4; T 22.2.1 |
| FILE-005 | Änderungszustand, explizites Verwerfen und Rückkehr zur gespeicherten Fassung | Datei / Schließen | DOC+APP | D | Abgebrochenes Schließen erhält alle Änderungen | B 3; A |
| FILE-006 | Lokale automatische Wiederherstellung mit Zeitintervall und Journal/Snapshot | Einstellungen / Speichern | DOC+IO+PLAT | X | Wiederherstellbare Fassung nach Prozessabbruch anbieten | B 3.5; T 22.1.3 |
| FILE-007 | Wiederherstellungsübersicht für ungespeicherte und beschädigte Sitzungen | Start / Wiederherstellen | APP+IO+DOC | X | Mehrere Kandidaten vergleichen, Originale nicht überschreiben | B 3.5; T 22.2.3 |
| FILE-008 | Versionen und lokale Sicherungskopien mit Datum und Wiederherstellung | Datei / Versionen | DOC+IO | M | Älteren Stand als Kopie öffnen | T 22.1.4; U |
| FILE-009 | Schreibschutz, Mehrfachzugriff und externe Dateiänderungen erkennen | Öffnen / Speichern | IO+PLAT+APP | X | Externe Änderung löst Konfliktwahl statt stillen Überschreibens aus | T 22.2.2 |
| FILE-010 | Dokumenteigenschaften: Titel, Autor, Stichwörter, benutzerdefinierte Werte | Datei / Informationen | DOC+IO | M | Typen und Unicode-Werte nach Roundtrip erhalten | B 2.11; T 24.2.2 |
| FILE-011 | Vorlage zuordnen, Styleabgleich steuern und Vorlagen verwalten | Datei / Vorlagen | DOC+APP+IO | D | Gleichnamige Styles kontrolliert zusammenführen | T 6.4; B 10 |
| FILE-012 | Text oder Dokument an Einfügestelle importieren, Style-/Listen-Konflikte auflösen | Einfügen / Text aus Datei | IO+DOC | D | Nummerierungen und Quellenkennungen kollidieren nicht | T 10, 14.6; S 1 |
| FILE-013 | Sicheres Reparaturöffnen mit Diagnose und salvagiertem Dokument als Kopie | Öffnen / Reparieren | IO+SEC | X | Defekte Teile benennen; keine Vollständigkeit vortäuschen | T 22.2.3; A |
| FILE-014 | Dokumentverbund aus Zentral-/Teildokumenten mit Status und Sperren | Ansicht / Gliederung | DOC+FLD+IO | D | Änderungen eines Teilkapitels ohne Beschädigung der anderen | T 22.3.1 |
| FILE-015 | Verknüpfte Textübernahme mit relativen Pfaden und Textmarkenbereich | Einfügen / Verknüpfter Text | FLD+IO+DOC | F | Fehlende Quelldatei zeigt Status und erhält letztes Ergebnis | T 22.3.2 |
| FILE-016 | Globale Normalvorlage und persönliche Defaults ohne VBA-Abhängigkeit | Datei / Vorlagen | DOC+APP | P | Eigene Defaults ohne Änderung fremder Vorlagen | T 6.4.5; U |
| FILE-017 | Dokumentfonts einbetten, Untergruppen wählen und fehlende Schriften zuordnen | Datei / Speichern / Schriften | IO+TXT+DOC | R | Einbettungsrechte prüfen; fehlende Glyphen und Layoutabweichungen melden | T 23.1, 24.2.1 |
| FILE-018 | Gesamte Dokumentensitzung nach unerwartetem Ende wiederaufnehmen | Start / Sitzung wiederherstellen | APP+DOC+IO | P | Alle vorher offenen Dokumente mit Wiederherstellungsstand zuordnen | W-2024 |

### 6.3 Eingabe, Auswahl, Zwischenablage und Suche

| ID | Funktion und Aufbau | Bedienort | Architektur | Austausch | Kennzeichnende Abnahme | Quelle |
|---|---|---|---|---|---|---|
| EDIT-001 | Unicode-Eingabe, kombinierende Zeichen und graphemgerechtes Löschen | Dokument | DOC+TXT+GUI | D | Emoji-/Akzentsequenzen nicht durch Backspace beschädigen | T 7; A |
| EDIT-002 | IME-Komposition, Sprachumschaltung und Kandidatenposition | Dokument | PLAT+TXT+GUI | D | Komposition abbrechen ohne halbe Zeichen im Dokument | B 2; A |
| EDIT-003 | Cursorbewegung nach Zeichen, Wort, Zeile, Absatz, Seite und Dokument | Dokument / Tastatur | DOC+LAY+GUI | D | Visuelle Bewegung bei Bidi und Umbrüchen korrekt | T 8.1; B 2.3 |
| EDIT-004 | Maus-/Tastaturauswahl, Auswahlmodus, Wort-/Absatz-/Gesamtauswahl | Dokument / Auswählen | DOC+GUI | D | Auswahl über Seiten und Tabellen reproduzierbar | T 8.2; B 2.5 |
| EDIT-005 | Rechteckige und nicht zusammenhängende Auswahl sowie Formatauswahl | Start / Auswählen | DOC+LAY+GUI | D | Mehrere Bereiche gemeinsam formatieren und zurücknehmen | T 8.2.1, 8.2.4 |
| EDIT-006 | Auswahl per Touch, Ziehpunkte und automatische Scrollbewegung | Dokument | GUI+LAY+PLAT | D | Auswahl über Bildschirmrand hinaus bedienbar | T 8.2.5; B 1.4 |
| EDIT-007 | Ausschneiden/Kopieren mit Text, Rich Text, HTML, Bildern und internen Strukturen | Start / Zwischenablage | DOC+IO+PLAT | X | Tabellen und Fußnoten intern strukturerhaltend kopieren | T 10.3; B 2.9 |
| EDIT-008 | Einfügeoptionen: Quelle beibehalten, Zielformat, Format kombinieren, nur Text | Einfügeauswahl | DOC+IO+APP | D | Identische Style-/Listen-IDs kontrolliert abbilden | T 10.3, 10.6; B 2.9 |
| EDIT-009 | Inhalte einfügen: verknüpft, eingebettet, Bild oder unformatierter Inhalt | Start / Inhalte einfügen | IO+DOC | X | Quelle und aktive Verknüpfung nachvollziehbar | T 10.3.2, 15.5 |
| EDIT-010 | Eigene Mehrfachelement-Zwischenablage mit Vorschau und Löschung | Zwischenablagebereich | APP+DOC+PLAT | P | Kein unerwartetes dauerhaftes Speichern sensibler Inhalte | B 2.9; E S. 21-22 |
| EDIT-011 | Drag-and-drop innerhalb/zwischen Dokumenten; Kopieren/Verschieben | Dokument | DOC+GUI | D | Quellen und Zielbereiche bleiben nach Undo konsistent | T 8.3, 10.2 |
| EDIT-012 | Unbegrenzte logisch gruppierte Undo-/Redo-Historie innerhalb konfigurierbarer Speichergrenzen | Schnellzugriff | DOC+APP | D | Mehrteiliger Vorgang als ein Schritt; Grenze transparent | B 2.4; A |
| EDIT-013 | Letzte geeignete Aktion wiederholen, auch Formatübertragung | Start / Wiederholen | DOC+APP | D | Ungültigen Kontext ohne Dokumentmutation melden | B 4.7 |
| EDIT-014 | Sichtbare Steuerzeichen: Leerraum, Absatz, Tab, Umbruch, versteckter Text | Start / Absatz | GUI+LAY+DOC | P | Anzeige ändert Druck- und Exportinhalt nicht | T 7.2; B 2.2 |
| EDIT-015 | Sonderzeichenpalette, Unicode-Codeeingabe, geschützte Leer-/Trennzeichen | Einfügen / Symbol | TXT+DOC+GUI | D | Unicodewert und Schriftabdeckung sichtbar | T 7.3-7.6; P 8 |
| EDIT-016 | Inkrementelle Textsuche mit Trefferliste, Kontext, Großschreibung und ganzen Wörtern | Navigation / Suchen | DOC+TXT+APP | D | Treffer in Fußnoten, Feldern und Textfeldern auswählbar | T 13.1-13.2 |
| EDIT-017 | Ersetzen einzeln/gesamt nach Text, Format, Sonder-/Steuerzeichen | Start / Ersetzen | DOC+TXT | D | Alles-Ersetzen atomar rückgängig; Bereich klar definiert | T 13.3; B 2.10 |
| EDIT-018 | Word-artige Mustersuche mit Gruppen/Rückbezügen und dokumentierten Sonderzeichen | Erweiterte Suche | TXT+DOC | D | Muster nicht still als fremden Regex-Dialekt interpretieren | T 13.4; P 10.2 |
| EDIT-019 | Gehe zu Seite, Abschnitt, Zeile, Überschrift, Textmarke, Note und Objekt | Navigation / Gehe zu | DOC+LAY+FLD | D | Rücksprung zur vorherigen Bearbeitungsstelle | T 8.1; S 1 |
| EDIT-020 | Text/Listen/Absätze sprachabhängig sortieren, Überschriften mit Inhalt verschieben | Start / Sortieren; Gliederung | DOC+TXT | D | Unterkapitel, Referenzen und Revisionen bleiben zugeordnet | T 8.3, 13.5 |
| EDIT-021 | Klick-und-Schreiben in freien Bereichen mit kontrollierten Tab-/Absatzformaten | Dokument / Doppelklick | DOC+LAY+GUI | D | Erzeugte Struktur bleibt sichtbar und rücknehmbar | B 2.8 |
| EDIT-022 | Importbereinigung für Leerraum, Zeilenenden, Trennungen und Formatierung mit Vorschau | Start / Text bereinigen | DOC+TXT | D | Fachsymbole und absichtliche Umbrüche nicht ungefragt löschen | T 10.6 |
| EDIT-023 | Einfüge-/Überschreibmodus und Optionen für intelligente Wort-/Absatzauswahl | Einstellungen / Bearbeiten | DOC+GUI | P | Modus erkennbar; Ersetzen einer Auswahl als ein Undo-Schritt | T 7.2; B 2.11 |

### 6.4 Zeichen, Absätze, Listen, Styles und Vorlagen

| ID | Funktion und Aufbau | Bedienort | Architektur | Austausch | Kennzeichnende Abnahme | Quelle |
|---|---|---|---|---|---|---|
| FMT-001 | Schriftfamilie, Größe, Schnitt, Farbe und Hervorhebung mit gemischtem Auswahlzustand | Start / Schrift | DOC+TXT | D | Auswahl verschiedener Werte zeigt keinen falschen Einzelwert | B 4.3; T 12 |
| FMT-002 | Unterstreichen, Durchstreichen, Kapitälchen, Großschrift, Hoch-/Tiefstellung, verborgen | Schrift / Erweitert | DOC+TXT | D | Kombinationen in Export und Auswahl erhalten | T 12.3; B 4.3 |
| FMT-003 | Laufweite, Skalierung, Zeichenposition, Kerning und verfügbare OpenType-Merkmale | Schrift / Erweitert | TXT+DOC | D | Nicht verfügbare Fontmerkmale nachvollziehbar anzeigen | T 12.2; P 2.3; A |
| FMT-004 | Absatz links/rechts/zentriert/Blocksatz einschließlich Bidi-Richtung | Start / Absatz | DOC+LAY+TXT | D | Letzte Zeile und RTL-Absatz korrekt ausrichten | T 9.5; B 4.4 |
| FMT-005 | Linker/rechter, Erstzeilen- und hängender Einzug | Absatz / Lineal | DOC+LAY | D | Numerische Eingabe und Lineal ergeben identische Werte | T 9.4; B 4.4 |
| FMT-006 | Einfacher/mehrfacher/exakter/mindestens Zeilenabstand und Absatzabstände | Absatz / Abstand | DOC+LAY | D | Hohe Inline-Objekte bei Mindestabstand nicht abschneiden | T 9.2, 9.4 |
| FMT-007 | Tabstopps links/rechts/zentriert/dezimal/Leiste mit Füllzeichen | Absatz / Tabulatoren | DOC+LAY+TXT | D | Dezimalzeichen sprachabhängig, Füllzeichen korrekt | T 9.6; B 7.1 |
| FMT-008 | Absatz-/Zeichenrahmen, Schattierung, Linienarten und Innenabstand | Start / Rahmen | DOC+LAY+GFX | D | Rahmen über Seitenumbrüche konsistent | T 12.4-12.5; B 4.6 |
| FMT-009 | Format übertragen, wiederholt anwenden, Direktformatierung selektiv löschen | Start / Format | DOC+APP | D | Zeichenformat löschen erhält Absatzstyle | B 4.7; T 6.2 |
| FMT-010 | Zeichen-, Absatz-, verknüpfte, Tabellen- und Listenstyles | Start / Formatvorlagen | DOC+GUI | D | Styletyp begrenzt zulässige Eigenschaften | T 6.1-6.3; S 3 |
| FMT-011 | Style erstellen aus Auswahl oder Definition, umbenennen, ändern und löschen | Formatvorlagenbereich | DOC+GUI | D | Verwendeten Style löschen mit kontrollierter Ersetzung | T 2.2, 6.3; B 6 |
| FMT-012 | Stylevererbung, Folgestyle, Schnellgalerie, Tastenkürzel und automatische Aktualisierung | Style / Eigenschaften | DOC+APP | D | Vererbungszyklus ablehnen; Änderung wirkt nachvollziehbar | T 6.2-6.3 |
| FMT-013 | Styleinspektor, effektive Formatierung, Herkunft und Vergleich zweier Textstellen | Formatvorlagen / Prüfen | DOC+GUI | D | Direktformatierung von vererbten Eigenschaften unterscheiden | T 6.5 |
| FMT-014 | Styleorganisator zwischen Dokumenten/Vorlagen mit Konfliktvergleich | Vorlagen / Organisieren | DOC+IO | D | Gleichnamige Definitionen nicht unbemerkt überschreiben | T 6.4; S 3 |
| FMT-015 | Überschriftenebenen, Gliederungsrang, Auf-/Zuklappen und Styletrenner | Start / Überschriften | DOC+FLD+GUI | D | Teilabsatz im Inhaltsverzeichnis ohne Resttext | T 2.3, 6.2.3; P 3.6 |
| FMT-016 | Aufzählungen mit Zeichen, eigener Glyphen-/Bildwahl und Abständen | Start / Aufzählung | DOC+LAY | D | Aufzählungszeichen und Textausrichtung bleiben gekoppelt | T 14.1-14.3; B 4.5 |
| FMT-017 | Nummerierte Listen, Startwert, Fortsetzen/Neustart, Zahl-/Buchstabenformate | Start / Nummerierung | DOC+FLD+LAY | D | Getrennte Listen mit gleicher Optik bleiben unterscheidbar | T 14.3; B 4.5 |
| FMT-018 | Mehrstufige Listen mit Ebenenbezug, Neustartregeln und Stylebindung | Start / Liste mit Ebenen | DOC+FLD+LAY | D | Kapitel verschieben aktualisiert Unterpunkte und Querverweise | T 2.4-2.5, 14.4-14.5; P 3.2 |
| FMT-019 | Listen zusammenführen, reparieren und beim Einfügen getrennt erhalten | Listen / Erweitert | DOC+IO | D | Nummerierungs-IDs zweier Dateien konfliktfrei übertragen | T 14.6; T 2.5 |
| FMT-020 | Dokumentdesign mit Farben, Schriften, Effekten und Absatzsätzen | Entwurf / Design | DOC+GFX+TXT | D | Designwechsel erhält explizite Abweichungen | B 4.2, 10.2; S 6 |
| FMT-021 | Initialen im Text/Rand mit Größe, Abstand und Absatzbezug | Einfügen / Initiale | DOC+LAY+TXT | D | Initiale passt sich Absatzumbruch an | S 6 |
| FMT-022 | AutoText/Schnellbausteine mit Kategorie, Vorschau und Ressourcen | Einfügen / Bausteine | DOC+IO+APP | D | Gespeicherter Baustein enthält auch erforderliche Styles/Quellen | T 11.4; B 9.2 |

### 6.5 Seiten, Abschnitte und Textsatz

| ID | Funktion und Aufbau | Bedienort | Architektur | Austausch | Kennzeichnende Abnahme | Quelle |
|---|---|---|---|---|---|---|
| PAGE-001 | Papiergröße, eigene Maße, Orientierung und Druckbereich pro Abschnitt | Layout / Seite einrichten | DOC+LAY | D | Wechsel Hoch-/Querformat ohne Verlust vorheriger Einstellungen | T 1.2; B 5.1-5.2 |
| PAGE-002 | Ränder, Bundsteg, Spiegelränder, Gegenüberseiten und Buchlayout | Layout / Ränder | DOC+LAY | D | Innen-/Außenrand auf geraden/ungeraden Seiten korrekt | T 1.2; B 5.1 |
| PAGE-003 | Zeilen-, Seiten-, Spalten- und Abschnittsumbrüche: fortlaufend, nächste/gerade/ungerade Seite | Layout / Umbrüche | DOC+LAY | D | Absichtlich eingefügte Leerseite in Vorschau und Export konsistent | T 1.1; P 2.2 |
| PAGE-004 | Abschnittseigenschaften, Gültigkeitsbereich, Übertragung und Zusammenführen | Seite einrichten / Anwenden auf | DOC+LAY | D | Umbruch löschen zeigt nachvollziehbare Übernahme der Eigenschaften | T 1.2.4-1.2.5 |
| PAGE-005 | Gleich-/ungleichbreite Spalten, Zwischenräume, Trennlinien und Spaltenausgleich | Layout / Spalten | DOC+LAY | D | Mehrspaltigen Abschnitt vor einspaltigem Text korrekt abschließen | T 1.4; P 2.4 |
| PAGE-006 | Kopf-/Fußzeilen mit separaten Stories und Abstand zum Rand | Einfügen / Kopf-/Fußzeile | DOC+LAY | D | Textauswahl bleibt innerhalb des bearbeiteten Bereichs | T 3; B 5.5 |
| PAGE-007 | Erste Seite anders, gerade/ungerade Varianten und Verknüpfung zum vorigen Abschnitt | Kontext Kopf-/Fußzeile | DOC+LAY | D | Verknüpfung lösen verändert andere Abschnitte nicht | T 3.1-3.2 |
| PAGE-008 | Seitennummern mit Beginn, Fortsetzen, römisch/arabisch und Kapitelpräfix | Kopf-/Fußzeile / Seitenzahl | FLD+DOC+LAY | F | Vortext und Haupttext mit verschiedenen Zählungen | T 4; P 4 |
| PAGE-009 | Seite X von Y, Abschnittsseiten, Fortsetzungshinweise und leere Anfangsseiten | Einfügen / Feld | FLD+LAY | F | Abschnitts- und Gesamtanzahl nicht verwechseln | T 4.1-4.2 |
| PAGE-010 | Laufende Kolumnentitel aus Überschrift/Nummer mit Sprach-/Stylezuordnung | Kopfzeile / Feld | FLD+DOC+LAY | F | Mehrere Überschriften auf einer Seite nach Feldregel auflösen | T 2.6 |
| PAGE-011 | Zeilennummerierung mit Intervall, Neustart und Ausnahmen | Layout / Zeilennummern | DOC+LAY | D | Unterdrückung für ausgewählte Absätze wirkt in Druck/PDF | T 14.7; P 2.3 |
| PAGE-012 | Automatische/manuelle Silbentrennung, Sprache und Trennverbote | Layout / Silbentrennung | TXT+DOC+LAY | D | Geschützter Bindestrich und bedingter Trennstrich unterschiedlich behandeln | T 9.1; B 2.8 |
| PAGE-013 | Absatz zusammenhalten, mit nächstem verbinden, Schusterjungen/Hurenkinder, Seitenumbruch davor | Absatz / Textfluss | DOC+LAY | D | Überschrift nicht allein am Seitenende | T 9.7; P 2 |
| PAGE-014 | Vertikale Seitenausrichtung, Satzspiegelanzeige und Grundlinienausrichtung | Layout / Seite; Ansicht | DOC+LAY | D | Mehrspaltige Grundlinien bei gleichen Parametern ausrichten | T 1.2-1.4; A |
| PAGE-015 | Leere Seite und eigene Deckblattvorlagen mit austauschbaren Metadaten | Einfügen / Seiten | DOC+APP | D | Deckblatt entfernen ohne Inhalt der Folgeseite zu verlieren | T 1.1.2; B 5.3 |
| PAGE-016 | Wasserzeichen, Seitenfarbe und Seitenrahmen mit Abschnitts-/Druckoptionen | Entwurf / Seitenhintergrund | DOC+LAY+GFX | D | Bildschirmdarstellung und Druckoption explizit getrennt | T 1.3; B 5.6 |
| PAGE-017 | Textfelder/Positionsrahmen, Innenabstände, Drehen und unabhängige Textausrichtung | Einfügen / Textfeld | DOC+LAY | D | Textfeldinhalt bleibt editierbar und verankert | T 1.5; S 6 |
| PAGE-018 | Verkettete Textfelder, Überlaufanzeige und Randnotizen | Textfeld / Verknüpfen | DOC+LAY | D | Überlauf ohne Textverlust in Folgefeld; Zyklus ablehnen | T 1.5.2-1.5.3 |
| PAGE-019 | Inkrementeller Satz mit stabilen Positionen, Hintergrundpaginierung und Abbruch | Dokument / automatisch | LAY+DOC | D | Änderung am Anfang aktualisiert Seiten/Referenzen ohne veralteten Stand | A; U |
| PAGE-020 | Unicode-Bidi, komplexe Schriften, Sprachwechsel, Fontfallback und vertikale Texte | Absatz / Sprache/Richtung | TXT+LAY+DOC | D | Arabisch/Hebräisch mit lateinischen Formeln und kombinierenden Zeichen | P 8; A |

### 6.6 Tabellen

**Implementierter Berechnungsteil vom 13. Oktober 2026:** [TAB-014 / Tabellenformeln](docs/TABLE_FORMULAS.md)
mit gemeinsamen Dezimalrechenfeldern, Zell-/Bereichsbezügen, Richtungsaggregaten,
Zahlenformaten, Fehleranzeige und DOCX-/nativer Zwischenablage. Weitere Word-Feldprofile
und automatische Umschreibung expliziter Zellbezüge bleiben Ausbauziele.

| ID | Funktion und Aufbau | Bedienort | Architektur | Austausch | Kennzeichnende Abnahme | Quelle |
|---|---|---|---|---|---|---|
| TAB-001 | Tabelle per Raster, Maßeingabe, Zeichnen oder eigener Vorlage erstellen | Einfügen / Tabelle | DOC+GUI | D | Leere Tabelle besitzt gültige Zellstruktur | T 15.1; B 7.2 |
| TAB-002 | Text zu Tabelle und zurück mit wählbaren Trennzeichen | Tabelle / Konvertieren | DOC+TXT | D | Absatz-/Tabulatortrennung ohne verlorene Inhalte | T 15.1.3; B 7.5; P 5.3 |
| TAB-003 | Zellnavigation, Tab am Tabellenende, Zeilen-/Spalten-/Gesamtauswahl | Tabelle | DOC+GUI | D | Tabellen-Tab und echter Tab im Zelltext unterscheidbar | T 15.2.1-15.2.2 |
| TAB-004 | Zeilen/Spalten/Zellen einfügen, löschen und verschieben | Tabellenlayout | DOC+GUI | D | Verbundene Zellen bleiben strukturell gültig | T 15.2.3-15.2.6 |
| TAB-005 | Zellen horizontal/vertikal verbinden und teilen | Tabellenlayout / Verbinden | DOC+LAY | D | Teilung und Undo erhalten Inhalte/Reihenfolge | T 15.2.4; B 7.3 |
| TAB-006 | Tabellen teilen/zusammenfügen und verschachtelte Tabellen | Tabellenlayout | DOC+LAY | D | Verschachtelte Zeilenhöhe berechnet sich ohne Endlosschleife | T 15.2.8; A |
| TAB-007 | Feste/bevorzugte Breiten, AutoAnpassen, gleichmäßig verteilen und Zeilenhöhen | Tabellenlayout / Größe | DOC+LAY | D | Mindesthöhe und exakte Höhe verschieden behandeln | T 15.2.7; B 7.3 |
| TAB-008 | Zellabstände, Innenränder, vertikale/horizontale Ausrichtung und Textrichtung | Tabellenlayout / Ausrichtung | DOC+LAY | D | Gedrehter Text beeinflusst Größenberechnung korrekt | T 15.3.4; B 7.4 |
| TAB-009 | Tabellenstyles mit erster/letzter Zeile/Spalte und gebänderten Bereichen | Tabellenentwurf | DOC+LAY | D | Direktformatierung und bedingte Styleanteile auflösen | T 6.3.4, 15.3.1 |
| TAB-010 | Rahmen, Schattierung, Linienzeichnen/-radieren und nur sichtbare Gitternetzlinien | Tabellenentwurf | DOC+GFX+LAY | D | Nicht druckbare Gitterlinien gelangen nicht in PDF | T 15.3.2-15.3.3; B 7.4 |
| TAB-011 | Tabellenposition, Einzug, umfließender Text und Anker | Tabelleneigenschaften | DOC+LAY | D | Tabelle mit Absatzverschiebung korrekt positionieren | T 15.3.5; B 7.3 |
| TAB-012 | Mehrseitige Tabellen mit wiederholten Kopfzeilen und erlaubten/verbotenen Zeilenumbrüchen | Tabelleneigenschaften / Zeile | DOC+LAY | D | Lange Zelle, Fußnote und Kopfzeilen auf Folgeseite korrekt | T 15.4; P 5.5 |
| TAB-013 | Mehrschlüsselige Sortierung mit Kopfzeilenausschluss und Sprach-/Zahlentypen | Tabellenlayout / Sortieren | DOC+TXT | D | Inhalte verknüpfter Zeilenzellen gemeinsam bewegen | T 15.6; S 4 |
| TAB-014 | Zellberechnungen und Zahlenformate über Feldengine | Tabellenlayout / Formel | FLD+DOC | F | Referenzen nach Zeilenänderung überprüfen; Fehler sichtbar | T 20.3.1 |
| TAB-015 | Tabellen aus Zwischenablage, externen Dateien und Kalkulationsdaten importieren | Einfügen / Tabelle | IO+DOC | X | Wählbar: echte Tabelle, eingebettetes Objekt oder Verknüpfung | T 15.5 |
| TAB-016 | Tabellenbeschriftung, Alternativtext und Kopfzellen-Semantik | Referenzen / Beschriftung; Eigenschaften | DOC+FLD+OUT | D | Screenreader/PDF erkennen Überschriften und Lesereihenfolge | P 5.2; T 24.1; A |

### 6.7 Quellenverwaltung als integrierter Kern

| ID | Funktion und Aufbau | Bedienort | Architektur | Austausch | Kennzeichnende Abnahme | Quelle |
|---|---|---|---|---|---|---|
| SRC-001 | Globale Quellenbibliothek und unabhängige dokumentbezogene Quellenliste | Referenzen / Quellen verwalten | BIB+APP | M | Dokument auf zweitem Rechner ohne globale Bibliothek vollständig zitierbar | T 5.6.1-5.6.2; U |
| SRC-002 | Quelle erstellen, ändern, kopieren, suchen, sortieren und löschen | Quellenbereich / Aktionen | BIB+GUI | M | Löschen verwendeter Quelle bietet Umhängen statt verwaister Zitate | T 5.6; A |
| SRC-003 | Quelltypen: Buch, Beitrag, Zeitschrift, Konferenz, Bericht, Abschlussarbeit, Webseite, Patent, Recht, Medien, Datensatz, Software und sonstige | Quelle / Typ | BIB | M | Typspezifische Felder ohne Verlust beim Typwechsel | T 5.6.1; U; A |
| SRC-004 | Alle Word-Quelltypen/-felder abbilden; unbekannte Erweiterungsfelder erhalten | Quelle / Alle Felder | BIB+IO | M | Originalfeld kann trotz fehlender UI verlustfrei zurückgeschrieben werden | W-BIB; U |
| SRC-005 | Personen mit Vor-/Familiennamen, Partikeln, Suffixen; Körperschaften und Rollen | Quelle / Personen | BIB+GUI | M | Mehrere Autoren/Herausgeber/Übersetzer bleiben geordnet | T 5.6.1; W-CSL |
| SRC-006 | Publikationsmetadaten: Titelvarianten, Reihe, Band, Heft, Auflage, Verlag, Ort, Seiten und Sprache | Quelle / Publikation | BIB | M | Fehlender Wert bleibt von leerem/absichtlich unterdrücktem Wert unterscheidbar | T 5.6.1; W-CSL |
| SRC-007 | Erscheinungs-/Zugriffs-/Originaldaten mit unvollständigen Datumsangaben und Zeiträumen | Quelle / Datum | BIB | M | Jahr allein wird nicht zu erfundenem Datum ergänzt | W-CSL; A |
| SRC-008 | DOI, ISBN, ISSN, URL und weitere Kennungen mit Validierung und Originalwert | Quelle / Kennungen | BIB | M | Normalisierung zerstört Originalschreibweise nicht | U; A |
| SRC-009 | Stabile interne Quellen-ID, Word-Tag und externe Schlüssel getrennt führen | Quellen / Eigenschaften | BIB+IO | M | Gleichnamige Word-Tags zweier Dateien konfliktfrei importieren | T 5.6.1; U |
| SRC-010 | Unvollständige Quellen und Zitatplatzhalter mit Aufgabenliste | Referenzen / Platzhalter | BIB+FLD+GUI | F | Platzhalter durch Quelle ersetzen aktualisiert alle Zitate | T 5.6.5 |
| SRC-011 | Globale und lokale Quelle kopieren/abgleichen, Änderungen vergleichen | Quellenverwaltung / Übernehmen | BIB+APP | M | Globale Änderung verändert abgeschlossene Dokumente nicht ungefragt | T 5.6.2; A |
| SRC-012 | Dublettenerkennung über Kennungen/Metadaten, manuelles Zusammenführen und Undo | Quellen / Dubletten | BIB+DOC | M | Alle Zitatbezüge auf verbleibende ID umhängen | U; A |
| SRC-013 | Schlagwörter, Sammlungen, Notizen und lokale Anlagen/Links | Quellenbereich | BIB+IO | M | Fehlende Anlage ändert zitierbare Metadaten nicht | U; A |
| SRC-014 | Word-Sources.xml sowie im Dokument eingebettete Quellen importieren/exportieren | Quellen / Austausch | BIB+IO | X | Personenrollen, Tags, Typen und Originalfelder erhalten | T 5.6; W-BIB |
| SRC-015 | BibTeX und BibLaTeX mit Schlüsseln, Namen, geschützter Großschreibung und Zusatzfeldern | Quellen / Import/Export | BIB+BAS | X | Nicht abbildbare Felder im Bericht und Originaldatensatz behalten | U |
| SRC-016 | RIS mit wiederholten Feldern, mehrzeiligen Werten und Typabbildung | Quellen / Import/Export | BIB+BAS | X | Mehrere Autoren nicht zu einem Namen zusammenziehen | U |
| SRC-017 | CSL-JSON mit Namen, Datumsarrays und Zusatzdaten | Quellen / Import/Export | BIB+BAS | X | Typ-/Datumspräzision nach Export und Wiederimport unverändert | U; W-CSL |
| SRC-018 | Importvorschau, Feldzuordnung, Zeichencodierung und Fehlerprotokoll | Quellen / Importassistent | BIB+IO+GUI | X | Fehlerhafte Quelle verhindert nicht nachvollziehbaren Import gültiger Datensätze | U; A |
| SRC-019 | Metadaten per DOI/ISBN/URL und Anbieter suchen, Unterschiede vor Übernahme zeigen | Quellen / Online ergänzen | BIB+SVC | X | Mehrdeutige Treffer nicht automatisch als sichere Quelle setzen | T 21.2; U |
| SRC-020 | Quelle einschließlich Herkunft und Abrufzeit eines Metadatenimports nachvollziehen | Quelle / Herkunft | BIB | M | Automatisch ergänzte Daten von Nutzereingaben unterscheidbar | U; A |
| SRC-021 | Literaturverwaltungen über offene Austauschformate und eigene Anbieteradapter anbinden | Quellen / Austausch | BIB+EXT | X | Zotero-/Citavi-/EndNote-/Mendeley-Daten prüfen; keine Zusage fremder Add-in-Ausführung | T 5.6; P 9.1-9.2; U |
| SRC-022 | Quellen sichern, übertragen und wiederherstellen, ohne Microsoft-Profil zu verändern | Quellen / Bibliothek | BIB+IO | X | Backup auf frischer Installation vollständig nutzbar | T 5.6.1; U |
| SRC-023 | Quellen und Zitierbeziehungen beim Kopieren zwischen Dokumenten automatisch mitnehmen | Dokument / Einfügen | BIB+DOC+IO | F | Zielbibliografie ergänzt richtige Quelle ohne Doppelanlage | U; A |
| SRC-024 | Validierung je Quelltyp/Zitierstil, fehlende Pflichtangaben und verwaiste Zitate melden | Referenzen / Prüfen | BIB+FLD | M | Unvollständige Quellen bleiben speicherbar, Fehler werden nicht erfunden behoben | T 5.6; U |

### 6.8 Zitate, Zitierstile und Literaturverzeichnisse

| ID | Funktion und Aufbau | Bedienort | Architektur | Austausch | Kennzeichnende Abnahme | Quelle |
|---|---|---|---|---|---|---|
| CITE-001 | Einzelzitat als strukturierter Feld-/Zitierknoten statt bloßem Text | Referenzen / Zitat einfügen | BIB+FLD+DOC | F | Quelle bleibt über Speichern und Öffnen bearbeitbar | T 5.6.3; P 9.3 |
| CITE-002 | Mehrfachzitate als geordneter Cluster mit Stil-Sortierung/-Zusammenfassung | Zitat / Quellen hinzufügen | BIB+FLD | F | Mehrere Quellen und individuelle Fundstellen getrennt erhalten | T 5.6.3; W-CSL |
| CITE-003 | Seiten, Seitenbereiche, Kapitel, Absatz, Band und andere Fundstellen | Zitat / Fundstelle | BIB | F | Originalangabe und formatierte Ausgabe unterscheiden | T 5.6.3; W-CSL |
| CITE-004 | Präfix/Suffix und Unterdrücken von Autor, Jahr oder Titel | Zitat / Bearbeiten | BIB+FLD | F | Word-Feldschalter und angezeigter Text stimmen überein | T 5.6.3 |
| CITE-005 | Narrative/in den Satz integrierte und eingeklammerte Zitate | Zitat / Darstellung | BIB+DOC | F | Autor im Satz nicht doppelt ausgeben | U; W-CSL |
| CITE-006 | Zitate im Fließtext, in Fuß-/Endnoten, Beschriftungen und Tabellen | Referenzen / Zitat | BIB+DOC+FLD | F | Bibliografie erfasst verwendete Quellen aus allen Stories | T 5.6.3; A |
| CITE-007 | Autor-Jahr-, numerische und Fußnotenstile mit nachvollziehbarer Stilwahl | Referenzen / Zitierstil | BIB | F | Stilwechsel ersetzt keine Quelldaten | T 5.6.4; W-CSL |
| CITE-008 | Eigene Implementierung eines CSL-1.0.2-Prozessors einschließlich abhängiger Styles/Locales | Stilverwaltung | BIB+BAS | F | Offizielle Spezifikationsfälle für Bedingungen, Sortierung, Namen und Daten bestehen | U; W-CSL |
| CITE-009 | Namensmehrdeutigkeit, Jahreszusätze, et al., Kurz-/Langform und wiederholte Nachweise | Zitat / automatisch | BIB+FLD | F | Einfügen einer weiteren Quelle aktualisiert frühere Mehrdeutigkeiten | W-CSL; U |
| CITE-010 | Sprach-/Gebietsschema pro Dokument, Stil und Zitat; Titel-/Interpunktionsregeln | Zitierstil / Sprache | BIB+TXT | F | Deutsche/englische Zitate mit Teil-Datumsangaben korrekt | T 5.6.3; W-CSL |
| CITE-011 | Versionierte Stilpakete für APA, MLA, Chicago, IEEE, Vancouver und ISO-690-Varianten | Referenzen / Stilverwaltung | BIB+APP | X | Exakte Stilversion/Locale speichern; Lizenz jeder mitgelieferten Datei nachweisen | T 5.6; U |
| CITE-012 | Eigene Hochschul-/Journalstile importieren, validieren und Vorschau mit Beispieldaten | Stilverwaltung / Import | BIB+GUI | X | Ungültiger Stil beschädigt weder Dokument noch bisherigen Stil | T 5.6.4; U |
| CITE-013 | Word-Bibliografiestil und CSL-Stil als unterschiedliche Profile führen | Zitierstil / Word-Austausch | BIB+IO | F | Kein verlustfreies automatisches XSL-CSL-Mapping vortäuschen | W-BIB; W-CSL; A |
| CITE-014 | CITATION/BIBLIOGRAPHY-Feldcodes und zwischengespeicherte Ergebnisse importieren/exportieren | Öffnen / Speichern | IO+BIB+FLD | F | Wiederöffnung und erneute Aktualisierung in Word getrennt testen | T 5.6; W-BIB |
| CITE-015 | Literaturverzeichnis aus zitierten oder ausdrücklich zusätzlich aufgenommenen Quellen | Referenzen / Literatur | BIB+FLD | F | Unzitierte Quellen erscheinen nur gemäß gewählter Regel | T 5.6.6 |
| CITE-016 | Mehrere Literaturverzeichnisse nach Kapitel, Bereich, Typ oder Sammlung | Literatur / Bereich | BIB+FLD+DOC | F | Gleiche Quelle in mehreren Bereichen ohne ID-Duplikat | T 5.6.7; U |
| CITE-017 | Literaturformat: Einzüge, Abstände, Sortierung, DOI/URL-Links und Hanging Indent | Literatur / Format | BIB+DOC+LAY | F | Verzeichnis bleibt nach Aktualisierung formatiert | T 5.6; W-CSL |
| CITE-018 | Alle Zitate/Literaturverzeichnisse konsistent aktualisieren; Änderungsvorschau | Referenzen / Aktualisieren | BIB+FLD+DOC | F | Abbruch lässt alten konsistenten Stand bestehen | T 5.6; A |
| CITE-019 | Zitat zur Quelle und Quellenverwendung im Dokument navigieren | Zitat / Quelle anzeigen | BIB+GUI+DOC | F | Alle Verwendungsorte einschließlich Noten erreichbar | U |
| CITE-020 | Zitatausgabe sperren oder ausdrücklich in statischen Text umwandeln | Zitat / Konvertieren | DOC+BIB+FLD | F | Datenverlust vor Umwandlung benennen, Undo möglich | T 5.6; A |
| CITE-021 | Herkunft der Zitate beibehalten; fremde Literatur-Add-in-Felder erkennen und erhalten | Import / Kompatibilitätsbericht | IO+BIB | R | Unbekannte Add-in-Felder nicht als normale jfWord-Zitate ausgeben | P 9.1-9.2; U |

### 6.9 Noten, Verweise, Beschriftungen und Verzeichnisse

| ID | Funktion und Aufbau | Bedienort | Architektur | Austausch | Kennzeichnende Abnahme | Quelle |
|---|---|---|---|---|---|---|
| REF-001 | Fuß-/Endnote einfügen, löschen und zwischen Verweis und Notentext wechseln | Referenzen / Noten | DOC+LAY+FLD | D | Löschen des Verweises entfernt genau die zugehörige Note | T 5.2 |
| REF-002 | Notennummern mit Startwert, Zahlen-/Symbolformat und Neustart je Seite/Abschnitt | Noten / Optionen | FLD+DOC+LAY | F | Verschobener Absatz aktualisiert Notenreihenfolge | T 5.2.5 |
| REF-003 | Fuß-/Endnoten umwandeln, Endnotenposition und Abschnittsunterdrückung | Noten / Konvertieren | DOC+LAY | D | Inhalt, Zitate und Querverweise bleiben zugeordnet | T 5.2; A |
| REF-004 | Notenstyles, Einzug, Trenn-/Fortsetzungslinien und seitenübergreifende Noten | Notenbereich / Format | DOC+LAY | D | Lange Note ohne Überlagerung mit Haupttext | T 5.2.6-5.2.8 |
| REF-005 | Textmarken als Punkte/Bereiche erstellen, benennen, auflisten und entfernen | Einfügen / Textmarke | DOC+FLD | F | Einfügen am Rand folgt dokumentierter Ankerregel | T 19.3 |
| REF-006 | Hyperlinks auf Web, Datei, E-Mail, Überschrift und Textmarke | Einfügen / Link | DOC+GUI+SEC | D | Sichtbarer Text und Ziel getrennt editierbar | T 5.1; P 7.5 |
| REF-007 | Querverweise auf Überschrift, Liste, Note, Abbildung, Tabelle, Formel und Textmarke | Referenzen / Querverweis | FLD+DOC | F | Umnummeriertes Ziel ändert alle Referenzen | T 5.1 |
| REF-008 | Referenzvarianten: Nummer, Text, Seite, relative Lage und mehrere Ziele | Querverweis / Ausgabe | FLD+LAY | F | Verweisbereich bleibt nach Zielverschiebung korrekt | T 5.1.2; P 7 |
| REF-009 | Fehlende/doppelte Referenzziele und ungültige Links als reparierbare Diagnose | Referenzen / Prüfen | FLD+DOC | F | Nicht vorhandenes Ziel zeigt keinen erfundenen Seitenwert | U; A |
| REF-010 | Beschriftungen mit eigenen Kategorien, SEQ-Zählern und Kapitelnummer | Referenzen / Beschriftung | FLD+DOC | F | Abbildung zwischen zwei vorhandenen einfügen aktualisiert Folgewerte | T 18; P 5.2, 6.3 |
| REF-011 | Beschriftung an schwebendes/inline Objekt koppeln, automatische Beschriftung | Beschriftung / Optionen | DOC+LAY+FLD | D | Objekt und Unterschrift bleiben beim Umbruch zusammen | T 18.1, 18.3 |
| REF-012 | Quellenangabe in Beschriftung vom Verzeichniseintrag abgrenzen | Beschriftung / Quellen | BIB+FLD+DOC | F | Abbildungsverzeichnis enthält gewünschte Kurzfassung | T 5.5.2, 18.1.3 |
| REF-013 | Inhaltsverzeichnis aus Ebenen, Styles und expliziten TC-Einträgen | Referenzen / Inhaltsverzeichnis | FLD+DOC+LAY | F | Überschriftenänderung aktualisiert Text und Seiten | T 5.4; P 3.3 |
| REF-014 | Inhaltsverzeichnis: Ebenen, Füllzeichen, Seiten-/Hyperlinkoptionen und eigene Styles | Inhaltsverzeichnis / Anpassen | FLD+DOC+LAY | F | Automatische Aktualisierung erhält Verzeichnisgestaltung | T 5.3-5.4 |
| REF-015 | Mehrere/abschnittsbezogene Inhaltsverzeichnisse, nur Seiten aktualisieren, statische Kopie | Inhaltsverzeichnis / Bereich | FLD+DOC | F | Haupt-/Teil-Verzeichnisse haben definierte, unabhängige Bereiche | P 3.4-3.6; T 5.4.6 |
| REF-016 | Abbildungs-, Tabellen- und Formelverzeichnisse nach Kategorien/Styles | Referenzen / Verzeichnis | FLD+DOC+LAY | F | Kategorieauswahl mischt Formeln nicht mit Tabellen | T 5.5; P 5.4, 6.3, 7.7 |
| REF-017 | Indexeinträge: Haupt-/Untereintrag, Mehrfachmarkierung, Bereiche, siehe/siehe auch | Referenzen / Indexeintrag | FLD+DOC | F | Versteckte XE-Marken bleiben beim Bearbeiten zugeordnet | T 5.8.1-5.8.4; P 12 |
| REF-018 | Index erzeugen: Sprache, Sortierung, Gruppen, Spalten, Buchstabenüberschriften | Referenzen / Index | FLD+TXT+LAY | F | Umlaute, Bereiche und getrennte Indexkategorien korrekt | T 5.8.5 |
| REF-019 | Rechtsgrundlagen markieren, Kategorien, Kurz-/Langzitat, Wiederverwendung und passim | Referenzen / Rechtsgrundlagen | FLD+DOC | F | TA-/TOA-Daten und Kategorien nach Roundtrip erhalten | T 5.7 |
| REF-020 | Integriertes Glossar und Abkürzungsverzeichnis mit Definition, Erstverwendung und Sortierung | Referenzen / Glossar | DOC+FLD | F | Eigene Funktion ersetzt manuelle Workarounds; Exportabbildung transparent | T 5.8.6; U |
| REF-021 | Dokumentübergreifende Verzeichnisse mit RD-/Dateibezügen und Kapitelpaginierung | Referenzen / Dokumentverbund | FLD+IO+LAY | F | Fehlende Kapitel und doppelte Seitennummern melden | T 5.9 |
| REF-022 | Nummerierungen/Verzeichnisse zentral prüfen und aktualisieren | Referenzen / Alles aktualisieren | FLD+BIB+LAY | F | Paginierung bis stabil; Zyklus/fehlendes Ziel mit Diagnose | T 5, 19; A |

**Implementierter Noten-Teilstand vom 13. Oktober 2026:** Die folgenden Teilumfänge sind
im Funktionsprototyp vorhanden. Sie schließen die umfassenden Anforderungen oben und
die Word-Referenzabnahme noch nicht ab. Bedienung und Dateiprofil stehen ausführlich in
[README.md](README.md#fuß--und-endnoten).

| Katalogbezug | Implementierter Teilumfang | Noch offen | Nachweis im Projekt |
|---|---|---|---|
| REF-001 | Einmaliger Haupttext- oder Tabellenzellenanker (auch verschachtelt/schwebend); Fuß-/Endnote einfügen, direkt auf der Papierseite mehrabsätzig bearbeiten, zwischen Verweis und Inhalt navigieren, gemeinsam entfernen und per Undo wiederherstellen | Weitere Ankertypen und Word-Referenzabnahme | `notes_checks.rs`, `notes.uis`, `rich.uis` |
| REF-002 | Getrennte dezimale Folgen ab 1; fortlaufende Nummerierung nach Lesereihenfolge einschließlich Tabellenzellen; Änderungen aktualisieren beide Folgen | Andere Startwerte, römische Zahlen/Symbole, Neustart je Seite/Abschnitt | `notes_checks.rs`, `notes_io_checks.rs` |
| REF-003 | Fuß-/Endnote einzeln umwandeln; Inhalt, Formate, stabile IDs und Zitatverweise bleiben erhalten | Abschnittsabhängige Endnotenposition, Abschnittsunterdrückung, Umwandlung ganzer Bereiche und echte Querverweise | `notes_checks.rs`, `notes.uis` |
| REF-004 | Zeichen-/Absatzformatierung in Noten; reservierter Fußnotenraum, feste Trennlinie, Fortsetzung langer Noten, Endnoten nach dem Haupttext | Freie Notenstyles und Trenn-/Fortsetzungslinien, flexiblere Verteilung mehrerer Noten derselben Haupttextzeile, Word-identischer Umbruch | `notes_layout_checks.rs`, Noten-Vorschauscreenshots |
| CITE-006, CITE-019 | Strukturierte Zitate im Haupttext und in beiden Notenarten; gemeinsame Quellenverwendung, Navigation, Löschschutz, Zusammenführung und dynamisches Literaturverzeichnis | Zitate in Beschriftungen/weiteren Stories; vollständige CSL-/wissenschaftliche Fußnotenstile | `notes_checks.rs`, `notes_io_checks.rs`, `notes.uis` |

Die Vorschau kennzeichnet beide Folgen derzeit als `[1]` und `[E1]`; DOCX verwendet
automatische hochgestellte Zeichen mit separaten dezimalen Folgen. Das begrenzte
DOCX-Profil erhält eigene Noten-, Absatz- und Quellenidentitäten einschließlich Zitatfeldern.
Es prüft Notenparts, Einstellungen, Standardtrennnoten und Verweise vollständig und
weist nicht darstellbare Nummerierungen, eigene Zeichen, verschachtelte Noten oder
unbekannte Eigenschaften zurück. Ein allgemeiner Word-Roundtrip ist weiterhin offen.

Der eigene Layoutkern kann Noten auf Folgeseiten fortsetzen. Fußnotenverweise und mindestens die
erste zugehörige Notenzeile müssen gemeinsam passen. Bei mehreren Fußnoten derselben
Haupttextzeile müssen zusätzlich die vorhergehenden Noten dieser Gruppe vollständig
passen; andernfalls wird eine Layoutgrenze gemeldet. Es gibt keine still ausgelassenen
Noteninhalte. Diese begrenzte Verteilung ist eine bekannte Abweichung vom vollständigen
Satz- und Kompatibilitätsziel. Die automatisierten Nachweise umfassen außerdem
`tests/notes.py` für unabhängige OPC/XML-Prüfungen; sie ersetzen keine Word-Abnahme.

### 6.10 Mathematische und wissenschaftliche Formeln

| ID | Funktion und Aufbau | Bedienort | Architektur | Austausch | Kennzeichnende Abnahme | Quelle |
|---|---|---|---|---|---|---|
| MATH-001 | Strukturierter Formeleditor mit Cursor, Auswahl, Platzhaltern und Inline/Display | Einfügen / Formel | MAT+DOC+GUI | D | Verschachtelte Teilstruktur bleibt nach Bearbeitung gültig | T 17.1-17.2; P 7.2 |
| MATH-002 | Brüche, Wurzeln, Potenzen/Indizes, große Operatoren und Grenzen | Formel / Strukturen | MAT+TXT | D | Mehrfach verschachtelte Brüche ohne abgeschnittene Glyphen | T 17; P 8 |
| MATH-003 | Matrizen, Fälle, Gleichungsgruppen, Klammern, Akzente und Operatornamen | Formel / Strukturen | MAT+TXT | D | Variable Klammern wachsen mit Inhalt | T 17; P 8 |
| MATH-004 | UnicodeMath-Eingabe und lineare/professionelle Darstellung | Formel / Konvertieren | MAT+BAS | D | Unterstützte Syntax reversibel, unbekannte Syntax bleibt sichtbar | T 17.3.1; P 8.1 |
| MATH-005 | LaTeX-Mathematikeingabe mit dokumentiertem Syntaxumfang | Formel / LaTeX | MAT+BAS | D | Kein vollständiges TeX-System behaupten; unerkannte Befehle melden | T 17.3.2; P 8.2 |
| MATH-006 | Mathematische AutoKorrektur, Symbolpalette und eigene Kürzel | Formel / Symbole | MAT+TXT+APP | P | Mathematische Kürzel außerhalb Formeln separat schaltbar | T 7.5-7.6, 17.3.3; P 7.3 |
| MATH-007 | Mathematikfonts, Kursivregeln, normale Texte in Formeln und Stilgrößen | Formel / Design | MAT+TXT+LAY | D | Variablen, Einheiten und Operatoren unterscheidbar | T 17.1.1-17.1.2; A |
| MATH-008 | Formelausrichtung, mehrzeilige Umbrüche und Ausrichtung an Relationszeichen | Formel / Ausrichtung | MAT+LAY | D | Lange Gleichung passt sich Satzbreite an | T 17.5; P 7 |
| MATH-009 | Formelnummern links/rechts, kapitelweise Zähler, Beschriftung und Querverweis | Referenzen / Formelnummer | FLD+MAT+LAY | F | Einfügen einer Formel aktualisiert Nummern/Verzeichnis | P 7.4-7.7; T 18 |
| MATH-010 | Eigene Formelsammlung mit Vorschau und wiederverwendbaren Strukturen | Formel / Sammlung | MAT+IO+APP | D | Gespeicherte Formel bleibt vollständig editierbar | T 17.6 |
| MATH-011 | OMML lesen/schreiben und MathML austauschen | Formel / Import/Export | IO+MAT | X | Operatorbaum statt Bild nach DOCX-Roundtrip erhalten | U; W-OOXML |
| MATH-012 | Handschriftliche Formeleingabe mit Korrektur vor Einfügen | Zeichnen / Formel erkennen | MAT+SVC+GUI | X | Erkennungsergebnis wird erst nach Bestätigung Dokumentinhalt | T 17.4 |
| MATH-013 | Alte EQ-Felder und eingebettete Formelobjekte erkennen, erhalten, gezielt konvertieren | Import / Formelkonvertierung | IO+MAT+FLD | R | Nicht konvertierbares Objekt bleibt erhalten und gekennzeichnet | T 19.2; P 8.3-8.4 |
| MATH-014 | Semantische Beschreibung, Lesereihenfolge und zugängliche Formelausgabe | Formel / Barrierefreiheit | MAT+OUT+GUI | D | Formel ist über Tastatur und assistive Ausgabe erschließbar | U; A |

### 6.11 Bilder, Zeichnungen und eingebettete Objekte

| ID | Funktion und Aufbau | Bedienort | Architektur | Austausch | Kennzeichnende Abnahme | Quelle |
|---|---|---|---|---|---|---|
| OBJ-001 | Rasterbilder aus Datei/Clipboard einfügen, ersetzen und als eigene Ressource speichern | Einfügen / Bild | GFX+IO+DOC | R | EXIF-Orientierung, Alpha und Farbprofil korrekt | T 16.1; B 8.1 |
| OBJ-002 | Größe, Seitenverhältnis, Drehen, Spiegeln, Zuschneiden und Formbeschnitt | Bildformat / Größe | GFX+DOC | D | Originalbild bleibt für Zuschneiden-Zurücksetzen erhalten | T 16.1.4; B 8.2 |
| OBJ-003 | Helligkeit, Kontrast, Farbe, Transparenz, künstlerische Effekte und Hintergrundentfernung | Bildformat / Anpassen | GFX+DOC | D | Effektparameter reproduzierbar, destruktiven Export benennen | B 8.2; U |
| OBJ-004 | Bildkompression, Zielauflösung und Löschen abgeschnittener Bereiche mit Vorschau | Bildformat / Komprimieren | GFX+IO | R | Dateigröße und irreversible Entfernung vor Anwendung kenntlich | T 16.1.3 |
| OBJ-005 | SVG und Office-Vektorgrafiken importieren; eigenes Icon-/Illustrationsangebot | Einfügen / Vektorgrafik | GFX+IO | R | Vektoren bei Zoom scharf; unbekannte Elemente erhalten/melden | T 16.3; B 8.5 |
| OBJ-006 | Formen mit Geometrie, Füllung, Verlauf, Linie, Pfeilen, Schatten und Text | Einfügen / Formen | GFX+DOC+LAY | D | Verbinder und Formtext bleiben getrennt editierbar | T 16.3; B 8.4 |
| OBJ-007 | Zeichenfläche, Auswahlbereich, Gruppieren/Entgruppieren und Z-Reihenfolge | Format / Anordnen | GFX+DOC+GUI | D | Gruppieren verändert Anker und relative Positionen nicht unbeabsichtigt | T 16.3.2-16.3.3 |
| OBJ-008 | Ausrichten, Verteilen, Raster/Fanghilfen und präzise Positionen | Layout / Anordnen | GFX+LAY+GUI | D | Zahlenwerte und Ziehen liefern konsistente Geometrie | T 16.7.4; B 8.4 |
| OBJ-009 | Inline, Quadrat, eng, durch, oben/unten, vor/hinter Text; Kontur bearbeiten | Objekt / Textumbruch | DOC+LAY | D | Kontur, Anker und Abstand bei Seitenwechsel erhalten | T 16.7; B 8.3 |
| OBJ-010 | Anker sperren, mit Text verschieben, seiten-/rand-/absatzrelative Position | Objekt / Position | DOC+LAY | D | Absatz verschieben und Querformatwechsel korrekt | T 16.7.3-16.7.6 |
| OBJ-011 | Strukturdiagramme mit Baum-/Listen-/Prozessmodell und eigenen Layoutvorlagen | Einfügen / Strukturdiagramm | GFX+DOC | D | SmartArt-Daten/Layouts beim Import prüfen; Vorschau nicht als vollständige Bearbeitung zählen | T 16.4; B 8.5 |
| OBJ-012 | Diagramme: Säule/Balken/Linie/Kreis/Fläche/XY/Blase/Kurs und Kombinationen | Einfügen / Diagramm | GFX+DOC | D | Achsen, Serien, Legenden, Datenbeschriftungen und Fehlerbalken editierbar | T 16.5; E S. 32-33; U |
| OBJ-013 | Diagrammdaten intern bearbeiten, aus Tabellen importieren oder extern verknüpfen | Diagramm / Daten | GFX+IO+DOC | R | Datenwechsel aktualisiert Grafik ohne fremde Tabellenkalkulation | T 16.5.1-16.5.2 |
| OBJ-014 | Dekorativer Text mit Kontur/Füllung/Transformation und eigenem Vorlagenangebot | Einfügen / Texteffekt | GFX+TXT+DOC | D | Text bleibt Text; WordArt-Import separat prüfen | B 8.5; U |
| OBJ-015 | Screenshot/Ausschnitt nach bewusster Auswahl als Bild übernehmen | Einfügen / Bildschirmaufnahme | PLAT+GFX+APP | R | Aufnahme und Abbruch ohne ungewählte Inhalte | T 16.2 |
| OBJ-016 | Eingebettete/verknüpfte OLE-/Kalkulationsobjekte mit Vorschau und Originalressource | Einfügen / Objekt | IO+DOC+SEC | R | Fremde Objektbearbeitung nur über vorhandenen Adapter; kein Schein-Support | T 15.5, 16.6; U |
| OBJ-017 | 3D-Modellobjekte mit Kamera/Ansicht, Vorschau und Originalmodell | Einfügen / 3D-Modell | GFX+IO+DOC | R | Modellformat und editierbare Kamera von statischer Vorschau trennen | B 8.5 |
| OBJ-018 | Audio-/Video-/Onlinevideo-Objekte mit Poster, Link und Wiedergabeanbieter | Einfügen / Medien | DOC+IO+SVC | R | Offline erhaltene Poster/Links ohne automatischen Netzabruf | S 6; U |
| OBJ-019 | Freihandstriche, Druck, Stifte, Radierer, Lasso und Umwandlung in Formen/Text | Zeichnen | GFX+DOC+PLAT | D | Striche als editierbare Daten; Erkennung gesonderter Anbieter | B 8.5; T 17.4 |
| OBJ-020 | Objektname, Alternativtext, dekoratives Kennzeichen und Lesereihenfolge | Objekt / Barrierefreiheit | DOC+GUI+OUT | M | Umordnen visueller Ebenen zerstört semantische Reihenfolge nicht | B 8.2; T 24.1 |

### 6.12 Felder und eigene Automatisierung

| ID | Funktion und Aufbau | Bedienort | Architektur | Austausch | Kennzeichnende Abnahme | Quelle |
|---|---|---|---|---|---|---|
| FLD-001 | Feldgalerie/-dialog und Tastatureingabe echter Feldbegrenzungen | Einfügen / Feld | FLD+DOC+GUI | F | Getippte Klammern nicht mit Feldstruktur verwechseln | T 19.1; P 10.3 |
| FLD-002 | Feldcode/Ergebnis umschalten, Schattierung, Sperren/Entsperren und Auflösen | Feld / Kontextmenü | FLD+DOC | F | Einzelnes Feld und gesamtes Dokument getrennt schaltbar | T 19.1 |
| FLD-003 | Verschachtelte Felder, Schalter, Sprach-/Zahl-/Datumsformate und Formatübernahme | Feld / Eigenschaften | FLD+BAS+DOC | F | Unbekannte Schalter erhalten und diagnostizieren | T 19.2; P 10.3 |
| FLD-004 | PAGE, NUMPAGES, SECTION, SECTIONPAGES, STYLEREF und Dokumentstatistik | Einfügen / Feld | FLD+LAY | F | Werte ändern sich mit Abschnitt und abgeschlossenem Layout | T 2.6, 4, 20 |
| FLD-005 | DATE, TIME, CREATEDATE, SAVEDATE, PRINTDATE, DOCPROPERTY, FILENAME | Einfügen / Feld | FLD+DOC+PLAT | F | Dynamische Werte erst bei definierter Aktualisierung verändern | B 9.1; T 19 |
| FLD-006 | REF, PAGEREF, NOTEREF, SEQ, LISTNUM und LINK/HYPERLINK-Bezüge | Einfügen / Feld | FLD+DOC+LAY | F | Bezüge bleiben nach strukturierter Bearbeitung stabil | T 5, 19-20 |
| FLD-007 | TOC, TC, INDEX, XE, TA, TOA, RD, CITATION und BIBLIOGRAPHY | Referenzen / Felder | FLD+BIB+LAY | F | Quellmarken und erzeugte Ergebnisse getrennt speichern | T 5, 19 |
| FLD-008 | IF, Vergleich, Rechenfelder, Zahlenformate, SET/ASK/FILLIN und Formularbezüge | Feld / Logik | FLD+DOC | F | Division durch null, ungültige Zahl und zyklischer Bezug sichtbar | T 20.3; P 10.3, 13 |
| FLD-009 | INCLUDETEXT, INCLUDEPICTURE und externe Feldbeziehungen mit kontrollierter Aktualisierung | Feld / Verknüpfungen | FLD+IO+SEC | F | Keine stillen Netz-/Dateizugriffe beim bloßen Öffnen | T 22.3.2; A |
| FLD-010 | Update-Graph, Dirty-Zustand, Fehlerliste, Story-übergreifende Aktualisierung | Referenzen / Aktualisieren | FLD+DOC+LAY | F | Zyklen und Nichtkonvergenz begrenzt melden | T 19.1.4; A |
| FLD-011 | Unbekannte/Legacy-Felder vollständig mit Ergebnis erhalten | Import / Kompatibilität | IO+FLD | F | Kein unerkannter Code wird als erfolgreich berechnet markiert | U; A |
| FLD-012 | Eigene aufgezeichnete Befehlsfolgen mit Parametern, Vorschau, Abbruch und Undo | Entwickeln / Aktionen | DOC+EXT+APP | X | Wiedergabe benötigt dieselben Rechte wie Handbedienung; kein VBA | P 10.1, 10.4; U |
| FLD-013 | Weitere Metadaten-/Benutzer-/Dokumentvariablenfelder mit Originalcode | Einfügen / Feld / Eigenschaften | FLD+DOC+APP | F | Benutzerprofil und gespeicherte Dokumentwerte nicht verwechseln | W-FIELDS |
| FLD-014 | AutoText-/Auswahlfelder, QUOTE, SYMBOL, ADVANCE und GotoButton | Einfügen / Feld / Text | FLD+DOC+LAY | F | Text-/Navigationsfelder bleiben editierbar; Positionswirkung korrekt | W-FIELDS |
| FLD-015 | Legacy-AutoNum-Familie und weitere Seriendruckzähler | Einfügen / Feld / Nummerierung | FLD+DOC | F | Alte Nummerierungen nicht still in andere Zählregeln umdeuten | W-FIELDS |
| FLD-016 | Feldfähigkeitsregister mit Berechnen, Import/Erhalt, Adapter oder Ausschluss je Code | Feld / Diagnose | FLD+IO+EXT | F | Jeder bekannte Code hat expliziten Status und zugehörigen Test | W-FIELDS; U |

Ergänzendes Feldinventar zu FLD-016, abgeglichen mit W-FIELDS. Die Gruppen sind
Implementierungszuordnungen; Schalter und verschachtelte Formen gehören jeweils dazu.

| Feldgruppe | Codes / Zuordnung |
|---|---|
| Eigenschaften | AUTHOR, COMMENTS, DOCPROPERTY, DOCVARIABLE, FILENAME, FILESIZE, INFO, KEYWORDS, LASTSAVEDBY, REVNUM, SUBJECT, TEMPLATE, TITLE, USERADDRESS, USERINITIALS, USERNAME; FLD-005/013 |
| Datum / Statistik / Seiten | CREATEDATE, DATE, PRINTDATE, SAVEDATE, TIME, NUMCHARS, NUMPAGES, NUMWORDS, PAGE, SECTION, SECTIONPAGES; FLD-004/005 |
| Text / Logik | ADVANCE, ASK, AUTOTEXT, AUTOTEXTLIST, COMPARE, FILLIN, GOTOBUTTON, IF, QUOTE, SET, SYMBOL, =; FLD-008/014 |
| Zählen / Referenzen | AUTONUM, AUTONUMLGL, AUTONUMOUT, LISTNUM, NOTEREF, PAGEREF, REF, SEQ, STYLEREF; FLD-006/015 |
| Verzeichnisse / Wissenschaft | BIBLIOGRAPHY, CITATION, EQ, INDEX, RD, TA, TC, TOA, TOC, XE; FLD-007, MATH-013 |
| Serienausgabe | ADDRESSBLOCK, GREETINGLINE, MERGEFIELD, MERGEREC, MERGESEQ, NEXT, NEXTIF, SKIPIF; MAIL-004/005, FLD-015 |
| Beziehungen / externe Aktionen | DATABASE, EMBED, HYPERLINK, INCLUDEPICTURE, INCLUDETEXT, LINK, PRIVATE; FLD-009/011, OBJ-016, SEC-008 |
| Ausführung / Gerätebefehle | MACROBUTTON und PRINT: Original/Ergebnis erhalten; keine Makroausführung oder ungeprüfte Druckerbefehle; SEC-007/008 |

Formularfelder und weitere im Testkorpus vorkommende Erweiterungen werden ergänzend
erfasst. Adapter-/Erhaltstatus ist kein Ersatz für berechnete Semantik. Der Ausschluss
ungeprüfter Gerätebefehle ist eine deklarierte Verhaltensgrenze der Ausgabe.

### 6.13 Sprache und Korrektur

| ID | Funktion und Aufbau | Bedienort | Architektur | Austausch | Kennzeichnende Abnahme | Quelle |
|---|---|---|---|---|---|---|
| LANG-001 | Sprache pro Textbereich und Standard, automatische Erkennung mit Override | Überprüfen / Sprache | DOC+TXT | D | Fremdsprachiges Zitat beeinflusst nicht restlichen Absatz | T 11.1.3; S 5 |
| LANG-002 | Offline-Rechtschreibung während Eingabe und gesamtes Dokument prüfen | Überprüfen / Korrektur | TXT+DOC+GUI | D | Überspringen/Ersetzen/Alle gleichartigen Fälle nachvollziehbar | T 11.1; B 2.6 |
| LANG-003 | Benutzer-/Fachwörterbücher, Wortlisten, Ausschlüsse und Prüfsprache | Einstellungen / Wörterbücher | TXT+APP | P | Fachbegriff in passendem Wörterbuch, nicht global ungeprüft ignorieren | T 11.1.4-11.1.5 |
| LANG-004 | Grammatik-/Stilregeln, Erklärungen, Kategorien und benutzerspezifische Ausnahmen | Korrekturbereich | TXT+EXT+GUI | P | Vorschlag nur nach Übernahme ändern; Dienstabhängigkeit sichtbar | T 11.2; B 2.6 |
| LANG-005 | AutoKorrektur mit formatierten/unformatierten Ersetzungen, Ausnahmen und Sicherung | Einstellungen / AutoKorrektur | TXT+DOC+APP | P | Korrektur unmittelbar rücknehmbar, eigene Regeln exportierbar | T 11.3; B 2.7 |
| LANG-006 | AutoFormat für Listen, Anführungszeichen, Brüche, Ordinalzahlen, Bindestriche und Links | Einstellungen / Eingabe | TXT+DOC | D | Einzelne Regeln schaltbar; Fachnotation nicht ungefragt ändern | T 7.3-7.4, 11.3; B 2.7 |
| LANG-007 | Thesaurus mit Sprache und bedeutungsbezogener Ersetzung | Überprüfen / Thesaurus | TXT+EXT+GUI | X | Gewählte Ersetzung übernimmt passenden Formatkontext | T 21.3; S 5 |
| LANG-008 | Übersetzung von Auswahl oder Dokument als Vorschlag/Kopie | Überprüfen / Übersetzen | SVC+DOC+APP | X | Original bleibt erhalten; Übertragung vor Nutzung erkennbar | T 21.4; B 9.3 |
| LANG-009 | Wort-/Zeichen-/Absatz-/Zeilen-/Seitenzählung mit Auswahl und Notenoptionen | Überprüfen / Statistik | DOC+TXT+LAY | D | Zählumfang einschließlich Textfelder/Noten explizit | T 20.1; B 2.11 |
| LANG-010 | Lesbarkeit, geschützte Begriffe und Konsistenzhinweise | Korrekturbereich | TXT+DOC | D | Keine Bewertung als wissenschaftlicher Wahrheitsnachweis ausgeben | T 11.2; A |
| LANG-011 | Diktat/Transkription mit Korrektur und ausgewähltem lokalen/Onlineanbieter | Start / Diktieren | SVC+EXT+DOC | X | Pausieren/Abbruch stoppt Eingabe und Aufnahme | U; A |
| LANG-012 | OCR/Scannerimport mit Seitenbild, erkanntem Text und Bereinigung | Einfügen / Text erkennen | IO+EXT+SVC | X | Erkennungsfehler korrigierbar; Quelle bleibt zuordenbar | T 10.5 |

### 6.14 Überarbeitung, Kommentare und Schutz der Bearbeitung

| ID | Funktion und Aufbau | Bedienort | Architektur | Austausch | Kennzeichnende Abnahme | Quelle |
|---|---|---|---|---|---|---|
| REV-001 | Änderungen an Text, Format, Listen, Tabellen und Verschiebungen erfassen | Überprüfen / Änderungen | DOC+GUI | M | Einfügen und Löschen bleiben getrennt rekonstruierbar | T 23.2; P 11 |
| REV-002 | Autoren, Zeitpunkte, Revisions-IDs und benutzerbezogene Darstellung | Änderungen / Optionen | DOC+APP | M | Autorfarben unterscheiden nicht allein durch Farbe | T 23.2.4 |
| REV-003 | Original, endgültig, einfaches/gesamtes Markup, Inline und Sprechblasen | Änderungen / Ansicht | DOC+LAY+GUI | P | Ausblenden löscht keine Revisionen | T 23.2-23.3 |
| REV-004 | Änderung einzeln, nach Autor/Typ oder alle annehmen/ablehnen | Überprüfen / Annehmen/Ablehnen | DOC | M | Noten, Quellen, Anker und Tabellen nach Entscheidung gültig | T 23.3.2 |
| REV-005 | Änderungsnavigation und Überarbeitungsbereich mit Filter | Überprüfen / Navigation | DOC+GUI | M | Filter versteckt, verwirft aber keine Änderungen | T 23.2; B 9.5 |
| REV-006 | Kommentare mit Bereichsanker, Antworten, Bearbeiten, Löschen und Erledigtstatus | Überprüfen / Kommentare | DOC+GUI | M | Anker bleibt bei Einfügen/Verschieben nachvollziehbar | T 23.5; B 9.5 |
| REV-007 | Dokumente vergleichen mit auswählbaren Inhalts-/Formatkategorien | Überprüfen / Vergleichen | DOC+IO | M | Vergleich erzeugt neues Ergebnis und ändert Originale nicht | T 23.4; P 11.2 |
| REV-008 | Revisionsstände verschiedener Bearbeiter zusammenführen und Konflikte zeigen | Überprüfen / Kombinieren | DOC+IO+GUI | M | Widersprüchliche Änderungen nicht still auswählen | P 11; U |
| REV-009 | Formatierung/Bearbeitung einschränken, Ausnahmen und Formularmodus | Überprüfen / Schutz | DOC+SEC+GUI | M | Gesperrte Bereiche über Tastatur/Clipboard ebenfalls geschützt | T 23.6; B 10.4 |
| REV-010 | Änderungsverfolgung erzwingen und Schutz aufheben | Schutz / Optionen | DOC+SEC | M | Bearbeitungsschutz nicht als Verschlüsselung bezeichnen | T 23.6.3-23.6.4 |
| REV-011 | Dokumentinspektion für Kommentare, Revisionen, verborgenen Text und persönliche Metadaten | Datei / Prüfen | DOC+IO+SEC | M | Bereinigung als überprüfbare Kopie; versteckte Daten benennen | T 23.7, 24.2.2 |
| REV-012 | Dokument abschließen, schreibgeschützt empfehlen und Veröffentlichungsfassung erzeugen | Datei / Abschließen | DOC+IO+APP | M | Abschlussmarkierung ist keine kryptografische Zugriffssperre | T 23.9 |
| REV-013 | Erledigte Kommentare filtern, wiederöffnen oder gesammelt löschen | Kommentare / Erledigt | DOC+GUI | M | Sammellöschung betrifft nur erledigte Threads und ist rücknehmbar | W-2024 |

### 6.15 Formulare und Seriendruck

| ID | Funktion und Aufbau | Bedienort | Architektur | Austausch | Kennzeichnende Abnahme | Quelle |
|---|---|---|---|---|---|---|
| FORM-001 | Inhaltssteuerelemente für Text/Rich Text, Datum, Bild, Auswahl, Kombinationsfeld und Checkbox | Entwickeln / Steuerelemente | DOC+GUI | D | Typgerechte Eingabe und Auswahlzustand nach Roundtrip erhalten | B 10.3; P 13 |
| FORM-002 | Titel, Tags, Platzhalter, Format, Sperren und Eigenschaften von Steuerelementen | Entwickeln / Eigenschaften | DOC+GUI | M | Gesperrtes Element nicht versehentlich entfernen | B 10.3-10.4 |
| FORM-003 | Wiederholungsabschnitte und Gruppen mit verschachtelten Eingabeelementen | Entwickeln / Wiederholen | DOC+GUI | D | Neue Instanz erhält eigene IDs und gültige Datenbindung | U; W-OOXML |
| FORM-004 | XML-Datenbindung und Zuordnung zu Dokumentteilen | Entwickeln / Datenbindung | DOC+BAS+IO | D | Mehrfach gebundene Anzeige bleibt konsistent | U; W-OOXML |
| FORM-005 | Legacy-Text-/Checkbox-/Auswahlfelder ohne VBA-Ausführung | Entwickeln / Legacy-Felder | DOC+IO+FLD | D | Dokument aus älterem Word ausfüllbar ohne Makros | B 10.3; U |
| FORM-006 | Entwurfs-/Ausfüllmodus, Tabreihenfolge, Hilfetexte und Eingabeprüfung | Entwickeln / Formularmodus | DOC+GUI | D | Formular vollständig per Tastatur ausfüllbar | B 10.3; A |
| FORM-007 | Formular-/Bereichsschutz und gezielte Freigabe einzelner Eingaben | Überprüfen / Schutz | DOC+SEC | M | Geschütztes Layout bleibt beim Einfügen aus Clipboard erhalten | B 10.4 |
| FORM-008 | Vorlagenverwaltung und Export strukturierter Formulardaten | Datei / Formular | IO+DOC | X | Leere, ungültige und nicht ausgefüllte Werte unterscheiden | S 1; U; A |
| MAIL-001 | Serienbrief, E-Mail, Umschlag, Etikett und Verzeichnis als Ausgabearten | Sendungen / Starten | DOC+APP | D | Wechsel des Typs zeigt Auswirkungen auf Layout | B 11; P 13; E S. 43-46 |
| MAIL-002 | Daten aus CSV/TSV, Tabellenkalkulationsdateien, eigenen Listen und Anbietern lesen | Sendungen / Empfänger | IO+EXT+APP | X | Trennzeichen, Encoding, Blatt/Bereich und Spalten vorab wählen | B 11.1; U |
| MAIL-003 | Empfängerliste erstellen, bearbeiten, sortieren, filtern und Datensätze ausschließen | Sendungen / Empfänger bearbeiten | DOC+IO+GUI | X | Führende Nullen in Postleitzahlen erhalten | B 11.1, 11.4 |
| MAIL-004 | Mergefelder, Adressblock und Grußzeile mit expliziter Feldzuordnung | Sendungen / Felder | FLD+DOC | F | Fehlende Anrede/Adresse als prüfbarer Fall, nicht erfundener Wert | B 11.2 |
| MAIL-005 | IF, NEXT/NEXTIF, SKIPIF, ASK/FILLIN und weitere Seriendruckregeln | Sendungen / Regeln | FLD+DOC | F | Datensatzwechsel und Bedingungen deterministisch | B 11.3; P 13 |
| MAIL-006 | Datensatzvorschau, Vorwärts/Rückwärts und Fehlerprüfung vor Ausgabe | Sendungen / Vorschau | DOC+FLD+LAY | F | Leere Daten, lange Namen und Randfälle vor Ausgabe prüfen | B 11.2 |
| MAIL-007 | Zusammenführen in Dokument, Einzeldokumente, PDF oder Druckauftrag | Sendungen / Fertigstellen | DOC+OUT+IO | X | Gewählter Datensatzbereich erzeugt exakt erwartete Empfänger | B 11.2; P 13 |
| MAIL-008 | E-Mail-Ausgabe über eigene Schnittstelle mit Vorschau und explizitem Versand | Sendungen / E-Mail | SVC+EXT+APP | X | Keine Nachricht allein durch Öffnen oder Vorschau versenden | E S. 46; U |
| MAIL-009 | Umschläge/Etiketten: Maße, Abstände, Druckzufuhr, Position und benutzerdefinierte Bögen | Sendungen / Umschläge/Etiketten | DOC+LAY+OUT | D | Kalibrierbare Position; Restbogen ab gewählter Zelle nutzen | B 5.8, 11.5 |
| MAIL-010 | Hauptdokument samt Datenquellenbezug speichern oder Verbindung ausdrücklich lösen | Sendungen / Dokumenttyp | DOC+IO | M | Erneutes Öffnen fragt fehlende Quelle ab und erhält Feldstruktur | B 11.2 |

### 6.16 Ansichten, Barrierefreiheit und Internationalisierung

| ID | Funktion und Aufbau | Bedienort | Architektur | Austausch | Kennzeichnende Abnahme | Quelle |
|---|---|---|---|---|---|---|
| VIEW-001 | Drucklayout, Entwurf, Weblayout, Gliederung und Lesemodus | Ansicht / Dokumentansichten | GUI+LAY+DOC | P | Ansichtswechsel ändert weder Inhalt noch Auswahlanker | B 1.5; E S. 62-66 |
| VIEW-002 | Fokus/Vollbild, Lesespalten, Seitenfarbe und lesefreundliche Textdarstellung | Ansicht / Lesen | GUI+LAY | P | Lesedarstellung verändert nicht gespeicherten Satz | B 9.4; U |
| VIEW-003 | Zoomstufen, Seitenbreite, ganze/mehrere Seiten und horizontales Blättern | Ansicht / Zoom; Statusleiste | GUI+LAY | P | Cursor-/Hit-Position bei jedem Zoom korrekt | B 1.5; T 8 |
| VIEW-004 | Lineale, Gitternetz, Textbegrenzungen, Absatz-/Objektanker und Weißraum anzeigen | Ansicht / Anzeigen | GUI+LAY | P | Hilfsanzeigen sind nicht druckbar | T 1.2.3; B 1.5 |
| VIEW-005 | Navigation nach Überschriften, Seitenminiaturen und Suchtreffern | Navigationsbereich | DOC+LAY+GUI | P | Große Gliederung bleibt nach Kapitelverschiebung konsistent | T 8.1.2; B 6.5 |
| VIEW-006 | Dokument teilen, unabhängige Ansichten und synchroner Vergleich | Ansicht / Fenster | GUI+DOC+LAY | P | Zwei Ansichten bearbeiten dasselbe Modell ohne getrennte Historien | B 3.7; T 23.4 |
| VIEW-007 | Vorlesen mit Stimme, Geschwindigkeit, Pause und Texthervorhebung | Überprüfen / Vorlesen | EXT+PLAT+GUI | X | Auswahl/Leseposition folgen Wortgrenzen | S 1; B 9.4 |
| VIEW-008 | Zugängliche Rollen, Namen, Werte, Fokus und Textebenen über OS-Brücken | Gesamte Oberfläche | GUI+PLAT+DOC | P | Screenreader kann Menüband, Text, Tabellen und Dialoge bedienen | T 24.1; A |
| VIEW-009 | Dokumentprüfung: Überschriften, Tabellenköpfe, Alternativtext, Linktexte und Lesereihenfolge | Überprüfen / Barrierefreiheit | DOC+OUT+GUI | M | Befund verlinkt genaue Stelle mit Korrekturmöglichkeit | T 24.1 |
| VIEW-010 | Hoher Kontrast, Skalierung, reduzierte Bewegung und gut sichtbarer Fokus | Einstellungen / Darstellung | STY+GUI+PLAT | P | Keine Pflichtinformation ausschließlich über Farbe/Animation | U; A |
| VIEW-011 | RTL-/CJK-Inhalte, Sprach-/Locale-spezifische Zahlen/Datum und Einheiten | Sprache / Einstellungen | TXT+DOC+LAY | D | Deutsche UI kann mehrsprachiges Dokument bearbeiten | P 8; U |
| VIEW-012 | Lesereihenfolge und Navigation in Noten, Formeln und schwebenden Objekten | Dokument / Tastatur | DOC+GUI+LAY | D | Visuelle und semantische Navigation nachvollziehbar | T 24.1; U |

### 6.17 Ausgabe, Drucken und Veröffentlichung

| ID | Funktion und Aufbau | Bedienort | Architektur | Austausch | Kennzeichnende Abnahme | Quelle |
|---|---|---|---|---|---|---|
| OUT-001 | Druckvorschau aus demselben Layoutstand wie Editor und Export | Datei / Drucken | OUT+LAY+GUI | X | Seitenzahl und Umbrüche stimmen bei gleichen Optionen überein | T 24.4.3; B 5.7 |
| OUT-002 | Drucker, Papier/Zufuhr, Kopien, Sortierung, Duplex und Bereich auswählen | Datei / Drucken | OUT+PLAT | X | Seiten-/Abschnittsangaben eindeutig; ungültige Bereiche ablehnen | T 24.4; B 5.7 |
| OUT-003 | Auswahl, aktuelle Seite, gerade/ungerade Seiten, mehrere Seiten pro Blatt und Broschüre | Drucken / Optionen | OUT+LAY | X | Reihenfolge bei Duplex/Broschüre anhand Testbogen korrekt | T 24.4; B 5.1 |
| OUT-004 | Kommentare/Markup, Hintergrund, versteckte Inhalte und Feldupdate bewusst wählen | Drucken / Inhalt | OUT+DOC+FLD | X | Vorschau zeigt tatsächlich gewählte Ausgabe | T 23.7, 24.4.4 |
| OUT-005 | Mehrseitiges PDF mit Text/Vektoren, Links, Lesezeichen und Metadaten | Datei / PDF exportieren | OUT+LAY+TXT | X | Text ist such-/kopierbar, keine reine Bildseiten-Ausgabe | T 24.3.1; B 3.8 |
| OUT-006 | Schriftuntergruppen, Einbettungsrechte, Bilder/Farbprofile und Exportqualität | PDF / Optionen | OUT+TXT+GFX | X | Fehlende/nicht einbettbare Schrift explizit melden | T 23.1, 24.2.1; W-FONT |
| OUT-007 | Getaggtes PDF mit Überschriften, Tabellen, Lesereihenfolge, Alttext und Sprache | PDF / Barrierefrei | OUT+DOC | X | Strukturbaum mit unabhängigen Prüfwerkzeugen nachweisbar | T 24.1; U |
| OUT-008 | Archiv-PDF und zugängliche PDF-Profile mit ausdrücklich festgelegter Version | PDF / Profil | OUT+SEC | X | PDF/A-/PDF/UA-Bezeichnung erst nach passender Konformitätsprüfung | U; A |
| OUT-009 | HTML/MHTML und Text als gezielter Export mit Verlustbericht | Datei / Export | IO+DOC | X | Fußnoten/Links nach gewähltem Zielformat erreichbar | B 3.8; T 24.3 |
| OUT-010 | EPUB-Ausgabe für digitale Publikation mit Navigation, Bildern und Metadaten | Datei / E-Book exportieren | IO+DOC+OUT | X | Lesereihenfolge/Verzeichnis im EPUB-Reader prüfen | T 24.3.2 |
| OUT-011 | PDF öffnen/re-konstruieren mit Layout- und Erkennungsbericht | Datei / Öffnen | IO+DOC+LAY | X | Rekonstruierter Text wird nicht als originaler DOCX-Aufbau bezeichnet | T 10.4 |
| OUT-012 | Druckfertige Publikationskopie mit aktualisierten Verweisen und geprüften Ressourcen | Datei / Veröffentlichen | DOC+FLD+OUT | X | Vorhandene Quellarbeit bleibt bearbeitbar und unverändert | T 23.9, 24 |
| OUT-013 | XPS-Ausgabe als eigenständiger Festlayout-Codec | Datei / Export | OUT+IO | X | Ohne Windows-Druckertreiber auf beiden Zielplattformen prüfbar | U; A |

### 6.18 Dateisicherheit und Datenintegrität

| ID | Funktion und Aufbau | Bedienort | Architektur | Austausch | Kennzeichnende Abnahme | Quelle |
|---|---|---|---|---|---|---|
| SEC-001 | Untrusted-Datei-/geschützte Ansicht mit nachvollziehbarem Vertrauensstatus | Öffnen / Hinweisleiste | SEC+IO+APP | P | Öffnen führt keine Makros, Add-ons oder externen Referenzen aus | T 10.1; B 3.4 |
| SEC-002 | Passwortgeschützte Office-Dateien lesen/schreiben nach jeweiligem Kryptoformat | Datei / Verschlüsseln | SEC+IO | X | Falsches Passwort unterscheidet sich von beschädigter Datei | B 3.10; W-FORMAT |
| SEC-003 | Verschlüsselung, Bearbeitungsschutz, Schreibschutz und Rechteverwaltung unterscheiden | Datei / Informationen | SEC+DOC+GUI | M | UI behauptet keine Zugriffssicherheit für reine Editiersperre | T 23.6; A |
| SEC-004 | Signaturen prüfen, Signaturstatus anzeigen und eigene Signieradapter vorsehen | Datei / Signaturen | SEC+IO+EXT | M | Inhaltsänderung kann Signatur ungültig machen und wird angezeigt | U; A |
| SEC-005 | Grenzen für ZIP-Entpackung, XML-Tiefe, Beziehungen, Bilder, Felder und Rekursion | Import / Diagnose | BAS+IO+SEC | X | Zip-Bomb, Pfadtraversal und externe XML-Entitäten führen nicht zu Zugriffen/Absturz | A |
| SEC-006 | Unbekannte Paketbestandteile samt Beziehungen und Originalbytes erhalten | Speichern / Kompatibilität | IO+DOC | R | Nicht bearbeitete unbekannte Teile bleiben identisch, unerfüllbare Erhaltung melden | U; W-OOXML |
| SEC-007 | DOCM/DOTM-/DOC-Makrostreams inaktiv durchreichen | Öffnen / Speichern | IO+SEC | R | Makrobytes bleiben im passenden Format erhalten, Ausführung findet nie statt | U |
| SEC-008 | Externe Bilder, Vorlagen, Links, DDE/OLE- und Netzwerkbezüge inventarisieren | Datei / Verknüpfungen | IO+SEC+APP | M | DDE-/AutoStart-Inhalte niemals beim Öffnen ausführen | A; U |
| SEC-009 | Atomisches Speichern, Speicher-/Plattenfehler, Abbruch und Crash-Recovery | Speichern / Wiederherstellen | IO+PLAT+DOC | X | Fehler lässt Original oder vollständige neue Datei, keine stille Teilüberschreibung | A |
| SEC-010 | Rechteverwaltete Dokumente/Labels erkennen und über autorisierte Adapter behandeln | Datei / Informationen | IO+SEC+EXT | M | Ohne benötigten Dienst klarer Status; keine Umgehung oder Scheinentschlüsselung | U; A |

### 6.19 Erweiterungen, Dienste und technische Betriebsfähigkeit

| ID | Funktion und Aufbau | Bedienort | Architektur | Austausch | Kennzeichnende Abnahme | Quelle |
|---|---|---|---|---|---|---|
| EXT-001 | Eigenes Add-on-Paket mit Manifest, Version, Hostanforderung und Fähigkeiten | Entwickeln / Erweiterungen | EXT+BAS+SEC | X | Unbekannte Pflichtversion vor Installation ablehnen | U |
| EXT-002 | Installieren, Aktivieren, Deaktivieren, Entfernen und kontrollierte Updates | Erweiterungsverwaltung | EXT+APP+PLAT | P | Deaktivierung erhält Dokumentdaten des Add-ons | U |
| EXT-003 | Erweiterungsbefehle, Menübandgruppen und Aufgabenbereiche mit eigenen IDs | Menüband / Erweiterungen | EXT+GUI+APP | P | ID-/Shortcut-Konflikte ohne Überschreiben bestehender Befehle | U |
| EXT-004 | Dokument lesen/ändern über Snapshots und atomare Transaktionen | Erweiterungs-API | EXT+DOC | D | Ungültiger Auftrag ändert Dokument nicht teilweise | U; A |
| EXT-005 | Import-/Export-, Quellen-, Zitierstil- und Sprachdienstanbieter | Erweiterungs-API | EXT+IO+BIB | X | Anbieter meldet tatsächliche Fähigkeiten und Verluste | U |
| EXT-006 | Explizite Datei-/Netz-/Dokumentrechte und begrenzte Datenübergabe | Erweiterung / Rechte | EXT+SEC | X | Nicht erlaubter Zugriff scheitert auch außerhalb der UI | U; A |
| EXT-007 | Isolation, Zeit-/Speicherlimits, Abbruch und Wiederanlauf nach Fehler | Erweiterungsverwaltung / Diagnose | EXT+PLAT+SEC | X | Absturz eines Fremdanbieters beschädigt nicht das Dokument | U; A |
| EXT-008 | SDK-Dokumentation, Beispielerweiterung und Version-/Kompatibilitätstests | Hilfe / Entwicklung | EXT | X | Beispiel baut ohne Office/VBA und arbeitet auf beiden Zielplattformen | U |
| EXT-009 | Fremde Office-/COM-/VSTO-/Office.js-/ActiveX-Erweiterungen erkennen, nicht laden | Import / Kompatibilität | IO+EXT+SEC | R | Referenzen erhalten; keine unbelegte Laufzeitkompatibilität behaupten | U |
| SVC-001 | Optionale Speicheranbieter, bewusste Anmeldung und Kontotrennung | Datei / Speicherorte | SVC+EXT+SEC | X | Lokale Dateien bleiben ohne Konto zugänglich | B 3.6; T 22.1; U |
| SVC-002 | Synchronisation, Offlinewarteschlange, Konfliktversionen und Versionshistorie | Datei / Versionen | SVC+DOC+IO | X | Offlineänderungen nach Wiederverbindung nicht still überschreiben | T 22.1, 23.8 |
| SVC-003 | Freigabe mit Lese-/Schreibrechten, Widerruf und Identitätsanzeige | Datei / Freigeben | SVC+SEC+APP | X | Entzogene Schreibrechte werden am Dienst durchgesetzt | B 9.5; T 23.8 |
| SVC-004 | Echtzeitbearbeitung mit Präsenz, Cursor, Kommentaren und Konfliktauflösung | Zusammenarbeit | SVC+DOC+GUI | X | Gleichzeitige Text-/Tabellenänderungen konvergieren überprüfbar | T 23.8; U |
| SVC-005 | KI-Entwurf, Umformulieren, Kürzen, Strukturieren und Zusammenfassen | Eigener Assistenzbereich | SVC+EXT+DOC | X | Ausgabe als Vorschlag; Übernahme als nachvollziehbare Änderung | B 12; T 10.7; P 14 |
| SVC-006 | KI-Chat zum Dokument und begrenzte Auswahl des übertragenen Kontexts | Assistenzbereich | SVC+EXT+SEC | X | Anbieter, Kontext und Abbruch sichtbar; kein automatisches Ganzdokument-Upload | B 12.7; U |
| SVC-007 | Recherche-/Ähnlichkeitsdienste mit Fundstellen und nachvollziehbarer Herkunft | Referenzen / Recherche | SVC+BIB+GUI | X | Ähnlichkeit nicht als Plagiatsbeweis, KI-Ausgabe nicht als geprüfte Quelle darstellen | T 21.2; A |
| SVC-008 | Onlinebilder, Fonts und Vorlagen mit Herkunft und Nutzungslizenz | Einfügen / Online; Vorlagen | SVC+IO+SEC | R | Download/Einbettung nur passend zur ausgewiesenen Lizenz | B 8.1; T 12.1 |
| SVC-009 | Lokale/Online-Sprach-, OCR-, Übersetzungs- und Metadatenanbieter austauschbar | Einstellungen / Anbieter | SVC+EXT | X | Ausfall eines Dienstes blockiert lokales Bearbeiten/Speichern nicht | T 10-11, 21; U |
| SVC-010 | Dienstzustand, Abbruch, Fehler, Authentifizierung und Datenlöschung verständlich zeigen | Dienstbereich / Einstellungen | SVC+SEC+APP | P | Abgemeldeter Anbieter erhält keine weiteren Dokumentdaten | U; A |
| SYS-001 | Headless-Dokumentoperationen und stabile Test-/Dump-Schnittstellen | Kommandozeile / Testmodus | DOC+IO+APP | X | Import, Feldupdate und Export ohne Fenster testbar | Repository GUIDE; A |
| SYS-002 | Große Dokumente mit Lazy-Ressourcen, begrenzten Caches und Hintergrundarbeit | Gesamte Anwendung | DOC+LAY+IO | D | Editor bleibt während Speichern/Paginierung bedienbar | U; A |
| SYS-003 | Fortschritt und Abbruch für Import, Export, Suche, Layout und Serienausgabe | Status / Aufgaben | APP+GUI+DOC | P | Abbruch hinterlässt gültigen vorherigen Zustand | A |
| SYS-004 | Windows-/Linux-Build, Wayland/X11 und DPI-/Theme-Wechsel | Plattform | PLAT+GUI+STY | P | Identisches Dokumentmodell bei gleichen Eingaben auf allen Backends | Repository README; U |
| SYS-005 | Deterministische Verarbeitung bei festen Fonts, Locales und Daten | Dokument-/Codec-Tests | DOC+LAY+BIB+IO | D | Wiederholungen erzeugen gleiche semantische Ausgabe | A |
| SYS-006 | Paketierung, eigene Dateizuordnungen, Updates und saubere Deinstallation | Installation | APP+PLAT | P | Testinstallation nur in Testumgebung, Nutzerdokumente erhalten | Repository PACKAGING; A |
| SYS-007 | Diagnose-/sicherer Start ohne Drittanbieter, beschädigte Konfiguration isolieren und Wiederherstellung anbieten | Startoptionen / Hilfe | APP+EXT+IO | P | Defektes Add-on/Profil verhindert nicht Zugriff auf Nutzerdokumente | T Anhang C.5-C.8 |

<a id="wissenschaft"></a>
## 7. Wissenschaftliche Arbeitsabläufe und Datenfluss

### 7.1 Quellenmodell und Zuständigkeiten

Eine Quelle besitzt eine interne stabile ID, Quelltyp, geordnete Personen-/Körperschaftsrollen,
Publikationsdaten, Datumspräzision, Kennungen und optionale Zusatz-/Originalfelder. Ein
Word-Tag, BibTeX-Schlüssel oder CSL-Schlüssel ist ein externer Bezeichner, kein universeller
Primärschlüssel. Importabbildungen werden gespeichert, damit Kollisionen auflösbar bleiben.

Eine Dokumentquelle ist eine dokumenteigene Fassung. Eine Verknüpfung zur globalen
Bibliothek ermöglicht Abgleich, erzwingt jedoch keine ungefragte Aktualisierung. Dadurch
bleiben eingereichte Arbeiten reproduzierbar. Anlagen und Onlinezugriff sind Zusatzfunktionen;
ohne sie müssen die bibliografischen Angaben aus dem Dokument verfügbar sein.

Ein Zitat besteht aus stabiler Zitat-ID, Dokumentanker, einem oder mehreren Quellelementen,
Fundstellen, Präfix/Suffix, Unterdrückungsoptionen und Stil-/Locale-Kontext. Die sichtbare
Zeichenfolge ist ein abgeleitetes Ergebnis. Literaturverzeichnisse besitzen explizite Bereiche
und Filter; ihre Einträge verweisen auf dieselben Quellen. Manuelle Ausgabeanpassungen sind
entweder strukturierte Overrides oder ausdrücklich statischer Text, niemals still verlorene
Änderungen bei der nächsten Aktualisierung.

### 7.2 Verbindliche Arbeitsabläufe

1. **Erfassen:** Quelle manuell/aus Austauschdatei anlegen -> Typ-/Feldprüfung -> Dubletten-
   vorschau -> globale oder lokale Aufnahme -> zitierbare Dokumentfassung.
2. **Zitieren:** Quelle(n) wählen -> Fundstelle/Darstellung bestimmen -> strukturierten
   Zitatknoten einfügen -> Zitierprozessor berechnet Ausgabe -> ein Undo-Schritt.
3. **Stil ändern:** Stilversion/Locale laden -> alle betroffenen Zitate und Bibliografien
   gemeinsam berechnen -> Disambiguierung/Nummerierung global auswerten -> Ergebnis atomar
   übernehmen -> Dokument erneut umbrechen.
4. **Quelle korrigieren:** lokale/globale Fassung vergleichen -> Ziel der Änderung wählen ->
   alle lokalen Verwendungsorte aktualisieren -> globale Änderung nur ausdrücklich übernehmen.
5. **Arbeiten zusammenführen:** IDs, Styles, Listen, Quellen und Anker in einer Transaktion
   abbilden -> Referenzen reparieren -> Inhalts-/Literaturverzeichnisse neu berechnen.
6. **Publizieren:** fehlende Quellen/Verweise/Fonts prüfen -> Felder und Layout stabilisieren ->
   Revisionen nach gewählter Ausgabe behandeln -> PDF/DOCX-Kopie mit Exportbericht erzeugen.

### 7.3 Satz und Felder dürfen keine veralteten Ergebnisse mischen

Dokumenttransaktion -> Inhalts-/Styleinvalidierung -> Quellen-/Zählerberechnung ->
Zeilen-/Seitensatz -> seitenabhängige Felder und Verzeichnisse -> erneuter Satz bei
Größenänderung. Die Wiederholung besitzt eine feste Grenze und eine Nichtkonvergenzdiagnose.
Ein asynchrones Ergebnis gilt nur für seine Dokumentversion. Automatische Updates dürfen
gesperrte Felder nicht überschreiben. Verweise in Noten, Textfeldern, Kopfzeilen und Tabellen
sind Bestandteil desselben Abhängigkeitsmodells.

### 7.4 Word und CSL

Word nutzt eigene Quellen-/Feldstrukturen und XSL-basierte Bibliografieprofile (W-BIB).
CSL ist eine andere Stilbeschreibung (W-CSL). Ein beliebiger CSL-Stil lässt sich deshalb
nicht automatisch verlustfrei als beliebiger Word-XSL-Stil exportieren.

Standardweg: bibliografische Metadaten, abbildbare CITATION-/BIBLIOGRAPHY-Felder und aktuelle
Ergebnisse exportieren; eigene Zusatzinformationen in dafür zulässigen Erweiterungsteilen
mitführen. Bei nicht abbildbaren Einstellungen zeigt der Bericht, ob Word beim Neuberechnen
abweichen wird. Optional kann der Nutzer eine statische Publikationskopie erzeugen. Diese
zählt nicht als editierbarer Zitat-Roundtrip. Die tatsächliche Erhaltung eigener Zusatzdaten
durch Word wird getestet, nicht vorausgesetzt. Fremde Literatur-Add-in-Felder werden ohne
passenden Datenadapter unverändert erhalten und als fremd gekennzeichnet.

<a id="kompatibilitaet"></a>
## 8. Dateiformate und Kompatibilitätsvertrag

### 8.1 Zielmatrix

Alle folgenden Fähigkeiten sind Zielanforderungen, nicht implementierter Status. Lesen und
Schreiben sind eigene Abnahmerichtungen. DOCX ist der Standard für neue austauschbare
Dokumente; kein proprietäres jfWord-Dokumentformat ist Voraussetzung für Kernfunktionen.
Lokale Wiederherstellungs-/Bibliotheks-/Add-on-Dateien haben eigene versionierte Formate.

| Formatfamilie | Zielrichtung und Bearbeitung | Besonders zu prüfen | Grenzen / auszuweisende Abweichung |
|---|---|---|---|
| DOCX Transitional | Lesen, neu erstellen, bearbeiten, schreiben; Standardausgabe | WordprocessingML, DrawingML/VML, Styles, Nummerierung, Sections, Beziehungen, Medien, Felder, Noten, Revisionen | Legacy-Bestandteile und Word-Erweiterungen separat abdecken |
| DOCX Strict | Lesen/schreiben im gewählten Strict-Profil | Namespaces, zulässige Elemente, Konvertierung von Transitional | Nicht zulässige Inhalte vor Konvertierung melden |
| DOTX | Vorlagen lesen/schreiben und neue Dokumente daraus erstellen | Styles, Bausteine, Dokumenteinstellungen, getrennte Identität | Vorlage beim Anlegen nicht verändern |
| DOCM/DOTM | Dokument-/Vorlagenfunktion plus inaktiver Makroerhalt | VBA-Paketbestandteile, Content Types, Beziehungen, Makrosignaturen | Kein Ausführen/Bearbeiten; DOCX-Konvertierung entfernt Makroanteile nur ausdrücklich |
| DOC/DOT | Binäre Word-97-2003-Familie nach MS-DOC/zugehörigen Spezifikationen lesen/schreiben | CFB, Text-/Formatstrukturen, Codepages, Listen, Felder, Drawing-Daten, Makrostreams | Nicht jedes historisch ebenfalls .doc genannte Fremdformat ist automatisch enthalten |
| RTF | Lesen/schreiben von Text, Format, Tabellen, Noten, Feldern und Objekten | Unicode-Fallbacks, Codepages, verschachtelte Gruppen, unbekannte Destinations | Fehlende Entsprechungen zu modernen OOXML-Funktionen melden |
| ODT/OTT | OpenDocument-Texte/Vorlagen lesen/schreiben; ODF 1.3 und 1.4 als getrennte Zielprofile | Styles, Listen, Sections, Noten, Referenzen, Formeln, Quellenabbildung | ODF-/OOXML-Semantik ist nicht überall gleich; pro Feature und Referenzbuild berichten |
| Word-2003-XML / Flat OPC | XML-Dokument beziehungsweise XML-Paketrepräsentation lesen/schreiben | Unterschiedliche Dialekte erkennen, Paketteile/Ressourcen erhalten | Eine .xml-Endung allein bestimmt keinen Word-Dialekt |
| TXT | Lesen/schreiben; UTF-8/16 mit BOM-Option, relevante Legacy-Codepages | Zeilenenden, Ersatzzeichen, nicht darstellbare Zeichen, Tabulatoren | Format-/Strukturverlust vor Textausgabe ausdrücklich anzeigen |
| HTML/MHTML | Lesen/schreiben mit Text, Bildern, Tabellen, Links und Styles soweit abbildbar | Ressourcen, Zeichensatz, relative Pfade, Mail-/Webexport | Kein Ausführen eingebetteter Skripte; paginiertes Layout kann abweichen |
| PDF | Export; Import als bewusst rekonstruierbares Dokument | Suchbarer Text, Vektoren, Bilder, Tags, Links, eingebettete Fonts; Import-Lesereihenfolge | PDF-Import kann den ursprünglichen Autoren-Dokumentbaum nicht generell rekonstruieren |
| PDF/A, PDF/UA | Gesonderte validierte Exportprofile | Passende Revision, Validator, Fonts, Tags, Metadaten | Normbezeichnung nur für tatsächlich bestandene Profilprüfung |
| EPUB | Export für E-Book-Publikation | Navigation, semantische Kapitel, Ressourcen, Metadaten, Formeln | Umfließender E-Book-Satz ist kein druckidentischer DOCX-Roundtrip |
| XPS | Festlayout-Export | Seiten, Vektoren, Fonts und Ressourcen | Eigenständige Ausgabe, keine vollständige Word-Bearbeitungsrepräsentation |
| Sources.xml / DOCX-Quellen | Bibliografiedaten lesen/schreiben | Tags, Typen, Personenrollen, optionale/originale Felder | Word-Ausgabestil ist nicht CSL |
| BibTeX / BibLaTeX / RIS / CSL-JSON | Bibliografiedaten importieren/exportieren | Schlüssel, Typen, Namen, Daten, Encoding und unbekannte Felder | Jede nicht verlustfreie Feldabbildung melden |
| CSV/TSV/XLSX/ODS als Datenquelle | Tabellen/Seriendruckdaten lesen, eigene Listen exportieren | Zelltypen, Blatt/Bereich, Datum, Zahlenformate, führende Nullen | Kein vollständiger Tabellenkalkulationsklon; Formeln nicht blind neu auswerten |
| Bild-/Vektor-/OLE-/Medienressourcen | Dokumentressourcen erhalten; Bearbeitung nach eigenem Capability-Eintrag | Originalbytes, Beziehungen, Vorschau, Farbräume und Einbettung | Bilderkennung, Vorschau, Bearbeitung und Fremdprogrammaktivierung unterscheiden |

### 8.2 Dimensionen der Kompatibilität

Pro Formatversion und Feature werden getrennte Ergebnisse geführt:

1. **Syntax/Paket:** Datei ist strukturell gültig und wird von Referenzanwendungen geöffnet.
2. **Semantik:** Text, Strukturen, IDs, Beziehungen, Quellen und Felder sind korrekt.
3. **Bearbeitung:** Objekt/Funktion lässt sich tatsächlich ändern, nicht nur anzeigen.
4. **Layout:** Fonts, Zeilen-/Seitenumbrüche, Positionen, Noten und Druck stimmen unter
   dokumentierten gleichen Voraussetzungen mit der Referenz überein.
5. **Neuberechnung:** Felder, Nummerierung, Verzeichnisse, Zitate und Tabellenberechnungen
   ergeben nach Aktualisierung korrekte Ergebnisse.
6. **Erhaltung:** Unbekannte nicht veränderte Bestandteile bleiben nachvollziehbar erhalten.

Ein erfolgreiches Öffnen beweist keine der übrigen Dimensionen. 100 % bedeutet weder
Byteidentität jedes ZIP-Containers noch kann eine bestandene begrenzte Testsammlung alle
denkbaren Dokumente beweisen. Der veröffentlichbare Stand muss daher die getesteten
Versionen, Richtungen, Dimensionen, Ausnahmen und noch offenen Fehler offenlegen.

### 8.3 Erhaltung und bewusste Verluste

- Unbekannte Paketteile, Attribute, Markup-Compatibility-/AlternateContent-Zweige,
  Custom-XML, Beziehungen und Binärressourcen werden inventarisiert und möglichst bewahrt.
  Nach Strukturänderungen müssen ihre Beziehungen weiterhin gültig sein; bloßes Kopieren
  unbekannter Bytes ist kein Beweis semantischer Korrektheit.
- Importbericht und Exportbericht nennen Feature, Fundstelle, Umfang und Alternative.
  Automatische Wiederherstellung darf keinen stillen Formatverlust einführen.
- Makros bleiben in dafür vorgesehenen Formaten inaktiv. Eine makrofreie Kopie ist ein
  expliziter Export. Ungültig gewordene Signaturen dürfen nicht als gültig angezeigt werden.
- Fehlende Fonts erzeugen eine nachvollziehbare Ersetzung. Exakte Layoutaussagen gelten
  nur für festgehaltene Fontdateien/-versionen, Locale und Layout-/Druckeinstellungen.
- Passwortverschlüsselung ist getrennt von Editierschutz. Nicht unterstützte Verschlüsselung
  oder fehlende Rechteanbieter führen zu einer präzisen Fehlermeldung, nicht zum Umgehen.
- Nicht unterstützte Ausführungssysteme (VBA, COM/VSTO/Office.js, ActiveX, DDE-Automatik)
  sind deklarierte Verhaltensausnahmen. Ihre Speicherung kann dennoch unterstützt werden.

<a id="erweiterungen"></a>
## 9. Eigene Erweiterungsschnittstelle

### 9.1 Geplanter Vertrag

Technischer Paketname für v1: `.jfwaddon`, ein ZIP-Paket mit UTF-8-JSON-Manifest,
Ressourcen, Lizenzinformationen und nach Zielplattform getrennten Anbieterprogrammen.
Das Manifest enthält eindeutige Erweiterungs-ID, Version, minimale/maximale API-Version,
Einstiegspunkte, angebotene Fähigkeiten und angeforderte Rechte. Installierte Erweiterungen
liegen außerhalb von Dokumenten; Öffnen einer Datei installiert oder aktiviert nichts.

Dokument-/Quellen-IDs, Befehle, Aufträge und strukturierte Antworten bilden das eigene
Protokoll. Die Fähigkeiten umfassen Commands, deklarative UI-Beiträge, Dokument-/Quellen-
abfragen, Änderungstransaktionen, Codec-/Metadaten-/Sprachanbieter und Exportaufträge.
Host und Anbieter handeln Version/Fähigkeiten vor der ersten Dokumentübergabe aus.

Automatisierung innerhalb jfWord startet als versionierte deklarative Befehlsfolge mit
Parametern; keine neue universelle Programmiersprache ist für den Kern nötig. Komplexe
Erweiterungen nutzen das eigene Anbieterprotokoll. Es wird keine Word-Objektmodell- oder
VBA-Quelltextkompatibilität versprochen.

### 9.2 Vertrauens- und Fehlergrenzen

Drittanbieterprogramme werden nicht als beliebige native Bibliothek in den Editorprozess
geladen. Für sie sind getrennte Prozesse, versionierte Nachrichten, begrenzte Ausgaben und
Abbruch vorgesehen. Tatsächliche Datei-/Netzrechte müssen durch Betriebssystemmechanismen
oder einen nachweisbar eingeschränkten Host durchgesetzt werden. Ein Manifest und eine
Absturzbehandlung allein bilden keine Sandbox. Wo diese Durchsetzung fehlt, darf der Host
die betreffende untrusted Fähigkeit nicht anbieten; das ist ein offener Umsetzungspunkt
des Erweiterungsmeilensteins, kein stillschweigendes Sicherheitsversprechen.

Dokumentänderungen werden vor Commit validiert, bilden einen Undo-Schritt und enthalten
eine erwartete Dokumentversion. Bei veraltetem Snapshot erhält der Anbieter einen Konflikt.
Timeout, Fehler oder Deaktivierung lassen den letzten gültigen Dokumentstand bestehen.
Native erstseitige Bibliotheksmodule können den bestehenden Modullader verwenden; sie sind
technisch und hinsichtlich Vertrauen von installierbaren Drittanbieter-Add-ons zu trennen.

<a id="recht"></a>
## 10. Rechtliche Einordnung und Gestaltungsänderungen

Arbeitsannahme ist Entwicklung und erste Veröffentlichung in Deutschland/EU. Die folgende
Tabelle ist eine quellenbasierte technische Ableitung, keine abgeschlossene Schutzrechts-
oder Markenfreigabe. Für andere Absatzmärkte ist die Prüfung zu erweitern. Die pauschalen
Formulierungen und internen Quellenmarker des Memos L reichen als Freigaben nicht aus.

| Thema / Aussage im Memo | Überprüfte Einordnung | Verbindliche Konsequenz |
|---|---|---|
| Funktionen und Ideen nachentwickeln | W-COPY und W-SAS unterscheiden Programmausdruck von Funktion/Idee; andere Schutzrechte bleiben gesondert relevant | Eigene Implementierung aus Spezifikation und beobachtbarem Verhalten; Herkunft dokumentieren |
| Clean-Room als zwingende Teamform | Zweiteiliges Team kann eine Nachweismethode sein; aus den genannten Grundlagen folgt keine pauschale Pflicht zu genau dieser Organisation | Keine unzulässig erlangten Interna oder fremden Programmtexte übernehmen; keine erfundene Clean-Room-Zertifizierung |
| Oberfläche/Icons nur Designrecht | Nach W-GUI kann auch allgemeiner urheberrechtlicher Schutz einer eigenständigen GUI relevant sein | Gesamteindruck, Icons, Vorlagen und Texte eigenständig entwickeln; reine Farbumstellung reicht nicht als Freigabe |
| Nahe Word-Bedienstruktur | Aus der funktionalen Anordnung folgt weder pauschales Verbot noch pauschale Freigabe für konkrete Gestaltung | Funktionsgruppen vertraut organisieren, visuellen Ausdruck und Herkunft klar eigenständig halten |
| § 4 Nr. 9 UWG | Die aktuelle Nachahmungsregel steht in § 4 Nr. 3 UWG (W-UWG) | Veraltete Paragraphenangabe nicht weiterverwenden; Herkunftstäuschung und Rufausbeutung prüfen |
| DOC sei nicht öffentlich zugänglich / generell zu vermeiden | W-FORMAT veröffentlicht binäre Office-Formate; W-OSP listet MS-DOC | DOC-Unterstützung nicht allein wegen dieser Memoaussage streichen |
| OOXML sei frei von allen Rechten | W-OSP ist ein begrenztes Patentversprechen für erfasste Spezifikationen und erforderliche Ansprüche, keine allgemeine Freigabe | Verwendete Spezifikationen/Revisionen und Abdeckung dokumentieren; Drittpatente nicht als ausgeschlossen behaupten |
| VBA sei generell Geschäftsgeheimnis | Sprachreferenz und MS-OVBA-Dateiformatspezifikation sind öffentlich; W-OSP listet OVBA | VBA wegen ausdrücklicher Produktentscheidung ausschließen, nicht mit pauschaler Geheimhaltungsbehauptung begründen |
| Fremde Fonts/Icons aus vorhandenen Styles | W-FONT unterscheidet installierte Nutzung, Einbettung und Weitergabe; Formatkonvertierung schafft keine neue Lizenz | Verteilbare eigene Assets samt Lizenznachweis; bestehende Microsoft-Font-Binärdateien nicht automatisch übernehmen |
| Word/Office-Namen und jfWord | Konkrete Markenverwendung und Verwechslungsgefahr hängen vom Einsatz ab | Interner Name bleibt Arbeitsname; öffentliches Branding, Logos und Kompatibilitätswerbung gesondert prüfen |
| Fehlende Konkurrenzfeatures hätten bestimmte rechtliche Gründe | Im Memo überwiegend Vermutungen ohne belastbaren Einzelnachweis | Nicht als Tatsachen oder Begründung zum Weglassen eigener Features übernehmen |
| EULA, Dekompilierung, Geschäftsgeheimnisse | Memo enthält verkürzte/veraltete Bezüge; zulässige Beobachtung und Interoperabilitätsausnahmen erfordern genaue Einzelfallprüfung | Primär öffentliche Spezifikationen verwenden; kein pauschales Reverse-Engineering-Verbot und keine allgemeine Erlaubnis behaupten |
| Patentrecherche und Lizenzwahl | Noch keine konkrete FTO-, Designregister-, Marken- oder Projektlizenzprüfung abgeschlossen | Offene Prüfpunkte für Veröffentlichung erfassen; keine Lizenz eigenmächtig festlegen |

Eigene Produkttexte, Vorlagen, wissenschaftliche Beispieldokumente und Testdaten werden neu
erstellt. Referenzbücher liefern Fakten/Arbeitsabläufe, keine zu übernehmende Gestaltung.
Auch frei zugängliche CSL-Styles, Wörterbücher und Fonts benötigen Prüfung ihrer konkreten
Lizenz und Weitergabebedingungen. Fehlender VBA-Support schließt funktional unabhängige
Felder, Formulare, Seriendruck und eigene Automatisierung ausdrücklich nicht aus.

<a id="reihenfolge"></a>
## 11. Umsetzungsreihenfolge

| Stufe | Ergebnis / Schwerpunkt | Eintritt und Abnahme |
|---|---|---|
| P0: Phase 1 | Dieser Katalog, Quellenzuordnung, Architektur, Kompatibilitäts- und Prüfvertrag | Dokumentprüfung, keine behauptete App-Implementierung |
| P1: Grundlagen | BAS, DOC, sichere Container, einfache DOCX-Pipeline, Font-/Textgrundlagen, Transaktionen | Headless Erstellen/Lesen/Ändern/Schreiben, Fehler-/Recoveryfälle |
| P2: Editor und Style | Eigener Office-Style, Dokumentansicht, Eingabe, Styles, Abschnitte, Tabellen, inkrementeller Satz | Tastatur-/Mausabläufe, Screenshots, Dirty-Rendering, bestehende Gallery weiter nutzbar |
| P3: Wissenschaftlicher Kern | BIB, CSL, Zitate, Noten, Felder, Verzeichnisse, Formeln, lange Arbeiten | Wissenschaftliche End-to-End-Szenarien und DOCX-Roundtrips; Teilentwicklungen parallel zum Editor möglich |
| P4: Breite Desktop-Abdeckung | Grafik/Diagramme, Revisionen, Formulare, Seriendruck, Druck/PDF, weitere Formate | Bidirektionale Formatmatrix und reproduzierbare Ausgabe; keine P3-Funktionen durch Platzhalter ersetzen |
| P5: Erweiterbarkeit | Eigene Pakete, API/SDK, Anbieterprozesse und durchgesetzte Rechte | Isolation, Transaktionen, Versionstests; unabhängige Beispielschnittstelle |
| P6: Dienste | Synchronisation, Mehrbenutzerbearbeitung, Recherche, Übersetzung, KI, OCR-/Sprachanbieter | Konflikt-/Offline-/Datenschutzfälle; Desktop-Kern weiterhin kontofrei |
| P7: Veröffentlichung | Kompatibilitätsbericht, zugängliche Ausgabeprofile, Packaging, Asset-/Marken-/Lizenzprüfung | Konkrete Build-/Font-/Spezifikationsstände und offene Fehler dokumentiert |

Bibliografiedatenmodell, Zitatanker und Quellen-Roundtrip müssen bereits in P1/P2 mitgedacht
werden. Sie dürfen nicht nachträglich als bloße Textplatzhalter an einen fertigen Editor
angehängt werden. Bestandsalgorithmen mit Assembly-Paritätsvertrag bleiben unverändert,
sofern keine separat dokumentierte Abweichung nötig ist. Neue jfWord-Funktionen und der
neue Style haben kein Assembly-Gegenstück; für sie gelten eigene Golden-/Semantiktests.

<a id="abnahme"></a>
## 12. Abnahme und Prüfszenarien

### 12.1 Nachweisregeln

Jede Feature-ID erhält bei Implementierung einen Status mit Nachweis: offen, implementiert,
geprüft oder bekannte Abweichung. Ein deaktivierter Menüpunkt, ein statischer Screenshot
oder erhaltene Originalbytes erfüllen kein Feature, das Bearbeitung/Neuberechnung verlangt.

Für Codec- und Layouttests werden ausschließlich eigene/lizenzierte Fixtures verwendet.
Referenztests halten Word-Version/Build, Dokumentformat, Fonts mit Version/Hash, Locale,
Druck-/Layoutoptionen und Betriebssystem fest. Primärreferenz ist Word 2024 für Windows;
Tests mit anderen Office-Versionen/Programmen sind ergänzende Interoperabilitätstests.

Strukturvergleich normalisiert nur ausdrücklich unerhebliche Unterschiede wie ZIP-Zeitstempel
oder Reihenfolge unbedeutender Attribute; Text, Beziehungen, Quellen, Feldcodes und relevante
Metadaten dürfen nicht wegnormalisiert werden. Layoutvergleiche prüfen zunächst Seiten-/
Zeilenumbrüche und Objektgeometrie, dann gerenderte Darstellung. Font-/Rasterdifferenzen
werden dokumentiert statt mit beliebig großer Pixeltoleranz verdeckt.

### 12.2 Verbindliche Szenarien

| Test-ID | Szenario | Erfolgskriterium / betroffene Bereiche |
|---|---|---|
| AT-01 | Eigene 200-seitige Abschlussarbeit mit 12 Kapiteln, römischem Vortext, Querformatseiten, 100 Noten, 50 Abbildungen, 30 Tabellen, 40 Formeln | Alle Strukturen bearbeitbar; korrekte Paginierung und Verzeichnisse; PAGE, TAB, REF, MATH |
| AT-02 | 300 Quellen und 1.200 Zitate, darunter gleiche Autor-/Jahreskombinationen, Körperschaften und unvollständige Daten | Keine ID-Kollision; Disambiguierung, Sortierung und Fehlerhinweise korrekt; SRC, CITE |
| AT-03 | Autor-Jahr -> numerisch -> Fußnotenstil; deutsche/englische Locale; globale Quellenkorrektur | Zitate/Bibliografie konsistent, Quelldaten unverändert, ein Undo je Stilwechsel |
| AT-04 | Zwei Arbeiten mit kollidierenden Quellen-/Style-/Listenkennungen zusammenführen | Alle Ziele richtig umgebunden, keine stillen Dubletten, Wiederöffnung gültig |
| AT-05 | Kapitel verschieben; neue Abbildung, Formel, Fußnote und Tabelle im ersten Drittel einfügen | Querverweise, Zähler, Seitenzahlen und alle Verzeichnisse aktualisieren sich korrekt |
| AT-06 | BibTeX/BibLaTeX, RIS, CSL-JSON und Sources.xml hin/zurück, auch Fehler und Zusatzfelder | Bekannte Daten verlustfrei, übrige Daten erhalten oder konkret gemeldet; keine erfundenen Werte |
| AT-07 | Word -> jfWord -> Word ohne Änderung und mit definierter Änderung | Struktur, Bearbeitbarkeit, Layout und Neuberechnung getrennt protokolliert; beide Richtungen testen |
| AT-08 | jfWord -> Word -> jfWord mit CSL-Profil, Word-Feldern und fremdem Literaturfeld | Verlustberichte korrekt; gecachte Darstellung und Neuberechnung nicht verwechselt |
| AT-09 | DOCM/DOTM und binäres DOC mit VBA, Makrosignatur und externen Beziehungen | Keine Ausführung; Originalstreams im passenden Format erhalten; Signaturzustand korrekt |
| AT-10 | Export als DOCX ohne Makros, TXT, ODT und PDF | Unvermeidbare Verluste vorab angezeigt; Originaldatei nicht überschrieben; korrekte Zielstruktur |
| AT-11 | Beschädigtes ZIP/XML/CFB, extreme Rekursion, Entpackbombe, falsches Passwort, zu große Bilder | Begrenzte Ressourcen; präzise Fehler; keine externen Zugriffe oder beschädigte Arbeitsdatei |
| AT-12 | Strom-/Prozessabbruch beim Schreiben und Wiederherstellung ungespeicherter Arbeit | Letzter gültiger Stand verfügbar; Original intakt; Kandidaten klar beschriftet |
| AT-13 | Lange Tabelle mit zusammengeführten Zellen, wiederholtem Kopf, Formeln und langen Fußnoten | Keine Überlappungen oder Endlosschleifen; korrekter Umbruch und editierbarer Roundtrip |
| AT-14 | Mehrsprachiger Text mit Arabisch, Hebräisch, CJK, Akzenten, Emoji und Mathematik | Grapheme, Bidi, Cursor, Auswahl, Fonts und Druck korrekt |
| AT-15 | Revisionen und Kommentare in Haupttext, Tabelle, Note und Textfeld annehmen/ablehnen; Dokumentvergleich | Quellen-/Bereichsanker bleiben gültig; Originalfassungen unberührt |
| AT-16 | Geschütztes Formular, Datenbindung, Serienbrief mit 1.000 Datensätzen, Bedingungen und leeren Feldern | Richtige Ausgabeanzahl, keine beschädigten Formulare; Versand nur explizit |
| AT-17 | PDF-Ausgabe und Druck bei identischen Einstellungen; Tags, Links, Fonts und mathematischer Text | Gleiche Seitengeometrie; suchbarer Text; profilabhängige externe Validierung |
| AT-18 | Tastatur-/Screenreader-Nutzung, Hell/Dunkel, Kontrast und 100/150/200 % auf Windows/Wayland/X11 | Kernabläufe vollständig erreichbar, keine abgeschnittenen Dialoge oder falschen Hit-Tests |
| AT-19 | Add-on mit falscher API-Version, ungültiger Transaktion, Timeout, Crash und verbotenen Zugriffen | Keine Teiländerung; wirkliche Rechtebegrenzung nachgewiesen; Kern weiter bedienbar |
| AT-20 | Zwei Bearbeiter, Offlinephase, konkurrierende Änderungen, Widerruf und erneute Verbindung | Definierte Konvergenz/Konfliktbehandlung ohne stillen Datenverlust |
| AT-21 | Abgebrochene KI/OCR/Recherche, fehlender Anbieter, manuell korrigierte Ausgabe | Lokale Bearbeitung weiter möglich; keine erfundenen Quellen als verifiziert übernehmen |
| AT-22 | Alle vorhandenen Apps mit ergänzten Lib-/Style-Verträgen; jfWord Headless-Dump/Script | Bestehende Parität unverändert; neue Funktionen mit eigenen Tests; kein erzwungenes Legacy-Gegenstück |

Performance-Nachweis: für AT-01/02 Zeit bis erste bearbeitbare Seite, vollständige Paginierung,
Suchlatenz, Zitierstilwechsel, Speicherbedarf und Eingabelatenz unter festgehaltener Hardware
messen. Erste Entwicklungsziele: p95 Eingaberückmeldung unter 50 ms bei unverändertem
sichtbaren Umfeld, Fortschrittsanzeige spätestens nach 500 ms für längere Jobs und reagierender
Abbruch innerhalb einer Sekunde an definierten Abbruchpunkten. Diese Ziele sind keine
gemessenen Bestandswerte; ein späterer Releasebericht enthält tatsächliche Messungen.

### 12.3 Abschlusskriterien für die vorliegende Phase 1

- Alle Referenzbücher sind im Quellenregister und im Abschnittsabgleich vertreten.
- Jede nummerierte Anforderung besitzt Funktion/Aufbau, Bedienort, Architektur,
  Austauschziel, Abnahme und Herkunft; wissenschaftliche Funktionen sind eigenständiger Kern.
- Nachweisbarer Bestand, Zielzustand und erst später zu prüfende Details sind getrennt.
- Formatmatrix, VBA-Ausnahme, eigene Erweiterungen und rechtliche Änderungen widersprechen
  sich nicht. Nicht geprüfte Schutzrechte werden nicht als freigegeben dargestellt.
- IDs, Quellenkennungen, Querverweise, Überschriftenanker und lokale Verweisziele sind geprüft.
- Nur diese Spezifikation wird als Projektergebnis angelegt; keine Bücher, App-Gerüste oder
  generierten API-Dateien werden dafür eingecheckt. Ein Build ist für diese Dokumentänderung
  kein sinnvoller Funktionsnachweis und wird nicht benötigt.

<a id="buchabgleich"></a>
## 13. Abgleich mit den Buchabschnitten

Die Zuordnung unten bezieht sich auf Funktionsanforderungen, nicht auf übernommene Texte.
Nummerierte Unterabschnitte sind in ihrer übergeordneten Zeile explizit mitgeführt. Beispiele,
Problemlösungen und Übungen zu denselben Funktionen sind dadurch abgedeckt. Ein Kapitel
über ein Fremdprodukt begründet Datenaustausch, nicht dessen eigene Laufzeitkompatibilität.


### 13.1 Tuhls (T)

Das EPUB-Inhaltsverzeichnis enthält 514 Einträge. Die folgenden 442 nummerierten Fachabschnitte sind vollständig zugeordnet; Kapitel-/Teilüberschriften sind Sammelüberschriften dieser Abschnitte. Vorspann und Anhänge stehen anschließend separat.

| Abschnitt einschließlich ausdrücklich erfasster Unterabschnitte | Feature-Zuordnung |
|---|---|
| T 1.1, 1.1.1, 1.1.2 | PAGE-003, PAGE-015 |
| T 1.2, 1.2.1, 1.2.2, 1.2.3, 1.2.4, 1.2.5 | PAGE-001, PAGE-002, PAGE-004, PAGE-014 |
| T 1.3 | PAGE-016 |
| T 1.4, 1.4.1, 1.4.2, 1.4.3, 1.4.4 | PAGE-005, PAGE-014 |
| T 1.5, 1.5.1, 1.5.2, 1.5.3, 1.5.4 | PAGE-017, PAGE-018 |
| T 2.1, 2.1.1, 2.1.2, 2.1.3 | FMT-001, FILE-017, PAGE-020 |
| T 2.2, 2.2.1, 2.2.2, 2.2.3, 2.2.4 | FMT-010, FMT-011 |
| T 2.3, 2.3.1, 2.3.2, 2.3.3, 2.3.4 | FMT-015, LANG-006 |
| T 2.4, 2.4.1, 2.4.2, 2.4.3 | FMT-017, FMT-018 |
| T 2.5 | FMT-019 |
| T 2.6, 2.6.1, 2.6.2, 2.6.3, 2.6.4, 2.6.5 | PAGE-010, FLD-004 |
| T 3.1, 3.1.1, 3.1.2, 3.1.3, 3.1.4 | PAGE-006, PAGE-007, VIEW-004 |
| T 3.2, 3.2.1, 3.2.2 | PAGE-006, PAGE-007 |
| T 4.1, 4.1.1, 4.1.2, 4.1.3, 4.1.4 | PAGE-008, PAGE-009 |
| T 4.2, 4.2.1, 4.2.2, 4.2.3, 4.2.4, 4.2.5, 4.2.6 | PAGE-008, PAGE-009, PAGE-017 |
| T 5.1, 5.1.1, 5.1.2, 5.1.3, 5.1.4 | REF-005, REF-006, REF-007, REF-008, REF-009 |
| T 5.2, 5.2.1, 5.2.2, 5.2.3, 5.2.4, 5.2.5, 5.2.6, 5.2.7, 5.2.8 | REF-001, REF-002, REF-003, REF-004 |
| T 5.3, 5.3.1, 5.3.2, 5.3.3, 5.3.4 | REF-013, REF-014, REF-016, CITE-015 |
| T 5.4, 5.4.1, 5.4.2, 5.4.3, 5.4.4, 5.4.5, 5.4.6 | REF-013, REF-014, REF-015, FMT-015 |
| T 5.5, 5.5.1, 5.5.2 | REF-012, REF-016 |
| T 5.6, 5.6.1, 5.6.2, 5.6.3, 5.6.4, 5.6.5, 5.6.6, 5.6.7 | SRC-001, SRC-002, SRC-003, SRC-004, SRC-005, SRC-006, SRC-009, SRC-010, SRC-011, SRC-014, SRC-022, CITE-001, CITE-002, CITE-003, CITE-004, CITE-006, CITE-007, CITE-011, CITE-012, CITE-013, CITE-014, CITE-015, CITE-016, CITE-017, CITE-018 |
| T 5.7, 5.7.1, 5.7.2, 5.7.3, 5.7.4 | REF-019 |
| T 5.8, 5.8.1, 5.8.2, 5.8.3, 5.8.4, 5.8.5, 5.8.6 | REF-017, REF-018, REF-020 |
| T 5.9 | REF-021 |
| T 6.1 | FMT-010, UI-009 |
| T 6.2, 6.2.1, 6.2.2, 6.2.3 | FMT-010, FMT-012, FMT-013, FMT-015 |
| T 6.3, 6.3.1, 6.3.2, 6.3.3, 6.3.4, 6.3.5 | FMT-011, FMT-012, TAB-009 |
| T 6.4, 6.4.1, 6.4.2, 6.4.3, 6.4.4, 6.4.5, 6.4.6 | FILE-011, FILE-016, FMT-014 |
| T 6.5, 6.5.1, 6.5.2, 6.5.3 | FMT-013 |
| T 7.1 | EDIT-001, EDIT-002, LANG-006 |
| T 7.2, 7.2.1, 7.2.2 | EDIT-012, EDIT-014, EDIT-023 |
| T 7.3, 7.3.1, 7.3.2, 7.3.3, 7.3.4, 7.3.5 | EDIT-015, LANG-006 |
| T 7.4, 7.4.1, 7.4.2, 7.4.3, 7.4.4 | LANG-006, EDIT-015 |
| T 7.5, 7.5.1, 7.5.2, 7.5.3, 7.5.4, 7.5.5 | EDIT-015, MATH-006 |
| T 7.6, 7.6.1, 7.6.2 | EDIT-001, EDIT-015, MATH-006 |
| T 8.1, 8.1.1, 8.1.2, 8.1.3, 8.1.4 | EDIT-003, EDIT-019, VIEW-005 |
| T 8.2, 8.2.1, 8.2.2, 8.2.3, 8.2.4, 8.2.5, 8.2.6 | EDIT-004, EDIT-005, EDIT-006 |
| T 8.3, 8.3.1, 8.3.2 | EDIT-011, EDIT-020, FMT-015 |
| T 9.1, 9.1.1, 9.1.2, 9.1.3 | PAGE-003, PAGE-012 |
| T 9.2 | FMT-006 |
| T 9.3 | PAGE-017, TAB-008, PAGE-020 |
| T 9.4, 9.4.1, 9.4.2, 9.4.3 | FMT-005, FMT-006, FMT-008 |
| T 9.5, 9.5.1, 9.5.2, 9.5.3 | FMT-004 |
| T 9.6, 9.6.1, 9.6.2, 9.6.3 | FMT-007 |
| T 9.7, 9.7.1, 9.7.2 | PAGE-013 |
| T 10.1 | SEC-001, SEC-008 |
| T 10.2 | EDIT-011 |
| T 10.3, 10.3.1, 10.3.2, 10.3.3 | EDIT-007, EDIT-008, EDIT-009, FILE-012 |
| T 10.4, 10.4.1, 10.4.2 | FILE-012, OUT-011 |
| T 10.5, 10.5.1, 10.5.2, 10.5.3 | LANG-012 (Herstellerprogramme sind Referenzabläufe, keine nachzubauenden Fremdanwendungen) |
| T 10.6, 10.6.1, 10.6.2, 10.6.3, 10.6.4 | EDIT-022, EDIT-008, FMT-019, FILE-017 |
| T 10.7 | SVC-005, SVC-006 |
| T 11.1, 11.1.1, 11.1.2, 11.1.3, 11.1.4, 11.1.5 | LANG-001, LANG-002, LANG-003 |
| T 11.2, 11.2.1, 11.2.2 | LANG-004, LANG-010 |
| T 11.3, 11.3.1, 11.3.2, 11.3.3, 11.3.4 | LANG-005, LANG-006 |
| T 11.4, 11.4.1, 11.4.2, 11.4.3, 11.4.4 | FMT-022 |
| T 12.1 | SVC-008 |
| T 12.2, 12.2.1, 12.2.2, 12.2.3 | FMT-001, FMT-003 |
| T 12.3, 12.3.1, 12.3.2, 12.3.3, 12.3.4 | FMT-002 |
| T 12.4, 12.4.1, 12.4.2, 12.4.3 | FMT-001, FMT-008, OUT-002 |
| T 12.5, 12.5.1, 12.5.2, 12.5.3, 12.5.4 | FMT-008 |
| T 13.1, 13.1.1, 13.1.2, 13.1.3, 13.1.4 | EDIT-016, EDIT-017 |
| T 13.2, 13.2.1, 13.2.2 | EDIT-016, VIEW-005 |
| T 13.3, 13.3.1, 13.3.2, 13.3.3 | EDIT-017 |
| T 13.4, 13.4.1, 13.4.2 | EDIT-018 |
| T 13.5, 13.5.1, 13.5.2, 13.5.3 | EDIT-020, TAB-013 |
| T 14.1 | FMT-016 |
| T 14.2 | FMT-016, FMT-017, LANG-006 |
| T 14.3, 14.3.1, 14.3.2, 14.3.3, 14.3.4, 14.3.5, 14.3.6 | FMT-016, FMT-017 |
| T 14.4, 14.4.1, 14.4.2, 14.4.3 | FMT-018, FMT-019 |
| T 14.5 | FMT-018 |
| T 14.6, 14.6.1, 14.6.2 | FMT-019, EDIT-008 |
| T 14.7 | PAGE-011 |
| T 15.1, 15.1.1, 15.1.2, 15.1.3 | TAB-001, TAB-002 |
| T 15.2, 15.2.1, 15.2.2, 15.2.3, 15.2.4, 15.2.5, 15.2.6, 15.2.7, 15.2.8 | TAB-003, TAB-004, TAB-005, TAB-006, TAB-007 |
| T 15.3, 15.3.1, 15.3.2, 15.3.3, 15.3.4, 15.3.5 | TAB-008, TAB-009, TAB-010, TAB-011 |
| T 15.4, 15.4.1, 15.4.2 | TAB-012, PAGE-001 |
| T 15.5, 15.5.1, 15.5.2, 15.5.3, 15.5.4 | TAB-015, OBJ-016, EDIT-008 |
| T 15.6 | TAB-013 |
| T 16.1, 16.1.1, 16.1.2, 16.1.3, 16.1.4 | OBJ-001, OBJ-002, OBJ-003, OBJ-004 |
| T 16.2 | OBJ-015 |
| T 16.3, 16.3.1, 16.3.2, 16.3.3 | OBJ-005, OBJ-006, OBJ-007 |
| T 16.4 | OBJ-011 |
| T 16.5, 16.5.1, 16.5.2 | OBJ-012, OBJ-013 |
| T 16.6 | OBJ-016, SEC-008 |
| T 16.7, 16.7.1, 16.7.2, 16.7.3, 16.7.4, 16.7.5, 16.7.6 | OBJ-008, OBJ-009, OBJ-010, PAGE-017 |
| T 17.1, 17.1.1, 17.1.2 | MATH-001, MATH-007 |
| T 17.2, 17.2.1, 17.2.2, 17.2.3 | MATH-001, MATH-002, MATH-003, MATH-006 |
| T 17.3, 17.3.1, 17.3.2, 17.3.3 | MATH-004, MATH-005, MATH-006 |
| T 17.4 | MATH-012 |
| T 17.5 | MATH-008 |
| T 17.6 | MATH-010 |
| T 18.1, 18.1.1, 18.1.2, 18.1.3 | REF-010, REF-011, REF-012 |
| T 18.2, 18.2.1, 18.2.2 | REF-010, MATH-009 |
| T 18.3 | REF-011 |
| T 19.1, 19.1.1, 19.1.2, 19.1.3, 19.1.4 | FLD-001, FLD-002, FLD-010 |
| T 19.2 | FLD-003, FLD-004, FLD-005, FLD-006, FLD-007, FLD-008, FLD-009, FLD-011, MATH-013 |
| T 19.3, 19.3.1, 19.3.2, 19.3.3, 19.3.4 | REF-005, REF-006 |
| T 19.4 | FLD-012, EXT-001, EXT-004, SEC-007; VBA-Ausführung und -Editor ausdrücklich ausgeschlossen |
| T 20.1 | LANG-009 |
| T 20.2, 20.2.1, 20.2.2, 20.2.3 | FLD-006, REF-010 |
| T 20.3, 20.3.1, 20.3.2 | TAB-014, FLD-008 |
| T 21.1 | UI-007 |
| T 21.2, 21.2.1, 21.2.2 | SRC-019, SRC-020, SVC-007 |
| T 21.3 | LANG-007 |
| T 21.4, 21.4.1, 21.4.2 | LANG-008 |
| T 22.1, 22.1.1, 22.1.2, 22.1.3, 22.1.4 | FILE-003, FILE-006, FILE-008, SVC-001, SVC-002 |
| T 22.2, 22.2.1, 22.2.2, 22.2.3 | FILE-002, FILE-004, FILE-009, FILE-013 |
| T 22.3, 22.3.1, 22.3.2 | FILE-014, FILE-015 |
| T 23.1 | FILE-017, OUT-006 |
| T 23.2, 23.2.1, 23.2.2, 23.2.3, 23.2.4 | REV-001, REV-002, REV-003, REV-005 |
| T 23.3, 23.3.1, 23.3.2 | REV-003, REV-004 |
| T 23.4 | REV-007, REV-008, UI-014 |
| T 23.5, 23.5.1, 23.5.2, 23.5.3 | REV-006 |
| T 23.6, 23.6.1, 23.6.2, 23.6.3, 23.6.4 | REV-009, REV-010, SEC-003 |
| T 23.7 | REV-011, OUT-004 |
| T 23.8, 23.8.1, 23.8.2 | SVC-002, SVC-003, SVC-004 |
| T 23.9 | REV-012, OUT-012 |
| T 24.1 | VIEW-008, VIEW-009, VIEW-012, OBJ-020, OUT-007 |
| T 24.2, 24.2.1, 24.2.2, 24.2.3 | FILE-003, FILE-010, FILE-017, SEC-002, REV-011 |
| T 24.3, 24.3.1, 24.3.2 | OUT-005, OUT-007, OUT-008, OUT-009, OUT-010 |
| T 24.4, 24.4.1, 24.4.2, 24.4.3, 24.4.4, 24.4.5, 24.4.6 | OUT-001, OUT-002, OUT-003, OUT-004, OUT-012 |

| Weitere Fundstelle | Behandlung |
|---|---|
| Titel, Vorwort, Buch-/Versionskonventionen | Referenzrahmen, keine zusätzliche Softwarefunktion |
| Einleitung: Bedienkonventionen einschließlich Unterabschnitten | UI-003, UI-004, UI-006, UI-008, FLD-001 |
| Einleitung: Copilot-Ergänzung | SVC-005, SVC-006 |
| Anhang 25 / Forenhinweise | Nutzungshinweise; eigene Hilfe UI-013, keine Forensoftware nachzubauen |
| Anhang A.1, A.2, A.2.1, A.2.2, A.3 | Online-/Mobilvarianten sind Vergleichs- und Nutzungskontext. Eigene Web-/Mobil-App außerhalb des bestätigten Desktop-Ziels; Zusammenarbeit SVC-001 bis SVC-004 bleibt enthalten |
| Anhang B.1, B.1.1, B.1.2 | UI-008, UI-018 |
| Anhang B.2, B.2.1, B.2.2 | UI-005, UI-018 |
| Anhang B.3, B.3.1, B.3.2 | UI-017, UI-018 |
| Anhang C.1 | EDIT-001, EDIT-014, EDIT-023, LANG-005 |
| Anhang C.2 | FMT-017, FMT-018, FMT-019, PAGE-008 |
| Anhang C.3 | PAGE-003, PAGE-004, PAGE-013, OBJ-009, OBJ-010 |
| Anhang C.4 | VIEW-001, VIEW-003, VIEW-004, FILE-017 |
| Anhang C.5, C.6 | SYS-007, EXT-002, EXT-007 |
| Anhang C.7 | FILE-006, FILE-007, SEC-009 |
| Anhang C.8 | UI-013, UI-015 |

### 13.2 Sherer (S)

Alle 64 Navigationspunkte des EPUB sind nachfolgend identifiziert. Die Anker ermöglichen die Zuordnung auch dort, wo die EPUB-Datei mehrere Kapitel zusammenfasst. Kapitelüberschriften fassen die folgenden Abschnitte zusammen.

| EPUB-Fundstelle | Feature-Zuordnung / Behandlung |
|---|---|
| S `part0003.xhtml` | Vor-/Nachwort oder Register; kein zusätzliches Feature |
| S `part0004.xhtml` | Kapitelüberschrift; sämtliche nachfolgenden Abschnittsanker zugeordnet |
| S `part0004.xhtml#a1GE` | UI-008 |
| S `part0004.xhtml#a1GF` | UI-002, UI-003, UI-010 |
| S `part0004.xhtml#a1GG` | FILE-001 |
| S `part0004.xhtml#a1GH` | VIEW-001, VIEW-003 |
| S `part0004.xhtml#a1GJ` | EDIT-004, EDIT-005 |
| S `part0004.xhtml#a1GK` | EDIT-003, EDIT-019 |
| S `part0004.xhtml#a1GM` | FILE-012 |
| S `part0004.xhtml#a1GN` | VIEW-007 |
| S `part0004.xhtml#a1GP` | FORM-001, FORM-006, FORM-008 |
| S `part0005.xhtml` | Kapitelüberschrift; sämtliche nachfolgenden Abschnittsanker zugeordnet |
| S `part0005.xhtml#a1GS` | PAGE-001, PAGE-005 |
| S `part0005.xhtml#a1GT` | FMT-004, FMT-005, FMT-006 |
| S `part0005.xhtml#a1GU` | PAGE-003, PAGE-004 |
| S `part0005.xhtml#a1GV` | PAGE-003 |
| S `part0005.xhtml#a1GW` | PAGE-003 |
| S `part0005.xhtml#a1GX` | PAGE-001, PAGE-002 |
| S `part0005.xhtml#a1GY` | FMT-005 |
| S `part0005.xhtml#a1GZ` | PAGE-008, PAGE-009 |
| S `part0005.xhtml#a1H0` | PAGE-006, PAGE-007 |
| S `part0005.xhtml#a1H1` | FMT-006 |
| S `part0005.xhtml#a1H2` | FMT-006 |
| S `part0005.xhtml#a1H3` | FMT-016, FMT-017, FMT-018 |
| S `part0006.xhtml` | Kapitelüberschrift; sämtliche nachfolgenden Abschnittsanker zugeordnet |
| S `part0006.xhtml#a1H5` | FMT-010 |
| S `part0006.xhtml#a1H6` | FMT-010, FMT-011 |
| S `part0006.xhtml#a1H7` | FMT-011 |
| S `part0006.xhtml#a1H8` | FMT-011, FMT-012 |
| S `part0006.xhtml#a1H9` | FILE-011, FILE-016 |
| S `part0006.xhtml#a1HA` | FMT-014 |
| S `part0007.xhtml` | Kapitelüberschrift; sämtliche nachfolgenden Abschnittsanker zugeordnet |
| S `part0007.xhtml#a1HC` | TAB-001 |
| S `part0007.xhtml#a1HD` | TAB-001 |
| S `part0007.xhtml#a1HE` | TAB-003 |
| S `part0007.xhtml#a1HF` | TAB-003 |
| S `part0007.xhtml#a1HG` | TAB-007, TAB-011 |
| S `part0007.xhtml#a1HH` | TAB-008 |
| S `part0007.xhtml#a1HJ` | TAB-005 |
| S `part0007.xhtml#a1HK` | TAB-009, TAB-010 |
| S `part0007.xhtml#a1HM` | LANG-002, REF-017, REF-018 |
| S `part0007.xhtml#a1HN` | LANG-002, REF-017, REF-018 |
| S `part0007.xhtml#a1HP` | LANG-002, LANG-003 |
| S `part0007.xhtml#a1HR` | LANG-004 |
| S `part0007.xhtml#a1HS` | EDIT-016, EDIT-017 |
| S `part0007.xhtml#a1HT` | EDIT-017 |
| S `part0007.xhtml#a1HU` | LANG-007 |
| S `part0007.xhtml#a1HV` | LANG-001, LANG-002 |
| S `part0007.xhtml#a1HW` | LANG-008 |
| S `part0008.xhtml` | Kapitelüberschrift; sämtliche nachfolgenden Abschnittsanker zugeordnet |
| S `part0008.xhtml#a1HY` | PAGE-017, FMT-021 |
| S `part0008.xhtml#a1HZ` | FMT-020 |
| S `part0008.xhtml#a1J0` | PAGE-001, PAGE-016 |
| S `part0008.xhtml#a1J1` | OBJ-001, OBJ-006, OBJ-011, OBJ-012 |
| S `part0008.xhtml#a1J2` | OBJ-007 |
| S `part0008.xhtml#a1J3` | OBJ-009, OBJ-010 |
| S `part0008.xhtml#a1J4` | PAGE-017, PAGE-018 |
| S `part0008.xhtml#a1J5` | FMT-021 |
| S `part0008.xhtml#a1J6` | PAGE-016 |
| S `part0008.xhtml#a1J7` | PAGE-001 |
| S `part0008.xhtml#a1J8` | PAGE-001, OUT-002 |
| S `part0008.xhtml#a1J9` | OBJ-018 |
| S `part0009.xhtml` | Vor-/Nachwort oder Register; kein zusätzliches Feature |
| S `part0010.xhtml` | Vor-/Nachwort oder Register; kein zusätzliches Feature |

### 13.3 Baumeister / Immler (B)

Zuordnung aller nummerierten Abschnitte aus der Inhaltsübersicht auf den gedruckten Seiten 4-12. Übungen sind Wiederholungs-/Abnahmekontext, keine zusätzlichen Produktfunktionen.

| Buchabschnitt | Feature-Zuordnung / Behandlung |
|---|---|
| B 1.1 | UI-001, SYS-004 |
| B 1.2 | FILE-001, FILE-003, OUT-001 |
| B 1.3 | UI-002, UI-003, UI-009, UI-010 |
| B 1.4 | UI-003, UI-005, UI-006, UI-008, UI-013, EDIT-006 |
| B 1.5 | VIEW-001, VIEW-003, VIEW-004 |
| B 2.1 | FILE-001 |
| B 2.2 | EDIT-001, EDIT-014 |
| B 2.3 | EDIT-003, EDIT-019 |
| B 2.4 | EDIT-012, EDIT-023 |
| B 2.5 | EDIT-004, EDIT-005 |
| B 2.6 | LANG-001, LANG-002, LANG-004 |
| B 2.7 | LANG-005, LANG-006 |
| B 2.8 | EDIT-021, PAGE-003, PAGE-012 |
| B 2.9 | EDIT-007, EDIT-008, EDIT-010, EDIT-011 |
| B 2.10 | EDIT-016, EDIT-017, EDIT-018 |
| B 2.11 | UI-012, EDIT-023, FILE-010, LANG-009 |
| B 2.12 | Übung; Anforderungen aus B 2.1-2.11 |
| B 3.1 | UI-001, FILE-002, FILE-003 |
| B 3.2 | FILE-001 |
| B 3.3 | FILE-003, FILE-006, SVC-001 |
| B 3.4 | FILE-002, FILE-004, FILE-009, SEC-001 |
| B 3.5 | FILE-006, FILE-007 |
| B 3.6 | SVC-001, UI-016; Microsoft-Kontoverwaltung wird nicht nachgebaut |
| B 3.7 | UI-014, VIEW-006 |
| B 3.8 | OUT-005, OUT-009; Formatmatrix Abschnitt 8 |
| B 3.9 | UI-012, SEC-001, FILE-006 |
| B 3.10 | SEC-002 |
| B 3.11 | Übung; Anforderungen aus B 3.1-3.10 |
| B 4.1 | FMT-001, FMT-004, FMT-010 |
| B 4.2 | FMT-020, FMT-006 |
| B 4.3 | FMT-001, FMT-002, FMT-003, EDIT-015, OBJ-014 |
| B 4.4 | FMT-004, FMT-005, FMT-006 |
| B 4.5 | FMT-016, FMT-017, FMT-018 |
| B 4.6 | FMT-008 |
| B 4.7 | FMT-009, EDIT-013 |
| B 4.8 | Übung; Anforderungen aus B 4.1-4.7 |
| B 5.1 | PAGE-001, PAGE-002, OUT-002, OUT-003 |
| B 5.2 | PAGE-003, PAGE-004 |
| B 5.3 | PAGE-015 |
| B 5.4 | PAGE-005 |
| B 5.5 | PAGE-006, PAGE-007, PAGE-008, PAGE-009 |
| B 5.6 | PAGE-016 |
| B 5.7 | OUT-001, OUT-002 |
| B 5.8 | MAIL-009 |
| B 5.9 | Übung; Anforderungen aus B 5.1-5.8 |
| B 6.1 | FMT-010 |
| B 6.2 | FMT-010, UI-009 |
| B 6.3 | FMT-011, FMT-020 |
| B 6.4 | FMT-011, FMT-012 |
| B 6.5 | FMT-015, FMT-018, REF-013, VIEW-005 |
| B 6.6 | Übung; Anforderungen aus B 6.1-6.5 |
| B 7.1 | FMT-007 |
| B 7.2 | TAB-001, TAB-003 |
| B 7.3 | TAB-004, TAB-005, TAB-007, TAB-011, TAB-012, TAB-013 |
| B 7.4 | TAB-001, TAB-008, TAB-009, TAB-010 |
| B 7.5 | TAB-002 |
| B 7.6 | Übung; Anforderungen aus B 7.1-7.5 |
| B 8.1 | OBJ-001, SVC-008 |
| B 8.2 | OBJ-002, OBJ-003, OBJ-004, OBJ-020 |
| B 8.3 | OBJ-009, OBJ-010 |
| B 8.4 | OBJ-006, OBJ-007, OBJ-008 |
| B 8.5 | OBJ-005, OBJ-011, OBJ-014, OBJ-017, OBJ-019, PAGE-017 |
| B 8.6 | Übung; Anforderungen aus B 8.1-8.5 |
| B 9.1 | FLD-005, MATH-001, EDIT-015, REF-001, FILE-012 |
| B 9.2 | FMT-022 |
| B 9.3 | LANG-008 |
| B 9.4 | VIEW-002, VIEW-007 |
| B 9.5 | REV-001, REV-006, SVC-003, SVC-004 |
| B 9.6 | Übung; Anforderungen aus B 9.1-9.5 |
| B 10.1 | FILE-001, FILE-011 |
| B 10.2 | FMT-020, PAGE-006, FILE-011 |
| B 10.3 | FORM-001, FORM-002, FORM-005, FORM-006 |
| B 10.4 | FORM-007 |
| B 11.1 | MAIL-002, MAIL-003 |
| B 11.2 | MAIL-001, MAIL-004, MAIL-006, MAIL-007, MAIL-010 |
| B 11.3 | MAIL-005 |
| B 11.4 | MAIL-003 |
| B 11.5 | MAIL-009 |
| B 11.6 | Übung; Anforderungen aus B 11.1-11.5 |
| B 12.1 | SVC-005 |
| B 12.2 | SVC-005 |
| B 12.3 | SVC-005 |
| B 12.4 | SVC-005 |
| B 12.5 | SVC-005 |
| B 12.6 | SVC-005, SVC-007 |
| B 12.7 | SVC-006 |
| Tastenkombinationen, Glossar, Stichwortregister | UI-008, UI-013; Buchregister nicht als Produktinhalt übernehmen |

### 13.4 Popmintchev (P)

Fundstellen folgen der Inhaltsübersicht auf PDF-Seiten 8-11. Seitenbereiche in dieser
Tabelle sind gedruckte Seiten. Kapitel 8 verwendet einzelne Unterabschnittsnummern mehrfach;
deshalb werden dort zusätzlich eindeutige Seitenbereiche verwendet.

| Buchabschnitt einschließlich Unterabschnitte | Feature-Zuordnung / Behandlung |
|---|---|
| P 1.1 | UI-001, SYS-004 |
| P 1.2 | UI-002, UI-003, EDIT-001, FMT-001, FMT-004, VIEW-001 |
| P 1.3, 1.3.1 | UI-008, UI-018 |
| P 1.4 | FILE-003, FILE-005 |
| P 1.5 | Anwendungshinweise; dieselben Grundfunktionen, keine zusätzliche Engine |
| P 2.1 | PAGE-001, PAGE-002 |
| P 2.2 | PAGE-003, PAGE-004 |
| P 2.3 | PAGE-011, PAGE-012, FMT-003 |
| P 2.4 | PAGE-005 |
| P 3.1 | FMT-010, FMT-011, FMT-012 |
| P 3.2 | FMT-018 |
| P 3.3 | REF-013, REF-014 |
| P 3.4, 3.5 | REF-015 |
| P 3.6 | FMT-015, REF-013 |
| P 4.1, 4.2 | PAGE-008, PAGE-009 |
| P 5.1 | TAB-001, TAB-003, TAB-004, TAB-005, TAB-007, TAB-008 |
| P 5.2 | REF-010, TAB-016 |
| P 5.3 | TAB-002 |
| P 5.4 | REF-016 |
| P 5.5 | TAB-012 |
| P 6.1 | OBJ-001, PAGE-017, OBJ-009, OBJ-010 |
| P 6.2 | OBJ-001, TAB-001, REF-011 |
| P 6.3 | REF-010, REF-016 |
| P 7.1, 7.2 | MATH-001, MATH-002, MATH-003 |
| P 7.3 | MATH-006 |
| P 7.4 | MATH-009, REF-010 |
| P 7.5 | REF-006, REF-007 |
| P 7.6, 7.7 | MATH-009, REF-016 |
| P 8.1, S. 95-124: Symbol-/Alphabetgruppen einschließlich sämtlicher nummerierter Untergruppen | EDIT-001, EDIT-015, PAGE-020, MATH-002, MATH-003, MATH-004, MATH-006; Unicode-Daten statt kopierter Buchtabellen |
| P 8.1, S. 125-141: Beispiele | MATH-004; Testfälle eigenständig erstellen |
| P 8.2, S. 142-157: sämtliche Operator-/Symbol-/Alphabetgruppen | MATH-002, MATH-003, MATH-005, MATH-006 |
| P 8.2, S. 158-161: Beispiele | MATH-005; unterstützte Syntax explizit dokumentieren |
| P 8.3, 8.4 | MATH-013, EXT-009; MathType-/GrindEQ-Laufzeit nicht nachbauen, Daten/Objekte kontrolliert erhalten |
| P 9.1, 9.2 | SRC-021, CITE-021, EXT-005; EndNote/Mendeley als Austausch-/Anbieterkontext |
| P 9.3 | SRC-001, SRC-002, SRC-014, CITE-001, CITE-007, CITE-015 |
| P 10.1 | FLD-012; eigene Befehlsfolgen ersetzen gewünschte Automatisierungsabläufe |
| P 10.2 | EDIT-016, EDIT-017, EDIT-018 |
| P 10.3 | FLD-001, FLD-002, FLD-003, FLD-004, FLD-005, FLD-006, FLD-007, FLD-008, FLD-009, FLD-010 |
| P 10.4 | SEC-007, EXT-001, EXT-004; VBA-Ausführung und -Editor ausdrücklich ausgeschlossen |
| P 11.1 | REV-001, REV-002, REV-003, REV-004, REV-005 |
| P 11.2 | REV-007, REV-008 |
| P 12 | REF-017, REF-018 |
| P 13 | FORM-001, FORM-005, FORM-006, MAIL-001, MAIL-002, MAIL-004, MAIL-005, MAIL-006, MAIL-007 |
| P 14 | SVC-005, SVC-006; eigene Assistenzfunktionen ohne Microsoft-Dienstkopie |
| P 15 | Schlusswort, kein zusätzlicher Funktionsbereich |
| P 16 | UI-008, UI-013; betriebssystemspezifische Kürzel nach Zielplattform |

### 13.5 Edward (E)

Die Inhaltsübersicht auf PDF-Seite 3 ist nach Registerkarten gegliedert. Alle dort genannten
Bereiche sind nachfolgend zugeordnet. Einzelne Beschreibungen vermischen Bedienorte;
die verbindliche jfWord-Zuordnung steht deshalb in Abschnitt 4, nicht in solchen Einzelangaben.

| PDF-Seiten / Themenblock | Feature-Zuordnung / Behandlung |
|---|---|
| 1-3: Titel, Rechte, Inhaltsübersicht | Quellenidentifikation, kein Produktinhalt |
| 4-13: Einstieg und Fenster-/Bedienelemente | UI-001, UI-002, UI-003, UI-005, UI-010, VIEW-001, VIEW-003, VIEW-004 |
| 14-18: Datei, Neu, Öffnen, Speichern, Drucken | FILE-001, FILE-002, FILE-003, OUT-001, OUT-002 |
| 19-25: Zwischenablage, Schrift, Absätze | EDIT-007, EDIT-008, EDIT-010, FMT-001, FMT-002, FMT-004, FMT-005, FMT-006 |
| 26-33: Seiten, Tabellen, Bilder, Formen, Icons, Strukturdiagramme, Diagramme, Formeln | PAGE-003, PAGE-015, TAB-001, TAB-004, OBJ-001, OBJ-005, OBJ-006, OBJ-011, OBJ-012, MATH-001 |
| 34-42: Seitenlayout und ergänzende Seitengestaltung | PAGE-001, PAGE-002, PAGE-003, PAGE-004, PAGE-005, PAGE-006, PAGE-007, PAGE-008, PAGE-011, PAGE-012, PAGE-016, FMT-005, FMT-006 |
| 43-46: Sendungen | MAIL-001, MAIL-002, MAIL-004, MAIL-005, MAIL-006, MAIL-007, MAIL-008, MAIL-009 |
| 47-50: Dokumententwurf | FMT-020, PAGE-016, FMT-001, FMT-008 |
| 51-55: Überarbeiten, Kommentare, Vergleich und Korrektur | REV-001, REV-003, REV-004, REV-006, REV-007, LANG-002, LANG-004 |
| 56-61: Referenzen und Recherche | SRC-001, SRC-019, CITE-001, CITE-015, REF-001, REF-007, REF-010, REF-013, REF-016, REF-017, REF-018, SVC-007 |
| 62-66: Ansichten, Fenster und Makros | VIEW-001, VIEW-002, VIEW-003, VIEW-004, VIEW-006, FLD-012, SEC-007; eigene Aktionen statt VBA |
| 67-68: Hilfe | UI-007, UI-013, UI-015; keine Übernahme von Microsoft-Hilfetexten |

### 13.6 Rechtsmemo (L) und eigenständige Ergänzungen

| Fundstelle | Behandlung |
|---|---|
| L Seite 1: Zusammenfassung und Programmschutz | Abschnitt 10, W-COPY und W-SAS; keine pauschale Freigabe |
| L Seite 2: Design, Patente, Kennzeichen | Abschnitt 10, W-GUI, W-UWG und W-OSP; konkrete Schutzrechte noch zu prüfen |
| L Seite 3: Interoperabilität und Lizenzen | Abschnitte 8/10; öffentliches DOC-Format und Rechteumfang gegenüber Pauschalaussagen korrigiert |
| L Seiten 3-5: Vergleich mit anderen Office-Produkten | Nicht als belastbarer Tatsachennachweis oder als Begründung fehlender Funktionen verwendet |
| L Seiten 5-6: Entwicklungsstrategien | Eigene Implementierung, nachvollziehbare Quellen-/Assetherkunft, Rechteprüfung; keine Pflicht zur exakt beschriebenen Teamorganisation behauptet |
| L Seite 7: Hinweise und Quellenmarker | Durch auflösbare Primärquellen ergänzt; unbelegte Strafmaß-/Einzelfallaussagen nicht übernommen |
| U: eigene GUI-Lib, grundlegende Codecs in Lib, eigener Style | Abschnitte 3/4; UI-016, SYS-004 |
| U: wissenschaftlicher Kern und offene Literaturformate | SRC-001 bis SRC-024, CITE-001 bis CITE-021, REF-001 bis REF-022, MATH-001 bis MATH-014 |
| U: eigene Add-ons, kein VBA, Makros inaktiv erhalten | Abschnitt 9; EXT-001 bis EXT-009, FLD-012, SEC-007 |
| U: vollständiger Zielumfang, Dienste später | Abschnitte 1/11; SVC-001 bis SVC-010 |
| A: Datenintegrität, Parserlimits, deterministische Verarbeitung und Prüfbarkeit | SEC-005, SEC-006, SEC-009, SYS-001 bis SYS-007, Abschnitt 12 |

### 13.7 Grenzen der Abdeckungsbehauptung

Der Katalog bildet die identifizierten Funktionsbereiche sämtlicher lokaler Referenzbücher
und die bestätigten Ergänzungen ab. Die 442 nummerierten Fachabschnitte des Tuhls-EPUB,
dessen Vorspann/Anhänge, alle 64 Sherer-Navigationspunkte, die 86 nummerierten Abschnitte
von Baumeister/Immler, Popmintchevs Kapitel/Unterbereiche und alle Registerblöcke bei Edward
haben eine explizite Zuordnung oder einen begründeten Ausschluss.

Diese Quellenabdeckung ist keine Behauptung über sämtliche undokumentierten Funktionen
aller Word-Builds. Ein im Implementierungsverlauf entdeckter relevanter Funktionsfall wird
als neue stabile ID ergänzt, vorhandene IDs werden nicht neu nummeriert. Ausschlüsse
bleiben sichtbar: VBA-/fremde Add-in-Ausführung, Microsoft-Konto-/Cloud-Nachbau als solcher
sowie eigenständige Web-/Mobilprodukte. Die entsprechenden Desktop-Arbeitsabläufe und
eigene Dienst-/Erweiterungsschnittstellen sind im Zielumfang enthalten.

