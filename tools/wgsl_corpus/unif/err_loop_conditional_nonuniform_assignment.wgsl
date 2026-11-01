// WGSL "Uniformity Analysis": a non-uniform assignment in a loop reaches the end
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0u;
  for (var k = 0u; k < 4u; k++) { if k == 2u { x = i; } }
  if x == 0u { workgroupBarrier(); }
}
