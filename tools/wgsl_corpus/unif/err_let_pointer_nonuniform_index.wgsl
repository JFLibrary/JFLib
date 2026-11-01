// WGSL "Uniformity Analysis": the same through a let
// expect: error
// uniformity
fn g(p: ptr<function, u32>) { if *p == 0u { workgroupBarrier(); } }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var a: array<u32, 64>;
  let p = &a[i];
  g(p);
}
