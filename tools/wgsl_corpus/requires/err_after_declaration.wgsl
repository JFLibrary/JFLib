// WGSL "Language Extensions": directives come before every declaration
// expect: error
const a = 1;
requires packed_4x8_integer_dot_product;
