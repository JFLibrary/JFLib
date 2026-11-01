// WGSL "Uniformity Analysis": dpdyFine in non-uniform control flow
// expect: error
// uniformity
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = 0.0;
  if x > 0.5 { c = dpdyFine(x); }
  return vec4f(c);
}
