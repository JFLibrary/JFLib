// WGSL "Uniformity Analysis": delta must be uniform
// expect: error
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupShuffleUp(1u, i);
}
