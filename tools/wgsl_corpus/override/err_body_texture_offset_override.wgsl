// WGSL "Pipeline Creation": a texture offset must be const
// expect: error
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
override a: i32 = 100;
@fragment fn fs() -> @location(0) vec4f { return textureSampleLevel(t, s, vec2f(), 0.0, vec2i(a)); }
