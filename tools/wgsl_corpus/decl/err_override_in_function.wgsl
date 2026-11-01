// WGSL "Variable Declarations": override is only allowed at module scope
// expect: error
fn f() { override a = 1; }
