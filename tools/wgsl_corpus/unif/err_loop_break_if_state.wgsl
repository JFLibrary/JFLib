// WGSL "Uniformity Analysis": the value at a break if reaches the end of the loop
// expect: error
// uniformity
@group(0) @binding(0) var<uniform> u: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0u;
  loop { continuing { x = i; break if u == 1u; } }
  if x == 0u { workgroupBarrier(); }
}
