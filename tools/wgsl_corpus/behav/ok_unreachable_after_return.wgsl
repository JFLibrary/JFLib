// WGSL "Statements Behavior Analysis": statements after return are allowed
// expect: valid
fn f() -> i32 { return 1; let a = 2; }
