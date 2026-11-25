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

Stand: G7g Teil 2 (2026-11-20). Teil 1 brachte den Kern, das Werkzeug und die fünf Beispiele; Teil 2 bringt
[Boolesche Operationen auf konvexen Körpern](#boolesche-operationen-brushes-exakt-entschieden) mit exakten
Prädikaten, das [Abschrägen](#abschrägen) von Kanten und Ecken, den [Beobachtungsmodus](#der-beobachtungsmodus)
(`modeltool --watch`), [Importe nur aus dem Ordner des Skripts](#importe-nur-aus-dem-ordner-des-skripts), Befunde
mit ihrem Ort und den [Bewertungslauf](#der-bewertungslauf): fünf Bezugsobjekte, die ein Agent aus Beschreibungen
gebaut hat, mit Bildern und Bericht als einziger Sicht. Mit Stufe S1 von JFScript (2026-11-24, JFScript c970852) sind
die Namen der Skript-API englisch, auch die der Parameter ([Namen englisch, Texte deutsch](#die-skriptseite)), und
`modeltool` gibt [Warnungen mit ihrer Zeile](#warnungen) aus; eine fehlgeschlagene Regel (`model.expect_size`,
`model.budget`) nennt die Zeile, in der sie steht. Mit Stufe S2 (2026-11-25, JFScript a21a0c3) sind Meshes und
Vektoren Objekte des Hosts: ein Mesh ist keine Map mehr, sondern ein `mesh`, das der Host freigibt, sobald das Skript
es nicht mehr hält; ein Vektor ist ein `vec` mit `+ − × ÷`; eine Beschreibung (`host::api()`) liefert `--api` und den
Sprachdiensten ihren Blick auf den Host, deren Warnungen `modeltool` jetzt mit ausgibt.

| Teil | Wo |
| --- | --- |
| Vektoren, Boxen, affine Abbildungen; Wurzel, sin, cos, atan2, Winkel in Grad | [`math.rs`](../lib/geom/src/math.rs) |
| das Netz: Daten, Abbildungen, Spiegeln, Felder, Zusammenfügen, Verschweißen | [`mesh.rs`](../lib/geom/src/mesh.rs) |
| Normalen (flach, glatt mit Schwellwinkel) und UV per Box-Projektion | [`normals.rs`](../lib/geom/src/normals.rs) |
| Grundkörper, Extrudieren, Grundriss hochziehen, Sweep, Drehkörper | [`build.rs`](../lib/geom/src/build.rs) |
| Profile: Fläche, Aufräumen, Zerlegen in Dreiecke, Rechteck, Kreis, Bogen | [`poly.rs`](../lib/geom/src/poly.rs) |
| Boolesche Operationen auf konvexen Zellen (`Solid`), Abschrägen | [`csg.rs`](../lib/geom/src/csg.rs) |
| exakte Arithmetik: Erweiterungen nach Shewchuk, `orient2d`, Ebenen und ihre Schnittpunkte | [`exact.rs`](../lib/geom/src/exact.rs) |
| die Prüfungen und ihr Bericht | [`check.rs`](../lib/geom/src/check.rs) |
| glTF schreiben | [`lib/gltf/write.rs`](../lib/gltf/write.rs) |
| die Skript-Funktionen, Materialien, `--api` | [`apps/modeltool/src/host.rs`](../apps/modeltool/src/host.rs) |
| Skript laufen lassen, Fehler, Messungen, Bericht | [`run.rs`](../apps/modeltool/src/run.rs) |
| Bilder über render3d (Kameras, Gitter und Licht teilt der Beobachtungsmodus) | [`render.rs`](../apps/modeltool/src/render.rs) |
| der Beobachtungsmodus: der Kern ohne Fenster, das Fenster | [`watch.rs`](../apps/modeltool/src/watch.rs), [`watch_ui.rs`](../apps/modeltool/src/watch_ui.rs) |
| Tests, Referenzen, Prüfsumme | [`selftest.rs`](../apps/modeltool/src/selftest.rs), [`selftest_csg.rs`](../apps/modeltool/src/selftest_csg.rs), [`refs.rs`](../apps/modeltool/src/refs.rs), [`checksum.rs`](../apps/modeltool/src/checksum.rs), im Browser [`tests/web.mjs`](../apps/modeltool/tests/web.mjs) |
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

`modeltool --api` druckt dieselbe Liste mit Typen (aus `host::api()`, siehe [Die Skriptseite](#die-skriptseite)).
Längen in Metern, Winkel in Grad, Y oben, vorn ist +Z. **Jeder Grundkörper steht auf y = 0, mittig über dem
Ursprung** (das ist die Regel, auf die sich ein Skript beim Stapeln verlässt). Ein Mesh ändert sich nie: Jede
Operation gibt ein neues. Punkte und Vektoren sind `vec`s oder Listen `[x, y, z]` (jede Funktion nimmt beides),
Profile Listen von `[x, y]`.

| Funktion | Was sie tut |
| --- | --- |
| `geom.box(w, h, d)` | Quader w × h × d (x, y, z) |
| `geom.cylinder(r, h, segments = 32)` | Zylinder um die Y-Achse; glatte Seite, flache Deckel |
| `geom.cone(r, h, segments = 32, r_top = 0)` | Kegel, mit `r_top` > 0 ein Kegelstumpf |
| `geom.sphere(r, segments = 32, rings = segments / 2)` | Kugel, Mitte bei y = r |
| `geom.torus(R, r, segments = 32, sides = 16)` | Torus, flach auf y = 0 |
| `geom.stairs(w, h, d, steps)` | Treppe als Block, erste Stufe vorn (z = d/2), steigt nach hinten |
| `geom.arch(w, h, d, thickness, segments = 16)` | Torbogen: zwei Pfeiler, Öffnung mit Halbkreis, darüber die Stärke `thickness` |
| `geom.extrude(profile, depth)` | Profil `[[x, y], ...]` (Ansicht von vorn) entlang Z, mittig um z = 0 |
| `geom.prism(footprint, h)` | Grundriss `[[x, z], ...]` (Ansicht von oben) von y = 0 nach oben |
| `geom.sweep(profile, path, closed = false, smooth = 30)` | Profil entlang eines Pfads `[[x, y, z], ...]`, Ecken auf Gehrung; Profil-x rechts des Pfads, Profil-y oben (bei senkrechtem Pfad: x ist X, y ist Z) |
| `geom.lathe(profile, segments = 32, smooth = 30)` | Drehkörper: Profil `[[radius, y], ...]` um die Y-Achse |
| `geom.merge(m1, m2, ...)` oder `geom.merge([m1, ...])` | zu einem Mesh (die Teile bleiben getrennt, auch wo sie sich durchdringen) |
| `geom.union(m1, m2, ...)` oder `geom.union([m1, ...])` | Vereinigung zu einem geschlossenen Körper: überlappende Ecken, Podest und Rampen, Turm und Kranz |
| `geom.difference(m, cut1, cut2, ...)` oder `geom.difference([m, ...])` | `m` ohne die Abzüge: ein Fenster aus einer Wand, eine Vertiefung, Zinnen |
| `geom.intersection(m1, m2, ...)` | was in allen liegt |
| `m.move(x, y, z)` oder `m.move(v)`, `m.place(x, y, z)`, `m.center()` | verschieben; die Mitte der Unterseite der Box nach (x, y, z); die Mitte der Box in den Ursprung |
| `m.rotate_x(deg)`, `rotate_y`, `rotate_z`, `m.rotate(deg, axis)` | drehen um eine Achse durch den Ursprung (rechtshändig; Vielfache von 90° exakt) |
| `m.scale(s)`, `m.scale(sx, sy, sz)`, `m.scale(v)` | skalieren um den Ursprung |
| `m.mirror("x")`, `"y"`, `"z"` | spiegeln an der Ebene durch den Ursprung; die Flächen bleiben außen |
| `m.flip()` | innen nach außen |
| `m.weld(tolerance = 0)` | Ecken an derselben Stelle zu einem Punkt verbinden (eine Hälfte und ihr Spiegelbild zu einer Fläche) |
| `m.array(count, dx, dy, dz)`, `m.radial(count, step = 360 / count)` | Kopien in einer Reihe; um die Y-Achse |
| `m.flat()`, `m.smooth(deg = 30)` | flache Normalen; glatte über Kanten flacher als `deg` Grad |
| `m.bevel(width, segments = 1, axis = nil)` | Kanten und Ecken abgeschrägt (`segments` 1) oder gerundet (so viele Flächen je Viertelkreis), `width` ist der Radius; `axis` `"x"`, `"y"`, `"z"`: nur die Kanten entlang dieser Achse; nur für konvexe Körper |
| `m.bounds()`, `m.area()`, `m.volume()`, `m.check()`, `m.tris` | `{min, max, size, center}` (je ein `vec`); m²; m³; `{ok, closed, outward, ...}`; die Zahl der Dreiecke (eine Eigenschaft, nur lesbar) |
| `profile.rect(w, h)`, `circle(r, n = 32)`, `rounded(w, h, r, n = 4)`, `arc(r, start, end, n = 16)` | Profile um den Ursprung (`arc`: ein offener Linienzug von `start` bis `end` Grad, beide Enden dabei) |
| `profile.move(p, dx, dy)`, `scale(p, sx, sy)`, `rotate(p, deg)`, `area(p)` | Profile ändern und messen |
| `vec(x, y, z)`, `vec([x, y, z])`; `v.x`, `v.y`, `v.z` | ein Vektor oder Punkt: unveränderlich, `+` und `-` zwischen Vektoren, `*` und `/` mit einer Zahl (auch `2 * v`), `-v`, `==` vergleicht die Zahlen genau |
| `v.length()`, `dot(w)`, `cross(w)`, `normalized()`, `distance(w)`, `lerp(w, t)` | Länge, Skalar- und Kreuzprodukt, Einheitsvektor, Abstand, `v + (w − v) · t` |
| `vec.add`, `sub`, `mul`, `dot`, `cross`, `len`, `dist`, `norm`, `lerp` | dasselbe als Funktionen, auf `vec`s und Listen aus 3 Zahlen (das Ergebnis ein `vec`) oder 2 Zahlen (eine Liste) |
| `model.add(m, material = "grau", name)` | ein Teil des Modells; das Material als Name oder `{color, roughness, metallic}` |
| `model.material(name, {color: "#rrggbb" oder [r, g, b], roughness = 0.8, metallic = 0})` | ein Material (Farben in sRGB, 0 bis 1) |
| `model.name(text)`, `model.budget(triangles)`, `model.expect_size(x, y, z, tolerance = 0.001)` | Name; Budget; erwartete Maße (`nil` für eine Achse, die nicht zählt); ein Befund und der Bericht nennen die Zeile der Regel |
| `model.uv_scale(meters = 1)` | Meter je UV-Einheit der Box-Projektion |
| `params` | die Parameter von `--param NAME=WERT` (Zahlen, `true`/`false`, sonst Text): `params.get("b", 0.8)` |

Eingebaute Materialien: grau, weiss, schwarz, holz, dunkelholz, stein, beton, ziegel, dach, metall, rot, gruen, blau,
gelb, glas (deckend). `import("datei.jfs")` lädt ein Modul relativ zum Skript ([`haus_teile.jfs`](../apps/modeltool/examples/haus_teile.jfs)),
nur aus dem Ordner des Skripts und darunter.

**Boolesche Operationen** nehmen konvexe geschlossene Körper (Quader, Zylinder, Kegel, Kugeln, Prismen und Extrusionen
konvexer Profile, auch gedreht und verschoben) oder Ergebnisse früherer Boolescher Operationen: ein solches Ergebnis
behält seine konvexen Zellen, auch nach `move`, `place`, `center`, `rotate*`, `scale` und `mirror` (`smooth`, `flat`,
`weld` ändern die Form nicht und behalten sie auch). `merge`, `array`, `radial` und `flip` geben ein Mesh ohne Zellen:
Es geht nur als konvexer Körper in eine weitere Operation (ein zusammengefügtes ist es nicht, die Meldung schlägt
`geom.union` vor). Das Ergebnis hat glatte Normalen bis 30 Grad (`.flat()` oder `.smooth(deg)` danach ändern das).
[`turm.jfs`](../apps/modeltool/examples/turm.jfs) zeigt alle drei Bausteine:

```js
let ring = geom.difference(geom.cylinder(r, z, 32), geom.cylinder(r - wand, z + 0.2, 32).move(0, -0.1, 0))
let scharte = geom.box(0.55, 0.45, 2 * r + 1).move(0, z - 0.45, 0)
let kranz = geom.difference(ring, scharte, scharte.rotate_y(45), scharte.rotate_y(90), scharte.rotate_y(135))
let schaft = geom.union(geom.cylinder(r, H, 32).move(0, s, 0), kranz.move(0, s + H, 0))
let sockel = geom.box(4, s, 4).bevel(0.12, 3)
```

## Die Ausgabe

`modeltool DATEI.jfs [--out ORDNER] [--param NAME=WERT ...] [--size PX] [--no-grid] [--no-images] [--force]` schreibt
in den Ordner (ohne `--out`: der Name des Skripts mit `.out`):

| Datei | Inhalt |
| --- | --- |
| `report.json` | `ok`, `errors` (Datei, Zeile, Spalte, Text, Aufrufe), `warnings` (Datei, Zeile, Spalte, Text; dazu `warnings_dropped`, wenn mehr als 1000 nicht aufgehoben wurden), `findings` (was nicht passt, deutsch, ein Satz je Fund; ein Fund über eine Regel des Skripts mit `datei:zeile: ` davor), `params`, `model` (Teile, Materialien, Ecken, Dreiecke, Budget, Box, Maße, Mitte, Fläche, Volumen), `checks` (der Bericht von `lib/geom` über das ganze Modell), `parts` (je Teil Material, Prüfung, Dreiecke, Box, Fläche, Volumen), `expect` (die Regeln des Skripts mit Datei und Zeile: `size` mit den erwarteten Maßen und den gemessenen, `budget` mit dem Budget und den Dreiecken), `materials`, `fingerprint` (FNV-1a über die Bits aller Teile), `output` (was das Skript gedruckt hat), `imports` (Module mit SHA-256), `files`, `views` (Größe, Gitterabstand, Kameras, GPU), `timing` (Skript, Geometrie, Prüfungen, Zeichnen, Export in ms; der Brennstoff des Skripts) |
| `front.png`, `side.png`, `top.png` | von vorn (+Z), von rechts (+X), von oben (vorn unten im Bild); alle drei im selben Maßstab, mit einem Gitter dahinter |
| `iso.png` | von vorn rechts oben, mit einem Gitter am Boden |
| `sheet.png` | die vier in einem Bild: vorn und rechts oben, von oben und schräg unten |
| `model.glb` | je Teil ein Mesh und ein Knoten mit seinem Material; die Szene heißt wie das Modell |

Exit-Code 0: gebaut und alle Prüfungen bestanden; 1: Fehler im Skript; 3: gebaut, aber eine Prüfung fehlgeschlagen; 2:
falscher Aufruf. `--report DATEI.jfs` gibt nur den Bericht auf stdout, `--api` die Funktionen. Ein Befund über die
Oberfläche nennt den ersten Ort und das Teil: `1 Kanten mit mehr als zwei Dreiecken, zuerst bei (0, 4.278199262, 0) in
Teil "dach"` (`Report::problem_at`: die Mitte der ersten offenen, nicht mannigfaltigen oder gedrehten Kante, sonst ein
eingeklemmter Punkt, die Mitte des ersten entarteten Dreiecks oder eine Ecke mit falscher Normale). Ein Befund über eine
Regel des Skripts nennt die Zeile, in der sie steht: `haus.jfs:71: Maße 9 x beliebig x 7 m erwartet (Toleranz 0.001),
sind …` (in einem Modul dessen Pfad, so wie Fehler ihn nennen).

### Warnungen

Was der Host annimmt, was aber sehr wahrscheinlich ein Versehen ist, meldet er als Warnung an der Stelle des Aufrufs
(JFScripts `cx.warn`, seit S1); das Skript läuft weiter, der Exit-Code bleibt. Davor stehen seit S2 die Warnungen der
Sprachdienste über das Skript und jedes Modul, das es lädt (`ls::check` mit der Beschreibung `host::api()`, ohne es
laufen zu lassen, also auch für Zeilen, die nie laufen). Die Konsole zeigt sie vor dem Ergebnis wie Fehler, mit der
Zeile der Quelle (bei `--report` auf stderr), die ersten zehn ganz, dann wie viele noch folgen; `report.json` hat alle
unter `warnings` (Datei, Zeile, Spalte, Text; zusammen höchstens 1000), der Beobachtungsmodus zeigt sie in einem
eigenen Balken. Die Sprachdienste warnen bei:

| Warnung | Beispiel |
| --- | --- |
| eine Eigenschaft oder Methode, die ein Mesh oder ein `vec` nicht hat, mit dem nächsten Namen | `mesh hat keine Methode mov – meinst du move?`, `vec hat keine Eigenschaft q` |
| eine Methode ohne Klammern gelesen oder eine Eigenschaft gerufen, eine nur lesbare gesetzt | `tris ist eine Eigenschaft von mesh, keine Methode – ohne Klammern: .tris`, `mesh.tris ist nur lesbar` |
| ein Name in `geom`, `profile`, `vec` oder `model`, den es nicht gibt | `Unbekannter Name vec.ad – meinst du vec.add?` |
| eine Division zweier Ganzzahlen | `Ganzzahlige Division: 7 / 2 schneidet ab (7 / 2 ist 3) – …` |

Der Typ eines Ausdrucks ist ihnen nur bekannt, wo die Beschreibung ihn sagt (`geom.box(..)` ist ein `mesh`, eine
Variable, der nur Meshes zugewiesen werden, auch); bei Parametern einer Funktion schweigen sie. Der Host warnt bei
vier Dingen:

| Warnung | Warum |
| --- | --- |
| mehr Argumente, als die Funktion liest (`geom.cylinder(0.3, 1.2, 32, 0.1)`) | die übrigen werden nicht angesehen; oft ist eine andere Funktion gemeint (`geom.cone`) oder ein Argument verrutscht |
| `closed` von `geom.sweep` ist weder `true` noch `false` noch `nil` | jeder andere Wert gilt als wahr, auch `0`; eine Zahl dort ist meist der Glättungswinkel eine Stelle zu früh |
| ein zweites `model.name`, `model.budget` oder `model.uv_scale` mit einem anderen Wert | der erste gilt nicht mehr (bei `uv_scale` bekommen alle Teile den letzten Wert); die Meldung nennt die Zeile des ersten |
| `model.material` legt einen Namen anders fest, den das Skript schon festgelegt hat oder den Teile schon benutzen | Teile davor behalten das alte Material, das Modell hat dann zwei Materialien eines Namens |

```text
tisch.jfs:12:7: Warnung: model.name: "Tisch 2" ersetzt den Namen "Tisch" aus tisch.jfs:2
  12 | model.name("Tisch 2")
     |       ^
```

Ein eingebautes Material neu festzulegen, das noch kein Teil benutzt, ist gewollt und keine Warnung; ebenso ein zweiter
Aufruf mit demselben Wert. `compile_with` meldet die Warnungen der Sprachdienste nicht, darum ruft `modeltool`
`ls::check` nach dem Übersetzen selbst: 0,1 bis 0,5 ms je Beispiel (das Haus mit seinem Modul am meisten), die
Beschreibung zu bauen rund 0,05 ms je Lauf.

### Der Beobachtungsmodus

`modeltool --watch DATEI.jfs [--param NAME=WERT ...]` öffnet ein Fenster mit dem Modell in einer `gpu_view` und baut es
neu, sobald das Skript oder eines seiner Module gespeichert wird (alle 300 ms wird verglichen, was in den Dateien
steht, nicht die Zeitstempel: Editoren ersetzen Dateien oder lassen den Stempel). Kameras, Gitter und Licht sind die
der Bilder ([`render.rs`](../apps/modeltool/src/render.rs) teilt sie): die Knöpfe „Vorn", „Rechts", „Oben" (ohne
Perspektive, mit Umrissen, an die Fenstergröße angepasst), „Schräg" und „Frei": Ziehen mit der linken Taste dreht um
die Mitte des Modells, das Rad bringt die Kamera näher oder weiter (beides wechselt in die freie Ansicht, ausgehend
von der gewählten). „Gitter" schaltet das Gitter, „Neu bauen" oder F5 baut von Hand, Strg+Q beendet. Ein Fehler im
Skript steht über der Ansicht (Ort und Meldung wie in der Konsole, die Zeile der Quelle, die Aufrufe); zu sehen
bleibt das Modell des letzten Baus, der lief. Ein fehlgeschlagener Befund (Maße, Budget, offen) steht als Warnung
dort, die [Warnungen](#warnungen) des Hosts in einem eigenen Balken darunter, jede mit Ort und Zeile. Die
Statusleiste: Name und Zustand, Teile, Dreiecke, Maße, die Ansicht, die Nummer des Baus, seine Dauer.
Der Kern ([`watch.rs`](../apps/modeltool/src/watch.rs)) hat kein Fenster und wird mit echten Dateien geprüft
(Gruppe `watch`), das Fenster headless mit [`tests/watch.uis`](../apps/modeltool/tests/watch.uis) und
[`tests/watch_fehler.uis`](../apps/modeltool/tests/watch_fehler.uis). Im Browser gibt es ihn nicht (keine Dateien).
modeltool startet dafür außerhalb des Browsers über `ui::main!` (ui und der Stil winui sind Abhängigkeiten).

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
  open_edges, nonmanifold_edges, flipped_edges, nonmanifold_points, inverted_components, degenerate, bad_normals,
  problem_at }` mit `closed()`, `outward()`, `ok()`, `size()`, `json()`; `DEGENERATE_RATIO`, `NORMAL_TOLERANCE`, `fnum`.
- **Boolesche Operationen** (`csg.rs`): `Solid::from_convex(&Mesh)`, `union`, `difference`, `intersection`,
  `to_mesh(glatt)`, `transformed(&Affine)`, `bevel(breite, segmente, achse) -> (Solid, Mesh)`, `cell_count`,
  `plane_count`, `cells`, `bounds`, `is_empty`; `Plane { n, w }`; auf Meshes `geom::union(&[&Mesh])`,
  `difference(&Mesh, &[&Mesh])`, `intersection(&[&Mesh])`, `bevel(&Mesh, breite, segmente, Option<Axis>)`;
  `WELD_RATIO` (10⁻⁹), `MAX_CELLS` (20 000).
- **Exakt** (`geom::exact`): `Exp` (Erweiterungen: `add`, `sub`, `scale`, `mul`, `compress`, `sign`, `estimate`),
  `orient2d`, `Meet` (wo drei Ebenen sich schneiden: `w_sign`, `dot_sign`, `point`), `div_round`.
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

### Boolesche Operationen: Brushes, exakt entschieden

Wie Brushes in Karten-Editoren: Ein `Solid` ist eine Menge konvexer Zellen, die sich nicht überlappen; jede Zelle ist
der Schnitt von Halbräumen, deren Ebenen sie mit den anderen Zellen teilt (eine Tabelle von Ebenen, die Zellen
verweisen darauf, von vorn oder von hinten). Vereinigung, Differenz und Schnitt zerlegen Zellen an Ebenen und geben
wieder eine solche Menge; darum lassen sich Ergebnisse weiter verknüpfen (eine Wand, aus der nacheinander drei
Fenster und eine Tür fallen), und darum ist die Eingabe konvex: Ein konvexer Körper ist eine Zelle.

- **Aus dem Mesh**: je Dreieck die Ebene durch eine seiner Ecken (eine Fläche quer zu einer Achse hat ihre Koordinate
  dann exakt), Dreiecke, deren Ebenen innerhalb `WELD_RATIO` (10⁻⁹ der Ausdehnung) gleich sind, sind eine Fläche
  (die Ebene des größten). Geprüft wird: geschlossen, ein Teil, Volumen positiv, keine Ecke mehr als die Toleranz
  vor einer Fläche (sonst „nicht konvex: die Ecke (…) liegt … m vor der Fläche mit der Normale (…)").
- **Ebenen verschweißen**: Kommen zwei Körper zusammen, wird jede Ebene des zweiten durch eine des ersten ersetzt,
  die innerhalb der Toleranz gleich ist (oder gleich und umgekehrt). So liegen bündige Flächen (ein Fenster bündig
  mit der Wand, Quader, die sich berühren, auch gedreht) wirklich in einer Ebene, und die exakten Entscheidungen
  behandeln sie als eine.
- **Exakt**: Ein Punkt ist, wo drei Ebenen sich schneiden; er wird während der Operation nie gerundet. Auf welcher
  Seite einer vierten Ebene er liegt, ist das Vorzeichen einer 4 × 4-Determinante der Ebenen-Koeffizienten (genauer:
  der Kofaktoren der drei Ebenen mal der vierten, mal dem Vorzeichen der Determinante der drei Normalen). Erst in f64
  mit einer Fehlerschranke aus den Permanenten (4 · 10⁻¹⁵ der Summe der Beträge, das Doppelte dessen, was die
  Rechnung braucht), nur nahe 0 (und genau bei 0) exakt mit Erweiterungen nach Shewchuk
  ([`exact.rs`](../lib/geom/src/exact.rs): Summen und Produkte von f64 ohne Rundungsfehler, nur `+ − ×`, also auf
  jedem Ziel dieselben Antworten). Damit stimmen Zerlegen einer Zelle, Schneiden einer Fläche und das Finden von
  Punkten auf Kanten immer überein, auch für Punkte genau auf einer Ebene (der übliche Fall, nicht die Ausnahme).
- **Eine Zelle zerlegen**: Die Seiten aller Ecken zur Ebene; liegen alle auf einer Seite, bleibt die Zelle ganz.
  Sonst wird jede Fläche geschnitten (die Kanten einer Fläche sind die Ebenen ihrer Nachbarn, ein Schnitt setzt die
  neue Ebene als Kante ein) und die neue Fläche ist ein großes Vieleck auf der Ebene, geschnitten mit den Ebenen der
  Flächen, die sie berühren. Ein Polytop aus Ebenen ist ein Würfel weit draußen, nacheinander an jeder Ebene zerlegt.
  Differenz: die Teile außerhalb jeder Ebene des Abzugs, nacheinander; Vereinigung: der erste Körper und die Teile
  des zweiten außerhalb des ersten; Schnitt: jede Zelle des einen an allen Ebenen jeder Zelle des anderen.
- **Die Oberfläche**: die Flächen aller Zellen je Ebene, von vorn und von hinten; wo sich zwei Zellen berühren,
  liegen ihre Flächen auf derselben Ebene in entgegengesetzter Richtung, und was übrig bleibt, wenn man die einen von
  den anderen abzieht (Abzug konvexer Vielecke in der Ebene), ist außen. Punkte, die genau auf einer Kante eines
  Nachbarn liegen (exakt geprüft), kommen in diese Kante, so teilt jede Kante genau zwei Dreiecke.
- **Gerundet wird einmal, am Ende**: jede Koordinate auf das f64, das dem exakten Quotienten am nächsten liegt (bei
  Gleichstand gerade), über den Rest `n − q·d` als Erweiterung. Gleiche Punkte geben gleiche Bits, wie auch immer ihre
  Ebenen gewählt wurden; die Topologie ist die der gleichen Bits (wie `weld(0)`).
- **Zerlegen in Dreiecke ohne Koordinaten**: Jede Fläche ist exakt konvex, ihre Ecken knicken echt, eingefügte Punkte
  liegen auf ihren Kanten. Drei Einträge liegen also genau dann auf einer Linie, wenn sie eine Kante der Fläche teilen;
  die Ohren werden nur danach gewählt. Der erste Entwurf zerlegte die gerundeten Punkte mit exaktem `orient2d`: Punkte,
  die exakt auf einer Kante liegen, stehen nach dem Runden um 10⁻¹⁷ daneben, und es entstanden Splitter mit einer
  Höhe von 10⁻¹⁷ (der Test fand sie als entartete Dreiecke).
- **Was unter der Toleranz liegt, fällt zusammen**: Ebenen in f64 treffen sich nicht genau in einem Punkt, wo mehr als
  drei Flächen zusammenkommen sollten (die Spitze eines Kegels, die Ringe einer Kugel, eine verschweißte Ebene an
  einer Ecke mit vier Flächen). Das exakte Ergebnis hat dort Kanten von 10⁻¹⁶; Punkte näher als die Toleranz werden
  ein Punkt (Dreiecke mit einem Punkt doppelt fallen weg, zwei auf denselben Punkten gegeneinander auch), und ein
  Splitter, dessen Spitze nur um Rundung neben einer langen Kante liegt, wird durch Umklappen dieser Kante entfernt
  (die Fläche bleibt, wo sie war). Ebenen mit exakten Koeffizienten durch drei Punkte hätten das vermieden, aber
  Determinanten vom Grad 9 gekostet, und das Verschweißen bündiger Flächen braucht die Toleranz ohnehin.
- **Grenzen**: Zwei Körper, die sich nur an einer Kante berühren, geben eine Kante mit vier Flächen; das ist richtig
  und wird gemeldet (im Bewertungslauf genau so passiert: die zwei Hälften eines Dachs). Allgemeine Netze
  (nicht konvex, ohne Zellen) gehen nicht hinein; `MAX_CELLS` 20 000 Zellen. Die Ergebnisse sind nicht minimal: Eine
  ebene Fläche aus mehreren Zellen bleibt in Stücken, mit mehr Dreiecken als nötig.

### Abschrägen

`m.bevel(width, segments, axis)` (Rust: `geom::bevel`, `Solid::bevel`) auf einem konvexen Körper fügt Ebenen hinzu:
längs jeder Kante `segments` Ebenen durch Punkte eines Kreisbogens mit dem Radius `width`, dessen Achse um den Radius
in beiden Flächen liegt (eine Kante, die um φ knickt, bekommt `round(segments · φ / 90°)` davon, mindestens eine:
die Zahl gilt je Viertelkreis, die Seiten eines 32-eckigen Zylinders werden nicht zerstückelt), an jeder Ecke von drei
Flächen die Dreiecke eines Kugelflickens mit demselben Radius (bei drei gleichen Kanten das Raster normalize(i n₁ + j n₂
+ (k − i − j) n₃), sonst Reihen von der Fläche gegenüber der Kante mit den wenigsten Segmenten). Der abgeschrägte
Quader ist die Minkowski-Summe des um den Radius geschrumpften Quaders und des Polytops dieser Richtungen; daraus ist
die Referenz der Tests gerechnet. `segments` 1 ist eine Fase (eine Fläche je Kante, ein Dreieck je Ecke); flache
Normalen. Ab 2 sind die Rundungen glatt: Eine Ecke einer Rundung bekommt die Normale der ursprünglichen Fläche, auf der
sie liegt, wo der Bogen in sie übergeht (dort ist die Fläche tangential: keine Kante, und die große Fläche bleibt
gleichmäßig hell), sonst den nach Winkeln gewichteten Mittelwert der Rundungsflächen um sie. `axis`: nur Kanten
entlang dieser Achse (auf ein Grad), die Ecken bleiben (eine Tischplatte mit runden senkrechten Kanten). Zu breit (die
nach innen versetzten Flächen schließen kein Volumen mehr ein) ist ein Fehler. Ecken mit mehr als drei Flächen (die
Spitze eines Kegels) bekommen keinen Flicken; nicht konvexe Körper gehen nicht (abschrägen vor der Booleschen
Operation: ein abgeschrägter Körper ist konvex).

### Importe nur aus dem Ordner des Skripts

`import()` liest nur Dateien im Ordner des Skripts und darunter: JFScript normalisiert den Pfad (relativ zum Skript,
„..", „." aufgelöst), [`run.rs`](../apps/modeltool/src/run.rs) prüft ihn, bevor ein Lader ihn sieht (`import_allowed`:
der Pfad beginnt mit dem Ordner des Skripts und einem „/", kein „.." bleibt; ein Skript im Arbeitsordner darf keinen
absoluten Pfad und nichts über „.." öffnen; Groß- und Kleinschreibung zählt, `C:/m/` ist nicht `c:/m/`), und der Lader
für Dateien folgt keiner Verknüpfung auf dem Weg vom Ordner zur Datei (unter Windows auch keiner Junction), damit
eine Verknüpfung im Ordner nicht hinausführt. Die Meldung: `Modul ../geheim.jfs: liegt außerhalb des Ordners des
Skripts (apps/x); import liest nur Dateien in diesem Ordner und darunter`. Andere Wege zu Dateien hat ein Skript
nicht (JFScripts Bibliothek liest keine, `params` sind Text).

### Der Bewertungslauf

Fünf Bezugsobjekte aus kurzen Beschreibungen, die vor dem Modellieren geschrieben wurden (eine Kiste mit Griffmulden,
eine Treppe mit Geländer, eine Tür mit Kassetten, ein Haus aus Wandmodulen in einer eigenen Datei, eine kleine Arena
mit Türmen), gebaut vom Agenten in JFScript, mit `sheet.png` und `report.json` als einziger Sicht. Die Beschreibungen,
jede Runde mit Skript, Bild und Bericht und die Notizen liegen außerhalb des Repositorys in
`Documents\JFLib-wip-archiv\g7g2-eval` (README.txt dort ist die Übersicht).

| Objekt | Runden | Was geschah |
| --- | ---: | --- |
| Kiste | 1 | Bretter einer Seite als `geom.union`, die Griffmulde ein Abzug mit gerundetem Schneider, Leisten mit `bevel` |
| Treppe | 2 | v1: „Maße 1.2 x beliebig x 2.24 m erwartet, sind … x 2.354 m": der Handlauf ragte vor (der Bericht fand es, nicht das Bild) |
| Tür | 2 | v0: `?:` gibt es nicht (Meldung mit Pfeil); v1: acht Kassetten als Abzüge mit gefasten Schneidern |
| Haus aus Modulen | 3 | v1: eine Kerbe am First; v2: das Dach nicht geschlossen (die Hälften berührten sich nur am First, ein echter Fehler im Modell); v3 richtig |
| Arena | 2 | v1: die Türme ragten aus den 30 m (`expect_size`); v2 bündig in den Ecken |

Was geholfen hat: die Booleschen Operationen (Öffnungen und Vertiefungen als ein Abzug, das Haus-Modul halb so lang
wie [`haus_teile.jfs`](../apps/modeltool/examples/haus_teile.jfs) aus Teil 1), `geom.union` über überlappende Teile,
`model.expect_size` (dreimal der erste Hinweis auf einen Fehler), `bevel` als ein Aufruf, die Meldungen mit Ort.
Was schwer war: Lage-Rechnungen von Hand (die Dachplatte: drehen, dann verschieben, mit sin und cos im Bogenmaß; die
Linie der Stufenkanten); ein Befund ohne Ort (seitdem nennen die Befunde Ort und Teil); Richtungen in den Bildern
(vorn liegt in der Seitenansicht links, keine Achsen im Bild); kein Bedingungsausdruck, Listen nur mit `push`; kleine
Teile sind bei 384 px kaum zu sehen (`--size 1024` hilft). Die Reihenfolge der Wünsche an JFScript steht unter
[Was JFScript braucht](#was-jfscript-braucht).

### Die Skriptseite

- **Ein Mesh ist ein Objekt des Hosts** (seit S2; bis dahin die Map `{mesh: id, tris: n}`, weil JFScript keine
  Host-Objekte hatte, und ihre Methoden kamen als `map.move` an). Das Skript kann es nicht öffnen und nicht ändern,
  `type(m)` ist `"mesh"`, `print` zeigt `mesh(12 Dreiecke)`, `==` ist dasselbe Mesh; `m.tris` liest der Host
  (`Host::get`), die Methoden gehen an `Host::method`. Ein Name, den ein Mesh nicht hat, wirft eine Meldung mit allen,
  die es hat. Meshes ändern sich nie. Gibt der Sammler eines frei, das das Skript nicht mehr hält, erfährt es der Host
  (`Host::drop`) und gibt Ecken und konvexe Zellen frei; die Id nimmt das nächste neue Mesh. Ein Teil des Modells hat
  seine eigene Kopie (`model.add`), ihm schadet das nie. Die Grenze von 4 Mio. Dreiecken zählt weiter jedes Mesh des
  Laufs, auch freigegebene Zwischenstände: Sie begrenzt die Arbeit eines Laufs, und sie hängt nicht davon ab, wann der
  Sammler läuft (seine Schwellen sind Sache von JFScript). Im Bericht steht in der letzten Zeile, wie viele Meshes ein
  Lauf gemacht hat und wie viele es höchstens zugleich waren (`meshes`, `meshes_peak`).
- **Ein Vektor ist ein Werttyp des Hosts** `vec` (seit S2): drei Zahlen, unveränderlich, `==` vergleicht die Zahlen
  genau (ohne Toleranz). Die Operatoren `+ −` zwischen Vektoren, `× ÷` mit einer Zahl und das Minus rechnet der Host
  mit den Grundrechenarten von `f64`, wie `vec.*`; sie geben dieselben Bits wie JFScripts Rechnung in Software. Jede
  Funktion, die einen Punkt oder Vektor nimmt, nimmt einen `vec` wie eine Liste `[x, y, z]` (oder drei Zahlen, wo
  sie das schon tat); Funktionen, die einen Punkt in 3D geben (`m.bounds()`, `vec.*`), geben einen `vec`, damit die
  Operatoren gehen (`v[0]` heißt jetzt `v.x`). `vec.*` mit Listen aus zwei Zahlen gibt weiter eine Liste: Profile
  bleiben Listen von `[x, y]`. Der Konstruktor heißt wie der Typ, `vec(x, y, z)`, neben dem Namensraum `vec.*`: Für
  die VM sind `vec` und `vec.add` zwei Namen des Hosts, für die Sprachdienste ist `vec` eine Funktion und zugleich
  der Namensraum der Namen mit Punkt.
- **Eine Beschreibung** (seit S2): `host::api()` ist ein `script::Api` mit allen Funktionen, dem Wert `params`, den
  Typen `mesh` und `vec` mit Eigenschaften, Methoden und Operatoren, die Signaturen in der Schreibweise von
  TypeScript (`geom.cone(r: float, h: float, segments: int = 32, r_top: float = 0): mesh`), die Texte deutsch.
  Daraus kommen `--api`, die Namen für `compile_with`, die Warnungen der Sprachdienste, die höchste Zahl der Argumente
  je Funktion (für die Warnung über weitere) und die Meldungen über fehlende Eigenschaften und Methoden. Eine Signatur
  nennt die längste Form (`m.move(x, y, z)`), die kürzeren (`m.move(v)`) stehen in ihrem Text: Eine Signatur hat
  keine Überladungen und `Api::validate` keine Vereinigungstypen.
- **Namen englisch, Texte deutsch** (Entscheidung G9.3 in JFScripts `docs/RICHTUNG.md`): Funktionen und ihre
  Parameter heißen englisch wie die Sprache und ihre Bibliothek (`m.bevel(width, segments, axis)`, `list.range(start,
  end, step)`); mit benannten Argumenten werden auch die Namen der Parameter Teil der Schnittstelle. Bis S1 hießen sie
  deutsch (`breite`, `segmente`, `achse`); Skripte ändert das nicht, sie übergeben Argumente nach ihrer Stelle.
  Beschreibungen (`--api`) und Meldungen bleiben deutsch, Schlüsselwörter werden nie übersetzt. Die Namen eines Skripts
  (Variablen, `--param`) wählt sein Autor.
- **Fehler auf Deutsch**, mit dem Namen der Funktion und des Parameters (`geom.box: die Höhe muss eine Zahl größer 0
  sein, nicht -2`, `geom.box: w muss eine Zahl sein, nicht str "x"`), wie JFScripts eigene. JFScript setzt Datei,
  Zeile und Spalte des Aufrufs dazu; `modeltool` zeigt die Zeile mit einem Pfeil und die Aufrufe. `compile_with` mit
  den Namen des Hosts macht einen Tippfehler (`boks`) schon vor dem Lauf zum Fehler, mit Vorschlag.
- **Grenzen**: 64 MiB Heap, 200 Mio. Schritte (eine Endlosschleife endet nach rund 2 s), Brennstoff der
  Host-Funktionen nach der Arbeit (je 64 Dreiecke einer).
- **Cache-Schlüssel**: SHA-256 (die ersten 128 Bit) über die Version des Werkzeugs und von JFLib, das Skript, die
  sortierten Parameter und jedes importierte Modul mit Pfad und Inhalt, Zeilenenden als LF. Ein Lauf mit gleichem
  Schlüssel, gleichen Bildoptionen und vorhandenen Dateien baut nicht neu. Seit S1 beginnt der Text mit
  `modeltool-key-2` (der Bericht hat Warnungen und die Zeilen der Regeln), seit S2 mit `modeltool-key-3` (Meshes und
  Vektoren sind Objekte, der Bericht hat die Warnungen der Sprachdienste): ein Bericht von vorher wird neu gebaut.

### Die Bilder

- **Kameras**: die drei Ansichten sind seit G7f Teil 2 ohne Perspektive (`render3d::Camera::ortho`) und mit Umrissen
  aus dem Tiefenpuffer, so dass Kanten gleich gefärbter Flächen (Stufen, Türrahmen) zu sehen sind; vorher kamen sie
  aus großer Entfernung mit 4° Sichtfeld. Alle drei haben **denselben Maßstab** (die größte Ausdehnung des Modells),
  wie die Ansichten einer Zeichnung: Verhältnisse zwischen den Bildern stimmen. Die Schrägansicht hat 20° von vorn rechts oben (35° seitlich, 28° hoch).
- **Gitter**: eine Platte hinter dem Modell (vorn: hinter seiner Rückseite, rechts: links von ihm, oben: unter ihm und
  unter y = 0) mit Linien im Abstand 1, 2 oder 5 mal einer Zehnerpotenz (höchstens 12 über das Modell), die Linien durch
  den Ursprung dunkelblau. Der Abstand steht im Bericht.
- **Kantenglättung**: render3d hat keine; jedes Bild wird doppelt so groß gezeichnet und 2 × 2 gemittelt.
- **Licht**: eine Sonne von rechts oben vorn, Umgebungslicht 0,32; Farben des Skripts in sRGB, für render3d und glTF
  linear umgerechnet (`pow` aus `rt`). Die Bilder sind nicht Teil der Prüfsumme (GPUs unterscheiden sich im letzten Bit).

## Was JFScript braucht

Was in Teil 1 fehlte und wie `modeltool` sich behilft. Gebaut wird es im Repository von JFScript, nach einem Ja des
Nutzers (E7); JFScript ist in Teil 2 unverändert. Die Richtung und die Stufen stehen seit 2026-11-23 in JFScripts
`docs/RICHTUNG.md`; Stufe S1 (JFScript c970852, 2026-11-24) brachte 3, 5 und 7 und für 6 die Warnung der
Sprachdienste, `modeltool` nutzt davon die Stelle des Aufrufs und die Warnungen. Stufe S2 (JFScript a21a0c3,
2026-11-25) brachte 2 und 9 und damit 1 als Werttyp des Hosts; `modeltool` zeigt seitdem auch die Warnungen der
Sprachdienste (6). Offen sind 4, 8 und 10 (S3). Der
[Bewertungslauf](#der-bewertungslauf) ordnet die Wünsche so: **1. Vektoren mit Operatoren** (die Lage-Rechnungen waren der größte Aufwand: die Dachplatte, die Linie der
Stufenkanten), **5. mehr Mathematik in Grad** (tan für das Dach), **4. benannte Argumente** (`bevel(width, segments,
axis)`), neu **10. ein Bedingungsausdruck** (`a ? b : c` fehlt; die Meldung hilft, kostet aber eine Runde), dann
**2./3. Host-Objekte und die Stelle des Aufrufs** (Warnungen mit Zeile, etwa „dieser Abzug trifft nichts").

1. **Vektoren mit Operatoren.** Heute: `let b2 = vec.add(b, vec.mul(richtung, 0.1))`. Gewünscht: `let b2 = b +
   richtung * 0.1` mit `b.x`, `b.y`, `b.z`. Ein Typ `vec` (2 und 3 Komponenten, `f64`, unveränderlich) mit `+ − ×
   ÷` (Skalar), `==`, Feldern und Methoden (`len`, `norm`, `dot`, `cross`); Listen von Zahlen als Punkte müssten
   weiter gehen. Behelf: Listen und `vec.*`. **Seit S2 da**, als Werttyp des Hosts statt als Typ der Sprache:
   `vec(x, y, z)` mit `v.x`, `+ −` zwischen Vektoren, `× ÷` mit einer Zahl, `-v`, `==` nach den Zahlen,
   `length`, `dot`, `cross`, `normalized`, `distance`, `lerp`; Listen gehen weiter als Punkte, `vec.*` auch. Nur in
   3D (Profile bleiben Listen von `[x, y]`). `treppe.jfs` rechnet so: `let b2 = b + richtung * 0.1`.
2. **Host-Objekte.** Heute ist ein Mesh die Map `{mesh: 3, tris: 12}`: Das Skript kann sie ändern (`m.mesh = 7`),
   `type(m)` sagt `map`, `m.len()` ist 2, Methodennamen der Map-Bibliothek (`merge`, `get`, `has`, `keys`, `values`,
   `len`, `delete`, `entries`) sind für Meshes verloren, und der Host kann nichts freigeben, weil er nicht erfährt, wann
   ein Mesh unerreichbar wird. Gewünscht: ein Wert `Host(typ, id)`, den das Skript nicht öffnen kann; `obj.m(..)` geht
   an `Host::call("mesh.m", ..)`; `type()` und `print` fragen den Host; der Sammler meldet freigegebene Ids
   (`Host::drop`). **Seit S2 da** (`Host::get`, `set`, `method`, `op`, `to_str`, `drop`): ein Mesh ist ein `mesh`,
   `m.tris` eine Eigenschaft, freigegebene Meshes gibt der Host während des Laufs frei ([Die Skriptseite](#die-skriptseite)).
3. **Die Stelle des Aufrufs für den Host.** `Cx` sagt nicht, in welcher Zeile die Host-Funktion gerufen wurde. So kann
   `model.expect_size` im Bericht nicht sagen, wo die Regel steht, und der Host kann keine Warnung mit Zeile geben
   („das Teil schwebt 2 cm über dem Boden"). Gewünscht: `cx.position() -> (datei, zeile, spalte)`, dazu Warnungen
   (`cx.warn(text)`), die nicht abbrechen. **Seit S1 da**: `modeltool` gibt [Warnungen](#warnungen) aus, und
   `model.expect_size` und `model.budget` merken sich ihre Zeile.
4. **Benannte Argumente.** `geom.cylinder(0.3, 1.2, 32)` ist für eine KI fehleranfällig (was ist r, was h?).
   Gewünscht: `geom.cylinder(r: 0.3, h: 1.2)` als letzte Map übergeben, oder Standardwerte in der Signatur. Behelf:
   Positionsargumente mit Namen in den Fehlermeldungen und in `--api` (seit S1 englisch, so wie sie als benannte
   Argumente heißen würden).
5. **Mehr Mathematik.** Es fehlen `tan`, `asin`, `acos`, `atan`, `atan2`, `hypot` und Winkel in Grad, die bei 90° exakt
   sind (`math.sin_deg`). `haus_teile.jfs` rechnet `tan` als `math.sin(a) / math.cos(a)` mit `a = neigung * math.pi / 180`.
   **Seit S1 da**: `math.tan`, `asin`, `acos`, `atan`, `atan2`, `hypot`, `sin_deg`, `cos_deg`, `tan_deg`. Die Beispiele
   rechnen weiter wie vorher, damit ihre Eingaben und die [Prüfsumme](#prüfsummen) gleich bleiben.
6. **Ganzzahl-Division.** `1 / 3` ist 0 und `(stufen + 2) / 3` schneidet ab: In Geometrie-Code ein leiser Fehler
   (die Säule braucht `i / 12.0`). Vorschlag: eine Warnung der Sprachdienste, wo zwei Ganzzahlen geteilt werden, oder
   `//` für die ganzzahlige Division. **Seit S1**: `check` warnt bei `int / int`; seit S2 zeigt `modeltool` die
   Warnungen der Sprachdienste ([Warnungen](#warnungen)).
7. **Bereiche mit Schritt und Zerlegen.** `for i in 0..n` nur ganzzahlig mit Schritt 1; eine Kurve abtasten heißt
   `t = i / 12.0` von Hand. Gewünscht: `range(a, b, schritt)`, `linspace(a, b, n)`. **Seit S1 da**:
   `list.range(start, end, step)`, `list.linspace(start, end, n)`.
8. **Zerlegen beim Zuweisen.** `let {rahmen, glas, bank} = teile.fenster(b, h)` und `let [x, y, z] = p` statt
   `f.rahmen` und `p[0]`.
9. **Beschreibungen der Host-Funktionen für die Sprachdienste.** `script::ls` kennt `HostItem`; `modeltool` hat die
   Liste in `host::API`. Für den Beobachtungsmodus und die IDE (G7o) gehören beide zusammen. **Seit S2 da**:
   `script::Api` mit Typen und Signaturen; `host::api()` ersetzt `host::API` und dient `--api`, `compile_with` und
   `ls::check`. Offen bleibt: `compile_with` gibt die Warnungen der Sprachdienste nicht mit zurück; `modeltool` ruft
   dafür `ls::check`, das das Skript ein zweites Mal liest.
10. **Ein Bedingungsausdruck** (neu aus dem Bewertungslauf). `"drücker ${seite > 0 ? "vorn" : "hinten"}"` geht nicht;
    Behelf: eine Variable und `if` davor. Ebenso fehlt ein Ausdruck für eine Liste aus einer Schleife (`[for i in
    0..n: ...]`); Behelf: `push` in der Schleife, was bei `geom.union(teile)` und `geom.difference(liste)` oft vorkommt.

## Tests

`python build_rs.py test modeltool` führt `tests/geom.uis` (`--test prims,ops,normals,checks,poly`), `tests/csg.uis`
(`--test csg,bevel`), `tests/examples.uis` (`--test gltf,script,watch,examples`) und die beiden Skripte des
Beobachtungsmodus (`tests/watch.uis`, `tests/watch_fehler.uis`, headless) aus; `modeltool --check` alle elf Gruppen
(`--quick` ein Zehntel).
Die Referenzen in `refs.rs` teilen keinen Code mit `lib/geom`: Volumen und Flächen aus geschlossenen Formeln, die aus der
Geometrie hergeleitet sind. Ein Drehkörper mit n Segmenten ist ein Polyeder aus ebenen Trapezen; der Körper zwischen
zwei Meridianebenen ist ein lineares Bild des Prismas über dem Profil mit der Jacobi-Determinante ρ sin Δ, also ist sein
Volumen die diskrete Pappus-Regel V = n sin(2π/n) · ∫∫ρ dA, die Fläche je Profilkante n gleichschenklige Trapeze. Ein
Sweep mit Gehrung um den Schwerpunkt des Profils hat das Volumen Fläche × Länge der Mittellinie.

Für die Booleschen Operationen: Die Überlappung zweier achsparalleler Quader ist ein Produkt von Intervall-Längen; für
alle anderen Körper prüfen Punkte, die um beide Eingaben gestreut werden, ob sie auf der richtigen Seite liegen: in
den Eingaben über die Ebenen ihrer Dreiecke, im Ergebnis über die Parität der Schnitte eines Strahls (beides in
einfachem f64, Punkte nahe einer Fläche bleiben weg), dazu die Gleichungen V(A ∪ B) + V(A ∩ B) = V(A) + V(B) und
V(A − B) + V(A ∩ B) = V(A). Für das Abschrägen: Der abgeschrägte Quader ist die Minkowski-Summe des geschrumpften
Quaders und des Polytops P der Richtungen; sein Volumen ist der geschrumpfte Quader, seine Flächen mal dem Radius,
seine Kanten mal der Fläche des Umrisses von P quer zu ihnen und das Volumen von P (8 Oktanten, je die Kegel über
ihren Rasterdreiecken).

| Gruppe | Prüft | Ergebnis |
| --- | --- | --- |
| `prims` | je 150 bis 300 Quader, Zylinder, Kegel und Stümpfe, Kugeln, Tori, Treppen, Bögen mit zufälligen Maßen und Segmenten: geschlossen, außen, nichts entartet, Normalen gut, ein Teil, Euler 2 (Torus 0), Volumen und Fläche gegen die Referenz auf 10⁻¹¹, Boxen exakt (bei Vielfachen von 4 Segmenten); was verweigert wird | 12 609 Prüfungen, 0 falsch |
| `ops` | Drehungen um Vielfache von 90° exakt (Boxen Bit für Bit), 200 Drehungen um beliebige Achsen, Skalieren (auch gespiegelt), Spiegeln an allen Achsen, Umdrehen, Felder, `radial`: Volumen, Fläche, Teile; sich berührende Quader bleiben zwei geschlossene Teile; sechs einzelne Quadratflächen werden erst durch `weld` ein Würfel; 300 Sterne extrudiert und hochgezogen (auch im Uhrzeigersinn); Sweeps entlang eines L und eines geschlossenen Quadrats gegen Fläche × Länge, um eine nicht ebene geschlossene Schleife und eine Schraubenlinie; 300 Drehkörper zufälliger Profile gegen Pappus | 9 920 Prüfungen, 0 falsch |
| `normals` | flach: jede Ecke mit der Normale ihres Dreiecks; ein Quader bleibt bei 30° flach; Kugelnormalen zeigen von der Mitte weg; Zylinder: Deckel genau ±Y, Seiten waagrecht und radial auf 10⁻¹²; ein Achteck bei 10° flach; UV einer verschobenen Box je Fläche gleich den Koordinaten, in drei Maßstäben | 706 Prüfungen, 0 falsch |
| `checks` | absichtlich kaputte Netze: ein Loch (3 offene Kanten), ein gedrehtes Dreieck (3 gegenläufige Kanten), innen nach außen, ein Splitter und ein Dreieck mit doppeltem Punkt, eine Kante mit vier Dreiecken, zwei Würfel an einer Ecke, Nullnormalen, Normalen nach innen; Euler eines Torus und zweier Kugeln; das JSON; der Ort des ersten Problems | 15 Prüfungen, 0 falsch |
| `poly` | 2000 zufällige Sternvielecke mit 3 bis 80 Punkten zerlegt: n − 2 Dreiecke, alle mit Fläche, die Summe gleich der Fläche, jede Randkante einmal vorwärts, jede innere Kante einmal in jede Richtung; Punkte auf geraden Strecken; was verweigert wird; Rechteck, Kreis, abgerundete Rechtecke | 8 013 Prüfungen, 0 falsch |
| `csg` | 300 Paare achsparalleler Quader (ein Drittel auf einem Raster von 1/64 m bündig, berührend, eins im anderen): Vereinigung, Differenz, Schnitt geschlossen, außen, nichts entartet, ein oder zwei Teile, Euler, Volumen gegen die Überlappung auf 10⁻¹¹, Grenzen exakt; 120 Paare gedrehter Quader, Zylinder, Kegel und Stümpfe, Kugeln und Prismen: die Volumen-Gleichungen auf 10⁻⁹, je 40 gestreute Punkte auf der richtigen Seite; 80 Prismen mit 3 bis 48 Ecken durch Platten (Euler 0, Volumen); 30 Wände mit bis zu vier Öffnungen nacheinander (Euler 2 − 2k); 20 Körper gedreht und weiter geschnitten gegen dasselbe aus gedrehten Eingaben; A ∪ A, A − A, A ∩ A, zwei Hälften, eine Reihe gedrehter Kisten; was verweigert wird (nicht konvex, offen, zwei Teile, leer) | 6 090 Prüfungen, 0 falsch |
| `bevel` | 120 Quader mit 1 bis 6 Segmenten: geschlossen, Euler 2, Volumen gegen die Minkowski-Summe auf 10⁻¹¹, Grenzen exakt, die flache Fläche flach und so groß wie erwartet, keine Naht am Übergang; 40 nur entlang Y (Querschnitt: Rechteck mit runden Ecken); 60 Zylinder, Stümpfe, gedrehte Quader; ein abgeschrägter Block mit Loch; was verweigert wird | 1 287 Prüfungen, 0 falsch |
| `gltf` | die sechs Beispiele geschrieben und gelesen: Meshes, Materialien, Knoten, Szenen gleich, keine Notizen; ein Knoten mit Matrix und Kind, ein Material mit allen Faktoren, MASK und doppelseitig; was verweigert wird (Textur, Index außerhalb, NaN) | 42 Prüfungen, 0 falsch |
| `script` | ein gutes Skript; zwölf kaputte: falsche Werte, falsche Typen, unbekannte Namen, Methoden, Materialien, sich schneidende Profile, nichts hinzugefügt, fehlende und kaputte Module, Endlosschleife, zu viele Dreiecke, falsche Achse: jede Meldung mit Datei, Zeile und Text; Pfeil unter der Spalte; Parameter, Module, Funden statt Fehlern, Abfragen; der Cache-Schlüssel (CRLF = LF); Boolesche Operationen und Abschrägen aus dem Skript (eine Wand mit Fenster: Euler 0, gedreht und weiter geschnitten) und ihre Meldungen; ein Befund mit Ort und Teil; Importe: zwölf Pfade für `import_allowed`, drei Skripte, die hinaus wollen, ein Skript im Unterordner; seit S1: Befunde und Regeln mit ihrer Zeile (auch aus einem Modul), sieben Warnungen mit Datei, Zeile, Spalte und Text, in der Konsole und im Bericht, eine Warnung vor einem Fehler, mehr als 1000 Warnungen; die Namen von `--api` englisch, `max_args` für jede Funktion; seit S2: die Beschreibung ganz (`Api::validate`), ein Mesh als Objekt (Typ, Text, `tris`, Gleichheit, fünf falsche Zugriffe mit ihren Meldungen und den Warnungen der Sprachdienste), ein `vec` (Konstruktor, Komponenten, Operatoren, `==`, Methoden, `vec.*` auf `vec`s und Listen, Punkte als `vec` in sieben Funktionen, elf Fehler mit ihren Meldungen), 40 401 Meshes mit höchstens 8191 zugleich (die Teile und ein gehaltenes Mesh bleiben heil), sechs Warnungen aus Sprachdiensten, Modul und Host in ihrer Reihenfolge | 217 Prüfungen, 0 falsch |
| `watch` | der Kern des Beobachtungsmodus mit echten Dateien in einem temporären Ordner: der erste Bau, unverändert, gespeichert, ein Fehler (das alte Modell bleibt), ein Modul geändert und gelöscht, ein Import aus dem Ordner darüber, ein Skript, das fehlt (im Browser: keine Dateien, keine Prüfung); eine Warnung mit Ort und Zeile | 18 Prüfungen, 0 falsch |
| `examples` | die sechs Beispiele: laufen, bestehen alle Prüfungen, ihr Bericht gleich dem in `examples/out` (ohne Dateien, Ansichten, Zeiten) | 24 Prüfungen, 0 falsch |

Zusammen 38 941 Prüfungen in rund 9 s (Entwicklungs-Build, Ryzen 5 3600; `bevel` und `csg` sind die längsten
Gruppen). Dazu headless: `watch.uis` (die Kiste im Fenster, die vier Ansichten, ziehen und Rad in der
freien Ansicht, Gitter aus, Neu bauen und F5) und `watch_fehler.uis` (ein Skript mit Fehler: der Balken, Ort und
Meldung, die Zeile; kein Modell; die Warnung davor in ihrem Balken).

## Prüfsummen

`modeltool --checksum` baut jedes Beispiel aus seinem Skript und faltet die Bits jedes Teils (Positionen, Normalen,
UV, Punkte, Indizes über `Mesh::fingerprint`), die Zahlen seiner Prüfung, den Brennstoff des Skripts und den Bericht des
ganzen Modells in FNV-1a; dazu 29 feste Operationen aller Arten (jeder Grundkörper mit ungeraden Segmenten, ein
Sternprofil extrudiert, ein abgerundeter Grundriss, ein Sweep um eine Schraubenlinie und eine nicht ebene geschlossene
Schleife, eine Vase, eine beliebige Drehung, ungleiches und gespiegeltes Skalieren, Felder, `radial` mit 7, `smooth`,
`flat` mit UV, `weld` mit Toleranz; seit Teil 2: eine Platte ohne ein gedrehtes Prisma und einen bündigen Quader,
vier gedrehte Quader vereint, eine Kugel geschnitten mit einem Quader, ein Körper gedreht und weiter geschnitten,
Abschrägen eines Quaders, eines Zylinders, eines Stumpfs und nur entlang Y).

**Die Zahl hat sich in Teil 2 geändert**, weil sie jetzt mehr faltet: das sechste Beispiel (`turm.jfs`) und die neun
neuen Operationen. Der alte Teil ist unverändert: `8d93441afd1b79d7` mit 183 303 Werten bis zum Stand a7b4f56b, also
nach allen Änderungen an `lib/geom` (Booleans, Abschrägen, `problem_at`) und vor dem neuen Beispiel; die alten
Beispiele geben denselben Bericht wie in `examples/out`.

**Die Zahl hat sich mit S2 geändert**, nur durch den Brennstoff: `treppe.jfs` rechnet den Handlauf mit Operatoren
statt mit `vec.*` (`b + richtung * 0.1`) und braucht 265 Schritte statt 275; die anderen Beispiele brauchen gleich
viele, auch mit Meshes als Objekten. Die Bits aller Meshes sind gleich: Ohne den Brennstoff gefaltet ergibt sich vor
und nach S2 `d9c23e8f809f620b` mit 109 751 Werten, und die Berichte in `examples/out` unterscheiden sich nur im
Schlüssel. Bis S2 war die Zahl `0fae175a31a0f15e` (S1 und `modeltool` vor S2 auch gegen JFScript a21a0c3).

| Ziel | Build | Prüfsumme | `--check` |
| --- | --- | --- | --- |
| Windows x86-64 (Ryzen 5 3600), Rust 1.100 | Entwicklung | `ff9e060c8233c600` | 38 941 bestanden (S2) |
| Linux x86-64 gegen glibc, in WSL Ubuntu 24.04 | Entwicklung | `ff9e060c8233c600` | 38 941 bestanden (S2) |
| macOS ARM64 (Mac mini M2, macOS 27.2), Rust 1.98.1 | Entwicklung | `0fae175a31a0f15e` | 38 906 bestanden (S1; S2 noch nicht geprüft; die fünf Skripte mit Metal: Stand Teil 2) |
| WebAssembly in Node 22 (V8): einfach, SIMD, Threads | Entwicklung | `ff9e060c8233c600` | 38 923 bestanden (S2; alle drei, ohne die 18 der Gruppe `watch`) |

Zusammen 234 941 Werte. Mit JFScript S1 (c970852) und den Warnungen war die Zahl auf Windows, Linux und in Node
dieselbe geblieben, auch die Zahl der Werte: Die Warnungen kosten keinen Brennstoff, und die Beispiele waren
unverändert. Auf x86-64 und ARM64 rechnet die Wurzel der Prozessor, im Browser `sqrt_portable`; JFScript
rechnet überall in Software, die exakte Arithmetik der Booleschen Operationen nur mit `+ − ×`; die Operatoren eines
`vec` rechnet der Host in `f64` mit denselben Bits. `--check` dauert im Entwicklungs-Build rund 11 s unter Windows
(unter Last bis 20 s), 11 s in WSL, 6 s auf dem M2 und 19 bis 25 s in Node (S1); mit S2 9 s unter Windows, 10 s in
WSL und 12 bis 15 s in Node. Gezeichnet haben die RTX 2070 SUPER (Vulkan) und der M2 (Metal); die Release-Builds
sind in Teil 2 nicht verglichen.

## Messwerte

Die Beispiele auf dem Entwicklungsrechner (Ryzen 5 3600, RTX 2070 SUPER, Entwicklungs-Build), Millisekunden:

| Beispiel | Teile | Dreiecke | Skript | Geometrie | Prüfungen | Bilder (4 × 1024², gemittelt auf 512²) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Kiste | 28 | 444 | 1,1 | 0,5 | 0,6 | 1122 (das erste: GPU und Pipelines) |
| Treppe | 6 | 964 | 0,5 | 0,7 | 0,9 | 290 |
| Tür | 7 | 688 | 0,5 | 0,6 | 0,7 | 281 |
| Haus aus Modulen | 29 | 952 | 1,2 | 1,4 | 1,0 | 282 |
| Säule | 3 | 3304 | 1,2 | 3,4 | 2,9 | 332 |
| Turm (Teil 2) | 2 | 3680 | 1,6 | 311 | 3,4 | 257 |

Skript und Geometrie sind gegen die Bilder vernachlässigbar, außer bei Booleschen Operationen mit runden Körpern: Der
Turm (ein 32-eckiger Schaft, hohl, mit Kranz, Tür und drei Fenstern als Abzüge) braucht 311 ms Geometrie, die Arena
des Bewertungslaufs 60 bis 160 ms, das Haus aus Modulen 18 ms, die Tür 60 ms. Teuer ist, dass jeder Abzug mit einem
32-Eck die Zelle in rund 34 Zellen zerlegt und jede Operation die Polytope aller Zellen neu baut (O(Ebenen²) je
Zelle); ein Vorfilter über Boxen und behaltene Polytope wären der nächste Schritt. Die Rückmeldeschleife kostet sonst
rund 0,3 s je Lauf, davon das meiste in render3d (Gerät, Szenen, vier Bilder mit Warten) und im PNG-Kodierer; im
Beobachtungsmodus zeichnet nur die eine Ansicht.

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
- Der Beobachtungsmodus mit echtem Fenster: eine kurze Probe unter Windows (das Fenster ohne Aktivierung und ohne
  gespeicherten Fensterzustand, die Datei geändert, während es offen war: Bau 2 mit den neuen Maßen, mit WM_CLOSE
  beendet); Linux, macOS und das Ziehen mit der Maus nur headless. Verknüpfungen auf dem Weg eines Imports sind nicht
  automatisch geprüft (Verknüpfungen anlegen braucht unter Windows Rechte).
- Boolesche Operationen mit sehr vielen Zellen (über einige hundert), sehr feinen Kugeln (über 1000 Ebenen) und
  Merkmalen nahe der Toleranz (10⁻⁹ der Ausdehnung): Sie fallen zusammen oder bleiben, geprüft ist es nicht.

## Was nicht gebaut ist

- Allgemeine Netz-Booleans (nicht konvexe Eingaben ohne Zellen), Abschrägen nicht konvexer Körper und von Ecken mit
  mehr als drei Flächen, das Zusammenfassen ebener Stücke einer Fläche (weniger Dreiecke), Vereinfachen für
  Detailstufen (G7j), Löcher in Profilen (ein Fenster entsteht jetzt als Abzug), Teil-Drehkörper (unter 360°),
  Texturen, Farben je Ecke, Instanzen im glTF (jedes Teil ist ein eigenes Mesh).
- Beschriftungen in den Bildern und im Beobachtungsmodus (Maße, Achsen, Himmelsrichtungen).

## Danach

- **Spiel 1, Teil 2** (die Inhalte von `apps/game1` sind nicht geändert): `werft.jfs` baut die Deckungsmauern, Kisten
  und Fässer als getrennte Teile, die sich durchdringen dürfen; mit `geom.union` würden Plattform und Rampen ein
  Körper (eine Kollisionsform ohne Fuge), mit `geom.difference` die Mauern mit Durchgängen und Toren aus einem Stück,
  mit `bevel` Kisten und Container mit Fasen (sichtbare Kanten in der Toon-Grafik). Die Arena des Bewertungslaufs
  zeigt das für eine ganze Karte. `figur.jfs` profitiert wenig (Teile, die sich bewegen, bleiben Teile).
- **Schneller**: Vorfilter über Boxen beim Zerlegen und behaltene Polytope zwischen Operationen (siehe
  [Messwerte](#messwerte)); ebene Stücke einer Fläche zusammenfassen.
- **Bausteine für die Lage-Rechnungen**, die im Bewertungslauf am meisten kosteten: Dächer, Rampen, „an zwei Punkten
  ausrichten", eine Vertiefung in einer Fläche; dann Beschriftungen in den Bildern (Text in render3d oder danach).
- **JFScript**: die Liste in [Was JFScript braucht](#was-jfscript-braucht) in der Reihenfolge des Bewertungslaufs,
  nach dem Ja des Nutzers.
- **Geteilter Code mit `lib/physics`**: `math::sqrt`/`sqrt_portable` stehen in beiden Crates gleich; ein gemeinsamer
  Ort (etwa `rt`) wäre eine eigene Entscheidung. Ein Mesh von `lib/geom` geht über `positions_f32` und die Indizes in ein
  `physics::TriMesh`.
