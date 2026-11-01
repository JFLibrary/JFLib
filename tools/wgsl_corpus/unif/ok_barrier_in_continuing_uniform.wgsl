// WGSL "Uniformity Analysis": a barrier in the continuing block of a uniform loop
// expect: valid
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var k = 0u;
  loop { k++; continuing { workgroupBarrier(); break if k > 4u; } }
}
