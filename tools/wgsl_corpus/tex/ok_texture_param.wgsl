// WGSL "Texture Built-in Functions": textures and samplers as function parameters
// expect: valid
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

fn g(tt: texture_2d<f32>, ss: sampler, uv: vec2f) -> vec4f { return textureSampleLevel(tt, ss, uv, 0.0); }
@fragment fn fs() -> @location(0) vec4f { return g(t, s, vec2f()); }
