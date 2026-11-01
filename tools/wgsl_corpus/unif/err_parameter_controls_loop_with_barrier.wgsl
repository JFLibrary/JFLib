// WGSL "Uniformity Analysis": a parameter that controls a loop around the barrier
// expect: error
// uniformity
fn f(n: u32) { for (var k = 0u; k < n; k++) { workgroupBarrier(); } }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  f(i);
}
