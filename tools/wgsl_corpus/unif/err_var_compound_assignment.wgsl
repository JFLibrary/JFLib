// WGSL "Uniformity Analysis": a compound assignment keeps the operands' uniformity
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0u;
  x += i;
  if x == 0u { workgroupBarrier(); }
}
