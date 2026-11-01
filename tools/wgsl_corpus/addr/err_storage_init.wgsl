// WGSL "Address Spaces": a storage var has no initializer
// expect: error
@group(0) @binding(0) var<storage, read_write> b: i32 = 1;
