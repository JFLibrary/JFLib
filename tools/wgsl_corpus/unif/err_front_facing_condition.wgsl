// WGSL "Uniformity Analysis": front_facing is non-uniform
// expect: error
// uniformity
@fragment fn fs(@builtin(front_facing) ff: bool) -> @location(0) vec4f {
  var c = 0.0;
  if ff { c = dpdx(1.0); }
  return vec4f(c);
}
