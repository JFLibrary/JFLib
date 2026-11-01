// WGSL "Uniformity Analysis": a parameter that decides an early return
// expect: error
// uniformity
fn f(x: u32) { if x == 0u { return; } workgroupBarrier(); }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  f(i);
}
