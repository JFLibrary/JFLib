// WGSL "Declaration and Scope": continue must not skip a declaration the continuing block uses
// expect: error
fn f(c: bool) { loop { if c { continue; } let k = 1; continuing { _ = k; break if true; } } }
