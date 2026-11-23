# JFDiagrammer – Plan für Phase 2: Codec und App

Stand 2026-11-14, auf Basis von `main` 757b83d8 und [FEATURES.md](FEATURES.md). Die IDs in
Klammern (DF-01, CN-05 …) verweisen auf die Feature-Liste. Aufwände sind geschätzt
(S = eine Session, M = zwei bis drei, L = mehr). Was noch nicht entschieden ist, steht unter
„Offene Fragen“.

**Erledigt** (2026-11-16, Branch `claude/jfdiagrammer-base`): JD0 Gerüst, JD1 ZIP und OPC
(`archive::opc`, DEFLATE, ZIP64, `Limits::ratio`), JD2 Dokumentmodell (`diagram_core`, Rückgängig
über inverse Schritte) und JD14 alle 303 Menüband-Icons.

**Erledigt** (2026-11-17, Welle C): JD3 VSDX lesen (`diagram_io::vsdx::read`, Test-App
`diagramiotest`, `build_rs.py test` führt die Unit-Tests der Crates aus), JD8 App-Hülle (Menüband mit allen
P1-Befehlen, 17 von 182 mit Handler; Backstage, Optionen, Sprachwechsel zur Laufzeit, Aufgabenbereiche;
`ui.ribbon_tabs`, `ui.backstage_begin`, `ui.settings_section` in der Bibliothek) und der Vektor-PDF-Schreiber
(`lib/pdf`, Test-App `pdftest`).

**Erledigt** (2026-11-18, Welle D, Meilenstein M1 Betrachter): JD4 Darstellung (Darstellungsliste in
`diagram_core::render`, Zeichnen in der neuen Crate `lib/diagram_view`, App öffnet Dateien schreibgeschützt),
JD5 VSDX schreiben (`diagram_io::vsdx::write`, neue Dokumente mit eigenem Design „NetLab“; Visio braucht
`windows.xml` in jeder Datei, sonst Fehler 271; Gegenprobe in Visio bestanden) und JD6 Formel-Engine
(`diagram_core::formula`; Orakel 94,7 % eigene Formeln, 98,1 % mit geerbten Zellen). Theme-Erweiterungen von
Visio: nur die Kennungen übernehmen, Werte aus NetLab (Entscheidung des Nutzers, umgesetzt in JD11).

**Erledigt** (2026-11-19, Welle E): JD7 Bearbeiten (Auswahl, Griffe, Drehen, Steuerpunkte, Kleben, Verbinder-Routing,
Text, Speichern, Rückgängig; Visio-Gegenprobe bestanden), JD9 Seitenleiste und Schablonen (Schablonen-Widget, `.vssx`,
Schnell-Shapes, Favoriten, Suche, Ablegen, Bauwerkzeug mit Startsatz „Standard-Shapes“; Visio-Gegenprobe bestanden)
und JD12 Export und Druck (`diagram_view::export` für PDF/SVG/PNG/JPEG, Verläufe und Schraffuren in `lib/pdf`; Druck unter
Linux als Vektor-PDF, unter Windows als Raster mit 300 dpi). Offen: `formulas/growth` in diagramiotest schwankt unter
Volllast (4-fache Shapes, 6,5- bis 17,8-fache Zeit); geklärt als Last- und Speicherrauschen, der Check ist jetzt robust.

**Erledigt** (2026-11-19, Welle F, Meilenstein M2 Editor): JD10 Strukturen (Container, Listen, Swimlanes mit
funktionsübergreifendem Flussdiagramm, Beschriftungen, Verbindungspfeile, Layout-Verhalten; Visio erkennt die
Mitgliedschaft über `Relationships`/`DEPENDSON`) und JD11 Designs und Format (eigene Designs NetLab, Kontur und
Kontrast mit je vier Varianten, vom Nutzer abgenommen; Galerie mit Live-Vorschau, Shape formatieren, Farbwähler,
Format übertragen, Schnellformatvorlagen). Offen: Klick auf einen Verbindungspfeil über einem vorhandenen Shape
wirkt nicht (in Welle G behoben); Visio-Gegenprobe von JD10.

**Erledigt** (2026-11-20, Welle G, Meilenstein M3 P1 komplett): JD13 Vorlagen und Startsatz (Katalog unter
Datei › Neu mit gerenderten Vorschauen; Vorlagen als `.vstx`, gebaut aus Textquellen in `templates/`: leere
Zeichnung, Standarddiagramm, Standardflussdiagramm, funktionsübergreifendes Flussdiagramm, Standard- und
detailliertes Netzwerk, Rack-Diagramm; eigene Vorlagen; Schablonen Blöcke, Pfeile, ISO 5807, Verbinder, sechs
Netzwerk- und drei Rack-Schablonen mit Einrasten auf Höheneinheiten; der gemeinsame Symbolsatz mit JFNetworker
liegt in `symbols/` und wird später ein eigenes Repository, das beide Apps als Submodule einbinden) und der Rest
von JD8 (223 von 235 Befehlen mit Funktion, die 12 übrigen sind P2 und stehen mit Begründung in
`src/cmds_later.rs`; Aufgabenbereiche, Schnellzugriff, Ebenen, Seitenbefehle; Klickfehler bei den
Verbindungspfeilen behoben). Offen: Visio-Gegenproben von JD10 und JD13, Bemaßung der Rack-Schablone (das
Bauwerkzeug schreibt noch keine Textfelder), Vektordruck unter Windows. Weiter mit Phase 2b und Phase 3.

## Ziel

Am Ende von Phase 2 öffnet, bearbeitet und speichert JFDiagrammer `.vsdx`- und `.vssx`-Dateien
mit allen **P1-Funktionen** aus FEATURES.md, Abschnitte 1–3 und 5.1. Die App zeigt den NetLab-Look
mit Menüband oben und läuft in fünf Sprachen. Die Codecs liegen als zwei neue Crates in JFLib.
Ein Startsatz an eigenen Shapes reicht, um alle Mechaniken zu prüfen.

Nicht Teil von Phase 2:
- P2- und P3-Funktionen, außer sie fallen nebenbei ab
- die älteren XML-Formate `.vsx`/`.vdx` und die Binärformate `.vss`/`.vsd` (Phase 2b, siehe unten)
- Datenverknüpfung und Assistenten
- der Nachbau aller 282 Schablonen (Phase 3)

## Ausgangslage

Vorhanden (Details in FEATURES.md 0.4):

- **Canvas:** [`lib/ui/diagram.rs`](../../lib/ui/diagram.rs) bringt Zoom, Verschieben, Raster,
  Auswahl, Ports, Kanten mit Wegpunkten, Übersichtskarte und Bildschirmleser-Semantik mit. Es fehlen
  Drehen, acht Griffe, Steuerpunkte, Kleben mit Richtung, Routing um Hindernisse, Ebenen, Lineale,
  Führungslinien und Text auf Shapes.
- **Menüband:** [`lib/ui/ribbon.rs`](../../lib/ui/ribbon.rs) liefert Gruppen, Knöpfe, Galerien und
  Einklappen. Registerleiste, Backstage und Lineal gibt es bisher nur in jfWord
  (`ribbon_tabs.rs`, `shell_ui.rs`, `ruler_ui.rs`).
- **Weitere UI-Bausteine:** `dock.rs`, `history.rs` (Rückgängig), `rich_edit.rs`, `color.rs`,
  `fontpick.rs`, `filedlg.rs`, `recent.rs`, `settings_page.rs` und i18n mit Umschalten ohne Neustart.
- **ZIP:** `lib/archive` liest ZIP mit DEFLATE, schreibt aber nur unkomprimiert. Die Grenzen
  (512 Einträge, 8 MiB je Teil, 32 MiB gesamt, kein ZIP64) sind zu eng für große Zeichnungen.
- **OPC:** `lib/doc_io/package.rs` und `preservation.rs` behandeln Beziehungen, Inhaltstypen und
  das Erhalten unbekannter Teile, allerdings nur für DOCX.
- **XML:** `lib/base/xml.rs`.
- **Grafik:** `lib/img_svg` liest SVG und `lib/render2d` zeichnet Pfade und, seit G6c, Text.
  SVG schreiben kann die Bibliothek nicht.
- **PDF:** Gedruckt wird nur als Rasterbild je Seite; einen Vektor-PDF-Schreiber gibt es nicht.
- **JFNetworker:** Die Shape-Seitenleiste steckt in der App selbst
  (`JFNetworker/src/workspace/palette.rs`), nicht in der Bibliothek.

Die Beispieldateien (FEATURES.md 1.6) legen fest, was der Codec in P1 mindestens verstehen muss:
- 13 ShapeSheet-Abschnitte und rund 330 Zellnamen
- 10 Arten von Geometriezeilen
- 60 Formelfunktionen
- Container über `User.msvStructureType`
- DrawingML-Themes mit Visio-Erweiterungen

## Entscheidungen

Vom Nutzer entschieden (FEATURES.md 0.1 und 8):

- **Zwei Crates** (F1): `lib/diagram_core` (Modell, ShapeSheet, Formeln, Geometrie, Routing,
  Layout, Themes) und `lib/diagram_io` (VSDX-Familie, später VDX/VSX/VSS, Exporte). Wie alle
  Bibliotheks-Crates ohne `std` (nur `alloc`). Die App darf `std` nutzen.
- **App** in `apps/JFDiagrammer`, nur gegen die öffentliche `jflib`-API. Später Umzug nach
  JFLibrary/JFDiagrammer, wie bei JFNetworker. Kein Submodul.
- **Look:** NetLab-Style, eckig, Menüband oben, Aufbau wie Visio und jfWord, nicht der JFOffice-Look.
  Die Icons im Menüband werden größtenteils neu im NetLab-Stil gezeichnet.
- **Sprachen:** DE, EN, ES, IT, FR, umschaltbar in den Einstellungen. Die Tastenkürzel folgen der
  Office-Belegung der jeweiligen Sprache (F10).
- **Inhalte:** Keine Visio-Schablonen, -Themes oder -Grafiken mitliefern. Genormte Symbole dürfen
  normgerecht aussehen (F2). AWS-Icons werden unverändert mitgeliefert (CC BY-ND 2.0), Azure-Icons
  lädt der Nutzer selbst herunter.
- **Formate:** `.vss` erst nach allen XML-Formaten (F4), nur DXF und kein DWG (F5).

Architektur für Phase 2 (vom Nutzer am 2026-11-14 übernommen):

- **V1 – Das ShapeSheet ist das Modell.** `diagram_core` speichert jede Zelle so, wie sie in der
  Datei steht: Formel, zwischengespeicherter Wert, Einheit und Herkunft (lokal, vom Master oder
  von der Formatvorlage geerbt, gelöscht). Shape, Seite und Dokument sind Sichten darauf. Damit
  geht beim Rundlauf durch die Datei nichts verloren (DF-03), und Verhalten aus Visio-Dateien
  funktioniert ohne Übersetzung.
- **V2 – Eigene Schablonen sind `.vssx`.** Die mitgelieferten Schablonen entstehen beim Bauen
  aus Quelldateien im Repo: je Master eine Textbeschreibung (Geometrie, Formeln, Steuerpunkte,
  Verbindungspunkte, Namen in fünf Sprachen, metrisch/US) und für gestaltete Shapes SVG-Grafik
  im JFNetworker-Stil. Ein Werkzeug schreibt daraus `.vssx`. So nutzen eigene und fremde Shapes
  denselben Code.
- **V3 – Eigene Standard-Designs.** Neue Zeichnungen bekommen eigene Designs (DrawingML-Themes
  mit Visio-Erweiterungen) im NetLab-Stil, an erster Stelle „NetLab“. Visios Designs werden nur
  angezeigt, wenn sie in einer Datei stecken.
- **V4 – Generischer OPC-Teil in `lib/archive`.** Paket, Beziehungen, Inhaltstypen und das
  Erhalten unbekannter Teile wandern aus `doc_io` in ein Modul `archive::opc`. DOCX und VSDX
  nutzen es gemeinsam.
- **V5 – Vektor-PDF als eigenes Bibliotheksmodul.** Unterstützt Text, Hyperlinks und Lesezeichen.
  jfWord kann es später mitnutzen.

## Meilensteine

### JD0: Gerüst (S)

- `lib/diagram_core` und `lib/diagram_io` anlegen: Cargo-Einträge, `jflib`-Features, Lizenzen,
  README, CHANGELOG, Einträge in API.md.
- App-Gerüst: `Cargo.toml`, `PACKAGE` (`org.jfreund.JFDiagrammer`), `STYLE` = `netlab`,
  `TARGETS`, `locales/ui.tsv` mit fünf Sprachspalten.
- Leeres Hauptfenster mit Platzhaltern für Menüband, Seitenleiste, Zeichenfläche und Statusleiste.
- Erstes `.uis`-Skript.
- **Fertig, wenn:** alle vier Ziele bauen (Windows, Linux, wasm32, Android) und das Fenster sich
  öffnet.

### JD1: ZIP und OPC (M)

- `lib/archive` bekommt:
  - DEFLATE beim Schreiben (`img_core::deflate` einhängen)
  - einstellbare Grenzen für große Zeichnungen
  - ZIP64 beim Lesen
  - feindliche Testfälle (Bombe, Namen, Tiefe) nach dem Muster von `codectest/hostile.rs` und
    `limits.rs`
- `archive::opc` nach V4.
- `doc_io` stellt auf das neue Modul um, jfWord bleibt grün.
- Hängt ab von: JD0. (DF-01, DF-02, DF-15)

### JD2: Dokumentmodell (L)

- `diagram_core` bekommt:
  - Dokument, Vorder- und Hintergrundseiten, Ebenen, Master, Shapes, Gruppen,
    Formatvorlagen, Theme-Daten
  - ShapeSheet-Speicher nach V1: Abschnitte, Zeilen, Zellen, gelöschte geerbte Zeilen (`Del="1"`)
  - Vererbungskette Shape → Master-Shape → Formatvorlage → Dokument
  - NameU und Name
  - Einheiten (intern Zoll wie in der Datei) und Hyperlinks
  - Shape-Daten und User-Zellen
- Eine Änderungs-API mit Transaktionen, sodass Rückgängig über `history.rs` geht.
- Hängt ab von: JD0. (DM-01–05, PG-01–07, LY-01–04, MS-01/02/10, SP-01, SS-01–45 soweit P1,
  DA-01, HL-01, TH-01)

### JD3: VSDX lesen (L)

- `diagram_io` liest alle Teile der VSDX-Familie ins Modell:
  - `document.xml`: Formatvorlagen, Dokument-ShapeSheet, Farben, Schriften
  - Seiten mit Shapes und `Connect`-Einträgen
  - Master, Themes, `windows.xml`, Eigenschaften und Medien
- Kommentare und Datensätze bleiben erhalten (DF-11, DF-12).
- `.vssx`, `.vssm`, `.vsdm` und `.vstx` werden erkannt; den Makro-Teil reicht die App unverändert
  durch und führt ihn nie aus.
- **Fertig, wenn:** die lokalen Dateien (`Zeichnung2`, `Zeichnung 3`, `Favoriten`) und eigene
  Testdateien ohne Fehler laden und die feindlichen Tests grün sind.
- Hängt ab von: JD1, JD2. (DF-01, DF-03, DF-06, DF-14, DF-15)

### JD4: Darstellung, erster Meilenstein „Betrachter“ (L)

- Geometrie: alle Zeilenarten aus den Beispieldateien, außerdem
  - Transformation (Pin, LocPin, Winkel, Spiegeln)
  - Gruppen
  - Linien mit Mustern und Linienenden
  - einfarbige Füllung, Grundform der Verläufe
  - Textblöcke mit Zeichen- und Absatzläufen und Drehverhalten (TX-04)
  - Fremdbilder
  - Theme-Auflösung
- Der Betrachter zeigt zunächst die **zwischengespeicherten Werte** aus der Datei; die
  Formel-Engine (JD6) ist dafür nicht nötig.
- Die App öffnet Dateien schreibgeschützt, mit Seitenregistern, Zoom und Verschieben.
- **Fertig, wenn:** die lokalen Dateien im Vergleich mit SVG-Exporten aus Visio (siehe Testen)
  innerhalb der Toleranz stimmen.
- Hängt ab von: JD3. (SP-02, SP-03, FM-01–03, TX-01–04, ZF-01, ZF-08, ZF-14)

### JD5: VSDX schreiben (M)

- Das Modell wird mit allem Unbekannten zurückgeschrieben.
- Neue Dokumente entstehen von Grund auf mit dem eigenen Standard-Design (V3).
- `.vssx` schreiben (EX-11, MS-06).
- **Fertig, wenn:**
  - der Rundlauf Lesen → Schreiben → Lesen für alle Testdateien dasselbe Modell ergibt und
  - der Nutzer je eine neue Zeichnung und Schablone in Visio öffnen kann.
- Hängt ab von: JD3. (DF-02, DF-03, DF-06)

### JD6: Formel-Engine (L)

- Parser für die universelle Schreibweise.
- Auswertung mit Abhängigkeitsgraph und inkrementeller Neuberechnung, Einheiten, Vererbung und
  Schutz (`GUARD`, Sperren).
- Funktionen:
  - alle P1-Funktionen aus FX-06 bis FX-09
  - die 60 Funktionen aus den Beispieldateien (`THEMEVAL`, `SETF`, `SETATREF`, `NURBS`,
    `POLYLINE` …)
  - `RUNADDON` und `DOCMD` nur, soweit sie ohne Fremdcode auskommen (FX-14, FX-15)
- Testorakel: Jede Zelle der lokalen Dateien wird neu berechnet und mit dem
  zwischengespeicherten Wert verglichen.
- Hängt ab von: JD2; die Korpus-Tests brauchen JD3. Läuft parallel zu JD4. (FX-01–09)

### JD7: Bearbeiten auf der Zeichenfläche (L)

- In `lib/ui`, als Erweiterung von `diagram.rs` oder als darauf aufbauendes Widget:
  - acht Griffe, Drehen, gelbe Steuerpunkte, Scheitelpunkte (P1-Teil)
  - 1D-Endpunkte
  - statisches und dynamisches Kleben an Verbindungspunkten
  - dynamischer Verbinder mit rechtwinkligem Routing um Hindernisse, Linienübergänge in P2
  - Gruppen und Stapelreihenfolge
  - Führungslinien, dynamisches Raster mit Ausrichtungshilfen
  - Lineale (aus jfWord in die Bibliothek), Seitenfläche und Seitenumbrüche
  - Text auf Shapes (`rich_edit.rs`), Livedynamik, Lupe
- Jede Geste ändert Zellen im Modell und löst die Neuberechnung aus (JD6).
- Hängt ab von: JD4, JD6. (SP-03–07, SP-10, CN-01–05, CN-12, CN-15, ZF-02–10, ZF-18, ZF-22)

### JD8: App-Hülle (L)

Die Hülle mit Platzhaltern beginnt schon nach JD0; die Befehle werden angeschlossen, sobald die
Funktionen dahinter fertig sind.

- **Menüband im NetLab-Look:** alle P1-Befehle der Register Start, Einfügen, Entwurf und Ansicht
  sowie die Kontextregister für Container und Text.
- **Weitere Fensterteile:** Schnellzugriff, Kontextmenüs, Statusleiste.
- **Backstage:**
  - Neu, mit Vorlagen-Vorschau
  - Öffnen, Speichern, Drucken, Exportieren
  - Optionen, darunter die Sprache
- **Aufgabenbereiche:** Shape formatieren, Größe und Position, Shape-Daten, Verschieben und Zoomen.
- **Sprachen:** Tastenkürzel je Sprache, Begriffsglossar und Prüfskript für die fünf Sprachen
  (LG-01–08, LG-11).
- Hängt ab von: JD0 (Hülle) und JD7 (Anschließen der Befehle). (UI-01–37, RS/RE/RW/RA/RK P1,
  AB-01–06, SB-01–04, KB-01–16, LG-*)

### JD9: Seitenleiste und Schablonen (M)

- Ein generisches Schablonen-Widget in `lib/ui`. Es wird neu geschrieben und lehnt sich nur an
  die Idee der JFNetworker-Seitenleiste an.
- Eine Schablone ist eine `.vssx`-Datei.
- Funktionen:
  - Schnell-Shapes, Favoriten und „Meine Shapes“
  - Suche über die Namen in allen fünf Sprachen
  - „Weitere Shapes“ und Leerzustand
- Bauwerkzeug für die mitgelieferten Schablonen nach V2.
- Hängt ab von: JD2, JD5. (SL-01–09, MS-04, MS-06, LG-04)

### JD10: Strukturen und Verbinder-Komfort (M)

- Container, Listen und Swimlanes, dazu das funktionsübergreifende Flussdiagramm.
- Beschriftungen (Callouts).
- „Verbindungspfeile“ (Platzhalter für den Begriff „AutoVerbinden“, CN-07, ZF-10) und
  „Shapes verbinden“.
- Layout-Verhalten je Shape, Seiteneinstellungen für Layout und Routing.
- Hängt ab von: JD7, JD9. (ST-01/02/04–06, CN-07, CN-16, CN-17, PG-11, RK-01, RK-03)

### JD11: Designs und Format (M)

- Designs anwenden, Varianten, eigene Designs nach V3.
- Dialoge für Linie und Füllung, Farbwähler, Format übertragen.
- Hängt ab von: JD6, JD8. (TH-01–04, FM-01–03, FM-07, FM-08, RW-05, RW-06)

### JD12: Export und Druck (M)

- SVG und PNG schreiben, Vektor-PDF nach V5.
- Drucken mit Vorschau, Druckbereich und Kacheln; „Seite einrichten“.
- Kopieren in die Zwischenablage als SVG/PNG.
- Der PDF-Schreiber kann jederzeit parallel entstehen; der Rest hängt an JD4.
- (EX-01, EX-03, EX-04, EX-14, EX-15, PR-01–09)

### JD13: Vorlagen und Startsatz an Shapes (M)

- Vorlagenkatalog (VL-01–04).
- Die P1-Vorlagen: leere Zeichnung, Standarddiagramm, Standardflussdiagramm,
  funktionsübergreifendes Flussdiagramm, Standard- und detailliertes Netzwerk, Rack (F11).
- Dazu die nötigen Schablonen im JFNetworker-Stil:
  - Standard-Shapes
  - Flussdiagramm nach ISO 5807
  - Netzwerk mit gemeinsamem Symbolsatz (F9)
  - Rack in Höheneinheiten
- AWS (F11) folgt am Ende von JD13 oder als Erstes in Phase 3.
- Hängt ab von: JD9, JD10, JD11.

### JD14: Icons im Menüband (M, parallel ab Beginn)

- Liste aus FEATURES.md Abschnitt 7.
- Zuerst ein Probeblatt mit etwa zwölf Icons im NetLab-Stil, das der Nutzer abnimmt; danach der
  Rest.
- Quellen als SVG im App-Ordner.
- Hängt ab von: nichts außer der Abnahme des Probeblatts.

### Phase 2b: die P2-Funktionen (JD15–JD25)

Stand 2026-11-23: In FEATURES.md stehen außerhalb der Schablonen rund 180 P2-Zeilen. Einige davon
sind schon da, weil frühere Etappen sie mitgenommen haben (z. B. `.vstx` öffnen, JPEG, Textfelder,
Shape-Daten definieren, Schnellzugriff anpassen). JD15 hält das fest, bevor der Rest beginnt.

#### JD15: Bestandsaufnahme und P1-Abnahme (M)

- Jede P1- und P2-Zeile aus FEATURES.md bekommt einen Stand (fertig, teilweise, fehlt), geprüft
  in der App und nicht nur im Code. Die Liste liegt als `STATUS.md` neben FEATURES.md, spätere
  Etappen tragen dort nach.
- Kleine P1-Lücken schließen, große als eigene Punkte auflisten.
- Offene Punkte aus Phase 2: die Bemaßung der Rack-Schablone (das Bauwerkzeug schreibt noch keine
  Textfelder), „Neue Seite“ hinter der aktuellen Seite oder am Ende (Entscheidung des Nutzers).

#### JD16: Ältere XML-Formate und Makro-Teile (M)

- `.vsx` und `.vdx` lesen (DF-20, DF-21), ein eigener Leser für das XML von 2003–2010 auf das
  vorhandene Modell.
- `.vsdm` und `.vssm`: den Makro-Teil unverändert durchreichen, nie ausführen (DF-04, DF-07).
- Dokumenteigenschaften und Fensterzustand (DF-09, DF-10), metrische und US-Varianten (MS-07).

#### JD17: Binäre Schablonen `.vss` importieren (L)

- DF-30 mit dem lokalen Korpus `context/files/SWDB_DE` (DF-34), nur Import.
- Es gibt keine offene Spezifikation. libvisio darf gelesen, aber nicht übernommen werden.
- Tests laufen nur, wenn der Korpus da ist; im Repo liegen keine Microsoft-Dateien.

#### JD18: Datenquellen und Verknüpfung (L)

- Ein XLSX-Leser in der Bibliothek (IM-10; JFLib hat noch keinen) und ein CSV-Leser.
- Daten importieren und mit Shapes verknüpfen, aktualisieren (IM-05, DA-04, DA-05, AS-03, AS-04,
  DF-12), Register „Daten“ mit externen Daten (RD-02, RD-03, RD-09, RD-10, AB-11).
- Verbundene Zeichnung aus CSV/Text (IM-08, AS-20), Shape-Daten beim Ablegen abfragen (SP-13).

#### JD19: Datengrafiken und Berichte (L)

- Datengrafiken mit ihren Feldern und der Legende (DA-07, DA-08, ST-08, RD-04, RD-05, RD-07, AB-12).
- Shape-Berichte samt Definition, Aktualisieren und Export (DA-09, DA-13, AS-05, EX-12, EX-16, RU-06).
- Hängt ab von: JD18.

#### JD20: Kommentare, Prüfen und Sprache (M)

- Kommentare mit Teil, Bereich und Knopf (HL-04, DF-11, AB-10, UI-33, RU-05).
- QuickInfo am Shape, Hyperlinkbasis, Link-Verlauf (HL-02, HL-03, HL-06).
- Rechtschreibprüfung je Sprache über die vorhandene Prüfung der Bibliothek (TX-07, LG-09, RU-01,
  RU-04, SB-03), Barrierefreiheit (RU-03, UI-17), Optionen der Dokumentprüfung (UI-14).

#### JD21: Verbinder-Automatik und Shape-Operationen (L)

- Linienübergänge (CN-06, bisher grau), automatisch einfügen und löschen, Verbinderwerkzeug
  (CN-08 bis CN-11), Seite neu layouten mit Auto-Ausrichten (CN-13, CN-14, RW-09).
- Shape-Operationen: Vereinigen, Kombinieren, Fragmentieren, Schneiden, Subtrahieren (SP-08);
  zu prüfen, ob die Booleschen Operationen von `lib/geom` passen.
- Shape ändern, Gruppe öffnen, Ausrichtungsrahmen, Grenzfälle (SP-11, SP-15 bis SP-17).
- Randmitglieder, zählende Legende (ST-03, ST-09); Shapes nummerieren, Array, Maßlinien (AS-11).

#### JD22: Seiten, Format und Ansicht (M)

- Seitenfarbe, Hintergründe, Rahmen und Titel als Galerien, Seitengröße am Rand ziehen (PG-08 bis
  PG-14, RW-07, RW-08).
- Schatten, Format wiederholen, Tabstopps und Textlineal (FM-04, FM-09, KB-18, TX-10, SS-08).
- Die Untermenüs der Varianten: Farben, Effekte, Verbinder, Schriftarten, Verzierung (TH-05, TH-06;
  bisher grau).
- Ansicht: Minisymbolleiste, Text-Zoom, Präsentationsmodus, Stempel, Zeichnung zentrieren,
  Nullpunkt (ZF-11, ZF-16, ZF-17, ZF-19 bis ZF-21); Navigation, Zeichnungsexplorer, Ebenen-Bereich
  (AB-07 bis AB-09, RA-01, RA-04, LY-05); Kopf- und Fußzeile beim Drucken (PR-04).

#### JD23: Entwicklertools ohne VBA (L)

- ShapeSheet-Fenster und Formel-Editor (SS-46, FX-16), die übrigen P2-Abschnitte (SS-05, SS-12,
  SS-13, SS-16, SS-21, SS-22, SS-26, SS-31, SS-33, SS-36, SS-42 bis SS-44) und Funktionen (FX-10,
  FX-11, FX-12, FX-14).
- Shape-Design, Schablonen und Master bearbeiten, Dokumentschablone, Formatvorlagen, Muster
  (RX-03 bis RX-09, RK-08, RK-10, SL-08, SL-10, MS-05, MS-08, MS-11; die Befehle „Schablone
  bearbeiten“, „Neues Master-Shape“, „Reihenfolge der Schablonen“ sind bisher grau).
- Verhalten-Assistent ohne Formeln (AS-17), Tastenkürzel der Entwicklertools (KB-17),
  übersetzte Formelnamen (LG-10).

#### JD24: Prozess und Diagrammprüfung (M)

- Teilprozesse (RP-01, RP-02), Diagrammüberprüfung mit Regelsätzen und Bereich „Probleme“
  (RP-04 bis RP-06, AB-13). Die Vorlagen aus JD13 bringen schon Regelsätze mit.

#### JD25: Dokument, Fenster, Export und Import (M)

- Dateigröße verringern, persönliche Informationen entfernen, automatisch wiederherstellen,
  Dokumentschutz (DM-06, DM-07, DM-10, DM-11).
- Informationen, Befehlssuche, Livevorschau, Hilfe, Fenster-Tastenkürzel (UI-07, UI-32, UI-35,
  RH-01, KB-14), mehrsprachige Shape-Suche (LG-05), Bildtools (RK-02), die restlichen P2-Befehle
  der Register Start und Einfügen.
- EMF und Webseite exportieren, SVG einfügen (EX-07, EX-09, IM-02).
- Vektordruck unter Windows (bisher Raster mit 300 dpi), falls dafür eine Druckschicht in `plat`
  entsteht.

### Phase 3: Shapes und Vorlagen P2 (JD26–JD34)

65 Schablonen mit rund 1 200 Shapes und 13 Vorlagen haben Priorität P2. Alle Shapes entstehen neu im
JFNetworker-Stil über das Bauwerkzeug, je Schablone erst ein Probeblatt für den Nutzer.

| Etappe | Inhalt | Shapes | Vorlagen |
|---|---|---|---|
| JD26 Allgemein und Geschäftsgrafiken (M) | SH-003, SH-004, SH-007, SH-030 bis SH-033, SH-041 | ~150 | VL-12, VL-32, VL-38 |
| JD27 Prozessnotationen (M) | BPMN SH-015 (Typwahl per Kontextmenü), EPK SH-036, Workflow SH-009, SH-023, SH-025 | ~110 | VL-22, VL-23, VL-33 |
| JD28 Organigramm (M) | SH-049, Ablegen auf Vorgesetzte, Layouts, Assistent (AS-01, DA-11, RK-04); hängt an JD18 | ~10 | VL-30 |
| JD29 Software und Datenbank (M) | UML SH-251 bis SH-258, Datenbank SH-222 bis SH-227 | ~130 | VL-70, VL-71 |
| JD30 Elektrotechnik (M) | IEC 60617 in der DIN-EN-Fassung (F14): SH-061, SH-063 | ~90 | VL-40 |
| JD31 Pläne, Bemaßung, Titelblöcke (L) | SH-171, SH-192, SH-213, SH-267, SH-268, SH-274, SH-276, eigene SH-E03; Maßstab und Bemaßung | ~200 | VL-60, VL-61 |
| JD32 AWS-Architektur (M) | SH-095 bis SH-121 als Originale (CC BY-ND 2.0, unverändert, mit Namensnennung), per Generator aus dem offiziellen Paket | ~530 | VL-54 |
| JD33 Zeitachse (S) | SH-266 mit der Mechanik der Zeitachse | ~20 | – |
| JD34 Eigene Kategorien (M) | SH-E01 Kubernetes, SH-E02 C4, SH-E05 KNX, SH-E08 Flucht- und Rettungspläne, SH-E13 Netzwerk-Dokumentation | offen | – |

Die P3-Schablonen (rund 180) und P3-Vorlagen kommen danach in einem eigenen Plan.

### Umzug

Sobald `diagram_core` und `diagram_io` mit einem jflib-Release auf crates.io sind, zieht die App nach
JFLibrary/JFDiagrammer. Der gemeinsame Netzwerk-Symbolsatz wird dann ein eigenes Repository, das
JFNetworker und JFDiagrammer als Submodule einbinden (`symbols/README.md`).

## Reihenfolge

Gearbeitet wird in Sessions mit eigenem Worktree, je Etappe eine, höchstens drei gleichzeitig.
Gemergt wird nach grünen gezielten Prüfungen, gepusht nach Rückfrage.

| Welle | Etappen | Ergebnis |
|---|---|---|
| A | JD0 | Gerüst |
| B | JD1, JD2, JD14 (Probeblatt) | ZIP/OPC, Modell, Icon-Abnahme |
| C | JD3, JD8 (Hülle mit Platzhaltern), PDF-Schreiber aus JD12 | Dateien laden |
| D | JD4, JD5, JD6 | **M1 Betrachter**: Visio-Dateien richtig anzeigen; Rundlauf |
| E | JD7, JD9, JD14 (Rest) | **M2 Editor**: zeichnen, verbinden, speichern; Visio öffnet es |
| F | JD10, JD11, JD12 (Rest) | Strukturen, Designs, Export |
| G | JD13, JD8 (Rest) | **M3 P1 komplett** |
| H | JD15, JD16, JD26 | Bestand, ältere XML-Formate, Geschäftsgrafiken |
| I | JD17, JD18, JD27 | `.vss`, Daten, Prozessnotationen |
| J | JD19, JD21, JD29 | Datengrafiken, Verbinder-Automatik, UML |
| K | JD20, JD22, JD28 | Kommentare, Seiten und Ansicht, Organigramm |
| L | JD23, JD24, JD30 | Entwicklertools, Prozess, Elektrotechnik |
| M | JD25, JD31, JD32 | Dokument und Export, Pläne, AWS |
| N | JD33, JD34 | **M4 P2 komplett** |

## Fallen

- **Beim Öffnen nichts neu berechnen.** Verbinder, Layout und Container werden erst neu
  geroutet, wenn sich etwas bewegt. Sonst sieht jede Visio-Datei nach dem Öffnen anders aus und
  ändert sich beim Speichern.
- **Formel und Wert gehören zusammen.** Eine Formel mit unbekannter Funktion bleibt samt
  gespeichertem Wert erhalten und wird markiert, sie erzeugt keinen Fehler. Eigene Werte werden
  nie ohne passende Formel geschrieben.
- **Koordinaten:** In der Datei stehen alle Längen in Zoll, die Anzeigeeinheit steht im Attribut
  `U`. Die y-Achse zeigt nach oben, der Ursprung liegt unten links.
- **Drehen und Spiegeln in Gruppen:** Die Transformationen müssen in Visios Reihenfolge
  zusammengesetzt werden.
- **Schriften:** Calibri, Segoe UI und andere fehlen oder sind nicht lizenziert. Der Schriftersatz
  (TX-09) ändert die Textbreiten, deshalb vergleichen die Tests mit Toleranz.
- **Fremdinhalte:** Master und Themes in Nutzerdateien sind meist Microsoft-Inhalt. Anzeigen ist
  in Ordnung, aber nie in eigene Schablonen übernehmen und nie committen. Nichts aus `context/`
  kommt ins Repo.
- **NetLab-Style ändern:** JFNetworker nutzt denselben Style. Änderungen für das Menüband dürfen
  dort nichts verschieben.
- **Vier Ziele:** Neue Crates und `lib/ui`-Änderungen müssen auch für wasm32 und Android bauen;
  `test --quick` prüft das nicht.
- **Große Zeichnungen:** Tausende Shapes, deshalb rechnet die Engine inkrementell und das Canvas
  nutzt den räumlichen Index.
- **Fremde Symbole:** AWS-Icons nur unverändert und mit Namensnennung (CC BY-ND 2.0); das offizielle
  Paket wird erst nach dem Ja des Nutzers geladen. Für die ISO-7010-Zeichen (SH-274, SH-E08) und die
  Kubernetes-Icons (SH-E01) die Rechte vor dem Zeichnen klären.
- **Keine App-Assets in der Bibliothek:** JFLib ist öffentlich. Geteilte Assets der eigenen Apps
  (Symbolsätze) kommen in eigene Repositorys, nicht nach `lib/`.

## Testen

- **Test-App für die Codecs** nach dem Muster von `codectest`, mit Rundlauf, feindlichen Dateien,
  Grenzen und Formelorakel. `diagramtest` wird für das Canvas erweitert, und die App bekommt
  eigene `.uis`-Skripte je Funktion.
- **Testdateien:**
  - Im Repo liegen nur selbst erzeugte Dateien.
  - Der lokale Korpus in `context/files` (Z2, Z3, Favoriten, SWDB) wird nur genutzt, wenn er da
    ist; sonst werden die Tests übersprungen.
- **Referenzbilder:** Der Nutzer exportiert die lokalen Dateien in Visio als SVG (nur lokal).
  Die Tests vergleichen Geometrie und Position mit Toleranz.
- **Visio-Gegenprobe:** Je Meilenstein öffnet der Nutzer einige von JFDiagrammer geschriebene
  Dateien in Visio.
- **Dauer:** Je Etappe nur gezielte Prüfungen, eine volle Suite je gepushtem Stand. Echte Fenster
  nur in Maßen, keine Wiederholungsschleifen.

## Offene Fragen

1. Kann der Nutzer die Referenz-Exporte und die Visio-Gegenprobe machen, hat er also Visio zur Hand?
2. Welcher deutsche Begriff ersetzt „AutoVerbinden“? Platzhalter: „Verbindungspfeile“.
3. Soll „Neue Seite“ wie Visio am Ende anhängen? Bisher fügt sie hinter der aktuellen Seite ein.
