// WGSL "Uniformity Analysis": a non-uniform return nested in a uniform if still counts
// expect: error
// uniformity
@group(0) @binding(0) var<uniform> u: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if u == 0u { if i == 0u { return; } }
  workgroupBarrier();
}
