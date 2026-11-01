// WGSL "Uniformity Analysis": with a uniform argument
// expect: valid
// uniformity
fn a(c: u32) { if c == 0u { workgroupBarrier(); } }
fn b(c: u32) { a(c + 1u); }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  b(4u);
}
