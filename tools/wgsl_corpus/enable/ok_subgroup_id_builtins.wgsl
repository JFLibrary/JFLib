// WGSL "Subgroups": subgroup_id and num_subgroups with the language feature
// expect: valid
enable subgroups;
requires subgroup_id;
@compute @workgroup_size(64) fn main(@builtin(subgroup_id) a: u32, @builtin(num_subgroups) b: u32) { }
