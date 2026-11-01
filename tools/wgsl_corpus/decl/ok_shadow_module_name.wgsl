// WGSL "Declaration and Scope": a local may shadow a module-scope declaration
// expect: valid
const a = 1;
fn f() -> u32 { let a = 2u; return a; }
