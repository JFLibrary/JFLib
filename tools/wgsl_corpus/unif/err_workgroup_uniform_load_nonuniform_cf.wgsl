// WGSL "Uniformity Analysis": workgroupUniformLoad in non-uniform control flow
// expect: error
// uniformity
var<workgroup> w: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if i == 0u { let x = workgroupUniformLoad(&w); }
}
