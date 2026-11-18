# jflib-dxf

DXF import for [JFLib](https://jflib.org). It reads AutoCAD's ASCII exchange format, from R12 to
2018, into a plain data model that needs no GPU: the header's variables, the tables of layers,
linetypes, text styles, block records and dimension styles, blocks with their entities, and the
entities of the drawing. A CAD view, a converter or a test reads the same model;
[`jflib-render2d`](https://crates.io/crates/jflib-render2d) maps it into a scene as it is read
(`render2d::dxf`). It is plain Rust, without the standard library and without a line of `unsafe`
code.

## How it is used

```rust
use jflib::{dxf, plat};

// a small file in memory: everything kept
let Some(bytes) = plat::read_file("plan.dxf") else { return };
let doc = match dxf::load(&bytes, &dxf::Limits::DEFAULT) {
    Ok(doc) => doc,
    Err(_) => return,                    // Binary, Invalid or Limit, with a text
};
for e in &doc.entities {
    let layer = doc.name(e.common.layer);
    match &e.kind {
        dxf::Kind::Line { a, b } => { /* ... */ }
        dxf::Kind::Polyline(p) => { /* vertices with bulges and widths */ }
        _ => {}
    }
}

// a large one piece by piece: the sink gets the header, the tables, each block and each entity
struct Count(u64);
impl dxf::Sink for Count {
    fn entity(&mut self, _e: dxf::Entity, _names: &dxf::Names) {
        self.0 += 1;
    }
}
let Some(file) = plat::File::open("city.dxf", plat::FO_READ) else { return };
let mut at = 0u64;
let mut source = |buf: &mut [u8]| {
    let n = file.read_at(at, buf).unwrap_or(0);
    at += n as u64;
    n
};
let mut count = Count(0);
let _ = dxf::read(&mut source, &dxf::Limits::DEFAULT, &mut count);
```

The reader knows LINE, POINT, CIRCLE, ARC, ELLIPSE, SPLINE (also HELIX), LWPOLYLINE and POLYLINE
with their vertices (bulges, widths, 3D polylines, polyface and polygon meshes), TEXT, ATTRIB,
ATTDEF, MTEXT (with R2018's columns), INSERT with MINSERT's array and its attributes, HATCH (paths
of lines, arcs, elliptic arcs, splines and bulged polylines; the pattern's line families; solid and
gradient), SOLID, TRACE, 3DFACE, DIMENSION and ACAD_TABLE (through their anonymous blocks), LEADER,
WIPEOUT, RAY and XLINE. Other kinds are counted by name. Strings before AutoCAD 2007 are decoded
from the header's code page, later ones as UTF-8, and `\U+XXXX` everywhere. `mtext::parse` turns
MTEXT's inline format codes into paragraphs of formatted runs, `specials` TEXT's `%%` codes into
runs with their underline, overline and strike-through. `aci_rgb` gives AutoCAD's colour index,
`font_substitute` which font of JFLib's repository stands in for a text style's font: SHX fonts
belong to Autodesk, and JFLib carries no Microsoft fonts.

## Files are untrusted input

`Limits` bounds the length of a line, the pairs, bytes and points of one entity, the entities,
points, blocks, names and table entries that are kept. A count in a file (vertices, edges, knots)
is never used to allocate: lists grow as their items arrive. Nothing recurses. A value that is not
a number where one belongs is skipped with a note, a group code that is not a number ends the file
with `Error::Invalid`, a binary DXF is `Error::Binary`. A file that ends early gives what was read.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::dxf`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.1

This crate is new and not part of 0.2.1: it comes with the next release. The
[changelog](https://docs.rs/crate/jflib-dxf/latest/source/CHANGELOG.md) will say so.

## Documentation

- [JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html): how an app is written
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#dxf): every public
  item of `dxf`, also on [docs.rs](https://docs.rs/jflib-dxf)
- [RENDER2D.md](https://github.com/JFLibrary/JFLib/blob/main/docs/RENDER2D.md#dxf) (in German):
  what is read, how render2d draws it, the font substitutes, the limits, the decisions and the
  tests

## License

MIT or Apache-2.0, at your choice.
