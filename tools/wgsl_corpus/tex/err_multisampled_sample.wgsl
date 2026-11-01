// WGSL "Texture Built-in Functions": multisampled textures cannot be sampled
// expect: error
@group(0) @binding(0) var t: texture_multisampled_2d<f32>;
@group(0) @binding(1) var s: sampler;
@fragment fn fs() -> @location(0) vec4f { return textureSample(t, s, vec2f()); }
