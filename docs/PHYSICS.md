# PHYSICS -- Kollision für Spiele

`lib/physics` (`jflib-physics`, in der Fassade `jflib::physics`) ist die Physik der Spiel-Engine ([GAME.md](GAME.md), Stufe G7c,
Entscheidung E5: eine eigene, keine Portierung). Formen (Kugel, Kapsel, Quader, konvexe Hülle, statisches Dreiecksnetz), eine
Breitphase (ein fester BVH je Netz, dynamische AABB-Bäume für die Körper) und die Abfragen, aus denen Treffer und der
Character-Controller entstehen: Strahlen (erster Treffer und alle), bewegte Formen mit Zeitpunkt, Normale und Kontaktpunkt,
Überlappung, Kontakte mit Eindringtiefe, der nächste Punkt. Ohne GPU und ohne Betriebssystem (`#![no_std]`,
`#![forbid(unsafe_code)]`, keine fremden Crates, abhängig nur von `rt`): Der Server von Spiel 1 rechnet ohne Fenster, unter
Linux statisch ohne libc. Und vor allem **bitgleich auf allen Zielen**, damit die Vorhersage eines Clients genau das rechnet,
was der Server rechnet (G7e).

Stand: G7c Teil 1 (dieser Lauf): Formen, Breitphase, Abfragen, Prüfsummen. Teil 2, der kinematische Character-Controller,
fehlt noch ([Für Teil 2](#für-teil-2-den-character-controller)). Starrkörper kommen mit G7i.

| Teil | Wo |
| --- | --- |
| Vektoren, Drehungen, Lagen, Boxen; Wurzel, sin, cos, atan2 | [`math.rs`](../lib/physics/src/math.rs) |
| Formen | [`shape.rs`](../lib/physics/src/shape.rs), Hülle: [`hull.rs`](../lib/physics/src/hull.rs), Netz: [`mesh.rs`](../lib/physics/src/mesh.rs) |
| Breitphase | BVH der Netze: [`bvh.rs`](../lib/physics/src/bvh.rs); dynamischer Baum: [`tree.rs`](../lib/physics/src/tree.rs) |
| Schmalphase | GJK, EPA, Cast: [`gjk.rs`](../lib/physics/src/gjk.rs); Stützfunktionen: [`convex.rs`](../lib/physics/src/convex.rs); Strahlen: [`ray.rs`](../lib/physics/src/ray.rs) |
| Abfragen | eine Form gegen eine: [`query.rs`](../lib/physics/src/query.rs); über alle Körper: [`world.rs`](../lib/physics/src/world.rs) |
| Tests, Prüfsummen, Messung | [`apps/physicstest`](../apps/physicstest/README.md): Referenzen [`refs.rs`](../apps/physicstest/src/refs.rs), Gruppen [`tests.rs`](../apps/physicstest/src/tests.rs), [`checksum.rs`](../apps/physicstest/src/checksum.rs), [`bench.rs`](../apps/physicstest/src/bench.rs), im Browser [`tests/web.mjs`](../apps/physicstest/tests/web.mjs) |

## Ein Beispiel

```rust
use jflib::physics::{Filter, Iso, Shape, TriMesh, Vec3, World};

// die Karte: dieselben f32-Positionen und Indizes wie ein Mesh von render3d
let karte = TriMesh::new(&positionen, &indizes).expect("gültige Indizes");
let mut welt = World::new();
welt.add(Shape::mesh(karte), Iso::IDENTITY);
let figur = welt.add(Shape::capsule(0.5, 0.3), Iso::at(Vec3::new(0.0, 1.0, 0.0)));

// Hitscan: der erste Körper auf 100 m, ohne die eigene Figur
let auge = Vec3::new(0.0, 1.7, 0.0);
if let Some((koerper, treffer)) = welt.ray_cast(auge, Vec3::new(0.0, 0.0, -1.0), 100.0, &Filter::ALL.excluding(figur)) {
    // treffer.t (Meter bei einer Richtung der Länge 1), treffer.point, treffer.normal, treffer.triangle
}

// die Kapsel um die Bewegung dieses Takts verschieben: wo sie zuerst etwas berührt
let schritt = Vec3::new(0.1, -0.05, 0.0);
if let Some((_, k)) = welt.cast(&Shape::capsule(0.5, 0.3), &Iso::at(Vec3::new(0.0, 1.0, 0.0)), schritt, &Filter::ALL.excluding(figur)) {
    // bis k.fraction der Bewegung gehen, an k.normal entlanggleiten; k.depth > 0: steckte schon drin
}
welt.set_iso(figur, Iso::at(Vec3::new(0.1, 0.95, 0.0)));
```

## API-Überblick

- **Mathematik** (`physics::math`, die Typen auch an der Wurzel): `Vec3` (f64; `dot`, `cross`, `length`, `try_normalize`,
  `normalize_or_zero`, `min`, `max`, `abs`, `lerp`, `any_perpendicular`, Operatoren; `From<[f64; 3]>`, `From<[f32; 3]>`),
  `Quat` (x, y, z, w wie render3d; `from_axis_angle`, `from_yaw_pitch`, `from_rotation_arc`, `mul`, `rotate`,
  `inverse_rotate`, `normalize`, `conjugate`, `to_mat3`, `from_f32`, `to_f32`), `Mat3`, `Iso { pos, rot }` (erst drehen, dann
  verschieben; `transform_point`, `inverse_transform_point`, `mul`, `inv_mul`), `Aabb`. Funktionen: `sqrt` (korrekt gerundet),
  `sqrt_portable`, `sin`, `cos`, `sincos` (Sinus zuerst), `atan2`, `min`, `max`, `clamp`.
- **Formen**: `Shape::{Sphere { radius }, Capsule { half_height, radius }, Box { half }, Hull(Arc<ConvexHull>),
  Mesh(Arc<TriMesh>)}` mit `sphere`, `capsule`, `cuboid`, `hull`, `hull_from_points`, `mesh`, `is_convex`, `margin`,
  `aabb(&Iso)`. Die Kapsel steht auf der Y-Achse. `ConvexHull::new(&[Vec3]) -> Result<_, HullError::{Count, NotFinite,
  Flat}>` (4 bis `MAX_HULL_POINTS` = 65 536 Punkte), `points`, `faces`, `planes`, `support`, `volume`, `tolerance`, `aabb`.
  `TriMesh::new(&[[f32; 3]], &[u32])`, `with_options(.., &MeshOptions { active_edge_cos, max_leaf })`, `MeshError::{IndexCount,
  IndexRange, NotFinite}`, `triangle_count`, `stored_triangle_count`, `vertices`, `aabb`, `nodes` (`BvhNode`), `depth`,
  `triangles()`, `edge_flags(id)`, `memory_bytes`; `ACTIVE_EDGE_COS` (cos 5°), `MAX_TRIANGLES` (2^28).
- **Welt**: `World::new()` (Rand 0,05 m um bewegte Körper), `with_margin`, `add(shape, iso) -> BodyId` (Ebene 1),
  `add_with_layers`, `remove`, `body(id) -> Option<&Body { shape, iso, layers, user, is_moving() }>`, `set_iso`, `set_layers`,
  `set_user`, `len`, `bodies()`, `trees()`. Abfragen: `ray_cast(origin, dir, max_t, &Filter) -> Option<(BodyId, RayHit)>`,
  `ray_cast_all(.., &mut Vec<(BodyId, RayHit)>)`, `cast(&shape, &iso, motion, &Filter) -> Option<(BodyId, CastHit)>`,
  `overlap(&shape, &iso, &Filter, &mut Vec<BodyId>)`, `contacts(&shape, &iso, margin, &Filter, &mut Vec<(BodyId, Contact)>)`,
  `closest_point(point, max_dist, &Filter) -> Option<(BodyId, PointHit)>`. `Filter { mask, exclude }` (`Filter::ALL`,
  `Filter::mask(m)`, `.excluding(id)`), `BodyId` (Index und Generation, geordnet).
- **Eine Form gegen eine** (`physics::query`): `ray`, `ray_all`, `cast`, `separation` (Abstand oder Eindringtiefe, beide
  Punkte, Normale), `intersects`, `contacts`, `closest_point`; `query::brute::*` dasselbe ohne den Baum eines Netzes.
  Ergebnisse: `RayHit { t, point, normal, triangle }`, `CastHit { fraction, point, normal, triangle, depth }`, `Contact {
  point, normal, depth, triangle }`, `Separation { distance, point_a, point_b, normal, triangle }`, `PointHit { point,
  distance, inside, triangle }`; `NO_TRIANGLE` bei allem außer Netzen. `CAST_SKIN` (1 µm), `cast_fallbacks()` (Statistik).
- **Bäume**: `DynamicTree` (`insert`, `remove`, `update`, `query`, `ray`, `height`, `validate`) und `BvhNode` sind öffentlich,
  damit Tests und spätere Stufen sie sehen; die Welt verwaltet sie selbst.

Konventionen: Meter, rechtshändig, Y oben. Normalen sind Einheitsvektoren und zeigen von dem getroffenen Körper zur
Abfrageform (bei Strahlen gegen den Strahl). Konvexe Formen sind massiv: Ein Strahl, der innen beginnt, trifft bei t = 0 mit
der Normale gegen den Strahl, ein Punkt innen hat den Abstand 0. Dreiecke haben zwei Seiten. Die Abfrageform eines Casts, einer
Überlappung, eines Kontakts muss konvex sein (ein Netz als Abfrageform gibt nichts). Der erste Treffer ist das kleinste
(t, Körper, Dreieck), Listen sind nach Körper und Dreieck sortiert (Strahlen zuerst nach t).

## Entscheidungen

### f64 für alles, f32 für die Ecken eines Netzes

Positionen, Drehungen und alle Rechnungen sind `f64`. Gründe: render3d hält Positionen auf der CPU schon in `f64` (mit einem
Ursprung für die GPU), spätere Stufen wollen große Welten (G7j), und die Physik soll dafür ihren Ursprung nicht verschieben
müssen; GJK und EPA brauchen Reserve in der Genauigkeit (Abstände nahe 0, Richtungen aus kleinen Differenzen). Skalar kostet
`f64` auf x86-64, ARM64 und in WebAssembly so viel wie `f32`; SIMD nutzt die Crate nicht. Die Ecken eines Netzes sind `f32` im
eigenen Rahmen des Netzes (wie die Meshes von render3d), dessen Lage `Iso` mit `f64` setzt: das spart Speicher, und die Boxen
des BVH sind damit exakt in `f32`.

Bereich und Genauigkeit:

| Größe | Wert |
| --- | --- |
| Abstand zweier `f64` bei 1 km / 100 km / 1000 km | 1,1 · 10⁻¹³ / 1,5 · 10⁻¹¹ / 1,2 · 10⁻¹⁰ m |
| Toleranz eines Casts (Lücke am Ziel) | 1 · 10⁻⁹ m; zwischen zwei Formen ohne Radius `CAST_SKIN` = 1 · 10⁻⁶ m |
| GJK hört auf, wenn ein neuer Stützpunkt den Abstand um weniger als | 10⁻¹² relativ |
| dieselbe Szene bei 10⁶ m verschoben | dieselben Casts auf 10⁻⁸ m (Test `edge`) |
| Raster der Ecken eines Netzes (f32) 1 km / 4 km / 16 km vom Ursprung des Netzes | 0,06 / 0,24 / 0,98 mm |

Abfragen rechnen im Rahmen der Zielform: Die große Zahl der Weltposition fällt einmal weg (in der relativen Lage), nicht in
jedem Stützpunkt. Bis 10⁶ m vom Weltursprung bleiben die Toleranzen im Nanometerbereich. Netze sollten nicht weiter als etwa
4 km von ihrem eigenen Ursprung reichen (eine Karte in Stücken mit eigener `Iso`), sonst werden ihre Ecken gröber als ein
Viertelmillimeter.

### Bitgleich auf allen Zielen

- **Nur `+ − × ÷` und korrekt gerundete Wurzeln** auf `f64`. Rust zieht `a * b + c` nie zu einem FMA zusammen, `mul_add`
  kommt nicht vor. Die Wurzel ist auf x86-64 und ARM64 die des Prozessors (`rt::fx::sqrtd`: `sqrtsd`, `fsqrt`), im Browser und
  im x86-64-Emulator von Android `sqrt_portable`: vier Newton-Schritte in `f64` aus einer Schätzung über die Bits, dann ein
  Test auf der Mantisse in 128-Bit-Ganzzahlen ((R − ½)² < X ≤ (R + ½)²), der den richtigen Nachbarn wählt. IEEE 754 verlangt
  die korrekt gerundete Wurzel, also ist das Ergebnis überall dasselbe; `rt`s eigene portable Wurzel rechnet Bit für Bit mit
  128-Bit-Zahlen und wäre im Browser für die Physik zu langsam. Geprüft gegen `rt::fx::sqrtd` an rund 900 000 Eingaben,
  auch an den schwersten Fällen (Doubles nächst am Quadrat eines Mittelpunkts und ihren Nachbarn).
- **sin, cos, atan2** kommen aus `rt`s portabler Mathematik (`portable/math.rs`, auf ein halbes ulp genau, ARM64.md, ARM2),
  auf x86-64 ausdrücklich nicht aus `rt::x87`, das dort die x87-Einheit ist. Die Abfragen selbst brauchen sie nicht; sie sind
  für die Drehungen eines Controllers da (`Quat::from_axis_angle`, `from_yaw_pitch`).
- **Keine ungeordneten Container, keine Threads, keine Uhr.** Sortiert wird mit eindeutigen Schlüsseln (bei Gleichstand der
  Index), jede Schleife hat eine feste Grenze, jede Wahl einen Gleichstandsbrecher.
- **Ergebnisse hängen nicht von den Bäumen ab.** Ein erster Treffer ist das kleinste (t, Körper, Dreieck), über alle Kandidaten
  in beliebiger Reihenfolge. Die Tests prüfen das Bit für Bit gegen Schleifen über alle Körper und Dreiecke; eine andere Form
  des Baums (die Aufteilung der Welt auf drei Bäume kam spät dazu) ließ die Prüfsumme unverändert. Ein Client, der nur einen
  Teil der Körper des Servers kennt, bekommt dieselben Antworten, wo nur diese Körper beteiligt sind, solange sie dieselben Ids
  haben (bei exakt gleichem t entscheidet die Id).
- **Geprüft** mit einer Prüfsumme über die Bits aller Ergebnisse fester Szenarien ([Prüfsummen](#prüfsummen)), auch zwischen
  Entwicklungs- und Release-Build (fat LTO).

### Schmalphase: GJK, EPA, Cast

- **GJK** (Gilbert, Johnson, Keerthi) für Abstände zwischen konvexen Formen, mit Ericsons Voronoi-Regionen für Strecke, Dreieck
  und Tetraeder. Jede Form ist ein Kern mit Stützfunktion (Punkt, Strecke, Quader, Hülle, Dreieck) und ein Rand (der Radius von
  Kugel und Kapsel). Der Rand bleibt außerhalb von GJK: Kugeln und Kapseln sind damit exakt rund, und ihre Kerne sind bei
  Berührung noch einen Radius auseinander, was jeden Cast gut konditioniert.
- **EPA** aus dem letzten Simplex von GJK, wenn sich die Kerne überlappen: ein Polyeder in der Minkowski-Differenz, je Schritt
  die nächste Fläche, ihr Stützpunkt, die sichtbaren Flächen weg, der Horizont neu verbunden. Eine flache Differenz (der
  Mittelpunkt einer Kugel genau in der Ebene eines Dreiecks) hat keinen Tetraeder; dann ist die Normale die der Ebene, auf der
  Seite eines Hinweises (Mitte gegen Mitte).
- **Cast** (eine Form bewegt sich gerade gegen eine andere): GJK-Strahlwurf mit konservativem Vorrücken (van den Bergen 2004,
  wie Box2Ds `b2ShapeCast`, hier in 3D): Die Stützebene der Differenz in Richtung des nächsten Punkts begrenzt den Abstand für
  jeden späteren Zeitpunkt, die Zeit rückt bis dahin vor, wo diese Grenze den Zielabstand erreicht, und nie über den ersten
  Kontakt hinaus. Ziel ist der Radius der beiden Formen, zwischen zwei Formen ohne Radius `CAST_SKIN`: Ein positiver Abstand
  hält die Richtung der nächsten Punkte definiert. Ein Vorrücken beginnt einen neuen Simplex; eine Ebene, die keinen Schritt
  mehr erlaubt (Rundung), ändert nichts, GJK rechnet bei dieser Zeit weiter. Bleibt GJK hängen oder gehen die 64 Schritte aus,
  übernimmt ein Newton-Verfahren auf dem Abstand: Er ist entlang einer geraden Bewegung eine konvexe Funktion der Zeit mit der
  Steigung n · r, also kommen Newton-Schritte von unten nie über den ersten Kontakt und konvergieren quadratisch. In allen Tests
  zusammen geschah das einmal.
- **Kontakt am Start**: Berühren oder überlappen sich die Formen schon, meldet ein Cast einen Treffer bei 0 genau dann, wenn
  die Bewegung tiefer hineinführt (n · r < 0, mit der Normale aus GJK oder EPA). Weil der Abstand konvex in der Zeit ist, wird
  er bei n · r ≥ 0 nie kleiner: Gleiten an einer Fläche und Weggehen sind frei, ohne dass die Form erst um einen Rand
  abgehoben werden muss. Wer aus einer Überlappung heraus muss, fragt `contacts`.
- **Strahlen** analytisch: Kugel in der stabilen Form aus Ray Tracing Gems (Kap. 7, ohne b² − c), Kapsel als Zylinder und zwei
  Kugeln, Quader mit Slabs, Hülle durch Schneiden mit ihren Ebenen, Dreieck wasserdicht nach Woop, Benthin und Wald (JCGT 2013):
  Die Kantenfunktionen einer gemeinsamen Kante sind in beiden Dreiecken dieselben Produkte, kein Strahl rutscht zwischen zwei
  Dreiecken durch, ein Strahl genau auf einer Kante trifft beide.
- **Nicht SAT** für die Paare: GJK deckt alle Paare einschließlich der Hüllen einheitlich ab. Trennachsen nutzt nur die Referenz
  der Tests.

### Netze, Nähte und aktive Kanten

Jedes Dreieck wird als das geschlossene Dreieck getestet, das es ist, also bleibt zwischen zwei Dreiecken mit gemeinsamer Kante
keine Lücke: Eine Form, die über die Naht fährt, trifft die Kante des einen oder des anderen. Das umgekehrte Problem sind
**Geisterkollisionen**: Gleitet eine Form über eine flache Naht und steckt dabei ein Haar in der Fläche, trifft sie die Kante
des nächsten Dreiecks mit einer Normale, die nach hinten kippt, und bleibt hängen. Dagegen hat jedes Netz **aktive Kanten**
(wie Jolts „active edges"): Eine Kante ist aktiv, wenn die Fläche dort um mehr als den Schwellwinkel (5°) nach außen knickt,
wenn sie offen ist oder mehr als zwei Dreiecke sie teilen; flache und nach innen geknickte Kanten sind inaktiv. Die Seite zählt:
Eine von vorn konkave Kante ist von hinten konvex, also trägt jedes Dreieck sechs Bits (drei Kanten, zwei Seiten). Liegt ein
Kontakt auf einer inaktiven Kante (oder auf einer Ecke, deren beide Kanten in diesem Dreieck inaktiv sind; erkannt an den
baryzentrischen Gewichten, ein Gewicht bis 10⁻⁹ zählt als 0), bekommt er die Normale des Dreiecks; ein Cast meldet ihn dann nur,
wenn die Bewegung in diese Fläche hineinführt. Kontakte behalten dabei ihren wahren Abstand (die Tiefe über der Ebene würde den
Knick einer leicht konvexen flachen Kante als Eindringen zählen). Die Nachbarn findet der Bau über die Positionen der Ecken
(gleiche Position, gleiche Ecke), nicht über die Indizes. Dreiecke ohne Fläche fallen beim Bau heraus. `MeshOptions {
active_edge_cos: 2.0 }` schaltet das ab (jede Kante aktiv); der Test zeigt so, dass es wirkt: Von 800
Gleitbewegungen einer 0,1 bis 10 µm eingesunkenen Kugel über ein flaches Gitter blieben ohne aktive Kanten 794 an einer Naht hängen, mit ihnen keine.

### Breitphase

- **BVH eines Netzes**: binned SAH mit 16 Fächern auf jeder Achse (Kosten eines Knotens 1, eines Dreiecks 1, höchstens 4 Dreiecke
  je Blatt), unterhalb der Tiefe 40 der Median auf der breitesten Achse, was die Tiefe für jede Eingabe begrenzt. Knoten zu 32
  Byte mit Boxen in `f32` (exakt, weil die Ecken `f32` sind), die Kinder eines Knotens nebeneinander, die Dreiecke in der
  Reihenfolge der Blätter. Strahl- und Box-Test der Knoten sind robust nach Ize („Robust BVH Ray Traversal", JCGT 2013): das
  Intervall eines Slabs um 2γ₃ < 10⁻¹⁵ geweitet, damit kein Strahl durch die Fläche oder Kante einer Box um ein ulp abgewiesen
  wird (sonst gingen Treffer auf Dreiecken an der Grenze eines Knotens verloren: der Test `ray` fand 113 von 3445 Strahlen, die
  an Kanten eines Gitters durchgingen, bevor das drin war).
- **Dynamische AABB-Bäume** der Welt nach dem Schema von Box2Ds `b2DynamicTree` (neu geschrieben): Blätter mit Boxen, eingefügt,
  wo die Oberfläche am wenigsten wächst, auf dem Weg nach oben durch Rotationen ausgeglichen. Ein Körper beginnt in einem Baum
  der stillen Körper, mit seiner genauen Box: Netze in einem, die übrigen Formen in einem zweiten. Beim ersten `set_iso`
  wechselt er in den Baum der bewegten Körper, mit einer um 0,05 m größeren Box, damit kleine Bewegungen sein Blatt behalten.
  Grund: In einem einzigen Baum vergrößert die Box der Karte jeden Knoten über ihr, und Strahlen, die in diesen großen Boxen
  beginnen, gehen viele Knoten ab, bevor die Karte sie kürzt: Strahlen durch Karte und 500 Körper schaffen in einem Baum
  354 000 je Sekunde, in den drei Bäumen 714 000 (Ryzen 5 3600; auf dem M2 444 000 gegen 947 000). Strahlen und
  Casts fragen die Bäume in dieser Reihenfolge; der Treffer auf der Karte kürzt die anderen.

## Tests

`python build_rs.py test physicstest` führt `apps/physicstest/tests/check.uis` aus (`--check`, Windows und Linux, 10 bis 15 s im
Entwicklungs-Build). Die Referenzen in `refs.rs` teilen keinen Code mit der Crate: nächste Punkte über Projektion und Minimum über
die Kanten, Strecken über Ericsons geklemmte Lösung, Kreuzungen über Möller-Trumbore; eine konvexe Form ist dort eine Menge von
Ecken, Flächennormalen, Kanten und Dreiecken. Der Abstand zweier solcher Formen ist ohne Schnitt das Minimum über alle Paare von
Merkmalen (exakt für getrennte konvexe Polyeder), mit Schnitt (eine Kante kreuzt ein Dreieck, eine Ecke liegt innen) die
Eindringtiefe über die Trennachsen (Flächennormalen beider, Kreuzprodukte ihrer Kanten; exakt für konvexe Polyeder). Der erste
Kontakt einer Bewegung kommt aus einer Dreiteilungssuche nach dem Minimum der konvexen Lückenfunktion und einer Halbierung davor.

| Gruppe | Prüft | Ergebnis |
| --- | --- | --- |
| `math` | `sqrt_portable` und `sqrt` gegen `rt::fx::sqrtd` Bit für Bit: Sonderwerte, 200 000 zufällige Bitmuster und Zahlen, 100 000 Doubles nächst am Quadrat eines Mittelpunkts mit je vier Nachbarn; sin/cos/atan2 auf Plausibilität; Drehungen, `Iso`, `from_rotation_arc` | 1 049 912 Prüfungen, 0 falsch |
| `hull` | 1500 Wolken aus 4 bis gut 200 Punkten (Würfel, Kugeloberfläche, Quaderflächen mit koplanaren Punkten, flach 1:1000, bei 10⁵ m, viele Doppelte): kein Punkt außerhalb, Ecken sind Eingabepunkte, geschlossen (jede Kante zweimal, gegenläufig, V − E + F = 2), Flächen nach außen, Stützpunkt gegen alle Punkte; Volumen von Quadern; was verweigert wird | 0 falsch |
| `ray` | 6000 Strahlen gegen Kugel, Kapsel, Quader, Hülle (Treffer auf der Oberfläche, nicht nach dem Eintritt der Referenz, kein Fehlen), die Kugel auch gegen die geometrische Formel (10⁻¹²); 20 000 gegen Dreiecke gegen Möller-Trumbore; Wasserdichtheit: Strahlen auf jede Ecke, Kante und Fläche eines Gitters mit wechselnden Diagonalen (senkrecht und schräg, von oben und unten; schräg nicht auf den offenen Rand) und 20 000 zufällige, dazu 20 000 aus einem geschlossenen Quader heraus, auch genau auf seine Ecken | 0 durchgerutscht |
| `distance` | 8000 Paare aus allen Formen und Dreiecken: Abstand (10⁻⁹ m), Eindringtiefe von EPA gegen die Trennachsen (10⁻⁷), die Normale trennt, Punkte auf den Oberflächen; `intersects`, `contacts`; analytisch Kugel–Kugel, Kapsel–Kapsel, Kugel–Quader; 8000 nächste Punkte | größter Fehler 2,7 · 10⁻¹⁵ m getrennt, 1,3 · 10⁻¹⁵ m überlappend |
| `cast` | 4000 Casts aller Formen gegen alle Formen und Dreiecke: am gemeldeten Zeitpunkt Lücke zwischen 0 und Ziel + 10⁻⁸ m, nie nach dem ersten Kontakt, kein verpasster Kontakt, Start-Kontakte nach der Regel; Kugel gegen Kugel gegen die exakte Zeit | 0 falsch |
| `tunnel` | 20 000 kleine Kugeln (1 bis 50 mm) mit bis zu 10 km je Schritt durch Dreiecke, 10 000 Kapseln und Quader durch eine 1 mm dünne Wand, 5000 Fälle aus 25 m bis 10 km Höhe auf die Karte | 0 durchgetunnelt (bis 9991 m je Schritt) |
| `seams` | 6000 Gleitbewegungen von Kugel, Kapsel und gedrehtem Quader über ein flaches Gitter, 1 mm darüber bis 10 µm darin; dasselbe ohne aktive Kanten als Gegenprobe; Fallen auf jede Ecke und Kante des Gitters (Normale genau nach oben); 3000 Kugeln auf Hügel fallen lassen (Kontakte in Ruhe zeigen nach oben, ≤ 10⁻⁸ m tief); 40 Läufer mit Kollidieren und Gleiten über die Karte, je bis 1500 Schritte | 0 blockiert, 0 eingesunken |
| `bvh` | der Baum einer Karte aus 3050 Dreiecken (jedes in einem Blatt, Boxen halten alles); 8000 Strahlen, 1000 `ray_all`, 1000 Casts, Kontakte, `intersects`, `separation`, nächste Punkte gegen `query::brute`, Bit für Bit | 0 anders |
| `world` | Karte und 400 Körper in zufälligen Ebenen; 60 Runden mit Verschieben, Entfernen, Hinzufügen und `validate` der Bäume; je 40 Abfragen aller Arten mit vier Filtern gegen Schleifen über alle Körper, Bit für Bit; Generationen | 0 anders |
| `fuzz` | 40 000 zufällige Paare mit Größen von 3 mm bis 300 m, ein Fünftel bei bis zu 10⁵ m: keine Panik, nichts nicht Endliches, Normalen der Länge 1, Brüche in [0, 1], `intersects` passt zum Abstand, Abstand symmetrisch | größte Asymmetrie 5 · 10⁻¹⁵ der Größe |
| `edge` | Netze, die verweigert werden, leere und flache Netze, ein Netz als Abfrageform, Casts ohne Bewegung, Berühren und Weggehen/Entlanggleiten/Hineinfahren, Strahl ohne Richtung, 3000 Casts bei 10⁶ m gegen dieselben am Ursprung, eine Bewegung von 1000 km, sortierte Kontakte an einer Ecke | 0 falsch |

## Prüfsummen

`physicstest --checksum` rechnet feste Szenarien und faltet die Bits jedes Ergebnisses (FNV-1a über `f64::to_bits`) in eine
Zahl: 20 000 Wurzeln (beide Wege), Sinus, Kosinus, atan2 und Drehungen; 20 Hüllen; der Baum und die Kantenbits einer Karte aus
7778 Dreiecken; eine Welt aus Karte und 300 Körpern über 10 Runden mit Verschieben, Entfernen und Hinzufügen, darin 3000 Strahlen,
300 `ray_cast_all`, 1500 Casts, Überlappungen, Kontakte, nächste Punkte; 3000 Paare mit Abstand und Cast; sechs Läufer mit
Kollidieren und Gleiten über 600 Schritte. Zusammen 294 963 Werte.

| Ziel | Build | Prüfsumme | `--check` |
| --- | --- | --- | --- |
| Windows x86-64 (Ryzen 5 3600), Rust 1.100 | Entwicklung und Release (fat LTO) | `0c7267a7636aaf7b` | 1 480 284 bestanden |
| Linux x86-64, statisch ohne libc, in WSL Ubuntu 24.04 | Entwicklung | `0c7267a7636aaf7b` | 1 480 284 bestanden |
| macOS ARM64 (Mac mini M2, macOS 27.2), Rust 1.98.1 | Entwicklung und Release | `0c7267a7636aaf7b` | 1 480 284 bestanden |
| macOS x86-64 unter Rosetta 2 (derselbe Mac) | Entwicklung | `0c7267a7636aaf7b` | 1 480 284 bestanden |
| Linux ARM64, statisch, unter `qemu-aarch64` in WSL | Entwicklung | `0c7267a7636aaf7b` | nicht gelaufen |
| WebAssembly in Node 22 (V8): einfach, SIMD, Threads | Entwicklung und Release | `0c7267a7636aaf7b` | 1 480 284 bestanden (alle drei) |
| Android 15 x86_64 im Emulator, aus `adb shell` und im Prozess einer App (`untrusted_app`, seccomp) | Entwicklung | `0c7267a7636aaf7b` | 1 480 284 bestanden |

Auf x86-64 und ARM64 rechnet die Wurzel der Prozessor, im Browser und im Android-Emulator `sqrt_portable`; zwei
Rust-Versionen, zwei Optimierungsstufen. Nicht gelaufen: ein echtes Android-Gerät mit ARM64 (nur mit einem Ja des Nutzers je
Gerät), iOS (`cargo check` der Crate für `aarch64-apple-ios` geht, gestartet nicht), Windows auf ARM64, Firefox und Safari
(die WebAssembly-Arithmetik ist spezifiziert; nur V8 lief).

## Messwerte

`physicstest --bench --runs 3` im Release-Build, Mediane. Die Karte: ein Hügelgelände aus 155 × 155 Feldern zu 1 m mit 160
Kisten, Stufen und einer Rampe, 50 068 Dreiecke, 25 684 Ecken. Strahlen: 200 000 von 2 bis 8 m Höhe nach unten und schräg,
40 m lang (79 % treffen). Kapsel-Casts: 40 000 Kapseln (Halbhöhe 0,5, Radius 0,3) knapp über dem Boden, 0,5 bis 3 m zur Seite
und 0,2 m nach unten (96 % treffen). Ein Kern, der Entwicklungsrechner wird von anderen Agenten mitbenutzt: Die Läufe
schwankten bis zum Fünffachen, die Tabelle nimmt den ruhigsten Lauf (die drei Wiederholungen darin lagen innerhalb 3 %).

| Messung | Ryzen 5 3600, Windows | M2, macOS | WebAssembly in Node 22, Ryzen 5 3600 |
| --- | ---: | ---: | ---: |
| Bau des BVH, 50 068 Dreiecke | 60 ms | 39 ms | 83–99 ms |
| Speicher | 2,76 MB, 55,2 Byte je Dreieck | gleich | gleich |
| Strahlen gegen die Karte (`query::ray`) | 1,32 Mio./s | 1,78 Mio./s | 0,67–0,77 Mio./s |
| Kapsel-Casts gegen die Karte (`query::cast`) | 285 000/s | 421 000/s | 150 000–180 000/s |
| Strahlen durch eine Welt nur mit der Karte | 1,18 Mio./s | 1,62 Mio./s | 0,62–0,73 Mio./s |
| Kapsel-Casts durch Karte und 500 Körper (`World::cast`) | 225 000/s | 354 000/s | 140 000–153 000/s |
| Strahlen durch Karte und 500 Körper (`World::ray_cast`) | 714 000/s | 947 000/s | 365 000–386 000/s |
| dasselbe mit Karte und Körpern in einem Baum | 354 000/s | 444 000/s | 111 000–199 000/s |
| `math::sqrt` / `sqrt_portable` / `rt::fx::sqrtd` | 2,2 / 26 / 2,2 ns | 1,0 / 16,5 / 0,9 ns | 39–44 / 38–41 / 210–236 ns |

Der Baum hat 50 053 Knoten bei Tiefe 21: Die SAH macht Blätter mit meist ein bis zwei Dreiecken (die Knoten sind 32 der 55
Byte je Dreieck; Ecken 6, Indizes 12, Ids 4, Kantenbits 1). Im Browser braucht die Physik rund das 1,7- bis 2-Fache der Zeit
nativ; ohne `sqrt_portable` (mit `rt`s Wurzel, 5,5-mal langsamer) wären es die Casts deutlich mehr. Für Spiel 1 heißt das:
16 Figuren mit je vier Casts in jedem Takt bei 60 Hz sind 3840 Casts in der Sekunde, gut 1 % eines Kerns des Servers, im
Browser für die eigene Figur samt Abgleich (zehn Takte nachrechnen) unter 1 ms je Bild.

## Was nicht getestet ist

- Die Ziele oben, die nicht liefen: ARM64 auf Android und Windows, iOS, andere JavaScript-Engines; `--check` unter
  `qemu-aarch64`.
- Hüllen aus mehr als 200 Punkten (der Bau nimmt bis 65 536; die Bergsteigersuche der Stützfunktion ab 33 Ecken läuft in den
  Tests nur mit Kugelwolken bis 200 Punkten), Netze über 50 000 Dreiecke, Welten über 10⁶ m vom Ursprung.
- Netze mit uneinheitlichem Umlaufsinn und nicht-mannigfaltige Kanten (beide gelten als aktiv, gezielt geprüft ist es nicht),
  sehr dünne Splitterdreiecke neben großen.
- EPA mit flachen Minkowski-Differenzen außer dem Fall Kugelmitte in der Dreiecksebene (Kapselachse genau in einer
  Dreiecksebene, zwei koplanare Dreiecke: Netze sind keine Abfrageformen).
- Die Rückfalllösung der Casts (Newton auf dem Abstand) lief in allen Tests einmal; geprüft ist sie damit nur durch diesen
  Fall und durch denselben Vergleich mit der Referenz.
- Gleichzeitige Abfragen aus mehreren Threads (`World` und die Formen sind `Send` und `Sync`, Abfragen nehmen `&self`; der
  Zähler `cast_fallbacks` ist atomar).
- Das Gefühl einer Bewegung: das kommt mit dem Controller und dem Nutzer.

## Was nicht gebaut ist

- Der Character-Controller (G7c Teil 2), Starrkörper, Gelenke und Ragdolls (G7i).
- Drehende Casts (eine Form dreht sich während der Bewegung), Netz gegen Netz, ein Netz als Abfrageform, Höhenfelder,
  zusammengesetzte Formen (mehrere Formen je Körper), skalierte Formen (`Iso` hat keine Skalierung; ein skaliertes Mesh von
  render3d braucht ein eigenes `TriMesh`).
- Zwei bewegte Körper gegeneinander: Ein Cast bewegt eine Form gegen ruhende; für zwei bewegte zieht der Aufrufer die
  Bewegung des einen von der des anderen ab.
- Kontakt-Mannigfaltigkeiten (ein Punkt je Paar und Dreieck), Ereignisse, Schlafen.
- Netze ändern (der BVH wird neu gebaut, 60 ms für 50 000 Dreiecke), ein Refit, ein gespeicherter BVH, komprimierte Knoten,
  SIMD. Größere Blätter sparten bis zu 16 Byte je Dreieck (`MeshOptions::max_leaf` begrenzt nur nach oben; die SAH wählt die
  Größe), kosteten aber Tests von Dreiecken.
- Flächen einer Hülle zusammenfassen: koplanare Dreiecke bleiben einzeln (der Strahl schneidet dann mit mehreren gleichen
  Ebenen, das ist richtig, nur etwas langsamer).

## Für Teil 2: den Character-Controller

- **Der Grundbaustein ist `World::cast`** mit der Kapsel der Figur und `Filter::ALL.excluding(eigener Körper)`: bis `fraction`
  minus einer Haut gehen (1 mm, wie `slide` in `tests.rs`), die restliche Bewegung in die Ebene von `normal` projizieren,
  höchstens vier Runden. `normal` zeigt vom Hindernis zur Figur, `point` liegt auf dem Hindernis.
- **Start-Kontakte**: Eine Figur, die eine Fläche berührt oder ein Haar darin steckt, gleitet frei an ihr entlang und von ihr
  weg; ein Treffer bei 0 kommt nur, wenn die Bewegung tiefer hineinführt, mit `depth` > 0, wenn sie schon drinsteckt. Zum
  Herausdrücken nach einem Teleport oder einem bewegten Hindernis ist `World::contacts` da (alle Körper und Dreiecke mit Tiefe
  und Normale); mehrere Kontakte nacheinander auflösen, nicht addieren.
- **Boden und Rampen**: kurz nach unten casten und `normal.y` mit dem Kosinus des größten Winkels vergleichen. Auf flachen und
  konkaven Nähten kommt die Normale des Dreiecks (aktive Kanten), also stimmt der Winkel auch dort; auf einer echten Kante
  (der Oberkante einer Stufe) kommt die schräge Normale der Kante, was das Erkennen einer Stufe erlaubt (hoch, vor, runter
  casten).
- **Drehung**: Die Kapsel steht auf Y und ist um Y symmetrisch, eine Figur braucht also nur `Iso::at(pos)`. Blickrichtung und
  Bewegung aus Gier- und Nickwinkel nur mit `physics::math::{sin, cos, sincos}` oder `Quat::from_yaw_pitch`, nie mit `f64::sin`
  aus std (das wäre auf jedem Ziel eine andere libm).
- **Bitgleichheit**: Der Controller muss dieselben Operationen in derselben Reihenfolge rechnen (keine Parallelisierung der
  Figuren, die das Ergebnis von der Reihenfolge abhängig macht); quantisiert das Netz Positionen (G7e), quantisieren Server und
  Client gleich, bevor sie weiterrechnen. Die Prüfsumme in `checksum.rs` lässt sich um den Controller erweitern.
- **Bewegte Plattformen**: `set_iso` bewegt sie, ein Cast sieht sie in ihrer jetzigen Lage; die Bewegung der Plattform muss der
  Controller selbst auf die Figur übertragen.
- **Kosten**: rund 285 000 Kapsel-Casts in der Sekunde auf einem Kern gegen eine Karte aus 50 000 Dreiecken, 225 000 mit 500
  Körpern dazu ([Messwerte](#messwerte)).
