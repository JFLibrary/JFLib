// WGSL "Uniformity Analysis": a non-uniform value stored through a pointer parameter
// expect: error
// uniformity
fn g(p: ptr<function, u32>, i: u32) { *p = i; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { var c = 0u; g(&c, i); if c == 0u { workgroupBarrier(); } }
