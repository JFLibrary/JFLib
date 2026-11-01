// WGSL "Diagnostics": diagnostic(off, ...) turns the failure off in the whole module
// expect: valid
// uniformity
diagnostic(off, derivative_uniformity);
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = 0.0;
  if x > 0.5 { c = dpdx(x); }
  return vec4f(c);
}
