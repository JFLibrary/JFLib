// WGSL "Texture Built-in Functions": textureSampleCompare needs a depth texture
// expect: error
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler_comparison;
@fragment fn fs() -> @location(0) vec4f { return vec4f(textureSampleCompare(t, s, vec2f(), 0.5)); }
