// WGSL "Language Extensions": requires immediate_address_space: var<immediate>
// expect: valid
requires immediate_address_space;
struct C { scale: vec4f, offset: vec4f }
var<immediate> c: C;
@vertex fn vs(@location(0) p: vec4f) -> @builtin(position) vec4f { return c.scale * p + c.offset; }
