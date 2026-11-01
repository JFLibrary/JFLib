// WGSL "Uniformity Analysis": a let holds the uniformity of its initializer
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let k = i * 2u;
  if k == 0u { workgroupBarrier(); }
}
