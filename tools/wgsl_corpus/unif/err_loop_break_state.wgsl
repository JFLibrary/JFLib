// WGSL "Uniformity Analysis": the value at a break reaches the end of the loop
// expect: error
// uniformity
@group(0) @binding(0) var<uniform> u: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0u;
  loop { x = i; if u == 4u { break; } x = 0u; if u == 5u { break; } }
  if x == 0u { workgroupBarrier(); }
}
