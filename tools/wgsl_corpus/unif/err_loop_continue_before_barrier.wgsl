// WGSL "Uniformity Analysis": a non-uniform continue makes the rest of the body non-uniform
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  for (var k = 0u; k < 8u; k++) { if i == k { continue; } workgroupBarrier(); }
}
