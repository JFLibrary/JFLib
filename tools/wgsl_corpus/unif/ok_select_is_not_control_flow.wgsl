// WGSL "Uniformity Analysis": select of non-uniform values is no branch
// expect: valid
// uniformity
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { let uv = select(vec2f(0.0), vec2f(1.0), x > 0.5); return textureSample(t, s, uv); }
