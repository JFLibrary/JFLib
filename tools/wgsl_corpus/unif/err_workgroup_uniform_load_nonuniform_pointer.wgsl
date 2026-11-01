// WGSL "Uniformity Analysis": workgroupUniformLoad needs a uniform pointer
// expect: error
// uniformity
var<workgroup> w: array<u32, 64>;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let x = workgroupUniformLoad(&w[i]);
}
