// WGSL "Structure Types": an alias may name another alias
// expect: valid
alias A = vec4<f32>;
alias B = A;
fn f() -> B { return A(1.0); }
