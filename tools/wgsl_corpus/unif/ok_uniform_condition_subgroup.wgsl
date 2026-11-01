// WGSL "Uniformity Analysis": a uniform buffer is
// expect: valid
// uniformity
enable subgroups;
@group(0) @binding(0) var<uniform> u: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if u == 0u { let c = subgroupAdd(1u); }
}
