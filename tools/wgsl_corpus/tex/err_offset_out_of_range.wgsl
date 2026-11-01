// WGSL "Texture Built-in Functions": a texel offset of 8
// expect: error
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

@fragment fn fs(@location(0) uv: vec2f) -> @location(0) vec4f { return textureSample(t, s, uv, vec2i(8, 0)); }
