// WGSL "Diagnostics": before a function body
// expect: valid
// uniformity
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f @diagnostic(off, derivative_uniformity) {
  var c = 0.0;
  if x > 0.5 { c = dpdx(x); }
  return vec4f(c);
}
