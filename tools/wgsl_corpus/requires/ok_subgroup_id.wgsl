// WGSL "Language Extensions": requires subgroup_id: subgroup_id and num_subgroups built-ins
// expect: valid
requires subgroup_id;
enable subgroups;
@group(0) @binding(0) var<storage, read_write> o: array<u32>;
@compute @workgroup_size(64) fn main(@builtin(subgroup_id) s: u32, @builtin(num_subgroups) n: u32, @builtin(local_invocation_index) i: u32) { o[i] = s * 1000u + n; }
