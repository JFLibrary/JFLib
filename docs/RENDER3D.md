# RENDER3D -- der Kern der 3D-Engine

`lib/render3d` zeichnet Szenen aus sehr vielen Instanzen über [`lib/gpu`](GPU.md): Meshes in gemeinsamen Puffern, Materialien mit Texturen als Tabelle, Instanzen mit Position in `f64`, Sichtbarkeit im Rechen-Shader, eine Instanzliste je Mesh, Seite und Alpha-Klasse (gespiegelte Instanzen drehen ihre Flächen um, deckende Listen zeichnet eine Pipeline ohne `discard`). Kein Befehl je Objekt, nirgends: Ein Bild sind ein Dispatch und ein indirekter Zeichenbefehl je Liste (Stufe Basis) oder ein Multi-Draw je Seite und Klasse für alle (Stufe Voll). Dieselben WGSL-Quellen laufen auf Vulkan Voll, auf Vulkan Basis und im Browser über WebGPU.

Stand: Listen je Alpha-Modus ([Listen je Alpha-Modus](#listen-je-alpha-modus)); G5d: Verdeckung der Kamera gegen eine Hi-Z-Pyramide in zwei Phasen ([Verdeckung](#verdeckung-g5d)), Detailstufen ([Detailstufen](#detailstufen-g5d)); G5c: Materialien nach PBR (glTF 2.0, Anhang B) mit allen fünf Texturen, Belichtung und Tonemapping im Fragment-Shader; kaskadierte Schatten der Sonne; Punkt- und Spotlichter (auch aus glTF über `KHR_lights_punctual`); Umgebungslicht aus sphärischen Harmonischen und einer vorgefilterten Würfel-Textur; auf G5b (Texturen, glTF-Import ([GLTF.md](GLTF.md)), gespiegelte Instanzen) und dem Kern aus G5a. G5e1: eine Szene als Teil einer jfgui-Oberfläche, unter Windows gebaut und geprüft ([In der Oberfläche](#in-der-oberfläche-g5e1)). G5e3: Schatten-Vorgabe für Stufe Basis (`Shadows::BASIS`, `Shadows::for_level`, [Entscheidung](#g5e2-schatten-auf-basis-schwache-gpus)). G5e2: dieselbe Ansicht unter Linux, im Browser im echten Fenster und unter Android, andere GPUs, sRGB für 8-Bit-Ziele ([sRGB](#srgb-g5e2)).

| Teil | Wo |
| --- | --- |
| `Renderer`, Pools, Materialien, Aufzeichnen | [`lib/render3d/lib.rs`](../lib/render3d/lib.rs) |
| `Scene`, Instanzen, Kamera, Hochladen | [`lib/render3d/scene.rs`](../lib/render3d/scene.rs) |
| Texturen: Klassen, Umrechnen, Mip-Stufen, Arrays | [`lib/render3d/texture.rs`](../lib/render3d/texture.rs) |
| ein glTF-Dokument auf die GPU | [`lib/render3d/gltf.rs`](../lib/render3d/gltf.rs); der Import selbst: [`lib/gltf`](GLTF.md) |
| Shader | [`cull.wgsl`](../lib/render3d/cull.wgsl), [`scatter.wgsl`](../lib/render3d/scatter.wgsl), [`draw.wgsl`](../lib/render3d/draw.wgsl), [`hiz.wgsl`](../lib/render3d/hiz.wgsl) (G5d), vorübersetzt in [`shaders_spv.rs`](../lib/render3d/shaders_spv.rs) |
| Die Pyramide der Verdeckung (G5d) | [`lib/render3d/hiz.rs`](../lib/render3d/hiz.rs) |
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

- **Renderer**: `Renderer::new(&device, &RendererDesc::new(farbformat))`, Felder danach einzeln ändern (`#[non_exhaustive]`, seit G5e2): `frames_in_flight` (1 bis 3 Bilder in Arbeit, Vorgabe 2), `encode_srgb` (G5e2, Vorgabe `true`: [sRGB](#srgb-g5e2)), `RendererDesc::encodes_srgb()`, `Renderer::desc()`; `add_mesh(&MeshData)`, `mesh_info(id)`, (G5d) `set_lods(mesh, &[Lod { mesh: Option<MeshId>, below }])`, `lods(mesh)`, `cull_sphere(mesh)`, `LODS_MAX` ([Detailstufen](#detailstufen-g5d)), `add_texture(&TextureData)`, `texture_info(id)`, `texture_count()`, `texture_bytes()`, `add_material(&Material)`, `set_material(id, &Material)`, `alpha_class(id)`, `set_size(b, h)`, `record(&mut encoder, &scene, farbe, tiefe)`, `multi_draw()`, `max_instances()`.
- **Scene**: `Scene::new(&renderer)`, `add(Instance)`, `remove(id)`, `set_transform`, `transform`, `set_material`, `set_mesh`, `len`, `set_camera`, `set_sun`, `set_origin`, `prepare(&mut renderer)`, `submitted(id)`, `wait_slot()`, (G5d) `set_occlusion(bool)` und `occlusion()` (die Verdeckung der Kamera, an; [Verdeckung](#verdeckung-g5d)).
- **Werte**: `Transform { pos: [f64; 3], rot: [f32; 4], scale: [f32; 3] }` (`mirrored()`: das Produkt der Skalierungen ist negativ), `Camera` (`look_at`; Sichtfeld, Seitenverhältnis, near, far, `f64::INFINITY` heißt keine ferne Ebene), `Sun { direction, color, ambient }` (Einheiten von KHR_lights_punctual: `color` ist die Beleuchtungsstärke in Lux, eine weiße matte Fläche zur Sonne zeigt `color / π`; `ambient` die Strahldichte einer gleichförmigen Umgebung; `Sun::DEFAULT`: 0,75 π und 0,25), `Scene::set_exposure(f32)` (1), `Scene::set_tonemap(Tonemap::{Neutral, None})` (Neutral).
- **Materialien** (glTFs Metallic-Roughness-Material): `Material { base_color, metallic, roughness, emissive, base_color_texture, metallic_roughness_texture, normal_texture, normal_scale, occlusion_texture, occlusion_strength, emissive_texture, alpha_cutoff }`, `Material::color(r, g, b)` (metallic 0, roughness 1), `Material::textured(TextureRef)`. Alles wirkt, wie glTF es beschreibt ([PBR](#pbr-g5c)); `alpha_cutoff: Some(c)` ist glTFs MASK. `Material::alpha_class()` → `AlphaClass::{Opaque, Mask}` (`#[non_exhaustive]`): die Listen und Pipelines seiner Instanzen ([Listen je Alpha-Modus](#listen-je-alpha-modus)).
- **Die Tabelle der Split-Sum-Näherung**: `DFG`, `DFG_N`, `dfg_table()` ([PBR](#pbr-g5c)).
- **Umgebungslicht** ([Umgebung](#umgebungslicht-g5c)): `Renderer::set_environment(&EnvironmentData::{Sky(Sky), Faces { size, rgba }})`, `Sky { zenith, horizon, ground, sun, glow }` (`Sky::DEFAULT`, `radiance(richtung)`), `Scene::set_environment(Option<f32>)` (Some(k): die Umgebung mal k statt `Sun::ambient`), zum Testen `Renderer::environment() -> Option<&Environment { size, levels, sh }>`, `env_dir`, `env_face`, `to_half`, `from_half`, `SKY_SIZE`.
- **Punkt- und Spotlichter** ([Lichter](#punkt--und-spotlichter-g5c)): `Scene::set_lights(&[Light])`, `Scene::lights()`, `Light { kind, pos, direction, color, intensity, range }`, `Light::point(pos, color, intensity, range)`, `LightKind::{Point, Spot { inner, outer }}`; aus glTF `Imported::lights` und `Imported::suns`.
- **Schatten der Sonne** ([Schatten](#schatten-der-sonne-g5c)): `Scene::set_shadows(Option<&Shadows>)`, `Shadows { cascades, size, distance, split, reach }` (`Shadows::DEFAULT`: 4 Kaskaden zu 2048, bis 100, Aufteilung 0,75, Reichweite 100, für Stufe Voll; seit G5e3 `Shadows::BASIS`: 2 Kaskaden zu 1024, sonst wie `DEFAULT`, für Stufe Basis, und `Shadows::for_level(gpu::Level)`, die Vorgabe für die Stufe eines Geräts: [Entscheidung](#g5e2-schatten-auf-basis-schwache-gpus)), `CASCADES_MAX`, `Scene::cascades() -> &[Cascade]` (was `prepare` rechnete: Ausschnitt, Mitte, Radius, Texel, Achsen, Tiefenbereich), `Renderer::set_shadow_mask(bool)` (false: Tiefen-Durchgang ohne Fragment-Shader), zum Testen `Renderer::shadow_map()`, `Scene::cascade_buffers(k)`; `FrameStats::cascades` (je Kaskade Sichtbarkeit und Tiefen-Durchgang in ms) und `FrameStats::draw`.
- **Texturen**: `TextureData { width, height, rgba, srgb, nearest }` (Zeilen von oben, 4 Byte je Texel, gerades Alpha), `TextureId`, `TextureInfo { class, size, layer, srgb, width, height, resampled }`, `TextureRef { texture, sampling }`, `Sampling { mag_nearest, min_nearest, mip: Mip, wrap_u: Wrap, wrap_v: Wrap }` (`Sampling::LINEAR`, `Sampling::NEAREST`), `Wrap::{Repeat, Clamp, Mirror}`, `Mip::{None, Nearest, Linear}`, `CLASSES`, `CLASS_MIN`, `CLASS_MAX`, `class_of(b, h)`, `mip_chain(&TextureData)` (was hochgeht, ohne Gerät).
- **glTF**: `gltf::upload(&mut renderer, &gltf::Document, szene) -> Imported { meshes, materials, default_material, textures, instances, sheared, notes }` ([GLTF.md](GLTF.md)).
- **Zum Messen und Testen**: `set_stats(true)`, `take_stats()`, `collect_stats()` (`FrameStats`: sichtbare Instanzen, Zeitstempel; G5d: `late`, die Instanzen der Phase 2, und `occlusion`, die Zeiten von Pyramide, Sichtbarkeit und Durchgang der Phase 2), `upload_info()` (`UploadInfo`: was `prepare` geschickt hat), `lists()` (`ListInfo` mit `mirrored`, `class` und `record`, dem Platz ihres Zeichenbefehls; was das letzte `prepare` festlegte), `draws_buffer()`, `visible_buffer()` (mit Verdeckung die Listen der Phase 1), `late_buffers()` (G5d: die der Phase 2), `set_culling(false)`, `texture_array(klasse, srgb)`, `Renderer::depth_texture()`, `Renderer::pyramid(&tiefenansicht)` (G5d).

Konventionen: Reverse-Z (Tiefe 1 an der nahen Ebene, `Compare::Greater`, Löschen mit `DEPTH_CLEAR`, also 0, immer `Depth32Float`), rechtshändige Welt, Vorderseiten gegen den Uhrzeigersinn (gespiegelte Instanzen behalten ihre Außenseite vorn), Licht in den Einheiten von KHR_lights_punctual mal der Belichtung, im Fragment-Shader tongemappt, Farben linear (ein sRGB-Ziel kodiert sie, für ein 8-Bit-Ziel ohne sRGB seit G5e2 der Fragment-Shader: [sRGB](#srgb-g5e2)), was das Ziel erreicht, ist deckend (Alpha 1, wie glTF es für OPAQUE und MASK verlangt). Texturkoordinate (0, 0) ist die erste Zeile der Daten. Fehler sind die von `lib/gpu`: `Invalid` für alles, was hier zu erkennen ist: tote Ids, Meshes, Materialien und Texturen, die der Renderer nicht hat (sofort bei `add`, `set_mesh`, `set_material`, `add_material`), nicht endliche Transformationen, ein negativer `alpha_cutoff`, Texturen ohne Fläche oder mit zu wenigen Bytes, ein falsches Ziel, `record` vor `prepare` oder mit Instanzen, die nach `prepare` dazukamen oder ihr Mesh oder ihre Seite wechselten (ein Wechsel der Alpha-Klasse nach `prepare` ist erlaubt: Das Bild zeigt den Stand von `prepare`). `Unsupported`, wenn eine Größenklasse mehr Texturen bekäme, als das Gerät Array-Ebenen hat. Eine Szene gehört zu dem Renderer, mit dem sie entstand; mit einem anderen verweigern `prepare` und `record` die Arbeit. Ein `prepare`, das kein Bild erreicht hat oder mittendrin scheiterte, schadet nicht: Das nächste schickt alles noch einmal.

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
| `list_first` | 16 Byte je Liste: erster Platz in der sichtbaren Liste, erste Detailstufe, Zahl der Stufen, Platz des Zeichenbefehls; dann die Stufen | `cull` |
| Sichtbare Liste, Zeichenbefehle | 4 Byte je Instanz, 20 Byte je Liste (nach Indexbreite, Klasse und Seite geordnet); je Ansicht | `cull` schreibt, Vertex-Shader und Grafik-Pipeline lesen |

Auf der CPU hält die Szene 80 Byte je Platz (`f64`-Position, Quaternion, Skalierung, Ids). Freie Plätze kommen auf eine Liste und werden zuerst wieder vergeben; ihr Radius ist `-f32::MAX`, sodass schon der erste Ebenentest sie verwirft und keine Shader-Zeile sie kennt. Eine Liste hält Platz für ihre Instanzen und verdoppelt ihn, wenn er nicht reicht, ihre Lage in der sichtbaren Liste ändert sich also selten. Kapazitäten schrumpfen nicht.

**Ein Bild** (`Renderer::record`):

| Schritt | Was geschieht |
| --- | --- |
| Barriere | gegen alle Zugriffe des Bildes davor |
| Kopien | Ansicht und Licht aus dem Upload-Puffer dieses Platzes; was sich an Materialien, Listen und (selten) an allen Instanzen geändert hat |
| `scatter` | geänderte Plätze, ein Dispatch |
| Vorlage | Zeichenbefehle mit Anzahl 0 aus einer Vorlage |
| `cull` | ein Dispatch über alle Plätze; je sichtbare Instanz ein `atomicAdd` auf die Anzahl ihrer Liste und ein Eintrag; (G5c) dasselbe je Kaskade der Sonne in deren eigene Listen; (G5d) für die Kamera mit Verdeckung `early`, nur was Phase 2 des Bildes davor sah |
| Tiefen-Durchgänge (G5c) | je Kaskade ein Durchgang in ihre Ebene der Schattenkarte, ihre Listen beidseitig ([Schatten](#schatten-der-sonne-g5c)) |
| Barriere, Durchgang | je Seite (erst wie sie sind, dann gespiegelt, mit den Pipelines, deren Vorderseiten im Uhrzeigersinn liegen) und Klasse (deckend mit `fs`, dann MASK mit `fs_mask`), nur wo Listen Platz haben: Voll: `draw_indexed_indirect_multi` über die Listen (je Indexbreite einer); Basis: je nicht leere Liste ein Push-Wert und ein `draw_indexed_indirect` |
| (G5d) Pyramide, `late`, Barriere, Durchgang | mit Verdeckung: die Pyramide aus dem Tiefenpuffer der Phase 1, die Sichtbarkeit der Phase 2 gegen sie, ein zweiter Durchgang mit dem, was neu sichtbar ist ([Verdeckung](#verdeckung-g5d)) |

**Entscheidungen und ihre Gründe.**

- **Vertex-Pulling statt fester Vertex-Eingaben.** Der erste Entwurf hatte Positionen, Normalen und die Instanznummer als Vertex-Attribute, wegen der einfachen Basis-Pfade. Er kostet in der Messszene rund drei Viertel mehr Zeichenzeit (Tabelle unten). Auf Kacheln rendernden Telefon-GPUs kann das anders aussehen: In G5e messen.
- **16-Bit-Indizes.** Der zweite Fund: Über Speicherpuffer kosten 32-Bit-Indizes auf der RTX 2070 SUPER bei gleichem Bild ein Drittel bis die Hälfte mehr als 16-Bit-Indizes (Tabelle unten; warum, ist nicht gesucht). Indizes sind relativ zur Basis-Ecke des Meshes, 16 Bit reichen also für jedes Mesh bis 65 536 Ecken. Lange Meshes gehen in einen zweiten Puffer; nebeneinanderliegende Listen gleicher Breite bleiben ein Multi-Draw.
- **Listen je Alpha-Klasse, festgelegt in `prepare`.** Begründung und Gegenwege: [Listen je Alpha-Modus](#listen-je-alpha-modus).
- **Listen im selben Durchgang gefüllt.** Die CPU kennt, wie viele Instanzen ein Mesh hat, also auch den Platz seiner Liste. Damit reicht ein Durchgang, kein Zählen vorweg und keine Präfixsumme auf der GPU.
- **Die Liste beginnt bei Voll an `first_instance`, bei Basis über einen Push-Wert.** `indirect-first-instance` ist in WebGPU und Vulkan 1.1 nicht sicher da; so gibt es einen Shader für beide Wege. Ein Push-Wert kostet je Liste wenig, und im Browser sind es dynamische Offsets auf einen schon gebundenen Puffer.
- **`scatter` statt vieler Kopien.** k geänderte Plätze wären k Kopierbefehle oder k Aufrufe von `writeBuffer`, und `Queue::write_buffer` läuft vor den Befehlen des Bildes, ohne Barriere gegen Bilder, die noch auf der GPU liegen (die Synchronisationsprüfung würde das melden). Also landet alles im Upload-Puffer des Platzes (96 Byte je Änderung), ein Dispatch verteilt es. Ab 4096 Änderungen und einem Achtel aller Plätze geht alles über einen temporären Puffer (84 Byte je Platz).
- **Fertige Matrix je Instanz, Normale ohne Rechnung je Ecke.** Eine Transformation ohne Scherung braucht für die Normale nur `m * (n * normal_scale)` mit `normal_scale = 1 / s²` je Achse. Das ist die Inverse-Transponierte, ohne sie zu speichern.
- **Material im Fragment-Shader.** Ferne Würfel haben mehr Ecken als Pixel; die Grundfarbe je Ecke zu holen, wäre teurer. PBR liest die Zeile ohnehin dort.
- **Große Koordinaten.** Auf der CPU `f64`, auf der GPU `f32` relativ zum Ursprung der Szene. Ein neuer Ursprung (`set_origin`) schickt alles einmal. Der Ursprung folgt der Kamera nicht von selbst: Wann das Hochladen sich lohnt, weiß das Programm.
- **Nichts Sichtbares darf fehlen.** `f32` rundet Kugeln und Ebenen. Beide werden deshalb um das geweitet, was das Runden kosten kann (Kugel: `2e-6` des Radius plus `5e-7` des größten Betrags ihres Mittelpunkts, Ebene ebenso). Das kostet fast nichts: Bei 1 Mio. Würfeln hielt die Liste im Prüflauf einen Eintrag mehr als `cull_ref` der Messszene (528 562 gegen 528 561, keiner fehlte).
- **Kein WGSL-Übersetzer im Programm.** Die Shader liegen wie die der UI vorübersetzt im Programm (SPIR-V 1.3, ohne Robustheit, für beide Stufen dieselben Wörter); `render3dtest --test shaders` vergleicht sie mit `wgsl::compile`, `--gen-shaders` schreibt sie neu. Der Browser bekommt den Text.

**Was die nächsten Stufen vorfinden.** Sichtbarkeit und Zeichnen nehmen eine `ViewLists` (eigener Uniform-Block, eigene Zeichenbefehle, eigene sichtbare Liste); Schatten-Kaskaden und die zweite Runde von Hi-Z sind weitere davon mit einer Pipeline ohne Fragment-Shader. Die erste `vec4f` einer Ecke reicht für reine Tiefendurchgänge. Das Material hat alle fünf Texturen von glTF (Grundfarbe, Metallic-Roughness, Normale, Verdeckung, Emission) mit ihren Faktoren; `sample_texture` in `draw.wgsl` liest jede davon, G5c muss nur die anderen vier abfragen. Die Bindungsgruppe 1 belegt 10 Texturen und einen Sampler von den 16, die WebGPU je Stufe sicher zulässt (G5c: Schattenkarte und Umgebung liegen in Gruppe 2, zusammen 12 Texturen und 3 Sampler; die Tabelle der Split-Sum ist ein Speicherpuffer in Gruppe 1).

## Texturen

**Das Bindungsmodell: ein Textur-Array je Größenklasse und Farbraum, auf allen drei Wegen.** Die Gruppe 1 hält zehn `texture_2d_array` (fünf Klassen in sRGB, fünf linear) und einen Sampler. Ein Texturwort im Material sagt Ebene, Klasse, Abtastmodus und Farbraum; der Fragment-Shader wählt das Array mit einem `switch` (jeder Fall tastet seine eigene Bindung ab). Gründe:

- **Bindless auf Voll hieße einen zweiten Shader.** `lib/wgsl` kennt kein `binding_array` (geprüft: keine Zeile dazu), und WebGPU hat es nicht im Kern; der Voll-Weg bräuchte also eigene Shader-Quellen, und „Voll gegen Basis Byte für Byte“ verglich dann zwei verschiedene Shader. Mit Arrays laufen alle drei Wege aus demselben WGSL, und die Tests zeigen es: 0 Pixel anders, auch im Browser.
- **Die Grenzen reichen überall.** Vulkan 1.1 und WebGPU garantieren 16 abgetastete Texturen und 16 Sampler je Stufe und 256 Array-Ebenen; NVIDIA meldet 2048 Ebenen.
- **Die Kosten:** Speicher für Texturen, die keine Klassengröße haben (sie werden umgerechnet, unten), und das Wachsen eines Arrays (Kopie auf der GPU). Mehr als die Ebenen des Geräts je Klasse geht nicht (`Unsupported`); dort wäre Bindless auf Voll der Ausweg, wenn eine Messung es verlangt.
- **Kein Befehl je Objekt bleibt:** weiter eine Liste je Mesh und Seite (seit den Listen je Alpha-Modus auch je Klasse), die Textur kommt je Instanz über ihr Material.

**Klassen und Umrechnen.** Klassen sind Quadrate von 128, 256, 512, 1024 und 2048 Texeln mit allen Mip-Stufen. Eine Textur anderer Größe kommt in die Klasse ihrer nächsten Zweierpotenz (128 bis 2048; glTF erlaubt das ausdrücklich) und wird auf der CPU umgerechnet: mit `nearest: true` (für Texturen, die nächstgelegen abgetastet werden) nächstgelegen, eine kleine also Texel für Texel wiederholt; sonst bilinear nach oben und mit einem Kasten nach unten, sRGB in linearem Licht. Größer als 2048 wird verkleinert. Nicht quadratische Texturen werden gestreckt (glTF adressiert über UV, das Bild bleibt).

**Mip-Stufen: ein 2 × 2-Kastenfilter auf der CPU** (`mip_chain` zeigt, was hochgeht). sRGB wird über eine Tabelle dekodiert, gemittelt und genau gerundet wieder kodiert (Schwellen-Tabelle; erst binär gesucht, dann über eine Tabelle von 4097 Startcodes mit höchstens einem Schritt, Messung unten). Gründe gegen einen Rechen-Shader: `lib/gpu` hat keine Mip-Erzeugung, sRGB-Formate sind keine Speicher-Texturen (der Shader müsste über einen Puffer schreiben und je Stufe kopieren), und der CPU-Weg ist genau, auf allen Wegen gleich und gegen einen eigenen Kastenfilter Byte für Byte prüfbar. Er kostet ungefähr so viel wie das Dekodieren des PNG oder JPEG selbst (Messung unten); wird das Laden großer Szenen zum Engpass, ist ein Rechen-Shader der Hebel (nicht gebaut).

**Abtasten.** Ein Sampler für alles: wiederholend, linear, mit linearen Mip-Stufen. Der häufigste Modus (wiederholend und trilinear) geht unverändert an ihn (`textureSampleGrad`). Die anderen macht der Shader genau, weil eine Klasse eine Zweierpotenz ist: Clamp über die Texelmitten der jeweiligen Stufe, Mirror als Clamp auf der gespiegelten Koordinate (an der Naht sind beide die Randtexel), Nearest an der Mitte eines Texels, die Stufe aus den Ableitungen, wie Vulkan sie definiert (ρ = die längere der beiden Ableitungen in Texeln, λ = log₂ ρ; nächste Stufe `floor(λ + 0.5)`, lineare zwischen `floor(λ)` und der nächsten). So braucht es keine Sampler-Tabelle und keine Fälle je Sampler. Anisotropie gibt es nicht (glTF verlangt sie nicht). Die Ableitungen sind `dpdxFine` und `dpdyFine`, gleich am Anfang des Fragment-Shaders, in gleichförmigem Kontrollfluss.

**MASK.** Ist das Alpha der Grundfarbe mal Textur kleiner als `alpha_cutoff`, verwirft der Shader das Fragment (`discard`). Bis zu den Listen je Alpha-Modus lief alles Deckende (Grenzwert −1, nie verworfen) durch dieselbe Pipeline; seitdem zeichnet es `fs` ohne `discard`, MASK `fs_mask` ([Listen je Alpha-Modus](#listen-je-alpha-modus)). BLEND ist nicht Teil von G5b.

**Hochladen.** `add_texture` schreibt alle Stufen über `Queue::write_texture` (vor der nächsten Übergabe, nach einer Barriere aus `Sampled`: Bilder, die das Array noch lesen, sind fertig). Ein volles Array wächst auf die doppelte Zahl von Ebenen (zuerst 4): ein neues Array, die alten Ebenen über einen Puffer von höchstens 64 MiB kopiert (`lib/gpu` hat keine Kopie von Textur zu Textur; die Zeilen sind dabei auf 256 Byte ausgerichtet, wie WebGPU es will), die Bindungsgruppe neu, das alte Array geht, wenn die GPU fertig ist. Klassen ohne Textur halten ein 1 × 1-Array, weil jede Bindung etwas halten muss.

## Gespiegelte Instanzen

Eine Transformation mit negativer Determinante (eine ungerade Zahl negativer Skalierungen) kehrt die Reihenfolge der Ecken um. Solche Instanzen kommen in eine eigene Liste ihres Meshes und werden mit einer zweiten Pipeline gezeichnet, deren Vorderseiten im Uhrzeigersinn liegen; so bleiben Rückseiten verworfen und die Außenseite vorn. Die Normale braucht nichts Neues: `m · (n / s²)` ist mit dem Vorzeichen in `m` genau die Inverse-Transponierte `r · (n / s)`. Die Kugel der Sichtbarkeit nimmt die Beträge der Skalierungen. Ein `set_transform`, das die Seite wechselt, verschiebt die Instanz in die andere Liste wie ein `set_mesh` (mit dem nächsten `prepare`). Auf Voll bleibt es bei einem Multi-Draw je Seite (seit den Listen je Alpha-Modus je Seite und Klasse: Die Zeichenbefehle liegen nach Indexbreite, Klasse und Seite); auf Basis ein Befehl je Liste wie bisher. Zwei negative Skalierungen sind eine Drehung um 180° und bleiben in der gewöhnlichen Liste.

## Listen je Alpha-Modus

**Eine Liste je Mesh, Seite und Alpha-Klasse** (Entscheidung des Nutzers vom 09.11.2026). `AlphaClass::Opaque` und `AlphaClass::Mask` (`Material::alpha_class`: `Mask` mit `alpha_cutoff`, sonst `Opaque`; glTFs BLEND wird weiter deckend gezeichnet und gehört zu `Opaque`). Deckende Listen zeichnet eine Pipeline, deren Fragment-Shader `fs` kein `discard` enthält (im SPIR-V kein `OpKill`, geprüft), MASK-Listen eine mit `fs_mask`; beide schattieren über dieselbe Funktion `shade`, `fs_mask` verwirft davor, was unter dem Grenzwert liegt. Im Schatten zeichnen deckende Werfer immer nur Tiefe (`shadow_depth`, kein Fragment-Shader), MASK-Werfer mit `fs_shadow` (ihre Löcher), nach `Renderer::set_shadow_mask(false)` auch nur Tiefe (volle Schatten).

- **`prepare` legt die Liste fest.** Das Material und damit die Klasse hält der Renderer, die Listen hält die Szene, und `Scene::add`, `set_material` und die anderen haben keinen Renderer in der Hand. Deshalb markieren `add`, `remove`, `set_mesh`, `set_transform` und `set_material` nur den Platz, und `prepare` vergleicht (Mesh, Seite, Klasse des Materials) mit der Liste, in der der Platz steht; wechselt etwas, kommt er in die andere Liste, und `list_of` geht über den vorhandenen Weg (`scatter` oder alles) hoch. Ändert `Renderer::set_material` die Klasse eines Materials, zählt der Renderer das mit; jede Szene sieht dann beim nächsten `prepare` alle Plätze an (selten, linear in den Plätzen) und schickt die verschobenen. `Scene::lists()` zeigt seitdem, was das letzte `prepare` festlegte (vorher galt eine neue Instanz sofort). Eine Klasse, die nach `prepare` wechselt, sperrt `record` nicht: Das Bild zeigt den Stand von `prepare`, auch das Material selbst geht erst mit dem nächsten hoch (der Test prüft beides). Verworfen: eine Tabelle der Klassen, die Szene und Renderer teilen; ohne `std` und ohne `unsafe` gibt es dafür keinen wachsenden, geteilten Speicher, und eine Instanz vorläufig in die deckende Liste zu stecken, ließe für jedes Mesh mit nur MASK-Instanzen eine leere Liste zurück.
- **Die Zeichenbefehle liegen nach Indexbreite, Klasse und Seite.** Listen entstehen in der Reihenfolge, in der Instanzen sie brauchen; eine deckende Liste nach einer MASK-Liste zerrisse die Läufe des Multi-Draws. Deshalb liegt der Zeichenbefehl einer Liste nicht mehr an ihrem Index, sondern an `ListInfo::record`: Ändert sich der Aufbau der Listen (eine neue Liste, mehr Platz), sortiert `prepare` die Befehle und schreibt den Platz in das vierte, bisher leere Wort ihres Eintrags in `list_first`, das `cull.wgsl` ohnehin liest (kein Zugriff mehr). Die Kennung einer Liste, der Wert in `list_of`, bleibt; eine neue Liste schickt also keine Instanz neu. Voll zeichnet so je Seite, Klasse und Durchgang einen Multi-Draw (je Indexbreite einer, wie bisher), und nur Kombinationen, deren Listen Platz haben (`room`, je `prepare` neu bestimmt). Die Tiefen-Durchgänge der Schatten fassen benachbarte Läufe mit derselben Pipeline und Indexbreite zusammen; ohne Rückseiten-Culling sind beide Seiten dort gleich, meist ist es also ein Multi-Draw je Klasse, ohne `set_shadow_mask` einer für alles. Basis: weiter ein Befehl je Liste mit Platz, in der Reihenfolge der Befehle.
- **Detailstufen**: Die Liste einer Stufe hat dieselbe Seite und Klasse wie die Ausgangsliste, denn das Material bleibt das der Instanz; `fit_lists` legt sie an und rechnet ihren Platz.
- **Verdeckung**: Beide Phasen zeichnen beide Klassen. MASK schreibt in Phase 1 Tiefe nur, wo es nicht verwirft (ein verworfenes Fragment schreibt keine Tiefe), die Pyramide sieht also durch die Löcher: konservativ. Geprüft im Test `alpha` mit einer Wand mit Fenstern vor 1500 kleinen Würfeln (ein Drittel MASK, ein Drittel gespiegelt) über sechs Bilder einer Fahrt: mit und ohne Verdeckung 0 Pixel anders, gegen den Strahlenwerfer 0 falsch, die Wand verdeckt (1259 von 1524 im Kegel gezeichnet), Phase 2 zeichnet beide Klassen (238 deckende, 113 MASK). **Mutation**: MASK-Listen in Phase 1 mit der deckenden Pipeline (Tiefe auch in den Löchern) lässt 12 Prüfungen je Stufe fehlschlagen, darunter jeden Vergleich mit dem Strahlenwerfer, der eine MASK-Instanz sieht, und die Fahrt (0 von 6 Bildern gleich).
- **Barrieren: keine geändert, keine neu.** Gelesen gegen die Zugriffe: `list_first` schreibt weiter nur die Kopie am Anfang des Bildes (vor der Barriere nach den Kopien), `cull` liest sein viertes Wort wie die anderen drei; die Zeichenbefehle schreibt weiter nur `cull` (jetzt an `record` statt am Index, derselbe Puffer, dieselbe Barriere davor); die neuen Pipelines lesen dieselben Bindungen wie die alten, ein Wechsel der Pipeline in einem Durchgang braucht keine Barriere. Die Validierung (mit der Heuristik für Shader-Zugriffe) zeigt 0 Fehler.
- **Eine dritte Klasse** (durchsichtig) ist vorbereitet, nicht gebaut: Die Klasse ist ein Index im Schlüssel der Listen (`ALPHA_CLASSES`), die Pipelines des Hauptdurchgangs sind ein Feld je Klasse und Seite, die Zeichenbefehle liegen nach Klasse. Dazu bräuchte sie Mischen, eine Sortierung von hinten nach vorn (je Instanz, nicht je Liste), einen eigenen Durchgang nach den anderen ohne Schreiben der Tiefe und keinen Platz in der Pyramide.
- **Gemessen** ([Messwerte](#messung-der-listen-je-alpha-modus)): Hauptdurchgang 9 bis 23 % schneller (1 Mio. Würfel, PBR), die 0,31 ms von `fs_shadow` für deckende Werfer sind weg; dafür kostet die Sichtbarkeit mehr, wenn beide Klassen in denselben Arbeitsgruppen liegen (0,064 → 0,145 ms ohne Verdeckung, die Szene wird trotzdem schneller).
- **Mehrere Szenen eines Renderers**: Ändert ein `Renderer::set_material` die Klasse zwischen dem `prepare` von Szene A und ihrem `record`, und bereitet Szene B dazwischen vor, zeichnet A ein Bild lang mit der neuen Materialzeile in der alten Liste (die Tabelle der Materialien geht mit dem ersten `record` nach einem `prepare` hoch, das war schon so). Aus deckend in MASK fehlen dann die Löcher für dieses Bild; umgekehrt verwirft `fs_mask` mit dem Grenzwert −1 nichts. Nicht getestet.

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
- **MASK im Schatten** (seit den [Listen je Alpha-Modus](#listen-je-alpha-modus) zeichnen deckende Werfer immer nur Tiefe, MASK-Werfer mit `fs_shadow`, außer nach `set_shadow_mask(false)`; das Folgende ist der Stand von G5c): beide Wege gemessen: Tiefen-Durchgang ohne Fragment-Shader (MASK wirft volle Schatten) gegen einen mit `fs_shadow`, das die Materialzeile liest und nur für MASK verwirft. **Gebaut ist der richtige Weg mit Fragment-Shader**: Die Löcher lassen die Sonne durch (der Test sieht es: ohne ihn 504 Pixel anders, 209 davon dunkler, gegen die Strahlen 498 statt 230 auf der falschen Seite). Er kostet in der Messszene ohne ein einziges MASK-Material 0,31 ms je Bild (12 % der Tiefen-Durchgänge; die nahen Kaskaden am meisten, 0,175 statt 0,079 ms: Ein Fragment-Shader mit `discard` nimmt dem Durchgang den frühen Tiefentest). `Renderer::set_shadow_mask(false)` ist der schnelle Weg für Szenen ohne MASK. Listen je Alpha-Modus holten beides.
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

**Eine Liste im Speicherpuffer** (`Scene::set_lights`, 64 Byte je Licht: Position relativ zum Ursprung und Reichweite, Farbe mal Intensität und Art, Achse, Kegel als Skala und Versatz des Kosinus), Gruppe 0, Bindung 6, nur der Fragment-Shader liest sie. Sie geht mit dem nächsten Bild hoch, wenn sie sich änderte, der Ursprung wanderte oder ein `prepare` sein Bild nicht erreichte; sonst kostet sie nichts (ein Bild ohne Änderung schickt weiter 496 Byte; seit G5d 688, der Uniform-Block der Ansicht wuchs). Der Fragment-Shader läuft für jedes Fragment über alle Lichter.

- **Abfall nach `KHR_lights_punctual`**: Intensität in Candela, `1 / d²`, mit Reichweite das Fenster `clamp(1 − (d / Reichweite)⁴, 0, 1)` (der Test: das Licht der Reichweite 4 ändert 3842 Pixel innerhalb, 0 außerhalb); Spot: `t = clamp(cos · Skala + Versatz, 0, 1)`, `t²`, mit `Skala = 1 / max(0,001, cos innen − cos außen)` und `Versatz = −cos außen · Skala`, beides auf der CPU in `f64` (kein Kosinus im Shader). Dieselbe BRDF wie die Sonne. Keine Schatten (offen).
- **Kosten je Licht** (unten): etwa 0,11 ms je Licht und Bild in der Messszene, auch wenn fast alle Fragmente außerhalb der Reichweiten liegen: Die Schleife lädt jedes Licht für jedes Fragment. **Gebaut ist keine Aufteilung in Cluster oder Kacheln**: Die Messung verlangt sie ab etwa acht Lichtern (acht kosten 0,75 ms, 64 schon 7,2 ms, 256 30 ms), aber sie ist ein eigener Baustein (ein Rechen-Shader, der die Lichter in Cluster des Sichtvolumens sortiert, Indexlisten je Cluster, der Fragment-Shader schlägt seinen Cluster nach) und blieb nicht im Rahmen dieser Stufe. Offener Punkt.
- **Aus glTF** ([GLTF.md](GLTF.md)): `render3d::gltf::upload` setzt Punkt- und Spotlichter an die Stelle ihres Knotens (Achse: dessen −z) in `Imported::lights`; gerichtete Lichter werden zu `Imported::suns` (Richtung zur Sonne, Grundlicht 0), mehr als eines ist eine Notiz (eine Szene hat eine Sonne).

## Umgebungslicht (G5c)

**Diffus über 9 sphärische Harmonische, spiegelnd über eine vorgefilterte Würfel-Textur mit Mip-Stufen je Rauheit**, beides mit der Tabelle der Split-Sum ([PBR](#pbr-g5c)) zusammengesetzt wie das gleichförmige Grundlicht: `spiegelnd · (f0 A + B) + diffus · c_diff · (1 − (f0 A + B))`, mal Verdeckung. `Renderer::set_environment` rechnet alles einmal auf der CPU in `f64`; `Scene::set_environment(Some(k))` nimmt es mal `k` statt `Sun::ambient` (None, die Vorgabe: das gleichförmige Grundlicht wie bisher; das Bild ist dann Pixel für Pixel dasselbe wie in einer Szene, die nie eine Umgebung hatte).

- **Quelle**: ein prozeduraler Himmel (`Sky`: Zenit, Horizont, Boden, ein weicher Schein um die Sonne mit cos⁸; überall glatt, keine Sonnenscheibe) oder sechs Seiten des Aufrufers (`Faces`: Zweierpotenz 4 bis 1024, RGBA `f32` linear, Seiten in der Reihenfolge von Vulkan; RGBE, Halbfloats und Ähnliches rechnet der Aufrufer um). Kein Download, keine Datei: Die Tests erzeugen ihre Umgebungen.
- **Diffus: 9 Koeffizienten im Uniform-Block der Lichter** (kein Texturplatz): die Projektion der Quelle (ein Himmel über 32 × 32 Punkte je Seite, Seiten über ihre Texel, nach Raumwinkel gewichtet und auf 4π normiert), mal den Faktoren der Kosinuskeule (π, 2π/3, π/4 nach Ramamoorthi und Hanrahan), geteilt durch π. Der Shader wertet ein Polynom in der Normalen aus, keine Textur, keine transzendente Funktion. Gemessen: 9 Koeffizienten geben die Bestrahlungsstärke des Himmels auf 1,7 % genau wieder (24 Normalen gegen das Integral).
- **Spiegelnd: eine `Rgba16Float`-Würfel-Textur**, Stufe 0 die Quelle (ein Himmel mit 64 × 64 je Seite), Stufe k die GGX-Faltung für Rauheit k / (Stufen − 1) mit N = V = R (128 Hammersley-Punkte je Texel; eine Seiten-Quelle wird auf der Stufe ihrer eigenen Kastenfilter-Kette gelesen, die der Raumwinkel des Punkts verlangt), **bis 4 × 4 herab**. Der Shader liest mit `textureSampleLevel` entlang der Spiegelung bei `roughness · (Stufen − 1)`. Gemessen: die Stufen 2,4 % neben einer Faltung mit 8192 Punkten (das Rauschen der 128 Punkte). Die Stufen enden bei 4 × 4, weil die breiteste Keule nicht weniger Texel braucht und weil eine 1 × 1-Seite beim nahtlosen Filtern nur aus Ecken besteht (zuerst gingen sie bis 1 × 1: der Test sah bei Rauheit 1 Abweichungen bis 6 Stufen, die Referenz kannte die Eckenregel von Vulkan nicht; mit Eckenregel und 4 × 4 bleibt die größte Abweichung 1).
- **Split-Sum mit der Tabelle**, nicht mit einer analytischen Näherung: dieselbe Tabelle wie das Grundlicht (Karis' Näherung lag 0,19 daneben). Für einen gleichförmigen Himmel ist die Split-Sum-Näherung genau (der Weiß-Ofen aus sechs weißen Seiten zeigt ein weißes Dielektrikum mit 1,0000).
- **Bindungen**: Die Würfel-Textur und ihr Sampler (linear, lineare Stufen) liegen in Gruppe 2 neben der Schattenkarte (Bindungen 2 und 3); vor dem ersten `set_environment` hält sie einen weißen 1 × 1-Würfel. Gruppe 2: 2 Texturen und 2 Sampler, Gruppe 1: 10 Texturen und 1 Sampler: zusammen 12 von 16 Texturen je Stufe. `lib/wgsl` und `lib/gpu` konnten `texture_cube`, `textureSampleLevel` auf Würfeln, Würfel-Texturen mit Stufen und ihr Hochladen schon.
- **Hochladen**: `Queue::write_texture` je Stufe, alle sechs Seiten auf einmal, `Undefined` → `Sampled`; die Übergabe danach trägt die Barriere der Uploads (Transfer → alle Befehle, `vk_dev.rs`). Eine neue Umgebung ist eine neue Textur; die alte geht, wenn die GPU fertig ist (Bilder in Flug lesen weiter die alte).
- **Zeit**: `set_environment(Sky)` 210 bis 340 ms im Testprogramm (Debug-Optionen der Bibliothek), vor allem die Faltung.

## Verdeckung (G5d)

**Zwei Phasen gegen eine Hi-Z-Pyramide, für die Kamera.** Phase 1 (`early` in `cull.wgsl`) nimmt, was Phase 2 des Bildes davor sichtbar fand und was im Sichtkegel liegt, und zeichnet es. Dann baut ein Rechen-Shader (`hiz.wgsl`) aus dem Tiefenpuffer eine Pyramide der kleinsten Tiefe (Reverse-Z: das Fernste). Phase 2 (`late`) prüft jede Instanz im Sichtkegel gegen sie, schreibt für jede ein Bit für das nächste Bild (verdeckt oder nicht) und nimmt in ihre Listen, was sichtbar ist und Phase 1 nicht gezeichnet hat; ein zweiter Durchgang zeichnet das. Normale und gespiegelte Listen, Voll, Basis und der Browser laufen durch denselben Code aus demselben WGSL. Die Schatten-Kaskaden haben keine Verdeckung (nicht verlangt; dort wäre die Pyramide eine je Kaskade).

| Schritt | Was geschieht |
| --- | --- |
| `early` | je Platz: Bit des Bildes davor gesetzt (verdeckt)? sonst Sichtkegel, dann in die Liste (die Listen der Kamera, `draws_buffer()`) |
| Durchgang der Phase 1 | Farbe und Tiefe mit `load` und `before` des Aufrufers, beide gespeichert; die Tiefe endet in `Layout::Sampled` |
| Pyramide | `Undefined → General` für alle Stufen; je Stufe ein Dispatch (8 × 8) und `General → Sampled` für diese Stufe |
| `late` | je Platz: Sichtkegel, dann der Test gegen die Pyramide; neu sichtbar: in die Listen der Phase 2 (`late_buffers()`); die Bits einer Arbeitsgruppe (256 Plätze, 8 Wörter) in Workgroup-Speicher gesammelt und nach der Barriere geschrieben |
| Durchgang der Phase 2 | `load: Keep`, Tiefe `Sampled → DepthTarget`, endet mit `store` und `after` des Aufrufers |

**Die Pyramide.** Stufe k (ab 1) hält das Minimum der Pixel `[x·2^k, (x+1)·2^k) × [y·2^k, (y+1)·2^k)`, am Bildrand abgeschnitten, also `⌈b/2^k⌉ × ⌈h/2^k⌉` Texel, bis 1 × 1. Sie liegt in einer quadratischen `R32Float`-Textur von 2^(Stufen−1) Texeln, Mip k−1 ist Stufe k: Mip-Stufen einer Textur halbieren mit Abrunden, die Stufen hier brauchen Aufrunden (1080 → 540 → 270 → 135 → 68 statt 67); was jenseits des Bildes liegt, liest niemand (1080p: 1024 × 1024, 11 Stufen, 5,6 MiB). Ein Texel nimmt die 2 × 2 darunter, bei ungerader Größe den Rand doppelt. Die halbe Auflösung als erste Stufe spart einen Durchgang über den ganzen Tiefenpuffer; ein Objekt kleiner als ein Pixel wird gegen Stufe 1 geprüft, das ist konservativ.

**Der Test.** Die Kugel einer Instanz (dieselbe wie im Sichtkegel, schon gegen das Runden von `f32` geweitet) wird noch einmal geweitet (×1,0001 plus 4 · 10⁻⁶ der größten Koordinate von Mittelpunkt und Auge) und im Raum der Kamera als Würfel um die Kugel projiziert: Die Grenzen in x/z und y/z sind Minimum und Maximum über die nahe und ferne Seite (`(c ± r) / (z ∓ r)`), ohne Wurzel und ohne Auslöschung, dafür etwas weiter als die genaue Projektion der Kugel. Das Rechteck in Pixeln wird auf jeder Seite um einen Pixel größer; die Stufe ist die erste, in der es höchstens 2 × 2 Texel trifft (`k = firstLeadingBit(Ausdehnung) + 1`, mindestens 1). Verdeckt ist die Instanz, wenn ihre nächste Tiefe (`zz + zw / (z − r)`) kleiner ist als das Minimum dieser Texel. Schneidet die Kugel die nahe Ebene, gilt sie als sichtbar. Die Skalen der Projektion stehen seit G5d im Uniform-Block der Ansicht (`proj`; mit `lod` für die Detailstufen 208 statt 176 Byte), Größe des Ziels und Zahl der Stufen kommen als Push-Werte.

**Nichts darf fehlen, auch nicht beim Sprung.** Phase 2 prüft immer gegen die Pyramide dieses Bildes, die nur aus Instanzen entsteht, die wirklich gezeichnet wurden; was sie verdeckt, ist im fertigen Bild verdeckt. Vom Bild davor bleibt nur, *welche* Instanzen Phase 1 zeichnet, und das ändert nur die Zeit. Deshalb fällt nichts zurück: Ein Kamerasprung macht Phase 1 nutzlos (sie zeichnet, was vorher sichtbar war und jetzt im Kegel liegt), die Pyramide ist dünn, Phase 2 zeichnet fast alles; ab dem nächsten Bild stimmen die Bits wieder. Eine neue Größe des Ziels ist eine neue Tiefenansicht und damit eine neue Pyramide. Ein neuer Ursprung, neue Kapazitäten und jedes Hochladen aller Plätze löschen alle Bits (alles gilt als gesehen, das erste Bild zeichnet alles im Kegel in Phase 1); eine geänderte Instanz (`scatter`) löscht ihr Bit, sie kommt also in Phase 1. Entschieden habe ich das „alles als sichtbar markieren“, nicht das Zurückfallen ohne Verdeckung: Es kostet ein Bild, in dem Phase 1 mehr zeichnet, und braucht keinen zweiten Weg.

**Phase 1 und die Pyramide des Bildes davor.** Verlangt war, dass Phase 1 gegen die Pyramide des Bildes davor prüft. Das tut sie mittelbar: Das Bit ist das Ergebnis dieses Tests, gerechnet in Phase 2 des Bildes davor, mit der Kamera und den Instanzen, zu denen die Pyramide gehört. Den Test zu Beginn des neuen Bildes zu wiederholen, gäbe mit derselben Kamera dasselbe; mit der neuen Kamera passte die Pyramide nicht mehr zu den Projektionen (sie wäre zu einer anderen Kamera gerastert, ein Reprojizieren ist nicht gebaut). Für die Richtigkeit ist es gleich, was Phase 1 zeichnet; die Pyramide hält dann weniger Verdecker.

**Bindungen und das Ziel.** Die Pyramide gehört zum Tiefenanhang: Der Renderer hält je Tiefenansicht, die er in den letzten 16 Aufzeichnungen sah, eine (eine Kette, die `record` mit `&self` verlängern kann, weil der Kodierer borgt, was er bindet; `Scene::prepare` mit `&mut` räumt auf). Erkannt wird die Ansicht an `TextureView::same` (neu in `lib/gpu`, additiv), ob sie lesbar ist, an `TextureView::usage` (dort jetzt öffentlich). Ohne `TextureUsage::SAMPLED` am Tiefenpuffer, ohne Culling (`set_culling(false)`) oder mit `set_occlusion(false)` bleibt es bei einem Durchgang wie in G5c. Der eigene Tiefenpuffer des Renderers (`set_size`) ist jetzt lesbar. Die Bindungen: `cull` hat in Gruppe 0 eine Bindung mehr (die Bits), `late` in Gruppe 1 die Pyramide als `texture_2d<f32>` (im Browser `unfilterable-float`); die Pyramide wird über `textureLoad` je Stufe gelesen und über eine `texture_storage_2d<r32float, write>` je Stufe geschrieben. Zwei Layouts für den Bau, weil der Browser einen Tiefen- von einem Float-Eingang im Layout unterscheidet.

**`draw_indexed_indirect_count` mit Zusammenschieben leerer Listen: nicht gebaut.** In der Messszene gibt es eine Liste; leere Listen kosten auf Voll im Multi-Draw einen leeren Befehl, auf Basis nur CPU (je Liste ein Befehl, wie bisher). Eine Messung, die das trägt, gab es nicht.

**Reihenfolge.** Mit zwei Phasen zeichnet die GPU Instanzen in anderer Reihenfolge. Fragmente gleicher Tiefe (genau gleiche, überlappende Flächen) entscheidet die Reihenfolge; die war schon vorher nicht fest (die Listen füllt ein `atomicAdd`).

**Barrieren** (selbst gegen die Zugriffe der Shader gelesen):

| Wo | Was davor | Was danach | Warum |
| --- | --- | --- | --- |
| Bits löschen (`fill_buffer`, nur wenn alle Plätze neu hochgehen oder der Puffer neu ist), danach eine Barriere, wenn auch `scatter` läuft | Kopie schreibt | Rechen-Shader liest und schreibt | `scatter` löscht die Bits geänderter Plätze mit `atomicAnd` (Lesen, Ändern, Schreiben). **Mutation**: ohne sie meldet die Validierung im Test `pixels` 10 Fehler (`READ_AFTER_WRITE`, „Shader stage COMPUTE reads VkBuffer … previously written by vkCmdFillBuffer“) |
| nach den Kopien (wie G5c, jetzt auch nach `COPY_READ`) | Kopie, `scatter` | Rechen-Shader, Vertex- und Fragment-Shader, Kopie | die Zeichenbefehle der Phase 2 aus der Vorlage; die Kopie für die Statistik liest sie, wenn Phase 2 nicht läuft |
| Ende des Durchgangs der Phase 1 (Tiefe `DepthTarget → Sampled`) | späte Fragment-Tests, Schreiben der Tiefe | Vertex-, Fragment-, Rechen-Shader, Lesen | `from_depth` liest den Tiefenpuffer (Lesen nach Schreiben). **Mutation**: mit `after: DepthTarget` meldet die Validierung 30 Fehler (`READ_AFTER_WRITE` „Shader stage COMPUTE reads VkImageView … written … by the attachment storeOp“, dazu falsche Layouts); die Bilder blieben richtig |
| Anfang des Baus (alle Stufen `Undefined → General`) | Farbausgabe, Kopie, Rechen-Shader (ohne Zugriff) | alle Befehle, Lesen und Schreiben | die Pyramide wird ganz neu geschrieben, der alte Inhalt darf gehen; die Rechen-Shader der Phase 2 des Bildes davor haben sie gelesen (Schreiben nach Lesen) |
| nach jeder Stufe (diese Stufe `General → Sampled`) | alle Befehle, Schreiben | Vertex-, Fragment-, Rechen-Shader, Lesen | die nächste Stufe liest sie, Phase 2 liest alle (Lesen nach Schreiben, und das Layout des Deskriptors). **Mutation**: ohne sie 30 Fehler (Layouts, `VUID-vkCmdDispatch-imageLayout-00344`); auf dieser GPU auch falsche Bilder (bis 112 Pixel anders, 8 sichtbare Instanzen fehlten) |
| Anfang des Durchgangs der Phase 2 (Tiefe `Sampled → DepthTarget`) | Shader (ohne Zugriff) | frühe und späte Fragment-Tests, Lesen und Schreiben | der Bau der Pyramide hat gelesen, der Tiefentest schreibt wieder (Schreiben nach Lesen) |
| Farbe zwischen den Durchgängen (`ColorTarget → ColorTarget`) | Farbausgabe, Schreiben | Farbausgabe, Lesen und Schreiben | zwei Durchgänge schreiben dasselbe Ziel |
| nach `late` | Rechen-Shader schreibt | indirekte Befehle, Vertex-Shader, Kopie | die Zeichenbefehle und Listen der Phase 2 vor ihrem Durchgang (und der Kopie für die Statistik). **Mutation**: ohne sie 10 Fehler (`READ_AFTER_WRITE`, „vkCmdDrawIndexedIndirect … VERTEX reads VkBuffer … written by vkCmdDispatch“), und auf dieser GPU falsche Bilder (bis 515 Pixel) |
| Bits zwischen Phase 1 und Phase 2 | Phase 1 liest | Phase 2 liest und schreibt | Schreiben nach Lesen: gedeckt durch die Barrieren dazwischen (nach den Kulls, die der Pyramide mit Quelle „alle Befehle“) |
| Bits zwischen den Bildern | Phase 2 schreibt | `scatter` und Phase 1 des nächsten Bildes | die Barriere am Anfang des Bildes (Rechen-Shader schreibt → Rechen-Shader). **Mutation** (Barriere am Anfang ohne Quelle): 0 Meldungen, auch mit sieben Bildern in Flug (`frames`). Der Grund steht seit Schritt 3 fest: Die Schicht (1.4.363) prüft nicht zwischen Übergaben, auch nicht auf einer Warteschlange (`gputest --test queues`, [GPU.md](GPU.md#rechen-warteschlange-g5d)); jedes Bild ist eine Übergabe. Gelesen: Die Kette aus der Barriere nach `late` (ihr Ziel schließt Kopien ein) und der Barriere nach den Kopien des nächsten Bildes deckt das Lesen der Phase 1 auch ohne sie; `scatter` liegt vor jener Barriere und braucht die am Anfang. Sie bleibt |

## Detailstufen (G5d)

**Ein Mesh bekommt Stufen, die der Aufrufer vorgibt**: `Renderer::set_lods(mesh, &[Lod])`, je Stufe ein anderes Mesh (oder `None`: gar nicht zeichnen) und eine Schwelle; die Schwellen fallen, höchstens `LODS_MAX` (8). Die Größe einer Instanz ist der Durchmesser ihrer Kugel über der Höhe der Ansicht, `r · fy / d` mit dem Abstand d zum Auge (nicht der Tiefe entlang der Blickrichtung: Ein Drehen der Kamera wechselt keine Stufe). Eine Instanz nimmt die letzte Stufe, deren Schwelle über ihrer Größe liegt. Gewählt wird in der Sichtbarkeit je Instanz, in allen Wegen (`main`, `early`, `late`), ohne einen Befehl mehr.

- **Listen**: Jede Stufe ist die Liste ihres Meshes, ihrer Seite und ihrer Klasse (gespiegelte Instanzen kommen in die gespiegelte Liste des Meshes der Stufe, MASK-Instanzen in seine MASK-Liste: Das Material bleibt das der Instanz). Eine Liste hat jetzt Platz für ihre eigenen Instanzen und für die der Meshes, die sie als Stufe nehmen (`ListInfo::room`); die Listen der Stufen legt `prepare` an. Der Puffer `list_first` hält je Liste 16 Byte (erster Platz, erste Stufe, Zahl der Stufen, seit den Listen je Alpha-Modus der Platz des Zeichenbefehls) und danach die Stufen (Schwelle, Liste); er geht mit dem Aufbau der Listen hoch.
- **Die Kugel**: Die Sichtbarkeit (Sichtkegel und Verdeckung) nimmt eine Kugel, die jede Stufe einschließt (`cull_sphere`: um den Mittelpunkt des Meshes, so groß, dass die Kugel jeder Stufe hineinpasst). Sonst könnte eine Stufe, die über ihr Mesh hinausreicht, falsch weggelassen werden. Ein `set_lods` lässt jede Szene beim nächsten Bild alles neu hochladen.
- **Schatten nehmen die Wahl der Kamera** (Auge und `fy` der Kamera stehen im Uniform-Block jeder Ansicht, `lod`): Ein Schatten zeigt dasselbe Mesh wie sein Werfer; mit der Größe in der Kaskade wählte die ferne Kaskade gröbere Meshes, und die Kante von Schatten und Werfer passte nicht mehr zusammen (der Bias der Empfänger ist auf das Mesh selbst ausgelegt).
- **Keine Vereinfachung in der Bibliothek**: Der Aufrufer gibt die Meshes; `render3dtest` erzeugt sie (Kugeln mit weniger Segmenten). glTF bleibt, wie es ist (kein `MSFT_lod`).

**Kleine Objekte auslassen (contribution culling) passt auf denselben Weg**: eine Stufe ohne Mesh. Sie kostet nichts, wenn sie keiner setzt, und gilt wie jede Stufe auch für die Schatten. Für die fernen Schattenkaskaden habe ich zusätzlich eine eigene Grenze in Texeln der Kaskade gebaut und gemessen (eine Wegwerf-Änderung, `Shadows::min_size`): In der Messszene sind die Würfel in der fernsten Kaskade 10 bis 30 Texel groß. Bis 4 Texel ließ sie nichts aus (Tiefe der fernsten Kaskade 1,81 ms gleich), mit 10 Texeln 1,65 ms, mit 30 Texeln fiel alles weg. Ihre 1,8 ms kommen also von der Zahl der Werfer, nicht von winzigen Werfern; eine Grenze, die etwas spart, nähme sichtbare Schatten weg. Die Messung trägt es nicht, die Grenze ist wieder entfernt.

## Zweite Warteschlange (G5d)

`lib/gpu` hat seit G5d eine Rechen-Warteschlange, nur auf Wunsch (`DeviceOptions::compute_queue`,
[GPU.md](GPU.md#rechen-warteschlange-g5d)). **render3d benutzt sie nicht**: Die Messung zeigt keinen klaren Gewinn.

- **Was danebenlaufen könnte.** Die Sichtbarkeit der Kaskaden (0,19 ms) braucht der Tiefen-Durchgang desselben Bildes,
  und der Hauptdurchgang braucht die Schattenkarte; neben ihr läge nur die Sichtbarkeit der Kamera (0,04 ms). Neben dem
  Zeichnen des laufenden Bildes liefe nur Arbeit des *nächsten* Bildes, mit doppelt gepufferten Listen oder
  Instanzdaten: `scatter` der Änderungen für das nächste Bild (bei 100 000 Änderungen je Bild 1,5 ms GPU) oder die
  Sichtbarkeit der Kaskaden des nächsten Bildes.
- **Gemessen** (`render3dtest --bench 1000000 --async-work N --async-on compute|main`; `compute` fordert die
  Rechen-Warteschlange an, `--compute-queue 1` auch ohne Arbeit daneben): neben jedem Bild läuft der
  echte `scatter`-Shader mit N Änderungen in eine zweite Kopie der Instanzdaten, auf der Rechen-Warteschlange oder
  nach dem Bild auf der des Geräts; gemessen wird der Bildabstand auf der GPU (Tabelle unter
  [Messwerte G5d](#g5d-verdeckung-detailstufen-zweite-warteschlange)). Ohne Schatten (Bild 0,57 ms) versteckt die
  Rechen-Warteschlange nichts: 100 000 Änderungen kosten nach dem Bild 1,57 ms mehr, daneben 1,62 ms (die GPU teilt
  sich zwischen beiden auf, der Bau der Pyramide dauert dann 1,6 statt 0,045 ms). Mit vier Kaskaden (Bild 3,83 ms)
  versteckt sie rund die Hälfte (5,43 gegen 4,70 ms, die Läufe streuen von 4,32 bis 5,01), mit 10 000 Änderungen
  0,04 von 0,13 ms. Das wäre der Gewinn, wenn render3d die Instanzdaten doppelt hielte (bei 1 Mio. Plätzen 84 MB
  mehr) und jede Änderung in beide Kopien schriebe; bei 10 000 Änderungen je Bild ist aber die CPU der Engpass
  (rund 5 ms je Bild), und große Änderungsmengen gehören in einen eigenen Rechen-Shader zum Bewegen (nicht gebaut).
  Kein klarer Gewinn: render3d bleibt bei einer Warteschlange.
- **Concurrent sharing kostet hier nichts Messbares**: dieselbe Szene mit Rechen-Warteschlange (Puffer concurrent) und mit
  `jfgui_GPU_COMPUTE=0` (exklusiv): 0,551 gegen 0,550 ms GPU je Bild. Gemessen nur auf der NVIDIA; weil andere GPUs
  (AMD) dafür Kompression aufgeben könnten und render3d die Warteschlange nicht nutzt, legt `lib/gpu` sie seit dem
  Review nur auf Wunsch an.

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

## sRGB (G5e2)

render3d rechnet in linearem Licht und schreibt am Ende des Fragment-Shaders die Farbe. Ein sRGB-Ziel (`Rgba8Srgb`, `Bgra8Srgb`) kodiert sie in der Hardware; ein 8-Bit-Ziel ohne sRGB nahm sie bis G5e1 roh, das Bild war zu dunkel. Solche Ziele sind häufig: die Swapchain eines Fensters (`Bgra8Unorm`, so `render3dtest --test window`) und die Ansicht der UI (`ui::GPU_VIEW_FORMAT`, ebenfalls `Bgra8Unorm`, damit der Framebuffer die Bytes ohne Umweg bekommt).

**Was gebaut ist** (Entscheidung des Nutzers vom 09.11.2026): Ist das Farbformat des Renderers ein 8-Bit-Unorm-Format ohne sRGB (`Rgba8Unorm`, `Bgra8Unorm`, `Rg8Unorm`, `R8Unorm`), kodiert der Fragment-Shader des Hauptdurchgangs nach sRGB, nach der Definition (IEC 61966-2-1: unter 0,0031308 mal 12,92, darüber 1,055 · v^(1/2,4) − 0,055, nach dem Klemmen auf 0 bis 1), nicht als Gamma 2,2. Gesteuert über eine Pipeline-Konstante (`@id(0) override ENCODE_SRGB` in `draw.wgsl`, gesetzt nur für `fs` und `fs_mask`): ein Shader, auf Vulkan dasselbe SPIR-V, im Browser dasselbe WGSL; Pipelines für andere Formate bekommen die Konstante nicht und sind die von vorher. Auch die Löschfarbe ist lineares Licht: `record` kodiert sie für solche Ziele auf der CPU (in `f64`), wie die Hardware es bei einem sRGB-Ziel tut. sRGB-Formate kodiert weiter die Hardware, Float-Formate (`Rgba16Float`, `Rgba32Float`) bleiben linear, 10-Bit-Formate (`Rgb10a2Unorm`) ebenfalls (die Regel nennt 8 Bit; offen).

**Abschalten:** `RendererDesc::encode_srgb = false` lässt ein 8-Bit-Ziel linear, wie vor G5e2. Dafür gibt es das Feld: Die Vergleiche mit dem Strahlenwerfer (alle Tests außer `srgb`, `window` und den Ansichten von `uigputest`) prüfen Licht in linearen Stufen auf ±1, und in sRGB-Stufen ist eine Stufe bei hellen Farben 2,3 lineare Stufen breit: Die Tests behalten ihre Schärfe, indem sie linear bleiben (`linear_desc` in `common.rs`); dazu kommen Programme, die ein 8-Bit-Zwischenbild selbst weiterverarbeiten. `RendererDesc` ist dafür `#[non_exhaustive]` geworden: `RendererDesc::new(format)`, dann Felder setzen (`render3dtest`: `frames_desc`, `linear_desc`). `RendererDesc::encodes_srgb()` sagt, ob die Shader kodieren; `Renderer::desc()` gibt die Beschreibung zurück.

**Tests** (`render3dtest --test srgb`, Voll und Basis, RTX 2070 SUPER): die zehn Bilder der Galerie (G5a bis G5d, mit Texturen in sRGB und linear, PBR, Schatten, Lichtern, Umgebung, Verdeckung)

| Vergleich | Ergebnis (je Stufe, 576 000 Pixel) |
| --- | --- |
| `Rgba8Unorm` (der Shader kodiert) gegen dasselbe Bild in `Rgba32Float`, auf der CPU nach der Definition kodiert | größte Abweichung 1; 14 877 Pixel um 1 (2,6 %), keins um 2 |
| `Bgra8Unorm` gegen `Rgba8Unorm` | 0 Pixel anders |
| `Rgba8Srgb` (die Hardware kodiert) gegen den Shader | größte Abweichung 1; 25 485 Pixel um 1 (4,4 %) |
| `encode_srgb = false` gegen das Float-Bild gerundet | größte Abweichung 1; 18 237 Pixel um 1 (die Hardware rundet beim Schreiben ins 8-Bit-Ziel nicht immer wie die CPU) |
| fünf Formen über einem Boden aus drei Richtungen (die Szene der Ansichten) gegen den Strahlenwerfer in sRGB-Stufen (`RefScene::srgb`) | 0 falsch, größte Abweichung 1 |
| die Löschfarbe (0,2; 0,4; 0,6) | (124, 170, 203), die sRGB-Stufen |

`window`: das dargestellte Bild (`Bgra8Unorm`) gegen dasselbe Bild offscreen 0 Pixel anders, gegen das Float-Bild auf der CPU kodiert größte Abweichung 1 (1921 Pixel um 1). Die Ansichten von `uigputest` vergleichen mit dem Strahlenwerfer in sRGB-Stufen (0 falsch, größte Abweichung 1).

**Was sich nicht ändert, vorher gegen nachher** (`render3dtest --dump DIR --format F [--linear]`, je Stufe die zehn Bilder, vor der Änderung geschrieben): `Rgba8Srgb`, `Rgba16Float`, `Rgba32Float` und `Rgba8Unorm`/`Bgra8Unorm` mit `encode_srgb = false`: 0 von je 1 152 000 Pixeln anders, Byte für Byte. Im Browser (`engine_web.mjs --save`, drei Bündel, Chrome 156): die Galerie im sRGB-Ziel (`srgb_NAME`) und die linearen Bilder 0 von 3 110 400 Pixeln anders. `Rgba8Unorm` mit Kodieren: jedes Pixel anders, wie es soll.

**Im Browser** (`engine_web.mjs`, Chrome 156 kopflos, drei Bündel): die Galerie mit dem Kodieren des Shaders (`enc_NAME`) gegen Vulkan in einem von 57 600 Pixeln um 1 (`gltf`, `pbr`, `occl`), sonst 0; gegen dasselbe Bild in `Rgba32Float` im Browser auf der CPU kodiert größte Abweichung 1; im sRGB-Ziel (`srgb_NAME`) gegen Vulkan 0 Pixel anders.

## In der Oberfläche (G5e1)

Eine Szene als Teil einer jfgui-Oberfläche, neben Widgets, unter Popups und Tooltips, in einem gewöhnlichen Fenster: `ui.gpu_view` ([GPU.md, 3D-Ansicht](GPU.md#3d-ansicht-g5e1); dort auch, wie die Ansicht ins Bild kommt, was sie kostet und was ohne GPU geschieht). `lib/ui` kennt render3d nicht und hängt nicht davon ab: Die Verbindung ist ein `ui::GpuContent` in der App, der `Renderer` und `Scene` hält, rund 40 Zeilen. So sieht sie aus (getestet als `View3d` in [`apps/uigputest/view.rs`](../apps/uigputest/view.rs)):

```rust
struct Ansicht { r: Option<render3d::Renderer>, s: Option<render3d::Scene>, kamera: render3d::Camera }

impl ui::GpuContent for Ansicht {
    // das erste Gerät, oder ein neues nach einem verlorenen: alles auf ihm neu bauen
    fn attach(&mut self, dev: &gpu::Device) -> gpu::Result<()> {
        let mut r = render3d::Renderer::new(dev, &render3d::RendererDesc::new(ui::GPU_VIEW_FORMAT))?;
        // ... Meshes, Texturen, Materialien
        let mut s = render3d::Scene::new(&r)?;
        // ... Instanzen, Sonne, Lichter; Schatten nach der Stufe des Geräts, das die Ansicht bekam (G5e3)
        s.set_shadows(Some(&render3d::Shadows::for_level(dev.level())))?;
        (self.r, self.s) = (Some(r), Some(s));
        Ok(())
    }
    // das Gerät geht: was darauf lag, fallen lassen
    fn detach(&mut self) {
        (self.s, self.r) = (None, None);
    }
    fn draw<'a>(&'a mut self, e: &mut gpu::CommandEncoder<'a>, target: gpu::ColorAttachment<'a>, (w, h): (u32, u32)) -> gpu::Result<()> {
        self.kamera.aspect = w as f64 / h as f64;
        if let (Some(r), Some(s)) = (self.r.as_mut(), self.s.as_mut()) {
            r.set_size(w, h)?;                     // der Tiefenpuffer folgt der Ansicht
            s.set_camera(&self.kamera);
            s.prepare(r)?;
        }
        let me: &'a Self = self;
        let (Some(r), Some(s)) = (&me.r, &me.s) else { return Err(gpu::GpuError::Invalid("nicht angebunden")) };
        r.record(e, s, gpu::ColorAttachment { clear: [0.1, 0.1, 0.12, 1.0], ..target }, None)
    }
    fn submitted(&mut self, id: gpu::SubmitId) {
        if let Some(s) = &mut self.s { s.submitted(id) }
    }
}

// im Bild der App: ein neues `ver`, wenn Szene oder Kamera sich ändern; die Eingabe wie bei ui.canvas
let inp = ui.gpu_view(ID, &self.ansicht, self.ver).clone();
```

- **Das Ziel** hat `ui::GPU_VIEW_FORMAT` (`Bgra8Unorm`), der Renderer entsteht mit diesem Format. Seit G5e2 kodiert render3d dafür nach sRGB ([sRGB](#srgb-g5e2)), die Ansicht zeigt also dieselben Farben wie ein sRGB-Ziel; bis G5e1 war sie zu dunkel.
- **Der Durchgang** beginnt in `target.before` und endet in `target.after` (lib/ui setzt beide: `Sampled`, beim ersten Bild `Undefined`). Mit Verdeckung teilt `record` ihn in zwei Durchgänge, Phase 1 mit `before`, Phase 2 mit `after`; render3d muss nichts weiter wissen. Lädt der Inhalt anders (`load`, `clear`), überschreibt er nur diese Felder.
- **Ein neues Gerät** (Treiberreset, `Ui::gpu_test_lose`, ein Wechsel vom eigenen Gerät zu dem der UI-Ausgabe): `detach`, dann `attach` auf dem neuen; der Renderer wird neu gebaut, das nächste Bild folgt von selbst. Bis dahin zeigt die Ansicht ihr letztes zurückgelesenes Bild. Geprüft im Test `viewlose`: 2 Anbindungen, dasselbe Bild auf dem neuen Gerät (0 Pixel anders als offscreen).
- **Stufe**: Mit `AF_GPU_3D` legt die UI ihr Gerät auf Stufe Voll an (ein Multi-Draw je Seite und Klasse); ohne den Wunsch läuft dieselbe Ansicht auf dem Basis-Gerät der UI (ein indirekter Aufruf je Liste), geprüft im Test `viewbasis`. Welche Stufe ein Inhalt bekam, sagt `dev.level()` in `attach`; Schatten danach wählt `Shadows::for_level` (G5e3: auf Basis 2 Kaskaden zu 1024, [Entscheidung](#g5e2-schatten-auf-basis-schwache-gpus)).
- **Eine Szene je Ansicht.** Zwei Ansichten mit zwei Inhalten teilen das Gerät (Test `views2`); zwei Ansichten mit *einem* Inhalt hieße `attach` zweimal auf demselben Inhalt: nicht vorgesehen.

## Tests

`python -u build_rs.py test render3dtest --target win` führt `apps/render3dtest/tests/*.uis` aus, alles offscreen außer `window.uis`, die meisten auf Voll und auf einem auf Basis begrenzten Gerät. Ohne GPU `SKIP`. Die Referenz ist unabhängig von der Engine: ein Strahlenwerfer in `f64` (eigene Wurzel, eigene Drehung über Quaternionen-Produkte, die Ebenen als Abstände im Kameraraum). Seit G5c schattiert er mit derselben BRDF, aus der Spezifikation neu geschrieben (Neutral-Tonemapper, Tangentenrahmen des Dreiecks aus seinen Kanten und UV); die Tabelle der Split-Sum ist für ihn Daten wie die Texel einer Textur, ihr Inhalt wird gegen sein eigenes Integral geprüft. Alle Bilder der G5a- und G5b-Tests haben ihre neue Referenz aus ihm, keine aus der GPU. An Kanten darf ein einfarbiges Objekt jede Farbe zwischen denen zeigen, die sein Dreieck mit den Normalen seiner Ecken und am Treffer hätte (vorher: jede Helligkeit zwischen Grundlicht und voller Sonne; mit Glanz und Fresnel ist die Farbe kein Vielfaches der Grundfarbe mehr). **Jenseits eines Rands:** Die GPU rastet Ecken auf 1/256 Pixel ein, deckt also auch ein Pixel, dessen Mitte ein Haar außerhalb des Dreiecks liegt, und liest die Attribute an genau dieser Stelle. Am Rand eines Würfels ist u dann etwa −0,00002, und `Repeat` liest die andere Seite der Textur, bei MASK womöglich ein deckendes Texel statt eines Lochs. Das folgt aus den Regeln von Vulkan und ist kein Fehler der Engine. Der Vergleich rechnet diese Stelle nach (`RefScene::beyond_rim`: die Mitte höchstens 1/128 Pixel außerhalb einer Vorderseite eines texturierten Objekts, nichts davor, die Farbe dort ±Toleranz), zählt solche Pixel und lässt nur wenige zu. Gefunden auf llvmpipe (Mesa 25.2.8): 14 Prüfungen in `alpha`, `mirror` und `textures` scheiterten an 1 bis 2 solchen Pixeln, jedes 0,0001 bis 0,0005 Pixel außerhalb und in der Farbe dieser Stelle (±1); auf dem Stand von G5e1 sind es dieselben Pixel, die Tests liefen dort nur nie unter Linux. Auf der RTX 2070 SUPER kommt in diesen drei Tests keins vor.

| Skript | Prüft | Ergebnis (RTX 2070 SUPER, Treiber 610.88) |
| --- | --- | --- |
| `pixels` | gedrehter Würfel, zwei Würfel in beiden Reihenfolgen, Kugel 2,5 × 1 × 0,5 mit glatten Normalen, Rückseiten von innen, ferne Ebene, Platten 1 auseinander in 100 000 Abstand mit near 0,01; Farbe je Pixel ±1 und die Werte des Tiefenpuffers (über einen Rechen-Shader gelesen) | 0 falsche Pixel; Tiefe: größter relativer Fehler 4,5 · 10⁻⁴, in 100 000 Abstand 6 · 10⁻⁸ |
| `visibility` | Zufallsszenen mit festem Seed (5000, 20 000, 60 000 Instanzen, 3 Meshes) gegen den exakten Test: nichts fehlt, nichts zu viel, nichts doppelt, keine falsche Liste; Kugeln genau an jeder der sechs Flächen (berührend: in der Liste, 0,01 dahinter: nicht); Bild mit Sichtbarkeit gegen Bild mit allem | 975, 2322 und 8820 sichtbar, Abweichung 0; 12 von 12 Kugeln richtig; 0 Pixel anders |
| `changes` | Hinzufügen, Verschieben, Entfernen, Plätze wieder vergeben, Material und Mesh ändern, eine Tabellenzeile; das Bild gegen eine neu aufgebaute Szene (in umgekehrter Reihenfolge); was hochgeht, in Byte; beide Wege des Hochladens; ein `prepare` ohne Bild; seit den Listen je Alpha-Modus die Plätze der Listen nach dem Bild, das sie festlegt (vorher wurde vor `prepare` verglichen, das konnte nicht fehlschlagen), und ein Fall innerhalb der Kapazität | 0 Pixel anders; ein Bild ohne Änderung schickt 224 Byte, 200 Änderungen 19 424 Byte; 15 von 48 Listen wuchsen (mehr Instanzen als Platz), 0 ohne Grund; innerhalb der Kapazität bleiben alle Plätze |
| `many` | 160 Meshes und 400 Materialien gegen den Strahlenwerfer; wachsende Pools; 16- und 32-Bit-Listen im Wechsel, ein Mesh mit ungerader Indexzahl | 0 falsche Pixel, 0 anders |
| `levels` | Voll gegen Basis, fünf Bilder der Galerie: drei aus G5a (3000 Instanzen, 48 Meshes, 96 Materialien), die texturierte (7 Texturen in 4 Klassen, jeder Abtastmodus, MASK, 600 Instanzen, ein Drittel gespiegelt) und die glTF-Szene | 0 von 57 600 Pixeln anders, in jedem Bild |
| `coords` | Ursprung und 1e7 mit gleicher relativer Kamera; dasselbe mit Positionen, die `f64` bei 1e7 runden muss; Ursprung nachträglich setzen; als Gegenprobe ohne Ursprung | 0 Pixel anders; ohne Ursprung 30 365 von 57 600 anders (der Test kann es also erkennen) |
| `frames` | sieben Bilder in Flug mit eigener Kamera und eigener Änderung gegen jedes einzeln gezeichnet; der Tiefenpuffer des Renderers | 0 Pixel anders |
| `errors`, `shaders` | was die API verweist; leere Szene, auch mit Ursprung und nach einem `prepare` ohne Bild; Instanzen nach `prepare`; ein Bild, das nie abgeschickt wurde; eine Szene mit fremdem Renderer; verlorenes Gerät; die eingebetteten Wörter | bestanden |
| `window` | ein echtes Fenster, sechs Bilder, das dargestellte Bild gegen dasselbe Bild offscreen, mit Validierung; ein einziger, kurzer Lauf; seit G5e2 dazu gegen das Float-Bild, auf der CPU nach sRGB kodiert | 0 Pixel anders; gegen das Float-Bild größte Abweichung 1 |
| `srgb` (G5e2) | [sRGB](#srgb-g5e2): die Galerie mit dem Kodieren des Shaders gegen `Rgba32Float` auf der CPU kodiert, gegen ein sRGB-Ziel, `Bgra8Unorm` gegen `Rgba8Unorm`, abgeschaltet gegen das Float-Bild gerundet; fünf Formen gegen den Strahlenwerfer in sRGB-Stufen; welche Formate kodieren | größte Abweichung 1, `Bgra8Unorm` 0 Pixel anders, 0 falsch |
| (G5e1) `uigputest --test view*` | eine Szene in einer jfgui-Oberfläche: das Bild im Framebuffer gegen dasselbe Bild offscreen auf demselben Gerät und gegen den Strahlenwerfer, jedes dargestellte Bild gegen den Framebuffer ([GPU.md](GPU.md#3d-ansicht-g5e1)) | 0 Pixel anders, gegen den Strahlenwerfer 0 falsch |
| `bench_shadows` (G5c) | ein kurzer Messlauf mit vier Kaskaden unter Validierung | 0 Fehler; die Listen der Kamera wie `cull_ref` |
| `validate`, `bench_smoke`, `bench_textures` | alles außer `window` unter der Validierungsschicht mit Synchronisationsprüfung, auch der Shader-Zugriffe; kurze Messläufe mit Änderungen und mit 16 Texturen: die Listen enthalten alles aus `cull_ref` der Messszene | 0 Fehler, 0 Warnungen; 0 fehlen |
| `textures` (G5b) | die Wrap-Modi vergrößert, nächstgelegen und bilinear; die Stufen 1 und 2 (ρ 2,6 und 5,2) nächstgelegen und zwischen zwei Stufen gemischt; die 8 Stufen einer 128er-Textur (sRGB und linear) aus dem Array zurückgelesen gegen den eigenen Kastenfilter; ein Boden unter flachem Winkel trilinear über den Sampler und im Shader (Clamp × Repeat, Mirror mit nächster Stufe); jede Klasse vergrößert; 40 Würfel einer Liste mit 40 Texturen (sRGB und linear); 90 Texturen mehr (die Arrays wachsen von 3 auf 15 MiB); 100 × 60 bilinear und 48 × 20 nächstgelegen umgerechnet gegen eigenes Umrechnen; 4096 × 64 in Klasse 2048; MASK mit Löchern und mit einem Faktor-Alpha unter der Schwelle; was verweigert wird | 0 falsche Pixel; größte Abweichung genau gefordert ±1: gemessen 1; bilinear (±2): 1; trilinear über den Sampler (±6): 5, im Shader: 1; Stufen: 0 Byte anders |
| `mirror` (G5b) | gespiegelte Würfel und Prismen (ungerade und gerade Zahl negativer Skalierungen) neben ungespiegelten, texturiert, damit die Spiegelung der UV zu sehen ist; die Listen je Mesh und Seite; jede Instanz mit `set_transform` auf die andere Seite gegen eine neu gebaute Szene; von innen aus einem gespiegelten Würfel; 20 000 Zufallsinstanzen, etwa die Hälfte gespiegelt, gegen den exakten Test | 0 falsche Pixel; Listen (Mesh, gespiegelt, Anzahl) wie erwartet; 0 Pixel anders; nur die Löschfarbe; 2458 sichtbar, 0 fehlen, 0 zu viel |
| `gltf` (G5b) | [GLTF.md](GLTF.md#tests): drei Formen derselben Szene, was gelesen wird, kaputte Dateien, die Szene gezeichnet gegen den Strahlenwerfer (aus der Beschreibung, nicht aus dem Import; seit G5c mit allen fünf Texturen, sRGB und linear) | bestanden; gezeichnet 0 falsche Pixel |
| `lights` (G5c) | Zwei Punktlichter (eins mit Reichweite 4), zwei Spotlichter (schmal mit innerem Kegel, weit ohne), eine schwache Sonne über Boden, Kugeln (eine metallisch) und Würfeln; dasselbe Licht in `Rgba16Float`; nichts jenseits der Reichweite; 64 Lichter auf der laufenden Szene und gegen eine neu gebaute; was hochgeht; dasselbe bei 1e7; was `set_lights` verweigert. glTF: eine Datei mit fünf Lichtern aller Arten (eins zweimal von Knoten gesetzt, eins unter einem Elternknoten), was gelesen wird, wohin die Knoten sie setzen, `extensionsRequired`, jede Länge abgeschnitten (2337), 600 verfälschte, 17 gebrochene Regeln, die Grenze, die Datei gezeichnet gegen den Strahlenwerfer aus der Beschreibung | 0 falsche Pixel, größte Abweichung 1 (64 Lichter: auch); `Rgba16Float` 0 falsch, größte relative Abweichung 6,2 · 10⁻³ an einem Glanzpunkt am Rand einer Kugel (der Vergleich erlaubt dort ein Viertel dessen, was 1/64 Pixel ändert: die GPU rastet Ecken auf 1/256 Pixel; zuerst schlug er an diesem einen Pixel fehl); 3842 Pixel innerhalb der Reichweite, 0 außerhalb; 0 Pixel anders als neu gebaut und als bei 0; 1704 Byte mit den Lichtern, 496 im Bild danach; 7 von 7 verweigert. glTF: wie geschrieben, Weltmatrizen 4,4 · 10⁻¹⁶ neben den hier zusammengesetzten; 0 der abgeschnittenen laden; von den verfälschten laden 12, 588 Fehler, keine Panik; alle 17 `Invalid`; Grenze 4: `Limit`; gezeichnet 0 falsche Pixel |
| `env` (G5c) | Auf der CPU: die 9 Koeffizienten des Himmels gegen eine eigene Projektion (64 × 64 je Seite), die Bestrahlungsstärke der Koeffizienten gegen ihr Integral (24 Normalen, 16 384 Punkte), die Stufen der Würfel-Textur gegen eine GGX-Faltung mit 8192 Punkten. Auf der GPU: 8 Kugeln (Rauheit 0 bis 1, dielektrisch und metallisch) nur im Himmel und mit Sonne gegen den Strahlenwerfer, der die Stufen wie Texel liest und abtastet, wie Vulkan Würfel definiert (Wahl der Seite, bilinear über Kanten hinweg, an Ecken das Mittel der drei Texel, linear zwischen Stufen); die Umgebung wieder aus; Weiß-Ofen aus sechs weißen Seiten in `Rgba16Float`; ein Würfel aus Seiten des Aufrufers (16 × 16, Farbe nach Richtung); was `set_environment` verweigert | 6,2 · 10⁻⁴; 1,7 %; 2,4 %; 0 falsche Pixel, größte Abweichung 1 (mit Sonne auch); aus: 0 Pixel anders; Weiß-Ofen 1,0000 (Metall 0,3069 bis 1,0000), 0 falsch; Seiten: größte Abweichung 1; 3 von 3 verweigert |
| `bench_lights` (G5c) | ein kurzer Messlauf mit 64 Punktlichtern unter Validierung | 0 Fehler; die Listen wie `cull_ref` |
| `shadows` (G5c) | Eine Szene über alle vier Kaskaden (Boden, Würfel bis 58 entfernt, Kugel, Mast, gespiegelter Würfel, eine MASK-Platte mit quadratischen Löchern zur Sonne), 400 × 225, Schattenweite 90, gegen exakte Schattenstrahlen: Jedes Pixel abseits von Kanten muss eine Mischung aus seiner beschatteten und seiner besonnten Farbe zeigen (±1), und die Mischung wird gegen den Strahl beurteilt (mehr als halb auf der anderen Seite: „andere Seite“; innerhalb von 3 Pixeln eines anders entschiedenen Strahls: an einer Schattenkante, sonst „innen“, also Akne oder Lecks); als Gegenprobe dasselbe Bild gegen die Szene ohne Schatten; MASK ohne `fs_shadow`; eine Kaskade zum Vergleich; dasselbe bei 1e7 vom Ursprung; die Radien und das Raster der Kaskaden auf der CPU (40 Kameras); die Karte zurückgelesen nach einer Bewegung um 0,2 und 3,2 Texel | 0 falsch, 0 innen; an Schattenkanten 230 von 53 531 Pixeln auf der anderen Seite (0,43 %: Filter und Normalen-Versatz), bei 1e7 127 (0,24 %); ohne Schatten 2494 falsch; ohne `fs_shadow` 498 statt 230 auf der anderen Seite; eine Kaskade: 601 an Kanten (1,12 %), 11 innen (0,02 %, gröbere Texel); Radien gleich, Mitten höchstens 7,2 · 10⁻⁷ Texel neben dem Raster; Karte 0 von 4 194 304 Texeln anders, um 3 Texel verschoben 0 anders bedeckt |
| `pbr` (G5c) | 8 Kugeln dielektrisch und metallisch, Rauheit 0,25 bis 1, mit Neutral und mit Belichtung 0,6 ohne Tonemapping; dasselbe Licht in einem `Rgba16Float`-Ziel ohne Tonemapping (±2 · 10⁻³ + 2 · 10⁻³ relativ: Halbfloats haben 11 Bit); Weiß-Ofen (weiße Umgebung 1, keine Sonne, Rauheit 0 bis 1); Metallic-Roughness-Textur; Normalenkarten auf Platten (wie sie sind, um 90° gedreht mit Skala 0,8, gespiegelt, mit gespiegeltem u; nächstgelegen) und auf Kugeln (gedreht, gespiegelt mit Skala 1,5; bilinear auf Stufe 0); in welche Richtung eine Normalenkarte kippt (+y im Bild unter einer Sonne von oben: heller, −y dunkler); Emission im Dunkeln; Verdeckung mit Stärke 0,7, und ohne Grundlicht ändert sie nichts. Auf der CPU: die Albedo der BRDF durch Wichtigkeitsabtastung, die Tabelle gegen das eigene Integral, Karis' Näherung gegen dasselbe | 0 falsche Pixel, größte Abweichung 1 (bilinear ±2: 1); `Rgba16Float`: größte relative Abweichung 3,1 · 10⁻³, 0 falsch; Weiß-Ofen: Dielektrikum 1,0000 bis 1,0000, Metall 0,307 bis 1,000; die Normalenkarten ändern 7388 Pixel; Helligkeit 546 > flach 355 > 176; Verdeckung ohne Grundlicht 0 Pixel anders. CPU: weißes Metall höchstens 1,0000 (Einfachstreuung verliert bis 0,307 bei Rauheit 1); weißes Dielektrikum bei n·v 1 und 0,7: 1,000 und 1,001, bei 0,3: 1,151, bei 0,05: 1,719 (die BRDF von Anhang B ist streifend nicht energieerhaltend: `1 − F(v·h)` lässt fast die ganze Streuung stehen, während die Spiegelung wächst); Tabelle 0,0039 neben dem Integral; Karis 0,193 |
| `occlusion` (G5d) | Wände mit 2400 kleinen Instanzen dazwischen und dahinter (ein Drittel gespiegelt), zwei Szenen eines Renderers, eine mit, eine ohne Verdeckung, zwölf Bilder einer Fahrt (Bild 4: 150 Instanzen dazu, 6: ein Sprung, 7: 300 weg, 8: ein neuer Ursprung, ab 9: ein kleineres Ziel), am Ursprung und bei 1e7; in drei Bildern gegen den Strahlenwerfer und gegen das, was seine Strahlen durch die Pixelmitten sehen; die Pyramide eines ersten Bildes (301 × 167, ungerade) gegen die Minima des Tiefenpuffers auf der CPU; drei Bilder über den eigenen Tiefenpuffer (`set_size`, auch eine neue Größe) gegen ohne Verdeckung; Voll und Basis | jedes Bild 0 Pixel anders als ohne; gegen den Strahlenwerfer 0 falsche Pixel; gezeichnet 615 / 1031 / 702 von 1890 / 1646 / 1290 im Kegel (Phase 2: 216 / 707 / 21), die Strahlen sehen 194 / 75 / 68, davon 0 nicht gezeichnet; Phase 2 zeichnete über die Fahrt 1356; Pyramide 9 Stufen, 16 967 Texel, 0 anders; eigener Tiefenpuffer 0 Pixel anders; Validierung 0 Fehler |
| `alpha` | Würfel und Platten mit Löchern (Textur, MASK), gespiegelt und nicht, vor einer Platte: eine Instanz wechselt mit `Scene::set_material` zwischen deckend und MASK (und zwei weitere, eine gespiegelt), zurück, ein Material wechselt seine eigene Klasse (`Renderer::set_material`), die MASK-Instanzen wechseln die Seite, ein Wechsel nach `prepare`; jedes Bild gegen dieselbe Szene neu gebaut und gegen den Strahlenwerfer, die Listen (Mesh, Seite, Klasse, Anzahl), die Zeichenbefehle nach Indexbreite, Klasse und Seite. Detailstufen: 1500 Kugeln (ein Drittel MASK mit Fenstern, ein Drittel gespiegelt), die Wahl (Mesh, Seite, Klasse) je Instanz gegen die CPU, die Listen der Stufen, das Bild mit Verdeckung gegen den Strahlenwerfer. Schatten: die MASK-Platte der Schattenszene deckend und zurück gegen exakte Schattenstrahlen, nur deckende Werfer mit und ohne `set_shadow_mask`. Verdeckung hinter einer MASK-Wand (oben). Voll und Basis | jedes Bild 0 Pixel anders als neu gebaut, gegen den Strahlenwerfer 0 falsch, größte Abweichung 1; Listen wie erwartet, Befehle geordnet; nach `prepare`: das Bild zeigt den Stand von `prepare` (0 Pixel anders), das nächste den Wechsel. Detailstufen: 1500 richtig, 0 falsch (MASK im Kegel je Stufe 21 / 95 / 248), die vier Ausgangslisten haben ihre Stufen gleicher Seite und Klasse; Bild 0 falsch (Grenze 2: an der Kante eines Fensters auf einer Kugel von wenigen Pixeln ändert sich die Farbe innerhalb von 1/64 Pixel um 2; mit 1 schlug ein Pixel an einer Kante fehl, ganze Pixel bleiben bei 1). Schatten: mit Löchern 230 an Kanten auf der anderen Seite, deckend 89, 0 falsch, 0 innen; deckend gegen neu gebaut 0, gegen die Platte mit Löchern 701 Pixel anders; nur deckende Werfer mit und ohne `fs_shadow` 0 Pixel anders; MASK wieder 0 anders als zuerst |
| `lods` (G5d) | Kugeln mit 16 × 24 Segmenten nehmen unter 0,08 und 0,03 der Bildhöhe Kugeln mit 8 × 12 und 4 × 6 Segmenten und fallen unter 0,008 weg, 1500 Instanzen in 3 bis 160 Abstand (ein Drittel gespiegelt, ein Achtel Würfel ohne Stufen; keine Größe innerhalb von 2 % einer Schwelle); die Wahl je Instanz ohne Verdeckung gegen die CPU (Liste, Mesh und Seite); das zweite Bild mit Verdeckung gegen den Strahlenwerfer, der jede Instanz mit dem Mesh zeichnet, das die CPU wählt; ohne Stufen und wieder mit; die Listen der vier Kaskaden gegen die Wahl der Kamera; eine Stufe 2 neben ihrem Mesh vergrößert die Kugel der Sichtbarkeit; was `set_lods` verweigert; Voll und Basis | 1500 richtig, 0 falsch (im Kegel 81 / 298 / 779 Kugeln je Stufe, 158 Würfel, 184 ausgelassen); 0 falsche Pixel; ohne Stufen 3127 Pixel anders, wieder mit 0; Kaskaden 2397 Einträge, 0 mit anderer Wahl; Kugel 3,0000 statt 1,7321; 6 von 6 verweigert |

**Umgebung im Browser**: Das Bild `env` (8 Kugeln und ein Boden im Himmel, mit Sonne) ist in allen drei Bündeln Pixel für Pixel das von Vulkan.

**Lichter im Browser**: Das Bild `lights` (vier Lichter, Schatten der Sonne in zwei Kaskaden) weicht in einem von 57 600 Pixeln um 1 ab, in allen drei Bündeln, gegen Voll wie gegen Basis (die Grenze des Tests: 2 Stufen, 1 % der Pixel). Vermutlich rundet der Übersetzer des Browsers eine Zeile anders (etwa ein zusammengezogenes Multiplizieren und Addieren im Kegel des Spotlichts, dessen Skala 12,9 kleine Unterschiede vergrößert); nicht weiter verfolgt.

**Verdeckung und Detailstufen im Browser** (G5d): Das Bild `occl` (die Szene des Tests `occlusion` mit Detailstufen, nach sechs Bildern einer Fahrt am Sprung, Phase 2 zeichnet viel) weicht in allen drei Bündeln in einem von 57 600 Pixeln um 1 ab, gegen Voll wie gegen Basis (innerhalb der Grenze des Tests); alle anderen Bilder wie vorher (0 Pixel, `lights` 1 Pixel um 1). Voll gegen Basis: 0 Pixel, auch `occl`. Zuerst schlug das Bild im Browser fehl: Die sechs Bilder davor gingen in dasselbe Ziel mit Rücklese-Puffer, den der Browser nach dem ersten Bild abbildet („used in submit while mapped“); die Bilder davor gehen jetzt in ein Ziel ohne Rücklese-Puffer.

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
- **Das meiste davon ist das `discard`**: eine Wegwerf-Änderung ohne die drei Zeilen, im Wechsel mit dem fertigen Stand gemessen (eine Serie von je drei Läufen, mehr GPU-Last nebenher als oben, daher höhere Werte): 1,265 ms (1,258–1,267) ohne gegen 1,285 ms (1,282–1,414) mit. Eine Pipeline ohne `discard` für deckende Listen holt das zurück; sie bräuchte Listen je Mesh, Seite und Alpha-Modus, und die Szene müsste den Modus der Materialien kennen (seit den [Listen je Alpha-Modus](#listen-je-alpha-modus) gebaut, Messung dort).
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

### Messung der Listen je Alpha-Modus

Dieselbe Maschine und Messszene (Release, offscreen, 1080p, 1 Mio. Würfel, 300 Bilder nach 60, 3 in Flug). Vorher ist
origin/main (55732b32) mit dem Schalter `--bench --mask P` (ed6cbf1d), nachher der fertige Stand; beide als Release
gebaut und je Variante im Wechsel gemessen (vorher, nachher, dreimal), in mehreren Runden, gezeigt der Median der
Mediane über alle Läufe und in Klammern ihre Spanne. `--mask 10`: 103 der 1024 Materialien sind MASK (Grenzwert 0,5,
eine 128er-Textur mit quadratischen Löchern auf einem Viertel der Fläche, trilinear), also rund 10 % der Würfel, in
jeder Gruppe von 32 Plätzen gemischt. GPU-Last anderer Prozesse (`nvidia-smi`) vor den Sätzen 7 bis 23 % (Firefox,
Discord, Word und andere Programme mit GPU-Zugriff, kein `rustc`); die ersten beiden Runden des gemischten Satzes ohne
Verdeckung lagen in einer unruhigen Zeit (Spannen bis 0,4 ms, höhere 99. Perzentile) und sind durch eine vierte Runde
ersetzt. „Zeichnen“ sind beide Durchgänge der Kamera.

| 1 Mio., ms | Zeichnen vorher | Zeichnen nachher | | Sichtbarkeit (Kamera) vorher / nachher | GPU gesamt vorher | GPU gesamt nachher |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| nur deckend, mit Verdeckung (n = 6) | 0,381 (0,378–0,428) | 0,325 (0,323–0,327) | −15 % | 0,039 / 0,039 | 0,551 (0,547–0,624) | 0,494 (0,488–0,495) |
| nur deckend, ohne Verdeckung (n = 6) | 1,560 (1,547–1,578) | 1,425 (1,413–1,444) | −9 % | 0,064 / 0,064 | 1,631 (1,618–1,665) | 1,496 (1,484–1,516) |
| 10 % MASK, mit Verdeckung (n = 6) | 0,461 (0,458–0,480) | 0,353 (0,351–0,354) | −23 % | 0,040 / 0,040 | 0,635 (0,631–0,666) | 0,523 (0,522–0,524) |
| 10 % MASK, ohne Verdeckung (n = 6) | 1,645 (1,626–1,659) | 1,427 (1,424–1,438) | −13 % | **0,064 / 0,145** | 1,716 (1,697–1,730) | 1,581 (1,575–1,592) |
| nur deckend, 4 Kaskaden, mit Verdeckung (n = 9) | 0,520 (0,516–0,529) | 0,434 (0,431–0,453) | −17 % | 0,039 / 0,039 | 4,109 (3,982–4,600) | 3,636 (3,515–4,352) |
| 10 % MASK, 4 Kaskaden, mit Verdeckung (n = 9) | 0,585 (0,582–0,606) | 0,471 (0,466–0,475) | −19 % | 0,040 / 0,040 | 4,208 (4,102–4,350) | 3,831 (3,669–3,875) |

Die Kaskaden (Tiefe je Kaskade, Median über alle Läufe; die ferne mit Spanne, die drei nahen als Summe mit Spanne; dazu der Median der Summe je Bild):

| 4 Kaskaden, ms | Kaskade 0 / 1 / 2 | nahe zusammen | ferne Kaskade | Tiefe je Bild | Sichtbarkeit der Kaskaden |
| --- | ---: | ---: | ---: | ---: | ---: |
| nur deckend, `mask_in_shadow` an, vorher (n = 9) | 0,175 / 0,246 / 0,504 | 0,925 (0,920–0,927) | 1,833 (1,829–2,069) | 3,085 (2,965–3,482) | 0,187 |
| nur deckend, `mask_in_shadow` an, nachher | 0,079 / 0,184 / 0,471 | 0,734 (0,729–0,736) | 1,765 (1,751–2,310) | 2,714 (2,494–3,327) | 0,187 |
| nur deckend, nur Tiefe (`set_shadow_mask(false)`), vorher | 0,079 / 0,184 / 0,472 | 0,735 (0,731–0,738) | 1,765 (1,754–2,135) | 2,521 (2,501–3,006) | 0,187 |
| nur deckend, nur Tiefe, nachher | 0,079 / 0,184 / 0,472 | 0,735 (0,730–0,736) | 1,766 (1,761–1,984) | 2,518 (2,507–2,875) | 0,186 |
| 10 % MASK, `mask_in_shadow` an, vorher | 0,181 / 0,258 / 0,517 | 0,956 (0,951–0,959) | 1,880 (1,862–2,100) | 3,079 (3,040–3,262) | 0,187 |
| 10 % MASK, `mask_in_shadow` an, nachher | 0,090 / 0,196 / 0,477 | 0,763 (0,759–0,764) | 1,772 (1,769–1,997) | 2,762 (2,545–2,816) | **0,236** |

- **Der Hauptdurchgang ohne `discard`: 9 bis 23 % weniger Zeichenzeit**, mehr als die 1,6 %, die G5b schätzte. Damals war der Fragment-Shader Lambert und billig; mit PBR kostet jedes Fragment viel, und ohne `discard` verwirft der frühe Tiefentest verdeckte Fragmente vor dem Shader. Am meisten bringt es, wo die Pixel zählen (mit Verdeckung zeichnet die Kamera 30 000 Würfel, der Bildschirm ist voll). Dass die gemischte Szene mehr gewinnt als die deckende (−23 gegen −15 %), liegt daran, dass vorher alle Würfel durch den Shader mit `discard` liefen, nachher nur die 10 % MASK.
- **Die 0,31 ms von `fs_shadow` sind weg**: Nachher zeichnen deckende Werfer mit `mask_in_shadow` genau so schnell wie der Weg nur mit Tiefe (nahe Kaskaden 0,734 gegen 0,735 ms, die ferne 1,765 gegen 1,766). In diesen Läufen kostete `fs_shadow` vorher 0,26 ms nach den Medianen der Kaskaden (0,19 nahe, 0,07 ferne), nach dem Median der Summe je Bild 0,37; G5c maß 0,31. Die ferne Kaskade springt zwischen Läufen und Bildern zwischen etwa 1,76 und 2,0 bis 2,3 ms, bei beiden Programmen und auch auf dem Weg nur mit Tiefe; die Summe je Bild streut deshalb mehr als die nahen Kaskaden.
- **MASK im Schatten**: Mit 10 % MASK kosten die Tiefen-Durchgänge nachher 0,763 ms in den nahen Kaskaden statt 0,956, nur 0,03 mehr als ohne MASK.
- **Was es kostet: die Sichtbarkeit, wenn die Klassen in den Gruppen gemischt sind.** Zwei Listen, deren Zähler dieselbe Arbeitsgruppe mit `atomicAdd` erhöht, sind auf dieser GPU teuer: ohne Verdeckung 0,145 statt 0,064 ms für die Kamera, in der fernen Kaskade 0,107 statt 0,057 ms. Das ist nicht der Umweg über `record`: Zwei deckende Listen im Wechsel (`--meshes 2`) kosten vorher wie nachher 0,316 ms. Gegenprobe aus G5a: 1024 Listen kosteten 0,079 ms; teuer sind wenige Zähler, die viele Gruppen zugleich treffen, vermutlich weil die Hardware gleiche Adressen in einem Warp nur zusammenfasst, wenn alle Aufrufe sie haben. Mit Verdeckung sieht Phase 1 nur 30 000 Instanzen, dort ändert sich nichts. Unter dem Strich bleibt die Szene ohne Verdeckung schneller (GPU gesamt 1,716 → 1,581 ms), die mit Schatten auch (4,208 → 3,831). Ein Hebel, nicht gebaut: Einträge je Arbeitsgruppe vorsammeln (Zähler in Workgroup-Speicher, ein `atomicAdd` je Liste und Gruppe). In echten Szenen liegen Instanzen eines Objekts oft nebeneinander; die Messszene mischt die Klassen in jeder Gruppe von 32 Plätzen.

### G5d: Verdeckung, Detailstufen, zweite Warteschlange

Dieselbe Maschine und Messszene (Release, offscreen, 1080p, 300 Bilder nach 60, 3 in Flug), je Variante drei Läufe
im Wechsel (`render3dtest --bench 1000000`), Median der Mediane, in Klammern die Spanne. GPU-Last anderer Sitzungen
vor dem ersten Satz 1 %; die 29 bis 48 % nach den Sätzen zeigte `nvidia-smi` direkt nach dem eigenen Lauf. „Zeichnen“
sind beide Durchgänge der Kamera; Pyramide / Phase 2 sind Bau der Pyramide, Sichtbarkeit und Durchgang der Phase 2
(in „Zeichnen“ steckt der Durchgang der Phase 2 mit). Alle Zeilen außer der dritten liefen mit Rechen-Warteschlange,
damals die Vorgabe (heute `--compute-queue 1`); die dritte zeigt, dass sie hier nichts kostet.

| 1 Mio., ms | gezeichnet | Sichtbarkeit (Phase 1) | Pyramide / Sichtbarkeit / Durchgang Phase 2 | Zeichnen | GPU gesamt | Bildabstand |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Würfel ohne Verdeckung | 528 700 | 0,064 | – | 1,566 (1,555–1,576) | 1,637 | 1,653 |
| Würfel mit Verdeckung | 30 200 (Phase 2: 11 900) | 0,038 | 0,045 / 0,077 / 0,036 | 0,382 (0,382–0,382) | 0,551 (0,550–0,552) | 0,567 |
| dasselbe, eine Warteschlange (`jfgui_GPU_COMPUTE=0`; seit dem Review die Vorgabe) | 30 200 | 0,038 | 0,045 / 0,077 / 0,037 | 0,381 | 0,550 (0,549–0,550) | 0,565 |
| Kugeln 8 × 16 (256 Dreiecke) ohne Verdeckung | 527 200 | 0,064 | – | 16,885 (16,755–16,889) | 16,956 | 16,972 |
| Kugeln mit Verdeckung | 78 800 | 0,045 | 0,057 / 0,080 / 0,897 | 2,530 (2,515–2,535) | 2,732 | 2,748 |
| Kugeln mit Verdeckung und Detailstufen (4 × 8 unter 0,02, 2 × 4 unter 0,008 der Bildhöhe) | 88 200 | 0,051 | 0,054 / 0,083 / 0,104 | 0,669 (0,665–0,669) | 0,867 (0,860–0,867) | 0,882 |
| dasselbe, unter 1 Pixel nichts | 88 200 | 0,052 | 0,053 / 0,083 / 0,103 | 0,665 | 0,863 | 0,879 |
| 4 Kaskaden ohne Verdeckung | 528 700 | 0,064 | – | 1,802 (1,796–1,803) | 5,036 (5,023–5,038) | 5,053 |
| 4 Kaskaden mit Verdeckung | 30 200 | 0,039 | 0,046 / 0,077 / 0,037 | 0,516 (0,515–0,518) | 3,806 (3,800–3,817) | 3,829 |

Kaskaden: Sichtbarkeit 0,187 ms, Tiefen-Durchgänge 2,826 (ohne) und 2,772 ms (mit Verdeckung der Kamera).

Arbeit neben dem Bild (`--async-work N`: der `scatter`-Shader mit N Änderungen in eine zweite Kopie der Instanzdaten):

| 1 Mio. Würfel mit Verdeckung, ms | GPU-Bildabstand, Arbeit nach dem Bild auf der Warteschlange des Geräts | auf der Rechen-Warteschlange |
| --- | ---: | ---: |
| ohne Arbeit daneben | 0,567 | – |
| 10 000 Änderungen | 0,698 (0,696–0,698) | 0,662 (0,660–0,665) |
| 100 000 Änderungen | 2,132 (2,131–2,182) | 2,183 (2,136–2,215) |
| 4 Kaskaden, ohne Arbeit daneben | 3,829 | – |
| 4 Kaskaden, 100 000 Änderungen | 5,428 (5,386–5,446) | 4,699 (4,323–5,011) |

- **Verdeckung: GPU je Bild 1,637 → 0,551 ms (−66 %).** Von 528 700 Würfeln im Kegel zeichnen beide Phasen 30 200
  (5,7 %), Phase 2 davon 11 900. Was die Verdeckung selbst kostet: Pyramide 0,045, Sichtbarkeit der Phase 2 0,077 ms;
  die Sichtbarkeit der Phase 1 ist billiger als die alte (0,038 statt 0,064 ms: weniger Einträge). Die 0,35 ms des
  Durchgangs der Phase 1 sind jetzt vor allem Pixel: Der Bildschirm ist voll, 2 Mio. Fragmente mit PBR.
- **Detailstufen: Kugeln 2,732 → 0,867 ms** mit Verdeckung; ohne beide 16,956 ms.
- **Kleine Objekte auslassen (unter 1 Pixel): nichts** (gleiche Zahl gezeichnet); was so klein ist, verdeckt schon die
  Pyramide. Für die fernen Kaskaden siehe [Detailstufen](#detailstufen-g5d): die Werfer sind 10 bis 30 Texel groß.
- **Kaskaden**: Die Verdeckung der Kamera spart mit Schatten 1,23 ms (ohne 1,09 ms: Der Hauptdurchgang mit Schatten
  ist je Fragment teurer); die Kaskaden selbst (2,8 ms Tiefe) bleiben, wie sie waren.

### G5e2: Schatten auf Basis, schwache GPUs

**Entschieden** (Nutzer, 10.11.2026, wie unten vorgeschlagen; gebaut in G5e3): **Auf Stufe Basis 2 Kaskaden × 1024 bis 100**, Aufteilung 0,75 und Reichweite 100 wie auf Voll. `Shadows::BASIS` ist diese Vorgabe, `Shadows::DEFAULT` bleibt, was es war (4 × 2048 bis 100, die Vorgabe für Voll), und `Shadows::for_level(level)` gibt die Vorgabe für die Stufe eines Geräts: `scene.set_shadows(Some(&Shadows::for_level(dev.level())))`. Basis heißt Browser, ältere und schwache GPUs, Telefone; ein Inhalt einer Ansicht ohne `AF_GPU_3D` bekommt das Basis-Gerät der UI ([In der Oberfläche](#in-der-oberfläche-g5e1)). Keine App wählt bisher Schatten je Gerät: Die Ansichten von `uigputest` und die Seite „3D-Ansicht“ der Taschen-Galerie setzen keine Schatten (ihre Vergleiche mit dem Strahlenwerfer rechnen ohne), die Tests mit Schatten behalten ihre festen Einstellungen.

**Wie es aussieht** (G5e3, zum ersten Mal angesehen): `render3dtest --shadow-shots DIR` zeichnet die Schatten-Galerie (`shadow_gallery_with`, Kamera 7 über dem Boden, 16 vor der Mitte) auf einem auf Basis begrenzten Gerät offscreen in 1920 × 1080, einmal mit 4 × 2048, einmal mit `Shadows::for_level(Basis)`, und schreibt dazu die Kaskaden: mit vier reicht die nahe bis 6,7 (Texel 7 mm), die zweite bis 14,9 (16 mm), die dritte bis 32 (35 mm), die vierte bis 100 (11 cm); mit zwei reicht die nahe bis 14,9 (Texel 32 mm: doppelt so grob wie die zweite von vier, 4,4-mal so grob wie die nahe), die ferne bis 100 (22 cm: sechsmal so grob wie die dritte von vier, doppelt so grob wie die vierte). Im ganzen Bild sind 0,95 % der Pixel anders (8 800 um mehr als 8 Stufen, höchstens 61), alle an Schattenkanten. Zu sehen: Die Kanten werden weicher und breiter (der Filter deckt 4 × 4 Texel, also die doppelte bis sechsfache Breite); am fernen Ende des Schattens der Kugel (rund 11 vor der Kamera) zeigt die Kante Stufen; der Würfel in der Mitte (rund 20 weit) und die Löcher der MASK-Platte werfen erkennbare, aber deutlich weichere Schatten. Akne oder Lücken gibt es in beiden Bildern nicht. Die Bilder liegen nicht im Repository (`build/`, mit dem Befehl oben neu zu machen). Was der Bias dazu tut: Er versetzt um zwei Texel der Kaskade, mit gröberen Texeln also weiter (in der fernen Kaskade bis 0,44); abgelöste Schatten am Fuß der Würfel sind in diesen Bildern nicht zu sehen, an anderen Szenen nicht geprüft.

Für die Entscheidung „Schatten-Vorgaben auf Basis“ (damals `Shadows::DEFAULT`: 4 Kaskaden × 2048 bis 100, für Voll entschieden). `render3dtest --bench 100000 --basis --frames 150 --warmup 30 [--shadows C --shadow-size S --shadow-distance D]`: die Messszene (100 000 Würfel, eine Liste, 1080p, offscreen, Release-Build für Linux), die Sonne mit `fs_shadow`; je Einstellung drei Läufe im Wechsel, gezeigt der Median der Mediane der GPU-Zeit des Bildes (Spannen eng, unter 1 %). Intel HD 610 (Fedora, Last 0,3–1,1), Iris Xe und NVIDIA T500 (ThinkPad, Last 0,2–0,3).

| GPU-Zeit je Bild (ms) | ohne | 4 × 2048 bis 100 (heute) | 4 × 1024 bis 100 | 2 × 2048 bis 100 | 2 × 1024 bis 100 | 2 × 1024 bis 50 | 1 × 1024 bis 50 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Intel HD 610 | 13,31 | 52,46 | 42,47 | 40,77 | 36,05 | 27,74 | 26,32 |
| … davon Kaskaden: Sichtbarkeit / Tiefe | – | 13,62 / 20,60 | 13,63 / 11,25 | 11,14 / 12,55 | 11,13 / 8,19 | 5,70 / 5,00 | 5,66 / 4,45 |
| Intel Iris Xe | 2,91 | 13,38 | 9,25 | 9,77 | 7,57 | 6,09 | 5,63 |
| … davon Kaskaden: Sichtbarkeit / Tiefe | – | 0,23 / 9,33 | 0,23 / 5,43 | 0,18 / 6,05 | 0,19 / 4,00 | 0,12 / 2,59 | 0,09 / 2,33 |
| NVIDIA T500 | 1,26 | 6,52 | 3,73 | 4,44 | 2,88 | 2,60 | 2,43 |
| … davon Kaskaden: Sichtbarkeit / Tiefe | – | 0,13 / 5,20 | 0,13 / 2,37 | 0,07 / 3,20 | 0,07 / 1,53 | 0,06 / 1,25 | 0,04 / 1,10 |
| (RTX 2070 SUPER, Voll, 1 Mio. Würfel, G5c) | 1,63 | 5,02 | | | | | |

- Die heutige Vorgabe kostet auf den schwachen GPUs das Drei- bis Fünffache des Bildes ohne Schatten (HD 610 +39 ms, Iris Xe +10,5 ms, T500 +5,3 ms bei 100 000 Würfeln). Fast alles sind die Tiefen-Durchgänge, und die wachsen mit den Texeln: 1024 statt 2048 halbiert sie auf Iris Xe und T500.
- Auf der HD 610 ist die **Sichtbarkeit der Kaskaden** der zweite große Posten (2,8–3,4 ms je Kaskade bei 100 000 Instanzen, auf den anderen 0,02–0,06 ms): Dort zählt die Zahl der Kaskaden und die Weite, nicht die Größe der Karte.
- **Vorschlag für Basis** (vom Nutzer am 10.11.2026 so entschieden, oben): **2 Kaskaden × 1024 bis 100**. Gleiche Reichweite wie auf Voll, halbe GPU-Zeit der Schatten oder weniger (HD 610 −31 %, Iris Xe −43 %, T500 −56 % des ganzen Bildes); Basis heißt Browser, ältere und schwache GPUs, Telefone. Der Preis sind gröbere Texel nahe der Kamera (zwei statt vier Ausschnitte: die nahe Kaskade reicht weiter, ihr Texel ist größer; G5c maß für eine Kaskade 1,12 % der Pixel an Schattenkanten auf der anderen Seite gegen 0,43 % mit vier); wie es aussieht, ist hier nicht beurteilt. Wer noch weniger will: 2 × 1024 bis 50 spart auf der HD 610 noch einmal 8 ms (die Sichtbarkeit), auf den anderen wenig; dahinter bleibt die Szene ohne Schatten.
- Nicht gemessen: andere Szenen (die Würfel sind klein und dicht; große Werfer und weite Szenen verteilen sich anders), ein Telefon (Basis auf Mali/Adreno). Die Bildqualität der Vorgabe ist seit G5e3 angesehen (oben), an einer Szene; nicht in Zahlen gegen Schattenstrahlen gemessen.

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

Der fertige Stand im Wechsel mit G5b (drei Läufe, vor dem ersten Satz 11 % GPU; `nvidia-smi` zeigte vor den weiteren Sätzen 100 %, gemessen direkt nach dem eigenen Lauf davor, die Spannen sind eng):

| 1 Mio. Würfel, ms | Sichtbarkeit | Kaskaden: Sichtbarkeit / Tiefe | Zeichnen (Hauptdurchgang) | GPU gesamt |
| --- | ---: | ---: | ---: | ---: |
| G5b (Lambert, e61e1ff7) | 0,063 | – | 1,278 (1,275–1,661) | 1,343 |
| G5c, PBR, Grundlicht | 0,063 | – | 1,578 (1,570–1,585) | 1,648 |
| G5c, PBR, Umgebungslicht (Himmel) | 0,063 | – | 1,653 (1,648–1,658) | 1,728 |
| G5c, Umgebung, 4 Kaskaden, 8 Punktlichter | 0,063 | 0,187 / 2,808 | 2,845 (2,775–2,851) | 6,003 |

- **PBR mit Grundlicht: 23 % mehr Zeichenzeit als G5b** (0,30 ms), die Umgebung weitere 0,075 ms (Polynom der Harmonischen, ein Zugriff auf den Würfel).

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

Aus G5e2: sRGB-Kodieren auf anderen GPUs als der RTX 2070 SUPER und Chromes D3D12 (die Funktion `pow` hat dort andere Genauigkeit; die Ansichten unter Linux und auf Android vergleichen mit dem Strahlenwerfer in sRGB-Stufen, `--test srgb` lief nur hier), 10-Bit-Ziele (`Rgb10a2Unorm` kodiert nicht), HDR-Ziele. Aus G5e1: mit Schatten, Lichtern und Umgebung in der Ansicht (die Tests nehmen fünf Formen unter der Sonne, die Messung die Galerie).

Aus den Listen je Alpha-Modus: andere GPUs (ob sie den frühen Tiefentest ohne `discard` wirklich ziehen, zeigt nur eine Messung dort), mehrere Szenen eines Renderers mit einem Klassenwechsel zwischen `prepare` und `record` (oben), Szenen mit sehr vielen Listen beider Klassen (Messung mit einer Liste je Klasse), der Klassenwechsel eines Materials mit 1 Mio. Instanzen (die Szene sieht dann alle Plätze an).

Aus G5d: die Verdeckung auf anderen GPUs, in 4K und in einem echten Fenster im Dauerbetrieb (das Fenster-Skript zeichnet sechs Bilder), Tiefenanhänge, die eine Array-Ansicht mit einer Schicht sind (die Pyramide liest `texture_depth_2d`; `lib/gpu` sagt die Dimension einer Ansicht nicht), Szenen, in denen Phase 1 lange nutzlos bleibt (eine Kamera, die in jedem Bild springt), Detailstufen in glTF-Szenen (kein `MSFT_lod`), die Rechen-Warteschlange auf GPUs ohne eigene Familie (der Rückweg ist mit `jfgui_GPU_COMPUTE=0` getestet), auf AMD (dort kostet concurrent sharing vielleicht etwas) und unter Linux.

Aus G5c: Schatten mit Dreiecken, die die nahe Ebene einer Kaskade schneiden (Pancaking), sehr flach stehende Sonne, Kaskaden auf anderen GPUs, Umgebungen aus echten HDR-Dateien (die Tests erzeugen ihre), große Seiten (über 64) und ihre Ladezeit, Normalenkarten, die mit MikkTSpace gebacken wurden (Facetten auf groben Meshes), Spotlichter mit Kegeln nahe π/2, sehr viele Lichter über 256.


Andere GPUs (AMD, Intel, Telefone), Vulkan 1.1 und 1.2 auf echter Hardware, die Tests unter Linux auf einer echten GPU (auf llvmpipe in WSL, Mesa 25.2.8, bestehen seit dem 2026-11-10 alle 26 Skripte) und Android (nicht gebaut), Firefox (kopflos kein WebGPU), Safari, 4K, ein Fenster im Dauerbetrieb, ein verlorenes Gerät mitten in der Arbeit über den Rückgabewert hinaus, Szenen über 1 Mio. Instanzen, Zeiten im Browser. Aus G5b: Texturen mit mehr Ebenen, als eine Klasse auf diesem Gerät hat (2048), eine Klasse am Limit von 256 Ebenen im Browser, Texturen größer als 2048 außer dem einen Fall 4096 × 64, eine sehr große Zahl von Texturen in einem echten glTF (die Tests schreiben ihre Dateien selbst; Beispielmodelle von Khronos wären ein Download), Dateien mit `KHR_texture_transform` und weiteren UV-Sätzen.

## Was nicht gebaut ist

Aus den Listen je Alpha-Modus: eine dritte Klasse für Durchsichtiges (glTFs BLEND bleibt deckend; was sie bräuchte: [oben](#listen-je-alpha-modus)), ein Klassenwechsel eines Materials, der nur dessen Instanzen ansieht statt aller Plätze, eine Pipeline ohne `fs_shadow` für MASK-Materialien ohne Löcher in der Textur (das Material sagt MASK, die Textur könnte deckend sein; der Aufrufer weiß es), Zähler der Listen je Arbeitsgruppe vorsammeln (die Sichtbarkeit gemischter Szenen kostet sonst mehr, [Messung](#messung-der-listen-je-alpha-modus); kommt mit der ersten richtigen 3D-Test-App, Entscheidung des Nutzers vom 09.11.2026, [ROADMAP](ROADMAP.md)).

Aus G5d: Verdeckung für die Schatten-Kaskaden, ein Reprojizieren der Pyramide des Bildes davor, ein Bau der Pyramide in einem Dispatch (heute einer je Stufe), feinere Stufen mit mehr als 2 × 2 Texeln je Test, `draw_indexed_indirect_count` mit Zusammenschieben leerer Listen (keine Messung trug es), eine Vereinfachung von Meshes in der Bibliothek, `MSFT_lod`, eine eigene Grenze in Texeln für kleine Werfer in den Kaskaden (gebaut, gemessen, wieder entfernt), der Einsatz der Rechen-Warteschlange in render3d (gemessen, kein klarer Gewinn), feinere Wartestufen in `submit_after`, eine Kopierwarteschlange.

Aus G5a heraus bewusst nicht: Meshes entfernen (die Pools wachsen nur), Skalierung 0 auf einzelnen Achsen (erlaubt, aber die Flächen ohne Normale werden schwarz; 0 auf allen Achsen versteckt eine Instanz sauber), Scherung (ein glTF-Knoten mit Scherung in der Weltmatrix wird ohne sie gezeichnet und gezählt), orthografische Kamera, Durchsichtigkeit (glTFs BLEND wird deckend gezeichnet), mehrere Ansichten (die Struktur ist da, `Scene` hält eine), mehr Instanzen, als ein Dispatch erreicht, MSAA (fehlt in `lib/gpu`), Schrumpfen von Kapazitäten, ein Rechen-Shader zum Bewegen. Aus G5b: Texturen entfernen, Bindless auf Voll, Mip-Stufen auf der GPU, anisotrope Filterung, beidseitige Materialien (`double_sided`: von vorn gezeichnet, gezählt), weitere UV-Sätze, komprimierte Texturen (BC, KTX2). Aus G5c: Cluster- oder Kachel-Shading für viele Lichter (die Messung verlangt es ab etwa acht), Tangenten aus der Datei oder MikkTSpace, Mehrfachstreuung (weißes Metall verliert bei Rauheit 1 bis 70 %), Schatten des Umgebungslichts (Verdeckung nur aus der Textur), Vorfiltern auf der GPU, RGBE-Leser, Überblenden zwischen Kaskaden, Ausblenden am Ende der Schattenweite, Schatten für Punkt- und Spotlichter, Auslassen kleiner Werfer in fernen Kaskaden. Eine Sichtbarkeit je Instanz ist nur Kugel gegen sechs Ebenen: Verdeckung kommt mit G5d, und dort liegt nach G1 der größte Hebel.
