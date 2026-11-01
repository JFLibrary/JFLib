// WGSL "override Declarations": i32 overflow in an override expression at pipeline creation wraps as in const expressions
// expect: valid
override m: i32 = 2147483647;
override n = m + 1;
@group(0) @binding(0) var<storage, read_write> o: array<i32>;
@compute @workgroup_size(1) fn main() { o[0] = n; }
