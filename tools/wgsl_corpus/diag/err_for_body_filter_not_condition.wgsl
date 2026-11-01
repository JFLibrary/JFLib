// WGSL "Diagnostics": before the body: not the condition
// expect: error
// uniformity
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = 0.0;
  for (var k = 0; k < 3 && dpdx(x) > 0.0; k++) @diagnostic(off, derivative_uniformity) { if x > 0.0 { break; } }
  return vec4f(c);
}
