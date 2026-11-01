// WGSL "Diagnostics": @diagnostic(off, subgroup_uniformity) on the body
// expect: valid
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if i == 0u @diagnostic(off, subgroup_uniformity) { let a = subgroupAdd(1u); }
}
