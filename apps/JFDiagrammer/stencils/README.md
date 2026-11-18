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

Ausgabe: `<file>_m.vssx` (metrisch, mm, A4) und `<file>_u.vssx` (US, Zoll, Letter). Beim Bauen
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
| `file` | ja | Dateistamm der Ausgabe, z. B. `standard` → `standard_m.vssx`, `standard_u.vssx` |
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

`Width` und `Height` sind Pflicht, und zwar als Längen (Konstanten wie `40 mm`, keine Formeln):
aus ihnen entstehen Pin und Masterseite.

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
  eine Gruppe mit einem Untershape je Format in der Reihenfolge, in der die Formate zuerst
  vorkommen; jedes Untershape ist so groß wie die Gruppe (`Width = Sheet.N!Width` …) und
  zeichnet seine Pfade. Liegen Formate abwechselnd übereinander, stimmt die Stapelfolge daher
  nur je Format.
- Farben werden als `#RRGGBB` übernommen (`FillForegnd`, `FillBkgnd`, `LineColor`, Deckkraft
  als `…Trans`), die Linienbreite im Verhältnis viewBox zu Shape-Größe (`LineWeight` in pt).

## Was in der `.vssx` steht

Visio-Attribute je Master: `ID`, `NameU`, `Name` und `Prompt` in der Bausprache,
`IsCustomName(U)=1`, `UniqueID`, `BaseID`, `IconSize=1`, `AlignName=2`, `MatchByName=0`,
`IconUpdate=1`, `PatternFlags=0`, `Hidden=0`, `MasterType=2`. Das Symbol ist ein 32×32-Icon mit
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
