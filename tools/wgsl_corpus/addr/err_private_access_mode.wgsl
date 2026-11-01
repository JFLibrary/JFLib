// WGSL "Address Spaces": private takes no access mode
// expect: error
var<private, read_write> a: f32;
