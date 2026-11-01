// WGSL "Subgroups": of subgroupBallot too
// expect: error
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  subgroupBallot(true);
}
