// WGSL "Uniformity Analysis": a uniform buffer read stored in non-uniform control flow
// expect: error
// uniformity
@group(0) @binding(0) var<uniform> u: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0u;
  if i == 0u { x = u; }
  if x == 0u { workgroupBarrier(); }
}
