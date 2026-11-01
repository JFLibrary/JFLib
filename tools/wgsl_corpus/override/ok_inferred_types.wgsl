// WGSL "override Declarations": override types are inferred like const: 1 -> i32, 1.5 -> f32
// expect: valid
override a = 1;
override b = 1.5;
override c: f32 = 1;
override d: bool = true;
@group(0) @binding(0) var<storage, read_write> o: array<f32>;
@compute @workgroup_size(1) fn main() { o[0] = f32(a) + b + c + f32(d); }
