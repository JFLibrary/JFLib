# Changelog of jflib-geom

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

First release on crates.io.

### Added

- The modelling kernel of the game engine (G7g part 1), deterministic to the bit on every target. `Mesh`: positions, normals, UV in f64, indices, points (the topology: hard edges and UV seams do not open a surface, touching parts stay apart); `transformed`, `translated`, `rotated` (degrees, exact at multiples of 90), `scaled`, `mirrored`, `flipped`, `merge`, `append`, `array`, `radial`, `weld`; `flat`, `smooth` (an angle threshold, corner-angle weights), `uv_box` (box projection in metres); `bounds`, `area`, `volume`, `fingerprint`. Generators standing on y = 0: `cuboid`, `cylinder`, `cone` (also a frustum), `sphere`, `torus`, `stairs`, `arch`; `extrude` (a front-view profile along Z), `prism` (a floor plan up along Y), `sweep` (a profile along a path: rotation-minimising frames, mitred corners, closed loops with their twist spread), `lathe` (a profile about Y, poles on the axis). `poly`: signed area, cleaning, `check_simple`, ear-clipping `triangulate`, `rect`, `circle`, `arc`, `rounded_rect`. `Mesh::check` gives a `Report`: closed and manifold (open, non-manifold and flipped edges, pinched points), outward (inverted components), degenerate triangles, bad normals, bounds, area, volume, components, Euler characteristic; `Report::json`. `math`: Vec3, Aabb, Affine in f64, a correctly rounded `sqrt`, `sin`, `cos`, `atan2` from rt's portable math, `sin_cos_deg`. docs/GEOM.md

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
