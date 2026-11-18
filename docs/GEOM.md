# GEOM -- Modellieren mit JFScript

`lib/geom` (`jflib-geom`, in der Fassade `jflib::geom`) ist der Modellierkern der Spiel-Engine, `apps/modeltool` die
Rückmeldeschleife darum ([GAME.md](GAME.md), Stufe G7g). Der Gedanke dahinter: Eine KI modelliert gut, wenn sie keine
einzelnen Ecken setzt, sondern Modelle aus Regeln, Bausteinen und Parametern beschreibt. Das Skript (JFScript,
Entscheidung E4) beschreibt; die Geometrie rechnet Rust in `lib/geom`; `modeltool` baut daraus die Meshes, prüft sie,
zeichnet sie aus festen Kameras und schreibt einen maschinenlesbaren Bericht und eine glTF-Datei. Die Anbindung an
JFScript liegt nur in `apps/modeltool` (Entscheidung E7: JFScript kommt per git wie bei `editortest`, ein Crate auf
crates.io darf nicht an einem git-Stand hängen); `lib/geom` hat eine reine Rust-API, `#![no_std]`,
`#![forbid(unsafe_code)]`, keine fremden Crates, abhängig nur von `rt`, und rechnet **bitgleich auf allen Zielen**:
Dasselbe Skript gibt auf Windows, Linux, macOS und im Browser dasselbe Modell, Bit für Bit.

Stand: G7g Teil 1 (dieser Lauf). Nicht in Teil 1: Boolesche Operationen auf konvexen Körpern, Abschrägen, der
Beobachtungsmodus für Menschen mit einer `gpu_view`, die beim Speichern neu baut, und der Bewertungslauf, in dem ein
Agent Bezugsobjekte aus Beschreibungen baut ([Für Teil 2](#für-teil-2)).

| Teil | Wo |
| --- | --- |
| Vektoren, Boxen, affine Abbildungen; Wurzel, sin, cos, atan2, Winkel in Grad | [`math.rs`](../lib/geom/src/math.rs) |
| das Netz: Daten, Abbildungen, Spiegeln, Felder, Zusammenfügen, Verschweißen | [`mesh.rs`](../lib/geom/src/mesh.rs) |
| Normalen (flach, glatt mit Schwellwinkel) und UV per Box-Projektion | [`normals.rs`](../lib/geom/src/normals.rs) |
| Grundkörper, Extrudieren, Grundriss hochziehen, Sweep, Drehkörper | [`build.rs`](../lib/geom/src/build.rs) |
| Profile: Fläche, Aufräumen, Zerlegen in Dreiecke, Rechteck, Kreis, Bogen | [`poly.rs`](../lib/geom/src/poly.rs) |
| die Prüfungen und ihr Bericht | [`check.rs`](../lib/geom/src/check.rs) |
| glTF schreiben | [`lib/gltf/write.rs`](../lib/gltf/write.rs) |
| die Skript-Funktionen, Materialien, `--api` | [`apps/modeltool/src/host.rs`](../apps/modeltool/src/host.rs) |
| Skript laufen lassen, Fehler, Messungen, Bericht | [`run.rs`](../apps/modeltool/src/run.rs) |
| Bilder über render3d | [`render.rs`](../apps/modeltool/src/render.rs) |
| Tests, Referenzen, Prüfsumme | [`selftest.rs`](../apps/modeltool/src/selftest.rs), [`refs.rs`](../apps/modeltool/src/refs.rs), [`checksum.rs`](../apps/modeltool/src/checksum.rs), im Browser [`tests/web.mjs`](../apps/modeltool/tests/web.mjs) |
| Beispiele mit ihren Bildern und Berichten | [`apps/modeltool/examples`](../apps/modeltool/examples) |

## Ein Beispiel

```js
// tisch.jfs: eine Platte auf vier Beinen
model.name("Tisch")
model.material("eiche", {color: "#b8864e", roughness: 0.7})
let h = params.get("hoehe", 0.75)                     // modeltool tisch.jfs --param hoehe=0.9
let bein = geom.cylinder(0.025, h - 0.04, 24).move(0.55, 0, 0.35)
let platte = geom.box(1.2, 0.04, 0.8).move(0, h - 0.04, 0)
let beine = geom.merge(bein, bein.mirror("x"))
model.add(platte, "eiche", "platte")
model.add(geom.merge(beine, beine.mirror("z")), "eiche", "beine")
model.budget(1000)                                    // Prüfung: höchstens 1000 Dreiecke
model.expect_size(1.2, h, 0.8)                        // Prüfung: diese Maße auf 1 mm
```

```text
> modeltool tisch.jfs
tisch.jfs: ok - 2 Teile, 1 Materialien, 396 Dreiecke, 448 Ecken; 1.2 x 0.75 x 0.8 m (B x H x T); Fläche 2.54 m², Volumen 0.044 m³
  geprüft: geschlossen, Normalen außen, keine entarteten Dreiecke, Budget 1000 eingehalten, Maße wie erwartet
  Bilder (Gitter 0.2 m): tisch.out/sheet.png (vorn | rechts / oben | schräg), front.png, side.png, top.png, iso.png
  Bericht: tisch.out/report.json; glTF: tisch.out/model.glb
```

Und ein Fehler, so wie die KI ihn liest (`--param hoehe=0`; die Spalte ist die des Funktionsnamens, so meldet JFScript
einen Aufruf):

```text
tisch.jfs:5:17: Fehler: geom.cylinder: die Höhe muss eine Zahl größer 0 sein, nicht -0.04
  5 | let bein = geom.cylinder(0.025, h - 0.04, 24).move(0.55, 0, 0.35)
    |                 ^
  im Hauptprogramm (tisch.jfs:5:17)
  Bericht: tisch.out/report.json
```

Ein zweiter Lauf ohne Änderung baut nicht neu: `tisch.jfs: ok - unverändert seit dem letzten Lauf (derselbe Schlüssel),
nicht neu gebaut`.

Dieselbe Geometrie in Rust:

```rust
use jflib::geom::{self, Axis, Mesh, Vec3};

let platte = geom::cuboid(1.2, 0.04, 0.8)?.translated(Vec3::new(0.0, 0.71, 0.0));
let bein = geom::cylinder(0.025, 0.71, 24)?.translated(Vec3::new(0.55, 0.0, 0.35));
let beine = Mesh::merge(&[&bein, &bein.mirrored(Axis::X)])?;
let tisch = Mesh::merge(&[&platte, &beine, &beine.mirrored(Axis::Z)])?.uv_box(1.0);
let bericht = tisch.check();                          // closed(), outward(), degenerate, size(), json()
let mesh = render3d::MeshData { positions: &tisch.positions_f32(), normals: &tisch.normals_f32(),
                                uvs: &tisch.uvs_f32(), indices: &tisch.indices };
```

## Die Skript-API

`modeltool --api` druckt dieselbe Liste. Längen in Metern, Winkel in Grad, Y oben, vorn ist +Z. **Jeder Grundkörper
steht auf y = 0, mittig über dem Ursprung** (das ist die Regel, auf die sich ein Skript beim Stapeln verlässt). Ein
Mesh ändert sich nie: Jede Operation gibt ein neues. Punkte und Vektoren sind Listen `[x, y, z]`, Profile Listen von
`[x, y]`.

| Funktion | Was sie tut |
| --- | --- |
| `geom.box(b, h, t)` | Quader b × h × t (x, y, z) |
| `geom.cylinder(r, h, segmente = 32)` | Zylinder um die Y-Achse; glatte Seite, flache Deckel |
| `geom.cone(r, h, segmente = 32, r_oben = 0)` | Kegel, mit `r_oben` > 0 ein Kegelstumpf |
| `geom.sphere(r, segmente = 32, ringe = segmente / 2)` | Kugel, Mitte bei y = r |
| `geom.torus(R, r, segmente = 32, seiten = 16)` | Torus, flach auf y = 0 |
| `geom.stairs(b, h, t, stufen)` | Treppe als Block, erste Stufe vorn (z = t/2), steigt nach hinten |
| `geom.arch(b, h, t, staerke, segmente = 16)` | Torbogen: zwei Pfeiler, Öffnung mit Halbkreis, darüber die Stärke |
| `geom.extrude(profil, tiefe)` | Profil `[[x, y], ...]` (Ansicht von vorn) entlang Z, mittig um z = 0 |
| `geom.prism(grundriss, h)` | Grundriss `[[x, z], ...]` (Ansicht von oben) von y = 0 nach oben |
| `geom.sweep(profil, pfad, geschlossen = false, glatt = 30)` | Profil entlang eines Pfads `[[x, y, z], ...]`, Ecken auf Gehrung; Profil-x rechts des Pfads, Profil-y oben (bei senkrechtem Pfad: x ist X, y ist Z) |
| `geom.lathe(profil, segmente = 32, glatt = 30)` | Drehkörper: Profil `[[radius, y], ...]` um die Y-Achse |
| `geom.merge(m1, m2, ...)` oder `geom.merge([m1, ...])` | zu einem Mesh |
| `m.move(x, y, z)`, `m.place(x, y, z)`, `m.center()` | verschieben; die Mitte der Unterseite der Box nach (x, y, z); die Mitte der Box in den Ursprung |
| `m.rotate_x(grad)`, `rotate_y`, `rotate_z`, `m.rotate(grad, [ax, ay, az])` | drehen um eine Achse durch den Ursprung (rechtshändig; Vielfache von 90° exakt) |
| `m.scale(s)`, `m.scale(sx, sy, sz)` | skalieren um den Ursprung |
| `m.mirror("x")`, `"y"`, `"z"` | spiegeln an der Ebene durch den Ursprung; die Flächen bleiben außen |
| `m.flip()` | innen nach außen |
| `m.weld(toleranz = 0)` | Ecken an derselben Stelle zu einem Punkt verbinden (eine Hälfte und ihr Spiegelbild zu einer Fläche) |
| `m.array(anzahl, dx, dy, dz)`, `m.radial(anzahl, schritt = 360 / anzahl)` | Kopien in einer Reihe; um die Y-Achse |
| `m.flat()`, `m.smooth(grad = 30)` | flache Normalen; glatte über Kanten flacher als `grad` |
| `m.bounds()`, `m.area()`, `m.volume()`, `m.check()`, `m.tris` | `{min, max, size, center}`; m²; m³; `{ok, closed, outward, ...}`; die Zahl der Dreiecke |
| `profile.rect(b, h)`, `circle(r, n = 32)`, `rounded(b, h, r, n = 4)`, `arc(r, von, bis, n = 16)` | Profile um den Ursprung (`arc`: ein offener Linienzug, beide Enden dabei) |
| `profile.move(p, dx, dy)`, `scale(p, sx, sy)`, `rotate(p, grad)`, `area(p)` | Profile ändern und messen |
| `vec.add`, `sub`, `mul`, `dot`, `cross`, `len`, `dist`, `norm`, `lerp` | Vektoren als Listen aus 2 oder 3 Zahlen |
| `model.add(m, material = "grau", name)` | ein Teil des Modells; das Material als Name oder `{color, roughness, metallic}` |
| `model.material(name, {color: "#rrggbb" oder [r, g, b], roughness = 0.8, metallic = 0})` | ein Material (Farben in sRGB, 0 bis 1) |
| `model.name(text)`, `model.budget(dreiecke)`, `model.expect_size(x, y, z, toleranz = 0.001)` | Name; Budget; erwartete Maße (`nil` für eine Achse, die nicht zählt) |
| `model.uv_scale(meter = 1)` | Meter je UV-Einheit der Box-Projektion |
| `params` | die Parameter von `--param NAME=WERT` (Zahlen, `true`/`false`, sonst Text): `params.get("b", 0.8)` |

Eingebaute Materialien: grau, weiss, schwarz, holz, dunkelholz, stein, beton, ziegel, dach, metall, rot, gruen, blau,
gelb, glas (deckend). `import("datei.jfs")` lädt ein Modul relativ zum Skript ([`haus_teile.jfs`](../apps/modeltool/examples/haus_teile.jfs)).

## Die Ausgabe

`modeltool DATEI.jfs [--out ORDNER] [--param NAME=WERT ...] [--size PX] [--no-grid] [--no-images] [--force]` schreibt
in den Ordner (ohne `--out`: der Name des Skripts mit `.out`):

| Datei | Inhalt |
| --- | --- |
| `report.json` | `ok`, `errors` (Datei, Zeile, Spalte, Text, Aufrufe), `findings` (was nicht passt, deutsch, ein Satz je Fund), `params`, `model` (Teile, Materialien, Ecken, Dreiecke, Budget, Box, Maße, Mitte, Fläche, Volumen), `checks` (der Bericht von `lib/geom` über das ganze Modell), `parts` (je Teil Material, Prüfung, Dreiecke, Box, Fläche, Volumen), `expect` (die erwarteten Maße und die gemessenen), `materials`, `fingerprint` (FNV-1a über die Bits aller Teile), `output` (was das Skript gedruckt hat), `imports` (Module mit SHA-256), `files`, `views` (Größe, Gitterabstand, Kameras, GPU), `timing` (Skript, Geometrie, Prüfungen, Zeichnen, Export in ms; der Brennstoff des Skripts) |
| `front.png`, `side.png`, `top.png` | von vorn (+Z), von rechts (+X), von oben (vorn unten im Bild); alle drei im selben Maßstab, mit einem Gitter dahinter |
| `iso.png` | von vorn rechts oben, mit einem Gitter am Boden |
| `sheet.png` | die vier in einem Bild: vorn und rechts oben, von oben und schräg unten |
| `model.glb` | je Teil ein Mesh und ein Knoten mit seinem Material; die Szene heißt wie das Modell |

Exit-Code 0: gebaut und alle Prüfungen bestanden; 1: Fehler im Skript; 3: gebaut, aber eine Prüfung fehlgeschlagen; 2:
falscher Aufruf. `--report DATEI.jfs` gibt nur den Bericht auf stdout, `--api` die Funktionen.

## API-Überblick (Rust)

- **Netz**: `Mesh { positions, normals, uvs, points, indices }` (`f64`; `points`: die Topologie, siehe unten);
  `Mesh::new`, `from_triangles(positionen, indizes)` (jede Ecke ihr eigener Punkt), `validate`, `vertex_count`,
  `point_count`, `triangle_count`, `triangle`, `corners`, `bounds`, `area`, `volume`; `transformed(&Affine)`,
  `translated`, `rotated(Axis, grad)`, `scaled`, `mirrored(Axis)`, `flipped`; `append`, `Mesh::merge(&[&Mesh])`,
  `array(anzahl, schritt)`, `radial(anzahl, grad)`, `weld(toleranz)`; `flat`, `smooth(grad)`, `uv_box(meter_je_einheit)`;
  `positions_f32`, `normals_f32`, `uvs_f32`; `fingerprint`, `Fnv`.
- **Erzeugen** (`geom::build`, an der Wurzel): `cuboid`, `cylinder`, `cone`, `sphere`, `torus`, `stairs`, `arch`,
  `extrude`, `prism`, `sweep(profil, pfad, geschlossen, glatt)`, `lathe(profil, segmente, glatt)`; `DEFAULT_SMOOTH_DEG`
  (30). Grenzen: `MAX_TRIANGLES` (2²⁴), `MAX_SEGMENTS` (4096), `MAX_COPIES` (100 000).
- **Profile** (`geom::poly`): `signed_area2`, `area`, `clean`, `ccw`, `check_simple`, `triangulate`, `rect`, `circle`,
  `arc`, `rounded_rect`; `MERGE_RATIO`, `MAX_PROFILE_POINTS`.
- **Prüfen**: `Mesh::check() -> Report { vertices, triangles, points, edges, bounds, area, volume, components, euler,
  open_edges, nonmanifold_edges, flipped_edges, nonmanifold_points, inverted_components, degenerate, bad_normals }`
  mit `closed()`, `outward()`, `ok()`, `size()`, `json()`; `DEGENERATE_RATIO`, `NORMAL_TOLERANCE`, `fnum`.
- **Mathematik** (`geom::math`, die Typen an der Wurzel): `Vec3`, `Aabb`, `Affine` (`IDENTITY`, `translation`, `scale`,
  `mirror`, `rotation(Axis, grad)`, `rotation_about(achse, grad)`, `then_after`, `apply`, `apply_vector`, `determinant`,
  `normal_matrix`, `is_similarity`), `Axis`; `sqrt`, `sqrt_portable`, `sin`, `cos`, `atan2`, `sin_cos_deg`, `angle_deg`,
  `floor`, `abs`, `min`, `max`; `MAX_DEGREES`.
- **Fehler**: `Error::{Param, Profile, Path, TooBig, Invalid}` mit deutschem Text (`message()`, `Display`).
- **glTF** (`lib/gltf`): `write_document(&Document) -> Result<Vec<u8>>`.

## Entscheidungen

### Die Topologie steht im Netz: Punkte

Ein Mesh für die GPU hat an jeder harten Kante und an jeder Naht der UV zwei Ecken an derselben Stelle. Normalen und
Prüfungen brauchen aber die Fläche dahinter. Der erste Entwurf verschweißte dafür nach der Lage (gleiche Bits, gleicher
Punkt). Das scheiterte an einer Wand aus Stücken: Zwei Quader, die sich berühren, teilen dann Kanten, und jede dieser
Kanten hatte vier Dreiecke, also „nicht mannigfaltig" für ein Modell, das völlig in Ordnung ist. Deshalb trägt jede
Ecke ihren Punkt (`Mesh::points`): Die Erzeuger verbinden, was eine Fläche ist (die Ringe eines Drehkörpers, die Pole),
das Teilen für Normalen und UV behält den Punkt, Zusammenfügen hält die Teile getrennt. Was zusammengehört, aber getrennt
entstand (eine Hälfte und ihr Spiegelbild), verbindet `weld` ausdrücklich, exakt oder mit einer Toleranz.

### f64, Grad, und bitgleich überall

- **Nur `+ − × ÷` und korrekt gerundete Wurzeln** auf `f64` (die Wurzel wie in `lib/physics`: die des Prozessors auf
  x86-64 und ARM64, sonst `sqrt_portable`), sin, cos und atan2 aus `rt`s portabler Mathematik, kein FMA. JFScript
  rechnet Gleitkomma in Software und ist schon bitgleich; damit gilt das für die ganze Kette Skript → Mesh. Die Bits
  prüft die [Prüfsumme](#prüfsummen).
- **Sortieren mit eindeutigen Schlüsseln**, keine Hash-Tabellen, keine Threads: Ecken werden über Sortieren
  zusammengelegt (Schlüssel: Punkt, Normale, UV als Bits; bei Gleichstand die Ecke), Gruppen über Union-Find, bei dem
  die kleinere Wurzel gewinnt.
- **Winkel in Grad** (`sin_cos_deg`): Der Winkel wird exakt auf das nächste Vielfache von 90° und einen Rest bis 45°
  zerlegt; der Rest geht in sin und cos. Damit sind 90°, 180°, 270° exakt (0 und ±1), ein um 90° gedrehter Quader hat
  exakte Ecken, vier Kopien eines `radial` treffen sich ohne Spalt, und ein Zylinder mit einem Vielfachen von 4
  Segmenten hat seine Ecken genau auf den Achsen. Mit Bogenmaß wäre `sin(π/2)` zwar 1, `cos(π/2)` aber 6 · 10⁻¹⁷.
- `-0.0` wird überall zu `+0.0` (die Bits eines Punktes sind sein Schlüssel).

### Grundkörper und Profile

- **Alles steht auf y = 0, mittig über dem Ursprung**, auch Kugel und Torus. Für Häuser, Möbel und Requisiten ist das
  die Lage, in der man Teile stapelt (`place` setzt die Unterseite irgendwohin).
- Zylinder, Kegel, Kugel und Torus sind Drehkörper (`lathe`). Ihre Seiten sind glatt bis zu einem Schritt von 85°, so
  bleibt ein dreiseitiger Zylinder flach, statt Normalen zu bekommen, die von seinen Flächen weg zeigen.
- **Profile**: ein einfaches Vieleck, der Umlaufsinn ist egal (wird gedreht), aufeinanderfolgende Punkte näher als 10⁻⁹
  der Ausdehnung zählen als einer (`(y + r) - r` aus dem Skript liegt nicht immer genau auf `y`; ohne das gab die Säule
  einen Ring aus 80 Dreiecken ohne Fläche). Sich schneidende Kanten sind ein Fehler mit den Nummern der Kanten.
- **Deckel durch Ohrenschneiden** (ear clipping, n − 2 Dreiecke über den Punkten des Profils): Ein Punkt auf einer
  geraden Strecke ist nie die Spitze eines Ohrs, er bleibt am Rand, und die Wand daneben trifft ihn. Robust für die
  Profile eines Skripts; exakte Prädikate kommen mit den Booleschen Operationen (Teil 2).
- **Sweep**: Rahmen, die sich mit dem Pfad drehen, ohne zu verdrillen (rotationsminimierend: je Ecke die kleinste
  Drehung von einer Richtung zur nächsten), Ecken auf Gehrung (das Profil in der Ebene der Winkelhalbierenden, also
  derselbe Ring für beide Abschnitte). Ein geschlossener Pfad hinterlässt beim Umlauf eine Verdrillung (bei einem nicht
  ebenen Pfad); sie wird gleichmäßig auf die Abschnitte verteilt. Ein Knick über 172° ist ein Fehler.
- **Drehkörper**: Das Profil ist ein geschlossenes Vieleck rechts der Achse; Kanten auf der Achse machen keine Flächen,
  Punkte auf ihr werden Pole. So gibt ein Profil von der Achse um die Außenseite zur Achse einen Körper ohne Loch (eine
  Säule, eine Vase), eins abseits der Achse einen Ring.

### Normalen und UV

- **Glatt mit Schwellwinkel über Fächer**: Zwei Dreiecke teilen an einem Punkt eine Normale, wenn sie über eine Kante
  verbunden sind, die genau zwei Dreiecke in entgegengesetzter Richtung haben und deren Flächen um höchstens den
  Schwellwinkel abweichen. Die Normale einer Gruppe ist die Summe der Flächennormalen, gewichtet mit dem Winkel an der
  Ecke (unabhängig davon, wie fein eine Fläche zerlegt ist). Der Vergleich jeder Fläche mit jeder um einen Punkt (statt
  über Kanten) hätte die Pole einer Kugel zerfranst.
- **Skalieren mit verschiedenen Faktoren** rechnet die Normalen aus den abgebildeten Flächen neu, in denselben
  Glättungsgruppen (Punkt und alte Normale); Drehen, Spiegeln und gleichmäßiges Skalieren drehen sie nur mit.
- **UV per Box-Projektion im Weltmaß**: je Dreieck entlang der Achse, in die seine Normale am meisten zeigt (bei
  Gleichstand Y, dann X); eine Einheit ist ein Meter (`model.uv_scale`), nirgends gespiegelt. `modeltool` projiziert
  jedes Teil am Ende an seinem Platz.

### Was geprüft wird

| Prüfung | Bedeutung |
| --- | --- |
| geschlossen (`closed`) | jede Kante zwischen genau zwei Dreiecken in entgegengesetzter Richtung, um jeden Punkt ein einziger Fächer (sonst `open_edges`, `nonmanifold_edges`, `flipped_edges`, `nonmanifold_points`) |
| Normalen außen (`outward`) | einheitlicher Umlaufsinn und kein geschlossenes Teil mit negativem Volumen (`inverted_components`) |
| keine entarteten Dreiecke | kein Punkt doppelt, Höhe über der längsten Kante mindestens 10⁻⁹ dieser Kante |
| gute Normalen | Länge 1 auf 10⁻⁶, nicht von der eigenen Fläche weg (`bad_normals`) |
| Maße in Metern | Box, Größe, Mitte; gegen `model.expect_size` auf die Toleranz |
| Budget | die Dreiecke aller Teile gegen `model.budget` |
| Fläche, Volumen, Teile, Euler-Charakteristik | V − E + F je Teil summiert: 2 je geschlossenes Teil ohne Loch, 0 für einen Torus |

`modeltool` prüft das ganze Modell und jedes Teil. Teile dürfen sich berühren und durchdringen (für Bilder und Spiele
in Ordnung); sie zu einem Körper zu vereinen ist Sache der Booleschen Operationen.

### Die Skriptseite

- **Ein Mesh ist im Skript eine Map** `{mesh: id, tris: n}`, weil JFScript keine Host-Objekte hat. Methoden wie
  `m.move(..)` erreichen den Host als `map.move`, weil die Map kein Feld `move` hat und die Bibliothek keine
  Map-Methode dieses Namens. Darum heißt das Zusammenfügen `geom.merge` und nicht `m.merge` (das ist `map.merge` der
  Bibliothek). Meshes ändern sich nie; alle eines Laufs bleiben bis zu seinem Ende im Host (höchstens 4 Mio. Dreiecke
  zusammen, auch Zwischenstände: eine Grenze mit einer Meldung statt eines vollen Speichers).
- **Fehler auf Deutsch**, mit dem Namen der Funktion und des Parameters (`geom.box: die Höhe muss eine Zahl größer 0
  sein, nicht -2`), wie JFScripts eigene. JFScript setzt Datei, Zeile und Spalte des Aufrufs dazu; `modeltool` zeigt die
  Zeile mit einem Pfeil und die Aufrufe. `compile_with` mit den Namen des Hosts macht einen Tippfehler (`boks`) schon
  vor dem Lauf zum Fehler, mit Vorschlag.
- **Grenzen**: 64 MiB Heap, 200 Mio. Schritte (eine Endlosschleife endet nach rund 2 s), Brennstoff der
  Host-Funktionen nach der Arbeit (je 64 Dreiecke einer).
- **Cache-Schlüssel**: SHA-256 (die ersten 128 Bit) über die Version des Werkzeugs und von JFLib, das Skript, die
  sortierten Parameter und jedes importierte Modul mit Pfad und Inhalt, Zeilenenden als LF. Ein Lauf mit gleichem
  Schlüssel, gleichen Bildoptionen und vorhandenen Dateien baut nicht neu.

### Die Bilder

- **Kameras**: render3d hat nur perspektivische Kameras. Die drei Ansichten kommen deshalb aus großer Entfernung mit
  4° Sichtfeld (die Rückseite eines Modells erscheint höchstens ein paar Prozent kleiner); alle drei haben **denselben
  Maßstab** (die größte Ausdehnung des Modells), wie die Ansichten einer Zeichnung: Verhältnisse zwischen den Bildern
  stimmen. Die Schrägansicht hat 20° von vorn rechts oben (35° seitlich, 28° hoch).
- **Gitter**: eine Platte hinter dem Modell (vorn: hinter seiner Rückseite, rechts: links von ihm, oben: unter ihm und
  unter y = 0) mit Linien im Abstand 1, 2 oder 5 mal einer Zehnerpotenz (höchstens 12 über das Modell), die Linien durch
  den Ursprung dunkelblau. Der Abstand steht im Bericht.
- **Kantenglättung**: render3d hat keine; jedes Bild wird doppelt so groß gezeichnet und 2 × 2 gemittelt.
- **Licht**: eine Sonne von rechts oben vorn, Umgebungslicht 0,32; Farben des Skripts in sRGB, für render3d und glTF
  linear umgerechnet (`pow` aus `rt`). Die Bilder sind nicht Teil der Prüfsumme (GPUs unterscheiden sich im letzten Bit).

## Was JFScript braucht

Was in Teil 1 fehlte und wie `modeltool` sich behilft. Gebaut wird es im Repository von JFScript, nach einem Ja des
Nutzers (E7).

1. **Vektoren mit Operatoren.** Heute: `let b2 = vec.add(b, vec.mul(richtung, 0.1))`. Gewünscht: `let b2 = b +
   richtung * 0.1` mit `b.x`, `b.y`, `b.z`. Ein Typ `vec` (2 und 3 Komponenten, `f64`, unveränderlich) mit `+ − ×
   ÷` (Skalar), `==`, Feldern und Methoden (`len`, `norm`, `dot`, `cross`); Listen von Zahlen als Punkte müssten
   weiter gehen. Behelf: Listen und `vec.*`.
2. **Host-Objekte.** Heute ist ein Mesh die Map `{mesh: 3, tris: 12}`: Das Skript kann sie ändern (`m.mesh = 7`),
   `type(m)` sagt `map`, `m.len()` ist 2, Methodennamen der Map-Bibliothek (`merge`, `get`, `has`, `keys`, `values`,
   `len`, `delete`, `entries`) sind für Meshes verloren, und der Host kann nichts freigeben, weil er nicht erfährt, wann
   ein Mesh unerreichbar wird. Gewünscht: ein Wert `Host(typ, id)`, den das Skript nicht öffnen kann; `obj.m(..)` geht
   an `Host::call("mesh.m", ..)`; `type()` und `print` fragen den Host; der Sammler meldet freigegebene Ids
   (`Host::drop`).
3. **Die Stelle des Aufrufs für den Host.** `Cx` sagt nicht, in welcher Zeile die Host-Funktion gerufen wurde. So kann
   `model.expect_size` im Bericht nicht sagen, wo die Regel steht, und der Host kann keine Warnung mit Zeile geben
   („das Teil schwebt 2 cm über dem Boden"). Gewünscht: `cx.position() -> (datei, zeile, spalte)`, dazu Warnungen
   (`cx.warn(text)`), die nicht abbrechen.
4. **Benannte Argumente.** `geom.cylinder(0.3, 1.2, 32)` ist für eine KI fehleranfällig (was ist r, was h?).
   Gewünscht: `geom.cylinder(r: 0.3, h: 1.2)` als letzte Map übergeben, oder Standardwerte in der Signatur. Behelf:
   Positionsargumente mit Namen in den Fehlermeldungen und in `--api`.
5. **Mehr Mathematik.** Es fehlen `tan`, `asin`, `acos`, `atan`, `atan2`, `hypot` und Winkel in Grad, die bei 90° exakt
   sind (`math.sin_deg`). `haus_teile.jfs` rechnet `tan` als `math.sin(a) / math.cos(a)` mit `a = neigung * math.pi / 180`.
6. **Ganzzahl-Division.** `1 / 3` ist 0 und `(stufen + 2) / 3` schneidet ab: In Geometrie-Code ein leiser Fehler
   (die Säule braucht `i / 12.0`). Vorschlag: eine Warnung der Sprachdienste, wo zwei Ganzzahlen geteilt werden, oder
   `//` für die ganzzahlige Division.
7. **Bereiche mit Schritt und Zerlegen.** `for i in 0..n` nur ganzzahlig mit Schritt 1; eine Kurve abtasten heißt
   `t = i / 12.0` von Hand. Gewünscht: `range(a, b, schritt)`, `linspace(a, b, n)`.
8. **Zerlegen beim Zuweisen.** `let {rahmen, glas, bank} = teile.fenster(b, h)` und `let [x, y, z] = p` statt
   `f.rahmen` und `p[0]`.
9. **Beschreibungen der Host-Funktionen für die Sprachdienste.** `script::ls` kennt `HostItem`; `modeltool` hat die
   Liste in `host::API`. Für den Beobachtungsmodus und die IDE (G7o) gehören beide zusammen.

## Tests

`python build_rs.py test modeltool` führt `tests/geom.uis` (`--test prims,ops,normals,checks,poly`) und
`tests/examples.uis` (`--test gltf,script,examples`) aus; `modeltool --check` alle acht Gruppen (`--quick` ein Zehntel).
Die Referenzen in `refs.rs` teilen keinen Code mit `lib/geom`: Volumen und Flächen aus geschlossenen Formeln, die aus der
Geometrie hergeleitet sind. Ein Drehkörper mit n Segmenten ist ein Polyeder aus ebenen Trapezen; der Körper zwischen
zwei Meridianebenen ist ein lineares Bild des Prismas über dem Profil mit der Jacobi-Determinante ρ sin Δ, also ist sein
Volumen die diskrete Pappus-Regel V = n sin(2π/n) · ∫∫ρ dA, die Fläche je Profilkante n gleichschenklige Trapeze. Ein
Sweep mit Gehrung um den Schwerpunkt des Profils hat das Volumen Fläche × Länge der Mittellinie.

| Gruppe | Prüft | Ergebnis |
| --- | --- | --- |
| `prims` | je 150 bis 300 Quader, Zylinder, Kegel und Stümpfe, Kugeln, Tori, Treppen, Bögen mit zufälligen Maßen und Segmenten: geschlossen, außen, nichts entartet, Normalen gut, ein Teil, Euler 2 (Torus 0), Volumen und Fläche gegen die Referenz auf 10⁻¹¹, Boxen exakt (bei Vielfachen von 4 Segmenten); was verweigert wird | 12 609 Prüfungen, 0 falsch |
| `ops` | Drehungen um Vielfache von 90° exakt (Boxen Bit für Bit), 200 Drehungen um beliebige Achsen, Skalieren (auch gespiegelt), Spiegeln an allen Achsen, Umdrehen, Felder, `radial`: Volumen, Fläche, Teile; sich berührende Quader bleiben zwei geschlossene Teile; sechs einzelne Quadratflächen werden erst durch `weld` ein Würfel; 300 Sterne extrudiert und hochgezogen (auch im Uhrzeigersinn); Sweeps entlang eines L und eines geschlossenen Quadrats gegen Fläche × Länge, um eine nicht ebene geschlossene Schleife und eine Schraubenlinie; 300 Drehkörper zufälliger Profile gegen Pappus | 9 920 Prüfungen, 0 falsch |
| `normals` | flach: jede Ecke mit der Normale ihres Dreiecks; ein Quader bleibt bei 30° flach; Kugelnormalen zeigen von der Mitte weg; Zylinder: Deckel genau ±Y, Seiten waagrecht und radial auf 10⁻¹²; ein Achteck bei 10° flach; UV einer verschobenen Box je Fläche gleich den Koordinaten, in drei Maßstäben | 706 Prüfungen, 0 falsch |
| `checks` | absichtlich kaputte Netze: ein Loch (3 offene Kanten), ein gedrehtes Dreieck (3 gegenläufige Kanten), innen nach außen, ein Splitter und ein Dreieck mit doppeltem Punkt, eine Kante mit vier Dreiecken, zwei Würfel an einer Ecke, Nullnormalen, Normalen nach innen; Euler eines Torus und zweier Kugeln; das JSON | 13 Prüfungen, 0 falsch |
| `poly` | 2000 zufällige Sternvielecke mit 3 bis 80 Punkten zerlegt: n − 2 Dreiecke, alle mit Fläche, die Summe gleich der Fläche, jede Randkante einmal vorwärts, jede innere Kante einmal in jede Richtung; Punkte auf geraden Strecken; was verweigert wird; Rechteck, Kreis, abgerundete Rechtecke | 8 013 Prüfungen, 0 falsch |
| `gltf` | die fünf Beispiele geschrieben und gelesen: Meshes, Materialien, Knoten, Szenen gleich, keine Notizen; ein Knoten mit Matrix und Kind, ein Material mit allen Faktoren, MASK und doppelseitig; was verweigert wird (Textur, Index außerhalb, NaN) | 36 Prüfungen, 0 falsch |
| `script` | ein gutes Skript; zwölf kaputte: falsche Werte, falsche Typen, unbekannte Namen, Methoden, Materialien, sich schneidende Profile, nichts hinzugefügt, fehlende und kaputte Module, Endlosschleife, zu viele Dreiecke, falsche Achse: jede Meldung mit Datei, Zeile und Text; Pfeil unter der Spalte; Parameter, Module, Funden statt Fehlern, Abfragen; der Cache-Schlüssel (CRLF = LF) | 21 Prüfungen, 0 falsch |
| `examples` | die fünf Beispiele: laufen, bestehen alle Prüfungen, ihr Bericht gleich dem in `examples/out` (ohne Dateien, Ansichten, Zeiten) | 20 Prüfungen, 0 falsch |

Zusammen 31 338 Prüfungen in rund 3,8 s (Entwicklungs-Build, Ryzen 5 3600; `script` ist mit der Endlosschleife die
längste Gruppe).

## Prüfsummen

`modeltool --checksum` baut jedes Beispiel aus seinem Skript und faltet die Bits jedes Teils (Positionen, Normalen,
UV, Punkte, Indizes über `Mesh::fingerprint`), die Zahlen seiner Prüfung, den Brennstoff des Skripts und den Bericht des
ganzen Modells in FNV-1a; dazu 20 feste Operationen aller Arten (jeder Grundkörper mit ungeraden Segmenten, ein
Sternprofil extrudiert, ein abgerundeter Grundriss, ein Sweep um eine Schraubenlinie und eine nicht ebene geschlossene
Schleife, eine Vase, eine beliebige Drehung, ungleiches und gespiegeltes Skalieren, Felder, `radial` mit 7, `smooth`,
`flat` mit UV, `weld` mit Toleranz).

| Ziel | Build | Prüfsumme | `--check` |
| --- | --- | --- | --- |
| Windows x86-64 (Ryzen 5 3600), Rust 1.100 | Entwicklung und Release (fat LTO) | `8d93441afd1b79d7` | 31 338 bestanden |
| Linux x86-64 gegen glibc, in WSL Ubuntu 24.04 | Entwicklung | `8d93441afd1b79d7` | 31 338 bestanden |
| macOS ARM64 (Mac mini M2, macOS 27.2), Rust 1.98.1 | Entwicklung | `8d93441afd1b79d7` | 31 338 bestanden |
| WebAssembly in Node 22 (V8): einfach, SIMD, Threads | Entwicklung | `8d93441afd1b79d7` | 31 338 bestanden (alle drei) |

Zusammen 183 303 Werte. Auf x86-64 und ARM64 rechnet die Wurzel der Prozessor, im Browser `sqrt_portable`; JFScript
rechnet überall in Software. `--check` dauert im Entwicklungs-Build rund 3,8 s unter Windows, 5,8 s in WSL, 1,6 s auf
dem M2 und 10 bis 16 s in Node. Gezeichnet haben die RTX 2070 SUPER (Vulkan), der M2 (Metal) und llvmpipe in WSL; die
Bilder sahen gleich aus (verglichen mit dem Auge, nicht Pixel für Pixel).

## Messwerte

Die Beispiele auf dem Entwicklungsrechner (Ryzen 5 3600, RTX 2070 SUPER, Entwicklungs-Build), Millisekunden:

| Beispiel | Teile | Dreiecke | Skript | Geometrie | Prüfungen | Bilder (4 × 1024², gemittelt auf 512²) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Kiste | 28 | 444 | 1,1 | 0,5 | 0,6 | 1122 (das erste: GPU und Pipelines) |
| Treppe | 6 | 964 | 0,5 | 0,7 | 0,9 | 290 |
| Tür | 7 | 688 | 0,5 | 0,6 | 0,7 | 281 |
| Haus aus Modulen | 29 | 952 | 1,2 | 1,4 | 1,0 | 282 |
| Säule | 3 | 3304 | 1,2 | 3,4 | 2,9 | 332 |

Skript und Geometrie sind gegen die Bilder vernachlässigbar; die Rückmeldeschleife kostet rund 0,3 s je Lauf, davon
das meiste in render3d (Gerät, Szenen, vier Bilder mit Warten) und im PNG-Kodierer.

## Was nicht getestet ist

- Android und iOS (`cargo check -p jflib` für `aarch64-linux-android` geht mit und ohne `std`; `modeltool` steht nicht
  in ihren `TARGETS`), Windows und Linux auf ARM64, andere JavaScript-Engines als V8.
- Große Modelle: das größte Beispiel hat 3304 Dreiecke, die Tests gehen bis rund 20 000 Dreiecke je Mesh. Die
  Prüfungen und Normalen sortieren (n log n); gemessen ist das über 100 000 Dreiecke nicht.
- Sehr dünne oder sehr große Modelle (unter 1 mm, über 1 km): die Toleranzen sind relativ, die Gitter und Kameras
  passen sich an, geprüft ist es nicht. Ebenso Profile mit Punkten, die sich fast berühren (Ohrenschneiden ohne exakte
  Prädikate), und Sweeps mit Knicken nahe 172°.
- Die Bilder: kein Vergleich Pixel für Pixel zwischen GPUs und keine Bezugsbilder; geprüft ist, dass gezeichnet wird.
  Die HD 610 und die Iris Xe haben nicht gezeichnet.
- Glatte Normalen nach starkem ungleichen Skalieren grober Netze: Sie werden neu gerechnet, können aber noch von einer
  Fläche weg zeigen (der Bericht zählt sie als `bad_normals`; `smooth` mit kleinerem Winkel hilft).
- `--examples` und der Cache mit Modulen, die sich nach dem ersten Lauf ändern, sind nur von Hand probiert; das
  Werkzeug unter einem Pfad mit Leerzeichen oder Nicht-ASCII-Zeichen gar nicht.

## Was nicht gebaut ist

- Boolesche Operationen (Teil 2: zuerst konvexe Körper mit exakten Prädikaten), Abschrägen, Vereinfachen für
  Detailstufen (G7j), Löcher in Profilen (ein Fenster in einer Wand entsteht aus Stücken um die Öffnung), Teil-Drehkörper
  (unter 360°), Texturen, Farben je Ecke, Instanzen im glTF (jedes Teil ist ein eigenes Mesh).
- Orthografische Kameras und Umrisslinien in render3d (die Ansichten sind fast-orthografisch, gleich gefärbte Flächen
  einer Ebene sind in der Vorderansicht nicht zu unterscheiden: die Beispiele setzen dafür leicht andere Farben).
- Der Beobachtungsmodus mit `gpu_view` und Neubau beim Speichern, Beschriftungen in den Bildern (Maße, Achsen).
- Freigeben von Meshes während eines Laufs (wartet auf Host-Objekte in JFScript).

## Für Teil 2

- **Boolesche Operationen auf konvexen Körpern**: `Mesh::points` und die Prüfungen sind dafür da; exakte Prädikate
  (Orientierung mit adaptiver Genauigkeit wie bei Shewchuk) gehören dann auch in das Ohrenschneiden. Ein Fenster in einer
  Wand wäre dann ein Abzug statt Stücke um die Öffnung (`haus_teile.jfs` zeigt, wie es heute geht).
- **Abschrägen** (bevel) der Kanten eines Quaders und eines Profils: Profile lassen sich vorher abrunden
  (`profile.rounded`), Kanten von Körpern noch nicht.
- **Beobachtungsmodus**: eine `gpu_view` mit derselben Szene wie `render.rs` (Kameras, Gitter, Sonne), die beim
  Speichern des Skripts neu baut (die Bauzeit ist gegen das Zeichnen vernachlässigbar, siehe [Messwerte](#messwerte)).
  `host::API` gehört dann als `HostItem`s in die Sprachdienste.
- **Bewertungslauf**: ein Agent baut aus Beschreibungen in Text Bezugsobjekte (Kiste, Treppe, Tür, Haus aus Modulen,
  eine kleine Karte) mit den Bildern und dem Bericht als einziger Sicht; die fünf Beispiele hier sind die Messlatte.
- **render3d**: eine orthografische Kamera (statt 4° aus der Ferne) und Umrisslinien (G7f Teil 2, Cartoon-Material)
  machten die Ansichten lesbarer; heute unterscheiden sich gleich gefärbte Flächen einer Ebene nicht. Beschriftungen
  (Maße, Achsennamen) im Bild brauchten Text in render3d oder ein Zeichnen danach.
- **JFScript**: die Liste in [Was JFScript braucht](#was-jfscript-braucht), zuerst Host-Objekte (danach können Meshes
  freigegeben werden) und die Stelle des Aufrufs für Warnungen.
- **Geteilter Code mit `lib/physics`**: `math::sqrt`/`sqrt_portable` stehen in beiden Crates gleich; ein gemeinsamer
  Ort (etwa `rt`) wäre eine eigene Entscheidung. Ein Mesh von `lib/geom` geht über `positions_f32` und die Indizes in ein
  `physics::TriMesh`.
