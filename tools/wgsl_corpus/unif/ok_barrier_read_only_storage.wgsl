// WGSL "Uniformity Analysis": a read-only storage value is uniform
// expect: valid
// uniformity
@group(0) @binding(0) var<storage, read> b: u32;
@compute @workgroup_size(64) fn main() { if b == 0u { workgroupBarrier(); } }
