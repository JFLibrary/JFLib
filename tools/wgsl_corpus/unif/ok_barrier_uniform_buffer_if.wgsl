// WGSL "Uniformity Analysis": workgroupBarrier under a condition on a uniform buffer value
// expect: valid
// uniformity
@group(0) @binding(0) var<uniform> u: u32;
@compute @workgroup_size(64) fn main() { if u == 0u { workgroupBarrier(); } }
