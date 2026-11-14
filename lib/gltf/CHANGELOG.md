# Changelog of jflib-gltf

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

## [0.2.0] - 2026-11-14

First release on crates.io.

### Added

- `gltf::load(bytes, &mut files, &Limits)`: glTF 2.0 from a `.glb` or a `.gltf` (external files
  through the caller's closure, data URIs), told apart by the first bytes, into a `Document` that
  needs no GPU; `glb_chunks` for the parts of a `.glb`.
- Meshes of triangles (`Mesh`, `Primitive`) with positions, normals and one set of texture
  coordinates: byte strides, normalized texture coordinates, sparse accessors, accessors without
  a buffer, strips and fans as lists, flat normals where the file has none.
- Materials after pbrMetallicRoughness with their five textures, the alpha mode and
  `doubleSided`; textures, samplers and images, PNG and JPEG decoded through jflib-img-png and
  jflib-img-jpeg to straight-alpha RGBA.
- Nodes with their world matrices in `f64`, `Document::instances` (each node's primitives with
  its world matrix), `decompose` (translation, rotation, scale, a mirror as a negative x scale,
  and the shear left over), `trs_matrix` and `mat_mul`.
- `KHR_lights_punctual`: `Document::lights`, `Node::light`, `Document::lights_of`; the extension
  may stand in `extensionsRequired`.
- Files are read as untrusted input: every index, range and count is checked, `Limits` bounds
  sizes and counts before anything is allocated (`Limits::buffer_bytes` counts what each buffer
  really holds and the zeros of accessors without a buffer), URIs that leave the file's folder are
  refused, and any other required extension is `Error::Unsupported`.
- `write_glb`, `base64_encode` and `base64_decode`.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
