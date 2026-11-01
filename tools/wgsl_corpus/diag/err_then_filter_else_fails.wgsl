// WGSL "Diagnostics": a filter on the then block does not cover the else block
// expect: error
// uniformity
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = 0.0;
  if x > 0.5 @diagnostic(off, derivative_uniformity) { c = dpdx(x); } else { c = dpdy(x); }
  return vec4f(c);
}
