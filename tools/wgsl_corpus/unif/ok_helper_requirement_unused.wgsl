// WGSL "Uniformity Analysis": a parameter requirement of a function nobody calls
// expect: valid
// uniformity
fn g(c: u32) { if c == 0u { workgroupBarrier(); } }
