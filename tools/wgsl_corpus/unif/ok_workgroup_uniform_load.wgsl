// WGSL "Uniformity Analysis": workgroupUniformLoad makes a workgroup value uniform
// expect: valid
// uniformity
var<workgroup> w: u32;
@compute @workgroup_size(64) fn main() { if workgroupUniformLoad(&w) == 0u { workgroupBarrier(); } }
