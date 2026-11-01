// WGSL "Diagnostics": before an else block
// expect: valid
// uniformity
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = 0.0;
  if x > 0.5 { c = 1.0; } else @diagnostic(off, derivative_uniformity) { c = dpdx(x); }
  return vec4f(c);
}
