// WGSL "Uniformity Analysis": a return on it neither
// expect: error
// uniformity
enable subgroups;
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  if subgroupAll(true) { return vec4f(); }
  return vec4f(dpdx(x));
}
