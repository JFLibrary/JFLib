// WGSL "Subgroups": the id of quadBroadcast is a const-expression
// expect: error
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = quadBroadcast(1u, i);
}
