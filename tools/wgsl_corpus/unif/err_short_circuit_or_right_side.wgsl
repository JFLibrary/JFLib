// WGSL "Uniformity Analysis": the same for ||
// expect: error
// uniformity
fn f() -> bool { workgroupBarrier(); return true; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if i == 0u || f() { }
}
