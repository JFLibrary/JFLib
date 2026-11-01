// WGSL "Subgroups": subgroup_invocation_id and subgroup_size in a compute shader
// expect: valid
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(subgroup_invocation_id) a: u32, @builtin(subgroup_size) b: u32) { }
