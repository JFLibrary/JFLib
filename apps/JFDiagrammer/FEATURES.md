# JFDiagrammer – Feature-Liste

Stand: 2026-11-14. Phase 1 von JFDiagrammer: vollständige Anforderungsliste; Planungsfragen F1–F14 entschieden (Abschnitt 8).

JFDiagrammer ist ein eigenständig entwickeltes Diagrammprogramm, das so nah wie möglich an
Microsoft Visio (Visio Professional für Microsoft 365, deutsche Oberfläche) heranreicht – ohne
VBA. Diese Liste beschreibt das **Gesamtziel**, nicht den Bestand: Sie sagt, was gebaut werden
muss, in welcher Reihenfolge (Priorität) und wo (Bibliothek oder App). „Visio“ wird hier nur
beschreibend als Referenzprodukt genannt.

## Inhalt

0. [Entscheidungen, Quellen und Leseschlüssel](#grundlagen)
1. [Dateiformate, Import und Export](#formate)
2. [Dokumentmodell](#modell)
3. [Oberfläche](#oberflaeche)
4. [Shape-Kategorien und Schablonen](#shapes)
5. [Vorlagen, Diagrammtypen und Assistenten](#vorlagen)
6. [Bewusst weggelassen](#weggelassen)
7. [Anhang: Ribbon-Icons für die Neuzeichnung](#icons)
8. [Entscheidungen zu den Planungsfragen F1–F14](#fragen)

<a id="grundlagen"></a>
## 0. Entscheidungen, Quellen und Leseschlüssel

### 0.1 Verbindliche Entscheidungen (Nutzer, 2026-11-14)

- **Ort:** Die App entsteht vorerst unter `apps/JFDiagrammer` (Rust, MIT OR Apache-2.0,
  öffentliches Repo) und baut nur auf der öffentlichen `jflib`-API auf; sie zieht später in ein
  eigenes Repo um (wie JFNetworker). Modell und Codecs kommen in **zwei neue Crates unter
  `lib/`**: `diagram_core` und `diagram_io` (F1).
- **Formate:** `.vsdx`, `.vssx`, `.vssm`, `.vsx` (XML-Schablone 2003–2010) und `.vss`
  (binär). Binärformate (`.vss`/`.vsd`) nur importieren. Bei `.vssm`/`.vsdm` wird der
  Makroteil beim Speichern unverändert durchgereicht und **nie** ausgeführt.
- **Optik:** NetLab-Style (Style-Crate `netlab`) mit Menüband (Ribbon) oben. Aufbau wie Visio
  bzw. jfWord, aber **nicht** der runde JFOffice-Look: Kanten eckig wie NetLab. Ribbon-Icons
  größtenteils neu im NetLab-Stil (Liste in Abschnitt 7).
- **Shapes:** Jede Shape und jede Kategorie wird neu im JFNetworker-Stil gebaut, dazu eigene
  Kategorien. Ausnahmen: **AWS**-Icons im Original (CC BY-ND 2.0, unverändert, mit
  Namensnennung); **Azure**-Icons per Download-Knopf (der Nutzer lädt das offizielle Paket
  selbst und stimmt den Bedingungen selbst zu; JFDiagrammer baut daraus eine Schablone und
  liefert nichts mit).
- **Rechtliches:** Formate lesen/schreiben ist erlaubt ([MS-VSDX] erlaubt Implementierungen).
  Visios eigene Schablonen, Designs (Themes), Hintergründe, Rahmen und Shape-Grafiken werden
  weder mitgeliefert noch nachgezeichnet. Genormte Symbole (ISO 5807, BPMN, UML, IEC usw.)
  dürfen normgerecht aussehen. libvisio (MPL-2.0) höchstens als Lesestoff, nie Code übernehmen.
- **Kein VBA.** Alles, was nur über VBA, COM-Add-ins oder Microsoft-Clouddienste geht, steht in
  Abschnitt 6.
- **Planungsfragen F1–F14 (2026-11-14):** Der Nutzer hat alle Vorschläge aus Abschnitt 8
  übernommen. Kurzfassung:
  - zwei Crates `diagram_core` und `diagram_io` (F1);
  - Normsymbole in Normform mit NetLab-Farben, gestaltete Grafiken im JFNetworker-Stil (F2);
  - AWS-Icons unverändert, je Kategorie eine Schablone (F3);
  - `.vss` mit P2 erst nach allen XML-Formaten, `.vsd` mit P3 (F4; vier `.vss` der Version 11
    liegen lokal als Testmaterial vor, Version 6 und `.vsd` fehlen noch);
  - nur DXF, kein DWG (F5);
  - CSV und `.xlsx` in P2 mit XLSX-Leser in JFLib, ODBC/SQL in P3 (F6);
  - JFScript-Schnittstelle später, nicht in Phase 2 (F7);
  - Begriffe Ebene, Beschriftung, Legende, Seite und deutsche statt englischer Visio-Begriffe
    (F8, F13);
  - gemeinsamer Netzwerk-Symbolsatz mit JFNetworker, Import von JFNetworker-Projekten P3 (F9);
  - deutsche Office-Tastenbelegung für Textformate (F10);
  - Rack-Diagramm schon in P1, AWS-Architektur P2 (F11);
  - keine weiteren Screenshots nötig (F12);
  - Normschablonen nach DIN EN/IEC statt US/ANSI (F14).
- **Sprachen (Nachtrag 2026-11-14):** Alles auf Deutsch, Englisch, Spanisch, Italienisch und
  Französisch, umschaltbar in den Einstellungen (Abschnitt 3.17). F8 und F13 gelten sinngemäß für
  alle fünf Sprachen. Für F10 ist die Belegung je Sprache abgeleitet und noch zu bestätigen (LG-08).

### 0.2 Quellen

Alle Quellen liegen nur lokal und sind gitignored (`apps/JFDiagrammer/context/`). Sie wurden
vollständig gelesen bzw. angesehen; übernommen wurde nichts, alle Beschreibungen sind eigene
Worte. Befehls- und Featurenamen stehen so da, wie die deutsche Oberfläche sie nennt.

| Kürzel | Quelle |
|---|---|
| `A hhmmss` | Screenshot der Visio-Oberfläche `context/app/Screenshot 2026-11-14 hhmmss.png`; `A 08-hhmmss` = `Screenshot 2026-11-08 hhmmss.png` |
| `S MMTT-hhmmss` | Screenshot einer Schablone `context/shapes/Screenshot 2026-MM-TT hhmmss.png`; `S Übersicht` = `Shapes-1024x265.png`, `S 2024` = `Screenshot 2024-10-04 121842.png` |
| `SbS n` | *Microsoft Visio 2016 Step by Step*, Kapitel n; `SbS A` = Anhang A (Anpassen, Schablonen, ShapeSheet, Formate), `SbS B` = Anhang B (Tastenkürzel), `SbS Gl` = Glossar, `SbS Vorwort` |
| `UG` | `UsrGuide.PDF` (Benutzerhandbuch zu Visio 5.0, 1997/98), `UG Kap. n`; die ShapeSheet-Referenz fehlt dort (sie stand in einem eigenen Entwicklerbuch), daher stützen sich 2.13/2.14 auf die Beispieldateien und MS-VSDX |
| `TP` | `microsoft_visio_tutorial.pdf` (Tutorial zu Visio 2016, 72 Kapitel) |
| `Z2`, `Z3`, `Fav` | Beispieldateien `Zeichnung2.vsdx`, `Zeichnung 3.vsdx`, `Favoriten.vssx` (entpackt und ausgewertet, Abschnitt 1.6) |
| `SWDB` | binäre Schablonen `context/files/SWDB_DE/` (Express-G, Jackson, ORM, ROOM; Version 11, deutsch, metrisch); Microsoft-Inhalt, nur lokal, nur die Namensliste ausgelesen |
| `MS-VSDX` | öffentliche Formatspezifikation [MS-VSDX] (nicht im Kontextordner; nur zur Einordnung) |
| `W` | Kenntnis des Referenzprodukts, in den lokalen Quellen nicht direkt belegt – vor der Umsetzung prüfen |

### 0.3 Leseschlüssel

Jede Feature-Zeile hat eine stabile ID und die Spalten:

- **Feature** – Name (bei Befehlen der Name aus der Oberfläche).
- **Beschreibung** – was es tut, in eigenen Worten.
- **Quelle** – Kürzel aus 0.2.
- **Prio** – **P1** Kern (erste brauchbare Version: typische Visio-Zeichnungen öffnen,
  bearbeiten, speichern, drucken), **P2** wichtig für Visio-Nähe im Alltag, **P3** selten oder
  speziell.
- **Ort** – `Lib-core` = neue Crate `lib/diagram_core` (Dokumentmodell, ShapeSheet, Formeln,
  Geometrie, Routing, Layout); `Lib-io` = neue Crate `lib/diagram_io` (VSDX/VDX/VSX/VSS-Codecs,
  Importe und Exporte), beide nach dem Muster `doc_core`/`doc_io` (F1); `Lib-xlsx` = schlanker
  XLSX-Leser in JFLib (F6; welche Crate, wird bei der Umsetzung festgelegt); `Lib-ui` = `lib/ui`;
  `Lib-plat`, `Lib-img`, `Lib-archive`, `Lib-svg`, `Lib-style` = die bestehenden Crates
  `plat`, `img_*`, `archive`, `img_svg`, `style_netlab`; `App` = `apps/JFDiagrammer`;
  `JFNetworker`/`JFScript` = die eigenen Repos.
  `A+B` heißt: A besitzt die Semantik, B hilft.
- **Bestand** – **vorhanden** (in JFLib/JFNetworker nutzbar), **teilweise** (Baustein da,
  Visio-Umfang fehlt), **neu**. Der Abgleich ist grob (Abschnitt 0.4), keine Tiefenanalyse.

Schablonen (Abschnitt 4) sind je **eine** Feature-Zeile mit Prio/Ort/Bestand; die
Shape-Tabellen darunter nennen nur Name und Beschreibung.

### 0.4 Grober Bestandsabgleich

| Baustein | Was es schon gibt | Was für JFDiagrammer fehlt |
|---|---|---|
| `lib/ui/diagram.rs` | Knoten/Kanten-Canvas: Zoom 0,1–8 mit Rad und Strg+Rad, Verschieben (Mittelmaus, Leertaste, Hand-Werkzeug), Einpassen, Übersichtskarte, Raster mit Einrasten, Rechteck-Auswahl (Umschalt/Strg), Pfeiltasten-Verschieben, Tab-Navigation, Ports an Knoten, Kanten gerade/Bezier/orthogonal mit Wegpunkten, Pfeilspitzen und Strichmuster (gfx), Rahmen mit Titelleiste und Einklappen, ein Größengriff, Bildschirmleser-Semantik, GPU-Kacheln | Drehen, acht Größengriffe, gelbe Steuerpunkte, Klebepunkte mit Richtung, dynamisches Verbinder-Routing um Shapes, Linienübergänge, verschachtelte Gruppen, Ebenen, Lineale, Führungslinien, dynamisches Raster (Ausrichtungshilfen), Seitenfläche und Seitenumbrüche, Text auf Shapes, Formelauswertung |
| `lib/ui/ribbon.rs` + jfWord | Ribbon-Gruppen mit Beschriftung und Dialogstarter, große/kleine Knöpfe, Menü- und Split-Knöpfe, Galerien und Kacheln, Einklappen schmaler Gruppen; Registerleiste und Backstage in jfWord (`ribbon_tabs.rs`, `shell_ui.rs`), Lineal (`ruler_ui.rs`) | Kontextregister, Live-Vorschau in allen Galerien, Netlab-Look (eckig), Visio-Register und -Befehle |
| `lib/ui` sonst | `dock.rs` (andockbare Bereiche), `history.rs` (Rückgängig), `color.rs` (Farbwahl), `fontpick.rs`, `filedlg.rs`, `recent.rs`, `settings_page.rs`, `grid.rs`, `dnd.rs`, `rich_edit.rs`, `chart.rs`, `timeline.rs`, `vicon.rs`, i18n, Barrierefreiheit | Aufgabenbereiche mit Visio-Inhalt, Shape-Daten-Fenster, Zeichnungsexplorer, ShapeSheet-Fenster |
| `lib/ui` i18n | `tr()` für Bibliothekstexte in neun Sprachen (darunter DE, EN, ES, IT, FR), App-Kataloge als TSV (`Catalog::from_tsv`, `locales/*.tsv`), Sprachwechsel zur Laufzeit (`set_language`, übersetzte Befehls- und Menütabellen), regionale Formate (`i18n_locale.rs`), Nachrichten mit Plural (`i18n_msg.rs`), Einstellungsseite (`settings_page.rs`) | App-Katalog mit fünf Spalten, Namen aller Schablonen/Shapes/Vorlagen in fünf Sprachen, Glossar, Prüfskript (3.17) |
| `lib/archive`, `img_core::deflate` | ZIP lesen (gespeichert + DEFLATE), ZIP schreiben (nur „gespeichert“), zlib-Kompressor | DEFLATE beim Schreiben einhängen; Grenzen (512 Teile, 8 MiB je Teil, 32 MiB gesamt, kein ZIP64) passen nicht für große Zeichnungen |
| `lib/doc_io` | OPC-Paket und XML für DOCX (Muster für VSDX: Beziehungen, Inhaltstypen, Erhalt unbekannter Teile) | VSDX-Schema |
| `lib/img_svg`, `img_*` | SVG lesen und rendern; PNG, JPEG, GIF, TIFF, BMP, WebP u. a. lesen/schreiben | SVG schreiben, SVG in Shape-Geometrie umwandeln, EMF/WMF lesen und schreiben |
| `lib/plat` Drucken | Systemdruckdialog (Windows, Linux-Portal, macOS), mehrseitige Aufträge, PDF nur als Rasterbild je Seite | Vektor-PDF mit Text, Hyperlinks und Lesezeichen; Kacheldruck über mehrere Blätter |
| JFNetworker | Geräte-Seitenleiste mit Kategorien, Suche, Kacheln, Klick-Platzieren und Drag&Drop (`src/workspace/palette.rs`), Katalog (`crates/jfn_model/catalog.rs`), ~30 Gerätesymbole als SVG im JFNetworker-Stil (`icons/`), Gruppenrahmen, Beschriftungen, Eigenschaften-Inspektor | Visio-Schablonenmodell (Master, Schnell-Shapes, Favoriten, Schablonen-Dateien), Visio-Kategorien |

### 0.5 Umfang

- **562 Feature-Zeilen** in den Abschnitten 1–3, 4.0 und 5 (davon P1 265, P2 186, P3 105; bei Zeilen mit zwei Prioritäten zählt die erste), darunter 11 Zeilen für Sprachen und Regionen (3.17).
- **282 Schablonen** in Abschnitt 4 mit **3607 Shape-Zeilen** (nach Zusammenfassen von Doppelungen) und **15 Vorschläge** für eigene Kategorien.
- **29 bewusst weggelassene** Funktionen (Abschnitt 6), **14 Entscheidungen** zu den Planungsfragen (Abschnitt 8). Noch offen: Testdateien für Version 6 und `.vsd` (F4), Bestätigung der Tastenbelegung je Sprache (LG-08), deutscher Begriff für „AutoVerbinden“ (Glossar 3.17).

<a id="formate"></a>
## 1. Dateiformate, Import und Export

### 1.1 Visio-Formate ab 2013 (OPC/XML, „VSDX-Familie“)

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| DF-01 | `.vsdx` lesen | Zeichnung als ZIP-Paket öffnen: Inhaltstypen, Beziehungen, `document.xml`, `pages.xml` + `pageN.xml`, `masters.xml` + `masterN.xml`, Themes, `windows.xml`, Dokumenteigenschaften, Medien. Alle drei Beispieldateien müssen fehlerfrei geladen werden. | Z2, Z3; MS-VSDX; SbS 1, 12 | P1 | Lib-io+Lib-archive | teilweise |
| DF-02 | `.vsdx` schreiben | Deterministisch speichern (gleiche Daten → gleiche Bytes), DEFLATE-komprimiert, nur Teile mit Inhalt; Visio muss die Datei ohne Reparaturmeldung öffnen. | MS-VSDX; SbS 1, 12 | P1 | Lib-io+Lib-archive | teilweise |
| DF-03 | Unbekanntes erhalten | Elemente, Attribute, Teile und `extLst`-Erweiterungen, die das Modell nicht versteht, beim Speichern unverändert zurückschreiben (Round-Trip). Gilt auch für Lösungs-XML (SolutionXML) von Visio-Add-ons. | Z3; MS-VSDX | P1 | Lib-io | neu |
| DF-04 | `.vsdm` (Zeichnung mit Makros) | Wie `.vsdx` öffnen; das VBA-Projekt (`vbaProject.bin` samt Beziehung) unverändert durchreichen, nie ausführen, beim Öffnen sichtbar darauf hinweisen. Speichern als `.vsdx` entfernt es nach Rückfrage. | MS-VSDX; Entscheidung | P2 | Lib-io+App | neu |
| DF-05 | `.vstx`/`.vstm` (Vorlage) | Vorlage öffnet eine neue, unbenannte Zeichnung mit deren Seiten, Schablonen-Verweisen, Formatvorlagen und Theme; „Speichern unter → Vorlage“ schreibt eine. Makroteil wie DF-04. | SbS 12 | P2 | Lib-io+App | neu |
| DF-06 | `.vssx` (Schablone) | Master einer Schablone für die Shapes-Seitenleiste laden (Name, Symbol, QuickInfo, Schnell-Shape-Markierung); eigene Schablonen (z. B. Favoriten) bearbeiten und speichern. | Fav; SbS 1, 2, 7, 12 | P1 | Lib-io+App | neu |
| DF-07 | `.vssm` (Schablone mit Makros) | Wie `.vssx`; Makroteil unverändert durchreichen, nie ausführen. | Entscheidung | P2 | Lib-io | neu |
| DF-08 | Vorschaubild | `docProps/thumbnail.emf` lesen (für Dateidialog/Zuletzt-verwendet) und beim Speichern schreiben; Optionen keine/Entwurf/detailliert und nur erste Seite oder alle Seiten (EMF, siehe EX-07). | Z2, Z3; UG Kap. 2 | P3 | Lib-io+Lib-img | neu |
| DF-09 | Dokumenteigenschaften | Kern- (Titel, Thema, Autor, Stichwörter, Beschreibung, Kategorie, Sprache, zuletzt geändert von, Erstellt/Geändert/Gedruckt), App- (Vorlage, Firma, Manager, Hyperlinkbasis, Seitentitel) und benutzerdefinierte Eigenschaften lesen, bearbeiten, schreiben. | Z2, Z3; SbS 1, 11, 12 | P2 | Lib-io+App | neu |
| DF-10 | Fenster- und Ansichtszustand | `windows.xml`: offene Schablonen mit Reihenfolge, Zeichenfenster mit Zoom, Mittelpunkt, aktiver Seite, Lineal/Raster/Führungslinien/Seitenumbrüche/Verbindungspunkte sichtbar, Einrast- und Klebe-Einstellungen. Lesen und schreiben, damit eine Datei so aufgeht, wie sie gespeichert wurde; „Arbeitsbereich speichern“ abschaltbar. | Z2, Z3; SbS 1; UG Kap. 2 | P2 | Lib-io+App | neu |
| DF-11 | Kommentare-Teil | Moderne Kommentare (`comments.xml`, Autorenliste, Antworten, Bezug auf Seite/Shape) lesen und schreiben; ältere Anmerkungen (Reviewer-/Annotation-Abschnitte) lesen. | SbS 3; MS-VSDX | P2 | Lib-io | neu |
| DF-12 | Datenverbindungen-Teile | Datenverbindungen und Datensatzgruppen (Abfrage, Spalten, Zeilen, Verknüpfung zu Shapes, Aktualisierungsintervall) lesen und schreiben; Daten ohne Netz-/Dateizugriff anzeigen. | SbS 10; MS-VSDX | P2 | Lib-io | neu |
| DF-13 | Validierungs-Teil | Regelsätze, Regeln (Filter-/Testausdruck) und gefundene Probleme lesen und schreiben (Diagrammprüfung, siehe DM-DATA). | SbS 14; MS-VSDX | P3 | Lib-io | neu |
| DF-14 | Eingebettete Medien | Bilder (`visio/media/*`: PNG, JPEG, GIF, BMP, TIFF, EMF, WMF, SVG) und eingebettete OLE-Objekte als Teile mit Beziehung lesen und schreiben; OLE nur mit Vorschaubild anzeigen (Abschnitt 6). | Z2 | P1 | Lib-io+Lib-img | teilweise |
| DF-15 | Robustheit | Jede Datei ist fremde Eingabe: Größen, Anzahl Teile/Elemente, Verschachtelung und Formellänge begrenzen; externe Verweise nie folgen; ZIP64 und große Pakete (> 32 MiB) unterstützen; beschädigte Dateien mit klarer Meldung ablehnen statt halb laden. | Lib-Regeln | P1 | Lib-io+Lib-archive | teilweise |

### 1.2 Ältere XML-Formate 2003–2010

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| DF-20 | `.vsx` lesen (XML-Schablone) | XML-Schablone im älteren „DatadiagramML“-Schema in Master für die Seitenleiste umwandeln; Zellen, Formeln und Abschnitte auf das Modell abbilden. | Entscheidung; MS-VSDX (Vorgänger) | P2 | Lib-io | neu |
| DF-21 | `.vdx` lesen (XML-Zeichnung) | Zeichnung im gleichen älteren XML-Schema öffnen (eine Datei statt Paket, Bilder base64-eingebettet). | W | P2 | Lib-io | neu |
| DF-22 | `.vtx` lesen (XML-Vorlage) | Vorlage im älteren XML-Schema als neue Zeichnung öffnen. | W | P3 | Lib-io | neu |
| DF-23 | `.vdx`/`.vsx` schreiben | Für Austausch mit sehr alten Programmen; nur wenn ein Nutzer es braucht. | W | P3 | Lib-io | neu |

### 1.3 Binärformate (nur Import)

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| DF-30 | `.vss` importieren (binäre Schablone) | Alte Schablonen bis Visio 2010 in Master umwandeln. Das Binärformat (OLE-Verbunddatei mit komprimierten Datenblöcken) ist von Microsoft nicht als offene Spezifikation veröffentlicht; Umsetzung nur aus eigener Analyse, libvisio höchstens als Lesestoff. Entscheidung F4: erst nach allen XML-Formaten. Testmaterial (Nachtrag 2026-11-14): vier binäre Schablonen der Version 11 (Visio 2003–2010) lokal in `context/files/SWDB_DE/` (`exprg_m.vss`, `jacksn_m.vss`, `orm_m.vss`, `room_m.vss`). Sie sind Microsoft-Inhalt und kommen nie ins Repo; Tests mit ihnen laufen nur lokal und werden übersprungen, wenn die Dateien fehlen. Es fehlen noch Version 6 (Visio 2000/2002) und binäre Zeichnungen. | Entscheidung; SWDB | P2 | Lib-io | neu |
| DF-31 | `.vsd` importieren (binäre Zeichnung) | Zeichnungen bis Visio 2010 öffnen, beim Speichern zu `.vsdx` werden. Gleiche Grundlage wie DF-30; Testdateien für `.vsd` fehlen noch. | Entscheidung | P3 | Lib-io | neu |
| DF-32 | `.vst` importieren (binäre Vorlage) | Wie DF-31, öffnet eine neue Zeichnung. | W | P3 | Lib-io | neu |
| DF-33 | Binär schreiben | Nicht vorgesehen (Entscheidung „nur Import“); Speichern unter bietet `.vsd`/`.vss` nicht an. | Entscheidung | – | – | – |
| DF-34 | Lokaler Testkorpus Binärformate | Testfälle für DF-30 bis DF-32 lesen Dateien aus einem lokalen, gitignorierten Ordner (heute `context/files/SWDB_DE/`, Version 11). Fehlt der Ordner, werden die Tests übersprungen, nicht als Fehler gewertet. Ziel: je eine Datei der Versionen 6 und 11, Schablonen und Zeichnungen. | Nachtrag 2026-11-14; SWDB | P2 | Lib-io | neu |

### 1.4 Export („Speichern unter“ und „Exportieren“)

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| EX-01 | PDF | Vektor-PDF mit echtem Text (Schriften eingebettet), Optionen: alle Seiten/aktuelle Seite/Auswahl/Bereich, Hintergrundseiten ein/aus, Dokumenteigenschaften, Barrierefreiheits-Tags, Lesezeichen je Seite, Hyperlinks klickbar, PDF/A. | SbS 1, 12; TP | P1 | Lib-io+Lib-plat | teilweise |
| EX-02 | XPS | Wie PDF im XPS-Format. | SbS 1, 12 | P3 | Lib-io | neu |
| EX-03 | SVG | Seite als SVG: Geometrie als Pfade, Text als Text, Farben/Verläufe/Transparenz, optional Visio-Zusatzdaten (Shape-Daten) als Attribute. | SbS 12; W | P1 | Lib-io | neu |
| EX-04 | PNG | Rastergrafik mit Ausgabeoptionen: Auflösung (Bildschirm/Drucker/Quelle/eigene DPI), Größe, Hintergrundfarbe oder transparent, Farbtiefe, Interlace; Bereich = Seite oder Auswahl. | SbS 12; W | P1 | Lib-io+Lib-img | teilweise |
| EX-05 | JPEG | Wie PNG, dazu Qualität/Kompression, Hintergrundfarbe. | SbS 12; W | P2 | Lib-io+Lib-img | teilweise |
| EX-06 | GIF, TIFF, BMP | Weitere Rasterformate mit denselben Optionen (TIFF: Kompression). | SbS 12; W | P3 | Lib-io+Lib-img | teilweise |
| EX-07 | EMF (erweiterte Metadatei) | Vektor-Metadatei für Office-Programme; auch für Zwischenablage und Vorschaubild (DF-08). | SbS 12; W | P2 | Lib-io | neu |
| EX-08 | WMF | Ältere Metadatei. | W | P3 | Lib-io | neu |
| EX-09 | Webseite (HTML) | Zeichnung als Webseite mit allen Seiten, Seitenleiste zum Navigieren, Zoom, Shape-Daten-Anzeige beim Klick, Suche; Grafik als SVG (Fallback PNG); Shape-Links als klickbare Flächen, Seitenlinks nur, wenn beide Seiten exportiert werden; eigene HTML-Vorlage mit Platzhaltern. | SbS 12; UG Kap. 11, 25 | P2 | App+Lib-io | neu |
| EX-10 | DXF | Seite als CAD-Zeichnung im offenen DXF-Format exportieren; DWG entfällt (F5, NO-29). | SbS 12 | P3 | Lib-io | neu |
| EX-11 | Als Vorlage/Schablone speichern | Zeichnung als `.vstx`, Schablone als `.vssx` speichern. | SbS 12 | P1 | Lib-io+App | neu |
| EX-12 | Shape-Bericht exportieren | Berichtsergebnis (Abschnitt 2.10) als CSV/Excel-Tabelle (`.xlsx`), HTML, XML oder als Tabellen-Shape in die Zeichnung. | SbS 7, 8; UG | P2 | App+Lib-io | neu |
| EX-13 | Organigramm-Daten exportieren | Personen und Hierarchie einer Organigramm-Zeichnung als CSV/Excel. | SbS 5; W | P3 | App | neu |
| EX-14 | Kopieren in die Zwischenablage | Auswahl als eigenes Format (verlustfrei innerhalb JFDiagrammer und zwischen Fenstern), als EMF/SVG/PNG und als Text für andere Programme. | SbS 2; W | P1 | App+Lib-ui | teilweise |
| EX-15 | Drucken | Siehe Abschnitt 3.15 (Seite einrichten, Kacheln, Vorschau). | SbS 12 | P1 | App+Lib-plat | teilweise |
| EX-16 | Daten exportieren (Inventar) | Shape-Daten ausgewählter Shapes einer Seite oder aller Seiten (Netzwerk, Büroplan, Organigramm) als CSV/Excel-Tabelle; Spaltenwahl. | UG Kap. 20 | P2 | App | neu |
| EX-17 | Ganze Zeichnung kopieren | Ohne Auswahl die ganze Zeichnung (alle Seiten) in die Zwischenablage; andere Programme zeigen die aktuelle Seite. | UG Kap. 13 | P3 | App | neu |

### 1.5 Import

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| IM-01 | Bilder einfügen | PNG, JPEG, GIF, BMP, TIFF, WebP als Bild-Shape (zuschneiden, Helligkeit/Kontrast, Transparenz). | SbS 3, 5, 12 | P1 | App+Lib-img | teilweise |
| IM-02 | SVG einfügen | SVG als Bild-Shape; zusätzlich „In Shape konvertieren“: Pfade werden echte Geometrie mit Füllung/Linie, Gruppen bleiben Gruppen. | W | P2 | Lib-io+Lib-svg | teilweise |
| IM-03 | EMF/WMF einfügen | Metadateien als Bild anzeigen, auf Wunsch in Shapes zerlegen. | W | P3 | Lib-io | neu |
| IM-04 | CAD-Zeichnung einfügen | DXF (DWG entfällt, F5) als gesperrtes Hintergrund-Objekt mit Ebenen, Maßstab und Ausschnitt; optional in Shapes umwandeln. | W | P3 | Lib-io | neu |
| IM-05 | Daten importieren | CSV und Excel (`.xlsx`, Leser IM-10) in P2, ODBC/SQL erst in P3 (F6), als Datenquelle für Datenverknüpfung, Organigramm- und Datenvisualisierer-Assistent (Abschnitt 2.10 und 5). | SbS 5, 10 | P2 | App+Lib-io | neu |
| IM-06 | Einfügen aus der Zwischenablage | Text (wird Textfeld-Shape), Bilder, SVG, eigene Shapes; „Inhalte einfügen“ mit Formatwahl. | SbS 2; W | P1 | App+Lib-ui | teilweise |
| IM-07 | Fremde Diagrammformate (eigene Erweiterung) | draw.io (`.drawio`), GraphML (yEd), Mermaid- und PlantUML-Text, BPMN-2.0-XML (öffentlicher OMG-Standard) importieren und exportieren. Visio bietet das nicht; Vorschlag. | eigener Vorschlag | P3 | Lib-io | neu |
| IM-08 | Verbundene Zeichnung aus CSV/Text | Textdatei mit Datensatzarten „Shape“ (ID, Master, Text, X, Y, Breite, Höhe, Eigenschaften) und „Link“ (ID, Master, Text, von, nach) erzeugt Shapes und Verbinder; Komma- oder Tab-getrennt, Text in Anführungszeichen (auch mehrzeilig), Kommentarzeilen. Einfacher Weg, Diagramme aus Skripten zu erzeugen. | UG Kap. 21 | P2 | Lib-io+App | neu |
| IM-09 | Weitere Vektorformate | EPS, Adobe Illustrator, CGM als Bild einfügen und, wo möglich, in Shapes zerlegen. Alte Fremdformate (ABC FlowCharter, CorelFLOW, CorelDRAW, Micrografx, PICT, IGES, PCX, DWF) entfallen (NO-26). | UG Kap. 9 | P3 | Lib-io | neu |
| IM-10 | XLSX-Leser | Schlanker Leser für `.xlsx`-Arbeitsmappen: Blätter, benannte Bereiche, Zellwerte mit Typ, ohne Formelauswertung; als fremde Eingabe begrenzt. Grundlage für Datenverknüpfung, Organigramm-Assistent und Datenvisualisierer. | Entscheidung F6; SbS 5, 10 | P2 | Lib-xlsx | neu |
| IM-11 | JFNetworker-Projekt importieren | Geräte, Kabel, Gruppen und Beschriftungen eines JFNetworker-Projekts als Netzwerkdiagramm mit den gemeinsamen Symbolen (SH-G09) übernehmen. | Entscheidung F9 | P3 | App | neu |

### 1.6 Befund aus den Beispieldateien

Die drei Beispieldateien wurden entpackt und vollständig ausgewertet (nur Struktur, nichts
übernommen). Sie legen fest, was der Codec in P1 mindestens verstehen muss:

- **Pakete:** `Fav` ist eine Schablone (Hauptinhaltstyp „stencil.main“, 29 Master, keine
  Seiteninhalte, keine Themes). `Z2` hat 1 Seite, 15 Master, 1 Theme, 3 PNG-Bilder, EMF-Vorschau.
  `Z3` hat 5 Seiten, 30 Master, 3 Themes (je Seite eines), EMF-Vorschau, 12 offene Schablonen in
  `windows.xml`. Beziehungsarten: document, pages/page, masters/master, theme, windows, image,
  thumbnail, core/extended/custom-properties.
- **Seiten:** A4 hoch (`Fav`), eigene Größe (`Z2`) und A4 quer (`Z3`), Maßeinheit mm, Maßstab
  1:1; Seiten-ShapeSheet mit Abschnitten `User` und `Layer`. Ebenen: „Verbinder“, „Container“,
  „Allgemeine Symbole“, „Geräte“, „Elektrogerät“. Keine Hintergrundseiten, keine Hyperlinks,
  keine Kommentare in den Beispielen – diese Teile müssen trotzdem gelesen werden (MS-VSDX).
- **Shapes:** 718 einfache Shapes, 303 Gruppen, 3 Fremdobjekte (PNG). Master-Vererbung mit
  `Master`/`MasterShape`, gelöschte geerbte Zeilen (`Del="1"`), Formatvorlagen-Verweise
  (`LineStyle`/`FillStyle`/`TextStyle`).
- **ShapeSheet-Abschnitte im Einsatz:** Geometry (2115×), User (434), Character (325), Control
  (314), Connection (234), Property/Shape-Daten (60), Paragraph (29), LineGradient (13),
  Layer (12), FillGradient (11), Actions (4), Tabs (3), Scratch (3).
- **Geometriezeilen:** MoveTo, LineTo, ArcTo, EllipticalArcTo, Ellipse, NURBSTo, PolylineTo,
  RelMoveTo, RelLineTo, RelCubBezTo.
- **Einzelzellen:** rund 330 verschiedene Zellnamen (Transformation, Linie, Füllung, Schatten,
  3D-/Fase-/Glanz-/Spiegelungs-Effekte, Skizzeneffekt, Text-Block und -Transformation, Sperren,
  Gruppeneigenschaften, Klebe- und Shape-Layout, Seiten-, Druck- und Rastereinstellungen,
  Theme-Indizes, Schnellformatvorlagen, Ereignisse, Bildeigenschaften, Ersetzen-Sperren).
- **Formelfunktionen im Einsatz (60):** ABS, AND, ARG, ATAN2, BITAND, BITOR, BITXOR, BOUND,
  CEILING, COS, DEFAULTEVENT, DEPENDSON, DOCMD, FONT, GUARD, HSL, HUE, IF, INDEX, ISERRVALUE,
  ISTHEMED, LOC, LUM, LUMDIFF, MASTERNAME, MAX, MIN, MODULUS, MSOTINT, NOT, NURBS,
  OPENTEXTWIN, OR, PAR, PNT, POLYLINE, REF, RGB, RUNADDON, SAT, SETATREF, SETATREFEVAL,
  SETATREFEXPR, SETF, SHADE, SHAPETEXT, SIN, SQRT, STRSAME, SUM, TAN, TEXTHEIGHT, TEXTWIDTH,
  THEMEGUARD, THEMEPROP, THEMEVAL, TINT, WALKGLUE, XFTRIGGER. Häufigste: IF, THEMEVAL, GUARD,
  THEMEGUARD, ABS, LUM, OR.
- **Einheiten:** MM, IN, PT, DL, DA, DEG, FT, M, DT, COLOR, BOOL, STR, GUID, NURBS, POLYLINE,
  IN_F, F_I.
- **Verbindungen:** 166 `Connect`-Einträge: Verbinder-Anfang/-Ende an festen Verbindungspunkten
  (`Connections.Xn`, statisch) und an `PinX` (dynamisch, Shape-zu-Shape); Steuerpunkte
  (`Controls.Row_n`) an Verbindungspunkten (Legenden/Beschriftungen).
- **Container:** `User.msvStructureType` = Container/Heading, `msvSDContainer*`-Zellen (Rand,
  automatische Größe, Sperre, Hervorhebung, Stil 1 von 14, Überschrift unten, ausgeschlossene
  Kategorien), `msvSDHeadingStyle` (1 von 4), `msvShapeCategories`.
- **Shape-Daten:** Netzwerk-Master mit Feldern wie Inventarnummer, Gebäude, Raum, Ort,
  Hersteller, Produktnummer, Seriennummer, IP-/MAC-Adresse, Subnetzmaske, Netzwerkname,
  Anzahl Anschlüsse, Betriebssystem, CPU, Arbeitsspeicher, Festplatte, Community String,
  Administrative Schnittstelle; Kategorien `ShapeClass`/`ShapeType`/`SubShapeType` (für
  Berichte und Legenden).
- **Kontextmenü-Aktionen:** Eckentyp, Ecken abschneiden, Ecken sperren, „Eigenschaften“
  (öffnet Shape-Daten).
- **Text:** formatierte Läufe (`cp`/`pp`/`tp`), keine Felder in den Beispielen.
- **Themes:** „Einfach“, „Linear“ (Farbe „Sequenz“), „Parallel“; DrawingML-Theme plus
  Visio-Erweiterungen (Farb- und Stilvarianten, Verbinder-Schema, Linien- und Schriftstile,
  Verzierungsgrad).
- **Formatvorlagen:** No Style, Text Only, None, Normal, Guide, Theme, Connector, Flow Normal,
  Symbol, Wireframe, Home Plan, Default Svg Style.
- **Dokument-ShapeSheet:** Ausgabeformat, Vorschau-Sperre, Markup anzeigen/hinzufügen,
  Ersetzen-/Seiten-Duplizieren-Sperre, keine Mitbearbeitung, Vorschauqualität/-umfang, Sprache.
- **Auffällig:** `Z3` enthält Shapes mit `User.3D_model_id`/`3D_type` (Visio-3D-Modelle) und
  einen Master „Azure“ (Microsoft-Produkt-Schablone) – beides muss erhalten bleiben, auch
  wenn JFDiagrammer es nur als Bild/Platzhalter zeigt.

<a id="modell"></a>
## 2. Dokumentmodell

Das Modell bildet das VSDX-Schema verlustfrei ab (Abschnitt 1.6) und rechnet wie Visio: Jede
sichtbare Eigenschaft ist eine ShapeSheet-Zelle (2.13), deren Formel die Engine (2.14)
auswertet. Die Abschnitte 2.1 bis 2.12 beschreiben die fachlichen Bausteine, 2.13 und 2.14 die
Zellen und Funktionen darunter.

### 2.1 Dokument

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| DM-01 | Dokument als Ganzes | Enthält Seiten (Vorder- und Hintergrund), Master, Formatvorlagen, Schriftliste, Farbtabelle, Themes, Dokument-ShapeSheet, Ereignisliste, Datenverbindungen, Kommentare, Validierung, Fensterzustand, Eigenschaften. Neue Zeichnungen heißen „Zeichnung1“, „Zeichnung2“ … (Zähler pro Programmlauf). | SbS 1; Z2, Z3 | P1 | Lib-core | neu |
| DM-02 | Rückgängig/Wiederholen | Jede Benutzeraktion ist **ein** Schritt, auch wenn sie viele Zellen ändert (z. B. Shape einfügen samt Verbinder-Teilung, Organigramm-Stil wechseln); beliebig tiefe Liste mit Namen der Aktion. | SbS 2, 5 | P1 | Lib-core+Lib-ui | teilweise |
| DM-03 | Einstellungen im Dokument | Einrasten (an/aus, worauf: Raster, Lineal-Unterteilungen, Führungslinien, Shape-Ziehpunkte, Scheitelpunkte, Verbindungspunkte, Shape-Geometrie, Ausrichtungsrahmen, Schnittpunkte), Kleben (an Führungslinien, Shape-Ziehpunkte, Scheitelpunkte, Verbindungspunkte, Geometrie), Einrast-Stärke, dynamisches Raster, Einrastwinkel; Schutz von Formatvorlagen/Shapes/Mastern/Hintergründen. | Z2, Z3; SbS 2 | P1 | Lib-core | neu |
| DM-04 | Standard-Formatvorlagen | Dokumentweite Vorgaben für Linie, Füllung, Text und Führungslinien (`DefaultLineStyle` …), Startseite (`TopPage`). | Z2, Z3 | P1 | Lib-core | neu |
| DM-05 | Schriften und Farben | Schriftliste mit Panose/Zeichensätzen und Ersatzschrift, wenn eine Schrift fehlt; alte Farbtabelle (Index → RGB) für Formeln mit Farbindex. | Z2, Z3 | P1 | Lib-core+Lib-ui | teilweise |
| DM-06 | Dateigröße verringern | Unbenutzte Master, Formatvorlagen, Themes und Datenquellen entfernen; Vorschau der Ersparnis. | SbS 1 | P2 | Lib-core+App | neu |
| DM-07 | Persönliche Informationen entfernen | Autor/Manager/Firma, Markup und zwischengespeicherte Fremddaten aus dem Dokument löschen; optional Warnung, wenn sie später wieder hineinkommen. | SbS 12 | P2 | Lib-core+App | neu |
| DM-08 | Kompatibilitätsprüfung | Liste der Inhalte, die ältere Formate (`.vdx`) oder ältere Visio-Versionen nicht darstellen; Kompatibilitätsmodus für importierte Binärdateien mit „Konvertieren“. | SbS 1 | P3 | Lib-core+Lib-io+App | neu |
| DM-09 | Schreibgeschützt öffnen | Datei nur lesen; Speichern führt zu „Speichern unter“. | SbS 1 | P1 | App | neu |
| DM-10 | Automatisch wiederherstellen | Regelmäßige Sicherung offener Zeichnungen, Wiederherstellen nach Absturz. | SbS 1 | P2 | App+Lib-plat | teilweise |
| DM-11 | Dokumentschutz | Formatvorlagen, Shapes, Master und Hintergründe gegen Änderung schützen (Dialog „Dokument schützen“, optional mit Kennwort); die Auswahlsperre einzelner Shapes wirkt erst, wenn zusätzlich „Shapes“ geschützt ist. | Z2, Z3; UG Kap. 19 | P2 | Lib-core+App | neu |

### 2.2 Seiten

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| PG-01 | Vordergrundseiten | Geordnete Liste von Zeichenblättern mit Name (lokalisiert + universell), eigener Größe, Ausrichtung, Maßeinheit und eigenem Design; gemischte Hoch-/Querformate in einer Datei. | SbS 3; Z3 | P1 | Lib-core | neu |
| PG-02 | Hintergrundseiten | Seiten, deren Inhalt hinter zugewiesenen Seiten erscheint, dort aber nicht auswählbar ist; Ketten (Hintergrund eines Hintergrunds); je Seite genau ein direkter Hintergrund; automatisch erzeugte heißen „VHintergrund-n“, manuelle „Hintergrund-n“. | SbS 3 | P1 | Lib-core | neu |
| PG-03 | Seiten verwalten | Einfügen (nach der aktuellen oder am Ende, erbt Eigenschaften der aktiven Seite), Duplizieren (Inhalt + Eigenschaften), Umbenennen, Löschen, Neu anordnen (Ziehen der Register oder Dialog), Seitenzählung. | SbS 3; A 184030 | P1 | Lib-core+App | neu |
| PG-04 | Seitengröße | Wie Druckerpapier, vordefinierte Formate (ISO A/B/C, US Letter/Legal/Tabloid, ANSI/ARCH), eigene Größe, „an Zeichnungsinhalt anpassen“; Ausrichtung unabhängig vom Papier. | SbS 3; Z2, Z3 | P1 | Lib-core | neu |
| PG-05 | Automatische Größe | Seite wächst kachelweise, wenn Shapes über den Rand gezogen werden, und schrumpft wieder, wenn der Platz leer wird; pro Seite schaltbar; ohne sie bleiben Shapes auf der Zeichenfläche außerhalb der Seite (nicht gedruckt). | SbS 3; Z2, Z3 | P1 | Lib-core+Lib-ui | neu |
| PG-06 | Zeichnungsmaßstab | Keiner (1:1), vordefiniert in den Gruppen Architektur, Bauingenieur, Metrisch (z. B. 1:1000 bis 50:1), Maschinenbau, oder eigener Maßstab „Seitenmaß = Zeichnungsmaß“; Lineale und Maße zeigen Zeichnungseinheiten, Shapes mit Maßstabsbezug behalten reale Größen; je Seite eigener Maßstab, Hintergrundseiten werden separat eingestellt. | SbS 7; Z2, Z3; UG Kap. 18 | P1 | Lib-core | neu |
| PG-07 | Maßeinheiten | Rund 20 Einheiten je Seite (mm, cm, m, km, Zoll, Fuß, Yard, Meile, Punkt, Pica, Cicero, Didot, Tage, Wochen …); Eingaben mit Einheit werden umgerechnet. | SbS 2, 3; Z3 | P1 | Lib-core | neu |
| PG-08 | Seitenfarbe und -schatten | Hintergrundfarbe der Seite (über Theme/Hintergrund) und optionaler Seitenschatten in der Ansicht. | Z2, Z3 | P2 | Lib-core+Lib-ui | neu |
| PG-09 | Hintergründe (Galerie) | Fertige Hintergrundmotive als Hintergrundseite anlegen und zuweisen; „Kein“ entfernt. **Motive neu gestalten** (Visios Motive sind geschützt). | SbS 3 | P2 | App | neu |
| PG-10 | Rahmen und Titel (Galerie) | Seitenrahmen mit Titelblock, Seitenzahl, Datum als Hintergrundseite; kombinierbar mit Hintergrund. **Neu gestalten.** | SbS 3 | P2 | App | neu |
| PG-11 | Seiten-Layout und Routing | Je Seite: Platzierungsstil (Flussdiagramm, Baum, radial, kreisförmig …), Richtung, Verbinder-Routing-Stil, Linienübergänge (Art, Größe, welche Linien springen), Abstände; bildet den Dialogteil „Layout und Routing“ ab. | SbS 3; Z2, Z3 | P1 | Lib-core | neu |
| PG-12 | Seiten-Shape-Daten und -Zellen | Seiten haben eigene Shape-Daten, User-Zellen, Hyperlinks und Aktionen (z. B. für Titelblöcke). | SbS 8; Z2, Z3 | P2 | Lib-core | neu |
| PG-13 | Seiten-Ansichtszustand | Zoom und Mittelpunkt je Seite merken (`ViewScale`, `ViewCenterX/Y`). | Z2, Z3 | P2 | Lib-core+App | neu |
| PG-14 | Seitengröße am Rand ziehen | Mit Strg an einer Seitenkante ziehen ändert die Größe nur dieser Seite; Shapes behalten ihre Linealkoordinaten. | UG Kap. 3 | P2 | App+Lib-ui | neu |
| PG-15 | Seitenansicht drehen | Ansicht der Seite um den Linealnullpunkt drehen (nur Ansicht, nicht Druck), z. B. um an einer schrägen Führungslinie zu arbeiten; „waagerecht/senkrecht ansehen“ an einer Führungslinie; standardmäßig aus. | UG Kap. 3 | P3 | App+Lib-ui | neu |

### 2.3 Ebenen

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| LY-01 | Ebenen je Seite | Benannte Eigenschaftsgruppen (keine Stapelreihenfolge); ein Shape auf keiner, einer oder mehreren Ebenen; gleiche Namen auf verschiedenen Seiten sind getrennt. | SbS 3; Z2, Z3 | P1 | Lib-core | neu |
| LY-02 | Ebeneneigenschaften | Sichtbar, Drucken, Aktiv (neue Shapes landen dort), Sperren (nicht wähl- oder änderbar), Einrasten, Kleben, Farbe + Transparenz (überschreibt die Farbe aller Mitglieder), Anzahl Shapes. | SbS 3; Z2, Z3 | P1 | Lib-core+App | neu |
| LY-03 | Zuordnen und Automatik | „Ebene zuweisen“-Dialog mit Mehrfachwahl; Master bringen ihre Ebene mit (z. B. „Verbinder“, „Container“, „Geräte“) und legen sie beim Ablegen an; Kopieren auf andere Seite legt fehlende Ebenen an. | SbS 3; Z3 | P1 | Lib-core+App | neu |
| LY-04 | Ebene löschen | Löscht ihre Shapes, außer solche, die noch auf einer anderen Ebene liegen (vorher warnen). Ebenen werden nicht an andere Seiten vererbt; Einrast- und Klebesperren wirken nur in eine Richtung (auf Shapes der Ebene). | SbS 3; UG Kap. 15 | P1 | Lib-core+App | neu |
| LY-05 | Nach Ebene auswählen | Alle Shapes einer oder mehrerer Ebenen auswählen, auch „keine Ebene“. | SbS 3; UG Kap. 15 | P2 | App | neu |

### 2.4 Master, Schablonen und Formatvorlagen

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| MS-01 | Master | Vorlage-Shape in Schablone oder Dokument: Name (lokal + universell), eindeutige ID und Basis-ID (GUID), Symbolbild, QuickInfo (Prompt), Symbolgröße, Ausrichtung des Namens, „Abgleich über Namen“, verborgen, Typ; Inhalt = eigene Seite mit Shapes. | Fav, Z2, Z3; SbS 1 | P1 | Lib-core | neu |
| MS-02 | Instanzen und Vererbung | Ein abgelegtes Shape verweist auf den Dokument-Master und speichert nur Abweichungen; Ablegen desselben Masters wird über die eindeutige ID mit einer vorhandenen Dokument-Kopie abgeglichen; Master im Dokument bearbeiten wirkt auf alle Instanzen. | Z2, Z3; SbS A | P1 | Lib-core | neu |
| MS-03 | Master-Verknüpfungen | Schablonen-Einträge, die auf Master anderer Schablonen zeigen, optional mit Ablege-Aktionen (Zellen beim Ablegen setzen). | MS-VSDX | P3 | Lib-core | neu |
| MS-04 | Schnell-Shapes | Pro Schablone ein oberer Bereich (durch Linie getrennt) mit den Shapes, die AutoVerbinden und die Schnell-Shapes-Leiste anbieten; Reihenfolge per Ziehen. | SbS 2 | P1 | Lib-core+App | neu |
| MS-05 | Dokumentschablone | Alle Master eines Dokuments als eigene Schablone anzeigen („Dokumentschablone“), dort bearbeiten, löschen, umbenennen. | W | P2 | App | neu |
| MS-06 | Eigene Schablonen | Neue Schablone anlegen, Shapes hineinziehen (Master entsteht), Master bearbeiten (Symbol, Name, Eigenschaften), speichern; „Favoriten“ und „Meine Shapes“ als feste Orte. | Fav; SbS A; S 1104-132533 | P1 | App+Lib-io | neu |
| MS-07 | Metrische und US-Varianten | Jede mitgelieferte Schablone/Vorlage in metrischer (A4, mm) und US-Fassung (Letter, Zoll); Wahl beim Erstellen. | SbS 1 | P2 | App | neu |
| MS-09 | Master-Eigenschaften | Name, QuickInfo, Ausrichtung und Größe des Symbols, Symbol automatisch oder manuell aktualisieren, Mastertyp (Shape, Linienmuster, Linienende, Füllmuster), Musterverhalten, „beim Ablegen über Namen abgleichen“. | UG Kap. 5 | P1 | App+Lib-core | neu |
| MS-10 | Instanz und Master im Detail | Ändert man eine Eigenschaft an einer Instanz, folgt nur diese Eigenschaft nicht mehr dem Master; alle anderen schon. Weicht der Maßstab eines Masters um mehr als Faktor 8 vom Seitenmaßstab ab, wird die Instanz beim Ablegen umgerechnet. | UG Kap. 5, 18 | P1 | Lib-core | neu |
| MS-11 | Muster als Master | Eigene Linienmuster (entlang der Linie biegen, kacheln, strecken, Originallinie zeigen), Linienenden (an der Linie oder aufrecht ausgerichtet) und Füllmuster (kacheln, zentriert, gestreckt) als Master in Schablonen; skaliert mit dem Maßstab oder fest; Farbe vom Shape oder eigene. Erscheinen in den Linien- und Füll-Listen. | UG Kap. 5; Z2, Z3 | P2 | Lib-core+App | neu |
| MS-12 | Shape-Hilfe | Je Master und je Vorlage ein Hilfetext (Kontextmenü „Hilfe zu diesem Shape“). | UG Kap. 1 | P3 | App | neu |
| MS-08 | Formatvorlagen (StyleSheets) | Benannte Linien-/Füll-/Text-Vorlagen mit Vererbung untereinander; Standardsatz „Kein Stil“, „Normal“, „Keine“, „Nur Text“, „Führungslinie“, „Theme“, „Verbinder“; Definieren mit „basiert auf“, „enthält Text/Linie/Füllung“, „lokale Formatierung beibehalten“; Zuweisen mit Rückfrage, ob alle Teile oder nur einer gelten; „Format des Masters verwenden“ setzt zurück; beim Einfügen gewinnt eine gleichnamige Vorlage im Ziel. | Z2, Z3; UG Kap. 14 | P2 | Lib-core+App | neu |

### 2.5 Shapes, Geometrie und Gruppen

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| SP-01 | Shape-Arten | 2D-Shapes (Fläche mit Rand), 1D-Shapes (Anfang/Ende, auch wenn sie flächig aussehen wie Rack-Geräte), Gruppen, Fremdobjekte (Bilder, Metadateien, OLE), Führungslinien und Führungspunkte; jedes Shape hat ID, Namen „Blatt.n“ bzw. „<Master>.n“ und frei wählbaren Namen. | SbS 2, 7, 11; Z2, Z3 | P1 | Lib-core | neu |
| SP-02 | Geometrie darstellen | Pfade aus Linien, Kreisbögen, elliptischen Bögen, Ellipsen, NURBS, Polylinien, Splines und relativen Bezierkurven; mehrere Geometrieabschnitte je Shape mit eigener Füll-/Linien-/Sichtbarkeitssteuerung (Grundlage für „intelligente“ Shapes, die Teile ein- und ausblenden). | Z2, Z3; SbS 1, 4 | P1 | Lib-core+Lib-ui | teilweise |
| SP-03 | Transformation | Verschieben, Größe ändern (Kanten- und Eckgriffe, Eckgriffe proportional), Drehen um den Drehpunkt (frei, 15°-Raster nahe am Griff, feiner weiter weg), Spiegeln horizontal/vertikal, Drehpunkt verlegen; gesperrte Eigenschaften zeigen gesperrte Griffe. | SbS 2, 3 | P1 | Lib-core+Lib-ui | teilweise |
| SP-04 | Steuerpunkte | Gelbe Griffe, die der Shape-Designer an Zellen bindet (Pfeilspitzenbreite, Textposition, Ecken, Kabelanschlüsse); können wie Verbinderenden an anderen Shapes kleben (Ethernet-Bus, Legendenschwanz). | SbS 2, 7; Z2, Z3 | P1 | Lib-core+Lib-ui | neu |
| SP-05 | Scheitelpunkte bearbeiten | Mit Stift- oder Linienwerkzeug Scheitelpunkte und Kontrollpunkte verschieben, mit Strg+Klick auf den Umriss hinzufügen, mit Entf löschen, mehrere mit Umschalt gemeinsam ziehen; Bögen über Kontroll- und Exzentrizitätsgriffe umformen; ein Segment beim Zeichnen durch Zurückziehen zurücknehmen; geschlossene Linienzüge werden gefüllte 2D-Shapes. | SbS 2; UG Kap. 4, 24 | P1/P2 | Lib-core+Lib-ui | neu |
| SP-06 | Gruppen | Gruppieren/Gruppierung aufheben (mit Warnung bei formelgebundenen Gruppen-Shapes), verschachtelte Gruppen, Unterauswahl per zweitem Klick, Gruppe hat eigenen Text/Daten/Geometrie, Größenänderung skaliert oder verschiebt Kinder (ResizeMode), Ablegen in Gruppe, Gruppe zuerst oder Kinder zuerst auswählen. | SbS 3; Z2, Z3 | P1 | Lib-core+Lib-ui | teilweise |
| SP-07 | Stapelreihenfolge | Jedes neue Shape liegt oben; In den Vordergrund/Hintergrund, Eine Ebene nach vorne/hinten. | SbS 6 | P1 | Lib-core+Lib-ui | teilweise |
| SP-08 | Shape-Operationen | Vereinigen, Kombinieren, Fragmentieren, Schneiden (Schnittmenge), Subtrahieren, Verbinden, Trimmen, Versatz, Kurve anpassen (Polygon → Spline durch die Punkte), benutzerdefiniertes Anpassen (Toleranz, Kreisbögen, Spitzen erhalten; vereinfacht importierte Grafiken): aus mehreren Shapes neue Geometrie berechnen. | UG Kap. 24; A 183847 | P2 | Lib-core | neu |
| SP-09 | Bilder als Shapes | Eingefügte Bilder sind Shapes: zuschneiden (Bild im Rahmen verschieben), drehen, Helligkeit, Kontrast, Gamma, Transparenz; SVG-Bilder als Vektor. | SbS 3, 5, 12; Z2 | P1 | Lib-core+Lib-img | teilweise |
| SP-10 | Führungslinien und -punkte | Aus dem Lineal ziehen (Führungspunkt aus der Linealecke); Shapes kleben mit Kante, Mitte, Griff oder Endpunkt daran und wandern beim Verschieben mit; nicht gedruckt; drehbar (Kleber bricht, wenn man nach dem Kleben dreht); per „Größe und Position“ genau setzen; Ausrichten/Verteilen kann Führungslinien anlegen und die Shapes ankleben; gleichabständig per Strg-Ziehen und Wiederholen. | SbS 2; UG Kap. 4, 17 | P1 | Lib-core+Lib-ui | neu |
| SP-11 | Shape ändern (ersetzen) | Ausgewählte Shapes durch einen anderen Master ersetzen und dabei Text, Shape-Daten, Format, Kommentare, Klebungen und die meisten Anpassungen übernehmen (gesteuert durch die Ersetzen-Zellen); mehrere auf einmal. | SbS 3; Z2, Z3 | P2 | Lib-core+App | neu |
| SP-12 | Intelligente Shapes | Verhalten aus Formeln: Teile erscheinen beim Vergrößern (Konferenztisch mit Stühlen), Typ per Kontextmenü umschalten (BPMN-Ereignis/-Aufgabe), Höhe in Rack-Einheiten, Text hält 45°-Schritte. Kein Sondercode je Shape – alles über ShapeSheet. | SbS 1, 3, 4, 7 | P1 | Lib-core | neu |
| SP-13 | Shape-Daten beim Ablegen abfragen | Felder mit „beim Ablegen fragen“ öffnen nach dem Ablegen/Duplizieren einen Eingabedialog. | SbS 8 | P2 | App | neu |
| SP-14 | Verschieben ohne Raster und Feinschritte | Pfeiltasten verschieben um Rasterschritte, mit Umschalt um ein Pixel/feine Schritte; Alt beim Ziehen hebt Einrasten auf. | SbS B | P1 | Lib-ui | vorhanden |
| SP-15 | Gruppe öffnen | Eigenes Fenster zum Bearbeiten der Mitglieder einer Gruppe; Shapes aus Seite oder Schablone lassen sich hineinziehen; wahlweise Doppelklick-Aktion. | UG Kap. 4, 7 | P2 | App | neu |
| SP-16 | Ausrichtungsrahmen aktualisieren | Rahmen einer Gruppe nach Änderungen im Inneren neu berechnen. | UG Kap. 4 | P2 | Lib-core | neu |
| SP-17 | Grenzfälle Gruppen | Führungslinie und angeklebte Shapes nur gemeinsam gruppieren; Bitmaps drehen bei Gruppendrehung nicht mit; importierte Metadateien erst in Gruppe umwandeln (In Gruppe umwandeln). | UG Kap. 4, 9 | P2 | Lib-core | neu |

### 2.6 Format: Linie, Füllung, Schatten, Effekte

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| FM-01 | Linie | Farbe + Transparenz, Stärke, Strichmuster (rund 24 Muster + eigene), Abschluss (rund/eckig/flach), Ecken-Verbindungstyp, Eckenrundung, Doppel-/Dreifachlinie, Verlaufslinie. | SbS 6; Z2, Z3 | P1 | Lib-core+Lib-ui | teilweise |
| FM-02 | Linienenden | Rund 45 Pfeilspitzen und Endsymbole für Anfang und Ende, je in Größen sehr klein bis kolossal; eigene Linienenden über Muster-Master. | SbS 6; Z2, Z3 | P1 | Lib-core+Lib-ui | teilweise |
| FM-03 | Füllung | Keine, einfarbig mit Transparenz, Farbverlauf (linear/radial/rechteckig/Pfad, Winkel, Richtung, Stopps mit Farbe/Position/Transparenz/Helligkeit, Voreinstellungen), Muster (Schraffuren mit Vorder-/Hintergrundfarbe); alte Visio-Füllmuster lesen. | SbS 6; Z2, Z3 | P1 (einfarbig), P2 | Lib-core+Lib-ui | teilweise |
| FM-04 | Schatten | Aus/an, Art (einfach, schräg), Farbe + Transparenz, Versatz, Größe, Winkel, Weichzeichnung; Standard-Versatz je Seite. | SbS 6; Z2, Z3; UG Kap. 5 | P2 | Lib-core+Lib-ui | neu |
| FM-05 | Effekte | Spiegelung, Leuchten, weiche Kanten, Abschrägung (3D-Format), 3D-Drehung, jeweils mit Voreinstellungen und Detailoptionen. | SbS 6; Z3 | P3 | Lib-core+Lib-ui | neu |
| FM-06 | Skizzeneffekt | Handgezeichneter Look für Linien und Füllung (für Designs „Handgezeichnet“). | SbS 6; Z2, Z3 | P3 | Lib-core | neu |
| FM-07 | Format übertragen | Format eines Shapes auf eines (Einfachklick) oder viele (Doppelklick, Ende mit Esc/Zeigerwerkzeug) übertragen, inklusive Theme-Bezug, Effekten und Schnellformatvorlage. | SbS 6 | P1 | App | neu |
| FM-08 | Farbwähler | Theme-Farben (Weiß, Schwarz, Hell, Dunkel, Akzent 1–6 je mit Abstufungen), Variantenfarben, Standardfarben, zuletzt verwendet, weitere Farben (Wabe, RGB/HSL-Eingabe), keine Farbe; Theme-Farben folgen dem Design, feste Farben nicht. | SbS 6 | P1 | Lib-ui+App | teilweise |
| FM-09 | Format wiederholen | Letzte Formatänderung mit F4 auf eine andere Auswahl anwenden. | UG Kap. 5 | P2 | App | neu |

### 2.7 Text und Felder

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| TX-01 | Shape-Text | Jedes Shape hat Text; Eingabe durch Lostippen, Doppelklick oder F2; Enter-Verhalten einstellbar (Absatz oder Eingabe beenden); Textsperre respektieren. | SbS 3 | P1 | Lib-core+Lib-ui | teilweise |
| TX-02 | Formatierter Text | Mehrere Läufe mit eigenem Zeichenformat, Absätze mit eigenem Absatzformat, Tabstopps, Aufzählungen und Nummerierung, Rechts-nach-links-Text. | Z2, Z3; SbS 3 | P1 | Lib-core+Lib-ui | teilweise |
| TX-03 | Textblock | Eigener Rahmen im Shape: verschieben, Größe ändern, drehen (nur Text dreht), neun Ausrichtungen (links/Mitte/rechts × oben/Mitte/unten), Ränder, Hintergrund, senkrechter Text. | SbS 3, 4; Z2, Z3 | P1 | Lib-core+Lib-ui | neu |
| TX-04 | Textausrichtung beim Drehen | Text dreht mit dem Shape, bleibt aufrecht (Verbinder) oder folgt einer Formel. | SbS 3 | P1 | Lib-core | neu |
| TX-05 | Textfelder (Textfeld-Shapes) | Reine Text-Shapes waagerecht oder senkrecht; mit Format, Design und Effekten wie jedes Shape. | SbS 3 | P1 | Lib-core+App | neu |
| TX-06 | Felder im Text | Dynamische Werte im Shape-Text: Shape-Daten, Datum/Uhrzeit, Dokumentinfo (Autor, Titel, Dateiname …), Seiteninfo (Name, Nummer, Anzahl), Geometrie (Breite, Höhe, Winkel), Objektinfo (ID, Name, Master), benutzerdefinierte Zellen, eigene Formel; jeweils mit Anzeigeformat; aktualisieren sich selbst. | SbS 8 | P1 | Lib-core+App | neu |
| TX-07 | Rechtschreibprüfung | Rechtschreibung und AutoKorrektur im Shape-Text, Sprache je Textlauf. | SbS 1 | P2 | App+Lib-ui | vorhanden (spell) |
| TX-08 | Suchen und Ersetzen | Text in Shapes, Shape-Daten, Shape-Namen und Master-Namen suchen, auf Auswahl/Seite/alle Seiten, Groß-/Kleinschreibung, ganze Wörter. | W | P1 | App | neu |
| TX-09 | Schriftersatz | Fehlende Schriften ersetzen und melden; Theme-Schriften (Überschrift/Text) folgen dem Design. | Z2, Z3 | P1 | Lib-ui | teilweise |
| TX-10 | Tabstopps und Textlineal | Bis zu zehn Tabstopps je Absatz (links, zentriert, rechts, dezimal), relativ zum Textblock; Lineal-Nullpunkt per Strg-Ziehen an den Textblockrand. | UG Kap. 7 | P2 | Lib-core+App | teilweise (jfWord-Lineal) |

### 2.8 Verbinder, Kleben, Routing und Layout

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| CN-01 | Verbindungspunkte | Punkte am Shape mit Richtung und Art (nach innen/außen/beides); sichtbar beim Verbinden (grau, Ziel grün) oder immer (Ansicht); hinzufügen, verschieben, löschen mit dem Verbindungspunkt-Werkzeug. | SbS 2; Z2, Z3 | P1 | Lib-core+Lib-ui | teilweise |
| CN-02 | Statisches Kleben | Verbinderende an einem bestimmten Punkt (Verbindungspunkt, Scheitelpunkt, Führungslinie, Steuerpunkt): bleibt dort, egal wohin sich die Shapes bewegen. | SbS 2, 4; Z2, Z3 | P1 | Lib-core+Lib-ui | teilweise |
| CN-03 | Dynamisches Kleben | Verbinderende auf das ganze Shape (Rand leuchtet): Andockpunkt wird bei jeder Bewegung neu gewählt; nur für dynamische Verbinder; Mischformen je Ende; mit Strg beim Ziehen zwischen statisch und dynamisch umschalten; Endpunkte zeigen die Art durch Farbe und Größe. | SbS 2; Z2, Z3; UG Kap. 6 | P1 | Lib-core+Lib-ui | neu |
| CN-04 | Dynamischer Verbinder | 1D-Shape, dessen Knicke automatisch entstehen und verschwinden; Stile rechtwinklig (Standard), gerade, gebogen; Knick- und Segment-Ziehpunkte zum Verschieben, Hinzufügen, Entfernen; „Verbinder zurücksetzen“ zeichnet neu mit wenigen Knicken; Text auf dem Verbinder bleibt aufrecht, mit eigenem Hintergrund. | SbS 2, 4; Z2, Z3 | P1 | Lib-core+Lib-ui | teilweise |
| CN-05 | Routing um Hindernisse | Rechtwinklige Wege um Shapes herum mit Gassen- und Blockabständen, Durchlässigkeit je Shape, Richtung der Verbindungspunkte, Seitenrouting-Stil (rechtwinklig, gerade, Flussdiagramm, Baum, Organigramm, einfach, Netzwerk …) und Richtung; Neu-Verlegen bei Bewegung. | SbS 2, 4; Z2, Z3; UG | P1 | Lib-core | neu |
| CN-06 | Linienübergänge | Wo Verbinder sich kreuzen: Bogen, Lücke, Quadrat oder 2- bis 7-seitiger Sprung; welche Linie springt (waagerechte, senkrechte, zuletzt verlegte, Anzeigereihenfolge), Größe; je Seite und je Verbinder. | Z2, Z3; W | P2 | Lib-core+Lib-ui | neu |
| CN-07 | AutoVerbinden | Beim Zeigen auf ein Shape vier blaue Pfeile; Klick verbindet mit dem nächsten Shape in dieser Richtung oder legt das in der Schablone markierte Shape im Standardabstand an und verbindet es; Ziehen eines Pfeils auf ein beliebiges Shape; Schnell-Shapes-Leiste mit den ersten vier Schnell-Shapes und Live-Vorschau; schaltbar je Dokument und global. | SbS 2, 4 | P1 | App+Lib-ui | neu |
| CN-08 | Automatisch einfügen | Shape auf einen Verbinder fallen lassen teilt ihn, fügt das Shape ein und schiebt folgende Shapes beiseite. | SbS 2; Z2, Z3 | P2 | Lib-core+App | neu |
| CN-09 | Automatisch löschen | Shape löschen entfernt überflüssige Verbinder; liegt es zwischen zwei Shapes, verbindet der verbleibende Verbinder beide. | SbS 2 | P2 | Lib-core+App | neu |
| CN-10 | Verbinderwerkzeug-Automatik | Bei aktivem Verbinderwerkzeug verbindet ein aus der Schablone gezogenes Shape sich mit dem zuvor gewählten Shape. | SbS 4 | P2 | App | neu |
| CN-11 | Verbinder-Werkzeug „einmal“ | Über die Minisymbolleiste genau einen Verbinder zeichnen, dann zurück zum Zeiger. | SbS 2 | P2 | App | neu |
| CN-12 | Linien statt Verbinder | Gerade Linien, Bögen, Freihand und Stift kleben ebenfalls an Verbindungspunkten, behalten aber ihre Form. | SbS 2 | P1 | Lib-core+Lib-ui | teilweise |
| CN-13 | Seite neu layouten | Automatisches Anordnen ganzer Diagramme nach Stil (Flussdiagramm, Baum, Organigramm, radial, kreisförmig, kompakter Baum, hierarchisch) und Richtung (oben→unten, links→rechts …), mit Abständen und Verbinder-Stil; Tiefe flach/mittel/tief (wie weit der Algorithmus vorausschaut); internes Layout-Raster (Block- und Gassengröße, Abstand Linie-zu-Linie und Linie-zu-Shape); Flussrichtung folgt Anfang→Ende der Verbinder; Live-Vorschau. | A 183642; TP; UG | P2 | Lib-core | neu |
| CN-14 | Auto-Ausrichten und -Abstand | Verbundene Shapes automatisch ausrichten und/oder gleichmäßig verteilen (Auswahl oder Seite), mit Live-Vorschau. | SbS 6 | P2 | Lib-core+App | neu |
| CN-15 | Verbindungsmodell | Einträge „von Zelle/Teil → zu Zelle/Teil“ (Anfang, Ende, Steuerpunkt → Verbindungspunkt, PinX, Geometrie); bleibt beim Speichern erhalten und wird bei jeder Bewegung neu gerechnet. | Z2, Z3 | P1 | Lib-core | neu |
| CN-16 | Shapes verbinden (Befehl) | Ausgewählte Shapes in Auswahlreihenfolge mit Verbindern verketten; nimmt den in der Schablone markierten Verbinder-Master, sonst den dynamischen Verbinder. | UG Kap. 6 | P1 | App | neu |
| CN-17 | Layout-Verhalten je Shape | „platzieren und umgehen“, „nie platzieren“, „automatisch“ (wird platzierbar, sobald ein dynamischer Verbinder angeklebt wird); steuert Seite-neu-layouten und Routing. | UG Kap. 6; Z2, Z3 | P1 | Lib-core+App | neu |

### 2.9 Container, Listen, Swimlanes, Beschriftungen, Legenden

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| ST-01 | Container | Umrandung mit Kopfzeile und logischer Mitgliedschaft; verschachtelbar; bewegt/kopiert/löscht sich mit Inhalt; Inneres ist „durchklickbar“ (Mitglieder direkt wählen, Auswahlrechteck innen starten); Hineinziehen macht Shapes zu Mitgliedern (Rand leuchtet), Herausziehen entfernt sie; Text tippen geht in die Kopfzeile. | SbS 13; Z2, Z3 | P1 | Lib-core+Lib-ui | teilweise (Rahmen in diagram.rs) |
| ST-02 | Containerformat | 14 Container-Stile und 2–4 Kopfzeilen-Stile je Stil (Kopf oben/unten/links/rechts, ohne Kopf), Ränder, automatische Größe (keine / nach Bedarf erweitern / immer an Inhalt anpassen), „An Inhalt anpassen“, Sperren, Inhalt auswählen, Container auflösen, ausgeschlossene/erforderliche Kategorien. **Stile neu gestalten.** | SbS 13; Z2, Z3 | P1 | Lib-core+App | neu |
| ST-03 | Randmitglieder | Shapes auf dem Containerrand bleiben an der Kante haften und wandern mit ihr (z. B. Register an einem Bedienfeld). | SbS 13 | P2 | Lib-core | neu |
| ST-04 | Listen | Container mit geordneten Mitgliedern (waagerecht/senkrecht, Richtung umkehrbar, Abstand); Einfügemarke beim Hineinziehen; Umordnen per Ziehen; Einfüge-Dreieck legt Standardmitglied an; Größe folgt dem Inhalt; Mitglieder kennen ihre Position (Nummerierung per Formel). | SbS 13 | P1 | Lib-core+Lib-ui | neu |
| ST-05 | Swimlanes | Funktionsübergreifendes Flussdiagramm als Liste aus Bahn-Containern mit Titelleiste und Phasen-Trennern; waagerecht/senkrecht; Bahn einfügen (Knopf, Kontextmenü vor/nach, Ziehen auf Grenze, Einfüge-Dreieck); Bahnen per Kopf umordnen; Shapes übernehmen den Bahntitel als Datenwert („Funktion“). | SbS 4, 13 | P1 | Lib-core+App | neu |
| ST-06 | Beschriftungen (Visio: „Legenden“) | Strukturierte Beschriftungen (Callouts) mit Ziel-Shape: bewegen sich mit dem Ziel, werden mit ihm kopiert und gelöscht, bleiben beim eigenen Verschieben verbunden; mehrere je Shape; Lösen/Neu-Anheften über den gelben Steuerpunkt; 14 Stile mit und ohne Schwanz. **Neu gestalten.** | SbS 13 | P1 | Lib-core+App | neu |
| ST-07 | Klassische Beschriftungen | Ältere Beschriftungs-Shapes ohne Zielbeziehung (Schwanz klebt per Steuerpunkt). | SbS 13 | P3 | App | neu |
| ST-08 | Legende für Datengrafiken | Automatisch erzeugte Legende (waagerecht/senkrecht) aus Listen/Containern, die erklärt, was Symbole, Farben und Balken bedeuten; nach Änderungen neu einfügen. | SbS 9, 13 | P2 | App+Lib-core | neu |
| ST-09 | Zählende Legende (Netzwerk) | Legenden-Shape, das die verwendeten Shape-Typen einer Seite mit Anzahl auflistet (`visLegendShape`). | Fav, Z2, Z3 | P2 | App+Lib-core | neu |
| ST-10 | Wireframes | Bedienoberflächen-Entwürfe aus Containern (Dialog, Bedienfeld) und Listen (Listenfeld). | SbS 13 | P3 | App | neu |

### 2.10 Daten

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| DA-01 | Shape-Daten | Beliebig viele benannte Felder je Shape, Seite und Dokument; acht Typen (Text, Zahl, feste Liste, variable Liste, Dauer mit Einheit, Datum, Währung, Boolesch) mit Typprüfung und Anzeigeformat; Werte als Formeln. | SbS 8; Z2, Z3, Fav | P1 | Lib-core | neu |
| DA-02 | Shape-Daten definieren | Bezeichnung, Typ, Format bzw. Listenwerte, Standardwert, QuickInfo; im Entwicklermodus zusätzlich interner Name, Sortierschlüssel, beim Ablegen fragen, verborgen; neu/löschen; für mehrere Shapes gleichzeitig. | SbS 8 | P1 | App+Lib-core | neu |
| DA-03 | Shape-Datensätze | Benannte Feldsätze (z. B. „Netzwerkgerät“) einmal definieren und auf viele Shapes anwenden. | W | P3 | App+Lib-core | neu |
| DA-04 | Datenverknüpfung | Datenquelle (CSV und `.xlsx` in P2, ODBC/SQL in P3; F6) anbinden, eindeutigen Schlüssel wählen, Zeilen an Shapes binden (automatisch über Spaltenabgleich, per Ziehen auf ein Shape, durch Ziehen auf leere Fläche = neues Shape aus dem markierten Master), Spalten umbenennen/ausblenden/umtypisieren, mehrere Quellen und n:m-Bindungen, Bindung lösen, Quelle entfernen. | SbS 10 | P2 | App+Lib-core+Lib-io | neu |
| DA-05 | Daten aktualisieren | Alle oder einzelne Quellen manuell oder im Intervall aktualisieren; Konflikte (geänderte/gelöschte Zeilen, überschriebene Benutzerwerte) anzeigen und lösen. | SbS 10 | P2 | App+Lib-core+Lib-io | neu |
| DA-06 | Schnellimport | Ein Schritt von der Excel-Datei zu verknüpften Shapes samt Datengrafiken, als ein Rückgängig-Schritt. | SbS 1, 10 | P3 | App | neu |
| DA-07 | Datengrafiken | Je Shape eine Datengrafik aus mehreren Elementen: Text(-callout), Symbolsatz (bis 5 Symbole mit Regeln), Datenleiste (Fortschritt, Sterne, Torte, Thermometer, Tacho, Mehrfachbalken; Min/Max), Farbe nach Wert (eindeutige Werte oder Bereiche); Position je Element, Shape-Text ausblenden; liegt auf eigener Ebene; Galerie, Bearbeiten, Duplizieren, Umbenennen, Löschen, „Shapes mit dieser Grafik auswählen“. **Symbole und Stile neu gestalten.** | SbS 9, 10 | P2 | App+Lib-core | neu |
| DA-08 | Datengrafik-Felder | Bereich mit allen Feldern; Klick wendet eine passende Grafik an oder entfernt sie; Position und Konfiguration im Ribbon. | SbS 10 | P2 | App | neu |
| DA-09 | Shape-Berichte | Vordefinierte (Inventar, Netzwerkgeräte, PC, Flussdiagramm, Organigramm, Raum, Fenster/Türen …) und eigene Berichte: Auswahl der Shapes (alle Seiten/Seite/Auswahl, Bedingungen), Spalten, Titel, Zwischensummen, Sortierung, Format; speichern im Dokument oder als Berichtsdefinition; Ausgabe CSV/Excel, HTML, XML oder Tabellen-Shape (statt eingebettetem Excel). | SbS 7, 8 | P2 | App+Lib-core+Lib-io | neu |
| DA-10 | Diagramm überprüfen | Regelsätze im Dokument (Flussdiagramm, Swimlanes, BPMN mit vielen Regeln) prüfen: Problemliste mit Sprung zum Shape, sortierbar, Problem bzw. Regel ignorieren, ignorierte anzeigen, Regelsätze aus anderen Dokumenten importieren, Regelsätze ein/aus. **Zusätzlich eigener Regel-Editor** (Visio hat keinen). | SbS 14 | P2 (Prüfen), P3 (Editor) | Lib-core+App | neu |
| DA-11 | Organigramm-Daten | Personen mit Name, Titel, Abteilung, Telefon, E-Mail, Vorgesetzter; Fotos je Person. | SbS 5, 8 | P2 | App | neu |
| DA-12 | Datenbank-Verknüpfung (ODBC) | Shapes mit Tabellenzeilen verbinden (Schlüsselfelder, Feld ↔ Zelle, Richtung), Zeichnungsmonitor (jede Zeile ein Shape, Löschen/Anlegen wird übertragen), Master aus Tabelle, Shape-Daten in Tabelle exportieren. Die Metadaten liegen in User-Zellen und müssen erhalten bleiben. | UG Kap. 20–22 | P3 | App+Lib-core+Lib-io | neu |
| DA-13 | Bericht aktualisieren | Tabellen-Shape eines Berichts über das Kontextmenü neu erzeugen; Berichte über Ebenen getrennt. | UG Kap. 20 | P2 | App | neu |

### 2.11 Hyperlinks, QuickInfos und Kommentare

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| HL-01 | Hyperlinks | Je Shape (und Seite) mehrere Links: Adresse (Datei, URL, `mailto:`), Unteradresse (Seite, Shape, Zoom; Excel-Bereich, Word-Textmarke, Folie), Beschreibung (QuickInfo), relativer oder absoluter Pfad; Folgen mit Strg+Klick oder Kontextmenü; Zeiger zeigt Link-Symbol. | SbS 11 | P1 | Lib-core+App | neu |
| HL-02 | Hyperlinkbasis | Dokumenteigenschaft, die allen relativen Links vorangestellt wird. | SbS 11; Z2, Z3 | P2 | Lib-core | neu |
| HL-03 | QuickInfo am Shape | Text, der nur beim Zeigen auf das Shape erscheint (Kommentar-Zelle). | SbS 3 | P2 | Lib-core+App | neu |
| HL-04 | Kommentare | Kommentare an Shape (Anker-Shape bei Mehrfachauswahl) oder Seite, mit Autor, Zeit, Antworten (Verlauf), einzelne Einträge löschen; Symbol am Shape (ein-/ausblendbar); Kommentarbereich mit allen Kommentaren, Klick springt zum Shape. | SbS 3, 15; A 183811, A 185607 | P2 | Lib-core+App | neu |
| HL-05 | Freihand-Markup | Überprüfungsebenen je Prüfer (Markup anzeigen/hinzufügen), Freihandanmerkungen. | Z2, Z3; SbS 12 | P3 | Lib-core+App | neu |
| HL-06 | Link-Verlauf | Vor/Zurück zwischen angesprungenen Seiten und Dokumenten (Alt+Pfeil links/rechts). | UG Kap. 25 | P2 | App | neu |
| HL-07 | Hyperlink kopieren | Link eines Shapes kopieren und anderswo als Hyperlink einfügen. | UG Kap. 25 | P3 | App | neu |

### 2.12 Designs (Themes), Varianten und Schnellformatvorlagen

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| TH-01 | Theme-Modell | Theme-Teil (Farbschema mit Dunkel/Hell/Akzent 1–6/Hyperlink, Schriftschema, Format-Schema mit Füll-, Linien-, Effekt- und Hintergrundstilen) plus Visio-Erweiterungen: vier Farb- und Stilvarianten, Verbinder-Schema, Linienstile mit Pfeilen, Schriftstile, Verzierungsgrad; je Seite ein Theme. | Z2, Z3; SbS 6 | P1 | Lib-core | neu |
| TH-02 | Designs anwenden | Design auf aktuelle Seite oder alle Seiten, Live-Vorschau, „Design auf neue Shapes anwenden“, Shapes können Designs ignorieren („Designs zulassen“); Theme-gebundene Zellen (`THEMEVAL`) folgen automatisch. | SbS 6 | P1 | Lib-core+App | neu |
| TH-03 | Eigene Designs | JFDiagrammer bringt **eigene** Designs mit (Visios rund 26 Designs mit Namen wie „Linear“, „Einfach“, „Parallel“ sind geschützt und werden nur aus Dateien gelesen); Kategorien z. B. Professionell, Modern, Kräftig, Handgezeichnet. | Entscheidung; SbS 6; Z2, Z3 | P1 | App | neu |
| TH-04 | Varianten | Vier Varianten je Design (Farbe und Stil). | SbS 6 | P1 | Lib-core+App | neu |
| TH-05 | Farben, Effekte, Verbinder, Verzierung | Eigene Farbsätze anlegen (Akzentfarben, Textfarben auf hell/dunkel, Hintergrund), benutzerdefinierte Sätze löschen; Effekt- und Verbinder-Sätze wählen; Verzierungsgrad niedrig/mittel/hoch. | SbS 6 | P2 | Lib-core+App | neu |
| TH-06 | Schnellformatvorlagen | Je Design 4 Variantenstile und eine Matrix 6 Stärken × 7 Farbvarianten; wirkt auf die Auswahl; Kontrastschutz für Text. | SbS 6; Z2, Z3 | P2 | Lib-core+App | neu |
| TH-07 | Designs übertragen | Ein eingefügtes Shape bringt sein Design (auch eigene Farben) in ein anderes Dokument mit. | SbS 6 | P3 | Lib-core | neu |

### 2.13 ShapeSheet: Abschnitte

Das ShapeSheet ist das Herz des Formats: Jede Seite, jedes Shape, jeder Master, jede
Formatvorlage und das Dokument selbst sind Tabellen aus Zellen mit **Formel** (`F`) und
**zwischengespeichertem Wert** (`V`, mit Einheit `U`). JFDiagrammer muss alle Abschnitte lesen
und schreiben; die Prio sagt, ab wann sie auch **wirken** (Darstellung, Bearbeitung).

| ID | Abschnitt | Zellen / Zeilen (Auswahl) und Wirkung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| SS-01 | Shape-Transformation | PinX/PinY (Drehpunkt auf der Seite), Width/Height, LocPinX/LocPinY (Drehpunkt im Shape), Angle, FlipX/FlipY, ResizeMode (Gruppenkinder skalieren oder neu positionieren) | Z2, Z3; MS-VSDX | P1 | Lib-core | neu |
| SS-02 | 1D-Endpunkte | BeginX/BeginY/EndX/EndY: Linien und Verbinder sind über Anfang und Ende definiert; Breite = Länge, Winkel folgt | Z2, Z3 | P1 | Lib-core | neu |
| SS-03 | Linienformat | LinePattern, LineWeight, LineColor, LineColorTrans, LineCap, Rounding (Eckenrundung), CompoundType (Doppel-/Dreifachlinie), BeginArrow/EndArrow und deren Größe | Z2, Z3 | P1 | Lib-core | neu |
| SS-04 | Füllformat und Schatten | FillForegnd/FillBkgnd mit Transparenz, FillPattern (einfarbig, Schraffuren, alte Verlaufsmuster), Schatten: ShdwForegnd, ShdwPattern, ShapeShdwType/-OffsetX/-OffsetY/-ObliqueAngle/-ScaleFactor/-Blur/-Show | Z2, Z3 | P1 (Füllung), P2 (Schatten) | Lib-core | neu |
| SS-05 | Verlaufseigenschaften + Verlaufsstopps | Fill-/LineGradientEnabled, -Dir (linear, radial, rechteckig, Pfad), -Angle, RotateGradientWithShape, UseGroupGradient; eigene Zeilenabschnitte FillGradient/LineGradient mit Stopp-Farbe, -Transparenz, -Position | Z2, Z3 | P2 | Lib-core | neu |
| SS-06 | Zeichen (Character) | je Textlauf: Font, Size, Color + Transparenz, Style (fett/kursiv/unterstrichen/Kapitälchen), Case, Pos (hoch/tief), Strikethru, DblUnderline, DoubleStrikethrough, Overline, Letterspace, FontScale, LangID, asiatische und komplexe Schrift samt Größe | Z2, Z3 | P1 | Lib-core | neu |
| SS-07 | Absatz (Paragraph) | Einzüge erste Zeile/links/rechts, Zeilenabstand, Abstand vor/nach, horizontale Ausrichtung, Aufzählung (Bullet, BulletStr, BulletFont, Größe, Textabstand), Flags (rechts nach links) | Z2, Z3 | P1 | Lib-core | neu |
| SS-08 | Tabstopps | Position und Ausrichtung je Tabstopp | Z2, Z3 | P2 | Lib-core | neu |
| SS-09 | Textblockformat | Ränder links/rechts/oben/unten, vertikale Ausrichtung, Texthintergrund + Transparenz, Standard-Tabstopp, Textrichtung (waagerecht/senkrecht) | Z2, Z3 | P1 | Lib-core | neu |
| SS-10 | Texttransformation | TxtPinX/TxtPinY, TxtWidth/TxtHeight, TxtLocPinX/TxtLocPinY, TxtAngle: eigener Textblock, verschiebbar und drehbar unabhängig vom Shape | Z2, Z3 | P1 | Lib-core | neu |
| SS-11 | Textfelder (Field) | eingefügte Felder: Wert (Formel), Format (Datums-/Zahlenbild), Typ, Kalender, Bearbeitungsmodus, Kategorie/Code für den Feld-Dialog | MS-VSDX | P2 | Lib-core | neu |
| SS-12 | Ereignisse | EventDblClick (Doppelklick-Verhalten), EventDrop, EventMultiDrop, EventXFMod (Verschieben/Größe), TheData, TheText | Z2, Z3 | P2 | Lib-core | neu |
| SS-13 | Bildeigenschaften | Helligkeit, Kontrast, Gamma, Weichzeichnen, Schärfen, Rauschen entfernen, Transparenz | Z2 | P2 | Lib-core+Lib-img | neu |
| SS-14 | Fremdbild-Info | ImgOffsetX/Y, ImgWidth/ImgHeight (Zuschneiden), ClippingPath | Z2 | P1 | Lib-core | neu |
| SS-15 | Schutz | Sperren für Breite, Höhe, Seitenverhältnis, Verschieben X/Y, Drehen, Löschen, Auswählen, Formatieren, Textbearbeitung, Scheitelpunkt-Bearbeitung, Zuschneiden, Gruppieren, Anfang/Ende, Shape-Daten, Format aus Gruppe, Theme-Farben/-Effekte/-Verbinder/-Schriften/-Index, Variante, Ersetzen, Vorschau; der Schutz-Dialog zeigt nur einen Teil, der Rest nur im ShapeSheet; Schlösser erscheinen nur bei Größen-/Drehsperren | Z2, Z3; UG Kap. 4, 19 | P1 | Lib-core+App | neu |
| SS-16 | Verschiedenes | keine Objekt-/Steuergriffe, kein Ausrichtungsrahmen, nicht drucken, Text ausblenden, Ausrichtungsrahmen aktualisieren, dynamische Rückmeldung, ObjType (Shape/Verbinder/platzierbar), Ablegequelle, QuickInfo (Comment), Maßstab beim Ablegen, Kalender, Sprache, Shape-Schlüsselwörter (für die Suche) | Z2, Z3 | P2 | Lib-core | neu |
| SS-17 | Gruppeneigenschaften | SelectMode (Gruppe zuerst/Kinder zuerst/nur Gruppe), DisplayMode (Gruppe vor/hinter Kindern/ohne), IsDropTarget (Ablegen fügt zur Gruppe hinzu), IsSnapTarget, IsTextEditTarget, DontMoveChildren | Z2, Z3 | P1 | Lib-core | neu |
| SS-18 | Klebeinfo | BegTrigger/EndTrigger (Neuberechnung beim Kleben), GlueType (dynamisch erlaubt/verboten), WalkPreference (welche Seite ein dynamischer Verbinder wählt) | Z2, Z3 | P1 | Lib-core | neu |
| SS-19 | Shape-Layout | Durchlässigkeit für Verbinder (X/Y/Platzierung), Platzierungsverhalten (fest/verschiebbar/schiebt andere), Routing-Stil je Shape, Linienübergang-Code/-Stil/-Richtung je Verbinder, Platzierungsspiegelung, Verbinder-Erweiterung, Verbinder teilen (ShapeSplit/ShapeSplittable), Anzeigeebene (DisplayLevel), Beziehungen (Relationships: Container/Liste/Legende) | Z2, Z3 | P1 | Lib-core | neu |
| SS-20 | Ebenenzugehörigkeit | LayerMember: Liste der Ebenen-Indizes eines Shapes | Z3 | P1 | Lib-core | neu |
| SS-21 | Theme-Eigenschaften | ThemeIndex, ColorSchemeIndex, EffectSchemeIndex, ConnectorSchemeIndex, FontSchemeIndex, VariationColorIndex, VariationStyleIndex, EmbellishmentIndex (pro Seite und pro Shape überschreibbar) | Z2, Z3 | P2 | Lib-core | neu |
| SS-22 | Schnellformatvorlage | QuickStyleLineColor/-FillColor/-ShadowColor/-FontColor (Theme-Farbindex), -LineMatrix/-FillMatrix/-EffectsMatrix/-FontMatrix (Stärke), QuickStyleType, QuickStyleVariation (Kontrastschutz) | Z2, Z3 | P2 | Lib-core | neu |
| SS-23 | 3D-Drehung | RotationXAngle/YAngle/ZAngle, RotationType (Voreinstellung), Perspective, DistanceFromGround, KeepTextFlat | Z3 | P3 | Lib-core | neu |
| SS-24 | Abschrägung (Fase) | BevelTop/Bottom Type/Width/Height, Tiefe und Kontur (Farbe/Größe), Material, Beleuchtung und Winkel | Z3 | P3 | Lib-core | neu |
| SS-25 | Weitere Effekte | Spiegelung (Transparenz/Größe/Abstand/Unschärfe), Leuchten (Farbe/Transparenz/Größe), weiche Kanten, Skizzeneffekt (an/aus, Seed, Stärke, Linienstärke, Linien-/Füllungsabweichung) | Z2, Z3 | P3 | Lib-core | neu |
| SS-26 | Shape-ersetzen-Verhalten | ReplaceLockShapeData, ReplaceLockText, ReplaceLockFormat, ReplaceCopyCells: was beim „Shape ändern“ erhalten bleibt | Z2, Z3 | P2 | Lib-core | neu |
| SS-27 | Ausrichtung (alt) | AlignLeft/Center/Right/Top/Middle/Bottom: Klebebeziehungen zu Führungslinien | MS-VSDX | P3 | Lib-core | neu |
| SS-28 | Geometrie (mehrere Abschnitte) | je Abschnitt NoFill, NoLine, NoShow, NoSnap, NoQuickDrag; Zeilen MoveTo, LineTo, ArcTo, EllipticalArcTo, NURBSTo, PolylineTo, SplineStart/SplineKnot, InfiniteLine, Ellipse, RelMoveTo, RelLineTo, RelCubBezTo, RelQuadBezTo, RelEllipticalArcTo | Z2, Z3; MS-VSDX | P1 | Lib-core | neu |
| SS-29 | Verbindungspunkte | X, Y, DirX/DirY (Richtung), Type (nach innen/außen/beides), AutoGen, Prompt; Grundlage fürs Kleben | Z2, Z3 | P1 | Lib-core | neu |
| SS-30 | Steuerelemente (Controls) | gelbe Steuerpunkte: X, Y, XDyn/YDyn (Ankerpunkt der Rückmeldung), XCon/YCon (Verhalten beim Größe ändern), CanGlue, Tip (QuickInfo) | Z2, Z3 | P1 | Lib-core+Lib-ui | neu |
| SS-31 | Aktionen | eigene Kontextmenüeinträge: Action (Formel), Menu, Checked, Disabled, ReadOnly, Invisible, BeginGroup, ButtonFace, SortKey, TagName, FlyoutChild | Z2, Z3 | P2 | Lib-core+App | neu |
| SS-32 | Aktionskennzeichen (Smart Tags) | Schaltflächen am Shape mit Aktionsmenü: Position, Name, Beschreibung, Ausblenden | MS-VSDX | P3 | Lib-core+App | neu |
| SS-33 | Hyperlinks | Beschreibung, Adresse, Unteradresse (Seite/Shape), Zusatzinfo, Rahmen, neues Fenster, Standard-Link, unsichtbar, Sortierung | MS-VSDX | P2 | Lib-core | neu |
| SS-34 | Shape-Daten (Property) | Label, Prompt, Type (Text, feste Liste, Zahl, Boolesch, variable Liste, Datum, Dauer, Währung), Format, Value, SortKey, Invisible, Verify (beim Ablegen abfragen), DataLinked, Kalender, Sprache | Z2, Z3, Fav | P1 | Lib-core | neu |
| SS-35 | Benutzerdefinierte Zellen (User) | Name, Wert (Formel), Prompt; Träger fast aller Shape-Logik (Container `msv*`, Legenden `vis*`, Netzwerk-Farben) | Z2, Z3, Fav | P1 | Lib-core | neu |
| SS-36 | Scratch | Hilfszeilen A–D, X, Y für Zwischenrechnungen | Z2 | P2 | Lib-core | neu |
| SS-37 | Reviewer/Annotation | alte Kommentar- und Prüferabschnitte | MS-VSDX | P3 | Lib-core | neu |
| SS-38 | Ebenen (Seite) | Name, Name universell, sichtbar, drucken, aktiv, sperren, einrasten, kleben, Farbe + Transparenz, Status | Z2, Z3 | P1 | Lib-core | neu |
| SS-39 | Seiteneigenschaften | PageWidth/PageHeight, PageScale/DrawingScale, DrawingScaleType (Architektur/Ingenieur/metrisch/eigene), DrawingSizeType (wie Drucker, an Zeichnung anpassen, Standard, eigene), DrawingResizeType (automatisch vergrößern), InhibitSnap, Schatten-Versatz der Seite, UIVisibility | Z2, Z3 | P1 | Lib-core | neu |
| SS-40 | Seitenlayout | PlaceStyle, PlaceDepth, PlaceFlip, RouteStyle, LineJumpCode/LineJumpStyle, Linienübergang-Faktoren, Gassen- und Blockgrößen (AvenueSize, BlockSize), Abstände Linie-zu-Linie/Linie-zu-Knoten, LineAdjustFrom/To, ResizePage, EnableGrid (dynamisches Raster), CtrlAsInput, PageShapeSplit, DynamicsOff, LineRouteExt, PlowCode | Z2, Z3 | P1 | Lib-core | neu |
| SS-41 | Druckeigenschaften | Seitenränder, zentrieren X/Y, auf eine Seite anpassen (OnPage), Seiten X/Y (Kacheln), Raster drucken, Ausrichtung, Papierformat (PaperKind), Papierquelle, Skalierung X/Y | Z2, Z3 | P1 | Lib-core | neu |
| SS-42 | Lineal und Raster | Lineal- und Raster-Ursprung, -Dichte (fein/normal/grob/fest), Rasterabstand X/Y | Z2, Z3 | P2 | Lib-core | neu |
| SS-43 | Dokumenteigenschaften (Dokument-ShapeSheet) | OutputFormat, LockPreview, AddMarkup, ViewMarkup, DocLockReplace, NoCoauth, DocLockDuplicatePage, PreviewQuality, PreviewScope, DocLangID | Z2, Z3, Fav | P2 | Lib-core | neu |
| SS-44 | Formatvorlagen-Eigenschaften | EnableLineProps/EnableFillProps/EnableTextProps (welche Teile eine Formatvorlage setzt), HideForApply | Z2, Z3 | P2 | Lib-core | neu |
| SS-45 | Muster-Master | Master mit PatternFlags als eigene Linienmuster, Linienenden und Füllmuster (MS-11) | MS-VSDX; UG Kap. 5 | P3 | Lib-core | neu |
| SS-46 | ShapeSheet-Fenster | Ansicht und Bearbeitung aller Abschnitte eines Shapes, einer Seite oder des Dokuments mit Formelzeile; Abschnitt/Zeile einfügen und löschen; Werte oder Formeln anzeigen; Vorgänger/Nachfolger einer Zelle anzeigen; Funktion einfügen (Auswahldialog) | SbS A; A 183847, A 190010; UG | P2 | App+Lib-ui | neu |

### 2.14 Formel-Engine und Funktionen

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| FX-01 | Formel-Parser | Visio-Formelsyntax: Zahlen mit Einheit (`10 mm`, `45 deg`), Zeichenketten, Operatoren `+ - * / ^ & = <> < > <= >= %`, Klammern, Funktionsaufrufe, Zellbezüge lokal (`Width`), in Abschnitten (`Geometry1.X1`, `Prop.Name`, `User.Farbe`, `Connections.X1`, `Controls.Row_1`), auf andere Shapes (`Sheet.5!Width`), auf Seite/Dokument (`ThePage!PageScale`, `TheDoc!…`), Fehlerwerte (`#REF!`, `#VALUE!`, `#DIV/0!`, `#NAME?`, `#N/A`). Lokalisierte und universelle Namen. | Z2, Z3; MS-VSDX | P1 | Lib-core | neu |
| FX-02 | Auswertung und Neuberechnung | Abhängigkeitsgraph über Zellen, Shapes, Seite und Dokument; nur betroffene Zellen neu rechnen; Zyklen erkennen und begrenzen; typisierte Ergebnisse (Zahl mit Einheit, Text, Bool, Farbe, Datum, Fehler); `V` beim Speichern aktuell halten. | MS-VSDX | P1 | Lib-core | neu |
| FX-03 | Vererbung | Zelle lokal → Master-Shape → Formatvorlage → Standard; lokale Formel überschreibt; „Formel löschen“ stellt Vererbung wieder her; nur Abweichungen speichern. | Z2, Z3 | P1 | Lib-core | neu |
| FX-04 | Schutzmechanik | `GUARD` (Formel bleibt bei Benutzeraktionen), `THEMEGUARD`, `SETATREF`/`SETATREFEXPR`/`SETATREFEVAL` (Benutzeränderung wird in eine andere Zelle umgeleitet), `SETF` (Formel aus einer Aktion setzen), `DEPENDSON`, Auslöser-Zellen (`XFTRIGGER`, BegTrigger/EndTrigger). | Z2, Z3 | P1 | Lib-core | neu |
| FX-05 | Einheiten | Umrechnung zwischen Seiten- und Zeichnungseinheiten (Maßstab), Anzeige in mm/cm/m/km/in/ft/yd/mi/pt/pica/Cicero/Didot, Winkel deg/rad/min/sec, Dauer und Datum. | Z2, Z3 | P1 | Lib-core | neu |
| FX-06 | Mathe und Logik | ABS, AND, OR, NOT, IF, MIN, MAX, SUM, SQRT, POW, EXP, LN, LOG10, SIN, COS, TAN, ASIN, ACOS, ATAN, ATAN2, SINH, COSH, TANH, PI, ROUND, INT, INTUP, TRUNC, FLOOR, CEILING, MODULUS, SIGN, MAGNITUDE, ANG360, RAD, DEG, BITAND, BITOR, BITXOR, BITNOT, INDEX, LOOKUP, RAND, TRUE, FALSE | Z2, Z3; MS-VSDX | P1 | Lib-core | neu |
| FX-07 | Geometrie | PNT, PNTX, PNTY, PAR, LOC, NURBS, POLYLINE, BOUND, ANGLETOLOC, ANGLETOPAR, LOCTOLOC, LOCTOPAR (P1); PATHLENGTH, POINTALONGPATH, ANGLEALONGPATH, DISTTOPATH, NEARESTPOINTONPATH, INTERSECTX, INTERSECTY, SEGMENTCOUNT, GRAVITY (P2) | Z2, Z3; MS-VSDX | P1/P2 | Lib-core | neu |
| FX-08 | Farbe und Theme | RGB, HSL, RED, GREEN, BLUE, HUE, SAT, LUM, HUEDIFF, SATDIFF, LUMDIFF, TINT, SHADE, TONE, BLEND, MSOTINT, MSOSHADE, THEMEVAL, THEMEPROP, THEMERESTORE, ISTHEMED, CELLISTHEMED | Z2, Z3 | P1 | Lib-core | neu |
| FX-09 | Text | SHAPETEXT, TEXTWIDTH, TEXTHEIGHT, FONT, STRSAME (P1); STRSAMEEX, LEN, LEFT, RIGHT, MID, FIND, SUBSTITUTE, REPLACE, REPT, LOWER, UPPER, TRIM, CHAR, FORMAT, FORMATEX, EVALTEXT (P2) | Z2, Z3; MS-VSDX | P1/P2 | Lib-core+Lib-ui | neu |
| FX-10 | Datum und Zeit | NOW, DATE, TIME, DATETIME, DATEVALUE, TIMEVALUE, DAY, MONTH, YEAR, HOUR, MINUTE, SECOND, WEEKDAY, DAYOFYEAR | MS-VSDX | P2 | Lib-core | neu |
| FX-11 | Dokument- und Shape-Info | MASTERNAME, PAGENAME, PAGENUMBER, PAGECOUNT, DOCCREATION, DOCLASTSAVE, DOCLASTPRINT, DOCLASTEDIT, ID, NAME, TYPE, TYPEDESC, DATA1/DATA2/DATA3, USERUI, IS1D, REF, ARG, ISERR, ISERROR, ISERRNA, ISERRVALUE, DEFAULTEVENT | Z2, Z3; MS-VSDX | P2 | Lib-core | neu |
| FX-12 | Struktur | CONTAINERCOUNT, CONTAINERSHEETREF, CONTAINERMEMBERCOUNT, LISTMEMBERCOUNT, LISTORDER, LISTSHEETREF, CALLOUTCOUNT, CALLOUTTARGETREF, HASCATEGORY, WALKGLUE | Z2, Z3; MS-VSDX | P2 | Lib-core | neu |
| FX-13 | Validierung | Funktionen der Regel-Ausdrücke der Diagrammprüfung (z. B. AGGCOUNT, FILTERSET, GLUEDSHAPES, CONNECTEDSHAPES, ONLAYER, ONBOUNDARYOF) | MS-VSDX | P3 | Lib-core | neu |
| FX-14 | Aktionen ohne VBA | OPENTEXTWIN, OPENSHEETWIN, OPENGROUPWIN, GOTOPAGE, HYPERLINK, PLAYSOUND, SETF, DOCMD (auf eigene Befehle abgebildet), RUNADDON nur für bekannte eingebaute Funktionen (z. B. Shape-Daten-Dialog), sonst ignorieren und melden | Z2, Z3 | P2 | Lib-core+App | neu |
| FX-15 | Nicht ausgeführt | RUNMACRO, CALLTHIS, RUNADDON/RUNADDONWARGS mit fremden Add-ons, Aufruf externer Programme: Formel bleibt erhalten, Wirkung entfällt (Abschnitt 6) | Entscheidung | – | Lib-core | – |
| FX-16 | Formel-Editor | Formeln in ShapeSheet, Shape-Daten und Feldern bearbeiten: Syntaxprüfung, Autovervollständigen von Zellen und Funktionen, Fehleranzeige | W | P2 | App+Lib-ui | neu |

<a id="oberflaeche"></a>
## 3. Oberfläche

Aufbau wie Visio: Titelleiste mit Schnellzugriff, Registerzeile mit Befehlssuche, Menüband,
links die Shapes-Seitenleiste, Mitte das Zeichenfenster mit Linealen und Seitenregistern,
rechts andockbare Aufgabenbereiche, unten die Statusleiste. Look: **NetLab-Style, eckig**,
nicht JFOffice. Befehlsnamen folgen der deutschen Visio-Oberfläche, wo sie beschreibend sind;
eigene Namen sind erlaubt, wo Visio Marken- oder Dienstnamen nutzt.

Typen in den Befehlstabellen: **K** Knopf, **SK** Split-Knopf, **DD** Dropdown, **Kat**
Katalog/Galerie, **KK** Kontrollkästchen, **Tog** Umschaltknopf, **Combo** Eingabe mit Liste,
**DS** Dialogstarter der Gruppe. „groß“/„klein“ = Icon-Größe. Ort ist bei Befehlen immer `App`
(die Befehlslogik) plus der genannte Baustein; Bestand bezieht sich auf die Funktion dahinter.

### 3.1 Fenster, Start und Backstage

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| UI-01 | Hauptfenster | Titelleiste (Dateiname, Schnellzugriff, Fensterknöpfe), Registerzeile, Menüband, Shapes-Seitenleiste, Zeichenfenster, Aufgabenbereiche, Statusleiste; mehrere Zeichnungen in einer Programminstanz, Fenster wechseln. | A 183536; SbS 1 | P1 | App+Lib-ui | teilweise |
| UI-02 | NetLab-Look | Eckige Kanten, NetLab-Farben und -Schriften für Menüband, Seitenleiste, Bereiche und Dialoge; hell/dunkel; hohe DPI. | Entscheidung; SbS 1 | P1 | Lib-style+App | teilweise |
| UI-03 | Startbildschirm | Beim Programmstart: Startseite/Neu/Öffnen links, Begrüßung, Reihe empfohlener Vorlagen (Standarddiagramm, Leere Zeichnung, Netzwerk, Flussdiagramm …), „Weitere Vorlagen“, Liste „Zuletzt verwendet“/„Angeheftet“ mit Name, Pfad und Änderungsdatum, „Weitere Zeichnungen“; abschaltbar. | A 183424, A 08-213513; SbS 1 | P1 | App+Lib-ui | teilweise (jfWord, recent.rs) |
| UI-04 | Vorlagen-Vorschau | Klick auf eine Vorlage zeigt Vorschau, Beschreibung, Wahl **Metrische Einheiten**/**US-Einheiten**, Beispiel-Startdiagramme und **Erstellen**; Doppelklick erstellt sofort. | A 190353; SbS 1 | P1 | App | neu |
| UI-05 | Neu | Vorlagen nach Kategorien (Abschnitt 5), Suche in den eigenen Vorlagen, „Neu aus vorhandener Zeichnung“, eigene Vorlagenordner als eigene Kategorie. | SbS 1, 12; TP | P1 | App | neu |
| UI-06 | Öffnen | Zuletzt verwendet, Orte, Durchsuchen; Dateitypfilter für Zeichnungen, Schablonen, Vorlagen, XML- und Binärformate, SVG, DXF; öffnen als Original, schreibgeschützt oder als Kopie. | A 190527; SbS 1; TP; UG Kap. 14 | P1 | App+Lib-ui | teilweise (filedlg) |
| UI-07 | Informationen | Pfad, Eigenschaften (Titel, Stichwörter, Kategorien, Autor, Firma, Kommentare, erweiterte Eigenschaften mit Hyperlinkbasis), Dateigröße verringern, persönliche Informationen entfernen, Kompatibilität, Schutz. | SbS 1, 11, 12 | P2 | App | neu |
| UI-08 | Speichern / Speichern unter | Strg+S; Speichern unter mit Dateityp (Abschnitt 1.4), Vorlage/Schablone, zuletzt genutzte Ordner. | SbS 1, 12; TP | P1 | App | teilweise |
| UI-09 | Drucken (Backstage) | Siehe 3.15. | A 08-213513; SbS 12; TP | P1 | App | teilweise |
| UI-10 | Freigeben | Ohne Microsoft-Clouddienste: „Kopie per E-Mail senden“ als Zeichnung oder PDF über das Mailprogramm des Systems. | A 184030; SbS 1; TP | P3 | App+Lib-plat | neu |
| UI-11 | Exportieren | PDF/XPS erstellen, Dateityp ändern (Zeichnung, Vorlage, Schablone, Grafik), Webseite. | SbS 1, 12 | P1 | App | neu |
| UI-12 | Schließen, Info | Zeichnung schließen (mit Speichern-Rückfrage); Info-Seite mit Version, Lizenzen und AWS-Namensnennung statt „Konto“. | A 08-213513; SbS 1 | P1 | App | teilweise |
| UI-13 | Optionen – Allgemein | Minisymbolleiste, Livevorschau (Menüband und Shapes-Seitenleiste), QuickInfo-Stil, Benutzername und Initialen (für Kommentare), Hell/Dunkel, Startbildschirm. | A 184427, A 08-214158; SbS 1 | P1 | App | teilweise |
| UI-14 | Optionen – Dokumentprüfung | AutoKorrektur, Rechtschreibung während der Eingabe, Ausnahmen (Großbuchstaben, Zahlen, Adressen), Wörterbücher. | A 184444; SbS 1 | P2 | App+Lib-ui | teilweise (spell) |
| UI-15 | Optionen – Speichern | Standardformat, AutoWiederherstellen-Intervall, Standardorte für Zeichnungen und persönliche Vorlagen. | A 184500; SbS 1 | P1 | App | neu |
| UI-16 | Optionen – Sprache | Anzeige- und Bearbeitungssprachen mit Reihenfolge; Oberflächensprache DE/EN/ES/IT/FR ohne Neustart (LG-02), Region für Einheiten und Formate (LG-07). | A 184517; SbS 1; Nachtrag 2026-11-14 | P1 | App+Lib-ui | teilweise (i18n) |
| UI-17 | Optionen – Barrierefreiheit | QuickInfo-Format, Tastenkürzel in QuickInfos. | A 184532 | P2 | App | neu |
| UI-18 | Optionen – Erweitert | Livedynamik, Verbinder teilen, Verbinder mit löschen, AutoVerbinden, Übergänge, Rad zoomt, Auswahl beim Zoomen zentrieren, Teilauswahl im Rahmen, mehr Größengriffe beim Zeigen, Formel-Autovervollständigen, Enter beendet Text, Rückgängig-Tiefe, Text-Zoom beim Bearbeiten; Freihand-Genauigkeit/Glättung; Standardeinheiten (Seite, Text, Winkel, Dauer); Seitenansicht drehen erlauben; Speichern/Öffnen-Warnungen, Visio-Daten in SVG; Shape-Suche (alle/irgendein Wort); Entwicklermodus; Dateispeicherorte (Zeichnungen, Vorlagen, Schablonen). | A 184549, A 184605, A 184620; SbS 1, 12; UG Kap. 3, 18 | P1/P2 | App | neu |
| UI-19 | Optionen – Menüband und Schnellzugriff anpassen | Register und Gruppen ein-/ausblenden, eigene Gruppen; Schnellzugriff mit beliebigen Befehlen, Reihenfolge, unter dem Menüband, Import/Export. | A 184640, A 184700; SbS 1, A | P2 (Schnellzugriff), P3 (Menüband) | App+Lib-ui | teilweise |
| UI-20 | Optionen – Erweiterungen | Statt COM/VBA: Liste eigener Erweiterungen (später), aktiv/inaktiv. | A 184718 | P3 | App | neu |
| UI-21 | Skript- und Erweiterungsschnittstelle | Automatisierung und Erweiterungen über JFScript mit dem Ereignismodell aus RX-10 (Auswahl, Zellen, Shapes, Seiten, Dokument); ersetzt VBA und COM-Add-ins. **Später, nicht in Phase 2** (F7). | Entscheidung F7; A 185942 | P3 | App+JFScript | neu |

### 3.2 Titelleiste, Registerzeile und Menüband allgemein

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| UI-30 | Schnellzugriff | Speichern, Rückgängig/Wiederholen mit Namen der Aktion („Rückgängig: Dynamisches Gitter“), anpassbar. | A 184700; SbS 1 | P1 | App+Lib-ui | teilweise |
| UI-31 | Registerzeile | Datei, Start, Einfügen, Zeichnen, Entwurf, Daten, Prozess, Überprüfen, Ansicht, Entwicklertools (nur im Entwicklermodus), Hilfe; aktives Register hervorgehoben. | A 183536; SbS 1 | P1 | App+Lib-ui | teilweise |
| UI-32 | Befehlssuche „Was möchten Sie tun?“ | Feld in der Registerzeile: leer = zuletzt verwendete und vorgeschlagene Aktionen; Tippen filtert alle Befehle (mit Synonymen), Ausführen direkt aus der Liste, nicht anwendbare ausgegraut. | A 185646; SbS 1 | P2 | App+Lib-ui | teilweise (cmd.rs) |
| UI-33 | Kommentare-Knopf | Öffnet den Kommentarbereich. | A 183536 | P2 | App | neu |
| UI-34 | Menüband-Verhalten | Gruppen mit Dialogstarter, große/kleine Knöpfe, Split-Knöpfe, Kataloge mit Scrollpfeilen und „Mehr“, QuickInfos mit Beschreibung und Kürzel; schmale Fenster klappen Gruppen zu einem Knopf; Menüband reduzieren, automatisch ausblenden, nur Register; Touch-Modus mit größeren Knöpfen. | A 183536; SbS Vorwort, 1 | P1 | Lib-ui | teilweise |
| UI-35 | Livevorschau | Kataloge (Designs, Varianten, Schnellformatvorlagen, Effekte, Ausrichten, Layout, Container, Legenden) zeigen die Wirkung beim Zeigen ohne Rückgängig-Schritt; abschaltbar. | SbS 6; A 184427 | P2 | Lib-ui+App | teilweise |
| UI-36 | Kontextregister | Erscheinen nur bei passender Auswahl rechts der festen Register mit farbigem Kopf (Container, Bild, Organigramm, Swimlanes, Brainstorming, Zeitachse, Kalender, Schablone bearbeiten, Symbol-Editor). | SbS 1, 4, 5; A 184640 | P1 | Lib-ui+App | neu |
| UI-37 | Zugriffstasten | Alt zeigt Buchstaben an Registern und Befehlen; Menüband per Tastatur bedienbar. | SbS B | P1 | Lib-ui | teilweise |

### 3.3 Register „Start“

| ID | Gruppe | Befehl | Typ | Beschreibung | Quelle | Prio | Bestand |
|---|---|---|---|---|---|---|---|
| RS-01 | Zwischenablage | Einfügen | SK groß | Einfügen; Inhalte einfügen (Format wählen); Als Link einfügen; Duplizieren | A 184131; SbS 2 | P1 | teilweise |
| RS-02 | Zwischenablage | Ausschneiden / Kopieren | K klein | Strg+X / Strg+C | A 183536 | P1 | teilweise |
| RS-03 | Zwischenablage | Format übertragen | K klein | einmal (Klick) oder mehrfach (Doppelklick) | A 183536; SbS 6 | P1 | neu |
| RS-04 | Schriftart | Schriftart, Schriftgrad | Combo | mit Livevorschau, Theme-Schriften zuerst | A 183536 | P1 | teilweise (fontpick) |
| RS-05 | Schriftart | Schriftgrad vergrößern/verkleinern | K klein | stufenweise | A 183536 | P2 | neu |
| RS-06 | Schriftart | Fett, Kursiv, Unterstrichen, Durchgestrichen | Tog klein | Zeichenformat | A 183536 | P1/P2 | teilweise |
| RS-07 | Schriftart | Groß-/Kleinschreibung | DD klein | Normal, Großbuchstaben, Kapitälchen | A 183536 | P3 | neu |
| RS-08 | Schriftart | Schriftfarbe | SK klein | Farbwähler (Designfarben mit Abstufungen, Variantenfarben, Standardfarben, zuletzt verwendet, weitere Farben) | A 184050; SbS 6 | P1 | teilweise |
| RS-09 | Schriftart | DS | – | Dialog „Text“ (Schriftart, Zeichen, Absatz, Textblock, Tabstopps, Aufzählungszeichen) | SbS 4 | P1 | neu |
| RS-10 | Absatz | Oben/Mittig/Unten ausrichten | Tog klein | vertikale Lage im Textblock | A 183536; SbS 3 | P1 | neu |
| RS-11 | Absatz | Links/Zentriert/Rechts/Blocksatz | Tog klein | horizontale Ausrichtung | A 183536; SbS 3 | P1 | neu |
| RS-12 | Absatz | Einzug verkleinern/vergrößern, Aufzählungszeichen, Text drehen | K/Tog klein | Einzüge, Aufzählung, Textblock um 90° | A 183536 | P2 | neu |
| RS-13 | Tools | Zeigertool | Tog klein | Auswählen, Verschieben, Größe (Strg+1) | A 183536; SbS 2 | P1 | vorhanden (DG_SELECT) |
| RS-14 | Tools | Verbinder | Tog klein | Verbinderwerkzeug bleibt aktiv (Strg+3) | A 183536; SbS 2 | P1 | teilweise (DG_CONNECT) |
| RS-15 | Tools | Text | Tog klein | Textfeld aufziehen oder Text in Shape (Strg+2) | A 183536; SbS 3 | P1 | neu |
| RS-16 | Tools | Zeichentools | SK klein | Rechteck, Ellipse, Linie, Freihandform, Bogen, Bleistift; Umschalt = Quadrat/Kreis | A 184113; SbS 2 | P1/P2 | neu |
| RS-17 | Tools | Verbindungspunkt | Tog klein | Verbindungspunkte hinzufügen (Strg+Klick), verschieben, löschen | A 183536 | P2 | neu |
| RS-18 | Tools | Textblock | Tog klein | Textblock getrennt verschieben, drehen, skalieren | A 183536; SbS 3 | P2 | neu |
| RS-19 | Formenarten | Schnellformatvorlagen | Kat groß | Variantenstile + Matrix 6 × 7, „Designs zulassen“ | A 183536; SbS 6 | P2 | neu |
| RS-20 | Formenarten | Füllung | DD klein | Farbwähler, Keine Füllung, Füllungsoptionen | A 183536; SbS 6 | P1 | neu |
| RS-21 | Formenarten | Linie | DD klein | Farbe, Keine Linie, Stärke (¼ bis 6 pt + weitere), Striche, Pfeile (13 Schnellwahlen + weitere), Linienoptionen | A 184201, A 184228; SbS 6 | P1 | teilweise |
| RS-22 | Formenarten | Effekte | DD klein | Schatten, Spiegelung, Leuchten, weiche Kanten, Abschrägung, 3D-Drehung mit Voreinstellungen | A 183536; SbS 6 | P2/P3 | neu |
| RS-23 | Formenarten | DS | – | Aufgabenbereich „Shape formatieren“ | SbS 6 | P1 | neu |
| RS-24 | Anordnen | Ausrichten | DD groß | Automatisch ausrichten; links/zentriert/rechts; oben/mittig/unten (zum Anker-Shape) | A 183536; SbS 6 | P1 | neu |
| RS-25 | Anordnen | Positionieren | DD groß | Automatischer Abstand, Automatisch ausrichten und Abstände, Abstandsoptionen; horizontal/vertikal verteilen, weitere Verteilungsoptionen; von Seitenumbrüchen verschieben, Seitenumbrüche vermeiden; Shapes drehen ▸, Diagramm drehen ▸ | A 184310; SbS 2, 6, 12 | P1/P2 | neu |
| RS-26 | Anordnen | In den Vordergrund / In den Hintergrund | SK klein | ganz bzw. eine Ebene nach vorn/hinten | A 183536; SbS 6 | P1 | teilweise |
| RS-27 | Anordnen | Gruppieren | DD klein | Gruppieren, Gruppierung aufheben, zu Gruppe hinzufügen, aus Gruppe entfernen | A 183536; SbS 3 | P1 | neu |
| RS-28 | Bearbeiten | Shape ändern | Kat groß | Ersetzen durch Master der offenen Schablonen | A 183536; SbS 3 | P2 | neu |
| RS-29 | Bearbeiten | Suchen | DD klein | Suchen (Strg+F), Ersetzen (Strg+H) | A 184334 | P1 | neu |
| RS-30 | Bearbeiten | Ebenen (Visio: „Layer“) | DD klein | Ebene zuweisen, Ebeneneigenschaften | A 184351; SbS 3 | P1 | neu |
| RS-31 | Bearbeiten | Markieren | DD klein | Alles markieren, Nach Typ auswählen (Shapes/Gruppen/Führungslinien/OLE/Freihand, Ebene, Formatvorlage), Bereich auswählen, Lassoauswahl | A 184405; SbS 2, 3 | P1/P2 | teilweise |

### 3.4 Register „Einfügen“

| ID | Gruppe | Befehl | Typ | Beschreibung | Quelle | Prio | Bestand |
|---|---|---|---|---|---|---|---|
| RE-01 | Seiten | Neue Seite (Visio: „Neues Zeichenblatt“) | SK groß | Leere Seite, Hintergrundseite, Diese Seite duplizieren (Umschalt+F11) | A 184742; SbS 3; TP | P1 | neu |
| RE-02 | Illustrationen | Bilder | DD groß | Bild aus Datei (Archiv- und Onlinebilder entfallen, NO-17) | A 184758; SbS 3 | P1 | teilweise |
| RE-03 | Illustrationen | Piktogramme | K groß | Sammlung einfacher Symbole einfügen – **eigene** Symbolsammlung (z. B. aus Fluent-Icons der Bibliothek, Lizenz prüfen) | A 183603 | P3 | teilweise (style_assets) |
| RE-04 | Illustrationen | Diagramm | K groß | Visio bettet ein Excel-Diagramm ein; JFDiagrammer: eigenes Diagramm-Shape (Balken, Linie, Torte) aus Shape-Daten oder CSV | A 183603; TP | P3 | teilweise (chart.rs) |
| RE-05 | Illustrationen | CAD-Zeichnung | K groß | DXF einfügen (IM-04) | A 183603 | P3 | neu |
| RE-06 | Diagrammteile | Container | Kat groß | 14 Container-Stile, um Auswahl legen oder leer einfügen | A 184817; SbS 13 | P1 | teilweise |
| RE-07 | Diagrammteile | Beschriftung (Visio: „Legende“) | Kat groß | 14 Beschriftungsstile an die Auswahl hängen (Linie, Klammer, Unterstrich, Klammer mit Zeiger, Rechteck mit Zeiger, eckige Sprechblase, Keil, Ellipse mit Zeiger, Kreis, Gedankenwolke, ovale Sprechblase, Tropfen, Zickzack, schräges Etikett) – **Formen neu zeichnen** | A 184834; SbS 13 | P1 | neu |
| RE-08 | Diagrammteile | Verbinder | K groß | einen Verbinder einfügen bzw. Werkzeug | A 183603 | P1 | teilweise |
| RE-09 | Links | Link | K groß | Hyperlink-Dialog (Strg+K) | A 183603; SbS 11 | P1 | neu |
| RE-10 | Text | Textfeld | SK groß | horizontales/vertikales Textfeld | A 184850; SbS 3 | P1 | neu |
| RE-11 | Text | QuickInfo | K groß | QuickInfo des Shapes bearbeiten | A 184911; SbS 3 | P2 | neu |
| RE-12 | Text | Objekt | K groß | OLE-Objekt (nur Erhalt, NO-04); stattdessen Datei als Anlage-Shape mit Symbol | A 183603 | P3 | neu |
| RE-13 | Text | Feld | K groß | Feld-Dialog (TX-06) | A 184911; SbS 8 | P1 | neu |
| RE-14 | Text | Symbol | DD groß | 20 häufige Sonderzeichen + „Weitere Symbole“ (Zeichentabelle) | A 184911 | P2 | teilweise (symbol.rs) |

### 3.5 Register „Zeichnen“ (Freihand)

| ID | Gruppe | Befehl | Typ | Beschreibung | Quelle | Prio | Bestand |
|---|---|---|---|---|---|---|---|
| RZ-01 | Tools | Auswählen, Lassoauswahl, Pinselstrichradierer, Mit Finger zeichnen | Tog groß | Freihand-Modus steuern | A 183624 | P3 | teilweise (touch.rs) |
| RZ-02 | Stifte | Stiftkatalog, Farbe, Linienstärke | Kat/DD | Stifte und Textmarker in mehreren Farben | A 183624 | P3 | neu |
| RZ-03 | Konvertieren | Freihand in Text / in Shapes | K groß | Handschrift bzw. Skizzen in Text oder Grundformen umwandeln | A 183624 | P3 | neu |

### 3.6 Register „Entwurf“

| ID | Gruppe | Befehl | Typ | Beschreibung | Quelle | Prio | Bestand |
|---|---|---|---|---|---|---|---|
| RW-01 | Seite einrichten | Ausrichtung | DD groß | Hoch-/Querformat | A 185238; SbS 3 | P1 | neu |
| RW-02 | Seite einrichten | Format | DD groß | Papierformate mit Maßen (Letter, Legal, A3–A5, B4/B5, Umschläge, Behördenformate …), „Größe an Zeichnung anpassen“, „Weitere Seitenformate“ | A 185254, A 185314; SbS 3 | P1 | neu |
| RW-03 | Seite einrichten | Automatisch anpassen | Tog groß | Auto-Größe der Seite (PG-05) | A 183642; SbS 3 | P1 | neu |
| RW-04 | Seite einrichten | DS | – | Dialog „Seite einrichten“ (3.15) | A 183642; SbS 3 | P1 | neu |
| RW-05 | Designs | Designkatalog | Kat | Gruppen Professionell, Modern, Kräftig, Handgezeichnet + „Kein Design“; „Design auf neue Shapes anwenden“; Rechtsklick: auf alle Seiten/aktuelle Seite. **Eigene Designs** (TH-03) | A 183642, A 183706; SbS 6 | P1 | neu |
| RW-06 | Varianten | Variantenkatalog + Menü | Kat | 4 Varianten; Farben ▸ (neue Designfarben), Effekte ▸, Verbinder ▸, Schriftarten ▸, Verzierung ▸ | A 183642; SbS 6 | P1/P2 | neu |
| RW-07 | Hintergründe | Hintergründe | DD groß | Hintergrundmotive + Hintergrundfarbe (PG-09) | A 183642; SbS 3 | P2 | neu |
| RW-08 | Hintergründe | Rahmen und Titel | DD groß | Rahmen mit Titelblock (PG-10) | A 183642; SbS 3 | P2 | neu |
| RW-09 | Layout | Seite neu layouten (Visio: „Zeichenblattlayout neu anordnen“) | DD groß | Flussdiagramm (4 Richtungen), Hierarchie, kompakte Struktur, radial, kreisförmig; „Weitere Layoutoptionen“ (Abstände, Verbinderstil, Routing auf Verbinder anwenden) | A 183642; TP | P2 | neu |
| RW-10 | Layout | Verbinder | DD groß | rechtwinklig/gerade/gebogen, Linienübergänge anzeigen und Stil, neu routen | A 183642; SbS 2 | P1 | teilweise |
| RW-11 | Layout | DS | – | „Seite einrichten → Layout und Routing“ | A 183642 | P1 | neu |

### 3.7 Register „Daten“

| ID | Gruppe | Befehl | Typ | Beschreibung | Quelle | Prio | Bestand |
|---|---|---|---|---|---|---|---|
| RD-01 | Externe Daten | Schnellimport | K groß | Excel/CSV in einem Schritt verknüpfen (DA-06) | A 183728; SbS 10 | P3 | neu |
| RD-02 | Externe Daten | Benutzerdefinierter Import | K groß | Datenauswahl-Assistent: CSV und `.xlsx` (P2), ODBC/SQL (P3, F6); Bereich, Spalten, Schlüssel (DA-04) | A 183728; SbS 10 | P2 | neu |
| RD-03 | Externe Daten | Alle aktualisieren | SK groß | Alle aktualisieren; Daten aktualisieren (je Quelle, Intervall); nach Aktualisierung automatisch verknüpfen | A 185335; SbS 10 | P2 | neu |
| RD-04 | Datengrafiken | Datengrafik-Katalog | Kat | Keine, Text, Datenbalken, Symbolsätze, Farbe nach Wert | A 183728; SbS 9, 10 | P2 | neu |
| RD-05 | Datengrafiken | Position, Konfigurieren | DD/K klein | Lage am Shape; Element bearbeiten | A 183728; SbS 10 | P2 | neu |
| RD-06 | Datengrafiken | Automatischer Abstand | K klein | Shapes mit Datengrafiken neu verteilen | A 183728 | P3 | neu |
| RD-07 | Daten anzeigen | Legende einfügen | DD groß | horizontal/vertikal (ST-08) | A 185355; SbS 9 | P2 | neu |
| RD-08 | Einblenden/Ausblenden | Fenster „Shape-Daten“, „Externe Daten“, „Datengrafikfelder“ | KK | Aufgabenbereiche ein/aus | A 183728; SbS 8, 10 | P1/P2 | neu |
| RD-09 | Erweiterte Datenverknüpfung | Daten verknüpfen | K klein | Assistent Spalte ↔ Shape-Feld (DA-04) | A 183728; SbS 10 | P2 | neu |
| RD-10 | Erweiterte Datenverknüpfung | Erweiterte Datengrafiken | DD klein | Katalog mit Neu, Bearbeiten, Umbenennen, Duplizieren, Löschen, „Shapes mit dieser Grafik auswählen“ | A 183728; SbS 9 | P2 | neu |

### 3.8 Register „Prozess“

| ID | Gruppe | Befehl | Typ | Beschreibung | Quelle | Prio | Bestand |
|---|---|---|---|---|---|---|---|
| RP-01 | Teilprozess | Neu erstellen | K groß | neue Seite nach Shape-Text benennen und vom Shape verlinken | A 183751; SbS 4 | P2 | neu |
| RP-02 | Teilprozess | Mit vorhandenem verknüpfen | DD groß | Seitenliste, anderes Dokument, Verknüpfung bearbeiten | A 185411; SbS 4 | P2 | neu |
| RP-03 | Teilprozess | Aus Auswahl erstellen | K groß | Auswahl auf neue Seite verschieben, durch Teilprozess-Shape ersetzen, Swimlanes nachbauen | A 183751; SbS 4 | P3 | neu |
| RP-04 | Diagrammüberprüfung | Diagramm überprüfen | SK groß | Prüfen; Zu überprüfende Regeln ▸; Regeln importieren aus ▸ (Flussdiagramm, BPMN 2.0) (DA-10) | A 185429, A 185446; SbS 14 | P2 | neu |
| RP-05 | Diagrammüberprüfung | Dieses Problem ignorieren | SK klein | Problem/Regel ignorieren und wieder beachten, ignorierte anzeigen | A 183751; SbS 14 | P2 | neu |
| RP-06 | Diagrammüberprüfung | Problemfenster | KK | Problemliste ein/aus | A 183751; SbS 14 | P2 | neu |
| RP-07 | (eigene) | Regeln bearbeiten | K | eigener Regel-Editor (DA-10), ersetzt die SharePoint-Workflow-Gruppe | SbS 14 | P3 | neu |

### 3.9 Register „Überprüfen“, „Ansicht“, „Entwicklertools“, „Hilfe“

| ID | Register / Gruppe | Befehl | Typ | Beschreibung | Quelle | Prio | Bestand |
|---|---|---|---|---|---|---|---|
| RU-01 | Überprüfen / Rechtschreibung | Rechtschreibung (F7) | K groß | Prüfdialog über alle Shapes und Seiten | A 183811; SbS 1 | P2 | teilweise (spell) |
| RU-02 | Überprüfen / Rechtschreibung | Thesaurus | K groß | Synonyme (offline, wenn Wörterbuch vorhanden); Recherchieren entfällt | A 183811 | P3 | neu |
| RU-03 | Überprüfen / Barrierefreiheit | Barrierefreiheit überprüfen | K groß | Alternativtexte, Seitennamen, Kontrast, Lesereihenfolge (`visAltText`, `visNavOrder`) | A 183811; Z2, Z3 | P2 | teilweise (a11y) |
| RU-04 | Überprüfen / Sprache | Sprache | DD groß | Korrektursprache für Auswahl/Text; Übersetzen entfällt (Onlinedienst) | A 185534 | P2 | teilweise |
| RU-05 | Überprüfen / Kommentare | Neuer Kommentar, Kommentarbereich (Tags anzeigen) | K/SK groß | HL-04 | A 183811, A 185607; SbS 3 | P2 | neu |
| RU-06 | Überprüfen / Berichte | Shape-Berichte | K groß | Dialog Berichte (DA-09) | A 185624; SbS 8 | P2 | neu |
| RU-07 | Überprüfen / Schutz | Dokument schützen | K | DM-11 (in neueren Visio-Versionen unter Entwicklertools) | W | P3 | neu |
| RA-01 | Ansicht / Ansichten | Präsentationsmodus | K groß | Vollbild (F5), Seiten blättern | A 183830; SbS 1 | P2 | neu |
| RA-02 | Ansicht / Anzeigen | Lineal, Seitenumbruch, Gitternetz, Führungslinien | KK | Anzeige ein/aus (Raster standardmäßig aus) | A 183830, A 183959; SbS 1, 2 | P1 | teilweise |
| RA-03 | Ansicht / Anzeigen | Aufgabenbereiche | DD groß | Shapes, Shape-Daten, Größe und Position, Zoom und Verschieben, Shape formatieren, Zeichnungsexplorer, Ebenen, Kommentare, Probleme | A 183830; SbS 1; TP | P1 | teilweise (dock) |
| RA-04 | Ansicht / Anzeigen | DS | – | Dialog „Lineal und Gitternetz“ (Unterteilung, Abstand, Ursprung) | A 183830; TP | P2 | neu |
| RA-05 | Ansicht / Zoom | Zoom, An Fenster anpassen, Seitenbreite | K klein | Zoom-Dialog; Seite einpassen (Strg+Umschalt+W); Breite einpassen | A 183830; SbS 1 | P1 | teilweise |
| RA-06 | Ansicht / Visuelle Unterstützung | Dynamisches Gitter, AutoVerbinden, Verbindungspunkte | KK | Hilfen ein/aus | A 183830; SbS 2 | P1 | neu |
| RA-07 | Ansicht / Visuelle Unterstützung | DS | – | Dialog „Ausrichten und Kleben“ (DM-03) | A 183830; UG | P1 | neu |
| RA-08 | Ansicht / Fenster | Neues Fenster, Alle anordnen, Überlappend, Fenster wechseln | K/DD groß | zweite Ansicht derselben Zeichnung, Anordnen | A 183830; SbS 1 | P2/P3 | teilweise |
| RA-09 | Ansicht / Makros | Add-Ons → eigene Werkzeuge | DD groß | Ersatz für Visios Add-ons ohne Code: Shapes in Reihen/Spalten anordnen (Array), Shapes nummerieren, Maßlinien, Zeichnung drucken nach Bereichen; Makros entfallen | A 185746 | P2 (Array, Nummerieren), P3 | neu |
| RX-01 | Entwicklertools / Code | Visual Basic, Makros, Makro aufzeichnen, Makrosicherheit | – | **entfällt** (NO-01) | A 183847 | – | – |
| RX-02 | Entwicklertools / Add-Ins, Steuerelemente | COM-Add-Ins, ActiveX einfügen, Entwurfsmodus | – | **entfällt** (NO-02, NO-03) | A 183847, A 185832, A 185905 | – | – |
| RX-03 | Entwicklertools / Shape-Design | ShapeSheet anzeigen | SK groß | Shape, Seite, Dokument (SS-46) | A 183847 | P2 | neu |
| RX-04 | Entwicklertools / Shape-Design | Zeichentools, Vorgänge, Gruppieren | SK/DD klein | Zeichnen; Shape-Operationen (SP-08) | A 183847 | P1/P2 | neu |
| RX-05 | Entwicklertools / Shape-Design | Shape-Name | K klein | Name, ID, Master, Typ anzeigen und ändern | A 183847; SbS 11 | P2 | neu |
| RX-06 | Entwicklertools / Shape-Design | Verhalten | K klein | Dialog: 1D/2D, Auswahlrahmen, Gruppenverhalten, Doppelklick-Aktion, Platzierung, Verbinder teilen | A 183847; UG | P2 | neu |
| RX-07 | Entwicklertools / Shape-Design | Schutz | K klein | Dialog mit allen Sperren (SS-15), Alle/Keine | A 183847; TP | P2 | neu |
| RX-08 | Entwicklertools / Schablone | Neue Schablone (metrisch/US) | K groß | MS-06 | A 183847 | P2 | neu |
| RX-09 | Entwicklertools / Einblenden | Dokumentschablone, Zeichnungsexplorer, Masterexplorer | KK | MS-05, 3.11 | A 183847 | P2 | neu |
| RX-10 | Entwicklertools / Tools | Ereignisüberwachung | K klein | Protokoll aller Modell-Ereignisse (Auswahl, Zelle geändert, Shape gelöscht …) als Fehlersuch-Werkzeug | A 185942 | P3 | neu |
| RX-11 | Entwicklertools / Tools | Beständige Ereignisse | K klein | Ereignis → Aktion je Dokument (nur eingebaute Aktionen) | A 183847 | P3 | neu |
| RX-12 | Entwicklertools / Tools | ShapeSheet drucken | K klein | Blatt (Dokument/Seite/Shape), Ziel (Zwischenablage/Datei/Drucker), Abschnittsauswahl | A 190010 | P3 | neu |
| RH-01 | Hilfe | Hilfe (F1), Schulung | K groß | eigene Offline-Hilfe und Online-Doku wie bei JFNetworker (networker.jfreund.org/docs-Muster); Support/Feedback als Link | A 183906 | P2 | teilweise (md_view) |

### 3.10 Kontext- und Vorlagenregister

| ID | Register | Gruppen und Befehle | Quelle | Prio | Bestand |
|---|---|---|---|---|---|
| RK-01 | Container-Tools › Format | Größe (Ränder, An Inhalt anpassen, Automatische Größe), Containerformatvorlagen (Stil-Katalog, Kopfzeilen-Katalog), Mitgliedschaft (Container sperren, Inhalt auswählen, Container auflösen) | SbS 13 | P1 | neu |
| RK-02 | Bildtools › Format | Anpassen (Helligkeit, Kontrast, automatischer Ausgleich, komprimieren), Bildformatvorlagen (Linie, Effekte), Anordnen (vor/zurück, drehen), Zuschneiden | SbS 3, 5, 12; TP | P2 | neu |
| RK-03 | Funktionsübergreifendes Flussdiagramm | Einfügen (Swimlane, Trennlinie/Phase, Seite), Anordnen (Ausrichtung, Richtung, Ränder), Entwurf (Formatvorlagen, Bahnbeschriftungen drehen, Titelleiste ein/aus, Trennlinien ein/aus) | SbS 4 | P1 | neu |
| RK-04 | Organigramm | Layout (Katalog mit 18 Layouts, neu anordnen, an Seite anpassen), Anordnen (Abstand +/−, verschieben mit Untergebenen, Untergeordnete ein-/ausblenden), Shapes (10 Stile, Höhe/Breite +/−, Positionstyp ändern), Bild (einfügen, mehrere, ändern, löschen, ein-/ausblenden), Organisationsdaten (Importieren, Exportieren, Vergleichen, Synchronisieren) – **Stile neu gestalten** | SbS 5; TP | P2 | neu |
| RK-05 | Brainstorming | Themen hinzufügen (Haupt-, Unter-, Gleichrangiges, mehrere Unterthemen), Anordnen (Thema ändern, Layout, Diagrammformat), Verwalten (Gliederungsfenster, Daten exportieren nach Word-/Excel-kompatiblen Formaten und XML, Daten importieren) | TP | P3 | neu |
| RK-06 | Zeitachse | Konfigurieren (Zeitraum, Skala, Format), Meilenstein/Intervall einfügen, Zeitachse erweitern, synchronisieren | TP | P3 | neu |
| RK-07 | Kalender | Monat/Woche konfigurieren, Termine importieren (iCalendar statt Outlook) | TP | P3 | neu |
| RK-08 | Schablone bearbeiten | Neuer Master, Master-Eigenschaften (Name, QuickInfo, Symbolgröße, Abgleich über Namen), Symbol bearbeiten, Master bearbeiten, Speichern | A 184640; Fav | P2 | neu |
| RK-09 | Symbol-Editor | Pixel-Symbol eines Masters bearbeiten – JFDiagrammer erzeugt Symbole stattdessen automatisch aus der Shape-Grafik; Editor nur P3 | A 184640 | P3 | neu |
| RK-10 | Master bearbeiten | Fenster zum Bearbeiten eines Masters mit allen Zeichenfunktionen, Schließen aktualisiert Instanzen | W | P2 | neu |
| RK-11 | Textbearbeitung | Beim Bearbeiten von Text: Einfügen › Symbol, Feld, Link, QuickInfo aktiv; Minisymbolleiste mit Schrift | A 184911 | P1 | neu |
| RK-12 | Gantt-Diagramm, Wertstromdiagramm, Raumplan, Elektro u. a. | vorlagenabhängige Register bzw. Kontextmenüs (Abschnitt 5) | SbS 1; TP | P3 | neu |

### 3.11 Shapes-Seitenleiste

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| SL-01 | Aufbau | Kopf „Shapes“ mit Einklappen (schmale Leiste nur mit Symbolen), oben die Titel aller offenen Schablonen (bleiben stehen), darunter die Shapes der aktiven Schablone; Trenner zwischen Titeln und Inhalt verschiebbar; Breite spaltenweise ziehen; links angedockt, an jeden Rand andockbar oder schwebend. | A 183536, A 185905; SbS 1, 7 | P1 | App+Lib-ui | teilweise (JFNetworker palette.rs, dock.rs) |
| SL-02 | Shape-Kacheln | Symbol + Name (bis zwei Zeilen, sonst gekürzt), ein- oder zweispaltig; Ansichten Symbole und Namen / nur Symbole / nur Namen / Symbole und Details; QuickInfo mit Beschreibung; Livevorschau beim Zeigen (Option). | A 183536; S 1104-131657 | P1 | App+Lib-ui | teilweise |
| SL-03 | Platzieren | Ziehen auf die Seite (mit Ausrichtungshilfen und Container-/Listen-Rückmeldung), Klick + Klick-Platzieren, Enter = Mitte der Ansicht, Strg-Ziehen für mehrere; Ablegen auf Verbinder teilt ihn. | SbS 2; Bestand JFNetworker | P1 | App+Lib-ui | teilweise |
| SL-04 | Shapes suchen | Suchfeld mit Verlauf; Treffer aus allen installierten und offenen Schablonen (Namen, Schlüsselwörter, Beschreibung) als eigene Ergebnisschablone; alle Wörter (UND) oder irgendein Wort (ODER); abschaltbar. | A 183536, A 184605; S 1104-131657 | P1 | App | teilweise (palette-Suche) |
| SL-05 | Weitere Shapes | Kaskadenmenü Kategorie → Schablone (Häkchen = offen), dazu Meine Shapes, Neue Schablone (metrisch/US), Schablone öffnen (Dateidialog mit Filter `.vssx/.vssm/.vss/.vsx`), Dokumentschablone anzeigen; Azure-Download-Eintrag (Abschnitt 4). | A 190527; S 1104-131657; SbS 1 | P1 | App | neu |
| SL-06 | Schnell-Shapes | Sammelansicht der Schnell-Shapes aller offenen Schablonen; leer mit Hinweis zum Ablegen; Schnell-Shapes speisen AutoVerbinden. | A 185905; S 1104-131657; SbS 2 | P1 | App | neu |
| SL-07 | Favoriten / Meine Shapes | Eigene Schablone „Favoriten“: Shapes per Rechtsklick „Zu meinen Shapes hinzufügen“ oder Ziehen hineinlegen, umordnen, entfernen; gespeichert als `.vssx` im Benutzerordner. | Fav; W | P1 | App | neu |
| SL-08 | Schablonen-Menü | Am Schablonentitel: Schließen, Speichern, Bearbeiten (Kontextregister RK-08), Ansichtsoptionen, neues Master, Reihenfolge. | A 185905 | P2 | App | neu |
| SL-09 | Leerzustand | Ohne offene Schablone ein Hinweis mit Link auf „Weitere Shapes“. | A 08-214121 | P1 | App | neu |
| SL-10 | Master aus Zeichnung | Shape von der Seite in eine bearbeitbare Schablone ziehen erzeugt einen Master mit automatisch gerendertem Symbol. | SbS A | P2 | App+Lib-core | neu |
| SL-11 | Schablonen-Bericht | Neue Zeichnung mit allen Mastern einer Schablone als Katalogübersicht. | UG Anhang | P3 | App | neu |

### 3.12 Zeichenfenster

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| ZF-01 | Seite und Zeichenfläche | Weißes Blatt mit Schatten auf grauer Zeichenfläche; Shapes dürfen außerhalb liegen (werden nicht gedruckt); Seitenumbrüche (Druckkacheln) als Linien; Seitenfarbe aus dem Design. | A 183536; SbS 1 | P1 | Lib-ui+App | teilweise |
| ZF-02 | Lineale | Oben und links in Seiteneinheiten (bzw. Zeichnungseinheiten bei Maßstab), Nullpunkt unten links der Seite, verschiebbar (Strg-Ziehen aus der Ecke); beim Ziehen/Größenändern Markierungen für Kanten und Mitte. | A 183536, A 183959; SbS 2 | P1 | Lib-ui | teilweise (jfWord ruler) |
| ZF-03 | Raster | Feines/normales/grobes/festes Raster, passt sich dem Zoom an; Einrasten; Dialog „Lineal und Gitternetz“. | A 183830; SbS 1; TP | P1 | Lib-ui | teilweise (dg_paint_grid) |
| ZF-04 | Führungslinien | Aus dem Lineal ziehen, kleben, mitnehmen, löschen (SP-10). | SbS 2 | P1 | Lib-ui+App | neu |
| ZF-05 | Dynamisches Gitter | Gestrichelte Ausrichtungslinien zu Kanten/Mitten anderer Shapes, Abstandspfeile bei gleichem Abstand, Maßgleichheit beim Größenändern. | SbS 2; TP | P1 | Lib-ui | neu |
| ZF-06 | Auswahl | Klick, Umschalt/Strg-Klick, Rahmenauswahl (ganz oder teilweise umschlossen), Lasso, Alles, Unterauswahl in Gruppen, Anker-Shape mit dickerem Rahmen (Ausrichten richtet sich nach ihm). | SbS 2, 3, 6 | P1 | Lib-ui | teilweise |
| ZF-07 | Griffe | 8 Größengriffe + Drehgriff, gesperrte Griffe mit Schraffur, gelbe Steuerpunkte, 1D-Endpunkte (offen/geklebt unterscheidbar), Scheitelpunkte beim Linienwerkzeug, Textblock-Griffe. | SbS 2; TP | P1 | Lib-ui | teilweise |
| ZF-08 | Zoom und Verschieben | Rad scrollt (Umschalt: waagerecht), Strg(+Umschalt)+Rad zoomt am Zeiger, Strg+Umschalt-Ziehen zoomt auf Rechteck, Strg+Umschalt-Rechtsklick zoomt heraus, Alt+F6/Umschalt+Alt+F6; ganze Seite, Seitenbreite, Prozentwerte; Ansicht je Seite merken. | SbS 1; A 183536 | P1 | Lib-ui | teilweise |
| ZF-09 | Livedynamik | Shapes folgen beim Ziehen sofort mit ihrer echten Darstellung (Option, sonst Umriss). | A 184549 | P1 | Lib-ui | teilweise |
| ZF-10 | AutoVerbinden-Pfeile und Schnell-Shape-Leiste | CN-07. | SbS 2; TP | P1 | App+Lib-ui | neu |
| ZF-11 | Minisymbolleiste | Bei Rechtsklick/Textauswahl: Schrift, Größe, Fett/Kursiv/Unterstrichen, Schriftfarbe, Formatvorlagen, Füllung, Linie, Format übertragen, Zeiger, Verbinder (einmal), Zeichentools, Textfelder, Text ausrichten (9), Gruppieren, Ausrichten, Positionieren, vor/zurück, Shape ändern. | SbS 2, 3; A 184427 | P2 | App+Lib-ui | neu |
| ZF-12 | Kontextmenü am Shape | Ausschneiden, Kopieren, Einfügen; Gruppierung ▸; Container ▸; Anordnen ▸; Daten ▸ (Shape-Daten, definieren, verknüpfte Zeile, Datengrafik); Hyperlink; Kommentar; Formatieren ▸ (Text, Linie, Füllung, Schatten); Shape ▸; Verbinderart (rechtwinklig/gerade/gebogen, zurücksetzen); Zu meinen Shapes; Hyperlinks des Shapes; plus Aktionen des Shapes (SS-31). | SbS 2, 3, 4, 11; TP | P1 | App | neu |
| ZF-13 | Kontextmenüs Seite, Register, Schablone | Seite: Einfügen, Zeichentools, Seite einrichten; Seitenregister: Einfügen, Löschen, Umbenennen, Duplizieren, Neu anordnen, Seite einrichten, Zeichnungsmaßstab; Schablone: siehe SL-08. | SbS 1, 3, 7 | P1 | App | neu |
| ZF-14 | Seitenregister | Register je Seite (aktiv hervorgehoben), Hintergrundseiten als eigene Register, „Alle“-Liste, Plus-Knopf, Doppelklick = Umbenennen, Ziehen = Umordnen, Strg+Bild auf/ab = Seite wechseln. | A 183536; SbS 1, 3 | P1 | App+Lib-ui | teilweise (Tabs) |
| ZF-15 | Hyperlink-Zeiger | Zeiger mit Link-Symbol und QuickInfo über Shapes mit Link; Strg+Klick folgt. | SbS 11 | P1 | App | neu |
| ZF-16 | Text-Zoom | Beim Bearbeiten sehr kleiner Schrift (unter 8 pt einstellbar) vergrößerte Bearbeitung. | A 184549 | P2 | App+Lib-ui | neu |
| ZF-17 | Präsentationsmodus | Vollbild ohne Oberfläche, blättern mit Pfeilen/Bild-Tasten, Links folgen. | A 183830; SbS 1 | P2 | App | neu |
| ZF-18 | Barrierefreiheit | Shapes und Verbinder als Bildschirmleser-Elemente mit Alternativtext und Lesereihenfolge, Tastatur-Navigation (Tab durch Shapes). | Z2 (`visAltText`, `visNavOrder`); Bestand | P1 | Lib-ui | teilweise (diagram_sems) |
| ZF-19 | Stempel-Werkzeug | Master in der Seitenleiste wählen, dann beliebig oft auf die Seite klicken; jedes Klicken legt ein Shape mit seinem Drehpunkt an die Stelle; Esc beendet. | UG Kap. 2; Bestand JFNetworker (Umschalt hält das Platzieren) | P2 | App | teilweise |
| ZF-20 | Zeichnung zentrieren | Alle Shapes so verschieben, dass die Zeichnung mittig auf der Seite liegt. | UG Kap. 10 | P2 | App | neu |
| ZF-21 | Nullpunkt verschieben | Strg-Ziehen aus der Linealecke (beide Achsen) oder aus einem Lineal (eine Achse); Doppelklick auf die Ecke setzt zurück; der Nullpunkt ist auch Drehzentrum für Seite und Führungslinien. | UG Kap. 3, 17 | P2 | Lib-ui | neu |
| ZF-22 | Lupe | Strg+Umschalt+Klick zoomt mit Lupenzeiger hinein, mit Rechtsklick heraus. | UG Kap. 17; SbS 1 | P1 | Lib-ui | neu |

### 3.13 Aufgabenbereiche

| ID | Bereich | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| AB-01 | Allgemein | Alle Bereiche andockbar (vier Ränder), schwebend, außerhalb des Hauptfensters, Pin zum automatischen Ausblenden, Größe änderbar, merken ihre letzte Position. | SbS 1, 8; TP | P1 | Lib-ui | teilweise (dock.rs) |
| AB-02 | Shapes | 3.11. | A 183536 | P1 | App | teilweise |
| AB-03 | Shape-Daten | Felder (Bezeichnung/Wert) des gewählten Shapes, passende Eingabe je Typ (Liste, Datum mit Kalender, Boolesch, Dauer); bei Mehrfachauswahl nur gemeinsame Felder; QuickInfo = Prompt; Kontextmenü: Shape-Daten definieren, Datensätze. | SbS 8 | P1 | App+Lib-ui | teilweise (inspector) |
| AB-04 | Größe und Position | 2D: X, Y, Breite, Höhe, Winkel, Drehpunkt (9 Lagen); 1D wahlweise Anfang+Ende, Anfang+Länge+Winkel oder Ende+Länge+Winkel; Führungslinien: Art (Punkt, waagerecht, senkrecht, gedreht), X, Y, Winkel; Eingaben mit Einheiten. | SbS 2; TP; UG Kap. 17 | P1 | App | neu |
| AB-05 | Shape formatieren | Register Füllung und Linie, Effekte (Schatten, Spiegelung, Leuchten, weiche Kanten, 3D-Format, 3D-Drehung), Text; Voreinstellungen je Abschnitt. | SbS 6; TP | P1 | App | neu |
| AB-06 | Verschieben und Zoomen | Übersicht der Seite mit Sichtrahmen: ziehen = schwenken, Kanten = zoomen, neues Rechteck = dorthin zoomen; Zoom-Regler. | SbS 1; TP | P1 | App+Lib-ui | teilweise (diagram_minimap) |
| AB-07 | Navigation | Liste aller Shapes der Seite (Name, Text) zum Auswählen und Ordnen der Lesereihenfolge. | A 183830; TP | P2 | App | neu |
| AB-08 | Zeichnungsexplorer | Baum: Vorder-/Hintergrundseiten → Shapes (auch Gruppenkinder), Ebenen, Master, Formatvorlagen, Füll-/Linienmuster, Linienenden, Fenster; Kontextmenüs (umbenennen, löschen, ShapeSheet). | A 183847; UG | P2 | App | neu |
| AB-09 | Ebenen | Ebenen der Seite mit Sichtbar/Drucken/Aktiv/Sperren/Einrasten/Kleben/Farbe direkt schaltbar (Visio nutzt einen Dialog; ein Bereich ist bequemer). | SbS 3; A 184351 | P2 | App | neu |
| AB-10 | Kommentare | HL-04. | SbS 3; A 185607 | P2 | App | neu |
| AB-11 | Externe Daten | Tabellenfenster je Datenquelle (Register), verknüpfte Zeilen mit Symbol, Kontextmenü (Quelle hinzufügen/entfernen, Spalteneinstellungen, verknüpfte Shapes, lösen, aktualisieren, Intervall, Datengrafikfelder). | SbS 10 | P2 | App+Lib-ui | teilweise (grid.rs) |
| AB-12 | Datengrafikfelder | DA-08. | SbS 10 | P2 | App | neu |
| AB-13 | Probleme | DA-10. | SbS 14 | P2 | App | neu |
| AB-14 | Gliederung (Brainstorming) | Themenbaum zum Bearbeiten der Mindmap. | TP | P3 | App | neu |
| AB-15 | Folienausschnitte | Ausschnitte markieren und als Folien exportieren – JFDiagrammer: als PDF-Seiten oder Bilder statt PowerPoint. | A 183830; TP | P3 | App | neu |
| AB-16 | Formatvorlagen (Entwickler) | Formatvorlagen anzeigen, zuweisen, definieren (MS-08). | UG | P3 | App | neu |

### 3.14 Statusleiste

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| SB-01 | Seite | „Seite 1 von n“ (Visio: „Zeichenbl.“); Klick öffnet Seitenliste. | A 183536; SbS 1 | P1 | App | neu |
| SB-02 | Maße der Auswahl | Breite, Höhe, Winkel bzw. Länge; Klick öffnet „Größe und Position“. | SbS 1 | P1 | App | neu |
| SB-03 | Sprache | Sprache des Textes an der Einfügemarke; Klick setzt sie. | A 183536 | P2 | App | neu |
| SB-04 | Zoom | Präsentationsmodus, Zoom −/+, Regler (Mitte 100 %), Zoomwert (Klick = Dialog), An Fenster anpassen, Fenster wechseln. | A 183536; SbS 1 | P1 | App+Lib-ui | teilweise |
| SB-05 | Anpassen | Rechtsklick blendet einzelne Elemente ein/aus. | SbS 1 | P3 | App | neu |
| SB-06 | Makrorekorder-Symbol | **entfällt** (NO-01). | A 183536 | – | – | – |

### 3.15 Drucken und Seite einrichten

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| PR-01 | Druckansicht | Backstage „Drucken“ (Strg+P): links Einstellungen, rechts Vorschau mit Blattwahl, Zoom, Verschieben, Kachelung sichtbar; Farbe/Graustufen. | SbS 12; TP | P1 | App+Lib-plat | teilweise |
| PR-02 | Druckbereich | Alle Seiten, aktuelle Seite, aktuelle Ansicht, Auswahl, Bereich; ohne Hintergrund; hohe Qualität (Effekte ein/aus); Exemplare; Drucker und Papier. | SbS 12; TP | P1 | App+Lib-plat | teilweise |
| PR-03 | Kacheln | Seite größer als Papier → über mehrere Blätter (Anpassen auf x %, „an n × m Blätter anpassen“), nie abgeschnitten, Kacheln beginnen oben links; Seitenumbrüche sichtbar, Shapes von Umbrüchen wegschieben/meiden. | SbS 3, 6, 12; UG Kap. 3, 10 | P1 | App | neu |
| PR-04 | Kopf- und Fußzeile | Je Druckblatt links/Mitte/rechts mit Platzhaltern für Seitenzahl, Gesamtzahl gedruckter Seiten, Seitenname, Dateiname, Dateiendung, Datum kurz/lang, Uhrzeit; Abstand vom Papierrand, Schrift. | SbS 12; UG Kap. 10 | P2 | App | neu |
| PR-05 | Seite einrichten – Drucken einrichten | Papierformat, Ausrichtung, Zoom (anpassen auf %), „Anpassen an“ Blätter, Gitternetz drucken, Vorschau. | SbS 3 | P1 | App | neu |
| PR-06 | Seite einrichten – Seitengröße | Wie Drucker, vordefiniert, benutzerdefiniert, automatisch erweitern, Ausrichtung. | SbS 3 | P1 | App | neu |
| PR-07 | Seite einrichten – Zeichnungsmaßstab | Kein Maßstab, vordefiniert (4 Gruppen), eigener; Anzeige der realen Seitengröße. | SbS 7; TP | P1 | App | neu |
| PR-08 | Seite einrichten – Seiteneigenschaften | Typ (Vorder-/Hintergrund), Name, Hintergrundseite, Maßeinheit, Formen-Schatten-Richtung. | SbS 3 | P1 | App | neu |
| PR-09 | Seite einrichten – Layout und Routing | Routing-Stil, Richtung, Verbinder-Darstellung, Linienübergänge (wo, Stil, Größe), Abstände, „Shapes beim Ablegen verschieben“, „Shapes auf Verbinder teilen“. | SbS 3; UG | P1 | App | neu |
| PR-10 | Seite einrichten – Schatten | Standard-Schattenversatz und -richtung für die Seite. | W | P3 | App | neu |

### 3.16 Tastenkürzel

Grundlage ist die Liste aus SbS Anhang B (englische Belegung) plus Fließtext und Tutorial.
Die deutsche Oberfläche belegt einige Textformate anders (Office-Konvention: Fett
Strg+Umschalt+F, Kursiv Strg+Umschalt+K); das kollidiert mit „In den Vordergrund“
(Strg+Umschalt+F) und „Kapitälchen“ (Strg+Umschalt+K). Entscheidung F10: Textformate nach
deutscher Office-Konvention (Fett Strg+Umschalt+F, Kursiv Strg+Umschalt+K, Unterstrichen
Strg+Umschalt+U); „In den Vordergrund“, „In den Hintergrund“ und „Kapitälchen“ bekommen freie
Tasten. Alles bleibt über UI-19 anpassbar und wird auf Konflikte geprüft. Für die anderen vier
Sprachen gilt die Office-Konvention der jeweiligen Sprache (LG-08, abgeleitet, noch zu bestätigen).
Alle Kürzel: Ort
`App` (+ `Lib-ui` für Zoom/Fokus), Prio wie angegeben.

| ID | Bereich | Aktion → Kürzel | Quelle | Prio | Bestand |
|---|---|---|---|---|---|
| KB-01 | Datei | Neu Strg+N, Öffnen Strg+O, Speichern Strg+S, Speichern unter F12, Drucken Strg+P, Schließen Strg+W/Strg+F4 | SbS 12; W | P1 | teilweise |
| KB-02 | Bearbeiten | Rückgängig Strg+Z, Wiederholen Strg+Y, Ausschneiden/Kopieren/Einfügen Strg+X/C/V, Duplizieren Strg+D, Alles Strg+A, Löschen Entf, Suchen Strg+F, Ersetzen Strg+H (Konflikt mit Spiegeln, siehe KB-07) | SbS 2, B; A 184334 | P1 | teilweise |
| KB-03 | Zoom | hinein Alt+F6, Strg+Umschalt+Linksklick, Strg(+Umschalt)+Rad vor; heraus Umschalt+Alt+F6, Strg+Umschalt+Rechtsklick, Strg(+Umschalt)+Rad zurück; Fenster einpassen Strg+Umschalt+W; Rechteck-Zoom Strg+Umschalt+Ziehen | SbS 1, B | P1 | teilweise (Strg+Rad, Strg+0) |
| KB-04 | Schwenken | Rad (senkrecht), Umschalt+Rad (waagerecht), Strg+Umschalt+Rechts-Ziehen | SbS 1, B | P1 | teilweise |
| KB-05 | Seiten | nächste/vorige Seite Strg+Bild ab/auf, alle Seiten Alt+F3, neue Seite Umschalt+F11, Seite einrichten Umschalt+F5 | SbS 1, B; TP | P1 | neu |
| KB-06 | Fokus/Barrierefreiheit | Tab/Umschalt+Tab von Shape zu Shape (gesperrte überspringen), Enter wählt fokussiertes Shape (mit Umschalt Mehrfachauswahl), Esc hebt auf | SbS B | P1 | teilweise (diagram.rs) |
| KB-07 | Anordnen | Pfeiltasten verschieben, Umschalt+Pfeil 1 Pixel; Gruppieren Strg+G/Strg+Umschalt+G; Aufheben Strg+Umschalt+U; Vordergrund/Hintergrund auf freien Tasten (Visio: Strg+Umschalt+F/B; F10); 90° links Strg+L, rechts Strg+R; horizontal spiegeln Strg+H, vertikal Strg+J; Shapes ausrichten F8 | SbS B | P1 | teilweise |
| KB-08 | Werkzeuge | Zeiger Strg+1, Text Strg+2, Verbinder Strg+3, Verbindungspunkt Strg+Umschalt+1, Zuschneiden Strg+Umschalt+2, Textblock Strg+Umschalt+4, Format übertragen Strg+Umschalt+P | SbS B | P1 | neu |
| KB-09 | Zeichenwerkzeuge | Bleistift Strg+4, Freihand Strg+5, Linie Strg+6, Bogen Strg+7, Rechteck Strg+8, Ellipse Strg+9; Umschalt beim Zeichnen = Quadrat/Kreis; Umschalt-Ziehen = Achse halten; Strg+Umschalt-Ziehen = duplizieren und Achse halten | SbS 2, B | P1 | neu |
| KB-10 | Text | Bearbeiten F2 (Ende F2/Esc), Text-Dialog F11, Umschalt+Enter neue Zeile (wenn Enter beendet), Rechtschreibung F7; Cursor/Markieren mit Pos1/Ende, Strg+Pfeile (Wort, Absatz), Umschalt-Varianten, Strg+Rücktaste | SbS 3, B | P1 | teilweise (rich_edit) |
| KB-11 | Textformat | Fett Strg+Umschalt+F, Kursiv Strg+Umschalt+K, Unterstrichen Strg+Umschalt+U (F10), doppelt unterstrichen Strg+Umschalt+D, Großbuchstaben Strg+Umschalt+A, Kapitälchen (freie Taste), tief Strg+=, hoch Strg+Umschalt+=, größer Strg+Umschalt+>, kleiner Strg+Umschalt+<; ältere Visio-Belegung: Fett Strg+Umschalt+B, Kursiv Strg+Umschalt+I, Kapitälchen Strg+Umschalt+Y, tief Strg+Umschalt+X, hoch Strg+Umschalt+Z | SbS B; UG Kap. 8 | P1/P2 | neu |
| KB-12 | Textausrichtung | links/zentriert/rechts/Blocksatz Strg+Umschalt+L/C/R/J, oben/Mitte/unten Strg+Umschalt+T/M/V | SbS B | P1 | neu |
| KB-13 | Links | Hyperlink Strg+K, folgen Strg+Klick | SbS 11 | P1 | neu |
| KB-14 | Fenster | Präsentationsmodus F5 (Esc beendet, Bild/Pfeile blättern), nebeneinander Umschalt+F7, untereinander Strg+Umschalt+F7, überlappend Alt+F7, Ausrichten und Kleben Alt+F9 | SbS B; TP | P2 | neu |
| KB-15 | Shapes-Seitenleiste | Pfeile zwischen Shapes, Pos1/Ende, Bild auf/ab, Strg+C/V in bearbeitbare Schablone, Strg+A, Umschalt+Enter (wählen), Strg+Enter (in die Zeichnung einfügen), Entf/Strg+X/F2 in bearbeitbarer Schablone | SbS B | P1 | teilweise (palette) |
| KB-16 | Menüband | Alt zeigt Zugriffstasten (z. B. Alt, R, … für Register), F6 wechselt zwischen Bereichen, Umschalt+F10/Menütaste Kontextmenü, F1 Hilfe | SbS B | P1 | teilweise |
| KB-17 | Entwickler | ShapeSheet des Dokuments: Umschalt+Rechtsklick auf Seite; Visual-Basic-Editor Alt+F11 **entfällt** | SbS A | P2 | neu |
| KB-18 | Wiederholen | F4 wiederholt die letzte Formatänderung bzw. die letzte Kopie mit gleichem Abstand | UG Kap. 5, 17 | P2 | neu |

### 3.17 Sprachen und Regionen

Nachtrag des Nutzers (2026-11-14): Alles gibt es auf **Deutsch, Englisch, Spanisch,
Italienisch und Französisch**, umschaltbar in den Einstellungen. Deutsch ist wie in allen
JFLib-Apps die Quellsprache der Kataloge. Die Entscheidungen F8 und F13 gelten sinngemäß für
alle fünf Sprachen: Jede Sprache bekommt eigene beschreibende Begriffe. Visio-eigene Begriffe
(SmartShapes, AutoConnect, Quick Shapes, ShapeSheet in der Oberfläche) werden in keiner
Sprache verwendet.

**Bestand in JFLib:** `lib/ui/i18n.rs` übersetzt die eigenen Texte der Bibliothek
(Dateidialog, Befehlsleiste, Kurzbefehle, Datumswahl …) schon in neun Sprachen, darunter alle
fünf. Apps bringen ihre Texte als TSV-Katalog mit (`Catalog::from_tsv`, `locales/*.tsv`, erste
Zeile = Sprachspalten). `set_language` wirkt prozessweit und **zur Laufzeit**. Befehls- und
Menütabellen werden je Sprache einmal übersetzt (`i18n::commands`/`menu`). JFNetworker
schaltet so schon ohne Neustart um (`src/app/tr.rs`, `apply`). `i18n_locale.rs` liefert
Zahlen-, Datums- und Währungsformate je Region, `i18n_msg.rs` Platzhalter und Pluralformen,
`settings_page.rs` die Einstellungsseite mit Auswahlfeldern. Die vorhandenen App-Kataloge haben
nur die Spalten `de` und `en`.

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| LG-01 | Fünf Oberflächensprachen | Menüband, Backstage, Dialoge, Aufgabenbereiche, Kontextmenüs, Meldungen, QuickInfos, Statusleiste und Assistenten auf DE, EN, ES, IT, FR; Katalog `locales/ui.tsv` mit fünf Spalten, Platzhalter und Plural über `msg`. Fehlende Übersetzungen fallen auf Englisch zurück, dann auf Deutsch. | Nachtrag 2026-11-14; `lib/ui/i18n.rs` | P1 | App+Lib-ui | teilweise (Bibliothekstexte fertig, App-Katalog neu) |
| LG-02 | Sprache umschalten ohne Neustart | Einstellung „Sprache“ (Systemsprache oder eine der fünf) auf der Einstellungsseite. Ein Wechsel wirkt sofort, wie in JFNetworker. Die App darf deshalb keine übersetzten Texte zwischenspeichern, außer über die Tabellen-Funktionen der Bibliothek. Gespeicherte Dokumente ändern ihre Namen dabei nicht (LG-04). | Nachtrag 2026-11-14; `i18n::set_language`, JFNetworker `tr::apply` | P1 | App+Lib-ui | vorhanden (Bibliothek), App neu |
| LG-03 | Hilfe und QuickInfos | QuickInfos mit Beschreibung und Tastenkürzel, Offline-Hilfe und Online-Dokumentation in allen fünf Sprachen; die Hilfe folgt der Oberflächensprache. | Nachtrag 2026-11-14; A 184532 | P1 | App | teilweise (md_view) |
| LG-04 | Namen von Schablonen, Shapes und Vorlagen | Jeder mitgelieferte Master, jede Schablone und jede Vorlage hat einen universellen Namen (`NameU`, sprachneutraler Schlüssel) und angezeigte Namen, QuickInfos und Suchbegriffe in allen fünf Sprachen. Beim Ablegen schreibt JFDiagrammer den Namen der aktuellen Sprache als `Name` ins Dokument. **Beim Import** ist `NameU` der Schlüssel (Master-Abgleich, Formelbezüge, Zuordnung zu eigenen Shapes); `Name` ist der angezeigte Name aus der Datei und bleibt unverändert. | Nachtrag 2026-11-14; Z2, Z3 (`Name` „Zeichenblatt-1“, `NameU` „Page-1“), Fav, SWDB (Liste alternativer Dateinamen je Sprache) | P1 | App+Lib-core | neu |
| LG-05 | Mehrsprachige Shape-Suche | Die Suche in der Seitenleiste findet Namen und Suchbegriffe der Oberflächensprache, dazu universelle Namen. Wer z. B. englische Namen kennt, findet die Shapes auch in einer anderen Sprache. | Nachtrag 2026-11-14 | P2 | App | teilweise (palette-Suche) |
| LG-06 | Begriffsglossar | Verbindliches Glossar der Fachbegriffe in allen fünf Sprachen als Datei im Repo (Quelle für Katalog und Hilfe). Ein Prüfskript meldet Abweichungen und verbotene Visio-Begriffe. Startfassung siehe unten. | Nachtrag 2026-11-14; F8, F13 | P1 | App | neu |
| LG-07 | Einheiten und Papierformate je Region | Die Region ist unabhängig von der Sprache einstellbar (z. B. Englisch mit A4). DE, ES, IT, FR und en-GB nutzen metrische Einheiten und A4; en-US nutzt Zoll und Letter. Jede mitgelieferte Schablone und Vorlage gibt es in metrischer und in US-Fassung, wie Visios `_m`/`_u`-Dateien. Der Vorlagendialog schlägt die Fassung der Region vor (MS-07, UI-04). Zahlen, Datum, Uhrzeit und Währung in Feldern, Shape-Daten und Berichten folgen der Region. | Nachtrag 2026-11-14; SbS 1; SWDB (`ORM_M.VSS`/`ORM_U.vss`); `i18n_locale.rs` | P1 | App+Lib-core+Lib-ui | teilweise (Formate vorhanden) |
| LG-08 | Tastenkürzel je Sprache | **Abgeleitet aus F10, vom Nutzer noch zu bestätigen:** Die Standardbelegung folgt der üblichen Office-Konvention der gewählten Sprache. Fett: DE Strg+Umschalt+F, EN Ctrl+B, ES Ctrl+N, IT/FR Ctrl+G; Kursiv und Unterstrichen entsprechend. Weil die Konventionen auch Speichern, Neu, Suchen und Gruppieren unterschiedlich belegen, wird jede Sprachbelegung auf Konflikte geprüft. Alles bleibt anpassbar (UI-19, 3.16). | Nachtrag 2026-11-14; F10 | P1 | App | neu |
| LG-09 | Rechtschreibung je Sprache | Rechtschreibprüfung und AutoKorrektur in den fünf Sprachen; die Sprache je Textlauf (`LangID`) wählt das Wörterbuch. | Nachtrag 2026-11-14; SbS 1; Z2, Z3 | P2 | App+Lib-ui | teilweise (spell) |
| LG-10 | Formelnamen | Lokalisierte und universelle Zell- und Funktionsnamen in Formeln (FX-01); der Formel-Editor zeigt die universelle Form. | MS-VSDX; Z2, Z3 | P2 | Lib-core | neu |
| LG-11 | Übersetzungsprüfung | Ein Prüfskript meldet fehlende Zellen in allen Katalogen und Namenslisten (alle fünf Spalten gefüllt, Platzhalter gleich), wie das i18n-Prüfskript von JFNetworker. | Nachtrag 2026-11-14 | P1 | App | teilweise (Muster in JFNetworker) |

**Glossar, Startfassung (Vorschlag, bei der Übersetzung zu prüfen).** Verbindlich wird es mit
LG-06. Zu jedem Begriff ist zu prüfen, dass er nicht Visios eigener Lokalisierung eines
Visio-Begriffs entspricht. Beim Deutschen gibt es dazu eine Rückfrage: „AutoVerbinden“ (F13)
ist zugleich Visios eigene deutsche Bezeichnung (A 183830). Die Alternative „Verbindungspfeile“
steht in Klammern, bis der Nutzer entscheidet.

| Begriff | DE | EN | ES | IT | FR |
|---|---|---|---|---|---|
| Seite | Seite | Page | Página | Pagina | Page |
| Hintergrundseite | Hintergrundseite | Background page | Página de fondo | Pagina di sfondo | Page d'arrière-plan |
| Ebene | Ebene | Layer | Capa | Livello | Calque |
| Schablone | Schablone | Stencil | Colección de formas | Raccolta di forme | Bibliothèque de formes |
| Master | Master-Shape | Master shape | Forma maestra | Forma master | Forme maître |
| Shape | Shape | Shape | Forma | Forma | Forme |
| Verbinder | Verbinder | Connector | Conector | Connettore | Connecteur |
| Verbindungspunkt | Verbindungspunkt | Connection point | Punto de conexión | Punto di connessione | Point de connexion |
| Steuerpunkt | Steuerpunkt | Control handle | Controlador | Maniglia di controllo | Poignée de contrôle |
| Container | Container | Container | Contenedor | Contenitore | Conteneur |
| Liste | Liste | List | Lista | Elenco | Liste |
| Swimlane | Bahn | Lane | Calle | Corsia | Couloir |
| Beschriftung (Callout) | Beschriftung | Callout | Llamada | Richiamo | Bulle d'annotation |
| Legende (Daten) | Legende | Legend | Leyenda | Legenda | Légende |
| Design | Design | Theme | Tema | Tema | Thème |
| Variante | Variante | Variant | Variante | Variante | Variante |
| Schnellformatvorlage | Schnellformatvorlage | Style preset | Estilo predefinido | Stile predefinito | Style prédéfini |
| Schnell-Shapes | Schnell-Shapes | Top shapes | Formas principales | Forme principali | Formes principales |
| AutoVerbinden | AutoVerbinden (Alternative: Verbindungspfeile) | Connect arrows | Flechas de conexión | Frecce di collegamento | Flèches de liaison |
| Intelligente Shapes | Intelligente Shapes | Adaptive shapes | Formas adaptables | Forme adattive | Formes adaptatives |
| ShapeSheet (in der Oberfläche) | Formelblatt | Formula sheet | Hoja de fórmulas | Foglio formule | Feuille de formules |
| Shape-Daten | Shape-Daten | Shape data | Datos de forma | Dati forma | Données de forme |
| Datengrafik | Datengrafik | Data graphic | Gráfico de datos | Grafica dati | Graphique de données |
| Führungslinie | Führungslinie | Guide | Guía | Guida | Repère |
| Raster | Raster | Grid | Cuadrícula | Griglia | Grille |
| Lineal | Lineal | Ruler | Regla | Righello | Règle |
| Menüband | Menüband | Ribbon | Cinta de opciones | Barra multifunzione | Ruban |
| Aufgabenbereich | Aufgabenbereich | Task pane | Panel de tareas | Riquadro attività | Volet |

<a id="shapes"></a>
## 4. Shape-Kategorien und Schablonen

Grundlage sind alle 278 Bilder in `context/shapes/` (deutsches Visio für Microsoft 365). Das
Menü „Weitere Shapes“ nennt 278 Schablonen. 247 davon sind mit ihren Shapes zu sehen, 31 nur
dem Namen nach; vier weitere stammen aus binären Testdateien älterer Versionen (SH-279 bis SH-282). Zusammen sind es rund 5400 Shape-Einträge; viele davon sind Doppelungen
(derselbe Verbinder in vielen Schablonen, elf Organigramm-Stile, 2D/3D-Paare, ältere
„– Visio 2013“-Fassungen).

**Kennzeichnung** je Schablone (bei gemischten je Shape-Gruppe):

- **Norm: …** – Symbole einer öffentlichen Norm oder Notation (ISO 5807, BPMN 2.0, UML 2.5,
  IEC 60617, ISO 10628/ISA 5.1, ISO 7010 …). Sie dürfen normgerecht aussehen und werden nach der
  Norm neu gezeichnet, und zwar in der DIN-EN/IEC-Fassung statt US/ANSI (F14). Visios Zeichnung
  ist dabei kein Vorbild. Farben, Strichstärken und Schrift kommen aus dem NetLab-Design (F2).
- **Generisch** – schlichte Grundgeometrie (Rechteck, Pfeil, Stern, Block), die jeder gleich
  zeichnen würde. Sie wird frei nach Maß gebaut.
- **Visio-Design** – gestaltete Grafiken (Geräte, Möbel, Personen, Dekor, 3D-Darstellungen).
  Sie werden **komplett neu im JFNetworker-Stil** gezeichnet; nachzeichnen ist verboten.
- **AWS-Original (CC BY-ND 2.0)** – die offiziellen AWS-Architektur-Icons, unverändert mit
  Namensnennung.
- **Azure-Download** – Microsoft-Azure-Icons. Sie werden nicht ausgeliefert, sondern per
  Download-Knopf vom Nutzer selbst geholt.

### 4.0 Regeln für alle Schablonen

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| SH-G01 | Stilrichtlinie JFNetworker-Stil | Gemeinsame Regeln für alle neu gezeichneten Shapes: Strichstärken, Ecken, Flächen, Akzentfarben aus dem Theme (Akzent 1–6), Schatten, Symbolraster, Text unter oder im Shape. Sie werden aus den JFNetworker-Symbolen (`icons/*.svg`) abgeleitet und schriftlich festgehalten, bevor Phase 3 beginnt; Normsymbole behalten ihre Normform (F2). | Entscheidung; JFNetworker `icons/` | P1 | App | teilweise |
| SH-G02 | Echte Master | Jede Shape ist ein vollständiger Master mit ShapeSheet: Geometrie mit Formeln, Verbindungspunkten, Steuerpunkten, Shape-Daten, Theme-Bezug (`THEMEVAL`), Ebene, Kategorien und Schlüsselwörtern. Nur so verhalten sich die Shapes wie in Visio und überstehen VSDX-Speichern und -Laden. | SbS 1, A; Z2, Z3 | P1 | App+Lib-core | neu |
| SH-G03 | Schablonen als Dateien | Mitgelieferte Schablonen liegen als `.vssx` (metrisch und US) im Programm; Name, Symbolbilder (aus dem Shape gerendert), QuickInfos und Suchbegriffe in allen fünf Sprachen, mit universellem Namen als Schlüssel (LG-04). | Fav; SbS 1 | P1 | App | neu |
| SH-G04 | Schnell-Shapes je Schablone | Für jede Schablone sind die ersten Shapes als Schnell-Shapes festgelegt (für AutoVerbinden). | SbS 2; S 1104-131657 | P1 | App | neu |
| SH-G05 | Fremde Master anzeigen | Shapes aus geöffneten Visio-Dateien zeigen immer ihre eigenen Master aus der Datei. Sie werden nicht durch JFDiagrammer-Shapes ersetzt. „Shape ändern“ kann auf eigene Shapes wechseln. | Z2, Z3; Entscheidung | P1 | Lib-core+App | neu |
| SH-G06 | AWS-Icons ausliefern | Die offiziellen AWS-Architektur-Icons als SVG unverändert in je einer Schablone pro AWS-Kategorie. Sie lassen sich nur skalieren, nicht umfärben (von Designs ausgenommen). Namensnennung und Lizenztext stehen im Info-Dialog und in den Schablonen-Eigenschaften. Neue Icon-Ausgaben werden mit einem Abgleich-Skript eingespielt. | Entscheidung; S 1104-132208 | P2 | App | neu |
| SH-G07 | Azure-Download-Knopf | Unter „Weitere Shapes › Netzwerk › Azure“ steht ein Knopf, der die offizielle Microsoft-Seite für die Azure-Icons im Browser öffnet. Der Nutzer lädt das ZIP selbst und stimmt den Bedingungen selbst zu. JFDiagrammer liest das ZIP (nur SVG) und baut daraus lokale Schablonen im Benutzerordner, je Azure-Kategorie eine. Nichts davon wird mitgeliefert oder weitergegeben. | Entscheidung; S 1104-132208 | P3 | App+Lib-svg | teilweise (img_svg, archive) |
| SH-G08 | Eigene Ergänzungen | Neue Kategorien und Normvarianten (DIN EN statt US/ANSI), siehe Vorschläge am Ende dieses Abschnitts. | Entscheidung | P2/P3 | App | neu |
| SH-G09 | Gemeinsamer Netzwerk-Symbolsatz | Die Netzwerkgeräte-Symbole liegen in einer Quelle für JFNetworker und JFDiagrammer, damit beide Apps gleich aussehen. Sie sind die Grundlage der P1-Netzwerkschablonen. | Entscheidung F9; JFNetworker `icons/` | P1 | App+JFNetworker | teilweise |

### 4.1 Kategorien aus „Weitere Shapes“

Hauptebene des Menüs (S 1104-131850): Meine Shapes, Allgemein, Flussdiagramm, Geschäft, Konstruktion, Netzwerk,
Pläne und Grundrisse, Software und Datenbank, Terminplan, Visio-Extras; darunter die Befehle „Schablone öffnen“,
„Neue Schablone (Metrisch)“, „Neue Schablone (US-Einheiten)“, „Dokumentschablone anzeigen“. „Meine Shapes“ ist der
Ordner eigener Schablonen; „Schablone öffnen“ zeigt dort eine Favoriten-Schablone und den Dateifilter
*.vssx/*.vssm/*.vss/*.vsx (S 1104-132533). Über den geöffneten Schablonen steht die Sammelansicht „Quick-Shapes“
(S 1104-131524); sie ist keine eigene Schablone. Mit „(nur Menü)“ markierte Schablonen sind im Material nur dem
Namen nach zu sehen.

- **Allgemein** (S 1104-131850): 3D-Block-Shapes, Blöcke, Dekorative Shapes, Diagramm- und Mathematik-Shapes,
  Perspektivische Blöcke, Standard-Shapes, Venn-Diagramm-Shapes
- **Flussdiagramm** (S 1104-131922): Abschlusszeichen - SharePoint 2010-Workflow, Abteilung, Abteilung - 3D,
  Aktionen - SharePoint 2010-Workflow (nur Menü), Aktionen – SharePoint-2013-Workflow (nur Menü),
  Bedingungen - SharePoint 2010-Workflow (nur Menü), Bedingungen – SharePoint-2013-Workflow (nur Menü),
  BPMN-Standard-Shapes, Funktionsübergreifende Flussdiagramm-Shapes, IDEF0-Diagramm-Shapes,
  Komponenten – SharePoint-2013-Workflow (nur Menü), Pfeil-Shapes, SDL-Diagramm-Shapes, Standardflussdiagramm-Shapes,
  Verschiedene Flussdiagramm-Shapes, Workflowobjekte, Workflowobjekte - 3D, Workflowschritte, Workflowschritte - 3D
- **Geschäft › Brainstorming** (S 1104-131939): Brainstorming-Shapes, Legenden-Shapes
- **Geschäft › Diagramme** (S 1104-132006): Diagramm-Shapes, Marketingdiagramme, Matrixdiagramm-Shapes,
  Pyramiden-Shapes, Zyklusdiagramm-Shapes
- **Geschäft › Geschäftsprozess** (S 1104-132020): Arbeitsflussdiagramm-Shapes, Auditdiagramm-Shapes,
  EPC-Diagramm-Shapes (nur Menü), Fehlerstrukturanalyse-Shapes, ITIL-Shapes, Kompatibilitäts-Shapes, Pfeil-Shapes
  (dieselbe Schablone wie unter Flussdiagramm), Shapes für Six Sigma-Flussdiagramme, Shapes für
  Ursache/Wirkung-Diagramme, Six-Sigma-House of Quality-Shapes, TQM-Diagramm-Shapes, Wertstromzuordnungs-Shapes
- **Geschäft › Organigramm** (S 1104-132035): Augenzahl, Blütenblätter, Eingebunden, Gürtel, Karten, Kerbe, Münze,
  Perspektive, Shapetakulär, Stein, Tafel (jeweils „… - Organigramm-Shapes“)
- **Geschäft › Pivotdiagramm** (S 1104-132055): Pivotdiagramm-Shapes
- **Konstruktion › Elektrotechnik** (S 1104-132112): Analog-Digital-Logisch, Baugruppen, Beschriftungssymbole,
  Beschriftungssymbole - Visio 2013 (nur Menü), Grundlegende Elemente, Grundlegende Elemente - Visio 2013 (nur Menü),
  Halbleiter und Elektronenröhren, Halbleiter und Elektronenröhren - Visio 2013, Integrierte Schaltkreise,
  Karten und Diagramme, Rotierende Geräte und mechanische Funktionen, Schalter und Relais,
  Schalter und Relais - Visio 2013 (nur Menü), Stecker und Anschlüsse,
  Telekommunikationsschaltung und Peripheriegerät, Transformatoren und Wicklungen, VHF-UHF-SHF, Wartungssymbole,
  Übertragungspfade (nur Menü), Übertragungspfade - Visio 2013 (nur Menü)
- **Konstruktion › Maschinenbau** (S 1104-132127): Federn und Lager, Geometrische Bemaßungen und Toleranzen,
  Pneumatik/Hydraulik - Geräte, Pneumatik/Hydraulik - Ventilanordnung, Pneumatik/Hydraulik - Ventile, Schrauben 1,
  Schrauben 2, Schweißsymbole
- **Konstruktion › Verfahrenstechnik** (S 1104-132145): Geräte - Behälter, Geräte - Pumpen, Geräte - Wärmetauscher,
  Instrumente, Prozessanmerkungen, Rohrleitungen, Ventile und Armaturen, Zubehör - Allgemein
- **Netzwerk** (S 1104-132208, S 1104-132221): Active Directory-Objekte, Active Directory-Standorte und -Dienste;
  27 AWS-Schablonen (AWS Allgemein, AWS AR VR, AWS Compute, AWS Endbenutzer-Computing, AWS Internet der Dinge,
  AWS Machine Learning, AWS Mobile, AWS Networking und Inhaltsbereitstellung, AWS Quantum-Technologien, AWS-Analysen,
  AWS-Anwendungsintegration, AWS-Blockchain, AWS-Container, AWS-Datenbank, AWS-Entwicklertools,
  AWS-Konformität der Sicherheitsidentität, AWS-Kostenmanagement, AWS-Kundenbefähigung, AWS-Kundenbindung,
  AWS-Management Governance, AWS-Mediendienste, AWS-Migrationsübertragung, AWS-Robotik, AWS-Satellite, AWS-Speicher,
  AWS-Spieltechnologie, AWS-Unternehmensanwendungen); 18 Azure-Schablonen (Azure App-Dienste, Azure Blockchain,
  Azure Compute, Azure DevOps, Azure Identity, Azure IoT, Azure KI und Maschinelles Lernen,
  Azure Management Governance, Azure Migrate, Azure Security, Azure Storage, Azure, allgemein, Azure, andere,
  Azure-Analysen, Azure-Container, Azure-Datenbanken, Azure-Integration, Azure-Netzwerk); Computer und Monitore,
  Computer und Monitore - 3D, Detailliertes Netzwerkdiagramm, Detailliertes Netzwerkdiagramm - 3D, Exchange-Objekte,
  Freistehende Gestelle, Gestellmontierte Geräte, Gestellmontierte Server, Gestellmontierte Server - 3D, LDAP-Objekte,
  Microsoft-Produkte, Netzwerk und Peripheriegeräte, Netzwerk und Peripheriegeräte - 3D (nur Menü), Netzwerkadressen,
  Netzwerkadressen - 3D, Netzwerkraumelemente, Netzwerksymbole, Netzwerksymbole - 3D, Server, Server - 3D
- **Pläne und Grundrisse › Bauplan** (S 1104-132248, S 1104-132303; Listenanfang am Bildrand angeschnitten):
  Alarmsteuerung und Zutrittskontrolle, Arbeitsbereiche, Arbeitsbereiche – Visio 2013 (nur Menü),
  Auslöser und Melder (nur Menü), Außen, Bad- und Küchenplan (nur Menü), Bad- und Küchenplan – Visio 2013 (nur Menü),
  Beweis, Bewässerung, Büroausstattung, Büroausstattung – Visio 2013 (nur Menü), Büromöbel,
  Büromöbel – Visio 2013 (nur Menü), Bürozubehör, Bürozubehör – Visio 2013 (nur Menü),
  Elektrik und Telekommunikation, Fahrzeuge, Fahrzeuge – Visio 2013 (nur Menü), Gartenzubehör,
  Gartenzubehör – Visio 2013 (nur Menü), Gebäudekern, Gebäudekern – Visio 2013 (nur Menü), Haushaltsgeräte,
  Haushaltsgeräte – Visio 2013 (nur Menü), HKL-Elemente, HKL-Rohre, HKL-Steuerungen, HKL-Steuerungszubehör, Innen,
  Interessante Punkte, Interessante Punkte – Visio 2013 (nur Menü), Lagerhaus - Versand und Warenannahme, Möbel,
  Möbel – Visio 2013 (nur Menü), Parkplätze und Straßen, Parkplätze und Straßen – Visio 2013 (nur Menü), Pflanzen,
  Pflanzen – Visio 2013 (nur Menü), Produktionsstätte - Lagerhaltung und Verteilung,
  Produktionsstätte - Maschinen und Einrichtungen, Ressourcen, Rohre und Ventile - Rohre 1,
  Rohre und Ventile - Rohre 2, Rohre und Ventile - Ventile 1, Rohre und Ventile - Ventile 2, Sanitär,
  Schieber, Gitter und Diffusoren, Schränke, Schränke – Visio 2013 (nur Menü), Sport- und Freizeitanlagen,
  Sport- und Freizeitanlagen – Visio 2013 (nur Menü), Videoüberwachung, Wände, Rohbau und Strukturen,
  Wände, Türen und Fenster, Öffentliche Plätze, Öffentliche Plätze – Visio 2013 (nur Menü)
- **Pläne und Grundrisse › Karte** (S 1104-132317): Freizeitanlagen-Shapes, Shapes für markante Gebäude o.ä.,
  Stadtbahn-Shapes, Straßen-Shapes, Verkehrs-Shapes, Wegbeschreibung 3D-Shapes
- **Software und Datenbank › Datenbank** (S 1104-132405): Chen-Datenbanknotation, Crow's Foot-Datenbanknotation,
  Entitätsbeziehung, IDEF1X-Datenbanknotation, Objekt - relational, UML-Datenbanknotation
- **Software und Datenbank › Drahtmodell** (S 1104-132418): Allgemeine Symbole, Cursor und Gesten,
  Mobile Karten und Container, Mobile Medien, Mobile Navigation, Mobile Steuerelemente, Mobiler Text und Anmerkungen,
  Web- und Mediensymbole, Web-Navigation, Webcontainer, Webmedien, Websteuerelemente, Webtext und -anmerkungen
- **Software und Datenbank › Software** (S 1104-132434): Allgemeine Symbole, COM und OLE, Cursor,
  Datenflussdiagramm-Shapes, Dialoge, Gane-Sarson, Speicherobjekte, Sprachebenen-Shapes, Steuerelemente,
  Symbolleisten, UML-Aktivität, UML-Anwendungsfall, UML-Klasse, UML-Kommunikation, UML-Komponente, UML-Sequenz,
  UML-Statuscomputer, UML-Verteilung, Unternehmensanwendung, Web- und Mediensymbole
- **Software und Datenbank › Webdiagramm** (S 1104-132449): Konzeptionelle Website-Shapes, Websiteübersicht-Shapes
- **Nur in älteren Versionen** (SWDB, nicht im aktuellen Menü): Express-G, Jackson, ORM-Diagramm, ROOM
- **Terminplan** (S 1104-132502): Gantt-Diagramm-Shapes, Kalender-Shapes, PERT-Diagramm-Shapes, Zeitachsen-Shapes
- **Visio-Extras** (S 1104-132515): Anmerkungen, Bemaßung - Technik, Benutzerdefinierte Linienmuster (nur Menü),
  Benutzerdefinierte Muster - Skaliert, Benutzerdefinierte Muster - Unskaliert, Beschriftungen, Ornamente, Symbole,
  Symbolsätze, Titelblöcke, Verbinder, Zeichentools

„Allgemeine Symbole“ und „Web- und Mediensymbole“ gibt es unter Drahtmodell und unter Software als zwei verschiedene
Schablonen mit gleichem Namen; unten sind sie mit „(Drahtmodell)“ bzw. „(Software)“ unterschieden.

### 4.2 Übersicht aller Schablonen

Prio folgt dem Richtwert; nach Entscheidung F11 sind die Rack-Schablonen SH-145 bis SH-147 P1. Weitere Abweichungen mit Begründung: SH-036 EPC-Diagramm-Shapes P2 (die EPK ist im deutschsprachigen
Raum verbreitet; Inhalt muss nachgeliefert werden), SH-049 Karten ist der eine P2-Organigramm-Stil (schlichtester,
zeitloser Stil; die stil-unabhängigen Hilfs-Shapes kommen aus SH-045), SH-063 Halbleiter P2 (Dioden und Transistoren
gehören zum Grundbestand jedes Schaltplans), SH-266 Zeitachsen P2 (häufig gebraucht, einfache Formen), SH-150
Microsoft-Produkte Prio „–“ (Markenlogos, werden nicht nachgebaut), SH-136 Azure-Container ist kein Download, sondern
generische Container-Rahmen (P3, neu zu bauen).

| ID | Schablone | Kategorie | Kennzeichnung | Shapes | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|---|---|
| SH-001 | 3D-Block-Shapes | Allgemein | Generisch | 19 | S 1104-133305 | P3 | App | neu |
| SH-002 | Blöcke | Allgemein | Generisch | 37 | S 1104-133322, S 1105-161433 | P1 | App | neu |
| SH-003 | Dekorative Shapes | Allgemein | Generisch | 17 | S 1104-131524, S 1104-131818 | P2 | App | neu |
| SH-004 | Diagramm- und Mathematik-Shapes | Allgemein | gemischt: Norm (Rechenzeichen) für Plus bis Ungleich, sonst Generisch | 10 | S 1104-131524, S 1104-131831, S 1104-133242 | P2 | App | neu |
| SH-005 | Perspektivische Blöcke | Allgemein | Generisch | 19 | S 1104-133340, S 1105-161419 | P3 | App | neu |
| SH-006 | Standard-Shapes | Allgemein | Generisch | 48 | S 1104-131524, S 1104-131657, S 1104-131740 | P1 | App | neu |
| SH-007 | Venn-Diagramm-Shapes | Allgemein | Generisch | 12 | S 1104-133402, S 1105-161409 | P2 | App | neu |
| SH-008 | Abschlusszeichen - SharePoint 2010-Workflow | Flussdiagramm | Visio-Design | 2 | S 1104-133757 | P3 | App | neu |
| SH-009 | Abteilung | Flussdiagramm | Visio-Design | 43 | S 1105-161358 | P2 | App | neu |
| SH-010 | Abteilung - 3D | Flussdiagramm | Visio-Design | 43 | S 1105-161346 | P3 | App | neu |
| SH-011 | Aktionen - SharePoint 2010-Workflow | Flussdiagramm | Visio-Design (vermutet) | –, nicht im Material | S 1104-131922 (Menü) | P3 | App | neu |
| SH-012 | Aktionen – SharePoint-2013-Workflow | Flussdiagramm | Visio-Design (vermutet) | –, nicht im Material | S 1104-131922 (Menü) | P3 | App | neu |
| SH-013 | Bedingungen - SharePoint 2010-Workflow | Flussdiagramm | Visio-Design (vermutet) | –, nicht im Material | S 1104-131922 (Menü) | P3 | App | neu |
| SH-014 | Bedingungen – SharePoint-2013-Workflow | Flussdiagramm | Visio-Design (vermutet) | –, nicht im Material | S 1104-131922 (Menü) | P3 | App | neu |
| SH-015 | BPMN-Standard-Shapes | Flussdiagramm | Norm: BPMN 2.0 | 16 | S 1105-161333 | P2 | App | neu |
| SH-016 | Funktionsübergreifende Flussdiagramm-Shapes | Flussdiagramm | Generisch | 4 | S 1105-161322 | P1 | App | neu |
| SH-017 | IDEF0-Diagramm-Shapes | Flussdiagramm | Norm: IDEF0 (FIPS 183 / IEEE 1320.1) | 9 | S 1105-161308 | P3 | App | neu |
| SH-018 | Komponenten – SharePoint-2013-Workflow | Flussdiagramm | Visio-Design (vermutet) | –, nicht im Material | S 1104-131922 (Menü) | P3 | App | neu |
| SH-019 | Pfeil-Shapes | Flussdiagramm (auch Geschäft › Geschäftsprozess) | Generisch | 20 | S 1104-131524, S 1104-131802 | P1 | App | neu |
| SH-020 | SDL-Diagramm-Shapes | Flussdiagramm | Norm: SDL (ITU-T Z.100) | 24 | S 1105-161254 | P3 | App | neu |
| SH-021 | Standardflussdiagramm-Shapes | Flussdiagramm | Norm: ISO 5807 | 14 | S 1105-161241 | P1 | App | neu |
| SH-022 | Verschiedene Flussdiagramm-Shapes | Flussdiagramm | gemischt: Norm: ISO 5807 (Grund- und Datenträgersymbole) und SDL-/Verknüpfungssymbole, Rest Generisch | 56 | S 1105-161229 | P3 | App | neu |
| SH-023 | Workflowobjekte | Flussdiagramm | Visio-Design | 25 | S 1105-161216 | P2 | App | neu |
| SH-024 | Workflowobjekte - 3D | Flussdiagramm | Visio-Design | 26 | S 1105-161206 | P3 | App | neu |
| SH-025 | Workflowschritte | Flussdiagramm | Visio-Design | 22 | S 1105-161154 | P2 | App | neu |
| SH-026 | Workflowschritte - 3D | Flussdiagramm | Visio-Design | 15 | S 1105-161144 | P3 | App | neu |
| SH-027 | Brainstorming-Shapes | Geschäft › Brainstorming | Generisch | 5 | S 1105-104416 | P3 | App | neu |
| SH-028 | Legenden-Shapes | Geschäft › Brainstorming | Visio-Design | 24 | S 1105-104405 | P3 | App | neu |
| SH-029 | Diagramm-Shapes | Geschäft › Diagramme | Generisch | 40 | S 1105-104634 | P3 | App | neu |
| SH-030 | Marketingdiagramme | Geschäft › Diagramme | Generisch | 30 | S 1105-104621 | P2 | App | neu |
| SH-031 | Matrixdiagramm-Shapes | Geschäft › Diagramme | Generisch | 15 | S 1105-104608 | P2 | App | neu |
| SH-032 | Pyramiden-Shapes | Geschäft › Diagramme | Generisch | 11 | S 1105-104442 | P2 | App | neu |
| SH-033 | Zyklusdiagramm-Shapes | Geschäft › Diagramme | Generisch | 20 | S 1105-104428 | P2 | App | neu |
| SH-034 | Arbeitsflussdiagramm-Shapes | Geschäft › Geschäftsprozess | gemischt: Visio-Design (Abteilungs-Icons), Generisch (Verbinder, Seitenverweise) | 42 | S 1105-110618 | P3 | App | neu |
| SH-035 | Auditdiagramm-Shapes | Geschäft › Geschäftsprozess | gemischt: Norm: ISO 5807 (Grundsymbole), Generisch (Markierungen, Titelblock, Notiz) | 33 | S 1105-110608 | P3 | App | neu |
| SH-036 | EPC-Diagramm-Shapes | Geschäft › Geschäftsprozess | Norm: EPK (vermutet) | –, nicht im Material | S 1104-132020 (Menü) | P2 | App | neu |
| SH-037 | Fehlerstrukturanalyse-Shapes | Geschäft › Geschäftsprozess | Norm: IEC 61025 | 13 | S 1105-110549 | P3 | App | neu |
| SH-038 | ITIL-Shapes | Geschäft › Geschäftsprozess | Visio-Design | 11 | S 1105-110535 | P3 | App | neu |
| SH-039 | Kompatibilitäts-Shapes | Geschäft › Geschäftsprozess | Visio-Design | 7 | S 1105-110523 | P3 | App | neu |
| SH-040 | Shapes für Six Sigma-Flussdiagramme | Geschäft › Geschäftsprozess | Norm: ISO 5807 | 14 | S 1105-105022 | P3 | App | neu |
| SH-041 | Shapes für Ursache/Wirkung-Diagramme | Geschäft › Geschäftsprozess | Generisch | 12 | S 1105-104956 | P2 | App | neu |
| SH-042 | Six-Sigma-House of Quality-Shapes | Geschäft › Geschäftsprozess | Generisch | 14 | S 1105-104721 | P3 | App | neu |
| SH-043 | TQM-Diagramm-Shapes | Geschäft › Geschäftsprozess | gemischt: Norm: ASME-Arbeitsablaufsymbole (Vorgang, Prüfung, Transport, Verzögerung, Lager), Rest Generisch | 52 | S 1105-104657 | P3 | App | neu |
| SH-044 | Wertstromzuordnungs-Shapes | Geschäft › Geschäftsprozess | Norm: Wertstrom-Notation (VSM, de facto) | 29 | S 1105-104645 | P3 | App | neu |
| SH-045 | Augenzahl - Organigramm-Shapes | Geschäft › Organigramm | Visio-Design | 15 | S 1105-114551 | P3 | App | neu |
| SH-046 | Blütenblätter - Organigramm-Shapes | Geschäft › Organigramm | Visio-Design | 15 | S 1105-114531 | P3 | App | neu |
| SH-047 | Eingebunden - Organigramm-Shapes | Geschäft › Organigramm | Visio-Design | 15 | S 1105-114517 | P3 | App | neu |
| SH-048 | Gürtel - Organigramm-Shapes | Geschäft › Organigramm | Visio-Design | 15 | S 1105-114502 | P3 | App | neu |
| SH-049 | Karten – Organigramm-Shapes | Geschäft › Organigramm | Visio-Design | 6 | S 1105-114445 | P2 | App | neu |
| SH-050 | Kerbe - Organigramm-Shapes | Geschäft › Organigramm | Visio-Design | 15 | S 1105-112213 | P3 | App | neu |
| SH-051 | Münze - Organigramm-Shapes | Geschäft › Organigramm | Visio-Design | 15 | S 1105-112156 | P3 | App | neu |
| SH-052 | Perspektive - Organigramm-Shapes | Geschäft › Organigramm | Visio-Design | 15 | S 1105-112138 | P3 | App | neu |
| SH-053 | Shapetakulär - Organigramm-Shapes | Geschäft › Organigramm | Visio-Design | 15 | S 1105-112127 | P3 | App | neu |
| SH-054 | Stein - Organigramm-Shapes | Geschäft › Organigramm | Visio-Design | 15 | S 1105-112112 | P3 | App | neu |
| SH-055 | Tafel - Organigramm-Shapes | Geschäft › Organigramm | Visio-Design | 15 | S 1105-112054 | P3 | App | neu |
| SH-056 | Pivotdiagramm-Shapes | Geschäft › Pivotdiagramm | Generisch | 1 | S 1105-114604 | P3 | App | neu |
| SH-057 | Analog-Digital-Logisch | Konstruktion › Elektrotechnik | Norm: IEC 60617-12/-13, IEEE 91 | 26 | S 1105-104341 | P3 | App | neu |
| SH-058 | Baugruppen | Konstruktion › Elektrotechnik | Norm: IEC 60617 / IEEE 315 | 28 | S 1105-104353 | P3 | App | neu |
| SH-059 | Beschriftungssymbole | Konstruktion › Elektrotechnik | Norm: IEC 60617-2/-6 | 25 | S 1105-104324 | P3 | App | neu |
| SH-060 | Beschriftungssymbole - Visio 2013 | Konstruktion › Elektrotechnik | Norm: IEC 60617 (vermutet) | –, nicht im Material | S 1104-132112 (Menü) | P3 | App | neu |
| SH-061 | Grundlegende Elemente | Konstruktion › Elektrotechnik | Norm: IEC 60617-2/-4/-6, IEEE 315 | 56 | S 1105-104202 | P2 | App | neu |
| SH-062 | Grundlegende Elemente - Visio 2013 | Konstruktion › Elektrotechnik | Norm: IEC 60617 (vermutet) | –, nicht im Material | S 1104-132112 (Menü) | P3 | App | neu |
| SH-063 | Halbleiter und Elektronenröhren | Konstruktion › Elektrotechnik | Norm: IEC 60617-5 | 30 | S 1105-104146 | P2 | App | neu |
| SH-064 | Halbleiter und Elektronenröhren - Visio 2013 | Konstruktion › Elektrotechnik | Norm: IEC 60617-5 | 27 | S 1105-104133 | P3 | App | neu |
| SH-065 | Integrierte Schaltkreise | Konstruktion › Elektrotechnik | Norm: IEC 60617-12, IEEE 91 | 35 | S 1105-104119 | P3 | App | neu |
| SH-066 | Karten und Diagramme | Konstruktion › Elektrotechnik | Norm: IEC 60617-11 | 36 | S 1105-104106 | P3 | App | neu |
| SH-067 | Rotierende Geräte und mechanische Funktionen | Konstruktion › Elektrotechnik | Norm: IEC 60617-6 | 19 | S 1105-104053 | P3 | App | neu |
| SH-068 | Schalter und Relais | Konstruktion › Elektrotechnik | Norm: IEC 60617-7, IEEE 315 | 50 | S 1105-104033 | P3 | App | neu |
| SH-069 | Schalter und Relais - Visio 2013 | Konstruktion › Elektrotechnik | Norm: IEC 60617 (vermutet) | –, nicht im Material | S 1104-132112 (Menü) | P3 | App | neu |
| SH-070 | Stecker und Anschlüsse | Konstruktion › Elektrotechnik | Norm: IEC 60617-3, IEEE 315 | 28 | S 1105-104018 | P3 | App | neu |
| SH-071 | Telekommunikationsschaltung und Peripheriegerät | Konstruktion › Elektrotechnik | Norm: IEC 60617, IEEE 315 | 28 | S 1105-104006 | P3 | App | neu |
| SH-072 | Transformatoren und Wicklungen | Konstruktion › Elektrotechnik | Norm: IEC 60617-6 | 21 | S 1105-103955 | P3 | App | neu |
| SH-073 | VHF-UHF-SHF | Konstruktion › Elektrotechnik | Norm: IEC 60617, IEEE 315 | 32 | S 1105-103943 | P3 | App | neu |
| SH-074 | Wartungssymbole | Konstruktion › Elektrotechnik | Norm: IEEE 315 | 13 | S 1105-103931 | P3 | App | neu |
| SH-075 | Übertragungspfade | Konstruktion › Elektrotechnik | Norm: IEC 60617 (vermutet) | –, nicht im Material | S 1104-132112 (Menü) | P3 | App | neu |
| SH-076 | Übertragungspfade - Visio 2013 | Konstruktion › Elektrotechnik | Norm: IEC 60617 (vermutet) | –, nicht im Material | S 1104-132112 (Menü) | P3 | App | neu |
| SH-077 | Federn und Lager | Konstruktion › Maschinenbau | Norm: ISO 8826, ISO 128 | 27 | S 1105-103549 | P3 | App | neu |
| SH-078 | Geometrische Bemaßungen und Toleranzen | Konstruktion › Maschinenbau | Norm: ISO 1101 | 37 | S 1105-103537 | P3 | App | neu |
| SH-079 | Pneumatik/Hydraulik - Geräte | Konstruktion › Maschinenbau | Norm: ISO 1219-1 | 35 | S 1105-103438 | P3 | App | neu |
| SH-080 | Pneumatik/Hydraulik - Ventilanordnung | Konstruktion › Maschinenbau | Norm: ISO 1219-1 | 52 | S 1105-103425 | P3 | App | neu |
| SH-081 | Pneumatik/Hydraulik - Ventile | Konstruktion › Maschinenbau | Norm: ISO 1219-1 | 21 | S 1105-103327 | P3 | App | neu |
| SH-082 | Schrauben 1 | Konstruktion › Maschinenbau | Norm: vereinfachte Normteil-Darstellung (ISO 128, ISO 6410) | 36 | S 1105-103308 | P3 | App | neu |
| SH-083 | Schrauben 2 | Konstruktion › Maschinenbau | Norm: vereinfachte Normteil-Darstellung (ISO 128, ISO 6410) | 27 | S 1105-103255 | P3 | App | neu |
| SH-084 | Schweißsymbole | Konstruktion › Maschinenbau | Norm: ISO 2553 | 27 | S 1105-103245 | P3 | App | neu |
| SH-085 | Geräte - Behälter | Konstruktion › Verfahrenstechnik | Norm: ISO 10628 | 23 | S 1105-103230 | P3 | App | neu |
| SH-086 | Geräte - Pumpen | Konstruktion › Verfahrenstechnik | Norm: ISO 10628 | 26 | S 1105-103219 | P3 | App | neu |
| SH-087 | Geräte - Wärmetauscher | Konstruktion › Verfahrenstechnik | Norm: ISO 10628 | 28 | S 1105-103207 | P3 | App | neu |
| SH-088 | Instrumente | Konstruktion › Verfahrenstechnik | Norm: ISA 5.1, IEC 62424 | 50 | S 1105-103133 | P3 | App | neu |
| SH-089 | Prozessanmerkungen | Konstruktion › Verfahrenstechnik | gemischt: Generisch (Text, Beschriftungen, Listen), Norm: ISO 10628 (Schnittstellen) | 22 | S 1105-103122 | P3 | App | neu |
| SH-090 | Rohrleitungen | Konstruktion › Verfahrenstechnik | Norm: ISO 10628 | 6 | S 1105-103109 | P3 | App | neu |
| SH-091 | Ventile und Armaturen | Konstruktion › Verfahrenstechnik | Norm: ISO 10628, ISA 5.1 | 50 | S 1105-103057 | P3 | App | neu |
| SH-092 | Zubehör - Allgemein | Konstruktion › Verfahrenstechnik | Norm: ISO 10628 | 40 | S 1105-103043 | P3 | App | neu |
| SH-093 | Active Directory-Objekte | Netzwerk | Visio-Design | 21 | S 1105-102933 | P3 | App | neu |
| SH-094 | Active Directory-Standorte und -Dienste | Netzwerk | gemischt: Visio-Design (3D-Symbole, Client), Generisch (2D-Grundformen) | 15 | S 1105-102806 | P3 | App | neu |
| SH-095 | AWS Allgemein | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 22 | S 1105-102754 | P2 | App | neu |
| SH-096 | AWS AR VR | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 2 | S 1105-102742 | P2 | App | neu |
| SH-097 | AWS Compute | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 71 | S 1105-102730 | P2 | App | neu |
| SH-098 | AWS Endbenutzer-Computing | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 5 | S 1105-102717 | P2 | App | neu |
| SH-099 | AWS Internet der Dinge | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 60 | S 1105-102703 | P2 | App | neu |
| SH-100 | AWS Machine Learning | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 29 | S 1105-102637 | P2 | App | neu |
| SH-101 | AWS Mobile | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 7 | S 1105-102624 | P2 | App | neu |
| SH-102 | AWS Networking und Inhaltsbereitstellung | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 36 | S 1105-102610 | P2 | App | neu |
| SH-103 | AWS Quantum-Technologien | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 2 | S 1105-102303 | P2 | App | neu |
| SH-104 | AWS-Analysen | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 28 | S 1105-102555 | P2 | App | neu |
| SH-105 | AWS-Anwendungsintegration | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 18 | S 1105-102250 | P2 | App | neu |
| SH-106 | AWS-Blockchain | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 4 | S 1105-102541 | P2 | App | neu |
| SH-107 | AWS-Container | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 18 | S 1105-102223 | P2 | App | neu |
| SH-108 | AWS-Datenbank | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 40 | S 1105-102208 | P2 | App | neu |
| SH-109 | AWS-Entwicklertools | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 13 | S 1105-102526 | P2 | App | neu |
| SH-110 | AWS-Konformität der Sicherheitsidentität | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 36 | S 1105-102153 | P2 | App | neu |
| SH-111 | AWS-Kostenmanagement | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 6 | S 1105-102507 | P2 | App | neu |
| SH-112 | AWS-Kundenbefähigung | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 5 | S 1105-102138 | P2 | App | neu |
| SH-113 | AWS-Kundenbindung | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 5 | S 1105-102122 | P2 | App | neu |
| SH-114 | AWS-Management Governance | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 50 | S 1105-102315 | P2 | App | neu |
| SH-115 | AWS-Mediendienste | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 9 | S 1105-102109 | P2 | App | neu |
| SH-116 | AWS-Migrationsübertragung | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 14 | S 1105-102001 | P2 | App | neu |
| SH-117 | AWS-Robotik | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 6 | S 1105-101948 | P2 | App | neu |
| SH-118 | AWS-Satellite | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 2 | S 1105-101935 | P2 | App | neu |
| SH-119 | AWS-Speicher | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 35 | S 1105-101659 | P2 | App | neu |
| SH-120 | AWS-Spieltechnologie | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 2 | S 1105-101519 | P2 | App | neu |
| SH-121 | AWS-Unternehmensanwendungen | Netzwerk › AWS | AWS-Original (CC BY-ND 2.0) | 4 | S 1105-101507 | P2 | App | neu |
| SH-122 | Azure App-Dienste | Netzwerk › Azure | Azure-Download | 9 | S 1105-101453 | – | App | – (Download) |
| SH-123 | Azure Blockchain | Netzwerk › Azure | Azure-Download | 1 | S 1105-101439 | – | App | – (Download) |
| SH-124 | Azure Compute | Netzwerk › Azure | Azure-Download | 20 | S 1105-101426 | – | App | – (Download) |
| SH-125 | Azure DevOps | Netzwerk › Azure | Azure-Download | 4 | S 1105-101405 | – | App | – (Download) |
| SH-126 | Azure Identity | Netzwerk › Azure | Azure-Download | 8 | S 1105-101347 | – | App | – (Download) |
| SH-127 | Azure IoT | Netzwerk › Azure | Azure-Download | 7 | S 1105-101334 | – | App | – (Download) |
| SH-128 | Azure KI und Maschinelles Lernen | Netzwerk › Azure | Azure-Download | 2 | S 1105-101321 | – | App | – (Download) |
| SH-129 | Azure Management Governance | Netzwerk › Azure | Azure-Download | 21 | S 1105-101304 | – | App | – (Download) |
| SH-130 | Azure Migrate | Netzwerk › Azure | Azure-Download | 3 | S 1105-101246 | – | App | – (Download) |
| SH-131 | Azure Security | Netzwerk › Azure | Azure-Download | 5 | S 1105-101234 | – | App | – (Download) |
| SH-132 | Azure Storage | Netzwerk › Azure | Azure-Download | 13 | S 1105-101208 | – | App | – (Download) |
| SH-133 | Azure, allgemein | Netzwerk › Azure | Azure-Download | 83 | S 1105-101154 | – | App | – (Download) |
| SH-134 | Azure, andere | Netzwerk › Azure | gemischt: Azure-Download (9 Icons), Generisch (6 Container-Rahmen) | 15 | S 1105-101126 | – | App | – (Download); Container wie SH-136 |
| SH-135 | Azure-Analysen | Netzwerk › Azure | Azure-Download | 9 | S 1105-101111 | – | App | – (Download) |
| SH-136 | Azure-Container | Netzwerk › Azure | Generisch | 6 | S 1104-145108 | P3 | App | neu |
| SH-137 | Azure-Datenbanken | Netzwerk › Azure | Azure-Download | 18 | S 1104-145049 | – | App | – (Download) |
| SH-138 | Azure-Integration | Netzwerk › Azure | Azure-Download | 11 | S 1104-141319 | – | App | – (Download) |
| SH-139 | Azure-Netzwerk | Netzwerk › Azure | Azure-Download | 25 | S 1104-141307 | – | App | – (Download) |
| SH-140 | Computer und Monitore | Netzwerk | Visio-Design | 12 | S 1104-141243, S Übersicht | P1 | App | teilweise (JFNetworker-Symbole) |
| SH-141 | Computer und Monitore - 3D | Netzwerk | Visio-Design | 10 | S 1104-141211 | P3 | App | neu |
| SH-142 | Detailliertes Netzwerkdiagramm | Netzwerk | Visio-Design | 15 | S 1104-140835, S Übersicht | P1 | App | teilweise (JFNetworker-Symbole) |
| SH-143 | Detailliertes Netzwerkdiagramm - 3D | Netzwerk | Visio-Design | 16 | S 1104-140818 | P3 | App | neu |
| SH-144 | Exchange-Objekte | Netzwerk | Visio-Design | 17 | S 1104-135400, S 1104-135605 | P3 | App | neu |
| SH-145 | Freistehende Gestelle | Netzwerk | Visio-Design | 9 | S 1104-135343 | P1 | App | neu |
| SH-146 | Gestellmontierte Geräte | Netzwerk | Visio-Design | 19 | S 1104-135329 | P1 | App | neu |
| SH-147 | Gestellmontierte Server | Netzwerk | Visio-Design | 17 | S 1104-135314, S Übersicht | P1 | App | neu |
| SH-148 | Gestellmontierte Server - 3D | Netzwerk | Visio-Design | 17 | S 1104-135224 | P3 | App | neu |
| SH-149 | LDAP-Objekte | Netzwerk | Visio-Design | 21 | S 1104-135208 | P3 | App | neu |
| SH-150 | Microsoft-Produkte | Netzwerk | Visio-Design (Markenlogos) | 22 | S 1104-135146 | – | App | – (nicht nachbauen) |
| SH-151 | Netzwerk und Peripheriegeräte | Netzwerk | Visio-Design | 32 | S 1104-135125, S Übersicht | P1 | App | teilweise (JFNetworker-Symbole) |
| SH-152 | Netzwerk und Peripheriegeräte - 3D | Netzwerk | Visio-Design (vermutet) | –, nicht im Material | S 1104-132221 (Menü) | P3 | App | neu |
| SH-153 | Netzwerkadressen | Netzwerk | Visio-Design | 8 | S 1104-135052, S Übersicht | P1 | App | neu |
| SH-154 | Netzwerkadressen - 3D | Netzwerk | Visio-Design | 8 | S 1104-135023 | P3 | App | neu |
| SH-155 | Netzwerkraumelemente | Netzwerk | Generisch | 4 | S 1104-135006 | P3 | App | neu |
| SH-156 | Netzwerksymbole | Netzwerk | Visio-Design | 24 | S 1104-134947, S Übersicht | P1 | App | teilweise (JFNetworker-Symbole) |
| SH-157 | Netzwerksymbole - 3D | Netzwerk | Visio-Design | 24 | S 1104-134837 | P3 | App | neu |
| SH-158 | Server | Netzwerk | Visio-Design | 17 | S 1104-134819, S Übersicht | P1 | App | teilweise (JFNetworker-Symbole) |
| SH-159 | Server - 3D | Netzwerk | Visio-Design | 17 | S 1104-134755 | P3 | App | neu |
| SH-160 | Alarmsteuerung und Zutrittskontrolle | Pläne und Grundrisse › Bauplan | Visio-Design | 15 | S 1104-134227 | P3 | App | neu |
| SH-161 | Arbeitsbereiche | Pläne und Grundrisse › Bauplan | Visio-Design | 16 | S 1105-115747 | P3 | App | neu |
| SH-162 | Arbeitsbereiche – Visio 2013 | Pläne und Grundrisse › Bauplan | Visio-Design (vermutet) | –, nicht im Material | S 1104-132248 (Menü) | P3 | App | neu |
| SH-163 | Auslöser und Melder | Pläne und Grundrisse › Bauplan | Norm: Brandmelde-Plansymbole (vermutet) | –, nicht im Material | S 1104-132248 (Menü) | P3 | App | neu |
| SH-164 | Außen | Pläne und Grundrisse › Bauplan | Visio-Design | 28 | S 1105-115735 | P3 | App | neu |
| SH-165 | Bad- und Küchenplan | Pläne und Grundrisse › Bauplan | Visio-Design (vermutet) | –, nicht im Material | S 1104-132248 (Menü) | P3 | App | neu |
| SH-166 | Bad- und Küchenplan – Visio 2013 | Pläne und Grundrisse › Bauplan | Visio-Design (vermutet) | –, nicht im Material | S 1104-132248 (Menü) | P3 | App | neu |
| SH-167 | Beweis | Pläne und Grundrisse › Bauplan | Visio-Design | 40 | S 1105-115713 | P3 | App | neu |
| SH-168 | Bewässerung | Pläne und Grundrisse › Bauplan | Generisch | 11 | S 1105-115700 | P3 | App | neu |
| SH-169 | Büroausstattung | Pläne und Grundrisse › Bauplan | Visio-Design | 25 | S 1105-115640 | P3 | App | neu |
| SH-170 | Büroausstattung – Visio 2013 | Pläne und Grundrisse › Bauplan | Visio-Design (vermutet) | –, nicht im Material | S 1104-132248 (Menü) | P3 | App | neu |
| SH-171 | Büromöbel | Pläne und Grundrisse › Bauplan | Visio-Design | 29 | S 1105-115625 | P2 | App | neu |
| SH-172 | Büromöbel – Visio 2013 | Pläne und Grundrisse › Bauplan | Visio-Design (vermutet) | –, nicht im Material | S 1104-132248 (Menü) | P3 | App | neu |
| SH-173 | Bürozubehör | Pläne und Grundrisse › Bauplan | Visio-Design | 29 | S 1105-115611 | P3 | App | neu |
| SH-174 | Bürozubehör – Visio 2013 | Pläne und Grundrisse › Bauplan | Visio-Design (vermutet) | –, nicht im Material | S 1104-132248 (Menü) | P3 | App | neu |
| SH-175 | Elektrik und Telekommunikation | Pläne und Grundrisse › Bauplan | Norm: Installationsplan-Symbole (IEC 60617-11, hier ANSI-Stil) | 27 | S 1105-115545 | P3 | App | neu |
| SH-176 | Fahrzeuge | Pläne und Grundrisse › Bauplan | Visio-Design | 28 | S 1105-115558 | P3 | App | neu |
| SH-177 | Fahrzeuge – Visio 2013 | Pläne und Grundrisse › Bauplan | Visio-Design (vermutet) | –, nicht im Material | S 1104-132248 (Menü) | P3 | App | neu |
| SH-178 | Gartenzubehör | Pläne und Grundrisse › Bauplan | Visio-Design | 30 | S 1105-115532 | P3 | App | neu |
| SH-179 | Gartenzubehör – Visio 2013 | Pläne und Grundrisse › Bauplan | Visio-Design (vermutet) | –, nicht im Material | S 1104-132248 (Menü) | P3 | App | neu |
| SH-180 | Gebäudekern | Pläne und Grundrisse › Bauplan | gemischt: Norm: DIN 1356 (Treppen, Rampe, Aufzug, Schacht), Visio-Design (Sanitärobjekte, Kabinen) | 26 | S 1105-115519 | P3 | App | neu |
| SH-181 | Gebäudekern – Visio 2013 | Pläne und Grundrisse › Bauplan | gemischt (vermutet wie SH-180) | –, nicht im Material | S 1104-132248 (Menü) | P3 | App | neu |
| SH-182 | Haushaltsgeräte | Pläne und Grundrisse › Bauplan | Visio-Design | 21 | S 1105-115506 | P3 | App | neu |
| SH-183 | Haushaltsgeräte – Visio 2013 | Pläne und Grundrisse › Bauplan | Visio-Design (vermutet) | –, nicht im Material | S 1104-132248 (Menü) | P3 | App | neu |
| SH-184 | HKL-Elemente | Pläne und Grundrisse › Bauplan | Norm: HLK-Symbole (ASHRAE-/DIN-artig) | 19 | S 1105-115452 | P3 | App | neu |
| SH-185 | HKL-Rohre | Pläne und Grundrisse › Bauplan | Norm: Lüftungskanal-Darstellung (DIN 1946, ISO 6412-artig) | 18 | S 1105-115435 | P3 | App | neu |
| SH-186 | HKL-Steuerungen | Pläne und Grundrisse › Bauplan | Norm: GA-/HLK-Fühlersymbole | 24 | S 1105-115421 | P3 | App | neu |
| SH-187 | HKL-Steuerungszubehör | Pläne und Grundrisse › Bauplan | Norm: HLK-Regelschema-Symbole | 32 | S 1105-115407 | P3 | App | neu |
| SH-188 | Innen | Pläne und Grundrisse › Bauplan | Visio-Design | 58 | S 1105-115354 | P3 | App | neu |
| SH-189 | Interessante Punkte | Pläne und Grundrisse › Bauplan | gemischt: Visio-Design (Pin-Kacheln), Motive teils Norm: ISO 7001/7010 | 75 | S 1105-115339 | P3 | App | neu |
| SH-190 | Interessante Punkte – Visio 2013 | Pläne und Grundrisse › Bauplan | gemischt (vermutet wie SH-189) | –, nicht im Material | S 1104-132248 (Menü) | P3 | App | neu |
| SH-191 | Lagerhaus - Versand und Warenannahme | Pläne und Grundrisse › Bauplan | Visio-Design | 19 | S 1105-115324 | P3 | App | neu |
| SH-192 | Möbel | Pläne und Grundrisse › Bauplan | Visio-Design | 33 | S 1105-115312 | P2 | App | neu |
| SH-193 | Möbel – Visio 2013 | Pläne und Grundrisse › Bauplan | Visio-Design (vermutet) | –, nicht im Material | S 1104-132248 (Menü) | P3 | App | neu |
| SH-194 | Parkplätze und Straßen | Pläne und Grundrisse › Bauplan | Generisch | 25 | S 1105-115252 | P3 | App | neu |
| SH-195 | Parkplätze und Straßen – Visio 2013 | Pläne und Grundrisse › Bauplan | Generisch (vermutet) | –, nicht im Material | S 1104-132248 (Menü) | P3 | App | neu |
| SH-196 | Pflanzen | Pläne und Grundrisse › Bauplan | Visio-Design | 25 | S 1105-115239 | P3 | App | neu |
| SH-197 | Pflanzen – Visio 2013 | Pläne und Grundrisse › Bauplan | Visio-Design (vermutet) | –, nicht im Material | S 1104-132303 (Menü) | P3 | App | neu |
| SH-198 | Produktionsstätte - Lagerhaltung und Verteilung | Pläne und Grundrisse › Bauplan | Visio-Design | 27 | S 1105-115222 | P3 | App | neu |
| SH-199 | Produktionsstätte - Maschinen und Einrichtungen | Pläne und Grundrisse › Bauplan | Visio-Design | 25 | S 1105-115209 | P3 | App | neu |
| SH-200 | Ressourcen | Pläne und Grundrisse › Bauplan | Generisch | 9 | S 1105-115157 | P3 | App | neu |
| SH-201 | Rohre und Ventile - Rohre 1 | Pläne und Grundrisse › Bauplan | Norm: ISO 6412, ISO 10628 | 42 | S 1105-115144 | P3 | App | neu |
| SH-202 | Rohre und Ventile - Rohre 2 | Pläne und Grundrisse › Bauplan | Norm: DIN 2429, ISO 6412 | 28 | S 1105-115129 | P3 | App | neu |
| SH-203 | Rohre und Ventile - Ventile 1 | Pläne und Grundrisse › Bauplan | Norm: ISO 10628, DIN 2429 | 21 | S 1105-115117 | P3 | App | neu |
| SH-204 | Rohre und Ventile - Ventile 2 | Pläne und Grundrisse › Bauplan | Norm: ISO 10628, ISA 5.1 | 17 | S 1105-115104 | P3 | App | neu |
| SH-205 | Sanitär | Pläne und Grundrisse › Bauplan | Norm: Sanitär-Plansymbole (DIN 1986, DIN 1356) | 22 | S 1105-115049 | P3 | App | neu |
| SH-206 | Schieber, Gitter und Diffusoren | Pläne und Grundrisse › Bauplan | Norm: HLK-Plansymbole (Luftdurchlässe) | 6 | S 1105-115034 | P3 | App | neu |
| SH-207 | Schränke | Pläne und Grundrisse › Bauplan | Generisch | 25 | S 1105-115021 | P3 | App | neu |
| SH-208 | Schränke – Visio 2013 | Pläne und Grundrisse › Bauplan | Generisch (vermutet) | –, nicht im Material | S 1104-132303 (Menü) | P3 | App | neu |
| SH-209 | Sport- und Freizeitanlagen | Pläne und Grundrisse › Bauplan | Visio-Design | 24 | S 1105-114949 | P3 | App | neu |
| SH-210 | Sport- und Freizeitanlagen – Visio 2013 | Pläne und Grundrisse › Bauplan | Visio-Design (vermutet) | –, nicht im Material | S 1104-132303 (Menü) | P3 | App | neu |
| SH-211 | Videoüberwachung | Pläne und Grundrisse › Bauplan | Visio-Design | 16 | S 1105-114936 | P3 | App | neu |
| SH-212 | Wände, Rohbau und Strukturen | Pläne und Grundrisse › Bauplan | Norm: DIN 1356, ISO 7519 | 41 | S 1105-114920 | P3 | App | neu |
| SH-213 | Wände, Türen und Fenster | Pläne und Grundrisse › Bauplan | Norm: DIN 1356, ISO 7519 | 15 | S 1105-114907 | P2 | App | neu |
| SH-214 | Öffentliche Plätze | Pläne und Grundrisse › Bauplan | Visio-Design | 35 | S 1105-114854 | P3 | App | neu |
| SH-215 | Öffentliche Plätze – Visio 2013 | Pläne und Grundrisse › Bauplan | Visio-Design (vermutet) | –, nicht im Material | S 1104-132303 (Menü) | P3 | App | neu |
| SH-216 | Freizeitanlagen-Shapes | Pläne und Grundrisse › Karte | Visio-Design | 28 | S 1105-114841 | P3 | App | neu |
| SH-217 | Shapes für markante Gebäude o.ä. | Pläne und Grundrisse › Karte | Visio-Design | 32 | S 1105-114826 | P3 | App | neu |
| SH-218 | Stadtbahn-Shapes | Pläne und Grundrisse › Karte | Generisch | 8 | S 2024 | P3 | App | neu |
| SH-219 | Straßen-Shapes | Pläne und Grundrisse › Karte | Generisch | 14 | S 1105-114801 | P3 | App | neu |
| SH-220 | Verkehrs-Shapes | Pläne und Grundrisse › Karte | gemischt: Norm: Verkehrszeichen (Wiener Übereinkommen, StVO-ähnlich) für Schilder, Generisch (Straßen, Schienen) | 29 | S 1105-114747 | P3 | App | neu |
| SH-221 | Wegbeschreibung 3D-Shapes | Pläne und Grundrisse › Karte | Visio-Design | 39 | S 1105-114626 | P3 | App | neu |
| SH-222 | Chen-Datenbanknotation | Software und Datenbank › Datenbank | Norm: Chen-ER-Notation | 10 | S 1105-160625 | P2 | App | neu |
| SH-223 | Crow's Foot-Datenbanknotation | Software und Datenbank › Datenbank | Norm: Krähenfuß-Notation (Information Engineering) | 6 | S 1105-160613 | P2 | App | neu |
| SH-224 | Entitätsbeziehung | Software und Datenbank › Datenbank | Norm: ER-Notation (Visio-Variante, IDEF1X-artig) | 7 | S 1105-160602 | P2 | App | neu |
| SH-225 | IDEF1X-Datenbanknotation | Software und Datenbank › Datenbank | Norm: IDEF1X (FIPS 184) | 5 | S 1105-160528 | P2 | App | neu |
| SH-226 | Objekt - relational | Software und Datenbank › Datenbank | Norm: ER-Notation (Visio-Variante, IDEF1X-artig) | 10 | S 1105-160516 | P2 | App | neu |
| SH-227 | UML-Datenbanknotation | Software und Datenbank › Datenbank | Norm: UML 2.x | 5 | S 1105-160503 | P2 | App | neu |
| SH-228 | Allgemeine Symbole (Drahtmodell) | Software und Datenbank › Drahtmodell | Visio-Design (Motive allgemein üblich) | 79 | S 1105-160451 | P3 | App | neu |
| SH-229 | Cursor und Gesten | Software und Datenbank › Drahtmodell | Visio-Design (Motive allgemein üblich) | 26 | S 1105-160434 | P3 | App | neu |
| SH-230 | Mobile Karten und Container | Software und Datenbank › Drahtmodell | Generisch | 22 | S 1105-160422 | P3 | App | neu |
| SH-231 | Mobile Medien | Software und Datenbank › Drahtmodell | Generisch | 13 | S 1105-160408 | P3 | App | neu |
| SH-232 | Mobile Navigation | Software und Datenbank › Drahtmodell | Generisch | 15 | S 1105-160355 | P3 | App | neu |
| SH-233 | Mobile Steuerelemente | Software und Datenbank › Drahtmodell | Generisch | 20 | S 1105-160338 | P3 | App | neu |
| SH-234 | Mobiler Text und Anmerkungen | Software und Datenbank › Drahtmodell | Generisch | 19 | S 1105-160324 | P3 | App | neu |
| SH-235 | Web- und Mediensymbole (Drahtmodell) | Software und Datenbank › Drahtmodell | Visio-Design (Motive allgemein üblich) | 26 | S 1105-160224 | P3 | App | neu |
| SH-236 | Web-Navigation | Software und Datenbank › Drahtmodell | Generisch | 12 | S 1105-160212 | P3 | App | neu |
| SH-237 | Webcontainer | Software und Datenbank › Drahtmodell | Generisch | 15 | S 1105-160200 | P3 | App | neu |
| SH-238 | Webmedien | Software und Datenbank › Drahtmodell | Generisch | 12 | S 1105-160146 | P3 | App | neu |
| SH-239 | Websteuerelemente | Software und Datenbank › Drahtmodell | Generisch | 20 | S 1105-120256 | P3 | App | neu |
| SH-240 | Webtext und -anmerkungen | Software und Datenbank › Drahtmodell | Generisch | 17 | S 1105-120241 | P3 | App | neu |
| SH-241 | Allgemeine Symbole (Software) | Software und Datenbank › Software | Visio-Design (Motive allgemein üblich) | 22 | S 1105-120225 | P3 | App | neu |
| SH-242 | COM und OLE | Software und Datenbank › Software | Generisch | 16 | S 1105-120213 | P3 | App | neu |
| SH-243 | Cursor | Software und Datenbank › Software | Generisch | 14 | S 1105-120201 | P3 | App | neu |
| SH-244 | Datenflussdiagramm-Shapes | Software und Datenbank › Software | Norm: DFD nach Yourdon/DeMarco | 21 | S 1105-120149 | P3 | App | neu |
| SH-245 | Dialoge | Software und Datenbank › Software | Generisch | 18 | S 1104-134039 | P3 | App | neu |
| SH-246 | Gane-Sarson | Software und Datenbank › Software | Norm: Gane-Sarson-DFD | 4 | S 1105-120134 | P3 | App | neu |
| SH-247 | Speicherobjekte | Software und Datenbank › Software | Generisch | 14 | S 1105-120122 | P3 | App | neu |
| SH-248 | Sprachebenen-Shapes | Software und Datenbank › Software | Norm: Structure Chart (Yourdon/Constantine) | 12 | S 1105-120053 | P3 | App | neu |
| SH-249 | Steuerelemente | Software und Datenbank › Software | Generisch | 16 | S 1105-120038 | P3 | App | neu |
| SH-250 | Symbolleisten | Software und Datenbank › Software | gemischt: Generisch (Menüleisten), Visio-Design (Befehls-Icons, Motive frei) | 19 | S 1105-120023 | P3 | App | neu |
| SH-251 | UML-Aktivität | Software und Datenbank › Software | Norm: UML 2.x | 9 | S 1105-120011 | P2 | App | neu |
| SH-252 | UML-Anwendungsfall | Software und Datenbank › Software | Norm: UML 2.x | 8 | S 1105-115957 | P2 | App | neu |
| SH-253 | UML-Klasse | Software und Datenbank › Software | Norm: UML 2.x | 15 | S 1105-115943 | P2 | App | neu |
| SH-254 | UML-Kommunikation | Software und Datenbank › Software | Norm: UML 2.x | 5 | S 1105-115930 | P2 | App | neu |
| SH-255 | UML-Komponente | Software und Datenbank › Software | Norm: UML 2.x | 10 | S 1105-115918 | P2 | App | neu |
| SH-256 | UML-Sequenz | Software und Datenbank › Software | Norm: UML 2.x | 12 | S 1105-115906 | P2 | App | neu |
| SH-257 | UML-Statuscomputer | Software und Datenbank › Software | Norm: UML 2.x | 8 | S 1105-115853 | P2 | App | neu |
| SH-258 | UML-Verteilung | Software und Datenbank › Software | Norm: UML 2.x | 20 | S 1105-115841 | P2 | App | neu |
| SH-259 | Unternehmensanwendung | Software und Datenbank › Software | gemischt: Visio-Design (Rechner-Icons), Generisch (Grenze, Prozess, Objekt, Dokument, Schnittstellen) | 19 | S 1104-134154 | P3 | App | neu |
| SH-260 | Web- und Mediensymbole (Software) | Software und Datenbank › Software | Visio-Design (Motive allgemein üblich) | 36 | S 1105-115827 | P3 | App | neu |
| SH-261 | Konzeptionelle Website-Shapes | Software und Datenbank › Webdiagramm | Generisch | 19 | S 1105-115815 | P3 | App | neu |
| SH-262 | Websiteübersicht-Shapes | Software und Datenbank › Webdiagramm | Visio-Design | 41 | S 1105-115800 | P3 | App | neu |
| SH-263 | Gantt-Diagramm-Shapes | Terminplan | Generisch | 14 | S 1105-160711 | P3 | App | neu |
| SH-264 | Kalender-Shapes | Terminplan | gemischt: Generisch (Kalenderraster, Termine), Visio-Design (Ereignis- und Wetter-Icons) | 40 | S 1105-160700 | P3 | App | neu |
| SH-265 | PERT-Diagramm-Shapes | Terminplan | Generisch | 9 | S 1104-134406, S 1105-160649 | P3 | App | neu |
| SH-266 | Zeitachsen-Shapes | Terminplan | Generisch | 20 | S 1105-160635 | P2 | App | neu |
| SH-267 | Anmerkungen | Visio-Extras | gemischt: Norm: DIN 1356 / ISO 7519 (Schnitt-, Höhen-, Nordzeichen), Generisch (Text, Beschriftungen) | 39 | S 1105-161134 | P2 | App | neu |
| SH-268 | Bemaßung - Technik | Visio-Extras | Norm: ISO 129, DIN 406 | 26 | S 1105-161123 | P2 | App | neu |
| SH-269 | Benutzerdefinierte Linienmuster | Visio-Extras | Generisch (vermutet) | –, nicht im Material | S 1104-132515 (Menü) | P3 | App | neu |
| SH-270 | Benutzerdefinierte Muster - Skaliert | Visio-Extras | Generisch | 1 | S 1105-160944 | P3 | App | neu |
| SH-271 | Benutzerdefinierte Muster - Unskaliert | Visio-Extras | Generisch | 1 | S 1105-160933 | P3 | App | neu |
| SH-272 | Beschriftungen | Visio-Extras | Generisch | 38 | S 1105-160921 | P1 | App | neu |
| SH-273 | Ornamente | Visio-Extras | Visio-Design | 41 | S 1105-160908 | P3 | App | neu |
| SH-274 | Symbole | Visio-Extras | gemischt: Norm: ISO 7001/7010/7000 (Piktogramme), Visio-Design (Wetter); „Recycling 1“ nicht nachbauen | 34 | S 1105-160857 | P2 | App | neu |
| SH-275 | Symbolsätze | Visio-Extras | Generisch | 14 | S 1105-160844 | P3 | App | neu |
| SH-276 | Titelblöcke | Visio-Extras | Norm: ISO 5457, ISO 7200 | 29 | S 1105-160831 | P2 | App | neu |
| SH-277 | Verbinder | Visio-Extras | Generisch | 69 | S 1105-160807 | P1 | App | neu |
| SH-278 | Zeichentools | Visio-Extras | Generisch | 31 | S 1105-160819 | P3 | App | neu |
| SH-279 | Express-G | Software und Datenbank (ältere Version) | Norm: ISO 10303-11 (EXPRESS-G) | 17 | SWDB | P3 | App | neu |
| SH-280 | Jackson | Software und Datenbank (ältere Version) | Norm: JSP-Notation | 5 | SWDB | P3 | App | neu |
| SH-281 | ORM-Diagramm | Software und Datenbank (ältere Version) | Norm: ORM-/NIAM-Notation | 26 | SWDB | P3 | App | neu |
| SH-282 | ROOM | Software und Datenbank (ältere Version) | Norm: ROOM-Notation | 17 | SWDB | P3 | App | neu |

### 4.3 Allgemein

Für 4.3 bis 4.11 gilt: Shapes, die in vielen Schablonen wiederkehren, stehen nur beim ersten Vorkommen in einer
Tabelle und fehlen danach – das sind Dynamischer Verbinder, Verbinder Linie, Verbinder Linie/Kurve, On-Page-Referenz,
Off-Page-Referenz, Textblock 8/10/12 Pt., Horizontale Beschriftung, Rechtwinkl. Beschriftung sowie Hinweis/Notiz und
die Beziehungslinien in den UML-Schablonen. Die Zahl in 4.2 zählt sie mit. Weitere Doppelungen (Server-Rollen,
Organigramm-Stile, 2D/3D-Paare, „– Visio 2013“) sind bei der jeweiligen Schablone mit Verweis vermerkt. Durch „/“
verbundene Namen in einer Zeile sind gleichartige Varianten (z. B. Richtungen, Nummern) und zählen einzeln.

#### SH-001 3D-Block-Shapes

Dicke, schattierte Körper mit Tiefe für plakative Blockgrafiken und Pfeilketten. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Quadratischer Block | Quader mit sichtbarer Vorder- und Seitenfläche |
| Kreis | Zylinderscheibe mit Tiefe |
| Horizontaler Balken / Vertikaler Balken | waagrechter bzw. senkrechter Balken mit Tiefe |
| Pfeil nach links / rechts / oben / unten | dicker 3D-Pfeil in eine Richtung |
| Pfeil n. links/rechts / Pfeil n. oben/unten | 3D-Doppelpfeil waagrecht bzw. senkrecht |
| Pfeil n. oben / unten / links / rechts, offen | 3D-Pfeil ohne Endfläche, zum Aneinandersetzen |
| 90-Grad 1 / 2 / 3 / 4 | Winkelstück in vier Ausrichtungen für Ecken |
| Rahmen | 3D-Rahmen mit Öffnung |

#### SH-002 Blöcke

Bausteine für Blockdiagramme: Blöcke, Pfeile, Ringe und verzweigende Linien. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Feld | rechteckiger Block mit Text |
| Kreis | runder Block |
| Raute | Block in Rautenform |
| Feld mit Pfeil | Block mit angesetzter Pfeilspitze nach unten |
| 1D-Pfeil | linienartiger Pfeil, Endpunkte kleben an Shapes |
| 1D-Doppelpfeil | linienartiger Pfeil mit zwei Spitzen |
| 2D-Pfeil | flächiger Pfeil, frei skalierbar |
| 2D-Doppel… | flächiger Doppelpfeil |
| Auto-Höhe | Block, der seine Höhe dem Text anpasst |
| Auto-Feldb… (Auto-Feldbreite) | Block, der seine Breite dem Text anpasst |
| Krummer Pfeil | gebogener linienartiger Pfeil |
| 1D-Pfeil, offen | Linienpfeil ohne Abschluss am Schaftende |
| 2D-Pfeil, offen | Flächenpfeil ohne Abschluss am Schaftende |
| 3D-Feld | Block mit angedeuteter Tiefe |
| Balken offen/geschl. | Balken, Enden offen oder geschlossen umschaltbar |
| Schaltfläche | erhabener Block in Knopfoptik |
| Konzentrisc… Ebene 1 / 2 / 3 | Ringe gleicher Mitte zum Ineinanderlegen |
| Konzentrisc… Zentrum | Mittelscheibe der Ringe |
| Teilebene 1 / 2 / 3 / 4 | Viertelring-Segmente verschiedener Radien |
| 2 Zweige (Quadrat) | Verbinder, der sich rechtwinklig in zwei Äste teilt |
| 2 Zweige (schräg) | Verbinder mit zwei schrägen Ästen |
| Dynamischer Verbinder | Verbinder, der Shapes umfährt und sich neu verlegt |
| Zweige schräg | Verbinder mit mehreren schrägen Ästen |
| Zweige eckig | Verbinder mit mehreren rechtwinkligen Ästen |
| Verbinder Linie/Kurve | Verbinder wahlweise gerade oder gebogen |
| Gepunktete Linie | Linie mit Punkt-Strich-Muster |
| Pfeil - Mitte | Linie mit Pfeilspitze in der Mitte |
| Gebogener Pfeil | Bogenlinie mit Spitze |
| Punkt und Pfeil | Linie mit Punkt am Anfang, Spitze am Ende |
| Pfeil mit Endpunkten | Linie mit Spitze und Punkten |
| Einfache Spitze | Linie mit einer Pfeilspitze |
| Doppelspitze | Linie mit Spitzen an beiden Enden |

#### SH-003 Dekorative Shapes

Schmuckformen für Präsentationen, Banner und Hinweise. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Gefaltete Ecke | Notizblatt mit umgeknickter Ecke |
| Lachendes Gesicht | Smiley, Mundwinkel per Ziehpunkt änderbar |
| Neutrales Gesicht | Gesicht mit geradem Mund |
| Trauriges Gesicht | Gesicht mit hängenden Mundwinkeln |
| Herz | Herzform |
| Gewitterblitz | Blitzform |
| Sonne | Kreis mit Strahlen |
| Mond | Mondsichel, Breite per Ziehpunkt |
| Cloud | Wolkenform |
| Band nach oben | Spruchband mit Enden nach oben |
| Band nach unten | Spruchband mit Enden nach unten |
| Nach oben gekrümmt… (Band) | gewölbtes Spruchband nach oben |
| Nach unten gekrümmt… (Band) | gewölbtes Spruchband nach unten |
| Vertikaler Scrollen | senkrechte Schriftrolle |
| Horizontales Scrollen | waagrechte Schriftrolle |
| Welle | Fläche mit gewelltem Rand |
| Doppelte Welle | Fläche mit zwei Wellenrändern |

#### SH-004 Diagramm- und Mathematik-Shapes

Rechenzeichen und einfache Diagrammteile. Kennzeichnung: gemischt – Norm (Rechenzeichen) für Plus bis Ungleich,
sonst Generisch.

| Shape | Beschreibung |
|---|---|
| Venn-Diagr… | halbtransparenter Kreis für Mengendiagramme |
| Kreissegment | Tortenstück, Winkel per Ziehpunkt |
| Kreisdiagra… | Bogenstück eines Kreisdiagramms |
| Mehrzeilig | Linienzug mit mehreren Knickpunkten |
| Plus | Pluszeichen |
| Minus | Minuszeichen |
| Multiplikati… | Malkreuz |
| Divisionszei… | Geteiltzeichen mit Punkten |
| Gleichheitsz… | Gleichheitszeichen |
| Ungleich | durchgestrichenes Gleichheitszeichen |

#### SH-005 Perspektivische Blöcke

Körper in Zentralperspektive, die alle auf einen gemeinsamen Fluchtpunkt ausgerichtet sind. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Block | Quader, Tiefe zeigt zum Fluchtpunkt |
| Kreis | Zylinder in Perspektive |
| Pfeil nach links / rechts / Pfeil, nach oben / Pfeil nach unten | perspektivischer Pfeil in eine Richtung |
| Pfeil n. links/rechts / Pfeil n. oben/unten | perspektivischer Doppelpfeil |
| Loch | nach innen gehende Vertiefung |
| Flacher Block | flache Platte in Perspektive |
| 1D-Pfeil nach oben/unten / 1D-Pfeil links/rechts | Verbindungspfeil zwischen Blöcken |
| 90-Grad 1 / 2 / 3 / 4 | perspektivisches Winkelstück |
| Drahtmodell Block 1 / 2 | Quader nur aus Kanten |
| Fluchtpunkt | Steuer-Shape; verschieben ändert die Perspektive aller Blöcke |

#### SH-006 Standard-Shapes

Die Grundformen jedes Diagramms. Fast alle Formparameter (Ecken, Zacken, Neigung) hängen an Steuerpunkten.
Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Rechteck | Grundform |
| Ellipse | Grundform |
| Quadrat | Rechteck mit festem Seitenverhältnis |
| Kreis | Kreis mit festem Seitenverhältnis |
| Dreieck | gleichschenkliges Dreieck, Spitze verschiebbar |
| Rechtwinkli… Dreieck | Dreieck mit rechtem Winkel |
| Fünfeck / Sechseck / Siebeneck / Achteck / Zehneck | regelmäßige Vielecke |
| Dose | Zylinder mit Deckelellipse |
| Parallelogramm | Viereck mit schrägen Seiten, Neigung per Ziehpunkt |
| Trapez | Viereck mit schrägen Seiten oben schmaler |
| Raute | auf der Spitze stehendes Viereck |
| Kreuz | Pluskreuz, Balkenbreite per Ziehpunkt |
| Chevron | Winkelpfeil mit Kerbe |
| Würfel | Quader in Schrägansicht, Tiefe per Ziehpunkt |
| Stern mit 4 / 5 / 6 / 7 / 16 / 24 / 32 Zacken | Sterne, Zackentiefe per Ziehpunkt |
| Abgerundet… Rechteck | Rechteck mit Eckenradius per Ziehpunkt |
| Rechteck, 1 abgeschnitt… | eine Ecke abgeschrägt |
| Rechteck, auf gleicher Sei… (abgeschnitten) | zwei benachbarte Ecken abgeschrägt |
| Rechteck, diagonal li… (abgeschnitten) | zwei gegenüberliegende Ecken abgeschrägt |
| Rechteck, 1 abgerunde… | eine Ecke gerundet |
| Rechteck, auf gleicher Sei… (abgerundet) | zwei benachbarte Ecken gerundet |
| Rechteck, diagonal li… (abgerundet) | zwei gegenüberliegende Ecken gerundet |
| Rechteck, je eine Ecke… | eine Ecke gerundet, eine abgeschrägt |
| Rechteck, abgeschnitt… (2 Varianten) | mehrere Ecken abgeschrägt |
| Rechteck, abgerunde… | mehrere Ecken gerundet |
| Rahmen | Rechteckrahmen, Breite per Ziehpunkt |
| Rahmenecke | L-förmiges Eckstück eines Rahmens |
| L-Form | L-Profil, Schenkelbreite per Ziehpunkt |
| Diagonaler Streifen | schräges Band über eine Ecke |
| Plakette | Rechteck mit nach innen gewölbten Ecken |
| Donut | Ring, Ringbreite per Ziehpunkt |
| Verbotszeic… | Kreis mit Schrägbalken |
| Am Mittelpun… | Kreis, der von der Mitte aus aufgezogen wird |
| Runde Klamm… (links / rechts) | runde Klammer als Linie |
| Linke / Rechte geschweif… | geschweifte Klammer als Linie |

#### SH-007 Venn-Diagramm-Shapes

Mengendiagramme aus überlappenden, halbtransparenten Kreisen und Quadraten. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Venn-Diagr… | einzelner transparenter Kreis |
| Quadratisc… Venn-Diagr… | einzelnes transparentes Quadrat |
| Quadratisc… Venn-Diagr… (2) | zwei überlappende Quadrate |
| Venn-Diagr… aus 2 Men… | zwei überlappende Kreise |
| Venn-Diagr… aus 2 Men… (Variante) | zwei Kreise mit Beschriftungslinien |
| Venn-Diagr… aus 3 Men… (2 Varianten) | drei überlappende Kreise, mit/ohne Beschriftung |
| Vereinigun… von 2 Men… | beide Kreise vollständig hervorgehoben |
| Schnittmen… von 2 Men… | nur Überlappung hervorgehoben |
| Differenzm… von 2 Men… | ein Kreis ohne Überlappung hervorgehoben |
| Ergänzungs… von 2 Men… | Ergänzung (Komplement) hervorgehoben |
| Zwiebeldia… | ineinanderliegende Kreise |

### 4.4 Flussdiagramm

#### SH-008 Abschlusszeichen - SharePoint 2010-Workflow

Start- und Endmarken für SharePoint-2010-Workflows; produktgebunden. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Start | Kreis mit Abspiel-Dreieck, Beginn des Workflows |
| Terminieren | Kreis mit Quadrat, Ende des Workflows |

#### SH-009 Abteilung

Kleine Symbole für Unternehmensbereiche, z. B. für Abläufe zwischen Abteilungen. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Buchhaltung | Abteilung Rechnungswesen |
| Verbindlich… (Verbindlichkeiten) | Kreditorenbuchhaltung, ausgehende Zahlungen |
| Forderungen | Debitorenbuchhaltung, eingehende Zahlungen |
| Überwachu… (Überwachung) | Kontrolle, Revision |
| Bank | Bank als externe Stelle |
| Vorstand | Geschäftsleitung als Gremium |
| Cafeteria | Kantine |
| Copyshop | Kopier- und Druckstelle |
| Kundenserv… | Kundendienst |
| Rechenzent… | IT-Rechenzentrum |
| Abteilung | allgemeine Abteilung |
| Entwurf | Design/Entwurf |
| Konstruktion | Technik/Konstruktion |
| Anlagen | Anlagen- und Gebäudeverwaltung |
| Finanzwesen | Finanzabteilung |
| Unternehm… | Unternehmen bzw. Hauptsitz |
| Personalwe… | Personalabteilung |
| Information… | Informationsdienste/IT |
| Internation… Abteilung | Auslandsabteilung |
| Inventar | Bestandsverwaltung |
| Rechtsabtei… | Rechtsabteilung |
| Poststelle | Posteingang und -ausgang |
| Management | Führungsebene |
| Produktion | Fertigung |
| Marketing | Marketingabteilung |
| Fuhrpark | Fahrzeuge des Unternehmens |
| Operatives Geschäft | Betrieb/Operations |
| Verpackung | Verpackungsstelle |
| Lohnbuchh… | Lohn- und Gehaltsabrechnung |
| Publikation… | Veröffentlichungen, Presse |
| Einkauf | Beschaffung |
| Qualitätssic… | Qualitätssicherung |
| Eingang | Warenannahme |
| Empfang | Rezeption |
| Forschung und Entwi… | Forschung und Entwicklung |
| Vertrieb | Verkauf |
| Sicherheit | Werkschutz/Sicherheit |
| Versand | Warenausgang |
| Niederlassu… | Zweigstelle |
| Lieferanten | externe Zulieferer |
| Telekomm… | Telekommunikation |
| Finanzverw… | Finanzverwaltung/Treasury |
| Lagerhaus | Lager |

#### SH-010 Abteilung - 3D

Dieselben Abteilungen wie SH-009 als 3D-Clipart. Kennzeichnung: Visio-Design. Abweichungen: Einige Namen lauten anders
(Kopierzentrum statt Copyshop, Kundendienst, Finanzen, Hauptsitz, Auslandsabteilung, Postdienst, Fertigung, Vorgänge,
Lohn und Gehalt, Veröffentlichung, Warenannahme, Zulieferer); „Abteilung“ fehlt; der Dynamische Verbinder ist enthalten.

#### SH-011 Aktionen - SharePoint 2010-Workflow

Aktionsbausteine für SharePoint-2010-Workflows. Inhalt nicht im Material.

#### SH-012 Aktionen – SharePoint-2013-Workflow

Aktionsbausteine für SharePoint-2013-Workflows. Inhalt nicht im Material.

#### SH-013 Bedingungen - SharePoint 2010-Workflow

Bedingungsbausteine für SharePoint-2010-Workflows. Inhalt nicht im Material.

#### SH-014 Bedingungen – SharePoint-2013-Workflow

Bedingungsbausteine für SharePoint-2013-Workflows. Inhalt nicht im Material.

#### SH-015 BPMN-Standard-Shapes

Geschäftsprozesse in der Notation BPMN 2.0; Ereignis-Auslöser, Gateway-Art und Aufgabentyp werden über Shape-Daten
gewählt. Kennzeichnung: Norm: BPMN 2.0.

| Shape | Beschreibung |
|---|---|
| Aufgabe | abgerundetes Rechteck für eine Tätigkeit |
| Gateway | Raute für Verzweigung oder Zusammenführung |
| Zwischener… (Zwischenereignis) | Kreis mit doppeltem Rand |
| Endereignis | Kreis mit dickem Rand |
| Startereignis | Kreis mit dünnem Rand |
| Reduzierter Teilprozess | Aufgabe mit Plus-Marke, Inhalt eingeklappt |
| Erweiterter Teilprozess | aufgeklappter Teilprozess als Container |
| Textanmerk… | offene Klammer mit Anmerkungstext |
| Sequenzfluss | durchgezogener Pfeil für die Reihenfolge |
| Assoziation | gepunktete Linie zu Daten oder Anmerkung |
| Nachrichte… (Nachrichtenfluss) | gestrichelter Pfeil zwischen Pools |
| Nachricht | Briefumschlag |
| Datenobjekt | Blatt mit Eselsohr |
| Datenspeic… | Zylinder für dauerhaft gespeicherte Daten |
| Gruppe | gestrichelter Rahmen zur Gruppierung |
| Pool/Verant… | Pool mit Bahnen (Container) |

#### SH-016 Funktionsübergreifende Flussdiagramm-Shapes

Bahnen (Swimlanes) und Phasentrenner für Abläufe über mehrere Zuständige. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Verantwortl… | waagrechte Bahn, nimmt Shapes auf und wächst mit |
| Trennzeichen | senkrechter Phasentrenner über alle Bahnen |
| Verantwortl… (vertikal) | senkrechte Bahn |
| Trennzeichen (vertikal) | waagrechter Phasentrenner für senkrechte Bahnen |

#### SH-017 IDEF0-Diagramm-Shapes

Funktionsmodelle nach IDEF0 mit Eingängen, Steuerung, Ausgängen und Mechanismen. Kennzeichnung: Norm: IDEF0.

| Shape | Beschreibung |
|---|---|
| Aktivitätsfeld | Funktionskasten mit Nummernfeld |
| Beschriftung | Pfeilbeschriftung mit Zackenlinie zum Pfeil |
| Titelblock | IDEF0-Formblattkopf mit Knoten, Titel, Nummer |
| Textblock 8 Pt. | Textfeld in fester Schriftgröße |
| Knoten | Verzweigungspunkt |
| Verbinderli… | Linie, die sich in mehrere Äste teilt |
| Verbinder, 1 Abschnitt | gerader Pfeil mit einem Abschnitt |
| IDEF0-Verbi… | rechtwinkliger IDEF0-Pfeil |

#### SH-018 Komponenten – SharePoint-2013-Workflow

Bausteine für SharePoint-2013-Workflows. Inhalt nicht im Material.

#### SH-019 Pfeil-Shapes

Flächige Pfeile in vielen Formen; Schaftbreite und Spitze per Steuerpunkt. Auch unter Geschäft › Geschäftsprozess
gelistet. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Einfacher Pfeil | Blockpfeil in eine Richtung |
| Einfacher Doppelpfeil | Blockpfeil mit zwei Spitzen |
| Moderner Pfeil | schlanker Pfeil mit spitz zulaufender Spitze |
| Flexibler Pfeil | Pfeil mit biegbarem Verlauf |
| Gebogener Pfeil | Pfeil mit 90-Grad-Bogen |
| 180-Grad-P… | U-förmiger Wendepfeil |
| Rechteckiger Pfeil | rechtwinklig abknickender Pfeil |
| Nach rechts gekrümmt… | Bogenpfeil nach rechts |
| Nach links gekrümmt… | Bogenpfeil nach links |
| Mehrzeilig | Linienzug mit Pfeilspitze |
| Zickzackpfeil | Pfeil in Zickzackform |
| 2D-Zickzac… | flächiger Zickzackpfeil |
| Gestreifter Pfeil | Pfeil mit Querstreifen am Schaftende |
| Eingekerbter Pfeil | Pfeil mit Kerbe am Schaftende |
| Blockpfeil | fünfeckiger Pfeil (Hausform) |
| Gebogener Pfeil.18 | zweite Bogenpfeil-Variante |
| Pfeil in vier Richtungen | Kreuz mit vier Spitzen |
| Pfeil nach links, rech… | T-Form mit drei Spitzen |
| Pfeilblock nach lin… | Block mit Pfeilen nach links und rechts |
| Pfeilblock in vier Richtu… | Block mit Pfeilen in vier Richtungen |

#### SH-020 SDL-Diagramm-Shapes

Ablauf- und Zustandsdiagramme in der Specification and Description Language. Kennzeichnung: Norm: SDL (ITU-T Z.100).

| Shape | Beschreibung |
|---|---|
| Start | Startsymbol (abgerundetes Rechteck) |
| Start (variabel) | Startsymbol mit verstellbarer Form |
| Prozedur | Prozeduraufruf, Rechteck mit Seitenstrichen |
| Prozess (variabel) | Prozesskasten mit verstellbarer Form |
| Anfrage erstellen | Erzeugen einer Instanz |
| Alternative | Dreieck für eine Alternative |
| Zurück | Rückkehr aus der Prozedur, Kreis mit Kreuz |
| Entscheidung 1 / 2 | Entscheidung als Sechseck bzw. Raute |
| Meldung v. Anwender | Signal-Eingang, Rechteck mit Kerbe |
| Eingang von Steuerung | Eingang aus der Steuerung |
| Meldung an Anwender | Signal-Ausgang, Rechteck mit Spitze |
| Ausgang an Steuerung | Ausgang an die Steuerung |
| Speichern | Speichervorgang, Parallelogramm |
| On-Page-R… (On-Page-Referenz) | Kreis als Sprungstelle auf derselben Seite |
| Off-Page-R… (Off-Page-Referenz) | Fünfeck als Verweis auf eine andere Seite |
| Mehrere Dokumente | Stapel von Dokumenten |
| Dokument | Blatt mit gewellter Unterkante |
| Datenspeic… | Datenspeicher |
| Geteilter Vorgang | Vorgang mit Trennlinie |
| Geteiltes Ereignis | Ereignis mit Trennlinie |
| Ende | Endsymbol (Stadionform) |

#### SH-021 Standardflussdiagramm-Shapes

Die Grundsymbole für Programmablauf- und Prozessdiagramme; außerdem On-/Off-Page-Referenz (siehe SH-020).
Kennzeichnung: Norm: ISO 5807.

| Shape | Beschreibung |
|---|---|
| Prozess | Rechteck für einen Verarbeitungsschritt |
| Entscheidung | Raute mit Ja/Nein-Ausgängen |
| Teilprozess | Rechteck mit Doppelrand, vordefinierter Ablauf |
| Start/Ende | Stadionform für Beginn und Ende |
| Dokument | Blatt mit gewellter Unterkante |
| Daten | Parallelogramm für Ein- und Ausgabe |
| Datenbank | liegender Zylinder |
| Externe Daten | gespeicherte Daten mit gewölbten Seiten |
| Benutzerdef. 1 / 2 / 3 / 4 | frei belegbare Formen: Trapez, Trapez gedreht, Fahne, Sechseck |

#### SH-022 Verschiedene Flussdiagramm-Shapes

Sammlung weiterer Flussdiagramm-Symbole, darunter historische Datenträger und Verknüpfungen. Kennzeichnung:
gemischt – Norm: ISO 5807 (Grund- und Datenträgersymbole) und SDL-/Verknüpfungssymbole, Rest Generisch.

| Shape | Beschreibung |
|---|---|
| Geteilter Vorgang 2 | Vorgang mit Trennlinie, zweite Form |
| Markierter Vorgang | Vorgang mit Markierungsecke |
| Prozess (Kreis) | runder Prozess (Datenflussdiagramm) |
| Datenspeic… 3 | Datenspeicher als offenes Rechteck |
| Externe Entität 1 / 2 | Quelle oder Senke außerhalb des Systems |
| Magnetban… | Magnetband als Kreis mit Strich |
| Vorgang abgerundet | Vorgang mit runden Ecken |
| Anfrage erstellen | Erzeugen einer Instanz (SDL) |
| Rechteck mit Rahmen / Rechteck mit Rahmen.10 | Kasten mit innerem Rahmen |
| Offenes Rechteck | Rechteck mit offener Seite |
| Verzögerung | D-Form für Wartezeit |
| Summieren… Verbindung | Kreis mit Kreuz, Zusammenfluss |
| Trommelsp… | Trommelspeicher (Zylinder liegend) |
| Oder | Kreis mit Plus, Oder-Verknüpfung |
| Sortierer | Sanduhr-Form |
| Trennen | Dreieck für Aufteilen |
| Offlinespeic… | Dreieck mit Spitze unten, Ablage |
| Zusammen… | Dreieck für Zusammenführen |
| Sortiervorg… 2 | Raute mit Trennlinie, Sortieren |
| Kartenstapel | Stapel Lochkarten |
| Kartendatei | Lochkarten-Datei |
| Mikroform | Mikrofilm-Datenträger |
| Mikroformsp… / Mikroformv… | Mikrofilm-Speicher bzw. -Verarbeitung |
| Duplizieren | doppelter Kreis, Vervielfältigen |
| Prozess m. Schatten | Prozess mit Schatten |
| Markiertes Dokument | Dokument mit Markierungsecke |
| Dokument mit Linien | Dokument mit Textzeilen |
| Start (variabel) / Prozess (variabel) | Start- bzw. Prozessform mit Steuerpunkten |
| Sortieren | Raute mit waagrechter Linie |
| Datenspeic… | Datenspeicher, zwei Linien |
| Datenbank | Zylinder |
| Eingang von Steuerung / Ausgang an Steuerung | Signal-Ein- und -Ausgang (SDL) |
| Meldung v. Anwender / Meldung an Anwender | Signal vom bzw. an den Anwender (SDL) |
| Ausgabe | Ausgabe an einen Empfänger |
| Feedback | Rückmeldung, Fahnenform |
| Prüfen | Raute für Prüfschritt |
| Prüfvorgang 2 | Quadrat für Prüfung |
| UND- Gatter / ODER- Gatter | Verknüpfung von Zweigen |
| Verbesserung | Rückführpfeil für Verbesserung |
| Zweig: Rückkehr / Zweig: ohne Rückkehr | Verzweigung mit bzw. ohne Rückkehr |
| Ausschließe… ODER (XOR) / Vertikales XOR | exklusive Verzweigung |
| Unterbrech… | Unterbrechung des Ablaufs |
| Vertikales P-UND | parallele UND-Verzweigung senkrecht |
| Externes Steuerelem… | Einfluss von außen |
| Ellipse | Ellipse, z. B. für Zustand |

#### SH-023 Workflowobjekte

Personen, Dokumente und Dinge, die in Geschäftsabläufen vorkommen. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Person | einzelne Person |
| Daten | Datenträger/Daten |
| Kunde | Kunde mit Einkaufswagen |
| Benutzer | Person am Rechner |
| Produkt | Ware/Paket |
| Dokument | Schriftstück |
| Posten | Eintrag/Posten |
| Feld | Kiste oder Fach |
| Wettbewer… | Mitbewerber |
| Vertrag | Vertrag mit Unterschrift |
| Kundenseg… | Kundengruppe, Tortensegment |
| Dokuments… | Dokumentstapel |
| Mitarbeiter | Mitarbeitergruppe |
| Führungskr… | Führungskraft |
| Ausgabe | Geldausgabe |
| Geschlosse… Ordner | geschlossener Ordner |
| Offener Ordner | geöffneter Ordner |
| Einnahme | Geldeingang |
| Post | Brief/Umschlag |
| Vorgesetzter | vorgesetzte Person |
| Präsentation | Präsentationsunterlage |
| Arbeitsblatt | Tabellenblatt |
| Team | Personengruppe |
| Händler | Zwischenhändler |
| Webseite | Webseite im Browser |

#### SH-024 Workflowobjekte - 3D

3D-Variante von SH-023. Kennzeichnung: Visio-Design. Abweichungen: Es fehlen Daten, Posten, Feld, Vertrag,
Kundensegment, Mitarbeiter, Führungskraft, Ausgabe, Einnahme, Vorgesetzter, Team und Händler; der Dynamische
Verbinder ist enthalten. Zusätzlich:

| Shape | Beschreibung |
|---|---|
| Person - Rumpf | Person als Brustbild |
| Kasten | Karton |
| CD-ROM | optischer Datenträger |
| Leitende/r Geschäftsfü… | Geschäftsführung |
| Leiter/in der Finanzabteil… | Finanzleitung |
| Leiter/in der PR-Abteilung | Leitung Öffentlichkeitsarbeit |
| Leiter/in Operatives … | Leitung Betrieb |
| Kiste | Holzkiste/Transportkiste |
| Datenkasset… | Bandkassette |
| Diskette | Diskette |
| Kalkulation… | Tabellenkalkulation |
| USB-Stick | USB-Speicherstick |
| Lieferant | Zulieferer mit Ware |

#### SH-025 Workflowschritte

Tätigkeiten und Ereignisse eines Arbeitsablaufs als Symbole. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Aufgabe | Aufgabe mit Haken |
| Problem | Klemmbrett mit Ausrufezeichen |
| Absenden | Ablage mit Pfeil, etwas einreichen |
| Anforderung | Sprechblase mit Fragezeichen |
| Genehmigen | Dokument mit Haken |
| Ablehnen | Dokument mit Kreuz |
| Vereinbarung | Handschlag zweier Personen |
| Analysieren | Lupe über Dokument |
| Zuweisen | Person mit Pfeil, Aufgabe zuteilen |
| Autor | Stift, etwas verfassen |
| Brainstormi… | Kopf mit Ideenwolke |
| Ende | Stopp-Quadrat im Kreis |
| Interview | Gespräch zweier Personen |
| Besprechung | Gruppe am Tisch |
| Telefonanruf | Telefonhörer |
| Präsentieren | Person vor Tafel |
| Prozess | Prozessbalken |
| Forschung | Buch mit Lupe |
| Überprüfen | Brille, Durchsicht |
| Anfang | Abspiel-Dreieck im Kreis |
| Transaktion | Austausch zwischen Personen |
| Virtuelle Besprechung | Besprechung über Bildschirm |

#### SH-026 Workflowschritte - 3D

3D-Variante von SH-025. Kennzeichnung: Visio-Design. Abweichungen: Es fehlen Problem, Absenden, Zuweisen,
Brainstorming, Ende, Präsentieren, Prozess, Überprüfen, Anfang, Transaktion und Virtuelle Besprechung; der Dynamische
Verbinder ist enthalten. Zusätzlich:

| Shape | Beschreibung |
|---|---|
| Thema | Dokument mit Betreff |
| Vorlegen | Unterlagen übergeben |
| Präsentation | Person mit Schaubild |

### 4.5 Geschäft

**Brainstorming**

#### SH-027 Brainstorming-Shapes

Mindmaps aus einem Hauptthema mit verzweigten Themen. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Hauptthema | Ellipse in der Mitte der Mindmap |
| Thema | Text auf einem Ast |
| Mehrere Themen | fügt mehrere Äste auf einmal ein |
| Assoziation… | gestrichelter Pfeil zwischen zwei Themen |

#### SH-028 Legenden-Shapes

Markierungssymbole zum Auflegen auf andere Shapes, dazu eine Legende, die die verwendeten Symbole auflistet.
Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Aufgaben | leeres Kästchen, offene Aufgabe |
| Fertig | abgehaktes Kästchen |
| Idee | Glühbirne |
| Nachverfol… erforderlich | rote Fahne |
| Wichtig | Ausrufezeichen |
| Frage | Fragezeichen |
| Hohe Bedeutung | Pfeil nach oben |
| Geringe Bedeutung | Pfeil nach unten |
| Legende | Tabelle der benutzten Symbole, füllt sich selbst |
| Gut | lachendes Gesicht |
| Schlecht | trauriges Gesicht |
| Priorität 1 / 2 / 3 | nummerierte farbige Kreise |
| Löschen | rotes Kreuz |
| Nicht | Verbotskreis |
| Besprechung | Kalenderblatt |
| Information | Info-Kreis |
| Stern | Stern |
| Aufgabe | Klemmbrett |
| Achtung | Glocke |
| Uhrzeit | Uhr |
| Warnung | Warndreieck |
| Hinweis | gelber Notizzettel |

**Diagramme**

#### SH-029 Diagramm-Shapes

Bausteine für Säulen-, Torten-, Linien- und Vergleichsdiagramme; Werte werden über Shape-Daten eingegeben.
Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Balkendiagr… 1 / 2 | Säulengruppe, Höhen aus Werten |
| 3D-Balkend… | Säulendiagramm mit Tiefe |
| 3D-Achse | Achsenkreuz für 3D-Säulen |
| 3D-Balken, Text vert. / Text hor. | einzelne 3D-Säule mit senkrechter bzw. waagrechter Schrift |
| Kreisdiagra… | Tortendiagramm, Anteile über Werte |
| Segment | einzelnes Tortenstück |
| Besonderes Segment | herausgezogenes Tortenstück |
| Unterteilter Balken 1 / 2 | gestapelter Balken |
| Prozessdiag… | Fortschrittsbalken |
| Ressourcen… | Ressourcenbalken |
| Featureverg… | Tabelle zum Merkmalsvergleich |
| Feature akt./deakt. | Zelle mit Ein/Aus-Punkt |
| Zeilenbesch… / Spaltenbesc… | Zeilen- bzw. Spaltenköpfe einer Tabelle |
| Gitter | Tabellenraster |
| Ja/Nein-Feld | Zelle mit Ja/Nein-Markierung |
| Normalvert… | Glockenkurve |
| Exponentielle Kurve | Wachstumskurve |
| Liniendiagr… | Liniendiagramm mit Achsen |
| Linie für Liniendiagr. | weitere Datenlinie |
| Datenpunkt | einzelner Punkt mit Wert |
| X-Y-Achse / X-Y-Z-Achse | Achsenkreuz 2D bzw. 3D |
| Diagrammsk… | Skala mit Teilstrichen |
| X- / Y- / Z-Achsenb… | Achsenbeschriftung mit Werten |
| X-Achse / Y-Achse / Z-Achse | einzelne Achse als Pfeil |
| Textblock 10 Pt. / 12 Pt. | Textfeld in fester Schriftgröße |
| 2D-Sprechblase | Sprechblase als Fläche |
| 1D-Sprechb… | Sprechblase mit Zeigerlinie |
| Horizontale Beschriftung | Text mit schräger Zeigerlinie |
| Anmerkung | Text mit Zeigerlinie und Haken |

#### SH-030 Marketingdiagramme

Fertige Schaubilder für Strategie und Marketing (SWOT, Portfolios, Kurven). Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Matrix | Vierfeldermatrix |
| SWOT | Matrix mit Stärken, Schwächen, Chancen, Risiken |
| Stufendiagr… | Stufen nebeneinander, Anzahl einstellbar |
| Zusätzliche Stufe | weitere Stufe zum Anfügen |
| Kreisförmige Pfeile | Pfeilkreis für Kreisläufe |
| Dreieck | Dreieck mit Ebenen |
| 3D-Matrix | Matrix mit Tiefe |
| 3D-Matrix m. Beschrift. | 3D-Matrix mit Achsentexten |
| Ansoff-Mat… | Produkt-Markt-Matrix |
| Boston-Mat… | Portfolio-Matrix mit Feldern |
| Positionsgr… | Positionierungsgrafik mit Achsen |
| Marktanteil | Ellipse mit Anteilswert |
| Marktanalyse | Balken für Marktdaten |
| 3D-Pyramide | Pyramide mit Tiefe |
| Kreis-Netz-Diagramm | Kreise um ein Zentrum verbunden |
| PLZ | Produktlebenszyklus-Kurve |
| Einführung… | Kurve für Markteinführung |
| Normalvert… | Glockenkurve |
| Sektoren 1 / 2 | Kreis in Sektoren geteilt |
| Marketing… | Marketing-Mix als Kreis mit Segmenten |
| 3D-Feld | Block mit Tiefe |
| Unterteilter 3D-Balken | gestapelter Balken mit Tiefe |
| 3D-Kreis | Scheibe mit Tiefe |
| Rechteck | Rechteck |
| Kreis | Kreis |
| Venn-Diagr… | zwei überlappende Kreise |
| Gemusterter Block | Block mit Füllmuster |
| Farbiger Block | Block mit Farbfüllung |

#### SH-031 Matrixdiagramm-Shapes

Matrizen und Achsen für Einordnungen und Portfolios. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Einfache Matrix 2x2 | vier Felder |
| Matrix 1 | vier Kreise im Raster |
| Matrix 2 | vier Quadrate mit Abstand |
| Matrix 3 | vier Quadrate in Farbabstufung |
| Einfache Matrix 3x3 | neun Felder |
| Matrix 3x3 mit Beschri… | neun Felder mit Zeilen- und Spaltentext |
| Matrix mit Beschriftun… | Vierfeldermatrix mit Achsentexten |
| SWOT 1 / 2 | SWOT-Matrix in zwei Gestaltungen |
| 3D-Matrix | Matrix mit Tiefe |
| Positionsdi… | Positionsdiagramm mit Kreisen |
| Ansoff-Mat… | Produkt-Markt-Matrix |
| Boston-Mat… | Portfolio-Matrix |
| Achse 1 | L-förmiges Achsenpaar |
| Achse 2 | Achsenkreuz durch die Mitte |

#### SH-032 Pyramiden-Shapes

Pyramiden, Trichter und Kegel mit einstellbarer Ebenenzahl. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Standardpy… | Pyramide mit Ebenen |
| Invertierte Pyramide | Pyramide auf der Spitze |
| Pyramide mit Ebenen | Pyramide mit farbigen Ebenen |
| Invertierter Kegel | Kegel auf der Spitze |
| Standardpy… mit Beschri… | Pyramide mit Texten neben den Ebenen |
| Invertierte Pyramide … | umgedrehte Pyramide mit Texten |
| Pyramide mit Beschriftun… | Ebenenpyramide mit Texten |
| Trichter | Trichter für Filterstufen |
| Gestufte Pyramide | Pyramide aus Stufen |
| Benutzerdef… invertierter … | einzelne Ebene einer umgedrehten Pyramide |
| Benutzerdef… Pyramide | einzelne Pyramidenebene zum Stapeln |

#### SH-033 Zyklusdiagramm-Shapes

Kreisläufe, Ringe und Zielscheiben für Abläufe und Schichtenmodelle. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Kreisförmiger Pfeil | Ringpfeil |
| Kreisbeweg… in vier Phas… | Kreislauf aus vier Pfeilsegmenten |
| Kreisbogen | Bogenstück |
| Teilebene 1 / 2 / 3 / 4 | Viertelring-Segmente |
| Zielsegmente | Kreis in Segmente geteilt |
| Zielsegmente mit zwei Eb… | Segmentkreis mit zwei Ringen |
| Pfeilkreis mit 6 Schritten | sechs Pfeile im Kreis |
| Radialzyklus | Kreise um eine Mitte |
| Pfeilschleife | Kreise auf einer Pfeilbahn |
| Radialdiagr… | Mitte mit Speichen zu Kreisen |
| Sechseckige Schleife | Sechsecke im Ring |
| Konzentrisc… Ebene 1 / 2 / 3 / Zentrum | Ringe gleicher Mitte |
| Target | Zielscheibe |
| Zwiebeldia… | ineinanderliegende Kreise |

**Geschäftsprozess** (dazu SH-019 Pfeil-Shapes)

#### SH-034 Arbeitsflussdiagramm-Shapes

Abteilungen, Personen und Stellen für Arbeitsabläufe zwischen Bereichen. Kennzeichnung: gemischt – Visio-Design
(Abteilungs-Icons), Generisch (Verbinder, Seitenverweise). Wie in SH-009/SH-010 beschrieben, hier als kleine
Icons: Buchhaltung, Verbindlichkeiten, Forderungen, Bank, Vorstand, Kopierzentrum, Kundendienst, Finanzen,
Informationsdienste, Auslandsabteilung, Inventar, Rechtsabteilung, Management, Fertigung, Marketing, Fuhrpark,
Verpackung, Lohn und Gehalt, Veröffentlichung, Einkauf, Qualitätssicherung, Warenannahme, Empfang, Forschung und
Entwicklung, Versand, Zulieferer, Telekommunikation, Finanzverwaltung. Neu in dieser Schablone:

| Shape | Beschreibung |
|---|---|
| Verteilung | Verteilstelle für Waren oder Post |
| Internation… Marketing | Auslandsmarketing |
| Internation… Vertrieb | Auslandsvertrieb |
| Postraum 1 / 2 | Poststelle in zwei Darstellungen |
| Person 1 / 2 | einzelne Person |
| Belegschaft | Mitarbeitergruppe |
| Vertrieb/PR | Vertrieb und Öffentlichkeitsarbeit |
| Lager | Lagerplatz |

#### SH-035 Auditdiagramm-Shapes

Flussdiagramme für Prüf- und Revisionsabläufe mit Markierungen für geprüfte Schritte. Kennzeichnung: gemischt –
Norm: ISO 5807 (Grundsymbole), Generisch (Markierungen, Titelblock, Notiz).

| Shape | Beschreibung |
|---|---|
| Markierter Vorgang | Vorgang mit Prüfmarke in der Ecke |
| Entscheidung | Raute |
| Markiertes Dokument | Dokument mit Prüfmarke |
| E/A | Parallelogramm für Ein-/Ausgabe |
| Manueller Vorgang | Trapez für Handarbeit |
| Ende | Stadionform |
| Aktenordner | Dreieck für Ablage |
| Anzeige | Bildschirmanzeige |
| Geteilter Vorgang | Vorgang mit Trennlinie |
| Mehrere Dokum./Pr… | Dokumente bzw. Prozesse gestapelt |
| Prozess m. Schatten | Vorgang mit Schatten |
| Dokument mit Linien | Dokument mit Textzeilen |
| Mehrere Dokumente | Dokumentstapel |
| Datenbank | Zylinder |
| Datenspeic… | gespeicherte Daten |
| Diskette 1 / 2 | Diskette als Datenträger |
| Magnetband | Bandspule |
| Datenübertr… | Datenübertragung, Sechseck |
| Manuelle Eingabe | Viereck mit schräger Oberkante |
| Prüfvorgang 1 / 2 | schwarze Raute bzw. Quadrat für Prüfung |
| Vergleich 1 / 2 | Raute bzw. Quadrat für Abgleich |
| Referenzpu… | Querverweis-Pfeil |
| Ereignis | Ereignis in Stadionform |
| Geteiltes Ereignis | Ereignis mit Trennlinie |
| Titelblock | Kopf mit Prüfer, Datum, Titel |
| Notizzettel | Notiz |

#### SH-036 EPC-Diagramm-Shapes

Ereignisgesteuerte Prozessketten (Ereignis, Funktion, Konnektoren). Inhalt nicht im Material.

#### SH-037 Fehlerstrukturanalyse-Shapes

Fehlerbäume mit Ereignissen und logischen Gattern. Kennzeichnung: Norm: IEC 61025.

| Shape | Beschreibung |
|---|---|
| UND-Gatter | alle Eingangsereignisse nötig |
| ODER-Gatter | eines der Eingangsereignisse genügt |
| Basisereignis | Kreis, nicht weiter zerlegt |
| Sekundäres Ereignis | Raute, nicht weiter untersucht |
| Sperrgatter | Sechseck, Ereignis mit Bedingung |
| Prioritäts-U… | UND-Gatter mit Reihenfolge |
| Ausschließe… ODER-Gatter | genau eines der Ereignisse |
| Schwellwert… (m/n) | m von n Ereignissen nötig |
| Ereignis | Rechteck für zusammengesetztes Ereignis |
| Ereignis 'Haus' | Hausform, normal eintretendes Ereignis |
| Bedingtes Ereignis | Ellipse für Bedingung |
| Transfersy… | Dreieck, Verweis auf Teilbaum |

#### SH-038 ITIL-Shapes

Bausteine für IT-Service-Management-Abläufe. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Vorfall | Störungsmeldung |
| Dienstanfor… | Serviceanfrage |
| Person | einzelne Person |
| Kundendie… | Kundendienst/Servicedesk |
| Komitee | Gremium, z. B. Änderungsbeirat |
| Abteilung | Abteilung |
| Änderungs… | Änderungsantrag |
| Vereinbarung | Servicevereinbarung |
| Datenbank für das Ko… | Konfigurationsdatenbank (CMDB) |
| Wissensdat… | Wissensdatenbank |
| Lieferant | externer Lieferant |

#### SH-039 Kompatibilitäts-Shapes

Symbole für Compliance- und Kontrollprozesse. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Risiko | offenes Schloss |
| Kontrolle | geschlossenes Schloss |
| Eingabe | Dokument mit Pfeil hinein |
| Ausgabe | Dokument mit Pfeil hinaus |
| Assertion | Prüfaussage, zwei Personen |
| Bearbeiter | Person bei der Arbeit |
| Rolle | Person mit Ausweis |

#### SH-040 Shapes für Six Sigma-Flussdiagramme

Gleiche 14 Shapes wie SH-021 Standardflussdiagramm-Shapes (Prozess bis Off-Page-Referenz), zusammengestellt für
Six-Sigma-Abläufe. Kennzeichnung: Norm: ISO 5807.

#### SH-041 Shapes für Ursache/Wirkung-Diagramme

Fischgrätdiagramme (Ishikawa) mit Haupt- und Nebenursachen. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Wirkung | Hauptgräte mit Kopf für die Wirkung |
| Kategorie 1 / 2 | schräge Gräte mit Kategorienkasten oben bzw. unten |
| Fischrahmen | Fischumriss als Hintergrund |
| Primäre Ursache 1 / 2 | beschriftete Linie an einer Kategorie |
| Sekundäre Ursache 1 / 2 / 3 / 4 / 5 / 6 | beschriftete Nebenlinie in verschiedenen Richtungen |

#### SH-042 Six-Sigma-House of Quality-Shapes

Bausteine für das Qualitätshaus (QFD). Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Bewertungen | Bewertungssymbole Dreieck, Kreis, Punkt |
| Wert | Zahlenfeld |
| Korrelationen | Plus- und Minussymbole fürs Dach |
| Komplett | fertiges Qualitätshaus |
| Zeilen und Spalten | Raster mit Kopfzeilen |
| Zeilenblock | Block mehrerer Zeilen |
| Zeilenbesch… | Zeilenköpfe |
| Zeile | einzelne Zeile |
| Spalte | einzelne Spalte |
| Matrix | Beziehungsmatrix |
| Spaltenblock | Block mehrerer Spalten |
| Spaltenbesc… | Spaltenköpfe |
| Schräge Spalten | Spaltenköpfe mit schräger Schrift |
| Korrelations… | dreieckiges Korrelationsdach |

#### SH-043 TQM-Diagramm-Shapes

Symbole für Qualitätsmanagement: Arbeitsablaufsymbole, Flussdiagramm, Ursache/Wirkung, Kräfte. Kennzeichnung:
gemischt – Norm: ASME-Arbeitsablaufsymbole (Vorgang, Prüfung, Transport, Verzögerung, Lager), Rest Generisch.

| Shape | Beschreibung |
|---|---|
| Verkehr | Pfeil für Transport |
| Eingehende Waren | Dreieck mit Spitze unten, Wareneingang |
| Speicher / Speicher.29 | Dreieck für Lagerung |
| Prozedur | Rechteck mit Seitenstrichen |
| Vorgang | Kreis für Bearbeitung |
| Vorgang/Prüfung | Kreis im Quadrat, kombiniert |
| Thema | abgerundetes Rechteck |
| Organisat.-f… | Organisationsfunktion mit Kopfleiste |
| 2-teilige Funktion | Rechteck mit zwei Feldern |
| Entscheidung 1 / 2 | Raute in zwei Formen |
| Mehrfach-Verzweigung | Raute mit mehreren Ausgängen |
| Externe Organisation | Quadrat mit Diagonalen |
| Externer Prozess | Raute im Quadrat |
| Prüfung / Messung | Quadrat für Prüfung |
| Metrisch | Kennzahl-Feld |
| 2-teiliges Maß | Messfeld mit zwei Teilen |
| Systemdate… / Systemunte… / Systemfunk… | Kreisvarianten für Systemdaten, -unterbrechung, -funktion |
| Verzögerung | D-Form für Wartezeit |
| Verbinder (TQM) | Kreis als Sprungstelle |
| Verbundene Themen | Kreuz für Querbezug |
| Fertigung | schwarzer Kreis |
| Verschieben | schwarzes Dreieck nach unten |
| Prüfung | schwarzes Quadrat |
| Auswählbarer Prozess | Prozess mit wählbarer Form |
| Arbeitsfluss… 1 / 2 | Arbeitsfluss-Linie mit Markierung |
| Rückkopplu… | Rückführung |
| Funktionsü… vert. / horiz. | Swimlane-Raster senkrecht bzw. waagrecht |
| Kräftediagr… | Kraftfeldanalyse fördernd/hemmend |
| Ursache 1 / 2 / 3 | Ursachenlinie mit Text |
| Kategorie | Kategorie-Gräte |
| Wirkung | Hauptgräte |
| Fischrahmen | Fischumriss |
| Ergebnis | Ja-Ausgang |
| Nein-Ergebnis | Nein-Ausgang |
| Zweig: Rückkehr / Zweig: ohne Rückkehr | Verzweigung mit bzw. ohne Rückkehr |
| Unterbrech… | Unterbrechung |
| Externes Steuerelem… | Einfluss von außen |
| Verbesserung | Pfeil für Verbesserung |
| Verbinder Linie | gerader Verbinder mit Klebeenden |

#### SH-044 Wertstromzuordnungs-Shapes

Wertstromanalyse (Lean): Prozesse, Bestände, Material- und Informationsfluss. Kennzeichnung: Norm:
Wertstrom-Notation (VSM, de facto).

| Shape | Beschreibung |
|---|---|
| Prozess | Prozesskasten mit Datenfeld |
| Inventar | Dreieck für Bestand |
| Druckpfeil | gestreifter Pfeil, Schiebeprinzip |
| Kunde/Lief… | Fabrikgebäude für Kunde oder Lieferant |
| Versandpfeil | breiter Pfeil für Lieferung |
| Versandfahr… | Lastwagen |
| Produktion… (Steuerung) | Kasten für Produktionsplanung |
| Persönliche Information… | gerader Pfeil, mündliche Information |
| Elektronische Information… | Blitzpfeil, elektronischer Datenfluss |
| Datentabelle | Tabelle mit Prozesskennzahlen |
| Zeitachsens… | Abschnitt der Zeitlinie |
| Zeitachse gesamt | Summenfeld der Zeitlinie |
| Produktion… (Kanban) | Produktionskanban-Karte |
| Entnahmek… | Entnahmekanban-Karte |
| Batchkanban | Kanban für Losgrößen |
| Batchentna… | Entnahmekanban für Lose |
| Signalkanban | Dreieckige Signalkarte |
| Kanbanpost | Sammelstelle für Karten |
| Supermarkt | Regal-Symbol, Pufferlager |
| Ausgleichsl… | Ausgleichskasten (Heijunka) |
| FIFO-Strecke | Fließstrecke „zuerst rein, zuerst raus“ |
| Ziehpfeil 1 / 2 / 3 / 4 | Pfeil für Ziehprinzip in vier Formen |
| Kaizen-Blitz | Stern für Verbesserungsbedarf |
| Physisches Ziehen | kreisförmiger Pfeil, Holen von Material |
| Sequenzierte Zugkugel | Kugel für Reihenfolge-Zug |
| Belastungsa… | Belastungsausgleich |

**Organigramm**

#### SH-045 Augenzahl - Organigramm-Shapes

Organigramm-Stil mit dunklen Rechteckkarten und kleinem Bildplatzhalter. Die Funktions-Shapes sind in allen Stilen
gleich und hier einmal beschrieben. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Führungskr… Augenzahl | Position der obersten Führung, mit Foto-Platzhalter |
| Vorgesetzter Augenzahl | Position mit Untergebenen |
| Position Augenzahl | normale Stelle |
| Assistent Augenzahl | Assistenz, wird seitlich angehängt |
| Berater Augenzahl | Stabsstelle/Berater |
| Freie Stelle Augenzahl | unbesetzte Stelle |
| Mitarbeiter Augenzahl | Liste mehrerer Mitarbeitender in einem Shape |
| Teamrahmen | Rahmen um ein Team (Container) |
| Mehrere Shapes | fügt mehrere Positionen auf einmal ein |
| Drei Positionen | fügt Vorgesetzten mit drei Untergebenen ein |
| Titel/Datum | Überschrift mit Datum |
| Titel | Überschrift |
| Gepunktete Linie Vorg… | gestrichelte, fachliche Berichtslinie |
| Zusätzlicher direkter V… | zweite direkte Berichtslinie |

#### SH-046 Blütenblätter - Organigramm-Shapes

Gleiche Shapes wie SH-045, Stil: blatt- bzw. tropfenförmige Formen. Kennzeichnung: Visio-Design.

#### SH-047 Eingebunden - Organigramm-Shapes

Gleiche Shapes wie SH-045, Stil: Text zwischen seitlichen Klammerlinien mit Bildplatzhalter. Kennzeichnung: Visio-Design.

#### SH-048 Gürtel - Organigramm-Shapes

Gleiche Shapes wie SH-045, Stil: Rechteck mit umlaufendem Band. Kennzeichnung: Visio-Design.

#### SH-049 Karten – Organigramm-Shapes

Stil: weiße Karte mit farbiger Randleiste und Feldern für Name und Titel. Kennzeichnung: Visio-Design.
Abweichung: Nur sechs Shapes sichtbar – Führung, Vorgesetzter, Assistent, Mitarbeiter, Berater, Freie Stelle (Bedeutung
wie SH-045; „Mitarbeiter“ ist hier eine einzelne Karte). Hilfs-Shapes wie Teamrahmen und Titel fehlen und werden für
diesen P2-Stil aus SH-045 übernommen.

#### SH-050 Kerbe - Organigramm-Shapes

Gleiche Shapes wie SH-045, Stil: Rechteck mit Bildplatzhalter und Kerbe am Rand. Kennzeichnung: Visio-Design.

#### SH-051 Münze - Organigramm-Shapes

Gleiche Shapes wie SH-045, Stil: runde Münze mit Personensilhouette. Kennzeichnung: Visio-Design.

#### SH-052 Perspektive - Organigramm-Shapes

Gleiche Shapes wie SH-045, Stil: schräg gestellte Fahnenform. Kennzeichnung: Visio-Design.

#### SH-053 Shapetakulär - Organigramm-Shapes

Gleiche Shapes wie SH-045, Stil: Rechteck mit Reiter-Ecke. Kennzeichnung: Visio-Design.

#### SH-054 Stein - Organigramm-Shapes

Gleiche Shapes wie SH-045, Stil: abgerundete, schräge Kieselform. Kennzeichnung: Visio-Design.

#### SH-055 Tafel - Organigramm-Shapes

Gleiche Shapes wie SH-045, Stil: Karte mit großem Foto-Platzhalter. Kennzeichnung: Visio-Design.

**Pivotdiagramm**

#### SH-056 Pivotdiagramm-Shapes

Knoten für Pivotdiagramme, die Daten aus einer Quelle gruppiert als Baum darstellen. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Pivotknoten | Datenknoten mit Summen, aufklappbar nach Feldern |

### 4.6 Konstruktion

**Elektrotechnik**

#### SH-057 Analog-Digital-Logisch

Logikgatter, Speicherglieder und Analogrechner-Bausteine. Der Gattertyp wird über Shape-Daten gewählt.
Kennzeichnung: Norm: IEC 60617-12/-13, IEEE 91.

| Shape | Beschreibung |
|---|---|
| Logische Schaltung 1 / 2 | Gatter als Kasten bzw. in Kurvenform, Typ wählbar |
| Wechselric… | Inverter (Nicht-Glied) |
| Puffer | Treiber ohne Invertierung |
| Flip-Flop | Speicherglied mit Setz- und Rücksetzeingang |
| Negativer Logikpunkt | Kreis für invertierten Anschluss |
| E/A-Port | Ein-/Ausgangsanschluss |
| Signalwelle | Verlauf eines Signals |
| Takt | Taktgeber |
| Funktionsg… / Funktionsg… 2 | Funktionsgenerator |
| Verstärker | Verstärker-Dreieck |
| Konverter | Umsetzer analog/digital |
| Analogsym… | Kennzeichen „analog“ |
| Digitalsymb… | Kennzeichen „digital“ |
| Funktionsp… | Funktionspotentiometer |
| Positionaler Servo | Stellantrieb mit Rückführung |
| Verzögerun… | Verzögerungsglied |
| Dreistellung… | Dreipunktglied |
| Integrator | Integrierglied |
| Generalisier… Integrator | Integrator mit mehreren Eingängen |
| Addierer | Summierglied |
| Multiplikator | Multiplizierglied |
| Divider | Dividierglied |
| Operationaler Verstärker / Operationaler Verstärker 2 | Operationsverstärker in zwei Formen |

#### SH-058 Baugruppen

Funktionsblöcke der Nachrichten- und Messtechnik (Verstärker, Gleichrichter, Geber). Kennzeichnung: Norm:
IEC 60617 / IEEE 315.

| Shape | Beschreibung |
|---|---|
| Verstärker | Verstärker-Dreieck |
| Gleichrichter | Gleichrichter-Block |
| 1-Weg-Rep… | Verstärker in einer Richtung |
| 2-Weg-Rep… (2 Varianten) | Verstärker in beide Richtungen |
| 2-Weg-Rep… 4-Draht | Zweiwegverstärker für Vierdrahtleitung |
| Brückenglei… | Brückengleichrichter |
| Demodulator | Demodulator |
| Magnetvers… | Magnetverstärker |
| NLT-Verstär… | Verstärker mit nichtlinearer Kennlinie (vermutet) |
| Verstärker mit Umg… | Verstärker mit Umgehungszweig |
| A. veränd. Verstärker | Verstärker mit einstellbarer Verstärkung |
| Gesteuerter Gleichrichter | Gleichrichter mit Steueranschluss |
| Netznieders… | Netz-Baugruppe, Name abgeschnitten |
| Phasenschi… | Phasenschieber |
| Zerhacker | Zerhacker (Chopper) |
| Gyrometer / Gyrometer 2 | Kreisel-Messgerät |
| Positionsan… Synchro | Stellungsanzeige mit Synchro |
| Positionsan… Spule | Stellungsanzeige mit Spule |
| Desynn-Pos… | Desynn-Stellungsanzeige |
| Positionsge… Spule | Stellungsgeber mit Spule |
| Feuerlöschk. 1 Kontakt / 2 Kontakte | Feuerlöschkontakt mit einem bzw. zwei Kontakten |
| Konverter | Umformer |
| Gleichrichte… | Gleichrichtergruppe |
| Wärmequelle | Heizelement |
| Näherungss… | Näherungssensor |

#### SH-059 Beschriftungssymbole

Zusatzzeichen für Schaltgruppen, Phasen und Polarität. Kennzeichnung: Norm: IEC 60617-2/-6.

| Shape | Beschreibung |
|---|---|
| 3-Phasen Zickzack | Zickzackschaltung |
| 4-Phasen | Vierphasen-Kreuz |
| Strahlung | Strahlungspfeil |
| Positive Polarität | Pluszeichen |
| Negative Polarität | Minuszeichen |
| Nullleitersy… | Kennzeichen „N“ für Neutralleiter |
| Mehrfachp… | Mehrphasen-Kennzeichen |
| 3 separate Wicklungen | drei getrennte Wicklungen |
| 2-Phasen 3-Draht / 2-Phasen 4-Draht | Zweiphasensystem mit drei bzw. vier Leitern |
| 3-Phasen (V) / 3-Phasen (T) | offene Dreieck- bzw. T-Schaltung |
| 3-Phasen Dreieck 1 / 2 / 3 / 4 | Dreieckschaltung in vier Varianten |
| 3-Phasen-S… (Stern) | Sternschaltung |
| 3-Phasen 4-Draht | Sternschaltung mit Neutralleiter |
| 6-Phasen Doppelstern | Doppelsternschaltung |
| 6-Phasen Doppeldrei… | Doppeldreieckschaltung |
| 6-Phasen-H… | Sechseckschaltung |
| 6-Phasen Aufspaltung | gegabelte Sechsphasenschaltung |
| Spezieller Verbinder | Sonderanschluss |
| Koaxialsym… | Koaxial-Kennzeichen |
| Elektret | Elektret-Kennzeichen |

#### SH-060 Beschriftungssymbole - Visio 2013

Ältere Fassung von SH-059. Inhalt nicht im Material.

#### SH-061 Grundlegende Elemente

Grundbauteile der Elektrotechnik für Stromlaufpläne. Kennzeichnung: Norm: IEC 60617-2/-4/-6, IEEE 315.

| Shape | Beschreibung |
|---|---|
| Widerstand | Widerstand |
| Kondensator | Kondensator |
| Wechselstr… | Wechselspannungsquelle |
| Gleichstro… | Gleichspannungsquelle |
| Äquipotenz | Potentialausgleich |
| Erde | Erdung |
| Spule | Induktivität |
| Quarz | Schwingquarz |
| Antenne | Antenne |
| Gehäuse | Masse am Gehäuse |
| Schaltkreisu… | Leitungsschutzschalter |
| Ideelle Quelle | ideale Quelle |
| Allgem. Komponente | allgemeines Bauteil mit Kennbuchstabe |
| Akkumulator / Akku | Batteriezelle |
| Wechselim… | Wechselimpuls |
| Halbspule | halbe Wicklung |
| Zündkapsel / Zündkapsel… (2) | Zündkapsel |
| Ferritkern | Ferritkern |
| Sicherung | Schmelzsicherung |
| Dauermagnet | Permanentmagnet |
| Glocke | Klingel |
| Magnetkern | Eisenkern |
| Thermoele… | Thermoelement |
| Sägezahn | Sägezahnsignal |
| Zündkerze | Zündkerze |
| Rahmenant… | Rahmenantenne |
| Mikrofon / Mikrofon 2 | Mikrofon in zwei Formen |
| Oszillator | Oszillator |
| Pickupkopf | Tonabnehmer |
| Initialzünder | Sprengzünder |
| Impuls | Rechteckimpuls |
| Lautsprecher | Lautsprecher |
| Messwandler / Messwandler 2 | Messwandler |
| Thermopaar | Thermopaar |
| Thermosäule | Thermosäule |
| Lampe / Lampe 2 | Leuchte in zwei Formen |
| Indikator | Anzeigeinstrument |
| Überspann… / Überspann… 2 | Überspannungsableiter |
| Material | Materialkennzeichen |
| Leuchtstoffl… | Leuchtstofflampe |
| Verzögerun… | Verzögerungsleitung |
| Sprungfunk… | Sprungfunktion |
| Summer | Summer |
| Stromgeste… Stromquelle | stromgesteuerte Stromquelle |
| Spannungs… Stromquelle | spannungsgesteuerte Stromquelle |
| Stromgeste… Spannungs… | stromgesteuerte Spannungsquelle |
| Spannungs… Spannungs… | spannungsgesteuerte Spannungsquelle |
| Mehrzellen… | Batterie aus mehreren Zellen |
| Mehrzellen… mit Abgriffen | Batterie mit Anzapfungen |
| Dämpfungs… | Dämpfungsglied |

#### SH-062 Grundlegende Elemente - Visio 2013

Ältere Fassung von SH-061. Inhalt nicht im Material.

#### SH-063 Halbleiter und Elektronenröhren

Dioden, Transistoren und Thyristoren (neuere Fassung, ohne Röhren). Kennzeichnung: Norm: IEC 60617-5.

| Shape | Beschreibung |
|---|---|
| Bipolar | Bipolartransistor |
| Diode | Diode |
| Photodiode | lichtempfindliche Diode |
| MOSFET N-Typ / MOSFET P-Typ | Feldeffekttransistor mit isoliertem Gate |
| Diac | Zweirichtungs-Diode |
| Zenerdiode… | Z-Diode |
| Verbindung | Sperrschicht-Feldeffekttransistor |
| UJT-Transis… | Unijunction-Transistor |
| Varaktor | Kapazitätsdiode |
| Vierschichti… Diode | Vierschichtdiode |
| Ohmsch | Transistor mit ohmschem Kontakt |
| Transvers | Transistor mit Querfeld |
| Tunneldiode | Tunneldiode |
| Uni-Tunnel… | Rückwärtsdiode |
| Darlington | Darlington-Transistor |
| Riegel | Thyristor-Variante (Name übersetzt) |
| Gesteuerter Schalter | steuerbarer Halbleiterschalter |
| Gesteuerter Gleichrichter | Thyristor |
| Abschalttri… | abschaltbarer Thyristor |
| Abschaltgle… | abschaltbarer Gleichrichter |
| Triac | Zweirichtungs-Thyristor |
| Konstantstr… | Stromregeldiode |
| Speichersch… | Speicherschaltdiode |
| Schottkydio… | Schottky-Diode |
| Stiftdiode | PIN-Diode (Fehlübersetzung) |
| IGBT | Bipolartransistor mit isoliertem Gate |
| Opto-TRIAC | lichtgesteuerter Triac |
| Photothyris… | lichtgesteuerter Thyristor |
| Doppelemit… | Transistor mit zwei Emittern |

#### SH-064 Halbleiter und Elektronenröhren - Visio 2013

Ältere Fassung von SH-063 in schwarzer Zeichnung. Kennzeichnung: Norm: IEC 60617-5. Abweichungen: Es fehlen MOSFET N/P,
Konstantstrom-, Speicherschalt-, Schottky- und Stiftdiode, IGBT, Opto-TRIAC, Photothyristor und Doppelemitter.
Zusätzlich:

| Shape | Beschreibung |
|---|---|
| MOSFET | Feldeffekttransistor ohne Kanaltyp |
| IGFET N-Typ / IGFET P-Typ | Feldeffekttransistor mit isoliertem Gate |
| Diodenröhre | Röhrendiode |
| Triodenröhre | Röhre mit Gitter |
| Tetrodenrö… | Röhre mit zwei Gittern |
| Pentodenrö… | Röhre mit drei Gittern |

#### SH-065 Integrierte Schaltkreise

IC-Gehäuse aus stapelbaren Teilen und digitale Funktionsblöcke. Kennzeichnung: Norm: IEC 60617-12, IEEE 91.

| Shape | Beschreibung |
|---|---|
| 4X Baust. Komplett / Oben / Mitte / Basis | breites IC-Gehäuse, Teile stapelbar für mehr Pins |
| 1X Baust. Komplett / Oben / Mitte / Basis | schmales IC-Gehäuse, Teile stapelbar |
| Horiz. Erweiterung / Vert. Erweiterung | zusätzliche Pin-Reihe |
| Tafel | Chip in Draufsicht |
| Negativer Logikpunkt | invertierter Anschluss |
| Schalterpun… | Schaltpunkt am Pin |
| Erde | Masseanschluss |
| MUX 2 / MUX 4 / MUX 8 | Multiplexer mit 2, 4, 8 Eingängen |
| 2-zu-4-Dec… / 3-zu-8-Dec… | Decoder |
| 4-Bit Zähler / 8-Bit Zähler | Zähler |
| 4-Bit-Zähler mit Teiler / 8-Bit-Zähler mit Teiler | Zähler mit Teilerausgang |
| 4-Bit-Register / 8-Bit-Register | Register |
| 4-Bit-A/D-K… / 8-Bit-A/D-K… | Analog-Digital-Umsetzer |
| 4-Bit-D/A-K… / 8-Bit-D/A-K… | Digital-Analog-Umsetzer |
| Treiber | Leitungstreiber |
| Analogscha… 2 / 4 | Analogschalter mit 2 bzw. 4 Kanälen |
| Monoflop | monostabile Kippstufe |
| Spannungs… | Spannungsregler (vermutet) |
| Impulsbreit… | Pulsweitenmodulator |

#### SH-066 Karten und Diagramme

Kraftwerke, Stationen und Funkanlagen für Lage- und Netzpläne. Kennzeichnung: Norm: IEC 60617-11.

| Shape | Beschreibung |
|---|---|
| Funkstation 1 / 2 | ortsfeste Funkstelle |
| Tragbare Funkstation | tragbares Funkgerät |
| Mobile Funkstation | Fahrzeugfunk |
| Funkpeilung | Peilstation |
| Funkleitstat… | Funkleitstelle |
| Endstation | Endstelle |
| Funkrelaisst… | Relaisfunkstelle |
| Teilnehmer… | Teilnehmerstation |
| Passives Relais | passiver Umsetzer |
| Raumstation | Raumfunkstelle |
| Erdfunkstati… | Bodenstation für Satelliten |
| Telegrafiere… (2) | Telegrafiestelle |
| Telegrafieg… (2) | Telegrafiegerät |
| Telefon | Fernsprechstelle |
| Kraftwerk | Kraftwerk allgemein |
| Heizkraftwerk | Kraft-Wärme-Kopplung |
| Wasserkraft… 1 / 2 / 3 / 4 | Wasserkraftwerk in vier Ausführungen |
| Wärmekraft… | Wärmekraftwerk |
| Kohlekraftw… | Kohlekraftwerk |
| Öl-/Gaskraf… | Öl- oder Gaskraftwerk |
| Kernkraftwe… | Kernkraftwerk |
| Erdwärmekr… | Geothermiekraftwerk |
| Sonnenkraf… | Solarkraftwerk |
| Windkraftw… | Windkraftwerk |
| Unterstation | Unterwerk |
| Antriebsma… | Antriebsmaschine |
| Umspannst… | Umspannwerk |
| Schaltstation | Schaltanlage |
| Gleichrichte… | Gleichrichterwerk |
| Plasmakraft… MHD | magnetohydrodynamisches Kraftwerk |

#### SH-067 Rotierende Geräte und mechanische Funktionen

Maschinen, Kupplungen und mechanische Wirkverbindungen. Kennzeichnung: Norm: IEC 60617-6.

| Shape | Beschreibung |
|---|---|
| Rotierende Maschine | Kreis mit Kennbuchstabe (M, G) |
| Läufer | Rotorwicklung |
| Bürste | Schleifbürste |
| Feld | Feldwicklung |
| Dauermagnet | Permanentmagnet |
| Wicklungsv… | Wicklungsschaltung |
| Synchro | Drehmelder |
| Bremse | Bremse |
| Getriebe | Getriebe |
| Drehung | Drehrichtungspfeil |
| Kupplung / Kupplung 2 | Kupplung in zwei Formen |
| Verzögerte Aktion | verzögerte Wirkung |
| Manuelle Steuerung | Handbetätigung |
| Blockierung | Sperre in einer Stellung |
| Verriegelung | gegenseitige Verriegelung |
| Mechanische Sperre | Rastsperre |
| Autom. Rücksetzung | selbsttätiger Rückgang |
| Sperre | Sperrklinke |

#### SH-068 Schalter und Relais

Schalter, Kontakte, Relais und Sicherungen. Kennzeichnung: Norm: IEC 60617-7, IEEE 315 (Zeichnung im ANSI-Stil).

| Shape | Beschreibung |
|---|---|
| Relais | Relaisspule mit Kontakt |
| Relaiskonta… | Relaiskontakt |
| Trennschalter | Trenner |
| Sicherung | Schmelzsicherung |
| Steuerlicht | Meldeleuchte |
| Relaisspule | Spule allein |
| Quecksilber… / Quecksilber… 2 | Quecksilberschalter |
| Flüssigkeits… / Flüssigkeits… 2 | Schwimmerschalter |
| Gasflussabh… | Schalter abhängig vom Gasfluss |
| Flussabhän… | Strömungsschalter |
| Druckabhä… | Druckschalter |
| Auslösungs… | Auslöser |
| Sicherheitss… | Sicherheitsschalter |
| Temp.-scha… | Temperaturschalter |
| Thermostat | Thermostat |
| Grenzschalter | Endschalter |
| Schaltkreisu… | Leistungsschalter |
| Wahlschalter 1 | Wahlschalter |
| Kurzschluss… 2 | Kurzschließer |
| Näherungs… | Näherungsschalter |
| Trägheitssc… | Trägheitsschalter |
| Tastschalter… | Taster |
| Manueller Schalter | handbetätigter Schalter |
| Schließkont… | Schließer |
| SPST | einpoliger Ein-Aus-Schalter |
| SPDT | einpoliger Umschalter |
| DPST | zweipoliger Ein-Aus-Schalter |
| DPDT | zweipoliger Umschalter |
| Unterbrech… | Öffner |
| Wechslerko… / Wechselko… | Wechsler |
| Durchgang… | Schalter mit Durchgang |
| Arretiert | rastender Schalter |
| Rückfederu… / 2 / 3 | Schalter mit Federrückstellung |
| Grenzschalter n/a | Endschalter Schließer |
| Grenzschalter offen/gesc… | Endschalter Öffner/Schließer |
| Zweistellun… / Dreistellung… / Vierstellung… | Stufenschalter mit 2, 3, 4 Stellungen |
| Tastschalter (schließend) | Taster als Schließer |
| Tastschalter 2-Schaltkreis | Taster mit zwei Kreisen |
| Verzögertes Schließen / Verzögertes Schließen 2 | zeitverzögerter Schließer |
| Verzögerte Unterbrech… / 2 | zeitverzögerter Öffner |
| Isolator | Isolator |

#### SH-069 Schalter und Relais - Visio 2013

Ältere Fassung von SH-068. Inhalt nicht im Material.

#### SH-070 Stecker und Anschlüsse

Steckverbinder, Buchsen und Klemmen. Kennzeichnung: Norm: IEC 60617-3, IEEE 315.

| Shape | Beschreibung |
|---|---|
| Adapter | Zwischenstecker |
| Schaltkreisa… | Anschlusspunkt |
| St./Bu.-Kontakt / St./Bu.-Kontakt 2 | Stecker-Buchse-Kontakt |
| Klemmbrett | Klemmleiste |
| Kabelabschl… | Kabelendverschluss |
| Buchse mit 2 Leitern | zweipolige Buchse |
| 3polige Buchse | dreipolige Buchse |
| Stecker mit 2 Leitern | zweipoliger Stecker |
| 3poliger Stecker | dreipoliger Stecker |
| Klinkenbuc… (2) | Klinkenbuchse |
| Schirmung - Koaxialverbi… | geschirmte Koaxialverbindung |
| Mittelleit-K… | Innenleiter-Kontakt |
| Sub D - groß | großer D-Sub-Stecker |
| Kleiner D-Verbinder | kleiner D-Sub-Stecker |
| C-Stecker | Kaltgerätestecker |
| Wechsels. Ausgang | Wechselspannungs-Ausgang |
| Abgeschir… Buchse | geschirmte Buchse |
| Koaxial-Bu… | Koaxialbuchse |
| Bu./St. 2 Pole (1) / (2) / (3) | zweipolige Steckverbindung in drei Formen |
| Bu./St. 3 Pole (1) / (2) / (3) / (4) / (5) | dreipolige Steckverbindung in fünf Formen |

#### SH-071 Telekommunikationsschaltung und Peripheriegerät

Bausteine der Nachrichten- und Speichertechnik. Kennzeichnung: Norm: IEC 60617, IEEE 315.

| Shape | Beschreibung |
|---|---|
| Frequenzba… | Frequenzband |
| Freq.-spek.-element | Frequenzspektrum-Element |
| Verbindung… | Verbindungspunkt |
| Signalstufe | Signalstufe |
| Schaltstufe | Schaltstufe |
| Schalteinric… 1 / 2 | Schalteinrichtung |
| Magnetisch | magnetische Speicherung |
| Bewegliche Spule | Tauchspule |
| Beweglicher Kern | Tauchkern |
| Stereo | Stereo-Kennzeichen |
| Magnetplatte | Plattenspeicher |
| Magnetban… Film | Band- oder Filmspeicher |
| Magnettro… | Trommelspeicher |
| Wiedergeben | Wiedergabe |
| Aufzeichnu… und Wiede… | Aufnahme und Wiedergabe |
| Löschen | Löschfunktion |
| Wandlerkopf | Tonkopf |
| Schaltkreis m. Verstärker | Leitung mit Verstärker |
| Impulsmod… | Pulsmodulation |
| Wandler | Wandler |
| Filter | Filter |
| Netzwerke | Netzwerk (Vierpol) |
| Gabelschalt… | Gabelschaltung |
| Modulator | Modulator |
| Aufzeichnu… | Aufnahme |
| Konzentrier… | Konzentrator |
| Optoelektr. Sensor | Lichtsensor |

#### SH-072 Transformatoren und Wicklungen

Transformatoren, Wandler, Spulen und Drosseln. Kennzeichnung: Norm: IEC 60617-6.

| Shape | Beschreibung |
|---|---|
| Transforma… | Transformator mit zwei Wicklungen |
| Transforma… 2 | Transformator in Kreisdarstellung |
| Magnetkern | Eisenkern |
| Spule | Wicklung |
| Drossel | Drosselspule |
| Variometer | einstellbare Induktivität |
| Koaxiale Drossel | Drossel in Koaxialbauweise |
| Transduktor | Transduktor (Magnetverstärker) |
| Gesättigt. Transf. | Transformator mit Sättigungskern |
| Regeltransf… (2) | Stelltransformator |
| 1-Phasen-I… Spann.regler | einphasiger Induktionsregler |
| 3-Phasen-T… | Drehstromtransformator |
| Aktueller Transforma… (3) | Stromwandler (Fehlübersetzung) |
| Potentialtra… / Potentialtra… 2 | Spannungswandler |
| Pot.-Trans. 3 Wicklungen | Spannungswandler mit drei Wicklungen |
| Außenspule | Außenwicklung |
| Linearkoppler | linearer Koppler |

#### SH-073 VHF-UHF-SHF

Elemente der Hochfrequenz- und Wellenleitertechnik. Kennzeichnung: Norm: IEC 60617, IEEE 315.

| Shape | Beschreibung |
|---|---|
| Diskontinui… | Stoßstelle im Wellenleiter |
| Scheinwide… | Impedanz |
| Kapazitiver BW / Induktiv. BW | kapazitiver bzw. induktiver Blindwiderstand |
| Widerstand | Wirkwiderstand |
| Serienschwi… | Serienschwingkreis |
| Scheinleitw… | Admittanz |
| Kapazitiver BLW / Induktiv. BLW | kapazitiver bzw. induktiver Blindleitwert |
| Wirkleitwert | Konduktanz |
| Versch. Tauchtrim… | verstellbarer Abstimmstift |
| E-H-Tuner | E-H-Abstimmglied |
| Stufentransf. | Stufentransformator |
| Richtkoppler | Richtkoppler |
| Modusfilter / Modusfilter 2 | Wellentypfilter |
| Drehverbin… | Drehkupplung |
| Isolator | Richtungsleitung |
| Phasenschi… | Phasenschieber |
| Gyrator | Gyrator |
| Zirkulator fest / umschaltb. | Zirkulator fest bzw. umschaltbar |
| Polaris. Drehfeld | Polarisationsdreher |
| Amplituden… feldpol. | Amplitudenregler (Name abgeschnitten) |
| Resonator | Hohlraumresonator |
| Balun | Symmetrierglied |
| Frequenzfilter | Frequenzfilter |
| Phasensch. (abgestimmt) | abgestimmter Phasenschieber |
| Ferritperlen… | Ferritperle |
| Wellenstrec… | Wellenleiterabschnitt |
| Laser/ Maser | Laser oder Maser |
| Rubinlaser | Rubinlaser |

#### SH-074 Wartungssymbole

Blöcke und Signalpfeile für Wartungs-Funktionspläne. Kennzeichnung: Norm: IEEE 315.

| Shape | Beschreibung |
|---|---|
| Signalgener… | Signalquelle |
| Lineares Element | lineares Glied |
| Relaiskonta… | Relaiskontakt als Block |
| Relaisspule | Relaisspule als Block |
| Schalter | Schalter als Block |
| Verbundsch… | Verbundschalter |
| Verstärker | Verstärker |
| Signalcode | Pfeil für Signalcodierung |
| Referenzsig… | Referenzsignal |
| Relaiserrege… | Relaiserregung |
| Geberimpuls | Geberimpuls |
| Testsignal | Prüfsignal |
| Rückkopplu… | Rückführung |

#### SH-075 Übertragungspfade

Leitungsarten und Übertragungswege. Inhalt nicht im Material.

#### SH-076 Übertragungspfade - Visio 2013

Ältere Fassung von SH-075. Inhalt nicht im Material.

**Maschinenbau**

#### SH-077 Federn und Lager

Wälzlager, Federn und Form-Elemente an Wellen und Bohrungen in vereinfachter Darstellung. Kennzeichnung: Norm:
ISO 8826, ISO 128.

| Shape | Beschreibung |
|---|---|
| Allgemein | Wälzlager allgemein im Schnitt |
| Rillenkugell… | Rillenkugellager |
| Schrägkuge… | Schrägkugellager |
| Schrägkugell. (dopp.) | zweireihiges Schrägkugellager |
| Pendelkugell. (dopp.) | Pendelkugellager |
| Kugelspurla… | Axial-Kugellager |
| Zylinderrolle | Zylinderrollenlager |
| Zylinderroll… (dop.) | zweireihiges Zylinderrollenlager |
| Kegelrollen | Kegelrollenlager |
| Nadellager | Nadellager |
| Tonnenlager (dop.) | Pendelrollenlager |
| Getriebe | Zahnrad in Draufsicht |
| Schraubenf… | Schraubenfeder |
| Federhaken | Federöse/-haken |
| Kegelwelle | kegeliges Wellenende mit Verhältnisangabe |
| Treibkeil | Keil mit Neigungsangabe |
| Runder Keil | Passfeder mit runden Enden |
| Fase | angefaste Kante |
| Aussparung | Freistich |
| Zentrierboh… | Zentrierbohrung |
| Ausschnitt | Bruchkante/Ausbruch |
| Spindelende | Gewindeende einer Spindel |
| Senkloch | Senkung in Draufsicht |
| Gewindebo… 1 / 2 | Gewindebohrung in Draufsicht bzw. Schnitt |
| Keilnut | Nut für Passfeder |
| Durchgang… | Durchgangsloch |

#### SH-078 Geometrische Bemaßungen und Toleranzen

Form- und Lagetoleranzen mit Toleranzrahmen und Bezügen. Kennzeichnung: Norm: ISO 1101.

| Shape | Beschreibung |
|---|---|
| Bezugssymbol | Bezugsbuchstabe im Kreis |
| Bezug (alt) / Bezug (neu) | Bezugskennzeichen alte bzw. neue Form |
| Bezugsziele | Bezugsstellen auf der Fläche |
| Rechteckige Beschriftung | Hinweis im Kasten mit Linie |
| Beschriftung | Hinweislinie mit Text |
| Rundumbe… | Kennzeichen „ringsum“ |
| Textblock | Textfeld |
| Einfacher Rahmen | Toleranzrahmen mit einem Feld |
| Basisrahmen | Toleranzrahmen mit Symbol und Wert |
| 1 / 2 / 3 Bezugsrah… | Toleranzrahmen mit ein bis drei Bezugsfeldern |
| Geradheit | Formtoleranz Geradheit |
| Flachheit | Formtoleranz Ebenheit |
| Linienprofil | Profil einer Linie |
| Kreis | Rundheit |
| Zylindrisch | Zylinderform |
| Flächenprofil | Profil einer Fläche |
| Position | Positionstoleranz |
| Konzentrizität | Koaxialität |
| Symmetrie | Symmetrie |
| Parallelität | Parallelität |
| Rechter Winkel | Rechtwinkligkeit |
| Winkel | Neigung |
| Vollst. Auslauf | Gesamtlauf |
| Kreisförm. Auslauf | Rundlauf |
| Materialzus… | Maximum-Material-Bedingung |
| Oberfläche… | Oberflächenangabe |
| Bogenlänge | Bogenmaß-Kennzeichen |
| Durchmesser | Durchmesserzeichen |
| Plan-/Stirnsenk. | Flachsenkung |
| Spitzsenke | Kegelsenkung |
| Tiefe | Tiefenzeichen |
| Neigung | Neigungszeichen |
| Konischer Kegel | Kegelverjüngung |
| Statist. Toleranz | statistisch tolerierte Maße |

#### SH-079 Pneumatik/Hydraulik - Geräte

Pumpen, Zylinder, Behälter und Messgeräte der Fluidtechnik. Kennzeichnung: Norm: ISO 1219-1.

| Shape | Beschreibung |
|---|---|
| Pumpe/Mo… (einfach) | Pumpe oder Motor, eine Richtung |
| Pumpe/Motor 1 / 2 | Pumpe oder Motor, verstellbar bzw. zwei Richtungen |
| Luftkompre… | Verdichter |
| Stellglied (halbdr.) | Schwenkantrieb |
| Antrieb | Antrieb allgemein |
| Einfach wirkend | einfach wirkender Zylinder |
| Doppelt wirkend | doppelt wirkender Zylinder |
| Dop.wirk. magn. | doppelt wirkender Zylinder mit Magnetkolben |
| Doppelend… | Zylinder mit durchgehender Kolbenstange |
| Teleskop | Teleskopzylinder |
| Stellglied | Linearantrieb |
| Verstärker | Druckübersetzer |
| Stellglied/Verstärker | Antrieb mit Übersetzer |
| Druckbehäl… | Druckspeicher |
| Luftbehälter | Druckluftbehälter |
| Energiequelle 1 / 2 | Druckquelle hydraulisch bzw. pneumatisch |
| Belüfteter Behälter | offener Tank |
| Dichter Behälter | geschlossener Tank |
| Filter | Filter |
| Trennzeichen | Abscheider (Fehlübersetzung) |
| Trockner/Öler | Trockner bzw. Öler |
| Belüftungss… | Schalldämpfer an Entlüftung |
| Wärmetaus… | Wärmetauscher |
| Druckmess… | Manometer |
| Thermomet… | Thermometer |
| Flussanzeige | Durchflussanzeige |
| Flussmessg… | Durchflussmesser |
| Drehzahlme… | Drehzahlmesser |
| Schalter/Messgröße… | Druckschalter/Messumformer |
| Impulszähler 1 / 2 | Impulszähler |
| Dämpfer | Dämpfer |
| Abfluss | Rücklauf zum Tank |

#### SH-080 Pneumatik/Hydraulik - Ventilanordnung

Baukasten für Wegeventile: Ventilkörper mit Schaltfeldern, Strömungswege, Betätigungen und Leitungselemente.
Kennzeichnung: Norm: ISO 1219-1.

| Shape | Beschreibung |
|---|---|
| 2 Stellung 2,3,4 Anschl. | Ventilkörper mit zwei Feldern, bis vier Anschlüsse |
| 2 Stellungen 5 Anschl. | zwei Felder, fünf Anschlüsse |
| 3 Stellungen 2,3,4 Anschl. / 5 Anschl. | drei Felder |
| 4 Stellungen 2,3,4 Anschl. / 5 Anschl. | vier Felder |
| Feld | einzelnes Schaltfeld |
| 2 Anschl. / 2 Anschl. geschl. | Feldeinsatz zwei Anschlüsse offen bzw. gesperrt |
| 3 Anschl. / 3 Anschl. überkreuzt | Feldeinsatz drei Anschlüsse |
| 4 Anschl. / geschl. / gekreuzt / offen / halbverb. / überkreuzt | Feldeinsätze vier Anschlüsse |
| 4-Anschl.-T… | Feldeinsatz mit T-Verbindung |
| 5 Anschl. / geschl. / überkreuzt | Feldeinsätze fünf Anschlüsse |
| Feder | Federrückstellung |
| Kolben | Druckbetätigung |
| Walze | Rollenbetätigung |
| Manuelle Korrektur | Handbetätigung |
| Druck-/Zug… | Druck-/Zugknopf |
| Hebel | Hebelbetätigung |
| Pedal/Fußhebel | Fußbetätigung |
| Elektr. Rotor | Elektromotor-Betätigung |
| Elektr. (line…) | Magnetbetätigung |
| Fernbedient | Fernsteuerung |
| Sperre | Raste |
| Verbindung… | Leitungsverbindung |
| Verbind./Kreuzung | Kreuzung mit Verbindung |
| Flexible Linie | Schlauchleitung |
| Entlüftung | Entlüftung |
| Pneum./Hyd. Energie | Energieflussrichtung |
| Abluftauslass | Auslass ins Freie |
| Pneu./Hyd. Fluss | Strömungsrichtung |
| Drehverbin… | Drehdurchführung |
| Variabler Pfeil | Einstellbarkeit |
| Pfeil gekrümmt | Drehrichtung |
| Pfeil | Richtungspfeil |
| Schaft | Welle |
| Stab | Stange |
| Positionsre… | Positionsraste |
| Riegel | Verriegelung |
| Geschl. Pfad / Geschl. Pfad (2) | gesperrter Weg |
| Elektrisch | elektrische Leitung |
| Reduzierung | Drosselstelle |
| Temperatur | Temperaturkennzeichen |

#### SH-081 Pneumatik/Hydraulik - Ventile

Fertige Wege-, Sperr-, Druck- und Stromventile. Kennzeichnung: Norm: ISO 1219-1.

| Shape | Beschreibung |
|---|---|
| 2/2-Ventil | zwei Anschlüsse, zwei Stellungen |
| 3/2-Ventil | drei Anschlüsse, zwei Stellungen |
| 4/2-Ventil | vier Anschlüsse, zwei Stellungen |
| 5/2-Ventil | fünf Anschlüsse, zwei Stellungen |
| 4/3 Ventil | vier Anschlüsse, drei Stellungen |
| 5/3-Ventil | fünf Anschlüsse, drei Stellungen |
| Reduzierve… | Drosselventil |
| Absperrschi… | Absperrventil |
| Einwegredu… | Drosselrückschlagventil |
| Strömungs… | Stromventil |
| Strömungst… | Stromteiler |
| Rückschlag | Rückschlagventil |
| Kupplung (verb.) / Kupplung (gelöst) | Schnellkupplung gesteckt bzw. getrennt |
| Wechselven… | Wechselventil (ODER) |
| Schnellausl… | Schnellentlüftungsventil |
| Kolbenventil | Sitzventil mit Kolben |
| Überdr.ventil / Überdr.ventil 2 | Druckbegrenzungsventil |
| Druckmind… | Druckregelventil |
| Überdr.-ventil (E) | Druckbegrenzung, elektrisch einstellbar |

#### SH-082 Schrauben 1

Schrauben und Muttern in Seitenansicht und Draufsicht („oben“). Kennzeichnung: Norm: vereinfachte
Normteil-Darstellung (ISO 128, ISO 6410).

| Shape | Beschreibung |
|---|---|
| Vierkantsch… / Vierkant oben | Vierkantschraube seitlich bzw. von oben |
| Vierkantmu… / Vierkantmu… oben | Vierkantmutter seitlich bzw. von oben |
| Sechskants… / Sechskant oben | Sechskantschraube |
| Sechskant… (Mutter) / Sechskant… (oben) | Sechskantmutter |
| Sechskanth… | Hutmutter |
| Innensechs… / Innensechs… oben | Zylinderschraube mit Innensechskant |
| Sechskantg… | Gewindestift mit Innensechskant |
| Flügelschra… / Flügelschra… oben | Flügelschraube |
| Flügelmutt… (2) / Flügelmutter oben | Flügelmutter |
| Flachkopf (Schlitz) / Flachkopf oben | Flachkopfschraube mit Schlitz |
| Halbrund (Schlitz) / Halbrund oben | Halbrundschraube mit Schlitz |
| Senkschr. (Schlitz) / Senkschr. oben | Senkschraube mit Schlitz |
| Lins-senk. (Schlitz) / Linsensenk… oben | Linsensenkschraube mit Schlitz |
| Zylinderkopf (Schlitz) (2) / Zylinder oben / Zylinderkopf oben | Zylinderschraube mit Schlitz |
| Flachkopf (Kreuzschlitz) | Flachkopfschraube mit Kreuzschlitz |
| Lins-senk. (Kreuzschlitz) | Linsensenkschraube mit Kreuzschlitz |
| Senkschr. (Kreuzschlitz) | Senkschraube mit Kreuzschlitz |
| Kreuzschlitz oben 1 / 2 | Kreuzschlitzkopf von oben |
| Sechskanta… | Sechskant-Ansatzschraube |
| Ansatz (Schlitz) | Ansatzschraube mit Schlitz |

#### SH-083 Schrauben 2

Gewindestifte, Niete, Scheiben und Stifte. Kennzeichnung: Norm: vereinfachte Normteil-Darstellung (ISO 128, ISO 6410).

| Shape | Beschreibung |
|---|---|
| Vierkantge… / Vierkant-GS oben | Gewindestift mit Vierkant |
| Gew.-Stift (Schlitz) / GSSchlitz oben | Gewindestift mit Schlitz |
| Sechskantg… / SechskantGS oben | Gewindestift mit Sechskant |
| Riffelgew.-S… / Riffel-GS oben | Gewindestift mit Rändel |
| Sechskantk… / Sechskantk… oben | Sechskantkopf |
| Halbrundniet | Halbrundniet |
| Halbrundniet flach | flacher Halbrundniet |
| Flachrundni… | Flachrundniet |
| Flachkopfniet | Flachkopfniet |
| Bundniet | Niet mit Bund |
| Senkniet 1 / 2 | Senkniet |
| Senkkopfniet | Senkkopfniet |
| Unterlegsch… / U.-Scheibe oben | Unterlegscheibe |
| U.-Scheibe mit Fase / U.-Scheibe m. F. ob. | Scheibe mit Fase |
| Federringdi… / Federringdi… oben | Federring |
| Kegelstift | Kegelstift |
| Bolzen | Bolzen |
| Kugelkolben | Kugeldruckstück |

#### SH-084 Schweißsymbole

Schweißnaht-Zeichen mit Bezugspfeil und Zusatzzeichen. Kennzeichnung: Norm: ISO 2553.

| Shape | Beschreibung |
|---|---|
| Pfeil / Pfeil mit Krümmung / Zusätzlicher Pfeil | Bezugspfeil mit Bezugslinie |
| Punkt | Punktnaht |
| Widerstand… | Rollennaht |
| Schlitz/Loch | Loch- oder Schlitznaht |
| Bolzen | Bolzenschweißung |
| Konturen / Konturen gewinkelt | Oberflächenform der Naht |
| Textblock | Maß- und Textangabe |
| Aussparung | Kehlnaht-Zeichen (Dreieck) |
| Rückwärtss… | Gegenlage |
| Auftragssch… | Auftragschweißung |
| Flanschecke / Flanschkante | Bördelnaht an Ecke bzw. Kante |
| Flache Fuge | I-Naht |
| V-Fuge | V-Naht |
| HV-Fuge | HV-Naht |
| U-Fuge | U-Naht |
| J-Fuge | HU-Naht |
| V-Fuge, Bördel / HV-Fuge, Bördel | Bördel-V- bzw. Bördel-HV-Naht |
| Überlappt | Schrägstoß |
| Durchschm… | Durchschweißung |
| Montagesc… | Montagenaht (Fähnchen) |
| Unterlage/… | Schweißbadsicherung |
| Einfügen | Einlegeteil |

**Verfahrenstechnik**

#### SH-085 Geräte - Behälter

Behälter, Tanks und Kolonnen für Fließschemata. Kennzeichnung: Norm: ISO 10628.

| Shape | Beschreibung |
|---|---|
| Behälter | stehender Behälter mit gewölbten Böden |
| Spalte | Kolonne (Fehlübersetzung von „Column“) |
| Autoklav | Druckbehälter mit Rührwerk |
| Tank | Tank mit Dach |
| Etagenkolo… | Bodenkolonne |
| Behälter m. Flüss.kontakt | Behälter mit Füllkörpern |
| Reaktionsb… | Reaktor |
| Offener Tank | Becken ohne Deckel |
| Klärbehälter | Absetzbehälter |
| Geschloss. Tank | geschlossener Tank |
| Tank mit Deckel | Tank mit Schwimmdeckel |
| Gasbehälter | Gasometer |
| Kugelbehält… | Kugeltank |
| Fass | Fass |
| Gasflasche | Druckgasflasche |
| Sack | Sack |
| Transportb… | Transportbehälter |
| Kolonnenb… (gestr.) / (mass.) | Kolonnenboden gestrichelt bzw. durchgezogen |
| Wasserober… | Flüssigkeitsspiegel |
| Zweigarmat… | Stutzen |
| Zugriffspunkt | Mannloch |
| Geflanschter Zugang | Flanschstutzen |

#### SH-086 Geräte - Pumpen

Pumpen, Verdichter, Lüfter und Turbinen. Kennzeichnung: Norm: ISO 10628.

| Shape | Beschreibung |
|---|---|
| Inlinepumpe | Pumpe in der Leitung |
| Zentrifugal… | Kreiselpumpe |
| Verdichter 1 / 2 / 3 | Verdichter in drei Formen |
| Ventilator 1 (wählbar) / Ventilator 2 (wählbar) | Ventilator, Bauart umschaltbar |
| Verdränger… | Verdrängerpumpe |
| Drehkolben… 1 / 2 | Drehkolbenpumpe |
| Dosierpumpe | Dosierpumpe |
| Pumpe 1 (wählbar) / Pumpe 2 / 3 (auswählbar) | Pumpe, Bauart umschaltbar |
| Rotationsve… | Rotationsverdichter |
| Motorturbine | Turbine mit Antrieb |
| Verdichter/Turbine | Verdichter oder Turbine |
| Kolbenpum… -verdichter | Kolbenpumpe bzw. -verdichter |
| Hubkolben… 2 | Hubkolbenpumpe |
| Fliehkraftlüf… | Radiallüfter |
| Axiallüfter 1 / 2 | Axiallüfter |
| Ejektor/Injektor | Strahlpumpe |
| Zerstäuber | Sprühdüse |
| Gebläseflügel / 3 Gebläseflügel | Lüfterrad |

#### SH-087 Geräte - Wärmetauscher

Wärmeübertrager, Kessel, Kühltürme und Verdampfer. Kennzeichnung: Norm: ISO 10628.

| Shape | Beschreibung |
|---|---|
| Wärmetaus… 1 / 2 | Wärmeübertrager allgemein |
| Boiler | Kessel |
| Kühlturm 1 / 2 / Kühlturm | Kühlturm in drei Formen |
| Kondensator | Kondensator |
| Kondensator (luftgekühlt) | Luftkondensator |
| Röhren- WA | Rohrbündel-Wärmeübertrager |
| Rohrbündel 1 / 2 | Rohrbündel-Wärmeübertrager |
| Kessel-WA | Kessel-Wärmeübertrager |
| Luftkühler | Luftkühler |
| Platten- WA | Plattenwärmeübertrager |
| Rippenrohr-WA | Rippenrohr-Wärmeübertrager |
| Doppelrohr-WA | Doppelrohr-Wärmeübertrager |
| Ölbrenner | Brenner |
| Befeuerter Heizer | Röhrenofen |
| Autom. Rohrbeschi… | Rohrreinigung (automatisch) |
| Kühlschränke | Kühlaggregat |
| Verdampfer | Verdampfer |
| Verdampfer, Wasser / Verdampfer, Luft | wasser- bzw. luftbeheizter Verdampfer |
| Ölabscheider | Ölabscheider |
| Abziehhaube | Abzugshaube |
| Autoklav | Autoklav |
| Gebläseflügel / 3 Gebläseflügel | Lüfterrad |

#### SH-088 Instrumente

Messstellen, Leitsystem-Funktionen und Wirkungslinien für R&I-Fließbilder. Kennzeichnung: Norm: ISA 5.1, IEC 62424.

| Shape | Beschreibung |
|---|---|
| Indikator | Messstellenkreis |
| Anzeigeme… 2 | Messstellenkreis mit Halterung |
| Druckmesser | Rechteck-Instrument „P“ |
| Thermomet… | Rechteck-Instrument „T“ |
| Bildschirm / Bildschirm 2 | Funktion im Leitsystem (Kreis im Quadrat) |
| SPS / SPS 2 | Funktion in einer Steuerung (Raute im Quadrat) |
| Computer / Computer 2 | Funktion im Rechner (Sechseck) |
| Licht | Signalleuchte |
| Gestrich. Mittellinie | Instrument hinter der Schalttafel |
| Dampfsensor | Messfühler |
| Doppelt | zwei Funktionen in einer Stelle |
| Füllstands… (2) | Füllstandsmessung |
| Durchfluss… (2) | Durchflussmessung |
| Anzeige/Aufnahme | Anzeige mit Aufzeichnung |
| Konverter | Signalumformer |
| Venturi | Venturidüse |
| Rotameter | Schwebekörper-Durchflussmesser |
| Wirbelsensor | Wirbelzähler |
| Propellerm… | Flügelradzähler |
| Allgemeine Einheit | Gerät allgemein |
| Bedienerfeld | Bedienfeld |
| UND-Schaltung / ODER-Schaltung / NICHT-Schaltung | Verknüpfungen |
| Korrekturel… | Stellglied |
| Raute | Raute (Verriegelung) |
| Signal | Wirkungslinie allgemein |
| Daten | Datenverbindung |
| Pneumatisch / 2 / binär | pneumatisches Signal |
| Elektrisch / 2 / 3 / binär / binär 2 | elektrisches Signal |
| Kapillarrohr | Kapillarleitung |
| Hydraulisch / 2 | hydraulisches Signal |
| Elektromag… / 2 | elektromagnetisches Signal |
| Erhitzt/Gekühlt | beheizte oder gekühlte Leitung |
| Wärmeregu… | Begleitheizung |
| Mechanisch / 2 | mechanische Verbindung |

#### SH-089 Prozessanmerkungen

Beschriftungen, Textfelder und automatische Listen für Fließbilder. Kennzeichnung: gemischt – Generisch (Text,
Beschriftungen, Listen), Norm: ISO 10628 (Schnittstellen).

| Shape | Beschreibung |
|---|---|
| Schnittstelle 1 / 2 | Übergabestelle an einer Leitung |
| Neigung | Gefällekennzeichen |
| Beschrift.-P… 1 / 2 / 3 | Beschriftungsfahne in drei Formen |
| Beschriftung 1 / 2 / 3 / 4 / 5 / 6 | Hinweisfeld mit Zeiger in sechs Formen |
| Text 6 Pt. / 8 Pt. Text / 10 Pt. Text | Textfeld fester Größe |
| Text 6 / 8 / 10 Pt. mit Linie | Text mit Bezugslinie |
| Zubehörliste | Tabelle aller Zubehör-Shapes der Zeichnung |
| Rohrleitung… | Tabelle aller Rohrleitungen |
| Ventilliste | Tabelle aller Ventile |
| Messgerätel… | Tabelle aller Messstellen |

#### SH-090 Rohrleitungen

Rohrleitungen als Verbinder, rechtwinklig geführt. Kennzeichnung: Norm: ISO 10628.

| Shape | Beschreibung |
|---|---|
| Hauptleitung | dicke Leitung ohne Flusspfeil |
| Hauptleitung R / L | dicke Leitung mit Flusspfeil am Ende bzw. Anfang |
| Nebenleitung | dünne Leitung ohne Flusspfeil |
| Nebenleitung R / L | dünne Leitung mit Flusspfeil |

#### SH-091 Ventile und Armaturen

Ventile, Rohrverbindungen und Rohrzubehör für R&I-Fließbilder; Ventile setzen sich in die Leitung. Kennzeichnung:
Norm: ISO 10628, ISA 5.1.

| Shape | Beschreibung |
|---|---|
| Schieberve… | Absperrschieber |
| Kugelventil | Kugelhahn-Symbol |
| Rückschlag… | Rückschlagventil |
| Gesteuertes Ventil | Ventil mit Antrieb |
| Niederschra… | Absperrventil mit Spindel |
| Absperrventil | Absperrventil |
| Membranv… | Membranventil |
| Nadelventil | Nadelventil |
| Überdruckv… | Sicherheitsventil |
| Eckventil | Eckventil |
| Schwimmer… | Schwimmerventil |
| Flanschventil | Ventil mit Flanschen |
| Drosselklap… | Drosselklappe |
| Keilschieb./Zylinderhahn | Keilschieber bzw. Hahn |
| Kugelhahn | Kugelhahn |
| Druck (Eckventil) | Eck-Sicherheitsventil |
| Reduzierve… | Druckminderer |
| Kegelventil | Kükenhahn |
| 3-Wege-Ke… | Dreiwegehahn |
| Mischventil | Mischventil |
| Wegeventil | Wegeventil |
| Ventilverteiler | Ventilblock |
| Reduzierstü… | Reduzierung |
| Allgemeine Verbindung | Rohrverbindung allgemein |
| Stumpfsch… | Stumpfschweißung |
| Geflanscht/… | Flanschverbindung |
| Gelötet | Lötverbindung |
| Schraubver… | Schraubverbindung |
| Stemmmuffe | Stemmmuffe |
| Muffenverb… | Muffenverbindung |
| Schweißmu… | Schweißmuffe |
| Drehgelenk… | Drehgelenk |
| Rohrverschl… 1 / 2 | Rohrende verschlossen |
| Leitend verbunden | elektrisch leitende Verbindung |
| Elektrisch isoliert | Isolierstück |
| Berstscheibe | Berstscheibe |
| Flammensp… | Flammensperre |
| Filter | Filter |
| Abscheider | Abscheider |
| Schalldämp… | Schalldämpfer |
| Trichter | Trichter |
| Glocke | Glockenförmige Erweiterung |
| Auspuffstut… | Abgasstutzen |
| Entlüftung offen | offene Entlüftung |
| Siphon | Siphon |
| Hydrant | Hydrant |
| Abflussdäm… | Abflussdämpfer |
| Y-Filter | Schmutzfänger |
| Flüss.-Dicht. offen/geschl. | Steckscheibe offen/geschlossen |

#### SH-092 Zubehör - Allgemein

Maschinen und Fördertechnik für Fließschemata. Kennzeichnung: Norm: ISO 10628.

| Shape | Beschreibung |
|---|---|
| Brecher | Brecher allgemein |
| Brechmasc… | Brechmaschine |
| Walzenbrec… | Walzenbrecher |
| Hammerbr… | Hammerbrecher |
| Versch. Brecher 1 / 2 | weitere Brecherbauarten |
| Kugelmühle | Kugelmühle |
| Versch. Mühlen | weitere Mühlen |
| Mischer | Mischer allgemein |
| Versch. Mischer | weitere Mischer |
| Bandmischer | Bandmischer |
| Doppelmisc… | Doppelmischer |
| Flüssigkeits… | Flüssigkeitsmischer |
| Kneter / Versch. Kneter | Kneter |
| Zyklon 1 / 2 | Fliehkraftabscheider |
| Filter 1 / 2 | Filter |
| Drehfilter | Trommelfilter |
| Bildschirm | Sieb (Fehlübersetzung von „Screen“) |
| Elektromag… | Magnetabscheider |
| Zentrifuge 1 / 2 | Zentrifuge |
| Brikettierm… | Brikettpresse |
| Spritzturm | Sprühturm |
| Trockner | Trockner |
| Förderband | Bandförderer |
| Förderrad | Becherrad |
| Ladevorrich… mit Ausleger | Verlader mit Ausleger |
| Kratzband | Kratzförderer |
| Förderschn… | Schneckenförderer |
| Aufzug/Hebezeug 1 / 2 | Becherwerk/Aufzug |
| Kippkübela… | Kippkübelaufzug |
| Deckenförd… | Hängeförderer |
| Hebezeuge | Kran/Hebezeug |
| Elektromotor | Motor |
| Tankfahrzeug | Straßentankwagen |
| Tankwagen | Kesselwagen |

### 4.7 Netzwerk

**Active Directory**

#### SH-093 Active Directory-Objekte

Objekte eines Verzeichnisdienstes für Struktur- und Berechtigungspläne. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Domäne | Domäne als Gebäudeblock |
| Computer | Computerkonto |
| Benutzer | Benutzerkonto |
| Gruppe | Benutzergruppe |
| Container | gelber Ordner als Container |
| Druckerwar… | Druckerwarteschlange |
| Kontakt | Kontaktkarte |
| Organisatio… | Organisationseinheit |
| Richtlinie | Gruppenrichtlinie |
| Lautstärke | freigegebener Ordner (Fehlübersetzung von „Volume“) |
| Allgemeines Objekt | Objekt ohne Typ |
| Standort | Standort |
| Standortver… | Standortverknüpfung |
| SO-Verknü… brücke | Standortverknüpfungsbrücke |
| Server | Server |
| NTDS-Stan… | Standorteinstellungen |
| IP-Subnetz | Subnetz |
| Zertifikatsv… | Zertifikatvorlage |
| Lizenzstand… | Lizenzierungsstandort |
| Verbindung | Replikationsverbindung |
| Verzeichnis… | Verzeichnisstruktur |

#### SH-094 Active Directory-Standorte und -Dienste

Standorte, Domänen und Replikation für Verzeichnistopologien. Kennzeichnung: gemischt – Visio-Design (3D-Symbole,
Client), Generisch (2D-Grundformen).

| Shape | Beschreibung |
|---|---|
| Standort o. Subnetz | graue Ellipse für Standort |
| 2D-Standort o. -Subnetz | flache Standort-Ellipse |
| 3D-Standor… | gelbe 3D-Scheibe für Standort |
| SO-Verknü… brücke 3D | Standortverknüpfungsbrücke in 3D |
| WAN | Weitverkehrsverbindung |
| 3D-Domäne / 2D-Domäne | Dreieck für Domäne |
| 3D-Domän… / 2D-Domän… | Domänencontroller |
| Replikation… | Doppelpfeil für Replikation |
| Datenbank | grüner Zylinder |
| 3D-Anwend… / 2D-Anwend… | Kreis für Anwendungspartition |
| DFÜ-Verbin… | Wählverbindung (Blitz) |
| Client | Arbeitsplatzrechner |

**AWS** – alle mit Kennzeichnung AWS-Original (CC BY-ND 2.0); ausgeliefert werden die Original-Icons (SH-G06).
„Dienst“ = Icon eines AWS-Dienstes, „Ressource“ = Icon einer Ressource oder eines Konzepts.

#### SH-095 AWS Allgemein

Allgemeine Ressourcen-Icons für AWS-Architekturen.

| Shape | Beschreibung |
|---|---|
| AWS E-Mail | Ressource: E-Mail |
| AWS Marketplace | Dienst: Software-Marktplatz |
| Client | Ressource: Arbeitsplatzrechner |
| Datenträger | Ressource: Datenträger/Disk |
| Foren | Ressource: Diskussionsforum |
| Allgemein | allgemeines AWS-Dienstsymbol |
| Generische Datenbank | Ressource: Datenbank |
| Generische Firewall | Ressource: Firewall |
| Internet alt1 / alt2 | Ressource: Internet als Wolke |
| Internetgat… | Ressource: Internet-Gateway |
| Mobiler Client | Ressource: Smartphone |
| Multimedia | Ressource: Medieninhalt |
| Bürogebäude | Ressource: Firmenstandort |
| SAML-Token | Ressource: Anmelde-Token |
| SDK | Ressource: Entwicklerpaket |
| SSL-Vorhän… | Ressource: verschlüsselte Verbindung |
| Bandspeicher | Ressource: Bandlaufwerk |
| Toolkit | Ressource: Werkzeugsatz |
| Herkömmli… Server | Ressource: klassischer Server vor Ort |
| Benutzer | Ressource: einzelner Nutzer |
| Benutzer.45 | Ressource: Nutzergruppe |

#### SH-096 AWS AR VR

| Shape | Beschreibung |
|---|---|
| Amazon Sumerian | Dienst: 3D-, AR- und VR-Anwendungen erstellen |
| AR VR | Kategorie-Icon AR/VR |

#### SH-097 AWS Compute

Rechendienste, EC2-Instanztypen und Container-Ressourcen.

| Shape | Beschreibung |
|---|---|
| Amazon EC2 | Dienst: virtuelle Server |
| Amazon EC2 Container-… | Dienst: Container-Registry |
| Amazon Elastic Co… | Dienst: Container-Orchestrierung (ECS) |
| Amazon Elastic Kub… | Dienst: verwaltetes Kubernetes (EKS) |
| Amazon Lightsail | Dienst: einfache virtuelle Server |
| AWS Batch | Dienst: Stapelverarbeitung |
| AWS Compu… | Dienst: Empfehlungen zur Instanzgröße |
| AWS Elastic Beanstalk | Dienst: Anwendungen bereitstellen |
| AWS Elastic Beanstalk-A… | Ressource: Beanstalk-Anwendung |
| AWS Elastic Beanstalk-B… | Ressource: Beanstalk-Bereitstellung |
| AWS Fargate | Dienst: Container ohne eigene Server |
| AWS Lambda | Dienst: Funktionen ohne Server |
| Lokale AWS-Zonen | Dienst: Rechenleistung nahe am Nutzer |
| AWS Outposts | Dienst: AWS-Hardware im eigenen Rechenzentrum |
| AWS Serverless… | Dienst: Katalog serverloser Anwendungen |
| AWS Thinkb… (7) | Dienste der Thinkbox-Familie für Rendering |
| AWS Wavelength | Dienst: Rechenleistung im Mobilfunknetz |
| Bottlerocket | Dienst: Betriebssystem für Container |
| EC2 Image Builder | Dienst: Server-Images bauen |
| VMware Cloud … | Dienst: VMware auf AWS |
| Parallel Cluster | Dienst: Rechencluster für HPC |
| Compute | Kategorie-Icon Rechnen |
| Automatisc… Skalierung | Ressource: automatische Skalierung |
| Automatisc… Skalierung … | Ressource: Skalierungsgruppe |
| AMI | Ressource: Maschinen-Image |
| Elastic IP-Adresse | Ressource: feste öffentliche IP |
| Instanz / Instanzen | Ressource: eine bzw. mehrere Instanzen |
| Instanz mit CloudWatch | Ressource: überwachte Instanz |
| Optimierte Instanz | Ressource: optimierte Instanz |
| Spot-Instanz | Ressource: günstige Überschuss-Instanz |
| DB auf Instanz | Ressource: Datenbank auf eigener Instanz |
| Rescue | Ressource: Rettungsinstanz |
| A1-Instanz | Instanztyp ARM-basiert |
| C4- / C5- / C5n-Instanz | Instanztypen rechenoptimiert |
| D2-Instanz / H1-Instanz / I3-Instanz | Instanztypen speicheroptimiert |
| F1-Instanz | Instanztyp mit FPGA |
| G3-Instanz / P2-Instanz / P3-Instanz | Instanztypen mit Grafikprozessor |
| High Memory-In… | Instanztyp mit sehr viel Speicher |
| M4- / M5- / M5a-Instanz | Instanztypen allgemein |
| R4- / R5- / R5a-Instanz | Instanztypen arbeitsspeicheroptimiert |
| T3-Instanz light bg …, T3-Instanz (Icon „T2“), T3a-Instanz | Instanztypen mit Leistungsreserve |
| X1- / X1e- / z1d-Instanz | Instanztypen für große Speicher bzw. hohe Taktung |
| Container- / Container1 / Container2 / Container3 | Ressource: Container |
| Registry | Ressource: Container-Registry |
| Dienst | Ressource: Container-Dienst |
| Aufgabe | Ressource: Container-Aufgabe |
| Lambda-Fu… | Ressource: Lambda-Funktion |

#### SH-098 AWS Endbenutzer-Computing

| Shape | Beschreibung |
|---|---|
| Amazon Appstrea… | Dienst: Anwendungen per Streaming |
| Amazon WorkDocs | Dienst: Dokumentenablage |
| Amazon WorkLink | Dienst: mobiler Zugriff auf Intranet |
| Amazon Workspaces | Dienst: virtuelle Desktops |
| Endbenutze… | Kategorie-Icon Endbenutzer-Computing |

#### SH-099 AWS Internet der Dinge

IoT-Dienste und viele Ressourcen-Icons für Geräte und Protokolle.

| Shape | Beschreibung |
|---|---|
| Amazon FreeRTOS | Dienst: Echtzeit-Betriebssystem für Mikrocontroller |
| AWS IoT 1-Click | Dienst: Ein-Knopf-Geräte |
| AWS IoT Analytics | Dienst: Auswertung von Gerätedaten |
| AWS IoT Schaltfläche | Ressource: IoT-Taster |
| AWS IoT Core | Dienst: Geräteverbindung und Nachrichten |
| AWS IoT Device … | Dienst: Geräteverwaltung bzw. -schutz |
| AWS IoT Events | Dienst: Ereigniserkennung |
| AWS IoT Greengrass | Dienst: lokale Ausführung auf Geräten |
| AWS IoT SiteWise | Dienst: Industriedaten erfassen |
| AWS IoT Things Graph | Dienst: Geräte verknüpfen |
| Internet der Dinge | Kategorie-Icon IoT |
| Kanal | Ressource: Datenkanal |
| Verbinder | Ressource: Verbindung |
| Dataset | Ressource: Datensatz |
| Datenspeic… | Ressource: Datenspeicher |
| Pipeline | Ressource: Verarbeitungskette |
| Notebook | Ressource: Analyse-Notizbuch |
| IoT Aktion | Ressource: Aktion einer Regel |
| IoT Actuator | Ressource: Stellglied |
| IoT Sensor | Ressource: Sensor |
| IoT Servo | Ressource: Servomotor |
| IoT Regel | Ressource: Regel |
| IoT Richtlinie | Ressource: Berechtigungsrichtlinie |
| IoT Certifica… | Ressource: Gerätezertifikat |
| IoT Shadow | Ressource: Geräteschatten |
| IoT erwünscht… | Ressource: gewünschter Zustand |
| IoT gemeldet… | Ressource: gemeldeter Zustand |
| IoT Thema | Ressource: Nachrichtenthema |
| IoT Geräte-Gate… | Ressource: Geräte-Gateway |
| IoT Geräte-Jo… | Ressource: Auftrag an Geräte |
| IoT Hardware-B… | Ressource: Platine |
| IoT Simulator | Ressource: Gerätesimulator |
| IoT Hilfsprogra… | Ressource: Hilfsprogramm |
| IoT Lambda-Fu… | Ressource: Funktion |
| IoT Over-the-Ai… | Ressource: Aktualisierung per Funk |
| IoT HTTP-Prot… / IoT HTTP 2-Protokoll | Ressource: HTTP bzw. HTTP/2 |
| IoT MQTT-Prot… | Ressource: MQTT |
| IoT Alexa aktiviert… | Ressource: Gerät mit Sprachsteuerung |
| IoT Alexa-Skill | Ressource: Sprach-Skill |
| IoT Alexa-Sprac… | Ressource: Sprachdienst |
| IoT Echo | Ressource: smarter Lautsprecher |
| IoT Fire-TV / IoT Fire-TV-Stick | Ressource: Streaming-Gerät |
| IoT Allgemein | Ressource: Gerät allgemein |
| IoT Bank | Ressource: Bank |
| IoT Bicycle | Ressource: Fahrrad |
| IoT Camera | Ressource: Kamera |
| IoT Auto | Ressource: Fahrzeug |
| IoT Warenkorb | Ressource: Einkaufswagen |
| IoT Kaffeemasc… | Ressource: Kaffeemaschine |
| IoT Türschloss | Ressource: Türschloss |
| IoT Factory | Ressource: Fabrik |
| IoT Haus | Ressource: Haus |
| IoT Glühlampe | Ressource: Lampe |
| IoT medizinisch… | Ressource: Medizingerät |
| IoT Polizei-Notr… | Ressource: Notruf |
| IoT Thermostat | Ressource: Thermostat |
| IoT Reisen | Ressource: Reisen |
| IoT Windpark | Ressource: Windpark |

#### SH-100 AWS Machine Learning

| Shape | Beschreibung |
|---|---|
| Amazon Augment… | Dienst: menschliche Prüfung von KI-Ergebnissen |
| Amazon CodeGuru | Dienst: Code-Prüfung mit KI |
| Amazon Comprehend | Dienst: Textanalyse |
| Amazon Elastic I… | Dienst: KI-Beschleunigung für Instanzen |
| Amazon Forecast | Dienst: Vorhersagen aus Zeitreihen |
| Amazon Fraud … | Dienst: Betrugserkennung |
| Amazon Kendra | Dienst: Unternehmenssuche |
| Amazon Lex | Dienst: Chatbots |
| Amazon Personalize | Dienst: Empfehlungen |
| Amazon Polly | Dienst: Text zu Sprache |
| Amazon Rekognition | Dienst: Bild- und Videoerkennung |
| Amazon Rekognitio… (2) | Ressource: Bild- bzw. Videoerkennung |
| Amazon SageMaker | Dienst: Modelle bauen und trainieren |
| Amazon SageMaker … | Dienst: Daten beschriften (Ground Truth) |
| Amazon SageMaker-… | Ressource: Modell |
| Amazon SageMak… (2) | Ressource: Notebook bzw. Training |
| Amazon Textract | Dienst: Text aus Dokumenten lesen |
| Amazon Transcribe | Dienst: Sprache zu Text |
| Amazon Translate | Dienst: Übersetzung |
| Apache MXNet a… | Dienst: Lernbibliothek MXNet |
| AWS Deep Learning… | Dienst: vorbereitete Lern-Images |
| AWS Deep Learning-C… | Dienst: vorbereitete Lern-Container |
| AWS DeepComp… | Dienst: KI-Musikkomposition zum Lernen |
| AWS DeepLens | Dienst: Lern-Kamera |
| AWS DeepRacer | Dienst: Lern-Rennauto |
| Machine Learning | Kategorie-Icon maschinelles Lernen |
| TensorFlow auf AWS | Dienst: Lernbibliothek TensorFlow |

#### SH-101 AWS Mobile

| Shape | Beschreibung |
|---|---|
| Amazon API-Gatewa… | Ressource: API-Endpunkt |
| Amazon API-Gateway | Dienst: Schnittstellen veröffentlichen |
| Amazon Pinpoint | Dienst: Nutzeransprache per Nachricht |
| AWS Amplify | Dienst: Web- und Mobil-Apps bauen |
| AWS AppSync | Dienst: GraphQL-Schnittstellen |
| AWS Geräte-Farm | Dienst: Apps auf echten Geräten testen |
| Mobilgerät | Ressource: Smartphone |

#### SH-102 AWS Networking und Inhaltsbereitstellung

| Shape | Beschreibung |
|---|---|
| Amazon API Gateway-En… | Ressource: API-Endpunkt |
| Amazon API Gateway | Dienst: Schnittstellen veröffentlichen |
| Amazon CloudFront | Dienst: Inhaltsauslieferung (CDN) |
| Amazon Route 53 | Dienst: DNS |
| Amazon VPC | Dienst: virtuelles privates Netz |
| AWS App Mesh | Dienst: Service-Mesh |
| AWS Client VPN | Dienst: VPN für Nutzer |
| AWS Cloud Map | Dienst: Dienstverzeichnis |
| AWS Direct Connect | Dienst: Standleitung zu AWS |
| AWS Global Accelerator | Dienst: globale Beschleunigung |
| AWS PrivateLink | Dienst: private Dienstanbindung |
| AWS Site to Site VPN | Dienst: VPN zwischen Standorten |
| AWS Transit Gateway | Dienst: zentraler Netzknoten |
| Elastic Load Balancing | Dienst: Lastverteilung |
| Networking und Inhalts… | Kategorie-Icon Netzwerk |
| Anwendun… | Ressource: Application Load Balancer |
| Netzwerklas… | Ressource: Network Load Balancer |
| Klassischer Load Bal… | Ressource: klassischer Lastverteiler |
| Kunden-Ga… | Ressource: Gateway beim Kunden |
| Download-… | Ressource: Download-Verteilung |
| Streamingv… | Ressource: Streaming-Verteilung |
| Microsoft Edge-Speic… | Ressource: Edge-Standort (Fehlübersetzung) |
| Elastic Netwo… | Ressource: Netzwerkadapter |
| Elastic Network-Sc… | Ressource: Netzwerkschnittstelle |
| Endpunkte | Ressource: VPC-Endpunkt |
| Flow Logs | Ressource: Verkehrsprotokolle |
| Gehostete Zone | Ressource: DNS-Zone |
| Internetgat… | Ressource: Internet-Gateway |
| NAT Gateway | Ressource: Adressumsetzung |
| Netzwerk-Z… | Ressource: Netzwerk-Zugriffsliste |
| Peering | Ressource: Netzkopplung |
| Routingtab… | Ressource: Routingtabelle |
| Router | Ressource: Router |
| Datenverke… | Ressource: Spiegelung von Datenverkehr |
| VPN-Verbin… | Ressource: VPN-Verbindung |
| VPN-Gatew… | Ressource: VPN-Gateway |

#### SH-103 AWS Quantum-Technologien

| Shape | Beschreibung |
|---|---|
| AWS Braket | Dienst: Zugang zu Quantenrechnern |
| Quantum Technologies | Kategorie-Icon Quantentechnik |

#### SH-104 AWS-Analysen

| Shape | Beschreibung |
|---|---|
| Amazon Athena | Dienst: SQL-Abfragen auf Dateien |
| Amazon CloudSearch | Dienst: Suchindex |
| Amazon Elasticsear… | Dienst: Such- und Analysecluster |
| Amazon EMR | Dienst: Big-Data-Cluster |
| Amazon EMR-Cluster | Ressource: EMR-Cluster |
| EMR-Modul / EMR-Modul MapR M3 / M5 / M7 | Ressource: EMR-Laufzeit bzw. MapR-Ausgaben |
| HDFS-Cluster | Ressource: verteiltes Dateisystem |
| Amazon Kinesis | Dienst: Datenströme |
| Amazon Kinesis Da… (3) | Dienste für Datenströme: Analyse, Weiterleitung, Streams |
| Amazon Kinesis Vid… | Dienst: Videoströme |
| Amazon Managed St… | Dienst: verwaltetes Kafka |
| Amazon Quicksight | Dienst: Berichte und Dashboards |
| Amazon Redshift | Dienst: Data Warehouse |
| Amazon Redshift D… (2) | Ressource: Rechen- bzw. Speicherknoten |
| Analytics | Kategorie-Icon Analysen |
| AWS Data Lake Resso… | Ressource: Data Lake |
| AWS Datenpipeli… | Dienst: Datenpipelines |
| AWS Glue | Dienst: Datenaufbereitung (ETL) |
| AWS Glue Crawler | Ressource: Erkennung von Datenschemata |
| AWS Glue-Daten… | Ressource: Datenkatalog |
| AWS Lake Formation | Dienst: Data Lake einrichten |
| Dokumente durchsuchen | Ressource: Suche in Dokumenten |

#### SH-105 AWS-Anwendungsintegration

| Shape | Beschreibung |
|---|---|
| Amazon AppSync | Dienst: GraphQL-Schnittstellen |
| Amazon EventBridge | Dienst: Ereignisbus |
| Amazon MQ | Dienst: Message-Broker |
| Amazon Simple N… | Dienst: Benachrichtigungen (SNS) |
| Amazon Simpl… | Dienst: Warteschlangen (SQS) |
| AWS Step-Funkti… | Dienst: Workflow-Steuerung |
| AWS Express Workflows | Ressource: schnelle Workflows |
| AWS Console Mobile App… | Dienst: Konsole als App |
| Anwendun… | Kategorie-Icon Anwendungsintegration |
| Benutzerdef… Ereignis-Bus | Ressource: eigener Ereignisbus |
| Standarder… | Ressource: Standard-Ereignisbus |
| Saas Ereignis-Bus | Ressource: Ereignisbus für SaaS |
| Ereignis | Ressource: Ereignis |
| E-Mail-Ben… | Ressource: Benachrichtigung per E-Mail |
| HTTP-Bena… | Ressource: Benachrichtigung per HTTP |
| Nachricht | Ressource: Nachricht |
| Warteschla… | Ressource: Warteschlange |
| Thema | Ressource: Thema für Abonnenten |

#### SH-106 AWS-Blockchain

| Shape | Beschreibung |
|---|---|
| Amazon Manage… | Dienst: verwaltete Blockchain |
| Amazon Quantu… | Dienst: unveränderliche Ledger-Datenbank |
| Blockchain-… | Ressource: Blockchain |
| Blockchain | Kategorie-Icon Blockchain |

#### SH-107 AWS-Container

Gruppen-Rahmen (Container) der AWS-Architektur-Icons; nehmen andere Shapes auf.

| Shape | Beschreibung |
|---|---|
| Region | Rahmen für eine AWS-Region |
| AWS Cloud / AWS Cloud 1 | Rahmen für die AWS-Cloud |
| Unternehm… | Rahmen für eigenes Rechenzentrum |
| Elastic Beanstalk-C… | Rahmen für Beanstalk-Umgebung |
| Öffentliches Subnetz | grüner Rahmen für öffentliches Subnetz |
| Privates Subnetz / Privates Subnetz 2 | blauer Rahmen für privates Subnetz |
| Server-Inhal… | Rahmen für Server-Inhalt |
| Generische Gruppe 1 / 2 | gestrichelter bzw. grauer Gruppenrahmen |
| Spot Fleet | Rahmen für Spot-Instanzgruppe |
| Sicherheits… | Rahmen für Sicherheitsgruppe |
| VPC | Rahmen für virtuelles Netz |
| Verfügbark… | gestrichelter Rahmen für Verfügbarkeitszone |
| Workflow für die AWS St… | Rahmen für Step-Functions-Workflow |
| EC2-Instanz… | Rahmen für Instanz-Inhalt |
| Auto Scaling Group | Rahmen für Skalierungsgruppe |

#### SH-108 AWS-Datenbank

| Shape | Beschreibung |
|---|---|
| Amazon Aurora | Dienst: relationale Cloud-Datenbank |
| Amazon Aurora-Inst… (2) | Ressource: Aurora-Instanz, neue und alte Form |
| Amazon DynamoDB | Dienst: NoSQL-Datenbank |
| Amazon Dynamo… | Ressource: DynamoDB-Beschleuniger |
| Amazon ElastiCache | Dienst: In-Memory-Cache |
| ElastiCache für Memc… / für Redis | Ressource: Cache mit Memcached bzw. Redis |
| Cache-Knot… | Ressource: Cache-Knoten |
| Von Amazon verwalteter… | Dienst: verwaltetes Cassandra |
| Amazon Neptun | Dienst: Graphdatenbank |
| Amazon RDS | Dienst: verwaltete relationale Datenbanken |
| Amazon RDS-Insta… / Amazon RDS-Instanz | Ressource: RDS-Instanz alt bzw. neu |
| Amazon RDS auf VMware | Dienst: RDS im eigenen Rechenzentrum |
| Amazon Redshift | Dienst: Data Warehouse |
| Dense Compute-K… / Dense Storage-Kn… | Ressource: Redshift-Knoten |
| Amazon Timestream | Dienst: Zeitreihendatenbank |
| AWS Datenbank-… | Dienst: Datenbank-Migration |
| Datenbank / Datenbank-… | Kategorie-Icon bzw. Ressource Datenbank |
| Attribut / Attribute | Ressource: Tabellenattribut(e) |
| Element / Elemente | Ressource: Tabelleneintrag/-einträge |
| Tabelle | Ressource: Tabelle |
| Globaler sekundär… | Ressource: globaler Sekundärindex |
| MariaDB instance / alt | Ressource: MariaDB-Instanz |
| MySQL-Inst… / alt. | Ressource: MySQL-Instanz |
| Oracle-Inst… / alt. | Ressource: Oracle-Instanz |
| PostgreSQL… / alt. | Ressource: PostgreSQL-Instanz |
| SQL Server-Insta… (2) | Ressource: SQL-Server-Instanz |
| PIOP / PIOPS | Ressource: Speicher mit garantierten IOPS |

#### SH-109 AWS-Entwicklertools

| Shape | Beschreibung |
|---|---|
| Amazon Dynamo… | Datenbank-Icon (Name abgeschnitten, Zuordnung unklar) |
| AWS Cloud Developme… | Dienst: Infrastruktur als Code (CDK) |
| AWS Cloud9 | Dienst: Entwicklungsumgebung im Browser |
| AWS CodeBuild | Dienst: Code bauen und testen |
| AWS CodeCommit | Dienst: Git-Repositorys |
| AWS CodeDeploy | Dienst: automatische Bereitstellung |
| AWS CodePipeline | Dienst: Auslieferungspipeline |
| AWS CodeStar | Dienst: Projektvorlagen für Entwicklung |
| AWS Befehlszeile… | Dienst: Kommandozeile (CLI) |
| AWS Tools und SDKs | Dienst: Werkzeuge und Bibliotheken |
| AWS X Ray | Dienst: Ablaufverfolgung von Anfragen |
| Cloud9-Res… | Ressource: Cloud9-Umgebung |
| Entwicklert… | Kategorie-Icon Entwicklerwerkzeuge |

#### SH-110 AWS-Konformität der Sicherheitsidentität

| Shape | Beschreibung |
|---|---|
| Access Analyzer | Ressource: Prüfung von Zugriffsrechten |
| AD Connector | Ressource: Anbindung an eigenes Verzeichnis |
| Add-On | Ressource: Zusatzmodul |
| Amazon Cloud … | Dienst: Cloud-Verzeichnis |
| Amazon Cognito | Dienst: Nutzeranmeldung für Apps |
| Amazon Detektiv | Dienst: Untersuchung von Sicherheitsvorfällen |
| Amazon GuardDuty | Dienst: Bedrohungserkennung |
| Amazon Inspector | Dienst: Schwachstellenprüfung |
| Amazon Inspect… | Ressource: Inspector-Agent |
| Amazon Macie | Dienst: Erkennung sensibler Daten |
| AWS Artifact | Dienst: Compliance-Berichte |
| AWS-Zertifi… | Ressource: Zertifizierungsstelle |
| AWS Certifica… | Dienst: Zertifikatsverwaltung |
| AWS CloudHSM | Dienst: Hardware-Sicherheitsmodul |
| AWS Verzeichnis… | Dienst: Verzeichnisdienst |
| AWS Managed … | Dienst: verwaltetes Active Directory |
| Simple AD | Ressource: einfaches Verzeichnis |
| AWS Firewall Manager | Dienst: zentrale Firewall-Regeln |
| AWS Schlüsselve… | Dienst: Schlüsselverwaltung (KMS) |
| AWS Resour… | Dienst: Ressourcen teilen |
| AWS Secrets Manager | Dienst: Geheimnisse verwalten |
| AWS-Sicher… | Dienst: Sicherheitsübersicht (Security Hub) |
| AWS Shield / AWS Shield erweitert | Dienst: DDoS-Schutz einfach bzw. erweitert |
| AWS Single Sign On | Dienst: Einmalanmeldung |
| AWS STS / AWS STS Alternate | Dienst: temporäre Zugangsdaten |
| AWS WAF | Dienst: Web-Firewall |
| AWS WAF-Filterr… | Ressource: Filterregel |
| Datenversc… | Ressource: Datenschlüssel |
| Verschlüsse… Daten | Ressource: verschlüsselte Daten |
| MFA-Token | Ressource: Zwei-Faktor-Token |
| Berechtigu… | Ressource: Berechtigungen |
| Rolle | Ressource: IAM-Rolle |
| Sicherheits… suchen | Ressource: Sicherheitsbefund |
| Sicherheitsi… und Konfor… | Kategorie-Icon Sicherheit und Konformität |

#### SH-111 AWS-Kostenmanagement

| Shape | Beschreibung |
|---|---|
| AWS Budgets | Dienst: Kostenbudgets |
| AWS Kosten- und Nutzu… | Dienst: Kosten- und Nutzungsbericht |
| AWS Cost Explorer | Dienst: Kostenanalyse |
| AWS Kostenman… | Kategorie-Icon Kostenmanagement |
| Berichte zu reserviert… | Ressource: Bericht zu reservierten Instanzen |
| Sparpläne | Dienst: Sparpläne |

#### SH-112 AWS-Kundenbefähigung

| Shape | Beschreibung |
|---|---|
| AWS IQ | Dienst: Vermittlung von Fachleuten |
| AWS Manag… | Dienst: Betrieb durch AWS |
| AWS Professione… | Dienst: Beratung durch AWS |
| AWS Support | Dienst: Support |
| Customer Enablement | Kategorie-Icon |

#### SH-113 AWS-Kundenbindung

| Shape | Beschreibung |
|---|---|
| Amazon Connect | Dienst: Contact-Center |
| Amazon Pinpoint | Dienst: Kampagnen und Nachrichten |
| Amazon Simpl… | Dienst: E-Mail-Versand (SES) |
| Kundenbin… | Kategorie-Icon Kundenbindung |
| E-Mail | Ressource: E-Mail |

#### SH-114 AWS-Management Governance

| Shape | Beschreibung |
|---|---|
| Amazon CloudWatch | Dienst: Überwachung und Metriken |
| AWS App-Konfig… | Dienst: Anwendungskonfiguration |
| AWS Auto Scaling | Dienst: automatische Skalierung |
| AWS CloudForm… | Dienst: Infrastruktur aus Vorlagen |
| AWS CloudTrail | Dienst: Protokoll aller API-Aufrufe |
| AWS Befehlszeile… | Dienst: Kommandozeile |
| AWS Konfigurati… | Dienst: Konfigurationsprüfung (Config) |
| AWS Control Tower | Dienst: Landezone für mehrere Konten |
| AWS License Manager | Dienst: Lizenzverwaltung |
| AWS Manag… | Dienst: Betrieb durch AWS |
| AWS Verwaltung… | Dienst: Verwaltungskonsole |
| AWS OpsWorks | Dienst: Konfigurationsmanagement |
| AWS Organisatio… | Dienst: Kontenorganisation |
| AWS Personal … | Dienst: persönliches Statusdashboard |
| AWS Service-Kat… | Dienst: Servicekatalog |
| AWS System-Ma… | Dienst: Systemverwaltung |
| AWS Trusted Advisor | Dienst: Empfehlungen zur Umgebung |
| AWS Well Architect… | Dienst: Architekturprüfung |
| Verwaltung und Gove… | Kategorie-Icon Verwaltung |
| Konto | Ressource: Konto |
| Alarm | Ressource: Alarm |
| Apps | Ressource: Anwendungen |
| Automatisi… | Ressource: Automatisierung |
| Change Set | Ressource: Änderungssatz |
| Checkliste Kosten / Fehlertolera… / Leistung / Checklisten… | Ressource: Prüfliste je Bereich |
| Prüfliste | Ressource: Prüfliste |
| Bereitstellu… | Ressource: Bereitstellung |
| Dokumente | Ressource: Dokumente |
| Ereignis (Ereig… basiert) / Ereignis (Zeit basiert) | Ressource: ereignis- bzw. zeitgesteuertes Ereignis |
| Instanzen | Ressource: Instanzen |
| Inventar | Ressource: Bestandsaufnahme |
| Ebenen | Ressource: Schichten |
| Wartungsfe… | Ressource: Wartungsfenster |
| Überwachu… | Ressource: Überwachung |
| Organisatio… | Ressource: Organisationseinheit |
| Parameter Store | Ressource: Parameterablage |
| Patch-Man… | Ressource: Patchverwaltung |
| Berechtigu… | Ressource: Berechtigungen |
| Ressourcen | Ressource: Ressourcen |
| Regel | Ressource: Regel |
| Befehl „Ausführen“ | Ressource: Befehl ausführen |
| Stapel / Stapel2 | Ressource: Vorlagen-Stack |
| Status-Man… | Ressource: Statusverwaltung |
| System-Ma… OpsCenter | Ressource: Betriebszentrale |
| Vorlage | Ressource: Vorlage |

#### SH-115 AWS-Mediendienste

| Shape | Beschreibung |
|---|---|
| Amazon Elastic Tr… | Dienst: Medien umkodieren |
| Amazon Kinesis Vide… | Dienst: Videoströme |
| AWS Element… (6) | Dienste der Elemental-Familie: Videoverarbeitung und -auslieferung |
| Media Services | Kategorie-Icon Mediendienste |

#### SH-116 AWS-Migrationsübertragung

| Shape | Beschreibung |
|---|---|
| AWS Applicati… | Dienst: Erkennung vorhandener Anwendungen |
| AWS DataSync | Dienst: Datenübertragung |
| AWS DataSy… | Ressource: DataSync-Agent |
| AWS FTP / AWS FTPS / AWS SFTP | Ressource: Dateiübertragung per FTP, FTPS, SFTP |
| AWS Transfer Family | Dienst: verwaltete Dateiübertragung |
| AWS Migrati… | Dienst: Migrationszentrale |
| AWS Server Migratio… | Dienst: Server-Migration |
| AWS Snowb… / AWS Snowball | Dienst: Transport-Datenträger mit bzw. ohne Rechenleistung |
| AWS Snowmobile | Dienst: Datentransport per Container-Lkw |
| CloudEndure Migration | Dienst: Server-Umzug |
| Migration und Übertr… | Kategorie-Icon Migration |

#### SH-117 AWS-Robotik

| Shape | Beschreibung |
|---|---|
| AWS RoboMaker | Dienst: Roboteranwendungen entwickeln |
| Cloud-Erwe… ROS | Ressource: Cloud-Erweiterung für ROS |
| Entwicklun… | Ressource: Entwicklungsumgebung |
| Flottenman… | Ressource: Flottenverwaltung |
| Robotik | Kategorie-Icon Robotik |
| Simulation | Ressource: Simulation |

#### SH-118 AWS-Satellite

| Shape | Beschreibung |
|---|---|
| AWS Ground Station | Dienst: Bodenstation für Satelliten |
| Satellit | Kategorie-Icon Satellit |

#### SH-119 AWS-Speicher

| Shape | Beschreibung |
|---|---|
| Amazon Elastic Blo… | Dienst: Blockspeicher (EBS) |
| Amazon Elastic Fil… | Dienst: Dateisystem (EFS) |
| Amazon FSx | Dienst: verwaltete Dateisysteme |
| Amazon FSx für Lustre | Dienst: Hochleistungs-Dateisystem |
| Amazon FSx für Window… | Dienst: Windows-Dateiserver |
| Amazon S3 Glacier | Dienst: Archivspeicher |
| Archiv | Ressource: Archiv |
| Tresor | Ressource: Archiv-Tresor |
| AWS Backup | Dienst: Datensicherung |
| AWS Snowb… / AWS Snowball | Dienst: Transport-Datenträger |
| Snowball Import Export | Ressource: Import/Export per Snowball |
| AWS Snowmobile | Dienst: Datentransport per Lkw |
| AWS Storage Gateway | Dienst: Brücke zwischen lokal und Cloud |
| Datei-Gate… | Ressource: Datei-Gateway |
| Volume-Ga… | Ressource: Volume-Gateway |
| Tape Gateway | Ressource: Band-Gateway |
| Virtual Tape Library | Ressource: virtuelle Bandbibliothek |
| Zwischenge… Volume | Ressource: zwischengespeichertes Volume |
| Nicht zwischenge… | Ressource: gespeichertes Volume |
| Lautstärke | Ressource: Volume (Fehlübersetzung) |
| Momentauf… | Ressource: Snapshot |
| Bucket / Bucket mit Objekten | Ressource: S3-Bucket leer bzw. gefüllt |
| Objekt | Ressource: gespeichertes Objekt |
| Standard-S… | Ressource: Standard-Speicherklasse |
| Seltener Zugriff | Ressource: Speicherklasse für seltenen Zugriff |
| EFS InfrequentA… / EFS Standard primary bg | Ressource: EFS-Speicherklassen |
| Dateisystem | Ressource: Dateisystem |
| Allgemeine Zugriffspun… | Ressource: Zugriffspunkt |
| VPC-Zugriff… | Ressource: Zugriffspunkt im VPC |
| Ressourcen… für mehrer… | Ressource: Zugriff für mehrere Regionen |
| CloudEndure Disaster Re… | Dienst: Notfallwiederherstellung |
| Speicher | Kategorie-Icon Speicher |

#### SH-120 AWS-Spieltechnologie

| Shape | Beschreibung |
|---|---|
| Amazon GameLift | Dienst: Spielserver betreiben |
| Game Tech | Kategorie-Icon Spieltechnik |

#### SH-121 AWS-Unternehmensanwendungen

| Shape | Beschreibung |
|---|---|
| Alexa für Unternehm… | Dienst: Sprachassistent im Büro |
| Amazon Chime | Dienst: Besprechungen und Chat |
| Amazon WorkMail | Dienst: E-Mail und Kalender |
| Geschäftsa… | Kategorie-Icon Geschäftsanwendungen |

**Azure** – Kennzeichnung Azure-Download (außer SH-136); die Icons werden nicht gezeichnet und nicht ausgeliefert,
sondern über den Download-Knopf (SH-G07) geholt. Daher nur die Namen.

#### SH-122 Azure App-Dienste

API-Verwalt… (API Management), App Service-Zert…, App Service-Do… (Domänen), App Service-Um… (Umgebungen),
App Service-Pläne, App Services, CDN-Profile, Notification Hubs, Suchdienste.

#### SH-123 Azure Blockchain

Azure Blockcha… (Blockchain Service).

#### SH-124 Azure Compute

App Services, Verfügbark… (Verfügbarkeitsgruppen), Cloud Services … (klassisch), Container Dienste …,
Datenträger (klassisch), Datenträger… (Verschlüsselungssätze), Datenträger, Funktionen…, Imagedefini…, Bildversionen,
Bilder, Kubernetes…, Mesh-Anw…, Betriebssyst… (klassisch), Service Fabric-Clus…, Freigegebene Bildkataloge,
Virtueller Computer, Virtuelle Comput… (klassisch), VM-Image (klassisch), VM-Skalier….

#### SH-125 Azure DevOps

Application Insights, Azure DevOps, DevTest Labs, Lab-Dienste.

#### SH-126 Azure Identity

Active Directo…, App-Registr…, Azure Active Directory, Azure AD Domai…, Unternehm… (Unternehmensanwendungen), Gruppen,
Identity Governance, Benutzer.

#### SH-127 Azure IoT

Event Grid-Abonn…, Event Hubs, Funktionen…, IoT Central-Apps, Logic Apps, Notification Hub-Name…, Stream Analytics-A….

#### SH-128 Azure KI und Maschinelles Lernen

Botdienste, Cognitive Services.

#### SH-129 Azure Management Governance

Aktivitätspr…, Advisor, Warnungen, Application Insights, Automatisi…, Azure Arc, Azure Lighthouse, Pläne, Diagnoseei…,
Bildung, MachinesA…, Managed Applicatio…, Metriken, Monitor, Network Watcher, Vorgangspr… (klassisch), Richtlinie,
Recovery Services-Tr…, Dienstanbie…, Lösungen, Benutzerdat….

#### SH-130 Azure Migrate

Datenfeldka…, Data Box, Recovery Services-Tr….

#### SH-131 Azure Security

Anwendun… (Anwendungssicherheitsgruppen), Azure Sentinel, Erweiterte Sicherheits…, Schlüsseltre…, Sicherheitsc….

#### SH-132 Azure Storage

Azure NetApp Files, Datenfeldka…, Data Box, Data Lake Storage Gen1, Datensee-S… 1. Generati…, Data Share-Einla…,
Datenfreiga…, Importieren… von Einzelv…, Recovery Services-Tr…, Speicherko… (klassisch), Speicherko…,
StorSimple Data Ma…, StorSimple Device ….

#### SH-133 Azure, allgemein

Alle Ressourcen, Biz Talk, BLOB-Block, BLOB-Seite, Branch, Browser, Käfer, Builds, Cache, Code, Commit,
Horizontale Steuerung, Steuerung, Kostenwarn…, Kostenanal…, Kostenbud…, Cost Management, Zähler, Cubes, Dashboard,
Entwicklerk…, Herunterlad…, Fehler, Erweiterung…, Datei, Dateien, Leerer Ordner, Ordner-We…, FTP, Globe Error,
Globe Success, Globe Warning, Globus, Führungslinie, Herz, Bild, Information…, Eingabe/Au…, Journey Hub,
Portal starten, Lernen, Auslastung…, Ort, Log Streaming, Verwaltung… (2), Marketplace, Mediendatei, Mobile Engagement,
Mobilgerät, Einschalten, Ein/Aus, PowerShell, Vorschau, Prozess-Exp…, Produktion… Datenbank, Schnellstart…, Aktuell,
Ressourcen… (2), Verknüpfte Ressource, Maßstab, Scheduler, Suchen, Serverfarm, Service Bus, Service Health, SSD,
Storage Azure Files, Speicherco…, Speicherwa…, Abonneme…, Tabelle, Kategorien, TFS-VC-Re…, Toolbox, Versionen,
Web-Slots, Webtest, Website Power, Website-Sta…, Arbeitsmap…, Workflow.

#### SH-134 Azure, andere

Application Gateways, Detonation, Datenträger…, Firewalls, Instanz-Pools, Internet Analyz…, IP-Gruppen, NAT,
Reservierung; dazu die Container-Rahmen Virtuelles Netzwerk, Subnetz mit NSG, Ressource, Subnetz, Container Format 1,
Container Format 2 (generisch, wie SH-136).

#### SH-135 Azure-Analysen

Analysis Services, Azure Data Explorer-Cl…, Azure Synaps…, Data Factory, Datensee-S… 1. Generati…, Event Hub-Cluster,
Event Hubs, HD Insight-Clu…, Stream Analyti….

#### SH-136 Azure-Container

Beschriftete Rahmen (Container) für Azure-Diagramme; trotz des Namens keine Container-Dienste. Kennzeichnung:
Generisch, wird selbst gebaut.

| Shape | Beschreibung |
|---|---|
| Virtuelles Netzwerk | Rahmen mit Kopfzeile für ein virtuelles Netz |
| Ressourcen… | Rahmen für eine Ressourcengruppe |
| Subnetz mit NSG | grau gefüllter Rahmen mit Kopfzeile |
| Subnetz | grau gefüllter Rahmen |
| Container Format 1 | schlichter Rahmen |
| Container Format 2 | Rahmen mit farbiger Kopfleiste |

#### SH-137 Azure-Datenbanken

Azure Cosmos DB, Azure Data Explorer-Cl…, Azure-Date… MariaDB-Se…, Azure Database … (MySQL),
Azure Database … (PostgreSQL), Azure SQL VM, Azure Synaps…, Cache Redis, Data Factory, Elastische-…, Instanzpools,
Verwaltete Datenbank, SQL Data Warehouse, SQL-Daten…, Elastische SQL-Pools, SQL-verwaltete Instanz, SQL Server,
Virtuelle Cluster.

#### SH-138 Azure-Integration

API-Verwalt…, Azure Data Catalog, Data Factory, Event Grid-Domä…, Event Grid-Abonn…, Event Grid-Themen,
Integrations…, Logic Apps, Relais, Service Bus, SQL Data Warehouse.

#### SH-139 Azure-Netzwerk

Anwendun… (Gateways), CDN-Profile, Connections, DDoS Protection-…, DNS-Zonen, ExpressRout…, Front Doors, Lastenausgl…,
NAT, Netzwerksc…, Netzwerksic…, Network Watcher, Privater Link, Näherungs…, Öffentliche IP-Adress… (2),
Öffentliche IP-Präfixe, Reservierte IP-Adress…, Routenfilter, Dienstendp…, Traffic Manager-Pr…,
Virtuelle Netzwerkga…, Virtuelle Netzwer… (klassisch), Virtuelle Netzwerke, Web Application….

**Netzwerk-Geräte und -Pläne**

#### SH-140 Computer und Monitore

Endgeräte für Netzwerkpläne. Für mehrere davon hat JFNetworker schon eigene Symbole. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| PC | Desktop-Rechner mit Bildschirm |
| Virtueller PC | virtuelle Maschine als Arbeitsplatz |
| Terminal | Terminal ohne eigene Rechenleistung |
| WDM (Wavelengt…) | Wellenlängenmultiplexer |
| Datenpipeli… | Datenleitung/-pipeline |
| Slate-Comp… | Tablet mit Stift |
| Tablet PC | Tablet-Computer |
| Laptop | Notebook |
| PDA | Taschencomputer |
| CRT-Monitor | Röhrenmonitor |
| LCD-Monitor | Flachbildschirm |

#### SH-141 Computer und Monitore - 3D

3D-Variante von SH-140. Kennzeichnung: Visio-Design. Abweichungen: Es fehlen Virtueller PC, WDM, Datenpipeline und
Slate-Computer. Zusätzlich:

| Shape | Beschreibung |
|---|---|
| iMac / Neuer iMac | All-in-One-Rechner (Produktform, nicht nachbauen; generisch „All-in-One“) |

#### SH-142 Detailliertes Netzwerkdiagramm

Spezielle Netz- und Peripheriegeräte für detaillierte Pläne. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| A/B-Umsch… | Umschalter zwischen zwei Geräten |
| Repeater | Signalverstärker |
| Diagnosege… | Messgerät für Netzwerkdiagnose |
| Smartcard-… | Chipkartenleser |
| Patchpanel | Patchfeld |
| Sendemast | Funkmast |
| Biometrisch… Lesegerät | Fingerabdruck-/Biometrieleser |
| Externe Festplatte | externe Festplatte |
| XML-Webdi… | XML-Webdienst |
| Glasfaser | Glasfaserstrecke |
| Satellitenan… | Satellitenschüssel |
| Satellit | Satellit |
| VoIP-Telefon | IP-Telefon |
| Telefonanla… | Telefonanlage (TK-Anlage) |
| MPLS | MPLS-Netz |

#### SH-143 Detailliertes Netzwerkdiagramm - 3D

3D-Variante von SH-142. Kennzeichnung: Visio-Design. Abweichungen: Es fehlen Glasfaser, VoIP-Telefon und MPLS; der
Dynamische Verbinder ist enthalten. Zusätzlich:

| Shape | Beschreibung |
|---|---|
| Daten | Datenspeicher als Scheibe |
| Externes Medienlauf… | externes Laufwerk |
| Faseroptisc… Sender | Glasfaser-Sender |

#### SH-144 Exchange-Objekte

Objekte einer Exchange-Mailserver-Umgebung; produktgebunden und veraltet. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Exchange-… | Exchange-Organisation |
| Nachrichte… (2) | Nachrichtenweiterleitung bzw. Connector |
| Exchange Server | Mailserver |
| Exchange-S… | Servergruppe |
| Öffentliche Ordner | Ordnerstruktur |
| Speichergru… | Speichergruppe |
| Empfänger… / Empfängerr… | Empfängerobjekt bzw. -richtlinie |
| Virtueller POP-Server | POP3-Zugang |
| Virtueller IMAP-Server | IMAP-Zugang |
| Virtueller NNTP-Server | News-Zugang |
| Virtuelles HTTP-Verze… | Webzugang |
| Virtueller Exchange-S… | virtueller Server |
| Sitzungen | Benutzersitzungen |
| Öffentlicher Speicher | Speicher für öffentliche Ordner |
| Postfachsp… | Postfachspeicher |

#### SH-145 Freistehende Gestelle

Geräte in Frontansicht für Regale und Tische. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Server | Tower-Server |
| Monitor | Bildschirm |
| Laptop | Notebook |
| Drucker | Drucker |
| A-B-Schaltk… | Umschaltkasten |
| Modem | Modem |
| Festplatte | externe Festplatte |
| Optisches Laufwerk | optisches Laufwerk |
| Hub | Hub |

#### SH-146 Gestellmontierte Geräte

Maßstäbliche Rack-Planung in Frontansicht; Einschübe rasten in Höheneinheiten ein. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Gestell | offenes 19-Zoll-Rack, Höhe in HE |
| Schrank | geschlossener Rack-Schrank |
| Server | Server-Einschub |
| RAID-Array | Plattensystem |
| Router 1 / Router 2 | Router-Einschub |
| Schalter | Switch-Einschub |
| Brücke | Bridge-Einschub |
| Eigenständi… Dateispeicher | NAS-Einschub |
| Bandlaufwerk | Bandlaufwerk |
| LCD-Monitor | Klappmonitor |
| Patchpanel | Patchfeld |
| Stromverso… | Stromversorgung/USV |
| Steckdosenl… | Steckdosenleiste |
| Tastaturabl… | Tastaturschublade |
| Regal | Fachboden |
| Kabelschac… | Kabelführung |
| Bemaßung - horizontal / vertikal | Maßkette waagrecht bzw. senkrecht |

#### SH-147 Gestellmontierte Server

Server-Rollen als Rack-Einschub. Dieselben 17 Rollen gibt es in SH-158 (flach) und als 3D in SH-148 und SH-159.
Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Webserver | Server für Webseiten |
| Dateiserver | Dateiablage |
| Anwendun… | Anwendungsserver |
| Zertifikatsse… | Zertifizierungsstelle |
| E-Commer… | Server für Online-Shop |
| Server für mobile Info… | Server für mobile Geräte |
| FTP-Server | Dateiübertragung |
| CMS-Server | Inhaltsverwaltung |
| Verzeichnis… | Verzeichnisdienst |
| Druckserver | Druckdienste |
| E-Mail-Server | Mailserver |
| Verwaltung… | Verwaltungsserver |
| Server für Echtzeitko… | Server für Echtzeitkommunikation |
| Öffentlicher… Zugriffsserver | Server für öffentlichen Zugang |
| Datenbanks… | Datenbankserver |
| Proxyserver | Zwischenserver |
| Streaming Media-Server | Medienstreaming |

#### SH-148 Gestellmontierte Server - 3D

3D-Variante von SH-147. Kennzeichnung: Visio-Design. Abweichungen: Es fehlen Zertifikatsserver, CMS-Server und
Öffentlicher Zugriffsserver; der Dynamische Verbinder ist enthalten. Zusätzlich:

| Shape | Beschreibung |
|---|---|
| Server | Server ohne Rolle |
| Inhaltsverw… | Inhaltsverwaltungsserver |

#### SH-149 LDAP-Objekte

Objektklassen eines LDAP-Verzeichnisses. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Land / Region | Länderobjekt |
| Organisation | Organisation |
| Organisatio… | Organisationseinheit |
| Ort | Ortsobjekt |
| Alias | Verweisobjekt |
| Person | Person |
| inetOrgPers… | Internet-Person |
| Privatperson | private Person |
| Unternehm… | Unternehmensobjekt |
| Organisat. Funktion | Rolle in der Organisation |
| Namensgru… | Gruppe von Namen |
| Gruppe eindeutig… | Gruppe eindeutiger Namen |
| Gerät | Gerät |
| CRL-Verteil… | Verteilpunkt für Sperrlisten |
| DSA | Verzeichnis-Systemagent |
| DMD | Verzeichnisverwaltungsdomäne |
| Anwendun… (2) | Anwendungsobjekt bzw. -prozess |
| Unbekannt | unbekanntes Objekt |
| Allgemeines Objekt | Objekt ohne Klasse |
| Verzeichnis… | Verzeichnisbaum |

#### SH-150 Microsoft-Produkte

Produktlogos von Microsoft; Markenzeichen, werden nicht nachgebaut und nicht ausgeliefert. Kennzeichnung:
Visio-Design (Markenlogos). Namen: Access, Excel, Exchange, Kaizala, Office, OneDrive, OneNote, Outlook,
Microsoft Planner, PowerPoint, Projekt, Publisher, SharePoint, Skype, Stream, Sway, Teams, Aufgaben, Visio,
Visual Studio, Word, Yammer.

#### SH-151 Netzwerk und Peripheriegeräte

Die wichtigsten Netz- und Bürogeräte für Netzwerkpläne. Für viele hat JFNetworker schon eigene Symbole.
Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Funkzugriff… | WLAN-Zugangspunkt |
| Ringnetzwerk | Ring-Bus; Geräte docken an Ziehpunkten an |
| Ethernet | Bus-Linie, Anschlusspunkte per Ziehen vermehrbar |
| Server | Server |
| Virtueller Server | virtueller Server |
| Mainframe | Großrechner |
| Supercomp… | Höchstleistungsrechner |
| Router | Router |
| Switch | Switch |
| Firewall | Firewall |
| Bridge | Brücke |
| Hub | Hub |
| Modem | Modem |
| DFÜ-Verbin… | Wählverbindung als Blitzlinie |
| Drucker | Drucker |
| Plotter | Plotter |
| Scanner | Scanner |
| Kopierer | Kopierer |
| Fax | Faxgerät |
| Multifunkti… | Multifunktionsgerät |
| Projektor | Beamer |
| Leinwand | Projektionsleinwand |
| Telefon | Tischtelefon |
| Mobiltelefon | Handy |
| Smartphone | Smartphone |
| Videotelefon | Bildtelefon |
| Digitalkame… | Fotokamera |
| Videokamera | Videokamera |
| Externes Medienlauf… | externes Laufwerk |
| Benutzer | Person |
| Legende | zählt die verwendeten Geräte automatisch |

#### SH-152 Netzwerk und Peripheriegeräte - 3D

3D-Variante von SH-151. Inhalt nicht im Material.

#### SH-153 Netzwerkadressen

Standortsymbole für WAN-Pläne. Der deutsche Name ist eine Fehlübersetzung („Network Locations“ = Standorte).
Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Rechenzent… | Rechenzentrum |
| Cloud | Wolke für Internet/Cloud |
| Regierungs… | Behördengebäude |
| Gebäude | Bürogebäude |
| Stadt | Stadt |
| Universitäts… | Hochschulgebäude |
| Haus | Wohnhaus |
| Großstadt | Großstadt mit Hochhäusern |

#### SH-154 Netzwerkadressen - 3D

3D-Variante von SH-153. Kennzeichnung: Visio-Design. Abweichungen: Rechenzentrum fehlt; der Dynamische Verbinder ist
enthalten.

#### SH-155 Netzwerkraumelemente

Einfache Raumteile für Netzwerk-Raumpläne. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Tür | Tür mit Aufschlag |
| Fenster | Fenster |
| Stuhl | Stuhl |
| Tabelle | Tisch (Fehlübersetzung von „Table“) |

#### SH-156 Netzwerksymbole

Klassische Netzwerksymbole für Router, Switches, Server und Sicherheit. Für viele hat JFNetworker schon eigene
Symbole. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Kommunik… | Kommunikationsserver |
| Terminalser… | Terminalserver |
| Host | Host-Rechner |
| Schlüssel | Schlüssel |
| Schloss | Schloss |
| Schloss und Schlüssel | Verschlüsselung |
| Arbeitsgrup… | Arbeitsgruppen-Switch |
| ATM-Switch | ATM-Switch |
| ATM/FastG… | ATM/Fast-Gigabit-Switch |
| ISDN-Switch | ISDN-Switch |
| ATM-Router | ATM-Router |
| Router | Router |
| Fühler | Messsonde |
| CDDI/FDDI-… | Ringkonzentrator |
| 100BaseT-H… | Fast-Ethernet-Hub |
| Kleiner Hub | kleiner Hub |
| Gateway | Gateway |
| WAN | Weitverkehrsnetz |
| DSU/CSU | Leitungsabschluss |
| Brücke | Bridge |
| OLAP-Date… | Analyse-Datenbank |
| Datenbank | Datenbank |
| Daten | Datenbestand |
| Lastenausgl… | Lastverteiler |

#### SH-157 Netzwerksymbole - 3D

3D-Variante von SH-156. Kennzeichnung: Visio-Design. Abweichungen: Es fehlen Schloss, Fühler, OLAP-Datenbank,
Datenbank, Daten und Lastenausgleich; der Dynamische Verbinder ist enthalten. Zusätzlich:

| Shape | Beschreibung |
|---|---|
| Probe | Messsonde |
| Relationale Datenbank | relationale Datenbank |
| Sperre | Vorhängeschloss |
| Server m. öff./pri… | Server mit öffentlichem und privatem Schlüssel |
| Zertifikatsse… | Zertifizierungsstelle |

#### SH-158 Server

Dieselben 17 Server-Rollen wie SH-147 (dort beschrieben), hier als flache, freistehende Symbole. Kennzeichnung:
Visio-Design.

#### SH-159 Server - 3D

3D-Variante von SH-158 als Tower. Kennzeichnung: Visio-Design. Abweichungen wie SH-148: Es fehlen Zertifikatsserver,
CMS-Server und Öffentlicher Zugriffsserver; zusätzlich „Server“ ohne Rolle und „Inhaltsverwaltungsserver“; der
Dynamische Verbinder ist enthalten.

### 4.8 Pläne und Grundrisse

**Bauplan** – die meisten Shapes sind maßstäbliche Draufsichten, deren Größe sich nach dem Zeichnungsmaßstab richtet.

#### SH-160 Alarmsteuerung und Zutrittskontrolle

Geräte der Zutrittskontrolle als einfache Strich-Plansymbole. Kennzeichnung: Visio-Design (leicht neu zu zeichnen,
keine verbindliche Norm erkennbar).

| Shape | Beschreibung |
|---|---|
| Kartenzutritt | Kartenleser an der Tür |
| Biometrisch… Zutritt | biometrischer Leser |
| Tastenblock | Zahlentastatur |
| Kartenleser mit Taste… | Kartenleser mit Tastatur |
| Kartenleser m. Stechuhr | Leser mit Zeiterfassung |
| Drehkreuz | Drehsperre |
| Drehtür | Karusselltür |
| Verkehrssch… | Schranke |
| Induk.schleif. Detektor | Induktionsschleife für Fahrzeuge |
| Elektron. Schloss | elektrischer Türöffner |
| Ausgangsv… | Ausgangstaster bzw. Türdrücker |
| Kamera mit Tastenblock | Kamera mit Tastatur |
| Kamera mit Sprechanlage | Kamera mit Gegensprechanlage |
| Kamera mit Kartenleser | Kamera mit Kartenleser |
| Sicherheitsf… | Sicherheitsfeld/-fenster |

#### SH-161 Arbeitsbereiche

Büro-Arbeitsplätze, Trennwände und Ablagen in Draufsicht. Kennzeichnung: Visio-Design (schlicht, fast generisch).

| Shape | Beschreibung |
|---|---|
| Würfelförm… Arbeitsplatz | Arbeitsplatz mit Trennwänden ringsum |
| L-förmiger Arbeitspla… (2) | Arbeitsplatz mit L-Tisch |
| Gerader Arbeitsplatz | Arbeitsplatz mit geradem Tisch |
| Kreisförmiger Arbeitsplatz | runder Arbeitsplatz |
| U-förmiger Arbeitsplatz | Arbeitsplatz mit U-Tisch |
| Abteil | Arbeitsabteil |
| Trennwand | gerade Stellwand |
| Trennwand, gebogen | gebogene Stellwand |
| Eckfläche | Eckverbindungsplatte |
| Ecke abgerundet | runde Eckplatte |
| Arbeitsinsel | Insel aus Tischen |
| Sockel | Rollcontainer |
| Archivschra… | Aktenschrank |
| Offenes Regal | offenes Regal |
| Breite Hängeablage | Hängeregister |

#### SH-162 Arbeitsbereiche – Visio 2013

Ältere Fassung von SH-161. Inhalt nicht im Material.

#### SH-163 Auslöser und Melder

Brandmelde- und Alarmgeräte für Gebäudepläne. Inhalt nicht im Material.

#### SH-164 Außen

Unfall- und Tatortskizzen im Freien: Fahrzeuge, Straßen, Bewuchs, Maßstab. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Referenzpu… | Bezugspunkt für Maße |
| Polizeiwagen | Streifenwagen von der Seite |
| Polizeiwagen (von oben) | Streifenwagen in Draufsicht |
| Mittelgroßer Pkw (von …) | Pkw in Draufsicht |
| Mittelgroßer Zweitürer… | Zweitürer |
| Kleiner Pickup | Pick-up |
| Kleintransp… / Kleintransp… (von oben) | Kleintransporter seitlich bzw. von oben |
| Geländewa… / Geländewa… (von oben) | Geländewagen seitlich bzw. von oben |
| Titelblock | Schriftfeld der Skizze |
| Hecke | Hecke |
| Kaktus | Kaktus |
| Strauch | Strauch |
| Topfpflanze | Topfpflanze |
| Holzzaun | Holzzaun als Linie |
| Maschendr… | Maschendrahtzaun |
| Straßenlater… | Straßenlaterne |
| Straßen | Straßenstück mit Markierung |
| Schnittpunkt | Einmündung |
| Trennzeich… | Fahrbahnmarkierung |
| Trennlinie | gestrichelte Leitlinie |
| Kompass | Nordrichtung |
| Textblock | Textfeld |
| Beschriftung | Hinweis mit Zeigerlinie |
| Bemaßungs… | Maßlinie |
| Skalasymbol | Maßstabsleiste |
| Maßstabbe… | Maßstabsangabe |

#### SH-165 Bad- und Küchenplan

Sanitär- und Kücheneinrichtung. Inhalt nicht im Material.

#### SH-166 Bad- und Küchenplan – Visio 2013

Ältere Fassung von SH-165. Inhalt nicht im Material.

#### SH-167 Beweis

Tatortskizzen: Beweismittel, Spuren und Körperumrisse; außerdem Kamera, Kompass und Referenzpunkt wie SH-164.
Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Generischer Beweis | Beweismarke mit Strichcode |
| Gewehr | Langwaffe |
| Schrotflinte / Abgesägte Schrotflinte | Schrotflinte, normal bzw. gekürzt |
| Pistole | Pistole |
| Revolver | Revolver |
| Pistolenclip | Magazin |
| Patronenhü… | Patronenhülse |
| Einschussst… | Einschussloch |
| Austrittsstelle einer Patrone | Austrittsloch |
| Schraubenz… | Schraubendreher |
| Klauenham… | Klauenhammer |
| Brechstange | Brecheisen |
| Baseballsch… | Schläger |
| Eispickel | Eispickel |
| Numchuks | Würgehölzer |
| Axt | Axt |
| Gliederkette | Kette |
| Großes / Langes / Kurzes Messer | Messer in drei Größen |
| Hackbeil | Hackbeil |
| Frauenspuren / Frau links / Frau rechts | Fußspuren einer Frau |
| Männerspur / Mann links / Mann rechts | Fußspuren eines Mannes |
| Blutlache (groß) / (klein) | Blutlache |
| Blutspur | Blutspur |
| Körperumriss / Körperumriss (seitlich) | Umriss einer liegenden Person |
| Arm / Bein / Oberkörper / Kopf | Körperteile für Umrisse |
| Kamera | Kamerastandort |

#### SH-168 Bewässerung

Gartenbewässerung mit Regnern und Leitungen. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Kreisregner variabel | Kreisregner, Wurfweite per Ziehpunkt |
| Viereckregner variabel | Viereckregner, Fläche per Ziehpunkt |
| Seitenleitung | dünne Leitung |
| Hauptleitung | dicke Leitung |
| Perlschlauch | Tropfschlauch |
| Ventil | Ventil |
| Ventilkenns… | Ventilkasten mit Kennzeichnung |
| Controller | Bewässerungssteuerung |
| Rückstausic… | Rückflussverhinderer |
| Wasserhahn | Zapfstelle |
| Wasserzähler | Wasserzähler |

#### SH-169 Büroausstattung

Bürogeräte in Draufsicht. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| PC | Rechner auf dem Tisch |
| Telefon | Telefon |
| Drucker | Drucker |
| Faxgerät | Faxgerät |
| Tastatur | Tastatur |
| Scanner | Scanner |
| Plotter | Plotter |
| Aktenvernic… | Aktenvernichter |
| Kopierer | Kopierer |
| Terminal | Terminal |
| Notebook-… | Notebook |
| Hub | Netzwerkverteiler |
| LSX | Gerät (Kürzel, Bedeutung unklar) |
| Telefonans… | Telefondose |
| Schalter | Lichtschalter |
| Steckdose | Steckdose |
| Monitorarme | Monitorhalterung |
| Telefonanla… | Telefonanlage |
| Maus | Maus |
| Monitor | Bildschirm |
| Projektionsl… | Leinwand |
| Projektor | Beamer |
| Tablet | Tablet |
| Tower-PC | Tower-Rechner |
| Overhead-P… | Overheadprojektor |

#### SH-170 Büroausstattung – Visio 2013

Ältere Fassung von SH-169. Inhalt nicht im Material.

#### SH-171 Büromöbel

Büromöbel in Draufsicht; Konferenztische mit einstellbarer Stuhlzahl. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Tisch | Schreibtisch |
| Bürostuhl | Drehstuhl |
| Schublade | Schubladenelement |
| Archivschra… | Aktenschrank |
| Bootförmiger Tisch | Tisch in Bootsform |
| Ovaler Tisch | ovaler Tisch |
| Privater Bürostuhl | Chefstuhl |
| Runder Tisch | runder Tisch |
| Runder Konferenzti… | runder Tisch mit Stühlen |
| Rechteckiger Konferenzti… | rechteckiger Tisch mit Stühlen |
| Ovaler Konferenzti… | ovaler Tisch mit Stühlen |
| Bootförm. Tisch mi… | Bootstisch mit Stühlen |
| Trapezförm… Konferenzti… | Trapeztisch mit Stühlen |
| Hocker | Hocker |
| Bequemer Tisch | Beistelltisch |
| Privater Büroschreib… | Chefschreibtisch |
| Sideboard | Sideboard |
| Wohnzimm… | Sessel |
| Ecktisch | Ecktisch |
| Eckfläche | Eckverbindung |
| Ecke abgerundet | runde Eckplatte |
| Trapezförm… Tisch | Trapeztisch |
| Modulare trapezförmi… | Trapez-Modultische |
| Modulare Rundtische | Rund-Modultische |
| Empfangstr… | Empfangstheke |
| Chefsessel | Chefsessel |
| Sofaverbind… nach innen | Sofa-Eckstück |
| Gästestuhl | Besucherstuhl |
| Sofa | Sofa |

#### SH-172 Büromöbel – Visio 2013

Ältere Fassung von SH-171. Inhalt nicht im Material.

#### SH-173 Bürozubehör

Ausstattung, Pflanzen und Personen in Draufsicht für Büropläne. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Palme / Große Palme | Zimmerpalme |
| Tafel | Wandtafel |
| Schreibtisc… | Schreibtischlampe |
| Tischlampe | Tischlampe |
| Tischtennis… | Tischtennisplatte |
| Poolbillardt… | Billardtisch |
| Kickertisch | Tischfußball |
| Quadratisc… Abfalleimer / Runder Abfalleimer | Papierkorb |
| Kleiderhaken | Garderobe |
| Pinbrett | Pinnwand |
| Verkaufsaut… | Verkaufsautomat |
| Kaffeemasc… | Kaffeemaschine |
| Gehende / Stehende / Arbeitende / Wartende Frau | Frau von oben in verschiedenen Haltungen |
| Sich entspannen… (Frau) | sitzende Frau |
| Gehender / Stehender / Arbeitender / Wartender Mann | Mann von oben in verschiedenen Haltungen |
| Sich entspannen… (Mann) | sitzender Mann |
| Blatt / Großes Blatt | Blattpflanze |
| Blüte / Große Blüte | Blühpflanze |
| Kaktuspflan… | Kaktus |

#### SH-174 Bürozubehör – Visio 2013

Ältere Fassung von SH-173. Inhalt nicht im Material.

#### SH-175 Elektrik und Telekommunikation

Elektro- und Telekommunikationsinstallation im Grundriss. Kennzeichnung: Norm: Installationsplan-Symbole
(IEC 60617-11; Visio zeichnet im ANSI-Stil, für DIN siehe SH-E03).

| Shape | Beschreibung |
|---|---|
| Deckenleuc… | Deckenleuchte |
| Eingelassene Deckenleuc… | Einbauleuchte |
| Wandleuchte | Wandleuchte |
| Lichtleiste | Lichtband |
| Spotleuchten | Strahler |
| Außenleuc… | Außenleuchte |
| Leuchtstoff… | Leuchtstofflampe |
| Anbauleuc… | Aufbauleuchte |
| Büroleuchte | Büroleuchte |
| Notbeleuch… | Notleuchte |
| Notsignal | Notsignal |
| Schalter | Schalter |
| Schnurscha… | Zugschalter |
| Dimmer | Dimmer |
| Wartungssc… | Wartungsschalter |
| Steckdose | Steckdose |
| Telefonans… | Telefonanschluss |
| Stereoansc… | Audioanschluss |
| TV-Anschluss | Antennenanschluss |
| Thermostat | Raumthermostat |
| Deckenvent… | Deckenventilator |
| Türöffner | Türöffner |
| Melder | Rauchmelder |
| Feueralarm | Brandmeldetaster |
| Hauptsteue… | Verteiler |
| Masse | Erdung |
| Kabelverbin… | Leitung |

#### SH-176 Fahrzeuge

Fahrzeuge in Draufsicht und Schleppkurven für Verkehrs- und Parkplanung. Kennzeichnung: Visio-Design
(Wendekreise generisch).

| Shape | Beschreibung |
|---|---|
| Kompakter / Mittelgroßer / Großer Pkw | Pkw in drei Größen |
| Limousine | Limousine |
| Kleiner / Großer Lieferwagen | Lieferwagen |
| Kompakter / Großer Pickup | Pick-up |
| Kl. / Gr. Sattelschlep… | Sattelzug |
| Große / Mittelgroße / Kleine Zugmaschine | Zugmaschine |
| LKW | Lastwagen |
| Sattelauflie… | Auflieger |
| Schienenbus / Schienenbus mit Gelenk | Bus bzw. Gelenkbus |
| Schulbus | Schulbus |
| Straßenbahn | Straßenbahn |
| Löschfahrze… | Feuerwehrfahrzeug |
| Krankenwa… | Rettungswagen |
| Fahrrad | Fahrrad |
| Motorrad | Motorrad |
| Wendekreis LKW / Bus / sehr groß / groß / klein | Schleppkurve für Fahrzeugklassen |

#### SH-177 Fahrzeuge – Visio 2013

Ältere Fassung von SH-176. Inhalt nicht im Material.

#### SH-178 Gartenzubehör

Zäune, Mauern, Beläge und Regner für Gartenpläne. Kennzeichnung: Visio-Design (Zäune und Mauern als Linien generisch).

| Shape | Beschreibung |
|---|---|
| Holzzaun | Holzzaun als Linie |
| Maschendr… (Zaun) | Maschendrahtzaun |
| Holzzauntor | Tor im Holzzaun |
| Maschendr… (Tor) | Tor im Maschendrahtzaun |
| Holzpfosten / Betonpfosten / Metallpfost… | Zaunpfosten |
| Mauer / Steinmauer | Mauer |
| Betonplatten | Plattenbelag |
| Ziegelpflast… 1 / 2 / 3 / 4 / 5 / 6 | Ziegelpflaster in Verlegemustern |
| Kopfsteinpf… 1 / 2 / 3 / 4 | Kopfsteinpflaster |
| Natursteinpf… | Natursteinbelag |
| Runde / Quadr. / Sechseckige Platten | Trittplatten |
| Einfahrt | Einfahrtsfläche |
| Terrasse | Terrassenfläche |
| Rasen | Rasenfläche |
| Gartensprin… 1 / 2 / 3 | Rasensprenger |

#### SH-179 Gartenzubehör – Visio 2013

Ältere Fassung von SH-178. Inhalt nicht im Material.

#### SH-180 Gebäudekern

Treppen, Aufzüge, Schächte und Sanitärräume. Kennzeichnung: gemischt – Norm: DIN 1356 (Treppen, Rampe, Aufzug,
Schacht), Visio-Design (Sanitärobjekte, Kabinen). Treppen: Stufenzahl über Shape-Daten.

| Shape | Beschreibung |
|---|---|
| Geradläufige Treppe | gerade Treppe mit Lauflinie |
| Treppenabs… | Podest |
| Ziertreppe | geschwungene Treppe |
| Doppelgan… | zweiläufige Treppe |
| Wendeltrep… | Wendeltreppe |
| Eckabsatz | Eckpodest |
| Treppenric… | Laufrichtungspfeil |
| Rolltreppe | Fahrtreppe |
| Aufzug | Aufzugsschacht mit Kreuz |
| Lastenaufzug | Lastenaufzug |
| Runder Panoramaa… | runder Glasaufzug |
| Rampe | Rampe |
| Laderampe | Laderampe |
| Handlauf / Eckhandlauf | Handlauf gerade bzw. um die Ecke |
| Schutzgelä… | Geländer |
| Schacht | Installationsschacht |
| Toilette | WC |
| Urinale | Urinal |
| Spülbecken | Spülbecken |
| Unterbausp… | Waschtisch mit Unterschrank |
| Kabinen | WC-Kabinen |
| Waschraum… 1 / 2 | Waschraum-Grundriss |
| Badewanne | Badewanne |
| Duschraum | Dusche |

#### SH-181 Gebäudekern – Visio 2013

Ältere Fassung von SH-180. Inhalt nicht im Material.

#### SH-182 Haushaltsgeräte

Küchen- und Unterhaltungsgeräte in Draufsicht. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Kühlschrank | Kühlschrank |
| Einbaukühl… | Einbaukühlschrank |
| Gefriertruhe | Gefriertruhe |
| Herdzeile | Herd mit Kochfeld |
| Einbauofen | Backofen |
| Mikrowelle | Mikrowelle |
| Geschirrspü… | Geschirrspüler |
| Waschmasc… und Trockner | Waschmaschine und Trockner |
| Klimagerät | Klimagerät |
| Warmwasse… | Warmwasserspeicher |
| Müllkompr… | Müllpresse |
| Kaffeemasc… | Kaffeemaschine |
| Wasserspen… | Wasserspender |
| Lebensmitt… | Snackautomat |
| Verkaufsaut… | Getränkeautomat |
| Fernseher | Fernseher |
| Videospiel | Spielkonsole |
| Medienwie… | Mediaplayer |
| Turmlautsp… | Standlautsprecher |
| Woofer | Basslautsprecher |
| Lautsprecher | Lautsprecher |

#### SH-183 Haushaltsgeräte – Visio 2013

Ältere Fassung von SH-182. Inhalt nicht im Material.

#### SH-184 HKL-Elemente

Lüftungs- und Klimageräte als Plansymbole. Kennzeichnung: Norm: HLK-Symbole (ASHRAE-/DIN-artig).

| Shape | Beschreibung |
|---|---|
| Drehkolben… | Drehkolbenpumpe |
| Pumpe | Pumpe |
| Hubkolben… | Hubkolbenpumpe |
| Schraubens… | Schraubenverdichter |
| Zentrifugal… (2) | Radialventilator |
| Gebläseflügel | Lüfterrad |
| Axialgebläse | Axialventilator |
| Propellerge… | Propellerventilator |
| Entfeuchter | Luftentfeuchter |
| Kondensator 1 / 2 | Verflüssiger |
| Luftfilter | Luftfilter |
| Trockner | Lufttrockner |
| Dämpfer | Schalldämpfer |
| Rohrschlange | Heiz- oder Kühlregister |
| Kühlmittelb… | Kältemittelbehälter |
| Kühler | Kühler |
| Zugangstür | Revisionsöffnung |

#### SH-185 HKL-Rohre

Lüftungskanäle in maßstäblicher Draufsicht; Breite und Länge per Ziehpunkt, Teile docken aneinander an.
Kennzeichnung: Norm: Lüftungskanal-Darstellung.

| Shape | Beschreibung |
|---|---|
| Gerade Rohrleitung | gerader Kanal |
| Rohrleitung… | Kanalende |
| Variable Krümmung | Bogen mit einstellbarem Winkel |
| Gehrungskr… | Gehrungsbogen |
| T-Verbindu… | T-Stück |
| Y-Verbindu… | Hosenstück |
| 3-Weg-Ver… | Dreiwegestück |
| Verbindung | Kanalverbindung |
| Abgeschräg… Verbindung | schräger Abzweig |
| Übergang | Querschnittsübergang |
| Versetzter Übergang | versetzter Übergang |
| Flexible Verbindung 1 / 2 | Segeltuchstutzen |
| Flexible Rohrleitung | Flexschlauch |
| Klappe mit Rohrleitung | Kanal mit Klappe |
| Klappe | Regelklappe |
| Vertikale Rohrleitung | senkrechter Kanal |
| VAV-Box | Volumenstromregler |

#### SH-186 HKL-Steuerungen

Fühler und Steuerglieder der Gebäudeautomation (Sechseck mit Messgröße). Kennzeichnung: Norm: GA-/HLK-Fühlersymbole.

| Shape | Beschreibung |
|---|---|
| Temperatur | Temperaturfühler |
| Feuchtigkeit | Feuchtefühler |
| Druck | Druckfühler |
| Fluss | Strömungsfühler |
| Spannung | Spannungsmessung |
| Strom | Strommessung |
| Geschwindi… | Geschwindigkeitsfühler |
| Enthalpie | Enthalpiefühler |
| Ein/Aus | Schaltzustand |
| Allgemein | Fühler allgemein |
| Ebene | Füllstand (Fehlübersetzung von „Level“) |
| Rauch | Rauchfühler |
| Zeitgeber | Zeitschaltung |
| Licht | Lichtfühler |
| Drehung | Drehzahlfühler |
| Vibration | Schwingungsfühler |
| Luftqualität | Luftgütefühler |
| Endschalter | Endschalter |
| Stromansch… | Stromanschluss |
| Stellglied | Stellantrieb |
| Ein-/Ausga… | Ein-/Ausgabemodul |
| Verbindung… | Wirkverbindung |
| Verkabelun… | Verkabelung |
| Beschriftung | Beschriftung |

#### SH-187 HKL-Steuerungszubehör

Bauteile für HLK-Regelschemata. Kennzeichnung: Norm: HLK-Regelschema-Symbole.

| Shape | Beschreibung |
|---|---|
| Rohrleitung / Rohrleitung… | Rohrleitung |
| Rücklaufka… | Rückluftkanal |
| Zulaufkanal | Zuluftkanal |
| Mehrfachg… | Mischkammer |
| VAV-Box / DD-VAV-Box | Volumenstromregler einfach bzw. Zweikanal |
| Gebläsegeh… | Ventilatorgehäuse |
| Thermoblock | Heizregister |
| Zentrifugal… | Radialventilator |
| Propellerge… | Propellerventilator |
| Schaufelaxi… | Axialventilator mit Leitschaufeln |
| Klappe | Klappe |
| Filter | Filter |
| Luftstromst… | Volumenstrommessung |
| Befeuchter | Befeuchter |
| Heiz-/KühlS… | Heiz-/Kühlregister |
| Ventil | Ventil |
| Wasserfluss… | Strömungswächter |
| Pumpe | Pumpe |
| Kühler | Kühler |
| Kühlturm | Kühlturm |
| Konverter | Umformer |
| Wärmetaus… | Wärmetauscher |
| Boiler | Kessel |
| Geräte | Gerät allgemein |
| Anlasser | Motorstarter |
| Regelantrieb | Stellantrieb |
| Seite/Unters.-, Seite/Seite-, Oben/Unten-Rohrverbind. | Rohranschluss in drei Richtungen |
| Flussrichtu… f. Rohre | Fließrichtungspfeil |

#### SH-188 Innen

Sammlung für Wohnungs- und Innenraumpläne: Räume, Möbel, Sanitär. Außerdem wie SH-164: Titelblock, Referenzpunkt,
Kompass, Textblock, Beschriftung, Bemaßung, Skalasymbol, Maßstabbalken. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Wand / Wand.127 | Wandstück |
| Tür / Doppeltür | Tür mit Aufschlag, einflügelig bzw. zweiflügelig |
| Fenster | Fenster |
| L-förmiger Raum / T-förmiger Raum | Raum in L- bzw. T-Form |
| Lampe / Lampe (seitlich) | Stehlampe von oben bzw. seitlich |
| Einzelbett / Breites Bett / Französisches Bett / Doppelbett | Betten verschiedener Breite |
| Etagenbett | Etagenbett |
| Nachttisch | Nachttisch |
| Kommode / Kommode 2 Schubl. / Kommode 3 Schubl. | Kommoden |
| Bücherschr… | Bücherregal |
| Sofa / Zweiersofa | Sofa |
| Ruhesessel | Sessel |
| Liegestuhl | Liege |
| Hocker | Hocker |
| Stuhl | Stuhl |
| Rechteck. Esstisch / Ovaler Esstisch / Kreisförmiger Tisch | Esstische mit Stühlen |
| Schreibtisch | Schreibtisch |
| Großes Klavier / Spinettklavier | Flügel bzw. Klavier |
| Tischtennis… | Tischtennisplatte |
| Palme / Blatt | Zimmerpflanzen |
| Fernsehgerät / Fernseher | Fernseher |
| Telefon | Telefon |
| Spüle 1 / 2 / 3 | Küchenspüle mit ein bis drei Becken |
| Dusche | Duschwanne |
| Badewanne 1 / 2 / Ovale Badewanne | Badewannen |
| Toilette 1 / 2 | WC |
| Bidet | Bidet |
| Bodensteh. Spüle / 2 | Standwaschbecken |
| Unterbausp… | Waschtisch mit Unterschrank |

#### SH-189 Interessante Punkte

Karten-Pins mit Piktogrammen für Gebäude-, Messe- und Lagepläne. Kennzeichnung: gemischt – Visio-Design
(Pin-Kacheln); Motive teils Norm: ISO 7001 (öffentliche Information) bzw. ISO 7010 (Sicherheit), die normgerecht
gezeichnet werden.

| Shape | Beschreibung |
|---|---|
| Ihr Standort | Standortmarke „Sie sind hier“ |
| Toiletten / Toiletten, Herren / Toiletten, Damen | WC allgemein, Herren, Damen |
| Rauchverbot / Raucher | Rauchen verboten bzw. erlaubt |
| Beenden | Ausgang (Fehlübersetzung von „Exit“) |
| Fluchtweg | Rettungsweg |
| Feueralarm… | Brandmelder |
| Feuerlöscher | Feuerlöscher |
| Erste Hilfe | Erste Hilfe |
| Sprinklerab… | Sprinkler-Absperrung |
| Wasserabsp… | Wasser-Absperrung |
| Elektrohaup… | Hauptschalter Strom |
| Feststehend… Generator | Notstromaggregat |
| Keine Einfahrt / Kein Ausgang | Durchgang verboten |
| Barrierefrei… | rollstuhlgerecht |
| Treppen / Aufzug | Treppe bzw. Aufzug |
| Luftverkehr | Flughafen |
| Gepäck / Gepäckwag… | Gepäck, Gepäckwagen |
| Bus / U-Bahn / Taxi / Schienenve… | öffentlicher Verkehr |
| Hubschrau… | Hubschrauberlandeplatz |
| Wasserverk… | Fähre/Schiff |
| Tunnel | Tunnel |
| Autoverleih / Mietwagen | Autovermietung |
| Parken / Parken verboten | Parkplatz bzw. Parkverbot |
| Ampeln | Ampel |
| Kartenverka… | Fahrkarten/Tickets |
| Information / Hotelinfor… | Auskunft |
| Hotel / Etagenkellner | Hotel, Zimmerservice |
| Restaurant / Kaffee / Cocktail / Bier / Wein / Fruchtsaft / Balken | Gastronomie („Balken“ = Bar, Fehlübersetzung) |
| Livemusik / Kasino | Unterhaltung |
| Geldautomat / Geldaustau… | Geldautomat, Geldwechsel |
| Zoll / Einwanderu… | Zoll, Einreise |
| Gebet | Andachtsraum |
| Reparaturen / Werkstätten | Reparatur, Werkstatt |
| Salon | Friseur |
| Telefon / WLAN | Telefon, WLAN |
| Papierkorb | Papierkorb |
| Trinkwasser | Trinkwasser |
| Säuglingspf… | Wickelraum |
| Turnhalle / Yoga / Pool | Sport |
| Tischfußball / Tischtennis / Videospiele | Freizeit |
| Konferenzra… / Telekonfere… | Besprechung, Videokonferenz |
| Kopierer / Drucker / Projektor | Bürotechnik |
| Bibliothek | Bibliothek |
| Post | Post |

#### SH-190 Interessante Punkte – Visio 2013

Ältere Fassung von SH-189. Inhalt nicht im Material.

#### SH-191 Lagerhaus - Versand und Warenannahme

Tore, Rampen und Krane für Lager- und Logistikpläne. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Ladebucht | Laderampenplatz |
| Ladedeck | Ladebrücke |
| Hebebühne f. Ladedeck | Hubtisch |
| Container-Rampe | Rampe für Container |
| Schwingtür 1 / 2 | Pendeltür |
| Rolladen | Rolltor |
| Isoliertür | Kühlraumtür |
| Tor | Tor |
| Verdichtun… | Torabdichtung |
| Radschutz | Radabweiser |
| Sicherheits… (2) | Sicherheitseinrichtung |
| Kippmulde | Container-Mulde |
| Gasbehälter | Gastank |
| Öltank | Öltank |
| Versandcon… | Versandcontainer |
| Container-Kran | Containerkran |
| Hafenkran | Hafenkran |

#### SH-192 Möbel

Wohnmöbel in maßstäblicher Draufsicht. Wie in SH-188 beschrieben: Sofa, Stuhl, Rechteckiger und Ovaler Esstisch,
Etagenbett, Einzelbett, Breites Bett, Französisches Bett, Doppelbett, Kommode, Zweiersofa, Hocker, Ruhesessel,
Kreisförmiger Tisch, Palme, Blatt, Tischtennisplatte, Großes Klavier, Spinettklavier, Bücherschrank, Nachttisch,
Kommode 2/3 Schubladen, Fernsehgerät. Kennzeichnung: Visio-Design. Neu in dieser Schablone:

| Shape | Beschreibung |
|---|---|
| Wohnzimm… | Wohnzimmerschrank/Sessel |
| Blüte | Blühpflanze |
| Kaktuspfla… | Kaktus |
| Tisch | Tisch |
| Rechteck. Tisch | rechteckiger Tisch |
| Minibar | Minibar |
| Ellipsentrai… | Crosstrainer |
| Laufband | Laufband |
| Ergometer | Heimtrainer |

#### SH-193 Möbel – Visio 2013

Ältere Fassung von SH-192. Inhalt nicht im Material.

#### SH-194 Parkplätze und Straßen

Parkplatzmarkierungen, Bordsteine und Straßenstücke in Draufsicht; Stellplatzanzahl per Ziehen. Kennzeichnung:
Generisch.

| Shape | Beschreibung |
|---|---|
| Parkstreifen 1 / 2 | Reihe von Stellplätzen gerade bzw. schräg |
| Strahlenförm. Streifen 1 / 2 | Stellplätze im Bogen |
| Stellplatz 1 / 2 | einzelner Stellplatz |
| Parkbucht 1 / 2 | Parkbucht am Rand |
| Mittelinsel 1 / 2 | Grünstreifen zwischen Reihen |
| Abschlussin… 1 / 2 | Endinsel einer Reihe |
| Parkinsel | Insel |
| Eckbordstein 1 / 2 / 3 | Bordstein um eine Ecke |
| Bordstein mit Rampe | abgesenkter Bordstein |
| Gehwegra… | Gehwegrampe |
| Schnittpunkt | Einmündung |
| Gerade Straße.76 | gerades Straßenstück |
| Gekrümmte Straße | Kurve |
| Straßenkreu… 1 / 2 | Einmündung bzw. Kreuzung |
| Zebrastreifen | Fußgängerüberweg |
| Fahrbahntei… | Fahrbahnteiler |

#### SH-195 Parkplätze und Straßen – Visio 2013

Ältere Fassung von SH-194. Inhalt nicht im Material.

#### SH-196 Pflanzen

Bäume, Sträucher und Beete in Draufsicht für Gartenpläne. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Nadelbaum A / B / C | Nadelbaum in drei Formen |
| Laubbaum A / B / C | Laubbaum in drei Formen |
| Nadelstrauch A / B / C | Nadelstrauch |
| Laub. Strauch A / B / C | Laubstrauch |
| Breitbl. immer… | immergrüner Laubbaum |
| Palme | Palme |
| Strauch, immergrün | immergrüner Strauch |
| Breitblätt. Hecke | Laubhecke |
| Nadelhecke | Nadelhecke |
| Bodenbede… | Bodendecker |
| Staudenrab… | Staudenbeet |
| Sukkulente | Sukkulente |
| Kaktus | Kaktus |
| Ziergras | Ziergras |
| Topfpflanze 1 / 2 | Kübelpflanze |
| Pflanzenbes… | Pflanzenbeschriftung mit Zeiger |

#### SH-197 Pflanzen – Visio 2013

Ältere Fassung von SH-196. Inhalt nicht im Material.

#### SH-198 Produktionsstätte - Lagerhaltung und Verteilung

Flurförderzeuge, Förderer, Krane und Regale in Draufsicht. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Elektrischer Gabelstapler | Elektrostapler |
| Gabelst. mit Dieselmotor | Dieselstapler |
| Gabelst. mit Hubkabine | Kommissionierstapler |
| Manueller Palletenwa… | Handhubwagen |
| Pallettenwa… mit Motor | Elektrohubwagen |
| Palettengab… | Palettengabel |
| Sammelhu… | Sammelhubwagen |
| Standardpal… | Palette |
| Förderband | Bandförderer |
| Rollenförde… / Rollenfb. mit Biegung | Rollenbahn gerade bzw. Kurve |
| Brückenkran | Brückenkran |
| Obenverstr. / Untenverstr. Auslegerkran | Schwenkkran |
| Wandausle… | Wandschwenkkran |
| Bodenkran | Werkstattkran |
| Portalkran | Portalkran |
| Fahrbares Regal | Rollregal |
| Freistehend. Regal | Standregal |
| Standardre… | Regal |
| Standardge… | Palettenregal |
| Gestellabsc… | Regalfeld |
| Herausziehb. Gestell | Auszugsregal |
| Schräggestell | Durchlaufregal |
| Drive-in-Gestell | Einfahrregal |
| Zwischenra… | Regalgang |
| Lagertrom… | Kabeltrommel |

#### SH-199 Produktionsstätte - Maschinen und Einrichtungen

Werkzeugmaschinen und Werkstatteinrichtung in Draufsicht. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| CNC-Dreh… | CNC-Drehmaschine |
| Spitzendreh… | Spitzendrehmaschine |
| Horizontale / Vertikale Bandsäge | Bandsäge |
| Sägemaschi… | Kreissäge |
| Revolv.kopf-Fräsmasch. | Revolverkopf-Fräsmaschine |
| Vertik. / Horiz. Fräsmaschine | Fräsmaschine |
| Oberfläche… | Flachschleifmaschine |
| Kurzhobel… | Shaping-Maschine |
| Falzmaschine | Abkantmaschine |
| Scherma… | Tafelschere |
| Handwalze… | Walzmaschine |
| Säulenbohr… | Säulenbohrmaschine |
| Bohrmaschi… | Bohrmaschine |
| WIG-Schwe… / MIG-Schwe… | Schweißgerät |
| Generator | Stromerzeuger |
| Kompressor | Druckluftkompressor |
| Bühnenwag… | Montagewagen |
| Werkbank | Werkbank |
| Fahrb. Werkzeugsc… | Werkzeugwagen |
| Standardsc… | Schrank |
| Feuerlöscher | Feuerlöscher |
| Bedienpers… | Bediener von oben |

#### SH-200 Ressourcen

Platzhalter für Flächen-, Inventar- und Umzugsplanung mit Berichten. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Raum | schraffierte Raumfläche mit Daten |
| Grenze | gestrichelte Bereichsgrenze |
| Posten | Inventaretikett |
| Person | Person mit Zuordnung |
| Computer | Rechner als Inventar |
| Drucker | Drucker als Inventar |
| Raumbericht | Tabelle aller Räume |
| Inventarberi… | Tabelle aller Inventarposten |
| Verschiebu… | Umzugsliste |

#### SH-201 Rohre und Ventile - Rohre 1

Rohrleitungen, Leitungsarten und Rohrhalterungen. Kennzeichnung: Norm: ISO 6412, ISO 10628.

| Shape | Beschreibung |
|---|---|
| Kreuzungen | Kreuzung ohne Verbindung |
| Verbindung… | Kreuzung mit Verbindungspunkt |
| Kreuz | Kreuzstück |
| Verzweigung | Abzweig |
| Kniestück 45° / 90° | Rohrbogen |
| Beheizt/Gekühlt | beheizte oder gekühlte Leitung |
| Signalleitung | Signalleitung |
| Elektrische Leitung | elektrische Leitung |
| Pneum. Leitung | Druckluftleitung |
| Hydraulische Leitung | Hydraulikleitung |
| Kapillarleitu… | Kapillarleitung |
| Strahlungsl… | Strahlungsleitung |
| Interne Verbindung | Systemverbindung |
| Mechanische Verbindung | mechanische Verbindung |
| Doppelwan… | doppelwandiges Rohr |
| Isoliert | gedämmte Leitung |
| Überschiebr… | Mantelrohr |
| Flussrichtung | Fließrichtungspfeil |
| Änderung im Rohrdurch… | Nennweitenwechsel |
| Reduzierstü… | Reduzierung |
| Elektr. Einrichtung | Begleitheizung |
| Vibrationsg… | Schwingungsdämpfung |
| Gewichtsei… | Gewichtsbelastung |
| Sprühvorric… | Sprühdüse |
| Rotationsbe… | Drehbewegung |
| Rühreinr./G… | Rührwerk |
| Fundament | Fundament |
| Führungssc… 1 / 2 | Rohrführung |
| Anschlag | Anschlag |
| Anker | Festpunkt |
| Stütze/Anker | Stütze mit Festpunkt |
| Hängelager | Rohraufhängung |
| Zugriffspun… | Revisionsöffnung |
| Klappen | Klappe |
| Flexibilitäts… | Kompensator |
| Ausgleichro… | Ausgleichsrohr |
| Muffenerw… | Muffenerweiterung |
| Dehnungsa… | Dehnungsausgleicher |
| Flexibler Schlauch | Schlauch |
| Drosselkörper | Drosselscheibe |

#### SH-202 Rohre und Ventile - Rohre 2

Rohrverbindungen und Rohrzubehör. Kennzeichnung: Norm: DIN 2429, ISO 6412. Wie in SH-091 beschrieben: Allgemeine
Verbindung, Stumpfgeschweißt, Gelötet, Schraubverbindung, Schweißmuffe, Geflanscht/geschraubt, Drehgelenk,
Elektrisch verbunden, Elektrisch isoliert, Rohrverschluss 1/2, Berstscheibe, Flammensperre, Filter, Y-Filter,
Abscheider, Abflussdämpfer, Schalldämpfer, Trichter, Flüss.-Dicht. offen/geschl., Offene Entlüftung, Siphon, Hydrant.
Neu in dieser Schablone:

| Shape | Beschreibung |
|---|---|
| Überschieb… | Überschiebmuffe |
| Muffe | Muffe |
| Rohrverschl… 3 | Rohrkappe |
| Glockenför… Ausweitung | Glockenmuffe |
| Auslassöffn… | Auslass |

#### SH-203 Rohre und Ventile - Ventile 1

Armaturen für Rohrleitungspläne. Kennzeichnung: Norm: ISO 10628, DIN 2429. Wie in SH-091 beschrieben:
Überdruckventil, Eckventile, Rückschlagventil, Schwimmerventil, Flanschventil, Drosselventil, Kugelventile,
Nadelventile, Membranventil, Keilschieber/Zylinder, Schieberventil, Kugelhähne, Gesteuerte Ventile,
Druck (Eckventil), Reduzierventil, Kegelventil, 3-Wege-Kegelventil, Mischventil, Wegeventil. Neu:

| Shape | Beschreibung |
|---|---|
| Reihen-Ventil | Durchgangsventil allgemein |
| Schraubventil | Ventil mit Schraubanschluss |

#### SH-204 Rohre und Ventile - Ventile 2

Ventilkörper mit verschiedenen Antriebsarten. Kennzeichnung: Norm: ISO 10628, ISA 5.1.

| Shape | Beschreibung |
|---|---|
| Reihen-Ventil | Durchgangsventil allgemein |
| Manuelle Isolierung | handbetätigte Absperrung |
| Elektrisch | Ventil mit Elektroantrieb |
| Statisch geladen | Ventil mit Gewicht/Last |
| Federbetrie… | Ventil mit Federantrieb |
| Fernsteueru… | ferngesteuertes Ventil |
| Membran | Ventil mit Membranantrieb |
| Ketten gesteuert | Ventil mit Kettenrad |
| Getriebeges… | Ventil mit Getriebe |
| Elektromag… | Magnetventil |
| Gewichtste… | gewichtsbelastetes Ventil |
| Schwimmer… | Ventil mit Schwimmer |
| Puffer-/Kolbenventil | Ventil mit Kolbenantrieb |
| Schnell öffnend/sc… | Schnellschlussventil |
| Verbindung… | Ventil mit Verbindungsantrieb |
| Motorelem… | Ventil mit Motorantrieb |
| Regulieren | Regelventil |

#### SH-205 Sanitär

Heizungs- und Sanitärobjekte im Grundriss und in Ansicht. Kennzeichnung: Norm: Sanitär-Plansymbole (DIN 1986,
DIN 1356).

| Shape | Beschreibung |
|---|---|
| Boiler | Warmwasserspeicher |
| Heiz-/Kühls… | Heiz- oder Kühlschlange |
| Pumpe | Umwälzpumpe |
| Radiator | Heizkörper |
| Konvektor | Konvektor |
| Strahlungsh… (ob.) / (vo.) | Strahlungsheizung von oben bzw. vorn |
| Heizer/Kühler | Heiz-/Kühlgerät |
| Tank offen/gesc… | Behälter offen bzw. geschlossen |
| Wasserober… | Wasserspiegel |
| Rohrschlan… | Rohrschlange |
| Spülbecken | Spüle |
| Waschbeck… / Waschbeck… (stl.) | Waschbecken von oben bzw. seitlich |
| Toilette 1 / 2 / Toilette (seitl.) | WC von oben bzw. seitlich |
| Bad / Bad (seitl.) | Badewanne von oben bzw. seitlich |
| Endansicht | Wanne in Stirnansicht |
| Duschkopf | Brause |
| Handtuchh… | Handtuchhalter |

#### SH-206 Schieber, Gitter und Diffusoren

Luftdurchlässe für Lüftungspläne. Kennzeichnung: Norm: HLK-Plansymbole.

| Shape | Beschreibung |
|---|---|
| Zulaufdiffu… | Zuluftdurchlass |
| Rücklaufdif… | Abluftdurchlass |
| Gitterdiffusor | Gitter-Luftdurchlass |
| Linearer Diffusor | Schlitzdurchlass |
| Bandförmig… Diffusor | Bandluftdurchlass |
| Gitter seitl. | Wandgitter |

#### SH-207 Schränke

Küchen- und Büroschränke als einfache Draufsichten. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Unterschrank 1 / 2 | Unterschrank |
| Seitenregal für US | Endregal am Unterschrank |
| Eckuntersc… / Eckuntersc… Blende | Eckunterschrank mit bzw. ohne Blende |
| Unterschra… schräg | schräger Unterschrank |
| Seitenregal für Insel | Endregal an Kochinsel |
| Inselunters… 1 / 2 | Inselunterschrank |
| L-förmiger Unterschrank | L-Unterschrank |
| Seitenregal für WS | Endregal am Wandschrank |
| Wandschrank 1 / 2 | Oberschrank |
| Eckwand | Eck-Oberschrank |
| WS Eckblende 1 / 2 | Eckblende Oberschrank |
| WS Ecke schräg | schräger Eck-Oberschrank |
| Wandschr.-… 1 | Oberschrank-Variante |
| Inselwands… 2 | Oberschrank über Insel |
| Insel-WS, Blende | Blende über Insel |
| Schienenab… / Schienenecke / 45°-Schiene | Abschlussleisten |
| Tablettschi… | Tablettfach |
| Schließfach | Spind |

#### SH-208 Schränke – Visio 2013

Ältere Fassung von SH-207. Inhalt nicht im Material.

#### SH-209 Sport- und Freizeitanlagen

Spielfelder und Becken in maßstäblicher Draufsicht. Kennzeichnung: Visio-Design (Feldmarkierungen folgen den
Sportregeln).

| Shape | Beschreibung |
|---|---|
| Pool rechteckig / Pool oval / Pool nierenförmig | Schwimmbecken |
| Warmwasse… | Whirlpool |
| Trainingsbe… / Wettkampf… | Trainings- bzw. Wettkampfbecken |
| Sprungbrett 1 / 2 | Sprungbrett |
| Fußballplatz | Fußballfeld |
| Tennisplatz | Tennisplatz |
| Basketballk… | Basketballkorb |
| Basketball-Freiwurfraum | Freiwurfzone |
| Basketball 3-Pkte | Dreipunktelinie |
| Basketballfe… | Basketballfeld |
| Badmintonf… | Badmintonfeld |
| Volleyballfeld | Volleyballfeld |
| American Football-Feld | Footballfeld |
| Baseballplatz | Baseballfeld |
| Cricketfeld | Cricketfeld |
| Hockeyfeld | Hockeyfeld |
| Stadion | Stadion |
| Hallenstadi… | Halle |
| Schaukeln | Schaukel |
| Spielgeräte | Spielplatzgerät |

#### SH-210 Sport- und Freizeitanlagen – Visio 2013

Ältere Fassung von SH-209. Inhalt nicht im Material.

#### SH-211 Videoüberwachung

Kameras, Melder und Videotechnik als Strich-Plansymbole. Kennzeichnung: Visio-Design (keine verbindliche Norm
erkennbar).

| Shape | Beschreibung |
|---|---|
| Kamera | feste Kamera |
| Schwenkka… | Schwenk-/Neigekamera |
| Bewegungs… | Bewegungsmelder |
| Videobewe… | Videobewegungsmelder |
| Glasbruchs… | Glasbruchmelder |
| Bistatischer Strahlensens. | Lichtschranke |
| Sicherheitsf… m. Alarm | überwachtes Fenster |
| Druckschalter | Trittmatte/Druckschalter |
| Schalter - manuell / Schalter - automatisch | Schalter |
| Sichtungsg… | Sichtgerät |
| Monitor | Monitor |
| Video-Multi… | Videomultiplexer |
| Aufzeichnu… | Rekorder |
| Video-Tasta… | Bedientastatur |
| Kabelverbin… | Leitung |

#### SH-212 Wände, Rohbau und Strukturen

Wände, Türen, Fenster und Tragwerk für Grundrisse; Türen und Fenster setzen sich in Wände, Wände verbinden sich.
Kennzeichnung: Norm: DIN 1356, ISO 7519.

| Shape | Beschreibung |
|---|---|
| Raum / "L"-Raum / "T"-Raum | Raum aus vier bzw. mehr Wänden |
| Raum / "L"-Raum / "T"-Raum (schraffiert) | Raumfläche mit Flächenberechnung |
| Wand | Wandstück, Dicke einstellbar |
| Außenwand | dicke Außenwand |
| Gekrümmte Wand | gebogene Wand |
| Tür | Drehflügeltür |
| Doppelt | zweiflügelige Tür |
| Ungleich | Tür mit ungleichen Flügeln |
| Gegenüberl… | gegenläufige Doppeltür |
| Doppelkipp… | Pendeltür |
| Drehtür | Karusselltür |
| Einschubtür / Doppeleins… | Schiebetür in die Wand |
| Schiebetür | Schiebetür vor der Wand |
| Falttür / Doppelfalttür | Falttür |
| Glas-Schieb… | Glasschiebetür |
| Kipptor | Garagentor |
| Fenster | Fenster |
| Schiebefens… | Schiebefenster |
| Fensterflügel | Flügelfenster |
| Gartenfenster | Erkerfenster |
| Vorhangwa… | Vorhangfassade |
| Fensterwand | Fensterband |
| Öffnung | Wandöffnung |
| Platte | Bodenplatte |
| Wandpfeiler / Eckpfeiler | Pfeiler in bzw. an der Wand |
| Trägerbalken | Unterzug |
| Rechteckige Säule / Runde Säule | Stütze |
| Steuerung: Bemaßung | Maßlinie |
| Raumbema… | Raummaße |
| Türenplan / Fensterplan | Tabelle aller Türen bzw. Fenster |
| Gitterursprung… | Ursprung des Achsrasters |
| Gitterlinie | Rasterachse |

#### SH-213 Wände, Türen und Fenster

Die Grundteile für Grundrisse; dieselben Shapes wie in SH-212 (Raum, "L"-Raum, "T"-Raum, Wand, Gekrümmte Wand, Tür,
Doppeltür, Fenster, Öffnung, Pfeiler, Eckpfeiler, Steuerung: Bemaßung, Raumabmessungen). Kennzeichnung: Norm:
DIN 1356, ISO 7519. Neu:

| Shape | Beschreibung |
|---|---|
| Fläche | schraffierte Fläche mit Flächenangabe |
| Beschriftung | Hinweis mit Zeigerlinie |

#### SH-214 Öffentliche Plätze

Stadtmobiliar und Außenanlagen in Draufsicht. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Lampenpfo… / Straßenlater… | Mastleuchte |
| Beleuchtung 1 / 2 | Leuchte |
| Licht | Bodenleuchte |
| Müllcontai… | Müllcontainer |
| Papierkorb | Abfallkorb |
| Feuerhydrant | Hydrant |
| Zahlautomat | Parkscheinautomat |
| Wächterhä… | Pförtnerhaus |
| Schranke | Schranke |
| Poller | Poller |
| Fahrradgest… | Fahrradständer |
| Einsteigesc… | Schachtdeckel |
| Abfluss | Gully |
| Barrierefrei… | Rollstuhlsymbol |
| Bank / Bank für den Außenbereich | Sitzbank |
| Gartenstuhl / Liegestuhl | Gartenmöbel |
| Regenschirm | Sonnenschirm |
| Tisch / Picknick-Tisch | Tisch |
| Felsbrocken | Findling |
| Vogeltränke | Vogeltränke |
| Grill | Grill |
| Bodenschw… | Bodenschwelle |
| Eingangstor | Tor |
| Verkehrspyl… | Leitkegel |
| Steg | Steg |
| Gewässer 1 / 2 / 3 / 4 | Teich, Bach in verschiedenen Formen |
| Brücke | Brücke |

#### SH-215 Öffentliche Plätze – Visio 2013

Ältere Fassung von SH-214. Inhalt nicht im Material.

**Karte**

#### SH-216 Freizeitanlagen-Shapes

Schwarze Piktogramm-Kacheln für Freizeit- und Wanderkarten. Kennzeichnung: Visio-Design (für die Neuzeichnung bietet
ISO 7001 eine normgerechte Vorlage).

| Shape | Beschreibung |
|---|---|
| Toilette | WC |
| Parkplatz | Parkplatz |
| Trinkwasser | Trinkwasser |
| Campingpl… | Zeltplatz |
| Wohnmobil | Wohnmobilstellplatz |
| Lagerfeuer | Feuerstelle |
| Dusche | Dusche |
| Zubehör/G… | Laden/Ausrüstung |
| Amphitheat… | Freilichtbühne |
| Wandern / Bergsteigen | Wandern bzw. Klettern |
| Reiten | Reiten |
| Golf / Tennis / Volleyball / Racquetball… | Ballsport |
| Angeln | Angeln |
| Schwimmen / Tauchen mit Tauchgerät | Wassersport |
| Kanuzugang / Kajaksport / Bootssteg / Marina | Bootssport und Anleger |
| Abfahrtsski / Skilanglauf / Snowboard… / Schneemobil | Wintersport |
| Roll-/Schlitt… | Roll- bzw. Schlittschuhlaufen |

#### SH-217 Shapes für markante Gebäude o.ä.

Bunte 3D-Icons für Wegbeschreibungen und Übersichtskarten. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Flughafen / Bahnhof | Verkehrsknoten |
| Bus / Taxi / Fähre / Fahrrad | Verkehrsmittel |
| Gebäude 1 / 2 | Bürogebäude |
| Wolkenkrat… | Hochhaus |
| Wohnblöcke | Wohnanlage |
| Vorstadthaus / Stadthaus | Wohnhaus |
| Großstadt | Stadtsilhouette |
| Rathaus | Rathaus |
| Schule | Schule |
| Hotel | Hotel |
| Laden / Einkaufszen… außerhalb | Geschäft bzw. Einkaufszentrum |
| Fabrik / Lagerhaus | Gewerbe |
| Scheune | Scheune |
| Tankstelle | Tankstelle |
| Feuerwehr | Feuerwache |
| Stadion | Stadion |
| Park | Park |
| Nadelbaum / Laubbaum | Baum |
| See / Fluss / Ozean | Gewässer |
| Nord | Nordpfeil |
| Maßstab | Maßstabsleiste |

#### SH-218 Stadtbahn-Shapes

Bausteine für schematische Liniennetzpläne. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Stadtbahnli… | gerades Linienstück, Länge ziehbar |
| Abgerundete Stadtbahnli… | Linienstück mit runden Enden |
| Flexible Stadtbahn | biegbare Linie mit Kurvenpunkten |
| Station | Haltestelle als Kreis auf der Linie |
| Umsteigest… | Umsteigehaltestelle aus zwei Kreisen |
| Haltestellen… | Linienende mit Namensfeldern |
| Stadtbahnk… 1 | rechtwinklige Kurve |
| Stadtbahnk… 2 | weite Bogenkurve |

#### SH-219 Straßen-Shapes

Straßen in Draufsicht mit Mittellinie. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Straße (eckig) / Straße (abgerundet) | gerades Straßenstück |
| Flexible Straße | biegbare Straße |
| Kurve 1 / 2 | Kurve |
| Ecke | Ecke |
| 4-Weg-Kreuzung / 3-Weg-Kre… | Kreuzung bzw. Einmündung |
| Kreisverkehr | Kreisverkehr |
| Kleeblattkre… | Kleeblatt-Autobahnkreuz |
| Rautenförm… Autobahnkr… | Rautenanschluss |
| Straßenunt… | Unterführung |
| Brücke | Brücke |
| Schmale Straße | Straße als Linie mit Namen |

#### SH-220 Verkehrs-Shapes

Verkehrszeichen, Gleise und sich selbst verbindende Straßen für Lagepläne. Kennzeichnung: gemischt – Norm:
Verkehrszeichen (Wiener Übereinkommen, StVO-ähnlich) für Schilder, Generisch (Straßen, Schienen).

| Shape | Beschreibung |
|---|---|
| Ampel | Verkehrsampel |
| Anhalten | Halt-Schild |
| Vorfahrt achten | Vorfahrt-gewähren-Schild |
| Keine Einfahrt | Einfahrt verboten |
| Parken verboten | Parkverbot |
| Verbotsschild | Verbotsschild allgemein |
| Durchfahrt verboten | Durchfahrtsverbot |
| Warnschild | Gefahrzeichen |
| Parkplatz | Parkplatz |
| Autobahn | Autobahnende-Zeichen |
| Autobahndi… | Raststätte |
| Europäische Union | Länderschild EU |
| Straßennu… 1 / 2 | Straßennummer |
| Autobahna… | Ausfahrtnummer |
| Informatio… | Hinweisschild |
| Richtungsschi… | Wegweiser |
| Pfeil | Richtungspfeil |
| Einbahnstra… | Einbahnstraße |
| Schienen / Schienenku… | Gleis gerade bzw. Kurve |
| Intellig. Straße 1 / 2 / 3 | Straße, die sich an Kreuzungen anschließt |
| Intellig. Straße (Ent…) | Straßenende |
| Intellig. Kreuzung 1 / 2 | Kreuzung für diese Straßen |
| Straße m. Ausrichtung | Straße mit Richtung |
| Punkt für Stadt | Ortspunkt mit Namen |

#### SH-221 Wegbeschreibung 3D-Shapes

Isometrische Bausteine für Anfahrtsskizzen. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Straße 1 / 2 / 3 / 4 | Straßenstück |
| Gerade Straße 1 / 2 | gerade Straße |
| Hochstraße 1 / 2 | Hochstraße |
| Drei Straßen | Einmündung |
| Kreuzungen | Kreuzung |
| Ecke 1 / 2 | Kurve |
| Überwege | Kreuzung mit Zebrastreifen |
| Brücke | Brücke |
| Fluss | Fluss |
| Auto 1 / 2 / 3 / 4 / 5 | Auto |
| Zug 1 / 2 | Zug |
| Baum | Baum |
| Fußgänger… | Fußgängerüberweg |
| Einbahnstra… | Einbahnstraßenpfeil |
| Block 1 / 2 / 3 / 4 / 5 / 6 / 7 | Gebäudeblock |
| Fabrik | Fabrik |
| Tankstelle 1 / 2 | Tankstelle |
| Geschäft 1 / 2 | Geschäft |
| Dach 1 | Dach |
| Flughafen | Flughafen |

### 4.9 Software und Datenbank

**Datenbank**

#### SH-222 Chen-Datenbanknotation

Entity-Relationship-Modelle in Chen-Notation. Kennzeichnung: Norm: Chen-ER-Notation.

| Shape | Beschreibung |
|---|---|
| Entität | Rechteck |
| Schwache Entität | Rechteck mit Doppelrand |
| Assoziative Entität | Raute im Rechteck |
| Attribut | Ellipse |
| Primärschlü… | Ellipse mit unterstrichenem Namen |
| Mehrwertiges Attribut | Ellipse mit Doppelrand |
| Abgeleitetes Attribut | gestrichelte Ellipse |
| Beziehung | Raute |
| Beziehung wird iden… | identifizierende Beziehung, Raute mit Doppelrand |
| Beziehungs… | Verbindungslinie mit Kardinalität |

#### SH-223 Crow's Foot-Datenbanknotation

Datenmodelle mit Krähenfuß-Kardinalitäten. Kennzeichnung: Norm: Krähenfuß-Notation (Information Engineering).

| Shape | Beschreibung |
|---|---|
| Entität | Tabellenkasten mit Kopf |
| Entität mit Attributen | Tabellenkasten mit Attributzeilen |
| Attribut | Attributzeile zum Einfügen |
| Primärschlü… | Schlüsselzeile |
| Primärschlü… (Trennlinie) | Trennlinie unter den Schlüsseln |
| Beziehung | Verbinder mit Krähenfuß-Enden, Kardinalität wählbar |

#### SH-224 Entitätsbeziehung

Datenmodelle mit Tabellen, Sichten und Generalisierung (Visio-Notation). Kennzeichnung: Norm: ER-Notation
(Visio-Variante, IDEF1X-artig).

| Shape | Beschreibung |
|---|---|
| Entität | Tabelle |
| Beziehung | Fremdschlüssel-Verbinder |
| Ansicht | Datenbanksicht |
| Kategorie | Generalisierungssymbol (Kreis mit Strichen) |
| Übergeordn… zu Kategorie | Linie vom Obertyp zur Kategorie |
| Kategorie zu Untergeord… | Linie von der Kategorie zum Untertyp |

#### SH-225 IDEF1X-Datenbanknotation

Datenmodelle nach IDEF1X. Kennzeichnung: Norm: IDEF1X (FIPS 184).

| Shape | Beschreibung |
|---|---|
| Entität | Entitätskasten mit Schlüsselbereich |
| Primärschlü… (2) | Schlüsselzeile bzw. Trennlinie |
| Attribut | Attributzeile |
| Beziehung | Beziehungslinie mit Punktende |

#### SH-226 Objekt - relational

Wie SH-224, ergänzt um Typen für objektrelationale Datenbanken. Kennzeichnung: Norm: ER-Notation (Visio-Variante).
Entität, Beziehung, Ansicht, Kategorie und die Kategorie-Linien wie SH-224. Neu:

| Shape | Beschreibung |
|---|---|
| Tabellenver… | Tabellenvererbung |
| Typ | benutzerdefinierter Datentyp |
| Typenverer… | Typvererbung |

#### SH-227 UML-Datenbanknotation

Datenmodelle als UML-Klassen. Kennzeichnung: Norm: UML 2.x.

| Shape | Beschreibung |
|---|---|
| Entität | Klassenkasten als Tabelle |
| Primärschlü… (2) | Schlüsselzeile bzw. Trennlinie |
| Attribut | Attributzeile |
| Beziehung | Assoziationslinie |

**Drahtmodell** (Wireframes für Web und Mobil, graue Platzhalter-Optik)

#### SH-228 Allgemeine Symbole (Drahtmodell)

Allgemeine UI-Icons für Wireframes. Kennzeichnung: Visio-Design (Motive allgemein üblich, im eigenen Stil neu zu
zeichnen).

| Shape | Beschreibung |
|---|---|
| Zurück / Weiterleiten | Pfeil links bzw. rechts |
| Erweitern / Reduzieren | Doppelpfeil auf bzw. zu |
| Hinzufügen / Entfernen | Plus bzw. Kreuz |
| Vergrößern / Verkleinern | Lupe mit Plus bzw. Minus |
| Suchen / Finden | Lupe bzw. Fernglas |
| Sperre / Berechtigung / Sicherheit / Datenschutz | Schloss, Schild |
| Sortieren / Filter | Sortieren A–Z, Trichter |
| Tools / Konfigurieren / Assistent | Werkzeug, Zahnrad, Zauberstab |
| Eigenschaft… / Kennzeichen / Tag | Etikett, Fahne, Lesezeichen |
| Kalender / Datum / Uhrzeit / Ereignis | Kalender, Uhr |
| Dokument / Skript / Ordner / Anlage | Blatt, Skriptrolle, Ordner, Büroklammer |
| Datenbank / Festplatte / Cloud | Speicher |
| Benutzer / Benutzer hinzufügen / Kontakt | Person, Visitenkarte |
| Netzwerk / Internet / Link / WLAN | Vernetzung |
| Favorit / Gefällt mir / Teilen | Stern, Daumen, Teilen-Symbol |
| Einkaufswa… 1 / Kreditkarte / Quittung | Einkauf und Bezahlung |
| RSS / E-Mail / Gesendet / Antworten / Antwort | Nachrichten |
| Kommentar / Chats / Ungelesene Nachricht / Feedback | Sprechblasen |
| Benachricht… / Emoticon | Glocke, Smiley |
| Verlauf / Aktualisieren | Uhr mit Pfeil, Kreispfeil |
| Start | Haus |
| Stopp | Hand |
| Ort / Karte / Drehbezugs… | Ortsmarke, Karte, Stecknadel |
| Speichern / Drucken / Bearbeiten / Format | Diskette, Drucker, Stift, Pinsel |
| Kopieren / Ausschneid… / Einfügen / Löschen | Zwischenablage, Papierkorb |
| Rückgängig / Wiederholen | Pfeil zurück bzw. vor |
| Hochladen / Herunterlad… | Pfeil nach oben bzw. unten |
| Information… / Häufig gestellte Fr… | Info-Kreis, Fragezeichen |
| Mehr | drei Punkte |

#### SH-229 Cursor und Gesten

Mauszeiger und Touch-Gesten für Wireframes. Kennzeichnung: Visio-Design (Motive allgemein üblich).

| Shape | Beschreibung |
|---|---|
| Auswählen | Pfeilzeiger |
| Verschieben | Vierfachpfeil |
| Ausgelastet | Sanduhr |
| Linkauswahl | Hand mit Zeigefinger |
| Vertikal / Horizontal / Diagonal skalieren 1 / 2 | Doppelpfeile zum Größe ändern |
| Textcursor | Textbalken |
| Genauigkeit | Fadenkreuz |
| Hintergrund | Pfeil mit Sanduhr |
| Nicht verfügbar | Verbotskreis |
| Hilfe | Pfeil mit Fragezeichen |
| Tippen / Doppeltippen | Finger tippt ein- bzw. zweimal |
| Drücken und halten | langer Druck |
| Vertikales / Horizontales Scrollen | Wischen zum Blättern |
| Halten und ziehen | Ziehen mit dem Finger |
| Nach links / rechts wischen | seitliches Wischen |
| Nach unten / oben streichen | senkrechtes Streichen |
| Drehen | Drehgeste |
| Zwei-Finger… | Geste mit zwei Fingern |
| Überfüllen | Spreizgeste (Fehlübersetzung) |

#### SH-230 Mobile Karten und Container

Rahmen, Listen und Karten für mobile Wireframes. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Container | leerer Bereich |
| Rahmen 320*480 / 320*533 / 360*640 | Gerätebildschirm in fester Größe |
| Listeneleme… | Listenzeile |
| Listeneleme… mit Symbol | Listenzeile mit Icon |
| Listeneleme… mit Schaltfl… | Listenzeile mit Knopf |
| E-Mail-Liste… | E-Mail-Listenzeile |
| Kontaktliste… | Kontaktzeile |
| Gitter | Kachelraster |
| Textkachel | Kachel mit Text |
| Medienkac… | Kachel mit Bild |
| Bild- und Textkarte 1 / 2 | Karte mit Bild und Text |
| Video- und Textkarte 1 / 2 | Karte mit Video und Text |
| Profilkarte 1 / 2 | Profilkarte |
| Kontaktinfo… | Kontaktdaten |
| Bezahlungs… | Bezahlformular |
| Anmeldeka… | Anmeldeformular |
| Soziale Medien-Karte | Beitrag aus sozialem Netzwerk |

#### SH-231 Mobile Medien

Platzhalter für Bilder, Videos und Karten. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Symbol | runder Icon-Platzhalter |
| Bild | Bildplatzhalter |
| Kreisförmiges Bild | runder Bildplatzhalter |
| Karussell | Bildkarussell |
| Banner | Werbebanner |
| Audioplayer | Audiowiedergabe |
| Videoplayer | Videowiedergabe |
| Karte | Kartenausschnitt |
| Webcamfe… | Kamerabild |
| Diagramme | Diagrammplatzhalter |
| Video | Videoplatzhalter |
| Hintergrund | Hintergrundfläche |
| Mobiltelefo… | Telefonrahmen |

#### SH-232 Mobile Navigation

Navigationselemente für mobile Wireframes. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Paginierung / Paginierung1 | Seitenpunkte |
| Breadcrumb | Pfadnavigation |
| Trennlinie (horizont…) | Trennlinie |
| Scrollleiste | Bildlaufleiste |
| Wartekreisel | Ladeanimation |
| Registerkart… | Tableiste |
| Akkordeon 1 / 2 | aufklappbare Abschnitte |
| Horizontale Navigations… | waagrechte Menüleiste |
| Assistent | Schrittfolge |
| Navigations… oben / unten | Navigationsleiste oben bzw. unten |
| Statusleiste | Statusleiste des Telefons |
| Abschnittst… | Abschnittstrenner |

#### SH-233 Mobile Steuerelemente

Bedienelemente für mobile Wireframes. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Schaltfläche | Knopf |
| Farbauswahl | Farbwähler |
| Kombinatio… | Kombinationsfeld |
| Bereichssch… 1 / 2 | Bereichsregler mit zwei Griffen |
| Schieberegler | Schieberegler |
| Captcha | Captcha-Feld |
| Datumsaus… | Datumswähler |
| Zeitauswahl | Zeitwähler |
| Kontrollkäst… (2) | Kontrollkästchen einzeln bzw. als Gruppe |
| Optionsfeld / Optionsfeld… | Optionsfeld einzeln bzw. als Gruppe |
| Umschalten | Kippschalter |
| Suchleiste | Suchfeld |
| Zahleneing… | Zahlenfeld mit Plus/Minus |
| Statusanzeige | Fortschrittsbalken |
| Hamburger… (2) | Menü-Knopf mit drei Strichen |
| Bildschirmt… | Bildschirmtastatur |

#### SH-234 Mobiler Text und Anmerkungen

Text- und Formularelemente für mobile Wireframes. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Trennzeichen | Trennlinie |
| Absatztext | Textabsatz als Platzhalter |
| Kategorienli… | Liste von Kategorien |
| Textfeld | Eingabefeld |
| Formularei… | Formularfeld mit Beschriftung |
| Aufzählung | Aufzählungsliste |
| Textlink | Link |
| Text-Editor | Textbereich mit Werkzeugleiste |
| Notiz | Notizzettel |
| QuickInfo | Tooltip |
| Schriftartau… | Schriftauswahl |
| Bewertung | Sternebewertung |
| Kontextmenü (rechts klic…) / Kontextmenü | Kontextmenü |
| Titel / Untertitel / Text | Überschriften und Fließtext |
| Logo | Logo-Platzhalter |
| Kennwort | Kennwortfeld |

#### SH-235 Web- und Mediensymbole (Drahtmodell)

Medien- und Bedienicons. Kennzeichnung: Visio-Design (Motive allgemein üblich).

| Shape | Beschreibung |
|---|---|
| Ein/Aus | Netzschalter |
| Wiedergabe / Pause / Stopp | Wiedergabesteuerung |
| Zurückspulen / Schnellvorl… / Zurück / Weiter | Spulen und Titelwechsel |
| Zufällige Wiedergabe | Zufallswiedergabe |
| Lautstärke / Ton aus | Lautstärke, stumm |
| Video / Musik / Foto / Album | Medienarten |
| Zuschneiden / Nach links drehen / Nach rechts drehen | Bildbearbeitung |
| CD | CD |
| Anruf | Telefonhörer |
| Kamera / Webcam | Kamera |
| Mikrofon / Kopfhörer / Lautsprecher | Audiogeräte |
| Vollbild | Vollbild-Ecken |

#### SH-236 Web-Navigation

Navigationselemente für Web-Wireframes. Kennzeichnung: Generisch. Wie SH-232: Paginierung, Paginierung1, Breadcrumb,
Trennlinie, Scrollleiste, Wartekreisel, Registerkarte, Horizontale Navigation, Assistent. Neu:

| Shape | Beschreibung |
|---|---|
| Navigations… mit zwei Re… | Menüleiste mit zwei Reihen |
| Leporello | Akkordeon |
| Vertikale Navigations… | senkrechtes Menü |

#### SH-237 Webcontainer

Fenster und Inhaltsblöcke für Web-Wireframes. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Container | leerer Bereich |
| Fenster | Fenster mit Titelleiste |
| Dialogfeld | Dialog |
| Warnfeld | Warnmeldung |
| Fußzeile | Seitenfuß |
| Internetbro… | Browserfenster |
| Beitrag in sozialem N… | Beitrag eines sozialen Netzwerks |
| Videoplayer… | Videofenster |
| Videoclipco… 1 / 2 | Video mit Text |
| Anmeldung… | Anmeldeformular |
| Produktbes… | Produktbeschreibung |
| Kontaktdet… | Kontaktdaten |
| Bild- und Textcontainer | Bild mit Text |
| Bezahlungs… | Bezahlformular |

#### SH-238 Webmedien

Gleiche Shapes wie SH-231 ohne „Mobiltelefon“, für Web-Wireframes. Kennzeichnung: Generisch.

#### SH-239 Websteuerelemente

Bedienelemente für Web-Wireframes. Kennzeichnung: Generisch. Wie SH-233: Schaltfläche, Farbauswahl, Kombinationsfeld,
Captcha, Schieberegler, Datumsauswahl, Zeitauswahl, Optionsfeld (einzeln und Gruppe), Kontrollkästchen (2), Umschalten,
Zahleneingabe, Suchleiste, Statusanzeige, Hamburger (2). Neu:

| Shape | Beschreibung |
|---|---|
| Prozentsatz… | Regler mit Prozentwert |
| Zahlen-Ber… | Bereichsregler mit Zahlen |
| Strukturliste | Baumliste |

#### SH-240 Webtext und -anmerkungen

Gleiche Shapes wie SH-234 ohne „Kennwort“ und das zweite „Kontextmenü“, für Web-Wireframes. Kennzeichnung: Generisch.

**Software**

#### SH-241 Allgemeine Symbole (Software)

22 UI-Icons, alle mit gleichem Motiv in SH-228 enthalten: Zurück, Weiterleiten, Erweitern, Reduzieren, Hinzufügen,
Entfernen, Vergrößern, Verkleinern, Sperren, Berechtigung, Sortieren, Filtern, Tools, Konfigurieren, Assistent,
Eigenschaften, Kalender, Dokument, Datenbank, Festplatte, Benutzer, Netzwerk. Kennzeichnung: Visio-Design (Motive
allgemein üblich).

#### SH-242 COM und OLE

Komponentenmodelle mit Schnittstellen, Referenzen und Objekthierarchien. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| COM-Objekt | Objekt mit Schnittstellen-Lollipops |
| Referenz | Pfeil für Verweis |
| Schwache Referenz | gestrichelter Verweis |
| V-Tabelle | Tabelle der Methodenzeiger |
| Schnittstelle | Lollipop-Linie |
| Prozessgren… | Linie für Prozessgrenze |
| OLE-Server/Anwendg. | Server-Anwendung |
| Objekthiera… | Baum von Objekten |
| Objektmodell | Objektmodell |
| Verbinder | Verbindungsstück |
| Dokument/Datei | Datei |
| Eingebett. Dokument | eingebettetes Dokument |
| Ordner / Ordner öffnen | Ordner geschlossen bzw. offen |
| Datenspeic… | Speicher |
| Grenze | gestrichelter Rahmen |

#### SH-243 Cursor

Mauszeiger für Bildschirm-Mockups; Motive wie SH-229 (Auswählen, Verschieben, Ausgelastet, Hyperlinkauswahl,
Skalieren vertikal/horizontal/diagonal 1/2, Text, Genauigkeit, Hintergrund, Nicht verfügbar, Hilfe). Kennzeichnung:
Generisch. Neu:

| Shape | Beschreibung |
|---|---|
| QuickInfo | Zeiger mit Tooltip-Kästchen |

#### SH-244 Datenflussdiagramm-Shapes

Datenflussdiagramme mit runden Prozessen sowie Zustandssymbole. Kennzeichnung: Norm: DFD nach Yourdon/DeMarco.

| Shape | Beschreibung |
|---|---|
| Datenverar… | runder Prozess |
| Mehrfachve… | Mehrfachprozess, doppelter Kreis |
| Externe Einheit | Quelle oder Senke |
| Datenspeic… | Datenspeicher zwischen zwei Linien |
| Objekt | Objekt als Wolkenform |
| Mitte an Mitte 1 / 2 | gebogener Datenfluss zwischen Mittelpunkten |
| Schleife auf Mitte 1 / 2 | Rückbezug auf denselben Prozess |
| Zustand | Zustand |
| Anfangszus… | Anfangszustand |
| Endzustand 1 / 2 | Endzustand |
| Variabler Zustand | Zustand mit verstellbarer Form |
| Entitätsbezi… | Beziehungsraute |
| Objektbesc… | Objektbeschriftung |
| Entität 1 / 2 | Entität |
| Ovaler Prozess | Prozess als Ellipse |
| Ovaler Proz. (Datenüber…) | Prozess mit Datenübertragung |

#### SH-245 Dialoge

Windows-Fenster und Meldungen für Oberflächenentwürfe. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Dialogfeldf… | Dialogfenster mit Titelleiste |
| Anwendun… | Anwendungsfenster |
| Tafel | Rahmenfläche |
| Dialogfeld-… | Schließen-Knopf |
| Statusleiste (4 Varianten) | Statusleiste schlicht, mit Feld, mit Verlauf, senkrecht |
| Registerkart… | Tableiste |
| Oberes / Unteres Registerkart… | Tab oben bzw. unten |
| Scrollleiste | Bildlaufleiste |
| Größenzieh… | Größenziehpunkt in der Fensterecke |
| Benachricht… | Benachrichtigungsleiste |
| Fehlersymbol | Fehler-Kreuz |
| Warnungss… | Warnung |
| Information… | Info |
| Fragesymbol | Frage |

#### SH-246 Gane-Sarson

Datenflussdiagramme in Gane-Sarson-Notation. Kennzeichnung: Norm: Gane-Sarson-DFD.

| Shape | Beschreibung |
|---|---|
| Prozess | abgerundetes Rechteck |
| Schnittstelle | externe Einheit, Quadrat |
| Datenspeic… | offenes Rechteck |
| Datenfluss | Pfeil-Verbinder |

#### SH-247 Speicherobjekte

Skizzen von Datenstrukturen im Speicher. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| 3D-Stapel oben / 3D-Stapelel… | Stapel in 3D, Oberteil bzw. Element |
| Stapel oder Heap | Stapelspeicher |
| Stapelzeiger | Zeiger auf Stapel |
| Array | Feld aus Zellen |
| Zeiger / Zeiger (1-D) | Zeiger als Kasten bzw. Pfeil |
| Abschnittsb… | Abschnittsgrenze |
| Datenblock | Speicherblock |
| Speicherabs… | Speicherabschnitt |
| Datenspeic… | Speicherbereich |
| Byte oder Variable | Einzelzelle |
| Knoten einf. verk. Liste | Knoten mit einem Zeiger |
| Knoten dopp. verk. Liste | Knoten mit zwei Zeigern |

#### SH-248 Sprachebenen-Shapes

Strukturdiagramme für Programmmodule und Aufrufe. Kennzeichnung: Norm: Structure Chart (Yourdon/Constantine).

| Shape | Beschreibung |
|---|---|
| Funktion / Subroutine | Modulkasten |
| Funktion mit Aufruf | Modul mit Aufrufmarke |
| Programms… | Programmstruktur |
| Cloud | Wolke für externe Teile |
| Bedingter Aufruf | Aufruf mit Bedingungsraute |
| Lexikalische Inklusion | eingebettetes Modul |
| Aufruf | Aufrufpfeil |
| Aufruf mit Sprung | Aufruf mit Rücksprung |
| Datenfluss | Datenkoppel (offener Kreis) |
| Flagfluss | Steuerkoppel (gefüllter Kreis) |
| Klammer | Klammer |
| Flussdiagr.-… | Flussdiagramm-Symbole |

#### SH-249 Steuerelemente

Klassische Windows-Bedienelemente für Mockups. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Beschriftung | Bezeichnungsfeld |
| Textfeld | Eingabefeld |
| Link | Link |
| Schaltfläche | Knopf |
| Listenfeld | Liste |
| Listenfeldel… | Listeneintrag |
| Dropdown | Auswahlliste |
| Wartekreisel | Drehfeld mit Pfeilen |
| Kontrollkäst… | Kontrollkästchen |
| Optionsfeld | Optionsfeld |
| Strukturste… (2) | Baumansicht bzw. Baumknoten |
| Schieberegler | Schieberegler |
| Statusanzeige | Fortschrittsbalken |
| Kalender | Monatskalender |
| Chevron | Auf-/Zuklapp-Knopf |

#### SH-250 Symbolleisten

Menüs und Befehlssymbole für Mockups. Kennzeichnung: gemischt – Generisch (Menüleisten), Visio-Design (Befehls-Icons,
Motive allgemein üblich).

| Shape | Beschreibung |
|---|---|
| Menüleiste / Menüleiste… | Menüleiste schlicht bzw. mit Eintrag |
| Dropdown… | aufgeklapptes Menü |
| Menüeleme… (2) | Menüeintrag bzw. Trennlinie |
| Ausschneid… / Kopieren / Einfügen / Löschen | Bearbeiten-Befehle |
| Neu / Öffnen / Speichern / Drucken / Seitenansicht | Datei-Befehle |
| Bearbeiten | Stift |
| Rückgängig / Wiederholen | Pfeil zurück bzw. vor |
| Verschieben | Vierfachpfeil |
| Ansichtsopt… | Listensymbol |

#### SH-251 UML-Aktivität

Aktivitätsdiagramme. Kennzeichnung: Norm: UML 2.x.

| Shape | Beschreibung |
|---|---|
| Aktion | abgerundeter Kasten |
| Entscheidung | Raute mit Bedingungen |
| Zusammen… | Raute zum Zusammenführen |
| Anfangskn… | gefüllter Kreis |
| Abschlussk… | Kreis mit Ring |
| Verzweigun… | Balken für Parallelisierung |
| Verbindung… | Balken für Synchronisation |
| Verantwortl… (vertikal) | Partition (Swimlane) |
| Hinweis | Notiz mit Eselsohr |

#### SH-252 UML-Anwendungsfall

Anwendungsfalldiagramme. Kennzeichnung: Norm: UML 2.x.

| Shape | Beschreibung |
|---|---|
| Akteur | Strichmännchen |
| Anwendun… | Ellipse für Anwendungsfall |
| Teilsystem | Systemgrenze (Container) |
| Assoziation | Linie zwischen Akteur und Fall |
| Abhängigkeit | gestrichelter Pfeil |
| Verallgemei… | Pfeil mit hohlem Dreieck |
| Einschließen | Abhängigkeit «include» |
| Erweitern | Abhängigkeit «extend» |

#### SH-253 UML-Klasse

Klassendiagramme. Kennzeichnung: Norm: UML 2.x.

| Shape | Beschreibung |
|---|---|
| Klasse | Kasten mit Name, Attributen, Operationen |
| Mitglied | Zeile für Attribut oder Operation |
| Trennzeichen | Trennlinie zwischen Abschnitten |
| Schnittstelle | Klasse mit «interface» |
| Enumeration | Klasse mit «enumeration» |
| Paket (erweitert) / Paket (reduziert) | Paket mit bzw. ohne Inhalt |
| Vererbung | Pfeil mit hohlem Dreieck |
| Schnittstell… | Realisierung, gestrichelt mit Dreieck |
| Gerichtete Assoziation | Linie mit offener Spitze |
| Aggregation | Linie mit hohler Raute |
| Zusammen… | Komposition, Linie mit gefüllter Raute |

#### SH-254 UML-Kommunikation

Kommunikationsdiagramme. Kennzeichnung: Norm: UML 2.x.

| Shape | Beschreibung |
|---|---|
| Lebenslinie | Objektkasten |
| Akteur | Akteur |
| Nachricht | Pfeil mit Nachricht |
| Antwortnac… | Antwortpfeil |
| Diagrammu… | Diagrammrahmen mit Kopf |

#### SH-255 UML-Komponente

Komponentendiagramme; Paket, Notiz und Beziehungen wie SH-253. Kennzeichnung: Norm: UML 2.x.

| Shape | Beschreibung |
|---|---|
| Komponente | Kasten mit Komponentensymbol |
| Bereitgestel… Schnittstelle | Lollipop |
| Erforderliche Schnittstelle | Halbkreis-Socket |

#### SH-256 UML-Sequenz

Sequenzdiagramme; Lebenslinien wachsen per Ziehen, Fragmente sind Container. Kennzeichnung: Norm: UML 2.x.

| Shape | Beschreibung |
|---|---|
| Objektlebe… | Lebenslinie eines Objekts |
| Akteur-Leb… | Lebenslinie eines Akteurs |
| Aktivierung | Ausführungsbalken |
| Schleifenfra… | Fragment „loop“ |
| Optionales Fragment | Fragment „opt“ |
| Alternatives Fragment | Fragment „alt“ |
| Anderes Fragment | Fragment mit wählbarem Operator |
| Interaktions… | Trennlinie zwischen Operanden |
| Nachricht | synchrone Nachricht |
| Antwortnac… | Antwort, gestrichelt |
| Self-Nachri… | Nachricht an sich selbst |
| Asynchrone Nachricht | Nachricht mit offener Spitze |

#### SH-257 UML-Statuscomputer

Zustandsautomaten („Statuscomputer“ ist eine Fehlübersetzung von „State Machine“). Kennzeichnung: Norm: UML 2.x.

| Shape | Beschreibung |
|---|---|
| Zustand | abgerundeter Kasten |
| Zustand mit internem … | Zustand mit Verhaltensabschnitt |
| Zusammen… Zustand | zusammengesetzter Zustand (Container) |
| Subautoma… | Verweis auf Unterautomat |
| Anfangszus… | Startpunkt |
| Endzustand | Endpunkt |
| Auswahl | Raute für Auswahl |

#### SH-258 UML-Verteilung

Verteilungsdiagramme; Komponente und Schnittstellen wie SH-255, Paket, Notiz und Beziehungen wie SH-253.
Kennzeichnung: Norm: UML 2.x.

| Shape | Beschreibung |
|---|---|
| Knoten / Knotenins… | Quader für Knoten bzw. Knoteninstanz |
| Artefakt / Artefaktinst… | Artefakt bzw. Instanz |
| Bereitstellu… (2) | Bereitstellungsspezifikation |
| Komponent… (Instanz) | Komponenteninstanz |
| Diagrammu… | Diagrammrahmen |
| Manifest | Abhängigkeit «manifest» |
| Bereitstellen | Abhängigkeit «deploy» |

#### SH-259 Unternehmensanwendung

Grobe Architekturbilder mit Rechnern, Speichern und Komponenten. Kennzeichnung: gemischt – Visio-Design
(Rechner-Icons), Generisch (Grenze, Prozess, Objekt, Dokument, Schnittstellen).

| Shape | Beschreibung |
|---|---|
| Server / Server.7 | Server einzeln bzw. mehrfach |
| Mainframe / Mainframes | Großrechner einzeln bzw. mehrfach |
| Arbeitsstati… (2) | Arbeitsplatz einzeln bzw. mehrfach |
| Laptop / Laptops | Notebook einzeln bzw. mehrfach |
| Reserve | Bereitschaftsserver |
| Benutzer | Strichmännchen |
| Grenze | Rahmen für Systemgrenze |
| Datenspeic… | Zylinder |
| Objekt | Ellipse |
| Prozess | Rechteck |
| Beschriftung | Hinweislinie mit Text |
| Dokument | Blatt mit Eselsohr |
| Komponente | Komponentensymbol |
| Schnittstelle | Lollipop-Linie |
| Kommunik… | Kommunikationsverbinder |

#### SH-260 Web- und Mediensymbole (Software)

UI- und Medienicons; Motive wie SH-228 und SH-235 (Suchen, Favorit, Einkaufswagen, RSS, Post, Finden, Anlage,
Kontakt, Chat, Verlauf, Internet, Link, Aktualisieren, Skript, Benutzer, Ein/Aus, Wiedergabe, Pause, Stopp,
Zurückspulen, Zurück, Weiter, Zufällige Wiedergabe, Ton aus, CD, Video, Musik, Fotos, Zuschneiden, Drehen links/rechts).
Kennzeichnung: Visio-Design (Motive allgemein üblich). Neu:

| Shape | Beschreibung |
|---|---|
| Homepage | Haus |
| Diskussion | zwei Sprechblasen |
| Abbrechen (Internet) | Stopp-Achteck |
| Vorlauf | Vorspulen |
| Lauter | Lautsprecher mit Wellen |

**Webdiagramm**

#### SH-261 Konzeptionelle Website-Shapes

Seitenstruktur und Datenflüsse einer geplanten Website. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Gruppe | Gruppenrahmen |
| Hauptobjekt | Hauptseite/-objekt |
| Webseite | einzelne Seite |
| Seitengruppe | Stapel von Seiten |
| Seitenelem… (2) | Element einer Seite |
| Popup | Popup-Fenster |
| Kleiner / Großer Siteübersic… | Sitemap-Rahmen |
| Start / Homepage | Startseite (Haus) |
| Formular | Formular |
| Gateway | Übergang zu anderem System |
| Jump | Sprung zu anderer Stelle |
| 2-Wege-Da… / 1-Weg-Dat… | Datenfluss in beide bzw. eine Richtung |
| Cloud | Wolke |

#### SH-262 Websiteübersicht-Shapes

Inhaltsarten einer Website als Icons, z. B. für automatisch erzeugte Sitemaps. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| HTML | HTML-Seite |
| Grafik (Bitmap) / Grafik (Vektor) | Rastergrafik bzw. Vektorgrafik |
| Text | Textdatei |
| Stylesheet | Stilvorlage |
| Skript (Client) / Skript (Server) | Skript im Browser bzw. auf dem Server |
| Webdienste | Webdienst |
| Java | Java-Programm |
| Audio / Video / Multimedia | Mediendateien |
| Imagemap | Bild mit Klickbereichen |
| Plug-In | Browser-Erweiterung |
| XML | XML-Datei |
| Archiv | gepackte Datei |
| Programm | ausführbares Programm |
| Generisch | Datei unbekannten Typs |
| Dokument / Tabelle / Präsentation / Projekt / Diagramm / Publikation | Büro-Dokumente |
| Website | Website |
| Datenbank | Datenbank |
| Datei | Datei |
| FTP / E-Mail / Newsgroup / Telnet | Internetdienste |
| Suchen | Suche |
| RSS | Feed |
| Start | Startseite |
| Goldener Kreis / Lilafarbene Raute / Blaues Dreiecke / Grünes Quadrat | farbige Markierungsformen |
| Off-Page-V… | Verweis auf andere Seite |

**Nur in älteren Visio-Versionen (binäre Schablonen, Testmaterial `SWDB`)**

Diese vier Schablonen stehen nicht mehr im Menü der aktuellen Version. Sie liegen als binäre
`.vss` (Version 11, Visio 2003–2010, deutsch, metrisch) lokal in `context/files/SWDB_DE/`. Die
Namen wurden aus der unkomprimierten Namensliste der Dateien gelesen. Einzelne Master stehen nur
in komprimierten Blöcken und sind daher nur bruchstückhaft lesbar; sie sind markiert. Die Dateien
sind Microsoft-Inhalt und kommen nie ins Repo.

#### SH-279 Express-G

Schema- und Entitätsdiagramme für Produktdatenmodelle in der grafischen EXPRESS-Notation (STEP).
Kennzeichnung: Norm: ISO 10303-11 (EXPRESS-G).

| Shape | Beschreibung |
|---|---|
| Entität | Kasten für eine Entität |
| Basistyp | Kasten für einen einfachen Datentyp (Zahl, Text, Wahrheitswert …) |
| Aufzählungstyp | Kasten für einen Typ mit fester Werteliste |
| Definierter Typ | Kasten für einen benannten, abgeleiteten Typ |
| Gewählter Typ | Kasten für einen Auswahltyp aus mehreren Typen |
| Kardinalität / Inverse Kardinalität | Mengenangabe einer Beziehung bzw. der Gegenrichtung |
| Normale Beziehung | Beziehungslinie mit Kreis am Zielende |
| Baumstruktur | kräftige Linie für Untertyp-Bäume |
| Beziehungszeiger | Markierung der Beziehungsrichtung (Bedeutung prüfen) |
| Untertyp/ Obertyp | Kennzeichnung einer Unter-/Obertyp-Beziehung |
| Verwendete Entität / Referenzierte Entität | Verweis auf eine Entität aus einem anderen Schema |
| Beschriftung | Textetikett |
| To-Page-Referenz / From-Page-Referenz | Sprungmarke zu bzw. von einer anderen Seite |
| Zeichenblatt-Referenz | Verweis auf eine andere Seite |

#### SH-280 Jackson

Programmstruktur nach der Jackson-Methode (JSP): Bäume aus Sequenz, Auswahl und Wiederholung.
Kennzeichnung: Norm: JSP-Notation (Jackson Structured Programming).

| Shape | Beschreibung |
|---|---|
| Prozedur | Kasten für einen Strukturknoten |
| Prozedur 2 | Variante des Strukturknotens (vermutlich mit Auswahl- oder Wiederholungsmarke) |
| Prozess | Kasten für einen Prozess |
| Strukturverbinder | Linie vom Elternknoten zu den Kindknoten |
| Logischer Verbinder | Linie für eine logische Verbindung zwischen Prozessen |

#### SH-281 ORM-Diagramm

Konzeptionelle Datenmodelle nach Object Role Modeling (ORM/NIAM): Objekttypen, Faktentypen mit
Rollen und Einschränkungen. Kennzeichnung: Norm: ORM-/NIAM-Notation.

| Shape | Beschreibung |
|---|---|
| Entität | Objekttyp (abgerundeter Kasten) |
| Wert / Unär / Binär | Werttyp sowie Faktentypen mit einer bzw. zwei Rollen (nur bruchstückhaft lesbar) |
| Vertikal binär | binärer Faktentyp mit senkrecht gestapelten Rollen |
| Quaternär | Faktentyp mit vier Rollen |
| Rollenverbinder / Erf. Rollenverbinder | Linie von einer Rolle zum Objekttyp, optional bzw. verpflichtend |
| Untertypverbinder | Pfeil vom Untertyp zum Obertyp |
| Einschränkungsverbinder | gestrichelte Linie zu einer Einschränkung |
| Eindeutigkeitseinschränkung / Eindeutigk.-Einschr. links / Mitte / rechts | Strich über Rollen, die eindeutig sind |
| Ext. Eindeutigkts.einschränkung | Eindeutigkeit über Rollen mehrerer Faktentypen |
| Erforderlichkeitseinschränkung / Ext. Erforderlichkts.einschrkg | Pflichtrolle bzw. Pflicht über mehrere Rollen |
| Frequenzeinschränkung / Ext. Frequ.-Einschränkung | Häufigkeitsangabe einer Rolle |
| Ausschlusseinschränkung | Kreis mit Kreuz: Rollen schließen sich aus |
| Gleichheitseinschränkung | Kreis mit Gleichheitszeichen |
| Subseteinschränkung | Kreis mit Teilmengenzeichen |
| Ringeinschränkung | Kennzeichen für Ringbeziehungen (reflexiv, symmetrisch …) |
| P-Einschränkung / Ext. P-Einschränkung | bevorzugter Identifikator (Primärschlüssel) |
| Index- Anmerkung | Anmerkung zu einem Index |

#### SH-282 ROOM

Echtzeitsysteme nach der ROOM-Methode (Real-time Object-Oriented Modeling): Akteure mit
Anschlüssen und Zustandsautomaten. Kennzeichnung: Norm: ROOM-Notation.

| Shape | Beschreibung |
|---|---|
| Akteurklasse | Rahmen einer Akteurklasse mit ihrer inneren Struktur |
| Akteurreferenz / Veränderte Akteurref. | eingebetteter Akteur, normal bzw. veränderlich |
| Relayanschlüsse / Endanschluss / Referenzanschlüsse | Anschlusspunkte am Rand eines Akteurs |
| Bindung | Verbindung zwischen zwei Anschlüssen |
| Layer/Export-Verbindungen | Verbindung zu Diensten der unteren Schicht |
| Zustandskontext | Rahmen eines Zustandsautomaten |
| Zustand | Zustand mit Namen |
| Übergang / Gruppenübergang / Interner Übergang / Übergang zu History | Zustandsübergänge in ihren Varianten |
| Übergangspunkte / Entscheidungspunkt | Anschlusspunkt am Zustandsrand bzw. Verzweigung |
| Textdefinitionen | Textblock für Definitionen |

### 4.10 Terminplan

#### SH-263 Gantt-Diagramm-Shapes

Gantt-Diagramme mit Vorgangstabelle und Zeitskala; Balken folgen den eingegebenen Daten. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Gantt-Diagr… | Rahmen mit Tabelle und Zeitskala |
| Spalte | weitere Tabellenspalte |
| Zeile | weiterer Vorgang |
| Taskleiste | Vorgangsbalken |
| Meilenstein | Raute für Termin |
| Verknüpfun… | Abhängigkeitspfeil zwischen Vorgängen |
| Titel | Diagrammtitel |
| Legende | Legende der Balkenarten |
| Rechtwinkl. Beschriftung | Text mit rechtwinkliger Zeigerlinie |

#### SH-264 Kalender-Shapes

Kalenderblätter mit Datumslogik und Symbole für Einträge. Kennzeichnung: gemischt – Generisch (Kalenderraster,
Termine), Visio-Design (Ereignis- und Wetter-Icons).

| Shape | Beschreibung |
|---|---|
| Monat | Monatsblatt, Datum beim Ablegen wählen |
| Woche / Mehrere Wochen | Wochenblatt |
| Miniaturans… des Monats | kleiner Monatskalender |
| Jahr | Jahresübersicht |
| Tag | Tagesblatt |
| Termin | Eintrag mit Uhrzeit, rastet auf den Tag |
| Mehrtägiges Ereignis | Balken über mehrere Tage |
| Uhr / Erinnerung | Uhr, Glocke |
| Hinweis | Notizzettel |
| Wichtig / Achtung | Ausrufezeichen, Fahne |
| Besprechung | Besprechung |
| Nadel | Pinnadel |
| Idee | Glühbirne |
| Reise-Luft / -Zug / -Auto / -Schiff | Reise mit Verkehrsmittel |
| Besonderes Ereignis | Ereignismarke |
| Urlaub | Urlaub |
| Feier / Geburtstag / Jahrestag | Feste |
| Sport | Sport |
| Fertigstellung | Abschluss |
| Meilenstein | Raute |
| Start | Start-Dreieck |
| Kreisbeschri… / Quadratbes… / Sternbeschr… | Markierungsrahmen um ein Datum |
| Aufgaben | Klemmbrett |
| Mondphasen | Mondphase |
| Sonne / Teilweise bewölkt / Bewölkt / Regen / Blitz / Sturm | Wetter |

#### SH-265 PERT-Diagramm-Shapes

Netzpläne mit Vorgangsknoten. Kennzeichnung: Generisch (Anlehnung an Netzplantechnik nach DIN 69900 möglich).

| Shape | Beschreibung |
|---|---|
| PERT 1 | Vorgangsknoten mit Feldern für Dauer und Termine |
| PERT 2 | Vorgangsknoten mit anderer Feldaufteilung |
| Legende | Erklärung der Felder |
| Zusammen… | Sammelvorgang mit Untervorgängen |

#### SH-266 Zeitachsen-Shapes

Zeitachsen, deren Meilensteine und Intervalle auf ihr Datum einrasten. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Blockzeitac… | Zeitachse als Balken |
| Linienzeitac… | Zeitachse als Linie |
| Zylindrische Zeitachse | Zeitachse als Röhre |
| Erweiterte Zeitachse | vergrößerter Ausschnitt einer Zeitachse |
| Meilenstein Linie / Raute / Pin | Meilenstein mit Strich, Raute, Nadel |
| Meilenstein Dreieck / Dreieck 2 / X / Quadrat / Kreis | weitere Meilensteinformen |
| Eckige Klammern-I… | Intervall mit eckigen Klammern |
| Geschweifte Klammern-I… | Intervall mit geschweiften Klammern |
| Blockintervall | Intervall als Block |
| 'Heute'-Ma… | Markierung des heutigen Tags |
| Verstrichene Zeit | bereits vergangene Zeit |
| Synchronisi… Meilenstein | Meilenstein, der an einer anderen Achse hängt |
| Synchronisi… Intervall | Intervall, das an einer anderen Achse hängt |

### 4.11 Visio-Extras

#### SH-267 Anmerkungen

Beschriftungen und Plan-Kennzeichen für technische und Bauzeichnungen. Kennzeichnung: gemischt – Norm: DIN 1356 /
ISO 7519 (Schnitt-, Höhen-, Nordzeichen), Generisch (Text, Beschriftungen).

| Shape | Beschreibung |
|---|---|
| Beschriftung gekr. / winkl. / ger. Linie | Hinweis mit gebogener, geknickter, gerader Linie |
| Beschrift. Kreis | Hinweis mit Kreis am Ende |
| Ovale / Rechteckige Beschriftung | Hinweis in Ellipse bzw. Rechteck |
| 12 Pt. / 8 Pt. / 6 Pt. Text | Text in fester Größe |
| Spalte 5 / 15 Zeilen | Textspalte mit Zeilen |
| Infolinie | Linie mit Text |
| Umbruchlinie | Bruchlinie |
| Abschnitt 1 / 2 / 3 | Schnittkennzeichen |
| Referenzeb… (Ort) / 1 / 2 | Höhenbezug |
| Benchmark | Höhenfestpunkt |
| Ebene | Höhenkote |
| Bezug | Bezugshöhe |
| Decke | Deckenhöhe |
| Nordpfeil 1 / 2 / 3 / 4 / 5 | Nordpfeil in fünf Formen |
| Überarb. Wolke | Revisionswolke |
| Zeichnungs… | Maßstabsangabe |
| Skalasymbol | Maßstabsleiste |
| Hinweissy… | Hinweisdreieck |
| Referenz Dreieck / Rechteck / Sechseck / Ellipse / Kreis | Positionsnummer in verschiedenen Rahmen |
| Referenzbes… 1 / 2 | Positionsnummer mit Zeiger |

#### SH-268 Bemaßung - Technik

Maßlinien, deren Wert sich aus Geometrie und Zeichnungsmaßstab berechnet. Kennzeichnung: Norm: ISO 129, DIN 406.

| Shape | Beschreibung |
|---|---|
| Horizontale / Vertikale Grundlinie | Bezugsbemaßung von einer Grundlinie |
| Horizontal / Horizontal außen | waagrechtes Maß innen bzw. außen |
| Vertikal / Vertikal außen | senkrechtes Maß |
| Ausgerichtet gleich / Ungleich ausgerichtet | schräges Maß |
| Auß. gleich ausger. / Auß. unglei… | schräges Maß außen |
| Radius / Radius außen / Bogenradius | Radiusmaß |
| Durchmesser / Durchm. außen | Durchmessermaß |
| Winkel von Mitte / gleich / ungleich / außen | Winkelmaß |
| Ordinate horizontal / vertikal | Koordinatenmaß einzeln |
| Ordinaten horizontal / vertikal | Koordinatenmaße in Reihe |
| Raummaß | Raummaß innen |
| Gewichtetes Trapping | Kennzeichen (Name übersetzt, Bedeutung unklar) |
| Sonderzeic… | Sonderzeichen für Maße (±, ⌀, °) |

#### SH-269 Benutzerdefinierte Linienmuster

Ablage für eigene Linienmuster. Inhalt nicht im Material.

#### SH-270 Benutzerdefinierte Muster - Skaliert

Ablage für eigene Füllmuster, die mit dem Maßstab skalieren. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Hilfe zu Mustern | Hilfe-Shape, erklärt eigene Muster |

#### SH-271 Benutzerdefinierte Muster - Unskaliert

Ablage für eigene Füllmuster mit fester Größe; enthält ebenfalls nur „Hilfe zu Mustern“. Kennzeichnung: Generisch.

#### SH-272 Beschriftungen

Callouts mit Zeigerlinie, deren Spitze an einem Shape klebt und ihm folgt. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Seitl. rechteckig… | Kasten mit seitlicher Zeigerlinie |
| Beschriftung m. Linie, s… | Text mit Linie seitlich |
| Textbeschri… seitlich | Text mit Zeiger seitlich |
| Rechteckige Beschriftu… | Kasten mit Zeiger |
| Beschriftung mit Linie, … | Text mit Linie |
| Textbeschri… Mitte (2) | Text mit Zeiger aus der Mitte |
| Textfeld, 90° Seite / Mitte | Kasten mit rechtwinkligem Zeiger |
| Text m. Linie 90° Seite / Mitte | Text mit rechtwinkliger Linie |
| Seitenbesch… 90° | seitlicher Text mit Winkelzeiger |
| Mittige Beschriftu… | Text mit mittigem Zeiger |
| Anmerkung | Anmerkung mit Haken |
| Knickbesch… | Zeiger mit Knick |
| Text mit geschweift… / Geschw. Klammer… | Text mit geschweifter Klammer |
| Eckige Klammer… | Text mit eckiger Klammer |
| Klammer mit Text rechts | Klammer, Text rechts |
| Text mit Teilklammer | Text mit halber Klammer |
| Eingeklam… Text | Text zwischen Klammern |
| Beschriftung Ellipse / Rechteck | Hinweis in Ellipse bzw. Rechteck |
| Text mit Linie | Text mit Unterstrich-Zeiger |
| Datei | Ordner-Callout |
| Gelber Notizzettel | Haftnotiz |
| Tag | Anhänger-Etikett |
| 2D-Sprechb… | Sprechblase als Fläche |
| Sprechblase horizontal / vertikal | Sprechblase mit Zeiger |
| Aufkleber eckig / Abgerundet… Aufkleber | Aufkleber |
| Gezackter Stern 1 / 2 | Explosionsstern |
| Auto-Bema… | Maß, das sich selbst berechnet |
| Benutzerdef. Beschriftu… (3) | frei gestaltbare Callouts |

#### SH-273 Ornamente

Bordüren, Rahmen und Kacheln als Schmuck. Kennzeichnung: Visio-Design.

| Shape | Beschreibung |
|---|---|
| Wellenabsc… / Wellenabsc… 2 / Wellenecke / Wellenecke 2 / 3 | Wellenbordüre: Abschnitt und Ecken |
| Flechtenabs… / Flechtenecke / Flechtenende | Flechtbordüre |
| Ägyptischer Abschnitt / Ägyptische Ecke / Ägyptisches Ende | Bordüre im ägyptischen Stil |
| Griechischer Abschnitt / Griechische Ecke / Griechischer Rahmen | Mäander |
| Kreuzabsch… / Kreuzecke / Kreuzende | Kreuzbordüre |
| Römischer Abschnitt | römische Bordüre |
| Sternabsch… | Sternenbordüre |
| Abschnitt Dreieck | Dreiecksbordüre |
| Karoabschn… | Karobordüre |
| 3D-Rahmen / Rechteckiger Rahmen / Fotorahmen / Kunstrahmen | Bilderrahmen |
| Edelsteinra… / Klassischer Rahmen / Phantasiera… | Zierrahmen |
| Rahmen, einfac… / Rahmen, mehrer… | schlichter bzw. mehrfacher Rahmen |
| Kunstkreis | Zierkreis |
| Kunstkachel / Wellenkachel / Gewebekac… / Zickzack-Ka… / Op-Art-Kac… / Rautenkachel / Grafische Kachel | Musterkacheln |
| Keltisches Ornament | Knotenmuster |
| Schaltfläch… | Zierknopf |
| Bogenorna… | Bogenornament |

#### SH-274 Symbole

Piktogramme für Hinweise, Leitsysteme und Wetter. Kennzeichnung: gemischt – Norm: ISO 7001/7010/7000 (Piktogramme),
Visio-Design (Wetter); „Recycling 1“ ähnelt einem geschützten Zeichen und wird nicht nachgebaut.

| Shape | Beschreibung |
|---|---|
| Zone | roter Kreisrahmen |
| Verbotsschild | Verbotskreis |
| Rauchverbot | Rauchen verboten |
| Warnschild | Warndreieck |
| Flughafen / Schienenve… / Bushaltestelle | Verkehr |
| Herren / Frauen / Behindert | WC und Barrierefreiheit |
| Kaffee / Restaurant / Getränke | Gastronomie |
| Unterkunft | Übernachtung |
| Telefon / Post / Information | Dienste |
| Fahrrad / Park / Tankstelle / Treppe | Wege und Orte |
| Mülleimer | Abfall |
| Zerbrechlich | Glas, Vorsicht zerbrechlich |
| Radioaktiv | Strahlungswarnung |
| Lebensgefahr | Biogefährdung |
| Quadratisc… Feld | leeres Piktogrammfeld |
| Sonne / Teilweise bewölkt / Bewölkt / Regen / Blitz / Sturm | Wetter |
| Recycling 1 | Kreispfeile (nicht nachbauen) |
| Recycling 2 | Recyclingdreieck aus Pfeilen |

#### SH-275 Symbolsätze

Statussymbole, deren Zustand über Shape-Daten oder Datengrafiken umgeschaltet wird. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Trendpfeile 1 / 2 | Pfeil hoch, seitwärts, runter |
| Kennzeichen | Fahne in Farben |
| Zustandssy… | Haken, Ausrufezeichen, Kreuz |
| Farbige Formen | farbige Kreise und Formen |
| Leuchten | Leuchtpunkt |
| Ampel 1 / 2 / 3 | Ampel senkrecht bzw. waagrecht |
| Wegbeschr… | Richtungsdreieck |
| Bewertung | Signalbalken |
| Gesichter | Smileys |
| Gefüllte Kreise | Kreis mit Füllgrad |
| Kästchen | Viertel-Kästchen mit Füllgrad |

#### SH-276 Titelblöcke

Zeichnungsrahmen und Schriftfelder; Felder füllen sich aus Dokumenteigenschaften. Kennzeichnung: Norm: ISO 5457,
ISO 7200.

| Shape | Beschreibung |
|---|---|
| Zonen - 4 / Zonen - 8 | Rahmen mit Feldeinteilung |
| Rahmen | Zeichnungsrahmen |
| Projektierung | Projektionssymbol |
| Spalte 5 / 15 Zeilen | Tabellenspalte |
| SI-Symbol | Kennzeichen „SI-Einheiten“ |
| Revisionsbl… | Änderungstabelle |
| Stücklisten… | Stückliste |
| Abschnittsü… | Abschnittsübersicht |
| Block, Text seitl. / Block, Text oben | Feld mit Beschriftung seitlich bzw. oben |
| Datum / Beschreibung / Erstellt von / Dateiname / Datei/Pfad / Zeichenblat… / Überarbeitet / Maßstab / Titel | Einzelfelder des Schriftfelds |
| Titelblock groß / klein | vollständiges Schriftfeld |
| Fortsetzung… groß / klein | Schriftfeld für Folgeblätter |
| Überarbeitu… groß / klein (je 2) | Revisionsfeld |

#### SH-277 Verbinder

Sammlung von Verbindern mit festem oder automatischem Verlauf. Die Linien mit Endsymbolen sind wie in SH-002
beschrieben (Gepunktete Linie, Pfeil - Mitte, Gebogener Pfeil, Punkt und Pfeil, Pfeil mit Endpunkten, Einfache
Pfeilspitze, Doppelspitze, 2 Zweige, Zweige schräg/eckig). Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Universeller Verbinder | Verbinder mit allen Routing-Arten |
| Layoutverbi… | Verbinder, der dem Seitenlayout folgt |
| Verbinder: Linie/Bogen | gerade oder als Bogen |
| Verbinder: Linie | gerader Verbinder |
| Kurvenverbi… 1 / 2 / 3 | geschwungener Verbinder |
| Welliger Verbinder 1 / 2 | Wellenlinie |
| Winkelverbi… 1 | Verbinder mit einem Knick |
| Seitenweise 1 / 2 / fest 1 / fest 2 | rechtwinklig von Seite zu Seite |
| Seiten | Verbinder zwischen Seitenflächen |
| Von unten nach oben 1 / 2 / Unten nach oben | von der Unter- zur Oberkante |
| Von oben nach unte… (2) | von der Ober- zur Unterkante |
| Oben oder unten | oben oder unten anschließend |
| Oben/unten nach Seite / Oben/unten - Seite | von oben/unten zur Seite |
| Seite nach oben/unten / Seite nach oben / Seite - oben/unten | von der Seite nach oben/unten |
| Unten nach Seite 1 / 2 / 3 | von unten zur Seite |
| Seite nach Seite | Seite zu Seite |
| Schleife (eckig) | Rückführung zum selben Shape |
| 2 Zweige (eckig) | rechtwinklige Zweiteilung |
| Gegabelter Verbinder | Gabel |
| Linie mit Pfeil 1 / 2 | Linie mit Spitze |
| Pfeil und Punkt 1 / 2 | Linie mit Spitze und Punkt |
| Übergabe der Kontrolle | Pfeil für Kontrollübergabe |
| Sprung | Linie mit Sprungbogen |
| Flexibler Pfeil | biegbarer Flächenpfeil |
| 1D-Pfeil / 1D-Doppel… / 1D-Pfeil, Ende offen | Flächenpfeil mit Klebeenden |
| 90-Grad 1 / 2 | Winkelpfeil |
| Hohler Verbinder 1 / 2 / 3 | Hohlpfeil als Verbinder |
| 2 Linien / 4 Linien | parallele Leitungen |
| 2 Winkellinien / 4 Winkellinien | parallele Leitungen mit Knick |
| DFÜ-Verbin… 1 / 2 | Blitzlinie |
| Bus | Bus mit Abzweigen |
| Ethernet | Ethernet-Bus |
| Stern | Sternverteiler zu mehreren Shapes |

#### SH-278 Zeichentools

Konstruktionshilfen, die Formen aus Punkten und Maßen erzeugen. Kennzeichnung: Generisch.

| Shape | Beschreibung |
|---|---|
| Maßtool | Messlinie |
| Horizontales / Vertikales Maß | Maßlinie waagrecht bzw. senkrecht |
| Kreis, Ellipse | Kreis oder Ellipse |
| Kreis, Durchm. / Kreis, Radius / Kreis, 3 Pt. | Kreis aus Durchmesser, Radius, drei Punkten |
| Kreise | konzentrische Kreise |
| Kreis, Tangente / 2 Kreise, Tangente | Tangente an einen bzw. zwei Kreise |
| Kreistangen… | Kreis an Tangente |
| Bogen, 3 Pt. / Bogen, grafisch / Bogen, numerisch | Bogen aus Punkten bzw. Werten |
| Bogentang… | Bogen an Tangente |
| Sektor, grafisch / Sektor, numerisch | Kreisausschnitt |
| Linie mit Verläng. | Linie mit Verlängerung |
| Rechter Winkel / Rechtw. Linien | rechter Winkel, senkrechte Linien |
| Dreieck, frei / Dreieck: Basis, Höhe | Dreieck frei bzw. aus Maßen |
| Rw. 3eck, WinkHyp. / Rw. 3eck, 2 Seiten | rechtwinkliges Dreieck aus Winkel bzw. Seiten |
| Rechteck / Rechteck, Diagonale | Rechteck aus Seiten bzw. Diagonale |
| Abgerundet… Rechteck | Rechteck mit runden Ecken |
| Schräges Rechteck / Schräge Ecke | gedrehtes Rechteck, Fase |
| Polygon, Kante / Polygon, Mittelpkt. | Vieleck aus Kante bzw. Mittelpunkt |

### 4.12 Mechaniken, die die Shapes voraussetzen

Aus den Standbildern erschlossen; das Verhalten selbst ist im Material nicht zu sehen.

| Mechanik | Beispiele | Bezug |
|---|---|---|
| Verbinder mit automatischer Führung, Klebeenden und Verbindungspunkten; feste Verläufe (seitlich, oben/unten) als eigene Verbinder | Dynamischer Verbinder in fast jeder Schablone; SH-277, SH-090, Wirkungslinien in SH-088 | CN-04, CN-05 |
| Steuerpunkte für Formparameter (Eckenradius, Pfeilspitze und Schaft, Zackentiefe, Neigung, Ringbreite) | SH-006, SH-019, SH-003, SH-004 | SS-30, SP-12 |
| Geometrie aus Formeln: Anzahl von Stufen, Ebenen, Segmenten, Treppenstufen; Ziehen vermehrt Elemente | SH-032, SH-033, SH-030, SH-180, Ethernet und Ringnetzwerk in SH-151, Parkstreifen in SH-194 | SP-12, DA-01 |
| Container, die Shapes aufnehmen und mitwachsen | Swimlanes SH-016, BPMN-Pool SH-015, UML-Fragmente SH-256, Systemgrenze SH-252, Teamrahmen SH-045, Gruppen-Rahmen SH-107 und SH-136 | ST-01 |
| Listen-Shapes mit einfügbaren Zeilen | Klasse/Mitglied SH-253, Entität/Attribut SH-223 und SH-225, Mitarbeiter-Liste SH-045, Gantt-Zeilen SH-263 | ST-04 |
| Callouts, deren Zeiger am Ziel-Shape klebt und mitwandert | SH-272, SH-267, SH-029, Pflanzenbeschriftung SH-196 | ST-06 |
| Zählende Legenden, Stücklisten und Berichte aus den Shapes der Zeichnung | Legende SH-151, SH-028, Listen in SH-089, Türen-/Fensterplan SH-212, Raum- und Inventarbericht SH-200 | ST-09, DA-01 |
| Umschaltbare Varianten über Shape-Daten oder Kontextmenü | Gattertyp SH-057, BPMN-Marker SH-015, „wählbar“-Pumpen SH-086, Ampel/Status SH-275, Kardinalität SH-223, Balken offen/geschlossen SH-002 | DA-01, SS-31 |
| Einfüge-Aktionen, die mehrere Shapes auf einmal anlegen | Mehrere Shapes und Drei Positionen SH-045, Mehrere Themen SH-027 | SS-31 |
| Datumsgebundene Shapes, die auf Tag oder Zeitpunkt einrasten | Kalender SH-264, Gantt SH-263, Zeitachse SH-266 | DA-01, SP-12 |
| Maßstäbliche Shapes, deren Größe dem Zeichnungsmaßstab folgt | Grundriss SH-212, Möbel SH-192, Fahrzeuge SH-176, Rack in Höheneinheiten SH-146, Kanäle SH-185 | PG-06 |
| Maße, die ihren Wert aus Geometrie und Maßstab berechnen | SH-268, SH-278, Auto-Bemaßung SH-272, Rack-Bemaßung SH-146 | PG-06, SP-12 |
| Andocken und Einrasten: Türen/Fenster in Wände, Ventile in Leitungen, Einschübe ins Rack, Wände untereinander, „intelligente“ Straßen | SH-212, SH-213, SH-091, SH-146, SH-220 | SP-12, CN-04 |
| Gemeinsamer Steuer-Shape, der viele Shapes beeinflusst | Fluchtpunkt SH-005 | SP-12 |
| Shapes, die Werte oder Datenquellen darstellen | Balken- und Tortendiagramm SH-029, Pivotknoten SH-056 | DA-01 |
| Felder aus Dokumenteigenschaften | Titelblöcke SH-276, IDEF0-Titelblock SH-017 | SP-12, DA-01 |
| Quick-Bereich je Schablone, Sammelansicht Quick-Shapes, „Hier Quick-Shapes ablegen“, Meine Shapes/Favoriten | S 1104-131524, S 1105-101519, S 1104-132533 | SH-G03, SH-G04 |
| Stilvarianten statt getrennter Schablonen (2D/3D, elf Organigramm-Stile, „– Visio 2013“) – in JFDiagrammer besser ein Master mit Stilwahl | SH-009/010, SH-045 bis SH-055, SH-063/064 | SH-G01, SH-G02 |

### 4.13 Hinweise

**Fehlübersetzungen in der deutschen Visio-Oberfläche.** JFDiagrammer verwendet die richtigen Begriffe; die
Visio-Namen bleiben als Suchbegriffe erhalten, damit Umsteiger sie finden.

| Visio-Name | Gemeint | Fundstelle |
|---|---|---|
| Lautstärke | Volume (Datenträger, freigegebener Ordner) | SH-093, SH-119 |
| Tabelle | Tisch | SH-155 |
| Spalte | Kolonne | SH-085 |
| Bildschirm | Sieb | SH-092 |
| Balken | Bar | SH-189 |
| Beenden | Ausgang | SH-189 |
| Aktueller Transformator | Stromwandler | SH-072 |
| Potentialtransformator | Spannungswandler | SH-072 |
| Stiftdiode | PIN-Diode | SH-063 |
| Trennzeichen | Abscheider | SH-079 |
| Ebene | Füllstand | SH-186 |
| Microsoft Edge-Speicherort | Edge-Standort (AWS) | SH-102 |
| Statuscomputer | Zustandsautomat | SH-257 |
| Netzwerkadressen | Netzwerk-Standorte | SH-153, SH-154 |
| Überfüllen | Spreiz-/Pinch-Geste | SH-229 |
| Blaues Dreiecke | Blaues Dreieck | SH-262 |

**Namensgleichheiten.** „Allgemeine Symbole“ (SH-228, SH-241) und „Web- und Mediensymbole“ (SH-235, SH-260) gibt es
je zweimal mit verschiedenem Inhalt; „Pfeil-Shapes“ steht in zwei Kategorien, ist aber dieselbe Schablone (SH-019).
SH-040 hat denselben Inhalt wie SH-021, SH-158 dieselben Rollen wie SH-147. Viele Shape-Namen wiederholen sich mit
unterschiedlicher Bedeutung (z. B. „Kategorie“, „Ereignis“, „Prozess“, „Verbindung“). JFDiagrammer braucht deshalb
eindeutige interne Namen je Master und die Schablone als Kontext in Suche und QuickInfo.

**Markenrecht.** Nicht nachgebaut werden die Microsoft-Produktlogos (SH-150), die iMac-Formen (SH-141; generisch als
„All-in-One“) und das Symbol „Recycling 1“ (SH-274, ähnelt einem geschützten Zeichen). Produktgebundene Schablonen
(SharePoint SH-008 und SH-011 bis SH-014, Exchange SH-144, Active Directory SH-093/094) werden nur bei Bedarf und
mit neutralen Namen nachgebaut. Die AWS-Icons dürfen nur unverändert im Original verwendet werden (SH-G06), die
Azure-Icons gar nicht mitgeliefert werden (SH-G07).

**US/ANSI-Darstellung gegenüber DIN EN/IEC.** Viele Norm-Schablonen zeichnen im US-Stil: Elektrotechnik (SH-057 bis
SH-074, z. B. Widerstand als Zickzack, Schalter- und Relaisformen nach IEEE 315), Elektroinstallation (SH-175), HLK
(SH-184 bis SH-187, SH-206) und Sanitär (SH-205). Auch Fahrzeuge (Pick-ups, Schulbus, SH-176), Sportfelder (American
Football, Baseball, SH-209) und die Verkehrszeichen (SH-220) sind US-geprägt. Entschieden (F14): Die
Norm-Schablonen werden nach DIN EN/IEC gezeichnet (IEC 60617, ISO 10628, ISO 1219, DIN 1356), eine ANSI-Variante
höchstens später. Die Prio-Spalte in 4.2 gilt für die DIN-EN/IEC-Fassung.

**Grenzen des Materials.** 31 Schablonen stehen nur im Menü. Viele Namen sind in Visio abgeschnitten („…“); Ergänzungen
in Klammern sind sinngemäß. Bei einigen Shapes ist die Bedeutung aus Bild und Namen nicht sicher (z. B. „LSX“ in
SH-169, „Gewichtetes Trapping“ in SH-268, „Netznieders…“ in SH-058); sie sind in den Tabellen so markiert.

### 4.14 Vorschläge für eigene Kategorien

| ID | Kategorie | Inhalt | Grundlage/Norm | Begründung | Prio |
|---|---|---|---|---|---|
| SH-E01 | Kubernetes und Container-Plattformen | Pod, Deployment, ReplicaSet, Service, Ingress, Namespace, Volume, ConfigMap, Secret, Helm-Chart | offizielle Kubernetes-Icons (freie Lizenz, vor Übernahme prüfen) | Visio hat nur AWS-/Azure-Dienste; Container-Plattformen sind Alltag in Architekturbildern. | P2 |
| SH-E02 | C4-Modell | Person, Softwaresystem, Container, Komponente, Beziehung, Systemkontext-Rahmen | C4-Modell (offene Methode) | Herstellerneutrale Architekturdarstellung aus schlichten Formen, in Visio nicht vorhanden. | P2 |
| SH-E03 | Elektro-Hausinstallation | Steckdosen, Schalter, Leuchten, Verteiler, LS-Schalter, FI-Schutzschalter, Zähler, Leitungen | DIN EN 60617-11 | SH-175 ist ANSI-geprägt und dünn; für den deutschen Markt der wichtigste Elektroplan. | P2 |
| SH-E04 | Stromlauf- und Verteilerplan | Schütze, Reihenklemmen, Sicherungen, RCD, Klemmleisten, Betriebsmittelkennzeichnung | DIN EN 81346, IEC 60617 | Hauptanwendung im Elektrohandwerk; in Visio nur verstreut. | P3 |
| SH-E05 | Smart Home und Gebäudeautomation | KNX-Linie und -Bereich, Matter/Thread, Zigbee, Sensoren, Aktoren, Gateways, Szenen | KNX-Topologie, herstellerneutral | Passt zur Netzwerk-Kompetenz von JFNetworker; Visio hat nur AWS-IoT-Ressourcen. | P2 |
| SH-E06 | Photovoltaik und Energie | PV-Modul und -String, Wechselrichter, Batteriespeicher, Wallbox, Wärmepumpe, Zähler, Netzanschluss | IEC 60617, Übersichtsschaltbild nach VDE-AR-N 4105 | Stark gefragt; Visio kennt nur Kraftwerkssymbole (SH-066). | P3 |
| SH-E07 | Git-Flows und Versionsverwaltung | Commit, Branch, Merge, Tag, Pull Request, Release | verbreitete Konvention | Entwickler bauen solche Bilder in Visio mühsam aus Kreisen und Linien. | P3 |
| SH-E08 | Flucht- und Rettungspläne | Rettungsweg, Notausgang, Sammelstelle, Feuerlöscher, Erste Hilfe, Standort | DIN ISO 23601, ISO 7010 | Visio bietet nur einzelne Piktogramme (SH-189, SH-274) ohne Plan-Systematik. | P2 |
| SH-E09 | Feuerwehr- und Brandschutzpläne | Brandabschnitte, Brandmelder, Zugänge, Löschwasser, Sperrbereiche | DIN 14095, DIN 14034-6 | Genormt und in Deutschland vorgeschrieben; in Visio fehlend. | P3 |
| SH-E10 | ArchiMate 3.x | Business-, Application-, Technology-Elemente und Beziehungen | ArchiMate (The Open Group) | Verbreitete Notation für Unternehmensarchitektur, in Visio nicht enthalten. | P3 |
| SH-E11 | GRAFCET, Kontaktplan, Funktionsplan | Schritte, Transitionen, Aktionen; Kontakte, Spulen, Funktionsbausteine | IEC 60848, IEC 61131-3 | Normnotationen der Steuerungstechnik, in Visio nicht vorhanden. | P3 |
| SH-E12 | SysML | Block, Anforderung, Parametrik, Zustand, Aktivität | OMG SysML | Systems Engineering baut auf UML auf; Visio hat nur UML. | P3 |
| SH-E13 | Netzwerk-Dokumentation im JFNetworker-Stil | Patchfeld mit Portbelegung, VLAN- und Subnetz-Bereiche, PoE, WLAN-Ausleuchtung, Kabeltypen als Verbinder | JFNetworker-Symbole und -Datenmodell | Vorhandenes Know-how; Visio hat Geräte-Icons ohne Port-Logik. | P2 |
| SH-E14 | Genogramm und Stammbaum | Person, Ehe, Trennung, Kinder, Generationen, Beziehungsarten | Genogramm-Symbole nach McGoldrick | Häufiger Wunsch in Beratung und Familienforschung, in Visio nicht vorhanden. | P3 |
| SH-E15 | Datenschutz und ISMS | Verarbeitungstätigkeit, Datenkategorie, Empfänger, Drittland, Schutzmaßnahme | DSGVO Art. 30, BSI IT-Grundschutz | Für DSGVO- und Sicherheitsdokumentation hat Visio nichts Passendes. | P3 |

<a id="vorlagen"></a>
## 5. Vorlagen, Diagrammtypen und Assistenten

Eine **Vorlage** öffnet eine neue Zeichnung mit passenden Schablonen, Seitengröße, Maßstab,
Ebenen, Design, Regelsätzen, ggf. Startdiagramm und ggf. eigenem Kontextregister. Visios
Vorlagendateien sind geschützte Inhalte: JFDiagrammer baut **eigene** Vorlagen mit demselben
Zweck (Ort `App`, Bestand überall „neu“, außer wo vermerkt). Jede Vorlage gibt es in metrischer
und US-Fassung (MS-07). Die Kategorien folgen „Neu“ bzw. „Weitere Shapes“.

### 5.1 Vorlagen-Infrastruktur

| ID | Feature | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| VL-01 | Vorlagenkatalog | Kategorien mit Vorschaubildern (aus der Vorlage gerendert), Beschreibung, Einheitenwahl, Startdiagramme mit löschbarem Tipp-Shape. | SbS 1; A 190353 | P1 | App | neu |
| VL-02 | Vorlagen-Inhalt | Liste der zu öffnenden Schablonen (Reihenfolge, welche aktiv), Seiten (auch Hintergrund), Seitengröße, Maßstab, Einheiten, Ebenen, Design, Formatvorlagen, Regelsätze, Kontextregister. | SbS 1, 12 | P1 | App+Lib-core | neu |
| VL-03 | Eigene Vorlagen | Speichern als Vorlage, eigener Vorlagenordner erscheint als Kategorie; Doppelklick auf eine Vorlage im Dateimanager startet eine neue Zeichnung. | SbS 12 | P1 | App | neu |
| VL-04 | Vorlagenabhängige Register | Organigramm, Swimlanes, Brainstorming, Zeitachse, Kalender, Gantt bringen eigene Register/Kontextmenüs mit (3.10). | SbS 1, 4, 5; TP | P1 | App | neu |

### 5.2 Vorlagen nach Kategorie

| ID | Kategorie | Vorlage | Zweck, Schablonen, Besonderheiten | Quelle | Prio | Norm? |
|---|---|---|---|---|---|---|
| VL-10 | Allgemein | Leere Zeichnung | ohne Schablone, Einheitenwahl | A 183424, A 190353 | P1 | – |
| VL-11 | Allgemein | Standarddiagramm | Standard-Shapes, Blöcke, Pfeile, Dekorative Shapes | A 183424; S 1104-131850 | P1 | generisch |
| VL-12 | Allgemein | Blockdiagramm, Blockdiagramm mit Perspektive, 3D-Blockdiagramm | Blöcke, 3D-Blöcke, perspektivische Blöcke mit Fluchtpunkt | S 1104-131850 | P2 | generisch |
| VL-20 | Flussdiagramm | Standardflussdiagramm | ISO-5807-Symbole, Regelsatz Flussdiagramm, Querformat, 8-pt-Text | SbS 4; A 183424 | P1 | ISO 5807 |
| VL-21 | Flussdiagramm | Funktionsübergreifendes Flussdiagramm | Swimlanes (ST-05), waagerecht/senkrecht, Phasen, Kontextregister | SbS 4, 13 | P1 | – |
| VL-22 | Flussdiagramm | BPMN-Diagramm | BPMN-2.0-Shapes mit Typwahl per Kontextmenü, BPMN-Regelsatz | SbS 4, 14 | P2 | BPMN 2.0 |
| VL-23 | Flussdiagramm | Arbeitsflussdiagramm (2D/3D) | Abteilungen, Workflowobjekte, -schritte | SbS 4; S 1104-131922 | P2 | – |
| VL-24 | Flussdiagramm | IDEF0-Diagramm | Aktivitätskästen mit ICOM-Pfeilen | SbS 4 | P3 | IDEF0 (FIPS 183) |
| VL-25 | Flussdiagramm | SDL-Diagramm | SDL-Symbole | SbS 4 | P3 | ITU-T Z.100 |
| VL-26 | Flussdiagramm | SharePoint-Workflow | **entfällt** (NO-05) | SbS 4; S 1104-131922 | – | – |
| VL-30 | Geschäft | Organigramm, Organigramm-Assistent | Organigramm-Shapes in mehreren Stilen, Ablegen auf Vorgesetzten, Layouts, Assistent (VL-80) | SbS 5; TP | P2 | – |
| VL-31 | Geschäft | Brainstormingdiagramm | Themen, Gliederungsfenster, Export/Import | TP; S 1104-131939 | P3 | – |
| VL-32 | Geschäft | Ursache-Wirkung-Diagramm (Ishikawa) | Fischgräten-Gerüst mit Haupt- und Nebenursachen | S 1104-131939 | P2 | Methode |
| VL-33 | Geschäft | Auditdiagramm, EPK-Diagramm | Prüfablauf; ereignisgesteuerte Prozesskette | S 1104-131939 | P2 | EPK-Notation |
| VL-34 | Geschäft | Fehlerstrukturanalyse | Fehlerbaum mit Logikgattern | S 1104-131939 | P3 | IEC 61025 |
| VL-35 | Geschäft | ITIL-Diagramm | IT-Service-Prozesse | S 1104-131939 | P3 | – |
| VL-36 | Geschäft | Six Sigma, House of Quality, TQM | Qualitätsmanagement-Diagramme | SbS 14; S 1104-131939 | P3 | Methode |
| VL-37 | Geschäft | Wertstromdiagramm | Wertstrom-Symbole (Lean) | SbS 1; S 1104-131939 | P3 | Lean-Notation |
| VL-38 | Geschäft | Diagramme und Grafiken, Marketingdiagramme | Balken-/Torten-Shapes, Matrix, Pyramide, Zyklus, Venn | S 1104-131939 | P2 | generisch |
| VL-39 | Geschäft | Pivotdiagramm | Datenhierarchie als Baum (VL-82) | S 1104-131939 | P3 | – |
| VL-40 | Konstruktion | Grundlagen Elektrotechnik, Schaltkreise und Logik, Industriesteuerungen, Elektrische Systeme | Elektro-Symbole | SbS 1; S 1104-132112 | P2 | IEC 60617 (DIN-EN-Fassung, F14) |
| VL-41 | Konstruktion | Teile und Baugruppen, Fluidtechnik | Federn, Lager, Schrauben, Schweißsymbole, Form- und Lagetoleranzen, Pneumatik/Hydraulik | S 1104-132112 | P3 | ISO 1219, ISO 2553, ISO 1101 |
| VL-42 | Konstruktion | R&I-Fließbild, Prozessflussdiagramm | Behälter, Pumpen, Wärmetauscher, Instrumente, Rohrleitungen, Armaturen | S 1104-132112 | P3 | ISO 10628, ISA 5.1 |
| VL-50 | Netzwerk | Standardnetzwerk (2D/3D) | Computer und Monitore, Netzwerk und Peripheriegeräte | SbS 7 | P1 | – |
| VL-51 | Netzwerk | Detailliertes Netzwerk (2D/3D) | + Server, Netzwerksymbole, Netzwerkadressen, Raumelemente; Netzwerkberichte | SbS 7; A 183424 | P1 | – |
| VL-52 | Netzwerk | Rack-Diagramm | Gestelle, gestellmontierte Geräte/Server, Höheneinheiten, Maßstab 1:20; wegen JFNetworker schon in P1 (F11) | SbS 7; Entscheidung F11 | P1 | 19-Zoll (EIA-310) |
| VL-53 | Netzwerk | Active Directory, LDAP-Verzeichnis | Verzeichnisobjekte, Standorte und Dienste | SbS 7; S 1104-132208 | P3 | – |
| VL-54 | Netzwerk | AWS-Architektur | AWS-Schablonen (Original-Icons) + Gruppenrahmen; früh in P2 (F11) | S 1104-132208; Entscheidung F11 | P2 | AWS-Original |
| VL-55 | Netzwerk | Azure-Architektur | nur nach Azure-Download (Abschnitt 4) | S 1104-132208 | P3 | Azure-Download |
| VL-60 | Pläne und Grundrisse | Grundriss, Hausplan, Büroplan | Wände/Türen/Fenster, Möbel, Bad/Küche, Bemaßung; Maßstab | TP; S 1104-132248 | P2 | DIN 1356 (Bauzeichnung, wo zutreffend) |
| VL-61 | Pläne und Grundrisse | Elektro- und Telekommunikationsplan | Installationssymbole | S 1104-132248 | P2 | DIN EN 60617-11 |
| VL-62 | Pläne und Grundrisse | HLK-Plan, HLK-Steuerungslogik, Sanitär- und Rohrleitungsplan | Lüftung, Heizung, Rohre, Ventile | S 1104-132248 | P3 | teils Norm |
| VL-63 | Pläne und Grundrisse | Deckenspiegel, Lageplan, Raumplan, Fabrikplan, Sicherheits- und Zutrittsplan, Flucht- und Rettungsplan | Raumbelegung, Außenanlagen, Maschinen, Alarm/Zutritt, Rettungswege | S 1104-132248, S 1104-132303 | P3 | ISO 7010 (Rettungszeichen) |
| VL-64 | Pläne und Grundrisse | Wegbeschreibung, Wegbeschreibung 3D | Straßen, Verkehr, Gebäude, Stadtbahn | S 1104-132317; S 2024 | P3 | – |
| VL-70 | Software und Datenbank | UML (Klasse, Sequenz, Anwendungsfall, Aktivität, Komponente, Verteilung, Zustand, Kommunikation) | UML-2.x-Shapes | SbS 1; S 1104-132405 | P2 | UML 2.5 |
| VL-71 | Software und Datenbank | Datenbankmodell (Krähenfuß, Chen, IDEF1X, UML-Datenbank, objektrelational) | Entitäten, Beziehungen | S 1104-132405 | P2 | Notationen |
| VL-72 | Software und Datenbank | Datenflussdiagramm (Gane-Sarson, Yourdon) | Prozesse, Speicher, Flüsse | S 1104-132405 | P3 | Notation |
| VL-73 | Software und Datenbank | Wireframe (Web, Mobil) | Steuerelemente, Container, Navigation | SbS 13; TP | P3 | – |
| VL-74 | Software und Datenbank | Websiteübersicht, konzeptionelle Website | Seitenstruktur | S 1104-132405 | P3 | – |
| VL-80 | Terminplan | Zeitachse | Block-/Linien-/Zylinderzeitachse, Meilensteine, Intervalle, „Heute“, erweiterte Zeitachse, Konfigurieren-Dialoge | TP; S 1104-132502 | P3 | – |
| VL-81 | Terminplan | Kalender | Monat/Woche/Tag, Termine, mehrtägige Ereignisse, Import aus iCalendar | TP; S 1104-132502 | P3 | – |
| VL-82 | Terminplan | Gantt-Diagramm, PERT-Diagramm | Vorgangstabelle mit Balken und Abhängigkeiten, Netzplan | SbS 1; S 1104-132502 | P3 | – |

### 5.3 Assistenten und Generatoren

| ID | Assistent | Beschreibung | Quelle | Prio | Ort | Bestand |
|---|---|---|---|---|---|---|
| AS-01 | Organigramm-Assistent | Quellen Excel/CSV/Text (Exchange entfällt); Feldzuordnung (Name, Vorgesetzter, Vorname); welche Felder auf dem Shape stehen und welche Shape-Daten werden; Fotos aus Ordner nach Namensabgleich; Seitenaufteilung (selbst oder automatisch ab einer Person) mit seitenübergreifenden Links und synchronisierten Kopien; Leer-Variante legt eine Beispiel-Tabelle an; Prüfung der Daten mit verständlichen Fehlern (fehlende Trennzeichen, doppelte IDs, Zirkelbezüge, mehrere Personen ohne Vorgesetzten). | SbS 5; TP; UG Kap. 21 | P2 | App | neu |
| AS-02 | Datenvisualisierer | Excel-/CSV-Tabelle mit festen Spalten (Schritt, Beschreibung, nächster Schritt, Bedingung, Funktion/Bahn, Phase, Shape-Typ) erzeugt ein Flussdiagramm oder funktionsübergreifendes Diagramm bzw. Organigramm; „Aktualisieren“ übernimmt Tabellenänderungen; Rückweg Diagramm → Tabelle. | A 183424, A 184532; TP | P3 | App | neu |
| AS-03 | Datenauswahl-Assistent | Benutzerdefinierter Import (DA-04): Quelle, Bereich, Spalten/Zeilen, Schlüssel. | SbS 10 | P2 | App | neu |
| AS-04 | Daten verknüpfen | Automatische Zuordnung Spalte ↔ Shape-Feld für eine Seite. | SbS 10 | P2 | App | neu |
| AS-05 | Berichtsdefinition | Vier Schritte: Shapes und Kriterien, Spalten, Titel/Zwischensummen/Sortierung/Format, Name und Speicherort (DA-09). | SbS 8 | P2 | App | neu |
| AS-06 | Pivotdiagramm | Datenquelle → Baum mit Aggregaten (Summe, Anzahl), Aufschlüsseln nach Spalten, Knoten-Datengrafiken. | S 1104-131939; W | P3 | App | neu |
| AS-07 | Gantt-Optionen | Zeitraum, Zeiteinheiten, Spalten, Format; Import/Export Vorgänge als CSV/Project-XML. | W | P3 | App | neu |
| AS-08 | Zeitachse/Kalender konfigurieren | Dialoge für Zeitraum, Skala, Datumsformat, Monat, Wochenbeginn; Import aus iCalendar statt Outlook. | TP | P3 | App | neu |
| AS-09 | Brainstorming-Export/-Import | Themenbaum als XML, Text-Gliederung (Word-kompatibel), CSV/Excel. | TP | P3 | App | neu |
| AS-10 | Raumplan-Start | Raumplan aus CAD-Grundriss oder Raumliste anlegen, Personen/Inventar zuordnen. | W | P3 | App | neu |
| AS-11 | Shapes nummerieren, Array, Maßlinien | Werkzeuge statt Visio-Add-ons: Shapes in Reihenfolge nummerieren (Präfix, Start, Schritt), Shapes als Raster vervielfältigen, Bemaßung zwischen zwei Punkten. | A 185746; TP | P2 | App | neu |
| AS-12 | Mehrere Shapes hinzufügen | Organigramm: Anzahl und Typ auf einmal. | SbS 5; TP | P3 | App | neu |
| AS-13 | Vergleichen und Synchronisieren | Organigramm-Versionen vergleichen (Unterschiedsbericht als HTML), synchronisierte Kopien. | TP | P3 | App | neu |
| AS-14 | Fremdformat-Importassistent (eigen) | draw.io, GraphML, Mermaid, PlantUML, BPMN-XML in Shapes umsetzen (IM-07). | eigener Vorschlag | P3 | App+Lib-io | neu |
| AS-15 | Seitenlayout-Assistent | Seitengröße, Ausrichtung, Maßstab, Rahmen und Schriftfeld (Titelblock) als Hintergrundseite in einem Schritt. | UG Kap. 3, Anhang | P3 | App | neu |
| AS-16 | Büroplan-Assistent | Maßstab, Seitengröße und Grundwände eines Büroplans. | UG Anhang | P3 | App | neu |
| AS-17 | Verhalten-Assistent (ohne Formeln) | Häufiges Shape-Verhalten per Dialog statt ShapeSheet einstellen: Text skaliert mit dem Shape, Text bleibt aufrecht, Sperren, Verbinder-Verhalten. | UG Kap. 8, 19, Anhang | P2 | App | neu |
| AS-18 | Diagramm-Shape-Assistent | Aus einem beliebigen Shape ein Shape bauen, das sich beim Strecken vervielfacht (Personen, Papierstapel) oder in zwei/drei Teilen dehnt (Bleistift, wachsende Blume); „Konfigurieren“ im Kontextmenü. | UG Kap. 24, Anhang | P3 | App | neu |
| AS-19 | Region bilden | Ausgewählte Karten-Shapes zu einer Region verschmelzen. | UG Anhang | P3 | App | neu |
| AS-20 | Verbundene Zeichnung aus Text | IM-08 als Assistent mit Vorschau und Master-Zuordnung. | UG Kap. 21 | P2 | App | neu |

<a id="weggelassen"></a>
## 6. Bewusst weggelassen

Weggelassen heißt: JFDiagrammer bietet die Funktion nicht an. Daten, die zu diesen Funktionen
gehören, bleiben in Dateien trotzdem **unverändert erhalten** (DF-03), damit ein Dokument nach
dem Speichern in Visio weiter funktioniert.

| ID | Funktion | Begründung | Umgang mit vorhandenen Daten | Quelle |
|---|---|---|---|---|
| NO-01 | VBA: Visual-Basic-Editor, Makros ausführen/aufzeichnen, Makro-Sicherheit, `RUNMACRO`/`CALLTHIS` | Entscheidung des Nutzers; Makros sind ein Sicherheitsrisiko und an die Microsoft-Laufzeit gebunden | VBA-Projekt in `.vsdm`/`.vssm`/`.vstm` unverändert durchreichen, nie ausführen, beim Öffnen darauf hinweisen; Automatisierung später über JFScript (UI-21, F7) | Entscheidung; SbS A; A 183847 |
| NO-02 | COM-Add-ins, VSL-Add-ons, „Lösungen“ von Drittanbietern, `RUNADDON` für fremde Add-ons | Windows-/COM-gebunden, fremder Code | Lösungs-XML und Formeln erhalten; Aufruf meldet „nicht verfügbar“ | SbS A; A 183847 |
| NO-03 | ActiveX-Steuerelemente auf der Seite (Entwurfsmodus, Steuerelemente einfügen) | nur Windows/COM, ausführbarer Code | als Fremdobjekt mit Vorschaubild erhalten | SbS A; A 183847 |
| NO-04 | OLE: eingebettete Office-Objekte bearbeiten (Excel-Tabelle, Word-Dokument) | erfordert die fremden Programme über COM | Objekt erhalten, Vorschaubild zeigen; „Öffnen mit …“ der eingebetteten Datei als P3-Ersatz | SbS 8; A 183603 |
| NO-05 | SharePoint-Workflows, Export nach SharePoint Designer, Power Automate | Clouddienst-/Serverbindung an Microsoft | Workflow-Shapes als normale Shapes zeichnen; Exportbefehle fehlen | SbS 4, 15; A 183751 |
| NO-06 | Visio Services, Visio-Webzeichnung (`.vdw`), Visio für das Web, Power-BI-Visual | Microsoft-Serverprodukte | – | SbS 4, 15; A 183751 |
| NO-07 | Mitbearbeitung in Echtzeit (Co-Authoring über OneDrive/SharePoint), Freigeben per Microsoft-Link, Teams-Integration | Microsoft-Clouddienste; eigene Zusammenarbeit wäre ein eigenes Projekt | `NoCoauth` und Markup-Zellen erhalten; Kommentare gehen lokal (UI-Abschnitt 3) | SbS 1, 15; A 184030 |
| NO-08 | Microsoft-Konto, Office-Designs der Titelleiste, Feedback an Microsoft, Office-Updates, „Smiley“-Knöpfe | Microsoft-Dienste | eigene Konto-freie Startseite, Info-Seite, Feedback per Link | A 183424 |
| NO-09 | Information Rights Management (IRM), Vertraulichkeitsbezeichnungen (Microsoft Purview) | Microsoft-Serverdienst | Datei mit IRM-Schutz ablehnen mit klarer Meldung | W |
| NO-10 | Datenquellen Microsoft Exchange (Organigramm), Outlook-Kalender, Microsoft Dataverse, Azure AD/Entra-Organigramm, SharePoint-Listen | Microsoft-Dienste mit Anmeldung | Ersatz: Excel/CSV-Import, iCalendar-Import (`.ics`) für den Kalender | SbS 5, 10; TP |
| NO-11 | Microsoft-Project-Austausch im Binärformat (`.mpp`) | kein offenes Format | Ersatz: Project-XML und CSV (P3) | W |
| NO-12 | Binäre Visio-Formate schreiben (`.vsd`, `.vss`, `.vst`) | Entscheidung „nur Import“; keine offene Spezifikation | nur lesen (DF-30 … DF-32) | Entscheidung |
| NO-13 | Visio-eigene Inhalte: mitgelieferte Schablonen, Shape-Grafiken, Designs/Themes, Hintergründe, Rahmen und Titel, Beispielzeichnungen, Vorlagen-Vorschaubilder | urheberrechtlich geschützt (Entscheidung) | Shapes aus geöffneten Dateien werden als deren Master angezeigt und erhalten; eigene Gestaltung neu | Entscheidung |
| NO-14 | AWS-Icons verändern (umfärben, beschneiden, zusammensetzen) | Lizenz CC BY-ND 2.0 erlaubt keine Bearbeitung | Icons nur unverändert skalieren; Namensnennung in Info-Dialog und Schablone | Entscheidung |
| NO-15 | Azure-Icons mitliefern | Microsoft-Nutzungsbedingungen; Nutzer lädt selbst | Download-Knopf und Umwandlung in eine lokale Schablone (SH-Abschnitt 4) | Entscheidung |
| NO-16 | Microsoft Search/„Was möchten Sie tun?“ mit Online-Hilfe, Smart Lookup/Intelligente Suche (Bing) | Online-Dienste | eigene Befehlssuche offline (UI-Abschnitt 3) | SbS 1; A 185646 |
| NO-17 | Einfügen von Online-Bildern (Bing-Bildsuche), Stockbilder, Symbole aus dem Office-Dienst | Online-Dienst, Lizenzlage unklar | lokale Bilder einfügen (IM-01) | SbS 1, 3; A 184758 |
| NO-18 | Freihand mit Stift über Windows Ink als eigenes Register „Zeichnen“ in voller Breite | plattformgebunden; nur einfacher Freihand-Stift in P3 | Freihand-Shapes erhalten und anzeigen | A 183624 |
| NO-19 | Ein-/Auschecken, Versionsverlauf, „Personen einladen“, „Link senden“ (SharePoint/OneDrive) | Microsoft-Clouddienste | Dateien bleiben lokal; Versionsstände über das Dateisystem/Git des Nutzers | SbS 1 |
| NO-20 | Online-Vorlagensuche | Office-Onlinekatalog | eigene Vorlagen und Vorlagenordner (VL-03) | SbS 1 |
| NO-21 | Web-Export als XAML (Silverlight) oder VML, Visio Viewer (Browser-Add-on), `.vdw`/`.vwi` | veraltete bzw. Microsoft-gebundene Techniken, schon in Visio 2016 überholt | Webseite als HTML + SVG (EX-09) | SbS 12, Gl |
| NO-22 | Berichte als eingebettetes Excel-Objekt; Organigramm-Assistent, der Excel/Notepad fernsteuert | OLE- und Programmsteuerung fremder Programme | Tabellen-Shape, CSV/XLSX direkt schreiben (DA-09); CSV-Beispieldatei anlegen und öffnen lassen (AS-01) | SbS 5, 8 |
| NO-23 | Netzwerk automatisch erkennen und zeichnen | gab es nur in sehr alten Versionen; Netzwerkzugriff | stattdessen Import von JFNetworker-Projekten (IM-11, P3, F9) | SbS 7 |
| NO-24 | Office-Hintergrund, Office-Design, Konto-Seite | Office-spezifisch | nur Hell/Dunkel des NetLab-Styles (UI-02) | SbS 1; A 184427 |
| NO-25 | Mitbearbeitungs-Marker, „Updates verfügbar“, Kontaktkarten, Kommentieren im Browser | an Microsoft-Dienste gebunden | lokale Kommentare mit Verlauf und Filter (HL-04) | SbS 15 |
| NO-26 | Alte Fremdformate (ABC FlowCharter, CorelFLOW, CorelDRAW, Micrografx Designer, Macintosh PICT, IGES, PCX, AutoCAD DWF) | ausgestorben bzw. ohne offene Spezifikation | ältere Zeichnungen über Visio oder SVG/PDF umwandeln | UG Kap. 9 |
| NO-27 | Lotus Notes, Laufzettel per Mail (Office 97), Exchange-Ordner, Outlook-Journal, Microsoft Binder, Visio im Internet Explorer (ActiveX-Dokument), serverseitige Image Maps | ausgestorbene Office- und Browser-Integration | – | UG Kap. 11–13, 23 |
| NO-28 | Arbeitsbereich als eigene `.vsw`-Datei | nur Visio 5; der Arbeitsbereich steckt heute in der Zeichnung (DF-10) | – | UG Kap. 2 |
| NO-29 | AutoCAD DWG lesen und schreiben | kein offen dokumentiertes Format (F5) | DXF verwenden (EX-10, IM-04) | SbS 12; Entscheidung F5 |

<a id="icons"></a>
## 7. Anhang: Ribbon-Icons für die Neuzeichnung

Alle Icons werden **neu im NetLab-Stil** gezeichnet (eckig, klare Linien, NetLab-Farben); das
Visio-Original dient nur als Hinweis, *welche* Befehle ein Icon brauchen. Die Spalte „Motiv
(Vorschlag)“ nennt eine allgemein übliche Bildidee, keine Vorlage zum Nachzeichnen. Größe:
**g** = groß (32 px, Text darunter), **k** = klein (16 px). Kataloge (Designs, Varianten,
Schnellformatvorlagen, Container, Callouts, Datengrafiken, Stifte, Layouts) bekommen **keine**
gezeichneten Icons, sondern Vorschauen, die aus echten Shapes gerendert werden. Shape-Symbole
der Schablonen sind Abschnitt 4. Vorhandene Bausteine: `lib/ui/keys.rs` hat bereits viele
`IC_*`/`IC_OFFICE_*`-Kennungen (Fluent System Icons in `style_assets`), die als Platzhalter
dienen können, bis die NetLab-Fassungen fertig sind.

Summe: rund 175 Icons (davon etwa 25 nur für entfallende Befehle, die nicht gebraucht werden).

### 7.1 Fenster, Backstage, Statusleiste, Seitenleiste

| Ort | Icon | Größe | Motiv (Vorschlag) | Prio |
|---|---|---|---|---|
| Schnellzugriff | Speichern | k | Diskette | P1 |
| Schnellzugriff | Rückgängig, Wiederholen | k | Bogenpfeil links, rechts | P1 |
| Schnellzugriff | Anpassen | k | Strich mit Dreieck | P2 |
| Titelleiste | Menüband-Anzeigeoptionen | k | Fenster mit Pfeil nach oben | P2 |
| Registerzeile | Befehlssuche | k | Lupe | P2 |
| Registerzeile | Kommentare | k | Sprechblase | P2 |
| Backstage | Zurück | g | Kreis mit Pfeil nach links | P1 |
| Backstage | Startseite, Neu, Öffnen | g | Haus, Blatt mit Eselsohr, offener Ordner | P1 |
| Backstage | Informationen, Speichern, Speichern unter, Drucken, Exportieren, Schließen, Optionen, Info | g | Info-i, Diskette, Diskette mit Stift, Drucker, Blatt mit Pfeil, X, Zahnrad, App-Logo | P1 |
| Backstage | Datei-Symbol in Listen | g | Blatt mit kleinem Diagramm und JFDiagrammer-Logo | P1 |
| Backstage | Kopie senden (Zeichnung / PDF) | g | Umschlag mit Büroklammer, PDF-Blatt | P3 |
| Statusleiste | Präsentationsmodus | k | Leinwand auf Ständer | P2 |
| Statusleiste | Zoom −, Zoom + | k | Minus, Plus | P1 |
| Statusleiste | An Fenster anpassen | k | Rahmen mit vier Pfeilen nach außen | P1 |
| Statusleiste | Fenster wechseln | k | zwei überlappende Fenster | P2 |
| Seitenregister | Seite einfügen, Alle Seiten | k | Plus im Kreis, Dreieck nach oben | P1 |
| Seitenleiste | Einklappen, Suche, Schablonen-Menü, Weitere Shapes | k | Winkel nach links, Lupe, 2×2-Raster, Pfeil nach rechts | P1 |
| Gruppen | Dialogstarter | k | Ecke mit Pfeil nach rechts unten | P1 |
| Menüband | Reduzieren | k | Winkel nach oben | P2 |

### 7.2 Start

| Gruppe | Icon | Größe | Motiv (Vorschlag) | Prio |
|---|---|---|---|---|
| Zwischenablage | Einfügen | g | Klemmbrett mit Blatt | P1 |
| Zwischenablage | Inhalte einfügen, Als Link einfügen, Duplizieren | k | Klemmbrett mit Liste, Klemmbrett mit Kette, zwei versetzte Kästen | P1/P2 |
| Zwischenablage | Ausschneiden, Kopieren | k | Schere, zwei Blätter | P1 |
| Zwischenablage | Format übertragen | k | Pinsel | P1 |
| Schriftart | Schrift vergrößern, verkleinern | k | A mit Pfeil hoch/runter | P2 |
| Schriftart | Fett, Kursiv, Unterstrichen, Durchgestrichen | k | F, K, U, durchgestrichenes „ab“ | P1 |
| Schriftart | Groß-/Kleinschreibung | k | Aa | P3 |
| Schriftart | Schriftfarbe | k | A mit Farbbalken | P1 |
| Absatz | Oben, Mittig, Unten ausrichten | k | Linien oben/Mitte/unten im Kasten | P1 |
| Absatz | Links, Zentriert, Rechts, Blocksatz | k | Linienstapel | P1 |
| Absatz | Aufzählung, Einzug −/+, Text drehen | k | Punkte mit Linien, Linien mit Pfeil, A mit Bogenpfeil | P2 |
| Tools | Zeigertool | k | Mauspfeil | P1 |
| Tools | Verbinder | k | rechtwinklige Linie zwischen zwei Kästen | P1 |
| Tools | Text | k | großes A | P1 |
| Tools | Rechteck, Ellipse, Linie, Freihandform, Bogen, Bleistift | k | Rechteck, Oval, Diagonale, Wellenlinie, Viertelbogen, Stift | P1/P2 |
| Tools | Verbindungspunkt | k | kleines X | P2 |
| Tools | Textblock | k | Rahmen mit Griffen und Drehpfeil | P2 |
| Formenarten | Schnellformatvorlagen | g | Kasten mit Pinsel | P2 |
| Formenarten | Füllung, Linie, Effekte | k | Farbeimer mit Balken, Stift mit Balken, Kasten mit Schatten | P1/P2 |
| Formenarten | Stärke, Striche, Pfeile, Linienoptionen, Weitere Farben | k | Linien dick→dünn, gestrichelte Linien, Linie mit Pfeilspitzen, Linie mit Zahnrad, Farbpalette | P1 |
| Anordnen | Ausrichten | g | Kästen an einer Kante mit Pfeil | P1 |
| Anordnen | Positionieren | g | drei Kästen mit Abstandsmarken | P1 |
| Anordnen | Ausrichten links/zentriert/rechts/oben/mittig/unten | k | je Kästen an Linie | P1 |
| Anordnen | Automatischer Abstand, Auto-Ausrichten und Abstand, Abstandsoptionen | k | Kästchenraster mit Marken | P2 |
| Anordnen | Horizontal/Vertikal verteilen | k | drei Balken, gleiche Lücken | P1 |
| Anordnen | Von Seitenumbrüchen verschieben | k | Blatt mit Trennlinie und Kasten | P2 |
| Anordnen | Drehen 90° links/rechts, Spiegeln horizontal/vertikal | k | Kasten mit Bogenpfeil, Dreiecke gespiegelt | P1 |
| Anordnen | In den Vordergrund, Eine Ebene nach vorne, In den Hintergrund, Eine Ebene nach hinten | k | überlappende Kästen, vorderer/hinterer betont | P1 |
| Anordnen | Gruppieren, Gruppierung aufheben | k | gestrichelter Rahmen um Kästen, Rahmen offen | P1 |
| Bearbeiten | Shape ändern | g | Quadrat mit Pfeil zu Kreis | P2 |
| Bearbeiten | Suchen, Ersetzen | k | Lupe, „ab→c“ | P1 |
| Bearbeiten | Ebene zuweisen, Ebeneneigenschaften | k | gestapelte Flächen, mit Häkchen bzw. Zahnrad | P1 |
| Bearbeiten | Markieren, Alles, Nach Typ, Bereich, Lasso | k | Pfeil, Pfeil mit Rahmen, Liste mit Pfeil, gestricheltes Rechteck, gestricheltes Oval | P1/P2 |

### 7.3 Einfügen, Zeichnen, Entwurf

| Register / Gruppe | Icon | Größe | Motiv (Vorschlag) | Prio |
|---|---|---|---|---|
| Einfügen / Seiten | Neue Seite; Leer, Hintergrund, Duplizieren | g; k | Blatt mit Plus; Blatt, Blatt mit Fläche, zwei Blätter | P1 |
| Einfügen / Illustrationen | Bilder, Piktogramme, Diagramm, CAD-Zeichnung | g | Bild mit Bergen, einfaches Symbol, Säulen, Blatt mit Bemaßung | P1/P3 |
| Einfügen / Diagrammteile | Container, Beschriftung, Verbinder | g | Rahmen mit Kopfleiste, Sprechblase, Verbinder mit Plus | P1 |
| Einfügen / Links | Link | g | Kettenglieder | P1 |
| Einfügen / Text | Textfeld; horizontal, vertikal | g; k | A im Rahmen; liegend/stehend | P1 |
| Einfügen / Text | QuickInfo, Objekt, Feld, Symbol | g | Notizzettel, Fenster, Feld mit Klammern, Ω | P2/P3 |
| Zeichnen / Tools | Auswählen, Mit Finger zeichnen, Radierer, Lasso | g | Pfeil, Hand mit Linie, Radiergummi, gestricheltes Oval | P3 |
| Zeichnen / Stifte | Farbe, Linienstärke | k | Stift mit Balken, Linien dick→dünn | P3 |
| Zeichnen / Konvertieren | Freihand in Text, Freihand in Shapes | g | geschriebenes „a“ → gedrucktes „a“, Kritzel → Kasten | P3 |
| Entwurf / Seite einrichten | Ausrichtung; Hoch-, Querformat | g | zwei Blätter; Blatt hoch, Blatt quer | P1 |
| Entwurf / Seite einrichten | Format; Papierformat-Einträge; An Zeichnung anpassen; Weitere Formate | g; k | Blatt mit Maßpfeil; Blattumriss im Seitenverhältnis; Rahmen mit Pfeilen; Blatt mit Zahnrad | P1 |
| Entwurf / Seite einrichten | Automatisch anpassen | g | Blatt mit Pfeilen nach außen | P1 |
| Entwurf / Varianten | Farben, Effekte, Verbinder, Schriftarten, Verzierung | k | Farbfelder, Stern, Verbinderlinie, Aa, Ornamentecke | P2 |
| Entwurf / Hintergründe | Hintergründe, Rahmen und Titel, Hintergrundfarbe | g; k | Blatt mit Fläche, Blatt mit Rand und Titelfeld; Farbeimer | P2 |
| Entwurf / Layout | Seite neu layouten; Layout-Einträge (Fluss in 4 Richtungen, Hierarchie, kompakt, radial, kreisförmig) | g; k | Kastenhierarchie; je kleine Schemazeichnung | P2 |
| Entwurf / Layout | Verbinder; rechtwinklig, gerade, gebogen, Sprünge | g; k | Verbinder zwischen Kästen; je Linienform, Linie mit Bogen | P1 |

### 7.4 Daten, Prozess, Überprüfen, Ansicht, Entwicklertools, Hilfe

| Register / Gruppe | Icon | Größe | Motiv (Vorschlag) | Prio |
|---|---|---|---|---|
| Daten / Externe Daten | Schnellimport, Benutzerdefinierter Import | g | Tabelle mit Pfeil zur Raute, Zylinder mit Pfeil zur Raute | P2/P3 |
| Daten / Externe Daten | Alle aktualisieren, Daten aktualisieren | g; k | Tabelle mit Kreispfeil | P2 |
| Daten / Datengrafiken | Position, Konfigurieren, Automatischer Abstand | k | Kasten mit markierter Ecke, Zahnrad mit Balken, Kästchenraster | P2 |
| Daten / Daten anzeigen | Legende einfügen; horizontal, vertikal | g | Tabelle mit Farbfeldern; quer/hoch | P2 |
| Daten / Einblenden | Shape-Daten, Externe Daten, Datengrafikfelder | k | Liste mit Feldern, Tabelle, Liste mit Balken | P1/P2 |
| Daten / Erweitert | Daten verknüpfen, Erweiterte Datengrafiken | k | Zylinder mit Kette, Tacho | P2 |
| Prozess / Teilprozess | Neu erstellen, Mit vorhandenem verknüpfen, Aus Auswahl erstellen | g | Prozesskasten mit Plus, mit Kette, gestrichelt | P2/P3 |
| Prozess / Überprüfung | Diagramm überprüfen, Zu überprüfende Regeln, Regeln importieren, Problem ignorieren, Problemfenster | g; k | Flussdiagramm mit Haken; Liste mit Haken; Liste mit Pfeil; Warndreieck mit X; Fenster mit Warndreieck | P2 |
| Prozess / (eigen) | Regeln bearbeiten | g | Liste mit Stift | P3 |
| Überprüfen | Rechtschreibung, Thesaurus | g | „abc“ mit Haken, aufgeschlagenes Buch | P2/P3 |
| Überprüfen | Barrierefreiheit überprüfen | g | Figur im Kreis mit Haken | P2 |
| Überprüfen | Sprache, Korrektursprache | g; k | A mit Schriftzeichen, Buch mit Globus | P2 |
| Überprüfen | Neuer Kommentar, Kommentarbereich, Tags anzeigen | g; k | Sprechblase mit Plus, Sprechblase, Sprechblase mit Auge | P2 |
| Überprüfen | Shape-Berichte, Dokument schützen | g | Blatt mit Balken, Blatt mit Schloss | P2/P3 |
| Ansicht | Präsentationsmodus | g | Leinwand | P2 |
| Ansicht | Aufgabenbereiche, Lineal und Gitternetz (DS), Ausrichten und Kleben (DS) | g; k | Fenster mit Seitenleiste; Lineal; Magnet | P1 |
| Ansicht | Zoom, An Fenster anpassen, Seitenbreite | k | Lupe, Rahmen mit Pfeilen, Doppelpfeil waagerecht | P1 |
| Ansicht | Neues Fenster, Alle anordnen, Überlappend, Fenster wechseln | g | Fenster mit Plus, zwei Fenster nebeneinander, gestaffelt, Fenster mit Kreispfeil | P2/P3 |
| Ansicht | Werkzeuge (Ersatz für Add-Ons): Array, Nummerieren, Maßlinie | g; k | Kästchenraster, Kästen mit 1-2-3, Maßpfeil | P2/P3 |
| Entwicklertools | ShapeSheet anzeigen (Shape, Seite, Dokument) | g; k | Tabellengitter | P2 |
| Entwicklertools | Vorgänge (Vereinigen, Kombinieren, Fragmentieren, Schneiden, Subtrahieren, Verbinden, Trimmen, Versatz, Kurve anpassen) | k | je zwei Kreise in der jeweiligen Mengenoperation | P2 |
| Entwicklertools | Shape-Name, Verhalten, Schutz | k | Raute mit Etikett, Raute mit Zahnrad, Raute mit Schloss | P2 |
| Entwicklertools | Neue Schablone (metrisch / US) | g | 2×2-Raster mit Plus | P2 |
| Entwicklertools | Dokumentschablone, Zeichnungsexplorer, Masterexplorer | k | Raster, Baum, Baum mit Raute | P2/P3 |
| Entwicklertools | Ereignisüberwachung, Beständige Ereignisse, ShapeSheet drucken | k | Liste mit Puls, Blatt mit Blitz, Tabelle mit Drucker | P3 |
| Hilfe | Hilfe, Schulung, Feedback | g | Fragezeichen im Kreis, Doktorhut, Sprechblase mit Herz | P2/P3 |
| entfallen | Visual Basic, Makros, Makro aufzeichnen/anhalten, Makrosicherheit, COM-Add-Ins, Steuerelemente (12 ActiveX), Entwurfsmodus, Eigenschaften, Code anzeigen, SharePoint-Workflow (3), Übersetzen, Recherchieren, Support | – | **nicht zeichnen** | – |

### 7.5 Kontextregister

| Register | Icons | Prio |
|---|---|---|
| Container-Tools | Ränder, An Inhalt anpassen, Automatische Größe, Kopfzeilenstil, Container sperren, Inhalt auswählen, Container auflösen | P1 |
| Bildtools | Helligkeit, Kontrast, Automatischer Ausgleich, Komprimieren, Zuschneiden, Bildrahmen | P2 |
| Funktionsübergreifend | Swimlane, Trennlinie (Phase), Ausrichtung, Richtung, Ränder, Bahnbeschriftungen drehen, Titelleiste, Trennlinien ein/aus | P1 |
| Organigramm | Neu layouten, An Seite anpassen, Abstand +/−, Verschieben links/rechts/hoch/runter, Untergeordnete ein-/ausblenden, Höhe/Breite +/−, Positionstyp, Bild einfügen/mehrere/ändern/löschen/anzeigen, Importieren, Exportieren, Vergleichen, Synchronisieren | P2 |
| Brainstorming | Hauptthema, Unterthema, Gleichrangiges Thema, Mehrere Unterthemen, Thema ändern, Layout, Diagrammformat, Gliederungsfenster, Daten exportieren, Daten importieren | P3 |
| Zeitachse / Kalender | Konfigurieren, Meilenstein, Intervall, Erweitern, Synchronisieren; Monat, Termin, Termine importieren | P3 |
| Schablone bearbeiten | Neuer Master, Master-Eigenschaften, Master bearbeiten, Symbol bearbeiten, Schablone speichern | P2 |

<a id="fragen"></a>
## 8. Entscheidungen zu den Planungsfragen F1–F14

Am 2026-11-14 hat der Nutzer alle Vorschläge zu den Planungsfragen aus Phase 1 übernommen
(„Alle übernehmen“). Die Spalte „Umgesetzt in“ nennt die Stellen, die danach angepasst wurden.
Nachtrag desselben Tages: fünf Oberflächensprachen (3.17) und Testmaterial zu F4. Offen sind noch Testdateien für Version 6 und für `.vsd` (F4), die Bestätigung der abgeleiteten Tastenbelegung je Sprache (F10, LG-08) und der deutsche Begriff für „AutoVerbinden“ (Glossar in 3.17).

| ID | Frage | Entscheidung (2026-11-14) | Umgesetzt in |
|---|---|---|---|
| F1 | Eine neue Crate oder zwei? | Zwei Crates nach dem Muster `doc_core`/`doc_io`: `diagram_core` (Modell, ShapeSheet, Formeln, Geometrie, Routing, Layout) und `diagram_io` (VSDX/VDX/VSX/VSS, Importe, Exporte). | 0.1, 0.3; Spalte „Ort“ (`Lib-core`, `Lib-io`) |
| F2 | Wie weit reicht „JFNetworker-Stil“ bei genormten Symbolen? | Normsymbole behalten ihre Normform; Strichstärken, Farben, Schriften und Schatten kommen aus dem NetLab-Design. Gestaltete Grafiken (Geräte, Möbel, Personen) werden ganz im JFNetworker-Stil gezeichnet. | 4 (Kennzeichnung), SH-G01 |
| F3 | Welche AWS-Icon-Ausgabe und in welcher Form? | Die aktuelle offizielle Ausgabe als SVG, unverändert, je AWS-Kategorie eine Schablone. Namensnennung im Info-Dialog und in den Schablonen-Eigenschaften. Die Icons sind von Designs ausgenommen. | SH-G06, NO-14 |
| F4 | Lohnt der Import von binären `.vss`/`.vsd`? | `.vss` mit P2, aber erst nach allen XML-Formaten; `.vsd` mit P3. Nachtrag: Testmaterial für Version 11 liegt vor (vier `.vss` in `context/files/SWDB_DE/`, nur Schablonen, nur lokal, nie ins Repo; Tests werden ohne die Dateien übersprungen). **Noch offen:** Version 6 (Visio 2000/2002) und binäre `.vsd`-Zeichnungen. | DF-30, DF-31, DF-34, SH-279 bis SH-282 |
| F5 | DWG oder nur DXF? | Nur DXF (Import und Export); DWG entfällt. | EX-10, IM-04, NO-29 |
| F6 | Welche Datenquellen in P2? | CSV und `.xlsx` in P2; dafür ein schlanker XLSX-Leser in JFLib. ODBC/SQL erst in P3. | IM-05, IM-10, DA-04, RD-02 |
| F7 | Ersatz für VBA-Automatisierung? | Später eine Skript- und Plugin-Schnittstelle über JFScript mit dem Ereignismodell aus RX-10; nicht in Phase 2. | UI-21, NO-01 |
| F8 | Welche Begriffe in der Oberfläche? | Durchgehend „Ebene“ (nicht „Layer“), „Beschriftung“ für Callouts, „Legende“ nur für Datenlegenden, „Seite“ statt „Zeichenblatt“. Befehlsnamen sonst wie in Visio. Nachtrag: gilt sinngemäß für alle fünf Sprachen. | Abschnitte 2.9, 3, 7 (Visio-Name jeweils in Klammern), Glossar in 3.17 |
| F9 | Wie eng hängt JFDiagrammer an JFNetworker? | Gemeinsamer Symbolsatz für Netzwerkgeräte (eine Quelle, zwei Apps); Import von JFNetworker-Projekten mit P3. | SH-G09, IM-11, NO-23 |
| F10 | Welche Tastenbelegung? | Deutsche Office-Konvention für Textformate; „In den Vordergrund“/„In den Hintergrund“ und „Kapitälchen“ auf freie Tasten; alles anpassbar. Nachtrag, **abgeleitet, vom Nutzer noch zu bestätigen:** In den anderen Sprachen folgt die Standardbelegung der jeweiligen Office-Konvention (Fett: EN Ctrl+B, ES Ctrl+N, IT/FR Ctrl+G). | 3.16, KB-07, KB-11, LG-08 |
| F11 | Welche Vorlagen kommen in P1? | Leere Zeichnung, Standarddiagramm, Standardflussdiagramm, funktionsübergreifendes Flussdiagramm, Standard- und detailliertes Netzwerk und wegen JFNetworker zusätzlich das Rack-Diagramm (samt Rack-Schablonen). Die AWS-Architektur kommt früh in P2. Grundrisse und Konstruktion folgen später. | VL-52, VL-54, SH-145 bis SH-147 |
| F12 | Mehr Screenshots? | Nein. Für das Verhalten reichen die Bücher; Screenshots wären nur für einen pixelnahen Nachbau nötig. | – |
| F13 | Visio-eigene Begriffe? | Deutsche beschreibende Begriffe („AutoVerbinden“, „Schnell-Shapes“); englische Visio-Begriffe in der App meiden. Nachtrag: je Sprache eigene beschreibende Begriffe; SmartShapes, AutoConnect, Quick Shapes und ShapeSheet in keiner Sprache. „AutoVerbinden“ ist zugleich Visios deutsche Bezeichnung (Rückfrage im Glossar). | Abschnitt 3, Glossar in 3.17 |
| F14 | DIN EN/IEC oder US/ANSI? | Normschablonen nach DIN EN/IEC; eine ANSI-Fassung höchstens später. | 4 (Kennzeichnung), 4.13, VL-40 |
