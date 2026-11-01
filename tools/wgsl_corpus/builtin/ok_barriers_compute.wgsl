// WGSL "Built-in Functions": workgroupBarrier, storageBarrier, textureBarrier, workgroupUniformLoad in compute
// expect: valid
var<workgroup> w: u32;
@compute @workgroup_size(8) fn main() { workgroupBarrier(); storageBarrier(); textureBarrier(); _ = workgroupUniformLoad(&w); }
