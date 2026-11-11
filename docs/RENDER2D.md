# RENDER2D -- der Kern des 2D-Renderers für CAD

`lib/render2d` zeichnet Zeichnungen aus sehr vielen Linien über [`lib/gpu`](GPU.md): eine Szene, die bleibt (Entitäten mit festen Ids, Ebenen, Stile), von der nur hochgeht, was sich geändert hat; Striche als Vierecke, deren Deckung der Fragment-Shader aus der Geometrie rechnet (kein MSAA), mit genauen Kappen und Ecken; Koordinaten in `f64`, auf der GPU ein Paar `f32` je Achse, von dem zuerst die Kamera abgezogen wird; Sichtbarkeit in Rechen-Shadern, die die Reihenfolge des Malers halten und die indirekten Zeichenbefehle schreiben. Wie `render3d` ist sie der Inhalt eines `ui.gpu_view`: Das Programm klebt sie mit einem `GpuContent` an, `lib/ui` kennt sie nicht. Dieselben WGSL-Quellen für Vulkan Voll, Vulkan Basis und WebGPU (im Browser gelaufen: [Browser](#browser)).

Stand: G6a1, der Kern; G6a2a: Browser, Messszene, schneller Weg für kurze dünne Striche, Intel-Grafik, Tests; G6a2b: Bögen und Kreise, Strichmuster (unten). G6a1, der Kern: Linien und Polylinien (offen, geschlossen), Ebenen, Stile mit Breite in Pixeln oder Zeichnungseinheiten, Kappen (stumpf, rund, quadratisch), Ecken (Gehrung mit Grenze, rund, abgeschrägt), Durchscheinendes ohne doppeltes Mischen an Ecken, Kamera mit Mitte, Maßstab und Drehung, Sichtbarkeit auf der GPU für Basis (ein indirekter Befehl je Ebene) und Voll (ein Multi-Draw). G6a2b: [Bögen und Kreise](#bögen-und-kreise) mit genauer Deckung aus dem Kreis, [Strichmuster](#strichmuster) bis 8 Einträge in Pixeln oder Einheiten auf Linien, Polylinien, Bögen und Kreisen. Noch nicht: Detailgrad (Entscheidung 32), Kurven, Flächen, Schraffuren, Blöcke (G6b), Text (G6c), Auswahl (G6d), CPU-Weg, DXF, andere Plattformen (G6e).

| Teil | Wo |
| --- | --- |
| `Renderer`, Pipelines, Aufzeichnen | [`lib/render2d/lib.rs`](../lib/render2d/lib.rs) |
| `Scene`, Entitäten, Ebenen, Stile, Kamera, Hochladen | [`lib/render2d/scene.rs`](../lib/render2d/scene.rs) |
| Shader | [`common.wgsl`](../lib/render2d/common.wgsl) (Ansicht, Records, Koordinaten), [`cull.wgsl`](../lib/render2d/cull.wgsl), [`scatter.wgsl`](../lib/render2d/scatter.wgsl), [`stroke.wgsl`](../lib/render2d/stroke.wgsl), vorübersetzt in [`shaders_spv.rs`](../lib/render2d/shaders_spv.rs) |
| Tests, Demo, Messung | [`apps/render2dtest`](../apps/render2dtest/main.rs); was beide Wege zeichnen und auf der CPU prüfen: [`gallery.rs`](../apps/render2dtest/gallery.rs); die Referenz über `lib/gfx`: [`refimg.rs`](../apps/render2dtest/refimg.rs); Szenen: [`scenes.rs`](../apps/render2dtest/scenes.rs); im Fenster: [`demo.rs`](../apps/render2dtest/demo.rs); im Browser: [`web.rs`](../apps/render2dtest/web.rs), [`tests/engine_web.mjs`](../apps/render2dtest/tests/engine_web.mjs); Messung: [`bench.rs`](../apps/render2dtest/bench.rs), die Messszene [`cad.rs`](../apps/render2dtest/cad.rs); [`tools/gen_area6.py`](../tools/gen_area6.py) schreibt `area6` |

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

// Achsen strichpunktiert in Zeichnungseinheiten, ein Kreis, eine Tür als Viertelbogen
let achse = scene.add_style(&render2d::Style::new(0xFF80_2020).dashed(&[6.0, 1.5, 0.8, 1.5], render2d::Unit::Drawing, 0.0))?;
scene.insert(mitte, achse, &render2d::Shape::Line { a: [1e7, -5.0], b: [1e7, 45.0] })?;
scene.insert(kontur, duenn, &render2d::Shape::Circle { center: [1e7 + 20.0, 3.0], radius: 0.4 })?;
scene.insert(kontur, duenn, &render2d::Shape::Arc { center: [1e7 + 5.0, 0.0], radius: 0.9, start: 0.0, sweep: core::f64::consts::FRAC_PI_2 })?;
```

## API-Überblick

- **Renderer**: `Renderer::new(&device, &RendererDesc::new(farbformat))` (`#[non_exhaustive]`, Felder danach einzeln ändern: `frames_in_flight`, 1 bis 3, Vorgabe 2); ein Format mit sRGB, Tiefe oder Kompression verweigert er (`Invalid`: [Farben](#farben)). `record(&mut encoder, &scene, farbe)`, `device()`, `desc()`, `multi_draw()` (Voll mit Multi-Draw und `first_instance` im indirekten Befehl), `max_records()`.
- **Scene**: `Scene::new(&renderer)`; Stile `add_style(&Style) -> StyleId`, `set_style`, `style`; Ebenen `add_layer(&Layer) -> LayerId`, `set_layer`, `layer`, `remove_layer` (nimmt ihre Entitäten mit), `drawn_layers()` (die sichtbaren in Zeichenreihenfolge); Entitäten `insert(ebene, stil, &Shape) -> EntityId`, `update(id, &Shape)`, `set_entity_style(id, stil)`, `remove(id)`, `len`, `is_empty`, `locate(id)` (erster Platz und Zahl seiner Records im Pool); `set_camera(&Camera)`, `camera()`; je Bild `prepare(&mut renderer, (b, h))`, `submitted(id)`, `wait_slot()`.
- **Werte**: `Camera { center: [f64; 2], scale, rotation }` (`scale` Zeichnungseinheiten je Pixel, `rotation` im Bogenmaß gegen den Uhrzeigersinn, y zeigt auf dem Bildschirm nach oben; `Camera::new(mitte, scale)`, `to_screen`, `to_drawing`, `pan(dx, dy)` in Pixeln, `zoom_at(punkt, faktor, größe)`: der Punkt unter dem Zeiger bleibt). `Style { color, width, unit, cap, join, miter_limit, dash, dashes, dash_unit, dash_phase }` (`Style::new(farbe)`: Haarlinie, `Unit::Px`, `Cap::Butt`, `Join::Miter`, Grenze 4, durchgehend; `.dashed(&muster, einheit, phase)`: bis `DASH_MAX` = 8 Längen, an und aus im Wechsel). `Unit::{Px, Drawing}`, `Cap::{Butt, Round, Square}`, `Join::{Miter, Round, Bevel}`, `Layer { visible, order }` (`Layer::new(order)`), `Shape::{Line { a, b }, Polyline { points, closed }, Arc { center, radius, start, sweep }, Circle { center, radius }}` (Winkel im Bogenmaß, positiv gegen den Uhrzeigersinn; ein Bogen sind Stücke von höchstens `ARC_STEP` = π/8). Typen, die wachsen werden, sind `#[non_exhaustive]`.
- **Zum Messen und Testen**: `set_stats(true)`, `take_stats()`, `collect_stats()` (`FrameStats`: sichtbare Records, vier Zeitstempel: Anfang, nach dem Hochladen, nach der Sichtbarkeit, nach dem Zeichnen), `upload_info()` (`UploadInfo`: was `prepare` geschickt hat: Records, Läufe, Scatter oder Kopien, neuer Pool, kopierte Bytes, Tabellen), `set_culling(false)`, `lists_buffer()`, `visible_buffer()`, `draws_buffer()`, `pool_buffer()`; die Shader als `COUNT_SPV` … `FS_SPV`, `VS_FULL_SPV`, `FS_FULL_SPV` und `CULL_WGSL`, `SCATTER_WGSL`, `STROKE_WGSL`.

Reihenfolge: Ebenen nach `Layer::order`, bei gleicher nach dem Hinzufügen (ein `set_layer` mit neuer `order` legt die Ebene obenauf in dieser Ordnung), in einer Ebene die Reihenfolge des Einfügens; ein `update` behält den Platz. Breite 0 ist eine Haarlinie von 1 Pixel; schmaler als 1 Pixel zeichnet sie nie (auch nicht in Zeichnungseinheiten beim Herauszoomen). Fehler sind die von `lib/gpu`: `Invalid` für tote Ids, Stile, die die Szene nicht hat, nicht endliche oder zu große Punkte (ab `1e30`), Formen ohne Punkt, negative oder nicht endliche Breiten, eine Gehrungsgrenze unter 1, mehr als `DASH_MAX` Einträge, negative oder nicht endliche Einträge, ein Muster aus lauter Nullen, Bögen mit Radius ≤ 0 oder Drehung 0, eine Kamera ohne endliche Mitte oder mit Maßstab ≤ 0, `record` vor `prepare`, nach einer Änderung seit `prepare`, mit einem Ziel anderen Formats oder anderer Größe, Szenen eines anderen Renderers; `Unsupported`, wenn die Records die Bindungen oder einen Dispatch des Geräts übersteigen oder mehr als 65 535 Ebenen sichtbar sind. Ein Punkt allein ist ein Segment der Länge 0: runde und quadratische Kappen zeigen ihn.

## Aufbau

**Daten.**

| Was | Wie groß | Wer liest |
| --- | --- | --- |
| Pool (Records) | 64 Byte je Segment: Anfang und Ende als (hi.x, hi.y, lo.x, lo.y), der eigene Vektor (`f32`), die Länge entlang der Entität an seinem Ende (`f64` als drei `f32`), bei einer geschlossenen Polylinie im ersten und letzten Record, wie weit das andere liegt, Stil + 1 (0: frei), Bits (verbunden am Anfang, am Ende; Anfang oder Ende der Polylinie; erstes Record der Entität; geschlossen) und Art. Ein Stück eines Bogens: Mitte, Ende, Radius (hi, lo), Länge, Bits ([Bögen und Kreise](#bögen-und-kreise)); davor ein Kopf mit dem Anfang des Bogens, Stil 0 | `count`, `write`, Vertex-Shader |
| Stile | 128 Byte: Farbe, Breite, Einheit, Kappe, Ecke, Gehrungsgrenze, Zahl und Einheit der Einträge des Musters, Perioden je Einheit (κ) und Phase, Periode, Anteil an, Striche je Periode, die Enden der Einträge ([Strichmuster](#strichmuster)) | `count`, Vertex- und Fragment-Shader |
| Ansicht | 64 Byte: Mitte der Kamera als hi/lo, was hi + lo von ihr nicht halten, 1 / Maßstab, Maßstab, cos und sin der Drehung, Größe des Ziels, Culling aus | alle |
| `order`, `layers` | je Seite der sichtbaren Ebenen ihre Nummer; je sichtbare Ebene ihre erste Seite und ihre Seiten | `count`, `scan` |
| `masks`, `counts`, `offsets` | je Seite 8 Wörter (ein Bit je Record), ihre Zahl, ihr erster Platz | `count` schreibt, `scan`, `write` |
| `lists`, `draws`, `visible` | je sichtbare Ebene erster Platz und Zahl, ihr indirekter Befehl (20 Byte); 4 Byte je sichtbares Record | `scan`, `write` schreiben, Vertex-Shader und Grafik-Pipeline lesen |
| Index-Puffer (je Renderer) | 16 384 Vierecke, `u16`, je Viereck 0 1 2 2 1 3 | Grafik-Pipeline |

Kein Vertex-Puffer: Der Vertex-Shader holt sein Record über die sichtbare Liste (der Platz aus der Instanz und der Nummer der Ecke, die Ebene aus den oberen 16 Bit der Instanz oder dem Push-Wert), die eigenen Vektoren seiner Nachbarn aus den Records davor und danach im Pool (bei einer geschlossenen Polylinie am ersten und letzten über den Abstand im Record). Bis G6a2a hielt jedes Record die Vektoren zum vorigen und nächsten Punkt selbst; der Platz ging an die Länge entlang der Entität, die ein Muster braucht, und so bleibt das Record bei 64 Byte. Jede Ebene besitzt einen Bereich des Pools aus ganzen Seiten zu 256 Records; die Records ihrer Entitäten liegen dort in der Reihenfolge des Einfügens, die einer Entität beieinander. Die Seiten der sichtbaren Ebenen in ihrer Zeichenreihenfolge sind also schon die Reihenfolge des Malers.

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
| Durchgang | ein Durchgang ohne Tiefe, Mischen mit vormultipliziertem Alpha; Voll: ein `draw_indexed_indirect_multi` je Folge von Ebenen mit derselben Pipeline ([Zwei Pipelines](#zwei-pipelines)); Basis: je Ebene ein Push-Wert und ein `draw_indexed_indirect` |

Die Barrieren stehen in [`lib.rs`](../lib/render2d/lib.rs) in `Renderer::record`; die Synchronisationsprüfung der Validierungsschicht meldet für alle Tests 0 Fehler und 0 Warnungen. G6a2b hat keine Barriere hinzugefügt oder geändert: Der Fragment-Shader liest jetzt die Stile (die Muster) und die Ansicht; die Barriere nach dem Hochladen gibt Kopien für `GRAPHICS_READ` frei, und das schließt in `lib/gpu` (`vk_cmd.rs`) den Fragment-Shader ein, die Barriere am Anfang deckt sein Lesen im Bild davor gegen die nächste Kopie. Die Stile gehen öfter hoch als bisher (ein Muster in Pixeln bei jeder Änderung des Maßstabs), über denselben Weg.

## Der Shader

**Vertex-Shader.** Er legt sein Segment relativ zur Kamera in Pixel ([Koordinaten](#koordinaten)) und schneidet es auf das Ziel plus seine Reichweite zu (Liang-Barsky); ein Ende jenseits davon ist offen: keine Kappe, keine Ecke, davon zeigt sich ohnehin nichts. Das Viereck deckt das Rechteck des Segments, an jedem Ende so weit, wie seine Kappe oder Ecke darüber hinausreicht (stumpf und abgehende Ecke: 0, rund und quadratisch: die halbe Breite, ankommende Gehrung: die halbe Breite mal max(sin φ, tan(φ/2)) bis zur Spitze), und ein Pixel mehr für die geglättete Kante. Die halbe Breite ist `max(Breite / 2, 0,5)` Pixel (Breiten in Zeichnungseinheiten durch den Maßstab).

**Fragment-Shader: Deckung, genau.** Der Anteil des Pixels (ein Quadrat der Seite 1) im Strich ist die Fläche des Quadrats in konvexen Stücken aus Geraden. Eine Kante allein: der Anteil der Halbebene, `cov`, der Kastenfilter einer geraden Kante unter jedem Winkel, in geschlossener Form. Die zwei parallelen Kanten des Streifens allein: `cov(links) + cov(rechts) - 1`, auch für Haarlinien genau. Mehr Kanten: `area6`, die Fläche des Quadrats in bis zu sechs Halbebenen nach dem Satz von Green (die Randstücke des Quadrats und der Geraden, jedes auf alle anderen gekürzt, die Summe der Kreuzprodukte), von [`tools/gen_area6.py`](../tools/gen_area6.py) ausgeschrieben (`--check` vergleicht ihn mit dem Text), ohne Schleifen und Felder: Mit Feldern lagen sie im lokalen Speicher, und der Shader war hundertmal langsamer. Ein Kreis wird an der Stelle des Pixels durch seine Tangente ersetzt (der Fehler ist quadratisch in der Pixelgröße über dem Radius). Die meisten Pixel liegen im Rumpf: der Streifen allein, kein Ende nahe, zwei `cov`.

**Kappen.** Stumpf: die Gerade im rechten Winkel durch das Ende. Quadratisch: dieselbe eine halbe Breite weiter. Rund: das Rechteck endet am Ende, der Halbkreis dahinter ist ein eigenes Stück; beide berühren sich nur an der Geraden, die Anteile addieren sich also genau.

**Ecken: Kacheln.** Nahe einer Ecke wird der Strich an der Winkelhalbierenden zerschnitten: der Streifen dieses Segments auf seiner Seite der Halbierenden, der des Nachbarn auf der anderen; eine Abschrägung schneidet beide mit ihrer Geraden; eine runde Ecke endet in beiden Kacheln an der Geraden im rechten Winkel durch die Ecke, der Kreissektor dahinter ist ein Stück für sich (nur der Sektor außen, wie bei SVG und `lib/gfx`, kein voller Kreis, der hinter stumpfe Enden ragte). Die Kacheln überlappen nicht und füllen den Strich, solange die Sehne der Halbierenden (die halbe Breite mal tan(φ/2) entlang jedes Segments) in beiden Segmenten liegt; dann addieren sich ihre Anteile genau, und außen ergibt sich die Gehrungsspitze, die abgeschrägte Ecke oder der Bogen von selbst. Eine Gehrung über ihrer Grenze wird zur Abschrägung (Grenze wie SVG: Länge der Spitze durch die Breite).

**Scharfe Ecken an kurzen Segmenten.** Ist die Sehne länger als ein Segment, nimmt der Shader die Vereinigung: die zwei Rechtecke, das Stück der Ecke jenseits beider Enden (Gehrungsviereck, Dreieck, Sektor; es berührt keines), `|a| + |w| + |b| - |a ∩ b|`. Liegt ein kurzes Segment nahe an beiden Ecken, sind es beide Vereinigungen; ausgelassen wird dann nur, wo sich die Rechtecke der zwei Nachbarn gegenseitig treffen, und der Schnitt `a ∩ b` ohne die fernen Enden (`area6` hat sechs Halbebenen). Das ist die einzige Näherung; sie trifft Polylinien, die auf wenigen Pixeln zweimal scharf umkehren.

**Der schnelle Weg.** Kurze dünne Striche (Segment bis `TINY_LEN` = 3 Pixel lang, halbe Breite bis `TINY_H` = 0,75: was eine Zeichnung aus der Ferne von ihren Details zeigt) rechnen nahe ihrer Enden nicht die genaue Fläche: Die Besitzregel und die Nähe-Tests bleiben, die Kachel eines Stücks ist das Produkt aus dem Anteil seines Streifens und dem Anteil des Pixels auf seiner Seite der Halbierenden (die Kappe an einem Polylinienende eine Gerade, rund: wo sie die gleiche Fläche hielte). Längere oder breitere Striche und jeder Strich im Rumpf bleiben genau. Das macht das Bild im Dichten der Übersicht schneller (unten), an Stellen kurzer Segmente mit scharfen Knicken aber um einige Codes anders, im Einzelfall um bis 200: [Messung](#messung).

**`discard` hält nicht an.** WGSL (und `OpDemoteToHelperInvocation` auf Vulkan) lässt das Programm nach einem `discard` weiterlaufen; `fs` kehrt deshalb nach jedem `discard` sofort zurück (`discard; return vec4f(0.0);`). Vorher zahlten die Pixel, die ein Nachbar besitzt, den ganzen Weg mit: 25,4 → 21,7 ms auf der RTX bei unveränderten Pixeln.

**Eine Stelle für `area6`.** Die Stücke nahe eines Pixels sind Bits einer Maske (`S_*` in `stroke.wgsl`), und eine Schleife addiert (oder zieht ab) ihre Anteile; `area6` steht im Shader einmal. Vorher stand es, nach dem Einsetzen der Funktionen, an rund vierzig Stellen, und die GPU verbrachte ihre Zeit mit dem Laden von Code: in der kleinen Messung (20 000 Segmente auf 320 × 180) 6,5 ms statt 1,3 ms für Haarlinien und 17 ms statt 2,4 ms für 3 px mit runden Ecken; derselbe Shader nur mit dem Rumpf brauchte 0,05 ms.

## Überlappung

Ein durchscheinender Strich darf an seinen Ecken nicht zweimal gemischt werden, und das ohne Stencil und ohne MSAA. Jedes Pixel nahe einer Ecke liegt in den Vierecken beider Segmente. Der Trick: **Ein Pixel nahe einer Ecke gehört dem näheren der beiden Segmente** (Abstand zur Mittellinie als Strecke; nahe der Ecke ist die Grenze die Winkelhalbierende, hinter dem Ende eines kurzen Nachbarn die längere Strecke), bei Gleichstand dem ankommenden; das andere verwirft es. Das besitzende Segment rechnet den Anteil beider Kacheln (oder der Vereinigung) und mischt einmal. Damit genau eines zeichnet, müssen beide dieselben Abstände Bit für Bit rechnen: Die Ecke kommt bei beiden aus denselben hi/lo-Werten, und die Vektoren rundet die CPU einmal (`f64`-Differenz, dann `f32`): Jedes Segment hält nur seinen eigenen Vektor (`Record.v`), und die Nachbarn lesen ihn dort, also dieselben Bits. Der Code: die Abfragen am Anfang von `fs` und `fs_full` in [`stroke.wgsl`](../lib/render2d/stroke.wgsl). Bögen: dasselbe an der radialen Geraden durch die Fuge zweier Stücke ([Bögen und Kreise](#bögen-und-kreise)).

Wo eine Polylinie sich fern einer Ecke selbst kreuzt, mischen ihre Stücke je einmal, wie zwei Entitäten es täten (ein Strich ohne Selbstüberlappung bräuchte dort Stencil oder eine Tiefe; nicht gebaut). Der Test `overlap` zeigt es an spitzen Ecken (Gehrung, Abschrägung, rund, Deckkraft 0x80 und 0x40): kein Pixel dunkler als ein einmaliges Mischen der Farbe erlaubt, gegen `lib/gfx` (dessen Füllung einmal mischt) die Tinte auf 0,04 % gleich.

## Koordinaten

Ein Punkt ist auf der CPU `f64` und auf der GPU ein Paar je Achse: `hi = f32(x)`, `lo = f32(x - hi)`. Die Mitte der Kamera kommt genauso, und was `hi + lo` von ihr nicht hält (`rest`, bei 1e7 höchstens 2⁻²⁶ einer Einheit), zieht der Shader danach ab. `rel2` in [`common.wgsl`](../lib/render2d/common.wgsl) zieht zuerst die Kamera ab: `hi - hi` und `lo - lo` je über eine genaue Zwei-Summe (Knuth), dann zusammengefasst zu einem Doppel-`f32`-Paar. Ein Punkt nahe der Kamera kommt so auf wenige 1e-15 Einheiten genau heraus, ein ferner Punkt mit den 24 Bit von `f32` seines Abstands. Liegen beide Enden eines Segments weiter als 131 072 Pixel weg (eine lange Linie durch das Bild), kommt die Lage seiner Geraden aus dem genauen Kreuzprodukt beider Enden in Doppel-`f32` (`far_foot`), sonst wanderte die Linie.

Die GPU darf das nicht umstellen: Der NVIDIA-Treiber fasste `(a - (s - bb)) + (b - bb)` zusammen und verlor den `lo`-Teil. Deshalb werden die Vertex- und die Sichtbarkeits-Shader mit `wgsl::CompileOptions { exact: true }` übersetzt (`NoContraction` an jeder Rechnung); den Fragment-Shader braucht das nicht.

Das reichte für die Bögen nicht (G6a2b): Derselbe Treiber machte trotz `NoContraction` aus `(a + b) - a` in einer Zwei-Summe `b`, der Fehlerterm wurde 0, und ein Kreis von 1e7 Radius lag 0,02 Einheiten neben seinem Platz. `opq(x)` in [`common.wgsl`](../lib/render2d/common.wgsl) gibt `x` zurück, aber durch eine Tür, durch die kein Übersetzer sieht: seine Bits mit denen von `view.misc.y` (immer 0) verknüpft (xor). Wo eine Summe oder ein Produkt der Zwei-Summen und Zwei-Produkte durch `opq` geht, kann keine Algebra darüber hinweg. Die Produkte: bis G6a2a `fma(a, b, -p)`; `Fma` in SPIR-V ist aber nur mit `NoContraction` eine einzige Rundung, und `lib/shader` setzt das nur an die Rechenoperationen. Jetzt das Produkt der Hälften nach Dekker (`split_f`, `two_prod`: 12 Bit je Hälfte, ihre Produkte sind genau). Die Bilder der Linien sind dadurch Byte für Byte gleich geblieben (90 Bilder gegen den Stand vor G6a2b). Winkel: [Bögen und Kreise](#bögen-und-kreise).

**Prüfung** (`coords`, beide Stufen): Dieselbe Zeichnung bei (1e7, 1e7), (-9 999 999,5, π · 1e6) und (9 876 543,210987, -9 999 000) gegen am Ursprung: 0 Pixel anders. Bei 1e-6 Einheiten je Pixel nahe (-9 876 543,21, 6 543 210,987), 24 Verschiebungen um 0,37 Pixel und eine Drehung, sechs Linien bis 2e7 lang: Der Schwerpunkt jeder Bildzeile liegt höchstens 0,0039 Pixel von der Stelle, an die die Linie gehört (Ziel: 1/16 = 0,0625), keine Zeile ohne Linie. Bögen, Kreise und Muster genauso: [Bögen und Kreise](#bögen-und-kreise), [Strichmuster](#strichmuster).

## Bögen und Kreise

`Shape::Arc { center, radius, start, sweep }` (`f64`, Winkel im Bogenmaß, positiv gegen den Uhrzeigersinn; ein Umlauf oder mehr ist ein Kreis, der bei `start` beginnt) und `Shape::Circle { center, radius }` (beginnt bei 0). Ein Bogen sind ein Kopf und Stücke von höchstens `ARC_STEP` = 22,5° (ein Kreis 16, also 17 Records). Das Record eines Stücks hält die Mitte des Kreises und das Ende des Stücks (beide hi/lo), den Radius (hi, lo), die Länge am Ende (für die Muster) und Kappen, Drehsinn, Musteranfang als Bits; sein Anfang ist das Ende des Records davor, beim ersten Stück der Kopf, der nur diesen Punkt hält und nie gezeichnet wird. So rechnen zwei Stücke ihre gemeinsame Fuge aus denselben Bits.

**Vertex-Shader** (`vs_arc`). Wo der Kreis bei der Kamera liegt, kommt aus |v|² − r² in Doppel-`f32` (v: Kamera minus Mitte; beide Quadrate können 1e14 und mehr sein, der Abstand ein Tausendstel): `D0 = (|v|² − r²) / (|v| + r)`, daraus F, der Punkt des Kreises, der der Mitte des Ziels am nächsten liegt, die Normale u dort und der Radius in Pixeln. Das Viereck eines Stücks liegt zwischen der Sehne auf dem inneren Radius und der Tangente auf dem äußeren (beide eine halbe Breite und ein Pixel vom Kreis), an den Fugen ein Pixel, an den eigenen Enden die Reichweite der Kappe weiter. Ist der Radius größer als die halbe Diagonale des Ziels plus Rand, liegt alles Sichtbare des Rings innerhalb einer Vierteldrehung von F, und das Viereck deckt nur die Winkel, unter denen der Ring das Ziel treffen kann (asin der Projektion der Ecken auf die Tangente in F). Die Ecken des Vierecks als Abstände von F: `(dr cos b − 2 r sin²(b/2)) u + (r + dr) sin b t`, genau für jeden Radius.

**Fragment-Shader** (`fs_full`). Der Abstand eines Pixels q vom Kreis: `(|q − F|² + 2 r (q − F)·u) / (|q − F + r u| + r)`, in `f32` genau, auch bei r = 1e13 Pixeln. Der Ring an der Stelle des Pixels ist der Streifen zwischen den Tangenten seiner beiden Kreise (wie Kappen und runde Ecken: der Fehler ist quadratisch in der Pixelgröße über dem Radius); im Rumpf zwei `cov`. Ein Pixel nahe der Fuge zweier Stücke gehört dem, auf dessen Seite der radialen Geraden durch die Fuge seine Mitte liegt (beide rechnen sie aus denselben Bits), und das rechnet den ganzen Ring dort: keine Naht, Durchscheinendes einmal gemischt. Kappen an den eigenen Enden: stumpf die radiale Gerade, quadratisch ein Rechteck entlang der Richtung am Ende (ein eigenes Stück, wie bei SVG gerade, nicht gebogen), rund der Halbkreis dahinter.

**Winkel ohne die GPU.** Sinus, Kosinus, atan2 und asin der GPU dürfen nach Vulkan absolut 2⁻¹¹ (sin, cos) oder 4096 ulp (atan2) daneben liegen; bei einem Radius von 1e13 Pixeln wären das bei Sinus und Kosinus Milliarden Pixel, bei atan2 eines Winkels von 1e-6 Tausende. Unter 0,5 (atan2: unter einem Zehntel) nimmt der Shader ihre Reihen (`sin_p`, `cos_p`, `atan2_p`), und das sind die Winkel eines großen Kreises.

**Was die Vierecke kosten** (Test `arcs`, die Vierecke von `vs_arc` auf der CPU nachgerechnet, gezählt die Pixelmitten darin, die der Fragment-Shader also sieht): ein Kreis von 540 Pixeln Radius in 1920 × 1080, Haarlinie: 44 416 Pixel, das 4,4-fache seines Rings samt geglätteter Kante (10 179), sein Quadrat 1,17 Mio.; halbe Breite 2 px: das 2,7-fache; 60 px Radius: das 1,5-fache; 5000 px Radius quer durchs Bild: 51 158 Pixel, das 8,9-fache des Rings im Bild. Mit festen 16 Stücken wächst der Abstand von Sehne und Tangente mit dem Radius (r (1 − cos 11,25°) = 0,019 r je Seite), bis der Ausschnitt ab der halben Diagonale greift; Stücke abhängig vom Zoom gäbe es nur, wenn ein Record mehrere Vierecke ausgäbe (nicht gebaut).

**Prüfung.** Gegen `lib/gfx` (`arcs`; die Referenz füllt den genauen Umriss des Strichs, denn `dash_stroke` dreht die Innenseite jedes Punkts eines feinen Vielecks durch den Punkt, und die Nicht-null-Füllung summiert das: ein Ring 0,25 px zu breit innen bei halber Breite 4,5): Breite 3 und 9 höchstens 10 Codes, kein Pixel über 16, die Tinte auf 0,05 % gleich; Haarlinien 1 bis 5 Pixel über 16 Codes, der größte Unterschied (60) am Kreis von 0,8 px Radius (Tangente an Kreisen unter einem Pixel), die Tinte bis +0,13 % (runde Kappen). Durchscheinende Ringe und Bögen: kein Pixel über ein einmaliges Mischen hinaus. Große Koordinaten (`coords`): dieselben Bögen, Kreise und Muster bei 1e7 wie am Ursprung, 0 Pixel anders; bei 1e-6 Einheiten je Pixel, 24 Verschiebungen um 0,37 px und einer Drehung: Kreise von 1e7 (zwei), 3e5 und 33 px Radius und ein Bogen von 1e7, dessen Enden 1e6 Einheiten fern liegen: der Schwerpunkt jedes Schnitts von Zeile oder Spalte höchstens 0,0073 px von seinem Platz (die großen 0,0034, 0,0048, 0,0040, 0,0031; 1163 Schnitte, keiner ohne Ring). Sichtbarkeit (`visibility`): jede sechste Entität der Zufallsszenen ein Bogen oder Kreis, geprüft gegen den Kasten der Sehne plus Pfeilhöhe in `f64`: genau.

## Strichmuster

`Style::dash` (bis `DASH_MAX` = 8 Längen, an, aus, an …), `dashes`, `dash_unit` (Pixel oder Einheiten, unabhängig von der Breite), `dash_phase`; `Style::dashed(&muster, einheit, phase)`. Wie SVG und `lib/gfx`: eine ungerade Zahl läuft zweimal, jedes Strichende bekommt die Kappe des Stils, eine Ecke nur, wo ein Strich durch den Punkt läuft, ein Strich der Länge 0 ist ein Punkt. Das Muster läuft über die ganze Entität: eine Polylinie über ihre Ecken, eine geschlossene einmal herum ab dem ersten Punkt, ein Bogen ab seinem Anfang, ein Kreis ab dem Winkel 0. Ist eine geschlossene Linie im ersten Punkt auf beiden Seiten an, verbinden sich der letzte und der erste Strich mit der Ecke (wie Chromium/Skia; `lib/gfx` setzt dort zwei Kappen).

**Wo das Muster steht.** Ein Record hält die Länge entlang der Entität an seinem Ende (`f64` als drei `f32`, deren Summe genau ist), das Record davor die am Anfang. Der Stil hält die Perioden je Zeichnungseinheit (κ, `f64` als drei `f32`; bei Mustern in Pixeln je Bild neu, wenn sich der Maßstab ändert, dann gehen die Stile hoch) und die Phase in Perioden. Der Vertex-Shader rechnet den Platz an einem Bezugspunkt: dem Fußpunkt der Kamera auf der Geraden des Segments, im Segment gehalten (ein Bogen: F im Stück), vom näheren Ende aus in Doppel-`f32` (beim Bogen weiter als 1e5 px vom Ende die Sehne in Doppel-`f32` und daraus 2r asin(c/2r) mit einem Newton-Schritt). Der Platz ist frac(L κ + p0); L κ ist bei 1e7 Einheiten und 1e-6 Einheiten je Pixel 1e12 Perioden und mehr, jenseits von `f32` und von Produkten in Doppel-`f32`: `periods` nimmt die genauen Teile der Produkte (Dekker) und addiert nur ihre Bruchteile, in Doppel-`f32`. Der Fragment-Shader läuft das Muster vom Bezugspunkt aus ab, in Mustereinheiten relativ zu dessen Platz, so dass auch die Enden einer Periode von einer Million Pixeln auf 1/16 px stimmen.

**Stücke.** Jeder Strich, der das Pixel trifft, ist ein Stück: der Streifen zwischen seinen Enden (eine quadratische Kappe verlängert ihn um die halbe Breite, Striche, die sich dann treffen, werden einer; beim Bogen ein eigenes Rechteck), eine runde Kappe ein Halbkreis für sich, zwei runde (beim Bogen auch quadratische) Kappen benachbarter Striche, die sich treffen, einmal gezählt (|a| + |b| − |a ∩ b|). Nahe einer Ecke kommen die Striche des Nachbarn dazu, abgeschnitten an der Geraden im rechten Winkel durch die Ecke, und wo ein Strich durch die Ecke läuft, das Stück der Ecke; die Überlappung der beiden durchlaufenden Striche innen (zwischen den inneren Kanten ihrer Streifen und den Geraden durch die Ecke) wird abgezogen und dort geschnitten, wo diese Striche enden (die Vereinigung von oben, für Muster immer: die Kacheln setzen durchgehende Striche voraus). Ein Strichende, das auf 1/1000 px an einem Ende des Segments liegt, liegt dort (sonst entschied das Runden, ob dort eine Kappe oder die Ecke steht).

**Flächenton.** Ist die Periode höchstens `DASH_FLAT` = 2 px, zeichnet der Shader den durchgehenden Strich mal dem Anteil des Musters (die Kappen mit der Fläche, die sie hinzufügen); ab `DASH_EXACT` = 4 px das Muster genau, dazwischen gemischt. Ein Muster von 1,5 px verschoben um 0,37 px entlang der Linie: kein Pixel ändert sich; die Tinte gegen die echten Striche von `lib/gfx` −0,19 % (1,5 px), +0,01 % (3 px), −0,18 % (6 px).

**Prüfung** (`dashes`): Die Referenz schneidet das Muster auf der CPU in `f64` (`dash_pieces`, SVGs Regeln; gegen `lib/gfx`s eigene Striche auf offenen Linien: höchstens 1 Code) und lässt `lib/gfx` jeden Strich mit Kappen und Ecken zeichnen. Linien unter acht Winkeln mit zwei Mustern und Phasen, ein ungerades Muster, Punkte (Länge 0), ein Zickzack, dessen Striche durch manche Ecken laufen (einer 0,17 px vor einer spitzen Gehrung), geschlossene Polylinien an und aus im ersten Punkt, ein Bogen und ein Kreis mit Muster: stumpf 8 bis 12 Pixel über 2 Codes, höchstens 50 (kurze Striche an Ecken), quadratisch 9 Pixel über 2 Codes, höchstens 29; rund 112 (Breite 3) und 63 (Breite 8) Pixel über 16 Codes, höchstens 50 und 60, die Tinte runder Kappen +1,46 % und +0,35 % (die Halbkreise von `lib/gfx` sind Vielecke von 0,52 rad Schritt bei halber Breite 1,5: −4,5 % ihrer Fläche, die Tangente der GPU deckt etwas mehr). Muster in Einheiten bei vier Maßstäben und Drehungen (Toleranz runder Kappen): bei 0,02 bis 0,1 Einheiten je Pixel höchstens 2 Pixel über 16 Codes, bei 0,2 (gedreht um −1,2) 89 Pixel über 16 Codes, höchstens 82, die Tinte −0,53 %. `set_style` von durchgehend zu Muster und zurück: jedes Bild wie neu gebaut. Genauigkeit fern (`phase`, beide Stufen, auch im Browser): eine Polylinie bei 1e7 mit Segmenten von 1e7, 1e-6 Einheiten je Pixel, 24 Verschiebungen: an ihrer Ecke Muster in Pixeln 0,0066 px, in Einheiten 0,0071 px; in der Mitte eines Segments (5e6 von beiden Enden) 0,039 und 0,038 px; eine Periode von einer Million Pixeln 0,045 px; ein Kreis von 1e7 Radius an der Fuge zweier Stücke 0,0078 px, in der Mitte eines Stücks 0,022 px (Ziel 1/16 = 0,0625). Gemessen als Anteil des Strichs je Pixelzeile gegen den Anteil des Musters über dem Pixel, beide aus denselben Zahlen (die Enden der Einträge als `f32`, κ aus `f64`): Bei 1e12 Perioden entscheidet die zwölfte Stelle der Periode, wo ein Strich liegt.

## Zwei Pipelines

Ebenen mit Bögen oder Strichmustern zeichnet `fs_full` mit `vs_full`, die anderen `fs` und `vs` wie bisher (Voll: ein Multi-Draw je Folge von Ebenen derselben Pipeline, Basis: je Ebene). Der Grund: In einem Shader mit beidem stand `area6` an zwei Stellen, und der NVIDIA-Treiber setzte es dann nicht mehr ein, sondern rief es als Funktion: jedes Pixel dreimal langsamer, auch die ohne Bogen (Übersicht 11 → 34 ms; mit dem neuen Teil ohne `area6` 10 ms). `fs_full` rechnet auch durchgehende Segmente (wie `fs`, nahe ihrer Ecken aber über die Vereinigung, damit `area6` dort einmal steht). Die Szene zählt je Ebene ihre Bögen und Entitäten mit Muster (`set_style`, das ein Muster an- oder ausschaltet, zählt die Entitäten des Stils neu). Die Reihenfolge des Malers bleibt: eine Ebene zeichnet eine Pipeline.

## Browser

`render2dtest` im Browser: `?gpuenv=jfgui_GPU_PROBE=test` (statt der Demo), [`apps/render2dtest/web.rs`](../apps/render2dtest/web.rs); `node apps/render2dtest/tests/engine_web.mjs [build] [--modes=simd,threads,scalar]` startet Chromium kopflos über Playwright, liest die Bilder von der Konsole, vergleicht sie mit `render2dtest --dump` auf derselben Maschine. Ohne WebGPU: SKIP.

Chromium 157 kopflos, RTX 2070 SUPER über D3D12 (Treiber 32.0.16.1088), die drei Bündel scalar, simd, threads (Stand G6a2b; die SwiftShader-Zeile aus G6a2a):

| Was | Ergebnis |
| --- | --- |
| Koordinaten | dieselbe Zeichnung an drei Stellen bis 1e7 gegen am Ursprung, Linien und ebenso Bögen, Kreise und Muster: 0 Pixel anders; 1e-6 Einheiten je Pixel nahe 1e7, 24 Pans, sechs Linien bis 2e7 lang: der Schwerpunkt jeder Zeile höchstens 0,0039 Pixel von seinem Platz; Ringe von 1e7, 3e5 und 33 px Radius: 0,0073 px (beides wie Vulkan) |
| Muster fern (`phase`) | alle sieben Fälle wie auf Vulkan, Zahl für Zahl: 0,0066 / 0,0071 px an der Ecke, 0,039 / 0,038 px in der Mitte von 1e7, 0,045 px bei einer Periode von 1e6 px, Kreis 0,0078 / 0,022 px |
| Besitzregel (`overlap`) | Durchscheinendes an spitzen Ecken und an den Fugen von Bögen und Kreisen: kein Pixel über ein einmaliges Mischen hinaus (0) |
| Sichtbarkeit | vier zufällige Szenen gegen die genaue Rechnung auf der CPU: 0 fehlend, 0 zu viel, 0 doppelt, 0 außer der Reihe |
| Bilder gegen Vulkan (Voll, Basis) | 69 Bilder (Kappen, Ecken, Haarlinien, Einheiten, Überlappung, kurze Segmente, Bögen, Muster, Muster in Einheiten, Flächenton): 25 von 5 658 000 Pixeln um 1 Code anders (am meisten `units_dash_1`: 4 Pixel); alle Bilder an allen Bündeln (G6a2a: 45 Bilder, 3 Pixel) |
| Validierung des Browsers | keine Meldung |
| SwiftShader (Chromium, `--use-webgpu-adapter=swiftshader`, Vulkan in Software) | dieselben Prüfungen bestehen, Koordinaten 0,0040 px; die Bilder gegen die GPU von Vulkan 101 067 von 3,7 Mio. Pixeln um 1 oder mehr anders (anderes Runden der CPU; höchstens 3 Codes, in `short_5` ein Pixel 122 Codes): dort meldet der Test FAIL, die CPU-Prüfungen bestehen |

**Damit ist die Rechnung mit den `f32`-Paaren im Browser hier bewiesen: bei Chromium 157 über D3D12 auf dieser NVIDIA-GPU und über SwiftShader; WGSL kennt kein `exact`, die Rechnung geht durch Tint und den Treiber.** Nicht bewiesen: Chromium über Vulkan (Linux, oder Windows mit dem Vulkan-Backend, das sich hier mit Flags nicht wählen ließ: dort ginge SPIR-V aus Tint ohne `NoContraction` an denselben NVIDIA-Treiber, der es auf dem Vulkan-Weg ohne `exact` wegfasste), Metal/Safari, Firefox (kopflos kein `navigator.gpu`), andere GPUs im Browser. Falls ein Ziel die Zwei-Summen wegfasst, zeigen das Bild-Test und Tiefzoom sofort; dann bliebe die Subtraktion der Kamera auf der CPU. Die undurchsichtige Null-Operation in `two_sum` ist seit G6a2b gebaut (`opq`, [Koordinaten](#koordinaten)): Der NVIDIA-Treiber hatte auf Vulkan trotz `NoContraction` eine Zwei-Summe vereinfacht.

## Sichtbarkeit

Die Listen müssen die Reihenfolge des Malers halten (Ebene, dann Einfügen): Ein durchscheinender Strich vor dem darunter gezeichnet, mischte falsch. Also kein atomarer Zähler je Liste wie in `render3d`, sondern eine stabile Verdichtung in drei Durchgängen ([`cull.wgsl`](../lib/render2d/cull.wgsl)):

| Durchgang | Was |
| --- | --- |
| `count` | eine Arbeitsgruppe je Seite der sichtbaren Ebenen: jedes Record gegen das Ziel, ein Bit je Record, die Zahl der Seite |
| `scan` | eine Arbeitsgruppe: die Präfixsumme der Zahlen in Seitenreihenfolge, je sichtbare Ebene erster Platz und Zahl und ihr indirekter Befehl: 6 Indizes je Viereck, höchstens 16 384 Vierecke je Instanz (`u16`-Indizes), so viele Instanzen, wie das braucht; auf Voll trägt `first_instance` die Ebene (`ebene << 16`) |
| `write` | jedes sichtbare Record an seinen Platz: der Platz der Seite plus die Bits davor |

Der Test eines Records: der Kasten um sein Segment, geweitet um die Reichweite des Strichs (halbe Breite mal der Schranke seiner Ecke, wo ein Nachbar ist, sonst die halbe Breite; ein Pixel für die Kante) und um das, was `f32` ferne Punkte runden kann, gegen das Rechteck des Ziels mit getrennten Achsen (x, y, die Richtung des Segments und ihre Normale). Ein Stück eines Bogens: der Kasten um seine Sehne (der Anfang aus dem Record davor), dazu um seine Pfeilhöhe geweitet (wie weit es über die Sehne hinausragt), bei eckigen Enden um weitere 0,18 halbe Breiten: Das Rechteck des Endes folgt der Tangente, die bis zu π/16 von der Sehne abweicht, seine Ecke reicht h (cos + sin) davon (der Test `visibility` legt diese Ecke allein ins Ziel). Freie Records und die Köpfe der Bögen bestehen nie, ohne Culling jedes lebende. Basis bekommt die Ebene über einen Push-Wert (`indirect-first-instance` ist in WebGPU und Vulkan 1.1 nicht sicher da), Voll über `first_instance`; derselbe Shader für beide.

**Prüfung** (`visibility`, beide Stufen): vier zufällige Szenen (bis 20 000 Polylinien, jede sechste Entität ein Bogen oder Kreis, gedreht, bis 1e6 Einheiten groß) gegen eine genaue Rechnung auf der CPU in `f64`: 0 fehlend, 0 zu viel, 0 doppelt, 0 außer der Reihe (Records, die die CPU auf einen Hauch an einer Kante sieht, dürfen beides). Haarlinien, die eine Kante berühren, sind drin; 0,05 Pixel dahinter sind sie es nicht. Mit und ohne Culling: 0 Pixel anders.

## Farben

Farben sind sRGB-Codes mit geradem Alpha (`0xAARRGGBB`), gemischt im Code-Raum, wie `lib/gfx` mischt (vormultipliziert, `Blend::PREMULTIPLIED`). Das Ziel hält Codes: ein Unorm-Format wie `Bgra8Unorm` der UI; ein sRGB-Format verweigert `Renderer::new`, es würde die Codes noch einmal kodieren.

## Tests

`render2dtest --test NAME` (`build_rs.py test render2dtest --target win` führt die `.uis` aus): auf jeder Stufe, die die GPU hat (RTX 2070 SUPER: Voll und Basis). Die Bilder entstehen in einem Ziel ohne Fenster und werden gegen dieselbe Zeichnung über `lib/gfx` verglichen (`dash_stroke` mit leerem Muster, nicht-null gefüllt mit genauer Flächendeckung, gemischt in sRGB-Codes; Bögen als ihr genauer Umriss, Muster als ihre Striche, je für sich: [Bögen und Kreise](#bögen-und-kreise), [Strichmuster](#strichmuster)).

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
| `churn` | mehrere zufällige Änderungen zufälliger Entitäten zwischen zwei Bildern (festgelegte Saat: einfügen, ändern, zweimal wachsen, wachsen und gleich entfernen, Stile) | jede Runde Byte für Byte wie neu gebaut |
| `arcs` | Bögen und Kreise: stumpf, rund, quadratisch, Breite 1, 3, 9, Haarlinien, Kreise von 0,8 bis 60 px Radius, Bögen über mehrere Stücke; durchscheinende Ringe; die Pixel der Vierecke auf der CPU nachgerechnet; der Wechsel der Pipeline, wenn eine Ebene einen Bogen bekommt; durchgehende Striche in einer Ebene von `fs_full` gegen `fs` | [Bögen und Kreise](#bögen-und-kreise), [Nicht gebaut](#nicht-gebaut-nicht-geprüft) |
| `dashes` | Strichmuster auf Linien, Polylinien (offen, geschlossen), Bögen, Kreisen; Einheiten, Maßstäbe, Drehungen; Flächenton; `set_style` an und aus; die Stücke der Referenz gegen die Striche von `lib/gfx` | [Strichmuster](#strichmuster) |
| `phase` | der Platz des Musters bei 1e-6 Einheiten je Pixel und 1e7 Länge, sieben Fälle, 24 Pans | höchstens 0,045 px |
| `short` | viele kurze Segmente mit scharfen Knicken (Kacheln, Vereinigungen, sich kreuzende Halbierende) gegen `lib/gfx` | Tinte innerhalb 6 % (diese Bilder falten sich: `lib/gfx` summiert Umläufe, `render2d` vereinigt; hier zählt vor allem, dass es läuft und Voll und Basis gleich sind; die Geometrie prüfen die anderen Bilder) |
| `visibility` | oben | genau |
| `coords` | oben, dazu Bögen, Kreise und Muster fern und Ringe im Tiefzoom | 0 Pixel, 0,0039 px, Ringe 0,0073 px |
| `levels` | Voll gegen Basis | Byte für Byte gleich |
| `errors` | jede `Invalid`-Prüfung | wie beschrieben |
| `validate` | alles mit Validierungsschicht und Synchronisationsprüfung | 0 Fehler, 0 Warnungen |
| `window` | die Demo im echten Fenster (ohne Aktivierung, ohne echte Eingabe): erstes Bild, Ziehen, Rad, jedes Mal die Ansicht im Framebuffer gegen `render2d` ohne Fenster | 0 Pixel anders; das Bild verschiebt sich mit dem Ziehen |
| `demo` | dieselbe Demo ohne Fenster über das Skript: Ziehen und Rad bewegen die Zeichnung | Pixel an den erwarteten Stellen |
| `bench_smoke` | die Messung, klein | läuft |
| `bench_cad_smoke` | die Messung mit der Messszene, klein | läuft |
| Browser (`engine_web.mjs`, kein `.uis`: läuft mit Node, Playwright) | [Browser](#browser) | oben |

22 von 22 Skripten bestehen (551 Prüfungen in `--test all` je Lauf, dazu Validierung und Fenster; G6a2a: 19 Skripte, 441 Prüfungen). Die 90 Bilder, die es vor G6a2b gab, sind Byte für Byte gleich geblieben (`--dump` gegen den Stand davor). In G6a2a lief `--test all` auch unter Linux auf Intel HD 610 (Fedora 44, Mesa 26), Intel Iris Xe und NVIDIA T500 (Kubuntu 26.04, Mesa 26 und Treiber 595): alle bestehen, ohne Pixelabweichung an den Prüfungen (Koordinaten 0 Pixel, 0,0040 px, Überlappung 0, Voll gegen Basis 0): die Besitzregel und die `exact`-Pässe halten auf allen vier GPUs. **G6a2b lief dort nicht**: Die Fedora-Maschine und das ThinkPad waren am 2026-11-11 um 20:45 und 21:41 nicht erreichbar (`Zielhost nicht erreichbar`, Port 22 ohne Antwort); Bögen, Muster, `opq` und die zweite Pipeline sind also auf Intel und der T500 ungeprüft.

**Toleranzen** ([`refimg.rs`](../apps/render2dtest/refimg.rs), `Tol`): beide Seiten rechnen die genaue Fläche des Pixels im Polygon; `lib/gfx` hält sie als Byte und mischt ganzzahlig, die GPU in Gleitkomma: 1 bis 2 Codes (`STRAIGHT`). Kreise: Die GPU schneidet sie an der Tangente, `lib/gfx` zeichnet ein Vieleck, dessen Kanten bis 0,05 Pixel innen liegen: bis 13 Codes entlang jedes Bogens, mehr bei Kreisen von einem halben Pixel Radius (`round`). Innere Ecken: Der Umriss von `lib/gfx` dreht sich durch die Ecke, und seine Nicht-null-Füllung summiert die Umläufe eines Pixels, bevor sie begrenzt; wo der Umriss sich selbst überlappt, kommt ein Anteil zu hoch heraus, bis voll. Die GPU rechnet die Vereinigung; nachgeprüft Pixel für Pixel gegen den genauen Anteil einer überabgetasteten Vereinigung (GPU 0,6026, genau 0,6025, `lib/gfx` 1,0) (`corners`). Zufällige Polylinien, die auf sich zurückfalten, tun das an vielen Stellen, dazu mischen ihre Stücke, die sich fern einer Ecke treffen, auf der GPU zweimal, in `lib/gfx` einmal (`folded`). Je Vergleich also: fast jedes Pixel innerhalb `small` Codes, eine begrenzte Zahl darüber, keines über `max`, die Tinte (Summe der Abstände vom Hintergrund) innerhalb `ink`.

## Messung

Die Messszene ([`cad.rs`](../apps/render2dtest/cad.rs), `render2dtest --bench 1000000 --scene cad`, feste Saat): ein Stadtplan in Metern bei Gauß-Krüger-Koordinaten (500 000 / 5 400 000): Straßen als lange Polylinien, Grundstücke, Gebäude mit doppelten Wänden, Innenwänden, Türen, Fenstern, Treppen, Möbeln, Schraffuren, Bäume, Beschriftung aus kurzen Strichen, lange Leitungen, Höhenlinien, 3100 × 1720 m. Seit G6a2b dazu: Türen schwenken in Viertelbögen, Bäume sind Kreise, Stützen in einem Viertel der Gebäude, Schächte alle 35 m an den Straßen, verdeckte Kanten (Vordächer, in der Hälfte der Gebäude) gestrichelt in Pixeln (5, 3), Straßenachsen strichpunktiert in Metern (6, 1,5, 0,8, 1,5), Grundstücksgrenzen strichpunktiert in Pixeln (9, 3, 2, 3); jedes auf eigenen Ebenen, wie eine CAD-Zeichnung sie hält. 464 086 Entitäten, davon 35 671 Bögen und Kreise (273 004 Stücke) und 8257 mit Muster (33 651 Records), genau 1 Mio. gezeichnete Records (dazu 35 671 Köpfe). `--plain`: die Zeichnung von G6a2a, Entität für Entität (461 703 Entitäten, 1 Mio. Segmente). Drei Ansichten bei 1920 × 1080: die Übersicht (alles im Bild, 1 000 000 Records; `--plain` 995 737), ein Stadtviertel (ein Sechzehntel der Fläche, 71 000; 67 000), die Arbeitsansicht (10 cm je Pixel, 4960; 4400). Hairlines (`styled`=nein) und die Breiten eines Plans (Straßen 3 px, Wände in Metern, Beschriftung rund). GPU-Zeiten aus den Zeitstempeln, Median über die Bilder.

**G6a2b, vorher und nachher** (RTX 2070 SUPER, Windows, 2026-11-11; drei Läufe abwechselnd, je 30 Bilder, der Median der Läufe; ms je Bild): „vorher“ ist `render2dtest` vom Stand vor G6a2b (21880797) mit der Zeichnung von G6a2a, „ohne“ dieser Stand mit `--plain` (dieselbe Zeichnung, nur durchgehende Striche: was die Änderungen an Records, Vertex-Shader und Pipelines kosten), „mit“ die Zeichnung mit Bögen und Mustern.

| Stufe, Striche | Übersicht: vorher / ohne / mit | Stadtviertel: ebenso | Arbeitsansicht: ebenso |
| --- | --- | --- | --- |
| Voll, Haarlinien | 9,440 / 9,474 / **12,878** | 1,764 / 1,757 / 2,189 | 0,431 / 0,434 / 0,595 |
| Voll, Plan-Breiten | 9,800 / 9,834 / **13,231** | 1,933 / 1,930 / 2,285 | 0,478 / 0,478 / 0,619 |
| Basis, Haarlinien | 9,443 / 9,472 / **12,882** | 1,761 / 1,757 / 2,189 | 0,434 / 0,434 / 0,586 |
| Basis, Plan-Breiten | 9,796 / 9,845 / **13,234** | 1,938 / 1,933 / 2,284 | 0,478 / 0,478 / 0,625 |
| Browser (Chromium 157, D3D12, RTX; Wanduhr; Median, in Klammern der schnellste Lauf), Haarlinien | 13,13 (10,91) / 13,60 (12,72) / 18,83 (16,79) | an der Grenze des Wartens: 4,04 / 4,04 / 4,24 | 4,04 / 4,04 / 4,04 |
| Browser, Plan-Breiten | 13,52 (11,72) / 14,54 (13,03) / 18,18 (15,75) | 4,04 / 4,04 / 4,43 | 4,04 / 4,04 / 4,25 |

Durchgehende Striche: **+0,3 bis +0,5 %** in der Übersicht (Voll 9,440 → 9,474 und 9,800 → 9,834, Basis 9,443 → 9,472 und 9,796 → 9,845), im Stadtviertel und in der Arbeitsansicht innerhalb ±0,6 %. Die Sichtbarkeit 0,196 → 0,198 ms. **Im Browser ist das nicht belastbar gemessen**: Während der Läufe im Browser (fünf bis sechs je Variante, abwechselnd) lief auf der GPU etwas anderes (30 % Last ohne eigene Arbeit, der Takt zwischen 1170 und 1605 MHz); derselbe Stand streut von 10,9 bis 14,1 ms, und ein zweiter Lauf auf Vulkan zur selben Zeit gab für den Stand vor G6a2b 13,2 statt 9,4 ms. Nach dem Median sind durchgehende Striche im Browser 4 bis 8 % langsamer, nach dem schnellsten Lauf 11 bis 17 %; ob das echt ist, lässt sich so nicht sagen. Eine Vermutung (Dawn gibt dem Fragment-Shader der durchgehenden Pipeline alle Werte des Vertex-Shaders mit, auch die der Muster) gebaut und gemessen (`fs` mit eigener, kleinerer Eingabe): kein Unterschied über dem Rauschen, verworfen. Die Zeichnung mit Bögen und Mustern kostet im Browser in der Übersicht +25 bis +38 % (Median) gegen dieselbe ohne. Die Zeichnung mit Bögen und Mustern kostet in der Übersicht **+36 %** gegen dieselbe ohne (12,88 statt 9,47 ms), im Stadtviertel +25 %, in der Arbeitsansicht +37 %; dort sind es auch 12 % mehr Records. Woher: Die Bögen sind 27 % der Records, und ihre Vierecke decken mehr als ihren Ring (oben: das 1,5- bis 4,4-fache bei Kreisen, die im Bild ganz liegen); jedes Pixel eines Bogens rechnet den Abstand vom Kreis statt der Geraden; die Ebenen mit Bögen und Mustern laufen über `fs_full`, ein längeres Programm. Erstes Bild: `prepare` 361 bis 393 ms CPU statt 247 bis 362 (streut stark), 63,2 statt 61,1 MB hoch (die Köpfe der Bögen).

Davor, G6a2a (die Zeichnung ohne Bögen und Muster), GPU je Bild, Haarlinien / Plan-Breiten (ms; Voll und Basis gleich, außer wo genannt):

| GPU | Übersicht | Stadtviertel | Arbeitsansicht |
| --- | --- | --- | --- |
| RTX 2070 SUPER (Windows) | **11,8** / 10,2 | 1,75 / 1,9 | 0,43 / 0,48 |
| NVIDIA T500 (Kubuntu) | 31,2 / 32,2 | 6,3 / 6,8 | 2,2 / 2,3 |
| Iris Xe (Kubuntu, Mesa) | 60,7 / 62,3 | 9,2 / 10,2 | 2,4 / 2,6 |
| HD 610 (Fedora, Mesa) | 348 / 358 | 50,6 / 55,8 | 12,5 / 13,5 |
| Browser (Chromium 157, D3D12, RTX, Haarlinien / Plan-Breiten; Wanduhr über 30 Bilder, drei voraus) | 11,1 / 11,7 (die Haarlinien an der Grenze des Wartens: ≥ 90 Bilder/s) | 11,1 / 11,1 (dieselbe Grenze) | 11,1 / 11,1 (dieselbe Grenze) |

(Die Sichtbarkeit davon: RTX 0,2 ms, Iris 2 ms, T500 1,5 ms, HD 610 7,5 ms. Die RTX misst heute für denselben Stand 9,4 statt 11,8 ms in der Übersicht der Haarlinien, warum, ist nicht untersucht; deshalb sind vorher und nachher oben am selben Tag abwechselnd gemessen.)

Vorher (G6a1, die Rechnung nur genau): RTX Übersicht 25,4 / 28,8 ms, Stadtviertel 2,0 / 2,5; Iris Xe 185 / 205, 12,5 / 13,8; HD 610 1067 / 1195, 77 / 86; T500 nicht gemessen.

**Messziel (1 Mio. Segmente im Bildtakt auf RTX 2070 SUPER und Iris Xe, mindestens 30 Bilder/s auf der HD 610 und im Browser).** RTX: erreicht (11,8 ms für alle Segmente im Bild, 60 Hz und 120 Hz). Browser auf der RTX: erreicht (86 Bilder/s, die Übersicht). **Iris Xe: bei der Übersicht verfehlt** (60,7 ms, also 3,6-fach über 16,7 ms; aus Sicht einer Arbeitsansicht und eines Stadtviertels erreicht: 9,2 und 2,4 ms). **HD 610: verfehlt** (348 ms = 2,9 Bilder/s, 10,5-fach über 33 ms; ein Stadtviertel 50,6 ms, 1,5-fach; die Arbeitsansicht 12,5 ms und 13,5 erreichen die 30 Bilder/s). Text kommt erst mit G6c dazu.

Woher die Zeit (RTX, Übersicht, Haarlinien, vor den Änderungen 25,5 ms): fester Farbwert im Fragment-Shader 1,3 ms (Vertex-Shader und Rasterung), Rumpf und Besitzregel 1,9 ms: der Fragment-Shader nahe der Enden kostete über 90 %. `discard` ließ das Programm weiterlaufen, 3 ms davon; das Zusammenziehen der Stücke über Polygone statt Halbebenen (`area6`) brachte die RTX auf 16 ms, machte aber Iris Xe langsamer (204 statt 185 ms) und ist deshalb nicht gebaut. Geholfen hat der [schnelle Weg](#der-shader) für kurze dünne Striche (bis 3 Pixel, halbe Breite bis 0,75). Dabei ändern sich Pixel: Die Übersicht (Haarlinien, 1 Mio.) in 541 972 von 2,07 Mio. Pixeln, 8 Codes im Mittel dieser (die Tinte +0,6 %), 30 090 Pixel über 28 Codes, höchstens 200; das Stadtviertel 55 486 Pixel (1,2 %), 2 048 Pixel in der Arbeitsansicht (0,1 %, dort Türen, Beschriftung: kurze Segmente auch nah), `short`-Testbilder 300 von 76 800 bei Linienbreite bis 1 Pixel, höchstens 179 Codes; Bilder mit längeren Segmenten (alle Prüfungen gegen `lib/gfx`, Überlappung, Koordinaten, Sichtbarkeit) unverändert, 0 Codes Unterschied. Ein weiterer Schritt für die HD 610 und die Iris (nicht gebaut): Segmente unterhalb eines Pixels gar nicht erst zeichnen (Detailgrad, Entitäten zu Punkten), und der Vertex-Shader. Ohne Detailstufe bleibt die Übersicht von 1 Mio. Segmenten auf der HD 610 bei 6 Bildern/s in der Regel und die Grenze des Fragment-Shaders der kurzen Striche liegt bei 36 ms für eine einzige feste Farbe (`fs_solid`, Vertex-Shader und Rasterung allein).

Alte Messung (G6a1, dichter Zufallsweg, schlimmster Fall, RTX): 61,6 / 118,9 ms (Haarlinien / 3 px, runde Ecken), jetzt 56,5 / 112,7 ms; ein Zehntel im Bild 6,5 / 18,0 ms. Erstes Bild: `prepare` 260 ms CPU (61 MB in die Upload-Puffer), 1000 Entitäten je Bild geändert: 6 ms CPU, GPU 0,2 ms.

## Nicht gebaut, nicht geprüft

- **Detailgrad** (Entscheidung 32), Kurven, Flächen, Schraffuren, Blöcke (G6b), Text (G6c), Überlagerung und Auswahl (G6d), der CPU-Weg und DXF (G6e, auch die Ausbuchtungen der DXF-Polylinien).
- **Gelaufen** auf RTX 2070 SUPER (Windows, Vulkan), T500, Iris Xe, HD 610 (Linux, Vulkan) und im Browser (Chromium 157, D3D12 und SwiftShader): oben. **Bögen und Muster (G6a2b) nur auf der RTX** (Vulkan Voll und Basis, Browser über D3D12): Die beiden Linux-Maschinen waren nicht erreichbar, SwiftShader ist nicht wieder gelaufen. Nicht geprüft: Android (gebaut, nicht gelaufen), AMD, Mac/iOS (Metal), Safari, Firefox, Browser auf Intel oder über Dawn-Vulkan, Treiber mit NVIDIA unter Linux im Browser. Ob andere Treiber die Rechnungen des Fragment-Shaders ebenso Bit für Bit gleich machen, wie die Besitzregel es braucht, ist nur für die genannten GPUs gezeigt.
- **Selbstkreuzungen fern einer Ecke** mischen zweimal (oben).
- **Die Näherung an kurzen Segmenten** zwischen zwei scharfen Ecken (oben).
- **Sehr viele Ebenen**: höchstens 65 535 sichtbare; jede Ebene belegt mindestens eine Seite (256 Records) im Pool.
- **Breiten unter 1 Pixel** werden als 1 Pixel gezeichnet, nicht dünner und blasser.
- **Leistung** bei großer Überdeckung und auf schwachen GPUs (oben); das Schreiben des Pools beim ersten Bild auf der CPU; Detailgrade für sehr kleine Entitäten.
- **Der schnelle Weg** ändert kurze Stücke nahe ihrer Enden (oben); nicht zuschaltbar (noch: `TINY_LEN`, `TINY_H` im Shader). In `fs` unverändert; `fs_full` hat ihn für durchgehende Segmente ebenso, nicht für Bögen und Muster.
- **Höchstens drei Striche je Seite und Pixel**: Liegen mehr Striche eines Musters in Reichweite eines Pixels (1,42 px, bei runden und quadratischen Kappen plus die Breite), fehlen die übrigen. Das trifft breite Striche mit solchen Kappen und kurzen Mustern über dem Flächenton (etwa 10 px breit, runde Kappen, ein Strich alle 3 px); nicht gemessen, wie es dann aussieht.
- **Kappen über eine Ecke hinweg**: Ist das Muster an einer Ecke aus, zeichnet jedes Segment seine Striche und Kappen für sich; überlappen sie sich jenseits der Ecke (scharfe Knicke, kurze Lücken), addieren sich ihre Anteile statt sich zu vereinigen (etwas zu viel Tinte, nie ein zweites Mischen: das Pixel hat einen Besitzer).
- **Die Überlappung zweier Striche innen an einer Ecke** wird an ihren Enden geschnitten, ohne die runden Kappen sehr kurzer Striche dort.
- **Sehr kurze Bogenstücke**: Endet ein Bogen weniger als etwa 0,7 px (plus die Kappe) nach der Fuge seiner letzten zwei Stücke (nur kleine Bögen: ein Stück ist der Radius mal höchstens 22,5°), schneidet der Besitzer der Pixel vor der Fuge nicht am Ende des Bogens.
- **Kreise unter einem Pixel Radius** über die Tangente: bis 60 Codes gegen den genauen Umriss.
- **Die Pipeline je Ebene**: In einer Ebene mit Bögen oder Mustern zeichnet auch jede durchgehende Linie mit `fs_full`, nahe ihrer Ecken über die Vereinigung statt der Kacheln (ein längeres Programm). Ecken und Kappen kommen gleich heraus (Test `arcs`: 0 Pixel anders an Gehrung, Abschrägung und runden Ecken, 2 Pixel um 1 Code an runden Kappen); kurze Segmente mit scharfen Knicken nicht: Dort nehmen beide die Vereinigung, mit anderen Näherungen, wo ein kurzes Segment beiden Ecken nahe ist (Bilder von `short`: Breite 1 39 Pixel anders, 28 um mehr als 2 Codes, höchstens 80; Breite 3 144 Pixel, 116, höchstens 196; gegen `lib/gfx` ist `fs_full` dabei nicht weiter weg: Tinte −4,12 statt −4,21 % und −0,21 statt −0,56 %). Wer schnelle Ebenen will, legt Bögen und Muster auf eigene Ebenen, wie die Messszene.
- **Geschlossene Polylinien mit Muster** verbinden Anfang und Ende wie Chromium (Skia), wenn das Muster dort an ist; `lib/gfx` setzt zwei Kappen (SVG lässt es offen).
- **Große Kreise, die das Bild schneiden**, decken bis zum 9-fachen ihres Rings im Bild (Kreis von 5000 px Radius: 51 158 Pixel, ohne den Ausschnitt wären es Vierecke über das ganze Bild). Viele große Kreise übereinander (Zoom in einen Lageplan voller Kreise) sind nicht gemessen.
- **Die Vierecke der Bögen** folgen dem Ring mit festen 16 Stücken je Kreis, unabhängig vom Zoom; ein Kreis von 540 px Radius lässt den Fragment-Shader das 4,4-fache seines Rings ansehen.

## Entscheidungen in G6a2b

- **Das Record bleibt 64 Byte**: Es hält nicht mehr die Vektoren zu den Nachbarpunkten (der Vertex-Shader liest die eigenen Vektoren der Nachbar-Records), der Platz geht an die Länge entlang der Entität (`f64` als drei `f32`). Kein Puffer daneben. Kosten für durchgehende Striche: +0,3 bis +0,5 % (oben).
- **Das Muster steht im Stil** (128 statt 32 Byte: die Enden der Einträge, κ, Phase, Periode, Anteil); Muster in Pixeln gehen bei jeder Änderung des Maßstabs mit allen Stilen neu hoch, nicht je Record.
- **Ein Bogen sind feste Stücke** von höchstens π/8 mit einem Kopf davor, unabhängig vom Zoom (ein Record ist ein Viereck); die Vierecke zwischen Sehne innen und Tangente außen, ab dem halben Bild der Ausschnitt um F. Der Radius liegt als hi/lo im Record, Mitte und Ende als Punkte wie bei Segmenten.
- **Zwei Pipelines, je Ebene gewählt** statt eines Shaders mit allem (der Treiber setzte `area6` bei zwei Aufrufen nicht mehr ein: dreifache Zeit); die Messszene legt Bögen und Muster auf eigene Ebenen.
- **Winkel und Doppel-`f32` gegen den Treiber geschützt**: Reihen für sin, cos, atan2 und asin bei kleinen Winkeln, `opq` in den Zwei-Summen, Zwei-Produkte nach Dekker statt `fma`. Das gilt auch für die Linien (deren Bilder Byte für Byte gleich blieben).
- **Wo das Muster beginnt**: am Anfang jeder Entität, eine Polylinie durchgehend über ihre Ecken und einmal herum, wenn sie geschlossen ist, ein Bogen bei `start`, ein Kreis bei 0, gegen den Uhrzeigersinn. Eine geschlossene Polylinie, die im ersten Punkt beidseits an ist, verbindet sich dort wie bei Chromium (Skia), nicht mit zwei Kappen wie `lib/gfx`.
- **Quadratische Kappen an Bögen** sind gerade Rechtecke entlang der Richtung am Ende (wie SVG), nicht gebogen.
- **Flächenton** ab einer Periode von 2 px, genau ab 4 px, dazwischen gemischt; der Anteil zählt die Fläche der Kappen mit.
- **Die Referenz für Bögen** ist ihr genauer Umriss, gefüllt von `lib/gfx`, nicht `dash_stroke` (das innen 0,25 px zu breit zeichnet); Muster als Striche aus `dash_pieces` (SVGs Regeln, gegen `dash_stroke` geprüft), jeder Strich über `lib/gfx`.
- **Der schnelle Weg für kurze dünne Striche** bleibt, wie er war; in `fs_full` gilt er für durchgehende Segmente ebenso.

## Entscheidungen in G6a2a

- **Kurze dünne Striche nicht auf die genaue Fläche**: bis 3 Pixel Länge und 0,75 Halbbreite ein Produkt von Anteilen (oben); Längere bleiben genau, die Prüfungen gegen `lib/gfx` unverändert. Das Messziel ohne diesen Schritt: Iris Xe 162,7 ms statt 60,7 ms.
- **`area6` per Polygon-Kantenintegral** gebaut und verworfen: RTX 25,5 → 16 ms bei nahezu gleichen Bildern, Iris Xe 185 → 204 ms (das Programm des Fragment-Shaders wurde länger; dort zählt das zuerst).
- **Die Grenzen der Messung im Browser** (Zeit zwischen Bildern, drei Bilder voraus, Warten über `is_done`): siehe oben; bis auf die Übersicht nur eine Untergrenze.

## Entscheidungen in G6a1

Die Vorgaben (eigene Crate, Szene mit Ids, analytische Linien, `f64` als hi/lo, Sichtbarkeit auf der GPU) kommen aus dem Plan für G6 (Entscheidungen 23 bis 31). Dazu hier getroffen:

- **Kein Tiefen- und kein Stencil-Puffer**: die Besitzregel an Ecken statt einer Tiefe; `prepare` bekommt die Größe des Ziels (`prepare(&mut renderer, (b, h))`), der Renderer hält keine Größe.
- **Stile gehören der Szene**, nicht dem Renderer; Farben als `u32` in `0xAARRGGBB` (sRGB, gerades Alpha); sRGB-Ziele werden verweigert.
- **Mindestens 1 Pixel breit**, runde Ecken als Sektor (wie SVG und `lib/gfx`).
- **Vertex- und Sichtbarkeits-Shader `exact` übersetzt** (oben).
- **Zufällige Szenen im Test `changes` deckend**: Durchscheinendes an Selbstkreuzungen mischt auf der GPU zweimal, in `lib/gfx` einmal; das prüft `overlap` gezielt.
