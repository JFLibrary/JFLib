# RENDER3D -- der Kern der 3D-Engine

`lib/render3d` zeichnet Szenen aus sehr vielen Instanzen über [`lib/gpu`](GPU.md): Meshes in gemeinsamen Puffern, Materialien als Tabelle, Instanzen mit Position in `f64`, Sichtbarkeit im Rechen-Shader, eine Instanzliste je Mesh. Kein Befehl je Objekt, nirgends: Ein Bild sind ein Dispatch und ein indirekter Zeichenbefehl je Mesh (Stufe Basis) oder ein Multi-Draw für alle (Stufe Voll). Dieselben WGSL-Quellen laufen auf Vulkan Voll, auf Vulkan Basis und im Browser über WebGPU.

Stand: G5a, der Kern. Texturen und glTF (G5b), PBR, Licht, Schatten (G5c), Hi-Z, Detailstufen, zweite Warteschlange (G5d), die 3D-Ansicht in der UI, Linux, Android, andere GPUs (G5e) kommen danach ([Roadmap](ROADMAP.md)). Beleuchtung heute: Lambert mit einer Sonne und einem Grundlicht.

| Teil | Wo |
| --- | --- |
| `Renderer`, Pools, Materialien, Aufzeichnen | [`lib/render3d/lib.rs`](../lib/render3d/lib.rs) |
| `Scene`, Instanzen, Kamera, Hochladen | [`lib/render3d/scene.rs`](../lib/render3d/scene.rs) |
| Shader | [`cull.wgsl`](../lib/render3d/cull.wgsl), [`scatter.wgsl`](../lib/render3d/scatter.wgsl), [`draw.wgsl`](../lib/render3d/draw.wgsl), vorübersetzt in [`shaders_spv.rs`](../lib/render3d/shaders_spv.rs) |
| Tests und Messung | [`apps/render3dtest`](../apps/render3dtest/main.rs), die Referenz auf der CPU: [`cpu.rs`](../apps/render3dtest/cpu.rs); im Browser: [`tests/engine_web.mjs`](../apps/render3dtest/tests/engine_web.mjs) |

## Ein Beispiel

```rust
let mut r = render3d::Renderer::new(&dev, &render3d::RendererDesc::new(gpu::Format::Rgba8Unorm))?;
let mesh = r.add_mesh(&render3d::MeshData { positions: &pos, normals: &nrm, uvs: &uv, indices: &idx })?;
let rot = r.add_material(&render3d::Material::color(0.8, 0.1, 0.1))?;
r.set_size(1920, 1080)?;                        // ein Tiefenpuffer, den der Renderer hält

let mut scene = render3d::Scene::new(&r)?;
let cube = scene.add(render3d::Instance { mesh, material: rot, transform: render3d::Transform::at([1e7, 0.0, 1e7]) })?;
scene.set_origin([1e7, 0.0, 1e7]);              // (0, 0, 0) auf der GPU; nahe der Kamera halten
scene.set_camera(&render3d::Camera::look_at([1e7 + 5.0, 2.0, 1e7], [1e7, 0.0, 1e7], [0.0, 1.0, 0.0],
                                            1.0, 16.0 / 9.0, 0.1, f64::INFINITY));

// je Bild
scene.prepare(&mut r)?;                         // wartet auf das letzte Bild dieses Platzes, schickt, was sich änderte
let mut e = dev.encoder()?;
r.record(&mut e, &scene, color_attachment, None)?;      // None: der Tiefenpuffer des Renderers
let id = dev.queue().submit([e.finish()?])?;    // oder swapchain.present(frame, [e.finish()?])
scene.submitted(id);
```

## API-Überblick

- **Renderer**: `Renderer::new(&device, &RendererDesc { color, frames_in_flight })` (1 bis 3 Bilder in Arbeit, `RendererDesc::new`: 2), `add_mesh(&MeshData)`, `mesh_info(id)`, `add_material(&Material)`, `set_material(id, &Material)`, `set_size(b, h)`, `record(&mut encoder, &scene, farbe, tiefe)`, `multi_draw()`, `max_instances()`.
- **Scene**: `Scene::new(&renderer)`, `add(Instance)`, `remove(id)`, `set_transform`, `transform`, `set_material`, `set_mesh`, `len`, `set_camera`, `set_sun`, `set_origin`, `prepare(&mut renderer)`, `submitted(id)`, `wait_slot()`.
- **Werte**: `Transform { pos: [f64; 3], rot: [f32; 4], scale: [f32; 3] }`, `Camera` (`look_at`; Sichtfeld, Seitenverhältnis, near, far, `f64::INFINITY` heißt keine ferne Ebene), `Sun { direction, color, ambient }`, `Material { base_color, metallic, roughness, emissive }`.
- **Zum Messen und Testen**: `set_stats(true)`, `take_stats()`, `collect_stats()` (`FrameStats`: sichtbare Instanzen, Zeitstempel), `upload_info()` (`UploadInfo`: was `prepare` geschickt hat), `lists()`, `draws_buffer()`, `visible_buffer()`, `set_culling(false)`.

Konventionen: Reverse-Z (Tiefe 1 an der nahen Ebene, `Compare::Greater`, Löschen mit `DEPTH_CLEAR`, also 0, immer `Depth32Float`), rechtshändige Welt, Vorderseiten gegen den Uhrzeigersinn, Farben linear (ein sRGB-Ziel kodiert sie). Fehler sind die von `lib/gpu`: `Invalid` für alles, was hier zu erkennen ist (tote Ids, negative Skalierung, ein falsches Ziel, `record` vor `prepare`). Eine Id eines Meshes oder Materials, das der Renderer nicht hat, fällt erst bei `prepare` auf.

## Aufbau

**Daten.**

| Was | Wie groß | Wer liest |
| --- | --- | --- |
| Ecken (ein Puffer je Renderer) | 32 Byte: `position_u`, `normal_v`, zwei `vec4f`, die UV in den `w` | Vertex-Shader |
| Indizes (zwei Puffer je Renderer) | 16 Bit bis 65 536 Ecken je Mesh, darüber 32 Bit | Grafik-Pipeline |
| Materialien | 64 Byte: Grundfarbe, emissive, metallic, roughness, vier Plätze für Texturen | Fragment-Shader |
| `spheres` | 16 Byte je Platz: Mittelpunkt relativ zum Ursprung, Radius | `cull` |
| `list_of` | 4 Byte je Platz: die Liste | `cull`, nur für Sichtbares |
| `instances` | 64 Byte je Platz: drei Zeilen der fertigen Matrix, Skalierung für die Normale, Material | Vertex-Shader |
| Sichtbare Liste, Zeichenbefehle | 4 Byte je Instanz, 20 Byte je Liste; je Ansicht | `cull` schreibt, Vertex-Shader und Grafik-Pipeline lesen |

Auf der CPU hält die Szene 80 Byte je Platz (`f64`-Position, Quaternion, Skalierung, Ids). Freie Plätze kommen auf eine Liste und werden zuerst wieder vergeben; ihr Radius ist `-f32::MAX`, sodass schon der erste Ebenentest sie verwirft und keine Shader-Zeile sie kennt. Eine Liste hält Platz für ihre Instanzen und verdoppelt ihn, wenn er nicht reicht, ihre Lage in der sichtbaren Liste ändert sich also selten. Kapazitäten schrumpfen nicht.

**Ein Bild** (`Renderer::record`):

| Schritt | Was geschieht |
| --- | --- |
| Barriere | gegen alle Zugriffe des Bildes davor |
| Kopien | Ansicht und Licht aus dem Upload-Puffer dieses Platzes; was sich an Materialien, Listen und (selten) an allen Instanzen geändert hat |
| `scatter` | geänderte Plätze, ein Dispatch |
| Vorlage | Zeichenbefehle mit Anzahl 0 aus einer Vorlage |
| `cull` | ein Dispatch über alle Plätze; je sichtbare Instanz ein `atomicAdd` auf die Anzahl ihrer Liste und ein Eintrag |
| Barriere, Durchgang | Voll: `draw_indexed_indirect_multi` über die Listen (je Indexbreite einer); Basis: je nicht leere Liste ein Push-Wert und ein `draw_indexed_indirect` |

**Entscheidungen und ihre Gründe.**

- **Vertex-Pulling statt fester Vertex-Eingaben.** Der erste Entwurf hatte Positionen, Normalen und die Instanznummer als Vertex-Attribute, wegen der einfachen Basis-Pfade. Er kostet in der Messszene rund drei Viertel mehr Zeichenzeit (Tabelle unten). Auf Kacheln rendernden Telefon-GPUs kann das anders aussehen: In G5e messen.
- **16-Bit-Indizes.** Der zweite Fund: Über Speicherpuffer kosten 32-Bit-Indizes auf der RTX 2070 SUPER bei gleichem Bild ein Drittel bis die Hälfte mehr als 16-Bit-Indizes (Tabelle unten; warum, ist nicht gesucht). Indizes sind relativ zur Basis-Ecke des Meshes, 16 Bit reichen also für jedes Mesh bis 65 536 Ecken. Lange Meshes gehen in einen zweiten Puffer; nebeneinanderliegende Listen gleicher Breite bleiben ein Multi-Draw.
- **Listen im selben Durchgang gefüllt.** Die CPU kennt, wie viele Instanzen ein Mesh hat, also auch den Platz seiner Liste. Damit reicht ein Durchgang, kein Zählen vorweg und keine Präfixsumme auf der GPU.
- **Die Liste beginnt bei Voll an `first_instance`, bei Basis über einen Push-Wert.** `indirect-first-instance` ist in WebGPU und Vulkan 1.1 nicht sicher da; so gibt es einen Shader für beide Wege. Ein Push-Wert kostet je Liste wenig, und im Browser sind es dynamische Offsets auf einen schon gebundenen Puffer.
- **`scatter` statt vieler Kopien.** k geänderte Plätze wären k Kopierbefehle oder k Aufrufe von `writeBuffer`, und `Queue::write_buffer` läuft vor den Befehlen des Bildes, ohne Barriere gegen Bilder, die noch auf der GPU liegen (die Synchronisationsprüfung würde das melden). Also landet alles im Upload-Puffer des Platzes (96 Byte je Änderung), ein Dispatch verteilt es. Ab 4096 Änderungen und einem Achtel aller Plätze geht alles über einen temporären Puffer (84 Byte je Platz).
- **Fertige Matrix je Instanz, Normale ohne Rechnung je Ecke.** Eine Transformation ohne Scherung braucht für die Normale nur `m * (n * normal_scale)` mit `normal_scale = 1 / s²` je Achse. Das ist die Inverse-Transponierte, ohne sie zu speichern.
- **Material im Fragment-Shader.** Ferne Würfel haben mehr Ecken als Pixel; die Grundfarbe je Ecke zu holen, wäre teurer. PBR (G5c) liest die Zeile ohnehin dort.
- **Große Koordinaten.** Auf der CPU `f64`, auf der GPU `f32` relativ zum Ursprung der Szene. Ein neuer Ursprung (`set_origin`) schickt alles einmal. Der Ursprung folgt der Kamera nicht von selbst: Wann das Hochladen sich lohnt, weiß das Programm.
- **Nichts Sichtbares darf fehlen.** `f32` rundet Kugeln und Ebenen. Beide werden deshalb um das geweitet, was das Runden kosten kann (Kugel: `2e-6` des Radius plus `5e-7` des größten Betrags ihres Mittelpunkts, Ebene ebenso). Das kostet fast nichts: Bei 1 Mio. Würfeln hielt die Liste im Prüflauf einen Eintrag mehr als `cull_ref` der Messszene (528 562 gegen 528 561, keiner fehlte).
- **Kein WGSL-Übersetzer im Programm.** Die Shader liegen wie die der UI vorübersetzt im Programm (SPIR-V 1.3, ohne Robustheit, für beide Stufen dieselben Wörter); `render3dtest --test shaders` vergleicht sie mit `wgsl::compile`, `--gen-shaders` schreibt sie neu. Der Browser bekommt den Text.

**Was die nächsten Stufen vorfinden.** Sichtbarkeit und Zeichnen nehmen eine `ViewLists` (eigener Uniform-Block, eigene Zeichenbefehle, eigene sichtbare Liste); Schatten-Kaskaden und die zweite Runde von Hi-Z sind weitere davon mit einer Pipeline ohne Fragment-Shader. Die erste `vec4f` einer Ecke reicht für reine Tiefendurchgänge. Das Material hat vier Plätze für Texturen (`u32::MAX`: keine), jede Ecke ihre UV.

## WebGPU

Es läuft dasselbe WGSL, `lib/gpu` macht `var<immediate>` zu einem Uniform-Block ([GPU.md](GPU.md#ausgabe-im-browser)). Das Bündel läuft in einem Worker und kann nicht warten: `Scene::wait_slot` wartet dort nicht, das ist in Ordnung, weil `writeBuffer` in der Reihenfolge der Warteschlange liegt.

| Grenze | Was daraus folgt |
| --- | --- |
| Speicherpuffer im Vertex-Shader: nur lesend | so gebaut: `cull` schreibt, `draw` liest über ein zweites Layout |
| Speicherpuffer je Stufe (mindestens 8) | `cull` 5, `scatter` 4, `draw` 3 im Vertex-, 1 im Fragment-Shader |
| Speicherpuffer im Vertex-Shader im Kompatibilitätsmodus (kann 0 sein) | geht nicht; ein Rückweg über Vertex-Puffer ist nicht gebaut |
| Bindungsgröße (mindestens 128 MiB, `gpu.js` verlangt, was der Adapter kann; hier 2047 MiB) | `Renderer::max_instances`: Bindung / 64 Byte, höchstens das, was ein Dispatch erreicht (hier 16,7 Mio.) |
| `indirect-first-instance` optional | wird nicht gebraucht |
| kein Multi-Draw | je Liste ein Zeichenbefehl |
| Zeitstempel fehlen | keine GPU-Zeiten im Browser |
| `writeBuffer` in Vielfachen von 4 Byte | 16-Bit-Indizes werden auf eine gerade Anzahl aufgefüllt |

## Tests

`python -u build_rs.py test render3dtest --target win` führt `apps/render3dtest/tests/*.uis` aus, alles offscreen außer `window.uis`, die meisten auf Voll und auf einem auf Basis begrenzten Gerät. Ohne GPU `SKIP`. Die Referenz ist unabhängig von der Engine: ein Strahlenwerfer in `f64` (eigene Wurzel, eigene Drehung über Quaternionen-Produkte, die Ebenen als Abstände im Kameraraum).

| Skript | Prüft | Ergebnis (RTX 2070 SUPER, Treiber 610.88) |
| --- | --- | --- |
| `pixels` | gedrehter Würfel, zwei Würfel in beiden Reihenfolgen, Kugel 2,5 × 1 × 0,5 mit glatten Normalen, Rückseiten von innen, ferne Ebene, Platten 1 auseinander in 100 000 Abstand mit near 0,01; Farbe je Pixel ±1 und die Werte des Tiefenpuffers (über einen Rechen-Shader gelesen) | 0 falsche Pixel; Tiefe: größter relativer Fehler 4,5 · 10⁻⁴, in 100 000 Abstand 6 · 10⁻⁸ |
| `visibility` | Zufallsszenen mit festem Seed (5000, 20 000, 60 000 Instanzen, 3 Meshes) gegen den exakten Test: nichts fehlt, nichts zu viel, nichts doppelt, keine falsche Liste; Kugeln genau an jeder der sechs Flächen (berührend: in der Liste, 0,01 dahinter: nicht); Bild mit Sichtbarkeit gegen Bild mit allem | 975, 2322 und 8820 sichtbar, Abweichung 0; 12 von 12 Kugeln richtig; 0 Pixel anders |
| `changes` | Hinzufügen, Verschieben, Entfernen, Plätze wieder vergeben, Material und Mesh ändern, eine Tabellenzeile; das Bild gegen eine neu aufgebaute Szene (in umgekehrter Reihenfolge); was hochgeht, in Byte; beide Wege des Hochladens; ein `prepare` ohne Bild | 0 Pixel anders; ein Bild ohne Änderung schickt 224 Byte, 200 Änderungen 19 424 Byte |
| `many` | 160 Meshes und 400 Materialien gegen den Strahlenwerfer; wachsende Pools; 16- und 32-Bit-Listen im Wechsel, ein Mesh mit ungerader Indexzahl | 0 falsche Pixel, 0 anders |
| `levels` | Voll gegen Basis, drei Bilder der Galerie (3000 Instanzen, 48 Meshes, 96 Materialien) | 0 von 57 600 Pixeln anders |
| `coords` | Ursprung und 1e7 mit gleicher relativer Kamera; dasselbe mit Positionen, die `f64` bei 1e7 runden muss; Ursprung nachträglich setzen; als Gegenprobe ohne Ursprung | 0 Pixel anders; ohne Ursprung 30 365 von 57 600 anders (der Test kann es also erkennen) |
| `frames` | sieben Bilder in Flug mit eigener Kamera und eigener Änderung gegen jedes einzeln gezeichnet; der Tiefenpuffer des Renderers | 0 Pixel anders |
| `errors`, `shaders` | was die API verweist; leere Szene; verlorenes Gerät; die eingebetteten Wörter | bestanden |
| `window` | ein echtes Fenster, sechs Bilder, das dargestellte Bild gegen dasselbe Bild offscreen, mit Validierung; ein einziger, kurzer Lauf | 0 Pixel anders |
| `validate`, `bench_smoke` | alles außer `window` unter der Validierungsschicht mit Synchronisationsprüfung; kurzer Messlauf mit Änderungen: die Listen enthalten alles aus `cull_ref` der Messszene | 0 Fehler, 0 Warnungen; 0 fehlen |

**Im Browser** ([`tests/engine_web.mjs`](../apps/render3dtest/tests/engine_web.mjs); Bündel bauen mit `python -u build_rs.py app render3dtest --target web`, dazu das Programm für Windows, das die Vergleichsbilder schreibt): Chrome 156 kopflos, alle drei Bündel, eigenes Profil. Die Galerie (die drei Bilder oben, eins davon an 1e7) gegen den Vulkan-Weg auf derselben Maschine: **0 von 57 600 Pixeln anders**, gegen Voll wie gegen Basis, in jedem Bündel; der Browser meldet keinen Validierungsfehler; die Zählung der Listen gleich dem exakten Test (2522). Im Browser gilt die Robustheit von WebGPU, auf Vulkan sind die Shader ohne übersetzt; am Bild ändert das nichts. Welches Backend Chrome unter WebGPU nimmt, ist nicht festgehalten, der Adapter heißt `nvidia turing`.

Die Referenz kennt drei Fälle: Ein Pixel, dessen Strahlen um 1/64 Pixel neben der Mitte dasselbe sehen, muss bis auf ±1 stimmen. Sehen sie Verschiedenes, ist es eine Kante und darf die Farbe eines dieser Objekte zeigen (die GPU rastert auf 1/256 Pixel genau, und ihre Normale liegt an der Kante knapp außerhalb des Dreiecks). Ein Pixel, das einen Treffer eines Nachbarpixels zeigt, wird als Splitter gezählt und darf nur selten sein. Zuerst schlug dieser Vergleich an der Kugel mit einem Pixel fehl: Die Kante lag dort, wo der Test nur exakte Farben der Nachbarstrahlen zuließ.

## Messwerte

Wie `gputest --bench` ([GPU.md](GPU.md#messszene)): GeForce RTX 2070 SUPER, Treiber 610.88, Ryzen 5 3600, Windows 11, Release-Build, ohne Validierung, offscreen, 1080p, 300 Bilder nach 60 zum Aufwärmen, 3 Bilder in Flug, normale Prozesspriorität. Dieselben Würfel (`move_all` zur Zeit 0), dieselbe Kamerafahrt mit Reverse-Z. Drei Läufe je Variante im Wechsel, gezeigt der Median der Mediane. Parallel lief eine andere Sitzung mit GPU-Tests (bei einem Blick 34 % GPU, 0 bis 1 `rustc`): Die 99. Perzentile lagen deshalb bei beiden Programmen bis 5 ms und werden hier nicht verglichen.

| Bild in ms | Würfel | CPU: aufzeichnen und übergeben | GPU: `move` bzw. Hochladen | GPU: Sichtbarkeit | GPU: Zeichnen | GPU gesamt (Spanne) | Bildabstand |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `gputest --bench` | 1 Mio. | 0,140 | 0,257 | 0,133 | 1,238 | 1,632 (1,631–1,638) | 1,702 |
| Engine | 1 Mio. | 0,124 | 0,002 | 0,063 | 1,241 | 1,306 (1,303–1,315) | 1,357 |
| `gputest --bench` | 100 000 | 0,062 | 0,033 | 0,016 | 0,234 | 0,284 (0,275–0,286) | 0,299 |
| Engine | 100 000 | 0,058 | 0,002 | 0,013 | 0,237 | 0,252 (0,249–0,252) | 0,264 |

Was die Engine gegenüber der nackten Messszene kostet:

- **Zeichnen: nichts Messbares** (1,241 gegen 1,238 ms). Die fertige Matrix spart das Quaternion je Ecke, doch das ist nicht der Engpass, der liegt laut G1 in der festen Funktion. Dafür liest der Vertex-Shader 64 statt 48 Byte je Instanz, und die Grundfarbe kommt je Fragment aus der Tabelle.
- **Sichtbarkeit: weniger als die Hälfte** (0,063 gegen 0,133 ms), weil 16 statt 48 Byte je Instanz gelesen werden. Dazu kommt je sichtbarer Instanz das Nachschlagen der Liste.
- **`move` entfällt**, die Würfel der Engine stehen still (Änderungen kommen von der CPU, unten). Die Engine ist deshalb 0,33 ms schneller je Bild; das ist kein fairer Vergleich mit einem Bild, in dem sich alles bewegt.
- **CPU**: gleich. `prepare` 0,003 ms, aufzeichnen 0,062 ms.
- **Speicher**: 1 Mio. Instanzen belegen 107 MiB Blöcke (einschließlich 16 MiB Bildziele) und 80 MB CPU-Speicher. Aufbau: 73 bis 112 ms für die Million `add`, das erste `prepare` 53 bis 61 ms.

Wie die Zeichenzeit zustande kommt (1 Mio. Würfel, `draw` in ms; Wegwerf-Änderungen an der Engine, je Zelle ein bis drei Läufe, mit 200 Bildern statt 300 bei 320 × 180):

| Vertex-Daten | Indizes | 320 × 180 | 1080p |
| --- | --- | ---: | ---: |
| Vertex-Attribute: 12 + 20 + 4 Byte in drei Puffern | 32 Bit | 2,18 | 2,23 |
| Vertex-Attribute: wie oben | 16 Bit | 2,03 | 2,21 |
| Speicherpuffer: 12 + 20 Byte, skalar gelesen | 32 Bit | 1,56 | 1,64 |
| Speicherpuffer: 32 Byte, zwei `vec4f` | 32 Bit | 1,78 | 1,66 |
| Speicherpuffer: 32 Byte, zwei `vec4f` | 16 Bit | 1,17 | 1,24 |
| zum Vergleich `gputest --bench` (Speicherpuffer, 16 Bit, Quaternion je Ecke) | 16 Bit | 1,18 | 1,24 |

Vorwärts-Z statt Reverse-Z machte keinen erkennbaren Unterschied (320 × 180, 32-Bit-Indizes: zwei Läufe 1,88 und 1,56 ms gegen 1,78 ms; die Streuung ist größer).

Änderungen und viele Listen (1 Mio. Würfel, 1080p; ein Satz von je drei Läufen, die andere Sitzung lief mit):

| Variante | CPU je Bild | GPU: Hochladen | GPU: Sichtbarkeit | GPU: Zeichnen (Spanne) |
| --- | ---: | ---: | ---: | ---: |
| eine Liste, Basis-Gerät | 0,134 | 0,002 | 0,063 | 1,237 |
| 10 000 Instanzen je Bild verschoben (`--moved`) | 5,08 | 0,163 | 0,066 | 1,234 |
| 100 000 Instanzen je Bild verschoben | 48,2 | 2,01 | 0,067 | 1,367 (1,24–1,53) |
| 1024 Listen, Voll (ein Multi-Draw) | 0,116 | 0,002 | 0,079 | 1,272 (1,27–1,68) |
| 1024 Listen, Basis (1024 Befehle) | 0,243 | 0,002 | 0,078 | 1,702 (1,27–2,70) |
| 16 384 Listen, Voll | 0,144 | 0,006 | 0,096 | 1,633 (1,33–1,75) |
| 16 384 Listen, Basis | 2,73 | 0,006 | 0,096 | 1,497 (1,50–1,51) |

- **Eine Änderung kostet rund 0,5 µs auf der CPU und 16 bis 20 ns auf der GPU.** Auf der CPU sind das vermutlich vor allem Cache-Fehlschläge: Die Stichprobe greift zufällig in 80 MB. Für eine Million bewegter Objekte taugt das nicht, dafür gehört die Bewegung in einen eigenen Rechen-Shader wie `move` (nicht gebaut).
- **Multi-Draw hilft messbar auf der CPU** (16 384 Listen: 0,14 statt 2,73 ms, je Befehl rund 160 ns samt Push-Wert). Auf der GPU lässt sich mit diesen Läufen nichts sagen, die Spannen überlappen. Ein `draw_indexed_indirect_count` ist nicht gebaut: Es brächte erst etwas mit einem Durchgang, der leere Listen zusammenschiebt, und der lohnt sich mit Hi-Z (G5d).

## Was nicht getestet ist

Andere GPUs (AMD, Intel, Telefone), Vulkan 1.1 und 1.2 auf echter Hardware, Linux und Android (das Programm wird dort nicht gebaut: `apps/render3dtest/TARGETS`), Firefox (kopflos kein WebGPU), Safari, 4K, ein Fenster im Dauerbetrieb, ein verlorenes Gerät mitten in der Arbeit über den Rückgabewert hinaus, Szenen über 1 Mio. Instanzen, sRGB-Ziele (ein Fenster im Browser zeigt die linearen Farben heute ungewandelt), Zeiten im Browser.

## Was nicht gebaut ist

Aus G5a heraus bewusst nicht: Meshes entfernen (die Pools wachsen nur), negative Skalierung (`Invalid`: gespiegelte Instanzen bräuchten eine zweite Pipeline und eigene Listen), Scherung, orthografische Kamera, Durchsichtigkeit (Alpha erreicht das Ziel, gemischt wird nicht), mehrere Ansichten (die Struktur ist da, `Scene` hält eine), mehr Instanzen, als ein Dispatch erreicht, MSAA (fehlt in `lib/gpu`), Schrumpfen von Kapazitäten, ein Rechen-Shader zum Bewegen. Eine Sichtbarkeit je Instanz ist nur Kugel gegen sechs Ebenen: Verdeckung kommt mit G5d, und dort liegt nach G1 der größte Hebel.
