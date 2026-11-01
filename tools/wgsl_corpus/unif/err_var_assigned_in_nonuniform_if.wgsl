// WGSL "Uniformity Analysis": a variable assigned in non-uniform control flow is non-uniform after the if
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0;
  if i == 0u { x = 1; }
  if x == 0 { workgroupBarrier(); }
}
