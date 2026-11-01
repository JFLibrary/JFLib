// WGSL "Arithmetic Expressions": a run-time shift by 32 is masked, no error
// expect: valid
fn f(k: u32) -> u32 { return 1u << k; }
