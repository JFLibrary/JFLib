// WGSL "Attributes": @binding only on resources
// expect: error
@group(0) @binding(0) var<private> a: i32;
