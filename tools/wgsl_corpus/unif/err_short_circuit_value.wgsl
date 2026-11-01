// WGSL "Uniformity Analysis": the value of a short-circuit expression depends on both sides
// expect: error
// uniformity
@group(0) @binding(0) var<uniform> u: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let b = i == 0u || u == 1u;
  if b { workgroupBarrier(); }
}
