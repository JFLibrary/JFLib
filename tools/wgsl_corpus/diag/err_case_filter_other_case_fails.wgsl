// WGSL "Diagnostics": a filter on a case block does not cover the other cases
// expect: error
// uniformity
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = 0.0;
  switch i32(x) { case 0 @diagnostic(off, derivative_uniformity) { c = dpdx(x); } default { c = dpdy(x); } }
  return vec4f(c);
}
