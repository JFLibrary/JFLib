// WGSL "Diagnostics": the requirement on delta follows the filter
// expect: valid
// uniformity
enable subgroups;
diagnostic(warning, subgroup_uniformity);
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupShuffleUp(1u, i);
}
