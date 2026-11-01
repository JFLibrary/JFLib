// WGSL "Diagnostics": before the body of a while loop
// expect: valid
// uniformity
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = 0.0;
  while c < x @diagnostic(off, derivative_uniformity) { c += dpdx(x); }
  return vec4f(c);
}
