// WGSL "Uniformity Analysis": the value analysis tracks assignments: a full assignment forgets the old value
// expect: valid
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = i;
  x = 0u;
  if x == 0u { workgroupBarrier(); }
}
