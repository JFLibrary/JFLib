// WGSL "Diagnostics": a warning in a function: its info is discarded
// expect: valid
// uniformity
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = 0.0;
  if x > 0.5 @diagnostic(info, derivative_uniformity) { c = dpdx(x); }
  if x > 0.25 @diagnostic(warning, derivative_uniformity) { c += dpdy(x); }
  return vec4f(c);
}
