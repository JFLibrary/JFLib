# jflib-gltf

glTF 2.0 import for [JFLib](https://jflib.org). It reads `.gltf` files (JSON with files beside it
or data URIs) and `.glb` files into a plain data model that needs no GPU: meshes of triangles,
physically based materials with their textures, decoded images, samplers, nodes with their world
matrices in `f64`, the instances of a scene and its lights. A 3D view, a CAD tool or a converter
reads the same model; [`jflib-render3d`](https://crates.io/crates/jflib-render3d) puts it on the
GPU in one call. It is plain Rust, without the standard library and without a line of `unsafe`
code.

## How it is used

```rust
use jflib::{gltf, plat};

// the file, and the files it names: relative paths, already decoded (never called for data URIs)
let Some(bytes) = plat::read_file("model/scene.gltf") else { return };
let doc = match gltf::load(&bytes, &mut |uri| plat::read_file(&format!("model/{uri}")), &gltf::Limits::DEFAULT) {
    Ok(doc) => doc,
    Err(_) => return,                            // Json, Invalid, Unsupported, Limit or Missing, with a text
};
for i in doc.instances(None) {                   // the file's scene, depth first
    let p = &doc.meshes[i.mesh].primitives[i.primitive];
    let trs = gltf::decompose(&i.world);         // translation, rotation, scale, shear
    // p.positions, p.normals, p.uvs, p.indices, p.material
}
```

`load` tells `.glb` from JSON by the first bytes. Each primitive comes as triangles with positions,
normals and one set of texture coordinates; strips and fans become lists, and a mesh without
normals gets flat ones. Materials keep glTF's metallic-roughness factors and their five textures,
the alpha mode and whether they are double-sided. PNG and JPEG images are decoded through
[`jflib-img-png`](https://crates.io/crates/jflib-img-png) and
[`jflib-img-jpeg`](https://crates.io/crates/jflib-img-jpeg) to straight-alpha RGBA, and only
images a texture uses. `decompose` splits a world matrix into translation, rotation and scale, a
mirror as a negative x scale, and says how much shear is left over. Point, spot and directional
lights of `KHR_lights_punctual` come with `Document::lights` and `lights_of`.

## Files are untrusted input

Every index, offset, length and count is checked against what it points into, and a file that
breaks a rule is an `Err`, never a panic. `Limits` bounds the file sizes, the JSON, all buffers
together, vertices, indices, nodes and their depth, the instances of a scene, the pixels of an
image and of all images, and the lights; a count is checked before anything of its size is
allocated, an image's size is read from its header before it is decoded. URIs that leave the
file's folder (absolute paths, drive letters, `..`, a scheme such as `http:`) are refused before
your closure sees them, so a file cannot make you read other files. A file whose
`extensionsRequired` names anything but `KHR_lights_punctual` (Draco, meshopt, KTX2,
quantization, texture transforms) is `Error::Unsupported`, because each of them changes what the
data means.

Not read: animations, skins, morph targets, cameras, tangents, vertex colours and further sets of
texture coordinates.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::gltf`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.0

jflib-gltf is new in this release: glTF 2.0 import of `.gltf` and `.glb` files into a model
without a GPU, with meshes, materials, decoded PNG and JPEG images, nodes in `f64`, the instances
of a scene and the lights of `KHR_lights_punctual`, all read as untrusted input within limits.
Everything is in the [changelog](https://docs.rs/crate/jflib-gltf/latest/source/CHANGELOG.md).

## Documentation

- [JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html): how an app is written
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#gltf): every public
  item of `gltf`, also on [docs.rs](https://docs.rs/jflib-gltf)
- [GLTF.md](https://github.com/JFLibrary/JFLib/blob/main/docs/GLTF.md) (in German): what is read,
  the limits, the decisions and the tests

## License

MIT or Apache-2.0, at your choice.
