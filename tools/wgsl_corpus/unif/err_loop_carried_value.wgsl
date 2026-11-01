// WGSL "Uniformity Analysis": an assignment later in the loop reaches the next iteration
// expect: error
// uniformity
@group(0) @binding(0) var<uniform> u: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0u;
  loop { if x == 0u { workgroupBarrier(); } x = i; if u == 4u { break; } }
}
