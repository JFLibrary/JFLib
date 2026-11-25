# PHYSICS -- Kollision für Spiele

`lib/physics` (`jflib-physics`, in der Fassade `jflib::physics`) ist die Physik der Spiel-Engine ([GAME.md](GAME.md), Stufe G7c,
Entscheidung E5: eine eigene, keine Portierung). Formen (Kugel, Kapsel, Quader, konvexe Hülle, statisches Dreiecksnetz), eine
Breitphase (ein fester BVH je Netz, dynamische AABB-Bäume für die Körper) und die Abfragen, aus denen Treffer und der
Character-Controller entstehen: Strahlen (erster Treffer und alle), bewegte Formen mit Zeitpunkt, Normale und Kontaktpunkt,
Überlappung, Kontakte mit Eindringtiefe, der nächste Punkt. Ohne GPU und ohne Betriebssystem (`#![no_std]`,
`#![forbid(unsafe_code)]`, keine fremden Crates, abhängig nur von `rt`): Der Server von Spiel 1 rechnet ohne Fenster, unter
Linux statisch ohne libc. Und vor allem **bitgleich auf allen Zielen**, damit die Vorhersage eines Clients genau das rechnet,
was der Server rechnet (G7e).

Stand: G7c Teil 1 (Formen, Breitphase, Abfragen, Prüfsummen) und Teil 2: der kinematische Character-Controller
([Der Character-Controller](#der-character-controller)) und eine einzige portable Wurzel in `rt`. Seit U18 (ROADMAP
Abschnitt 13) die Starrkörper mit Kontakten und Gelenken ([Starrkörper](#starrkörper)), der Teil „Physik 2“ von G7i.

| Teil | Wo |
| --- | --- |
| Vektoren, Drehungen, Lagen, Boxen; Wurzel, sin, cos, atan2 | [`math.rs`](../lib/physics/src/math.rs) |
| Formen | [`shape.rs`](../lib/physics/src/shape.rs), Hülle: [`hull.rs`](../lib/physics/src/hull.rs), Netz: [`mesh.rs`](../lib/physics/src/mesh.rs) |
| Breitphase | BVH der Netze: [`bvh.rs`](../lib/physics/src/bvh.rs); dynamischer Baum: [`tree.rs`](../lib/physics/src/tree.rs) |
| Schmalphase | GJK, EPA, Cast: [`gjk.rs`](../lib/physics/src/gjk.rs); Stützfunktionen: [`convex.rs`](../lib/physics/src/convex.rs); Strahlen: [`ray.rs`](../lib/physics/src/ray.rs) |
| Abfragen | eine Form gegen eine: [`query.rs`](../lib/physics/src/query.rs); über alle Körper: [`world.rs`](../lib/physics/src/world.rs) |
| Character-Controller | [`character.rs`](../lib/physics/src/character.rs) |
| Starrkörper, Kontakte, Gelenke | [`sim.rs`](../lib/physics/src/sim.rs), [`solver.rs`](../lib/physics/src/solver.rs), [`manifold.rs`](../lib/physics/src/manifold.rs), [`joint.rs`](../lib/physics/src/joint.rs), [`mass.rs`](../lib/physics/src/mass.rs) ([Starrkörper](#starrkörper)) |
| Tests, Prüfsummen, Messung | [`apps/physicstest`](../apps/physicstest/README.md): Referenzen [`refs.rs`](../apps/physicstest/src/refs.rs), Gruppen [`tests.rs`](../apps/physicstest/src/tests.rs), der Controller [`character.rs`](../apps/physicstest/src/character.rs), [`checksum.rs`](../apps/physicstest/src/checksum.rs), [`bench.rs`](../apps/physicstest/src/bench.rs), im Browser [`tests/web.mjs`](../apps/physicstest/tests/web.mjs) |

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
  verschieben; `transform_point`, `inverse_transform_point`, `mul`, `inv_mul`), `Aabb`. Funktionen: `sqrt` (korrekt gerundet,
  `rt::fx::sqrtd`), `sqrt_portable` (`rt::fx::sqrt_portable`), `sin`, `cos`, `sincos` (Sinus zuerst), `atan2`, `min`, `max`,
  `clamp`.
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
- **Character-Controller**: `CharacterController::new(CharacterParams)`, `step(&state, &world, &filter, &input, dt) ->
  (CharacterState, StepEvents)`, `params`, `jump_speed`, `walkable_y`, `walkable`, `height`, `capsule`, `capsule_iso`;
  `CharacterState { position, velocity, crouched, grounded, ground_normal, ground_body }` (`new`), `CharacterInput { wish,
  run, jump, crouch }`, `StepEvents { landed, impact_speed, jumped, left_ground, hit_ceiling, walls, stepped_up, snapped_down,
  stand_blocked, pushed_out, casts }` (`walls()`, `wall_count()`), `Touch { body, point, normal, triangle }`,
  `MAX_WALL_TOUCHES` ([Der Character-Controller](#der-character-controller)).
- **Starrkörper**: `Simulation`, `RigidBody`, `MassProps`, die Gelenke und Ereignisse ([Starrkörper](#starrkörper), API).
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
  kommt nicht vor. `math::sqrt` ist `rt::fx::sqrtd`: auf x86-64 und ARM64 die Wurzel des Prozessors (`sqrtsd`, `fsqrt`), im
  Browser und im x86-64-Emulator von Android `rt::fx::sqrt_portable`: vier Newton-Schritte in `f64` aus einer Schätzung über
  die Bits, dann ein Test auf der Mantisse in 128-Bit-Ganzzahlen ((R − ½)² < X ≤ (R + ½)²), der den richtigen Nachbarn wählt.
  IEEE 754 verlangt die korrekt gerundete Wurzel, also ist das Ergebnis überall dasselbe; Sonderwerte (±0, Unendlich, NaN,
  negative Zahlen) gibt sie wie `sqrtsd`, NaN eingeschlossen. Bis G7c Teil 1 hatten `lib/physics` und `lib/geom` je eine
  Kopie davon und `rt` im Browser eine Wurzel Bit für Bit (fünf- bis elfmal langsamer, [Messwerte](#messwerte)); seit Teil 2
  gibt es die eine in `rt` (vom Nutzer entschieden am 18.11.2026), `physics::math::sqrt_portable` und
  `geom::math::sqrt_portable` reichen sie durch. Geprüft gegen eine Ganzzahl-Wurzel der Tests (`refs::sqrt_ref`) an rund
  1,5 Mio. Eingaben, Bit für Bit mit NaN: Sonderwerte, zufällige Bitmuster über alles, Subnormale, jede Zweierpotenz mit
  Nachbarn, Quadrate k²·4ʲ mit Nachbarn, die schwersten Fälle (Doubles nächst am Quadrat eines Mittelpunkts).
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
  wechselt er in den Baum der bewegten Körper, mit einer um 0,05 m größeren Box, damit kleine Bewegungen sein Blatt behalten
  (auch ein Netz, etwa eine fahrende Plattform; eine Karte bleibt besser still).
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
| `math` | `sqrt_portable`, `sqrt`, `rt::fx::sqrtd` und `rt`s portables `sqrtd` gegen die Ganzzahl-Wurzel `refs::sqrt_ref` Bit für Bit, NaN eingeschlossen: 25 Sonderwerte (NaN mit Vorzeichen und Nutzlast, signalisierende), 200 000 zufällige Bitmuster über alles, 200 000 positive, 200 000 Zahlen unter 10⁴, 20 000 Subnormale, jede Zweierpotenz mit je vier Nachbarn, 50 000 Quadrate k²·4ʲ mit Nachbarn, 100 000 Doubles nächst am Quadrat eines Mittelpunkts mit je vier Nachbarn; sin/cos/atan2 auf Plausibilität; Drehungen, `Iso`, `from_rotation_arc` | 1 530 516 Prüfungen, 0 falsch |
| `hull` | 1500 Wolken aus 4 bis gut 200 Punkten (Würfel, Kugeloberfläche, Quaderflächen mit koplanaren Punkten, flach 1:1000, bei 10⁵ m, viele Doppelte): kein Punkt außerhalb, Ecken sind Eingabepunkte, geschlossen (jede Kante zweimal, gegenläufig, V − E + F = 2), Flächen nach außen, Stützpunkt gegen alle Punkte; Volumen von Quadern; was verweigert wird | 0 falsch |
| `ray` | 6000 Strahlen gegen Kugel, Kapsel, Quader, Hülle (Treffer auf der Oberfläche, nicht nach dem Eintritt der Referenz, kein Fehlen), die Kugel auch gegen die geometrische Formel (10⁻¹²); 20 000 gegen Dreiecke gegen Möller-Trumbore; Wasserdichtheit: Strahlen auf jede Ecke, Kante und Fläche eines Gitters mit wechselnden Diagonalen (senkrecht und schräg, von oben und unten; schräg nicht auf den offenen Rand) und 20 000 zufällige, dazu 20 000 aus einem geschlossenen Quader heraus, auch genau auf seine Ecken | 0 durchgerutscht |
| `distance` | 8000 Paare aus allen Formen und Dreiecken: Abstand (10⁻⁹ m), Eindringtiefe von EPA gegen die Trennachsen (10⁻⁷), die Normale trennt, Punkte auf den Oberflächen; `intersects`, `contacts`; analytisch Kugel–Kugel, Kapsel–Kapsel, Kugel–Quader; 8000 nächste Punkte | größter Fehler 2,7 · 10⁻¹⁵ m getrennt, 1,3 · 10⁻¹⁵ m überlappend |
| `cast` | 4000 Casts aller Formen gegen alle Formen und Dreiecke: am gemeldeten Zeitpunkt Lücke zwischen 0 und Ziel + 10⁻⁸ m, nie nach dem ersten Kontakt, kein verpasster Kontakt, Start-Kontakte nach der Regel; Kugel gegen Kugel gegen die exakte Zeit | 0 falsch |
| `tunnel` | 20 000 kleine Kugeln (1 bis 50 mm) mit bis zu 10 km je Schritt durch Dreiecke, 10 000 Kapseln und Quader durch eine 1 mm dünne Wand, 5000 Fälle aus 25 m bis 10 km Höhe auf die Karte | 0 durchgetunnelt (bis 9991 m je Schritt) |
| `seams` | 6000 Gleitbewegungen von Kugel, Kapsel und gedrehtem Quader über ein flaches Gitter, 1 mm darüber bis 10 µm darin; dasselbe ohne aktive Kanten als Gegenprobe; Fallen auf jede Ecke und Kante des Gitters (Normale genau nach oben); 3000 Kugeln auf Hügel fallen lassen (Kontakte in Ruhe zeigen nach oben, ≤ 10⁻⁸ m tief); 40 Läufer mit Kollidieren und Gleiten über die Karte, je bis 1500 Schritte | 0 blockiert, 0 eingesunken |
| `bvh` | der Baum einer Karte aus 3050 Dreiecken (jedes in einem Blatt, Boxen halten alles); 8000 Strahlen, 1000 `ray_all`, 1000 Casts, Kontakte, `intersects`, `separation`, nächste Punkte gegen `query::brute`, Bit für Bit | 0 anders |
| `world` | Karte und 400 Körper in zufälligen Ebenen; 60 Runden mit Verschieben, Entfernen, Hinzufügen und `validate` der Bäume; je 40 Abfragen aller Arten mit vier Filtern gegen Schleifen über alle Körper, Bit für Bit; Generationen | 0 anders |
| `fuzz` | 40 000 zufällige Paare mit Größen von 3 mm bis 300 m, ein Fünftel bei bis zu 10⁵ m: keine Panik, nichts nicht Endliches, Normalen der Länge 1, Brüche in [0, 1], `intersects` passt zum Abstand, Abstand symmetrisch | größte Asymmetrie 5 · 10⁻¹⁵ der Größe |
| `character` | der Controller in Szenen mit genauen Erwartungen ([Tests des Controllers](#tests-des-controllers)) | 447 Prüfungen, 0 falsch |
| `charfuzz` | 86 400 Schritte zufälliger Figuren auf 24 zufälligen, dichten Karten, Würfe bis 1000 m/s ([Tests des Controllers](#tests-des-controllers)) | 0 in etwas, 0 durch eine Wand |
| `mass` bis `dynfuzz` | die Starrkörper ([Tests der Starrkörper](#tests-der-starrkörper)) | 1271 Prüfungen, 0 falsch |
| `edge` | Netze, die verweigert werden, leere und flache Netze, ein Netz als Abfrageform, Casts ohne Bewegung, Berühren und Weggehen/Entlanggleiten/Hineinfahren, Strahl ohne Richtung, 3000 Casts bei 10⁶ m gegen dieselben am Ursprung, eine Bewegung von 1000 km, sortierte Kontakte an einer Ecke | 0 falsch |

## Prüfsummen

`physicstest --checksum` rechnet feste Szenarien und faltet die Bits jedes Ergebnisses (FNV-1a über `f64::to_bits`) in drei
Zahlen; die dritte, über Starrkörper und Gelenke, steht unter [Prüfsumme der Starrkörper](#prüfsumme-der-starrkörper). Die erste (Zeile `physicstest checksum=`) seit Teil 1 unverändert: 20 000 Wurzeln (beide Wege), Sinus, Kosinus, atan2 und Drehungen; 20 Hüllen; der Baum und die Kantenbits einer Karte aus
7778 Dreiecken; eine Welt aus Karte und 300 Körpern über 10 Runden mit Verschieben, Entfernen und Hinzufügen, darin 3000 Strahlen,
300 `ray_cast_all`, 1500 Casts, Überlappungen, Kontakte, nächste Punkte; 3000 Paare mit Abstand und Cast; sechs Läufer mit
Kollidieren und Gleiten über 600 Schritte. Zusammen 294 963 Werte. Die zweite (`physicstest controller checksum=`, seit Teil 2):
zwölf Figuren aus vier Parametersätzen (Vorgabe, schnell, klein, groß) als Körper einer Welt aus der Karte mit einem Zaun und
40 Körpern, zufällige Eingaben (alle 20 Takte neu) und Würfe bis 30 m/s, 600 Takte; je Takt und Figur Lage, Geschwindigkeit,
Bodennormale, die Ereignisse und die berührten Wände, zusammen 113 214 Werte.

| Ziel | Build | Prüfsumme | Controller (Teil 2) | `--check` (Teil 2) |
| --- | --- | --- | --- | --- |
| Windows x86-64 (Ryzen 5 3600), Rust 1.100 | Entwicklung und Release (fat LTO) | `0c7267a7636aaf7b` | `13dd288c669c6eaa` | 2 047 737 bestanden |
| Linux x86-64, statisch ohne libc, in WSL Ubuntu 24.04 | Entwicklung | `0c7267a7636aaf7b` | `13dd288c669c6eaa` | 2 047 737 bestanden |
| macOS ARM64 (Mac mini M2, macOS 27.2.1), Rust 1.98.1 | Entwicklung und Release | `0c7267a7636aaf7b` | `13dd288c669c6eaa` | 2 047 737 bestanden |
| macOS x86-64 unter Rosetta 2 (derselbe Mac) | Entwicklung | `0c7267a7636aaf7b` (Teil 1) | nicht gelaufen: Rosetta fehlt seit dem Update auf 27.2.1 | – |
| Linux ARM64, statisch, unter `qemu-aarch64` in WSL | Entwicklung | `0c7267a7636aaf7b` (Teil 1) | nicht gelaufen | nicht gelaufen |
| WebAssembly in Node 22 (V8): einfach, SIMD, Threads | Entwicklung und Release | `0c7267a7636aaf7b` | `13dd288c669c6eaa` | 2 047 737 bestanden |
| Android 15 x86_64 im Emulator, aus `adb shell` und im Prozess einer App (`untrusted_app`, seccomp) | Entwicklung und Release | `0c7267a7636aaf7b` | `13dd288c669c6eaa` | 2 047 737 bestanden (`adb shell`) |

Auf x86-64 und ARM64 rechnet die Wurzel der Prozessor, im Browser und im Android-Emulator `rt::fx::sqrt_portable`; zwei
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

**Seit Teil 2** (`--bench sqrt`, `--bench controller`; der Entwicklungsrechner lief dabei unter Volllast anderer Agenten, seine
Werte schwankten bis zum Fünffachen und taugen nur für Verhältnisse im selben Lauf; der M2 war ruhiger):

| Messung | M2, macOS (Release) | Ryzen 5 3600, Windows (Release, unter Last) | WebAssembly in Node 22 (Release, unter Last) | Android-Emulator x86_64 (Release) |
| --- | ---: | ---: | ---: | ---: |
| Character-Controller: Schritte je ms, 32 Figuren als Körper auf der Karte aus 50 068 Dreiecken, zufällige Eingaben, 600 Takte | 178 | 31–69 | 15–29 | 21–40 |
| Casts je Schritt im Mittel | 3,02 | 3,02 | 3,02 | 3,02 |
| `rt::fx::sqrt_portable` | 15,8 ns | 42 ns | 74–273 ns | 60–90 ns |
| `rt::fx::sqrtd` vorher (Bit für Bit) / nachher | 0,9 ns (Prozessor) | Prozessor | 474–1888 / 66–343 ns | 700–1201 / 46–76 ns |
| die Ganzzahl-Wurzel der Tests (wie `rt`s alte) | 110 ns | 188 ns | 574–1819 ns | 627–1048 ns |

Im Browser und im Emulator ist `rt::fx::sqrtd` jetzt so schnell wie `sqrt_portable`: im selben Lauf war es vorher 4,9- bis
8,2-mal langsamer als `sqrt_portable` (Node) und 9- bis 20-mal (Emulator; in einem ruhigeren Lauf 493 gegen 44 ns). Das betrifft alles, was dort `rt::fx::sqrtd` rechnet (`gfx`, `fx`,
`render2d` und andere), nicht die Physik, die dort schon `sqrt_portable` nahm. Der Controller braucht auf dem M2 für 32 Figuren
bei 60 Hz rund 1 % eines Kerns (5,6 µs je Schritt); im Wechsel mit dem nativen Programm gemessen braucht er im Browser
rund das 1,5- bis 3-Fache (nativ 65 und 31, WebAssembly 22 und 21 Schritte je ms, alles unter Last).

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
- Gleichzeitige Abfragen aus mehreren Threads (`World`, `Shape` und `TriMesh` sind `Send` und `Sync`, das prüft der Test beim
  Übersetzen; Abfragen nehmen `&self`, der Zähler `cast_fallbacks` ist atomar).
- Das Gefühl der Bewegung (die Vorgaben des Controllers): das prüft der Nutzer von Hand, wenn eine Demo die Figur steuert.
- Der Controller: bewegte Plattformen (nicht gebaut), Figuren in großer Zahl gegeneinander (die Prüfsumme hat zwölf, die Messung
  32, gezielte Tests von Begegnungen keine), Netze mit uneinheitlichem Umlaufsinn als Boden, Takte anders als 60 Hz (die
  Rechnung hängt nicht daran; geprüft ist nur 1/60 s), Figuren fern vom Ursprung (die Messung reicht bis 155 m).

## Was nicht gebaut ist

- Was den Starrkörpern fehlt, steht unter [Grenzen der Starrkörper](#grenzen-der-starrkörper).
- Am Controller: bewegte Plattformen (die Bewegung des Bodens überträgt er nicht; `ground_body` sagt, worauf die Figur steht),
  eine Form ändern (`World` kann es nicht; eine geduckte Figur als Körper braucht `remove` und `add`), eine liegende oder
  gedrehte Kapsel, Schwimmen, Leitern, Rutschen beim Ducken. In einer V-Rinne aus zwei zu steilen Flächen ruht die Figur,
  gilt aber nicht als am Boden (kein Sprung). Unter rund 0,33 m/s nimmt sie keine Stufe.
- Drehende Casts (eine Form dreht sich während der Bewegung), Netz gegen Netz, ein Netz als Abfrageform, Höhenfelder,
  zusammengesetzte Formen (mehrere Formen je Körper), skalierte Formen (`Iso` hat keine Skalierung; ein skaliertes Mesh von
  render3d braucht ein eigenes `TriMesh`).
- Zwei bewegte Körper gegeneinander: Ein Cast bewegt eine Form gegen ruhende; für zwei bewegte zieht der Aufrufer die
  Bewegung des einen von der des anderen ab.
- In den Abfragen der `World`: Kontakt-Mannigfaltigkeiten (ein Punkt je Paar und Dreieck), Ereignisse, Schlafen (das hat die
  `Simulation`).
- Netze ändern (der BVH wird neu gebaut, 60 ms für 50 000 Dreiecke), ein Refit, ein gespeicherter BVH, komprimierte Knoten,
  SIMD. Größere Blätter sparten bis zu 16 Byte je Dreieck (`MeshOptions::max_leaf` begrenzt nur nach oben; die SAH wählt die
  Größe), kosteten aber Tests von Dreiecken.
- Flächen einer Hülle zusammenfassen: koplanare Dreiecke bleiben einzeln (der Strahl schneidet dann mit mehreren gleichen
  Ebenen, das ist richtig, nur etwas langsamer).

## Der Character-Controller

`CharacterController` ([`character.rs`](../lib/physics/src/character.rs), G7c Teil 2) bewegt eine aufrechte Kapsel kinematisch
durch eine `World`: gehen, laufen, springen, ducken, Stufen bis `step_height`, Rampen bis `max_slope_deg` (steilere Flächen sind
Wände, an denen die Figur herabrutscht), an Wänden und in Ecken entlanggleiten, am Boden bleiben auf dem Weg Rampen und Treppen
hinunter, Steuerung in der Luft. Ein Schritt ist eine Funktion von Zustand, Eingabe, Welt und `dt` allein, mit denselben Bits auf
jedem Ziel: dieselbe Rechnung wie die Abfragen (`f64`, `+ − × ÷`, die korrekt gerundete Wurzel; der Kosinus des Grenzwinkels
einmal in `new` aus `rt`s portabler Mathematik), keine ungeordneten Container, feste Grenzen für jede Schleife.

```rust
use jflib::physics::{CharacterController, CharacterInput, CharacterParams, CharacterState, Filter, Vec3, math};

let figur = CharacterController::new(CharacterParams::default());
let mut zustand = CharacterState::new(Vec3::new(10.0, 2.0, 10.0)); // die Füße: der tiefste Punkt der Kapsel
// als Körper der Welt, damit Strahlen und andere Figuren sie treffen
let koerper = welt.add(figur.capsule(false), figur.capsule_iso(&zustand));

// in jedem Takt (60 Hz): die Richtung aus dem Gierwinkel (0 schaut nach -Z), nur mit physics::math
let (s, c) = math::sincos(gier);
let eingabe = CharacterInput { wish: Vec3::new(-s, 0.0, -c), run: true, jump: springen, crouch: ducken };
let (neu, ereignisse) = figur.step(&zustand, &welt, &Filter::ALL.excluding(koerper), &eingabe, 1.0 / 60.0);
welt.set_iso(koerper, figur.capsule_iso(&neu));
zustand = neu;
// ereignisse.landed und .impact_speed, .jumped, .left_ground, .hit_ceiling, .walls(), .stepped_up, .snapped_down, ...
```

### Ein Schritt

1. **Herausdrücken**, wenn die Kapsel in etwas steckt (nach einem Teleport, einem bewegten Körper, einer gerundeten Position aus
   dem Netz): der tiefste Kontakt aus `World::contacts` zuerst, entlang seiner Normale um Tiefe und Haut, als Cast (so geht das
   Herausdrücken durch nichts anderes hindurch), höchstens vier Runden. Die eigenen Bewegungen des Controllers sind Casts und
   enden nie in etwas.
2. **Boden**: ein Cast um drei Häute nach unten. Er trägt, wenn die Normale begehbar ist (`y ≥ cos(max_slope_deg)`), oder wenn
   der Kontakt auf der Kante einer begehbaren Fläche liegt (Stufe, Sims): ein Strahl knapp hinter dem Kontakt (2 cm, weg von
   der Achse) findet diese Fläche, ihre Normale gilt. Nicht, wenn sich die Figur schneller als 0,25 m/s von ihm entfernt
   (Sprung, ein Stoß von außen).
3. **Ducken und Aufstehen**: Am Boden kommt beim Ducken der Kopf herunter, in der Luft kommen die Beine hoch (ein Sprung mit
   Ducken erreicht 0,6 m Höheres). Aufstehen nur mit Platz: Ein Cast der geduckten Kapsel um den Höhenunterschied nach oben
   überstreicht genau die stehende Kapsel; in der Luft gehen zuerst die Beine nach unten, so weit Platz ist.
4. **Springen, Beschleunigen, Bremsen**: Ein Sprung setzt die senkrechte Geschwindigkeit auf `sqrt(2 · gravity ·
   jump_height)`. Am Boden mit Eingabe nähert sich die Geschwindigkeit der Wunschgeschwindigkeit (Wunschrichtung entlang des
   Bodens mal Wunschgeschwindigkeit) um höchstens `accel` · Wunschgeschwindigkeit · `dt`; ohne Eingabe bremst Quakes Reibung
   (um `friction` · max(Geschwindigkeit, `stop_speed`) · `dt`). In der Luft Quakes Beschleunigung: Die Geschwindigkeit entlang
   der Wunschrichtung wächst um höchstens `air_control` · `accel` · Wunschgeschwindigkeit · `dt`, nicht über die
   Wunschgeschwindigkeit. Danach die Grenzen `max_speed` (waagrecht) und `max_fall_speed`.
5. **Bewegen**: Gleiten mit bis zu vier Casts, jeder hält `skin` vor dem Kontakt (entlang der Bewegung), die Geschwindigkeit
   gedreht nach Quake 3s Clip-Ebenen (`PM_SlideMove`): die Ebene des Bodens und die ursprüngliche Richtung von Anfang an, jede
   getroffene Fläche dazu; geht die geclippte Geschwindigkeit in eine zweite Ebene, gleitet sie entlang der Kante beider, geht
   diese in eine dritte, hält die Figur. Am Boden zählen steile Flächen als senkrechte Wände (gegen sie drücken klettert
   nichts hinauf), eine begehbare dreht die Geschwindigkeit auf sich bei gleichem Betrag (Rampen hinauf ohne Verlust). Bleibt
   die Figur am Boden an etwas hängen, versucht sie eine **Stufe**: hoch (höchstens `step_height`, so weit Platz ist),
   waagrecht vor mit derselben Geschwindigkeit (alles als senkrechte Wand: sie rutscht an keiner Kante hoch), hinunter auf
   tragenden Boden höchstens `step_height` über den Füßen; genommen, wenn sie so weiter kommt.
6. **Boden am Ende**: Stand sie am Anfang und sprang nicht, rastet sie auf Boden bis `step_height` darunter ein (Rampen und
   Treppen hinunter, ohne abzuheben); sonst landet sie auf Boden innerhalb der drei Häute. Beim Landen bleibt die waagrechte
   Geschwindigkeit, entlang des Bodens. Kein Boden: Sie fällt weiter, und wer stand, hat ihn verlassen (`left_ground`).

Die Schwerkraft rechnet in der Luft mit der mittleren Geschwindigkeit des Takts (`v − g · dt / 2`): Nach k freien Takten ist die
Höhe genau `y₀ + v₀ t − g t² / 2` mit `t = k · dt` (bis auf Rundung), bei jeder Taktrate.

### Parameter

`CharacterParams::default()` ist für einen schnellen Cartoon-Arena-Shooter gestimmt; `CharacterController::new` begrenzt, was
keinen Sinn ergibt (Radius ab 1 cm, Höhen ab dem doppelten Radius, Haut 0,1 mm bis ein Viertel des Radius, Winkel 0 bis 89°,
nichts Negatives; nicht Endliches nimmt die Vorgabe).

| Feld | Vorgabe | Warum |
| --- | --- | --- |
| `radius` | 0,4 m | eine Breite von 0,8 m wie Quakes 32 Einheiten; Cartoon-Figuren sind breit |
| `height`, `crouch_height` | 1,8 m, 1,2 m | geduckt ein Drittel kleiner: unter Decken ab 1,2 m hindurch |
| `walk_speed`, `run_speed`, `crouch_speed` | 5, 9, 3 m/s | laufen (die übliche Gangart) etwas schneller als Quakes 320 Einheiten/s (8,1 m/s); gehen ist der leise Modus |
| `accel` | 12 /s | mit Eingabe in 1/12 s auf voller Geschwindigkeit (fünf Takte bei 60 Hz), Richtungswechsel ebenso schnell |
| `friction`, `stop_speed` | 6 /s, 2,5 m/s | Quakes Werte: ohne Eingabe aus vollem Lauf in rund 0,4 s zum Stand |
| `air_control` | 0,3 | in der Luft 30 % der Beschleunigung am Boden; wie in Quake 3 bringt Lenken schräg zur Bewegung Geschwindigkeit (Strafe-Jumping) bis `max_speed`, Bunny-Hopping verliert nichts |
| `gravity`, `jump_height` | 20 m/s², 1,2 m | doppelt so stark wie auf der Erde: kurze, knackige Sprünge (Absprung 6,93 m/s, 0,69 s in der Luft) auf eine Kiste |
| `step_height` | 0,4 m | Treppen und Kanten ohne Springen (Quake: 18 Einheiten, 0,46 m) |
| `max_slope_deg` | 45° | Quakes Grenze (Normale y ≥ 0,7, 45,6°) |
| `skin` | 5 mm | der Abstand zu allem: unsichtbar, hält die Casts aus dem Berühren; nach einer schrägen Landung zwischen 0 und `skin` |
| `max_speed`, `max_fall_speed` | 30 m/s, 50 m/s | Obergrenzen, auch gegen unsinnige Geschwindigkeiten von außen |

### Entscheidungen

- **Am Boden Annäherung statt Quakes Beschleunigung.** Quakes Formel (Geschwindigkeit entlang des Wunsches wächst bis zur
  Wunschgeschwindigkeit, Reibung in jedem Takt) macht eine Figur, die schräg gegen eine Wand drückt, schneller als frei (bei
  45° rund 10,6 statt 9 m/s), und ein Stick unter `stop_speed` · `friction` / `accel` (1,25 m/s) käme gegen die Reibung nie in
  Gang. Mit der Annäherung bleibt an der Wand genau der Anteil der Wunschgeschwindigkeit entlang der Wand, und jeder Stick
  erreicht seine Geschwindigkeit. Die Reibung bremst nur ohne Eingabe; in der Luft bleibt Quakes Modell (Luftsteuerung,
  Strafe-Jumping), und ein Sprung im ersten Takt am Boden kommt vor jeder Reibung: Bunny-Hopping hält die Geschwindigkeit
  Bit für Bit.
- **Quake 3s Clip-Ebenen** statt einfachem Projizieren auf die zuletzt getroffene Fläche: In einem spitzen Winkel zwischen zwei
  Wänden würde das Projizieren hin und her springen. Mit den Ebenen ruht die Figur in jeder Ecke (Test: 60 Takte ohne die
  kleinste Bewegung, beide Wände innerhalb der Haut).
- **Stufen ohne Hochrutschen.** Die Bewegung nach vorn auf Stufenhöhe ist waagrecht, und der Auflagepunkt der Landung darf
  höchstens `step_height` über den Füßen liegen: Sonst rollte die Kugel der Kapsel über höhere Kanten (zuerst kam die Figur so
  0,45 und 0,6 m hohe Stufen hinauf). Jetzt ist die höchste Stufe `step_height` plus die Haut.
- **Kanten tragen.** Ohne das hinge die Kugel nach einer Stufe auf der Kante, rutschte
  zurück, und langsam (1 m/s) käme die Figur keine Treppe hinauf. So steht sie auch an einem Sims, bis die Kugel ganz darüber
  hinaus ist, wie in Quake mit dem Quader. Erst unter rund 0,33 m/s reicht ein Takt nicht über die Haut hinaus, und eine
  Stufe geht nicht.
- **Der Zustand ist Lage, Geschwindigkeit und Ducken.** `grounded`, `ground_normal`, `ground_body` sind Ausgaben; jeder Schritt
  findet den Boden neu (ein Cast mehr). So muss das Netz (G7e) nur diese drei abgleichen, und eine gerundete Lage passt von
  selbst wieder zum Boden.
- **Die Haut ist 5 mm.** Die Casts halten sie entlang der Bewegung, also senkrecht zur Fläche `skin` · cos(Winkel): an einer
  Wand, die die Figur schräg trifft, und nach einer schrägen Landung weniger, nie 0 (bis auf 10⁻⁹ m der Casts). Das Einrasten
  am Boden zieht nur nach unten (kein Cast nach oben), eine Figur in Ruhe kriecht nicht (Schwelle 10⁻⁹ m).

### Tests des Controllers

`physicstest --test character` (Szenen) und `--test charfuzz` (zufällige Läufe), in `--check` enthalten:

| Szene | Erwartung | Ergebnis |
| --- | --- | --- |
| Sprung auf einem Gitter (auf einer Ecke, einer Kante, einer Fläche) | Scheitel = Maximum von `y₀ + v₀ t − g t²/2` über die Takte (10⁻⁹ m), höchstens `g dt² / 8` unter `jump_height`; Landung genau im ersten Takt, der innerhalb von drei Häuten endet, Aufprall `g (k − ½) dt − v₀` (10⁻⁹) | gleich |
| Fall aus 3 m, Bunny-Hopping | Landetakt und Aufprall nach der Formel; mit gehaltenem Sprung bleiben 9 m/s waagrecht über vier Sprünge (10⁻⁹) | gleich |
| Treppen mit Stufen 0,15 / 0,25 / 0,375 m bei 9, 5, 3 und 1 m/s | oben, nie in der Luft, auf `n · Stufe + skin` (10⁻⁹) | alle 12 |
| Stufen 0,45 und 0,6 m | angehalten mit der Achse bei `x₀ − r` (10⁻⁹), am Boden | gleich |
| Rampen 20°, 30°, 40°, 44° (begehbar) | oben auf `Höhe + skin`, nie in der Luft; halb oben abgesetzt: steht still (200 Takte ohne Bewegung) | alle |
| Rampen 46°, 50°, 60°, 75° (zu steil) | die Kugel hält dort, wo sie die Ebene berührt: Achse bei `x₀ + ((skin + r) cos θ − r) / sin θ` (bis 2 Häute davor), nie höher; halb oben abgesetzt: rutscht hinunter, nie am Boden auf der Rampe | alle |
| Wand (gedrehter Quader in der Welt, gedrehtes Prisma im Netz) unter 9 Winkeln 0° bis 89°, vier Drehungen | ohne Reibung und Beschleunigung: Geschwindigkeit danach genau der Anteil entlang der Wand (10⁻⁹), Weg entlang der Wand bis auf die Haut, Abstand zwischen 0 und `skin`; mit dem Vorgabegefühl: `run_speed` · sin φ entlang der Wand (10⁻⁹), 10 Takte ohne Bewegung zur Wand hin (10⁻⁹) | 72 Fälle gleich |
| V-Ecken 30°, 60°, 90°, 120°, 150°, entlang der Winkelhalbierenden und 10° daneben | 60 Takte ohne Bewegung (10⁻⁹), Abstand zu beiden Wänden zwischen 0 und `skin` | 15 Fälle |
| Decke auf 1,5 m | stehend angehalten an ihrer Unterkante (Achse bei `4 − r_rand`); geduckt hindurch; Aufstehen genau, wenn die stehende Kapsel passt (jeder Takt gegen die Rechnung, 10⁻⁶ Randzone ausgenommen); Sprung mit Ducken genau 0,6 m höher | gleich |
| Rampen 15°, 30°, 44° und eine Treppe hinunter, laufend, gehend, langsam | nie in der Luft, immer innerhalb der Haut an etwas, eingerastet | alle |
| Sims von 1 m | einmal `left_ground`, einmal `landed` | gleich |
| Teleport bis zu einem halben Radius in einen Quader oder den Boden (200 Fälle) | nach einem Schritt in nichts mehr (10⁻⁹), `pushed_out` > 0 | alle |
| dieselbe Figur als Körper der Welt (vom Filter ausgelassen) | 240 Takte Bit für Bit gleich ohne ihn | gleich |
| `dt` 0, negativ, NaN, unendlich; Lage NaN; Geschwindigkeit NaN; unsinnige Parameter | nichts ändert sich; Geschwindigkeit 0; begrenzt | gleich |

**Zufällige Läufe** (`charfuzz`): 24 zufällige Karten, je eine dichte Arena von 24 × 24 × 8 m aus Hügeln, Wänden und Decke, quer
geteilt durch eine Wand von 2 bis 30 cm Dicke ohne Lücke; darin Kisten, Rampen von 10° bis 70° in allen vier Richtungen,
Treppen mit Stufen bis 0,45 m, niedrige Decken, dünne Wände (1 bis 5 cm), schräge Platten und 8 bis 17 konvexe Körper. In
jeder Karte sechs Figuren mit je einem der Parametersätze Vorgabe, schnell (bis 1000 m/s), klein (Radius 0,1 m), groß
(Radius 0,8 m, 3 m hoch), leicht (Schwerkraft 2), Eis (ohne Reibung, 30°), 600 Takte mit zufälligen Eingaben und Würfen (je
Takt mit 1/150 eine zufällige Geschwindigkeit bis 60 m/s, für die schnelle Figur bis 1000 m/s, also 16,7 m je Takt). Nach
jedem Schritt: endlich, in nichts (Tiefe ≤ 10⁻⁶ m), noch in der eigenen Hälfte der Arena, nicht unter den Hügeln. Ergebnis:
86 400 Schritte, 0 Fehler, tiefste Überlappung 0; dabei 990 Landungen, 772 Sprünge, 2166 Stufen, 7853 Einrasten, 35 670
Wandkontakte, 655 Decken. **Nichts tunnelt bis 1000 m/s** (schnellster Wurf 965 m/s). Die Gegenprobe: eine Figur, die sich
ohne Casts bewegt, fangen dieselben Prüfungen.

### Für G7e und G7h

- **Vorhersage**: Der Client ruft `step` mit denselben Eingaben, `dt` und derselben Welt wie der Server und bekommt dieselben
  Bits (Prüfsumme auf allen Zielen). Abzugleichen sind `position`, `velocity` und `crouched`; alles andere leitet der Schritt
  ab. Quantisiert der Server Lagen für das Netz (G7e), rechnet er selbst mit dem quantisierten Stand weiter, sonst weichen
  Client und Server ab. Steckt eine gerundete Lage in etwas, drückt der nächste Schritt sie heraus (`pushed_out`); bei einer
  Rundung bis rund 1 mm geschieht das nie (die Haut ist 5 mm).
- **Die Welt der Vorhersage**: Karte und Körper mit denselben Ids und Lagen (bei exakt gleichem Abstand entscheidet die Id).
  Andere Figuren kennt der Client nur interpoliert; was daraus folgt, ist ein Vorhersagefehler, kein Determinismusfehler.
  Der Server bewegt die Figuren in einer festen Reihenfolge (nach Id) und setzt nach jedem Schritt `set_iso` der Kapsel; die
  Reihenfolge bestimmt, wer bei einer Begegnung zuerst ausweicht.
- **Eine geduckte Figur als Körper**: `World` kann die Form eines Körpers nicht ändern. Bis dahin bleibt die stehende Kapsel
  (wie in Prüfsumme und Messung) oder die Figur wird neu hinzugefügt (neue Id). Für Treffer (G7h) sind ohnehin eigene
  Trefferzonen besser als die Kapsel.
- **Kosten**: auf dem M2 178 Schritte je ms mit 32 Figuren auf 50 000 Dreiecken (rund 3 Casts je Schritt): der Server von
  Spiel 1 braucht für 32 Figuren bei 60 Hz rund 1 % eines Kerns, ein Client für zehn Takte Abgleich der eigenen Figur rund
  0,06 ms. Im Browser rund das Doppelte ([Messwerte](#messwerte)).
- **Ereignisse** für Klang und Animation: `landed` mit `impact_speed`, `jumped`, `left_ground`, `stepped_up`, `snapped_down`,
  `hit_ceiling`, `walls()` (Körper, Punkt, Normale, Dreieck; für Wandsprünge später).
- **Eingabe**: `wish` aus dem Gierwinkel nur mit `physics::math::sincos` (oder `Quat::from_yaw_pitch`), nie mit `f64::sin` aus
  std.

## Starrkörper

Seit U18 (ROADMAP Abschnitt 13, der Teil „Physik 2“ von G7i) bewegt eine `Simulation` die Körper ihrer `World`: statische
(eine Karte), kinematische, die nur ihre Geschwindigkeit bewegt (eine Tür, eine Plattform, ein Aufzug), und dynamische, die
Schwerkraft, Kräfte, Kontakte und Gelenke bewegen. Rays, Casts und der Character-Controller fragen `sim.world()` und sehen
jeden Körper dort, wo er nach dem letzten Schritt ist.

| Teil | Wo |
| --- | --- |
| Masse, Schwerpunkt, Trägheitstensor | [`mass.rs`](../lib/physics/src/mass.rs) |
| Kontaktpunkte (Manifolds) | [`manifold.rs`](../lib/physics/src/manifold.rs) |
| Löser: Körper des Schritts, weiche Kontakte | [`solver.rs`](../lib/physics/src/solver.rs) |
| Gelenke | [`joint.rs`](../lib/physics/src/joint.rs) |
| Simulation: Körper, Schritt, Paare, Sweeps, Schlaf, Ereignisse | [`sim.rs`](../lib/physics/src/sim.rs) |
| Tests, Prüfsumme, Messung | [`apps/physicstest/src/dynamics.rs`](../apps/physicstest/src/dynamics.rs) |

```rust
use jflib::physics::{ContactEvent, Iso, JointKind, Quat, RigidBody, Shape, Simulation, Vec3};

let mut welt = Simulation::new(); // Schwerkraft 9,81 m/s² nach -Y, vier Teilschritte je Schritt
welt.add(RigidBody::fixed(Shape::mesh(karte), Iso::IDENTITY))?;
let kiste = welt.add(RigidBody::dynamic(Shape::cuboid(Vec3::splat(0.5)), Iso::at(Vec3::new(0.0, 5.0, 0.0))))?;
let fass = welt.add(RigidBody::dynamic(Shape::capsule(0.4, 0.3), Iso::at(Vec3::new(1.0, 6.0, 0.0)))
    .with_density(400.0)
    .with_rolling_resistance(0.05))?;

// eine Tür an einem Scharnier (die X-Achse des Gelenks ist seine Achse), bis 100 Grad offen
let pfosten = welt.add(RigidBody::fixed(Shape::sphere(0.05), Iso::at(Vec3::new(4.0, 1.0, 0.0))).with_layers(0, 0))?;
let tuer = welt.add(RigidBody::dynamic(Shape::cuboid(Vec3::new(0.5, 1.0, 0.05)), Iso::at(Vec3::new(4.5, 1.0, 0.0))))?;
let hoch = Quat::from_axis_angle(Vec3::Z, core::f64::consts::FRAC_PI_2);
welt.add_joint(pfosten, tuer, Iso::new(Vec3::new(4.0, 1.0, 0.0), hoch), JointKind::Hinge { limits: Some((0.0, 1.75)), motor: None, spring: None })?;

welt.step(1.0 / 60.0);
let lage = welt.iso(kiste); // für den Renderer
for e in welt.events() {
    if let ContactEvent::Begin { speed, .. } = e { /* ein Geräusch so laut wie der Aufprall */ }
}
```

### API

- **Körper**: `RigidBody::{fixed, dynamic, kinematic}(shape, iso)` mit `with_velocity`, `with_angular_velocity`,
  `with_density` (kg/m³, Vorgabe 1000), `with_mass(MassProps)`, `with_friction` (Vorgabe 0,6; zwei Körper nehmen die Wurzel
  des Produkts), `with_restitution` (0; das Größere), `with_rolling_resistance` (0), `with_damping` (linear 0, Drehung 0,05
  je Sekunde), `with_gravity_scale`, `with_layers(layers, mask)` (zwei Körper stoßen, wenn jede Maske eine Ebene des anderen
  hat), `with_user`, `sensor()`, `bullet()`, `sleepless()`, `gyroscopic()`, `fixed_rotation()`, `asleep()`. Ein Netz kann
  statisch oder kinematisch sein, nicht dynamisch. `BodyError::{DynamicMesh, Mass, NotFinite}`.
- **Simulation**: `new`, `with_params(SimParams)`, `add`, `remove` (mit seinen Gelenken; was ihn berührte, wacht auf),
  `world`, `state(id) -> BodyState { kind, iso, center, linear_velocity, angular_velocity, mass, awake }`, `iso`, `velocity`,
  `velocity_at`, `mass`, `is_awake`, `set_iso`, `set_velocity`, `apply_impulse`, `apply_angular_impulse`, `apply_force`,
  `apply_torque` (für den nächsten Schritt), `move_kinematic(id, ziel, dt)` (setzt die Geschwindigkeiten so, dass er nach
  einem Schritt dort ist, und endet genau dort), `wake`, `set_material`, `set_layers`, `set_gravity_scale`, `step(dt)`,
  `events()`, `manifolds()`, `stats()`. `Simulation` ist `Clone`: eine Kopie ist ein Schnappschuss, der zu denselben Bits
  weiterrechnet (die Impulse für den nächsten Schritt und die schlafenden Inseln eingeschlossen) – für einen Client, der
  zurückspult und neu vorhersagt (G7e).
- **Masse**: `Shape::mass_properties(dichte) -> Option<MassProps { mass, center, inertia }>`, `MassProps::new`, `point`,
  `transformed`, `inertia_about` (Steiner), `scaled`, `is_valid`. Neu in `math`: an `Mat3` `inverse`, `transpose`, `mul`, `add`,
  `sub`, `scale`, `determinant`, `rotated(q)` (R M Rᵀ), `outer`, `skew`, `trace`, `diagonal`, `from_cols`, `at`; an `Quat`
  `from_scaled_axis` (genau: Winkel = Betrag), `to_scaled_axis`, `dot`, `is_finite`.
- **Gelenke**: `add_joint(a, b, rahmen, kind)` (der Rahmen in Weltkoordinaten bei den Lagen von jetzt; sein Ursprung ist der
  Anker, seine X-Achse die Achse), `add_distance_joint(a, b, pa, pb, kind)`, `add_joint_local(Joint { a, b, frame_a,
  frame_b, kind, collide_connected, user })`, `remove_joint`, `joint`, `set_joint(id, kind, collide_connected)`, `joints()`,
  `joint_position` (Winkel eines Scharniers, Lage eines Schiebers, Länge eines Abstandsgelenks, Verdrehung eines
  Kugelgelenks), `joint_force` (Kraft und Moment auf b im letzten Teilschritt). `JointKind::{Ball { cone, twist }, Hinge {
  limits, motor, spring }, Fixed, Slider { limits, motor, spring }, Distance { min, max, spring }}`, `JointKind::{BALL, HINGE,
  SLIDER, rod(l), rope(l)}`, `Motor { speed, max_force }`, `Spring { target, hertz, damping_ratio }`, `JointError::{DeadBody,
  SameBody, NothingMoves, NotFinite, Range}`. Ein Gelenk an der Welt hängt an einem statischen Körper (eine kleine Kugel mit
  `with_layers(0, 0)`, die mit nichts stößt).
- **Ereignisse**: `ContactEvent::{Begin { a, b, point, normal, speed }, End { a, b }, SensorBegin { sensor, other },
  SensorEnd { sensor, other }}`; `speed` ist die Annäherung entlang der Normale vor dem Stoß (für die Lautstärke). Ein
  entfernter Körper gibt `End` mit seiner toten Id.
- **Parameter** (`SimParams`): Schwerkraft, 4 Teilschritte, Kontakte 30 Hz mit Dämpfung 10, Gelenke 60 Hz mit Dämpfung 2,
  höchstens 3 m/s Herausdrücken, Restitution ab 1 m/s, 5 mm Schlupf, 2 cm spekulativer Abstand, höchstens 1000 m/s, Schlaf
  nach 0,5 s unter 0,05 m/s, Sweeps an.

### Ein Schritt

1. Kinematische Ziele werden zu Geschwindigkeiten.
2. **Paare**: Jeder wache Körper fragt die Bäume der Welt mit seiner Box, vergrößert um den spekulativen Abstand und um das,
   was er im Schritt zurücklegen kann (|v| dt + |ω| dt · Radius, höchstens sein Radius). Statische Körper suchen nie selbst.
   Ebenen, Sensoren und Gelenke ohne `collide_connected` werden hier gefiltert.
3. **Manifolds** ([Kontaktpunkte](#kontaktpunkte)): bis zu vier Punkte je Normale, gegen ein Netz je Dreieck.
4. **Warmstart**: Jeder neue Punkt übernimmt die Impulse des alten Punkts desselben Paars, der in a’s Rahmen höchstens 2 cm
   (vier Schlupf) entfernt liegt und dessen Normale um weniger als 18° abweicht; die Reibung als Vektor, auf die neue
   Tangentialebene projiziert.
5. **Wecken**: Berührt ein Paar einen schlafenden Körper, wacht dessen Insel auf und sucht im selben Schritt ihre Paare.
6. **Teilschritte** (h = dt / 4), nach dem Soft Step von Box2D v3: Geschwindigkeiten (Schwerkraft, Kräfte, Dämpfung,
   Kreiselkräfte), Warmstart (Gelenke, dann Kontakte), Lösen mit Bias (Gelenke, dann Kontakte), Geschwindigkeiten begrenzen,
   Positionen, Lösen ohne Bias („relax“). Danach einmal Restitution.
7. **Neue Lagen**: Schwerpunkt und Drehung aus den Teilschritten, kinematische Körper genau auf ihr Ziel.
8. **Sweep** eines Körpers, der weiter kam, als seine spekulativen Kontakte reichten: ein Cast gegen das, was sich nicht
   bewegt (ein `bullet` auch gegen Dynamisches), Halt 5 mm vor der Fläche entlang ihrer Normale. Dann ein Strahl des
   Schwerpunkts gegen dasselbe: Der Schwerpunkt geht nie durch eine Fläche.
9. **Schlaf**: Inseln aus Kontakten und Gelenken (Union-Find über die wachen dynamischen Körper); eine Insel, deren Körper
   alle 0,5 s lang an jedem Punkt langsamer als 0,05 m/s waren, schläft. Ein Körper auf einer bewegten kinematischen Plattform
   bleibt wach.
10. **Ereignisse** aus dem Vergleich mit dem letzten Schritt; die Manifolds schlafender Paare bleiben (für den Warmstart nach
    dem Aufwachen und für `manifolds()`).

### Kontaktpunkte

GJK und EPA geben eine Normale und ein Paar nächster (oder tiefster) Punkte. Ein Quader auf einer Fläche braucht seine vier
Ecken, sonst kippelt er. Wie in Jolt wird deshalb von jeder Form die Fläche genommen, die am meisten entlang der Normale schaut:
die Seite eines Quaders, das Polygon der koplanaren Dreiecke einer Hülle (gegen den Uhrzeigersinn sortiert ohne Winkel, über
Halbebene und Kreuzprodukt), ein Dreieck eines Netzes, die Achse einer Kapsel, sonst ein Punkt. Die Fläche, die paralleler zur
Normale liegt, ist die Referenz; die andere wird an ihren Seitenebenen geschnitten (Sutherland–Hodgman, die Ebenen entlang der
Normale projiziert). Jeder geschnittene Punkt, dessen Abstand zur Referenzebene entlang der Normale höchstens der spekulative
Abstand ist, wird ein Kontakt. Mehr als vier werden auf die vier reduziert, die die größte Fläche aufspannen (der tiefste, der
fernste davon, das größte Dreieck, der größte Zuwachs). Schneiden ergibt nichts (eine Ecke, eine Kugel): der Punkt von GJK.

Gegen ein Netz entsteht ein Manifold je Dreieck, mit der Normale des Dreiecks auf einer inaktiven Kante
([Netze, Nähte und aktive Kanten](#netze-nähte-und-aktive-kanten)); Dreiecke mit gleicher Normale (innerhalb 2,6°) werden ein
Manifold. Ein Quader über der Naht zweier Bodendreiecke hat einen mit vier Punkten (Test `contact`).

Eine Kapsel gibt beide Enden ihrer Achse, außer sie steht entlang der Normale: Gab sie schräg nur das untere Ende, kippte sie in
einem Schritt mit dem oberen Ende in den Boden und ritt dann auf den zweiseitigen Dreiecken hindurch (gefunden im Haufen des
Tests `stack`: eine Kapsel von 2,6 cm Radius fiel durch das Netz).

### Entscheidungen

- **Soft Step** (Box2D v3, Catto: „Solver2D“, 2024), in 3D: Kontakte und Gelenke sind gedämpfte Federn, implizit über einen
  Teilschritt gelöst; der Bias drückt Überlappungen mit höchstens 3 m/s auseinander, ohne zu überschießen, der Relax-Durchgang
  nimmt die Geschwindigkeit des Drückens wieder heraus. Kontakte mit 30 Hz (gegen Unbewegtes 60 Hz) und Dämpfung 10, Gelenke
  mit 60 Hz und Dämpfung 2, wie in Box2D. Gemessen: Kontakte mit 60 Hz sinken viermal weniger ein, aber ein Turm aus 12
  Kisten zitterte 1 cm zur Seite und schlief nicht ein, der Haufen auch nicht; Gelenke mit 120 Hz dehnten sich im Aufprall von
  Ragdolls weniger (2,5 statt 3,8 cm), gewannen aber bei 1:100 Masse an langen Hebeln bis zum 120-fachen an Energie.
- **Spekulative Kontakte**: Ein Punkt mit Spalt erlaubt dem Paar, sich im Teilschritt genau bis zur Berührung zu nähern, starr
  (Bias = Spalt / h). Der Spalt folgt den Körpern durch die Teilschritte – an den **Punkten der Kerne**: dem Mittelpunkt einer
  Kugel, einem Punkt der Kapselachse, der Oberfläche sonst. Folgte er dem Kontaktpunkt auf der Oberfläche, öffnete eine
  rollende Kugel einen Spalt, der nicht da ist (r (1 − cos θ) je Teilschritt): ein Kügelchen mit 500 m/s, das der Sweep auf eine
  1 mm dünne Wand gesetzt hatte, drehte sich mit 200 rad/s und ging durch sie hindurch.
- **Die Reichweite** der Paare und Punkte wächst mit dem, was ein Körper im Schritt zurücklegen kann, höchstens sein Radius;
  darüber hinaus sweept der Schritt. So hält eine fallende Kiste genau am Boden, ohne erst einzusinken.
- **Sweeps** halten 5 mm vor der Fläche, entlang ihrer Normale: So beginnt der nächste Schritt mit einem Spalt, also starr.
  Ein Halt genau auf der Fläche (Spalt −2 · 10⁻¹⁶ durch Rundung) fiel in den weichen Zweig, der von 500 m/s nur 97 % nimmt; der
  Rest trug das Kügelchen durch ein Dreieck. Ein Halt zurück entlang der Bewegung bremste Körper, die eine Fläche streifen.
- **Der Strahl des Schwerpunkts**: Eine Kiste, die mit 120 m/s und 30 rad/s aufschlägt, dreht sich mit bis zu 190 rad/s, drei
  Radiant je Schritt; keine Kontaktpunkte vom Anfang des Schritts folgen dem, eine Ecke kann einen Schritt lang 15 cm im Boden
  stecken. Solange der Schwerpunkt auf der richtigen Seite bleibt, drückt der Kontakt sie heraus. Ein zweiseitiges Dreieck,
  über das der Schwerpunkt schon hinaus ist, drückte sie dagegen hindurch.
- **Kontakte gegen Unbewegtes zuletzt**, und eine Überlappung tiefer als ein Viertel der kleinsten Ausdehnung des bewegten
  Körpers starr: Eine Kugel von 2,5 cm unter einem 100 kg schweren Körper sank sonst mit der weichen Feder (Last / m_eff ·
  g / ω²) mehr als ihren Radius in das Netz und fiel hindurch.
- **Gelenke**: die Teile nacheinander, das Wichtigste zuletzt (Motor und Feder, Grenzen, gesperrte Drehungen, der Anker).
  Grenzen sind einseitig und spekulativ wie Kontakte. Von einem Fehler geht je Durchgang höchstens 0,2 m und 8° in den Bias
  (wie `maxLinearCorrection` und `maxAngularCorrection` in Box2D 2.4): Ein Gelenk, das 2 rad außerhalb seiner Grenzen
  angelegt wird, kommt zurück, statt seinen Körper mit 136 rad/s herumzureißen. Die Impulse der Gelenke gehen zu 85 % in den
  nächsten Teilschritt (Bullets `warmstartingFactor`): Ein Körper zwischen zwei Gelenken, die einander widersprechen (eine
  Kugel, an zwei schwere Körper geschweißt), sammelt große Impulse, die sich aufheben; dreht er sich, heben sie sich nicht mehr
  auf. In 24 zufälligen Szenen mit allen Gelenkarten gewann eine das 60-fache an Energie, mit 85 % keine (mit 80 % hingen die
  Ketten 7 mm durch).
- **Geschwindigkeiten** werden nach dem Lösen noch einmal begrenzt (1000 m/s, eine Achteldrehung je Teilschritt), nicht nur
  vorher: Widersprüchliche Gelenke an einem leichten Körper mit langen Hebeln drehten ihn sonst mit 8000 rad/s.
- **Drehen**: `Quat::from_scaled_axis(ω h)` ist genau (eine Drehung mit fester Rate dreht um Rate · Zeit, Test `motion`); der
  Trägheitstensor in Weltkoordinaten bleibt über den Schritt der vom Anfang (ihn je Teilschritt neu zu drehen machte die
  Zufallsszenen schlechter, nicht besser). **Kreiselkräfte** (`gyroscopic()`) nach Jolt: ein expliziter Schritt des Drehimpulses
  im Körperrahmen, auf seinen Betrag zurückgesetzt. Bullets impliziter Schritt verlor in 20 s 8,6 % des Drehimpulses, Jolts
  weniger als 2 %; die Kiste dreht sich um ihre mittlere Achse wie im All um (Dschanibekow-Effekt).
- **Rollwiderstand** (wie in Box2D 3.1): Die gegenseitige Drehung wird mit höchstens c × Normalkraft × kleinster Radius
  gebremst. Ohne ihn rollt eine Kugel, gebremst nur von der Dämpfung, eine Minute weiter.
- **Massen**: Kugel, Kapsel (Zylinder und Halbkugeln, deren Schwerpunkt 3r/8 jenseits des Zylinders liegt) und Quader
  geschlossen; eine Hülle über die Tetraeder zwischen einem inneren Punkt und ihren Dreiecken (das zweite Moment eines
  Tetraeders mit einer Ecke im Ursprung ist det / 120 · (a aᵀ + b bᵀ + c cᵀ + s sᵀ), s = a + b + c), dann Steiner zum
  Schwerpunkt.
- **Bitgleich** wie die Abfragen: nur `+ − × ÷`, korrekt gerundete Wurzeln und `rt`s portables `sin`, `cos`, `atan2`; alles in
  der Reihenfolge der Slots, keine ungeordneten Container, keine Threads. Gelöst wird in der Reihenfolge der Paare (a, b), über
  einen sortierten Index: in der Reihenfolge ihres Entstehens kamen die Kontakte von Inseln, die mitten in der Suche aufwachen,
  zuletzt, und der Haufen des Tests `stack` schlief nie ein (in der Reihenfolge der Paare nach 367 Schritten).

### Tests der Starrkörper

| Gruppe | Prüft | Ergebnis |
| --- | --- | --- |
| `mass` | Kugel und Quader gegen die geschlossene Form (10⁻¹⁴), Kapseln gegen Scheiben mit Simpson (400 000 Scheiben, 10⁻⁶), 200 Hüllen aus den Ecken gedrehter und verschobener Quader gegen den Quader (10⁻⁹), 500 Tetraeder gegen die Formeln von Tonon (2004, 10⁻⁸), ein Polyeder aus 3200 Punkten gegen die Kugel (1 %), Steiner, was verweigert wird | 904 Prüfungen, 0 falsch |
| `motion` | freier Fall und Wurf gegen die genaue Summe des Integrators (y = y₀ + v₀ h k + g h² k (k+1)/2 nach k Teilschritten; 10⁻⁹), eine feste Drehung genau, Dämpfung genau, Kräfte, Impulse, ein kinematischer Körper genau auf seinem Ziel, Schwerkraftfaktor 0, Kreiselkräfte (Drehimpuls auf 2 %, die Kiste schlägt um; ohne sie bleibt ω) | 24, 0 falsch |
| `contact` | Kugel, Quader, Kapsel kommen auf dem Boden zur Ruhe (höchstens 6 mm darin) und schlafen ein; Restitution 0,5, 0,8, 1 (Steighöhe e² h auf 4–5 %); Reibung auf 20°: 0,6 hält (unter 5 mm in 1,5 s), 0,2 und 0 gleiten mit g (sin − μ cos) (3 %); eine Kugel rollt mit 5/7 g sin (2 %); Rollwiderstand bremst mit 5/7 c g, ohne ihn rollt sie weiter; Kugeln stoßen frontal (Impuls auf 10⁻⁹, elastisch tauschen sie, plastisch gemeinsam); ein Quader auf einem Netz: ein Manifold mit vier Punkten über die Nähte; eine kinematische Plattform trägt und hebt eine Kiste; Massen 1:10 (mm) und 1:100 (bleibt oben) | 26, 0 falsch |
| `stack` | Türme aus 5 und 12 Kisten stehen (Spitze 0,1 und 5,6 mm zur Seite) und schlafen ein, eine Pyramide aus 12 Reihen (78 Kisten), ein Haufen aus 120 Formen aller Arten mit Rollwiderstand in einer Grube auf einem Netz: nichts verlässt sie oder steckt im Boden, alles schläft ein (nach 367 Schritten) | 127, 0 falsch |
| `joint` | Pendel (Periode gegen 2π √(I/(m g L)) mit der Reihe in der Amplitude, 5° und 60°, 0,5 %), die Kraft des Gelenks (m g auf 1 %), eine Kette aus 20 Gliedern (im Schwung 1,7 cm, in Ruhe 4,9 mm), eine Tür in ihren Grenzen (2°), Motoren (Drehzahl auf 1 %, ein schwacher auf sein Moment), Federn an Scharnier, Schieber und Abstand (Periode 3 %), Schweißen im Sturz (1 cm, 1,5°), ein Kragbalken, ein Schieber an seiner Grenze und mit Motor, Seil und Stange, Kugelgelenk mit Kegel und Verdrehung (3°), acht Ragdolls auf einem Boden mit Kisten (im Aufprall 3,5 cm und 10,9°, in Ruhe 0,9 mm und 0,9°), Gelenke außerhalb ihrer Grenzen angelegt, was verweigert wird | 35, 0 falsch |
| `ccd` | ein Kügelchen von 1 cm mit 500 m/s gegen eine 1 mm dünne Wand und gegen ein Netz (ohne Sweep geht es durch, die Gegenprobe), ein `bullet` gegen eine leichte Kiste (sie bewegt sich; ohne `bullet` geht es durch), 200 Formen aus bis zu 20 m mit 50 bis 150 m/s und 30 rad/s auf den Boden geworfen: kein Schwerpunkt im Boden, nach 4 s keiner mehr als 2 cm darin | 7, 0 falsch |
| `events` | ein `Begin` beim Aufprall mit der Geschwindigkeit √(2 g h) (5 %), keines in Ruhe, `End` beim Abheben und beim Entfernen; ein Sensor meldet Eintritt und Austritt und bremst nicht (die Fallgeschwindigkeit bitgenau), auch für einen kinematischen Körper | 8, 0 falsch |
| `dynfuzz` | 24 zufällige Szenen (Kiste oder Netz als Boden, 50 Körper aller Formen, Dichten 50 bis 20 000, bis 15 m/s und 20 rad/s, Gelenke aller Arten mit Grenzen, Motoren, langen Hebeln, auch mehrere an einem Körper): nichts nicht Endliches, kein Schwerpunkt im Boden, keine Energie außer der, die Motoren geben können (2 %); dieselbe Szene zweimal Bit für Bit gleich, eine Kopie rechnet gleich weiter; Hinzufügen und Entfernen im Lauf; was verweigert wird | 140, 0 falsch |

Wo eine Grenze im Test steht, ist sie gemessen und begründet; was die weichen Federn kosten, steht unter
[Grenzen der Starrkörper](#grenzen-der-starrkörper).

### Prüfsumme der Starrkörper

Die dritte Zeile von `physicstest --checksum` (`physicstest dynamics checksum=`) rechnet die Szene `showcase` 600 Schritte
lang: die Karte der Tests, ein Haufen aus 80 Formen mit Rollwiderstand, eine Pyramide aus 6 Reihen auf einem Sockel, zwei
Ragdolls, eine Kette aus 12 Gliedern, ein Rad mit Motor, ein Schieber mit Feder, ein Ball an einem Seil, eine kinematische
Plattform mit vier Kisten, ein Sensor. Je Schritt und Körper Lage, Drehung, beide Geschwindigkeiten und ob er wach ist, jedes
Ereignis, alle 10 Schritte Lage und Kraft jedes Gelenks und die Zähler: 1 277 776 Werte.

| Ziel | Build | Starrkörper (U18) | Abfragen, Controller |
| --- | --- | --- | --- |
| Linux x86-64 (statisch ohne libc) | Entwicklung und Release (fat LTO) | `ee4c2de351b53085` | unverändert `0c7267a7636aaf7b`, `13dd288c669c6eaa` |
| Windows x86-64 unter Wine | Entwicklung und Release | `ee4c2de351b53085` | unverändert |
| WebAssembly in Node 22 (V8): einfach und SIMD | Entwicklung | `ee4c2de351b53085` | unverändert |

`--check` bestand auf allen drei mit 2 049 008 Prüfungen. Nicht gelaufen: macOS (kein Mac in dieser Sitzung), Android
(kein Emulator), ARM64 unter `qemu-aarch64` (nicht installiert), der Threads-Build für den Browser (braucht `rust-src`).

### Messwerte der Starrkörper

`physicstest --bench dynamics --runs 3` im Release-Build unter Linux, ein Kern eines i7-1165G7 (der Rechner wird von anderen
Sitzungen mitbenutzt, die Werte schwanken um 10 bis 20 %), Mediane. Die Szene `showcase` in drei Größen: die erste Sekunde (alles
fällt und stößt, der teure Teil) und die vier danach.

| Szene | Körper | ms je Schritt, 1. Sekunde | ms je Schritt, danach | nach 5 s wach | Manifolds / Punkte |
| --- | ---: | ---: | ---: | ---: | ---: |
| 100 Formen, Pyramide aus 8 Reihen, 4 Ragdolls | 207 | 0,62 | 0,88 | 60 | 381 / 994 |
| 300 Formen, 12 Reihen, 8 Ragdolls | 493 | 1,7 | 2,9 | 207 | 1042 / 2664 |
| 1000 Formen, 16 Reihen, 16 Ragdolls | 1339 | 6,0 | 13,0 | 762 | 3123 / 7201 |
| eine Pyramide aus 10 Reihen (nie schlafend) | 55 | 0,51 | | | |
| eine Pyramide aus 20 Reihen (nie schlafend) | 210 | 2,3 | | | |

Bei 60 Schritten je Sekunde sind das für ein paar hundert Körper wenige Prozent eines Kerns. Ein Profil (callgrind) des
Benchmarks vor den letzten Umbauten: Kopien (`memcpy`) 27 %, das Lösen der Kontakte 19 % (darin die Impulse auf die Körper,
Matrix mal Vektor, 8 %), GJK und EPA der Manifolds 17 %, die Suche der Paare in den Bäumen 14 %. Die Umbauten brachten 15 bis
20 %: die Löserkörper werden an Ort und Stelle gelöst statt kopiert (184 Byte je Körper, acht Durchgänge je Teilschritt), die
Constraints (je rund ein Kilobyte) entstehen an Ort und Stelle und werden über einen sortierten Index gefunden statt
umsortiert. Offen sind die Kopien des Simplex in GJK (rund 6 %); GJK gehört auch den Abfragen, ein Umbau dort ist ein eigener Schritt.

### Grenzen der Starrkörper

- **Weiche Kontakte sacken unter Last**: um Last / m_eff · g / ω², bei 30 Hz 0,28 mm je Vielfaches der eigenen Masse. Ein
  Turm aus 12 Kisten ist an der Spitze 3,6 cm niedriger, als er gestapelt wurde (5 Kisten: 5,5 mm); ein schwerer Körper auf
  einem 100-mal leichteren drückt sich 4 cm in ihn, der leichte 1 cm in den Boden (bei 1:10 sind es Millimeter). Wie in Box2D:
  Mehr Teilschritte helfen erst zusammen mit einer höheren Frequenz, und 60 Hz zittern (oben).
- **Weiche Gelenke geben nach**: Eine Kette aus 20 Gliedern geht im Schwung bis 1,7 cm auseinander, in Ruhe 4,9 mm; ein
  Ragdoll, der aus bis zu 4 m aufschlägt, für ein paar Schritte bis 3,5 cm und 11° über eine Grenze (der Boden wird zuletzt
  gelöst und drückt das Glied über den Kegel); in Ruhe halten sie auf 1 mm und 1°.
- **Drehung im Sweep fehlt**: Sweeps verschieben, sie drehen nicht; eine schnell drehende Ecke kann einen Schritt lang im
  Boden stecken, der Schwerpunkt nie.
- **Kontaktereignisse** nur `Begin` und `End` je Paar (mit der Annäherung); kein Rückruf, der einen Kontakt ändert oder
  verwirft (Plattformen, durch die man von unten springt), keine Kräfte je Punkt außer über `manifolds()`.
- Nicht gebaut: mehrere Formen je Körper, dynamische Netze und konvexe Zerlegung, ein elliptischer Kegel und ein Gelenk mit
  sechs frei wählbaren Achsen, Zahnräder und Riemen, Fahrzeuge, weiche Körper und Stoff, Auftrieb, Threads (Inseln oder
  Graphfärbung), SIMD. Der Character-Controller schiebt keine Körper (das Spiel gibt ihnen Impulse) und folgt keiner
  bewegten Plattform.
