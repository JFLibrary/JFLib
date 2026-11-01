// WGSL "Uniformity Analysis": a reduction of a uniform value as delta
// expect: valid
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupShuffleUp(1u, subgroupAdd(1u));
}
