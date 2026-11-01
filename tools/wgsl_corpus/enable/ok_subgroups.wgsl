// WGSL "Enable Extensions": enable subgroups: subgroupAdd, subgroupBroadcast, subgroup_size
// expect: valid
enable subgroups;
@group(0) @binding(0) var<storage, read_write> o: array<u32>;
@compute @workgroup_size(64) fn main(@builtin(subgroup_invocation_id) i: u32, @builtin(subgroup_size) n: u32, @builtin(global_invocation_id) g: vec3u) {
  o[g.x] = subgroupAdd(i) + subgroupBroadcast(n, 0u) + subgroupBroadcastFirst(i);
}
