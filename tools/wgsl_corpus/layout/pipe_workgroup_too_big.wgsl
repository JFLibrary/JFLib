// WGSL "Memory Layout": workgroup storage above maxComputeWorkgroupStorageSize (16384 bytes)
// expect: pipeline-error
var<workgroup> w: array<f32, 4097>;
@compute @workgroup_size(1) fn main() { w[0] = 1.0; }
