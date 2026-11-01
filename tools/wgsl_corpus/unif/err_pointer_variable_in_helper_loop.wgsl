// WGSL "Uniformity Analysis": a pointer parameter behaves like a variable in a loop
// expect: error
// uniformity
fn g(p: ptr<function, u32>, c: u32) { for (var k = 0u; k < 2u; k++) { if *p == 0u { workgroupBarrier(); } *p = c; } }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0u;
  g(&x, i);
}
