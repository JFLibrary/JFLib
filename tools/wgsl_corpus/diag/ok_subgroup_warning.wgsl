// WGSL "Diagnostics": diagnostic(warning, subgroup_uniformity)
// expect: valid
// uniformity
enable subgroups;
diagnostic(warning, subgroup_uniformity);
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if i == 0u { let a = subgroupAdd(1u); }
}
