// WGSL "Uniformity Analysis": workgroupUniformLoad needs workgroup uniformity
// expect: error
// uniformity
enable subgroups;
var<workgroup> w: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if subgroupAny(true) { let x = workgroupUniformLoad(&w); }
}
