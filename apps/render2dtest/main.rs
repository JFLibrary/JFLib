// render2dtest -- tests and measurements of lib/render2d (docs/RENDER2D.md), offscreen except `window` and the demo.
//
//   render2dtest --test NAME [--validate]    a test on the device's own level and on one limited to Basis:
//        shaders     wgsl::compile of lib/render2d/*.wgsl (common.wgsl in front) = the embedded SPIR-V, word for word
//        caps        butt, round and square caps at many angles, widths in pixels, against lib/gfx (dash_stroke,
//                    exact-area coverage) on the CPU (refimg.rs: what is compared and the tolerance)
//        joins       miter (also beyond its limit), bevel and round joins, sharp and blunt, open and closed
//                    polylines, against lib/gfx
//        short       many short segments with sharp turns (the labels and details of a plan seen from afar): against lib/gfx
//        hairlines   width 0 (1 px) at every angle and fractional positions; widths below a pixel drawn as one
//        units       widths in drawing units and in pixels at several scales and a rotation: the same against lib/gfx
//        overlap     translucent polylines at sharp corners, a closed one, against lib/gfx's non-zero fill (blended
//                    once) and against the same polyline opaque: no pixel blended twice
//        arcs        circles and arcs of every radius, sweep and cap against their exact outlines filled by lib/gfx,
//                    translucent ones (no pixel where two pieces meet blended twice), what a big circle's quads cover,
//                    the pipeline of a layer with and without an arc
//        dashes      dash patterns in pixels and drawing units, caps, joins where dashes run through vertices, closed
//                    polylines, dashed arcs and circles, against lib/gfx stroking each dash (cut on the CPU in f64,
//                    itself against lib/gfx's own dashes); flat tones of short periods; set_style dashed and solid
//        layers      layer order, hidden layers, the order of insertion, translucent entities over each other
//        changes     insert, update (in place, growing at the end, growing in the middle: the layer laid out anew),
//                    remove (free places, laid out anew), style changes, many changes at once (the scatter shader):
//                    the image = that of a scene built anew; only what changed goes up
//        churn       several random changes of random entities between two frames, fixed seeds (grown twice, grown
//                    and removed right after, styles): the image = that of a scene built anew, byte for byte
//        visibility  the visible lists against the exact test on the CPU (random scenes, fixed seeds, rotated
//                    cameras, records exactly at the target's edges); culled = not culled
//        coords      the same drawing at the origin and at 1e7: the same image (also arcs and dash patterns); a deep
//                    zoom to 1e-6 units per pixel with tiny pans: every line where it belongs within 1/16 px, also
//                    lines 1e7 long, and every ring (circles of radius 1e7, 3e5 and 33 px, an arc of 1e7)
//        phase       dash patterns at 1e-6 units per pixel on a polyline at 1e7 with segments 1e7 long (at its vertex,
//                    in a segment's middle; pattern in pixels, in units, a period of a million pixels) and on a circle
//                    of radius 1e7: every dash's end where it belongs within 1/16 px over 24 pans
//        levels      Basis and Voll draw the same image, byte for byte
//        errors      what the API refuses, an empty scene
//        all         all of the above
//        window      a real window (once; not in `all`): the demo in a ui.gpu_view, a pan and a zoom at a point,
//                    the view in the framebuffer = the same scene offscreen
//   render2dtest --bench N [--res 1080p|WxH] [--frames F] [--basis]   N segments (polylines of random lengths), frame
//                GPU time and what uploads cost (bench.rs)
//   render2dtest --bench N --scene cad [--plain] [..]  the same on a town plan of N drawn records (cad.rs): overview,
//                district, working zoom, hairlines and the plan's widths; with arcs, circles and dash patterns, or
//                (--plain) without, as in G6a2a
//   render2dtest --dump DIR                  gallery::shots() of both levels as DIR/voll_NAME.bgra, DIR/basis_NAME.bgra
//                (rows of B, G, R, A): what tests/engine_web.mjs compares the browser with
//   render2dtest --demo                      the demo: a drawing in a view, pan by dragging, zoom with the wheel at
//                                            the pointer (in the browser and on Android the app is this)
//   render2dtest --gen-shaders FILE          lib/render2d/shaders_spv.rs from lib/render2d/*.wgsl
//
// Without Vulkan or a usable GPU every test prints SKIP and passes.  Exit code 1 when a check failed.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod demo;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
mod bench;
#[cfg(not(target_os = "android"))]
mod cad;
#[cfg(not(target_os = "android"))]
mod gallery;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
mod refimg;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
mod scenes;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
mod tests;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
ui::main!(tests::run());
#[cfg(target_arch = "wasm32")]
ui::main!(match web::wanted() {
    Some(probe) => web::start(probe),
    None => demo::launch(),
});
#[cfg(target_os = "android")]
ui::main!(demo::launch());
