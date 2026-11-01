// WGSL "Pipeline Creation": in a fragment shader
// expect: pipeline-error
override a: i32 = 0;
@fragment fn fs() -> @location(0) vec4f { var x = 1; x = 1 / a; return vec4f(f32(x)); }
