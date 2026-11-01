// WGSL "Subgroups": the id is below 128
// expect: error
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupBroadcast(1u, 128u);
}
