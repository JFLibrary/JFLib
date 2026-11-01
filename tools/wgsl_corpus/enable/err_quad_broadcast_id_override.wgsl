// WGSL "Subgroups": an override is not enough
// expect: error
enable subgroups;
override o: u32 = 1u;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = quadBroadcast(1u, o);
}
