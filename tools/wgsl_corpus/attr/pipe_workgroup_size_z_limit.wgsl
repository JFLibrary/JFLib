// WGSL "Attributes": z = 65 exceeds maxComputeWorkgroupSizeZ (64)
// expect: pipeline-error
@compute @workgroup_size(1, 1, 65) fn main() {}
