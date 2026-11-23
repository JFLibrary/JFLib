# JFDiagrammer – Stand der Features

Stand 2026-11-24, Etappe JD15 (PLAN.md › „JD15: Bestandsaufnahme und P1-Abnahme“), mit JD16 und JD18. Diese Datei hält für
jede P1- und P2-Zeile aus [FEATURES.md](FEATURES.md) fest, wie weit sie in JFDiagrammer ist. Geprüft wurde
in der laufenden App (Skripte unter `tests/`, Screenshots), nicht nur im Code; die Schablonen aus
FEATURES 4.2 stehen mit einer Zeile je Schablone da.

**Nachtragen (JD16–JD34):** Wer eine Zeile weiterbringt, ändert hier Stand, Beleg und „Fehlt“ in derselben
Commit-Folge und lässt dann `python apps/JFDiagrammer/check_status.py --write` laufen. Das Skript prüft die
Datei gegen FEATURES.md (jede P1- und P2-Zeile genau einmal, Prio wie dort, Stand, Beleg bei fertig und
teilweise, „Fehlt“ bei teilweise, genannte Skripte vorhanden) und schreibt die Zusammenfassung neu; ohne
`--write` meldet es nur. Neue FEATURES-Zeilen kommen in derselben Form dazu, erledigte große Lücken fallen
aus der Liste unten.

Leseschlüssel:

- **Stand:** *fertig* – im Umfang der Beschreibung da (bei großen P1-Zeilen der Kern, den PLAN für P1
  vorsah); *teilweise* – „Fehlt“ sagt, was fehlt; *fehlt* – nichts davon bedienbar (ein grauer Befehl
  zählt als fehlt).
- **Beleg:** ein vorhandener Check, der die Zeile prüft: `tests/x.uis` (App-Skript, läuft in
  `python build_rs.py test JFDiagrammer` unter Windows und Linux), `diagramiotest GRUPPE`
  (`diagramiotest --check --only GRUPPE`, Teil derselben Suite), ein Unit-Test
  (`diagram_core modul::tests::name`, `cargo test -p jflib-diagram-core --lib`).
  Kürzel: `dit` und `dio` = diagramiotest-Gruppe, `core` und `io` = Unit-Tests von `diagram_core` und
  `diagram_io`; Skriptnamen ohne Pfad liegen in `tests/`. „Befehl …“ heißt: in der App bedienbar, aber noch
  ohne eigenen Check (nur bei teilweise); „(Probe)“ ist eine Beobachtung aus JD15 zusätzlich zum Check.
- Die P2-Zeilen gehören laut PLAN in Phase 2b (JD16–JD25) und Phase 3 (JD26–JD34). JD16 (ältere XML-Formate,
  Makro-Teile, Dokumenteigenschaften, Fensterzustand) und JD18 (Datenquellen und Verknüpfung) sind eingetragen.

## Zusammenfassung

<!-- summary:begin (check_status.py --write) -->
| Abschnitt | P1 fertig | P1 teilweise | P1 fehlt | P2 fertig | P2 teilweise | P2 fehlt |
|---|---:|---:|---:|---:|---:|---:|
| 1.1 Visio-Formate ab 2013 (OPC/XML, „VSDX-Familie“) | 5 | 1 | 0 | 4 | 3 | 0 |
| 1.2 Ältere XML-Formate 2003–2010 | 0 | 0 | 0 | 2 | 0 | 0 |
| 1.3 Binärformate (nur Import) | 0 | 0 | 0 | 0 | 0 | 2 |
| 1.4 Export („Speichern unter“ und „Exportieren“) | 3 | 3 | 0 | 0 | 1 | 4 |
| 1.5 Import | 0 | 2 | 0 | 3 | 0 | 1 |
| 2.1 Dokument | 5 | 1 | 0 | 0 | 0 | 4 |
| 2.2 Seiten | 6 | 2 | 0 | 0 | 2 | 4 |
| 2.3 Ebenen | 4 | 0 | 0 | 1 | 0 | 0 |
| 2.4 Master, Schablonen und Formatvorlagen | 3 | 2 | 1 | 1 | 1 | 2 |
| 2.5 Shapes, Geometrie und Gruppen | 5 | 6 | 0 | 1 | 0 | 5 |
| 2.6 Format: Linie, Füllung, Schatten, Effekte | 3 | 2 | 0 | 0 | 1 | 1 |
| 2.7 Text und Felder | 6 | 2 | 0 | 0 | 0 | 2 |
| 2.8 Verbinder, Kleben, Routing und Layout | 4 | 6 | 0 | 0 | 1 | 6 |
| 2.9 Container, Listen, Swimlanes, Beschriftungen, Legenden | 4 | 1 | 0 | 1 | 0 | 2 |
| 2.10 Daten | 2 | 0 | 0 | 1 | 1 | 6 |
| 2.11 Hyperlinks, QuickInfos und Kommentare | 1 | 0 | 0 | 0 | 0 | 4 |
| 2.12 Designs (Themes), Varianten und Schnellformatvorlagen | 4 | 0 | 0 | 1 | 0 | 1 |
| 2.13 ShapeSheet: Abschnitte | 11 | 12 | 0 | 6 | 9 | 1 |
| 2.14 Formel-Engine und Funktionen | 5 | 4 | 0 | 2 | 2 | 1 |
| 3.1 Fenster, Start und Backstage | 7 | 7 | 0 | 0 | 1 | 3 |
| 3.2 Titelleiste, Registerzeile und Menüband allgemein | 2 | 3 | 0 | 0 | 1 | 2 |
| 3.3 Register „Start“ | 19 | 4 | 0 | 0 | 3 | 4 |
| 3.4 Register „Einfügen“ | 6 | 2 | 0 | 0 | 0 | 2 |
| 3.6 Register „Entwurf“ | 6 | 2 | 0 | 0 | 0 | 3 |
| 3.7 Register „Daten“ | 0 | 1 | 0 | 3 | 0 | 4 |
| 3.8 Register „Prozess“ | 0 | 0 | 0 | 0 | 0 | 5 |
| 3.9 Register „Überprüfen“, „Ansicht“, „Entwicklertools“, „Hilfe“ | 3 | 3 | 0 | 0 | 1 | 15 |
| 3.10 Kontext- und Vorlagenregister | 1 | 2 | 0 | 0 | 0 | 4 |
| 3.11 Shapes-Seitenleiste | 3 | 5 | 0 | 0 | 2 | 0 |
| 3.12 Zeichenfenster | 5 | 11 | 0 | 0 | 1 | 5 |
| 3.13 Aufgabenbereiche | 1 | 5 | 0 | 0 | 2 | 5 |
| 3.14 Statusleiste | 2 | 1 | 0 | 0 | 0 | 1 |
| 3.15 Drucken und Seite einrichten | 3 | 5 | 0 | 0 | 0 | 1 |
| 3.16 Tastenkürzel | 9 | 6 | 0 | 0 | 1 | 2 |
| 3.17 Sprachen und Regionen | 7 | 1 | 0 | 1 | 0 | 2 |
| 4.0 Regeln für alle Schablonen | 4 | 2 | 0 | 0 | 0 | 2 |
| 4.2 Übersicht aller Schablonen | 15 | 1 | 0 | 0 | 0 | 65 |
| 4.14 Vorschläge für eigene Kategorien | 0 | 0 | 0 | 0 | 0 | 6 |
| 5.1 Vorlagen-Infrastruktur | 4 | 0 | 0 | 0 | 0 | 0 |
| 5.2 Vorlagen nach Kategorie | 5 | 2 | 0 | 0 | 0 | 13 |
| 5.3 Assistenten und Generatoren | 0 | 0 | 0 | 3 | 0 | 4 |
| **zusammen** | **173** | **107** | **1** | **30** | **33** | **194** |

P2-Zeilen, die schon fertig sind (30): DF-04, DF-05, DF-07, DF-12, DF-20, DF-21, IM-05, IM-08, IM-10, LY-05, MS-07, SP-13, ST-09, DA-05, TH-06, SS-11, SS-21, SS-22, SS-31, SS-33, SS-36, FX-11, FX-12, RD-02, RD-03, RD-09, LG-05, AS-03, AS-04, AS-20.
<!-- summary:end -->

## Offene Entscheidungen des Nutzers

- **Neue Seite (RE-01, PG-02):** Plus-Knopf und Einfügen › Neue Seite fügen die Seite hinter der aktuellen ein;
  Visio 2016 hängt sie am Ende an (nur das Kontextmenü des Seitenregisters fügt dort hinter der aktuellen ein).
  Bis zur Entscheidung bleibt es hinter der aktuellen Seite (PLAN › Offene Fragen 3).
- **Begriff für „AutoVerbinden“ (CN-07, ZF-10):** Platzhalter „Verbindungspfeile“ (PLAN › Offene Fragen 2).
- **Lokalisierte Visio-Tasten (KB-*, LG-08):** it „in primo piano“ Strg+O, „ruota a sinistra“ Strg+T, fr Rechteck
  Strg+Umschalt+I sind nicht übernommen; die Tabelle hält Visios englische Tasten für Visio-eigene Befehle
  (Übergabe JD8 Rest).
- **Standardnamen „Sheet.n“ (SP-01):** Ungenannte Shapes heißen in jeder Sprache „Sheet.n“ (NameU wie Visio);
  FEATURES nennt für die deutsche Oberfläche „Blatt.n“. Eine Übersetzung der Anzeige änderte die Namen in allen
  Skripten und für Bildschirmleser.
- **Gemeinsamer Symbolsatz (SH-G09):** das eigene Repository (Vorschlag `JFLibrary/JFSymbols`) legt der Nutzer an;
  bis dahin `apps/JFDiagrammer/symbols/`.
- **Schablonen neben einer Zeichnung (DF-10):** Schablonen aus dem Fensterteil öffnen nur, wenn sie eingebaut sind
  oder in `data://shapes` liegen; Dateien neben der Zeichnung zu lesen, erlaubt `fs.user` nicht (Übergabe JD16).
- **Visio-Gegenproben offen:** JD10 (Strukturen), JD13 Netzwerk und Rack (`Documents\JFLib-jd13n-visio-check`),
  JD16 (`Documents\JFLib-jd16-visio-check`), JD15: Textfelder in den Bemaßungen (Field-Abschnitt ohne
  UICat/UICod/UIFmt, Annahme: Visio zeigt sie als eigene Formel), Platzierungsstil und -richtung (PG-11, Werte von
  PlaceFlip nach Annahme), Actions-Zeilen im Kontextmenü (SP-12).

## In JD15 geschlossen

| Bereich | IDs | Checks |
|---|---|---|
| Schablonen | SH-146 (Bemaßung horizontal/vertikal), SH-272 (Auto-Bemaßung), SH-G04 (Schnell-Shapes der Beschriftungen); Bauwerkzeug schreibt Textfelder (`[Field]`, `{fld N}`) | network_rack_dimension.uis, callouts_auto_dimension.uis; dit stencils; io tests::stencil_text_fields |
| Datei und Menüs | UI-12/KB-01 (Speichern-Rückfrage), DF-15, DM-09, UI-04, UI-05, UI-06 (Kopie), UI-08, UI-11, UI-13 (Keine QuickInfos), UI-15, UI-18 (Rückgängig-Tiefe), TX-01/KB-10 (Eingabetaste), LG-07, DM-04, SP-01 (Dialog), ST-05, AB-03 | file_*.uis, context_jd15.uis; core ops::tests |
| Zeichenfläche | RA-02, RS-31, RA-06, RA-07 (Dialog), SB-02, SB-04, ZF-06, ZF-08, KB-04, KB-06, KB-09, CN-03, SS-29, SS-39, AB-06 | canvas_*.uis |
| Format, Seite, Druck, Modell | RW-02, PG-04, PG-06/PR-07, PG-07 (ohne Tage/Wochen), PR-08, PG-11, PR-05, RS-09, RS-21, MS-10, SS-18, FX-06 (RAND), CN-16 (Shapes verbinden), SP-12/SS-31, SL-04 | page_formats.uis, actions_menu.uis, text_dialog.uis; core instance/glue/formula-Tests |
| ohne eigenen Check bisher | 8 neue Prüfskripte für Zeilen, die schon gingen | status_*.uis |

## Große P1-Lücken nach Etappe

Was in P1-Zeilen noch fehlt und mehr als eine kleine Änderung ist. Aufwand S = eine Session, M = zwei bis drei,
L = mehr. P2-Teile von P1-Zeilen stehen bei ihrer P2-Etappe.

| Etappe | ID | Was fehlt | Aufwand |
|---|---|---|---|
| JD20 | UI-16 | Bearbeitungs- und Korrektursprachen | S |
| JD20 | ZF-18 | Alternativtext und Lesereihenfolge für Bildschirmleser (RU-03) | S |
| JD21 | SP-04, SS-30 | Steuerpunkte kleben (CanGlue: Ethernet, Ringnetz, Bus), Tip als QuickInfo | S |
| JD21 | SP-05 | Scheitelpunkte: Rahmen folgt der Geometrie, Mehrfachauswahl, Bogen-Griffe | S |
| JD21 | SP-06, SS-17 | Gruppen: Warnung, Ablegen in eine Gruppe, SelectMode 1/2, DontMoveChildren | S |
| JD21 | CN-01, CN-02 | Verbindungspunkt-Werkzeug; Kleben an Scheitelpunkt, Führungslinie, Steuerpunkt | S |
| JD21 | CN-04, CN-05, SS-19, SS-40 | Knick- und Segment-Ziehpunkte; Routing-Stile, Richtung, Durchlässigkeit je Richtung | M |
| JD21 | CN-07, SL-06, ZF-10 | Schnell-Shapes-Leiste an den Verbindungspfeilen mit Livevorschau | M |
| JD21 | CN-16 | Einfügen › Verbinder mit dem Verbinder-Master der markierten Kachel (Shapes verbinden kann es) | S |
| JD21 | ST-02 | Container-Stilkatalog (14) als Galerie, Kategorien bearbeiten | S |
| JD21 | RS-25, PR-03 | Automatischer Abstand, Auto-Ausrichten und Abstände, Seitenumbrüche meiden, Diagramm drehen | M |
| JD21 | RS-27 | Zu Gruppe hinzufügen / Aus Gruppe entfernen | S |
| JD21 | SL-03, PR-09 | Strg-Ziehen mehrerer Master, Teilen beim Ablegen auf Verbinder, Leuchten von Container/Liste; Linienübergänge-Optionen | M |
| JD22 | DM-03, RA-07 | Einrast- und Klebeziele, Stärke, Einrastwinkel, im Dokument speichern | S |
| JD22 | PG-03, ZF-14 | Seitenregister per Ziehen umordnen (`ui.tabs`) | S |
| JD22 | SP-10, ZF-04, AB-04 | Führungslinien per Klick wählen, verschieben, drehen, Mitnehmen geklebter Shapes, Führungspunkte | M |
| JD22 | FM-01, SS-03 | Ecken-Verbindungstyp, Doppel-/Dreifachlinie, Verlaufslinie, alle Strichmuster | S |
| JD22 | TX-03, ZF-07, KB-08 | Textblock-Werkzeug (mit Verbindungspunkt- und Zuschneide-Werkzeug, Strg+Umschalt+1/2/4) | M |
| JD22 | ZF-01, ZF-02, ZF-03, ZF-05 | Seitenfarbe aus dem Design, Nullpunkt verschieben, Rasterstufen und Dialog „Lineal und Gitternetz“, Abstandspfeile im dynamischen Gitter | M |
| JD22 | ZF-09 | Livedynamik aus: Umriss beim Ziehen | S |
| JD22 | AB-05 | Effekte Spiegelung, Leuchten, weiche Kanten, 3D; Schatten-Voreinstellungen | M |
| JD22 | KB-11 | doppelt unterstrichen, Großbuchstaben, Kapitälchen, hoch/tief, größer/kleiner als Befehle | S |
| JD22 | PR-02, SS-41 | Drucken: „Aktuelle Ansicht“ und „Auswahl“, Druckeinrichtung je Seite | M |
| JD22 | RE-06, RE-07 | je 8 weitere Container- und Beschriftungsstile, Katalog mit Vorschau | S |
| JD23 | MS-02, MS-06, MS-09 | Master bearbeiten, Master-Eigenschaften, Ziehen von der Seite in eine Schablone | M |
| JD23 | SS-14 | ClippingPath bei Bildern (Darstellung, PDF, SVG) | S–M |
| JD23 | SS-15 | Schutz-Dialog, Sperren durchsetzen (LockDelete, Select, Format, CustProp, Crop, Replace) | S |
| JD23 | FX-01 | lokalisierte Formelnamen (LG-10) | S |
| JD23 | KB-15, SL-05 | Tasten in einer bearbeitbaren Schablone, Dokumentschablone anzeigen | S |
| JD25 | DF-14, IM-01, SP-09 | TIFF, WebP, EMF, WMF, SVG und OLE-Vorschaubilder; Bildtools (Zuschneiden, Helligkeit, Kontrast, Gamma, Transparenz) | M |
| JD25 | EX-01, EX-04 | PDF: Auswahl, Tags, PDF/A, Dokumenteigenschaften; PNG-Optionen | M |
| JD25 | EX-14, IM-06 | eigenes Format und EMF über die Zwischenablage (Mehrformat in `plat`); SVG einfügen | M |
| JD25 | UI-01 | mehrere Zeichnungen in einer Instanz, Fenster wechseln (mit RA-08, KB-14) | L |
| JD25 | UI-03 | Startbildschirm (Begrüßung, empfohlene Vorlagen, Zuletzt verwendet, Angeheftet) | S |
| JD25 | UI-06 | Orte in Datei › Öffnen | S |
| JD25 | UI-15 | AutoWiederherstellen (DM-10) | M |
| JD25 | RS-04, SL-02 | Livevorschau der Schrift und beim Zeigen auf eine Kachel | S |
| JD25 | SL-01 | eingeklappte Seitenleiste mit Symbolen, spaltenweise Breite | S |
| JD25 | PR-01 | Druckvorschau zoomen und verschieben | S |
| JD25 | KB-16, LG-03 | Hilfe (F1), Beschreibungen in den QuickInfos aller Befehle in fünf Sprachen | M |
| JD31 | SH-272 | Benutzerdefinierte Beschriftung 1–3 (Konfigurations-Mechanik) | S |
| vor JD26 | SH-G02 | Ebenen in den mitgelieferten Mastern (Bauwerkzeug), Kategorien für alle Schablonen | S |
| Umzug | SH-G09 | Symbolsatz als eigenes Repository, Submodule in JFDiagrammer und JFNetworker | S |
| neu | TX-02, SS-07 | Textbearbeitung mit Formaten (Läufe, Tabstopps, Nummerierung, Rechts-nach-links, Blocksatz) | L |
| neu | UI-18, UI-13 | Optionen anschließen (Livedynamik, Verbinder teilen/mit löschen, Rad zoomt, Zoom zentrieren, Einheiten, Entwicklermodus …; Minisymbolleiste mit ZF-11) | M |
| neu | UI-34 | Menüband-Anzeige: reduzieren, automatisch ausblenden, Touch-Modus, QuickInfos mit Beschreibung | M |
| neu | RK-03 | CFF-Register vervollständigen (Einfügen › Seite, Ausrichtung eines vorhandenen Flussdiagramms, Richtung, Ränder) | M |
| neu | SL-01, AB-01 | Andocken oben (`lib/ui/dock.rs`) | S |
| neu | SS-06 | Schrift je Schriftsystem (asiatisch, komplex) | S |

Kleinere Reste, die in JD15 offen blieben: PG-07 Tage und Wochen (mit JD33), FX-06 RAND flüchtig, SP-12
Trennlinie (BeginGroup) im Menü, KB-06 Enter/Umschalt+Enter-Auswahl, SS-39 UIVisibility.

## 1. Dateiformate, Import und Export

### 1.1 Visio-Formate ab 2013 (OPC/XML, „VSDX-Familie“)

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| DF-01 | `.vsdx` lesen | P1 | fertig | dit samples, corpus (Z2/Z3/Fav: 0 Warnungen), viewer.uis | – |
| DF-02 | `.vsdx` schreiben | P1 | fertig | dit roundtrip (deterministisch, 2. Schreiben bytegleich, DEFLATE), edit_save.uis; Visio-Gegenproben JD5/JD7 | – |
| DF-03 | Unbekanntes erhalten | P1 | fertig | dit samples (Extras an ihrem Platz, fremde Teile/Beziehungen), dit roundtrip samples + corpus modellgleich | – |
| DF-04 | `.vsdm` | P2 | fertig | macros.uis (Hinweis, Datei › Info, Speichern als .vsdx fragt: ohne Makros / als .vsdm / Abbrechen), status_formats.uis; dit vdx (Makro-Rundlauf), dit rels, roundtrip vsdm | Visio-Gegenprobe von JD16 offen |
| DF-05 | `.vstx`/`.vstm` | P2 | fertig | templates_own.uis (Vorlage → neue Zeichnung, Als Vorlage speichern); dit vdx (.vstm: die neue Zeichnung behält die Makros), roundtrip vstx/vstm | Visio-Gegenprobe von JD16 offen |
| DF-06 | `.vssx` | P1 | fertig | stencils.uis, stencils_favorites.uis, my_shapes.uis, status_formats.uis; dit stencils, roundtrip „new stencil masters“ | – |
| DF-07 | `.vssm` | P2 | fertig | status_formats.uis (öffnet in der Seitenleiste); dit samples, roundtrip vssm, vdx (Makro-Rundlauf .vssm) | Visio-Gegenprobe von JD16 offen |
| DF-09 | Dokumenteigenschaften | P2 | teilweise | xml_formats.uis (Datei › Info zeigt und ändert den Titel, gespeichert und wieder geöffnet); dit vdx (props) | benutzerdefinierte Eigenschaften in der Oberfläche, Datum nach Region, „Geändert“/„zuletzt geändert von“ beim Speichern; Visio-Gegenprobe von JD16 offen |
| DF-10 | Fenster- und Ansichtszustand | P2 | teilweise | xml_formats.uis (Zoom aus der Datei); dit vdx (windows), roundtrip (Zeichenfenster) | „Arbeitsbereich speichern“ abschaltbar, Verbindungspunkte-Anzeige, Schablonen neben der Zeichnung öffnen (fs.user); Visio-Gegenprobe von JD16 offen |
| DF-11 | Kommentare-Teil | P2 | teilweise | `comments.xml` bleibt unverändert erhalten (dit samples/parts) | Kommentare ins Modell lesen, anzeigen, schreiben (HL-04) |
| DF-12 | Datenverbindungen-Teile | P2 | fertig | dit data (Rundlauf von connections.xml, recordsets.xml und den ADO-Datenteilen, Paket wie Visio es schreibt: Visio_RowID, RefreshConflict, AutoLinkComparison, unbekannte Attribute; Aktualisieren gespeichert; Nichtverstandenes bleibt Bytes; feindliche Teile), io data::tests; die Daten zeigt der Bereich Externe Daten ohne Dateizugriff (data_import_xlsx.uis); Visio-Gegenprobe ausstehend | – |
| DF-14 | Eingebettete Medien | P1 | teilweise | PNG/JPEG/GIF/BMP lesen, schreiben, zeichnen: pictures.uis, dit render/pictures, roundtrip „new pictures“ | TIFF, EMF, WMF, SVG und OLE-Vorschaubilder nur als Platzhalter gezeichnet (gelesen und geschrieben werden sie) |
| DF-15 | Robustheit | P1 | fertig | file_broken.uis (beschädigte Datei auf der Kommandozeile meldet sich), file_open_modes.uis (Durchsuchen: Meldung sofort); dit xml, limits, zip, refs, rels (Grenzen, ZIP64, Bomben) | – (der Detailtext aus vsdx::Error ist englisch) |

### 1.2 Ältere XML-Formate 2003–2010

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| DF-20 | `.vsx` lesen | P2 | fertig | xml_stencil.uis (.vsx in der Seitenleiste, Master ablegen); dit vdx | Visio-Gegenprobe von JD16 offen |
| DF-21 | `.vdx` lesen | P2 | fertig | xml_formats.uis (.vdx öffnen, Speichern schreibt .vsdx, wieder öffnen); dit vdx (Rundlauf, feindliche Dateien, Grenzen) | Visio-Gegenprobe von JD16 offen |

### 1.3 Binärformate (nur Import)

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| DF-30 | `.vss` importieren | P2 | fehlt | – | alles (JD17) |
| DF-34 | Lokaler Testkorpus Binärformate | P2 | fehlt | (nur der XML-Korpus: dit corpus wird übersprungen, wenn er fehlt) | Korpus-Tests für `.vss`/`.vsd` (JD17) |

### 1.4 Export („Speichern unter“ und „Exportieren“)

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| EX-01 | PDF | P1 | teilweise | export.uis; dit export/pdf (Text eingebettet, ToUnicode, Lesezeichen, Links, Hintergrundseiten, Seitenbereich) | Auswahl als Bereich, Barrierefreiheits-Tags, PDF/A, Dokumenteigenschaften außer Titel |
| EX-03 | SVG | P1 | fertig | export.uis; dit export/svg (Pfade, Text, Verläufe, Schraffuren, Transparenz, Shape-Daten als Attribute, Links) | – |
| EX-04 | PNG | P1 | teilweise | export.uis (96/150/300/eigene dpi, transparent); dit export/png | Auflösung „Quelle“, Größe wählen, Hintergrundfarbe, Farbtiefe, Interlace, Bereich = Auswahl (nur „Als Bild kopieren“ nimmt die Auswahl) |
| EX-05 | JPEG | P2 | teilweise | export.uis (Qualität); dit export/jpeg | Hintergrundfarbe, Bereich = Auswahl |
| EX-07 | EMF | P2 | fehlt | – | alles (JD25) |
| EX-09 | Webseite | P2 | fehlt | Format in der Liste grau | alles (JD25) |
| EX-11 | Als Vorlage/Schablone speichern | P1 | fertig | templates_own.uis (`.vstx`), status_formats.uis (`.vssx`), dit roundtrip „new stencil masters“, templates | – |
| EX-12 | Shape-Bericht exportieren | P2 | fehlt | – | alles (JD19) |
| EX-14 | Kopieren in die Zwischenablage | P1 | teilweise | export.uis (Seite/Auswahl als PNG+DIB oder SVG-Text), edit_group.uis (app-interne Zwischenablage) | eigenes Format über die System-Zwischenablage (zwischen Fenstern), EMF, Text für andere Programme, Bild + SVG in einem Vorgang |
| EX-15 | Drucken | P1 | fertig | print.uis (Seite einrichten, Kacheln, Vorschau) | – (Windows druckt als Raster 300 dpi; Vektor JD25) |
| EX-16 | Daten exportieren | P2 | fehlt | – | alles (JD19) |

### 1.5 Import

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| IM-01 | Bilder einfügen | P1 | teilweise | pictures.uis (PNG, BMP, GIF), dit roundtrip „new pictures“ (JPEG) | TIFF, WebP; Zuschneiden, Helligkeit/Kontrast, Transparenz |
| IM-02 | SVG einfügen | P2 | fehlt | Bild aus Datei nimmt nur png/jpg/gif/bmp | alles (JD25) |
| IM-05 | Daten importieren | P2 | fertig | data_import_xlsx.uis (Blatt, Schlüssel), data_import_csv.uis (Semikolon, Windows-1252, Dezimalkomma, ohne Kopfzeile), data_refresh_conflict.uis (erneut lesen); CSV und .xlsx in P2, ODBC/SQL grau (P3, F6); als Quelle des Organigramm-Assistenten mit JD28 (AS-01) | – |
| IM-06 | Einfügen aus der Zwischenablage | P1 | teilweise | paste_special.uis (Text → Textfeld, Bild, eigene Shapes, Inhalte einfügen mit Formatwahl, Als Link) | SVG einfügen; eigene Shapes aus einem anderen Fenster/Programmlauf (kein Systemformat) |
| IM-08 | Verbundene Zeichnung aus CSV | P2 | fertig | data_text_drawing.uis (Komma, mehrzeiliger Text, Shape-Daten, X/Y, Probleme je Zeile, Ebenen-Layout, geklebte Verbinder, Rückgängig, neue Seite), io textdraw::tests (Tab, Semikolon, sep=, Kommentar-Überschriften, Einheiten, feindliche Texte, Layout, Aufbau); Feldfolge nach UG Kap. 21, ohne Visio-Gegenprobe | – |
| IM-10 | XLSX-Leser | P2 | fertig | lib/sheet_io (`cargo test -p jflib-sheet-io --lib`: Blätter, Namen, Tabellen, Shared Strings, alle Zelltypen, Datumssysteme, strict OOXML, feindliche Mappen), codectest --sheet (Bomben, Tiefe, Grenzen mit Zeitschranke), data_import_xlsx.uis | – |

## 2. Dokumentmodell

### 2.1 Dokument

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| DM-01 | Dokument als Ganzes | P1 | fertig | core tests::pages_backgrounds_masters_styles; dit samples (EventList, Teile für Kommentare/Daten, Einstellungen, Farben, Schriften); edit_save.uis („Zeichnung2“) | – |
| DM-02 | Rückgängig/Wiederholen | P1 | fertig | edit_shapes.uis (je Geste ein Schritt), qat.uis (Name des Schritts), core tests::transaction_undo_redo_cells, merge_drag_steps, glue_tests::undo_takes_a_whole_gesture_back | – (fest 500 Stufen; die Option „Rückgängig-Stufen“ wirkt nicht) |
| DM-03 | Einstellungen im Dokument | P1 | teilweise | canvas_snap.uis (Dialog Ausrichten und Kleben: Ausrichten, Kleben, dynamisches Gitter, an Raster / Führungslinien), edit_view.uis | Speichern im Dokument (DocumentSettings Snap/Glue werden nur erhalten), übrige Einrast- und Klebeziele (Geometrie, Ziehpunkte, Scheitelpunkte), Stärke, Einrastwinkel, Schutz (DM-11) |
| DM-04 | Standard-Formatvorlagen | P1 | fertig | core ops::tests::new_shapes_take_the_documents_default_styles; file_open_modes.uis (TopPage beim Öffnen); dit roundtrip/new (Default*Style, TopPage geschrieben) | – |
| DM-05 | Schriften und Farben | P1 | fertig | dit render/text/substitution, render/fills/colour-index; Hinweisleiste „Ersetzte Schriften“ (Probe) | – |
| DM-06 | Dateigröße verringern | P2 | fehlt | – | alles (JD25) |
| DM-07 | Persönliche Informationen entfernen | P2 | fehlt | – | alles (JD25) |
| DM-09 | Schreibgeschützt öffnen | P1 | fertig | file_open_modes.uis (Kästchen in Datei › Öffnen, Titel „[Schreibgeschützt]“, Strg+S führt zu Speichern unter, die Datei wird nie geschrieben) | – |
| DM-10 | Automatisch wiederherstellen | P2 | fehlt | Option „AutoWiederherstellen“ vorhanden, ohne Wirkung | alles (JD25) |
| DM-11 | Dokumentschutz | P2 | fehlt | – | alles (JD25) |

### 2.2 Seiten

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| PG-01 | Vordergrundseiten | P1 | fertig | pages.uis, page_setup_ribbon.uis, viewer.uis (Seiten mit eigener Größe/Maßstab), designs.uis (Design je Seite), core pagesetup::tests::a_new_page_takes_the_settings_of_the_active_one | – |
| PG-02 | Hintergrundseiten | P1 | fertig | pages.uis („Hintergrund-1“, Zuweisung), viewer.uis, dit render/pages/background, refs/background-cycle | – („VHintergrund-n“ entsteht erst mit PG-09) |
| PG-03 | Seiten verwalten | P1 | teilweise | pages.uis (einfügen, duplizieren, umbenennen, löschen, Dialog neu anordnen, Zählung) | Umordnen durch Ziehen der Register (`ui.tabs` kann es nicht); „Neue Seite“ hinter der aktuellen (Nutzerentscheidung offen) |
| PG-04 | Seitengröße | P1 | fertig | page_setup_ribbon.uis, page_formats.uis (34 Formate im Dialog: ISO A/B, JIS B, Umschläge, Executive, Government, ANSI A–E, ARCH A–E) | – |
| PG-05 | Automatische Größe | P1 | fertig | page_setup_ribbon.uis; core pagesetup::tests::the_page_grows_by_tiles_to_the_right_and_up, the_page_shrinks_back_when_a_tile_is_empty | – |
| PG-06 | Zeichnungsmaßstab | P1 | fertig | print.uis, network_rack.uis (1:20), page_formats.uis (Architektur, Bauingenieurwesen); dit render/pages/scale | – |
| PG-07 | Maßeinheiten | P1 | teilweise | panes_edit.uis, page_formats.uis (km, yd, mi, Pica, Cicero, Didot im Dialog und in Größe und Position); core tests::units_codes_and_conversion | Tage und Wochen (Zeitachsen) in Oberfläche und Eingabe |
| PG-08 | Seitenfarbe und -schatten | P2 | teilweise | Zeichenfläche: Seitenschatten (jede Ansicht, z. B. smoke.uis) | Seitenfarbe (Design/Hintergrund), Schatten abschaltbar (JD22) |
| PG-09 | Hintergründe (Galerie) | P2 | fehlt | – | alles (JD22) |
| PG-10 | Rahmen und Titel | P2 | fehlt | – | alles (JD22) |
| PG-11 | Seiten-Layout und Routing | P1 | fertig | structures_layout.uis, page_formats.uis (Platzierungsstil, Richtung, Tiefe, Spiegeln) | – (PlaceFlip-Bits nach Annahme, Visio-Gegenprobe) |
| PG-12 | Seiten-Shape-Daten und -Zellen | P2 | teilweise | dit samples, roundtrip (Seitenblatt mit Property/User/Hyperlink gelesen, erhalten, Formeln) | Oberfläche (Shape-Daten/Links der Seite ohne Auswahl) |
| PG-13 | Seiten-Ansichtszustand | P2 | fehlt | ViewScale/ViewCenter bleiben nur erhalten | alles (mit DF-10, JD16) |
| PG-14 | Seitengröße am Rand ziehen | P2 | fehlt | – | alles (JD22) |

### 2.3 Ebenen

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| LY-01 | Ebenen je Seite | P1 | fertig | layers.uis; core tests::layers | – |
| LY-02 | Ebeneneigenschaften | P1 | fertig | layers.uis (Sichtbar, Drucken, Aktiv, Sperren, Einrasten, Kleben, Farbe + Transparenz, Anzahl); core tests::layers_print_and_show | – |
| LY-03 | Zuordnen und Automatik | P1 | fertig | layers.uis (Ebene zuweisen); core instance::tests::place_brings_the_masters_layers, ops::tests::paste_on_another_page_brings_the_layers | – |
| LY-04 | Ebene löschen | P1 | fertig | layers.uis (Rückfrage, Shapes anderer Ebenen bleiben) | – |
| LY-05 | Nach Ebene auswählen | P2 | fertig | select_by_type.uis (nach Ebene, „Keine Ebene“) | – |

### 2.4 Master, Schablonen und Formatvorlagen

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| MS-01 | Master | P1 | fertig | dit stencils (Namen, IDs, BaseID/UniqueID, Icon, Prompt, Hidden …), stencils.uis, dit corpus | – |
| MS-02 | Instanzen und Vererbung | P1 | teilweise | stencils_drop.uis (Master einmal im Dokument), core instance::tests::import_copies_once_and_maps_styles, tests::inheritance_master_style_default | Dokument-Master bearbeiten (wirkt auf alle Instanzen): keine Oberfläche |
| MS-04 | Schnell-Shapes | P1 | fertig | stencils.uis (oben, Trennlinie), stencils_favorites.uis (Umordnen per Ziehen) | – |
| MS-05 | Dokumentschablone | P2 | fehlt | – | alles (JD23) |
| MS-06 | Eigene Schablonen | P1 | teilweise | Neue Schablone metrisch/US, Kacheln hineinziehen, Favoriten, Meine Shapes: stencils_favorites.uis, my_shapes.uis | Master bearbeiten (Symbol, Name, Eigenschaften; „Schablone bearbeiten“, „Neues Master-Shape“ grau); Ziehen von der Zeichenfläche in eine Schablone |
| MS-07 | Metrische und US-Varianten | P2 | fertig | templates.uis (US-Einheiten, Letter), dit templates (beide Fassungen), Schablonen `_m`/`_u` | – („Weitere Shapes“ folgt der Region, nicht dem Maßsystem der Zeichnung) |
| MS-09 | Master-Eigenschaften | P1 | fehlt | – | Dialog Master-Eigenschaften (Name, QuickInfo, Symbol, Typ, Abgleich über Namen) |
| MS-10 | Instanz und Master im Detail | P1 | fertig | core instance::tests::place_adjusts_beyond_eight_times_the_scale, formula::tests::inherited_formula_runs_in_the_instance; dit stencils/network/rack/range-of-eight | – (Bogenzellen A/B werden nicht umgerechnet) |
| MS-11 | Muster als Master | P2 | fehlt | – | alles (JD23) |
| MS-08 | Formatvorlagen | P2 | teilweise | gelesen, vererbt, geschrieben (core tests::inheritance_*); Nach Typ auswählen › Formatvorlage (select_by_type.uis) | Definieren, Zuweisen mit Rückfrage, „Format des Masters verwenden“, Regel beim Einfügen (JD23) |

### 2.5 Shapes, Geometrie und Gruppen

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| SP-01 | Shape-Arten | P1 | teilweise | 2D/1D/Gruppen/Fremdobjekte/Führungslinien: edit_*.uis, pictures.uis, edit_view.uis; context_jd15.uis (Dialog Shape-Name: Name und Name universell) | Führungspunkte; Anzeige „Sheet.n“ statt „Blatt.n“ (Entscheidung offen, siehe oben) |
| SP-02 | Geometrie darstellen | P1 | fertig | dit render/geometry (alle Zeilenarten, mehrere Abschnitte mit NoFill/NoLine/NoShow), render/corpus; core geometry-Tests | – |
| SP-03 | Transformation | P1 | fertig | edit_shapes.uis, edit_golden.uis; core ops::tests::every_grip_keeps_the_opposite_edge, rotate_about_a_centre, set_pin_leaves_the_shape_in_place, lock_bits_follow_the_lock_names | – |
| SP-04 | Steuerpunkte | P1 | teilweise | status_model.uis (ziehen, Rückgängig); core ops::tests::control_points_write_cells_as_visio_does; structures_callout.uis | Steuerpunkte an anderen Shapes kleben (CanGlue; `edit_canvas` verwirft den Kleber außer bei Beschriftungen) |
| SP-05 | Scheitelpunkte bearbeiten | P1/P2 | teilweise | status_model.uis (ziehen, Strg+Klick fügt ein); core ops::tests::vertices_move_insert_delete_with_closing_rows | Width/Height/Pin folgen der Geometrie nicht; mehrere mit Umschalt; Bogen-Griffe; Segment beim Zeichnen zurücknehmen |
| SP-06 | Gruppen | P1 | teilweise | edit_group.uis (gruppieren, aufheben, Unterauswahl); core ops::tests::group_inside_a_group_and_setatref, turning_and_flipping_a_group_takes_its_members_along | Warnung bei formelgebundenen Gruppen, Ablegen in Gruppe (IsDropTarget), „Kinder zuerst“ (nur SelectMode 0 wirkt) |
| SP-07 | Stapelreihenfolge | P1 | fertig | edit_group.uis; core ops::tests::reorder_front_back_forward_backward | – |
| SP-08 | Shape-Operationen | P2 | fehlt | – | alles (JD21) |
| SP-09 | Bilder als Shapes | P1 | teilweise | pictures.uis (Bild-Shape, drehbar, gespeichert); dit render/pictures (ImgOffset/ImgWidth) | Zuschneiden, Helligkeit, Kontrast, Gamma, Transparenz; SVG als Vektor |
| SP-10 | Führungslinien und -punkte | P1 | teilweise | edit_view.uis (aus dem Lineal, Shapes rasten ein), canvas_guides.uis (Ansicht › Führungslinien aus, Nach Typ auswählen, Entf, Rückgängig) | per Klick wählen, verschieben, drehen; Kleben und Mitnehmen; Führungspunkte; per Größe und Position; Ausrichten legt Führungslinien an |
| SP-11 | Shape ändern | P2 | fehlt | – | alles (JD21) |
| SP-12 | Intelligente Shapes | P1 | fertig | actions_menu.uis (Actions-Zeilen im Kontextmenü, Formel wirkt, Rückgängig), network_rack.uis (Höheneinheiten); dit render/geometry/sections | – (Trennlinie BeginGroup fehlt im Menü) |
| SP-13 | Shape-Daten beim Ablegen abfragen | P2 | fertig | data_ask_drop.uis (Ablegen aus Favoriten, Duplizieren, Einfügen, Strg+Umschalt+Ziehen; OK schreibt, Abbrechen behält, Rückgängig) | – |
| SP-14 | Verschieben ohne Raster und Feinschritte | P1 | fertig | status_model.uis (Pfeil = Rasterschritt, Umschalt = 1 px, Alt-Ziehen frei) | – |
| SP-15 | Gruppe öffnen | P2 | fehlt | – | alles (JD21) |
| SP-16 | Ausrichtungsrahmen aktualisieren | P2 | fehlt | – | alles (JD21) |
| SP-17 | Grenzfälle Gruppen | P2 | fehlt | – | alles (JD21) |

### 2.6 Format: Linie, Füllung, Schatten, Effekte

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| FM-01 | Linie | P1 | teilweise | format.uis (Stärke, Strich), Shape formatieren (Farbe, Transparenz, Abschluss, Eckenrundung), dit render/lines | Ecken-Verbindungstyp, Doppel-/Dreifachlinie (gezeichnet einfach), Verlaufslinie, nur 8 der 23 Strichmuster angeboten |
| FM-02 | Linienenden | P1 | fertig | status_model.uis (Pfeile-Menü), Shape formatieren (45 Enden × 7 Größen), dit render/lines/arrows, arrow-sizes; core render::style::tests::every_arrow_has_a_shape | – (eigene Enden über Muster-Master = MS-11, P2) |
| FM-03 | Füllung | P1 (einfarbig), P2 | teilweise | P1-Teil fertig: format.uis (einfarbig, Transparenz, keine); dit render/fills zeichnet Verläufe und Muster | nur P2-Teil: Verläufe und Muster einstellen |
| FM-04 | Schatten | P2 | teilweise | format.uis (Schatten außen: Farbe, Transparenz, Versatz) | Art schräg, Größe, Winkel, Weichzeichnung (gezeichnet als harter Versatz), Standard-Versatz je Seite |
| FM-07 | Format übertragen | P1 | fertig | format_painter.uis | – |
| FM-08 | Farbwähler | P1 | fertig | format.uis (Designfarben mit Abstufungen, Variantenfarben, zuletzt verwendet, Weitere Farben, keine Füllung) | – |
| FM-09 | Format wiederholen | P2 | fehlt | – | alles (JD22) |

### 2.7 Text und Felder

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| TX-01 | Shape-Text | P1 | fertig | edit_text.uis (F2, Lostippen, Doppelklick, Textwerkzeug), file_options.uis (Eingabetaste beendet, Umschalt+Eingabe neue Zeile); LockTextEdit (ops.rs) | – |
| TX-02 | Formatierter Text | P1 | teilweise | dit render/text/runs, bullet; text_dialog.uis | Bearbeiten macht Läufe zu einem Format; Tabstopps, Nummerierung, Rechts-nach-links |
| TX-03 | Textblock | P1 | teilweise | text_dialog.uis (9 Ausrichtungen, Ränder, Hintergrund), text_vertical.uis, core ops::tests::text_block_frame_moves_and_sizes | Textblock auf der Zeichenfläche verschieben, Größe ändern, drehen (kein Werkzeug) |
| TX-04 | Textausrichtung beim Drehen | P1 | fertig | dit render/transform/text-flipx, text-flipy, text-angle, render/text/txtangle | – |
| TX-05 | Textfelder | P1 | fertig | edit_text.uis (Textwerkzeug), text_vertical.uis | – |
| TX-06 | Felder im Text | P1 | fertig | fields.uis, text_pages.uis, network_rack_dimension.uis; core field::tests; io tests::stencil_text_fields (Felder in Schablonen) | – |
| TX-07 | Rechtschreibprüfung | P2 | fehlt | – | alles (JD20) |
| TX-08 | Suchen und Ersetzen | P1 | fertig | find.uis, text_pages.uis | – |
| TX-09 | Schriftersatz | P1 | fertig | dit render/text/substitution; Hinweisleiste (Probe); Designschriften über THEMEVAL (core formula::tests::themed_values) | – (Auffälligkeit 5) |
| TX-10 | Tabstopps und Textlineal | P2 | fehlt | nur Standard-Tabstopps (DefaultTabStop) | alles (JD22) |

### 2.8 Verbinder, Kleben, Routing und Layout

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| CN-01 | Verbindungspunkte | P1 | teilweise | beim Verbinden sichtbar (edit_connect.uis), Ansicht › Verbindungspunkte; core glue_tests::connection_points_turn_with_the_shape | Verbindungspunkt-Werkzeug (hinzufügen, verschieben, löschen); Art innen/außen ohne Wirkung |
| CN-02 | Statisches Kleben | P1 | teilweise | dit edit (statisch an Punkten), draw_tools.uis; core glue_tests::static_glue_writes_visio_cells_and_follows | Kleben an Scheitelpunkt, Führungslinie, Steuerpunkt |
| CN-03 | Dynamisches Kleben | P1 | fertig | edit_connect.uis (ganzes Shape, grüne Enden, folgt), canvas_glue.uis (Strg schaltet statisch/dynamisch, mit dem Verbinder-Werkzeug und beim Ziehen eines Endes); core glue_tests::dynamic_glue_* | – |
| CN-04 | Dynamischer Verbinder | P1 | teilweise | edit_connect.uis, edit_golden.uis; Verbinder rechtwinklig/gerade/gebogen/zurücksetzen (core glue_tests::styles_straight_and_curved, a_fixed_route_keeps_its_corners) | Knick- und Segment-Ziehpunkte (verschieben, hinzufügen, entfernen) |
| CN-05 | Routing um Hindernisse | P1 | teilweise | edit_golden.uis, structures_layout.uis; core glue_tests::routes_never_cross_obstacles, a_hundred_obstacles, the_page_scale_and_spacing_count | Seitenrouting-Stile außer rechtwinklig/gerade (Flussdiagramm, Baum, Organigramm, Netzwerk …) samt Richtung; Durchlässigkeit je Richtung |
| CN-06 | Linienübergänge | P2 | fehlt | Befehl grau (cmds_later.rs) | alles (JD21) |
| CN-07 | AutoVerbinden | P1 | teilweise | structures_arrows.uis, structures_arrows_overlap.uis (Pfeile, Klick, Ziehen, Schalter) | Schnell-Shapes-Leiste mit Live-Vorschau |
| CN-08 | Automatisch einfügen | P2 | fehlt | Option „Verbinder teilen“ ohne Wirkung | alles (JD21) |
| CN-09 | Automatisch löschen | P2 | fehlt | Option „Verbinder löschen“ ohne Wirkung | alles (JD21) |
| CN-10 | Verbinderwerkzeug-Automatik | P2 | fehlt | – | alles (JD21) |
| CN-11 | Verbinder-Werkzeug „einmal“ | P2 | fehlt | – | alles (JD21/JD22 Minisymbolleiste) |
| CN-12 | Linien statt Verbinder | P1 | fertig | draw_tools.uis; core glue_tests::a_line_keeps_its_form, freeform_glues_and_keeps_its_form, arc_is_a_quarter_ellipse_that_glues | – |
| CN-13 | Seite neu layouten | P2 | fehlt | nur das Layout funktionsübergreifender Flussdiagramme | alles (JD21) |
| CN-14 | Auto-Ausrichten und -Abstand | P2 | teilweise | shell_commands.uis (Automatisch ausrichten) | gleichmäßiger Abstand, ganze Seite, Live-Vorschau |
| CN-15 | Verbindungsmodell | P1 | fertig | dit edit (Connect-Einträge und Formeln bleiben, Enden folgen nach dem Lesen), edit_save.uis | – |
| CN-16 | Shapes verbinden | P1 | teilweise | structures_arrows.uis (Auswahlreihenfolge, Kurvenverbinder aus der markierten Kachel) | das Werkzeug Einfügen › Verbinder nimmt weiter den masterlosen dynamischen Verbinder |
| CN-17 | Layout-Verhalten je Shape | P1 | fertig | structures_layout.uis; Router lässt ObjType 4 aus | – |

### 2.9 Container, Listen, Swimlanes, Beschriftungen, Legenden

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| ST-01 | Container | P1 | fertig | structures_container.uis; core structure::tests (membership_follows_moves, container_carries_its_members, overlapping_containers_both_take_a_shape); dit structures | – |
| ST-02 | Containerformat | P1 | teilweise | structures_container.uis, context.uis (Container-Tools: Ränder, Größe, Sperren, Inhalt, Auflösen, 4 Kopfzeilenstile) | 6 statt 14 Stile, als Menü statt Galerie; ausgeschlossene/erforderliche Kategorien ohne Oberfläche |
| ST-03 | Randmitglieder | P2 | fehlt | – | alles (JD21) |
| ST-04 | Listen | P1 | fertig | structures_list.uis; core structure::tests::lists_keep_order, structure_functions (LISTORDER) | – |
| ST-05 | Swimlanes | P1 | fertig | structures_cff.uis, context_jd15.uis (Bahn davor/danach einfügen); dit structures | – |
| ST-06 | Beschriftungen | P1 | fertig | structures_callout.uis; core structure::tests::callouts_follow_their_target; Lösen/Anheften über den Steuerpunkt (structures.rs `callout_point`) | – |
| ST-08 | Legende für Datengrafiken | P2 | fehlt | – | alles (JD19) |
| ST-09 | Zählende Legende | P2 | fertig | network_devices.uis (zählt, wächst und schrumpft) | – |

### 2.10 Daten

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| DA-01 | Shape-Daten | P1 | fertig | panes_edit.uis (Zahl, feste Liste); Bereich kennt alle 8 Typen (pane_data.rs); Werte über `Engine::user_set` | – |
| DA-02 | Shape-Daten definieren | P1 | fertig | panes_edit.uis (Shape-Daten definieren, mehrere Shapes) | – |
| DA-04 | Datenverknüpfung | P2 | teilweise | data_import_xlsx.uis (Quelle anbinden, Schlüssel, Spalten umbenennen/umtypisieren/ordnen, Quelle umbenennen und entfernen), data_drag_row.uis (Zeile auf ein Shape, auf die leere Fläche mit und ohne gewählten Master, mehrere Zeilen, Verknüpfung aufheben), data_link.uis (automatisch über Spaltenabgleich), core data::tests | Spalten ausblenden (Visios Format kennt dafür kein Feld, das wir kennen); mehrere Zeilen auf ein Shape gezogen: nur die erste |
| DA-05 | Daten aktualisieren | P2 | fertig | data_refresh_conflict.uis (alle und einzeln, Intervall, Quelle neu wählen, fehlende Zeile, eigener Wert, gelöst mit Rückgängig), core data::tests (Schlüssel, Reihenfolge, doppelte Schlüssel, überschreiben), dit data (Aktualisieren gespeichert); Visio-Gegenprobe ausstehend | – |
| DA-07 | Datengrafiken | P2 | fehlt | – | alles (JD19) |
| DA-08 | Datengrafik-Felder | P2 | fehlt | – | alles (JD19) |
| DA-09 | Shape-Berichte | P2 | fehlt | – | alles (JD19) |
| DA-10 | Diagramm überprüfen | P2 (Prüfen) | fehlt | Vorlagen bringen Regelsätze mit (dit templates), Prüfen gibt es nicht | alles (JD24) |
| DA-11 | Organigramm-Daten | P2 | fehlt | – | alles (JD28) |
| DA-13 | Bericht aktualisieren | P2 | fehlt | – | alles (JD19) |

### 2.11 Hyperlinks, QuickInfos und Kommentare

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| HL-01 | Hyperlinks | P1 | fertig | hyperlink.uis, text_pages.uis; dit export/pdf/links, svg/links | – |
| HL-02 | Hyperlinkbasis | P2 | fehlt | nur als Feld `HYPERLINKBASE()` | alles (JD20) |
| HL-03 | QuickInfo am Shape | P2 | fehlt | Befehl grau | alles (JD20) |
| HL-04 | Kommentare | P2 | fehlt | Befehl grau | alles (JD20) |
| HL-06 | Link-Verlauf | P2 | fehlt | – | alles (JD20) |

### 2.12 Designs (Themes), Varianten und Schnellformatvorlagen

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| TH-01 | Theme-Modell | P1 | fertig | dit designs (Teile, Rundlauf, Golden), core design-Tests | – |
| TH-02 | Designs anwenden | P1 | fertig | designs.uis (alle Seiten, Seite, Live-Vorschau, neue Shapes, Undo); dit designs/apply (THEMEGUARD, Sperren) | – |
| TH-03 | Eigene Designs | P1 | fertig | designs.uis (NetLab, Kontur, Kontrast; vom Nutzer abgenommen) | – |
| TH-04 | Varianten | P1 | fertig | designs.uis, dit designs/variant | – |
| TH-05 | Farben, Effekte, Verbinder, Verzierung | P2 | fehlt | fünf Befehle grau | alles (JD22) |
| TH-06 | Schnellformatvorlagen | P2 | fertig | format.uis (Variantenstile, Matrix, Golden format_quick), shell_commands.uis | – |

### 2.13 ShapeSheet: Abschnitte

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| SS-01 | Shape-Transformation | P1 | fertig | dit render/transform; core ops::tests; edit_shapes.uis | – |
| SS-02 | 1D-Endpunkte | P1 | fertig | dit render/transform/1d; core glue_tests | – |
| SS-03 | Linienformat | P1 | teilweise | dit render/lines (Muster, Kappen, Rundung, Transparenz, Enden) | CompoundType wird einfach gezeichnet |
| SS-04 | Füllformat und Schatten | P1 (Füllung), P2 | teilweise | P1-Teil fertig: dit render/fills | nur P2-Teil: Schatten ohne Weichzeichnung, schräge Art, Skalierung |
| SS-05 | Verlaufseigenschaften + Verlaufsstopps | P2 | teilweise | dit render/fills/gradients, export/pdf/fills/shadings | rechteckige und Pfad-Verläufe als radial genähert, Linienverläufe genähert, keine Oberfläche |
| SS-06 | Zeichen (Character) | P1 | teilweise | dit render/text (Läufe, fett/kursiv, Unterstreichung, Durchstreichung, Hoch/Tief, Case, FontScale) | asiatische und komplexe Schrift samt Größe |
| SS-07 | Absatz (Paragraph) | P1 | teilweise | dit render/text (Einzüge, Abstände, Ausrichtung, Aufzählung) | Blocksatz/verteilt wird linksbündig gezeichnet, Rechts-nach-links, BulletFont/-größe |
| SS-08 | Tabstopps | P2 | teilweise | dit roundtrip (Abschnitt bleibt erhalten) | nicht ausgewertet (nur DefaultTabStop) |
| SS-09 | Textblockformat | P1 | fertig | dit render/text/valign, background, vertical | – |
| SS-10 | Texttransformation | P1 | fertig | dit render/text/txtangle; core ops::tests::text_block_frame_moves_and_sizes | – |
| SS-11 | Textfelder (Field) | P2 | fertig | fields.uis; core field::tests | – |
| SS-12 | Ereignisse | P2 | teilweise | Datei: diagram_core formula (EventXFMod/_XFTRIGGER werden ausgewertet) | EventDblClick, EventDrop … werden in der App nicht ausgeführt |
| SS-13 | Bildeigenschaften | P2 | teilweise | dit roundtrip (Abschnitt bleibt erhalten) | Helligkeit, Kontrast, Gamma … nicht angewandt |
| SS-14 | Fremdbild-Info | P1 | teilweise | dit render/pictures (ImgOffset/ImgWidth, Zuschnitt am Shape-Kasten) | ClippingPath (Zuschnitt per Pfad in Darstellung, PDF und SVG; S–M, JD23) |
| SS-15 | Schutz | P1 | teilweise | core ops::tests::lock_bits_follow_the_lock_names, move_shapes_subshapes_lines_and_locks, set_frame_locks_aspect_and_1d; graue Griffe | Schutz-Dialog; LockDelete, LockSelect, LockFormat, LockCustProp, LockCrop, LockReplace ohne Wirkung |
| SS-16 | Verschiedenes | P2 | teilweise | stencils_search.uis (ShapeKeywords); dit stencils/structure (HideText); NonPrinting/ObjType in der Darstellung | Comment als QuickInfo, NoObjHandles/NoCtlHandles, DropOnPageScale, Kalender |
| SS-17 | Gruppeneigenschaften | P1 | teilweise | network_devices.uis (SVG-Gruppen mit SelectMode 0); DisplayMode in der Darstellung (dit render) | SelectMode 1/2, IsDropTarget, IsSnapTarget, IsTextEditTarget, DontMoveChildren |
| SS-18 | Klebeinfo | P1 | fertig | core glue_tests::static_glue_writes_visio_cells_and_follows, glue::tests::walk_preference_picks_the_sides; dit edit | – (WalkPreference als Vorrang zwischen gleich teuren Wegen) |
| SS-19 | Shape-Layout | P1 | teilweise | ObjType, ShapeRouteStyle, ConFixedCode, Relationships wirken (structures_layout.uis, dit structures) | Durchlässigkeit je Richtung, ShapePlowCode/Platzierung (hängt an CN-13), Linienübergänge je Verbinder (CN-06), DisplayLevel |
| SS-20 | Ebenenzugehörigkeit | P1 | fertig | layers.uis; dit render/fills/hidden-layer, two-layers | – |
| SS-21 | Theme-Eigenschaften | P2 | fertig | dit designs (foreign/mixed-numbers), render/pages/themed; core formula::tests::themed_values | – |
| SS-22 | Schnellformatvorlage | P2 | fertig | format.uis (Schnellformatvorlagen); core formula::tests::themed_values | – |
| SS-26 | Shape-ersetzen-Verhalten | P2 | teilweise | dit roundtrip (Abschnitt bleibt erhalten) | ohne Wirkung (SP-11 fehlt) |
| SS-28 | Geometrie (mehrere Abschnitte) | P1 | fertig | dit render/geometry (alle Zeilenarten inkl. Spline, Rel*), core geometry-Tests | – |
| SS-29 | Verbindungspunkte | P1 | fertig | canvas_glue.uis (Type 1 ist kein Ziel, Prompt als QuickInfo); core glue_tests::connection_points_turn_with_the_shape, static_glue_* | – |
| SS-30 | Steuerelemente (Controls) | P1 | teilweise | status_model.uis; core ops::tests::control_points_write_cells_as_visio_does | CanGlue (SP-04), Tip nicht als QuickInfo |
| SS-31 | Aktionen | P2 | fertig | actions_menu.uis (Menu, Checked, Disabled, Invisible; Klick führt Action aus); core formula::tests::run_action_effects | – (BeginGroup, Effekte wie GOTOPAGE ohne eigene Wirkung, FX-14) |
| SS-33 | Hyperlinks | P2 | fertig | hyperlink.uis (Beschreibung, Adresse, Unteradresse, Standard), übrige Zellen erhalten | – |
| SS-34 | Shape-Daten (Property) | P1 | fertig | panes_edit.uis (Typ, Format, Wert, Invisible), Shape-Daten definieren | – (Verify = SP-13, P2) |
| SS-35 | Benutzerdefinierte Zellen (User) | P1 | fertig | core formula::tests::parses_reference_forms, dit structures (msv*-Zellen), dit formulas | – |
| SS-36 | Scratch | P2 | fertig | core formula::tests::parses_reference_forms; dit formulas (Korpus) | – |
| SS-38 | Ebenen (Seite) | P1 | fertig | layers.uis | – |
| SS-39 | Seiteneigenschaften | P1 | teilweise | page_setup_ribbon.uis, canvas_glue.uis (InhibitSnap der Seite schaltet Raster, Führungslinien und dynamisches Gitter ab); core pagesetup::tests | UIVisibility ohne Wirkung |
| SS-40 | Seitenlayout | P1 | teilweise | RouteStyle, AvenueSize, LineToNode wirken (structures_layout.uis) | PlaceStyle/PlaceDepth/PlaceFlip, LineJump*, PlowCode, DynamicsOff, CtrlAsInput ohne Wirkung (meist an CN-06/CN-13, P2) |
| SS-41 | Druckeigenschaften | P1 | teilweise | print.uis (Ränder, Zentrieren, Kacheln, Papier, Ausrichtung, Skalierung) | Raster drucken; Druckeinrichtung gilt je Dokument statt je Seite |
| SS-42 | Lineal und Raster | P2 | teilweise | dit roundtrip (Abschnitt bleibt erhalten) | Rasterabstand/-dichte und Ursprung der Seite wirken nicht |
| SS-43 | Dokumenteigenschaften (Dokument-ShapeSheet) | P2 | teilweise | erhalten (dit samples/doc-sheet) | ohne Wirkung |
| SS-44 | Formatvorlagen-Eigenschaften | P2 | teilweise | dit roundtrip (Abschnitt bleibt erhalten) | EnableLine/Fill/TextProps, HideForApply ohne Wirkung |
| SS-46 | ShapeSheet-Fenster | P2 | fehlt | – | alles (JD23) |

### 2.14 Formel-Engine und Funktionen

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| FX-01 | Formel-Parser | P1 | teilweise | core formula::tests::parses_* (Einheiten, Operatoren, Bezüge, Fehlerwerte) | lokalisierte Namen (nur universelle Schreibweise) |
| FX-02 | Auswertung und Neuberechnung | P1 | fertig | dit formulas (Orakel 96,85 % / 99,31 %, Wachstum linear); core formula::tests::recalc_*, cycles_do_not_crash, deep_chain_* | – |
| FX-03 | Vererbung | P1 | fertig | core tests::inheritance_*, formula::tests::style_formula_runs_in_the_shape, inherited_formula_runs_in_the_instance | – |
| FX-04 | Schutzmechanik | P1 | fertig | core formula::tests::user_set_guard_and_plain_cells, user_set_through_setatref, user_set_setatref_with_expression, run_action_setf_toggles | – |
| FX-05 | Einheiten | P1 | fertig | core tests::units_codes_and_conversion, formula::tests::parses_numbers_with_units; dit render/pages/scale | – |
| FX-06 | Mathe und Logik | P1 | teilweise | core formula::tests::math_functions, rounding_functions, logic_and_errors, index_and_lookup, rand_with_a_fixed_seed | RAND rechnet nur, wenn die Zelle neu berechnet wird (nicht flüchtig) |
| FX-07 | Geometrie | P1/P2 | teilweise | P1-Teil fertig: core formula::tests::points_and_information_functions, loctoloc_loctopar_and_angles, bound_function, nurbs_and_polyline_texts | nur P2-Teil: PATHLENGTH, POINTALONGPATH, ANGLEALONGPATH, DISTTOPATH, NEARESTPOINTONPATH, INTERSECTX/Y, SEGMENTCOUNT, GRAVITY |
| FX-08 | Farbe und Theme | P1 | fertig | core formula::tests::colour_functions, hsl_hue_sat_lum, tint_shade_msotint_blend, themeval_and_references_as_values | – |
| FX-09 | Text | P1/P2 | teilweise | P1-Teil fertig: core formula::tests::shape_text_and_text_sizes, text_functions, find_function, substitute_replace_rept | nur P2-Teil: FORMAT, FORMATEX, EVALTEXT nicht berechnet |
| FX-10 | Datum und Zeit | P2 | teilweise | NOW (core field::tests::clock_file_and_properties) | DATE, TIME, DATETIME, DATEVALUE, TIMEVALUE, DAY … DAYOFYEAR |
| FX-11 | Dokument- und Shape-Info | P2 | fertig | core formula::tests::shape_and_page_information, points_and_information_functions; field::tests | – |
| FX-12 | Struktur | P2 | fertig | core structure::tests::structure_functions | – (WALKGLUE rechnet der Router in glue.rs, nicht die Engine) |
| FX-14 | Aktionen ohne VBA | P2 | teilweise | core formula::tests::run_action_effects | App führt Effekte (OPENTEXTWIN, GOTOPAGE, HYPERLINK, DOCMD …) nicht aus |
| FX-16 | Formel-Editor | P2 | fehlt | – | alles (JD23) |

## 3. Oberfläche

### 3.1 Fenster, Start und Backstage

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| UI-01 | Hauptfenster | P1 | teilweise | tests/smoke.uis, tests/panes.uis, tests/templates.uis (Titel „ZeichnungN – JFDiagrammer“) | nur **eine** Zeichnung je Instanz (`app.opened: Option`), Neu/Öffnen ersetzt sie; kein Fensterwechsel |
| UI-02 | NetLab-Look | P1 | fertig | tests/options.uis (Hell/Dunkel per Token), tests/smoke.uis; bei 200 % Skalierung scharf (Probe) | – |
| UI-03 | Startbildschirm | P1 | teilweise | tests/backstage.uis, tests/templates.uis (Vorlagen in Datei › Neu), tests/edit_save.uis (Zuletzt verwendet in Öffnen) | kein Startbildschirm: die App startet immer mit „Zeichnung1“; Option „Startbildschirm …“ wird gespeichert, aber nirgends gelesen; Begrüßung, empfohlene Vorlagen + Zuletzt verwendet auf einer Seite, Angeheftet, Änderungsdatum fehlen |
| UI-04 | Vorlagen-Vorschau | P1 | fertig | tests/templates.uis, tests/backstage.uis, tests/file_templates.uis (Doppelklick auf eine Katalogkachel erstellt), tests/status_ribbon_3.uis | – |
| UI-05 | Neu | P1 | fertig | tests/templates.uis, tests/templates_own.uis, tests/file_templates.uis (Suche in fünf Sprachen), tests/file_open_modes.uis (Neu aus vorhandener Zeichnung) | – |
| UI-06 | Öffnen | P1 | teilweise | tests/backstage.uis, tests/edit_save.uis, tests/viewer.uis, tests/file_open_modes.uis (schreibgeschützt, als Kopie), tests/xml_formats.uis | Orte (JD25); Filter für Binär-, SVG- und DXF-Formate (P2/P3) |
| UI-07 | Informationen | P2 | fehlt | – | Info zeigt nur Version/Lizenzen; keine Dokumenteigenschaften, Datei verkleinern, Schutz … (JD25) |
| UI-08 | Speichern / Speichern unter | P1 | fertig | tests/edit_save.uis, tests/templates_own.uis, tests/file_save_as.uis (Dateitypen, zuletzt genutzte Ordner) | – |
| UI-09 | Drucken (Backstage) | P1 | fertig | tests/print.uis (Einzelheiten 3.15) | – |
| UI-11 | Exportieren | P1 | fertig | tests/export.uis (PDF, SVG, PNG, JPEG, Seiten, Zwischenablage), tests/file_save_as.uis (Dateityp ändern) | – (Webseite ist P2, EX-09; XPS P3) |
| UI-12 | Schließen, Info | P1 | fertig | tests/file_ask_save.uis (Rückfrage bei Schließen, Strg+W, Neu, Öffnen, Fenster schließen; Abbrechen), tests/backstage.uis (Info) | – (die AWS-Namensnennung in Info kommt mit den AWS-Icons, JD32) |
| UI-13 | Optionen – Allgemein | P1 | teilweise | tests/options.uis (Hell/Dunkel), tests/status_ribbon_2.uis (Livevorschau aus), tests/file_options.uis („Keine QuickInfos“ wirkt) | QuickInfos „mit Beschreibung“ (Texte, UI-34), Startbildschirm (UI-03), Benutzername/Initialen (mit Kommentaren, JD20), Option Minisymbolleiste (JD22) |
| UI-14 | Optionen – Dokumentprüfung | P2 | fehlt | – | keine Seite (JD20) |
| UI-15 | Optionen – Speichern | P1 | teilweise | tests/options.uis, tests/templates_own.uis (Ort persönlicher Vorlagen), tests/file_options.uis (Standardspeicherort für Zeichnungen) | AutoWiederherstellen (DM-10, JD25) |
| UI-16 | Optionen – Sprache | P1 | teilweise | tests/lang_switch.uis (5 Sprachen ohne Neustart), tests/options.uis (Region) | Bearbeitungs-/Korrektursprachen mit Reihenfolge |
| UI-17 | Optionen – Barrierefreiheit | P2 | fehlt | – | (JD20) |
| UI-18 | Optionen – Erweitert | P1 | teilweise | tests/options.uis (Verbindungspfeile), tests/file_options.uis (Rückgängig-Tiefe ab der nächsten Zeichnung, Eingabetaste beendet Text) | wirkungslos: Livedynamik, Verbinder teilen, Verbinder mit Shapes löschen, Rad zoomt, Auswahl beim Zoomen zentrieren, Maßeinheit, Entwicklermodus; ganz fehlend: Übergänge, mehr Griffe, Formel-Autovervollständigen, Text-Zoom, Freihand, Seitenansicht drehen, Warnungen, Visio-Daten in SVG, Speicherorte Schablonen |
| UI-19 | Menüband/Schnellzugriff anpassen | P2 | teilweise | tests/qat.uis (beliebige Befehle, Reihenfolge, Zurücksetzen) | unter dem Menüband, Import/Export (Menüband anpassen P3) |

### 3.2 Titelleiste, Registerzeile und Menüband allgemein

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| UI-30 | Schnellzugriff | P1 | fertig | tests/qat.uis, tests/status_ribbon.uis (QuickInfo „Rückgängig: Ausrichtung (Strg+Z)“), tests/status_ribbon_2.uis | – |
| UI-31 | Registerzeile | P1 | teilweise | tests/smoke.uis | Register Zeichnen, Daten, Prozess, Überprüfen, Entwicklertools (Entwicklermodus), Hilfe |
| UI-32 | Befehlssuche | P2 | fehlt | – | kein Feld in der Registerzeile (JD25) |
| UI-33 | Kommentare-Knopf | P2 | fehlt | – | (JD20) |
| UI-34 | Menüband-Verhalten | P1 | teilweise | tests/smoke.uis (DS, groß/klein), tests/status_ribbon_2.uis (schmales Fenster: nur Icons, Gruppen falten zu einem Knopf), tests/keytips.uis | Menüband reduzieren / automatisch ausblenden / nur Register, Touch-Modus, QuickInfos mit Beschreibung (nur Name + Kürzel), Kataloge mit Scrollpfeilen (Designs nur ▾-Menü) |
| UI-35 | Livevorschau | P2 | teilweise | tests/designs.uis, tests/status_ribbon_2.uis (abschaltbar) | nur Designs und Varianten; Schnellformatvorlagen, Effekte, Ausrichten, Layout, Container, Legenden, Schrift ohne |
| UI-36 | Kontextregister | P1 | teilweise | tests/structures_container.uis, tests/context.uis (Container, farbiger Kopf), tests/edit_text.uis (Text), tests/structures_cff.uis, tests/templates.uis (CFF) | Bild (P2), Organigramm (P2), Schablone bearbeiten (P2), Brainstorming/Zeitachse/Kalender/Symbol-Editor (P3); CFF-Register ohne farbigen Kopf (gewöhnliches Register) |
| UI-37 | Zugriffstasten | P1 | fertig | tests/keytips.uis | – (Backstage ohne Key-Tips, Menüs über Mnemonics) |

### 3.3 Register „Start“

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| RS-01 | Einfügen (SK) | P1 | fertig | tests/paste_special.uis, tests/edit_group.uis (Duplizieren) | – |
| RS-02 | Ausschneiden / Kopieren | P1 | fertig | tests/edit_group.uis, tests/status_ribbon.uis | – |
| RS-03 | Format übertragen | P1 | fertig | tests/format_painter.uis | – |
| RS-04 | Schriftart, Schriftgrad | P1 | teilweise | tests/status_ribbon.uis (Georgia, 24 pt wirken, Dialog zeigt sie), tests/shell_commands.uis | Livevorschau; Liste = Design-, Dokument- und feste Familien (keine installierten) |
| RS-05 | Schriftgrad +/- | P2 | fehlt | – | kein Befehl (JD25) |
| RS-06 | F/K/U/Durchgestrichen | P1 | fertig | tests/format.uis (Fett), tests/status_ribbon.uis (Kursiv, Unterstrichen, Durchgestrichen) | – (P2-Rest: Durchgestrichen zeigt keinen Umschaltzustand) |
| RS-08 | Schriftfarbe | P1 | fertig | tests/status_ribbon.uis (Design-, Variant-, Standardfarben, Zuletzt verwendet, Farbe im Dialog) | – |
| RS-09 | DS Dialog „Text“ | P1 | fertig | tests/text_dialog.uis (mit Register Zeichen: Position, Groß-/Kleinschreibung, Skalierung, Zeichenabstand) | – (Tabstopps, Aufzählungszeichen sind P2) |
| RS-10 | Oben/Mittig/Unten | P1 | fertig | tests/status_ribbon.uis, tests/text_dialog.uis | – |
| RS-11 | Links/Zentriert/Rechts/Blocksatz | P1 | fertig | tests/format.uis, tests/status_ribbon.uis | – |
| RS-12 | Einzug, Aufzählung, Text drehen | P2 | teilweise | tests/text_dialog.uis (Einzug im Dialog) | Knöpfe im Menüband, Aufzählungszeichen, Text drehen (JD22/JD25) |
| RS-13 | Zeigertool | P1 | fertig | tests/smoke.uis, tests/status_ribbon.uis (Strg+1) | – |
| RS-14 | Verbinder | P1 | fertig | tests/edit_connect.uis, tests/status_ribbon.uis (Strg+3, bleibt aktiv) | – |
| RS-15 | Text | P1 | fertig | tests/edit_text.uis, tests/status_ribbon.uis (Strg+2) | – |
| RS-16 | Zeichentools | P1 | fertig | tests/draw_tools.uis, tests/status_ribbon.uis (Bleistift, Linie) | – |
| RS-17 | Verbindungspunkt-Werkzeug | P2 | fehlt | – | (JD21/JD23) |
| RS-18 | Textblock-Werkzeug | P2 | fehlt | – | (JD22) |
| RS-19 | Schnellformatvorlagen | P2 | teilweise | tests/format.uis, tests/shell_commands.uis | nur im Bereich Shape formatieren, keine Galerie im Menüband; „Designs zulassen“ |
| RS-20 | Füllung | P1 | fertig | tests/format.uis | – |
| RS-21 | Linie | P1 | fertig | tests/format.uis, tests/shell_commands.uis, tests/status_ribbon.uis (13 Pfeil-Schnellwahlen) | – |
| RS-22 | Effekte | P2 | teilweise | tests/format.uis (Schatten im Bereich) | kein Effekte-Menü im Menüband; Spiegelung, Leuchten usw. (P3) |
| RS-23 | DS Shape formatieren | P1 | fertig | tests/panes.uis, tests/format.uis | – |
| RS-24 | Ausrichten | P1 | fertig | tests/shell_commands.uis (Automatisch ausrichten), tests/status_ribbon.uis (Links ausrichten zum Anker) | – |
| RS-25 | Positionieren | P1 | teilweise | tests/status_ribbon.uis (Horizontal verteilen), tests/edit_shapes.uis (Drehen/Spiegeln) | Automatischer Abstand, Automatisch ausrichten und Abstände, Abstandsoptionen, weitere Verteilungsoptionen, Seitenumbrüche (verschieben/vermeiden), Diagramm drehen |
| RS-26 | Vordergrund / Hintergrund | P1 | fertig | tests/edit_group.uis | – |
| RS-27 | Gruppieren | P1 | teilweise | tests/edit_group.uis | Zu Gruppe hinzufügen, Aus Gruppe entfernen |
| RS-28 | Shape ändern | P2 | fehlt | – | (JD21) |
| RS-29 | Suchen | P1 | fertig | tests/find.uis | – |
| RS-30 | Ebenen | P1 | fertig | tests/layers.uis | – |
| RS-31 | Markieren | P1 | teilweise | tests/qat.uis, tests/edit_group.uis (Lasso), tests/select_by_type.uis, tests/canvas_guides.uis (Führungslinien nach Typ) | Nach Typ: OLE und Freihand |

### 3.4 Register „Einfügen“

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| RE-01 | Neue Seite | P1 | fertig | tests/pages.uis | – (offen: Entscheidung „hinter aktueller Seite oder am Ende“) |
| RE-02 | Bilder | P1 | fertig | tests/pictures.uis | – |
| RE-06 | Container | P1 | teilweise | tests/structures_container.uis, tests/status_ribbon_2.uis (leer einfügen) | 6 statt 14 Stile; Menü statt Katalog mit Vorschau |
| RE-07 | Beschriftung | P1 | teilweise | tests/structures_callout.uis, tests/status_ribbon_2.uis (ohne Auswahl) | 6 statt 14 Stile (fehlen u. a. Unterstrich, Klammer/Rechteck mit Zeiger, Keil, Ellipse mit Zeiger, Gedankenwolke, ovale Sprechblase, Tropfen, Zickzack, schräges Etikett); Menü statt Katalog |
| RE-08 | Verbinder | P1 | fertig | tests/status_ribbon_2.uis (schaltet das Werkzeug ein) | – |
| RE-09 | Link | P1 | fertig | tests/hyperlink.uis | – |
| RE-10 | Textfeld | P1 | fertig | tests/text_vertical.uis, tests/edit_text.uis | – |
| RE-11 | QuickInfo | P2 | fehlt | Befehl grau (cmds_later) | (JD20) |
| RE-13 | Feld | P1 | fertig | tests/fields.uis | – |
| RE-14 | Symbol | P2 | fehlt | Befehl grau (cmds_later), nur im Textregister | (JD25) |

### 3.6 Register „Entwurf“

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| RW-01 | Ausrichtung | P1 | fertig | tests/page_setup_ribbon.uis | – |
| RW-02 | Format | P1 | fertig | tests/page_setup_ribbon.uis, tests/page_formats.uis (Umschläge und Behördenformate im Menü Größe) | – |
| RW-03 | Automatisch anpassen | P1 | fertig | tests/page_setup_ribbon.uis | – |
| RW-04 | DS Seite einrichten | P1 | fertig | tests/page_setup_ribbon.uis, tests/print.uis, tests/panes.uis | – (Inhalt 3.15) |
| RW-05 | Designkatalog | P1 | fertig | tests/designs.uis, tests/status_ribbon_2.uis (eigenes Design speichern) | – (eigene Designs statt Visios Gruppen, V3) |
| RW-06 | Varianten | P1 | teilweise | tests/designs.uis | Farben, Effekte, Verbinder, Schriftarten, Verzierung grau (P2-Teil, cmds_later) |
| RW-07 | Hintergründe | P2 | fehlt | – | (JD22) |
| RW-08 | Rahmen und Titel | P2 | fehlt | – | (JD22) |
| RW-09 | Seite neu layouten | P2 | fehlt | – | (JD21) |
| RW-10 | Verbinder (Layout) | P1 | teilweise | tests/status_ribbon_2.uis (Rechtwinklig, Gerade, Gebogen), tests/edit_connect.uis | Linienübergänge anzeigen und Stil (grau, P2 CN-06) |
| RW-11 | DS Layout und Routing | P1 | fertig | tests/structures_layout.uis | – |

### 3.7 Register „Daten“

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| RD-02 | Benutzerdefinierter Import | P2 | fertig | data_import_xlsx.uis, data_import_csv.uis (Datenauswahl-Assistent; ODBC/SQL grau, P3) | – |
| RD-03 | Alle aktualisieren | P2 | fertig | data_refresh_conflict.uis (Alle aktualisieren, Daten aktualisieren..., Intervall, nach Aktualisierung automatisch verknüpfen) | – |
| RD-04 | Datengrafik-Katalog | P2 | fehlt | – | (JD19) |
| RD-05 | Position, Konfigurieren | P2 | fehlt | – | (JD19) |
| RD-07 | Legende einfügen | P2 | fehlt | – | (JD19) |
| RD-08 | Fenster Shape-Daten, Externe Daten, Datengrafikfelder | P1 | teilweise | tests/panes.uis, tests/panes_edit.uis, data_import_xlsx.uis (Register Daten: Fenster „Shape-Daten“ und „Externe Daten“) | Datengrafikfelder (JD19) |
| RD-09 | Daten verknüpfen | P2 | fertig | data_link.uis | – |
| RD-10 | Erweiterte Datengrafiken | P2 | fehlt | – | (JD19) |

### 3.8 Register „Prozess“

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| RP-01 | Teilprozess neu erstellen | P2 | fehlt | – | Register fehlt (JD24) |
| RP-02 | Mit vorhandenem verknüpfen | P2 | fehlt | – | (JD24) |
| RP-04 | Diagramm überprüfen | P2 | fehlt | – | (JD24; Vorlagen bringen Regelsätze schon mit) |
| RP-05 | Problem ignorieren | P2 | fehlt | – | (JD24) |
| RP-06 | Problemfenster | P2 | fehlt | – | (JD24) |

### 3.9 Register „Überprüfen“, „Ansicht“, „Entwicklertools“, „Hilfe“

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| RU-01 | Rechtschreibung (F7) | P2 | fehlt | – | Register Überprüfen fehlt (JD20) |
| RU-03 | Barrierefreiheit überprüfen | P2 | fehlt | – | (JD20) |
| RU-04 | Sprache | P2 | fehlt | – | (JD20) |
| RU-05 | Kommentare | P2 | fehlt | „Neuer Kommentar“ im Shape-Menü grau | (JD20) |
| RU-06 | Shape-Berichte | P2 | fehlt | – | (JD19) |
| RA-01 | Präsentationsmodus | P2 | fehlt | – | (JD22) |
| RA-02 | Lineal, Seitenumbruch, Gitternetz, Führungslinien | P1 | fertig | tests/panes.uis (Gitternetz, Lineal), tests/edit_view.uis, tests/status_ribbon_2.uis, tests/canvas_guides.uis (Führungslinien aus: nicht gezeichnet, kein Einrasten, keine neue aus dem Lineal) | – |
| RA-03 | Aufgabenbereiche | P1 | teilweise | tests/panes.uis, tests/shell_commands.uis | Zeichnungsexplorer, Ebenen, Kommentare, Probleme (alle P2: JD22/JD20/JD24) |
| RA-04 | DS Lineal und Gitternetz | P2 | fehlt | – | (JD22) |
| RA-05 | Zoom, An Fenster anpassen, Seitenbreite | P1 | fertig | tests/shell_commands.uis, tests/viewer.uis, tests/status_ribbon_2.uis | – |
| RA-06 | Dyn. Gitter, Verbindungspfeile, Verbindungspunkte | P1 | fertig | tests/edit_view.uis, tests/structures_arrows.uis, tests/status_ribbon_2.uis, tests/canvas_snap.uis (Verbindungspunkte aller Shapes, Pixelprüfung) | – |
| RA-07 | DS Ausrichten und Kleben | P1 | teilweise | tests/canvas_snap.uis (Dialogstarter und Alt+F9 öffnen den Dialog; OK übernimmt, Abbrechen nicht) | übrige Ziele (Geometrie, Ziehpunkte, Scheitelpunkte …), Kleben-Ziele, Einraststärke, im Dokument speichern (DM-03; JD22) |
| RA-08 | Neues Fenster, Anordnen, Wechseln | P2 | fehlt | – | (JD25) |
| RA-09 | Add-Ons (Array, Nummerieren) | P2 | fehlt | – | (JD21) |
| RX-03 | ShapeSheet anzeigen | P2 | fehlt | – | Register fehlt (JD23) |
| RX-04 | Zeichentools, Vorgänge, Gruppieren | P1 | teilweise | tests/draw_tools.uis, tests/edit_group.uis (im Register Start) | Register Entwicklertools (JD23); Vorgänge (P2, SP-08, JD21) |
| RX-05 | Shape-Name | P2 | fehlt | – | (JD23) |
| RX-06 | Verhalten | P2 | fehlt | – | (JD23) |
| RX-07 | Schutz | P2 | fehlt | – | (JD23) |
| RX-08 | Neue Schablone (metrisch/US) | P2 | teilweise | Befehl Weitere Shapes › Neue Schablone (legt „Schablone1“ an; kein eigener Check) | kein Register Entwicklertools; kein Check (JD23) |
| RX-09 | Dokumentschablone, Zeichnungsexplorer, Masterexplorer | P2 | fehlt | – | (JD23) |
| RH-01 | Hilfe (F1) | P2 | fehlt | – | kein Register, kein F1 (JD25) |

### 3.10 Kontext- und Vorlagenregister

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| RK-01 | Container-Tools › Format | P1 | fertig | tests/structures_container.uis, tests/status_ribbon_3.uis (Ränder, Automatische Größe, Stile, Kopfzeile, Sperren, Inhalt auswählen) | – |
| RK-02 | Bildtools › Format | P2 | fehlt | – | (JD25) |
| RK-03 | Funktionsübergreifendes Flussdiagramm | P1 | teilweise | tests/structures_cff.uis, tests/templates.uis, tests/status_ribbon_3.uis | Einfügen › Seite; Ausrichtung wirkt nur auf **neue** Flussdiagramme (auf ein vorhandenes ohne Wirkung); Richtung, Ränder, Formatvorlagen |
| RK-04 | Organigramm | P2 | fehlt | – | (JD28) |
| RK-08 | Schablone bearbeiten | P2 | fehlt | STENCIL_EDIT/NEW_MASTER/ORDER grau | (JD23) |
| RK-10 | Master bearbeiten | P2 | fehlt | – | (JD23) |
| RK-11 | Textbearbeitung | P1 | teilweise | tests/edit_text.uis, tests/fields.uis, tests/context_text.uis, tests/status_ribbon_3.uis (Feld, Link an) | Symbol und QuickInfo grau (P2), Minisymbolleiste (P2, ZF-11) |

### 3.11 Shapes-Seitenleiste

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| SL-01 | Aufbau | P1 | teilweise | stencils.uis, status_ui_panel.uis (Dock-Menü) | Einklappen gibt nur eine Leiste mit dem Bereichssymbol (keine Shape-/Schablonensymbole); kein Andocken oben (nur links/rechts/unten/schwebend); spaltenweises Breitenziehen nicht vorhanden/geprüft |
| SL-02 | Shape-Kacheln | P1 | teilweise | stencils.uis (4 Ansichten), Probe (QuickInfo „Name – Beschreibung“) | Livevorschau beim Zeigen (Option) |
| SL-03 | Platzieren | P1 | teilweise | stencils_drop.uis, status_ui_panel.uis (Strg+Enter) | Strg-Ziehen für mehrere (Probe: legt eins ab); Ablegen auf Verbinder teilt ihn nicht (p4); Container-/Listen-Leuchten beim Ziehen aus der Leiste (JD10-Notiz) |
| SL-04 | Shapes suchen | P1 | fertig | stencils_search.uis (ein Wort genügt, Suche ausblenden), stencils_lang.uis; core instance::tests::search_reads_the_quickinfo | – |
| SL-05 | Weitere Shapes | P1 | teilweise | stencils.uis, network_devices.uis, Probe | „Dokumentschablone anzeigen“; Filter `.vss/.vsx` (Formate JD16/17); Azure-Eintrag (P3) |
| SL-06 | Schnell-Shapes | P1 | teilweise | stencils.uis | Schnell-Shapes speisen die Verbindungspfeile nicht (keine Schnell-Shape-Leiste, CN-07-Rest) |
| SL-07 | Favoriten / Meine Shapes | P1 | fertig | stencils_favorites.uis, my_shapes.uis | – (`.vssx` im Benutzerordner nur in Fensterläufen, headless im Speicher) |
| SL-08 | Schablonen-Menü | P2 | teilweise | stencils.uis, panes.uis | Bearbeiten, Neues Master-Shape, Reihenfolge grau (cmds_later.rs) → JD23 |
| SL-09 | Leerzustand | P1 | fertig | stencils.uis | – |
| SL-10 | Master aus Zeichnung | P2 | teilweise | my_shapes.uis (über Kontextmenü) | Ziehen von der Seite in eine Schablone (Zeichenfläche ist keine Ziehquelle) → JD23 |

### 3.12 Zeichenfenster

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| ZF-01 | Seite und Zeichenfläche | P1 | teilweise | status_ui_print.uis (Seitenumbrüche + Shot), page_setup_ribbon.uis, Probe | Seitenfarbe aus dem Design (Seite immer weiß) |
| ZF-02 | Lineale | P1 | teilweise | edit_view.uis, Probe (1:10-Seite zählt in Zeichnungseinheiten, Marken der Auswahl) | Nullpunkt verschieben (Strg-Ziehen aus der Ecke wirkungslos, = ZF-21) |
| ZF-03 | Raster | P1 | teilweise | panes.uis, edit_shapes.uis (Einrasten) | Rasterstufen fein/normal/grob/fest; Dialog „Lineal und Gitternetz“ |
| ZF-04 | Führungslinien | P1 | teilweise | edit_view.uis (aus dem Lineal, Einrasten) | Führungslinie auswählen, verschieben, löschen, geklebte Shapes mitnehmen (Probe: Klick wählt nichts, Ziehen/Entf wirkungslos) |
| ZF-05 | Dynamisches Gitter | P1 | teilweise | edit_view.uis | Abstandspfeile bei gleichem Abstand, Maßgleichheit beim Größenändern |
| ZF-06 | Auswahl | P1 | fertig | edit_shapes.uis, edit_group.uis, status_ui_panel.uis, select_by_type.uis, canvas_keys.uis (Option „Teilweise umschlossene Shapes auswählen“) | – |
| ZF-07 | Griffe | P1 | teilweise | edit_golden.uis, edit_shapes.uis, edit_connect.uis | Textblock-Griffe (TX-03); gesperrte Griffe grau statt schraffiert |
| ZF-08 | Zoom und Verschieben | P1 | fertig | status_ui_keys.uis, edit_view.uis, panes.uis, canvas_view.uis (Zoom und Ausschnitt je Seite) | – |
| ZF-09 | Livedynamik | P1 | teilweise | edit_shapes.uis (Vorschau beim Ziehen) | Option „Livedynamik aktivieren“ ohne Wirkung (nur in options.rs gelesen), kein Umriss-Modus |
| ZF-10 | AutoVerbinden-Pfeile und Schnell-Shape-Leiste | P1 | teilweise | structures_arrows.uis, structures_arrows_overlap.uis | Schnell-Shape-Leiste mit Vorschau (CN-07-Rest) |
| ZF-11 | Minisymbolleiste | P2 | fehlt | Probe | → JD22 |
| ZF-12 | Kontextmenü am Shape | P1 | teilweise | status_ui_panel.uis, hyperlink.uis, my_shapes.uis, actions_menu.uis (Aktionen des Shapes), context_jd15.uis (Shape-Name, Bahn davor/danach), data_drag_row.uis (Daten › Verknüpfung mit Zeile aufheben) | Daten ▸ verknüpfte Zeile anzeigen, Datengrafik (JD19), Kommentar grau (JD20) |
| ZF-13 | Kontextmenüs Seite, Register, Schablone | P1 | fertig | status_ui_panel.uis, pages.uis, panes.uis | – |
| ZF-14 | Seitenregister | P1 | teilweise | pages.uis, viewer.uis | Ziehen = Umordnen (Probe; nur Dialog „Seiten neu anordnen“) |
| ZF-15 | Hyperlink-Zeiger | P1 | fertig | hyperlink.uis | – (Link-Cursor nicht skriptbar) |
| ZF-16 | Text-Zoom | P2 | fehlt | – | → JD22 |
| ZF-17 | Präsentationsmodus | P2 | fehlt | – | → JD22 |
| ZF-18 | Barrierefreiheit | P1 | teilweise | edit_shapes.uis (Tab), status_ui_keys.uis | Alternativtext (`visAltText`) und Lesereihenfolge (`visNavOrder`); Shapes heißen nur nach Name/Text |
| ZF-19 | Stempel-Werkzeug | P2 | teilweise | status_ui_panel.uis | kein eigenes Werkzeug; wiederholtes Ablegen nur mit gehaltener Umschalttaste → JD22 |
| ZF-20 | Zeichnung zentrieren | P2 | fehlt | – | → JD22 |
| ZF-21 | Nullpunkt verschieben | P2 | fehlt | Probe | → JD22 |
| ZF-22 | Lupe | P1 | fertig | edit_view.uis | – |

### 3.13 Aufgabenbereiche

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| AB-01 | Allgemein | P1 | teilweise | panes.uis, status_ui_panel.uis | Andocken oben (Dock kennt links/rechts/unten, schwebend, Einklappen, Ausblenden) |
| AB-02 | Shapes | P1 | teilweise | stencils.uis, status_ui_panel.uis (wie 3.11) | wie SL-01 bis SL-06 (eingeklappte Leiste, Livevorschau, Teilen beim Ablegen …) |
| AB-03 | Shape-Daten | P1 | teilweise | panes_edit.uis (Zahl, feste Liste), context_jd15.uis (Kontextmenü: Shape-Daten definieren), data_ask_drop.uis (Boolesch, Mehrfachauswahl; Boolesch schrieb immer FALSE, behoben in JD18) | Shape-Datensätze im Kontextmenü (DA-03, P3); Datum ohne Skript |
| AB-04 | Größe und Position | P1 | teilweise | panes_edit.uis | Führungslinien-Felder (Art, X, Y, Winkel); 1D-Formen „Anfang+Länge+Winkel“/„Ende+…“ als Wahl (alle Felder auf einmal) |
| AB-05 | Shape formatieren | P1 | teilweise | format.uis, panes.uis | Effekte Spiegelung, Leuchten, weiche Kanten, 3D grau; Schatten nur aus/außen ohne Voreinstellungen |
| AB-06 | Verschieben und Zoomen | P1 | fertig | panes_edit.uis, status_ui_panel.uis (Rahmen ziehen schwenkt), canvas_view.uis (Kante oder Ecke ziehen zoomt) | – |
| AB-07 | Navigation | P2 | fehlt | – | → JD22 |
| AB-08 | Zeichnungsexplorer | P2 | fehlt | – | → JD22 |
| AB-09 | Ebenen | P2 | teilweise | layers.uis (Dialog) | als Bereich → JD22 |
| AB-10 | Kommentare | P2 | fehlt | commands_check.uis (COMMENT_NEW zurückgestellt) | → JD20 |
| AB-11 | Externe Daten | P2 | teilweise | data_import_xlsx.uis, data_import_csv.uis, data_drag_row.uis (Register je Quelle, Kettensymbol, Sortieren, Kontextmenü, Ziehen) | Datengrafikfelder im Menü (JD19); seitliches Scrollen bei vielen Spalten |
| AB-12 | Datengrafikfelder | P2 | fehlt | – | → JD19 |
| AB-13 | Probleme | P2 | fehlt | – | → JD24 |

### 3.14 Statusleiste

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| SB-01 | Seite | P1 | fertig | viewer.uis, status_ui_keys.uis | – |
| SB-02 | Maße der Auswahl | P1 | fertig | panes.uis, status_ui_keys.uis, canvas_keys.uis (1D: „Länge: 89 mm  Winkel: -27°“) | – |
| SB-03 | Sprache | P2 | fehlt | – | → JD20 |
| SB-04 | Zoom | P1 | teilweise | panes.uis, shell_commands.uis, canvas_view.uis (Regler mit 100 % in der Mitte) | Präsentationsmodus (ZF-17, JD22), Fenster wechseln (JD25) |

### 3.15 Drucken und Seite einrichten

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| PR-01 | Druckansicht | P1 | teilweise | print.uis, status_ui_print.uis | Zoom und Verschieben in der Vorschau |
| PR-02 | Druckbereich | P1 | teilweise | print.uis, status_ui_print.uis | „Aktuelle Ansicht“ und „Auswahl“ (M, JD22), hohe Qualität (Effekte ein/aus); Drucker nur über den Systemdialog |
| PR-03 | Kacheln | P1 | teilweise | print.uis (4 × 2 Blätter), page_setup_ribbon.uis, status_ui_print.uis | Shapes von Umbrüchen wegschieben/meiden |
| PR-04 | Kopf- und Fußzeile | P2 | fehlt | – | → JD22 |
| PR-05 | Drucken einrichten | P1 | teilweise | print.uis, status_ui_print.uis, page_formats.uis (Gitternetz drucken: Häkchen und Vorschau) | Vorschau im Dialog; PDF-Druck des Gitters unter Linux ungeprüft |
| PR-06 | Seitengröße | P1 | fertig | print.uis, status_ui_print.uis, page_setup_ribbon.uis (Automatisch anpassen über Entwurf) | – |
| PR-07 | Zeichnungsmaßstab | P1 | fertig | page_setup_ribbon.uis, status_ui_print.uis, page_formats.uis (vier Gruppen) | – |
| PR-08 | Seiteneigenschaften | P1 | fertig | print.uis, status_ui_print.uis, page_formats.uis (Maßeinheit der Seite) | – |
| PR-09 | Layout und Routing | P1 | teilweise | structures_layout.uis, status_ui_print.uis | Richtung, Verbinder-Darstellung, Größe der Linienübergänge; „Shapes auf Verbinder teilen“/„beim Ablegen verschieben“ ohne Wirkung |

### 3.16 Tastenkürzel

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| KB-01 | Datei | P1 | fertig | status_ui_keys.uis, lang_switch.uis, file_ask_save.uis (Strg+W mit Rückfrage) | – |
| KB-02 | Bearbeiten | P1 | fertig | edit_shapes.uis, edit_group.uis, find.uis | – (Spiegeln auf Strg+Umschalt+H) |
| KB-03 | Zoom | P1 | fertig | status_ui_keys.uis, edit_view.uis, viewer.uis | – |
| KB-04 | Schwenken | P1 | fertig | status_ui_keys.uis (Rad, Umschalt+Rad), canvas_keys.uis (Strg+Umschalt+Rechts-Ziehen schwenkt, ohne Kontextmenü) | – |
| KB-05 | Seiten | P1 | fertig | status_ui_keys.uis, viewer.uis | – |
| KB-06 | Fokus/Barrierefreiheit | P1 | teilweise | status_ui_keys.uis, canvas_keys.uis (Tab ohne Auswahl wählt das erste Shape, Umschalt+Tab das letzte) | Tab wählt sofort (Enter/Umschalt+Enter-Auswahl entfällt, Enter bearbeitet Text); Überspringen gesperrter Shapes ungeprüft |
| KB-07 | Anordnen | P1 | fertig | status_ui_keys.uis, edit_shapes.uis, edit_group.uis | – (Aufheben Strg+Umschalt+G, Vorder-/Hintergrund Strg+Umschalt+Bild↑/↓ laut shortcuts.tsv, F8 nur mit ≥ 2 Shapes) |
| KB-08 | Werkzeuge | P1 | teilweise | status_ui_keys.uis, format_painter.uis | Strg+Umschalt+1/2/4 (Verbindungspunkt-, Zuschneide-, Textblock-Werkzeug fehlen) |
| KB-09 | Zeichenwerkzeuge | P1 | fertig | draw_tools.uis, status_ui_keys.uis, canvas_keys.uis (Strg+Umschalt-Ziehen kopiert und hält die Achse, eine Rückgängig-Stufe) | – |
| KB-10 | Text | P1 | teilweise | edit_text.uis, text_dialog.uis, file_options.uis (Eingabetaste beendet / Umschalt+Eingabe) | F7 Rechtschreibung (JD20) |
| KB-11 | Textformat | P1/P2 | teilweise | status_ui_keys.uis (Fett/Kursiv/Unterstrichen) | doppelt unterstrichen, Großbuchstaben, Kapitälchen, tief/hoch, größer/kleiner, ältere Visio-Belegung |
| KB-12 | Textausrichtung | P1 | fertig | status_ui_keys.uis | – |
| KB-13 | Links | P1 | fertig | hyperlink.uis | – |
| KB-14 | Fenster | P2 | teilweise | canvas_snap.uis (Alt+F9 öffnet Ausrichten und Kleben) | F5, Fenster anordnen → JD25 |
| KB-15 | Shapes-Seitenleiste | P1 | teilweise | status_ui_panel.uis (Pfeile, Pos1/Ende, Strg+Enter) | Bild↑/↓; Umschalt+Enter wählen (legt ab); Strg+C/V/X, Strg+A, Entf, F2 in bearbeitbarer Schablone |
| KB-16 | Menüband | P1 | teilweise | keytips.uis, status_ui_keys.uis (Umschalt+F10) | F1 Hilfe (keine Hilfe); F6 wechselt den Fokus (Probe, ohne Skript) |
| KB-17 | Entwickler | P2 | fehlt | – | → JD23 |
| KB-18 | Wiederholen | P2 | fehlt | – | → JD22 |

### 3.17 Sprachen und Regionen

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| LG-01 | Fünf Oberflächensprachen | P1 | fertig | lang_en/es/it/fr.uis, lang_switch.uis, check_lang.py | – |
| LG-02 | Umschalten ohne Neustart | P1 | fertig | lang_switch.uis | – |
| LG-03 | Hilfe und QuickInfos | P1 | teilweise | Probe („Kopieren (Strg+C)“) | Beschreibung in Befehls-QuickInfos (Option „Mit Beschreibung“ ohne Wirkung); Offline-Hilfe/Online-Doku |
| LG-04 | Namen von Schablonen, Shapes, Vorlagen | P1 | fertig | stencils_drop.uis, stencils_lang.uis, dio stencils (masters/place), dio templates | – |
| LG-05 | Mehrsprachige Shape-Suche | P2 | fertig | stencils_search.uis, stencils_lang.uis | – |
| LG-06 | Begriffsglossar | P1 | fertig | lang/glossary.tsv, check_lang.py | – (Begriff „AutoVerbinden“ weiter offen) |
| LG-07 | Einheiten und Papierformate je Region | P1 | fertig | file_region_us.uis (en-US: Startzeichnung und Datei › Neu in Zoll/Letter); dio stencils (_m/_u), templates.uis (US-Vorlage) | – |
| LG-08 | Tastenkürzel je Sprache | P1 | fertig | lang/shortcuts.tsv, check_lang.py (Konflikte), lang_switch.uis | – |
| LG-09 | Rechtschreibung je Sprache | P2 | fehlt | – | → JD20 |
| LG-10 | Formelnamen | P2 | fehlt | – | → JD23 |
| LG-11 | Übersetzungsprüfung | P1 | fertig | check_lang.py (Kataloge), Bauwerkzeug + dio stencils/errors (Namenslisten) | – |

## 4. Shape-Kategorien und Schablonen

### 4.0 Regeln für alle Schablonen

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| SH-G01 | Stilrichtlinie | P1 | fertig | stencils/README.md › Stilregeln, symbols/README.md (vom Nutzer abgenommen) | – (SVG-Symbole behalten feste Kategoriefarben) |
| SH-G02 | Echte Master | P1 | teilweise | dio stencils (masters, engine, shapes, network) | keine Ebene in den Mastern (Bauwerkzeug schreibt keine Master-Ebenen); Kategorien nur bei Flussdiagramm, Strukturen, Rack |
| SH-G03 | Schablonen als Dateien | P1 | fertig | dio stencils, stencils_lang.uis | – |
| SH-G04 | Schnell-Shapes je Schablone | P1 | fertig | stencils.uis, dio stencils/masters, dio stencils/structure/callouts/*/quick (Beschriftungen mit vier Schnell-Shapes) | – (Nutzung für AutoVerbinden: CN-07, JD21) |
| SH-G05 | Fremde Master anzeigen | P1 | fertig | dio render (lokaler Korpus), viewer.uis | – („Shape ändern“ ist SP-11, P2/JD21) |
| SH-G06 | AWS-Icons ausliefern | P2 | fehlt | – | → JD32 |
| SH-G08 | Eigene Ergänzungen | P2 | fehlt | – | → JD34 |
| SH-G09 | Gemeinsamer Netzwerk-Symbolsatz | P1 | teilweise | network_devices.uis, dio stencils/network | liegt in `apps/JFDiagrammer/symbols/`; eigenes Repo + Submodule, JFNetworker noch auf eigenem `icons/` |

### 4.2 Übersicht aller Schablonen

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| SH-002 | Blöcke (`blocks`, 37) | P1 | fertig | dio stencils/shapes | – |
| SH-003 | Dekorative Shapes | P2 | fehlt | – | – |
| SH-004 | Diagramm- und Mathematik-Shapes | P2 | fehlt | – | – |
| SH-006 | Standard-Shapes (`standard`, 50) | P1 | fertig | stencils.uis, dio stencils | – (+ Pfeil, Textfeld) |
| SH-007 | Venn-Diagramm-Shapes | P2 | fehlt | – | – |
| SH-009 | Abteilung | P2 | fehlt | – | – |
| SH-015 | BPMN-Standard-Shapes | P2 | fehlt | – | – |
| SH-016 | Funktionsübergreifendes Flussdiagramm (`cff`, 4 + 3 verborgen) | P1 | fertig | structures_cff.uis, dio stencil_structs | – |
| SH-019 | Pfeil-Shapes (`arrows`, 20) | P1 | fertig | dio stencils/shapes | – |
| SH-021 | Standardflussdiagramm ISO 5807 (`flowchart`, 14) | P1 | fertig | dio stencils/shapes/flowchart, templates.uis | – |
| SH-023 | Workflowobjekte | P2 | fehlt | – | – |
| SH-025 | Workflowschritte | P2 | fehlt | – | – |
| SH-030 | Marketingdiagramme | P2 | fehlt | – | – |
| SH-031 | Matrixdiagramm-Shapes | P2 | fehlt | – | – |
| SH-032 | Pyramiden-Shapes | P2 | fehlt | – | – |
| SH-033 | Zyklusdiagramm-Shapes | P2 | fehlt | – | – |
| SH-036 | EPC-Diagramm-Shapes | P2 | fehlt | – | – |
| SH-041 | Ursache/Wirkung-Diagramme | P2 | fehlt | – | – |
| SH-049 | Karten – Organigramm-Shapes | P2 | fehlt | – | – |
| SH-061 | Grundlegende Elemente (Elektro) | P2 | fehlt | – | – |
| SH-063 | Halbleiter und Elektronenröhren | P2 | fehlt | – | – |
| SH-095 | AWS Allgemein | P2 | fehlt | – | – |
| SH-096 | AWS AR VR | P2 | fehlt | – | – |
| SH-097 | AWS Compute | P2 | fehlt | – | – |
| SH-098 | AWS Endbenutzer-Computing | P2 | fehlt | – | – |
| SH-099 | AWS Internet der Dinge | P2 | fehlt | – | – |
| SH-100 | AWS Machine Learning | P2 | fehlt | – | – |
| SH-101 | AWS Mobile | P2 | fehlt | – | – |
| SH-102 | AWS Networking und Inhaltsbereitstellung | P2 | fehlt | – | – |
| SH-103 | AWS Quantum-Technologien | P2 | fehlt | – | – |
| SH-104 | AWS-Analysen | P2 | fehlt | – | – |
| SH-105 | AWS-Anwendungsintegration | P2 | fehlt | – | – |
| SH-106 | AWS-Blockchain | P2 | fehlt | – | – |
| SH-107 | AWS-Container | P2 | fehlt | – | – |
| SH-108 | AWS-Datenbank | P2 | fehlt | – | – |
| SH-109 | AWS-Entwicklertools | P2 | fehlt | – | – |
| SH-110 | AWS-Konformität der Sicherheitsidentität | P2 | fehlt | – | – |
| SH-111 | AWS-Kostenmanagement | P2 | fehlt | – | – |
| SH-112 | AWS-Kundenbefähigung | P2 | fehlt | – | – |
| SH-113 | AWS-Kundenbindung | P2 | fehlt | – | – |
| SH-114 | AWS-Management Governance | P2 | fehlt | – | – |
| SH-115 | AWS-Mediendienste | P2 | fehlt | – | – |
| SH-116 | AWS-Migrationsübertragung | P2 | fehlt | – | – |
| SH-117 | AWS-Robotik | P2 | fehlt | – | – |
| SH-118 | AWS-Satellite | P2 | fehlt | – | – |
| SH-119 | AWS-Speicher | P2 | fehlt | – | – |
| SH-120 | AWS-Spieltechnologie | P2 | fehlt | – | – |
| SH-121 | AWS-Unternehmensanwendungen | P2 | fehlt | – | – |
| SH-140 | Computer und Monitore (`net_computers`, 11) | P1 | fertig | dio stencils/network | – |
| SH-142 | Detailliertes Netzwerkdiagramm (`net_detailed`, 15) | P1 | fertig | dio stencils/network | – |
| SH-145 | Freistehende Gestelle (`rack_freestanding`, 9; Titel „Freistehende Geräte“) | P1 | fertig | dio stencils/network | – |
| SH-146 | Gestellmontierte Geräte (`rack_equipment`, 19) | P1 | fertig | network_rack.uis, network_rack_dimension.uis; dit stencils/network (rack, rack_equipment/*/dimension) | – |
| SH-147 | Gestellmontierte Server (`rack_servers`, 17) | P1 | fertig | dio stencils/network | – |
| SH-151 | Netzwerk und Peripheriegeräte (`net_peripherals`, 31) | P1 | fertig | network_devices.uis, dio stencils/network | – |
| SH-153 | Netzwerkadressen (`net_locations`, 8; Titel „Netzwerkstandorte“) | P1 | fertig | dio stencils/network | – |
| SH-156 | Netzwerksymbole (`net_symbols`, 24) | P1 | fertig | dio stencils/network | – |
| SH-158 | Server (`net_servers`, 17) | P1 | fertig | dio stencils/network | – |
| SH-171 | Büromöbel | P2 | fehlt | – | – |
| SH-192 | Möbel | P2 | fehlt | – | – |
| SH-213 | Wände, Türen und Fenster | P2 | fehlt | – | – |
| SH-222 | Chen-Datenbanknotation | P2 | fehlt | – | – |
| SH-223 | Crow's Foot-Datenbanknotation | P2 | fehlt | – | – |
| SH-224 | Entitätsbeziehung | P2 | fehlt | – | – |
| SH-225 | IDEF1X-Datenbanknotation | P2 | fehlt | – | – |
| SH-226 | Objekt - relational | P2 | fehlt | – | – |
| SH-227 | UML-Datenbanknotation | P2 | fehlt | – | – |
| SH-251 | UML-Aktivität | P2 | fehlt | – | – |
| SH-252 | UML-Anwendungsfall | P2 | fehlt | – | – |
| SH-253 | UML-Klasse | P2 | fehlt | – | – |
| SH-254 | UML-Kommunikation | P2 | fehlt | – | – |
| SH-255 | UML-Komponente | P2 | fehlt | – | – |
| SH-256 | UML-Sequenz | P2 | fehlt | – | – |
| SH-257 | UML-Statuscomputer | P2 | fehlt | – | – |
| SH-258 | UML-Verteilung | P2 | fehlt | – | – |
| SH-266 | Zeitachsen-Shapes | P2 | fehlt | – | – |
| SH-267 | Anmerkungen | P2 | fehlt | – | – |
| SH-268 | Bemaßung - Technik | P2 | fehlt | – | – |
| SH-272 | Beschriftungen (`callouts`, 36 + Tipp) | P1 | teilweise | structures_callout.uis, callouts_auto_dimension.uis; dit stencils/structure | Benutzerdefinierte Beschriftung 1–3 (Konfigurations-Mechanik; JD31) |
| SH-274 | Symbole | P2 | fehlt | – | – |
| SH-276 | Titelblöcke | P2 | fehlt | – | – |
| SH-277 | Verbinder (`connectors`, 69) | P1 | fertig | dio stencils/shapes/1d | – (Verhalten: siehe Auffälligkeiten) |

### 4.14 Vorschläge für eigene Kategorien

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| SH-E01 | Kubernetes und Container-Plattformen | P2 | fehlt | – | → JD34 |
| SH-E02 | C4-Modell | P2 | fehlt | – | → JD34 |
| SH-E03 | Elektro-Hausinstallation | P2 | fehlt | – | → JD31 |
| SH-E05 | Smart Home und Gebäudeautomation | P2 | fehlt | – | → JD34 |
| SH-E08 | Flucht- und Rettungspläne | P2 | fehlt | – | → JD34 |
| SH-E13 | Netzwerk-Dokumentation im JFNetworker-Stil | P2 | fehlt | – | → JD34 |

## 5. Vorlagen, Diagrammtypen und Assistenten

### 5.1 Vorlagen-Infrastruktur

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| VL-01 | Vorlagenkatalog | P1 | fertig | templates.uis, network_templates.uis | – |
| VL-02 | Vorlagen-Inhalt | P1 | fertig | dio templates (402), templates.uis | – (Regelsätze werden mitgeschrieben, Prüfung erst JD24) |
| VL-03 | Eigene Vorlagen | P1 | fertig | templates_own.uis (`.vstx` öffnen = neue Zeichnung; Dateimanager über PACKAGE `filetypes`) | – |
| VL-04 | Vorlagenabhängige Register | P1 | fertig | templates.uis (CFF-Register mit der Vorlage) | – (übrige Register kommen mit P2/P3-Vorlagen) |

### 5.2 Vorlagen nach Kategorie

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| VL-10 | Leere Zeichnung | P1 | fertig | templates.uis | – |
| VL-11 | Standarddiagramm | P1 | teilweise | templates.uis, dio templates | Dekorative Shapes (SH-003, P2) in der Schablonenliste → JD26 |
| VL-12 | Blockdiagramm, Blockdiagramm mit Perspektive, 3D-Blockdiagramm | P2 | fehlt | – | → JD26 |
| VL-20 | Standardflussdiagramm | P1 | fertig | templates.uis, dio templates | – |
| VL-21 | Funktionsübergreifendes Flussdiagramm | P1 | fertig | templates.uis, structures_cff.uis | – |
| VL-22 | BPMN-Diagramm | P2 | fehlt | – | → JD27 |
| VL-23 | Arbeitsflussdiagramm (2D/3D) | P2 | fehlt | – | → JD27 |
| VL-30 | Organigramm, Organigramm-Assistent | P2 | fehlt | – | → JD28 |
| VL-32 | Ursache-Wirkung-Diagramm (Ishikawa) | P2 | fehlt | – | → JD26 |
| VL-33 | Auditdiagramm, EPK-Diagramm | P2 | fehlt | – | → JD27 |
| VL-38 | Diagramme und Grafiken, Marketingdiagramme | P2 | fehlt | – | → JD26 |
| VL-40 | Grundlagen Elektrotechnik, Schaltkreise und Logik, Industriesteuerungen, Elektrische Systeme | P2 | fehlt | – | → JD30 |
| VL-50 | Standardnetzwerk (2D/3D) | P1 | fertig | network_templates.uis, network_devices.uis, dio templates | – (3D-Form braucht P3-Schablonen) |
| VL-51 | Detailliertes Netzwerk (2D/3D) | P1 | teilweise | network_templates.uis, dio templates | Netzwerkberichte (DA-09 → JD19); Raumelemente (SH-155) und 3D sind P3 |
| VL-52 | Rack-Diagramm | P1 | fertig | network_templates.uis, network_rack.uis | – (Startdiagramm nur mit leerem Gestell) |
| VL-54 | AWS-Architektur | P2 | fehlt | – | → JD32 |
| VL-60 | Grundriss, Hausplan, Büroplan | P2 | fehlt | – | → JD31 |
| VL-61 | Elektro- und Telekommunikationsplan | P2 | fehlt | – | → JD31 |
| VL-70 | UML (Klasse, Sequenz, Anwendungsfall, Aktivität, Komponente, Verteilung, Zustand, Kommunikation) | P2 | fehlt | – | → JD29 |
| VL-71 | Datenbankmodell (Krähenfuß, Chen, IDEF1X, UML-Datenbank, objektrelational) | P2 | fehlt | – | → JD29 |

### 5.3 Assistenten und Generatoren

| ID | Feature | Prio | Stand | Beleg | Fehlt |
|---|---|---|---|---|---|
| AS-01 | Organigramm-Assistent | P2 | fehlt | – | → JD28 |
| AS-03 | Datenauswahl-Assistent | P2 | fertig | data_import_xlsx.uis, data_import_csv.uis (Quelle, Bereich, Spalten und Zeilen, Schlüssel) | – |
| AS-04 | Daten verknüpfen | P2 | fertig | data_link.uis (Seite oder Auswahl, mehrere Paare, ersetzen, Rückgängig) | – |
| AS-05 | Berichtsdefinition | P2 | fehlt | – | → JD19 |
| AS-11 | Shapes nummerieren, Array, Maßlinien | P2 | fehlt | – | → JD21 |
| AS-17 | Verhalten-Assistent (ohne Formeln) | P2 | fehlt | – | → JD23 |
| AS-20 | Verbundene Zeichnung aus Text | P2 | fertig | data_text_drawing.uis (Vorschau der Datensätze und Probleme als Tabelle, Trennzeichen und Einheit, Master-Zuordnung aus geöffneten Schablonen und Zeichnung, angezeigte oder neue Seite) | – |
