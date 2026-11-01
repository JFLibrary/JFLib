// WGSL "Subgroups": subgroupAnd only on integers
// expect: error
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupAnd(1.0f);
}
