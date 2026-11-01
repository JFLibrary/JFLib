// WGSL "Uniformity Analysis": a function with a barrier must be called from uniform control flow
// expect: error
// uniformity
fn f() { workgroupBarrier(); }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if i == 0u { f(); }
}
