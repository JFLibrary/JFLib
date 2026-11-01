// WGSL "Address Spaces": the function address space is not for module-scope vars
// expect: error
var<function> a: i32;
