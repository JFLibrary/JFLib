// WGSL "Subgroups": the result must be used
// expect: error
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  subgroupAdd(1u);
}
