// WGSL "Uniformity Analysis": discard under a non-uniform condition leaves later control flow uniform (it demotes to a helper)
// expect: valid
// uniformity
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { if x > 0.5 { discard; } return textureSample(t, s, vec2f(x)); }
