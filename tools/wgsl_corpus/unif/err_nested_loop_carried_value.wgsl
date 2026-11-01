// WGSL "Uniformity Analysis": and through a nested loop
// expect: error
// uniformity
@group(0) @binding(0) var<uniform> u: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0u;
  loop { loop { if x == 0u { workgroupBarrier(); } if u == 1u { break; } } x = i; if u == 4u { break; } }
}
