// WGSL "Subgroups": the id may be a runtime value
// expect: valid
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupShuffle(1u, i);
}
