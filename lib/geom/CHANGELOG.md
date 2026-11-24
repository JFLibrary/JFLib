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
- Boolean operations on convex solids, as brushes in a map editor (G7g part 2): `Solid` (convex cells that do not overlap, sharing their planes) from a convex mesh (`from_convex`), `union`, `difference`, `intersection`, `transformed`, `to_mesh` (one closed surface: where cells touch their faces cancel, points on a neighbour's edge go into it); on meshes `geom::union`, `difference`, `intersection`. Exact where it decides: a point is where three planes meet, never rounded during the operation, its side of a plane the sign of a 4x4 determinant (f64 with an error bound, Shewchuk expansions near 0: the new module `exact` with `Exp`, `orient2d`, `Meet`, `div_round`); every corner rounded once to the nearest f64; faces triangulated on the exact geometry alone; planes within `WELD_RATIO` (1e-9 of the extent) are one, also between the operands. Bevel: `geom::bevel` and `Solid::bevel` chamfer or round edges and corners of a convex solid (segments per quarter turn, corner patches where three faces meet, normals tangent where an arc meets a face, edges along one axis only). `Report::problem_at`: where the first problem of a mesh lies. docs/GEOM.md
- (JD21) `region`: Boolean operations on plane regions with holes for every 2-D app (JFDiagrammer's shape operations first): `boolean` / `union` / `intersection` / `difference` / `xor` of operands made of closed contours (outer contours and holes nested to any depth, filled non-zero or even-odd, several operands), `decompose` into the faces of the overlay with the operands that cover each (open polylines as cutters), `Contour` with a `Mark` per edge (the input edge it lies on, so that callers find their flattened curves again), `Piece`, `area`. Decided by iterated snap rounding on an integer grid of 2^30 steps across the input with exact integer predicates (i128): no crossings in the output, touching edges and points, identical operands, holes on their outer contour and self-overlap need no special cases; the same bits on every target; limits `MAX_EDGES`, `MAX_HOT`. Unit tests in `src/tests.rs` (the crate's first; they run in the suite), docs/GEOM.md "Ebene Regionen"

### Changed

- `math::sqrt_portable` is a thin wrapper of rt's one portable square root (`rt::fx::sqrt_portable`, before a copy of its own), `math::sqrt` is `rt::fx::sqrtd` on every target; the same bits as before for numbers, and NaN results follow `sqrtsd` now (the negative default NaN for a negative number, a quieted NaN for a NaN)

