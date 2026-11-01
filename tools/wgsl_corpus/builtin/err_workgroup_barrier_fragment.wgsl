// WGSL "Built-in Functions": workgroupBarrier only in compute shaders
// expect: error
@fragment fn fs() { workgroupBarrier(); }
