// WGSL "Address Spaces": a workgroup var has no initializer
// expect: error
var<workgroup> a: i32 = 1;
