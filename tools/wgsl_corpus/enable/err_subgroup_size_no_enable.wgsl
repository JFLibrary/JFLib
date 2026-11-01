// WGSL "Subgroups": subgroup_size needs enable subgroups
// expect: error
@compute @workgroup_size(64) fn main(@builtin(subgroup_size) a: u32) { }
