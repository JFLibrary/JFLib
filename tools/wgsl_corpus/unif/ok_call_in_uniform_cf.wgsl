// WGSL "Uniformity Analysis": the same call in uniform control flow
// expect: valid
// uniformity
fn f() { workgroupBarrier(); }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  f();
}
