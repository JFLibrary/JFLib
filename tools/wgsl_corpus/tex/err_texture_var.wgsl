// WGSL "Texture Built-in Functions": a texture in a function var
// expect: error
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

fn f() { var tt: texture_2d<f32>; }
