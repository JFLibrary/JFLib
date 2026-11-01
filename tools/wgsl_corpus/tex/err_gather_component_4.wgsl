// WGSL "Texture Built-in Functions": textureGather component 4 is out of range
// expect: error
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

@fragment fn fs() -> @location(0) vec4f { return textureGather(4, t, s, vec2f()); }
