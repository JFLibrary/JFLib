// WGSL "Uniformity Analysis": a variable holding a reduction of a uniform value
// expect: valid
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = subgroupBroadcastFirst(1u);
  if x == 0u { let a = subgroupAdd(1u); }
}
