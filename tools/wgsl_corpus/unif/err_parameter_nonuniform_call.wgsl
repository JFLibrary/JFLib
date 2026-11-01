// WGSL "Uniformity Analysis": a function whose parameter controls a barrier, called with a non-uniform value
// expect: error
// uniformity
fn g(c: u32) { if c == 0u { workgroupBarrier(); } }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { g(i); }
