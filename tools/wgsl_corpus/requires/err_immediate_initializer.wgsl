// WGSL "Language Extensions": a var<immediate> has no initializer
// expect: error
requires immediate_address_space;
var<immediate> c: vec4f = vec4f();
