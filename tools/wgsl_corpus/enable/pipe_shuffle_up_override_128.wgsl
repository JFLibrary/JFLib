// WGSL "Subgroups": and an override delta
// expect: pipeline-error
enable subgroups;
override o: u32 = 200u;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupShuffleUp(1u, o);
}
