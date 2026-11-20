# JF-Netzwerksymbole

Der gemeinsame Symbolsatz für Netzwerkgeräte von JFNetworker und JFDiagrammer (Entscheidung F9,
FEATURES.md SH-G09): eine Quelle, zwei Apps, gleiches Aussehen. Alles hier ist eigene Arbeit, nichts
ist aus Visio oder anderen Symbolsätzen übernommen oder nachgezeichnet.

**Ort:** Der Ordner wird ein eigenes Repository (Vorschlag `JFLibrary/JFSymbols`), das beide Apps als
git-Submodule auf festem Commit einbinden: JFNetworker an Stelle von `icons/`, JFDiagrammer hier
unter `apps/JFDiagrammer/symbols/`. Bis das Repository angelegt ist, liegt er in JFDiagrammer; der
Aufbau ist schon der des späteren Repositorys, beim Umstellen wird nur der Ordner gegen das
Submodule getauscht.

```
network/       Symbole im Raster 64 x 48 (Netzwerkpläne, JFNetworkers Geräte)
rack/          19-Zoll-Geräte in Frontansicht, Maße in mm (Rack-Diagramme, Maßstab 1:20)
roles/         Piktogramme der Serverrollen (12 x 12, weiß)
make_roles.py  erzeugt daraus network/server_<rolle>.svg und rack/server_<rolle>.svg
```

## network/: Symbole im Raster 64 x 48

Die 30 Symbole aus JFNetworkers `icons/` (Stand 2026-11-19, unverändert) und die neuen für die
Netzwerkschablonen von JFDiagrammer. Regeln (dieselben wie bei JFNetworkers `tools/vsym.py` in
JFLib, das die Dateien für `ui::Symbol` übersetzt):

- `viewBox="0 0 64 48"`, Zeichnung innerhalb 2 … 62 x 2 … 46.
- Wurzel: `fill="none" stroke="#2E343B" stroke-width="1.5" stroke-linejoin="round"
  stroke-linecap="round"`.
- Elemente: `path`, `rect` (`rx`), `circle`, `ellipse`, `line`, `polyline`, `polygon`, `g`,
  `transform`; `title` beschreibt das Symbol auf Englisch. Kein `text`, kein `image`, keine Verläufe.
- `class` nennt die Farbrolle („Füllung [Linie]“); JFNetworker färbt nach der Rolle aus seiner
  Palette, JFDiagrammer übernimmt die Farbwerte der Datei. Beide müssen also passen:

| Rolle | Farbe | wofür |
|---|---|---|
| `ink` | `#2E343B` | Umriss, Linien |
| `body` | `#F7F8FA` | Gehäuse, helle Flächen, weiße Piktogramme |
| `shade` | `#DCDFE3` | Blenden, Schächte, Ohren |
| `screen` | `#CAD4DF` | Bildschirme, Fenster |
| `port` | `#4F555D` | Anschlüsse, Lüftungsschlitze |
| `ok` | `#3C9A5F` | Leuchtanzeigen |
| `tint` | nach Kategorie | Endgeräte `#7A828C`, Netzwerk `#4F7396`, Sicherheit `#A65A4A`, Funk/WAN/Standorte `#3E8A86`, Server `#6A6390`, verteilte Systeme `#8E8540` |

- Serverrollen (`server_<rolle>.svg`): derselbe Tower, rechts unten eine Plakette in Server-Violett
  mit hellem Ring und weißem Piktogramm der Rolle. Nicht von Hand ändern: Piktogramm in
  `roles/<rolle>.svg` bearbeiten und `python apps/JFDiagrammer/symbols/make_roles.py` aufrufen
  (deterministisch, schreibt auch die 1-HE-Fassungen in `rack/`).
- Stapelfolge: beide Apps zeichnen in Dateireihenfolge (JFNetworkers `vsym.py`, JFDiagrammers
  Bauwerkzeug seit JD13); ein gefülltes Piktogramm auf einer Fläche gleicher Farbe braucht trotzdem
  einen Umriss, sonst verschwindet es.
- Dateiname = Name des Symbols (Kleinbuchstaben, `_`); JFNetworker spricht Symbole über ihn an.

## rack/: Frontansichten

- `viewBox` in Millimetern, Breite 482,6 (19-Zoll-Frontplatte mit Ohren), Höhe n x 44,45 (n HE).
- Aufbau wie die Symbole: Ohren (`shade`) mit Langlöchern (`port`), Gehäuse (`body`), Umriss
  (`ink`, 2,5 mm), links ein Farbfeld (`tint`) nach Kategorie, dann die Front des Geräts.
- Linienbreiten in mm (bei 1:20 auf dem Papier ein Zwanzigstel).

## Prüfen

`diagramiotest --symbol-sheet apps/JFDiagrammer/symbols/network OUT.png` zeichnet alle Symbole
eines Ordners mit JFDiagrammers eigener Darstellung auf ein Blatt (Fehler mit Datei und Zeile).
