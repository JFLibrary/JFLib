// WGSL "Subgroups": mask of subgroupShuffleXor
// expect: error
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupShuffleXor(1u, 128u);
}
