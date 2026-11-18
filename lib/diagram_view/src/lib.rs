// diagram_view -- draws diagram pages (the display list of diagram_core) on render2d and gfx
//
//   FontSet / Class / FontSource     the fonts texts are drawn with (the app hands them over:
//                                    its style's fonts, installed ones); missing families are
//                                    substituted by class with a width factor (TX-09)
//   PageView::new(&doc, page, &mut fonts, &opts)   a page as a render2d scene (made without a
//                                    device) plus gradients and pictures
//   view.draw(dc, sf, r, &cam)       on the CPU: render2d's CPU path, gfx in between
//   view.render(w, h) -> Surface     the whole page as an XRGB image (tests, exports, thumbnails)
//   view.fit(size, margin) -> Camera
//   image::{bytes, decode, base64}   Foreign shapes' pictures: PNG and JPEG decoded, EMF / WMF
//                                    and the rest as placeholders
//   export::{pdf, svg, raster}       (JD12) pages as PDF (lib/pdf), SVG, PNG and JPEG
//
// Why a crate of its own (JD4): diagram_core holds the model and the display list without any
// dependency, so that diagram_io's exports (SVG, PDF, JD12) and JD7's hit tests use the same
// list; drawing it needs render2d (and through it the GPU layer), gfx and the image codecs,
// which neither the model nor the file formats should pull in.  ui knows nothing of diagrams:
// the app connects a PageView to its canvas (ui.diagram's DgPaint, or a ui.gpu_view later).
//
// What render2d's scene cannot hold -- gradient fills and pictures -- is drawn by gfx between
// two of the scene's layers on the CPU path; a scene attached to a GPU (Scene::attach) shows
// everything else.  Coordinates: paper inches, y up, the origin at the page's bottom left
// corner (render2d's camera has y up too).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod direct;
pub mod export;
pub mod fonts;
pub mod image;
mod view;

pub use fonts::{class_of, Class, FontSet, FontSource};
pub use view::{PageView, SYNTHETIC_ITALIC};

pub(crate) fn sqrt(x: f64) -> f64 {
    rt::fx::sqrtd(x)
}

/// (no_std: f64::floor needs std)
pub(crate) fn floor(x: f64) -> f64 {
    if !(x.abs() < 4.5e15) {
        return x;
    }
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}
