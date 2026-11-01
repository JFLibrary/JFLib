// WGSL "Texture Built-in Functions": textureSampleCompare needs a sampler_comparison
// expect: error
@group(0) @binding(0) var t: texture_depth_2d;
@group(0) @binding(1) var s: sampler;
@fragment fn fs() -> @location(0) vec4f { return vec4f(textureSampleCompare(t, s, vec2f(), 0.5)); }
