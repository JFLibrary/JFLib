// WGSL "Template Lists": a template argument may be a call with commas inside its parentheses
// expect: valid
const n = 2;
fn f() -> array<i32, select(1, n, true)> { return array(1, 2); }
