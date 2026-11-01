// WGSL "Diagnostics": several failures in one function: one warning
// expect: valid
// uniformity
diagnostic(warning, derivative_uniformity);
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = 0.0;
  if x > 0.5 { c = dpdx(x); }
  if x > 0.25 { c += fwidth(x); }
  return vec4f(c);
}
