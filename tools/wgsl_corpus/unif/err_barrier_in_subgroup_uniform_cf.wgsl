// WGSL "Uniformity Analysis": that is not uniform for a barrier
// expect: error
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if subgroupBroadcastFirst(1u) == 0u { workgroupBarrier(); }
}
