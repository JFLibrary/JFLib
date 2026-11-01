// WGSL "Pipeline Creation": an index out of bounds of a const array
// expect: pipeline-error
override a: i32 = 5;
@compute @workgroup_size(1) fn main() { const v = array(1, 2, 3); var x = 1; x = v[a]; }
