// WGSL "Uniformity Analysis": a derivative's result is not subgroup-uniform
// expect: error
// uniformity
enable subgroups;
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = 0.0;
  if dpdx(x) > 0.5 { c = subgroupAdd(1.0); }
  return vec4f(c);
}
