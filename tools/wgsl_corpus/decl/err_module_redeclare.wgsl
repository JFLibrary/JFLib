// WGSL "Declaration and Scope": a module-scope name declared twice (const and fn)
// expect: error
const f = 1;
fn f() {}
