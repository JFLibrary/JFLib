// WGSL "Uniformity Analysis": textureBarrier is a barrier too
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if i == 0u { textureBarrier(); }
}
