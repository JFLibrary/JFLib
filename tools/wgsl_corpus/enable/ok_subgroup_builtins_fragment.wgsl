// WGSL "Subgroups": and in a fragment shader
// expect: valid
enable subgroups;
@fragment fn fs(@builtin(subgroup_invocation_id) a: u32, @builtin(subgroup_size) b: u32) -> @location(0) vec4f { return vec4f(); }
