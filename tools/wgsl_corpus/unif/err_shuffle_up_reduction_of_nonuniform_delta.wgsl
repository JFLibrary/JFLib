// WGSL "Uniformity Analysis": a reduction of a non-uniform value is not
// expect: error
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupShuffleUp(1u, subgroupAdd(i));
}
