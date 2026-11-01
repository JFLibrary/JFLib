// WGSL "Uniformity Analysis": the same function called with a uniform value
// expect: valid
// uniformity
fn g(c: u32) { if c == 0u { workgroupBarrier(); } }
@group(0) @binding(0) var<uniform> u: u32;
@compute @workgroup_size(64) fn main() { g(u); }
