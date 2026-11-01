// WGSL "Texture Built-in Functions": a texture in a let (texture_and_sampler_let)
// expect: valid
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

@fragment fn fs() -> @location(0) vec4f { let tt = t; let ss = s; return textureSample(tt, ss, vec2f()); }
