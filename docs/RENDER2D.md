# RENDER2D -- der Kern des 2D-Renderers für CAD

`lib/render2d` zeichnet Zeichnungen aus sehr vielen Linien über [`lib/gpu`](GPU.md): eine Szene, die bleibt (Entitäten mit festen Ids, Ebenen, Stile), von der nur hochgeht, was sich geändert hat; Striche als Vierecke, deren Deckung der Fragment-Shader aus der Geometrie rechnet (kein MSAA), mit genauen Kappen und Ecken; Koordinaten in `f64`, auf der GPU ein Paar `f32` je Achse, von dem zuerst die Kamera abgezogen wird; Sichtbarkeit in Rechen-Shadern, die die Reihenfolge des Malers halten und die indirekten Zeichenbefehle schreiben. Wie `render3d` ist sie der Inhalt eines `ui.gpu_view`: Das Programm klebt sie mit einem `GpuContent` an, `lib/ui` kennt sie nicht. Dieselben WGSL-Quellen für Vulkan Voll, Vulkan Basis und WebGPU (im Browser noch nicht gelaufen, unten).

Stand: G6a1, der Kern. Linien und Polylinien (offen, geschlossen), Ebenen, Stile mit Breite in Pixeln oder Zeichnungseinheiten, Kappen (stumpf, rund, quadratisch), Ecken (Gehrung mit Grenze, rund, abgeschrägt), Durchscheinendes ohne doppeltes Mischen an Ecken, Kamera mit Mitte, Maßstab und Drehung, Sichtbarkeit auf der GPU für Basis (ein indirekter Befehl je Ebene) und Voll (ein Multi-Draw). Noch nicht: Bögen, Kreise, Strichmuster (G6a2), Kurven, Flächen, Schraffuren, Blöcke (G6b), Text (G6c), Auswahl (G6d), CPU-Weg, DXF, andere Plattformen (G6e).

| Teil | Wo |
| --- | --- |
| `Renderer`, Pipelines, Aufzeichnen | [`lib/render2d/lib.rs`](../lib/render2d/lib.rs) |
| `Scene`, Entitäten, Ebenen, Stile, Kamera, Hochladen | [`lib/render2d/scene.rs`](../lib/render2d/scene.rs) |
| Shader | [`common.wgsl`](../lib/render2d/common.wgsl) (Ansicht, Records, Koordinaten), [`cull.wgsl`](../lib/render2d/cull.wgsl), [`scatter.wgsl`](../lib/render2d/scatter.wgsl), [`stroke.wgsl`](../lib/render2d/stroke.wgsl), vorübersetzt in [`shaders_spv.rs`](../lib/render2d/shaders_spv.rs) |
| Tests, Demo, Messung | [`apps/render2dtest`](../apps/render2dtest/main.rs); die Referenz über `lib/gfx`: [`refimg.rs`](../apps/render2dtest/refimg.rs); Szenen: [`scenes.rs`](../apps/render2dtest/scenes.rs); im Fenster: [`demo.rs`](../apps/render2dtest/demo.rs); [`bench.rs`](../apps/render2dtest/bench.rs) |

## Ein Beispiel

```rust
let mut r = render2d::Renderer::new(&dev, &render2d::RendererDesc::new(gpu::Format::Bgra8Unorm))?;
let mut scene = render2d::Scene::new(&r)?;
let kontur = scene.add_layer(&render2d::Layer::new(0))?;
let mitte = scene.add_layer(&render2d::Layer::new(1))?;           // über `kontur`

let mut dick = render2d::Style::new(0xFF20_2020);                  // 0xAARRGGBB, sRGB
dick.width = 0.5;                                                  // halbe Zeichnungseinheit
dick.unit = render2d::Unit::Drawing;
dick.cap = render2d::Cap::Round;
dick.join = render2d::Join::Round;
let dick = scene.add_style(&dick)?;
let duenn = scene.add_style(&render2d::Style::new(0x80D0_2020))?;  // Haarlinie, halb durchscheinend

let teil = scene.insert(kontur, dick, &render2d::Shape::Polyline { points: &umriss, closed: true })?;
scene.insert(mitte, duenn, &render2d::Shape::Line { a: [1e7, 0.0], b: [1e7 + 40.0, 0.0] })?;
scene.set_camera(&render2d::Camera { center: [1e7 + 20.0, 0.0], scale: 0.05, rotation: 0.0 })?;  // 20 px je Einheit

// je Bild
scene.prepare(&mut r, (breite, hoehe))?;      // wartet auf das letzte Bild dieses Platzes, schickt, was sich änderte
let mut e = dev.encoder()?;
r.record(&mut e, &scene, farbe)?;             // ein Durchgang ohne Tiefe in `farbe`, Größe wie bei prepare
let id = dev.queue().submit([e.finish()?])?;  // oder swapchain.present(frame, [e.finish()?])
scene.submitted(id);

scene.update(teil, &render2d::Shape::Polyline { points: &neu, closed: true })?;   // behält seinen Platz in der Reihenfolge
```

## API-Überblick

- **Renderer**: `Renderer::new(&device, &RendererDesc::new(farbformat))` (`#[non_exhaustive]`, Felder danach einzeln ändern: `frames_in_flight`, 1 bis 3, Vorgabe 2); ein Format mit sRGB, Tiefe oder Kompression verweigert er (`Invalid`: [Farben](#farben)). `record(&mut encoder, &scene, farbe)`, `device()`, `desc()`, `multi_draw()` (Voll mit Multi-Draw und `first_instance` im indirekten Befehl), `max_records()`.
- **Scene**: `Scene::new(&renderer)`; Stile `add_style(&Style) -> StyleId`, `set_style`, `style`; Ebenen `add_layer(&Layer) -> LayerId`, `set_layer`, `layer`, `remove_layer` (nimmt ihre Entitäten mit), `drawn_layers()` (die sichtbaren in Zeichenreihenfolge); Entitäten `insert(ebene, stil, &Shape) -> EntityId`, `update(id, &Shape)`, `set_entity_style(id, stil)`, `remove(id)`, `len`, `is_empty`, `locate(id)` (erster Platz und Zahl seiner Records im Pool); `set_camera(&Camera)`, `camera()`; je Bild `prepare(&mut renderer, (b, h))`, `submitted(id)`, `wait_slot()`.
- **Werte**: `Camera { center: [f64; 2], scale, rotation }` (`scale` Zeichnungseinheiten je Pixel, `rotation` im Bogenmaß gegen den Uhrzeigersinn, y zeigt auf dem Bildschirm nach oben; `Camera::new(mitte, scale)`, `to_screen`, `to_drawing`, `pan(dx, dy)` in Pixeln, `zoom_at(punkt, faktor, größe)`: der Punkt unter dem Zeiger bleibt). `Style { color, width, unit, cap, join, miter_limit }` (`Style::new(farbe)`: Haarlinie, `Unit::Px`, `Cap::Butt`, `Join::Miter`, Grenze 4). `Unit::{Px, Drawing}`, `Cap::{Butt, Round, Square}`, `Join::{Miter, Round, Bevel}`, `Layer { visible, order }` (`Layer::new(order)`), `Shape::{Line { a, b }, Polyline { points, closed }}`. Typen, die wachsen werden, sind `#[non_exhaustive]`.
- **Zum Messen und Testen**: `set_stats(true)`, `take_stats()`, `collect_stats()` (`FrameStats`: sichtbare Records, vier Zeitstempel: Anfang, nach dem Hochladen, nach der Sichtbarkeit, nach dem Zeichnen), `upload_info()` (`UploadInfo`: was `prepare` geschickt hat: Records, Läufe, Scatter oder Kopien, neuer Pool, kopierte Bytes, Tabellen), `set_culling(false)`, `lists_buffer()`, `visible_buffer()`, `draws_buffer()`, `pool_buffer()`; die Shader als `COUNT_SPV` … `FS_SPV` und `CULL_WGSL`, `SCATTER_WGSL`, `STROKE_WGSL`.

Reihenfolge: Ebenen nach `Layer::order`, bei gleicher nach dem Hinzufügen (ein `set_layer` mit neuer `order` legt die Ebene obenauf in dieser Ordnung), in einer Ebene die Reihenfolge des Einfügens; ein `update` behält den Platz. Breite 0 ist eine Haarlinie von 1 Pixel; schmaler als 1 Pixel zeichnet sie nie (auch nicht in Zeichnungseinheiten beim Herauszoomen). Fehler sind die von `lib/gpu`: `Invalid` für tote Ids, Stile, die die Szene nicht hat, nicht endliche oder zu große Punkte (ab `1e30`), Formen ohne Punkt, negative oder nicht endliche Breiten, eine Gehrungsgrenze unter 1, eine Kamera ohne endliche Mitte oder mit Maßstab ≤ 0, `record` vor `prepare`, nach einer Änderung seit `prepare`, mit einem Ziel anderen Formats oder anderer Größe, Szenen eines anderen Renderers; `Unsupported`, wenn die Records die Bindungen oder einen Dispatch des Geräts übersteigen oder mehr als 65 535 Ebenen sichtbar sind. Ein Punkt allein ist ein Segment der Länge 0: runde und quadratische Kappen zeigen ihn.

## Aufbau

**Daten.**

| Was | Wie groß | Wer liest |
| --- | --- | --- |
| Pool (Records) | 64 Byte je Segment: Anfang und Ende als (hi.x, hi.y, lo.x, lo.y), die Vektoren zum vorigen und nächsten Punkt (`f32`, (0, 0): keiner), Stil + 1 (0: frei), der eigene Vektor als `f32`-Bits, wo die Polylinie endet (Bit 1: der vorige Punkt ist ihr erster, Bit 2: der nächste ihr letzter), Art | `count`, `write`, Vertex-Shader |
| Stile | 32 Byte: Farbe, Breite, Einheit, Kappe, Ecke, Gehrungsgrenze | `count`, Vertex-Shader |
| Ansicht | 64 Byte: Mitte der Kamera als hi/lo, was hi + lo von ihr nicht halten, 1 / Maßstab, Maßstab, cos und sin der Drehung, Größe des Ziels, Culling aus | alle |
| `order`, `layers` | je Seite der sichtbaren Ebenen ihre Nummer; je sichtbare Ebene ihre erste Seite und ihre Seiten | `count`, `scan` |
| `masks`, `counts`, `offsets` | je Seite 8 Wörter (ein Bit je Record), ihre Zahl, ihr erster Platz | `count` schreibt, `scan`, `write` |
| `lists`, `draws`, `visible` | je sichtbare Ebene erster Platz und Zahl, ihr indirekter Befehl (20 Byte); 4 Byte je sichtbares Record | `scan`, `write` schreiben, Vertex-Shader und Grafik-Pipeline lesen |
| Index-Puffer (je Renderer) | 16 384 Vierecke, `u16`, je Viereck 0 1 2 2 1 3 | Grafik-Pipeline |

Kein Vertex-Puffer: Der Vertex-Shader holt sein Record über die sichtbare Liste (der Platz aus der Instanz und der Nummer der Ecke, die Ebene aus den oberen 16 Bit der Instanz oder dem Push-Wert). Jede Ebene besitzt einen Bereich des Pools aus ganzen Seiten zu 256 Records; die Records ihrer Entitäten liegen dort in der Reihenfolge des Einfügens, die einer Entität beieinander. Die Seiten der sichtbaren Ebenen in ihrer Zeichenreihenfolge sind also schon die Reihenfolge des Malers.

**Ändern.** `insert` hängt an das Ende seiner Ebene. `update` schreibt an Ort und Stelle, wenn die Records in den Platz der Entität passen (wächst sie und ist sie die letzte ihrer Ebene, wächst ihr Platz mit); sonst wird die Ebene neu gelegt, mit dem Doppelten an Platz für sie. `remove` lässt freie Records (Nullen) zurück; hat eine Ebene mehr als 1024 freie und mehr freie als lebende, wird sie ohne sie neu gelegt. Ist ein Bereich zu klein, eine Ebene neu gelegt, hinzugekommen oder entfernt, gibt es einen neuen Pool: Die GPU kopiert, was bleibt, aus dem alten (keine Kopie innerhalb eines Puffers: WebGPU verbietet sie), von der CPU kommen nur die Records geänderter Entitäten. Die geänderten Records gehen in Läufen hoch (bis 256 Läufe: je ein Kopierbefehl aus dem Upload-Puffer) oder über `scatter` (darüber: ein Dispatch, 80 Byte je Record). Der Test `changes` prüft nach jeder Änderungsrunde, dass das Bild Byte für Byte dem einer neu aufgebauten Szene gleicht.

**Ein Bild** (`Renderer::record`):

| Schritt | Was geschieht |
| --- | --- |
| Barriere | gegen alle Zugriffe des Bildes davor (Kopien, Rechen-Shader, indirekte Befehle, Vertex-Shader) |
| neuer Pool (selten) | löschen, Barriere, was bleibt aus dem alten kopieren, Barriere |
| Kopien, `scatter` | Ansicht, geänderte Stile, Tabellen der Ebenen, geänderte Records |
| Barriere | Hochgeladenes vor Sichtbarkeit und Vertex-Shader |
| `count`, Barriere, `scan`, Barriere, `write` | die sichtbaren Listen ([Sichtbarkeit](#sichtbarkeit)) |
| Barriere | Listen, Befehle und sichtbare Liste vor den indirekten Befehlen und dem Vertex-Shader |
| Durchgang | ein Durchgang ohne Tiefe, Mischen mit vormultipliziertem Alpha; Voll: ein `draw_indexed_indirect_multi` über alle sichtbaren Ebenen; Basis: je Ebene ein Push-Wert und ein `draw_indexed_indirect` |

Die Barrieren stehen in [`lib.rs`](../lib/render2d/lib.rs) in `Renderer::record`; die Synchronisationsprüfung der Validierungsschicht meldet für alle Tests 0 Fehler und 0 Warnungen.

## Der Shader

**Vertex-Shader.** Er legt sein Segment relativ zur Kamera in Pixel ([Koordinaten](#koordinaten)) und schneidet es auf das Ziel plus seine Reichweite zu (Liang-Barsky); ein Ende jenseits davon ist offen: keine Kappe, keine Ecke, davon zeigt sich ohnehin nichts. Das Viereck deckt das Rechteck des Segments, an jedem Ende so weit, wie seine Kappe oder Ecke darüber hinausreicht (stumpf und abgehende Ecke: 0, rund und quadratisch: die halbe Breite, ankommende Gehrung: die halbe Breite mal max(sin φ, tan(φ/2)) bis zur Spitze), und ein Pixel mehr für die geglättete Kante. Die halbe Breite ist `max(Breite / 2, 0,5)` Pixel (Breiten in Zeichnungseinheiten durch den Maßstab).

**Fragment-Shader: Deckung, genau.** Der Anteil des Pixels (ein Quadrat der Seite 1) im Strich ist die Fläche des Quadrats in konvexen Stücken aus Geraden. Eine Kante allein: der Anteil der Halbebene, `cov`, der Kastenfilter einer geraden Kante unter jedem Winkel, in geschlossener Form. Die zwei parallelen Kanten des Streifens allein: `cov(links) + cov(rechts) - 1`, auch für Haarlinien genau. Mehr Kanten: `area6`, die Fläche des Quadrats in bis zu sechs Halbebenen nach dem Satz von Green (die Randstücke des Quadrats und der Geraden, jedes auf alle anderen gekürzt, die Summe der Kreuzprodukte), von einem Skript ausgeschrieben, ohne Schleifen und Felder: Mit Feldern lagen sie im lokalen Speicher, und der Shader war hundertmal langsamer. Ein Kreis wird an der Stelle des Pixels durch seine Tangente ersetzt (der Fehler ist quadratisch in der Pixelgröße über dem Radius). Die meisten Pixel liegen im Rumpf: der Streifen allein, kein Ende nahe, zwei `cov`.

**Kappen.** Stumpf: die Gerade im rechten Winkel durch das Ende. Quadratisch: dieselbe eine halbe Breite weiter. Rund: das Rechteck endet am Ende, der Halbkreis dahinter ist ein eigenes Stück; beide berühren sich nur an der Geraden, die Anteile addieren sich also genau.

**Ecken: Kacheln.** Nahe einer Ecke wird der Strich an der Winkelhalbierenden zerschnitten: der Streifen dieses Segments auf seiner Seite der Halbierenden, der des Nachbarn auf der anderen; eine Abschrägung schneidet beide mit ihrer Geraden; eine runde Ecke endet in beiden Kacheln an der Geraden im rechten Winkel durch die Ecke, der Kreissektor dahinter ist ein Stück für sich (nur der Sektor außen, wie bei SVG und `lib/gfx`, kein voller Kreis, der hinter stumpfe Enden ragte). Die Kacheln überlappen nicht und füllen den Strich, solange die Sehne der Halbierenden (die halbe Breite mal tan(φ/2) entlang jedes Segments) in beiden Segmenten liegt; dann addieren sich ihre Anteile genau, und außen ergibt sich die Gehrungsspitze, die abgeschrägte Ecke oder der Bogen von selbst. Eine Gehrung über ihrer Grenze wird zur Abschrägung (Grenze wie SVG: Länge der Spitze durch die Breite).

**Scharfe Ecken an kurzen Segmenten.** Ist die Sehne länger als ein Segment, nimmt der Shader die Vereinigung: die zwei Rechtecke, das Stück der Ecke jenseits beider Enden (Gehrungsviereck, Dreieck, Sektor; es berührt keines), `|a| + |w| + |b| - |a ∩ b|`. Liegt ein kurzes Segment nahe an beiden Ecken, sind es beide Vereinigungen; ausgelassen wird dann nur, wo sich die Rechtecke der zwei Nachbarn gegenseitig treffen, und der Schnitt `a ∩ b` ohne die fernen Enden (`area6` hat sechs Halbebenen). Das ist die einzige Näherung; sie trifft Polylinien, die auf wenigen Pixeln zweimal scharf umkehren.

**Eine Stelle für `area6`.** Die Stücke nahe eines Pixels sind Bits einer Maske (`S_*` in `stroke.wgsl`), und eine Schleife addiert (oder zieht ab) ihre Anteile; `area6` steht im Shader einmal. Vorher stand es, nach dem Einsetzen der Funktionen, an rund vierzig Stellen, und die GPU verbrachte ihre Zeit mit dem Laden von Code: in der kleinen Messung (20 000 Segmente auf 320 × 180) 6,5 ms statt 1,3 ms für Haarlinien und 17 ms statt 2,4 ms für 3 px mit runden Ecken; derselbe Shader nur mit dem Rumpf brauchte 0,05 ms.

## Überlappung

Ein durchscheinender Strich darf an seinen Ecken nicht zweimal gemischt werden, und das ohne Stencil und ohne MSAA. Jedes Pixel nahe einer Ecke liegt in den Vierecken beider Segmente. Der Trick: **Ein Pixel nahe einer Ecke gehört dem näheren der beiden Segmente** (Abstand zur Mittellinie als Strecke; nahe der Ecke ist die Grenze die Winkelhalbierende, hinter dem Ende eines kurzen Nachbarn die längere Strecke), bei Gleichstand dem ankommenden; das andere verwirft es. Das besitzende Segment rechnet den Anteil beider Kacheln (oder der Vereinigung) und mischt einmal. Damit genau eines zeichnet, müssen beide dieselben Abstände Bit für Bit rechnen: Die Ecke kommt bei beiden aus denselben hi/lo-Werten, und die Vektoren rundet die CPU einmal (`f64`-Differenz, dann `f32`): Der eigene Vektor eines Segments ist genau der negierte Vektor zum vorigen Punkt des nächsten Records und der Vektor zum nächsten Punkt des vorigen (`Record.info`). Der Code: die Abfragen am Anfang von `fs` in [`stroke.wgsl`](../lib/render2d/stroke.wgsl).

Wo eine Polylinie sich fern einer Ecke selbst kreuzt, mischen ihre Stücke je einmal, wie zwei Entitäten es täten (ein Strich ohne Selbstüberlappung bräuchte dort Stencil oder eine Tiefe; nicht gebaut). Der Test `overlap` zeigt es an spitzen Ecken (Gehrung, Abschrägung, rund, Deckkraft 0x80 und 0x40): kein Pixel dunkler als ein einmaliges Mischen der Farbe erlaubt, gegen `lib/gfx` (dessen Füllung einmal mischt) die Tinte auf 0,04 % gleich.

## Koordinaten

Ein Punkt ist auf der CPU `f64` und auf der GPU ein Paar je Achse: `hi = f32(x)`, `lo = f32(x - hi)`. Die Mitte der Kamera kommt genauso, und was `hi + lo` von ihr nicht hält (`rest`, bei 1e7 höchstens 2⁻²⁶ einer Einheit), zieht der Shader danach ab. `rel2` in [`common.wgsl`](../lib/render2d/common.wgsl) zieht zuerst die Kamera ab: `hi - hi` und `lo - lo` je über eine genaue Zwei-Summe (Knuth), dann zusammengefasst zu einem Doppel-`f32`-Paar. Ein Punkt nahe der Kamera kommt so auf wenige 1e-15 Einheiten genau heraus, ein ferner Punkt mit den 24 Bit von `f32` seines Abstands. Liegen beide Enden eines Segments weiter als 131 072 Pixel weg (eine lange Linie durch das Bild), kommt die Lage seiner Geraden aus dem genauen Kreuzprodukt beider Enden in Doppel-`f32` (`far_foot`, mit `fma` für die Produkte), sonst wanderte die Linie.

Die GPU darf das nicht umstellen: Der NVIDIA-Treiber fasste `(a - (s - bb)) + (b - bb)` zusammen und verlor den `lo`-Teil. Deshalb werden die Vertex- und die Sichtbarkeits-Shader mit `wgsl::CompileOptions { exact: true }` übersetzt (`NoContraction` an jeder Rechnung); den Fragment-Shader braucht das nicht.

**Prüfung** (`coords`, beide Stufen): Dieselbe Zeichnung bei (1e7, 1e7), (-9 999 999,5, π · 1e6) und (9 876 543,210987, -9 999 000) gegen am Ursprung: 0 Pixel anders. Bei 1e-6 Einheiten je Pixel nahe (-9 876 543,21, 6 543 210,987), 24 Verschiebungen um 0,37 Pixel und eine Drehung, sechs Linien bis 2e7 lang: Der Schwerpunkt jeder Bildzeile liegt höchstens 0,0039 Pixel von der Stelle, an die die Linie gehört (Ziel: 1/16 = 0,0625), keine Zeile ohne Linie.

## Sichtbarkeit

Die Listen müssen die Reihenfolge des Malers halten (Ebene, dann Einfügen): Ein durchscheinender Strich vor dem darunter gezeichnet, mischte falsch. Also kein atomarer Zähler je Liste wie in `render3d`, sondern eine stabile Verdichtung in drei Durchgängen ([`cull.wgsl`](../lib/render2d/cull.wgsl)):

| Durchgang | Was |
| --- | --- |
| `count` | eine Arbeitsgruppe je Seite der sichtbaren Ebenen: jedes Record gegen das Ziel, ein Bit je Record, die Zahl der Seite |
| `scan` | eine Arbeitsgruppe: die Präfixsumme der Zahlen in Seitenreihenfolge, je sichtbare Ebene erster Platz und Zahl und ihr indirekter Befehl: 6 Indizes je Viereck, höchstens 16 384 Vierecke je Instanz (`u16`-Indizes), so viele Instanzen, wie das braucht; auf Voll trägt `first_instance` die Ebene (`ebene << 16`) |
| `write` | jedes sichtbare Record an seinen Platz: der Platz der Seite plus die Bits davor |

Der Test eines Records: der Kasten um sein Segment, geweitet um die Reichweite des Strichs (halbe Breite mal der Schranke seiner Ecke, wo ein Nachbar ist, sonst die halbe Breite; ein Pixel für die Kante) und um das, was `f32` ferne Punkte runden kann, gegen das Rechteck des Ziels mit getrennten Achsen (x, y, die Richtung des Segments und ihre Normale). Freie Records bestehen nie, ohne Culling jedes lebende. Basis bekommt die Ebene über einen Push-Wert (`indirect-first-instance` ist in WebGPU und Vulkan 1.1 nicht sicher da), Voll über `first_instance`; derselbe Shader für beide.

**Prüfung** (`visibility`, beide Stufen): vier zufällige Szenen (bis 20 000 Polylinien, gedreht, bis 1e6 Einheiten groß) gegen eine genaue Rechnung auf der CPU in `f64`: 0 fehlend, 0 zu viel, 0 doppelt, 0 außer der Reihe (Records, die die CPU auf einen Hauch an einer Kante sieht, dürfen beides). Haarlinien, die eine Kante berühren, sind drin; 0,05 Pixel dahinter sind sie es nicht. Mit und ohne Culling: 0 Pixel anders.

## Farben

Farben sind sRGB-Codes mit geradem Alpha (`0xAARRGGBB`), gemischt im Code-Raum, wie `lib/gfx` mischt (vormultipliziert, `Blend::PREMULTIPLIED`). Das Ziel hält Codes: ein Unorm-Format wie `Bgra8Unorm` der UI; ein sRGB-Format verweigert `Renderer::new`, es würde die Codes noch einmal kodieren.

## Tests

`render2dtest --test NAME` (`build_rs.py test render2dtest --target win` führt die `.uis` aus): auf jeder Stufe, die die GPU hat (RTX 2070 SUPER: Voll und Basis). Die Bilder entstehen in einem Ziel ohne Fenster und werden gegen dieselbe Zeichnung über `lib/gfx` verglichen (`dash_stroke` mit leerem Muster, nicht-null gefüllt mit genauer Flächendeckung, gemischt in sRGB-Codes).

| Test | Was | Ergebnis |
| --- | --- | --- |
| `shaders` | die eingebetteten SPIR-V-Wörter gegen `wgsl::compile` | gleich |
| `caps` | stumpf, rund, quadratisch, Breite 1 bis 15 | stumpf und quadratisch: kein Pixel über 2 Codes; rund: bis 43 Codes an Haarlinien-Kreisen (`lib/gfx` zeichnet Vielecke) |
| `joins` | Gehrung, Abschrägung, rund, Breite 1 bis 18, Gehrungsgrenze 4 und 10 | innere Ecken bis 80 Codes, wo `lib/gfx` zu viel deckt (unten); Tinte auf 0,4 % gleich |
| `hairlines` | Breite 0, 0,4 (gezeichnet als 1), 1 | gleich, höchstens 1 Code |
| `units` | Breiten in Einheiten und Pixeln, Maßstäbe 0,02 bis 0,2, gedreht | Tinte auf 0,04 % |
| `overlap` | Durchscheinendes an spitzen Ecken | oben |
| `layers` | Reihenfolge der Ebenen, `set_layer`, unsichtbare Ebenen, `remove_layer` | gegen `lib/gfx`: Tinte auf 0,03 %; `set_layer` schickt keine Records |
| `changes` | zufällige Änderungen: einfügen, wachsen, entfernen, Stile; Läufe, Scatter, neuer Pool | jede Runde Byte für Byte wie neu gebaut, gegen `lib/gfx` |
| `visibility` | oben | genau |
| `coords` | oben | 0 Pixel, 0,0039 px |
| `levels` | Voll gegen Basis | Byte für Byte gleich |
| `errors` | jede `Invalid`-Prüfung | wie beschrieben |
| `validate` | alles mit Validierungsschicht und Synchronisationsprüfung | 0 Fehler, 0 Warnungen |
| `window` | die Demo im echten Fenster (ohne Aktivierung, ohne echte Eingabe): erstes Bild, Ziehen, Rad, jedes Mal die Ansicht im Framebuffer gegen `render2d` ohne Fenster | 0 Pixel anders; das Bild verschiebt sich mit dem Ziehen |
| `demo` | dieselbe Demo ohne Fenster über das Skript: Ziehen und Rad bewegen die Zeichnung | Pixel an den erwarteten Stellen |
| `bench_smoke` | die Messung, klein | läuft |

16 von 16 Skripten bestehen (213 Prüfungen in `--test all` je Lauf, dazu Validierung und Fenster).

**Toleranzen** ([`refimg.rs`](../apps/render2dtest/refimg.rs), `Tol`): beide Seiten rechnen die genaue Fläche des Pixels im Polygon; `lib/gfx` hält sie als Byte und mischt ganzzahlig, die GPU in Gleitkomma: 1 bis 2 Codes (`STRAIGHT`). Kreise: Die GPU schneidet sie an der Tangente, `lib/gfx` zeichnet ein Vieleck, dessen Kanten bis 0,05 Pixel innen liegen: bis 13 Codes entlang jedes Bogens, mehr bei Kreisen von einem halben Pixel Radius (`round`). Innere Ecken: Der Umriss von `lib/gfx` dreht sich durch die Ecke, und seine Nicht-null-Füllung summiert die Umläufe eines Pixels, bevor sie begrenzt; wo der Umriss sich selbst überlappt, kommt ein Anteil zu hoch heraus, bis voll. Die GPU rechnet die Vereinigung; nachgeprüft Pixel für Pixel gegen den genauen Anteil einer überabgetasteten Vereinigung (GPU 0,6026, genau 0,6025, `lib/gfx` 1,0) (`corners`). Zufällige Polylinien, die auf sich zurückfalten, tun das an vielen Stellen, dazu mischen ihre Stücke, die sich fern einer Ecke treffen, auf der GPU zweimal, in `lib/gfx` einmal (`folded`). Je Vergleich also: fast jedes Pixel innerhalb `small` Codes, eine begrenzte Zahl darüber, keines über `max`, die Tinte (Summe der Abstände vom Hintergrund) innerhalb `ink`.

## Messung

RTX 2070 SUPER, Windows, Vulkan, `render2dtest --bench 1000000 --frames 10`: 1 Mio. Segmente als 60 734 Polylinien aus 2 bis 33 Punkten, ein Zufallsweg mit Schritten bis 12 Pixel je Achse über 1920 × 1080 bei 1 Einheit je Pixel (also eine sehr dichte Zeichnung: jedes Pixel unter Dutzenden Strichen). Mediane über 10 Bilder, jedes Bild abgewartet; GPU-Zeiten aus den Zeitstempeln der Szene.

| | Voll | Basis |
| --- | --- | --- |
| Haarlinien, alles im Bild (984 000 sichtbar): GPU je Bild | 61,6 ms (Sichtbarkeit 0,20, Zeichnen 61,4) | 61,3 ms |
| Haarlinien, ein Zehntel im Bild (103 000 sichtbar) | 10,1 ms | 9,8 ms |
| 3 px, runde Ecken und Kappen, alles im Bild | 118,9 ms | 118,5 ms |
| 3 px, ein Zehntel im Bild | 19,6 ms | 19,5 ms |
| erstes Bild: alles hoch (61,1 MB) | `prepare` 260 ms CPU, GPU 5,0 ms | 442 ms CPU, 5,0 ms |
| 1000 Entitäten je Bild geändert (16 563 Records, 1,26 MB) | `update` + `prepare` 6,0 ms CPU, GPU 0,20 ms | 6,6 ms, 0,20 ms |
| Einfügen (CPU) | 53 ms | 89 ms |

Die Zeit liegt im Fragment-Shader: Derselbe Durchgang mit einer festen Farbe brauchte in der kleinen Messung 0,05 ms statt 1,3 ms. Diese Szene ist das Schlimmste für ihn: Fast jedes Pixel liegt nahe an Ecken vieler kurzer Segmente. Eine CAD-Zeichnung mit langen Linien bleibt im Rumpf (zwei `cov`); die Messszene dafür kommt in G6a2. Die Sichtbarkeit kostet für 1 Mio. Records 0,2 ms, das Hochladen geänderter Entitäten unter 0,2 ms. `prepare` beim ersten Bild (Records bauen, 61 MB in den Upload-Puffer) ist nicht optimiert; die Streuung zwischen Voll und Basis dort ist CPU-Rauschen.

## Nicht gebaut, nicht geprüft

- **Bögen, Kreise, Strichmuster** (G6a2, ebenso die Messszene mit langen Linien), Kurven, Flächen, Schraffuren, Blöcke (G6b), Text (G6c), Überlagerung und Auswahl (G6d), der CPU-Weg und DXF (G6e).
- **Nur unter Windows auf der RTX 2070 SUPER gelaufen.** Linux, Android und der Browser sind gebaut (`build_rs.py app render2dtest --target linux|android|web`), aber nicht ausgeführt; der Shader hält sich an WebGPU (keine SPIR-V-eigenen Mittel, höchstens drei Bindungsgruppen, Push-Werte als `var<immediate>`), im Browser gelaufen ist er nicht. Andere GPUs: nicht geprüft; ob andere Treiber die Rechnungen des Fragment-Shaders ebenso Bit für Bit gleich machen, wie die Besitzregel es braucht (die Vertex- und Sichtbarkeits-Shader sind `exact`), ist offen.
- **Selbstkreuzungen fern einer Ecke** mischen zweimal (oben).
- **Die Näherung an kurzen Segmenten** zwischen zwei scharfen Ecken (oben).
- **Sehr viele Ebenen**: höchstens 65 535 sichtbare; jede Ebene belegt mindestens eine Seite (256 Records) im Pool.
- **Breiten unter 1 Pixel** werden als 1 Pixel gezeichnet, nicht dünner und blasser.
- **Leistung** bei großer Überdeckung (oben); das Schreiben des Pools beim ersten Bild auf der CPU.

## Entscheidungen in G6a1

Die Vorgaben (eigene Crate, Szene mit Ids, analytische Linien, `f64` als hi/lo, Sichtbarkeit auf der GPU) kommen aus dem Plan für G6 (Entscheidungen 23 bis 31). Dazu hier getroffen:

- **Kein Tiefen- und kein Stencil-Puffer**: die Besitzregel an Ecken statt einer Tiefe; `prepare` bekommt die Größe des Ziels (`prepare(&mut renderer, (b, h))`), der Renderer hält keine Größe.
- **Stile gehören der Szene**, nicht dem Renderer; Farben als `u32` in `0xAARRGGBB` (sRGB, gerades Alpha); sRGB-Ziele werden verweigert.
- **Mindestens 1 Pixel breit**, runde Ecken als Sektor (wie SVG und `lib/gfx`).
- **Vertex- und Sichtbarkeits-Shader `exact` übersetzt** (oben).
- **Zufällige Szenen im Test `changes` deckend**: Durchscheinendes an Selbstkreuzungen mischt auf der GPU zweimal, in `lib/gfx` einmal; das prüft `overlap` gezielt.
