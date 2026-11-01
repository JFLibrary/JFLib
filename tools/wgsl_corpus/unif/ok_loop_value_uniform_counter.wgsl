// WGSL "Uniformity Analysis": a sum of uniform values in a loop
// expect: valid
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0u;
  for (var k = 0u; k < 4u; k++) { x += k; }
  if x == 0u { workgroupBarrier(); }
}
