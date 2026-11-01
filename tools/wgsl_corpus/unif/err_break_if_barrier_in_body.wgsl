// WGSL "Uniformity Analysis": break if on a non-uniform value
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  loop { workgroupBarrier(); continuing { break if i == 0u; } }
}
