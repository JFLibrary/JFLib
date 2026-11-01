// WGSL "Uniformity Analysis": also not for a derivative
// expect: error
// uniformity
enable subgroups;
@fragment fn fs(@builtin(subgroup_size) sz: u32) -> @location(0) vec4f { var c = 0.0; if sz == 32u { c = dpdx(1.0); } return vec4f(c); }
