// WGSL "Subgroups": an abstract id is i32
// expect: valid
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupBroadcast(1u, 3);
}
