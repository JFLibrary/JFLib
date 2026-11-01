// WGSL "Subgroups": of type u32
// expect: error
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(subgroup_size) a: i32) { }
