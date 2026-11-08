# RENDER3D -- der Kern der 3D-Engine

`lib/render3d` zeichnet Szenen aus sehr vielen Instanzen über [`lib/gpu`](GPU.md): Meshes in gemeinsamen Puffern, Materialien mit Texturen als Tabelle, Instanzen mit Position in `f64`, Sichtbarkeit im Rechen-Shader, eine Instanzliste je Mesh und Seite (gespiegelte Instanzen drehen ihre Flächen um). Kein Befehl je Objekt, nirgends: Ein Bild sind ein Dispatch und ein indirekter Zeichenbefehl je Liste (Stufe Basis) oder ein Multi-Draw je Seite für alle (Stufe Voll). Dieselben WGSL-Quellen laufen auf Vulkan Voll, auf Vulkan Basis und im Browser über WebGPU.

Stand: G5b, Texturen, glTF-Import ([GLTF.md](GLTF.md)) und gespiegelte Instanzen auf dem Kern aus G5a. PBR, Licht, Schatten (G5c), Hi-Z, Detailstufen, zweite Warteschlange (G5d), die 3D-Ansicht in der UI, Linux, Android, andere GPUs (G5e) kommen danach ([Roadmap](ROADMAP.md)). Beleuchtung heute: Lambert mit einer Sonne und einem Grundlicht, die Grundfarbe mal ihrer Textur.

| Teil | Wo |
| --- | --- |
| `Renderer`, Pools, Materialien, Aufzeichnen | [`lib/render3d/lib.rs`](../lib/render3d/lib.rs) |
| `Scene`, Instanzen, Kamera, Hochladen | [`lib/render3d/scene.rs`](../lib/render3d/scene.rs) |
| Texturen: Klassen, Umrechnen, Mip-Stufen, Arrays | [`lib/render3d/texture.rs`](../lib/render3d/texture.rs) |
| ein glTF-Dokument auf die GPU | [`lib/render3d/gltf.rs`](../lib/render3d/gltf.rs); der Import selbst: [`lib/gltf`](GLTF.md) |
| Shader | [`cull.wgsl`](../lib/render3d/cull.wgsl), [`scatter.wgsl`](../lib/render3d/scatter.wgsl), [`draw.wgsl`](../lib/render3d/draw.wgsl), vorübersetzt in [`shaders_spv.rs`](../lib/render3d/shaders_spv.rs) |
| Tests und Messung | [`apps/render3dtest`](../apps/render3dtest/main.rs), die Referenz auf der CPU: [`cpu.rs`](../apps/render3dtest/cpu.rs); Texturen und Spiegeln: [`tex.rs`](../apps/render3dtest/tex.rs); glTF: [`gltftest.rs`](../apps/render3dtest/gltftest.rs) mit Dateien aus [`gltfgen.rs`](../apps/render3dtest/gltfgen.rs); im Browser: [`tests/engine_web.mjs`](../apps/render3dtest/tests/engine_web.mjs) |

## Ein Beispiel

```rust
let mut r = render3d::Renderer::new(&dev, &render3d::RendererDesc::new(gpu::Format::Rgba8Unorm))?;
let mesh = r.add_mesh(&render3d::MeshData { positions: &pos, normals: &nrm, uvs: &uv, indices: &idx })?;
let rot = r.add_material(&render3d::Material::color(0.8, 0.1, 0.1))?;
let holz = r.add_texture(&render3d::TextureData { width: 512, height: 512, rgba: &px, srgb: true, nearest: false })?;
let brett = r.add_material(&render3d::Material::textured(render3d::TextureRef { texture: holz, sampling: render3d::Sampling::LINEAR }))?;
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

// eine glTF-Datei (.gltf oder .glb): lesen ohne GPU, dann Meshes, Texturen, Materialien und die Instanzen
let doc = gltf::load(&bytes, &mut |uri| plat::read_file(&ordner_und(uri)), &gltf::Limits::DEFAULT)?;
let imported = render3d::gltf::upload(&mut r, &doc, None)?;
for i in &imported.instances { scene.add(*i)?; }
```

## API-Überblick

- **Renderer**: `Renderer::new(&device, &RendererDesc { color, frames_in_flight })` (1 bis 3 Bilder in Arbeit, `RendererDesc::new`: 2), `add_mesh(&MeshData)`, `mesh_info(id)`, `add_texture(&TextureData)`, `texture_info(id)`, `texture_count()`, `texture_bytes()`, `add_material(&Material)`, `set_material(id, &Material)`, `set_size(b, h)`, `record(&mut encoder, &scene, farbe, tiefe)`, `multi_draw()`, `max_instances()`.
- **Scene**: `Scene::new(&renderer)`, `add(Instance)`, `remove(id)`, `set_transform`, `transform`, `set_material`, `set_mesh`, `len`, `set_camera`, `set_sun`, `set_origin`, `prepare(&mut renderer)`, `submitted(id)`, `wait_slot()`.
- **Werte**: `Transform { pos: [f64; 3], rot: [f32; 4], scale: [f32; 3] }` (`mirrored()`: das Produkt der Skalierungen ist negativ), `Camera` (`look_at`; Sichtfeld, Seitenverhältnis, near, far, `f64::INFINITY` heißt keine ferne Ebene), `Sun { direction, color, ambient }`.
- **Materialien** (glTFs Metallic-Roughness-Material, für PBR in G5c fertig angelegt): `Material { base_color, metallic, roughness, emissive, base_color_texture, metallic_roughness_texture, normal_texture, normal_scale, occlusion_texture, occlusion_strength, emissive_texture, alpha_cutoff }`, `Material::color(r, g, b)`, `Material::textured(TextureRef)`. G5b zeigt die Grundfarbe mal ihrer Textur; `alpha_cutoff: Some(c)` ist glTFs MASK.
- **Texturen**: `TextureData { width, height, rgba, srgb, nearest }` (Zeilen von oben, 4 Byte je Texel, gerades Alpha), `TextureId`, `TextureInfo { class, size, layer, srgb, width, height, resampled }`, `TextureRef { texture, sampling }`, `Sampling { mag_nearest, min_nearest, mip: Mip, wrap_u: Wrap, wrap_v: Wrap }` (`Sampling::LINEAR`, `Sampling::NEAREST`), `Wrap::{Repeat, Clamp, Mirror}`, `Mip::{None, Nearest, Linear}`, `CLASSES`, `CLASS_MIN`, `CLASS_MAX`, `class_of(b, h)`, `mip_chain(&TextureData)` (was hochgeht, ohne Gerät).
- **glTF**: `gltf::upload(&mut renderer, &gltf::Document, szene) -> Imported { meshes, materials, default_material, textures, instances, sheared, notes }` ([GLTF.md](GLTF.md)).
- **Zum Messen und Testen**: `set_stats(true)`, `take_stats()`, `collect_stats()` (`FrameStats`: sichtbare Instanzen, Zeitstempel), `upload_info()` (`UploadInfo`: was `prepare` geschickt hat), `lists()` (`ListInfo` mit `mirrored`), `draws_buffer()`, `visible_buffer()`, `set_culling(false)`, `texture_array(klasse, srgb)`.

Konventionen: Reverse-Z (Tiefe 1 an der nahen Ebene, `Compare::Greater`, Löschen mit `DEPTH_CLEAR`, also 0, immer `Depth32Float`), rechtshändige Welt, Vorderseiten gegen den Uhrzeigersinn (gespiegelte Instanzen behalten ihre Außenseite vorn), Farben linear (ein sRGB-Ziel kodiert sie), was das Ziel erreicht, ist deckend (Alpha 1, wie glTF es für OPAQUE und MASK verlangt). Texturkoordinate (0, 0) ist die erste Zeile der Daten. Fehler sind die von `lib/gpu`: `Invalid` für alles, was hier zu erkennen ist: tote Ids, Meshes, Materialien und Texturen, die der Renderer nicht hat (sofort bei `add`, `set_mesh`, `set_material`, `add_material`), nicht endliche Transformationen, ein negativer `alpha_cutoff`, Texturen ohne Fläche oder mit zu wenigen Bytes, ein falsches Ziel, `record` vor `prepare` oder mit Instanzen, die nach `prepare` dazukamen oder ihr Mesh oder ihre Seite wechselten. `Unsupported`, wenn eine Größenklasse mehr Texturen bekäme, als das Gerät Array-Ebenen hat. Eine Szene gehört zu dem Renderer, mit dem sie entstand; mit einem anderen verweigern `prepare` und `record` die Arbeit. Ein `prepare`, das kein Bild erreicht hat oder mittendrin scheiterte, schadet nicht: Das nächste schickt alles noch einmal.

## Aufbau

**Daten.**

| Was | Wie groß | Wer liest |
| --- | --- | --- |
| Ecken (ein Puffer je Renderer) | 32 Byte: `position_u`, `normal_v`, zwei `vec4f`, die UV in den `w` | Vertex-Shader |
| Indizes (zwei Puffer je Renderer) | 16 Bit bis 65 536 Ecken je Mesh, darüber 32 Bit | Grafik-Pipeline |
| Materialien | 80 Byte: Grundfarbe, emissive, metallic, roughness, Normalen-Skala, Verdeckungs-Stärke, `alpha_cutoff` (negativ: deckend), fünf Texturwörter (Ebene, Klasse, Modus, Farbraum) | Fragment-Shader |
| Texturen (je Renderer) | je Größenklasse (128, 256, 512, 1024, 2048) und Farbraum ein Array mit allen Mip-Stufen, ein Sampler | Fragment-Shader |
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
| Barriere, Durchgang | je Seite (erst wie sie sind, dann gespiegelt, mit der Pipeline, deren Vorderseiten im Uhrzeigersinn liegen): Voll: `draw_indexed_indirect_multi` über die Listen (je Indexbreite einer); Basis: je nicht leere Liste ein Push-Wert und ein `draw_indexed_indirect` |

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

**Was die nächsten Stufen vorfinden.** Sichtbarkeit und Zeichnen nehmen eine `ViewLists` (eigener Uniform-Block, eigene Zeichenbefehle, eigene sichtbare Liste); Schatten-Kaskaden und die zweite Runde von Hi-Z sind weitere davon mit einer Pipeline ohne Fragment-Shader. Die erste `vec4f` einer Ecke reicht für reine Tiefendurchgänge. Das Material hat alle fünf Texturen von glTF (Grundfarbe, Metallic-Roughness, Normale, Verdeckung, Emission) mit ihren Faktoren; `sample_texture` in `draw.wgsl` liest jede davon, G5c muss nur die anderen vier abfragen. Die Bindungsgruppe 1 belegt 10 Texturen und einen Sampler von den 16, die WebGPU je Stufe sicher zulässt: Für Schattenkarten und Umgebungslicht bleiben 6 und 15.

## Texturen

**Das Bindungsmodell: ein Textur-Array je Größenklasse und Farbraum, auf allen drei Wegen.** Die Gruppe 1 hält zehn `texture_2d_array` (fünf Klassen in sRGB, fünf linear) und einen Sampler. Ein Texturwort im Material sagt Ebene, Klasse, Abtastmodus und Farbraum; der Fragment-Shader wählt das Array mit einem `switch` (jeder Fall tastet seine eigene Bindung ab). Gründe:

- **Bindless auf Voll hieße einen zweiten Shader.** `lib/wgsl` kennt kein `binding_array` (geprüft: keine Zeile dazu), und WebGPU hat es nicht im Kern; der Voll-Weg bräuchte also eigene Shader-Quellen, und „Voll gegen Basis Byte für Byte“ verglich dann zwei verschiedene Shader. Mit Arrays laufen alle drei Wege aus demselben WGSL, und die Tests zeigen es: 0 Pixel anders, auch im Browser.
- **Die Grenzen reichen überall.** Vulkan 1.1 und WebGPU garantieren 16 abgetastete Texturen und 16 Sampler je Stufe und 256 Array-Ebenen; NVIDIA meldet 2048 Ebenen.
- **Die Kosten:** Speicher für Texturen, die keine Klassengröße haben (sie werden umgerechnet, unten), und das Wachsen eines Arrays (Kopie auf der GPU). Mehr als die Ebenen des Geräts je Klasse geht nicht (`Unsupported`); dort wäre Bindless auf Voll der Ausweg, wenn eine Messung es verlangt.
- **Kein Befehl je Objekt bleibt:** weiter eine Liste je Mesh und Seite, die Textur kommt je Instanz über ihr Material.

**Klassen und Umrechnen.** Klassen sind Quadrate von 128, 256, 512, 1024 und 2048 Texeln mit allen Mip-Stufen. Eine Textur anderer Größe kommt in die Klasse ihrer nächsten Zweierpotenz (128 bis 2048; glTF erlaubt das ausdrücklich) und wird auf der CPU umgerechnet: mit `nearest: true` (für Texturen, die nächstgelegen abgetastet werden) nächstgelegen, eine kleine also Texel für Texel wiederholt; sonst bilinear nach oben und mit einem Kasten nach unten, sRGB in linearem Licht. Größer als 2048 wird verkleinert. Nicht quadratische Texturen werden gestreckt (glTF adressiert über UV, das Bild bleibt).

**Mip-Stufen: ein 2 × 2-Kastenfilter auf der CPU** (`mip_chain` zeigt, was hochgeht). sRGB wird über eine Tabelle dekodiert, gemittelt und genau gerundet wieder kodiert (Schwellen-Tabelle; erst binär gesucht, dann über eine Tabelle von 4097 Startcodes mit höchstens einem Schritt, Messung unten). Gründe gegen einen Rechen-Shader: `lib/gpu` hat keine Mip-Erzeugung, sRGB-Formate sind keine Speicher-Texturen (der Shader müsste über einen Puffer schreiben und je Stufe kopieren), und der CPU-Weg ist genau, auf allen Wegen gleich und gegen einen eigenen Kastenfilter Byte für Byte prüfbar. Er kostet ungefähr so viel wie das Dekodieren des PNG oder JPEG selbst (Messung unten); wird das Laden großer Szenen zum Engpass, ist ein Rechen-Shader der Hebel (nicht gebaut).

**Abtasten.** Ein Sampler für alles: wiederholend, linear, mit linearen Mip-Stufen. Der häufigste Modus (wiederholend und trilinear) geht unverändert an ihn (`textureSampleGrad`). Die anderen macht der Shader genau, weil eine Klasse eine Zweierpotenz ist: Clamp über die Texelmitten der jeweiligen Stufe, Mirror als Clamp auf der gespiegelten Koordinate (an der Naht sind beide die Randtexel), Nearest an der Mitte eines Texels, die Stufe aus den Ableitungen, wie Vulkan sie definiert (ρ = die längere der beiden Ableitungen in Texeln, λ = log₂ ρ; nächste Stufe `floor(λ + 0.5)`, lineare zwischen `floor(λ)` und der nächsten). So braucht es keine Sampler-Tabelle und keine Fälle je Sampler. Anisotropie gibt es nicht (glTF verlangt sie nicht). Die Ableitungen sind `dpdxFine` und `dpdyFine`, gleich am Anfang des Fragment-Shaders, in gleichförmigem Kontrollfluss.

**MASK.** Ist das Alpha der Grundfarbe mal Textur kleiner als `alpha_cutoff`, verwirft der Shader das Fragment (`discard`). Alles Deckende hat den Wert −1 und wird nie verworfen; es läuft durch dieselbe Pipeline (was das kostet, steht unten: nicht von den anderen Änderungen getrennt gemessen). BLEND ist nicht Teil von G5b.

**Hochladen.** `add_texture` schreibt alle Stufen über `Queue::write_texture` (vor der nächsten Übergabe, nach einer Barriere aus `Sampled`: Bilder, die das Array noch lesen, sind fertig). Ein volles Array wächst auf die doppelte Zahl von Ebenen (zuerst 4): ein neues Array, die alten Ebenen über einen Puffer von höchstens 64 MiB kopiert (`lib/gpu` hat keine Kopie von Textur zu Textur; die Zeilen sind dabei auf 256 Byte ausgerichtet, wie WebGPU es will), die Bindungsgruppe neu, das alte Array geht, wenn die GPU fertig ist. Klassen ohne Textur halten ein 1 × 1-Array, weil jede Bindung etwas halten muss.

## Gespiegelte Instanzen

Eine Transformation mit negativer Determinante (eine ungerade Zahl negativer Skalierungen) kehrt die Reihenfolge der Ecken um. Solche Instanzen kommen in eine eigene Liste ihres Meshes und werden mit einer zweiten Pipeline gezeichnet, deren Vorderseiten im Uhrzeigersinn liegen; so bleiben Rückseiten verworfen und die Außenseite vorn. Die Normale braucht nichts Neues: `m · (n / s²)` ist mit dem Vorzeichen in `m` genau die Inverse-Transponierte `r · (n / s)`. Die Kugel der Sichtbarkeit nimmt die Beträge der Skalierungen. Ein `set_transform`, das die Seite wechselt, verschiebt die Instanz in die andere Liste wie ein `set_mesh`. Auf Voll bleibt es bei einem Multi-Draw je Seite (Läufe nebeneinanderliegender Listen gleicher Indexbreite und Seite); auf Basis ein Befehl je Liste wie bisher. Zwei negative Skalierungen sind eine Drehung um 180° und bleiben in der gewöhnlichen Liste.

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
| abgetastete Texturen und Sampler je Stufe (mindestens 16) | Gruppe 1: 10 Arrays und 1 Sampler |
| kein `binding_array` im Kern (und keins in `lib/wgsl`) | ein Array je Klasse und Farbraum, im Shader gewählt |
| Kopien zwischen Puffer und Textur: Zeilen in Vielfachen von 256 Byte | das Wachsen eines Arrays richtet jede Stufe so aus |
| `rgba8unorm-srgb` ist keine Speicher-Textur | Mip-Stufen auf der CPU |

## Tests

`python -u build_rs.py test render3dtest --target win` führt `apps/render3dtest/tests/*.uis` aus, alles offscreen außer `window.uis`, die meisten auf Voll und auf einem auf Basis begrenzten Gerät. Ohne GPU `SKIP`. Die Referenz ist unabhängig von der Engine: ein Strahlenwerfer in `f64` (eigene Wurzel, eigene Drehung über Quaternionen-Produkte, die Ebenen als Abstände im Kameraraum).

| Skript | Prüft | Ergebnis (RTX 2070 SUPER, Treiber 610.88) |
| --- | --- | --- |
| `pixels` | gedrehter Würfel, zwei Würfel in beiden Reihenfolgen, Kugel 2,5 × 1 × 0,5 mit glatten Normalen, Rückseiten von innen, ferne Ebene, Platten 1 auseinander in 100 000 Abstand mit near 0,01; Farbe je Pixel ±1 und die Werte des Tiefenpuffers (über einen Rechen-Shader gelesen) | 0 falsche Pixel; Tiefe: größter relativer Fehler 4,5 · 10⁻⁴, in 100 000 Abstand 6 · 10⁻⁸ |
| `visibility` | Zufallsszenen mit festem Seed (5000, 20 000, 60 000 Instanzen, 3 Meshes) gegen den exakten Test: nichts fehlt, nichts zu viel, nichts doppelt, keine falsche Liste; Kugeln genau an jeder der sechs Flächen (berührend: in der Liste, 0,01 dahinter: nicht); Bild mit Sichtbarkeit gegen Bild mit allem | 975, 2322 und 8820 sichtbar, Abweichung 0; 12 von 12 Kugeln richtig; 0 Pixel anders |
| `changes` | Hinzufügen, Verschieben, Entfernen, Plätze wieder vergeben, Material und Mesh ändern, eine Tabellenzeile; das Bild gegen eine neu aufgebaute Szene (in umgekehrter Reihenfolge); was hochgeht, in Byte; beide Wege des Hochladens; ein `prepare` ohne Bild | 0 Pixel anders; ein Bild ohne Änderung schickt 224 Byte, 200 Änderungen 19 424 Byte |
| `many` | 160 Meshes und 400 Materialien gegen den Strahlenwerfer; wachsende Pools; 16- und 32-Bit-Listen im Wechsel, ein Mesh mit ungerader Indexzahl | 0 falsche Pixel, 0 anders |
| `levels` | Voll gegen Basis, fünf Bilder der Galerie: drei aus G5a (3000 Instanzen, 48 Meshes, 96 Materialien), die texturierte (7 Texturen in 4 Klassen, jeder Abtastmodus, MASK, 600 Instanzen, ein Drittel gespiegelt) und die glTF-Szene | 0 von 57 600 Pixeln anders, in jedem Bild |
| `coords` | Ursprung und 1e7 mit gleicher relativer Kamera; dasselbe mit Positionen, die `f64` bei 1e7 runden muss; Ursprung nachträglich setzen; als Gegenprobe ohne Ursprung | 0 Pixel anders; ohne Ursprung 30 365 von 57 600 anders (der Test kann es also erkennen) |
| `frames` | sieben Bilder in Flug mit eigener Kamera und eigener Änderung gegen jedes einzeln gezeichnet; der Tiefenpuffer des Renderers | 0 Pixel anders |
| `errors`, `shaders` | was die API verweist; leere Szene, auch mit Ursprung und nach einem `prepare` ohne Bild; Instanzen nach `prepare`; ein Bild, das nie abgeschickt wurde; eine Szene mit fremdem Renderer; verlorenes Gerät; die eingebetteten Wörter | bestanden |
| `window` | ein echtes Fenster, sechs Bilder, das dargestellte Bild gegen dasselbe Bild offscreen, mit Validierung; ein einziger, kurzer Lauf | 0 Pixel anders |
| `validate`, `bench_smoke`, `bench_textures` | alles außer `window` unter der Validierungsschicht mit Synchronisationsprüfung, auch der Shader-Zugriffe; kurze Messläufe mit Änderungen und mit 16 Texturen: die Listen enthalten alles aus `cull_ref` der Messszene | 0 Fehler, 0 Warnungen; 0 fehlen |
| `textures` (G5b) | die Wrap-Modi vergrößert, nächstgelegen und bilinear; die Stufen 1 und 2 (ρ 2,6 und 5,2) nächstgelegen und zwischen zwei Stufen gemischt; die 8 Stufen einer 128er-Textur (sRGB und linear) aus dem Array zurückgelesen gegen den eigenen Kastenfilter; ein Boden unter flachem Winkel trilinear über den Sampler und im Shader (Clamp × Repeat, Mirror mit nächster Stufe); jede Klasse vergrößert; 40 Würfel einer Liste mit 40 Texturen (sRGB und linear); 90 Texturen mehr (die Arrays wachsen von 3 auf 15 MiB); 100 × 60 bilinear und 48 × 20 nächstgelegen umgerechnet gegen eigenes Umrechnen; 4096 × 64 in Klasse 2048; MASK mit Löchern und mit einem Faktor-Alpha unter der Schwelle; was verweigert wird | 0 falsche Pixel; größte Abweichung genau gefordert ±1: gemessen 1; bilinear (±2): 1; trilinear über den Sampler (±6): 5, im Shader: 1; Stufen: 0 Byte anders |
| `mirror` (G5b) | gespiegelte Würfel und Prismen (ungerade und gerade Zahl negativer Skalierungen) neben ungespiegelten, texturiert, damit die Spiegelung der UV zu sehen ist; die Listen je Mesh und Seite; jede Instanz mit `set_transform` auf die andere Seite gegen eine neu gebaute Szene; von innen aus einem gespiegelten Würfel; 20 000 Zufallsinstanzen, etwa die Hälfte gespiegelt, gegen den exakten Test | 0 falsche Pixel; Listen (Mesh, gespiegelt, Anzahl) wie erwartet; 0 Pixel anders; nur die Löschfarbe; 2458 sichtbar, 0 fehlen, 0 zu viel |
| `gltf` (G5b) | [GLTF.md](GLTF.md#tests): drei Formen derselben Szene, was gelesen wird, kaputte Dateien, die Szene gezeichnet gegen den Strahlenwerfer (aus der Beschreibung, nicht aus dem Import) | bestanden; gezeichnet 0 falsche Pixel |

**Im Browser** ([`tests/engine_web.mjs`](../apps/render3dtest/tests/engine_web.mjs); Bündel bauen mit `python -u build_rs.py app render3dtest --target web`, dazu das Programm für Windows, das die Vergleichsbilder schreibt): Chrome 156 kopflos, alle drei Bündel, eigenes Profil. Die Galerie (die fünf Bilder oben, eins davon an 1e7; die glTF-Szene als `.glb` im Bündel selbst geschrieben und gelesen) gegen den Vulkan-Weg auf derselben Maschine: **0 von 57 600 Pixeln anders**, gegen Voll wie gegen Basis, in jedem Bündel und jedem Bild, auch dem texturierten mit trilinearer Filterung über den Sampler; der Browser meldet keinen Validierungsfehler; die Zählung der Listen gleich dem exakten Test (2522). Im Browser gilt die Robustheit von WebGPU, auf Vulkan sind die Shader ohne übersetzt; am Bild ändert das nichts. Welches Backend Chrome unter WebGPU nimmt, ist nicht festgehalten, der Adapter heißt `nvidia turing`.

**Texturen in der Referenz.** Der Strahlenwerfer hält jede Textur wie die GPU (Klassengröße, 8-Bit-Stufen aus einem eigenen Kastenfilter, sRGB nach der Definition mit eigenem `exp` und `ln`) und tastet ab, wie Vulkan nächstgelegen, bilinear und trilinear mit den drei Wrap-Modi definiert, unabhängig von `draw.wgsl` (dessen Weg über Texelmitten). Die Stufe kommt aus denselben Ableitungen, die die GPU mit `dpdxFine` nimmt: UV dort, wo die Strahlen der Pixel ihres 2 × 2-Quads die Ebene des Dreiecks treffen. Gespiegelte Instanzen dreht er per Definition um (die Außenseite bleibt vorn). Genau verglichen (±1 je Kanal, wie in G5a wegen der Beleuchtung) wird, wo die GPU treffen muss: nächstgelegene Texel, Stufe 0, eine nächstgelegene Stufe, die Stufen im Array Byte für Byte; bilinear ±2 (die Gewichte der Hardware haben 8 Bit); trilinear über den Sampler ±6 (dort rechnet die Hardware λ selbst und näherungsweise, gemessen 5). Zuerst schlug `textures` an allen linearen Texturen als Grundfarbe fehl: der Shader las dafür nur die sRGB-Arrays. Seitdem trägt das Texturwort den Farbraum, und jeder Platz kann beide.

Die Referenz kennt drei Fälle: Ein Pixel, dessen Strahlen um 1/64 Pixel neben der Mitte dasselbe sehen, muss bis auf ±1 stimmen. Sehen sie Verschiedenes, ist es eine Kante und darf die Farbe eines dieser Objekte zeigen (die GPU rastert auf 1/256 Pixel genau, und ihre Normale liegt an der Kante knapp außerhalb des Dreiecks). Ein Pixel, das einen Treffer eines Nachbarpixels zeigt, wird als Splitter gezählt und darf nur selten sein. Zuerst schlug dieser Vergleich an der Kugel mit einem Pixel fehl: Die Kante lag dort, wo der Test nur exakte Farben der Nachbarstrahlen zuließ.

**Was die Bilder nicht zeigen.** Zuerst fehlte die Barriere zwischen `cull` und dem Vertex-Shader, der die sichtbare Liste liest. Alle Bilder stimmten trotzdem, auf dieser GPU sieht man es nicht, und die Synchronisationsprüfung schwieg, weil sie Shader-Zugriffe erst mit einer eigenen Einstellung ansieht ([GPU.md](GPU.md#validierung)). Mit ihr meldet sie den Fehler; seitdem ist sie immer an. Ob die Barrieren auf anderen GPUs reichen, zeigt ein Lauf dort (G5e).

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
- **Kontrolllauf auf dem fertigen Stand**, mit der Barriere aus [Was die Bilder nicht zeigen](#tests), je ein Lauf, nebenher ein `rustc` einer anderen Sitzung: 1 Mio. Würfel GPU 1,299 ms (Sichtbarkeit 0,063, Zeichnen 1,235) gegen 1,634 ms der Messszene; 100 000 Würfel 0,250 gegen 0,272 ms. Die Barriere kostet also nichts Messbares. CPU je Bild 0,073 gegen 0,051 ms und 0,033 gegen 0,045 ms: Dazu sagt ein Lauf nichts.
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

### G5b: Texturen, Hochladen, glTF

Dieselbe Maschine und Messszene wie oben (Release, offscreen, 1080p, 300 Bilder nach 60, 3 in Flug). Je Variante drei Läufe im Wechsel, gezeigt der Median der Mediane, in Klammern die Spanne. G5a ist der Commit 2a20b91e, für diese Messung neu gebaut, im selben Wechsel gemessen. Nebenher liefen andere Sitzungen (bei Blicken 18 bis 63 % GPU, die CPU einmal bei 79 %, keine `rustc`): Die 99. Perzentile lagen bei allen Varianten um 3,3 ms und werden nicht verglichen, die CPU-Zeiten je Bild schwanken zwischen den Läufen um das Doppelte.

| 1 Mio. Würfel, ms | CPU je Bild | GPU: Sichtbarkeit | GPU: Zeichnen | GPU gesamt | Bildabstand |
| --- | ---: | ---: | ---: | ---: | ---: |
| G5a | 0,065 | 0,063 | 1,240 (1,239–1,243) | 1,305 | 1,346 |
| G5b ohne Texturen | 0,107 | 0,063 | 1,266 (1,265–1,268) | 1,332 | 1,350 |
| G5b, 256 Texturen 256 × 256 (sRGB, wiederholend, trilinear), 1024 Materialien | 0,114 | 0,063 | 1,339 (1,334–1,340) | 1,406 | 1,463 |

- **Ohne Texturen kostet G5b 2 % mehr Zeichenzeit** (1,266 gegen 1,240 ms, die Spannen überlappen nicht). Darin stecken die Ableitungen am Anfang des Fragment-Shaders, die 80 statt 64 Byte große Materialzeile (gelesen werden davon Grundfarbe, Texturwort, Schwelle), die Verzweigung und das `discard`, das MASK in derselben Pipeline braucht.
- **Das meiste davon ist das `discard`**: eine Wegwerf-Änderung ohne die drei Zeilen, im Wechsel mit dem fertigen Stand gemessen (eine Serie von je drei Läufen, mehr GPU-Last nebenher als oben, daher höhere Werte): 1,265 ms (1,258–1,267) ohne gegen 1,285 ms (1,282–1,414) mit. Eine Pipeline ohne `discard` für deckende Listen holt das zurück; sie bräuchte Listen je Mesh, Seite und Alpha-Modus, und die Szene müsste den Modus der Materialien kennen (ein Vorschlag für G5c, nicht gebaut).
- **256 Texturen kosten weitere 6 %** (1,339 ms; 8 % über G5a). Die Würfel sind fern und klein; je Fragment ein trilinearer Zugriff über den Sampler.
- **Sichtbarkeit und Hochladen: gleich.** Die 256 Texturen mit ihren Stufen sind 85 MiB Arrays und waren nach 397 ms (349–398) auf der GPU.

Hochladen je Klasse (`render3dtest --bench-upload`; die Stufen auf der CPU allein mit `mip_chain`, dann `add_texture` bis die GPU sie hat, in einem frischen Renderer; je Größe 4 bis 8 Texturen, drei Läufe, Median; die CPU war beschäftigt, die Läufe streuen bis 1,6-fach):

| Textur | Stufen auf der CPU | bis die GPU sie hat | MiB mit Stufen |
| --- | ---: | ---: | ---: |
| 128 × 128 sRGB / linear | 0,30 / 0,13 ms | 0,30 / 0,19 ms | 0,08 |
| 256 × 256 sRGB / linear | 0,73 / 0,38 ms | 1,35 / 0,51 ms | 0,33 |
| 512 × 512 sRGB / linear | 4,4 / 2,0 ms | 5,4 / 4,3 ms | 1,3 |
| 1024 × 1024 sRGB / linear | 18 / 9,2 ms | 26 / 13 ms | 5,3 |
| 2048 × 2048 sRGB / linear | 50 / 29 ms | 126 / 41 ms | 21 |
| 1536 × 1280 sRGB, umgerechnet auf 2048 | 330 ms | 363 ms | 21 |

- **Die sRGB-Kodierung war zuerst der größte Posten**: mit binärer Suche über die 255 Schwellen kosteten die Stufen einer 2048er sRGB-Textur 73 ms gegen 20 ms linear (Faktor 3,7, eine Serie von drei Läufen); über die Tabelle von Startcodes noch 50 gegen 29 ms (Faktor 1,7, eine zweite Serie, unter mehr Last). Die Bytes sind dieselben (der Test liest die Stufen zurück: 0 Byte anders).
- **Das Umrechnen ist teuer** (330 ms für 1536 × 1280 auf 2048): bilinear je Kanal in `f32`, mit Gewichten je Zieltexel. Für Dateien mit vielen Texturen, die keine Zweierpotenzen sind, ist das der erste Hebel.
- **Zum Vergleich**: das Dekodieren eines PNG oder JPEG dieser Größe kostet in derselben Größenordnung; eine GPU-Mip-Erzeugung wäre ein Bruchteil davon (nicht gebaut, oben warum).

glTF laden (`render3dtest --bench-gltf`; die Datei im Programm geschrieben, nicht gezählt; Speicher: private Bytes des Prozesses vor und nach `gltf::load`, darin liegt auch, was der Parser wieder freigab; drei Läufe, Median, in Klammern die Spanne):

| Datei | .glb | `gltf::load` | privater Speicher | das Modell hält | `gltf::upload` | `Scene::add` | erstes `prepare` |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 100 000 Knoten mit Mesh (ein Fünftel gespiegelt) unter 100 Gruppen, 4 Meshes | 17,0 MB | 330 ms (255–457) | +138 MB | 34,4 MB | 58 ms (50–98) | 17 ms | 14 ms |
| ein Mesh mit 1 Mio. Ecken und 2 Mio. Dreiecken | 56,0 MB | 167 ms (138–170) | +56 MB | 56,0 MB | 146 ms (105–182) | – | – |

- **Große Meshes lesen schnell** (330 bis 400 MB/s), der Speicher ist das Modell selbst.
- **Viele Knoten kosten im JSON**: Der Baum aus `agent_core::json` braucht für 17 MB Text vorübergehend rund das Sechsfache; ein Knoten im Modell hält rund 340 Byte (zwei `f64`-Matrizen, Name, Kinder). Für Szenen mit Millionen Knoten wären ein Leser, der den Baum nicht aufbaut, und schlankere Knoten der Weg (nicht gebaut).

## Was nicht getestet ist

Andere GPUs (AMD, Intel, Telefone), Vulkan 1.1 und 1.2 auf echter Hardware, Linux und Android (das Programm wird dort nicht gebaut: `apps/render3dtest/TARGETS`), Firefox (kopflos kein WebGPU), Safari, 4K, ein Fenster im Dauerbetrieb, ein verlorenes Gerät mitten in der Arbeit über den Rückgabewert hinaus, Szenen über 1 Mio. Instanzen, sRGB-Ziele (ein Fenster im Browser zeigt die linearen Farben heute ungewandelt), Zeiten im Browser. Aus G5b: Texturen mit mehr Ebenen, als eine Klasse auf diesem Gerät hat (2048), eine Klasse am Limit von 256 Ebenen im Browser, Texturen größer als 2048 außer dem einen Fall 4096 × 64, eine sehr große Zahl von Texturen in einem echten glTF (die Tests schreiben ihre Dateien selbst; Beispielmodelle von Khronos wären ein Download), Dateien mit `KHR_texture_transform` und weiteren UV-Sätzen.

## Was nicht gebaut ist

Aus G5a heraus bewusst nicht: Meshes entfernen (die Pools wachsen nur), Skalierung 0 auf einzelnen Achsen (erlaubt, aber die Flächen ohne Normale werden schwarz; 0 auf allen Achsen versteckt eine Instanz sauber), Scherung (ein glTF-Knoten mit Scherung in der Weltmatrix wird ohne sie gezeichnet und gezählt), orthografische Kamera, Durchsichtigkeit (glTFs BLEND wird deckend gezeichnet), mehrere Ansichten (die Struktur ist da, `Scene` hält eine), mehr Instanzen, als ein Dispatch erreicht, MSAA (fehlt in `lib/gpu`), Schrumpfen von Kapazitäten, ein Rechen-Shader zum Bewegen. Aus G5b: Texturen entfernen, Bindless auf Voll, Mip-Stufen auf der GPU, anisotrope Filterung, beidseitige Materialien (`double_sided`: von vorn gezeichnet, gezählt), eine Pipeline ohne `discard` für deckende Listen, weitere UV-Sätze, komprimierte Texturen (BC, KTX2). Eine Sichtbarkeit je Instanz ist nur Kugel gegen sechs Ebenen: Verdeckung kommt mit G5d, und dort liegt nach G1 der größte Hebel.
