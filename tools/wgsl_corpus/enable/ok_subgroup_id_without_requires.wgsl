// WGSL "Subgroups": language features apply without requires
// expect: valid
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(subgroup_id) a: u32, @builtin(num_subgroups) b: u32) { }
