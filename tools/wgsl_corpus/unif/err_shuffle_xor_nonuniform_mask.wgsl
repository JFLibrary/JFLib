// WGSL "Uniformity Analysis": mask must be uniform
// expect: error
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupShuffleXor(1u, i);
}
