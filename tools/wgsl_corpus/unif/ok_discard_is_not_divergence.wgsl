// WGSL "Uniformity Analysis": discard keeps the behavior {Next}
// expect: valid
// uniformity
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = 0.0;
  for (var k = 0; k < 4; k++) { if x > 0.5 { discard; } c += dpdx(x); }
  return vec4f(c);
}
