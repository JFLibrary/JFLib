// WGSL "Subgroups": subgroup_id needs enable subgroups
// expect: error
@compute @workgroup_size(64) fn main(@builtin(subgroup_id) a: u32) { }
