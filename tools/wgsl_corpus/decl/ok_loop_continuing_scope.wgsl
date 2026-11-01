// WGSL "Declaration and Scope": continuing sees the names declared in the loop body
// expect: valid
fn f() -> i32 { var n = 0; loop { let k = n + 1; continuing { n = k; break if n > 3; } } return n; }
