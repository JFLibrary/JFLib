// WGSL "Diagnostics": a warning filter on the helper
// expect: valid
// uniformity
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;
@diagnostic(warning, derivative_uniformity) fn h() -> vec4f { return textureSample(t, s, vec2f()); }
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = vec4f();
  if x > 0.5 { c = h(); }
  return c;
}
