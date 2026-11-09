# RENDER3D -- der Kern der 3D-Engine

`lib/render3d` zeichnet Szenen aus sehr vielen Instanzen über [`lib/gpu`](GPU.md): Meshes in gemeinsamen Puffern, Materialien mit Texturen als Tabelle, Instanzen mit Position in `f64`, Sichtbarkeit im Rechen-Shader, eine Instanzliste je Mesh und Seite (gespiegelte Instanzen drehen ihre Flächen um). Kein Befehl je Objekt, nirgends: Ein Bild sind ein Dispatch und ein indirekter Zeichenbefehl je Liste (Stufe Basis) oder ein Multi-Draw je Seite für alle (Stufe Voll). Dieselben WGSL-Quellen laufen auf Vulkan Voll, auf Vulkan Basis und im Browser über WebGPU.

Stand: G5c, Schritte 1 bis 3: Materialien nach PBR (glTF 2.0, Anhang B) mit allen fünf Texturen, Belichtung und Tonemapping im Fragment-Shader; kaskadierte Schatten der Sonne; Punkt- und Spotlichter (auch aus glTF über `KHR_lights_punctual`); auf G5b (Texturen, glTF-Import ([GLTF.md](GLTF.md)), gespiegelte Instanzen) und dem Kern aus G5a. Umgebungslicht ist der letzte Schritt von G5c; Hi-Z, Detailstufen, zweite Warteschlange (G5d), die 3D-Ansicht in der UI, Linux, Android, andere GPUs (G5e) kommen danach ([Roadmap](ROADMAP.md)).

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
- **Werte**: `Transform { pos: [f64; 3], rot: [f32; 4], scale: [f32; 3] }` (`mirrored()`: das Produkt der Skalierungen ist negativ), `Camera` (`look_at`; Sichtfeld, Seitenverhältnis, near, far, `f64::INFINITY` heißt keine ferne Ebene), `Sun { direction, color, ambient }` (Einheiten von KHR_lights_punctual: `color` ist die Beleuchtungsstärke in Lux, eine weiße matte Fläche zur Sonne zeigt `color / π`; `ambient` die Strahldichte einer gleichförmigen Umgebung; `Sun::DEFAULT`: 0,75 π und 0,25), `Scene::set_exposure(f32)` (1), `Scene::set_tonemap(Tonemap::{Neutral, None})` (Neutral).
- **Materialien** (glTFs Metallic-Roughness-Material): `Material { base_color, metallic, roughness, emissive, base_color_texture, metallic_roughness_texture, normal_texture, normal_scale, occlusion_texture, occlusion_strength, emissive_texture, alpha_cutoff }`, `Material::color(r, g, b)` (metallic 0, roughness 1), `Material::textured(TextureRef)`. Alles wirkt, wie glTF es beschreibt ([PBR](#pbr-g5c)); `alpha_cutoff: Some(c)` ist glTFs MASK.
- **Die Tabelle der Split-Sum-Näherung**: `DFG`, `DFG_N`, `dfg_table()` ([PBR](#pbr-g5c)).
- **Punkt- und Spotlichter** ([Lichter](#punkt--und-spotlichter-g5c)): `Scene::set_lights(&[Light])`, `Scene::lights()`, `Light { kind, pos, direction, color, intensity, range }`, `Light::point(pos, color, intensity, range)`, `LightKind::{Point, Spot { inner, outer }}`; aus glTF `Imported::lights` und `Imported::suns`.
- **Schatten der Sonne** ([Schatten](#schatten-der-sonne-g5c)): `Scene::set_shadows(Option<&Shadows>)`, `Shadows { cascades, size, distance, split, reach }` (`Shadows::DEFAULT`: 4 Kaskaden zu 2048, bis 100, Aufteilung 0,75, Reichweite 100), `CASCADES_MAX`, `Scene::cascades() -> &[Cascade]` (was `prepare` rechnete: Ausschnitt, Mitte, Radius, Texel, Achsen, Tiefenbereich), `Renderer::set_shadow_mask(bool)` (false: Tiefen-Durchgang ohne Fragment-Shader), zum Testen `Renderer::shadow_map()`, `Scene::cascade_buffers(k)`; `FrameStats::cascades` (je Kaskade Sichtbarkeit und Tiefen-Durchgang in ms) und `FrameStats::draw`.
- **Texturen**: `TextureData { width, height, rgba, srgb, nearest }` (Zeilen von oben, 4 Byte je Texel, gerades Alpha), `TextureId`, `TextureInfo { class, size, layer, srgb, width, height, resampled }`, `TextureRef { texture, sampling }`, `Sampling { mag_nearest, min_nearest, mip: Mip, wrap_u: Wrap, wrap_v: Wrap }` (`Sampling::LINEAR`, `Sampling::NEAREST`), `Wrap::{Repeat, Clamp, Mirror}`, `Mip::{None, Nearest, Linear}`, `CLASSES`, `CLASS_MIN`, `CLASS_MAX`, `class_of(b, h)`, `mip_chain(&TextureData)` (was hochgeht, ohne Gerät).
- **glTF**: `gltf::upload(&mut renderer, &gltf::Document, szene) -> Imported { meshes, materials, default_material, textures, instances, sheared, notes }` ([GLTF.md](GLTF.md)).
- **Zum Messen und Testen**: `set_stats(true)`, `take_stats()`, `collect_stats()` (`FrameStats`: sichtbare Instanzen, Zeitstempel), `upload_info()` (`UploadInfo`: was `prepare` geschickt hat), `lists()` (`ListInfo` mit `mirrored`), `draws_buffer()`, `visible_buffer()`, `set_culling(false)`, `texture_array(klasse, srgb)`.

Konventionen: Reverse-Z (Tiefe 1 an der nahen Ebene, `Compare::Greater`, Löschen mit `DEPTH_CLEAR`, also 0, immer `Depth32Float`), rechtshändige Welt, Vorderseiten gegen den Uhrzeigersinn (gespiegelte Instanzen behalten ihre Außenseite vorn), Licht in den Einheiten von KHR_lights_punctual mal der Belichtung, im Fragment-Shader tongemappt, Farben linear (ein sRGB-Ziel kodiert sie), was das Ziel erreicht, ist deckend (Alpha 1, wie glTF es für OPAQUE und MASK verlangt). Texturkoordinate (0, 0) ist die erste Zeile der Daten. Fehler sind die von `lib/gpu`: `Invalid` für alles, was hier zu erkennen ist: tote Ids, Meshes, Materialien und Texturen, die der Renderer nicht hat (sofort bei `add`, `set_mesh`, `set_material`, `add_material`), nicht endliche Transformationen, ein negativer `alpha_cutoff`, Texturen ohne Fläche oder mit zu wenigen Bytes, ein falsches Ziel, `record` vor `prepare` oder mit Instanzen, die nach `prepare` dazukamen oder ihr Mesh oder ihre Seite wechselten. `Unsupported`, wenn eine Größenklasse mehr Texturen bekäme, als das Gerät Array-Ebenen hat. Eine Szene gehört zu dem Renderer, mit dem sie entstand; mit einem anderen verweigern `prepare` und `record` die Arbeit. Ein `prepare`, das kein Bild erreicht hat oder mittendrin scheiterte, schadet nicht: Das nächste schickt alles noch einmal.

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
- **Material im Fragment-Shader.** Ferne Würfel haben mehr Ecken als Pixel; die Grundfarbe je Ecke zu holen, wäre teurer. PBR liest die Zeile ohnehin dort.
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

## PBR (G5c)

**Die BRDF ist die von glTF 2.0, Anhang B**, so wie dessen Beispiel-Implementierung sie schreibt: `c_diff = lerp(Grundfarbe, schwarz, metallic)`, `f0 = lerp(0,04, Grundfarbe, metallic)`, `α = roughness²` (höchstens bis 0,001 herab: Rauheit 0 teilte am Glanzpunkt 0 durch 0), Schlicks Fresnel `F = f0 + (1 − f0)(1 − v·h)⁵`, GGX/Trowbridge-Reitz `D`, die getrennte Smith-Sichtbarkeit `V = G / (4 |n·l| |n·v|)` des Anhangs, `f = (1 − F) c_diff / π + F D V`. Keine transzendente Funktion außer Wurzeln (die fünfte Potenz sind Produkte): Das hält die Wege gleich, im Browser wie auf Vulkan sind es 0 Pixel Unterschied.

**Texturen.** Grundfarbe (sRGB) mal Faktor; Metallic-Roughness (linear, g mal roughness, b mal metallic, danach auf 0 bis 1 geklemmt); Normalenkarte (linear, `(t · 2 − 1)` mit `normal_scale` auf x und y); Verdeckung (linear, `1 + strength (r − 1)`, nur auf das Grundlicht, nie auf die Sonne: der Test prüft, dass sie ohne Grundlicht kein Pixel ändert); Emission (sRGB mal Faktor, Strahldichte vor der Belichtung).

**Tangenten: der Rahmen des Dreiecks aus Ableitungen im Fragment-Shader**, nicht im Vertex-Format. Die Tangente ist `∂p/∂u` des Dreiecks, aus `dpdxFine`/`dpdyFine` der Weltposition und der UV gelöst, gegen die interpolierte Normale orthogonalisiert; die Bitangente ist `n × t`, so gedreht, dass sie zu fallendem v zeigt (glTF: +y ist oben im Bild). Damit kommen gespiegelte UV-Inseln und gespiegelte Instanzen ohne Vorzeichen in den Daten richtig heraus. Gründe:
- **Die andere Möglichkeit kostet Zeichenzeit.** Tangenten im Vertex-Format (48 statt 32 Byte je Ecke, eine `vec4f` mehr gezogen und als Varying weitergegeben, dafür keine Ableitungen der Position): Wegwerf-Änderung, 1 Mio. Würfel, 1080p, drei Läufe im Wechsel, 17 bis 53 % GPU-Last anderer Sitzungen: Zeichnen 1,989 ms (1,817–2,032) gegen 1,767 ms (1,564–1,786) mit Ableitungen, rund 13 % mehr. Der Würfel-Szene liegt die feste Funktion und das Ziehen der Ecken am Herzen ([G1](#messwerte)); jedes Byte je Ecke zählt dort.
- **MikkTSpace auf der CPU** wäre ein großer, eigener Algorithmus (und glTF verlangt ihn nur ohne TANGENT-Attribut; `lib/gltf` liest TANGENT nicht).
- **Die Abweichung von MikkTSpace**, dokumentiert und gemeldet (`gltf::upload` notiert sie bei jeder Normalenkarte): MikkTSpace mittelt Tangenten je Ecke über die anliegenden Dreiecke und interpoliert sie; hier hat jedes Dreieck seinen eigenen, exakten Rahmen. Auf ebenen Flächen und bei stetiger UV sind beide gleich; auf gekrümmten Meshes mit wenigen Dreiecken zeigt eine mit MikkTSpace gebackene Normalenkarte hier leichte Facetten, an UV-Nähten kein Glätten. Dreiecke ohne UV-Gefälle (alle UV gleich) behalten die Normale.

**Belichtung und Tonemapping im Fragment-Shader**: Licht linear in den Einheiten von KHR_lights_punctual, mal `Scene::set_exposure`, dann **Khronos PBR Neutral** oder nichts (`Tonemap::None`, für HDR-Ziele wie `Rgba16Float` und für Messungen). Neutral statt einer ACES-Näherung: Es hält Grundfarben unter weißem Licht bis 0,76 unverändert (bis auf einen kleinen Versatz im Schwarz), komprimiert nur die Lichter und entsättigt sie leicht; ACES verschiebt Kontrast und Sättigung jeder Farbe, eine Produkt- oder CAD-Ansicht zeigte also nicht mehr die Farben ihrer Materialien. Neutral braucht keine transzendente Funktion. Kein Nachbearbeitungs-Pass: Das Ziel bekommt die fertige Farbe.

**Das Grundlicht** (`Sun::ambient`, eine gleichförmige Umgebung) geht über die Split-Sum-Näherung: spiegelnd `ambient · (f0 A + B)`, diffus `ambient · c_diff · (1 − (f0 A + B))` (was die Spiegelung nicht nimmt, geht in die Streuung; ein weißes Dielektrikum zeigt im Weiß-Ofen also genau 1), beides mal Verdeckung. **A und B kommen aus einer Tabelle, nicht aus einer analytischen Näherung**: Karis' Näherung (nach Lazarov) lag für diese BRDF mit ihrer getrennten Smith-Sichtbarkeit bis 0,19 neben dem Integral (Rauheit 0,5, n·v 1; gemessen vom `pbr`-Test), die Tabelle 0,004. Sie hat 32 × 32 Einträge über n·v und Rauheit (`dfg.rs`, `render3dtest --gen-dfg` schreibt sie, `--test shaders` vergleicht sie Bit für Bit mit einer frischen Rechnung: GGX-Wichtigkeitsabtastung des Halbvektors, 4096 Hammersley-Punkte je Eintrag in `f64`), liegt als Speicherpuffer in Gruppe 1 (kein Texturplatz) und wird im Shader bilinear mit eigenen Gewichten gelesen (auf allen Wegen genau dieselben). Schritt 4 (Umgebungslicht) nimmt dieselbe Tabelle.

**Nächstgelegenes Abtasten lädt jetzt das Texel** (`textureLoad`) statt die Mitte eines Texels über den linearen Sampler zu lesen: Die Hardware mischt dort mit ihren 8-Bit-Gewichten 1/256 eines Nachbarn bei, was G5b nicht sah (±1), ein metallischer Glanzpunkt aber auf 2 Stufen vervielfachte (zuerst schlug der Test der Metallic-Roughness-Textur an einem Pixel so fehl).

## Schatten der Sonne (G5c)

**Kaskadierte Schattenkarten**, eine `Depth32Float`-Array-Textur mit vier Ebenen, die der Renderer hält (sie wird erst angelegt, wenn eine Szene Schatten will, und bleibt zwischen den Bildern in `Layout::Sampled`). Jede Kaskade ist eine weitere `ViewLists`: eigener Uniform-Block, eigene Zeichenbefehle und sichtbare Liste, dieselbe Sichtbarkeit `cull.wgsl` auf der GPU, dann ein Tiefen-Durchgang in ihre Ebene. Ein Bild: Kopien, `scatter`, die Sichtbarkeit der Kamera, die der Kaskaden, eine Barriere, die Tiefen-Durchgänge, der Hauptdurchgang.

- **Vier Kaskaden** (1 bis 4 wählbar). Gemessen kostet die Zahl kaum etwas: Den Preis macht die ferne Kaskade, die bis zur Schattenweite reicht (1 Mio. Würfel: 1,8 bis 2,3 ms Tiefen-Durchgang, unten); jede weitere nahe Kaskade kostet 0,04 ms Sichtbarkeit und 0,2 bis 0,5 ms Tiefe und bringt Auflösung, wo man hinsieht (Kaskade 0 hat bei Schattenweite 90 ein Texel von 6,6 mm statt 9,8 cm mit einer Kaskade). Mit vier war das ganze Bild sogar am schnellsten (der Hauptdurchgang liest kleinere Karten).
- **Aufteilung**: zwischen gleichmäßig und logarithmisch (`split`, 0,75 als Vorgabe), bis `distance` entlang der Blickrichtung.
- **Stabil**: Eine Kaskade deckt ihren Ausschnitt mit der Kugel um ihn ab; deren Radius hängt nur von Sichtfeld, Seitenverhältnis und Aufteilung ab, ändert sich also nicht, wenn sich die Kamera dreht (der Test prüft 40 Kameras, gedreht und verschoben, die Hälfte bei 1e7: dieselben Radien Bit für Bit). Die Mitte wird in der Basis des Lichts auf das Raster der Texel gerastet, in Weltkoordinaten (`f64`, nicht relativ zum Ursprung): Eine Kamera, die sich innerhalb eines Texels bewegt, zeichnet dieselbe Karte (gelesen: 0 von 4,2 Mio. Texeln anders), eine, die sich um ganze Texel bewegt, dieselbe verschoben (keine Bedeckung anders; die Tiefen weichen im letzten Bit ab, höchstens 7,5 · 10⁻⁹, weil die Konstanten der Matrix anders runden). Die Basis des Lichts hängt nur von der Sonne ab (z zur Sonne, x und y von der Welt-y-Achse, bei Sonne im Zenit von x).
- **Tiefe**: Reverse-Z auch hier (1 an der Seite zur Sonne). Die Sichtbarkeit einer Kaskade testet vier Seiten und die ferne Seite, keine nahe: Werfer zwischen Sonne und Kaskade bleiben in der Liste, der Vertex-Shader drückt sie auf die nahe Ebene (Pancaking), so braucht die Tiefe nicht bis zur Sonne zu reichen. `reach` sagt, wie weit vor der Kugel die Tiefe noch stimmt; ein Dreieck, das diese Ebene schneidet, bekommt dahinter eine falsche Tiefe (bekannte Grenze des Pancaking, nicht getestet).
- **Beide Seiten werfen** (`Cull::None` im Tiefen-Durchgang): glTFs einseitige Platten werfen auch Schatten, wenn die Sonne von hinten kommt.
- **PCF**: 3 × 3 Abtastungen des bilinearen Vergleichs (ein Vergleichs-Sampler, `GreaterEqual`), zusammen ein Zelt über 4 × 4 Texel. Die Kaskade ist die erste, deren Karte den Punkt mit dem Rand des Filters enthält; jenseits der letzten ist Sonne. Kein Überblenden zwischen Kaskaden.
- **Bias auf der Seite des Empfängers**, im Shader, nicht im Rasterizer: Der Punkt wird um 2 Texel seiner Kaskade entlang der (geometrischen) Normalen versetzt, der Vergleichswert um 0,25 Texel zur Sonne. Zwei Texel reichen für die Reichweite des Filters auf jeder Neigung (Versatz `k` entlang `n` gibt `k / cos θ` Abstand, das Zelt sieht bis `2 tan θ` Texel weit); weil der Versatz in Texeln der Kaskade gemessen wird, passt er sich der Auflösung an. `lib/gpu` hat keinen Tiefen-Bias im Pipeline-Zustand (`RenderPipelineDesc` ist nicht erweiterbar, ohne SemVer zu brechen), und so ist es auf allen Wegen derselbe Code. Gemessen: 0 Pixel Akne (gegen exakte Schattenstrahlen keine abweichenden Pixel abseits von Schattenkanten), der Preis ist eine um bis zu zwei Texel verschobene Kante (unten im Test).
- **MASK im Schatten**: keine Listen je Alpha-Modus (das ist offen, siehe unten), sondern beide Wege gemessen: Tiefen-Durchgang ohne Fragment-Shader (MASK wirft volle Schatten) gegen einen mit `fs_shadow`, das die Materialzeile liest und nur für MASK verwirft. **Gebaut ist der richtige Weg mit Fragment-Shader**: Die Löcher lassen die Sonne durch (der Test sieht es: ohne ihn 504 Pixel anders, 209 davon dunkler, gegen die Strahlen 498 statt 230 auf der falschen Seite). Er kostet in der Messszene ohne ein einziges MASK-Material 0,31 ms je Bild (12 % der Tiefen-Durchgänge; die nahen Kaskaden am meisten, 0,175 statt 0,079 ms: Ein Fragment-Shader mit `discard` nimmt dem Durchgang den frühen Tiefentest). `Renderer::set_shadow_mask(false)` ist der schnelle Weg für Szenen ohne MASK. Listen je Alpha-Modus holten beides.
- **Bindungen**: Die Schattenkarte und ihr Vergleichs-Sampler liegen in einer eigenen Gruppe 2, die nur der Hauptdurchgang bindet: WebGPU verbietet eine Textur als Anhang und gleichzeitig als gebundene Textur im selben Durchgang (auch wenn der Shader sie nicht liest), und Gruppe 1 braucht der Tiefen-Durchgang für die Texturen von MASK. Gruppe 1 hat weiter 10 Texturen und einen Sampler, Gruppe 2 eine Textur und einen Sampler; mit den Push-Werten (im Browser Gruppe 3) sind alle vier Gruppen belegt.
- **`lib/wgsl` und `lib/gpu` konnten alles schon**: `texture_depth_2d_array`, `sampler_comparison`, `textureSampleCompareLevel` (SPIR-V `OpImageSampleDrefExplicitLod`, im Browser als WGSL), Tiefentexturen als Sampled, Vergleichs-Sampler, Ansichten einzelner Ebenen als Tiefenanhang, Layout-Übergänge Tiefe → Lesen über `DepthAttachment::after`. Keine Änderung an veröffentlichten Crates.

**Barrieren** (selbst gegen die Zugriffe der Shader gelesen):

| Wo | Was davor | Was danach | Warum |
| --- | --- | --- | --- |
| Anfang des Bildes (wie G5a, um die Kopien der Kaskaden erweitert) | alle Zugriffe des Bildes davor (Kopien, Rechen-, Vertex- und Fragment-Shader, indirekte Befehle) | Kopien, Rechen-Shader | die Uniform-Blöcke der Kaskaden, ihre Zeichenbefehle und Listen werden neu geschrieben, während das Bild davor sie gelesen haben kann |
| nach den Kopien (wie G5a) | Kopien, `scatter` | Rechen-Shader (Lesen und Schreiben), Vertex- und Fragment-Shader | `cull` liest die Uniform-Blöcke der Kaskaden und schreibt in die aus der Vorlage kopierten Zeichenbefehle; `vs_shadow`/`vs_depth` lesen die Uniform-Blöcke |
| nach aller Sichtbarkeit (wie G5a, deckt jetzt alle Ansichten) | Rechen-Shader schreibt | indirekte Befehle, Vertex-Shader, Kopie | die Zeichenbefehle und Listen jeder Kaskade vor ihrem Tiefen-Durchgang |
| Anfang eines Tiefen-Durchgangs (Layout `Sampled` → `DepthTarget`, nur diese Ebene) | Vertex-, Fragment-, Rechen-Shader | frühe und späte Fragment-Tests, Lesen und Schreiben der Tiefe | der Hauptdurchgang des Bildes davor hat die Ebene gelesen (Schreiben nach Lesen) |
| Ende eines Tiefen-Durchgangs (`DepthTarget` → `Sampled`) | späte Fragment-Tests, Schreiben der Tiefe | Vertex-, Fragment-, Rechen-Shader, Lesen | der Hauptdurchgang liest die Ebene (Lesen nach Schreiben); **Mutation**: ohne diesen Übergang (`after: DepthTarget`) meldet die Validierung 30 Fehler (Layout und `READ_AFTER_WRITE`, „Shader stage FRAGMENT reads … previously written … by the attachment storeOp“), die Bilder bleiben auf dieser GPU richtig |
| Lichtliste hochladen (G5c; die Kopie liegt vor der Barriere nach den Kopien) | Kopie schreibt | Fragment-Shader liest (die Barriere nach den Kopien) | die Lichter dieses Bildes; gegen das Lesen des Bildes davor wartet die Barriere am Anfang (Schreiben nach Lesen). **Mutation**: die Kopie hinter alle Barrieren, direkt vor den Hauptdurchgang gelegt: die Validierung meldet 12 Fehler (`READ_AFTER_WRITE`, „Shader stage FRAGMENT reads VkBuffer … previously written by vkCmdCopyBuffer“), die Bilder bleiben auf dieser GPU richtig |
| Anlegen der Karte (eigene Übergabe) | — (`Undefined`) | wie oben nach `Sampled`, gelöscht | jede Ebene ist definiert und im erwarteten Layout, auch die einer Kaskade, die nie gezeichnet wird |

## Punkt- und Spotlichter (G5c)

**Eine Liste im Speicherpuffer** (`Scene::set_lights`, 64 Byte je Licht: Position relativ zum Ursprung und Reichweite, Farbe mal Intensität und Art, Achse, Kegel als Skala und Versatz des Kosinus), Gruppe 0, Bindung 6, nur der Fragment-Shader liest sie. Sie geht mit dem nächsten Bild hoch, wenn sie sich änderte, der Ursprung wanderte oder ein `prepare` sein Bild nicht erreichte; sonst kostet sie nichts (ein Bild ohne Änderung schickt weiter 496 Byte). Der Fragment-Shader läuft für jedes Fragment über alle Lichter.

- **Abfall nach `KHR_lights_punctual`**: Intensität in Candela, `1 / d²`, mit Reichweite das Fenster `clamp(1 − (d / Reichweite)⁴, 0, 1)` (der Test: das Licht der Reichweite 4 ändert 3842 Pixel innerhalb, 0 außerhalb); Spot: `t = clamp(cos · Skala + Versatz, 0, 1)`, `t²`, mit `Skala = 1 / max(0,001, cos innen − cos außen)` und `Versatz = −cos außen · Skala`, beides auf der CPU in `f64` (kein Kosinus im Shader). Dieselbe BRDF wie die Sonne. Keine Schatten (offen).
- **Kosten je Licht** (unten): etwa 0,11 ms je Licht und Bild in der Messszene, auch wenn fast alle Fragmente außerhalb der Reichweiten liegen: Die Schleife lädt jedes Licht für jedes Fragment. **Gebaut ist keine Aufteilung in Cluster oder Kacheln**: Die Messung verlangt sie ab etwa acht Lichtern (acht kosten 0,75 ms, 64 schon 7,2 ms, 256 30 ms), aber sie ist ein eigener Baustein (ein Rechen-Shader, der die Lichter in Cluster des Sichtvolumens sortiert, Indexlisten je Cluster, der Fragment-Shader schlägt seinen Cluster nach) und blieb nicht im Rahmen dieser Stufe. Offener Punkt.
- **Aus glTF** ([GLTF.md](GLTF.md)): `render3d::gltf::upload` setzt Punkt- und Spotlichter an die Stelle ihres Knotens (Achse: dessen −z) in `Imported::lights`; gerichtete Lichter werden zu `Imported::suns` (Richtung zur Sonne, Grundlicht 0), mehr als eines ist eine Notiz (eine Szene hat eine Sonne).

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
| abgetastete Texturen und Sampler je Stufe (mindestens 16) | Gruppe 1: 10 Arrays und 1 Sampler; Gruppe 2 (G5c): die Schattenkarte und ihr Vergleichs-Sampler |
| eine Textur nicht als Anhang und gebunden im selben Durchgang | die Schattenkarte in einer Gruppe, die der Tiefen-Durchgang nicht bindet |
| kein `binding_array` im Kern (und keins in `lib/wgsl`) | ein Array je Klasse und Farbraum, im Shader gewählt |
| Kopien zwischen Puffer und Textur: Zeilen in Vielfachen von 256 Byte | das Wachsen eines Arrays richtet jede Stufe so aus |
| `rgba8unorm-srgb` ist keine Speicher-Textur | Mip-Stufen auf der CPU |

## Tests

`python -u build_rs.py test render3dtest --target win` führt `apps/render3dtest/tests/*.uis` aus, alles offscreen außer `window.uis`, die meisten auf Voll und auf einem auf Basis begrenzten Gerät. Ohne GPU `SKIP`. Die Referenz ist unabhängig von der Engine: ein Strahlenwerfer in `f64` (eigene Wurzel, eigene Drehung über Quaternionen-Produkte, die Ebenen als Abstände im Kameraraum). Seit G5c schattiert er mit derselben BRDF, aus der Spezifikation neu geschrieben (Neutral-Tonemapper, Tangentenrahmen des Dreiecks aus seinen Kanten und UV); die Tabelle der Split-Sum ist für ihn Daten wie die Texel einer Textur, ihr Inhalt wird gegen sein eigenes Integral geprüft. Alle Bilder der G5a- und G5b-Tests haben ihre neue Referenz aus ihm, keine aus der GPU. An Kanten darf ein einfarbiges Objekt jede Farbe zwischen denen zeigen, die sein Dreieck mit den Normalen seiner Ecken und am Treffer hätte (vorher: jede Helligkeit zwischen Grundlicht und voller Sonne; mit Glanz und Fresnel ist die Farbe kein Vielfaches der Grundfarbe mehr).

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
| `bench_shadows` (G5c) | ein kurzer Messlauf mit vier Kaskaden unter Validierung | 0 Fehler; die Listen der Kamera wie `cull_ref` |
| `validate`, `bench_smoke`, `bench_textures` | alles außer `window` unter der Validierungsschicht mit Synchronisationsprüfung, auch der Shader-Zugriffe; kurze Messläufe mit Änderungen und mit 16 Texturen: die Listen enthalten alles aus `cull_ref` der Messszene | 0 Fehler, 0 Warnungen; 0 fehlen |
| `textures` (G5b) | die Wrap-Modi vergrößert, nächstgelegen und bilinear; die Stufen 1 und 2 (ρ 2,6 und 5,2) nächstgelegen und zwischen zwei Stufen gemischt; die 8 Stufen einer 128er-Textur (sRGB und linear) aus dem Array zurückgelesen gegen den eigenen Kastenfilter; ein Boden unter flachem Winkel trilinear über den Sampler und im Shader (Clamp × Repeat, Mirror mit nächster Stufe); jede Klasse vergrößert; 40 Würfel einer Liste mit 40 Texturen (sRGB und linear); 90 Texturen mehr (die Arrays wachsen von 3 auf 15 MiB); 100 × 60 bilinear und 48 × 20 nächstgelegen umgerechnet gegen eigenes Umrechnen; 4096 × 64 in Klasse 2048; MASK mit Löchern und mit einem Faktor-Alpha unter der Schwelle; was verweigert wird | 0 falsche Pixel; größte Abweichung genau gefordert ±1: gemessen 1; bilinear (±2): 1; trilinear über den Sampler (±6): 5, im Shader: 1; Stufen: 0 Byte anders |
| `mirror` (G5b) | gespiegelte Würfel und Prismen (ungerade und gerade Zahl negativer Skalierungen) neben ungespiegelten, texturiert, damit die Spiegelung der UV zu sehen ist; die Listen je Mesh und Seite; jede Instanz mit `set_transform` auf die andere Seite gegen eine neu gebaute Szene; von innen aus einem gespiegelten Würfel; 20 000 Zufallsinstanzen, etwa die Hälfte gespiegelt, gegen den exakten Test | 0 falsche Pixel; Listen (Mesh, gespiegelt, Anzahl) wie erwartet; 0 Pixel anders; nur die Löschfarbe; 2458 sichtbar, 0 fehlen, 0 zu viel |
| `gltf` (G5b) | [GLTF.md](GLTF.md#tests): drei Formen derselben Szene, was gelesen wird, kaputte Dateien, die Szene gezeichnet gegen den Strahlenwerfer (aus der Beschreibung, nicht aus dem Import; seit G5c mit allen fünf Texturen, sRGB und linear) | bestanden; gezeichnet 0 falsche Pixel |
| `lights` (G5c) | Zwei Punktlichter (eins mit Reichweite 4), zwei Spotlichter (schmal mit innerem Kegel, weit ohne), eine schwache Sonne über Boden, Kugeln (eine metallisch) und Würfeln; dasselbe Licht in `Rgba16Float`; nichts jenseits der Reichweite; 64 Lichter auf der laufenden Szene und gegen eine neu gebaute; was hochgeht; dasselbe bei 1e7; was `set_lights` verweigert. glTF: eine Datei mit fünf Lichtern aller Arten (eins zweimal von Knoten gesetzt, eins unter einem Elternknoten), was gelesen wird, wohin die Knoten sie setzen, `extensionsRequired`, jede Länge abgeschnitten (2337), 600 verfälschte, 17 gebrochene Regeln, die Grenze, die Datei gezeichnet gegen den Strahlenwerfer aus der Beschreibung | 0 falsche Pixel, größte Abweichung 1 (64 Lichter: auch); `Rgba16Float` 0 falsch, größte relative Abweichung 6,2 · 10⁻³ an einem Glanzpunkt am Rand einer Kugel (der Vergleich erlaubt dort ein Viertel dessen, was 1/64 Pixel ändert: die GPU rastet Ecken auf 1/256 Pixel; zuerst schlug er an diesem einen Pixel fehl); 3842 Pixel innerhalb der Reichweite, 0 außerhalb; 0 Pixel anders als neu gebaut und als bei 0; 1704 Byte mit den Lichtern, 496 im Bild danach; 7 von 7 verweigert. glTF: wie geschrieben, Weltmatrizen 4,4 · 10⁻¹⁶ neben den hier zusammengesetzten; 0 der abgeschnittenen laden; von den verfälschten laden 12, 588 Fehler, keine Panik; alle 17 `Invalid`; Grenze 4: `Limit`; gezeichnet 0 falsche Pixel |
| `bench_lights` (G5c) | ein kurzer Messlauf mit 64 Punktlichtern unter Validierung | 0 Fehler; die Listen wie `cull_ref` |
| `shadows` (G5c) | Eine Szene über alle vier Kaskaden (Boden, Würfel bis 58 entfernt, Kugel, Mast, gespiegelter Würfel, eine MASK-Platte mit quadratischen Löchern zur Sonne), 400 × 225, Schattenweite 90, gegen exakte Schattenstrahlen: Jedes Pixel abseits von Kanten muss eine Mischung aus seiner beschatteten und seiner besonnten Farbe zeigen (±1), und die Mischung wird gegen den Strahl beurteilt (mehr als halb auf der anderen Seite: „andere Seite“; innerhalb von 3 Pixeln eines anders entschiedenen Strahls: an einer Schattenkante, sonst „innen“, also Akne oder Lecks); als Gegenprobe dasselbe Bild gegen die Szene ohne Schatten; MASK ohne `fs_shadow`; eine Kaskade zum Vergleich; dasselbe bei 1e7 vom Ursprung; die Radien und das Raster der Kaskaden auf der CPU (40 Kameras); die Karte zurückgelesen nach einer Bewegung um 0,2 und 3,2 Texel | 0 falsch, 0 innen; an Schattenkanten 230 von 53 531 Pixeln auf der anderen Seite (0,43 %: Filter und Normalen-Versatz), bei 1e7 127 (0,24 %); ohne Schatten 2494 falsch; ohne `fs_shadow` 498 statt 230 auf der anderen Seite; eine Kaskade: 601 an Kanten (1,12 %), 11 innen (0,02 %, gröbere Texel); Radien gleich, Mitten höchstens 7,2 · 10⁻⁷ Texel neben dem Raster; Karte 0 von 4 194 304 Texeln anders, um 3 Texel verschoben 0 anders bedeckt |
| `pbr` (G5c) | 8 Kugeln dielektrisch und metallisch, Rauheit 0,25 bis 1, mit Neutral und mit Belichtung 0,6 ohne Tonemapping; dasselbe Licht in einem `Rgba16Float`-Ziel ohne Tonemapping (±2 · 10⁻³ + 2 · 10⁻³ relativ: Halbfloats haben 11 Bit); Weiß-Ofen (weiße Umgebung 1, keine Sonne, Rauheit 0 bis 1); Metallic-Roughness-Textur; Normalenkarten auf Platten (wie sie sind, um 90° gedreht mit Skala 0,8, gespiegelt, mit gespiegeltem u; nächstgelegen) und auf Kugeln (gedreht, gespiegelt mit Skala 1,5; bilinear auf Stufe 0); in welche Richtung eine Normalenkarte kippt (+y im Bild unter einer Sonne von oben: heller, −y dunkler); Emission im Dunkeln; Verdeckung mit Stärke 0,7, und ohne Grundlicht ändert sie nichts. Auf der CPU: die Albedo der BRDF durch Wichtigkeitsabtastung, die Tabelle gegen das eigene Integral, Karis' Näherung gegen dasselbe | 0 falsche Pixel, größte Abweichung 1 (bilinear ±2: 1); `Rgba16Float`: größte relative Abweichung 3,1 · 10⁻³, 0 falsch; Weiß-Ofen: Dielektrikum 1,0000 bis 1,0000, Metall 0,307 bis 1,000; die Normalenkarten ändern 7388 Pixel; Helligkeit 546 > flach 355 > 176; Verdeckung ohne Grundlicht 0 Pixel anders. CPU: weißes Metall höchstens 1,0000 (Einfachstreuung verliert bis 0,307 bei Rauheit 1); weißes Dielektrikum bei n·v 1 und 0,7: 1,000 und 1,001, bei 0,3: 1,151, bei 0,05: 1,719 (die BRDF von Anhang B ist streifend nicht energieerhaltend: `1 − F(v·h)` lässt fast die ganze Streuung stehen, während die Spiegelung wächst); Tabelle 0,0039 neben dem Integral; Karis 0,193 |

**Lichter im Browser**: Das Bild `lights` (vier Lichter, Schatten der Sonne in zwei Kaskaden) weicht in einem von 57 600 Pixeln um 1 ab, in allen drei Bündeln, gegen Voll wie gegen Basis (die Grenze des Tests: 2 Stufen, 1 % der Pixel). Vermutlich rundet der Übersetzer des Browsers eine Zeile anders (etwa ein zusammengezogenes Multiplizieren und Addieren im Kegel des Spotlichts, dessen Skala 12,9 kleine Unterschiede vergrößert); nicht weiter verfolgt.

**Im Browser** ([`tests/engine_web.mjs`](../apps/render3dtest/tests/engine_web.mjs); Bündel bauen mit `python -u build_rs.py app render3dtest --target web`, dazu das Programm für Windows, das die Vergleichsbilder schreibt): Chrome 156 kopflos, alle drei Bündel, eigenes Profil. Die Galerie (die fünf Bilder oben, eins davon an 1e7; die glTF-Szene als `.glb` im Bündel selbst geschrieben und gelesen; seit G5c dazu `pbr`: Kugeln aller Materialien, Platten mit Normalenkarte, Metallic-Roughness-Textur, Emission und Verdeckung, Belichtung 1,2, Neutral) gegen den Vulkan-Weg auf derselben Maschine: **0 von 57 600 Pixeln anders**, auch mit PBR (Wurzeln, Divisionen, die Tabelle mit eigenen Gewichten; keine Exponentialfunktion), gegen Voll wie gegen Basis, in jedem Bündel und jedem Bild, auch dem texturierten mit trilinearer Filterung über den Sampler; der Browser meldet keinen Validierungsfehler; die Zählung der Listen gleich dem exakten Test (2522). Im Browser gilt die Robustheit von WebGPU, auf Vulkan sind die Shader ohne übersetzt; am Bild ändert das nichts. Welches Backend Chrome unter WebGPU nimmt, ist nicht festgehalten, der Adapter heißt `nvidia turing`.

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

### G5c: PBR

Dieselbe Maschine und Messszene (Release, offscreen, 1080p, 300 Bilder nach 60, 3 in Flug), je drei Läufe im Wechsel mit dem G5b-Stand (e61e1ff7, im selben Wechsel gemessen), Median der Mediane, in Klammern die Spanne. GPU-Last anderer Sitzungen bei Blicken 1, 1 und 27 %.

| 1 Mio. Würfel, ms | CPU je Bild | GPU: Sichtbarkeit | GPU: Zeichnen | GPU gesamt |
| --- | ---: | ---: | ---: | ---: |
| G5b (Lambert) | 0,116 | 0,063 | 1,278 (1,269–1,518) | 1,345 |
| G5c, PBR, Neutral, Grundlicht über die Tabelle | 0,120 | 0,064 | 1,647 (1,567–1,725) | 1,713 |

Schatten (dieselbe Szene, die Sonne bei (0,3, 0,8, 0,5), vier Kaskaden zu 2048 bis 100 bei einer Szene von 300 Einheiten; je drei Läufe im Wechsel, die Spannen eng, die GPU vor dem ersten Satz bei 1 %; die 100 % vor den weiteren Sätzen zeigte `nvidia-smi` direkt nach dem eigenen Lauf davor):

| 1 Mio. Würfel, ms | Sichtbarkeit Kamera | Sichtbarkeit Kaskaden | Tiefen-Durchgänge | Hauptdurchgang | GPU gesamt |
| --- | ---: | ---: | ---: | ---: | ---: |
| PBR ohne Schatten | 0,063 | – | – | 1,565 (1,563–1,731) | 1,634 |
| 4 Kaskaden, mit `fs_shadow` (MASK richtig) | 0,063 | 0,187 | 2,821 (2,804–2,953) | 1,780 | 5,016 |
| 4 Kaskaden, nur Tiefe (MASK voll) | 0,063 | 0,187 | 2,508 (2,507–2,515) | 1,759 | 4,711 |
| 3 / 2 / 1 Kaskaden, mit `fs_shadow` (je ein Lauf) | 0,063 | 0,141 / 0,099 / 0,059 | 2,640 / 2,583 / 2,340 | 1,824 / 2,171 / 2,149 | 4,875 / 5,311 / 5,100 |

Je Kaskade (Sichtbarkeit / Tiefe, ms, Median): mit `fs_shadow` 0,041 / 0,175; 0,040 / 0,250; 0,048 / 0,506; 0,057 / 1,844; nur Tiefe 0,041 / 0,079; 0,040 / 0,185; 0,049 / 0,473; 0,057 / 1,768.

Punktlichter (dieselbe Szene ohne Schatten, die Lichter zufällig über die Szene verteilt, Reichweite ein Achtel der Ausdehnung, 37 Einheiten; je drei Läufe im Wechsel; vor dem ersten Satz 1 % GPU):

| 1 Mio. Würfel, ms | 0 | 1 | 8 | 64 | 256 Lichter |
| --- | ---: | ---: | ---: | ---: | ---: |
| Zeichnen (Spanne) | 1,577 (1,573–1,592) | 1,650 (1,647–1,675) | 2,324 (2,324–2,444) | 8,731 (8,659–8,921) | 31,616 (31,171–33,614) |
| je Licht mehr | – | 0,073 | 0,093 | 0,112 | 0,117 |

- **Schatten kosten hier 3,4 ms je Bild**, fast alles in der fernen Kaskade (Radius 100: sie sieht eine Säule von Würfeln bis zur Sonne). Hebel, nicht gebaut: kleine Werfer in fernen Kaskaden auslassen, Hi-Z für Kaskaden (G5d), eine kürzere Schattenweite.
- **Die Sichtbarkeit je Kaskade** kostet 0,04 bis 0,06 ms, wenig mehr als die der Kamera (mehr Instanzen bestehen den Test).
- **Der Hauptdurchgang** zahlt 0,2 ms für die neun Vergleiche (mit einer oder zwei Kaskaden 0,6 ms: größere Texel je Karte, weniger Treffer im Cache).

- **PBR kostet 29 % mehr Zeichenzeit** (0,37 ms). Darin: die BRDF (zwei Wurzeln, eine Normierung des Halbvektors), die vier Lesezugriffe der Tabelle, der Tonemapper, die Weltposition als Varying und ihre zwei Ableitungen, die Sicht im Fragment-Shader. Nicht weiter aufgeteilt. Die Würfel sind klein; je mehr Fläche ein Bild hat, desto mehr zählt der Fragment-Shader.

## Was nicht getestet ist

Andere GPUs (AMD, Intel, Telefone), Vulkan 1.1 und 1.2 auf echter Hardware, Linux und Android (das Programm wird dort nicht gebaut: `apps/render3dtest/TARGETS`), Firefox (kopflos kein WebGPU), Safari, 4K, ein Fenster im Dauerbetrieb, ein verlorenes Gerät mitten in der Arbeit über den Rückgabewert hinaus, Szenen über 1 Mio. Instanzen, sRGB-Ziele (ein Fenster im Browser zeigt die linearen Farben heute ungewandelt), Zeiten im Browser. Aus G5b: Texturen mit mehr Ebenen, als eine Klasse auf diesem Gerät hat (2048), eine Klasse am Limit von 256 Ebenen im Browser, Texturen größer als 2048 außer dem einen Fall 4096 × 64, eine sehr große Zahl von Texturen in einem echten glTF (die Tests schreiben ihre Dateien selbst; Beispielmodelle von Khronos wären ein Download), Dateien mit `KHR_texture_transform` und weiteren UV-Sätzen.

## Was nicht gebaut ist

Aus G5a heraus bewusst nicht: Meshes entfernen (die Pools wachsen nur), Skalierung 0 auf einzelnen Achsen (erlaubt, aber die Flächen ohne Normale werden schwarz; 0 auf allen Achsen versteckt eine Instanz sauber), Scherung (ein glTF-Knoten mit Scherung in der Weltmatrix wird ohne sie gezeichnet und gezählt), orthografische Kamera, Durchsichtigkeit (glTFs BLEND wird deckend gezeichnet), mehrere Ansichten (die Struktur ist da, `Scene` hält eine), mehr Instanzen, als ein Dispatch erreicht, MSAA (fehlt in `lib/gpu`), Schrumpfen von Kapazitäten, ein Rechen-Shader zum Bewegen. Aus G5b: Texturen entfernen, Bindless auf Voll, Mip-Stufen auf der GPU, anisotrope Filterung, beidseitige Materialien (`double_sided`: von vorn gezeichnet, gezählt), eine Pipeline ohne `discard` für deckende Listen, weitere UV-Sätze, komprimierte Texturen (BC, KTX2). Aus G5c: Cluster- oder Kachel-Shading für viele Lichter (die Messung verlangt es ab etwa acht), Listen je Alpha-Modus (sie holten die 0,31 ms von `fs_shadow` und das `discard` im Hauptdurchgang zurück; eine Entscheidung des Nutzers), Überblenden zwischen Kaskaden, Ausblenden am Ende der Schattenweite, Schatten für Punkt- und Spotlichter, Auslassen kleiner Werfer in fernen Kaskaden. Eine Sichtbarkeit je Instanz ist nur Kugel gegen sechs Ebenen: Verdeckung kommt mit G5d, und dort liegt nach G1 der größte Hebel.
