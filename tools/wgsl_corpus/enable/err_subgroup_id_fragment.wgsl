// WGSL "Subgroups": subgroup_id only in compute shaders
// expect: error
enable subgroups;
@fragment fn fs(@builtin(subgroup_id) a: u32) -> @location(0) vec4f { return vec4f(); }
