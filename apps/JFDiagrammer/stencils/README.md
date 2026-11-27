# Mitgelieferte Schablonen – Quellformat

Die Schablonen von JFDiagrammer entstehen beim Bauen aus den Textdateien in diesem Ordner
(PLAN.md, Entscheidung V2). Das Build-Skript der App (`build_steps.rs`, aufgerufen von `build.rs` und
von den abgeleiteten Builds für Browser und Android) übersetzt jede Schablone mit
`diagram_io::stencil` in zwei `.vssx`-Dateien, eine metrische und eine US-Fassung (MS-07), und
bettet sie in das Programm ein. Eigene und fremde Shapes gehen danach durch denselben Code: die
Seitenleiste liest die eingebetteten Dateien mit `vsdx::read` wie jede andere Schablone.

Alle Geometrie, alle Formeln und alle Grafiken hier sind eigene Arbeit. Aus Visios Schablonen
wird nichts übernommen oder nachgezeichnet. Genormte Symbole dürfen normgerecht aussehen
(FEATURES.md F2).

```
stencils/
  standard/               eine Schablone je Ordner
    stencil.txt           Name in fünf Sprachen, Kategorie, Reihenfolge, Schnell-Shapes
    rectangle.master      ein Master je Datei
    ...
    server.svg            Grafik für gestaltete Shapes (optional)
```

Ausgabe: `jfd_<file>_m.vssx` (metrisch, mm, A4) und `jfd_<file>_u.vssx` (US, Zoll, Letter). Das Präfix
`jfd_` hält unsere Dateinamen von Visios eigenen fern (Visio hat etwa ein `ARROWS_M.VSSX`): eine
Vorlage nennt ihre Schablonen beim Dateinamen, und Visio öffnet eine eigene Schablone gleichen
Namens, wenn es eine hat (`diagram_io::stencil::file_name`, `parse_file_name`). Beim Bauen
der App landen sie in `OUT_DIR`; zum Ansehen in Visio schreibt
`JFDiagrammer --write-stencils ORDNER` sie in einen Ordner (die Startschablone auch
`diagramiotest --write-stencils ORDNER`, dazu eine Testschablone aus SVG-Grafiken).

## Gemeinsame Regeln

- UTF-8, Zeilenenden LF oder CRLF.
- Eine Zeile, deren erstes sichtbares Zeichen `#` ist, ist ein Kommentar.
- **Kopfzeilen** `schlüssel: wert` stehen vor dem ersten Abschnitt. Der Wert wird an beiden
  Enden gekürzt.
- **Endungen** am Schlüssel oder Zellnamen:
  - Sprache: `.de` `.en` `.es` `.it` `.fr`. Namen, QuickInfos, Stichworte und Texte gibt es in
    allen fünf Sprachen. Fehlt eine, bricht das Bauen mit Datei und Zeile ab.
  - Fassung: `.metric` oder `.us`. Die Zeile gilt nur für diese Fassung und ersetzt die Zeile
    ohne Endung. Beispiel: `Width = 40 mm` und `Width.us = 1.5 in`.
  - Beide Endungen dürfen zusammen stehen, in beliebiger Reihenfolge (`Tip.de.us`).
- Fehler nennen immer Datei, Zeile und Grund (`rectangle.master:12: unbekannter Abschnitt
  [Geometrie]`). Zeile 0 meint die Datei als Ganzes.
- Unbekannte Schlüssel, Abschnitte, Zeilenarten und Zellnamen sind Fehler: das Werkzeug prüft
  jeden Zellnamen gegen die Zellen seines Abschnitts (ein Tippfehler wie `Widht` fällt sofort
  auf). Fehlt eine Zelle, die Visio kennt, wird die Liste in `lib/diagram_io/src/stencil.rs`
  ergänzt.

## stencil.txt

| Schlüssel | Pflicht | Bedeutung |
|---|---|---|
| `stencil` | ja | universeller Name der Schablone (NameU-artiger Schlüssel), z. B. `Basic Shapes` |
| `file` | ja | Dateistamm der Ausgabe, z. B. `standard` → `jfd_standard_m.vssx`, `jfd_standard_u.vssx` |
| `name.<sprache>` | ja, alle fünf | Titel in der Seitenleiste und im Menü „Weitere Shapes“ |
| `category` | ja | Kategorie in „Weitere Shapes“: `general`, `flowchart`, `business`, `engineering`, `network`, `maps`, `software`, `schedule`, `extras` |
| `masters` | ja | die Master-Dateien (ohne `.master`), durch Komma getrennt, in Anzeigereihenfolge |
| `quick` | nein | Anzahl der ersten Master, die Schnell-Shapes sind (SH-G04, MS-04); Standard 0 |
| `language` | nein | Sprache von `Name`, `Prompt` und `ShapeKeywords` in der Datei; Standard `de` |

## Master-Dateien (`*.master`)

### Kopf

| Schlüssel | Pflicht | Bedeutung |
|---|---|---|
| `master` | ja, erste Kopfzeile | `NameU` des Masters, sprachneutral, eindeutig in der Schablone (LG-04) |
| `name.<sprache>` | ja, alle fünf | angezeigter Name |
| `prompt.<sprache>` | ja, alle fünf | QuickInfo |
| `keywords.<sprache>` | nein | Suchbegriffe, durch Komma getrennt |
| `text.<sprache>` | nein | Text, den das Shape beim Ablegen trägt |
| `style` | nein | Formatvorlage für Linie, Füllung und Text, über ihren NameU; Standard `Normal` |
| `line_style`, `fill_style`, `text_style` | nein | einzeln statt `style` |
| `svg` | nein | Grafikdatei im selben Ordner: ihre Pfade werden zu Geometrie (siehe unten) |
| `hidden` | nein | `true`: verborgener Master (`Hidden="1"`), steht in der `.vssx`, erscheint aber nicht in der Seitenleiste; die App legt ihn per Code ab (z. B. die Listen des funktionsübergreifenden Flussdiagramms). Standard `false` |
| `include` | nein | (JD13) Dateien mit Abschnitten, die viele Master teilen, durch Komma getrennt, relativ zum Schablonenordner (`../net_common/asset.inc`); siehe „Eingebundene Dateien“ |
| `scale` | nein | (JD13) Maßstab der Masterseite `1:N` für maßstäbliche Shapes (Rack `1:20`): `DrawingScale` = N, `DrawingScaleType` 4 (metrisch) bzw. 3 (US); die Linienbreiten aus dem SVG gelten auf dem Papier. Standard ohne Maßstab |
| `size` | nein | (JD13) `Breite x Höhe` (`482.6 mm x 88.9 mm`, mit `.us`-Fassung): nötig, wenn `Width` oder `Height` eine Formel ist (Höhe aus Höheneinheiten); daraus entstehen Pin und Masterseite |

### Abschnitte

Ein Abschnitt beginnt mit `[Name]`. Die Namen sind die ShapeSheet-Abschnitte wie in der Datei
(`N`-Attribut in VSDX), nur die Steuerpunkte heißen wie im ShapeSheet-Fenster und in Formeln
`[Controls]` (in VSDX `N="Control"`; `[Control]` geht auch). Jeder Abschnitt außer
`[Geometry]` darf nur einmal vorkommen.

| Abschnitt | Inhalt |
|---|---|
| `[Shape]` | Einzelzellen des Shapes: `Width`, `Height`, `LocPinX`, `Rounding`, `LineWeight`, `TxtWidth` … |
| `[Geometry]` | ein Geometrieabschnitt; jedes weitere `[Geometry]` beginnt den nächsten (IX 0, 1, …) |
| `[Connection]` | Verbindungspunkte |
| `[Controls]` | Steuerpunkte (gelbe Rauten), in VSDX `Control` |
| `[User]` | benutzerdefinierte Zellen |
| `[Property]` | Shape-Daten |
| `[Character]`, `[Paragraph]`, `[Actions]`, `[Hyperlink]`, `[Scratch]` | wie in VSDX |

**Einzelzellen** (in `[Shape]`, und am Kopf eines Zeilenabschnitts für dessen eigene Zellen wie
`NoFill`): `Zelle = Formel`.

**Zeilen:** Eine Zeile beginnt in Spalte 1 mit ihrem Kopf, die Zellen folgen durch `;` getrennt.
Eine Folgezeile, die mit Leerraum beginnt, setzt die Zeile fort. Der Kopf ist

- in `[Geometry]` die Zeilenart: `MoveTo`, `LineTo`, `ArcTo`, `EllipticalArcTo`, `Ellipse`,
  `NURBSTo`, `PolylineTo`, `RelMoveTo`, `RelLineTo`, `RelCubBezTo`, `RelQuadBezTo`,
  `RelEllipticalArcTo`, `InfiniteLine`, `SplineStart`, `SplineKnot`;
- in Abschnitten mit Index (`[Connection]`, `[Character]`, `[Paragraph]`) das Wort `Row`;
- in Abschnitten mit Namen (`[Controls]`, `[User]`, `[Property]`, `[Actions]`, `[Hyperlink]`) der
  Zeilenname mit Doppelpunkt, z. B. `Row_1:` oder `Owner:`.

```
[Controls]
Row_1:  X = 5 mm; Y = Height
        X.us = 0.2 in
        XDyn = Controls.Row_1; YDyn = Controls.Row_1.Y
        XCon = 2; YCon = 1; CanGlue = FALSE
        Tip.de = "Eckenradius ändern"
        Tip.en = "Change the corner radius"
        ...
```

`Tip` ist der Name aus Visios ShapeSheet-Fenster; in die Datei kommt die Zelle als `Prompt` (so steht
sie in Visios Dateien, und dort lesen Visio und JFDiagrammer die QuickInfo des Steuerpunkts, JD21).

### Formeln

- Formeln sind Visios universelle Schreibweise (wie im `F`-Attribut), z. B. `Width*0.5`,
  `MIN(Controls.Row_1, Width*0.5, Height*0.5)`, `Geometry1.X1`.
- **Konstanten** werden als Wert ohne Formel geschrieben, wie Visio es tut:
  - eine Zahl mit oder ohne Einheit (`40 mm`, `1.5 in`, `0.75 pt`, `30 deg`, `0`, `50%`); die
    Datei speichert Längen in Zoll und die geschriebene Einheit in `U` (`V="1.5748" U="MM"`)
  - `TRUE`/`FALSE`
  - eine Zeichenkette in Anführungszeichen (`"Text"`, `""` für ein Anführungszeichen)
  - eine Farbe `#RRGGBB`
- Alles andere wird als Formel geschrieben. Ihr Wert wird beim Bauen mit der Formel-Engine
  (`diagram_core::formula`) berechnet und mitgespeichert, damit Visio und der Betrachter ohne
  Neuberechnung richtig zeichnen.
- Eine Formel mit unbekannter Funktion oder ein Bezug ins Leere ist ein Baufehler. Ins Leere
  zeigt ein Bezug auf eine Zelle, die das Shape (mit seinen Formatvorlagen) nicht hat, außer den
  Einzelzellen mit Visios Vorgabewert (`Angle`, `FlipX` …), auf ein `Sheet.N` oder einen
  Abschnitt, den es nicht gibt; ebenso eine Formel, die einen Fehlerwert ergibt (`#REF!`,
  `#DIV/0!` …), und ein Zirkelbezug.
- Zellen mit Sprachendung (`Tip.de`, `Label.fr`, `Prompt.it`) sind Texte: Die Datei bekommt die
  Sprache aus `language`, die übrigen stehen in den `JFD_Tr…`-Zellen (unten). Die
  Anführungszeichen dürfen fehlen. In `[Geometry]` gibt es keine Sprachendungen.

### Vorgaben, die das Werkzeug ergänzt

Fehlt die Zelle im Master, setzt das Werkzeug:

- `PinX`, `PinY`: halbe Breite und Höhe (Konstanten, der Master liegt auf seiner Masterseite)
- `LocPinX = Width*0.5`, `LocPinY = Height*0.5`
- Masterseite: `PageWidth`, `PageHeight` = Shape-Größe, Maßstab 1:1 in mm bzw. Zoll

`Width` und `Height` sind Pflicht. Meist sind es Längen (Konstanten wie `40 mm`); eine Formel ist
erlaubt, außer zusammen mit `svg:` (JD13, z. B. die Felder mit Auto-Höhe:
`GUARD(MAX(10 mm, TEXTHEIGHT(TheText,Width)))` mit `LockHeight`). Pin und Masterseite entstehen
aus dem berechneten Wert. Mit `svg:` geht eine Formel nur zusammen mit `size:` im Kopf, das dann
Pin und Masterseite bestimmt (JD13, die Rack-Einschübe: Höhe aus den Höheneinheiten). Ausnahme
sind 1D-Master (Abschnitt „1D-Shapes“).

### Gruppen aus Teilen (`[Part NAME]`, JD26)

Ein Master mit Teilen wird eine Gruppe: jedes `[Part NAME]` beginnt ein Untershape. Danach folgen
seine eigenen Kopfzeilen und seine Abschnitte bis zum nächsten `[Part …]` oder zum Dateiende;
alles vor dem ersten `[Part]` gehört zum Master-Shape (der Gruppe). Die Teile liegen in der
Reihenfolge der Datei übereinander (ein späteres oben).

| Kopfzeile eines Teils | Bedeutung |
|---|---|
| `text.<sprache>` | sein Text, alle fünf oder keiner |
| `style`, `line_style`, `fill_style`, `text_style` | seine Formatvorlagen (Standard `Normal`) |

- **Bezüge:** In allen Formeln des Masters zählt `Sheet.N!` logisch: `Sheet.1!` ist das
  Master-Shape (die Gruppe), `Sheet.2!` der erste Teil, `Sheet.3!` der zweite usw. Das Werkzeug
  schreibt die echten IDs (Zeichenketten in Anführungszeichen bleiben); ein `Sheet.N!` ohne Shape ist
  ein Baufehler.
- **Größe und Lage:** Fehlt `PinX`, `PinY`, `Width`, `Height`, `LocPinX` oder `LocPinY`, bekommt
  das Teil die Größe der Gruppe (`GUARD(Sheet.1!Width)` …). Eigene Transformationen schreiben die
  Master ebenso mit `GUARD(...)`: Visio skaliert ein Untershape beim Vergrößern der Gruppe sonst
  selbst, statt die Formel rechnen zu lassen.
- **Texte:** Der Text des Teils steht in der Bausprache in der Datei, alle fünf in `User.JFD_Text`
  seines Blatts im Master. Beim Ablegen (`instance::localize_parts`, von `instance::place`
  gerufen) und in neuen Zeichnungen aus Vorlagen (`template::to_drawing`) bekommt ein Teil den Text
  der aktuellen Sprache, solange es einen der fünf (oder keinen) zeigt. Instanzen erben
  `JFD_Text` vom Master, sie schreiben ihn nicht.
- **Nicht erlaubt:** Zellen mit Sprachendung in Teilen (übersetzte Zellen wie `Tip` oder Shape-Daten
  gehören ans Master-Shape), Teile zusammen mit `svg:` und Teile an 1D-Mastern.
- Klicken wählt erst die Gruppe, ein zweiter Klick das Teil (Visios `SelectMode` 1, nicht gesetzt).

### Eingebundene Dateien (`include:`, JD13)

Eine eingebundene Datei (Endung `.inc`, in einem Ordner ohne `stencil.txt`, z. B.
`stencils/net_common/`) hat nur Abschnitte, keine Kopfzeilen. Ihre Abschnitte werden in die des
Masters gemischt:

- Einen Abschnitt, den der Master nicht hat, bekommt er; `[Geometry]`-Abschnitte kommen immer
  hinzu, nach den eigenen.
- In einem Abschnitt, den der Master auch hat, stehen die eingebundenen Zellen und Zeilen vorn;
  die des Masters gewinnen: Eine Zelle gleichen Namens (mit gleicher Sprach- und Fassungsendung)
  fällt weg, eine benannte Zeile gleichen Namens bekommt nur die eingebundenen Zellen, die der
  Master nicht selbst setzt (`RackUnits: Value = 2` übernimmt Bezeichnung und Typ aus der Datei).
- Fehler nennen die eingebundene Datei und ihre Zeile; ihr Text gehört zur `UniqueID`.

### Textfelder (`[Field]`, JD15)

Ein Text kann Felder enthalten, die einen berechneten Wert zeigen (TX-06), etwa die Länge einer
Bemaßung. Im Text (`text.<sprache>`) steht dafür die Marke `{fld N}`; sie zeigt Zeile N (ab 0) des
Abschnitts `[Field]` (in VSDX `N="Field"`, Visios Textfelder). Jede andere `{`-Folge bleibt Text. Eine
Marke ohne ihre Zeile ist ein Baufehler. Die Zeilen beginnen wie bei `[Connection]` mit `Row`:

```
text.de: {fld 0}
...
[Field]
Row  Value = ABS(Width); Unit = mm; Format = "0 u"; Type = 2
     Unit.us = in
     Format.us = "0.## u"
```

- `Value`: die Formel, deren Wert das Feld zeigt; `Format`: das Formatbild (Visios FORMAT, siehe
  `diagram_core::field`); `Type`: 0 Text, 2 Zahl, 5 Datum oder Uhrzeit, 6 Dauer, 7 Währung.
- `Unit` ist ein Schlüssel des Werkzeugs, keine Visio-Zelle: die Einheit, in der das Feld den Wert
  zeigt (`U` von `Value`; `mm`, `cm`, `m`, `in`, `ft`, `pt`, `deg` …). Ohne `Unit` zeigt eine Länge
  in Zoll.
- Beim Ablegen bekommt die Instanz den Text mit seinen Feldern; die Feldzeilen erbt sie vom Master,
  der Wert folgt jeder Änderung (`diagram_core::field::marked`, `instance::place`).
- Im Einsatz: die Bemaßungen der Rack-Schablone (`rack_equipment/dim_*.master`) und die
  Auto-Bemaßung der Beschriftungen (`callouts/auto_dimension.master`, zeigt ihre eigene Länge).
  (JD21) Ansicht › Add-Ons › Maßlinien legt genau diese drei Master ab (horizontal, vertikal,
  ausgerichtet) und setzt ihre Enden an Shapes (`diagram_core::tools::dimension`); ihre NameU und
  der Steuerpunkt `Controls.Line` sind dafür fest.

## 1D-Shapes (Verbinder, Linienpfeile)

Ein Master ist ein 1D-Shape, wenn `[Shape]` die Enden `BeginX`, `BeginY`, `EndX` und `EndY`
setzt (eine davon genügt für die Erkennung, dann sind alle vier Pflicht). Das ist Visios eigene
Speicherform: Es gibt kein eigenes Kennzeichen, ein Shape mit `BeginX` ist eindimensional; die
App (`diagram_core::glue::is_one_d`) liest es genauso.

- `BeginX` … `EndY` sind Längen ab 0 (Konstanten wie `40 mm`, keine Formeln): die Enden auf der
  Masterseite. Die Masterseite ist `BeginX+EndX` breit und `BeginY+EndY` hoch, die Enden haben
  also zum Rand so viel Abstand wie zum anderen Rand; eine waagrechte Linie braucht deshalb
  `BeginY = EndY = 2.5 mm` o. ä. statt 0. Fallen die Enden zusammen oder wäre die Seite leer, ist
  das ein Baufehler.
- `Height` ist Pflicht: die Dicke (Länge ab 0, eine Linie hat `0 mm`) oder eine Formel.
- Fehlt die Zelle, setzt das Werkzeug Visios 1D-Formeln, so dass der Kasten den Enden folgt:

  | Zelle | Formel |
  |---|---|
  | `PinX`, `PinY` | `GUARD((BeginX+EndX)/2)`, `GUARD((BeginY+EndY)/2)` |
  | `Width` | `GUARD(SQRT((EndX-BeginX)^2+(EndY-BeginY)^2))` (die Länge) |
  | `Angle` | `GUARD(ATAN2(EndY-BeginY,EndX-BeginX))` (der Kasten liegt auf der Linie) |
  | `LocPinX`, `LocPinY` | `GUARD(Width*0.5)`, `GUARD(Height*0.5)` |

  Die Geometrie zeichnet dann im gedrehten Kasten: x von 0 (Anfang) bis `Width` (Ende), die
  Linie selbst auf `Height*0.5`.
- **Aufrechter Kasten** (wie der dynamische Verbinder der App, für Verläufe von Seite zu Seite):
  der Master schreibt `Width = GUARD(EndX-BeginX)`, `Height = GUARD(EndY-BeginY)`,
  `Angle = GUARD(0DA)`; beide Maße dürfen negativ sein, lokal (0, 0) ist der Anfang und
  (`Width`, `Height`) das Ende. Geometrie in Anteilen des Kastens (`RelMoveTo`, `RelLineTo`,
  `RelCubBezTo`) bleibt so bei jeder Lage richtig.
- **Kleben:** Die Enden kleben an Verbindungspunkte (statisch, `PAR(PNT(…))`, schreibt die App
  beim Kleben selbst). `GlueType = 2` erlaubt auch das Kleben an das ganze Shape (das Ende sucht
  sich den Rand). Trigger (`BegTrigger`, `EndTrigger`) braucht der Master nicht.
- **Kein `ObjType = 2`** für Shapes mit eigener Geometrie: Das Kennzeichen „verlegbar“ macht ein
  1D-Shape zum dynamischen Verbinder; die App verlegt ihn dann selbst und ersetzt seine
  Geometrie. Formatvorlage: Linienverbinder `Connector` (Linie, Pfeilspitze und Text mit
  Hintergrund aus dem Design), gefüllte flächige Pfeile `Normal`. Ein Verbinder nur aus Umrissen
  (ohne Füllung) nimmt ebenfalls `Connector`, mit `BeginArrow = 0` und `EndArrow = 0`: mit
  `Normal` verschwände er in Designs ohne Shape-Linien (Kontrast).
- Text eines Linienverbinders: ein fester Textblock (z. B. `TxtWidth = 30 mm`) mitten auf dem
  Verlauf; der Hintergrund aus `Connector` erscheint nur hinter vorhandenem Text.

Die App legt einen 1D-Master heute wie ein 2D-Shape ab (`instance::place` setzt `PinX`/`PinY`
als Konstanten, die Enden bleiben auf den Werten der Masterseite); siehe Prüfung
`diagramiotest --check --only stencils/shapes/drop`.

## Stilregeln (SH-G01, vom Nutzer mit dem JD13-Probeblatt abgenommen)

Verbindlich für alle mitgelieferten Master; das Probeblatt (`diagramiotest
--write-stencil-previews DIR`, `probeblatt.png`) zeigt sie, `<ordner>-alle.png` zeigt je
Schablone alle Master.

- **Formatvorlage:** 2D-Master ohne `style:` (also `Normal`), Linien- und Umrissverbinder
  `style: Connector`. Keine Farben, Linienstärken oder Schriftfarben im Master, damit die
  Designs (JD11) wirken. Ausnahmen nur begründet im Kopfkommentar (Tipp: `LineWeight 3 pt`,
  `LineCap 1`; 3D-Feld: `Rounding 0`; Hohler Verbinder: `BeginArrow`/`EndArrow 0`).
- **Form:** ein Umriss je Fläche, eckig mit geraden Kanten. Bögen nur, wo Name oder Norm sie
  verlangen (`ArcTo`, `EllipticalArcTo`; Ellipsenbögen durch den 45°-Punkt). Mehrere Flächen
  nur nebeneinander (Visio füllt nach der Gerade-Ungerade-Regel); Innenkanten als eigener
  Abschnitt mit `NoFill TRUE`. Keine Rundung im Master, `Rounding` kommt aus dem Design.
- **Genormte Symbole** (ISO 5807 …) sehen normgerecht aus; Flussdiagramm-Shapes 36 × 24 mm (3:2).
- **Maße:** 5-mm-Raster (40 × 20, 40 × 30, 40 × 25, 30 × 30 mm), US-Fassung in runden Zoll
  (1.5 × 0.75, 1.5 × 1.125, 1.25 × 1.25, 1.5 × 1). Pfeile: Spitze etwa 0,4 × Länge, Schaft halb
  so dick wie die Spitze, Bogen außen 1,5 × Schaft. 1D-Flächenpfeile: Dicke 10 mm, Spitze 8 mm.
- **Steuerpunkte:** ein gelber Punkt je Formparameter auf der Ecke, die er bewegt; unnötige
  Richtung gesperrt (`XCon`/`YCon` 1); anteilig (0) für Formen, die mitskalieren, Abstand vom
  Rand (2 oder 4) für Maße, die beim Strecken bleiben (Tiefe, Spitze eines 1D-Pfeils,
  Zylinderenden). Werte in `User`-Zellen mit `MIN`/`MAX` begrenzt; `Tip` in fünf Sprachen.
- **Verbindungspunkte:** Mitte jeder Seite in der Reihenfolge unten, rechts, oben, links,
  `DirX`/`DirY` nach innen. Pfeile: Schaftende und Spitze(n); Raute: an den Ecken; Dokument:
  unten auf der Welle. 1D-Shapes haben keine, dort kleben die Enden.
- **Text:** Textblock auf der Hauptfläche (Kasten, Vorderseite, über der Welle, Bandmitte),
  `TxtLocPin` die halbe Größe; Linienverbinder: Block 30 × 8 mm mittig auf dem Verlauf.
- **Sprachen:** Name, ein Prompt-Satz (was es ist, dazu einer zu den Steuerpunkten) und 5–8
  Stichworte je Sprache; NameU ist der naheliegende englische Begriff.
- **Kategorien für Regeln:** Flussdiagramm-Shapes tragen `User.msvShapeCategories`
  (`"Flowchart"`, die Entscheidung `"Flowchart;Decision"`, Start/Ende `"Flowchart;Terminator"`).
- **Hindernisse (`ObjType`, JD21v):** Dynamische Verbinder weichen nur platzierbaren Shapes aus
  (`ObjType` mit Bit 1, wie Visio 16). Ein Master bekommt `ObjType = 1`, wo Visios Gegenstück
  platzierbar ist (Gegenprobe visio-obst-2): die Flussdiagramm-Shapes, die gestellmontierten Server,
  Wolke, Ring-Netzwerk und Ethernet. Die übrigen 2D-Master bleiben ohne `ObjType` (0, Visio
  entscheidet: platzierbar erst, wenn ein Verbinder an ihnen klebt). Visios Gegenstücke sind dort
  0, 4 oder Gruppen mit 8 ohne platzierbare Teile (Standard-Shapes, Blöcke, Netzwerk- und
  Server-Symbole, Marketing, Venn, Pyramide, Matrix, Datengrafiken 4), also keine Hindernisse.

Ergänzt mit dem JD26-Probeblatt (vom Nutzer abgenommen, 2026-11-23):

- **Mehrere Farben in einem Shape** (Mengen, Felder, Ebenen, Phasen): Teile mit
  `QuickStyleFillColor` und `QuickStyleLineColor` 100 bis 106, den sieben Variantenfarben des
  Designs; schattierte 3D-Flächen über Formeln auf die Designfarbe (`THEMEGUARD(SHADE(THEMEVAL("FillColor"),25))`).
  Feste Farben bleiben verboten. Durchsicht (`FillForegndTrans`) nur begründet (Venn).
- **Designs ohne Shape-Linien** (Kontrast): Was nur Linien zeichnet, verschwindet dort. Trennungen
  zwischen Flächen sind deshalb Fugen (1 mm) statt Innenlinien, kleine Innenformen (Augen, Mund)
  Löcher. Reine Linien-Shapes (Achsen, Gräten) nehmen `style: Connector` mit
  `QuickStyleType = 3` (ausdrücklich: Visio wählt die Verbinder-Liste des Designs sonst nach der
  Art des Shapes, JFDiagrammer nach dem Namen der Formatvorlage) und `BeginArrow`/`EndArrow 0`,
  wenn das Shape eigene Spitzen zeichnet. Linien und Flächen in einem Shape: `line_style:
  Connector`, `fill_style`/`text_style: Normal`, `QuickStyleType = 3`, `QuickStyleFillMatrix` und
  `QuickStyleFontMatrix` 100; oder eine Gruppe mit einem Linien- und einem Flächen-Teil.
- **Schnellformatzellen ausdrücklich** (Visio-Gegenprobe JD26): Visio nimmt `QuickStyleType` und
  alle `QuickStyle…Matrix`-Zellen aus der *Füll*-Formatvorlage, JFDiagrammer die Linienmatrix aus
  der Linien- und die Schriftmatrix aus der Text-Formatvorlage. Wer die Formatvorlagen mischt
  oder `style: Connector` mit `QuickStyleType = 3` nimmt, setzt `QuickStyleLineMatrix` selbst:
  reine Linien `2` (Linie 2 der Verbinder-Liste, 1 pt in der Variantenfarbe), Linien mit
  Flächen wie Normal `100`. Linie 1 der Verbinder-Liste (und Linie 2 der Shape-Liste) von
  NetLab ist `dk2`; die kennt Visio in Linienlisten nicht und zeichnet dort die Variantenfarbe.
  Die Pfeilspitze des Designs (`EndArrow` aus Connector) steht am Ende des *letzten* Pfads:
  eine Linie mit Kasten zeichnet den Kasten zuerst (Kategorie-Gräten).
- **Textblock ganz oder gar nicht:** Visio liest eine fehlende Zelle eines Textblocks, von dem
  das Shape eine Zelle hat, als 0 (Breite 0, am linken Rand); das Bauwerkzeug ergänzt deshalb
  die übrigen der sechs Zellen (`TxtPinX` … `TxtLocPinY`) mit dem Shape-Kasten.
- **Anzahl aus Shape-Daten** (Ebenen, Stufen, Phasen, Segmente): `Prop.<Name>` (Typ 2) mit Label
  und Prompt in fünf Sprachen, `User.N` begrenzt; Teile oder Abschnitte über N mit `NoShow` und
  `HideText`, deckungsgleich mit dem letzten sichtbaren und darunter. Wo Ziehen Elemente
  vermehrt, schreibt ein Steuerpunkt per `SETATREF(Prop.<Name>, SETATREFEVAL(…))` die Zahl und die
  Größe ist Zahl x Elementgröße (`SETATREF` der Größe schickt das Strecken in die Elementgröße);
  gezeichnet wird nach der tatsächlichen Größe (Width/N).

## SVG-Grafik (`svg:`)

Für gestaltete Shapes im JFNetworker-Stil. Die SVG-Datei braucht eine `viewBox`; sie wird auf
den Shape-Kasten abgebildet (x nach rechts, y nach oben gedreht).

- Elemente: `path`, `rect` (auch mit `rx`/`ry`), `circle`, `ellipse`, `line`, `polyline`,
  `polygon`, `g`; `transform` (`translate`, `scale`, `matrix`, `rotate`, `skewX`, `skewY`) an
  jedem davon. `title`, `desc`, `metadata` und Elemente fremder Namensräume (Editordaten)
  werden übergangen, alles andere (`text`, `image`, `use`, Verläufe …) ist ein Fehler mit Zeile.
- Pfadbefehle: `M L H V C S Q T A Z` groß und klein. Strecken werden `LineTo`, Kurven werden
  exakte kubische `NURBSTo`-Zeilen (quadratische angehoben, Bögen in Stücke bis 90° zerlegt).
  Eine `NURBSTo`-Zeile ist ein Bézier-Stück: Grad 3, Knoten 0 0 0 0 1 1 1 1, alle Gewichte 1
  (`A = 0`, `B = 1`, `C = 0`, `D = 1`, `E = NURBS(1,3,0,0,x1,y1,0,1,x2,y2,0,1)` mit den
  Kontrollpunkten als Anteil von Breite und Höhe, `U="NURBS"`).
- Jeder Teilpfad wird ein eigener Geometrieabschnitt. Alle Koordinaten sind Formeln relativ zur
  Größe (`Width*0.25`, `Height*0.8`), das Shape skaliert also mit. Visio füllt alle Abschnitte
  eines Shapes zusammen nach der Gerade-Ungerade-Regel; Teilpfade, die sich mit gleicher
  Richtung überlagern (SVG `nonzero`), werden dort also ausgespart.
- `fill="none"` → `NoFill`, `stroke="none"` → `NoLine`. Ein gefüllter und umrandeter Teilpfad
  ohne `Z` wird zwei Abschnitte: ein geschlossener mit `NoLine` (die Füllung, die SVG
  stillschweigend schließt) und der offene mit `NoFill`.
- Attribute: `fill`, `stroke`, `stroke-width`, `fill-opacity`, `stroke-opacity`, `opacity`,
  `stroke-linecap`, `display`, auch in `style="…"`, von `g` geerbt. Farben: `#rgb`, `#rrggbb`,
  `rgb()`, ein paar Namen (`black`, `white`, `red` …).
- Haben alle Elemente dieselbe Füllung, Linie und Linienbreite, trägt das Master-Shape selbst
  die Geometrie und diese Formate (außer den Zellen, die `[Shape]` selbst setzt). Sonst wird es
  eine Gruppe mit einem Untershape je Folge gleich formatierter Elemente, in der Reihenfolge der
  Datei (JD13: wie JFNetworkers `vsym.py`; ein Format, das später wiederkommt, wird ein eigenes
  Untershape über allem dazwischen). Jedes Untershape ist so groß wie die Gruppe
  (`Width = Sheet.N!Width` …) und zeichnet seine Pfade.
- Farben werden als `#RRGGBB` übernommen (`FillForegnd`, `FillBkgnd`, `LineColor`, Deckkraft
  als `…Trans`), die Linienbreite im Verhältnis viewBox zu Shape-Größe (`LineWeight` in pt).

## Strukturen: Container, Listen, Beschriftungen (JD10)

Die Schablonen `containers`, `cff` und `callouts` liefern Shapes, die andere aufnehmen oder auf
sie zeigen. Die App (`diagram_core::structure`) liest nur diese Zellen; ihre Namen sind Visios
Namen (Kompatibilität), Werte, Formeln und Aussehen sind eigene Arbeit. Längen metrisch mit
`.us`-Fassung in Zoll.

| Zelle | Inhalt |
|---|---|
| `User.msvStructureType` | `"Container"`, `"List"` oder `"Callout"` |
| `User.msvShapeCategories` | Kategorien, `;`-getrennt ohne Leerzeichen (`"Swimlane;DoNotContain"`) |
| `User.msvSDContainerMargin` | Abstand der Mitglieder zum Inneren (Länge) |
| `User.msvSDContainerResize` | 0 keine Größenänderung, 1 nach Bedarf erweitern, 2 immer anpassen |
| `User.msvSDContainerLocked`, `…NoHighlight`, `…NoRibbon` | `FALSE` |
| `User.msvSDContainerStyle`, `…StyleCount`, `msvSDHeadingStyle`, `…HeadingStyleCount` | Stil und Kopfzeilenstil (1-basiert) und ihre Anzahl; der Master schaltet seine Looks selbst per Formel |
| `User.msvSDContainerRequiredCategories`, `…ExcludedCategories` | wer Mitglied werden darf |
| `User.msvSDListDirection` | 0 links→rechts, 1 rechts→links, 2 oben→unten, 3 unten→oben |
| `User.msvSDListSpacing`, `…Alignment`, `…RequiredCategories`, `…ExcludedCategories`, `…ItemMaster` | Abstand Kante zu Kante, Ausrichtung 0/1/2, Elementfilter, NameU des Standard-Elements |
| `User.JFD_ListStretch` | `TRUE`: alle Elemente quer so lang wie die Liste |
| `User.JFD_HeadingEdge` | Kante, an der die Kopfzeile liegt: `"Top"`, `"Bottom"`, `"Left"`, `"Right"`, `"None"` |
| `User.JFD_HeadingSize` | Tiefe der Kopfzeile (Länge, 0: ohne Kopf) |
| `User.JFD_LabelUpright` | Bahnen und Trennlinien: `TRUE` = Text im seitlichen Kopf waagerecht statt um 90° gedreht |
| `Controls.Association` | Beschriftungen: die Spitze des Zeigers, die der Nutzer auf das Ziel zieht |
| `User.msvSDCalloutNoHighlight` | Beschriftungen: `FALSE` |

Regeln für die Master:

- Der Text ist die Überschrift; der Textblock (`TxtPinX/Y`, `TxtWidth/Height`, `TxtAngle`) liegt
  in der Kopfzeile. Kopfband, Kopflinie und Textblock hängen nur von `JFD_HeadingEdge` und
  `JFD_HeadingSize` ab (Hilfszellen `HeadTop` … `HeadRight`): setzt die App `JFD_HeadingSize = 0`,
  verschwindet der Kopf samt Text (`HideText`), die Box bleibt sauber gezeichnet. Beim Master
  `Container` sind beide Formeln aus `msvSDHeadingStyle` (1 Kopfband oben, 2 nur Titel oben,
  3 Kopfband links, 4 ohne Kopf).
- (JD21, ST-02) Der Master `Container` hat 14 Containerstile (`msvSDContainerStyleCount = 14`;
  1–6 unverändert seit JD10, 7–14: Leiste, Doppelrahmen, Schiefer, Reiter, Kopflinie, Klammer,
  Signal, Kopf unten) und je Stil 2–4 Kopfzeilenstile (`msvSDHeadingStyleCount` per Formel aus dem
  Stil; Stil 14 legt den Kopf unten bzw. rechts). Ein Kopfzeilenstil über der Anzahl zeigt keinen
  Kopf. Die App zeigt die Stile als Galerien mit Bildern dieses Masters (`container_styles.rs`)
  und bietet nur so viele Stile an, wie der Master des Containers hat (ältere Zeichnungen: 6).
  Probeblatt: `diagramiotest --write-stencil-previews DIR` › `container-styles.png`.
- Geometrie nur aus `Width`, `Height` und Zellformeln: die App vergrößert die Shapes per Code.
  Teile, die ein Stil nicht zeigt, schaltet `NoShow` im Geometrieabschnitt ab; ein Shape hat
  eine Linie und eine Füllung, verschiedene Looks entstehen über `NoFill`/`NoLine` je Abschnitt.
- Beschriftungen zeichnen den Zeiger per Formel vom Körper zu `Controls.Association`; liegt der
  Punkt im Körper, ist der Zeiger aus (`NoShow` bzw. Keilpunkte auf der Kante).
- Die Listen des funktionsübergreifenden Flussdiagramms und sein Rahmen sind verborgene Master
  (`hidden: true`); die Listen zeichnen nichts (`NoShow`).
- Vorschau aller Master mit allen Stilen: `diagramiotest --write-stencil-previews ORDNER`;
  Prüfungen: `diagramiotest --check --only stencils/structure`.

## Allgemein und Geschäftsgrafiken (JD26)

Acht Schablonen: `decorative` (SH-003), `charting` (SH-004), `venn` (SH-007) in der Kategorie
`general`, `marketing` (SH-030), `matrix` (SH-031), `pyramid` (SH-032), `cycle` (SH-033) und
`cause_effect` (SH-041) in `business`.

- **Gruppen aus Teilen:** SWOT, Matrizen, Venn-Diagramme, Pyramiden und Zyklen mit Text je Feld,
  Menge, Ebene oder Phase (siehe „Gruppen aus Teilen“).
- **Zahlen und Werte aus Shape-Daten:** Ebenen, Stufen, Phasen, Segmente, Ringe; das
  Kreisdiagrammsegment nimmt Beginn, Anteil und Farbe (`Prop.Start`, `Prop.Share`, `Prop.Color`).
  Das Stufendiagramm und die Pyramide mit Ebenen wachsen beim Ziehen am Steuerpunkt.
- **Fischgräte:** Die Hauptgräte (`Effect`) hat 15 Verbindungspunkte entlang der Gräte. Die
  Kategorien sind 1D-Shapes mit aufrechtem Kasten; ihr Ende klebt an der Hauptgräte, ihre Gräte
  hat selbst Punkte für die Ursachen (begründete Ausnahme von „1D-Shapes haben keine
  Verbindungspunkte“). JFDiagrammer bietet die Verbindungspunkte von 1D-Shapes beim Kleben an
  (`edit_canvas.rs`).
- Gemeinsame Master als Kopie mit gleichem NameU: Teil- und Konzentrische Ebenen aus `blocks` in
  `cycle`, der dynamische Verbinder aus `blocks` in `marketing`, das Venn-Diagramm aus 2 Mengen in
  `marketing`, das Zwiebeldiagramm aus `venn` in `cycle`.
- Prüfen: `diagramiotest --check --only stencils/business`, Ansicht
  `diagramiotest --write-stencil-previews DIR` (`probeblatt-jd26.png`, `<ordner>-alle.png` und
  `<ordner>-alle-kontrast.png`).

## Datengrafiken (JD19)

Die verborgene Schablone `datagraphics` (Kategorie `extras`, in „Weitere Shapes“ nicht angeboten)
liefert die Elemente, die eine Datengrafik an ein Shape setzt (DA-07): drei Text-Elemente (`DG Text`,
`DG Text box`, `DG Text underlined`), vier Datenleisten (`DG Progress bar`, `DG Star rating`,
`DG Thermometer`, `DG Speedometer`) und vier Symbolsätze (`DG Traffic lights`, `DG Arrows`,
`DG Flags`, `DG Rating`). Alle Master sind `hidden: true`; die App setzt Instanzen als Untershapes an
die Shapes (`diagram_core::datagraphic`). Die Zellnamen sind Visios, damit Visio die Elemente erkennt
und bearbeiten kann; Zeichnungen, Formeln und Farben sind eigene Arbeit. Der NameU beginnt mit `DG `,
damit er nicht mit Visios Mastern gleichen Namens in einer Datei zusammenfällt.

| Zelle | Inhalt |
|---|---|
| `User.msvCalloutType` | `"Text Callout"`, `"Data Bar"` oder `"Icon Set"` |
| `Prop.msvCalloutField` | der Wert, den das Element zeigt (die App schreibt eine Formel auf die Shape-Daten des Shapes; `SortKey` = Bezeichnung des Felds) |
| `Prop.msvCalloutProp*` | die Optionen; `Prompt` = Art des Eingabefelds wie bei Visio (`Number`, `Bool`, `Format`, `String`, `List`): `Min`, `Max`, `ShowValue`, `ValueFormat`, `Label`, `LabelPosition`, `Border`, `Fill` |
| `User.msvCalloutIconCount`, `User.msvCalloutIconNumber` | Symbolsätze: Anzahl der Symbole, das gezeigte (-1 keins); die App schreibt dort die aus den Regeln übersetzte Formel |

- Datenleisten rechnen den Anteil `User.F` = (Wert - Minimum) / (Maximum - Minimum), begrenzt auf 0 bis 1;
  Breite, Säule, Sterne und Zeiger folgen ihm. Gefüllte Teile in der Linienfarbe (`FillForegnd =
  LineColor`).
- Symbolsätze schalten ihre Geometrieabschnitte mit `NoShow`/`NoFill` nach `User.msvCalloutIconNumber`.
  Die Farben der Ampel, Pfeile und Flaggen sind die Bedeutung der Symbole und deshalb fest
  (begründete Ausnahme der Stilregeln).
- Text-Elemente zeigen ein Textfeld: Bezeichnung und Wert (`FORMAT` im Wertformat) nach
  `LabelPosition`; Rahmen und Füllung nach `Border` und `Fill`.
- Prüfen: `diagramiotest --check --only datagraphics`, Gegenprobe `diagramiotest --write-dg DIR`.

## Was in der `.vssx` steht

Visio-Attribute je Master: `ID`, `NameU`, `Name` und `Prompt` in der Bausprache,
`IsCustomName(U)=1`, `UniqueID`, `BaseID`, `IconSize=1`, `AlignName=2`, `MatchByName=0`,
`IconUpdate=1`, `PatternFlags=0`, `Hidden=0` (`1` mit `hidden: true`), `MasterType=2`. Das Symbol ist ein 32×32-Icon mit
16 Farben, aus dem Shape gerendert (SH-G03): eine ICO-Datei wie die, die Visio schreibt (4 Bit
je Pixel, eigene Palette, Maske für die Durchsicht, 766 Bytes), mit 2 Pixeln Rand eingepasst;
Text erscheint darin als Balken. Die Suchbegriffe der Bausprache stehen im PageSheet des Masters
in `ShapeKeywords` (dort sucht auch Visio). Die Masterseite ist so groß wie das Shape, Maßstab
1:1 (`DrawingSizeType = 1`).

- `BaseID` ist ein fester GUID aus Schablone, NameU und Fassung.
- `UniqueID` hängt zusätzlich vom Inhalt ab (den Zeilen der Fassung ohne Kommentare, dem SVG,
  der Bausprache, dem Schnell-Shape-Merkmal und der Version des Werkzeugs). Ein geänderter
  Master bekommt also beim nächsten Bauen eine neue Kennung, ein unveränderter behält sie (MS-02:
  Abgleich beim Ablegen).
- Beide sind namensbasierte GUIDs (SHA-1, RFC 4122 Version 5), groß geschrieben in
  geschweiften Klammern. Gleiche Quellen ergeben byte-gleiche `.vssx`-Dateien.
- Formeln stehen mit ihrem berechneten Wert in der Datei, Konstanten als Wert mit Einheit
  (`V="1.5748" U="MM"`), `TRUE`/`FALSE` als `1`/`0` mit `U="BOOL"`, Zeichenketten mit `U="STR"`.

Eigene Zellen (Präfix `JFD_`), damit alle fünf Sprachen mit der Datei reisen. Mehrsprachige
Werte stehen in der festen Reihenfolge `de|en|es|it|fr`, `\|` und `\\` maskieren:

| Ort | Zelle | Inhalt |
|---|---|---|
| Dokument-ShapeSheet der Schablone | `User.JFD_Name` | Titel in fünf Sprachen |
| | `User.JFD_Category` | Kategorie aus `stencil.txt` |
| PageSheet des Masters | `User.JFD_Name` | Namen in fünf Sprachen |
| | `User.JFD_Prompt` | QuickInfos |
| | `User.JFD_Keywords` | Suchbegriffe je Sprache |
| | `User.JFD_Text` | Text des Shapes, wenn es einen hat |
| | `User.JFD_Quick` | `1`: Schnell-Shape (MS-04) |
| | `User.JFD_Tr1`, `JFD_Tr2` … | `Zellbezug|de|en|es|it|fr` für übersetzte Zellen des Master-Shapes; der Bezug ist `Abschnitt.Zeile.Zelle` mit dem VSDX-Abschnittsnamen, die Zeile ihr Name oder ihr Index, z. B. `Control.Row_1.Tip|…` oder `Connection.0.Prompt|…`; eine Einzelzelle nur mit ihrem Namen (`Comment|…`) |

Beim Ablegen schreibt JFDiagrammer den Namen der aktuellen Sprache als `Name` ins Dokument und
behält `NameU` als Schlüssel (LG-04). Übersetzte Zellen und der Text kommen ebenfalls in der
aktuellen Sprache auf die Instanz.

## Netzwerk und Rack (JD13)

Neun Schablonen der Kategorie `network`: `net_computers` (SH-140), `net_detailed` (SH-142),
`net_peripherals` (SH-151), `net_locations` (SH-153), `net_symbols` (SH-156), `net_servers`
(SH-158), `rack_freestanding` (SH-145), `rack_equipment` (SH-146), `rack_servers` (SH-147).

- **Grafiken:** aus dem gemeinsamen Symbolsatz von JFNetworker und JFDiagrammer
  (`../symbols/`, F9, SH-G09; später ein eigenes Repository als git-Submodule), angesprochen mit
  `svg: ../../symbols/network/router.svg`. Der Ordner hat eine eigene README mit Stilregeln.
- **Gemeinsame Abschnitte** in `net_common/` (kein `stencil.txt`, per `include:`):
  `symbol.inc` (24 x 18 mm, Name unter dem Symbol), `asset.inc` (Inventar, `ShapeClass`,
  `User.visLegendShape`), `network.inc` (Adressen), `computer.inc` (Betriebssystem, CPU …),
  `location.inc` (Standorte), `rackdev.inc` (19-Zoll-Einschübe). Die Zeilennamen der Shape-Daten
  sind Visios (`AssetNumber`, `IPAddress` …), Bezeichnungen und Werte eigene.
- **Rack (VL-52):** Masterseiten im Maßstab 1:20 (`scale:`). Gestell und Schrank: Höhe =
  Sockel + Kopf + HE x 44,45 mm aus `Prop.RackUnits`; beim Ziehen schickt `SETATREF` die
  gerundete HE-Zahl in die Shape-Daten (`size:` gibt die Anfangsgröße). Sie sind Container für
  die Kategorie `RackEquipment` und haben auf jeder HE-Grenze der linken Schiene einen
  Verbindungspunkt „nach innen“. Einschübe (`rackdev.inc`): 482,6 mm breit, Höhe aus
  `Prop.RackUnits` ebenso, ein Verbindungspunkt „nach außen“ unten links, mit dem sie in
  JFDiagrammer (`diagram_core::dock`, App `rack.rs`) und in Visio auf den HE einrasten.
- **Besondere Master:** `ethernet` und `ring_network` (acht Steuerpunkte, deren Leitungen zu den
  Geräten führen; in Visio kleben sie), `legend` (JFDiagrammer schreibt die Zählung, App
  `legend.rs`), `dialup` (Wählverbindung, ein 1D-Shape mit Blitz), `dim_horizontal` und
  `dim_vertical` (JD15: Bemaßungen der Rack-Schablone, 1D mit aufrechtem Kasten; ein Textfeld zeigt
  den waagerechten bzw. senkrechten Abstand der Enden, der gelbe Punkt verschiebt die Maßlinie;
  Pfeile und Text in Papiermaß über `ThePage!DrawingScale/ThePage!PageScale`).
- Vorlagen: `templates/network` (VL-50), `templates/network_detailed` (VL-51), `templates/rack`
  (VL-52, Seite 1:20).
- Prüfen: `diagramiotest --check --only stencils/network`, Ansicht
  `diagramiotest --write-network-previews DIR`, Visio-Gegenprobe
  `diagramiotest --write-network-gegenprobe DIR`.
