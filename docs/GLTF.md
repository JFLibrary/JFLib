# GLTF -- glTF-2.0-Import

`lib/gltf` (Crate `jflib-gltf`, in der Fassade `jflib::gltf`) liest glTF 2.0 als `.gltf` (JSON mit Dateien daneben oder Data-URIs) und als `.glb` in ein Datenmodell, das keine GPU braucht: Meshes aus Dreiecken mit Positionen, Normalen und einem UV-Satz, Materialien mit den fünf Texturen von pbrMetallicRoughness, dekodierte Bilder, Sampler, Knoten mit ihren Weltmatrizen in `f64`, die Instanzen einer Szene und (G5c) die Lichter von `KHR_lights_punctual`. [`render3d::gltf`](RENDER3D.md) legt ein solches Dokument auf die GPU; ein CAD- oder Spielwerkzeug liest dasselbe ohne Gerät. Entstanden in G5b ([Roadmap](ROADMAP.md)).

| Teil | Wo |
| --- | --- |
| Datenmodell, `load`, `.glb`, URIs, Base64, Matrizen, `decompose` | [`lib/gltf/lib.rs`](../lib/gltf/lib.rs) |
| das JSON ins Modell: Puffer, Accessoren, Meshes, Materialien, Bilder, Knoten, Szenen | [`lib/gltf/read.rs`](../lib/gltf/read.rs) |
| auf die GPU | [`lib/render3d/gltf.rs`](../lib/render3d/gltf.rs) |
| Tests, Messung | [`apps/render3dtest/gltftest.rs`](../apps/render3dtest/gltftest.rs), die Dateien schreibt [`gltfgen.rs`](../apps/render3dtest/gltfgen.rs) |

## Ein Beispiel

```rust
// die Datei und alles, was sie nennt; relative Pfade, schon dekodiert (nie für Data-URIs gerufen)
let bytes = plat::read_file("modell/szene.gltf").ok_or("fehlt")?;
let doc = gltf::load(&bytes, &mut |uri| plat::read_file(&format!("modell/{uri}")), &gltf::Limits::DEFAULT)?;
for i in doc.instances(None) {                       // die Szene der Datei, Tiefe zuerst
    let p = &doc.meshes[i.mesh].primitives[i.primitive];
    let trs = gltf::decompose(&i.world);             // Verschiebung, Drehung, Skalierung (gespiegelt: x negativ), Scherung
    // ... p.positions, p.normals, p.uvs, p.indices, p.material
}
// oder gleich auf die GPU:
let imported = render3d::gltf::upload(&mut renderer, &doc, None)?;
```

## API-Überblick

- **Laden**: `load(bytes, &mut dateien, &Limits) -> Result<Document>` (`.glb` an den ersten vier Bytes `glTF`, sonst JSON), `glb_chunks(bytes)` (JSON- und BIN-Teil), `Limits` (`Limits::DEFAULT`), `Error::{Json, Invalid, Unsupported, Limit, Missing}` mit einem Text, der sagt wo.
- **Modell**: `Document { version, generator, meshes, materials, textures, images, nodes, scenes, scene, notes }`, `Mesh { name, primitives }`, `Primitive { positions, normals, uvs, indices, material, flat_normals, has_uvs }`, `Material` (glTFs Vorgaben, wo die Datei schweigt: weiß, metallic 1, roughness 1), `TextureRef { texture, tex_coord }`, `Texture { name, image, sampler }`, `Sampler { mag, min, mip, wrap_s, wrap_t }` (`Sampler::DEFAULT`: linear, lineare Stufen, wiederholend), `Image { name, width, height, rgba }`, `Node { name, mesh, children, parent, local, world }`, `SceneDef`, `AlphaMode`, `Filter`, `Wrap`.
- **Instanzen**: `doc.instances(szene) -> Vec<InstanceRef { node, mesh, primitive, world }>` (None: die Szene der Datei; ohne Szenen jeder Knoten ohne Eltern).
- **Matrizen**: `decompose(&[f64; 16]) -> Trs { translation, rotation, scale, shear }`, `trs_matrix`, `mat_mul`, `IDENTITY`, `sqrt` (spaltenweise 4 × 4, wie glTF).
- **Werkzeuge**: `write_glb(json, bin)`, `base64_encode`, `base64_decode`.

## Was gelesen wird

- **Geometrie**: Dreiecke (TRIANGLES mit und ohne Indizes; Streifen und Fächer werden zu Listen), POSITION, NORMAL, TEXCOORD_0. Ohne Normalen: flache Normalen, je Dreieck drei eigene Ecken (glTF verlangt das). Ohne UV: 0. Byte-Schrittweiten, dünn besetzte Accessoren (`sparse`), Accessoren ohne Puffer (Nullen). Komponenten, wie glTF sie erlaubt: Positionen und Normalen `float`, UV `float` oder normierte `u8`/`u16`, Indizes `u8`, `u16`, `u32`. Punkte und Linien, Primitive ohne POSITION oder ohne ein Dreieck werden ausgelassen (eine Notiz).
- **Knoten**: TRS oder Matrix (die letzte Zeile muss 0 0 0 1 sein: glTF verlangt Matrizen, die sich in T · R · S zerlegen lassen), Weltmatrizen in `f64` von den Wurzeln her ohne Rekursion; jeder Knoten hat höchstens einen Elternknoten, Zyklen sind ein Fehler.
- **Materialien**: pbrMetallicRoughness mit Faktoren und Texturen, Normale (mit `scale`), Verdeckung (mit `strength`), Emission, `alphaMode` mit `alphaCutoff`, `doubleSided`.
- **Bilder**: PNG und JPEG über `lib/img_png` und `lib/img_jpeg`, erkannt an den ersten Bytes (nicht am `mimeType`), als `rgba` mit geradem Alpha. Farbprofile und Gamma der Dateien werden ignoriert (glTF sagt so), ebenso die EXIF-Ausrichtung eines JPEG (UV meinen das gespeicherte Bild). Nur Bilder, die eine Textur nennt, werden dekodiert.
- **Lichter** (G5c): `KHR_lights_punctual`, gerichtet, Punkt und Spot, mit Name, Farbe (Vorgabe weiß), Intensität (Vorgabe 1; Candela, gerichtet Lux), Reichweite und Kegelwinkeln (Vorgaben 0 und π/4); `Document::lights`, `Node::light`, `doc.lights_of(szene) -> Vec<LightRef { node, light, world }>` (`position()`, `direction()`: das −z des Knotens). Geprüft wie alles andere: eine der drei Arten, Farbe mit drei endlichen Werten ab 0, Intensität endlich ab 0, Reichweite über 0, `0 ≤ innen < außen ≤ π/2`, ein Spot braucht `spot`, ein Knoten nennt nur vorhandene Lichter; `Limits::lights` (65 536) begrenzt die Zahl, bevor etwas zugewiesen wird. Die Reichweite eines gerichteten Lichts wird übergangen (Notiz).
- **Erweiterungen**: unbekannte werden übergangen. Nennt `extensionsRequired` irgendeine außer `KHR_lights_punctual` (Draco, meshopt, KTX2/basisu, Quantisierung, Texturtransformation, ...), ist das `Error::Unsupported`: Jede davon ändert, was die Daten bedeuten.
- **Nicht gelesen**: Animationen, Skins, Morph-Ziele, Kameras, Tangenten, Farben je Ecke, weitere UV-Sätze (eine Textur auf `texCoord` 1 behält den Wert, `render3d` nimmt UV 0 und notiert das).

## Dateien sind nicht vertrauenswürdig

- **Jeder Index geht über `get()`, jeder Bytebereich über geprüfte Arithmetik** gegen den Puffer, in dem er liegt; die Profile der Bibliothek rechnen ohne Überlaufprüfung, also prüft der Code selbst. Ein Accessor muss mit seiner Schrittweite in seine Ansicht passen, die Ansicht in ihren Puffer, der Puffer seine `byteLength` haben; Werte müssen endlich sein, Indizes kleiner als die Zahl der Ecken.
- **Grenzen vor dem Zuweisen**: `Limits` begrenzt Dateigröße (1 GiB je Datei), JSON (256 MiB, 16 Mio. Werte; Tiefe 64), alle Puffer zusammen (1 GiB), Ecken (64 Mio.) und Indizes (192 Mio.) aller Primitive, Knoten (1 Mio.) und ihre Tiefe (1024), Instanzen einer Szene (16 Mio.), Pixel eines Bildes (8192² , gelesen aus dem Kopf, bevor dekodiert wird) und aller Bilder (2 GiB). Eine Anzahl wird gegen ihre Grenze geprüft, bevor ein `Vec` dieser Größe entsteht.
- **URIs**: Data-URIs nur mit Base64; alles andere geht als relativer, %-dekodierter Pfad an den Aufrufer. Ein Schema (`http:`, `file:`), ein absoluter Pfad, ein Laufwerk oder ein `..` werden abgelehnt, bevor der Aufrufer sie sieht: Eine fremde Datei kann so keine anderen Dateien lesen lassen.
- **`.glb`**: Kopf, Länge, Chunks und ihre Reihenfolge geprüft; unbekannte Chunks übergangen, fehlende Polsterung geduldet.
- **Fehler statt Panik**: alles oben ist ein `Err` der passenden Art. Die Tests prüfen das an abgeschnittenen und verfälschten Dateien (unten).

## Entscheidungen

- **Eine eigene Bibliothek, nicht ein Modul von `render3d`.** Das Modell braucht keine GPU; ein CAD-Werkzeug, ein Konverter oder ein Test liest Dateien ohne Gerät und ohne `lib/gpu` zu linken. `render3d` hängt an `jflib-gltf` (für `render3d::gltf::upload`), nicht umgekehrt.
- **JSON über `agent_core::json`** (`parse_with_limits`): streng (doppelte Schlüssel sind ein Fehler), begrenzt, Zahlen als genauer Text, `f64` über `str::parse`. Sein Baum kostet bei vielen Knoten vorübergehend etwa das Sechsfache des Textes (Messung in [RENDER3D.md](RENDER3D.md#g5b-texturen-hochladen-gltf)).
- **Weltmatrizen in `f64`, Zerlegung getrennt.** `Node::world` ist das Produkt der Matrizen; `decompose` gibt T, R (Quaternion), S und ein Maß der Scherung (|cos| des schlechtesten Achsenpaars) zurück. Eine Spiegelung (negative Determinante) wird zu einer negativen x-Skalierung; `render3d` zeichnet sie in eigenen Listen. Scherung (ein ungleich skalierter Elternknoten mit gedrehtem Kind) kann `render3d::Transform` nicht halten: `upload` zeichnet ohne sie und zählt sie (`Imported::sheared`).

## Tests

`render3dtest --test gltf` (`tests/gltf.uis`), alles mit Dateien, die [`gltfgen.rs`](../apps/render3dtest/gltfgen.rs) im Test schreibt: eigene PNGs (gespeicherte Deflate-Blöcke, eigenes CRC-32 und Adler-32), JPEGs aus `img_jpeg`, das JSON von Hand. Nichts wird geladen. Ergebnisse auf der RTX 2070 SUPER:

| Prüft | Ergebnis |
| --- | --- |
| dieselbe Szene als `.gltf` mit fünf Dateien daneben (darunter ein Name mit `%20`), als `.gltf` mit Data-URIs, als `.glb` mit Bildern in Pufferansichten | alle drei laden und ergeben gleiche Dokumente |
| Geometrie: `u8`-, `u16`-, `u32`- und keine Indizes, Schrittweite 32 (verschränkt), normierte `u8`- und `u16`-UV, ein dünn besetzter Accessor, flache Normalen | 6 Primitive, 329 Ecken, 654 Indizes Bit für Bit; flache Normalen (hier selbst berechnet) auf 10⁻⁶ |
| Materialien (Faktoren, die fünf Texturen, MASK), Texturen (Bild und Sampler, jeder Filter- und Wrap-Modus), Bilder | wie geschrieben; PNG Byte für Byte, JPEG (Qualität 95) im Mittel 1,7 neben dem Kodierten |
| 48 Knoten mit TRS und Matrizen, eine Kette 40 tief, ein gespiegelter Knoten, zwei Szenen; die Instanzen in Reihenfolge | Weltmatrizen gleich den hier zusammengesetzten (Abstand 0); 7 Instanzen in Reihenfolge |
| `decompose` | T · R · S wieder auf 4,4 · 10⁻¹⁶, Spiegelung genau am gespiegelten Knoten; Scherung 0,600 bei einem ungleich skalierten Eltern mit gedrehtem Kind |
| jede der ersten 256 Längen der `.glb` und jede 61. danach (2274 Dateien), jede Länge des `.gltf`-Textes (12 901) | keine lädt |
| 1200 Dateien mit 1 bis 6 zufällig geänderten Bytes (`.glb` und `.gltf`) | 31 laden noch, 1169 Fehler, keine Panik |
| 51 Dateien mit je einer gebrochenen Regel (Accessor über die Ansicht, Anzahl 10¹², 0 und negativ, Ansicht über den Puffer, Schrittweite 2 und kleiner als das Element, fremde Indizes, falsche Typen, Indizes als `float`, vier Indizes für Dreiecke, Modus 7, Zyklen, zwei Eltern, Knoten als eigenes Kind, Szene mit Nicht-Wurzel, `1e999`, Text statt Zahl, Drehung der Länge 0, Matrix mit falscher letzter Zeile, Draco/meshopt/basisu/Quantisierung verlangt, glTF 1.0, fehlende Datei, `../`, `/`, `C:/`, `http://`, kaputtes `%`, kaputtes Base64, Data-URI ohne Base64, zu kurzer Puffer, zu viele und ungeordnete Sparse-Indizes, fremde Bilder, Texturen, Sampler-Werte, Alpha-Modi; ein Array statt Objekt, kein JSON, ein Schlüssel doppelt) | jede der erwartete Fehler (`Invalid`, `Unsupported`, `Limit`, `Missing`, `Json`) |
| Bilder: ein PNG, das 70 000 × 70 000 behauptet, ein PNG mit verfälschten Daten, ein GIF | `Limit`, `Invalid`, `Unsupported` |
| acht Grenzen unter die Datei gesenkt (Knoten, Tiefe, Instanzen, Pixel, Puffer, JSON, Ecken, Datei) | jedes Mal `Limit` |
| `.glb`: ohne Magie, Version 1, Kopf länger als die Datei, Chunk über das Ende, BIN zuerst, ein zweiter JSON-Chunk | `Invalid`/`Unsupported`; ohne Magie hält `load` die Datei für JSON und sagt `Json` |
| Streifen und Fächer, Punkte, ein Primitiv ohne POSITION | Dreiecke 0 1 2, 2 1 0 und 0 1 2, 0 2 0; ausgelassen mit Notiz |
| (G5c) `KHR_lights_punctual` (`render3dtest --test lights`): eine Datei mit fünf Lichtern aller Arten, eins von zwei Knoten gesetzt, eins unter einem Elternknoten; dieselbe mit der Erweiterung in `extensionsRequired`; jede Länge abgeschnitten; 600 Dateien mit 1 bis 6 geänderten Bytes; 17 gebrochene Regeln (keine Art, unbekannte Art, Art als Zahl, zwei Farbkanäle, negative Farbe, negative, unendliche und als Text geschriebene Intensität, Reichweite 0 und negativ, innen = außen, außen über π/2, negativer innerer Winkel, Spot ohne `spot`, ein Knoten nennt Licht 9 und −1, `lights` als Objekt); die Grenze unter der Zahl; gezeichnet über `upload` gegen den Strahlenwerfer aus der Beschreibung | wie geschrieben, Weltmatrizen 4,4 · 10⁻¹⁶ neben den im Test zusammengesetzten, der Spot zeigt nach unten; lädt; 2337 abgeschnittene: keine lädt; 12 laden, 588 Fehler, keine Panik; jede `Invalid`; `Limit`; 0 falsche Pixel |
| die Szene über `render3d::gltf::upload` gezeichnet, gegen den Strahlenwerfer, der aus der Beschreibung des Tests gebaut ist (nicht aus dem Import), Voll und Basis; dasselbe Bild im Browser | 0 falsche Pixel (Toleranz 6 wegen bilinearer und trilinearer Texturen); im Browser 0 Pixel anders als Vulkan |

Messungen (Laden einer großen Datei, Speicher): [RENDER3D.md](RENDER3D.md#g5b-texturen-hochladen-gltf).

## Was nicht gebaut ist

Schreiben von glTF (außer `write_glb` für Tests), Animationen, Skins, Morph-Ziele, Kameras, Erweiterungen, die Daten umdeuten (Draco, meshopt, KTX2/basisu, `KHR_mesh_quantization`, `KHR_texture_transform`), weitere UV-Sätze, Tangenten, Farben je Ecke, Data-URIs ohne Base64, ein Lesen ohne JSON-Baum für Dateien mit Millionen Knoten.
