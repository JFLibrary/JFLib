// WGSL "Subgroups": 127 is the largest delta
// expect: valid
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupShuffleUp(1u, 127u);
}
