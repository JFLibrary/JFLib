// WGSL "Texture Built-in Functions": textureNumLayers only of arrayed textures
// expect: error
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

fn f() -> u32 { return textureNumLayers(t); }
