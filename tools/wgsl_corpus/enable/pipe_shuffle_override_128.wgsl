// WGSL "Subgroups": an override id of 128 is a pipeline error
// expect: pipeline-error
enable subgroups;
override o: u32 = 200u;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupShuffle(1u, o);
}
