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

## Bauen, starten, testen

```sh
python build_rs.py app JFDiagrammer          # build/win/JFDiagrammer.exe und build/linux/JFDiagrammer
python build_rs.py run JFDiagrammer          # bauen und starten (Linux: --target linux)
python build_rs.py test JFDiagrammer         # bauen und tests/*.uis ausführen
python build_rs.py i18n JFDiagrammer         # der Sprachkatalog gegen die Quellen
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
bekommen Englisch. Umschalten in den Einstellungen kommt mit den Optionen (JD8). Ein deutsches
Wort mit zwei Bedeutungen trägt im Katalog einen Kontext nach `|` (`Einfügen|Zwischenablage` ist
„Paste“, `Einfügen` das Register „Insert“), `tc()` zeigt ihn nie an.

## Dateien

| Datei | Inhalt |
|---|---|
| `src/main.rs` | Einstieg, Katalog, Zustand der App |
| `src/shell.rs` | das Fenster: Registerzeile, Menüband, Seitenleiste, Zeichenfläche, Statusleiste |
| `src/ribbon.rs` | die Platzhalter-Gruppen der vier Register |
| `src/icons.rs`, `src/icons_data.rs` | die eigenen Menüband-Icons (Hook, Farben; Daten aus `mkicons.py`) |
| `icons/icons.txt`, `icons/*.svg`, `mkicons.py` | Liste und Quellen der Icons, ihr Skript; `icons/probeblatt.png` (abgenommen), `icons/uebersicht.png` |
| `locales/ui.tsv` | die Texte in fünf Sprachen |
| `tests/*.uis` | die Testskripte (`smoke.uis` deutsch, `lang_*.uis` je Sprache) |
| `STYLE`, `TARGETS`, `PACKAGE`, `PERMISSIONS` | Stil (`netlab`), Zielplattformen, Identität (`org.jfreund.JFDiagrammer`), Rechte |
| `android/` | Manifest und Platzhalter-Symbol für die APK (`--target android`) |

Der Ordner `context/` (Visio-Screenshots, Bücher, Testdateien) liegt nur lokal, ist gitignored
und kommt nie ins Repository.
