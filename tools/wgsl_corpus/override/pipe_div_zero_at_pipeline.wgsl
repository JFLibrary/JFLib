// WGSL "override Declarations": a division by zero in an override expression shows at pipeline creation
// expect: pipeline-error
// constants: {"d": 0}
override d: i32;
override q = 10 / d;
@group(0) @binding(0) var<storage, read_write> o: array<i32>;
@compute @workgroup_size(1) fn main() { o[0] = q; }
