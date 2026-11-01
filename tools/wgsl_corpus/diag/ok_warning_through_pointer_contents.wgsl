// WGSL "Diagnostics": a warning filter on a pointer contents requirement
// expect: valid
// uniformity
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
@diagnostic(warning, derivative_uniformity) fn h(p: ptr<function, f32>) -> vec4f { var r = vec4f(); if *p > 0.0 { r = textureSample(t, s, vec2f()); } return r; }
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var a = x;
  return h(&a);
}
