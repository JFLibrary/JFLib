// WGSL "Memory Layout": @size on the last member extends the struct
// expect: valid
struct S { a: u32, @size(28) b: u32 }
@group(0) @binding(0) var<storage> s: array<S>;
@compute @workgroup_size(1) fn main() { _ = s[1].b; }
