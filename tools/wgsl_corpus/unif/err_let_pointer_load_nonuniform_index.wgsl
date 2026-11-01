// WGSL "Uniformity Analysis": a load through a let pointer with a non-uniform index
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var a: array<u32, 64>;
  let p = &a[i];
  if *p == 0u { workgroupBarrier(); }
}
