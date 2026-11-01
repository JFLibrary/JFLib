// WGSL "Uniformity Analysis": a break on a uniform condition keeps the loop uniform
// expect: valid
// uniformity
@group(0) @binding(0) var<uniform> u: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  loop { workgroupBarrier(); if u == 0u { break; } }
}
