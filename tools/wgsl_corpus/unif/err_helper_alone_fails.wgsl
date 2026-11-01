// WGSL "Uniformity Analysis": a function that is never called is analyzed too
// expect: error
// uniformity
@group(0) @binding(0) var<storage, read_write> rw: u32;
fn g() { if rw == 0u { workgroupBarrier(); } }
