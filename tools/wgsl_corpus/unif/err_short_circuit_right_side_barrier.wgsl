// WGSL "Uniformity Analysis": the right side of && runs only for some invocations
// expect: error
// uniformity
fn f() -> bool { workgroupBarrier(); return true; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if i == 0u && f() { }
}
