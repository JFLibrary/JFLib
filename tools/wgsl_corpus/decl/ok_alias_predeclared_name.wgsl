// WGSL "Declaration and Scope": a module-scope alias may take a predeclared type name
// expect: valid
alias float4 = vec4<f32>;
fn f() -> float4 { return float4(); }
