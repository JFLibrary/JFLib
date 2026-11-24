# Mitgelieferte Vorlagen – Quellformat

Die Vorlagen von JFDiagrammer (VL-02, FEATURES.md 5.1) entstehen beim Bauen aus den Textdateien
in diesem Ordner, genau wie die Schablonen (PLAN.md V2, `../stencils/README.md`). Das Build-Skript
der App (`build_steps.rs`) baut zuerst die Schablonen, dann mit `diagram_io::template` jede
Vorlage als `.vstx`, metrisch und US, und bettet sie ein. Eine eingebaute Vorlage ist also eine
gewöhnliche Visio-Vorlage: Die App öffnet eingebaute und eigene Vorlagen (VL-03, EX-11) mit
demselben Code, und Visio öffnet die gebauten Dateien auch.

Alles hier ist eigene Arbeit: Seiten, Texte, Startdiagramme, Regeln. Aus Visios Vorlagen wird
nichts übernommen. Visios Namen für Teile, Elemente, Attribute und Zellen werden für die
Kompatibilität verwendet, Inhalte und Werte sind eigene.

```
templates/
  flowchart/              eine Vorlage je Ordner
    template.txt          Name, Beschreibung, Kategorie, Schablonen, Seiten, Formatvorlagen
    sample.starter        ein Startdiagramm (optional, beliebig viele)
    rules.txt             ein Regelsatz (optional)
```

Ausgabe je Vorlage und Fassung (`m` metrisch, `u` US):

- `<file>_m.vstx`, `<file>_u.vstx`: die Vorlage selbst (im Katalog „Leer“)
- `<file>-<starter>_m.vstx`, `<file>-<starter>_u.vstx`: je Startdiagramm eine eigene Vorlage,
  die dieselben Einstellungen hat und auf der ersten Seite die Shapes des Startdiagramms trägt

`JFDiagrammer --write-templates ORDNER` schreibt alle gebauten Vorlagen zum Ansehen in Visio in
einen Ordner (wie `--write-stencils`).

## Gemeinsame Regeln

Wie bei den Schablonen (`../stencils/README.md`, „Gemeinsame Regeln“): UTF-8, `#`-Kommentare,
Kopfzeilen `schlüssel: wert`, Abschnitte `[Name]`, Zeilen mit Kopf in Spalte 1 und Zellen
`Zelle = Wert` durch `;` getrennt, eingerückte Folgezeilen, die Endungen `.de .en .es .it .fr`
und `.metric`/`.us`. Fehler nennen Datei, Zeile und Grund; unbekannte Schlüssel, Abschnitte und
Zellen sind Fehler.

Längen sind Konstanten mit Einheit (`40 mm`, `1.5 in`); jede Länge, die von der Fassung abhängt,
bekommt eine `.us`-Zeile. In Vorlagen stehen keine Formeln außer in den Regeln.

## template.txt

### Kopf

| Schlüssel | Pflicht | Bedeutung |
|---|---|---|
| `template` | ja, erste Kopfzeile | universeller Name der Vorlage, z. B. `Basic Flowchart` |
| `file` | ja | Dateistamm der Ausgabe, z. B. `flowchart` |
| `name.<sprache>` | ja, alle fünf | Titel im Katalog |
| `description.<sprache>` | ja, alle fünf | Beschreibung in der Vorschau des Katalogs |
| `category` | ja | Kategorie im Katalog, dieselbe Liste wie bei Schablonen: `general`, `flowchart`, `business`, `engineering`, `network`, `maps`, `software`, `schedule`, `extras` |
| `order` | nein | Platz im Katalog, kleinere Zahl zuerst (Standard 100); P1: Leere Zeichnung 10, Standarddiagramm 20, Standardflussdiagramm 30, Funktionsübergreifend 40, Netzwerk 50–59, Rack 60 |
| `stencils` | nein | die Schablonen, die mit der Vorlage aufgehen, durch Komma getrennt, in der Reihenfolge der Seitenleiste. Genannt wird der Dateistamm der eingebauten Schablone (`file` in ihrer `stencil.txt`, z. B. `standard`); das Werkzeug prüft, dass es sie gibt |
| `active` | nein | die Schablone, die aufgeklappt ist (Standard: die erste) |
| `design` | nein | Design aller Seiten: `NetLab` (Standard), `Contour`, `Contrast` (`diagram_core::design::catalog`) |
| `variant` | nein | Variante des Designs, 1–4 (Standard 1) |
| `registers` | nein | Kontextregister, die die Vorlage mitbringt (VL-04), durch Komma getrennt. Bekannt: `cff` (Register „Funktionsübergreifendes Flussdiagramm“) |
| `rules` | nein | Datei mit einem Regelsatz (siehe unten) |
| `starters` | nein | die Startdiagramme, durch Komma getrennt, ohne Endung `.starter`, in Katalog-Reihenfolge |
| `default` | nein | was der Katalog vorwählt: `blank` (Standard) oder ein Startdiagramm (sein Eintrag in `starters` oder sein universeller Name) |
| `blank` | nein | `false`: die Vorlage ohne Startdiagramm erscheint nicht im Katalog (etwa wenn jede Fassung ein Startdiagramm braucht); gebaut wird sie trotzdem (`BuiltTemplate::listed` ist `false`, `build_steps.rs` lässt sie aus `templates.rs` weg). Verlangt `default: <Startdiagramm>`. Standard `true` |
| `language` | nein | Sprache der Texte in der Datei (Seitennamen, Shape-Texte, Regeln); Standard `de` |

### Abschnitte

| Abschnitt | Inhalt |
|---|---|
| `[PageSheet]` | Zellen des Seitenblatts für jede Seite der Vorlage: `PageWidth`, `PageHeight`, `PageScale`, `DrawingScale`, `DrawingScaleType`, `DrawingSizeType`, `PrintPageOrientation`, `PageLeftMargin` … (die Liste steht in `lib/diagram_io/src/template.rs`, `PAGE_CELLS`). Fehlt eine Zelle, gilt die Vorgabe neuer Zeichnungen: A4 (metrisch) bzw. Letter (US) quer, Maßstab 1:1. Steht `PageWidth` oder `PageHeight` ohne `PrintPageOrientation` da, folgt die Ausrichtung der Größe (höher als breit: hoch, `1`, sonst quer, `2`) |
| `[Layer]` | Ebenen jeder Vordergrundseite, Zeilen `Row` mit `Name.<sprache>`, `Visible`, `Print`, `Active`, `Lock`, `Snap`, `Glue` (`TRUE`/`FALSE`), `Color` (0–255). Der universelle Name (`NameUniv`) ist der englische; die fünf Namen stehen im Seitenblatt in `User.JFD_LayerN` (Ebene mit IX N−1) |
| `[Style <NameU>]` | Zellen einer Formatvorlage, z. B. `[Style Normal]` mit `Char.Size = 8 pt` (Zeile 0 von Character und Paragraph als `Char.<Zelle>` und `Para.<Zelle>`, sonst Einzelzellen wie `LineWeight`, dieselben Namen wie in `[Shape]` eines Masters). Bekannte Formatvorlagen: `No Style`, `Theme`, `Normal`, `Connector`, `Guide`, `Text Only` (`diagram_io::blank`) |
| `[Pages]` | weitere Seiten nach der ersten (siehe unten) |

Maßstab: `PageScale` ist die Länge auf dem Papier, `DrawingScale` die Länge in der Wirklichkeit.
1:20 metrisch also `PageScale = 1 mm; DrawingScale = 20 mm; DrawingScaleType = 3`, in US etwa
`PageScale.us = 1 in; DrawingScale.us = 20 in`. Einheit der Anzeige ist die Einheit, in der
`PageWidth` geschrieben ist (`U="MM"`, `U="IN"`).

### Seiten

Jede Vorlage hat mindestens eine Vordergrundseite, `Page-1`. Weitere Seiten stehen in
`[Pages]`, je Zeile eine:

```
[Pages]
Page-2:       Name.de = "Seite-2"; Name.en = "Page-2"; ...
Background-1: Background = TRUE; Name.de = "Hintergrund-1"; ...
```

Zeilenkopf ist der universelle Name der Seite. Zellen: `Name.<sprache>` (alle fünf),
`Background = TRUE`, `BackPage = "<NameU>"` (die Hintergrundseite dieser Seite). Den Namen der
ersten Seite setzt `page1.<sprache>` im Kopf (Standard „Seite-1“, „Page-1“, „Página-1“,
„Pagina-1“, „Page-1“); eine Zeile `Page-1:` in `[Pages]` gibt ihr mit `BackPage` (nur dieser
Zelle) eine Hintergrundseite. Das Seitenblatt jeder weiteren Seite ist eine Kopie des ersten
(nach `[PageSheet]`).

## Startdiagramme (`*.starter`)

Ein Startdiagramm (VL-01) ist eine Vorlage mit Inhalt auf der ersten Seite: ein Beispiel, das der
Nutzer abwandelt, und ein Tipp-Shape, das er löscht.

### Kopf

| Schlüssel | Pflicht | Bedeutung |
|---|---|---|
| `starter` | ja, erste Kopfzeile | universeller Name; der Dateistamm der Ausgabe ist `<file>-<starter>` |
| `name.<sprache>` | ja, alle fünf | Name im Katalog, z. B. „Waagerecht“, „Beispiel: Bestellung“ |
| `description.<sprache>` | nein | eigene Beschreibung (sonst die der Vorlage) |

### `[Shapes]`

Je Zeile ein Shape; der Zeilenkopf ist ein Name nur für diese Datei (`start:`). Die Zeilen
werden der Reihe nach angelegt, spätere liegen oben.

| Zelle | Bedeutung |
|---|---|
| `Stencil`, `Master` | Dateistamm der Schablone und `NameU` des Masters (`Stencil = "flowchart"; Master = "Process"`). Das Werkzeug übernimmt den Master in die Vorlage, wie das Ablegen in der App |
| `PinX`, `PinY` | Position (Mitte des Shapes, Länge) |
| `Width`, `Height` | Größe (Länge), sonst die des Masters |
| `Angle` | Drehung (`90 deg`) |
| `Text.<sprache>` | Text in allen fünf Sprachen; ohne ihn der Text des Masters |
| `Tip` | `TRUE`: das Tipp-Shape. Es bekommt `User.JFD_Tip`; die App wählt es nach dem Anlegen nicht aus und Tests finden es daran |
| `From`, `To` | statt `Master`: ein dynamischer Verbinder zwischen zwei Shapes dieser Datei (die Zeilennamen von Zeilen davor, keine Verbinder), an beiden Enden dynamisch geklebt (`glue::new_connector`); `Text.<sprache>` beschriftet ihn |
| `Cff` | statt `Master`: ein funktionsübergreifendes Flussdiagramm, `"horizontal"` oder `"vertical"`, mit `Lanes` Bahnen (Standard 2, mindestens 1) und `Phases` Phasen (Standard 1); `PinX`/`PinY` ist hier die linke obere Ecke, `Text.<sprache>` der Titel. Die Master kommen aus der Schablone `cff`, aufgebaut wie die App es tut (`diagram_core::structure::cff::create`) |
| `BeginX`, `BeginY`, `EndX`, `EndY` | (JD26) statt `PinX`/`PinY` (und ohne `Width`, `Angle`): ein 1D-Master an diesen Enden (Längen) |
| `BeginOn`, `BeginPoint`, `EndOn`, `EndPoint` | (JD26) das Ende klebt an Verbindungspunkt `…Point` (ab 1, wie Visios `Connections.X1`) des Shapes der Zeile `…On` davor (kein Verbinder aus `From`/`To`), wie das Kleben in der App (`glue::glue`); braucht `BeginX` … `EndY`. Beispiele: `block/sample.starter` (Pfeile zwischen Blöcken), `cause_effect/fishbone.starter` (Kategorien an der Hauptgräte, Ursachen an den Kategorien) |

```
[Shapes]
start:  Stencil = "flowchart"; Master = "Start/End"
        PinX = 60 mm; PinY = 170 mm; PinX.us = 2.4 in; PinY.us = 6.6 in
        Text.de = "Start"; Text.en = "Start"; Text.es = "Inicio"; Text.it = "Inizio"; Text.fr = "Début"
step:   Stencil = "flowchart"; Master = "Process"; PinX = 60 mm; PinY = 140 mm
        PinX.us = 2.4 in; PinY.us = 5.4 in
        Text.de = "Bestellung prüfen"; ...
c1:     From = "start"; To = "step"
tip:    Stencil = "callouts"; Master = "Tip"; Tip = TRUE; PinX = 230 mm; PinY = 180 mm
        PinX.us = 9 in; PinY.us = 7 in
        Text.de = "Tipp: Ziehen Sie Shapes aus der Seitenleiste auf die Seite. Diesen Hinweis können Sie löschen."
        ...
```

## Regelsatz (`rules.txt`)

Ein Regelsatz zur Diagrammprüfung (VL-20, DA-10). JFDiagrammer prüft erst in P2 (DA-10); die
Vorlage trägt den Regelsatz schon jetzt, Visio zeigt ihn unter „Prozess › Diagramm überprüfen“.

| Schlüssel | Pflicht | Bedeutung |
|---|---|---|
| `ruleset` | ja, erste Kopfzeile | universeller Name (`NameU`) |
| `name.<sprache>` | ja, alle fünf | Name des Regelsatzes |
| `description.<sprache>` | ja, alle fünf | Beschreibung |

Abschnitt `[Rules]`, je Zeile eine Regel, Zeilenkopf ist ihr universeller Name (wie der des
Regelsatzes ohne `.` und `|`):

| Zelle | Bedeutung |
|---|---|
| `Category.<sprache>` | Kategorie (alle fünf) |
| `Description.<sprache>` | was falsch ist, wie es in der Problemliste steht (alle fünf) |
| `Target` | `0` Shape, `1` Seite, `2` Dokument |
| `Filter` | Formel: für welche Ziele die Regel gilt (in Anführungszeichen) |
| `Test` | Formel: wahr, wenn das Ziel die Regel erfüllt (in Anführungszeichen) |

Die Formeln nutzen Visios Prüffunktionen (`ROLE()`, `HASCATEGORY()`, `AGGCOUNT()`,
`GLUEDSHAPES()`, `CONNECTEDSHAPES()`, `ONLAYER()` …) und die Zellen der Shapes, etwa
`User.msvShapeCategories`. Das Werkzeug prüft nur, dass sie sich lesen lassen.

## Was in der `.vstx` steht

- **Paket:** eine Visio-Vorlage (`application/vnd.ms-visio.template.main+xml`), geschrieben mit
  `vsdx::write`. Formatvorlagen und Design wie neue Zeichnungen (`diagram_io::blank`), darauf die
  Zellen aus `[Style …]` und das gewählte Design.
- **Seiten:** Seitenblatt mit den Zellen aus `[PageSheet]`, Ebenen, Seitennamen in der
  Bausprache. Shapes der Startdiagramme sind Instanzen ihrer Master; die Master stehen im
  Dokument.
- **Schablonen (VL-02):** im Fensterteil `visio/windows.xml`, so wie Visio ihn schreibt: ein
  Zeichenfenster (`WindowType='Drawing'`, `ID='0'`) und je Schablone ein Element
  `Window WindowType='Stencil'` mit `Document='jfd_<file>_m.vssx'` (bzw. `_u`; das Präfix hält
  sie von Visios eigenen Schablonen fern, `diagram_io::stencil::file_name`), `ParentWindow='0'`,
  `WindowState='1025'` (angedockt, `visWSDockedLeft | visWSAnchorMerged`) oder `'67109889'` für die
  aufgeklappte (`| visWSActive`) und den Kindern `StencilGroup` 10 und `StencilGroupPos` 1, 2 …
  in der Reihenfolge der Seitenleiste. JFDiagrammer erkennt seine eingebauten Schablonen am
  Dateinamen (nur am Präfix, nie an Visios Namen wie BASIC_M.VSSX); andere Namen sind Pfade, die
  es öffnet, wenn es sie gibt (eigene Vorlagen). Visio findet unsere Schablonen in seinen Ordnern nicht und öffnet die Vorlage
  ohne sie. Das eigene Zeichenfenster ist minimal
  (`<Window ID='0' WindowType='Drawing' WindowState='1073741824' ContainerType='Page' Page='0'/>`,
  maximiert, erste Seite); `set_stencil_windows` behält in einem gelesenen Fensterteil alle
  anderen Fenster wörtlich und gibt neuen Fenstern IDs hinter allen, die der Teil hatte.
- **Regelsatz:** der Teil `visio/validation.xml` (Beziehung
  `http://schemas.microsoft.com/visio/2010/relationships/validation`, Inhaltstyp
  `application/vnd.ms-visio.validation+xml`) mit `Validation › RuleSets › RuleSet › Rule`
  (`RuleFilter`, `RuleTest`) in der Bausprache. [MS-VSDX] beschreibt diesen Teil nicht; Aufbau
  nach Visios Dateien, die Visio-Gegenprobe von JD13 prüft ihn. `pack.rs` verlinkt ihn vom
  Hauptteil aus (ein Teil mit einem Beziehungstyp aus Visios Namensraum).
- **Eigene Zellen** (`JFD_`, mehrsprachig in der Reihenfolge `de|en|es|it|fr` wie bei den
  Schablonen):

| Ort | Zelle | Inhalt |
|---|---|---|
| Dokumentblatt | `User.JFD_Template` | universeller Name der Vorlage (bleibt in Zeichnungen erhalten) |
| | `User.JFD_Name`, `JFD_Description` | Titel und Beschreibung im Katalog |
| | `User.JFD_Category`, `JFD_Order` | Kategorie und Platz im Katalog (`JFD_Order` als Zahl) |
| | `User.JFD_Starter` | Name des Startdiagramms (nur in `<file>-<starter>`), `JFD_Default` = `1` in der Fassung, die der Katalog vorwählt (genau eine je Vorlage); `JFD_Description` ist dort die eigene Beschreibung des Startdiagramms, sonst die der Vorlage |
| | `User.JFD_Registers` | Kontextregister (VL-04), `;`-getrennt; bleibt in Zeichnungen erhalten, damit das Register mit der Zeichnung wiederkommt |
| | `User.JFD_RuleN` | Übersetzungen der Regeln: `RuleSetNameU.RuleNameU|de|en|es|it|fr|de|en|es|it|fr` (Beschreibung, dann Kategorie), `JFD_RuleSetN`: `RuleSetNameU|5 Namen|5 Beschreibungen` des Regelsatzes |
| Seitenblatt | `User.JFD_Name` | Seitenname in fünf Sprachen |
| | `User.JFD_LayerN` | Name der Ebene mit IX N−1 in fünf Sprachen |
| Shape | `User.JFD_Text` | Text in fünf Sprachen |
| | `User.JFD_Tip` | `TRUE` am Tipp-Shape (`V='1' U='BOOL'`) |

## Neue Zeichnung aus einer Vorlage

`diagram_io::template::to_drawing(doc, sprache)` macht aus einer gelesenen Vorlage (eingebaut
oder eine `.vstx` des Nutzers) eine neue, unbenannte Zeichnung (DF-05, VL-03):

1. Dokumentart Zeichnung (`.vsdx`), alle Seiten, Master, Formatvorlagen, Designs, der
   Fensterteil und der Regelsatz bleiben.
2. Seitennamen, Ebenennamen, Shape-Texte und Instanznamen kommen in der gewählten Sprache aus
   den `JFD_`-Zellen: der Text aus dem `JFD_Text` des Shapes, sonst aus dem des Masters, wenn das
   Shape einen der Master-Texte zeigt; Name (mit dem „.ID“ einer zweiten Instanz) und übersetzte
   Zellen (`JFD_TrN`) wie beim Ablegen (`instance::place`), soweit das Shape einen ihrer Werte
   zeigt. Name, Beschreibung und Kategorien im Regelsatz-Teil kommen in der gewählten Sprache
   aus `JFD_RuleSetN`/`JFD_RuleN`. Danach fallen `JFD_Name`, `JFD_Description`, `JFD_Category`,
   `JFD_Order`, `JFD_Starter`, `JFD_Default`, `JFD_Text` und `JFD_LayerN` weg. `JFD_Template`,
   `JFD_Registers`, `JFD_Tip` und die Regel-Übersetzungen bleiben. Neu berechnet werden nur die
   geänderten Shapes (und was von ihnen abhängt).
3. Die App öffnet die Schablonen aus dem Fensterteil in dieser Reihenfolge und klappt die aktive
   auf (`template::stencil_windows`).

Speichern als Vorlage (EX-11) schreibt die Zeichnung als `.vstx` und setzt vorher den Fensterteil
auf die offenen Schablonen (`template::set_stencil_windows`).

## Prüfen

- `diagramiotest --check --only templates`: jede Vorlage gebaut, gelesen, geschrieben und wieder
  gelesen (gleiches Modell), Schablonen, Seiten, Zellen, Regelsatz, Startdiagramme,
  `to_drawing`. diagramiotest bettet dafür alle Quellen von `stencils/` und `templates/` ein
  (sein `build.rs`), eine neue Vorlage wird ohne weitere Zeile mitgeprüft.
- `diagramiotest --write-templates ORDNER`: alle `.vstx` und die erste Seite jedes Startdiagramms
  als PNG.
- `JFDiagrammer --write-templates ORDNER`: alle `.vstx` zum Ansehen in Visio.
- Unit-Tests: `cargo test -p jflib-diagram-io --lib` (Parser, Bauen, Fensterteil, `to_drawing`).
