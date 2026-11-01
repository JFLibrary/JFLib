// WGSL "Uniformity Analysis": a delta from a uniform buffer
// expect: valid
// uniformity
enable subgroups;
@group(0) @binding(0) var<uniform> u: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupShuffleUp(1u, u);
}
