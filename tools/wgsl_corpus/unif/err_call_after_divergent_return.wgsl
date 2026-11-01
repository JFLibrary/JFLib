// WGSL "Uniformity Analysis": a call after a non-uniform return
// expect: error
// uniformity
fn f() { workgroupBarrier(); }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if i == 0u { return; }
  f();
}
