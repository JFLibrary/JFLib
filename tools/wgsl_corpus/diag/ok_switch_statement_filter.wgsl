// WGSL "Diagnostics": before a switch statement
// expect: valid
// uniformity
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = 0.0;
  @diagnostic(off, derivative_uniformity) switch i32(x) { case 0 { c = dpdx(x); } default { c = dpdy(x); } }
  return vec4f(c);
}
