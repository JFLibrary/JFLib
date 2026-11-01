// WGSL "Uniformity Analysis": a plain workgroup load is non-uniform
// expect: error
// uniformity
var<workgroup> w: u32;
@compute @workgroup_size(64) fn main() { if w == 0u { workgroupBarrier(); } }
