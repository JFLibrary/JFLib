// WGSL "Built-in Functions": pack4xU8 / unpack4xI8 (packed_4x8_integer_dot_product)
// expect: valid
fn f(v: vec4u) -> vec4i { return unpack4xI8(pack4xU8(v)); }
