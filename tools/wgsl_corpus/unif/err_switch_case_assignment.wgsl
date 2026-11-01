// WGSL "Uniformity Analysis": an assignment in a case on a non-uniform selector
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0u;
  switch i { case 0u { x = 1u; } default { } }
  if x == 0u { workgroupBarrier(); }
}
