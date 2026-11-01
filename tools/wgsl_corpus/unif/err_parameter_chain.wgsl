// WGSL "Uniformity Analysis": a parameter requirement passes through a chain of functions
// expect: error
// uniformity
fn a(c: u32) { if c == 0u { workgroupBarrier(); } }
fn b(c: u32) { a(c + 1u); }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  b(i);
}
