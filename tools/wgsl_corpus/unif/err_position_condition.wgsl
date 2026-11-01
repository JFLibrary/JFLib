// WGSL "Uniformity Analysis": the fragment position is non-uniform
// expect: error
// uniformity
@fragment fn fs(@builtin(position) p: vec4f) -> @location(0) vec4f {
  var c = 0.0;
  if p.x > 0.5 { c = dpdx(p.x); }
  return vec4f(c);
}
