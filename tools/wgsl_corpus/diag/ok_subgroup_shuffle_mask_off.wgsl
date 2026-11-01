// WGSL "Diagnostics": and on mask
// expect: valid
// uniformity
enable subgroups;
diagnostic(off, subgroup_uniformity);
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupShuffleXor(1u, i);
}
