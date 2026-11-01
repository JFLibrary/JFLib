// WGSL "Uniformity Analysis": textureSample after a non-uniform early return
// expect: error
// uniformity
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { if x > 0.5 { return vec4f(); } return textureSample(t, s, vec2f(x)); }
