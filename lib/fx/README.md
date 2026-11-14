# jflib-fx

The image effects and adjustments of [JFLib](https://jflib.org): blurs, distortions, noise,
artistic and photographic effects, rendered clouds and fractals, and the adjustments of an image
editor from levels and curves to hue and saturation. Every effect computes each pixel from the
source and its parameters alone, so the result is the same however the image is cut into pieces
and however many threads work on it. It is plain Rust, without the standard library and without a
line of `unsafe` code.

## What's in it

65 effects and adjustments, each described by an `FxDesc` with an ASCII key, a category and its
parameters:

- **Adjustments:** auto level, black and white, brightness and contrast, curves, exposure,
  highlights and shadows, hue and saturation, invert colours, invert alpha, levels, posterize,
  sepia, temperature and tint, colour balance.
- **Blurs:** bokeh, fragment, Gaussian, median, motion, radial, sketch, square, surface, unfocus,
  zoom.
- **Distortions:** bulge, crystallize, dents, frosted glass, morphology, pixelate, polar
  inversion, tile reflection, twist.
- **Artistic and photo:** ink sketch, oil painting, pencil sketch, pastel, dream, glow, red eye,
  sharpen, soften portrait, portrait retouch, straighten, vignette, dodge and burn, content-aware
  fill.
- **Objects, noise, colour and rendering:** drop shadow, inner shadow, bevel, feather, add and
  reduce noise, quantize, clouds, Julia and Mandelbrot sets, turbulence.
- **Stylize:** edge detect, emboss, outline, relief.

## How it is used

Every effect follows one contract. `prepare` runs once when the effect has one and gathers what
the whole image decides, such as statistics, lookup tables or noise tables. `render` then fills
rectangles of the destination, from several worker threads at once if you like, and checks a
cancel flag once per row. Pixels are straight ARGB (`gfx::FMT_ARGB`); a selection mask can limit
the statistics and is blended back with `gfx::straight::lerp_mask_row`. Random effects take their
noise from a hash of the pixel's position and the seed, never from a running generator.

```rust
use jflib::{fx, gfx};

fx::use_all();                                    // once: register the built-in effects
let Some(desc) = fx::find("gaussian_blur") else { return };
let mut params = fx::Params::default();
fx::defaults(desc, &mut params);
params.set_i(0, 8);                               // property 0, the radius, lives in slot 0

let roi = [0, 0, src.w, src.h];                   // src: a gfx::Surface in FMT_ARGB
let args = fx::FxArgs::new(&src, roi, &params);
let aux = match desc.prepare {
    Some(prepare) => match prepare(&args) {
        Ok(aux) => aux,
        Err(_) => return,
    },
    None => None,
};
let args = fx::FxArgs { aux: aux.as_deref(), ..args };
let mut out = gfx::Surface::new(src.w, src.h, gfx::FMT_ARGB);
let mut dst = fx::Dst::of_surface(&mut out, roi);
if (desc.render)(&args, &mut dst) == 0 {
    // `out` holds the blurred image
}
```

For parallel work, `fx::tiles` turns disjoint rectangles of one surface into destinations, and the
worker pool's `plat::JobCtl` serves as the cancel flag (`FxArgs::job`). `FXF_BANDS` asks for
full-width bands, `FXF_WHOLE` for one call over the whole image. An app adds effects of its own
with `fx::register`.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::fx`. All of JFLib's crates share one version
and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.0

`black_white_plus` with its default weighting (luminance) gives the right picture now: it
divided the weighted sum by 100 instead of 1000, so nearly every pixel came out white.
Everything is in the [changelog](https://docs.rs/crate/jflib-fx/latest/source/CHANGELOG.md).

## Documentation

- [Effects in JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html#10-effects)
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#fx): every public item
  of `fx`, also on [docs.rs](https://docs.rs/jflib-fx)

## License

MIT or Apache-2.0, at your choice. Many effects and adjustments implement the algorithms of
paint.net 3.36 and of Pinta, both under the MIT license (paint.net's with exceptions for parts
JFLib does not use, such as its logo and artwork), and the sine and cosine of the warp code use
fdlibm's kernel polynomials under Sun's notice; the license texts are in the crate's `licenses/`
folder.
