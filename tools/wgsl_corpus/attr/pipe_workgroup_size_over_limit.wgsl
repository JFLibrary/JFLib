// WGSL "Attributes": 16 x 17 = 272 invocations exceed maxComputeInvocationsPerWorkgroup (256)
// expect: pipeline-error
@compute @workgroup_size(16, 17) fn main() {}
