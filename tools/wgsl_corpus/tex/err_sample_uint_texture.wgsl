// WGSL "Texture Built-in Functions": textureSample needs a float texture
// expect: error
@group(0) @binding(0) var t: texture_2d<u32>;
@group(0) @binding(1) var s: sampler;
@fragment fn fs() -> @location(0) vec4f { return vec4f(textureSample(t, s, vec2f())); }
