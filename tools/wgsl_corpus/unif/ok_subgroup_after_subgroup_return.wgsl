// WGSL "Uniformity Analysis": for a subgroup function it is
// expect: valid
// uniformity
enable subgroups;
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  if subgroupAll(true) { return vec4f(); }
  return vec4f(subgroupAdd(1.0));
}
