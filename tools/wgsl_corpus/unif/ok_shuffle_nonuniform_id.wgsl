// WGSL "Uniformity Analysis": the id of subgroupShuffle need not be
// expect: valid
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupShuffle(1u, i);
}
