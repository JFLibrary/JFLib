// WGSL "Limits": in a private variable
// expect: error
var<private> v: array<u32, 65536>;
