// WGSL "Uniformity Analysis": and for a whole array
// expect: valid
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var a = array<u32, 64>();
  let p = &a;
  *p = array<u32, 64>();
  if a[0] == 0u { workgroupBarrier(); }
}
