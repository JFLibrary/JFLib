// WGSL "Subgroups": subgroupAdd not on a matrix
// expect: error
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupAdd(mat2x2f());
}
