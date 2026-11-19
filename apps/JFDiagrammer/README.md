# JFDiagrammer

Diagramme zeichnen wie in Visio, ohne VBA: Shapes aus Schablonen, Verbinder, Container, Designs,
Öffnen und Speichern der VSDX-Familie. Die App baut nur auf der öffentlichen `jflib`-API auf und
zieht später in ein eigenes Repository um (wie JFNetworker). Modell und Dateiformate entstehen als
Bibliotheks-Crates [`lib/diagram_core`](../../lib/diagram_core) und [`lib/diagram_io`](../../lib/diagram_io).

- [FEATURES.md](FEATURES.md): die Feature-Liste mit den Entscheidungen des Nutzers
- [PLAN.md](PLAN.md): Phase 2 in Etappen JD0–JD14, Architekturentscheidungen V1–V5

**Stand: JD0 (Gerüst).** Das Hauptfenster im NetLab-Style zeigt die Registerzeile mit Start,
Einfügen, Entwurf und Ansicht, das Menüband mit Platzhalter-Gruppen (alle Knöpfe deaktiviert),
links die Seitenleiste „Shapes“ ohne Schablone, in der Mitte eine leere Zeichenfläche und unten
die Statusleiste mit Seite und Zoom. Funktionen gibt es noch keine.

**JD14 (Icons):** alle 303 Menüband-Icons aus FEATURES.md Abschnitt 7 sind gezeichnet, im
NetLab-Stil nach dem abgenommenen Probeblatt ([`icons/probeblatt.png`](icons/probeblatt.png));
die Übersicht steht in [`icons/uebersicht.png`](icons/uebersicht.png). Die Platzhalter im
Menüband tragen sie schon (nur die Designs-Galerie nicht, Galerien bekommen Vorschauen).

**JD8 (App-Hülle):** eigene Titelleiste mit Schnellzugriff (Speichern, Rückgängig, Wiederholen),
Registerzeile mit Datei, Start, Einfügen, Entwurf, Ansicht und den Kontextregistern
Container-Tools › Format und Textwerkzeuge › Text, das Menüband mit allen P1-Befehlen dieser
Register (32-px-Icons auf großen Knöpfen), Zugriffstasten (Alt oder F10), Backstage mit Neu
(Vorlagen-Vorschau), Öffnen, Speichern, Drucken, Exportieren, Info und Optionen (Allgemein,
Speichern, Sprache, Erweitert, Tastenkürzel), Aufgabenbereiche im Dock (Shapes, Shape formatieren,
Größe und Position, Shape-Daten, Verschieben und Zoomen), Statusleiste mit Seite, Maßen und Zoom,
Kontextmenüs für Shape, Seite, Seitenregister und Schablone. Die Sprache wechselt in den Optionen
ohne Neustart; jede Sprache hat ihre Office-Tastenkürzel ([`lang/shortcuts.tsv`](lang/shortcuts.tsv)).
Befehle ohne Funktion sind sichtbar und ausgegraut: [`src/commands.rs`](src/commands.rs) hat je
Befehl eine Zeile, spätere Etappen hängen nur einen Handler in `HANDLERS` an.

**JD9 (Seitenleiste und Schablonen):** Die Seitenleiste „Shapes“ ist das Schablonen-Widget der
Bibliothek (`ui.stencil_panel`):
- Aufbau: Suchfeld mit Verlauf, „Weitere Shapes“ mit den Kategorien, Titel der offenen Schablonen,
  Schnell-Shapes je Schablone und als Sammelansicht, vier Ansichten, Leerzustand.
- Die Suche läuft über die Namen in allen fünf Sprachen, NameU und die Stichworte.
- Favoriten werden als `.vssx` im Benutzerordner gespeichert.
- Shapes legt man per Ziehen, per Klick + Klick oder mit Enter ab. Der Master kommt dabei einmal ins
  Dokument, das Shape heißt wie in der aktuellen Sprache (NameU bleibt der Schlüssel), alles in einer
  Transaktion mit Neuberechnung.

Die mitgelieferten Schablonen sind `.vssx`, die beim Bauen aus Textquellen in
[`stencils/`](stencils/README.md) entstehen (Entscheidung V2, `build_steps.rs` mit
`diagram_io::stencil`), metrisch und in US-Einheiten. Gewählt wird nach der Region. Den Startsatz
„Standard-Shapes“ mit sieben Grundformen schreibt `JFDiagrammer --write-stencils ORDNER` zum Ansehen
in Visio heraus.

## Bauen, starten, testen

```sh
python build_rs.py app JFDiagrammer          # build/win/JFDiagrammer.exe und build/linux/JFDiagrammer
python build_rs.py run JFDiagrammer          # bauen und starten (Linux: --target linux)
python build_rs.py test JFDiagrammer         # bauen und tests/*.uis ausführen
python build_rs.py i18n JFDiagrammer         # der Sprachkatalog gegen die Quellen
python apps/JFDiagrammer/check_lang.py       # Katalog, Glossar, Tastenkürzel
```

## Icons

Die Menüband-Icons sind eigene Zeichnungen (FEATURES.md Abschnitt 7). [`icons/icons.txt`](icons/icons.txt)
listet alle mit Nummer (Codepunkt), Größen, Priorität, Name, Ort und Motiv; je Größe gibt es eine
SVG-Datei von Hand auf dem Pixelraster: `icons/NAME_16.svg` (jeder Befehl) und `icons/NAME_32.svg`
(große Knöpfe). Fett, Kursiv und Unterstrichen gibt es je Buchstabe, die App wählt nach Sprache.
Linien 1 px auf der Pixelmitte, Ecken eckig, vier Farbrollen über die Klassennamen (`ink`, `body`,
`tint` für den Akzent, `hi` für die Akzentfläche; die Farben in den Dateien gelten nur fürs
Ansehen). Die Regeln stehen in [`mkicons.py`](mkicons.py); das Skript macht aus den SVGs gefüllte
Flächen für `ui::Symbol` (`src/icons_data.rs`) und auf Wunsch das Probeblatt:

```sh
python apps/JFDiagrammer/mkicons.py --strict                          # src/icons_data.rs
python apps/JFDiagrammer/mkicons.py --overview apps/JFDiagrammer/icons/uebersicht.png
python apps/JFDiagrammer/mkicons.py --part kontext --prio P2 --overview x.png   # ein Ausschnitt
```

## Sprachen

Deutsch, Englisch, Spanisch, Italienisch und Französisch (FEATURES.md 3.17). Die Texte stehen
auf Deutsch im Code und übersetzt in [`locales/ui.tsv`](locales/ui.tsv) mit den Spalten
`de en es it fr`. Die Sprache folgt `--lang`, `jfgui_LANG` oder dem System; Sprachen ohne Spalte
bekommen Englisch. In der App wechselt Datei › Optionen › Sprache sie sofort, ohne Neustart.
Verbindliche Begriffe stehen im Glossar [`lang/glossary.tsv`](lang/glossary.tsv) (LG-06), die
Tastenkürzel je Sprache nach der Office-Belegung in [`lang/shortcuts.tsv`](lang/shortcuts.tsv)
(LG-08; eigene Kürzel im Abschnitt `[keys]` der Einstellungsdatei); `check_lang.py` prüft
Katalog, Glossar und Kürzel (LG-11). Ein deutsches
Wort mit zwei Bedeutungen trägt im Katalog einen Kontext nach `|` (`Einfügen|Zwischenablage` ist
„Paste“, `Einfügen` das Register „Insert“), `tc()` zeigt ihn nie an.

## Dateien

| Datei | Inhalt |
|---|---|
| `src/main.rs` | Einstieg, Katalog, Zustand der App |
| `src/shell.rs` | das Fenster: Titelleiste mit Schnellzugriff, Registerzeile, Menüband, Dock, Statusleiste |
| `src/commands.rs` | jeder Befehl: Beschriftung, Icons, Handler, Zustand, Befehlstabelle je Sprache |
| `src/ribbon.rs` | die Register Start, Einfügen, Entwurf, Ansicht und die Kontextregister |
| `src/backstage.rs`, `src/options.rs` | Datei-Register (Backstage) und Optionen (`base::settings`) |
| `src/panes.rs` | Aufgabenbereiche, Zeichenfläche mit Seitenregistern, Statusleiste, Kontextmenüs |
| `src/lang.rs` | die fünf Sprachen, Umschalten zur Laufzeit, Buchstaben-Icons für F/K/U |
| `src/viewer.rs` | (JD4) die geöffnete Zeichnung: Laden, Seiten, Zeichnen über `diagram_view` |
| `src/stencils.rs`, `src/stencils_panel.rs`, `src/stencils_drop.rs` | (JD9) Schablonen, Favoriten, Suche, „Weitere Shapes“; das Panel mit Vorschauen und Menüs; Ablegen auf der Seite |
| `build.rs`, `build_steps.rs`, `stencils/` | (JD9) die Quellen der mitgelieferten Schablonen und ihr Bau zu `.vssx` ([`stencils/README.md`](stencils/README.md): das Quellformat) |
| `lang/shortcuts.tsv`, `lang/glossary.tsv`, `check_lang.py` | Tastenkürzel je Sprache, Glossar, Sprachprüfung |
| `src/icons.rs`, `src/icons_data.rs` | die eigenen Menüband-Icons (Hook, Farben; Daten aus `mkicons.py`) |
| `icons/icons.txt`, `icons/*.svg`, `mkicons.py` | Liste und Quellen der Icons, ihr Skript; `icons/probeblatt.png` (abgenommen), `icons/uebersicht.png` |
| `locales/ui.tsv` | die Texte in fünf Sprachen |
| `tests/*.uis` | die Testskripte (`smoke.uis`, `backstage.uis`, `options.uis`, `panes.uis`, `keytips.uis`, `context*.uis` deutsch, `lang_*.uis` je Sprache, `lang_switch.uis` der Wechsel, `viewer.uis`, `stencils*.uis` die Seitenleiste) |
| `STYLE`, `TARGETS`, `PACKAGE`, `PERMISSIONS` | Stil (`netlab`), Zielplattformen, Identität (`org.jfreund.JFDiagrammer`), Rechte |
| `android/` | Manifest und Platzhalter-Symbol für die APK (`--target android`) |

Der Ordner `context/` (Visio-Screenshots, Bücher, Testdateien) liegt nur lokal, ist gitignored
und kommt nie ins Repository.
