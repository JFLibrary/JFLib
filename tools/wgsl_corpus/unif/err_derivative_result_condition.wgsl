// WGSL "Uniformity Analysis": a derivative's result is not uniform
// expect: error
// uniformity
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  let d = dpdx(1.0);
  var c = 0.0;
  if d > 0.5 { c = dpdy(x); }
  return vec4f(c);
}
