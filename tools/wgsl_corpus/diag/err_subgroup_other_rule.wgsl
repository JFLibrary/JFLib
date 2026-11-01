// WGSL "Diagnostics": a filter of the other rule does not help
// expect: error
// uniformity
enable subgroups;
diagnostic(off, derivative_uniformity);
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if i == 0u { let a = subgroupAdd(1u); }
}
