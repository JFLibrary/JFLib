// WGSL "Built-in Functions": workgroupUniformLoad takes a workgroup pointer
// expect: error
var<private> p: u32;
@compute @workgroup_size(8) fn main() { _ = workgroupUniformLoad(&p); }
