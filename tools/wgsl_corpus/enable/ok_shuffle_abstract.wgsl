// WGSL "Subgroups": abstract integers convert
// expect: valid
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupShuffle(1u, 1);
  let b = subgroupShuffleUp(1u, 1);
}
