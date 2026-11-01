// WGSL "Uniformity Analysis": not in a fragment shader
// expect: error
// uniformity
enable subgroups;
@fragment fn fs(@builtin(subgroup_size) sz: u32) -> @location(0) vec4f { if sz == 32u { let a = subgroupAdd(1u); } return vec4f(); }
