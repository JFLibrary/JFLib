// WGSL "Pipeline Creation": of a vector
// expect: pipeline-error
override a: i32 = 5;
@compute @workgroup_size(1) fn main() { var v = vec3i(); var x = 1; x = v[a]; }
