// WGSL "Uniformity Analysis": at subgroup scope a reduction is as uniform as its argument
// expect: error
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if subgroupBroadcastFirst(i) == 0u { let a = subgroupAdd(1u); }
}
