// WGSL "Uniformity Analysis": a read_write storage value is non-uniform
// expect: error
// uniformity
@group(0) @binding(0) var<storage, read_write> b: u32;
@compute @workgroup_size(64) fn main() { if b == 0u { workgroupBarrier(); } }
