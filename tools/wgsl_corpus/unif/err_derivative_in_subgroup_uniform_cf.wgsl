// WGSL "Uniformity Analysis": a subgroup decision is not uniform for a derivative
// expect: error
// uniformity
enable subgroups;
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = 0.0;
  if subgroupAll(true) { c = dpdx(x); }
  return vec4f(c);
}
