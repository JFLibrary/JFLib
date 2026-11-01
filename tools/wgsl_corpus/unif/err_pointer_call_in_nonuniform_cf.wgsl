// WGSL "Uniformity Analysis": a call with a pointer in non-uniform control flow makes the pointee non-uniform
// expect: error
// uniformity
fn f(p: ptr<function, i32>) { *p = 1; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0;
  if i == 0u { f(&x); }
  if x == 0 { workgroupBarrier(); }
}
