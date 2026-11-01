// WGSL "Uniformity Analysis": a conditional store on a non-uniform parameter
// expect: error
// uniformity
fn w(p: ptr<function, u32>, c: u32) { if c == 0u { *p = 1u; } }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0u;
  w(&x, i);
  if x == 0u { workgroupBarrier(); }
}
