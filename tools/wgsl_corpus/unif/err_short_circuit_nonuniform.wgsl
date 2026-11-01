// WGSL "Uniformity Analysis": && with a non-uniform left side makes the right side non-uniform control flow
// expect: error
// uniformity
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

fn h() -> bool { return textureSample(t, s, vec2f()).x > 0.0; }
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { if x > 0.5 && h() { return vec4f(1.0); } return vec4f(); }
