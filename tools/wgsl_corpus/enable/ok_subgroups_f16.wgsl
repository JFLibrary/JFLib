// WGSL "Enable Extensions": subgroup operations on f16 with both extensions
// expect: valid
enable f16;
enable subgroups;
@group(0) @binding(0) var<storage, read_write> o: array<f16>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) g: vec3u) { o[g.x] = subgroupMax(o[g.x]); }
