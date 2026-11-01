// WGSL "Diagnostics": before an if statement: all clauses and conditions
// expect: valid
// uniformity
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = 0.0;
  @diagnostic(off, derivative_uniformity) if x > 0.5 { c = 1.0; } else if dpdx(x) > 0.0 { c = 2.0; }
  return vec4f(c);
}
