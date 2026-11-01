// WGSL "Diagnostics": an info filter on a parameter requirement
// expect: valid
// uniformity
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
@diagnostic(info, derivative_uniformity) fn h(c: f32) -> vec4f { var r = vec4f(); if c > 0.0 { r = textureSample(t, s, vec2f()); } return r; }
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  return h(x);
}
