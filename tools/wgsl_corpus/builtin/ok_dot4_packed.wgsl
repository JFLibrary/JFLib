// WGSL "Built-in Functions": dot4U8Packed / dot4I8Packed (packed_4x8_integer_dot_product)
// expect: valid
fn f(a: u32, b: u32) -> u32 { return dot4U8Packed(a, b) + u32(dot4I8Packed(a, b)); }
