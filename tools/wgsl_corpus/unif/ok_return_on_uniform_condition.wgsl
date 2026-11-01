// WGSL "Uniformity Analysis": a return on a uniform condition keeps control flow uniform
// expect: valid
// uniformity
@group(0) @binding(0) var<uniform> u: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if u == 0u { return; }
  workgroupBarrier();
}
