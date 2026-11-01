// WGSL "Language Extensions": requires packed_4x8_integer_dot_product
// expect: valid
requires packed_4x8_integer_dot_product;
fn f(a: u32, b: u32) -> i32 { return dot4I8Packed(a, b); }
