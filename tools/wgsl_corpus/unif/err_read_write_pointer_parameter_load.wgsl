// WGSL "Uniformity Analysis": a load through a read_write pointer parameter may be non-uniform
// expect: error
// uniformity
fn g(p: ptr<storage, u32, read_write>) { if *p == 0u { workgroupBarrier(); } }
