# jflib-gfx

The 2D graphics of [JFLib](https://jflib.org): images in memory, a draw context that fills,
copies and clips, and anti-aliased vector paths with strokes, dashes and gradients. JFLib's UI
draws its widgets with it on the CPU, and a paint program draws its canvas with it. It is plain
Rust, without the standard library and without a line of `unsafe` code.

## Surfaces and the draw context

A `Surface` is the pixels of one image: rows of 32-bit pixels, or 8-bit coverage for masks. Four
formats cover what an app meets: `FMT_XRGB` for a window's framebuffer, `FMT_PARGB` for images and
layers (premultiplied, the form JFLib draws in), `FMT_ARGB` for straight alpha, which the image
effects of [`jflib-fx`](https://crates.io/crates/jflib-fx) work on, and `FMT_A8` for coverage and
masks. Colours in the API are always straight `0xAARRGGBB`.

A `Dc` holds what a drawing call needs besides the target: the clip, the origin, the colour and an
opacity. The surface is passed to every call, so one context can draw into several targets, and
`begin` / `end` switch between them with their clip and origin saved. `fill`, `clear`, `hline`,
`vline`, `frame`, `blit` and `blit_scaled` are the basic operations; `begin_rows` lets a thread
draw one band of rows of a taller target and get exactly the bytes the whole target would get
there.

```rust
use jflib::gfx::{self, path::Path, raster::Raster, Dc, Surface};

let mut sf = Surface::new(256, 256, gfx::FMT_PARGB);
let mut dc = Dc::new(sf.w, sf.h);
dc.clear(&mut sf, 0xFFFF_FFFF);
dc.set_color(0xFF00_78D4);
dc.fill(&mut sf, 16, 16, 96, 64);

// an ellipse, filled and outlined; path coordinates are 24.8 fixed point
let (mut path, mut ras) = (Path::default(), Raster::default());
path.begin(&dc);
path.ellipse_fx(160 * gfx::FX_ONE, 160 * gfx::FX_ONE, 64 * gfx::FX_ONE, 40 * gfx::FX_ONE);
path.fill(&mut ras, &dc, &mut sf, gfx::RULE_NONZERO);
dc.set_color(0xFF20_2020);
path.stroke(&mut ras, &dc, &mut sf, 3 * gfx::FX_ONE, gfx::JOIN_ROUND | gfx::CAP_ROUND << 4);
```

## Paths, strokes and the rest

`Path` builds outlines from lines, quadratic and cubic Béziers, rectangles, rounded rectangles and
ellipses, in fixed point relative to the origin or in float pixels. `Raster` turns them into
coverage with a scanline rasterizer that accumulates each edge's signed area, filled with the
non-zero or the even-odd rule. `Path::stroke` draws outlines with mitre, round or bevel joins and
butt, round or square caps; `dash` adds dash patterns and arrow heads. `Shapes` has the shortcuts a
UI needs: rounded rectangles, circles, rings, ellipses and round-capped lines.

Around them sit the pieces of an image editor: `blendmode` with paint.net's layer blend modes,
`gradient` with seven shapes (linear, reflected, diamond, radial, conical and two spirals),
three repeat modes and optional dithering, `tmask` for sparse tiled selection masks that combine,
invert, threshold and trace their outlines, `flood` for the magic wand and the paint bucket,
`pathdata` for the path language of SVG and XAML, `xform` for affine matrices, and the row kernels
of `blend` and `straight`.

The row kernels give the same bytes whichever SIMD instructions the processor offers: their fast
paths are exact shortcuts of the per-pixel formulas, and the float code keeps a fixed order of
operations.

## Large copies

`Dc::blit` copies an XRGB source of `COPY_PAR_MIN` bytes or more in row bands on the loop that
`gfx::set_parallel` was given, and from `COPY_STREAM_MIN` on with streaming stores that bypass the
caches. JFLib's UI runtime hands it the worker pool. Without a loop the copy runs on the calling
thread; the bytes are the same either way.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::gfx`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports. Inside a window, the
`canvas` widget of [`jflib-ui`](https://crates.io/crates/jflib-ui) hands its draw code a `Dc` and
the target surface.

## What's new in 0.2.0

Large blits are faster: an XRGB source of 1 MB or more is copied in row bands on the worker pool,
and from 12 MB on past the caches, so the blit of a 4K canvas frame took about 2 ms on the UI
thread instead of about 5 ms in JFLib's measurement. `gfx::set_parallel` sets the loop the bands
run on, and `blend::copy_stream_rows` copies a block of rows with one fence. `blend::copy_stream`
now ends with the fence its streaming stores need, and `ellipse_fx` and `rrect_fx` no longer
wrap for radii from about 59,500 px. Everything is in the
[changelog](https://docs.rs/crate/jflib-gfx/latest/source/CHANGELOG.md).

## Documentation

- [Drawing in a canvas, in JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html#5-widgets)
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#gfx): every public
  item of `gfx`, also on [docs.rs](https://docs.rs/jflib-gfx)

## License

MIT or Apache-2.0, at your choice. The blend modes, the flood fill's colour test and the gradients
follow paint.net 3.36 and Pinta, both under the MIT license (paint.net's with exceptions for parts
JFLib does not use, such as its logo and artwork); the license texts are in the crate's
`licenses/` folder.
